# Windows TDR hunt, 2026-10-10 (branch `claude/tdr-hunt-20261010` from `integration/windows-20261010` eff1b692 = code 459da55d)

STATUS: RESEARCH (live notes; see "Result" at the bottom when it exists). Host RTX 4070, driver 595.91.07.
Labels: **[measured]** = a log line / counter / guest event with its run; **[inferred]** = reasoning not yet tested.

## Runbook (host-only tooling outside git, `/var/lib/kf-windows-20261005`)

* Runner used for runs 264+: `tooling/tdr-run.sh` (copy of `winprod/winprod-run3.sh`; host copy `tdrhunt/tdr-run.sh`). It
  switches IOMMU group 11 to identity, launches `kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod2.sh run N`
  (binary `kf3-bins/$KF3_REV_BIN`, default 459da55d), scripted sign-in + Edge + Shorts URL, then holds `HOLD_SECS`, pressing
  "down" every 20 s, logging the TDR (GSP cycle) count against the host clock every 5 s (`tdr-timeline.txt`), then collects
  guest events through QGA, optionally decodes a live DxgKrnl ETW session (`ETW=1`), stops the guest, restores DMA-FQ.
* Launch: `env KF_GUEST_PW=<test pw> ETW=1 HOLD_SECS=600 RUN_WIN_FLAGS="KF3_X=1 ..." setsid bash -c 'flock -o /tmp/kayfabe-fastguest.lock bash tdrhunt/tdr-run.sh N'`.
  Output: `winprod/runN/` (winprod.log, evidence.log, tdr-timeline.txt, guest-*.txt, etw-*.txt, screenshots), QEMU log
  `boundary-kayfabe-N/qemu.log`.
* After a stop: `rm -f /tmp/kf-stop-winprod`; `cat /sys/kernel/iommu_groups/11/type` must say DMA-FQ; `pgrep qemu-system-x86` empty; GPU on `nvidia`.
* `qemu.log` has no wall clock; `mem t=<s>` lines are seconds since the device was realised (QEMU start). Guest UTC equals host UTC.
* Offline guest event log of a finished run: `qemu-nbd -r -c /dev/nbd0 boundary-kayfabe-N/windows.qcow2; ntfs-3g -o ro,norecover /dev/nbd0p3 /mnt/x`, copy
  `Windows/System32/winevt/Logs/System.evtx`, read with python `Evtx` (`tdrhunt/evx.py`); then `umount; qemu-nbd -d /dev/nbd0`.

## Correction to the record before the hunt starts [measured, run 263 System.evtx read offline]

`traces/windows_prod_20261010/` calls run 263 "READY, 7 TDR resets in ~8 min". The guest's own System log says more:
* nvlddmkm event 153 ("Resetting / Reset / Restarting TDR occurred on GPUID:6") at 01:05:03 (host t=60 s, 9 s after the logon event 7001),
  at 01:05:25 (t=82..84 s, 2.5 s after the Edge double-click) and at 01:08:24 (t=261 s, during the Shorts page). kayfabe's `qemu.log`
  shows GSP cycles at mem t = 60.4, 84.1, 261.4, 276.2, 280.3, 285.4, 289.8 s.
* **The guest bugchecked at 01:09:12 (t=309 s): `0x116 (0xffff9f81f8198010, 0xfffff8015f744580, 0x0, 0xd)`**, rebooted, and the harness's "READY alive=1" and
  ACPI stop belong to the rebooted guest. The last five resets (261, 276, 280, 285, 289 s) are inside one TdrLimitTime window (5 in 29 s):
  **[inferred]** the default `TdrLimitCount=5 / TdrLimitTime=60 s` rule turned the cluster into the bugcheck.
* The Windows 4101 event ("stopped responding and has recovered") count is 0; the only guest-visible TDR record is nvlddmkm 153.

## Hypotheses and falsifiers, written BEFORE run 264 (diagnostic: `KF3_COMPLETION_PROBE=1500 KF3_PT_STALL_SNAPSHOT=1` + live DxgKrnl ETW)

Common premise: every TDR coincides with a new GPU client appearing (logon -> DWM/shell, Edge's GPU process, Shorts playback) [measured, run 263 times above].
* **H-A (Translated ring, kernel CE/paging)**: a Translated ring (the kernel copy channel, token 0x80c, or the GR ring) does not retire a submission, or
  retires it and the guest never sees the release. Predicts a `PROBE-DUMP ... HOST-FENCE-OVERDUE` / `GUEST-SILENT-AFTER-COMPLETION` line in `qemu.log`
  within 2 s before the first `Running -> Suspending`, and an ETW DMA packet of the paging node with a start and no stop. Falsified if neither appears.
* **H-B (Passthrough twin of a new user process)**: a twin's host channel stalls (GPGet != GPPut at the stall, an un-retired semaphore acquire, a pushbuffer the
  twin cannot read). Predicts a `PT-SNAP BEGIN stall` with some twin not drained, ETW unfinished packets of a 3D/compute node. Falsified if every twin is drained
  and the unfinished ETW packets belong to the paging node.
