# M2 — Windows TSG mirror gates: NOT GREEN, 2026-10-19

MEASURED lane **M2**, branch **codex/tsg-mirror-win-gates-20261019**. Asked: one Windows Vulkan/CUDA/video pass, then concurrent load, without restarting the owner's VM. Tested binary **0d7ba743**, source lineage **c2685412e159534b88097ea824d05fe963be83b1**, MD5 **48ab1b70c2cf2c26d2c1287bf7311a9a**, candidate-5 + mirror. Mirror **ON**, environment kill switch unset. Failure-stage head **c8d9dc968abae90b9a372b4fdf96c67bb7b4cbfc** verified on origin; this final evidence/report commit follows it. No product code changed.

MEASURED results (Xid values are deltas; TDR column records nvlddmkm/4101 events):

| Step | Tool/version | Result | Xid delta | TDR | Evidence path |
|---|---|---|---:|---|---|
| Baseline | NVIDIA-SMI 580.88; host 595.91.07 | PASS, 60.091 s idle | 0 | 0 | [baseline-result.md](baseline-result.md) |
| Vulkan | vulkaninfo 1.4.309; GravityMark 1.89 | PASS isolated, 29 queues / 6 families; 167 s render; SYNC + 2 ASYNC subcontexts | 0 | 0 | [vulkan-result.md](vulkan-result.md), [vulkan.png](vulkan.png) |
| CUDA | nvcuda 580.88; Python 3.13.13 | PASS isolated, 2 contexts / 8 streams; 16,777,216 correct integer comparisons, both rc=0 | 0 | 0 | [cuda-result.md](cuda-result.md) |
| Video | FFmpeg/FFprobe n8.1.3-14-g330caae0c1 | PASS isolated, NVENC H.264/HEVC 300 frames each; NVDEC 300/300 hashes match CPU | 0 | 0 | [video-result.md](video-result.md) |
| Combined | Same Vulkan + 2 CUDA jobs + H.264 NVENC | **FAIL**, >111 s encode stall, TDR, device lost, CUDA 999, FFmpeg 0xC0000005 | 0 | **3 Event 153; 0 Event 4101** | [combined-result.md](combined-result.md), [failure-readonly.txt](failure-readonly.txt) |
| Final | NVIDIA-SMI; QMP/OS/process queries | PARTIAL: responsive, same VM/binary; cleanup stopped | 0 | 3 cumulative | [final-readonly.txt](final-readonly.txt), [host-health.jsonl](host-health.jsonl) |

MEASURED DONE: all isolated areas tested once; combined failure preserved; stage commits secret-scanned, pushed and origin-verified. MEASURED PARTIAL: final guest responsiveness proven but graphics recovered through a TDR; the VM is not in exactly its original application state. CODE READ/MEASURED NOT DONE: retry and cleanup withheld under the lane's explicit hang/crash **STOP, read-only collection** rule. The failed GravityMark PID **5080**, two Ready M2 tasks, and **C:\kf-m2** remain; do not mistake this for clean teardown. Owner VM remains up, QEMU PID **3878933**, run **3014**, **0d7ba743**, mirror **ON**, no restart/stop/kill-switch change or shared-lock action by M2.

MEASURED test counts before -> after: Windows area gates 0 -> **3 isolated PASS**; combined 0 -> **1 FAIL**; baseline 1 PASS, final responsiveness PASS / cleanup incomplete. CUDA n=1 pair, durations 60.211–60.215 s (two processes, not repeated trials), 16 buffer checks; Vulkan n=1 / 167.0 s; video n=1 per codec/decode; combined n=1. No repeat spread. Combined CPU final checks and valid 3900-frame encode were not reached. Unit tests/clippy and mutation kill table **N/A: no product behavior changed**, no cargo build/target created. Vulkan's standalone numeric child exit code was missed by the initial harness; complete scene/timing/normal shutdown are recorded instead. The wrapper-only pre-render launch stall was retried once; the real renderer ran once. See per-area reports for all limitations.

MEASURED host Xid total **21 -> 21** across the lane; host NVIDIA-SMI healthy. TDR XML explicitly says Resetting / Reset / Restarting TDR on GPUID:6, with unchanged Windows boot time. Host mirror/channel refusal counters remain 0; **9 CLEARED OVER ABSENCE** records and GSP-query refusal counts are disclosed, not hidden by the zero-Xid result. Guest and host wall clocks differ by about two hours; correlate host snapshots and ordered guest records, not absolute cross-clock timestamps. End display tick **59.97 Hz**, changing-desktop copies **24.9/s** in the final sample; no broker new-content gap percentiles, 1%/0.1% lows, repeated-present rate, or performance-parity claim was measured.

INFERRED decisions for owner:

1. **Do not accept this mirror build for mixed use or promote it**; recommend owner-controlled return to a known configuration pending investigation. Isolated passes do not override a TDR. Mirror-specific causality is unproved without a matched control.
2. Decide follow-up cleanup of own failed PID 5080, the two M2 tasks and C:\kf-m2; M2 left them after the read-only stop rather than hiding an incomplete cleanup.
3. Decide whether to authorize a recovered-guest retry/investigation; **Linux fat-guest CUDA/Vulkan/video gates remain UNRUN**. Historical legacy Linux int_gfx Xid 69 is cited in [baseline-result.md](baseline-result.md), never rerun; no matched Windows legacy combined result exists here.

MEASURED evidence retention: all 56 guest source/log/hash/image files saved by an in-memory read-only archive on the lane host; public artifacts are text/PNG only. The five executed scripts match committed sources byte for byte; CPU/GPU frame-hash files match byte for byte. Host transfer server stopped, no local build/target, no M2-held lock; **one failed guest process remains** as above. Raw host paths and final publication SHA are recorded in the private M2 handoff; none of the forbidden/public-private host paths were added to this branch.
