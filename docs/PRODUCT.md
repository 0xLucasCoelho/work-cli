# Product: Orca-shaped ADE + company tenancy

This is **not** a herdr wrapper. This is not a terminal multiplexer product.
We are building an **Agent Development Environment** in the shape of
[Orca](https://www.onorca.dev/) — parallel agents, git worktrees, one desktop
window — with one extra first-class object Orca does not have: a **company**.

Orca's wall is a worktree. A worktree does not stop an agent from reading
`~/.claude`, `~/.ssh`, a sibling employer's repo, or `gh` as the wrong identity.
That is the gap. We add a real tenancy wall so one laptop can hold several
companies without their code, credentials, or agent memory touching.

```
Company          isolation wall (container + volume + net + identity)
  └── Product    git repo (Orca "project")
        └── Task git worktree (Orca "workspace")
              └── Agent session  — CLI we spawn and own (Claude, Codex, …)
```

The host app is the ADE. It owns terminals, worktrees, agent lifecycle, diffs,
and notifications. The company box is only the place that work is allowed to
exist. Do not outsource panes to herdr, tmux, or any other mux.

## What we steal from Orca

- One desktop window. Sidebar of work, not one OS window per company.
- Every task is a git worktree. Agents do not share a dirty tree.
- Bring-your-own agents (Claude Code, Codex, OpenCode, …). We are not a model.
- Agent session lifecycle (working / idle / needs you) and OS notifications.
- Ghostty-style embedded terminals we render ourselves.
- Fan-out / race is in-company only (same tenancy).

## What Orca does not do, and we must

- Company as the top-level object. Switching company switches the isolation
  domain, not just the folder.
- Per-company `$HOME`, git identity, `gh` / ssh, Claude/Codex/Grok config.
- Agents for company A cannot see company B. Worktrees never cross that wall.

## Forbidden

- herdr, tmux, Zellij, or any third-party mux as the session runtime
- One shared `$HOME` / host bind across companies
- Treating a git worktree as the company wall
