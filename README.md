# rot

SSH automation tool that runs tasks across multiple remote hosts in parallel using YAML configuration.

## Building

Requires Rust and [just](https://github.com/casey/just).

```bash
just build                      # Debug build
just build release              # Release build (musl)
just build release gnu          # Release build (static glibc binary)
```

## Usage

```bash
rot check config.yaml           # Validate config
rot run config.yaml             # Run tasks
rot run config.yaml -v          # Verbose output
rot list config.yaml hosts      # List hosts/tasks/groups
```

## Documentation

- [Quickstart Guide](docs/QUICKSTART.md) - Tutorial for creating your first script
- [Configuration Guide](docs/CONFIGURATION.md) - Complete reference

## Claude Code Integration

rot includes a Claude Code skill that helps generate configuration files through natural language.

### Installation

Copy the skill directory to your Claude Code skills folder:

```bash
cp -r .claude/skills/rot-config ~/.claude/skills/
```

Or symlink it for automatic updates:

```bash
ln -s "$(pwd)/.claude/skills/rot-config" ~/.claude/skills/rot-config
```

### Usage

In Claude Code, invoke the skill with `/rot-config` followed by a description of what you want to automate:

```
/rot-config deploy my app to three web servers with file upload
```

```
/rot-config run database backups on all postgres servers nightly
```

```
/rot-config configure windows servers with powershell scripts
```

The skill will:
1. Ask clarifying questions about hosts, authentication, and tasks
2. Generate appropriate YAML configuration files
3. Write them to your project's `config/` directory
4. Provide validation and run commands

### What It Generates

- **Simple configs**: Single-file YAML with hosts, tasks, and task_groups
- **Complex configs**: Multi-file setup with separate hosts.yaml, tasks.yaml, and main.yaml
- **Proxmox integration**: VM management with snapshots and rollback
- **Windows support**: PowerShell/CMD configurations with admin elevation
