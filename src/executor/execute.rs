use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use futures::stream::{self, FuturesUnordered, StreamExt};
use log::debug;
use tokio::sync::watch;

use crate::config::template::EnvironmentExt;
use crate::config::{
    BuiltinContext, Config, ExecCmdline, RenderedTaskFields, TaskGroup, TaskGroupItem, render_steps,
};
use crate::output::OutputFormatter;
use crate::proxmox::ProxmoxClient;

use super::result::{HostResult, TaskExecution, TaskGroupResult, TaskResult};
use super::task::execute_local_commands;

/// Type alias for streaming output callback.
type StreamingCallback = Option<Box<dyn FnMut(&str, bool) + Send>>;

/// Executor for running tasks across multiple hosts.
pub struct Executor {
    config: Config,
    sudo_password: Option<String>,
    verbose: bool,
    capture_output: bool,
    proxmox_client: Option<ProxmoxClient>,
    /// Timestamp for builtin template variable (YYYY-MM-DD_HH-MM-SS format).
    timestamp: String,
}

impl Executor {
    /// Create a new executor with the given configuration.
    pub fn new(
        config: Config,
        sudo_password: Option<String>,
        verbose: bool,
        capture_output: bool,
        proxmox_client: Option<ProxmoxClient>,
    ) -> Self {
        let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
        Self {
            config,
            sudo_password,
            verbose,
            capture_output,
            proxmox_client,
            timestamp,
        }
    }

    /// Run all configured task groups and return results.
    /// Task groups without dependencies start immediately in parallel.
    /// Task groups with dependencies wait for all dependencies to complete first.
    /// Tasks within each task group execute sequentially.
    /// Calls `on_host_start` when a host begins a task, and `on_host_complete` when finished.
    /// Calls `on_group_start`/`on_group_complete` at the boundaries of each task group.
    pub async fn run_all<S, F, GS, GF>(
        &self,
        on_host_start: S,
        on_host_complete: F,
        on_group_start: GS,
        on_group_complete: GF,
    ) -> Vec<TaskGroupResult>
    where
        S: Fn(&str, &str, u64) + Send + Sync,
        F: Fn(&str, &HostResult) + Send + Sync,
        GS: Fn(&str) + Send + Sync,
        GF: Fn(&TaskGroupResult) + Send + Sync,
    {
        let on_host_start = Arc::new(on_host_start);
        let on_host_complete = Arc::new(on_host_complete);
        let on_group_start = Arc::new(on_group_start);
        let on_group_complete = Arc::new(on_group_complete);
        let groups = &self.config.task_groups;

        // Map group key -> index for dependency lookup
        let key_to_idx: HashMap<&str, usize> = groups
            .iter()
            .enumerate()
            .map(|(i, g)| (g.key.as_str(), i))
            .collect();

        // Completion signals for each group (watch channels)
        // Sender signals completion, receivers wait for it
        let channels: Vec<(watch::Sender<bool>, watch::Receiver<bool>)> =
            (0..groups.len()).map(|_| watch::channel(false)).collect();

        // Extract senders and receivers
        let senders: Vec<watch::Sender<bool>> = channels.iter().map(|(s, _)| s.clone()).collect();
        let receivers: Vec<watch::Receiver<bool>> =
            channels.iter().map(|(_, r)| r.clone()).collect();

        // Spawn all groups with dependency handling
        let futures: FuturesUnordered<_> = groups
            .iter()
            .enumerate()
            .map(|(idx, task_group)| {
                let config = self.config.clone();
                let sudo_password = self.sudo_password.clone();
                let proxmox_client = self.proxmox_client.clone();
                let on_host_start = on_host_start.clone();
                let on_host_complete = on_host_complete.clone();
                let on_group_start = on_group_start.clone();
                let on_group_complete = on_group_complete.clone();
                let global_verbose = self.verbose;
                let senders = senders.clone();
                let receivers = receivers.clone();
                let task_group = task_group.clone();

                // Get indices of dependencies
                let dep_indices: Vec<usize> = task_group
                    .depends_on
                    .iter()
                    .filter_map(|key| key_to_idx.get(key.as_str()).copied())
                    .collect();

                let global_capture_output = self.capture_output;
                let timestamp = self.timestamp.clone();
                async move {
                    // Wait for all dependencies to complete
                    for dep_idx in dep_indices {
                        let mut rx = receivers[dep_idx].clone();
                        // Wait until the dependency signals completion
                        while !*rx.borrow() {
                            // Ignore errors (sender dropped means it completed)
                            let _ = rx.changed().await;
                        }
                    }

                    on_group_start(&task_group.name);

                    // Execute this group
                    let result = Self::execute_task_group(
                        &config,
                        sudo_password.as_ref(),
                        proxmox_client.as_ref(),
                        &task_group,
                        &*on_host_start,
                        &*on_host_complete,
                        global_verbose,
                        global_capture_output,
                        &timestamp,
                    )
                    .await;

                    on_group_complete(&result);

                    // Signal completion to all waiters
                    let _ = senders[idx].send(true);

                    result
                }
            })
            .collect();

        futures.collect::<Vec<TaskGroupResult>>().await
    }

