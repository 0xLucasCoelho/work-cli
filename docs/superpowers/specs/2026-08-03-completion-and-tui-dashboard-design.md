# Design: Shell completion + interactive TUI dashboard

- **Date:** 2026-08-03
- **Status:** Design (awaiting user review) → implementation plan
- **Scope:** Two features for the `work` CLI — (A) shell TAB completion incl. live workspace names, and (B) an interactive ratatui dashboard reached by bare `work`.

> This spec was stress-tested by an adversarial multi-agent review (4 lenses, 14 grounded
> findings survived refute-by-default). Findings that changed the design are cited inline as
> `[RV-<n>]` and reconciled in §9.

---

## 1. Goals

1. **Completion.** TAB completes commands, subcommands, flags, **and live workspace names**
   (e.g. `work start ac<TAB>` → `acme`). The workspace-name completion reads only the config
   directory — never the container engine — so it is instant.
2. **Dashboard.** Bare `work` in an interactive terminal opens a fullscreen ratatui UI: a
   nested **workspace → tmux tabs** list, arrow-key navigation, and attach + lifecycle actions
   (start/stop/rm/new/tab). Non-interactive invocations keep today's `ls()` text output.

## 2. Non-goals

- No new attach mechanism — reuse `Workspace::shell()` / `Workspace::tab()`.
- No "return to dashboard after detach" loop (D4: `work` exits on detach; re-run `work` to reopen).
- No async runtime (no tokio). Sync + `std::thread` + `std::sync::mpsc` only.
- No new crate — the TUI is a module under `crates/cli`. scope/edit/fwd/browse are **not** in the TUI.
- No completion-file generation (see §5.6): the dynamic install is regenerated on each shell startup.

## 3. Background — current state (verified)

- **Binary** `crates/cli` (`work`) over **engine** `crates/core` (`work-core`). clap derive.
  `Command` enum in `main.rs`; bare-name dispatch in `main()` (`main.rs:240-245`).
- **Two reserved-token lists exist and already diverge** `[RV-7]`:
  - `main.rs::RESERVED` (`main.rs:184-206`) drives bare dispatch + `normalize_help_arg`; it
    includes `resume`, `stop-all`, and flag forms (`--help`,`-h`,`--version`,`-V`).
  - `naming::RESERVED` (`naming.rs:4-7`) drives `validate_name`; it **lacks** `resume` and
    `stop-all` → **pre-existing bug**: `work new resume` / `work new stop-all` succeed (pass
    validation) but bare `work resume` / `work stop-all` route to the command, orphaning those
    workspaces.
- **Data sources for the TUI:**
  - `config::list_workspace_names() -> Result<Vec<String>>` — sorted directory read (`config.rs:145`).
  - `workspace::list_all() -> Result<Vec<WorkspaceStatus{name, state, session_live}>>` (`workspace.rs:570`).
    ⚠️ It never calls `engine.is_running()` and coerces any failed `container_state` to
    `Missing` (`workspace.rs:575-577`) → an engine that is **down** renders every workspace as
    `Missing` `[RV-6]`.
  - `Workspace::list_tabs() -> Result<()>` **only prints**; it parses a **private** `WindowRow`
    {`index,name,panes,active,command`} via private `parse_window_line` (`workspace.rs:931-950`) `[RV-14]`.
- **Lifecycle already exists** on `Workspace`: `shell()`, `tab(name)`, `start()`, `stop()`,
  `remove(purge)`, `status()`, `has_live_session()`, `create(...)`. The TUI maps actions 1:1 onto these.
- **Safety policy is pure** (`safety.rs`): `decide(severity:{WorkLoss,DataLoss}, has_live_session,
  is_tty, yes) -> Action{Proceed,Prompt,Refuse}`. The TUI renders the `Prompt` arm as an inline UI.
- `main()` order today: `normalize_help_arg` (`:233`) → `_update_guard = update::run_check()`
  (`:237`; may print a one-line hint to **stderr**, gated on `stderr().is_terminal()`) → bare
  dispatch (`:240-245`) → `Cli::parse_from` (`:247`). `main()` returns `Result<ExitCode>` and
  propagates errors with `?` (`main.rs:229`). No `panic="abort"` profile exists.

