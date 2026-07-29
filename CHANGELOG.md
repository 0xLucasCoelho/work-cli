# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

The first public release. Not yet tagged — see
[commits](https://github.com/coelhucas-dev/work-cli/commits/main).

### Added — isolation & lifecycle
- `work new <ws>` — create an isolated workspace: one Linux container, one named
  volume mounted at `/home/dev`, one dedicated bridge network.
- `work <ws>` — attach to (or create) a **persistent in-container tmux session**
  named `work`; survives detach / closing the terminal (not `work stop`).
- `work ls` — list workspaces with state and a SESSION column (`live`/`—`).
- `work start` / `work stop` / `work stop-all` — container lifecycle.
- `work rm <ws> [--purge]` — remove container + network + config; **keeps the
  volume by default**, `--purge` deletes it (irreversible; needs `--yes`).
- `work fwd <ws> <port>` — opt-in host→workspace port bridge for your own logins.
- `work config <ws> [--edit]` — show/edit non-secret workspace metadata.
- `work doctor` — isolation + engine sanity check (unique network per workspace,
  only its own volume mounted, non-root, no host ports, image match).

### Added — workflow layer (sessions)
- `work resume` (= `work all`) — **cockpit**: tile every running session in one
  host tmux (prefix `Ctrl-a`, distinct from the in-container `Ctrl-b`).
- Familiarity: host-shell auto-detection (`$SHELL`, clamped to `zsh`/`bash`),
  opt-in `--import-shell-config [<path>]` / `--import-tmux-config [<path>]`
  (verbatim, owned by `dev`, with a secret-free warning), optional global default.
- First-run shell-prompt suppression (the volume overlays `/home/dev`).

### Added — images
- `work image build [--tag <t>] [--dockerfile <f>]` — build the default
  `work-base:latest` (node + git + tmux + zsh + bash + curl/jq/build-essential,
  non-root `dev`) or a custom image.
- `work image init [--output <f>]` — scaffold a personal-image Dockerfile that
  extends `work-base`, with the glibc/musl gotcha documented.

### Added — safety
- Uniform destructive-operation policy: prompts only on a TTY; two severities
  (data loss = volume purge; work loss = stop/rm/recreate with a live session);
  `--yes`/`-y` skips; non-interactive contexts refuse without `--yes`.

### Security
- Cross-context isolation enforced at the container/network/volume boundary;
  `work doctor` verifies it. `work` never reads, writes, or moves secrets.

### Changed
- Container-engine-agnostic: auto-detect OrbStack → Docker → Podman → Colima.

[Unreleased]: https://github.com/coelhucas-dev/work-cli/commits/main
