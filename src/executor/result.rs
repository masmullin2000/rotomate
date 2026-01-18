// Re-export CommandResult from ssh module
pub use crate::ssh::CommandResult;

/// Result of a Proxmox VM operation.
#[derive(Debug, Clone)]
pub struct ProxmoxResult {
    /// Action that was executed (e.g., "snapshot 'pre-deploy'", "start").
    pub action: String,
    /// VM ID.
    pub vmid: u32,
    /// Proxmox node name.
    pub node: String,
    /// Ok(()) on success, or error message on failure.
    pub result: Result<(), String>,
}

impl ProxmoxResult {
    /// Returns true if the operation succeeded.
    pub const fn success(&self) -> bool {
        self.result.is_ok()
    }
}

/// Result of copying a file.
#[derive(Debug, Clone)]
pub struct CopyResult {
    /// Local path (source for uploads, destination for downloads).
    pub local_path: String,
    /// Remote path (destination for uploads, source for downloads).
    pub remote_path: String,
    /// Bytes transferred, or error message.
    pub result: Result<u64, String>,
}

impl CopyResult {
    /// Returns true if the copy succeeded.
    pub const fn success(&self) -> bool {
        self.result.is_ok()
    }
}

/// Result of deleting a remote file.
#[derive(Debug, Clone)]
pub struct DeleteResult {
    /// Remote path that was deleted.
    pub remote_path: String,
    /// Ok(()) on success, or error message on failure.
    pub result: Result<(), String>,
}

impl DeleteResult {
    /// Returns true if the delete succeeded.
    pub const fn success(&self) -> bool {
        self.result.is_ok()
    }
}

/// Result of executing a task (sequence of commands and/or file copies) on a host.
#[derive(Debug, Clone)]
pub struct TaskResult {
    /// Name of the task.
    pub task_name: String,
    /// Results of each Proxmox VM operation.
    pub proxmox: Vec<ProxmoxResult>,
    /// Results of each host command (executed locally).
    pub local_commands: Vec<CommandResult>,
    /// Results of each file upload (copy to remote).
    pub uploads: Vec<CopyResult>,
    /// Results of each command in the task (executed remotely).
    pub commands: Vec<CommandResult>,
    /// Results of each file download (copy from remote).
    pub downloads: Vec<CopyResult>,
    /// Results of each remote file deletion.
    pub deletes: Vec<DeleteResult>,
    /// Results of each local file deletion.
    pub local_deletes: Vec<DeleteResult>,
    /// Whether execution was stopped early due to an error.
    pub stopped_early: bool,
    /// Whether verbose output was enabled for this task.
    pub verbose: bool,
    /// Whether output should be captured and displayed when task completes.
    pub capture_output: bool,
    /// Inactivity timeout in seconds.
    pub inactivity_timeout: u64,
    /// Duration of task execution in seconds.
    pub duration_secs: f64,
}

impl TaskResult {
    /// Creates a new builder for `TaskResult`.
    pub fn builder(task_name: impl Into<String>) -> TaskResultBuilder {
        TaskResultBuilder::new(task_name)
    }

    /// Returns true if all operations succeeded.
    pub fn success(&self) -> bool {
        self.proxmox.iter().all(ProxmoxResult::success)
            && self.local_commands.iter().all(CommandResult::success)
            && self.uploads.iter().all(CopyResult::success)
            && self.commands.iter().all(CommandResult::success)
            && self.downloads.iter().all(CopyResult::success)
            && self.deletes.iter().all(DeleteResult::success)
            && self.local_deletes.iter().all(DeleteResult::success)
    }

    /// Returns the number of failed operations.
    pub fn failed_count(&self) -> usize {
        self.proxmox.iter().filter(|p| !p.success()).count()
            + self.local_commands.iter().filter(|c| !c.success()).count()
            + self.uploads.iter().filter(|c| !c.success()).count()
            + self.commands.iter().filter(|c| !c.success()).count()
            + self.downloads.iter().filter(|c| !c.success()).count()
            + self.deletes.iter().filter(|d| !d.success()).count()
            + self.local_deletes.iter().filter(|d| !d.success()).count()
    }
}

/// Builder for constructing `TaskResult` instances.
#[derive(Debug, Clone, Default)]
pub struct TaskResultBuilder {
    task_name: String,
    proxmox: Vec<ProxmoxResult>,
    local_commands: Vec<CommandResult>,
    uploads: Vec<CopyResult>,
    commands: Vec<CommandResult>,
    downloads: Vec<CopyResult>,
    deletes: Vec<DeleteResult>,
    local_deletes: Vec<DeleteResult>,
    stopped_early: bool,
    verbose: bool,
    capture_output: bool,
    inactivity_timeout: u64,
    duration_secs: f64,
}

