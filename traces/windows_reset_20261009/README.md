# Windows under kf3: the display reset ~19.6 s after driver start (runs 98/99) — cause and fix

**STATUS: LIVE, 2026-10-09.** Branch `claude/windows-reset-20261009` from `claude/display-reply-diff-20261008`
`1d505f2c` (+ the merged BAR0 read-trace mode, `claude/kf3-read-trace-20261008` `e60b2d46`). Question: with the real
GPU's caps page (`KF3_DISPLAY_CAPS_PROBE`, run 99) Windows programs window 0 after the modeset, and ~19.6 s after driver
start the guest resets the display and stops answering. What differs from the real RTX 4070 at and before the reset?

Labels: `[measured, <source>]` read from a capture or a disk (the runs and captures of 2026-10-08/09, RTX 4070); `[code]` read from kf3's code; `[source]` read from a pinned
reference source; `[inferred]` reasoning, never evidence. kf3 times are the `kf_mem::maplog` clock of the run's log
(`WTRACE t=`); hardware times are seconds of day of VFIO DVI reference boot3 (`traces/vfio_dvi_reference_20261008/`, branch
`claude/vfio-dvi-reference-20261008`).

## 0. Answer (GPU-free, from runs 98/99 and their disks)

1. **What Windows was doing at the reset: a TDR and then bugcheck `0x116` VIDEO_TDR_FAILURE.** `[measured, run 99 and
   run 98 disks, read offline with scripts/bench/windows/recover_bugcheck.py, 2026-10-09]` `pagefile.sys` holds a kernel
   crash dump header `PAGEDU64`, build 26100: run 99 `0x116 (0xffffe4027264b300, 0xfffff80049884930, 0xffffffffc000009a,
   0x4)`, run 98 `0x116 (0xffffd58ce6747300, 0xfffff8060e304930, 0xffffffffc000009a, 0x4)` ([run99](run99-bugcheck.json),
   [run98](run98-bugcheck.json)). Same code, same P2 low bits (`…4930`, the same nvlddmkm location at another load base),
   same P3 `STATUS_INSUFFICIENT_RESOURCES` and P4 — the signature of the earlier 0x116s of runs 57 and 76
   (`traces/windows_code43_walls_20261007/run57-session-probe-boot2.txt`, `run76-crash.txt`: same P2 low bits, same P3/P4),
   so the parameters name nvlddmkm's TDR-recovery failure, not its cause. `System.evtx`
   ends at 22:37:47 UTC, before the first flip (the harness saw the window programming by 22:37:51): the log was never
   flushed after it, so it holds no 4101/LiveKernelEvent record.
   Sequence in the run 99 log `[measured]`: the guest's display teardown at 271547.430 (`RM_INTR_EN_HEAD_TIMING(0) ← 0`,
   417 FREEs, every window/core/cursor channel freed), the RM's display re-init at 271548.131 (all eight windows and the
   core re-initialised), new channels pushed every 2 s with nothing consumed, then RPC `SWITCH_TO_VGA` (fn 49, refused
   `0x56` by kf3) as the guest's last kayfabe-visible act — `[inferred]` the bugcheck's display hand-over — and nothing
   after it (the status line's `trapped=` count frozen until the kill). The screendumps' "firmware logo + spinner" is the
   stale boot framebuffer, not a reboot (no second boot in the log).
2. **Why: kf3 halted its own display engine 0.09 s after the first flip.** `[measured, run 99 log line 19681; run 98
   line 19690]` `kf3: display: scanout REFUSED SDR colour program: LUT requires matching unmirrored DIRECT8 or DIRECT10
   extent`. `[code, crates/kf-qemu/src/display.rs scan.failed → Engine::halt_scanout]` a refused colour program stops every
   display channel without publishing GETs, notifiers or semaphore releases. `[measured]` the method counter stops at
   `14 updates completed, 5736 methods` (271529.95) and never moves again (status line `disp[… methods=5736 updates=14]` at
   +15 s, +40 s and at the stop); the last `TRACE METHOD` line is 19677. Every later PUT — window 0 at 271532.22 and
   271535.43, WindowImm 0, core at 271537.42 / 539.43 / 541.42 / 545.43 — is never consumed.
3. **The program kf3 refused** `[measured, run 99 TRACE METHOD lines ≤ 19680]`: window 0 (C67E, channel 1)
   `SET_ILUT_CONTROL = 0x0004050a` (DIRECT10, SIZE 1029, **MIRROR=1**, no interpolation), `SET_CONTEXT_DMA_ILUT
   0xff1fe313`, `SET_OFFSET_ILUT = 0x21`; FMT identity; `SET_CSC11CONTROL = 1` with identity; `SET_TMO_CONTROL = 1` without a
   TMO context DMA (bypass). Head 0 (C77D): `HEAD_SET_OLUT_CONTROL = 0x0004050a` (MIRROR=1 too), `OLUT_FP_NORM_SCALE
   0xffffffff`, OLUT `0xff1fe144 + 0`, `OCSC0` enabled = identity + 0x20 rounding bias, `OCSC1` off. With the authored caps
   page (runs 88-97) the guest never programmed a window after the modeset, so this decoder path was never reached by
   Windows before run 98. NVKMS and nouveau never set MIRROR (`nvkms-evo3.c:4340`, `:5005`; nouveau `headc57d.c:122`).
4. **The second wall behind it** `[code + source]`: kf3 reads `SET_OFFSET_ILUT` in bytes (`color.rs`, shift 0; its test
   said "ILUT offset is bytes"), so the guest's `0x21` is a misaligned table (`LUT entry address is misaligned`). The
   unit is 256 bytes: nouveau writes `SET_OFFSET_ILUT = offset >> 8` (`[source]` linux `6f3ed7fec`,
   `drivers/gpu/drm/nouveau/dispnv50/wndwc57e.c:140`), as kf3 already does for the OLUT and TMO offsets; NVKMS always
   passes 0 for the ILUT (`nvkms-evo3.c:4224`, `offsetof(NVEvoLutDataRec, base)`), so it could not show the unit.
   `0x21 << 8 = 0x2100` is exactly NVKMS's own `NVEvoLutDataRec.output` offset (`nvkms-types.h:880-885`).

## 1. The first kayfabe-visible difference after window 0 starts flipping (align.py `--post`)

[tools/align.py](../display_reply_diff_20261008/tools/align.py) `--post` aligns, from the first window-0 PUT after the
modeset, 20 s of hardware boot3 (x-gsp-observer RPCs + `vfio_region_*` display accesses + `vfio_msi_interrupt`) with the
kf3 log up to the guest's display teardown (RPCs + `WTRACE` writes; per-frame `EVT_STAT` acks excluded; the PUT values
differ by the driver-start push base, §0 P17 of the reply-diff record, and are not counted as differences). Outputs:
[post-aligned-boot3-vs-run99.txt.gz](post-aligned-boot3-vs-run99.txt.gz), [post-counts-boot3-vs-run99.txt](post-counts-boot3-vs-run99.txt)
(per kind and per second).

