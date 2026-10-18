# Steel Nomad gap, kayfabe vs plain VFIO, measured inside the guest (2026-10-18)

STATUS: LIVE, 2026-10-18 (measurement; Sonnet 5.5 agent, owner away). Branch `claude/steelnomad-gap-20261018` (off master 7a9059fb0).
Scripts: `scripts/bench/steelnomad_gap/`. Big evidence (ETL, 400-500 MB CSVs, perf.data): host `/data/claude-sngap-20261018/` (not in git).

Setup. Same disk (`win11-full-b`), same CPU/RAM/devices, same driver 580.88 in the guest, 3DMark Demo Steel Nomad DX12, 1920x1080.
kayfabe = candidate-4 rev 0452613b, perf profile, owner launcher `claude-manual-owner-perf4.sh` (runs 3005: k1-k4, 3006: k5 and the probes).
VFIO = `vfio_owner_ref.sh` (throw-away qcow2 overlay, real 4070, `VDR_NOINPUT=1`, boot 2): v1-v4. One VM at a time, GPU lock held for the whole VFIO session.
HAGS: kayfabe guest `dxdiag`: "Hardware Scheduling: DriverSupportState:Stable Enabled:True" (registry `HwSchMode` unset = default, on both sides).
The VFIO guest shows the same hardware-queue events (`hHwQueue`, ids 450/451), so HAGS is on there too; dxdiag was NOT run on the VFIO side. No guest setting was changed.

## What was automated, and how
* 3DMark Demo is a CEF window (UI Automation exposes no DOM): `sn_ui.ps1` runs in the interactive session (`sn_it.ps1`, one-shot scheduled task /it as the logged-in user,
  pattern of `appmatrix/native/run_overlay_it.ps1`) and clicks at window-relative fractions: BENCHMARKS (0.717, 0.086), Steel Nomad card (0.493, 0.773), RUN (0.861, 0.389),
  RUN AGAIN on the results page (0.1786, 0.845). Launch = the owner's `3DMark Demo.url` shortcut (Steam was already running). Screenshots come back through the diag channel.
  Steel Nomad was fully automated; GravityMark was not needed.
* Capture = `sn_run_capture.sh TAG` (host-driven through `kfagent_server.py`): live ETW session `Microsoft-Windows-DxgKrnl` keyword 0x1 (Base), level 5, QPC clock, 128 KiB buffers,
  `logman -ets` (nothing armed for boot, no TdrDelay change) + PresentMon 2.6.0 (already on the guest) -> click RUN -> stop -> `tracerpt` CSV in the guest -> zip -> POST to the broker.
  `sn_run_capture_late.sh` starts ETW 26 s after the click (keyword 0x3 = Base + Profiler: DDI enter/exit, 105/106) for the DDI timings.
* Analysis (stdlib Python): `sn_extract.py` (CSV -> events), `sn_analyze.py` (per-capture metrics), `sn_compare.py`, `sn_pos.py` (per packet position in the frame), `sn_ddi.py`/`sn_ddi_cmp.py`.
* Probe `sn_gpu_probe.ps1`: D3D12 raw-COM micro-probe (pattern of `d3d12_signal_probe.ps1`), identical on both sides, 3 runs each (`probe/`, `probe_summary.md`).
* Event field names are from the guest's own manifest (Get-WinEvent on the same ETL). Meaning of 450/451 (HwQueue submit / progress fence) is read from the field names (INFERRED).

