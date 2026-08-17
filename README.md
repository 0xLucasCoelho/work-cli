# work

An Orca-shaped Agent Development Environment with a company wall, so one
machine can hold several employers without their code, credentials, or agents
touching.

Orca's unit is a git worktree. Ours is **company → product → worktree →
agent**. The wall is a persistent Linux box per company (own home, own
network, own `~/.claude` / `gh` / ssh). The ADE is a Tauri desktop app that
owns terminals and agent sessions. This repo's CLI is the isolation engine
that app calls.

**Not herdr. Not a multiplexer wrapper.** See [docs/PRODUCT.md](docs/PRODUCT.md).

This is **best-effort container tenancy** against a curious agent — not
Qubes-grade isolation. Rootless Podman is the Linux default. Anyone who can
talk to a rootful Docker daemon as your user can read every volume.

See [docs/PRODUCT.md](docs/PRODUCT.md), [docs/ISOLATION.md](docs/ISOLATION.md),
and [docs/UI.md](docs/UI.md).

## Status

Refactor in progress. Shipped so far:

- product / isolation / UI contracts (ADE + company tenancy)
- `work-core` engine (Podman-first, cap-drop ALL, pinned `HOME`/agent dirs)
- CLI: `new` / `ls` / `start` / `stop` / `rm` / `doctor` / `attach` / `image build`

Not yet: ADE (Tauri), worktrees, agent sessions, per-company browser, migrate.

## Requirements

- [Podman](https://podman.io/) (preferred) or Docker
- Rust stable (to build from source)

## CLI

```sh
cargo install --path crates/cli

work image build              # build work-base:latest
work new acme --git-email you@acme.io
work acme                     # fallback login shell in that company box
work ls
work doctor                   # re-check every isolation invariant
work stop acme
work rm acme                  # keep the volume
work rm acme --purge --yes    # delete the volume
```

Inside the box you install your own tools (Claude Code, Codex, `gh`).
`work` never touches secrets. The ADE will launch those agents later.

## Layout

```
crates/core     isolation, engine, lifecycle, doctor
crates/cli      `work` binary
crates/docker   work-base image (no passwordless sudo)
docs/           isolation + UI contracts
```
