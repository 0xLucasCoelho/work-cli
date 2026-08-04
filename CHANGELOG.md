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
- `work browse <ws>` — forward URLs that in-container tools open (`xdg-open`/`$BROWSER`) to your host browser, so OAuth/subscription logins (Claude Code, Cursor CLI, …) complete without leaving the terminal. Also auto-bridges the OAuth `localhost:<port>` callback into the workspace, so a login completes with one command (no separate `work fwd`).
- `work config <ws> [--edit]` — show/edit non-secret workspace metadata.
- `work update <ws>` — re-sync managed config files into a running container
  **in place**: no image rebuild, no recreate, no session loss. Source mirrors
  `work new` (--import-* flags → global config → embedded templates).
  `--dry-run` previews; `-a`/`--all` updates every workspace.
- `work doctor` — isolation + engine sanity check (unique network per workspace,
  only its own volume mounted, non-root, no host ports, image match).

### Added — workflow layer (sessions)
- `work resume` (= `work all`) — **cockpit**: tile every running session in one
  host tmux (prefix `Ctrl-a`, distinct from the in-container `Ctrl-b`).
- `work tab <ws> [--name <n>]` — open a NEW tmux window ("tab") in the
  workspace's session and attach to it. Each call = one persistent window that
  survives closing the terminal (not `work stop`) and becomes the session's active
  window. Bare `work <ws>` still resumes into the existing session.
- `work tabs <ws>` — list the workspace's tmux windows (index, name, panes,
  active marker, current command).
- Familiarity: host-shell auto-detection (`$SHELL`, clamped to `zsh`/`bash`),
  opt-in `--import-shell-config [<path>]` / `--import-tmux-config [<path>]`
  (verbatim, owned by `dev`, with a secret-free warning), optional global default.
- First-run shell-prompt suppression (the volume overlays `/home/dev`).

### Added — images
- `work image build [--tag <t>] [--dockerfile <f>]` — build the default
  `work-base:latest` (git + tmux + zsh + bash + curl/jq/build-essential,
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
- `work new --default` seeds the repo's bundled `templates/` dotfiles
  (embedded in the binary at build) + the configured `default_image` — one-command setup.
- `Dockerfile.personal` (the `work-lucas` image, **Ubuntu 24.04 LTS**) bakes
  claude, codex, omp, Antigravity (`agy`), and nvim alongside
  fzf/ripgrep/direnv/starship/zoxide/fd/bat/eza/git-delta/atuin/mise.

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
- Base image switched from `node:20-bookworm-slim` to `debian:trixie-slim`;
  Node is no longer bundled (it was only there for the personal image's npm
  globals; the personal image installs Node 22 via NodeSource).
- Personal image (`Dockerfile.personal`) is now a standalone Ubuntu 24.04 LTS
  image instead of extending `work-base`. Noble's glibc 2.39 and neovim 0.9.5
  eliminate the atuin musl workaround and the nvim tarball download.

### Fixed
- Default workspace containers render TUI agents (omp, Claude Code, …) and
  nested tmux correctly. The base image now ships `ncurses-term` (terminfo for
  `xterm-256color`/`tmux-256color`) + a UTF-8 locale and bakes `TERM`/`LANG`/
  `LC_ALL`, since `docker exec` does not propagate the host environment; a
  minimal default `.tmux.conf` enables 256-color + truecolor when no tmux config
  is imported.
- Forward the host's `COLORTERM` into workspace attach so TUI agents detect
  truecolor (they were forced to ANSI/16-color because `docker exec` dropped it).
  The default `.tmux.conf` also re-picks up `COLORTERM` on re-attach.
- Set `NERD_FONTS=1` in workspace containers so agents like omp render Nerd Font
  glyphs. work's in-container tmux makes them see `TERM_PROGRAM=tmux` instead of
  the host terminal, defeating their Nerd-Font auto-detection (omp only trusts
  Ghostty/iTerm/WezTerm/Kitty/Alacritty); `NERD_FONTS=1` forces it. The host
  terminal still renders the glyphs. The value is injected at both `docker run`
  (container env) and `docker exec` (attach), so it reaches omp even in
  containers created before the fix; the default `.tmux.conf` re-picks it up
  via `update-environment` on re-attach.

[Unreleased]: https://github.com/coelhucas-dev/work-cli/commits/main
