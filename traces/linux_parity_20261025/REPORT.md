# LX2 — the tested Linux graphics probe does not reach parity

STATUS: RESEARCH / PARTIAL, 2026-10-26. Primary matched measurement is complete; attribution stopped on native diagnostic instability. These are owner-authored probes, not application benchmark certification or a general Linux performance verdict.

CODE READ: the owner replaced VFIO with host bare metal as native reference. Workloads were built once on the host and copied unchanged into a matching host-mode guest. Both arms used NVIDIA driver/userspace 595.91.07 and kernel 7.0.0-34-generic. The kayfabe source/binary revision is 6564dd597; the lane branch contains harness/evidence only. The preregistration precedes all data: [PREREG.md](PREREG.md).


Phase 1 — MEASURED 2026-10-26, LX2 runs `01-N` through `12-N`, pinned 6564dd597, mirror default ON, kernel 7.0.0-34-generic / driver 595.91.07. Six complete paired runs (six fresh K boots), all four workload groups/run exited 0, no campaign Xid/OOM/instability receipt. Host cleanup receipt: no QEMU, GDM restored active.

| Workload | N median iterations/s [min,max] | K median iterations/s [min,max] | Paired K/N median [min,max] | Paired bootstrap 95% interval |
|---|---:|---:|---:|---:|
| surface | 1215.7211 [1183.5741,1217.5095] | 672.9025 [667.7851,676.9670] | 0.5547 [0.5494,0.5642] | [0.5510,0.5601] |
| cuda-spin-N16 | 226549.3677 [192233.0097,236842.1053] | 63057.3248 [61262.3762,64327.4854] | 0.2744 [0.2662,0.3346] | [0.2675,0.3075] |
| cuda-spin-N2048 | 113.3265 [113.1290,113.4518] | 113.3730 [113.1821,113.5885] | 1.0012 [0.9976,1.0023] | [0.9987,1.0023] |
| cuda-block-N16 | 44989.7774 [42289.6198,89189.1892] | 55463.2809 [51004.6368,57225.4335] | 1.2328 [0.6390,1.3532] | [0.8780,1.3077] |
| cuda-block-N2048 | 110.8890 [110.6776,111.2558] | 111.1826 [111.0546,111.6234] | 1.0027 [0.9992,1.0085] | [0.9995,1.0058] |
| stream-16MiB | 67461.6773 [67027.7590,68041.2371] | 46544.4288 [44574.5160,47075.6063] | 0.6885 [0.6605,0.6983] | [0.6694,0.6975] |
| stream-64MiB | 6599.5600 [6585.9500,6604.4029] | 6300.1162 [6287.3111,6312.5677] | 0.9556 [0.9527,0.9565] | [0.9531,0.9562] |
| stream-256MiB | 1719.6307 [1719.2276,1720.7516] | 1587.9508 [1586.5130,1588.5241] | 0.9232 [0.9226,0.9238] | [0.9228,0.9237] |

MEASURED 2026-10-26, LX2 primary frame distributions below: each cell is the median of six per-run statistics; exact per-run observations and ranges are in [primary-summary.json](primary-summary.json). Each run has 1000 measured frames after 20 warmups. Offscreen completion gaps are not presentation or broker frame times.

| Arm/interval | Median ms | p90 ms | p99 ms | Max observed ms | Median fraction >2x own median | Fraction >33/50/100 ms |
|---|---:|---:|---:|---:|---:|---:|
| N/completion_gaps | 0.812370 | 0.817029 | 1.170362 | 2.268330 | 0.2000% | 0.0000%/0.0000%/0.0000% |
| N/render_gpu | 0.778240 | 0.779264 | 0.783360 | 0.908288 | 0.0000% | 0.0000%/0.0000%/0.0000% |
| N/sample_gpu | 0.016384 | 0.016384 | 0.016384 | 0.022528 | 0.0000% | 0.0000%/0.0000%/0.0000% |
| K/completion_gaps | 1.420338 | 1.530007 | 3.454929 | 8.009950 | 1.9000% | 0.0000%/0.0000%/0.0000% |
| K/render_gpu | 1.364992 | 1.369600 | 1.574912 | 4.318208 | 0.1000% | 0.0000%/0.0000%/0.0000% |
| K/sample_gpu | 0.016384 | 0.017408 | 0.019456 | 0.265216 | 0.1000% | 0.0000%/0.0000%/0.0000% |

