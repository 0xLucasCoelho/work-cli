//! High-level workspace orchestration: composes engine + config + naming.

use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::config::{self, WorkspaceConfig};
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

    /// `work new <ws>`: create volume + network + container, persist config.
    pub fn create(
        name: &str,
        image: Option<String>,
        git_name: Option<String>,
        git_email: Option<String>,
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
        let opts = RunOpts {
            name: ctr.clone(),
            image: image.clone(),
            network: net.clone(),
            volume: vol.clone(),
            volume_target: "/home/dev".into(),
            workdir: "/home/dev".into(),
            cmd: vec!["sleep".into(), "infinity".into()],
        };
        engine.run(&opts)?;

        let cfg = WorkspaceConfig {
            name: name.to_string(),
            image,
            git_name: git_name.clone(),
            git_email: git_email.clone(),
            shell: None,
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
                let opts = RunOpts {
                    name: ctr.clone(),
                    image: self.cfg.image.clone(),
                    network: naming::network(&self.cfg.name),
                    volume: naming::volume(&self.cfg.name),
                    volume_target: "/home/dev".into(),
                    workdir: "/home/dev".into(),
                    cmd: vec!["sleep".into(), "infinity".into()],
                };
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

    /// `work <ws>`: ensure running, then exec an interactive shell.
    pub fn shell(&self) -> Result<()> {
        self.ensure_running()?;
        let ctr = naming::container(&self.cfg.name);
        let shell = self.cfg.shell.as_deref().unwrap_or("zsh");
        self.engine.exec_interactive(&ctr, &[shell, "-l"])
    }

    pub fn status(&self) -> Result<WorkspaceStatus> {
        let ctr = naming::container(&self.cfg.name);
        let state = self.engine.container_state(&ctr)?;
        Ok(WorkspaceStatus {
            name: self.cfg.name.clone(),
            state,
        })
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
        let opts = RunOpts {
            name: ctr.clone(),
            image: self.cfg.image.clone(),
            network: naming::network(&self.cfg.name),
            volume: naming::volume(&self.cfg.name),
            volume_target: "/home/dev".into(),
            workdir: "/home/dev".into(),
            cmd: vec!["sleep".into(), "infinity".into()],
        };
        self.engine.run(&opts)?;
        self.apply_git_identity()?;
        Ok(())
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

/// `work ls`: every workspace on disk with its container state.
pub fn list_all() -> Result<Vec<WorkspaceStatus>> {
    let engine = crate::engine::detect()?;
    let mut out = Vec::new();
    for name in config::list_workspace_names()? {
        let ctr = naming::container(&name);
        let state = engine
            .container_state(&ctr)
            .unwrap_or(ContainerState::Missing);
        out.push(WorkspaceStatus { name, state });
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

/// `work all`: open a tmux session `work` with one window per workspace.
pub fn tmux_all() -> Result<()> {
    let names = config::list_workspace_names()?;
    if names.is_empty() {
        bail!("no workspaces yet; create one with `work new <ws>`");
    }
    // Kill any prior `work` session for a clean window set.
    let _ = Command::new("tmux")
        .args(["kill-session", "-t", "work"])
        .status();

    let bin = std::env::current_exe()
        .context("locating the `work` binary")?
        .to_string_lossy()
        .into_owned();

    let mut first = true;
    for name in &names {
        let window_cmd = format!("{bin} {name}");
        if first {
            let _ = Command::new("tmux")
                .args(["new-session", "-d", "-s", "work", "-n", name, &window_cmd])
                .status()?;
            first = false;
        } else {
            let _ = Command::new("tmux")
                .args(["new-window", "-t", "work", "-n", name, &window_cmd])
                .status()?;
        }
    }

    // Attach (or switch-client if already inside tmux).
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

fn now_rfc3339() -> String {
    use chrono::Utc;
    Utc::now().to_rfc3339()
}
