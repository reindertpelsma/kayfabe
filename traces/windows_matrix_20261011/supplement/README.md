# Offline Windows coverage additions

**STATUS: LIVE, 2026-10-11.** Packaging and GPU-free compilation only; GPU execution pending.

HF generation uses pinned Qwen2-0.5B-Instruct, original Linux prompt and greedy
64-token generation, full FP16 CUDA placement and exact generated-token digest.
[Model source](https://huggingface.co/Qwen/Qwen2-0.5B-Instruct).
Additional wheels are pinned/HTTP/hash verified; shared base wheel versions are unchanged.

GPU-recursive quicksort uses NVRTC relocatable PTX plus cudadevrt linking, a
[documented Windows route](https://docs.nvidia.com/cuda/archive/12.9.0/nvrtc/index.html#example-dynamic-parallelism).
Device-side child launch/error counters and exact sorted readback establish the
oracle; CPU sorting is used only to check the result. NVRTC12.9.86 Linux compiler
accepted the device source for compute89 with RDC, no warnings. This is not proof
of Windows linkage or GPU execution. The first compiler attempt could not find
its builtins; setting the compiler library directory in LD_LIBRARY_PATH repaired
the setup and compilation passed. No GPU host build competed with the matrix.

The addon ISO contains only the extra packages, guest scripts and tool overrides.
Its native test requires the previously verified base Python/wheels already staged.
A fresh kayfabe guest needs a full rebuilt disk containing both base and additions;
never attach this addon alone and claim missing base packages are product failures.
Existing frozen102-test campaign uses its original disk and harness unchanged.

Regenerate downloads with appdisk.py fetch --only hf_wheels,hf_model,cuda_nvrtc,cuda_cudart,
then build --only the same IDs. Full fresh-guest media must include all required
base packages. build-addon2.log and verify-addon2.log record the actual addon image.
