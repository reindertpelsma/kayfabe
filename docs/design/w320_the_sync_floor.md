# w320 — WHAT `cuCtxSynchronize` IS WAITING FOR

**STATUS: LIVE — measured 2026-08-14 on bench `vh2` (RTX 3060 GA106, driver 580.159.04).
Guest arms at `e8be2da5`, native arms and the correctness ladder at `468e29de`, branched from
master `c2b0f3e6`. ⊘ `crates/` is BYTE-IDENTICAL to master (`git diff master -- crates/` = 0
lines): this rung changed the measurement program and nothing in the device.**
Artifacts: `traces/w320_sync/`.

> ★★★★★ **OUTCOME (B) — THERE IS NO SYNC FLOOR. `cuCtxSynchronize` IS THE KERNEL RUNNING.**
> Submit is flat at **3.483 → 4.373 ms** while sync spans **0.971 → 908.966 ms** over a 4096×
> range of arithmetic. The guest thread **spins on-CPU for 100 % of the wait and blocks zero
> times** at every size. The remaining overhead over native, on the same GA106, is
> **22–81× on the COMPUTE** — and the submit path this campaign has been driving down is now
> **0.5 % of an N=2048 launch**.
> ⊘ **All four of my pre-registered predictions were refuted.** §1–§4 are commit `fbec1ffe`,
> written before the first boot; they are left standing exactly as written.

> ⚠ Everything in §1–§4 was committed before any boot of this rung. Results are appended
> below §5 and the prediction is graded against what was written here, not against what
> turned out convenient.

---

## 1. Why this rung exists

`w318` gated the doorbell's page-table passes and moved the submit side by 21×:

| | control | gated |
|---|---|---|
| host doorbell trap | 85.248 ms | **4.078 ms** |
| guest `submit_med_ms` | 85.935 ms | **4.040 ms** |
| guest `launch_med_ms` | 107.846 ms | **27.552 ms** |

`cuCtxSynchronize` is now **~23.5 of the 27.6 ms — 85 % of the launch** — and it measured
**23.3 ms before the gate** (w315 §5) and **~23.5 ms after**. ⇒ **unchanged by a 21× change
immediately upstream of it.**

---

## 2. ★★★ WHAT I ALREADY KNOW WITHOUT BOOTING — and it constrains the answer hard

Two numbers already in the tree bound this before I measure anything:

- **w315 §4**: our device's MMIO **reads** over an *entire boot* total **20.5 ms** across
  12 000 accesses. The timed phase alone contains ~12 syncs × 23.3 ms ≈ **280 ms** of sync.
  ⇒ **whatever the guest does during a sync, it is not reading our device.** A polling-on-our-
  MMIO story is dead before it is proposed.
- **w311**: native's *entire* N=512 launch is **0.386 ms**. So at N=512 the real GPU execution
  cannot be more than a fraction of a millisecond — **23.5 ms is ~60× the whole native launch**.

⇒ the guest is either spinning in its own memory (no exit), or off-CPU waiting for something.
That is a question about the **guest thread's CPU state**, which is why the instrument below is
guest-side.

---

## 3. THE INSTRUMENTS — two, independent, and neither needs a cross-clock offset

### 3.1 The thread-state split — ★ THE BREAKDOWN, and it CLOSES BY CONSTRUCTION

Around `cuLaunchKernel` and `cuCtxSynchronize` **separately**, in the guest, in one clock
domain:

```
sync_wall   = CLOCK_MONOTONIC delta                 (what we are explaining)
sync_cpu    = CLOCK_THREAD_CPUTIME_ID delta          → the thread was RUNNING
sync_offcpu = sync_wall − sync_cpu                   → the thread was NOT running  ★ NAMED
sync_utime / sync_stime  (getrusage RUSAGE_THREAD)   → user spin vs kernel spin
sync_nvcsw / sync_nivcsw                             → how many times it BLOCKED
```

