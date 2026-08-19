//! Container-engine abstraction. One trait, one CLI adapter.
//!
//! Podman is preferred when available. Docker, OrbStack, and Colima remain
//! supported through their Docker-compatible CLI; Podman uses the same verbs
//! through its `podman` binary.

use std::collections::BTreeSet;
use std::fs::File;
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

/// Container hardening applied to every workspace (and forwarder) `run`.
///
/// Workspace hardening: a pids limit + the managed label always ship, appended
/// additively in `Engine::run`. cap-drop ALL / no-new-privileges are deliberately
/// NOT set — they block `sudo`/setuid, which workspaces need to install tools at
/// runtime. `cap_add`/`memory`/`cpus` are operational knobs (empty/None by
/// default); fixed memory/CPU ceilings are policy, not a security default, and
/// would break large monorepo/cargo builds, so they stay opt-in. `read_only_rootfs`
/// is OFF by default because the browser shim writes system paths.
#[derive(Debug, Clone)]
pub struct HardenOpts {
    /// Capabilities to add beyond the engine default. Empty by default.
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

/// `run` options for a workspace container.
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
    /// Hardening flags `Engine::run` appends (pids limit, the managed label,
    /// optional cap_add/memory/cpus/read-only/tmpfs).
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

    /// Interactive exec — inherits the calling process's stdio/tty. `shell` is
    /// the resolved shell basename; injected as `$SHELL` so herdr spawns the
    /// correct pane shell — and so containers created before this env was set
    /// at container creation time still heal on attach.
    fn exec_interactive(&self, name: &str, cmd: &[&str], shell: &str) -> Result<()>;
    /// Non-interactive exec — captures stdout.
    fn exec_capture(&self, name: &str, cmd: &[&str]) -> Result<String>;
    /// `exec --user root <name> <cmd...>` (non-interactive), require
    /// success. For one-off system setup touching root-owned paths the `dev`
    /// user can't write (e.g. installing the shim under `/usr/local/bin`).
    /// Workspace containers are not cap-dropped, so this runs as full root.
    fn exec_root(&self, name: &str, cmd: &[&str]) -> Result<()>;
    /// True iff container `name` has a live in-container multiplexer runtime
    /// (the headless herdr server). Returns `false` (NOT an error) when the
    /// container is missing/stopped or the server is absent — so `ls`/`stop`/
    /// `rm` never choke on a downed box.
    fn runtime_up(&self, name: &str) -> Result<bool>;
    /// Install host file `src` into container `name` at `dest`, owned by `dev`.
    /// Streams the bytes through `tee` as the `dev` user, so the file is born
    /// dev-owned — no separate `chown` step. See `seed_dir`.
    fn seed_file(&self, name: &str, src: &Path, dest: &str) -> Result<()>;
    /// Recursively copy a host directory's *contents* into `dest_dir`, owned by
    /// `dev`. Streams a tarball extracted as `dev` — files are born dev-owned,
    /// so no `chown` is ever needed. Mirrors `seed_file` for a tree.
    fn seed_dir(&self, name: &str, src_dir: &Path, dest_dir: &str) -> Result<()>;

    fn image_exists(&self, image: &str) -> Result<bool>;

