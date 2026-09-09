# ★★★★★ w394 — CUDA APPS PASS, and the BAR TRAP IS **NOT** THE PARITY GATE

**STATUS: LIVE**, 2026-09-09. Box `50376491` (RTX 3060, GA106, `580.159.04`), binary stamped
`kayfabe-rev:a9815c91` (= tree HEAD, the LLM-passing master rev). Everything below is measured
from `traces/w394_cuda_apps/`.

## ✅ LANE (1) "CUDA APPS WORK" IS A PASS

```
W394_OUTCOME=(P)  — all 4 VERIFYING probes byte-correct in the Mode-2 guest
vector_add_test      PASS: 1024-element vec_add
matmul_test          PASS: 1024x1024 fp32 matmul
big_memcpy_test      PASS: 4194304 bytes round-tripped exactly
mem_bandwidth_probe  correctness: OK (16777216 bytes verified)
```
Each verifies its own output against a CPU-computed expectation, so none of these can be
scored by a completion we wrote ourselves. The same binaries with the same arguments pass
natively on the same box in the same hour.

⊘ `gpu_bench` and `cuda_micro` are **timing only** — `gpu_bench` does not check one result
byte. They can never carry a correctness verdict here, and their absence below is not a
correctness gap.

## ★★★ THE OWNER'S HYPOTHESIS WAS TESTED AND THE MEASUREMENT REFUTED IT

Owner, 2026-09-09: *"Probably Lane (3) is needed to get Lane (2) to work"* — i.e. untrapped
BAR1/2 is a prerequisite for parity. It is a good hypothesis and the arithmetic fit it almost
exactly. **It is wrong, and one number kills it.**

The model: if a 16 MiB `cuMemcpyHtoD` reached the device through the trapped BAR1 window as
CPU stores, it would cost `16 MiB / 4 B = 4.2 M` accesses, each a VM exit at ~4 µs ⇒ **16.8 s**
per copy. Measured H2D was ~17 s per copy. A near-perfect fit.

**The falsifier, registered before the counters were read: that boot's BAR1 access total must
then be in the MILLIONS.** It is not:
```
BAR1 (translated): 0 reads / 5839 writes resolved through the GMMU, 0 REFUSED by name
BAR1 access log  : 16 of 5839 access(es) recorded in full
doorbells        : 2089 arrived, 2089 served, 0 REFUSED
trap-status      : nvkvm-bar1-window ... IO — TRAPPED, every guest access reaches this device
```
The window is genuinely trapped (so 5 839 is the **true total**, not a sampled one), the guest
moved **128 MB** of H2D traffic in this boot, and BAR1 saw **5 839 accesses**. The bulk data
never goes through BAR1 — it DMAs to guest RAM by GPA, which is what the hardware design says
should happen. ⇒ **The 16.8 s fit was a coincidence of magnitudes.**

⚠ This does **not** argue against untrapping BAR1/2 — the owner called that the intended
implementation and it stands on its own (`w393-bar-passthrough`). It argues only that
**parity does not wait on it**, and that sequencing lane (3) ahead of lane (2) on this
reasoning would have been sequencing on a coincidence.

★ Note the shape: the hypothesis was *quantitatively* excellent and *causally* empty. A model
that predicts the right number from the wrong mechanism is the most expensive kind, because
the fit itself reads as confirmation. The only thing that separated them was a counter that
the mechanism — and not the number — made a prediction about.

## ★★ WHERE THE PARITY GAP ACTUALLY IS — the triad splits it

| leg | guest | native | per-copy (16 MiB) | what it is |
|---|---|---|---|---|
| D2D | 0.05 GB/s | 2904.01 GB/s | ~335 ms | **per-SUBMIT floor.** The GPU itself needs 5.5 µs for this copy; ~335 ms is ≈4 doorbell traps at the known ~86 ms trap cost. Per-byte cost ≈ 0. |
| H2D | 0.00 GB/s | 10.83 GB/s | ~17 s | per-BYTE, and **not** BAR1 (above). Unattributed. |
| D2H | 0.00 GB/s | 6.89 GB/s | ~17 s | as H2D. |

D2D is a pure device-side copy with no CPU in the data path, and it is still ~58 000× down.
⇒ **A per-submit floor exists independently of any CPU window**, and it is the thing the
launch-floor work (`arm_the_gate_at_the_sink`, `the_launch_floor_is_our_own_doorbell_handler`)
has already characterised: 91.5 % of an 86.7 ms trap is page-table + publication.
⊘ The H2D/D2H per-byte cost is **measured and unattributed**. Do not attribute it to BAR1; the
counter above forbids it. It is the next thing to instrument.

⚠ These numbers are **for the arming of this boot** — `KAYFABE_VAS_PUBLISH=drain`,
`KAYFABE_PT_SWEEP=on`, the legacy/slow arming. Quoting them without the arming is quoting a
different experiment.

