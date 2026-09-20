---
title: "kayfabe — the v2 architecture, the machine as it runs, and the surface we present"
subtitle: "A working document, assembled 2026-09-20 (w818). Written to be argued with."
date: "branch w749-fable-legb"
---

\newpage

# Read this first

This bundle is four documents, in the order they should be read:

1. **The v2 architecture** — the design I propose. Short, and the part to attack first.
2. **The machine, as it is** — what the code actually does today, from a `file:line` survey.
   Every place the code contradicts the proposal is marked.
3. **The surface we present** — every GSP RPC, RM control command, MMIO register and
   pushbuffer method we implement: what it *is*, what we do with it, and the plan for it.
4. **The scrub question** — one open owner ruling, isolated because it is a live security
   defect and it is the smallest change on the list.

## The markers, used consistently throughout

| marker | meaning |
|---|---|
| `[MEASURED]` | a number from a real boot, with the run that produced it |
| `[CODE]` | read out of the tree, with `file:line` |
| **[PROPOSE]** | a design opinion. These are the arguable ones. |
| **[UNVERIFIED]** | the survey could not establish it. Deliberately not filled in with a guess. |
| ⊘ | a refusal, a deletion, or a thing that is *not* true |
| ★ | a load-bearing fact |
| ⚠ | a hazard, or a place a future edit would break something |

## What is knowingly missing

⊘ Stated up front, because a document that hides its gaps cannot be argued with:

- **Registers are GA106 only.** Three other chip modules exist and were not enumerated.
- **The 160-row capability allowlist is not transcribed.** It gates *admission*, not service.
- **A second `PushMethod` consumer** (`kayfabe-rt/src/ceutils.rs`) was confirmed to use the
  same vocabulary but **not** audited for additional method addresses.
- **Three live contradictions are left in, unresolved**, because only one side of each can be
  true and I could not determine which. They are flagged where they appear; the most
  consequential is whether guest MMIO traps run with the QEMU BQL held.
- **No performance model.** The one number here is a 205.7 µs page-table walk and a 1.62 s
  worst-case trap; there is no throughput or syscall-cost measurement for the proposed design.

## Where the code actually stands, by the numbers

`[MEASURED 2026-09-20]`

| | lines |
|---|---|
| `crates/*/src` — code | **144,175** |
| `crates/*/src` — comments | 125,503 (45 % of the file bytes) |
| tests (`crates/*/tests` + `tests/`) | 182,052 |
| **total Rust** | **463,996** |

⇒ A 50k-line target is a **~3x reduction of actual code**, not 9x — the comment density is
doing a lot of the apparent bulk, and the comments are the part worth keeping. The deletions
proposed here (joins, the address table, VA translation, the CPU executor, the isolate IPC)
are concentrated in the four largest crates: `kayfabe-isolate-host` (46,895), `kayfabe-abi`
(43,588), `kayfabe-qemu-raw` (37,057) and `kayfabe-device` (27,496).

\newpage
