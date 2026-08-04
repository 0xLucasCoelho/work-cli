//! High-level workspace orchestration: composes engine + config + naming.

use std::collections::HashSet;
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
        Ok(Self { cfg, engine })
    }

    /// `work new <ws>`: create volume + network + container, persist config,
    /// and (optionally) seed shell/tmux configs for familiarity.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        name: &str,
        image: Option<String>,
        git_name: Option<String>,
        git_email: Option<String>,
        import_shell: Option<ImportSrc>,
        import_tmux: Option<ImportSrc>,
        import_starship: Option<ImportSrc>,
        import_dotfiles: Option<std::path::PathBuf>,
        use_author_default: bool,
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
        // Optional dotfiles tree import (explicit --import-dotfiles overrides the
        // global default; --default falls back to the embedded templates).
        let dotfiles_dir = import_dotfiles.or(global.import_dotfiles.clone());
        let seed_author_default = use_author_default && dotfiles_dir.is_none();

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
            resolve_import(import_starship, global.import_starship_config.as_deref()).map(|s| {
                (
                    s.to_path(".config/starship.toml"),
                    "/home/dev/.config/starship.toml".into(),
                    "starship",
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
        // Browser bridge shim: install early so a brand-new workspace can
        // forward `xdg-open` calls. Idempotent; best-effort (warn, don't fail
        // workspace creation over a convenience shim).
        let _ = crate::browser::install_shim(&*engine, &ctr);

        // Seed the dotfiles tree first (explicit dir, or the author's embedded
        // templates via --default) so per-file imports below can still
        // override individual files like .zshrc.
        if let Some(dir) = &dotfiles_dir {
            engine
                .seed_dir(&ctr, dir, "/home/dev")
                .with_context(|| format!("seeding dotfiles from {}", dir.display()))?;
            println!(
                "⚠  Copied dotfiles from {} into '{name}'. Ensure they contain no secrets — they now live in that workspace's volume.",
                dir.display()
            );
        } else if seed_author_default {
            let extracted = crate::templates::extract_to_tempdir()?;
            engine
                .seed_dir(&ctr, extracted.path(), "/home/dev")
                .context("seeding author-default dotfiles")?;
            println!(
                "⚠  Copied the author's default dotfiles into '{name}'. Ensure they contain no secrets — they now live in that workspace's volume."
            );
        }

        // Seed validated configs (verbatim, warned) + suppress the shell's
        // first-run prompt (the named volume overlays the image's /home/dev).
        for (src, dest, kind) in &seeds {
            seed_into(&*engine, &ctr, src, dest, kind, name)?;
        }
        let shell_imported = seeds.iter().any(|(_, _, kind)| *kind == "shell");
        ensure_default_rc(&*engine, &ctr, rc, shell_imported)?;
        let tmux_imported = seeds.iter().any(|(_, _, kind)| *kind == "tmux");
        ensure_default_tmux_conf(&*engine, &ctr, tmux_imported)?;

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

        // Lossless one-time migration: rename a stale `work` session to <ws>.
        self.migrate_session_name(&ctr, session);

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
    /// Lossless one-time migration: rename a stale `work` session to <ws> in
    /// place (running shells/agents inside it survive). No-op once renamed, or
    /// for a workspace literally named "work". Shared by `shell()` and `tab()`.
    fn migrate_session_name(&self, ctr: &str, session: &str) {
        if session != "work"
            && self.engine.session_exists(ctr, "work").unwrap_or(false)
            && !self.engine.session_exists(ctr, session).unwrap_or(false)
        {
            let _ = self
                .engine
                .exec_capture(ctr, &["tmux", "rename-session", "-t", "work", session]);
        }
    }

    /// `work tab <ws> [--name <n>]`: open a NEW tmux window ("tab") in the
    /// workspace's session and attach to it. Each call = one persistent window
    /// that survives detach / closing the terminal (not `work stop`). Creates
    /// the session if it is missing. The new window becomes the session's active
    /// window so the attaching client lands in it; a tmux session shares one
    /// active window across clients (as `Ctrl-b c` does), so other attached
    /// terminals move to it too.
    pub fn tab(&self, name: Option<&str>) -> Result<()> {
        self.ensure_running()?;
        let ctr = naming::container(&self.cfg.name);
        let shell = self.cfg.shell.as_deref().unwrap_or("zsh");
        let session = naming::session(&self.cfg.name);

        if let Some(n) = name {
            validate_window_name(n)?;
        }
        self.migrate_session_name(&ctr, session);

        // Banner + detach hint (suppressed in cockpit windows).
        if std::env::var_os("WORK_COCKPIT").is_none() {
            let show = config::load_global().map(|g| g.show_banner).unwrap_or(true);
            if show {
                self.print_banner(&ctr);
            }
            println!("Ctrl-b d or close terminal = detach (keeps running) · exit = close this tab");
        }

        // Name the host terminal tab (best-effort).
        {
            use std::io::Write;
            print!("\x1b]0;work:{}\x07", self.cfg.name);
            let _ = std::io::stdout().flush();
        }

        if !self.engine.session_exists(&ctr, session).unwrap_or(false) {
            // Session missing: create it detached with THIS window (the tab),
            // named after the workspace (or the explicit --name), then attach.
            let win_name = name.unwrap_or(session);
            self.engine
                .exec_capture(
                    &ctr,
                    &[
                        "tmux",
                        "new-session",
                        "-d",
                        "-s",
                        session,
                        "-n",
                        win_name,
                        "--",
                        shell,
                        "-l",
                    ],
                )
                .context("creating in-container session")?;
            self.engine
                .exec_interactive(&ctr, &["tmux", "attach", "-t", session])?;
        } else {
            // Session exists: append a window after the LAST window
            // (`-a -t <session>:$`) so indices stay monotonic. `-P -F` prints the
            // new window's index, which we attach to (making it the active window).
            // Bare `-t <session>` would place the window *before* the active window
            // and collide on its index (confirmed on tmux 3.4).
            let last = format!("{session}:$");
            let cmd: &[&str] = if let Some(n) = name {
                &[
                    "tmux",
                    "new-window",
                    "-d",
                    "-a",
                    "-P",
                    "-F",
                    "#{window_index}",
                    "-t",
                    last.as_str(),
                    "-n",
                    n,
                    "--",
                    shell,
                    "-l",
                ]
            } else {
                &[
                    "tmux",
                    "new-window",
                    "-d",
                    "-a",
                    "-P",
                    "-F",
                    "#{window_index}",
                    "-t",
                    last.as_str(),
                    "--",
                    shell,
                    "-l",
                ]
            };
            let idx = self
                .engine
                .exec_capture(&ctr, cmd)
                .context("creating in-container tmux window")?;
            let target = if idx.is_empty() {
                session.to_string()
            } else {
                format!("{session}:{idx}")
            };
            self.engine
                .exec_interactive(&ctr, &["tmux", "attach", "-t", target.as_str()])?;
        }
        Ok(())
    }

    /// `work tabs <ws>`: list the tmux windows ("tabs") in the workspace's
    /// session — index, name, pane count, active marker, current command.
    /// Read-only and graceful: a stopped/missing container or absent session
    /// prints a hint instead of erroring.
    pub fn list_tabs(&self) -> Result<()> {
        let ctr = naming::container(&self.cfg.name);
        let session = naming::session(&self.cfg.name);

        match self.engine.container_state(&ctr)? {
            ContainerState::Missing => {
                println!(
                    "workspace '{}' has no container (run `work new {}`)",
                    self.cfg.name, self.cfg.name
                );
                return Ok(());
            }
            ContainerState::Stopped => {
                println!(
                    "workspace '{}' is stopped (run `work {}` to start its session)",
                    self.cfg.name, self.cfg.name
                );
                return Ok(());
            }
            ContainerState::Running => {}
        }

        if !self.engine.session_exists(&ctr, session)? {
            println!(
                "no live session in '{}' (run `work {}` to start one)",
                self.cfg.name, self.cfg.name
            );
            return Ok(());
        }

        let rows = self.tmux_windows()?;

        if rows.is_empty() {
            println!("no windows in session '{}'", self.cfg.name);
            return Ok(());
        }

        println!("Tabs in '{}' (session live):\n", self.cfg.name);
        println!("  IDX  NAME             PANES  ACTIVE  CMD");
        for row in &rows {
            let mark = if row.active { "*" } else { "" };
            println!(
                "  {:>3}  {:<16} {:>5}  {:<6}  {}",
                row.index, row.name, row.panes, mark, row.command
            );
        }
        println!(
            "\nOpen another: `work tab {} [--name <n>]`   ·   switch: Ctrl-b <idx>",
            self.cfg.name
        );
        Ok(())
    }
    /// Raw tmux window rows for a RUNNING workspace with a LIVE session (no gating;
    /// callers check state/session first). Used by both `windows()` and `list_tabs()`
    /// so neither duplicates the docker exec.
    fn tmux_windows(&self) -> Result<Vec<WindowRow>> {
        let ctr = naming::container(&self.cfg.name);
        let session = naming::session(&self.cfg.name);
        let out = self.engine.exec_capture(
            &ctr,
            &["tmux", "list-windows", "-t", session, "-F",
               "#{window_index}\t#{window_name}\t#{window_panes}\t#{window_active}\t#{pane_current_command}"],
        )?;
        Ok(out.lines().filter_map(parse_window_line).collect())
    }

    /// Structured tmux windows ("tabs") for this workspace's session. Self-contained:
    /// returns an empty vec for a stopped/missing container or an absent session, so
    /// TUI callers can render an empty state without special handling.
    pub fn windows(&self) -> Result<Vec<WindowRow>> {
        let ctr = naming::container(&self.cfg.name);
        let session = naming::session(&self.cfg.name);
        if !matches!(self.engine.container_state(&ctr)?, ContainerState::Running) {
            return Ok(Vec::new());
        }
        if !self.engine.session_exists(&ctr, session)? {
            return Ok(Vec::new());
        }
        self.tmux_windows()
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
        import_tmux: Option<ImportSrc>,
        import_starship: Option<ImportSrc>,
        import_dotfiles: Option<std::path::PathBuf>,
        dry_run: bool,
    ) -> Result<UpdateReport> {
        let global = config::load_global()?;
        let dotfiles_dir = import_dotfiles.or(global.import_dotfiles.clone());

        // Per-file imports: a flag overrides the global default (mirrors `create`).
        let rc = config::rc_name(self.cfg.shell.as_deref().unwrap_or("zsh"));
        let seeds: Vec<(std::path::PathBuf, String)> = [
            resolve_import(import_shell, global.import_shell_config.as_deref())
                .map(|s| (s.to_path(rc), format!("/home/dev/{rc}"))),
            resolve_import(import_tmux, global.import_tmux_config.as_deref())
                .map(|s| (s.to_path(".tmux.conf"), "/home/dev/.tmux.conf".into())),
            resolve_import(import_starship, global.import_starship_config.as_deref()).map(|s| {
                (
                    s.to_path(".config/starship.toml"),
                    "/home/dev/.config/starship.toml".into(),
                )
            }),
        ]
        .into_iter()
        .flatten()
        .collect();

        // Validate sources BEFORE touching the container, so a bad path fails fast.
        for (src, _dest) in &seeds {
            if !src.exists() {
                bail!(
                    "config to import not found at {}; pass an explicit --import-* path",
                    src.display()
                );
            }
        }
        if let Some(dir) = &dotfiles_dir {
            if !dir.exists() {
                bail!("dotfiles dir not found at {}", dir.display());
            }
        }

        // Tree to seed: an explicit dotfiles dir, else the embedded templates.
        let templates_tmp;
        let tree_dir: Option<&Path> = match &dotfiles_dir {
            Some(d) => Some(d.as_path()),
            None => {
                templates_tmp = Some(crate::templates::extract_to_tempdir()?);
                Some(templates_tmp.as_ref().unwrap().path())
            }
        };

        // `docker cp` + `chown` need the container up.
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
/// container hash comes from `docker exec sha256sum <dest>` (coreutils, present
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

/// `docker run` options for a workspace container. Sets the `WORK`/`WORKSPACE`
/// identity env, the xdg-open browser shim, and `NERD_FONTS=1`: work's
/// in-container tmux makes agents like omp see `TERM_PROGRAM=tmux` instead of
/// the host terminal, so their Nerd-Font auto-detection fails and they fall
/// back to ASCII glyphs. `NERD_FONTS=1` forces Nerd Font glyphs — the host
/// terminal still renders them. Override per-workspace by unsetting it in your
/// shell rc.
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
            ("BROWSER".into(), crate::browser::SHIM_DEST.into()),
            ("NERD_FONTS".into(), "1".into()),
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
    );
    // Cleanup forwarders on normal/error exit. Ctrl-C is handled by the process
    // group receiving SIGINT -> each `docker run --rm` forwarder stops + removes.
    for fwd in &fwd_names {
        let _ = engine.remove_container(fwd);
    }
    result
}

/// Read URLs from the bridge FIFO forever; for each, auto-bridge a loopback
/// OAuth callback port (if any) then open the URL in the host browser.
fn browse_loop(
    engine: &dyn Engine,
    ctr: &str,
    net: &str,
    ws: &str,
    opener: &str,
    bridged: &mut HashSet<u16>,
    fwd_names: &mut Vec<String>,
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
        match Command::new(opener).arg(url).status() {
            Ok(_) => println!("↗ opened {url}"),
            Err(e) => eprintln!("· could not open {url} via {opener} ({e})"),
        }
    }
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

const ZSHRC_DEFAULT: &str = r#"# Default work prompt. Override: `work new --import-shell-config`.
setopt PROMPT_SUBST
PROMPT='%F{magenta}⬡%f %F{cyan}$WORK%f %F{blue}%~%f %# '
"#;

const BASHRC_DEFAULT: &str = r#"# Default work prompt. Override: `work new --import-shell-config`.
PS1="\[\e[35m\]⬡\[\e[0m\] \[\e[36m\]${WORK}\[\e[0m\] \[\e[34m\]\w\[\e[0m\] $ "
"#;

/// Default rc body for the resolved shell.
fn default_rc(rcname: &str) -> &'static str {
    match rcname {
        ".bashrc" => BASHRC_DEFAULT,
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

const TMUX_CONF_DEFAULT: &str = r#"# Default work tmux config. Override: `work new --import-tmux-config`.
# 256-color terminal + truecolor passthrough so TUI agents (omp, Claude Code)
# render correctly instead of falling back to the 8-color `screen` default.
set -g default-terminal "tmux-256color"
set -ga terminal-overrides ",*256col*:Tc"
# Re-pick up forwarded COLORTERM + NERD_FONTS on attach, so apps detect
# truecolor and render Nerd Font glyphs without needing the in-container
# tmux server to restart.
set -ag update-environment " COLORTERM NERD_FONTS"
# Vim/agent friendly: don't delay Esc.
set -sg escape-time 10
# OSC 52 clipboard sync: copy-mode selections (and inner apps that yank via
# OSC 52, e.g. nvim) land on the host clipboard. `work <ws>` attaches through a
# transparent PTY (docker exec -it ... tmux), so escapes reach the host
# terminal (Ghostty allows clipboard writes by default). `on` lets inner apps
# set it too; Ms is auto-present for xterm* (container TERM is xterm-256color).
set -s set-clipboard on
set -as terminal-features ",xterm*:clipboard"
"#;

/// Ensure `/home/dev/.tmux.conf` exists. If a tmux config was imported (or a
/// dotfiles tree provided one) it is already present — never overwrite. If
/// nothing was imported and the file is absent, write a minimal default that
/// enables 256-color + truecolor + OSC 52 clipboard sync so TUI agents render correctly inside the
/// in-container tmux session.
fn ensure_default_tmux_conf(engine: &dyn Engine, ctr: &str, imported: bool) -> Result<()> {
    let path = "/home/dev/.tmux.conf";
    if engine.exec_capture(ctr, &["test", "-e", path]).is_ok() {
        return Ok(()); // seeded or persisted — leave it alone.
    }
    if imported {
        let _ = engine.exec_capture(ctr, &["touch", path]);
        return Ok(()); // import source was absent; keep empty rather than impose.
    }
    let dir = tempfile::tempdir().context("staging default tmux.conf")?;
    let src = dir.path().join(".tmux.conf");
    std::fs::write(&src, TMUX_CONF_DEFAULT).context("writing default tmux.conf")?;
    engine
        .seed_file(ctr, &src, path)
        .context("seeding default .tmux.conf")?;
    Ok(())
}

fn now_rfc3339() -> String {
    use chrono::Utc;
    Utc::now().to_rfc3339()
}
/// Validate a tmux window ("tab") name. Reject empty (after trim), the tmux
/// target separator `:`, and control characters. PURE.
fn validate_window_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        bail!("tab name cannot be empty");
    }
    if trimmed.contains(':') {
        bail!("tab name cannot contain ':' (tmux uses it as a target separator)");
    }
    if trimmed.chars().any(|c| c.is_control()) {
        bail!("tab name cannot contain control characters");
    }
    Ok(())
}

/// One parsed row of `tmux list-windows -F` output. PURE (built by
/// `parse_window_line`). Public so the TUI can render structured tabs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowRow {
    pub index: String,
    pub name: String,
    pub panes: String,
    pub active: bool,
    pub command: String,
}

