use colored::Colorize;

use crate::executor::{HostResult, TaskExecution, TaskGroupResult};

/// Lines containing these patterns are filtered from output.
const FILTERED_PATTERNS: &[&str] = &["stty: 'standard input': Inappropriate ioctl for device"];

/// Color palette for host output.
const COLORS: &[&str] = &["cyan", "green", "yellow", "magenta", "blue"];

/// Formatter for displaying execution output.
pub struct OutputFormatter {
    colors: std::collections::HashMap<String, &'static str>,
}

impl OutputFormatter {
    /// Create a new formatter with colors assigned to hosts.
    pub fn new(host_names: &[String]) -> Self {
        let mut colors = std::collections::HashMap::new();
        for (i, name) in host_names.iter().enumerate() {
            colors.insert(name.clone(), COLORS[i % COLORS.len()]);
        }
        Self { colors }
    }

    /// Width for host name prefix alignment.
    const PREFIX_WIDTH: usize = 15;

    /// Format a line of output from a host with aligned prefix.
    pub fn format_line(&self, host_name: &str, line: &str) -> String {
        let color = self.colors.get(host_name).unwrap_or(&"white");
        let prefix = format!("[{host_name}]");
        let padded_prefix = format!("{prefix:<width$}", width = Self::PREFIX_WIDTH);
        let colored_prefix = match *color {
            "cyan" => padded_prefix.cyan(),
            "green" => padded_prefix.green(),
            "yellow" => padded_prefix.yellow(),
            "magenta" => padded_prefix.magenta(),
            "blue" => padded_prefix.blue(),
            _ => padded_prefix.white(),
        };
        format!("{colored_prefix}{line}")
    }

    /// Check if a line should be filtered from output.
    fn should_filter_line(line: &str) -> bool {
        FILTERED_PATTERNS
            .iter()
            .any(|pattern| line.contains(pattern))
    }

    /// Print a single streamed line with host prefix.
    ///
    /// Used for real-time output streaming when verbose mode is enabled.
    /// Stderr lines are printed in red. Some noise lines are filtered.
    pub fn print_streaming_line(&self, host_name: &str, line: &str, is_stderr: bool) {
        if Self::should_filter_line(line) {
            return;
        }
        let formatted = self.format_line(host_name, line);
        if is_stderr {
            println!("{}", formatted.red());
        } else {
            println!("{formatted}");
        }
    }

    /// Print a task start marker for a host.
    pub fn print_task_start(&self, host_name: &str, task_name: &str, inactivity_timeout: u64) {
        let color = self.colors.get(host_name).unwrap_or(&"white");
        let marker = format!("#[{host_name}: START -> {task_name} (timeout: {inactivity_timeout}s)]#");
        let colored = match *color {
            "cyan" => marker.cyan(),
            "green" => marker.green(),
            "yellow" => marker.yellow(),
            "magenta" => marker.magenta(),
            "blue" => marker.blue(),
            _ => marker.white(),
        };
        println!("{colored}");
    }

    /// Format a duration in seconds to a human-readable string.
    fn format_duration(secs: f64) -> String {
        if secs < 60.0 {
            format!("{secs:.2}s")
        } else {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let mins = (secs / 60.0).floor() as u64;
            #[allow(clippy::cast_precision_loss)]
            let remaining_secs = (mins as f64).mul_add(-60.0, secs);
            format!("{mins}m {remaining_secs:.2}s")
        }
    }

