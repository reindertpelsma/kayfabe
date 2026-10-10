# Windows TDR hunt, 2026-10-10 (branch `claude/tdr-hunt-20261010` from `integration/windows-20261010` eff1b692 = code 459da55d)

STATUS: RESEARCH, 2026-10-10 — round 3 in progress (runs 276-284): shape F measured, ordering race and wrong post-latch state separated; no fix yet. Latest at the bottom. Host RTX 4070, driver 595.91.07.
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

## Runs 268/269: the guest goes silent for ~2 s with VSync interrupts pending (measured), then the flip queue times out

Instruments added in this branch (diagnostic, probe thread only, `KF3_COMPLETION_PROBE`): `RELAY-LAG` (guest GP_GET trail of every relayed twin, 100 ms samples), `VCPU-MAX` / `VCPU-STUCK`
(a vCPU inside one BAR0 write handler for >200 ms is named). Binary `fcafeb2e` (= branch tip at that commit; built on the host from the branch).

**[measured, run 268, `KF3_DISPLAY_WRITE_TRACE`, binary a4b96ee0]** Time series, without the GET refresh: 11 of 17 relayed twins are behind in steady state (cumulative 64058 of 79832 twin-samples lagged,
max lag 1374 entries, 57 runs of >= 500 ms) - the lag does not heal by itself without the flag (and with it, 0 at free/disable). **It is not the TDR cause (run 267).**

**[measured, runs 268 and 269, kayfabe display trace + guest ETW]** At the first TDR of the run, with kayfabe's display model healthy and delivering:
* kayfabe raises a VSync (LAST_DATA, `evt=0x6 en=0x2 rm=0x2`) every 16.7 ms throughout, and the guest acked every one (`WRITE 0x611800 <- 0x2`) up to a point, then **acks none for ~2.2 s**
  (run 268: 58898.8..58900.85; run 269: 59541.9..59544.1 maplog seconds, 30 VSyncs raised per 0.5 s the whole time), then acks again and the guest starts its own TDR teardown.
* The guest's DxgKrnl ETW (run 269, circular session, 10:11:01.862 -> 10:11:07.585 UTC) agrees to the millisecond: the last VSync interrupt event at 10:11:01.862, then **NO DxgKrnl event of any kind for
  2.34 s** (no VSync, no packet, no queue event), then three small events, a gap of 1.7 s, and the recovery's paging burst from 10:11:06.5. All DMA packets that had started had stopped
  (`DMA starts 1586 stops 1586` before the recovery; the "never stopped" list is only the recovery's own tail). So nothing on the engines is outstanding: the flip queue's retiring VSync is simply never serviced.
* During the silence the CPU_INTR interrupt-tree write counter is flat (no ISR runs; `irq[writes=]` unchanged over 100 ms) while `raised` grows ~18 per 100 ms and `leaf0=0x6 leaf4=0x4000000` stay pending (probe dumps).
* `VCPU-MAX` stays at 131 us through the stall and there is no `VCPU-STUCK`: **no vCPU is inside a kayfabe BAR0 write handler**. `unreconciled=0`, no `HOST-FENCE-OVERDUE`, no host Xid.
* The last guest action before the silence, in both runs: one RPC `GSP_RM_CONTROL` that kayfabe answers as UNSERVICED (`GSP rpc UNSERVICED { code: 76 }` 15-20 ms before the last ack); the per-control refusal counters
  of the two status lines around it differ in exactly one entry: **`fn76/0x007302a5` (NV0073, display) +1** (run 269). 0x007302a5 is in no OGKM header and is refused by name since run 30
  (`traces/windows_code43_walls_20261007/README.md`: "issued ~6 times per boot ... not on the TDR's path"). Whether the silence is a consequence of that refusal or only coincident is NOT established:
  inferred only.

**Not yet known (what the next run measures):** what the guest's CPUs are executing during the silence. Plan (run 270): `STALLDUMP=1` (this branch's `tooling/tdr-run.sh`): when the display trace shows
VSyncs raised and no ack for 600 ms, take six `stop; info registers -a; cont` samples of every vCPU and a full guest-memory dump at the stall, resolve RIPs with the Microsoft symbols (ntoskrnl/dxgkrnl/dxgmms2) and
read the thread stacks with the Volatility tools (`windows.tdrctx`/`waitfast`). Falsifier of "the guest is spinning/blocked in its own driver and the interrupt cannot run": all vCPUs idle (HLT) with the
vector pending (then the interrupt delivery path is the culprit: MSI-X routing/mask/irqfd).


## RESULT (8 hardware runs, 264-271; evidence in `evidence/`)

| run | build | flags beyond production | what it was for | outcome |
|---|---|---|---|---|
| 264 | 459da55d | COMPLETION_PROBE, PT_STALL_SNAPSHOT, live ETW (non-circular) | first look | 8 TDRs / 13 min; WATCHDOG dump (0x117); ETW capped before the stall |
| 265 | 459da55d | same, bad ETW watcher | boot TDR t=18 s + one at t=53 s; then the display stayed black and the guest wedged, QEMU killed |
| 266 | 459da55d | same | 7 TDRs (cluster of 4 at 82-108 s after READY, then 5 calm minutes); WATCHDOG dump |
| 267 | 459da55d | `KF3_RELAY_GET_REFRESH=1` only | H-D test | GET lag 0; first TDR 5 s after sign-in, 2nd 5 s after Edge, cluster at 81-102 s, then NO TDR for 13 min (6 cycles in 15 min). **H-D falsified** (cause of first TDR / cluster) |
| 268 | a4b96ee0 | probe + display traces | display/relay time series | 2 TDRs after sign-in, none later in 4 min; acks stop ~2.2 s |
| 269 | fcafeb2e | probe + display write trace + circular ETW | name the stuck packet | 7 cycles; ETW captured the first TDR: user packets stuck 4.35 s |
| 270 | fcafeb2e | probe + display write trace, STALLDUMP | what the vCPUs do during the silence | all vCPUs in the guest's own live-dump corral (IF=0) |
| 271 | bb53ec57 | probe + circular ETW | pending object + both cursor sides | paging-queue packets pending 2.475 s; host side had completed them |

Not one of the 8 reproduces "once a minute": the TDRs come as a first one 5 s after the sign-in keys, a second one ~5 s after the Edge launch, a cluster 80-125 s after READY (3-5 resets 4-5 s apart) in
263/264/266/267/269/271, and then often nothing for 5-13 minutes (264: one at 461 s; 267: none in 13 min). Run 263's tail ended in a 0x116 bugcheck (5 resets in 29 s trips TdrLimitCount=5/60 s).

### Measured (by run)
1. **The TDR is declared by dxgmms2's flip-queue check** (`VidSchiCheckHwProgress -> VidSchiCheckFlipQueueTimeout -> VidSchiReportHwHang -> TdrIsRecoveryRequired -> TdrCollectDbgInfoStage1 -> live dump`),
   in all three guest dumps (264, 265 = a boot TDR, 266). `evidence/watchdog-dump-run26[456]-stack.txt`. It is not an engine-node timeout.