## Commands (export SN_HOST=root@<trusted host>; scripts in `scripts/bench/steelnomad_gap/`)
```
guest_put.sh FILE 'C:\kf\sn\FILE'                       # push the scripts into the guest through the diag channel
sn_step.sh FX FY out.png                                  # click in 3DMark + screenshot (navigation)
RUN_ARGS="-FX 0.861 -FY 0.389" sn_run_capture.sh TAG 0x1 110   # first run from the start page; later runs use RUN AGAIN (default)
HOSTCAP="OUTDIR QPID 65" sn_run_capture.sh k4 0x1 110     # with the host capture.sh profiler
sn_run_capture_late.sh TAG 0x3 26 45                      # Base + Profiler, ETW starts 26 s after the click
python3 sn_extract.py DIR TAG; python3 sn_analyze.py DIR TAG DIR/TAG/TAG_pm.csv   (DIR/TAG/ layout: sn_run_capture.sh output)
python3 sn_compare.py DIR kayfabe:k1,k2,k3 vfio:v1,v2,v3; python3 sn_pos.py DIR k1 k2 k3 -- v1 v2 v3; python3 sn_ddi.py DIR TAG; python3 sn_ddi_cmp.py DIR k5 v4
interactive: sn_it.ps1 -Script C:\kf\sn\sn_gpu_probe.ps1 -ScriptArgs "-Out C:\kf\sn\ui.txt"    (modes: -Mode A | A2 | H, -WithTimestamps)
```
Note: `sn_extract.py` / `sn_analyze.py` read `DIR/TAG_dxg.csv` etc.; on the host the files were moved into `DIR/TAG/` and `sn_pos.py`/`sn_ddi.py`/`sn_compare.py` expect that layout (run extract/analyze with DIR=`DIR/TAG` and TAG as in the host tree).

## Captures (n, spread)
kayfabe: k1, k2, k3 (ETW Base), k4 (ETW + host `capture.sh` perf/offcpu running, so slightly perturbed), k5 (ETW Base+Profiler, 45 s). VFIO: v1, v2, v3 (ETW Base), v4 (Base+Profiler, in-guest `nvidia-smi dmon`).
Main table = k1-k3 vs v1-v3 (`compare_k123_v123.md`); all captures: `compare_all_k1-5_v1-4.md`. Window = largest run of frames with >=20 packets on the heavy queues, minus 3 s at each end (~55-57 s).
3DMark scores: kayfabe 2520, 2510, 2504, 2445, 2369 (k4, perf running), 2466 (k5), and the lead's earlier 2369 x2 and 2405; VFIO 3841, 3851, 3955, 3955.
So tracing did not slow kayfabe (the first runs even scored higher than 2369-2405); scores drift down over consecutive kayfabe runs (2520 -> 2369), not explained.

## Table (MEASURED; median over captures [min..max]; kayfabe n=3 / VFIO n=3)
| metric | kayfabe | VFIO | ratio |
|---|---|---|---|
| fps (ETW present packets) | 25.04 [24.46..25.10] | 38.52 [38.39..39.55] | 0.65 |
| frame ms median / p95 / p99 | 39.4 / 45.0 / 51.5 | 26.1 / 28.4 / 29.2 | 1.51 / 1.58 / 1.77 |
| frame ms stdev | 3.83 [3.3..5.5] | 1.52 [1.51..1.65] | 2.5 |
| PresentMon CPU wait / in-Present (ms) | 0.30 / 0.30 | 0.23 / 0.23 | CPU is not waiting on the GPU path |
| PresentMon GPU time = GPU busy / GPU latency (ms) | 39.8 / 112 | 25.6 / 71 | 1.55 / 1.58 |
| frames in flight (mean) | 2.82 | 2.80 | 1.0 |
| heavy-queue packets per frame / per queue per s | 28.6 / 237 | 28.9 / 367 | same work per frame |
| hardware-queue outstanding depth, mean over 3 queues | 5.97 (6+ for 99.4% of the time) | 5.90 | 1.0 |
| time with any packet outstanding on a hardware queue | 99.55% [99.43..99.70] | 99.99% | no guest-visible idle in either |
| guest-visible idle gaps | 0.02 /s (1-2 gaps of 6-15 ms per 57 s) | 4.5 /s, median 19 us (0.002 ms/frame) | idle ~0 on both |
| per-queue empty fraction | 0.02% | 0.2% | |
| packet service time (completion spacing), median / p95 / p99 / mean (ms) | 2.91 / 14.1 / 17.3 / 4.21 | 0.83 / 11.4 / 14.2 / 2.69 | 3.5 / 1.23 / 1.21 / 1.57 |
| queue occupancy (sum of service per s) | 997 ms/s | 998 ms/s | all queues are always busy on both |
| completion -> UnwaitQueuePacket (238), median (ms) | 0.59 | 0.15 | +0.44 |
| `DdiSubmitCommandToHwQueue` median / p99 (us), k5 vs v4 | 10.6 / 23 | 1.5 / 5.7 | +9 us x ~27 per frame = 0.25 ms/frame |
| ISR `DpiFdoMessageInterruptRoutine` median (us) / calls per frame | 70.6 / 33.6 | 20.8 / 10.3 | 3.4x / 3.3x |
| frame ms, sum of per-position completion spacing | 40.0 | 25.7 | +14.35 |

