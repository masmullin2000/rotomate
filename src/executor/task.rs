use std::time::Instant;

use anyhow::Result;

use crate::config::{ExecCmdline, ShellType, Task};
use crate::shell::get_shell;
use crate::ssh::{CommandResult, DEFAULT_CHUNK_SIZE, Session};

use super::result::{CopyResult, DeleteResult, ProxmoxResult, TaskResult};

/// Parse a copy spec string into (local, remote) paths.
/// eg. `- /path/to/local /path/to/remote` -> `Option<"/path/to/local", "/path/to/remote">`
fn parse_copy_spec(spec: &str) -> Option<(&str, &str)> {
    let parts: Vec<&str> = spec.splitn(2, ' ').collect();
    if parts.len() == 2 {
        Some((parts[0], parts[1]))
    } else {
        None
    }
}

/// Execute file upload operations (local -> remote) and return results.
async fn execute_uploads(
    session: &mut Session,
    upload: &[String],
    stop_on_error: bool,
) -> Vec<CopyResult> {
    let mut uploads = Vec::new();

    for copy_spec in upload {
        let copy_result = match parse_copy_spec(copy_spec) {
            Some((local_path, remote_path)) => session
                .copy_file_to_remote(local_path, remote_path, DEFAULT_CHUNK_SIZE)
                .await
                .map_or_else(
                    |e| CopyResult {
                        local_path: local_path.to_string(),
                        remote_path: remote_path.to_string(),
                        result: Err(e.to_string()),
                    },
                    |bytes| CopyResult {
                        local_path: local_path.to_string(),
                        remote_path: remote_path.to_string(),
                        result: Ok(bytes),
                    },
                ),
            None => CopyResult {
                local_path: copy_spec.clone(),
                remote_path: String::new(),
                result: Err(format!("Invalid copy spec: {copy_spec}")),
            },
        };

        let failed = !copy_result.success();
        uploads.push(copy_result);

        if failed && stop_on_error {
            return uploads;
        }
    }

    uploads
}

/// Execute file download operations (remote -> local) and return results.
async fn execute_downloads(
    session: &mut Session,
    download: &[String],
    stop_on_error: bool,
) -> Vec<CopyResult> {
    let mut downloads = Vec::new();

    for copy_spec in download {
        let copy_result = match parse_copy_spec(copy_spec) {
            Some((remote_path, local_path)) => session
                .copy_file_from_remote(remote_path, local_path)
                .await
                .map_or_else(
                    |e| CopyResult {
                        local_path: local_path.to_string(),
                        remote_path: remote_path.to_string(),
                        result: Err(e.to_string()),
                    },
                    |bytes| CopyResult {
                        local_path: local_path.to_string(),
                        remote_path: remote_path.to_string(),
                        result: Ok(bytes),
                    },
                ),
            None => CopyResult {
                local_path: String::new(),
                remote_path: copy_spec.clone(),
                result: Err(format!("Invalid copy spec: {copy_spec}")),
            },
        };

        let failed = !copy_result.success();
        downloads.push(copy_result);

        if failed && stop_on_error {
            return downloads;
        }
    }

    downloads
}

/// Execute remote file deletions and return results.
async fn execute_deletes(
    session: &mut Session,
    delete_remote: &[String],
    stop_on_error: bool,
) -> Vec<DeleteResult> {
    let mut deletes = Vec::new();

    for remote_path in delete_remote {
        let delete_result = session.delete_remote_file(remote_path).await.map_or_else(
            |e| DeleteResult {
                remote_path: remote_path.clone(),
                result: Err(e.to_string()),
            },
            |()| DeleteResult {
                remote_path: remote_path.clone(),
                result: Ok(()),
            },
        );

        let failed = !delete_result.success();
        deletes.push(delete_result);

        if failed && stop_on_error {
            return deletes;
        }
    }

    deletes
}

