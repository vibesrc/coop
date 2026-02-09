use std::ffi::CString;
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use nix::sched::CloneFlags;
use nix::unistd::ForkResult;

use crate::config::{self, Coopfile};

/// Compute a hash of the config fields that affect the rootfs (image + setup).
fn config_hash(config: &Coopfile) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    config.sandbox.image.hash(&mut hasher);
    config.sandbox.setup.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Path to the rootfs manifest file
fn manifest_path() -> Result<std::path::PathBuf> {
    Ok(config::coop_dir()?.join("rootfs").join("manifest"))
}

/// Write the config hash to the manifest file after a successful build.
fn write_manifest(config: &Coopfile) -> Result<()> {
    let hash = config_hash(config);
    std::fs::write(manifest_path()?, &hash).context("Failed to write rootfs manifest")?;
    Ok(())
}

/// Check if the rootfs needs to be rebuilt (missing or config changed).
fn rootfs_is_stale(config: &Coopfile) -> Result<bool> {
    let base_path = config::rootfs_base_path()?;
    if !base_path.exists() {
        return Ok(true);
    }
    let mp = manifest_path()?;
    match std::fs::read_to_string(&mp) {
        Ok(stored) => Ok(stored.trim() != config_hash(config)),
        Err(_) => Ok(true), // no manifest = stale
    }
}

/// Ensure the rootfs exists. Builds on first run, warns if config has changed.
/// Use `coop init`/`coop rebuild` or `coop --build` for explicit rebuilds.
pub async fn ensure_rootfs(force_build: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let config = Coopfile::resolve(&cwd, None)?;
    let base_path = config::rootfs_base_path()?;

    if force_build {
        do_build_rootfs(&config).await?;
        return Ok(());
    }

    if !base_path.exists() {
        println!("First run — building rootfs...");
        do_build_rootfs(&config).await?;
    } else if rootfs_is_stale(&config)? {
        eprintln!("Warning: coop.toml has changed since last build. Run `coop rebuild` to update.");
    }

    Ok(())
}

/// Build the rootfs from the Coopfile.
/// This pulls the OCI image, installs packages, and runs setup commands.
pub async fn build_rootfs(_coopfile_path: &str, no_cache: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let config = Coopfile::resolve(&cwd, None)?;
    config.validate()?;

    let base_path = config::rootfs_base_path()?;

    if base_path.exists() && !no_cache && !rootfs_is_stale(&config)? {
        println!("Rootfs already built at {}", base_path.display());
        println!("Use `coop rebuild` to force rebuild");
        return Ok(());
    }

    do_build_rootfs(&config).await
}

/// Core rootfs build logic shared by `build_rootfs` and `ensure_rootfs`.
async fn do_build_rootfs(config: &Coopfile) -> Result<()> {
    let base_path = config::rootfs_base_path()?;

    // Ensure parent dirs exist
    if let Some(parent) = base_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Clean up existing rootfs if rebuilding
    if base_path.exists() {
        std::fs::remove_dir_all(&base_path)?;
    }

    println!("Building rootfs...");

    // Step 1: Pull and extract OCI image
    if let Some(image) = &config.sandbox.image {
        println!("  Pulling base image: {}", image);
        pull_oci_image(image, &base_path)?;
    } else {
        println!("  No base image specified, creating minimal rootfs");
        create_minimal_rootfs(&base_path)?;
    }

    // Step 2: Run setup commands
    let total = config.sandbox.setup.len();
    for (i, cmd) in config.sandbox.setup.iter().enumerate() {
        eprint!("  [{}/{}] Running: {} ... ", i + 1, total, cmd);
        match run_in_rootfs(&base_path, cmd) {
            Ok(()) => eprintln!("ok"),
            Err(e) => {
                eprintln!("FAILED");
                return Err(e);
            }
        }
    }

    // Step 3: Write manifest so we can detect config changes later
    write_manifest(config)?;

    println!("Rootfs built successfully at {}", base_path.display());
    Ok(())
}

