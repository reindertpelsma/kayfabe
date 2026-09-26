# V3 app matrix — which real CUDA apps work in a kayfabe v3 fat guest

**STATUS: LIVE, 2026-09-26 — current result is §R2 (kayfabe `670bd310`, measured 05:00–07:30 UTC):
58/65 apps work (was 35/65).** §0–§5 below are the first measurement at `79848341`, kept unchanged
as the baseline R2 is compared against; their cause list is SUPERSEDED by §R2.3 (A, B, D, E, F fixed).

## R2 — re-run at kayfabe `670bd310` (2026-09-26)

Since `79848341`, master gained: one host TSG per guest context share (the 2nd-CUDA-stream fix),
pooled/host-managed walker capacity (the no-PM multi-process fix), NVENC/NVDEC, headless
Vulkan/EGL, the GSP reopen fix, and batched host maps. Same app set, same predicates (two
tightened, §R2.4), same harness.

**Box:** vast 52624429 (`vh`), **RTX 3060 12 GB (GA106)**, AMD EPYC 7452, host driver 580.159.04
(kernel-open); kf3 binary `/workspace/bench/kf3-bins/670bd310/qemu-system-x86_64` (boot_capture
stamp `kf3-bin-rev:670bd310`); guest = a **copy** of the box's LLM fat guest (`guest_apps.qcow2`,
Ubuntu 24.04, kernel 6.8.0-139, stock 580.159.04) provisioned with `provision_guest_apps.sh`, so the
LLM guest stayed untouched; `fb-mb=8192`, 16 GiB RAM, 6 vCPUs. The bundle was rebuilt on this box
(`build_bundle.sh`, bundle sha `4ec119b39ec2395e`, llama.cpp pinned to `4b1a27f`). No RTX 3070 this
round — cause I (3070-only) is not re-measured.

### R2.0 Headline

- **Host (bare metal, same box): 71/71 PASS** (`h1`; `h2` re-ran clpeak + llama_cpp_gen under the
  tightened predicates: PASS).
- **Guest: 58/65 apps PASS, and all 6 stream probes PASS** — without persistence mode, several apps
  per boot. **With persistence mode: the identical 58/65 and the identical 7 failures** (`pm2`).
- **Cause A (non-default streams, Xid 13 SKEDCHECK05) is gone**: all 21 apps it blocked and all 5
  non-default `stream_probe` shapes pass — PyTorch (`torch_correct`, CNN-train digest = host),
  llama.cpp gen (token-identical to host) + bench, CuPy, hashcat, Blender CUDA+OptiX, Geekbench,
  every CUDA-graph / multi-stream sample.
- **NVENC/NVDEC, EGL, Vulkan now work**: nvenc_h264/hevc, nvdec_h264, egl_offscreen, vulkaninfo,
  vkpeak (every vkpeak figure within 1 % of bare metal).
- **No-PM multi-app boots now work** (§R2.2): 39 CUDA processes in one boot without PM (was 5–6).
  The budget that remains is a host `NoMemory` at the ~40th process (no PM) / ~60th (PM) — cause J.
- **What is left: one family** — a host **Xid 31 MMU fault on the guest's GR work** (5 apps
  `FAULT_PDE`, 1 app `FAULT_PTE`), incl. the known **silent wrong answer** in `conjugateGradientUM`
  — plus the known `gpu_burn` SIGSEGV. `clpeak` moved to FAIL: it was a **false PASS** of the old
  predicate (§R2.4), and it shows the fault is **not CUDA-managed-memory-only** (OpenCL hits it).
- Digests host vs guest: `torch_correct` cnn_train_step `763c693a5a53f948` ==, `hf_generate`
  `0d973108a6251e14` == (also == the 79848341 run), `llama_cpp_gen` generated text
  `caf613a5d956b7e3` == (pm2; nb2/iso1 compared by text, token-identical).

### R2.1 Results

Runs (results under `traces/v3_app_matrix/vh_rtx3060_670bd310/<run>/`, `triage.txt` = one evidence
line per app from `scripts/apps/triage.py`):
- `nb1` — no PM, **all 71 rows in one boot**. Stopped by hand after the boot wedged at
  `UnifiedMemoryStreams` (row 18; see C′) — the harness then had no wedge detection (added, §R2.4).
- `nb2` — no PM, rows 19–71 batched (`conjugateGradientUM` onward) with the wedge probe: 2 boots.
- `iso1` — every non-PASS app alone in a fresh boot (the verdict of record for failures).
- `seq2` — no PM, 100× `vectorAdd` in one boot (the process budget).
- `pm2` — guest PM on, all 71 rows batched with the wedge probe: 2 boots.

A row PASSes if it passed in a batched boot (a pass in a shared boot is still a pass); a failure is
confirmed alone (`iso1`). Old = the 79848341 3060 column of §2.

