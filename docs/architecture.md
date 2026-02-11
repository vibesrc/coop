# Architecture

## Overview

Coop has three layers:

```
┌─────────────────────────────────────────────┐
│  CLI Client (coop binary, thin client)      │
│  - Parses commands, connects to daemon      │
│  - Bridges local terminal to remote PTY     │
└──────────────────┬──────────────────────────┘
                   │ Unix socket (IPC)
┌──────────────────▼──────────────────────────┐
│  Daemon (background, auto-spawn/shutdown)   │
│  - Session lifecycle management             │
│  - PTY multiplexing + scrollback            │
│  - Web UI server                            │
│  - Client connection handling               │
└──────────────────┬──────────────────────────┘
                   │ fork + namespaces
┌──────────────────▼──────────────────────────┐
│  Sandbox (Linux namespaces + overlayfs)     │
│  - User namespace (root-inside mapping)     │
│  - Mount namespace (overlayfs + binds)      │
│  - UTS namespace (custom hostname)          │
│  - Network namespace (optional)             │
└─────────────────────────────────────────────┘
```

## Sandbox creation

When a box is created:

1. **fork()** -- parent becomes daemon, child becomes namespace init
2. **unshare(CLONE_NEWUSER | CLONE_NEWNS | CLONE_NEWUTS)** -- child creates new namespaces
3. Parent writes UID/GID mappings via `/proc/<pid>/uid_map`
4. Child mounts overlayfs: `lowerdir=base_rootfs, upperdir=session_upper, workdir=session_work`
5. Child bind-mounts workspace, persist dirs, and user-configured mounts
6. Child calls **pivot_root()** to make the overlay the new root
7. Child signals parent that filesystem is ready (parent waits before returning)
8. Child **exec()s** the agent command inside the sandbox

The parent only returns after step 7, so subsequent operations (like `nsenter_shell`) always see a fully set up namespace.

## Entering an existing namespace

Namespace file descriptors are pinned at session creation time (opened from `/proc/<pid>/ns/*` and `/proc/<pid>/root`). This keeps the namespace alive even after the init process exits, which is critical for restart support.

When `coop shell` or `coop restart` spawns a new process in an existing box:

1. **fork()** (using pre-opened namespace fds, not `/proc/<pid>/`)
2. Child: **setns()** into each namespace (user first, then mount, uts, net)
3. Child: **fchdir()** to the pinned root fd, then **chroot(".")**
4. Child: **exec()** the shell command

Both agent and shell processes use the same PTY infrastructure. The agent is just PTY 0 with `auto_restart=true`.

## Daemon lifecycle

The daemon is invisible to the user:

- **Auto-spawn**: First `coop` invocation starts the daemon as a background process
- **Auto-shutdown**: After 30 seconds with zero sessions, the daemon exits
- **Socket**: `~/.coop/daemon.sock` (permissions 0600)
- **PID file**: `~/.coop/daemon.pid`
- **Logs**: `~/.coop/daemon.log`

The daemon uses an async Tokio runtime. Each client connection is a spawned task.

## IPC protocol

Client-daemon communication uses a length-prefixed JSON protocol over Unix sockets:

```
[4 bytes: length (big-endian u32)] [JSON payload]
```

### Command phase

Client sends `Command` messages, daemon replies with `Response` messages.

### Stream phase

After `Attach` or `Shell` succeeds, the connection upgrades to stream mode using tagged frames:

```
[1 byte: frame type] [4 bytes: length] [payload]
```

Frame types:
- `0x00` -- PTY data (terminal I/O)
- `0x01` -- Control (JSON: Resize, Detach, etc.)

## PTY architecture

Each PTY has:
- A **master fd** (held by the daemon)
- A **broadcast channel** (fan-out to all connected clients)
- A **scrollback buffer** (256KB ring, replayed on reattach)
- An **exit watcher** (background task that detects process exit)

```
PTY master fd
     │
     ▼
spawn_pty_reader (tokio task)
     │
     ├──▶ broadcast::Sender ──▶ Client A (stream mode)
     │                     ──▶ Client B (stream mode)
     │                     ──▶ Web UI client
     │
     └──▶ scrollback buffer (Arc<Mutex<Vec<u8>>>)
```

