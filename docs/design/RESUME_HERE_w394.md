# ★★★★★ RESUME HERE — w394/w395/w396, 2026-09-09

**STATUS: LIVE.** Supersedes `RESUME_HERE_w392_overnight.md` (whose two objectives are both met
and in the ledger). Everything here is measured unless marked. Box `50376491` = `ssh w393`.

## ✅ WHAT IS DONE AND GRADED
| lane | state |
|---|---|
| (1) CUDA apps | **PASS** — `W394_OUTCOME=(P)`, 4/4 verifying probes byte-correct in the guest vs a same-box native control |
| LLM | **PASS**, n=2 — text byte-identical to the same-boot CPU oracle, 0 new Xids |
| (3) BAR untrapped | **MERGED to master** (`281ba010`) — BAR1 traps 88 193 → **91** (969×), BAR2 own arm, client `(P)` 4/4 on all arms |
| (2) Parity | **MEASURED, and it is ONE TERM: the launch** |

## ★★★★★ PARITY IS ONE NUMBER, AND EVERY WORKLOAD AGREES ON IT
```
rm_control    72.6×   alloc+free  70.2× / 65.7×   launch RTT  56 757×   launch+sync 23 648×
uvm_alloc      1.075× (the negative control — a path that never reaches us is native)
GEMM  0.014×  ⊘ NOT a compute deficit: 5 launches × 342 ms = 95.7 % of the wall;
              the residual runs at ~138 GFLOP/s against a native 426.2
LLM   0.015 tok/s = 65 161 ms/token ÷ 342 ms/launch = 190 LAUNCHES PER TOKEN
              (a 24-layer 0.5 B forward pass — an independent workload, the same constant)
```
⇒ Two clusters three orders apart: a flat **~70× control-plane tax** and a **~24 000–57 000×
submit path**. Fix the launch and everything moves together.

## ★★★★★ THE OWNER'S RULINGS OF 2026-09-09 — these are the specification
- *"no inline blocking executions in mmio traps. just general rule. real gpu also never holds
  any mmio write for milliseconds right. like rpc mmio starts the operation, the block is for
  example a semaphore"*
- *"the thing is to avoid a blocking call on vcpu thread at all, unless its required like a
  memslot install, even mmaps in vmm va can often run largely off vcpu thread"* ⇒ an
  **ALLOWLIST**, not a blocklist.
- *"submit register is a schedule, you should return to vm immediately and run it off the vcpu
  threads"* · *"do not execute emulated channels under a doorbell"* · *"doorbell must be simple
  … its like a hundred lines what it touches"* · *"that must be on, and the off path eventually
  discarded"*

### What landed for them
- `KAYFABE_DOORBELL_ASYNC` **default flipped to `on`** (`be9ac427`), `off` kept as the control,
  default **pinned by tests**. Measured before flipping: GEMM 8.2 → 26.8 GFLOP/s (separates,
  `min(on) > max(off)`); correctness unchanged.
  ⊘ The launch-RTT half of that claim was **WITHDRAWN** at n=2 — the arms overlap, and
  `submit_ms` is a per-boot lottery (9.1× across three boots of one build).
- `worst_trap` now carries **`at=bar<N>+<offset>`** and a `slow_traps(>1000us)` **count**.
- `BlockingSection` is now the **allowlist** (`8a84ef9f`): `enter(what)` marks
  `⊘ NOT ALLOWLISTED` inside a trap, `enter_required_on_vcpu(what)` is the door, and the census
  **states its own coverage** — the whole workspace had ONE call site while a trap ran 1.79 s.
- `VERBCOST` (per verb kind) and `INLINE-BY-REASON` (per inline site) censuses.

## ★★★★★ THE WORST TRAP IS `NV_PGSP_QUEUE_HEAD(0)`, NOT THE DOORBELL
`worst_trap=1791581us at=bar0+0x110c00`, `slow_traps=380`. The write routes to
`BootStep::CommandDoorbell` → `boot.rs:945 self.doorbell(ram, policy)`, which **services the
whole GSP command queue inside the guest's store**. All the launch-floor work aimed at a
different register — which is why `worst_trap` barely moved (1.93 → 1.86 → 1.79 s) whatever I
did to the doorbell.
⏳ **IN FLIGHT**: agent on branch `w395-gsp-submit-async` (pushed) making it post-and-return.

## ★★★★★ THE MULTI-APP WEDGE — ROOT FOUND, AND IT IS NOT WPR2
```
356.557 _memmgrMemUtilsScrubInitRegisterCallback: EVENT NOTIFICATION CONTROL FAILED
      → objCreate(&pScrubber->pCeUtils) → scrubberConstruct → gpuStateLoad
      → RmInitNvDevice: *** Cannot load state into the device      (0x25)
358.282 Cannot initialize the device                                (0x24)
359.551 _kgspBootGspRm: unexpected WPR2 already up                  (0x62)  ← THIRD symptom
```
⊘ It is **not "the 5th open"** as an ordinal: four opens shared ONE
`RmInitAdapter↔RmShutdownAdapter` span; the failure is the **teardown→re-init cycle**.
⊘ `OsEventLog::gated`'s doc predicts this exact shape and says *"no test in this repository can
observe"* it — so `OSEVENT batches/gated/…` now prints at teardown (`2a26c950`). **A wedged
boot with `gated` large and `batches` small implicates the latched event gate; `gated == 0`
exonerates it.** That measurement has NOT been taken yet — it is the next cheap decisive step.

## ⇒ NEXT, in order
1. Take one wedged boot and read `OSEVENT` — implicate or exonerate the event gate.
2. `w396` battery (`scripts/bench/w396_hook.sh`) — **12 workloads in ONE process**, each with
   its own CPU oracle, detection self-tested by fault injection. Breadth for the owner's *"I
   really want iteration over many cuda apps"* **without** waiting for the re-init fix, because
   the wedge is per device OPEN, not per workload.
3. Land `w395` (QUEUE_HEAD post-and-return), then re-measure the whole parity table.
4. Wall 2 — H2D/D2H ~17 s per 16 MiB, measured, **NOT BAR1** (5 839 accesses in a boot that
   moved 128 MB), still **unattributed**.
5. Anomalies: an unsynced launch costing 2.4× MORE than a synced one; `cuLaunchKernel 719` in
   `cuda_micro` subtest 6 with 1 host Xid (lane 4, UVM).

## ⚠ TRAPS PAID FOR TODAY
- **A scalar over many sites is not a measurement.** Three instances in one day —
  `inline_exceptions` over 4 sites, the GEMM ratio over launch-vs-compute, `worst_trap` over
  every register. Each looked like an attribution. Attaching a site cost ~20 lines and found
  QUEUE_HEAD.
- **Read a failure log in TIMESTAMP ORDER.** The loudest line (`WPR2 already up`, which even
  says the GPU "may need to be reset") was the furthest downstream.
- **n=1 is not a grade for a timing number.** I quoted a 1.23× that did not survive a second boot.
- **A default with no test drifts.** The contract, the owner's ruling and the arm all existed;
  only the default disagreed, so every boot measured the violation.
- **Verify a binary BY CONTENT** (`strings | grep`), never by its rev stamp.
- `build_qom_shim.sh` needs `<qemu-src> <build-dir>` and **exits 1 without them while the boot
  proceeds on the OLD binary**.
