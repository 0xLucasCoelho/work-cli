# Host UI contract (the ADE)

Daily driver is one Tauri desktop app — an Orca-shaped ADE. Isolation stays
per-company. The chrome lists companies, products, worktrees, and agent
sessions. It does not merge them.

herdr is not part of this product. Terminals are PTYs the ADE owns.

## Layout

```
sidebar                              main
────────                             ────
acme ▾                               one focused agent terminal
  mmw ▾                              (split later, same company only)
    ● claude  feat/auth
    ○ nvim    feat/auth
    ! tests   fix/pager
globex ▾
  billing
personal
```

- Top-level rows = **companies** (the isolation wall).
- Next = **products** (repos under `/home/dev/src/<product>/` in that box).
- Leaves = **worktrees / agent sessions** Orca would put in its sidebar.
- One visible terminal in v1. Split only later, and only inside one company.

## Keyboard (defaults, remappable)

| Keys | Action |
|---|---|
| `Alt+1` … `Alt+9` | Focus company 1…9 |
| `Ctrl+1` … `Ctrl+9` | Focus session 1…9 in the current company |
| `Alt+[` / `Alt+]` | Prev/next company |
| `Ctrl+Tab` | Next session in company |
| `Ctrl+Shift+T` | New worktree + session in current company |
| `Ctrl+Shift+N` | New company |
| `Ctrl+W` | Close session (confirm if a live agent) |

## How the ADE talks to a company box

1. **Visible terminal:** host PTY → `podman exec` (or `work attach`) into that
   box, cwd = the worktree. Switching tears down that attach; the agent process
   stays in the box if we later persist it. v1 may be "session dies on switch"
   if persistence is not ready — say so in the implementation plan, do not
   paper over it with a mux.
2. **Status / notifications:** the ADE tracks processes **it started** in each
   box (working / idle / needs you). No third-party socket API.

## Notifications

OS notification title is prefixed with the company (`globex · claude needs you`).
In-app toast if the focused company is different from the event.

## Rejected

- herdr, tmux, Zellij, or host `herdr --remote`
- One multiplexer server for all companies
- Bind-mounting every volume into the GUI
- Using Orca itself as the wall (same OS user, same `$HOME`)

CLI (`work`) remains for scripts, doctor, and a fallback login shell.
The ADE is the daily driver.
