# Matching-driver stock Windows reference: remaining apps

**STATUS: ANSWERED, 2026-10-11.**15/15 PASS, zero guest driver/reset/WER records.
RTX4090, stock Windows guest NVIDIA580.88, own Vast55385171. Harness frozen at
582d72dc; overrides in guests/g1/tool-overrides.json. Native GSP cycles remain
unmeasured (`-`), never interpret this as zero. No kayfabe/VFIO parity inference
from matching driver alone: the trusted kayfabe host has RTX4070 and595.91.07.

Vulkan cube, llama CUDA generate/bench and Vulkan bench, dxdiag, Blender Cycles,
vkpeak, GravityMark D3D12 raster/RT/default, OpenGL and D3D11, Torch AI bench,
OpenCL clpeak and Torch burn pass their original oracles. OpenCL clpeak775s,
inside its existing1500s budget; its benign missing temporary dep-file warning
is retained. All original output, PDH proof, health and screenshots are saved.
No recovery records. Timings are per-box and are not cross-GPU parity claims.

Run finished16:24:51UTC and was copied to the controller before this checkpoint.
The next50 previously unmeasured matching-driver canonical cases are a separate
serial run; they are pending, not counted as passes here. GravityMark Vulkan's
previous stock TIMEOUT remains a separate reference failure.
