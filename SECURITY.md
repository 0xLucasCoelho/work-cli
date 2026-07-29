# Security Policy

`work`'s entire purpose is a **hard guarantee against cross-context data breach**:
one workspace's code, credentials, and agent state must not be able to reach
another. Security reports are taken seriously.

## Reporting a vulnerability

**Please do NOT open a public GitHub issue for security problems.** Instead,
report privately via one of:

- **GitHub Security Advisory** (preferred): go to
  [Security › Advisories › Report a vulnerability](https://github.com/coelhucas-dev/work-cli/security/advisories/new),
  or
- **Email** the maintainer (add a private contact address here).

Please include:

- A description of the issue and its impact.
- Steps to reproduce, including the engine used (OrbStack / Docker / Podman / Colima).
- The `work doctor` output for the affected workspaces.

We aim to acknowledge reports within **72 hours**.

## Scope

We especially want to hear about anything that **weakens workspace isolation**,
for example:

- A path that lets one workspace reach another's network or volume.
- A way for a workspace to read host secrets, or another workspace's data.
- A privilege issue (running as root, host bind-mounts, published host ports).
- A case where `work doctor` passes but isolation is actually broken.

## Out of scope (by design)

These are explicitly **not** part of `work`'s threat model (see
`docs/superpowers/specs/2026-07-28-work-cli-design.md`):

- **Kernel / container-engine escapes.** `work` relies on the OS container
  boundary; the adversary is a *non-malicious* AI agent that may read the
  filesystem and persist memory — not a hostile workload targeting the kernel.
- **What users install inside their own workspaces.** `work` provides the
  sandbox; the user installs and authenticates their own tools and is
  responsible for their contents.
- **The host kernel or container runtime itself.** Report those upstream
  (OrbStack / Docker / Podman / Colima / Linux).

## Hardening you can verify

Run `work doctor` — it enforces, per workspace: a unique dedicated network, only
its own home volume mounted, non-root user, image matching config, and no
published host ports.
