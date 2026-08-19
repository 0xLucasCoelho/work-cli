# Contributing to `work`

Thanks for your interest in improving `work`! This is a small, focused tool and
we aim to keep it that way.

## What `work` is (and isn't)

`work` is a **session + isolation manager**. It creates isolated, persistent
Linux containers (one per workspace) and lets you drive them from one terminal.
It deliberately does **not** install tools, manage logins/credentials, or be
opinionated about what runs inside. Please keep contributions aligned with that
scope — see [`docs/superpowers/specs/`](docs/superpowers/specs/) for the design.

## Development setup

```bash
git clone https://github.com/0xlucascoelho/work-cli
cd work-cli
cargo build --workspace
cargo test --workspace
```

You'll need a live supported container engine for end-to-end checks. The
preferred contributor setup is rootless Podman on Linux, Podman machine or
Podman Desktop on macOS, and Podman inside a WSL2 distribution on Windows.
Docker-compatible engines are useful fallback and compatibility targets;
OrbStack and Colima are macOS compatibility targets, not project defaults.

| Host | Contributor requirement |
|---|---|
| Linux | Podman preferred, rootless where practical; Docker-compatible fallback supported. |
| macOS | Podman machine/Podman Desktop supported; Docker, OrbStack, and Colima may be used for compatibility checks. |
| Windows | WSL2 only; install the project and engine inside the WSL2 distro. Native Windows containers/backends are out of scope. |

Before running the CLI, verify the backend from the same shell that will run
the tests:

```bash
podman info                      # preferred path
cargo run -q -- doctor           # engine + isolation sanity
```

For a Docker-compatible fallback, use `docker info`; on macOS, start the
Podman machine first with `podman machine start`. On Windows, run these commands
inside WSL2, never from PowerShell or Command Prompt.

## Code quality gates (must pass before merge)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Integration tests that talk to a real engine are marked `#[ignore]`; they are a
required compatibility check for changes affecting engine selection, workspace
lifecycle, isolation, images, or browser/port bridging. Run them explicitly
with a live backend:

```bash
cargo test --workspace -- --ignored
```

Unit tests and `work doctor` are not substitutes for this test: the ignored
suite is the evidence that the selected CLI and backend can really create,
inspect, attach to, and clean up a workspace.

## Architecture notes for contributors

- **Isolation logic lives in exactly one place: `crates/core`.** The CLI is a
  thin client. Don't push isolation decisions into the CLI.
- **The engine is abstracted.** `crates/core/src/engine.rs` defines the `Engine`
  trait and one CLI adapter. Selection is platform-aware: Podman is preferred
  on Linux, WSL2, and macOS, with Docker-compatible fallback and OrbStack/
  Colima compatibility on macOS. The adapter must invoke the selected CLI
  (`podman` for Podman; the compatible `docker` command for Docker-compatible
  engines), not assume that the host is Docker Desktop.
- **Explicit selection is part of the portability contract.**
  `WORK_ENGINE=podman|docker|orbstack|colima` overrides automatic selection
  for the current command. Invalid, unavailable, or stopped selections should
  fail with an actionable diagnostic rather than silently switching to another
  daemon.
- **Pure functions are unit-tested; shell-out paths are verified at milestones.**
  Keep decision logic (naming, validation, config, isolation/hardening analysis)
  separate from collection (selected-engine calls) so it stays testable.

### Adding support for a new container runtime

If a runtime exposes a Docker-compatible CLI, the adapter can usually cover it,
but selection must still identify the actual runtime and its daemon. Otherwise
implement the `Engine` trait for it and add platform-aware selection. Run
`work doctor` and the ignored integration suite against it; confirm the
isolation invariants, rootless behavior where applicable, image operations,
interactive attach, and cleanup all hold.

## Commit messages

Conventional Commits style, e.g. `feat(core): …`, `fix(cli): …`, `docs: …`.
Keep commits focused; one logical change per commit.

## Reporting issues

Use the issue templates. For suspected isolation regressions, include the full
output of `work doctor`.
