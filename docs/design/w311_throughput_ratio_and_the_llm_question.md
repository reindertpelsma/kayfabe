# w311 — THE THROUGHPUT RATIO, AND WHAT IT SAYS ABOUT AN LLM

**STATUS: LIVE — measured 2026-08-14, two guest boots + one native run, all on the SAME
physical GA106 (`vh`, RTX 3060, driver 580.159.04).**
Source revision: **`564185e0`** (guest arm binary stamped `89defd9c`, stamp gate passed on
both boots). Artifacts: `traces/w311_bench/`.

> ⚠ **Every number here is per-launch STEADY STATE unless it says otherwise.** w308's *"39 s
> workload wall"* is a one-shot figure that fuses `cuInit`, context creation, the PTX JIT,
> allocation, publication, two copies, ONE launch and an O(N²) host verify. It is not a
> throughput number and this document does not use it as one. (It is, however, *explained*
> below: ~27 s of it was startup.)

---

## 1. THE HEADLINE — the ratio is 0.003 → 0.036, and it IMPROVES with size

Guest ÷ native GFLOP/s, per-launch median, first launch discarded, same physical GPU, same
source file (md5 `199a5a8e…` on both arms — a comparison, not a claim):

| N | guest med | native med | guest GFLOP/s | native GFLOP/s | **RATIO** | gap (ms) |
|---|---|---|---|---|---|---|
| 512  | 126.132 ms | 0.386 ms  | 2.128  | 695.740 | **0.00306** | 125.75 |
| 1024 | 198.084 ms | 3.010 ms  | 10.841 | 713.453 | **0.01520** | 195.07 |
| 2048 | 623.790 ms | 22.469 ms | 27.541 | 764.616 | **0.03602** | 601.32 |

⇒ **Pre-registered outcome (C): ratio < 0.1 at every size.** This is a full result, not a
failure, and it is far more valuable now than after building on the assumption.

★ But the ratio is not a constant: it improves **12×** from N=512 to N=2048. That is the
signature of a large FIXED per-launch cost being amortised, and it is the single most
important structural fact in this document.

---

## 2. ⊘⊘ THE BRIEF POSED (D) AND (E) AS ALTERNATIVES. **BOTH ARE TRUE AT ONCE.**

The rung pre-registered *(D) per-launch cost flat in N ⇒ fixed overhead dominates* and
*(E) per-launch cost scales with N ⇒ we are in the data path*, as if the measurement would
pick one. It does not. Fitting `guest_med = C + k · native_med` over the three sizes — and
**three sizes is what made this possible; two points fit a two-parameter model exactly and
leave no residual to check** — gives:

| fitted from | k (proportional) | C (fixed, ms) |
|---|---|---|
| N=512, 1024  | 27.4× | 115.6 |
| N=512, 2048  | 22.5× | 117.4 |
| N=1024, 2048 | 21.9× | 132.2 |

⇒ **C ≈ 115–132 ms of FIXED per-launch cost (D), PLUS a ≈22–27× slowdown on the
size-proportional part (E).** Reporting either one alone would have been wrong.

### 2.1 The batch phase splits the fixed cost again — and most of it is NOT the sync

K=10 launches enqueued back-to-back with ONE `cuCtxSynchronize` at the end. Solving
`solo = E + s` and `batched_per_launch = E + s/10`:

| N | solo (ms) | batched/launch (ms) | ⇒ per-launch E | ⇒ per-sync s |
|---|---|---|---|---|
| 512  | 126.132 | 115.014 | 113.8 | 12.3 |
| 1024 | 198.084 | 107.540 | 97.4  | 100.7 |
| 2048 | 623.790 | 471.661 | 454.8 | 169.0 |

