use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Top-level Coopfile structure (coop.toml)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Coopfile {
    #[serde(default)]
    pub sandbox: SandboxConfig,
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub network: NetworkConfig,
    #[serde(default)]
    pub session: SessionConfig,
    #[serde(default)]
    pub input_filter: InputFilterConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    #[serde(default = "default_workspace_mount")]
    pub mount: String,
    #[serde(default = "default_workspace_path")]
    pub path: String,
}

fn default_workspace_mount() -> String {
    ".".to_string()
}

fn default_workspace_path() -> String {
    "/workspace".to_string()
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            mount: default_workspace_mount(),
            path: default_workspace_path(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    pub image: Option<String>,
    /// Deprecated: use `agent` instead. Kept for backwards compat.
    #[serde(skip_serializing)]
    pub command: Option<String>,
    /// The agent process command (the main long-running process in the box)
    pub agent: Option<String>,
    /// The shell command for `coop shell` sessions (default: /bin/bash)
    pub shell: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub setup: Vec<String>,
    #[serde(default = "default_user")]
    pub user: String,
    /// Mounts into the sandbox. Path-based entries (e.g. "~/.bashrc:~/.bashrc")
    /// are bind-mounted directly. Named entries (e.g. "claude-config:~/.claude")
    /// use managed persistent storage (see `coop volume ls/rm/prune`).
    #[serde(default)]
    pub mounts: Vec<MountConfig>,
}

impl SandboxConfig {
    /// Resolve the agent command: `agent` > `command` > None
    pub fn agent_command(&self) -> Option<&str> {
        self.agent.as_deref().or(self.command.as_deref())
    }

    /// Resolve the shell command: `shell` > "/bin/bash"
    pub fn shell_command(&self) -> &str {
        self.shell.as_deref().unwrap_or("/bin/bash")
    }
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            image: None,
            command: None,
            agent: None,
            shell: None,
            args: Vec::new(),
            setup: Vec::new(),
            user: default_user(),
            mounts: Vec::new(),
        }
    }
}

/// A bind mount from host into the sandbox.
/// Can be specified as a string "host:container" or as a table { host = "...", container = "..." }.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MountConfig {
    Short(String),
    Full { host: String, container: String },
}

impl MountConfig {
    /// Expand ~ in a container path to the sandbox home directory
    fn expand_container_path(path: &str, sandbox_home: &str) -> String {
        if path == "~" {
            sandbox_home.to_string()
        } else if let Some(rest) = path.strip_prefix("~/") {
            format!("{}/{}", sandbox_home, rest)
        } else {
            path.to_string()
        }
    }

    /// Check if the left side looks like a path (starts with /, ~, or .)
    fn is_path(s: &str) -> bool {
        s.starts_with('/') || s.starts_with('~') || s.starts_with('.')
    }

    /// Check if this mount/volume uses a named volume (left side is a name, not a path)
    pub fn is_named_volume(&self) -> bool {
        match self {
            MountConfig::Short(s) => {
                let left = s.splitn(2, ':').next().unwrap_or("");
                !Self::is_path(left)
            }
            MountConfig::Full { host, .. } => !Self::is_path(host),
        }
    }

    /// Get the volume name (for named volumes only)
    pub fn volume_name(&self) -> Option<String> {
        if !self.is_named_volume() {
            return None;
        }
        match self {
            MountConfig::Short(s) => Some(s.splitn(2, ':').next().unwrap_or("").to_string()),
            MountConfig::Full { host, .. } => Some(host.clone()),
        }
    }

    /// Get just the container path (expanded with sandbox home)
    pub fn container_path(&self, sandbox_home: &str) -> Result<String> {
        match self {
            MountConfig::Short(s) => {
                let parts: Vec<&str> = s.splitn(2, ':').collect();
                if parts.len() != 2 {
                    bail!("Invalid format '{}', expected 'source:container'", s);
                }
                Ok(Self::expand_container_path(parts[1], sandbox_home))
            }
            MountConfig::Full { container, .. } => {
                Ok(Self::expand_container_path(container, sandbox_home))
            }
        }
    }

