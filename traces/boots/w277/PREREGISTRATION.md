# w277 — PRE-REGISTRATION

**Written before the boot.** Branch `w276-port-the-whole-vas-sweep`, base `03d6108`.
One variable: `KAYFABE_PT_SWEEP` (`on` / `off`), arming otherwise byte-identical to
`w271_pin` / `w276`.

## ⊘⊘ THE BANNER — this rung CANNOT move the wall, and says so first

Every change is an **instrument**. The refusal carries more fields; the settlement counts a
loss it used to swallow; the address table prints its own coverage. **Nothing binds
differently, nothing is relaxed, no policy moved.** ⇒ `CUP2_RC=124` with the fault unmoved is
the **expected** outcome. It is registered here so the null reads as the control it is rather
than as a disappointment — and so that a rung that *did* move something would be visible as a
surprise.

## THE QUESTION

`[measured, w276b_on, read from `traces/boots/w276/run_w276b_on_qemu.log.gz`]`
`refusals=255 by_kind={"StraddlesLiveBinding": 255}` on all 88 doorbells, payload = a VA and
nothing else. **Why do 255 bindings straddle, and where is the faulting VA dropped?**

## ARMS — the straddle's cause

| # | arm | fires if |
|---|---|---|
| A1 | **page-size mismatch** — a finer leaf over a coarser live binding | `live0x` extent is a page size the regime enumerates (`0x1000/0x10000/0x200000/0x20000000`) and `shape ∈ {InsideLarger, SameStartShorter}` dominates |
| A2 | **extent mismatch** — the live binding is an *allocation*, not a page | `live0x` extent is **not** an enumerated page size |
| A3 | **stale binding** — a shape neither containment explains | any `CrossesEnd` row |
| A4 | ★ **one fact at two granularities** — nobody is wrong | `contradicting=0` on every row |
| A5 | **two sources genuinely disagree** | `contradicting>0` |
| A6 | **the live side is HOST-PUBLISHED** ⇒ the fix is on the forwarding plane, not here | any `/pub1` signature |
| A7 | **aperture mismatch specifically** | `Contradicts` rows whose two phys values *do* line up (readable off `first_straddle`) |
| A8 | ⊘ **the 255 does not reproduce** | `straddles=NONE`, or a total ≠ 255 |
| A9 | ⊘ **more than 12 distinct signatures** — the space is not small and the instrument is the finding | `CAPPED at 12 of` present |

⊘ A1 and A2 are **not** exclusive with A4: a page-size straddle whose two shapes agree is both.
The pair `(shape, agreement)` is deliberately two axes because collapsing them is what made
`w276`'s single bucket uninformative.

## ARMS — the third outcome (the faulting VA is neither bound nor refused)

| # | arm | fires if |
|---|---|---|
| B1 | **dropped at a NAMED site** — the desired-set key collision | `shape_collisions>0` **and** a collision names the faulting VA |
| B2 | **collisions exist but not at the fault VA** | `shape_collisions>0`, fault VA absent from them |
| B3 | ★★ **the fault VA is BOUND** ⇒ *"our mirror missed the mapping"* is **dead** for it | `TABLE-DESCRIBES` join says **BOUND** |
| B4 | **UNBOUND, no collision, no refusal** ⇒ the third state is still unnamed and this rung fails to close it | join says **UNBOUND**, `shape_collisions=0` |
| B5 | ⊘ **NOT MEASURED** — the table run list capped, or no picture printed | join says so |
| B6 | ⊘ **duplicates without collisions** — two pages, identical leaves | `dup_leaves>0 shape_collisions=0` |

## ARMS — does anything move

| # | arm |
|---|---|
| C1 | `CUP2_RC=0` |
| C2 | `CUP2_RC=1` with bounded progress |
| C3 | ★ **`124`, same Xid identity, fault address ASLR-shifted but same low bits** — **the expected arm** |
| C4 | `124`, fault kind/engine **changed** ⇒ an instrument changed behaviour, which would be a defect in *this rung* |
| C5 | the armed and control arms differ in any carried counter ⇒ the instruments are not read-only |

## ⊘ WHAT THIS RUN CANNOT ANSWER, registered before it runs

- **Which of two colliding declarations SHOULD win.** RM's own walker makes the state
  impossible in a settled tree — `_mmuWalkResolveSubLevelConflicts` invalidates the other
  sub-table over the VA range on every map/unmap/sparsify (`ogkm-580:
  src/nvidia/src/libraries/mmu/mmu_walk.c:476-488, :1066-1092`) — and the source **nowhere**
  states what hardware does if a VA *is* valid in both. There is no ground truth to be right
  against, so this rung reports and does not choose.
- **Which source is guilty on a `Contradicts` row.** Only the *leaf's* level is carried; the
  live binding's producer is not recorded anywhere.
- **Whether reconciling would bind the fault VA** — nothing is reconciled here.
- One workload, one chip (GA106), one driver (`580.159.04`), one boot per arm.
