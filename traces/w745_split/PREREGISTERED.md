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
