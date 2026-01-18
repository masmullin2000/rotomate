use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use russh::keys::{PrivateKeyWithHashAlg, load_secret_key};
use russh::{ChannelMsg, Disconnect, Preferred, client};
use russh_sftp::client::SftpSession;
use tokio::net::ToSocketAddrs;

use super::client::Client;

/// Authentication method for SSH connections.
#[derive(Debug, Clone)]
pub enum AuthMethod {
    /// Authenticate using a public/private key pair.
    PublicKey(PathBuf),
    /// Authenticate using a password.
    Password(String),
}

/// Result of executing a single command over SSH.
#[derive(Debug, Clone)]
pub struct CommandResult {
    /// The command that was executed.
    pub command: String,
    /// Exit code (0 = success).
    pub exit_code: u32,
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
}

impl CommandResult {
    /// Returns true if the command succeeded (exit code 0).
    pub const fn success(&self) -> bool {
        self.exit_code == 0
    }
}

/// A wrapper around a russh SSH client session.
pub struct Session {
    session: client::Handle<Client>,
}

impl Session {
    /// Connect to a remote host via SSH.
    pub async fn connect<A>(
        auth: AuthMethod,
        user: impl Into<String>,
        addrs: A,
        inactivity_timeout_secs: u64,
    ) -> Result<Self>
    where
        A: ToSocketAddrs,
    {
        let config = client::Config {
            inactivity_timeout: Some(Duration::from_secs(inactivity_timeout_secs)),
            // Increase window size for better throughput (16MB)
            window_size: 16 * 1024 * 1024,
            // Request max packet size (32KB)
            maximum_packet_size: 32 * 1024,
            // Disable Nagle's algorithm for lower latency
            nodelay: true,
            preferred: Preferred {
                kex: Cow::Owned(vec![
                    russh::kex::CURVE25519_PRE_RFC_8731,
                    russh::kex::EXTENSION_SUPPORT_AS_CLIENT,
                ]),
                ..Default::default()
            },
            ..Default::default()
        };

        let config = Arc::new(config);
        let sh = Client::new(false);
        let user = user.into();

        let mut session = client::connect(config, addrs, sh).await?;

        match auth {
            AuthMethod::PublicKey(key_path) => {
                let key_pair = load_secret_key(&key_path, None)?;
                if !session
                    .authenticate_publickey(
                        &user,
                        PrivateKeyWithHashAlg::new(
                            Arc::new(key_pair),
                            session.best_supported_rsa_hash().await?.flatten(),
                        ),
                    )
                    .await?
                    .success()
                {
                    anyhow::bail!("Authentication (with publickey) failed");
                }
            }
            AuthMethod::Password(password) => {
                if !session
                    .authenticate_password(&user, &password)
                    .await?
                    .success()
                {
                    anyhow::bail!("Authentication (with password) failed");
                }
            }
        }

        Ok(Self { session })
    }

    /// Execute a command and capture output.
    pub async fn call(&mut self, command: &str) -> Result<CommandResult> {
        self.call_with_stdin(command, None).await
    }

    /// Execute a command with optional stdin data and capture output.
    pub async fn call_with_stdin(
        &mut self,
        command: &str,
        stdin_data: Option<&[u8]>,
    ) -> Result<CommandResult> {
        self.call_with_streaming(command, stdin_data, None::<fn(&str, bool)>)
            .await
    }

    /// Execute a command with optional stdin data, streaming output line-by-line.
    ///
    /// When `on_output` is provided, each complete line of output is passed to the callback
    /// as it arrives. The callback receives `(line, is_stderr)` where `is_stderr` is true
    /// for stderr output and false for stdout.
    ///
    /// Output is always captured and returned in the `CommandResult` regardless of streaming.
    pub async fn call_with_streaming<F>(
        &mut self,
        command: &str,
        stdin_data: Option<&[u8]>,
        on_output: Option<F>,
    ) -> Result<CommandResult>
    where
        F: FnMut(&str, bool),
    {
        let mut channel = self.session.channel_open_session().await?;
        channel.exec(true, command).await?;

        // Write stdin data if provided
        if let Some(data) = stdin_data {
            channel.data(data).await?;
            channel.eof().await?;
        }

        let mut code = None;
        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();
        let mut stdout_line_buf = LineBuffer::new();
        let mut stderr_line_buf = LineBuffer::new();
        let mut callback = on_output;

        loop {
            let Some(msg) = channel.wait().await else {
                break;
            };
            match msg {
                ChannelMsg::Data { ref data } => {
                    stdout_buf.extend_from_slice(data);
                    // Stream complete lines if callback provided
                    if let Some(ref mut cb) = callback {
                        for line in stdout_line_buf.feed(data) {
                            cb(&line, false);
                        }
                    }
                }
                ChannelMsg::ExtendedData { ref data, ext } => {
                    if ext == 1 {
                        // stderr
                        stderr_buf.extend_from_slice(data);
                        // Stream complete lines if callback provided
                        if let Some(ref mut cb) = callback {
                            for line in stderr_line_buf.feed(data) {
                                cb(&line, true);
                            }
                        }
                    }
                }
                ChannelMsg::ExitStatus { exit_status } => {
                    code = Some(exit_status);
                }
                _ => {}
            }
        }

        // Flush any remaining partial lines
        if let Some(ref mut cb) = callback {
            if let Some(line) = stdout_line_buf.flush() {
                cb(&line, false);
            }
            if let Some(line) = stderr_line_buf.flush() {
                cb(&line, true);
            }
        }

        let exit_code = code.ok_or_else(|| anyhow::anyhow!("program did not exit cleanly"))?;

        Ok(CommandResult {
            command: command.to_string(),
            exit_code,
            stdout: String::from_utf8_lossy(&stdout_buf).into_owned(),
            stderr: String::from_utf8_lossy(&stderr_buf).into_owned(),
        })
    }

