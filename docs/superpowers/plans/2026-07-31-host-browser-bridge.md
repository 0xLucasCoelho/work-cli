# Host Browser Bridge (`work browse`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let in-container tools' browser-open attempts (Claude Code, Cursor CLI, `gh`, Python `webbrowser`, …) open in the user's real host browser via an explicit `work browse <ws>` foreground bridge.

**Architecture:** An `xdg-open` shim inside the container writes each `http(s)` URL to a FIFO in the workspace volume; `work browse <ws>` reads that FIFO on the host and launches each URL via the host browser opener (`open`/`xdg-open`). Container-wide `$BROWSER` points at the shim. No network/host-gateway/ports/mounts are added — isolation and `work doctor` are untouched. Mirrors the existing `work fwd` pattern.

**Tech Stack:** Rust 2021, `anyhow`, `clap`, the `work-core` `Engine` trait + `DockerCli` adapter, POSIX `sh` for the in-container shim.

## Global Constraints

- Isolation invariants (`work doctor`) must stay green: no new container networks, no `--add-host`/host-gateway, no published host ports, no extra mounts, no change to user/restart/image.
- No new workspace dependencies (no `which`/`open` crate); use `std::process::Command`.
- Follow existing patterns exactly: free fn `workspace::browse` mirrors `workspace::forward`; `Engine::exec_root` mirrors the `--user root` pattern already in `seed_file`/`seed_dir`; `Command::Browse` mirrors `Command::Fwd`.
- Pure logic is unit-tested in `#[cfg(test)] mod tests` (matches the codebase). Docker-touching code is validated by the manual smoke test (Task 7) — consistent with how `run_forwarder`/`forward`/`create` are validated (the `tests/integration.rs` suite is `#[ignore]`).
- Reserve the token `"browse"` in **both** `RESERVED` lists so it can't be a workspace name.
- All work in the worktree `.worktrees/feat-browser-forwarding` (branch `feat-browser-forwarding`).

## File Structure

- **Create** `crates/core/src/browser.rs` — browser-bridge primitives: `SHIM` (shell script), path consts, `host_opener_for`/`host_opener` (host browser selection), `is_openable_url` (http(s) filter), `install_shim` (root-install the shim via `exec_root`), `ensure_fifo` (create the FIFO as dev). Unit-tested pure helpers live here.
- **Modify** `crates/core/src/lib.rs` — `pub mod browser;`
- **Modify** `crates/core/src/engine.rs` — add `Engine::exec_root` (+ `DockerCli` impl).
- **Modify** `crates/core/src/workspace.rs` — `run_opts` gains `BROWSER` env; new free fn `browse`; `create` calls `browser::install_shim`. Update the existing `run_opts_sets_identity_env_and_names` test.
- **Modify** `crates/core/src/naming.rs` — add `"browse"` to `RESERVED`.
- **Modify** `crates/cli/src/commands.rs` — `pub fn browse`.
- **Modify** `crates/cli/src/main.rs` — `Command::Browse` variant + dispatch arm; add `"browse"` to `RESERVED`.
- **Modify** `README.md`, `CHANGELOG.md`.

---

### Task 1: Add `Engine::exec_root`

A general "run a command inside the container as root, success-required" primitive, mirroring the `--user root` usage already in `seed_file`/`seed_dir`. Not unit-tested (engine adapter over `docker`, like `run_forwarder`); exercised by the smoke test.

**Files:**
- Modify: `crates/core/src/engine.rs` (trait, after `exec_capture` ~line 76; impl, after `exec_capture` ~line 370)

**Interfaces:**
- Produces: `fn exec_root(&self, name: &str, cmd: &[&str]) -> Result<()>` on the `Engine` trait + `DockerCli`.

- [ ] **Step 1: Add the trait method**

In `crates/core/src/engine.rs`, add to the `Engine` trait immediately after `exec_capture` (after line 76):

```rust
    /// `docker exec --user root <name> <cmd...>` (non-interactive), require
    /// success. For one-off system setup the `dev` user can't perform (e.g.
    /// installing a shim under `/usr/local/bin`). Mirrors the `--user root`
    /// pattern in seed_file/seed_dir, but as a general exec.
    fn exec_root(&self, name: &str, cmd: &[&str]) -> Result<()>;
```

