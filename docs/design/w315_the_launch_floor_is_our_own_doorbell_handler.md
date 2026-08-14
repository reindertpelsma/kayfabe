# w315 — THE ~100 ms PER-LAUNCH FLOOR IS OUR OWN DOORBELL HANDLER, AND 91 % OF IT IS THE PAGE-TABLE PASS

**STATUS: LIVE — measured 2026-08-14 on bench `vh2` (RTX 3060 GA106, driver 580.159.04),
five guest boots, source revision `9196b8fa` (stamp gate passed on every boot; the `base`,
`census`, `full` and `inject` arms ran at `72b6f66f`, the confirmation `full2` at
`9196b8fa`).** Artifacts: `traces/w315_floor/`.
⊘ `vh2` is itself a KVM guest — our guest is at **L2** and every MMIO access is a nested
vmexit. §4 measures what that costs and it is **not** the answer.

> ★★★★★ **PRE-REGISTERED OUTCOME: (A) — ONE SEGMENT DOMINATES.**
> `vas_publish` is **48.3 ms of a 113.6 ms launch (42.5 %)**, and its family — the whole-VAS
> publication plus the page-table decode/sweep/census passes we run **on every doorbell** —
> is **79.4 ms per launch (69.9 %)**.
> The **real host RM forward is 3.6 ms (3.2 %)**. Our own logging is **0.2 ms (0.2 %)**.
> ⊘ **95.6 % of the trap is `shape=work`** — it costs the same on bare metal.

---

## 0. The one-line answer

**The guest's `cuLaunchKernel` does not return until our MMIO doorbell handler does, and our
handler spends 91.5 % of its time re-deriving page tables that it then publishes nothing
from.** The vCPU is halted inside a single trap for **86.7 ms of the 90.9 ms `cuLaunchKernel`
takes**; the remaining ~2.2 ms is the vmexit plus the guest driver's own work.

| | ms per launch | share of the launch |
|---|---|---|
| guest `launch_ms` (median, N=512) | **113.6** | 100 % |
| ├─ `cuLaunchKernel` (SUBMIT) | 90.9 | 80.0 % |
| │  ├─ **inside our doorbell MMIO trap** | **86.7** | **76.3 %** |
| │  └─ vmexit + guest driver (outside our handler) | 2.2 | 2.0 % |
| └─ `cuCtxSynchronize` (COMPLETION) | 23.3 | 20.5 % |

---

## 1. What was measured, and with which clock

Two instruments, on two clocks, and **no offset between them is assumed anywhere**.

- **Guest**, `scripts/bench/cup8bench.c`: `clock_gettime(CLOCK_MONOTONIC)` around
  `cuLaunchKernel` and `cuCtxSynchronize` **separately**. That split needs no correspondence
  at all — both halves are the guest's own clock — and it is what first localised the floor to
  the **submit** side.
- **Host**, `crates/kayfabe-qemu-raw/src/kftime.rs`: `Instant` brackets on the vCPU thread,
  inside the MMIO trap, at every phase boundary of `Regs::write` and `SharedDoorbell::ring`.

The correspondence is **derived, not assumed** (`scripts/bench/w315_align.py`): the offset δ
between the two clocks is the value that minimises `RMS(host_trap_duration − guest_submit_ms)`
over the twelve launches. At the best δ:

```
    i    host_trap    g_submit    g_launch       resid
    0       86.092      88.471     111.741      -2.379
    1       72.178      73.890      97.025      -1.712
    2       93.067      95.758     119.186      -2.691
    ...                                          ...
   11       92.402      94.276     117.846      -1.874

   best RMS = 2.269 ms, 39.3× better than the median offset; 12/12 windows matched
   Σ host trap 1041.5 ms  vs  Σ g_submit 1068.2 ms   ⇒ 97.5 %
   residual (submit − trap) = 2.230 ms per launch, sd 0.421
```

★ The residual is **systematically negative and tight** (−1.7 to −3.3 ms, sd 0.42). A random
alignment does not produce a constant offset across twelve independent durations; this one is
a measurement, and the residual is itself a quantity (see §4).

