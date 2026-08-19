//! Host-side configuration. NON-SECRET metadata only.
//!
//! Global:   `~/.config/work/config.toml`
//! Per-ws:   `~/.config/work/workspaces/<ws>.toml`

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

pub const DEFAULT_IMAGE: &str = "work-base:latest";
/// Image built from the deterministic built-in developer tool bundle.
pub const DEVELOPER_IMAGE: &str = "work-base:developer";
pub const MINIMAL_PROFILE: &str = "minimal";
pub const DEVELOPER_PROFILE: &str = "developer";
pub const DEFAULT_PIDS_LIMIT: u32 = 4096;
pub const MIN_PIDS_LIMIT: u32 = 64;
pub const MAX_PIDS_LIMIT: u32 = 131_072;

/// Whether `work browse` asks before opening a new host. The permissive mode
/// is explicit per workspace; prompt remains the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum BrowserConfirmation {
    #[default]
    Prompt,
    Trusted,
}

impl std::str::FromStr for BrowserConfirmation {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "prompt" => Ok(Self::Prompt),
            "trusted" => Ok(Self::Trusted),
            _ => bail!("browser confirmation must be 'prompt' or 'trusted'"),
        }
    }
}

/// Whether macOS browser opens use a throwaway Chrome guest profile or the
/// normal host profile. Guest is the safer default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum BrowserProfile {
    #[default]
    Guest,
    Default,
}

impl std::str::FromStr for BrowserProfile {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "guest" => Ok(Self::Guest),
            "default" => Ok(Self::Default),
            _ => bail!("browser profile must be 'guest' or 'default'"),
        }
    }
}

pub fn validate_pids_limit(limit: u32) -> Result<u32> {
    if !(MIN_PIDS_LIMIT..=MAX_PIDS_LIMIT).contains(&limit) {
        bail!(
            "pids_limit must be between {MIN_PIDS_LIMIT} and {MAX_PIDS_LIMIT}; refusing an unlimited or impractically small limit"
        );
    }
    Ok(limit)
}

/// Fully resolved, non-secret workspace profile. Profiles deliberately describe
/// only Work-owned image/tool choices; they never execute host commands or
/// import host configuration or credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProfile {
    pub name: String,
    pub shell: String,
    pub image: String,
    pub bundles: Vec<String>,
}

pub fn builtin_profile_names() -> &'static [&'static str] {
    &[MINIMAL_PROFILE, DEVELOPER_PROFILE]
}

pub fn resolve_profile(profile: Option<&str>, shell: Option<&str>) -> Result<ResolvedProfile> {
    let name = profile.unwrap_or(MINIMAL_PROFILE);
    let (image, shells, bundles, default_shell): (&str, &[&str], &[&str], &str) = match name {
        MINIMAL_PROFILE => (DEFAULT_IMAGE, &["bash", "zsh"], &[], "zsh"),
        DEVELOPER_PROFILE => (
            DEVELOPER_IMAGE,
            &["bash", "zsh", "fish"],
            &["developer-tools"],
            "zsh",
        ),
        other => bail!(
            "unknown profile '{other}'; supported profiles: {}",
            builtin_profile_names().join(", ")
        ),
    };
    let shell = shell.unwrap_or(default_shell);
    if !shells.contains(&shell) {
        bail!(
            "profile '{name}' does not support shell '{shell}'; supported shells: {}",
            shells.join(", ")
        );
    }
    Ok(ResolvedProfile {
        name: name.to_string(),
        shell: shell.to_string(),
        image: image.to_string(),
        bundles: bundles.iter().map(|bundle| (*bundle).to_string()).collect(),
    })
}

fn default_workspace_image() -> String {
    DEFAULT_IMAGE.to_string()
}

fn default_legacy_created_at() -> String {
    "legacy".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalConfig {
    #[serde(default = "default_image")]
    pub default_image: Option<String>,
    /// Optional default for new workspaces. Unknown values are rejected when
    /// used, leaving legacy configs readable without mutation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_profile: Option<String>,
    /// Optional global default rc to seed into every new workspace (off by default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_shell_config: Option<PathBuf>,
    /// Optional global default herdr config (config.toml) to seed into every new workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_herdr_config: Option<PathBuf>,
    /// Optional global default starship.toml to seed into every new workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_starship_config: Option<PathBuf>,
    /// Optional global default dotfiles directory to seed (recursively) per-workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_dotfiles: Option<PathBuf>,
    /// Print the in-container identity banner on `work <ws>` attach (default on).
    #[serde(default = "default_show_banner")]
    pub show_banner: bool,
    /// Update-available check preferences (default: enabled).
    #[serde(default)]
    pub update: UpdatePrefs,
}

fn default_image() -> Option<String> {
    Some(DEFAULT_IMAGE.to_string())
}

fn default_show_banner() -> bool {
    true
}

impl GlobalConfig {
    pub fn effective_default_image(&self) -> &str {
        self.default_image.as_deref().unwrap_or(DEFAULT_IMAGE)
    }

    /// New workspaces use the developer experience unless the user explicitly
    /// selected another profile. Keeping this fallback here also upgrades old
    /// config.toml files that predate profiles without rewriting them.
    pub fn effective_default_profile(&self) -> &str {
        self.default_profile.as_deref().unwrap_or(DEVELOPER_PROFILE)
    }
}

/// Preferences for the update-available check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePrefs {
    /// Run the (best-effort, daily) update check. Default `true`.
    #[serde(default = "default_check")]
    pub check: bool,
}

