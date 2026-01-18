//! `PowerShell` implementation for Windows systems.

use super::Shell;

/// `PowerShell` for Windows systems.
pub struct PowerShellShell;

impl Shell for PowerShellShell {
    fn wrap_command(&self, command: &str) -> String {
        let escaped = escape_for_powershell(command);
        // Append `exit $LASTEXITCODE` to propagate native command exit codes.
        // Without this, PowerShell exits 0 even when native commands (sc.exe, etc.) fail.
        format!("powershell -NoProfile -NonInteractive -Command \"{escaped}; exit $LASTEXITCODE\"")
    }

    fn wrap_privileged(&self, command: &str, password: Option<&str>) -> (String, Option<String>) {
        // For Windows privilege escalation, we use Start-Process with -Verb RunAs
        // However, this approach has limitations in non-interactive SSH sessions.
        // The more reliable approach for automation is to use PSCredential.
        //
        // For now, we'll use a simpler approach that works when the user has
        // appropriate privileges or when UAC is configured to allow elevation.

        password.map_or_else(
            || {
                // Without password, just run the command normally
                // The user is expected to be running as Administrator already
                (self.wrap_command(command), None)
            },
            |pwd| {
                // Use PSCredential with Start-Process for elevation
                // This creates an admin process with the provided credentials
                // First escape for double-quoted outer string, then escape single quotes
                // for use inside the single-quoted ArgumentList
                let escaped_cmd = escape_for_single_quotes(&escape_for_powershell(command));
                let escaped_pwd = escape_for_single_quotes(&escape_for_powershell(pwd));

                let ps_script = format!(
                    "$secpasswd = ConvertTo-SecureString '{escaped_pwd}' -AsPlainText -Force; \
                     $cred = New-Object System.Management.Automation.PSCredential ($env:USERNAME, $secpasswd); \
                     Start-Process powershell -Credential $cred -ArgumentList '-NoProfile','-NonInteractive','-Command','{escaped_cmd}; exit $LASTEXITCODE' -Wait -NoNewWindow -PassThru | ForEach-Object {{ exit $_.ExitCode }}"
                );
                let wrapped = format!(
                    "powershell -NoProfile -NonInteractive -Command \"{ps_script}\""
                );
                (wrapped, None)
            },
        )
    }
}

/// Escape a string for use inside `PowerShell` double-quoted strings.
/// When `PowerShell` is invoked via cmd.exe (e.g., `powershell -Command "..."`),
/// the command line is first parsed by cmd.exe, then by `PowerShell`.
/// We use `\"` which cmd.exe interprets as a literal quote inside a quoted string,
/// and `PowerShell` then sees the unescaped quote as intended.
fn escape_for_powershell(command: &str) -> String {
    command
        .replace('`', "``") // Escape backticks first
        .replace('"', "\\\"") // Escape double quotes for cmd.exe parsing
        .replace('$', "`$") // Escape dollar signs
}

/// Escape a string for use inside `PowerShell` single-quoted strings.
/// Single quotes are doubled to escape them.
fn escape_for_single_quotes(command: &str) -> String {
    command.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_for_powershell() {
        assert_eq!(escape_for_powershell("echo hello"), "echo hello");
        assert_eq!(escape_for_powershell("$var"), "`$var");
        assert_eq!(escape_for_powershell("\"quoted\""), "\\\"quoted\\\"");
        assert_eq!(escape_for_powershell("`backtick`"), "``backtick``");
    }

    #[test]
    fn test_wrap_command() {
        let shell = PowerShellShell;
        assert_eq!(
            shell.wrap_command("Get-Process"),
            "powershell -NoProfile -NonInteractive -Command \"Get-Process; exit $LASTEXITCODE\""
        );
        assert_eq!(
            shell.wrap_command("Get-ChildItem C:\\"),
            "powershell -NoProfile -NonInteractive -Command \"Get-ChildItem C:\\; exit $LASTEXITCODE\""
        );
    }

    #[test]
    fn test_wrap_command_with_variable() {
        let shell = PowerShellShell;
        assert_eq!(
            shell.wrap_command("Write-Host $env:PATH"),
            "powershell -NoProfile -NonInteractive -Command \"Write-Host `$env:PATH; exit $LASTEXITCODE\""
        );
    }

    #[test]
    fn test_wrap_privileged_without_password() {
        let shell = PowerShellShell;
        let (cmd, stdin) = shell.wrap_privileged("Restart-Service Spooler", None);
        assert_eq!(
            cmd,
            "powershell -NoProfile -NonInteractive -Command \"Restart-Service Spooler; exit $LASTEXITCODE\""
        );
        assert_eq!(stdin, None);
    }
}
