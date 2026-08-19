//! High-level workspace orchestration: composes engine + config + naming.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::config::{self, ImportSrc, WorkspaceConfig};
use crate::doctor;
use crate::engine::{ContainerState, Engine, HardenOpts, RunOpts};
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
    /// True iff the container is up and its in-container herdr server is live
    /// (shown as the SESSION column in `work ls`).
    pub session_live: bool,
}

/// The active daemon's view of the three resources that constitute a
/// workspace. This deliberately contains only facts needed to decide whether
/// changing a workspace's recorded daemon can be safe.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DaemonRecoveryResources {
    pub container_exists: bool,
    pub volume_exists: bool,
    pub network_exists: bool,
    pub container_managed: bool,
    pub volume_managed: bool,
    pub network_managed: bool,
    pub container_isolated: bool,
}

/// A daemon mismatch recovery is safe only when this daemon already contains
/// the complete, work-managed, isolated workspace. Partial or foreign
/// resources are never adopted automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonRecoveryState {
    Empty,
    CompleteManagedIsolated,
    Conflict,
}

impl DaemonRecoveryState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::CompleteManagedIsolated => "complete-managed-isolated",
            Self::Conflict => "conflict",
        }
    }
}

/// Pure recovery policy. It is intentionally conservative: a partial set of
/// resources, an unmanaged object, or a container outside its expected
/// topology is a conflict rather than a cue to create/adopt anything.
pub fn classify_daemon_recovery(resources: DaemonRecoveryResources) -> DaemonRecoveryState {
    if !resources.container_exists && !resources.volume_exists && !resources.network_exists {
        DaemonRecoveryState::Empty
    } else if resources.container_exists
        && resources.volume_exists
        && resources.network_exists
        && resources.container_managed
        && resources.volume_managed
        && resources.network_managed
        && resources.container_isolated
    {
        DaemonRecoveryState::CompleteManagedIsolated
    } else {
        DaemonRecoveryState::Conflict
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonRecoveryStatus {
    pub workspace: String,
    pub recorded_daemon_id: Option<String>,
    pub active_daemon_id: String,
    pub state: DaemonRecoveryState,
}

/// `work update` outcome: managed config files classified against the running
/// container, each path relative to `/home/dev`.
#[derive(Debug, Clone, Default)]
pub struct UpdateReport {
    /// Absent in the container (would be / were created).
    pub added: Vec<String>,
    /// Present but differing (would be / were overwritten).
    pub updated: Vec<String>,
    /// Present and byte-identical (skipped).
    pub unchanged: Vec<String>,
}

impl UpdateReport {
    /// Files that differ or are absent — i.e. written by a real update.
    pub fn touched(&self) -> usize {
        self.added.len() + self.updated.len()
    }
    /// Every classified file.
    pub fn total(&self) -> usize {
        self.touched() + self.unchanged.len()
    }
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
        let mut ws = Self { cfg, engine };
        ws.verify_daemon()?;
        Ok(ws)
    }

    /// Inspect whether an explicit daemon rebind can be made safely. Unlike
    /// `open`, this is available precisely when ordinary daemon verification
    /// refuses the workspace; it never writes configuration or changes engine
    /// resources.
    pub fn daemon_recovery_status(name: &str) -> Result<DaemonRecoveryStatus> {
        naming::validate_name(name)?;
        let cfg = config::load_workspace(name)?;
        let engine = crate::engine::detect()?;
        let active_daemon_id = engine.daemon_id()?;
        if active_daemon_id.is_empty() {
            bail!(
                "active engine '{}' did not report a stable daemon identity; refusing daemon recovery",
                engine.binary()
            );
        }
        let state = inspect_daemon_recovery_resources(&*engine, name)?;
        Ok(DaemonRecoveryStatus {
            workspace: cfg.name,
            recorded_daemon_id: cfg.daemon_id,
            active_daemon_id,
            state,
        })
    }

    /// Persist a new daemon identity only after `daemon_recovery_status` has
    /// verified that the active daemon contains the full managed workspace.
    /// Caller confirmation is intentionally handled by the CLI command, so
    /// library callers cannot accidentally bypass the inspection.
    pub fn rebind_daemon(name: &str) -> Result<DaemonRecoveryStatus> {
        let status = Self::daemon_recovery_status(name)?;
        if status.state != DaemonRecoveryState::CompleteManagedIsolated {
            bail!(
                "refusing to rebind workspace '{}' on this daemon: resource state is {}. \
                 Rebind requires the complete managed and isolated workspace; no resources were adopted or changed.",
                status.workspace,
                status.state.as_str(),
            );
        }
        let mut cfg = config::load_workspace(name)?;
        cfg.daemon_id = Some(status.active_daemon_id.clone());
        config::save_workspace(&cfg)?;
        Ok(status)
    }

    /// Enforce that the active daemon matches the one this workspace was created
    /// on. A workspace created on daemon A that later resolves to daemon B (a
    /// changed engine endpoint/context / a second engine instance) is refused —
    /// names are not an isolation boundary across daemons. `daemon_id == None`
    /// (created before this field existed) is backfilled on first open, never a
    /// hard failure; a daemon that won't report an ID is treated as unverified.
    fn verify_daemon(&mut self) -> Result<()> {
        let current = self.engine.daemon_id().unwrap_or_default();
        if current.is_empty() {
            return Ok(());
        }
        match &self.cfg.daemon_id {
            None => {
                self.cfg.daemon_id = Some(current);
                let _ = config::save_workspace(&self.cfg);
            }
            Some(expected) if expected != &current => bail!(
                "workspace '{}' was created on daemon {}, but the active engine now resolves to \
                 {}. Switch back to the original engine/context before opening it.",
                self.cfg.name,
                expected,
                current
            ),
            _ => {}
        }
        Ok(())
    }

    /// `work new <ws>`: create volume + network + container, persist config,
    /// and (optionally) seed shell/herdr configs for familiarity.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        name: &str,
        image: Option<String>,
        git_name: Option<String>,
        git_email: Option<String>,
        import_shell: Option<ImportSrc>,
        import_herdr: Option<ImportSrc>,
        import_starship: Option<ImportSrc>,
        import_dotfiles: Option<std::path::PathBuf>,
        use_author_default: bool,
    ) -> Result<Self> {
        Self::create_with_profile(
            name,
            image,
            None,
            None,
            None,
            None,
            None,
            git_name,
            git_email,
            import_shell,
            import_herdr,
            import_starship,
            import_dotfiles,
            use_author_default,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_with_profile(
        name: &str,
        image: Option<String>,
        profile: Option<String>,
        requested_shell: Option<String>,
        requested_pids_limit: Option<u32>,
        requested_browser_confirmation: Option<config::BrowserConfirmation>,
        requested_browser_profile: Option<config::BrowserProfile>,
        git_name: Option<String>,
        git_email: Option<String>,
        import_shell: Option<ImportSrc>,
        import_herdr: Option<ImportSrc>,
        import_starship: Option<ImportSrc>,
        import_dotfiles: Option<std::path::PathBuf>,
        use_author_default: bool,
    ) -> Result<Self> {
        naming::validate_name(name)?;
        if config::workspace_exists(name) {
            bail!("workspace '{name}' already exists");
        }
        let global = config::load_global()?;
        // An explicit image is an intentional legacy/custom-image override of
        // the saved default. An explicitly requested profile, by contrast,
        // must remain reproducible and therefore cannot be paired with an
        // arbitrary image.
        let explicit_profile = profile.is_some();
        let selected_profile = if image.is_some() {
            profile
        } else {
            Some(
                profile
                    .or_else(|| Some(global.effective_default_profile().to_string()))
                    .expect("default profile is always available"),
            )
        };
        // A profile is a fixed Work-owned image/tool declaration. A custom
        // image can still be used by legacy workflows, but never pretends to
        // provide a profile's advertised tools.
        if explicit_profile && image.is_some() {
            bail!(
                "--profile cannot be combined with --image; profiles use a fixed Work-owned image"
            );
        }
        let resolved = match selected_profile.as_deref() {
            Some(profile) => Some(config::resolve_profile(
                Some(profile),
                requested_shell.as_deref(),
            )?),
            None => {
                if let Some(shell) = requested_shell.as_deref() {
                    if !matches!(shell, "bash" | "zsh") {
                        bail!("shell '{shell}' requires a profile that advertises it (use --profile developer for fish)");
                    }
                }
                None
            }
        };
        let image = resolved
            .as_ref()
            .map(|profile| profile.image.clone())
            .or(image)
            .unwrap_or_else(|| global.effective_default_image().to_string());
        let shell = resolved
            .as_ref()
            .map(|profile| profile.shell.clone())
            .or(requested_shell)
            .unwrap_or_else(|| match config::detect_shell().as_str() {
                // A custom/legacy image is only contracted to ship bash/zsh.
                // Fish remains available when the developer profile is used.
                "fish" => "zsh".to_string(),
                detected => detected.to_string(),
            });

        let pids_limit = config::validate_pids_limit(
            requested_pids_limit.unwrap_or(config::DEFAULT_PIDS_LIMIT),
        )?;
        let browser_confirmation = requested_browser_confirmation.unwrap_or_default();
        let browser_profile = requested_browser_profile.unwrap_or_default();

        let engine = crate::engine::detect()?;
        if !engine.is_running()? {
            bail!(
                "container engine '{}' is not running; start it first (on macOS/WSL, also start the Podman machine if applicable)",
                engine.binary()
            );
        }
        // Optional dotfiles tree import (explicit --import-dotfiles overrides the
        // global default; --default falls back to the embedded templates).
        let dotfiles_dir = import_dotfiles.or(global.import_dotfiles.clone());
        let seed_bundled_defaults = dotfiles_dir.is_none()
            && (use_author_default
                || resolved
                    .as_ref()
                    .is_some_and(|profile| profile.name == config::DEVELOPER_PROFILE));

        // Ensure the image exists: build the default, or pull a custom one.
        ensure_image(&*engine, &image)?;

        // Familiarity: resolve + validate import sources BEFORE creating any
        // resources, so a bad path fails fast with no orphaned volume/net/container.
        let rc = config::rc_name(&shell);
        let shell_source =
            resolve_import(import_shell.clone(), global.import_shell_config.as_deref())
                .map(|source| source.to_path(rc));
        let herdr_source =
            resolve_import(import_herdr.clone(), global.import_herdr_config.as_deref())
                .map(|source| source.to_path(".config/herdr/config.toml"));
        let starship_source = resolve_import(
            import_starship.clone(),
            global.import_starship_config.as_deref(),
        )
        .map(|source| source.to_path(".config/starship.toml"));
        let seeds: Vec<(std::path::PathBuf, String, &str)> = [
            shell_source
                .clone()
                .map(|source| (source, format!("/home/dev/{rc}"), "shell")),
            herdr_source.clone().map(|source| {
                (
                    source,
                    "/home/dev/.config/herdr/config.toml".into(),
                    "herdr",
                )
            }),
            starship_source
                .clone()
                .map(|source| (source, "/home/dev/.config/starship.toml".into(), "starship")),
        ]
        .into_iter()
        .flatten()
        .collect();
        for (src, _dest, kind) in &seeds {
            validate_import_file(src, kind)?;
        }
        if let Some(dir) = &dotfiles_dir {
            if !dir.exists() {
                bail!(
                    "dotfiles dir not found at {}; pass a path to --import-dotfiles",
                    dir.display()
                );
            }
        }

        let vol = naming::volume(name);
        let net = naming::network(name);
        let ctr = naming::container(name);
        if engine.volume_exists(&vol)? {
            if !engine.object_has_label(&vol, "volume", naming::LABEL_KEY)? {
                bail!(
                    "volume '{vol}' exists but isn't work-managed — refusing to reuse it. Remove \
                     it manually (e.g. `{} volume rm {vol}`) if you know it's safe.",
                    engine.binary()
                );
            }
        } else {
            engine.create_volume(&vol)?;
        }
        if engine.network_exists(&net)? {
            if !engine.object_has_label(&net, "network", naming::LABEL_KEY)? {
                bail!(
                    "network '{net}' exists but isn't work-managed — refusing to reuse it. Remove \
                     it manually (e.g. `{} network rm {net}`) if you know it's safe.",
                    engine.binary()
                );
            }
        } else {
            engine.create_network(&net)?;
        }
        // Recreate container if a stale one lingers.
        if engine.container_exists(&ctr)? {
            engine.remove_container(&ctr)?;
        }
        let opts = run_opts(name, &image, &shell, pids_limit);
        engine.run(&opts)?;
        // Persist the config as soon as the container exists — recording the
        // daemon identity + resolved image id — so a later seeding failure leaves
        // a visible (if bare) workspace rather than an orphaned container.
        let daemon_id = engine.daemon_id().ok();
        let image_digest = engine.image_id(&image).ok();
        let cfg = WorkspaceConfig {
            name: name.to_string(),
            legacy_fields: std::collections::BTreeMap::new(),
            image,
            git_name: git_name.clone(),
            git_email: git_email.clone(),
            shell: Some(shell),
            profile: resolved.as_ref().map(|profile| profile.name.clone()),
            bundles: resolved.map(|profile| profile.bundles).unwrap_or_default(),
            import_shell_config: shell_source,
            import_herdr_config: herdr_source,
            import_starship_config: starship_source,
            import_dotfiles: dotfiles_dir.clone(),
            pids_limit,
            browser_confirmation,
            browser_profile,
            daemon_id,
            image_digest,
            created_at: now_rfc3339(),
        };
        config::save_workspace(&cfg)?;
        // Browser bridge shim: install early so a brand-new workspace can
        // forward `xdg-open` calls. Idempotent; best-effort (warn, don't fail
        // workspace creation over a convenience shim).
        let _ = crate::browser::install_shim(&*engine, &ctr);

        // Seed the dotfiles tree first (explicit dir, or the author's embedded
        // templates via --default or the developer profile) so per-file imports below can still
        // override individual files like .zshrc.
        if let Some(dir) = &dotfiles_dir {
            let staged = stage_allowed_dotfiles(dir)
                .with_context(|| format!("staging dotfiles from {}", dir.display()))?;
            engine
                .seed_dir(&ctr, staged.path(), "/home/dev")
                .with_context(|| format!("seeding dotfiles from {}", dir.display()))?;
            println!(
                "⚠  Copied ALLOWLISTED dotfiles from {} into '{name}'. Anything outside the \
                 allowlist (.npmrc, .aws/, …) was skipped. Ensure staged files hold no secrets — \
                 they now live in that workspace's volume.",
                dir.display()
            );
        } else if seed_bundled_defaults {
            let extracted = crate::templates::extract_to_tempdir()?;
            engine
                .seed_dir(&ctr, extracted.path(), "/home/dev")
                .context("seeding author-default dotfiles")?;
            println!(
                "✓  Seeded the bundled developer dotfiles into '{name}'. Per-file imports can override them."
            );
        }

        // Seed validated configs (verbatim, warned) + suppress the shell's
        // first-run prompt (the named volume overlays the image's /home/dev).
        for (src, dest, kind) in &seeds {
            seed_into(&*engine, &ctr, src, dest, kind, name)?;
        }
        let shell_imported = seeds.iter().any(|(_, _, kind)| *kind == "shell");
        ensure_default_rc(&*engine, &ctr, rc, shell_imported)?;

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
                let opts = run_opts(
                    &self.cfg.name,
                    &self.cfg.image,
                    self.cfg.shell.as_deref().unwrap_or("zsh"),
                    self.cfg.pids_limit,
                );
                self.engine.run(&opts)?;
            }
            ContainerState::Stopped => {
                self.engine.start_container(&ctr)?;
            }
            ContainerState::Running => {}
        }
        self.verify_before_attach(&ctr)?;
        Ok(())
    }

    /// Shared doctor gate before attach. Only verified isolation-boundary
    /// violations refuse attach; configuration drift remains visible through
    /// `work doctor` as a warning instead of making a safe workspace unusable.
    fn verify_before_attach(&self, _ctr: &str) -> Result<()> {
        let (checks, _) = doctor::workspace_checks(&*self.engine, &self.cfg)?;
        let blockers: Vec<_> = checks
            .iter()
            .filter(|check| check.blocks_attach())
            .collect();
        if !blockers.is_empty() {
            let details = blockers
                .iter()
                .map(|check| check.detail.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            bail!(
                "workspace '{}' failed its isolation check before attach (run `work doctor`): {}",
                self.cfg.name,
                details
            );
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

    /// `work <ws>`: ensure running, then attach to the in-container herdr
    /// runtime. The headless `herdr` server is launched on first attach and
    /// survives detach / closing the terminal; it does NOT survive `work stop`.
    /// Prints an identity banner and sets the terminal title first.
    pub fn shell(&self) -> Result<()> {
        self.ensure_running()?;
        let ctr = naming::container(&self.cfg.name);

        let show = config::load_global().map(|g| g.show_banner).unwrap_or(true);
        if show {
            self.print_banner(&ctr);
        }
        println!("Ctrl-b q or close terminal = detach (keeps running)");

        // Name the terminal tab (best-effort).
        {
            use std::io::Write;
            print!("\x1b]0;work:{}\x07", self.cfg.name);
            let _ = std::io::stdout().flush();
        }

        // Bare `herdr` launches the headless server on first run and attaches a
        // TUI client to it thereafter (the `tmux new-session -A` equivalent).
        self.engine
            .exec_interactive(&ctr, &["herdr"], self.cfg.shell.as_deref().unwrap_or("zsh"))
    }

    /// Gather hostname/OS/git-branch via one engine `exec` and print the banner.
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

    /// True iff the container is up AND its in-container herdr server is live.
    /// Used by the destructive-op safety policy to decide whether a stop/rm/
    /// recreate would actually lose live work. Returns `Result`: a transient
    /// inspect/runtime failure MUST surface, not resolve to `false` — callers
    /// gate destructive ops on this and fail-open would skip the work-loss
    /// prompt for a session that was actually live.
    pub fn has_live_session(&self) -> Result<bool> {
        let ctr = naming::container(&self.cfg.name);
        match self.engine.container_state(&ctr)? {
            ContainerState::Running => Ok(self.engine.runtime_up(&ctr)?),
            _ => Ok(false),
        }
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
    /// volume + network). Used by `work config` when the image changes and by
    /// `work harden` to apply the current hardening flags to a stale container.
    pub fn recreate(&self) -> Result<()> {
        let ctr = naming::container(&self.cfg.name);
        if self.engine.container_exists(&ctr)? {
            self.engine.remove_container(&ctr)?;
        }
        ensure_image(&*self.engine, &self.cfg.image)?;
        let opts = run_opts(
            &self.cfg.name,
            &self.cfg.image,
            self.cfg.shell.as_deref().unwrap_or("zsh"),
            self.cfg.pids_limit,
        );
        self.engine.run(&opts)?;
        // Re-record the resolved image id (a rebuild can change it) so `doctor`
        // detects future drift. Best-effort: a daemon that won't report it isn't
        // worth failing a recreate over.
        if let Ok(id) = self.engine.image_id(&self.cfg.image) {
            let mut cfg = self.cfg.clone();
            cfg.image_digest = Some(id);
            let _ = config::save_workspace(&cfg);
        }
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
        // + warn rather than failing this data-safe removal. A missing network
        // is already clean and should not produce a misleading warning.
        if self.engine.network_exists(&net)? {
            if let Err(e) = self.engine.remove_network(&net) {
                eprintln!(
                    "· could not remove network {net} ({e}); stop any `work fwd` for this workspace first"
                );
            }
        }
        if purge && self.engine.volume_exists(&vol)? {
            self.engine.remove_volume(&vol)?;
        }
        // Only drop the config once the container is genuinely gone, so a removal
        // that no-op'd (a restart policy racing `rm -f`) leaves the workspace
        // listed + re-removable instead of an invisible running box.
        if !self.engine.container_exists(&ctr)? {
            let _ = std::fs::remove_file(config::workspace_config_path(&self.cfg.name));
        } else {
            bail!(
                "container {ctr} still present after removal — config kept so the workspace stays \
                 visible; retry `work rm {}`",
                self.cfg.name
            );
        }
        Ok(())
    }
    /// `work update`: re-seed managed config files into the running container in
    /// place — no rebuild, no recreate, no session loss. Source resolution
    /// mirrors `work new` (explicit `--import-*` flags → global config defaults
    /// → embedded templates), but update seeds the embedded templates whenever
    /// no dotfiles dir is resolved, so a bare `work update <ws>` pushes the
    /// current templates without `--default`. `dry_run` classifies every file
    /// without writing. Returns the per-file classification for reporting.
    pub fn update(
        &self,
        import_shell: Option<ImportSrc>,
        import_herdr: Option<ImportSrc>,
        import_starship: Option<ImportSrc>,
        import_dotfiles: Option<std::path::PathBuf>,
        dry_run: bool,
    ) -> Result<UpdateReport> {
        let global = config::load_global()?;
        let dotfiles_dir = import_dotfiles
            .or(self.cfg.import_dotfiles.clone())
            .or(global.import_dotfiles.clone());

        // Precedence is explicit update flag -> source persisted at create ->
        // global default. This makes bare updates repeatable without silently
        // discarding a user's chosen shell config.
        let rc = config::rc_name(self.cfg.shell.as_deref().unwrap_or("zsh"));
        let shell_source = resolve_import(import_shell, self.cfg.import_shell_config.as_deref())
            .or_else(|| global.import_shell_config.clone().map(ImportSrc::Explicit))
            .map(|source| source.to_path(rc));
        let herdr_source = resolve_import(import_herdr, self.cfg.import_herdr_config.as_deref())
            .or_else(|| global.import_herdr_config.clone().map(ImportSrc::Explicit))
            .map(|source| source.to_path(".config/herdr/config.toml"));
        let starship_source =
            resolve_import(import_starship, self.cfg.import_starship_config.as_deref())
                .or_else(|| {
                    global
                        .import_starship_config
                        .clone()
                        .map(ImportSrc::Explicit)
                })
                .map(|source| source.to_path(".config/starship.toml"));
        let seeds: Vec<(std::path::PathBuf, String)> = [
            shell_source
                .clone()
                .map(|source| (source, format!("/home/dev/{rc}"))),
            herdr_source
                .clone()
                .map(|source| (source, "/home/dev/.config/herdr/config.toml".into())),
            starship_source
                .clone()
                .map(|source| (source, "/home/dev/.config/starship.toml".into())),
        ]
        .into_iter()
        .flatten()
        .collect();

        // Validate sources BEFORE touching the container, so a bad path fails fast.
        for (src, _dest) in &seeds {
            validate_import_file(src, "import")?;
        }
        if let Some(dir) = &dotfiles_dir {
            if !dir.exists() {
                bail!("dotfiles dir not found at {}", dir.display());
            }
        }

        // Tree to seed: an explicit dotfiles dir — staged through the allowlist
        // (never seed a user-provided tree verbatim) — else the embedded
        // templates. Both the diff and the seed read the staged snapshot, closing
        // the scan-then-copy TOCTOU a denylist has.
        let templates_tmp;
        let staged: Option<tempfile::TempDir> = match dotfiles_dir.as_deref() {
            Some(d) => Some(
                stage_allowed_dotfiles(d)
                    .with_context(|| format!("staging dotfiles from {}", d.display()))?,
            ),
            None => None,
        };
        let tree_dir: Option<&Path> = match &staged {
            Some(s) => Some(s.path()),
            None => {
                templates_tmp = Some(crate::templates::extract_to_tempdir()?);
                Some(templates_tmp.as_ref().unwrap().path())
            }
        };

        // Engine copy + `chown` need the container up.
        self.ensure_running()?;
        let ctr = naming::container(&self.cfg.name);

        // Build the file list keyed by container dest. Per-file imports override
        // any same-dest file from the tree (mirrors `create`'s seeding order).
        use std::collections::BTreeMap;
        let mut files: BTreeMap<String, (std::path::PathBuf, String)> = BTreeMap::new();
        if let Some(dir) = tree_dir {
            walk_tree(dir, dir, &mut |host, rel| {
                files.insert(format!("/home/dev/{rel}"), (host, rel));
            })?;
        }
        for (src, dest) in &seeds {
            files.insert(dest.clone(), (src.clone(), container_rel(dest)));
        }

        // Diff every file against its in-container counterpart by content hash.
        let mut report = UpdateReport::default();
        for (dest, (host, rel)) in &files {
            match file_status(&*self.engine, &ctr, host, dest)? {
                FileStatus::Missing => report.added.push(rel.clone()),
                FileStatus::Same => report.unchanged.push(rel.clone()),
                FileStatus::Changed => report.updated.push(rel.clone()),
            }
        }

        // Dry-run reports only. Nothing to write when every file is in sync.
        if dry_run || report.touched() == 0 {
            return Ok(report);
        }

        // Apply: seed the whole tree, then per-file imports on top.
        if let Some(dir) = tree_dir {
            self.engine
                .seed_dir(&ctr, dir, "/home/dev")
                .context("seeding config tree")?;
        }
        for (src, dest) in &seeds {
            self.engine
                .seed_file(&ctr, src, dest)
                .with_context(|| format!("copying {} into {dest}", src.display()))?;
        }
        self.apply_git_identity()?;
        Ok(report)
    }
}

/// Read the named workspace resources from the active daemon without changing
/// them. Container topology is evaluated only when all resources exist; any
/// inspection failure remains a hard error rather than being treated as safe.
fn inspect_daemon_recovery_resources(
    engine: &dyn Engine,
    name: &str,
) -> Result<DaemonRecoveryState> {
    let ctr = naming::container(name);
    let vol = naming::volume(name);
    let net = naming::network(name);
    let container_exists = engine.container_exists(&ctr)?;
    let volume_exists = engine.volume_exists(&vol)?;
    let network_exists = engine.network_exists(&net)?;

    let mut resources = DaemonRecoveryResources {
        container_exists,
        volume_exists,
        network_exists,
        ..Default::default()
    };
    if container_exists {
        resources.container_managed =
            engine.object_has_label(&ctr, "container", naming::LABEL_KEY)?;
    }
    if volume_exists {
        resources.volume_managed = engine.object_has_label(&vol, "volume", naming::LABEL_KEY)?;
    }
    if network_exists {
        resources.network_managed = engine.object_has_label(&net, "network", naming::LABEL_KEY)?;
    }
    if container_exists && volume_exists && network_exists {
        let networks = engine.container_networks(&ctr)?;
        let mounts = engine.container_mounts(&ctr)?;
        resources.container_isolated = doctor::analyze_isolation(name, &networks, &mounts).ok;
    }
    Ok(classify_daemon_recovery(resources))
}

// ---------- `work update` helpers ----------

/// A host file's relationship to its in-container counterpart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileStatus {
    Missing,
    Same,
    Changed,
}

/// Strip the `/home/dev/` prefix from an absolute in-container path, returning a
/// path relative to the volume root for reporting. PURE.
fn container_rel(dest: &str) -> String {
    dest.strip_prefix("/home/dev/")
        .or_else(|| dest.strip_prefix("/home/dev"))
        .unwrap_or(dest)
        .to_string()
}

/// Recursively enumerate every FILE under `root`, invoking `emit` with each
/// (absolute host path, path relative to `root` using forward slashes). Skips
/// directories and symlinks. PURE-ish: host FS reads only, no container IO.
fn walk_tree(
    root: &Path,
    dir: &Path,
    emit: &mut impl FnMut(std::path::PathBuf, String),
) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_tree(root, &path, emit)?;
        } else if path.is_file() {
            let rel = path
                .strip_prefix(root)
                .with_context(|| format!("stripping prefix from {}", path.display()))?
                .to_string_lossy()
                .to_string();
            emit(path, rel);
        }
    }
    Ok(())
}

