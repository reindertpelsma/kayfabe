# Forwarding latency decomposition (2026-05-31, vast.ai RTX 3060, 580.159.04)

Question: which syscalls are slower than host, and **is it syscalls at all?**
Method: `LD_PRELOAD` shim (`tests/integration/syscall_prof.c`) timing each libc-
wrapped syscall with vDSO `clock_gettime` (~20 ns overhead, no ptrace), plus
`/usr/bin/time -v` (counts ALL kernel time incl. asm-inline syscalls), plus
one-shot in-tree timers in the stub (rdtsc around the real nvidia ioctl) and in
QEMU (clock around the stub round-trip).  Workload: `gpu_bench` (ring OFF).

## Headline: it is NOT the syscalls — it is the per-ioctl round-trip

- **Compute is unaffected**: GEMM throughput 421.6 (guest) vs 422.9 (host) GFLOP/s.
- **The NVIDIA ioctl in the stub ≈ host native** (same driver, same speed):
  RM_ALLOC 105 µs (host 100), RM_CONTROL 229 µs (host 214), RM_FREE 75 µs,
  MAP 69 µs.  The driver is not slower under forwarding.
- **Each forwarded ioctl pays a ~1–1.6 ms round-trip**, roughly constant
  regardless of the underlying call's native cost.

### Layer decomposition (per RM_ALLOC, exemplar)

| layer | latency | share |
|---|---|---|
| NVIDIA driver call (in stub) | ~105 µs | ~7 % |
| + QEMU↔stub socket/worker round-trip | +277 µs (→382 µs) | ~17 % |
| + guest↔QEMU (virtio + thread-pool offload + wakeups/sched) | +1233 µs (→1615 µs) | ~76 % |

The dominant cost (~76 %) is the **guest↔QEMU hop**: VM-exit on the virtqueue
kick, the IOCTL_ON_ISOLATE thread-pool offload, the completion bounce back to the
device AioContext, and the interrupt that wakes the blocked guest vCPU — a chain
of ~4–5 scheduler wakeups, each ~100s of µs on this oversubscribed shared host.
`/usr/bin/time -v` corroborates: guest **9939 voluntary context switches vs 48 on
host** (~200×) — the workload is dominated by *blocking handoffs*, not CPU-in-
syscall.  Guest System time 5.4 s / wall 10.2 s with ~4.5 s purely blocked.

## So "is it syscalls?"

No.  The syscall (the NVIDIA ioctl) is cheap and the same speed as host.  The tax
is the **multi-hop blocking round-trip** to forward each ioctl, ~76 % of it in the
guest↔QEMU layer (scheduling/wakeup latency, amplified by host oversubscription).

## Why this matters per workload

- **Decode/compute throughput**: unaffected — the hot launch/sync path is mapped
  doorbell/fence memory (zero forwarded ioctls), so it never pays the round-trip.
  Launch RTT is only +47 µs (the few control ioctls), alloc is 56× because it is a
  *chain* of ~6 forwarded ioctls × ~1.2 ms each.
- The optimization lever is **fewer hops/wakeups per forwarded ioctl** (e.g.,
  inline trivial ioctls on the vq thread to skip the thread-pool bounce; or a
  shared-memory path that removes the blocking handoff — the SPSC ring does this,
  but only helps batchable, ioctl-bound paths, which decode is not).  A dedicated
  (non-oversubscribed) host would also cut the ~1.2 ms scheduling component.

## Config experiment: CPU pinning + KVM halt_poll_ns (2026-05-31)

Tested the "it's mostly scheduling" hypothesis with zero code: pinned QEMU threads
to cores 2-6, stub workers to 7-10 (host 0-1), swept `halt_poll_ns`.  Alloc bench
(ring OFF), 11-core vast.ai instance:

| config | alloc RTT (µs/pair) |
|---|---|
| baseline (unpinned, halt_poll=200µs) | 6812 |
| pinned, halt_poll=0 | 7204 |
| pinned, halt_poll=200µs (default) | 5473 |
| pinned, halt_poll=2ms | 3856 |
| pinned, halt_poll=10ms | 3952 (plateau) |

- **~43% cut for free** (6812 → ~3850), `halt_poll_ns` the dominant knob —
  the vCPU polls for the completion instead of yielding and eating a re-schedule.
