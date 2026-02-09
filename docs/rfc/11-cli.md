# Section 11: CLI Reference

## 11.1 Overview

The `coop` binary is the sole user-facing interface. All commands implicitly call `ensure_daemon()` (see [Section 6.2](./06-daemon.md#62-auto-spawn)) before executing.

## 11.2 Session Commands

### `coop`

Smart default. Behavior depends on context:

- If a session exists for the current workspace → attach to it
- If no session exists → create one and attach
- Equivalent to: `coop attach` or `coop create --attach`

```
coop [OPTIONS]

Options:
  -d, --detach           Create session but don't attach
  -w, --workspace <DIR>  Workspace directory (default: cwd)
  -n, --name <NAME>      Session name (default: derived from workspace basename)
```

**Examples:**

```bash
cd ~/github/nlst
coop                     # create or attach to "nlst" session
coop -d                  # create detached
coop -n my-session       # explicit name
coop -w ~/other/project  # explicit workspace
```

### `coop attach [SESSION]`

Attach to an existing session's agent PTY.

```
coop attach [SESSION]

Arguments:
  SESSION  Session name or workspace path (default: current workspace)
```

### `coop shell [SESSION]`

Spawn a new shell PTY inside an existing session and attach to it.

```
coop shell [SESSION] [OPTIONS]

Arguments:
  SESSION  Session name (default: current workspace's session)

Options:
  -c, --command <CMD>  Shell command (default: /bin/sh)
```

### `coop ls`

List all running sessions.

```
coop ls [OPTIONS]

Options:
  --json  Output as JSON
```

Output:

```
SESSION    WORKSPACE                STATE     PTYS   CLIENTS        AGE
nlst       ~/github/nlst           running   2      1 local         2h 15m
llmq       ~/github/llmq           running   1      0               15m
docproc    ~/github/docproc        running   3      2 web, 1 tunnel 3d 4h
```

### `coop kill [SESSION]`

Kill a session and all its processes.

```
coop kill [SESSION] [OPTIONS]

Arguments:
  SESSION  Session name (default: current workspace's session)

Options:
  --all    Kill all sessions
  -f       Force kill (SIGKILL, no grace period)
```

## 11.3 Environment Commands

### `coop init`

Build the rootfs from the Coopfile. This pulls the OCI image, installs packages, and runs setup commands. Required before first session.

```
coop init [OPTIONS]

Options:
  -f, --file <PATH>  Coopfile path (default: ./coop.toml)
  --no-cache         Ignore cached OCI layers
```

### `coop rebuild`

Rebuild the rootfs. Alias for `coop init --no-cache` with cleanup of old rootfs.

```
coop rebuild
```

### `coop status`

Show current rootfs info, Coopfile config, and daemon status.

```
coop status
```

Output:

```
Daemon:     running (PID 1234)
Rootfs:     built 2d ago (node:22-alpine + 12 packages)
Coopfile:   ./coop.toml
Agent:      claude
Network:    veth
Sessions:   3 running
```

## 11.4 Remote Access Commands

### `coop serve`

Start the embedded web UI server. A random auth token is always generated and embedded in the displayed URLs and QR code.

```
coop serve [OPTIONS]

Options:
  -p, --port <PORT>    Port number (default: 8888)
  -H, --host <HOST>    Bind address (default: 127.0.0.1)
  --token <TOKEN>      Use a specific auth token instead of auto-generating
  --stop               Stop the running web server
```

### `coop tunnel`

Create a P2P WebRTC tunnel. The tunnel provides access to all sessions on the daemon. A QR code is displayed for scanning from a phone or remote browser.

```
coop tunnel [OPTIONS]

Options:
  --stun <URL>         Custom STUN server
  --no-stun            Disable STUN (LAN only)
  --no-qr              Don't display QR code
```

## 11.5 Daemon Commands

### `coop shutdown`

Gracefully shut down the daemon. Sessions are detached, not killed.

```
coop shutdown
```

### `coop logs`

Tail the daemon log.

```
coop logs [OPTIONS]

Options:
  -f, --follow    Follow log output
  -n <LINES>      Number of lines (default: 50)
```

## 11.6 Global Options

These apply to all commands:

```
Options:
  --config <PATH>      Global config path (default: ~/.config/coop/default.toml)
  --socket <PATH>      Daemon socket path (default: ~/.coop/sock)
  -v, --verbose        Verbose output
  -q, --quiet          Suppress non-essential output
  --version            Print version
  -h, --help           Print help
```

## 11.7 Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error |
| 2 | Invalid arguments |
| 3 | Session not found |
| 4 | Daemon failed to start |
| 5 | Rootfs not built (run `coop init`) |
| 6 | Tunnel failed (NAT traversal failure) |
| 130 | Interrupted (Ctrl+C) |
