# Section 1: Introduction

## 1.1 Background

AI coding agents like Claude Code, Codex, and Aider run as interactive CLI processes that read and write files, execute shell commands, and interact with the user via a terminal. Developers need to:

- **Sandbox** these agents so they can't damage the host system while still having read-write access to the project workspace.
- **Detach** agent sessions so they survive terminal disconnects, laptop lid closures, and SSH drops.
- **Access remotely** from a phone, tablet, or another machine — with a proper UI, not a terminal emulator on a phone screen.
- **Manage multiple sessions** across different projects simultaneously.

Existing solutions either wrap Docker/Podman (adding heavyweight dependencies), require SSH tunneling (poor mobile UX), or are web-only without local terminal integration.

Coop solves all of these with a single static binary and zero external dependencies.

## 1.2 Scope

This specification defines:

- The Coopfile format for declaring sandbox environments
- The sandbox layer (Linux namespaces, overlayfs, bind mounts)
- The auto-spawning daemon and session lifecycle
- The IPC protocol between CLI client and daemon (unix socket)
- PTY management, multiplexing, and input filtering
- The embedded web UI for local network access
- The WebRTC-based P2P tunnel for remote access
- The CLI command surface

This specification does NOT define:

- Agent-specific behavior (Claude Code commands, Codex workflows, etc.)
- OCI image registry authentication flows (deferred to implementation)
- Firewall or router configuration for NAT traversal
- IDE/editor integrations

## 1.3 Goals

The system MUST:

1. Provide container-level filesystem and process isolation using only Linux kernel primitives
2. Require zero external runtime dependencies (no Docker, Podman, systemd, tmux, node, etc.)
3. Ship as a single static binary under 10MB
4. Start sessions in under 100ms after initial rootfs build
5. Allow local terminal attach and web-based remote attach to the same session simultaneously
6. Auto-spawn and auto-shutdown the daemon without user intervention
7. Support multiple concurrent sessions across different project workspaces
8. Support multiple PTYs per session (agent + shells)
9. Filter dangerous input on web-connected PTYs to prevent accidental agent termination
10. Work identically on Ubuntu, Arch, Alpine, and WSL

The system SHOULD:

1. Support peer-to-peer remote access without any central relay server
2. Provide a mobile-friendly web UI
3. Allow environment definitions to be shared and composed
4. Support any interactive CLI agent, not just Claude Code

## 1.4 Document Organization

- [Section 2](./02-terminology.md) defines terminology and conventions
- [Section 3](./03-architecture.md) describes the system architecture and data flow
- [Section 4](./04-coopfile.md) specifies the Coopfile format
- [Section 5](./05-sandbox.md) details the sandbox isolation layer
- [Section 6](./06-daemon.md) covers the daemon lifecycle and session management
- [Section 7](./07-ipc.md) defines the unix socket IPC protocol
- [Section 8](./08-pty.md) covers PTY management and input filtering
- [Section 9](./09-web-ui.md) describes the embedded web UI
- [Section 10](./10-tunnel.md) specifies the WebRTC tunnel
- [Section 11](./11-cli.md) provides the CLI command reference
- [Section 12](./12-security.md) addresses security considerations
- [Section 13](./13-references.md) lists references