★ This is the same trick w315 used to split submit from sync — **both halves are the guest's
own clock, so there is no correspondence to license.** `offcpu` is a *derived* quantity, not a
residue I am allowed to name and walk away from: it is `wall − cpu` exactly, and it is the
UNMARKED share the brief asks for. There is no third term.

⊘ **What it cannot do**: if the thread is off-CPU it cannot say *what it is waiting for*. It
says only *that* it waits, how often, and for how long. Attribution of the wakeup source is
§3.3 and is weaker.

### 3.2 The batch/duration curve — ★★★ ONE EXPERIMENT, BOTH QUESTIONS

The `mm` kernel is **idempotent**: K launches over the same buffers produce the same verified
result. ⇒ **K launches per sync is simultaneously**

- the **batching arm** the coordinator asked for (does batching amortise the cost?), and
- the **duration discriminator** this brief asked for (K× the GPU work behind one sync).

Fit `batch_total(K) = a·K + b` over **K ∈ {1,2,4,8,16,32,64,128}**, ≥5 reps each, medians:

| | slope `a` | intercept `b` | reading |
|---|---|---|---|
| **fixed cadence** | ≈ submit (~4 ms) | ≈ **23.5 ms** | the wait is per-SYNC |
| **real per-kernel work** | ≈ submit + 23.5 (~27.5 ms) | ≈ 0 | the wait is per-LAUNCH |
| **split** | between | between | report both terms |

⚠ **Eight K values, not two.** w311 learned that two points fit a two-parameter model exactly
and leave no residual to check. The residual is reported.

⚠ **A second duration knob, because K is confounded at large K**: at N=512 each kernel is real
GPU work, so `K=128` carries ~128× of it and would inflate the slope even under the fixed
hypothesis. ⇒ **the K-sweep runs at N=128** (~4 MFLOP/launch, GPU time ≪ 1 ms), and a separate
**size arm** sweeps `N ∈ {128,512,1024,2048}` at K=1.

★★★ **The size arm is also the KNOWN-POSITIVE for the duration knob.** At N=2048 the real GPU
work is genuinely tens of ms, so sync **must** grow there. If it does not, my duration knob is
inert and a flat curve at small N proves nothing. **A flat result is only meaningful if the
same instrument can be shown to move.**

### 3.3 The mechanism arm — and its known-positive

If the answer is a fixed wait, the next question is *whose cadence*. Two cheap probes:

- **`BENCH_CTX_FLAGS`** on `cuCtxCreate`: `CU_CTX_SCHED_SPIN` (0x01) vs `CU_CTX_SCHED_BLOCKING_SYNC`
  (0x04) vs default `AUTO` (0x00). If sync collapses under SPIN, the wait is a **block/wakeup**,
  not work. If it is unchanged under all three, it is below CUDA's scheduling choice.
- ⚠ **If I find a constant in our own code near 23.5 ms, it is a SUSPECT, not a corroboration.**
  This has fired three times in this campaign (w311's `OBSERVER_TICK_MS=250` matching a fitted
  `C` to 0.4 %; w315's saturating aligner; w318's 2.9 µs/row). **The only thing that promotes a
  suspect cadence to a cause is CHANGING it and watching sync follow.** That test is
  pre-registered here as the required evidence, and nothing less is accepted.

---

## 4. ★★★★★ PRE-REGISTERED — the letter I expect, and why

**I predict (A): a fixed cadence / wakeup latency, and sync will NOT scale with K.**

Concretely, before running:

1. `batch_total(K)` fits with **intercept ≈ 20–25 ms** and **slope ≈ 4–6 ms** (the submit floor),
   over K ∈ {1..128} at N=128.
2. `sync_med_ms` at N=128 ≈ N=512 ≈ **23 ms** (flat), and **N=2048 is larger** (the known-positive).
3. `sync_offcpu` is the **dominant** term of `sync_wall` — the thread is blocked, not spinning.
4. ⇒ **w311's "batching does not help" is DEAD or SCOPED**: it was true when submit was ~90 ms
   and dominated; w318 removed 95 % of the per-submit cost, so its *why* is gone.