⇒ **~100 ms per launch survives batching.** It is a per-SUBMIT cost, not a per-SYNC cost, so
**batching does not rescue it** — an LLM that enqueues 200 kernels and syncs once still pays
~100 ms × 200. ⚠ E at N=512 (113.8) and N=1024 (97.4) disagree by ~16 ms; treat the floor as
*~100 ms, ±15*, not as a precise constant.

---

## 3. ★★★★★ THE COPY PLANE IS A FLAT ~9 MiB/s — the cleanest number in the run

| N | dir | MiB | guest ms | native ms | guest MiB/s | native MiB/s | **guest µs per 4 KiB page** |
|---|---|---|---|---|---|---|---|
| 512  | H2D | 2  | 232.069  | 0.615 | 8.62 | 3252 | 453.3 |
| 512  | D2H | 1  | 102.575  | 0.252 | 9.75 | 3968 | 400.7 |
| 1024 | H2D | 8  | 1019.768 | 1.101 | 7.84 | 7266 | 497.9 |
| 1024 | D2H | 4  | 446.494  | 0.746 | 8.96 | 5362 | 436.0 |
| 2048 | H2D | 32 | 3459.421 | 4.195 | 9.25 | 7628 | 422.3 |
| 2048 | D2H | 16 | 1729.489 | 2.136 | 9.25 | 7491 | 422.2 |

⇒ **~420–500 µs per 4 KiB page, in BOTH directions, at ALL three sizes.** Bandwidth is flat
at ~9 MiB/s across a 32× span of transfer size — perfectly linear in bytes, ~**800× slower
than native**. This is the most mechanically legible finding in the rung: the copy plane
costs a fixed amount *per page*, and nothing else about it varies.

⊘ The copies sit OUTSIDE the timed launch window on both arms, so they are **not** in the
headline ratio. They are reported because an LLM's weights are resident but its activations,
KV-cache traffic and logits readback are not.

---

## 4. STARTUP, reported separately — and it explains w308's 39 s

| | guest | native | ratio |
|---|---|---|---|
| `cuInit` | 11 455 ms | 68.6 ms | 167× |
| `cuCtxCreate` | 14 919 ms | 302.7 ms | 49× |
| `cuModuleLoadData` (PTX JIT) | 430 ms | 0.15 ms | — |
| **total before any work** | **~26.8 s** | **~0.37 s** | **~72×** |

⇒ w308's 39 s was **~27 s of startup**, ~4 s of copies, <1 s of launch and the rest host-side
verification. ★ Paid ONCE per process. For a long-running inference server this is a boot
cost, not a throughput cost, and it must not be counted against tokens/s.

---

## 5. PUBLICATION IS A STARTUP COST — w308's reading holds at ~90 launches

`★DRAINED` rows = **877**, Σ drain wall = **7 191 ms**, across the WHOLE run: 3 sizes, 60 solo
+ 30 batched launches, 2 processes. w308 measured **275 rows / 3 894 ms for ONE launch**.

⇒ ~90× the launches cost **less than 2×** the drain time. Two drains (`pinned=13263` in
3 000 ms and `pinned=13313` in 2 864 ms — the first doorbell of each process) account for
**5 864 of the 7 191 ms**; the other 875 drains total ~1 327 ms.

⇒ **Publication is NOT in the per-launch path.** w308's *"publication cost is essentially
independent of allocation size"* is joined by *"…and independent of launch count."* Whatever
the ~100 ms per-launch floor is, **it is not the drain.**

---

## 6. ⊘⊘ AN INSTRUMENT ARTEFACT I NEARLY REPORTED AS A MECHANISM

The device's `SEMA-WRITE` lines carry timestamps, and the GR completion semaphore writes
arrive on a **hard 251 ms cadence** — min 251, median 252, and every inter-cluster gap a
multiple (501, 2 258, 2 506, 3 009). It reads exactly like a 250 ms service tick, and
251/2 = 125.5 ms is a near-perfect match for the fitted fixed cost C ≈ 115–132 ms. I was one
step from reporting *"the completion plane has a 250 ms poll and that IS the fixed cost."*

