# Fixtures of the GPU-use judge (scripts/ci/test_gpu_use.py)

**STATUS: LIVE, 2026-11-17.** Test data only: nothing here is evidence of a hardware run of the judge. Each file is a GPU case and its CPU case for one row of
the Linux app matrix, or a window of the kf3 host log. No real GPU UUIDs (there are none in these lines).

Labels: **REAL** = lines copied from a committed trace of a real run (the trace is named); **SYNTHETIC** = written from the tool's documented output or edited from a
real line, because no run has printed it yet.

## kf3 host log windows

| file | what it is | label |
|---|---|---|
| `kf3_base.txt` | the status line before the window (the first `kf3: family=` line of the vectorAdd window below) | REAL, `traces/v3_candidates/cand1_20261004/apps/vectorAdd.kf3.log` line 1 (RTX 3060, 2026-10-04) |
| `kf3_window_gpu.log` | the same window trimmed to its status lines, its six `RETIRED` lines and the first lines: births +6, submissions +411 | REAL, same file |
| `kf3_base_end.txt`, `kf3_window_idle.log` | the last status line of that run, and a window holding the same line again (nothing moved) | SYNTHETIC (one real line repeated) |
| `kf3_window_cpu_app.log` | that line with `parks=`, `trapped=` and `irq[writes=` raised: other counters move, no GPU-work counter does | SYNTHETIC (an edit of one real line) |
| `kf3_window_no_status.log` | a slice with no status line | REAL lines (GSP rpc, same file) without the status line |

## Application logs

| row | GPU case | CPU case | label |
|---|---|---|---|
| `blender_cycles` | `blender_gpu.log` | `blender_cpu.log` (CPU render), `blender_fallback.log` (CUDA asked, CPU rendered) | the `BLENDER_OK` and `Finished` lines are REAL (`traces/apps_linux_20261107/r1/blender_cycles.guest.log`); the `BLENDER_DEVICES` lines are the format of the new `blender_render.py`, SYNTHETIC until a run |
| `llama_cpp_gen` | `llama_gen_gpu.log` | `llama_gen_cpu_ngl0.log` (`-ngl 0`), `llama_gen_cpu_nocuda.log` (no CUDA backend) | REAL lines of `llama_cpp_gen.guest.log`; the CPU cases SYNTHETIC |
| `llama_bench` | `llama_bench_gpu.log` | `llama_bench_cpu_ngl0.log` (backend column still says CUDA, ngl 0) | REAL; the CPU case an edit of the real table |
| `hashcat` | `hashcat_gpu.log` | `hashcat_cpu.log` (pocl) | the tail is REAL (`hashcat.guest.log`); the `* Device #N:` lines SYNTHETIC (hashcat 6.2.x format) |
| `clpeak` | `clpeak_gpu.log` (the real log: NVIDIA block AND the pocl CPU block) | `clpeak_cpu.log` (the pocl block alone) | REAL |
| `vkpeak` | `vkpeak_gpu.log` | `vkpeak_cpu.log` (llvmpipe) | GPU REAL, CPU SYNTHETIC |
| `egl_offscreen` | `egl_gpu.log` | `egl_cpu.log` (llvmpipe) | GPU REAL, CPU SYNTHETIC |
| `nvenc_h264` | `nvenc_gpu.log` | `nvenc_cpu.log` (`libx264` encoder tag) | the ffmpeg lines REAL; `ENCODER_TAG` is the new line of the row, SYNTHETIC value |
| `nvdec_h264` | `nvdec_gpu.log` | `nvdec_cpu.log` (`pix_fmt: yuv420p`), `nvdec_failed_setup.log` | the `frame=` lines REAL; the `Reinit context ... pix_fmt:` marker INFERRED from the ffmpeg sources, never seen in a log of this tree |
| `torch_*`, `hf_generate`, `cupy` | `torch_gpu.log`, `cupy_gpu.log` | `torch_cpu.log`, `torch_noproof.log` (the row dropped the proof) | SYNTHETIC: the `GPU_USE_PROOF` line the scripts now print |

## ffmpeg 4.4.2 (Ubuntu 22.04) rows of the FIXED commands

| file | what it is | label |
|---|---|---|
| `nvenc_gpu_ffmpeg44.log`, `nvdec_gpu_ffmpeg44.log` | the nvenc_h264 and nvdec_h264 rows after the fix (stream-mapping line kept, both `Reinit context` lines kept), bare metal, RTX 3090 | REAL (`traces/open_rows_guest_20261117/runs/F1/`, host name scrubbed) |