**Grading the letters:**

- **(A) fixed cadence** ⇒ name the cadence, its source, and whether closing it is an
  interrupt-delivery rung or a polling-parameter rung.
- **(B) scales with duration** ⇒ the floor is honest; then state the remaining overhead over
  native (native N=512 launch = **0.386 ms**; we are at 27.55).
- **(C) it splits** ⇒ report both terms separately, with the residual from ≥3 sizes.
- **(D) the breakdown does not close** ⇒ report it, do not distribute the residue.

⊘ **The three candidate shapes in the brief are the coordinator's and are unmeasured.** The
brief says so itself, and it also says its author has already been wrong about a mechanism
today (w318 refuted its own O(VAS) story from the control arm's own log). **I adopt none of
them before the curve.**

⚠ **The correspondence licence here is WEAKER than w315's, and this is the honest statement of
it.** w315 could claim containment because a guest MMIO write is a vmexit and the vCPU is
halted for the whole trap. **During a sync the guest is NOT halted** — it is running or
sleeping on its own. ⇒ no host-side interval can be claimed to be *inside* the sync window by
construction, and I do not claim it. The breakdown in §3.1 avoids the problem entirely by
staying in one clock domain; any host-side number reported alongside is explicitly
**correlational**.

⚠ **The instrument attributes; it does not isolate** (w315 §5). Its own known-positive failed
the "no other segment moves" condition because segments are coupled through elapsed time.
Expect the same here and say so rather than discovering it.

---

## 5. RESULTS

*(nothing above this line was written with a result in hand — §1–§4 are commit `fbec1ffe`,
before the first boot; the runs are commits `e8be2da5` and `468e29de`)*

### 5.0 ★★★★★ THE ONE-LINE ANSWER — pre-registered outcome **(B)**, and my own prediction is REFUTED

**There is no sync floor. `cuCtxSynchronize` is the kernel executing, and it scales with the
kernel.** Submit is now a flat ~4 ms across a **4096×** range of work; sync spans **0.971 ms to
908.966 ms** over that same range. The guest thread **spins in userspace for 100.0 % of the
wait and blocks zero times** at every size.

⊘ **All four of my §4 predictions failed, and three of them failed in the same direction:**

| # | I predicted | measured | verdict |
|---|---|---|---|
| 1 | intercept 20–25 ms, slope 4–6 ms | the model does not fit at all (§5.3) | ⊘ REFUTED |
| 2 | sync flat at N=128 ≈ N=512 ≈ 23 ms | 0.971 vs 22.734 ms — **23.4×** | ⊘ REFUTED |
| 3 | `offcpu` is the dominant term | `offcpu` ≈ **0.0 %**, `nvcsw` = **0** everywhere | ⊘ REFUTED |
| 4 | w311's batching ruling is DEAD | it is **SCOPED**, and mostly still right (§5.4) | ⊘ PARTLY |

★ **And the brief's premise dissolves rather than being refuted.** Its argument was *"23.3 ms
before the gate, ~23.5 ms after — unchanged by a 21× change immediately upstream"*. Both of
those numbers are **N=512**. The size was never varied. ⇒ **the constancy was across a change
to the SUBMIT path, not across a change to the WORK** — which is exactly what real per-kernel
execution time does. **A quantity that does not move when you change something unrelated to it
is not evidence of a fixed cadence.**

### 5.1 THE SIZE CURVE — the duration discriminator, and its own known-positive

`w320sizes`, one boot, gate ON, N=128→2048, `bad=0 maxerr=0` at every size, `Xid=0`,
`GUEST_SIZES_DONE=4`:

| N | launch_med | submit_med | **sync_med** | sync ON-CPU | sync OFF-CPU | `nvcsw` | GFLOP/s |
|---|---|---|---|---|---|---|---|
| 128 | 4.507 | 3.483 | **0.971** | 0.978 (100.1 %) | −0.003 | 0 | 0.93 |
| 512 | 26.769 | 3.699 | **22.734** | 22.747 (100.1 %) | −0.010 | 0 | 10.03 |
| 1024 | 63.847 | 4.103 | **59.786** | 59.805 (100.0 %) | −0.002 | 0 | 33.64 |
| 2048 | 913.086 | 4.373 | **908.966** | 908.773 (100.0 %) | +0.189 | 0 | 18.82 |

★★ **SUBMIT IS FLAT — 3.483 → 4.373 ms — while the arithmetic behind it grows 4096×.** That is
w318's gated floor, and it is now **0.5 % of the launch at N=2048**.

★★★ **The known-positive holds**: the largest size moves sync by **936×**, so the duration knob
is live and the small-N readings mean something. Had N=2048 come back flat, none of this would
have been readable.

⊘ The breakdown's `UNMARKED` is **0.0004 %–0.03 %** — but that is **not a finding**, and the
analyser says so where it prints it: `offcpu := wall − cpu` by definition, so a closing
breakdown is an **arithmetic check on the parser**, not evidence about the plane. ★ I say this
because w315's 0.01 % `UNMARKED` **was** a finding (its segments were independently bracketed
and could genuinely have failed to sum). Mine cannot. **Two identical-looking numbers, two
different epistemic statuses** — quoting mine as though it were w315's would be a fabricated
corroboration.