2. **What is pending at the declaration is queue packets whose host work is already done.** Run 269 (ETW): RENDER/SOFTWARE/DEVICE command-buffer packets of three user processes (pids 0x19DC, 0x18D4, 0xD18) queued
   within 15 ms at 10:10:59.85, no other packet slower than 132 ms in 4412, all stopped only in the recovery 4.35 s later; declaration (= the guest's last VSync event) at 10:11:01.862, i.e. 2.0 s (TdrDelay)
   after they were queued. Run 271: the kernel paging queue's packets 4562-4586+ queued 10:23:17.484 stopped only at 10:23:19.959 (2.475 s); **kayfabe's own log of the same instant:
   `completed fence seq=2774 gp_get=14331 submit->seen=72us seen 1523ms ago` for the Translated paging ring (host GP_GET == guest GP_PUT == 14331), i.e. kayfabe completed it ~1.5 s before; no
   `HOST-FENCE-OVERDUE` anywhere (0 in 271), no `DEAD`, `unreconciled=0`, no host Xid.** (`evidence/run269-etw-summary.txt`, `run271-etw-summary.txt`; log lines in `qemu.log` of run 271.)
3. **Every relayed twin was fully consumed by the host engine at the stall** (RELAY-DUMP of run 271: engine GET == relay host_put == guest PUT for all 22 twins); only the guest-visible GET trails (without
   `KF3_RELAY_GET_REFRESH`), and refreshing it does not change the TDR (run 267).
4. **At the stall the interrupts are PENDING and unserviced, not absent**: `leaf0=0x6` (CE2 vec 1 + CE3 vec 2) and `leaf4=0x4000000` (display) set with enables on, `top=0x5`; the guest's interrupt-tree
   register-write counter does not move for >=0.4 s while `raised` keeps growing (runs 268, 269, 271 PROBE-DUMP device lines); display VSyncs are raised every 16.7 ms and were acked up to the declaration.
5. **Nothing in kayfabe blocks a vCPU**: `VCPU-MAX` 131-332 us for whole runs, no `VCPU-STUCK`.
6. **The silence after the declaration is the guest's own live dump**: at the stall (run 270) all 8 vCPUs sit in `IopLiveDumpProcessCorralStateChange` / `IopLiveDumpBufferDumpData` with interrupts off
   (`evidence/run270-rip-symbols.txt`), which is why the ISR is not running then. The declaration is therefore BEFORE the silence; the stale packets are older than it.

### Ruled out (with the run)
* H-D, stale guest `GP_GET` of relayed twins (267). H-A as an engine stall in the host (no HOST-FENCE-OVERDUE, host rings drained; 264-271). A vCPU blocked in a kayfabe BAR0 handler (269-271 VCPU-MAX).
  A lost VSync (kayfabe raises and the guest acks up to the declaration; 268/269). A CPU_INTR shadow-publication race as the TDR trigger is not excluded in general but nothing showed
  shadow != atomics (not measured directly: no shadow-vs-atomics counter was added).
* A "once a minute" rate; a boot-time-only effect; the ETW itself as the cause (TDRs happen without it: 263, 266, 267).

### Still inferred (not measured)
* Which interrupt the guest never sees for the stale packets: the picture (host done, fence words written by the engine, `leaf0` CE2/CE3 bits pending, packets completing only at the TDR) fits a completion
  notification the guest never acts on (the audit's finding 3 `NotArmed` drop of host non-stall edges, or finding 2's shadow race, or the Translated-CE relay not raising for a batch) — none of them is tied to a
  specific stale packet yet.
* Why the guest does not run its ISR for pending CE2/CE3 bits during the 2 s before the declaration (the live dump only explains the time AFTER it).

### Exact next step
One boot with both sides time-stamped at packet level: (a) kayfabe, for every Translated-CE pump and every Passthrough host non-stall edge, log (vector latched? MSI raised? leaf bit already set? armed?) with the maplog time;
the `NotArmed`/`unvectored` verdicts per engine at the same second as the stall (the counters exist; they are only printed at 2 s granularity); (b) the same circular ETW stopped by the runner at the first TDR (works:
`tooling/tdr-run.sh ETW=1`) so the first stale packet's queue time is known; (c) one `info registers -a` sample 1.0 s before the expected declaration (arm on the packet's queue time) to see what the vCPUs run while the CE2/CE3
bits are pending and no ISR runs. Cheap falsifier first: make a host non-stall edge latch+raise its vector unconditionally (the audit's finding-3 switch) and see whether the first TDR disappears.

### Code in this branch (diagnostic only, probe thread, default off)
`RELAY-LAG`, `VCPU-STUCK`/`VCPU-MAX`, guest-silence `PT-SNAP` + `RELAY-DUMP` (all behind `KF3_COMPLETION_PROBE`), `scripts/bench/windows/dxg_etw_stop_tail.ps1`, `tooling/tdr-run.sh`. `cargo test -p kf-qemu -p kf-chan` on the host: 295 passed, 0 failed
(the local disk was full). No behaviour changed, no fix. Host left clean after every run (DMA-FQ, no QEMU, nvidia bound, stop file removed, Xid 0); big dumps stay on the host under `dumps/` (`run264/265/266-WATCHDOG.dmp`, `run270-stall.elf`) and are not in git.

---
## Round 2 (coordinator reset: runs 272-275; merged `origin/integration/windows-20261010` at `96646daa`; binary `40481dd5` = merge + diagnostics only, no kf-mem change)

### CORRECTION to "Measured 4" above (found while planning round 2)
Every observation of "pending CE2/CE3/display leaf bits and the guest's interrupt-tree write counter flat" (PROBE-DUMP device lines of runs 268/269/271) was taken AFTER the guest declared the TDR and
entered its own live dump (run 270: all vCPUs corralled with IF=0 at +0.1..0.6 s after the last VSync ack; the earliest PROBE dumps after the declaration). Run 269 at 59541.30, before the last ack: `leaf0=0x0 top=0x0`.
So those lines show the dump, not an interrupt the guest failed to take during the 2 s before the declaration. What is still true: packets queued ~2.0 s before the declaration (run 269) / a paging batch (271) complete
only at the recovery, while kayfabe had completed the host side. Whether the guest was *delivered* the completion interrupts in the 2 s BEFORE the declaration is therefore still unmeasured: it is what round 2 samples
(`tooling/irq_sampler.py`: LAPIC IRR/ISR/TPR/PPR per vCPU, MSI-X table + PBA of the device, eventfd counts, RFLAGS.IF, every 250 ms from the sign-in to the first TDR, plus kayfabe's `IRQ-RING`: every interrupt decision
with a UTC millisecond, who decided it and what came of it (raised / held / NotArmed / owed), dumped at the silence).

### Run 272 — H-G (written before the run)
**H-G:** a host non-stall (or FIFO_EVENT_MTHD) edge that the relay judges `NotArmed` is dropped, so the guest is never told that a Passthrough twin's work completed; WDDM then waits for the fence until the 2 s timeout.
**Variable:** `KF3_DIAG_NSI_UNCONDITIONAL=1` (every edge judged armed, none dropped; diagnostic only). Everything else = production + measurement flags that do not change behaviour
(`KF3_COMPLETION_PROBE`, `KF3_DISPLAY_WRITE_TRACE`, the LAPIC sampler).
**Falsifier:** the first TDR still comes within ~10 s of the sign-in keys (as in every run so far: 5-9 s) and the run still shows the 80-125 s cluster; with H-G true the first TDR (and the cluster) disappear or are much rarer. Control: the
`IRQ-RING` of the same run must show `NotArmed` verdicts in the baseline (run 271: `nsi` counters CE2 `wakes=3064 raised=2775`, so ~10 % of edges dropped) and none in this run, else the switch did nothing.

### Run 272 result (H-G, `KF3_DIAG_NSI_UNCONDITIONAL=1`, binary 40481dd5) — INVALID as a test
Two host Xid 31 (MMU FAULT_PTE, channels 0x3c GR0_PBDMA read @0x1499c000 and 0x36 GRAPHICS write @0x040fc000) hit at boot (t=16 s and 27 s, the two `Running -> Suspending` cycles at mem t=20.4/29.1 s, no nvlddmkm 153
in the System log), then the display stayed black and the guest wedged (QGA dead, no screenshots after sign-in): no TDR after the sign-in because there was no desktop. Not a result for H-G (a boot fault run, like 265). H-G is neither confirmed nor refuted; needs a clean replicate after the flip ledger.

### Owner challenge (via coordinator): "is every flip answered? trace it. dependency not done = buffers we allocated for the flip hanging"
**What the code says (read before measuring):** `kf_disp::Engine` stops a window channel at its UPDATE until (a) its interlock group is complete, (b) the head's next vblank (non-tearing flip), (c) its ACQUIRE semaphore holds its value in guest memory
(`latch_group`: the whole group keeps waiting if any acquire fails; re-polled every 2 ms). On completion it arms and states notifier / release semaphore / GET as `Effect`s; the plane (`display.rs`, step 6-7) queues each with `need: scan.barrier`
and delivers it only when scanout copy number `need` is done (`scan.done`): **a flip's completion is gated on the console frame copy to the host broker**, and the copy needs a broker frame slot the host compositor window releases.
The log shows the broker reclaiming slots that were never released (`broker: reclaimed frame slot N: no RELEASE X ms after its commit`): [measured, runs 263-271] in every run the first TDR is preceded (within ~1000 log lines) by such a reclaim of
4.5-4.7 s (264: 4701, 266: 4493, 267: 4723, 268: 4703, 271: 4700), and the cluster resets by reclaims of 1.8-2.4 s (about TdrDelay); runs with few TDRs have few reclaims (272: 7). The release comes from a real Wayland compositor window on the host.
**What was missing:** a per-flip ledger. Added (always on, plain counters on the display thread; no vCPU/drainer cost): `FLIP-LEDGER` every 2 s (`chN: committed/completed (max ms, slow, acquire-blocked, pending age)`, completions queued/oldest/delivered/slow/max wait,
copies started/done), `FLIP-SLOW` for every update > 50 ms commit-to-complete (what blocked: acquire ctxdma/offset/value, update data, assembly surface/semaphore/notifier ctxdma), `FLIP-QUEUED-LAG` for every completion that waited > 50 ms behind the console copy.

### Run 273 — H-L (written before the run; binary 735b352e = merge + diagnostics, production flags + `KF3_COMPLETION_PROBE`)
**H-L:** the flip the guest queues about 2 s before its first TDR is committed (UPDATE reached the engine) but its completion (notifier / release / GET) is delivered late because it waits behind the console copy (no free broker slot / slot not released),
or it waits on an acquire / has no window to latch. **Falsifier:** around the first TDR (5-9 s after the sign-in keys) every committed update completes < 50 ms after commit (no `FLIP-SLOW`) and every queued completion is delivered < 50 ms after it was queued (no
`FLIP-QUEUED-LAG`); then the answer path is exonerated and the hunt returns to the paging/user-queue completions.

### Run 273 result (H-L): the display answer path is EXONERATED for that TDR (binary 735b352e, production + `KF3_COMPLETION_PROBE`)
`FLIP-LEDGER`, the whole run: window 0 (ch1) 367 UPDATEs committed / 367 completed (slowest 17 ms), window-imm (ch33) 355/355 (max 0 ms), core (ch0) 39/39 (max 0 ms), acquire-blocked 0, `FLIP-SLOW` 0, `FLIP-QUEUED-LAG` 0, every queued completion
delivered within 1 ms (`max_wait=1ms`), console copies started == done. The two TDRs of this run (nvlddmkm 153 at 10:58:09.5 and 10:58:14.5, mem t=18.3 / 23.3 s: BOOT-time, before any sign-in) happened with the display engine at
`pend 0 ms` on every channel (ledger lines 2 s apart before and after). So: no unanswered flip, no flip held behind the broker's frame slots (the broker reclaims of 1-4 s exist: 9 in this run, none delayed a completion), no acquire waits.
Third cycle at t=55.8 s (the sign-in) has no nvlddmkm event (an ordinary driver restart); none after it in 240 s. [measured]
=> The flip the guest's VidSch times out on is not waiting for kayfabe's display answer. It waits on its own dependency (the render/device packets pending in run 269; the paging batch in 271) — consistent with the owner's "dependency not done". **Falsifier of H-L met: H-L refuted** for this run.
Note: TDR timing varies run to run: first TDR at BOOT (t=18 s: 265, 272-cycle, 273) or 5 s after the sign-in (263, 264, 266-271).

### Run 274 — interrupt-delivery state at the TDR (coordinator step 2; written before the run)
Binary 735b352e. Flags: production + `KF3_COMPLETION_PROBE=1500` (IRQ-RING) + `KF3_DISPLAY_WRITE_TRACE=1` (the last VSync ack = the declaration time) + the LAPIC sampler from the QGA answer (every ~0.3 s: per-vCPU IRR/ISR/TPR/PPR, RFLAGS.IF/HLT, the
device's MSI-X table + PBA, eventfd counts). Decision tree (coordinator): vector in IRR but undelivered with IF=1 -> guest/LAPIC state; vector never in IRR while the ring shows raises -> our delivery path (MSI-X mask/PBA, irqfd routing,
coalescing, NotArmed drop); vector in ISR never EOI'd -> guest ISR stuck / level semantics. **Hypothesis H-I:** during the ~2 s BEFORE the declaration, the CE2/CE3/GR/display vector is raised by kayfabe (ring: `res=0` messages) while no vCPU's IRR/ISR ever holds
vector 0x62 for a stale packet's completion, i.e. the eventfd/irqfd path loses or coalesces them (eventfd count > 0 or PBA bit set at the stall). **Falsifier:** the vector shows in IRR/ISR and is serviced (ISR/EOI cycles) throughout the 2 s window.

### Run 274 result and a revision-linked finding (binary 735b352e = integration aeda9ffd = the FIRST, unfixed batched-map decisions code, + diagnostics)
Run 274 again produced host **Xid 31 FAULT_PTE** (3 new: CE3_PBDMA0 reads at VA 0x04036000 on channels 0x0200003d/37/3f, GRAPHICS write fault 0x1483b000), a boot-time TDR at t=19 s, no cycle after the sign-in, a slow READY (185 s) and a wedged shutdown; the sampler got `EAGAIN` from QMP (busy) after
its first sample, so there is no LAPIC time series from it (the `IRQ-RING`, `FLIP-LEDGER` and display trace are in `qemu.log`; the ledger again showed nothing unanswered). Host Xid count by run [measured, `dmesg`]:

| runs | binary / code base | host Xid 31 FAULT_PTE |
|---|---|---|
| 260-271 | 459da55d, a4b96ee0, fcafeb2e, bb53ec57 (integration eff1b692 + diagnostics only) | **0** in every run |
| 272 | 40481dd5 (merge of integration aeda9ffd = first batched-map decisions code) | 2 (GR0_PBDMA0 read 0x1499c000; GRAPHICS write 0x040fc000), both at boot |
| 273 | 735b352e (same code base + ledger) | 2 more (cumulative 4) |
| 274 | 735b352e | 3 more (cumulative 7): CE3_PBDMA0 reads 0x04036000 x3, GRAPHICS read 0x1483b000 |

0 faults in 12 runs before the merge, 7 in 3 runs after it: the first batched-map decisions code (review: FIX-FIRST, steer-vs-map race) is the only difference of significance. **Runs 272, 274 are therefore not valid tests of the TDR hypotheses** (they are runs with a mirror fault); run 273 (2 Xids at boot, then clean) is the only usable one of the three and it is the flip-ledger result above. The addresses 0x4034000/0x4036000 are the same twin-space faults recorded in `traces/windows_reset_20261009/` (rows unmapped by a walk while a twin still reads them).

### Run 275 — H-M (written before the run; binary 2da71abe = this branch merged with `origin/claude/batched-map-decisions-20261010` 2540b547, i.e. the FIXED batched-map code; production flags + `KF3_COMPLETION_PROBE` + `KF3_DISPLAY_WRITE_TRACE`)
**H-M:** the FIXED code runs without host Xids and behaves like the eff1b692-era runs (first TDR timing as in 263-271: 5 s after the sign-in, cluster 80-125 s after READY), i.e. the Xid/black-screen runs 272-274 were the unfixed code. **Falsifier:** any host Xid 31 in run 275, or a boot that wedges again. As a data point for the owner's question (does batching explain the black screen / the stale packets): if H-M holds AND the first TDR still comes at the usual time with the display ledger clean, batching is not the cause of the TDR (the eff1b692-era runs 264-271 had the older batched-map code too, and the NO_BATCHED_MAP control was not run: see below).

### Run 275 result (H-M; binary 2da71abe = FIXED batched-map code 2540b547 merged; production + probe + display write trace)
**H-M's falsifier did not fire: no new host Xid** (`xid_lines` 7 before and after; no `failclosed`/`panicked` in `qemu.log`; no `FLIP-SLOW`, no `FLIP-QUEUED-LAG`, ledger `ch1 10/10 max 17 ms, ch0 28/28, ch33 2/2`, `pend 0 ms`).
But the guest was NOT healthy at boot: two `Running -> Suspending` cycles at mem t=19.1 and 24.4 s, a WATCHDOG live dump at 11:19:26 (the boot-time TDR; the same flip-queue-timeout class as 265), and then a **bugcheck 0x113 (VIDEO_DXGKRNL_FATAL_ERROR, 0x2b, 0xffff8601dbbbe5a0, 0xffffd385508b3000, 0)** at ~11:19:44 (t~40 s) and an in-guest reboot
(Kernel-Power 41 + WER 1001 in `evidence/run275-guest-events.txt`); after that second boot no TDR at all in the 4-minute hold. So with the fixed code: no mirror faults (0 Xid) but a boot-time TDR and a 0x113.

### ROUND 2 SUMMARY (runs 272-275; no measured cause)
| run | binary (code base) | change | outcome |
|---|---|---|---|
| 272 | 40481dd5 (aeda9ffd batched-map decisions, unfixed) | `KF3_DIAG_NSI_UNCONDITIONAL=1` + sampler | 2 host Xid 31 at boot, 2 boot cycles, display black, guest wedged; INVALID for H-G |
| 273 | 735b352e (aeda9ffd + flip ledger) | production + probe | 2 boot TDRs (nvlddmkm 153 at t=18/23 s), no later TDR in 240 s; **flip ledger: all UPDATEs answered, `FLIP-SLOW` 0, `FLIP-QUEUED-LAG` 0**; 2 Xid (cumulative 4) |
| 274 | 735b352e | + sampler | 3 more Xid (cumulative 7), boot TDR t=19, wedged shutdown; sampler lost to QMP `EAGAIN` |
| 275 | 2da71abe (fixed batched-map 2540b547 merged) | production + probe | 0 Xid; boot TDR (t=19/24) then bugcheck 0x113 and reboot; no TDR afterwards |

**Measured in round 2:** (1) the display answer path is clean: committed == answered for every window/core channel, nothing waits behind the console copy or an acquire (runs 273, 275). (2) The first batched-map decisions code (aeda9ffd) produces host Xid 31 FAULT_PTE mirror faults
(0 in 12 earlier runs; 7 in the 3 runs on it; 0 in the run on the fixed code); the TDRs themselves are NOT explained by mirror state: before the first TDR of run 271 the mirror counters are flat (`absent_cleared` 3 constant, `unreconciled` 0, `named_missed` 0, no REFUSED line in the 6 s before), and no host fault occurred in 264-271.
(3) The corrected picture of "interrupt pending" (above).
**Not done / not measured:** the LAPIC/MSI-X/eventfd time series at the stall (the sampler works, 2918 samples in run 272, but runs 272-275 were consumed by the Xid/boot-crash problem and one QMP-contention failure); the NO_BATCHED_MAP control; the per-address mirror resolution of the stale packets' dependencies (kf-mem not touched per instruction; no per-VA event ring was added); H-G (unconditional NSI) has no valid run.
**What is ruled out so far (all rounds):** engine stalls on the host (no HOST-FENCE-OVERDUE, rings drained), stale relayed GP_GET (267), blocked vCPU handlers, a lost VSync, the display answer path (flips answered, 273/275), the broker/console-copy gate (ledger), the live dump as cause of the silence (it is the effect).
**Still open:** what the pending packets (user render/device in 269, paging in 271, and at boot the first flip) wait for in the guest: host work is done, completions are not seen until the recovery. Exact next step: rerun the sampler on the FIXED build (2da71abe) in a run that reaches the post-sign-in TDR, starting it from the QGA answer with QMP not contended (no screenshots during the window), and read vector 0x62 in IRR/ISR/eventfd/PBA in the 2 s before the last VSync ack, together with `IRQ-RING` (UTC aligned).

---
## Round 3 (escalated hunt, 2026-10-10 afternoon): the dependency, named from data already on disk (zero hardware runs)

Method: the guest's own DxgKrnl ETW captures of runs 269 and 271 (public Microsoft-Windows-DxgKrnl provider, circular session,
re-decoded event by event with `tooling/etwwin.py`), aligned with kayfabe's `qemu.log` (`tooling/tslog.py`, display-trace time
base; run 269 maplog = UTC + 22879.93 s ± 0.05 s, calibrated on the recovery's `HeadTimingEn(0)` at 59544.138 = ETW recovery at
10:11:04.20 and on the paging ring's last doorbell). Event-ID semantics are those of the provider's own field names
(QueuePacket 178/179/180, DmaPacket 175/176/177, wait 244, signal 245/294/295/297, fence signalled 551/552, paging-queue op
322/324/325, VidSch wait 18/19 with its reason string, FlushScheduler 360, child status 1096/1097).
**The Microsoft public PDBs carry no type information** (`user_types` is empty for dxgkrnl and dxgmms2), so a walk of the
VidSch flip-queue structures "via the PDB types" is not possible; the dependency below is named from the provider's events.

