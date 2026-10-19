# CUDA: PASS

MEASURED: Python 3.13.13 (Blender 5.2 bundled interpreter), stock guest nvcuda.dll / NVIDIA 580.88, hand-written PTX 7.8 sm_86 and CUDA Driver API. Command per process: `python.exe -B -S C:\kf-m2\cuda_streams.py 60 <seed>`, seeds 12345/24690. Source and exact launch/CPU oracle: `cuda_streams.py`, `cuda.ps1`.

MEASURED: **two processes/contexts concurrently**, PIDs 12024 and 2172, each with **four nonblocking streams** and 1,048,576 uint32 outputs per stream. Both PIDs visible together in guest `nvidia-smi --query-compute-apps`. Both initial and final full-buffer CPU checks pass: **16 buffer comparisons / 16,777,216 integer comparisons total**, no differing element, both child exit codes **0**, wrapper result **0**. Durations 60.215 / 60.211 s, overlapping starts 0.596 ms apart; 551,936 / 949,376 kernel launches. These counts prove sustained submissions and correctness, not performance parity. n=1 concurrent pair, no repeat spread.

MEASURED: host Xid **21 -> 21**, delta **0**; guest System 4101/nvlddmkm **0**; boot-only reset req=done=1 unchanged; mirror/channel refusal counters **0**, no inspected non-status/non-lifecycle error/refusal/0x1f lines. Mirror groups 41 -> 43, ctx_created 46 -> 48, latejoin_skipped 5 -> 19; see `cuda-log-check.txt` and `host-health.jsonl`. Pre-existing unsupported GSP-query refusal total 1675 -> 1721, distinct set stayed 75.

CODE READ/MEASURED scope: two CUDA contexts and multiple channels/streams are covered; do not infer multiple subcontexts inside one CUDA TSG from two processes. Vulkan separately supplied that proof. This arithmetic kernel does not exercise the old local-memory SKEDCHECK05 kernel, UVM, full CUDA app matrix, or Linux fat guest. No product change or behavior mutation was made; mutation table N/A.
