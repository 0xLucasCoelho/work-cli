//! Workspace lifecycle. Isolation flags come from `isolation::workspace_run_opts`.

use anyhow::{bail, Result};

use crate::config::{self, WorkspaceConfig};
use crate::engine::{self, ContainerState, Engine};
use crate::isolation;
use crate::naming;

pub struct Workspace {
    pub name: String,
    pub cfg: WorkspaceConfig,
    engine: Box<dyn Engine>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceStatus {
    pub name: String,
    pub state: ContainerState,
    pub session_live: bool,
}

impl Workspace {
    pub fn engine(&self) -> &dyn Engine {
        &*self.engine
    }

    pub fn open(name: &str) -> Result<Self> {
        let cfg = config::load_workspace(name)?;
        let engine = engine::detect()?;
        if let Some(want) = cfg.daemon_id.as_deref() {
            match engine.daemon_id() {
                Ok(have) if have != want => {
                    bail!(
                        "workspace '{name}' was created on daemon {want}, current daemon is {have}"
                    );
                }
                Ok(_) => {}
                Err(_) => {}
            }
        }
        Ok(Self {
            name: name.to_string(),
            cfg,
            engine,
        })
    }

    pub fn create(name: &str, image: Option<String>, git_name: Option<String>, git_email: Option<String>) -> Result<Self> {
        naming::validate_name(name)?;
        if config::workspace_exists(name) {
            bail!("workspace '{name}' already exists");
        }
        let engine = engine::detect()?;
        if !engine.is_running()? {
            bail!(
                "container engine '{}' is not running",
                engine.binary()
            );
        }
        if !engine.kind().is_rootless_default() {
            eprintln!(
                "warning: engine '{}' is not rootless-by-default. Anyone in the docker group can read every volume.",
                engine.kind().as_str()
            );
        }

        let global = config::load_global()?;
        let image = image.unwrap_or_else(|| global.effective_default_image().to_string());
        ensure_image(&*engine, &image)?;

        let shell = config::detect_shell();
        let vol = naming::volume(name);
        let net = naming::network(name);
        let ctr = naming::container(name);

        if engine.volume_exists(&vol)? {
            if !engine.object_has_label(&vol, "volume", naming::LABEL_KEY)? {
                bail!(
                    "volume '{vol}' exists but isn't work-managed — refusing to reuse it"
                );
            }
        } else {
            engine.create_volume(&vol)?;
        }
        if engine.network_exists(&net)? {
            if !engine.object_has_label(&net, "network", naming::LABEL_KEY)? {
                bail!(
                    "network '{net}' exists but isn't work-managed — refusing to reuse it"
                );
            }
        } else {
            engine.create_network(&net)?;
        }
        if engine.container_exists(&ctr)? {
            engine.remove_container(&ctr)?;
        }

        let opts = isolation::workspace_run_opts(
            name,
            &image,
            config::shell_path(&shell),
            engine.kind().userns_auto(),
        );
        engine.run(&opts)?;

        let daemon_id = engine.daemon_id().ok();
        let image_digest = engine.image_id(&image).ok();
        let cfg = WorkspaceConfig {
            name: name.to_string(),
            image,
            git_name: git_name.clone(),
            git_email: git_email.clone(),
            shell: Some(shell),
            daemon_id,
            image_digest,
            created_at: now_rfc3339(),
        };
        config::save_workspace(&cfg)?;

        let ws = Self {
            name: name.to_string(),
            cfg,
            engine,
        };
        if git_name.is_some() || git_email.is_some() {
            let _ = ws.apply_git_identity();
        }
        let _ = ws.seed_src_dir();
        Ok(ws)
    }

    fn seed_src_dir(&self) -> Result<()> {
        let ctr = naming::container(&self.name);
        self.engine.exec_capture(&ctr, &["mkdir", "-p", "/home/dev/src"])?;
        Ok(())
    }

    pub fn apply_git_identity(&self) -> Result<()> {
        let ctr = naming::container(&self.name);
        if let Some(n) = &self.cfg.git_name {
            self.engine
                .exec_capture(&ctr, &["git", "config", "--global", "user.name", n])?;
        }
        if let Some(e) = &self.cfg.git_email {
            self.engine
                .exec_capture(&ctr, &["git", "config", "--global", "user.email", e])?;
        }
        Ok(())
    }

    pub fn ensure_running(&self) -> Result<()> {
        let ctr = naming::container(&self.name);
        match self.engine.container_state(&ctr)? {
            ContainerState::Running => Ok(()),
            ContainerState::Stopped => self.engine.start_container(&ctr),
            ContainerState::Missing => {
                let shell = self.cfg.shell.as_deref().unwrap_or("zsh");
                let opts = isolation::workspace_run_opts(
                    &self.name,
                    &self.cfg.image,
                    config::shell_path(shell),
                    self.engine.kind().userns_auto(),
                );
                self.engine.run(&opts)
            }
        }
    }

    pub fn start(&self) -> Result<()> {
        self.ensure_running()
    }

