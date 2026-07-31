# `work browse`: forward in-container browser-open requests to the host browser

**Date:** 2026-07-31
**Status:** Approved (direction)
**Goal:** When a tool inside a workspace container (Claude Code, Cursor CLI, `gh`, Python `webbrowser`, …) tries to open a URL for an OAuth/subscription login, that URL opens in the user's real host browser — automatically, with no manual copy-paste.

## Background

Containers have no browser and no `xdg-open`; a tool's `xdg-open <url>` call (or `$BROWSER` lookup) fails silently. `work fwd` only bridges a host→container *port* for a callback; it does nothing for the *initial* browser-open, and it requires the user to already have the URL. There is no container→host channel today: workspace containers live on isolated bridge networks with no route to the host, and `work doctor` enforces "no published host ports, own network only, no extra mounts."

## Approach

An explicit, opt-in, foreground bridge command — `work browse <ws>` — mirroring the existing `work fwd` pattern. The container→host channel is a **named pipe (FIFO) in the workspace volume**: it adds no network surface, is portable across every supported engine (OrbStack/Docker/Colima/Podman), and leaves every `work doctor` invariant untouched.

Rejected alternatives: (a) a host HTTP listener reached via `--add-host=host.docker.internal:host-gateway` — adds a container→host network path (philosophy tension) plus engine-specific bind/port portability issues; (b) auto-starting the bridge as a background child of `work <ws>` — more lifecycle complexity (child spawn/teardown, cockpit spawns N, signal handling), harder to debug. Both may be revisited later; this spec ships the explicit command.

## Components

### 1. In-container shim — `/usr/local/bin/xdg-open` (root-owned, `0755`)
POSIX `sh`. Installed system-wide so every tool finds it regardless of rc/profile:
- Selects the first argument matching `http(s)://*`; **no-ops (exit 0)** for anything else so calls on files/dirs don't break tools.
- Writes the URL line to the FIFO `$HOME/.work/browser.fifo` via `timeout 5 sh -c 'printf "%s\n" "$url" > "$fifo"'` — non-blocking, so a tool is never stuck if no bridge is running.
- **Always also echoes** `🌐 <url>` to the terminal, so even without the bridge the URL is visible instead of "command not found".
- Symlinks `/usr/local/bin/{sensible-browser,x-www-browser}` → `xdg-open` (covers Python `webbrowser` and Debian-alternatives callers).

### 2. `$BROWSER` env (most robust interception)
`run_opts` gains `("BROWSER", "/usr/local/bin/xdg-open")`. Set as container env (`-e`) so it is present for every process — beats relying on rc/profile sourcing. Node's `open`, Claude Code, and most CLIs check `$BROWSER` first.

### 3. Shim install — idempotent `Workspace::install_browser_shim()`
Writes the shim + symlinks + `/etc/profile.d/work-browser.sh` (belt-and-suspenders `export BROWSER=…`) as root. Called from:
- `Workspace::create()` — new workspaces ship with it.
- the start of `work browse` — so **existing workspaces** get it on first use, with **no image rebuild**.

The shim lives in the container writable layer and is re-installed on recreate. The install runs as root via one new `Engine` method — `exec_root(name, cmd)` (`docker exec --user root …`, success-required, stderr reported on failure) — mirroring the existing `--user root` pattern in `seed_file`/`seed_dir`. The existing `seed_file` is unsuitable here because it chowns to `dev` and can't `chmod`/symlink; `exec_root` lets `install_browser_shim` write the file, `chmod 0755`, `ln -sf` the symlinks, and drop the profile.d snippet in one `sh -c`.

### 4. `work browse <ws>` — `Workspace::browse()` (foreground, mirrors `work fwd`)
1. `ensure_running()` → `install_browser_shim()` → ensure the FIFO exists (`mkdir -p ~/.work`; `mkfifo` if absent; `chown dev:dev`).
2. Print: `Browsing for <ws> — URLs tools open will launch in your host browser. Ctrl-C to stop.`
3. Loop: `docker exec <ctr> cat /home/dev/.work/browser.fifo` (blocks until the shim writes a line) → for each non-empty line, validate `http(s)` → open via the **host opener** → print `↗ opened <url>`.
4. Ctrl-C kills the foreground `docker exec`, the loop exits, print `stopped`. The FIFO persists in the volume for next time.

### 5. Host opener (pure, testable)
`host_opener_for(os) -> &'static str`: `open` on macOS, `xdg-open` on Linux; an explicit `$WORK_HOST_BROWSER` override wins if set and the binary exists. Open is fire-and-forget (`Command::new(opener).arg(url).status()`); `work browse` never blocks on the browser.

## Isolation impact
**None.** No new network, no host-gateway, no published ports, no extra mounts, no change to user/restart/image. The shim is a system file; the FIFO is a user-owned node in the workspace's own volume. `work doctor` invariants are unchanged and stay green.

## CLI wiring
- New subcommand `Browse { ws: String }` with help text paralleling `Fwd`.
- Add `"browse"` to both reserved-token lists: `crates/cli/src/main.rs::RESERVED` and `crates/core/src/naming.rs::RESERVED`.
- `commands::browse(ws)` → `workspace::browse(ws)`.

## Files affected
- Modify: `crates/core/src/engine.rs` — add `Engine::exec_root` (+ `DockerCli` impl).
- Modify: `crates/core/src/workspace.rs` — `run_opts` (`BROWSER` env), `install_browser_shim()`, `browse()`, call install from `create()`; shim script `const`.
- Modify: `crates/core/src/lib.rs` — re-export `browse` if needed by the cli crate (match existing export style).
- Modify: `crates/cli/src/commands.rs` — `browse()`.
- Modify: `crates/cli/src/main.rs` — `Browse` variant + `RESERVED`.
- Modify: `crates/core/src/naming.rs` — `RESERVED`.
- Modify: `README.md` (OAuth section + CLI table), `CHANGELOG.md`.

## Testing
- Unit (pure): `host_opener_for(os)` → `open`/`xdg-open`, `$WORK_HOST_BROWSER` override; `is_openable_url()` http/https filter (used for the host-side defense; the shim re-implements the same filter in `sh`).
- Smoke (manual, end-to-end): `work new demo` → in a second terminal `work browse demo` → inside the container `xdg-open https://example.com` → confirm the host browser opens it and `work browse` prints `↗ opened …`; Ctrl-C stops cleanly. Repeat against an **existing** workspace (pre-feature) to confirm first-run install.

## Decisions
- **Explicit command over auto-on-attach:** consistent with `work fwd` and the project's opt-in-bridge philosophy; zero background processes.
- **FIFO over network listener:** portable, isolation-neutral, doctor-clean.
- **Echo as well as forward:** graceful degradation — with no bridge running the URL is still visible, never silently lost.
- **`timeout 5` on the FIFO write:** bounds a tool's `xdg-open` call so it can't hang indefinitely when no bridge is attached.

## Out of scope
- Auto-starting the bridge on attach (future enhancement; possibly a `browser_bridge` config flag).
- A shared host daemon or host-gateway network path.
- Opening non-`http(s)` targets (files, dirs, `mailto:`) — intentionally no-op'd.
- A `work doctor` check for the shim (YAGNI; the shim is best-effort, not an isolation invariant).
