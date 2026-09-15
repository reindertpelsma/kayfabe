# w745 — the ownership split: PREDICTIONS, committed BEFORE the boot has produced a line

**STATUS: LIVE, 2026-09-15.** Written and committed **before any output exists**. The box
(`vast 51149807`, RTX 3060 / GA106, Quebec CA, host driver to be swapped to `580.159.04`) is
rented and provisioning; `scripts/bench/w736_fbstore_run.sh` has **not** been run at this
revision.

⊘ Predictions are not evidence. They exist so that a result cannot be read as whatever was
convenient afterwards — this campaign's `fix_the_criterion_before_the_boot` lesson. Each row
carries the value that **refutes** it.

## ⚠ THE HEADLINE PREDICTION, AND IT IS A NEGATIVE ONE

**I predict the raw client does NOT pass on the split arm.** The reason is named in
`constraint 26c`'s commit message and is not a hedge: **leg B is declined for a store slice.**
The guest's USERD handle is a real RM operand (`ChannelAllocParams::h_userd_memory_0`) and a
per-proc client has no handle for the scratchpad's object, so the channel is born with OUR
USERD while the guest advances `GP_PUT` in its own framebuffer page. That is the
`GP_PUT`-races-an-empty-cursor silence `AdoptedGuestUserd`'s own docs describe.

⇒ **What this boot is for is the FOUR FROZEN NUMBERS.** `[w740, w742, w743]` — three boots,
three revisions, one variable each — reported `(6)=17`, `BIRTH-AT-ALLOC REFUSED=11`,
`PassthroughDoorbellBirth=19` and `ADOPTING=0`, **byte-identical every time**. w743 drove its
own target (`CpuCeFb`) from 63 to 0 and not one of those four moved. If the ownership split is
the right diagnosis, they move here; if they do not, the diagnosis is wrong and that is the
deliverable.

## The rows

| # | line | predicted | ⊘ REFUTED BY |
|---|---|---|---|
| 1 | ★★★★★ ARM 1 `W392D_GUEST_OUTCOME=` | **`(P)`, `THREADS 8 of 8`, `MEAN_FALSIFIER=PASS`** | anything else ⇒ **the binary is broken and NO other row on any arm is interpretable.** Read this first |
| 2 | ARM 1 `STORE-MAP` | `⊘ NOT BUILT` | a census line ⇒ the split armed on the control, which is then not a control |
| 3 | ARM 2 the four frozen numbers | `(6)=17`, `BIRTH-AT-ALLOC REFUSED=11`, `DoorbellBirth=19`, `ADOPTING=0` | **any other value** ⇒ this binary changed the device arm by itself, and arm 3's movement cannot be attributed to the split |
| 4 | ARM 3 `STORE-MAP AT REALIZE` | `★★★★★ ARMED` | `⊘⊘ … NO PORT COULD BE BUILT` ⇒ read the `SCRATCHPAD` line; every row below is then about a boot with bare address spaces and nobody to map them, which is **worse** than the control and must not be read as the design failing |
| 5 | ★★★ ARM 3 `W745-HANDOVERS` | **≥ 1** | `0` ⇒ the publish path never reached the port. Read `W745-SM-FIRST` and the per-proc refusal line; `HANDOVER_OF_A_NON_BARE_SPACE` would mean the arm reached the VMM and **not the factory** |
| 6 | ★★★★★ ARM 3 `W745-MAPS` | **≥ 1** | `0` ⇒ nothing was mapped and the rest of arm 3 is a boot with no data plane at all. **THE MECHANISM ROW** |
| 7 | ARM 3 `W745-MAP-REFUSED` | **not predicted** — `PlacementRefused` here would be constraint 28 firing on real hardware, which is a *finding* in either direction | — |
| 8 | ★★★★★ ARM 3 `W745-ADOPTWHY-6` | **0** — conjunct (6) is what the slice binding exists to satisfy | **17** ⇒ the binding never reached the address table and the whole chain is inert. This is the number that has not moved in three boots |
| 9 | ★★★ ARM 3 `W745-BIRTH-ADOPTING` | **≥ 1** | `0` ⇒ no channel was ever adopted; read `W745-RING-NOT-A-SLICE` next |
| 10 | ★★★ ARM 3 `W745-RING-NOT-A-SLICE` | **0** | ≥1 ⇒ the oracle refused. ⚠ **That is the fail-closed path working, not a bug**: it means a ring was offered whose slice this port had not placed, and the boot must say which |
| 11 | ARM 3 `W745-FOREIGN-HANDLE` | **0** | ≥1 ⇒ `RingProvenance` did not remove the handle from the plan and option (ii) is not built the way this branch claims |
| 12 | ★★★★★ ARM 3 `W392D_GUEST_OUTCOME=` | ⊘ **`(R)`, `THREADS 0 of 8`** — see the headline. A `(P)` would be **better than predicted** and would mean leg B was not load-bearing here | `(P)` ⇒ stop and report immediately, per the brief |
| 13 | ARM 3 `RmInitAdapter failed!` / `SMI_RC=` | **0 occurrences / `0`** | ≥1 / non-zero ⇒ the split broke the driver's own bring-up, which is a regression and not a wall |
| 14 | ARM 3 bar1/bar2 `TRAP_FILLS=` / `misses=` | **0 / 0** | ≥1 either ⇒ regression against w743 |
| 15 | ARM 3 `W745-WITHHELD-UNMAPS` | **not predicted** — constraint 27's barrier fires or does not depending on pool timing. ⊘ Non-zero **beside** `pending=true` at teardown would be the hang it can cause and IS a defect | — |
| 16 | ★★ `W745-VCPU-BLOCKING` on arms 2 and 3 | **not predicted**, and deliberately: the brief asks for the number, not for a claim about it. w742 measured `total=197 doors=9 worst_trap=44440us` on the device arm | — ⚠ a **rise** on arm 3 is a cost of the split and must be reported as one |
| 17 | ARM 3 `W745-BARE-SPACE-REFUSED` | **not predicted** | ⚠ a large number would mean something in the per-proc isolate is still trying to map, i.e. the split is incomplete and the refusal is carrying the weight a design should |

