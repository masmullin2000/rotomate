---
name: rot-config
description: Help create rot SSH automation configuration files. Use when users want to create, generate, or set up rot/rotomate YAML configs for SSH automation tasks.
argument-hint: [description of what to automate]
allowed-tools: Read, Write, Edit, Glob, Grep
---

# rot Configuration Generator

Help the user create rot SSH automation configuration files based on their requirements.

## Reference Documentation

See the quick reference at [reference.md](reference.md) for all configuration options.
For complete documentation, see: https://DOCS_URL_PLACEHOLDER/CONFIGURATION.md

## Your Task

The user wants to create a rot configuration. Based on their description in `$ARGUMENTS`, help them by:

1. **Understand the requirements**
   - What hosts/servers do they need to manage?
   - What tasks do they want to automate?
   - Do they need file transfers (upload/download)?
   - Do they need privilege escalation (sudo/admin)?
   - Are there dependencies between tasks?

2. **Ask clarifying questions** if the request is vague:
   - Target hosts (hostnames, IPs, SSH ports)
   - Authentication method (SSH key path, password prompt)
   - Operating systems (Linux/Windows)
   - Commands to run
   - Files to transfer

3. **Generate appropriate configuration** following these patterns:

### For simple single-file configs:
```yaml
defaults:
  username: user
  private_key: ~/.ssh/id_ed25519

hosts:
  server1:
    hostname: 192.168.1.10

tasks:
  my_task:
    description: "What this task does"
    remote_command:
      - echo "hello"

task_groups:
  main:
    - name: "Task Group Name"
    - task: my_task
      hosts: server1
```

### For complex multi-file configs:
Suggest splitting into:
- `hosts.yaml` - Host definitions and groups
- `tasks.yaml` - Task definitions
- `main.yaml` - Imports, task_groups, campaigns

## Configuration Best Practices

1. **Use defaults** for common connection settings to reduce repetition
2. **Use groups** to organize hosts logically (web_servers, databases, etc.)
3. **Use variables** for values that change between environments
4. **Use campaigns** when you need different execution scenarios
5. **Set `stop_on_error: true`** (default) for critical tasks
6. **Use `become_root: true`** only when necessary
7. **Use descriptive names** for tasks and task_groups
8. **Add descriptions** to document what tasks do

## Template Variables

Remind users they can use:
- `{{ var_name }}` - Custom variables from `vars:` section
- `{{ env('VAR') }}` - Environment variables
- `{{ host.name }}`, `{{ host.hostname }}` - Host context (in commands)
- `{{ builtin.timestamp }}` - Execution timestamp (in commands)

## File Transfer Format

Always use the format: `"source destination"` with quotes:
```yaml
upload:
  - "local/file.txt /remote/path/file.txt"
download:
  - "/remote/file.txt local/file.txt"
```

## Windows Hosts

For Windows, remind users to set:
```yaml
hosts:
  windows_host:
    hostname: win.example.com
    os: windows
    shell: powershell  # or cmd
```

## Proxmox Integration

If managing VMs, suggest proxmox configuration:
```yaml
proxmox:
  url: https://proxmox:8006
  token_id: user@pam!token
  token_secret: xxx

hosts:
  vm1:
    hostname: 192.168.1.10
    proxmox:
      vmid: 100
      node: pve-node
      snapshot: clean-state
```

## Output

Write the generated configuration to the appropriate file(s) in the user's project. Suggest a `config/` directory if one doesn't exist.

After generating, remind users how to validate and run:
```bash
rot check -c config/main.yaml    # Validate
rot run -c config/main.yaml -v   # Run with verbose output
```