    /// Print a task end marker for a host with result summary.
    pub fn print_task_end(&self, host_name: &str, task_name: &str, result: &HostResult) {
        let color = self.colors.get(host_name).unwrap_or(&"white");

        let status_text = match &result.result {
            Ok(task_result) => {
                // Print any Proxmox errors before the END marker
                for proxmox_result in &task_result.proxmox {
                    if let Err(e) = &proxmox_result.result {
                        let msg = format!(
                            "Proxmox {} failed (node={} vmid={}): {e}",
                            proxmox_result.action, proxmox_result.node, proxmox_result.vmid
                        );
                        let line = self.format_line(host_name, &msg);
                        println!("{}", line.red());
                    }
                }

                let status = if task_result.success() {
                    "SUCCESS"
                } else {
                    "FAILED"
                };
                let mut parts = Vec::new();
                if !task_result.proxmox.is_empty() {
                    parts.push(format!("{} proxmox", task_result.proxmox.len()));
                }
                if !task_result.local_commands.is_empty() {
                    parts.push(format!("{} local", task_result.local_commands.len()));
                }
                if !task_result.uploads.is_empty() {
                    parts.push(format!("{} uploads", task_result.uploads.len()));
                }
                if !task_result.commands.is_empty() {
                    parts.push(format!("{} remote", task_result.commands.len()));
                }
                if !task_result.downloads.is_empty() {
                    parts.push(format!("{} downloads", task_result.downloads.len()));
                }
                if !task_result.deletes.is_empty() {
                    parts.push(format!("{} remote deletes", task_result.deletes.len()));
                }
                if !task_result.local_deletes.is_empty() {
                    parts.push(format!("{} local deletes", task_result.local_deletes.len()));
                }
                let ops = if parts.is_empty() {
                    String::from("no operations")
                } else {
                    parts.join(", ")
                };
                let duration = Self::format_duration(task_result.duration_secs);
                format!(
                    "{status} ({ops}, {} failed) in {duration}",
                    task_result.failed_count()
                )
            }
            Err(e) => format!("FAILED - {e}"),
        };

        let marker = format!("#[{host_name}: END -> {task_name}: {status_text}]#");
        let colored = match *color {
            "cyan" => marker.cyan(),
            "green" => marker.green(),
            "yellow" => marker.yellow(),
            "magenta" => marker.magenta(),
            "blue" => marker.blue(),
            _ => marker.white(),
        };
        println!("{colored}");
    }

    /// Print a task group result summary.
    pub fn print_summary(&self, result: &TaskGroupResult) {
        println!();
        println!("Summary for task group '{}':", result.group_name);
        println!("{}", "=".repeat(40));

        for task_execution in &result.tasks {
            Self::print_task_summary(task_execution);
        }

        println!();
        println!(
            "Total: {} tasks succeeded, {} failed",
            result.success_task_count(),
            result.failed_task_count()
        );
    }

    /// Print a single task execution summary.
    pub fn print_task_summary(task_execution: &TaskExecution) {
        println!();
        println!("  Task '{}':", task_execution.task_name);

        for host_result in &task_execution.hosts {
            Self::print_host_summary(host_result);
        }

        println!(
            "  Task total: {} succeeded, {} failed",
            task_execution.success_count(),
            task_execution.failed_count()
        );
    }

    /// Print a single host's result summary.
    pub fn print_host_summary(result: &HostResult) {
        let status = if result.success() {
            "SUCCESS".green()
        } else {
            "FAILED".red()
        };

        match &result.result {
            Ok(task_result) => {
                let proxmox_count = task_result.proxmox.len();
                let host_cmd_count = task_result.local_commands.len();
                let upload_count = task_result.uploads.len();
                let cmd_count = task_result.commands.len();
                let download_count = task_result.downloads.len();
                let delete_count = task_result.deletes.len();
                let local_delete_count = task_result.local_deletes.len();
                let failed = task_result.failed_count();

                let mut parts = Vec::new();
                if proxmox_count > 0 {
                    parts.push(format!("{proxmox_count} proxmox"));
                }
                if host_cmd_count > 0 {
                    parts.push(format!("{host_cmd_count} local"));
                }
                if upload_count > 0 {
                    parts.push(format!("{upload_count} uploads"));
                }
                if cmd_count > 0 {
                    parts.push(format!("{cmd_count} remote"));
                }
                if download_count > 0 {
                    parts.push(format!("{download_count} downloads"));
                }
                if delete_count > 0 {
                    parts.push(format!("{delete_count} remote deletes"));
                }
                if local_delete_count > 0 {
                    parts.push(format!("{local_delete_count} local deletes"));
                }
                let ops = if parts.is_empty() {
                    String::from("no operations")
                } else {
                    parts.join(", ")
                };

                println!(
                    "  {}:\t{} ({}, {} failed)",
                    result.host_name, status, ops, failed
                );
            }
            Err(error) => {
                println!("  {}:\t{} - {}", result.host_name, status, error);
            }
        }
    }

    /// Print detailed output for all tasks in a task group, with start/end markers.
    pub fn print_detailed(&self, result: &TaskGroupResult) {
        for task_execution in &result.tasks {
            for host_result in &task_execution.hosts {
                let inactivity_timeout = host_result
                    .result
                    .as_ref()
                    .map_or(0, |tr| tr.inactivity_timeout);
                self.print_task_start(
                    &host_result.host_name,
                    &task_execution.task_name,
                    inactivity_timeout,
                );
                self.print_host_detailed(host_result);
                self.print_task_end(&host_result.host_name, &task_execution.task_name, host_result);
            }
        }
    }