**It is refuted by the guest's own latency distribution.** The 21 N=512 launches measured:

```
102.870 107.916 113.775 118.857 119.371 121.780 122.292 124.641 125.204 126.132 127.025
127.129 129.482 129.519 129.519 131.058 133.797 134.910 135.005 135.927 138.051
```

A continuous band 102.9–138.1 ms. If completion detection were quantised to a 251 ms tick
with random phase, these would spread across [0, 251] or cluster on multiples of it. They do
neither. ⇒ **The 251 ms is the device's own semaphore-page SCAN/REPORT cadence — the
observer's clock, not the plane's.** Same class as *"suspect the instrument first"* and *"a
probe that shares the allocator is not an observer"*, and the coincidence with C/2 is what
made it dangerous: it arrived pre-corroborated.

⇒ **The ~100 ms per-launch floor is measured and its MECHANISM IS UNATTRIBUTED.** Named
candidates, none established here: the doorbell trap path (283 GrCompute doorbells over ~90
launches ≈ 3 per launch ⇒ ~33 ms per doorbell if it is all there), the guest RM's per-launch
ioctls, or the isolate round trip. **The next rung should timestamp the doorbell path
directly rather than inferring it from a fit.**

---

## 7. WHAT THIS MEANS FOR AN LLM — arithmetic, not vibes

A 7B transformer at fp16: ~14 GB of weights; ~2·7e9 = 14 GFLOP per token; on the order of
**200–300 kernel launches per token** (~32 layers × 6–9 kernels).

| leg | at measured guest rates | verdict |
|---|---|---|
| load weights (14 GB @ 9.25 MiB/s) | **~26 min, once** | painful, survivable — it is a boot cost |
| process startup | ~27 s, once | irrelevant to steady state |
| arithmetic (14 GFLOP @ 27.5 GFLOP/s) | **0.51 s/token → ~2 tok/s** | the *optimistic* bound |
| **per-launch floor (250 × ~100 ms)** | **~25 s/token → ~0.04 tok/s** | ★★★ **THE BINDING CONSTRAINT** |

⇒ **60 tok/s means ~16 ms per token across ~250 launches — a budget of ~64 µs per launch. We
are at ~100 000 µs. That is a ~1 500× gap on the fixed cost alone**, before the 22×
proportional term is touched.

★ **The constructive reading, and it is real:** the dominant term is FIXED, so it is the kind
of cost that yields to engineering rather than to physics. Two independent things must both
move:

1. **The ~100 ms per-launch floor → sub-millisecond.** Worth the most, and it is the one with
   no known mechanism yet (§6). ⊘ Batching does not touch it.
2. **The ~22× proportional term.** Even with the floor at zero, N=2048 would sit at ratio
   ≈0.043. Fixing the floor ALONE does not reach (A).

⊘ Reaching ratio ≳0.5 at N=2048 needs guest ≈45 ms against today's 624 ms — a **~14×**
end-to-end improvement, from two separate sources. **That is the honest size of the LLM gap.**

---

## 8. CORRECTNESS — guarded on both arms, and the guard cost a second boot

`bad = 0` and `maxerr = 0` on **every timed iteration at every size**, `GUEST_XID_COUNT=0`,
criterion (E) VERDICT=0 (E1 ✔, E2 drain ran 877 rows Σpinned=31 541, E3 invariants ✔).

★ The verifier here has **no early-exit guard**, so `bad` IS a whole-matrix total and `maxerr`
IS a whole-matrix maximum — unlike `cup8.c`'s `bad<8`, whose numbers were partial and had to
be caveated in every report of them.

★★★ **And `bad=0` is licensed by a KNOWN-POSITIVE on both arms.** `BENCH_NOLAUNCH=1` skips
every launch and changes nothing else — the poison fill, the readback and the verify all still
run — so it MUST report `bad = N²`:

