use indexmap::IndexMap;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::config::HostContext;

/// Operating system type for a host.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OsType {
    /// Linux/Unix-like systems (default).
    #[default]
    Linux,
    /// Windows systems.
    Windows,
}

/// Shell type for command execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShellType {
    /// POSIX shell (sh).
    Sh,
    /// `PowerShell`.
    #[serde(alias = "pwsh")]
    PowerShell,
    /// Windows Command Prompt (cmd.exe).
    Cmd,
}

impl ShellType {
    /// Get the default shell type for an OS.
    pub const fn default_for_os(os: OsType) -> Self {
        match os {
            OsType::Linux => Self::Sh,
            OsType::Windows => Self::PowerShell,
        }
    }
}

/// Proxmox VE API configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct ProxmoxConfig {
    /// Base URL for Proxmox API (e.g., `https://proxmox.example.com:8006`).
    pub url: String,

    /// API token ID (e.g., `user@pam!token-name`).
    pub token_id: String,

    /// API token secret.
    pub token_secret: String,

    /// Whether to verify SSL certificates (default: true).
    #[serde(default = "default_verify_ssl")]
    pub verify_ssl: bool,

    /// Request timeout in seconds (default: 30).
    #[serde(default = "default_timeout")]
    pub timeout: u64,
}

const fn default_verify_ssl() -> bool {
    true
}

const fn default_timeout() -> u64 {
    30
}

/// Root configuration structure.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    /// Other config files to import (processed before this file).
    #[serde(default)]
    pub imports: Vec<PathBuf>,

    /// Template variables for Jinja2-style substitution.
    #[serde(default)]
    pub vars: HashMap<String, serde_yaml::Value>,

    /// Proxmox VE API configuration.
    pub proxmox: Option<ProxmoxConfig>,

    /// Global defaults for host connections.
    #[serde(default)]
    pub defaults: Defaults,

    /// Named host definitions.
    #[serde(default)]
    pub hosts: HashMap<String, Host>,

    /// Named groups of hosts.
    #[serde(default)]
    pub groups: HashMap<String, Vec<String>>,

    /// Named task definitions.
    #[serde(default)]
    pub tasks: HashMap<String, Task>,

    /// Named task groups - each group executes in parallel with others.
    /// Within each group, tasks execute sequentially.
    /// Format: key is identifier, first list item has `name`, rest have `task`/`hosts`.
    #[serde(default, rename = "task_groups")]
    pub task_groups: HashMap<String, Vec<TaskGroupEntry>>,

    /// Named campaigns - each campaign is a whitelist of `task_groups` to include.
    /// If no campaigns defined, all `task_groups` run.
    /// If campaigns exist, the first one defined is auto-selected unless --campaign is specified.
    #[serde(default)]
    pub campaigns: IndexMap<String, Campaign>,

    /// File-scoped vars for each `task_group` (not from YAML, populated by loader).
    /// Maps `task_group` key to the file-scoped vars of the file that defined it.
    #[serde(skip)]
    pub task_groups_vars: HashMap<String, HashMap<String, serde_yaml::Value>>,

    /// Source file tracking for hosts (not from YAML, populated by loader).
    #[serde(skip)]
    pub hosts_source: HashMap<String, PathBuf>,

    /// Source file tracking for tasks (not from YAML, populated by loader).
    #[serde(skip)]
    pub tasks_source: HashMap<String, PathBuf>,

    /// Source file tracking for `task_groups` (not from YAML, populated by loader).
    #[serde(skip)]
    pub groups_source: HashMap<String, PathBuf>,

    /// Source file tracking for campaigns (not from YAML, populated by loader).
    #[serde(skip)]
    pub campaigns_source: HashMap<String, PathBuf>,
}

impl Config {
    /// Merge another config into this one.
    /// Values from `other` override values in `self`.
    pub fn merge(&mut self, other: Self) {
        self.vars.extend(other.vars);
        if other.proxmox.is_some() {
            self.proxmox = other.proxmox;
        }
        self.defaults.merge(other.defaults);
        self.hosts.extend(other.hosts);
        self.groups.extend(other.groups);
        self.tasks.extend(other.tasks);
        self.task_groups.extend(other.task_groups);
        self.task_groups_vars.extend(other.task_groups_vars);
        self.campaigns.extend(other.campaigns);
        // Merge source tracking
        self.hosts_source.extend(other.hosts_source);
        self.tasks_source.extend(other.tasks_source);
        self.groups_source.extend(other.groups_source);
        self.campaigns_source.extend(other.campaigns_source);
    }

