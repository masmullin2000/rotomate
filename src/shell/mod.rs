//! Shell abstraction for executing commands on different platforms.
//!
//! This module provides a `Shell` trait that abstracts command wrapping and
//! privilege escalation across different shells (sh, `PowerShell`, cmd).

mod powershell;
mod sh;

pub use powershell::PowerShellShell;
pub use sh::ShShell;

use crate::config::ShellType;

/// A shell for executing commands on a remote host.
pub trait Shell: Send + Sync {
    /// Wrap a command for execution in this shell.
    fn wrap_command(&self, command: &str) -> String;

    /// Wrap a command for privileged execution (sudo/admin).
    /// Returns the wrapped command and optional stdin data (e.g., password).
    fn wrap_privileged(&self, command: &str, password: Option<&str>) -> (String, Option<String>);
}

/// Get a shell implementation for the given shell type.
pub fn get_shell(shell_type: ShellType) -> Box<dyn Shell> {
    match shell_type {
        ShellType::Sh => Box::new(ShShell),
        ShellType::PowerShell => Box::new(PowerShellShell),
        ShellType::Cmd => Box::new(CmdShell),
    }
}

/// Windows Command Prompt shell (cmd.exe).
/// Note: cmd does not support privilege escalation.
pub struct CmdShell;

impl Shell for CmdShell {
    fn wrap_command(&self, command: &str) -> String {
        // Escape special characters for cmd
        let escaped = escape_for_cmd(command);
        format!("cmd /c \"{escaped}\"")
    }

    fn wrap_privileged(&self, command: &str, _password: Option<&str>) -> (String, Option<String>) {
        // cmd.exe doesn't have built-in privilege escalation like sudo
        // The user would need to connect as Administrator or use runas externally
        (self.wrap_command(command), None)
    }
}

/// Escape a string for use in cmd /c "..."
fn escape_for_cmd(command: &str) -> String {
    // In cmd, we need to escape: ^ & | < > ( ) @ " %
    // For simplicity, we'll just escape % and " which are most common
    command.replace('%', "%%").replace('"', "\"\"")
}
