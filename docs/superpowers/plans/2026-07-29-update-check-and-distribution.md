# Update-Awareness & macOS Distribution — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a non-intrusive update-available check (background, daily-cached, channel-aware) and a real macOS distribution story (Homebrew tap + cargo-binstall + `curl | sh`), co-releasing as the first tagged release.

**Architecture:** All update logic lives in a new `work-core` module `update.rs`. `main()` spawns a best-effort worker thread (only when the daily cache is stale), prints a one-line channel-aware hint from the cached latest version, and bounded-joins the worker (≤1s, via a channel + `Drop`) at exit so the cache refreshes for short-lived commands. The check is gated off in CI/non-TTY and opt-out via config/env. Distribution adds cargo-binstall metadata, an `install.sh`, a Homebrew formula template rendered by CI, and a README rewrite — all reusing binaries the existing `release.yml` already builds.

**Tech Stack:** Rust 2021 (stable), clap 4, anyhow, serde/serde_json, toml, chrono (already workspace deps); new deps `ureq` (sync HTTP) + `semver`. GitHub Releases API + Actions. Homebrew formula (Ruby). POSIX `sh` installer.

**Spec:** `docs/superpowers/specs/2026-07-29-update-check-and-distribution-design.md`

## Global Constraints

- **Isolation invariants are unchanged and must remain provably intact:** one container, one named volume mounted only at `/home/dev`, one bridge network, non-root `dev`, no host ports, image match. `work doctor` must still pass. The update check is host-side only; it never runs inside a container and touches no mount/network/user/port.
- **No secrets, no host bind-mounts.** The update check is an unauthenticated public-API read plus a local non-secret JSON cache. Distribution artifacts are external.
- **Rust 2021, stable toolchain** (`rust-toolchain.toml`). Quality gates every commit: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- **Non-blocking contract:** the user's command never waits on the network. Only process exit may pause ≤1s once/day (stale-cache refresh). Off-TTY / `CI` set / `[update] check = false` / `WORK_NO_UPDATE_CHECK` set → no network, no output.
- **Locked decisions:** Homebrew formula lives in a **separate** `coelhucas-dev/homebrew-tap` repo; the ≤1s/day exit-join tradeoff is accepted over fully-detached.
- **Conventional commits**, e.g. `feat(core): …`, `chore(dist): …`.

## Prerequisites (external, not file-edits — do before/around Part 2)

- **P-1.** Create repository `coelhucas-dev/homebrew-tap` (empty, default branch `main`). Needed for Task 9's CI push target.
- **P-2.** Create a GitHub PAT with `repo` (or fine-grained `Contents: write`) scope for `coelhucas-dev/homebrew-tap`, and add it as a repository secret named `HOMEBREW_TAP_TOKEN` in `coelhucas-dev/work-cli`. Needed for Task 9's CI push step.
- **P-3.** A first `v*` tag with published release assets must exist before distribution can be smoke-tested end-to-end (the formula/binstall/install.sh all consume release tarballs). The update-check code is usable from `cargo install` builds immediately.

## File Structure

**Part 1 — update-available check (code):**
- **Create** `crates/core/src/update.rs` — PURE helpers (version parse/compare, tag parse), cache (read/atomic-write/stale), channel detection + hint, enablement gate, refresh (injectable fetcher), HTTP fetcher, orchestration (`run_check` + `UpdateGuard`). Inline unit tests.
- **Modify** `crates/core/src/lib.rs` — declare `pub mod update;`.
- **Modify** `crates/core/src/config.rs` — `GlobalConfig.update: UpdatePrefs` + `UpdatePrefs` (default `check = true`).
- **Modify** `crates/core/Cargo.toml` — add `ureq`, `semver`.
- **Modify** `crates/cli/src/main.rs` — call `update::run_check()` early; hold the guard to end of `main()`.

**Part 2 — distribution (artifacts):**
- **Modify** `crates/cli/Cargo.toml` — `[package.metadata.binstall]`.
- **Create** `install.sh` — repo-root POSIX installer.
- **Create** `distribution/homebrew/work.rb.template` — formula template rendered by CI.
- **Modify** `.github/workflows/release.yml` — `update-tap` job (render + push formula).
- **Modify** `README.md` — rewrite "Install"; add "Upgrade".
- **Modify** `CHANGELOG.md` — entry under the first tagged release.

---

### Task 1: Pure version helpers + module + deps

Version comparison and release-JSON parsing — the foundation. No IO, fully testable. Folds in the new module + dependencies.