/// Pull an OCI image and extract it to the target path.
///
/// Tries multiple approaches:
/// 1. `skopeo copy` + `umoci unpack` (most reliable)
/// 2. `crane export` (simpler, single tool)
/// 3. Fallback: create minimal rootfs and warn the user
fn pull_oci_image(image: &str, target: &Path) -> Result<()> {
    // Try skopeo + umoci first
    if let Ok(skopeo) = which("skopeo") {
        if let Ok(umoci) = which("umoci") {
            let oci_dir = target.with_file_name("oci_tmp");
            std::fs::create_dir_all(&oci_dir)?;

            let docker_ref = if image.contains("://") {
                image.to_string()
            } else {
                format!("docker://{}", image)
            };

            let status = Command::new(&skopeo)
                .args(["copy", &docker_ref, &format!("oci:{}:latest", oci_dir.display())])
                .status()
                .context("Failed to run skopeo")?;

            if status.success() {
                let status = Command::new(&umoci)
                    .args(["unpack", "--image", &format!("{}:latest", oci_dir.display()), &target.display().to_string()])
                    .status()
                    .context("Failed to run umoci")?;

                let _ = std::fs::remove_dir_all(&oci_dir);

                if status.success() {
                    return Ok(());
                }
            }

            let _ = std::fs::remove_dir_all(&oci_dir);
        }
    }

    // Try crane export (exports a tarball of the image filesystem)
    if let Ok(crane) = which("crane") {
        std::fs::create_dir_all(target)?;

        let output = Command::new(&crane)
            .args(["export", image, "-"])
            .output()
            .context("Failed to run crane")?;

        if output.status.success() {
            // Extract the tar to target
            let status = Command::new("tar")
                .args(["xf", "-", "-C", &target.display().to_string()])
                .stdin(std::process::Stdio::piped())
                .spawn()
                .and_then(|mut child| {
                    if let Some(stdin) = child.stdin.as_mut() {
                        use std::io::Write;
                        stdin.write_all(&output.stdout)?;
                    }
                    child.wait()
                });

            if let Ok(s) = status {
                if s.success() {
                    return Ok(());
                }
            }
        }
    }

    // Try docker/podman: create a temp container and export its filesystem
    for runtime in &["docker", "podman"] {
        if let Ok(rt) = which(runtime) {
            println!("  Pulling with {}...", runtime);
            let pull = Command::new(&rt)
                .args(["pull", image])
                .status();
            if !matches!(pull, Ok(s) if s.success()) {
                continue;
            }

            // Create a container (don't start it) and export its filesystem
            let create = Command::new(&rt)
                .args(["create", "--name", "coop_export_tmp", image, "/bin/true"])
                .output();
            if !matches!(&create, Ok(o) if o.status.success()) {
                let _ = Command::new(&rt).args(["rm", "coop_export_tmp"]).status();
                continue;
            }

            std::fs::create_dir_all(target)?;

            let status = Command::new("sh")
                .args([
                    "-c",
                    &format!(
                        "{} export coop_export_tmp | tar xf - -C {}",
                        rt.as_str(),
                        target.display()
                    ),
                ])
                .status();

            let _ = Command::new(&rt).args(["rm", "coop_export_tmp"]).status();

            if matches!(status, Ok(s) if s.success()) {
                return Ok(());
            }
        }
    }

    // Fallback: create a minimal rootfs
    println!("  Warning: No OCI tools found (skopeo, crane, docker, podman)");
    println!("  Creating minimal rootfs from host binaries");
    create_minimal_rootfs(target)?;

    Ok(())
}

