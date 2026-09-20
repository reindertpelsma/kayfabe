---
title: "kayfabe v3 — the architecture, the machine as it runs, and every constant we touch"
subtitle: "A working document, assembled 2026-09-20 (w821). Written to be argued with."
date: "branch w749-fable-legb"
---

\newpage

# Read this first

★★★ **Part 1 is the thing to argue with.** `[owner]` *"I'll look at your intended target
instead and argue with that."* Everything after it is the evidence that target rests on.

1. ★★★ **THE PLAN** — what gets built, in what order, what proves each step, the crate model,
   and an honest read on whether 50 k lines is reachable. **Read this first and attack it.**
2. **The v3 architecture** — the design the plan rests on, rewritten after the owner's review
   and two adversarial reviews of the worker protocol.
3. **The surface, at constant level** — every GSP RPC, RM object class, RM control command,
   BAR0 register and pushbuffer method we touch, **by its NVIDIA constant name and hex value**,
   each with a plain-language sentence for a reader who does not know NVIDIA's constants.
4. **ogkm residue** — what NVIDIA's open drop leaves readable about monolithic RM, pre-Turing
   and Windows. ★ Includes the BAR1/BAR2 split finding, which changes the memory plane.
5. **The Windows axis** — the `OS` axis, surveyed. ⊘ Nothing here is in the plan; it is scoping.
6. ⊘ **The machine, as it is** — the current tree, kept as a **record, not an argument**.
   `[owner]` *"not the point I'm arguing most with."* Skip unless you want the contrast.
7. **The surface we present (v2)** — the earlier plan-level inventory, kept for its rulings.
8. **The scrub question** — one open owner ruling, isolated because it is a live security defect.

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
| ✔ | ★ **resolved** — a question that has been answered or a defect that has been ruled on |

### ⚠ How to read repeated markers — added w821, because they were being misread

`[owner]` *"why is this three times ⊘ — haven't we already solved it?"* ★ **A fair question, and
the answer was that the marker was stale.** Repetition here means **severity**, and severity alone
says nothing about **status**. So, from now on:

- ⊘⊘⊘ / ★★★★★ mark **how much a thing matters**, at the moment it was written.
- ✔ / ⚠ mark **where it stands now**, and a heading **always carries one of them** once its status
  is known.
- ⊘ **A finding that has been ruled on keeps the severity it had when found** — that is the record
  of why it mattered — **but its heading changes to ✔ and names the ruling.** The severity is
  history; the status is current.

⚠ This is the same failure this project already documents for design docs: *a correct document
that stopped being true and did not say so*. A stale severity marker is that failure in miniature —
it reads as an open catastrophe long after the catastrophe was closed.

⚠ **Nothing in this bundle is filled in from memory of NVIDIA's headers.** Every constant name,
number and structure layout is cited to a file and line in either this tree or the open kernel
modules under `research_clones/ogkm`. Where a survey could not establish something, it says
**[UNVERIFIED]** rather than guessing — that convention is load-bearing, because this project's
most expensive recurring failure is a plausible value that was never measured.

\newpage
