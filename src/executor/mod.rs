mod execute;
mod host;
mod result;

pub mod task;

pub use execute::Executor;
pub use host::Host;
pub use result::{
    CommandResult, CopyResult, DeleteResult, HostResult, ProxmoxResult, TaskExecution,
    TaskGroupResult, TaskResult,
};
