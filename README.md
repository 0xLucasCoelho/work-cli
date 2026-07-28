# `work`

**Isolated multi-context session manager for developers.** Run multiple fully
isolated coding contexts — one persistent Linux container per workspace — on a
single machine, with a structural guarantee against cross-context data breach.

`work` gives each context its own container, named volume (mounted at
`/home/dev`), and dedicated bridge network. Code, AI agents, and credentials in
one workspace **physically cannot reach another**. You drive them all from one
terminal: attach a shell, or open them together in tmux.

> `work` is a **session + isolation manager**. It does **not** install tools and
> does **not** manage credentials. You install and authenticate your own tools
> (Claude Code, Codex, z.ai, Gemini CLI, …) inside each container. `work`
> provides the sandbox; it never touches your secrets.

[![CI](https://github.com/lucascoelho/work-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/lucascoelho/work-cli/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## Requirements

- A container engine: **OrbStack** (recommended on macOS), **Docker**, **Podman**,
  or **Colima**. `work` auto-detects in that order.
- `tmux` (only needed for `work all`).
- macOS today; Linux shares the same codebase.

## Install

From source (for now; Homebrew tap + release binaries are planned):

```bash
cargo install --git https://github.com/lucascoelho/work-cli
# or, from a clone:
cargo install --path .
```

Verify:

```bash
work --version
work doctor     # engine sanity + isolation check (no workspaces yet)
```

## Quickstart

```bash
# 1. Create an isolated workspace (volume + network + container).
work new acme --git-name "Jane Doe" --git-email jane@acme.io

# 2. Shell into it.
work acme
#   -> you are now `dev` in /home/dev, inside an isolated Linux container.

# 3. Inside the container, install & log into YOUR OWN tools.
#    (work never does this for you, and never sees your credentials)
npm i -g @anthropic-ai/claude-code && claude   # example

# 4. List / control your workspaces from the host.
work ls                 # acme  running
work stop acme          # stop (state persists in the volume)
work start acme         # start again
work all                # open every workspace in a tmux session
work doctor             # verify isolation holds
```

Each workspace is fully persistent: whatever you install, your repos, your
dotfiles, and your logins live in that workspace's volume and survive reboots.

## Why isolation matters

AI coding agents can read the filesystem and persist memory across sessions.
Switching env vars or OS accounts is not enough — an agent running as you can
read sibling directories. `work` enforces isolation at the OS container boundary:

- **One container per workspace**, on its **own bridge network** (`work-net-<ws>`).
  Workspace containers cannot address each other (no shared L2), so there is no
  network path between them.
- **One named volume per workspace**, mounted at `/home/dev` and **nowhere else**.
  No host bind-mounts.
- **Secrets live only inside the workspace's volume.** `work` holds no secrets
  and never moves them.

`work doctor` verifies all of the above and fails loudly if any invariant is
violated (e.g. a container that somehow shares a network, mounts a foreign
volume, runs as root, or publishes a host port).

## Logging into tools that need a browser (OAuth)

Some CLIs complete login via a callback to `localhost:<port>`. `work` offers an
**opt-in, manual** port bridge — it does not run or orchestrate the login, it
just forwards a host port into the workspace when you ask:

```bash
# In the workspace, start the login; then on the host:
work fwd acme 8080      # bridge http://127.0.0.1:8080 -> acme:8080
# Complete the login in your browser, then Ctrl-C to tear the bridge down.
```

## CLI reference

| Command | Effect |
|---|---|
| `work new <ws>` | Create an isolated workspace (volume + network + container). Flags: `--image`, `--git-name`, `--git-email`. |
| `work <ws>` | Ensure running; exec an interactive shell. |
| `work ls` | List workspaces and container state. |
| `work start <ws>` / `work stop <ws>` | Lifecycle (state persists). |
| `work stop-all` | Stop every workspace. |
| `work all` | Open all workspaces in a tmux session named `work`. |
| `work fwd <ws> <port>` | (opt-in) forward a host port into a workspace for your own logins. |
| `work config <ws>` | Show config. `--edit` opens it in `$EDITOR`. |
| `work image build` | Build the default `work-base:latest`; `--tag`/`--dockerfile` for custom images. |
| `work doctor` | Isolation + engine sanity check. |

## Configuration

Non-secret metadata lives under `~/.config/work/`:

```
~/.config/work/config.toml              # default_image, etc.
~/.config/work/workspaces/<ws>.toml     # per-workspace: image, git identity, …
```

`work` stores **only non-secret metadata** there. Credentials live inside each
workspace's volume, never on the host via `work`.

## Project layout

```
crates/core   work-core: isolation + orchestration engine (the only place isolation logic lives)
crates/cli    the thin `work` binary
crates/docker/work-base.Dockerfile   the default base image
```

## License

MIT. See [LICENSE](LICENSE).
