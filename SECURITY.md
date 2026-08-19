# Security Policy

`work`'s entire purpose is a **hard guarantee against cross-context data breach**:
one workspace's code, credentials, and agent state must not be able to reach
another. Security reports are taken seriously.

## Reporting a vulnerability

**Please do NOT open a public GitHub issue for security problems.** Instead,
report privately via one of:

- **GitHub Security Advisory** (preferred): go to
  [Security › Advisories › Report a vulnerability](https://github.com/0xlucascoelho/work-cli/security/advisories/new),
  or
- **Email** the maintainer at contact@lucascoelho.ai.

Please include:

- A description of the issue and its impact.
- Steps to reproduce, including the host platform, WSL2 status when applicable,
  and the engine used (Podman, Docker, OrbStack, or Colima).
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
  Report engine or kernel escapes to the relevant upstream project (Podman,
  Docker, OrbStack, Colima, WSL2, or the host OS) as well as to us when the
  `work` integration contributes to the exposure.
- **What users install inside their own workspaces.** `work` provides the
  sandbox; the user installs and authenticates their own tools and is
  responsible for their contents.
- **The host kernel or container runtime itself.** Report those upstream. The
  supported runtime boundary is platform-specific: Podman is preferred on
  Linux and WSL2, Podman machine/Podman Desktop is supported on macOS, and
  Docker-compatible engines remain compatibility paths. Native Windows
  containers and native Windows backends are out of scope.

## Hardening you can verify

Run `work doctor` — it enforces, per workspace: a unique dedicated network, only
its own home volume mounted, non-root user, image matching config, and no
published host ports.

For the full setup walkthrough, the threat model, and the runtime-neutral
flags (with a clearly labeled Docker-compatible CLI example) that enforce each
invariant, see
[`docs/SETUP_AND_ISOLATION.md`](docs/SETUP_AND_ISOLATION.md).