- [ ] **Step 2: Add the `DockerCli` impl**

In `impl Engine for DockerCli`, immediately after the `exec_capture` impl (after line 370):

```rust
    fn exec_root(&self, name: &str, cmd: &[&str]) -> Result<()> {
        let out = self
            .cmd()
            .args(["exec", "--user", "root", name])
            .args(cmd)
            .output()
            .with_context(|| format!("exec (root) into {name}"))?;
        if !out.status.success() {
            bail!(
                "exec (root) into {name} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }
```

- [ ] **Step 3: Compile-check**

Run: `cargo build -p work-core`
Expected: builds clean (no warnings from the new code).

- [ ] **Step 4: Commit**

```bash
git add crates/core/src/engine.rs
git commit -m "feat(core): add Engine::exec_root (run a command in a container as root)"
```

---

### Task 2: `browser` module — pure helpers (TDD) + shim/fifo primitives

Create the module with the unit-testable pure helpers first (TDD), then the non-pure primitives. The pure helpers are `host_opener_for(os)` and `is_openable_url(s)`.

**Files:**
- Create: `crates/core/src/browser.rs`
- Modify: `crates/core/src/lib.rs` (register module)

**Interfaces:**
- Produces: `pub const SHIM: &str`, `pub const FIFO_PATH: &str = "/home/dev/.work/browser.fifo"`, `pub const SHIM_DEST: &str = "/usr/local/bin/xdg-open"`, `pub fn host_opener_for(os: &str) -> &'static str`, `pub fn host_opener() -> String`, `pub fn is_openable_url(s: &str) -> bool`, `pub fn install_shim(engine: &dyn Engine, ctr: &str) -> Result<()>`, `pub fn ensure_fifo(engine: &dyn Engine, ctr: &str) -> Result<()>`.
- Consumes: `crate::engine::Engine` (for `install_shim`/`ensure_fifo`), `anyhow::{Result, bail}`.

- [ ] **Step 1: Register the module**

In `crates/core/src/lib.rs`, add (e.g. after `pub mod banner;` to keep alphabetical-ish order):

```rust
pub mod browser;
```

- [ ] **Step 2: Write the failing tests**

Create `crates/core/src/browser.rs` with only the tests + stubs that won't typecheck-meaningfully yet — actually write the tests referencing not-yet-defined items so compilation fails:

```rust
//! Browser bridge primitives: the in-container `xdg-open` shim, the volume
//! FIFO path, host-browser selection, and the http(s) URL filter. Pure helpers
//! are unit-tested; `install_shim`/`ensure_fifo` touch the container and are
//! validated by the `work browse` smoke test.

use anyhow::{bail, Context, Result};

use crate::engine::Engine;

/// Where the FIFO lives (in the workspace volume, so it persists).
pub const FIFO_PATH: &str = "/home/dev/.work/browser.fifo";
/// Where the shim is installed (system path, on PATH for every shell).
pub const SHIM_DEST: &str = "/usr/local/bin/xdg-open";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_opener_per_os() {
        assert_eq!(host_opener_for("macos"), "open");
        assert_eq!(host_opener_for("linux"), "xdg-open");
        assert_eq!(host_opener_for("freebsd"), "xdg-open"); // unknown -> xdg-open
    }

    #[test]
    fn openable_url_filters_scheme() {
        assert!(is_openable_url("https://example.com/login"));
        assert!(is_openable_url("http://127.0.0.1:8080/cb?code=x"));
        assert!(!is_openable_url("ftp://example.com"));
        assert!(!is_openable_url("mailto:a@b.com"));
        assert!(!is_openable_url("file:///etc/hosts"));
        assert!(!is_openable_url(""));
        assert!(!is_openable_url("not a url"));
        // tolerates surrounding whitespace (docker exec capture can pad)
        assert!(is_openable_url("  https://example.com  "));
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p work-core browser 2>&1 | tail -5`
Expected: compile error — `host_opener_for` / `is_openable_url` not defined.

