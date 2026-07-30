# Update-awareness & macOS distribution for `work`

**Date:** 2026-07-29
**Status:** Proposed
**Theme:** Give installed users a non-intrusive nudge when a new `work` release ships, and make `work` installable/upgradeable the idiomatic macOS way. These two features co-release as the first **tagged** release (today the README/CHANGELOG both say "not yet tagged").

## Goal

Two coupled asks:

1. **Update-available check.** When the project ships a new version, a running `work` notices its installed version is behind the latest release and prints a one-line, channel-aware hint ("`work 0.2.0 available — brew upgrade work`"). Best-effort, opt-out, never blocks, never errors.
2. **Install on macOS.** A real distribution story beyond `cargo install`: Homebrew tap (primary) with cargo-binstall and a `curl | sh` one-liner as fallbacks. All reuse the binaries the existing `release.yml` already builds.

They are coupled because the install *channel* determines the exact upgrade command the update-check suggests, and Homebrew's `brew upgrade` is the native close-the-loop path.

## Background — what exists today

- **Release infra:** `.github/workflows/release.yml` already builds release binaries for `aarch64-apple-darwin`, `x86_64-apple-darwin`, and `x86_64-unknown-linux-gnu` on `v*` tag pushes, packages each as `work-<target>.tar.gz`, and attaches them to a GitHub Release via `softprops/action-gh-release`. So the binary artifacts the distribution paths consume already exist.
- **Install:** README says *"From source (for now; Homebrew tap + release binaries are planned)"* — only `cargo install --git` / `--path`.
- **Update awareness:** none. No version-tracking state, no `upgrade`/`update` command.
- **Config:** host-side, non-secret metadata only — global `~/.config/work/config.toml` (`GlobalConfig`), per-workspace `~/.config/work/workspaces/<ws>.toml`. `work` "never reads, writes, or moves secrets."

## Architecture

- All update logic lives in a new `work-core` module `crates/core/src/update.rs`, re-exported from `lib.rs`. The CLI calls one entry point. Matches the repo's existing PURE/testable style (see `normalize_help_arg`).
- Distribution is primarily external artifacts (a tap repo, a formula template, an installer script) plus small in-repo additions (cargo-binstall metadata, README). The release pipeline gains one job to keep the formula current.
- Neither feature touches isolation: no new mount, network, user, port, or secret path. `work doctor`'s invariants are unchanged.

---

## Part 1 — Update-available check

### Trigger & gate

The check runs at the start of `main()`, before dispatch, on **every** invocation path — including the bare `work <ws>` attach path (where most users will actually see the notice) and the `work resume` cockpit. It is **enabled iff all** hold:

- `stderr` is a TTY (`std::io::IsTerminal`, already used in `commands.rs`).
- `CI` env var is unset.
- `[update] check` is not `false` in `~/.config/work/config.toml` (default `true`).
- `WORK_NO_UPDATE_CHECK` env var is unset.

Off-TTY / piped / scripted / CI runs do no network call and print nothing — so they never corrupt `work ls`/`work config` output or slow down automation.

### Source of truth

GitHub Releases API, unauthenticated:

```
GET https://api.github.com/repos/coelhucas-dev/work-cli/releases/latest
```

Parse `tag_name` (e.g. `v0.2.0`), strip the leading `v`, parse with the `semver` crate, compare against `CARGO_PKG_VERSION`. Unauthenticated calls are rate-limited to 60/hour per IP — a daily check is nowhere near that. (Pre-release tags are excluded by `/releases/latest`.)

### Flow (non-blocking)

1. **Read cache** `~/.config/work/update-check.json` = `{ "last_check": <ISO8601>, "latest": "0.2.0" }`. This is transient runtime state, kept **out** of the user-edited `config.toml`.
2. If `latest` (semver) > current → print one stderr line, **channel-aware** (see below).
3. **Spawn a worker thread** only if the cache is older than 24h or missing. The worker does the HTTP GET (2s timeout via `ureq`), parses the tag, and **atomically** writes the cache (write to `update-check.json.tmp`, `fsync`, `rename` — so a process killed mid-write never leaves a corrupt cache). Any network/parse error is silently swallowed.
4. The main command runs uninterrupted. At process exit, the worker is **joined with a bounded wait of ≤1s** (only when the cache was stale). This guarantees the cache actually refreshes for short-lived commands (`work ls`, `work doctor`); the dominant `work <ws>` attach runs for the whole session, so its worker finishes near-instantly and the join returns immediately. The join only refreshes the *cache* — this run already printed from the prior cache.

