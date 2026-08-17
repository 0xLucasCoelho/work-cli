# Host UI contract

Daily driver is a single Tauri app. Isolation stays per-company. The chrome
switches companies and tabs; it does not merge them.

## Layout (Zen + Claude Squad)

```
sidebar                         main
─────────                       ────
acme ▾                          one embedded terminal
  ● claude                      attached to the focused pane
  ○ nvim
globex ▾
  ! tests
personal
```

- Top-level rows = companies (tenancy).
- Children = that company's in-box herdr tabs/panes, with agent status.
- One visible terminal in v1. Split only later, and only inside one company.

## Keyboard (defaults, remappable)

| Keys | Action |
|---|---|
| `Alt+1` … `Alt+9` | Focus company 1…9 |
| `Ctrl+1` … `Ctrl+9` | Focus tab 1…9 in the current company |
| `Alt+[` / `Alt+]` | Prev/next company |
| `Ctrl+Tab` | Next tab in company |
| `Ctrl+Shift+T` | New tab in current company |
| `Ctrl+Shift+N` | New company |
| `Ctrl+W` | Close tab (confirm if a live agent) |

## Attach (never one herdr for the fleet)

1. **Visible pane:** host PTY → `podman exec` / `work attach <ws> --pane <id>`
   into that box's herdr. Switching tears down the attach; agents keep running.
2. **Metadata / notifications:** per-company herdr Unix socket proxied to
   `$XDG_RUNTIME_DIR/work/<ws>.sock` (mode 0600). Fan-out `agent.list` and
   `events.subscribe`. Never point one herdr client at every socket as one
   session.

## Notifications

Subscribe to every running company's herdr events. OS notification title is
prefixed with the company (`globex · claude needs you`). In-app toast if the
focused company is different from the event.

## Rejected

- One herdr server for all companies
- Host `herdr --remote` + in-box sshd
- Bind-mounting every volume into the GUI
- Streaming two companies' pane bytes into one multiplexer

CLI (`work`) remains for scripts and SSH. The TUI picker is a fallback, not
the daily driver.
