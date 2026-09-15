# w746 — the hand-over's ENSURE path: PREDICTIONS, committed BEFORE the box exists

**STATUS: LIVE, 2026-09-15.** Written and committed with **no box rented**. `vastai show
instances` is empty at the moment of this commit; `scripts/bench/w736_fbstore_run.sh` has not
been run at this revision and no log exists.

⊘ Predictions are not evidence. They exist so a result cannot be read as whatever is convenient
afterwards. Each row carries the value that **refutes** it.

## ⊘⊘ WHAT IS ALREADY MEASURED, AND NEEDS NO BOOT

Two findings in this increment are derived from **w745's own committed artifacts**
(`traces/w745_split/w745_evidence.tgz`) and are not predictions:

1. **The 4619 refusals were `Vas::host_vas == None`, not a mis-routed PDB.** The split arm's
   `VERBCOST` names `EngineObject n=8 … 100.0%` — the only verb that ran — and every engine
   object was refused (`forwarded=0 refused=10`, against `forwarded=8 refused=2` on arm 2,
   same binary). No verb that mints `host_vas` ever committed.
2. **`W745-BARE-SPACE-REFUSED = 0` was vacuous.** `alloc_channel_in` maps its ring through
   `RmConnection::raw_map_dma`, which carried no bare-space refusal; the four `RmBackend`
   verbs that did are not on that path. The row w745 pre-registered as the confirmation of
   its own amendment **could not fire**.

## ⚠ THE HEADLINE PREDICTION, AND IT IS AGAIN A NEGATIVE ONE

**I predict the raw client does NOT reach `(P)` on arm 3.** Two reasons, both structural and
both stated before the boot:

- **Leg B is still declined for a store slice.** `h_userd_memory_0` is a real RM operand and a
  per-proc client holds no handle for the scratchpad's object. A channel born with OUR USERD
  while the guest advances `GP_PUT` in its own framebuffer page is the `GP_PUT == GP_GET`
  silence. ⊘ This is an **owner ruling**, not a thing to improvise (brief, "Leg B / the USERD").
- **Ordering.** `[w745 log lines 141, 227, 325 vs 960]` the first engine objects arrive
  **before** the first `VAS-PUBLISH`. Those births still take `RingSource::Ours`, still need a
  map through a bare space, and are now refused **by name** rather than by RM's `0x51`. A
  hand-over that works does not make them retroactively adoptable.

⇒ **What this boot is for is rows 5–8**: does the wall MOVE from "the hand-over refused 4619
times" to something downstream of it, and is the new wall NAMED?

## The rows