INFERRED: the tested Linux graphics loss is real beyond boot spread, so the remaining investigation is not solely a Windows-specific trigger. The surface render interval increases while the sampled pass and large matrix workload do not. This does not prove compression, DRAM traffic or active shader execution: timestamp intervals include GPU scheduling/dependency waits. Clocks, host telemetry and phase-2 traces must distinguish those explanations.

CODE READ/limitations: tiny-kernel printed timing is quantized to 1 microsecond; this is a tiny N=16 GEMM, not an empty kernel. Blocking tiny-kernel runs have wide native spread and remain indeterminate. CUDA streaming GB/s includes fixed launch/completion cost and is a lower bound for short kernels. No microsecond ratio should be presented with more effective precision than that instrument. One interim GPU-free analysis ran briefly while the primary campaign was active; no observation was excluded, and its perturbation was not separately bounded.

Phase-2 selection, before diagnostic data: worst normalized deficit is `cuda-spin-N16`; additionally trace `surface`, the GPU-heavy loss relevant to the owner’s question. Falsifier of submission-only cost for the tiny case: paired kernel durations rise comparably under the same CUPTI trace. Falsifier of a compression/placement interpretation for surface: no compressible mapping exposure, unchanged render GPU intervals after correctly supported placement, or a clock/queueing difference that accounts for the interval. No product fix or blind PTE-kind forwarding is proposed.


MEASURED 2026-10-26, LX2 primary telemetry: guest clock-bracket uncertainty was at most 3.44 ms. Native graphics clock means were 2785–2865 MHz, guest 2815–2854 MHz; memory clocks 10501 versus 10251 MHz, P0 versus P2. Graphics windows were under 1.5 seconds, so utilization means have startup-averaging limitations. Polling was nominally 100 ms; the maximum retained whole-boot gap was 1.258 seconds during initialization. No DRAM-byte or per-engine execution attribution follows from NVML utilization.

MEASURED 2026-10-26, LX2 diagnostic N1: a separate native long surface and traced surface completed, then the combined RM/strace/CUPTI CUDA step ended with SIGSEGV reporting. The application printed a content-check PASS; the CUPTI file contained only a clock marker and no usable kernel activity. The failing component and lifecycle point remain unknown. The suite reported `DIAG_TRACE_FAIL_CUPTI_INVALID`; no guest diagnostic, KINDSTAT hardware pass, debug ogkm pass or profiler retry followed. Raw traces stay private; no closed-binary code inspection was performed.

INFERRED: use Linux graphics as a debugging target, but do not equate a longer GPU timestamp interval with extra active shader execution. Compression, page size/placement, scheduling and submission-related starvation remain unseparated. A matched long-window utilization/kind census and working native tracing prerequisite are needed before a causal fix. No product fix or blind compressed-kind forwarding is proposed.

MEASURED 2026-10-26, LX2 cleanup receipts: all owned workloads, VMs, samplers and lease holders exited; GDM was restored and the GPU remained bound to the original NVIDIA module. Another session subsequently acquired the shared host; its VM was untouched. No dev build target was created. No VFIO/device detach, rental, delegated agent, Claude call, firmware or product-code change occurred.

GPU-free validation is in [VALIDATION.md](VALIDATION.md): 20 synthetic tests and five analysis mutants, each killed by at least two tests. Product before/after tests and product mutations are N/A. One CPU-only RM fixture checked decoded fields and preserved ENOTTY behavior; the combined hardware tracer remains unqualified after its native crash.

UNRUN: empty-kernel/event/graph/async-allocation/UVM/copy sweep extensions, cuBLAS, LLM, all-scene graphics suites, video and the ten-minute mixed loop. The primary matrix probe is naive GEMM, not cuBLAS TFLOPS. No long-session hitch absence or Linux-wide parity claim is made.