`[measured, boot3 vs run 99, RTX 4070]`:
- **+0.000-+0.060 s: identical** — window 0 flips, core PUTs, the two SPD-infoframe blocks, WindowImm 0 PUT `0x10`,
  UNBLANK (`0x680240 ← 2`).
- +0.09 s: kf-only `RM_INTR_EN_HEAD_TIMING(0) ← 0x2` then `← 0` (the known LAST_DATA disable, reply-diff §0 P43; also in run
  97 where no window was programmed, so not a consequence of the halt); **kf3-internal at the same instant: the colour
  refusal and the halt** (no hardware counterpart, not visible to the guest except through GETs that stop).
- +0.37-+2 s: the known non-display refusals (`0x20809004` unserviced, `0x2080b201`/`852f`/`853a`/`a618` 0x56) — present
  in runs 88-97 as well (reply-diff §0 P2).
- **+2.19 s onward — the flip stream:** hardware flips window 0 at 60 Hz (`0x690000` 207-243 writes/s and WindowImm 0
  51-60/s from +3 s to +7 s; 1047 + 258 in 20 s, with 258 `0x690a2c`/`0x690aec` read-backs); kf3's guest writes 16
  window-0 PUTs in total and none after +5.57 s — `[inferred]` its flips wait for completions that the halted engine
  never gives.
- **+13.58 s: `NVC370_CTRL_CMD_GET_CHANNEL_INFO` ×599 in one second, 690 in all** (kf3 answers BUSY while GET ≠ PUT —
  `[code]` `kf-disp/src/model.rs` `channel_info`); **hardware: 0 in the window (2 in the whole boot, 12.50 s and 108.96
  s)**. +17.57 s the teardown.