/// Execute local file deletions and return results.
fn execute_local_deletes(delete_local: &[String], stop_on_error: bool) -> Vec<DeleteResult> {
    let mut deletes = Vec::new();

    for local_path in delete_local {
        let delete_result = std::fs::remove_file(local_path).map_or_else(
            |e| DeleteResult {
                remote_path: local_path.clone(),
                result: Err(e.to_string()),
            },
            |()| DeleteResult {
                remote_path: local_path.clone(),
                result: Ok(()),
            },
        );

        let failed = !delete_result.success();
        deletes.push(delete_result);

        if failed && stop_on_error {
            return deletes;
        }
    }

    deletes
}

/// Execute commands and return results.
///
/// When `on_output` is provided and `verbose` is true, output is streamed line-by-line
/// in real-time as it arrives from the remote host.
#[allow(clippy::too_many_arguments)]
async fn execute_commands<F>(
    session: &mut Session,
    exec_cmdline: &ExecCmdline,
    shell_type: ShellType,
    become_root: bool,
    privilege_password: Option<&str>,
    stop_on_error: bool,
    verbose: bool,
    mut on_output: Option<F>,
) -> Vec<CommandResult>
where
    F: FnMut(&str, bool),
{
    let shell = get_shell(shell_type);
    let mut commands = Vec::new();

    for command in exec_cmdline.as_commands() {
        let result = if become_root {
            let (wrapped_cmd, stdin_data) = shell.wrap_privileged(command, privilege_password);
            let stdin_bytes = stdin_data.as_ref().map(String::as_bytes);

            if verbose {
                session
                    .call_with_streaming(&wrapped_cmd, stdin_bytes, on_output.as_mut())
                    .await
                    .unwrap_or_else(|e| CommandResult {
                        command: command.to_string(),
                        exit_code: 1,
                        stdout: String::new(),
                        stderr: format!("Failed to execute privileged command: {e}"),
                    })
            } else {
                session
                    .call_with_stdin(&wrapped_cmd, stdin_bytes)
                    .await
                    .unwrap_or_else(|e| CommandResult {
                        command: command.to_string(),
                        exit_code: 1,
                        stdout: String::new(),
                        stderr: format!("Failed to execute privileged command: {e}"),
                    })
            }
        } else {
            let wrapped_cmd = shell.wrap_command(command);
            if verbose {
                session
                    .call_with_streaming(&wrapped_cmd, None, on_output.as_mut())
                    .await
                    .unwrap_or_else(|e| CommandResult {
                        command: command.to_string(),
                        exit_code: 1,
                        stdout: String::new(),
                        stderr: format!("Failed to execute command: {e}"),
                    })
            } else {
                session
                    .call(&wrapped_cmd)
                    .await
                    .unwrap_or_else(|e| CommandResult {
                        command: command.to_string(),
                        exit_code: 1,
                        stdout: String::new(),
                        stderr: format!("Failed to execute command: {e}"),
                    })
            }
        };

        let failed = !result.success();
        commands.push(result);

        if failed && stop_on_error {
            return commands;
        }
    }

    commands
}