**Files:**
- Modify: `crates/core/Cargo.toml` (`[dependencies]`)
- Modify: `crates/core/src/lib.rs` (after the existing `pub mod …;` declarations)
- Create: `crates/core/src/update.rs`
- Test: inline in `crates/core/src/update.rs`

**Interfaces:**
- Produces (module-private for now): `fn strip_tag(tag: &str) -> &str`, `fn is_newer(latest: &str, current: &str) -> bool`, `fn parse_latest_tag(json: &str) -> Option<String>`, `pub const CURRENT: &str`, `const RELEASES_URL: &str`. Consumed by Tasks 5 and 6.

- [ ] **Step 1: Add dependencies**

In `crates/core/Cargo.toml`, append to the `[dependencies]` table (after the `chrono` line):
```toml
ureq = { version = "2", default-features = false, features = ["tls"] }
semver = "1"
```
(`default-features = false` + `features = ["tls"]` keeps ureq on `rustls` TLS with no `native-tls`/OpenSSL system dep. `serde_json` and `chrono` are already present.)

- [ ] **Step 2: Declare the module**

In `crates/core/src/lib.rs`, add after the last `pub mod` line (`pub mod workspace;`):
```rust
pub mod update;
```

- [ ] **Step 3: Write the failing tests**

Create `crates/core/src/update.rs`:
```rust
//! Update-available awareness: best-effort, daily-cached, non-blocking.
//!
//! Pure helpers here; IO (cache, HTTP, threads) is added in later tasks.

/// The version of `work`/`work-core` currently running (workspace version).
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

const RELEASES_URL: &str =
    "https://api.github.com/repos/coelhucas-dev/work-cli/releases/latest";

/// Strip a leading `v` from a release tag (`v0.2.0` -> `0.2.0`). PURE.
fn strip_tag(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
}

/// True iff `latest` is a strictly newer semver than `current`. Any parse
/// failure -> `false` (never claims an update we can't understand). PURE.
fn is_newer(latest: &str, current: &str) -> bool {
    match (semver::Version::parse(latest), semver::Version::parse(current)) {
        (Ok(l), Ok(c)) => l > c,
        _ => false,
    }
}

/// Extract the `tag_name` (v-stripped) from a GitHub `releases/latest` body.
/// `None` on any shape we don't recognise. PURE.
fn parse_latest_tag(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let tag = v.get("tag_name")?.as_str()?;
    Some(strip_tag(tag).to_string())
}


#[cfg(test)]
mod tests {
    use super::*;

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
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p work-core update::`
Expected: PASS (4 tests). (These are defined alongside their impls; this task ships impl + tests together because the helpers are trivial and the module/deps are the real scaffolding.)

- [ ] **Step 5: Commit**

```bash
git add crates/core/Cargo.toml crates/core/src/lib.rs crates/core/src/update.rs
git commit -m "feat(core): add update module with pure version helpers"
```

---

### Task 2: Channel detection + upgrade hint

Detect how `work` was installed from its binary path and produce the tailored upgrade line.

**Files:**
- Modify: `crates/core/src/update.rs` (append before the `#[cfg(test)]` block)
- Test: inline in `crates/core/src/update.rs`

**Interfaces:**
- Produces: `pub enum Channel { Homebrew, Cargo, Other }`, `pub fn detect_channel(exe: &std::path::Path) -> Channel`, `impl Channel { pub fn hint(&self, latest: &str) -> String }`. Consumed by Task 6.

- [ ] **Step 1: Write the failing test**

Add these tests to the `mod tests` block in `crates/core/src/update.rs`:
```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p work-core detect_channel_from_path`
Expected: FAIL — `cannot find function detect_channel`.

- [ ] **Step 3: Add the types + function**

Insert into `crates/core/src/update.rs` (before the `#[cfg(test)]` block):
```rust
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p work-core detect_channel_from_path channel_hint_is_channel_aware`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/update.rs
git commit -m "feat(core): add update channel detection + upgrade hint"
```

---

### Task 3: Enablement gate + config field

Decide whether the check runs at all, driven by TTY/CI/config/env.

**Files:**
- Modify: `crates/core/src/config.rs` (`GlobalConfig`, `load_global` default)
- Modify: `crates/core/src/update.rs` (append `is_enabled`)
- Test: inline in both files

**Interfaces:**
- Produces (config): `#[derive(Debug, Clone, Serialize, Deserialize, Default)] pub struct UpdatePrefs { #[serde(default = "default_check")] pub check: bool }` + `GlobalConfig { ..., #[serde(default)] pub update: UpdatePrefs }`.
- Produces (update): `pub fn is_enabled(is_tty: bool, ci_set: bool, check_cfg: bool, no_update_env_set: bool) -> bool`. Consumed by Task 6.

