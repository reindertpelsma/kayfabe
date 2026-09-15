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

---

# ✔✔✔ MEASURED 2026-09-15 (w746), ON HARDWARE, THREE ARMS, ONE BINARY

`[vast **51155860**, RTX 3060 **GA106**, Quebec CA, host driver **580.159.04 OPEN**,
`TREE_REV = 0c0dd2ce` on all three arms, control first; evidence `w746_run.log` +
`w746_evidence.tgz`]` ⊘ No constraint was relaxed. No completion was forged.

## ⇒ THE CLIENT, FIRST

| arm | store / VAS owner | client |
|---|---|---|
| 1 | `arena` / `isolate` | **`(P)` — `THREADS 8 of 8 ✔`, `MEAN_FALSIFIER=PASS`** |
| 2 | `device` / `isolate` | `(R)` — 0/8, four frozen numbers **reproduced byte-identically** |
| 3 | `device` / **`scratchpad`** | ⊘ **`(R)` — 0/8** |

⊘ **THE GATE IS NOT MET.** Row 13 predicted exactly this, and the reason predicted (leg B) is
**not** the reason it happened — the wall moved one step and stopped somewhere new.

★ **Arm 2 reproduces `(6)=17`, `BIRTH REFUSED=11`, `DoorbellBirth=19`, `ADOPTING=0`** — the same
four numbers as w740/w742/w743/w745, on **this** binary. ⇒ arm 3 is attributable to the split.

## ⇒ ★★★★★ THE WALL MOVED, AND THE NEW ONE IS RM'S OWN

`HANDOVER-ASSERTS asked=4637 leaf_untabled=0 space_not_held=0 ⇒ ★★★ ASKED AND PASSED`

