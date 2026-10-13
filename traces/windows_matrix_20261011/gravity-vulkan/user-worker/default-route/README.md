# Default Windows USER FIFO migration route

**STATUS: RESEARCH, 2026-10-13.** Plan written before the hardware run.

Run1630 at3ee0dd10a passed original GravityMark Vulkan181s/rc0 with two FIFO
source growth transitions, no hostXid/guest153/4101/WER/recovery/reboot, original
image/timing/GPU proof. GPU-free2156, Linux9+10/30 and desktop smoke passed.

This source enables the reviewed worker route at birth for every eligible
Windows USER channel by default. Linux lacks the Windows mapping permit and
keeps ordinary Passthrough. Explicit KF3_WIN_USER_RING_RELAY=0 is retained as
the diagnostic negative control; shadow remains an explicit separate diagnostic.
No application names, ring sizes or captured die values select the product route.
The private native/scanner identity and actual consumption-ordered migration
remain as in the passing run; no native deferred-host caller is added.

H-DEFAULT: same Linux hardware bar and original Windows desktop/H264/D3D12/
CUDA-readback/Vulkan oracles pass without KF3_WIN_USER_RING_RELAY=1. Falsifier:
any Linux failure, regression relative to prior Windows app evidence, wrong
output, hostXid or guest driver recovery. D3D12's prior failure remains history;
no new pass follows from source inspection. No host build during timed guests.
Original pinned executables/media and budgets, including Vulkan600s, unchanged.
