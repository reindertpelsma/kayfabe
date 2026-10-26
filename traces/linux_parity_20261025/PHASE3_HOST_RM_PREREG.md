# Supplementary host allocation capture

STATUS: PREREGISTERED before phase3 boot `16-K`, 2026-10-26; the exact pre-data receipt is retained privately with the launcher and supplemental protocol.

MEASURED 2026-10-26, `lx2-phase3-20261026/14-N` and `15-K`: the original matched surface traces are preserved, both valid. Their common physical colour/depth-format allocation fields match. The own-QEMU log is empty because the affinity wrapper opened the observer's exclusive file before target exec. Two CPU-only fixture checks reproduce this transport failure, then pass with target-only injection. This is not evidence of absent host allocations or an application/driver instability.

Plan: one fresh K boot `16-K`, unchanged original 1000-frame surface, pinned primary product `6564dd597`, mirror ON, identical image/kernel/595.91.07 userspace and 100 ms sampler. Enable only the unchanged open-header observer in the owned host QEMU after the affinity wrapper; no guest strace/preload, KINDSTAT or CUPTI. Preserve all previous files and exclude this diagnostic from every speed ratio.

Metric: the host-authored 8 GiB store's explicit allocation input/output attrs, format, size, alignment field and RM status. Falsifiers: missing/ambiguous payload or status, or returned compression capability contrary to the source-read NONE policy. Keep implicit/unknown payloads undecoded rather than guessing. No product mapping/allocation change or outcome-driven performance repetition. The lifetime lease, empty-slot preconditions and stop-on-Xid/OOM/instability rules remain unchanged.
