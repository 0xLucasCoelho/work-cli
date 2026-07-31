# `work browse` Callback Bridge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `work browse` auto-bridge the OAuth loopback callback port (parsed from the login URL's `redirect_uri`) so a full login completes with one command.

**Architecture:** `work browse` reads each URL from the FIFO; a pure `callback_port(url)` extracts the loopback redirect port; a new non-blocking `Engine::spawn_forwarder` starts a socat bridge (`host 127.0.0.1:<port>` → `container:<port>`) as a foreground child in the process group; cleanup is via Ctrl-C (process-group SIGINT → `--rm`) plus explicit `remove_container` on error exit.

**Tech Stack:** Rust 2021, `anyhow`, the `url` crate (already transitive via `ureq`), the `work-core` `Engine` trait.

## Global Constraints
- Isolation invariants stay green: forwarders are separate `work-browse-<ws>-<port>` containers on the workspace's own network (same shape as `work fwd`, which `work doctor` already tolerates). No host-gateway/mount/user/restart change.
- No new signal-handling dependency — cleanup reuses the process-group SIGINT model proven by `work fwd`.
- Work in the worktree `.worktrees/feat-callback-bridge` (branch `feat-callback-bridge`), based on `main`.
- Re-read each file freshly before editing (line numbers are a guide, not gospel).

## File Structure
- Modify: root `Cargo.toml` + `crates/core/Cargo.toml` — `url` dependency.
- Modify: `crates/core/src/browser.rs` — `callback_port` + tests.
- Modify: `crates/core/src/engine.rs` — `Engine::spawn_forwarder` + impl.
- Modify: `crates/core/src/workspace.rs` — `browse` auto-bridge + cleanup + `browse_loop` helper + `HashSet` import.
- Modify: `README.md`, `CHANGELOG.md`.

---

### Task 1: Promote `url` to a direct `work-core` dependency

**Files:** root `Cargo.toml` (`[workspace.dependencies]`), `crates/core/Cargo.toml` (`[dependencies]`).

- [ ] **Step 1: Add to workspace deps**

In root `Cargo.toml` `[workspace.dependencies]`, add (e.g. after the `tempfile` line):
```toml
url = "2"
```

- [ ] **Step 2: Add to work-core deps**

In `crates/core/Cargo.toml` `[dependencies]`, add:
```toml
url.workspace = true
```

- [ ] **Step 3: Verify it resolves (no new download)**

Run: `cargo build -p work-core 2>&1 | tail -3`
Expected: builds (uses already-present `url` 2.5.8).

- [ ] **Step 4: Commit**
```bash
git add Cargo.toml crates/core/Cargo.toml
git commit -m "chore(core): promote url to a direct work-core dependency"
```

---

### Task 2: `browser::callback_port` (TDD)

**Files:** `crates/core/src/browser.rs`

- [ ] **Step 1: Write the failing tests**

In the `#[cfg(test)] mod tests` block in `browser.rs`, add:
```rust
    #[test]
    fn callback_port_from_login_url() {
        let url = "https://provider.example/oauth/authorize?client_id=x&redirect_uri=http%3A%2F%2Flocalhost%3A8080%2Fcallback&code_challenge=y";
        assert_eq!(callback_port(url), Some(8080));
        assert_eq!(
            callback_port("https://p/a?redirect_uri=http://127.0.0.1:9000/cb"),
            Some(9000)
        );
        assert_eq!(
            callback_port("https://p/a?redirect_uri=http://[::1]:7000/cb"),
            Some(7000)
        );
    }

    #[test]
    fn callback_port_none_when_not_loopback_or_absent() {
        assert_eq!(callback_port("https://p/a?redirect_uri=https://example.com/cb"), None);
        assert_eq!(callback_port("https://p/a?redirect_uri=http://example.com:8080/cb"), None);
        assert_eq!(callback_port("https://p/a"), None);
        assert_eq!(callback_port("https://p/a?redirect_uri=http://localhost/cb"), None);
        assert_eq!(callback_port("not a url"), None);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p work-core callback_port 2>&1 | tail -5`
Expected: compile error — `callback_port` not defined.

- [ ] **Step 3: Implement**