    pub fn resolve_hosts(
        &self,
    ) -> impl Iterator<Item = anyhow::Result<(String, crate::executor::Host)>> + use<'_> {
        self.hosts.iter().map(|(name, host)| {
            Ok((name.clone(), self.resolve_host(name, host)?))
        })
    }

    /// Resolve a single host with defaults applied.
    fn resolve_host(&self, name: &str, host: &Host) -> anyhow::Result<crate::executor::Host> {
        let port = host.port.or(self.defaults.port).unwrap_or(22);

        let username = host
            .username
            .clone()
            .or_else(|| self.defaults.username.clone())
            .map_or_else(
                || whoami::username().map_err(|e| anyhow::anyhow!("Failed to get username: {e}")),
                Ok,
            )?;

        let private_key = host
            .private_key
            .clone()
            .or_else(|| self.defaults.private_key.clone());

        // Resolve OS type: host > defaults > Linux
        let os = host.os.or(self.defaults.os).unwrap_or_default();

        // Resolve shell type: host > defaults > default for OS
        let shell = host
            .shell
            .or(self.defaults.shell)
            .unwrap_or_else(|| ShellType::default_for_os(os));

        Ok(crate::executor::Host {
            ctx: HostContext {
                name: name.to_string(),
                hostname: host.hostname.clone(),
                port,
                username,
                os,
                shell,
            },
            private_key,
            sudo_prompt: host.sudo_prompt,
            sudo_password: host.sudo_password.clone(),
            ssh_password: None, // Set at runtime via main.rs (when no private_key)
            admin_prompt: host.admin_prompt,
            admin_password: host.admin_password.clone(),
            proxmox_vmid: host.proxmox.as_ref().map(|p| p.vmid),
            proxmox_node: host.proxmox.as_ref().map(|p| p.node.clone()),
            proxmox_snapshot: host.proxmox.as_ref().and_then(|p| p.snapshot.clone()),
        })
    }

    /// Expand campaign targets recursively, resolving campaign references.
    /// Returns the set of `task_group` keys that should be included.
    fn expand_campaign_targets(
        &self,
        campaign_name: &str,
        task_group_keys: &HashSet<&String>,
    ) -> anyhow::Result<HashSet<String>> {
        let mut result = HashSet::new();
        let mut visited = HashSet::new();
        self.expand_campaign_targets_recursive(
            campaign_name,
            task_group_keys,
            &mut result,
            &mut visited,
        )?;
        Ok(result)
    }

    /// Recursive helper for expanding campaign targets.
    fn expand_campaign_targets_recursive(
        &self,
        campaign_name: &str,
        task_group_keys: &HashSet<&String>,
        result: &mut HashSet<String>,
        visited: &mut HashSet<String>,
    ) -> anyhow::Result<()> {
        // Check for circular reference
        if !visited.insert(campaign_name.to_string()) {
            anyhow::bail!("Circular campaign reference detected: '{campaign_name}'");
        }

        let campaign = self
            .campaigns
            .get(campaign_name)
            .ok_or_else(|| anyhow::anyhow!("Campaign '{campaign_name}' not found"))?;

        for target in campaign {
            if task_group_keys.contains(target) {
                // Target is a task_group - add it directly
                result.insert(target.clone());
            } else if self.campaigns.contains_key(target) {
                // Target is another campaign - expand it recursively
                self.expand_campaign_targets_recursive(target, task_group_keys, result, visited)?;
            } else {
                anyhow::bail!(
                    "Target '{target}' in campaign '{campaign_name}' is neither a task_group nor a campaign"
                );
            }
        }

        Ok(())
    }

    /// Collect all dependencies recursively for `task_groups` in the campaign.
    /// Adds any dependencies not already in `campaign_groups` to `deps_to_add`.
    fn collect_all_dependencies(
        &self,
        campaign_groups: &HashSet<String>,
        all_group_keys: &HashSet<&String>,
        deps_to_add: &mut Vec<String>,
    ) {
        let mut visited: HashSet<String> = campaign_groups.clone();
        let mut to_visit: Vec<String> = campaign_groups.iter().cloned().collect();

        while let Some(group_key) = to_visit.pop() {
            if let Some(items) = self.task_groups.get(&group_key) {
                // Extract dependencies for this group
                for item in items {
                    if let TaskGroupEntry::Depends { depends } = item {
                        for dep in depends {
                            // Only process if it exists and hasn't been visited
                            if all_group_keys.contains(dep) && !visited.contains(dep) {
                                visited.insert(dep.clone());
                                deps_to_add.push(dep.clone());
                                to_visit.push(dep.clone());
                            }
                        }
                    }
                }
            }
        }
    }

    /// Resolve task items within a group, validating task and host references.
    fn resolve_task_items(
        &self,
        group_key: &str,
        items: &[TaskGroupEntry],
        group_hosts: Option<&HostRef>,
    ) -> anyhow::Result<Vec<crate::config::TaskGroupItem>> {
        items
            .iter()
            .filter_map(|item| match item {
                TaskGroupEntry::TaskGroupItem(task_item) => Some(task_item),
                TaskGroupEntry::Name { .. }
                | TaskGroupEntry::Depends { .. }
                | TaskGroupEntry::Hosts(_) => None,
            })
            .map(|TaskGroupItem { task, hosts, .. }| {
                if !self.tasks.contains_key(task) {
                    anyhow::bail!(
                        "Task '{task}' referenced in task group '{group_key}' does not exist"
                    );
                }

                // Use task-level hosts if specified, otherwise fall back to group-level hosts
                let effective_hosts = hosts.as_ref().or(group_hosts);

                // hosts is optional - if None, this is a local-only task
                let host_names: Vec<_> = if let Some(host_ref) = effective_hosts {
                    expand_host_ref(host_ref, self)
                        .map(|name| {
                            if !self.hosts.contains_key(&name) {
                                anyhow::bail!(
                                    "Host '{name}' referenced in task group for task '{task}' does not exist"
                                );
                            }
                            Ok(name)
                        })
                        .collect::<anyhow::Result<_>>()?
                } else {
                    Vec::new()
                };

                Ok(crate::config::TaskGroupItem {
                    task_name: task.clone(),
                    host_names,
                })
            })
            .collect()
    }

    /// Resolve a single task group entry into a resolved `TaskGroup`.
    fn resolve_single_task_group(
        &self,
        group_key: &str,
        items: &[TaskGroupEntry],
        group_keys: &HashSet<&String>,
        campaign_groups: Option<&HashSet<String>>,
        lenient_campaign: bool,
    ) -> anyhow::Result<(String, crate::config::TaskGroup)> {
        let display_name = items
            .iter()
            .find_map(|item| match item {
                TaskGroupEntry::Name { name } => Some(name.clone()),
                TaskGroupEntry::Depends { .. }
                | TaskGroupEntry::Hosts(_)
                | TaskGroupEntry::TaskGroupItem(_) => None,
            })
            .unwrap_or_else(|| group_key.to_owned());

        // Extract group-level default hosts (if specified)
        let group_hosts = items.iter().find_map(|item| match item {
            TaskGroupEntry::Hosts(h) => Some(&h.hosts),
            TaskGroupEntry::Name { .. }
            | TaskGroupEntry::Depends { .. }
            | TaskGroupEntry::TaskGroupItem(_) => None,
        });

        let original_depends_on: Vec<_> = items
            .iter()
            .filter_map(|item| match item {
                TaskGroupEntry::Depends { depends } => Some(depends.clone()),
                TaskGroupEntry::Name { .. }
                | TaskGroupEntry::Hosts(_)
                | TaskGroupEntry::TaskGroupItem(_) => None,
            })
            .flatten()
            .collect();

        // Validate that all dependencies exist in config
        if let Some(dep) = original_depends_on
            .iter()
            .find(|dep| !group_keys.contains(dep))
        {
            anyhow::bail!(
                "Task group '{group_key}' depends on '{dep}' which does not exist"
            );
        }

        let depends_on = filter_dependencies_for_campaign(
            group_key,
            original_depends_on,
            campaign_groups,
            lenient_campaign,
        );

        let resolved_tasks = self.resolve_task_items(group_key, items, group_hosts)?;

        let group_vars = self
            .task_groups_vars
            .get(group_key)
            .cloned()
            .unwrap_or_default();

        Ok((
            group_key.to_owned(),
            crate::config::TaskGroup {
                key: group_key.to_owned(),
                name: display_name,
                tasks: resolved_tasks,
                depends_on,
                vars: group_vars,
            },
        ))
    }

    /// Resolve all task groups, expanding group references.
    /// If a campaign is specified, filters to only `task_groups` in that campaign.
    /// When `lenient_campaign` is false (default), dependencies are auto-included in the campaign.
    /// When `lenient_campaign` is true, dependencies not in the campaign are filtered out with warnings.
    /// Returns an error if dependencies reference non-existent groups or if there are cycles.
    pub fn resolve_task_groups(
        &self,
        campaign: Option<&str>,
        lenient_campaign: bool,
    ) -> anyhow::Result<Vec<crate::config::TaskGroup>> {
        // First pass: collect all group keys
        let group_keys: HashSet<_> = self.task_groups.keys().collect();

        // Determine which task_groups to include (expanding campaign references)
        let mut campaign_groups: Option<HashSet<String>> = campaign
            .map(|name| self.expand_campaign_targets(name, &group_keys))
            .transpose()?;

        // When not lenient, auto-include all dependencies recursively
        if !lenient_campaign && let Some(ref mut cg) = campaign_groups {
            let mut deps_to_add: Vec<String> = Vec::new();
            self.collect_all_dependencies(cg, &group_keys, &mut deps_to_add);
            for dep in deps_to_add {
                cg.insert(dep);
            }
        }

        let resolved_groups = self
            .task_groups
            .iter()
            .filter(|(group_key, _)| {
                campaign_groups
                    .as_ref()
                    .is_none_or(|cg| cg.contains(group_key.as_str()))
            })
            .map(|(group_key, items)| {
                self.resolve_single_task_group(
                    group_key,
                    items,
                    &group_keys,
                    campaign_groups.as_ref(),
                    lenient_campaign,
                )
            });

        // Detect circular dependencies using DFS
        Ok(detect_dependency_cycle(resolved_groups)?
            .into_iter()
            .map(|(_, group)| group)
            .collect())
    }
}