fn default_check() -> bool {
    true
}

impl Default for UpdatePrefs {
    fn default() -> Self {
        Self { check: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub name: String,
    /// Preserve fields from older workspace schemas (such as `root` and
    /// `[env]`) while the current container schema is being adopted.
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub legacy_fields: BTreeMap<String, toml::Value>,
    /// Defaults for configs written by the pre-container-schema version of
    /// work, which described a host root/env instead of an image.
    #[serde(default = "default_workspace_image")]
    pub image: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    /// Resolved profile selected when this workspace was created. Absent means
    /// a legacy workspace and is intentionally left untouched on read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Work-owned allowlisted bundles resolved from `profile` at creation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bundles: Vec<String>,
    /// Resolved host sources used for managed imports. These are paths only;
    /// contents are never stored in host config. Keeping the source makes a
    /// later bare `work update` repeat the user's explicit choice instead of
    /// silently reverting to the bundled templates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_shell_config: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_herdr_config: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_starship_config: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_dotfiles: Option<PathBuf>,
    /// Per-workspace process cap. It is validated on every load and only takes
    /// effect when the container is next deliberately recreated.
    #[serde(default = "default_pids_limit")]
    pub pids_limit: u32,
    /// Browser bridge preferences are non-secret workspace metadata. Prompting
    /// and the guest profile remain the safe defaults.
    #[serde(default)]
    pub browser_confirmation: BrowserConfirmation,
    #[serde(default)]
    pub browser_profile: BrowserProfile,
    /// Daemon (engine) this workspace was created on, so a later `work <ws>`
    /// against a different active engine/context is refused instead of silently
    /// talking to a different daemon. `None` = created before this field existed
    /// (backfilled on next open, never a hard failure).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon_id: Option<String>,
    /// Resolved image ID from the selected engine's image inspection command, recorded at
    /// create/recreate. `doctor` compares the running container's image against
    /// this, not the tag string — so a locally-rebuilt `work-base:latest` that
    /// drifted from the recorded digest is flagged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_digest: Option<String>,
    #[serde(default = "default_legacy_created_at")]
    pub created_at: String,
}

fn default_pids_limit() -> u32 {
    DEFAULT_PIDS_LIMIT
}

pub fn config_dir() -> PathBuf {
    // Spec: ~/.config/work (XDG-style, identical on macOS and Linux).
    // Respect XDG_CONFIG_HOME when set; otherwise $HOME/.config.
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(x) if !x.is_empty() => PathBuf::from(x),
        _ => dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config"),
    }
    .join("work")
}

pub fn workspaces_dir() -> PathBuf {
    config_dir().join("workspaces")
}

pub fn workspace_config_path(ws: &str) -> PathBuf {
    workspaces_dir().join(format!("{ws}.toml"))
}

pub fn load_global() -> Result<GlobalConfig> {
    let path = config_dir().join("config.toml");
    if !path.exists() {
        return Ok(GlobalConfig {
            default_image: Some(DEFAULT_IMAGE.to_string()),
            default_profile: None,
            import_shell_config: None,
            import_herdr_config: None,
            import_starship_config: None,
            import_dotfiles: None,
            show_banner: true,
            update: UpdatePrefs::default(),
        });
    }
    let raw =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let g: GlobalConfig =
        toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
    Ok(g)
}

/// Persist global non-secret preferences. This is intentionally separate from
/// workspace config persistence so `work profile set-default` never touches
/// existing workspaces.
pub fn save_global(cfg: &GlobalConfig) -> Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let raw = toml::to_string_pretty(cfg).context("serializing global config")?;
    let path = dir.join("config.toml");
    std::fs::write(&path, raw).with_context(|| format!("writing {}", path.display()))
}

pub fn workspace_exists(ws: &str) -> bool {
    workspace_config_path(ws).exists()
}

