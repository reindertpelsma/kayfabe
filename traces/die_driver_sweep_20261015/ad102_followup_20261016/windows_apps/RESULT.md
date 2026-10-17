# AD102 Windows580 short app acceptance

**STATUS: LIVE, 2026-10-16 18:44 UTC. MEASURED PASS; this is one actual Windows build.**

Owned instance56267214, physical AD102/RTX4090, host595.91.07. Actual guest
Windows580.88/32.0.15.8088, Code0. QEMU110767 runs private source
77a72da9eb4d3f4e694fbb449d75e5fe3926b28c, binary SHA256
e9744a23ab38ff45afbf788d9e426d71913d1f1e6954fa1ea0da1ea1920b25a6,
public guest ABI580.65.06. Product crates/qemu/cuda are unchanged from the
7d1be8882 production seven-minute hold; later commits change bench/docs only.
The actual process receipt and flags.txt record all19production flags, RPC
diagnostics off. Display/GOP on, standard VGA off. This is nested kayfabe,
not the native/VFIO reference. The separate421second hold is in ../windows580/.

H-A5 (INFERRED, acceptance falsifier): after native Blender CUDA passes, this
guest also renders using CUDA and runs hardware WebGL without corrupt pixels,
TDR, or breaking CUDA/D3D afterward. Any application failure, software fallback,
pixel mismatch, System4101 event, or failed survivor probe falsifies it.
MEASURED result: falsifier did not fire in the completed18:35:12..18:38:56 run.

| Check | MEASURED result | Evidence |
|---|---|---|
|Before/after adapter and desktop|32.0.15.8088, Code0, DWM and Explorer|adaref2_persistent_apps.log|
|Before/after CUDA|cup2/3/8 pass, including byte-exact2048x2048matmul|same log|
|Before/after graphics|hardware NVIDIA D3D11+12, compute/readback/fences pass|same log|
|Blender4.5 CUDA|exit0; NVIDIA RTX4090; mean0.6405;532493byte PNG SHA9be7843c543aa4cf162b600cabbe1364724d5c8b9bc5e629fb8b156a8f918428|same log|
|Edge short|WebGL2 via NVIDIA ANGLE/D3D11; software=false; pixel64,127,191,255;722frames/12.0128seconds; glerr0|same log, KFEDGE JSON|
|TDR|readable System4101 log;0during apps and0since boot afterward|same log|
|Host Xid|5before/5after, delta0; earlier5belong to deliberate Linux raw fault arms|post_stop_metadata.json|
|Owned teardown|QGA shutdown, QEMU absent, PID marker removed; no forced kill|adaref2_stop_77a72da9.log; post_stop_metadata.json|

Native Blender4.5 CUDA previously passed on this same stock595 board, mean0.6405
(../native_blender/). Matching means are observations, not image parity: native
and Windows PNG sizes differ. Images and executable packages stayed on rental;
only text/hash receipts returned. Production QEMU log is1914818bytes; SHA in
post_stop_metadata.json, no raw bodies exported. The rental was subsequently
destroyed; unsaved raw files are no longer available.

Harness negatives remain explicit. The first Windows render actually produced
BLENDER_OK CUDA and a PNG, but PowerShell's Process.ExitCode was null; the
supervisor incorrectly failed and Edge did not run. Caching the process handle
before WaitForExit fixes that receipt. Another attempt failed in the legacy
QGA client's initial synchronization before running a GPU program. Bulk file
staging also stopped at chunk13's read-only sync; the bounded file helper now
reconnects only before opening a file. These are not Code43/GPU failures.
The successful persistent client launches each guest program exactly once,
reconnecting only while reading status for the same PID. It has a16MiB reply
bound; it is a bench recipe, not a product or generic-client fix.

All96bounded Blender chunks reassembled to the vendor ZIP SHA
2ee75e9466d293a784fdf020f60fe1309c1e0610ecf73c64f1fc09b01e5eec56.
Reproduction scripts and CPU file-staging regression logs are archived here.
Edge long-run, S3, and OptiX huge tiles were not run. No family-complete claim.
