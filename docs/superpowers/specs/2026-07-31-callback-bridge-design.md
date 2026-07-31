# `work browse` callback bridge: auto-forward the OAuth callback port

**Date:** 2026-07-31
**Status:** Approved (direction)
**Goal:** Make `work browse` complete an entire OAuth/subscription login by itself — it opens the auth URL in the host browser *and* automatically bridges the loopback callback port so the provider's redirect reaches the in-container listener. No separate `work fwd`.

## Background

`work browse` (shipped) forwards in-container `xdg-open` calls to the host browser. That solves the *initial* open, but OAuth logins (Claude Code, Cursor, `gh`, …) also need the **callback**: the CLI starts a listener on `127.0.0.1:<port>` inside the container and registers `redirect_uri=http://localhost:<port>/callback`. After the user authenticates, the provider redirects the **host** browser to `http://localhost:<port>/callback` — but that `localhost` is the host's loopback, where nothing listens, so the callback never reaches the container and the login hangs.

**Why we bridge instead of rewriting the host:** OAuth providers validate `redirect_uri` and, for native apps, lock it to a **loopback** host (`localhost`/`127.0.0.1`/`::1`, any port — RFC 8252). Rewriting `localhost` to another host would make the provider reject the mismatch. So the host in the URL must stay loopback; the fix is to bridge host `127.0.0.1:<port>` → the container's `<port>` — exactly what `work fwd` does, but automatically and only for the callback port.

## Approach

`work browse` parses each forwarded URL for a loopback `redirect_uri` port and auto-starts a forwarder for it *before* opening the URL. Reuses the existing socat bridge pattern. Tool-agnostic (standard loopback OAuth).

## Components

### 1. `browser::callback_port(url) -> Option<u16>` (pure, unit-tested)
Parse the URL with the `url` crate; find the first `redirect_uri` query param (`query_pairs()` already percent-decodes); parse its value as a URL; return its port **iff** scheme is `http` and the host is loopback (`Ipv4`/`Ipv6` `is_loopback()`, or domain `localhost`). Otherwise `None` (non-loopback, absent, no port → device-code/non-OAuth flows need no bridge).

### 2. `Engine::spawn_forwarder` (non-blocking sibling of `run_forwarder`)
Same socat bridge args as `run_forwarder` (`host 127.0.0.1:<port>` → `target:<port>` on the workspace network, `alpine/socat`, `--rm`), but spawned with `.spawn()` + null stdio and the child handle **dropped** — so the call returns immediately instead of blocking. The forwarder runs as a foreground child in `work browse`'s process group.

### 3. `workspace::browse` auto-bridge
The read loop, before opening a URL:
- `callback_port(url)` → `Some(port)` and not already bridged (a `HashSet<u16>`) → `spawn_forwarder("work-browse-<ws>-<port>", net, port, ctr, port)`; on success record the name in a `Vec<String>` and print `· bridged callback port <port>`; on failure (e.g. host port busy) warn + continue (the URL still opens; user may `work fwd` manually).
- Then open the URL in the host browser as today.

**Cleanup (no leaked containers):**
- **Ctrl-C** → SIGINT to the foreground process group → each forwarder's `docker run --rm` stops + removes the container (identical to how `work fwd` cleans up — proven, no signal-handler dependency).
- **Error exit** → `browse` explicitly `remove_container`s every forwarder name it tracked, then returns the error.

### 4. Dependency: promote `url` to a direct `work-core` dep
`url` 2.5.8 is already in `Cargo.lock` (transitive via `ureq`), so this adds no new download — just a direct dependency for clean parsing.

## Isolation impact
**None.** Forwarders are separate containers (`work-browse-<ws>-<port>`) on the workspace's own bridge network with a `127.0.0.1` host port — exactly the established `work fwd` shape, which `work doctor` already tolerates (doctor inspects only `work-<ws>` workspace containers, not forwarders). No new host-gateway/mount/user/restart change.

## Files affected
- Modify: `crates/core/Cargo.toml` + root `Cargo.toml` — add `url` to `[workspace.dependencies]` and `url.workspace = true` in work-core.
- Modify: `crates/core/src/browser.rs` — `callback_port` (+ `use url::{Url, Host}`) + unit tests.
- Modify: `crates/core/src/engine.rs` — `Engine::spawn_forwarder` (+ `DockerCli` impl).
- Modify: `crates/core/src/workspace.rs` — `browse` auto-bridge + cleanup.
- Modify: `README.md` (note auto-callback-bridge in the OAuth section), `CHANGELOG.md`.

## Testing
- Unit (pure): `callback_port` — encoded `redirect_uri=http://localhost:8080/cb` → `Some(8080)`; `127.0.0.1:9000` → `Some(9000)`; `[::1]:7000` → `Some(7000)`; non-loopback (`example.com`) → `None`; no `redirect_uri` → `None`; no port → `None`.
- Smoke (real OrbStack): a login-shaped URL with `redirect_uri=http://localhost:<port>` → `work browse` prints `· bridged callback port <port>`, a request to host `127.0.0.1:<port>` reaches a listener in the container; Ctrl-C leaves no `work-browse-*` containers (`docker ps -a` clean); a non-OAuth URL opens without bridging.

## Decisions
- **Foreground child + process-group cleanup over a `ctrlc` handler:** matches `work fwd`'s proven cleanup, avoids a new signal-handling dependency.
- **Same port both sides:** loopback OAuth listens on the `redirect_uri` port, so the bridge uses that port on host and container.
- **Auto, no flag:** non-login URLs are unaffected (no `redirect_uri`); port conflicts warn-and-continue rather than fail.

## Out of scope
- Tearing down a bridge after its specific callback fires (forwarders live until `work browse` stops — matches `work fwd` UX).
- A `--no-bridge` opt-out (port-conflict warn-and-continue covers the failure mode).
- Non-loopback / non-RFC-8252 redirect flows (they return `None` and just open).
