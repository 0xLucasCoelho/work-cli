# Multiplatform portability and engine selection

**Status:** documentation/compatibility contract

**Scope:** define the supported host platforms, container-engine preference, and
verification obligations for `work`. This is a new portability note; it does
not rewrite or supersede historical design plans.

## Goal

Make `work` an open-source multiplatform tool whose user-facing contract is
about isolated Linux workspaces, not about a particular commercial desktop
runtime. The engine adapter may use Docker-compatible commands internally, but
the selected runtime is an explicit compatibility boundary.

## Supported host matrix

| Host | Supported path | Policy |
|---|---|---|
| Linux | Podman, preferably rootless; Docker-compatible fallback | Podman is the default preference. A Docker-compatible engine is supported when Podman is unavailable or explicitly selected. |
| macOS | Podman machine or Podman Desktop; Docker, OrbStack, and Colima | Prefer Podman. Keep Docker, OrbStack, and Colima compatible, but do not describe OrbStack as the default. |
| Windows | WSL2 Linux distribution with Podman inside the distribution | Install and run `work` and its engine inside WSL2. Native Windows containers, native Windows backends, and PowerShell execution are out of scope. |

Every workspace remains a Linux container with one named volume and one private
network. macOS and WSL2 provide the Linux execution layer; they do not change
the isolation invariants.

## Selection contract

Automatic selection is platform-aware:

1. Prefer Podman on Linux and inside WSL2.
2. Prefer Podman on macOS when its machine/backend is available.
3. Use a Docker-compatible fallback when Podman is unavailable.
4. Keep OrbStack and Colima as macOS compatibility selections, never as the
   portable default.

The detector must distinguish an installed CLI from a usable backend. A
present-but-stopped Podman machine or daemon should produce an actionable
diagnostic from `work doctor` or the first engine operation.

### `WORK_ENGINE`

`WORK_ENGINE` overrides automatic selection for the current process. Accepted
values are:

```text
podman
docker
orbstack
colima
```

Examples:

```bash
WORK_ENGINE=podman work doctor
WORK_ENGINE=docker work new acme
```

The override must select an installed, running backend. It must not silently
fall back to a different engine when the requested value is invalid,
unavailable, or stopped. It also must not imply that volumes or containers are
shared between daemons. Switching engines is a separate migration or recovery
operation.

## CLI and isolation semantics

All engine-facing lifecycle operations use the selected adapter: image build or
pull, volume and network management, container creation, exec, inspect, port
forwarding, and cleanup. The portability layer must preserve these invariants:

- one `work-<ws>` container per workspace;
- one `work-<ws>-home` named volume mounted only at `/home/dev`;
- one `work-net-<ws>` network and no published host ports;
- a non-root workspace user;
- `work doctor` checks the selected backend's actual state.

Documentation may show `docker run`, `docker inspect`, or similar commands only
as explicitly labeled Docker-compatible CLI examples. Podman users should be
shown the equivalent `podman` command or told to substitute the selected CLI.
The word “Docker” must not be used as a synonym for the engine abstraction.

## Rootless and machine guidance

Linux contributors and users should prefer rootless Podman and run `work` as the
same user that owns the Podman state. `sudo podman` and rootless Podman are
different engines from the user's point of view and must not be mixed casually.

Podman on macOS needs a running Podman machine. The setup and troubleshooting
docs must cover `podman machine list`, `podman machine init`, `podman machine
start`, and `podman info`. Podman Desktop is a supported way to manage that
machine.

WSL2 users must verify the engine from inside the target distribution. A Podman
or Docker installation visible only to Windows is not a supported backend for
`work`; native Windows containers are intentionally excluded.

## Verification requirements

Static checks and unit tests are necessary but insufficient. A real integration
run against each claimed engine path must exercise at least:

- engine selection and `WORK_ENGINE` failure behavior;
- image availability/build or pull;
- workspace create, inspect, attach, start, stop, and remove;
- isolation checks for network, volume, user, and published ports;
- browser/port bridge behavior where supported;
- cleanup after success and failure.

The ignored integration suite (`cargo test --workspace -- --ignored`) is the
repository's real-runtime evidence. `work doctor` is a diagnostic and invariant
check, not a replacement for those end-to-end tests.

## Documentation consequences

The README and setup guide should lead with Podman and the platform matrix.
Contributor guidance should require a live engine and identify WSL2 as the only
Windows path. Security reports should record the host platform, WSL2 status when
applicable, and the selected engine. Historical plans remain historical and are
not rewritten by this note.