* **H-C (lost interrupt/notification)**: host work finished and the guest's fence words are visible, but the guest never ran the completion DPC. Predicts: no
  PROBE-DUMP, twins drained, an ETW packet with a start and no stop whose ring state in kayfabe is complete, and CPU_INTR leaf bits set (`irq[...]` counters
  raised > serviced). Falsified by H-A or H-B holding, or by every ETW packet completing and the TDR being a driver-internal timeout.

## Run 264-266 results so far (diagnostic flags `KF3_COMPLETION_PROBE=1500 KF3_PT_STALL_SNAPSHOT=1`; 264 also a live DxgKrnl ETW session)

| run | build | flags beyond production | outcome |
|---|---|---|---|
| 264 | 459da55d | COMPLETION_PROBE, PT_STALL_SNAPSHOT, live ETW (non-circular, 512 MiB) | desktop + Edge welcome page rendered; 8 TDR resets in 13 min (nvlddmkm 153 triples at 08:56:49, 56:58, 57:16, 57:24, 09:00:15, 00:28, 00:37, 09:06:31); a WATCHDOG live dump (bugcheck code 0x117 = VIDEO_TDR_TIMEOUT_DETECTED) at the first; guest alive at the end; ETW hit its 512 MiB cap at 08:56:46.8 (2.5 s BEFORE the first TDR): no stall captured |
| 265 | 459da55d | same, ETW stopped by a watcher that fired on a BOOT-time cycle (bug, fixed in `tdr-run.sh`) | boot-time TDR at mem t=18 s (live dump 0x117 in the guest) and a second cycle at t=53.7 s; then the display stayed black and the guest wedged: no BAR0 write after `trapped=259047`, QGA dead at the end, QEMU had to be killed; no sign-in effect |
| 266 | 459da55d | same, ETW arm failed (QGA timed out at t~35..63 s) | first TDR at t=74.8 s, 5 s after the sign-in keys; second at t=84.0 s |

**[measured, runs 263/264/266, `USERD relay released` lines at each twin's free]** For Passthrough (relayed, Windows user-work) twins that received
at least one submission, the guest-visible `GP_GET` is BEHIND the host's while the host twin is drained (host GET == PUT): run 263 77 of 79 twins,
run 264 157 of 161, run 266 62 of 62. The lag is the last submission(s) (typically 0x11 = 17 entries, up to 0x22+). Production has
`KF3_RELAY_GET_REFRESH` OFF, so the guest's `GP_GET` of a relayed twin is only ever updated at a doorbell, from the engine's value read at doorbell
time (`docs/design/V3_USERD_RELAY.md` §2.2).

**H-D (written before run 267):** a Windows guest thread that waits for a relayed channel to go idle (`GP_GET == GP_PUT` in its USERD) never sees it,
because kayfabe never refreshes the guest's `GP_GET` after the last doorbell; the thread holds guest locks, the GPU scheduler's fences and the
display flips queue behind it, and Windows' TDR timer then fires (nvlddmkm 153, `0x117`). Predicts: with `KF3_RELAY_GET_REFRESH=1` (refresh on every host
non-stall wake and on the worker tick; code exists, default off) the guest's `GP_GET` equals the host's at every twin's free and the TDR cadence
drops. **Falsifier:** with the flag on, `GP_GET` agrees at free but the first TDR still comes within ~10 s of the sign-in keys and the 15-minute TDR count is not below the
baselines above (263: 7 in 5 min then bugcheck; 264: 8 in 13 min).

## Run 267 (H-D test) and the guest's own TDR-time snapshot (WATCHDOG live dumps of runs 264/265/266)

**H-D FALSIFIED as the cause of the first TDR and of the cluster.** Run 267 (`KF3_RELAY_GET_REFRESH=1`, no other measurement flag, 459da55d): the lag is gone
[measured: 0 of 46 submitted twins behind at free; 0 of 72 `act disable channels` samples behind, against 104 of 123 in run 266 and 157 of 161 at free in 264], but
the first TDR still came 5 s after the sign-in keys (mem t=58.0 s, sign-in sent t~53 s), the second 5 s after the Edge double-click (t=84.7 s), and the same
cluster appeared 81..102 s after READY (tdr_cycles 2 -> 6 by hold_t=102). (The GET refresh is still a fidelity fix: see "refresh" notes below.)