Per position in the frame (`pos_k123_v123.txt`, queue Q0, 10 packets/frame, spacing = completion to completion):
`t_kayfabe = 1.07 * t_vfio + 1.40 ms` (9 positions, r = 0.98). Small packets (0.3-1.0 ms native) become 1.5-3.8 ms; the 11 ms packet becomes 13.6 ms.
=> 12.6 of the 14.35 ms/frame (88%) is a fixed ~1.4 ms per dependent step; 1.8 ms (12%) scales with packet size.

Micro-probe (3 runs each, `probe_summary.md`, us unless noted):
| probe | kayfabe | VFIO |
|---|---|---|
| A empty-queue Signal, polled / event | 1.4 / 2.6 | 1.4 / 2.8 |
| A2 64 KiB copy + Signal round trip, polled | 837 [823..929] | 61 [60..61] |
| B2 same, event wait | 879 [876..980] | 70 [70..70] |
| F DIRECT signal -> COPY wait -> COPY signal, polled | 3.8 | 3.9 |
| D / E PCIe sysmem -> VRAM / VRAM -> sysmem, GiB/s | 22.9 / 22.6 | 24.6 / 24.5 |
| C 256 MiB VRAM copy incl. one round trip, GiB/s | 121 | 206 |
| I size sweep (kayfabe only): round trip 64 KiB..16 MiB flat 830-880; 64 MiB 1114; 256 MiB 2072 | slope 192 MiB / 958 us = 196 GiB/s | native slope not run (C implies >= 206) |
| J (kayfabe only): CPU inside ExecuteCommandLists / Signal / wait for fence | 16 / 2.4 / 819 | not run |
| G / H (kayfabe only): 64 lists in one Execute + Signal / 64 separate Executes + one Signal | 1138 / 35,560 (0.555 ms per Execute) | not run |

Host counters while the kayfabe probe ran (`host/sngap_M2_counters.txt`, `host/sngap_H_counters.txt`, `host/sngap_H_top.txt`), MEASURED:
* one Execute(tiny copy)+Signal round trip = 25.7 trapped BAR0 writes (385,172 / 15,000), 6.1 interrupt edges raised to the guest, 74 worker "served", `applied` unchanged;
  an empty Signal causes ~0 trapped writes (console only, not archived); during H: 18.7 trapped writes per Execute, 33 K trapped writes/s, kf3-worker0/1 ~83% CPU each.
* Steel Nomad (k4 host capture): ~5,750 trapped writes/s, ~99 applied/s, GPU SM util ~75-80% (`host/dmon.txt`: 146 W, 2829 MHz, mclk 10251, PCIe RX 114 MB/s);
  VFIO guest dmon (`host/vfio_guest_dmon_v4.txt`): 197 W, 99.5% SM, 2706 MHz, mclk 10501, PCIe RX 2296 MB/s.