| # | line | predicted | ⊘ REFUTED BY |
|---|---|---|---|
| 1 | ★★★★★ ARM 1 `W392D_GUEST_OUTCOME=` | **`(P)`, `THREADS 8 of 8`, `MEAN_FALSIFIER=PASS`** | anything else ⇒ **the binary is broken and NO other row on any arm is interpretable.** Read this first |
| 2 | ★★★★★ ARM 2 the four frozen numbers | `(6)=17`, `BIRTH REFUSED=11`, `DoorbellBirth=19`, `ADOPTING=0` | **any other value** ⇒ this binary moved the device arm by itself and arm 3 is not attributable. ⚠ I expect them UNMOVED: nothing in w746 runs on arm 2 — `store_owns_vas()` is false, and no space on that arm is bare |
| 3 | ARM 3 `W746-CONTENT` | `handover_asserts≥1 refusal_line≥1` | `0` ⇒ the bench served an older binary; **stop, nothing below is about this revision** |
| 4 | ARM 3 `W746-ASSERTS-LINE` | **1** | `0` ⇒ the census never printed, so every w746 number is UNMEASURED rather than zero |
| 5 | ★★★★★ ARM 3 `W746-HANDOVER-MINTED` | **≥ 1** | `0` ⇒ **THE FIX DID NOT FIRE.** w745 measured `0` here with 4619 refusals; a `0` again with the refusals gone would mean the publish route stopped reaching the port, which is a different and worse finding. **THE MECHANISM ROW** |
| 6 | ★★★★★ ARM 3 `W745-ADOPTS` / `W745-MAPS` | **≥ 1 each** | `0` ⇒ the hand-over succeeded and the scratchpad still mapped nothing; read `W745-MAP-REFUSED` and `W745-SM-FIRST` next. ⊘ `maps ≥ 1` is the first time in this campaign that `adopt_vaspace`, `map_store_slice` and constraint 28 are exercised **on hardware at all** |
| 7 | ★★★ ARM 3 `W745-BARE-SPACE-REFUSED` | **≥ 1** | `0` ⇒ ⚠ either the emulated births genuinely need no map (w745's third reading, now testable because the gate is on the path) **or** the gate is still unreachable and `ownership_split_gates` is asserting the wrong site. Non-zero is the **confirmation of w745's own amendment**, a year of reading away from where it looked |
| 8 | ★★★ ARM 3 `W745-ADOPTWHY-6` | **< 17** | `17` ⇒ the binding still carries no host object and the whole chain is inert downstream of a hand-over that worked — which would be a NEW wall and the next increment's subject |
| 9 | ARM 3 `W745-RING-NOT-A-SLICE` / `W745-ASSERTED` | **not predicted**, and the PAIR is what is read | ⚠ `RING-NOT-A-SLICE=0` with `asserted=0` is **vacuous**, exactly as at w745. Only `asserted ≥ 1` makes a zero here a pass. The `HANDOVER-ASSERTS` line states which |
| 10 | ⊘ ARM 3 `W746-ROUTE-DISAGREES` | **0** | ≥1 ⇒ **a DEFECT**, not a transient: the caller's `ProcId` and the spine disagree, and the subtraction this increment made was wrong |
| 11 | ⊘ ARM 3 `W746-C30-REFUSED` / `W746-C30-BIRTH-REFUSED` | **0 / 0** | ≥1 either ⇒ constraint 30 fired on real traffic. ⊘ That is the gate WORKING, not a bug — and it would mean a space or a birth crossed the privilege boundary and must be reported to the owner before anything else |
| 12 | ⊘ ARM 3 `W746-RING-HANDLE-REACHED-RM` | **0** | ≥1 ⇒ the premise on which `AdoptedGuestRing::memory` was deleted has stopped holding |
| 13 | ★★★★★ ARM 3 `W392D_GUEST_OUTCOME=` | ⊘ **`(R)`, `THREADS 0 of 8`** — see the headline | `(P)` ⇒ **stop and report immediately**, per the brief |
| 14 | ARM 3 `RmInitAdapter failed!` / `SMI_RC=` | **`0` occurrences / `0`** | ≥1 / non-zero ⇒ a REGRESSION. ⚠ This is the row I am least sure of: the emulated kernel channels' births now fail **by name** where they previously failed with RM's `0x51`, and a named refusal is not a *different* outcome — the birth failed either way. If this moves, the naming changed something it should not have |
| 15 | ARM 3 bar1/bar2 `TRAP_FILLS=` / `misses=` | **0 / 0** | ≥1 either ⇒ regression against w743/w745 |
| 16 | ARM 3 failing-test-name set | identical to the **30**-name baseline | any name added or removed ⇒ report it, with the name |
| 17 | ★★ `W745-VCPU-BLOCKING` arms 2 and 3 | **not predicted** — the brief asks for the number. w742/w745 measured `total=197 doors=9` | ⚠ a RISE on arm 3 is a cost of this change and must be reported as one. The hand-over declines on a vCPU by name and that is unchanged |
| 18 | ARM 3 `W745-WITHHELD-UNMAPS` / `pending=` | **not predicted**; `pending=true` at teardown IS a defect | — |

## ⊘ What this boot CANNOT say, stated before it runs

- **Nothing about the USERD ruling.** Leg B is declined by construction on this arm, so a `(R)`
  cannot distinguish *"the split is wrong"* from *"the split is right and the cursor is not
  carried"*.
- **Nothing about whether a scratchpad-placed MAPPING conveys the scratchpad's privilege.**
  Constraint 30 part 2 asks it; this increment refuses the one shape ogkm has already answered
  (a channel birth) and leaves the mapping question `[UNMEASURED]`. ⚠ It is **not** answered by
  `maps ≥ 1` succeeding — constraint 30 part 2 says so in as many words.
- **Nothing about throughput.** No workload here is a perf measurement.
- It is **one driver on one chip**: `580.159.04` on a GA106.
