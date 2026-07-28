# `work` Sessions Layer — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the workflow layer to `work`: persistent in-container tmux sessions, a cockpit tiler, shell/tmux-config seeding, safe `work rm`, and a uniform TTY-aware destructive-op safety policy — without weakening isolation.

**Architecture:** Sessions live INSIDE each container (in-container tmux server, session `work`). The host multiplexer is ONLY the `work resume` cockpit tiler (host tmux, prefix `C-a`), which spawns one `work <ws>` window per running workspace. Engine gains `session_exists` + `seed_file`. A pure `safety` module decides Proceed/Prompt/Refuse from (severity, live-session, tty, --yes); the CLI applies it. Isolation model (one container + one volume @ /home/dev + one network per ws) is unchanged.

**Tech Stack:** Rust 2021 (stable 1.96), clap 4 (derive), anyhow, serde/toml, chrono. `std::io::IsTerminal` for TTY detection (no extra dep). Docker CLI via OrbStack/Docker/Podman/Colima. In-container tmux (already in base image).

## Spec-review refinements (APPROVED — supersede the corresponding sentences in `docs/superpowers/specs/2026-07-28-work-sessions-design.md`)

The user approved these at the spec-review gate. The design-spec prose is the user's to edit; this plan encodes them as the build contract:

1. **Shell detection clamps** to `zsh` or `bash`; any other basename (fish/nu/sh/unknown/unset) falls back to `zsh`. Only promise shells the image ships.
2. **`--import-shell-config` with no path**: if the inferred rc (`~/.zshrc` / `~/.bashrc`) is absent, error clearly — never silently skip.
3. **Cockpit reuses `work <ws>`** (not a raw `docker exec … tmux attach`). One combined detach hint on the host session; `work <ws>` suppresses its own one-line hint when launched as a cockpit child via `WORK_COCKPIT=1`.
4. **`--yes`/`-y`** is a clap `global = true` arg on the top-level CLI, read after parse; subcommands only. Bare `work <ws>` is non-destructive and outside clap's manual dispatcher.
5. **Defensive semantics**: `session_exists()` returns `false` (not an error) when the container is stopped/missing; `work resume` prechecks host `tmux`.

## Global Constraints

- **License:** MIT. Repo-only edits.
- **Locked decisions:** in-container tmux (not host/daemon); seeding opt-in + verbatim + warned; `work rm` keeps volume by default, `--purge`+`--yes` deletes it; isolation unchanged (no new cross-container path); CLI-only (no GUI/daemon/remote).
- **Naming:** container `work-<ws>`, volume `work-<ws>-home` @ `/home/dev`, network `work-net-<ws>`, in-container tmux session `work`, host cockpit session `work`. Host prefix `C-a`, in-container prefix `C-b`.
- **Attach command (single source of truth):** `docker exec -it -w /home/dev work-<ws> tmux new-session -A -s work -- <shell> -l`.
- **Shell resolution:** `WorkspaceConfig.shell` (`zsh`|`bash`), default `zsh`.
- **Quality gates:** `cargo fmt`; `cargo clippy -- -D warnings`; `cargo test` clean (pure unit tests). Docker paths verified end-to-end against OrbStack.

## File Structure

- `crates/core/src/engine.rs` — ADD `session_exists`, `seed_file` to trait + `DockerCli`.
- `crates/core/src/config.rs` — ADD `detect_shell()`, `rc_name()`, `GlobalConfig.import_shell_config/import_tmux_config`.
- `crates/core/src/safety.rs` — NEW: pure `Severity`, `Action`, `decide()`.
- `crates/core/src/workspace.rs` — REWRITE `shell()`; ADD seeding + first-run suppression in `create()`; ADD `has_live_session()`, `remove(purge)`, `resume()`; ADD `session_live` to `WorkspaceStatus` + `list_all()`.
- `crates/core/src/lib.rs` — `pub mod safety;`.
- `crates/cli/src/safety.rs` — NEW: `confirm_destructive()` TTY-aware prompt over `safety::decide`.
- `crates/cli/src/commands.rs` — ADD `rm`, `resume`; change `new` (seeding), `stop`/`stop_all`/`config_edit` (confirms), `ls` (SESSION col), `all`→`resume` alias.
- `crates/cli/src/main.rs` — ADD global `--yes`/`-y`, `Rm`, `Resume` variants; thread `yes` through.
- `crates/docker/work-base.Dockerfile` — ADD `bash`.
- Tests: `crates/core/tests/safety.rs` (NEW); extend `config.rs` test; `naming.rs` already reserves `rm`.

