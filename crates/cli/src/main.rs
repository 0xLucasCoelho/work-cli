use std::io::{self, IsTerminal, Write};

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use work_core::config;
use work_core::doctor;
use work_core::engine::{self, ContainerState};
use work_core::naming;
use work_core::safety::{self, Action, Severity};
use work_core::workspace::{self, Workspace};

#[derive(Parser)]
#[command(name = "work", version, about = "Isolated multi-company environments")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create an isolated company environment
    New {
        name: String,
        #[arg(long)]
        image: Option<String>,
        #[arg(long)]
        git_name: Option<String>,
        #[arg(long)]
        git_email: Option<String>,
    },
    /// List companies
    Ls,
    /// Start a stopped company box
    Start { name: String },
    /// Stop a running company box (files persist)
    Stop {
        name: String,
        #[arg(long)]
        yes: bool,
    },
    /// Remove container + net (+ volume with --purge)
    Rm {
        name: String,
        #[arg(long)]
        purge: bool,
        #[arg(long)]
        yes: bool,
    },
    /// Verify isolation for every company and the daemon
    Doctor,
    /// Attach to the in-box session
    Attach { name: String },
    /// Build or pull images
    #[command(subcommand)]
    Image(ImageCmd),
}

#[derive(Subcommand)]
enum ImageCmd {
    /// Build the default work-base image
    Build,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 2 && !args[1].starts_with('-') && naming::validate_name(&args[1]).is_ok() {
        if config::workspace_exists(&args[1]) {
            return Workspace::open(&args[1])?.attach();
        }
        bail!("workspace '{}' not found (work new {})", args[1], args[1]);
    }

    match Cli::parse().cmd {
        None => print_list(),
        Some(Cmd::New {
            name,
            image,
            git_name,
            git_email,
        }) => {
            Workspace::create(&name, image, git_name, git_email)?;
            println!("created {name}  (attach: work {name})");
            Ok(())
        }
        Some(Cmd::Ls) => print_list(),
        Some(Cmd::Start { name }) => {
            Workspace::open(&name)?.start()?;
            println!("started {name}");
            Ok(())
        }
        Some(Cmd::Stop { name, yes }) => {
            let ws = Workspace::open(&name)?;
            gate(Severity::WorkLoss, ws.has_live_session()?, yes)?;
            ws.stop()?;
            println!("stopped {name}");
            Ok(())
        }
        Some(Cmd::Rm { name, purge, yes }) => {
            let ws = Workspace::open(&name)?;
            let sev = if purge {
                Severity::DataLoss
            } else {
                Severity::WorkLoss
            };
            gate(sev, ws.has_live_session()?, yes)?;
            ws.remove(purge)?;
            if purge {
                println!("removed {name} and purged volume");
            } else {
                println!("removed {name} (volume kept)");
            }
            Ok(())
        }
        Some(Cmd::Doctor) => {
            let engine = engine::detect()?;
            let results = doctor::run(&*engine)?;
            let mut failed = 0;
            for r in &results {
                let mark = if r.ok { "ok" } else { "FAIL" };
                println!("{mark:4}  {}  {}", r.label, r.detail);
                if !r.ok {
                    failed += 1;
                }
            }
            if failed > 0 {
                bail!("{failed} isolation check(s) failed");
            }
            Ok(())
        }
        Some(Cmd::Attach { name }) => Workspace::open(&name)?.attach(),
        Some(Cmd::Image(ImageCmd::Build)) => {
            workspace::build_default_image()?;
            println!("built {}", config::DEFAULT_IMAGE);
            Ok(())
        }
    }
}

fn print_list() -> Result<()> {
    let rows = workspace::list_all()?;
    if rows.is_empty() {
        println!("(no companies)");
        return Ok(());
    }
    println!("{:16} {:10} SESSION", "COMPANY", "STATE");
    for r in rows {
        let state = match r.state {
            ContainerState::Running => "running",
            ContainerState::Stopped => "stopped",
            ContainerState::Missing => "missing",
        };
        let sess = if r.session_live { "live" } else { "—" };
        println!("{:16} {:10} {sess}", r.name, state);
    }
    Ok(())
}

fn gate(severity: Severity, live: bool, yes: bool) -> Result<()> {
    let tty = io::stdin().is_terminal();
    match safety::decide(severity, live, tty, yes) {
        Action::Proceed => Ok(()),
        Action::Refuse => bail!("refusing destructive action without --yes (not a TTY)"),
        Action::Prompt => {
            let what = match severity {
                Severity::DataLoss => "This deletes the company volume. Type y to continue: ",
                Severity::WorkLoss => "A live session will be killed. Type y to continue: ",
            };
            eprint!("{what}");
            let _ = io::stderr().flush();
            let mut line = String::new();
            io::stdin().read_line(&mut line)?;
            if line.trim().eq_ignore_ascii_case("y") {
                Ok(())
            } else {
                bail!("aborted");
            }
        }
    }
}