/// Parse one `tmux list-windows -F` line (TSV: index, name, panes, active,
/// command). PURE. Returns None on a malformed (under-long) line.
fn parse_window_line(line: &str) -> Option<WindowRow> {
    let mut f = line.split('\t');
    Some(WindowRow {
        index: f.next()?.to_string(),
        name: f.next()?.to_string(),
        panes: f.next()?.to_string(),
        active: f.next()? == "1",
        command: f.next()?.to_string(),
    })
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
                ("BROWSER".to_string(), "/usr/local/bin/xdg-open".to_string()),
                ("NERD_FONTS".to_string(), "1".to_string()),
            ]
        );
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
    fn default_tmux_conf_enables_truecolor() {
        // Must advertise a 256-color tmux terminal and pass truecolor through,
        // so TUI agents render correctly instead of the 8-color `screen` default.
        assert!(TMUX_CONF_DEFAULT.contains("tmux-256color"));
        assert!(TMUX_CONF_DEFAULT.contains("Tc"));
        // And re-pick up the forwarded COLORTERM on attach so apps detect
        // truecolor without a tmux server restart.
        assert!(TMUX_CONF_DEFAULT.contains("COLORTERM"));
    }
    #[test]
    fn default_tmux_conf_enables_clipboard() {
        // OSC 52 sync: copy-mode selections (and inner apps that yank via OSC 52,
        // e.g. nvim) must reach the host clipboard over the transparent `work`
        // attach PTY. `on` (not `external`) so inner apps can set it too.
        assert!(TMUX_CONF_DEFAULT.contains("set -s set-clipboard on"));
        assert!(TMUX_CONF_DEFAULT.contains(":clipboard"));
    }
    #[test]
    fn validate_window_name_rules() {
        assert!(validate_window_name("build").is_ok());
        assert!(validate_window_name("  ok-name  ").is_ok()); // trimmed
                                                              // empty / whitespace-only
        assert!(validate_window_name("").is_err());
        assert!(validate_window_name("   ").is_err());
        // tmux target separator
        assert!(validate_window_name("a:b").is_err());
        // control characters (tab / newline, even mid-name)
        assert!(validate_window_name("a\tb").is_err());
        assert!(validate_window_name("a\nb").is_err());
    }

    #[test]
    fn parse_window_line_fields() {
        let row = parse_window_line("2\tbuild\t3\t0\tnpm").unwrap();
        assert_eq!(row.index, "2");
        assert_eq!(row.name, "build");
        assert_eq!(row.panes, "3");
        assert!(!row.active);
        assert_eq!(row.command, "npm");
    }

    #[test]
    fn parse_window_line_active_marker() {
        let row = parse_window_line("0\tacme\t1\t1\tzsh").unwrap();
        assert!(row.active);
    }

    #[test]
    fn parse_window_line_rejects_malformed() {
        assert!(parse_window_line("").is_none());
        assert!(parse_window_line("only\ttwo").is_none());
    }
    #[test]
    fn container_rel_strips_volume_root() {
        assert_eq!(container_rel("/home/dev/.tmux.conf"), ".tmux.conf");
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
        std::fs::write(root.join(".tmux.conf"), b"x").unwrap();
        std::fs::create_dir_all(root.join(".config")).unwrap();
        std::fs::write(root.join(".config").join("starship.toml"), b"y").unwrap();

        let mut got: Vec<String> = Vec::new();
        walk_tree(root, root, &mut |_host, rel| got.push(rel)).unwrap();
        got.sort();
        assert_eq!(
            got,
            vec![
                ".config/starship.toml".to_string(),
                ".tmux.conf".to_string()
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