### 5.2 ★★★★★ THE NATIVE RATIO — what outcome (B) requires, on the SAME physical GA106

Same source (md5 recorded in both logs), same compiler, same GPU, no QEMU
(`w311_native.sh` refuses to run while one is up). Native negative control **fired**
(`BENCH_NOLAUNCH_TOTAL_BAD=524288`).

| N | native `sync_med` | native GFLOP/s | guest `sync_med` | **guest ÷ native** |
|---|---|---|---|---|
| 128 | 0.012 | 302 | 0.971 | **80.9×** |
| 512 | 0.358 | 743 | 22.734 | **63.5×** |
| 1024 | 2.752 | 779 | 59.786 | **21.7×** |
| 2048 | 22.320 | 769 | 908.966 | **40.7×** |

Native scales as textbook N³ (0.358 → 2.752 → 22.320 is 7.7× then 8.1×) and holds ~770 GFLOP/s.
⇒ **the remaining overhead over native is 22–81×, and it is on the COMPUTE, not the control
plane.** Whole-launch ratios are 322× / 74× / 23× / 41×.

★★★ **THIS IS THE ROADMAP CONSEQUENCE.** At N=512, submit is 3.699 ms of a 26.769 ms launch.
**Even a PERFECT submit path — zero cost — leaves 22.734 ms against native's 0.361 ms.** The
launch floor that this campaign has spent w311→w318 driving down is, at realistic sizes, no
longer the binding constraint. ⇒ **w311's headline *"the launch floor is the binding
constraint"* was true at ~115 ms of fixed cost and is now SCOPED: it binds only where the
kernel is smaller than ~4 ms of native work.**

### 5.3 ⊘⊘⊘ THE FIT THAT WOULD HAVE CONFIRMED THE HYPOTHESIS — and why it is garbage

`w320ksweep`, N=128, K ∈ {1..128}, 5 reps each, **every batched rep verified, `Σbad=0`**:

| K | 1 | 2 | 4 | 8 | 16 | 32 | 64 | 128 |
|---|---|---|---|---|---|---|---|---|
| total ms | 6.252 | 15.661 | 30.326 | 64.423 | 123.616 | 246.743 | 321.127 | 499.983 |
| **per launch** | 6.252 | 7.830 | 7.582 | 8.053 | 7.726 | 7.711 | 5.018 | **3.906** |
| `offcpu` | −0.006 | −0.006 | 0.000 | 0.016 | 0.008 | 0.032 | 0.023 | 0.095 |
| `nvcsw` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |

