# Section 2: Terminology and Conventions

## 2.1 Requirements Language

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this document are to be interpreted as described in RFC 2119.

## 2.2 Definitions

**Agent**
: An interactive CLI program that acts as an AI coding assistant. Examples: Claude Code, Codex CLI, Aider. The agent is the primary process inside a session.

**Session**
: A running coop namespace containing one agent PTY and zero or more shell PTYs, all sharing the same filesystem view. Sessions are long-lived and survive terminal disconnects.

**Namespace**
: A Linux kernel isolation primitive. Coop uses user, mount, PID, UTS, and optionally network namespaces to isolate sessions from the host.

**Rootfs**
: The root filesystem inside a session, constructed from a base image layer (read-only) and a session-specific writable upper layer via overlayfs.

**Coopfile**
: A TOML configuration file (`coop.toml`) that declares the environment for a session — base image, packages, agent command, environment variables, and network mode.

**Daemon**
: A background process that owns all session namespaces and PTYs. Auto-spawned on first `coop` command, auto-exits when idle. The daemon is an implementation detail invisible to the user.

**PTY**
: A pseudo-terminal. Each agent and shell within a session runs in its own PTY. The daemon holds the master side; clients (local terminal, web UI, tunnel) connect to it.

**Workspace**
: The host directory bind-mounted into the session at `/workspace`. File changes are bidirectional and instant.

**Tunnel**
: A WebRTC DataChannel connection that bridges a remote browser to a session's PTY, enabling access from outside the local network without a relay server.

**Serve**
: Running a local HTTP/WebSocket server that exposes session PTYs via an embedded web UI on the local network.

## 2.3 Abbreviations

| Abbreviation | Expansion |
|--------------|-----------|
| OCI | Open Container Initiative |
| PTY | Pseudo-Terminal |
| IPC | Inter-Process Communication |
| UDS | Unix Domain Socket |
| SDP | Session Description Protocol (WebRTC) |
| STUN | Session Traversal Utilities for NAT |
| TURN | Traversal Using Relays around NAT |
| QoS | Quality of Service |
| LAN | Local Area Network |
| WAN | Wide Area Network |
