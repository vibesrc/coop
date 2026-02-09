# Section 8: PTY Management & Input Filtering

## 8.1 Overview

Each session contains one or more PTYs. PTY 0 is always the agent. Additional PTYs are user shells. The daemon holds all PTY master file descriptors and bridges them to clients with optional input filtering.

## 8.2 PTY Allocation

### 8.2.1 Agent PTY (PTY 0)

Created during session startup (see [Section 5.5](./05-sandbox.md#55-session-startup-sequence)):

1. Call `forkpty()` inside the namespace
2. Child side: `exec()` the agent command
3. Parent side (daemon): retain the master fd, register as PTY 0

The agent PTY is always present while the session exists. If the agent exits and `auto_restart` is true, a new PTY 0 is allocated and the agent is restarted.

### 8.2.2 Shell PTYs (PTY 1+)

Created on demand via the `shell` command:

1. `nsenter()` into the session's existing namespaces (mount, PID, UTS, net)
2. `forkpty()` inside the namespace
3. Child: `exec()` the shell command (default: `/bin/sh`)
4. Parent: retain master fd, assign next available PTY ID

Shell PTYs are destroyed when the shell process exits. They are NOT auto-restarted.

## 8.3 PTY Multiplexing

Multiple clients MAY connect to the same PTY simultaneously:

**Output fan-out**: When the PTY master produces output, the daemon writes it to ALL connected clients. Every client sees the same terminal state.

**Input merge**: When multiple clients send input, all keystrokes are forwarded to the PTY master in arrival order. There is no locking or arbitration — this matches the behavior of multiple `tmux attach` clients to the same session.

**Resize handling**: When any client sends a resize, the PTY is resized to those dimensions. If clients have different terminal sizes, the last resize wins. The implementation SHOULD track the smallest connected terminal size and use that, to avoid wrapping issues.

## 8.4 Input Filtering

Input filtering applies ONLY to:

- Web UI connections (WebSocket clients)
- Tunnel connections (WebRTC clients)

Input filtering does NOT apply to:

- Local terminal clients (`coop attach` from a terminal)
- Shell PTYs (PTY 1+) — only agent PTYs are filtered

### 8.4.1 Filter Rules

The input filter sits between the client connection and the PTY master write:

```
Web client → [Input Filter] → PTY master → Agent
```

#### Blocked Sequences

The following byte sequences MUST be blocked entirely on filtered connections:

| Sequence | Meaning | Reason |
|----------|---------|--------|
| `\x04` | Ctrl+D | Sends EOF, exits agent |
| `\x1c` | Ctrl+\ | SIGQUIT, kills agent |
| `exit\r` or `exit\n` | "exit" + enter | Exits shell/agent |
| `/exit\r` or `/exit\n` | "/exit" + enter | Claude Code exit command |
| `quit\r` or `quit\n` | "quit" + enter | Generic quit |

When a blocked sequence is detected, the filter MUST:

1. NOT forward the bytes to the PTY
2. Send a warning message to the client:
   `\r\n\x1b[1;33m⚠  Blocked from web. Use local terminal to exit.\x1b[0m\r\n`

#### Debounced Sequences

The following require rate limiting rather than blocking:

| Sequence | Meaning | Behavior |
|----------|---------|----------|
| `\x03` | Ctrl+C | Allow first, suppress subsequent within debounce window |

**Ctrl+C debounce logic:**

1. First `\x03` received → forward to PTY, start debounce timer
2. Subsequent `\x03` within `ctrl_c_debounce_ms` (default: 500ms) → suppress, send warning
3. After timer expires → next `\x03` is treated as "first" again

This allows the user to interrupt the agent's current generation (single Ctrl+C) while preventing the rapid Ctrl+C that exits Claude Code entirely.

### 8.4.2 Sequence Detection

For multi-byte blocked sequences (like `exit\r`), the filter MUST use a streaming multi-pattern matcher that maintains match state across input chunks. The implementation MUST use an Aho-Corasick automaton (e.g., the `aho-corasick` crate) rather than a rolling buffer:

1. At filter initialization, build an Aho-Corasick automaton from all blocked sequences (built-in + custom from Coopfile)
2. On each input chunk, advance the automaton state
3. If a match completes: suppress the matched bytes, send warning, do NOT forward
4. Bytes preceding a partial match MUST be held until the match either completes (suppress) or fails (forward)
5. **Partial match timeout**: if partial match state is held for >500ms with no new input, flush the held bytes as non-matching

This handles the case where `exit` arrives as separate keystrokes (`e`, `x`, `i`, `t`, `\r`) across multiple WebSocket messages, while avoiding unbounded buffering.

### 8.4.3 Custom Filters

The Coopfile `[input_filter].block_sequences` field allows additional blocked sequences. These are specified as strings and converted to byte sequences. Escape sequences like `\x03` MUST be supported.

## 8.5 PTY Lifecycle

### Table 8-1: PTY State Machine

| State | Description | Transitions |
|-------|-------------|-------------|
| `running` | Process is alive, PTY is active | → `exited` (process exits) |
| `exited` | Process has exited | → `restarting` (if auto_restart, agent only) → `dead` |
| `restarting` | Waiting restart_delay_ms | → `running` (new PTY allocated) |
| `dead` | Terminal state, PTY is closed | (removed from session) |

When a PTY transitions to `exited`, all connected clients receive:

```
\r\n\x1b[2m[process exited (code N)]\x1b[0m\r\n
```

If the PTY is restarting, clients additionally receive:

```
\x1b[2m[restarting in Nms...]\x1b[0m\r\n
```

And are automatically reconnected to the new PTY when it enters `running`.