    /// Parse into (host_path, container_path). Short form is "host:container".
    pub fn resolve_with_home(&self, sandbox_home: &str) -> Result<(PathBuf, String)> {
        match self {
            MountConfig::Short(s) => {
                let parts: Vec<&str> = s.splitn(2, ':').collect();
                if parts.len() != 2 {
                    bail!("Invalid mount format '{}', expected 'host:container'", s);
                }
                let host = shellexpand::tilde(parts[0]);
                let container = Self::expand_container_path(parts[1], sandbox_home);
                Ok((PathBuf::from(host.as_ref()), container))
            }
            MountConfig::Full { host, container } => {
                let host = shellexpand::tilde(host);
                let container = Self::expand_container_path(container, sandbox_home);
                Ok((PathBuf::from(host.as_ref()), container))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NetworkMode {
    None,
    Host,
    Veth,
}

impl Default for NetworkMode {
    fn default() -> Self {
        NetworkMode::Host
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    #[serde(default)]
    pub mode: NetworkMode,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            mode: NetworkMode::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    #[serde(default = "default_persist")]
    pub persist: Vec<String>,
    #[serde(default = "default_true")]
    pub auto_restart: bool,
    #[serde(default = "default_restart_delay")]
    pub restart_delay_ms: u64,
}

fn default_user() -> String {
    "coop".to_string()
}

fn default_persist() -> Vec<String> {
    vec![".claude".to_string()]
}

fn default_true() -> bool {
    true
}

fn default_restart_delay() -> u64 {
    100
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            persist: default_persist(),
            auto_restart: true,
            restart_delay_ms: 100,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputFilterConfig {
    #[serde(default = "default_debounce")]
    pub ctrl_c_debounce_ms: u64,
    #[serde(default)]
    pub block_sequences: Vec<String>,
}

fn default_debounce() -> u64 {
    500
}

impl Default for InputFilterConfig {
    fn default() -> Self {
        Self {
            ctrl_c_debounce_ms: 500,
            block_sequences: Vec::new(),
        }
    }
}

impl Coopfile {
    /// Parse a Coopfile from a TOML string
    pub fn parse(content: &str) -> Result<Self> {
        let coopfile: Coopfile = toml::from_str(content).context("Failed to parse Coopfile")?;
        Ok(coopfile)
    }

    /// Load a Coopfile from a file path
    pub fn load(path: &Path) -> Result<Self> {
        let content =
            std::fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
        Self::parse(&content)
    }

    /// Merge another Coopfile on top of this one (other overrides self)
    pub fn merge(&mut self, other: &Coopfile) {
        // Sandbox
        if other.sandbox.image.is_some() {
            self.sandbox.image = other.sandbox.image.clone();
        }
        if other.sandbox.command.is_some() {
            self.sandbox.command = other.sandbox.command.clone();
        }
        if other.sandbox.agent.is_some() {
            self.sandbox.agent = other.sandbox.agent.clone();
        }
        if other.sandbox.shell.is_some() {
            self.sandbox.shell = other.sandbox.shell.clone();
        }
        if !other.sandbox.args.is_empty() {
            self.sandbox.args = other.sandbox.args.clone();
        }
        if !other.sandbox.setup.is_empty() {
            self.sandbox.setup.extend(other.sandbox.setup.iter().cloned());
        }
        if other.sandbox.user != default_user() {
            self.sandbox.user = other.sandbox.user.clone();
        }
        if !other.sandbox.mounts.is_empty() {
            self.sandbox.mounts.extend(other.sandbox.mounts.iter().cloned());
        }

        // Env: additive merge
        for (k, v) in &other.env {
            self.env.insert(k.clone(), v.clone());
        }

        // Network: override
        self.network.mode = other.network.mode;

        // Session: override
        if other.session.persist != default_persist() {
            self.session.persist = other.session.persist.clone();
        }
        self.session.auto_restart = other.session.auto_restart;
        self.session.restart_delay_ms = other.session.restart_delay_ms;

        // Input filter: override
        self.input_filter.ctrl_c_debounce_ms = other.input_filter.ctrl_c_debounce_ms;
        if !other.input_filter.block_sequences.is_empty() {
            self.input_filter
                .block_sequences
                .extend(other.input_filter.block_sequences.iter().cloned());
        }
    }

    /// Resolve the full Coopfile by merging layers: defaults -> global -> project -> CLI
    pub fn resolve(workspace_dir: &Path, cli_overrides: Option<&Coopfile>) -> Result<Self> {
        let mut config = Coopfile::default();

        // Layer 1: Global config
        let global_path = super::global_config_path()?;
        if global_path.exists() {
            let global = Coopfile::load(&global_path)?;
            config.merge(&global);
        }

        // Layer 2: Project config
        let project_path = workspace_dir.join("coop.toml");
        if project_path.exists() {
            let project = Coopfile::load(&project_path)?;
            config.merge(&project);
        }

        // Layer 3: CLI overrides
        if let Some(overrides) = cli_overrides {
            config.merge(overrides);
        }

        Ok(config)
    }

    /// Expand $VARIABLE references in env values from host environment
    pub fn expand_env(&mut self) {
        let expanded: HashMap<String, String> = self
            .env
            .iter()
            .map(|(k, v)| {
                let value = if let Some(var_name) = v.strip_prefix('$') {
                    std::env::var(var_name).unwrap_or_default()
                } else {
                    v.clone()
                };
                (k.clone(), value)
            })
            .collect();
        self.env = expanded;
    }

    /// Validate the Coopfile, returning errors for invalid configuration
    pub fn validate(&self) -> Result<()> {
        if self.sandbox.agent_command().is_none() {
            bail!("sandbox.agent or sandbox.command is required (set it in coop.toml or ~/.config/coop/default.toml)");
        }
        Ok(())
    }

    /// Resolve the workspace mount path relative to a base directory
    pub fn resolve_workspace_mount(&self, base: &Path) -> PathBuf {
        if self.workspace.mount == "." {
            base.to_path_buf()
        } else {
            PathBuf::from(&self.workspace.mount)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal() {
        let toml = r#"
[sandbox]
agent = "claude"
image = "node:22-alpine"
"#;
        let cf = Coopfile::parse(toml).unwrap();
        assert_eq!(cf.sandbox.agent_command(), Some("claude"));
        assert_eq!(cf.sandbox.image.as_deref(), Some("node:22-alpine"));
        assert_eq!(cf.network.mode, NetworkMode::Host);
    }

    #[test]
    fn test_parse_legacy_command() {
        let toml = r#"
[sandbox]
command = "claude"
"#;
        let cf = Coopfile::parse(toml).unwrap();
        assert_eq!(cf.sandbox.agent_command(), Some("claude"));
    }

    #[test]
    fn test_merge() {
        let mut base = Coopfile::default();
        base.env.insert("KEY1".into(), "val1".into());

        let mut overlay = Coopfile::default();
        overlay.env.insert("KEY2".into(), "val2".into());
        overlay.sandbox.agent = Some("claude".into());

        base.merge(&overlay);
        assert_eq!(base.env.get("KEY1").unwrap(), "val1");
        assert_eq!(base.env.get("KEY2").unwrap(), "val2");
        assert_eq!(base.sandbox.agent_command(), Some("claude"));
    }

    #[test]
    fn test_defaults() {
        let cf = Coopfile::default();
        assert_eq!(cf.session.auto_restart, true);
        assert_eq!(cf.session.restart_delay_ms, 100);
        assert_eq!(cf.input_filter.ctrl_c_debounce_ms, 500);
        assert_eq!(cf.session.persist, vec![".claude"]);
        assert_eq!(cf.network.mode, NetworkMode::Host);
    }
}