/// DFS helper for cycle detection.
fn cycle_dfs<'a>(
    node: &'a str,
    deps_map: &'a HashMap<String, Vec<String>>,
    visited: &mut HashSet<&'a str>,
    in_stack: &mut HashSet<&'a str>,
    path: &mut Vec<&'a str>,
) -> Option<Vec<String>> {
    if in_stack.contains(node) {
        let cycle_start = path.iter().position(|&n| n == node).unwrap();
        let mut cycle: Vec<_> = path[cycle_start..]
            .iter()
            .copied()
            .map(ToString::to_string)
            .collect();
        cycle.push(node.to_string());
        return Some(cycle);
    }

    if visited.contains(node) {
        return None;
    }

    visited.insert(node);
    in_stack.insert(node);
    path.push(node);

    if let Some(deps) = deps_map.get(node) {
        for dep in deps {
            if let Some(cycle) = cycle_dfs(dep.as_str(), deps_map, visited, in_stack, path) {
                return Some(cycle);
            }
        }
    }

    path.pop();
    in_stack.remove(node);
    None
}

/// Detect circular dependencies in task groups using DFS.
/// Collects the iterator, validates, and returns the collected groups.
fn detect_dependency_cycle(
    groups: impl IntoIterator<Item = anyhow::Result<(String, crate::config::TaskGroup)>>,
) -> anyhow::Result<Vec<(String, crate::config::TaskGroup)>> {
    // Collect groups and build adjacency map in a single pass
    let (collected, deps_map) = groups.into_iter().try_fold(
        (Vec::new(), HashMap::new()),
        |(mut collected, mut deps_map), result| {
            let (key, group) = result?;
            deps_map.insert(key.clone(), group.depends_on.clone());
            collected.push((key, group));
            anyhow::Ok((collected, deps_map))
        },
    )?;

    // Track visited nodes and nodes in current path
    let mut visited = HashSet::new();
    let mut in_stack = HashSet::new();

    for (key, _) in &collected {
        let mut path = Vec::new();
        if let Some(cycle) = cycle_dfs(
            key.as_str(),
            &deps_map,
            &mut visited,
            &mut in_stack,
            &mut path,
        ) {
            anyhow::bail!(
                "Circular dependency detected in run groups: {}",
                cycle.join(" -> ")
            );
        }
    }

    Ok(collected)
}