---

## Task 1: Engine — `session_exists` + `seed_file`

**Files:** Modify `crates/core/src/engine.rs` (trait ~L77, `impl Engine for DockerCli` ~L355).

**Interfaces — Produces:**
```rust
// trait Engine
fn session_exists(&self, name: &str, session: &str) -> Result<bool>;
fn seed_file(&self, name: &str, src: &Path, dest: &str) -> Result<()>;
```

- [ ] **Step 1 — Add to the trait** (after `exec_capture`):
```rust
    /// True iff container `name` has a running tmux server with `session`.
    /// Returns `false` (NOT an error) when the container is missing/stopped
    /// or the session is absent — so callers (`ls`, `stop`, `rm`) never choke.
    fn session_exists(&self, name: &str, session: &str) -> Result<bool>;

    /// Copy host file `src` into container `name` at `dest`, owned by `dev`.
    /// `docker cp` + `chown dev:dev` (chown as root). No host bind-mount created.
    fn seed_file(&self, name: &str, src: &Path, dest: &str) -> Result<()>;
```

- [ ] **Step 2 — Implement in `impl Engine for DockerCli`** (after `exec_capture`):
```rust
    fn session_exists(&self, name: &str, session: &str) -> Result<bool> {
        // Any non-zero (no container / not running / no session) -> false.
        let code = self
            .cmd()
            .args(["exec", name, "tmux", "has-session", "-t", session])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        Ok(code.success())
    }
    fn seed_file(&self, name: &str, src: &Path, dest: &str) -> Result<()> {
        let src_s = src.to_string_lossy();
        let target = format!("{name}:{dest}");
        self.run_success(&["cp", &src_s, &target])
            .with_context(|| format!("copying {} into {target}", src.display()))?;
        // docker cp preserves source uid/gid numerically; chown to dev as root.
        self.run_success(&["exec", "--user", "root", name, "chown", "dev:dev", dest])
            .with_context(|| format!("chown {dest} to dev"))?;
        Ok(())
    }
```

- [ ] **Step 3 — `cargo check -p work-core`** (compiles; used by later tasks).

## Task 2: Base image — add `bash`

**Files:** Modify `crates/docker/work-base.Dockerfile`.

- [ ] **Step 1 — Add `bash` to the apt install list** (so a host bash user gets a matching shell):
```
      git openssh-client ca-certificates tmux zsh bash curl jq build-essential sudo \
```
- [ ] **Step 2 — Rebuild** (done at the end-to-end milestone): `cargo run -q -- image build`.

## Task 3: Config — shell detection + global import defaults

**Files:** Modify `crates/core/src/config.rs`; extend `crates/core/tests/config.rs`.

**Interfaces — Produces:**
```rust
pub fn detect_shell() -> String;                 // basename($SHELL) clamped to zsh|bash, else zsh
pub fn rc_name(shell: &str) -> &'static str;     // "zsh"->".zshrc", else ".bashrc"
// GlobalConfig gains:
#[serde(default, skip_serializing_if = "Option::is_none")]
pub import_shell_config: Option<PathBuf>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub import_tmux_config: Option<PathBuf>,
```

- [ ] **Step 1 — Write failing test** (`crates/core/tests/config.rs`):
```rust
#[test]
fn detect_shell_clamps_to_zsh_or_bash() {
    use work_core::config::{detect_shell, rc_name};
    let sh = detect_shell();
    assert!(sh == "zsh" || sh == "bash", "got {sh}");
    assert_eq!(rc_name("zsh"), ".zshrc");
    assert_eq!(rc_name("bash"), ".bashrc");
    assert_eq!(rc_name("fish"), ".bashrc"); // non-zsh -> bashrc
}

#[test]
fn global_config_supports_import_defaults() {
    let g: work_core::config::GlobalConfig =
        toml::from_str("import_shell_config = '/Users/x/.zshrc'\nimport_tmux_config = '/Users/x/.tmux.conf'\n").unwrap();
    assert_eq!(g.import_shell_config.as_deref(), Some(std::path::Path::new("/Users/x/.zshrc")));
    assert_eq!(g.import_tmux_config.as_deref(), Some(std::path::Path::new("/Users/x/.tmux.conf")));
}
```

