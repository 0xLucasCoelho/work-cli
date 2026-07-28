use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};

mod commands;
use commands::NewArgs;

#[derive(Parser)]
#[command(
    name = "work",
    version,
    about = "Isolated multi-context session manager"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Create an isolated workspace: volume + network + container.
    New(NewArgs),
    /// List workspaces and container state.
    Ls,
    /// Start a workspace container.
    Start { name: String },
    /// Stop a workspace container.
    Stop { name: String },
    /// Stop every workspace.
    #[command(name = "stop-all")]
    StopAll,
    /// Open all workspaces in a tmux session named `work`.
    All,
    /// (opt-in) forward a host port into a workspace for your own logins.
    Fwd { ws: String, port: u16 },
    /// Show a workspace's config; use --edit to open it in $EDITOR.
    Config {
        ws: String,
        #[arg(long)]
        edit: bool,
    },
    /// Build/rebuild images.
    #[command(name = "image")]
    Image {
        #[command(subcommand)]
        action: ImageCmd,
    },
    /// Isolation + engine sanity check.
    Doctor,
}

#[derive(Subcommand)]
enum ImageCmd {
    /// Build the default `work-base:latest` image, or a custom one with --tag/--dockerfile.
    Build {
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        dockerfile: Option<PathBuf>,
    },
}

// Reserved tokens — if the first arg matches none and isn't a flag, treat it as
// `work <ws>` (interactive shell).
const RESERVED: &[&str] = &[
    "new",
    "all",
    "ls",
    "start",
    "stop",
    "stop-all",
    "fwd",
    "config",
    "image",
    "doctor",
    "help",
    "version",
    "--help",
    "-h",
    "--version",
    "-V",
];

fn main() -> Result<ExitCode> {
    let raw: Vec<String> = std::env::args().skip(1).collect();

    // Bare workspace name dispatch: `work <ws>`.
    if let Some(first) = raw.first() {
        if !first.starts_with('-') && !RESERVED.contains(&first.as_str()) {
            commands::shell(first)?;
            return Ok(ExitCode::SUCCESS);
        }
    }

    let cli = Cli::parse();
    match cli.command {
        None => commands::ls()?,
        Some(Command::New(a)) => commands::new(&a.name, a.image, a.git_name, a.git_email)?,
        Some(Command::Ls) => commands::ls()?,
        Some(Command::Start { name }) => commands::start(&name)?,
        Some(Command::Stop { name }) => commands::stop(&name)?,
        Some(Command::StopAll) => commands::stop_all()?,
        Some(Command::All) => commands::all()?,
        Some(Command::Fwd { ws, port }) => commands::fwd(&ws, port)?,
        Some(Command::Config { ws, edit }) => {
            if edit {
                commands::config_edit(&ws)?;
            } else {
                commands::config_show(&ws)?;
            }
        }
        Some(Command::Image { action }) => match action {
            ImageCmd::Build { tag, dockerfile } => {
                commands::image_build(tag.as_deref(), dockerfile.as_deref())?;
            }
        },
        Some(Command::Doctor) => {
            return commands::doctor();
        }
    }
    Ok(ExitCode::SUCCESS)
}
