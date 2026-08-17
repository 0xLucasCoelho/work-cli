# work

Isolated company environments on one machine, with one app to switch them.

Each company is a persistent Linux box: its own home volume, its own network,
its own agent memory (`~/.claude`, `~/.codex`, `gh`, ssh). Code and agents in
`acme` cannot read `globex`. The daily driver is a Tauri shell (Zen-style
sidebar + one terminal). This repo's CLI is the isolation engine that shell
calls.

This is **best-effort container tenancy** against a curious agent — not
Qubes-grade isolation. Rootless Podman is the Linux default. Anyone who can
talk to a rootful Docker daemon as your user can read every volume.

See [docs/ISOLATION.md](docs/ISOLATION.md) and [docs/UI.md](docs/UI.md).

## Status

Refactor in progress. Shipped so far:

- isolation invariants + UI contract
- `work-core` engine (Podman-first, cap-drop ALL, pinned `HOME`/agent dirs)
- CLI: `new` / `ls` / `start` / `stop` / `rm` / `doctor` / `attach` / `image build`

Not yet: Tauri app, herdr socket proxy, per-company browser, volume migrate.

## Requirements

- [Podman](https://podman.io/) (preferred) or Docker
- Rust stable (to build from source)

## CLI

```sh
cargo install --path crates/cli

work image build              # build work-base:latest
work new acme --git-email you@acme.io
work acme                     # attach (login shell, or herdr if installed in the box)
work ls
work doctor                   # re-check every isolation invariant
work stop acme
work rm acme                  # keep the volume
work rm acme --purge --yes    # delete the volume
```

Inside the box you install your own tools (Claude Code, Codex, `gh`, herdr).
`work` never touches secrets.

## Layout

```
crates/core     isolation, engine, lifecycle, doctor
crates/cli      `work` binary
crates/docker   work-base image (no passwordless sudo)
docs/           isolation + UI contracts
```