At the top of `browser.rs` change the `use` to include `url`, and add the fn near `is_openable_url`:
```rust
use url::{Host, Url};
```
```rust
/// If `raw_url` is a login URL carrying a loopback `redirect_uri` (RFC 8252),
/// return the callback port to bridge so the host browser's redirect reaches
/// the in-container listener. `None` for non-loopback / absent / no-port. PURE.
pub fn callback_port(raw_url: &str) -> Option<u16> {
    let url = Url::parse(raw_url).ok()?;
    let redirect = url
        .query_pairs()
        .find(|(k, _)| k == "redirect_uri")
        .map(|(_, v)| v.into_owned())?;
    let r = Url::parse(&redirect).ok()?;
    if r.scheme() != "http" {
        return None;
    }
    let loopback = matches!(r.host(), Some(Host::Ipv4(ip)) if ip.is_loopback())
        || matches!(r.host(), Some(Host::Ipv6(ip)) if ip.is_loopback())
        || matches!(r.host(), Some(Host::Domain(d)) if d == "localhost");
    if !loopback {
        return None;
    }
    r.port()
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p work-core callback_port 2>&1 | tail -4`
Expected: `test result: ok. 2 passed`.

- [ ] **Step 5: Commit**
```bash
git add crates/core/src/browser.rs
git commit -m "feat(core): browser::callback_port (parse loopback OAuth redirect port)"
```

---

### Task 3: `Engine::spawn_forwarder`

**Files:** `crates/core/src/engine.rs`

- [ ] **Step 1: Add the trait method**

Add to the `Engine` trait immediately after `run_forwarder`:
```rust
    /// Like `run_forwarder` but non-blocking: spawn the bridge as a detached
    /// child (in the caller's process group) and return immediately. Used by
    /// `work browse` to auto-bridge OAuth callback ports without blocking its
    /// read loop. Cleanup is via Ctrl-C (process-group SIGINT -> `--rm`) or an
    /// explicit `remove_container`.
    fn spawn_forwarder(
        &self,
        name: &str,
        network: &str,
        host_port: u16,
        target: &str,
        target_port: u16,
    ) -> Result<()>;
```

- [ ] **Step 2: Add the impl**

Add to `impl Engine for DockerCli` immediately after the `run_forwarder` impl:
```rust
    fn spawn_forwarder(
        &self,
        name: &str,
        network: &str,
        host_port: u16,
        target: &str,
        target_port: u16,
    ) -> Result<()> {
        let publish = format!("127.0.0.1:{host_port}:{host_port}");
        let listen = format!("TCP-LISTEN:{host_port},fork,reuseaddr");
        let connect = format!("TCP:{target}:{target_port}");
        let _child = self
            .cmd()
            .args([
                "run", "--rm", "--name", name, "--network", network, "--entrypoint", "socat",
                "-p", &publish, "alpine/socat", &listen, &connect,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("spawning forwarder {name}"))?;
        // Handle dropped on purpose: the forwarder runs in our process group, so
        // Ctrl-C stops it (`--rm` removes it); explicit cleanup is by name.
        Ok(())
    }
```

- [ ] **Step 3: Build**
Run: `cargo build -p work-core 2>&1 | tail -3` — Expected: clean.

- [ ] **Step 4: Commit**
```bash
git add crates/core/src/engine.rs
git commit -m "feat(core): Engine::spawn_forwarder (non-blocking callback bridge)"
```

---

### Task 4: `workspace::browse` auto-bridge + cleanup

**Files:** `crates/core/src/workspace.rs`

- [ ] **Step 1: Add the `HashSet` import**

At the top imports, add:
```rust
use std::collections::HashSet;
```

- [ ] **Step 2: Replace `browse` with the bridging version + `browse_loop` helper**