    /// Execute a single task group (tasks execute sequentially within the group).
    #[allow(clippy::too_many_arguments)]
    async fn execute_task_group<S, F>(
        config: &Config,
        sudo_password: Option<&String>,
        proxmox_client: Option<&ProxmoxClient>,
        task_group: &TaskGroup,
        on_host_start: S,
        on_host_complete: F,
        global_verbose: bool,
        global_capture_output: bool,
        timestamp: &str,
    ) -> TaskGroupResult
    where
        S: Fn(&str, &str, u64) + Send + Sync,
        F: Fn(&str, &HostResult) + Send + Sync,
    {
        let start_time = Instant::now();
        let task_group_name = &task_group.name;
        let task_group_vars = &task_group.vars;
        let tasks = stream::iter(&task_group.tasks)
            .then(|task_item| {
                Self::execute_task(
                    config,
                    sudo_password,
                    proxmox_client,
                    task_item,
                    task_group_name,
                    task_group_vars,
                    &on_host_start,
                    &on_host_complete,
                    global_verbose,
                    global_capture_output,
                    timestamp,
                )
            })
            .collect()
            .await;

        TaskGroupResult {
            group_name: task_group.name.clone(),
            tasks,
            duration_secs: start_time.elapsed().as_secs_f64(),
        }
    }

    /// Run a specific task on its configured hosts.
    /// Searches all runs for the task.
    /// If the task only has host commands (no remote operations), runs locally.
    /// Calls `on_host_start` when a host begins, and `on_host_complete` when finished.
    pub async fn run_task<S, F>(
        &self,
        task_name: &str,
        on_host_start: S,
        on_host_complete: F,
    ) -> Option<TaskExecution>
    where
        S: Fn(&str, &str, u64) + Send + Sync,
        F: Fn(&str, &HostResult) + Send + Sync,
    {
        // First check if the task exists
        let task = self.config.get_task(task_name)?;

        // Check if task is local-only (only local_command, no remote operations, no proxmox)
        let is_local_only = if task.has_steps() {
            task.proxmox_command.is_empty()
                && task.steps.iter().all(crate::config::TaskStep::is_local)
        } else {
            !task.local_command.is_empty()
                && task.proxmox_command.is_empty()
                && task.remote_command.is_empty()
                && task.upload.is_empty()
                && task.download.is_empty()
        };

        let verbose = task.verbose.unwrap_or(false) || self.verbose;
        let capture_output = task.capture_output.unwrap_or(false) || self.capture_output;
        let timeout = task.timeout();

        if is_local_only {
            // Notify start
            on_host_start("localhost", task_name, timeout);
            let start_time = Instant::now();

            // Render local commands with builtin context
            let builtin = BuiltinContext {
                host: "localhost".to_string(),
                task_group: String::new(),
                timestamp: self.timestamp.clone(),
            };
            let vars = effective_vars(&self.config.vars, &HashMap::new(), task);
            let (local_cmd, template_error) =
                render_local_commands(&task.local_command, &vars, &builtin);

            // If template rendering failed, return error result
            if let Some(err) = template_error {
                let host_result = HostResult {
                    host_name: "localhost".to_string(),
                    result: Err(err),
                };
                on_host_complete(task_name, &host_result);
                return Some(TaskExecution {
                    task_name: task_name.to_string(),
                    hosts: vec![host_result],
                });
            }

            // Execute host commands locally without needing a run
            // Use spawn_blocking to avoid blocking the async runtime
            let stop_on_err = task.stop_on_error;
            let local_commands = tokio::task::spawn_blocking(move || {
                execute_local_commands(&local_cmd, stop_on_err, verbose)
            })
            .await
            .unwrap_or_default();
            let stopped_early = task.stop_on_error && local_commands.iter().any(|c| !c.success());

            let task_result = TaskResult {
                task_name: task_name.to_string(),
                proxmox: Vec::new(),
                local_commands,
                uploads: Vec::new(),
                commands: Vec::new(),
                downloads: Vec::new(),
                deletes: Vec::new(),
                local_deletes: Vec::new(),
                stopped_early,
                verbose,
                capture_output,
                timeout,
                duration_secs: start_time.elapsed().as_secs_f64(),
            };

            // Create a synthetic host result for "localhost"
            let host_result = HostResult {
                host_name: "localhost".to_string(),
                result: Ok(task_result),
            };

            on_host_complete(task_name, &host_result);

            return Some(TaskExecution {
                task_name: task_name.to_string(),
                hosts: vec![host_result],
            });
        }

        // Find the task reference in any task group, preserving the group name and vars
        let (task_group_name, task_group_vars, task_item) = self
            .config
            .task_groups
            .iter()
            .flat_map(|g| g.tasks.iter().map(move |t| (g.name.as_str(), &g.vars, t)))
            .find(|(_, _, t)| t.task_name == task_name)?;

        Some(
            Self::execute_task(
                &self.config,
                self.sudo_password.as_ref(),
                self.proxmox_client.as_ref(),
                task_item,
                task_group_name,
                task_group_vars,
                on_host_start,
                on_host_complete,
                self.verbose,
                self.capture_output,
                &self.timestamp,
            )
            .await,
        )
    }