- [ ] **Step 4: Implement the pure helpers + consts + shim**

Append to `crates/core/src/browser.rs` (above the `#[cfg(test)]` block):

```rust
/// The in-container shim. Forwards an http(s) URL to the FIFO (non-blocking,
/// 5s timeout) and always echoes it. Non-URL args are a silent no-op so calls
/// on files/dirs don't break tools.
pub const SHIM: &str = r#"#!/bin/sh
# `work` browser shim: forward an http(s) URL to the host browser via the
# `work browse` bridge (a FIFO in the volume). Installed by `work new` /
# `work browse`. With no bridge running it still echoes the URL.
url=
for a in "$@"; do
  case "$a" in
    http://*|https://*) url="$a"; break ;;
  esac
done
[ -z "$url" ] && url="${1:-}"
case "$url" in
  http://*|https://*) ;;
  *) exit 0 ;;
esac
fifo="$HOME/.work/browser.fifo"
if [ -p "$fifo" ]; then
  timeout 5 sh -c 'printf "%s\n" "$1" > "$2"' sh "$url" "$fifo" 2>/dev/null
fi
printf '\n🌐  %s\n\n' "$url"
"#;

/// Host browser binary for a given OS string (`std::env::consts::OS`). PURE.
pub fn host_opener_for(os: &str) -> &'static str {
    match os {
        "macos" => "open",
        _ => "xdg-open",
    }
}

/// Effective host browser opener: `$WORK_HOST_BROWSER` wins if set, else the
/// OS default. (The override is used verbatim — the caller owns it.)
pub fn host_opener() -> String {
    if let Some(b) = std::env::var_os("WORK_HOST_BROWSER") {
        return b.to_string_lossy().into_owned();
    }
    host_opener_for(std::env::consts::OS).to_string()
}

/// True iff `s` is an `http(s)` URL (the only thing the bridge forwards). PURE.
pub fn is_openable_url(s: &str) -> bool {
    let s = s.trim();
    s.starts_with("http://") || s.starts_with("https://")
}

/// Idempotently install the shim + symlinks + profile.d export as root.
pub fn install_shim(engine: &dyn Engine, ctr: &str) -> Result<()> {
    let script = format!(
        "set -e\n\
         cat > {dest} <<'WORK_SHIM_EOF'\n{shim}WORK_SHIM_EOF\n\
         chmod 0755 {dest}\n\
         ln -sf {dest} /usr/local/bin/sensible-browser\n\
         ln -sf {dest} /usr/local/bin/x-www-browser\n\
         mkdir -p /etc/profile.d\n\
         cat > /etc/profile.d/work-browser.sh <<'WORK_PROF_EOF'\nexport BROWSER={dest}\nWORK_PROF_EOF\n\
         chmod 0644 /etc/profile.d/work-browser.sh\n",
        dest = SHIM_DEST,
        shim = SHIM,
    );
    engine
        .exec_root(ctr, &["sh", "-c", &script])
        .with_context(|| format!("installing browser shim in {ctr}"))
}

/// Idempotently create the FIFO (owned by dev). Runs as dev.
pub fn ensure_fifo(engine: &dyn Engine, ctr: &str) -> Result<()> {
    engine.exec_capture(
        ctr,
        &[
            "sh",
            "-c",
            "mkdir -p \"$HOME/.work\"; [ -p \"$HOME/.work/browser.fifo\" ] || mkfifo \"$HOME/.work/browser.fifo\"",
        ],
    )?;
    // mkfifo as dev can fail only if ~ isn't writable; surface a clear error.
    if engine
        .exec_capture(ctr, &["test", "-p", FIFO_PATH])
        .is_err()
    {
        bail!("could not create browser FIFO {FIFO_PATH} in {ctr}");
    }
    Ok(())
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p work-core browser 2>&1 | tail -8`
Expected: `test result: ok. 2 passed; 0 failed`.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/browser.rs crates/core/src/lib.rs
git commit -m "feat(core): browser bridge primitives (shim, fifo, host opener, url filter)"
```

---

### Task 3: `run_opts` sets `BROWSER` (TDD on existing code)

The shim is only useful if tools find it. Set `$BROWSER` container-wide. The existing test `run_opts_sets_identity_env_and_names` asserts the exact env vec, so it must be updated in lockstep.

**Files:**
- Modify: `crates/core/src/workspace.rs` (test ~line 623, `run_opts` ~line 369)

**Interfaces:**
- Produces: `RunOpts.env` now includes `("BROWSER", "/usr/local/bin/xdg-open")`.

- [ ] **Step 1: Update the failing test**

In `crates/core/src/workspace.rs`, in `run_opts_sets_identity_env_and_names`, replace the `assert_eq!(opts.env, ...)` block with:

```rust
        assert_eq!(
            opts.env,
            vec![
                ("WORK".to_string(), "acme".to_string()),
                ("WORKSPACE".to_string(), "acme".to_string()),
                (
                    "BROWSER".to_string(),
                    "/usr/local/bin/xdg-open".to_string()
                ),
            ]
        );
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p work-core run_opts_sets_identity_env_and_names 2>&1 | tail -12`
Expected: FAIL — the left vec lacks the `BROWSER` entry.

- [ ] **Step 3: Implement — add the env to `run_opts`**

In `run_opts` (~line 378), change the `env` vec to:

```rust
        env: vec![
            ("WORK".into(), name.into()),
            ("WORKSPACE".into(), name.into()),
            ("BROWSER".into(), crate::browser::SHIM_DEST.into()),
        ],
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p work-core run_opts_sets_identity_env_and_names 2>&1 | tail -4`
Expected: `test result: ok. 1 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/workspace.rs
git commit -m "feat(core): set BROWSER=/usr/local/bin/xdg-open on workspace containers"
```

---

### Task 4: `workspace::browse` orchestration + install-on-create

The foreground bridge loop (mirrors `workspace::forward`), plus seeding the shim at `work new` so new workspaces ship with it.

**Files:**
- Modify: `crates/core/src/workspace.rs` (new `browse` fn near `forward`; one call in `create`)

**Interfaces:**
- Consumes: `crate::browser::{install_shim, ensure_fifo, host_opener, is_openable_url, FIFO_PATH}`, `crate::engine::Engine`, `std::process::Command`.
- Produces: `pub fn browse(name: &str) -> Result<()>`.

- [ ] **Step 1: Add `browse` next to `forward`**

In `crates/core/src/workspace.rs`, immediately after the `forward` fn (after line 444), add:

```rust
/// `work browse <ws>`: forward URLs that in-container tools try to open
/// (`xdg-open`/`$BROWSER`) to the host browser. Installs/refreshes the shim,
/// ensures the volume FIFO exists, then blocks reading it — each `http(s)`
/// URL is opened via the host browser opener. Ctrl-C stops (the container
/// is unaffected; the FIFO persists). Mirrors `work fwd`.
pub fn browse(name: &str) -> Result<()> {
    let ws = Workspace::open(name)?;
    ws.ensure_running()?;
    let engine = ws.engine();
    let ctr = naming::container(name);
    crate::browser::install_shim(engine, &ctr)?;
    crate::browser::ensure_fifo(engine, &ctr)?;
    println!("Browsing for {name} — URLs tools open will launch in your host browser.");
    println!("(Ctrl-C to stop)");
    let opener = crate::browser::host_opener();
    loop {
        let line = engine.exec_capture(&ctr, &["cat", crate::browser::FIFO_PATH])?;
        let url = line.trim();
        if crate::browser::is_openable_url(url) {
            match Command::new(&opener).arg(url).status() {
                Ok(_) => println!("↗ opened {url}"),
                Err(e) => eprintln!("· could not open {url} via {opener} ({e})"),
            }
        } else if !url.is_empty() {
            eprintln!("· ignored non-http(s) target: {url}");
        }
    }
}
```

- [ ] **Step 2: Seed the shim at create time**

In `Workspace::create`, after `engine.run(&opts)?;` (line 132) and before the dotfiles-seeding block (line 134), add:

```rust
        // Browser bridge shim: install early so even a brand-new workspace
        // can forward `xdg-open` calls. Idempotent; best-effort (warn, don't
        // fail workspace creation over a convenience shim).
        let _ = crate::browser::install_shim(&*engine, &ctr);