**[measured] What declared the first TDR, in all three runs with a dump.** The guest generated a WATCHDOG live dump (bugcheck code 0x117 = VIDEO_TDR_TIMEOUT_DETECTED)
at its first TDR in runs 264, 265 (a BOOT-time TDR, mem t=18 s) and 266. The dump header's context record is the thread that was writing the dump; its stack
(Volatility 3 + the Microsoft public symbols of ntoskrnl/dxgkrnl/dxgmms2 only; host-side tools `dumpcfg/plugins/windows/tdrctx.py`, `symz3.py`, outputs `dumpcfg/ctx26[456].txt`) reads,
innermost first, identical in all three:

    IoCaptureLiveDump ... dxgkrnl!TdrCaptureLiveKernelDumpCallback <- dxgkrnl!TdrCollectDbgInfoStage1 <- dxgkrnl!TdrAllowToDebugTimeout
    <- dxgkrnl!TdrIsRecoveryRequired <- dxgmms2!VidSchiReportHwHang <- dxgmms2!VidSchiCheckFlipQueueTimeout <- dxgmms2!VidSchiCheckHwProgress
    <- dxgmms2!VidSchiScheduleCommandToRun <- dxgmms2!VidSchiRun_PriorityTable <- dxgmms2!VidSchiWorkerThread

So **the TDR is declared by the VidSch FLIP QUEUE timeout, not by an engine-node (copy/graphics) timeout**: a flip that Windows queued for a display source was not
retired within the TDR delay. (Run 232's paging-node picture was taken under a 30 s TdrDelay and an older build; it is not what the default-delay production
TDR looks like.) Other threads in the run-264 dump: LogonUI waiting in `dxgkrnl!DxgkWaitForVerticalBlankEventInternal`, dwm in `DxgkSetSyncRefreshCountWaitTarget`,
the VidSch worker threads idle in `VidSchiWaitForSchedulerEvents` (nothing else stuck in dxgkrnl/dxgmms2 locks).

**[measured, kayfabe side, runs 264/266]** The display model reports, immediately before each first TDR, "`display: +N ms the console shows no new frame: a lit head has no window`" and then
"`... BLACK 1920x1080 (the scanout shown is lost: a lit head has had no window past the hold)`" (run 266: t=74.4 s and 74.6 s; TDR teardown at t=74.8 s, 5 s after the sign-in).
No `HOST-FENCE-OVERDUE` probe, no `DEAD`, `unreconciled=0`, no Xid: no engine ring is behind.

**H-F (next, written before run 268):** a window flip queued by Windows (LogonUI -> desktop transition, Edge launch) is never retired because kayfabe's display model does not
produce the completion the KMD waits for (the VSync/LAST_DATA edge after the window update, the flip's notifier/semaphore release, or the core-update completion) while the head is
lit but its window is detached/unscanned. Run 268 collects: DxgKrnl ETW (circular, stopped by a guest task at the first nvlddmkm 153) naming the flip queue packet that never stops,
and `KF3_DISPLAY_WRITE_TRACE` + `KF3_DISPLAY_METHOD_TRACE` for the display writes/methods around it. Falsifier: the unfinished ETW packet is not a flip (MMIOFLIP/flip-queue) packet, or the
display trace shows the flip's notifier/release/VSync was delivered inside the TDR window.

**Answers to the coordinator's refresh questions (code reading + measurement):** (1) `relay_refresh_all` runs on a worker (a) on every host non-stall wake (`on_other` of the engine tag,
before the guest interrupt) and (b) from `on_other(TICK_TAG)` once per worker loop that parks, i.e. at most every `PARK_MS` = 50 ms when a worker is idle (and `OWED_PARK_MS` = 1 ms when
the non-stall relay owes a raise); the tick is skipped on a loop iteration that found work (`continue`), which is why (a) matters under load. The refresh keeps no "owed" state:
every tick compares the engine's GP_GET to the last value stored and stores if different, so it keeps catching up for as long as any worker loops, and cannot go idle while a lag remains;
a relay whose lock a doorbell step holds is skipped that tick and caught up on the next. Measured at two instants only (free, disable): 0 lag with the flag, ~85-100 % lag without; a time series
needs the `RELAY-LAG` line added in this branch (probe thread, 100 ms samples, summary per 2 s; first used in run 268). (2) The refresh stores only the engine's value (bounded to
`[0, entries)`), never reads the guest's GP_PUT back into the twin, never rings; relay-lock takers are workers (`relay_serve`, `relay_refresh_all`, try_lock) and the act thread (`drop_relay`, after the
host channel is gone, and the disable snapshot, try_lock); no vCPU or drainer path takes it (doorbell trap only sets the token bit).
