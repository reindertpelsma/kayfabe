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
