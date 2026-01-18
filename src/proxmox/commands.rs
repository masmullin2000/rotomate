use anyhow::{Result, bail};

use super::client::ProxmoxClient;
use crate::executor::ProxmoxResult;

/// Proxmox VM action to execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxmoxAction {
    /// Create a snapshot with the given name.
    Snapshot { name: String },
    /// Rollback to a snapshot with the given name (None = use host's default).
    Rollback { name: Option<String> },
    /// Delete a snapshot with the given name.
    DeleteSnapshot { name: String },
    /// Start the VM.
    Start,
    /// Gracefully shutdown the VM.
    Shutdown,
    /// Force stop the VM (immediate power off).
    Stop,
}

impl ProxmoxAction {
    /// Parse a command string into a `ProxmoxAction`.
    ///
    /// Supported formats:
    /// - `snapshot <name>` - Create a snapshot
    /// - `rollback [name]` - Rollback to a snapshot (uses host default if no name)
    /// - `delete <name>` - Delete a snapshot
    /// - `start` - Start the VM
    /// - `shutdown` - Graceful shutdown
    /// - `stop` - Force stop
    pub fn parse(command: &str) -> Result<Self> {
        let parts: Vec<&str> = command.trim().splitn(2, ' ').collect();
        let action = parts[0].to_lowercase();
        let name = if parts.len() > 1 {
            Some(parts[1].trim())
        } else {
            None
        };

        match action.as_str() {
            "snapshot" => {
                let Some(name) = name else {
                    bail!("snapshot command requires a name: 'snapshot <name>'");
                };
                Ok(Self::Snapshot { name: name.into() })
            }
            "rollback" => Ok(Self::Rollback {
                name: name.map(ToString::to_string),
            }),
            "delete" => {
                let Some(name) = name else {
                    bail!("delete command requires a snapshot name: 'delete <name>'");
                };
                Ok(Self::DeleteSnapshot { name: name.into() })
            }
            "start" => Ok(Self::Start),
            "shutdown" => Ok(Self::Shutdown),
            "stop" => Ok(Self::Stop),
            _ => bail!(
                "Unknown proxmox command '{action}'. Supported: snapshot, rollback, delete, start, shutdown, stop"
            ),
        }
    }

    /// Get a human-readable description of the action.
    pub fn description(&self) -> String {
        match self {
            Self::Snapshot { name } => format!("snapshot '{name}'"),
            Self::Rollback { name: Some(name) } => format!("rollback '{name}'"),
            Self::Rollback { name: None } => String::from("rollback (default)"),
            Self::DeleteSnapshot { name } => format!("delete '{name}'"),
            Self::Start => String::from("start"),
            Self::Shutdown => String::from("shutdown"),
            Self::Stop => String::from("stop"),
        }
    }
}

