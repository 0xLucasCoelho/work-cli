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
    let items: Vec<ListItem> = app
        .model()
        .iter()
        .flat_map(|w| {
            let dot = if w.session_live { "●" } else { "—" };
            let st = match w.state {
                ContainerState::Running => "running",
                ContainerState::Stopped => "stopped",
                ContainerState::Missing => "missing",
            };
            let row = ListItem::new(Line::from(format!("{:<16} {:<8} {}", w.name, st, dot)));

            // When this workspace is the expanded one AND its tabs are loaded,
            // append an indented child row per tab beneath it. Tabs not loaded
            // yet (Task 3.2) just show the workspace row.
            let children: Vec<ListItem> = if expanded == Some(w.name.as_str()) {
                app.expanded_tabs()
                    .map(|tabs| {
                        tabs.iter()
                            .map(|t| {
                                let active = if t.active { " (active)" } else { "" };
                                ListItem::new(Line::from(format!("    · {}{}", t.name, active)))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            std::iter::once(row).chain(children)
        })
        .collect();

    let list = List::new(items)
        .block(Block::bordered().title("Workspaces"))
        .highlight_symbol("> ")
        .highlight_style(Style::new().reversed())
        .repeat_highlight_symbol(true);

    // Selection stays workspace-level, but the flat list now interleaves tab
    // rows under an expanded workspace. Offset the cursor by however many tab
    // rows sit above the selected workspace (i.e. the expanded workspace's tabs,
    // only when it precedes the selected one in the model).
    let mut state = ListState::default();
    if let Some(name) = app.selected_name() {
        let tabs_above = match app.expanded_name() {
            Some(exp) if exp != name => {
                let precedes = app
                    .model()
                    .iter()
                    .position(|w| w.name == exp)
                    .zip(app.model().iter().position(|w| w.name == name))
                    .is_some_and(|(ep, sp)| ep < sp);
                if precedes { app.expanded_tabs().map(|t| t.len()).unwrap_or(0) } else { 0 }
            }
            _ => 0,
        };
        if let Some(i) = app.model().iter().position(|w| w.name == name) {
            state.select(Some(i + tabs_above));
        }
    }
    frame.render_stateful_widget(list, chunks[0], &mut state);

    let footer = if let Some(c) = app.confirm() {
        c.blurb.clone()
    } else {
        app.status_message().unwrap_or(
            "Up/Dn move · Enter attach · s start · x stop · d rm · t tab · n new · / filter · r refresh · q/Ctrl-C quit"
        ).to_string()
    };
    frame.render_widget(Paragraph::new(footer), chunks[1]);
}