- [ ] **Step 2 — Run test → FAIL** (`cargo test -p work-core --test config`).

- [ ] **Step 3 — Implement** in `config.rs`: add `use std::path::Path;` (already has PathBuf). Add the two fields to `GlobalConfig`. Add:
```rust
/// Detected host shell, clamped to a shell the base image ships.
pub fn detect_shell() -> String {
    let sh = std::env::var("SHELL")
        .ok()
        .and_then(|s| Path::new(&s).file_name().map(|f| f.to_string_lossy().into_owned()))
        .unwrap_or_default();
    match sh.as_str() {
        "zsh" | "bash" => sh,
        _ => "zsh".to_string(),
    }
}

/// rc filename for a shell; non-zsh -> .bashrc.
pub fn rc_name(shell: &str) -> &'static str {
    if shell == "zsh" { ".zshrc" } else { ".bashrc" }
}
```
Also add the fields to the `load_global()` empty-config fallback (`GlobalConfig { default_image: Some(...), import_shell_config: None, import_tmux_config: None }`).

- [ ] **Step 4 — Run test → PASS.**

## Task 4: Workspace — rewrite `shell()` + seeding + first-run suppression + `has_live_session`

**Files:** Modify `crates/core/src/workspace.rs`. `Workspace::create()` signature changes (consumers updated in Task 8).

**Interfaces — Produces:**
```rust
pub fn shell(&self) -> Result<()>;                       // attach-or-create tmux session `work`
pub fn has_live_session(&self) -> bool;                  // running + session_exists("work")
// create() gains:
pub fn create(
    name: &str, image: Option<String>, git_name: Option<String>, git_email: Option<String>,
    import_shell: Option<ImportSrc>, import_tmux: Option<ImportSrc>,
) -> Result<Self>;
// ImportSrc lives in config.rs:
pub enum ImportSrc { Auto, Explicit(PathBuf) }
```

- [ ] **Step 1 — Add `ImportSrc` to `config.rs`:**
```rust
/// Where a seeded config file comes from.
#[derive(Debug, Clone)]
pub enum ImportSrc {
    /// Use the detected default (e.g. ~/.zshrc, ~/.tmux.conf).
    Auto,
    /// An explicit host path.
    Explicit(PathBuf),
}
```

- [ ] **Step 2 — Rewrite `shell()`** (attach-or-create; suppress hint under `WORK_COCKPIT`):
```rust
    /// `work <ws>`: ensure running, then attach-or-create the in-container
    /// tmux session `work`. Survives detach / terminal close; not `work stop`.
    pub fn shell(&self) -> Result<()> {
        self.ensure_running()?;
        let ctr = naming::container(&self.cfg.name);
        let shell = self.cfg.shell.as_deref().unwrap_or("zsh");
        if std::env::var_os("WORK_COCKPIT").is_none() {
            println!("Ctrl-b d or close terminal = detach (keeps running) · exit = close session");
        }
        self.engine.exec_interactive(
            &ctr,
            &["tmux", "new-session", "-A", "-s", "work", "--", shell, "-l"],
        )
    }
```

- [ ] **Step 3 — Add `has_live_session()`** (after `status()`):
```rust
    /// True iff the container is up AND its tmux session `work` exists.
    pub fn has_live_session(&self) -> bool {
        let ctr = naming::container(&self.cfg.name);
        match self.engine.container_state(&ctr) {
            Ok(ContainerState::Running) => {}
            _ => return false,
        }
        self.engine.session_exists(&ctr, "work").unwrap_or(false)
    }
```