    /// Print detailed output for a single host.
    #[allow(clippy::too_many_lines)]
    pub fn print_host_detailed(&self, host_result: &HostResult) {
        let Ok(task_result) = &host_result.result else {
            return;
        };

        // Print Proxmox results
        for proxmox_result in &task_result.proxmox {
            let msg = match &proxmox_result.result {
                Ok(()) => format!(
                    "Proxmox {}: node={} vmid={}",
                    proxmox_result.action, proxmox_result.node, proxmox_result.vmid
                ),
                Err(e) => format!(
                    "Proxmox {} failed (node={} vmid={}): {e}",
                    proxmox_result.action, proxmox_result.node, proxmox_result.vmid
                ),
            };
            let line = self.format_line(&host_result.host_name, &msg);
            if proxmox_result.success() {
                println!("{}", line.green());
            } else {
                println!("{}", line.red());
            }
        }

        // Print host command results (local commands)
        for cmd_result in &task_result.local_commands {
            // Print stdout
            for line in cmd_result.stdout.lines() {
                if Self::should_filter_line(line) {
                    continue;
                }
                let line = self.format_line(&host_result.host_name, line);
                println!("{line}");
            }
            // Print stderr
            for line in cmd_result.stderr.lines() {
                if Self::should_filter_line(line) {
                    continue;
                }
                let line = self.format_line(&host_result.host_name, line);
                println!("{}", line.red());
            }
        }

        // Print upload results
        for copy_result in &task_result.uploads {
            let msg = copy_result.result.as_ref().map_or_else(
                |e| {
                    format!(
                        "Upload failed {local} -> {remote}: {e}",
                        local = copy_result.local_path,
                        remote = copy_result.remote_path
                    )
                },
                |bytes| {
                    format!(
                        "Uploaded {local} -> {remote} ({bytes} bytes)",
                        local = copy_result.local_path,
                        remote = copy_result.remote_path
                    )
                },
            );
            let line = self.format_line(&host_result.host_name, &msg);
            if copy_result.success() {
                println!("{}", line.green());
            } else {
                println!("{}", line.red());
            }
        }

        // Print command results
        for cmd_result in &task_result.commands {
            // Print stdout
            for line in cmd_result.stdout.lines() {
                if !Self::should_filter_line(line) {
                    println!("{}", self.format_line(&host_result.host_name, line));
                }
            }
            // Print stderr
            for line in cmd_result.stderr.lines() {
                if !Self::should_filter_line(line) {
                    let formatted = self.format_line(&host_result.host_name, line);
                    println!("{}", formatted.red());
                }
            }
        }

        // Print download results
        for copy_result in &task_result.downloads {
            let msg = match &copy_result.result {
                Ok(bytes) => format!(
                    "Downloaded {} -> {} ({bytes} bytes)",
                    copy_result.remote_path, copy_result.local_path
                ),
                Err(e) => format!(
                    "Download failed {} -> {}: {e}",
                    copy_result.remote_path, copy_result.local_path
                ),
            };
            let line = self.format_line(&host_result.host_name, &msg);
            if copy_result.success() {
                println!("{}", line.green());
            } else {
                println!("{}", line.red());
            }
        }

        // Print delete results
        for delete_result in &task_result.deletes {
            let msg = match &delete_result.result {
                Ok(()) => format!("Deleted remote {}", delete_result.remote_path),
                Err(e) => format!("Delete remote failed {}: {e}", delete_result.remote_path),
            };
            let line = self.format_line(&host_result.host_name, &msg);
            if delete_result.success() {
                println!("{}", line.green());
            } else {
                println!("{}", line.red());
            }
        }

        // Print local delete results
        for delete_result in &task_result.local_deletes {
            let msg = match &delete_result.result {
                Ok(()) => format!("Deleted local {}", delete_result.remote_path),
                Err(e) => format!("Delete local failed {}: {e}", delete_result.remote_path),
            };
            let line = self.format_line(&host_result.host_name, &msg);
            if delete_result.success() {
                println!("{}", line.green());
            } else {
                println!("{}", line.red());
            }
        }
    }
}
