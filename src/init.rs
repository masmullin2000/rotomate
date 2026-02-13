use std::fmt::Write as _;
use std::io::{self, BufRead, Write};
use std::path::Path;

use anyhow::{Result, bail};

struct HostEntry {
    name: String,
    hostname: String,
}

struct TaskEntry {
    name: String,
    commands: Vec<String>,
}

struct ConfigBuilder {
    username: String,
    port: u16,
    private_key: String,
    hosts: Vec<HostEntry>,
    tasks: Vec<TaskEntry>,
    /// For each task, the host reference (a group name, single host, or comma-separated list)
    task_host_refs: Vec<String>,
}

#[allow(clippy::too_many_lines)]
pub fn run(output: &Path) -> Result<()> {
    let stdin = io::stdin();
    let mut reader = stdin.lock();

    println!();
    println!("rot init — generate a rotomate configuration");
    println!("=============================================");
    println!();
    println!("Output file: {}", output.display());

    // Check for existing file before asking any questions
    if output.exists() {
        let overwrite = prompt_yes_no(
            &mut reader,
            &format!("{} already exists. Overwrite?", output.display()),
            false,
        )?;
        if !overwrite {
            println!("Aborted.");
            return Ok(());
        }
    }

    // --- Defaults ---
    println!();
    println!("— Defaults —");

    let current_user = whoami::username().unwrap_or_else(|_| "user".to_string());
    let username = prompt(&mut reader, "SSH username", &current_user)?;

    let port_str = prompt(&mut reader, "SSH port", "22")?;
    let port: u16 = port_str
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid port number: {port_str}"))?;
    if port == 0 {
        bail!("Port must be between 1 and 65535");
    }

    let private_key = prompt(&mut reader, "SSH private key", "~/.ssh/id_ed25519")?;

    // Warn if key file doesn't exist (but don't block)
    let expanded_key = shellexpand::tilde(&private_key);
    if !Path::new(expanded_key.as_ref()).exists() {
        eprintln!("  Warning: {expanded_key} not found (you can fix this later)");
    }

    // --- Hosts ---
    println!();
    println!("— Hosts —");

    let mut hosts = Vec::new();
    loop {
        let name = prompt_nonempty(&mut reader, "Host name (a label for this host)")?;
        if !is_valid_identifier(&name) {
            eprintln!("  Invalid name: use letters, digits, underscores, hyphens (start with letter or _)");
            continue;
        }
        if hosts.iter().any(|h: &HostEntry| h.name == name) {
            eprintln!("  Host '{name}' already defined, choose a different name");
            continue;
        }
        let hostname = prompt_nonempty(&mut reader, "Hostname or IP")?;
        hosts.push(HostEntry { name, hostname });

        if !prompt_yes_no(&mut reader, "Add another host?", false)? {
            break;
        }
    }

    // --- Tasks ---
    println!();
    println!("— Tasks —");

    let mut tasks = Vec::new();
    loop {
        let name = prompt_nonempty(&mut reader, "Task name")?;
        if !is_valid_identifier(&name) {
            eprintln!("  Invalid name: use letters, digits, underscores, hyphens (start with letter or _)");
            continue;
        }
        if tasks.iter().any(|t: &TaskEntry| t.name == name) {
            eprintln!("  Task '{name}' already defined, choose a different name");
            continue;
        }
        let commands = prompt_multiline(&mut reader, "Command (empty line to finish)")?;
        if commands.is_empty() {
            eprintln!("  At least one command is required");
            continue;
        }
        tasks.push(TaskEntry { name, commands });

        if !prompt_yes_no(&mut reader, "Add another task?", false)? {
            break;
        }
    }

    // --- Wiring ---
    println!();
    println!("— Wiring —");

    let host_names: Vec<&str> = hosts.iter().map(|h| h.name.as_str()).collect();
    let mut task_host_refs = Vec::new();

    for task in &tasks {
        let label = format!(
            "Which hosts should task '{}' run on? (comma-separated, or 'all') [all]",
            task.name
        );
        let input = prompt(&mut reader, &label, "all")?;
        let host_ref = resolve_host_ref(&input, &host_names)?;
        task_host_refs.push(host_ref);
    }

    drop(reader);

    let builder = ConfigBuilder {
        username,
        port,
        private_key,
        hosts,
        tasks,
        task_host_refs,
    };

    let yaml = builder.render();
    std::fs::write(output, &yaml)?;

    println!();
    println!("Wrote {}", output.display());
    println!("Validate: rot {} --check", output.display());
    println!("Run:      rot {} -v", output.display());

    Ok(())
}

