# Section 10: Tunnel & P2P Remote Access

## 10.1 Overview

`coop tunnel` establishes a peer-to-peer connection between the coop daemon and a remote browser using WebRTC DataChannels. Terminal data flows directly between the two peers — encrypted, low-latency, and with no relay server in the data path.

The only centralized element is signaling (exchanging connection metadata), which is handled via QR code or copy-paste to eliminate the need for any server infrastructure.

## 10.2 Architecture

```
Your machine                              Remote browser (opencoop.sh/connect)
┌────────────┐                           ┌──────────────────────┐
│ coop daemon │◄═══ WebRTC PeerConn ═════►│ Full React UI        │
│            │     (P2P, DTLS encrypted) │ ┌──────┬───────────┐ │
│ Session A  │     control channel ──────►│ │Sessi-│ xterm.js  │ │
│  PTY 0: 🤖 │     pty-0 channel ────────►│ │ons   │ (Agent)   │ │
│  PTY 1: sh │     pty-1 channel ────────►│ │      │───────────│ │
│            │                           │ │      │ xterm.js  │ │
│ Session B  │                           │ │      │ (Shell 1) │ │
│  PTY 0: 🤖 │                           │ └──────┴───────────┘ │
└────────────┘                           └──────────────────────┘
       │                                        │
       └──── SDP offer (QR/paste) ──────────────┘
              SDP answer (QR/paste) ◄───────────┘
```

## 10.3 Signaling

WebRTC requires a one-time exchange of SDP (Session Description Protocol) offers and answers to establish a connection. Coop uses out-of-band signaling with no signaling server.

### 10.3.1 Offer Generation

When the user runs `coop tunnel`:

1. Daemon creates a WebRTC PeerConnection
2. Generates an SDP offer containing:
   - ICE candidates (discovered via STUN)
   - DTLS fingerprint
   - DataChannel configuration
3. Wraps the SDP in a connection envelope with machine metadata:
   ```json
   {
     "sdp": "<sdp-offer>",
     "machine_id": "a1b2c3d4",
     "hostname": "bens-desktop",
     "version": 1
   }
   ```
   `machine_id` is a stable random ID generated once and stored in `~/.coop/machine_id`. `hostname` is the system hostname at tunnel creation time.
4. Compresses and base64-encodes the envelope
5. Generates a connection URL: `https://opencoop.sh/connect#<encoded-envelope>`
5. Displays in terminal:

```
  🐔 Coop tunnel ready

  Scan to connect:
  ██████████████
  ██          ██
  ██  ▄▄▄▄▄  ██
  ██          ██
  ██████████████

  Or open: https://opencoop.sh/connect#eyJzZHA...

  Waiting for peer...
```

### 10.3.2 Static Connect Page

