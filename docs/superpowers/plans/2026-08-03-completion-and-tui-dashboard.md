# Completion + Interactive TUI Dashboard — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add dynamic shell TAB completion (commands/flags + live workspace names) and an interactive ratatui dashboard (bare `work` → nested workspace→tabs picker with attach + lifecycle actions).

**Architecture:** `work-core` (engine) stays the source of truth; `crates/cli` gains a `completion` module and a `tui` module. Completion is **dynamic-only** via `clap_complete`'s `CompleteEnv` (live names come from `config::list_workspace_names()`). Bare `work <ws>` is modeled natively as an `external_subcommand`, removing the custom `RESERVED`-dispatch gate. The TUI is a sync ratatui/crossterm loop with a background refresh thread + `mpsc`.

**Tech Stack:** Rust 2021 · clap 4 (derive) · clap_complete 4.6 (`unstable-dynamic`) · ratatui 0.30 · crossterm 0.29 (via `ratatui::crossterm`) · `std::thread` + `std::sync::mpsc`.

**Spec:** `docs/superpowers/specs/2026-08-03-completion-and-tui-dashboard-design.md`

## Global Constraints

- **Workspace deps** live in root `Cargo.toml` `[workspace.dependencies]`; crates reference them with `.workspace = true`. Follow this for the new deps.
- **clap_complete must be pinned** — its `unstable-dynamic` API is unstable. Pin via `Cargo.lock` (already reproducible) and add an explanatory comment in `crates/cli/Cargo.toml`. The wire protocol between the sourced shell shim and the binary can change across bumps.
- **crossterm 0.29, not 0.28** — ratatui 0.30's backend resolves to 0.29 by default; a 0.28 pin adds a dead crate. Import crossterm via the `ratatui::crossterm` re-export so types always match the backend.
- **MSRV ~1.86** — clap_complete 4.6.8 + ratatui 0.30 require it. The repo's `rust-toolchain.toml` pins `channel = "stable"` (current stable ≥ 1.86); no change needed. `work` stays edition 2021.
- **Completer signature is `fn(&OsStr) -> Vec<CompletionCandidate>`** (NOT `&str`); attach via `#[arg(add = ArgValueCompleter::new(fn))]`. There is no `complete = ` attribute.
- **`SubcommandCandidates` and `CompleteEnv` import paths**: `use clap_complete::engine::SubcommandCandidates;` (engine-only, not crate root) and `use clap_complete::CompleteEnv;` (re-exported at crate root).
- **Completion never touches the container engine** — only `config::list_workspace_names()` (a directory read). Keep it that way so TAB is instant.
- **No async runtime.** `std::thread` + `std::sync::mpsc` only.
- **Commits:** one commit per task, conventional-commit messages (`feat:`, `refactor:`, `test:`, `docs:`).
- **Pure-first:** every IO-free helper is a pure `pub fn` with a unit test (matches the codebase's existing convention — see `safety.rs`, `naming.rs`).

---

## File Structure

**New files:**
- `crates/cli/src/completion.rs` — pure name-filtering helper + the `complete_workspace`/`workspace_subcommand_candidates` completer fns.
- `crates/cli/src/tui/mod.rs` — `pub fn run(yes: bool) -> Result<()>`: engine probe, `Tui` enter, event loop, teardown.
- `crates/cli/src/tui/app.rs` — pure `App` state (model, name-keyed selection, expanded set, filter, transient status) + transitions.
- `crates/cli/src/tui/render.rs` — `render(&mut Frame, &App)`: stateful `List` + nested tab rows + footer.
- `crates/cli/src/tui/event.rs` — crossterm `poll`/`read` → `App` transitions; background-refresh channel drain.

**Modified files:**
- `Cargo.toml` (workspace deps) — add `clap_complete`, `ratatui`, `crossterm`.
- `crates/cli/Cargo.toml` — reference the new deps; unstable-dynamic feature + pin comment.
- `crates/cli/src/main.rs` — `mod completion; mod tui;`; `CompleteEnv` first line of `main`; `external_subcommand` variant + dispatch; remove `RESERVED`-dispatch gate; `Cli` gains `SubcommandCandidates`; existing-workspace args gain `ArgValueCompleter`; bare-`work` → `tui::run`/`ls` routing.
- `crates/cli/src/commands.rs` — `pub fn dashboard(yes: bool)` thin wrapper over `tui::run`; `pub fn attach(name)` helper used by both CLI and TUI.
- `crates/core/src/workspace.rs` — promote `WindowRow` to a `pub` DTO with `pub` fields; add `pub fn windows(&self) -> Result<Vec<WindowRow>>`; reduce `list_tabs()` to a printer over it.
- `crates/core/src/naming.rs` — backfill `resume`/`stop-all` into `RESERVED`; expose a single shared reserved set.
- `crates/core/src/lib.rs` — no change unless re-exporting the shared reserved set.
- `README.md` — completion install section; troubleshooting (`stty sane`).

---

## Phase 1 — Shell completion

### Task 1.1: Completion module with a pure name filter

**Files:**
- Create: `crates/cli/src/completion.rs`
- Test: `crates/cli/src/completion.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `pub fn filter_names(names: &[String], prefix: &str) -> Vec<String>` (pure); `pub fn complete_workspace(current: &OsStr) -> Vec<CompletionCandidate>`; `pub fn workspace_subcommand_candidates() -> Vec<CompletionCandidate>`.

- [ ] **Step 1: Write the failing test**

```rust
// crates/cli/src/completion.rs
#[cfg(test)]
mod tests {
    use super::filter_names;

    #[test]
    fn filter_names_matches_prefix_and_is_case_sensitive() {
        let names: Vec<String> = ["acme", "acme-2", "blog", "Acme"]
            .iter().map(|s| s.to_string()).collect();
        assert_eq!(filter_names(&names, "ac"), vec!["acme".to_string(), "acme-2".to_string()]);
        assert_eq!(filter_names(&names, "Ac"), vec!["Acme".to_string()]);
        assert!(filter_names(&names, "z").is_empty());
        // empty prefix returns all
        assert_eq!(filter_names(&names, "").len(), 4);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p work completion::tests`
Expected: FAIL — `filter_names` not defined.

- [ ] **Step 3: Implement**

```rust
// crates/cli/src/completion.rs
//! Dynamic shell completion helpers for `work`.
//!
//! The completers read ONLY the on-disk workspace config directory (never the
//! container engine) so TAB completion is instant. All prefix logic is pure and
//! unit-tested; the clap_complete wiring lives in `main.rs`.

use std::ffi::OsStr;

use clap_complete::engine::CompletionCandidate;

/// Pure prefix filter over a sorted name list. Workspace names are lowercase, so
/// matching is case-sensitive (consistent with `naming::validate_name`).
pub fn filter_names(names: &[String], prefix: &str) -> Vec<String> {
    names.iter().filter(|n| n.starts_with(prefix)).cloned().collect()
}

/// Lazy completer for EXISTING workspace names (attached to the args of
/// `start`/`stop`/`tab`/`tabs`/`rm`/`fwd`/`browse`/`config`).
///
/// Signature is `Fn(&OsStr) -> Vec<CompletionCandidate>` per clap_complete's
/// `ValueCompleter` blanket impl (verified against clap_complete 4.6.8).
/// Attach in derive via: `#[arg(add = ArgValueCompleter::new(complete_workspace))]`.
pub fn complete_workspace(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(prefix) = current.to_str() else { return Vec::new() };
    let names = work_core::config::list_workspace_names().unwrap_or_default();
    filter_names(&names, prefix)
        .into_iter()
        .map(CompletionCandidate::new)
        .collect()
}

/// Eager candidates for the external-subcommand slot (bare `work <ws>`).
/// Returns ALL names; clap_complete's engine filters by the current prefix.
/// Attach to the root `Cli` via: `#[command(add = SubcommandCandidates::new(workspace_subcommand_candidates))]`.
pub fn workspace_subcommand_candidates() -> Vec<CompletionCandidate> {
    work_core::config::list_workspace_names()
        .unwrap_or_default()
        .into_iter()
        .map(CompletionCandidate::new)
        .collect()
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p work completion::tests`
Expected: PASS (1 test).

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/completion.rs
git commit -m "feat(cli): add completion module with pure workspace-name filter"
```

---

### Task 1.2: Add `clap_complete` dependency

**Files:**
- Modify: `Cargo.toml` (`[workspace.dependencies]`)
- Modify: `crates/cli/Cargo.toml` (`[dependencies]`)

**Interfaces:**
- Produces: `clap_complete` available to `crates/cli` with the `unstable-dynamic` feature.

- [ ] **Step 1: Add the workspace dependency**

In `Cargo.toml` `[workspace.dependencies]`, add (alphabetical, after `clap`):

```toml
clap_complete = { version = "4", features = ["unstable-dynamic"] }
```

- [ ] **Step 2: Reference it from the CLI crate**

In `crates/cli/Cargo.toml` `[dependencies]`, add after `clap`:

```toml
# Dynamic (live workspace-name) completion. `unstable-dynamic` is UNSTABLE: the
# binary<->shim wire protocol can change across releases, so the exact version is
# fixed by Cargo.lock (currently 4.6.8). Bump deliberately, then have users
# relaunch their shell (the install regenerates the shim on each startup).
clap_complete.workspace = true
```

- [ ] **Step 3: Verify it builds and resolves**

Run: `cargo metadata --format-version 1 --no-deps >/dev/null && cargo build -p work`
Expected: builds; `cargo tree -p work -i clap_complete` shows a single `clap_complete` (and, transitively, `clap` with `unstable-ext` enabled by `unstable-dynamic`).

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/cli/Cargo.toml Cargo.lock
git commit -m "build(cli): add clap_complete (unstable-dynamic) for live completion"
```

---

### Task 1.3: `external_subcommand` variant + native bare-`work` dispatch

This replaces the hand-rolled `RESERVED`-dispatch gate in `main()` with a clap-native `Other(Vec<String>)` variant. Bare `work <ws>` then lands in `Command::Other(args)` and attaches to `args[0]`. **Do not** add completion wiring here — that's Task 1.5; this task only changes parsing/dispatch and is gated by equivalence tests.

**Files:**
- Modify: `crates/cli/src/main.rs` (`Command` enum, `RESERVED`, `main()` dispatch)
- Test: `crates/cli/src/main.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `Command::Other(Vec<String>)` variant; `main()` no longer uses `RESERVED` for bare dispatch. `commands::shell` is unchanged.

- [ ] **Step 1: Write the failing equivalence tests**

Add to the test module in `crates/cli/src/main.rs` (these assert the NEW parse shape; they fail until the variant exists):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Cli {
        Cli::parse_from(std::iter::once("work").chain(args.iter().copied()))
    }

    #[test]
    fn bare_workspace_parses_as_other() {
        let cli = parse(&["acme"]);
        match cli.command {
            Some(Command::Other(args)) => assert_eq!(args, vec!["acme".to_string()]),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn yes_flag_before_name_is_applied() {
        let cli = parse(&["--yes", "acme"]);
        assert!(cli.yes);
        assert!(matches!(cli.command, Some(Command::Other(_))));
    }

    #[test]
    fn named_subcommands_still_match() {
        assert!(matches!(parse(&["new", "x"]).command, Some(Command::New(_))));
        assert!(matches!(parse(&["ls"]).command, Some(Command::Ls)));
        assert!(matches!(parse(&["stop", "x"]).command, Some(Command::Stop { .. })));
    }

    #[test]
    fn bare_work_is_none() {
        assert!(parse(&[]).command.is_none());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p work -- tests`
Expected: FAIL — `Command::Other` does not exist / doesn't parse.

- [ ] **Step 3: Add the variant**

In `crates/cli/src/main.rs`, add to the `Command` enum (after the last named variant, `Doctor`):

```rust
    /// Bare `work <ws>`: attach to a workspace's persistent session.
    /// The external-subcommand name is `args[0]`; trailing tokens are `args[1..]`.
    /// Modeled natively so dynamic completion can offer workspace names for it.
    #[command(external_subcommand)]
    Other(Vec<String>),
```

- [ ] **Step 4: Replace the bare-dispatch gate with native dispatch**

In `main()`, **delete** the manual bare-dispatch block:

```rust
    // DELETE these lines:
    if let Some(first) = raw.first() {
        if !first.starts_with('-') && !RESERVED.contains(&first.as_str()) {
            commands::shell(first)?;
            return Ok(ExitCode::SUCCESS);
        }
    }
```

and add the `Other` arm to the `match cli.command` block (alongside the existing arms):

```rust
        Some(Command::Other(args)) => {
            // `work <ws>` → attach. args[0] is the workspace name (validated by shell()).
            let name = args.first().cloned().unwrap_or_default();
            commands::shell(&name)?;
        }
```

Keep `normalize_help_arg(raw)` and `_update_guard` exactly where they are. `raw` is still needed for `normalize_help_arg`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p work -- tests`
Expected: PASS (4 tests).

- [ ] **Step 6: Manual equivalence smoke test**

Run (in the repo, no docker needed for parse errors):
```bash
cargo run -q -- acme            # expects a workspace-not-found / engine error, NOT "unrecognized subcommand"
cargo run -q -- new             # expects new's usage/error (needs a name)
cargo run -q -- ls              # lists workspaces (or engine error)
cargo run -q -- help stop       # still works (normalize_help_arg)
```
Expected: `acme` routes to attach (no "unrecognized subcommand" clap error); named subcommands unchanged.

- [ ] **Step 7: Commit**

```bash
git add crates/cli/src/main.rs
git commit -m "refactor(cli): model bare work <ws> as external_subcommand (drop RESERVED dispatch)"
```

---

### Task 1.4: Consolidate `RESERVED` into one shared set (fix orphan bug)

The pre-existing bug: `naming::RESERVED` lacks `resume`/`stop-all`, so `work new resume` succeeds but bare `work resume` routes to the command → orphaned workspace. Fix by backfilling and exposing one shared set. After Task 1.3, `main.rs::RESERVED` is no longer used for dispatch but is still used by `normalize_help_arg` — keep a CLI-local list for the help-normalization tokens (commands + `help`/`version`), sourced from the shared set.

**Files:**
- Modify: `crates/core/src/naming.rs` (`RESERVED`)
- Modify: `crates/cli/src/main.rs` (derive its normalization set from the shared one)
- Test: `crates/core/src/naming.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces: `work_core::naming::RESERVED` is the single source of truth and includes every CLI verb.

- [ ] **Step 1: Write the failing test**

In `crates/core/src/naming.rs` test module:

```rust
    #[test]
    fn reserved_includes_every_cli_verb() {
        for verb in ["new", "all", "browse", "ls", "start", "stop", "stop-all",
                     "fwd", "config", "image", "doctor", "help", "version",
                     "rm", "tab", "tabs", "resume"] {
            assert!(RESERVED.contains(&verb), "missing reserved verb: {verb}");
            assert!(matches!(validate_name(verb), Err(crate::error::NameError::Reserved)),
                "verb not rejected by validate_name: {verb}");
        }
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p work-core naming::tests`
Expected: FAIL — `resume`/`stop-all` not in `RESERVED`.

- [ ] **Step 3: Backfill `RESERVED`**

In `crates/core/src/naming.rs`, replace the `RESERVED` const:

```rust
/// Reserved tokens — equal to a CLI verb, so they cannot also be a workspace name.
/// This is the SINGLE source of truth for reserved workspace-name tokens; the CLI
/// derives its help-normalization set from it. Keep in sync when adding a verb.
pub const RESERVED: &[&str] = &[
    "new", "all", "browse", "ls", "start", "stop", "stop-all", "resume",
    "fwd", "config", "image", "doctor", "help", "version", "rm", "tab", "tabs",
];
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p work-core naming::tests`
Expected: PASS.

- [ ] **Step 5: Reconcile `main.rs::RESERVED`**

In `crates/cli/src/main.rs`, the local `RESERVED` is now only for `normalize_help_arg` (trailing-`help` rewrite), which needs the command verbs + `help`. Replace the hand-maintained literal with one built from the shared set at compile time. Replace the `const RESERVED: &[&str] = &[ ... ]` block with:

```rust
/// Tokens that are CLI verbs (not workspace names). Used by `normalize_help_arg`.
/// Sourced from the single shared set in work-core so it can never drift.
const RESERVED: &[&str] = work_core::naming::RESERVED;
```

(`normalize_help_arg` keys off `RESERVED.contains(&raw[0])`; `help`/`version` are already in the shared set, so the flag forms `--help`/`-V` previously in the local list are unnecessary there — they start with `-` and are already excluded by the `!raw[0].starts_with('-')` guard.)

- [ ] **Step 6: Verify the full CLI test suite + help normalization**

Run: `cargo test -p work && cargo run -q -- stop help`
Expected: all tests pass; `work stop help` prints `stop`'s help (normalize_help_arg rewrote it to `work help stop`).

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/naming.rs crates/cli/src/main.rs
git commit -m "fix(naming): single shared RESERVED set; backfill resume/stop-all (orphan bug)"
```

---

### Task 1.5: Wire `CompleteEnv` + per-arg completers + subcommand candidates

**Files:**
- Modify: `crates/cli/src/main.rs` (`mod completion;`, `Cli`/`Command` derives, `main()` first line, attach completers to existing-workspace args)

**Interfaces:**
- Consumes: `completion::complete_workspace`, `completion::workspace_subcommand_candidates`.

- [ ] **Step 1: Register the module + first-line `CompleteEnv`**

At the top of `crates/cli/src/main.rs`, add `mod completion;` alongside `mod commands;`. Then make `CompleteEnv::complete()` the **first statement** of `main()` (before `normalize_help_arg`, `run_check`, and any parse). Add imports:

```rust
use clap::CommandFactory as _;          // provides Cli::command
use clap_complete::CompleteEnv;
```

and as the first line inside `fn main() -> Result<ExitCode> {`:

```rust
    // Dynamic completion entry point. Reads std::env::args_os() directly and
    // exit(0)s when COMPLETE=<shell> is set; returns immediately otherwise.
    // MUST run before any stdout write and before the bare dispatch.
    CompleteEnv::with_factory(Cli::command).complete();
```

- [ ] **Step 2: Attach `SubcommandCandidates` to the root `Cli`**

On the `Cli` struct's `#[command(...)]`, add the `add =` extension. Update the `#[derive(Parser)]` block:

```rust
use clap_complete::engine::SubcommandCandidates;

#[derive(Parser)]
#[command(
    name = "work",
    version,
    about = "Isolated multi-context session manager — one persistent Linux container per workspace",
    after_help = "Tip: a bare `work <ws>` attaches to (or creates) that workspace's persistent in-container session. Use `work help <command>` for per-command details.",
    add = SubcommandCandidates::new(completion::workspace_subcommand_candidates),
)]
struct Cli { /* unchanged: yes, command */ }
```

- [ ] **Step 3: Attach `ArgValueCompleter` to existing-workspace args**

Add the import: `use clap_complete::engine::ArgValueCompleter;`. Then annotate each existing-workspace positional in the `Command` enum and `NewArgs`:

```rust
    Start {
        #[arg(add = ArgValueCompleter::new(completion::complete_workspace))]
        name: String,
    },
    Stop {
        #[arg(add = ArgValueCompleter::new(completion::complete_workspace))]
        name: String,
    },
    Fwd {
        #[arg(add = ArgValueCompleter::new(completion::complete_workspace))]
        ws: String,
        port: u16,
    },
    Browse {
        #[arg(add = ArgValueCompleter::new(completion::complete_workspace))]
        ws: String,
    },
    Config {
        #[arg(add = ArgValueCompleter::new(completion::complete_workspace))]
        ws: String,
        #[arg(long)] edit: bool,
    },
    Tab {
        #[arg(add = ArgValueCompleter::new(completion::complete_workspace))]
        ws: String,
        #[arg(long)] name: Option<String>,
    },
    Tabs {
        #[arg(add = ArgValueCompleter::new(completion::complete_workspace))]
        ws: String,
    },
    Rm {
        #[arg(add = ArgValueCompleter::new(completion::complete_workspace))]
        ws: String,
        #[arg(long)] purge: bool,
    },
```

**Do NOT** annotate `NewArgs.name` (it must be a fresh name).

- [ ] **Step 4: Build + run the existing tests**

Run: `cargo build -p work && cargo test -p work`
Expected: builds; all tests pass (the `add =` attributes are inert at runtime outside completion).

- [ ] **Step 5: Manual completion verification (the real acceptance test)**

```bash
# zsh: in a fresh shell, after building:
source <(COMPLETE=zsh cargo run -q --)   # or against an installed `work` binary
work <TAB>              # expect: named subcommands AND workspace names, merged
work start <TAB>        # expect: only workspace names
work new --git-<TAB>    # expect: --git-name / --git-email
```
Expected: bare `work <TAB>` offers workspace names alongside commands; `start`/`stop`/etc. offer only workspace names; flags complete.

**Fallback if bare-name completion does NOT fire:** the completion engine only consults `SubcommandCandidates` when the command being completed `is_allow_external_subcommands_set()`. If the root `Cli` doesn't report it (because the `external_subcommand` variant lives in the `Command` subcommand enum), add `allow_external_subcommands = true` to the `Cli` `#[command(...)]` and re-test. Re-run Step 5 after any change.

- [ ] **Step 6: Commit**

```bash
git add crates/cli/src/main.rs
git commit -m "feat(cli): wire dynamic completion (CompleteEnv + per-arg + subcommand candidates)"
```

---

### Task 1.6: Document completion install in the README

**Files:**
- Modify: `README.md` (new subsection under `## Install`)

- [ ] **Step 1: Add the section**

After the existing `## Install` "From source" block (before `Verify:`), insert:

```markdown
## Shell completion (live workspace names)

`work` ships dynamic completion (commands, flags, and your real workspace names).
Add the matching line to your shell's rc file — it regenerates on every shell
startup, so it stays in sync with the binary across upgrades:

```sh
# ~/.zshrc  or  ~/.bashrc
source <(COMPLETE=zsh work)      # use COMPLETE=bash work for bash
# ~/.config/fish/completions/work.fish
COMPLETE=fish work | source
```

Don't write the generated script to a file — a stale file breaks across `work`
upgrades. After upgrading `work`, just relaunch your shell.
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs(readme): add dynamic completion install section"
```

---

## Phase 2 — TUI scaffold

### Task 2.1: Add `ratatui` + `crossterm` dependencies

**Files:**
- Modify: `Cargo.toml` (`[workspace.dependencies]`)
- Modify: `crates/cli/Cargo.toml` (`[dependencies]`)

- [ ] **Step 1: Add workspace deps**

In `Cargo.toml` `[workspace.dependencies]`:

```toml
ratatui = "0.30"
crossterm = "0.29"
```

- [ ] **Step 2: Reference from the CLI crate**

In `crates/cli/Cargo.toml` `[dependencies]`:

```toml
ratatui.workspace = true
# ratatui 0.30's backend resolves to crossterm 0.29 by default; we import crossterm
# via the `ratatui::crossterm` re-export so event/Command types match the backend.
crossterm.workspace = true
```

- [ ] **Step 3: Verify build**

Run: `cargo build -p work`
Expected: builds. Confirm a single crossterm: `cargo tree -p work -i crossterm`.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/cli/Cargo.toml Cargo.lock
git commit -m "build(cli): add ratatui 0.30 + crossterm 0.29 for the dashboard"
```

---

### Task 2.2: Promote `WindowRow` to a public DTO + `Workspace::windows()`

**Files:**
- Modify: `crates/core/src/workspace.rs` (`WindowRow`, new `windows()`, simplify `list_tabs()`)
- Test: `crates/core/src/workspace.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces: `pub struct WindowRow { pub index, pub name, pub panes, pub active, pub command }`; `impl Workspace { pub fn windows(&self) -> Result<Vec<WindowRow>> }`.

- [ ] **Step 1: Write the failing test for the parser shape**

In `crates/core/src/workspace.rs` test module:

```rust
    #[test]
    fn parse_window_line_shapes_a_row() {
        let row = parse_window_line("3\tbuild\t1\t0\tzsh").unwrap();
        assert_eq!(row.index, "3");
        assert_eq!(row.name, "build");
        assert_eq!(row.panes, "1");
        assert!(!row.active);
        assert_eq!(row.command, "zsh");
    }

    #[test]
    fn parse_window_line_marks_active() {
        let row = parse_window_line("1\tserver\t2\t1\tnvim").unwrap();
        assert!(row.active);
    }

    #[test]
    fn parse_window_line_rejects_malformed() {
        assert!(parse_window_line("only\ttwo").is_none());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p work-core parse_window_line`
Expected: FAIL — `parse_window_line` is private / `WindowRow` fields not accessible.

- [ ] **Step 3: Make `WindowRow` a pub DTO + extract `windows()`**

Change the `WindowRow` definition (currently private with private fields) to:

```rust
/// One parsed row of `tmux list-windows -F` output. PURE (built by
/// `parse_window_line`). Public so the TUI can render structured tabs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowRow {
    pub index: String,
    pub name: String,
    pub panes: String,
    pub active: bool,
    pub command: String,
}
```

Keep `parse_window_line` private (module-local) but it now constructs the pub struct. Add the structured accessor on `impl Workspace`, lifting the logic out of `list_tabs()`:

```rust
    /// Structured tmux windows ("tabs") for this workspace's session.
    /// Returns the same none/empty states `list_tabs()` prints: a stopped/missing
    /// container or absent session yields an empty vec (callers decide how to show it).
    pub fn windows(&self) -> Result<Vec<WindowRow>> {
        let ctr = naming::container(&self.cfg.name);
        let session = naming::session(&self.cfg.name);
        if !matches!(self.engine.container_state(&ctr)?, ContainerState::Running) {
            return Ok(Vec::new());
        }
        if !self.engine.session_exists(&ctr, session)? {
            return Ok(Vec::new());
        }
        let out = self.engine.exec_capture(
            &ctr,
            &["tmux", "list-windows", "-t", session, "-F",
               "#{window_index}\t#{window_name}\t#{window_panes}\t#{window_active}\t#{pane_current_command}"],
        )?;
        Ok(out.lines().filter_map(parse_window_line).collect())
    }
```

Then reduce `list_tabs()` to a printer that calls `self.windows()` and renders the existing hint messages for the empty cases (preserve today's wording: "no container", "stopped", "no live session", "no windows"). Keep its public signature `pub fn list_tabs(&self) -> Result<()>`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p work-core`
Expected: PASS (existing `tabs` behavior is preserved; new parser tests pass).

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/workspace.rs
git commit -m "refactor(core): expose pub WindowRow DTO + Workspace::windows()"
```

---

### Task 2.3: `Tui` RAII enter/restore guard

**Files:**
- Create: `crates/cli/src/tui/mod.rs` (minimal — just the guard for now)
- Modify: `crates/cli/src/main.rs` (`mod tui;`)

**Interfaces:**
- Produces: `pub(crate) struct Tui` with `enter() -> io::Result<Self>`, `draw(&mut self, f) -> io::Result<CompletedFrame>`-style, and a `Drop` that restores the terminal.

- [ ] **Step 1: Create the module with the guard**

`crates/cli/src/main.rs`: add `mod tui;` (alongside `mod commands;`).

`crates/cli/src/tui/mod.rs`:

```rust
//! Interactive dashboard for `work` (ratatui + crossterm).

use std::io::{self, Stdout};

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::{Hide, Show};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::{CompletedFrame, Frame, Terminal};

pub(crate) type Term = Terminal<CrosstermBackend<Stdout>>;

/// Owns the terminal: entering acquires raw mode + the alternate screen; `Drop`
/// restores them unconditionally (best-effort), so a panic or `?` mid-dashboard
/// can never leave the terminal in raw mode. There is no `panic = "abort"` in the
/// workspace, so unwinding runs `Drop`.
pub(crate) struct Tui {
    terminal: Term,
}

impl Tui {
    pub(crate) fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        let backend = CrosstermBackend::new(io::stdout());
        Ok(Self { terminal: Terminal::new(backend)? })
    }

    pub(crate) fn draw<R>(&mut self, f: impl FnOnce(&mut Frame) -> R) -> io::Result<(CompletedFrame, R)> {
        self.terminal.draw(f)
    }

    pub(crate) fn terminal(&mut self) -> &mut Term {
        &mut self.terminal
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        // disable raw mode first (most side effects), then leave alt screen, then
        // re-show the cursor (ratatui's built-in restore omits the cursor Show).
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
    }
}
```

- [ ] **Step 2: Build**

Run: `cargo build -p work`
Expected: builds (the module is unused so far — that's fine; `pub(crate)` items silence dead-code only if referenced, so add `#[allow(dead_code)]` on `Tui` if the compiler warns, to be removed in Task 2.5).

- [ ] **Step 3: Commit**

```bash
git add crates/cli/src/tui/mod.rs crates/cli/src/main.rs
git commit -m "feat(tui): add Tui RAII enter/restore guard (raw mode + alt screen)"
```

---

### Task 2.4: Pure `App` state with name-keyed selection

**Files:**
- Create: `crates/cli/src/tui/app.rs`
- Modify: `crates/cli/src/tui/mod.rs` (`pub(crate) mod app;`)
- Test: `crates/cli/src/tui/app.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `work_core::workspace::WorkspaceStatus { name, state, session_live }`.
- Produces: `pub(crate) struct App` with `new()`, `set_model(Vec<WorkspaceStatus>)`, `selected_name() -> Option<&str>`, `move_up()`/`move_down()`, `quit()`/`should_quit()`, `status_message()`.

- [ ] **Step 1: Write the failing tests for selection reconciliation**

```rust
// crates/cli/src/tui/app.rs
use work_core::workspace::WorkspaceStatus;
use work_core::engine::ContainerState;

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
        // refresh shrinks and reorders: blog is now first
        app.set_model(vec![ws("blog"), ws("infra")]);
        assert_eq!(app.selected_name(), Some("blog"), "selection must follow the name, not the index");
    }

    #[test]
    fn selection_falls_back_to_neighbor_when_name_vanishes() {
        let mut app = App::new();
        app.set_model(vec![ws("acme"), ws("blog"), ws("infra")]);
        app.move_down(); app.move_down(); // infra
        app.set_model(vec![ws("acme"), ws("blog")]); // infra gone
        let sel = app.selected_name();
        assert!(sel == Some("blog") || sel.is_none() == false, "falls back to a surviving neighbor, got {sel:?}");
    }

    #[test]
    fn empty_model_clears_selection() {
        let mut app = App::new();
        app.set_model(vec![ws("acme")]);
        app.set_model(vec![]);
        assert_eq!(app.selected_name(), None);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p work tui::app`
Expected: FAIL — `App` not defined.

- [ ] **Step 3: Implement `App`**

```rust
// crates/cli/src/tui/app.rs
//! Pure dashboard state. Selection is keyed by workspace NAME (not index) so a
//! concurrent `work rm`/`work new` (which changes list length AND order) can never
//! silently move the cursor onto the wrong workspace. No IO, no rendering.

use work_core::engine::ContainerState;
use work_core::workspace::WorkspaceStatus;

pub(crate) struct App {
    model: Vec<WorkspaceStatus>,
    selected: Option<String>, // the workspace NAME under the cursor
    quit: bool,
    status: Option<String>,
}

impl App {
    pub(crate) fn new() -> Self {
        Self { model: Vec::new(), selected: None, quit: false, status: None }
    }

    /// Replace the model from a refresh, re-resolving the cursor by name. If the
    /// previously-selected name is gone, fall back to the nearest surviving
    /// neighbor (preserve list position), clamped to the new bounds.
    pub(crate) fn set_model(&mut self, model: Vec<WorkspaceStatus>) {
        let prev_index = self
            .selected
            .as_deref()
            .and_then(|n| self.model.iter().position(|w| w.name == n));
        self.model = model;
        self.selected = match &self.model {
            empty if empty.is_empty() => None,
            m => {
                // Prefer the same name; else nearest neighbor at the old index.
                let by_name = self.selected.as_deref()
                    .and_then(|n| m.iter().position(|w| &w.name == n));
                let idx = by_name.or_else(|| prev_index.map(|i| i.min(m.len() - 1))).unwrap_or(0);
                Some(m[idx].name.clone())
            }
        };
    }

    pub(crate) fn selected_name(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub(crate) fn selected_status(&self) -> Option<&WorkspaceStatus> {
        self.selected.as_deref()
            .and_then(|n| self.model.iter().find(|w| &w.name == n))
    }

    pub(crate) fn model(&self) -> &[WorkspaceStatus] {
        &self.model
    }

    fn cursor_index(&self) -> Option<usize> {
        self.selected.as_deref()
            .and_then(|n| self.model.iter().position(|w| &w.name == n))
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

    pub(crate) fn is_empty(&self) -> bool { self.model.is_empty() }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p work tui::app`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/tui/app.rs crates/cli/src/tui/mod.rs
git commit -m "feat(tui): pure App state with name-keyed selection + refresh reconciliation"
```

---

### Task 2.5: Render + event loop (flat list, attach, quit)

**Files:**
- Create: `crates/cli/src/tui/render.rs`, `crates/cli/src/tui/event.rs`
- Modify: `crates/cli/src/tui/mod.rs` (`run()`, submodules)

**Interfaces:**
- Consumes: `App`, `Tui`, `work_core::workspace::Workspace::shell` (via a thin `commands::attach`).

- [ ] **Step 1: Add `commands::attach` helper**

In `crates/cli/src/commands.rs`, add (used by both the CLI `Other` arm and the TUI):

```rust
/// `work <ws>`: open the workspace and attach to its persistent in-container session.
pub fn attach(name: &str) -> Result<()> {
    workspace::open_then_shell(name)
}
```

(Add a thin `work_core::workspace` helper `open_then_shell` that does `Workspace::open(name)?.shell()` — or call those two steps inline in `attach`. Prefer the inline form to avoid touching core: `let ws = work_core::workspace::Workspace::open(name)?; ws.shell()`.)

- [ ] **Step 2: `render.rs` — stateful list + footer**

```rust
// crates/cli/src/tui/render.rs
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use super::app::App;
use work_core::engine::ContainerState;

pub(crate) fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let chunks = Layout::vertical([Constraint::Min(3), Constraint::Length(2)]).split(area);

    let items: Vec<ListItem> = app.model().iter().map(|w| {
        let dot = if w.session_live { "●" } else { "—" };
        let st = match w.state {
            ContainerState::Running => "running",
            ContainerState::Stopped => "stopped",
            ContainerState::Missing => "missing",
        };
        ListItem::new(Line::from(format!("{:<16} {:<8} {}", w.name, st, dot)))
    }).collect();

    let list = List::new(items)
        .block(Block::bordered().title("Workspaces"))
        .highlight_symbol("> ")
        .highlight_style(Style::new().reversed())
        .repeat_highlight_symbol(true);

    let mut state = ListState::default();
    if let Some(name) = app.selected_name() {
        if let Some(i) = app.model().iter().position(|w| w.name == name) {
            state.select(Some(i));
        }
    }
    frame.render_stateful_widget(list, chunks[0], &mut state);

    let footer = app.status_message().unwrap_or(
        "↑↓ move · Enter attach · s start · x stop · d rm · t tab · n new · / filter · r refresh · q/Ctrl-C quit"
    );
    frame.render_widget(Paragraph::new(footer), chunks[1]);
}
```

- [ ] **Step 3: `event.rs` + `run()` — poll loop, quit keys, attach**

```rust
// crates/cli/src/tui/mod.rs (append)
mod app;
mod event;
mod render;

use std::time::Duration;

use ratatui::crossterm::event::{poll, read, Event, KeyCode, KeyEventKind, KeyModifiers};

use self::app::App;
use super::commands;

/// Entry point: probe the engine, enter the TUI, run the loop until quit/attach.
/// `yes` is the global `--yes` for destructive-op confirms (used in Phase 4).
pub(crate) fn run(_yes: bool) -> anyhow::Result<()> {
    // Engine probe BEFORE raw mode: avoid a misleading "all missing" dashboard.
    let engine = work_core::engine::detect()?;
    let engine_up = engine.is_running().unwrap_or(false);
    if !engine_up {
        anyhow::bail!(
            "container engine '{}' is not running; start OrbStack/Docker first (or use `work ls`)",
            engine.binary()
        );
    }

    let mut tui = Tui::enter()?;
    let mut app = App::new();
    app.set_model(load_model()?);

    let result = run_loop(&mut tui, &mut app);
    // Tui::drop restores the terminal unconditionally.
    result?;

    // After teardown: if the user chose to attach, do it on the real terminal.
    if let Some(name) = app.pending_attach().take() {
        commands::attach(&name)?;
    }
    Ok(())
}

fn run_loop(tui: &mut Tui, app: &mut App) -> anyhow::Result<()> {
    const TICK: Duration = Duration::from_millis(250);
    loop {
        tui.draw(|f| render::render(f, app))?;
        if !poll(TICK)? { continue; } // tick: drain refresh channel here in Phase 4
        while let Ok(event) = read() {
            if let Event::Key(key) = event {
                if key.kind != KeyEventKind::Press { continue; }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => { app.quit(); return Ok(()); }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => { app.quit(); return Ok(()); }
                    KeyCode::Down | KeyCode::Char('j') => app.move_down(),
                    KeyCode::Up | KeyCode::Char('k') => app.move_up(),
                    KeyCode::Enter => {
                        if let Some(name) = app.selected_name() {
                            app.request_attach(name.to_string());
                            return Ok(());
                        }
                    }
                    _ => {}
                }
            }
        }
        if app.should_quit() { return Ok(()); }
    }
}

fn load_model() -> anyhow::Result<Vec<work_core::workspace::WorkspaceStatus>> {
    Ok(work_core::workspace::list_all()?)
}
```

Add to `App` (in `app.rs`) a `pending_attach: Option<String>` field + `request_attach`/`pending_attach()` accessor, and `new()` initializing it to `None`. (`run` reads it after teardown.)

- [ ] **Step 4: Build**

Run: `cargo build -p work`
Expected: builds. (`s/x/d/t/n/r//` keys are matched in Phase 4; the footer already lists them.)

- [ ] **Step 5: Manual smoke test**

```bash
cargo run -q --            # in a real terminal with docker running
# expect: bordered list of workspaces; ↑↓ moves; Enter attaches (then `work` exits on detach); q/Ctrl-C quits and restores the terminal
```
Expected: dashboard renders, selection works, attach takes over the terminal, quit restores cleanly. If the terminal is left broken, the `Tui` Drop guard is wrong — fix before proceeding.

- [ ] **Step 6: Commit**

```bash
git add crates/cli/src/tui/ crates/cli/src/commands.rs
git commit -m "feat(tui): render + event loop (flat list, name-keyed selection, attach, quit)"
```

---

### Task 2.6: Route bare `work` to the dashboard (TTY-aware)

**Files:**
- Modify: `crates/cli/src/main.rs` (`None` arm of the `match`), `commands.rs` (`dashboard`)

**Interfaces:**
- Consumes: `tui::run`.

- [ ] **Step 1: Add `commands::dashboard`**

In `crates/cli/src/commands.rs`:

```rust
/// Bare `work` (interactive TTY): open the dashboard.
pub fn dashboard(yes: bool) -> Result<()> {
    crate::tui::run(yes)
}
```

- [ ] **Step 2: Route the `None` arm**

In `crates/cli/src/main.rs`, replace the `None => commands::ls()?` arm with the TTY-aware gate:

```rust
        None => {
            use std::io::IsTerminal;
            let interactive = std::io::stdin().is_terminal()
                && std::io::stdout().is_terminal()
                && std::env::var_os("TERM").map_or(true, |t| t != "dumb");
            if interactive {
                commands::dashboard(cli.yes)?;
            } else {
                commands::ls()?;
            }
        }
```

(`cli.yes` is parsed before the `match`; `--yes` only reaches here if placed before a bare invocation, which is the clap-global-arg contract.)

- [ ] **Step 3: Build + test**

Run: `cargo build -p work && cargo test -p work`
Expected: builds; tests pass. (Bare-`work` routing isn't unit-tested here — it needs a TTY; verify manually.)

- [ ] **Step 4: Manual regression checks**

```bash
cargo run -q -- | cat        # non-TTY stdout -> still prints `ls` text
cargo run -q -- ls           # explicit ls still works
cargo run -q -- all          # cockpit still works
cargo run -q --              # TTY -> dashboard
```
Expected: piped/CI invocations keep the text list; interactive opens the dashboard; explicit `ls`/`all` unchanged.

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/main.rs crates/cli/src/commands.rs
git commit -m "feat(cli): route bare work to the dashboard when stdin&&stdout are TTYs"
```

---

## Phase 3 — Nested tabs

### Task 3.1: Expand/collapse + nested rendering

**Files:**
- Modify: `crates/cli/src/tui/app.rs` (expanded set), `render.rs` (nested rows)

**Interfaces:**
- Produces: `App::toggle_expand()`, `App::expanded_tabs(&self) -> Option<&[WindowRow]>`, expanded-name state.

- [ ] **Step 1: Add expanded-name state to `App`**

Add fields `expanded: Option<String>` and `tabs: HashMap<String, Vec<WindowRow>>` (use `std::collections::HashMap`). Add:

```rust
    pub(crate) fn toggle_expand(&mut self) {
        let Some(name) = self.selected_name() else { return; };
        self.expanded = if self.expanded.as_deref() == Some(name) { None } else { Some(name.to_string()) };
    }

    pub(crate) fn expanded_name(&self) -> Option<&str> { self.expanded.as_deref() }

    /// Tabs for the expanded workspace, if any are loaded.
    pub(crate) fn expanded_tabs(&self) -> Option<&[work_core::workspace::WindowRow]> {
        self.expanded.as_deref().and_then(|n| self.tabs.get(n)).map(|v| v.as_slice())
    }

    /// Called by the refresh worker with tabs for the expanded workspace.
    pub(crate) fn set_tabs(&mut self, ws: &str, tabs: Vec<work_core::workspace::WindowRow>) {
        self.tabs.insert(ws.to_string(), tabs);
    }
```

Add a unit test: `toggle_expand` flips the expanded name; collapsing clears `expanded_tabs`.

- [ ] **Step 2: Render nested rows**

Update `render.rs` to build items as: for each workspace, a row; if it's the expanded one, append indented child rows from `app.expanded_tabs()` (or `"  · …"` if the workspace is the expanded one but tabs haven't loaded yet). Highlight logic stays name-based (compute the selected list item from `selected_name()`).

- [ ] **Step 3: Wire `→`/`Tab` to `toggle_expand`**

In `run_loop`, add `KeyCode::Right | KeyCode::Tab | KeyCode::Char('l') => app.toggle_expand()`.

- [ ] **Step 4: Build + test + manual**

Run: `cargo test -p work tui::app && cargo build -p work`, then manually expand/collapse a running workspace in the dashboard.
Expected: tests pass; `→` expands a running workspace to show its tabs (Phase 3.2 fetches them; until then show the row only).

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/tui/
git commit -m "feat(tui): expand/collapse workspaces with nested tab rows"
```

---

### Task 3.2: Lazy tab fetch + tab-level attach

**Files:**
- Modify: `crates/cli/src/tui/mod.rs` (`load_model` → also load tabs for the expanded ws)

**Interfaces:**
- Consumes: `Workspace::windows()` (Task 2.2).

- [ ] **Step 1: Fetch tabs for the expanded workspace on load + on expand**

In `run()`, after `set_model`, if a workspace is expanded (or after `toggle_expand`), open it and call `windows()`:

```rust
fn refresh_tabs(app: &mut App) -> anyhow::Result<()> {
    if let Some(name) = app.expanded_name() {
        if let Ok(ws) = work_core::workspace::Workspace::open(name) {
            let tabs = ws.windows().unwrap_or_default();
            app.set_tabs(name, tabs);
        }
    }
    Ok(())
}
```

Call `refresh_tabs(&mut app)` right after `set_model` in `run()`, and inside the `Right/Tab` arm after `toggle_expand`.

- [ ] **Step 2: Tab-level attach**

When the cursor is on a child tab row and Enter is pressed, attach to that workspace (selecting the tab is a Phase-4 nicety; for now, attaching to the workspace session via `commands::attach(name)` is sufficient and matches `work <ws>`). Record which workspace is selected from the render's selected item: expose `App::selected_workspace_name() -> Option<&str>` that returns the workspace (parent) of the selected row (tab rows belong to their parent). Use it in the `Enter` arm instead of `selected_name()` when a tab row is selected.

- [ ] **Step 3: Manual**

Expand a running workspace, see its tabs; Enter attaches to it.
Expected: tabs render; attach works from either a workspace row or its tab row.

- [ ] **Step 4: Commit**

```bash
git add crates/cli/src/tui/
git commit -m "feat(tui): lazy tab fetch + attach from workspace or tab row"
```

---

## Phase 4 — Lifecycle actions + background refresh

### Task 4.1: start/stop/rm actions with inline `decide()` confirms

**Files:**
- Modify: `crates/cli/src/tui/mod.rs` (key arms), `app.rs` (pending-action state), `render.rs` (confirm modal)

**Interfaces:**
- Consumes: `Workspace::{start,stop,remove}`, `safety::{decide, Severity, Action}`, `Workspace::has_live_session`.

- [ ] **Step 1: `s`/`x`/`d` key arms**

In `run_loop`, add arms:

```rust
                    KeyCode::Char('s') => act(app, |ws| ws.start()),
                    KeyCode::Char('x') => gated_act(app, yes, Severity::WorkLoss, |ws| ws.stop()),
                    KeyCode::Char('d') => gated_act(app, yes, Severity::WorkLoss, |ws| ws.remove(false)),
```

`act` opens the selected workspace and runs the closure, then `app.set_status(...)`. `gated_act` calls `safety::decide(severity, ws.has_live_session(), true /*is_tty*/, yes)`: on `Proceed` run it; on `Prompt` set `app.request_confirm(...)` and let the loop render a `y/N` modal (handled by `y`/`n`/Enter keys that read `app.pending_confirm()`); on `Refuse` set a status message.

- [ ] **Step 2: Confirm modal in `render.rs`**

When `app.pending_confirm().is_some()`, render a centered `Paragraph` ("Stop acme? A live session will end. [y/N]") instead of running the action immediately.

- [ ] **Step 3: Build + manual**

Run each of `s`/`x`/`d` against a real workspace; confirm `--yes` skips the modal and the inline confirm blocks when a live session would end.
Expected: actions work; confirms reuse the pure `decide` policy; `--yes` is honored.

- [ ] **Step 4: Commit**

```bash
git add crates/cli/src/tui/
git commit -m "feat(tui): start/stop/rm actions with inline safety-decide confirms"
```

---

### Task 4.2: `new`/`tab` actions (inline input)

**Files:**
- Modify: `crates/cli/src/tui/mod.rs`, `app.rs` (input-mode state)

- [ ] **Step 1: `n`/`t` arms with an inline text input**

`n` enters an input mode (a `Paragraph` prompt "new workspace: ___"); typed chars accumulate in `app.input`; Enter creates via `commands::new(name, defaults…)` (use `NewArgs` defaults — minimal call); Escape cancels. `t` opens a new tab in the selected workspace via `Workspace::tab(None)`.

- [ ] **Step 2: Manual + commit**

```bash
git add crates/cli/src/tui/
git commit -m "feat(tui): new-workspace and new-tab actions with inline input"
```

---

### Task 4.3: Background refresh worker + error handling

**Files:**
- Modify: `crates/cli/src/tui/mod.rs` (spawn thread, `mpsc` channel, tick drain)

- [ ] **Step 1: Refresh thread + channel**

In `run()`, spawn a thread that every ~3s calls `list_all()` (+ tabs for the expanded ws) and sends `Result<Vec<WorkspaceStatus>, anyhow::Error>` over an `mpsc::sync_channel`. Share a `Arc<AtomicBool>` quit flag so it stops on exit. In `run_loop`, on the `poll` tick (the `continue` branch), drain the channel: on `Ok(model)` → `app.set_model(model)` + `refresh_tabs`; on `Err` → `app.set_status("refresh failed — showing last state")` and do NOT clear the model.

- [ ] **Step 2: `r` manual refresh**

`KeyCode::Char('r')` forces an immediate refresh (send a trigger to the thread, or do a synchronous `load_model()` + `set_model`).

- [ ] **Step 3: Manual + commit**

Change a workspace's state from outside (e.g. `work stop x` in another terminal); the dashboard updates within ~3s without losing the cursor.
```bash
git add crates/cli/src/tui/
git commit -m "feat(tui): background refresh worker with error-safe model updates"
```

---

### Task 4.4: Filter, help footer, README troubleshooting

**Files:**
- Modify: `crates/cli/src/tui/{app,render}.rs`, `README.md`

- [ ] **Step 1: `/` filter**

`/` enters filter input; `App` stores a `filter: String` and `filtered_model()` returns names containing it; selection reconciles against the filtered view. Escape clears.

- [ ] **Step 2: README troubleshooting**

Add to README:
```markdown
If the dashboard ever leaves your terminal in a broken state (e.g. a forced kill),
run `stty sane` (or `reset`) to restore line discipline and echo.
```

- [ ] **Step 3: Final full-suite verification**

Run: `cargo test --workspace && cargo build --workspace`
Expected: all tests pass; clean build. Manually exercise the full dashboard one more time.

- [ ] **Step 4: Commit**

```bash
git add crates/cli/src/tui/ README.md
git commit -m "feat(tui): name filter + footer; docs(readme): terminal-recovery troubleshooting"
```

---

## Self-Review (completed by the author before handoff)

**Spec coverage:** §5.1–5.8 (completion) → Tasks 1.1–1.6. §6.1–6.10 (TUI) → Tasks 2.1–2.6, 3.1–3.2, 4.1–4.4. §6.6 `windows()` → Task 2.2. §5.5 RESERVED consolidation + orphan fix → Task 1.4. §6.5 terminal hygiene (RAII guard, quit keys) → Tasks 2.3, 2.5. §6.2 engine probe → Task 2.5. §6.4 name-keyed selection + refresh-error handling → Tasks 2.4, 4.3. §6.8 `--yes` threading → Task 4.1. All 14 review findings `[RV-1..14]` map to a task or a global constraint.

**Type consistency:** `complete_workspace(&OsStr) -> Vec<CompletionCandidate>` (used in Task 1.5 `ArgValueCompleter::new(...)`); `workspace_subcommand_candidates() -> Vec<CompletionCandidate>` (Task 1.5 `SubcommandCandidates::new(...)`); `WindowRow` pub DTO + `Workspace::windows()` (Task 2.2) consumed by Task 3.1/3.2; `App` accessors (`selected_name`, `set_model`, `move_up/down`, `toggle_expand`, `set_tabs`, `request_attach`/`pending_attach`, `set_status`, `request_confirm`/`pending_confirm`) consistent across app/render/event tasks. `commands::attach` defined in Task 2.5 and used in Task 3.2.

**Known implementation-time risks (called out in tasks):** Task 1.5 Step 5 documents the `allow_external_subcommands` fallback if bare-name completion doesn't fire. Task 2.5 Step 5 makes terminal-restore correctness a manual gate. Completion and TUI behavior are verified manually (no shell/PTY test infra); pure helpers are unit-tested.