| app | host | guest no-PM (batched) | guest alone | guest PM (batched) | old 3060 | failure point / evidence |
|---|---|---|---|---|---|---|
| nvidia_smi … simpleIPC (16 rows¹) | PASS | PASS | - | PASS | 9 PASS, 7 A | ¹ nvidia_smi, deviceQuery, vectorAdd, vectorAddDrv, matrixMul, matrixMulDrv, bandwidthTest, simpleStreams, asyncAPI, simpleAtomicIntrinsics, simpleCallback, simpleOccupancy, simpleZeroCopy, simpleCooperativeGroups, concurrentKernels, simpleIPC |
| UnifiedMemoryStreams | PASS | TIMEOUT (wedge) | **TIMEOUT (C′)** | FAIL 719 | FAIL (A) | host `Xid 31 … GPC1 … @0x7c70_d8489000 FAULT_PTE VIRT_READ`; kf3 `REFUSED VasKey(..) root 0x201000: 1 run(s) not applied: map 0x7c70d8400000+0x10000: Other(31)` (RM `NV_ERR_INVALID_ARGUMENT`) + `split ticket REFUSED`; no PM ⇒ the boot is **wedged** afterwards (sanity vectorAdd fails) |
| UnifiedMemoryPerf | PASS | TIMEOUT (after the wedge) | **FAIL (C)** | FAIL | FAIL (A) | `matrixMultiplyPerf.cu:435 code=719 cudaStreamSynchronize`; host `Xid 31 … @0x7c7d_22000000 FAULT_PDE VIRT_READ` |
| conjugateGradientUM | PASS | FAIL | **FAIL (C)** | FAIL | FAIL (C) | ⊘ **silent wrong answer**: `Error amount = 1.000000, result = SUCCESS`; host `Xid 31 … @0x791b_b6200000 FAULT_PDE VIRT_READ` |
| cudaTensorCoreGemm … memcpy2d (27 rows²) | PASS | PASS | - | PASS | 23 PASS, 4 A | ² incl. globalToShmemAsyncCopy, graphMemoryNodes, simpleCudaGraphs, MersenneTwisterGP11213 (were A), all nvkvm-pv realapp kernels |
| attach_verify | PASS | FAIL | **FAIL (C)** | FAIL | FAIL (A) | `cudaDeviceSynchronize() -> 719`, the buffers verified `0 mismatched` first; host `Xid 31 … @0x7280_a8007000 FAULT_PDE VIRT_READ` |
| stream_default … stream_created2nd (6 probes) | PASS | PASS | - | PASS | 1 PASS, 5 A | every stream shape, incl. `created2nd` |
| gpu_burn | PASS | FAIL | **FAIL (G)** | FAIL | FAIL (G) | SIGSEGV right after `cuInit returned 0` (guest `segfault at 7ffc83508000 … error 4 in gpu_burn`); no Xid, no kf3 refusal — known, branch `v3-appfix` |
| torch_correct | PASS | PASS | - | PASS | FAIL (A) | digest == host |
| torch_ai_bench | PASS | FAIL (J: no CUDA) | **FAIL (C)** | TIMEOUT (J) | FAIL (C) | alone: `RuntimeError: CUDA error: unspecified launch failure`, host `Xid 31 … @0x79af_c2127000 FAULT_PDE VIRT_WRITE`; in nb2 it was the ~41st process of the boot: kf3 `act birth translated REFUSED (0x56): USERD view of store 0xc0000: NoMemory` ⇒ `torch.cuda.is_available()` False; in pm2 the ~60th: `NV_ESC_RM_MAP_MEMORY … NoMemory` |
| hf_generate, cupy, llama_cpp_gen, llama_bench | PASS | PASS | - | PASS | 1 PASS, 2 B, 1 A | digests == host |
| vulkaninfo, vkpeak, egl_offscreen | PASS | PASS | - | PASS | F, F, E | vkpeak fp32 9335 vs host 9275 GFLOPS, fp16-matrix 55690 vs 55538 |
| clinfo | PASS | PASS | - | PASS | PASS | |
| clpeak | PASS | PASS (old predicate) | **FAIL (C)** | FAIL | PASS (old predicate) | ⊘ false PASS: GPU integer, integer-24bit, transfer and launch-latency groups `clFinish (-36)` → `Tests skipped`; host `Xid 31 … @0x772b_de422000 FAULT_PDE VIRT_WRITE`. Host: 0 skipped |
| nvenc_h264, nvenc_hevc, nvdec_h264 | PASS | PASS | - | PASS | D | 600 frames each, NVDEC used (no software fallback) |
| hashcat, blender_cycles, geekbench_gpu | PASS | PASS | - | PASS | A | hashcat cracks; Blender CUDA+OptiX `mean=0.6405` == host; GB6 OpenCL completes (`internal code 35` is printed on the host too) |

**Totals (65 apps, probes excluded): host 65/65; guest 58/65 — no PM and PM alike** (was 35/65 on
the 3060 at `79848341`; 34/65 on the 3070). Apples-to-apples with the old, weaker clpeak predicate:
59/65. Stream probes 6/6 (was 1/6).

### R2.2 No-PM multi-app boots — they hold now, up to ~40 CUDA processes

- `seq2` (no PM, 100× `vectorAdd`, one boot): **39 PASS**, the 40th hangs: kf3
  `REFUSED VasKey(18446744069414584321) root 0x1f1cac000: 1 run(s) not applied: window map
  0x110000+0x200000 (store @0x1000000): arm: NV_ESC_RM_MAP_MEMORY store@0x1000000+0x200000: NoMemory`
  — the **same** signature as §3 J; the boot is then wedged (sanity vectorAdd fails). At `79848341`
  the same experiment stopped after **5–6** (cause B, `slot N is full`): **B is gone** —
  `slot … is full` appears in no log of this round.