**Non-blocking guarantee:** the user's command never waits on the network. The only cost is a process-exit pause of at most ~1s, once per day, only on the invocation that finds a stale cache — and only to finish writing the cache for next time.

### Channel-aware hint

The message is tailored to how `work` was installed, detected from the running binary's path (`std::env::current_exe()`):

- `…/Cellar/work/…` (or `/opt/homebrew/...`, `/usr/local/...` Cellar) → `work 0.2.0 available — run "brew upgrade work"`
- `…/.cargo/bin/work` → `work 0.2.0 available — run "cargo install --git https://github.com/coelhucas-dev/work-cli"`
- otherwise → `work 0.2.0 available — see https://github.com/coelhucas-dev/work-cli/releases`

All printed to **stderr** (never stdout), so piping `work ls` is unaffected. A single line; no ANSI when not a TTY (already guaranteed off-TTY suppression).

### New dependencies

- `ureq` — tiny, synchronous HTTP client. Fits the worker-thread model (no async runtime needed). Added to `work-core`.
- `semver` — parse/compare versions, handles pre-release. Added to `work-core`.
- `serde_json` — already a workspace dependency; used to read the releases JSON and the cache.

### Opt-out

Two ways, both honored by the gate:

- `~/.config/work/config.toml`:
  ```toml
  [update]
  check = false
  ```
- env: `WORK_NO_UPDATE_CHECK=1` (handy for CI/automation without editing config).

---

## Part 2 — Install on macOS: Homebrew + fallbacks

### 2a. Homebrew tap (primary)

- New repository `coelhucas-dev/homebrew-tap` containing `Formula/work.rb`.
- Users install with `brew install coelhucas-dev/tap/work` and upgrade with `brew upgrade work` — the native, idiomatic macOS path that the update-check's hint points to.
- The formula uses **pre-built bottles** (binaries) from the existing `release.yml` assets — no Rust toolchain required for end users:

  ```ruby
  class Work < Formula
    desc "Isolated multi-context session manager for developers"
    homepage "https://github.com/coelhucas-dev/work-cli"
    license "MIT"
    version "0.2.0"

    on_macos do
      on_arm do
        url "https://github.com/coelhucas-dev/work-cli/releases/download/v0.2.0/work-aarch64-apple-darwin.tar.gz"
        sha256 "<arm64-sha>"
      end
      on_intel do
        url "https://github.com/coelhucas-dev/work-cli/releases/download/v0.2.0/work-x86_64-apple-darwin.tar.gz"
        sha256 "<x86_64-sha>"
      end
    end
    on_linux do
      url "https://github.com/coelhucas-dev/work-cli/releases/download/v0.2.0/work-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "<linux-sha>"
    end

    def install
      bin.install "work"
    end

    test do
      assert_match version.to_s, shell_output("#{bin}/work --version")
    end
  end
  ```

- **Auto-update job** (extend `release.yml` or add `tap.yml`, triggered after the release publishes): download each published asset, compute its sha256, render `Formula/work.rb` from a checked-in template, and commit it to `homebrew-tap`. **Infra prerequisite to flag:** a PAT or deploy key with `contents: write` to the tap repo, stored as a repository secret.

### 2b. cargo-binstall (fallback for Rust toolchains)

Add `[package.metadata.binstall]` to `crates/cli/Cargo.toml` so `cargo binstall --git https://github.com/coelhucas-dev/work-cli work` fetches the correct pre-built binary:

```toml
[package.metadata.binstall]
pkg-url = "{ repo }/releases/download/v{ version }/work-{ target }.tar.gz"
bin-dir = "work-{ bin }{ binary-ext }"
```

Zero new infra — reuses the same release tarballs.

### 2c. `curl | sh` one-liner (universal fallback)

Ship `install.sh` at the repo root (served via `raw.githubusercontent.com`). It detects OS + arch, fetches the matching tarball from the latest release, and extracts `work` to a writable bin dir (`/usr/local/bin` on macOS, falling back to `$CARGO_HOME/bin` then `~/.local/bin`, prompting if none is writable). Documented as:

```bash
curl -fsSL https://raw.githubusercontent.com/coelhucas-dev/work-cli/main/install.sh | sh
```

Standard, well-understood pattern; the universal fallback for users without Homebrew or a Rust toolchain.

### 2d. README "Install" rewrite

Replace the "From source (for now)" section. Order by recommendation:

1. **Homebrew (recommended, macOS):** `brew install coelhucas-dev/tap/work`; upgrade via `brew upgrade work`.
2. **One-line script:** the `curl | sh` command.
3. **cargo-binstall:** for users with a Rust toolchain.
4. **From source (devs):** `cargo install --git …` (keep, demoted).