```

- [ ] **Step 3: Compile-check + run unit tests**

Run: `cargo test -p work-core 2>&1 | tail -6`
Expected: builds clean; all existing tests still pass.

- [ ] **Step 4: Commit**

```bash
git add crates/core/src/workspace.rs
git commit -m "feat(core): workspace::browse host-browser bridge + install shim on create"
```

---

### Task 5: CLI wiring (`work browse <ws>`)

Expose the bridge as a `work` subcommand mirroring `work fwd`, and reserve the token.

**Files:**
- Modify: `crates/cli/src/commands.rs` (new `browse`)
- Modify: `crates/cli/src/main.rs` (`Command::Browse` variant, dispatch arm, `RESERVED`)
- Modify: `crates/core/src/naming.rs` (`RESERVED`)

**Interfaces:**
- Produces: `commands::browse(ws)`, `Command::Browse { ws }`.

- [ ] **Step 1: Add `commands::browse`**

In `crates/cli/src/commands.rs`, immediately after the `fwd` fn (after line 271), add:

```rust
/// `work browse <ws>`: forward URLs tools open inside the container to the
/// host browser (for OAuth/subscription logins). Ctrl-C stops.
pub fn browse(ws: &str) -> Result<()> {
    workspace::browse(ws)
}
```

(Confirm `workspace` is imported at the top of `commands.rs` — it is, since `fwd` calls `workspace::forward`.)

- [ ] **Step 2: Add the `Command::Browse` variant**

In `crates/cli/src/main.rs`, immediately after the `Fwd { .. }` variant (after line 78), add:

```rust
    /// Forward URLs that in-container tools open (`xdg-open`/`$BROWSER`) to your
    /// host browser — for OAuth/subscription logins (Claude Code, Cursor CLI, …).
    ///
    /// Installs an `xdg-open` shim in the workspace that sends each http(s) URL
    /// to a volume FIFO; this command reads it and opens each URL in your real
    /// browser. Ctrl-C stops it (the container keeps running).
    ///
    /// Example:
    ///   work browse acme
    Browse {
        /// Workspace whose browser-open requests to forward.
        ws: String,
    },
