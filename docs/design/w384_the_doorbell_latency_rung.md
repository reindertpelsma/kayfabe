# ★★★★★ WHAT ONE DOORBELL COSTS THE SUBMITTING THREAD — a five-second gate for `w383-doorbell-async`

**STATUS — 2026-09-06 — LIVE.** Adds one rung to the raw client (`--doorbell-latency`) and its
differential harness.

> **THE THREE RESULTS, UP FRONT.**
> 1. ★★★★★ **A guest channel dies at the submission that writes `GP_PUT = 0`**, and the wedge
>    **tracks the ring's own modulus** — 64 entries ⇒ dies at the 64th submission, 32 entries ⇒
>    dies at the 32nd, while the same binary walks six laps of either on bare metal. Localised
>    to one submission, by a pre-registered experiment whose alternative reading it falsified,
>    in a 30-second run from an unprivileged raw client with no CUDA. §4.2.
> 2. ★★★★★ **`w383-doorbell-async` changed nothing measurable on this path** — same box, two
>    shims, 1.03× on the graded arm. §4.4.
> 3. ⊘⊘⊘ **This document's own gate argument is refuted by its own control**: the native-relative
>    ratio is **8.7× apart** on two GA106 boxes, so the gate is not portable. §4.4.1.
>
> ⚠ The *latency* the rung was commissioned to catch **would have passed the gate**. Only the
> closing positive control turned a green into a `NOTRUN`, and finding (1) is downstream of that.

It does not supersede `w381_the_guest_servable_probe.md`; it is built on that lane's `LAUNCH_DMA`
primitive and inherits its scope caveats verbatim. Mechanism sections §1–§3 are read off this
tree's own source and are checkable without a GPU. §4 carries the measurements; every number in
it names the arm it came from.

---

> ### ⊘⊘⊘ THE GROUND MOVED WHILE THIS LANE WAS BUILDING — read this before §0
> ### 2026-09-06, and it changes what a guest PASS means, not what the rung does.
>
> This lane was briefed against master **`30eb4627`**, with `w383-doorbell-async` described as
> **live and in flight**. It is not: master is now **`758a5752`**, *"Merge branch
> 'w383-doorbell-async'"*, and that lane's own commits say
> **`w383 §13: the gate armed — 95.7 percent skipped, 3.9x less publication wall, CUP3_VAL=43
> held`** and **`w383 §14: ★★★★★ LLM_TOKENS=16 — the LLM generates`**.
>
> ⇒ **The 60–71 ms this rung was built to catch has already been worked on, and the branch that
> did it has landed.** So:
> - ⊘ **A guest PASS is no longer automatically the finding §3 pre-registers it as.** On
>   `30eb4627` a pass would have meant *"the cost is not on the raw client's doorbell path"*. On
>   `758a5752` it may simply mean **the fix works**. The two readings are not distinguishable
>   from one arm, and this doc must not let one be read as the other.
> - ★★★ **Hence the deliberate two-arm design that replaced the single guest arm**: the same
>   rung, same binary shape, on **`30eb4627` + this rung** (`w384-prefix-device`, pushed) and
>   on **`758a5752` + this rung** (`w384-doorbell-latency`, rebased onto current master). A
>   controlled before/after is the only thing that can say whether the rung DISCRIMINATES, and
>   it is strictly stronger evidence than the red it was commissioned to produce.
> - ⚠ **Nothing about the rung, the gate or the controls was changed in response.** The
>   multiple stays at 1000×, argued from the two orders of magnitude in §3 and not fitted to
>   whatever the guest turns out to cost. Tuning a threshold after seeing the arm it grades is
>   how a gate stops being one.
>
> ⚠ Same class as `a_blocker_i_declared_was_already_fixed` and `a_rulings_date_is_part_of_the_
> citation`: **the brief's `master` SHA was three hours stale, and every consequence of that was
> in how the RESULT reads, not in whether the work was worth doing.**

## §0 THE ONE-LINE PROBLEM

`[measured, LLM boot, w383 lane]` publication runs **60–71 ms per doorbell, inline on the vCPU
thread**:

```
TRAPWITNESS off_trap_claims=0 inline_exceptions=61865 worst_trap=1750538us
            (target: inline_exceptions=0)
```

Thousands of doorbells at that price is why the LLM rung is killed by a harness timeout with
doorbells still being served — and **nothing in this tree measured it in under twenty minutes.**
`LLM_TOKENS`, the async lane's only feedback, needs a full boot plus a model load and can come
back `UNMEASURED` for reasons that have nothing to do with the change under test. A lane whose
only instrument costs twenty minutes and can answer *"no result"* is a lane iterating blind.

## §1 WHY A NEW RUNG AND NOT ONE OF THE SEVEN

Owner, 2026-09-06: *"the most valuable raw client is one that passes on host and fails in guest,
then its just iterate unless there is a blocker."*

All seven w379/w381 rungs pass natively; six of seven pass in the guest (`w381` §4.1). By that
criterion they discriminate almost nothing. `missing_page` is the one with the valuable shape,
and it is about the **error notifier** — not about anything on the critical path the async lane
is moving. So the shape had to be built, not found.

## §2 THE MEASURED REGION, STATED EXACTLY

For the graded arm the region is **one `HostRmBackend::submit_copy_at` call and nothing else**.

| inside the region | outside it |
|---|---|
| handle narrow; one `HashMap` lookup for the channel's parts | the store that seeds the source word |
| the pushbuffer encode (**allocates one small `Vec`**) | the every-16 drain |
| ~12 stores into the ring object; 2 GPFIFO words; the `GP_PUT` store | every `println!` |
| two release fences and **the doorbell store** | all statistics |

