//! Container-engine abstraction. One trait, one CLI adapter.
//!
//! The `docker` CLI is the common substrate: OrbStack, Docker, and Colima all
//! expose it; Podman is CLI-compatible (`podman` binary, identical verbs).

use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::naming;
use anyhow::{bail, Context, Result};
use serde::Deserialize;

/// Forwarder relay image pinned to a registry digest (supply-chain hardening):
/// `alpine/socat`. Pinned so a re-pointed tag or registry compromise can't swap
/// the bridge OAuth callback ports flow through.
const FORWARDER_IMAGE: &str =
    "alpine/socat@sha256:e7b17711daaa7d49107a7193112689e91fb1a27bddd9cb0b32641b55b8e9e3b0";

/// The `--label` value marking a work-managed object (`dev.work-cli.managed=true`).
fn managed_label() -> String {
    format!("{}=true", naming::LABEL_KEY)
}

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

/// Container hardening applied to every workspace (and forwarder) `docker run`.
///
/// The security defaults that cost nothing real workflows need — cap-drop ALL,
/// no-new-privileges, a pids limit — ship ON and are appended additively in
/// `Engine::run`. `cap_add`/`memory`/`cpus` are operational knobs (empty/None by
/// default); fixed memory/CPU ceilings are policy, not a security default, and
/// would break large monorepo/cargo builds, so they stay opt-in. `read_only_rootfs`
/// is OFF by default because the browser shim writes system paths.
#[derive(Debug, Clone)]
pub struct HardenOpts {
    /// Capabilities added back on top of cap-drop ALL. Empty by default.
    pub cap_add: Vec<String>,
    /// `--memory` (also sets `--memory-swap`). `None` = engine default.
    pub memory: Option<String>,
    /// `--cpus`. `None` = engine default.
    pub cpus: Option<String>,
    /// `--pids-limit`. `Some(4096)` by default — bounds fork bombs while leaving
    /// headroom for heavy parallel builds. (`pids.max` counts THREADS as well as
    /// processes, so a tight cap breaks cargo/LLVM/linkers/node/rust-analyzer.)
    pub pids_limit: Option<u32>,
    /// `--read-only` rootfs. OFF by default (shim writes `/usr/local/bin` etc.).
    pub read_only_rootfs: bool,
    /// `--tmpfs path:opts` mounts for writable scratch under a read-only rootfs.
    pub tmpfs_mounts: Vec<(String, String)>,
}

