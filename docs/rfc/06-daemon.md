# Section 6: Daemon & Session Management

## 6.1 Overview

The daemon is a background process that owns all session namespaces, PTY masters, web servers, and tunnel connections. It is invisible to the user — auto-spawned when needed, auto-shutdown when idle.

## 6.2 Auto-Spawn

Every `coop` CLI invocation begins with `ensure_daemon()`:

1. Attempt to connect to `~/.coop/sock` (unix domain socket)
2. **Connected** → perform protocol version handshake (see [Section 7.2.1](./07-ipc.md#721-protocol-version-handshake)), then send the command
3. **Connection refused / socket missing** → fork self as daemon:
   a. Double-fork via `fork::daemon()` to detach from terminal and reparent to PID 1
   b. Child (now the daemon) binds `~/.coop/sock` and enters the tokio event loop
   c. Parent polls `~/.coop/sock` until it accepts connections (timeout: 2.5s)
   d. Parent sends the original command

The daemon MUST write its PID to `~/.coop/daemon.pid` after binding the socket. This is advisory only — the socket is the authoritative liveness check.

The daemon MUST redirect stdout/stderr to `~/.coop/logs/daemon.log`.

### 6.2.1 Race Conditions

If two `coop` commands run simultaneously and both detect no daemon:

- Both attempt the double-fork
- Only one will successfully `bind()` the socket (the other gets `EADDRINUSE`)
- The loser MUST detect this, abort its daemon attempt, and connect as a client
- The implementation SHOULD use a file lock on `~/.coop/daemon.lock` during the spawn window to reduce races

## 6.3 Auto-Shutdown

The daemon MUST track an idle timer. The timer resets on:

- Any client connection
- Any session activity (PTY I/O)
- Any active web server or tunnel

When the timer expires (default: 30 seconds) AND all of the following are true:

- Zero active sessions
- Zero connected clients
- Zero active web servers
- Zero active tunnels

The daemon MUST clean up and exit. The socket file and PID file are removed on exit.

> **Note:** The idle timeout only applies when there are no sessions. A daemon with active sessions MUST NOT auto-shutdown regardless of client connections.

## 6.4 Graceful Shutdown

On `SIGTERM` or `coop shutdown`:

1. Stop accepting new connections
2. Close all web servers and tunnels
3. For each session: send `{"event": "detached"}` to all connected clients (see [Section 7.6.2](./07-ipc.md#762-daemon--client)), then close connections. Do NOT kill the namespace process.
4. Remove socket and PID files
5. Exit

Orphaned session namespaces continue running. They can be discovered via `/proc` environ scanning (see [Section 6.6](#66-session-discovery)) and reattached when a new daemon spawns.

On `SIGKILL` (unclean):

- Socket file may be stale. The next `ensure_daemon()` MUST detect a stale socket (connect fails despite file existing), remove it, and spawn a fresh daemon.
- Session namespaces survive (reparented to init). They are rediscovered on next daemon startup.

## 6.5 Session Lifecycle

### 6.5.1 Creation

A session is created when:

- User runs `coop` or `coop -d` in a workspace
- User creates a session via web UI

The daemon:

1. Parses and merges the Coopfile
2. Assigns a session name (derived from workspace directory basename, or user-specified)
3. Creates the namespace (see [Section 5.5](./05-sandbox.md#55-session-startup-sequence))
4. Allocates PTY 0 for the agent, starts the agent command
5. Sets `COOP_SESSION`, `COOP_WORKSPACE`, `COOP_CREATED` environment variables in the namespace init process
6. Registers the session in its in-memory session table

### 6.5.2 Attach

A client (local terminal, web, tunnel) attaches to a session by name. The daemon:

1. Looks up the session
2. Creates a bridge between the client's connection and the requested PTY
3. Multiple clients MAY attach to the same PTY simultaneously (output is fanned out, input is merged)

### 6.5.3 Shell Spawn

Within an existing session, additional shell PTYs can be created:

1. Client sends a `shell` command for a session
2. Daemon `fork()`s inside the existing namespace (via `nsenter` or `setns()`)
3. Allocates a new PTY, execs `/bin/sh` (or the user's preferred shell)
4. Returns the PTY ID to the client

Shell PTYs are tracked per-session and have independent lifecycle from the agent PTY.

### 6.5.4 Agent Restart

If the agent process exits and `[session].auto_restart` is true:

1. Daemon detects the PTY master has closed (agent exited)
2. Sends `{"event": "pty_exited", "code": N}` to all attached clients (see [Section 7.6.2](./07-ipc.md#762-daemon--client))
3. Sends `{"event": "pty_restarting", "delay_ms": N}` to all attached clients
4. Waits `[session].restart_delay_ms` milliseconds
5. Allocates a new PTY inside the same namespace
6. Execs the agent command again
7. All attached clients are seamlessly reconnected to the new PTY

If `auto_restart` is false, the PTY is marked as exited. Attached clients receive `{"event": "pty_exited", "code": N}` only.

### 6.5.5 Destruction

A session is destroyed when:

- User runs `coop kill <name>`
- User kills via web UI

The daemon:

1. Sends `SIGTERM` to the namespace init process (which cascades to all processes inside)
2. Waits up to 5 seconds for graceful shutdown
3. Sends `SIGKILL` if processes remain
4. Unmounts the overlayfs
5. Cleans up session directory (preserving `persist/`)
6. Removes session from in-memory table

## 6.6 Session Discovery

Sessions are discoverable without any file-based registry by scanning `/proc`:

1. Iterate `/proc/*/environ` for all processes owned by the current UID
2. Look for `COOP_SESSION` environment variable
3. Extract `COOP_WORKSPACE` and `COOP_CREATED` from the same environ

This is used by:

- `coop ls` when no daemon is running (fallback discovery)
- A new daemon on startup to rediscover orphaned sessions from a crashed daemon
- The daemon itself as a consistency check

The `procfs` crate provides this functionality via `procfs::process::all_processes()` and `Process::environ()`.

> **Note:** `/proc/<pid>/environ` is only readable by the same UID (or root), which is the correct security boundary.

## 6.7 Workspace Awareness

The daemon tracks sessions by workspace path. When a `coop` command arrives without an explicit session name:

1. The client sends its current working directory
2. The daemon looks up sessions by workspace path
3. If found → attach to existing session
4. If not found → create a new session

This enables the "smart default" UX where `coop` in a project directory just does the right thing.