---

## 4. Approved decisions (D1–D5) — and review-driven revisions

| ID | Decision | Status |
|----|----------|--------|
| D1 | TUI is a module `crates/cli/src/tui/`, not a new crate. | **Kept.** |
| D2 | Bare `work` → TUI when interactive; else `ls()`. | **Revised `[RV-11]`**: gate is `stdin().is_terminal() && stdout().is_terminal()` (attach runs `docker exec -it`, which needs a TTY stdin); also bail on `TERM=dumb`. |
| D3 | Refresh via background thread + `mpsc`; no tokio. | **Revised `[RV-10]`**: selection keyed by workspace **name** (not index); reconcile on each refresh; a refresh **error** keeps the old model + transient marker rather than blanking. |
| D4 | Enter restores terminal then `Workspace::shell()`; `work` exits on detach. | **Kept.** |
| D5 | `clap_complete` static (`aot`) + dynamic; a `Completions{shell}` subcommand. | **Replaced `[RV-1, RV-3, RV-9, RV-13]`**: `aot` and dynamic are **mutually exclusive in effect** — the static script never calls back, so live names are dead under `eval "$(work completions zsh)"`. Use **dynamic-only** via `CompleteEnv` at the top of `main()`; per-shell `source <(COMPLETE=$SHELL work)` install. No `completions` subcommand. |

---

## 5. Feature A — Shell completion (dynamic-only)

### 5.1 Mechanism decision (corrects D5)

clap_complete has two completion families that **do not compose** `[RV-1]`:

- **`aot`** ("Prebuilt completions"): `aot::generate` emits a self-contained static shell script
  that hardcodes the command tree and **never calls back** into the binary.
- **`engine`/`env`** (gated by `unstable-dynamic`): `CompleteEnv` emits a small shell shim that,
  on each TAB, calls back into the binary (`COMPLETE=$SHELL <bin>`) to compute candidates —
  including runtime values from `ArgValueCandidates`/`ArgValueCompleter` closures.

Only the **dynamic** family can produce live workspace names. The dynamic engine *also* completes
commands, subcommands, and flags, so it is a strict superset of what `aot` offers. We therefore
ship **dynamic-only** and drop the `Completions{shell}` subcommand entirely (it would only mislead
users into a static install that lacks the headline feature).

### 5.2 `main()` wiring `[RV-3]`

`CompleteEnv` must run **before** the bare-dispatch block (`main.rs:240`) and before any stdout
output, because in completion mode it reads `std::env::args_os()` directly and `exit(0)`s; it
returns immediately when `COMPLETE` is unset (normal runs unaffected). Place it as the **first
statement of `main()`**:

```rust
// crates/cli/src/main.rs — top of main()
use clap_complete::env::CompleteEnv;
CompleteEnv::with_factory(|| Cli::command()).complete();   // no-op unless COMPLETE=<shell> set
// …existing normalize_help_arg / run_check / dispatch…
```

Exact closure shape is part of the unstable API; confirm against the pinned version at impl time.

### 5.3 Per-argument completers (existing workspaces only)

Attach an `ArgValueCandidates` completer to every arg that takes an **existing** workspace:

`Start.name`, `Stop.name`, `Tab.ws`, `Tabs.ws`, `Rm.ws`, `Fwd.ws`, `Browse.ws`, `Config.ws`.

The completer is a pure, fast function over the **config directory only** (no docker):

```rust
use std::ffi::OsStr;
use clap_complete::engine::{ArgValueCompleter, CompletionCandidate};

/// Lazy completer for EXISTING workspace names. Reads ONLY the config dir (no docker) → instant.
/// Signature is `Fn(&OsStr) -> Vec<CompletionCandidate>` (NOT `&str`) per the `ValueCompleter`
/// blanket impl (verified against clap_complete 4.6.8 source). Attach in derive via:
///   #[arg(add = ArgValueCompleter::new(complete_workspace))]
pub fn complete_workspace(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(prefix) = current.to_str() else { return Vec::new() };
    work_core::config::list_workspace_names()
        .map(|names| names.into_iter().filter(|n| n.starts_with(prefix)).map(CompletionCandidate::new).collect())
        .unwrap_or_default()
}
```