/// Execute a task on an already-connected SSH session.
///
/// # Arguments
/// * `session` - The SSH session to execute commands on
/// * `task` - The task to execute
/// * `shell_type` - The shell type to use for command wrapping
/// * `privilege_password` - Password for privilege escalation (sudo/admin)
/// * `proxmox_results` - Results from any Proxmox commands already executed
/// * `proxmox_stopped_early` - Whether Proxmox execution stopped early due to error
/// * `on_output` - Optional callback for streaming remote command output line-by-line
#[allow(clippy::too_many_lines)]
pub async fn execute_task<F>(
    session: &mut Session,
    task: &Task,
    shell_type: ShellType,
    privilege_password: Option<&str>,
    proxmox_results: Vec<ProxmoxResult>,
    proxmox_stopped_early: bool,
    on_output: Option<F>,
) -> Result<TaskResult>
where
    F: FnMut(&str, bool),
{
    let start_time = Instant::now();
    let verbose = task.verbose.unwrap_or(false);
    let capture_output = task.capture_output.unwrap_or(false);
    let inactivity_timeout = task.inactivity_timeout();

    // If proxmox stopped early, return immediately with those results
    if proxmox_stopped_early {
        return Ok(TaskResult {
            task_name: task.name.clone(),
            proxmox: proxmox_results,
            local_commands: Vec::new(),
            uploads: Vec::new(),
            commands: Vec::new(),
            downloads: Vec::new(),
            deletes: Vec::new(),
            local_deletes: Vec::new(),
            stopped_early: true,
            verbose,
            capture_output,
            inactivity_timeout,
            duration_secs: start_time.elapsed().as_secs_f64(),
        });
    }

    // 1. Execute host commands locally (before any remote operations)
    // Use spawn_blocking to avoid blocking the async runtime
    let local_cmd = task.local_command.clone();
    let stop_on_err = task.stop_on_error;
    let local_commands = tokio::task::spawn_blocking(move || {
        execute_local_commands(&local_cmd, stop_on_err, verbose)
    })
    .await
    .unwrap_or_default();
    if local_commands.iter().any(|c| !c.success()) && task.stop_on_error {
        return Ok(TaskResult {
            task_name: task.name.clone(),
            proxmox: proxmox_results,
            local_commands,
            uploads: Vec::new(),
            commands: Vec::new(),
            downloads: Vec::new(),
            deletes: Vec::new(),
            local_deletes: Vec::new(),
            stopped_early: true,
            verbose,
            capture_output,
            inactivity_timeout,
            duration_secs: start_time.elapsed().as_secs_f64(),
        });
    }

    // 2. Upload files to remote
    let uploads = execute_uploads(session, &task.upload, task.stop_on_error).await;
    if uploads.iter().any(|c| !c.success()) && task.stop_on_error {
        return Ok(TaskResult {
            task_name: task.name.clone(),
            proxmox: proxmox_results,
            local_commands,
            uploads,
            commands: Vec::new(),
            downloads: Vec::new(),
            deletes: Vec::new(),
            local_deletes: Vec::new(),
            stopped_early: true,
            verbose,
            capture_output,
            inactivity_timeout,
            duration_secs: start_time.elapsed().as_secs_f64(),
        });
    }

    // 3. Execute remote commands
    let commands = execute_commands(
        session,
        &task.remote_command,
        shell_type,
        task.become_root,
        privilege_password,
        task.stop_on_error,
        verbose,
        on_output,
    )
    .await;

    if commands.iter().any(|c| !c.success()) && task.stop_on_error {
        return Ok(TaskResult {
            task_name: task.name.clone(),
            proxmox: proxmox_results,
            local_commands,
            uploads,
            commands,
            downloads: Vec::new(),
            deletes: Vec::new(),
            local_deletes: Vec::new(),
            stopped_early: true,
            verbose,
            capture_output,
            inactivity_timeout,
            duration_secs: start_time.elapsed().as_secs_f64(),
        });
    }

    // 4. Download files from remote
    let downloads = execute_downloads(session, &task.download, task.stop_on_error).await;
    if downloads.iter().any(|c| !c.success()) && task.stop_on_error {
        return Ok(TaskResult {
            task_name: task.name.clone(),
            proxmox: proxmox_results,
            local_commands,
            uploads,
            commands,
            downloads,
            deletes: Vec::new(),
            local_deletes: Vec::new(),
            stopped_early: true,
            verbose,
            capture_output,
            inactivity_timeout,
            duration_secs: start_time.elapsed().as_secs_f64(),
        });
    }

    // 5. Delete remote files
    let deletes = execute_deletes(session, &task.delete_remote, task.stop_on_error).await;
    if deletes.iter().any(|d| !d.success()) && task.stop_on_error {
        return Ok(TaskResult {
            task_name: task.name.clone(),
            proxmox: proxmox_results,
            local_commands,
            uploads,
            commands,
            downloads,
            deletes,
            local_deletes: Vec::new(),
            stopped_early: true,
            verbose,
            capture_output,
            inactivity_timeout,
            duration_secs: start_time.elapsed().as_secs_f64(),
        });
    }

    // 6. Delete local files
    let local_deletes = execute_local_deletes(&task.delete_local, task.stop_on_error);
    let stopped_early = task.stop_on_error && local_deletes.iter().any(|d| !d.success());

    Ok(TaskResult {
        task_name: task.name.clone(),
        proxmox: proxmox_results,
        local_commands,
        uploads,
        commands,
        downloads,
        deletes,
        local_deletes,
        stopped_early,
        verbose,
        capture_output,
        inactivity_timeout,
        duration_secs: start_time.elapsed().as_secs_f64(),
    })
}

