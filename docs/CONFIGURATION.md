# rot Configuration Guide

rot is an SSH automation tool that runs tasks across multiple remote hosts in parallel using YAML configuration files.

## Quick Start

```bash
# Run tasks from a configuration file (default)
rot config.yaml

# Validate configuration without executing
rot config.yaml --check

# Run with verbose output (streams command output in real-time)
rot config.yaml -v

# Run with sudo password prompt (for become_root tasks)
rot config.yaml --root

# Run multiple config files (merged in order, later files override)
rot hosts.yaml tasks.yaml runs.yaml

# Run a specific campaign
rot config.yaml -c quick_deploy

# Capture output to stdout after completion
rot config.yaml -o std

# Capture output to a file
rot config.yaml -o /tmp/output.txt

# List hosts, tasks, or groups
rot config.yaml --list hosts
rot config.yaml --list tasks
rot config.yaml --list groups
rot config.yaml --list campaigns
rot config.yaml --list all
```

## Configuration File Structure

A rot configuration file is a YAML document with these top-level sections:

```yaml
imports: []           # Other config files to include
vars: {}              # Template variables
proxmox: {}           # Proxmox VE API configuration (optional)
defaults: {}          # Default connection settings
hosts: {}             # Host definitions
groups: {}            # Named groups of hosts
tasks: {}             # Task definitions
task_groups: {}       # Execution groups (what runs and where)
campaigns: {}         # Named subsets of task_groups
```

All sections are optional. Multiple configuration files can be merged, with later files overriding earlier ones.

---

## Imports

Import other configuration files to split your config across multiple files:

```yaml
imports:
  - hosts.yaml           # Relative to current file
  - tasks.yaml
  - /etc/rot/common.yaml # Absolute paths supported
  - ~/shared/hosts.yaml  # Tilde expansion supported
```

Imports are processed recursively. Circular imports are detected and skipped automatically.

---

## Variables (vars)

Define template variables for use throughout your configuration:

```yaml
vars:
  deploy_dir: /opt/myapp
  log_level: debug
  servers:
    - web1
    - web2
```

Variables use Jinja2-style syntax: `{{ variable_name }}`

```yaml
tasks:
  deploy:
    remote_command:
      - mkdir -p {{ deploy_dir }}
      - echo "Log level: {{ log_level }}"
```

Variables can reference other variables:

```yaml
vars:
  base_dir: /opt
  app_name: myapp
  deploy_dir: "{{ base_dir }}/{{ app_name }}"
```

### Built-in Template Functions

**`env(name)`** - Get environment variable (empty string if unset):
```yaml
vars:
  api_key: "{{ env('API_KEY') }}"
```

**`env_or(name, default)`** - Get environment variable with default:
```yaml
vars:
  port: "{{ env_or('PORT', '8080') }}"
```

---

## Defaults

Set default values for all host connections:

```yaml
defaults:
  port: 22                        # SSH port (default: 22)
  username: deploy                # SSH username (default: current user)
  private_key: ~/.ssh/id_ed25519  # Path to SSH private key
  inactivity_timeout: 15          # Seconds before SSH timeout (default: 15)
  os: linux                       # Operating system: linux or windows
  shell: sh                       # Shell type: sh, powershell, or cmd
```

Host-specific settings override these defaults.

---

## Hosts

Define the remote hosts to connect to:

```yaml
hosts:
  webserver:
    hostname: 192.168.1.10        # Required: IP or hostname
    port: 2222                    # Override default port
    username: admin               # Override default username
    private_key: ~/.ssh/web_key   # Override default key
    os: linux                     # linux (default) or windows
    shell: sh                     # sh (default for linux), powershell, or cmd

  database:
    hostname: db.example.com
    sudo_password: "secret"       # Password for become_root tasks
    sudo_prompt: true             # Prompt for sudo password at runtime

  windows_host:
    hostname: win.example.com
    os: windows
    shell: powershell             # powershell (default for windows) or cmd
    admin_prompt: true            # Prompt for admin password at runtime
    admin_password: "secret"      # Or specify directly
```

### Host Options Reference

| Option | Type | Description |
|--------|------|-------------|
| `hostname` | string | **Required.** IP address or DNS hostname |
| `port` | integer | SSH port (default: from defaults or 22) |
| `username` | string | SSH username (default: from defaults or current user) |
| `private_key` | path | Path to SSH private key |
| `os` | string | `linux` (default) or `windows` |
| `shell` | string | `sh`, `powershell`, or `cmd` |
| `sudo_password` | string | Password for privilege escalation on Linux |
| `sudo_prompt` | boolean | Prompt for sudo password at runtime |
| `admin_password` | string | Password for privilege escalation on Windows |
| `admin_prompt` | boolean | Prompt for admin password at runtime |
| `proxmox` | object | Proxmox VM configuration (see Proxmox section) |

