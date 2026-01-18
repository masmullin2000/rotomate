#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::similar_names)]

pub mod config;
pub mod executor;
pub mod output;
pub mod proxmox;
pub mod shell;
pub mod ssh;

pub use config::{Campaign, Config, Defaults, HostRef, Task, TaskGroupItem};
pub use executor::{
    CommandResult, CopyResult, Executor, Host, HostResult, ProxmoxResult, TaskExecution,
    TaskGroupResult, TaskResult,
};
pub use output::OutputFormatter;
pub use proxmox::ProxmoxClient;
pub use ssh::Session;