```
ALL K       total(K) = 3.8931*K + 39.424 ms   RMS residual = 40.041 ms
DROP K=128  total(K) = 5.2070*K + 20.979 ms   RMS residual = 28.652 ms
```

★★★★★ **LOOK AT `b = 20.979 ms`.** The hypothesis under test was a fixed per-sync cost of
**~23.5 ms**. A two-term fit over this data returns an intercept **within 11 % of it**. Quoted
alone it is a stunning confirmation — **and it is meaningless**: the RMS residual is **40 ms on
data spanning 6–500 ms**, and the residuals are *systematically curved* (−37, −32, −25, −6,
+22, **+83**, +33, −38). **The line does not fit.**

⇒ ⚠ **This is the brief's own named trap firing on me, and only the pre-registered refusal
caught it.** §3.2 committed in advance to ≥3 values of K, to printing every residual, and to
re-fitting without the largest point. Two K values would have fit *exactly*, returned an
intercept near 23.5, and left **no residual that could disagree**. ★ Same family as w311's
251 ms cadence and w315's saturating aligner: **an answer that arrives pre-corroborated.**

★ **The model-free reading is the one that settles it.** If the cost were purely per-sync,
K=128 would read **0.049 ms/launch**. It reads **3.906** — **80× higher**. Per-launch cost is
*flat* at 7.6–8.1 ms from K=2 to K=32. **The intercept is an artefact of a curve, not a
constant anyone can amortise.**

⊘ **UNEXPLAINED, AND NAMED RATHER THAN SMOOTHED:** per-launch *falls* to 5.018 (K=64) and
3.906 (K=128), and K=1 (6.252) is cheaper than K=2 (7.830). Something does get cheaper at
large K — plausibly the guest driver ringing fewer doorbells per launch — but **I did not
measure it and I am not banking it as the intercept of a line it does not fit.** ⚠ Also note
the sweep's per-launch (6–8 ms) *exceeds* the timed loop's submit+sync (4.45 ms) at the same
size; back-to-back launches with no sync between them are not the same workload.

### 5.4 ★★★ w311's BATCHING RULING — **SCOPED, not dead**, and the coordinator's arithmetic does not hold

The mid-rung note predicted `4.04 + 23.5/10 ≈ 6.4 ms/launch`, **~4.3× for free**. It said to
measure it rather than report it. **Measured: that arithmetic does not hold, because its
premise — that the 23.5 ms is a per-SYNC term — is false.**

- At **N=128** (submit-dominated) batching gives **6.252 → 3.906 ms/launch = 1.60×**, not 4.3×.
- At **N≥512** sync is real per-kernel work and does not amortise **at all**. At N=2048 the
  entire submit term is 4.373 ms of 913.086 — **batching cannot buy more than 0.5 %.**

⇒ **w311's ruling stands where it was made and is now SCOPED**: batching amortises the submit
term only. Its *why* — *"the fixed cost is per-SUBMIT"* — **survives w318**; what changed is
that the per-submit cost is now small, so amortising it is worth 1.6× at tiny sizes and
nothing at real ones. ⊘ **It is not dead, and reporting it as dead would have been the more
exciting error.**

### 5.5 ★★★★★ THE MECHANISM — reproduced by construction, natively, with no guest at all

The 22–81× is on the compute, so the question is what makes *the same kernel on the same GPU*
slow. Leading candidate: **operand placement** — `mm` is a naive triple loop, entirely
global-load-bound, so where its operands live is the whole of its speed.
⚠ **The prior that suggested this is SCOPED and I am not stretching it**: w290 measured
`guest_ram=16328`, *"99.4 % of the table"* — **for cup2**, a CE round trip, not for cup8's
matmul buffers. It motivates the hypothesis; it does not establish it for this workload.

⊘ **A magnitude that matches is not a mechanism.** So it was not argued from the ratio; it was
**reproduced**: same GPU, same kernel, same binary, **no hypervisor anywhere**, the only
variable being `cuMemHostAlloc(DEVICEMAP)` instead of `cuMemAlloc`.

