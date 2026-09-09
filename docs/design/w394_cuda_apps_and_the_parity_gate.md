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
