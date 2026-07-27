# Crate maturity + dependency map — what can start, and what it is waiting on

Purpose: make the work queue **legible**, so nothing is started on a dependency that is not yet
mature. Snapshot at `d65eb75` (496 tests green). Line counts are implementation only.

## The map

| crate | lines | maturity | blocked on |
|---|---|---|---|
| `kayfabe-abi` | 9727 | **built** — 11 structs, 3 oracles, encode+decode swept | — *(incremental: RPC payload structs added per-message as GSP needs them)* |
| `kayfabe-core` | 8317 | **built + hardened** — identity family closed, mutation-checked | — |
| `kayfabe-linux-raw` | 6291 | **built** — the entire unsafe surface, 37 relaxations across 5 `*_unsafe.rs` | — |
| `kayfabe-rt` | 2556 | **built** — ranked locks, R1/R3/R5, executor | — |
| `kayfabe-vmm-kvm` | 2555 | **built, ONE OPEN DEFECT** | **#57** — R1 assert trips ~1-in-6 under race |
| `kayfabe-fwd` | 2304 | **built** — never exercised against a real GPU | hardware bring-up (phase 3) |
| `kayfabe-mocks` | 2156 | **built** — instruments; excluded from the mutation denominator | — |
| `kayfabe-isolate` | 1617 | **built** — cancel seam + conservation | — |
| `kayfabe-shell` | 995 | **built** — reactor, F1 as two numbers | — |
| `kayfabe-vmm` | 881 | **built** — the port; `GuestRamMap` prove-RAM | — |
| `kayfabe-completion` | 756 | **built** | — |
| `kayfabe-util` | 665 | **built** — `lockwitness`, `leafwitness` | — |
| `kayfabe-arch` | 556 | **built** | — |
| `kayfabe-mmu` | 309 | **built** | — |
| **`kayfabe-gsp`** | **34** | ★ **SKELETON — the critical path** | port plan in flight; then ABI payloads per-message; traces for *validation* only |
| `kayfabe-trace` | ~900 | **built** — vocabulary + sink port + one-counter ordering + budget counters + projection differential; 17 tests, every mechanism bite-checked | — *(incremental: plane call sites are threaded when a plane needs them; the GSP replay harness is its first real consumer)* |

## ★ The sequencing finding: `kayfabe-trace` is upstream of the GSP oracle

`kayfabe-trace`'s own doc says it will own *"the **replay format** the differential harness
consumes"* and a `TraceSink` trait. The GSP milestone's oracle **is** trace replay — record
BAR0 + RPC traces from the C, replay them into the Rust crate, assert identical responses.

⇒ **The GSP replay harness consumes the format `kayfabe-trace` is supposed to define.** Building
GSP's harness first would either duplicate that vocabulary or hard-code it in the wrong crate.

**Its dependencies are already mature**: the planes it must name — rmgraph apply, routing
decision, address bind/miss, doorbell dispatch, completion post/drain/poll, isolate verb — all
exist and are tested. So it is startable **now**, and it should land **before** the GSP replay
harness rather than beside it.

**★ DONE (2026-07-27).** It landed first, as sequenced. Two findings worth carrying forward:

1. **It had to sit BELOW `kayfabe-core`,** not beside it — every plane must be able to emit, so
   every plane must be able to depend on it. It therefore cannot name `RmEvent`, `ProcId`,
   `FwdFault` or `RmGraphError`. The bridge is a **trait the owning crate implements with an
   exhaustive `match`** (`kayfabe_core::trace`, `kayfabe_fwd::trace`), so a new variant upstream
   fails the build until the trace vocabulary names it.
2. **`IrqSpec` did not need inventing** — `kayfabe-vmm` already has the portable one
   (`Msix`/`IntxLevel`). The port plan's §6.1 sketch spells interrupts as the C's raise API;
   spelling them that way in a gated crate would have been a VMM-vocabulary breach.

Still open, deliberately: **no `&mut Trace` is threaded through any plane signature.** The
vocabulary is driven from the conformance suite's seam observer, which proves it can express
what the planes return and that the order is faithful; wiring a plane is a per-plane decision
with a real churn cost, and the first plane that needs it is the GSP queue.

## What is deliberately NOT started, and why

- **L2-Q (the QEMU adapter crate)** — its design is de-risked (BQL settled, ≥10.2 floor, the
  facilities inventory), but it would sit on `kayfabe-vmm-kvm`, which has **#57 open**. Building a
  hypervisor adapter on a memory plane with a known R1 violation under contention means any hang
  found later has two candidate causes instead of one. **Blocked on #57, deliberately.**
- **GSP trace capture** — bench work (GPU box, strictly serial, fresh boot per clean run, and the
  C's one-CUDA-process-per-lifetime bug to work around). Needed for *validation*, not
  *construction*, so it does not gate the GSP build. It does need the bench free.
- **`kayfabe-fwd` against real hardware** — that is phase 3 (bring-up), not a crate task.

## Slot discipline (the reason this file exists)

**One cargo per box.** A separate `CARGO_TARGET_DIR` is **not** sufficient isolation — `~/.cargo`
is shared, and a concurrent build corrupted a mutation run this way (`couldn't read metadata for
libkayfabe_isolate….rlib`). Two boxes ⇒ **two** concurrent cargo tasks, no more. Research and
docs agents are unlimited.

Every build task ends with **`./scripts/ci_gates.sh --all`** (`ALL GATES CLEAN`, 11 steps) before
it reports done — three pushes went out red because gates were reasoned about instead of run.
