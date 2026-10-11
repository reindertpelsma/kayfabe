# Per-thread CUDA stream benchmark startup correction

**STATUS: RESEARCH, 2026-10-11.** Bench-only change, hardware pending.
The original counterpart imported Torch and CuPy and mixed their arrays/streams.
Run1262 measures82.5s Torch startup and93.5s CuPy import. The Linux row requires
per-thread CUDA stream execution, not interoperability between two Python stacks.

`stream_perthread.py` adds the existing bundled CUDA DLL directory directly,
loads CuPy, checks the actual NVIDIA CUDA device and makes Stream.ptds current.
It computes1Mi float32 values and multiply-by-two on the GPU, synchronizes that
stream, copies back, and checks every element against independent NumPy values.
No CPU fallback. The original120s deadline, packages, pass marker and GPU proof
requirements remain. Old timeout evidence is retained separately.

Public CuPy14.2 Stream.ptds/context/synchronize contract:
https://docs.cupy.dev/en/stable/reference/generated/cupy.cuda.Stream.html
This is the documented per-thread default stream, not a newly created substitute.
The old shared Torch script remains available for historical reproduction.

Hypothesis: removing unrelated Torch startup lets the same CUDA stream purpose
complete within120s. Falsifier: corrected stock or kayfabe run fails numerical/GPU
proof or still times out. Hardware comparison pending; no supported pass yet.
GPU-free: Python syntax check,76 existing harness tests,11 inventory/doc tests PASS.
