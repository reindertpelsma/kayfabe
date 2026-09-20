---
title: "kayfabe v3 — the architecture, the machine as it runs, and every constant we touch"
subtitle: "A working document, assembled 2026-09-20 (w821). Written to be argued with."
date: "branch w749-fable-legb"
---

\newpage

# Read this first

This bundle is five documents, in the order they should be read:

1. **The v3 architecture** — the design I propose, rewritten after the owner's review and
   after an adversarial review of the worker protocol. Short, and the part to attack first.
2. **The surface, at constant level** — ★ *new in this edition.* Every GSP RPC, RM object
   class, RM control command, BAR0 register and pushbuffer method we touch, **by its NVIDIA
   constant name and hex value**, each with a plain-language sentence saying what it *is* for a
   reader who does not know NVIDIA's constants, and what we do with it.
3. ★ **The Windows axis** — *new in this edition.* The first survey of the `OS` axis, which
   carried **zero coverage** until now. ⚠ Read it before costing any Windows work: the headline
   is a **premise** problem, not a compatibility one.
4. **The machine, as it is** — what the code actually does today, from a `file:line` survey.
   Every place the code contradicts the proposal is marked.
5. **The surface we present (v2)** — the earlier plan-level inventory: KEEP / DELETE per item.
   Kept because the *plans* in it are still the plans; document 2 supersedes its *data*.
6. **The scrub question** — one open owner ruling, isolated because it is a live security
   defect and it is the smallest change on the list.

## What changed since the v2 bundle (w818)

- ★★★ **A second privilege boundary.** `[owner]` The doorbell is adversarial **to the guest's
  own root**, because unprivileged guest userspace writes it. This is not hardening bolted on —
  it restructures the entire vCPU trap path. v3 §2.
- ⊘ **The doorbell hint queue and every FULL_REFRESH-shaped fallback are DELETED**, replaced by
  a 64 KiB bit table. Deleted from the *proposal*, before any code was written.
- ⊘ **Per-vCPU register rings are refuted** — they preserve per-vCPU order where the device
  needs the guest's cross-vCPU order.
- ★ **The worker wakeup protocol is fully specified**, with its bit widths, its layout, its
  memory orderings and the interleavings that break it.
- ★ **This document, part 2** — the constant-level reference the v2 bundle did not have.

## The markers, used consistently throughout

| marker | meaning |
|---|---|
| `[MEASURED]` | a number from a real boot, with the run that produced it |
| `[CODE]` | read out of the tree, with `file:line` |
| `[ogkm]` | read out of NVIDIA's open kernel modules, with `file:line` |
| **[PROPOSE]** | a design opinion. These are the arguable ones. |
| **[UNVERIFIED]** | the survey could not establish it. Deliberately not filled in with a guess. |
| ⊘ | a refusal, a deletion, or a thing that is *not* true |
| ★ | a load-bearing fact |
| ⚠ | a trap, or a claim whose evidence is weaker than it looks |

⚠ **Nothing in this bundle is filled in from memory of NVIDIA's headers.** Every constant name,
number and structure layout is cited to a file and line in either this tree or the open kernel
modules under `research_clones/ogkm`. Where a survey could not establish something, it says
**[UNVERIFIED]** rather than guessing — that convention is load-bearing, because this project's
most expensive recurring failure is a plausible value that was never measured.

\newpage
