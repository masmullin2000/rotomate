use std::collections::HashSet;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

use rotomate::ProxmoxClient;
use rotomate::config;
use rotomate::config::OsType;
use rotomate::executor::Executor;
use rotomate::output::OutputFormatter;

#[derive(Parser)]
#[command(name = "rot")]
#[command(about = "SSH automation tool for running tasks across multiple hosts")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run tasks from configuration file(s)
    Run {
        /// Path to YAML configuration file(s)
        #[arg(required = true)]
        config: Vec<PathBuf>,

        /// Run only a specific task (by name)
        #[arg(short, long)]
        task: Option<String>,

        /// Run only `task_groups` in the specified campaign
        #[arg(long)]
        campaign: Option<String>,

        /// Show detailed output from each host (streams output in real-time)
        #[arg(short, long)]
        verbose: bool,

        /// Capture and display output when task completes
        #[arg(long)]
        capture_output: bool,

        /// Prompt for sudo password (required for tasks with `become_root: true`)
        #[arg(long)]
        root: bool,
    },

    /// Validate configuration file(s) without executing
    Check {
        /// Path to YAML configuration file(s)
        #[arg(required = true)]
        config: Vec<PathBuf>,
    },

    /// List hosts, tasks, or run groups from configuration
    List {
        /// Path to YAML configuration file(s)
        #[arg(required = true)]
        config: Vec<PathBuf>,

        /// What to list
        #[arg(value_enum)]
        what: ListType,

        /// Show source file for each item
        #[arg(short, long)]
        verbose: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum ListType {
    /// List everything (hosts, tasks, groups, campaigns)
    All,
    /// List all hosts
    Hosts,
    /// List all tasks
    Tasks,
    /// List all task groups
    Groups,
    /// List all campaigns
    Campaigns,
}

fn main() -> Result<()> {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
            config,
            task,
            campaign,
            verbose,
            capture_output,
            root,
        } => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(run_command(
                &config,
                task,
                campaign,
                verbose,
                capture_output,
                root,
            ))
        }
        Commands::Check { config } => check_command(&config),
        Commands::List {
            config,
            what,
            verbose,
        } => list_command(&config, what, verbose),
    }
}

fn load_configs(paths: &[PathBuf]) -> Result<config::schema::Config> {
    config::load_configs(paths)
}

