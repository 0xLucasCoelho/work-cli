//! Interactive dashboard for `work` (ratatui + crossterm).
//!
//! Unreached until Task 2.6 wires `run` into the bare-mode router; silence
//! dead_code module-wide until then.
#![allow(dead_code)]

use std::io::{self, Stdout};

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::{Hide, Show};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::{CompletedFrame, Frame, Terminal};

mod app;
mod render;
pub(crate) type Term = Terminal<CrosstermBackend<Stdout>>;

/// Owns the terminal: entering acquires raw mode + the alternate screen; `Drop`
/// restores them unconditionally (best-effort), so a panic or `?` mid-dashboard
/// can never leave the terminal in raw mode. There is no `panic = "abort"` in the
/// workspace, so unwinding runs `Drop`.
pub(crate) struct Tui {
    terminal: Term,
}

impl Tui {
    pub(crate) fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        let backend = CrosstermBackend::new(io::stdout());
        Ok(Self {
            terminal: Terminal::new(backend)?,
        })
    }

    pub(crate) fn draw(&mut self, f: impl FnOnce(&mut Frame)) -> io::Result<CompletedFrame<'_>> {
        self.terminal.draw(f)
    }

    pub(crate) fn terminal(&mut self) -> &mut Term {
        &mut self.terminal
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        // disable raw mode first (most side effects), then leave alt screen, then
        // re-show the cursor (ratatui's built-in restore omits the cursor Show).
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
    }
}

use std::time::Duration;

use ratatui::crossterm::event::{poll, read, Event, KeyCode, KeyEventKind, KeyModifiers};

use self::app::App;
use super::commands;