**Excluded:** `New.name` (must be a **fresh**, non-existing name — completing existing names would
be wrong) and `Fwd.port` (free integer). Image args are left to clap defaults.

### 5.4 Bare-name completion (`work <ws>`)

Bare `work <ws>` has no clap arg today (custom dispatch). The clean, **public** path is clap's
external-subcommand mechanism `[RV-8]`:

- Add `Other(#[command(external_subcommand)] Vec<String>)` to `Command`, and attach
  `SubcommandCandidates::new(workspace_candidates)` to the root `Cli`. clap then merges subcommand
  names + workspace candidates with no hand-rolled `RESERVED`-prefix logic.
- Dispatch `Command::Other(args)` → attach to `args[0]` (same `commands::shell` path as today).

This **removes** the `RESERVED`-as-dispatch gate in `main()` (`main.rs:240-245`) — `RESERVED`
retains its role only in `validate_name` and as the "don't suggest these as workspace names" set.

**Acceptance gate (must not regress):** `work <existing-ws>`, `work new …`, `work stop <ws>`,
`work help <cmd>` / `work <cmd> help` normalization, and `work --yes <ws>` all behave unchanged.
Add tests asserting each. If, during implementation, `external_subcommand` proves too invasive
(e.g. it interferes with global `--yes` parsing or help normalization), fall back to a bespoke
candidate injection inside the `CompleteEnv` path — but `external_subcommand` is the primary plan.

### 5.5 RESERVED consolidation `[RV-2, RV-7]`

- Backfill `resume` and `stop-all` into `naming::RESERVED` (fixes the pre-existing
  orphan-workspace bug).
- **Collapse to a single shared constant** in `work-core` (re-export from `naming`), consumed by
  both `main.rs` (dispatch/normalization) and `validate_name`. The flag forms (`--help`,`-h`,…)
  stay in the CLI-local set since they are a CLI-concern, not a naming concern.
- No new `completions` verb is added (no subcommand) — so no new reserved token is required; the
  collapse is the relevant change here.

### 5.6 Install (per-shell, self-correcting) `[RV-9, RV-13]`

Document in the README `Install` section (no `completions` subcommand, no file write):

```sh
# zsh / bash  -> ~/.zshrc / ~/.bashrc
source <(COMPLETE=zsh work)     # or COMPLETE=bash work
# fish         -> ~/.config/fish/completions/work.fish
COMPLETE=fish work | source
```

The shim is regenerated on **every shell startup**, so it stays in sync with the binary across
upgrades ("self-correcting" per clap_complete's `env` docs). Explicitly advise **against** writing
the generated script to a file (a stale file breaks across any `clap_complete` bump). Nushell is
out of scope (separate `clap_complete_nushell` crate) — note as a future option.

### 5.7 Stability / risk `[RV-13]`

`unstable-dynamic` is unstable: a `clap_complete` bump can change the binary↔shim wire protocol.
Mitigations: (1) **pin `clap_complete`** — `Cargo.lock` already fixes the exact version across
contributors; additionally add a comment in `crates/cli/Cargo.toml` flagging the
`unstable-dynamic` wire-protocol risk so a future bump is intentional (and optionally use an
exact `version = "=4.6.8"` pin in the manifest to make it load-bearing);
(2) keep the workspace `clap` requirement at `"4"` (resolves to ≥4.5.20, which clap_complete
4.6.x itself requires) — don't downgrade;
(3) the self-correcting startup install means a user only needs to relaunch their shell after an
upgrade. Call this out in the README so users aren't surprised.

### 5.8 Testing

- **Unit (pure):** `workspace_candidates(prefix)` filtering — fast, deterministic.
- **Unit (pure):** `validate_name` still rejects every reserved token after the consolidation, and
  the dispatch-preserving refactor's equivalence tests (§5.4 acceptance gate).
- **Integration (optional):** shell-level completion via clap_complete's `completest`/
  `completest-pty` dev-deps — nice-to-have, not required to ship.

---

## 6. Feature B — TUI dashboard

### 6.1 Entry behavior (D2, revised) `[RV-11]`

Bare `work` (the `None` arm, `main.rs:249`) routes as:

```rust
let interactive = std::io::stdin().is_terminal()
    && std::io::stdout().is_terminal()
    && std::env::var_os("TERM").map_or(true, |t| t != "dumb");
if interactive { commands::dashboard()? } else { commands::ls()? }
```

- A fullscreen TUI reads keys from stdin and the attach path runs `docker exec -it` (needs a TTY
  stdin) → gate on **both** stdin and stdout, not stdout alone.
- Non-TTY (pipes, scripts, CI) → unchanged `ls()` text. `work ls` and `work all`/`resume` are
  untouched and remain the explicit text/cockpit entry points.

### 6.2 Engine probe before raw mode `[RV-6]`

Before entering raw mode/alt-screen, resolve the engine and its liveness:

- `engine::detect()` **Err** (no engine binary) → print the human message and fall back to `ls()`
  without ever entering the TUI.
- `engine.is_running() == false` (binary present, daemon down) → render a dedicated **"engine not
  running — start OrbStack/Docker"** state instead of querying containers (which would falsely
  report every workspace as `Missing`). Do **not** add an `is_running()` call into `list_all()`
  itself (keep core read paths lean); the TUI performs this probe at the presentation layer.

