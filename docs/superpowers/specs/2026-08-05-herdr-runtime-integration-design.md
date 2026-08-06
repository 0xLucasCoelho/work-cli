# herdr as the in-container runtime — design

- **Date:** 2026-08-05
- **Status:** Verifier-reviewed (findings incorporated); ready for implementation
- **Supersedes (partially):** `2026-07-28-work-sessions-design.md` (the in-container tmux session model)
- **Related:** `2026-07-28-work-cli-design.md`, `2026-08-03-work-update-config-design.md`, `2026-08-03-completion-and-tui-dashboard-design.md`

## 1. Context

`work` self-describes (README:12) as a **"session + isolation manager."** Those are two
jobs, and only one of them is `work`'s genuine differentiator:

- **Isolation** — one container + named volume + dedicated bridge network per workspace,
  non-root `dev` user, `work doctor` invariants, `fwd`/`browse` shims. This is `work`'s moat.
  No other tool in this space does it; herdr's own docs state *"herdr does not sandbox your
  agents … isolation is still on you."*
- **Session/multiplexing** — an in-container tmux server per workspace, `work tab`/`tabs`,
  and a host "cockpit" (`work resume`). This is a homegrown, weaker reimplementation of what
  [herdr](https://herdr.dev/) now does **better** (agent lifecycle awareness —
  `working`/`blocked`/`idle`/`done` — that our tmux layer never had).

The tmux layer is hardcoded as string literals across `engine.rs`, `workspace.rs`, the base
image, and `templates/`. There is no multiplexer abstraction.

## 2. Goals

- **Delegate multiplexing to herdr; keep isolation as `work`'s only terminal-side job.**
- Preserve every existing isolation guarantee (`work doctor` invariants untouched).
- Make `work <ws>` drop the user into the workspace's herdr with no homegrown session model.
- Shrink the codebase: delete the bespoke tmux orchestration rather than wrap it.

## 3. Non-goals

- A unified **cross-workspace** host view (herdr sidebar spanning all containers). This is
  *Option C* (host aggregation) and is explicitly deferred — see §10.
- Supporting multiple multiplexer backends behind a trait. We are replacing tmux, not
  abstracting it. (A trait with one impl is dead weight.)
- Changing anything about volume/network/container isolation, identity env, `fwd`/`browse`,
  image build, or dotfiles import (except the tmux-config slot).

## 4. Decision

**Topology: Option A — one herdr runtime per container.** Each workspace container ships the
herdr binary and runs its own headless `herdr server`; `work <ws>` attaches a client into it.

**Scope: Thin.** `work` exits the multiplexing business. `work <ws>` is the only attach
command; tabs/panes/agent-state are herdr's native UX. We do not wrap herdr's API.

### Why A over host-herdr (B) or hybrid (C)
- A is 1:1 with `work`'s isolation guarantee: each workspace is a fully isolated herdr
  environment (own server, own sidebar, own panes). B co-locates runtime state on the host
  (weaker isolation) and its agent-detection across a `docker exec` PTY is unverified.
