//! Interactive dashboard for `work` (ratatui + crossterm).
//!
//! Unreached until Task 2.6 wires `run` into the bare-mode router; silence
//! dead_code module-wide until then.
#![allow(dead_code)]

use std::io::{self, Stdout};

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::{Hide, Show};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
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
        Ok(Self { terminal: Terminal::new(backend)? })
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
            "container engine '{}' is not running; start OrbStack/Docker first (or use `work ls`)",
            engine.binary()
        );
    }

    // Scope the TUI guard so its `Drop` restores the terminal (raw mode off, alt
    // screen left, cursor shown) BEFORE we attach — `attach` spawns an interactive
    // shell that needs a normal TTY. `Drop` fires on early `?` return too, so the
    // terminal is never left in raw mode, even on error.
    let pending = {
        let mut tui = Tui::enter()?;
        let mut app = App::new();
        app.set_model(load_model()?);
        refresh_tabs(&mut app)?;
        run_loop(&mut tui, &mut app, yes)?;
        app.pending_attach().take()
    };
    if let Some(name) = pending {
        commands::attach(&name)?;
    }
    Ok(())
}

fn run_loop(tui: &mut Tui, app: &mut App, yes: bool) -> anyhow::Result<()> {
    use work_core::safety::Severity;
    use work_core::workspace::Workspace;
    use self::app::ConfirmAction;
    const TICK: Duration = Duration::from_millis(250);
    loop {
        tui.draw(|f| render::render(f, app))?;
        if !poll(TICK)? {
            continue; // background-refresh worker wires here in Task 4.3
        }
        // Non-blocking drain: handle all ready events, then redraw.
        loop {
            let Ok(event) = read() else { break; };
            let Event::Key(key) = event else {
                if !poll(Duration::ZERO)? { break; }
                continue;
            };
            if key.kind != KeyEventKind::Press {
                if !poll(Duration::ZERO)? { break; }
                continue;
            }

            // While a confirm is pending, only y / n / Esc / q / Ctrl-C respond.
            if app.confirm().is_some() {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Enter => {
                        if let Some(c) = app.take_confirm() {
                            let r = match c.action {
                                ConfirmAction::Stop => Workspace::open(&c.ws).and_then(|w| w.stop()),
                                ConfirmAction::Remove => Workspace::open(&c.ws).and_then(|w| w.remove(false)),
                            };
                            app.set_status(result_msg(r, &c.ws, verb_for(c.action)));
                        }
                    }
                    KeyCode::Char('n') | KeyCode::Esc => app.cancel_confirm(),
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
                    _ => {}
                }
            } else {
                // Quit keys must work regardless of whether a workspace is selected,
                // so an empty list never traps the user.
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
                    _ => {}
                }
                if let Some(name) = app.selected_name().map(str::to_string) {
                    match key.code {
                        KeyCode::Down | KeyCode::Char('j') => app.move_down(),
                        KeyCode::Up | KeyCode::Char('k') => app.move_up(),
                        KeyCode::Right | KeyCode::Tab | KeyCode::Char('l') => {
                            app.toggle_expand();
                            refresh_tabs(app)?;
                        }
                        KeyCode::Enter => {
                            app.request_attach(name);
                            return Ok(()); // attach after teardown (run() handles it)
                        }
                        KeyCode::Char('s') => {
                            let r = Workspace::open(&name).and_then(|w| w.start());
                            app.set_status(result_msg(r, &name, "started"));
                        }
                        KeyCode::Char('x') => gate(app, &name, yes, Severity::WorkLoss, ConfirmAction::Stop, "Stop", |w| w.stop()),
                        KeyCode::Char('d') => gate(app, &name, yes, Severity::WorkLoss, ConfirmAction::Remove, "Remove", |w| w.remove(false)),
                        _ => {}
                    }
                }
            }
            if !poll(Duration::ZERO)? { break; }
        }
    }
}

fn verb_for(a: self::app::ConfirmAction) -> &'static str {
    match a { self::app::ConfirmAction::Stop => "stopped", self::app::ConfirmAction::Remove => "removed" }
}

fn result_msg(r: anyhow::Result<()>, ws: &str, verb: &str) -> String {
    match r { Ok(()) => format!("{ws}: {verb}"), Err(e) => format!("{ws}: {verb} failed: {e}") }
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
    let live = Workspace::open(name).map(|w| w.has_live_session()).unwrap_or(false);
    match safety::decide(severity, live, true, yes) {
        Action::Proceed => {
            let r = Workspace::open(name).and_then(|w| f(&w));
            app.set_status(result_msg(r, name, verb_for(action)));
        }
        Action::Prompt => app.request_confirm(self::app::Confirm {
            ws: name.to_string(),
            action,
            blurb: format!("{verb} {name}? A live session will end. [y/N]"),
        }),
        Action::Refuse => app.set_status(format!("{name}: {verb} refused (non-interactive; pass --yes)")),
    }
}

fn load_model() -> anyhow::Result<Vec<work_core::workspace::WorkspaceStatus>> {
    work_core::workspace::list_all()
}

/// Fetch tabs for the expanded workspace (if any) via `Workspace::windows()`.
/// Best-effort: a docker error just leaves the previously-known tabs (or none),
/// never crashing the TUI.
fn refresh_tabs(app: &mut App) -> anyhow::Result<()> {
    let name = match app.expanded_name() {
        Some(name) => name.to_string(),
        None => return Ok(()),
    };
    if let Ok(ws) = work_core::workspace::Workspace::open(&name) {
        let tabs = ws.windows().unwrap_or_default();
        app.set_tabs(&name, tabs);
    }
    Ok(())
}
