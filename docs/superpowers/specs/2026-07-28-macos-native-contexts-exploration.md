# `work` — macOS-Native Contexts: Exploration of Options

- **Date:** 2026-07-28
- **Status:** Exploration — **not for implementation**. Captures research and trade-offs for a possible future direction. No code is planned.
- **Owner:** Lucas Coelho
- **Project:** `work-cli` · open source (MIT) · Rust
- **Relates to:** `docs/superpowers/specs/2026-07-28-work-cli-design.md` (v1 product spec)

## The question

Could `work` offer **macOS-native** isolated contexts — running real macOS,
with the user's native toolchain (Homebrew, signed apps, `xcrun`, …) — that are
**as lightweight as Linux containers** (instant start, MBs of RAM, no full VM)?

Short answer: **no, not with a hard isolation guarantee.** macOS lacks the kernel
primitives that make lightweight containers possible. There *is* a lightweight
native option (Seatbelt sandboxes), but it is **soft** isolation resting on an
**unsupported** API, with a real network-isolation hole. This doc records why, so
the thinking survives and the decision can be revisited if Apple's platform moves.

## North-star priorities (from the maintainer)

1. **As light as possible** — no friction on the dev environment. Many concurrent
   contexts on a laptop.
2. **No compromise on the hard guarantee** unless it is an explicit, labeled
   trade-off. `work`'s reason to exist is "context A's data cannot reach B."
3. **Pragmatic acceptance** that running many heavy contexts may simply require
   more hardware — that is a fair user expectation, not a product failure.

These priorities are the lens for every option below.

## The hard constraint

Linux containers are cheap because the Linux kernel exposes **namespaces +
cgroups + overlay filesystems**: each container gets its own process tree,
network stack, and filesystem view while sharing one kernel.

**macOS has no equivalent.** The XNU kernel does not expose namespace isolation,
cgroups, or an overlay filesystem to userspace. There is no "macOS container"
primitive — you cannot carve a Mac into lightweight isolated macOS contexts the
way Docker carves Linux. This is a platform fact, not a gap `work` could close
with engineering effort.

Consequence: any macOS-native isolation must be built from weaker primitives
(sandboxing, separate accounts) or from a full VM.

## Options surveyed

| Approach | Real macOS? | Isolation | Weight | Feasible in `work`? |
|---|---|---|---|---|
| **Linux container** (shipped) | ❌ (Linux guest) | **Hard** (OS boundary) | Light | ✅ today — the sweet spot |
| **macOS full VM** (`tart` / Virtualization.framework) | ✅ | **Hard** (full guest OS) | **Heavy** (~4–8 GB RAM, ~50 GB disk, slow boot each) | Roadmap *engine*; 1–2 contexts, not many |
| **Seatbelt sandbox contexts** (`sandbox-exec`) | ✅ (host) | **Soft–medium** | **Lightest** (instant) | Experimental; unsupported primitive |
| **Separate macOS user accounts** (`sysadminctl`/`dscl`) | ✅ (host) | **Weak** (homes are 755 by default; shared system; no net isolation) | Light | Clunky lifecycle; needs sudo/admin |
| **`chroot` / `fakechroot`** | ✅ (host) | **Negligible** (escape-prone) | Light | Rejected |

## Deep dive: the lightweight candidate — Seatbelt sandbox contexts

This is the closest thing to "lightweight macOS containers," and it maps neatly
onto an idea nobody does well today.

macOS ships **Seatbelt** (`sandboxd`, a TrustedBSD MAC layer). You write a `.sb`
profile (a Scheme-dialect ruleset) and run a process under it with `sandbox-exec`.
A profile can say: *deny every path by default; allow only this context's
directory and read-only tool dirs; deny `~/.ssh` and the Keychain and every
sibling context; allow (or restrict) the network.*

### Sketch of a macOS-native engine

```
~/Library/Containers/work/<ctx>/          # each context's "home"
~/Library/Containers/work/<ctx>.sb        # generated Seatbelt profile
```

`work <ctx>` would run `sandbox-exec -p <ctx>.sb -- zsh -l`. The profile is a
deny-all allow-list (illustrative — real syntax has evolved and is partly
reverse-engineered):

```scheme
(version 1)
(deny default)
(allow process-fork process-exec
       (subpath "/usr/bin") (subpath "/bin") (subpath "/opt/homebrew/bin"))
(allow file-read* (subpath "/usr") (subpath "/opt/homebrew"))
(allow file-read* file-write*
       (subpath "/Users/you/Library/Containers/work/<ctx>"))
(deny file-read* file-write*
       (subpath "/Users/you/Library/Containers/work"))   ; block siblings
(deny file-read* (subpath "/Users/you/.ssh")
               (subpath "/Users/you/Library/Keychains"))
(allow network*)                                          ; or restrict
```

