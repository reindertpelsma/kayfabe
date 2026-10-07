# w290 LEG 8 — THE PUBLICATION IS BUILT, IT WORKS, AND THE WALL MOVED BY IDENTITY

Three boots at `9af4d8c1`+, stamp gate PASS on each. ⊘ **RELAXED ARMS, LABELLED:**
`KAYFABE_PT_SWEEP=on` + `KAYFABE_OPERAND_JOIN=join` (carried from `w290cup2`) plus
`KAYFABE_VAS_PUBLISH=assert|publish`. A relaxed green is never the milestone.

| boot | arm | published | `host_rows` | Xid, BY IDENTITY | `CUP2_RC` |
|---|---|---|---|---|---|
| `w290passert` | `assert` (**control**) | 0 | 4 of 16425 | `GRAPHICS HUBCLIENT_FE @ 0x72ba_e0e00000 FAULT_PDE **WRITE**` ch `0x9` | **1** |
| `w290ppublish` | `publish` | **34** | **34 of 16426** | `**CE2 HUBCLIENT_CE0** @ 0x7c43_fe500000 FAULT_PDE **READ**` ch `0x1000011` | **1** |
| `w290ppublish` (rerun) | `publish` | **34** | **34 of 16426** | `**CE2 HUBCLIENT_CE0** @ 0x7bb3_0a500000 FAULT_PDE **READ**` ch `0x1000011` | **1** |

★★★★★ **PRE-REGISTERED OUTCOME (3) — PUBLICATION WAS NECESSARY AND NOT SUFFICIENT.** Two boots
from one binary differing only in the arm, so the delta is attributable; and the new wall
**reproduces across two independent publish boots** — different engine (GRAPHICS→CE2),
different client (FE→CE0), different access (WRITE→READ), different channel. Both faults land
at **run base + `0x500000`** of a `0xc00000` run. **That is a full result.**

## ⊘⊘⊘ LEAD: THREE THINGS THAT CONTRADICT THE BUILD BRIEF, ALL MEASURED

### 1. ★★★★★ MY OWN `w290` HEADLINE WAS TOO STRONG — `host_rows` READS ONE OF **TWO** RECORDS

`commit_pin_guest_ram` (`kayfabe-fwd/src/lib.rs:1886-1893`) inserts into `Vas::guest_ram_pins`
and **never calls `table.bind`, never sets `Binding::host`**. A guest-RAM range that *is*
mapped in the host VAS at the guest's own VA by a real `OS_DESCRIPTOR` is therefore
**invisible** to the row I reported. Measured after the fix: cup2's VAS carries
**`already_pinned=57`** rows the old instrument could not see.

⇒ *"The host VAS the GPU walks is empty"* was **overdrawn**. The correct statement is: the
host mapping state lives in **two disjoint places** and my instrument read one of them.
`HOST-PUBLISHED` now prints `host_rows=… pins=…` and the census carries `already_pinned` as
its own bucket. ⚠ Same class as `a_second_source_of_truth_beside_a_complete_value`.

★★ **And this is the owner's point arriving from a third direction.** The owner recalls the C
carrying the host mapping *as a field in its object structure*; `promote.rs:1180-1187` says the
handle *"is not carried to the bind"*; and this says the one place a handle **does** exist for
guest RAM keeps it **outside `Binding` entirely**. Three lines, one gap.

### 2. ⊘ "COALESCE BY RUN, PUBLISH EXTENTS" IS BLOCKED BY THE VERB ITSELF