/// Filter dependencies to only those in the campaign.
/// In lenient mode, warns about missing dependencies; otherwise they were auto-included.
fn filter_dependencies_for_campaign(
    group_key: &str,
    depends_on: Vec<String>,
    campaign_groups: Option<&HashSet<String>>,
    lenient_campaign: bool,
) -> Vec<String> {
    if let Some(cg) = campaign_groups {
        depends_on
            .into_iter()
            .filter(|dep| {
                let in_campaign = cg.contains(dep.as_str());
                if !in_campaign && lenient_campaign {
                    eprintln!(
                        "Warning: task_group '{group_key}' has dependency '{dep}' \
                         which is not in the campaign - skipping dependency"
                    );
                }
                in_campaign
            })
            .collect()
    } else {
        depends_on
    }
}

/// Expand a host reference to a list of host names.
fn expand_host_ref(host_ref: &HostRef, config: &Config) -> Box<dyn Iterator<Item = String>> {
    match host_ref {
        HostRef::Single(name) => Box::new(
            config
                .groups
                .get(name)
                .cloned()
                .unwrap_or_else(|| vec![name.clone()])
                .into_iter(),
        ),
        HostRef::List(names) => {
            let expanded: HashSet<_> = names
                .iter()
                .flat_map(|name| {
                    config
                        .groups
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| vec![name.clone()])
                })
                .collect();
            Box::new(expanded.into_iter())
        }
    }
}