- [ ] **Step 1: Write the failing config test**

Add to the bottom of `crates/core/src/config.rs` (new test module; the file currently has none):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_prefs_default_check_is_true() {
        assert!(UpdatePrefs::default().check);
    }

    #[test]
    fn load_global_defaults_include_update_enabled() {
        // config.toml absent -> defaults, with the update check on.
        let dir = std::env::temp_dir();
        // We assert the default struct directly (load_global reads a real path).
        let g = GlobalConfig {
            default_image: Some(DEFAULT_IMAGE.to_string()),
            import_shell_config: None,
            import_tmux_config: None,
            import_starship_config: None,
            show_banner: true,
            update: UpdatePrefs::default(),
        };
        assert!(g.update.check);
    }

    #[test]
    fn update_prefs_parses_check_false() {
        let g: GlobalConfig = toml::from_str(
            "[update]\ncheck = false\n",
        )
        .unwrap();
        assert!(!g.update.check);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p work-core update_prefs_default_check_is_true`
Expected: FAIL — `cannot find type UpdatePrefs`.

- [ ] **Step 3: Add `UpdatePrefs` + wire into `GlobalConfig`**

In `crates/core/src/config.rs`, add the `update` field to `GlobalConfig` (after `show_banner`):
```rust
    /// Update-available check preferences (default: enabled).
    #[serde(default)]
    pub update: UpdatePrefs,
```

Add the `UpdatePrefs` struct after the `GlobalConfig` definition (after its `impl` block, before `WorkspaceConfig`):
```rust
/// Preferences for the update-available check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePrefs {
    /// Run the (best-effort, daily) update check. Default `true`.
    #[serde(default = "default_check")]
    pub check: bool,
}

fn default_check() -> bool {
    true
}

impl Default for UpdatePrefs {
    fn default() -> Self {
        Self { check: true }
    }
}
```

Update the default construction inside `load_global` (the `if !path.exists()` branch) to add the field:
```rust
        return Ok(GlobalConfig {
            default_image: Some(DEFAULT_IMAGE.to_string()),
            import_shell_config: None,
            import_tmux_config: None,
            import_starship_config: None,
            show_banner: true,
            update: UpdatePrefs::default(),
        });
```

- [ ] **Step 4: Run config tests to verify they pass**

Run: `cargo test -p work-core config::`
Expected: PASS (3 tests).

- [ ] **Step 5: Write the failing gate test**

Add to `crates/core/src/update.rs` `mod tests`:
```rust
    #[test]
    fn is_enabled_truth_table() {
        // enabled: tty, no CI, config on, no env override.
        assert!(is_enabled(true, false, true, false));
        // disabled by each factor in turn.
        assert!(!is_enabled(false, false, true, false)); // not a tty
        assert!(!is_enabled(true, true, true, false));   // CI set
        assert!(!is_enabled(true, false, false, false)); // config opt-out
        assert!(!is_enabled(true, false, true, true));   // env opt-out
    }
```

- [ ] **Step 6: Run test to verify it fails**

Run: `cargo test -p work-core is_enabled_truth_table`
Expected: FAIL — `cannot find function is_enabled`.

- [ ] **Step 7: Add `is_enabled`**

Insert into `crates/core/src/update.rs` (before `#[cfg(test)]`):
```rust
/// Whether the update check should run, given the four enablement factors.
/// PURE — callers gather the booleans from the environment/config. Enabled iff
/// interactive (tty) AND not CI AND config on AND no env override.
pub fn is_enabled(is_tty: bool, ci_set: bool, check_cfg: bool, no_update_env_set: bool) -> bool {
    is_tty && !ci_set && check_cfg && !no_update_env_set
}
```

- [ ] **Step 8: Run test to verify it passes**

