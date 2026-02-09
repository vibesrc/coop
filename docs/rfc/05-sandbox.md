# Section 5: Sandbox Layer

## 5.1 Overview

The sandbox layer provides container-level isolation using Linux kernel primitives directly — no container runtime involved. It creates an isolated filesystem, process tree, and optionally network stack for each session.

## 5.2 Namespaces

Each session is created with `clone()` or `unshare()` using the following namespace flags:

| Namespace | Flag | Purpose |
|-----------|------|---------|
| User | `CLONE_NEWUSER` | UID/GID mapping — process is "root" inside, unprivileged user on host |
| Mount | `CLONE_NEWNS` | Isolated mount tree — session has its own filesystem view |
| PID | `CLONE_NEWPID` | Isolated process IDs — agent is PID 1 inside |
| UTS | `CLONE_NEWUTS` | Isolated hostname — session gets its own hostname |
| Network | `CLONE_NEWNET` | Isolated network stack — OPTIONAL, depends on `[network].mode` |

### 5.2.1 User Namespace

The implementation MUST set up UID/GID mappings via `/proc/<pid>/uid_map` and `/proc/<pid>/gid_map` so that:

- The process runs as UID 0 (root) inside the namespace
- This maps to the invoking user's UID on the host
- No actual root privileges are gained on the host

This allows the agent to install packages and modify system files within the session without host privileges.

### 5.2.2 Network Namespace

Network isolation depends on the Coopfile `[network].mode`:

- **`none`**: `CLONE_NEWNET` is set, no veth pair created. The session has a loopback-only network stack.
- **`host`**: `CLONE_NEWNET` is NOT set. The session shares the host network. Simplest, least isolated.
- **`veth`**: `CLONE_NEWNET` is set. A veth pair connects the session to the host with NAT for internet access. The session is isolated from the host LAN but can reach the internet.

## 5.3 Overlayfs

The session filesystem is an overlay of a read-only base layer and a writable session-specific upper layer.

```
┌─────────────────────────────┐
│  Upper (session writes)      │  ~/.coop/sessions/<id>/upper/
│  tmpfs or temp dir           │  ephemeral per session
├─────────────────────────────┤
│  Base (built rootfs)         │  ~/.coop/rootfs/base/
│  read-only at runtime        │  shared across all sessions
└─────────────────────────────┘
```

### 5.3.1 Base Layer

The base layer is built during `coop init`:

1. If `[base].image` is specified, pull and extract the OCI image layers
2. Apply `[packages]` — run package manager commands inside a temporary namespace
3. Apply `[setup].run` — execute setup commands in order
4. Apply `[agent].install` — install the agent
5. Snapshot the result as the read-only base

The base layer MUST NOT be modified after `coop init`. Changes require `coop rebuild`.

### 5.3.2 Upper Layer

Each session gets its own upper directory. All writes inside the session go here. When the session ends, the upper layer is discarded unless `[session].persist` specifies directories to preserve.

### 5.3.3 Persistence

Directories listed in `[session].persist` are bind-mounted from `~/.coop/sessions/<id>/persist/<dir>` into the session. They survive session restarts and upper layer cleanup.

The default persist list MUST include `.claude` to preserve Claude Code session state.

## 5.4 Bind Mounts

The following mounts are set up inside the namespace:

| Type | Source (host) | Target (session) | Mode |
|------|---------------|-------------------|------|
| overlay | base + upper | `/` | rw (writes go to upper) |
| bind | workspace dir | `/workspace` | rw |
| bind | persist dirs | `/home/user/<dir>` | rw |
| proc | (new) | `/proc` | rw (new PID namespace) |
| tmpfs | (new) | `/tmp` | rw |
| devpts | (new) | `/dev/pts` | rw (for PTY allocation) |

After mounting, the implementation MUST call `pivot_root()` to switch the session's root to the overlay, then `exec()` the agent or shell command.

## 5.5 Session Startup Sequence

The full startup sequence for a new session:

1. Parse and merge Coopfile (global + project + CLI overrides)
2. Verify base rootfs exists (error if not — user must run `coop init`)
3. Create session directory under `~/.coop/sessions/<id>/`
4. `clone()` with namespace flags
5. Inside child: set up UID/GID mappings
6. Mount overlayfs (base + upper → `/`)
7. Bind-mount workspace into `/workspace`
8. Bind-mount persist directories
9. Mount `/proc`, `/tmp`, `/dev/pts`
10. Set up network (if `veth` mode)
11. `pivot_root()` into the overlay
12. Set environment variables from `[env]`
13. Set `COOP_SESSION`, `COOP_WORKSPACE`, `COOP_CREATED` env vars for discovery
14. Allocate PTY for the agent
15. `exec()` the agent command

Steps 4-15 complete in under 100ms (no image pulling, no package installation — that was done in `coop init`).

## 5.6 Kernel Requirements

The implementation REQUIRES Linux kernel 5.11 or later for:

- Unprivileged user namespaces
- Overlayfs in user namespaces (kernel 5.11+)
- `/proc/<pid>/uid_map` and `gid_map` writes from unprivileged users

> **Note:** Some distributions disable unprivileged user namespaces by default. Users may need to set `sysctl kernel.unprivileged_userns_clone=1`.