---

## Groups

Create named groups of hosts for easier targeting:

```yaml
groups:
  web_servers:
    - webserver1
    - webserver2
    - webserver3

  databases:
    - primary_db
    - replica_db

  all_servers:
    - webserver1
    - webserver2
    - primary_db
```

Groups can be used anywhere a host is referenced in task_groups.

---

## Tasks

Define reusable tasks that execute commands and transfer files:

```yaml
tasks:
  deploy_app:
    description: "Deploy the application"
    stop_on_error: true           # Stop on first error (default: true)
    become_root: false            # Run commands as root via sudo
    inactivity_timeout: 60        # Override default SSH timeout
    verbose: true                 # Stream output in real-time
    capture_output: true          # Display output when task completes

    # Commands execute in this order:
    # 1. proxmox_command (VM operations)
    # 2. local_command (run on local machine)
    # 3. upload (copy files to remote)
    # 4. remote_command (run on remote hosts)
    # 5. download (copy files from remote)
    # 6. delete_remote / delete_local (cleanup)

    local_command:
      - npm run build
      - tar -czf dist.tar.gz dist/

    upload:
      - "dist.tar.gz /tmp/dist.tar.gz"
      - "config/app.conf /etc/myapp/app.conf"

    remote_command:
      - tar -xzf /tmp/dist.tar.gz -C /opt/myapp
      - systemctl restart myapp

    download:
      - "/var/log/myapp/deploy.log ./logs/deploy.log"

    delete_remote:
      - /tmp/dist.tar.gz

    delete_local:
      - dist.tar.gz
```

### Task Options Reference

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `description` | string | "" | Human-readable description |
| `stop_on_error` | boolean | true | Stop execution on first command failure |
| `become_root` | boolean | false | Run remote commands as root via sudo |
| `inactivity_timeout` | integer | 15 | SSH inactivity timeout in seconds |
| `verbose` | boolean | false | Stream command output in real-time |
| `capture_output` | boolean | false | Display output when task completes |
| `proxmox_command` | list | [] | Proxmox VM operations |
| `local_command` | list/string | [] | Commands to run locally |
| `remote_command` | list/string | [] | Commands to run on remote hosts |
| `upload` | list | [] | Files to upload: "local remote" |
| `download` | list | [] | Files to download: "remote local" |
| `delete_remote` | list | [] | Files to delete on remote host |
| `delete_local` | list | [] | Files to delete locally |

### Multi-line Scripts

Commands can be a list of individual commands or a single multi-line script:

```yaml
tasks:
  # List of commands (each runs separately)
  list_style:
    remote_command:
      - echo "Step 1"
      - echo "Step 2"
      - echo "Step 3"

  # Single script (runs as one unit)
  script_style:
    remote_command: |
      #!/bin/bash
      set -e
      echo "Starting deployment"
      for i in 1 2 3; do
        echo "Step $i"
      done
      echo "Done"
```

### File Transfer Format

Upload and download use the format: `"source destination"`

```yaml
upload:
  - "local/file.txt /remote/path/file.txt"
  - "~/config.yaml /etc/app/config.yaml"

download:
  - "/var/log/app.log ./logs/app.log"
  - "/etc/app/config.yaml ./backup/config.yaml"
```

For Windows paths, use backslashes:

```yaml
upload:
  - "local.txt C:\\Users\\Admin\\file.txt"
download:
  - "C:\\logs\\app.log ./logs/app.log"
```

### Steps

By default, task operations execute in a fixed order: `proxmox_command` → `local_command` → `upload` → `remote_command` → `download` → `delete_remote` → `delete_local`, and each operation type can only appear once. Use `steps` to run operations in YAML declaration order with repeats allowed:

```yaml
tasks:
  deploy_and_verify:
    description: "Upload, verify, then clean up"
    steps:
      - remote_command:
          - mkdir -p /tmp/deploy
      - upload:
          - "dist.tar.gz /tmp/deploy/dist.tar.gz"
      - remote_command:
          - tar -xzf /tmp/deploy/dist.tar.gz -C /opt/myapp
          - systemctl restart myapp
      - remote_command:
          - curl -f http://localhost:8080/health
      - delete_remote:
          - /tmp/deploy/dist.tar.gz
```

Each step is a single-key mapping with one of the supported operation types:

| Step type | Description |
|-----------|-------------|
| `remote_command` | Commands to run on remote hosts |
| `local_command` | Commands to run on the local machine |
| `upload` | Files to upload: `"local remote"` |
| `download` | Files to download: `"remote local"` |
| `delete_remote` | Files to delete on remote host |
| `delete_local` | Files to delete locally |

