# Section 12: Security Considerations

## 12.1 Threat Model

### 12.1.1 Assets

- Host filesystem and credentials
- Agent API keys (e.g., `ANTHROPIC_API_KEY`)
- Workspace source code
- Terminal session content (may contain secrets in output)

### 12.1.2 Assumed Attackers

- **Malicious agent behavior**: The AI agent attempts to escape the sandbox, access host files outside the workspace, or exfiltrate data
- **Network attacker**: An attacker on the same network attempts to access the web UI or intercept tunnel traffic
- **Remote attacker**: An attacker on the internet attempts to connect to a tunnel or web server

### 12.1.3 Non-Threats

- Physical access to the machine (out of scope)
- Compromise of the host kernel (out of scope — namespaces rely on kernel integrity)
- Supply chain attacks on the OCI image (user responsibility)

## 12.2 Sandbox Security

### 12.2.1 Filesystem Isolation

The session can only see:

- The overlay rootfs (base + session upper)
- The bind-mounted workspace at `/workspace`
- Explicitly mounted persist directories

The host filesystem is NOT visible. Host credentials (SSH keys, `.env` files, cloud credentials) are NOT accessible unless explicitly bind-mounted.

The implementation MUST verify that bind mount paths are within the user's home directory and that no path traversal via symlinks is possible.

### 12.2.2 User Namespace

Processes run as UID 0 inside the namespace but map to the invoking user's unprivileged UID on the host. This means:

- The agent can install packages and modify files inside the namespace
- The agent CANNOT modify host files outside the bind mounts
- The agent CANNOT gain actual root privileges on the host

### 12.2.3 Network Isolation

In `veth` mode:

- The session can reach the internet (for package installation, API calls)
- The session CANNOT reach the host LAN (no direct access to other machines)
- NAT is applied — the session's traffic appears to come from the host

In `none` mode:

- The session has no network access at all (most secure for pure coding tasks)
- The agent cannot exfiltrate data

In `host` mode:

- No network isolation (convenient but least secure)
- SHOULD only be used when the agent needs LAN access

### 12.2.4 Process Isolation

PID namespace ensures the session cannot see or signal host processes. UTS namespace prevents hostname-based fingerprinting.

### 12.2.5 Known Limitations

- **Workspace is read-write**: The agent can modify or delete any file in the workspace. This is by design (it's a coding agent) but the user should be aware.
- **Overlayfs escape**: In older kernels, overlayfs in user namespaces had privilege escalation bugs. The spec requires kernel 5.11+ which addresses known issues.
- **`/proc` information leak**: Some `/proc` entries may leak host information even inside a PID namespace. The implementation SHOULD mount a restrictive `/proc` with appropriate hidepid settings.

## 12.3 Web UI Security

### 12.3.1 LAN Exposure

`coop serve` defaults to `127.0.0.1` (localhost only). Binding to non-localhost addresses requires an explicit `--host` flag (e.g., `--host 0.0.0.0`).

Token authentication is always active — the daemon auto-generates a random token on startup and embeds it in the QR code URL. Scanning the QR grants access; requests without a valid token are rejected. See [Section 9.5](./09-web-ui.md#95-authentication) for details.

### 12.3.2 No TLS by Default

`coop serve` uses plain HTTP. For LAN use this is acceptable. For any WAN exposure, `coop tunnel` (which uses DTLS) SHOULD be used instead.

The implementation MAY support `--tls` with auto-generated self-signed certificates for LAN HTTPS.

### 12.3.3 Input Filtering as Defense-in-Depth

Input filtering (see [Section 8.4](./08-pty.md#84-input-filtering)) is a convenience feature to prevent accidental agent termination. It is NOT a security boundary. A determined attacker with web UI access has full control of the terminal.

## 12.4 Tunnel Security

### 12.4.1 Encryption

WebRTC DataChannels use DTLS (Datagram Transport Layer Security). All terminal data in transit is encrypted and integrity-protected.

### 12.4.2 Authentication

The SDP offer contains a DTLS fingerprint. Since the offer is transmitted via:

- **QR code**: requires physical proximity to the terminal
- **Copy-paste**: requires access to the terminal output

An attacker would need to intercept AND replace the SDP offer to perform a man-in-the-middle attack. This is a reasonable security property for the use case.

### 12.4.3 STUN Privacy

STUN requests reveal the host's public IP to the STUN server. The implementation SHOULD document this. Users who require IP privacy MAY disable STUN and use only LAN mode.

### 12.4.4 Short Code Risk

If the OPTIONAL short code feature is used (see [Section 10.3.4](./10-tunnel.md#1034-short-codes-optional)), the SDP offer is temporarily stored on a third-party service. The offer contains:

- ICE candidates (IP addresses and ports)
- DTLS fingerprint

This is connection metadata, not session content. However, it reveals the host's network topology. Short codes MUST expire within 5 minutes and be single-use.

## 12.5 Daemon Security

### 12.5.1 Socket Permissions

The unix domain socket at `~/.coop/sock` MUST have permissions `0600`. Only the owning user can connect.

**Peer credential verification**: The daemon MUST verify the connecting client's UID via `SO_PEERCRED` (Linux) on each new connection. The implementation SHOULD use `UnixStream::peer_cred()` from the standard library. Connections from a different UID MUST be rejected.

**Atomic socket creation**: The daemon MUST create the socket safely:

1. If the socket path already exists, verify it is owned by the current UID before unlinking
2. Bind the new socket with `umask(0o177)` to enforce `0600`
3. If the path is a symlink, the daemon MUST abort with an error — never follow symlinks during socket creation

### 12.5.2 Environment Variable Exposure

Session environment variables (including API keys passed via `$VARIABLE` expansion) are:

- Visible in `/proc/<pid>/environ` — but only to the same UID
- NOT written to disk or logs
- NOT transmitted over the IPC protocol in cleartext (they're set directly in the namespace)

### 12.5.3 Log Sensitivity

Daemon logs at `~/.coop/logs/daemon.log` MUST NOT contain:

- Environment variable values
- Terminal session content
- API keys or tokens

Logs SHOULD contain only operational events (session created, client connected, errors).

## 12.6 Recommendations

1. Use `network = "none"` or `network = "veth"` — avoid `host` mode
2. Use `coop tunnel` instead of `coop serve` for remote access
3. Keep API keys in `~/.config/coop/default.toml` with `$VARIABLE` expansion, not in project Coopfiles
4. Review workspace contents before granting agent access
5. Use `coop kill` to clean up sessions when done
