# Creating Your First rot Script

This tutorial walks you through creating rot automation scripts, starting simple and building up to more advanced features.

## Prerequisites

- rot installed and available in your PATH
- For remote execution (Stage 2+): SSH access to at least one remote host

---

## Stage 1: Local Commands

Let's start with the simplest possible rot script: running commands on your local machine.

### Your First Script

Create a file called `hello.yaml`:

```yaml
tasks:
  hello:
    local_command:
      - echo "Hello from rot!"
      - date
      - whoami

task_groups:
  main:
    - task: hello
```

### Running the Script

Validate the configuration first:

```bash
rot check hello.yaml
```

If validation passes, run it:

```bash
rot run hello.yaml
```

You should see output from the three commands.

### Understanding the Structure

Every rot script needs at minimum:

1. **tasks** - Define what commands to run
2. **task_groups** - Define when/where to run them

The `local_command` field runs commands on your local machine using your default shell.

### Adding More Tasks

Let's create a more practical example. Create `build.yaml`:

```yaml
vars:
  project_name: myproject
  output_dir: ./dist

tasks:
  clean:
    description: "Remove old build artifacts"
    local_command:
      - rm -rf {{ output_dir }}
      - mkdir -p {{ output_dir }}

  build:
    description: "Build the project"
    local_command:
      - echo "Building {{ project_name }}..."
      - echo "Build complete" > {{ output_dir }}/build.log

  verify:
    description: "Verify build output"
    local_command:
      - ls -la {{ output_dir }}
      - cat {{ output_dir }}/build.log

task_groups:
  build_pipeline:
    - name: "Build Pipeline"
    - task: clean
    - task: build
    - task: verify
```

Run it:

```bash
rot run build.yaml
```

Tasks in a task_group execute sequentially, so `clean` runs first, then `build`, then `verify`.

### Multi-line Scripts

For complex logic, use a multi-line script instead of a list:

```yaml
tasks:
  complex_build:
    local_command: |
      #!/bin/bash
      set -e

      echo "Starting build..."

      for dir in src lib tests; do
        if [ -d "$dir" ]; then
          echo "Processing $dir"
        fi
      done

      echo "Build complete!"
```

### Verbose Output

To see command output in real-time, use the `-v` flag:

```bash
rot run build.yaml -v
```

Or set `verbose: true` on individual tasks:

```yaml
tasks:
  chatty_task:
    verbose: true
    local_command:
      - echo "You'll see this immediately"
```

---

## Stage 2: Remote Commands

Now let's connect to remote hosts and run commands over SSH.

### Setting Up Hosts

Create `remote.yaml`:

```yaml
defaults:
  username: your_username
  private_key: ~/.ssh/id_ed25519

hosts:
  server1:
    hostname: 192.168.1.10

tasks:
  check_server:
    remote_command:
      - hostname
      - uptime
      - df -h /

task_groups:
  check:
    - name: "Server Health Check"
    - task: check_server
      hosts: server1
```

Replace `your_username` with your SSH username and `192.168.1.10` with your server's IP or hostname.

Run it:

```bash
rot run remote.yaml -v
```

### Understanding Remote Execution

The key differences from local execution:

1. **hosts** - Define remote machines with connection details
2. **defaults** - Set default SSH settings (username, key, port)
3. **hosts: server1** in task_groups - Specifies which host runs the task

### Multiple Hosts

Add more hosts and run tasks across all of them:

```yaml
defaults:
  username: deploy
  private_key: ~/.ssh/id_ed25519

hosts:
  web1:
    hostname: 192.168.1.10

  web2:
    hostname: 192.168.1.11

  db1:
    hostname: 192.168.1.20
    username: dbadmin    # Override default username
    private_key: ~       # No key - rot will prompt for SSH password

groups:
  web_servers:
    - web1
    - web2

  all_servers:
    - web1
    - web2
    - db1

tasks:
  check_disk:
    remote_command:
      - df -h /
      - df -h /var

task_groups:
  check_web:
    - name: "Check Web Servers"
    - task: check_disk
      hosts: web_servers    # Use the group

  check_all:
    - name: "Check All Servers"
    - task: check_disk
      hosts: all_servers
```

### Running with Sudo

For commands that need root privileges:

```yaml
hosts:
  server1:
    hostname: 192.168.1.10
    sudo_prompt: true      # rot will prompt for password automatically

tasks:
  restart_nginx:
    become_root: true
    remote_command:
      - systemctl restart nginx
      - systemctl status nginx

task_groups:
  restart:
    - task: restart_nginx
      hosts: server1
```

With `sudo_prompt: true`, rot automatically prompts for the sudo password when running `become_root` tasks on that host.

Alternatively, use the `--root` flag for a global sudo password that applies to all hosts:

```bash
rot run config.yaml --root
```

### Using Host Variables

Access host information in your commands:

