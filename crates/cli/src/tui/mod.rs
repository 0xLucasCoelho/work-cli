//! Interactive dashboard for `work` (ratatui + crossterm).

use std::io::{self, Stdout};

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::{Hide, Show};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::{CompletedFrame, Frame, Terminal};

mod app;
#[allow(dead_code)] // used from Task 2.5
pub(crate) type Term = Terminal<CrosstermBackend<Stdout>>;

/// Owns the terminal: entering acquires raw mode + the alternate screen; `Drop`
/// restores them unconditionally (best-effort), so a panic or `?` mid-dashboard
/// can never leave the terminal in raw mode. There is no `panic = "abort"` in the
/// workspace, so unwinding runs `Drop`.
#[allow(dead_code)] // used from Task 2.5
pub(crate) struct Tui {
    terminal: Term,
}

#[allow(dead_code)] // used from Task 2.5
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