```

- [ ] **Step 3: Add the dispatch arm**

In `crates/cli/src/main.rs`, immediately after the `Fwd` arm (after line 220), add:

```rust
        Some(Command::Browse { ws }) => commands::browse(&ws)?,
```

- [ ] **Step 4: Reserve the token in both lists**

In `crates/cli/src/main.rs` `RESERVED` (line 145+), add `"browse",` (e.g. after `"all",`).
In `crates/core/src/naming.rs` `RESERVED` (line 4+), add `"browse",` (e.g. after `"all",`).

- [ ] **Step 5: Build the CLI + check the reserved-name test**

Run: `cargo build -p work 2>&1 | tail -5`
Run: `cargo test -p work-core naming 2>&1 | tail -5`
Expected: clean build; naming tests pass (the `invalid_names_rejected` test covers reserved tokens).

- [ ] **Step 6: Commit**

```bash
git add crates/cli/src/commands.rs crates/cli/src/main.rs crates/core/src/naming.rs
git commit -m "feat(cli): `work browse <ws>` subcommand"
```

---

### Task 6: Docs (README + CHANGELOG)

**Files:**
- Modify: `README.md` (the "Logging into tools that need a browser (OAuth)" section; CLI reference table)
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Update the OAuth section in README**

Replace the existing "## Logging into tools that need a browser (OAuth)" section body with one that leads with `work browse` and keeps `work fwd` for the callback port. New section:

```markdown
## Logging into tools that need a browser (OAuth)

CLIs like Claude Code, Cursor, or `gh` open a browser URL to log in. Inside a
container there's no browser, so `work` forwards those open-requests to your
**host** browser.

```bash
# In one terminal, start the bridge:
work browse acme
#   -> URLs that tools open inside `acme` now launch in your host browser.
#      Ctrl-C stops it (the container keeps running).
```