★ **The `Vec` and the lookup are inside the region on BOTH arms, so they cancel in the ratio the
gate is taken on.** That is the reason the gate is a *multiple of a measured native median* and
not an absolute number: any cost that is arm-independent divides out.

### ★★★ THE INSTRUMENT THAT WOULD HAVE BEEN THE MEASUREMENT

`RmConnection::doorbell` prints **two `eprintln!` lines per store for its first 512 stores**
(`DOORBELL_WITNESS_MAX`), and those lines are *inside* the region. On bare metal a doorbell is
microseconds, so a formatted write to stderr is not a rounding error on it — it is a large
fraction of the number. ⇒ every gate multiplied out of that floor would have been **uniformly,
quietly too loose**.

`kayfabe_isolate_host::rm::mute_doorbell_witness()` **exhausts** the witness (advances its
counter past the cap) rather than disabling it: the periodic tally still prints, and **every
refusal still prints in full**, by `doorbell`'s own argument that suppressing the rare event to
make room for the common one inverts the witness's purpose. The rung **prints that it muted**,
because a quiet log a reader takes for a complete one is a failure class this tree has paid for.

## §3 THE GATE — a multiple of a measurement, and the multiple is argued

⊘ A hard-coded millisecond figure would be a standard nobody agreed to, on a box nobody
characterised, and it would expire as the box changed with no one noticing — the shape
`a_capture_derived_table_expires_as_a_vendor_regression` records. So:

```
gate = native_p50 × DBL_GATE_MULTIPLE          DBL_GATE_MULTIPLE = 1000
```

`native_p50` is measured **minutes earlier, on the same box, by the same binary** (md5 printed on
both arms) and is fed to the guest arm by the harness as `--doorbell-latency-native-us`. Nothing
is baked into the binary.

**1000× is the only decade that separates the two things the rung must tell apart**, and both
sides of the band are measured rather than assumed:

| | what it is | order |
|---|---|---|
| the **floor a correct emulation cannot go below** | a doorbell in a Mode-2 guest is a trapped MMIO store: one VM exit, one VMM dispatch, one return — a cost an async design still pays | **10–100×** |
| the **defect** | 60–71 ms inline on the vCPU thread | **~10⁴×** |

⇒ 1000× sits a decade **above** the most expensive honest emulation cost and a decade **below**
the defect. A gate outside that band either reddens a working async design or greens the thing it
exists to catch. ⚠ It is a **discrimination threshold, not a performance target**: passing it is
not a claim that anything is fast.

### ★★ AND THE NATIVE ARM HAS ITS OWN BAR, because a calibration cannot fail a gate derived from itself

`DBL_NATIVE_SANITY_CEILING_US = 1000`. Nothing in `release_fence()` plus a store into a mapped
write-combining window legitimately medians above a millisecond; if it does, the box is
contended or the path is not what we think, and **the number must not be used as anyone's
floor**. A native run above it is a red, printed as one. The native verdict says in words that
it is *not* "native met a gate".

### ★★★★★ THE SAMPLE FLOOR HAS AN ESCAPE HATCH, and without it the rung is backwards on exactly the case it exists for

The wall budget is what makes this a five-second gate, and its consequence is that **the worse
the arm is, the fewer samples it produces**. At `worst_trap=1750538us` a 1.5 s repetition yields
**one** sample. A flat rule of *"under 50 samples ⇒ UNMEASURED"* would report the most
catastrophic possible result as *"we could not tell"* — the one arm the rung could never grade
would be the broken one.

The hatch is narrow and **sound rather than lenient**: it does not lower the bar, it uses a
statistic that needs no sample size. If the **fastest** submission observed is already over the
gate, no median over any number of further samples can be under it — every sample is at least the
minimum, by definition. The rung prints `DBL_ROLE=GRADED_ON_MIN` and says both facts.

## §3.1 THE CONTROLS — the rung is these, not the timing loop

- **Two positive controls, one before the loop and one after.** Each is a four-byte `LAUNCH_DMA`
  into the rung's own mapped object, read back through an **independent** CPU mapping. ★ The
  closing one is what makes the loop attributable: the opening control alone would be passed by a
  loop that killed its own channel on submission 3 and then timed 500 cheap refusals. Either
  control failing ⇒ `RUNG_doorbell_latency=NOTRUN`, ⊘ **not** a red.
- **Refusals are excluded from the samples and counted separately** (`refused=` on every
  distribution line). A refusal is not a fast submission.
- **`truncated=`** is printed on every distribution. A truncated run is still a valid
  distribution; a run that does not *say* it was truncated is not.
- **Five order statistics and a total, never a mean.** The failure being gated is a tail that
  dominates an aggregate, and a mean is exactly the summary that hides it.
- ⊘ **`GP_GET` is not read anywhere in this rung.** It has no writer in the workspace and an
  emulated device has no PBDMA, so `gp_get == 0` is the only value it can hold on every
  configuration. The ring itself is measured instead.

### §3.2 TWO UNGRADED ARMS, and they are what make a green readable

- **`arm=bare`** — `ring_doorbell_only`: one fence and one store, no ring stores, no `GP_PUT`
  advance, no new entry. ⊘ **Ungraded**: a device is entitled to notice `GP_PUT` has not moved
  and do nothing, and *"a no-op is fast"* is a finding about the no-op. ★ What it is for is
  **attribution** — if a real submission is expensive and this is cheap, the cost is in the ring
  stores or the planning; if this is expensive too, the cost is in the trap.
