# coop

Sandboxed AI agent sessions using Linux namespaces. No Docker, no Podman, no runtime dependencies. Just syscalls.

```
coop                        # create box + attach to agent
coop shell                  # open a shell inside the box
coop logs -f                # follow agent output
coop restart                # restart the agent process
```

## What it does

Coop runs AI coding agents (Claude Code, Codex, Aider, etc.) in an isolated sandbox with its own filesystem, hostname, and (optionally) network namespace. Sessions survive terminal disconnects and can be accessed from multiple terminals, a web UI, or a phone.

**One binary. No containers. Sub-100ms startup.**

## Quick start

```bash
# Build from source
cargo build --release

# Initialize a project
cd ~/my-project
coop init                   # creates coop.toml
coop build                  # pulls base image + builds rootfs

# Start working
coop                        # creates a box, attaches to the agent
```

Press `Ctrl+]` to detach. The agent keeps running.

```bash
coop shell                  # open a shell inside the same box
coop attach                 # reattach to the agent
coop kill                   # tear it all down
```

## coop.toml

```toml
[sandbox]
image = "debian:latest"
agent = "claude"
shell = "bash"
user = "coop"

setup = [
  "apt-get update && apt-get install -y git curl",
]

mounts = [
  "~/.bashrc:~/.bashrc",            # bind mount from host
  "claude-config:~/.claude",        # named volume (persistent)
]

[network]
mode = "host"               # "host", "none", or "veth"

[session]
auto_restart = true          # restart agent on exit
restart_delay_ms = 1000
```

Config layers: defaults < `~/.config/coop/default.toml` < project `coop.toml` < CLI flags.

## Commands

### Core workflow

| Command | Description |
|---------|-------------|
| `coop` | Create box (if needed) and attach to the agent |
| `coop -d` | Create box detached |
| `coop attach` | Reattach to the agent (PTY 0) |
| `coop kill` | Kill the box |
| `coop ls` | List running boxes |

### Shell management

| Command | Description |
|---------|-------------|
| `coop shell` | Open a shell inside the box |
| `coop shell --new` | Force a new shell (don't reuse existing) |
| `coop shell -c zsh` | Open a specific shell command |
| `coop shell ls` | List shell sessions |
| `coop shell kill <id>` | Kill a shell session |
| `coop shell attach <id>` | Attach to a shell by ID |
| `coop shell logs <id>` | View shell scrollback |
| `coop shell restart <id>` | Restart a shell process |

### Agent logs & restart

| Command | Description |
|---------|-------------|
| `coop logs` | Print agent scrollback |
| `coop logs -f` | Follow agent output live |
| `coop logs -n 50` | Last 50 lines |
| `coop restart` | Restart the agent process |

### Build & init

| Command | Description |
|---------|-------------|
| `coop init` | Create a default `coop.toml` |
| `coop build` | Build rootfs from config |
| `coop build --no-cache` | Rebuild from scratch |

### System management

| Command | Description |
|---------|-------------|
| `coop system status` | Daemon status |
| `coop system logs` | Daemon log |
| `coop system shutdown` | Stop the daemon |
| `coop system volumes` | List named volumes |
| `coop system df` | Disk usage |
| `coop system clean` | Remove rootfs |
| `coop system prune` | Remove everything |

### Web UI & remote access

| Command | Description |
|---------|-------------|
| `coop serve` | Start web UI on localhost:8888 |
| `coop tunnel` | P2P WebRTC tunnel (coming soon) |

## How it works

1. **Sandbox**: `fork()` + `unshare(CLONE_NEWUSER | CLONE_NEWNS | CLONE_NEWUTS)` creates isolated namespaces. OverlayFS layers a writable filesystem on top of a shared base rootfs. `pivot_root()` makes the sandbox the new root.

2. **Daemon**: An invisible background daemon (auto-spawned, auto-shutdown) manages sessions over a Unix socket. The CLI is a thin client.

3. **PTYs**: Each box has PTY 0 (the agent) plus any number of shell PTYs. All PTY output is broadcast to connected clients and buffered in a scrollback ring (256KB). Detach and reattach without losing output.

4. **Mounts**: Path-based entries (`~/.bashrc:~/.bashrc`) bind-mount from the host. Named entries (`claude-config:~/.claude`) use managed persistent storage that survives box restarts.

## Requirements

- Linux kernel 5.11+ (user namespaces, overlayfs)
- `/etc/subuid` and `/etc/subgid` configured for your user (for full UID range)
- Rust 1.75+ to build

Works on native Linux and WSL2.

## Architecture

```
coop (CLI client)
  |
  | Unix socket (auto-spawn daemon if not running)
  v
Daemon (background, manages all sessions)
  |
  |-- Session "my-project"
  |     |-- PTY 0 (agent: claude)     <- auto-restarts on exit
  |     |-- PTY 1 (shell: bash)       <- cleaned up on exit
  |     `-- PTY 2 (shell: bash)
  |
  `-- Session "other-project"
        `-- PTY 0 (agent: claude)
```

See [`docs/`](./docs/) for full documentation.

## License

MIT
