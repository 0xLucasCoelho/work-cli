//! Pure dashboard state. Selection is keyed by workspace NAME (not index) so a
//! concurrent work rm/new (changes length AND order) can't shift the cursor onto
//! the wrong workspace. No IO, no rendering.

#[cfg(test)]
use work_core::engine::ContainerState;
use std::collections::HashMap;
use work_core::workspace::{WindowRow, WorkspaceStatus};

#[derive(Clone, Copy)]
pub(crate) enum ConfirmAction { Stop, Remove }

/// A pending destructive-op confirmation. While `Some`, the event loop only
/// answers y / n / Esc / q / Ctrl-C.
pub(crate) struct Confirm {
    pub ws: String,
    pub action: ConfirmAction,
    pub blurb: String,
}
/// A deferred action to run after the TUI tears down (so it gets a normal TTY).
/// Attach/NewTab both start an interactive shell; Create builds a workspace and
/// then attaches to it.
pub(crate) enum PendingAction { Attach(String), Create(String), NewTab(String) }

/// Inline text-input sub-mode (e.g. entering a new-workspace name). While `Some`
/// the event loop intercepts text keys instead of normal navigation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode { New, Filter }
pub(crate) struct App {
    model: Vec<WorkspaceStatus>,
    selected: Option<String>, // workspace NAME under the cursor
    filter: Option<String>, // active name substring filter, if any (None = show all)
    quit: bool,
    status: Option<String>,
    confirm: Option<Confirm>, // pending destructive-op confirm (s/x/d gate)
    pending: Option<PendingAction>, // deferred action (attach/create/new-tab) after TUI teardown
    mode: Option<Mode>, // active inline text-input sub-mode, if any
    buf: String, // text buffer for the active input mode
    expanded: Option<String>, // workspace NAME that's expanded to show its tabs
    tabs: HashMap<String, Vec<WindowRow>>, // cached tab rows per workspace NAME
}