- native: `262144` (= 4 × 65 536 at N=256), `maxerr` 6.26e18 (the 0xDEADBEEF poison)
- guest: **`262144`**, rc=0, wall 33 s

⇒ the poison fill and the readback are **live on our plane**, so a launch that did nothing
could not have passed by leaving the previous iteration's correct answer in C.

⊘ **This took a second boot, and the reason is itself a finding.** In the measurement boot the
control ran second, as a SECOND CUDA context, and hung at `cuCtxCreate` while the first
process's teardown was still emitting `STOP_CHANNEL` / `GPU_EVICT_CTX`
`NV_ERR_NOT_SUPPORTED` — the known #12 shape. It reached its own 180 s bound and wrote
`rc=124`. ⚠ **That is the INNER `timeout` firing on a job that was genuinely stuck — not the
124 that means a launcher expired while the job ran on fine, and not the 143 that means
something killed it.** The rc file plus the missing verdict line is exactly what makes those
three distinguishable. Arm `KAYFABE_BENCH_ONLY=negctrl` reruns the control as the first and
only context; it then completes in 33 s. ⊘ It had to be a separate boot, not a reordering:
running the control first would have made the MEASUREMENT the second context.

### 8.1 Two limits on the correctness claim, stated

- ⊘ **The host-side Xid check is vacuous on this run.** `run_w311bench_hostdmesg.log` is
  **0 bytes**, and (E1) passes by reading an empty file — it says so ("delta file exists, 0
  bytes"). The measured Xid=0 is the **guest-side** one (`dmesg | grep -c Xid`). w308 has the
  same 0-byte host delta. An empty artefact reads as benign; only its content distinguishes
  *nothing happened* from *nothing was recorded*.
- ⊘ `GUEST_BENCH_DEVICE=[]` — `cuDeviceGetName` returned **SUCCESS with an empty string** in
  the guest. This independently reproduces the known `GPU_GET_NAME_STRING` defect (right size,
  zero bytes) that makes `nvidia-smi` print `ERR!` in the Name column. Reported, not fixed
  here.

---

## 9. WHAT THE BRIEF GOT RIGHT AND WRONG

- ★ **RIGHT, and load-bearing:** *"An LLM is not one matmul. It is thousands of launches over
  memory that is already resident."* The per-launch fixed cost is exactly the dominant term.
  A rung that had measured only one big matmul would have reported ratio 0.036 and missed
  that the binding constraint is a per-launch constant.
- ⊘ **WRONG:** (D) and (E) were posed as alternatives. Both are true simultaneously (§2).
- ⊘ **WRONG in emphasis:** *"first-launch cost carries publication, report it separately."*
  It barely matters — guest first-launch is 127/295/700 ms against medians of 126/198/624 ms.
  Publication is real but it is amortised across the *process*, not paid at the first launch
  (§5). The startup cost that actually deserved separating was `cuInit` + `cuCtxCreate` (§4),
  which the brief did not name.
- ⊘ **UNDERSPECIFIED:** *"≥2 sizes."* Two sizes fit a two-parameter model exactly and leave
  no residual. Three were needed to show the fit is *consistent* (§2).

---

## 10. Reproducing

```
# native reference (on the bench host, refuses to run if a QEMU is up)
scripts/bench/w311_native.sh

# guest arm — one boot, measurement + control
scripts/bench/w311_run.sh

# guest arm — control only, as the first and only context
KAYFABE_TAG=w311neg KAYFABE_BENCH_ONLY=negctrl scripts/bench/w311_run.sh

# the deliverable, from the two logs; exits 2 rather than inventing a reference
scripts/bench/w311_ratio.sh <guest_probe.log> <native.log>
```

Knobs: `BENCH_SIZES` `BENCH_ITERS` `BENCH_BATCH` `BENCH_VERIFY` `BENCH_NOLAUNCH`
(and `KAYFABE_BENCH_*` for the guest hook).
