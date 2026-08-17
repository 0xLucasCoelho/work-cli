//! Isolation policy that every workspace `run` must satisfy. PURE.

use crate::naming;

/// In-container home. The only writable tenancy mount.
pub const HOME: &str = naming::HOME_TARGET;

/// Identity + agent-brain env pinned inside the volume.
///
/// Host `~/.claude`, `~/.codex`, `~/.config/gh` stay unreachable when these
/// resolve under `/home/dev`.
pub fn identity_env(ws: &str, shell_path: &str) -> Vec<(String, String)> {
    vec![
        ("HOME".into(), HOME.into()),
        ("WORK".into(), ws.into()),
        ("WORKSPACE".into(), ws.into()),
        ("SHELL".into(), shell_path.into()),
        ("CLAUDE_CONFIG_DIR".into(), format!("{HOME}/.claude")),
        ("CODEX_HOME".into(), format!("{HOME}/.codex")),
        ("GH_CONFIG_DIR".into(), format!("{HOME}/.config/gh")),
        ("XDG_CONFIG_HOME".into(), format!("{HOME}/.config")),
        ("XDG_DATA_HOME".into(), format!("{HOME}/.local/share")),
        ("XDG_STATE_HOME".into(), format!("{HOME}/.local/state")),
        ("XDG_CACHE_HOME".into(), format!("{HOME}/.cache")),
        ("XDG_RUNTIME_DIR".into(), "/tmp/runtime-dev".into()),
        ("NERD_FONTS".into(), "1".into()),
    ]
}

/// Hardening applied to every workspace container.
#[derive(Debug, Clone)]
pub struct HardenOpts {
    pub cap_drop_all: bool,
    pub no_new_privileges: bool,
    pub pids_limit: Option<u32>,
    /// Podman `--userns=auto`. Off for Docker (rootless Docker already maps).
    pub userns_auto: bool,
    pub memory: Option<String>,
    pub cpus: Option<String>,
    pub read_only_rootfs: bool,
    pub tmpfs_mounts: Vec<(String, String)>,
}

impl Default for HardenOpts {
    fn default() -> Self {
        Self {
            cap_drop_all: true,
            no_new_privileges: true,
            pids_limit: Some(4096),
            userns_auto: false,
            memory: None,
            cpus: None,
            read_only_rootfs: false,
            tmpfs_mounts: Vec::new(),
        }
    }
}

/// `docker/podman run` options for a workspace container.
///
/// Construction goes through [`workspace_run_opts`] so a caller cannot request
/// a host bind, published port, or extra network.
#[derive(Debug, Clone)]
pub struct RunOpts {
    pub name: String,
    pub image: String,
    pub network: String,
    pub volume: String,
    pub volume_target: String,
    pub workdir: String,
    pub cmd: Vec<String>,
    pub env: Vec<(String, String)>,
    pub harden: HardenOpts,
}

/// Footguns the CLI and engine must refuse. PURE so tests pin the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forbidden {
    DockerSock,
    Privileged,
    HostNetwork,
    HostPid,
    HostHomeBind,
    PublishedPort,
}

impl Forbidden {
    pub fn as_str(self) -> &'static str {
        match self {
            Forbidden::DockerSock => "docker.sock must never be mounted",
            Forbidden::Privileged => "--privileged is refused",
            Forbidden::HostNetwork => "--network=host is refused",
            Forbidden::HostPid => "--pid=host is refused",
            Forbidden::HostHomeBind => "host $HOME bind-mounts are refused",
            Forbidden::PublishedPort => "workspace containers must not publish host ports",
        }
    }
}

/// True if `source` looks like a host path (bind mount), not a named volume.
pub fn is_host_bind(source: &str) -> bool {
    source.starts_with('/') || source.contains(":\\")
}

/// Build the only legal run spec for a company box.
pub fn workspace_run_opts(ws: &str, image: &str, shell_path: &str, userns_auto: bool) -> RunOpts {
    let harden = HardenOpts {
        userns_auto,
        ..HardenOpts::default()
    };
    RunOpts {
        name: naming::container(ws),
        image: image.to_string(),
        network: naming::network(ws),
        volume: naming::volume(ws),
        volume_target: HOME.into(),
        workdir: HOME.into(),
        cmd: vec!["sleep".into(), "infinity".into()],
        env: identity_env(ws, shell_path),
        harden,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_env_pins_agent_homes_inside_volume() {
        let env = identity_env("acme", "/usr/bin/zsh");
        let get = |k: &str| {
            env.iter()
                .find(|(a, _)| a == k)
                .map(|(_, v)| v.as_str())
                .unwrap()
        };
        assert_eq!(get("HOME"), "/home/dev");
        assert_eq!(get("WORK"), "acme");
        assert_eq!(get("CLAUDE_CONFIG_DIR"), "/home/dev/.claude");
        assert_eq!(get("CODEX_HOME"), "/home/dev/.codex");
        assert_eq!(get("GH_CONFIG_DIR"), "/home/dev/.config/gh");
        assert_eq!(get("XDG_CONFIG_HOME"), "/home/dev/.config");
        assert!(!get("CLAUDE_CONFIG_DIR").starts_with("/home/lucas"));
    }

    #[test]
    fn default_harden_drops_caps() {
        let h = HardenOpts::default();
        assert!(h.cap_drop_all);
        assert!(h.no_new_privileges);
        assert_eq!(h.pids_limit, Some(4096));
    }

    #[test]
    fn workspace_opts_use_named_volume_not_bind() {
        let opts = workspace_run_opts("acme", "work-base:latest", "/usr/bin/zsh", true);
        assert_eq!(opts.volume, "work-acme-home");
        assert_eq!(opts.volume_target, "/home/dev");
        assert_eq!(opts.network, "work-net-acme");
        assert!(!is_host_bind(&opts.volume));
        assert!(opts.harden.userns_auto);
    }

    #[test]
    fn host_paths_are_binds() {
        assert!(is_host_bind("/home/you"));
        assert!(is_host_bind("/var/run/docker.sock"));
        assert!(!is_host_bind("work-acme-home"));
    }
}