impl App {
    pub(crate) fn new() -> Self {
        Self {
            model: Vec::new(),
            selected: None,
            filter: None,
            quit: false,
            status: None,
            pending: None,
            mode: None,
            buf: String::new(),
            confirm: None,
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

    /// Set (or clear) the substring filter. An empty string clears it (no filter).
    pub(crate) fn set_filter(&mut self, f: Option<String>) {
        self.filter = f.filter(|s| !s.is_empty());
    }

    pub(crate) fn filter_active(&self) -> bool { self.filter.is_some() }

    /// Names currently shown (filtered by substring, else all), in model order.
    pub(crate) fn visible_names(&self) -> Vec<String> {
        match &self.filter {
            None => self.model.iter().map(|w| w.name.clone()).collect(),
            Some(f) => self
                .model
                .iter()
                .filter(|w| w.name.contains(f))
                .map(|w| w.name.clone())
                .collect(),
        }
    }

    pub(crate) fn move_up(&mut self) {
        let names = self.visible_names();
        if names.is_empty() { return; }
        let next = match self
            .selected
            .as_deref()
            .and_then(|n| names.iter().position(|x| x == n))
        {
            None | Some(0) => 0,
            Some(i) => i - 1,
        };
        self.selected = Some(names[next].clone());
    }

    pub(crate) fn move_down(&mut self) {
        let names = self.visible_names();
        if names.is_empty() { return; }
        let last = names.len() - 1;
        let next = match self
            .selected
            .as_deref()
            .and_then(|n| names.iter().position(|x| x == n))
        {
            None => 0,
            Some(i) => (i + 1).min(last),
        };
        self.selected = Some(names[next].clone());
    }

    pub(crate) fn quit(&mut self) { self.quit = true; }
    pub(crate) fn should_quit(&self) -> bool { self.quit }

    pub(crate) fn set_status(&mut self, msg: impl Into<String>) { self.status = Some(msg.into()); }
    pub(crate) fn status_message(&self) -> Option<&str> { self.status.as_deref() }
    pub(crate) fn confirm(&self) -> Option<&Confirm> { self.confirm.as_ref() }
    pub(crate) fn request_confirm(&mut self, c: Confirm) { self.confirm = Some(c); }
    pub(crate) fn cancel_confirm(&mut self) { self.confirm = None; }
    pub(crate) fn take_confirm(&mut self) -> Option<Confirm> { self.confirm.take() }

    pub(crate) fn request_attach(&mut self, name: String) { self.pending = Some(PendingAction::Attach(name)); }
    pub(crate) fn request_create(&mut self, name: String) { self.pending = Some(PendingAction::Create(name)); }
    pub(crate) fn request_tab(&mut self, name: String) { self.pending = Some(PendingAction::NewTab(name)); }
    pub(crate) fn pending(&mut self) -> &mut Option<PendingAction> { &mut self.pending }

    pub(crate) fn mode(&self) -> Option<Mode> { self.mode }
    pub(crate) fn enter_mode(&mut self, m: Mode) { self.mode = Some(m); self.buf.clear(); }
    pub(crate) fn cancel_mode(&mut self) { self.mode = None; self.buf.clear(); }
    pub(crate) fn buf_push(&mut self, c: char) { self.buf.push(c); }
    pub(crate) fn buf_pop(&mut self) { self.buf.pop(); }
    pub(crate) fn buf_str(&self) -> &str { &self.buf }
    pub(crate) fn buf_take(&mut self) -> String { std::mem::take(&mut self.buf) }

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

    #[test]
    fn visible_names_all_when_no_filter() {
        let mut app = App::new();
        app.set_model(vec![ws("acme"), ws("blog"), ws("infra")]);
        assert_eq!(app.visible_names(), vec!["acme", "blog", "infra"]);
        assert!(!app.filter_active());
    }

    #[test]
    fn filter_narrows_visible_names_by_substring() {
        let mut app = App::new();
        app.set_model(vec![ws("alpha"), ws("beta"), ws("gamma"), ws("delta")]);
        app.set_filter(Some("e".into()));
        // case-sensitive substring: only "beta"/"delta" contain 'e'
        assert_eq!(app.visible_names(), vec!["beta", "delta"]);
        assert!(app.filter_active());
    }

    #[test]
    fn set_filter_empty_clears_filter() {
        let mut app = App::new();
        app.set_model(vec![ws("acme"), ws("blog")]);
        app.set_filter(Some("acme".into()));
        assert!(app.filter_active());
        app.set_filter(Some(String::new())); // empty -> no filter
        assert!(!app.filter_active());
        assert_eq!(app.visible_names(), vec!["acme", "blog"]);
        app.set_filter(Some("a".into()));
        app.set_filter(None); // None -> no filter
        assert!(!app.filter_active());
    }

    #[test]
    fn move_navigates_visible_set_and_clamps_at_edges() {
        let mut app = App::new();
        app.set_model(vec![ws("alpha"), ws("beta"), ws("gamma"), ws("delta")]);
        app.set_filter(Some("e".into())); // visible: beta, delta
        // selection starts on the first model item (alpha), which is filtered out.
        app.move_down(); // snaps to the first visible item (beta)
        assert_eq!(app.selected_name(), Some("beta"));
        app.move_down(); // beta -> delta
        assert_eq!(app.selected_name(), Some("delta"));
        app.move_down(); // bottom edge: stays on delta
        assert_eq!(app.selected_name(), Some("delta"));
        app.move_up(); // delta -> beta
        assert_eq!(app.selected_name(), Some("beta"));
        app.move_up(); // top edge: stays on beta
        assert_eq!(app.selected_name(), Some("beta"));
    }

    #[test]
    fn move_snaps_to_visible_when_selection_filtered_out() {
        let mut app = App::new();
        app.set_model(vec![ws("acme"), ws("blog"), ws("infra")]);
        app.move_down(); app.move_down(); // infra
        app.set_filter(Some("blog".into())); // only blog visible
        app.move_up(); // infra is filtered out -> snap to the only visible item
        assert_eq!(app.selected_name(), Some("blog"));
    }
}
