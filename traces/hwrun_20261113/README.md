# hwrun 2026-11-13: Phase 2 streaming SPEC run and the proprietary image, Vast box 60401716

**STATUS: LIVE, 2026-11-13.** MEASURED = a line in a file of this directory; INFERRED = reasoned from those lines, not seen. Box: Vast instance
60401716 (RTX 4090 AD102, AMD EPYC 7B13, 0.806 USD/h, rented about 2026-11-12 20:22Z, destroyed after this evidence was copied; cost in the report).
Revision under test: kayfabe `439ff38e7` (head of PR #360, branch `claude/user-ring-phase2-20261112`), kf3 binary `kf3-bins/439ff38e`, full performance
configuration (the `KF3_*` flags of `kf3_flags.sample.env`; lazy observe, put cache and pointer lifecycle are ON by default, none was set to 0).

## A. SPECviewperf 15 on Phase 2 (stage `specviewperf_stage.sh`, narrowed table `spec/spec_table.json`, hence NARROWED verdicts)
| viewset | result | note |
|---|---|---|
| creo-04 | PASS 13/13 | the viewset that reset the guest on 5d7a5bc6 (refusal `USER_RING_SCAN_BUDGET len=1042284`) |
| catia-07 | PASS 6/6 | |
| energy-04 | PASS 6/6 | |
| enscape-01 | run 1: FAIL 0/0 reason=GUEST; run 2 (fresh boot): NOT_RUN reason=TIME at 8/10 subtests | see below |

Counters of the whole kf3 boot of run 1 (`spec/counters.txt`, line `USER_RING_LIFECYCLE SUMMARY (all relays gone, #1)`): `segments_streamed=1`,
`stream_parts_max=2`, `stream_restarts=0`, `stream_restarts_max=0`, `stream_publish_retries=0`, `window_entry_max_bytes=1042284` (the exact length that was
refused before), `refused_scan_budget=0`; `USER_RING_REFUSED` 0, host Xid 0, guest resets 0. The only streamed segment falls in the creo-04 part of the log (per-channel
aggregation by log line, `spec/agg.py`; the log has no clock). Run 2 (enscape-01 alone): `segments_streamed=0`, no refusals, host Xid 0 (`spec/counters_hw2.txt`).
Falsifier of the hypothesis "creo-04 still resets/TDRs with Phase 2" did not hit in this one run (n=1); it does not prove the old reset was the scan budget.

enscape-01 run 1 (MEASURED, `spec/enscape_events.txt`): the guest shut itself down 9 minutes into the viewset: System event 1074, process
`wlms.exe`: "The license period for this installation of Windows has expired. The operating system is shutting down", 62 minutes after the boot (the harness'
Windows 11 evaluation image; earlier note: boots must stay under about 55 minutes). Not a reset, no refusal, host Xid 0: INFERRED to be a harness/licence artifact, not kayfabe.
Run 2 (own boot, copy and kf3 phase repeated): enscape-01 reached 8 of 10 subtests (`09_RhinoIntWhiteOutlineHigh` not completed) when the stage's cut budget (2813 s,
because the deadline was set for the whole run) ran out: NOT_RUN TIME, which is RETRY and not a verdict; no reset, no refusal. A full enscape-01 pass is NOT shown.

Harness notes: the Worker double (`spec/double.py`) must answer the helper's `/<prefix>/bench-data/<run>/<name>` path and send `Content-Length: 0`, else curl exit 56/404
(my fault, two failed attempts). A tag-specific gate log is required by the stage preflight (`hw2_windows.log` was a copy of `hw1_windows.log`; same overlay).
A pkill on `qemu-system-x86` by me killed the stage's own copy guest once (about 10 minutes lost); never match by comm.

## B. Proprietary image (private R2 `windows-proprietary/kfimg-20261112/image.zip`), offline, on a throwaway overlay
- Downloaded with a presigned GET, `propimage.unpack` verified every member (sha256) and the seed; clock override 17:17:00Z => rtc base 17:49:48Z, guest read 17:52Z (MEASURED).
- **swtpm**: Ubuntu 22.04's swtpm 0.6.3 / libtpms 0.9.3 cannot read the image's TPM state ("Minimum version 4 higher than implementation version 3", MEASURED); swtpm/libtpms 0.10.1 built from source under /opt/tpm worked.
  The TPM EK hash matched.
- Identity (`propimage.py verify`): SMBIOS, serials, MAC, disk serial, MachineGuid, TPM EK hash, CPU profile, clock window MATCH. GPU: **Code 43 on the 4090 host** at first (the image's driver was set up on a 4070; kf3 shows
  the guest a new display device instance `0002` whose class key has no `EnableGpuFirmware`; MEASURED: key present on `0001` (4070) and absent on `0002`). Setting `EnableGpuFirmware=1` on `0002` in the overlay and
  restarting the device gave `nvidia-smi` = seed GPU UUID, driver 580.88 (MEASURED). So the image is die-bound unless the key is set per new instance (`gpu_fix` in `propimage_check.py`).
- Headless question: the KF3_* flags were exported in every boot here (broker TEST backend, no window); a boot WITHOUT the flags was not run on this box, so "the headless path needs the flags" is not re-measured here.
  The broker test backend gave the guest a monitor and an interactive desktop (screenshots work).
- Apps (screenshots `propimage/shots/`, UIA dumps `propimage/uia/`): SolidWorks Design Standard 2026 SP5.2 "Academic Use Only" starts offline (window title in `uia_solidworks.txt`), DaVinci Resolve 21 starts to the Project Manager,
  3DMark starts a Chromium (CEF) window (UIA exposes only two panes: not automatable by UIA without enabling Chromium accessibility; INFERRED idea: the CEF `--force-renderer-accessibility` switch, untested),
  Superposition launcher is a Qt window with no UIA children, Black Myth Wukong Benchmark Tool: its Steam launch dialog (with a Play button) came up (Steam signed out; not signed in, the tool itself was not run).
  eDrawings dump in `uia_edrawings.txt`.
- **Superposition real run** (MEASURED): launcher, preset 1080p Medium, DirectX, clicked RUN by a pixel derived from the window rectangle: `results/log.html` "Benchmark running 18:22:44 ... Benchmark stopped 18:25:41" (FILE), result screen (`shots/sp_after.png`, SCREEN):
  score 7685, FPS min 15.05 / avg 57.49 / max 69.96, GPU RTX 4090 (AD102), driver 580.88. SAVE was not pressed in time (the click landed after the launcher had closed): no saved result file.
- NOT run: 3DMark Time Spy + SAVE, SolidWorks part rotation, DaVinci Resolve project and render, Black Myth benchmark itself.
- `scripts/bench/windows/propimage_check.py` (one python file) ran the unattended pipeline once (`propimage/run1/result.md`): unpack, clock, overlay, boot, ssh passed; verify and gpu failed on that run because the GPU fix was not yet in the script
  (fixed afterwards, then not re-run end to end: out of time); app rows are LAUNCH_ONLY (screen evidence of a launch is never a PASS). The script was NOT re-run to a full unattended pass.