fn prompt(reader: &mut impl BufRead, label: &str, default: &str) -> Result<String> {
    eprint!("{label} [{default}]: ");
    io::stderr().flush()?;
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

fn prompt_nonempty(reader: &mut impl BufRead, label: &str) -> Result<String> {
    loop {
        eprint!("{label}: ");
        io::stderr().flush()?;
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            eprintln!("  This field is required");
            continue;
        }
        return Ok(trimmed.to_string());
    }
}

fn prompt_yes_no(reader: &mut impl BufRead, label: &str, default_yes: bool) -> Result<bool> {
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    eprint!("{label} {hint}: ");
    io::stderr().flush()?;
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let trimmed = line.trim().to_lowercase();
    if trimmed.is_empty() {
        Ok(default_yes)
    } else {
        Ok(trimmed.starts_with('y'))
    }
}

fn prompt_multiline(reader: &mut impl BufRead, label: &str) -> Result<Vec<String>> {
    eprintln!("{label}");
    let mut commands = Vec::new();
    loop {
        eprint!("> ");
        io::stderr().flush()?;
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        commands.push(trimmed.to_string());
    }
    Ok(commands)
}

fn is_valid_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Resolve user input like "all", "web,db", or "webserver" into the host ref string
/// used in the generated YAML.
fn resolve_host_ref(input: &str, host_names: &[&str]) -> Result<String> {
    let input = input.trim();
    if input.eq_ignore_ascii_case("all") {
        // If there's only one host, reference it directly
        if host_names.len() == 1 {
            return Ok(host_names[0].to_string());
        }
        return Ok("all".to_string());
    }

    // Comma-separated list — validate each name
    let refs: Vec<&str> = input.split(',').map(str::trim).collect();
    for r in &refs {
        if !host_names.contains(r) {
            bail!("Unknown host: '{r}'. Available: {}", host_names.join(", "));
        }
    }

    if refs.len() == 1 {
        Ok(refs[0].to_string())
    } else {
        Ok(refs.join(", "))
    }
}

impl ConfigBuilder {
    fn render(&self) -> String {
        let mut out = String::new();

        out.push_str("# rotomate configuration — edit and extend as needed\n");
        out.push_str("# Docs: rot <this-file> --check to validate, rot <this-file> -v to run\n");
        out.push('\n');

        // Defaults
        out.push_str("defaults:\n");
        let _ = writeln!(out, "  username: {}", self.username);
        let _ = writeln!(out, "  port: {}", self.port);
        let _ = writeln!(out, "  private_key: {}", self.private_key);
        out.push('\n');

        // Hosts
        out.push_str("hosts:\n");
        for host in &self.hosts {
            let _ = writeln!(out, "  {}:", host.name);
            let _ = writeln!(out, "    hostname: {}", host.hostname);
        }
        out.push('\n');

        // Groups — only if 2+ hosts
        if self.hosts.len() >= 2 {
            out.push_str("groups:\n");
            let names: Vec<&str> = self.hosts.iter().map(|h| h.name.as_str()).collect();
            let _ = writeln!(out, "  all: [{}]", names.join(", "));
            out.push('\n');
        }

        // Tasks
        out.push_str("tasks:\n");
        for task in &self.tasks {
            let _ = writeln!(out, "  {}:", task.name);
            out.push_str("    remote_command:\n");
            for cmd in &task.commands {
                let _ = writeln!(out, "      - {}", yaml_escape(cmd));
            }
        }
        out.push('\n');

        // Task groups
        out.push_str("task_groups:\n");
        out.push_str("  main:\n");
        out.push_str("    - name: \"Main\"\n");
        for (task, host_ref) in self.tasks.iter().zip(self.task_host_refs.iter()) {
            let _ = writeln!(out, "    - task: {}", task.name);
            if host_ref.contains(',') {
                let refs: Vec<&str> = host_ref.split(',').map(str::trim).collect();
                let _ = writeln!(out, "      hosts: [{}]", refs.join(", "));
            } else {
                let _ = writeln!(out, "      hosts: {host_ref}");
            }
        }

        out
    }
}

/// Escape a YAML string value if it contains special characters.
fn yaml_escape(s: &str) -> String {
    if s.contains(':')
        || s.contains('#')
        || s.contains('{')
        || s.contains('}')
        || s.contains('[')
        || s.contains(']')
        || s.contains('\'')
        || s.contains('"')
        || s.contains('&')
        || s.contains('*')
        || s.contains('?')
        || s.contains('|')
        || s.contains('>')
        || s.contains('!')
        || s.contains('%')
        || s.contains('@')
        || s.starts_with(' ')
        || s.ends_with(' ')
    {
        let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    } else {
        s.to_string()
    }
}
