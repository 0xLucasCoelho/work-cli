use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Args;

use work_core::{
    config, doctor, engine, image,
    workspace::{self, Workspace, WorkspaceStatus},
};

pub fn new(
    name: &str,
    image: Option<String>,
    git_name: Option<String>,
    git_email: Option<String>,
) -> Result<()> {
    let ws = Workspace::create(name, image, git_name, git_email)?;
    println!("✓ created workspace '{}'", ws.cfg.name);
    println!();
    println!("Next:");
    println!("  work {name}        # shell into it");
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

pub fn stop(name: &str) -> Result<()> {
    let ws = Workspace::open(name)?;
    ws.stop()?;
    println!("✓ stopped '{}'", name);
    Ok(())
}

pub fn stop_all() -> Result<()> {
    let names = config::list_workspace_names()?;
    if names.is_empty() {
        println!("no workspaces");
        return Ok(());
    }
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

pub fn ls() -> Result<()> {
    let entries = workspace::list_all()?;
    if entries.is_empty() {
        println!("no workspaces yet; create one with `work new <name>`");
        return Ok(());
    }
    println!("{:<24} STATE", "WORKSPACE");
    for WorkspaceStatus { name, state } in entries {
        let label = match state {
            engine::ContainerState::Running => "running",
            engine::ContainerState::Stopped => "stopped",
            engine::ContainerState::Missing => "missing",
        };
        println!("{:<24} {label}", name);
    }
    Ok(())
}

pub fn all() -> Result<()> {
    workspace::tmux_all()
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

/// `work config <ws>`: print the (non-secret) workspace metadata.
pub fn config_show(name: &str) -> Result<()> {
    let cfg = config::load_workspace(name)?;
    println!("{}", toml::to_string_pretty(&cfg)?);
    Ok(())
}

/// `work config <ws> --edit`: open the workspace config in `$EDITOR` (default
/// `vi`), then re-validate, re-apply git identity, and recreate the container
/// if the image changed.
pub fn config_edit(name: &str) -> Result<()> {
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

#[derive(Args)]
pub struct NewArgs {
    pub name: String,
    #[arg(long)]
    pub image: Option<String>,
    #[arg(long = "git-name")]
    pub git_name: Option<String>,
    #[arg(long = "git-email")]
    pub git_email: Option<String>,
}