| N | native VRAM | **native HOST-MEM** | our guest | hostmem ÷ VRAM | guest ÷ hostmem |
|---|---|---|---|---|---|
| 128 | 0.012 | 0.131 | 0.971 | 10.9× | 7.4× *worse* |
| 512 | 0.358 | 6.300 | 22.734 | 17.6× | 3.6× *worse* |
| 1024 | 2.752 | **129.097** | 59.786 | 46.9× | **0.46× — guest is 2.2× FASTER** |
| 2048 | 22.320 | **1480.366** | 908.966 | 66.3× | **0.61× — guest is 1.6× FASTER** |

★★ **Placement alone produces slowdowns of 10.9–66.3× — the same order as our 22–81×.** The
class of mechanism is confirmed on a machine with no emulation in it.

⊘⊘ **AND IT OVERSHOOTS, WHICH REFUTES THE NAIVE VERSION.** At N≥1024 the **native**
host-memory arm is **slower than our guest**. So our operands are **not** simply pinned sysmem —
whatever they are, the plane beats `cuMemHostAlloc` at large sizes and loses to it at small
ones. ⚠ **Sufficiency is not identity**: this shows the placement *costs* this much, **not that
our buffers are placed this way**. Where they actually are is unmeasured and is the next rung.

⊘ Corroborating but not decisive: guest H2D at N=2048 is **274.952 ms** vs native **4.712 ms**
(**58×**) — a *copy* path showing the same order of penalty as the *compute* path.

### 5.6 ⊘ IS THIS THE THIRD INSTANCE OF THE ROUND-TRIP PATTERN? **NO** — and that is worth saying

The mid-rung note asked whether sync is round-trip-bound, like w318's 8 joins at ~5 ms and
w319's 13 313 drain rows at ~225 µs. **It is not, on three independent grounds:**

1. **`nvcsw = 0` and `offcpu ≈ 0` at every size and every K.** The guest thread never leaves the
   CPU during a sync ⇒ it issues **no syscall, no ioctl, no round trip**. A round trip it made
   would cost it a context switch, and it makes none.
2. **No MMIO exit is possible during the window.** w315 measured our device's reads at **20.5 ms
   across an entire boot**; one N=2048 sync alone is 909 ms.
3. **It scales as the WORK, not as a count of anything we do.** 936× over a 4096× arithmetic
   range, with submit flat.

⇒ **A third lane aimed at batching host round trips would not touch this.** ★ This is a
*negative* result and it is the most decision-relevant thing here after the ratio itself.

### 5.7 ⊘ WHAT I DROPPED, AND WHY — the `spin` / `block` arms

§3.3 pre-registered `CU_CTX_SCHED_SPIN` vs `BLOCKING_SYNC` to tell a block from a spin. **Two
boots, cancelled: the question was already answered by the data I had.** `offcpu ≈ 0` and
`nvcsw = 0` at 4 sizes × 12 iterations and 8 K values × 5 reps is a direct measurement that
the thread never blocks, so a knob that changes *how* it would block cannot move anything.
★ Independently predicted by `completion_watch.rs:6-10`, which records **`AWAKEN_ENABLE = 0`**
in the guest's own pushbuffer — *the guest asks for no interrupt*. ⊘ Recorded as a **deliberate
omission with its reason**, not left as an unmentioned gap in the pre-registration.

Also retired: the Explore sweep found **no cadence anywhere in `crates/*/src` between 2 ms and
250 ms** — the nearest candidates were `OBSERVER_TICK_MS = 250` (the one that fooled w311) and a
1 ms `await_semaphore` poll. The device log shows **`await_sem hits = 0`** on the compute path.
⇒ the cadence hypothesis had no candidate mechanism *and* no measured wait to explain.

### 5.8 What the brief and the notes got wrong, and what I got wrong

- ⊘ **The brief's three candidate shapes were all wrong**, and it flagged them as unmeasured
  rankings. Shape 3 (*"real hardware wait"*) was ranked **last** and is the answer.