- Not visible on kf3 (stated, not inferred away): MSIs (hardware 2718 on vector 0 in the window), display reads (kf3's
  shadow reads never exit in this run), GSP events (`0x1003` ×29 on hardware), the hardware's `0x0073028b`×35 /
  `0x0073010c`×24 cluster at +2.35 s (absent on kf3: `[inferred]` part of the flip/present path the halted guest never took).

**Cheap check of the task's alternative (a kf3 timer/event at +19.6 s):** none. The teardown follows the guest's own
2-s cadence of core updates (271537.42, 539.43, 541.42, [543.43: the GET_CHANNEL_INFO poll instead of an update],
545.43, 547.43 teardown); kf3's display model has no event of its own there — the only kf3 event is the refusal at
271529.95.

## 2. Fix: two default-off experiments (GPU-free tests)

| flag | crate | change | exact because |
|---|---|---|---|
| `KF3_DISPLAY_LUT_MIRROR=1` | kf-disp `color.rs`, kf-qemu `display.rs` | `*_LUT_CONTROL_MIRROR` accepted on a DIRECT8/DIRECT10 ILUT/OLUT | `[inferred]` MIRROR reflects the table about zero (changes negative inputs only). The kernels have no negative side, so it is accepted only where no negative value can reach the table: the ILUT always (its index is a UNORM8 component, `kf_color_compose`), the OLUT only when every armed window matrix and the head's OCSC0 are nonnegative (`color::mirror_inert`; the ILUT is validated to [0,1], CSC LUTs and TMO are nonnegative); otherwise still refused |
| `KF3_DISPLAY_ILUT_OFFSET_256=1` | kf-disp `color.rs` | `SET_OFFSET_ILUT` read in 256-byte units | `[source]` nouveau `wndwc57e.c:140`; same unit as OLUT/TMO |

Test: `color::tests::the_windows_mirrored_program_decodes_only_under_the_experiments` feeds run 99's measured program (RTX 4070, 2026-10-09)
(both driver tags): off → the run's exact refusal; mirror alone → decoded but the 0x21 byte offset is misaligned; both →
ILUT `0xff1fe313 + 0x2100`, 1025 entries, OLUT `0xff1fe144 + 0`, TMO bypass, the pipeline nonnegative; a negative coefficient
before a mirrored OLUT is not inert. Startup confirmation lines: `EXPERIMENT KF3_DISPLAY_LUT_MIRROR=1 …`,
`EXPERIMENT KF3_DISPLAY_ILUT_OFFSET_256=1 …`; at first use `… a mirrored LUT accepted (…)`.

