# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

This project uses `just` as the task runner. Key commands:

```bash
just build                      # Debug build
just build release              # Release build (glibc)
just build release musl         # Release build (static musl binary)
just test                       # Run tests
just clippy                     # Lint with pedantic and nursery warnings
just check                      # Check without building
```

**Important:** When verifying code compiles, always use `just check` (not `cargo build`).

## Running the Application

```bash
just run -- config/main.yaml              # Run with config file
just run -- config/main.yaml -v           # Verbose output
just run -- config/main.yaml --root       # Prompt for sudo password
just run -- config/main.yaml --check      # Validate config without executing
just run -- config/main.yaml --list       # List hosts, tasks, groups, campaigns

# Multiple config files (merged in order)
just run -- config/hosts.yaml config/tasks.yaml config/runs.yaml
```

## Architecture

rotomate is an SSH automation tool that runs tasks across multiple remote hosts in parallel using YAML configuration.

### Core Components

- **src/main.rs** - CLI entry point using clap. Commands: `run` (execute tasks) and `check` (validate config)

- **src/config/** - Configuration loading and resolution
  - `schema.rs` - Config structs: Config, Defaults, Host, Task, Run, HostRef
  - `loader.rs` - YAML loading with recursive import support and circular detection
  - `resolver.rs` - Converts raw config to ResolvedConfig with defaults applied, expands host groups

- **src/executor/** - Task execution engine
  - `parallel.rs` - Executor runs groups in parallel, runs within groups sequentially
  - `task.rs` - Executes uploads → commands → downloads, handles sudo elevation
  - `result.rs` - Result types: CommandResult, CopyResult, TaskResult, HostResult, RunResult

- **src/ssh/** - SSH client using russh
  - `session.rs` - Session wrapper with connect(), call(), copy_file_to/from_remote()
  - `client.rs` - russh Handler implementation, known_hosts verification

- **src/output/formatter.rs** - Color-coded host output formatting

### Execution Flow

1. Config files loaded and merged (later files override earlier)
2. Imports resolved recursively with circular detection
3. Raw config resolved: defaults applied, host groups expanded
4. Executor runs task groups in parallel
5. Within each group, runs execute sequentially
6. Each run: uploads files → executes commands → downloads files
7. Results streamed via callbacks for real-time output

### Configuration Structure

```yaml
imports: [hosts.yaml, tasks.yaml]  # Optional file imports

defaults:
  port: 22
  username: user
  private_key: ~/.ssh/id_ed25519

hosts:
  webserver:
    hostname: 192.168.1.10

groups:
  all_servers: [webserver, dbserver]

tasks:
  deploy:
    remote_command:
      - echo "deploying"
    upload:
      - "local/file /remote/path"
    become_root: true
    stop_on_error: true

runs:
  main_group:
    - name: "Deploy Application"
    - task: deploy
      hosts: all_servers
```

### Key Patterns

- Async/await with Tokio single-threaded runtime
- Error handling with anyhow::Result
- Tilde expansion (~) in all paths via shellexpand
- Host-specific sudo prompts supported
- SFTP for file transfers via russh-sftp