- `nb2` boot 1 ran **37 apps** (CUDA samples, cuBLAS/cuFFT, the realapp kernels, torch_correct,
  gpu_burn's crash, two Xid-31 faults) with every verdict equal to its isolated verdict; the 38th row
  (`torch_ai_bench`, the ~41st CUDA process counting 3 wedge probes) failed `cuInit` on a second
  NoMemory spelling (`USERD view of store 0xc0000: NoMemory`).
  `nb2` boot 2 ran the remaining 15 rows (LLMs, Vulkan, EGL, OpenCL, video, hashcat, Blender,
  Geekbench) — all PASS except clpeak (C).
- With PM (`pm2`) the budget is ~60 processes (J at `torch_ai_bench`, the 56th row + 5 wedge
  probes), matching §3 J's ~60th at `79848341`. ⇒ **J is now THE per-boot process budget** in both
  modes; it is the "~60th-process host OOM leak" already being worked on (branch `v3-appfix`).
- ⊘ One no-PM-only hazard: after `UnifiedMemoryStreams`' fault (C′) the no-PM boot is **wedged**
  (nb1: the next two apps hung silently, then a guest kernel channel `host 0x1001a
  REFUSED-AND-POISONED (§7)`; iso1: the sanity vectorAdd hung). With PM the same app fails fast with
  719 and the boot carries on.

### R2.3 Causes, ranked by apps blocked (670bd310)

| rank | cause | apps (of 65) | status vs 79848341 | signature |
|---|---|---|---|---|
| 1 | **C — host Xid 31 `FAULT_PDE` on the guest's GR work** | **5**: conjugateGradientUM (**silent wrong answer**), attach_verify, UnifiedMemoryPerf, torch_ai_bench, clpeak | was 2 (most were masked by A) — known, `v3-appfix` | host `Xid 31, MMU Fault: ENGINE GRAPHICS GPCn … FAULT_PDE ACCESS_TYPE_VIRT_READ/WRITE` at a user VA (`0x72xx…0x7exx`); kf3 `RC host twin … except_type=0x1f (Xid 31) — forwarding RC_TRIGGERED` on every channel of the context; no kf3 refusal precedes it. ⚠ **clpeak is OpenCL** — the class is not CUDA-managed-memory-only |
| 2 | **C′ — `FAULT_PTE` behind a refused host map** | 1: UnifiedMemoryStreams | new signature (was masked by A) | kf3 `1 run(s) not applied: map <va>+0x10000: Other(31)` (`NV_ERR_INVALID_ARGUMENT`) then host `Xid 31 … FAULT_PTE VIRT_READ` at/near that VA (nb1: map `0x7111f8600000+0x10000`, fault `0x7111_f8603000` — inside it); no PM ⇒ boot wedged afterwards |
| 3 | G — gpu_burn SIGSEGV | 1 | unchanged — known, `v3-appfix` | segfault just after `cuInit`; no Xid, no refusal |
| — | **J — host `NoMemory` process budget** | 0 alone; every app past the ~40th (no PM) / ~60th (PM) process of a boot | **now the only per-boot budget** (B gone) — known, `v3-appfix` | `NV_ESC_RM_MAP_MEMORY store@0x1000000+0x200000: NoMemory`, or `act birth … USERD view of store 0xc0000: NoMemory` |
| ✔ | A — non-default streams (Xid 13 SKEDCHECK05) | 0 (was 21 + 5 probes) | **FIXED** | 0 host `Xid 13` in the 13 captured host-dmesg windows (the same windows hold every Xid 31 of §R2.1, so the capture works); 0 `SKEDCHECK` in any per-app kf3 log |
| ✔ | B — walker slot exhaustion | 0 (was 5 + the per-boot budget) | **FIXED** | 0 `is full and nothing can be retired` in any per-app kf3 log (the same logs carry the `REFUSED VasKey` lines of J, so they capture `mem` refusals) |
| ✔ | D — video engines | 0 (was 3) | **FIXED** | |
| ✔ | E — EGL / F — Vulkan | 0 (was 1 / 2) | **FIXED** | vulkaninfo returns in 2 s; 0 `scrubberDestruct` in any per-app guest dmesg |
| ? | I — walk beyond the FB span (`fb-mb=6144`, 3070 only) | – | not re-measured (no 3070 box) | |

### R2.4 Harness changes in this round (branch `v3-apps2`, scripts only)

- **Wedge probe** (`apps_hook.sh`): after any non-PASS row, run `vectorAdd` (60 s); if it fails,
  write `APPS_WEDGE` and end the boot — `apps_matrix.sh` reboots and continues with the rest. Without
  it, nb1 burned every later app's full timeout after one wedge.
- **clpeak predicate**: FAIL on `clFinish (-N)` / `Tests skipped` (its bandwidth line — the old
  predicate — prints even when the compute groups abort).
- **llama_cpp_gen digest**: over the generated text only. This llama.cpp rev logs `VRAM: <n> MiB`
  (7871 in the guest vs 11909 on the host) and timings between tokens, so three different digests
  came out of one token-identical output.
- **Bench lock**: `apps_matrix.sh` takes `/tmp/kayfabe-fastguest.lock` per boot (and for the host
  run) and releases it between boots.
- **`KF_GUEST_IMG`** (`boot_nvkvm.sh`, `provision_guest_apps.sh`): run on a copy of the guest image so
  another lane's guest (here the LLM guest) stays intact.
- `build_bundle.sh` pins llama.cpp to `4b1a27f` (it cloned HEAD).
- `triage.py`: per-app evidence line (verdict, guest Xid, first kf3 RC line, first non-baseline kf3
  refusal, note).

Reproduce (on `vh`-shaped box): as §5, plus `KF_GUEST_IMG=/workspace/bench/guest_apps.qcow2` for
provisioning and every guest run; `APPS_PER_BOOT=100` for the batched no-PM run, `APPS_GUEST_PM=1`
for the PM run, `APPS_PER_BOOT=1` for isolation.

---

# R1 — the first measurement, kayfabe `79848341`

**STATUS of R1: ANSWERED, 2026-09-26** (measured 00:10–04:00 UTC). kayfabe **`79848341`** (origin/master; the kf3 binary was built
from it, `kf3-bins/79848341`); harness commits on branch `v3-apps` touch only `scripts/apps/`,
`traces/v3_app_matrix/` and this file. Two rented vast KVM boxes, host driver **580.159.04**
(kernel-open), guest Ubuntu 24.04 with the stock **580.159.04** guest driver, kf3 device, 16 GiB
guest RAM, 6 vCPUs:
- **va1 — RTX 3060 12 GB (GA106)**, vast 52660722, guest FB `fb-mb=8192` (default);
- **va2 — RTX 3070 8 GB (GA104)**, vast 52660725, guest FB `fb-mb=6144` (8192 would exceed the card).

Question (owner): nvkvm-pv (the shipped Mode-1 sibling) validated a set of real CUDA apps. Run the
same set inside a kayfabe **v3 fat guest** and record which ones **work** — correct output, no
hang, no crash, no Xid. Not parity, not timing. Every app ran on the **host of the same box first**
(bare metal, identical binaries/runtime), so a guest-only failure indicts kayfabe, not the app.

## 0. Headline

- **Host: 71/71 PASS on both boxes** (65 apps + 6 `stream_probe` shapes). Every guest failure below is
  a guest-only failure.
- **Guest: 35/65 apps work on the 3060, 34/65 on the 3070** (one app per fresh boot). What works:
  nvidia-smi, deviceQuery, every **default-stream** CUDA program (vectorAdd, the nvkvm-pv HPC kernels
  incl. cuBLAS SGEMM and cuFFT, reduction/scan/sort/histogram, tensor-core GEMMs, dynamic
  parallelism, cooperative groups, simpleCUBLAS/CUFFT, conjugateGradient), HF transformers greedy
  generate (**token-identical to the host**), OpenCL (clinfo, clpeak).
- **One defect blocks most of the rest: any kernel launched on a non-default CUDA stream** faults the
  host GPU with **`Xid 13 SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE`** (§3 A) — 21 of the 65 apps, including
  PyTorch, llama.cpp, Blender, hashcat, Geekbench and every CUDA-graph / multi-stream sample.
- **Without guest persistence mode a guest can run only ~5–6 CUDA processes per boot**; the next one
  hangs forever and so does every one after it (§3 B). **With persistence mode that wall is gone**
  (59 processes in one boot, identical verdicts on both boxes, B-class apps such as CuPy pass) — until a
  host-side `NV_ESC_RM_MAP_MEMORY … NoMemory` wedges the boot at the ~60th process (§3 J).
- **One silent wrong answer**: `conjugateGradientUM` prints `result = SUCCESS` with
  `Error amount = 1.000000` after a host `Xid 31` MMU fault (§3 C).
- Not exposed at all: **NVENC/NVDEC** ("unsupported device"), **EGL** (`eglInitialize failed`),
  **Vulkan** (`vulkaninfo` hangs, vkpeak "No vulkan device").

## 1. The app set (inventory of nvkvm-pv) and what "pass" means here

Sources: `nvkvm-pv/tests/perf/{realapp_matrix.md,matrix_remote.sh,graphics_remote.sh,apps/}`,
`tests/integration/attach_verify.cu`, `docs/reference/{parity,blender-opendata,openbenchmarking-clpeak}.md`,
`tests/uvm_multiarch/build_apps.sh`, `tests/validate.sh`.

Harness: `scripts/apps/`. One binary set is built on the host (`build_bundle.sh`: CUDA 12.6
toolkit, `-arch=sm_86`, cuda-samples v12.5, gpu-burn, llama.cpp `4b1a27f` CUDA, clpeak 1.1.2,
vkpeak 20250531) and the **same files** run on both sides; `setup_side.sh` installs the identical
runtime on host and guest (torch 2.6.0+cu124, torchvision 0.21.0, transformers 4.46.3,
cupy-cuda12x 13.3.0, distro ffmpeg / hashcat / vulkan-tools / clinfo, Blender 4.5.0, Geekbench
6.4.0). `run_apps.sh` holds every app's command and pass predicate: **rc 0 and the app's own success
string and no `CHECK … FAIL`** (predicates were calibrated on the host baseline; many cuda-samples
verify on the CPU and report only through their exit code).

| group | apps | pass predicate |
|---|---|---|
| nvidia-smi | `nvidia_smi` | lists the RTX GPU |
| cuda-samples v12.5 | deviceQuery, vectorAdd, vectorAddDrv, matrixMul, matrixMulDrv, bandwidthTest, simpleStreams, asyncAPI, simpleAtomicIntrinsics, simpleCallback, simpleOccupancy, simpleZeroCopy, simpleCooperativeGroups, concurrentKernels, simpleIPC, cudaTensorCoreGemm, bf16TensorCoreGemm, globalToShmemAsyncCopy, cdpSimpleQuicksort, graphMemoryNodes, simpleCudaGraphs, simpleCUBLAS, simpleCUFFT, conjugateGradient, MersenneTwisterGP11213, reduction, sortingNetworks, scan, histogram, BlackScholes, fastWalshTransform, transpose | the sample's own CPU verification (PASS line or exit code) |
| nvkvm-pv realapp kernels | stream_triad, reduce, nbody, blackscholes, mandelbrot, conv2d, sgemm_cublas, fft_cufft, sha256, memcpy2d | their `CHECK ok` (value checks; several are only `isfinite`) |
| managed memory | attach_verify, conjugateGradientUM, UnifiedMemoryStreams, UnifiedMemoryPerf | own check (UMStreams/UMPerf: completion only) |
| stress | gpu_burn 60 s | `GPU 0: OK` (0 compare errors) |
| PyTorch | torch_correct (new: GPU vs CPU matmul fp32/fp16, cuDNN conv, CNN train step, 1 GiB round trip, sort/cumsum, 4 streams), torch_ai_bench (nvkvm-pv `ai_bench.py`) | every `CHECK ok`; torch_correct is numerical vs CPU |
| LLM | hf_generate (transformers greedy, Qwen2-0.5B fp16), llama_cpp_gen (llama.cpp, Qwen2.5-1.5B Q4_K_M greedy), llama_bench | completes; greedy output digest compared host vs guest |
| CuPy | cupy (NVRTC kernel, cuBLAS, cuFFT, cuRAND vs NumPy) | every `CHECK ok` |
| Vulkan / OpenGL / OpenCL | vulkaninfo, vkpeak, egl_offscreen (EGL device platform), clinfo, clpeak, geekbench_gpu (GB6 OpenCL) | NVIDIA device / GFLOPS / no GL error / NVIDIA platform / bandwidth / all GB workloads run |
| video | nvenc_h264, nvenc_hevc, nvdec_h264 (`-hwaccel cuda`, fails on software fallback) | 600 frames encoded (ffprobe) / decoded on NVDEC |
| crypto / render | hashcat (md5 mask attack), blender_cycles (scripted Cycles scene, CUDA then OptiX) | cracks the plaintext / both renders finish |
| probe (added) | stream_probe {default, created, nonblocking, perthread, two, created2nd} | one launch + full verify on that stream shape |

**Out of scope on a 1-GPU 8–12 GB box**: multi_gpu_app.py, nccl_allreduce.py, vLLM tensor-parallel
(≥2 GPUs); vLLM Qwen2.5-32B-AWQ (48 GB); llama.cpp 7B → 1.5B (same code path, fits the 8 GB card);
RAPIDS cuDF, glmark2 + weston, Unsloth, `validate.sh` (nvkvm-specific bring-up; its CUDA/Vulkan/GL
checks are covered above). Blender's Open Data launcher was replaced by a scripted render because
its mirror TLS-timed-out on the first box.

## 2. Results

One app per **fresh boot** (`APPS_PER_BOOT=1`), because a boot degrades after ~6 CUDA processes (§3
B) — a batched verdict would be contaminated by the apps before it. Guest cells carry the root-cause
letter of §3. `cupy`, EGL, Vulkan, NVENC/NVDEC and the stream probes are from the harness-corrected
re-run `r2` (r1 lacked the CUDA headers CuPy's NVRTC path needs, and did not load `nvidia_drm` in the
guest as the host has it — neither changed a verdict except CuPy). The **PM column** is a second
experiment: guest persistence mode on (`nvidia-smi -pm 1`) and ALL rows in ONE boot, in table order;
its verdicts were **identical on both boxes** for every row it reached, and the boot wedged at
`llama_bench` (the 60th process, cause J) — every later row in that boot hung.

| app | host 3060 | guest 3060 | host 3070 | guest 3070 | guest + PM, ONE boot (both boxes) | failure point (first failing box) |
|---|---|---|---|---|---|---|
| nvidia_smi | PASS | PASS | PASS | PASS | PASS |  |
| deviceQuery | PASS | PASS | PASS | PASS | PASS |  |
| vectorAdd | PASS | PASS | PASS | PASS | PASS |  |
| vectorAddDrv | PASS | PASS | PASS | PASS | PASS |  |
| matrixMul | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at matrixMul.cu:206 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)" |
| matrixMulDrv | PASS | PASS | PASS | PASS | PASS |  |
| bandwidthTest | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at bandwidthTest.cu:834 code=719(cudaErrorLaunchFailure) "cudaDeviceSynchronize()" |
| simpleStreams | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at simpleStreams.cu:371 code=719(cudaErrorLaunchFailure) "cudaEventSynchronize(stop_event)" |
| asyncAPI | PASS | PASS | PASS | PASS | PASS |  |
| simpleAtomicIntrinsics | PASS | FAIL (A) | PASS | TIMEOUT (B) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at simpleAtomicIntrinsics.cu:122 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)" |
| simpleCallback | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at simpleCallback.cu:146 code=719(cudaErrorLaunchFailure) "status" |
| simpleOccupancy | PASS | PASS | PASS | PASS | PASS |  |
| simpleZeroCopy | PASS | PASS | PASS | TIMEOUT (B) | PASS | silent hang (no output until the kill); kf3: slot N is full and nothing can be retired |
| simpleCooperativeGroups | PASS | PASS | PASS | PASS | PASS |  |
| concurrentKernels | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at concurrentKernels.cu:196 code=719(cudaErrorLaunchFailure) "cudaEventSynchronize(stop_event)" |
| simpleIPC | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at simpleIPC.cu:161 code=719(cudaErrorLaunchFailure) "cudaMemcpyAsync(&verification_buffer[0], ptrs |
| UnifiedMemoryStreams | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at UnifiedMemoryStreams.cu:221 code=13(CUBLAS_STATUS_EXECUTION_FAILED) "cublasDgemv(handle[tid + 1] |
| UnifiedMemoryPerf | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — Running .CUDA error at matrixMultiplyPerf.cu:435 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(strea |
| conjugateGradientUM | PASS | FAIL (C) | PASS | FAIL (C) | FAIL | Xid 31, MMU Fault: ENGINE GRAPHICS GPC1 faulted @VA. Fault is of type FAULT_PDE ACCESS_TYPE_VIRT_READ — Test Summary: Error amount = 1.000000, result = SUCCESS |
| cudaTensorCoreGemm | PASS | PASS | PASS | PASS | PASS |  |
| bf16TensorCoreGemm | PASS | PASS | PASS | PASS | PASS |  |
| globalToShmemAsyncCopy | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at globalToShmemAsyncCopy.cu:863 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)" |
| cdpSimpleQuicksort | PASS | PASS | PASS | PASS | PASS |  |
| graphMemoryNodes | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at graphMemoryNodes.cu:322 code=719(cudaErrorLaunchFailure) "cudaMemcpyAsync(hostArrays->square, d_ |
| simpleCudaGraphs | PASS | FAIL (A) | PASS | TIMEOUT (B) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at simpleCudaGraphs.cu:273 code=719(cudaErrorLaunchFailure) "cudaGraphLaunch(graphExec, streamForGr |
| simpleCUBLAS | PASS | PASS | PASS | PASS | PASS |  |
| simpleCUFFT | PASS | PASS | PASS | PASS | PASS |  |
| conjugateGradient | PASS | PASS | PASS | PASS | PASS |  |
| MersenneTwisterGP11213 | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at MersenneTwister.cpp:115 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)" |
| reduction | PASS | PASS | PASS | PASS | PASS |  |
| sortingNetworks | PASS | PASS | PASS | PASS | PASS |  |
| scan | PASS | PASS | PASS | PASS | PASS |  |
| histogram | PASS | PASS | PASS | PASS | PASS |  |
| BlackScholes | PASS | PASS | PASS | PASS | PASS |  |
| fastWalshTransform | PASS | PASS | PASS | PASS | PASS |  |
| transpose | PASS | PASS | PASS | PASS | PASS |  |
| stream_triad | PASS | PASS | PASS | PASS | PASS |  |
| reduce | PASS | PASS | PASS | PASS | PASS |  |
| nbody | PASS | PASS | PASS | PASS | PASS |  |
| blackscholes | PASS | PASS | PASS | PASS | PASS |  |
| mandelbrot | PASS | PASS | PASS | PASS | PASS |  |
| conv2d | PASS | PASS | PASS | PASS | PASS |  |
| sgemm_cublas | PASS | PASS | PASS | PASS | PASS |  |
| fft_cufft | PASS | PASS | PASS | PASS | PASS |  |
| sha256 | PASS | PASS | PASS | PASS | PASS |  |
| memcpy2d | PASS | PASS | PASS | PASS | PASS |  |
| attach_verify | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — FAIL cudaStreamSynchronize(s[b]) -> 719 (unspecified launch failure) |
| gpu_burn | PASS | FAIL (G) | PASS | FAIL (G) | FAIL | SIGSEGV in gpu_burn right after cuInit (guest dmesg: segfault at the stack top); no Xid, no kf3 refusal |
| torch_correct | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — RuntimeError: GET was unable to find an engine to execute this computation |
| torch_ai_bench | PASS | FAIL (C) | PASS | FAIL (A) | FAIL | Xid 31, MMU Fault: ENGINE GRAPHICS GPC2 faulted @VA. Fault is of type FAULT_PDE ACCESS_TYPE_VIRT_WRITE — RuntimeError: CUDA error: unspecified launch failure |
| hf_generate | PASS | PASS | PASS | TIMEOUT (I) | PASS | silent hang; kf3 `REFUSED walk: run[0] leaves the guest's GPGA: gpga=0x1d5ed0000 … span is 0x180000000` (3070 box ran `fb-mb=6144`) |
| cupy | PASS | TIMEOUT (B) | PASS | PASS | PASS | silent hang (no output until the kill); kf3: slot N is full and nothing can be retired |
| llama_cpp_gen | PASS | TIMEOUT (B) | PASS | FAIL (A) | FAIL | 3060: silent hang, kf3 `slot 16 is full and nothing can be retired`; 3070: Xid 13 SKEDCHECK05 |
| llama_bench | PASS | FAIL (A) | PASS | FAIL (A) | TIMEOUT (J) | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — /workspace/apps/srcs/llama.cpp/ggml/src/ggml-cuda/ggml-cuda.cu:109: CUDA error |
| vulkaninfo | PASS | TIMEOUT (F) | PASS | TIMEOUT (F) | hung (after J) | blocks inside an RM ioctl (strace: last call `NV_ESC_RM_CONTROL`, then nothing); guest dmesg: `scrubberDestruct: Timed out`, `ce_utils.c:349` assert |
| vkpeak | PASS | FAIL (F) | PASS | FAIL (F) | hung (after J) | No vulkan device |
| egl_offscreen | PASS | FAIL (E) | PASS | FAIL (E) | hung (after J) | CHECK egl_gl_Mtri_s FAIL |
| clinfo | PASS | PASS | PASS | PASS | hung (after J) |  |
| clpeak | PASS | PASS | PASS | PASS | not reached |  |
| nvenc_h264 | PASS | FAIL (D) | PASS | FAIL (D) | not reached | [h264_nvenc @ 0x5aeb9435b9c0] OpenEncodeSessionEx failed: unsupported device (2): (no details) |
| nvenc_hevc | PASS | FAIL (D) | PASS | FAIL (D) | not reached | [hevc_nvenc @ 0x5b9be2c6a9c0] OpenEncodeSessionEx failed: unsupported device (2): (no details) |
| nvdec_h264 | PASS | FAIL (D) | PASS | FAIL (D) | not reached | [h264 @ 0x6003c48bbf80] decoder->cvdl->cuvidGetDecoderCaps(&caps) failed -> CUDA_ERROR_NO_DEVICE: no CUDA-capa |
| hashcat | PASS | FAIL (A) | PASS | FAIL (A) | not reached | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — Watchdog: Temperature abort trigger set to 90c |
| blender_cycles | PASS | TIMEOUT (A) | PASS | FAIL (A) | not reached | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — RuntimeError: Error: Launch failed in CUDA queue synchronize (integrator_shade_surface) |
| geekbench_gpu | PASS | FAIL (A) | PASS | FAIL (A) | not reached | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — [0926/022702:ERROR:optimizer.cpp(122)] build_patches_padding: optimization failed for size { 32, 1, 1, }: Wait |
| stream_default | PASS | PASS | PASS | PASS | PASS |  |
| stream_created | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CHECK created FAIL sync=719(unspecified launch failure) |
| stream_nonblocking | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CHECK nonblocking FAIL sync=719(unspecified launch failure) |
| stream_perthread | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CHECK perthread FAIL sync=719(unspecified launch failure) |
| stream_two | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CHECK two_a FAIL sync=719(unspecified launch failure) |
| stream_created2nd | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CHECK c2_created FAIL sync=719(unspecified launch failure) |