/// An entry in a task group - either the name declaration, dependencies, default hosts, or a task item.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum TaskGroupEntry {
    /// The display name for this task group (must be first item).
    Name { name: String },
    /// Dependencies on other task groups (must complete before this group starts).
    Depends { depends: Vec<String> },
    /// Default hosts for all tasks in this group (can be overridden per-task).
    Hosts(TaskGroupHosts),
    /// A task to run on hosts.
    TaskGroupItem(TaskGroupItem),
}

/// Default hosts declaration for a task group.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskGroupHosts {
    /// Target hosts - can be a group name, a single host, or a list.
    pub hosts: HostRef,
}

/// Default values for host connections.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Defaults {
    /// Default SSH port.
    pub port: Option<u16>,

    /// Default username.
    pub username: Option<String>,

    /// Default path to private key.
    pub private_key: Option<PathBuf>,

    /// SSH inactivity timeout in seconds (default: 15).
    #[serde(alias = "inactivity_timeout")]
    pub timeout: Option<u64>,

    /// Default OS type (linux or windows).
    pub os: Option<OsType>,

    /// Default shell type (sh, powershell, or cmd).
    pub shell: Option<ShellType>,
}

impl Defaults {
    /// Merge another defaults into this one.
    /// Values from `other` override values in `self` if they are Some.
    pub fn merge(&mut self, other: Self) {
        if other.port.is_some() {
            self.port = other.port;
        }
        if other.username.is_some() {
            self.username = other.username;
        }
        if other.private_key.is_some() {
            self.private_key = other.private_key;
        }
        if other.timeout.is_some() {
            self.timeout = other.timeout;
        }
        if other.os.is_some() {
            self.os = other.os;
        }
        if other.shell.is_some() {
            self.shell = other.shell;
        }
    }
}

/// Host connection configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct Host {
    /// Hostname or IP address.
    pub hostname: String,

    /// SSH port (overrides default).
    pub port: Option<u16>,

    /// Username (overrides default).
    pub username: Option<String>,

    /// Path to private key (overrides default).
    pub private_key: Option<PathBuf>,

    /// Whether to prompt for a host-specific sudo password.
    #[serde(default)]
    pub sudo_prompt: bool,

    /// Host-specific sudo password (for `become_root` tasks).
    pub sudo_password: Option<String>,

    /// Operating system type (linux or windows).
    pub os: Option<OsType>,

    /// Shell type for command execution (sh, powershell, or cmd).
    pub shell: Option<ShellType>,

    /// Whether to prompt for Windows administrator password at runtime.
    #[serde(default)]
    pub admin_prompt: bool,

    /// Windows administrator password (for privilege escalation).
    pub admin_password: Option<String>,

    /// Proxmox VM configuration for this host.
    pub proxmox: Option<HostProxmox>,
}

/// Proxmox VM configuration for a host.
#[derive(Debug, Clone, Deserialize)]
pub struct HostProxmox {
    /// Proxmox VM ID for this host (for `proxmox_command` operations).
    pub vmid: u32,

    /// Proxmox node name where this VM resides.
    pub node: String,

    /// Default snapshot name for rollback operations.
    pub snapshot: Option<String>,
}

/// A single step within a task's `steps` sequence.
///
/// Steps execute in declaration order and allow repeating operation types
/// (e.g., upload -> `remote_command` -> upload -> `remote_command`).
///
/// YAML format (each step is a single-key mapping):
/// ```yaml
/// steps:
///   - remote_command:
///       - echo hello
///   - upload:
///       - "local/file /remote/path"
/// ```
#[derive(Debug, Clone)]
pub enum TaskStep {
    /// Commands to execute on remote hosts.
    RemoteCommand(ExecCmdline),
    /// Commands to execute on the local machine.
    LocalCommand(ExecCmdline),
    /// Files to upload to remote host (format: "local remote").
    Upload(Vec<String>),
    /// Files to download from remote host (format: "remote local").
    Download(Vec<String>),
    /// Files to delete on remote host.
    DeleteRemote(Vec<String>),
    /// Files to delete on local host.
    DeleteLocal(Vec<String>),
}