- **Plateaus at ~2ms**: beyond that the vCPU-wake component is fully covered, so
  the rest is NOT vCPU scheduling.
- **The ~3850 µs/pair floor is genuine multi-hop software round-trip, not jitter**:
  host load 0.92, CPU steal negligible.  So the half-right verdict: config removes
  the scheduling half; the other half is real plumbing (QEMU iothread + thread-pool
  + completion bounce + socket + stub-worker wakeups, × several ioctls/pair).
- Launch RTT (~54 µs) and throughput (422 GFLOP/s) unchanged throughout — confirms
  the launch/compute path is not ioctl-round-trip-bound.

**Takeaway:** pinning is a free always-on win; `halt_poll_ns` is a latency-vs-CPU
tunable (higher burns idle-vCPU CPU, only worth it for latency-bound deployments).
The residual software floor would only fall to the transport rework (collapse hops
/ host kernel module) — justified ONLY if a real workload is setup-latency-bound,
which decode/compute are not (already at host parity).

## Decode is 14× slower than host — root causes (2026-05-31)

Got the missing host baseline: same model/llama-cli on the host = **387 t/s** vs
guest **28 t/s** — decode is **14× slower**, NOT at parity. (Earlier "parity" was
matmul = ONE big compute-bound kernel; decode = hundreds of tiny launches/token,
so it's launch/exit-bound, a totally different regime.)

KVM exit profile during guest decode (`kvm:kvm_mmio` ftrace + debugfs counters)
found three culprits, in order discovered:

1. **HPET clocksource (70% of MMIO exits).** `-cpu host,hypervisor=off` cleared
   the hypervisor CPUID bit → guest never discovered kvm-clock → fell back to
   HPET → every clock read is a VM-exit (~3800/token). The `hypervisor=off` was
   to hide the VM from NVIDIA Code-43 detection, but in the FORWARDING model the
   NVIDIA driver runs on the host, not the guest — nothing to fool.
   **Fix: drop `hypervisor=off` → kvm-clock.** Forwarding + matmul still pass.

2. **virtio-nvgpu on legacy shared INTx, not MSI-X.** Every completion interrupt
   triggered shared-line ISR-status demux MMIO reads (~2150/token at the *net*
   device's BAR, sharing the line) + fasteoi. The block devices already used
   MSI-X; the nvgpu PCI wrapper left `nvectors` at the struct default 0 (not
   DEV_NVECTORS_UNSPECIFIED) so the usual idiom never enabled it.
   **Fix: force `vpci_dev->nvectors = 4` (3 VQs + config) before qdev_realize**
   (msix_init runs during device_plugged, so a later set is ignored). MSI-X
   Enable+ Count=4 confirmed; mmio_exits 2155→366/token.

**But single-stream decode barely moved (28 → 32 t/s).** Both fixes are real
(HPET 114k→0 exits; mmio 5×↓) and cut host-CPU/VM-exits per token — a density /
multi-tenant win — but they did NOT move single-stream throughput, and neither
did `halt_poll_ns` (0/200µs/2ms/8ms all ~32 t/s). So the 12× residual is **NOT**
VM-exit, interrupt, clocksource, or halt-scheduling bound. It's **per-launch
latency**: ~570 launches/token × ~54 µs/launch (vs 6.65 µs host) ≈ 31 ms/token.
Host view shows decode is single-threaded, one vCPU ~67% busy (~1/3 blocked) —
latency-bound in the launch+sync path itself, not exit-bound.

**Open (next investigation, guest-side):** where does the ~54 µs/launch go if not
exits/interrupts/halts? Candidates: doorbell-write path, completion-fence
visibility/coherency (PCIe-read vs cached), or guest-module CPU in the forward
hot path. Needs guest-side per-launch profiling, not host exit-counting.

## Per-subsystem microbenchmark (cuda_micro.c) — host vs guest, 2026-05-31

Driver-API microbenches, each isolating ONE subsystem (the right way to localize
the gap instead of inferring from LLM decode). Same binary, host vs guest:

| subtest | host | guest | ratio |
|---|---|---|---|
| 1 rm_control (cuMemGetInfo)   | 13.3 µs | 807 µs   | **60×** |
| 2 alloc+free (cuMemAlloc)     | 134 µs  | 3761 µs  | **28×** |
| 3 bandwidth HtoD              | 13.2 GB/s | 12.0 GB/s | ~parity |
| 3 bandwidth **DtoH**          | 10.2 GB/s | **0.1 GB/s** | **~100× slower** |
| 4 launch_sync (noop+sync)     | 6.5 µs  | 13.3 µs  | **2×** |
| 5 uvm_alloc (cuMemAllocManaged)| 163 µs | **fails (null→crash)** | broken |
| 6 uvm_migrate (CPU<->GPU)     | 3742 µs | (n/a, alloc fails) | broken |

Findings (several overturning earlier guesses):
- **DtoH is ~100× slower than HtoD** (0.1 vs 12 GB/s) while HtoD is at parity —
  an asymmetric bug in the device→host writeback path (memfd→shm readback),
  almost certainly unbatched/per-page. HIGH priority, was completely unsuspected.
- **rm_control 60× / alloc 28×** = the ~800 µs forwarding round-trip per ioctl
  (matches the earlier decomposition); the transport work targets this.
- **launch_sync only 2×** — the doorbell+fence path is NOT the big problem;
  this REFUTES the earlier "decode is launch-bound (~54µs/launch)" inference.
- **UVM managed memory is broken** — cuMemAllocManaged returns null in the guest
  (CPU-touch then segfaults); the managed/demand-paged path isn't supported.

Implication for decode: it's a MIX dominated by control round-trips (60×, the
most frequent per-token op) and possibly DtoH if results are copied back — NOT
launches. Fix priority: (1) the DtoH writeback bug (asymmetric, surprising),
(2) control/alloc round-trip (transport), (3) UVM managed (separate). Tool:
tests/integration/cuda_micro.c (driver API, dlopen libcuda, runs host+guest).

## Subtest 7 (poll_sync) + corrected DtoH picture (2026-05-31)

Added a poll-heavy subtest: CU_EVENT_BLOCKING_SYNC makes cuEventSynchronize block
on the event fd via poll() (vs the default spin). Loop: noop launch + event record
+ blocking sync.

| | host | guest | ratio |
|---|---|---|---|
| 4 launch_sync (spin)        | 6.1 µs  | 13.1 µs  | 2.1× |
| 7 poll_sync (blocking event)| 72.8 µs | 130.7 µs | **1.8×** |

**The poll/blocking-completion path is only 1.8× slower in the guest — NOT a
bottleneck.** This refutes the "poll dominates DtoH" reading: the ~92ms-per-poll
seen during DtoH was the poll *waiting* for slow migration work to finish, not
poll overhead. DtoH correctness verified OK (byte-exact).

So DtoH's real cost is the **host-side UVM per-page migration**: a plain
cuMemAlloc + cuMemcpyDtoH issues ~6118 `uvm 0x48` ioctls (per-page migration)
instead of a bulk copy-engine DMA; the guest blocks in poll waiting for it.
DtoH-slow and the cuMemAllocManaged err=999 are the SAME root area (UVM path).
Next: trace the host-side UVM migration during DtoH (why per-page 0x48, can it
be bulk/copy-engine) — that's the real lever, not the copy transport or poll.

## DtoH fix investigation (2026-06-01) — confirmed mechanism, found the lever, hit a wall

Goal: fix pageable cuMemcpyDtoH (100x slow). Findings:
- **Pinned (copy-engine) DtoH is ~instant: ~1857 GB/s vs pageable 0.1 GB/s.** So
  the copy-engine path is fine; only the pageable path is pathological.
- **Pageable slowness = per-page UVM_VALIDATE_VA_RANGE (0x48): ~6118 forwarded
  ioctls/copy.** libcuda routes pageable cuMemcpy through UVM's pageable-access
  path. DtoH correctness verified byte-exact.
- **cuMemHostAlloc (pinned) has an ~8MB cap**: works ≤8MB, FAILS ≥16MB with
  err=304 (OPERATING_SYSTEM). Separate bug; smells like a slot/mmap size limit.
  (libcuda's internal bounce buffers are small, so this doesn't block the fix.)
- **The fix lever**: make libcuda use the copy-engine/bounce path for pageable.
  TRIED forcing pageable-access UNSUPPORTED via the UVM_PAGEABLE_MEM_ACCESS(39)
  + _ON_GPU(70) ioctl response → **NO effect** (pageable DtoH still 0.1). So
  libcuda's pageable-copy decision is NOT gated on that UVM ioctl.
- **STUCK / next**: the decision is gated on the device attribute
  PAGEABLE_MEMORY_ACCESS (CUdeviceGetAttribute 88, =1 in guest, from an
  RM_CONTROL) or an internal libcuda decision. Need to identify the exact
  RM_CONTROL/field that reports pageable support and override it to 0, then
  re-test — OR fix the ~8MB pinned cap so apps using pinned go fast directly.
  Both are source-level digs (libcuda decision tracing / RM control byte-diff).

## "Deep migration" probe (2026-06-01) — DtoH does NOT use cpu_page_migrate

Hypothesis: pageable DtoH slowness is per-page cpu_page_migrate (the memfd
page-migration path), so bulk-migrating the whole range to one memfd would fix it.
Instrumented nvkvm_cpu_page_migrate with a call counter and ran a 32MB pageable
DtoH (still 0.1 GB/s, byte-exact). Result: **cpu_page_migrate fired 0 times.**

So the pageable DtoH does NOT go through our cpu_page_migrate/efault_resolve
memfd path at all — it's entirely UVM-internal (the ~6118 forwarded
UVM_VALIDATE_VA_RANGE ioctls + UVM's own page handling inside the stub). The
"bulk migration" fix targets the wrong path and would not help DtoH. (Probing
first avoided implementing a complex, correctness-sensitive mm change for a path
DtoH doesn't use.)

Confirmed levers for DtoH remain: (1) make libcuda avoid the pageable-UVM path
via the PAGEABLE_MEMORY_ACCESS(88) attribute → copy-engine (instant); (2) speed
up / cache the forwarded UVM ioctls. Both need source-level work (deferred to
the user's deep-dive).

## Bulk-migration idea tested → MOOT (2026-06-01)

Hypothesis: pageable DtoH slow because of per-page CPU-page migration (cpu_page_-
migrate via EFAULT-resolve); fix = migrate the whole range to one memfd at once.

TESTED (counter on the migrate path, measure-before-implement): **zero
cpu_page_migrate calls during a pageable DtoH** (count=0). So the page-migration
path is NOT triggered at all — there is nothing to bulk-migrate. The cost is
purely libcuda's forwarded UVM_VALIDATE_VA_RANGE storm:
  16MB DtoH x8 = 2022 uvm 0x48 ioctls (381ms);  64MB x8 = 6118 (scales w/ size).
DtoH byte-exact correct, no migration, no writeback involved.

So the ONLY levers for pageable DtoH are: (1) make libcuda not use the pageable
UVM path (the PAGEABLE_MEMORY_ACCESS(88) device-attr / RM_CONTROL lever — source
dig), or (2) cache/short-circuit UVM_VALIDATE_VA_RANGE in the guest (idempotent
range query; correctness-risky), or (3) general transport speedup. Bulk page
migration does NOT apply.

## DtoH poll root cause (2026-06-01) — it's the GPU copy, via the UVM pageable path

Stub-side rdtsc (around the real nvidia ioctl) + the poll breakdown localize it:
- poll=4.5s vs all-ioctls=1.5s during a 16MB-x8 DtoH → the guest is overwhelmingly
  blocked in poll() waiting for the GPU copy to COMPLETE (not forwarding/ioctls).
- Decisive: an EMPTY completion (cuEventSynchronize / poll_sync) = 131us; a
  DtoH-copy completion = ~45ms. That 45ms IS the GPU doing the copy.
- So pageable cuMemcpyDtoH runs the copy via the slow UVM per-page migration path
  (~45ms/16MB) instead of the copy engine (~1.6ms, what pinned uses). The relay
  works (131us); the forwarding works; the GPU copy itself is ~28x slow because
  libcuda took the UVM pageable path (PAGEABLE_MEMORY_ACCESS=1).
- Secondary: some forwarded allocs are slow IN THE STUB — RM_ALLOC(0x2b) up to
  31ms, RM_ALLOC_MEMORY(0x27) 13ms, RM_CONTROL(0x2a) 11ms (~73ms total, ~6%).
  Separate stub-side question (why are these host-driver calls 11-31ms?).

CONCLUSION: every DtoH symptom (OS_DESCRIPTOR migrate, validate storm, slow poll)
is the UVM pageable path. The single fix is forcing libcuda onto the copy engine
via the PAGEABLE_MEMORY_ACCESS(88) attribute (source dig: find the RM_CONTROL/
field). Stub slow-ioctl probe (>2ms) left in nvkvm_stub.c for the investigation.

## DtoH RESOLVED (2026-06-01, commit 578662f) — it was an uncached mapping, not ioctls

The "GPU copy via slow UVM pageable path" and "PAGEABLE_MEMORY_ACCESS(88) lever"
conclusions above were WRONG. Method that found the truth (host baseline first,
per the standing rule "always explain why the host does NOT have the issue"):

- Built dtoh_probe, ran the SAME libcuda binary on host and guest under strace.
- Host warm 16MB pageable DtoH = 9.66 GB/s; guest = 0.073 GB/s (130x).
- ioctl streams are IDENTICAL: 399 (guest) vs 402 (host) total, UVM cmd tally
  byte-identical, UVM_VALIDATE_VA_RANGE issued EXACTLY ONCE on both. So there is
  NO validate storm and the attrs match (88=1, 89=1 on both). The gap is 100%
  data path, zero ioctl involvement.
- Asymmetry: HtoD at parity (12.8 GB/s) because the guest fills its buffer while
  it is still cached anon RAM (before migration) and the stub then reads the
  memfd as host RAM — the guest never reads through the window on HtoD. DtoH
  forced the guest to READ the result through the migrated window.
- The migrated-range VMA (nvkvm_mmap.c bulk migrate) was remapped
  pgprot_noncached (UC). The window is memfd-backed host RAM in a KVM RAM
  memslot, not device MMIO — UC was gratuitous and made every guest read an
  uncached, unprefetched load.

FIX: vm_get_page_prot(vm_flags) => write-back cached. x86 keeps guest-WB /
stub-WB / GPU-DMA coherent (DMA snoops). Warm DtoH 0.073 -> 8.595 GB/s (118x,
host parity), byte-exact; HtoD unchanged; matmul + vector_add pass.

LESSON (reinforced): "guest slow because nvidia-internal / different path" was a
non-answer that hid the real bug. Forcing the question "why is the HOST fast?"
(same libcuda, identical ioctls) collapsed it to one wrong line of our own code.

## DtoH cached fix → 2.76x LLM DECODE (A/B on identical VM, 2026-06-01)

The cached-mapping fix is not just a DtoH-microbench win — it nearly TRIPLES 7B
decode throughput. Clean A/B (Qwen2.5-7B Q4_K_M, -ngl 99, -n 120, same VM/prompt,
only the one pgprot line changed + rebuild + restart between runs):
  - uncached (pgprot_noncached) baseline: 23.0 t/s generation
  - cached (vm_get_page_prot, WB):        63.4 t/s generation  (2.76x)
  (prompt eval ~287 -> ~326 t/s.)

This REFRAMES the decode-14x analysis: a large share of the decode gap was NOT
per-launch latency — it was the per-token host readback (logits + sampling
buffers) passing through the uncached migrated-range window. llama.cpp reads back
far more per token than logits alone (the effect is ~2.8x, not the ~15% logits
estimate). Host generation reference still higher; remaining gap is the next
target, but this single line closed most of the decode deficit. LESSON repeated:
microbench impact (118x on a 16MB copy) under-predicted real-workload impact
because real decode does many small pageable readbacks/token, each uncached.

## New decode bottleneck PROVEN (2026-06-01): GPU starved by per-launch latency

After the cached-mapping fix (decode 63 t/s vs host 387, ~6x), the residual gap
was localized by measurement, every alternative ruled out:

- NOT ioctls: strace -c, 8-token vs 128-token decode -> IDENTICAL ioctl count
  (958). 120 extra tokens add ZERO ioctls. Per-token path is ioctl-free.
- NOT VM exits: /sys/kernel/debug/kvm deltas, (N=128)-(N=8) -> mmio_exits delta
  EXACTLY 0 (1396 vs 1396); io/halt/irq/total all flat to noise. 120 extra
  tokens add ~0 exits of any kind. The vCPU neither traps nor halts during
  decode — it spins on a polled shared-memory handshake (doorbell/fence in the
  now-cached window).
- NOT compute, NOT readback: GPU utilization sampled on the host (same physical
  GPU) during steady decode:
    host-native decode: 98% GPU util, 100% mem util, ~170 W  (saturated)
    guest decode:       0-5% GPU util, 0% mem util, ~38 W    (starved/idle)
  The GPU does this exact work at 98% on the host; in the guest it sits idle
  waiting between tiny kernel launches.

CONCLUSION: decode is per-kernel-LAUNCH-LATENCY bound. The GPU is starved (0-5%)
because each launch is a long round-trip (doorbell ring -> host observes -> GPU
executes -> fence -> guest observes) with no trap/ioctl to accelerate or even
measure via exits. Rough: guest 16.7ms/token @ ~3% util => ~16ms/token idle;
at ~570 launches/token that's ~28us idle gap per launch (host ~us). This is why
the SPSC ring (#93) gave no decode win — it offloaded control ioctls, but the
launch path is not ioctls. NEXT (needs guest-side per-launch timing, do not
guess): decompose the 28us — doorbell-observe latency (is QEMU/stub polling the
doorbell region, and at what period?) vs fence-writeback propagation to the
guest's poll. That decomposition is the prerequisite for any launch-path fix.

## Decode launch latency localized to the COMPLETION-READ memory type (2026-06-01)

launchstorm microbench (empty kernel, CU_CTX_SCHED_SPIN), host vs guest:
  A pipelined submit:  host 2.99us  guest 4.82us   (1.6x — small, genuine virt cost)
  B launch+sync RT:    host 6.53us  guest 12.51us  (1.9x)
  C empty cuCtxSync:   host 0.36us  guest 3.19us   (8.9x  <-- the tax)
strace -f -c: 270k launches/syncs => only ~396 ioctls / 74 poll / 78 futex. So
submit AND sync make NO syscalls per op. The guest's 3.19us empty-sync is pure
userspace memory access = an UNCACHED read of GPU completion state (native MMIO/
cached read ~0.3us; ~3us = same read taxed uncached via WC mapping in the guest).

ROOT CAUSE (same class as the DtoH bug): the guest blanket-maps the device-mmap
path write-combining (nvkvm_mmap.c:105 pgprot_writecombine), regardless of whether
the region is real BAR (must be WC/UC) or pinned SYSMEM (should be WB). The host
backs all of these as WB RAM memslots but does NOT tell the guest the type; on x86
the guest's WC pgprot combines with EPT-WB to effective WC, so sysmem completion
semaphores are read uncached -> 8.9x slow sync -> serializes decode, GPU starves.

WHY host/VFIO-passthrough/vGPU/gVisor-nvproxy do NOT have it: all run the real
driver / pass the real mmap with the driver's CORRECT memory type — completion is
a WB-cached sysmem semaphore (or MSI via APICv), never an uncached read. None
re-type GPU memory. We do (blanket WC). => NOT an inherent KVM/EPT tax (VFIO+vGPU
are also KVM/EPT and don't pay it); it's our memory-type handling. FIXABLE.

FIX (must be per-region — cannot blanket-WB): a WB mapping of the real doorbell
BAR would leave the ring store in cache and never reach the device => decode HANG.
So plumb a memtype (WB sysmem / WC BAR) from the host mmap classification to the
guest, and have remap_pfn_range honor it (WB for sysmem, WC for BAR). Tracked #95.

## WB-sysmem fix landed — sync tax gone, but decode NOT bound by it (2026-06-01)

Mapped nvidiactl/nvidia-uvm (sysmem) mmaps WB-cached (kept GPU/DRM/modeset BAR at
WC). Results:
  - empty cuCtxSynchronize: 3.19us -> 0.37us (HOST PARITY, was 8.9x). Confirms the
    tax WAS our uncached mapping of a sysmem completion semaphore — i.e. our
    memory-type bug, NOT an inherent KVM/EPT tax (answer to "is it a hw VM tax":
    NO; VFIO/vGPU/gVisor keep this on WB sysmem and we now do too).
  - launch+sync RT: 12.5 -> 8.0us. matmul passes (doorbell BAR stayed WC -> no hang).
  - DECODE: 63.4 -> 64.4 t/s (UNCHANGED). GPU still 0% util / 38W (starved).

So decode's per-launch starvation is a SEPARATE bottleneck the empty-kernel
launchstorm did not reproduce (it OVER-predicted decode impact, mirror of how the
DtoH microbench under-predicted it). Decode is a CHAIN of DEPENDENT small kernels;
the GPU finishes one and idles waiting for the next to be submitted, so the cost
is per-dependent-launch latency, not the completion-semaphore read and not
pipelined submission throughput (A sustains ~240k/s). NEXT (#95): profile with a
DECODE-SHAPED workload — a dependency chain of small kernels on one stream (no CPU
sync between), guest vs host — and guest-side rdtsc around the submit of a kernel
that depends on the prior, to expose where the inter-launch idle comes from.

## CORRECTION: decode is at PARITY; "starved/14x" was measurement error (2026-06-01)

Evidence-based reconciliation overturns the decode-gap premise:
- Clean host decode (no profiler, same flags): 67.9 t/s. Guest: 64.4 t/s = ~95%.
- CUDA-API interposer (LD_PRELOAD, runtime API, cycle-accurate, same TSC): the
  big cost is cudaMemcpyAsync H2D >=1M — 155 calls x ~28MB = ~4.3GB = the WHOLE
  4.5GB model. That is the one-time MODEL LOAD upload, not per-token. Decode's
  per-token CUDA calls (small H2D <4K embeddings, launches) are minor.
- GPU-util TIMELINE across a guest run (sampled every 1s): 0-13s = 0-4% (model
  load, GPU idle), 14-22s = 95-97% util / 100% mem / 169W (decode, GPU SATURATED,
  identical to host 98%/170W), then done. The earlier "guest decode 0-5% util"
  was sampled at 4-8s = during model LOAD, not decode. With correct sampling,
  guest decode is GPU-bandwidth-bound at full util, same as host.

CONCLUSIONS:
1. Decode/inference throughput is at PARITY (~95%, GPU-saturated). There is no
   per-launch decode bottleneck. The "decode 14x slow" / "387 t/s host" was a
   STALE pre-session number (different model/build); it does not hold here.
2. The launchstorm micro-taxes are real but IRRELEVANT to decode: decode is
   memory-bandwidth-bound on the GPU (100% mem util), so CPU-side submission/
   sync/completion-read taxes are hidden behind GPU work. They only matter for
   CPU-bound / launch-bound or sync-heavy workloads.
3. The genuine remaining guest tax is MODEL LOAD: cudaMemcpyAsync H2D >=1M at
   ~0.82 GB/s guest vs ~14 GB/s host (17x) — a one-time ~10-15s startup penalty
   (llama.cpp uploads weights from the file-backed gguf mmap; that HtoD path is
   not the anon-OS_DESCRIPTOR path and is slow). Optimization target IF startup
   latency matters; not a throughput issue.

LESSON (again): get the clean baseline + correct sampling before declaring a gap.
The DtoH(#94) and WB-sysmem fixes remain correct wins on their own paths; they
just don't move decode because decode was never CPU/copy-bound.

## Model-load HtoD 17x — ROOT CAUSE = guest RAM < model; FIXED by -m 16G (2026-06-01)

The model-load HtoD tax (H2D >=1M, 0.82 GB/s guest vs 14 GB/s host) was NOT a
forwarding / memory-type bug. Evidence:
- htod_probe (fresh vs reused, anon vs file-backed): guest == host at every point
  (anon-reuse 12/13 GB/s; anon-fresh 1.2/1.4; file-mmap-fresh 4.2/4.7). So the
  HtoD path itself is at parity — the probe did NOT reproduce 0.82 GB/s because
  its file was page-cache-warm.
- Guest RAM was -m 4G (3915 MB) but the model is 4683 MB (4.4 GiB) — BIGGER than
  guest RAM. So the gguf could never be page-cached (buff/cache capped ~3.3 GB);
  every weight upload faulted from the virtio-blk disk during cudaMemcpyAsync.
  Host has 49 GB, caches the whole model (warm dd 8.3 GB/s), uploads at 14 GB/s.
- FIX: -m 16G in scripts/run_test_vm.sh (host has 49 GB). After warming the cache,
  H2D >=1M: 87.9M -> 5.78M cyc/call = ~12.5 GB/s = HOST PARITY (15x). H2D <1M also
  20x. Decode unchanged at 65 t/s (it was never the issue).

LESSON: the probe that "fails to reproduce" is itself evidence — it pointed away
from the GPU path and at page-cache/RAM. The whole guest-slowness saga reduces to
two real fixes (DtoH cached map #94, WB-sysmem map) + correcting two MEASUREMENT
errors (stale 387 t/s baseline; util sampled during model load) + one VM-sizing
fix (RAM >= model). No remaining inherent forwarding tax on the compute/IO paths.