Add an **Upgrade** subsection that the update-notice cross-references — the notice already suggests the right command per channel, so the doc just reinforces it.

---

## Isolation impact

**None.** Both features are host-side and metadata/display-only:

- The update check is an outbound HTTPS read to a public API plus a local JSON cache file — both entirely outside any container. It never runs inside a workspace and touches no mount, network, user, or port.
- Distribution changes (formula, installer script, binstall metadata, README) are external artifacts; the `work` binary and its isolation invariants are untouched.
- `work doctor`'s invariants (unique network per workspace, only its own volume mounted, non-root, no host ports, image match) are unchanged.
- No new cross-container path; no host bind-mount; no secret is moved or read. The cache and config additions contain only non-secret version strings and a boolean preference.

## Files affected

- `crates/core/src/update.rs` — **new**: gate logic, cache read/write (atomic), worker thread, semver compare, channel detection, pure parse helpers; `update` module re-exported from `lib.rs`.
- `crates/core/src/config.rs` — `GlobalConfig` gains `update: UpdatePrefs` (`#[serde(default)]`, with `check: bool` defaulting `true`).
- `crates/core/src/lib.rs` — `pub mod update;`.
- `crates/core/Cargo.toml` — add `ureq`, `semver` (`serde_json` and `chrono` already present).
- `crates/cli/src/main.rs` — call the update entry point at the top of `main()` (before the bare-name dispatch), and bounded-join the worker at exit.
- `crates/cli/Cargo.toml` — add `[package.metadata.binstall]`.
- `install.sh` — **new**: repo-root installer script.
- `Formula/work.rb` template — **new** (lives in `homebrew-tap`, templated/rendered by CI).
- `.github/workflows/release.yml` — add the formula-render-and-commit job (or a `tap.yml`).
- `README.md` — rewrite "Install"; add "Upgrade".
- `CHANGELOG.md` — entry under the first tagged release.

## Testing

- **Unit (pure), no network:**
  - `strip_tag("v0.2.0") == "0.2.0"`; `is_newer("0.1.0", "0.2.0") == true`; pre-release ordering (`0.2.0-rc.1 < 0.2.0`).
  - Cache round-trip (serialize/deserialize) and atomic-write via `tempfile`.
  - Gate logic truth table (TTY/CI/config/env combinations) with stubbed predicates.
  - Channel detection given a synthetic `current_exe` path → expected hint string.
  - `parse_latest_tag(releases_json)` extracts `tag_name`.
- **HTTP isolation:** the fetch is behind an injectable `Fn` (or a trait), so unit tests inject a canned response / error and assert "swallowed error → no cache write, no panic."
- **Integration (manual milestone):** on a dev build, point the source URL at a fake "latest = newer" response, run `work ls` → assert the one-liner prints to stderr and stdout is untouched; assert a second run within 24h does no network (stale-cache path); assert `WORK_NO_UPDATE_CHECK=1` and `[update] check = false` both suppress it.
- **Distribution smoke (manual):** `brew install coelhucas-dev/tap/work` on arm64 + x86_64; `cargo binstall --git … work`; `curl … | sh` — each yields a working `work --version` / `work doctor`.

## Risks / open questions

1. **Tap-repo + PAT** is external infra outside this repo. Decision needed: separate `homebrew-tap` repo (idiomatic) vs. a `tap/` subdir served from this repo. Recommendation: separate repo (Homebrew convention; cleaner history/secrets).
2. **Join-at-exit ≤1s/day** is a deliberate tradeoff for cache freshness on short-lived commands. Alternative considered: fully detached worker (zero exit latency) — rejected because it never refreshes for users who only ever run short commands and exits before the fetch completes. The bounded join is the robust middle.
3. **Channel detection is heuristic.** A binary copied out of its Cellar won't be detected as Homebrew. Fallback is the generic releases link — acceptable; detection only improves the message, never blocks it.
4. **Rate limits / GitHub outages.** Unauthenticated `/releases/latest` at 60/hr is ample for a daily check; failures are swallowed. If the project later moves off GitHub, the source URL is a single constant.
5. **First-tagged-release sequencing:** the formula/binstall/install.sh all depend on a real `v*` tag with published assets. The update-check is usable immediately from `cargo install` builds (compares against the latest tag, even before a Homebrew build exists).

## Out of scope

- Background daemons, launchd agents, or OS-level auto-update — the check is strictly a one-line nudge; the user performs the upgrade.
- Post-upgrade "what's new" / changelog splash (the other interpretation we set aside).
- A self-update command (`work upgrade` performing the upgrade itself) — out of scope; the hint points the user at the channel-native command.
- Windows support.
- Authenticated/GitHub-token release checks.
