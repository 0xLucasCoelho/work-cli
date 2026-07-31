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

### Added — in-container identity
- `work <ws>` prints a fastfetch-style **identity banner** on attach and sets the
  terminal title; the in-container tmux session is named after the workspace (legacy
  `work` sessions are renamed in place, losslessly).
- Every container exports `WORK=<ws>` / `WORKSPACE=<ws>`; the no-import default shell
  (zsh) gets a workspace-aware prompt.
- `work new --import-starship-config [<path>]` seeds a `starship.toml` (e.g. to drop
  Starship's `[Docker]` marker or show `$WORK`). `show_banner` global opt-out.

### Added — dotfiles & author default
- `work new --import-dotfiles <dir>` recursively seeds a dotfiles tree into `/home/dev`
  (covers `.config/nvim`, `.config/atuin`, …); optional global `import_dotfiles`.
- `work new --use-author-default` seeds the repo's bundled `personal/dotfiles` templates
  (embedded in the binary at build) + the configured `default_image` — one-command setup.
- `Dockerfile.personal` (the `work-lucas` image) bakes claude, codex, omp, Antigravity
  (`agy`), and nvim alongside the existing CLI tools.

### Added — distribution & update awareness
- Homebrew tap: `brew install coelhucas-dev/tap/work` (bottles from release
  assets); `cargo binstall` and a `curl | sh` installer as fallbacks.
- Non-intrusive update-available check: once a day, prints a channel-aware
  one-line hint to stderr. Opt out with `[update] check = false` or
  `WORK_NO_UPDATE_CHECK=1`; off in CI / non-TTY.

### Security
- Cross-context isolation enforced at the container/network/volume boundary;
  `work doctor` verifies it. `work` never reads, writes, or moves secrets.

### Changed
- Container-engine-agnostic: auto-detect OrbStack → Docker → Podman → Colima.

### Fixed
- Default workspace containers render TUI agents (omp, Claude Code, …) and
  nested tmux correctly. The base image now ships `ncurses-term` (terminfo for
  `xterm-256color`/`tmux-256color`) + a UTF-8 locale and bakes `TERM`/`LANG`/
  `LC_ALL`, since `docker exec` does not propagate the host environment; a
  minimal default `.tmux.conf` enables 256-color + truecolor when no tmux config
  is imported.

[Unreleased]: https://github.com/coelhucas-dev/work-cli/commits/main
