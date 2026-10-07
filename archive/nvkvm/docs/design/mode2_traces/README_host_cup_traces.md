# Host-side ioctl traces of the `cup` CUDA ladder — GA106, driver 580.159.03

Captured **2026-08-09** on a rented vast.ai box (instance `47225865`, since destroyed):
**NVIDIA GeForce RTX 3060, PCI device id `0x2503` = GA106**, `Product Architecture: Ampere`,
VBIOS `94.06.14.80.B7`, **driver / `libcuda` 580.159.03**, kernel `6.8.0-117-generic`,
container image `nvidia/cuda:12.4.1-devel-ubuntu22.04`. Native host — **no VM, no QEMU, no guest.**

⚠ Every claim below is scoped to **driver 580.159.03**. The vendored open-kernel trees are
`ogkm-580.159.04` and `ogkm` = `610.43.02`; this is a *third* version, one patch below the
`580.159.04` tree.

## Exact commands

    gcc -shared -fPIC -O2 -o nvioctl_trace.so nvioctl_trace.c -ldl   # scripts/mode2_diag/nvioctl_trace.c
    gcc -O2 -I/usr/local/cuda/include -o cupN cupN.c -lcuda -lm      # tests/mode2/cup{2,4,8}.c

    NVALLOC=64 NVOUTER=64 NVCONTENT=48 \
      NVTRACE=host_cupN_trace.txt LD_PRELOAD=./nvioctl_trace.so ./cupN

★ **`nvcc` is NOT required** — verified: `cup4`/`cup8` are driver-API + inline PTX
(`cuModuleLoadData`), so `gcc` + `-lcuda` builds and runs them. Confirmed on this box.

★ The tracer that produces this format is **`scripts/mode2_diag/nvioctl_trace.c`** (an
`LD_PRELOAD` shim), *not* `tools/diag/nvioctl_trace.c` (a ptrace tool emitting a different
format). ⊘ The `LD_PRELOAD` shim filters on **ioctl type `0x46`** — it does **not** filter on the
fd path, and it is therefore **blind to the `/dev/nvidia-uvm` plane**, whose ioctls are type `0`.
`host_cup_ioctl_census.txt` closes that hole with a full `strace` census.

## Files

| file | records | program | result |
|---|---|---|---|
| `host_cup2_trace.txt` | 316 | `cup2` (pre-existing, ~2026-06-07, different box) | control reference |
| `host_cup2_trace_ga106_580.159.03.txt` | 392 | `cup2` — `cuInit→cuCtxCreate→cuMemAlloc→CE memcpy` | `rc=0`, `CE rv=0xabcd1234 PASS` |
| `host_cup4_trace.txt` | 391 | `cup4` — N=16 matmul, 1 block, **real GR kernel launch** | `rc=0`, `bad=0 maxerr=0 PASS` |
| `host_cup8_trace.txt` | 393 | `cup8` — N=2048 matmul, 128×128 grid, 48 MiB device | `rc=0`, `bad=0 maxerr=0 PASS` |
| `host_cup_ioctl_census.txt` | — | all three, via `strace` | all-plane (incl. UVM) census |

Exit codes read with `set +e; ./cupN; rc=$?` — never through a pipe.

## 1. The `cup2` control REPRODUCES

The 2026-08-09 `cup2` capture reproduces `host_cup2_trace.txt` **exactly on the alloc plane**:
100 `NV_ESC_RM_ALLOC` records, the same **18 distinct classes in the same first-appearance
order with identical multiplicities**:

    0x0000 x2, 0xcb33 x1, 0x0080 x1, 0x2080 x1, 0x2081 x1, 0xc461 x1, 0x00de x1,
    0x90f1 x2, 0x50a0 x1, 0x0040 x15, 0xa06c x3, 0x9067 x1, 0x003e x22,
    0xc56f x16, 0xc7c0 x8, 0xc7b5 x16, 0x0079 x7, 0x83de x1

The 316 → 392 record difference is **entirely machine topology, not driver drift**: the 2026-08-09
box exposes `/dev/nvidia0..7` (8 GPUs; `nvidia-smi`/`cuDeviceGetCount` show 1), so `libcuda`
enumerates 7 extra devices. Every single delta is in the `NV01_ROOT` GPU-enumeration family:

| ctrl | committed | 2026-08-09 | Δ |
|---|---|---|---|
| `0x00000201` | 2 | 9 | +7 |
| `0x00000202` | 3 | 44 | +41 |
| `0x00000205` | 2 | 9 | +7 |
| `0x00000215` | 1 | 8 | +7 |
| `0x00000216` | 0 | 7 | +7 |
| `NV_ESC_RM_FREE` (`nr=0x29`) | 2 | 9 | +7 |