- **`arm=freshmap`** — a **fresh** 64 KiB object at a **fresh** VA before every ring, so there is
  always something unpublished behind it. ★★★ This exists so that a **green on the graded arm is
  interpretable**: if publication is incremental, a loop that rings the same channel at the same
  address N times may pay once and be cheap thereafter, and would pass while a real workload —
  which maps as it goes — does not.

## §3.3 WHAT THIS DOES NOT MEASURE — scope, stated up front

- ⊘ **The ladder builds its OWN `FERMI_VASPACE_A` and its own CE channel inside the guest.** The
  claim it supports is *"a doorbell rung by an unprivileged guest process costs N"* — the
  quantity the async lane is moving. It is **not** *"the LLM's per-doorbell cost is N"*, and a
  reader who takes it for that has this campaign's recorded failure shape: a probe in the wrong
  address space.
- ⊘ **The guest arm runs `KAYFABE_CE_EXECUTOR=local`**, so it measures the CPU copy-engine
  emulator's doorbell path. `=host` is a different measurement and needs its own row.
- ⊘ **Within-process repetitions cannot see a per-boot lottery.** `submit_ms` has been measured
  at **9.1× across three consecutive boots of one build**. The rung repeats three times inside
  one process (within-run stability) and the harness runs the whole binary three times on each
  arm (between-process). Neither substitutes for the other, and **neither sees a per-boot
  effect**, because all three guest processes share a boot.

## §4 THE MEASUREMENTS

### §4.1 NATIVE — the calibration, RTX 3060 (GA106), host driver 580.159.04 open

Bench `kb`, source `5d0d6695`, binary md5 `06477aa0ae12145529af970d50916216`, **three separate
processes**, verbatim:

```
--- run 1 ---
DBL_DIST arm=submit   rep=POOLED n=1536 min_us=8.925 p50_us=8.966 p90_us=9.037 max_us=28.473 total_ms=40.1 truncated=false refused=0
DBL_DIST arm=bare     rep=0 ⊘UNGRADED n=512 min_us=0.040 p50_us=0.040 p90_us=0.050 max_us=3.326  total_ms=0.3
DBL_DIST arm=freshmap rep=0 ⊘UNGRADED n=64  min_us=9.137 p50_us=9.268 p90_us=9.427 max_us=18.233 total_ms=22.9
--- run 2 ---
DBL_DIST arm=submit   rep=POOLED n=1536 min_us=9.137 p50_us=9.236 p90_us=9.278 max_us=30.517 total_ms=40.4 truncated=false refused=0
DBL_DIST arm=bare     rep=0 ⊘UNGRADED n=512 min_us=0.040 p50_us=0.040 p90_us=0.040 max_us=3.176
DBL_DIST arm=freshmap rep=0 ⊘UNGRADED n=64  min_us=9.297 p50_us=9.498 p90_us=9.678 max_us=25.537
--- run 3 ---
DBL_DIST arm=submit   rep=POOLED n=1536 min_us=8.426 p50_us=8.514 p90_us=8.556 max_us=21.430 total_ms=39.1 truncated=false refused=0
DBL_DIST arm=bare     rep=0 ⊘UNGRADED n=512 min_us=0.039 p50_us=0.040 p90_us=0.040 max_us=3.357
DBL_DIST arm=freshmap rep=0 ⊘UNGRADED n=64  min_us=8.565 p50_us=8.826 p90_us=8.976 max_us=23.804
```

