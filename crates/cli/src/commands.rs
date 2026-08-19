use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;
use std::str::FromStr;

use anyhow::{Context, Result};
use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use work_core::config::ImportSrc;
use work_core::safety::{decide, Action, Severity};
use work_core::{
    config, doctor, engine, image,
    workspace::{self, UpdateReport, Workspace, WorkspaceStatus},
};

#[allow(clippy::too_many_arguments)]
pub fn new(
    name: &str,
    image: Option<String>,
    git_name: Option<String>,
    git_email: Option<String>,
    import_shell_config: Option<String>,
    import_herdr_config: Option<String>,
    import_starship_config: Option<String>,
    import_dotfiles: Option<std::path::PathBuf>,
    use_author_default: bool,
) -> Result<()> {
    new_with_profile(
        name,
        image,
        None,
        None,
        None,
        None,
        None,
        git_name,
        git_email,
        import_shell_config,
        import_herdr_config,
        import_starship_config,
        import_dotfiles,
        use_author_default,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn new_with_profile(
    name: &str,
    image: Option<String>,
    profile: Option<String>,
    shell: Option<String>,
    pids_limit: Option<u32>,
    browser_confirmation: Option<String>,
    browser_profile: Option<String>,
    git_name: Option<String>,
    git_email: Option<String>,
    import_shell_config: Option<String>,
    import_herdr_config: Option<String>,
    import_starship_config: Option<String>,
    import_dotfiles: Option<std::path::PathBuf>,
    use_author_default: bool,
) -> Result<()> {
    // Keep the happy path short: the developer profile and bundled templates
    // are automatic, while copying host state remains an explicit, one-line
    // choice. Non-interactive invocations stay deterministic and never prompt.
    let import_shell_config = if profile.is_none()
        && shell.is_none()
        && image.is_none()
        && import_shell_config.is_none()
        && import_herdr_config.is_none()
        && import_starship_config.is_none()
        && import_dotfiles.is_none()
        && !use_author_default
        && io::stdin().is_terminal()
        && io::stdout().is_terminal()
    {
        print!("Import your detected host shell config into this workspace? [y/N] ");
        io::stdout()
            .flush()
            .context("flushing shell-config prompt")?;
        let mut answer = String::new();
        io::stdin()
            .read_line(&mut answer)
            .context("reading shell-config choice")?;
        if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            Some(String::new())
        } else {
            None
        }
    } else {
        import_shell_config
    };

    // Flag value: None=off, ""=auto (detected rc), <path>=explicit.
    let to_src = |v: Option<String>| match v {
        None => None,
        Some(s) if s.is_empty() => Some(ImportSrc::Auto),
        Some(s) => Some(ImportSrc::Explicit(s.into())),
    };
    let ws = Workspace::create_with_profile(
        name,
        image,
        profile,
        shell,
        pids_limit,
        browser_confirmation
            .as_deref()
            .map(config::BrowserConfirmation::from_str)
            .transpose()?,
        browser_profile
            .as_deref()
            .map(config::BrowserProfile::from_str)
            .transpose()?,
        git_name,
        git_email,
        to_src(import_shell_config),
        to_src(import_herdr_config),
        to_src(import_starship_config),
        import_dotfiles,
        use_author_default,
    )?;
    println!("✓ created workspace '{}'", ws.cfg.name);
    println!();
    println!("Next:");
    println!("  work {name}        # attach to its persistent session");
    println!("  # then install & log into your own tools inside the container");
    Ok(())
}

/// Inspect or change the default declarative profile used for future
/// workspaces. This never installs host packages or edits existing workspaces.
pub fn profile_list() -> Result<()> {
    let global = config::load_global()?;
    let default = global.effective_default_profile();
    println!("default profile: {default}");
    println!("minimal    shells: bash, zsh; tools: git and core utilities");
    println!("developer  shells: bash, zsh, fish; tools: fish, neovim, git and core utilities");
    Ok(())
}

pub fn profile_set_default(name: &str) -> Result<()> {
    // Resolve now so a typo does not get persisted and fail on the next `new`.
    let _ = config::resolve_profile(Some(name), None)?;
    let mut global = config::load_global()?;
    global.default_profile = Some(name.to_string());
    config::save_global(&global)?;
    println!("✓ default profile set to '{name}' for future workspaces");
    Ok(())
}

pub fn shell(name: &str) -> Result<()> {
    let ws = Workspace::open(name)?;
    ws.shell()
}

/// `work daemon status <ws>` remains usable when ordinary workspace open
/// refuses a daemon mismatch. It is inspection-only.
pub fn daemon_status(name: &str) -> Result<()> {
    let status = Workspace::daemon_recovery_status(name)?;
    let recorded = status.recorded_daemon_id.as_deref().unwrap_or("unrecorded");
    println!("workspace: {}", status.workspace);
    println!("recorded daemon: {recorded}");
    println!("active daemon: {}", status.active_daemon_id);
    println!("active resources: {}", status.state.as_str());
    match status.state {
        work_core::workspace::DaemonRecoveryState::CompleteManagedIsolated => {
            println!(
                "rebind is available: run `work daemon rebind {} --confirm`",
                status.workspace
            );
        }
        work_core::workspace::DaemonRecoveryState::Empty => {
            println!("rebind is unavailable: this daemon has no workspace resources; switch back to the recorded daemon.");
        }
        work_core::workspace::DaemonRecoveryState::Conflict => {
            println!("rebind is unavailable: resources are partial, unmanaged, or not isolated; no resources were adopted.");
        }
    }
    Ok(())
}

/// `work daemon rebind <ws> --confirm`: update only the stored daemon ID after
/// the core has re-inspected the active resources. `--confirm` is intentionally
/// separate from global `--yes`: changing this security binding must always be
/// an explicit, visible invocation.
pub fn daemon_rebind(name: &str, confirmed: bool) -> Result<()> {
    if !confirmed {
        anyhow::bail!(
            "refusing to change workspace daemon binding without --confirm; run `work daemon status {name}` first"
        );
    }
    let status = Workspace::rebind_daemon(name)?;
    println!(
        "✓ rebound '{}' to the active daemon after verifying its complete managed isolated resources",
        status.workspace
    );
    Ok(())
}

#[allow(dead_code)] // wired into the dashboard in Task 2.6; near-twin of `shell`
/// `work <ws>`: open the workspace and attach to its persistent in-container session.
pub fn attach(name: &str) -> Result<()> {
    let ws = work_core::workspace::Workspace::open(name)?;
    ws.shell()
}

pub fn start(name: &str) -> Result<()> {
    let ws = Workspace::open(name)?;
    ws.start()?;
    println!("✓ started '{}'", name);
    Ok(())
}

pub fn stop(name: &str, yes: bool) -> Result<()> {
    let ws = Workspace::open(name)?;
    let live = ws.has_live_session().unwrap_or(true);
    confirm(
        Severity::WorkLoss,
        live,
        yes,
        name,
        "stopping will end its running session",
    )?;
    ws.stop()?;
    println!("✓ stopped '{}'", name);
    Ok(())
}

pub fn stop_all(yes: bool) -> Result<()> {
    let names = config::list_workspace_names()?;
    if names.is_empty() {
        println!("no workspaces");
        return Ok(());
    }
    let any_live = names.iter().any(|n| {
        // Assume live on any error (fail-closed): the work-loss prompt must
        // fire when liveness can't be determined.
        Workspace::open(n)
            .and_then(|ws| ws.has_live_session())
            .unwrap_or(true)
    });
    confirm(
        Severity::WorkLoss,
        any_live,
        yes,
        "all",
        "stopping will end running sessions",
    )?;
    for name in &names {
        if let Ok(ws) = Workspace::open(name) {
            match ws.stop() {
                Ok(()) => println!("✓ stopped '{name}'"),
                Err(e) => println!("· '{name}': {e}"),
            }
        }
    }
    Ok(())
}

/// `work rm <ws> [--purge]`.
pub fn rm(name: &str, purge: bool, yes: bool) -> Result<()> {
    let ws = Workspace::open(name)?;
    let live = ws.has_live_session().unwrap_or(true);
    let (sev, desc) = if purge {
        (
            Severity::DataLoss,
            format!("purge volume work-{name}-home (irreversible)"),
        )
    } else {
        (
            Severity::WorkLoss,
            "removing the container will end its running session".to_string(),
        )
    };
    confirm(sev, live, yes, name, &desc)?;
    ws.remove(purge)?;
    if purge {
        println!("removed workspace '{name}' and purged volume work-{name}-home (irreversible).");
    } else {
        println!(
            "removed workspace '{name}' (volume work-{name}-home kept). `work new {name}` recreates it with your files intact; `work rm {name} --purge` deletes the volume."
        );
    }
    Ok(())
}

pub fn ls() -> Result<()> {
    let entries = workspace::list_all()?;
    if entries.is_empty() {
        println!("no workspaces yet; create one with `work new <name>`");
        return Ok(());
    }
    println!("{:<24} {:<8} SESSION", "WORKSPACE", "STATE");
    for WorkspaceStatus {
        name,
        state,
        session_live,
    } in entries
    {
        let label = match state {
            engine::ContainerState::Running => "running",
            engine::ContainerState::Stopped => "stopped",
            engine::ContainerState::Missing => "missing",
        };
        let sess = if session_live { "live" } else { "—" };
        println!("{:<24} {:<8} {sess}", name, label);
    }
    Ok(())
}

/// Bare `work` (interactive TTY): open the dashboard.
pub fn dashboard(yes: bool) -> Result<()> {
    crate::tui::run(yes)
}

pub fn doctor() -> Result<ExitCode> {
    let engine = engine::detect()?;
    let results = doctor::run(&*engine)?;
    let all_ok = doctor::all_ok(&results);
    for r in &results {
        let mark = if r.ok { '✓' } else { '✗' };
        println!("{mark} {:<24} {}", r.label, r.detail);
    }
    println!();
    println!(
        "{}",
        if all_ok {
            "work doctor: OK"
        } else {
            "work doctor: FAILURES (see above)"
        }
    );
    Ok(if all_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

pub fn image_build(tag: Option<&str>, dockerfile: Option<&std::path::Path>) -> Result<()> {
    let engine = engine::detect()?;
    let built = tag.unwrap_or(config::DEFAULT_IMAGE);
    image::build(&*engine, tag, dockerfile)?;
    println!("✓ built {built}");
    Ok(())
}

/// `work image init`: scaffold a personal workspace Dockerfile/Containerfile to customize.
pub fn image_init(output: Option<&std::path::Path>) -> Result<()> {
    let default = std::path::Path::new("Dockerfile.work");
    let path = output.unwrap_or(default);
    image::init_template(path)?;
    println!("✓ wrote {}", path.display());
    println!();
    println!("Edit it, then build and use it:");
    println!(
        "  work image build --tag my-work:latest --dockerfile {}",
        path.display()
    );
    println!("  work new <ws> --image my-work:latest");
    println!("  # or set default_image = \"my-work:latest\" in ~/.config/work/config.toml");
    Ok(())
}

/// `work harden [<ws>|--all]`: recreate workspace container(s) so they pick up
/// the current hardening defaults (pids limit, the managed label) and re-record
/// the image digest. A container created
/// before those defaults shipped — or that drifted — otherwise keeps the old
/// flags forever: a stopped container only `start`s, never recreates. Ends any
/// live session, gated by the safety policy.
pub fn harden(name: Option<&str>, all: bool, yes: bool) -> Result<()> {
    let names: Vec<String> = if all {
        config::list_workspace_names()?
    } else {
        let n = name.ok_or_else(|| {
            anyhow::anyhow!("specify a workspace: `work harden <ws>`, or `work harden --all`")
        })?;
        vec![n.to_string()]
    };
    if names.is_empty() {
        println!("no workspaces to harden");
        return Ok(());
    }
    for n in &names {
        let ws = Workspace::open(n)?;
        confirm(
            Severity::WorkLoss,
            ws.has_live_session().unwrap_or(true),
            yes,
            n,
            "recreating to apply hardening ends its running session",
        )?;
        ws.recreate()?;
        println!("✓ hardened '{n}' (pids limit, managed label)");
    }
    Ok(())
}

/// `work config <ws>`: print the (non-secret) workspace metadata.
pub fn config_show(name: &str) -> Result<()> {
    let cfg = config::load_workspace(name)?;
    println!("{}", toml::to_string_pretty(&cfg)?);
    Ok(())
}

/// `work config <ws> --edit`: open the workspace config in `$EDITOR` (default
/// `vi`), then re-validate, re-apply git identity, and recreate the container
/// if the image changed (gated by the destructive-op safety policy).
pub fn config_edit(name: &str, yes: bool) -> Result<()> {
    let path = config::workspace_config_path(name);
    if !path.exists() {
        anyhow::bail!("workspace '{name}' has no config at {}", path.display());
    }
    let before = config::load_workspace(name)?;

    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "vi".into());
    let status = std::process::Command::new(&editor)
        .arg(&path)
        .status()
        .with_context(|| format!("launching editor '{editor}'"))?;
    if !status.success() {
        anyhow::bail!(
            "editor '{editor}' exited with {:?}; config unchanged",
            status.code()
        );
    }

    // Re-validate the edited file by parsing it.
    let _ = config::load_workspace(name)
        .with_context(|| format!("edited config is invalid; fix {} and retry", path.display()))?;

    let ws = Workspace::open(name)?;
    if ws.cfg.image != before.image || ws.cfg.pids_limit != before.pids_limit {
        confirm(
            Severity::WorkLoss,
            ws.has_live_session().unwrap_or(true),
            yes,
            name,
            "recreating the container will end its running session",
        )?;
        println!("runtime configuration changed; recreating container…");
        ws.recreate()?;
    } else {
        ws.apply_git_identity()?;
    }
    println!("✓ updated '{name}'");
    Ok(())
}

/// Apply validated, non-secret runtime/browser preferences without exposing the
/// config file as the primary UX. PID changes recreate only after the existing
/// live-session policy permits it; browser preferences apply to the next
/// `work browse` process without restarting the workspace.
pub fn config_set_preferences(
    name: &str,
    pids_limit: Option<u32>,
    browser_confirmation: Option<String>,
    browser_profile: Option<String>,
    yes: bool,
) -> Result<()> {
    if pids_limit.is_none() && browser_confirmation.is_none() && browser_profile.is_none() {
        anyhow::bail!("provide at least one preference to change");
    }
    let before = config::load_workspace(name)?;
    let mut cfg = before.clone();
    if let Some(limit) = pids_limit {
        cfg.pids_limit = config::validate_pids_limit(limit)?;
    }
    if let Some(value) = browser_confirmation {
        cfg.browser_confirmation = config::BrowserConfirmation::from_str(&value)?;
    }
    if let Some(value) = browser_profile {
        cfg.browser_profile = config::BrowserProfile::from_str(&value)?;
    }
    let runtime_changed = cfg.pids_limit != before.pids_limit;
    if runtime_changed {
        let ws = Workspace::open(name)?;
        confirm(
            Severity::WorkLoss,
            ws.has_live_session().unwrap_or(true),
            yes,
            name,
            "recreating to apply the PID limit ends its running session",
        )?;
        config::save_workspace(&cfg)?;
        // Re-open only after persisting so the recreated container receives the
        // newly validated runtime setting rather than the stale in-memory cfg.
        Workspace::open(name)?.recreate()?;
    } else {
        config::save_workspace(&cfg)?;
    }
    println!("✓ updated preferences for '{name}'");
    Ok(())
}

struct UpdateSrcs {
    shell: Option<ImportSrc>,
    herdr: Option<ImportSrc>,
    starship: Option<ImportSrc>,
    dotfiles: Option<std::path::PathBuf>,
}

/// `work update [ws] [--all] [--dry-run]`: re-seed managed config files into a
/// running workspace's container in place — no rebuild, no recreate. Source
/// resolution mirrors `work new`: explicit --import-* flags → global config →
/// the embedded templates. Overwrites only managed config files.
pub fn update(args: &UpdateArgs) -> Result<()> {
    let to_src = |v: &Option<String>| match v {
        None => None,
        Some(s) if s.is_empty() => Some(ImportSrc::Auto),
        Some(s) => Some(ImportSrc::Explicit(s.clone().into())),
    };
    let srcs = UpdateSrcs {
        shell: to_src(&args.import_shell_config),
        herdr: to_src(&args.import_herdr_config),
        starship: to_src(&args.import_starship_config),
        dotfiles: args.import_dotfiles.clone(),
    };

    if args.dry_run {
        println!("· dry run — no files will be written\n");
    }

    if args.all {
        let names = config::list_workspace_names()?;
        if names.is_empty() {
            println!("no workspaces to update");
            return Ok(());
        }
        let mut touched = 0usize;
        let mut unchanged = 0usize;
        let mut skipped = 0usize;
        for name in &names {
            match Workspace::open(name).and_then(|ws| {
                ws.update(
                    srcs.shell.clone(),
                    srcs.herdr.clone(),
                    srcs.starship.clone(),
                    srcs.dotfiles.clone(),
                    args.dry_run,
                )
            }) {
                Ok(rep) => {
                    print_report(name, &rep, args.dry_run);
                    touched += rep.touched();
                    unchanged += rep.unchanged.len();
                }
                Err(e) => {
                    eprintln!("· '{name}': {e}");
                    skipped += 1;
                }
            }
        }
        let verb = if args.dry_run { "would sync" } else { "synced" };
        println!(
            "\n{verb} {touched} file(s) across {} workspace(s) ({unchanged} in sync, {skipped} skipped)",
            names.len()
        );
        return Ok(());
    }

    let name = args.ws.as_deref().ok_or_else(|| {
        anyhow::anyhow!("specify a workspace, or use --all to update every workspace")
    })?;
    let ws = Workspace::open(name)?;
    let rep = ws.update(
        srcs.shell,
        srcs.herdr,
        srcs.starship,
        srcs.dotfiles,
        args.dry_run,
    )?;
    print_report(name, &rep, args.dry_run);
    Ok(())
}

/// Print a workspace's update classification: a one-line summary then each file.
fn print_report(name: &str, rep: &UpdateReport, dry_run: bool) {
    let verb = if dry_run { "would update" } else { "updated" };
    let mut parts = Vec::new();
    if !rep.updated.is_empty() {
        parts.push(format!("{} changed", rep.updated.len()));
    }
    if !rep.added.is_empty() {
        parts.push(format!("{} added", rep.added.len()));
    }
    if !rep.unchanged.is_empty() {
        parts.push(format!("{} in sync", rep.unchanged.len()));
    }
    let summary = if parts.is_empty() {
        "no managed files".to_string()
    } else {
        parts.join(", ")
    };
    println!("✓ {verb} '{name}' — {summary}");
    for f in &rep.added {
        println!("  + {f}");
    }
    for f in &rep.updated {
        println!("  ~ {f}");
    }
    for f in &rep.unchanged {
        println!("  = {f}");
    }
}

/// `work fwd <ws> <port>`: opt-in port bridge for the user's own logins.
pub fn fwd(ws: &str, port: u16) -> Result<()> {
    workspace::forward(ws, port)
}

/// `work browse <ws>`: forward URLs tools open inside the container to the
/// host browser (for OAuth/subscription logins). Ctrl-C stops.
pub fn browse(ws: &str) -> Result<()> {
    workspace::browse(ws)
}

/// Apply the destructive-op safety policy. Prompts only when stdin is a TTY;
/// `--yes` skips; non-interactive + no `--yes` refuses with a clear error.
fn confirm(
    severity: Severity,
    has_live_session: bool,
    yes: bool,
    ws: &str,
    describe: &str,
) -> Result<()> {
    let is_tty = std::io::stdin().is_terminal();
    match decide(severity, has_live_session, is_tty, yes) {
        Action::Proceed => Ok(()),
        Action::Prompt => {
            eprint!("'{ws}': {describe}. continue? [y/N] ");
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            match line.trim().to_lowercase().as_str() {
                "y" | "yes" => Ok(()),
                _ => anyhow::bail!("aborted"),
            }
        }
        Action::Refuse => anyhow::bail!(
            "'{ws}': {describe} refused (non-interactive, no --yes). Re-run with --yes/-y to proceed."
        ),
    }
}

#[derive(Args)]
pub struct NewArgs {
    /// Workspace name: lowercase [a-z0-9][a-z0-9-]*, not a reserved command.
    pub name: String,
    /// Container image to use (defaults to the configured default_image).
    #[arg(long)]
    pub image: Option<String>,
    /// Declarative Work-owned profile: minimal or developer. Profiles never import host tools or credentials.
    #[arg(long)]
    pub profile: Option<String>,
    /// Shell supplied by the selected profile. Fish is available only with --profile developer.
    #[arg(long)]
    pub shell: Option<String>,
    /// Per-workspace process/thread cap (64 through 131072). Changing it later recreates the container.
    #[arg(long = "pids-limit")]
    pub pids_limit: Option<u32>,
    /// Browser host confirmation: prompt (default) or trusted (explicit opt-in).
    #[arg(long = "browser-confirmation")]
    pub browser_confirmation: Option<String>,
    /// Browser profile on macOS: guest (default) or default.
    #[arg(long = "browser-profile")]
    pub browser_profile: Option<String>,
    /// Optional git user.name to set inside the workspace.
    #[arg(long = "git-name")]
    pub git_name: Option<String>,
    /// Optional git user.email to set inside the workspace.
    #[arg(long = "git-email")]
    pub git_email: Option<String>,
    /// Copy your shell rc into the workspace (no value = detected ~/.zshrc/~/.bashrc).
    #[arg(long = "import-shell-config", num_args = 0..=1, default_missing_value = "")]
    pub import_shell_config: Option<String>,
    /// Copy a herdr config.toml into the workspace (no value = ~/.config/herdr/config.toml).
    #[arg(long = "import-herdr-config", num_args = 0..=1, default_missing_value = "")]
    pub import_herdr_config: Option<String>,
    /// Copy a starship.toml into the workspace (no value = ~/.config/starship.toml).
    #[arg(long = "import-starship-config", num_args = 0..=1, default_missing_value = "")]
    pub import_starship_config: Option<String>,
    /// Recursively copy a dotfiles directory into the workspace's /home/dev.
    #[arg(long = "import-dotfiles")]
    pub import_dotfiles: Option<std::path::PathBuf>,
    /// Use the author's bundled dotfiles + configured default image.
    #[arg(long = "default")]
    pub use_author_default: bool,
}

#[derive(Args)]
pub struct UpdateArgs {
    /// Workspace whose config to re-sync (omit with --all for every workspace).
    #[arg(add = ArgValueCompleter::new(crate::completion::complete_workspace))]
    pub ws: Option<String>,
    /// Re-sync every workspace.
    #[arg(short, long)]
    pub all: bool,
    /// Preview which files would change; write nothing.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
    /// Copy your shell rc into the workspace (no value = detected rc).
    #[arg(long = "import-shell-config", num_args = 0..=1, default_missing_value = "")]
    pub import_shell_config: Option<String>,
    /// Copy a herdr config.toml into the workspace (no value = ~/.config/herdr/config.toml).
    #[arg(long = "import-herdr-config", num_args = 0..=1, default_missing_value = "")]
    pub import_herdr_config: Option<String>,
    /// Copy a starship.toml into the workspace (no value = ~/.config/starship.toml).
    #[arg(long = "import-starship-config", num_args = 0..=1, default_missing_value = "")]
    pub import_starship_config: Option<String>,
    /// Recursively copy a dotfiles directory into the workspace's /home/dev.
    #[arg(long = "import-dotfiles")]
    pub import_dotfiles: Option<std::path::PathBuf>,
}