When a PTY process exits:
- The reader task detects EOF and fires a oneshot channel
- The exit watcher task receives the signal
- If `auto_restart=true`: sends a restart message via broadcast, waits, then restarts
- If `auto_restart=false`: cleans up the PTY (removes from session)
- The broadcast channel closes when all senders are dropped
- Connected clients receive a `PtyExited` event and disconnect

## Security model

The sandbox gives the agent a full development environment that looks and feels like root, while preventing it from damaging the host.

### What's isolated

| Layer | Protection |
|-------|------------|
| **User namespace** | uid 0 inside maps to your unprivileged uid on the host. No real root. |
| **Mount namespace** | OverlayFS absorbs all rootfs writes. `rm -rf /` is harmless. |
| **pivot_root** | Agent can't see host filesystem paths outside explicit mounts. |
| **UTS namespace** | Own hostname, can't change the host's. |
| **Network namespace** | Optional (`network.mode = "none"` for full isolation). |

### What's exposed (by design)

| Resource | Why |
|----------|-----|
| **Workspace** | Bind-mounted r/w — the agent needs to read/write project files. |
| **Configured mounts** | Explicitly opted-in by the user (e.g. `~/.bashrc`, `~/.gitconfig`). |
| **Host network** | Default (`network.mode = "host"`) — agents need to install packages, hit APIs, run servers. |

### Known limitations / TODO

- **No PID namespace**: Host processes are visible inside the sandbox (enumerable but not readable). The agent can also signal processes owned by the host user. Fix: double-fork with `CLONE_NEWPID` when `network.mode != "host"` (PID isolation pairs naturally with network isolation — host networking needs PID visibility for port conflict debugging). Requires changes to `create_session`, `nsenter_shell`, and fresh `/proc` mount inside the new PID namespace.
- **No seccomp filter**: All syscalls are allowed within the user namespace. Low priority since user namespace already limits what privileged syscalls can actually do.
- **Full capabilities in user namespace**: Expected — the agent needs `CAP_SYS_ADMIN` (for mounts, package installs) and other caps for normal dev work.

## File layout

```
~/.coop/
├── daemon.sock          # Unix socket
├── daemon.pid           # PID file
├── daemon.log           # Daemon logs
├── rootfs/
│   └── base/            # Shared base rootfs (from OCI image + setup)
├── oci-cache/           # Downloaded OCI layers
├── volumes/
│   └── claude-config/   # Named volume data
└── sessions/
    └── my-project/
        ├── upper/       # Overlayfs upper layer (per-session writes)
        ├── work/        # Overlayfs work dir
        ├── merged/      # Mount point (active while session runs)
        └── persist/     # Persistent data (survives kill)
```

## Source layout

```
src/
├── main.rs
├── cli/mod.rs           # CLI parsing and dispatch
├── config/
│   ├── coopfile.rs      # coop.toml parsing, merging, validation
│   └── paths.rs         # ~/.coop/ path helpers
├── daemon/
│   ├── client.rs        # Client-side daemon connection
│   ├── server.rs        # Server-side connection handling + stream mode
│   ├── session.rs       # SessionManager, PtyState, exit watchers
│   ├── spawn.rs         # Daemon auto-spawn logic
│   └── logs.rs          # Daemon log tailing
├── ipc/
│   ├── messages.rs      # Command, Response, DaemonEvent types
│   └── codec.rs         # MessageCodec + StreamCodec (framing)
├── sandbox/
│   ├── namespace.rs     # create_session, nsenter_shell, kill_session
│   └── init.rs          # OCI image pull, rootfs build
├── pty/
│   ├── filter.rs        # Input filtering (Ctrl+C debounce, block sequences)
│   └── manager.rs       # (unused, planned PTY pool)
├── web/
│   ├── server.rs        # Axum web server
│   └── api.rs           # REST + WebSocket API
└── tunnel/
    └── signaling.rs     # (stub, planned WebRTC)
```
