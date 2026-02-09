use std::collections::HashMap;
use std::ffi::CString;
use std::os::unix::io::{FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use nix::sched::CloneFlags;
use nix::unistd::{ForkResult, Pid};

use crate::config::{self, Coopfile, NetworkMode};

/// Result of creating a sandboxed session
pub struct SessionNamespace {
    /// PID of the init process inside the namespace (as seen from host)
    pub child_pid: u32,
    /// Master side of the PTY allocated for the agent
    pub pty_master_fd: RawFd,
    /// Session name
    pub name: String,
}

/// Information about a discovered session from /proc scanning
#[derive(Debug, Clone)]
pub struct DiscoveredSession {
    pub name: String,
    pub workspace: String,
    pub created: u64,
    pub pid: u32,
}

/// Namespace flags for session isolation
pub fn namespace_flags(network_mode: NetworkMode) -> CloneFlags {
    // Note: CLONE_NEWPID is omitted because unshare() puts only children
    // in the new PID namespace (not the caller), which prevents the shell
    // from forking. A double-fork after unshare would fix this but adds
    // complexity. For now, user+mount+uts provides sufficient isolation.
    let mut flags = CloneFlags::CLONE_NEWUSER
        | CloneFlags::CLONE_NEWNS
        | CloneFlags::CLONE_NEWUTS;

    if network_mode != NetworkMode::Host {
        flags |= CloneFlags::CLONE_NEWNET;
    }

    flags
}

/// Create a fully isolated session namespace.
///
/// This forks the process, sets up user/mount/pid/uts/net namespaces,
/// mounts overlayfs, bind mounts workspace and persist dirs, does pivot_root,
/// then forkpty + exec the agent command inside.
///
/// Returns the child PID (as seen from host) and the PTY master fd.
pub fn create_session(
    name: &str,
    config: &Coopfile,
    workspace_host: &Path,
) -> Result<SessionNamespace> {
    let base_path = config::rootfs_base_path()?;
    if !base_path.exists() {
        bail!(
            "Rootfs not found at {}. Run `coop init` first.",
            base_path.display()
        );
    }

    // Create session directories
    let session_dir = config::session_dir(name)?;
    let upper_path = session_dir.join("upper");
    let work_path = session_dir.join("work");
    let persist_path = session_dir.join("persist");
    let merge_path = session_dir.join("merged");

    std::fs::create_dir_all(&upper_path)?;
    std::fs::create_dir_all(&work_path)?;
    std::fs::create_dir_all(&persist_path)?;
    std::fs::create_dir_all(&merge_path)?;

    // Two pipes for parent-child synchronization:
    // Pipe 1 (child→parent): child signals after unshare(), parent then writes UID/GID maps
    // Pipe 2 (parent→child): parent signals after writing maps, child then proceeds
    let (pipe1_rd_owned, pipe1_wr_owned) = nix::unistd::pipe().context("Failed to create sync pipe 1")?;
    let pipe1_rd = pipe1_rd_owned.into_raw_fd(); // parent reads
    let pipe1_wr = pipe1_wr_owned.into_raw_fd(); // child writes
    let (pipe2_rd_owned, pipe2_wr_owned) = nix::unistd::pipe().context("Failed to create sync pipe 2")?;
    let pipe2_rd = pipe2_rd_owned.into_raw_fd(); // child reads
    let pipe2_wr = pipe2_wr_owned.into_raw_fd(); // parent writes

    // Resolve the agent command before forking
    let agent_cmd = config
        .sandbox
        .command
        .as_deref()
        .unwrap_or("claude");
    let agent_args = &config.sandbox.args;
    let workspace_path = &config.workspace.path;
    let persist_dirs = &config.session.persist;
    let user_env = &config.env;
    let network_mode = config.network.mode;
    let ns_flags = namespace_flags(network_mode);

    // Allocate PTY before forking so the parent gets the master fd
    let pty = nix::pty::openpty(None, None).context("Failed to allocate PTY")?;
    let master_fd = pty.master.into_raw_fd();
    let slave_fd = pty.slave.into_raw_fd();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Store paths as owned values before the fork
    let base_path_owned = base_path.clone();
    let upper_path_owned = upper_path.clone();
    let work_path_owned = work_path.clone();
    let merge_path_owned = merge_path.clone();
    let persist_path_owned = persist_path.clone();
    let workspace_host_owned = workspace_host.to_path_buf();
    let workspace_path_owned = workspace_path.clone();
    let persist_dirs_owned = persist_dirs.clone();
    // Resolve sandbox user and home path
    let sandbox_user = &config.sandbox.user;
    let sandbox_home = format!("/home/{}", sandbox_user);

    // Resolve extra mounts (host_path, container_path) before fork
    let mut extra_mounts: Vec<(PathBuf, String)> = config
        .sandbox
        .mounts
        .iter()
        .filter_map(|m| match m.resolve_with_home(&sandbox_home) {
            Ok(pair) => Some(pair),
            Err(e) => {
                eprintln!("coop: warning: skipping invalid mount: {}", e);
                None
            }
        })
        .collect();

    // Auto-mount the agent command binary into the sandbox if it exists on the host
    if let Some(cmd_name) = &config.sandbox.command {
        if let Ok(host_path) = resolve_host_binary(cmd_name) {
            let container_path = format!("/usr/local/bin/{}", cmd_name);
            extra_mounts.push((host_path, container_path));
        }
    }
    let sandbox_user_owned = sandbox_user.clone();
    let sandbox_home_owned = sandbox_home.clone();
    let user_env_owned = user_env.clone();
    let name_owned = name.to_string();
    let agent_cmd_owned = agent_cmd.to_string();
    let agent_args_owned = agent_args.clone();

    // Fork: child becomes the namespace init process
    match unsafe { nix::unistd::fork() }.context("fork() failed")? {
        ForkResult::Parent { child } => {
            // Close slave fd and child-side pipe ends in parent
            unsafe { nix::libc::close(slave_fd) };
            unsafe { nix::libc::close(pipe1_wr) };
            unsafe { nix::libc::close(pipe2_rd) };

            // Wait for child to unshare() before writing UID/GID maps
            let mut buf = [0u8; 1];
            let _ = nix::unistd::read(pipe1_rd, &mut buf);
            unsafe { nix::libc::close(pipe1_rd) };

            // Write UID/GID mappings for the child's user namespace
            setup_uid_map(child)?;

            // Signal child that maps are ready
            let wr_fd = unsafe { OwnedFd::from_raw_fd(pipe2_wr) };
            nix::unistd::write(&wr_fd, &[1u8])
                .context("Failed to signal child")?;
            drop(wr_fd);

            Ok(SessionNamespace {
                child_pid: child.as_raw() as u32,
                pty_master_fd: master_fd,
                name: name.to_string(),
            })
        }
        ForkResult::Child => {
            // Close master fd and parent-side pipe ends in child
            unsafe { nix::libc::close(master_fd) };
            unsafe { nix::libc::close(pipe1_rd) };
            unsafe { nix::libc::close(pipe2_wr) };

            // Unshare namespaces (this is the fork+unshare approach)
            if let Err(e) = nix::sched::unshare(ns_flags) {
                eprintln!("coop: unshare failed: {}", e);
                std::process::exit(1);
            }

            // Signal parent that unshare() is done so it can write UID/GID maps
            let wr_fd = unsafe { OwnedFd::from_raw_fd(pipe1_wr) };
            let _ = nix::unistd::write(&wr_fd, &[1u8]);
            drop(wr_fd);

            // Wait for parent to write UID/GID maps
            let mut buf = [0u8; 1];
            let _ = nix::unistd::read(pipe2_rd, &mut buf);
            unsafe { nix::libc::close(pipe2_rd); }

            // Redirect child stderr to a log file for debugging
            // (daemon stderr goes to /dev/null so child errors are otherwise lost)
            if let Ok(log_path) = crate::config::coop_dir() {
                let log_file = log_path.join("child-debug.log");
                if let Ok(f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log_file)
                {
                    use std::os::unix::io::IntoRawFd;
                    let fd = f.into_raw_fd();
                    unsafe { nix::libc::dup2(fd, 2); }
                    if fd > 2 { unsafe { nix::libc::close(fd); } }
                }
            }

            // Now we are "root" inside the user namespace.
            // Set up the filesystem.
            if let Err(e) = child_setup_fs(
                &base_path_owned,
                &upper_path_owned,
                &work_path_owned,
                &merge_path_owned,
                &workspace_host_owned,
                &workspace_path_owned,
                &persist_dirs_owned,
                &persist_path_owned,
                &extra_mounts,
                &sandbox_user_owned,
                &sandbox_home_owned,
            ) {
                eprintln!("coop: filesystem setup failed: {:?}", e);
                std::process::exit(1);
            }

            // Set hostname
            if let Err(e) = nix::unistd::sethostname(&name_owned) {
                eprintln!("coop: sethostname failed: {}", e);
                // Non-fatal
            }

            // Set environment variables
            std::env::set_var("COOP_SESSION", &name_owned);
            std::env::set_var("COOP_WORKSPACE", &workspace_host_owned);
            std::env::set_var("COOP_CREATED", now.to_string());
            std::env::set_var("HOME", &sandbox_home_owned);
            std::env::set_var("USER", &sandbox_user_owned);
            std::env::set_var("PATH", format!(
                "{}/.claude/local/bin:{}/.local/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
                sandbox_home_owned, sandbox_home_owned
            ));
            std::env::set_var("TERM", "xterm-256color");

            for (k, v) in &user_env_owned {
                std::env::set_var(k, v);
            }

            // Redirect stdin/stdout/stderr to the slave PTY
            unsafe {
                nix::libc::setsid();
                nix::libc::ioctl(slave_fd, nix::libc::TIOCSCTTY as _, 0);
                nix::libc::dup2(slave_fd, 0);
                nix::libc::dup2(slave_fd, 1);
                nix::libc::dup2(slave_fd, 2);
                if slave_fd > 2 {
                    nix::libc::close(slave_fd);
                }
            }

            // Set working directory to workspace inside the namespace
            let _ = std::env::set_current_dir(&workspace_path_owned);

            // Exec the agent command
            let cmd = CString::new(agent_cmd_owned.as_str())
                .unwrap_or_else(|_| CString::new("/bin/sh").unwrap());

            let mut argv: Vec<CString> = vec![cmd.clone()];
            for arg in &agent_args_owned {
                if let Ok(a) = CString::new(arg.as_str()) {
                    argv.push(a);
                }
            }

            // Collect environment for exec
            let env: Vec<CString> = std::env::vars()
                .filter_map(|(k, v)| CString::new(format!("{}={}", k, v)).ok())
                .collect();

            // execvpe
            let _ = nix::unistd::execvpe(&cmd, &argv, &env);

            // If exec fails, try /bin/sh as fallback
            let sh = CString::new("/bin/sh").unwrap();
            let sh_args = [sh.clone(), CString::new("-c").unwrap(), cmd];
            let _ = nix::unistd::execvpe(&sh, &sh_args, &env);

            eprintln!("coop: exec failed");
            std::process::exit(1);
        }
    }
}

/// Set up UID/GID mappings for the user namespace.
///
/// Tries newuidmap/newgidmap first (maps full subordinate range so all uids/gids
/// exist inside the namespace — needed for Debian/Ubuntu package managers).
/// Falls back to writing /proc/<pid>/uid_map directly (single uid 0 only).
pub fn setup_uid_map(child_pid: Pid) -> Result<()> {
    let uid = nix::unistd::getuid();
    let gid = nix::unistd::getgid();
    let pid = child_pid.as_raw();

    // Try newuidmap/newgidmap for full subordinate range
    if let (Ok(sub_uid), Ok(sub_gid)) = (get_subid("/etc/subuid"), get_subid("/etc/subgid")) {
        // newuidmap <pid> 0 <real_uid> 1 1 <sub_start> <sub_count>
        let uid_status = std::process::Command::new("newuidmap")
            .args([
                &pid.to_string(),
                "0", &uid.to_string(), "1",
                "1", &sub_uid.0.to_string(), &sub_uid.1.to_string(),
            ])
            .status();

        let gid_status = std::process::Command::new("newgidmap")
            .args([
                &pid.to_string(),
                "0", &gid.to_string(), "1",
                "1", &sub_gid.0.to_string(), &sub_gid.1.to_string(),
            ])
            .status();

        if matches!(uid_status, Ok(s) if s.success()) && matches!(gid_status, Ok(s) if s.success())
        {
            return Ok(());
        }

        eprintln!("coop: newuidmap/newgidmap failed, falling back to single-uid mapping");
    }

    // Fallback: single uid/gid mapping (only uid 0 exists inside)
    std::fs::write(format!("/proc/{}/setgroups", pid), "deny")
        .context("Failed to write setgroups")?;

    let uid_map = format!("0 {} 1\n", uid);
    std::fs::write(format!("/proc/{}/uid_map", pid), &uid_map)
        .context("Failed to write uid_map")?;

    let gid_map = format!("0 {} 1\n", gid);
    std::fs::write(format!("/proc/{}/gid_map", pid), &gid_map)
        .context("Failed to write gid_map")?;

    Ok(())
}

/// Parse /etc/subuid or /etc/subgid for the current user.
/// Returns (start, count) of the subordinate range.
fn get_subid(path: &str) -> Result<(u64, u64)> {
    let username = std::env::var("USER").unwrap_or_default();
    let uid_str = nix::unistd::getuid().to_string();

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path))?;

    for line in content.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() >= 3 && (parts[0] == username || parts[0] == uid_str) {
            let start: u64 = parts[1].parse().context("Invalid subid start")?;
            let count: u64 = parts[2].parse().context("Invalid subid count")?;
            return Ok((start, count));
        }
    }

    bail!("No subordinate ID range found for user in {}", path)
}