⊘ **What licenses the containment claim is not the clock, it is the vmexit.** A guest MMIO
write halts the vCPU for the whole trap, so a host interval measured here is *inside* the
guest's launch window by construction. δ is only needed to say *which* trap belongs to
*which* launch.

---

## 2. ⊘⊘ THE ESTIMATOR THAT LIED — and it lied exactly the way w311's did

The first version of the alignment maximised **total host trap time overlapping the launch
windows**. It reported `δ = −29 601 ms` and *"99.7 % of the launch window is inside a doorbell
trap"*, on a **5 ms-wide plateau of identical maxima**.

**It is degenerate.** One doorbell in that boot lasted **1 991 ms**. Sliding the guest's 3.2 s
of windows anywhere underneath it credits *every* window in full, so the score saturates at
exactly `Σ launch_ms` — the arithmetic ceiling — for any δ in a wide band.

⇒ **The tell was that the score equalled its own theoretical maximum.** That is the same
species as w311's `251/2 = 125.5 ms ≈ C`: an answer that **arrives pre-corroborated**, whose
agreement is a property of how it was computed rather than of the world.

The replacement credits, per launch window, **only the longest trap whose START falls inside
it**, and scores by shape (RMS against twelve `submit_ms` values). A blanket trap starts in at
most one window, so it cannot saturate. And it **refuses** when the minimum is shallow
(< 5× better than the median offset), which is pre-registered outcome (E).

⚠ Both estimators are in `w315_align.py`, the dead one in the docstring, because a replaced
estimator that leaves no trace is one the next reader re-invents.

---

## 3. ★★★★★ THE BREAKDOWN — of the twelve launch doorbells only

⊘ **Not the whole-boot census.** That one is dominated by `cuInit`/`cuCtxCreate` traffic and
by two multi-second outliers (a 1 991 ms doorbell, a 2 070 ms `reap`, a 1 671 ms
`materialize`). These twelve are the doorbells the *timed launches* rang.

```
Σ trap = 1040.791 ms over 12 launches = 86.733 ms/launch

segment          shape   total ms   ms/launch   median ms    share
vas_publish      work     579.447      48.287      48.894    55.7%
pt_decode        work     267.384      22.282      23.403    25.7%
pt_sweep         work      79.611       6.634       6.644     7.6%
core             host      42.944       3.579       3.628     4.1%
ringproj         work      37.023       3.085       2.710     3.6%
pt_vascensus     work      25.352       2.113       2.058     2.4%
ce_try           work       3.222       0.269       0.206     0.3%
log_vas_publish  log        1.699       0.142       0.025     0.2%
bindcensus       work       0.923       0.077       0.076     0.1%
operand_join     work       0.900       0.075       0.067     0.1%
log_ptdecode     log        0.765       0.064       0.062     0.1%
pt_witness       work       0.705       0.059       0.055     0.1%
err_notifier     work       0.513       0.043       0.039     0.0%
pin_ring         work       0.068       0.006       0.006     0.0%
UNMARKED         -          0.112       0.009           -     0.0%
NESTED core_rm_ipc host     40.833       3.403           -     3.9%  ⊘ INSIDE `core`

ROLL-UP
  page-table + publication         79.375 ms/launch   91.5%
  ring projection / probes          3.555 ms/launch    4.1%
  THE ACTUAL HOST FORWARD           3.579 ms/launch    4.1%
  our own logging (instrument)      0.215 ms/launch    0.2%

  shape=work   82.930 ms/launch  95.6%   costs the SAME on bare metal
  shape=host    3.579 ms/launch   4.1%   blocked in the host RM / the isolate child
  shape=log     0.215 ms/launch   0.2%   the instrument's own printing
```

★★ **THE SECOND BOOT, side by side** (`full2`, 12 launches, 93.491 ms/launch):

