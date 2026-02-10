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

When `coop shell` or `coop restart` spawns a new process in an existing box:

1. Open `/proc/<pid>/ns/{user,mnt,uts,net}` for the namespace init process
2. Open `/proc/<pid>/root` to get a handle to the namespace's root filesystem
3. **fork()**
4. Child: **setns()** into each namespace (user first, then mount, uts, net)
5. Child: **fchdir()** to the root fd, then **chroot(".")**
6. Child: **exec()** the shell command

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