impl<'de> serde::Deserialize<'de> for TaskStep {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        let mapping = serde_yaml::Mapping::deserialize(deserializer)?;
        if mapping.len() != 1 {
            return Err(D::Error::custom(format!(
                "each step must have exactly one key, got {}",
                mapping.len()
            )));
        }

        let (key, value) = mapping.into_iter().next().unwrap();
        let key_str = key
            .as_str()
            .ok_or_else(|| D::Error::custom("step key must be a string"))?;

        match key_str {
            "remote_command" => {
                let exec = ExecCmdline::deserialize(value).map_err(D::Error::custom)?;
                Ok(Self::RemoteCommand(exec))
            }
            "local_command" => {
                let exec = ExecCmdline::deserialize(value).map_err(D::Error::custom)?;
                Ok(Self::LocalCommand(exec))
            }
            "upload" => {
                let paths = Vec::<String>::deserialize(value).map_err(D::Error::custom)?;
                Ok(Self::Upload(paths))
            }
            "download" => {
                let paths = Vec::<String>::deserialize(value).map_err(D::Error::custom)?;
                Ok(Self::Download(paths))
            }
            "delete_remote" => {
                let paths = Vec::<String>::deserialize(value).map_err(D::Error::custom)?;
                Ok(Self::DeleteRemote(paths))
            }
            "delete_local" => {
                let paths = Vec::<String>::deserialize(value).map_err(D::Error::custom)?;
                Ok(Self::DeleteLocal(paths))
            }
            other => Err(D::Error::custom(format!(
                "unknown step type '{other}', expected one of: \
                 remote_command, local_command, upload, download, delete_remote, delete_local"
            ))),
        }
    }
}

impl TaskStep {
    /// Returns true if this step requires an SSH connection.
    pub const fn needs_ssh(&self) -> bool {
        matches!(
            self,
            Self::RemoteCommand(_) | Self::Upload(_) | Self::Download(_) | Self::DeleteRemote(_)
        )
    }

    /// Returns true if this step runs locally (no SSH required).
    pub const fn is_local(&self) -> bool {
        matches!(self, Self::LocalCommand(_) | Self::DeleteLocal(_))
    }
}

/// Task definition - a named sequence of commands and/or file operations.
#[derive(Debug, Clone, Deserialize)]
pub struct Task {
    /// Name of the task (set manually, not parsed from YAML).
    #[serde(skip)]
    pub name: String,

    /// Sudo password for `become_root` tasks (set at runtime, not from YAML).
    #[serde(skip)]
    pub sudo_password: Option<String>,

    /// Per-file scoped variables (inherited from parent + file's own vars).
    /// Set by the loader, not from YAML. Used at execution time to override
    /// global config vars so each file's tasks see their own variable scope.
    #[serde(skip)]
    pub vars: HashMap<String, serde_yaml::Value>,

    /// Keys of variables declared directly in the task's own file (not inherited).
    /// Used at execution time to layer only the task's own vars on top of group vars.
    #[serde(skip)]
    pub own_var_keys: HashSet<String>,

    /// Human-readable description of the task.
    #[serde(default)]
    pub description: String,

    /// Whether to stop execution on first command failure.
    #[serde(default = "default_stop_on_error")]
    pub stop_on_error: bool,

    /// Whether to run commands as root via sudo.
    #[serde(default)]
    pub become_root: bool,

    /// SSH timeout in seconds (overrides global default).
    #[serde(alias = "inactivity_timeout")]
    pub timeout: Option<u64>,

    /// Proxmox VM commands to execute (e.g., "snapshot pre-deploy", "rollback clean-state").
    /// These execute first, before any other operations.
    #[serde(default)]
    pub proxmox_command: Vec<String>,

    /// Commands to execute on remote hosts.
    #[serde(default)]
    pub remote_command: ExecCmdline,

    /// Commands to execute on the local machine (before remote execution).
    #[serde(default)]
    pub local_command: ExecCmdline,

    /// Files to upload to remote host (format: "local remote").
    #[serde(default)]
    pub upload: Vec<String>,

    /// Files to download from remote host (format: "remote local").
    #[serde(default)]
    pub download: Vec<String>,

    /// Files to delete on remote host.
    #[serde(default)]
    pub delete_remote: Vec<String>,