**Important:**
- A task must use **either** `steps` **or** flat fields (`remote_command`, `upload`, etc.), never both. Mixing is rejected at config validation time.
- `proxmox_command` is not supported in steps (it runs before the SSH connection is established). Use it as a flat field in a separate task.
- When using steps, `local_command` runs per-host (within the host loop), unlike flat `local_command` which runs once before any host connections.
- All task options (`stop_on_error`, `become_root`, `verbose`, etc.) still apply to steps tasks.
- Template variables (`{{ vars }}`, `{{ host.* }}`, `{{ builtin.* }}`) work inside steps.

---

## Task Groups

Task groups define what tasks run on which hosts. Groups execute in parallel with each other, while tasks within a group execute sequentially.

```yaml
task_groups:
  deploy_web:
    - name: "Deploy Web Servers"     # Display name (optional)
    - task: stop_service
      hosts: web_servers             # Use a group name
    - task: deploy_app
      hosts: web_servers
    - task: start_service
      hosts: web_servers

  deploy_db:
    - name: "Deploy Database"
    - task: backup_db
      hosts: primary_db              # Use a single host
    - task: migrate_db
      hosts: [primary_db, replica_db] # Use a list of hosts

  local_only:
    - name: "Local Build"
    - task: build_artifacts          # No hosts = local-only task
```

### Dependencies

Control execution order between task groups:

```yaml
task_groups:
  build:
    - name: "Build Phase"
    - task: compile
    - task: test

  deploy:
    - name: "Deploy Phase"
    - depends:                       # Wait for these groups to complete
        - build
    - task: deploy_app
      hosts: all_servers

  verify:
    - name: "Verification"
    - depends:
        - deploy
    - task: health_check
      hosts: all_servers
```

Circular dependencies are detected and reported as errors.

---

## Campaigns

Campaigns define named subsets of task_groups to run. Useful for different deployment scenarios:

```yaml
campaigns:
  # First campaign is auto-selected when no --campaign specified
  full_deploy:
    - build
    - deploy_web
    - deploy_db
    - verify

  quick_deploy: [deploy_web]     # Only deploy web servers

  rollback:
    - restore_backup
    - restart_services
```

Campaign items can reference task_groups or other campaigns:

```yaml
campaigns:
  core:
    - build
    - deploy_web

  extended:
    - core                       # Include all items from 'core' campaign
    - deploy_db
    - monitoring
```

Run a specific campaign:

```bash
rot config.yaml --campaign quick_deploy
# Or use short form:
rot config.yaml -c quick_deploy
```

If multiple campaigns exist, the first one (by definition order) is auto-selected if no `--campaign` is specified.

### Campaign Dependency Handling

By default, if a task_group in a campaign depends on another task_group not explicitly listed in the campaign, the dependency is automatically included:

```yaml
campaigns:
  deploy: [web_deploy]  # web_deploy depends on build

task_groups:
  build:
    - task: compile
  web_deploy:
    - depends: [build]  # build is auto-included in 'deploy' campaign
    - task: deploy
      hosts: web
```

Use `--lenient-campaign` (or `-l`) to disable auto-inclusion and only warn about missing dependencies:

```bash
rot config.yaml -c deploy -l  # Warns about 'build' but doesn't include it
```

---

## Runtime Template Variables

Certain template variables are only available at execution time (in `remote_command`, `local_command`, `upload`, `download`):

### Host Context (`host.*`)

Information about the current host:

```yaml
tasks:
  show_host_info:
    remote_command:
      - echo "Host name: {{ host.name }}"
      - echo "Hostname: {{ host.hostname }}"
      - echo "Username: {{ host.username }}"
      - echo "Port: {{ host.port }}"
```

| Variable | Description |
|----------|-------------|
| `host.name` | The key name from the hosts config |
| `host.hostname` | The actual hostname/IP |
| `host.username` | SSH username |
| `host.port` | SSH port |

### Builtin Context (`builtin.*`)

Execution context variables:

```yaml
tasks:
  log_execution:
    remote_command:
      - echo "Running on {{ builtin.host }}"
      - echo "Task group: {{ builtin.task_group }}"
      - echo "Timestamp: {{ builtin.timestamp }}"

  create_log:
    download:
      - "/var/log/app.log ./logs/{{ builtin.host }}-{{ builtin.timestamp }}.log"
```

| Variable | Description |
|----------|-------------|
| `builtin.host` | Current host name (same as `host.name`) |
| `builtin.task_group` | Name of the current task group |
| `builtin.timestamp` | Execution timestamp (YYYY-MM-DD_HH-MM-SS) |

---

## Proxmox Integration

Control Proxmox VE virtual machines directly from rot:

### API Configuration

```yaml
proxmox:
  url: https://proxmox.example.com:8006
  token_id: user@pam!token-name
  token_secret: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
  verify_ssl: true          # Verify SSL certificates (default: true)
  timeout: 30               # API request timeout in seconds (default: 30)
```

