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
    /// Skip all destructive-operation confirmations (script-friendly).
    #[arg(short = 'y', long = "yes", global = true)]
    yes: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Create an isolated workspace: volume + network + container.
    New(NewArgs),
    /// List workspaces, container state, and session liveness.
    Ls,
    /// Start a workspace container.
    Start { name: String },
    /// Stop a workspace container (ends its in-container session).
    Stop { name: String },
    /// Stop every workspace.
    #[command(name = "stop-all")]
    StopAll,
    /// Cockpit: tile all running workspaces' sessions in a host tmux.
    Resume,
    /// (alias of `resume`)
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
    /// Remove a workspace: container + network + config (keeps the volume
    /// unless --purge, which deletes it irreversibly).
    Rm {
        ws: String,
        #[arg(long)]
        purge: bool,
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
    /// Scaffold a personal workspace Dockerfile (extends work-base) to customize.
    Init {
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

// Reserved tokens — if the first arg matches none and isn't a flag, treat it as
// `work <ws>` (attach to its persistent session).
const RESERVED: &[&str] = &[
    "new",
    "all",
    "ls",
    "start",
    "stop",
    "stop-all",
    "resume",
    "rm",
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
        Some(Command::New(a)) => commands::new(
            &a.name,
            a.image,
            a.git_name,
            a.git_email,
            a.import_shell_config,
            a.import_tmux_config,
        )?,
        Some(Command::Ls) => commands::ls()?,
        Some(Command::Start { name }) => commands::start(&name)?,
        Some(Command::Stop { name }) => commands::stop(&name, cli.yes)?,
        Some(Command::StopAll) => commands::stop_all(cli.yes)?,
        Some(Command::Resume) | Some(Command::All) => commands::resume()?,
        Some(Command::Fwd { ws, port }) => commands::fwd(&ws, port)?,
        Some(Command::Config { ws, edit }) => {
            if edit {
                commands::config_edit(&ws, cli.yes)?;
            } else {
                commands::config_show(&ws)?;
            }
        }
        Some(Command::Image { action }) => match action {
            ImageCmd::Build { tag, dockerfile } => {
                commands::image_build(tag.as_deref(), dockerfile.as_deref())?;
            }
            ImageCmd::Init { output } => {
                commands::image_init(output.as_deref())?;
            }
        },
        Some(Command::Rm { ws, purge }) => commands::rm(&ws, purge, cli.yes)?,
        Some(Command::Doctor) => {
            return commands::doctor();
        }
    }
    Ok(ExitCode::SUCCESS)
}