## ⊘⊘ THE TIMING ARM IS UNMEASURED, AND THE CAUSE IS A KNOWN PRE-EXISTING DEFECT

```
gpu_bench  → cuInit failed: 999      cuda_micro → cuInit(0) failed: 999
guest dmesg: NVRM: _kgspBootGspRm: unexpected WPR2 already up, cannot proceed with booting GSP
             NVRM: RmInitAdapter failed! (0x62:0x40:2028)
```
The suite opens the device **once per probe**. Probes 1–4 passed; probes **5 and 6** died. That
is w370's *"the 5th DEVICE-OPEN wedges the GPU"*, reproduced exactly — and reproduced by
accident, by a harness that had no idea it was a multi-open test.

⇒ **Consequence for the roadmap, and it is bigger than the timing numbers:** a real workload is
many processes. "CUDA apps work" is measured green for four apps **in four opens**; the fifth
open in a boot is a wall that no amount of per-app work will move. The multi-open wedge belongs
ahead of parity, not behind it.
⇒ **Consequence for the harness:** the timing arm must be **one probe per boot**. A suite that
shares a boot measures the wedge, not the probe.

## Reproduce
```
scripts/bench/w394_cuda_suite.sh native            # the denominator, host GPU direct
POST_CAPTURE_HOOK=scripts/bench/w394_hook.sh \
  bash scripts/bench/boot_capture.sh <tag>         # the numerator, fully armed
scripts/bench/w394_grade.sh <native.log> <guest probe log>
```
⚠ `scripts/build_qom_shim.sh` takes `<qemu-source-tree>` — calling it bare exits 1 and the boot
then silently uses whatever binary was already there. Check `W394_BINSTAMP` against the tree
HEAD before believing any run.

---

# ★★★★★ ADDENDUM — OWNER'S FOUR CORRECTIONS, ANSWERED FROM THE SAME BOOT
2026-09-09, same log. Owner: *"bar1/bar2 are used for some setup stuff, but I agree 58000x is
something about your locks"* · *"we never proved CPU cacheability worked even"* · *"I think its
a stack of multiple walls now"* · *"I hope its a real CE on the GPU, not a memcpy"*.

## ✅ IT IS A REAL CE. No user-proc byte was moved by our CPU.
```
CE-SERVED-LOCAL (cpu-ce):  278 records — ALL proc=0
OPERAND-SOURCE-CE:         proc=2:229  proc=3:238  proc=4:229  proc=5:386   (1082 total)
```
`proc=0` is the SYSTEM proc, whose CeUtils scrub the device's own log calls *"FORGED, never
forwarded"* — that is the documented, intended local path (`l1_concurrency.md` §12.26). The four
**user** procs are exactly the four probes that ran, and every one of them appears only on the
decode-and-forward path. ⇒ **Zero user-proc copies served locally.**
★ The inference closes because the bytes are VERIFIED: `big_memcpy_test` round-tripped 4 MiB
byte-exact and `mem_bandwidth_probe` verified 16 MiB. Correct bytes + no CPU copy ⇒ the host
engine moved them. Neither half alone would do: a CPU copy also produces correct bytes, and a
zero-local-serve count alone would be consistent with nothing having happened.
⊘ This is NOT the C artifact's posture. There, `m2cexec` was OFF and the data plane was CPU +
emulator; its 20→60 tok/s were CPU-copy numbers. This port forwards user CE work for real.

## ★★★★★ THE LOCKS — the owner's read, with the number
```
TRAPWITNESS  off_trap_claims=0  inline_exceptions=46568  worst_trap=2666348us
             (target: inline_exceptions=0)
```
**A single trap held for 2.67 SECONDS**, and 46 568 units of work ran INLINE ON THE TRAP THREAD
against a stated target of zero. D2D — a device-side copy with no CPU in its data path and no
BAR1 access — costs ~335 ms per 16 MiB. That is this, not memory.
⇒ The per-submit floor is a **scheduling/lock** defect, and `inline_exceptions` is its named,
already-instrumented metric with an explicit target. Drive it to 0.

## ★★★★ CACHEABILITY IS NOT UNPROVEN — IT IS UNREACHABLE, AND THIS IS THE REAL CASE FOR LANE (3)
```
trap-status nvkvm-bar1-window: base 0xe0000000000 -> region 'nvkvm-bar1-window' (OURS),
            IO — TRAPPED, every guest access reaches this device
```
A QEMU **IO** region has no cacheability *at all*: there is no WB, no WC, no combining — every
access is an exit by construction. So the guest CPU has never had a cacheable or write-combined
view of BAR1, and no measurement of one exists because none was possible.
★★ **This is a stronger and completely independent argument for untrapping BAR1/2 than the
parity one refuted above.** `DEVICE_LOCAL | HOST_VISIBLE` is not merely "fewer exits" — it is
the only shape in which cacheability/WC is expressible. ⇒ lane (3) keeps its priority; what it
loses is only the claim that lane (2) *waits* on it.
⊘ And the owner's *"bar1/bar2 are used for some setup stuff"* is exactly what the counter shows:
5 839 writes / 0 reads is a **setup** profile, not a data-plane one.

