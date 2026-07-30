# `work` author-default setup: dotfiles templates + `--import-dotfiles` + `--use-author-default`

**Date:** 2026-07-30
**Status:** Approved (direction)
**Goal:** Let a workspace come up with the author's full environment — CLI tools baked into the image, dotfiles seeded into `/home/dev` — via one flag, with a general directory-import mechanism underneath.

## Background

The repo already ships `Dockerfile.personal` (the `work-lucas:latest` image: fzf/ripgrep/direnv + starship/zoxide/fd/bat/eza/git-delta/mise/atuin/zinit). The author's host setup additionally uses: claude, codex, omp (Oh My Pi), Antigravity (`agy`), and a LazyVim nvim config. Existing per-file import flags (`--import-shell-config`, `--import-tmux-config`, `--import-starship-config`) cover only single files, not a tree (nvim/atuin live under `.config/`).

## Components

### 1. `personal/dotfiles/` — secret-free templates (source of truth in the repo)
Verbatim copies of the author's files, preserving structure so a recursive copy lands at the right paths:
```
personal/dotfiles/
  .zshrc .zshenv .tmux.conf .gitconfig
  .config/starship.toml   # disables the [Docker] container module, shows $WORK
  .config/nvim/...        # LazyVim (lua/config, lua/plugins, init.lua, lazy-lock.json)
  .config/atuin/config.toml  # secret-free (real encryption key NOT included)
```
Secret scan passed: the only atuin "key/secret" hits are documentation comments; the real key lives in `~/.local/share/atuin/key` (not copied). nvim `ai.lua` references the `claude` CLI, no keys.

### 2. `Dockerfile.personal` extended — bake what the dotfiles depend on
Added on top of the existing image (as `root`, system-wide):
- `npm i -g @anthropic-ai/claude-code @openai/codex` (node 20 ships in work-base).
- omp: `curl -fsSL https://omp.sh/install | bash` (as dev → `~/.local/bin/omp`).
- Antigravity (`agy`): `curl -fsSL https://antigravity.google/cli/install.sh | bash` (as dev → `~/.local/bin/agy`).
- nvim: GitHub release tarball → `/usr/local/bin/nvim` (LazyVim needs ≥0.9; bookworm apt has 0.7).
- Cursor agent: **omitted** (no reliable public install method).

### 3. `work new --import-dotfiles <dir>` (general; new `work` code)
Recursively copy a host directory tree into `/home/dev`:
- New `Engine::seed_dir(name, src_dir, dest_dir)`: `docker cp <src_dir>/. <name>:<dest_dir>/` then `exec --user root chown -R dev:dev <dest_dir>`. (Reuses the parent-mkdir logic already added for `seed_file`.)
- New CLI flag `--import-dotfiles <dir>` (value required) + global config `import_dotfiles: Option<PathBuf>`.
- Prints the same secret-free warning as the other import flags.

### 4. `work new --use-author-default` (convenience; embedded templates)
- The `personal/dotfiles/` templates are **embedded into the `work-core` binary at build** via the `include_dir` crate (`include_dir!("../../personal/dotfiles")`), so the flag needs no external path.
- At runtime: extract the embedded tree to a tempdir, `seed_dir` it into `/home/dev`, and use the configured `default_image` (the author's is already `work-lucas:latest`). Prints the secret-free warning.
- This makes `work new <ws> --use-author-default` reproduce the author's whole shell/editor/agent setup in one command.

## Decisions
- **Embedding** (not config-path): the user asked for templates in the repo, referenced automatically. Compile-time `include_dir` is the portable fulfillment (no absolute paths, no runtime repo lookup).
- **`.gitconfig` PII:** the embedded template includes the author's git identity. Acceptable for the author's own tool; flagged so it can be sanitized before any wide publish.
- **atuin sync** won't work in-container (key not shipped) — documented; `atuin login` inside the container if needed.
- **Cursor** omitted per user decision.

## Isolation impact
**None.** Baked tools are system binaries; seeded dotfiles are user-owned files in the workspace's own volume; no new mount/network/user/port. `work doctor` invariants unchanged.

## Files affected
- Create: `personal/dotfiles/**` (templates).
- Modify: `Dockerfile.personal` (agents + nvim).
- Modify: `crates/core/Cargo.toml` (+ `include_dir`), `crates/core/src/{lib,engine,workspace,config}.rs`, `crates/cli/src/{commands,main}.rs`.
- Modify: `README.md`, `CHANGELOG.md`.

## Out of scope
- A first-class "dotfiles sync/update on existing workspaces" command (seeding happens at `work new`, as today).
- Atuin history/key sync.
- Windows support.
