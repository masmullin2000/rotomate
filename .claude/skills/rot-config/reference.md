# rot Configuration Quick Reference

## Configuration Structure

```yaml
imports: []           # Other config files to include
vars: {}              # Template variables
proxmox: {}           # Proxmox VE API config (optional)
defaults: {}          # Default connection settings
hosts: {}             # Host definitions
groups: {}            # Named groups of hosts
tasks: {}             # Task definitions
task_groups: {}       # Execution groups
campaigns: {}         # Named subsets of task_groups
```

## Defaults

```yaml
defaults:
  port: 22
  username: deploy
  private_key: ~/.ssh/id_ed25519
  inactivity_timeout: 15
  os: linux              # linux | windows
  shell: sh              # sh | powershell | cmd
```

## Hosts

```yaml
hosts:
  server_name:
    hostname: 192.168.1.10      # Required
    port: 22
    username: admin
    private_key: ~/.ssh/key
    os: linux                   # linux | windows
    shell: sh                   # sh | powershell | cmd
    sudo_password: "secret"     # For become_root tasks
    sudo_prompt: true           # Prompt at runtime instead
    admin_password: "secret"    # Windows privilege escalation
    admin_prompt: true          # Prompt at runtime instead
    proxmox:                    # Optional VM config
      vmid: 100
      node: pve-node
      snapshot: clean-state
```

## Groups

```yaml
groups:
  web_servers:
    - web1
    - web2
  all:
    - web1
    - web2
    - db1
```

## Tasks

```yaml
tasks:
  task_name:
    description: "What this does"
    stop_on_error: true       # Default: true
    become_root: false        # Run as root via sudo
    inactivity_timeout: 60    # SSH timeout override
    verbose: true             # Stream output
    capture_output: true      # Show output when done

    # Execution order: proxmox -> local -> upload -> remote -> download -> delete
    proxmox_command:
      - rollback
      - start
    local_command:
      - npm run build
    upload:
      - "local/file /remote/file"
    remote_command:
      - echo "hello"
    download:
      - "/remote/file local/file"
    delete_remote:
      - /tmp/cleanup
    delete_local:
      - ./temp
```

### Commands as Script

```yaml
tasks:
  scripted:
    remote_command: |
      #!/bin/bash
      set -e
      echo "Multi-line script"
      for i in 1 2 3; do
        echo $i
      done
```

## Task Groups

```yaml
task_groups:
  group_key:
    - name: "Display Name"
    - depends:                  # Optional dependencies
        - other_group
    - task: task_name
      hosts: group_name         # Group, single host, or list
    - task: another_task
      hosts: [host1, host2]
    - task: local_only_task     # No hosts = local only
```

## Campaigns

```yaml
campaigns:
  # First campaign is auto-selected if no --campaign specified
  full:
    - build
    - deploy_only
    - verify
  quick: [deploy_only]
```

## Variables

```yaml
vars:
  app_name: myapp
  deploy_dir: /opt/{{ app_name }}
  api_key: "{{ env('API_KEY') }}"
  port: "{{ env_or('PORT', '8080') }}"
```

## Runtime Variables (in commands only)

| Variable | Description |
|----------|-------------|
| `{{ host.name }}` | Host key name |
| `{{ host.hostname }}` | Actual hostname/IP |
| `{{ host.username }}` | SSH username |
| `{{ host.port }}` | SSH port |
| `{{ builtin.host }}` | Same as host.name |
| `{{ builtin.task_group }}` | Current task group name |
| `{{ builtin.timestamp }}` | YYYY-MM-DD_HH-MM-SS |

## Proxmox Commands

| Command | Description |
|---------|-------------|
| `start` | Start VM |
| `stop` | Force stop |
| `shutdown` | Graceful shutdown |
| `rollback` | Rollback to default snapshot |
| `rollback <name>` | Rollback to named snapshot |
| `snapshot <name>` | Create snapshot |
| `delete <name>` | Delete snapshot |

## CLI Commands

```bash
rot check -c config.yaml              # Validate config
rot run -c config.yaml                # Run tasks
rot run -c config.yaml -v             # Verbose output
rot run -c config.yaml --root         # Prompt for sudo
rot run -c config.yaml --campaign X   # Run specific campaign
rot list -c config.yaml hosts         # List hosts
rot list -c config.yaml tasks         # List tasks
rot list -c config.yaml groups        # List task groups
rot list -c config.yaml campaigns     # List campaigns
rot list -c config.yaml all           # List everything
```
