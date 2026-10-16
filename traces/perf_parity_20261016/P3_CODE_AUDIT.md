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

Offline probe ordering update (02:55 CEST): `Stream16` includes MFENCE before and after
its read batch. Initialization also uses MFENCE rather than a release-only fence. The probe
changes the first cache line 128 times and checks each available strategy after every CPU
write. This checks CPU-writer freshness; it does not establish GPU-writer coherence.
Intel documents the WC temporary streaming buffers and ordering requirements in the
[MOVNTDQA entry of the Intel SDM, volume 2](https://cdrdv2-public.intel.com/671110/325383-sdm-vol-2abcd.pdf).
The fences are part of the timed candidate, and the entire addition remains feature gated.

The optional `--gpu-freshness` control maps the probe's private 64 KiB allocation in
its own GPU VA space and uses the existing USER copy-engine HostRing. The engine copies
changing source patterns to the destination, then writes its real RELEASE_WFI fence.
Current copy and vector candidates are compared against the expected bytes for
64/256/416/4096-byte aligned and unaligned reads. The source receives a sysmembar and
VIDEO_MEMORY invalidation using the public ctrl2080fb.h aperture definition (595.84).
Those control calls are outside the timed reads. A fence timeout or baseline-copy mismatch
invalidates the GPU control; a candidate byte mismatch rejects that candidate.
The probe compiles, but this GPU control has not yet run. No production reader uses it.

Remaining measurement scope: record host clocks during the bare-host probe; idle P-state
latencies must not be silently substituted for the earlier Heaven-loaded measurements.
The later per-frame table needs new view-copy timing counters on its own built source;
the approximately 867 reads / 232 KiB / 3.7 ms values remain inherited 0a746117 evidence.