impl Default for HardenOpts {
    fn default() -> Self {
        Self {
            cap_add: Vec::new(),
            memory: None,
            cpus: None,
            pids_limit: Some(4096),
            read_only_rootfs: false,
            tmpfs_mounts: Vec::new(),
        }
    }
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
    /// Hardening flags `Engine::run` appends (cap-drop ALL, no-new-privileges,
    /// pids limit, optional memory/cpus/read-only/tmpfs) + the managed label.
    pub harden: HardenOpts,
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
    /// True iff container `name` has a live in-container multiplexer runtime
    /// (the headless herdr server). Returns `false` (NOT an error) when the
    /// container is missing/stopped or the server is absent — so `ls`/`stop`/
    /// `rm` never choke on a downed box.
    fn runtime_up(&self, name: &str) -> Result<bool>;
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
    /// Stable identity of the active daemon (`docker info --format {{.ID}}`),
    /// so a workspace created on one daemon refuses to talk to another.
    fn daemon_id(&self) -> Result<String>;
    /// True iff object `name` of `kind` (`"volume"`/`"network"`/`"container"`)
    /// carries label `key` — used to refuse reusing a same-named object we
    /// didn't create.
    fn object_has_label(&self, name: &str, kind: &str, key: &str) -> Result<bool>;
    /// Resolved image ID (`docker image inspect --format {{.Id}} <image>`),
    /// recorded at create/recreate so `doctor` can flag a drifted image.
    fn image_id(&self, image: &str) -> Result<String>;
    /// All container names on the daemon (`docker ps -a --format {{.Names}}`),
    /// so `doctor` can surface forwarder containers / orphans.
    fn list_containers(&self) -> Result<Vec<String>>;

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
    // A binary that spawns but exits non-zero (broken install, blocked by
    // policy) is NOT "present": only a successful `--version` counts, so
    // `detect()` never picks a half-installed runtime.
    Command::new(bin)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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
        let label = managed_label();
        self.run_success(&["volume", "create", "--label", &label, name])
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
        let label = managed_label();
        self.run_success(&["network", "create", "--label", &label, name])
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
        // `docker inspect` exits non-zero when the container is absent — that is
        // `Missing`. A *different* failure (daemon down, auth, malformed name)
        // must NOT collapse into `Missing`, or `has_live_session`/`doctor`/
        // `ensure_running` would treat a down daemon as "no container" and fail
        // open. Inspect stderr to tell them apart (case-insensitive: Docker says
        // "No such container", Podman "no such container").
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
            let stderr = String::from_utf8_lossy(&out.stderr);
            let lower = stderr.to_ascii_lowercase();
            if lower.contains("no such container") || lower.contains("no such object") {
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
        // Hardening (HardenOpts) + the managed label, appended after identity
        // args so they always apply. cap-drop ALL + no-new-privileges + a pids
        // limit cost nothing real shells/builds need; memory/cpus/read-only/tmpfs
        // are opt-in. See `HardenOpts`.
        c.arg("--label").arg(managed_label());
        c.args(["--cap-drop", "ALL", "--security-opt", "no-new-privileges"]);
        for cap in &opts.harden.cap_add {
            c.args(["--cap-add", cap.as_str()]);
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
            let spec = format!("{path}:{mo}");
            c.args(["--tmpfs", spec.as_str()]);
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
        let mut c = self.cmd();
        c.arg("exec");
        // Forward terminal capabilities into the exec so TUI apps inside the
        // container (Claude Code, omp, …) detect the host's real color depth
        // and render Nerd Font glyphs. `docker exec` does not propagate the
        // host environment: without COLORTERM apps degrade below truecolor,
        // and without NERD_FONTS=1 agents like omp fall back to ASCII glyphs
        // because the in-container multiplexer hides the host terminal from them.
        for var in TERMINAL_ENV_TO_FORWARD {
            if let Ok(val) = std::env::var(var) {
                c.arg("-e").arg(format!("{var}={val}"));
            }
        }
        for (var, val) in TERMINAL_ENV_HARDCODED {
            c.arg("-e").arg(format!("{var}={val}"));
        }
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
    fn runtime_up(&self, name: &str) -> Result<bool> {
        // `herdr status server` reports liveness on stdout and exits 0 whether
        // the server is up or not, so parse the output rather than the exit
        // code. Any failure (container missing/stopped, herdr absent) -> false,
        // never an error, so `ls`/`stop`/`rm` never choke on a downed box.
        let out = self
            .cmd()
            .args(["exec", name, "herdr", "status", "server"])
            .output()?;
        if !out.status.success() {
            return Ok(false);
        }
        Ok(String::from_utf8_lossy(&out.stdout).contains("status: running"))
    }
    fn seed_file(&self, name: &str, src: &Path, dest: &str) -> Result<()> {
        let src_s = src.to_string_lossy();
        let target = format!("{name}:{dest}");
        // Ensure dest's parent dir exists (e.g. /home/dev/.config for starship.toml);
        // `docker cp` won't create intermediate dirs. Idempotent — /home/dev exists
        // for rc seeds, so this is a no-op there.
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
    fn daemon_id(&self) -> Result<String> {
        self.run_capture(&["info", "--format", "{{.ID}}"])
    }
    fn object_has_label(&self, name: &str, kind: &str, key: &str) -> Result<bool> {
        // Containers expose labels at .Config.Labels; volumes/networks at .Labels.
        let value = if kind == "container" {
            let fmt = format!("{{{{index .Config.Labels \"{key}\"}}}}");
            self.run_capture(&["inspect", "--type", "container", "--format", &fmt, name])?
        } else {
            let fmt = format!("{{{{index .Labels \"{key}\"}}}}");
            self.run_capture(&[kind, "inspect", "--format", &fmt, name])?
        };
        Ok(value.trim() == "true")
    }
    fn image_id(&self, image: &str) -> Result<String> {
        self.run_capture(&["image", "inspect", "--format", "{{.Id}}", image])
    }
    fn list_containers(&self) -> Result<Vec<String>> {
        let out = self.run_capture(&["ps", "-a", "--format", "{{.Names}}"])?;
        Ok(out.lines().map(str::to_string).collect())
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
        let label = managed_label();
        // Foreground + attached: this call BLOCKS until the user interrupts.
        // Ctrl-C is delivered to the whole process group; `docker run` catches
        // it, stops the container, and `--rm` removes it — cleanup is robust
        // even if this process is killed before returning. Hardened: cap-drop
        // ALL + no-new-privileges + managed label + a digest-pinned image, so a
        // relay bridging the workspace network is as locked down as the workspace.
        let status = self
            .cmd()
            .args([
                "run",
                "--rm",
                "--name",
                name,
                "--network",
                network,
                "--label",
                &label,
                "--cap-drop",
                "ALL",
                "--security-opt",
                "no-new-privileges",
                "--entrypoint",
                "socat",
                "-p",
                &publish,
                FORWARDER_IMAGE,
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
        let label = managed_label();
        let _child = self
            .cmd()
            .args([
                "run",
                "--rm",
                "--name",
                name,
                "--network",
                network,
                "--label",
                &label,
                "--cap-drop",
                "ALL",
                "--security-opt",
                "no-new-privileges",
                "--entrypoint",
                "socat",
                "-p",
                &publish,
                FORWARDER_IMAGE,
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

/// Host terminal env forwarded into a workspace `exec` so in-container TUI apps
/// (Claude Code, omp, …) detect the host's real color depth. `docker exec`
/// does not propagate the host environment, and `COLORTERM` is the de-facto
/// signal apps check for truecolor — without it they degrade below what the
/// terminal can render. (TERM/TERM_PROGRAM are set by the multiplexer itself, not us.)
const TERMINAL_ENV_TO_FORWARD: &[&str] = &["COLORTERM"];
/// Env vars to unconditionally inject into every workspace `exec` (not copied
/// from the host). `NERD_FONTS=1` forces Nerd Font glyph rendering in agents
/// like omp, whose auto-detection fails inside the in-container multiplexer.
/// Belt-and-suspenders: the same value is baked at `docker run` time (see
/// `workspace::run_opts`), but forwarding it on every `exec` also covers
/// containers created before that fix landed.
const TERMINAL_ENV_HARDCODED: &[(&str, &str)] = &[("NERD_FONTS", "1")];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_env_forward_list_advertises_truecolor() {
        // COLORTERM is the de-facto signal apps (supports-color, etc.) check
        // for truecolor; it must be forwarded or they degrade below the
        // terminal's real capability.
        assert!(TERMINAL_ENV_TO_FORWARD.contains(&"COLORTERM"));
    }

    #[test]
    fn terminal_env_hardcoded_forces_nerd_fonts() {
        // NERD_FONTS=1 must be injected on every exec so agents like omp
        // render Nerd Font glyphs even when the in-container multiplexer hides the host
        // terminal from their auto-detection.
        assert!(TERMINAL_ENV_HARDCODED.contains(&("NERD_FONTS", "1")));
    }
}