## ⇒ IT IS A STACK. Four walls, separately instrumented, none subsuming another.
| # | wall | metric that names it | status |
|---|---|---|---|
| 1 | inline-exception / lock floor | `inline_exceptions=46568`, `worst_trap=2.67s` | measured, target stated (0) |
| 2 | H2D/D2H per-byte cost | ~17 s per 16 MiB, NOT BAR1 (5 839 accesses) | measured, **UNATTRIBUTED** |
| 3 | 5th-device-open wedge | `cuInit 999`, `WPR2 already up` | reproduced, w370 |
| 4 | BAR cacheability | region kind `IO` | unreachable by construction |
⚠ Stop hunting a single cause. Wall 1 explains D2D and cannot explain the H2D/D2H excess over
it; wall 4 cannot explain either, because the bytes do not go through BAR1.

---

# ★★★★★ LANE (2) PARITY — MEASURED, AND IT IS **ONE** TERM: THE LAUNCH
2026-09-09, box `50376491`, one probe per boot (`W394_ONLY`), fully armed, legacy arming
(`KAYFABE_VAS_PUBLISH=drain`, `KAYFABE_PT_SWEEP=on`). Both probes ran to completion — the
single-probe-per-boot change removed the 5th-device-open wedge exactly as predicted.

| metric | guest | native | ratio |
|---|---|---|---|
| `rm_control` (`cuMemGetInfo`) | 506.28 µs | 6.97 µs | **72.6×** |
| `alloc+free` (gpu_bench) | 7 700.16 µs | 109.70 µs | **70.2×** |
| `alloc+free` (cuda_micro) | 7 736.56 µs | 117.69 µs | **65.7×** |
| **`launch RTT`** | **342 242.60 µs** | 6.03 µs | **56 757×** |
| **`launch+sync`** | **139 759.64 µs** | 5.91 µs | **23 648×** |
| `uvm_alloc` (managed+touch) | 231.80 µs | 215.59 µs | **1.075×** |
| GEMM throughput | 6.0 GFLOP/s | 426.2 GFLOP/s | 0.014× |

## ★★★★★ THE GEMM NUMBER IS NOT A COMPUTE DEFICIT. IT IS 5 LAUNCHES.
```
GEMM wall 1.789 s;  5 launches × 342.24 ms = 1.711 s = 95.7 % OF IT
residual (everything that is not launch overhead) = 0.078 s ⇒ ~138 GFLOP/s
```
⇒ **0.014× reads as "the GPU is 71× slower at arithmetic". It is not.** 95.7 % of that wall is
our own launch cost, and the arithmetic underneath is running at ~138 GFLOP/s against a native
426.2 — same order, on a residual that is a small difference of two large numbers and therefore
imprecise. ⚠ Quote the 0.014× without this decomposition and you have described a compute
problem that does not exist.

## ★★★ THE TABLE HAS EXACTLY TWO CLUSTERS, AND THEY ARE THREE ORDERS APART
- **~66–73×** — `rm_control`, both `alloc+free`. A flat control-plane tax on forwarded ioctls.
- **~24 000–57 000×** — `launch RTT`, `launch+sync`. The submit path.
⇒ These are not one phenomenon with a spread; they are two mechanisms. Fixing the control-plane
tax buys ~70× on control ops and **nothing** on the thing that dominates every real workload.

## ★ THE NEGATIVE CONTROL, and it is load-bearing: `uvm_alloc` IS AT PARITY (1.075×)
A path exists, in this same boot, that costs the same as native. ⇒ The 70× is **not** an
architectural "everything through the emulated device is slow" tax, and it is not the guest, or
QEMU, or the box.
⊘ **Scope it honestly**: `cuMemAllocManaged`+touch+free most likely never reaches our device at
all, so this measures *"a path that does not reach us is native"* — which is trivially true and
still useful, because it **eliminates the whole-stack explanations** rather than confirming
ours can be fast.

## ⊘ TWO ANOMALIES, RECORDED AND NOT EXPLAINED AWAY
1. **An unsynced launch costs MORE than a synced one** — `launch RTT` 342 ms vs `launch+sync`
   140 ms, 2.4× the wrong way round. An async launch should be the cheap one. Recorded as an
   observation; I have no mechanism for it and will not invent one.
2. **`cuLaunchKernel … failed: 719`** (`CUDA_ERROR_LAUNCH_FAILED`) in `cuda_micro` subtest 6
   (`uvm_migrate`), after `uvm_alloc` passed, plus **1 host Xid** in that boot. That is lane (4)
   territory and is not on the parity path, but it is a real defect and it is written down.
