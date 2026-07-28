//! Container-engine abstraction. One trait, one CLI adapter.
//!
//! The `docker` CLI is the common substrate: OrbStack, Docker, and Colima all
//! expose it; Podman is CLI-compatible (`podman` binary, identical verbs).

use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineKind {
    OrbStack,
    Docker,
    Podman,
    Colima,
}

impl EngineKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EngineKind::OrbStack => "orbstack",
            EngineKind::Docker => "docker",
            EngineKind::Podman => "podman",
            EngineKind::Colima => "colima",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerState {
    Running,
    Stopped,
    Missing,
}

/// `docker run` options for a workspace container.
pub struct RunOpts {
    pub name: String,
    pub image: String,
    pub network: String,
    pub volume: String,
    pub volume_target: String, // "/home/dev"
    pub workdir: String,
    pub cmd: Vec<String>, // e.g. ["sleep", "infinity"]
}

/// The only thing that talks to a container runtime.
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

    /// Interactive exec — inherits the calling process's stdio/tty.
    fn exec_interactive(&self, name: &str, cmd: &[&str]) -> Result<()>;
    /// Non-interactive exec — captures stdout.
    fn exec_capture(&self, name: &str, cmd: &[&str]) -> Result<String>;

    fn image_exists(&self, image: &str) -> Result<bool>;

    /// Inspect a container's networks -> set of network names.
    fn container_networks(&self, name: &str) -> Result<BTreeSet<String>>;
    /// Inspect a container's mounts -> list of (source_name_or_path, destination).
    fn container_mounts(&self, name: &str) -> Result<Vec<(String, String)>>;
}

// ---------- PURE selection ----------

