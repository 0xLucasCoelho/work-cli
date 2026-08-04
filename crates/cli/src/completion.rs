//! Dynamic shell completion helpers for `work`. (Pure helpers for now; the
//! clap_complete completer functions are added in a later task.)

/// Pure prefix filter over a sorted name list. Workspace names are lowercase, so
/// matching is case-sensitive (consistent with `naming::validate_name`).
pub fn filter_names(names: &[String], prefix: &str) -> Vec<String> {
    names.iter().filter(|n| n.starts_with(prefix)).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::filter_names;

    #[test]
    fn filter_names_matches_prefix_case_sensitive() {
        let names: Vec<String> = ["acme", "acme-2", "blog", "Acme"].iter().map(|s| s.to_string()).collect();
        assert_eq!(filter_names(&names, "ac"), vec!["acme".to_string(), "acme-2".to_string()]);
        assert_eq!(filter_names(&names, "Ac"), vec!["Acme".to_string()]);
        assert!(filter_names(&names, "z").is_empty());
    }

    #[test]
    fn filter_names_empty_prefix_returns_all() {
        let names: Vec<String> = ["acme", "blog"].iter().map(|s| s.to_string()).collect();
        assert_eq!(filter_names(&names, "").len(), 2);
    }
}