- [ ] **Step 4 — Extend `create()`** signature + seeding. After `engine.run(&opts)?;` and before `config::save_workspace`, resolve and seed, and set `shell`. Use this block (replacing the `shell: None` literal with the detected shell, and inserting seeding before `apply_git_identity`):
```rust
        let shell = config::detect_shell();

        // --- familiarity seeding (opt-in, verbatim, warned) ---
        if let Some(src) = resolve_import(import_shell, global.import_shell_config.as_deref()) {
            let rc = config::rc_name(&shell);
            let host_path = match src {
                std::path::PathBuf p @ if /* Explicit */ => p,
                /* Auto */ => home_rc(rc),
            };
            seed_into(&*engine, &ctr, &host_path, &format!("/home/dev/{rc}"), &shell, name)?;
        }
        if let Some(src) = resolve_import(import_tmux, global.import_tmux_config.as_deref()) {
            let host_path = match src { Explicit(p) => p, Auto => home_rc(".tmux.conf") };
            seed_into(&*engine, &ctr, &host_path, "/home/dev/.tmux.conf", "tmux", name)?;
        }
        // First-run prompt suppression: ensure /home/dev/.<rc> exists (empty if not seeded),
        // because the named volume overlays the image's baked-in /home/dev.
        ensure_rc_present(&*engine, &ctr, &config::rc_name(&shell))?;
```
(Implement `resolve_import`, `home_rc`, `seed_into`, `ensure_rc_present` as free fns in `workspace.rs` — see Step 5.) Set `cfg.shell = Some(shell)` in the `WorkspaceConfig { .. }`.

