use std::io::IsTerminal;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Args;

use work_core::config::ImportSrc;
use work_core::safety::{decide, Action, Severity};
use work_core::{
    config, doctor, engine, image,
    workspace::{self, Workspace, WorkspaceStatus},
};

#[allow(clippy::too_many_arguments)]
pub fn new(
    name: &str,
    image: Option<String>,
    git_name: Option<String>,
    git_email: Option<String>,
    import_shell_config: Option<String>,
    import_tmux_config: Option<String>,
    import_starship_config: Option<String>,
    import_dotfiles: Option<std::path::PathBuf>,
    use_author_default: bool,
) -> Result<()> {
    // Flag value: None=off, ""=auto (detected rc), <path>=explicit.
    let to_src = |v: Option<String>| match v {
        None => None,
        Some(s) if s.is_empty() => Some(ImportSrc::Auto),
        Some(s) => Some(ImportSrc::Explicit(s.into())),
    };
    let ws = Workspace::create(
        name,
        image,
        git_name,
        git_email,
        to_src(import_shell_config),
        to_src(import_tmux_config),
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

pub fn shell(name: &str) -> Result<()> {
    let ws = Workspace::open(name)?;
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
    let live = ws.has_live_session();
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
        Workspace::open(n)
            .map(|ws| ws.has_live_session())
            .unwrap_or(false)
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
    let live = ws.has_live_session();
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

/// `work resume` / `work all`: host tmux cockpit tiling running sessions.
pub fn resume() -> Result<()> {
    workspace::resume()
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

/// `work image init`: scaffold a personal workspace Dockerfile to customize.
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
    if ws.cfg.image != before.image {
        confirm(
            Severity::WorkLoss,
            ws.has_live_session(),
            yes,
            name,
            "recreating the container will end its running session",
        )?;
        println!(
            "image changed ({} -> {}); recreating container…",
            before.image, ws.cfg.image
        );
        ws.recreate()?;
    } else {
        ws.apply_git_identity()?;
    }
    println!("✓ updated '{name}'");
    Ok(())
}

/// `work fwd <ws> <port>`: opt-in port bridge for the user's own logins.
pub fn fwd(ws: &str, port: u16) -> Result<()> {
    workspace::forward(ws, port)
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
    /// Optional git user.name to set inside the workspace.
    #[arg(long = "git-name")]
    pub git_name: Option<String>,
    /// Optional git user.email to set inside the workspace.
    #[arg(long = "git-email")]
    pub git_email: Option<String>,
    /// Copy your shell rc into the workspace (no value = detected ~/.zshrc/~/.bashrc).
    #[arg(long = "import-shell-config", num_args = 0..=1, default_missing_value = "")]
    pub import_shell_config: Option<String>,
    /// Copy a .tmux.conf into the workspace (no value = ~/.tmux.conf).
    #[arg(long = "import-tmux-config", num_args = 0..=1, default_missing_value = "")]
    pub import_tmux_config: Option<String>,
    /// Copy a starship.toml into the workspace (no value = ~/.config/starship.toml).
    #[arg(long = "import-starship-config", num_args = 0..=1, default_missing_value = "")]
    pub import_starship_config: Option<String>,
    /// Recursively copy a dotfiles directory into the workspace's /home/dev.
    #[arg(long = "import-dotfiles")]
    pub import_dotfiles: Option<std::path::PathBuf>,
    /// Seed the author's bundled dotfile templates (from the repo) into the workspace.
    #[arg(long = "use-author-default")]
    pub use_author_default: bool,
}
