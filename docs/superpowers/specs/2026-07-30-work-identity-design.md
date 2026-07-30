# `work` in-container identity & "separated environment" feel

**Date:** 2026-07-30
**Status:** Proposed
**Theme:** Make each workspace container immediately, unmistakably identifiable as its own isolated environment — without weakening isolation or fighting the bring-your-own-tools principle.

## Goal

Today, attaching with `work <ws>` drops you into a shell whose prompt betrays nothing
about *which* workspace you are in. Users importing their host shell config see
Starship render a generic `[Docker]` marker; users who don't import get the bare
default `%m%#` prompt. Nothing surfaces the workspace name, the isolation guarantee,
or the container's identity.

This spec adds four things, all metadata/display-only (zero isolation impact):

1. A `WORK` environment variable baked into every container.
2. A fastfetch-style **identity banner** printed by `work` on every attach.
3. A **default workspace prompt** (no-import case) that shows the workspace name.
4. **Workspace-named sessions + titles**, so the in-container tmux session, window,
   and terminal tab all carry the workspace identity.

It also documents the truth about the `[Docker]` marker and how to suppress it.

## Background — what `~ [Docker]` actually is

Empirically verified against the user's live `work-test1` container (image
`work-lucas:latest`, OrbStack 2.2.1, `orbstack` docker context):

- A **plain** container with an empty `.zshrc` under OrbStack shows `%m%#` (hostname +
  `%`). There is **no** OrbStack prompt injection for plain `docker exec`.
- The `[Docker]` marker is **Starship's `container` module**. The user's `~/.zshrc`
  (imported via `--import-shell-config`) runs `eval "$(starship init zsh)"`, and
  Starship detects `/.dockerenv` and renders the fixed engine label `[Docker]`.
- The label is **engine-derived and fixed**: tested `-e container=acme` and
  `--hostname acme` — Starship ignores both and still shows `[Docker]`. It cannot be
  redirected by env or hostname; changing it requires the user's Starship config.

**Conclusion:** `work` must not silently edit a user's prompt config (it violates the
verbatim, bring-your-own principle). Instead `work` (a) ships a `WORK` env var any
prompt/banner can read, and (b) documents an opt-in Starship snippet. The banner and
session identity make `[Docker]` redundant regardless.

**Mechanism constraint (verified):** a login zsh on `node:20-bookworm-slim` does **not**
source `/etc/profile.d/*` — `/etc/zsh/zprofile` is the bare comment-only template. So the
banner cannot ride on login-shell profile sourcing. It is emitted **host-side** instead.

## Architecture

All identity logic lives in `work-core` (`workspace.rs`, `engine.rs`, `config.rs`,
`naming.rs`). The CLI is unchanged. No base-image change is required, so existing
personal images (`work-lucas:latest`, …) keep working with no rebuild.

---

## Part 1 — `WORK` environment variable (foundation)

Every workspace container is created with two env vars, set via a new `RunOpts` field:

```
-e WORK=<ws>
-e WORKSPACE=<ws>     # familiar alias; some prompts/tools key off WORKSPACE
```

- **`engine::RunOpts`** gains `env: Vec<(String, String)>`.
- **`DockerCli::run`** emits `-e KEY=VALUE` for each pair (before the image name).
- **`workspace.rs`** builds `RunOpts` via a shared helper so `create()` and the
  `ensure_running()` recreate-from-config path both set it (the recreate path today
  forgets nothing important, but `WORK` must survive a manual `docker rm` + reattach).

This is pure metadata — a name — and changes nothing about isolation (verified by the
`doctor` invariants: still one container, one volume @ `/home/dev`, one network, non-root).

---

## Part 2 — Identity banner (host-side, on every attach)

`Workspace::shell()` emits a compact banner to the terminal immediately before the
interactive tmux attach. Composed by `work` itself from its own invariants plus a single
combined `exec_capture`:

- From `work`'s own state: workspace name, image, "isolated · single-context", home dir.
- From one `docker exec … bash -c` (name, `. /etc/os-release` for `$PRETTY_NAME`,
  `git -C /home/dev rev-parse --abbrev-ref HEAD`): hostname, OS pretty name, git branch.

```
  ╭─ work ────────────────────────────────────╮
  │                                            │
  │    workspace      acme                     │
  │    image          work-lucas:latest        │
  │    system         Debian GNU/Linux 12      │
  │    hostname       219f91abe34e             │
  │    network        isolated · single-context│
  │    home           /home/dev                │
  │    git            main                     │
  │                                            │
  │    isolated container — bring your own tools│
  ╰────────────────────────────────────────────╯
```

Properties:

- **Works for every user** — imported config or default — because it is printed by the
  host process, not sourced inside the shell. Zero image-rebuild dependency; lights up
  existing workspaces on the next `work <ws>`.
- **Printed once per attach** (not per tmux pane), so it is not noisy.
- **Fail-soft:** if the `exec_capture` errors or the container lacks `git`/`os-release`,
  the missing field renders as `—` and the attach still proceeds.
- **Opt-out:** a global config flag `show_banner = false` (default `true`) suppresses it.
- Suppressed alongside the existing hint under `WORK_COCKPIT=1` so cockpit windows stay
  compact (the banner prints on the bare `work <ws>` path; cockpit windows may opt in
  later).

---

## Part 3 — Default workspace prompt (no-import case only)

Replace today's "empty rc" behavior with a small default rc when the user did **not**
import a shell config. `ensure_rc_present` becomes `ensure_default_rc(engine, ctr, rc, imported)`:

- If a shell config was imported, the seeded rc already exists → no-op (verbatim import
  wins, exactly as today).
