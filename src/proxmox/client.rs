use anyhow::{Context, Result, bail};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use std::time::Duration;

/// Client for interacting with Proxmox VE API.
#[derive(Debug, Clone)]
pub struct ProxmoxClient {
    client: Client,
    base_url: String,
    token_id: String,
    token_secret: String,
}

/// Response wrapper for Proxmox API responses.
#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    data: T,
}

/// Task status response from Proxmox.
#[derive(Debug, Deserialize)]
struct TaskStatus {
    status: String,
    #[serde(default)]
    exitstatus: Option<String>,
}

/// VM status response from Proxmox.
#[derive(Debug, Deserialize)]
struct VmStatus {
    status: String,
}

/// UPID (task ID) response from Proxmox.
#[derive(Debug, Deserialize)]
struct TaskUpid {
    data: String,
}

impl ProxmoxClient {
    /// Create a new Proxmox client.
    ///
    /// # Arguments
    /// * `url` - Base URL for Proxmox API (e.g., `https://proxmox.example.com:8006`)
    /// * `token_id` - API token ID (e.g., `user@pam!token-name`)
    /// * `token_secret` - API token secret
    /// * `verify_ssl` - Whether to verify SSL certificates
    /// * `timeout_secs` - Request timeout in seconds
    pub fn new(
        url: &str,
        token_id: &str,
        token_secret: &str,
        verify_ssl: bool,
        timeout_secs: u64,
    ) -> Result<Self> {
        let client = Client::builder()
            .danger_accept_invalid_certs(!verify_ssl)
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .context("Failed to create HTTP client")?;

        // Normalize URL - remove trailing slash
        let base_url = url.trim_end_matches('/').to_string();

        Ok(Self {
            client,
            base_url,
            token_id: token_id.to_string(),
            token_secret: token_secret.to_string(),
        })
    }

    /// Build the authorization header value.
    fn auth_header(&self) -> String {
        format!("PVEAPIToken={}={}", self.token_id, self.token_secret)
    }