A 26 ['MersenneTwisterGP11213', 'UnifiedMemoryPerf', 'UnifiedMemoryStreams', 'attach_verify', 'bandwidthTest', 'blender_cycles', 'concurrentKernels', 'geekbench_gpu', 'globalToShmemAsyncCopy', 'graphMemoryNodes', 'hashcat', 'llama_bench', 'llama_cpp_gen', 'matrixMul', 'simpleAtomicIntrinsics', 'simpleCallback', 'simpleCudaGraphs', 'simpleIPC', 'simpleStreams', 'stream_created', 'stream_created2nd', 'stream_nonblocking', 'stream_perthread', 'stream_two', 'torch_ai_bench', 'torch_correct']

Totals (65 apps, probes excluded): **host 65/65 on both; guest 35/65 (3060), 34/65 (3070)**.
Digests: `hf_generate` greedy tokens **identical** host vs guest on the 3060 (`0d973108a6251e14`).
Raw rows: `traces/v3_app_matrix/<box>/{r1,r2,seq}/{host,guest}.res` + `triage.txt`; evidence excerpts
in `traces/v3_app_matrix/va1_rtx3060/evidence_excerpts.txt`.

## 3. Failure causes, ranked by apps blocked

| rank | cause | apps blocked (of 65) | signature |
|---|---|---|---|
| 1 | **A — kernels on a non-default stream fault the host GR** | **21** (+ all 5 non-default stream probes) | host `Xid 13, Graphics Exception: SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed`, class `c7c0`; kf3 `RC host twin … (guest chid 0x8/0x9, engine 0x1) … Xid 13 — forwarding RC_TRIGGERED`; app sees `cudaErrorLaunchFailure (719)` |
| 2 | **B — VA-space slot exhaustion ⇒ silent hang** | **5** directly (cupy, llama_cpp_gen, simpleZeroCopy, simpleAtomicIntrinsics, simpleCudaGraphs — nondeterministic, box-dependent) **and every app from the ~6th–7th CUDA process of a boot on** | kf3 `REFUSED VasKey(..) root 0x201000: slot N is full and nothing can be retired — raise WalkCfg::runs_per_pdb` + `split ticket N REFUSED`; the process then makes no progress, no Xid |
| 3 | **D — video engines absent** | 3 (nvenc_h264, nvenc_hevc, nvdec_h264) | `OpenEncodeSessionEx failed: unsupported device`; `cuvidGetDecoderCaps … CUDA_ERROR_NO_DEVICE`; no kf3 refusal (the guest never asks) |
| 4 | **C — MMU fault on a managed/UVM mapping** | 2 (conjugateGradientUM — **silent wrong answer**, torch_ai_bench on the 3060) | host `Xid 31 … MMU Fault: ENGINE GRAPHICS … FAULT_PDE`, first compute channel (guest chid 0x7) |
| 5 | **F — Vulkan** | 2 (vulkaninfo, vkpeak) | vulkaninfo blocks in an RM ioctl; vkpeak `No vulkan device`; guest RM `scrubberDestruct: Timed out`, `ce_utils.c:349` |
| 6 | E — EGL | 1 (egl_offscreen) | `eglInitialize failed`, no Xid, no kf3 refusal |
| 6 | G — gpu_burn | 1 | SIGSEGV in gpu_burn right after `cuInit` (reads past its stack); no Xid |
| 6 | J — host map `NoMemory` late in a long PM boot | 1 (llama_bench as the ~60th process; the boot then hangs for every later app) | kf3 `REFUSED VasKey(..) root 0x1f1cac000: 1 run(s) not applied: window map 0x110000+0x200000 (store @0x1000000): arm: NV_ESC_RM_MAP_MEMORY store@0x1000000+0x200000: NoMemory` |
| 6 | I — walk refused beyond the FB span | 1 (hf_generate, 3070 only) | `REFUSED walk: run[0] leaves the guest's GPGA: gpga=0x1d5ed0000 … span is 0x180000000` with `fb-mb=6144` |

