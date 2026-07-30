//! High-level workspace orchestration: composes engine + config + naming.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

use crate::config::{self, ImportSrc, WorkspaceConfig};
use crate::engine::{ContainerState, Engine, RunOpts};
use crate::image;
use crate::naming;

pub struct Workspace {
    pub cfg: WorkspaceConfig,
    engine: Box<dyn Engine>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceStatus {
    pub name: String,
    pub state: ContainerState,
    /// True iff the container is up and its in-container tmux session `work`
    /// exists (shown as the SESSION column in `work ls`).
    pub session_live: bool,
}

impl Workspace {
    pub fn engine(&self) -> &dyn Engine {
        &*self.engine
    }

    /// Load an existing workspace by name.
    pub fn open(name: &str) -> Result<Self> {
        naming::validate_name(name)?;
        let cfg = config::load_workspace(name)?;
        let engine = crate::engine::detect()?;
        Ok(Self { cfg, engine })
    }

    /// `work new <ws>`: create volume + network + container, persist config,
    /// and (optionally) seed shell/tmux configs for familiarity.
    pub fn create(
        name: &str,
        image: Option<String>,
        git_name: Option<String>,
        git_email: Option<String>,
        import_shell: Option<ImportSrc>,
        import_tmux: Option<ImportSrc>,
    ) -> Result<Self> {
        naming::validate_name(name)?;
        if config::workspace_exists(name) {
            bail!("workspace '{name}' already exists");
        }
        let engine = crate::engine::detect()?;
        if !engine.is_running()? {
            bail!(
                "container engine '{}' is not running; start OrbStack/Docker first",
                engine.binary()
            );
        }

        let global = config::load_global()?;
        let image = image.unwrap_or_else(|| global.effective_default_image().to_string());

        // Ensure the image exists: build the default, or pull a custom one.
        ensure_image(&*engine, &image)?;

        // Familiarity: resolve + validate import sources BEFORE creating any
        // resources, so a bad path fails fast with no orphaned volume/net/container.
        let shell = config::detect_shell();
        let rc = config::rc_name(&shell);
        let seeds: Vec<(std::path::PathBuf, String, &str)> = [
            resolve_import(import_shell, global.import_shell_config.as_deref())
                .map(|s| (s.to_path(rc), format!("/home/dev/{rc}"), "shell")),
            resolve_import(import_tmux, global.import_tmux_config.as_deref()).map(|s| {
                (
                    s.to_path(".tmux.conf"),
                    "/home/dev/.tmux.conf".into(),
                    "tmux",
                )
            }),
        ]
        .into_iter()
        .flatten()
        .collect();
        for (src, _dest, kind) in &seeds {
            if !src.exists() {
                bail!(
                    "{kind} config not found at {}; pass an explicit path (e.g. --import-{kind}-config <path>)",
                    src.display()
                );
            }
        }

        let vol = naming::volume(name);
        let net = naming::network(name);
        let ctr = naming::container(name);
        if !engine.volume_exists(&vol)? {
            engine.create_volume(&vol)?;
        }
        if !engine.network_exists(&net)? {
            engine.create_network(&net)?;
        }
        // Recreate container if a stale one lingers.
        if engine.container_exists(&ctr)? {
            engine.remove_container(&ctr)?;
        }
        let opts = run_opts(name, &image);
        engine.run(&opts)?;

        // Seed validated configs (verbatim, warned) + suppress the shell's
        // first-run prompt (the named volume overlays the image's /home/dev).
        for (src, dest, kind) in &seeds {
            seed_into(&*engine, &ctr, src, dest, kind, name)?;
        }
        ensure_rc_present(&*engine, &ctr, rc)?;

        let cfg = WorkspaceConfig {
            name: name.to_string(),
            image,
            git_name: git_name.clone(),
            git_email: git_email.clone(),
            shell: Some(shell),
            created_at: now_rfc3339(),
        };
        config::save_workspace(&cfg)?;

        let ws = Self { cfg, engine };
        ws.apply_git_identity()?;
        Ok(ws)
    }

    /// Ensure the container exists and is running.
    pub fn ensure_running(&self) -> Result<()> {
        let ctr = naming::container(&self.cfg.name);
        match self.engine.container_state(&ctr)? {
            ContainerState::Missing => {
                // Recreate from config (e.g. container removed manually).
                let opts = run_opts(&self.cfg.name, &self.cfg.image);
                self.engine.run(&opts)?;
            }
            ContainerState::Stopped => {
                self.engine.start_container(&ctr)?;
            }
            ContainerState::Running => {}
        }
        Ok(())
    }

    pub fn start(&self) -> Result<()> {
        self.ensure_running()
    }

