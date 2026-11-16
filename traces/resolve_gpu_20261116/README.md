**STATUS: LIVE, 2026-11-16.** Evidence of the Resolve GPU lane work (text only; no screenshots, no media, UUIDs stripped). Source revisions: harness and lane code on branch
`claude/resolve-gpu-20261116` (commits 8cd6fa84d .. 2a9cf975d and later; the box runs used kf3 QEMU revisions `785a7ed3e`, `a564ba3c5`; the PC session used `fb274c4b`).
Design and the measured/inferred split: `docs/design/V3_RESOLVE_GPU_LANE.md`. Nothing here shows the five-render row passing: it has not passed on any platform (see below).

## What each directory is

| Directory | Run | What it shows (MEASURED unless marked) |
|---|---|---|
| `pc_rtx4070/` | PC, image A, kf3 `fb274c4b`, host driver 595.91.07, RTX 4070, held guest driven by hand | One UHD H.264 render of the 14 s test timeline (the render was cancelled at the missing-still failure and later runs were stopped by hand): guest per-engine counters of the Resolve process (`guest_engine_counters_h264_render.csv`: video decode up to 98.7 percent, copy 22, 3D 0, no compute engine, 3.0 to 3.4 GiB dedicated GPU memory), host `nvidia-smi` of the physical GPU during the render (`host_nvidia_smi_during_h264_render.csv`: 95 to 98 percent), Resolve's GPU lines (`resolve_log_gpu_lines.txt`: CUDA selected, CUDA board manager, NVENC/NVDEC listed, `NVML Error Unknown Error (999)`, GPU name `"NVIDIA "`). Resolve stayed up about 25 minutes. |
| `box_rtx4000ada/` | Vast box, one RTX 4000 Ada (workstation AD104GL), host 580.159.04 | Guest display device state during the failed harness `gpu` step (CM_PROB_REINSTALL, then CM_PROB_FAILED_POST_START = Code 43). Box destroyed; the harness text was not kept. |
| `box_rtx4060ti_gpu_step_fail_kf3_refusals.txt` | Vast box, RTX 4060 Ti (AD106) | kf3 host-log lines of the failed `gpu` step: `W349REFUSE rm.0x20802801 / rm.0x20802806` (unmeasured on AD106/580), authored values served for `rm.0x20802a08` and `rm.0x20800a59`, `GSP REFUSED` fn76 codes. These refusals are NOT the cause of the step failure (the 4090 failed identically without any refusal). |
| `box_rtx4090/` | Vast box, RTX 4090 (AD102), host 580.159.04, kf3 `a564ba3c5` | `guest_pnp_state_*`: the `gpu` step failure and its cause (below); `guest_remedies_*`: `pnputil /scan-devices` does not clear Code 43, `Disable-PnpDevice`/`Enable-PnpDevice` does and `nvidia-smi` then prints the seed UUID; `resolve_crash_after_import_run1.txt` and `guest_events_three_resolve_crashes.txt`: Resolve.exe died three times (nvoglv64.dll 0xc0000409, "Unable to recover from a kernel exception", nvlddmkm Id 153 x3 just before); `host_dmesg_nvrm_and_vamgr_lines.txt`; `driver_run_rg9/`: the graded row of one driver run (`verdict.json`: status INCONCLUSIVE, failure class CRASH, Resolve's log facts, kf3 census; the engine counters and host `nvidia-smi` of that run). |

## Findings in one place (full wording in the report)

1. Harness defect, fixed (commits 068680b5d, 438dfcd4f): `render_gpu_fix` restarted every NVIDIA display device including the image's old RTX 4070 phantom (CM_PROB_PHANTOM), so the `gpu` step failed on every die other than the image's own. After the fix the step passes on the RTX 4090 ("post-fix seed UUID exact match").
2. Resolve on image A under kf3 on the RTX 4090 crashes 3 to 4 minutes after the project is loaded (3 of 3 long-enough sessions); on the PC's RTX 4070 it did not in about 25 minutes. Cause not known (die, host driver 580.159.04 vs 595.91.07, kf3 revision and the nested host are all different between the two); the control (VFIO on the PC) is queued, the PC was powered off at 06:00Z.
3. Guest `nvidia-smi` reports `[N/A]` for utilisation and encoder/decoder columns (284 of 284 samples); the physical GPU is observable only on the host.

## Not done

The five-render matrix never completed; no render was verified on the host except the PC's partial H.264 file (not kept). Discovery rows, optical flow, Fusion, the Preferences GPU page, PugetBench and the spec-timeline import are not driven (design doc, "Not covered").