`https://opencoop.sh` hosts the **full Coop web UI** — the same React application described in [Section 9.4](./09-web-ui.md#94-web-ui-design). It is a static site (hosted on GitHub Pages or similar) with zero backend.

#### `opencoop.sh/connect#<envelope>` — Connect to a Machine

When opened with a URL fragment:

1. Extract the connection envelope from the URL fragment (never sent to server)
2. Parse `machine_id` and `hostname` from the envelope
3. Create a WebRTC PeerConnection with the SDP offer
4. Generate an SDP answer (sent back over the DataChannel or displayed for manual exchange)
5. Open the control DataChannel and negotiate session info
6. Save machine to localStorage: `{machine_id, hostname, lastSeen, layout}`
7. Render the full UI: session sidebar, PTY tabs, split panels

If the `machine_id` already exists in localStorage, the app restores the saved layout (panel splits, active PTY tabs, theme) for that machine.

#### `opencoop.sh` — Machine Dashboard

When opened without a URL fragment, the app displays a dashboard of known machines from localStorage:

```
┌─────────────────────────────────┐
│ 🐔 Coop                        │
├─────────────────────────────────┤
│                                 │
│  Your machines:                 │
│                                 │
│  ● bens-desktop    (connected)  │
│  ○ work-server     (offline)    │
│  ○ pi-cluster      (offline)    │
│                                 │
│  [Scan QR to add machine]       │
│                                 │
└─────────────────────────────────┘
```

- **Connected** machines (active WebRTC PeerConnection) can be tapped to enter the full UI
- **Offline** machines show the last-seen timestamp; tapping shows "Scan a new QR code to reconnect"
- Machines can be removed from the dashboard (deletes from localStorage)

The URL fragment (`#...`) is never sent to the server — it's client-side only. The UI is identical to the `coop serve` web UI, but uses the WebRTC transport instead of HTTP/WebSocket (see [Section 9.6](./09-web-ui.md#96-shared-ui-architecture)).

> **Note:** `coop serve` also hosts the connect page at `/connect`, enabling fully offline use where the remote browser navigates to `http://<coop-ip>:8888/connect#<offer>` on LAN.

### 10.3.3 Answer Exchange

For most network configurations (non-symmetric NAT), the offer contains enough ICE candidates for the browser to connect directly. The answer is sent back over the DataChannel itself once established.

For restrictive networks where the browser cannot reach the offer's candidates:

1. The static page displays the SDP answer as a QR code or copyable string
2. The user scans/pastes it back to the `coop tunnel` terminal
3. The daemon applies the answer and completes the connection

This two-step exchange is only needed when direct connectivity fails.

### 10.3.4 Short Codes (OPTIONAL)

For convenience, the implementation MAY support short codes:

1. `coop tunnel` posts the compressed offer to a public paste service (e.g., a simple key-value store)
2. Returns a short code like `coop-abc123`
3. The connect page fetches the offer by short code
4. The paste expires after 5 minutes or first retrieval

This is OPTIONAL and requires a minimal external service. The QR/paste flow MUST always work without it.

## 10.4 NAT Traversal

### 10.4.1 STUN

The implementation MUST use STUN to discover the public IP and port mapping. Default STUN servers:

- `stun:stun.l.google.com:19302`
- `stun:stun1.l.google.com:19302`

The implementation SHOULD allow custom STUN servers via configuration.

### 10.4.2 TURN (Fallback)

For symmetric NAT and other restrictive network configurations where STUN fails, TURN provides a relay fallback. However:

- TURN requires a relay server (centralized)
- Coop does NOT bundle or require a TURN server
- The implementation MAY support user-configured TURN credentials

If NAT traversal fails entirely, `coop tunnel` MUST inform the user clearly and suggest alternatives (`coop serve` on LAN, or configuring port forwarding).

### 10.4.3 Expected Success Rates

| Network configuration | STUN success | Notes |
|----------------------|--------------|-------|
| Both on same LAN | 100% | Direct connection |
| One behind cone NAT | ~90% | Standard home router |
| One on mobile data | ~85% | Carrier NAT is usually cone |
| Both behind symmetric NAT | ~10% | Requires TURN |

## 10.5 DataChannel Configuration

A single WebRTC PeerConnection carries multiple DataChannels — one for control and one per attached PTY. All DataChannels share the DTLS encryption of the PeerConnection.

### 10.5.1 Control Channel

The control channel is created by the daemon immediately on connection:

- **Label**: `"coop-control"`
- **Ordered**: true
- **Reliable**: true

The control channel carries JSON messages that mirror the IPC command/response protocol (see [Section 7.4](./07-ipc.md#74-command-messages)):

**Browser → Daemon:**

```json
{"cmd": "ls"}
{"cmd": "shell", "session": "nlst", "cols": 120, "rows": 40}
{"cmd": "attach", "session": "nlst", "pty": 0, "cols": 120, "rows": 40}
{"cmd": "kill", "session": "nlst"}
```

**Daemon → Browser:**

Responses use the same format as [Section 7.5](./07-ipc.md#75-response-messages), including structured error codes.

The `attach` command over the control channel instructs the daemon to open a new PTY DataChannel (see below).

### 10.5.2 PTY Channels

Each PTY attachment gets its own DataChannel, created by the daemon in response to an `attach` command on the control channel:

- **Label**: `"coop-pty-<session>-<pty_id>"` (e.g., `"coop-pty-nlst-0"`)
- **Ordered**: true (terminal data must arrive in order)
- **Reliable**: true (no dropped keystrokes)

PTY channels carry tagged frames identical to IPC stream mode (see [Section 7.6](./07-ipc.md#76-stream-mode)):

```
┌──────────────┬──────────┬─────────────────────┐
│ Length (4 BE) │ Type (1) │ Payload             │
└──────────────┴──────────┴─────────────────────┘
```

- **Type `0x00`** — PTY data (raw terminal bytes, bidirectional)
- **Type `0x01`** — Control message (JSON — `resize`, `detach`, daemon events)

Each PTY channel is independent — a slow terminal does not block other PTYs. The input filter applies to all PTY channels on agent PTYs (PTY 0). See [Section 8.4](./08-pty.md#84-input-filtering).

### 10.5.3 Channel Lifecycle

1. Browser sends `{"cmd": "attach", "session": "nlst", "pty": 0, ...}` on the control channel
2. Daemon responds with `{"ok": true}` on the control channel
3. Daemon creates a new DataChannel labeled `"coop-pty-nlst-0"`
4. Browser receives the `ondatachannel` event, wires it to an xterm.js instance
5. Tagged frames flow bidirectionally on the PTY channel
6. To disconnect from a PTY: browser sends a `detach` control frame on the PTY channel, or closes the DataChannel directly

Multiple PTY channels can be open simultaneously (for split panel views). The browser manages which xterm.js instance maps to which DataChannel.

## 10.6 Session Binding

`coop tunnel` binds to the daemon, not to a specific session. The tunnel provides access to ALL sessions — the remote browser's UI includes the full session sidebar and can attach to any PTY in any session, same as the web UI.

```bash
coop tunnel              # open tunnel to daemon
```

> **Note:** A future enhancement MAY support `coop tunnel <session>` to restrict the tunnel to a single session for security. The initial implementation exposes all sessions.

## 10.7 Multiple Tunnels

Multiple tunnels MAY be active simultaneously:

- Multiple remote users can each have their own tunnel to the same daemon
- Each tunnel has its own WebRTC PeerConnection and independent DataChannels
- Tunnels are independent — one user's connection does not affect another's

## 10.8 Tunnel Lifecycle

A tunnel remains active until:

- The remote browser disconnects
- The user presses Ctrl+C on the `coop tunnel` CLI
- The underlying session is killed
- The daemon shuts down

Tunnels are ephemeral. They are NOT persisted or auto-reconnected. If the connection drops, the user runs `coop tunnel` again for a new QR code.

## 10.9 Security

WebRTC DataChannels are encrypted via DTLS by default. This provides:

- Confidentiality: terminal data is encrypted in transit
- Integrity: data cannot be tampered with
- Authentication: the DTLS fingerprint in the SDP offer/answer binds the connection to the intended peers

The SDP offer contains the DTLS fingerprint. Since the offer is transmitted via QR code (physical proximity) or manual copy-paste, the signaling channel has implicit authentication — an attacker would need to intercept and replace the QR code.

See [Section 12](./12-security.md) for full security analysis.
