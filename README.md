# rot

SSH automation tool that runs tasks across multiple remote hosts in parallel using YAML configuration.

### Why "rotomate"

It's a warning.

The yaml scripts you write to use rotomate will rot.  You will build up a set
of scripts that you will constantly tweak and modify as your infrastructure changes.
You'll have a different file that you upload, and you will just hard-code it in
a given task.  You'll then need to upload a different file for a new purpose,
and re-hard-code it in overwriting the previous location, this will break your previous script.

Hopefully those changes are just to keys or hosts, and not to the actual tasks that you run.
But that's the risk.

## Building

Requires Rust and [just](https://github.com/casey/just).

```bash
just build                      # Debug build
just build release              # Release build (musl)
just build release gnu          # Release build (static glibc binary)
```

## Usage

```bash
rot config.yaml                    # Run tasks (default)
rot config.yaml --check            # Validate config
rot config.yaml -v                 # Verbose output
rot config.yaml --list             # List hosts/tasks/groups
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