    /// Inspect a container's networks -> set of network names.
    fn container_networks(&self, name: &str) -> Result<BTreeSet<String>>;
    /// Inspect a container's mounts -> list of (source_name_or_path, destination).
    fn container_mounts(&self, name: &str) -> Result<Vec<(String, String)>>;
    /// Generic container `inspect --format` (Go template).
    /// Used by `doctor` for restart-policy / user / image / port checks.
    fn inspect_format(&self, name: &str, format: &str) -> Result<String>;
    /// Stable identity of the active daemon or Podman store,
    /// so a workspace created on one daemon refuses to talk to another.
    fn daemon_id(&self) -> Result<String>;
    /// True iff object `name` of `kind` (`"volume"`/`"network"`/`"container"`)
    /// carries label `key` — used to refuse reusing a same-named object we
    /// didn't create.
    fn object_has_label(&self, name: &str, kind: &str, key: &str) -> Result<bool>;
    /// Resolved image ID (`image inspect --format {{.Id}} <image>`),
    /// recorded at create/recreate so `doctor` can flag a drifted image.
    fn image_id(&self, image: &str) -> Result<String>;
    /// All container names on the daemon (`ps -a --format {{.Names}}`),
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

// ---------- PURE selection and platform helpers ----------

const SUPPORTED_ENGINE_NAMES: &str = "podman, docker, orbstack, colima";
const WINDOWS_WSL_ONLY: &str =
    "Windows support is WSL-only; run work from inside a WSL distribution.";

/// Parse an optional `WORK_ENGINE` value.
///
/// An unset or blank value means automatic detection. Non-blank values are
/// intentionally limited to the four engine names understood by `detect()` so
/// a typo cannot silently change the selected runtime.
pub fn parse_engine_override(value: Option<&str>) -> Result<Option<EngineKind>> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let value = raw.trim();
    if value.is_empty() {
        return Ok(None);
    }

    let kind = match value.to_ascii_lowercase().as_str() {
        "podman" => EngineKind::Podman,
        "docker" => EngineKind::Docker,
        "orbstack" => EngineKind::OrbStack,
        "colima" => EngineKind::Colima,
        _ => bail!("invalid WORK_ENGINE value '{raw}'; expected one of: {SUPPORTED_ENGINE_NAMES}"),
    };
    Ok(Some(kind))
}

/// Return the Windows support explanation for a target OS, if applicable.
///
/// WSL processes target Linux, so native Windows is the only target rejected
/// here. Keeping this helper pure makes the policy testable without changing
/// the process environment or compiling the crate for Windows.
pub fn unsupported_platform_message(target_os: &str) -> Option<&'static str> {
    (target_os == "windows").then_some(WINDOWS_WSL_ONLY)
}

/// Detect WSL from the Linux kernel strings exposed by `/proc`.
pub fn is_wsl_kernel(os_release: &str, proc_version: &str) -> bool {
    let release = os_release.to_ascii_lowercase();
    let version = proc_version.to_ascii_lowercase();
    release.contains("microsoft")
        || release.contains("wsl")
        || version.contains("microsoft")
        || version.contains("wsl")
}

