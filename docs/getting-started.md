# Getting Started

## Installation

Build from source (requires Rust 1.75+):

```bash
git clone https://github.com/user/coop
cd coop
cargo build --release
cp target/release/coop ~/.local/bin/
```

## Prerequisites

**Linux kernel 5.11+** with user namespace support. Works on native Linux and WSL2.

Check that your user has subordinate UID/GID ranges configured:

```bash
grep $USER /etc/subuid
grep $USER /etc/subgid
```

If those files are empty or missing, add entries:

```bash
sudo usermod --add-subuids 100000-165535 --add-subgids 100000-165535 $USER
```

This enables the full UID range inside the sandbox (needed for `apt-get` and other tools that create system users).

## First run

```bash
cd ~/my-project

# Create a coop.toml
coop init

# Edit it to set your agent command, base image, etc.
$EDITOR coop.toml

# Build the rootfs (pulls OCI image, runs setup commands)
coop build

# Launch the agent
coop
```

On first run, `coop` will:
1. Pull the base image (e.g. `debian:latest`) from a registry
2. Unpack it into `~/.coop/rootfs/base/`
3. Run your `setup` commands inside the rootfs
4. Create the sandbox and launch the agent

Subsequent runs skip all of this and start in under 100ms.

## Basic workflow

```bash
# Start or reattach to the agent
coop

# Detach with Ctrl+]  (agent keeps running)

# Open a shell in the same sandbox
coop shell

# Exit shell with Ctrl+D (returns to host)

# Check what's running
coop ls
coop shell ls

# View agent output history
coop logs
coop logs -f          # follow live

# Restart the agent
coop restart

# Kill everything
coop kill
```

## Multiple projects

Each project directory gets its own box. The box name defaults to the directory basename:

```bash
cd ~/project-a && coop    # creates box "project-a"
cd ~/project-b && coop    # creates box "project-b"
coop ls                   # shows both
```

You can also name boxes explicitly:

```bash
coop -n my-box -w ~/project-a
```

## Global defaults

Create `~/.config/coop/default.toml` to set defaults that apply to all projects:

```toml
[sandbox]
agent = "claude"
shell = "bash"

mounts = [
  "claude-config:~/.claude",
]
```

Project-level `coop.toml` overrides global defaults. The merge order is:

1. Built-in defaults
2. `~/.config/coop/default.toml`
3. Project `coop.toml`
4. CLI flags

## What's next

- [Configuration Reference](./configuration.md) -- full coop.toml spec
- [CLI Reference](./cli.md) -- all commands
- [Architecture](./architecture.md) -- how it works under the hood
