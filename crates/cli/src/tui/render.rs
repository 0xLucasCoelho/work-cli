//! Dashboard view: a stateful workspace List driven by name-keyed selection,
//! plus a one-line footer of key hints (or the latest status message).

use ratatui::layout::{Constraint, Layout};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use super::app::App;
use work_core::engine::ContainerState;

pub(crate) fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let chunks = Layout::vertical([Constraint::Min(3), Constraint::Length(2)]).split(area);

    let items: Vec<ListItem> = app.model().iter().map(|w| {
        let dot = if w.session_live { "●" } else { "—" };
        let st = match w.state {
            ContainerState::Running => "running",
            ContainerState::Stopped => "stopped",
            ContainerState::Missing => "missing",
        };
        ListItem::new(Line::from(format!("{:<16} {:<8} {}", w.name, st, dot)))
    }).collect();

    let list = List::new(items)
        .block(Block::bordered().title("Workspaces"))
        .highlight_symbol("> ")
        .highlight_style(Style::new().reversed())
        .repeat_highlight_symbol(true);

    let mut state = ListState::default();
    if let Some(name) = app.selected_name() {
        if let Some(i) = app.model().iter().position(|w| w.name == name) {
            state.select(Some(i));
        }
    }
    frame.render_stateful_widget(list, chunks[0], &mut state);

    let footer = app.status_message().unwrap_or(
        "Up/Dn move · Enter attach · s start · x stop · d rm · t tab · n new · / filter · r refresh · q/Ctrl-C quit"
    );
    frame.render_widget(Paragraph::new(footer), chunks[1]);
}