**The hand-over now runs.** It minted a bare space, committed it, and returned it — **4637
times** — and every one of constraint 29's and 30's asserts was **exercised and did not fire**:

    W746-ROUTE-DISAGREES=0          (assert 1 — and see below: this also refutes w745's diagnosis ON HARDWARE)
    leaf_untabled=0                 (assert 2 — every leaf offered WAS a row of the routed Vas)
    space_not_held=0                (assert 3)
    W746-C30-REFUSED=0              (constraint 30 — the space was always the asking proc's own isolate's)
    W746-C30-BIRTH-REFUSED=0        (constraint 30 — the scratchpad birthed in no adopted space)
    W746-RING-HANDLE-REACHED-RM=0   (constraint 29 — the deleted field's premise still holds)

⊘ **These are `asserted-and-passed`, not `never-asked`** — `asked=4637` is printed beside them
for exactly that reason, and it is the row w745's `RING-NOT-A-SLICE=0` did not have.

### THE NEW WALL: `NV_ERR_INSUFFICIENT_PERMISSIONS` ON THE DUP

    STORE-MAP … adopts=0 adopt_refused=4637 … first_refusal=[Rm("InsufficientPermissions")]
    VAS-PUBLISH(proc=2 pdb=0x0) → ⊘⊘ CONSTRAINT 26: the scratchpad REFUSED the hand-over:
                                    Rm("InsufficientPermissions")

`adopt_vaspace` — `NV_ESC_RM_DUP_OBJECT` of the isolate's `FERMI_VASPACE_A` into the
scratchpad's client — **was issued for the first time in this campaign** and RM refused it.
⊘ w745 measured `adopts=0 adopt_refused=0`: the verb was never reached.

★★★ **ogkm names the gate exactly** `[ogkm-580.159.04,
src/nvidia/src/libraries/resserv/src/rs_client.c:537-551, `clientCopyResource_IMPL`]`:

```c
if (((pParams->pSecInfo->privLevel < RS_PRIV_LEVEL_KERNEL) ||
     (pParams->flags & NV04_DUP_HANDLE_FLAGS_REJECT_KERNEL_DUP_PRIVILEGE)) &&
    (pServer->bRsAccessEnabled || (pParams->pSrcClient->hClient != pClientDst->hClient)))
{
    RS_ACCESS_MASK_ADD(&rightsRequired, RS_ACCESS_DUP_OBJECT);
    status = rsAccessCheckRights(pParams->pSrcRef, pClientDst, &rightsRequired);
}
```

⇒ a **cross-client** dup by a client that is **not** `RS_PRIV_LEVEL_KERNEL` needs
`RS_ACCESS_DUP_OBJECT` **granted on the source object**, and `rsAccessCheckRights` ends
`return NV_ERR_INSUFFICIENT_PERMISSIONS` (`rs_access_map.c:540`) when every grant path fails.
We grant nothing. ⇒ **the refusal is correct and the missing piece is an explicit share**:
`NV0000_CTRL_CMD_CLIENT_SHARE_OBJECT` (`0xd06`, `ctrl0000client.h:146-162`) applied **by the
per-proc isolate, on its own VA space, naming `RS_ACCESS_DUP_OBJECT`**.

### ★★★★★ AND THIS ANSWERS CONSTRAINT 30 FOR THIS RESOURCE, FROM HARDWARE, FAVOURABLY

Constraint 30's stated worry is that *"F11 records our isolates' kernel-visible euid is 0 on a
root VMM, so `rmclientIsAdmin(...)` plausibly holds for the scratchpad"* ⇒ effectively
privileged. **Measured false for this operation.** `rmclientIsAdmin` yields
`RS_PRIV_LEVEL_ADMIN`; the gate above demands `>= RS_PRIV_LEVEL_KERNEL`; **`ADMIN < KERNEL`**,
so RM treated our scratchpad as an ordinary user client and refused. ⊘ Our isolates are *not*
kernel-privileged to RM, and this is the first evidence of it rather than an assumption.

⚠ **It also SCOPES `[w744]`'s `NV_OK` on the same dup.** That probe ran both clients from one
process on a bare box; this one crosses two isolate **processes**. The ogkm gate names two
conditions and the campaign has now measured the answer on both sides of one of them. ⇒ *"a dup
that succeeds can still be the wrong thing"* has a companion: **a dup that succeeded once says
nothing about a dup between different parties.**

### ⊘⊘⊘ AND w745's AMENDMENT IS CONFIRMED — BY A ROW THIS FILE'S OWN GREP COULD NOT SEE

    ENGINE-OBJECT … → REFUSED host_chan=NONE Rm { err: Other(19266) }  [seen=10 forwarded=0 refused=10]
    VERBCOST total=49095us over 9 plan(s) [EngineObject n=8 … 94.0%] [Release n=1 …]

`19266 = 0x4B42 = MAP_THROUGH_A_BARE_SPACE`. **The restated guard fired ten times** — every
emulated channel birth *does* map its own ring through the bare space, which is precisely what
w745's amendment predicted and what its vacuous `BARE-SPACE-REFUSED=0` retired.

⊘ **And `W745-BARE-SPACE-REFUSED` printed `0` again, for a THIRD reason:** the harness grepped
`0x4b42` and the constant's name, while `RmError::Other`'s `Debug` prints **decimal**. A counter
blind to the thing it counts, inside the increment whose subject is that exact class. It was
caught only because the `ENGINE-OBJECT` tally is an **independent second row** for the same
event. ✔ Fixed in `w736_fbstore_run.sh`; every private status is now grepped in decimal first.

## ⇒ THE ROWS, GRADED

| # | predicted | measured | |
|---|---|---|---|
| 1 | arm 1 `(P)` 8/8 | **`(P)`, `THREADS 8 of 8`, `MEAN_FALSIFIER=PASS`** | ✔ |
| 2 | 17 / 11 / 19 / 0 | **17 / 11 / 19 / 0** | ✔ |
| 3 | `W746-CONTENT ≥1` | gate passed (the run reached the arms) | ✔ |
| 4 | `W746-ASSERTS-LINE=1` | **1** on every arm | ✔ |
| 5 | `W746-HANDOVER-MINTED ≥ 1` | **0** — ⊘ but the row is MIS-NAMED: it greps the success line that prints only after `adopt` succeeds. `HANDOVER-ASSERTS asked=4637` is the row that says the hand-over RAN | ◐ |
| 6 | `adopts ≥ 1` / `maps ≥ 1` | **`adopts=0 adopt_refused=4637`, `maps=0`** — ⊘ REFUTED, and the refusal is RM's, named | ⊘ |
| 7 | `BARE-SPACE-REFUSED ≥ 1` | **10**, read off `ENGINE-OBJECT … Other(19266)`; the named row said `0` because its grep was blind | ✔ (via a second row) |
| 8 | `ADOPTWHY-6 < 17` | **17** — the chain is still inert downstream | ⊘ |
| 9 | `RING-NOT-A-SLICE` read WITH `asserted` | `0` with `asserted=0` ⇒ **VACUOUS, and said so** | ✔ (as an instrument) |
| 10 | `ROUTE-DISAGREES=0` | **0** over 4637 asks | ✔ |
| 11 | `C30-REFUSED / C30-BIRTH-REFUSED = 0 / 0` | **0 / 0** | ✔ |
| 12 | `RING-HANDLE-REACHED-RM=0` | **0** | ✔ |
| 13 | arm 3 `(R)` 0/8 | **`(R)` 0/8** | ✔ (predicted) |
| 14 | `RmInitAdapter failed!=0`, `SMI_RC=0` | **0 / 0 on all three arms** | ✔ |
| 15 | bar1/bar2 `TRAP_FILLS=0`, `misses=0` | **0 / 0 on all three arms** | ✔ |
| 16 | failing-name set = baseline | **byte-identical**, `diff` empty both ways (31 names) | ✔ |
| 17 | `VCPU-BLOCKING` reported | **`total=197 doors=9`** on arms 2 and 3 — **identical to w742 and w745** | ⊘ no change |
| 18 | `withheld_unmaps` / `pending` | `withheld_unmaps=0 worst_unmaps_outstanding=0 pending=false` | ✔ |

★ `HOST_DMESG_XID`: **arena 1**, device 0, split 0. The one is the pre-existing
`Xid 31 … MMU Fault: ENGINE CE0 … FAULT_PDE @ 0xa0_00000000` (w555/w711), on the arm that
passes, and is neither caused nor fixed here.

## ⊘ WHAT THIS BOOT STILL CANNOT SAY

- **Nothing about leg B / the USERD.** Never reached: the dup is upstream of it.
- **Nothing about `map_store_slice` or constraint 28 on hardware.** `maps=0` — still unmeasured,
  for the second boot running, and now for a *different* reason than at w745.
- **Nothing about whether a scratchpad-placed MAPPING conveys the scratchpad's privilege.**
  Constraint 30 part 2's open question is untouched; the dup never landed.
