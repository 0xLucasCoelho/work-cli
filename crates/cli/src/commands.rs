use std::process::ExitCode;

use anyhow::Result;
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

pub fn image_build() -> Result<()> {
    let engine = engine::detect()?;
    image::build_default(&*engine)?;
    println!("✓ built {}", config::DEFAULT_IMAGE);
    Ok(())
}

/// `work config <ws>` (Phase 2 will make this interactive). v1 prints the config.
pub fn config_show(name: &str) -> Result<()> {
    let cfg = config::load_workspace(name)?;
    println!("{}", toml::to_string_pretty(&cfg)?);
    Ok(())
}

/// `work fwd` is not implemented in this build (planned for Phase 2).
pub fn fwd_stub(ws: &str, port: u16) -> Result<()> {
    println!("`work fwd` is not implemented in this build (planned for Phase 2).");
    println!("would forward host :{port} -> {ws} :{port}");
    Ok(())
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