Run: `cargo test -p work-core is_enabled_truth_table`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/core/src/config.rs crates/core/src/update.rs
git commit -m "feat(core): add UpdatePrefs config + update enablement gate"
```

---

### Task 4: Cache (read / atomic write / staleness)

The daily-cache file that backs the non-blocking design.

**Files:**
- Modify: `crates/core/src/update.rs` (append cache logic)
- Test: inline in `crates/core/src/update.rs`

**Interfaces:**
- Produces (module-private): `struct Cache { last_check: String, latest: String }`, `fn cache_path() -> std::path::PathBuf`, `fn read_cache(path: &Path) -> Option<Cache>`, `fn write_cache_atomic(path: &Path, latest: &str, now: chrono::DateTime<chrono::Utc>) -> Result<()>`, `fn is_stale(cache: Option<&Cache>, now: chrono::DateTime<chrono::Utc>) -> bool`. Consumed by Tasks 5 and 6.

- [ ] **Step 1: Write the failing tests**

Add to `crates/core/src/update.rs` `mod tests`:
```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p work-core cache_round_trip_and_read_missing`
Expected: FAIL — `cannot find function read_cache`.

- [ ] **Step 3: Add the cache logic**

Insert into `crates/core/src/update.rs` (before `#[cfg(test)]`). This task adds `use anyhow::Result;` (first use of `Result` in the module).
```rust
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::config;

/// Daily cache backing the update check. Transient runtime state — kept out of
/// the user-edited `config.toml`.
#[derive(Serialize, Deserialize)]
struct Cache {
    last_check: String, // RFC3339
    latest: String,
}

/// Where the cache file lives: `~/.config/work/update-check.json`.
fn cache_path() -> PathBuf {
    config::config_dir().join("update-check.json")
}

fn read_cache(path: &Path) -> Option<Cache> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Write the cache atomically (tmp + rename) so a process killed mid-write
/// never leaves a corrupt file.
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
fn is_stale(cache: Option<&Cache>, now: DateTime<Utc>) -> bool {
    match cache {
        None => true,
        Some(c) => match DateTime::parse_from_rfc3339(&c.last_check) {
            Ok(t) => now - t.with_timezone(&Utc) > Duration::hours(24),
            Err(_) => true,
        },
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p work-core cache_round_trip_and_read_missing is_stale_when_missing_or_old cache_path_lives_under_config_dir`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/update.rs
git commit -m "feat(core): add update-check daily cache (atomic, staleness)"
```

---

### Task 5: Refresh (injectable fetcher) + real HTTP fetcher

The worker's core: fetch latest, write cache. The fetch is behind an injected closure so it's unit-testable without network.

**Files:**
- Modify: `crates/core/src/update.rs` (append refresh + fetch)
- Test: inline in `crates/core/src/update.rs`

**Interfaces:**
- Produces (module-private): `fn refresh_cache(path: &Path, now: DateTime<Utc>, fetcher: impl Fn() -> Option<String>)`, `fn fetch_latest() -> Option<String>`. Consumed by Task 6.

- [ ] **Step 1: Write the failing test**

Add to `crates/core/src/update.rs` `mod tests`:
```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p work-core refresh_cache_writes_when_fetcher_succeeds`
Expected: FAIL — `cannot find function refresh_cache`.

- [ ] **Step 3: Add `refresh_cache` + `fetch_latest`**

Insert into `crates/core/src/update.rs` (before `#[cfg(test)]`):
```rust
use std::time::Duration as StdDuration;

/// Fetch the latest version (if any) and write it to the cache. The fetcher is
/// injected so this is unit-testable without network. Swallows all errors.
fn refresh_cache(path: &Path, now: DateTime<Utc>, fetcher: impl Fn() -> Option<String>) {
    if let Some(latest) = fetcher() {
        let _ = write_cache_atomic(path, &latest, now);
    }
}

/// Real fetcher: GET `releases/latest`, parse `tag_name`. 2s timeout, no auth.
/// GitHub requires a `User-Agent`. Returns `None` on any failure.
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p work-core refresh_cache_`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/update.rs
git commit -m "feat(core): add update refresh (injectable fetcher) + HTTP fetcher"
```

---

### Task 6: Orchestration (`run_check` + `UpdateGuard`) and wire into `main`

Tie it together: gate, read cache, print hint, spawn worker, bounded-join at exit. Then call it from the CLI.

**Files:**
- Modify: `crates/core/src/update.rs` (append `run_check` + `UpdateGuard`)
- Modify: `crates/cli/src/main.rs` (`main()`, after `normalize_help_arg` ~line 191)
- Test: inline in `crates/core/src/update.rs` (orchestration helpers); manual smoke for `run_check`

**Interfaces:**
- Produces (public): `pub fn run_check() -> UpdateGuard`, `pub struct UpdateGuard` (impl `Drop` does the bounded join). Consumed by `crates/cli/src/main.rs`.

- [ ] **Step 1: Add `run_check` + `UpdateGuard`**

Append to `crates/core/src/update.rs` (before `#[cfg(test)]`). `std::io::IsTerminal` is already used elsewhere; import it here:
```rust
use std::io::IsTerminal;
use std::sync::mpsc;
use std::thread;

/// Guard returned by `run_check`. Held for the life of the CLI; on drop it
/// bounded-joins the worker (≤1s) so a stale cache actually refreshes for
/// short-lived commands. Dropping never blocks longer than the budget.
pub struct UpdateGuard {
    done: Option<mpsc::Receiver<()>>,
}

impl UpdateGuard {
    /// An inert guard that does nothing on drop (check disabled / cache fresh).
    fn none() -> Self {
        Self { done: None }
    }
}

impl Drop for UpdateGuard {
    fn drop(&mut self) {
        if let Some(rx) = self.done.take() {
            // Bounded: don't let a slow network delay exit beyond 1s. If the
            // worker is still running, it is simply killed at process exit;
            // the cache write is atomic, so nothing is corrupted.
            let _ = rx.recv_timeout(StdDuration::from_secs(1));
        }
    }
}

/// Gather the four enablement factors from the live environment/config.
fn enabled_in_env() -> bool {
    let is_tty = std::io::stderr().is_terminal();
    let ci_set = std::env::var_os("CI").is_some();
    let no_update_env = std::env::var_os("WORK_NO_UPDATE_CHECK").is_some();
    let check_cfg = config::load_global()
        .map(|g| g.update.check)
        .unwrap_or(true);
    is_enabled(is_tty, ci_set, check_cfg, no_update_env)
}

/// Entry point for the CLI. Best-effort, non-blocking:
///   1. If disabled, return an inert guard.
///   2. Print a one-line, channel-aware hint if the cached `latest` is newer.
///   3. If the cache is stale, spawn a worker that refreshes it; the returned
///      guard bounded-joins it at exit.
/// Never panics, never errors into the caller.
pub fn run_check() -> UpdateGuard {
    if !enabled_in_env() {
        return UpdateGuard::none();
    }

    let path = cache_path();
    let now = Utc::now();
    let cached = read_cache(&path);

    if let Some(c) = cached.as_ref() {
        if is_newer(&c.latest, CURRENT) {
            let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("work"));
            eprintln!("{}", detect_channel(&exe).hint(&c.latest));
        }
    }

    if is_stale(cached.as_ref(), now) {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            refresh_cache(&path, now, fetch_latest);
            let _ = tx.send(());
        });
        UpdateGuard { done: Some(rx) }
    } else {
        UpdateGuard::none()
    }
}

```

