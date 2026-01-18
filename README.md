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