impl TaskResultBuilder {
    /// Creates a new builder with the given task name.
    pub fn new(task_name: impl Into<String>) -> Self {
        Self {
            task_name: task_name.into(),
            ..Default::default()
        }
    }

    /// Adds a Proxmox operation result.
    pub fn proxmox(mut self, result: Vec<ProxmoxResult>) -> Self {
        self.proxmox = result;
        self
    }

    /// Adds a local command result.
    pub fn local_command(mut self, result: CommandResult) -> Self {
        self.local_commands.push(result);
        self
    }

    /// Adds an upload result.
    pub fn upload(mut self, result: CopyResult) -> Self {
        self.uploads.push(result);
        self
    }

    /// Adds a remote command result.
    pub fn command(mut self, result: CommandResult) -> Self {
        self.commands.push(result);
        self
    }

    /// Adds a download result.
    pub fn download(mut self, result: CopyResult) -> Self {
        self.downloads.push(result);
        self
    }

    /// Adds a remote delete result.
    pub fn delete(mut self, result: DeleteResult) -> Self {
        self.deletes.push(result);
        self
    }

    /// Adds a local delete result.
    pub fn local_delete(mut self, result: DeleteResult) -> Self {
        self.local_deletes.push(result);
        self
    }

    /// Sets whether execution was stopped early due to an error.
    pub const fn stopped_early(mut self, stopped: bool) -> Self {
        self.stopped_early = stopped;
        self
    }

    /// Sets whether verbose output was enabled.
    pub const fn verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Sets whether output should be captured and displayed when task completes.
    pub const fn capture_output(mut self, capture_output: bool) -> Self {
        self.capture_output = capture_output;
        self
    }

    /// Sets the inactivity timeout in seconds.
    pub const fn inactivity_timeout(mut self, inactivity_timeout: u64) -> Self {
        self.inactivity_timeout = inactivity_timeout;
        self
    }

    /// Sets the duration in seconds.
    pub const fn duration_secs(mut self, duration_secs: f64) -> Self {
        self.duration_secs = duration_secs;
        self
    }

    /// Builds the `TaskResult`.
    pub fn build(self) -> TaskResult {
        TaskResult {
            task_name: self.task_name,
            proxmox: self.proxmox,
            local_commands: self.local_commands,
            uploads: self.uploads,
            commands: self.commands,
            downloads: self.downloads,
            deletes: self.deletes,
            local_deletes: self.local_deletes,
            stopped_early: self.stopped_early,
            verbose: self.verbose,
            capture_output: self.capture_output,
            inactivity_timeout: self.inactivity_timeout,
            duration_secs: self.duration_secs,
        }
    }
}

/// Result of executing a task on a specific host.
#[derive(Debug, Clone)]
pub struct HostResult {
    /// Name of the host.
    pub host_name: String,
    /// Task execution result, or error message if connection failed.
    pub result: Result<TaskResult, String>,
}

impl HostResult {
    /// Returns true if the task succeeded on this host.
    pub fn success(&self) -> bool {
        self.result.as_ref().is_ok_and(TaskResult::success)
    }

    /// Create a host result from a connection error.
    pub fn connection_error(host_name: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            host_name: host_name.into(),
            result: Err(error.into()),
        }
    }
}

/// Result of executing a single task across multiple hosts.
#[derive(Debug, Clone)]
pub struct TaskExecution {
    /// Name of the task that was executed.
    pub task_name: String,
    /// Results from each host.
    pub hosts: Vec<HostResult>,
}

impl TaskExecution {
    /// Returns true if the task succeeded on all hosts.
    pub fn success(&self) -> bool {
        self.hosts.iter().all(HostResult::success)
    }

    /// Returns the number of hosts where the task failed.
    pub fn failed_count(&self) -> usize {
        self.hosts.iter().filter(|h| !h.success()).count()
    }

    /// Returns the number of hosts where the task succeeded.
    pub fn success_count(&self) -> usize {
        self.hosts.iter().filter(|h| h.success()).count()
    }
}

/// Result of a task group (a sequence of tasks executed in order).
#[derive(Debug, Clone)]
pub struct TaskGroupResult {
    /// Name of the task group.
    pub group_name: String,
    /// Results for each task in the group.
    pub tasks: Vec<TaskExecution>,
}

impl TaskGroupResult {
    /// Returns true if all tasks succeeded.
    pub fn success(&self) -> bool {
        self.tasks.iter().all(TaskExecution::success)
    }

    /// Returns the number of task executions that failed.
    pub fn failed_task_count(&self) -> usize {
        self.tasks.iter().filter(|t| !t.success()).count()
    }

    /// Returns the number of task executions that succeeded.
    pub fn success_task_count(&self) -> usize {
        self.tasks.iter().filter(|t| t.success()).count()
    }
}
