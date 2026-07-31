//! Update-available awareness: best-effort, daily-cached, non-blocking.
//!
//! Pure helpers here; IO (cache, HTTP, threads) is added in later tasks.

/// The version of `work`/`work-core` currently running (workspace version).
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[allow(dead_code)]
const RELEASES_URL: &str = "https://api.github.com/repos/coelhucas-dev/work-cli/releases/latest";

/// Strip a leading `v` from a release tag (`v0.2.0` -> `0.2.0`). PURE.
#[allow(dead_code)]
fn strip_tag(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
}

/// True iff `latest` is a strictly newer semver than `current`. Any parse
/// failure -> `false` (never claims an update we can't understand). PURE.
#[allow(dead_code)]
fn is_newer(latest: &str, current: &str) -> bool {
    match (
        semver::Version::parse(latest),
        semver::Version::parse(current),
    ) {
        (Ok(l), Ok(c)) => l > c,
        _ => false,
    }
}

/// Extract the `tag_name` (v-stripped) from a GitHub `releases/latest` body.
/// `None` on any shape we don't recognise. PURE.
#[allow(dead_code)]
fn parse_latest_tag(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let tag = v.get("tag_name")?.as_str()?;
    Some(strip_tag(tag).to_string())
}

use std::path::Path;

/// How `work` appears to have been installed, inferred from its binary path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Homebrew,
    Cargo,
    Other,
}

/// Detect the install channel from the running binary's path. Heuristic only;
/// a binary copied out of its Cellar falls back to `Other`. PURE.
pub fn detect_channel(exe: &Path) -> Channel {
    let s = exe.to_string_lossy();
    if s.contains("Cellar") {
        Channel::Homebrew
    } else if s.contains(".cargo") {
        Channel::Cargo
    } else {
        Channel::Other
    }
}

impl Channel {
    /// One-line, user-facing upgrade hint for `latest`, tailored to the channel.
    pub fn hint(&self, latest: &str) -> String {
        match self {
            Channel::Homebrew => {
                format!("work {latest} available — run \"brew upgrade work\"")
            }
            Channel::Cargo => format!(
                "work {latest} available — run \"cargo install --git https://github.com/coelhucas-dev/work-cli\""
            ),
            Channel::Other => format!(
                "work {latest} available — see https://github.com/coelhucas-dev/work-cli/releases"
            ),
        }
    }
}

/// Whether the update check should run, given the four enablement factors.
/// PURE — callers gather the booleans from the environment/config. Enabled iff
/// interactive (tty) AND not CI AND config on AND no env override.
pub fn is_enabled(is_tty: bool, ci_set: bool, check_cfg: bool, no_update_env_set: bool) -> bool {
    is_tty && !ci_set && check_cfg && !no_update_env_set
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_enabled_truth_table() {
        // enabled: tty, no CI, config on, no env override.
        assert!(is_enabled(true, false, true, false));
        // disabled by each factor in turn.
        assert!(!is_enabled(false, false, true, false)); // not a tty
        assert!(!is_enabled(true, true, true, false)); // CI set
        assert!(!is_enabled(true, false, false, false)); // config opt-out
        assert!(!is_enabled(true, false, true, true)); // env opt-out
    }

    #[test]
    fn strip_tag_removes_leading_v() {
        assert_eq!(strip_tag("v0.2.0"), "0.2.0");
        assert_eq!(strip_tag("0.2.0"), "0.2.0");
        assert_eq!(strip_tag(""), "");
    }

    #[test]
    fn is_newer_compares_semver() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
        // Pre-release is older than its release.
        assert!(!is_newer("0.2.0-rc.1", "0.2.0"));
        // Unparseable -> never claims newer.
        assert!(!is_newer("not-a-version", "0.1.0"));
        assert!(!is_newer("0.2.0", "not-a-version"));
    }

    #[test]
    fn parse_latest_tag_reads_tag_name() {
        let body = r#"{"tag_name":"v0.3.0","name":"0.3.0","html_url":"x"}"#;
        assert_eq!(parse_latest_tag(body), Some("0.3.0".to_string()));
        assert_eq!(parse_latest_tag("{}"), None);
        assert_eq!(parse_latest_tag("not json"), None);
    }

    #[test]
    fn current_is_a_version() {
        // CARGO_PKG_VERSION is always valid semver in this workspace.
        assert!(semver::Version::parse(CURRENT).is_ok());
    }

    use std::path::Path;

    #[test]
    fn detect_channel_from_path() {
        assert_eq!(
            detect_channel(Path::new("/opt/homebrew/Cellar/work/0.2.0/bin/work")),
            Channel::Homebrew
        );
        assert_eq!(
            detect_channel(Path::new("/usr/local/Cellar/work/0.2.0/bin/work")),
            Channel::Homebrew
        );
        assert_eq!(
            detect_channel(Path::new("/Users/jane/.cargo/bin/work")),
            Channel::Cargo
        );
        assert_eq!(
            detect_channel(Path::new("/usr/local/bin/work")),
            Channel::Other
        );
    }

    #[test]
    fn channel_hint_is_channel_aware() {
        assert_eq!(
            Channel::Homebrew.hint("0.2.0"),
            "work 0.2.0 available — run \"brew upgrade work\""
        );
        assert!(Channel::Cargo.hint("0.2.0").contains("cargo install --git"));
        assert!(Channel::Other.hint("0.2.0").contains("releases"));
    }
}