### 6.3 Module layout (`crates/cli/src/tui/`)

```
tui/
  mod.rs      — pub fn run(yes: bool) -> Result<()>: probe, enter/restore, event loop
  app.rs      — pure App state (model, selection, expanded set, filter, transient status)
  render.rs   — render(&mut Frame, &App): stateful List + nested tab rows
  event.rs    — crossterm poll/read → App transitions; background-refresh channel drain
  actions.rs  — key → action: attach/start/stop/rm/new/tab (delegates to work-core)
```

`App` holds **pure** state; `render` and the key→action map are pure functions of `App`, so
selection/expand/action transitions are unit-testable without a terminal (optional render smoke
via ratatui `TestBackend`).

### 6.4 Event loop + off-thread refresh (D3, revised) `[RV-10]`

- A worker `std::thread` refreshes on a ~3s tick and on demand. It calls `workspace::list_all()`
  always, and `Workspace::windows()` **only for the expanded workspace** (avoids N×M docker calls).
  Results flow over `std::sync::mpsc` to the render thread.
- The render thread runs the canonical ratatui loop: `terminal.draw`, then `event::poll(timeout)`;
  on no input it drains the refresh channel and redraws.
- **Selection is keyed by workspace name, not index.** On each received refresh: re-resolve the
  selected name's index; if absent, fall back to the nearest surviving neighbor and clamp to
  `len-1`. This prevents a concurrent `work rm`/`work new` from silently shifting the cursor onto
  the wrong workspace (or panicking on OOB).
- **Refresh errors do not blank the list:** a refresh `Err` keeps the previous model and surfaces a
  transient "refresh failed" marker; only a valid result replaces + reconciles the model.
- Worker cancellation: the channel + a shared quit flag ensure the thread stops on `q`/Ctrl-C.

### 6.5 Terminal hygiene `[RV-4, RV-5]`

- **RAII restore guard**, constructed immediately after enabling raw mode / entering the alt
  screen, held for the whole TUI lifetime. Its `Drop` (best-effort, errors ignored) runs
  `disable_raw_mode()`; `execute!(stdout, LeaveAlternateScreen, Show, DisableMouseCapture)`. Any
  `?`/`panic` mid-TUI unwinds through it (no `panic="abort"`, so this is sufficient). Do **not**
  scatter enable/disable across functions.
- **Quit keys are explicit** because crossterm raw mode disables ISIG (Ctrl-C is **not** SIGINT —
  it arrives as `Key{Char('c'), CONTROL}`). Match `Ctrl-C`, `q`, and `Esc` as clean quits that
  return normally so the restore guard fires. State `q/Ctrl-C quit` in the footer.
- For signals that bypass unwinding (SIGTERM/SIGKILL), document `stty sane` / `reset` recovery
  (and optionally a `signal_hook` that restores before re-raising) in the README troubleshooting.

### 6.6 Data source — structured tabs `[RV-14]`

Promote the parsed tab row to a public DTO in `work-core`:

```rust
// crates/core/src/workspace.rs
pub struct WindowRow {
    pub index: String,
    pub name: String,
    pub panes: String,
    pub active: bool,
    pub command: String,
}
impl Workspace {
    /// Structured tmux windows for this workspace's session. None/empty/Ok(vec) states
    /// mirror list_tabs(); used by the TUI and by list_tabs()'s printer.
    pub fn windows(&self) -> Result<Vec<WindowRow>> { /* the existing detect+exec_capture+parse */ }
}
```

Reduce `list_tabs()` to a thin printer over `self.windows()` (behavior unchanged). `parse_window_line`
stays private (module-local). Making the **struct** public is insufficient — the **fields** must be
`pub` for cross-crate reads (`row.name`, `row.active`).

### 6.7 Layout

```
 work
┌─ Workspaces ──────────────────────────────┐
│ ▸ acme      running  ● session live       │   ▸/▾ expand-collapse
│   · build                                 │
│   · server                 (active)       │
│ ▸ blog      stopped                       │
│ ▸ infra     running  — no session         │
└───────────────────────────────────────────┘
 ↑↓ move · →/Tab expand · Enter attach · s start · x stop · d rm · t tab · n new · / filter · r refresh · q/Ctrl-C quit
```

### 6.8 Actions & keys

- **Enter** attach (to the selected tab if a workspace is expanded and a tab is selected, else the
  workspace session): restore terminal → `Workspace::shell()`/`tab()` → `work` exits on detach (D4).