- If **not** imported and the rc is absent, write a default rc that sets a
  workspace-aware prompt reading `$WORK`:

  zsh (`/home/dev/.zshrc`):
  ```sh
  # Default work prompt. Override: `work new --import-shell-config`.
  setopt PROMPT_SUBST
  PROMPT='%F{magenta}⬡%f %F{cyan}$WORK%f %F{blue}%~%f %# '
  ```

  bash (`/home/dev/.bashrc`):
  ```sh
  # Default work prompt. Override: `work new --import-shell-config`.
  PS1='\[\e[35m\]⬡\[\e[0m\] \[\e[36m\]'"${WORK}"'\[\e[0m\] \[\e[34m\]\w\[\e[0m\] \$ '
  ```

  Rendered: `⬡ acme ~/proj %#`.

- Written via the existing `seed_file` path (temp file → `docker cp` + chown dev), so no
  new engine capability is needed.
- A persisted/edited rc is never overwritten (the existence check already guards this).

For imported-config users (the common case for this user), the prompt stays their own;
the workspace identity comes from the banner, the session/window name, and `$WORK`
(surfaced in their prompt only if they opt into a Starship custom module — see Part 5).

---

## Part 4 — Workspace-named session + titles

The in-container tmux session is renamed from the hardcoded `work` to the workspace name,
and window/tab titles carry it too.

- **`naming::session(ws) -> &str`** centralizes it (returns `ws`). Replaces the literal
  `"work"` at: `Workspace::shell()` (attach), `has_live_session()`, `status()`/`list_all()`
  (the `ls` SESSION column).
- **Attach command** becomes
  `tmux new-session -A -s <ws> -n <ws> -- <shell> -l`
  (`-n <ws>` names the first window).
- **Lossless migration:** the first attach after upgrade detects an old `work` session
  with no `<ws>` session and runs `tmux rename-session -t work <ws>` **in place** —
  running shells/agents inside it are preserved, not killed. `has-session`/`rename-session`
  target the tmux server by name, so this works on an unattached session.
- **Terminal/tab title:** `work` emits the OSC 0/2 sequence `\x1b]0;work:<ws>\x07` before
  attaching (names the terminal tab when not inside a host tmux). Inside tmux the window
  name (`-n`) already carries it. The host cockpit (`work resume`) already names each host
  window after the workspace and is unchanged.

The **host** cockpit tmux session stays named `work` (unchanged) — only the in-container
session is per-workspace.

---

## Part 5 — The `[Docker]` marker (documentation only)

No code suppresses it automatically. `work` ships `$WORK` and documents two opt-in
Starship options the user can add to their `~/.config/starship.toml`:

1. **Disable the engine label** (the marker is redundant once the banner names the
   workspace):
   ```toml
   [container]
   disabled = true
   ```
2. **Show the workspace instead**, via a custom module reading `$WORK`:
   ```toml
   [custom.work]
   command = "echo $WORK"
   when = """ test -n "$WORK" """
   format = '[$output]($style) '
   style = 'bold magenta'
   ```

This keeps prompt ownership with the user, per the bring-your-own principle.

---

## Isolation impact

**None.** Every change is metadata or display:

- `WORK`/`WORKSPACE` are non-secret name strings; they change no mount, network, user,
  or port. `work doctor`'s invariants (unique network, only own volume, non-root, no host
  ports, image match) are untouched.
- The banner is a read-only `docker exec` that prints to the host terminal.
- The default rc is a user-owned file in the workspace's own volume.
- Session renaming is a tmux-server operation local to one container.

No new cross-container path is created; no host bind-mount; no secret is moved or read.

## Files affected

- `crates/core/src/engine.rs` — `RunOpts.env`; `DockerCli::run` emits `-e`; (no trait
  change beyond the struct).
- `crates/core/src/workspace.rs` — shared `run_opts()` helper setting env; `shell()` gains
  banner + OSC title + session rename migration + `-s/-n <ws>`; `has_live_session()`,
  `status()`/`list_all()` use `naming::session`; `ensure_rc_present` →
  `ensure_default_rc(engine, ctr, rc, imported)`.
- `crates/core/src/naming.rs` — `session(ws)`.
- `crates/core/src/config.rs` — `GlobalConfig.show_banner` (default `true`); banner/prompt
  content constants.
- `README.md` — "separated environment" section: banner, `$WORK`, default prompt,
  `[Docker]` opt-in snippet, session-naming note.
- `crates/docker/work-base.Dockerfile` — **unchanged** (no rebuild required).

## Testing

- **Unit (pure):** `naming::session`; banner string composition given a fixed
  `(name, image, system, hostname, git)` tuple (pure function over inputs).
- **End-to-end (manual milestone):** `work new demo2` → attach → assert banner prints,
  prompt shows `⬡ demo2`, `echo $WORK` == `demo2`, `tmux ls` shows `demo2`, terminal tab
  named `work:demo2`. Then verify the migration path: create a session under the old
  `work` name, attach, confirm it is renamed in place (running process survives).
- **Regression:** existing `test1`/`demo` workspaces still attach after upgrade
  (migration rename fires once, losslessly).

## Risks / open questions

1. **One-time session rename** changes the session identity existing users may script
   against (`tmux attach -t work`). Mitigated by lossless in-place rename; documented as a
   breaking-but-migrated change.
2. **Banner latency:** one extra `docker exec` per attach (~0.1–0.3s). Acceptable; fail-soft.
3. **Starship `[Docker]`** remains by default — intentional (prompt ownership stays with
   the user); documented as opt-out.

## Out of scope

- Auto-editing the user's Starship/shell config.
- A baked-in banner script in the base image (host-side printing avoids the rebuild).
- Persistent banner-content customization (fields are fixed; `show_banner` toggle only).
- Renaming the **host** cockpit tmux session (stays `work`).
