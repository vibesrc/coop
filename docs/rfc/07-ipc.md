# Section 7: IPC Protocol

## 7.1 Overview

All communication between the `coop` CLI client and the daemon occurs over a unix domain socket at `~/.coop/sock`. The protocol is a length-prefixed message exchange that upgrades to tagged-frame streaming for PTY attach operations.

## 7.2 Transport

The transport is a Unix Domain Socket (UDS) of type `SOCK_STREAM`. The daemon MUST listen on `~/.coop/sock`. Clients connect, send a command, and receive a response.

The socket file MUST have permissions `0600` (owner read/write only).

**Peer credential verification**: On each new connection, the daemon MUST verify the connecting client's UID via `SO_PEERCRED` (Linux) or the equivalent `peer_cred()` call. Connections from a different UID MUST be rejected immediately.

**Socket creation safety**: The daemon MUST create the socket atomically:

1. If `~/.coop/sock` already exists, verify ownership (same UID) then unlink it
2. Bind the new socket with `umask(0o177)` to enforce `0600` permissions
3. The daemon MUST NOT follow symlinks when unlinking or binding — if the path is a symlink, abort with an error

### 7.2.1 Protocol Version Handshake

Immediately after connecting, the client MUST send a version handshake message:

```json
{"version": 1}
```

The daemon responds with:

```json
{"version": 1, "ok": true}
```

If the version is unsupported, the daemon responds with:

```json
{"ok": false, "error": "VERSION_MISMATCH", "message": "Unsupported protocol version 2"}
```

…and closes the connection. All subsequent messages use the framing defined in Section 7.3.

## 7.3 Message Framing

Messages are length-prefixed:

```
┌──────────────┬───────────────────────┐
│ Length (4 BE) │ JSON payload (UTF-8)  │
└──────────────┴───────────────────────┘
```

- **Length**: 4 bytes, big-endian unsigned 32-bit integer. Indicates the byte length of the JSON payload.
- **Payload**: UTF-8 encoded JSON object.

Maximum message size: 1MB. Messages exceeding this MUST be rejected.

> **Implementation note:** The `tokio-util::codec::LengthDelimitedCodec` crate provides a production-ready implementation of this framing. The spec remains transport-agnostic, but implementations SHOULD use this codec rather than hand-rolling length-prefix parsing.

## 7.4 Command Messages

Client → Daemon:

```json
{
  "cmd": "<command>",
  ...command-specific fields
}
```

### 7.4.1 Commands

**`create`** — Create a new session

```json
{
  "cmd": "create",
  "name": "nlst",           // OPTIONAL, derived from workspace basename if omitted
  "workspace": "/home/b/github/nlst",
  "coopfile": "...",         // OPTIONAL, serialized Coopfile override
  "detach": false            // true = create only, false = create and attach
}
```

**`attach`** — Attach to a session PTY

```json
{
  "cmd": "attach",
  "session": "nlst",         // session name or workspace path (see resolution below)
  "pty": 0,                  // PTY index, 0 = agent
  "cols": 120,               // terminal width
  "rows": 40                 // terminal height
}
```

**Session resolution**: If `session` matches an existing session name, use it. Otherwise, treat it as a workspace path and resolve to the session bound to that workspace. Session names MUST NOT contain `/` — this disambiguates names from paths.

**`shell`** — Spawn a new shell PTY in a session

```json
{
  "cmd": "shell",
  "session": "nlst",
  "command": "/bin/sh",      // OPTIONAL, default: /bin/sh
  "cols": 120,
  "rows": 40
}
```

Response: `{"ok": true, "pty": 2}` where `pty` is the assigned PTY index.

**`ls`** — List sessions

```json
{
  "cmd": "ls"
}
```

**`kill`** — Destroy a session

```json
{
  "cmd": "kill",
  "session": "nlst"
}
```

**`resize`** — Resize a PTY (sent mid-stream during attach)

```json
{
  "cmd": "resize",
  "cols": 200,
  "rows": 50
}
```

**`serve`** — Start web server

```json
{
  "cmd": "serve",
  "port": 8888,
  "host": "127.0.0.1",      // OPTIONAL, default: 127.0.0.1
  "token": "abc123"          // OPTIONAL, override auto-generated token
}
```

Response: `{"ok": true, "port": 8888, "host": "127.0.0.1", "token": "f7a3b1..."}`

The daemon MUST always generate a random token if one is not provided. The token is returned in the response and embedded in the QR code URL displayed at startup. Binding to non-localhost addresses (e.g., `0.0.0.0`) requires an explicit `--host` flag from the CLI.

