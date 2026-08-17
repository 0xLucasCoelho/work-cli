# Isolation invariants

`work` isolates **company tenancy** on one machine. The adversary is a curious
AI agent (and accidental human mix-up), not a kernel 0-day hunter.

Honest contract: after `find / -readable` inside company A, the agent still
cannot see B's files, secrets, or agent memory, cannot authenticate as B, and
cannot control the host runtime. This is **not** Qubes-grade isolation.

Anyone who can talk to a **rootful** engine as your user can read every volume.
Rootless Podman is the Linux default for that reason.

## Must hold

1. One named volume per company, mounted only at `/home/dev`. No host bind of
   `$HOME`, `/`, `/tmp`, `/var/run`, or `XDG_RUNTIME_DIR`.
2. Agent brains stay in that volume. `HOME`, `XDG_*`, `CLAUDE_CONFIG_DIR`,
   `CODEX_HOME`, and `GH_CONFIG_DIR` resolve under `/home/dev`.
3. No host `SSH_AUTH_SOCK`, no host git credential helper, no `docker.sock`.
4. Dedicated bridge network. No `--network=host`. No published ports on the
   workspace container.
5. Repos live under `/home/dev/src/<product>/` so parent `CLAUDE.md` walks hit
   nothing useful at `$HOME`.
6. `cap-drop ALL` + `no-new-privileges`. Tools belong in the image, not
   passwordless sudo at runtime.
7. Managed label on every volume/network/container. Refuse unlabeled reuse.
8. `work` never reads, writes, or stores secrets. Host `~/.config/work/` is
   metadata only.

## Allowed punch-holes

- Browser for OAuth: host `work browse` with a **per-company** Chromium
  profile and a one-shot `127.0.0.1` forward for an allowlisted callback port.
  Never `xdg-open` the logged-in host profile.
- Same public IP, host clipboard, vendor seeing files the agent opens.

## `work doctor`

Fails if any container on the daemon mounts `work-<ws>-home` or joins
`work-net-<ws>` besides that workspace, or if the workspace itself has extra
nets, bind mounts, published ports, root user, or missing hardening.

## What this is not

- Inner Claude/Codex sandboxes (bubblewrap / Seatbelt) — defense in depth
  *inside* a company, not the company wall.
- Git worktrees — Orca's unit. Task isolation inside one company, never the
  company wall.
- herdr / tmux / any mux — not part of this product. The ADE owns terminals.
