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

use anyhow::Result;
use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::config;

/// Daily cache backing the update check. Transient runtime state — kept out of
/// the user-edited `config.toml`.
#[allow(dead_code)]
#[derive(Serialize, Deserialize)]
struct Cache {
    last_check: String, // RFC3339
    latest: String,
}

/// Where the cache file lives: `~/.config/work/update-check.json`.
#[allow(dead_code)]
fn cache_path() -> PathBuf {
    config::config_dir().join("update-check.json")
}

#[allow(dead_code)]
fn read_cache(path: &Path) -> Option<Cache> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Write the cache atomically (tmp + rename) so a process killed mid-write
/// never leaves a corrupt file.
#[allow(dead_code)]
fn write_cache_atomic(path: &Path, latest: &str, now: DateTime<Utc>) -> Result<()> {
    let cache = Cache {
        last_check: now.to_rfc3339(),
        latest: latest.to_string(),
    };
    let raw = serde_json::to_string(&cache)?;
    let tmp = path.with_file_name("update-check.json.tmp");
    fs::write(&tmp, raw)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// True if the cache is missing or older than 24h (or unparseable). PURE-ish:
/// pure given `now`.
#[allow(dead_code)]
fn is_stale(cache: Option<&Cache>, now: DateTime<Utc>) -> bool {
    match cache {
        None => true,
        Some(c) => match DateTime::parse_from_rfc3339(&c.last_check) {
            Ok(t) => now - t.with_timezone(&Utc) > Duration::hours(24),
            Err(_) => true,
        },
    }
}
use std::time::Duration as StdDuration;

/// Fetch the latest version (if any) and write it to the cache. The fetcher is
/// injected so this is unit-testable without network. Swallows all errors.
#[allow(dead_code)] // wired into the worker in a later task.
fn refresh_cache(path: &Path, now: DateTime<Utc>, fetcher: impl Fn() -> Option<String>) {
    if let Some(latest) = fetcher() {
        let _ = write_cache_atomic(path, &latest, now);
    }
}

/// Real fetcher: GET `releases/latest`, parse `tag_name`. 2s timeout, no auth.
/// GitHub requires a `User-Agent`. Returns `None` on any failure.
#[allow(dead_code)] // wired into the worker in a later task.
fn fetch_latest() -> Option<String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(StdDuration::from_secs(2))
        .build();
    let resp = agent
        .get(RELEASES_URL)
        .set("User-Agent", "work-cli")
        .set("Accept", "application/vnd.github+json")
        .call()
        .ok()?;
    let body = resp.into_string().ok()?;
    parse_latest_tag(&body)
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
    use chrono::{TimeZone, Utc};
    use tempfile::tempdir;

    #[test]
    fn cache_round_trip_and_read_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("update-check.json");
        assert!(read_cache(&path).is_none());
        let now = Utc::now();
        write_cache_atomic(&path, "0.5.0", now).unwrap();
        let c = read_cache(&path).unwrap();
        assert_eq!(c.latest, "0.5.0");
    }

    #[test]
    fn is_stale_when_missing_or_old() {
        let now = Utc::now();
        assert!(is_stale(None, now));
        let fresh = Cache {
            last_check: now.to_rfc3339(),
            latest: "0.1.0".to_string(),
        };
        assert!(!is_stale(Some(&fresh), now));
        let old = Cache {
            last_check: Utc
                .timestamp_opt(now.timestamp() - 25 * 3600, 0)
                .unwrap()
                .to_rfc3339(),
            latest: "0.1.0".to_string(),
        };
        assert!(is_stale(Some(&old), now));
    }

    #[test]
    fn cache_path_lives_under_config_dir() {
        let p = cache_path();
        assert!(p.ends_with("update-check.json"));
    }

    #[test]
    fn refresh_cache_writes_when_fetcher_succeeds() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("update-check.json");
        refresh_cache(&path, Utc::now(), || Some("0.9.0".to_string()));
        assert_eq!(read_cache(&path).unwrap().latest, "0.9.0");
    }

    #[test]
    fn refresh_cache_no_write_when_fetcher_returns_none() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("update-check.json");
        refresh_cache(&path, Utc::now(), || None);
        assert!(read_cache(&path).is_none());
    }
}
