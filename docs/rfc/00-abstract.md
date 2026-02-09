# Abstract

Coop is a single static Rust binary that provides sandboxed execution environments for AI coding agents (Claude Code, Codex, Aider, etc.) using Linux namespaces, with built-in session management and remote access. It combines three layers into one tool: container-level isolation without a container runtime, long-lived session management via an invisible auto-spawning daemon, and peer-to-peer remote access via WebRTC.

## Status of This Document

**Status:** Draft
**Version:** 0.1.0-draft
**Obsoletes:** None
**Updates:** None

This document specifies the architecture, protocols, and behavior of the Coop system for implementers and contributors. Distribution of this document is unlimited.

## License

MIT