/// Given which runtimes are present, pick per the locked order.
pub fn pick_kind(orb: bool, docker: bool, podman: bool, colima: bool) -> Option<EngineKind> {
    if orb {
        Some(EngineKind::OrbStack)
    } else if docker {
        Some(EngineKind::Docker)
    } else if podman {
        Some(EngineKind::Podman)
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
        .is_ok()
}

/// Detect OrbStack presence (the app exposes `docker`; `orb` is its own CLI).
fn orbstack_present() -> bool {
    which("orb")
}

pub fn detect() -> Result<Box<dyn Engine>> {
    let orb = orbstack_present();
    let docker = which("docker");
    let podman = which("podman");
    let colima = which("colima");

    let kind = pick_kind(orb, docker, podman, colima).ok_or_else(|| {
        anyhow::anyhow!(
            "no supported container engine found. Install OrbStack, Docker, Podman, or Colima."
        )
    })?;

    // OrbStack/Colima both drive the `docker` binary; Podman uses `podman`.
    let binary = match kind {
        EngineKind::Podman => "podman",
        _ => "docker",
    };
    Ok(Box::new(DockerCli {
        kind,
        binary: binary.to_string(),
    }))
}

// ---------- DockerCli adapter ----------

pub struct DockerCli {
    kind: EngineKind,
    binary: String,
}

impl DockerCli {
    fn cmd(&self) -> Command {
        Command::new(&self.binary)
    }

    /// Run a command, require success, return trimmed stdout.
    fn run_capture(&self, args: &[&str]) -> Result<String> {
        let out = self
            .cmd()
            .args(args)
            .output()
            .with_context(|| format!("spawning '{} {}'", self.binary, args.join(" ")))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            bail!("'{} {}' failed: {}", self.binary, args.join(" "), stderr.trim());
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn run_success(&self, args: &[&str]) -> Result<()> {
        let status = self
            .cmd()
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .with_context(|| format!("spawning '{} {}'", self.binary, args.join(" ")))?;
        if !status.success() {
            bail!(
                "'{} {}' failed (exit {:?})",
                self.binary,
                args.join(" "),
                status.code()
            );
        }
        Ok(())
    }
}

impl Engine for DockerCli {
    fn kind(&self) -> EngineKind {
        self.kind
    }
    fn binary(&self) -> &str {
        &self.binary
    }
    fn is_running(&self) -> Result<bool> {
        Ok(self
            .cmd()
            .arg("info")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?
            .success())
    }

    fn volume_exists(&self, name: &str) -> Result<bool> {
        let code = self
            .cmd()
            .args(["volume", "inspect", name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        Ok(code.success())
    }
    fn create_volume(&self, name: &str) -> Result<()> {
        self.run_success(&["volume", "create", name])
    }
    fn remove_volume(&self, name: &str) -> Result<()> {
        self.run_success(&["volume", "rm", name])
    }

    fn network_exists(&self, name: &str) -> Result<bool> {
        let code = self
            .cmd()
            .args(["network", "inspect", name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        Ok(code.success())
    }
    fn create_network(&self, name: &str) -> Result<()> {
        self.run_success(&["network", "create", name])
    }
    fn remove_network(&self, name: &str) -> Result<()> {
        self.run_success(&["network", "rm", name])
    }

    fn container_exists(&self, name: &str) -> Result<bool> {
        Ok(matches!(
            self.container_state(name)?,
            ContainerState::Running | ContainerState::Stopped
        ))
    }

    fn container_state(&self, name: &str) -> Result<ContainerState> {
        // `docker inspect` exits non-zero when the container is absent; that is
        // Missing, not an error. Only treat a *present* container's status.
        let out = self
            .cmd()
            .args([
                "inspect",
                "--type",
                "container",
                "--format",
                "{{.State.Status}}",
                name,
            ])
            .output()?;
        if !out.status.success() {
            return Ok(ContainerState::Missing);
        }
        let state = String::from_utf8_lossy(&out.stdout).trim().to_string();
        match state.as_str() {
            "running" => Ok(ContainerState::Running),
            _ => Ok(ContainerState::Stopped),
        }
    }
    fn run(&self, opts: &RunOpts) -> Result<()> {
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
            &opts.image,
        ]);
        for arg in &opts.cmd {
            c.arg(arg);
        }
        // Capture output so the container ID doesn't leak, and so we can report
        // docker's stderr if creation fails.
        let out = c.output().with_context(|| format!("running container {}", opts.name))?;
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

    fn exec_interactive(&self, name: &str, cmd: &[&str]) -> Result<()> {
        let status = self
            .cmd()
            .args(["exec", "-it", "-w", "/home/dev", name])
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
            .output()?;
        if !out.status.success() {
            bail!(
                "exec capture into {name} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn image_exists(&self, image: &str) -> Result<bool> {
        let code = self
            .cmd()
            .args(["image", "inspect", image])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        Ok(code.success())
    }

    fn container_networks(&self, name: &str) -> Result<BTreeSet<String>> {
        let json = self.run_capture(&[
            "inspect",
            "--type",
            "container",
            "--format",
            "{{json .NetworkSettings.Networks}}",
            name,
        ])?;
        let map: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&json).unwrap_or_default();
        Ok(map.keys().cloned().collect())
    }
    fn container_mounts(&self, name: &str) -> Result<Vec<(String, String)>> {
        let json = self.run_capture(&[
            "inspect",
            "--type",
            "container",
            "--format",
            "{{json .Mounts}}",
            name,
        ])?;
        #[derive(Deserialize)]
        struct Mount {
            #[serde(rename = "Type")]
            typ: String,
            #[serde(rename = "Name", default)]
            name: Option<String>,
            #[serde(rename = "Source", default)]
            source: Option<String>,
            #[serde(rename = "Destination")]
            destination: String,
        }
        let mounts: Vec<Mount> = serde_json::from_str(&json).unwrap_or_default();
        let out = mounts
            .into_iter()
            .map(|m| {
                let src = if m.typ == "volume" {
                    m.name.unwrap_or_default()
                } else {
                    m.source.unwrap_or_default()
                };
                (src, m.destination)
            })
            .collect();
        Ok(out)
    }
}

/// Helper exposed to the `image` module: build `tag` from a context dir + Dockerfile path,
/// inheriting the user's terminal so build logs stream.
pub fn build_image_at(engine: &dyn Engine, tag: &str, context_dir: &Path, dockerfile: &Path) -> Result<()> {
    let status = Command::new(engine.binary())
        .args(["build", "-t", tag, "-f"])
        .arg(dockerfile)
        .arg(".")
        .current_dir(context_dir)
        .status()
        .with_context(|| format!("building image {tag}"))?;
    if !status.success() {
        bail!("image build for {tag} failed (exit {:?})", status.code());
    }
    Ok(())
}