### Host VM Configuration

```yaml
hosts:
  test_vm:
    hostname: 192.168.1.100
    proxmox:
      vmid: 100              # Proxmox VM ID
      node: pve-node1        # Proxmox node name
      snapshot: clean-state  # Default snapshot for rollback
```

### Proxmox Commands

```yaml
tasks:
  vm_rollback:
    description: "Rollback VM to clean state"
    proxmox_command:
      - rollback              # Uses host's default snapshot
      - start                 # Start the VM

  vm_snapshot:
    description: "Create a snapshot"
    proxmox_command:
      - shutdown              # Graceful shutdown
      - snapshot post-deploy  # Create named snapshot

  vm_cleanup:
    proxmox_command:
      - delete old-snapshot   # Delete a snapshot
      - stop                  # Force stop (immediate power off)
```

Available Proxmox commands:

| Command | Description |
|---------|-------------|
| `start` | Start the VM |
| `stop` | Force stop (immediate power off) |
| `shutdown` | Graceful shutdown via ACPI |
| `rollback` | Rollback to host's default snapshot |
| `rollback <name>` | Rollback to named snapshot |
| `snapshot <name>` | Create a snapshot |
| `delete <name>` | Delete a snapshot |

---

## Windows Support

rot supports Windows hosts with PowerShell or CMD:

```yaml
hosts:
  windows_server:
    hostname: win.example.com
    os: windows
    shell: powershell        # or cmd
    username: Administrator
    admin_prompt: true       # Prompt for admin password

tasks:
  windows_deploy:
    description: "Deploy to Windows"
    upload:
      - "app.zip C:\\deploy\\app.zip"
    remote_command:
      - Expand-Archive -Path C:\deploy\app.zip -DestinationPath C:\app -Force
      - Restart-Service MyApp
    download:
      - "C:\\logs\\deploy.log ./logs/deploy.log"

task_groups:
  deploy_windows:
    - name: "Windows Deployment"
    - task: windows_deploy
      hosts: windows_server
```

---

## Complete Example

Here's a complete configuration demonstrating all features:

```yaml
# config/main.yaml
imports:
  - hosts.yaml
  - tasks.yaml

vars:
  app_name: myapp
  deploy_dir: /opt/{{ app_name }}
  log_dir: /var/log/{{ app_name }}

defaults:
  port: 22
  username: deploy
  private_key: ~/.ssh/deploy_key
  inactivity_timeout: 30

task_groups:
  preparation:
    - name: "Preparation"
    - task: build_locally

  deployment:
    - name: "Deploy Application"
    - depends:
        - preparation
    - task: stop_service
      hosts: app_servers
    - task: deploy
      hosts: app_servers
    - task: start_service
      hosts: app_servers

  verification:
    - name: "Verify Deployment"
    - depends:
        - deployment
    - task: health_check
      hosts: app_servers
    - task: collect_logs
      hosts: app_servers

campaigns:
  # First campaign is auto-selected
  full:
    - preparation
    - deployment
    - verification

  quick: [deployment]
```

```yaml
# config/hosts.yaml
hosts:
  web1:
    hostname: 192.168.1.10
    sudo_password: "{{ env('SUDO_PASS') }}"

  web2:
    hostname: 192.168.1.11
    sudo_password: "{{ env('SUDO_PASS') }}"

groups:
  app_servers:
    - web1
    - web2
```

```yaml
# config/tasks.yaml
tasks:
  build_locally:
    description: "Build application artifacts"
    local_command:
      - npm ci
      - npm run build
      - tar -czf dist.tar.gz dist/

  stop_service:
    description: "Stop the application service"
    become_root: true
    remote_command:
      - systemctl stop {{ app_name }}

  deploy:
    description: "Deploy application files"
    upload:
      - "dist.tar.gz /tmp/dist.tar.gz"
    remote_command:
      - tar -xzf /tmp/dist.tar.gz -C {{ deploy_dir }}
      - chown -R {{ app_name }}:{{ app_name }} {{ deploy_dir }}
    delete_remote:
      - /tmp/dist.tar.gz

  start_service:
    description: "Start the application service"
    become_root: true
    remote_command:
      - systemctl start {{ app_name }}

  health_check:
    description: "Verify application is running"
    remote_command:
      - curl -f http://localhost:8080/health

  collect_logs:
    description: "Download deployment logs"
    download:
      - "{{ log_dir }}/deploy.log ./logs/{{ builtin.host }}-{{ builtin.timestamp }}.log"
```

Run the full deployment:

```bash
rot config/main.yaml -v
```

Run just the deployment (skip build and verification):

```bash
rot config/main.yaml -c quick
```