### A — non-default streams (rank 1)
The split is exact and measured by `stream_probe` (r2, both boxes): **`default` PASS; `created`,
`nonblocking`, `perthread`, `two`, `created2nd` all FAIL with 719**, and `created2nd` passes its
default-stream half then fails the created-stream half in the same process. Every other A row names
a stream in its failing call (`cudaStreamSynchronize(stream)`, `cudaEventSynchronize`,
`cudaGraphLaunch`, cuBLAS/cuDNN handles bound to streams). The faulting host channel is always the
guest's **second or third** compute channel (guest chid 0x8/0x9); the first (chid 0x7) runs
default-stream work fine. `SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE` means the QMD asked for more shader
local memory than that channel's context has configured — i.e. the per-channel (per-subcontext)
local-memory setup that the driver does for the first channel is not in effect for the additional
channels on the host. ⚠ Mechanism is a hypothesis; the signature and the stream split are measured.
None of the failing user kernels use local memory themselves (`cuobjdump -res-usage`: `LOCAL:0`).

### B — slot exhaustion (rank 2)
`seq` experiment: 10× `vectorAdd` in ONE boot → **6 PASS then 4 TIMEOUT on the 3060, 5 PASS then 5
TIMEOUT on the 3070**, with `slot 58 is full and nothing can be retired` at root `0x201000` at the
first hang. The first r1 attempt (all apps batched in one boot) wedged at the 3rd CUDA app and every
later app timed out silently (`traces/v3_app_matrix/va1_rtx3060/r1_batched_contaminated/`). Single
processes with many mappings (llama.cpp loading a 1 GB model, CuPy) hit it on their own. ⇒ In
practice this blocks *every* app for any guest that has already run a handful of CUDA processes; it
is ranked 2 only because the per-boot matrix isolates it. With guest persistence mode the budget disappears (§4).