⊘ Bandwidth printed `0.0 GB/s` again at 16 MiB — and `DtoH correctness: OK (0/4194304
mismatched)` again alongside it. Correct bytes, unusable rate: the house failure shape.

## ⇒ WHAT THIS SETTLES FOR THE ROADMAP
**Parity is one term, and it is the launch.** `inline_exceptions=46568` with
`worst_trap=2 666 348 µs` is the already-named metric for it, with a stated target of **0**, and
342 ms/launch is ~4 doorbell traps at the known ~86 ms cost. Nothing about memory, BAR, or the
CE plane is on the critical path for parity — all three were measured and all three are
elsewhere.

---

# ★★★★★ THE LLM AGREES WITH THE MICROBENCHMARK — 190 LAUNCHES PER TOKEN
2026-09-09, boot `w394c`, binary stamped `c3e31ce6` (= tree HEAD, verified). Arming as above.

```
W392_OUTCOME=(P)   GPU text BYTE-IDENTICAL to the same-boot CPU oracle   (n=2 for correctness)
W392_GPU_MS=1042577.8   W392_GPU_TOKS_PER_S=0.015     ⇒ 65 161 ms PER TOKEN
W392_CPU_MS=1588.9      W392_CPU_TOKS_PER_S=10.070    ⊘ correctness oracle, NOT a perf baseline
W392_XIDS=4/4/4         ⇒ no NEW Xid during the run
```

## ★★★ THE CONFIRMATION, and it is independent
```
65 161 ms/token ÷ 342.24 ms/launch  =  190 launches per token
```
A 24-layer 0.5 B transformer's forward pass is ~190–200 kernel launches. **A workload with
nothing in common with `gpu_bench` — different binary, different framework, different kernels —
lands on the same constant.** The launch floor is not a microbenchmark artefact; it is the
whole cost model.
⇒ Everything this campaign has measured today reduces to one number: **342 ms per launch.**
GEMM (95.7 % launch), the LLM (190 launches/token), `launch+sync` directly. Fix that and every
figure moves together; fix anything else and none of them do.

## ⊘ WHAT THIS NUMBER IS AND IS NOT
- It is **decode throughput** — `generate()` only. It **excludes** model load and the
  `.to(device)` weight upload, which is where w394 measured H2D at ~17 s per 16 MiB. The
  end-to-end figure is therefore **worse** than 0.015 tok/s, not better.
- ⊘ The CPU arm's 10.07 tok/s is **not** a baseline to beat: it is the correctness oracle, and
  `W392_TPS_RATIO_GPU_OVER_CPU=0.001` is printed only so nobody derives it by hand and calls it
  parity. The parity denominator is a NATIVE GPU run, which this boot does not contain.
- ⚠ **For this arming** (`KAYFABE_VAS_PUBLISH=drain`, `KAYFABE_PT_SWEEP=on`).

## ⊘ AND FOR SCALE, AGAINST THE C ARTIFACT
The C's 20→60 tok/s figures are **CPU-copy** numbers from a run with `m2cexec` OFF — not a
forwarding baseline, as `CLAUDE.md` says in the same breath. They are not this number's
competitor and must not be quoted beside it as though they were.

---

# ★★★★★ WALL 1, ATTRIBUTED — AND `off_trap_claims=0` IS THE HEADLINE
Boot `w394e`/`w394d`, binary stamped `9fed06ab`, **verified by content** (`HAS_ATTRIBUTION=1`),
not by stamp. Client re-passed on this build: `THREADS 4 of 4`, `MEAN_FALSIFIER=PASS`.

```
TRAPWITNESS off_trap_claims=0  inline_exceptions=166  worst_trap=1771955us
  INLINE-BY-REASON [134 × kayfabe_rt::SharedDevice::verb_op — the execute phase]
                   [ 32 × kayfabe_fwd::dispose_on — the revocation chain (NOT deferrable)]
PUBQUEUE coalesce=false queued=0 coalesced=0 refused=0 taken=0 completed=0 depth=0 cap=4096
```

## ★★★★★ `off_trap_claims=0` — THE DEFERRAL MACHINERY HAS NEVER ONCE FIRED
`at_a_host_verb` takes the honest branch: `claim` off-trap, `inline_under_bql` on a trap. The
census says **every host verb in the boot took the inline branch, and none took the other one.**
The `PUBQUEUE` is idle in every column — `queued=0 taken=0 completed=0 depth=0` against
`cap=4096`.
⇒ This is not *"deferral is tuned badly"*. It is *"deferral is built, wired, and has never
executed"* — the [[a_deferral_is_a_claim_about_consequences]] shape again, and the same shape
this tree recorded for the completion plane (23 producers on the fill side, **0** on the drain
side). ⚠ A capability with a `cap=4096` and a zero in every counter reads as *configured*; it
is *unexercised*.