    pub fn stop(&self) -> Result<()> {
        let ctr = naming::container(&self.cfg.name);
        match self.engine.container_state(&ctr)? {
            ContainerState::Missing => {
                bail!("workspace '{}' has no container to stop", self.cfg.name)
            }
            ContainerState::Stopped => {}
            ContainerState::Running => {
                self.engine.stop_container(&ctr)?;
            }
        }
        Ok(())
    }

    /// `work <ws>`: ensure running, then attach-or-create the in-container tmux
    /// session named after the workspace. Prints an identity banner and sets the
    /// terminal title first. The session (and anything started inside it) survives
    /// detach / closing the terminal; it does NOT survive `work stop`.
    pub fn shell(&self) -> Result<()> {
        self.ensure_running()?;
        let ctr = naming::container(&self.cfg.name);
        let shell = self.cfg.shell.as_deref().unwrap_or("zsh");
        let session = naming::session(&self.cfg.name);

        // Banner + detach hint (suppressed in cockpit windows).
        if std::env::var_os("WORK_COCKPIT").is_none() {
            let show = config::load_global().map(|g| g.show_banner).unwrap_or(true);
            if show {
                self.print_banner(&ctr);
            }
            println!("Ctrl-b d or close terminal = detach (keeps running) · exit = close session");
        }

        // Lossless one-time migration: rename a stale `work` session to <ws> in
        // place (running shells/agents inside it survive). No-op once renamed, or
        // for a workspace literally named "work".
        if session != "work"
            && self.engine.session_exists(&ctr, "work").unwrap_or(false)
            && !self.engine.session_exists(&ctr, session).unwrap_or(false)
        {
            let _ = self
                .engine
                .exec_capture(&ctr, &["tmux", "rename-session", "-t", "work", session]);
        }

        // Name the terminal tab (best-effort). The tmux window name is set via -n.
        {
            use std::io::Write;
            print!("\x1b]0;work:{}\x07", self.cfg.name);
            let _ = std::io::stdout().flush();
        }

        self.engine.exec_interactive(
            &ctr,
            &[
                "tmux",
                "new-session",
                "-A",
                "-s",
                session,
                "-n",
                session,
                "--",
                shell,
                "-l",
            ],
        )
    }

    /// Gather hostname/OS/git-branch via one `docker exec` and print the banner.
    /// Fail-soft: any error renders the dynamic fields as "—".
    fn print_banner(&self, ctr: &str) {
        let probe = "h=$(hostname 2>/dev/null); . /etc/os-release 2>/dev/null; s=${PRETTY_NAME:-}; g=$(git -C /home/dev rev-parse --abbrev-ref HEAD 2>/dev/null || true); printf '%s\\t%s\\t%s' \"$h\" \"$s\" \"$g\"";
        let gathered = self
            .engine
            .exec_capture(ctr, &["bash", "-c", probe])
            .unwrap_or_default();
        let mut parts = gathered.splitn(3, '\t');
        let hostname = parts.next().filter(|s| !s.is_empty()).unwrap_or("—");
        let system = parts.next().filter(|s| !s.is_empty()).unwrap_or("—");
        let git = parts.next().filter(|s| !s.is_empty()).unwrap_or("—");
        println!(
            "{}",
            crate::banner::compose(&self.cfg.name, &self.cfg.image, system, hostname, git)
        );
    }

    pub fn status(&self) -> Result<WorkspaceStatus> {
        let ctr = naming::container(&self.cfg.name);
        let state = self.engine.container_state(&ctr)?;
        Ok(WorkspaceStatus {
            name: self.cfg.name.clone(),
            state,
            session_live: false,
        })
    }

    /// True iff the container is up AND its in-container tmux session `work`
    /// exists. Used by the destructive-op safety policy to decide whether a
    /// stop/rm/recreate would actually lose live work.
    pub fn has_live_session(&self) -> bool {
        let ctr = naming::container(&self.cfg.name);
        if !matches!(
            self.engine.container_state(&ctr),
            Ok(ContainerState::Running)
        ) {
            return false;
        }
        self.engine
            .session_exists(&ctr, naming::session(&self.cfg.name))
            .unwrap_or(false)
    }

    /// Apply optional git identity (user.name/user.email) inside the container.
    pub fn apply_git_identity(&self) -> Result<()> {
        let ctr = naming::container(&self.cfg.name);
        if let Some(name) = &self.cfg.git_name {
            let _ = self
                .engine
                .exec_capture(&ctr, &["git", "config", "--global", "user.name", name]);
        }
        if let Some(email) = &self.cfg.git_email {
            let _ = self
                .engine
                .exec_capture(&ctr, &["git", "config", "--global", "user.email", email]);
        }
        Ok(())
    }