### C — Xid 31 on managed memory (rank 4)
`conjugateGradientUM` finishes, prints `result = SUCCESS`, and reports `Error amount = 1.000000`
(host `0.000000`): a **silent wrong result** behind a host MMU fault (`FAULT_PDE`, VA
`0x7d09_ac200000`, a UVM managed range). `torch_ai_bench` hit the same fault class on the 3060 (the
3070 run died earlier, on A).

## 4. Persistence mode, and what is still open

- ★ **Persistence mode removes B's per-boot budget.** `seqpm` (3060, `nvidia-smi -pm 1` in the guest,
  then `stream_created` + 9× `vectorAdd` in ONE boot): **all 9 vectorAdd PASS** (without PM: 6 then
  hang), and `stream_created` still FAILs with 719 (A is independent of PM). ⇒ B is tied to the guest
  RM tearing the adapter down and re-initialising it per process (what happens without PM), not to the
  processes themselves. `traces/v3_app_matrix/va1_rtx3060/seqpm/`. A full one-boot PM run of all 71
  rows (`pm1`) is the PM column of §2: B vanishes, A/C/G are unchanged, J appears at the 60th process.
- EGL / Vulkan / gpu_burn root causes are not localised beyond the signatures above (no kf3 refusal is
  logged for any of them).

## 5. Reproduce

    # on a box provisioned by scripts/bench/{provision_box,provision_host_driver,provision_bench_tree,build_kf3}.sh
    ln -sfn /workspace/apps /opt/apps
    bash scripts/apps/build_bundle.sh            # host: CUDA 12.6 toolkit + the bundle
    bash scripts/apps/setup_side.sh              # host runtime (identical to the guest's)
    bash scripts/apps/provision_guest_apps.sh    # guest image: bundle + runtime (stock qemu, slirp)
    bash scripts/apps/apps_matrix.sh host  <run> all
    KF3_BIN=/workspace/bench/kf3-bins/<rev>/qemu-system-x86_64 bash scripts/apps/apps_matrix.sh guest <run> all
    #   (KF3_FB_MB=6144 on an 8 GB card; APPS_PER_BOOT=N to batch; diag_hook.sh for strace diagnostics)
