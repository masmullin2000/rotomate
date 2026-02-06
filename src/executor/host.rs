use std::path::PathBuf;
use std::time::{Duration, Instant};

use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::config::HostContext;
use crate::config::schema::{OsType, Task};
use crate::proxmox::ProxmoxClient;
use crate::ssh::{AuthMethod, Session};

use super::result::{HostResult, ProxmoxResult};
use super::task;

/// A fully resolved host configuration with all defaults applied.
#[derive(Debug, Clone)]
pub struct Host {
    pub ctx: HostContext,
    pub private_key: Option<PathBuf>,

    /// Whether this host requires a separate sudo password prompt.
    pub sudo_prompt: bool,
    /// Host-specific sudo password (from config or set at runtime).
    pub sudo_password: Option<String>,
    /// SSH password (set at runtime when no `private_key` is available).
    pub ssh_password: Option<String>,

    /// Whether to prompt for Windows administrator password at runtime.
    pub admin_prompt: bool,
    /// Windows administrator password (for privilege escalation).
    pub admin_password: Option<String>,

    /// Proxmox VM ID for this host (for `proxmox_command` operations).
    pub proxmox_vmid: Option<u32>,
    /// Proxmox node name where this VM resides.
    pub proxmox_node: Option<String>,
    /// Default Proxmox snapshot name for rollback operations.
    pub proxmox_snapshot: Option<String>,
}

impl Host {
    /// Execute a task on this host.
    ///
    /// # Arguments
    /// * `task` - The task to execute
    /// * `proxmox_client` - Optional Proxmox client for VM operations
    /// * `on_output` - Optional callback for streaming remote command output line-by-line
    #[allow(clippy::too_many_lines)]
    pub async fn execute<F>(
        &self,
        task: &Task,
        proxmox_client: Option<&ProxmoxClient>,
        on_output: Option<F>,
    ) -> HostResult
    where
        F: FnMut(&str, bool),
    {
        let start_time = Instant::now();
        let host_name = &self.ctx.name;

        log::debug!(
            "Running task '{task}' on host '{host}'",
            task = task.name,
            host = task.name
        );

        // Execute proxmox commands first (if applicable)
        let (proxmox_results, proxmox_stopped_early) =
            self.execute_proxmox(task, proxmox_client).await;

        // If proxmox stopped early, return without connecting to SSH
        if proxmox_stopped_early {
            let builder = super::TaskResult::builder(&task.name)
                .stopped_early(true)
                .verbose(task.verbose.unwrap_or(false))
                .capture_output(task.capture_output.unwrap_or(false))
                .timeout(task.timeout())
                .duration_secs(start_time.elapsed().as_secs_f64())
                .proxmox(proxmox_results);

            return HostResult {
                host_name: host_name.clone(),
                result: Ok(builder.build()),
            };
        }

        // Check if task has any SSH operations - if not, return with just proxmox results
        let needs_ssh = if task.has_steps() {
            task.steps.iter().any(crate::config::TaskStep::needs_ssh)
        } else {
            !task.remote_command.is_empty()
                || !task.upload.is_empty()
                || !task.download.is_empty()
                || !task.delete_remote.is_empty()
        };

        if !needs_ssh {
            let builder = super::TaskResult::builder(&task.name)
                .verbose(task.verbose.unwrap_or(false))
                .capture_output(task.capture_output.unwrap_or(false))
                .timeout(task.timeout())
                .duration_secs(start_time.elapsed().as_secs_f64())
                .proxmox(proxmox_results);

            return HostResult {
                host_name: host_name.clone(),
                result: Ok(builder.build()),
            };
        }

        log::debug!(
            "[{host_name}] Connecting to {host}:{port}",
            host = self.ctx.hostname,
            port = self.ctx.port
        );

        // Build authentication method
        let auth = match (&self.private_key, &self.ssh_password) {
            (Some(key_path), _) => AuthMethod::PublicKey(key_path.clone()),
            (None, Some(password)) => AuthMethod::Password(password.clone()),
            (None, None) => {
                return HostResult::connection_error(
                    host_name,
                    "No authentication method available (no private_key or ssh_password)",
                );
            }
        };

        // Connect to the host
        let session_result = Session::connect(
            auth,
            &self.ctx.username,
            (self.ctx.hostname.as_str(), self.ctx.port),
            task.timeout(),
        )
        .await;

        let mut session = match session_result {
            Ok(s) => s,
            Err(e) => {
                return HostResult::connection_error(host_name, format!("Connection failed: {e}"));
            }
        };

        log::debug!(
            "[{host_name}] Connected, executing task '{name}'",
            name = task.name
        );

        // Determine privilege password based on OS type
        // For Linux: use sudo_password (from task or host)
        // For Windows: use admin_password (from host)
        let privilege_password = match self.ctx.os {
            OsType::Linux => task
                .sudo_password
                .as_deref()
                .or(self.sudo_password.as_deref()),
            OsType::Windows => self.admin_password.as_deref(),
        };

        // Execute the task with the appropriate shell and privilege settings
        let task_result = if task.has_steps() {
            task::execute_task_with_steps(
                &mut session,
                task,
                self.ctx.shell,
                privilege_password,
                proxmox_results,
                false, // proxmox didn't stop early (we checked above)
                on_output,
            )
            .await
        } else {
            task::execute_task(
                &mut session,
                task,
                self.ctx.shell,
                privilege_password,
                proxmox_results,
                false, // proxmox didn't stop early (we checked above)
                on_output,
            )
            .await
        };

        // Close the session
        let _ = session.close().await;

        // Calculate total duration including Proxmox and SSH connection time
        let total_duration = start_time.elapsed().as_secs_f64();

        task_result.map_or_else(
            |e| {
                HostResult::connection_error(
                    host_name.clone(),
                    format!("Task execution failed: {e}"),
                )
            },
            |mut result| {
                // Override with total duration (includes Proxmox + SSH connect time)
                result.duration_secs = total_duration;
                HostResult {
                    host_name: host_name.clone(),
                    result: Ok(result),
                }
            },
        )
    }