/// Entry point: probe the engine BEFORE raw mode, enter the TUI, run until quit/attach.
/// `yes` is the global --yes (used in Phase 4 for destructive confirms).
pub(crate) fn run(yes: bool) -> anyhow::Result<()> {
    let engine = work_core::engine::detect()?;
    if !engine.is_running().unwrap_or(false) {
        anyhow::bail!(
            "container engine '{}' is not running; start it first (on macOS/WSL, also start the Podman machine if applicable; or use `work ls`)",
            engine.binary()
        );
    }

    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::sync::Arc;
    // Background refresh worker: ~3 s cadence. Stops on the shared quit flag
    // (checked every 100 ms) or when the receiver is dropped (send error). A
    // refresh ERROR flows as `Err` over the channel so the UI keeps the last
    // good model and shows a transient status — the list is never blanked.
    let (tx, rx) = mpsc::channel::<Result<Vec<work_core::workspace::WorkspaceStatus>, String>>();
    let quit = Arc::new(AtomicBool::new(false));
    let quit_w = quit.clone();
    std::thread::spawn(move || {
        while !quit_w.load(Ordering::Relaxed) {
            let msg = work_core::workspace::list_all().map_err(|e| format!("{e}"));
            if tx.send(msg).is_err() {
                return;
            } // receiver gone -> exit
            for _ in 0..30 {
                // ~3 s, checking quit every 100 ms for prompt exit
                if quit_w.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    });

    let mut app = App::new();
    app.set_model(load_model()?);
    // Attach must run with a normal TTY. When Herdr exits or attach fails, enter
    // the dashboard again and keep the workspace list plus an actionable result.
    // This makes a failed attach recoverable instead of dropping the user into
    // an opaque command-line error.
    loop {
        let pending = {
            let mut tui = Tui::enter()?;
            run_loop(&mut tui, &mut app, yes, &rx)?;
            app.pending().take()
        };
        match pending {
            Some(app::PendingAction::Attach(n)) => {
                app.set_status(attach_result(commands::attach(&n), &n));
            }
            Some(app::PendingAction::Create(n)) => {
                app.set_status(create_result(
                    commands::new(&n, None, None, None, None, None, None, None, false),
                    &n,
                ));
            }
            None => break,
        }
    }
    // Stop the background refresh worker (it also exits when `rx` drops on return).
    quit.store(true, Ordering::Relaxed);
    Ok(())
}

fn run_loop(
    tui: &mut Tui,
    app: &mut App,
    yes: bool,
    rx: &std::sync::mpsc::Receiver<Result<Vec<work_core::workspace::WorkspaceStatus>, String>>,
) -> anyhow::Result<()> {
    use self::app::ConfirmAction;
    use work_core::safety::Severity;
    use work_core::workspace::Workspace;
    const TICK: Duration = Duration::from_millis(250);
    loop {
        // Drain any background refresh updates before drawing so each frame
        // renders the freshest model.
        drain_refresh(rx, app);
        tui.draw(|f| render::render(f, app))?;
        if !poll(TICK)? {
            continue; // background-refresh worker wires here in Task 4.3
        }
        // Non-blocking drain: handle all ready events, then redraw.
        while let Ok(event) = read() {
            let Event::Key(key) = event else {
                if !poll(Duration::ZERO)? {
                    break;
                }
                continue;
            };
            if key.kind != KeyEventKind::Press {
                if !poll(Duration::ZERO)? {
                    break;
                }
                continue;
            }

            // While a confirm is pending, only y / n / Esc / q / Ctrl-C respond.
            if app.confirm().is_some() {
                match key.code {
                    KeyCode::Char('y') => {
                        if let Some(c) = app.take_confirm() {
                            let r = match c.action {
                                ConfirmAction::Stop => {
                                    Workspace::open(&c.ws).and_then(|w| w.stop())
                                }
                                ConfirmAction::Remove => {
                                    Workspace::open(&c.ws).and_then(|w| w.remove(false))
                                }
                            };
                            app.set_status(result_msg(r, &c.ws, action_words(c.action)));
                        }
                    }
                    KeyCode::Char('n') | KeyCode::Esc | KeyCode::Enter => app.cancel_confirm(),
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(())
                    }
                    _ => {}
                }
            } else {
                // Quit keys work regardless of selection. When an input mode is
                // active, Esc cancels input (handled in the input branch) rather
                // than quitting — so only quit on Esc when no mode is active.
                match key.code {
                    KeyCode::Char('q') if app.mode().is_none() => return Ok(()),
                    KeyCode::Esc if app.mode().is_none() => return Ok(()),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(())
                    }
                    _ => {}
                }
                // Inline text-input mode (e.g. new-workspace name entry).
                if app.mode().is_some() {
                    match key.code {
                        KeyCode::Char(c) => app.buf_push(c),
                        KeyCode::Backspace => app.buf_pop(),
                        KeyCode::Enter => match app.mode() {
                            Some(app::Mode::Filter) => {
                                let f = app.buf_take();
                                app.cancel_mode();
                                app.set_filter(Some(f));
                            }
                            Some(app::Mode::New) => {
                                let name = app.buf_take();
                                app.cancel_mode();
                                if let Err(e) = work_core::naming::validate_name(&name) {
                                    app.set_status(format!("invalid name: {e}"));
                                } else {
                                    app.request_create(name);
                                    return Ok(()); // create+attach after teardown
                                }
                            }
                            None => {}
                        },
                        KeyCode::Esc => match app.mode() {
                            Some(app::Mode::Filter) => {
                                app.cancel_mode();
                                app.set_filter(None);
                            }
                            _ => app.cancel_mode(),
                        },
                        _ => {}
                    }
                } else {
                    // `n` starts a new workspace (needs no selection).
                    if key.code == KeyCode::Char('n') {
                        app.enter_mode(app::Mode::New);
                    } else if key.code == KeyCode::Char('/') {
                        app.enter_mode(app::Mode::Filter);
                    } else if key.code == KeyCode::Char('r') {
                        match load_model() {
                            Ok(m) => {
                                app.set_model(m);
                                app.set_status("refreshed");
                            }
                            Err(e) => app.set_status(format!("refresh failed: {e}")),
                        }
                    } else if let Some(name) = app.selected_name().map(str::to_string) {
                        match key.code {
                            KeyCode::Down | KeyCode::Char('j') => app.move_down(),
                            KeyCode::Up | KeyCode::Char('k') => app.move_up(),
                            KeyCode::Enter => {
                                app.request_attach(name);
                                return Ok(()); // attach after teardown (run() handles it)
                            }
                            KeyCode::Char('s') => {
                                let r = Workspace::open(&name).and_then(|w| w.start());
                                app.set_status(result_msg(r, &name, ("start", "started")));
                            }
                            KeyCode::Char('x') => gate(
                                app,
                                &name,
                                yes,
                                Severity::WorkLoss,
                                ConfirmAction::Stop,
                                "Stop",
                                |w| w.stop(),
                            ),
                            KeyCode::Char('d') => gate(
                                app,
                                &name,
                                yes,
                                Severity::WorkLoss,
                                ConfirmAction::Remove,
                                "Remove",
                                |w| w.remove(false),
                            ),
                            _ => {}
                        }
                    }
                }
            }
            if !poll(Duration::ZERO)? {
                break;
            }
        }
    }
}

