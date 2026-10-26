# LX2 phase 3 — the render deficit is depth-sensitive

STATUS: RESEARCH, 2026-10-26. Requested measurements are complete; causal compression attribution and an exact allocation-to-leaf VA join remain unverified. The original [primary result](REPORT.md) is unchanged. Phase 3 was explicitly resumed after the earlier native combined-tracing instability; CUPTI remained disabled.

MEASURED 2026-10-26, run `lx2-phase3-20261026`, six native/K pairs `02-N` through `13-N`: colour-only render interval K/N is 0.998682, depth-only 3.034574 and combined 1.756261. Colour-only completion throughput is 0.937124 K/N, so render parity does not mean end-to-end parity. All 60 variant groups completed with valid applicable colour/depth checks and zero render-FBO samples/sample buffers. No product behavior changed.

INFERRED from `lx2-phase3-20261026`: the strongest association is depth rather than colour writes in this probe. Compression/layout policy is a supported lead. Target composition also changes shader/ROP work, and GPU timestamps include waits/preemption/submission starvation. These data do not prove how much additional time compression causes or establish a general Linux/application result.

## Variant comparisons

MEASURED 2026-10-26, run `lx2-phase3-20261026`, [preregistered](PHASE3_PREREG.md): same executable in both arms; 64 layers, 20 warmups, 1000 frames/configuration; six balanced pairs, six fresh K boots. Native references are fresh host processes. Rotate variant order identically within each pair. All runs are retained. Native and guest use kernel 7.0.0-34-generic / NVIDIA 595.91.07; headless EGL, host desktop stopped under the lifetime lease, affinity CPUs 0–7; K has eight vCPUs, 8 GiB RAM and 8 GiB store. Native capacity differs. Pinned primary product revision `6564dd5977bbcd6aa947359b55b4aa3d9a5e92f4`, SHA256 `82a806eeacbfc3df5f42e1575fd4b242d7fda7d3f4781f8fe0f708398c0a658a`, mirror default ON, diagnostics disabled for this batch.

MEASURED 2026-10-26, run `lx2-phase3-20261026`, [phase3-variants.json](phase3-variants.json): render columns are medians of per-run medians. Render K/N is a median of six pair ratios, larger is slower. Throughput K/N uses reciprocal mean completion gaps, larger is faster. Bootstrap resamples complete pairs (seed 20261025, 10000 samples). Exact per-run ranges, IQRs, both ratio intervals and stdout hashes are in JSON.

| Variant | N render ms | K render ms | Render K/N [pair range] | 95% CI | Throughput K/N [pair range] |
|---|---:|---:|---:|---:|---:|
| base | 0.778240 | 1.367040 | 1.756261 [1.750000, 1.759211] | [1.751974, 1.759211] | 0.561133 [0.552599, 0.568293] |
| colour | 0.777216 | 0.776192 | 0.998682 [0.996063, 1.000000] | [0.996714, 1.000000] | 0.937124 [0.909509, 0.946633] |
| depth | 0.192512 | 0.584192 | 3.034574 [3.026455, 3.048128] | [3.029185, 3.045341] | 0.330839 [0.317359, 0.337311] |
| small | 0.107520 | 0.185344 | 1.732097 [1.723810, 1.962264] | [1.723810, 1.851324] | 0.516184 [0.480118, 0.528960] |
| large | 3.060736 | 5.544960 | 1.811592 [1.804594, 1.813985] | [1.806947, 1.813817] | 0.544212 [0.543258, 0.546302] |


CODE READ, `surface_variants.c`: base is colour+depth 1920x1080; colour omits depth storage/testing; depth has no colour attachment/output and no sampled pass. Small/large retain both attachments at 512x512 / 3840x2160. Single-sample storage is already the baseline; MSAA-off is checked rather than counted as an independent treatment. Original `surface.c`/executable is unchanged. Variant executable SHA256 `9ca2731f972f6c678a7c3eaa782bcda870251a685c7147da3f14756f141cb4fa`.

MEASURED 2026-10-26, run `lx2-phase3-20261026`, [phase3-variants.json](phase3-variants.json): new base K medians all fall within original K spread. New native base median remains 0.778240 ms; two native per-run medians, 0.776192 and 0.782336 ms, fall outside the original single-value quantized median range. Keep this batch's comparisons internal; do not replace the primary six-pair result with it. Frame median/p90/p99, maxima and spike counts are stamped in [PHASE3_DISTRIBUTIONS.md](PHASE3_DISTRIBUTIONS.md).

