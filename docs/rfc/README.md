# Coop: Sandboxed AI Agent Sessions with Remote Access

**Version:** 0.1.0-draft
**Status:** Draft
**Last Updated:** 2026-02-09

## Abstract

Coop is a single static binary that sandboxes AI coding agents using Linux namespaces and provides remote access via an auto-spawning daemon, embedded web UI, and peer-to-peer WebRTC tunnels. No container runtime, no service manager, no external dependencies. Just syscalls.

## Table of Contents

1. [Abstract](./00-abstract.md)
2. [Introduction](./01-introduction.md)
3. [Terminology](./02-terminology.md)
4. [Architecture](./03-architecture.md)
5. [Coopfile Specification](./04-coopfile.md)
6. [Sandbox Layer](./05-sandbox.md)
7. [Daemon & Session Management](./06-daemon.md)
8. [IPC Protocol](./07-ipc.md)
9. [PTY Management & Input Filtering](./08-pty.md)
10. [Web UI & Local Serving](./09-web-ui.md)
11. [Tunnel & P2P Remote Access](./10-tunnel.md)
12. [CLI Reference](./11-cli.md)
13. [Security Considerations](./12-security.md)
14. [References](./13-references.md)

## Document Index

| Section | File | Description | Lines |
|---------|------|-------------|-------|
| Abstract | [00-abstract.md](./00-abstract.md) | Status and summary | ~30 |
| Introduction | [01-introduction.md](./01-introduction.md) | Motivation, scope, goals | ~120 |
| Terminology | [02-terminology.md](./02-terminology.md) | Definitions and conventions | ~80 |
| Architecture | [03-architecture.md](./03-architecture.md) | System overview, layers, data flow | ~200 |
| Coopfile | [04-coopfile.md](./04-coopfile.md) | Environment definition format | ~200 |
| Sandbox | [05-sandbox.md](./05-sandbox.md) | Namespaces, overlayfs, bind mounts | ~180 |
| Daemon | [06-daemon.md](./06-daemon.md) | Auto-spawn, lifecycle, session mgmt | ~250 |
| IPC Protocol | [07-ipc.md](./07-ipc.md) | Unix socket wire protocol | ~200 |
| PTY Management | [08-pty.md](./08-pty.md) | Multiplexing, input filtering | ~180 |
| Web UI | [09-web-ui.md](./09-web-ui.md) | Embedded server, xterm.js, mobile | ~150 |
| Tunnel | [10-tunnel.md](./10-tunnel.md) | WebRTC, signaling, QR codes | ~200 |
| CLI Reference | [11-cli.md](./11-cli.md) | Command reference | ~150 |
| Security | [12-security.md](./12-security.md) | Threat model, mitigations | ~150 |
| References | [13-references.md](./13-references.md) | Normative and informative refs | ~50 |

## Quick Navigation

- **Implementers**: Start with [Architecture](./03-architecture.md), then [Daemon](./06-daemon.md)
- **Users**: Start with [CLI Reference](./11-cli.md) and [Coopfile](./04-coopfile.md)
- **Security review**: See [Security Considerations](./12-security.md)
- **Protocol details**: See [IPC Protocol](./07-ipc.md) and [Tunnel](./10-tunnel.md)
