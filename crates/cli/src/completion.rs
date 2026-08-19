//! Dynamic shell completion helpers for `work`.
//!
//! `filter_names` is the pure prefix filter; `complete_workspace` and
//! `workspace_subcommand_candidates` are the clap_complete engine hooks.

/// Pure prefix filter over a sorted name list. Workspace names are lowercase, so
/// matching is case-sensitive (consistent with `naming::validate_name`).
pub fn filter_names(names: &[String], prefix: &str) -> Vec<String> {
    names
        .iter()
        .filter(|n| n.starts_with(prefix))
        .cloned()
        .collect()
}
use std::ffi::OsStr;

use clap_complete::engine::CompletionCandidate;

/// Lazy completer for EXISTING workspace names (attached to the args of
/// start/stop/tab/tabs/rm/fwd/browse/config). Reads ONLY the config dir (no engine calls).
pub fn complete_workspace(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(prefix) = current.to_str() else {
        return Vec::new();
    };
    let names = work_core::config::list_workspace_names().unwrap_or_default();
    filter_names(&names, prefix)
        .into_iter()
        .map(CompletionCandidate::new)
        .collect()
}

/// Eager candidates for the external-subcommand slot (bare `work <ws>`).
/// Returns ALL names; the engine filters by the current prefix.
pub fn workspace_subcommand_candidates() -> Vec<CompletionCandidate> {
    work_core::config::list_workspace_names()
        .unwrap_or_default()
        .into_iter()
        .map(CompletionCandidate::new)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::filter_names;

    #[test]
    fn filter_names_matches_prefix_case_sensitive() {
        let names: Vec<String> = ["acme", "acme-2", "blog", "Acme"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            filter_names(&names, "ac"),
            vec!["acme".to_string(), "acme-2".to_string()]
        );
        assert_eq!(filter_names(&names, "Ac"), vec!["Acme".to_string()]);
        assert!(filter_names(&names, "z").is_empty());
    }

    #[test]
    fn filter_names_empty_prefix_returns_all() {
        let names: Vec<String> = ["acme", "blog"].iter().map(|s| s.to_string()).collect();
        assert_eq!(filter_names(&names, "").len(), 2);
    }
}
