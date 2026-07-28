//! Host-side configuration. NON-SECRET metadata only.
//!
//! Global:   `~/.config/work/config.toml`
//! Per-ws:   `~/.config/work/workspaces/<ws>.toml`

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const DEFAULT_IMAGE: &str = "work-base:latest";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalConfig {
    #[serde(default = "default_image")]
    pub default_image: Option<String>,
}

fn default_image() -> Option<String> {
    Some(DEFAULT_IMAGE.to_string())
}

impl GlobalConfig {
    pub fn effective_default_image(&self) -> &str {
        self.default_image.as_deref().unwrap_or(DEFAULT_IMAGE)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub name: String,
    pub image: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    pub created_at: String,
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
        });
    }
    let raw =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let g: GlobalConfig =
        toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
    Ok(g)
}

pub fn workspace_exists(ws: &str) -> bool {
    workspace_config_path(ws).exists()
}

pub fn load_workspace(ws: &str) -> Result<WorkspaceConfig> {
    let path = workspace_config_path(ws);
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("workspace '{ws}' not found at {}", path.display()))?;
    let cfg: WorkspaceConfig =
        toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
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
