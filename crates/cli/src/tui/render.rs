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

    let expanded = app.expanded_name();
    let visible = app.visible_names();
    let selected = app.selected_name();

    // Build the flat item list from the (possibly filtered) visible set, and
    // record the selected workspace's flat index along the way. Tab child rows
    // are appended under their expanded parent but are not selectable, so they
    // push later rows down without changing which NAME is highlighted.
    let mut items: Vec<ListItem> = Vec::new();
    let mut selected_idx: Option<usize> = None;
    for name in &visible {
        let Some(w) = app.model().iter().find(|w| &w.name == name) else { continue; };
        if Some(w.name.as_str()) == selected {
            selected_idx = Some(items.len());
        }
        let dot = if w.session_live { "●" } else { "—" };
        let st = match w.state {
            ContainerState::Running => "running",
            ContainerState::Stopped => "stopped",
            ContainerState::Missing => "missing",
        };
        items.push(ListItem::new(Line::from(format!("{:<16} {:<8} {}", w.name, st, dot))));

        // When this workspace is the expanded one AND its tabs are loaded,
        // append an indented child row per tab beneath it. Tabs not loaded
        // yet just show the workspace row.
        if expanded == Some(w.name.as_str()) {
            if let Some(tabs) = app.expanded_tabs() {
                for t in tabs {
                    let active = if t.active { " (active)" } else { "" };
                    items.push(ListItem::new(Line::from(format!("    · {}{}", t.name, active))));
                }
            }
        }
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
    frame.render_stateful_widget(list, chunks[0], &mut state);

    let footer = if let Some(c) = app.confirm() {
        c.blurb.clone()
    } else if app.mode() == Some(super::app::Mode::Filter) {
        format!("filter: {}  (Enter=apply · Esc=clear)", app.buf_str())
    } else if app.mode() == Some(super::app::Mode::New) {
        format!("new workspace: {}  (Enter=create · Esc=cancel)", app.buf_str())
    } else {
        app.status_message().unwrap_or(
            "Up/Dn move · Enter attach · s start · x stop · d rm · t tab · n new · / filter · r refresh · q/Ctrl-C quit"
        ).to_string()
    };
    frame.render_widget(Paragraph::new(footer), chunks[1]);
}
