use std::path::PathBuf;

use log::warn;
use russh::client;
use russh::keys::ssh_key;

/// SSH client handler for managing connection events.
pub struct Client {
    /// Known hosts file path for host key verification.
    known_hosts_path: PathBuf,
    /// Whether to accept unknown host keys (with a warning).
    accept_unknown: bool,
}

impl Client {
    /// Create a new client handler.
    pub fn new(accept_unknown: bool) -> Self {
        // Use platform-appropriate null device as fallback
        #[cfg(unix)]
        let null_device = PathBuf::from("/dev/null");
        #[cfg(windows)]
        let null_device = PathBuf::from("NUL");

        let known_hosts_path =
            dirs::home_dir().map_or_else(|| null_device, |h| h.join(".ssh/known_hosts"));

        Self {
            known_hosts_path,
            accept_unknown,
        }
    }
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        if let Ok(known_hosts_content) = std::fs::read_to_string(&self.known_hosts_path) {
            let key_fingerprint = server_public_key.fingerprint(ssh_key::HashAlg::Sha256);

            // Parse known_hosts and check for matching key
            for line in known_hosts_content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }

                // known_hosts format: hostname[,hostname...] keytype base64key [comment]
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3
                    && let Ok(known_key) = ssh_key::PublicKey::from_openssh(&format!(
                        "{host_name} {key_type}",
                        host_name = parts[1],
                        key_type = parts[2]
                    ))
                    && known_key.fingerprint(ssh_key::HashAlg::Sha256) == key_fingerprint
                {
                    return Ok(true);
                }
            }

            // Key not found in known_hosts
            if self.accept_unknown {
                warn!("Unknown host key (fingerprint: {key_fingerprint}), accepting anyway");
                return Ok(true);
            }

            warn!("Host key not found in known_hosts (fingerprint: {key_fingerprint})");
            return Ok(false);
        }

        // No known_hosts file - accept if configured to do so
        if self.accept_unknown {
            warn!("No known_hosts file found, accepting host key");
            return Ok(true);
        }

        warn!("No known_hosts file found and accept_unknown is false");
        Ok(false)
    }
}