    /// Recreate this workspace's container from its current config (keeps the
    /// volume + network). Used by `work config` when the image changes.
    pub fn recreate(&self) -> Result<()> {
        let ctr = naming::container(&self.cfg.name);
        if self.engine.container_exists(&ctr)? {
            self.engine.remove_container(&ctr)?;
        }
        ensure_image(&*self.engine, &self.cfg.image)?;
        let opts = run_opts(&self.cfg.name, &self.cfg.image);
        self.engine.run(&opts)?;
        self.apply_git_identity()?;
        Ok(())
    }

    /// `work rm <ws> [--purge]`: remove container + network + config.
    /// Keeps the named volume unless `purge` (data loss; the caller gates this
    /// via the destructive-op safety policy).
    pub fn remove(&self, purge: bool) -> Result<()> {
        let ctr = naming::container(&self.cfg.name);
        let net = naming::network(&self.cfg.name);
        let vol = naming::volume(&self.cfg.name);
        if self.engine.container_exists(&ctr)? {
            self.engine.remove_container(&ctr)?; // `rm -f` stops + removes
        }
        // The network may be held by a lingering forwarder (work-fwd-*); best-effort
        // + warn rather than failing this data-safe removal.
        if let Err(e) = self.engine.remove_network(&net) {
            eprintln!(
                "· could not remove network {net} ({e}); stop any `work fwd` for this workspace first"
            );
        }
        if purge && self.engine.volume_exists(&vol)? {
            self.engine.remove_volume(&vol)?;
        }
        let _ = std::fs::remove_file(config::workspace_config_path(&self.cfg.name));
        Ok(())
    }
}

/// `docker run` options for a workspace container. Sets the `WORK`/`WORKSPACE`
/// identity env so prompts, banners, and tools can name the workspace.
fn run_opts(name: &str, image: &str) -> RunOpts {
    RunOpts {
        name: naming::container(name),
        image: image.to_string(),
        network: naming::network(name),
        volume: naming::volume(name),
        volume_target: "/home/dev".into(),
        workdir: "/home/dev".into(),
        cmd: vec!["sleep".into(), "infinity".into()],
        env: vec![
            ("WORK".into(), name.into()),
            ("WORKSPACE".into(), name.into()),
        ],
    }
}

/// Build the default image or pull a custom one so it is locally present.
fn ensure_image(engine: &dyn Engine, image: &str) -> Result<()> {
    if engine.image_exists(image)? {
        return Ok(());
    }
    if image == config::DEFAULT_IMAGE {
        println!("image '{image}' not found; building it now…");
        image::build_default(engine)
    } else {
        println!("image '{image}' not found; pulling…");
        engine.pull_image(image)
    }
}

/// `work ls`: every workspace on disk with its container state + session liveness.
pub fn list_all() -> Result<Vec<WorkspaceStatus>> {
    let engine = crate::engine::detect()?;
    let mut out = Vec::new();
    for name in config::list_workspace_names()? {
        let ctr = naming::container(&name);
        let state = engine
            .container_state(&ctr)
            .unwrap_or(ContainerState::Missing);
        let session_live = state == ContainerState::Running
            && engine
                .session_exists(&ctr, naming::session(&name))
                .unwrap_or(false);
        out.push(WorkspaceStatus {
            name,
            state,
            session_live,
        });
    }
    Ok(out)
}

/// `work fwd <ws> <port>`: bridge `127.0.0.1:<port>` (host) to `<ws>:<port>`.
/// Runs the forwarder in the foreground on the workspace's dedicated network;
/// Ctrl-C lets `docker run` stop + `--rm` the container.
pub fn forward(name: &str, port: u16) -> Result<()> {
    let ws = Workspace::open(name)?;
    ws.ensure_running()?;
    let engine = ws.engine();
    let fwd_name = format!("work-fwd-{name}-{port}");
    if engine.container_exists(&fwd_name)? {
        engine.remove_container(&fwd_name)?;
    }
    println!("Forwarding http://127.0.0.1:{port} -> {name}:{port}");
    println!("(Ctrl-C to stop the bridge)");
    // Blocks until interrupted; cleanup is handled by `docker run --rm`.
    engine.run_forwarder(
        &fwd_name,
        &naming::network(name),
        port,
        &naming::container(name),
        port,
    )?;
    println!("bridge stopped");
    Ok(())
}

/// `work resume` (= `work all`): host tmux cockpit tiling every RUNNING
/// workspace's in-container session. Host prefix C-a (in-container is C-b).
/// Each window runs `work <ws>` — an isolated `docker exec` client into one
/// container on its own network. No path between containers is created.
pub fn resume() -> Result<()> {
    if !which_host("tmux") {
        bail!("host `tmux` not found; install it (`brew install tmux`) to use the cockpit");
    }
    let engine = crate::engine::detect()?;
    let mut running = Vec::new();
    let mut stopped = Vec::new();
    for name in config::list_workspace_names()? {
        let ctr = naming::container(&name);
        match engine
            .container_state(&ctr)
            .unwrap_or(ContainerState::Missing)
        {
            ContainerState::Running => running.push(name),
            _ => stopped.push(name),
        }
    }
    if running.is_empty() {
        if stopped.is_empty() {
            bail!("no workspaces yet; create one with `work new <ws>`");
        }
        bail!(
            "no running workspaces. Stopped: {}. Start one with `work start <ws>`",
            stopped.join(", ")
        );
    }

    // Fresh host session so the window set + prefix are deterministic.
    let _ = Command::new("tmux")
        .args(["kill-session", "-t", "work"])
        .status();
    let first = &running[0];
    let status = Command::new("tmux")
        .args([
            "new-session",
            "-d",
            "-s",
            "work",
            "-n",
            first,
            &cockpit_cmd(first),
        ])
        .status()?;
    if !status.success() {
        bail!("failed to create host tmux session 'work'");
    }
    // Host prefix C-a so it doesn't clash with the in-container C-b.
    let _ = Command::new("tmux")
        .args(["set-option", "-t", "work", "prefix", "C-a"])
        .status();
    for name in running.iter().skip(1) {
        let _ = Command::new("tmux")
            .args(["new-window", "-t", "work", "-n", name, &cockpit_cmd(name)])
            .status();
    }

    if !stopped.is_empty() {
        eprintln!(
            "stopped: {} — `work start <ws>` to include",
            stopped.join(", ")
        );
    }

    println!("cockpit: Ctrl-a = switch window / detach cockpit · inside a window: Ctrl-b d = detach one session");
    let attach = if std::env::var_os("TMUX").is_some() {
        Command::new("tmux")
            .args(["switch-client", "-t", "work"])
            .status()
    } else {
        Command::new("tmux")
            .args(["attach-session", "-t", "work"])
            .status()
    };
    if let Err(e) = attach {
        bail!("failed to attach to tmux session 'work': {e}");
    }
    Ok(())
}

/// Cockpit window command: run `work <ws>` with the per-window hint suppressed.
fn cockpit_cmd(ws: &str) -> String {
    let bin = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "work".into());
    format!("WORK_COCKPIT=1 {bin} {ws}")
}

