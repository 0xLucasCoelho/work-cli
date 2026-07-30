//! In-container identity banner. PURE: composes the fastfetch-style block from
//! inputs gathered by the attach path. No IO, no dependency on the engine.

/// Compose the workspace identity banner. A left-bordered block (no fragile
/// right-edge alignment) so variable-length values render cleanly everywhere.
pub fn compose(name: &str, image: &str, system: &str, hostname: &str, git: &str) -> String {
    let row = |k: &str, v: &str| -> String { format!("  │   {k:<10} {v}") };
    [
        "  ╭─ work ─────────────────────────────".to_string(),
        String::new(),
        row("workspace", name),
        row("image", image),
        row("system", system),
        row("hostname", hostname),
        row("network", "isolated · single-context"),
        row("home", "/home/dev"),
        row("git", git),
        String::new(),
        "  │   isolated container — bring your own tools".to_string(),
        "  ╰──────────────────────────────────────".to_string(),
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_lists_every_field() {
        let s = compose(
            "acme",
            "work-base:latest",
            "Debian GNU/Linux 12",
            "abc1234",
            "main",
        );
        for needle in [
            "acme",
            "work-base:latest",
            "Debian GNU/Linux 12",
            "abc1234",
            "isolated · single-context",
            "/home/dev",
            "main",
            "bring your own tools",
        ] {
            assert!(s.contains(needle), "banner missing {needle:?}:\n{s}");
        }
        // Each label sits on the same line as its value (paired, in order).
        assert!(s
            .lines()
            .any(|l| l.contains("workspace") && l.contains("acme")));
        assert!(s
            .lines()
            .any(|l| l.contains("hostname") && l.contains("abc1234")));
        assert!(s.lines().any(|l| l.contains("git") && l.contains("main")));
    }

    #[test]
    fn compose_handles_missing_fields() {
        let s = compose("demo", "work-lucas:latest", "—", "—", "—");
        // A missing git branch renders the em-dash on the git row.
        assert!(s.lines().any(|l| l.contains("git") && l.ends_with('—')));
    }
}