### Run 269 (first TDR after sign-in), measured: the chain, from the declaration backwards
1. Declared 10:11:01.86 by the flip-queue check. The pending flip-queue entries (`FLIPMODE_IMMEDIATE_SW_FLIP_QUEUE` 279, 280, 281)
   are retired only by the recovery at 10:11:04.2026. The last flip the scheduler handed to the driver was submit sequence 279 at
   10:10:57.947 (event 259); kayfabe saw the matching window PUT at maplog 59537.878 and latched it at 59537.894 — every flip
   the guest programmed was latched (no display-side loss).
2. Those flips wait on the render packets of pids 0x19DC / 0x18D4 queued at 10:10:59.853. Each of those contexts has a WAIT packet
   ahead of its render packet: **context 0xFFFF820EC17720F0 waits on monitored fence 0xFFFF820EC1D5FD10 for 7098 and 7099
   (current value 7097, signalled at 10:10:59.854286); context 0xFFFF820EBCA0B310 waits on 0xFFFF820EBBFD2A10 for 7234 (current
   7231, signalled 10:10:59.854988)**. Both fences are **VidMm paging fences of the process devices, signalled by the CPU**
   (event 551 by the VidMm worker thread when a paging-queue operation finishes), not by a GPU semaphore write.
3. The operations that would signal 7098/7099 and 7232-7234 are VidMm paging-queue ops 97, 98 (pid 0x19DC, MakeResident),
   231-233 and 2, 3 (pid 0x18D4), queued at 10:10:59.853. **The VidMm worker never processes them** until 10:11:04.2035 (the
   recovery), where each one completes in < 1 ms without a GPU packet and signals 7098, 7099, 7232-7234 at once.