**`tunnel`** — Start WebRTC tunnel for a session

```json
{
  "cmd": "tunnel",
  "session": "nlst"
}
```

**`shutdown`** — Gracefully shut down daemon

```json
{
  "cmd": "shutdown"
}
```

Response: `{"ok": true}`

**`detach`** — Clean disconnect during stream mode

```json
{
  "cmd": "detach"
}
```

Sent as a control message within stream mode (see Section 7.6). The daemon responds with `{"ok": true}` then closes the connection. The session and PTY remain running.

## 7.5 Response Messages

Daemon → Client:

```json
{
  "ok": true,
  ...response-specific fields
}
```

Or on error:

```json
{
  "ok": false,
  "error": "SESSION_NOT_FOUND",
  "message": "Session 'nlst' not found"
}
```

The `error` field is a machine-readable error code (SCREAMING_SNAKE_CASE). The `message` field is a human-readable description. Clients SHOULD match on `error`, not `message`.

#### Defined Error Codes

| Code | Meaning |
|------|---------|
| `SESSION_NOT_FOUND` | No session with the given name or workspace path |
| `SESSION_EXISTS` | A session with that name already exists |
| `PTY_NOT_FOUND` | The requested PTY index does not exist in the session |
| `INVALID_COMMAND` | Unknown or malformed command |
| `VERSION_MISMATCH` | Client protocol version not supported |
| `MESSAGE_TOO_LARGE` | Message exceeds the 1MB size limit |

### 7.5.1 Responses

**`create` response:**

```json
{
  "ok": true,
  "session": "nlst",
  "pid": 42381
}
```

**`ls` response:**

```json
{
  "ok": true,
  "sessions": [
    {
      "name": "nlst",
      "workspace": "/home/b/github/nlst",
      "pid": 42381,
      "created": 1739097600,
      "ptys": [
        { "id": 0, "role": "agent", "command": "claude" },
        { "id": 1, "role": "shell", "command": "/bin/sh" }
      ],
      "web_clients": 2,
      "local_clients": 1
    }
  ]
}
```

**`tunnel` response:**

```json
{
  "ok": true,
  "offer_sdp": "...",       // base64-encoded SDP offer
  "short_code": "abc123",   // human-readable short code
  "qr_data": "..."          // data to encode in QR
}
```

## 7.6 Stream Mode

After an `attach` or `shell` command, the connection upgrades to **stream mode**. Stream mode retains the `LengthDelimitedCodec` framing — there is no "upgrade" to raw bytes. Each frame carries a 1-byte type tag after the length prefix:

```
┌──────────────┬──────────┬─────────────────────┐
│ Length (4 BE) │ Type (1) │ Payload             │
└──────────────┴──────────┴─────────────────────┘
```

- **Type `0x00`** — PTY data: raw terminal bytes (keystrokes or output)
- **Type `0x01`** — Control message: JSON payload, same schema as command/response messages

This framing is **bidirectional**. Both client→daemon and daemon→client use tagged frames.

### 7.6.1 Client → Daemon

- **PTY data frames** (`0x00`): raw terminal input bytes (keystrokes)
- **Control frames** (`0x01`): JSON commands such as `resize` or `detach`

### 7.6.2 Daemon → Client

- **PTY data frames** (`0x00`): raw terminal output bytes
- **Control frames** (`0x01`): JSON event messages

#### Daemon Event Messages

```json
{"event": "pty_exited", "code": 0}
```

```json
{"event": "pty_restarting", "delay_ms": 1000}
```

```json
{"event": "detached"}
```

Stream mode continues until the client sends a `detach` control message, the client disconnects, or the PTY exits.

## 7.7 Concurrency

The daemon MUST handle multiple simultaneous client connections. Each connection is an independent command stream. Multiple clients MAY attach to the same PTY.

**Output fan-out**: PTY output is sent to all attached clients. The daemon MUST NOT block on slow clients — if a client's write buffer is full, the daemon MAY drop frames for that client. The daemon SHOULD send a lag indicator to the affected client when frames are dropped so the client can inform the user.

**Input merge**: Input bytes from all attached clients are forwarded to the PTY master in arrival order. There is no atomicity guarantee across keystrokes from different clients — interleaving is possible and expected (same behavior as multiple `tmux attach` clients).

> **Implementation note:** The fan-out SHOULD use `tokio::sync::broadcast` to decouple PTY output from per-client write speeds.