- [ ] **Step 2: Build + clippy**

Run: `cargo clippy -p work-core --all-targets -- -D warnings`
Expected: PASS (no warnings).

- [ ] **Step 3: Add a focused test for `enabled_in_env` wiring via the pure gate**

(We already cover `is_enabled`; `run_check` itself is exercised by the manual smoke. No new unit test needed — do not fabricate a thread-based unit test. Instead, assert the guard compiles and is inert when disabled.)

Add to `crates/core/src/update.rs` `mod tests`:
```rust
    #[test]
    fn inert_guard_drops_cleanly() {
        let g = UpdateGuard::none();
        drop(g); // must not block or panic
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p work-core inert_guard_drops_cleanly`
Expected: PASS.

- [ ] **Step 5: Wire into the CLI**

In `crates/cli/src/main.rs`, immediately after the `let raw = normalize_help_arg(raw);` line (~line 191) and before the "Bare workspace name dispatch" block, add:
```rust
    // Best-effort update-available awareness (non-blocking, daily-cached).
    // Held to end of main(); drops at exit -> bounded-joined (≤1s) so a stale
    // cache refreshes for short-lived commands. Off in CI/non-TTY.
    let _update_guard = work_core::update::run_check();
```

- [ ] **Step 6: Build the whole workspace + run all tests**

Run: `cargo build --workspace && cargo test --workspace`
Expected: PASS (existing tests unaffected).

- [ ] **Step 7: Manual smoke (Part 1 done)**

With no config file present:
```bash
# (a) Disabled off-TTY: no network, no output.
WORK_NO_UPDATE_CHECK=1 cargo run -- ls
CI=1 cargo run -- ls
cargo run -- ls | cat        # piped -> no TTY -> silent

# (b) Forced-newer cache: seed a cache claiming a newer version, then run.
mkdir -p ~/.config/work
echo '{"last_check":"2020-01-01T00:00:00Z","latest":"99.0.0"}' > ~/.config/work/update-check.json
cargo run -- ls
# Expected: a "work 99.0.0 available — ..." line on stderr, BEFORE/AROUND the ls output.

# (c) Remove the seed; confirm no output (cache stale -> worker refreshes
#     against the real latest, which is <= current -> no hint on next run).
rm ~/.config/work/update-check.json
cargo run -- ls
```
Record the observed output. Clean up any seed cache afterward.

