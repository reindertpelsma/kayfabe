---
title: "kayfabe — the design, the plan, and what we know"
subtitle: "Stated as decided, 2026-09-20. Written to be argued with from scratch."
date: "branch w749-fable-legb"
---

\newpage

# How to read this

This is a **decision document, not a record of how the decisions were reached.** Everything here
is stated as it ended up, with the reasoning that supports it. Where an earlier draft said
something different, that is in git and not in these pages.

| part | what it is | attack it if… |
|---|---|---|
| **1. The design** | what kayfabe is, and how it works | you think a mechanism is wrong, or a simpler one exists |
| **2. The plan** | what gets built, in what order, and what proves each step | you think the sequence is wrong or a phase is mis-sized |
| **3. The surface** | every RPC, class, control, register and pushbuffer method we touch, by name and hex | you think something is missing, or we do the wrong thing with it |
| **4. What the oracles tell us** | what NVIDIA's open driver and nouveau establish, and where each stops | you think a claim is unsupported, or a source says otherwise |
| **5. Open questions** | what is genuinely undecided, and what each would cost | you think something here is actually decided, or something elsewhere should be here |

## The markers

| marker | meaning |
|---|---|
| ★ | a load-bearing fact — the argument rests on it |
| ⊘ | a refusal, a deletion, or a thing that is **not** true |
| ⚠ | a trap, or a claim whose evidence is weaker than it looks |
| `[MEASURED]` | a number from a real run |
| `[ogkm]` / `[nouveau]` | read out of that source, with file:line |
| **[PROPOSE]** | a design opinion, stated as one |

⚠ **Repetition marks how much something matters, not whether it is settled.** A settled question
says so in words.

## What this design is for

A guest runs a **stock, unpatched NVIDIA driver** against an emulated GPU and a firmware processor
that does not exist. Real compute reaches a real host GPU from an **unprivileged** host process.

The cut line is **below** D3DKMT and below `/dev/nvidia*` — lower than any shipping product cuts.
That is the whole difficulty and the whole point: an ioctl allowlist filters *what* is called,
while modelling the device forces us to know what it *means*.

★ **The value proposition is guest-internal isolation.** A sandboxed process inside the guest must
not be able to reach guest root through us. That single requirement shapes the trap path, the
doorbell plane, and most of what §5 of Part 1 says.

\newpage
