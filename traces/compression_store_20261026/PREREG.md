# CMP1 E5 preregistration

STATUS: RESEARCH, 2026-10-26. Registered before any CMP1 GPU allocation or work.
Start 2026-10-26 04:54 UTC; deadline 10:54 UTC (six hours).
Base: 6564dd5977bbcd6aa947359b55b4aa3d9a5e92f4.

H-a: An unprivileged raw-RM client can obtain a contiguous 1 GiB compression-capable
NV01_MEMORY_LOCAL_USER allocation, requesting COMPR_ANY, generic-compressible format,
block-linear allocation attributes and PREFER_NO_ZBC. Falsifiers: allocation refused,
returned COMPR_NONE, returned uncompressed format, or no evidence of comptag backing.
GET_SURFACE_INFO and GET_SURFACE_COMPRESSION_COVERAGE status and fields are recorded;
raw-mode zero comptag offset alone is inconclusive. Record FB free before/after, attrs,
format, size, offset, alignment, and GPU/driver/GDM/process state under the host lock.

H-b: RM retains compressible kind overrides on this backing at both 64 KiB and 4 KiB.
Four maps: each page size crossed with generic-compressible and PITCH overrides.
Falsifiers: any map refusal, absent/invalid PTE, wrong page size, or a read-back kind
other than the requested kind. GET_PTE_INFO is read-only. Do not set PTEs directly.
A matching uncompressed backing is the downgrade control if the primary maps pass.

H-c: PREFER_NO_ZBC suppresses hardware ZBC even when mapping a ZBC-capable kind.
Clear 0 and all-ones through the compressible view; read both decoded and raw PITCH
aliases with GPU work and a real completion. Falsifiers: wrong decoded clear value,
or a uniform raw tile marker differing from the logical clear value. A CE-only fill
cannot certify the ROP/ZBC path unless a positive control establishes sensitivity.
If no valid ZBC producer/positive control is available, H-c is UNVERIFIED, never PASS.
Returned PREFER_ZBC contradicts the requested suppression and requires an owner question.

H-d: Fresh COMPR_ANY backing has no stale compression state. Compress a 64 KiB object,
free it after completion, allocate fresh backing, read decoded and PITCH aliases.
Falsifier per lane instruction: any nonzero read. Record whether the physical page is
actually reused; no reuse is inconclusive. SAME-PROCESS stale bytes do not independently
prove a cross-process leak (OWNER_RULINGS AM.1); do not turn that into such a claim.

H-bw: A GPU fill/blit of identical data through the compressed view improves bandwidth
or decreases memory-active time versus PITCH on the same memory. Falsifier: no difference
beyond run spread. Balanced order, at least six pairs if the correctness gates permit,
identical size/count/completions; record n, per-run values, clocks, power and sampler
limits. Timings alone cannot prove DRAM-byte reduction or ROP ZBC suppression.

Gates: no GPU operation without the shared host lock; no foreign QEMU is stopped.
Lock probe 04:55:06 UTC returned 1; next probe no earlier than 05:00:06 UTC.
On Xid/host instability, safety notice, or a failed correctness/safety gate: preserve
outputs and park; no retry by changing policy, privileges, kind or request semantics.
Task 2 remains conditional on compression-capable backing AND resolved safety gates.
Only GPU-free inspection/preparation is allowed while waiting for the host lock.