- [ ] **Step 8: Commit**

```bash
git add crates/core/src/update.rs crates/cli/src/main.rs
git commit -m "feat(cli): wire non-blocking update-available check into main"
```

---

### Task 7: cargo-binstall metadata

Let `cargo binstall --git … work` fetch a pre-built binary from the release assets.

**Files:**
- Modify: `crates/cli/Cargo.toml` (append metadata)

- [ ] **Step 1: Add binstall metadata**

Append to `crates/cli/Cargo.toml`. First add `repository.workspace = true` to the `[package]` table — the `{ repo }` template below resolves from the package's `repository` field, which the workspace already defines as the GitHub URL. Then append the binstall metadata block:
```toml
[package.metadata.binstall]
# Reuses the binaries published by .github/workflows/release.yml.
# bin-dir matches the release archive layout: the `work` binary is at the archive root.
pkg-url = "{ repo }/releases/download/v{ version }/work-{ target }.tar.gz"
bin-dir = "{ bin }{ binary-ext }"
```

- [ ] **Step 2: Verify it parses**

Run: `cargo metadata --no-deps --format-version 1 | jq '.packages[] | select(.name=="work") | .metadata.binstall'`
Expected: prints the `pkg-url` / `bin-dir` object. (If `jq` is absent, `cargo metadata --no-deps --format-version 1 | grep -o 'binstall[^}]*}'` to eyeball it.)

- [ ] **Step 3: Commit**

```bash
git add crates/cli/Cargo.toml
git commit -m "chore(dist): add cargo-binstall metadata"
```

---

### Task 8: `install.sh` installer (universal fallback)

A POSIX `sh` one-liner that downloads the right release archive.

**Files:**
- Create: `install.sh` (repo root)

- [ ] **Step 1: Write the installer**

Create `install.sh`:
```sh
#!/bin/sh
# Universal installer for `work` — downloads a pre-built binary from the latest
# GitHub release into a writable bin dir. macOS + Linux, arm64 + x86_64.
set -e

REPO="coelhucas-dev/work-cli"
BIN_NAME="work"

command -v curl >/dev/null 2>&1 || { echo "error: curl is required" >&2; exit 1; }
command -v tar  >/dev/null 2>&1 || { echo "error: tar is required"  >&2; exit 1; }

# Resolve the latest tag (strip surrounding quotes / leading 'v' kept for URL).
latest_tag=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)
[ -n "$latest_tag" ] || { echo "error: could not determine latest release" >&2; exit 1; }

# Map (os, arch) -> release archive target name.
os=$(uname -s | tr '[:upper:]' '[:lower:]')
arch=$(uname -m)
case "$os-$arch" in
    darwin-arm64)   target="work-aarch64-apple-darwin" ;;
    darwin-x86_64)  target="work-x86_64-apple-darwin" ;;
    linux-x86_64)   target="work-x86_64-unknown-linux-gnu" ;;
    *) echo "error: no prebuilt binary for $os-$arch" >&2; exit 1 ;;
esac

# Pick a writable install dir.
install_dir="/usr/local/bin"
if [ ! -w "$install_dir" ]; then
    install_dir="${CARGO_HOME:-$HOME/.cargo}/bin"
fi
if [ ! -d "$install_dir" ]; then
    install_dir="$HOME/.local/bin"
fi
mkdir -p "$install_dir"

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT
url="https://github.com/${REPO}/releases/download/${latest_tag}/${target}.tar.gz"
echo "Downloading $url"
curl -fsSL "$url" | tar -xz -C "$tmpdir"

cp "$tmpdir/$BIN_NAME" "$install_dir/$BIN_NAME"
chmod +x "$install_dir/$BIN_NAME"
echo "Installed $BIN_NAME to $install_dir"
echo "Run: $install_dir/$BIN_NAME --version"
```

- [ ] **Step 2: Make it executable + syntax-check**

Run: `chmod +x install.sh && sh -n install.sh`
Expected: no output (syntax OK).

- [ ] **Step 3: Commit**

```bash
git add install.sh
git commit -m "chore(dist): add curl|sh installer"
```

---

### Task 9: Homebrew formula template + CI render-and-push job

