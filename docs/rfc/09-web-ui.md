# Section 9: Web UI & Local Serving

## 9.1 Overview

`coop serve` starts an HTTP/WebSocket server embedded in the daemon. It serves a mobile-friendly web interface for managing sessions and attaching to PTYs. All static assets (HTML, CSS, JS, xterm.js) are compiled into the binary via `rust-embed`.

## 9.2 Activation

The web server is started via the `serve` IPC command (see [Section 7.4.1](./07-ipc.md#741-commands)):

```bash
coop serve                    # default: 127.0.0.1:8888
coop serve --port 3000        # custom port
coop serve --host 0.0.0.0     # LAN accessible (explicit opt-in)
```

The daemon starts an axum HTTP server on the requested address. The server runs as a task within the daemon's tokio runtime — it does NOT spawn a separate process.

On startup, the CLI SHOULD print:

```
  🐔 Coop web UI

  Local:   http://localhost:8888?token=f7a3b1...
  Network: http://192.168.1.50:8888?token=f7a3b1...

  ██████████████
  ██          ██    ← QR code encoding the network URL (with token)
  ██  ▄▄▄▄▄  ██
  ██          ██
  ██████████████
```

The QR code MUST encode the network-accessible URL with the token included, so scanning it from a phone grants immediate access.

## 9.3 HTTP Endpoints

### 9.3.1 Static Assets

| Path | Content |
|------|---------|
| `GET /` | Main HTML page (session list + terminal UI) |
| `GET /assets/*` | CSS, JS, xterm.js, xterm-addon-fit, fonts |

All assets are served from memory (embedded in binary). No disk access required.

### 9.3.2 REST API

| Method | Path | Description |
|--------|------|-------------|
| `GET /api/sessions` | List all sessions with PTY info |
| `POST /api/sessions` | Create a new session |
| `DELETE /api/sessions/:name` | Kill a session |
| `POST /api/sessions/:name/shell` | Spawn a shell PTY |

These mirror the IPC commands and are thin wrappers around the daemon's internal session management.

### 9.3.3 WebSocket

| Path | Description |
|------|-------------|
| `GET /ws?session=<name>&pty=<id>` | Attach to a session PTY |

The WebSocket connection carries:

- **Server → Client**: raw terminal output bytes (binary frames)
- **Client → Server**: raw terminal input bytes (binary frames), or JSON control messages (text frames) for resize events

JSON control messages:

```json
{"type": "resize", "cols": 120, "rows": 40}
```

The server applies input filtering to all WebSocket connections on agent PTYs (PTY 0). See [Section 8.4](./08-pty.md#84-input-filtering).

## 9.4 Web UI Design

The web UI is a single-page application optimized for both desktop and mobile.

### 9.4.1 Layout

The UI has two levels of navigation: a **session sidebar** and **PTY tabs** within each session. The main terminal area supports **split panels** — any PTY tab can be dragged to split horizontally or vertically, VS Code-style.

**Mobile (< 768px):**

```
┌──────────────────────┐
│ 🐔 Coop   [≡] [+]   │  ← header, [≡] toggles session drawer
├──────────────────────┤
│                      │
│   xterm.js terminal  │  ← full-width terminal (single panel)
│                      │
│                      │
├──────────────────────┤
│ [🤖 Agent] [sh1] [+] │  ← PTY tabs, agent pinned first
└──────────────────────┘
```

On mobile, the session sidebar is a slide-out drawer toggled from the header. Split panels are disabled — only one terminal is visible at a time, with swipe to switch PTY tabs.

**Desktop (≥ 768px):**

```
┌────────┬─────────────────────┬──────────────────┐
│Sessions│                     │                  │
│        │  xterm.js (Agent)   │  xterm.js (sh1)  │
│ ● nlst │                     │                  │
│ ● llmq │                     │                  │
│        │                     │                  │
│[+ New] ├─────────────────────┴──────────────────┤
│        │ [🤖 Agent] [Shell 1] [Shell 2] [+]     │
└────────┴─────────────────────────────────────────┘
```

The desktop layout supports multiple split panels. Each panel displays one PTY. Panels are resizable by dragging the divider. PTY tabs along the bottom reflect all PTYs in the active session — the focused panel is highlighted in the tab bar.

**Session sidebar:**

- Lists all sessions on the daemon (fetched via `GET /api/sessions`)
- Shows status indicators: running/exited, attached client count
- Clicking a session switches the main area to that session's PTY tabs
- The browser MAY have multiple sessions "open" simultaneously — each maintains its own WebSocket connections and PTY tab state
- `[+ New]` button creates a new session

**PTY tabs:**

- The **Agent tab** (PTY 0) is always pinned as the first tab and cannot be closed or reordered
- Shell tabs (PTY 1+) appear after the Agent tab in creation order
- Shell tabs are closeable (closing disconnects from the PTY but does not kill the shell process)
- `[+]` button spawns a new shell PTY via `POST /api/sessions/:name/shell`
- Tabs can be dragged into the terminal area to create a split panel

### 9.4.2 Split Panels

Split panels allow viewing multiple PTYs side-by-side within a session. The implementation SHOULD use a recursive split model (like VS Code):

- Each panel contains one xterm.js instance connected to one PTY
- Panels can be split horizontally or vertically by dragging a tab into the panel area
- Panel sizes are adjustable by dragging the divider between them
- Closing all splits returns to a single-panel view
- Each panel independently handles terminal resize via the `fit` addon

Split panels are a SHOULD — the initial implementation MAY ship with tabs only and add splits later.

### 9.4.3 Features

The web UI MUST support:

- Listing all sessions with status indicators (running, attached count)
- Creating new sessions (name, workspace path)
- Attaching to any PTY in any session
- Spawning new shell PTYs within a session
- Switching between PTYs via tabs with pinned Agent tab
- Terminal resize (xterm.js `fit` addon, responds to browser resize)
- Killing sessions (with confirmation)
- Visual indicator when input is blocked by the filter

The web UI SHOULD support:

- Split panels (VS Code-style resizable panes)
- Touch-friendly controls for mobile
- Swipe between PTY tabs on mobile
- Keyboard shortcut overlay
- Session creation from a directory browser
- Dark/light theme (default: dark, matching terminal aesthetic)

### 9.4.4 Client-Side State

The web UI persists layout state in `localStorage` to survive page reloads and reconnections:

| Key | Value | Purpose |
|-----|-------|---------|
| `coop:machines` | `[{machine_id, hostname, lastSeen}]` | Known machines (tunnel mode) |
| `coop:sessions` | `["nlst", "llmq"]` | Which sessions were open |
| `coop:active-session` | `"nlst"` | Last focused session |
| `coop:layout:<session>` | `{splits, activePty}` | Panel layout and active PTY per session |
| `coop:layout:<machine_id>` | `{sessions, splits}` | Per-machine layout (tunnel mode) |
| `coop:theme` | `"dark"` | Theme preference |

**HTTP mode (`coop serve`) — on page load:**

1. Read saved state from `localStorage`
2. Fetch current sessions from `GET /api/sessions`
3. Reconnect to sessions that are still alive (intersect saved list with live list)
4. Restore panel layout for reconnected sessions
5. Discard state for sessions that no longer exist

The token from the URL (`?token=...`) SHOULD be stored in `sessionStorage` (not `localStorage`) so it persists across navigations within the same tab but is cleared when the tab is closed.

**WebRTC mode (`opencoop.sh`) — on page load:**

1. If URL has `#<envelope>`: parse `machine_id`, connect via WebRTC, save/update machine in `coop:machines`
2. If no fragment: show machine dashboard from `coop:machines`
3. On successful connection: restore saved layout for that `machine_id`
4. On disconnect: mark machine as offline in the dashboard, preserve layout for next reconnect

### 9.4.5 Dependencies (Embedded)

The following libraries MUST be embedded in the binary:

| Library | Version | Purpose |
|---------|---------|---------|
| `react` | 18.x+ | UI framework |
| `react-dom` | 18.x+ | DOM rendering |
| `allotment` | 1.x | VS Code-style resizable split panels |
| `xterm.js` | 5.x | Terminal emulator |
| `xterm-addon-fit` | 0.x | Auto-resize terminal to container |
| `xterm-addon-webgl` | 0.x | GPU-accelerated rendering (OPTIONAL) |

No CDN dependencies. The web UI MUST work fully offline on the local network. All assets are bundled at build time and embedded in the binary via `rust-embed`.

## 9.5 Authentication

`coop serve` MUST always require token authentication. On startup, the daemon generates a cryptographically random token (minimum 128 bits of entropy) unless the user provides one via `--token <token>`.

Token validation:

- Accepted as `?token=<t>` query parameter or `Authorization: Bearer <t>` header
- The token is embedded in the QR code URL and the printed URLs at startup — scanning the QR grants access with no extra step
- Requests without a valid token MUST receive `403 Forbidden`
- WebSocket upgrade requests MUST include the token (via query parameter)

The `--token <token>` flag allows setting a specific token instead of auto-generating one. This is useful for scripts or bookmarked URLs.

For remote access beyond LAN, `coop tunnel` (which uses DTLS encryption) is the recommended approach (see [Section 10](./10-tunnel.md)).

## 9.6 Shared UI Architecture

The Coop web UI is a single React application deployed in two contexts:

| Context | Hosting | Transport | Auth |
|---------|---------|-----------|------|
| `coop serve` | Embedded in binary via `rust-embed` | HTTP REST + WebSocket | Token (query param) |
| `coop tunnel` | Static site at `opencoop.sh` | WebRTC DataChannels | DTLS (implicit via SDP) |

The UI code is identical in both cases. The only difference is the transport layer.

### 9.6.1 Transport Abstraction

The UI MUST use a transport abstraction that hides whether the backend is HTTP/WebSocket or WebRTC DataChannels. The transport interface provides:

- **`listSessions()`** → HTTP: `GET /api/sessions` / WebRTC: `{"cmd": "ls"}` on control channel
- **`createSession(opts)`** → HTTP: `POST /api/sessions` / WebRTC: `{"cmd": "create", ...}` on control channel
- **`killSession(name)`** → HTTP: `DELETE /api/sessions/:name` / WebRTC: `{"cmd": "kill", ...}` on control channel
- **`spawnShell(session, opts)`** → HTTP: `POST /api/sessions/:name/shell` / WebRTC: `{"cmd": "shell", ...}` on control channel
- **`attachPty(session, ptyId, opts)`** → HTTP: opens WebSocket to `/ws?session=...&pty=...` / WebRTC: sends `attach` on control channel, receives new PTY DataChannel

Each PTY attachment returns a bidirectional byte stream (tagged frames). The xterm.js component reads/writes this stream without knowing the underlying transport.

### 9.6.2 Build & Deployment

The React app is built as a static bundle:

1. `npm run build` produces `dist/` with HTML, JS, CSS
2. For `coop serve`: the `dist/` contents are embedded into the Rust binary via `rust-embed` at compile time
3. For `opencoop.sh`: the same `dist/` is deployed to GitHub Pages (or similar static hosting)

The build SHOULD produce a single-page app with a small footprint. Code splitting by transport (HTTP vs WebRTC) is RECOMMENDED so the static site doesn't bundle unused HTTP client code and vice versa.

## 9.7 Lifecycle

The web server runs until:

- The user sends Ctrl+C to the `coop serve` CLI process
- The user runs `coop serve --stop`
- The daemon shuts down

Stopping the web server does NOT affect running sessions. They continue running and are accessible via `coop attach` or a new `coop serve`.