## ★ THE RANKING, AND ITS SCOPE
`verb_op` is **81 %** of the mints; `dispose_on` is 19 %; `Proc::drop` and `fwd::round_trip`
fired **zero** times. ⇒ Two of four sites are dead weight in this workload and the fix has one
obvious first target.
⊘ **SCOPE, and it matters**: this is the *mean client* — 198 doorbells, 166 mints ≈ **0.84 per
doorbell**. The boot that produced `inline_exceptions=46568` had 2 089 doorbells ≈ **22.3 per
doorbell**, a 27× different regime. A ranking measured at one scale is a hypothesis at the
other, so `w394e` re-measures it under `gpu_bench` — the workload that actually carries the
342 ms launch.
⚠ Do not quote the 81 % as the CUDA-workload split until that lands.

## ⇒ THE FIX IS NAMED
Make `SharedDevice::verb_op`'s execute phase take the `claim` branch — i.e. run the host verbs
on the publication worker instead of on the vCPU inside its own MMIO exit. The pubqueue that
would carry them already exists and is empty.
⊘ **NOT the async-doorbell lane** ([[the_llm_passes_and_the_cause_was_an_unreachable_default]]:
default-off and UNSAFE). This is the *execute* phase moving off the trap thread, with the
doorbell's observable ordering unchanged — a different change with a different safety argument,
and that argument has to be made explicitly before any of it is armed.

---

# ★★★★★ THE OWNER WAS RIGHT: WE HOLD THE DOORBELL WRITE FOR MILLISECONDS
Owner, 2026-09-09: *"are you sure you are not holding a doorbell write for milliseconds"*.
**No — and the instrument had already said so, in a field I had read past twice.**

```
worst_trap = 1 927 527 us   ⇒ ONE guest MMIO register write held for 1.93 SECONDS
mean       =     234 ms     per launch (the doorbell write not returning)
```
`TrapGuard::enter()` wraps `regs.write(bar, off, size, val)` (`shim_unsafe.rs:1378`) — it is
the **guest register write itself**, not some enclosing scope. The comment beside it already
cites w317 measuring a **3.70 s** hold.

## ⊘ AND THE TREE HAD ALREADY RULED ON IT, TWICE
- `GuestChannelKind::Emulated` declares `TrapContract::ScheduleAndReturn`, whose doc says
  *"the handler must not run on the vCPU thread"*, with `may_run_on_the_vcpu_thread()` as the
  predicate.
- **Owner ruling, 2026-09-06, quoted in our own source**: *"never do a blocking call during a
  doorbell write."*
- [`DOORBELL_ASYNC_ENV`] (`KAYFABE_DOORBELL_ASYNC`) exists for exactly this and **defaults to
  `off`**; its own comment says the contract was until then *"read and reported as violated in
  the same breath."*
⇒ **Every measurement in this document — 342 ms/launch, 190 launches/token, the 1.9 s trap —
was taken in the configuration that violates that ruling.** This is not a newly discovered
wall. It is a wall we documented, built the fix for, defaulted to off, and then measured
around. That is why `PUBQUEUE` read zero in every column: nothing was ever queued because the
lane was never armed.

## ★★★★★ AND `VERBCOST` OVERTURNS MY OWN COST MODEL — IT IS NOT THE HOST ROUND TRIP
```
VERBCOST total=1429942us over 4309 plan(s)
  [PinGuestRam n=3676 mean=0.20ms 50.4%] [JoinFbLeaf n=79 mean=3.27ms 18.0%]
  [EngineObject n=32 mean=7.97ms 17.8%] [Doorbell n=493 mean=0.21ms 7.2%]
  [Release n=5 mean=12.66ms 4.4%] [ChannelBirth n=16 mean=1.00ms 1.1%]
  [AliasFbLeaf n=8 mean=1.59ms 0.9%]
```
| per launch | ms | share |
|---|---|---|
| wall | 234.0 | 100 % |
| **host verbs (measured)** | **7.15** | **3.1 %** |
| **our own trap-side work** | **226.8** | **96.9 %** |

⊘ **My "11.98 ms per verb plan" was wrong**, and wrong in an instructive way: I computed it as
*wall ÷ mint count*, which silently assumes the verbs are the wall. Measured directly, they are
**3 %**. A ratio built from two numbers that were never the same quantity will look like an
attribution and be an artefact.
★ This **reproduces w315 exactly** — *"91.5 % of an 86.7 ms trap is page-table + publication,
the real host forward 4 %"* — on a different workload, a different instrument and a year of
intervening work. The cost was never the host; it is our own page-table decode and publication,
run inline on the vCPU inside its own MMIO exit.
⇒ Which is **precisely what `KAYFABE_DOORBELL_ASYNC=on` moves off the vCPU**: *"the trap
validates, offers the token to the coalescing publication lane and returns; a worker thread
runs the identical body in the identical order."*

