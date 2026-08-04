use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use clap::CommandFactory;
use clap_complete::CompleteEnv;
use clap_complete::engine::{ArgValueCompleter, SubcommandCandidates};

mod commands;
mod completion;
use commands::NewArgs;

#[derive(Parser)]
#[command(
    name = "work",
    version,
    about = "Isolated multi-context session manager — one persistent Linux container per workspace",
    after_help = "Tip: a bare `work <ws>` attaches to (or creates) that workspace's persistent in-container session. Use `work help <command>` for per-command details.",
    add = SubcommandCandidates::new(completion::workspace_subcommand_candidates),
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
    ///
    /// Each workspace is a fully isolated Linux container with its own named
    /// volume at /home/dev and a dedicated network. Your host shell ($SHELL) is
    /// auto-detected and used inside the container.
    ///
    /// Examples: `work new acme`; `work new acme --git-name 'Jane' --git-email j@x.io`;
    /// `work new acme --import-shell-config` (seed ~/.zshrc); `work new acme --image my-work:latest`.
    New(NewArgs),
    /// List workspaces with container state and session liveness.
    ///
    /// Columns: WORKSPACE, STATE (running/stopped/missing), SESSION (live if the
    /// in-container tmux session `work` exists, else —).
    Ls,
    /// Start a workspace container (creates it from config if missing).
    Start {
        /// Workspace to start.
        #[arg(add = ArgValueCompleter::new(completion::complete_workspace))]
        name: String,
    },
    /// Stop a workspace container.
    ///
    /// Ends its in-container session and any running processes; files in the
    /// volume persist. Warns (and asks) if a live session would be ended.
    Stop {
        /// Workspace to stop.
        #[arg(add = ArgValueCompleter::new(completion::complete_workspace))]
        name: String,
    },
    /// Stop every workspace.
    #[command(name = "stop-all")]
    StopAll,
    /// Cockpit: tile every running workspace's session in a host tmux.
    ///
    /// Opens one host tmux session (prefix Ctrl-a) with a window per running
    /// workspace, each attached to its in-container session. Stopped workspaces
    /// are listed in a note. `work all` is an alias.
    Resume,
    /// Alias of `resume`.
    All,
    /// (opt-in) Forward a host port into a workspace for your own logins.
    ///
    /// Bridges 127.0.0.1:<port> on the host to <ws>:<port> — e.g. so a
    /// browser-based OAuth login inside the container can complete. Ctrl-C stops
    /// the bridge.
    ///
    /// Example:
    ///   work fwd acme 8080
    Fwd {
        /// Workspace to forward into.
        #[arg(add = ArgValueCompleter::new(completion::complete_workspace))]
        ws: String,
        /// Port to bridge (used on both host and container).
        port: u16,
    },
    /// Forward URLs that in-container tools open (`xdg-open`/`$BROWSER`) to your
    /// host browser — for OAuth/subscription logins (Claude Code, Cursor CLI, …).
    ///
    /// Installs an `xdg-open` shim in the workspace that sends each http(s) URL
    /// to a volume FIFO; this command reads it and opens each URL in your real
    /// browser. Ctrl-C stops it (the container keeps running).
    ///
    /// Example:
    ///   work browse acme
    Browse {
        /// Workspace whose browser-open requests to forward.
        #[arg(add = ArgValueCompleter::new(completion::complete_workspace))]
        ws: String,
    },
    /// Show a workspace's (non-secret) config; use --edit to open it in $EDITOR.
    ///
    /// On --edit: re-validates the file, re-applies git identity, and recreates
    /// the container if the image changed (gated by the safety policy).
    Config {
        /// Workspace whose config to show or edit.
        #[arg(add = ArgValueCompleter::new(completion::complete_workspace))]
        ws: String,
        /// Open the config in $EDITOR (default vi), then re-apply/recreate.
        #[arg(long)]
        edit: bool,
    },
    /// Build or scaffold workspace images.
    #[command(name = "image")]
    Image {
        #[command(subcommand)]
        action: ImageCmd,
    },
    /// Open a new tmux window ("tab") in a workspace's session and attach to it.
    ///
    /// Each run opens one persistent window that survives closing the terminal
    /// (not `work stop`). Bare `work <ws>` still attaches/resumes into the
    /// existing session; `work tab <ws>` always opens a fresh one. The new tab
    /// becomes the session's active window (other attached clients move to it too,
    /// as with `Ctrl-b c`).
    ///
    /// Examples: `work tab acme`; `work tab acme --name build`.
    Tab {
        /// Workspace to open a new tab in.
        #[arg(add = ArgValueCompleter::new(completion::complete_workspace))]
        ws: String,
        /// Name the new tmux window (default: auto, shows the running command).
        #[arg(long)]
        name: Option<String>,
    },
    /// List the tmux windows ("tabs") in a workspace's session.
    ///
    /// Read-only: index, name, pane count, active marker, current command.
    /// Prints a hint if the container is stopped or has no live session.
    ///
    /// Example: `work tabs acme`.
    Tabs {
        /// Workspace whose tabs to list.
        #[arg(add = ArgValueCompleter::new(completion::complete_workspace))]
        ws: String,
    },
    /// Remove a workspace: container + network + config.
    ///
    /// Keeps the named volume by default (data-safe) — `work new <ws>` then
    /// recreates the container with your files intact. --purge also deletes the
    /// volume (irreversible; requires --yes or an interactive confirm).
    ///
    /// Examples: `work rm acme` keeps the volume; `work rm acme --purge -y` deletes it too.
    Rm {
        /// Workspace to remove.
        #[arg(add = ArgValueCompleter::new(completion::complete_workspace))]
        ws: String,
        /// Also delete the named volume (irreversible; requires --yes).
        #[arg(long)]
        purge: bool,
    },
    /// Isolation + engine sanity check.
    ///
    /// Verifies each workspace is on its own network, mounts only its own
    /// volume, runs non-root, uses the configured image, and publishes no host
    /// ports.
    Doctor,
    /// Bare `work <ws>`: attach to a workspace's persistent session.
    /// External-subcommand name is `args[0]`; trailing tokens are `args[1..]`.
    /// Modeled natively so dynamic completion can offer workspace names for it.
    #[command(external_subcommand)]
    Other(Vec<String>),
}