Harness: `scripts/bench/windows/windows_broker.sh` gained `WIN_TRACE=1` / `WIN_GSP_OBSERVER=1` (default off; as
`win_vm.sh`'s `WINVM_TRACE`): QEMU trace events from `scripts/bench/trace-events-vfio-reference.txt` into `$RUN/trace.log`,
the shared GSP observer into `$RUN/gsp.jsonl`, so a kf3 Windows boot produces the VFIO reference's own line formats.

## 3. Boot 1 (run 100): H-mirror — stated before the boot

Binary: this branch's tip through `build_kf3.sh` on the read-trace QEMU tree. Flags: run 99's set
(`… KF3_DISPLAY_CAPS_PROBE`) + `KF3_DISPLAY_LUT_MIRROR=1 KF3_DISPLAY_ILUT_OFFSET_256=1` + the read trace
(`KF3_BAR0_READ_TRACE=1 KF3_READ_TRACE_RANGES=0x110000-0x110fff,0xb81000-0xb81fff`, display range always,
`WIN_TRACE=1 WIN_GSP_OBSERVER=1`).

**H-mirror** (stated 2026-10-09, before run 100 on the RTX 4070)**:** the 0x116 is caused by kf3's halt after refusing Windows' mirrored colour program. **Prediction:** both
EXPERIMENT lines; no `scanout REFUSED`; `updates completed` passes 14 and the status line's `methods=` keeps rising;
window 0 keeps flipping (window PUTs at the hardware's rate, hundreds per second while the desktop draws); no
`GET_CHANNEL_INFO` burst; no display teardown at ~+19.6 s; no `0x116` header in `pagefile.sys` afterwards; the QGA probes
connect. **Falsifier:** with both flags confirmed, no refusal and the engine consuming methods throughout, the guest still
tears the display down / bugchecks 0x116 → the halt was not the cause (then the read trace and the GSP observer of this
boot are the comparison with boot3). **Other outcome:** a different `scanout REFUSED` line → the next wall on the same
path (not a falsification; it names the next field to support).

## 4. Run 100 (binary `kf3-bins/f5c93b21`, boot 1, RTX 4070, 2026-10-09): the halt is gone; the 0x116 is not — two new walls, measured

`[measured, run 100 at f5c93b21, RTX 4070, 2026-10-09 23:09:56-23:13:28 UTC]` files: [harness log](run100-harness.log),
[marker](run100-marker.txt), [command](run100-command.json), [qemu log](run100-qemu.log.gz), [BAR0 read trace](run100-trace.log.gz)
(VFIO reference format; 138242 records, flushed 23:10:05-23:10:16), [GSP observer](run100-gsp.jsonl.gz),
[bugcheck recovery](run100-bugcheck.json) and [dump headers](run100-dump-headers.txt), [System event log tail](run100-system-evtx-tail.txt), probe outputs `run100-flip-*.out`.
kf3 clock → UTC: `UTC = kf − 273462.364 s` (window-0 PUT `0x740` at kf 273473.554091 = trace 23:10:11.189627; the teardown's
`RM_INTR_EN ← 0` at kf 273476.995547 = 23:10:14.631097, residual 0.1 ms).

**H-mirror's prediction, item by item:** both EXPERIMENT lines — yes; `a mirrored LUT accepted (ILUT mirrored=true, OLUT … mirror:
true)` — yes; **no `scanout REFUSED`** — yes; the engine consumes: `updates` 66 at +17 s (run 99: 14), methods 6102 → 11477, **28 window
LATCHes at 60 Hz with WindowImm PUTs** (run 99: none after the halt) — yes; `GET_CHANNEL_INFO` ×2 in the boot (= hardware's 2; run 99:
690) — yes. But: **display teardown 7.2 s after the first frame and bugcheck 0x116 again** — `[measured, run 100 disk, 2026-10-09]` `Minidump/100826-4875-01.dmp`
`0x116 (0xffffa50b72918010, 0xfffff8016b1d4930, 0xffffffffc000009a, 0x4)` (System event 1001 at the next boot: "rebooted from a
bugcheck"). Windows rebooted itself (`-action reboot=reset`); the second Windows boot in the same QEMU process has the adapter at
**Code 43** (known: an in-process reboot never restarts kf3's GSP model), so the D3D probes found only the Basic Render Driver and
`nvidia-smi` failed. **Verdict:** H-mirror is confirmed for what it said about the halt (the halt caused run 99's stalled flips and
its GET_CHANNEL_INFO poll) and **falsified as the cause of the 0x116**: with the halt gone the guest still TDRs. Two further walls:

1. **The TDR trigger (open): all guest GPU work stops at 23:10:11.24 — 5 s of desktop after the first frame.**
   `[measured, run 100 kf3 log + BAR0 trace]` first window scanout `+10412 ms` (≈23:10:06.4); flips and LATCHes at 60 Hz up to the
   last LATCH at 23:10:11.199; the last user-work doorbell `0x20016` = token `0x1016` (the Passthrough CE channel of process
   `0xc1d00037`, born ~23:10:10.5, `CtxBind{bound:false}`) at 23:10:11.2348, the last kernel-CE doorbell (`0x1000c`) at 11.2375;
   then **no doorbell, no RPC, no window PUT** for 2.0 s while VSync interrupts keep being raised and acknowledged (`EVT_STAT`
   write-clears every frame, 515 MSIs, ISR reads of `0x611c00`/`0x611ec0`/`0xb810xx` throughout); at **23:10:13.32 the TDR** (the
   guest reads the core ARMED area `0x688000…`, `0x680240`, `0x682288` and posts GSP RPCs — the KMD's timeout state collection);
   23:10:14.60-14.91 the teardown (window disable, core free, all 9 Passthrough twins freed). Host: **no Xid** during the run
   (`dmesg` 01:09:55-01:13:28 CEST holds only the two module loads). `[inferred]` a GPU-scheduler packet whose completion never
   reached the guest, not a fault; which packet/fence is not visible to kf3 (Passthrough work runs on the host; kf3 logs the
   doorbells, not the host GP_GET).
2. **The TDR recovery then fails at a kf3 refusal → 0x1B0 → 0x116.** `[measured]` `LiveKernelReports/WATCHDOG-20261008-2310.dmp` =
   `0x1B0 VIDEO_MINIPORT_FAILED_LIVEDUMP (0x2 "start device failed", 0xc000009a, 0x108, …)`. In the recovery's restart
   (kf 273478.009 = 23:10:15.645) kf3 refuses the kernel driver's new GR channel: `chan 0xc1d0004a:0xff040001 birth REFUSED: twin
   state: VA space … (KernelInUserSpace(1)) — a Translated channel never runs in a space a user channel runs in
   (V3_P1P2_TSPACE.md §4.2)` (RmAlloc `0xc56f` → `0x40`); 10 ms earlier a Passthrough CE twin (`0xc1d00048:0xff040000`, token `0x802`)
   was born in that same VA space. The guest's last kayfabe-visible act is `SWITCH_TO_VGA` (fn 49, refused `0x56`), as in run 99.
   This is what makes the TDR fatal (a recovery that succeeds leaves a TDR, not a bugcheck).

**What kf3 itself did at the black frame `[measured, run 100, 2026-10-09]`:** nothing of its own — the black frame (`+17712 ms the console shows BLACK`)
follows the guest's own window disable in the teardown (`+17462 ms … a lit head has no window`), 3.4 s after the last flip; no kf3
display event, hotplug, EDID or mode change in between.

**Owner observation (UNCONFIRMED, 2026-10-09):** watching run 100's window, the owner briefly saw the Windows taskbar clock before
the screen went black. Consistent with the log (5 s of 60 Hz flips of window 0 before the stall) but not confirmed by a frame: run
100 took screendumps only at +40 s and at the end (both after the reboot). From boot 2 on the harness takes one timestamped
screendump per second ([tools/frames.py](tools/frames.py) classifies them and builds a contact sheet).

## 5. Boot 2 (run 101, RTX 4070, 2026-10-09), stated before the boot: reproduce with frames, and arm the stall measurement

Same binary and flags as run 100; `WR_SHOTS=90` (one screendump per second from the launch, host UTC in the name) and
`WR_TDROFF_ARM=1` (QGA, as soon as the guest answers — in its first or its post-bugcheck boot — runs `tdr_off_etw_arm.ps1`:
`TdrLevel=0` and the boot-time DxgKrnl ETW session, both for the NEXT boot of this disk).
**H-repro** (stated 2026-10-09, before run 101 on the RTX 4070): run 100's course repeats — window-0 flips for several seconds, all
doorbells stop, a TDR ~2 s later, the recovery refused at the T-space rule, 0x116. Falsifier: no stall within 60 s of the first
frame, or a different abort. **Owner observation:** confirmed if a frame with desktop content (a non-black taskbar strip) precedes
the first black frame; falsified if no frame between the boot logo and the black frame shows the desktop.
Boot 3 (run 101's disk again, `WR_REUSE=1 WR_STALL_ETW=1`): with `TdrLevel=0` the stall is held; the harness stops the ETW session
in it and decodes the tail. **H-tail falsifier:** the decoded tail does not end at an unfinished queue/DMA packet (then the stall is
not a GPU-scheduler wait).

## 6. Run 101 (binary `kf3-bins/f5c93b21`, boot 2, RTX 4070, 2026-10-09): H-repro falsified in its timing; the lock screen shown; a host MMU fault

`[measured, run 101 at f5c93b21, RTX 4070, 2026-10-09 23:22:58-23:34:52 UTC]` files `run101-*` ([harness](run101-harness.log),
[qemu log](run101-qemu.log.gz), [frames](run101-frames.tsv), [a frame](run101-frame-232320-lockscreen.png), [bugcheck](run101-bugcheck.json)).
kf3 clock → UTC: `UTC = kf − 273282.363 s` (the teardown's `RM_INTR_EN ← 0` at kf 274501.265 = trace 23:27:18.902).
- **The owner's observation is CONFIRMED in substance:** from 23:23:14 to the end of the frame series (23:24:13) every
  screendump is the **Windows lock screen drawn through kf3's display by the NVIDIA driver** (clock "11:23", "Thursday,
  October 8", the Windows wallpaper; [frame](run101-frame-232320-lockscreen.png)) — a clock, as the owner saw (the lock
  screen's, no taskbar: no user is logged on). Frames before 23:23:14 are black (the boot layer). The first window scanout
  was at +10.96 s (23:23:09.5).
- **H-repro is falsified in its timing:** no stall 5 s after the first frame — the lock screen ran for ~4 min (64
  LATCHes, flips when the clock changes). Then: 23:27:08.57 a walk (`SPLIT` ticket by the kernel copy channel `0x80c`)
  **unmaps the D3D process's VA range** in its space `VasKey(…399696)` (0x4014000-0x4024000, 0x4046000-…, 0x40cc000-…;
  34+8+34+8+34+30 rows), the space where its Passthrough twins `0x15` (GR, host 0x3c, GPFIFO 0x4000000) and `0x1016`
  (CE3, host 0x2003d, GPFIFO 0x4036000; last doorbell 274255.768 = 23:23:13.4) still live; **23:27:14 host Xid 31** (two
  lines, `dmesg`): `channel 0x0200003d … ENGINE CE3_PBDMA0 HUBCLIENT_ESC faulted @ 0x0_04036000 … FAULT_PTE
  ACCESS_TYPE_VIRT_READ` and `channel 0x0000003c … GR0_PBDMA0 … @ 0x0_04034000 … FAULT_PTE ACCESS_TYPE_VIRT_WRITE`
  — the two twins' PBDMAs touched their own ring/USERD-area VAs after the guest unmapped them; 23:27:18.87 the display
  teardown (TDR), the recovery's channel birth refused at the T-space rule again (`birth REFUSED` ×1, `SWITCH_TO_VGA`
  ×1), **0x116** `(…, 0xfffff807224d4930, 0xffffffffc000009a, 0x4)` (same signature). The guest hung (no reboot), killed
  after 300 s. `[inferred]` a process (the lock screen's?) went away while its channels' twins were still scheduled;
  why a twin's PBDMA fetched after 4 min of silence is open (a guest write to the adopted USERD's GP_PUT reaches the
  host channel without a doorbell).
- **QGA never answered** during the 4 min of desktop (60 attempts), so the TDR-off/ETW arming did not happen;
  `TdrLevel=0` cannot be set offline on the bench host (no hivex). The ETW plan is dropped for now.

So the stall trigger is not one event: run 100 a silent stop of all GPU work 5 s after the first frame (no Xid), run 101 a
host MMU fault on a D3D process's twins after the guest unmapped their VA (Xid 31). Common: the Passthrough twins of one
D3D process (GR `0x15` + CE `0x1016`, born ~0.7 s before the stall in run 100), and the same fatal recovery.

## 7. Boot 3 (run 102), stated before the boot (2026-10-09, RTX 4070): the armed-rule relay + the stall snapshot

Binary `kf3-bins/3e9bcdce` = this branch with `claude/passthrough-nsi-nogate-20261008` (`4b8399d1`, the owner-ruled
armed-rule non-stall relay: every host edge to every VM whose guest armed the event, kernel `0x7e`/`0x78` registrations
counted as armed, `FIFO_EVENT_MTHD` on its own vector, nothing dropped) merged in place of the old live-twin + doorbell gate
(conflicts resolved in favour of the new relay; the `KF3_RELAY_GET_REFRESH` refresh kept in its engine-edge raise; GPU-free
tests, clippy debt, fmt and `ci_gates.sh` green at `3e9bcdce`), plus the new default-off **stall snapshot**
(`KF3_PT_STALL_SNAPSHOT=1`: after 1 s with no doorbell to any live Passthrough twin, and at each twin's free, every twin's
USERD `GPGet`/`GPPut` from guest RAM, its ring entries, the decoded methods of the last segments and the value at every
semaphore they name; GPU-free test `snap_tests`). Same flags as run 100 otherwise (caps probe, mirror + ILUT experiments,
read trace, frames).
**Falsifier (coordinator's, stated before the boot):** with the armed-rule relay the guest still stops all GPU work ~5 s
after the first frame (last doorbell on token `0x1016`, then silence while VSync continues). The report carries the
relay's report line (`fifo_armed`, `kernel_nonstall_registered`, edges/raised/not-armed per vector) and, from the
snapshot, for `0x1016` and every twin whether `GPGet == GPPut` (the GPU finished: a completion not delivered) or not
(which method/semaphore it waits on and who should release it). VFIO boot3 cannot show the same channel's state: the
reference traces BAR0 and GSP RPCs, never guest memory.