## ⏭ THE EXPERIMENT, PRE-REGISTERED (`w394g`)
Correctness **first**, because an unsafe reordering shows up in content verification and never
in a timing number:
1. `KAYFABE_DOORBELL_ASYNC=on` + the mean client ⇒ must still print `W392D_OUTCOME=(P)`,
   `THREADS 4 of 4`, `MEAN_FALSIFIER=PASS`.
2. Same arm + `gpu_bench` ⇒ `launch RTT` must fall from ~234 ms, `off_trap_claims` must become
   **> 0**, and `PUBQUEUE` must stop reading zero.
⊘ If (1) goes red the arm is unsafe as built and the perf number is irrelevant — report the
red, do not quote the speed. ⊘ If `off_trap_claims` stays 0 the arm did not engage and any
timing change is something else; `nocoalesce` is the negative control for the coalescing half.

---

# ★★★★★ THE WORST MMIO TRAP IS **`NV_PGSP_QUEUE_HEAD(0)`** — THE GSP RPC SUBMIT REGISTER
Boot `w394h`, stamp `f365d831`, content-verified. **The env was deliberately unset**, so this
also verifies the flipped default: `off_trap_claims=11491` proves the async lane armed itself.

```
TRAPWITNESS off_trap_claims=11491 inline_exceptions=51
            worst_trap=1791581us at=bar0+0x110c00  slow_traps(>1000us)=380
PUBQUEUE coalesce=true queued=526 taken=526 completed=525 depth=0 high_water=15
```

`bar0+0x110c00` is **`NV_PGSP_QUEUE_HEAD(0)`** (`kayfabe-device/src/ga10x.rs:94`). The write
handler routes to `BootStep::CommandDoorbell` → `self.doorbell(ram, policy)`
(`kayfabe-gsp/src/boot.rs:945`), which **drains and services the GSP command queue inline** —
inside the guest's MMIO store.

★★★ **This is the owner's own example, arrived at from the other end.** Owner, 2026-09-09:
*"real gpu also never holds any mmio write for milliseconds right. **like rpc mmio starts the
operation, the block is for example a semaphore**"*. `QUEUE_HEAD` is exactly an RPC submit: on
hardware it posts the message and returns, and the driver waits on the response queue. We
execute the whole RPC in the store.
⇒ **The worst trap was never the channel doorbell.** All of today's launch-floor work was aimed
at a different register, and the async-doorbell arm — correctly ruled and correctly defaulted —
does not touch this one at all. `worst_trap` barely moved across every arm today
(1 927 527 → 1 855 594 → 1 791 581 us) for exactly that reason.

★ **And it is a POPULATION, not an outlier**: `slow_traps(>1000us)=380`. A maximum can be waved
away; 380 violations in one boot cannot.

## ⊘⊘ CORRECTION — MY "launch RTT 1.23×" CLAIM DOES NOT SURVIVE n=2
| arm | GEMM GFLOP/s | launch RTT µs |
|---|---|---|
| `off` | 8.2, 8.3, 6.0 | 233 979 · 253 979 · 342 242 |
| `on` | 26.8, 20.1 | 189 551 · **343 213** |

- **GEMM SEPARATES**: `min(on)=20.1 > max(off)=8.3`. A real **2.4–3.3×**, no overlap.
- **launch RTT DOES NOT**: `on` spans 189 551–343 213 and `off` spans 233 979–342 242 — fully
  overlapping. ⇒ **My "233 979 → 189 551, 1.23×" was one boot against one boot**, and this tree
  already measured that trap: [[submit_ms_is_a_per_boot_lottery]] — 85.32 / 9.41 / 64.39 ms on
  three CONSECUTIVE boots of one build. I quoted a lottery ticket as a result.
⊘ The async default still stands: it is the **owner's ruling** and the **declared contract**,
and GEMM improves with no overlap. But the *launch-RTT* half of my justification is withdrawn
until n is larger.

## ⇒ NEXT CUT, AND IT IS NOT WHERE I WAS LOOKING
Make `QUEUE_HEAD` post-and-return: the store validates and enqueues, a worker services the GSP
command queue, and the guest observes completion the way hardware makes it — the response queue
plus the status IRQ that `BootStep::ClearStatusIrq` already models. The seam exists on the
event side; what runs inline is the servicing.

---

# ★★★★★ THE MULTI-APP WEDGE: THE ROOT IS `gpuStateLoad`, **NOT** WPR2
Owner, 2026-09-09: *"I really want iteration over many cuda apps."* That is currently gated by
the wedge, because **each app is one device open** and the suite dies partway through a boot.

The guest's own dmesg, in order (`traces/w394_cuda_apps/w394_guest_dmesg_after.log:274-303`):
```
356.583  RmInitNvDevice: *** Cannot LOAD STATE into the device  → RmInitAdapter failed (0x25:0xffff:1249)
358.282  RmInitNvDevice: *** Cannot INITIALIZE the device       → RmInitAdapter failed (0x24:0x40:1220)
359.551  _kgspBootGspRm: unexpected WPR2 already up             → RmInitAdapter failed (0x62:0x40:2028)
360.207  … repeats — by now the GPU is genuinely wedged
```
⊘⊘ **`WPR2 already up` is the THIRD failure and a CONSEQUENCE.** Two earlier re-inits already
failed and left the device dirty; only then does the WPR2 gate fire. **The root is the first
line: `gpuStateLoad` fails on the re-init after teardown.**

