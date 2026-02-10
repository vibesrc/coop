# CLI Reference

## Smart default

```
coop [OPTIONS]
```

With no subcommand, coop creates a box for the current directory (if one doesn't exist) and attaches to the agent (PTY 0). If a box already exists, it reattaches.

| Flag | Description |
|------|-------------|
| `-d, --detach` | Create box but don't attach |
| `-w, --workspace <PATH>` | Workspace directory (default: cwd) |
| `-n, --name <NAME>` | Box name (default: directory basename) |
| `-b, --build` | Force rebuild rootfs before starting |
| `--no-cache` | Ignore cached rootfs (use with `--build`) |
| `-v, --verbose` | Verbose logging |
| `-q, --quiet` | Suppress non-essential output |

## Core commands

### coop attach [NAME]

Reattach to a running box's agent (PTY 0). Defaults to the box for the current directory.

### coop ls [--json]

List all running boxes with their workspace, PTY count, client count, and age.

### coop kill [NAME] [--all] [-f]

Kill a box and all its processes. `--all` kills every box. `-f` sends SIGKILL immediately (default: SIGTERM with 5s grace period).

### coop logs [-f] [-n N]

View the agent's (PTY 0) scrollback buffer. `-f` follows live output (like `tail -f`). `-n 50` shows the last 50 lines. Press `Ctrl+]` to stop following.

### coop restart

Restart the agent process (PTY 0). Connected clients stay connected -- they see a brief gap then the new process output.

## Shell management

### coop shell [OPTIONS]

Open a shell session inside the box. Creates the box first if needed.

| Flag | Description |
|------|-------------|
| `-c, --command <CMD>` | Shell command (default: from config) |
| `--new` | Force a new shell (don't reuse existing) |

By default, `coop shell` reuses an existing shell running the same command. Use `--new` to always create a fresh one.

Exit with `Ctrl+D` (normal shell exit) to return to the host. The shell PTY is cleaned up automatically.

Detach with `Ctrl+]` to keep the shell running in the background.

### coop shell ls

List all PTY sessions in the current box (agent + shells).

### coop shell attach ID

Attach to a specific shell session by PTY ID.

### coop shell kill ID

Kill a specific shell session by PTY ID.

### coop shell logs ID [-f] [-n N]

View a shell's scrollback buffer. Same flags as `coop logs`.

### coop shell restart [ID]

Restart a shell process. Defaults to PTY 1 (first shell).

## Build & init

### coop init

Create a default `coop.toml` in the current directory.

### coop build [--no-cache]

Build the rootfs from the Coopfile. Pulls the base OCI image, unpacks it, and runs setup commands. `--no-cache` ignores previously cached layers and rootfs.

## System management

### coop system status

Show daemon status and session count.

### coop system logs [-f] [-n N]

Tail the daemon log file.

### coop system shutdown

Gracefully stop the daemon. Running sessions are unaffected -- the daemon will be auto-spawned again on next `coop` invocation.

### coop system volumes

List named volumes with their sizes.

### coop system volume-rm NAME

Remove a named volume.

### coop system volume-prune

Remove all named volumes.

### coop system df

Show disk usage for rootfs, OCI cache, volumes, and sessions.

### coop system clean [--all]

Remove the rootfs. `--all` also removes the OCI layer cache.

### coop system prune

Remove everything: rootfs, OCI cache, volumes, and session data.

## Web UI

### coop serve [-p PORT] [-H HOST] [--token TOKEN]

Start the embedded web UI. Default: `http://127.0.0.1:8888`.

### coop tunnel

Create a P2P WebRTC tunnel for remote access. (Coming soon.)

## Escape sequences

| Key | Action |
|-----|--------|
| `Ctrl+]` | Detach from the current session (keeps it running) |
| `Ctrl+D` | Normal shell exit (in shells only -- returns to host) |
| `Ctrl+C` | Interrupt (debounced, see `input_filter.ctrl_c_debounce_ms`) |

## Exit behavior

- **Agent (PTY 0)**: If `auto_restart` is enabled (default), the agent is restarted automatically after `restart_delay_ms`. Connected clients see a message and stay connected.
- **Shells (PTY 1+)**: When a shell exits, the PTY is cleaned up and the client returns to the host terminal.