/// Child-side filesystem setup: overlay, bind mounts, pivot_root
/// Falls back to bind-mount + chroot when overlayfs is not available (e.g. WSL2 user namespaces).
fn child_setup_fs(
    base_path: &Path,
    upper_path: &Path,
    work_path: &Path,
    merge_path: &Path,
    workspace_host: &Path,
    workspace_path: &str,
    persist_dirs: &[String],
    persist_path: &Path,
    extra_mounts: &[(PathBuf, String)],
    sandbox_user: &str,
    sandbox_home: &str,
) -> Result<()> {
    // Make our mount namespace fully private so mounts don't propagate to the host
    nix::mount::mount(
        None::<&str>,
        "/",
        None::<&str>,
        nix::mount::MsFlags::MS_REC | nix::mount::MsFlags::MS_PRIVATE,
        None::<&str>,
    )
    .context("Failed to make mount namespace private")?;

    // Try overlayfs first; fall back to bind-mount if it fails
    let root = if setup_overlay(base_path, upper_path, work_path, merge_path).is_ok() {
        merge_path.to_path_buf()
    } else {
        eprintln!("coop: overlayfs failed, falling back to bind-mount rootfs");
        // Bind-mount the base rootfs to merge_path so we have a consistent root
        std::fs::create_dir_all(merge_path)?;

        // Make the base path a mount point first (bind to itself) so pivot_root works
        nix::mount::mount(
            Some(base_path),
            merge_path,
            None::<&str>,
            nix::mount::MsFlags::MS_BIND | nix::mount::MsFlags::MS_REC,
            None::<&str>,
        )
        .context("Failed to bind-mount rootfs base")?;

        merge_path.to_path_buf()
    };

    // Set up bind mounts inside the root
    setup_bind_mounts(&root, workspace_host, workspace_path, persist_dirs, persist_path, extra_mounts, sandbox_home)?;

    // Set up the sandbox user (uid 0 mapped to host user, named per config)
    setup_sandbox_user(&root, sandbox_user, sandbox_home)?;

    // Create /dev/null, /dev/zero, /dev/random, /dev/urandom symlinks/nodes
    setup_dev_nodes(&root)?;

    // Pivot root
    pivot_root(&root)?;

    Ok(())
}

