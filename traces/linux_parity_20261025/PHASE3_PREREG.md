# Surface attribution preregistration

STATUS: PREREGISTERED, 2026-10-26, LX2 phase 3. This supplements [PREREG.md](PREREG.md); the primary six pairs stay unchanged. Hardware requires the shared lifetime lease, an empty host slot, captured desktop stop/restore and the original stop-on-Xid/instability/OOM rules. Another session's VM is never stopped. The phase is limited to four hours, with teardown reserve. CUPTI stays disabled after the prior native combined-tracing failure.

## 1. Accepted mapping census

One fresh guest boot, unchanged `surface` executable, 1000 frames, PF1 source revision `217d0504f248c2b876ca00b1e97af29a21303741`, binary SHA256 `a55dd8c0481ee664f881080ff537cc6517277ef19d9272dc747c5fc48d47e9f8`. Enable only optional `KF3_DIAG_KINDSTAT=1`; mirror defaults ON. Keep the primary kernel, driver, workload inputs, affinity and 100 ms sampler.

Report stripped bytes / all accepted mapping bytes and stripped bytes / compressible mapping bytes, including vidmem/sysmem subsets, raw/accepted kinds and guest leaf sizes. A zero denominator is undefined. CODE READ `crates/kf-mem/src/kindstat.rs`: aliases and remaps count again; these counters describe accepted mapping activity rather than unique allocation bytes, residency or traffic. Whole-boot totals include driver initialization. Periodic snapshots may bound a workload interval; without a usable boundary pair, do not call the final census workload-only.

Hypothesis: this surface workload exposes appreciable stripping of compressible guest kinds. Falsifier: near-zero compressible/stripped mapping exposure. Compare the diagnostic render interval median with the original K per-run median range. If it falls outside, explicitly report binary/instrumentation non-equivalence. One diagnostic boot cannot establish a new performance distribution or identify overhead as the cause.

## 2. Attachment and target-size controls

CODE READ `scripts/bench/linux_parity/surface.c`: the original probe has fixed dimensions/attachments and uses single-sample storage. Keep that executable unchanged for the census. A separate benchmark-only `surface_variants.c` supplies runtime attachment and size controls; no product/kernel/driver change is proposed.

| Variant | Render target | Dimensions | Sampled colour pass |
|---|---|---|---|
| base | RGBA8 + depth24 | 1920x1080 | yes |
| colour | RGBA8, no depth storage/test | 1920x1080 | yes |
| depth | depth24, no colour attachment/output | 1920x1080 | absent |
| small | RGBA8 + depth24 | 512x512 | yes |
| large | RGBA8 + depth24 | 3840x2160 | yes |

All configurations query the render FBO's sample count/buffers and require zero. MSAA-off is already the baseline/control, not a distinct treatment. Depth-only has no sampled-pass timing claim. Retain 64 ordered layers, 20 warmups, 1000 frames, GPU timestamps and completion waits. Verify final colour where present and final depth where present. Nominal attachment size is not actual padded allocation size or DRAM traffic. A cache-capacity comparison also needs the actual device cache-size metadata; nominal fitting alone does not prove cache residency.

Compile the executable once on the host and copy it unchanged into the guest image. Use the pinned primary product `6564dd5977bbcd6aa947359b55b4aa3d9a5e92f4`, without diagnostics, for six matched native/K pairs: N,K / K,N / N,K / K,N / N,K / K,N. Each K run is a fresh boot; native references are fresh processes. Rotate the five configurations by pair index modulo five, identically within a pair. Keep all observations; no outcome-driven replacement or repetition.

Report render interval K/N (larger is slower), reciprocal-mean-completion throughput K/N, six pair ratios and their median/min/max/IQR plus complete-pair bootstrap 95% interval (10000 resamples, seed 20261025). Retain per-run frame median/p90/p99/max, >2x own median and >33/50/100 ms fractions. Compare the new base with the original range; disclose any baseline mismatch and keep variant comparisons internal to this batch.

Hypothesis: the excess write-pass interval changes with target footprint and/or depth versus colour. Falsifiers: essentially invariant ratios across attachment composition/size, or the same excess with the small target. Changes support association; shader, ROP, cache and depth-test work also changes, so they cannot prove compression causality.

## 3. Matched allocation observation

One native and one fresh K surface-only trace, original executable, matching open-595-header observer and `strace -f -yy -e trace=ioctl -s 0`. No CUPTI. Trace timings are diagnostic and excluded from speed ratios. Public GL storage-call boundary markers are separately qualified with GPU-free forwarding checks before hardware. Allocations outside synchronous boundaries remain ambiguous.

Decode input/output NVOS32 flags/attr/attr2, COMPR bits13:12, ZBC bits1:0, requested page size, format, size and comptag fields. Keep serialized/unknown interfaces undecoded. Match requests by public GL storage boundary where possible; size/sequence alone is weaker evidence. Observe host-authored backing allocations in the owned QEMU, supported by pinned-source reading.

CODE READ `crates/kf-abi/src/submit.rs`: `NvMemoryAllocationParams.kind` encodes the allocation **type**, not the PTE kind. Keep that field, allocation `format` and raw/accepted guest leaf kinds separate. CODE READ `crates/kf-mem/src/apply.rs`: current policy strips compressible kinds while retaining known layout kinds. Client-side RM allocation attributes are not automatically visible at the twin; the join is guest leaves plus host-authored backing attributes.

Hypothesis: guest surfaces request compression and their leaf kinds carry that information while host backing lacks compression resources. Falsifiers: absent compressible requests/leaves, undecodable or ambiguous target allocations, or host compression-capable backing. An aggregate census cannot identify an individual GL allocation. No kind forwarding, store-policy change, module reload or compression intervention is authorized by this experiment.