    /// Execute a single task on multiple hosts in parallel.
    /// Results are streamed as each host completes.
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)]
    async fn execute_task<S, F>(
        config: &Config,
        sudo_password: Option<&String>,
        proxmox_client: Option<&ProxmoxClient>,
        task_item: &TaskGroupItem,
        task_group_name: &str,
        group_vars: &HashMap<String, serde_yaml::Value>,
        on_host_start: S,
        on_host_complete: F,
        global_verbose: bool,
        global_capture_output: bool,
        timestamp: &str,
    ) -> TaskExecution
    where
        S: Fn(&str, &str, u64) + Send + Sync,
        F: Fn(&str, &HostResult) + Send + Sync,
    {
        let Some(task) = config.get_task(&task_item.task_name) else {
            return TaskExecution {
                task_name: task_item.task_name.clone(),
                hosts: vec![],
            };
        };

        let mut host_results = Vec::new();
        let verbose = task.verbose.unwrap_or(false) || global_verbose;
        let capture_output = task.capture_output.unwrap_or(false) || global_capture_output;

        // Choose the right timeout to display: proxmox timeout for proxmox-only tasks,
        // SSH timeout otherwise
        let has_ssh_ops = if task.has_steps() {
            task.steps.iter().any(crate::config::TaskStep::needs_ssh)
        } else {
            !task.remote_command.is_empty()
                || !task.upload.is_empty()
                || !task.download.is_empty()
                || !task.delete_remote.is_empty()
        };
        let timeout = if !has_ssh_ops && !task.proxmox_command.is_empty() {
            proxmox_client.map_or_else(|| task.timeout(), ProxmoxClient::timeout_secs)
        } else {
            task.timeout()
        };

        // Run local commands once (not per-host) — skip when using steps
        // (local_command is part of steps and runs per-host)
        // Use spawn_blocking to avoid blocking the async runtime
        if !task.has_steps() && !task.local_command.is_empty() {
            on_host_start("local", &task_item.task_name, timeout);
            let start_time = Instant::now();

            // Render local commands with builtin context
            let builtin = BuiltinContext {
                host: "local".to_string(),
                task_group: task_group_name.to_string(),
                timestamp: timestamp.to_string(),
            };
            let vars = effective_vars(&config.vars, group_vars, task);
            let (local_cmd, template_error) =
                render_local_commands(&task.local_command, &vars, &builtin);

            // If template rendering failed, report error and stop
            if let Some(err) = template_error {
                let local_result = HostResult {
                    host_name: "local".to_string(),
                    result: Err(err),
                };
                on_host_complete(&task_item.task_name, &local_result);
                host_results.push(local_result);
                return TaskExecution {
                    task_name: task_item.task_name.clone(),
                    hosts: host_results,
                };
            }

            let stop_on_err = task.stop_on_error;
            let local_commands = tokio::task::spawn_blocking(move || {
                execute_local_commands(&local_cmd, stop_on_err, verbose)
            })
            .await
            .unwrap_or_default();
            let stopped_early = task.stop_on_error && local_commands.iter().any(|c| !c.success());

            let local_result = HostResult {
                host_name: "local".to_string(),
                result: Ok(TaskResult {
                    task_name: task_item.task_name.clone(),
                    proxmox: Vec::new(),
                    local_commands,
                    uploads: Vec::new(),
                    commands: Vec::new(),
                    downloads: Vec::new(),
                    deletes: Vec::new(),
                    local_deletes: Vec::new(),
                    stopped_early,
                    verbose,
                    capture_output,
                    timeout,
                    duration_secs: start_time.elapsed().as_secs_f64(),
                }),
            };

            on_host_complete(&task_item.task_name, &local_result);
            let should_stop = stopped_early;
            host_results.push(local_result);

            if should_stop {
                return TaskExecution {
                    task_name: task_item.task_name.clone(),
                    hosts: host_results,
                };
            }
        }

        // Check if task has any remote operations - if not, skip host execution entirely
        let has_remote_ops = if task.has_steps() {
            task.steps.iter().any(crate::config::TaskStep::needs_ssh)
                || !task.proxmox_command.is_empty()
        } else {
            !task.remote_command.is_empty()
                || !task.upload.is_empty()
                || !task.download.is_empty()
                || !task.delete_remote.is_empty()
                || !task.proxmox_command.is_empty()
        };

        if !has_remote_ops {
            return TaskExecution {
                task_name: task_item.task_name.clone(),
                hosts: host_results,
            };
        }

        // Execute on all hosts in parallel using FuturesUnordered
        // to yield results as they complete
        let mut futures: FuturesUnordered<_> = config
            .hosts(task_item)
            .map(|host| {
                // Notify host start before launching async execution
                on_host_start(&host.ctx.name, &task_item.task_name, timeout);

                let mut task = task.clone();
                // Use host-specific sudo password if set, otherwise fall back to global
                task.sudo_password = host
                    .sudo_password
                    .clone()
                    .or_else(|| sudo_password.cloned());
                task.name.clone_from(&task_item.task_name);
                if task.verbose.is_none() {
                    task.verbose = Some(global_verbose);
                }
                if task.capture_output.is_none() {
                    task.capture_output = Some(global_capture_output);
                }

                let remote_commands: Vec<_> = task
                    .remote_command
                    .as_commands()
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                let local_commands: Vec<_> = task
                    .local_command
                    .as_commands()
                    .iter()
                    .map(ToString::to_string)
                    .collect();

                let builtin = BuiltinContext {
                    host: host.ctx.name.clone(),
                    task_group: task_group_name.to_string(),
                    timestamp: timestamp.to_string(),
                };

                // Capture template rendering error to report later
                let vars = effective_vars(&config.vars, group_vars, &task);
                let template_error = if task.has_steps() {
                    // Render steps with host + builtin context
                    match render_steps(&vars, host.context(), &builtin, &task.steps) {
                        Ok(rendered_steps) => {
                            task.steps = rendered_steps;
                            None
                        }
                        Err(e) => Some(e.to_string()),
                    }
                } else {
                    match RenderedTaskFields::try_new(
                        &vars,
                        host.context(),
                        &builtin,
                        &remote_commands,
                        &local_commands,
                        &task.upload,
                        &task.download,
                    ) {
                        Ok(rendered) => {
                            // Apply rendered fields to task
                            task.remote_command = if rendered.remote_command.is_empty() {
                                ExecCmdline::Empty
                            } else {
                                ExecCmdline::List(rendered.remote_command)
                            };
                            task.upload = rendered.upload;
                            task.download = rendered.download;
                            None
                        }
                        Err(e) => Some(e.to_string()),
                    }
                };

                // Clear local_command - already executed once before parallel host execution
                // (not needed for steps tasks - local_command runs per-host via steps)
                if !task.has_steps() {
                    task.local_command = ExecCmdline::Empty;
                }

                let host = host.clone();
                let proxmox_client = proxmox_client.cloned();
                // Create formatter for streaming output
                let host_names: Vec<String> = config.hosts.keys().cloned().collect();
                let formatter = Arc::new(OutputFormatter::new(&host_names));
                let host_name_for_callback = host.ctx.name.clone();

                async move {
                    // Return error if template rendering failed
                    if let Some(err) = template_error {
                        return HostResult {
                            host_name: host.ctx.name.clone(),
                            result: Err(format!("Template rendering failed: {err}")),
                        };
                    }

                    debug!(
                        "Running task '{task}' on host '{host}'",
                        task = task.name,
                        host = host.ctx.name
                    );
                    // Create streaming callback if verbose mode is enabled
                    let callback: StreamingCallback = if task.verbose.unwrap_or(false) {
                        let host_name = host_name_for_callback.clone();
                        Some(Box::new(move |line: &str, is_stderr: bool| {
                            formatter.print_streaming_line(&host_name, line, is_stderr);
                        }))
                    } else {
                        None
                    };
                    host.execute(&task, proxmox_client.as_ref(), callback).await
                }
            })
            .collect();

        // Collect results as they complete, calling the callback for each
        while let Some(result) = futures.next().await {
            on_host_complete(&task_item.task_name, &result);
            host_results.push(result);
        }

        TaskExecution {
            task_name: task_item.task_name.clone(),
            hosts: host_results,
        }
    }
}