/// Set up the overlayfs mount for a session.
pub fn setup_overlay(
    base_path: &Path,
    upper_path: &Path,
    work_path: &Path,
    mount_point: &Path,
) -> Result<()> {
    // Ensure directories exist
    std::fs::create_dir_all(upper_path)?;
    std::fs::create_dir_all(work_path)?;
    std::fs::create_dir_all(mount_point)?;

    let options = format!(
        "lowerdir={},upperdir={},workdir={}",
        base_path.display(),
        upper_path.display(),
        work_path.display()
    );

    nix::mount::mount(
        Some("overlay"),
        mount_point,
        Some("overlay"),
        nix::mount::MsFlags::empty(),
        Some(options.as_str()),
    )
    .context("Failed to mount overlayfs")?;

    Ok(())
}

/// Set up bind mounts inside the namespace
pub fn setup_bind_mounts(
    root: &Path,
    workspace_host: &Path,
    workspace_path: &str,
    persist_dirs: &[String],
    session_persist_path: &Path,
    extra_mounts: &[(PathBuf, String)],
    sandbox_home: &str,
) -> Result<()> {
    // Bind-mount workspace
    let ws_target = root.join(workspace_path.trim_start_matches('/'));
    std::fs::create_dir_all(&ws_target)?;

    nix::mount::mount(
        Some(workspace_host),
        &ws_target,
        None::<&str>,
        nix::mount::MsFlags::MS_BIND,
        None::<&str>,
    )
    .context("Failed to bind-mount workspace")?;

    // Mount /proc (requires PID namespace; non-fatal if it fails since we
    // may still be in the parent's PID namespace after unshare+no-fork)
    let proc_path = root.join("proc");
    std::fs::create_dir_all(&proc_path)?;
    if let Err(e) = nix::mount::mount(
        Some("proc"),
        &proc_path,
        Some("proc"),
        nix::mount::MsFlags::empty(),
        None::<&str>,
    ) {
        // Try bind-mounting the host /proc instead
        eprintln!("coop: mounting new /proc failed ({}), bind-mounting host /proc", e);
        let _ = nix::mount::mount(
            Some("/proc"),
            &proc_path,
            None::<&str>,
            nix::mount::MsFlags::MS_BIND | nix::mount::MsFlags::MS_REC,
            None::<&str>,
        );
    }

    // Mount /tmp as tmpfs
    let tmp_path = root.join("tmp");
    std::fs::create_dir_all(&tmp_path)?;
    nix::mount::mount(
        Some("tmpfs"),
        &tmp_path,
        Some("tmpfs"),
        nix::mount::MsFlags::empty(),
        None::<&str>,
    )
    .context("Failed to mount /tmp")?;

    // Mount /dev as tmpfs, then populate
    let dev_path = root.join("dev");
    std::fs::create_dir_all(&dev_path)?;
    nix::mount::mount(
        Some("tmpfs"),
        &dev_path,
        Some("tmpfs"),
        nix::mount::MsFlags::empty(),
        Some("mode=0755"),
    )
    .context("Failed to mount /dev")?;

    // Mount /dev/pts for PTY allocation
    let devpts_path = root.join("dev/pts");
    std::fs::create_dir_all(&devpts_path)?;
    nix::mount::mount(
        Some("devpts"),
        &devpts_path,
        Some("devpts"),
        nix::mount::MsFlags::empty(),
        Some("newinstance,ptmxmode=0666"),
    )
    .context("Failed to mount /dev/pts")?;

    // Create /dev/ptmx symlink
    let ptmx_link = root.join("dev/ptmx");
    let _ = std::os::unix::fs::symlink("pts/ptmx", &ptmx_link);

    // Extra bind mounts from coop.toml [[mounts]]
    for (host_path, container_path) in extra_mounts {
        if !host_path.exists() {
            eprintln!("coop: warning: mount source does not exist, skipping: {}", host_path.display());
            continue;
        }
        let target = root.join(container_path.trim_start_matches('/'));
        if host_path.is_dir() {
            std::fs::create_dir_all(&target)?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&target, b"")?;
        }
        nix::mount::mount(
            Some(host_path.as_path()),
            &target,
            None::<&str>,
            nix::mount::MsFlags::MS_BIND | nix::mount::MsFlags::MS_REC,
            None::<&str>,
        )
        .with_context(|| format!("Failed to bind-mount {} -> {}", host_path.display(), container_path))?;
    }

    // Bind-mount persist directories (skip if an explicit mount already covers the path)
    for dir in persist_dirs {
        let home_rel = sandbox_home.trim_start_matches('/');
        let target_path = root.join(home_rel).join(dir);
        let target_str = format!("{}/{}", sandbox_home, dir);

        // Skip if an explicit extra mount already targets this path
        if extra_mounts.iter().any(|(_, cp)| cp.trim_end_matches('/') == target_str.trim_end_matches('/')) {
            continue;
        }

        let host_path = session_persist_path.join(dir);
        std::fs::create_dir_all(&host_path)?;
        std::fs::create_dir_all(&target_path)?;

        nix::mount::mount(
            Some(&host_path),
            &target_path,
            None::<&str>,
            nix::mount::MsFlags::MS_BIND,
            None::<&str>,
        )
        .with_context(|| format!("Failed to bind-mount persist dir: {}", dir))?;
    }

    Ok(())
}