## ⊘ What this boot CANNOT say, stated before it runs

- **Nothing about the USERD ruling.** Leg B is declined by construction on this arm, so the
  boot cannot distinguish *"the split is wrong"* from *"the split is right and the cursor is
  not carried"*. Only a boot with an arming path or a scratchpad-side birth could.
- **Nothing about throughput.** No workload here is a perf measurement.
- It is **one driver on one chip**. `580.159.04` on a GA106; w744's RM answers were taken on
  `580.159.03` and `580.126.20`.

---

## ⊘⊘⊘ AMENDMENT, written 2026-09-15 BEFORE the boot, from reading my own diff

**I expect row 13 (`RmInitAdapter failed! = 0`) to be the one that breaks, and I am recording
why here rather than discovering it in the log.**

On the `scratchpad` arm **every** per-proc isolate gets a bare address space — including the
**system proc**, whose channels are `Emulated`. An emulated channel is born over
`RingSource::Ours`: `alloc_channel_in` **allocates its own ring object and maps it** into the
guest VAS. That map is `map_gpu_va`, which now refuses `MAP_THROUGH_A_BARE_SPACE` by name.

⇒ **The split as built refuses a class of mapping that is not the one it was aimed at.** §26's
own flow — *"the scratchpad … maps slices of the one object into it"* — covers guest vidmem;
it does not cover an object the isolate allocated **for itself** to serve an emulated channel,
and `map_store_slice` cannot serve one (it maps the reservation, at an offset).

### What each outcome would mean

| measured | reading |
|---|---|
| `RmInitAdapter failed!` ≥ 1 **and** `W745-BARE-SPACE-REFUSED` ≥ 1 | ✔ this amendment, confirmed. The split needs a **second** answer for isolate-owned objects, and w744's Q4 already measured one candidate: P *can* build a **bounded subrange** in a space where S holds the whole-space range — **in that order**, which is an ordering constraint this build does not enforce |
| `RmInitAdapter failed!` ≥ 1 **and** `W745-BARE-SPACE-REFUSED` = 0 | ⊘ something else broke; this amendment is wrong and must not be used to explain it |
| `RmInitAdapter failed!` = 0 | ★ better than predicted — the emulated path did not need a map on this boot, and rows 5–10 are then the real subject |

⚠ **I am NOT changing the code to pre-empt this.** w744 measured the subrange ordering **once**,
on a bare box with no guest, and the working order there was *space → S's whole range → P's
subrange*; on the boot path the hand-over is lazy and may land after the first emulated birth.
Building an ordering on one measurement of a different sequence is exactly the improvisation
`B1`'s *"do not pick one by implementation convenience"* refuses. ⇒ **measure first.**

