mod loader;
pub mod resolver;
pub mod schema;
pub mod template;

pub use loader::load_configs;
pub use resolver::{Config, HostContext, TaskGroup, TaskGroupItem};
pub use schema::{Campaign, Defaults, ExecCmdline, HostRef, OsType, ShellType, Task, TaskStep};
pub use template::{BuiltinContext, RenderedTaskFields, render_steps};