/// sha256 of a host file's bytes, as lowercase hex. PURE given the file.
fn file_sha256(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).with_context(|| format!("hashing {}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

/// Compare a host file against its in-container counterpart by sha256. The
/// container hash comes from engine `exec sha256sum <dest>` (coreutils, present
/// on every base image). A missing file or any read error resolves to `Missing`
/// — the apply step then creates it; a transient exec failure is unlikely
/// because `update` ensures the container is running first.
fn file_status(engine: &dyn Engine, ctr: &str, host: &Path, dest: &str) -> Result<FileStatus> {
    let want = file_sha256(host)?;
    let have = engine
        .exec_capture(ctr, &["sha256sum", dest])
        .ok()
        .and_then(|s| s.split_whitespace().next().map(str::to_string));
    Ok(match have {
        Some(h) if h == want => FileStatus::Same,
        Some(_) => FileStatus::Changed,
        None => FileStatus::Missing,
    })
}

/// `run` options for a workspace container. Sets the `WORK`/`WORKSPACE`
/// identity env, the xdg-open browser shim, and `NERD_FONTS=1`: the in-container
/// multiplexer (herdr) makes agents like omp see `TERM_PROGRAM=herdr` instead of
/// the host terminal, so their Nerd-Font auto-detection fails and they fall back
/// to ASCII glyphs. `NERD_FONTS=1` forces Nerd Font glyphs — the host terminal
/// still renders them. Override per-workspace by unsetting it in your shell rc.
fn run_opts(name: &str, image: &str, shell: &str, pids_limit: u32) -> RunOpts {
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
            ("BROWSER".into(), crate::browser::SHIM_DEST.into()),
            ("NERD_FONTS".into(), "1".into()),
            // herdr picks its pane shell from `$SHELL` (its config: "empty means
            // $SHELL, then /bin/sh"). A container started with `sleep infinity`
            // skips login, so `$SHELL` is unset and herdr spawns sh — sourcing
            // neither .zshrc nor .bashrc. Set it to the resolved shell's
            // in-container path so the seeded rc actually runs.
            ("SHELL".into(), config::shell_path(shell).into()),
        ],
        harden: HardenOpts {
            pids_limit: Some(pids_limit),
            ..HardenOpts::default()
        },
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
    } else if image == config::DEVELOPER_IMAGE {
        println!("developer image '{image}' not found; building it now…");
        image::build_developer(engine)
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
        let session_live =
            state == ContainerState::Running && engine.runtime_up(&ctr).unwrap_or(false);
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
/// Ctrl-C lets the selected engine's `run` stop + `--rm` the container.
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
    // Blocks until interrupted; cleanup is handled by the selected engine's `run --rm`.
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

/// `work browse <ws>`: forward URLs that in-container tools try to open
/// (`xdg-open`/`$BROWSER`) to the host browser, and auto-bridge the OAuth
/// loopback callback port so a login completes with one command. Installs/
/// refreshes the shim, ensures the volume FIFO exists, then blocks reading it
/// — each `http(s)` URL is opened via the host browser opener, and if it
/// carries a loopback `redirect_uri` that port is bridged host→container.
/// Ctrl-C stops it (the container is unaffected; the FIFO persists). Mirrors
/// `work fwd`.
pub fn browse(name: &str) -> Result<()> {
    let ws = Workspace::open(name)?;
    ws.ensure_running()?;
    let engine = ws.engine();
    let ctr = naming::container(name);
    let net = naming::network(name);
    crate::browser::install_shim(engine, &ctr)?;
    crate::browser::ensure_fifo(engine, &ctr)?;
    println!("Browsing for {name} — login URLs also bridge their callback port to the host.");
    println!("(Ctrl-C to stop)");
    let opener = crate::browser::host_opener();
    let confirmation = if std::env::var("WORK_BROWSE_CONFIRM").ok().as_deref() == Some("no") {
        config::BrowserConfirmation::Trusted
    } else {
        ws.cfg.browser_confirmation
    };
    let mut guard = crate::browser::BrowseGuard::new(confirmation);
    let mut bridged: HashSet<u16> = HashSet::new();
    let mut fwd_names: Vec<String> = Vec::new();
    let result = browse_loop(
        engine,
        &ctr,
        &net,
        name,
        &opener,
        &mut bridged,
        &mut fwd_names,
        &mut guard,
        ws.cfg.browser_profile,
    );
    // Cleanup forwarders on normal/error exit. Ctrl-C is handled by the process
    // group receiving SIGINT -> each engine `run --rm` forwarder stops + removes.
    for fwd in &fwd_names {
        let _ = engine.remove_container(fwd);
    }
    result
}

/// Read URLs from the bridge FIFO forever; for each, auto-bridge a loopback
/// OAuth callback port (if any) then open the URL in the host browser.
#[allow(clippy::too_many_arguments)]
fn browse_loop(
    engine: &dyn Engine,
    ctr: &str,
    net: &str,
    ws: &str,
    opener: &str,
    bridged: &mut HashSet<u16>,
    fwd_names: &mut Vec<String>,
    guard: &mut crate::browser::BrowseGuard,
    browser_profile: config::BrowserProfile,
) -> Result<()> {
    loop {
        let line = engine.exec_capture(ctr, &["cat", crate::browser::FIFO_PATH])?;
        let url = line.trim();
        if !crate::browser::is_openable_url(url) {
            if !url.is_empty() {
                eprintln!("· ignored non-http(s) target: {url}");
            }
            continue;
        }
        if let Some(port) = crate::browser::callback_port(url) {
            if bridged.insert(port) {
                let fwd = format!("work-browse-{ws}-{port}");
                match engine.spawn_forwarder(&fwd, net, port, ctr, port) {
                    Ok(_) => {
                        fwd_names.push(fwd);
                        println!("· bridged callback port {port} (host 127.0.0.1:{port} -> {ws})");
                    }
                    Err(e) => {
                        bridged.remove(&port);
                        eprintln!(
                            "· could not bridge callback port {port} ({e}); run `work fwd {ws} {port}` if needed"
                        );
                    }
                }
            }
        }
        if !guard.should_open(url) {
            continue;
        }
        match crate::browser::open_url(opener, url, browser_profile) {
            Ok(()) => println!("↗ opened {url}"),
            Err(e) => eprintln!("· could not open {url} ({e})"),
        }
    }
}

/// Effective import source: a per-workspace flag overrides the global default.
fn resolve_import(flag: Option<ImportSrc>, global: Option<&Path>) -> Option<ImportSrc> {
    flag.or_else(|| global.map(|p| ImportSrc::Explicit(p.to_path_buf())))
}

/// Top-level dotfile entries an explicit `--import-dotfiles` may stage into a
/// workspace. An ALLOWLIST (not a denylist): anything not listed is refused, so
/// pointing `--import-dotfiles` at `~` by habit can't drag in arbitrary
/// credentials (`.npmrc`, `.aws/`, `.config/gh`, …). Extend it deliberately.
const ALLOWED_DOTFILES: &[&str] = &[
    ".zshrc",
    ".bashrc",
    ".zshenv",
    ".gitconfig",
    ".tmux.conf",
    ".vimrc",
    ".config/nvim",
    ".config/starship.toml",
    ".config/atuin",
    ".config/fish",
    ".config/git",
    ".config/herdr",
];

/// Stage only the allowlisted dotfile entries from `src` into a fresh tempdir,
/// rejecting symlinks at every level, then return the staging dir. The caller
/// seeds from the staged snapshot — never the live source — closing the
/// scan-then-copy TOCTOU a denylist has.
fn stage_allowed_dotfiles(src: &Path) -> Result<tempfile::TempDir> {
    let staging = tempfile::tempdir().context("staging dotfiles")?;
    for name in ALLOWED_DOTFILES {
        let from = src.join(name);
        if !from.exists() {
            continue;
        }
        let to = staging.path().join(name);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        copy_tree_rejecting_symlinks(&from, &to)?;
    }
    Ok(staging)
}

/// Copy a file or recurse a directory, refusing symlinks at every level (a
/// symlinked dotfile could escape the workspace volume or point at a host secret).
fn copy_tree_rejecting_symlinks(from: &Path, to: &Path) -> Result<()> {
    let meta =
        std::fs::symlink_metadata(from).with_context(|| format!("reading {}", from.display()))?;
    if meta.file_type().is_symlink() {
        bail!("refusing to import symlink {}", from.display());
    }
    if meta.is_dir() {
        std::fs::create_dir_all(to).with_context(|| format!("creating {}", to.display()))?;
        for entry in
            std::fs::read_dir(from).with_context(|| format!("reading {}", from.display()))?
        {
            let entry = entry?;
            let entry_name = entry.file_name();
            copy_tree_rejecting_symlinks(&entry.path(), &to.join(entry_name))?;
        }
    } else {
        std::fs::copy(from, to)
            .with_context(|| format!("copying {} -> {}", from.display(), to.display()))?;
    }
    Ok(())
}

/// Validate a host config file before it can reach a workspace volume. Imports
/// are regular files only: a symlink could otherwise smuggle an unrelated host
/// secret into the workspace.
fn validate_import_file(src: &Path, kind: &str) -> Result<()> {
    let meta = std::fs::symlink_metadata(src)
        .with_context(|| format!("reading {kind} config {}", src.display()))?;
    if meta.file_type().is_symlink() {
        bail!("refusing to import symlink {}", src.display());
    }
    if !meta.is_file() {
        bail!("{kind} config at {} is not a regular file", src.display());
    }
    Ok(())
}

/// Copy a validated host config file into the container volume (owned by dev),
/// verbatim, printing the secret warning.
fn seed_into(
    engine: &dyn Engine,
    ctr: &str,
    src: &Path,
    dest: &str,
    kind: &str,
    ws: &str,
) -> Result<()> {
    validate_import_file(src, kind)?;
    engine
        .seed_file(ctr, src, dest)
        .with_context(|| format!("seeding {dest} from {}", src.display()))?;
    println!(
        "⚠  Copied your {kind} config into '{ws}'. Ensure it contains no secrets — it now lives in that workspace's volume."
    );
    Ok(())
}

const ZSHRC_DEFAULT: &str = r#"# Default work prompt. Override: `work new --import-shell-config`.
setopt PROMPT_SUBST
PROMPT='%F{magenta}⬡%f %F{cyan}$WORK%f %F{blue}%~%f %# '
"#;

const BASHRC_DEFAULT: &str = r#"# Default work prompt. Override: `work new --import-shell-config`.
PS1="\[\e[35m\]⬡\[\e[0m\] \[\e[36m\]${WORK}\[\e[0m\] \[\e[34m\]\w\[\e[0m\] $ "
"#;

const FISHRC_DEFAULT: &str = r#"# Default work prompt. Override: `work new --import-shell-config`.
function fish_prompt
    set_color magenta; echo -n '⬡ '
    set_color cyan; echo -n "$WORK "
    set_color blue; echo -n (prompt_pwd)
    set_color normal; echo -n '> '
end
"#;

/// Default rc body for the resolved shell.
fn default_rc(rcname: &str) -> &'static str {
    match rcname {
        ".bashrc" => BASHRC_DEFAULT,
        ".config/fish/config.fish" => FISHRC_DEFAULT,
        _ => ZSHRC_DEFAULT,
    }
}

/// Ensure `/home/dev/<rcname>` exists. If a shell config was imported it is
/// already present (verbatim) — never overwrite. If nothing was imported and the
/// rc is absent, write the minimal default rc with a workspace-aware prompt.
fn ensure_default_rc(engine: &dyn Engine, ctr: &str, rcname: &str, imported: bool) -> Result<()> {
    let path = format!("/home/dev/{rcname}");
    if engine.exec_capture(ctr, &["test", "-e", &path]).is_ok() {
        return Ok(()); // seeded or persisted — leave it alone.
    }
    if imported {
        let _ = engine.exec_capture(ctr, &["touch", &path]);
        return Ok(()); // import source was absent; keep empty rather than impose.
    }
    let dir = tempfile::tempdir().context("staging default rc")?;
    let src = dir.path().join(rcname);
    std::fs::write(&src, default_rc(rcname)).context("writing default rc")?;
    engine
        .seed_file(ctr, &src, &path)
        .with_context(|| format!("seeding default {rcname}"))?;
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
        let opts = run_opts(
            "acme",
            "work-base:latest",
            "zsh",
            config::DEFAULT_PIDS_LIMIT,
        );
        assert_eq!(opts.name, "work-acme");
        assert_eq!(opts.network, "work-net-acme");
        assert_eq!(opts.volume, "work-acme-home");
        assert_eq!(
            opts.env,
            vec![
                ("WORK".to_string(), "acme".to_string()),
                ("WORKSPACE".to_string(), "acme".to_string()),
                ("BROWSER".to_string(), "/usr/local/bin/xdg-open".to_string()),
                ("NERD_FONTS".to_string(), "1".to_string()),
                ("SHELL".to_string(), "/usr/bin/zsh".to_string()),
            ]
        );
        assert_eq!(opts.harden.pids_limit, Some(4096));
        assert!(opts.harden.cap_add.is_empty());
        assert!(!opts.harden.read_only_rootfs);
    }

    #[test]
    fn default_rc_is_workspace_aware() {
        let z = default_rc(".zshrc");
        assert!(z.contains("PROMPT"));
        assert!(z.contains("$WORK"));
        let b = default_rc(".bashrc");
        assert!(b.contains("PS1"));
        assert!(b.contains("WORK"));
    }

    #[test]
    fn container_rel_strips_volume_root() {
        assert_eq!(container_rel("/home/dev/.gitconfig"), ".gitconfig");
        assert_eq!(
            container_rel("/home/dev/.config/starship.toml"),
            ".config/starship.toml"
        );
    }

    #[test]
    fn container_rel_passes_through_non_volume_paths() {
        assert_eq!(container_rel("/etc/hostname"), "/etc/hostname");
        // Bare volume root without a trailing slash -> empty relative path.
        assert_eq!(container_rel("/home/dev"), "");
    }

    #[test]
    fn walk_tree_lists_nested_files_relative() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitconfig"), b"x").unwrap();
        std::fs::create_dir_all(root.join(".config")).unwrap();
        std::fs::write(root.join(".config").join("starship.toml"), b"y").unwrap();

        let mut got: Vec<String> = Vec::new();
        walk_tree(root, root, &mut |_host, rel| got.push(rel)).unwrap();
        got.sort();
        assert_eq!(
            got,
            vec![
                ".config/starship.toml".to_string(),
                ".gitconfig".to_string()
            ]
        );
    }

    #[test]
    fn update_report_touched_and_total() {
        let r = UpdateReport {
            added: vec!["a".into()],
            updated: vec!["b".into(), "c".into()],
            unchanged: vec!["d".into()],
        };
        assert_eq!(r.touched(), 3);
        assert_eq!(r.total(), 4);
    }
}