It installs an `xdg-open` shim in the workspace (and sets `$BROWSER`) that
sends each `http(s)` URL to a FIFO in the volume; `work browse` reads it and
opens each URL via `open` (macOS) / `xdg-open` (Linux). Override the host
opener with `WORK_HOST_BROWSER=<bin>`. Existing workspaces get the shim on
first `work browse` — no image rebuild.

Some logins additionally need a callback to `localhost:<port>`. For those,
`work` offers an **opt-in, manual** port bridge alongside `work browse`:

```bash
work fwd acme 8080      # bridge http://127.0.0.1:8080 -> acme:8080
# Complete the login in your browser, then Ctrl-C to tear the bridge down.
```
```

- [ ] **Step 2: Add a `work browse` row to the CLI reference table**

In the README CLI reference table, add (next to the `work fwd` row):

```markdown
| `work browse <ws>` | Forward URLs tools open inside the workspace to your host browser (OAuth logins). Ctrl-C stops. |
```

- [ ] **Step 3: Add a CHANGELOG entry**

Prepend an entry under the topmost `[Unreleased]` (or a new section header if none) in `CHANGELOG.md`:

```markdown
- `work browse <ws>`: forward URLs that in-container tools open (`xdg-open`/`$BROWSER`) to your host browser, so OAuth/subscription logins (Claude Code, Cursor CLI, …) complete without leaving the terminal.
```

- [ ] **Step 4: Commit**

```bash
git add README.md CHANGELOG.md
git commit -m "docs: document `work browse` host-browser bridge"
```

---

### Task 7: Verify — full suite + manual smoke test

**Files:** none (verification only)

- [ ] **Step 1: Full workspace test suite**

Run: `cargo test --workspace 2>&1 | tail -20`
Expected: all tests pass (baseline was green; new browser tests + updated run_opts test added).

- [ ] **Step 2: `cargo fmt` + clippy**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -10`
Expected: no warnings. (Fix any formatting/lint nits inline before committing.)

- [ ] **Step 3: Build a release binary for the smoke test**

Run: `cargo build --release -p work 2>&1 | tail -3`
Expected: builds `target/release/work`.

- [ ] **Step 4: Smoke test — new workspace**

Requires a running container engine (OrbStack/Docker). In the worktree:

```bash
W=./target/release/work
$W new smoketest --yes
# Terminal A:
$W browse smoketest        # expect: "Browsing for smoketest … (Ctrl-C to stop)"
# Terminal B (attach): $W smoketest, then inside the container run:
xdg-open https://example.com
```
Expected: Terminal A prints `↗ opened https://example.com` and the URL opens in the host browser; the container shell also prints `🌐  https://example.com`.

- [ ] **Step 5: Smoke test — existing workspace + non-URL + Ctrl-C**

```bash
# existing/pre-feature workspace gets the shim on first browse:
$W browse <an-old-ws>          # then xdg-open https://example.com inside it
# non-URL is ignored:
xdg-open /etc/hostname         # no-op, no host open, no error in `work browse`
# Ctrl-C in the `work browse` terminal stops it cleanly; the container keeps running.
$W doctor                      # isolation still green
```
Expected: existing workspace forwards on first run; non-URL ignored; Ctrl-C exits `work browse`; `work doctor` reports all ok; container unaffected.

- [ ] **Step 6: Final commit (if any lint fixes)**

```bash
git add -A
git commit -m "chore: fmt/clippy" --allow-empty
```

## Self-review notes
- Spec coverage: shim ✓ (T2), `$BROWSER` ✓ (T3), install-on-create + on-browse ✓ (T2/T4), `work browse` loop ✓ (T4), host opener ✓ (T2), CLI wiring ✓ (T5), isolation untouched ✓ (no task touches network/mount/port; `work doctor` run in T7), docs ✓ (T6).
- Type/signature consistency: `browse(name: &str) -> Result<()>` matches `forward(name, port)`; `exec_root` signature identical in trait + impl; `install_shim`/`ensure_fifo(engine: &dyn Engine, ctr: &str)` match call sites.
- The `run_opts` test is updated in the same task that changes `run_opts` (T3) — no stale test.
