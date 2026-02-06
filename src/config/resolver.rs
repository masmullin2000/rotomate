use indexmap::IndexMap;
use std::collections::HashMap;

use crate::executor::Host;

use super::schema::{self, Campaign, Task};

use crate::config::schema::{OsType, ShellType};

/// Host context available during task execution.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HostContext {
    pub name: String,
    pub hostname: String,
    pub username: String,
    pub port: u16,
    #[serde(skip)]
    pub os: OsType,
    #[serde(skip)]
    pub shell: ShellType,
}

/// A resolved configuration with all defaults applied and references expanded.
#[derive(Debug, Clone)]
pub struct Config {
    pub hosts: HashMap<String, Host>,
    pub tasks: HashMap<String, Task>,
    /// Task groups - each group executes in parallel, tasks within a group execute sequentially.
    pub task_groups: Vec<TaskGroup>,
    /// SSH inactivity timeout in seconds.
    pub inactivity_timeout: u64,
    /// Template variables for runtime rendering.
    pub vars: HashMap<String, serde_yaml::Value>,
    /// Available campaigns (for listing and selection purposes).
    pub campaigns: IndexMap<String, Campaign>,
    /// Source file tracking for hosts.
    pub hosts_source: HashMap<String, std::path::PathBuf>,
    /// Source file tracking for tasks.
    pub tasks_source: HashMap<String, std::path::PathBuf>,
    /// Source file tracking for `task_groups`.
    pub groups_source: HashMap<String, std::path::PathBuf>,
    /// Source file tracking for campaigns.
    pub campaigns_source: HashMap<String, std::path::PathBuf>,
}

impl Config {
    /// Get hosts for a task group item.
    pub fn hosts(&self, item: &TaskGroupItem) -> impl Iterator<Item = &Host> {
        item.host_names
            .iter()
            .filter_map(|name| self.hosts.get(name))
    }

    /// Get a task by name.
    pub fn get_task(&self, name: &str) -> Option<&Task> {
        self.tasks.get(name)
    }

    /// Resolve a raw config into a resolved config, optionally filtered by campaign.
    /// When `lenient_campaign` is false (default), dependencies not in the campaign are auto-included.
    /// When `lenient_campaign` is true, dependencies are filtered out with warnings.
    pub fn resolve(
        config: schema::Config,
        campaign: Option<&str>,
        lenient_campaign: bool,
    ) -> anyhow::Result<Self> {
        // Validate tasks (e.g., no mixing steps with flat fields)
        for (name, task) in &config.tasks {
            task.validate(name)?;
        }

        let hosts: HashMap<_, _> = config.resolve_hosts().collect::<anyhow::Result<_>>()?;
        let task_groups = config.resolve_task_groups(campaign, lenient_campaign)?;
        let tasks = config.tasks;
        let inactivity_timeout = config.defaults.inactivity_timeout.unwrap_or(15);
        let vars = config.vars;
        let campaigns = config.campaigns;
        let hosts_source = config.hosts_source;
        let tasks_source = config.tasks_source;
        let groups_source = config.groups_source;
        let campaigns_source = config.campaigns_source;

        Ok(Self {
            hosts,
            tasks,
            task_groups,
            inactivity_timeout,
            vars,
            campaigns,
            hosts_source,
            tasks_source,
            groups_source,
            campaigns_source,
        })
    }
}

impl TryFrom<schema::Config> for Config {
    type Error = anyhow::Error;

    fn try_from(config: schema::Config) -> anyhow::Result<Self> {
        Self::resolve(config, None, false)
    }
}

/// A resolved task group containing task items that execute sequentially.
#[derive(Debug, Clone)]
pub struct TaskGroup {
    /// Key of the task group (from YAML config).
    pub key: String,
    /// Display name of the task group.
    pub name: String,
    /// Task items to execute sequentially within this group.
    pub tasks: Vec<TaskGroupItem>,
    /// Keys of task groups that must complete before this group starts.
    pub depends_on: Vec<String>,
}

/// A resolved task group item with host references expanded to actual host names.
#[derive(Debug, Clone)]
pub struct TaskGroupItem {
    pub task_name: String,
    pub host_names: Vec<String>,
}