    pub fn stop(&self) -> Result<()> {
        let ctr = naming::container(&self.name);
        if self.engine.container_state(&ctr)? == ContainerState::Running {
            self.engine.stop_container(&ctr)?;
        }
        Ok(())
    }

    pub fn verify_before_attach(&self) -> Result<()> {
        let ctr = naming::container(&self.name);
        let nets = self.engine.container_networks(&ctr)?;
        let mounts = self.engine.container_mounts(&ctr)?;
        let check = crate::doctor::analyze_isolation(&self.name, &nets, &mounts);
        if !check.ok {
            bail!("isolation check failed before attach: {}", check.detail);
        }
        Ok(())
    }

    /// Attach a login shell in the company box. The ADE owns agent terminals;
    /// this CLI path is a fallback, not a multiplexer.
    pub fn attach(&self) -> Result<()> {
        self.ensure_running()?;
        self.verify_before_attach()?;
        let ctr = naming::container(&self.name);
        let shell = self.cfg.shell.clone().unwrap_or_else(config::detect_shell);
        if config::load_global()?.show_banner {
            eprint!("{}", banner(&self.name, &self.cfg.image));
        }
        self.engine
            .exec_interactive(&ctr, &[config::shell_path(&shell), "-l"], &shell)
    }

    pub fn has_live_session(&self) -> Result<bool> {
        let ctr = naming::container(&self.name);
        Ok(self.engine.container_state(&ctr)? == ContainerState::Running)
    }

    pub fn status(&self) -> Result<WorkspaceStatus> {
        let ctr = naming::container(&self.name);
        let state = self.engine.container_state(&ctr)?;
        Ok(WorkspaceStatus {
            name: self.name.clone(),
            state,
            session_live: state == ContainerState::Running,
        })
    }

    pub fn remove(&self, purge: bool) -> Result<()> {
        let ctr = naming::container(&self.name);
        let net = naming::network(&self.name);
        let vol = naming::volume(&self.name);
        if self.engine.container_exists(&ctr)? {
            self.engine.remove_container(&ctr)?;
        }
        if self.engine.network_exists(&net)? {
            let _ = self.engine.remove_network(&net);
        }
        if purge && self.engine.volume_exists(&vol)? {
            self.engine.remove_volume(&vol)?;
        }
        config::remove_workspace_config(&self.name)?;
        Ok(())
    }
}

pub fn list_all() -> Result<Vec<WorkspaceStatus>> {
    let engine = engine::detect()?;
    let mut out = Vec::new();
    for name in config::list_workspace_names()? {
        let ctr = naming::container(&name);
        let state = engine
            .container_state(&ctr)
            .unwrap_or(ContainerState::Missing);
        out.push(WorkspaceStatus {
            name,
            state,
            session_live: state == ContainerState::Running,
        });
    }
    Ok(out)
}

fn ensure_image(engine: &dyn Engine, image: &str) -> Result<()> {
    if engine.image_exists(image)? {
        return Ok(());
    }
    if image == config::DEFAULT_IMAGE {
        bail!(
            "image '{image}' is not present. Build it with: work image build\n\
             (or `podman build -t work-base:latest -f crates/docker/work-base.Dockerfile .`)"
        );
    }
    eprintln!("image '{image}' not found; pulling…");
    engine.pull_image(image)
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn banner(name: &str, image: &str) -> String {
    format!(
        "\n  ╭─ work ────────────────────────────────────╮\n  │  company        {name:<24} │\n  │  image          {image:<24} │\n  │  isolated · single-context                 │\n  ╰────────────────────────────────────────────╯\n\n"
    )
}

pub fn build_default_image() -> Result<()> {
    let engine = engine::detect()?;
    let dockerfile = default_dockerfile_path()?;
    let context = dockerfile
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .unwrap_or(std::path::Path::new("."));
    eprintln!("building {} from {}…", config::DEFAULT_IMAGE, dockerfile.display());
    engine.build_image(config::DEFAULT_IMAGE, context, dockerfile.to_str().unwrap())
}

fn default_dockerfile_path() -> Result<std::path::PathBuf> {
    let candidates = [
        std::path::PathBuf::from("crates/docker/work-base.Dockerfile"),
        std::path::PathBuf::from("docker/work-base.Dockerfile"),
    ];
    for c in candidates {
        if c.exists() {
            return Ok(c);
        }
    }
    // Walk up from CARGO_MANIFEST_DIR at compile time is wrong at runtime.
    // Search next to the executable's ancestors, then cwd.
    if let Ok(exe) = std::env::current_exe() {
        for anc in exe.ancestors() {
            let p = anc.join("crates/docker/work-base.Dockerfile");
            if p.exists() {
                return Ok(p);
            }
        }
    }
    bail!("could not find crates/docker/work-base.Dockerfile; run from the repo root")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_names_the_company() {
        let b = banner("acme", "work-base:latest");
        assert!(b.contains("acme"));
        assert!(b.contains("isolated"));
    }
}
