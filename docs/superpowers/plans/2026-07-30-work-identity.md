# In-Container Identity & "Separated Environment" Feel — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every `work` workspace container immediately, unmistakably identifiable as its own isolated environment — banner on attach, workspace-named sessions/titles, a `WORK` env var, and a default workspace prompt — without weakening isolation or requiring an image rebuild.

**Architecture:** All identity logic lives in `work-core`. `RunOpts` gains an `env` field so every container is born with `WORK=<ws>`; the attach path (`Workspace::shell`) composes a fastfetch-style banner from a single `docker exec` and prints it host-side before attaching, renames the in-container tmux session from the literal `work` to `<ws>` (losslessly), sets the terminal title, and the no-import path writes a minimal default rc with a workspace-aware prompt. The CLI is untouched. The base Dockerfile is untouched.

**Tech Stack:** Rust 2021 (stable), clap 4, anyhow, serde/toml, tempfile (already workspace deps). Docker CLI via OrbStack/Docker/Podman/Colima. tmux in-container.

**Spec:** `docs/superpowers/specs/2026-07-30-work-identity-design.md`

## Global Constraints

- **Isolation invariants are unchanged and must remain provably intact:** one container, one named volume mounted only at `/home/dev`, one bridge network, non-root `dev`, no host ports, image match. `work doctor` must still pass. Every change here is metadata or display.
- **No base-image change / no rebuild required.** `crates/docker/work-base.Dockerfile` is NOT modified; existing personal images (`work-lucas:latest`, …) keep working.
- **No silent edits to the user's prompt config.** The `[Docker]` marker is Starship's (documented as opt-in), never auto-removed.
- **Rust 2021, stable toolchain** (`rust-toolchain.toml`). Quality gates every commit: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- **Naming rule unchanged:** workspace names are lowercase `[a-z0-9][a-z0-9-]*`, length 1..=40, not reserved. The in-container tmux session name equals the workspace name.

## File Structure

- **Create** `crates/core/src/banner.rs` — PURE banner composition (one function + tests). No IO.
- **Modify** `crates/core/src/lib.rs` — declare `pub mod banner;`.
- **Modify** `crates/core/src/naming.rs` — add `session(ws)`.
- **Modify** `crates/core/src/engine.rs` — `RunOpts.env`; `DockerCli::run` emits `-e`.
- **Modify** `crates/core/src/config.rs` — `GlobalConfig.show_banner` (default `true`).
- **Modify** `crates/core/src/workspace.rs` — `run_opts()` helper; rewrite `shell()` (banner + title + lossless session rename + `-s/-n`); `has_live_session()` + `list_all()` use `naming::session`; `ensure_rc_present` → `ensure_default_rc` + default-rc constants.
- **Modify** `README.md` — "A separated environment" section + `[Docker]` opt-in snippet.

---

### Task 1: `naming::session` helper

Centralizes the in-container tmux session name as the workspace name.

**Files:**
- Modify: `crates/core/src/naming.rs` (append after `container`, ~line 19)

**Interfaces:**
- Produces: `pub fn session(ws: &str) -> &str` — returns the workspace name verbatim (the in-container tmux session name). Consumed by Tasks 5 and 6.

- [ ] **Step 1: Write the failing test**

Append to `crates/core/src/naming.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_name_is_the_workspace_name() {
        assert_eq!(session("acme"), "acme");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p work-core session_name_is_the_workspace_name`
Expected: FAIL — `cannot find function session`.

- [ ] **Step 3: Add the function**