fn host_is_wsl() -> bool {
    #[cfg(target_os = "linux")]
    {
        let release = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
        let version = std::fs::read_to_string("/proc/version").unwrap_or_default();
        is_wsl_kernel(&release, &version)
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Given which runtimes are available, pick the platform-neutral order.
///
/// Podman is preferred, followed by Docker-compatible OrbStack and Colima,
/// then the generic Docker CLI.
pub fn pick_kind(orb: bool, docker: bool, podman: bool, colima: bool) -> Option<EngineKind> {
    if podman {
        Some(EngineKind::Podman)
    } else if orb {
        Some(EngineKind::OrbStack)
    } else if colima {
        Some(EngineKind::Colima)
    } else if docker {
        Some(EngineKind::Docker)
    } else {
        None
    }
}

fn kind_available(kind: EngineKind, orb: bool, docker: bool, podman: bool, colima: bool) -> bool {
    match kind {
        EngineKind::OrbStack => orb,
        EngineKind::Docker => docker,
        EngineKind::Podman => podman,
        EngineKind::Colima => colima,
    }
}

/// Apply an explicit override when present, otherwise use automatic selection.
pub fn select_kind(
    override_kind: Option<EngineKind>,
    orb: bool,
    docker: bool,
    podman: bool,
    colima: bool,
) -> Result<EngineKind> {
    if let Some(kind) = override_kind {
        if !kind_available(kind, orb, docker, podman, colima) {
            bail!(
                "WORK_ENGINE={} requested, but that engine is not available; install it or unset WORK_ENGINE",
                kind.as_str()
            );
        }
        return Ok(kind);
    }

    pick_kind(orb, docker, podman, colima).ok_or_else(|| {
        anyhow::anyhow!(
            "no supported container engine found. Install one of: {SUPPORTED_ENGINE_NAMES}."
        )
    })
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
    if let Some(message) = unsupported_platform_message(std::env::consts::OS) {
        bail!(message);
    }

    let override_value = match std::env::var("WORK_ENGINE") {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            bail!("WORK_ENGINE must be valid UTF-8; expected one of: {SUPPORTED_ENGINE_NAMES}")
        }
    };
    let override_kind = parse_engine_override(override_value.as_deref())?;

    let docker = which("docker");
    // OrbStack and Colima both drive the Docker CLI; the app/runtime marker
    // alone is not enough if the compatible CLI is missing from PATH.
    let orb = docker && orbstack_present();
    let podman = which("podman");
    let colima = docker && which("colima");

    let kind = select_kind(override_kind, orb, docker, podman, colima).map_err(|error| {
        if host_is_wsl() {
            error.context("WSL detected; install a supported engine inside this WSL distribution")
        } else {
            error
        }
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

// ---------- CLI adapter ----------

/// CLI adapter for Docker-compatible engines and Podman.
///
/// The `DockerCli` name is retained to keep the existing public surface stable;
/// its binary is selected at construction time.
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
        // Capture stderr (not just the status) so a failing CLI verb reports
        // its cause — e.g. a "no such container" would otherwise surface as a
        // bare "failed (exit 1)" with nothing to act on.
        let out = self
            .cmd()
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .with_context(|| format!("spawning '{} {}'", self.binary, args.join(" ")))?;
        if !out.status.success() {
            bail!(
                "'{} {}' failed (exit {:?}): {}",
                self.binary,
                args.join(" "),
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).trim(),
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
        // `inspect` exits non-zero when the container is absent — that is
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
        // Managed label + pids limit always apply (appended after identity args).
        // cap-drop ALL / no-new-privileges are deliberately NOT set here: they
        // block `sudo`/setuid, which workspaces need to install tools at runtime.
        // Forwarders (run_forwarder/spawn_forwarder) stay locked down — they never
        // run a user shell. See `HardenOpts` for the opt-in knobs.
        c.arg("--label").arg(managed_label());
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
        // the CLI's stderr if creation fails.
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
        // Forward terminal capabilities into the exec so TUI apps inside the
        // container (Claude Code, omp, …) detect the host's real color depth
        // and render Nerd Font glyphs. `exec` does not propagate the
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
        // `$SHELL` selects herdr's pane shell (its config: "empty means $SHELL,
        // then /bin/sh"). Inject it on the exec so workspaces created before
        // this env was set at container creation time still spawn the right shell —
        // mirrors the NERD_FONTS injection above.
        c.arg("-e")
            .arg(format!("SHELL={}", crate::config::shell_path(shell)));
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
        // Install `src` at `dest` AS THE DEV USER by streaming its bytes through
        // `tee`, so the file is born dev-owned — no separate `chown` step needed
        // (`cp` would preserve the host uid and force a follow-up chown).
        // `tee` writes 0666 & ~umask
        // (0644 under the image's 022) — correct for the config files this seeds;
        // it carries no exec bit, but none of these seeds needs one.
        if let Some(parent) = Path::new(dest).parent() {
            if !parent.as_os_str().is_empty() {
                let parent_s = parent.to_string_lossy();
                self.run_success(&["exec", "--user", "dev", name, "mkdir", "-p", &parent_s])?;
            }
        }
        let file = File::open(src).with_context(|| format!("opening {}", src.display()))?;
        let out = self
            .cmd()
            .args(["exec", "--user", "dev", "-i", name, "tee", dest])
            .stdin(Stdio::from(file))
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .with_context(|| format!("installing {dest} into {name}"))?;
        if !out.status.success() {
            bail!(
                "installing {dest} into {name} failed (exit {:?}): {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).trim(),
            );
        }
        Ok(())
    }
    fn seed_dir(&self, name: &str, src_dir: &Path, dest_dir: &str) -> Result<()> {
        // Stream a tarball of `src_dir`'s contents into the container and extract
        // AS THE DEV USER, so every file is born dev-owned with its mode preserved
        // — no separate `chown` step. (`cp` would preserve the host uid for
        // the whole tree, forcing a follow-up chown.) `--no-same-owner` skips GNU
        // tar's (impossible as non-root)
        // ownership step cleanly; COPYFILE_DISABLE keeps macOS bsdtar from
        // embedding `._` AppleDouble members into the stream.
        let mut tar = Command::new("tar");
        tar.env("COPYFILE_DISABLE", "1")
            .arg("-C")
            .arg(src_dir)
            .args(["-cf", "-", "."])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut tar_child = tar
            .spawn()
            .with_context(|| format!("spawning tar for {}", src_dir.display()))?;
        let tar_stdout = tar_child.stdout.take().unwrap();
        let out = self
            .cmd()
            .args([
                "exec",
                "--user",
                "dev",
                "-i",
                name,
                "tar",
                "-C",
                dest_dir,
                "--no-same-owner",
                "-xf",
                "-",
            ])
            .stdin(Stdio::from(tar_stdout))
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .with_context(|| format!("extracting dotfiles into {name}:{dest_dir}"))?;
        let tar_status = tar_child.wait().ok();
        if !out.status.success() {
            bail!(
                "seeding {} into {}:{} failed (exit {:?}): {}",
                src_dir.display(),
                name,
                dest_dir,
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).trim(),
            );
        }
        // tar can only fail downstream (SIGPIPE, exit 141) if the CLI closed its
        // stdin early — the real cause is reported above — so flag only a
        // self-contained tar failure.
        if let Some(ts) = tar_status {
            if !ts.success() && ts.code() != Some(141) {
                bail!(
                    "archiving {} failed (exit {:?})",
                    src_dir.display(),
                    ts.code()
                );
            }
        }
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
        if self.kind == EngineKind::Podman {
            // Podman has no Docker-compatible top-level `.ID` in `info`.
            // These host/store paths identify the active local or remote
            // Podman context while staying within the CLI interface.
            let id = self.run_capture(&[
                "info",
                "--format",
                "{{.Host.RemoteSocket.Path}}|{{.Store.GraphRoot}}|{{.Store.RunRoot}}",
            ])?;
            if !id.is_empty() && !id.split('|').any(|part| part == "<no value>") {
                return Ok(format!("podman:{id}"));
            }
        }
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
        // Ctrl-C is delivered to the whole process group; the CLI `run` catches
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
    let mut command = Command::new(engine.binary());
    command.args(["build", "-t", tag, "-f"]);
    // Rootless Podman can leave build containers without the host's normal
    // route to large release assets (while ordinary `podman run` networking
    // works). Host networking is scoped to the image-build phase only; runtime
    // workspace containers still use their dedicated Work network.
    if Path::new(engine.binary())
        .file_name()
        .is_some_and(|name| name == "podman")
    {
        command.arg("--network=host");
    }
    let status = command
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
/// (Claude Code, omp, …) detect the host's real color depth. `exec`
/// does not propagate the host environment, and `COLORTERM` is the de-facto
/// signal apps check for truecolor — without it they degrade below what the
/// terminal can render. (TERM/TERM_PROGRAM are set by the multiplexer itself, not us.)
const TERMINAL_ENV_TO_FORWARD: &[&str] = &["COLORTERM"];
/// Env vars to unconditionally inject into every workspace `exec` (not copied
/// from the host). `NERD_FONTS=1` forces Nerd Font glyph rendering in agents
/// like omp, whose auto-detection fails inside the in-container multiplexer.
/// Belt-and-suspenders: the same value is baked at container creation time (see
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