★ I had started down the WPR2 path — our FSM takes WPR2 down only on the Booter Unload
(`boot.rs:225`, SEC2 STARTCPU with the Unload argument → `Halted`), so "the guest never
unloads" was a tidy, plausible story. It is a story about the **third** symptom. Reading the
dmesg **in timestamp order** cost one command and redirected the whole investigation.
⚠ Same class as `rank_divergences_by_kind_never_by_index`: the first message by **time** is not
the first cause by **rank**, and here the loudest, most-quotable line (`WPR2 already up`, which
even tells you the GPU "may need to be reset") is the furthest downstream.

## ⇒ WHAT THIS MAKES THE NEXT QUESTION
Not *"why is WPR2 still up"* but **"why does `gpuStateLoad` fail on the second
`RmInitAdapter`"**. The tree already states the surrounding fact
(`kayfabe-gsp/src/boot.rs:1208`): `MESSAGE_QUEUE_INFO` lives **exactly one
RmInitAdapter↔RmShutdownAdapter span** (`kernel_gsp.c:3607` create, `:4353` destroy), *"which
the bench measured directly: the guest printed `Expected 0` on EVERY cycle."* So the re-init
path is known territory — it is the **state load** inside it that has never been made to work.

⊘ And note what this is NOT: it is not "the 5th open" as a magic ordinal (w370's framing). Four
opens succeeded because they shared one adapter span; the failure is the **teardown→re-init
cycle**, which is what a 5th open happened to trigger. A suite of 4 apps is not safe by being
under a limit — it is safe by never having torn the adapter down.

---

# ⊘⊘⊘ CORRECTION — THE 1.79 s IS A **LOCK WAIT**, NOT THE GSP SERVICING
Measured on branch `w395-gsp-submit-async` (14 correctness boots + 4 perf boots, both arms,
binary content-verified each round).

I attributed `worst_trap=1791581us at=bar0+0x110c00` to the inline GSP command drain, because
that register's handler services the queue in the store. **That inference was wrong**, and
segment timing (`KAYFABE_KFTIME=census`) says so:
```
KFTIME-SEG materialize  max_us=1877849 (armed)   1875049 (control)   ← matches worst_trap to 58 µs
KFTIME-SEG plane        max_us≈13000  on BOTH arms  ← and the control's `plane` INCLUDES the
                                                      inline GSP service
```
⇒ **The inline GSP service never cost more than ~13 ms per store.** The 1.9 s is the vCPU
**waiting for the device write lock**, held by an off-vCPU worker — and it is 1.9 s on the arm
where the GSP work was moved off the vCPU *and* on the arm where it was not.

## ★★★★★ WHY THIS IS THE DAY'S REAL FINDING
It explains a result I could not explain and reported three times as a puzzle: **`worst_trap`
barely moved no matter what I deferred** (1.93 → 1.86 → 1.79 s across the async-doorbell arms,
and now unchanged again across the GSP arms). Of course it did not. Deferring work off the vCPU
**relocates** the stall unless the worker also stops holding the lock the trap path takes.

⇒ **Corollary, and it belongs in the owner's rule:** *work moved off the vCPU must not hold a
lock the trap path takes.* Otherwise "asynchronous" buys the contract (the store returns without
doing the work) and **none of the latency** (the next store blocks on the worker instead).

## ★★ AND THE INSTRUMENT LESSON, WHICH IS THE SAME ONE AGAIN
`at=bar0+0x110c00` is **where the guest touched**, not **what the trap waited on**. Attaching a
site to `worst_trap` was a real improvement — it is what found QUEUE_HEAD at all — but a site is
not a cause, and I read it as one. The thing that actually decided it was **segment timing
inside the trap**, which no census had.
⚠ Fourth instance today of one shape: a number that names *something* being read as naming *the
cause*. `inline_exceptions` over four sites; the GEMM ratio over launch-vs-compute; `worst_trap`
over every register; and now a trap's site over a trap's cause.

## ⊘ WHAT `w395` DID AND DID NOT BUY
- **Correctness: unchanged.** 14 boots, both arms: `(P)`, `THREADS 4 of 4`, `MEAN_FALSIFIER=PASS`,
  `Xid=0`, `rpcRecvPoll=0`, `RmInitAdapter failed=0`.
- **The contract: bought.** The armed store no longer services the ring; the control's census row
  `[493 × GSP command ring serviced INLINE in the QUEUE_HEAD store ⊘ NOT ALLOWLISTED]` is present
  on `off` and **absent** on `on`. `cap1` (359 062 records) replays byte-identical Inline vs
  Deferred.
- **Latency: NOT bought.** `worst_trap` unchanged; `gpu_bench` INCONCLUSIVE (`on` 17.5–27.6
  GFLOP/s vs `off` 21.4–26.3 — the within-arm spread exceeds the between-arm gap).
⇒ Merge it for the contract and for the census, not for a speed claim. The speed is the next
rung: **time the lock acquire separately and name the holder.**

---

# ★★★★★ THE DOORBELL PUBLICATION WAS COVERING FOR **ONE** SYNCHRONIZATION POINT
Owner's model, 2026-09-09: all mappings are obtainable at three blockable points —
**(1) TLB invalidate, (2) RPC RM map calls, (3) kernel emulated channels setting up UVM**.

`[measured w399b]` doorbell publication **deleted**, `PT_SWEEP=on`, `KAYFABE_MMU_INVAL=on`
(armed, 688 triggers), BAR passthrough off:
```
P1  rm-invalidate  ✔ VERIFIED over 4 rounds     ← point (1)
P2  uvm-memop      ✔ VERIFIED over 4 rounds     ← point (3)
STALE RACE         ✔ VERIFIED over 2 rounds
THREADS            ✔ 4 of 4 verified
P3  rpc-bind       ★★★ CONTENT MISMATCH — still the poison 0xdeadbeef after 3s,
                       "the GR channel was scheduled but NEVER [completed]"   ← point (2)
```
⇒ **Two of the three points already carry their own mappings. The third — the RPC map-call
capture — does not, and the doorbell publication was covering for it.** That is the entire
residual dependency, and it is one named path rather than a diffuse "publication is needed".

★ The client's rows were built to exercise exactly these transports, so the mapping from
verdict to synchronization point is direct: `P1 rm-invalidate`, `P2 uvm-memop`, `P3 rpc-bind`.

## ⊘ AND A DELETE THAT WAS TOO WIDE, CAUGHT BY THE COMPILER
The first attempt removed `publish_vas_rows` and `PublishContext` along with the arm — 713
lines. The build then failed on `shim.rs:14315`:
```rust
let mut ctx = self.doorbell_port.publish_ctx();
ctx.vas_publish = VasPublishArm::Publish;
```
That is the **TLB-invalidate path's own publisher**. Deleting the publisher would have removed
the doorbell trigger *and* the good trigger's ability to publish — i.e. it would have deleted
point (1) while trying to delete a doorbell. ⇒ What must go is the **trigger and the knob**;
the publisher is shared and stays.

## ⇒ THE BAR RESULT, SEPARATELY AND UNAMBIGUOUSLY
Five boots: every arming with BAR passthrough **on** fails with `THREADS 0 of 4`; the only pass
has it off. Not publication (a one-variable control with publication back on still failed) and
not the sweep. Default reverted. ⚠ Note the failure SHAPE differs from P3's: BAR-on gives
`0 of 4` and copies that never retire; the RPC-bind gap gives `4 of 4` with one content
mismatch. Two different defects, and conflating them would have hidden the second.

---

# ⊘⊘ THE RPC-BIND PUBLISH IS RIGHT AND IS **NOT** THE FIX FOR P3
`[measured w401]` both synchronization points genuinely armed (`MMUINVAL armed=true`, RPC-bind
publication firing 4×, publishing 3 rows once and 0 three times). **P3 is still red.**

Three checks, in order, and each moved the diagnosis:
```
P3's VA 0x9140000000 in the log:            115 ×
   … inside an RPCBIND-PUBLISH line:          0 ×     ⇒ not a scope we published
first RPCBIND publish  line 141
P3's VA first appears  line 903                        ⇒ publication ran FIRST — not an ordering bug
contexts naming that VA: ALREADY-JOINED 46×, JOINED, WALK:, JOIN-RELEASE, CE-OPERAND
                                                       ⇒ THE VA IS BACKED
```
⇒ **P3 does not fail because promoted rows went unpublished.** The target is joined 46 times
over. The failure text says so plainly and I under-read it: *"the GR channel was **scheduled but
NEVER** [completed]"* — an **execution** failure, not a mapping one.

## ⊘ SO WHAT THE RPC PUBLISH ACTUALLY IS
Architecturally right and independently justified — synchronization point (2) should publish
what it binds, and before this it recorded rows into the spine and backed nothing, relying on a
doorbell leg that is now deleted. Keep it. But it is **not** P3's fix, and the commit that
added it must not be read as one. ⚠ It published 3 rows in this boot; whether anything needed
them is unmeasured.

## ⇒ THE NEXT QUESTION IS NARROWER THAN THE ONE I WAS ASKING
Not *"which mapping is missing"* but **"why was a scheduled GR channel never executed"**. The
distinction matters because the two have disjoint suspects: publication/join on one side,
doorbell routing and the forward on the other — and the join census already says the mapping
side is satisfied for this VA.