MEASURED 2026-10-26, run `lx2-phase3-20261026`, device metadata query: L2 capacity is 37,748,736 bytes (36 MiB), without a CUDA context/kernel/profiler. Nominal three-attachment footprints are 3 MiB small and 94.921875 MiB large. Small/large render ratios remain 1.732097 / 1.811592; a strong proportional deficit persists below nominal capacity. INFERRED: no demonstrated cache-capacity threshold or off-chip-traffic result. Nominal fit does not establish actual padded size or residency.

MEASURED 2026-10-26, run `lx2-phase3-20261026`, [phase3-telemetry.json](phase3-telemetry.json): guest clock bracket uncertainty ≤2.901 ms. Base windows contain 8–9 native / 14–15 K query receipts; depth 2–3 / 6–7; small 1–2 / 2–3; large 31–32 / 57–58. Depth SM-clock run means are 2850–2880 native / 2810–2880 MHz K; memory clocks 10501 / 10251 MHz. Small K includes a 2620 MHz sampled run mean and retains its timing outlier. The whole-boot maximum sampler gap is 2.215487 s. Queries are nominally 100 ms; NVML averaging and short windows prevent per-pass active-time or DRAM-traffic claims.

## KINDSTAT exposure

MEASURED 2026-10-26, `lx2-phase3-20261026/01-K`: unchanged original 1000-frame surface under PF1 `217d0504f248c2b876ca00b1e97af29a21303741`, binary SHA256 `a55dd8c0481ee664f881080ff537cc6517277ef19d9272dc747c5fc48d47e9f8`, optional `KF3_DIAG_KINDSTAT=1`, mirror ON. Exactly one full exit summary; app/boot exit 0 and final content checks pass. Render median 1.369088 ms lies within primary K range [1.364992,1.369088]; sampled-pass median 0.016384 ms. n=1 does not qualify this binary's variance. Full frame statistics and counters: [phase3-kindstat.json](phase3-kindstat.json).

MEASURED 2026-10-26, `lx2-phase3-20261026/01-K`, final census: 8,418,930,688 accepted mapping bytes; 8,359,116,800 compressible and all of those stripped. Stripped/all is 99.289531%; stripped/compressible 100%. Vidmem stripped/all is 99.452882%; sysmem has no compressible denominator.

| Raw → accepted kind | Memory | Guest leaf | Accepted mapping bytes |
|---|---|---|---:|
| 00 → 00 | vidmem | 4K | 20,480 |
| 00 → 00 | sysmem | 64K | 65,536 |
| 06 → 06 | vidmem | 4K | 1,400,832 |
| 06 → 06 | vidmem | 64K | 4,718,592 |
| 06 → 06 | vidmem | 2M | 39,845,888 |
| 06 → 06 | sysmem | 4K | 13,762,560 |
| 08 → 06 | vidmem | 2M | 31,457,280 |
| 09 → 06 | vidmem | 2M | 8,317,173,760 |
| 0C → 03 | vidmem | 2M | 10,485,760 |


CODE READ, PF1 `crates/kf-mem/src/kindstat.rs`: accepted mapping activity counts aliases/remaps again, excludes CPU views and refused/rolled-back rows, and records accepted host requests rather than hardware PTE readback. Guest leaf size is not host PTE size. Raw `08/09` are generic compressible kinds; `0C` is S8Z24 compressible with PLC disabled. MEASURED 2026-10-26, `01-K`: the approximately 8 GiB raw09 bin dominates. Periodic counters are zero immediately before app setup, and all compressible bytes appear by an in-frame snapshot; interval bounds include context/driver setup and teardown. They are not frame-only bytes, unique residency or bytes written by the probe. Periodic JSON has a following status field; extraction uses JSON's bounded decoder, preserving full raw lines privately.

## Matched RM requests and host backing

MEASURED 2026-10-26, `lx2-phase3-20261026/14-N` and `15-K`: original surface under the same open-595-header observer and `strace -f -yy -e trace=ioctl -s 0`, no CUPTI. Both app/boot exits and applicable content checks pass; app stderr is empty; strace contains no SIGSEGV/SIGABRT/SIGBUS/SIGILL. NVOS32 allocation call counts are 101 native / 96 K. Storage-only GL boundary markers enclose no allocation calls: allocation is deferred. Matching physical colour/depth-format signatures below occur outside those markers. Association with specific GL attachments relies on format/size/order and remains INFERRED; an object/VA join is not claimed. Full sanitized signatures/counts and trace hashes: [phase3-rm.json](phase3-rm.json).

MEASURED 2026-10-26, `14-N`/`15-K` decoded allocation fields, identical common physical signatures in both arms:

| Field | Colour-format request → return | Depth-format request → return |
|---|---|---|
| Flags | 0x0000C000 → 0x0000C000 | 0x0000C000 → 0x0000C000 |
| attr | 0x00022004 → 0x11821004 | 0x00222004 → 0x11A21004 |
| attr2 | 0x00000004 → 0x00100006 | 0x00000004 → 0x00100006 |
| COMPR | ANY → REQUIRED | ANY → REQUIRED |
| format | 00 → 08 | 00 → 0C |
| Page policy | DEFAULT → HUGE/2 MiB | DEFAULT → HUGE/2 MiB |
| ZBC | DEFAULT → PREFER_ZBC | DEFAULT → PREFER_ZBC |
| Bytes | 8,847,360 → 10,485,760 | 8,847,360 → 10,485,760 |
| RM return status | 0 | 0 |


MEASURED 2026-10-26, [preregistered supplementary capture](PHASE3_HOST_RM_PREREG.md) `lx2-phase3-20261026/16-K`: own-QEMU observer records the successful contiguous 8 GiB store allocation, explicit 128-byte NVOS21/class40 payload. Request: flags0, attr0x10000000, attr2=0, format0, COMPR_NONE, pageDEFAULT, ZBC_DEFAULT, alignment field1 GiB. Return: flags0x8000, attr0x11800000, attr2=0x00100005, format06, COMPR_NONE, HUGE/2 MiB, PREFER_NO_ZBC, alignment field2 MiB; size remains 8 GiB, RM status0. No fallback store attempt appears. Internal-library implicit-size NVOS64 payloads remain undecoded. The host backing is uncompressed despite the guest-side compressed surface formats.

CODE READ, `kf-abi/src/submit.rs:1588`: `NvMemoryAllocationParams.kind` is the allocation type (`IMAGE=0`), not PTE kind; the separate allocation format field starts zero. Open595 `nvos.h` documents alignment flags HINT0x80/FORCE0x100; neither is set in the store request. The returned 2 MiB alignment field does not establish actual physical base alignment. Do not infer the source comment's 1 GiB congruence guarantee from the requested field alone; this needs separate review.

CODE READ, `kf-mem/src/apply.rs:290` and `kf-chip/src/ptekind.rs`: current mapping policy strips raw08/09→06 and0C→03. Open595 `virtual_mem.c:1375` also downgrades compressible overrides over uncompressed backing lacking comptag resources. INFERRED from those sources plus `01-K`, `14-N`/`15-K` and `16-K`: the raw leaf kind carries compression/layout/PLC intent, but not the entire CPU RM allocation attr/attr2/ZBC request. Compression-capable backing and view/ZBC/comptag correctness must be established before honoring such a leaf. No compression intervention or blind forwarding was performed.

## Qualification, incidents and cleanup

MEASURED 2026-10-26, run `lx2-phase3-20261026`, [phase3-validation.json](phase3-validation.json): existing analyzer/sampler 20/20; public GL observer 4 expected failures then 4/4; target-only injection 2 expected failures then 2/2; private guardian lifecycle 3 expected failures then 3/3. Six GL observer mutants each have two killing tests; four injection mutants each two; wrong guardian namespace three. Product behavior/tests/mutations are N/A; PF1's own qualification is separate. CPU fixtures preserve the actual ENOTTY result. These checks and successful surface captures are not general profiler/transparency qualification.

MEASURED 2026-10-26, run `lx2-phase3-20261026`: the first guardian mistakenly used the prior phase directory, saw its stale RELEASE flag, stopped/restored the desktop and exited without a VM/workload. It overwrote prior guardian metadata; the original release receipt survives in the earlier saved audit, and the incident is preserved without silently rebuilding the old files. Three lifecycle tests reproduced it before the namespace correction. One early lock probe was 22 seconds short of five minutes; subsequent guarded probes enforced the interval. The first own-QEMU RM log was empty because the affinity wrapper consumed its exclusive preload log. CPU tests reproduced that transport bug; target-only injection enabled one preregistered supplementary host capture, excluded from speed ratios. No fault was reclassified or waived.

MEASURED 2026-10-26, run `lx2-phase3-20261026`, cleanup receipt: 16/16 valid runs, 64/64 workload completions, nine fresh guest boots exit0, no new Xid/OOM/instability match; all owned workloads/samplers/VMs exited, desktop restored and lifetime lease released at 04:06:31 UTC. No forced QMP termination, module/firmware change, VFIO detach, product fix, rental or delegated agent. The earlier CUDA tracing SIGSEGV remains unresolved and CUPTI was not retried.

Unverified: exact GL object→allocation→guest leaf VA identity, actual host PTE readback/comptag coverage, cache residency/traffic, active-work versus waits, compression causality, actual physical base alignment, full applications and long sessions. The next owner decision is whether to authorize a correctness-qualified compression backing/view/ZBC experiment; these timing and allocation observations alone do not justify enabling compression.