---

# ✔✔✔ MEASURED 2026-09-15 (w745), ON HARDWARE, THREE ARMS, ONE BINARY

`[vast **51149807**, RTX 3060 **GA106** (`10de:2504`), Quebec CA, host driver **580.159.04
OPEN**, `TREE_REV = 1edb741a` stamped on all three arms, control first; evidence
`w745_evidence.tgz` + `w745_run.log` beside this file]`
⊘ **No constraint was relaxed. No completion was forged.**

## ★★★ THE CLIENT'S VERDICT, FIRST

| arm | `KAYFABE_FB_STORE` / `KAYFABE_VAS_OWNER` | client |
|---|---|---|
| 1 control | `arena` / `isolate` | **`(P)` — `THREADS 8 of 8 verified ✔`, `MEAN_FALSIFIER=PASS`** |
| 2 w743 repro | `device` / `isolate` | `(R)` — `THREADS 0 of 8` |
| 3 **the test** | `device` / **`scratchpad`** | ⊘ **`(R)` — `THREADS 0 of 8`** |

⇒ **The raw client does not pass on the split arm.** That is what row 12 predicted, and the
predicted *reason* (leg B) is **not** the reason it failed — see below.

## The rows

| # | predicted | **measured** | verdict |
|---|---|---|---|
| 1 | ARM 1 `(P)` 8/8 | **`(P)`, 8 of 8, `MEAN_FALSIFIER=PASS`, `ARENA_BOOT_RC=0`** | ✔ **HELD** — the binary is sound, so every row below is interpretable |
| 2 | ARM 1 `STORE-MAP ⊘ NOT BUILT` | **`STORE-MAP AT REALIZE: ⊘ KAYFABE_VAS_OWNER=isolate`**, `DEVICE-LEAF-SPLIT handovers=0 … ⊘ THE SPLIT'S PUBLISH ARM NEVER RAN` | ✔ **HELD** — the gate's off position is provably ABSENT, not silent |
| 3 | ARM 2 the four frozen numbers | **`(6)=17`, `BIRTH REFUSED=11`, `DoorbellBirth=19`, `ADOPTING=0`** | ✔ **HELD, byte-identical to w740/w742/w743** ⇒ this binary does not change the device arm by itself |
| 4 | ARM 3 `STORE-MAP AT REALIZE ★★★★★ ARMED` | **`★★★★★ ARMED — KAYFABE_VAS_OWNER=scratchpad`** | ✔ **HELD** |
| 5 | ★★★ ARM 3 `HANDOVERS` ≥ 1 | **`handovers=0`, `handover_refused=4619`** | ⊘⊘⊘ **REFUTED** |
| 6 | ★★★★★ ARM 3 `MAPS` ≥ 1 | **`maps=0 map_refused=0 adopts=0 asserted=0 first_refusal=[none]`** | ⊘⊘⊘ **REFUTED — THE MECHANISM ROW** |
| 7 | `MAP-REFUSED` not predicted | **0** — no map was ever attempted, so constraint 28 was never exercised on hardware here | — |
| 8 | ★★★★★ ARM 3 `(6)` = **0** | **17** | ⊘⊘⊘ **REFUTED** — nothing was bound, so conjunct (6) could not move |
| 9 | ★★★ ARM 3 `ADOPTING` ≥ 1 | **0** | ⊘ **REFUTED**, for row 8's reason |
| 10 | ARM 3 `RING-NOT-A-SLICE` = 0 | **0** | ✔ held, and **VACUOUS**: no ring was ever offered as a slice, so the oracle was never asked (`asserted=0`) |
| 11 | ARM 3 `FOREIGN-HANDLE` = 0 | **0** | ✔ held — and also vacuous on this boot |
| 12 | ★★★★★ ARM 3 client `(R)` 0/8 | **`(R)`, `THREADS 0 of 8`** | ✔ **HELD — but for the WRONG REASON.** Read the next section |
| 13 | ARM 3 `RmInitAdapter failed!` 0 / `SMI_RC` 0 | **0 occurrences / `0`**, `HOST_DMESG_XID=0` | ✔ **HELD** |
| 14 | ARM 3 bar1/bar2 `TRAP_FILLS` / `misses` 0/0 | **`TRAP_FILLS=0 / 0`, `misses=0 / 0`**, `quiesce[calls=0 removed=0]` | ✔ **HELD** — no regression |
| 15 | `WITHHELD-UNMAPS` not predicted | **`withheld_unmaps=0 worst_unmaps_outstanding=0 pending=false`**, `unmaps_outstanding=0 drain_trips=0` on every refresh | ★ constraint 27 is LIVE and never had to fire; `pending=false` at teardown ⇒ no hang |
| 16 | ★★ `VCPU-BLOCKING` not predicted | arm 2 **`total=197 doors=9 worst_trap=63351us`** · arm 3 **`total=197 doors=9 worst_trap=79686us`** | ⊘ **the split neither reduced nor multiplied the doors** — identical to w742's `197/9`. The brief's "second win" is **not** available |
| 17 | `BARE-SPACE-REFUSED` not predicted | **0** | ★ see the amendment's verdict below |