    /// Copy a local file to the remote host via SFTP with pipelined writes.
    /// If `remote_path` ends with `/`, the local filename is appended.
    pub async fn copy_file_to_remote(
        &mut self,
        local_path: impl AsRef<Path>,
        remote_path: &str,
    ) -> Result<u64> {
        use log::debug;
        use std::time::Instant;

        const CHUNK_SIZE: usize = 256 * 1024;

        let local_path = local_path.as_ref();

        // If remote_path ends with /, append the local filename
        let remote_path = if remote_path.ends_with('/') {
            let filename = local_path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| anyhow::anyhow!("Invalid local filename"))?;
            format!("{remote_path}{filename}")
        } else {
            remote_path.to_string()
        };

        let start = Instant::now();
        let contents = std::fs::read(local_path).map_err(|e| {
            anyhow::anyhow!(
                "Failed to read local file {path}: {e}",
                path = local_path.display()
            )
        })?;
        debug!(
            "Read local file in {:?} ({} bytes)",
            start.elapsed(),
            contents.len()
        );

        let file_size = contents.len();
        // Convert to Bytes for zero-copy slicing during pipelined writes
        let contents = bytes::Bytes::from(contents);

        // Open SFTP channel
        let start = Instant::now();
        let channel = self.session.channel_open_session().await?;
        channel.request_subsystem(true, "sftp").await?;
        let sftp = SftpSession::new(channel.into_stream()).await?;
        debug!("Opened SFTP session in {:?}", start.elapsed());

        // Create file and write contents using pipelined writes
        let start = Instant::now();
        let mut remote_file = sftp.create(&remote_path).await?;
        debug!("Created remote file in {:?}", start.elapsed());

        // Use pipelined writes with 256KB chunks for maximum throughput
        let start = Instant::now();
        remote_file
            .write_all_pipelined(contents, CHUNK_SIZE)
            .await?;
        debug!("Wrote {} bytes in {:?}", file_size, start.elapsed());

        let start = Instant::now();
        sftp.close().await?;
        debug!("Closed SFTP session in {:?}", start.elapsed());

        Ok(file_size as u64)
    }

    /// Copy a file from the remote host to a local path via SFTP.
    /// If `local_path` ends with `/`, the remote filename is appended.
    pub async fn copy_file_from_remote(
        &mut self,
        remote_path: &str,
        local_path: impl AsRef<Path>,
    ) -> Result<u64> {
        use tokio::io::AsyncReadExt;

        let local_path = local_path.as_ref();

        // If local_path ends with /, append the remote filename
        let local_path: Cow<'_, Path> = if local_path.ends_with("/") {
            let filename = Path::new(remote_path)
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| anyhow::anyhow!("Invalid remote filename"))?;
            Cow::Owned(local_path.join(filename))
        } else {
            Cow::Borrowed(local_path)
        };

        // Open SFTP channel
        let channel = self.session.channel_open_session().await?;
        channel.request_subsystem(true, "sftp").await?;
        let sftp = SftpSession::new(channel.into_stream()).await?;

        // Read remote file
        let mut remote_file = sftp.open(remote_path).await?;
        let mut contents = Vec::new();
        remote_file.read_to_end(&mut contents).await?;
        sftp.close().await?;

        // Write to local file
        std::fs::write(local_path.as_ref(), &contents).map_err(|e| {
            anyhow::anyhow!("Failed to write local file {}: {e}", local_path.display())
        })?;

        Ok(contents.len() as u64)
    }

    /// Delete a file on the remote host via SFTP.
    pub async fn delete_remote_file(&mut self, remote_path: &str) -> Result<()> {
        // Open SFTP channel
        let channel = self.session.channel_open_session().await?;
        channel.request_subsystem(true, "sftp").await?;
        let sftp = SftpSession::new(channel.into_stream()).await?;

        // Remove the file
        sftp.remove_file(remote_path).await?;
        sftp.close().await?;

        Ok(())
    }

    /// Close the SSH session.
    pub async fn close(&mut self) -> Result<()> {
        self.session
            .disconnect(Disconnect::ByApplication, "", "English")
            .await?;
        Ok(())
    }
}

/// Buffer for accumulating partial lines from SSH data chunks.
///
/// SSH data arrives in arbitrary chunks that may not align with line boundaries.
/// This buffer accumulates partial lines and yields complete lines as they become available.
struct LineBuffer {
    partial: String,
}

impl LineBuffer {
    /// Create a new empty line buffer.
    const fn new() -> Self {
        Self {
            partial: String::new(),
        }
    }

    /// Feed data into the buffer and return any complete lines.
    fn feed(&mut self, data: &[u8]) -> Vec<String> {
        let text = String::from_utf8_lossy(data);
        self.partial.push_str(&text);

        let mut lines = Vec::new();
        while let Some(pos) = self.partial.find('\n') {
            let line = self.partial[..pos].to_string();
            self.partial = self.partial[pos + 1..].to_string();
            lines.push(line);
        }
        lines
    }

    /// Flush any remaining partial line from the buffer.
    fn flush(&mut self) -> Option<String> {
        if self.partial.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.partial))
        }
    }
}