/// Run a command inside the rootfs using a temporary user+mount namespace.
/// This is used during `coop init` to install packages and run setup commands.
/// Output is captured and only displayed on failure.
fn run_in_rootfs(rootfs: &Path, cmd: &str) -> Result<()> {
    let rootfs_owned = rootfs.to_path_buf();
    let cmd_owned = cmd.to_string();

    // Two pipes for parent-child sync (same pattern as create_session):
    // Pipe 1 (child→parent): child signals after unshare(), parent then writes UID/GID maps
    // Pipe 2 (parent→child): parent signals after writing maps, child then proceeds
    let (pipe1_rd_owned, pipe1_wr_owned) = nix::unistd::pipe()?;
    let pipe1_rd = pipe1_rd_owned.into_raw_fd();
    let pipe1_wr = pipe1_wr_owned.into_raw_fd();
    let (pipe2_rd_owned, pipe2_wr_owned) = nix::unistd::pipe()?;
    let pipe2_rd = pipe2_rd_owned.into_raw_fd();
    let pipe2_wr = pipe2_wr_owned.into_raw_fd();

    // Output capture pipe: child writes stdout/stderr here, parent buffers it
    let (out_rd_owned, out_wr_owned) = nix::unistd::pipe()?;
    let out_rd = out_rd_owned.into_raw_fd();
    let out_wr = out_wr_owned.into_raw_fd();

    match unsafe { nix::unistd::fork() }.context("fork() for rootfs command failed")? {
        ForkResult::Parent { child } => {
            unsafe { nix::libc::close(pipe1_wr); }
            unsafe { nix::libc::close(pipe2_rd); }
            unsafe { nix::libc::close(out_wr); }

            // Wait for child to unshare() before writing UID/GID maps
            let mut buf = [0u8; 1];
            let _ = nix::unistd::read(pipe1_rd, &mut buf);
            unsafe { nix::libc::close(pipe1_rd); }

            // Write UID/GID maps
            super::namespace::setup_uid_map(child)?;

            // Signal child that maps are ready
            let wr_fd = unsafe { std::os::unix::io::OwnedFd::from_raw_fd(pipe2_wr) };
            let _ = nix::unistd::write(&wr_fd, &[1u8]);
            drop(wr_fd);

            // Read all output from child (must drain before waitpid to avoid deadlock)
            let mut output = Vec::new();
            let mut read_buf = [0u8; 4096];
            loop {
                match nix::unistd::read(out_rd, &mut read_buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => output.extend_from_slice(&read_buf[..n]),
                }
            }
            unsafe { nix::libc::close(out_rd); }

            // Wait for child to complete
            match nix::sys::wait::waitpid(child, None) {
                Ok(nix::sys::wait::WaitStatus::Exited(_, 0)) => Ok(()),
                Ok(nix::sys::wait::WaitStatus::Exited(_, code)) => {
                    // Dump captured output on failure
                    let output_str = String::from_utf8_lossy(&output);
                    if !output_str.is_empty() {
                        eprintln!("{}", output_str);
                    }
                    bail!("Command '{}' exited with code {}", cmd, code)
                }
                Ok(status) => {
                    let output_str = String::from_utf8_lossy(&output);
                    if !output_str.is_empty() {
                        eprintln!("{}", output_str);
                    }
                    bail!("Command '{}' terminated: {:?}", cmd, status)
                }
                Err(e) => bail!("waitpid failed: {}", e),
            }
        }
        ForkResult::Child => {
            unsafe { nix::libc::close(pipe1_rd); }
            unsafe { nix::libc::close(pipe2_wr); }
            unsafe { nix::libc::close(out_rd); }

            // Redirect stdout and stderr to the output capture pipe
            unsafe {
                nix::libc::dup2(out_wr, 1);
                nix::libc::dup2(out_wr, 2);
                if out_wr > 2 { nix::libc::close(out_wr); }
            }

            // Unshare user + mount namespaces
            if let Err(e) =
                nix::sched::unshare(CloneFlags::CLONE_NEWUSER | CloneFlags::CLONE_NEWNS)
            {
                eprintln!("coop init: unshare failed: {}", e);
                std::process::exit(1);
            }

            // Signal parent that unshare() is done
            let wr_fd = unsafe { std::os::unix::io::OwnedFd::from_raw_fd(pipe1_wr) };
            let _ = nix::unistd::write(&wr_fd, &[1u8]);
            drop(wr_fd);

            // Wait for parent to write UID/GID maps
            let mut buf = [0u8; 1];
            let _ = nix::unistd::read(pipe2_rd, &mut buf);
            unsafe { nix::libc::close(pipe2_rd); }

            // chroot into rootfs
            if let Err(e) = nix::unistd::chroot(&rootfs_owned) {
                eprintln!("coop init: chroot failed: {}", e);
                std::process::exit(1);
            }
            let _ = std::env::set_current_dir("/");

            // Mount /proc so package managers work
            let _ = std::fs::create_dir_all("/proc");
            let _ = nix::mount::mount(
                Some("proc"),
                "/proc",
                Some("proc"),
                nix::mount::MsFlags::empty(),
                None::<&str>,
            );

            // Ensure DNS resolution works
            let _ = std::fs::create_dir_all("/etc");
            let _ = std::fs::write("/etc/resolv.conf", "nameserver 8.8.8.8\nnameserver 8.8.4.4\n");

            // Disable apt privilege dropping (only uid 0 is mapped in our user namespace)
            let _ = std::fs::create_dir_all("/etc/apt/apt.conf.d");
            let _ = std::fs::write(
                "/etc/apt/apt.conf.d/01-coop-nosandbox",
                "APT::Sandbox::User \"root\";\n",
            );

            // Exec the command via /bin/sh -c
            let sh = CString::new("/bin/sh").unwrap();
            let c_flag = CString::new("-c").unwrap();
            let c_cmd = CString::new(cmd_owned.as_str()).unwrap_or_else(|_| {
                CString::new("true").unwrap()
            });

            let env: Vec<CString> = vec![
                CString::new("PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin").unwrap(),
                CString::new("HOME=/root").unwrap(),
                CString::new("TERM=dumb").unwrap(),
                CString::new("DEBIAN_FRONTEND=noninteractive").unwrap(),
            ];

            let _ = nix::unistd::execvpe(&sh, &[sh.clone(), c_flag, c_cmd], &env);

            eprintln!("coop init: exec failed");
            std::process::exit(1);
        }
    }
}