Replace the existing `pub fn browse(...)` body with:
```rust
pub fn browse(name: &str) -> Result<()> {
    let ws = Workspace::open(name)?;
    ws.ensure_running()?;
    let engine = ws.engine();
    let ctr = naming::container(name);
    let net = naming::network(name);
    crate::browser::install_shim(engine, &ctr)?;
    crate::browser::ensure_fifo(engine, &ctr)?;
    println!("Browsing for {name} — login URLs also bridge their callback port to the host.");
    println!("(Ctrl-C to stop)");
    let opener = crate::browser::host_opener();
    let mut bridged: HashSet<u16> = HashSet::new();
    let mut fwd_names: Vec<String> = Vec::new();
    let result = browse_loop(engine, &ctr, &net, name, &opener, &mut bridged, &mut fwd_names);
    // Cleanup forwarders on normal/error exit. Ctrl-C is handled by the process
    // group receiving SIGINT -> each `docker run --rm` forwarder stops + removes.
    for fwd in &fwd_names {
        let _ = engine.remove_container(fwd);
    }
    result
}

/// Read URLs from the bridge FIFO forever; for each, auto-bridge a loopback
/// OAuth callback port (if any) then open the URL in the host browser.
fn browse_loop(
    engine: &dyn Engine,
    ctr: &str,
    net: &str,
    ws: &str,
    opener: &str,
    bridged: &mut HashSet<u16>,
    fwd_names: &mut Vec<String>,
) -> Result<()> {
    loop {
        let line = engine.exec_capture(ctr, &["cat", crate::browser::FIFO_PATH])?;
        let url = line.trim();
        if !crate::browser::is_openable_url(url) {
            if !url.is_empty() {
                eprintln!("· ignored non-http(s) target: {url}");
            }
            continue;
        }
        if let Some(port) = crate::browser::callback_port(url) {
            if bridged.insert(port) {
                let fwd = format!("work-browse-{ws}-{port}");
                match engine.spawn_forwarder(&fwd, net, port, ctr, port) {
                    Ok(_) => {
                        fwd_names.push(fwd);
                        println!("· bridged callback port {port} (host 127.0.0.1:{port} -> {ws})");
                    }
                    Err(e) => {
                        bridged.remove(&port);
                        eprintln!(
                            "· could not bridge callback port {port} ({e}); run `work fwd {ws} {port}` if needed"
                        );
                    }
                }
            }
        }
        match Command::new(opener).arg(url).status() {
            Ok(_) => println!("↗ opened {url}"),
            Err(e) => eprintln!("· could not open {url} via {opener} ({e})"),
        }
    }
}
```

- [ ] **Step 3: Build + unit tests**
Run: `cargo test -p work-core --lib 2>&1 | grep -E "test result|error"`
Expected: all pass.

- [ ] **Step 4: Commit**
```bash
git add crates/core/src/workspace.rs
git commit -m "feat(core): work browse auto-bridges the OAuth callback port"
```

---

### Task 5: Docs

**Files:** `README.md` (OAuth section), `CHANGELOG.md`.

- [ ] **Step 1: README** — in the OAuth section, append after the `work browse` block a one-line note:
```markdown
For logins that call back to `localhost:<port>` (most OAuth), `work browse` also **auto-bridges that port** from the host into the workspace — you don't need a separate `work fwd`.
```
- [ ] **Step 2: CHANGELOG** — append to the existing `work browse` bullet:
```
Also auto-bridges the OAuth `localhost:<port>` callback into the workspace, so a login completes with one command (no separate `work fwd`).
```
- [ ] **Step 3: Commit**
```bash
git add README.md CHANGELOG.md
git commit -m "docs: work browse auto-bridges the OAuth callback port"
```

---

### Task 6: Verify

- [ ] **Step 1:** `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings` — clean.
- [ ] **Step 2:** `cargo test --workspace` — all pass.
- [ ] **Step 3 (smoke, real OrbStack):** `work new cb` → in another terminal `work browse cb` → inside the container run a tiny listener `python3 -m http.server 8090 &` then `xdg-open 'https://provider.example/auth?redirect_uri=http://localhost:8090/callback'` → expect `work browse` prints `· bridged callback port 8090` and `↗ opened …`; then from the host `curl -s http://127.0.0.1:8090/` reaches the container listener (HTTP 200 / directory listing); Ctrl-C → `docker ps -a | grep work-browse-cb` is empty (no leaks); `work doctor` green; `work rm cb --purge --yes`.

## Self-review
- Spec coverage: callback_port ✓ (T2), spawn_forwarder ✓ (T3), browse auto-bridge + cleanup ✓ (T4), url dep ✓ (T1), docs ✓ (T5), isolation untouched ✓ (forwarders mirror `work fwd`).
- Signatures: `spawn_forwarder` identical in trait+impl; `browse_loop` params match the call site; `callback_port -> Option<u16>`.
