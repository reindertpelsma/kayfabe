# Archive — superseded architectures, kept as reference

**These documents do not describe how kayfabe works.** They describe how it *used to*, or how it
was once proposed to. The live design is **`../design/THE_DESIGN.md`**.

## Why they are kept rather than deleted

This tree's most expensive recurring failure is **a correct document that stopped being true and
did not say so** — measured five times in two days, including a ruling superseded the next day
that sent two bench lanes at work already proved unnecessary. Deleting these would trade that
failure for a worse one: re-deriving findings that were already paid for.

⇒ **What remains valuable in here:**

- ★ **Measurements.** Every number taken on real hardware, with the run that produced it. These
  do not expire when an architecture does.
- ★ **ogkm findings.** What NVIDIA's driver does, read out of its source. Independent of our design.
- ★ **Reasoning.** *Why* a thing was tried and what it cost. The architectures changed; the
  constraints that forced them mostly did not.

⊘ **What is NOT valid in here:** the architecture itself — isolates and the IPC plane, the address
table, joins, per-process containers, publication epochs, the dirty gate, the CPU copy executor,
the completion watch. Every one of those is deleted in the live design.

## How to read a file in here

Each one opens with a banner saying it is archived. ⚠ **If you find a file in here without that
banner, it was moved by hand and its status is unverified** — treat it as archived anyway.

⊘ **Do not cite a file in here as current**, and do not implement from one. If something in here
is still true and still matters, the fix is to state it in the live design — not to link here.