Insert after the `container` function in `crates/core/src/naming.rs`:
```rust
/// In-container tmux session name for a workspace. Equals the workspace name, so
/// `tmux ls` and the status bar inside a container reflect its identity. PURE.
pub fn session(ws: &str) -> &str {
    ws
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p work-core session_name_is_the_workspace_name`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/naming.rs
git commit -m "feat(core): add naming::session (in-container tmux session == ws)"
```

---

### Task 2: `WORK`/`WORKSPACE` identity env

Bakes the workspace name into every container's environment so prompts, banners, and tools can read it.

**Files:**
- Modify: `crates/core/src/engine.rs:40-48` (`RunOpts`), `:292-327` (`DockerCli::run`)
- Modify: `crates/core/src/workspace.rs:108-117` (`create`), `:147-156` (`ensure_running` recreate), `:249-258` (`recreate`)
- Test: inline in `crates/core/src/workspace.rs`

**Interfaces:**
- Consumes: `naming::{container,network,volume}` (existing).
- Produces: `RunOpts { env: Vec<(String,String)> }`; free fn `fn run_opts(name: &str, image: &str) -> RunOpts` in `workspace.rs` (module-private). `DockerCli::run` emits `-e KEY=VALUE` per pair.

- [ ] **Step 1: Write the failing test**

Append a test module to `crates/core/src/workspace.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_opts_sets_identity_env_and_names() {
        let opts = run_opts("acme", "work-base:latest");
        assert_eq!(opts.name, "work-acme");
        assert_eq!(opts.network, "work-net-acme");
        assert_eq!(opts.volume, "work-acme-home");
        assert_eq!(
            opts.env,
            vec![
                ("WORK".to_string(), "acme".to_string()),
                ("WORKSPACE".to_string(), "acme".to_string()),
            ]
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p work-core run_opts_sets_identity_env_and_names`
Expected: FAIL — `cannot find function run_opts`.

- [ ] **Step 3: Add the `env` field to `RunOpts`**

In `crates/core/src/engine.rs`, replace the `RunOpts` struct (lines 40-48):
```rust
/// `docker run` options for a workspace container.
pub struct RunOpts {
    pub name: String,
    pub image: String,
    pub network: String,
    pub volume: String,
    pub volume_target: String, // "/home/dev"
    pub workdir: String,
    pub cmd: Vec<String>, // e.g. ["sleep", "infinity"]
    /// Extra environment (`-e KEY=VALUE`). Identity metadata only.
    pub env: Vec<(String, String)>,
}
```

- [ ] **Step 4: Emit `-e` in `DockerCli::run`**

In `crates/core/src/engine.rs`, replace the body of `fn run` from the `c.args([` line through the `for arg in &opts.cmd` loop (lines 294-312) with:
```rust
        let mut c = self.cmd();
        c.args([
            "run",
            "-d",
            "--name",
            &opts.name,
            "--network",
            &opts.network,
            "--restart",
            "unless-stopped",
            "-v",
            &mount,
            "-w",
            &opts.workdir,
        ]);
        for (k, v) in &opts.env {
            c.arg("-e").arg(format!("{k}={v}"));
        }
        c.arg(&opts.image);
        for arg in &opts.cmd {
            c.arg(arg);
        }
```

- [ ] **Step 5: Add `run_opts()` and switch all three `RunOpts` construction sites**

In `crates/core/src/workspace.rs`, add near the other free fns (e.g. just above `fn ensure_image`):
```rust
/// `docker run` options for a workspace container. Sets the `WORK`/`WORKSPACE`
/// identity env so prompts, banners, and tools can name the workspace.
fn run_opts(name: &str, image: &str) -> RunOpts {
    RunOpts {
        name: naming::container(name),
        image: image.to_string(),
        network: naming::network(name),
        volume: naming::volume(name),
        volume_target: "/home/dev".into(),
        workdir: "/home/dev".into(),
        cmd: vec!["sleep".into(), "infinity".into()],
        env: vec![
            ("WORK".into(), name.into()),
            ("WORKSPACE".into(), name.into()),
        ],
    }
}
```

Replace the three `let opts = RunOpts { … };` blocks:
- `create` (lines 108-116) → `let opts = run_opts(name, &image);`
- `ensure_running` Missing arm (lines 147-155) → `let opts = run_opts(&self.cfg.name, &self.cfg.image);`
- `recreate` (lines 249-257) → `let opts = run_opts(&self.cfg.name, &self.cfg.image);`

- [ ] **Step 6: Run test to verify it passes + gates**

Run: `cargo test -p work-core run_opts_sets_identity_env_and_names && cargo fmt && cargo clippy --all-targets -- -D warnings`
Expected: test PASS; fmt clean; clippy clean.

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/engine.rs crates/core/src/workspace.rs
git commit -m "feat(core): bake WORK/WORKSPACE identity env into every container"
```

---

### Task 3: `show_banner` global config

Adds an opt-out for the attach banner.

**Files:**
- Modify: `crates/core/src/config.rs:13-23` (`GlobalConfig`), `:25-27` (default fn), `:68-82` (`load_global` default)

**Interfaces:**
- Produces: `GlobalConfig { show_banner: bool }`, serde default `true`. Consumed by Task 5.

- [ ] **Step 1: Write the failing test**

Append to the `#[cfg(test)]` test module (or add one) in `crates/core/src/config.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_config_defaults_banner_on() {
        let g = GlobalConfig {
            default_image: None,
            import_shell_config: None,
            import_tmux_config: None,
            show_banner: true,
        };
        assert!(g.show_banner);
        // Round-trip: a TOML without `show_banner` deserializes to default true.
        let parsed: GlobalConfig = toml::from_str("").unwrap();
        assert!(parsed.show_banner);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p work-core missing_config_defaults_banner_on`
Expected: FAIL — `no field show_banner` on `GlobalConfig`.

- [ ] **Step 3: Add the field + default**

In `crates/core/src/config.rs`, add the field to `GlobalConfig` (after `import_tmux_config`):
```rust
    /// Print the in-container identity banner on `work <ws>` attach (default on).
    #[serde(default = "default_show_banner", skip_serializing_if = "std::ops::Not::not")]
    pub show_banner: bool,
```
Add the default fn next to `default_image`:
```rust
fn default_show_banner() -> bool {
    true
}
```

- [ ] **Step 4: Update the `load_global` absent-file default**

In `load_global` (lines 71-75), add `show_banner: true` to the returned `GlobalConfig`:
```rust
        return Ok(GlobalConfig {
            default_image: Some(DEFAULT_IMAGE.to_string()),
            import_shell_config: None,
            import_tmux_config: None,
            show_banner: true,
        });
```

- [ ] **Step 5: Run test to verify it passes + gates**

Run: `cargo test -p work-core missing_config_defaults_banner_on && cargo fmt && cargo clippy --all-targets -- -D warnings`
Expected: PASS; fmt clean; clippy clean.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/config.rs
git commit -m "feat(core): add show_banner global config (default on)"
```

---

### Task 4: `banner` module (pure composition)

The fastfetch-style identity block, composed from inputs the attach path gathers.

**Files:**
- Create: `crates/core/src/banner.rs`
- Modify: `crates/core/src/lib.rs` (declare module)

**Interfaces:**
- Produces: `pub fn compose(name: &str, image: &str, system: &str, hostname: &str, git: &str) -> String`. Consumed by Task 5.

- [ ] **Step 1: Write the failing test**

Create `crates/core/src/banner.rs` with the test first:
```rust
//! In-container identity banner. PURE: composes the fastfetch-style block from
//! inputs gathered by the attach path. No IO, no dependency on the engine.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_lists_every_field() {
        let s = compose("acme", "work-base:latest", "Debian GNU/Linux 12", "abc1234", "main");
        assert!(s.contains("workspace   acme"));
        assert!(s.contains("image       work-base:latest"));
        assert!(s.contains("system      Debian GNU/Linux 12"));
        assert!(s.contains("hostname    abc1234"));
        assert!(s.contains("network     isolated · single-context"));
        assert!(s.contains("home        /home/dev"));
        assert!(s.contains("git         main"));
        assert!(s.contains("bring your own tools"));
    }

    #[test]
    fn compose_handles_missing_fields() {
        let s = compose("demo", "work-lucas:latest", "—", "—", "—");
        assert!(s.contains("git         —"));
        assert!(!s.contains("git         — ")); // label column fixed width
    }
}
```

- [ ] **Step 2: Declare the module**

In `crates/core/src/lib.rs`, add (after `pub mod safety;`):
```rust
pub mod banner;
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p work-core compose_lists_every_field`
Expected: FAIL — `cannot find function compose`.

- [ ] **Step 4: Implement `compose`**

Add to `crates/core/src/banner.rs`:
```rust
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
```

- [ ] **Step 5: Run tests to verify they pass + gates**

Run: `cargo test -p work-core banner && cargo fmt && cargo clippy --all-targets -- -D warnings`
Expected: PASS; fmt clean; clippy clean.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/banner.rs crates/core/src/lib.rs
git commit -m "feat(core): add pure identity banner composer"
```

---

### Task 5: Rewrite `shell()` — banner, title, lossless session rename

The attach path: print the banner, set the terminal title, migrate a stale `work` session to `<ws>` in place, then attach to `tmux new-session -A -s <ws> -n <ws>`.

**Files:**
- Modify: `crates/core/src/workspace.rs:184-199` (`shell`)

**Interfaces:**
- Consumes: `naming::session` (Task 1), `config::load_global().show_banner` (Task 3), `banner::compose` (Task 4), `engine::{exec_capture, session_exists, exec_interactive}` (existing).
- Produces: the new attach behavior. No signature change to `pub fn shell(&self) -> Result<()>`.

- [ ] **Step 1: Replace `shell()`**

In `crates/core/src/workspace.rs`, replace the whole `shell` method (lines 184-199) with:
```rust
    /// `work <ws>`: ensure running, then attach-or-create the in-container tmux
    /// session named after the workspace. Prints an identity banner and sets the
    /// terminal title first. The session (and anything started inside it) survives
    /// detach / closing the terminal; it does NOT survive `work stop`.
    pub fn shell(&self) -> Result<()> {
        self.ensure_running()?;
        let ctr = naming::container(&self.cfg.name);
        let shell = self.cfg.shell.as_deref().unwrap_or("zsh");
        let session = naming::session(&self.cfg.name);

        // Banner + detach hint (suppressed in cockpit windows).
        if std::env::var_os("WORK_COCKPIT").is_none() {
            let show = config::load_global().map(|g| g.show_banner).unwrap_or(true);
            if show {
                self.print_banner(&ctr);
            }
            println!("Ctrl-b d or close terminal = detach (keeps running) · exit = close session");
        }

        // Lossless one-time migration: rename a stale `work` session to <ws> in
        // place (running shells/agents inside it survive). No-op once renamed, or
        // for a workspace literally named "work".
        if session != "work"
            && self.engine.session_exists(&ctr, "work").unwrap_or(false)
            && !self.engine.session_exists(&ctr, session).unwrap_or(false)
        {
            let _ = self
                .engine
                .exec_capture(&ctr, &["tmux", "rename-session", "-t", "work", session]);
        }

        // Name the terminal tab (best-effort). The tmux window name is set via -n.
        {
            use std::io::Write;
            print!("\x1b]0;work:{}\x07", self.cfg.name);
            let _ = std::io::stdout().flush();
        }

        self.engine.exec_interactive(
            &ctr,
            &[
                "tmux",
                "new-session",
                "-A",
                "-s",
                session,
                "-n",
                session,
                "--",
                shell,
                "-l",
            ],
        )
    }

    /// Gather hostname/OS/git-branch via one `docker exec` and print the banner.
    /// Fail-soft: any error renders the dynamic fields as "—".
    fn print_banner(&self, ctr: &str) {
        let probe = "h=$(hostname 2>/dev/null); . /etc/os-release 2>/dev/null; \
s=${PRETTY_NAME:-}; g=$(git -C /home/dev rev-parse --abbrev-ref HEAD 2>/dev/null || true); \
printf '%s\\t%s\\t%s' \"$h\" \"$s\" \"$g\"";
        let gathered = self
            .engine
            .exec_capture(ctr, &["bash", "-c", probe])
            .unwrap_or_default();
        let mut parts = gathered.splitn(3, '\t');
        let val = |p: Option<&str>| p.filter(|s| !s.is_empty()).unwrap_or("—");
        let hostname = val(parts.next());
        let system = val(parts.next());
        let git = val(parts.next());
        println!(
            "{}",
            banner::compose(&self.cfg.name, &self.cfg.image, system, hostname, git)
        );
    }
```

- [ ] **Step 2: Compile + gates**

Run: `cargo build -p work-core && cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: build OK; fmt clean; clippy clean; tests pass.

- [ ] **Step 3: End-to-end smoke (requires a running engine)**

Run:
```bash
work image build >/dev/null 2>&1 || true
work new banner-demo --git-name Test --git-email t@t.test
work banner-demo
# Inside: Ctrl-b d to detach
```
Expected: the identity banner prints (workspace `banner-demo`, image, system, network, home, git), then the prompt. Detach returns to host.

Run: `echo "check session name"`
Expected (in a second host shell): the banner showed `workspace banner-demo`.

- [ ] **Step 4: Verify session is workspace-named**

Run:
```bash
docker exec work-banner-demo tmux ls
```
Expected: a line beginning with `banner-demo:`.

- [ ] **Step 5: Verify the env var**

Run:
```bash
docker exec -e WORK= work-banner-demo bash -lc 'echo "[$WORK]"'
```
(We override to empty only to prove the container's own env supplies it without the override.) Better:
```bash
docker exec work-banner-demo bash -lc 'echo "WORK=$WORK"'
```
Expected: `WORK=banner-demo`.

- [ ] **Step 6: Clean up**

```bash
work rm banner-demo --yes
```

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/workspace.rs
git commit -m "feat(core): banner + workspace-named session + title on attach"
```

---

### Task 6: Session name in liveness probes

`has_live_session` and `list_all` must look for the workspace-named session, not the literal `work`.

**Files:**
- Modify: `crates/core/src/workspace.rs:214-223` (`has_live_session`), `:303-320` (`list_all`)

**Interfaces:**
- Consumes: `naming::session` (Task 1).

- [ ] **Step 1: Update `has_live_session`**

In `crates/core/src/workspace.rs`, replace line 222:
```rust
        self.engine.session_exists(&ctr, "work").unwrap_or(false)
```
with:
```rust
        self.engine
            .session_exists(&ctr, naming::session(&self.cfg.name))
            .unwrap_or(false)
```

- [ ] **Step 2: Update `list_all`**

In `list_all` (line 312), replace:
```rust
            && engine.session_exists(&ctr, "work").unwrap_or(false);
```
with:
```rust
            && engine
                .session_exists(&ctr, naming::session(&name))
                .unwrap_or(false);
```

- [ ] **Step 3: Build + gates + integration check**

Run:
```bash
cargo build -p work-core && cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
work new probe-demo >/dev/null
work probe-demo </dev/null   # may fail to attach with no tty; instead:
docker exec work-probe-demo tmux new-session -d -s probe-demo
work ls
```
Expected: `probe-demo` row shows `live` in the SESSION column (proving `list_all` resolves the workspace-named session).

- [ ] **Step 4: Clean up**

```bash
work rm probe-demo --yes
```

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/workspace.rs
git commit -m "feat(core): probe the workspace-named tmux session"
```

---

### Task 7: Default workspace prompt (no-import case)

Replaces the empty-rc behavior with a minimal default rc carrying a workspace-aware prompt. Import still wins verbatim.

**Files:**
- Modify: `crates/core/src/workspace.rs:119-124` (seed call in `create`), `:477-487` (`ensure_rc_present` → `ensure_default_rc`)

**Interfaces:**
- Consumes: `engine::seed_file` (existing), `tempfile::tempdir` (existing dep), `config::rc_name`.
- Produces: `fn ensure_default_rc(engine, ctr, rcname, imported) -> Result<()>` (module-private); constants `ZSHRC_DEFAULT`, `BASHRC_DEFAULT`; `fn default_rc(rcname) -> &'static str`.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `crates/core/src/workspace.rs`:
```rust
    #[test]
    fn default_rc_is_workspace_aware() {
        let z = default_rc(".zshrc");
        assert!(z.contains("PROMPT"));
        assert!(z.contains("$WORK"));
        let b = default_rc(".bashrc");
        assert!(b.contains("PS1"));
        assert!(b.contains("WORK"));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p work-core default_rc_is_workspace_aware`
Expected: FAIL — `cannot find function default_rc`.

- [ ] **Step 3: Replace `ensure_rc_present` with `ensure_default_rc` + constants**

In `crates/core/src/workspace.rs`, replace the `ensure_rc_present` function (lines 477-487) with:
```rust
const ZSHRC_DEFAULT: &str = "\
# Default work prompt. Override: `work new --import-shell-config`.
setopt PROMPT_SUBST
PROMPT='%F{magenta}⬡%f %F{cyan}$WORK%f %F{blue}%~%f %# '
";

const BASHRC_DEFAULT: &str = "\
# Default work prompt. Override: `work new --import-shell-config`.
PS1=\"\\[\\e[35m\\]⬡\\[\\e[0m\\] \\[\\e[36m\\]${WORK}\\[\\e[0m\\] \\[\\e[34m\\]\\w\\[\\e[0m\\] $ \"
";

/// Default rc body for the resolved shell.
fn default_rc(rcname: &str) -> &'static str {
    match rcname {
        ".bashrc" => BASHRC_DEFAULT,
        _ => ZSHRC_DEFAULT,
    }
}

/// Ensure `/home/dev/<rcname>` exists. If a shell config was imported it is
/// already present (verbatim) — never overwrite. If nothing was imported and the
/// rc is absent, write the minimal default rc with a workspace-aware prompt.
fn ensure_default_rc(engine: &dyn Engine, ctr: &str, rcname: &str, imported: bool) -> Result<()> {
    let path = format!("/home/dev/{rcname}");
    if engine.exec_capture(ctr, &["test", "-e", &path]).is_ok() {
        return Ok(()); // seeded or persisted — leave it alone.
    }
    if imported {
        let _ = engine.exec_capture(ctr, &["touch", &path]);
        return Ok(()); // import source was absent; keep empty rather than impose.
    }
    let dir = tempfile::tempdir().context("staging default rc")?;
    let src = dir.path().join(rcname);
    std::fs::write(&src, default_rc(rcname)).context("writing default rc")?;
    engine
        .seed_file(ctr, &src, &path)
        .with_context(|| format!("seeding default {rcname}"))?;
    Ok(())
}
```

- [ ] **Step 4: Thread the `imported` flag through `create`**

In `crates/core/src/workspace.rs` `create`, replace the seed loop + `ensure_rc_present` call (lines 121-124):
```rust
        for (src, dest, kind) in &seeds {
            seed_into(&*engine, &ctr, src, dest, kind, name)?;
        }
        let shell_imported = seeds.iter().any(|(_, _, kind)| *kind == "shell");
        ensure_default_rc(&*engine, &ctr, rc, shell_imported)?;
```

- [ ] **Step 5: Run test + gates**

Run: `cargo test -p work-core default_rc_is_workspace_aware && cargo fmt && cargo clippy --all-targets -- -D warnings`
Expected: PASS; fmt clean; clippy clean.

- [ ] **Step 6: End-to-end smoke (no import → default prompt)**

Run:
```bash
work new prompt-demo --git-name Test --git-email t@t.test
docker exec work-prompt-demo cat /home/dev/.zshrc
```
Expected: the default rc with `PROMPT='%F{magenta}⬡%f … $WORK …'` (default shell is zsh). Then:
```bash
docker exec -it work-prompt-demo zsh -i -c 'print -r -- "$PROMPT"'
```
Expected: a prompt string containing `⬡` / color codes referencing `$WORK`.

- [ ] **Step 7: Clean up**

```bash
work rm prompt-demo --yes
```

- [ ] **Step 8: Commit**

```bash
git add crates/core/src/workspace.rs
git commit -m "feat(core): default workspace-aware prompt when no shell config imported"
```

---

### Task 8: README — separated-environment docs

Document the identity surfaces and the opt-in `[Docker]` snippet.

**Files:**
- Modify: `README.md` (add a section after "## Familiarity (optional)")

- [ ] **Step 1: Add the section**

Insert after the "## Familiarity (optional)" section in `README.md`:
```markdown
## A separated environment (in-container identity)

Every `work <ws>` attach makes the workspace unmistakably identifiable:

- **Identity banner.** `work` prints a compact block — workspace, image, system,
  hostname, `network: isolated · single-context`, home dir, git branch — before
  attaching. Opt out in `~/.config/work/config.toml`:
  ```toml
  show_banner = false
  ```
- **`$WORK`.** Each container exports `WORK=<ws>` (and `WORKSPACE=<ws>`), so any
  prompt or tool can name the workspace.
- **Default prompt.** With no `--import-shell-config`, the container gets a minimal
  prompt that shows the workspace: `⬡ acme ~/proj %#`. Import your own rc and it
  wins verbatim.
- **Workspace-named session.** The in-container tmux session is named after the
  workspace (`tmux ls` shows `acme`, not `work`), the window is named `<ws>`, and
  the terminal tab is titled `work:<ws>`. Existing `work`-named sessions are renamed
  in place (lossless) on the next attach.

### The `[Docker]` marker (Starship)

If you import a shell config that runs [Starship](https://starship.rs), its
`container` module renders a fixed `[Docker]` label — it is Starship detecting
`/.dockerenv`, not `work` or OrbStack, and `work` won't edit your prompt config.
Two opt-ins in `~/.config/starship.toml`:

```toml
# 1) Drop the now-redundant engine label:
[container]
disabled = true

# 2) …or show the workspace name instead, via a custom module:
[custom.work]
command = "echo $WORK"
when = """ test -n "$WORK" """
format = '[$output]($style) '
style = 'bold magenta'
```
```

- [ ] **Step 2: Sanity-render check**

Run: `grep -n "A separated environment" README.md`
Expected: one match.

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: document in-container identity and the Starship [Docker] marker"
```

---

## Final verification milestone

After all tasks, run the full gate suite plus an end-to-end pass that exercises the migration:

- [ ] **Quality gates:** `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` — all clean.
- [ ] **Isolation unchanged:** `work doctor` — all invariants hold.
- [ ] **End-to-end + migration:**
  ```bash
  work new verify-demo --git-name Test --git-email t@t.test
  work verify-demo          # banner prints; prompt shows ⬡ verify-demo; detach with Ctrl-b d
  docker exec work-verify-demo tmux ls     # session "verify-demo"
  docker exec work-verify-demo bash -lc 'echo $WORK'   # verify-demo
  work ls                   # SESSION = live
  work rm verify-demo --yes
  ```
- [ ] **Existing workspaces still attach** (e.g. `work test1`): if a legacy `work` session exists it is renamed in place to the workspace name on first attach; no error.

## Self-Review notes

- **Spec coverage:** Part 1 (`WORK`/`WORKSPACE`) → Task 2. Part 2 (banner) → Tasks 3, 4, 5. Part 3 (default prompt) → Task 7. Part 4 (session + titles) → Tasks 1, 5, 6. Part 5 (`[Docker]` docs) → Task 8. All spec parts covered.
- **Type consistency:** `naming::session(&str) -> &str` (Task 1) used identically in Tasks 5 and 6. `RunOpts.env: Vec<(String,String)>` (Task 2) built by `run_opts` and read by `run`. `banner::compose(&str,&str,&str,&str,&str) -> String` (Task 4) called with matching args in Task 5. `GlobalConfig.show_banner: bool` (Task 3) read as `g.show_banner` in Task 5.
- **No placeholders:** every code step contains complete, compilable code; every test step contains runnable assertions.
