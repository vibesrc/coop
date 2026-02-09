# Section 3: Architecture

## 3.1 Overview

Coop is a single binary that serves as both CLI client and daemon. It has three architectural layers:

```
┌─────────────────────────────────────────────────────────┐
│  Access Layer                                           │
│  ┌──────────┐  ┌──────────────┐  ┌───────────────────┐ │
│  │ Local    │  │ Web UI       │  │ WebRTC Tunnel     │ │
│  │ Terminal │  │ (LAN/xterm)  │  │ (P2P/WAN)         │ │
│  └────┬─────┘  └──────┬───────┘  └────────┬──────────┘ │
├───────┼────────────────┼───────────────────┼────────────┤
│  Session Layer         │                   │            │
│  ┌─────────────────────┴───────────────────┴──────────┐ │
│  │  Daemon (auto-spawned, owns all sessions)          │ │
│  │  ┌─────────┐ ┌─────────┐ ┌─────────┐              │ │
│  │  │Session A│ │Session B│ │Session C│  ...          │ │
│  │  │ PTY 0:a │ │ PTY 0:a │ │ PTY 0:a │              │ │
│  │  │ PTY 1:sh│ │ PTY 1:sh│ │         │              │ │
│  │  └────┬────┘ └────┬────┘ └────┬────┘              │ │
│  └───────┼────────────┼──────────┼────────────────────┘ │
├──────────┼────────────┼──────────┼──────────────────────┤
│  Sandbox Layer        │          │                      │
│  ┌────────────────────┴──────────┴────────────────────┐ │
│  │  Linux Namespaces + Overlayfs + Bind Mounts        │ │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐         │ │
│  │  │NS: user  │  │overlayfs │  │bind mount│         │ │
│  │  │NS: mount │  │base+upper│  │/workspace│         │ │
│  │  │NS: pid   │  │          │  │          │         │ │
│  │  │NS: uts   │  │          │  │          │         │ │
│  │  │NS: net?  │  │          │  │          │         │ │
│  │  └──────────┘  └──────────┘  └──────────┘         │ │
│  └────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
```

### Figure 3-1: Layer Interaction

## 3.2 Single Binary Design

The `coop` binary operates in two modes determined at runtime:

**Client mode** (default): Every user-facing `coop` command runs in client mode. It connects to the daemon over `~/.coop/sock` and sends a command. If the daemon isn't running, the client forks itself into daemon mode first.

**Daemon mode**: The daemon is a long-running async process (tokio) that:
- Listens on `~/.coop/sock` for client commands
- Owns all session namespaces and their PTY master file descriptors
- Runs the embedded web server when `serve` is active
- Manages WebRTC tunnels when `tunnel` is active
- Auto-exits after 30 seconds of idle (no sessions, no active commands)

The user never explicitly starts or stops the daemon. It is an implementation detail.

## 3.3 Data Flow

### 3.3.1 Local Terminal Attach

```
User terminal
  │ stdin/stdout
  ▼
coop attach ──UDS──► Daemon ──PTY master──► Agent (in namespace)
```

The client sends an `attach` command over the unix socket. The daemon bridges the client's connection to the PTY master for the requested session. Raw terminal bytes flow bidirectionally.

### 3.3.2 Web UI Attach

```
Browser (xterm.js)
  │ WebSocket
  ▼
Daemon HTTP server ──► Input filter ──► PTY master ──► Agent
```

The embedded web server serves static assets (xterm.js, HTML, CSS — all baked into the binary). WebSocket connections are bridged to PTY masters with an input filtering layer for agent PTYs.

### 3.3.3 WebRTC Tunnel

```
Remote browser (xterm.js)
  │ WebRTC DataChannel (P2P, DTLS encrypted)
  ▼
Daemon WebRTC listener ──► Input filter ──► PTY master ──► Agent
```

The tunnel establishes a direct peer-to-peer connection. Terminal data flows over a DataChannel. Signaling is done out-of-band via QR code or copy-paste.

## 3.4 Filesystem Layout

```
~/.coop/
  rootfs/
    base/                   # Built rootfs from coop init (read-only at runtime)
  sessions/
    <name>/
      upper/                # Overlayfs upper dir (session writes, ephemeral)
      work/                 # Overlayfs work dir (kernel requirement)
      persist/              # Persisted dirs (.claude, etc.) across sessions
  cache/
    oci/                    # Cached OCI image layers
  logs/
    daemon.log              # Daemon log output
  sock                      # Unix domain socket (daemon IPC)
  machine_id                # Stable random ID for tunnel identity (generated once)
  config/
    default.toml            # Global default Coopfile
```

## 3.5 Process Tree

When two sessions are running with the daemon:

```
init (PID 1)
  └── coop daemon
        ├── [Session "nlst"]
        │     ├── PID 1 (inside NS): claude
        │     └── PID 2 (inside NS): /bin/sh (user shell)
        └── [Session "llmq"]
              └── PID 1 (inside NS): claude
```

The daemon is the direct parent of all namespace init processes. If the daemon is killed, sessions receive SIGHUP. The daemon SHOULD handle SIGTERM gracefully by detaching sessions rather than killing them (see [Section 6.4](./06-daemon.md#64-graceful-shutdown)).

## 3.6 Rust Crate Dependencies

The implementation SHOULD use the following crate ecosystem:

| Crate | Purpose |
|-------|---------|
| `fork` | Double-fork for daemon spawning |
| `procfs` | `/proc` scanning for session discovery |
| `nix` | Namespace syscalls, forkpty, unix sockets, signals |
| `tokio` | Async runtime for daemon event loop |
| `axum` | HTTP server for web UI and REST API |
| `tokio-tungstenite` | WebSocket for terminal streaming |
| `rust-embed` | Embed static web assets into binary |
| `webrtc-rs` or `str0m` | WebRTC DataChannel for tunnels |
| `qrcode` | QR code generation in terminal |
| `clap` | CLI argument parsing |
| `tokio-util` | Length-delimited codec for IPC framing |
| `aho-corasick` | Streaming multi-pattern matching for input filtering |
| `serde` / `toml` | Coopfile parsing |

> **Note:** The specific crates are recommendations. The implementation MAY substitute alternatives that provide equivalent functionality, provided all behavioral requirements in this spec are met.
