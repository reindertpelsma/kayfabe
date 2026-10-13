# Flag-free Windows default route, run 1660 (source ec45aad99)

**STATUS: RESEARCH (measured evidence), 2026-10-13.** One Windows run of the exact source `ec45aad99a81cb16eb0554b77f0ff331c47cf6d3` (branch `codex/windows-matrix-20261011`, "Use consumption-ordered
FIFO migration for Windows USER channels by default") on the RTX 4070 / host driver 595.91.07 / Windows 580.88 (guest-visible 4096 MiB), under the global GPU lock, run number 1660, 2026-10-13 09:38-09:48 UTC (11:38-11:48 CEST).
Stage script: `claude-default-windows-ec45aad99.sh` (Codex's `default-route/run_windows.sh` with only the run directory renamed and `--max-tdr 99` added; see below). Linux for the same revision (Codex, 11:17-11:22 CEST): gates 9/9 + gate 10, USER birth 12/12, fast 30/30, desktop smoke PASS_WITH_WARNINGS.

## Result (MEASURED, `relay/summary.md`, `relay/apps/*.verdict.json`)

| app | verdict | detail |
|---|---|---|
| edge_video_h264 | PASS | proof pdh:videodecode, gsp_cycles 1 (same as the earlier PASS_WITH_RECOVERY at 0f9c01f9) |
| gravitymark_d3d12 | FAIL (harness) | rc 0, 183 s, result image 866321 bytes, 33921 timing lines, zero TDR/WER/GSP cycles; failed only `no-gpu-proof` |
| vectorAdd_demo | PASS | proof cuda-vectoradd-readback |
| gravitymark_vk | **PASS** | original 600 s oracle, 180 s, rc 0, proof pdh:copy, tdr 0, wer 0, gsp_cycles 0, no restart |

Host Xid count before/after: 56/56 (`relay/host_xid.txt`; the lines printed there are earlier Linux-suite records). Guest TDR events in passing and failing apps: 0; WER reports: 0. Stopped clean, no QEMU, IOMMU group 11 `DMA-FQ`.
This is the first flag-free (no `KF3_WIN_USER_RING_RELAY`) PASS of GravityMark Vulkan on kayfabe. Earlier Oct 11 results at `72741e2b`: gravitymark_vk TIMEOUT, gravitymark_d3d12 FAIL with 3 guest TDRs and 1 GSP cycle.

## What this is not
- **Not a zero-flag run.** `qemu-command.json` shows the production profile's behaviour flags (KF3_ASYNC_PREEMPT, KF3_DEFERRED_API, KF3_DISPLAY_CAPS_PROBE, KF3_DISPLAY_CTRL_PROBE, KF3_DISPLAY_ILUT_OFFSET_256, KF3_DISPLAY_IMP_ENABLE, KF3_DISPLAY_LUT_MIRROR, KF3_DISPLAY_SDR_COLOR, KF3_GFX_POOL_PROBE, KF3_KERNEL_GR_CE,
  KF3_KERNEL_GR_WORK, KF3_MEMORY_LIST_PROBE, KF3_PREEMPT_BIND_PROBE, KF3_SW_RUNLIST_HOST_OWNED, KF3_SW_RUNLIST_PROBE, KF3_TRANSLATED_CE_RELAY, KF3_WIN_TWIN_DEFAPI_OBJECT, KF3_WIN_USER_CHANNELS_PASSTHROUGH, KF3_ZCULL_BIND_PROBE); only the relay route is default.
- **Not a matrix, soak or performance result.** One run, four apps, one guest; no latency/CPU measurement, no cross-VM test (channel RC scope is unknown from source, see `docs/design/V3_CHANNEL_RC_SCOPE_20261012.md` on `claude/rc-scope-20261012`).
- **The D3D12 FAIL is a harness proof verdict, not a render failure.** `verdict.py` accepts a `pdh:Cuda|Compute|3D|Copy` proof only if an engine counter reaches `ENGINE_MIN_UTIL = 0.5`; this run read copy 0.42 and 3d 0 (the sampler's keys have no compute or cuda engine). INFERRED: under kayfabe the guest's
  per-engine utilization counters read about 0 for 3D and compute (natively, 3D apps show 3d/copy; the Vulkan PASS here rests on `pdh:copy` only), a telemetry fidelity gap to check (`nvidia-smi` utilization inside the guest during a 3D load). Not verified.

## Why the harness setting changed
Codex's run 1650 of the same source aborted in the harness: the default `max_tdr=3` counted three `nvlddmkm 153` events (a GPU error report that also fires on healthy native runs, see `traces/windows_baseline_20261010/README.md`) as TDRs
(`guest_tdr = nvlddmkm_153 + display_4101`), the restart path ran the IOMMU hook before the old QEMU had stopped, and three following guest boots failed (`pre-guest hook rc=1`). Run 1660 raised the budget to 99 and booted one guest for all four apps.
