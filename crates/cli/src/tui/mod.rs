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
pub(crate) fn run(_yes: bool) -> anyhow::Result<()> {
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
        run_loop(&mut tui, &mut app)?;
        app.pending_attach().take()
    };
    if let Some(name) = pending {
        commands::attach(&name)?;
    }
    Ok(())
}

fn run_loop(tui: &mut Tui, app: &mut App) -> anyhow::Result<()> {
    const TICK: Duration = Duration::from_millis(250);
    loop {
        tui.draw(|f| render::render(f, app))?;
        if !poll(TICK)? {
            continue; // tick: background-refresh channel drains here in Phase 4
        }
        while let Ok(event) = read() {
            if let Event::Key(key) = event {
                if key.kind != KeyEventKind::Press { continue; }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
                    KeyCode::Down | KeyCode::Char('j') => app.move_down(),
                    KeyCode::Up | KeyCode::Char('k') => app.move_up(),
                    KeyCode::Enter => {
                        if let Some(name) = app.selected_name() {
                            app.request_attach(name.to_string());
                            return Ok(());
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn load_model() -> anyhow::Result<Vec<work_core::workspace::WorkspaceStatus>> {
    work_core::workspace::list_all()
}