/// Set up the sandbox user inside the namespace.
/// Since we're in a user namespace with uid 0 mapped to the host user,
/// we write /etc/passwd and /etc/group to name uid 0 as the configured user.
fn setup_sandbox_user(root: &Path, user: &str, home: &str) -> Result<()> {
    let etc = root.join("etc");
    std::fs::create_dir_all(&etc)?;

    let home_dir = root.join(home.trim_start_matches('/'));
    std::fs::create_dir_all(&home_dir)?;

    std::fs::write(
        etc.join("passwd"),
        format!("{user}:x:0:0:{user}:{home}:/bin/sh\nnobody:x:65534:65534:nobody:/:/sbin/nologin\n"),
    )?;

    std::fs::write(
        etc.join("group"),
        format!("{user}:x:0:\nnogroup:x:65534:\n"),
    )?;

    std::fs::write(etc.join("shadow"), format!("{user}:!:0::::::\n"))?;

    Ok(())
}

/// Create basic device nodes in /dev
fn setup_dev_nodes(root: &Path) -> Result<()> {
    let dev = root.join("dev");
    std::fs::create_dir_all(&dev)?;

    // Symlinks to /proc/self/fd for /dev/stdin, /dev/stdout, /dev/stderr, /dev/fd
    let _ = std::os::unix::fs::symlink("/proc/self/fd", dev.join("fd"));
    let _ = std::os::unix::fs::symlink("/proc/self/fd/0", dev.join("stdin"));
    let _ = std::os::unix::fs::symlink("/proc/self/fd/1", dev.join("stdout"));
    let _ = std::os::unix::fs::symlink("/proc/self/fd/2", dev.join("stderr"));

    // Create /dev/null, /dev/zero, /dev/random, /dev/urandom as bind mounts from host
    // These will fail if the host doesn't have them, which is fine for most cases
    for name in &["null", "zero", "random", "urandom"] {
        let host_dev = PathBuf::from(format!("/dev/{}", name));
        let target = dev.join(name);
        if host_dev.exists() {
            // Create the target file for bind mount
            let _ = std::fs::write(&target, "");
            let _ = nix::mount::mount(
                Some(&host_dev),
                &target,
                None::<&str>,
                nix::mount::MsFlags::MS_BIND,
                None::<&str>,
            );
        }
    }

    Ok(())
}