4. At 10:10:59.8537 pid 0xD18 issues a display child-status query (1096/1097, NonDestructiveOnly = false, success in 0.26 ms) and
   at 10:10:59.853989 **`DXGADAPTER_FLUSHSCHEDULER_SUSPEND`**. It queues a DEVICE command buffer at .868602; the VidSch worker takes it
   and enters **`VIDSCH_WAIT_COMPLETION` at 10:10:59.868825 and stays there until 10:11:04.203000**; the matching
   `FLUSHSCHEDULER_RESUME` is at 10:11:04.203472 (after the recovery). The scheduler is suspended for the whole TDR window.
5. Kayfabe, same instant (maplog 59539.776-59539.793 = UTC .846-.863): the guest disables **every** Passthrough twin
   (`DISABLE_CHANNELS(bDisable=true)` with an async-preempt event, 22 calls, client 0xc1d00002), kayfabe answers NV_OK and posts
   22 `RUNLIST_PREEMPT_COMPLETE` events — and **no `bDisable=false` follows**; afterwards the guest issues only periodic
   performance controls until the recovery. In healthy suspend/resume cycles of the same runs the `bDisable=false` list follows
   the disable list immediately (e.g. run 269 lines 13987-14149: D+ x18 then D- x18).
6. Ruled out for this TDR: render work outstanding on the GPU (854 of 854 render submissions 450/451 paired before the suspend,
   p99 1.4 ms), paging DMA outstanding (last paging DMA 1809 completed 10:10:59.854888), a lost VSync, the display answer path,
   **and family B (data not visible): the fences the stuck packets wait on are CPU-signalled paging fences whose values are
   behind because the CPU-side producer never ran, not GPU-written words that failed to land.**