- [ ] **Step 5 — Helper fns in `workspace.rs`:**
```rust
use crate::config::ImportSrc;
use std::path::{Path, PathBuf};

/// Effective import source: per-workspace flag overrides the global default.
fn resolve_import(flag: Option<ImportSrc>, global: Option<&Path>) -> Option<ImportSrc> {
    flag.or_else(|| global.map(ImportSrc::Explicit))
}

fn home_rc(name: &str) -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(name)
}

/// Copy a host file into the container volume, owned by dev, with the secret warning.
fn seed_into(engine: &dyn Engine, ctr: &str, src: &Path, dest: &str, kind: &str, ws: &str) -> Result<()> {
    if !src.exists() {
        bail!("{} config not found at {}; pass an explicit path", kind, src.display());
    }
    engine.seed_file(ctr, src, dest)
        .with_context(|| format!("seeding {} from {}", dest, src.display()))?;
    println!(
        "⚠  Copied your {kind} config into '{ws}'. Ensure it contains no secrets — it now lives in that workspace's volume."
    );
    Ok(())
}

/// Ensure /home/dev/<rcname> exists (create empty as dev if absent) so the
/// shell's first-run prompt never fires inside the container.
fn ensure_rc_present(engine: &dyn Engine, ctr: &str, rcname: &str) -> Result<()> {
    let path = format!("/home/dev/{rcname}");
    // `test -e` -> 0 if exists; else touch as the dev user.
    let exists = engine
        .exec_capture(ctr, &["test", "-e", &path])
        .is_ok();
    if !exists {
        engine.exec_capture(ctr, &["touch", &path]).ok();
    }
    Ok(())
}
```
(Note: `resolve_import`'s `match` in Step 4 is illustrative — implement via a small helper `fn import_path(src: ImportSrc, auto_name: &str) -> PathBuf` returning `Explicit(p)` or `home_rc(auto_name)`, to avoid the invalid `PathBuf p @ if` pseudocode. Clean it up: `ImportSrc` needs a method `to_path(&self, auto: &str) -> PathBuf`.)

- [ ] **Step 6 — `cargo check -p work-core`** (Task 8 updates the CLI callsite).

## Task 5: Workspace — `remove(purge)` + `resume()` cockpit

**Files:** Modify `crates/core/src/workspace.rs`. Remove the old `tmux_all()` body (rewrite as `resume()`).

**Interfaces — Produces:**
```rust
pub fn remove(&self, purge: bool) -> Result<()>;   // container+net+config; volume iff purge
pub fn resume() -> Result<()>;                      // host tmux cockpit (was tmux_all)
```

- [ ] **Step 1 — `remove()`** (keeps volume by default):
```rust
    /// `work rm <ws> [--purge]`: remove container + network + config.
    /// Keeps the named volume unless `purge` (data loss; gated by the caller).
    pub fn remove(&self, purge: bool) -> Result<()> {
        let ctr = naming::container(&self.cfg.name);
        let net = naming::network(&self.cfg.name);
        let vol = naming::volume(&self.cfg.name);
        if self.engine.container_exists(&ctr)? {
            self.engine.remove_container(&ctr)?; // `rm -f` stops + removes
        }
        // Network may fail if a lingering forwarder (work-fwd-*) is attached;
        // best-effort + warn rather than failing the data-safe rm.
        if let Err(e) = self.engine.remove_network(&net) {
            eprintln!("· could not remove network {net} ({e}); stop any `work fwd` for this workspace first");
        }
        if purge && self.engine.volume_exists(&vol)? {
            self.engine.remove_volume(&vol)?;
        }
        let _ = std::fs::remove_file(config::workspace_config_path(&self.cfg.name));
        Ok(())
    }
```

- [ ] **Step 2 — `resume()` cockpit** (rewrite of `tmux_all`; prefix `C-a`, running-only, footer, `WORK_COCKPIT=1` windows, combined hint). Replace `tmux_all()`:
```rust
/// `work resume` (= `work all`): host tmux cockpit tiling every RUNNING
/// workspace's session. Host prefix C-a (in-container is C-b). No path between
/// containers is created — each window is an isolated `docker exec` client.
pub fn resume() -> Result<()> {
    if which_host("tmux").not() {
        bail!("host `tmux` not found; install it (`brew install tmux`) to use the cockpit");
    }
    let engine = crate::engine::detect()?;
    let mut running = Vec::new();
    let mut stopped = Vec::new();
    for name in config::list_workspace_names()? {
        let ctr = naming::container(&name);
        match engine.container_state(&ctr).unwrap_or(ContainerState::Missing) {
            ContainerState::Running => running.push(name),
            _ => stopped.push(name),
        }
    }
    if running.is_empty() {
        if stopped.is_empty() {
            bail!("no workspaces yet; create one with `work new <ws>`");
        }
        bail!(
            "no running workspaces. Stopped: {}. Start one with `work start <ws>`",
            stopped.join(", ")
        );
    }

    let _ = Command::new("tmux").args(["kill-session", "-t", "work"]).status();
    // Create the host session detached so we can set prefix + add windows.
    let first = &running[0];
    let _ = Command::new("tmux")
        .args(["new-session", "-d", "-s", "work", "-n", first, cockpit_cmd(first)])
        .status()?;
    let _ = Command::new("tmux").args(["set-option", "-t", "work", "prefix", "C-a"]).status()?;
    for name in running.iter().skip(1) {
        let _ = Command::new("tmux")
            .args(["new-window", "-t", "work", "-n", name, cockpit_cmd(name)])
            .status()?;
    }
    if !stopped.is_empty() {
        let note = format!("stopped: {} — `work start <ws>` to include", stopped.join(", "));
        let _ = Command::new("tmux").args(["set-option", "-t", "work", "-q", "status-bottom", "on"]).status();
        let _ = Command::new("tmux").args(["set-option", "-t", "work", "-q", "status-format", "1", &format!("[{note}]")]).status();
        eprintln!("{note}");
    }

    println!("cockpit prefix: Ctrl-a (switch window / detach cockpit) · in-container: Ctrl-b d (detach one session)");
    let attach = if std::env::var_os("TMUX").is_some() {
        Command::new("tmux").args(["switch-client", "-t", "work"]).status()
    } else {
        Command::new("tmux").args(["attach-session", "-t", "work"]).status()
    };
    if let Err(e) = attach {
        bail!("failed to attach to tmux session 'work': {e}");
    }
    Ok(())
}

/// Window command: run `work <ws>` with the cockpit hint suppressed.
fn cockpit_cmd(ws: &str) -> String {
    let bin = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "work".into());
    format!("WORK_COCKPIT=1 {bin} {ws}")
}

fn which_host(bin: &str) -> bool {
    Command::new(bin).arg("-V").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
}
```
(Add `use std::process::Stdio;` if not present. `which_host(...).not()` → use `!which_host(...)`.)

## Task 6: `work ls` — SESSION column

**Files:** Modify `crates/core/src/workspace.rs` (`WorkspaceStatus`, `list_all`) + `crates/cli/src/commands.rs` (`ls`).

- [ ] **Step 1 — Add `session_live` to `WorkspaceStatus`** and probe in `list_all()`:
```rust
#[derive(Debug, Clone)]
pub struct WorkspaceStatus {
    pub name: String,
    pub state: ContainerState,
    pub session_live: bool,
}
// in list_all():
let session_live = state == ContainerState::Running
    && engine.session_exists(&ctr, "work").unwrap_or(false);
out.push(WorkspaceStatus { name, state, session_live });
```

- [ ] **Step 2 — Print the column** in `commands::ls`:
```rust
    println!("{:<24} {:<8} SESSION", "WORKSPACE", "STATE");
    for WorkspaceStatus { name, state, session_live } in entries {
        let label = match state { Running=>"running", Stopped=>"stopped", Missing=>"missing" };
        let sess = if session_live { "live" } else { "—" };
        println!("{:<24} {:<8} {sess}", name, label);
    }
```

## Task 7: Safety — pure `decide()` + tests

**Files:** NEW `crates/core/src/safety.rs`; NEW `crates/core/tests/safety.rs`; `crates/core/src/lib.rs` (`pub mod safety;`).

**Interfaces — Produces:**
```rust
pub enum Severity { WorkLoss, DataLoss }
pub enum Action { Proceed, Prompt, Refuse }
pub fn decide(severity: Severity, has_live_session: bool, is_tty: bool, yes: bool) -> Action;
```

- [ ] **Step 1 — Write failing test** (`crates/core/tests/safety.rs`):
```rust
use work_core::safety::{decide, Action, Severity};

#[test]
fn dataloss_needs_yes_or_tty() {
    assert_eq!(decide(Severity::DataLoss, false, false, false), Action::Refuse);
    assert_eq!(decide(Severity::DataLoss, true, false, false), Action::Refuse);
    assert_eq!(decide(Severity::DataLoss, false, true, false), Action::Prompt);
    assert_eq!(decide(Severity::DataLoss, false, false, true), Action::Proceed);
}

#[test]
fn workloss_silent_when_no_live_session() {
    // nothing to lose -> proceeds silently regardless of tty/yes
    assert_eq!(decide(Severity::WorkLoss, false, false, false), Action::Proceed);
    assert_eq!(decide(Severity::WorkLoss, false, true, false), Action::Proceed);
}

#[test]
fn workloss_with_live_session_prompts_or_refuses() {
    assert_eq!(decide(Severity::WorkLoss, true, true, false), Action::Prompt);
    assert_eq!(decide(Severity::WorkLoss, true, false, false), Action::Refuse);
    assert_eq!(decide(Severity::WorkLoss, true, true, true), Action::Proceed);
}
```

- [ ] **Step 2 — Run → FAIL.**

- [ ] **Step 3 — Implement** `crates/core/src/safety.rs`:
```rust
//! Destructive-operation safety policy (PURE). The CLI supplies TTY/`--yes`
//! and applies the returned Action.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Kills a live session / agents (data safe). E.g. stop, rm (default), recreate.
    WorkLoss,
    /// Irreversible data loss: volume purge (`rm --purge`).
    DataLoss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Proceed,
    Prompt,
    Refuse,
}

/// Decide what to do for a destructive op.
/// - DataLoss: always gated -> `--yes` proceeds, else TTY prompts, else refuse.
/// - WorkLoss: silent when nothing live; otherwise same gating as DataLoss.
pub fn decide(severity: Severity, has_live_session: bool, is_tty: bool, yes: bool) -> Action {
    if yes {
        return Action::Proceed;
    }
    let gated = match severity {
        Severity::DataLoss => true,
        Severity::WorkLoss => has_live_session,
    };
    if !gated {
        return Action::Proceed;
    }
    if is_tty {
        Action::Prompt
    } else {
        Action::Refuse
    }
}
```
Add `pub mod safety;` to `lib.rs`.

- [ ] **Step 4 — Run → PASS.**

## Task 8: CLI — global `--yes`, commands, flags, TTY confirms

**Files:** `crates/cli/src/main.rs`, `crates/cli/src/commands.rs`, NEW `crates/cli/src/safety.rs`.

**Interfaces — Consumes:** `safety::{decide, Action, Severity}`, `Workspace::{has_live_session, remove}`, `workspace::resume`, `config::ImportSrc`, `Workspace::create(.., import_shell, import_tmux)`.

- [ ] **Step 1 — `crates/cli/src/safety.rs`** (TTY-aware prompt over `decide`):
```rust
use anyhow::{bail, Result};
use std::io::IsTerminal;
use work_core::safety::{decide, Action, Severity};

/// Apply the safety policy. `describe` names what is lost (e.g. "purge volume
/// work-x-home", "end its 1 running session").
pub fn confirm(severity: Severity, has_live_session: bool, yes: bool, ws: &str, describe: &str) -> Result<()> {
    let is_tty = std::io::stdin().is_terminal();
    match decide(severity, has_live_session, is_tty, yes) {
        Action::Proceed => Ok(()),
        Action::Prompt => {
            eprint!("'{ws}': {describe}. continue? [y/N] ");
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            match line.trim().to_lowercase().as_str() {
                "y" | "yes" => Ok(()),
                _ => bail!("aborted"),
            }
        }
        Action::Refuse => bail!(
            "'{ws}': {describe} refused (non-interactive, no --yes). Re-run with --yes/-y to proceed."
        ),
    }
}
```

- [ ] **Step 2 — `main.rs`:** add global `--yes`/`-y` (global arg on `Cli`), `Rm`, `Resume`, `All`→resume alias, and thread `yes`:
```rust
struct Cli {
    #[arg(short = 'y', long, global = true)]
    yes: bool,
    #[command(subcommand)]
    command: Option<Command>,
}
// enum Command gains:
    /// Remove a workspace (keeps the volume unless --purge).
    Rm { ws: String, #[arg(long)] purge: bool },
    /// Cockpit: tile all running workspaces' sessions in a host tmux.
    Resume,
    /// (alias of `resume`)
    All,
// match arms:
    Some(Command::Rm { ws, purge }) => commands::rm(&ws, purge, cli.yes)?,
    Some(Command::Resume) | Some(Command::All) => commands::resume()?,
    Some(Command::Stop { name }) => commands::stop(&name, cli.yes)?,
    Some(Command::StopAll) => commands::stop_all(cli.yes)?,
    Some(Command::Config { ws, edit }) => {
        if edit { commands::config_edit(&ws, cli.yes)? } else { commands::config_show(&ws)? }
    }
    Some(Command::New(a)) => commands::new(&a.name, a.image, a.git_name, a.git_email, a.import_shell_config.clone(), a.import_tmux_config.clone())?,
```
Add `"resume"` and `"rm"` are already handled; ensure bare-name RESERVED includes nothing new (none needed). Keep `All` dispatch.

- [ ] **Step 3 — `commands.rs`:** new/changed functions:
```rust
mod safety;  // or declare in main.rs
use safety::confirm;
use work_core::safety::Severity;
use work_core::config::ImportSrc;

pub fn new(name, image, git_name, git_email, import_shell: Option<String>, import_tmux: Option<String>) -> Result<()> {
    let to_src = |v: Option<String>| match v {
        None => None,
        Some(s) if s.is_empty() => Some(ImportSrc::Auto),
        Some(s) => Some(ImportSrc::Explicit(s.into())),
    };
    let ws = Workspace::create(name, image, git_name, git_email, to_src(import_shell), to_src(import_tmux))?;
    println!("✓ created workspace '{}'", ws.cfg.name);
    println!();
    println!("Next:");
    println!("  work {name}        # attach to its persistent session");
    Ok(())
}

pub fn stop(name: &str, yes: bool) -> Result<()> {
    let ws = Workspace::open(name)?;
    let live = ws.has_live_session();
    confirm(Severity::WorkLoss, live, yes, name, "stopping will end its running session")?;
    ws.stop()?;
    println!("✓ stopped '{}'", name);
    Ok(())
}

pub fn stop_all(yes: bool) -> Result<()> {
    let names = config::list_workspace_names()?;
    if names.is_empty() { println!("no workspaces"); return Ok(()); }
    let mut any_live = false;
    for n in &names { if let Ok(ws) = Workspace::open(n) { if ws.has_live_session() { any_live = true; break; } } }
    confirm(Severity::WorkLoss, any_live, yes, "all", "stopping will end running sessions")?;
    for name in &names {
        if let Ok(ws) = Workspace::open(name) {
            match ws.stop() { Ok(()) => println!("✓ stopped '{name}'"), Err(e) => println!("· '{name}': {e}") }
        }
    }
    Ok(())
}

pub fn rm(name: &str, purge: bool, yes: bool) -> Result<()> {
    let ws = Workspace::open(name)?;
    let live = ws.has_live_session();
    let (sev, desc) = if purge {
        (Severity::DataLoss, format!("purge volume work-{name}-home (irreversible)"))
    } else {
        (Severity::WorkLoss, "removing the container will end its running session".to_string())
    };
    confirm(sev, live, yes, name, &desc)?;
    ws.remove(purge)?;
    if purge {
        println!("removed workspace '{name}' and purged volume work-{name}-home (irreversible).");
    } else {
        println!(
            "removed workspace '{name}' (volume work-{name}-home kept). `work new {name}` recreates it with your files intact; `work rm {name} --purge` deletes the volume."
        );
    }
    Ok(())
}

pub fn resume() -> Result<()> { workspace::resume() }
pub fn all() -> Result<()> { workspace::resume() }

pub fn config_edit(name: &str, yes: bool) -> Result<()> {
    // ... existing editor launch ...
    if ws.cfg.image != before.image {
        confirm(Severity::WorkLoss, ws.has_live_session(), yes, name, "recreating the container will end its running session")?;
        ws.recreate()?;
    } else { ws.apply_git_identity()?; }
    // ...
}
```

- [ ] **Step 4 — `NewArgs`** gains the optional-value flags:
```rust
#[derive(Args)]
pub struct NewArgs {
    pub name: String,
    #[arg(long)] pub image: Option<String>,
    #[arg(long = "git-name")] pub git_name: Option<String>,
    #[arg(long = "git-email")] pub git_email: Option<String>,
    #[arg(long = "import-shell-config", num_args = 0..=1, default_missing_value = "")]
    pub import_shell_config: Option<String>,
    #[arg(long = "import-tmux-config", num_args = 0..=1, default_missing_value = "")]
    pub import_tmux_config: Option<String>,
}
```

- [ ] **Step 5 — `cargo build` + fix callsites** (create() signature, stop/stop_all/config_edit/new signatures). Remove the now-dead `pub fn shell(name)`/`stop(name)` old signatures.

## Task 9: Quality gates

- [ ] `cargo fmt --all`
- [ ] `cargo clippy --all-targets -- -D warnings` → clean
- [ ] `cargo test` → all unit tests pass (safety, config, naming, doctor, engine-detect)

## Task 10: End-to-end verification (real engine, OrbStack)

Run each; capture output as evidence. Commit per milestone.

- [ ] **Rebuild base image** (`bash` added): `cargo run -q -- image build`
- [ ] **Persistence:** `work new t1` → `work t1` (start `top`/a loop inside), detach (Ctrl-b d) by sending the session to background, re-`work t1` → process still running.
- [ ] **Cockpit:** create `t2`, leave both running → `work resume` tiles both; prefix C-a switches windows.
- [ ] **`work ls` SESSION:** shows `live` for running-session workspaces, `—` otherwise.
- [ ] **Seeding:** `work new t3 --import-shell-config` lands `/home/dev/.zshrc` (owned by dev) in the container; warning printed.
- [ ] **rm keep:** `work rm t1` removes container+net+config, keeps volume; `work new t1` → prior files in /home/dev intact.
- [ ] **rm purge:** `work rm t2 --purge` without `--yes` in a pipe → refuses; with `--yes` → deletes volume.
- [ ] **doctor:** `work doctor` passes — isolation intact (unique net, only own volume, non-root, no host ports).

---

## Self-Review (completed)

- **Spec coverage:** Part 1→T1+T4(shell); Part 2→T5(resume); Part 3→T6; Part 4→T3+T4(seeding); Part 5→T5(remove); Part 6→T7+T8(confirms/--yes); base image bash→T2. All six parts covered.
- **Refinements encoded:** 1(shell clamp, T3), 2(missing-rc error, T4 seed_into), 3(cockpit reuse + WORK_COCKPIT, T4+T5), 4(--yes scope, T8), 5(defensive session_exists + tmux precheck, T1+T5).
- **Type consistency:** `ImportSrc` defined in config.rs (T3), consumed in workspace.rs (T4) and commands.rs (T8). `WorkspaceStatus.session_live` added once (T6), read once (T6). `create()` new signature matches the callsite update in T8.