fn action_words(a: self::app::ConfirmAction) -> (&'static str, &'static str) {
    match a {
        self::app::ConfirmAction::Stop => ("stop", "stopped"),
        self::app::ConfirmAction::Remove => ("remove", "removed"),
    }
}

fn result_msg(r: anyhow::Result<()>, ws: &str, words: (&str, &str)) -> String {
    let (infinitive, completed) = words;
    match r {
        Ok(()) => format!("{ws}: {completed}. Status will refresh shortly."),
        Err(e) => format!(
            "{ws}: couldn't {infinitive} — {e}. Run `work doctor`, resolve the reported issue, then retry."
        ),
    }
}

fn attach_result(r: anyhow::Result<()>, ws: &str) -> String {
    match r {
        Ok(()) => format!("{ws}: Herdr session closed; back at the dashboard."),
        Err(e) => format!(
            "{ws}: couldn't open Herdr — {e}. Check `work doctor`, start the workspace if needed, then press Enter to retry."
        ),
    }
}

fn create_result(r: anyhow::Result<()>, ws: &str) -> String {
    match r {
        Ok(()) => format!("{ws}: created. Select it and press Enter to open Herdr."),
        Err(e) => format!(
            "{ws}: couldn't create workspace — {e}. Correct the issue, then press n to retry."
        ),
    }
}

/// Apply the destructive-op safety policy. Proceed runs now; Prompt queues an
/// inline confirm; Refuse (non-interactive w/o --yes) reports a status.
fn gate(
    app: &mut self::app::App,
    name: &str,
    yes: bool,
    severity: work_core::safety::Severity,
    action: self::app::ConfirmAction,
    verb: &str,
    f: impl FnOnce(&work_core::workspace::Workspace) -> anyhow::Result<()>,
) {
    use work_core::safety::{self, Action};
    use work_core::workspace::Workspace;
    let live = Workspace::open(name)
        .and_then(|w| w.has_live_session())
        .unwrap_or(true);
    match safety::decide(severity, live, true, yes) {
        Action::Proceed => {
            let r = Workspace::open(name).and_then(|w| f(&w));
            app.set_status(result_msg(r, name, action_words(action)));
        }
        Action::Prompt => app.request_confirm(self::app::Confirm {
            ws: name.to_string(),
            action,
            blurb: format!("{verb} {name}? A live session will end. [y/N]"),
        }),
        Action::Refuse => app.set_status(format!(
            "{name}: {verb} refused (non-interactive; pass --yes)"
        )),
    }
}

fn load_model() -> anyhow::Result<Vec<work_core::workspace::WorkspaceStatus>> {
    work_core::workspace::list_all()
}

/// Drain all pending background-refresh messages: `Ok` reconciles the model
/// (name-keyed, so selection survives); `Err` keeps the last good model and
/// surfaces a transient status — the list is never blanked.
fn drain_refresh(
    rx: &std::sync::mpsc::Receiver<Result<Vec<work_core::workspace::WorkspaceStatus>, String>>,
    app: &mut App,
) {
    while let Ok(msg) = rx.try_recv() {
        match msg {
            Ok(model) => {
                app.set_model(model);
            }
            Err(_) => app.set_status("refresh failed — showing last state"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_errors_name_a_recovery_step() {
        let message = result_msg(
            Err(anyhow::anyhow!("engine unavailable")),
            "docs",
            ("start", "started"),
        );

        assert!(message.contains("couldn't start"));
        assert!(message.contains("work doctor"));
        assert!(message.contains("retry"));
    }

    #[test]
    fn attach_errors_keep_a_clear_retry_path() {
        let message = attach_result(Err(anyhow::anyhow!("workspace is stopped")), "docs");

        assert!(message.contains("couldn't open Herdr"));
        assert!(message.contains("start the workspace"));
        assert!(message.contains("Enter to retry"));
    }
}