#[derive(Subcommand)]
enum ImageCmd {
    /// Build the default `work-base:latest`, or a custom image.
    ///
    /// Examples: `work image build` (work-base:latest); `work image build --tag my-work:latest --dockerfile ./Dockerfile.work`.
    Build {
        /// Image tag to build (defaults to work-base:latest).
        #[arg(long)]
        tag: Option<String>,
        /// Path to a Dockerfile (required for a non-default --tag).
        #[arg(long)]
        dockerfile: Option<PathBuf>,
    },
    /// Scaffold a personal workspace Dockerfile (extends work-base) to customize.
    ///
    /// Writes a starter Dockerfile with a working baseline, commented tool
    /// examples, and the glibc/musl gotcha documented. Edit it, then
    /// `work image build`.
    Init {
        /// Where to write the Dockerfile (defaults to ./Dockerfile.work).
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

/// Tokens that are CLI verbs (not workspace names). Used by `normalize_help_arg`.
/// Sourced from the single shared set in work-core so it can never drift.
/// (Flag forms like `--help` are unnecessary here: normalize_help_arg's
/// `!raw[0].starts_with('-')` guard already excludes them.)
const RESERVED: &[&str] = work_core::naming::RESERVED;

/// Rewrite `work <cmd> help` -> `work help <cmd>` (and `work image build help` ->
/// `work help image build`) so the natural trailing-help form works. clap only
/// natively supports `work help <cmd>` and `work <cmd> --help`. PURE / testable.
/// Only fires when the first token is a reserved command name, so it never
/// collides with a real (non-reserved) workspace name.
fn normalize_help_arg(raw: Vec<String>) -> Vec<String> {
    let trailing_help = raw.last().is_some_and(|s| s == "help");
    if raw.len() >= 2
        && trailing_help
        && !raw[0].starts_with('-')
        && RESERVED.contains(&raw[0].as_str())
    {
        let mut out = Vec::with_capacity(raw.len());
        out.push("help".to_string());
        out.extend(raw[..raw.len() - 1].iter().cloned());
        out
    } else {
        raw
    }
}

fn main() -> Result<ExitCode> {
    // Dynamic completion entry point. Reads args_os() directly and exit(0)s when
    // COMPLETE=<shell> is set; returns immediately otherwise. Must run first.
    CompleteEnv::with_factory(|| Cli::command()).complete();
    let raw: Vec<String> = std::env::args().skip(1).collect();
    // Normalize `work <cmd> help` -> `work help <cmd>` so the trailing-help form
    // works (clap only natively does `work help <cmd>` / `work <cmd> --help`).
    let raw = normalize_help_arg(raw);
    // Best-effort update-available awareness (non-blocking, daily-cached).
    // Held to end of main(); drops at exit -> bounded-joined (≤1s) so a stale
    // cache refreshes for short-lived commands. Off in CI/non-TTY.
    let _update_guard = work_core::update::run_check();

    let cli = Cli::parse_from(std::iter::once("work").chain(raw.iter().map(String::as_str)));
    match cli.command {
        None => commands::ls()?,
        Some(Command::New(a)) => commands::new(
            &a.name,
            a.image,
            a.git_name,
            a.git_email,
            a.import_shell_config,
            a.import_tmux_config,
            a.import_starship_config,
            a.import_dotfiles,
            a.use_author_default,
        )?,
        Some(Command::Ls) => commands::ls()?,
        Some(Command::Start { name }) => commands::start(&name)?,
        Some(Command::Stop { name }) => commands::stop(&name, cli.yes)?,
        Some(Command::StopAll) => commands::stop_all(cli.yes)?,
        Some(Command::Resume) | Some(Command::All) => commands::resume()?,
        Some(Command::Fwd { ws, port }) => commands::fwd(&ws, port)?,
        Some(Command::Browse { ws }) => commands::browse(&ws)?,
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
        Some(Command::Tab { ws, name }) => commands::tab(&ws, name.as_deref())?,
        Some(Command::Tabs { ws }) => commands::tabs(&ws)?,
        Some(Command::Rm { ws, purge }) => commands::rm(&ws, purge, cli.yes)?,
        Some(Command::Doctor) => {
            return commands::doctor();
        }
        Some(Command::Other(args)) => {
            // `work <ws>` -> attach. args[0] is the workspace name (validated by shell()).
            let name = args.first().cloned().unwrap_or_default();
            commands::shell(&name)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn trailing_help_becomes_help_subcommand() {
        assert_eq!(normalize_help_arg(v(&["new", "help"])), v(&["help", "new"]));
        assert_eq!(normalize_help_arg(v(&["rm", "help"])), v(&["help", "rm"]));
        assert_eq!(
            normalize_help_arg(v(&["image", "build", "help"])),
            v(&["help", "image", "build"])
        );
    }

    #[test]
    fn bare_workspace_name_not_rewritten() {
        // "acme" is not a command, so `work acme help` stays as-is.
        assert_eq!(
            normalize_help_arg(v(&["acme", "help"])),
            v(&["acme", "help"])
        );
    }

    #[test]
    fn help_subcommand_form_untouched() {
        assert_eq!(normalize_help_arg(v(&["help", "new"])), v(&["help", "new"]));
    }

    fn parse(args: &[&str]) -> Cli {
        Cli::parse_from(std::iter::once("work").chain(args.iter().copied()))
    }

    #[test]
    fn bare_workspace_parses_as_other() {
        let cli = parse(&["acme"]);
        match cli.command {
            Some(Command::Other(args)) => assert_eq!(args, vec!["acme".to_string()]),
            other => panic!(
                "expected Some(Command::Other(_)), got discriminant {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    #[test]
    fn yes_flag_before_name_is_applied() {
        let cli = parse(&["--yes", "acme"]);
        assert!(cli.yes);
        assert!(matches!(cli.command, Some(Command::Other(_))));
    }

    #[test]
    fn named_subcommands_still_match() {
        assert!(matches!(parse(&["new", "x"]).command, Some(Command::New(_))));
        assert!(matches!(parse(&["ls"]).command, Some(Command::Ls)));
        assert!(matches!(parse(&["stop", "x"]).command, Some(Command::Stop { .. })));
        assert!(matches!(parse(&["tab", "x"]).command, Some(Command::Tab { .. })));
    }

    #[test]
    fn bare_work_is_none() {
        assert!(parse(&[]).command.is_none());
    }
}
