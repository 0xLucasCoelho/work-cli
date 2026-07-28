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
git clone https://github.com/lucascoelho/work-cli
cd work-cli
cargo build --workspace
cargo test --workspace
```

You'll need a container engine (OrbStack / Docker / Podman / Colima) for
end-to-end checks:

```bash
cargo run -q -- doctor          # engine + isolation sanity
```

## Code quality gates (must pass before merge)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Integration tests that talk to a real engine are marked `#[ignore]`; run them
explicitly when you have an engine available:

```bash
cargo test --workspace -- --ignored
```

## Architecture notes for contributors

- **Isolation logic lives in exactly one place: `crates/core`.** The CLI is a
  thin client. Don't push isolation decisions into the CLI.
- **The engine is abstracted.** `crates/core/src/engine.rs` defines the `Engine`
  trait; `DockerCli` is the adapter that shells out to the `docker` binary
  (OrbStack, Docker, and Colima all expose it; Podman is CLI-compatible).
  Auto-detection order is OrbStack → Docker → Podman → Colima.
- **Pure functions are unit-tested; shell-out paths are verified at milestones.**
  Keep decision logic (naming, validation, config, isolation/hardening analysis)
  separate from collection (docker calls) so it stays testable.

### Adding support for a new container runtime

If a runtime exposes a docker-compatible CLI, detection + the `docker` binary
usually cover it. Otherwise implement the `Engine` trait for it and add a branch
to `pick_kind` / `detect` in `engine.rs`. Run `work doctor` against it and
confirm the isolation invariants still hold.

## Commit messages

Conventional Commits style, e.g. `feat(core): …`, `fix(cli): …`, `docs: …`.
Keep commits focused; one logical change per commit.

## Reporting issues

Use the issue templates. For suspected isolation regressions, include the full
output of `work doctor`.