/// Compute effective vars for a task: config vars → group vars → task's own vars.
/// Group vars come from the file that defined the invoking `task_group`.
/// Task's own vars are only the keys explicitly declared in the task's file
/// (not inherited), so the group's scope takes precedence for inherited keys.
fn effective_vars(
    config_vars: &HashMap<String, serde_yaml::Value>,
    group_vars: &HashMap<String, serde_yaml::Value>,
    task: &crate::config::schema::Task,
) -> HashMap<String, serde_yaml::Value> {
    let mut vars = config_vars.clone();
    vars.extend(group_vars.iter().map(|(k, v)| (k.clone(), v.clone())));
    // Layer on only the task's own declared vars (not inherited from parent)
    for key in &task.own_var_keys {
        if let Some(val) = task.vars.get(key) {
            vars.insert(key.clone(), val.clone());
        }
    }
    vars
}

/// Render local commands with vars and builtin context.
/// Returns (rendered commands, optional error message).
fn render_local_commands(
    local_command: &ExecCmdline,
    vars: &HashMap<String, serde_yaml::Value>,
    builtin: &BuiltinContext,
) -> (ExecCmdline, Option<String>) {
    use minijinja::{Environment, Value};

    let commands: Vec<String> = local_command
        .as_commands()
        .iter()
        .map(ToString::to_string)
        .collect();

    if commands.is_empty() {
        return (ExecCmdline::Empty, None);
    }

    let env = Environment::build();

    // Build context with vars and builtin
    let mut context_map: HashMap<String, Value> = vars
        .iter()
        .map(|(k, v)| (k.clone(), yaml_to_jinja_value(v)))
        .collect();

    let builtin_value = Value::from_serialize(builtin);
    context_map.insert("builtin".to_string(), builtin_value);

    let context = Value::from_serialize(&context_map);

    // Render each command, collecting errors
    let mut render_error: Option<String> = None;
    let rendered: Vec<String> = commands
        .iter()
        .map(|cmd| {
            if !cmd.contains("{{") && !cmd.contains("{%") {
                return cmd.clone();
            }
            match env.render_str(cmd, &context) {
                Ok(rendered) => rendered,
                Err(e) => {
                    if render_error.is_none() {
                        render_error = Some(format!("Template rendering failed: {e}"));
                    }
                    cmd.clone()
                }
            }
        })
        .collect();

    (ExecCmdline::List(rendered), render_error)
}

/// Convert a `serde_yaml::Value` to minijinja Value.
fn yaml_to_jinja_value(v: &serde_yaml::Value) -> minijinja::Value {
    use minijinja::Value;

    match v {
        serde_yaml::Value::Null => Value::UNDEFINED,
        serde_yaml::Value::Bool(b) => Value::from(*b),
        serde_yaml::Value::Number(n) => n
            .as_i64()
            .map(Value::from)
            .or_else(|| n.as_f64().map(Value::from))
            .unwrap_or_else(|| Value::from(n.to_string())),
        serde_yaml::Value::String(s) => Value::from(s.clone()),
        serde_yaml::Value::Sequence(seq) => {
            let items: Vec<Value> = seq.iter().map(yaml_to_jinja_value).collect();
            Value::from(items)
        }
        serde_yaml::Value::Mapping(map) => {
            let items: HashMap<String, Value> = map
                .iter()
                .filter_map(|(k, v)| {
                    k.as_str()
                        .map(|key| (key.to_string(), yaml_to_jinja_value(v)))
                })
                .collect();
            Value::from_serialize(&items)
        }
        serde_yaml::Value::Tagged(tagged) => yaml_to_jinja_value(&tagged.value),
    }
}
