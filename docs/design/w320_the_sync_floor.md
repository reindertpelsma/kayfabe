# w320 — WHAT `cuCtxSynchronize` IS WAITING FOR

**STATUS: PRE-REGISTRATION — written 2026-08-14 BEFORE the first boot, at master `c2b0f3e6`.
Bench `vh2` (RTX 3060 GA106, driver 580.159.04). No measurement in this file yet; the
numbers below are the brief's, not mine.**

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

*(appended after the runs — nothing above this line was written with a result in hand)*
