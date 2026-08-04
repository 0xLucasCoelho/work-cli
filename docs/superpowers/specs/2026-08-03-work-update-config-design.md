# `work update` — in-place config sync

## Problem

`work new <ws>` seeds config files (`.zshrc`, `.tmux.conf`, `.gitconfig`,
`.config/…`) into a container's named volume as a **snapshot**. After editing
the source templates/dotfiles, the only way to push the update into an existing
container is to rebuild the image, remove the container, and recreate it —
losing the running session and forcing a full recreate.

## Goal

A frictionless command that re-seeds managed config files into a running
container in place — no rebuild, no recreate, no session loss.

## Command surface

```
work update <ws>                 re-seed configs into one workspace's container
work update -a / --all           re-seed into every workspace
work update <ws> --dry-run       preview which files change; write nothing
work update <ws> --import-tmux-config ~/.tmux.conf   override source (mirrors `work new`)
                 --import-shell-config / --import-starship-config / --import-dotfiles <dir>
```

- Bare `work update` (no `<ws>`, no `-a`) → usage error.
- The global `-y/--yes` composes but `update` adds no destructive prompt of its
  own (the `--dry-run` valve + printed summary cover safety).

## Behavior (per workspace)

1. **Resolve the source** — identical chain to `work new`: explicit `--import-*`
   flags → `~/.config/work/config.toml` defaults → embedded `templates/` tree.
   Reuses `resolve_import` + the dotfiles-dir logic. When no dotfiles dir is
   resolved, the embedded templates are always seeded (update's default), so a
   bare `work update <ws>` pushes the current templates without `--default`.
2. **Ensure the container is running** (`ensure_running`): starts a stopped one,
   recreates a missing one from config. Required for `docker cp` + `chown`.
3. **Diff**: enumerate the source tree + per-file imports; hash each source file
   (host `sha2`) against the in-container file (`docker exec sha256sum`).
   Classify each path as `added` / `updated` / `unchanged`.
4. **`--dry-run`**: print the classification; write nothing; exit.
5. **Apply**: `seed_dir` the tree + `seed_file` per import into `/home/dev`
   (overwriting managed configs), `chown -R dev:dev`, re-apply git identity.
   Skip the write (early return) when nothing differs. Print the summary.

`update` overwrites only **managed** config files. Projects, repos, and agent
state in the volume are never touched. It does not stop, recreate, or rebuild.

## `-a` flow

Iterate `list_workspace_names()` (sorted). Per workspace: print a one-line
header + the change summary. A final tally line:
`updated N (M unchanged, K skipped)`. Workspaces whose container can't be
brought up are skipped with a `· '<ws>': <error>` note, never aborting the batch.

## Reporting

```
✓ updated 'acme' — 3 changed, 1 added, 2 in sync
  + .config/starship.toml
  ~ .tmux.conf
  ~ .zshrc
  ~ .gitconfig
  = .zshenv
  = .config/atuin/config.toml
```

Dry-run replaces the verb with `would update` / `would sync` and prints a
`· dry run — no files written` banner.

## Implementation

- **`crates/core/Cargo.toml`** — add `sha2` (host-side content hashing).
- **`crates/core/src/naming.rs`** — add `"update"` to `RESERVED`.
- **`crates/core/src/workspace.rs`** —
  - `pub struct UpdateReport { added, updated, unchanged: Vec<String> }` with
    `touched()` / `total()` helpers.
  - `Workspace::update(shell, tmux, starship, dotfiles, dry_run) -> Result<UpdateReport>`
    — resolves the source (mirrors `create`'s seeding resolution), diffs via
    hashing, and applies when not dry-run.
  - Pure helpers: `walk_tree` (recursive file enumeration, rel paths),
    `container_rel` (strip `/home/dev/` prefix), `file_sha256`.
- **`crates/cli/src/main.rs`** — `Update(UpdateArgs)` variant + dispatch.
- **`crates/cli/src/commands.rs`** — `UpdateArgs` (mirrors `NewArgs` import
  flags + `ws: Option<String>`, `--all`, `--dry-run`) and `pub fn update`.
- **Docs** — README CLI reference + CHANGELOG + `--help` text.

## Tests

Pure helpers only (no Docker): `container_rel` prefix stripping and
`walk_tree` enumeration over a tempdir. Diffing/seeding is integration-level IO
and stays covered by the existing engine tests' shape.

## Non-goals

- No workspace-metadata changes (that is `work config --edit`).
- No image rebuild / recreate.
- No recording of each workspace's original source (YAGNI — the resolution
  chain `flags → global → templates` covers every case).