/// Determine which campaign to use based on available campaigns and user request.
/// - No campaigns defined: returns None (run all `task_groups`)
/// - Campaigns exist + no request: auto-selects the first one (by definition order)
/// - Campaigns exist + request: use the specified campaign
fn determine_campaign(
    campaigns: &indexmap::IndexMap<String, config::schema::Campaign>,
    requested: Option<String>,
) -> Result<Option<String>> {
    if campaigns.is_empty() {
        if let Some(name) = requested {
            anyhow::bail!("Campaign '{name}' requested but no campaigns are defined in config");
        }
        return Ok(None);
    }

    // Get campaign name - either requested or first by definition order
    let name = if let Some(name) = requested {
        if !campaigns.contains_key(&name) {
            let available: Vec<_> = campaigns.keys().collect();
            anyhow::bail!(
                "Campaign '{name}' not found.\nAvailable campaigns: {}",
                available
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        name
    } else {
        // Auto-select the first campaign (by definition order in YAML)
        let name = campaigns.keys().next().unwrap().clone();
        eprintln!("Auto-selecting campaign: {name}");
        name
    };

    Ok(Some(name))
}

fn prompt_password(prompt: &str) -> Result<String> {
    use std::io::IsTerminal;

    eprint!("{prompt}");
    io::stderr().flush()?;

    // Read from TTY if available, otherwise from stdin (for piped input)
    let password = if io::stdin().is_terminal() {
        rpassword::read_password()?
    } else {
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        line.trim_end().to_string()
    };
    Ok(password)
}

#[allow(clippy::too_many_lines)]
async fn run_command(
    config_paths: &[PathBuf],
    task_name: Option<String>,
    campaign_name: Option<String>,
    verbose: bool,
    capture_output: bool,
    root: bool,
) -> Result<()> {
    let start_time = Instant::now();

    // Load raw configuration
    let raw_config = load_configs(config_paths)?;

    // Create Proxmox client if configured
    let proxmox_client = if let Some(ref proxmox_cfg) = raw_config.proxmox {
        Some(ProxmoxClient::new(
            &proxmox_cfg.url,
            &proxmox_cfg.token_id,
            &proxmox_cfg.token_secret,
            proxmox_cfg.verify_ssl,
            proxmox_cfg.timeout,
        )?)
    } else {
        None
    };

    // Determine which campaign to use (auto-selection logic)
    let effective_campaign = determine_campaign(&raw_config.campaigns, campaign_name)?;

    // Resolve configuration with optional campaign filter
    let mut config = config::Config::resolve(raw_config, effective_campaign.as_deref())?;

    // Check if we're running a local-only task (skip SSH/sudo prompts if so)
    let is_local_only_task = task_name.as_ref().is_some_and(|name| {
        config.tasks.get(name).is_some_and(|task| {
            !task.local_command.is_empty()
                && task.proxmox_command.is_empty()
                && task.remote_command.is_empty()
                && task.upload.is_empty()
                && task.download.is_empty()
        })
    });

    // Collect the set of hosts that will actually be used
    let used_hosts: HashSet<String> = if let Some(ref name) = task_name {
        // When running a specific task, only collect hosts for that task's task items
        config
            .task_groups
            .iter()
            .flat_map(|g| &g.tasks)
            .filter(|t| &t.task_name == name)
            .flat_map(|t| t.host_names.iter().cloned())
            .collect()
    } else {
        // When running all tasks, collect hosts from all task groups
        config
            .task_groups
            .iter()
            .flat_map(|g| &g.tasks)
            .flat_map(|t| t.host_names.iter().cloned())
            .collect()
    };

    // Check if any tasks require sudo (become_root: true)
    let sudo_required = config.tasks.values().any(|task| task.become_root);

    // Only prompt for global sudo password if --root flag is set AND there are tasks needing sudo
    let sudo_password = if root && sudo_required {
        Some(prompt_password("Sudo password: ")?)
    } else {
        None
    };

    if !is_local_only_task {
        // Build a map of host -> whether any task on that host needs become_root
        let hosts_needing_privilege: HashSet<String> = config
            .task_groups
            .iter()
            .flat_map(|g| &g.tasks)
            .filter(|t| {
                config
                    .tasks
                    .get(&t.task_name)
                    .is_some_and(|task| task.become_root)
            })
            .flat_map(|t| t.host_names.iter().cloned())
            .collect();

        // Prompt for SSH passwords when no private_key is available (only for used hosts)
        for host in config.hosts.values_mut() {
            if used_hosts.contains(&host.ctx.name) && host.private_key.is_none() {
                let prompt = format!(
                    "SSH password for '{}@{}': ",
                    host.ctx.username, host.ctx.name
                );
                host.ssh_password = Some(prompt_password(&prompt)?);
            }
        }

        // Prompt for host-specific sudo passwords (only for hosts running tasks that need sudo)
        for host in config.hosts.values_mut() {
            if used_hosts.contains(&host.ctx.name)
                && host.sudo_prompt
                && hosts_needing_privilege.contains(&host.ctx.name)
            {
                let prompt = format!("Sudo password for '{}': ", host.ctx.name);
                host.sudo_password = Some(prompt_password(&prompt)?);
            }
        }

        // Prompt for Windows admin passwords (only for Windows hosts running tasks that need become_root)
        for host in config.hosts.values_mut() {
            if used_hosts.contains(&host.ctx.name)
                && host.ctx.os == OsType::Windows
                && host.admin_prompt
                && hosts_needing_privilege.contains(&host.ctx.name)
            {
                let prompt = format!("Admin password for '{}': ", host.ctx.name);
                host.admin_password = Some(prompt_password(&prompt)?);
            }
        }
    }

    // Collect all host names for the formatter
    let host_names: Vec<String> = config.hosts.keys().cloned().collect();
    let formatter = OutputFormatter::new(&host_names);

    // Create executor
    let executor = Executor::new(
        config,
        sudo_password,
        verbose,
        capture_output,
        proxmox_client,
    );

    // Callback to print when a host starts a task
    let on_host_start = |host_name: &str, task_name: &str, inactivity_timeout: u64| {
        formatter.print_task_start(host_name, task_name, inactivity_timeout);
    };

    // Callback to print results as each host completes
    let on_host_complete = |task_name: &str, host_result: &rotomate::executor::HostResult| {
        formatter.print_task_end(&host_result.host_name, task_name, host_result);
    };

    // Run tasks
    let results = if let Some(task) = task_name {
        match executor
            .run_task(&task, on_host_start, on_host_complete)
            .await
        {
            Some(task_execution) => {
                // Wrap single task execution in a synthetic task group
                vec![rotomate::executor::TaskGroupResult {
                    group_name: task.clone(),
                    tasks: vec![task_execution],
                }]
            }
            None => {
                anyhow::bail!("Task '{task}' not found in configuration");
            }
        }
    } else {
        executor.run_all(on_host_start, on_host_complete).await
    };

    // Display captured output after all tasks complete (if capture_output enabled)
    if capture_output {
        println!();
        println!("=== Captured Output ===");
        println!();
        for result in &results {
            formatter.print_detailed(result);
        }
    }

    // Display final summary
    let mut all_success = true;
    for result in &results {
        formatter.print_summary(result);
        if !result.success() {
            all_success = false;
        }
    }

    // Display total execution time
    let total_secs = start_time.elapsed().as_secs_f64();
    let total_time = if total_secs < 60.0 {
        format!("{total_secs:.2}s")
    } else {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let mins = (total_secs / 60.0).floor() as u64;
        #[allow(clippy::cast_precision_loss)]
        let remaining_secs = (mins as f64).mul_add(-60.0, total_secs);
        format!("{mins}m {remaining_secs:.2}s")
    };
    println!();
    println!("Total time: {total_time}");

    if all_success {
        Ok(())
    } else {
        std::process::exit(1);
    }
}

fn check_command(config_paths: &[PathBuf]) -> Result<()> {
    // Load and resolve configuration
    let config = load_configs(config_paths)?;
    let resolved: config::Config = config.try_into()?;

    println!("Configuration is valid!");
    println!();
    println!("Hosts ({}):", resolved.hosts.len());
    for (name, host) in &resolved.hosts {
        let os_info = match host.ctx.os {
            OsType::Linux => "",
            OsType::Windows => " [windows]",
        };
        println!(
            "  - {}: {}:{} (user: {}){os_info}",
            name, host.ctx.hostname, host.ctx.port, host.ctx.username
        );
    }

    println!();
    println!("Tasks ({}):", resolved.tasks.len());
    for (name, task) in &resolved.tasks {
        println!(
            "  - {}: {} command(s){}",
            name,
            task.remote_command.len(),
            if task.description.is_empty() {
                String::new()
            } else {
                format!(" - {}", task.description)
            }
        );
    }

    println!();
    let total_tasks: usize = resolved.task_groups.iter().map(|g| g.tasks.len()).sum();
    println!(
        "Task groups ({} groups, {} tasks total):",
        resolved.task_groups.len(),
        total_tasks
    );
    for group in &resolved.task_groups {
        println!("  {}:", group.name);
        for task_item in &group.tasks {
            println!(
                "    - task '{}' on {} host(s)",
                task_item.task_name,
                task_item.host_names.len()
            );
        }
    }

    Ok(())
}

fn list_command(config_paths: &[PathBuf], what: ListType, verbose: bool) -> Result<()> {
    let config = load_configs(config_paths)?;
    let resolved: config::Config = config.try_into()?;

    match what {
        ListType::All => {
            println!("=== Hosts ===");
            list_hosts(&resolved, verbose);
            println!("\n=== Tasks ===");
            list_tasks(&resolved, verbose);
            println!("\n=== Groups ===");
            list_groups(&resolved, verbose);
            println!("\n=== Campaigns ===");
            list_campaigns(&resolved, verbose);
        }
        ListType::Hosts => list_hosts(&resolved, verbose),
        ListType::Tasks => list_tasks(&resolved, verbose),
        ListType::Groups => list_groups(&resolved, verbose),
        ListType::Campaigns => list_campaigns(&resolved, verbose),
    }

    Ok(())
}

fn format_source(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_else(|| path.to_str().unwrap_or("unknown"))
        .to_string()
}

fn list_hosts(resolved: &config::Config, verbose: bool) {
    if resolved.hosts.is_empty() {
        println!("No hosts defined");
        return;
    }

    let mut names: Vec<_> = resolved.hosts.keys().collect();

    // Sort: by source file first (if verbose), then alphabetically by name
    names.sort_by(|a, b| {
        if verbose {
            let source_a = resolved.hosts_source.get(*a).map(|p| format_source(p));
            let source_b = resolved.hosts_source.get(*b).map(|p| format_source(p));
            source_a.cmp(&source_b).then_with(|| a.cmp(b))
        } else {
            a.cmp(b)
        }
    });

    let max_name_len = names.iter().map(|n| n.len()).max().unwrap_or(0);

    for name in names {
        if verbose {
            let source = resolved
                .hosts_source
                .get(name)
                .map(|p| format!("[{}]", format_source(p)))
                .unwrap_or_default();
            println!("{name:max_name_len$}  {source}");
        } else {
            println!("{name}");
        }
    }
}

fn list_tasks(resolved: &config::Config, verbose: bool) {
    if resolved.tasks.is_empty() {
        println!("No tasks defined");
        return;
    }

    let mut task_names: Vec<_> = resolved.tasks.keys().collect();

    // Sort: by source file first (if verbose), then alphabetically by name
    task_names.sort_by(|a, b| {
        if verbose {
            let source_a = resolved.tasks_source.get(*a).map(|p| format_source(p));
            let source_b = resolved.tasks_source.get(*b).map(|p| format_source(p));
            source_a.cmp(&source_b).then_with(|| a.cmp(b))
        } else {
            a.cmp(b)
        }
    });

    let max_name_len = resolved.tasks.keys().map(String::len).max().unwrap_or(0);
    let max_desc_len = resolved
        .tasks
        .values()
        .map(|t| t.description.len())
        .max()
        .unwrap_or(0);

    for name in task_names {
        let task = &resolved.tasks[name];
        if verbose {
            let source = resolved
                .tasks_source
                .get(name)
                .map(|p| format!("[{}]", format_source(p)))
                .unwrap_or_default();
            println!(
                "{:name_w$}  {:desc_w$}  {source}",
                name,
                task.description,
                name_w = max_name_len,
                desc_w = max_desc_len
            );
        } else if task.description.is_empty() {
            println!("{name}");
        } else {
            println!(
                "{:width$}  {}",
                name,
                task.description,
                width = max_name_len
            );
        }
    }
}

fn list_groups(resolved: &config::Config, verbose: bool) {
    if resolved.task_groups.is_empty() {
        println!("No task groups defined");
        return;
    }

    let mut groups: Vec<_> = resolved.task_groups.iter().collect();

    // Sort: by source file first (if verbose), then alphabetically by key
    groups.sort_by(|a, b| {
        if verbose {
            let source_a = resolved.groups_source.get(&a.key).map(|p| format_source(p));
            let source_b = resolved.groups_source.get(&b.key).map(|p| format_source(p));
            source_a.cmp(&source_b).then_with(|| a.key.cmp(&b.key))
        } else {
            a.key.cmp(&b.key)
        }
    });

    let max_key_len = resolved
        .task_groups
        .iter()
        .map(|g| g.key.len())
        .max()
        .unwrap_or(0);
    let max_name_len = resolved
        .task_groups
        .iter()
        .map(|g| g.name.len())
        .max()
        .unwrap_or(0);

    for group in groups {
        if verbose {
            let source = resolved
                .groups_source
                .get(&group.key)
                .map(|p| format!("[{}]", format_source(p)))
                .unwrap_or_default();
            println!(
                "{:key_w$}  {:name_w$}  {source}",
                group.key,
                group.name,
                key_w = max_key_len,
                name_w = max_name_len
            );
        } else {
            println!("{:width$}  {}", group.key, group.name, width = max_key_len);
        }
    }
}

fn list_campaigns(resolved: &config::Config, verbose: bool) {
    if resolved.campaigns.is_empty() {
        println!("No campaigns defined");
        return;
    }

    // First campaign by definition order is the primary (auto-selected) one
    let primary_name = resolved.campaigns.keys().next().map(String::as_str);

    let max_name_len = resolved
        .campaigns
        .keys()
        .map(String::len)
        .max()
        .unwrap_or(0);

    // Iterate in definition order (IndexMap preserves insertion order)
    for (name, campaign) in &resolved.campaigns {
        let primary_marker = if Some(name.as_str()) == primary_name { " (primary)" } else { "" };
        if verbose {
            let source = resolved
                .campaigns_source
                .get(name)
                .map(|p| format!("[{}]", format_source(p)))
                .unwrap_or_default();
            println!("{name:max_name_len$}{primary_marker}  {source}");
        } else {
            println!("{name}{primary_marker}");
        }
        for target in campaign {
            println!("  - {target}");
        }
    }
}