⇒ **zero** deltas anywhere in the device / subdevice / VAS / channel / GR plane. Control passes.

## 2. ★★★ THE RESULT: a kernel launch adds NOTHING at ioctl granularity

Multiset diff of `(ALLOC class | CTRL cmd | IOCTL nr)` against the same-box `cup2`:

| | `cup4` vs `cup2` | `cup8` vs `cup2` |
|---|---|---|
| **alloc classes present only in cup4/cup8** | **NONE** | **NONE** |
| **controls present only in cup4/cup8** | **NONE** | **NONE** |
| `0x0040 NV01_MEMORY_LOCAL_USER` | 15 (same) | **17 (+2)** |
| `0x20800110 GPU_GET_NAME_STRING` | 1 (**−1**) | 1 (**−1**) |

The only differences are *fewer* records (`cup2` calls `cuDeviceGetName`, `cup4`/`cup8` do not)
and *more device memory* (`cup8` allocates 3 × 16 MiB instead of 1 × 4 KiB). Deterministic:
3/3 repeat runs of each gave byte-identical record counts.

**Specifically refuted, all three targets named in the brief:**

| expectation | measured, all of cup2/cup4/cup8, and the committed 2026-06 trace |
|---|---|
| `0xc797 AMPERE_B` appears on launch | **0 occurrences anywhere** — a compute-only CUDA process never allocates the 3D class |
| a **second** `0x83de GT200_DEBUGGER` on launch | **exactly 1**, in all four traces |
| new `0x2080_12xx` GR-family controls on launch | **16 records, 7 distinct** (`…1201 …1210 …1218 …121b …1227 …122a …122b`) — *identical* in all four traces |
| `0xc7c0 AMPERE_COMPUTE_B` is launch-time | **8 allocations, already in `cup2`** — allocated by `cuCtxCreate` |

### Where the ioctls actually are

From `host_cup_ioctl_census.txt` (`strace -e trace=ioctl,write`, running ioctl count at each
CUDA call's return):

| CUDA call | ioctls it costs (cup4) | (cup8) |
|---|---|---|
| `cuInit` | 170 | 170 |
| `cuDeviceGetCount` / `cuDeviceGet` | 0 | 0 |
| **`cuCtxCreate`** | **302** | **302** |
| `cuModuleLoadData` (PTX JIT + upload) | **0** | **0** |
| `cuModuleGetFunction` | **0** | **0** |
| `cuMemAlloc` | 3 (first only; 1 KiB) | 3 each (16 MiB each) |
| `cuMemcpyHtoD` / `cuMemsetD32` | 0 | 0 |
| **`cuLaunchKernel`** | **0** | **0** |
| **`cuCtxSynchronize`** | **0** | **0** |
| `cuMemcpyDtoH` (reads the result) | 0 | 0 |

⇒ **`cuCtxCreate` provisions everything a launch needs.** Module load is a userspace JIT plus a
pushbuffer/CE upload into already-allocated memory; the launch itself is pure pushbuffer + doorbell.

### The UVM plane is the same story

`/dev/nvidia-uvm` ioctls are type `0` and invisible to the `LD_PRELOAD` tracer. `strace` census:

| UVM ioctl | cup2 | cup4 | cup8 |
|---|---|---|---|
| `UVM_CREATE_EXTERNAL_RANGE` (73) | 25 | 25 | **27** |
| `UVM_MAP_EXTERNAL_ALLOCATION` (33) | 25 | 25 | **27** |
| `UVM_REGISTER_CHANNEL` (27) | 16 | 16 | 16 |
| `UVM_CREATE_RANGE_GROUP` (23) | 8 | 8 | 8 |

`cup4` adds **nothing**; `cup8`'s `+2/+2` tracks its two extra large allocations (`cup4`'s three
1 KiB buffers are suballocated from an existing pool and cost no new range).

## 3. What this means for kayfabe

`docs/design/mode2_traces/host_cup2_trace.txt` was **already a complete ioctl-plane oracle for a
CUDA kernel launch.** There is no missing alloc/control set to discover; `UNCITED-3` in
`alloc_rpc_plane_map.md` §1 ("no ioctl-granularity trace of cup4/cup8 exists") is now closed, and
the answer is that the trace would have been the same one.

⊘ **Scope, so this is not over-read.** Measured for: one process, one context, one module, one
stream, first launch, no `cuMemAllocManaged`, no graphs, no multi-GPU, no cuBLAS/cuDNN, GA106,
driver 580.159.03. A *second* context, a second process, managed memory, or a fault-replay path may
well add ioctls — none of those are in this ladder. What is settled is that **`cup4`/`cup8` do not**.