/// Create a minimal rootfs structure (used when no OCI tools are available)
fn create_minimal_rootfs(path: &Path) -> Result<()> {
    let dirs = [
        "bin",
        "dev",
        "dev/pts",
        "etc",
        "home",
        "home/user",
        "lib",
        "lib64",
        "proc",
        "root",
        "run",
        "sbin",
        "sys",
        "tmp",
        "usr",
        "usr/bin",
        "usr/lib",
        "usr/sbin",
        "var",
        "var/tmp",
        "workspace",
    ];

    for dir in &dirs {
        std::fs::create_dir_all(path.join(dir))
            .with_context(|| format!("Failed to create {}", dir))?;
    }

    // Create basic /etc files
    std::fs::write(path.join("etc/hostname"), "coop\n")?;
    std::fs::write(
        path.join("etc/passwd"),
        "root:x:0:0:root:/root:/bin/sh\nuser:x:1000:1000:user:/home/user:/bin/sh\n",
    )?;
    std::fs::write(
        path.join("etc/group"),
        "root:x:0:\nuser:x:1000:\n",
    )?;
    std::fs::write(path.join("etc/resolv.conf"), "nameserver 8.8.8.8\nnameserver 8.8.4.4\n")?;
    std::fs::write(
        path.join("etc/nsswitch.conf"),
        "passwd: files\ngroup: files\nhosts: files dns\n",
    )?;

    // Try to copy essential binaries from the host for a functional rootfs
    copy_host_binary("/bin/sh", path)?;
    copy_host_binary("/bin/busybox", path)?;
    copy_host_binary("/usr/bin/env", path)?;

    Ok(())
}

/// Try to copy a binary from the host into the rootfs
fn copy_host_binary(host_path: &str, rootfs: &Path) -> Result<()> {
    let src = Path::new(host_path);
    if !src.exists() {
        return Ok(());
    }

    let dest = rootfs.join(host_path.trim_start_matches('/'));
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::copy(src, &dest)
        .with_context(|| format!("Failed to copy {} to rootfs", host_path))?;

    // Copy shared library dependencies (basic ldd parsing)
    if let Ok(output) = Command::new("ldd").arg(host_path).output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                // Parse lines like: "libfoo.so => /lib/x86_64-linux-gnu/libfoo.so (0x...)"
                if let Some(path_str) = line.split("=>").nth(1) {
                    let path_str = path_str.trim().split_whitespace().next().unwrap_or("");
                    if !path_str.is_empty() && Path::new(path_str).exists() {
                        let lib_dest = rootfs.join(path_str.trim_start_matches('/'));
                        if let Some(parent) = lib_dest.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        let _ = std::fs::copy(path_str, &lib_dest);
                    }
                }
                // Also handle direct paths like "/lib64/ld-linux-x86-64.so.2 (0x...)"
                let trimmed = line.trim();
                if trimmed.starts_with('/') {
                    let path_str = trimmed.split_whitespace().next().unwrap_or("");
                    if Path::new(path_str).exists() {
                        let lib_dest = rootfs.join(path_str.trim_start_matches('/'));
                        if let Some(parent) = lib_dest.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        let _ = std::fs::copy(path_str, &lib_dest);
                    }
                }
            }
        }
    }

    Ok(())
}

/// Check if a command exists on PATH
fn which(cmd: &str) -> Result<String> {
    let output = Command::new("which")
        .arg(cmd)
        .output()
        .context("Failed to run which")?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        bail!("{} not found", cmd)
    }
}

use std::os::unix::io::{FromRawFd, IntoRawFd};