    /// Files to delete on local host.
    #[serde(default)]
    pub delete_local: Vec<String>,

    /// Ordered sequence of operations. When non-empty, flat fields
    /// (`remote_command`, `local_command`, `upload`, `download`, `delete_remote`,
    /// `delete_local`) are ignored and steps are executed in declaration order.
    /// `proxmox_command` is NOT supported in steps — it must stay as a flat field.
    #[serde(default)]
    pub steps: Vec<TaskStep>,

    /// Verbose printing of command output (streams output in real-time).
    #[serde(default)]
    pub verbose: Option<bool>,

    /// Capture and display output when task completes.
    #[serde(default)]
    pub capture_output: Option<bool>,
}

impl Task {
    /// Check if the task has any commands or file operations to execute.
    pub fn is_empty(&self) -> bool {
        self.proxmox_command.is_empty()
            && self.steps.is_empty()
            && self.remote_command.is_empty()
            && self.local_command.is_empty()
            && self.upload.is_empty()
            && self.download.is_empty()
            && self.delete_remote.is_empty()
            && self.delete_local.is_empty()
    }

    /// Returns true if this task uses the `steps:` syntax.
    pub const fn has_steps(&self) -> bool {
        !self.steps.is_empty()
    }

    pub fn timeout(&self) -> u64 {
        const DEFAULT: u64 = 15;
        self.timeout.unwrap_or(DEFAULT)
    }

    /// Validate that flat operation fields are not mixed with `steps`.
    pub fn validate(&self, name: &str) -> anyhow::Result<()> {
        if !self.has_steps() {
            return Ok(());
        }

        let mut conflicts = Vec::new();
        if !self.remote_command.is_empty() {
            conflicts.push("remote_command");
        }
        if !self.local_command.is_empty() {
            conflicts.push("local_command");
        }
        if !self.upload.is_empty() {
            conflicts.push("upload");
        }
        if !self.download.is_empty() {
            conflicts.push("download");
        }
        if !self.delete_remote.is_empty() {
            conflicts.push("delete_remote");
        }
        if !self.delete_local.is_empty() {
            conflicts.push("delete_local");
        }

        if !conflicts.is_empty() {
            anyhow::bail!(
                "Task '{name}' uses 'steps' but also has flat fields: {}. \
                 Use either 'steps' or flat fields, not both.",
                conflicts.join(", ")
            );
        }
        Ok(())
    }
}

const fn default_stop_on_error() -> bool {
    true
}

/// A task group item maps a task to target hosts.
#[derive(Debug, Clone, Deserialize)]
pub struct TaskGroupItem {
    /// Name of the task to run.
    pub task: String,

    /// Target hosts - can be a group name, a single host, or a list.
    /// Optional for local-only tasks.
    #[serde(default)]
    pub hosts: Option<HostRef>,
}

/// Reference to hosts - can be a single name or a list.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum HostRef {
    /// A single host or group name.
    Single(String),
    /// A list of host or group names.
    List(Vec<String>),
}

/// Commands to execute - can be a list of commands or a single script.
///
/// Valid YAML formats:
/// ```yaml
/// # List of commands:
/// remote_command:
///   - echo hello
///   - ls -la
///
/// # Single script:
/// local_command: |
///   echo hello
///   echo world
///
/// # Empty/null:
/// remote_command: ~
/// ```
///
/// **Important:** Commands containing `: ` (colon-space) must be YAML-quoted:
/// ```yaml
/// # Wrong — YAML interprets `: ` as a key-value separator:
///   - echo "Current dir: $(pwd)"
///
/// # Correct — wrap the entire command in YAML quotes:
///   - 'echo "Current dir: $(pwd)"'
/// ```
#[derive(Debug, Clone, Default)]
pub enum ExecCmdline {
    /// No commands.
    #[default]
    Empty,
    /// A list of individual commands to execute in sequence.
    List(Vec<String>),
    /// A single script (bash or other) to execute as one unit.
    Script(String),
}

impl<'de> serde::Deserialize<'de> for ExecCmdline {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        let value = serde_yaml::Value::deserialize(deserializer)?;

