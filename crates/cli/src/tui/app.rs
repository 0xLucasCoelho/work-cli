//! Pure dashboard state. Selection is keyed by workspace NAME (not index) so a
//! concurrent work rm/new (changes length AND order) can't shift the cursor onto
//! the wrong workspace. No IO, no rendering.

#[cfg(test)]
use work_core::engine::ContainerState;
use std::collections::HashMap;
use work_core::workspace::{WindowRow, WorkspaceStatus};

pub(crate) struct App {
    model: Vec<WorkspaceStatus>,
    selected: Option<String>, // workspace NAME under the cursor
    quit: bool,
    status: Option<String>,
    pending_attach: Option<String>, // workspace NAME to attach after TUI teardown
    expanded: Option<String>, // workspace NAME that's expanded to show its tabs
    tabs: HashMap<String, Vec<WindowRow>>, // cached tab rows per workspace NAME
}

impl App {
    pub(crate) fn new() -> Self {
        Self {
            model: Vec::new(),
            selected: None,
            quit: false,
            status: None,
            pending_attach: None,
            expanded: None,
            tabs: HashMap::new(),
        }
    }

    /// Replace the model from a refresh, re-resolving the cursor by NAME. If the
    /// previously-selected name is gone, fall back to the nearest surviving
    /// neighbor (preserve list position), clamped to the new bounds.
    pub(crate) fn set_model(&mut self, model: Vec<WorkspaceStatus>) {
        let prev_index = self.selected.as_deref()
            .and_then(|n| self.model.iter().position(|w| w.name == n));
        self.model = model;
        self.selected = match &self.model {
            empty if empty.is_empty() => None,
            m => {
                let by_name = self.selected.as_deref()
                    .and_then(|n| m.iter().position(|w| &w.name == n));
                let idx = by_name
                    .or_else(|| prev_index.map(|i| i.min(m.len() - 1)))
                    .unwrap_or(0);
                Some(m[idx].name.clone())
            }
        };
    }

    pub(crate) fn selected_name(&self) -> Option<&str> { self.selected.as_deref() }

    pub(crate) fn selected_status(&self) -> Option<&WorkspaceStatus> {
        self.selected.as_deref().and_then(|n| self.model.iter().find(|w| &w.name == n))
    }

    pub(crate) fn model(&self) -> &[WorkspaceStatus] { &self.model }

    fn cursor_index(&self) -> Option<usize> {
        self.selected.as_deref().and_then(|n| self.model.iter().position(|w| &w.name == n))
    }

    pub(crate) fn move_up(&mut self) {
        if let Some(i) = self.cursor_index().filter(|&i| i > 0) {
            self.selected = Some(self.model[i - 1].name.clone());
        }
    }

    pub(crate) fn move_down(&mut self) {
        if let Some(i) = self.cursor_index() {
            if i + 1 < self.model.len() {
                self.selected = Some(self.model[i + 1].name.clone());
            }
        }
    }

    pub(crate) fn quit(&mut self) { self.quit = true; }
    pub(crate) fn should_quit(&self) -> bool { self.quit }

    pub(crate) fn set_status(&mut self, msg: impl Into<String>) { self.status = Some(msg.into()); }
    pub(crate) fn status_message(&self) -> Option<&str> { self.status.as_deref() }

    pub(crate) fn request_attach(&mut self, name: String) { self.pending_attach = Some(name); }
    pub(crate) fn pending_attach(&mut self) -> &mut Option<String> { &mut self.pending_attach }

    /// Toggle the expanded state of the workspace under the cursor. Expanding a
    /// second workspace collapses the first (only one expanded at a time). With
    /// nothing selected this is a no-op.
    pub(crate) fn toggle_expand(&mut self) {
        let Some(name) = self.selected_name() else { return; };
        self.expanded =
            if self.expanded.as_deref() == Some(name) { None } else { Some(name.to_string()) };
    }

    /// Name of the workspace whose tab rows are currently shown, if any.
    pub(crate) fn expanded_name(&self) -> Option<&str> { self.expanded.as_deref() }

    /// Cached tab rows for the expanded workspace, if any. `Some([])` is a loaded
    /// workspace with zero tabs; `None` means "not loaded yet" (Task 3.2 fetches).
    pub(crate) fn expanded_tabs(&self) -> Option<&[WindowRow]> {
        self.expanded.as_deref().and_then(|n| self.tabs.get(n)).map(|v| v.as_slice())
    }

    /// Cache (or replace) the tab rows for a workspace, keyed by NAME.
    pub(crate) fn set_tabs(&mut self, ws: &str, tabs: Vec<WindowRow>) {
        self.tabs.insert(ws.to_string(), tabs);
    }

    pub(crate) fn is_empty(&self) -> bool { self.model.is_empty() }
}
#[cfg(test)]
fn ws(name: &str) -> WorkspaceStatus {
    WorkspaceStatus { name: name.into(), state: ContainerState::Running, session_live: true }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_tracks_name_across_refresh() {
        let mut app = App::new();
        app.set_model(vec![ws("acme"), ws("blog"), ws("infra")]);
        app.move_down(); // acme -> blog
        assert_eq!(app.selected_name(), Some("blog"));
        app.set_model(vec![ws("blog"), ws("infra")]); // shrinks + reorders, blog now first
        assert_eq!(app.selected_name(), Some("blog"));
    }

    #[test]
    fn selection_falls_back_to_neighbor_when_name_vanishes() {
        let mut app = App::new();
        app.set_model(vec![ws("acme"), ws("blog"), ws("infra")]);
        app.move_down(); app.move_down(); // infra
        app.set_model(vec![ws("acme"), ws("blog")]); // infra gone
        // must land on a SURVIVING workspace (the nearest neighbor), not panic, not None
        assert!(app.selected_name().is_some());
        assert_ne!(app.selected_name(), Some("infra"));
    }

    #[test]
    fn empty_model_clears_selection() {
        let mut app = App::new();
        app.set_model(vec![ws("acme")]);
        app.set_model(vec![]);
        assert_eq!(app.selected_name(), None);
    }

    #[test]
    fn move_down_then_up_round_trips() {
        let mut app = App::new();
        app.set_model(vec![ws("acme"), ws("blog"), ws("infra")]);
        assert_eq!(app.selected_name(), Some("acme"));
        app.move_down();
        assert_eq!(app.selected_name(), Some("blog"));
        app.move_up();
        assert_eq!(app.selected_name(), Some("acme"));
    }

    #[test]
    fn toggle_expand_flips_expanded_and_tabs() {
        use work_core::workspace::WindowRow;
        let mut app = App::new();
        app.set_model(vec![ws("acme"), ws("blog")]);
        assert_eq!(app.expanded_name(), None);
        app.toggle_expand(); // expands the selected (acme)
        assert_eq!(app.expanded_name(), Some("acme"));
        app.set_tabs("acme", vec![WindowRow { index: "1".into(), name: "build".into(), panes: "1".into(), active: true, command: "zsh".into() }]);
        assert_eq!(app.expanded_tabs().map(|t| t.len()), Some(1));
        app.toggle_expand(); // collapses
        assert_eq!(app.expanded_name(), None);
    }
}