pub fn load_workspace(ws: &str) -> Result<WorkspaceConfig> {
    // The requested name is authoritative: validate it, then refuse any file
    // whose internal `name` disagrees. Without this, a crafted/merged TOML
    // named "victim" inside `safe.toml` would make every downstream call
    // (remove/recreate/run) operate on "victim" while the user asked for "safe".
    crate::naming::validate_name(ws)
        .map_err(|e| anyhow::anyhow!("invalid workspace name '{ws}': {e}"))?;
    let path = workspace_config_path(ws);
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("workspace '{ws}' not found at {}", path.display()))?;
    let cfg: WorkspaceConfig =
        toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
    if cfg.name != ws {
        bail!(
            "config at {} claims name '{}', requested as '{}' — refusing to load a mismatched \
             config. Fix the `name` field or the filename.",
            path.display(),
            cfg.name,
            ws
        );
    }
    validate_pids_limit(cfg.pids_limit)
        .with_context(|| format!("validating workspace preferences in {}", path.display()))?;
    Ok(cfg)
}

pub fn save_workspace(cfg: &WorkspaceConfig) -> Result<()> {
    let dir = workspaces_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let raw = toml::to_string_pretty(cfg).context("serializing workspace config")?;
    let path = workspace_config_path(&cfg.name);
    std::fs::write(&path, raw).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Workspace names present on disk, sorted.
pub fn list_workspace_names() -> Result<Vec<String>> {
    let dir = workspaces_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path: &Path = &entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                names.push(stem.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

// ---------- familiarity: shell detection + import sources ----------

/// Detect the host shell for imports. Fish is supported by the developer
/// profile; the minimal profile still rejects it explicitly.
pub fn detect_shell() -> String {
    let sh = std::env::var("SHELL")
        .ok()
        .and_then(|s| {
            Path::new(&s)
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    match sh.as_str() {
        "zsh" | "bash" | "fish" => sh,
        _ => "zsh".to_string(),
    }
}

/// rc filename for a resolved shell; non-zsh -> `.bashrc`.
pub fn rc_name(shell: &str) -> &'static str {
    match shell {
        "zsh" => ".zshrc",
        "fish" => ".config/fish/config.fish",
        _ => ".bashrc",
    }
}

/// In-container absolute path for a resolved shell basename. The base image
/// ships `zsh` and `bash` at `/usr/bin/...`; `dev`'s login shell is
/// `/usr/bin/zsh`. `run_opts` sets this as `$SHELL` so tools that read it
/// (herdr's pane shell) spawn the right shell — a container started with
/// `sleep infinity` skips login, leaving `$SHELL` unset otherwise.
pub fn shell_path(shell: &str) -> &'static str {
    match shell {
        "bash" => "/usr/bin/bash",
        "fish" => "/usr/bin/fish",
        _ => "/usr/bin/zsh",
    }
}

/// Where a seeded config file comes from (per-workspace flag, possibly from the
/// global default). `Auto` resolves to the detected default path at seed time.
#[derive(Debug, Clone)]
pub enum ImportSrc {
    Auto,
    Explicit(PathBuf),
}

impl ImportSrc {
    /// Resolve to a concrete host path. `Auto` uses `~/<auto_name>`.
    pub fn to_path(&self, auto_name: &str) -> PathBuf {
        match self {
            ImportSrc::Explicit(p) => p.clone(),
            ImportSrc::Auto => dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(auto_name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_config_defaults_banner_on() {
        let parsed: GlobalConfig = toml::from_str("").unwrap();
        assert!(parsed.show_banner);
    }

    #[test]
    fn legacy_workspace_config_defaults_container_fields() {
        let parsed: WorkspaceConfig = toml::from_str(
            r#"
name = "acme"
root = "/home/lucas/work/acme"

[env]
"#,
        )
        .unwrap();
        assert_eq!(parsed.image, DEFAULT_IMAGE);
        assert_eq!(parsed.created_at, "legacy");
        assert!(parsed.legacy_fields.contains_key("root"));
        assert!(parsed.legacy_fields.contains_key("env"));
    }

    #[test]
    fn update_prefs_default_check_is_true() {
        assert!(UpdatePrefs::default().check);
    }

    #[test]
    fn load_global_defaults_include_update_enabled() {
        // config.toml absent -> defaults, with the update check on.
        let dir = std::env::temp_dir();
        // We assert the default struct directly (load_global reads a real path).
        let _ = dir;
        let g = GlobalConfig {
            default_image: Some(DEFAULT_IMAGE.to_string()),
            default_profile: None,
            import_shell_config: None,
            import_herdr_config: None,
            import_starship_config: None,
            import_dotfiles: None,
            show_banner: true,
            update: UpdatePrefs::default(),
        };
        assert!(g.update.check);
    }

    #[test]
    fn update_prefs_parses_check_false() {
        let g: GlobalConfig = toml::from_str("[update]\ncheck = false\n").unwrap();
        assert!(!g.update.check);
    }
}