        match value {
            serde_yaml::Value::Null => Ok(Self::Empty),
            serde_yaml::Value::String(s) => Ok(Self::Script(s)),
            serde_yaml::Value::Sequence(seq) => {
                let mut commands = Vec::with_capacity(seq.len());
                for item in &seq {
                    match item {
                        serde_yaml::Value::String(s) => commands.push(s.clone()),
                        serde_yaml::Value::Mapping(m) => {
                            let reconstructed = reconstruct_mapping_as_command(m);
                            return Err(D::Error::custom(format!(
                                "command must be quoted\n\
                                 Suggested fix:\n  \
                                 - '{reconstructed}'"
                            )));
                        }
                        other => {
                            let type_name = yaml_type_name(other);
                            return Err(D::Error::custom(format!(
                                "command expected a string, got {type_name}"
                            )));
                        }
                    }
                }
                Ok(Self::List(commands))
            }
            other => {
                let type_name = yaml_type_name(&other);
                Err(D::Error::custom(format!(
                    "expected a list of commands, a single script string, or null, but got {type_name}"
                )))
            }
        }
    }
}

/// Get a human-readable type name for a `serde_yaml::Value`.
const fn yaml_type_name(value: &serde_yaml::Value) -> &'static str {
    match value {
        serde_yaml::Value::Null => "null",
        serde_yaml::Value::Bool(_) => "a boolean",
        serde_yaml::Value::Number(_) => "a number",
        serde_yaml::Value::String(_) => "a string",
        serde_yaml::Value::Sequence(_) => "a list",
        serde_yaml::Value::Mapping(_) => "a mapping",
        serde_yaml::Value::Tagged(_) => "a tagged value",
    }
}

/// Reconstruct the approximate original command from a YAML mapping.
///
/// When a command like `echo "Current dir: $(pwd)"` is misinterpreted by YAML,
/// it becomes a mapping: `{"echo \"Current dir": "$(pwd)\""}`.
/// This function joins key-value pairs back with `: ` to show what the user wrote.
fn reconstruct_mapping_as_command(map: &serde_yaml::Mapping) -> String {
    map.iter()
        .map(|(k, v)| {
            let key = yaml_value_to_display(k);
            let val = yaml_value_to_display(v);
            if val.is_empty() {
                key
            } else {
                format!("{key}: {val}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Convert a YAML value to a display string for error messages.
fn yaml_value_to_display(value: &serde_yaml::Value) -> String {
    match value {
        serde_yaml::Value::Null => String::new(),
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        _ => format!("{value:?}"),
    }
}

impl ExecCmdline {
    /// Check if there are no commands to execute.
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Empty => true,
            Self::List(cmds) => cmds.is_empty(),
            Self::Script(s) => s.trim().is_empty(),
        }
    }

    /// Get the number of commands/scripts.
    pub fn len(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::List(cmds) => cmds.len(),
            Self::Script(s) if s.trim().is_empty() => 0,
            Self::Script(_) => 1,
        }
    }

    /// Get commands as a slice for iteration.
    /// For scripts, returns a single-element slice with the whole script.
    pub fn as_commands(&self) -> Vec<&str> {
        match self {
            Self::Empty => vec![],
            Self::List(cmds) => cmds.iter().map(String::as_str).collect(),
            Self::Script(s) => vec![s.as_str()],
        }
    }
}

/// A campaign definition - a named whitelist of `task_groups` to run.
///
/// When multiple campaigns exist and none is specified, the first one (alphabetically) is used.
/// Can be specified as a simple list: `campaign_name: [task1, task2]`
pub type Campaign = Vec<String>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exec_cmdline_list_of_strings() {
        let yaml = "- uname -a\n- hostname\n- whoami\n";
        let result: ExecCmdline = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(result, ExecCmdline::List(ref v) if v.len() == 3));
    }

    #[test]
    fn test_exec_cmdline_script() {
        let yaml = "|\n  echo hello\n  echo world\n";
        let result: ExecCmdline = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(result, ExecCmdline::Script(_)));
    }

    #[test]
    fn test_exec_cmdline_empty() {
        let yaml = "~\n";
        let result: ExecCmdline = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(result, ExecCmdline::Empty));
    }

    #[test]
    fn test_exec_cmdline_colon_space_in_plain_scalar() {
        // `: ` in a YAML plain scalar is interpreted as a mapping key-value separator.
        // This must produce an actionable error, not the generic untagged enum message.
        let yaml = "- echo \"Current dir: $(pwd)\"\n";
        let result: Result<ExecCmdline, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err(), "Should fail: plain scalar with `: ` is a YAML mapping");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("must be quoted"),
            "Error should be actionable: {err}"
        );
    }

    #[test]
    fn test_exec_cmdline_quoted_colon_space() {
        // Properly YAML-quoted string with `: ` works fine
        let yaml = "- 'echo \"Current dir: $(pwd)\"'\n";
        let result: ExecCmdline = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(result, ExecCmdline::List(ref v) if v.len() == 1));
    }
}
