# USER scan and private-ring accessors, 2026-10-12

STATUS: MEASURED GPU-free primitives; runtime relay remains disabled.

Source: containing commit on `codex/windows-matrix-20261011`. Previous confirmed
push `4df9c0095254504ce8007d202f258f39d05b4a92`; hardware results below apply ONLY
to that baseline revision, not this checkpoint's new primitives.

MEASURED local: `cargo test -p kf-trap -p kf-core -p kf-abi -p kf-rm -p kf-chan
-p kf-qemu`, with incremental/debug information disabled for disk space:
1899 passed, zero failed, exit0. New scanner has19 tests. Three additional tests
exercise production Storage<CpuView> accessors against real anonymous CPU
mappings, without opening NVIDIA or executing GPU work. Existing suite includes
USERD allocation lifetime, source mapping admission, migration ordering models,
native quota ownership and Loom dispatch interleavings. None substitutes for the
remaining runtime adapter and hardware validation.

Failing-first: header-only packet fragments caused an index-out-of-bounds panic
in two new tests. Both fail in `scanner-red.log`; retaining pending metadata
without reading a nonexistent datum fixes them. Scanner tests also exercise all
hardware fragmentation boundaries, modern NON_INC/ONE_INC state, exact trigger
and suffix ordering, unsupported software packet splits, invalid fields and
transactional refusal. Fetch tests cover wrap/backpressure, source generations,
duplicate/impossible GET and sequence exhaustion. These are descriptor receipts,
never engine-completion receipts or permission to unmap engine operands.

Independent review found four additional parser gates, now fixed and tested:
nonzero Legacy packets (the shared decoder sizes but collapses INC/NON_INC);
sub-device filters (hardware may suppress methods); modern reserved bit12
(outside supported framing, not a measured native fault); and hardware
subchannels0..4 (never CPU software dispatch). Final review found no remaining
blocker in GPU-free primitive scope. Own-space/source lifetime admission,
actual native ordering and independent registry validation remain external gates.

Private-ring tests preserve the original direct USER GP address and SYNC_WAIT,
exercise ring wrap, reject indirect/unsupported/oversized entries before writes,
bound trigger staging and the immutable witness, and verify that publishing PUT
does not forge GP_GET or Reference. `write_user_gp` has no runtime caller yet.

MEASURED trusted host172.22.1.20 baseline4df9c0095: gates9/9, gate10 PASS,
USER births12/12/refused0, rebuilt exact QEMU/raw client/fresh fast guest,
fast30/30/crash0/NOTRUN0/exit0. Five Xid31 records match the deliberate unmapped
read/write negative arms; no Xid32. Evidence: linux-gates/fast/xid logs here.
Start15:11:04UTC; exit15:15:49UTC. No QEMU remained; IOMMU group11 DMA-FQ.

MEASURED Linux desktop baseline4df9c0095: disposable child of the preserved
provisioned Cinnamon image; stock NVIDIA580.159.04; Cinnamon/Xorg present;
OpenGL renderer RTX4070, gears297/300/301frames per5seconds (~60FPS).
No host Xid, no QEMU after cleanup, group11 DMA-FQ. Guest startup logs contain
gpu_user_shared_data NOT_SUPPORTED assertions and one DRM head0 flip timeout:
classify PASS_WITH_WARNINGS for desktop/gears smoke, not a clean desktop gate
or render-hash parity. The executed runner initially printed PASS because its
oracle checked Xid/software rendering only; that missed the warnings. Exact
executed runner is retained separately; the reusable runner now reports them.

INFERRED implementation direction: scan ordinary USER commands from the first
submission; native USER privileges and own explicitly admitted address space
enforce access, not CPU scan/"private" naming. Forward original ordinary GP
references after guarded admission; stage/patch trigger segments only. A hostile
post-scan change must never authorize a host action; channel-owned registration
and migration operands require independent validation. No claim yet that channel
RC cannot affect another VM; containment testing remains open. No new Windows
run, GravityMark PASS, full matrix or compatibility-axis result.