fn which_host(bin: &str) -> bool {
    Command::new(bin)
        .arg("-V")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Effective import source: a per-workspace flag overrides the global default.
fn resolve_import(flag: Option<ImportSrc>, global: Option<&Path>) -> Option<ImportSrc> {
    flag.or_else(|| global.map(|p| ImportSrc::Explicit(p.to_path_buf())))
}

/// Copy a host config file into the container volume (owned by dev), verbatim,
/// printing the secret warning. Errors clearly if the source is absent.
fn seed_into(
    engine: &dyn Engine,
    ctr: &str,
    src: &Path,
    dest: &str,
    kind: &str,
    ws: &str,
) -> Result<()> {
    if !src.exists() {
        bail!(
            "{kind} config not found at {}; pass an explicit path (e.g. --import-{kind}-config <path>)",
            src.display()
        );
    }
    engine
        .seed_file(ctr, src, dest)
        .with_context(|| format!("seeding {dest} from {}", src.display()))?;
    println!(
        "⚠  Copied your {kind} config into '{ws}'. Ensure it contains no secrets — it now lives in that workspace's volume."
    );
    Ok(())
}

/// Ensure `/home/dev/<rcname>` exists so the shell's first-run prompt never
/// fires (the named volume overlays the image's baked-in /home/dev). No-op if
/// the rc is already present (e.g. it was just seeded, or persisted in the vol).
fn ensure_rc_present(engine: &dyn Engine, ctr: &str, rcname: &str) -> Result<()> {
    let path = format!("/home/dev/{rcname}");
    if engine.exec_capture(ctr, &["test", "-e", &path]).is_err() {
        // Create an empty rc as the dev user (the container's default exec user).
        let _ = engine.exec_capture(ctr, &["touch", &path]);
    }
    Ok(())
}

fn now_rfc3339() -> String {
    use chrono::Utc;
    Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_opts_sets_identity_env_and_names() {
        let opts = run_opts("acme", "work-base:latest");
        assert_eq!(opts.name, "work-acme");
        assert_eq!(opts.network, "work-net-acme");
        assert_eq!(opts.volume, "work-acme-home");
        assert_eq!(
            opts.env,
            vec![
                ("WORK".to_string(), "acme".to_string()),
                ("WORKSPACE".to_string(), "acme".to_string()),
            ]
        );
    }
}
