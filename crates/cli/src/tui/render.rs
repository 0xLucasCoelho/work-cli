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
    let chunks = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(2),
    ])
    .split(area);
    let body = Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(chunks[0]);

    let visible = app.visible_names();
    let selected = app.selected_name();

    // Build the flat item list from the (possibly filtered) visible set, and
    // record the selected workspace's flat index along the way.
    let mut items: Vec<ListItem> = Vec::new();
    let mut selected_idx: Option<usize> = None;
    for name in &visible {
        let Some(w) = app.model().iter().find(|w| &w.name == name) else {
            continue;
        };
        if Some(w.name.as_str()) == selected {
            selected_idx = Some(items.len());
        }
        let dot = if w.session_live { "●" } else { "—" };
        let st = match w.state {
            ContainerState::Running => "running",
            ContainerState::Stopped => "stopped",
            ContainerState::Missing => "missing",
        };
        items.push(ListItem::new(Line::from(format!(
            "{:<16} {:<8} {}",
            w.name, st, dot
        ))));
    }

    let list = List::new(items)
        .block(Block::bordered().title("Workspaces"))
        .highlight_symbol("> ")
        .highlight_style(Style::new().reversed())
        .repeat_highlight_symbol(true);

    // Cursor = the flat index of the selected workspace within the visible
    // list. If the selection is filtered out (or nothing is selected) nothing
    // is highlighted.
    let mut state = ListState::default();
    if let Some(i) = selected_idx {
        state.select(Some(i));
    }
    frame.render_stateful_widget(list, body[0], &mut state);

    let details = match app.selected_details() {
        Some(details) => format!(
            "Workspace: {}\n\nRuntime: {}\nHerdr session: {}\n\nEnter opens Herdr after leaving the dashboard.",
            details.name, details.runtime, details.herdr
        ),
        None if app.is_empty() => "No workspaces yet.\n\nPress n to create one.".to_string(),
        None => "No workspace matches the current filter.".to_string(),
    };
    frame.render_widget(
        Paragraph::new(details).block(Block::bordered().title("Details")),
        body[1],
    );

    let status = if let Some(c) = app.confirm() {
        c.blurb.clone()
    } else if app.mode() == Some(super::app::Mode::Filter) {
        format!("filter: {}  (Enter=apply · Esc=clear)", app.buf_str())
    } else if app.mode() == Some(super::app::Mode::New) {
        format!(
            "new workspace: {}  (Enter=create · Esc=cancel)",
            app.buf_str()
        )
    } else {
        app.status_message().unwrap_or("Ready").to_string()
    };
    frame.render_widget(
        Paragraph::new("↑/k ↓/j move · Enter Herdr · s start · x stop · d remove · n new · / filter · r refresh · q quit"),
        chunks[1],
    );
    frame.render_widget(Paragraph::new(status), chunks[2]);
}