/// Execute host commands locally with optional real-time output streaming.
/// Always captures output into `CommandResult`; when verbose=true, also streams in real-time.
pub fn execute_local_commands(
    exec_cmdline: &ExecCmdline,
    stop_on_error: bool,
    verbose: bool,
) -> Vec<CommandResult> {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::thread;

    let mut results = Vec::new();

    for cmd in exec_cmdline.as_commands() {
        // Use platform-appropriate shell for local command execution
        #[cfg(unix)]
        let child = Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();

        #[cfg(windows)]
        let child = Command::new("cmd")
            .arg("/c")
            .arg(cmd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();

        let result = match child {
            Ok(mut child) => {
                // Always capture output; stream when verbose
                let (tx, rx) = mpsc::channel();

                let stdout_tx = tx.clone();
                let stdout_verbose = verbose;
                let stdout_handle = child.stdout.take().map(|stdout| {
                    thread::spawn(move || {
                        let mut content = String::new();
                        let reader = BufReader::new(stdout);
                        for line in reader.lines().map_while(Result::ok) {
                            if stdout_verbose {
                                let _ = stdout_tx.send((false, line.clone()));
                            }
                            content.push_str(&line);
                            content.push('\n');
                        }
                        content
                    })
                });

                let stderr_tx = tx;
                let stderr_verbose = verbose;
                let stderr_handle = child.stderr.take().map(|stderr| {
                    thread::spawn(move || {
                        let mut content = String::new();
                        let reader = BufReader::new(stderr);
                        for line in reader.lines().map_while(Result::ok) {
                            if stderr_verbose {
                                let _ = stderr_tx.send((true, line.clone()));
                            }
                            content.push_str(&line);
                            content.push('\n');
                        }
                        content
                    })
                });

                // Stream output in real-time if verbose
                if verbose {
                    for (is_stderr, line) in rx {
                        if is_stderr {
                            eprintln!("{line}");
                        } else {
                            println!("{line}");
                        }
                    }
                }

                // Wait for output threads to complete and collect content
                let stdout_content = stdout_handle
                    .and_then(|h| h.join().ok())
                    .unwrap_or_default();
                let stderr_content = stderr_handle
                    .and_then(|h| h.join().ok())
                    .unwrap_or_default();

                let status = child.wait();
                let exit_code = status.map_or(1, |s| s.code().unwrap_or(1));

                CommandResult {
                    command: cmd.to_string(),
                    #[allow(clippy::cast_sign_loss)]
                    exit_code: exit_code as u32,
                    stdout: stdout_content,
                    stderr: stderr_content,
                }
            }
            Err(e) => CommandResult {
                command: cmd.to_string(),
                exit_code: 1,
                stdout: String::new(),
                stderr: format!("Failed to execute command: {e}"),
            },
        };

        let failed = !result.success();
        results.push(result);

        if failed && stop_on_error {
            return results;
        }
    }

    results
}