- A needs no nested multiplexer (today's cockpit is host-tmux nesting in-container-tmux).
- C (host aggregation on top of A) remains available later without rework.

### Experiment evidence (2026-08-05, herdr 0.8.0, linux/aarch64 container)
Ran a minimal `ubuntu:24.04` container mirroring `work-base` (dev user, `ncurses-term`,
UTF-8 locale) with the statically-linked herdr binary:

1. **Headless server runs detached in-container.** `docker exec -d <ctr> herdr server` →
   `herdr status server` reports `status: running`, socket at
   `/home/dev/.config/herdr/herdr.sock`. Process persists (PID 11) independent of clients.
2. **Programmatic socket API works in-container and returns JSON.**
   `herdr workspace create --label acme --no-focus` →
   `{"result":{"workspace":{...,"workspace_id":"w1"}, "tab":{"tab_id":"w1:t1"},
   "root_pane":{"pane_id":"w1:p1"}}}` — richer and cleaner to parse than tmux's
   `list-windows -F`.
3. **State persists across separate exec clients.** A workspace created in one `docker exec`
   is still present in a subsequent `docker exec herdr workspace list`. This is the
   tmux-equivalent guarantee (server holds state independent of clients) — **the load-bearing
   assumption for Option A, confirmed.**
4. **Socket subcommands do NOT auto-spawn the server** — they return `server_not_running`
   with the hint *"run `herdr` to start or attach it."* So the bare `herdr` command is the
   start-or-attach launcher (the `new-session -A` equivalent); programmatic ops require an
   already-live server.
5. **`herdr session list` works with the server down** (reads on-disk session metadata:
   `default` session + socket path) — so `work ls`'s SESSION column can query liveness
   cheaply without spawning a server.

## 5. Architecture (after)

```
Host                          Container (one per workspace)
┌───────────────────┐         ┌─────────────────────────────────────┐
│ work <ws>         │ docker  │ dev@/home/dev  (volume, own network)│
│  = attach client  │ exec -it│                                     │
│                   │────────▶│ herdr server (headless daemon)      │
│ work ls           │         │   └ workspaces / tabs / panes       │
│  = status probe   │ docker  │   └ agent lifecycle awareness       │
│                   │ exec    │                                     │
└───────────────────┘         │ herdr (TUI client) ← attached user   │
                              └─────────────────────────────────────┘
```

`work` owns the container boundary. herdr owns everything inside the terminal.

## 6. Detailed design

### 6.1 Lifecycle mapping

| `work` command | Today (tmux) | After (herdr) |
|---|---|---|
| `work <ws>` (attach) | `exec_interactive(tmux new-session -A -s <ws> -n <ws> -- <shell> -l)` (`workspace.rs:280-298`) | `exec_interactive(herdr)` — bare `herdr` launches server+TUI on first run, attaches thereafter |
| `work tab <ws>` | tmux `new-window` dance (`workspace.rs:309-412`) | **Removed.** Run `work <ws>` again to attach a second client (herdr is multi-client). |
| `work tabs <ws>` | `tmux list-windows -F` (`workspace.rs:414+`) | **Removed.** Use herdr's sidebar / `herdr tab list` inside. |
| `work ls` SESSION col | `session_exists` → `tmux has-session` (`engine.rs:423`) | `runtime_up` → `herdr status server` (`running` / `not running`) |
| `work resume` / `work all` | host-tmux cockpit (`workspace.rs:944-1025`) | **Removed** (deferred to Option C). |
| `work stop` | kills tmux server | kills herdr server; volume state persists; herdr *expected* to restore on next attach (unverified — §8 #2, tested §9.3) |

### 6.2 `Engine` trait (`engine.rs`)

- **Rename `session_exists(name, session) -> Result<bool>` → `runtime_up(name) -> Result<bool>`.**
  The `session` argument is meaningless under herdr (one `default` session per container).
  Keep the lenient semantics: return `false` (not an error) when the container is
  missing/stopped, so `ls`/`stop`/`rm` never choke on a downed box.
- **Implementation** (`engine.rs:420-427`): replace
  `exec … tmux has-session -t <session>` with `exec … herdr status server` and parse for
  `status: running`. (Non-zero exit / no socket ⇒ `false`.)
- `exec_interactive` / `exec_capture` / `exec_root` are unchanged — they are generic
  `docker exec` wrappers, not tmux-specific.
- **Surviving callers of the renamed method (must UPDATE, not delete) — both are build-breaks
  if missed (verifier finding):** `Workspace::has_live_session()` (`workspace.rs:530-541`) is
  the destructive-op liveness gate (stop/rm/recreate confirms via `commands.rs` + `tui/mod.rs`)
  and calls both the renamed `session_exists` and the deleted `naming::session` — rewrite to
  `self.engine.runtime_up(&ctr).unwrap_or(false)` (the `container_state` precheck can stay or
  drop; `runtime_up` already returns false when stopped). `list_all()` (`workspace.rs:824-827`,
  the `work ls` SESSION column) likewise — rewrite to `engine.runtime_up(&ctr)`.

### 6.3 `shell()` = `work <ws>` (`workspace.rs:247-287`)

Becomes:

```text
ensure_running()?
ctr = container(name)
banner + detach hint   // hint text: "Ctrl-b q = detach (keeps running) · …"
set terminal title     // unchanged: \x1b]0;work:<ws>\x07
exec_interactive(ctr, ["herdr"])
```

- Drop `migrate_session_name` (`workspace.rs:291-300`) and its call — tmux-specific rename
  with no herdr analogue (herdr's session is always `default`).
- Drop the `shell`/`session`/`name` derivation used only for tmux targeting.
- The detach hint changes from `Ctrl-b d` to `Ctrl-b q` (herdr's detach key; herdr's prefix
  is also `Ctrl-b`, so muscle memory carries).

### 6.4 Removals (Thin)

Delete outright — no shims, no re-exports:

| Symbol / file | Location |
|---|---|
| `Workspace::tab` | `workspace.rs:309-412` |
| `Workspace::list_tabs` (`work tabs`) | `workspace.rs:414+` |
| `Workspace::windows` (pub DTO accessor) | `workspace.rs:487+` |
| `Workspace::tmux_windows` (private helper) | `workspace.rs:473+` |
| `Workspace::migrate_session_name` | `workspace.rs:291-300` |
| `Workspace::resume` (cockpit) + `cockpit_cmd` | `workspace.rs:944-1033` |
| `which_host` (only used by cockpit) | `workspace.rs:1035-1042` |
| `TMUX_CONF_DEFAULT` + `ensure_default_tmux_conf` | `workspace.rs:1112-1153` |
| `validate_window_name` | `workspace.rs:1159-1173` |
| `WindowRow` + `parse_window_line` | `workspace.rs:1175-1197` |
| `naming::session` (tmux session-name helper) | `naming.rs:24-28` |
| `templates/.tmux.conf` | `templates/` |
| tmux-config import plumbing | `commands.rs`, `main.rs`, `config.rs`, **`workspace.rs`** (`create` + `update` import paths) |

In `create()` (`workspace.rs:67-240`): **replace** the `"tmux"` seed tuple (`:107-113`) with a
herdr-config seed tuple (src `--import-herdr-config` path or `.config/herdr/config.toml`, dest
`/home/dev/.config/herdr/config.toml`, kind `"herdr"`); rename the `import_tmux` param (`:73`)
→ `import_herdr`. Drop only the tmux-specific `tmux_imported` tracking (`:190`) and the
`ensure_default_tmux_conf` call (`:191`) — herdr ships sensible defaults, so no
`ensure_default_herdr_conf` is needed. (This *replaces*, not drops, the seed tuple — otherwise
`--import-herdr-config` would be a silent no-op in `create()`.)

**TUI dashboard tab feature (resolved downstream-consumer finding).** The ratatui dashboard
(`crates/cli/src/tui/`, per `2026-08-03-completion-and-tui-dashboard-design.md`) consumes
`WindowRow` + `Workspace::windows()` to render an expandable per-workspace tab list
(`app.rs:8,50,247,255`; `mod.rs:109 refresh_tabs`). Tabs are now herdr's job, so this feature
is **removed, not rewired**: the dashboard becomes a **workspace selector** only (list + state
+ attach). Concretely drop:

| TUI symbol | Location |
|---|---|
| `App.tabs` / `App.expanded` fields + `expanded_tabs()` / `set_tabs()` + the tab part of `toggle_expand()` | `tui/app.rs` |
| `refresh_tabs()` + the background tabs-fetch worker | `tui/mod.rs` |
| expanded-tabs rendering in `render::render` | `tui/render.rs` |
| the `WindowRow` import + its `toggle_expand` unit test | `tui/app.rs:8,318-330` |

The workspace-selector core (`App`, `WorkspaceStatus` model, list render, attach action) stays.

### 6.5 Base image (`crates/docker/work-base.Dockerfile` + `image.rs::DEFAULT_DOCKERFILE`)

- Remove `tmux` from the `apt-get install` line (`work-base.Dockerfile:6`).
- Add the herdr binary. It is **statically linked** (no glibc dependency), so install via a
  pinned release fetch in the Dockerfile:

  ```dockerfile
  ARG HERDR_VERSION=0.8.0
  ARG HERDR_ARCH=aarch64
  RUN curl -fsSL -o /usr/local/bin/herdr \
        "https://github.com/herdrdev/herdr/releases/download/v${HERDR_VERSION}/herdr-linux-${HERDR_ARCH}" \
      && chmod +x /usr/local/bin/herdr
  ```

  Multi-arch build (`aarch64`/`x86_64`) selects `HERDR_ARCH` per platform. Pin
  `HERDR_VERSION` (herdr is pre-1.0; see §8).
- Keep `zsh bash curl jq build-essential sudo ncurses-term locales` — herdr needs terminfo
  and a UTF-8 locale exactly as tmux did.
- The `TERM_PROGRAM=tmux` / Nerd-Font comments in `image.rs` and `engine.rs` (see §6.7)
  still apply: agents mis-detect inside *any* multiplexer, so `NERD_FONTS=1` stays.

### 6.6 CLI surface (`commands.rs`, `main.rs`, `tui/`, `naming.rs`)

- Remove the `Tab`, `Tabs`, and `Resume` subcommands (and the `work all` alias) from
  `Command` (`main.rs`) and their dispatch + `commands::tab` / `tabs` / `resume` wrappers.
- The bare-`work` **dashboard stays** as a workspace selector (it is an isolation/workspace
  affordance, not a tmux feature), minus its tab-expansion (§6.4).
- Replace `--import-tmux-config` / `import_tmux_config` with `--import-herdr-config` /
  `import_herdr_config`, seeding to `/home/dev/.config/herdr/config.toml` (same secret-free
  warning). This is a **three-site sweep** (omitting any is a build-break): the
  `GlobalConfig.import_tmux_config` field (`config.rs:22`), `Workspace::create()` param +
  seed tuple (`workspace.rs:73,107-113`), AND `Workspace::update()` (`workspace.rs:606`
  param `import_tmux` → `import_herdr`; `:619-620` read `global.import_herdr_config` +
  reseed `.config/herdr/config.toml`).
- Reword **surviving** command doc-comments tmux→herdr: `Ls` (`main.rs:44-46`) and `Update`
  (`main.rs:172`). The `Tab`/`Tabs`/`Resume` docstrings (`:65-70,120,133,137`) are deleted
  with their commands, not reworded.
- `naming::RESERVED` currently includes `tab`, `tabs`, `resume`, `all` (`naming.rs:9-10`).
  Leave them reserved so a workspace cannot silently shadow a future command name.
- **README.md** is in the change set: drop the `work tab`/`work tabs` reference rows
  (`:335-336`), rename `--import-tmux-config` → `--import-herdr-config` (`:174,333,369`),
  reword the "persistent in-container session" / `Ctrl-b d` detach + cockpit sections
  (`:127-164,334`) to herdr.

### 6.7 Comment / doc corrections (TERM_PROGRAM, NERD_FONTS)

The `NERD_FONTS=1` / `COLORTERM` forwarding rationale lives in three spots, all reworded
tmux→herdr (the rationale is identical — agents can't see the host terminal from inside *any*
multiplexer): the `run_opts` docstring (`workspace.rs:776-782`, the docker-run-time
rationale), `engine.rs` (`:372, :641-648`, the per-exec forwarding), and the `image.rs`
terminfo/locale *test* comment (`:132-137` — this is the `tmux-256color`/nested-tmux terminfo
comment, NOT the NERD_FONTS rationale). Behavior (`NERD_FONTS=1`, `COLORTERM` forwarding) is
unchanged.

## 7. Compatibility / migration

- **Breaking for existing workspaces.** Containers built from the old image have tmux, not
  herdr. After upgrading `work`, `work new` builds the herdr image; **existing containers
  must be recreated** (`work rm <ws>` keeps the volume; `work new <ws>` rebuilds). Volume
  contents (repos, credentials, installed tools) survive — only the installed multiplexer
  binary changes. Call this out in the CHANGELOG and the upgrade hint.
- A workspace that still has a stale `.tmux.conf` in its volume is harmless (unused); `work
  update`'s managed-set should drop `.tmux.conf` and add `.config/herdr/config.toml`.
- The dashboard's expandable-tabs view is removed; tabs now live in herdr's in-container
  sidebar (visible once you `work <ws>`).

## 8. Risks & open questions

1. **herdr is pre-1.0 (v0.8.0).** Socket-API JSON shapes could shift between releases.
   Mitigation: pin `HERDR_VERSION` in the image; `work` drives only two surfaces
   (`herdr status server` for liveness, and bare `herdr` for attach) — minimal exposure.
2. **Session restore across `work stop` / `work start` is unverified.** Confirmed the server
   holds state across *clients*; did **not** confirm herdr restores panes/agents after a full
   server kill+restart. `[INFERENCE]` likely fine (herdr documents session-state restore);
   **must be tested before shipping** (§9).
3. **Interactive TUI attach** (`docker exec -it herdr`) is confirmed by herdr's design and
   its own error messaging but was not re-exercised without a real TTY in the experiment.
   `[INFERENCE]` — verify in §9.
4. **`work doctor` has no tmux dependency** *(resolved during spec self-review)*. `doctor.rs`
   checks only isolation invariants (networks, mounts, user, ports, restart-policy, image);
   grep found no tmux reference in `doctor.rs`. Only the `image.rs:63-64` docstring
   ("tmux/zsh/bash") needs rewording to "herdr/zsh/bash".
5. **`--import-tmux-config` → `--import-herdr-config`** is a flag rename (breaking). Acceptable
   for a pre-1.0 `work`; document in CHANGELOG. Alternative: keep the name but seed herdr
   config — rejected as misleading.

## 9. Verification plan

1. **TUI attach** — `work new htest`, then `work htest` lands in the herdr TUI; detach
   (`Ctrl-b q`) and re-`work htest` reattaches to the same server/layout.
2. **Persistence across clients** — start an agent in one `work htest`, open a second
   `work htest`, confirm both see the same layout (multi-client).
3. **Session restore across stop/start** — `work htest`, start a long process, `work stop
   htest`, `work start htest`, `work htest` — confirm herdr restores (or document the
   limitation). This closes risk #2.
4. **`work ls` SESSION column** — shows `live` while attached, `—` after `work stop`.
5. **Full lifecycle** — `work new`/`work <ws>`/`work ls`/`work stop`/`work start`/`work rm`
   end-to-end on OrbStack; `work doctor` passes.
6. **Existing unit tests** — update/remove tmux-specific tests (`crates/core/tests/config.rs`
   tmux-config assertions; `workspace.rs` `mod tests` tmux cases). The pure helpers being
   deleted take their tests with them.

## 10. Out of scope (future)

- **Option C — host aggregation.** A host-level herdr (or a `work resume` rewrite) that
  peers into each container's herdr server (`herdr --remote`) to give one cross-workspace
  sidebar. Build only if the per-container sidebar proves insufficient.
- **herdr agent integrations** (`herdr integration install claude`, etc.) baked into the
  image for better lifecycle detection — a later enhancement, not required for the swap.