| segment | `full` ms/launch | `full` share | `full2` ms/launch | `full2` share |
|---|---|---|---|---|
| `vas_publish` | 48.287 | 55.7 % | **52.175** | **55.8 %** |
| `pt_decode` | 22.282 | 25.7 % | 22.251 | 23.8 % |
| `pt_sweep` | 6.634 | 7.6 % | 8.569 | 9.2 % |
| `pt_vascensus` | 2.113 | 2.4 % | 3.026 | 3.2 % |
| `core` (host RM) | 3.579 | 4.1 % | 4.502 | 4.8 % |
| `ringproj` | 3.085 | 3.6 % | 2.350 | 2.5 % |
| **page-table family** | **79.375** | **91.5 %** | **86.075** | **92.1 %** |
| `shape=work` | 82.930 | 95.6 % | 88.859 | 95.0 % |
| `shape=log` | 0.215 | 0.2 % | 0.120 | 0.1 % |
| `UNMARKED` | 0.009 | 0.0 % | 0.010 | 0.0 % |

⇒ **Every share reproduces to within ~1.6 points.** `vas_publish` is 55.7 / 55.8 %.

★★★ **`UNMARKED` inside the bracket is 9 µs of 86 733 µs — 0.01 %.** The breakdown closes. The
pre-registered (D) — *the breakdown does not sum, and the missing time is the finding* — is
**not** the outcome here; the only unattributed slice is the 2.2 ms *outside* the trap, and it
is named in §4 rather than distributed.

### 3.1 The forward is 4 % of the doorbell

`core` — plan, worker checkout, the real host RM verbs, the ring fetch and the pushbuffer
decode — is **3.579 ms**, of which **3.403 ms is the blocking IPC round trip to the isolate
child** (`core_rm_ipc`, 2 calls per doorbell). ⇒ **the work we exist to do costs 4 % of the
time we take doing it.**

### 3.2 ⊘ The 22 ms decode binds nothing — and the C already paid for this lesson

`pt_decode` costs **22.3 ms per launch**, and the line it prints on those very doorbells reads:

```
PT-DECODE … drained=162 latched=52 … rounds=1 → bound=0 unchanged=0 repointed=0 unbound=0
  learned=0 published=0/0 … refusals=1592 …
```

**`bound=0 … published=0/0`.** This independently reproduces the sibling lane w316's census
(`bound=0` on 4 373 decode emissions against 893 `bound=4`) *from the timing side*, and it is
the shape the C artifact measured and fixed: 99.97 % of its walks backed nothing, and
dirty-gating them bought **200×** (0.1 → 20.1 tok/s).

⊘ **I did not steer to this.** The coordinator flagged w316 mid-rung and said explicitly to
believe the measurement over the corroboration. The measurement is independent — a stopwatch
on a segment, not a count of emissions — and it agrees.

---

## 4. ⊘⊘ THE BENCH IS NESTED, AND IT IS **NOT** THE ANSWER — measured, not argued

`vh2` is itself a KVM guest (`systemd-detect-virt` → `kvm`, `hypervisor` in `/proc/cpuinfo`,
nested KVM present, Xeon W-2133), so our guest runs at **L2** and every MMIO access takes a
nested vmexit. The C attributes a **2.5× llama.cpp gap** to exactly this
(`C: docs/MILESTONES.md:12-14`, 49.9 tok/s bare metal vs ~20 nested).

⊘ **No segment can contain that cost** — the exit is over before any hook runs. It can appear
only as the *outside-the-trap* residual, and that residual is measured:

**2.230 ms per launch, sd 0.421 — 2.0 % of the 113.6 ms launch.**

⚠ That residual also contains the guest driver's own work around the store, so 2.2 ms is an
**upper bound** on the nested-virt tax per launch, not the tax. Either way:

⇒ **The coordinator's arithmetic was right and is now measured: nested virt is not the floor.
Moving to bare metal would recover at most ~2 % of this launch.** The fix target is
`shape=work` and travels with us.

Supporting counts, from the 1 Hz KVM sampler (`run_<tag>_kvmexits.log`) and our own census:

| | whole boot |
|---|---|
| all VM exits | 1 681 577 |
| `mmio_exits` | 277 894 (peak 27 434/s) |
| our device's MMIO **reads** | 12 000, **20.5 ms total** (mean 1 µs) |
| our device's MMIO **non-doorbell writes** | 416 800, 4 854 ms total — **of which 4 031 ms is two single outlier events** |
| our device's MMIO **doorbells** | ~529, **14 968 ms total** (mean 28.3 ms, median 20.1 ms, max 1 992 ms) |

★ **The doorbell is ~95 % of all the time we spend inside MMIO handlers**, and reads — the
thing a nested bench punishes hardest — are **20 ms across an entire boot**. A polling guest
was a live hypothesis and it is refuted.

⊘ The KVM counters' first delta read **`exits=0`**: `/sys/kernel/debug/kvm/*` is per-live-VM
and resets when the VM exits, so last-minus-first is zero on any *completed* boot — a number
that looks like a measurement and is an artefact of the subject being gone. Fixed by summing
positive per-row deltas; recorded here because the wrong version printed `exits=0` beside a
real peak of 27 434 mmio_exits/s **in the same file**.

---

## 5. THE KNOWN-POSITIVE — watched attributing, and it did NOT pass cleanly

`KAYFABE_KFTIME_INJECT_US=30000 KAYFABE_KFTIME_INJECT_SEG=vas_publish`, one boot, one
variable, same binary.

| | census arm | inject arm | Δ | expected |
|---|---|---|---|---|
| `vas_publish` mean | 19 348 µs | **52 383 µs** | **+33 035** | +30 000 ✔ |
| guest `submit_med_ms` | 85.504 | **126.247** | **+40.743** | +30 × ~1.2 doorbells ✔ |
| guest `sync_med_ms` | 23.248 | 23.293 | +0.045 | 0 ✔ |
| `core` mean | 714 µs | 884 µs | +170 | 0 ✔ |
| **`pt_decode` mean** | **13 845 µs** | **22 870 µs** | **+9 025** | **0 ✘** |
| `pt_sweep` mean | 5 349 µs | 6 229 µs | +880 | 0 ✘ |
| `ringproj` mean | 1 290 µs | 2 176 µs | +886 | 0 ✘ |

★★ **Two of the three pre-registered conditions hold, and they are the load-bearing ones:**
the injected microseconds landed **in the injected segment**, and **the guest saw them** —
which is the part that proves the segment is on the guest's critical path at all. `sync_ms`
did not move, so the injection did not leak into the completion half.

⊘⊘ **The third condition — "no other segment moves" — FAILED.** `pt_decode` grew by 9 ms.

I do not think that is an instrument defect, and I am not certain it is not. The mechanism I
can name is that **the segments are not independent**: delaying the doorbell by 30 ms lets 30 ms
more guest work accumulate before the *next* doorbell, so the next `pt_decode` has more dirty
pages to walk. That is a real coupling in the plane, not a bracketing error. ⚠ But it is
**stated as a mechanism with one boot behind it, not a measured fact**, and a second candidate
— ordinary boot-to-boot variance — is not excluded by n=1.

⇒ **The instrument attributes. It does not isolate.** Any future use of an injection here must
re-check the innocent segments rather than assume them.

---

## 6. THE INSTRUMENT'S OWN COST

Same workload, same binary, one variable, four boots:

| arm | `KAYFABE_KFTIME` | guest `med_ms` | `submit_med` | per-event lines printed |
|---|---|---|---|---|
| `base` | unset | **111.985** | 88.408 | 0 (asserted: 0 `KFTIME` lines in the device log) |
| `census` | `census` | **108.931** | 85.504 | 0 |
| `full` | `on` | **113.562** | 90.865 | 416 969 |
| `full2` | `on` | **116.909** | 93.977 | (confirmation boot) |

⇒ **The instrument's cost is not resolvable above the run-to-run scatter.** The four medians
span 8.0 ms; the *unarmed* arm is not the fastest of them. Even the `full` arm — which writes
**416 969 lines** to the QEMU log from inside the BQL — is 1.6 ms (1.4 %) above the baseline,
and the per-segment accounting agrees: `shape=log` is **0.215 ms of 86.7 ms**.

