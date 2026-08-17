//! Container-engine abstraction. One trait, one CLI adapter.
//!
//! The `docker` CLI is the common substrate. Podman is CLI-compatible.
//! Linux default pick order: Podman → Docker → OrbStack → Colima.

use std::collections::BTreeSet;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

use crate::isolation::RunOpts;
use crate::naming;

fn managed_label() -> String {
    format!("{}=true", naming::LABEL_KEY)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineKind {
    Podman,
    Docker,
    OrbStack,
    Colima,
}

impl EngineKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EngineKind::Podman => "podman",
            EngineKind::Docker => "docker",
            EngineKind::OrbStack => "orbstack",
            EngineKind::Colima => "colima",
        }
    }

    pub fn binary(self) -> &'static str {
        match self {
            EngineKind::Podman => "podman",
            EngineKind::Docker | EngineKind::OrbStack | EngineKind::Colima => "docker",
        }
    }

    /// User-namespace auto-mapping is a Podman flag. Rootless Docker already maps.
    pub fn userns_auto(self) -> bool {
        matches!(self, EngineKind::Podman)
    }

    pub fn is_rootless_default(self) -> bool {
        matches!(self, EngineKind::Podman)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerState {
    Running,
    Stopped,
    Missing,
}

pub trait Engine: Send + Sync {
    fn kind(&self) -> EngineKind;
    fn binary(&self) -> &str;
    fn is_running(&self) -> Result<bool>;

    fn volume_exists(&self, name: &str) -> Result<bool>;
    fn create_volume(&self, name: &str) -> Result<()>;
    fn remove_volume(&self, name: &str) -> Result<()>;

    fn network_exists(&self, name: &str) -> Result<bool>;
    fn create_network(&self, name: &str) -> Result<()>;
    fn remove_network(&self, name: &str) -> Result<()>;

    fn container_exists(&self, name: &str) -> Result<bool>;
    fn container_state(&self, name: &str) -> Result<ContainerState>;
    fn run(&self, opts: &RunOpts) -> Result<()>;
    fn start_container(&self, name: &str) -> Result<()>;
    fn stop_container(&self, name: &str) -> Result<()>;
    fn remove_container(&self, name: &str) -> Result<()>;

    fn exec_interactive(&self, name: &str, cmd: &[&str], shell: &str) -> Result<()>;
    fn exec_capture(&self, name: &str, cmd: &[&str]) -> Result<String>;

    fn image_exists(&self, image: &str) -> Result<bool>;
    fn pull_image(&self, image: &str) -> Result<()>;
    fn build_image(&self, tag: &str, context: &std::path::Path, dockerfile: &str) -> Result<()>;

    fn container_networks(&self, name: &str) -> Result<BTreeSet<String>>;
    fn container_mounts(&self, name: &str) -> Result<Vec<(String, String)>>;
    fn inspect_format(&self, name: &str, format: &str) -> Result<String>;
    fn daemon_id(&self) -> Result<String>;
    fn object_has_label(&self, name: &str, kind: &str, key: &str) -> Result<bool>;
    fn image_id(&self, image: &str) -> Result<String>;
    fn list_containers(&self) -> Result<Vec<String>>;
}

/// Given which runtimes are present, pick per the Linux-first order.
pub fn pick_kind(podman: bool, docker: bool, orb: bool, colima: bool) -> Option<EngineKind> {
    if podman {
        Some(EngineKind::Podman)
    } else if docker {
        Some(EngineKind::Docker)
    } else if orb {
        Some(EngineKind::OrbStack)
    } else if colima {
        Some(EngineKind::Colima)
    } else {
        None
    }
}

fn which(bin: &str) -> bool {
    Command::new(bin)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn detect() -> Result<Box<dyn Engine>> {
    let podman = which("podman");
    let docker = which("docker");
    let orb = which("orb");
    let colima = which("colima");

    let kind = pick_kind(podman, docker, orb, colima).ok_or_else(|| {
        anyhow::anyhow!(
            "no supported container engine found. Install Podman (preferred) or Docker."
        )
    })?;
    Ok(Box::new(DockerCli { kind }))
}

pub struct DockerCli {
    kind: EngineKind,
}

impl DockerCli {
    fn cmd(&self) -> Command {
        Command::new(self.kind.binary())
    }

    fn run_success(&self, args: &[&str]) -> Result<()> {
        let out = self
            .cmd()
            .args(args)
            .output()
            .with_context(|| format!("{} {}", self.kind.binary(), args.join(" ")))?;
        if !out.status.success() {
            bail!(
                "{} {} failed (exit {:?}): {}",
                self.kind.binary(),
                args.join(" "),
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }

    fn exists_by_inspect(&self, kind: &str, name: &str) -> Result<bool> {
        let out = self
            .cmd()
            .args([kind, "inspect", name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .with_context(|| format!("inspect {kind} {name}"))?;
        Ok(out.success())
    }
}

impl Engine for DockerCli {
    fn kind(&self) -> EngineKind {
        self.kind
    }
    fn binary(&self) -> &str {
        self.kind.binary()
    }

    fn is_running(&self) -> Result<bool> {
        let status = self
            .cmd()
            .args(["info"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        Ok(status.success())
    }

    fn volume_exists(&self, name: &str) -> Result<bool> {
        self.exists_by_inspect("volume", name)
    }
    fn create_volume(&self, name: &str) -> Result<()> {
        self.run_success(&["volume", "create", "--label", &managed_label(), name])
    }
    fn remove_volume(&self, name: &str) -> Result<()> {
        self.run_success(&["volume", "rm", name])
    }

    fn network_exists(&self, name: &str) -> Result<bool> {
        self.exists_by_inspect("network", name)
    }
    fn create_network(&self, name: &str) -> Result<()> {
        self.run_success(&["network", "create", "--label", &managed_label(), name])
    }
    fn remove_network(&self, name: &str) -> Result<()> {
        self.run_success(&["network", "rm", name])
    }

    fn container_exists(&self, name: &str) -> Result<bool> {
        match self.container_state(name)? {
            ContainerState::Missing => Ok(false),
            _ => Ok(true),
        }
    }

    fn container_state(&self, name: &str) -> Result<ContainerState> {
        let out = self
            .cmd()
            .args(["inspect", "--format", "{{.State.Status}}", name])
            .output()
            .with_context(|| format!("inspect container {name}"))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.contains("No such object")
                || stderr.contains("no such container")
                || stderr.contains("error looking up")
            {
                return Ok(ContainerState::Missing);
            }
            bail!("could not inspect container {name}: {}", stderr.trim());
        }
        let state = String::from_utf8_lossy(&out.stdout).trim().to_string();
        match state.as_str() {
            "running" => Ok(ContainerState::Running),
            _ => Ok(ContainerState::Stopped),
        }
    }

    fn run(&self, opts: &RunOpts) -> Result<()> {
        if crate::isolation::is_host_bind(&opts.volume) {
            bail!("{}", crate::isolation::Forbidden::HostHomeBind.as_str());
        }
        if opts.volume_target != crate::isolation::HOME {
            bail!(
                "workspace volume must mount at {}, not {}",
                crate::isolation::HOME,
                opts.volume_target
            );
        }

        let mount = format!("{}:{}", opts.volume, opts.volume_target);
        let mut c = self.cmd();
        c.args([
            "run",
            "-d",
            "--name",
            &opts.name,
            "--network",
            &opts.network,
            "--restart",
            "unless-stopped",
            "-v",
            &mount,
            "-w",
            &opts.workdir,
            "--label",
            &managed_label(),
        ]);
        for (k, v) in &opts.env {
            c.arg("-e").arg(format!("{k}={v}"));
        }
        if opts.harden.cap_drop_all {
            c.args(["--cap-drop", "ALL"]);
        }
        if opts.harden.no_new_privileges {
            c.arg("--security-opt").arg("no-new-privileges:true");
        }
        if opts.harden.userns_auto {
            c.arg("--userns=auto");
        }
        if let Some(mem) = &opts.harden.memory {
            c.args(["--memory", mem.as_str(), "--memory-swap", mem.as_str()]);
        }
        if let Some(cpus) = &opts.harden.cpus {
            c.args(["--cpus", cpus.as_str()]);
        }
        if let Some(pids) = opts.harden.pids_limit {
            let pids = pids.to_string();
            c.args(["--pids-limit", pids.as_str()]);
        }
        if opts.harden.read_only_rootfs {
            c.arg("--read-only");
        }
        for (path, mo) in &opts.harden.tmpfs_mounts {
            c.args(["--tmpfs", &format!("{path}:{mo}")]);
        }
        c.arg(&opts.image);
        for arg in &opts.cmd {
            c.arg(arg);
        }
        let out = c
            .output()
            .with_context(|| format!("running container {}", opts.name))?;
        if !out.status.success() {
            bail!(
                "failed to create container {} (exit {:?}): {}",
                opts.name,
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }

    fn start_container(&self, name: &str) -> Result<()> {
        self.run_success(&["start", name])
    }
    fn stop_container(&self, name: &str) -> Result<()> {
        self.run_success(&["stop", name])
    }
    fn remove_container(&self, name: &str) -> Result<()> {
        self.run_success(&["rm", "-f", name])
    }

    fn exec_interactive(&self, name: &str, cmd: &[&str], shell: &str) -> Result<()> {
        let mut c = self.cmd();
        c.arg("exec");
        for var in ["TERM", "COLORTERM", "LANG"] {
            if let Ok(val) = std::env::var(var) {
                c.arg("-e").arg(format!("{var}={val}"));
            }
        }
        c.arg("-e")
            .arg(format!("SHELL={}", crate::config::shell_path(shell)));
        c.arg("-e").arg("HOME=/home/dev");
        let status = c
            .args(["-it", "-w", "/home/dev", name])
            .args(cmd)
            .status()?;
        if !status.success() {
            bail!("exec into {name} failed (exit {:?})", status.code());
        }
        Ok(())
    }

    fn exec_capture(&self, name: &str, cmd: &[&str]) -> Result<String> {
        let out = self
            .cmd()
            .args(["exec", "-w", "/home/dev", name])
            .args(cmd)
            .output()
            .with_context(|| format!("exec in {name}"))?;
        if !out.status.success() {
            bail!(
                "exec in {name} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    fn image_exists(&self, image: &str) -> Result<bool> {
        let out = self
            .cmd()
            .args(["image", "inspect", image])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        Ok(out.success())
    }

    fn pull_image(&self, image: &str) -> Result<()> {
        self.run_success(&["pull", image])
    }

    fn build_image(&self, tag: &str, context: &std::path::Path, dockerfile: &str) -> Result<()> {
        self.run_success(&[
            "build",
            "-t",
            tag,
            "-f",
            dockerfile,
            context.to_str().unwrap_or("."),
        ])
    }

    fn container_networks(&self, name: &str) -> Result<BTreeSet<String>> {
        let raw = self.inspect_format(name, "{{json .NetworkSettings.Networks}}")?;
        let map: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(raw.trim()).unwrap_or_default();
        Ok(map.keys().cloned().collect())
    }

    fn container_mounts(&self, name: &str) -> Result<Vec<(String, String)>> {
        let raw = self.inspect_format(name, "{{json .Mounts}}")?;
        let arr: Vec<serde_json::Value> = serde_json::from_str(raw.trim()).unwrap_or_default();
        Ok(arr
            .into_iter()
            .filter_map(|m| {
                let src = m
                    .get("Name")
                    .and_then(|v| v.as_str())
                    .or_else(|| m.get("Source").and_then(|v| v.as_str()))?
                    .to_string();
                let dst = m.get("Destination").and_then(|v| v.as_str())?.to_string();
                Some((src, dst))
            })
            .collect())
    }

    fn inspect_format(&self, name: &str, format: &str) -> Result<String> {
        let out = self
            .cmd()
            .args(["inspect", "--format", format, name])
            .output()
            .with_context(|| format!("inspect {name}"))?;
        if !out.status.success() {
            bail!(
                "inspect {name} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    fn daemon_id(&self) -> Result<String> {
        let out = self
            .cmd()
            .args(["info", "--format", "{{.ID}}"])
            .output()
            .context("daemon id")?;
        if !out.status.success() {
            bail!(
                "could not read daemon id: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if id.is_empty() {
            bail!("daemon id empty");
        }
        Ok(id)
    }

    fn object_has_label(&self, name: &str, kind: &str, key: &str) -> Result<bool> {
        // Volume/network labels live at `.Labels`; containers at `.Config.Labels`.
        let format = if kind == "container" {
            format!("{{{{index .Config.Labels \"{key}\"}}}}")
        } else {
            format!("{{{{index .Labels \"{key}\"}}}}")
        };
        let mut c = self.cmd();
        match kind {
            "volume" => c.args(["volume", "inspect", "--format", &format, name]),
            "network" => c.args(["network", "inspect", "--format", &format, name]),
            _ => c.args(["inspect", "--format", &format, name]),
        };
        let out = c.output()?;
        if !out.status.success() {
            return Ok(false);
        }
        let val = String::from_utf8_lossy(&out.stdout);
        Ok(val.trim() == "true" || val.contains("true"))
    }

    fn image_id(&self, image: &str) -> Result<String> {
        let out = self
            .cmd()
            .args(["image", "inspect", "--format", "{{.Id}}", image])
            .output()?;
        if !out.status.success() {
            bail!(
                "image inspect {image} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn list_containers(&self) -> Result<Vec<String>> {
        let out = self
            .cmd()
            .args(["ps", "-a", "--format", "{{.Names}}"])
            .output()?;
        if !out.status.success() {
            bail!(
                "ps failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_prefers_podman() {
        assert_eq!(
            pick_kind(true, true, true, true),
            Some(EngineKind::Podman)
        );
        assert_eq!(
            pick_kind(false, true, true, false),
            Some(EngineKind::Docker)
        );
        assert_eq!(
            pick_kind(false, false, true, true),
            Some(EngineKind::OrbStack)
        );
        assert_eq!(pick_kind(false, false, false, false), None);
    }
}