`plan_back_fb_leaf` refuses on three grounds before any host verb exists, and two pull opposite
ways: `FbLeafGranularity` (`kayfabe-fwd/src/lib.rs:2244-2247`, *"RM places a fixed mapping in
64 KiB granules"*) **wants** a run; `FbLeafExtent` (`:2352-2358`) requires the request to be
**exactly one table row** and so **forbids** one. The proven verb cannot be handed a run.
Measured consequence: `not_granular=6 (1 021 440 bytes)` — coalescing would have rescued
**6 rows**, not the bulk. Widening `FbLeafExtent` is a real change to the fwd commit (one host
object writing `host` into many rows, and the reclaim frees per row); it was **not** smuggled
into this rung.

### 3. ⊘ THERE IS NO UVM PLANE TO KEY CLEANUP ON — AND CLEANUP ALREADY SHIPS

`uvm_release` / `uvm_va_space_destroy` / `uvm_va_space_mm_shutdown` are **not observable events
in this port at all**: we emulate a *GPU*, so the guest's `nvidia-uvm` talks to the guest's
`nvidia.ko` and reaches us only as `RpcFunction::Free` (fn 10, `kayfabe-gsp/src/rpc.rs:261`) ⇒
`RmEvent::Free` ⇒ `Spine::refresh`. A `SIGKILL`ed guest process still gets there, because the
guest's own `nvidia.ko` `close()` frees the client root.

★★★ **The unpin exists and needs no new code.** `Spine::stage_dropped_vases`
(`kayfabe-core/src/gpu.rs:3229-3273`) walks `vas.table.iter()` and stages `unmap`-then-`free`
for **every** binding whose `host()` is `Some`, reached from `Spine::vacate` (`:3645-3664`),
*"THE ONE REMOVAL POINT"* (`:3622`), on all three routes: VAS dropped while the proc lives
(`sync_proc_to_boundary`, `:3117`), clean proc death (`:3903`), violent death (`retire_proc`,
`:4181-4225`, backstopped by the isolate process boundary, `:1812-1817`). Leg 8 mints nothing
new — it drives the existing `join_one_fb_leaf`, so its rows are ordinary `Binding::host` rows
and are reclaimed by that walk. ⚠ **Residual, named:** no per-leaf release short of VAS death.
Pre-existing and shared with leg 7, whose own doc says *"the missing half is the trigger, not
the mechanism … ⊘ Not wired this rung"*. Its cost was measured: `RepointsPublished=1`,
`UnbindsPublished=1` on the publish boot — the guest tried to edit a frozen row **twice**.

## THE CENSUS — cup2's VAS, `proc=2 pdb=0x201000`, after publication

```
total=16426  already_host=34  already_pinned=57  guest_ram=16328
not_vidmem=0  not_granular=6(1021440 bytes)  candidates=1  published=1  sum_ok=true
```

★★★★★ **`guest_ram=16328` — 99.4 % of the table.** The rows the fault needs are **guest RAM,
not framebuffer**, and `back_fb_leaf` is the wrong verb for them by construction. The FB half of
the publication is now **complete**: `candidates=1` at the end means there is nothing left it
can take.

⚠ **The `map_dma` numbers, as asked, and they are the good news:** 34 publications in
**63 + 13 + 12 + 7 + 6 = 101 ms total**, **zero refusals**, **zero budget exhaustions**, over
156 doorbells whose steady state is `published=0 refused=0 in 0 ms`. ⇒ **Not slow. Not
exhausted.** Outcome (1) and (2) did not fire.

## ⊘ A DEFECT CAUGHT BEFORE IT BURNED A BOOT — `ProcId(0)` IS `SYSTEM_PROC`

`plan_back_fb_leaf` refuses `Gpu::SYSTEM_PROC` by name (§12.26, `kayfabe-fwd/src/lib.rs:2318`).
Measured: proc 0 holds **6787 rows**, and its `pdb=0x2efa9c000` alone offers
**`candidates=6144 (12 884 901 888 bytes — the whole 12 GB framebuffer)`**. Handing those to
the verb would have issued thousands of doomed round trips and reported them as `refused=`,
which reads exactly like RM exhaustion and is nothing of the kind. The pass now states the
refusal **once, as a property of the proc**, and still prints the census so the rows are visible
rather than absent.

## ★★★ THE BIND-SITE MAP THE OWNER ASKED FOR — THE OMISSION IS SYSTEMIC

`AddressTable` has exactly one insert (`bind`), and `Binding` exactly two constructors:
`declared_by_guest` (`host: None` is a **literal**, `kayfabe-mmu/src/lib.rs:625`) and
`real_gpu_memory` (`host: Some`, `:683`).

| # | site | source | ctor | `host` | gap |
|---|---|---|---|---|---|
| 1 | `core/gpu.rs:3072` | RPC `MapMemoryDma` sync | `declared_by_guest` | `None` | SUPPLY |
| 2 | `core/promote.rs:1190` | `GPU_PROMOTE_CTX` | `declared_by_guest` | `None` | SUPPLY — *self-documented at `:1178-1186`* |
| 3-4 | `mmu/walker.rs:1071`, `:1101` | PT decode / CE capture / whole-VAS sweep — **the 16 000-row bulk** | `declared_by_guest` | `None` | SUPPLY by design (`:1013-1017`) |
| 5 | `fwd/lib.rs:2146` (`commit_publish`) | `publish_backing` | `real_gpu_memory` | **`Some`** | ⊘ **no production caller** |
| 6 | `fwd/lib.rs:2826` (`bind_backed_fb_leaf`) | fb-leaf adopt | `real_gpu_memory` | **`Some`** | env-gated, narrow — **the one leg 8 widened** |
| — | `fwd/lib.rs:1886` (`commit_pin_guest_ram`) | guest-RAM pin | **never binds** | n/a | ★ **PLUMBING** — handles in scope, parked in a second map |

⇒ **7 production bind sites, 5 sources; only 2 can ever set `host`, one of them unreachable and
one env-gated.** That is why `host_rows=4 of 16425` — not an oversight at one call site.
⇒ ★★★ **And it explains the promote lead cleanly rather than only refuting it by measurement:**
a completed two-phase join binds through site 2 and therefore describes a mapping that exists
**only in our bookkeeping**. The join was doing its job. It had nothing host-side to record.

## ⇒ THE SUCCESSOR, NAMED BY MEASUREMENT

The fault is at `run_base + 0x500000` inside a `0xc00000` run that `GUEST-DESCRIBES` and
`TABLE-DESCRIBES` own and `HOST-PUBLISHED` does not — and those rows are in the
**`guest_ram=16328`** bucket. ⇒ The next step is **the guest-RAM half of the same idea**:
`pin_guest_ram` (`kayfabe-fwd/src/lib.rs:1575`) driven over the VAS the way leg 8 drives
`back_fb_leaf`, **and its result written into `Binding::host`** so the two records become one
field — exactly what the owner recalls the C having.

⊘ Do not re-derive: the parked halves (retired in `RESULT.md`), 64 KiB rounding,
`GET_PTE_INFO`, copy-and-swap promotion, ioctl divergence, run-coalescing (blocked by
`FbLeafExtent`).

**Still owed:** criterion 1's address half (`CONTROL-NEVER-LANDED`).