⇒ **Pre-registered outcome (C) — the floor does not reproduce under instrumentation — is
REFUTED.** It reproduces at 108.9–113.6 ms across all arms, against w311's 126.1 ms at the same
size on a different revision.

---

## 7. What this means for the numbers w311 published

w311 fitted `guest = C + k·native` and got **C ≈ 115–132 ms** fixed per launch. This rung
measures **86.7 ms of that C inside one function**, `SharedDoorbell::ring`, on a **different
revision** (master with `PT_SWEEP`/`OPERAND_JOIN` restored by w313) where the whole launch is
**113.6 ms** rather than 126.1.

⊘ The two are not the same number and must not be quoted as one. What survives is the
structure: a **fixed, per-submit, work-shaped** cost, now located.

★ The arithmetic w311 did on the product metric is unchanged in form and improved in
tractability: 60 tok/s needs ~64 µs per launch. **Of today's 113.6 ms, 79.4 ms is a page-table
pass that publishes nothing** — and the C's precedent for exactly this shape is a 200×.
⊘ That is a precedent, not a projection; nothing here has measured what removing it costs or
buys, and **this rung deliberately fixed nothing**.

---

## 8. What the brief got wrong, and what I got wrong

- ⊘ **The brief's framing "timestamp the doorbell path" was right, and its worry that the
  floor might not be attributable to a segment was wrong.** It attributes almost perfectly:
  `UNMARKED` inside the bracket is 0.01 %, and 97.5 % of `submit_ms` is one trap.
- ⊘ **My own instrument was wrong first, in the brief's own named class.** The overlap-maximising
  aligner reported a confident `δ = −29 601 ms` and a 99.7 % figure that was the arithmetic
  ceiling. It was caught only because the score equalled its own maximum. **I built the
  known-positive the brief demanded, and the thing that actually saved the rung was noticing a
  number was too round.**
- ⊘ **My pre-registration for the known-positive was too strong.** *"NO other segment moved"*
  is not achievable in a plane whose segments are coupled through elapsed time; the honest
  criterion is *"the injected segment moves by the injected amount and the guest sees it"*,
  plus a **named** account of anything else that moved.
- ⊘ **`mmio_read` was an afterthought, added only after the coordinator's nested-virt note,
  and it turned out to matter — by being ~zero.** Without it I could not have refuted the
  polling-guest hypothesis, and a doorbell-only instrument would have reported a fast write
  path beside an unexplained remainder.
- ⚠ **`bad = 0` in these runs is UNGUARDED.** All five boots ran `KAYFABE_BENCH_ONLY=measure`;
  the `BENCH_NOLAUNCH` negative control was not run in this rung. w311 ran it (guest
  `262144`), but at a different revision. ⇒ correctness is inherited, not asserted here.
- ⚠ **One size (N=512), one guest, one physical GPU, five boots.** The per-launch breakdown
  has **n=12 launches × 2 boots**; the segment shares are stable across them, the absolute
  numbers less so.

## 9. Reproducing

```
scripts/bench/w315_floor.sh base      # instrument OFF — the baseline
scripts/bench/w315_floor.sh census    # the aggregate breakdown
scripts/bench/w315_floor.sh full      # per-event lines — needed for the alignment
scripts/bench/w315_floor.sh inject    # the known-positive

scripts/bench/w315_align.py     <probe.log> <qemu.log>   # THE DELIVERABLE
scripts/bench/w315_attribute.py <probe.log> <qemu.log>   # the aggregate view + the exit bound
```

Knobs: `KAYFABE_KFTIME` (`off`/`census`/`on`), `KAYFABE_KFTIME_INJECT_US`,
`KAYFABE_KFTIME_INJECT_SEG`, `KAYFABE_KFTIME_CENSUS_EVERY`; plus `KAYFABE_BENCH_*`.