```yaml
tasks:
  identify:
    remote_command:
      - echo "Running on host: {{ host.name }}"
      - echo "Hostname: {{ host.hostname }}"
      - echo "Connected as: {{ host.username }}"
```

---

## Stage 3: File Transfers

rot can upload files to remote hosts and download files back.

### Uploading Files

Upload format: `"local_path remote_path"`

```yaml
defaults:
  username: deploy
  private_key: ~/.ssh/id_ed25519

hosts:
  server1:
    hostname: 192.168.1.10

tasks:
  deploy_config:
    upload:
      - "config/app.conf /etc/myapp/app.conf"
      - "scripts/startup.sh /opt/myapp/startup.sh"
    remote_command:
      - chmod +x /opt/myapp/startup.sh
      - cat /etc/myapp/app.conf

task_groups:
  deploy:
    - task: deploy_config
      hosts: server1
```

### Downloading Files

Download format: `"remote_path local_path"`

```yaml
tasks:
  fetch_logs:
    download:
      - "/var/log/myapp/app.log ./logs/app.log"
      - "/var/log/myapp/error.log ./logs/error.log"

task_groups:
  get_logs:
    - task: fetch_logs
      hosts: server1
```

### Build, Upload, Execute Pattern

A common pattern combines local builds with remote deployment:

```yaml
vars:
  app_name: myapp
  deploy_dir: /opt/{{ app_name }}

defaults:
  username: deploy
  private_key: ~/.ssh/id_ed25519

hosts:
  production:
    hostname: prod.example.com
    sudo_prompt: true

tasks:
  build:
    description: "Build application locally"
    local_command:
      - npm ci
      - npm run build
      - tar -czf dist.tar.gz dist/

  deploy:
    description: "Deploy to server"
    upload:
      - "dist.tar.gz /tmp/dist.tar.gz"
    remote_command:
      - tar -xzf /tmp/dist.tar.gz -C {{ deploy_dir }}
      - ls -la {{ deploy_dir }}
    delete_remote:
      - /tmp/dist.tar.gz
    delete_local:
      - dist.tar.gz

  restart:
    description: "Restart the service"
    become_root: true
    remote_command:
      - systemctl restart {{ app_name }}
      - systemctl status {{ app_name }}

task_groups:
  full_deploy:
    - name: "Full Deployment"
    - task: build
    - task: deploy
      hosts: production
    - task: restart
      hosts: production
```

### Using Templates in File Paths

Use variables and host info in file paths:

```yaml
vars:
  log_dir: ./logs

tasks:
  fetch_logs:
    download:
      - "/var/log/app.log {{ log_dir }}/{{ host.name }}-{{ builtin.timestamp }}.log"

task_groups:
  collect:
    - task: fetch_logs
      hosts: all_servers
```

This creates uniquely named log files for each host with timestamps.

### Execution Order

Within a task, operations execute in this order:

1. `proxmox_command` - VM operations (if configured)
2. `local_command` - Commands on your machine
3. `upload` - Copy files to remote
4. `remote_command` - Commands on remote host
5. `download` - Copy files from remote
6. `delete_remote` - Clean up remote files
7. `delete_local` - Clean up local files

This order ensures you can:
- Build locally before uploading
- Upload before running remote commands
- Run remote commands before downloading results
- Clean up after everything completes

If you need a different order, split into multiple tasks within a task_group.

---

## Next Steps

Now that you understand the basics, explore these advanced features:

- **Imports**: Split config across multiple files (`imports: [hosts.yaml, tasks.yaml]`)
- **Dependencies**: Control task_group execution order (`depends: [build]`)
- **Campaigns**: Define named subsets of task_groups for different scenarios
- **Proxmox Integration**: Manage VMs alongside deployments
- **Windows Support**: Deploy to Windows hosts with PowerShell

See the full [Configuration Guide](CONFIGURATION.md) for complete documentation.

---

## Quick Reference

### Minimal Local Script

```yaml
tasks:
  mytask:
    local_command:
      - echo "Hello"

task_groups:
  main:
    - task: mytask
```

### Minimal Remote Script

```yaml
hosts:
  myserver:
    hostname: 192.168.1.10

tasks:
  mytask:
    remote_command:
      - echo "Hello from remote"

task_groups:
  main:
    - task: mytask
      hosts: myserver
```

### Common Commands

```bash
rot check config.yaml              # Validate config
rot run config.yaml                # Run tasks
rot run config.yaml -v             # Run with verbose output
rot run config.yaml --root         # Prompt for global sudo password
rot run config.yaml -c deploy      # Run specific campaign
rot run config.yaml -o std         # Capture and display output after completion
rot run config.yaml -o output.txt  # Capture output to file
rot list config.yaml hosts         # List configured hosts
rot list config.yaml tasks         # List configured tasks
rot list config.yaml campaigns     # List configured campaigns
```