Keep a formula template in-repo; on release, CI computes per-target SHAs, renders it, and pushes to `coelhucas-dev/homebrew-tap`.

**Files:**
- Create: `distribution/homebrew/work.rb.template`
- Modify: `.github/workflows/release.yml` (add `update-tap` job)

- [ ] **Step 1: Write the formula template**

Create `distribution/homebrew/work.rb.template` (the `@VERSION@` / `@<key>_SHA@` tokens are substituted by CI):
```ruby
# Generated by .github/workflows/release.yml from distribution/homebrew/work.rb.template.
# Do not edit by hand — it is regenerated on every release.
class Work < Formula
  desc "Isolated multi-context session manager for developers"
  homepage "https://github.com/coelhucas-dev/work-cli"
  license "MIT"
  version "@VERSION@"

  on_macos do
    on_arm do
      url "https://github.com/coelhucas-dev/work-cli/releases/download/v@VERSION@/work-aarch64-apple-darwin.tar.gz"
      sha256 "@ARM64_SHA@"
    end
    on_intel do
      url "https://github.com/coelhucas-dev/work-cli/releases/download/v@VERSION@/work-x86_64-apple-darwin.tar.gz"
      sha256 "@INTEL_SHA@"
    end
  end
  on_linux do
    url "https://github.com/coelhucas-dev/work-cli/releases/download/v@VERSION@/work-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "@LINUX_SHA@"
  end

  def install
    bin.install "work"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/work --version")
  end
end
```

- [ ] **Step 2: Add the `update-tap` job**

Append to `.github/workflows/release.yml` (after the `build` job, same indentation `jobs:`):
```yaml
  update-tap:
    needs: build
    runs-on: ubuntu-latest
    steps:
      - name: Resolve version
        id: v
        run: echo "version=${GITHUB_REF_NAME#v}" >> "$GITHUB_OUTPUT"
      - name: Fetch assets + sha256
        id: sha
        run: |
          TAG="${GITHUB_REF_NAME}"
          BASE="https://github.com/${{ github.repository }}/releases/download/${TAG}"
          curl -fsSL "$BASE/work-aarch64-apple-darwin.tar.gz"     -o a.tar.gz
          curl -fsSL "$BASE/work-x86_64-apple-darwin.tar.gz"      -o i.tar.gz
          curl -fsSL "$BASE/work-x86_64-unknown-linux-gnu.tar.gz" -o l.tar.gz
          echo "arm64=$(sha256sum a.tar.gz | cut -d' ' -f1)" >> "$GITHUB_OUTPUT"
          echo "intel=$(sha256sum i.tar.gz | cut -d' ' -f1)" >> "$GITHUB_OUTPUT"
          echo "linux=$(sha256sum l.tar.gz | cut -d' ' -f1)" >> "$GITHUB_OUTPUT"
      - name: Render Formula
        run: |
          mkdir -p out/Formula
          sed \
            -e "s/@VERSION@/${{ steps.v.outputs.version }}/g" \
            -e "s/@ARM64_SHA@/${{ steps.sha.outputs.arm64 }}/g" \
            -e "s/@INTEL_SHA@/${{ steps.sha.outputs.intel }}/g" \
            -e "s/@LINUX_SHA@/${{ steps.sha.outputs.linux }}/g" \
            distribution/homebrew/work.rb.template > out/Formula/work.rb
          cat out/Formula/work.rb
      - name: Push to homebrew-tap
        uses: actions/checkout@v7
        with:
          repository: coelhucas-dev/homebrew-tap
          path: tap
          token: ${{ secrets.HOMEBREW_TAP_TOKEN }}
      - name: Commit formula
        run: |
          mkdir -p tap/Formula
          cp out/Formula/work.rb tap/Formula/work.rb
          cd tap
          git config user.name "github-actions[bot]"
          git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
          git add Formula/work.rb
          git commit -m "work ${{ steps.v.outputs.version }}" || echo "no changes"
          git push
```

- [ ] **Step 3: Lint the workflow YAML**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/release.yml')); print('yaml ok')"`
Expected: `yaml ok`.

- [ ] **Step 4: Commit**

```bash
git add distribution/homebrew/work.rb.template .github/workflows/release.yml
git commit -m "chore(dist): add Homebrew formula template + release tap-update job"
```

> **Note:** End-to-end validation of this job requires a real `v*` tag (Prerequisite P-3) and the tap repo + secret (P-1, P-2). On the first release, confirm the commit lands in `coelhucas-dev/homebrew-tap` and `brew install coelhucas-dev/tap/work` works on arm64 + Intel.

