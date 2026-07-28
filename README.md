# `work`

Isolated multi-context session manager for developers — run multiple fully
isolated coding contexts (one persistent Linux container per workspace) on a
single machine, with a structural guarantee against cross-context data breach.

> Status: early development. See
> [`docs/superpowers/specs/2026-07-28-work-cli-design.md`](docs/superpowers/specs/2026-07-28-work-cli-design.md)
> for the design and
> [`docs/superpowers/plans/2026-07-28-work-cli-v1.md`](docs/superpowers/plans/2026-07-28-work-cli-v1.md)
> for the implementation plan.

## What it is

Each workspace is a persistent container with its own named volume
(mounted at `/home/dev`) and its own dedicated bridge network. Code, AI
agents, and credentials in one workspace physically cannot reach another.
`work` does **not** install tools and does **not** manage credentials — you
install and authenticate your own tools inside the container.

## License

MIT.