/// Perform pivot_root to switch the session's root to the overlay
pub fn pivot_root(new_root: &Path) -> Result<()> {
    let old_root = new_root.join("old_root");
    std::fs::create_dir_all(&old_root)?;

    nix::unistd::pivot_root(new_root, &old_root).context("pivot_root failed")?;

    // Change to new root
    std::env::set_current_dir("/").context("Failed to chdir to /")?;

    // Unmount old root
    nix::mount::umount2("/old_root", nix::mount::MntFlags::MNT_DETACH)
        .context("Failed to unmount old root")?;

    std::fs::remove_dir("/old_root").ok();

    Ok(())
}

/// Kill a session by sending SIGTERM to its namespace init process,
/// then SIGKILL after a timeout.
pub fn kill_session(pid: u32, force: bool) -> Result<()> {
    let pid = Pid::from_raw(pid as i32);

    if force {
        nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL)
            .context("Failed to SIGKILL session")?;
    } else {
        nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM)
            .context("Failed to SIGTERM session")?;
    }

    Ok(())
}

/// Discover running coop sessions by scanning /proc/*/environ for COOP_SESSION.
///
/// This is used for:
/// - `coop ls` when no daemon is running (fallback discovery)
/// - New daemon startup to rediscover orphaned sessions
/// - Consistency checks
pub fn discover_sessions() -> Vec<DiscoveredSession> {
    let mut sessions = Vec::new();

    let procs = match procfs::process::all_processes() {
        Ok(p) => p,
        Err(_) => return sessions,
    };

    let my_uid = nix::unistd::getuid().as_raw();

    for proc_entry in procs.flatten() {
        // Only look at processes owned by our UID
        if let Ok(status) = proc_entry.status() {
            let proc_uid = status.ruid;
            if proc_uid != my_uid {
                continue;
            }
        } else {
            continue;
        }

        // Read environ
        let environ = match proc_entry.environ() {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Check for COOP_SESSION
        let session_name = match environ.get(&std::ffi::OsString::from("COOP_SESSION")) {
            Some(v) => v.to_string_lossy().to_string(),
            None => continue,
        };

        let workspace = environ
            .get(&std::ffi::OsString::from("COOP_WORKSPACE"))
            .map(|v| v.to_string_lossy().to_string())
            .unwrap_or_default();

        let created = environ
            .get(&std::ffi::OsString::from("COOP_CREATED"))
            .and_then(|v| v.to_string_lossy().parse::<u64>().ok())
            .unwrap_or(0);

        sessions.push(DiscoveredSession {
            name: session_name,
            workspace,
            created,
            pid: proc_entry.pid() as u32,
        });
    }

    // Deduplicate: keep the lowest PID per session name (that's the init process)
    let mut by_name: HashMap<String, DiscoveredSession> = HashMap::new();
    for s in sessions {
        let entry = by_name.entry(s.name.clone()).or_insert(s.clone());
        if s.pid < entry.pid {
            *entry = s;
        }
    }

    by_name.into_values().collect()
}

/// Resolve a command name to its real path on the host via `which` + canonicalize.
/// Returns the resolved real path (following symlinks).
fn resolve_host_binary(cmd: &str) -> Result<PathBuf> {
    let output = std::process::Command::new("which")
        .arg(cmd)
        .output()
        .context("Failed to run which")?;

    if !output.status.success() {
        bail!("Command '{}' not found on host", cmd);
    }

    let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let real_path = std::fs::canonicalize(&path_str)
        .with_context(|| format!("Failed to resolve real path of {}", path_str))?;

    Ok(real_path)
}
