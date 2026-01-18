//! POSIX shell (sh) implementation.

use super::Shell;

/// POSIX shell (sh) for Linux/Unix systems.
pub struct ShShell;

impl Shell for ShShell {
    fn wrap_command(&self, command: &str) -> String {
        let escaped = escape_for_sh(command);
        format!("sh -c '{escaped}'")
    }

    fn wrap_privileged(&self, command: &str, password: Option<&str>) -> (String, Option<String>) {
        let escaped = escape_for_sh(command);

        password.map_or_else(
            || {
                // Assume NOPASSWD is configured - no password needed
                let sudo_cmd = format!("sudo sh -c '{escaped}'");
                (sudo_cmd, None)
            },
            |pwd| {
                // Run command with sudo -S -p '' sh -c '...' to handle redirections properly
                let sudo_cmd = format!("sudo -S -p '' sh -c '{escaped}'");
                let stdin_data = format!("{pwd}\n");
                (sudo_cmd, Some(stdin_data))
            },
        )
    }
}

/// Escape a command for use inside single quotes in sh -c '...'
fn escape_for_sh(command: &str) -> String {
    // Replace single quotes with '\'' (end quote, escaped quote, start quote)
    command.replace('\'', "'\\''")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_for_sh() {
        assert_eq!(escape_for_sh("echo hello"), "echo hello");
        assert_eq!(escape_for_sh("echo 'hello'"), "echo '\\''hello'\\''");
        assert_eq!(
            escape_for_sh("echo \"hello world\""),
            "echo \"hello world\""
        );
    }

    #[test]
    fn test_wrap_command() {
        let shell = ShShell;
        assert_eq!(shell.wrap_command("echo hello"), "sh -c 'echo hello'");
        assert_eq!(
            shell.wrap_command("echo 'test'"),
            "sh -c 'echo '\\''test'\\'''"
        );
    }

    #[test]
    fn test_wrap_privileged_with_password() {
        let shell = ShShell;
        let (cmd, stdin) = shell.wrap_privileged("systemctl restart nginx", Some("secret123"));
        assert_eq!(cmd, "sudo -S -p '' sh -c 'systemctl restart nginx'");
        assert_eq!(stdin, Some("secret123\n".to_string()));
    }

    #[test]
    fn test_wrap_privileged_without_password() {
        let shell = ShShell;
        let (cmd, stdin) = shell.wrap_privileged("systemctl restart nginx", None);
        assert_eq!(cmd, "sudo sh -c 'systemctl restart nginx'");
        assert_eq!(stdin, None);
    }
}
