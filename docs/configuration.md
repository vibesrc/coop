# Configuration Reference

Coop is configured via `coop.toml` in the project root. Global defaults can be set in `~/.config/coop/default.toml`.

## Full example

```toml
[sandbox]
image = "debian:latest"
agent = "claude"
shell = "bash"
user = "coop"
args = ["--dangerously-skip-permissions"]
setup = [
  "DEBIAN_FRONTEND=noninteractive apt-get update && apt-get install -y bash curl git ca-certificates",
  "curl -fsSL https://deb.nodesource.com/setup_22.x | bash - && apt-get install -y nodejs",
]
mounts = [
  "~/.bashrc:~/.bashrc",
  "~/.gitconfig:~/.gitconfig",
  "claude-config:~/.claude",
]

[workspace]
mount = "."
path = "/workspace"

[env]
ANTHROPIC_API_KEY = "$ANTHROPIC_API_KEY"
GITHUB_TOKEN = "$GITHUB_TOKEN"

[network]
mode = "host"

[session]
persist = [".claude"]
auto_restart = true
restart_delay_ms = 1000

[input_filter]
ctrl_c_debounce_ms = 500
block_sequences = []
```

## [sandbox]

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `image` | string | none | OCI image to use as base rootfs (e.g. `debian:latest`, `node:22-alpine`) |
| `agent` | string | none | Command for the agent process (PTY 0). Required. |
| `shell` | string | `"/bin/bash"` | Default command for `coop shell` |
| `user` | string | `"coop"` | Username inside the sandbox |
| `args` | string[] | `[]` | Arguments passed to the agent command |
| `setup` | string[] | `[]` | Shell commands run during `coop build` to set up the rootfs |
| `mounts` | mount[] | `[]` | Mounts into the sandbox (see below) |

### Mounts

Mounts can be specified as strings or tables:

```toml
# String form: "source:destination"
mounts = [
  "~/.bashrc:~/.bashrc",           # path-based: bind mount from host
  "claude-config:~/.claude",       # named: managed persistent volume
]

# Table form (equivalent)
[[sandbox.mounts]]
host = "~/.bashrc"
container = "~/.bashrc"
```

**Path-based mounts** (source starts with `/`, `~`, or `.`): bind-mounted directly from the host into the sandbox. Tilde (`~`) is expanded on both sides (host home on the left, sandbox home on the right).

**Named mounts** (source is a plain name like `claude-config`): use managed persistent storage in `~/.coop/volumes/<name>/`. On first use, the volume is seeded from the equivalent host path if it exists. Named volumes persist across box restarts and can be managed with `coop system volumes`.

## [workspace]

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `mount` | string | `"."` | Host directory to mount as workspace (relative to project root) |
| `path` | string | `"/workspace"` | Mount point inside the sandbox |

The workspace is bind-mounted read-write into the sandbox.

## [env]

Key-value pairs set as environment variables inside the sandbox. Values starting with `$` are expanded from the host environment:

```toml
[env]
ANTHROPIC_API_KEY = "$ANTHROPIC_API_KEY"   # reads from host env
MY_CUSTOM_VAR = "literal-value"            # literal string
```

## [network]

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `mode` | string | `"host"` | Network isolation mode |

Modes:
- `"host"` -- shared network namespace (agent can access the internet normally)
- `"none"` -- no network access (fully isolated)
- `"veth"` -- virtual ethernet pair (not yet implemented)

## [session]

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `persist` | string[] | `[".claude"]` | Directories inside the sandbox to persist across restarts (relative to sandbox home) |
| `auto_restart` | bool | `true` | Auto-restart the agent when it exits |
| `restart_delay_ms` | u64 | `1000` | Delay before restarting (ms) |

When `auto_restart` is enabled, connected clients see a `[process exited, restarting in 1000ms...]` message and then the new process output, without disconnecting.

## [input_filter]

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `ctrl_c_debounce_ms` | u64 | `500` | Minimum interval between Ctrl+C signals (prevents accidental double-interrupt) |
| `block_sequences` | string[] | `[]` | Byte sequences to block from reaching the PTY |

## Config resolution

Configs are merged in order (later overrides earlier):

1. Built-in defaults
2. Global: `~/.config/coop/default.toml`
3. Project: `./coop.toml`
4. CLI flags

For array fields (`setup`, `mounts`), overlay values are *appended* to the base. For scalar fields, overlay values *replace* the base.