**Named dependency (run 269):** the timed-out flip waits on render packets that wait on two VidMm paging fences (values 7098
and 7234), which wait on paging operations that the scheduler never lets run because a `FlushScheduler(SUSPEND)` from a display
child-status poll never completes; the suspend's own wait (`VIDSCH_WAIT_COMPLETION`) coincides with the guest driver's
preempt-all of the 22 twins, answered by kayfabe with 22 `RUNLIST_PREEMPT_COMPLETE` posts and never followed by the re-enable.
**[inferred, not measured]** the suspend waits for the driver to report the preemption complete; whether the guest consumed the
22 posted events (the async-preempt KEVENTs in guest memory, whose addresses are in kayfabe's log as `eventData`) is the
unmeasured link.

### Run 271 (its ETW-captured TDR), measured: a second shape, no suspend
* Paging queue packet 4562 (and every later one, queued from 10:23:17.4848) never gets a DMA start; the last paging DMA (1664) starts
  10:23:17.488136 and completes .489154. No `FLUSHSCHEDULER_SUSPEND` anywhere near it.
* The VidSch worker's last events: a flip handed to the driver at 10:23:17.4884 (present 0x12D: events 530/382/541), one event at
  .5528, then **no wait event (18/19) and no submission until the recovery at 10:23:19.9615** — it is not waiting in VidSch's own
  wait; the flip queue entry 471 (`MMIOFLIP`, queued .566974 by dwm) and two dwm render submissions are victims.
* So in this shape the scheduler thread stops right after handing a flip to the driver. Not yet named: what it is blocked on.

### Both shapes across the runs (kayfabe-side signature: a disable list not followed by its enable list before the cycle)
`evidence/disable-enable-vs-tdr-runs263-271.txt`. "S" (preempt-all not re-enabled before the reset): 263 #1, #2; 264 #3, #4;
266 #3; 267 #2; 269 #1, #2; 271 #2. "F" (only balanced lists before the reset): 264 #1, #2; 266 #1, #2; 267 #1; 268 #1, #2;
271 #1 (the ETW one); 273/275 boot TDRs. The first TDR after the sign-in keys is F in 264/266/267/268/271 and S in 263/269.

### Next measurements (proposed; each with its falsifier)
* **S:** a host-side trigger on "disable list without its enable list for 300 ms" takes `dump-guest-memory` (before the 2 s
  declaration) and reads the `eventData` KEVENTs of the open list (public `_KEVENT`, ntoskrnl types): **H-S** "the guest never
  consumed the posted preempt-complete events". Falsifier: every open KEVENT is signalled (SignalState = 1) or has no waiter.
* **F:** `KF3_BAR0_READ_TRACE` on the display range plus the interrupt tree, ETW on: **H-F2** "during the 2 s the driver thread that
  received the flip polls a BAR0 register kayfabe never changes". Falsifier: no BAR0 offset is read repeatedly in the window
  (then the wait is on guest memory or a lock, and the same S-type dump at the declaration names the thread).

### Shape S, offline reading before run H-S (coordinator's items 1-2)
**How kayfabe delivers a `RUNLIST_PREEMPT_COMPLETE` (code).** `ChanPlane::disable_channels` (act thread) runs the host disable +
preempt and pushes `(client, eventData)` on `preempt_done` (bounded 64) only after the HOST verb returned. The drainer, when its
privileged ring is empty, calls `Device::deliver_preempt_complete` (`crates/kf-qemu/src/device.rs`): under the GSP lock and only
while no reply is held (`chan::preempt_posts_ready`: the event follows its control's reply, as vfio-10 shows), it encodes one LIST
`POST_EVENT` per completion (`kf_abi::postevent::SubdeviceNotify`: hClient/hEvent = the guest's live registration for notifier 139,
notifyIndex 139, data 0, info16 0, status 0, eventDataSize 8, bNotifyList 1, eventData = the guest's own `pRunlistPreemptEvent`),
writes it into the status queue (`GspFsm::post`: elements first, writePtr last, `swgen0_pending`), publishes the registers
(IRQSTAT SWGEN0) and latches the GSP vector (`GSP_STALL_VECTOR` 0x9b) once per pass. `QueueFull` requeues the tail in order.
**Contract (open ogkm 595.84, `kernel_gsp.c` / `kernel_gsp_tu102.c`).** On the GSP interrupt `kgspService_TU102` clears SWGEN0
(IRQSCLR) BEFORE servicing, then `kgspRpcRecvEvents` drains **every** pending element until the queue is empty; in addition every
synchronous RPC's `_kgspRpcRecvPoll` processes all events queued ahead of its reply. `_kgspRpcPostEvent` needs
`CliGetEventInfo(hClient, hEvent)` (else the element is dropped with an assert) and, with `bNotifyList` on a Subdevice notifier,
calls `gpuNotifySubDeviceEvent(139, eventData, 8, ...)`. **Byte comparison with the VFIO reference** (boundary-vfio-10 `gsp.jsonl`,
fn 4099 elements of notifyIndex 0x8b): `hClient 0xc1d00002, hEvent 0xff0620a0, notify 0x8b, data 0, info16 0, status 0,
eventDataSize 8, bNotifyList 1, eventData = a guest kernel pointer` — the same fields and order kayfabe encodes; per async
disable the reference posts reply then event, as kayfabe does.
**Exactly once / in order / not coalesced away:** one post per completed preempt, FIFO; in run 269 the 22 were posted in two
drainer passes (12 at maplog 59539.776, 10 at .793), one GSP vector latch per pass; ogkm drains all elements per interrupt, so 22
elements behind one interrupt are not a loss by themselves. The one known loss window (completion audit finding 10: a guest
IRQSCLR applied after a newer post clears its SWGEN0) leaves elements unread only until the guest's NEXT RPC drains them
— in run 269 five RPCs followed 17-50 ms later (perf controls of another guest thread), **so by ogkm's code the 22 events were
consumed by maplog ~59539.83 [inferred]**. That is what run H-S measures directly (`GSPQ-*` lines below, plus the KEVENTs).
**Healthy vs failing cycle (kayfabe's log, run 269).** Healthy (mem t 65.364, client 0xc1d0004f, 18 twins): per twin
`DISABLE(true)` → reply → event, then the `DISABLE(false)` list for the same 18 immediately, no other RPC between. Failing
(59539.776, client 0xc1d00002, 22 twins): the same per-twin sequence (held reply posted, then the event), no RPC in flight at the
same time (every `HELD-REPLY depth=1`), then only five periodic performance controls (answered NOT_SUPPORTED as always), then
nothing. kayfabe's side shows **no protocol difference** between the two cycles. The guest-visible GP_GET lag of 19 of 22 relays
in 269 is not required for shape S: run 267 (GET refresh on) has an S reset with guest GET == PUT on every disabled twin.

### Diagnostic added for H-S (probe thread, `KF3_COMPLETION_PROBE` only; no behaviour change)
`GSPQ-UNREAD` / `GSPQ-CONSUMED` / `GSPQ` (every 2 s): our status-queue writePtr vs the guest's readPtr and the unread count,
sampled every 100 ms with `try_lock` (the probe never waits on the GSP lock); an episode of unread elements with an unmoved
readPtr for >= 200 ms is logged once with the guest-visible and FSM IRQSTAT, the interrupt tree and the IRQ ring. Each
`RUNLIST_PREEMPT_COMPLETE posted` line now carries `utc_ms` and the queue's w/r/unread at the post. `kf_gsp::GspFsm::stat_queue_diag`
is read-only. `cargo test -p kf-gsp -p kf-qemu`: 192 passed, 0 failed.

### Run 276 — H-S (written before the run; binary f7303e72 = this branch, production flags + `KF3_COMPLETION_PROBE=1500` + `KF3_DISPLAY_WRITE_TRACE=1`, guest ETW, `PREEMPTDUMP=1` = `tooling/pwatch.sh` in the host runner)
**H-S:** in a shape-S reset the guest never consumes (or never acts on) the `RUNLIST_PREEMPT_COMPLETE` events kayfabe posted for
the open disable list, so the KEVENTs the guest driver waits on stay unsignalled. Measured by: (a) `GSPQ-UNREAD` (readPtr behind
writePtr >= 200 ms) around the open list; (b) the guest-memory dump taken 500 ms after the last unanswered disable: `_KEVENT`
SignalState and waiters at every open `eventData`. **Falsifier:** the status queue is drained (`unread=0`, no `GSPQ-UNREAD`) AND every
open KEVENT is signalled or has no waiter — then the events reached their KEVENTs and the suspend waits on something else (named
from the waiting threads in the same dump). Branches: unread + interrupt pending/undelivered → family A (GSP vector delivery);
unread + IRQSTAT 0 while unread → the IRQSCLR-after-post race (audit finding 10); consumed but KEVENT unsignalled → the RM-side
notification path (registration / `CliGetEventInfo`).

### Run 276 result (H-S; binary f7303e72 = the fixed batched-map code 2540b547 + the GSPQ diagnostic)
* **INVALID as a TDR-mechanism run: 8 host Xid 31 FAULT_PTE** (`evidence/run276-held-by-host-and-xid.txt`). Every faulting VA is a
  guest leaf kayfabe logged as `HELD BY HOST (host RM placed its own buffer there)` in the same boot: `0x4036000` (CE3 PBDMA reads,
  5 Xids — a Passthrough CE twin's own GPFIFO VA, `gpfifo=0x4036000` in its birth line), `0x15bb2000` (CE0), `0x1496c000` (GR0 PBDMA).
  `HELD BY HOST` lines: 0 in runs 263-271 (older batched-map code), 21 in 273, 4 in 275 (0 Xid there by luck), 7 in 276.
  **[measured] The fixed batched-map code still leaves guest-declared leaves unmapped in a twin's mirror space where host RM holds
  the VA, and the twin faults on them.** This is a kf-mem defect of the batched-map decisions line (reported to that owner);
  until it is fixed, TDR runs must use the eff1b692-era code (0 Xid in 12 runs).
* **H-S part (a), measured: every element kayfabe posted was consumed.** `GSPQ` 100 ms samples for the whole boot: readPtr == writePtr
  at every 2 s summary, `longest_unread_ms=0`, no `GSPQ-UNREAD`; at each of the 21 posts of the preempt-all burst (12:00:04.342 UTC)
  the guest's readPtr trailed by exactly the reply+event just written. **The guest drains the GSP status queue, including every
  `RUNLIST_PREEMPT_COMPLETE`: family A (GSP vector delivery) and the IRQSCLR-after-post race are ruled out for this burst.**
* H-S part (b): the `eventData` values are not KEVENTs (`_KEVENT` header invalid at every one of the 21 addresses in the dump); they
  name driver-private objects, so the "is the KEVENT signalled" reading is not possible from public types. Not pursued further.
* The watcher fired too early: in this boot the guest's own enable list (17 of 21) came 1.1 s after the disable list (maplog
  66082.28 → 66083.40), 70 ms after a host Xid on one of the disabled set's VAs. The quiet threshold is raised to 1500 ms for later runs.
* First TDR (F, 10 s after sign-in): a CE3 Xid on `0x4036000` at 65887.07 inside its 2 s window (last flip 65886.117) — a mirror
  fault, so not a clean F sample either.

### Run 277 (written before the run): clean code base, both shapes
Binary e6e6a9ed = branch `claude/tdr-opus-base-20261010`: caef62dc (the eff1b692-era code of runs 263-271, 0 Xid in 12 runs) + the
IRQ-RING, FLIP-LEDGER and GSPQ diagnostics cherry-picked (no kf-mem change). Flags: production + `KF3_COMPLETION_PROBE=1500` +
`KF3_DISPLAY_WRITE_TRACE=1`; guest ETW (full CSV recovered offline from the disk image); `PREEMPTDUMP=1` with a 1500 ms quiet
threshold (`tooling/pwatch.sh`; host runner tdr-run9).
* **H-F1 (shape F):** the flip whose queue entry times out was handed to the driver (ETW 259/386 with its present id ~2.0 s
  before the declaration) but **never reached kayfabe's window channel** (no window `PUT` after it). Falsifier: a window `PUT`
  (and `LATCH`) follows that hand-off within a frame — then the flip was programmed and its completion report is what is missing.
* **H-S** (as for 276): unconsumed status-queue elements at a preempt-all with no enable. Falsifier: `GSPQ` drained.
* Validity gate: any host Xid in the run makes it a mirror-fault run, not a TDR-mechanism run.

### Run 277 result (clean base e6e6a9ed; 0 new host Xid)
3 TDR cycles (mem t 48.4 / 58.2 / 92.1 s). The preempt-open watcher misfired on a healthy disable-16/enable-16 cycle (its burst
counter was reset and refilled within one 100 ms chunk: fixed, `tooling/pwatch.sh` now reads lines in order) and the hold rule
then ended the hold at READY; the guest ETW stop failed (QGA lost during the dump). **No usable sample for H-S or H-F1.**
`GSPQ`: drained throughout again (no `GSPQ-UNREAD`).

### H-P (owner: "a suspended scheduler should not stop paging, and an invalidate just completes — a kayfabe bug?") — offline part
1. **Paging during healthy suspends** (`evidence/scheduler-suspend-windows-paging.txt`, every `FLUSHSCHEDULER_SUSPEND→RESUME` in the
   guest ETW of runs 232/237/238/269/271): healthy suspends last 0.07-68 ms; paging ops START inside them occasionally (238: one op in a
   68.6 ms suspend; 232/237/271: one op in a 0.2-0.3 ms suspend), paging DMA starts inside a suspend only in 237 (2) and 269's failing one
   (1, op 230 finishing). The failing 269 suspend lasted 4349 ms with 0 op starts. Long suspends without paging also appear in
   232 (2760 ms, its 30 s-TdrDelay run) and 237 (32529 ms, the run-232-class hang). No VFIO-reference DxgKrnl ETW exists to compare.
   **[measured] paging ops do start during a suspend, but rarely; [inferred] the VidMm worker can be gated by a suspend — not decided.**
2. **MMU invalidates (VA-thread side, the only per-invalidate quantity the old logs carry):** `evidence/invalidate-va-side-stats-runs263-277.txt`.
   Every run's status lines give per 2 s window the count and the mean arrive→clear, and the boot-cumulative maximum. Means are
   0.2-0.7 ms in every window of every run (no window mean > 12 ms). **The maximum is above 12 ms in every run** (44-90 ms; 270 ms in
   276): the first ~45 ms one appears in the boot window (t≈10 s) of every run, larger ones near TDR recoveries (263: 58.7/70.9 ms right
   after reset 1; 264: 51.7 ms after reset 1; 269: 51.9 ms in the window after reset 1). **Before the first TDR of 269 and 271 the
   maximum was 46.6 ms and 44.2 ms**: no invalidate was held anywhere near the 2 s the TDR needs, all were cleared at every 2 s
   sample (inval == cleared), none pending at the declaration; the run-223 class (an invalidate held for seconds while the guest polls
   under its locks) is **ruled out for 269 and 271**. Not available from the old logs: the per-invalidate distribution (p50/p99), each one
   over 12 ms with its time, and the GUEST-visible latency (the VA side stamps "arrive" when the VA thread takes the request; the time
   from the guest's trapped trigger write to that point is not measured, so a VA thread busy in a large apply is invisible here).
   **These latencies violate ruling §AD (12 ms) irrespective of the TDR**: reported to the batched-map / VA-manager owner.
3. New diagnostic for that (base branch 1b921070, probe flag only): `INVAL-SLOW` = every invalidate whose GUEST-visible latency (vCPU
   trap that armed the trigger → the VA thread's publication of the idle word in the BAR0 read shadow, which the guest spin-reads)
   exceeds 12 ms, with the VA thread's state (walk in flight, pending wants, settled, ms since its previous publish); `INVAL-PENDING`
   for one still armed past 12 ms; `INVAL-LAT` distribution (n, p50/p90/p99, max) every 2 s.

### Run 278 (written before the run): binary 1b921070, same flags as 277, runner tdr-run10 (fixed watcher, full hold)
* **H-P (guest-visible):** some invalidate is held > 12 ms near a suspend/TDR window. Falsifier: no `INVAL-SLOW` within 2 s before any
  declaration (the distribution is reported either way).
* **H-S** and **H-F1** as for 277.

### Run 278 result (binary 1b921070, clean base; 0 new host Xid) and the shape-F finding across runs 268-278
**Guest-visible MMU invalidate latency (H-P, ruling §AD), measured** (`INVAL-LAT`, whole boot): n=16391, avg 0.30 ms, p50 0.16 ms,
p90 0.26 ms, p99 1.3 ms, **max 74.8 ms, 46 invalidates over 12 ms** (each named by `INVAL-SLOW` with its time; the VA thread was
either "away" 12-60 ms since its previous publish with no walk and nothing pending — busy outside the walk path — or not away while a
walk/settle held the clear). They cluster at guest activity bursts (process creation, `DEFERRED-API` context init) and around each
TDR: e.g. 8 between 66944.12 and 66944.87, right after the flip that timed out at TDR 1 (66944.09). **[measured] invalidates do
exceed the 12 ms ruling by up to ~6x**; **[measured] none stays armed for anything near the 2 s a TDR needs** (max 74.8 ms; the run-223
class is ruled out again). Whether a 26-68 ms invalidate burst is what delays the flip's completion is answered below: it is not the
mechanism the flips show.

**Shape F, measured on every first TDR with a display trace (runs 268, 269, 276, 277, 278; `tooling/vsrace.py`,
`evidence/vsync-ack-vs-latch-runs268-278.txt`).** For each vblank, kayfabe raises the head's VSync interrupt, then — about 0.9 ms
later, in the same display-thread pass that applies the vblank's effects — LATCHes the pending window update and writes its
notifier/semaphore completion. The guest's VSync ack (`W1C 0x611800`, as applied by the display thread) lands, for **every one of
~1680 flips that completed**, 0.50-0.57 ms BEFORE that latch (p10..p90; at most 4 per run within +0.15 ms). **For the flip whose
queue entry timed out, in all five runs, the ack is applied at the latch** (-0.016, +0.115, -0.007, +34.1, -0.008 ms): the guest's
VSync handling and kayfabe's latch of that flip ran together, and from then on every VSync DPC (ETW 273) reports the PREVIOUS
present as current until the TDR. Run 277/278 ETW with the flips aligned to kayfabe's window PUTs (5-flip interval patterns match
to 0.1 ms): the driver reports each completed flip at the VSync DPC that runs ~0.5 ms BEFORE kayfabe latches it (so the driver does
not wait for kayfabe's notifier), and the one DPC that ran at/after the latch reported the flip not done, for good.
**[inferred]** the guest driver decides "flip displayed" at the first VSync after programming from state that kayfabe changes during
its latch pass (GET past the UPDATE, ARMED words, LOADV, the semaphore/notifier words), and a decision taken mid-pass is wrong and
never revisited. Hardware latches at the vblank BEFORE raising it, so a driver never observes a half-applied latch there.

### Run 279 (written before the run): which value the guest's VSync handling reads differently (H-F3)
Binary 1b921070; flags production + probe + display write trace + **`KF3_BAR0_READ_TRACE=1`** (display range reads, display-range
writes only; the VFIO reference's own trace format, so the read set compares 1:1 with `vfio-dvi-20261008/boot3/trace.log`);
`WIN_TRACE=1`; ETW. PERTURBING (each traced read is a VM exit). **H-F3:** at the vblank of the stuck flip the guest reads at least one
display register with a value different from what it reads at every completing vblank (a register kayfabe changes during its latch
pass). **Falsifier:** the same reads with the same values as at completing vblanks — then the decision is taken from guest memory
(notifier/semaphore), named next from the same trace's timing.

### Run 279 result (read trace; stopped after 2 TDRs) and the named cause of shape F
* Same signature again (`vsrace`: stuck flips at 67885.028 with the guest's VSync handling +19.3 ms after the latch, and 67894.927
  at +0.03 ms). The read trace at the first one shows the guest's VSync interrupt for that vblank was not taken before kayfabe's
  latch pass at all (no ISR read sequence between the flip's PUT at 12:30:05.944 and 12:30:05.978, one frame later): the guest's
  handling of the vblank in which the flip latched came after the latch pass — the same condition as the other stuck flips.
* Register reads in the ISR are the same at completing and at stuck vblanks (`0x611c30`, `0x611ec0`, `0x611c00`, `0x611800` = 0x6,
  then the W1C ack): **H-F3's falsifier is met — no register value differs**; the decision is taken from guest memory. (Differences
  from the VFIO reference noted for later, not the cause: kayfabe returns 0 for the reads the driver makes before each flip,
  `0x690a2c`/`0x690aec`/`0x680220` (HW 0xcf/0xa0/0xe5), and `0x611800` reads 0x6 where HW reads 0x7/0x5.)
* **CORRECTION (2026-10-10, after runs 280, 282, 283): the release mechanism in the next bullet was INFERRED, never measured, and
  is FALSIFIED as the cause.** Run 280 (flip-away release, default order) still had 3 resets. Runs 282/283 (hardware order forced,
  flip-away vs latch release) both had 5 resets in boot. The release timing is kept because it is what hardware does (NVKMS
  `nvkms-displayless.c:272-280`, `nvkms-headsurface-priv.h:236-244`, `nvkms-api.h:680-690`), not because it fixes anything. See
  "Owner's question" below for what is measured.
* **What kayfabe writes to guest memory in its latch pass that hardware would not have written yet: the flip's own RELEASE
  semaphore.** The engine (`kf_disp::engine::Engine::complete`) wrote, at flip N's latch, the release value programmed WITH flip N.
  NVDisplay writes that value when flip N is flipped away by the next latched update; open NVKMS states this for the behaviour it
  emulates: *"We write the semaphore's release value when the NVHsChannelFlipQueueEntry is removed from current (i.e., when we do the
  equivalent of 'flip away')"* (`ogkm-595.84 nvidia-modeset/include/nvkms-headsurface-priv.h:236-244`). A driver whose VSync handling
  runs after the latch pass therefore finds the new flip already "released" (= no longer scanned out) at the vblank it should become
  current, never reports that present (ETW: the previous present stays current at every later VSync), and VidSch's flip queue times out
  2 s later. When the handling runs before the pass (0.5 ms margin, ~99.7 % of flips) the driver reports the flip before the early
  release lands, which is why it works almost always.
* **Fix (branch `claude/tdr-opus-base-20261010` 20390253, also to be carried onto the integration line):** at a window latch the
  engine writes the release of the OUTGOING armed entry (the one this latch flipped away), never the incoming one; a window's first
  latch writes none. Test `a_non_tearing_flip_waits_for_vblank_and_its_acquire` pins both. `cargo test -p kf-disp -p kf-qemu`: 286
  passed, 0 failed.

### Run 280 (written before the run): the fix on the production profile, zero measurement flags
Binary 20390253 (clean base + diagnostics that are inert without their flags + the fix). Flags: production only (no `KF3_*`
measurement flag, no guest ETW). Scripted sign-in, Edge, Shorts, hold 1000 s with a "down" key every 20 s.
**Prediction:** no flip-queue (shape F) TDR. **Falsifier:** any TDR cycle; if one occurs, the qemu.log `seq`/ack-latch analysis tells
whether it is F (fix incomplete/wrong) or S (the remaining shape).

**Correction (round 3's run-269 chain, measured from the same ETW):** the run-269 TDR also begins with a stuck flip. Present 0x117
was handed to the driver at 10:10:57.947191 and never reported (every VSync DPC from 57.963 to the declaration reports 0x116
current); this is the flip `vsrace` names for run 269 (latch 59537.894, guest VSync ack +0.115 ms after it). The display child-status
poll's `FLUSHSCHEDULER_SUSPEND` at 10:10:59.854, the preempt-all without re-enable, the VidMm paging stall and the user render waits
all came ~1.9 s AFTER the flip stuck: shape S as seen in 269 is downstream of shape F, not an independent cause. [inferred] The other S
resets likely share it; run 280 tests the fix against both.

### Run 280 result: the release fix is NOT sufficient (falsifier met)
Binary 20390253, production profile, zero measurement flags: 3 resets (nvlddmkm 153 triples at 12:34:26 sign-in, 12:34:49 Edge,
12:35:54 Shorts), the same cadence as before; display counters confirm the new release timing was active (releases = notifies - 40).
So writing the release at flip-away (hardware semantics, kept: it is correct per NVKMS) does not by itself stop the stuck flips;
the late-VSync-handling signature stands, the memory word the driver decides on is not (only) the release. Stopped early.
Read trace of run 279, additional fact: after a stuck flip the driver polls the window channel's GET/PUT (`0x690004`/`0x690000`)
at every VSync (GET == PUT, idle), which it never does after a completed flip — a different branch of its VSync handling.

### Run 281 (written before the run): the guest memory the driver reads at the stuck flip
Binary eab39a4b (base + the release fix + `NOTIFY`/`RELEASE` write-trace lines with time and resolved address), flags production +
probe + display write trace, guest ETW, and `tooling/flipwatch.py` on the host: it recognises the stuck-flip signature online (a latch
whose vblank the guest handled at/after the latch, then no PUT for 400 ms; replay on runs 268-279 fires first exactly on each run's
first stuck flip) and dumps guest memory right then (before the 2 s declaration). **H-M:** at the stuck flip the notifier slot /
semaphore slot of the stuck flip in guest memory hold a state the completed flips' slots never had at their first VSync (read from the
dump at the logged addresses). **Falsifier:** the stuck flip's slots are byte-identical in form to those of completed flips (same status,
same semaphore value pattern) — then the decision is in driver-private state and the next step is the gdb breakpoint on the driver's
VSync path (diagnosis-only), as the owner suggested.

### Run 281 result (H-M): falsifier met
The dump at the stuck flip: the stuck flip's notifier slot and semaphore slot have the same form as completed flips' slots (notifier
status FINISHED + timestamp, release value in sequence). So the decision is not visible as an odd slot value; it is in what the driver
reads WHEN, or in driver-private state.

### Owner's question: "how can the guest see a completed semaphore for something that has not flipped?" — measured contract
Measured facts (labels as above; host traces are kayfabe's VFIO-format BAR0 trace, `WIN_TRACE=1` + `KF3_BAR0_READ_TRACE=1`):
* **The race (ordering), measured, runs 268-279:** in the default order kayfabe raises the head's LAST_DATA event and the display
  interrupt at the vblank tick, and delivers that vblank's latch results (ARMED, notifier, semaphore, GET) about 0.9 ms later, behind
  the console copy. For about 1680 completed flips the guest acked the VSync 0.50-0.57 ms BEFORE the latch pass. So the guest's
  handler normally saw the PRE-latch state and found the flip one vblank later. The first stuck flip of each run had its ack within
  about ±0.12 ms of the latch pass, or later (`evidence/vsync-ack-vs-latch-runs268-278.txt`). Two late stuck flips (+19.3 ms in 279,
  +34.1 ms in 277) saw the COMPLETE post-latch state and still stuck. That points at the post-latch state itself, which is a separate
  question from the ordering.
* **Why real hardware does not race:** the trace cannot show GPU memory writes, so the order of the latch's memory writes against
  the interrupt on hardware is not measured. The open sources imply it: RM services LAST_DATA as the vblank, and NVKMS's vblank
  callback reads the window notifier and expects BEGUN (`nvkms-modeset.c:1878-1918`, `nvkms-headsurface.c:2291`). That only works if
  the latch's writes are visible before the interrupt. [inferred from source] The VFIO reference (`vfio-dvi-20261008/boot3`,
  Windows on a real RTX 4070) shows the ISR reads `0x611c30`, `0x611ec0`, `0x611c00`=2, `0x611800`=7, then W1C 2. It never reads
  AWAKEN/SEM_WIN, and never reads the window GET except for ring-wrap polls.
* **Lock-step of the slots (slot history, run 283, `KF3_DIAG_SLOT_HISTORY`):** the guest resets each window notifier slot to 0
  (NOT_BEGUN) before the UPDATE that names it (`SLOT REQ ... now=0x0`). It names a new 16-byte semaphore slot per flip, and the
  value increases by one per flip (release `+0x0`=1, `+0x10`=2, ...). Kayfabe writes each once, after the request. There is no stale
  write over a re-armed slot in the window before the first TDR. **Open item:** two window completions at boot (`completed#3/#4`,
  69540.31/.33) re-wrote the same notifier (`+0xf80`) and release (`+0x0`, 1) on consecutive vblanks while the logger had seen no
  new request. This is either a logger counting gap or kayfabe completing without a request (violation "write with no request
  pending"); to be resolved from the engine's group logic.
* **The contract in the open sources (ogkm 595.84; AD104 runs the C6 path: core C77D, window C67E, `nvkms-hal.c:148`):**
  * Release: written at FLIP-AWAY (the three citations above).
  * Semaphore lock-step (DRM path): the driver sets the slot NOT_READY, the CPU sets READY once rendering is done, the display
    acquires READY, and DONE is written at flip-away (`nvkms-kapi-notifiers.c:291-326`, `nvkms-kapi.c:2691-2707`).
  * **Window flip notifier: reset to NOT_BEGUN by the driver; hardware writes BEGUN when it performs the flip**
    (`nvkms-headsurface.c:1925-1952`: `IsPreviousFlipDone` tests `== BEGUN`; `:2037` asserts it).
  * FINISHED is the CORE completion notifier's value (`nvkms-evo3.c:6224-6243`).
  * **kayfabe writes FINISHED for window notifiers too: a second difference in the post-latch state, not yet tested.**

### Run 282 (binary 6b8e9b8e: hardware order as the default via `vblankgate`, flip-away release; production profile, zero measurement flags)
**Prediction:** no TDR. **Falsifier:** any TDR cycle. **Result, falsifier met:** 5 TDR cycles (nvlddmkm 153 x15) between
12:51:04 and 12:51:26, all before sign-in. The lock screen froze; ch1 had completed only 5 window flips. No forced edges and no new
host Xid. **So with the hardware order, kayfabe's post-latch state is wrong: every flip whose VSync handler sees it gets stuck.** The
default order works about 99.7 % of the time only because the handler usually runs before the latch pass. The gate went back to
opt-in (`1712c5c9`: `KF3_DIAG_VBLANK_ORDER=latch-first|raise-delay`, `KF3_DIAG_RELEASE_AT_LATCH`, `KF3_DIAG_SLOT_HISTORY`, all
diagnostic and off by default; raise-delay is a delay and diagnostic only).
Runner note (owner's question about run 280): run 280 ended after 20 s of hold with `stopfile=1` because I touched
`/tmp/kf-stop-winprod` at READY. Its falsifier (3 TDRs) was already met. The runner's cleanup removes the file. No stale stop file
was left; run 282 started with none.

### Run 283 (binary 1712c5c9: latch-first + the OLD release at latch + slot history + BAR0 read/write traces)
**Owner's prediction:** with the old release, shape F reproduces in seconds. **Result:** 5 TDR cycles in boot, the same as 282. So
the release timing does not decide it. The read trace at the stuck first flip (PUT 12:57:44.0876, latch and VSync 44.0945, MSI
44.0947) shows the ISR reads the window GET/PUT at once (`0x690004`=`0x690000`=0xa80, idle). It does so at every later VSync and
never disables LAST_DATA. A healthy default-order flip (run 279, PUT 12:29:37.2783): the first VSync does NOT read GET; the second
reads GET==PUT, then the driver disables LAST_DATA (`0x611d80`<-0). **So the guest's state machine needs a word that it gets when its
first VSync sees the PRE-latch state and then the post-latch state, and that it never gets when the first VSync already sees
kayfabe's post-latch state.** The candidate is the window notifier status (NOT_BEGUN, then BEGUN on hardware; kayfabe goes straight
to FINISHED).

### Run 284 (written before the run; binary dbbcb387: latch-first + window notifiers written BEGUN, `KF3_DIAG_WINDOW_NOTIFIER_BEGUN`)
**H-N:** with hardware's status value (BEGUN) for window flip notifiers, the hardware order completes every flip. **Falsifier:** TDR
cycles in boot as in 282/283. Flags: latch-first + BEGUN + slot history + BAR0 traces, hold 240 s.

### Run 284 result (H-N): falsifier met
5 TDR cycles in boot and sign-in, the same as 282/283, so BEGUN alone does not complete the flip under the hardware order. The status
word does reach the driver's decision, measured: with FINISHED the first post-latch VSync reads the window GET/PUT once, and does
so once per later VSync. With BEGUN the same VSync is followed by a ~1.5 ms busy poll of GET/PUT (about 20 reads, GET==PUT==0xa80),
repeated at every later VSync. The driver reads the notifier at VSync and branches on it. BEGUN (hardware's value) puts it in a
"flip begun, wait for it to finish" branch that kayfabe never ends. Slot history 284: the guest re-arms each new notifier slot to 0
before its UPDATE. The open item from 283 is a LOGGER gap: UPDATEs decoded in the run-on after a latch (inside `vblank`) were not
sampled. It is not an engine write without a request; the guest itself re-used `+0xf80` / release `+0x0`=1 for two boot updates.

### Where this stands (runs 276-284; time-box: 9 of 10 runs used)
* Measured: (1) the ordering race in the default order. (2) kayfabe's post-latch state is wrong: under the hardware order every
  first flip sticks, whichever release timing and whichever notifier status (FINISHED or BEGUN) is written. (3) The driver reads the
  notifier status at VSync and branches on it.
* Not yet known: which further guest-visible word the driver needs after BEGUN. Candidates are the notifier timestamp words
  (kayfabe writes the GPU time at the latch pass; hardware time-stamps the flip), PRESENT_COUNT (bits 7:0, always 0 in kayfabe), and
  a later FINISHED.
* Next measurement (the owner's item 2): hardware read-watchpoints on the stuck flip's notifier words through the QEMU gdbstub.
  Find the guest VA of the notifier page by walking the kernel page tables (`info tlb` filtered on the host for its PA, e.g.
  0x239975f40 in run 284). Set `rwatch` on status and timestamp. Log value and RIP per hit, symbolised with MS public symbols where
  they apply (diagnosis only; outputs stay on the host). Use latch-first + BEGUN, so the stuck path runs deterministically in the
  first seconds of boot.
* Product: no change is proposed yet. The default stays the old order with the flip-away release, which is hardware semantics per
  NVKMS. Latch-first, raise-delay, release-at-latch and window-BEGUN are diagnostics, off by default. The "publish, then raise owed
  interrupts" structure the owner specified is the intended product shape, once the post-latch state is right.

### Coordinator question (a): the run-276 mapping hole — exact revision and Xid details
* Binary **f7303e72**: this branch with `2540b547` merged, the FIRST fixed batched-map version. It contains none of the eight later
  kf-mem review commits that integration has (`604a1826` .. `80b30741`: chunked steer claims, steer restores rows, steer hand-over
  of the ctx range, StillReserved refusals, ...). **Whether the hole remains on the reviewed code (integration 6c51302b+) is NOT
  measured by any run of mine.** Runs 277-284 used the eff1b692-era base (0 new Xid).
* The Xids (host dmesg, `evidence/run276-held-by-host-and-xid.txt`), all `FAULT_PTE`:
  * VA `0x4036000`: 5 of 8, engine `CE3_PBDMA0` / `HUBCLIENT_ESC`, channels `0x02000040`, `0x0200003d`, `0x0200004b`,
    `0x02000041`, `0x0200003b`.
  * VA `0x15bb2000`: `CE0` / `HUBCLIENT_CE1`, channel `0x36`.
  * VA `0x1496c000`: `GR0_PBDMA0`, channel `0x42`.
  * VA `0x1974b000`: `GRAPHICS GPC1 PE_3`, channel `0x3b`.
* The kayfabe events before them, in qemu.log order:
  * Several Passthrough CE twins are BORN with `gpfifo=0x4036000x8192` (lines 2977, 3629, 6868, 6911; tokens 0x1010, 0x1016,
    0x1019, 0x101a).
  * Then `kf-mem: HELD-BY-HOST guest row 0x4036000+0x10000 (ram=true) — host RM already maps that VA` (line 6966), and the same for
    `0x4035000+0x1000`, `0x15bb2000`, `0x1496c000`, `0x9b580000`, `0x9b8a0000`, `0x15bb1000`, `0x15bb4000`.
  * `PT-SNAP tok=0x1019 GP[0x0] @0x4036000: 0x4036000 not placed by us` (lines 7794, 8111).
  * So the twin's GPFIFO VA is a guest leaf that kf-mem refused to map because host RM already holds that VA in the mirror space,
    and the twin's PBDMA then fetches it.

### Run 285 (written before the run): guest reads of the stuck flip's notifier (read-watchpoints)
Binary dbbcb387. Flags: `KF3_DIAG_VBLANK_ORDER=latch-first`, `KF3_DIAG_WINDOW_NOTIFIER_BEGUN`, `KF3_DIAG_SLOT_HISTORY`, display write
trace, BAR0 read trace (`WIN_TRACE=1`). Runner `tdr-run13.sh` with `RWATCH=1`. Host-only tooling: `tdropus/rwatch.py`,
`revmap.py`, `gdbwatch.py`.
* For each driver life, the first `SLOT REQ` names the window notifier page's guest PA.
* `revmap.py` walks the guest's kernel page tables from a CPL-0 vCPU's CR3, reading guest RAM through QEMU's shared memfd (no VM
  stop), and finds the kernel VA(s) of that page.
* `gdbwatch.py` sets x86 access watchpoints through QEMU's gdbstub on slot `+0xf40` (the stuck flip: status+word1, then the
  timestamp) and `+0xf80` (the previous one). It logs time, vCPU, pc, rsp, stack and the 16 bytes at every guest access.
**H-W:** at the VSyncs after the stuck flip latches, the driver reads that slot (status and/or timestamp). The value and pc tell
which word decides. **Falsifier:** no guest access to the slot between its latch and the TDR (other than the guest's own reset
before the UPDATE). Then the decision does not read the notifier through these mappings, and the semaphore slot / other words are
next.