/// Execute a list of Proxmox commands for a VM.
///
/// # Arguments
/// * `client` - The Proxmox API client
/// * `node` - The Proxmox node name
/// * `vmid` - The VM ID
/// * `commands` - List of command strings to execute
/// * `stop_on_error` - Whether to stop execution on first error
/// * `default_snapshot` - Default snapshot name to use for rollback without explicit name
///
/// # Returns
/// A tuple of (results, `stopped_early`) where `stopped_early` indicates if execution was halted due to an error.
///
impl ProxmoxClient {
    pub async fn execute(
        &self,
        node: &str,
        vmid: u32,
        commands: &[String],
        stop_on_error: bool,
        default_snapshot: Option<&str>,
    ) -> (Vec<ProxmoxResult>, bool) {
        let mut results = Vec::new();

        for cmd_str in commands {
            let action = match ProxmoxAction::parse(cmd_str) {
                Ok(a) => a,
                Err(e) => {
                    results.push(ProxmoxResult {
                        action: cmd_str.clone(),
                        vmid,
                        node: node.to_string(),
                        result: Err(e.to_string()),
                    });
                    if stop_on_error {
                        return (results, true);
                    }
                    continue;
                }
            };

            log::debug!(
                "Executing proxmox command '{}' on node '{}' vmid {}",
                action.description(),
                node,
                vmid
            );

            let result = match &action {
                ProxmoxAction::Snapshot { name } => self.snapshot_create(node, vmid, name).await,
                ProxmoxAction::Rollback { name } => {
                    // Use provided name or fall back to default_snapshot
                    let snapshot_name = name.as_deref().or(default_snapshot);
                    match snapshot_name {
                        Some(snap) => self.snapshot_rollback(node, vmid, snap).await,
                        None => Err(anyhow::anyhow!(
                            "rollback command requires a snapshot name (none provided and no default configured)"
                        )),
                    }
                }
                ProxmoxAction::DeleteSnapshot { name } => {
                    self.snapshot_delete(node, vmid, name).await
                }
                ProxmoxAction::Start => {
                    // Start VM and wait for it to be running
                    let start_result = self.vm_start(node, vmid).await;
                    if start_result.is_ok() {
                        self.wait_for_vm_status(node, vmid, "running", 120).await
                    } else {
                        start_result
                    }
                }
                ProxmoxAction::Shutdown => {
                    // Shutdown VM and wait for it to be stopped
                    let shutdown_result = self.vm_shutdown(node, vmid).await;
                    if shutdown_result.is_ok() {
                        self.wait_for_vm_status(node, vmid, "stopped", 120).await
                    } else {
                        shutdown_result
                    }
                }
                ProxmoxAction::Stop => {
                    // Force stop VM and wait for it to be stopped
                    let stop_result = self.vm_stop(node, vmid).await;
                    if stop_result.is_ok() {
                        self.wait_for_vm_status(node, vmid, "stopped", 60).await
                    } else {
                        stop_result
                    }
                }
            };

            let proxmox_result = ProxmoxResult {
                action: action.description(),
                vmid,
                node: node.to_string(),
                result: result.map_err(|e| e.to_string()),
            };

            let failed = proxmox_result.result.is_err();
            results.push(proxmox_result);

            if failed && stop_on_error {
                return (results, true);
            }
        }

        (results, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_snapshot() {
        let action = ProxmoxAction::parse("snapshot pre-deploy").unwrap();
        assert_eq!(
            action,
            ProxmoxAction::Snapshot {
                name: String::from("pre-deploy")
            }
        );
    }

    #[test]
    fn test_parse_rollback() {
        let action = ProxmoxAction::parse("rollback clean-state").unwrap();
        assert_eq!(
            action,
            ProxmoxAction::Rollback {
                name: Some(String::from("clean-state"))
            }
        );
    }

    #[test]
    fn test_parse_rollback_no_name() {
        let action = ProxmoxAction::parse("rollback").unwrap();
        assert_eq!(action, ProxmoxAction::Rollback { name: None });
    }

    #[test]
    fn test_parse_start() {
        let action = ProxmoxAction::parse("start").unwrap();
        assert_eq!(action, ProxmoxAction::Start);
    }

    #[test]
    fn test_parse_shutdown() {
        let action = ProxmoxAction::parse("shutdown").unwrap();
        assert_eq!(action, ProxmoxAction::Shutdown);
    }

    #[test]
    fn test_parse_stop() {
        let action = ProxmoxAction::parse("stop").unwrap();
        assert_eq!(action, ProxmoxAction::Stop);
    }

    #[test]
    fn test_parse_case_insensitive() {
        let action = ProxmoxAction::parse("SNAPSHOT test").unwrap();
        assert_eq!(
            action,
            ProxmoxAction::Snapshot {
                name: String::from("test")
            }
        );
    }

    #[test]
    fn test_parse_snapshot_missing_name() {
        let result = ProxmoxAction::parse("snapshot");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_rollback_empty_string() {
        // Rollback with empty string after space should use default (None)
        let result = ProxmoxAction::parse("rollback   ");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), ProxmoxAction::Rollback { name: None });
    }

    #[test]
    fn test_parse_delete() {
        let action = ProxmoxAction::parse("delete old-snapshot").unwrap();
        assert_eq!(
            action,
            ProxmoxAction::DeleteSnapshot {
                name: String::from("old-snapshot")
            }
        );
    }

    #[test]
    fn test_parse_delete_missing_name() {
        let result = ProxmoxAction::parse("delete");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_unknown_command() {
        let result = ProxmoxAction::parse("reboot");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_with_whitespace() {
        let action = ProxmoxAction::parse("  snapshot   my-snap  ").unwrap();
        assert_eq!(
            action,
            ProxmoxAction::Snapshot {
                name: String::from("my-snap")
            }
        );
    }
}