## ⊘⊘⊘ THE AMENDMENT IS REFUTED, AND THAT IS THE CHEAPEST FINDING HERE

The amendment predicted `RmInitAdapter failed! ≥ 1` **and** `BARE-SPACE-REFUSED ≥ 1`, because the
system proc's `Emulated` channels map their own ring through `map_gpu_va`. Measured:
**`MAP_THROUGH_A_BARE_SPACE` fired ZERO times and `RmInitAdapter failed!` is 0.** The emulated
path did not need a map into a bare space on this boot. ⇒ **the third row of the amendment's own
table** — *"better than predicted"*. ⚠ It is a measured zero on one boot, not a proof that the
path cannot be reached.

## ★★★★★ WHY IT ACTUALLY FAILED — ONE REFUSAL, ONE CAUSE, 4 619 TIMES

    VAS-PUBLISH(proc=2 pdb=0x0) leaf va=0x8000000000 pdb=Pdb(0)
      → ⊘⊘ CONSTRAINT 26: the per-proc isolate would not hand its address space over:
        NoVas(ChanId(0))

Every refusal is the same one and every leaf came under **`pdb=Pdb(0)`**.

`SharedDevice::vaspace_handover` routes with `kayfabe_fwd::route_pdb(spine, gpu, pdb)`. The
publish route offers framebuffer leaves under a **placeholder pdb** — `Vas::pdb` is
`Option<Pdb>` and is `None` until a `SetPageDir` declaration arrives, and the publish context
prints `pdb=0x0` — so the routing finds no `Vas` and the hand-over cannot even be attempted.

⇒ **The defect is that the hand-over RE-DERIVED a routing key its caller already held.**
`join_one_fb_leaf` is handed the `IsolateId` and the pdb by a caller that already routed to the
right proc; keying the hand-over on `(gpu, pdb)` made it a *second* statement of a routing
decision, and the caller's key can be a placeholder that the second statement cannot resolve.
That is this tree's `a_second_source_of_truth_beside_a_complete_value`, in a new place.

⊘ **It is not a hardware finding and not a design refutation.** Nothing in the RM layer was
reached: `adopts=0` means `NV_ESC_RM_DUP_OBJECT` was never issued on this boot, so w744's
hardware answers were neither confirmed nor challenged here.

## ⚠ A DIAGNOSTIC DEFECT IN MY OWN REFUSAL LINE, found by reading it

The line says *"`HANDOVER_OF_A_NON_BARE_SPACE` here means this isolate was spawned WITHOUT
`--bare-vaspaces on`"* — and the error it actually printed is `NoVas(ChanId(0))`. The sentence
names a cause the value contradicts. ⇒ **a refusal that suggests a diagnosis its own payload
rules out is worse than one that says nothing**, and it is the shape that sends the next reader
to the factory wiring when the fault is in routing.

## ⊘ What this boot did NOT measure, stated plainly

- **Nothing about `adopt_vaspace`, `map_store_slice`, the slice binding, the ring oracle or
  constraint 28 on hardware.** All are downstream of the hand-over and none was reached.
  `asserted=0` says the oracle was never asked; `first_refusal=[none]` says the port refused
  nothing — it was never called.
- **Nothing about leg B / the USERD ruling.** Row 12's predicted *reason* is untested: the boot
  never got far enough to birth a channel over a store slice.
- ★ What it DID measure, and could not have been measured any other way: the split **arms**, the
  publish route **reaches** it, the vCPU guard **declines by name 14 times without panicking**,
  and the control arm survives the whole change at `(P)` 8/8.
