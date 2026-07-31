# `work`

**Isolated multi-context session manager for developers.** Run multiple fully
isolated coding contexts — one persistent Linux container per workspace — on a
single machine, with a structural guarantee against cross-context data breach.

`work` gives each context its own container, named volume (mounted at
`/home/dev`), and dedicated bridge network. Code, AI agents, and credentials in
one workspace **physically cannot reach another**. You drive them all from one
terminal: attach to a persistent session, or tile all live sessions in a cockpit.

> `work` is a **session + isolation manager**. It does **not** install tools and
> does **not** manage credentials. You install and authenticate your own tools
> (Claude Code, Codex, z.ai, Gemini CLI, …) inside each container. `work`
> provides the sandbox; it never touches your secrets.

[![CI](https://github.com/coelhucas-dev/work-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/coelhucas-dev/work-cli/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## Requirements

- A container engine: **OrbStack** (recommended on macOS), **Docker**, **Podman**,
  or **Colima**. `work` auto-detects in that order.
- `tmux` on the host (only needed for `work resume`/`work all`). The in-container
  `tmux` (and `zsh`/`bash`) ships inside the base image.
- macOS today; Linux shares the same codebase.

## Install

**Homebrew (recommended, macOS):**

```bash
brew install coelhucas-dev/tap/work
```

Upgrade with `brew upgrade work`.

**One-line script (macOS + Linux):**

```bash
curl -fsSL https://raw.githubusercontent.com/coelhucas-dev/work-cli/main/install.sh | sh
```

**cargo-binstall** (if you have a Rust toolchain):

```bash
cargo binstall --git https://github.com/coelhucas-dev/work-cli work
```

**From source** (developers):

```bash
cargo install --git https://github.com/coelhucas-dev/work-cli
# or, from a clone:
cargo install --path .
```

Verify:

```bash
work --version
work doctor     # engine sanity + isolation check (no workspaces yet)
```

## Upgrade

`work` checks for a new release once a day and prints a one-line hint (to stderr,
so it never interferes with scripting). The hint matches how you installed it
(`brew upgrade work`, `cargo install --git …`, or a link to Releases). To disable
the check, set in `~/.config/work/config.toml`:

```toml
[update]
check = false
```

…or export `WORK_NO_UPDATE_CHECK=1`. The check is automatically off in CI and
when output isn't a terminal.

## Quickstart

```bash
# 1. Create an isolated workspace (volume + network + container).
work new acme --git-name "Jane Doe" --git-email jane@acme.io

# 2. Attach to its persistent session.
work acme
#   -> you are `dev` in /home/dev, inside an isolated Linux container, in a
#      tmux session named `work`. Start an agent, then detach with Ctrl-b d
#      (or just close the terminal) — it keeps running.

# 3. Inside the container, install & log into YOUR OWN tools.
#    (work never does this for you, and never sees your credentials)
npm i -g @anthropic-ai/claude-code && claude   # example

# 4. List / control / remove your workspaces from the host.
work ls                 # WORKSPACE  STATE    SESSION
                        # acme       running  live
work resume             # cockpit: tile every running session (host prefix Ctrl-a)
work stop acme          # stop the container (ends its session; files persist)
work start acme         # start again
work rm acme            # remove container+net+config, KEEP the volume
work rm acme --purge    # also delete the volume (irreversible; needs --yes)
work doctor             # verify isolation holds
```

Each workspace is fully persistent: whatever you install, your repos, your
dotfiles, and your logins live in that workspace's volume and survive reboots.

## Persistent sessions

`work <ws>` attaches to (or creates) a **tmux session named `work` inside the
container**. Anything you start there — shells, editors, AI agents — survives
detaching, closing the terminal/tab, and host sleep. It does **not** survive
`work stop` (an explicit power-off: running processes end, but files and on-disk
state in the volume persist) or `work rm`.

- **Detach:** `Ctrl-b d`, or just close the terminal.
- **Reattach:** `work <ws>` again.
- **Close the session:** `exit` at its prompt.

## The cockpit (`work resume`)

`work resume` (alias: `work all`) opens one **host** tmux session with a window
per **running** workspace, each attached to that container's session. The host
prefix is **`Ctrl-a`** so it doesn't clash with the in-container `Ctrl-b`:
`Ctrl-a <window>` switches workspaces, `Ctrl-a d` detaches the cockpit. Stopped
workspaces are listed in a note. No path is created between containers — each
window is an isolated client into one container on its own network.

## Familiarity (optional)

`work` detects your host shell (`$SHELL`, clamped to `zsh`/`bash`) and uses it
inside the container. You can optionally seed a verbatim copy of your own config:

```bash
work new acme --import-shell-config            # copies ~/.zshrc (or ~/.bashrc)
work new acme --import-shell-config ~/my.zshrc # copies that file -> /home/dev/.zshrc
work new acme --import-tmux-config             # copies ~/.tmux.conf -> /home/dev/.tmux.conf
work new acme --import-starship-config          # copies ~/.config/starship.toml -> /home/dev/.config/starship.toml
work new acme --import-dotfiles ~/dotfiles        # recursively copies that dir -> /home/dev
work new acme --use-author-default                # seed the repo's bundled personal/dotfiles templates
```

`work` prints a warning when it copies a config — **make sure it is secret-free**,
since it now lives in that workspace's volume. You can set a global default in
`~/.config/work/config.toml` (`import_shell_config = "…"`). A copied rc may
reference host paths that don't exist in the container; the copy is verbatim and
best-effort.

`--import-dotfiles <dir>` copies an entire directory tree (e.g. your `.zshrc`,
`.config/nvim`, `.config/atuin`) into `/home/dev` in one shot — useful for configs
that aren't single files. `--use-author-default` does the same with the **bundled
`personal/dotfiles` templates** (embedded in the binary at build) plus the configured
`default_image`, so `work new <ws> --use-author-default` reproduces the author's full
setup. Both also read a global default (`import_dotfiles = "…"`). The same secret-free
warning applies; explicit `--import-*` flags override individual files from a dotfiles seed.

## A separated environment (in-container identity)

Every `work <ws>` attach makes the workspace unmistakably identifiable as its own
isolated environment:

- **Identity banner.** `work` prints a compact block — workspace, image, system,
  hostname, `network: isolated · single-context`, home dir, git branch — before
  attaching. Opt out in `~/.config/work/config.toml`:
  ```toml
  show_banner = false
  ```
- **`$WORK`.** Each container exports `WORK=<ws>` (and `WORKSPACE=<ws>`), so any
  prompt or tool can name the workspace.
- **Default prompt.** With no `--import-shell-config`, the default shell (zsh) gets
  a minimal prompt that shows the workspace: `⬡ acme ~/proj %#`. Import your own rc
  and it wins verbatim. (A bash workspace keeps Debian's default `~/.bashrc`.)
- **Workspace-named session.** The in-container tmux session is named after the
  workspace (`tmux ls` shows `acme`, not `work`), the window is named `<ws>`, and
  the terminal tab is titled `work:<ws>`. Existing `work`-named sessions are renamed
  in place (lossless — running shells/agents survive) on the next attach.

### The `[Docker]` marker (Starship)

If you import a shell config that runs [Starship](https://starship.rs), its
`container` module renders a fixed `[Docker]` label — it is Starship detecting
`/.dockerenv`, not `work` or OrbStack, and `work` won't edit your prompt config.
Two opt-ins in `~/.config/starship.toml`:

```toml
# 1) Drop the now-redundant engine label:
[container]
disabled = true

# 2) …or show the workspace name instead, via a custom module:
[custom.work]
command = "echo $WORK"
when = """ test -n "$WORK" """
format = '[$output]($style) '
style = 'bold magenta'
```

To ship that snippet into every new workspace automatically, put it in
`~/.config/starship.toml` on the host and pass `--import-starship-config` (or set
`import_starship_config` globally) — `work` seeds it to
`/home/dev/.config/starship.toml`.

## Custom images (your tools, baked in)

Want every workspace to start with your full toolchain instead of importing it
each time? Build a personal image that extends the isolation-safe base:

1. Scaffold a starter: `work image init` (writes `./Dockerfile.work`).
2. Edit it — add your tools (the comments show `apt`, `cargo-binstall`, and the
   glibc/musl gotcha for binaries that need a newer glibc than the base ships).
3. Build: `work image build --tag my-work:latest --dockerfile ./Dockerfile.work`.
4. Use it: `work new <ws> --image my-work:latest`, or set
   `default_image = "my-work:latest"` in `~/.config/work/config.toml`.

`FROM work-base:latest` preserves every invariant `work doctor` checks (non-root
`dev` @ `/home/dev`, tmux/zsh/bash). Bake tool **binaries** in (system-wide, e.g.
`/usr/local/bin`); bring your `~/.zshrc` per-workspace via `--import-shell-config`
— the volume overlays `/home/dev`, so image-baked rc files get hidden.

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

CLIs like Claude Code, Cursor, or `gh` open a browser URL to log in. Inside a
container there's no browser, so `work` forwards those open-requests to your
**host** browser.

```bash
# In one terminal, start the bridge:
work browse acme
#   -> URLs that tools open inside `acme` now launch in your host browser.
#      Ctrl-C stops it (the container keeps running).
```

It installs an `xdg-open` shim in the workspace (and sets `$BROWSER`) that
sends each `http(s)` URL to a FIFO in the volume; `work browse` reads it and
opens each URL via `open` (macOS) / `xdg-open` (Linux). Override the host
opener with `WORK_HOST_BROWSER=<bin>`. Existing workspaces get the shim on
first `work browse` — no image rebuild.

Some logins additionally need a callback to `localhost:<port>`. For those,
`work` offers an **opt-in, manual** port bridge alongside `work browse`:

```bash
work fwd acme 8080      # bridge http://127.0.0.1:8080 -> acme:8080
# Complete the login in your browser, then Ctrl-C to tear the bridge down.
```

## CLI reference

| Command | Effect |
|---|---|
| `work new <ws>` | Create an isolated workspace (volume + network + container). Flags: `--image`, `--git-name`, `--git-email`, `--import-shell-config [<path>]`, `--import-tmux-config [<path>]`, `--import-starship-config [<path>]`, `--import-dotfiles <dir>`, `--use-author-default`. |
| `work <ws>` | Attach to (or create) the persistent in-container session. `Ctrl-b d` detaches. |
| `work ls` | List workspaces with state and session liveness (`live`/`—`). |
| `work start <ws>` / `work stop <ws>` | Lifecycle. `stop` ends the session (warns if one is live). |
| `work stop-all` | Stop every workspace. |
| `work resume` / `work all` | Cockpit: tile every running session in a host tmux (`Ctrl-a`). |
| `work rm <ws>` | Remove container + network + config, **keep** the volume. |
| `work rm <ws> --purge` | Also delete the volume (irreversible). Needs `--yes`. |
| `work fwd <ws> <port>` | (opt-in) forward a host port into a workspace for your own logins. |
| `work browse <ws>` | Forward URLs tools open inside the workspace to your host browser (OAuth logins). Ctrl-C stops. |
| `work config <ws>` | Show config. `--edit` opens it in `$EDITOR`. |
| `work image build` | Build the default `work-base:latest`; `--tag`/`--dockerfile` for custom images. |
| `work image init` | Scaffold a personal-image Dockerfile (extends `work-base`) to customize. |
| `work doctor` | Isolation + engine sanity check. |
| `work --yes` / `-y` | Global flag: skip all destructive-operation confirmations. |

**Destructive-operation safety.** `work` always warns + confirms destructive
ops, prompting only on a TTY. Two severities: **data loss** (`rm --purge`)
requires `--yes` or an interactive confirm and is **refused** in non-interactive
contexts; **work loss** (`stop`/`stop-all`/`rm`/`config --edit` recreate) warns
only when a live session would be ended. `--yes`/`-y` skips all prompts.

## Configuration

Non-secret metadata lives under `~/.config/work/`:

```
~/.config/work/config.toml              # default_image, import_shell_config, import_tmux_config, import_starship_config, import_dotfiles
~/.config/work/workspaces/<ws>.toml     # per-workspace: image, git identity, shell, …
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