    /// Execute proxmox commands for this host if configured.
    ///
    /// Returns (results, `stopped_early`).
    async fn execute_proxmox(
        &self,
        task: &Task,
        proxmox_client: Option<&ProxmoxClient>,
    ) -> (Vec<ProxmoxResult>, bool) {
        // Skip if no proxmox commands to execute
        if task.proxmox_command.is_empty() {
            return (Vec::new(), false);
        }

        let name = &self.ctx.name;

        // Check if host has proxmox VM configuration
        let (Some(vmid), Some(node), Some(proxmox)) =
            (&self.proxmox_vmid, &self.proxmox_node, proxmox_client)
        else {
            log::debug!(
                "vmid: {vmid:?}, node: {node:?}, client: {c:?}",
                vmid = self.proxmox_vmid,
                node = self.proxmox_node,
                c = proxmox_client
            );
            log::info!("[{name}] Skipping proxmox_command: host has no proxmox configuration");
            return (Vec::new(), false);
        };

        log::debug!(
            "[{name}] Executing {len} proxmox command(s) on node '{node}' vmid {vmid}",
            len = task.proxmox_command.len(),
        );

        let (results, stopped_early) = proxmox
            .execute(
                node,
                *vmid,
                &task.proxmox_command,
                task.stop_on_error,
                self.proxmox_snapshot.as_deref(),
            )
            .await;

        // If any start commands succeeded, wait for SSH to become accessible
        if !stopped_early
            && task
                .proxmox_command
                .iter()
                .any(|cmd| cmd.trim().to_lowercase() == "start")
            && results
                .iter()
                .any(|r| r.action.contains("start") && r.result.is_ok())
        {
            log::info!("[{name}] VM started, waiting for SSH port to become accessible...");
            if !self.wait_for_ssh_port(120).await {
                // SSH port not accessible - this is a warning, not a failure
                // The SSH connection will fail later with a clearer error
                log::warn!("[{name}] SSH port did not become accessible within timeout");
            }
        }

        (results, stopped_early)
    }

    pub const fn context(&self) -> &HostContext {
        &self.ctx
    }

    /// Wait for SSH port to become accessible.
    async fn wait_for_ssh_port(&self, timeout_secs: u64) -> bool {
        let addr = format!("{}:{}", self.ctx.hostname, self.ctx.port);
        let connection_timeout = Duration::from_secs(timeout_secs);
        let name = &self.ctx.name;

        log::debug!("[{name}] Waiting for SSH port {addr} to become accessible...");

        let start = tokio::time::Instant::now();
        loop {
            const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
            if start.elapsed() > connection_timeout {
                log::warn!("[{name}] Timeout waiting for SSH port to become accessible");
                return false;
            }
            match timeout(CONNECT_TIMEOUT, TcpStream::connect(&addr)).await {
                Ok(Ok(_)) => {
                    break;
                }
                Ok(Err(_)) | Err(_) => {
                    log::debug!("[{name}] SSH port not yet accessible, retrying...");
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await; // slight delay to ensure readiness
        true
    }
}
