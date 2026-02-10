# Troubleshooting

## "Rootfs not found. Run `coop init` first."

The base rootfs hasn't been built yet. Run:

```bash
coop build
```

If you don't have a `coop.toml`, create one first:

```bash
coop init
coop build
```

## Shell opens as root on host filesystem

This was a race condition bug (fixed). If you see `root@hostname:/#` instead of your sandbox prompt:

1. Kill the stale session: `coop kill`
2. Shut down the daemon: `coop system shutdown`
3. Rebuild: `cargo build --release`
4. Try again: `coop shell`

The fix ensures the parent process waits for the child to complete filesystem setup (overlayfs + pivot_root) before returning.

## "Session not found" errors

Session lookup accepts both the box name and the workspace path. If you get "not found" errors:

```bash
# Check what sessions are running
coop ls

# Kill by name
coop kill my-project

# Or kill all
coop kill --all
```

## Daemon won't start

Check for stale socket/pid files:

```bash
ls -la ~/.coop/daemon.sock ~/.coop/daemon.pid
```

Remove them if the daemon isn't actually running:

```bash
rm -f ~/.coop/daemon.sock ~/.coop/daemon.pid
```

## "unshare failed: Operation not permitted"

Your kernel doesn't allow unprivileged user namespaces. Fix:

```bash
# Check the current setting
sysctl kernel.unprivileged_userns_clone

# Enable it (requires root)
sudo sysctl -w kernel.unprivileged_userns_clone=1

# Make it permanent
echo 'kernel.unprivileged_userns_clone=1' | sudo tee /etc/sysctl.d/99-userns.conf
```

On some distros (Debian/Ubuntu), you may also need:

```bash
sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0
```

## "newuidmap: ... not found" or UID mapping errors

Install `uidmap` and configure subordinate ranges:

```bash
sudo apt-get install uidmap
sudo usermod --add-subuids 100000-165535 --add-subgids 100000-165535 $USER
```

Verify:

```bash
grep $USER /etc/subuid  # should show something like: user:100000:65536
grep $USER /etc/subgid
```

Without subordinate ranges, only UID 0 is mapped inside the namespace (single-user fallback). This means tools like `apt-get` that create system users will fail.

## Agent keeps restarting

If the agent command doesn't exist inside the sandbox, it will fail immediately and auto-restart in a loop. Check:

1. Is the agent binary available? Coop auto-mounts it from the host if found in `$PATH`.
2. Check the agent logs: `coop logs -n 20`
3. Open a shell to debug: `coop shell`
4. Disable auto-restart temporarily by setting `auto_restart = false` in `coop.toml`

## Ctrl+D doesn't return to host

Make sure you're in a shell session (not the agent). The agent has `auto_restart` enabled by default, so exiting it restarts it rather than returning to host.

For shells, `Ctrl+D` exits the shell process. The exit watcher detects this, cleans up the PTY, and the client disconnects back to the host terminal.

Use `Ctrl+]` to detach from any session without exiting the process.

## Disk space

Check what's using space:

```bash
coop system df
```

Clean up:

```bash
coop system clean          # remove rootfs (re-built on next run)
coop system clean --all    # also remove OCI cache
coop system volume-prune   # remove all named volumes
coop system prune          # remove everything
```