---

### Task 10: README install/upgrade rewrite + CHANGELOG

Surface the three install paths and the upgrade story.

**Files:**
- Modify: `README.md` (replace the "## Install" section, lines ~28-43)
- Modify: `CHANGELOG.md` (add entries under `## [Unreleased]`)

- [ ] **Step 1: Rewrite the README "Install" section**

Replace the existing `## Install` block (the "From source (for now; …)" section through the `work doctor` verify block) with:
````markdown
## Install

**Homebrew (recommended, macOS):**

```bash
brew install coelhucas-dev/tap/work
```

Upgrade with `brew upgrade work`.

**One-line script (macOS + Linux):**

```bash
curl -fsSL https://raw.githubusercontent.com/coelhucas-dev/work-cli/main/install.sh | sh
```

**cargo-binstall** (if you have a Rust toolchain):

```bash
cargo binstall --git https://github.com/coelhucas-dev/work-cli work
```

**From source** (developers):

```bash
cargo install --git https://github.com/coelhucas-dev/work-cli
# or, from a clone:
cargo install --path .
```

Verify:

```bash
work --version
work doctor     # engine sanity + isolation check (no workspaces yet)
```

## Upgrade

`work` checks for a new release once a day and prints a one-line hint (to stderr,
so it never interferes with scripting). The hint matches how you installed it
(`brew upgrade work`, `cargo install --git …`, or a link to Releases). To disable
the check, set in `~/.config/work/config.toml`:

```toml
[update]
check = false
```

…or export `WORK_NO_UPDATE_CHECK=1`. The check is automatically off in CI and
when output isn't a terminal.
````

- [ ] **Step 2: Add CHANGELOG entries**

In `CHANGELOG.md`, under `## [Unreleased]`, add an `### Added` block (create the heading if the Unreleased section has no `### Added` yet):
```markdown
### Added — distribution & update awareness
- Homebrew tap: `brew install coelhucas-dev/tap/work` (bottles from release
  assets); `cargo binstall` and a `curl | sh` installer as fallbacks.
- Non-intrusive update-available check: once a day, prints a channel-aware
  one-line hint to stderr. Opt out with `[update] check = false` or
  `WORK_NO_UPDATE_CHECK=1`; off in CI / non-TTY.
```

- [ ] **Step 3: Commit**

```bash
git add README.md CHANGELOG.md
git commit -m "docs: rewrite install/upgrade docs + changelog for distribution"
```

---

## Self-Review (run after writing — done inline)

**1. Spec coverage:**
- Update-available check semantics → Tasks 1-6 (gate, daily cache, background worker, channel-aware hint, opt-out, off in CI/non-TTY). ✓
- Source of truth `releases/latest`, tag parse, semver compare → Task 1. ✓
- Cache `~/.config/work/update-check.json`, atomic write → Task 4. ✓
- Non-blocking + bounded-join ≤1s → Task 6 (`UpdateGuard`/`Drop`/channel). ✓
- Channel-aware hint (Homebrew/cargo/generic) → Task 2. ✓
- New deps `ureq` + `semver` → Task 1. ✓
- Opt-out config + env → Task 3. ✓
- Homebrew tap (separate repo, bottles) → Task 9 + Prerequisites. ✓
- cargo-binstall fallback → Task 7. ✓
- `curl | sh` fallback → Task 8. ✓
- README rewrite + Upgrade section → Task 10. ✓

**2. Placeholder scan:** None. Every code step has complete, compilable code; every test has real assertions. The formula `@VERSION@`/`@*_SHA@` tokens are template variables consumed by the CI `sed` (Task 9), not placeholders.

**3. Type consistency:** `detect_channel`/`Channel::hint` (Task 2) match the call site in `run_check` (Task 6). `is_enabled` signature (Task 3) matches `enabled_in_env` (Task 6). `read_cache`/`write_cache_atomic`/`is_stale` (Task 4) match `run_check` + `refresh_cache` (Tasks 5-6). `Cache`/`cache_path` consistent across Tasks 4-6. `UpdatePrefs` (Task 3) field name `check` matches `g.update.check` (Task 6).

## Out of plan (per spec's "Out of scope")

- Self-update command, post-upgrade changelog splash, background daemons, Windows, authenticated release checks.

## Execution Handoff

Two execution options:

1. **Subagent-Driven (recommended)** — dispatch a fresh subagent per task with two-stage review between tasks.
2. **Inline Execution** — execute tasks in this session via executing-plans, batched with checkpoints.

Which approach?
