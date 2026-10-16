# Read-cost investigation: source facts before hardware measurement

STATUS: RESEARCH, 2026-10-16 02:40 CEST. Preparation only; no new hardware read-cost number yet.

Examined production baseline `0a746117` and the P2 shared-view build `c66ac9de`.

`VolatileRegion::copy_out` is a bounds-checked `copy_nonoverlapping` data copy, not an
explicit scalar volatile loop. Its actual generated load width therefore needs measurement.
`CachePolicy::Uncached` on DeviceFile is a declaration: the kernel driver's mmap handler
selects the effective attribute. Changing that Rust enum alone is not a cache-type experiment.

Primary source cloned read-only for derivation: NVIDIA open-gpu-kernel-modules tag 595.84,
commit `722ae84526a09ed672fbe75448e2909834ba4cce`. `nvos.h:1827-1832` defines the Linux
CPU-mapping cache hint in bits 25:23: CACHED=0, UNCACHED=1, WRITECOMBINED=2, WRITEBACK=5,
DEFAULT=6. `kernel-open/nvidia/nv-mmap.c:368-420` selects the page protection from the
mapping context's cache type. The probe uses explicit authored hints and records refusal;
its requested hint alone will not be described as a verified effective WC mapping.
The baseline `HostRm::arm_cpu_view_on` supplies access bits only (zero caching bits).

The opt-in `store-read-bench` feature compares current data copy, aligned 8-byte volatile
loads, 16-byte SSE2, 32-byte AVX, four independent vector loads, four separated streams,
and SSE4.1 MOVNTDQA. CPU features, every source/destination bound, alignment where needed,
and tails are checked in the unsafe perimeter. Exact output comparison follows every timed
read. Cases include 64/256/416/4096 bytes, aligned and unaligned starts. Default product
builds do not enable this feature. Hardware runs must execute without a VM under the GPU lock.

Why the approximately 867 copies/frame occur (source reading, not a fresh measurement):
`userpump.rs` snapshots each GP descriptor (8 bytes), then the entire admitted PB segment
before `stage_plan`. That function scans packet framing, bindings, subdevice masks and software
triggers. Ordinary entries are forwarded as guest GP pointers. A segment with an intercepted
trigger uses the validated private staged replacement. The diagnostic trace can also re-read
segments when explicitly enabled; it is not on the default path.

The scanner's `datum` uses payload values only for SET_OBJECT binding (method 0) and a software
trigger (0x200 on a software-bound subchannel); ordinary engine argument values are not needed
for host action authorization. Packet form/count and method-overflow checks still apply to all
packets, including continuations across GP entries and subdevice-mask state. This suggests a
header/binding/trigger-only accessor, but replacing the whole snapshot requires preserving
placement receipts, retry atomicity, parser history, SYNC_WAIT gates, rare-segment staging and
free/reuse admission. No argument bytes have been skipped by this branch. No lagging scan or
unvalidated guest-byte-derived host action is introduced.

Cheap subsequent CPU-only candidates: avoid allocating/copying `patched` for an ordinary
segment (stage_plan currently copies every Vec before knowing rare=false); avoid rescanning
an ordinary immutable snapshot twice. These save RAM/CPU work, not the measured BAR1 bytes.
They require transaction/parser tests and per-stage timing before a speedup claim.
