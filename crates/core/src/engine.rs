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
    /// Extra environment (`-e KEY=VALUE`). Identity metadata only.
    pub env: Vec<(String, String)>,
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
    /// `docker exec --user root <name> <cmd...>` (non-interactive), require
    /// success. For one-off system setup the `dev` user can't perform (e.g.
    /// installing a shim under `/usr/local/bin`). Mirrors the `--user root`
    /// pattern in seed_file/seed_dir, but as a general exec.
    fn exec_root(&self, name: &str, cmd: &[&str]) -> Result<()>;
    /// True iff container `name` has a tmux server with `session`. Returns
    /// `false` (NOT an error) when the container is missing/stopped or the
    /// session is absent — so `ls`/`stop`/`rm` never choke on a downed box.
    fn session_exists(&self, name: &str, session: &str) -> Result<bool>;
    /// Copy host file `src` into container `name` at `dest`, owned by `dev`.
    /// `docker cp` + `chown dev:dev` (as root). Creates no host bind-mount.
    fn seed_file(&self, name: &str, src: &Path, dest: &str) -> Result<()>;
    /// Recursively copy a host directory's *contents* into `dest_dir` in the
    /// container, then chown -R to dev. Mirrors seed_file for a whole tree.
    fn seed_dir(&self, name: &str, src_dir: &Path, dest_dir: &str) -> Result<()>;

    fn image_exists(&self, image: &str) -> Result<bool>;

    /// Inspect a container's networks -> set of network names.
    fn container_networks(&self, name: &str) -> Result<BTreeSet<String>>;
    /// Inspect a container's mounts -> list of (source_name_or_path, destination).
    fn container_mounts(&self, name: &str) -> Result<Vec<(String, String)>>;
    /// Generic `docker inspect --format` for a container (docker Go-template).
    /// Used by `doctor` for restart-policy / user / image / port checks.
    fn inspect_format(&self, name: &str, format: &str) -> Result<String>;

    /// Pull an image if absent (custom workspace images).
    fn pull_image(&self, image: &str) -> Result<()>;

    /// Run a detached port-forwarder container on `network` that bridges
    /// `127.0.0.1:host_port` (host) to `target:target_port` (in-network).
    fn run_forwarder(
        &self,
        name: &str,
        network: &str,
        host_port: u16,
        target: &str,
        target_port: u16,
    ) -> Result<()>;
    /// Like `run_forwarder` but non-blocking: spawn the bridge as a detached
    /// child (in the caller's process group) and return immediately. Used by
    /// `work browse` to auto-bridge OAuth callback ports without blocking its
    /// read loop. Cleanup is via Ctrl-C (process-group SIGINT -> `--rm`) or an
    /// explicit `remove_container`.
    fn spawn_forwarder(
        &self,
        name: &str,
        network: &str,
        host_port: u16,
        target: &str,
        target_port: u16,
    ) -> Result<()>;
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
            bail!(
                "'{} {}' failed: {}",
                self.binary,
                args.join(" "),
                stderr.trim()
            );
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
        ]);
        for (k, v) in &opts.env {
            c.arg("-e").arg(format!("{k}={v}"));
        }
        c.arg(&opts.image);
        for arg in &opts.cmd {
            c.arg(arg);
        }
        // Capture output so the container ID doesn't leak, and so we can report
        // docker's stderr if creation fails.
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
    fn exec_root(&self, name: &str, cmd: &[&str]) -> Result<()> {
        let out = self
            .cmd()
            .args(["exec", "--user", "root", name])
            .args(cmd)
            .output()
            .with_context(|| format!("exec (root) into {name}"))?;
        if !out.status.success() {
            bail!(
                "exec (root) into {name} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }
    fn session_exists(&self, name: &str, session: &str) -> Result<bool> {
        // Any non-zero exit (container missing / not running / no session) -> false.
        let code = self
            .cmd()
            .args(["exec", name, "tmux", "has-session", "-t", session])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        Ok(code.success())
    }
    fn seed_file(&self, name: &str, src: &Path, dest: &str) -> Result<()> {
        let src_s = src.to_string_lossy();
        let target = format!("{name}:{dest}");
        // Ensure dest's parent dir exists (e.g. /home/dev/.config for starship.toml);
        // `docker cp` won't create intermediate dirs. Idempotent — /home/dev exists
        // for rc/.tmux.conf seeds, so this is a no-op there.
        if let Some(parent) = std::path::Path::new(dest).parent() {
            if !parent.as_os_str().is_empty() {
                let parent_s = parent.to_string_lossy();
                let _ =
                    self.run_success(&["exec", "--user", "root", name, "mkdir", "-p", &parent_s]);
            }
        }
        self.run_success(&["cp", &src_s, &target])
            .with_context(|| format!("copying {} into {target}", src.display()))?;
        // `docker cp` preserves source uid/gid numerically; chown to dev as root.
        self.run_success(&["exec", "--user", "root", name, "chown", "dev:dev", dest])
            .with_context(|| format!("chown {dest} to dev"))?;
        Ok(())
    }
    fn seed_dir(&self, name: &str, src_dir: &Path, dest_dir: &str) -> Result<()> {
        // `docker cp <src>/. <dest>` copies the directory's CONTENTS (preserving
        // the tree, e.g. .config/nvim) into dest_dir; then chown -R to dev.
        let src_s = format!("{}/.", src_dir.display());
        let target = format!("{name}:{dest_dir}");
        self.run_success(&["cp", &src_s, &target])
            .with_context(|| format!("copying {} into {target}", src_dir.display()))?;
        self.run_success(&[
            "exec", "--user", "root", name, "chown", "-R", "dev:dev", dest_dir,
        ])
        .with_context(|| format!("chown -R {dest_dir} to dev"))?;
        Ok(())
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
    fn inspect_format(&self, name: &str, format: &str) -> Result<String> {
        self.run_capture(&["inspect", "--type", "container", "--format", format, name])
    }
    fn pull_image(&self, image: &str) -> Result<()> {
        // Stream pull output to the user's terminal.
        let status = self
            .cmd()
            .args(["pull", image])
            .status()
            .with_context(|| format!("pulling image {image}"))?;
        if !status.success() {
            bail!("pull of {image} failed (exit {:?})", status.code());
        }
        Ok(())
    }
    fn run_forwarder(
        &self,
        name: &str,
        network: &str,
        host_port: u16,
        target: &str,
        target_port: u16,
    ) -> Result<()> {
        let publish = format!("127.0.0.1:{host_port}:{host_port}");
        let listen = format!("TCP-LISTEN:{host_port},fork,reuseaddr");
        let connect = format!("TCP:{target}:{target_port}");
        // Foreground + attached: this call BLOCKS until the user interrupts.
        // Ctrl-C is delivered to the whole process group; `docker run` catches
        // it, stops the container, and `--rm` removes it — cleanup is robust
        // even if this process is killed before returning.
        let status = self
            .cmd()
            .args([
                "run",
                "--rm",
                "--name",
                name,
                "--network",
                network,
                "--entrypoint",
                "socat",
                "-p",
                &publish,
                "alpine/socat",
                &listen,
                &connect,
            ])
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()?;
        // Exit 130 = SIGINT (Ctrl-C) — the normal way to stop the bridge.
        if !status.success() && status.code() != Some(130) {
            bail!("port forwarder exited with {:?}", status.code());
        }
        Ok(())
    }
    fn spawn_forwarder(
        &self,
        name: &str,
        network: &str,
        host_port: u16,
        target: &str,
        target_port: u16,
    ) -> Result<()> {
        let publish = format!("127.0.0.1:{host_port}:{host_port}");
        let listen = format!("TCP-LISTEN:{host_port},fork,reuseaddr");
        let connect = format!("TCP:{target}:{target_port}");
        let _child = self
            .cmd()
            .args([
                "run",
                "--rm",
                "--name",
                name,
                "--network",
                network,
                "--entrypoint",
                "socat",
                "-p",
                &publish,
                "alpine/socat",
                &listen,
                &connect,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("spawning forwarder {name}"))?;
        // Handle dropped on purpose: the forwarder runs in our process group, so
        // Ctrl-C stops it (`--rm` removes it); explicit cleanup is by name.
        Ok(())
    }
}

/// Helper exposed to the `image` module: build `tag` from a context dir + Dockerfile path,
/// inheriting the user's terminal so build logs stream.
pub fn build_image_at(
    engine: &dyn Engine,
    tag: &str,
    context_dir: &Path,
    dockerfile: &Path,
) -> Result<()> {
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