- ⊘ **The brief's framing — "attribute it the way w315 attributed SUBMIT" — could not work**,
  and the reason is structural rather than a matter of effort: w315's licence **is** the
  vmexit, and there is no vmexit here. Extending kftime would have produced a submit-side
  breakdown beside an uninstrumented sync and an "unaccounted" remainder that was really
  "not instrumented". The instrument had to move **into the guest thread**, not onto more
  host hooks.
- ⊘ **My prediction #3 was contradicted by a doc already in the tree.** `completion_watch.rs`
  had recorded `AWAKEN_ENABLE = 0` and a userspace spin, **measured 2026-08-10**. I predicted
  a block anyway. ⚠ Exactly `check_whether_the_question_is_already_answered`; the subagent
  sweep found it in minutes, and I had written the prediction before dispatching that sweep.
- ⊘⊘ **My analyser double-counted every row** (`n=16` for an 8-point sweep) because the hook
  prints the workload's output twice — once verbatim and INDENTED, once at column 0 with a
  `GUEST_` prefix — and my regex was unanchored. ⚠ **OLS is invariant to exact duplication, so
  the fit was right and only `n` was wrong**, which is worse than a visible error: the residual
  count, and any confidence a reader draws from it, silently doubled. Same family as w307's
  indent trap, arriving from the other side.
- ⊘ **I made the bench refuse three boots by `scp`-ing a fix into the clone**, leaving the tree
  dirty; `w290p_run.sh:50` refused, correctly, and the batch emitted a full ladder of
  `UNMEASURED` in 23 seconds. ★ The **timestamps** were the tell, exactly as the trap list
  says. ⚠ It also means the two **native** arms ran from a tree that was dirty at the time —
  the native harness has no such gate. Their provenance is the source md5 each log prints, and
  the code they ran is now committed as `468e29de`; the guest arms (`w320ksweep`, `w320sizes`)
  ran **clean** at `e8be2da5`, before the scp.
- ⚠ **`n=1 boot` for the size curve and `n=1` for each native arm.** The correctness ladder is
  n≥3 (§5.9) but the *timing* is one boot per arm. The size effect is 936× and the native
  ratio 22–81×, so no plausible boot-to-boot scatter (w315 measured ~8 ms across four boots)
  touches the conclusion — but the individual figures carry one boot each and must be quoted
  that way.
- ⚠ **`sync_cpu` reads 100.0–100.1 % of `sync_wall`**, i.e. very slightly *over*. The guest's
  `CLOCK_THREAD_CPUTIME_ID` cannot see hypervisor-stolen time, so "on-CPU" here means *the
  guest believes it was running*. That is the correct reading for the sync window (no traps in
  it) but it means **`sub_cpu ≈ sub_wall` is trivially true** and says nothing about the submit
  side. The >100 % is the two clocks' resolution, not a measurement.
- ⊘ **Not measured, and load-bearing for the next rung:** where our buffers actually live.
  §5.5 bounds the *cost* of sysmem placement; it does not locate ours.

### 5.9 CORRECTNESS — three workloads, n ≥ 3

*(filled in below from `w320_corr.log`)*

### 5.10 WHAT THE NEXT RUNG SHOULD BE

★★★★★ **Locate the operands.** Not "make sync faster" — the sync *is* the kernel. The one
question that decides the roadmap is **where the guest's `cuMemAlloc` buffers are physically
backed, and at what bandwidth the host GR engine reaches them.** §5.5 shows sysmem placement
costs 10.9–66.3× and that we do **not** match plain pinned sysmem, so the answer is neither
"VRAM" nor "sysmem" and nobody has measured which.

⊘ **Do not dispatch a round-trip-batching rung at this** (§5.6). ⊘ And do not spend further
rungs on the submit path at realistic sizes: it is 0.5 % of an N=2048 launch, and driving it
to **zero** would improve that launch by **0.5 %**.