This **feels** like a container: the agent sees only its own dir, cannot read
siblings, cannot touch secrets — and it is instant, native, zero VM, MBs of RAM.
For `work`'s *non-malicious-agent* threat model, that is a real wall. It also
aligns conceptually with how macOS itself sandboxes apps (`~/Library/Containers/
<bundle-id>/Data`), so it reads as native rather than hacky.

### Why it is **not** a production foundation

| Hole | Impact |
|---|---|
| **No network isolation** | macOS has no network-namespace equivalent. You cannot give each context its own network/IP like Linux containers do — `work`'s "dedicated network per workspace" invariant **cannot be replicated natively**. Best available: `pf` filter rules per uid (filtering, not virtualization). |
| **Unsupported primitive** | `sandbox-exec` ships, but Apple has de-emphasized it (their modern path is signed App Sandbox via entitlements). The `.sb` format is minimally documented and **can change between macOS versions**, silently breaking contexts. Building a product's *hard guarantee* on an unsupported API is high-risk. |
| **Soft guarantee** | It runs on the **host kernel** over the **real filesystem** (a restricted view, not a virtualized one). A sandbox-escape bug or a kernel vulnerability = full host. (Linux containers share this caveat; Seatbelt's smaller surface cuts both ways.) |
| **Not a fresh macOS** | It is your real Mac with paths fenced off — no overlay filesystem, no per-context userspace, no clean-install feeling. |
| **Separate-user-accounts variant is weaker still** | macOS homes are `755` (+ ACL) by default, so credential separation alone does **not** stop one local user reading another's home without explicit `chmod 700` + Seatbelt on top. Clunky to create/delete at scale; needs admin. |

## Mapping to the north-star priorities

- **"As light as possible":** the only **light + hard-isolation** option is the
  **Linux container** (what `work` ships). Seatbelt is light but **soft**;
  macOS VMs are **hard but heavy**. There is no light + hard macOS-native option.
- **"No compromise on the guarantee":** a Seatbelt mode would be a **different
  posture** — "native macOS convenience, accept soft isolation." Coherent as a
  labeled tier (e.g. `--native`), but it **dilutes the core promise** if added
  silently. It should never become the default.
- **"Accept more hardware":** for users who genuinely need macOS-only tooling,
  the honest answer is a **macOS VM per context** (`tart`). Each is a full guest
  — running several means a beefier machine. That is a fair expectation to set,
  not a limitation to apologize for.

## Recommendation (for the record)

1. **Default stays Linux containers.** Strong isolation + light weight + the
   user's cross-platform toolchain (starship/zoxide/atuin/mise/zinit, shells,
   languages). This is the right answer for ~all current needs.
2. **macOS VM engine (`tart`)** is the legitimate roadmap item if/when there is
   real demand for macOS-only tooling (Xcode/iOS/signed native builds). It plugs
   into the existing engine abstraction. Position it honestly as heavy.
3. **Seatbelt native mode** is a fascinating **experimental** direction and a
   genuine market gap, but it is **soft isolation on an unsupported primitive**
   with a network-isolation hole. Worth a spike someday; not a foundation for
   `work`'s hard guarantee, and never a silent default.

## Roadmap status

- **Not pursued now.** No code, no plan. This doc exists so the research is
  committed and the trade-offs can be re-evaluated if Apple ships a real
  container primitive, or if macOS-only-tooling demand materializes.
- Re-open if: (a) Apple documents/stabilizes a lightweight isolation API beyond
  App Sandbox; (b) `tart`/Virtualization.framework becomes light enough for
  many concurrent guests; (c) a concrete macOS-only-tooling use case appears.

## Open questions

1. Does `tart`-on-Apple-Silicon get light enough (memory, boot time) to host
   3–5 concurrent macOS contexts on a high-end MacBook? Worth benchmarking before
   committing to a VM engine.
2. Is there a future where `pf` + Seatbelt together approximate a credible
   per-context network + filesystem boundary — enough to call it "soft-isolation
   macOS mode" without misleading users?
3. Would a macOS VM engine share enough with the `DockerCli` engine to fit the
   current `Engine` trait, or does VM lifecycle (clone, snapshot, IP, GUI vs
   headless) demand a distinct trait?