- **→/Tab** expand/collapse a workspace (lazy-fetches tabs for the expanded row via `windows()`).
- **s** start, **x** stop, **d** rm, **t** new tab, **n** new (inline name input; defaults; advanced
  flags via the CLI), **/** filter by name, **r** manual refresh, **q/Ctrl-C/Esc** quit.
- **Destructive ops reuse `safety::decide` with `is_tty=true`** and render the `Prompt` arm inline.
  The global **`--yes` is threaded in** `[RV-12]`: `decide(severity, has_live_session, true, cli.yes)`
  — `--yes` skips the inline confirm exactly as in the CLI. (`cli.yes` is parsed at `main.rs:247`,
  in scope where the dashboard launches.)

### 6.9 Empty / error states

- No workspaces yet (`list_workspace_names()` empty): "No workspaces yet — press `n` to create one".
- Engine down (§6.2): dedicated state, no false `Missing`s.
- A workspace's container stopped/missing: shown inline (state column), attach/start still offered.
- Refresh failure: transient marker, list preserved (§6.4).

### 6.10 Testing

- Unit (pure): `App` selection move/expand-collapse/reconcile-on-refresh (incl. name-absent +
  index-clamp), filter, action dispatch, `--yes` confirm gating.
- Unit (pure, work-core): `windows()` parsing/shape; `validate_name` reserved-set after consolidation.
- Render smoke (optional): ratatui `TestBackend` draw of a small model.
- Manual: open the dashboard with a real OrbStack/Docker, exercise attach + each lifecycle key,
  confirm terminal is sane after `q`/Ctrl-C and after a forced error.

---

## 7. Dependencies (workspace-pinned in `crates/cli/Cargo.toml`)

- `clap_complete = { version = "4", features = ["unstable-dynamic"] }` — exact patch fixed by `Cargo.lock` (current `4.6.8`); see §5.7 for the unstable-wire-protocol pinning note.
- `ratatui = "0.30"` — pulls `ratatui-crossterm`; prefer importing crossterm via the `ratatui::crossterm` re-export so event/Command types always match the backend.
- `crossterm = "0.29"` — **not 0.28**: ratatui 0.30's backend defaults to crossterm 0.29 (verified from ratatui-crossterm's cfg-if precedence); pinning 0.28 adds a dead crate and `CrosstermBackend` rejects a 0.28 stdout. (A direct crossterm dep is optional if you import via the re-export.)

(`clap` stays `"4"`, resolving to ≥4.5.20.) No async runtime.

> **Toolchain:** clap_complete 4.6.8 (`unstable-dynamic`) and ratatui 0.30 raise the build toolchain
> floor to ~**1.85–1.86**. The repo's `rust-toolchain.toml` pins `channel = "stable"` (current stable
> ≥ 1.86), so no change is required; just don't pin an older MSRV. `work` itself stays edition 2021.
## 8. Phasing (outline for the implementation plan)

1. **Completion** — `CompleteEnv` at top of `main()`; per-arg workspace completers; bare-name via
   `external_subcommand` + `SubcommandCandidates`; `RESERVED` consolidation (+ orphan bug fix);
   README install docs; unit + dispatch-equivalence tests.
2. **TUI scaffold** — `tui/` module; RAII restore guard; poll event loop; flat list from
   `list_all()`; name-keyed selection + refresh reconciliation; engine probe + states;
   stdin&&stdout TTY routing; Enter→attach.
3. **Nested tabs** — `windows()` DTO in work-core; expand/collapse; tab-level attach; lazy tab fetch.
4. **Lifecycle actions** — start/stop/rm/new/tab + inline `decide` confirms (threading `--yes`);
   filter; background-refresh worker.

Phases are independently shippable: completion first (no TUI dependency), then the scaffold, etc.

---

## 9. Review findings reconciliation

| # | Severity | Finding | Resolution in spec |
|---|----------|---------|--------------------|
| RV-1 | HIGH | `aot` (static) and dynamic `CompleteEnv` are mutually exclusive; `eval "$(work completions …)"` yields no live names. | §5.1: dynamic-only; drop `Completions{shell}` subcommand. |
| RV-2 | HIGH | `completions` not in `RESERVED` → subcommand unreachable. | §5.5: no subcommand added; RESERVED consolidated regardless. |
| RV-3 | HIGH | `CompleteEnv::complete()` must precede the bare-dispatch block + any stdout. | §5.2: first statement of `main()`. |
| RV-4 | HIGH | No terminal-restore discipline; errors/panics leave raw mode + alt screen. | §6.5: RAII Drop guard held for whole TUI lifetime. |
| RV-5 | HIGH | Ctrl-C swallowed under raw mode; no quit key defined. | §6.5: explicit Ctrl-C/q/Esc quit; documented in footer. |
| RV-6 | HIGH | Engine-down misreported as "all missing". | §6.2: probe `detect()`/`is_running()` before raw mode; dedicated state. |
| RV-7 | HIGH | Two `RESERVED` lists diverge (orphan bug); new verb needs both. | §5.5: single shared constant; backfill `resume`/`stop-all`. |
| RV-8 | MED | Bare-name completion: clean path is `external_subcommand` + `SubcommandCandidates`. | §5.4: adopt it (with dispatch-equivalence tests). |
| RV-9 | MED | Dynamic install is per-shell `source <(COMPLETE=…)>`, not `eval`. | §5.6: per-shell self-correcting install; no file write. |
| RV-10 | MED | Refresh race: index-keyed selection jumps / OOB; errors blank list. | §6.4: name-keyed selection + reconcile; errors keep model. |
| RV-11 | MED | TTY gate checks only stdout; attach needs stdin TTY too. | §6.1: `stdin && stdout` TTY (+ `TERM=dumb`). |
| RV-12 | MED | `--yes` not threaded into TUI confirms. | §6.8: `decide(.., is_tty=true, cli.yes)`. |
| RV-13 | LOW | Pinning alone insufficient; file installs break on bump. | §5.7: pin + self-correcting startup install. |
| RV-14 | LOW | "pub WindowRow" insufficient — fields must be pub. | §6.6: pub DTO with pub fields. |

---

## 10. Open questions / implementation-time risks

- **`external_subcommand` dispatch equivalence (§5.4):** the highest-risk item in Phase 1. Validate
  early that it preserves `work <ws>`, `work <cmd> help` normalization, and global `--yes`. If not,
  fall back to a bespoke `CompleteEnv` candidate injection.
- **`CompleteEnv` exact API (§5.2):** unstable; confirm the factory closure signature against the
  pinned `clap_complete` version during implementation.
- **`Completions` subcommand not shipped:** if a user later wants a static (engine-free) script,
  `clap_complete::aot::generate` can be wired behind an opt-in; deliberately deferred (YAGNI).
- **Bare-name with `external_subcommand` + extra args:** decide whether `work foo bar` attaches to
  `foo` (today) or errors; spec assumes attach-to-first (preserve current behavior).