    /// Create a snapshot of a VM.
    pub async fn snapshot_create(&self, node: &str, vmid: u32, name: &str) -> Result<()> {
        let url = format!(
            "{}/api2/json/nodes/{}/qemu/{}/snapshot",
            self.base_url, node, vmid
        );

        let response = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .form(&[("snapname", name)])
            .send()
            .await
            .context("Failed to send snapshot create request")?;

        if response.status() == StatusCode::OK {
            let task: TaskUpid = response
                .json()
                .await
                .context("Failed to parse snapshot response")?;
            self.wait_for_task(node, &task.data).await
        } else {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("unknown error"));
            bail!("Snapshot create failed ({status}): {body}");
        }
    }

    /// Rollback a VM to a snapshot.
    pub async fn snapshot_rollback(&self, node: &str, vmid: u32, name: &str) -> Result<()> {
        let url = format!(
            "{}/api2/json/nodes/{}/qemu/{}/snapshot/{}/rollback",
            self.base_url, node, vmid, name
        );

        let response = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .context("Failed to send snapshot rollback request")?;

        if response.status() == StatusCode::OK {
            let task: TaskUpid = response
                .json()
                .await
                .context("Failed to parse rollback response")?;
            self.wait_for_task(node, &task.data).await
        } else {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("unknown error"));
            bail!("Snapshot rollback failed ({status}): {body}");
        }
    }

    /// Delete a snapshot from a VM.
    pub async fn snapshot_delete(&self, node: &str, vmid: u32, name: &str) -> Result<()> {
        let url = format!(
            "{}/api2/json/nodes/{}/qemu/{}/snapshot/{}",
            self.base_url, node, vmid, name
        );

        let response = self
            .client
            .delete(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .context("Failed to send snapshot delete request")?;

        if response.status() == StatusCode::OK {
            let task: TaskUpid = response
                .json()
                .await
                .context("Failed to parse delete response")?;
            self.wait_for_task(node, &task.data).await
        } else {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("unknown error"));
            bail!("Snapshot delete failed ({status}): {body}");
        }
    }

    /// Start a VM. Returns Ok if VM starts successfully or is already running.
    pub async fn vm_start(&self, node: &str, vmid: u32) -> Result<()> {
        let url = format!(
            "{}/api2/json/nodes/{}/qemu/{}/status/start",
            self.base_url, node, vmid
        );

        let response = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .context("Failed to send VM start request")?;

        if response.status() == StatusCode::OK {
            let task: TaskUpid = response
                .json()
                .await
                .context("Failed to parse start response")?;
            match self.wait_for_task(node, &task.data).await {
                Ok(()) => Ok(()),
                Err(e) => {
                    // Treat "already running" as success
                    let msg = e.to_string();
                    if msg.contains("already running") {
                        log::debug!("VM {vmid} is already running");
                        Ok(())
                    } else {
                        Err(e)
                    }
                }
            }
        } else {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("unknown error"));
            bail!("VM start failed ({status}): {body}");
        }
    }

    /// Gracefully shutdown a VM. Returns Ok if VM shuts down successfully or is already stopped.
    pub async fn vm_shutdown(&self, node: &str, vmid: u32) -> Result<()> {
        let url = format!(
            "{}/api2/json/nodes/{}/qemu/{}/status/shutdown",
            self.base_url, node, vmid
        );

        let response = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .context("Failed to send VM shutdown request")?;

        if response.status() == StatusCode::OK {
            let task: TaskUpid = response
                .json()
                .await
                .context("Failed to parse shutdown response")?;
            match self.wait_for_task(node, &task.data).await {
                Ok(()) => Ok(()),
                Err(e) => {
                    // Treat "not running" as success
                    let msg = e.to_string();
                    if msg.contains("not running") {
                        log::debug!("VM {vmid} is already stopped");
                        Ok(())
                    } else {
                        Err(e)
                    }
                }
            }
        } else {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("unknown error"));
            bail!("VM shutdown failed ({status}): {body}");
        }
    }

    /// Force stop a VM (immediate power off). Returns Ok if VM stops successfully or is already stopped.
    pub async fn vm_stop(&self, node: &str, vmid: u32) -> Result<()> {
        let url = format!(
            "{}/api2/json/nodes/{}/qemu/{}/status/stop",
            self.base_url, node, vmid
        );

        let response = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .context("Failed to send VM stop request")?;

        if response.status() == StatusCode::OK {
            let task: TaskUpid = response
                .json()
                .await
                .context("Failed to parse stop response")?;
            match self.wait_for_task(node, &task.data).await {
                Ok(()) => Ok(()),
                Err(e) => {
                    // Treat "not running" as success
                    let msg = e.to_string();
                    if msg.contains("not running") {
                        log::debug!("VM {vmid} is already stopped");
                        Ok(())
                    } else {
                        Err(e)
                    }
                }
            }
        } else {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("unknown error"));
            bail!("VM stop failed ({status}): {body}");
        }
    }

    /// Wait for a Proxmox task to complete.
    async fn wait_for_task(&self, node: &str, upid: &str) -> Result<()> {
        let url = format!(
            "{}/api2/json/nodes/{}/tasks/{}/status",
            self.base_url, node, upid
        );

        loop {
            let response = self
                .client
                .get(&url)
                .header("Authorization", self.auth_header())
                .send()
                .await
                .context("Failed to check task status")?;

            if response.status() != StatusCode::OK {
                let status = response.status();
                let body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| String::from("unknown error"));
                bail!("Failed to get task status ({status}): {body}");
            }

            let status: ApiResponse<TaskStatus> = response
                .json()
                .await
                .context("Failed to parse task status")?;

            match status.data.status.as_str() {
                "stopped" => {
                    // Task completed - check exit status
                    if let Some(exit) = &status.data.exitstatus {
                        if exit == "OK" {
                            return Ok(());
                        }
                        bail!("Task failed with exit status: {exit}");
                    }
                    return Ok(());
                }
                "running" => {
                    // Task still running - wait and poll again
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                other => {
                    bail!("Unknown task status: {other}");
                }
            }
        }
    }

    /// Get the current status of a VM.
    pub async fn get_vm_status(&self, node: &str, vmid: u32) -> Result<String> {
        let url = format!(
            "{}/api2/json/nodes/{}/qemu/{}/status/current",
            self.base_url, node, vmid
        );

        let response = self
            .client
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .context("Failed to get VM status")?;

        if response.status() == StatusCode::OK {
            let status: ApiResponse<VmStatus> =
                response.json().await.context("Failed to parse VM status")?;
            Ok(status.data.status)
        } else {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("unknown error"));
            bail!("Failed to get VM status ({status}): {body}");
        }
    }

    /// Wait for VM to reach a specific status (e.g., "running" or "stopped").
    pub async fn wait_for_vm_status(
        &self,
        node: &str,
        vmid: u32,
        target_status: &str,
        timeout_secs: u64,
    ) -> Result<()> {
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(timeout_secs);

        loop {
            let status = self.get_vm_status(node, vmid).await?;
            if status == target_status {
                return Ok(());
            }

            if start.elapsed() > timeout {
                bail!(
                    "Timeout waiting for VM {vmid} to reach status '{target_status}' (current: '{status}')"
                );
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}