`DBL_CALIBRATION_NATIVE_P50_US = 8.97 / 9.24 / 8.51`. ⇒ **floor `9.24 us`** (the harness takes
the **worst** of the three, never the best: a floor picked from the fastest run makes the gate
tighter than the host can reliably deliver, and the first thing that reddens is host jitter
wearing the guest's name), so **gate = 9.24 ms**.

★ **The instrument is not noisy.** Over 1536 samples in one process the spread from `min` to
`p90` is **1.2 %**; across three processes the medians span **1.09×**. That tightness is what
makes a three-decade gate meaningful — the number this rung reports is not a lottery.

### §4.1.0 ★★★ TWO GA106 BOXES DISAGREE BY 3×, WHICH IS WHY THE GATE IS A MULTIPLE

The same binary's native calibration, on two rented RTX 3060 (GA106) hosts running the same
driver, measured within an hour of each other:

| box | native `arm=submit` p50 |
|---|---|
| `kb` (50013922, 23 cores) | **8.75 – 9.31 µs** |
| `kb2` (50080571, 19 cores) | **2.62 – 3.66 µs** |

⇒ **A factor of three, between two boxes of the same GPU and the same driver.** An absolute
millisecond threshold picked on either one would have been 3× wrong on the other — silently, and
in whichever direction nobody checked. ★ It is why the harness measures the floor **minutes
before** the guest arm rather than reading a constant.

> ⊘⊘⊘ **AND DO NOT READ THE NEXT SENTENCE OFF THIS TABLE, WHICH IS WHAT THIS DOCUMENT ORIGINALLY
> DID.** *"The floor is a property of the host, so the only portable statement is a ratio"* is the
> conclusion §3 was built on, and **§4.4.1 measures it FALSE**: the guest arm and the native arm
> move in **opposite** directions between these two boxes, so the ratio is 8.7× apart on them.
> A native-only comparison cannot see that — it takes the guest arm on both boxes, which is why
> the refutation arrives four sections later and not here.

### §4.1.1 ★★★ THE NATIVE ARMS DISAGREE BY 225×, AND THAT IS THE MOST USEFUL NUMBER HERE

| arm | native p50 | what it contains |
|---|---|---|
| `bare` | **0.040 µs** | one `release_fence` + one 32-bit store into the mapped usermode window |
| `submit` | **8.97 µs** | the above **plus** ~12 ring stores, 2 GPFIFO words, the `GP_PUT` store |
| `freshmap` | **9.27 µs** | `submit`, with a fresh 64 KiB object mapped at a fresh VA first |

⇒ **On bare metal the doorbell store is 40 ns and is 0.4 % of a submission.** The other 99.6 % is
stores into device memory. ⚠ **This is load-bearing for reading the guest arm** and it is not
what one would guess from the name of the rung: the graded number is a **submission** number, not
a **doorbell** number, and the two arms are what separate them. If the guest's `bare` is large,
the cost is **the trap**; if `bare` is small and `submit` is large, it is in the ring stores or
in what the VMM does behind them.

⇒ And **`freshmap` ≈ `submit` natively (9.27 vs 8.97 µs)**: mapping a fresh page before every
ring costs a submission 0.3 µs on hardware. That is the control that makes any guest-side gap
between those two arms attributable to **publication** rather than to the arm's own extra work.

### §4.1.2 ★★★ THE VERDICT VOCABULARY, EXERCISED IN ALL FOUR STATES — with no guest at all

⊘ *"A refusal needs a negative control."* A gate whose red is unreachable is not a gate, and the
only way to know is to make it fire. Every state below was produced on bare metal by feeding the
rung a floor or a sample count, on the committed binary:

| how | `DBL_ROLE` | verdict |
|---|---|---|
| default | `CALIBRATION` | `PASS` (and the line says in words that this is *not* "native met a gate") |
| `--doorbell-latency-native-us 9.27` | `GRADED`, `DBL_RATIO_X=0.9` | `PASS` |
| `--doorbell-latency-native-us 0.001` | `GRADED`, `DBL_RATIO_X=8446.0` | **`FAIL`** — the red is reachable |
| `-n 5 --doorbell-latency-native-us 0.001` | `GRADED_ON_MIN` | **`FAIL`**, determinate on 5 samples |
| `-n 5` (no floor) | — | **`NOTRUN`** |
| `-n 100000 --budget-ms 100` | — | `truncated=true n=3792`, the budget binds |

#### ⊘⊘ AND THE FOURTH ROW WAS A REAL DEFECT THAT THIS EXERCISE CAUGHT

The `-n 5` case printed, in prose, *"UNMEASURED, and NOT a failure value"* — and then emitted
**`RUNG_doorbell_latency=FAIL`**. The closure returned `false`, the positive control had
**passed**, so `false` fell through to the failure arm. ⇒ **the anchored machine-readable line
said the opposite of the sentence directly above it, and a grader reads the anchored line.**

★ `control_ok` alone could not express it: the control *did* pass and there was still nothing to
grade. The fix is a third state (`unmeasured`) and three outcomes that are **not ordered by
severity**. ⚠ This is the same class as *"w377 printed prose while its grader looked for a name
nothing emitted"*, inverted: here the name was emitted and **disagreed with the prose**. It was
found only because the negative control was run — the rung's own numbers on its own arm would
never have reached that branch.

### §4.2 ★★★★★ GUEST — THE GATE WOULD HAVE SAID PASS, AND THE CONTROL SAID THE CHANNEL IS DEAD

Same box, same boot's binary, `md5 6d2005aab792b9ad9ba4f9df19329e00` on both arms. Mode-2 guest,
`KAYFABE_CE_EXECUTOR=local`, `KAYFABE_ISOLATES=real`, `KAYFABE_VAS_PUBLISH=drain`,
`KAYFABE_GR_ROUTE=passthrough`, shim built at `8d5863fe` (= current master + this rung). **Three
processes, and all three are identical to the submission index:**

```
info  R6 control (open)  = Landed
DBL_DIST  arm=submit rep=0 n=64 min_us=616.887 p50_us=668.563 p90_us=697.328 max_us=736.922
DBL_DRAIN rep=0 every=16 drains=4 timeouts=1 drain_ms=2000.2 stalled=true  first_stall_at=63
DBL_DIST  arm=submit rep=1 n=16 p50_us=1111.995
DBL_DRAIN rep=1 every=16 drains=1 timeouts=1 drain_ms=2000.2 stalled=true  first_stall_at=15
DBL_DIST  arm=submit rep=2 n=16 p50_us=1113.137
DBL_DRAIN rep=2 every=16 drains=1 timeouts=1 drain_ms=2000.2 stalled=true  first_stall_at=15
DBL_STALL reps=3 stalled_reps=3 drains=6 timeouts=3 first_stall_at=63
info  R6 control (close) = Lost { saw: 3735880580 }        ⊘ 0xDEAD0384 — the SENTINEL
RUNGCTL_doorbell_latency=FAIL
RUNG_doorbell_latency=NOTRUN
```

`saw: 3735880580` is `0xDEAD0384`, [`DBL_SENTINEL`] — **nothing wrote there at all.**

#### ★★★ THE HEADLINE IS NOT THE LATENCY. THE LATENCY WOULD HAVE PASSED.

| quantity | native | guest | ratio |
|---|---|---|---|
| `arm=submit` p50, **first 48 submissions** | 9.02 µs | **448 µs** | **50×** |
| `arm=bare` p50 (the doorbell store alone) | 0.040 µs | **101–241 µs** | **≈2500–6000×** |
| gate (`native_p50 × 1000`) | — | 9 310 µs | — |

⇒ **The guest submission is 50–73× the native floor, which is 130× UNDER the gate.** Had the
rung graded on its distribution it would have printed `PASS`, and that would have been a true
statement about a channel that stops working after 64 submissions.

★★★ **Only the CLOSING CONTROL caught it**, and it is the arm this rung was told to build:
*"a positive control that must pass, or the result is UNINTERPRETABLE and prints as such."* The
opening control passed on all three runs; the loop produced 96 clean samples with **zero
refusals**; every printed latency number was real. **The verdict is `NOTRUN` — correctly — and
the reason is a line of its own.**

⚠ And note the shape of the near-miss: **the submissions that never execute are the CHEAP
ones.** A rung that kept sampling past the stall would have pulled its own median DOWN. That is
why the rep stops at the first drain timeout and why no gate is applied when the control fails.

#### ★★★★★ IT DIES AT SUBMISSION 63, AND THE GPFIFO HAS 64 ENTRIES

`first_stall_at=63`, **three runs out of three, to the index**. The drain windows end at
submissions 15, 31, 47 and 63; the first three retire and the fourth never does.
`PUSHBUFFER_SLOTS = 32` and the GPFIFO `entries = 64` (`kayfabe-isolate-host/src/rm.rs`), and
`submit_entry` sets `GP_PUT = (slot.gp + 1) % entries` — so **submission index 63 is the first
one whose `GP_PUT` is `0`.**

The bracket, run in the same boot and needing no second guest:

```
--- n=48 (strictly inside one lap of the GPFIFO) ---
DBL_DIST  arm=submit n=48 min_us=408.331 p50_us=448.493 p90_us=482.482 max_us=571.213
DBL_DRAIN drains=3 timeouts=0 drain_ms=0.1 stalled=false first_stall_at=none
info  R6 control (open)  = Landed
info  R6 control (close) = Landed          ★ BOTH CONTROLS PASS
```

⇒ **`drain_ms=0.1` for three windows inside one lap; `drain_ms=2000.2` for the one window that
crosses it.** It is a **cliff, not a slope** — the channel retires instantly right up to the
boundary and then never again.

⊘ **THAT IS A BRACKET, NOT A MECHANISM.** What is measured is that the failure tracks the lap
boundary exactly and reproducibly; *why* `GP_PUT = 0` is not fetched is a device-side question,
and every file that could answer it (`kayfabe-rt/`, `kayfabe-core/`, `kayfabe-qemu-raw/`) is
outside this lane. ⚠ Naming the cause without opening those files would be the
`shape_cannot_discriminate_origin` mistake.

★ **And the native arm is the control that makes it attributable**: the **same binary** completes
**512 submissions — eight full laps — with `drains=32 timeouts=0 drain_ms=8.4` and both controls
landing.** So the wrap arithmetic in this crate is not the defect; w381's `RingSlot` fix holds.
⇒ *"the probe is wrong"* is ruled out by the arm that exists to rule it out.

#### ★★★★★ AND THE BRACKET CLOSES TO ONE SUBMISSION — `n=63` vs `n=64`, same boot

```
--- n=63 ---  (loop indices 0..62)
DBL_DIST  arm=submit n=63 min_us=467.546 p50_us=511.199 p90_us=547.607 max_us=620.012
DBL_DRAIN drains=3 timeouts=0 drain_ms=0.1 stalled=false first_stall_at=none   ★ NO STALL
info  R6 control (open)  = Landed
info  R6 control (close) = Lost { saw: 3735880580 }                            ⊘ AND YET LOST

--- n=64 ---  (loop indices 0..63)
DBL_DRAIN drains=4 timeouts=1 drain_ms=2000.4 stalled=true first_stall_at=63
info  R6 control (close) = Lost { saw: 3735880580 }
```

★ **`n=63` is the decisive row and it took a moment to read.** Its three drain windows (ending at
loop indices 15, 31, 47) all retire in **0.1 ms total** — the channel is healthy throughout — and
then the *closing control*, which is simply the next submission, is **Lost**. Count the
submissions on the channel: the **opening control** is submission #1 and takes `slot.gp = 0`, so
loop index `i` takes `slot.gp = i + 1`, and `submit_entry` writes
`GP_PUT = (slot.gp + 1) % entries` with `entries = 64`:

| submission | `slot.gp` | `GP_PUT` written | outcome |
|---|---|---|---|
| opening control | 0 | 1 | Landed |
| loop `i` = 0 … 61 | 1 … 62 | 2 … 63 | all retire |
| **loop `i` = 62** | **63** | **0** | ⊘ **the wrap** |
| loop `i` = 63 | 0 | 1 | never retires |
| closing control | 1 | 2 | **Lost** |

⇒ On `n=48` the highest `GP_PUT` ever written is **50** and everything lands. On `n=63` the loop
reaches the wrap at `i = 62`, no drain window falls after it, and the very next submission is
lost. On `n=64` the window that spans the wrap is the one that times out.

★★★★★ **The channel works for exactly as long as `GP_PUT` never takes the value `0`, and dies at
the submission that sets it to `0`.** That is a one-submission localisation from an unprivileged
raw client, in a 30-second run, with no libcuda and no device-side instrumentation.

#### ★★★ SEPARATING THE TWO READINGS — `KAYFABE_LADDER_GPFIFO_ENTRIES`, PRE-REGISTERED

Two readings fit everything above and **coincide exactly on a 64-entry ring**:

- **(A) the WRAP** — *"a `GP_PUT` of `0` is not consumed"*, and
- **(B) an ABSOLUTE index or offset** — *"entry 63 / the last 8 bytes of the region is not
  consumed"*.

They separate the moment the ring has a different number of entries, so the knob was built
(`ladder_gpfifo_entries`, defaulting to the constant and printing on stderr whenever it does not).
**Written down before the run:**

| | (A) the wrap | (B) an absolute index |
|---|---|---|
| `entries=32`, `n=24` (`GP_PUT` never 0) | clean, both controls land | clean |
| `entries=32`, `n=32` | ⊘ **stall, window at loop index 31** (loop `i=30` writes `GP_PUT=0`) | ★ **no stall — index 63 is never reached on a 32-entry ring** |
| `entries=32`, `n=48` | ⊘ stall at loop index 31 | ★ no stall |

★ **The knob's own native control, run first**: `entries=32`, `n=200` — six full laps —
`drains=12 timeouts=0 drain_ms=3.2 stalled=false`, both controls land, p50 `8.746 µs`; the
`entries=64` arm on the same box is `9.337 µs` with the same clean drain. ⇒ RM accepts the
32-entry ring and this crate submits through it correctly, **so a guest-side stall at 32 is
attributable to the device and not to the knob.**

#### ★★★★★ THE ANSWER: (A). THE WEDGE TRACKS THE RING'S OWN MODULUS.

Mode-2 guest, `kb2`, `entries=32`, three invocations in one boot, verbatim:

```
--- n=24, entries=32 ---   (GP_PUT takes 2..25 — never 0)
DBL_DRAIN drains=1 timeouts=0 drain_ms=0.1 stalled=false first_stall_at=none
info  R6 control (open)  = Landed
info  R6 control (close) = Landed                         ★ CLEAN

--- n=32, entries=32 ---   (loop i=30 writes GP_PUT=0)
DBL_DRAIN drains=2 timeouts=1 drain_ms=2000.1 stalled=true first_stall_at=31
info  R6 control (close) = Lost { saw: 3735880580 }       ⊘ STALLED

--- n=48, entries=32 ---
DBL_DRAIN drains=2 timeouts=1 drain_ms=2000.3 stalled=true first_stall_at=31
info  R6 control (close) = Lost { saw: 3735880580 }
```

⇒ **Entry index 63 is never touched on a 32-entry ring, and the channel dies anyway — at index
31.** Reading **(B)** is **refuted**; reading **(A)** predicted `first_stall_at=31` and got it.

★★★★★ **THE MECHANISM, NAMED: a `GP_PUT` of `0` is not consumed by the emulated device, and the
channel never recovers from it.** The wedge is at the ring's own modulus, wherever that modulus
is put — 64 entries ⇒ dies at submission 64, 32 entries ⇒ dies at submission 32 — and the same
binary walks six laps of either ring on bare metal without a single stalled drain.

⊘ **What is still NOT claimed.** *Where* in the device the value is dropped — the codec, the
cursor comparison, the resume point — is a question for the files this lane does not own
(`kayfabe-rt/`, `kayfabe-core/`, `kayfabe-qemu-raw/`). What is established is the **input** that
triggers it, to one submission, with the alternative reading falsified by its own pre-registered
prediction. ★ A device-side lane can now write a failing test from this paragraph alone.

#### ⊘⊘ HOW THIS WAS VERY NEARLY MISSED — a diagnostic gated on the failure

The **first** guest run of this rung returned early on the failed closing control, so
`arm=bare` and `arm=freshmap` **never ran on the one arm that had something to explain** — and
the drain was an uninstrumented `sleep`-until-2s, so a stalled channel presented as *"the budget
truncated the rep"* rather than as a stall. Both were fixed before the run quoted above, and the
fix is what produced `first_stall_at`. ⚠ Same class as `a_diagnostic_gated_on_the_failure`, in a
file written by someone who had read that lesson the same day.

#### ⊘ A SIXTH DEVICE-OPEN PRINTED NOTHING, AND IT READ AS A SILENT RUNG

The second boot's hook ran **six** GPU-touching processes. The fifth (`n=200`) printed **nothing
at all** and the sixth (`missing_page`) printed only its return code, while the first four were
clean. That is `the_harness_stopped_where_the_bug_starts` — *"the 5th DEVICE-OPEN wedges the
GPU"* — reproducing from a raw client with no CUDA anywhere. ⇒ `w384_hook.sh` now computes and
**prints** its device-open count and warns above four. ⚠ The failure mode is empty output, which
reads as *"the rung printed nothing"* rather than as *"the device was wedged before it ran"*.

### §4.4 ★★★★★ BEFORE/AFTER `w383-doorbell-async`, ON ONE BOX — AND NOTHING MOVED

The measurement §0's correction block promised: **the same rung and the same scripts over two
shims**, one built from `30eb4627` (master **before** the async lane merged) and one from
`758a5752` (**after**), on **the same box `kb2`, within twenty minutes of each other**, with the
native calibration re-taken for each.

The row to read is the **`n=48` single-lap arm**: it is the only guest arm whose channel is
healthy from the opening control to the closing one, so it is the only one whose numbers are a
latency and not a post-mortem.

| | **pre-fix** shim `30eb4627` | **post-fix** shim `758a5752` | change |
|---|---|---|---|
| native `submit` p50 (worst of 3) | 3.66 µs | 2.77 µs | — |
| guest `submit` p50, `n=48` | **1194.95 µs** | **1159.06 µs** | **1.03×** |
| guest `bare` p50, `n=48` | **192.06 µs** | **183.99 µs** | **1.04×** |
| guest `freshmap` p50 | 1102.99 µs | 1150.81 µs | 0.96× |
| guest `drain_ms` over 3 windows | 0.2 | 0.2 | — |
| `first_stall_at` (`n=64` arm) | **63** | **63** | unchanged |
| `RUNG_missing_page` | FAIL | FAIL | unchanged |

⇒ ★★★★★ **On the raw client's doorbell path, the async lane's merge changed NOTHING
measurable — 3 % on the graded arm and 4 % on the bare doorbell, both inside this rung's own
between-process spread.**

⊘ **That is not a contradiction of `w383 §13`'s *"3.9× less publication wall"* and it must not be
reported as one.** Those are different subjects: that lane measured the LLM workload's publication
wall, this rung measures a doorbell rung by an ordinary unprivileged guest process on a channel it
created itself. What this establishes is the **scope** of the gain — it is not visible here — and
the scope is exactly the thing a five-second gate needs to state before anyone iterates against it.

### §4.4.1 ⊘⊘⊘ AND IT REFUTES THIS DOCUMENT'S OWN §3 ARGUMENT — THE RATIO IS NOT PORTABLE

§3 justifies the gate as `native_p50 × 1000` on the reasoning that *"any cost that is
arm-independent divides out"*. **Measured, that is false**, and the two boxes say so directly —
same GPU (GA106), same driver (580.159.04 open), same binary, same **post-fix** shim:

| box | native `submit` p50 | guest `submit` p50 (`n=48`) | **ratio** |
|---|---|---|---|
| `kb` | 9.31 µs | 448.49 µs | **48×** |
| `kb2` | 2.77 µs | 1159.06 µs | **418×** |

⇒ **8.7× apart.** The two arms move in *opposite* directions between the boxes: `kb2` is **3.4×
faster natively** and **2.6× slower in the guest**. Native cost is dominated by store latency to
device memory; guest cost is dominated by VM exits and what the VMM does behind them; **those
scale with different properties of the host, so they do not cancel.**

⚠ **The consequence is concrete and it is a defect in the gate, not in the boxes.** At 1000× the
gate has **2.4× of headroom on `kb2` and 21× on `kb`** — the same threshold is nearly tight on one
machine and nearly meaningless on the other, and nothing in the run says which one you are on.

⊘ **NOT FIXED HERE, DELIBERATELY.** Picking a new rule now would be choosing it from the two data
points that embarrassed the old one, which is the same mistake in a new place. The options, stated
for whoever takes it:
1. a **guest-side** reference — calibrate against this rung's own `n=48` arm on a known-good build
   of the same box, so both halves of the ratio are guest numbers;
2. per-box absolute calibration, recorded with the box id;
3. keep the native ratio but widen the band and stop claiming portability for it.
★ What survives untouched is everything the gate was NOT load-bearing for: the controls, the
stall bisection, and the before/after above — none of which consult the threshold at all.

### §4.4.2 ★ THE WEDGE IS OLDER THAN THE ASYNC LANE

`first_stall_at=63` and `n=48` clean / `n=64` wedged reproduce **identically on both shims and on
both boxes** — four independent (box, device-revision) pairs. ⇒ the `GP_PUT = 0` wedge is **not a
regression introduced by `w383-doorbell-async`**; it predates it and was simply never reached,
because nothing in this tree had submitted 64 times on one guest channel before.

### §4.3 ★★ `missing_page`, RE-ASKED ON BOTH ARMS, and it has NOT moved

```
XID_WATERMARK_BEFORE=9   ⊘ bracketed, never absolute — other lanes provoke Xid 31 too
info  R3 notifier   = fired=true status=0xffff except_type=0x1f engine=0x0001
★     R3 NAMED      = ... except_type 0x1f is what a host kernel log prints as `Xid 31`
★     R3 CONTAINED  = a channel in another address space kept landing across the fault
RUNGCTL_missing_page=PASS
RUNG_missing_page=PASS
XID_WATERMARK_AFTER=10  delta=1
```

⇒ the native half of w381 §4.1.2 reproduces exactly on this branch's binary. ⚠ The brief for
this lane recorded the host Xid watermark as **5**; it was **9** an hour later. That is not drift
in the measurement — it is why the rule is *bracket, never count absolutely*.

**And in the guest, on current master's shim, in the same boot as the latency rung:**

```
info  R3 notifier   = fired=false status=0x0000 except_type=0x0 engine=0x0000
FAIL  R3 SILENT     = the channel stopped and the notifier is quiet
★     R3 CONTAINED  = a channel in another address space kept landing across the fault
RUNGCTL_missing_page=PASS
RUNG_missing_page=FAIL
```

⇒ **Identical to w381 §4.1.2, digit for digit, after `w383-doorbell-async` merged.** Containment
holds; the fault is still never named. `missing_page` **has not moved**, and this is a
measurement of that rather than an inference from the previous lane's table.

⊘ **THE FIX IS NOT THIS LANE'S TO MAKE.** The guest-side half needs the device to author slot 0
at the guest-physical address in `errorNotifierMem.base` and to send `RC_TRIGGERED` with the
**guest's** ChID. Every file that would take that change —
`kayfabe-core/src/fault.rs`, `kayfabe-qemu-raw/src/shim.rs`, `kayfabe-rt/src/device.rs` — is
**owned by the live `w383-doorbell-async` lane**. So this lane asks the question in the same boot
(`w384_hook.sh`) and does not touch the answer.

★ **And the assertion the next lane needs is only half built.** `missing_page` grades **one** bit
(`n.fired()`), while the mechanism has **two mandatory halves**: the notifier write *and* the
`RC_TRIGGERED` event. A raw client can distinguish them — allocate an `NV01_EVENT` on the channel
and poll it — and until it does, a guest red cannot say *which* half is missing. That is the
sharpening to make before the fix, not after it.

## §5 HOW TO USE IT

```
# the whole differential, native calibration then guest arm, one command
KAYFABE_REPO=<tree> CARGO_TARGET_DIR=<dir> scripts/bench/w384_doorbell_latency.sh <tag>

# just the number, on any box with a GPU (~1 s):
kayfabe-rm-ladder --doorbell-latency

# graded against a floor somebody else measured:
kayfabe-rm-ladder --doorbell-latency --doorbell-latency-native-us 9.24
```

★ **The line to graph is `DBL_RATIO_X`**, printed on both outcomes. A pass/fail alone tells an
iterating lane nothing about whether it moved: two builds can both be red and be a factor of
forty apart. ⊘ A lane that only records its ratio when it fails cannot tell a fix from a lucky
boot.

Knobs (each edits the config, so order does not matter and any one of them implies the rung):
`--doorbell-latency-n`, `--doorbell-latency-budget-ms`, `--doorbell-latency-reps`,
`--doorbell-latency-native-us`, `--doorbell-latency-gate`.

And one **environment** knob, deliberately not a flag because it changes the CHANNEL rather than
the measurement: `KAYFABE_LADDER_GPFIFO_ENTRIES=<power of two ≤64>` gives the ladder's own
channel a smaller GPFIFO. ⊘ Unset ⇒ the committed 64, byte-identical to every other arm; set ⇒
it prints on stderr, every time, because an experiment whose arm is not on its own log cannot be
compared to anything. It exists for §4.2's separating experiment and nothing else.

★★★ **The harness lines a grader should read, in order of what they can say:**
1. `RUNGCTL_doorbell_latency=` — did the channel carry work **before and after** the loop. If
   `FAIL`, stop: nothing below is a latency.
2. `DBL_STALL … first_stall_at=` — the submission index whose drain window never retired.
3. `DBL_DIST arm=submit rep=POOLED` — the distribution, five order statistics and a total.
4. `DBL_RATIO_X` — the continuous number to graph.
5. `RUNG_doorbell_latency=` — the gate. ⚠ **Last**, because §4.4.1 measures the gate itself to be
   the least portable thing this rung prints.

## §6 THE BRANCH'S OWN HEALTH — checked against a baseline, not asserted

- **`cargo test --workspace --all-targets --features host-isolates --no-fail-fast`**, on the
  bench at `5452669c` (the branch tip, **after** the rebase onto master `758a5752`): **235 test
  binaries ran**, and exactly **three targets fail** —
  `admitted_is_served`, `doorbell_reaches_the_completion_observer`,
  `ring_out_of_our_own_framebuffer`. That is **the same set master already fails, not a
  superset**. ⚠ `--no-fail-fast` is not optional: without it `cargo test` stops at the first
  failing *target* and reports a stopping point rather than a result, and the binary count is
  what makes *"the list shrank"* distinguishable from *"the list was truncated"*.
  ⊘ w381 §6 recorded **234** binaries at `4a501a7f`; the count moved with master, the **set** did
  not, and the set is the assertion. ⚠ It was run **twice** — once before the rebase and once at
  the tip — because a rebase moves the code onto a different master and *"the tests passed"* on
  the pre-rebase commit is a statement about a tree nobody will ever check out.
- **`cargo fmt -p kayfabe-isolate-host -- --check`**: the remaining hunks are byte-for-byte the
  pre-existing ones (`rmladder.rs:58`, `:65`, `child.rs`, `isolate.rs` ×2, `rm.rs:5316`, four in
  `tests/`). ⊘ `rustfmt` wanted to reformat three of those as a side effect and they were
  **reverted deliberately**: a formatting change outside the range this branch touches makes a
  `HEAD~1` baseline comparison useless, which is the only thing that turns *"fmt is clean"* into
  a checkable claim.
- **`cargo clippy -p kayfabe-isolate-host --all-targets`**: no warning in any range this branch
  touches. ⚠ It caught one on the way — `manual_range_contains` inside `ladder_gpfifo_entries`,
  which is exactly the kind of thing that gets waved through as *"it is only an experiment's
  knob"* and then lives in `rm.rs` forever; it was fixed rather than allowed. The ones that remain are the pre-existing set w381 §6 already names
  (`rm.rs`'s two collapsible `if`s, `export.rs:97`'s missing doc, the `chunks_exact` family).
  ⊘ `--features host-isolates` does **not** exist on this package — it belongs to
  `kayfabe-qemu-raw`, and passing it makes clippy fail with a *feature* error that reads like a
  lint failure.
- ⚠ **Nothing in `kayfabe-rt/`, `kayfabe-core/` or `kayfabe-qemu-raw/` was changed.** Those were
  the `w383-doorbell-async` lane's, and this rung was built to measure them from outside rather
  than to touch them. The non-`bin` changes are three additions to
  `kayfabe-isolate-host/src/rm.rs` — `mute_doorbell_witness`, `ring_doorbell_only` and
  `ladder_gpfifo_entries` — of which the first two add no caller to any existing path and the
  third replaces one use of a constant with a function that **returns that constant unless an
  environment variable overrides it, and prints on stderr when it does**. ⇒ every previously
  committed arm is byte-identical.