## Conclusion
MEASURED:
1. Both sides are GPU-pipeline-bound in the guest's view: same packets per frame (28.6/28.9), same frames in flight (2.8), same queue depth (~6), queues occupied 99.6-100% of the time, the app does not wait on the CPU
   (PresentMon CPU wait 0.3 ms). So the missing 13.9 ms/frame is NOT a guest-visible gap between packets, NOT guest CPU submission cost (`DdiSubmitCommandToHwQueue` +9 us per call, ~0.25 ms/frame),
   and NOT the guest completion path: a polled fence and an event-waited fence cost the same on kayfabe (A2 837 vs B2 879 us; native 61 vs 70), the guest completion DPC chain is microseconds.
2. The time is inside each hardware-queue packet: a fixed ~1.4 ms per dependent step (88% of the missing time, fit r=0.98) plus ~7% proportional (12%). An isolated 64 KiB copy takes 837 us from submit to a visible fence on
   kayfabe and 61 us natively (13.7x), CPU time inside ExecuteCommandLists is only 16 us, the empty Signal is equal on both sides. VRAM bandwidth is within 5% (196 vs >=206 GiB/s), PCIe within 7-8%, so it is not slower GPU execution of the work.
3. kayfabe delivers 3.3x more interrupts per frame and each ISR is 3.4x slower (70 vs 21 us): 2.4 ms of guest CPU per frame, not on the GPU critical path (queues never run dry).
INFERRED (not isolated): the ~0.55-0.84 ms per packet is host-side submission handling (kf3 worker / trapped-write drain / non-stall interrupt relay), because (a) a packet round trip costs 25 trapped writes and 6 relayed interrupt edges,
(b) with 64 back-to-back submissions the rate is 1 packet per 0.555 ms = 18.7 trapped writes at ~30 us, with both workers at ~83% CPU, (c) a frame position carries 3 packets (3 heavy queues in lockstep); 3 x 0.555 ms = 1.7 ms against the fitted fixed 1.4 ms per position.
That is "submission latency (the doorbell / submission-to-GPU-start path)", host-side, in the sense of the brief; it is not guest CPU submission, not completion/interrupt latency as seen by the guest, and not slower GPU execution.
The remaining ~12% proportional part is unexplained (memory clock 10251 vs 10501 MHz is only -2.4%).
Falsifiers: a native isolated-packet round trip >= 0.8 ms would kill it (measured 61 us); if the missing time were slow execution the per-position excess would grow with packet size (it is 1.1-2.8 ms for 0.3 ms and for 11 ms packets alike);
a kayfabe build that removes the per-submission trapped writes / worker hops and still shows A2 ~ 0.8 ms and unchanged fps would falsify the host-side attribution.

## Could not be done / caveats
* GPU timestamp probe (submit -> GPU start, GPU end -> visible) failed: `ResolveQueryData` into a readback buffer made the D3D12 device report DEVICE_REMOVED (0x887A0005) on kayfabe, without a host Xid (count stayed 20);
  kept behind `-WithTimestamps`, not run natively. So submit->GPU-start was NOT measured directly; the 0.84 ms is submit-call-return to fence-visible.
* Native G/H/I/J were not run (the probe grew after the VFIO session; the overlay is deleted). Native A2/B2/C/D/E/F are 3 runs. H/I/J rest on kayfabe alone.
* The per-position fit assumes position i is the same work on both sides (same packet counts per frame, identical DmaBufferSize 32768 for all positions, so size is not visible). Q0, Q1, Q2 have identical statistics (lock-step queues).
* "Idle" and "busy" are guest-visible outstanding-packet intervals; a queue can be outstanding while the GPU is blocked. The host SM util of ~75-80% (kayfabe) vs 99.5% (native) is a different run and tool.
* k0 (first test run) hit the 400 MiB ETL cap with 1 MiB buffers; k3's stop was run by hand after a script edit during the run; k1 also enabled keyword 0x4000000 (no events).
* `nvidia-persistenced` is masked on this host since 2026-10-09 (journal), so `restore` could not start it; this was the state before the session.
