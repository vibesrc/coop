# Section 4: Coopfile Specification

## 4.1 Overview

A Coopfile is a TOML file named `coop.toml` that declares the environment for a coop session. It defines the base image, packages, agent command, environment variables, network mode, and session behavior.

## 4.2 Resolution Order

Coopfiles are resolved by layered merging, with later layers overriding earlier ones:

1. **Built-in defaults** — hardcoded sensible defaults
2. **Global config** — `~/.config/coop/default.toml`
3. **Project config** — `./coop.toml` (in the workspace directory)
4. **CLI flags** — runtime overrides

For map-type fields (`[env]`, `[packages]`), merging is additive — keys from both layers are combined, with later values overriding conflicts. For scalar fields, later values replace earlier ones entirely.

## 4.3 Full Schema

```toml
# ── Base Image ─────────────────────────────────────────────
[base]
# OCI image reference. Pulled and extracted during `coop init`.
# OPTIONAL. If omitted, uses a minimal Alpine base.
image = "node:22-alpine"

# ── Packages ───────────────────────────────────────────────
[packages]
# Package manager invocations run during `coop init`.
# Keys are package manager commands. Values are lists of packages.
# OPTIONAL. Multiple managers MAY be specified.
apk = ["git", "curl", "python3", "build-base"]
pip = ["numpy", "requests"]
npm = ["typescript"]

# ── Setup Commands ─────────────────────────────────────────
[setup]
# Arbitrary shell commands run during `coop init` after packages.
# Run in order, inside the rootfs, with network access.
# OPTIONAL.
run = [
    "npm install -g @anthropic-ai/claude-code",
    "mkdir -p /workspace",
]

# ── Agent Configuration ────────────────────────────────────
[agent]
# Command to run as the primary agent process.
# REQUIRED (no default — must be specified somewhere in the chain).
command = "claude"

# Arguments passed to the agent command.
# OPTIONAL. Default: []
args = []

# Install command run during `coop init` to install the agent.
# OPTIONAL. If the agent is already in the base image, omit this.
install = "npm install -g @anthropic-ai/claude-code"

# ── Workspace ──────────────────────────────────────────────
[workspace]
# Host directory to bind-mount. "." means the cwd when coop is invoked.
# OPTIONAL. Default: "."
mount = "."

# Mount point inside the session.
# OPTIONAL. Default: "/workspace"
path = "/workspace"

# ── Environment Variables ──────────────────────────────────
[env]
# Key-value pairs set inside the session.
# Values prefixed with "$" are expanded from the host environment.
# OPTIONAL.
EDITOR = "vim"
ANTHROPIC_API_KEY = "$ANTHROPIC_API_KEY"
TERM = "xterm-256color"

# ── Network ────────────────────────────────────────────────
[network]
# Network isolation mode.
#   "none"  — No network access (most secure)
#   "host"  — Share host network stack (easiest, least isolated)
#   "veth"  — Virtual ethernet pair (internet access, isolated from host LAN)
# OPTIONAL. Default: "veth"
mode = "veth"

# ── Session Behavior ───────────────────────────────────────
[session]
# Directories inside the session to persist between session restarts.
# Paths are relative to the user's home dir inside the session.
# OPTIONAL. Default: [".claude"]
persist = [".claude", ".config"]

# Automatically restart the agent if it exits.
# OPTIONAL. Default: true
auto_restart = true

# Delay before auto-restart (milliseconds).
# OPTIONAL. Default: 1000
restart_delay_ms = 1000

# ── Web/Remote Input Filtering ─────────────────────────────
[input_filter]
# Debounce interval for Ctrl+C on web-connected PTYs (milliseconds).
# A single Ctrl+C passes through. Rapid successive Ctrl+C within this
# window are suppressed.
# OPTIONAL. Default: 500
ctrl_c_debounce_ms = 500

# Exact byte sequences to block entirely on web-connected PTYs.
# These are in addition to the built-in blocked set (Ctrl+D, exit, /exit).
# OPTIONAL. Default: []
block_sequences = []
```

## 4.4 Minimal Coopfile

The smallest valid Coopfile for Claude Code:

```toml
[agent]
command = "claude"
install = "npm install -g @anthropic-ai/claude-code"

[base]
image = "node:22-alpine"
```

Everything else falls back to defaults.

## 4.5 Global Defaults Example

A typical `~/.config/coop/default.toml`:

```toml
[env]
ANTHROPIC_API_KEY = "$ANTHROPIC_API_KEY"
EDITOR = "vim"

[network]
mode = "veth"

[session]
persist = [".claude"]
auto_restart = true
```

This sets API keys and preferences once. Project-level Coopfiles only need to specify what's unique to that project.

## 4.6 Sharing Coopfiles

Coopfiles SHOULD be committed to project repositories. A team can share a single `coop.toml` that ensures everyone runs their agent in an identical environment.

Sensitive values MUST use the `$VARIABLE` expansion syntax rather than hardcoded secrets. Implementations MUST NOT write expanded secrets to disk or logs.

## 4.7 Validation

Implementations MUST validate the Coopfile at parse time and report clear errors for:

- Unknown keys (typos)
- Invalid types (string where list expected, etc.)
- Missing required fields (`[agent].command` if no global default provides it)
- Invalid `[network].mode` values
- Invalid `[base].image` format

Implementations SHOULD warn on:

- `$VARIABLE` references that are unset in the host environment
- `[packages]` keys that don't match a known package manager
