# Windows under kf3: the display reset ~19.6 s after driver start (runs 98/99) — cause and fix

**STATUS: LIVE, 2026-10-09.** Branch `claude/windows-reset-20261009` from `claude/display-reply-diff-20261008`
`1d505f2c` (+ the merged BAR0 read-trace mode, `claude/kf3-read-trace-20261008` `e60b2d46`). Question: with the real
GPU's caps page (`KF3_DISPLAY_CAPS_PROBE`, run 99) Windows programs window 0 after the modeset, and ~19.6 s after driver
start the guest resets the display and stops answering. What differs from the real RTX 4070 at and before the reset?

**Where it stands after the 4-boot time-box (runs 100-103, RTX 4070, 2026-10-09; details §4-§11):** the halt of §0 is fixed
behind two default-off flags and Windows then flips at 60 Hz and draws its lock screen through kf3; the guest still TDRs and
bugchecks 0x116. Measured below: in runs 101/103 the D3D process's Passthrough GR/CE twins take host Xid 31 (FAULT_PTE) on VAs
kf3 never unmapped, after page-table walks that unmapped other rows of the same 2 MiB region (H-pde, §10, next); in runs
100/102 no fault, every twin fetched and released everything (stall snapshot, §8). Falsified: the armed-rule relay as the fix
(§8), H-flipdone (§10). Every recovery fails at a classification + T-space refusal (§11, owner decision proposed).

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

## 8. Run 102 (binary `kf3-bins/3e9bcdce`, boot 3, RTX 4070, 2026-10-09): falsifier MET — the armed-rule relay does not end the stall; the GPU finished everything

`[measured, run 102 at 3e9bcdce, RTX 4070, 2026-10-09 23:45:31-23:47:32 UTC]` files `run102-*`
([harness](run102-harness.log), [qemu log](run102-qemu.log.gz), [BAR0 trace](run102-trace.log.gz), [frames](run102-frames.tsv),
[bugcheck](run102-bugcheck.json)). kf3 clock → UTC: `UTC = kf − 275562.358 s` (teardown `RM_INTR_EN ← 0` at kf 275613.530 =
23:45:51.173).
- **Relay:** the new relay is live (`non-stall relay (owner ruling 2026-10-08): every host edge -> the guest vector of every event
  this guest ARMED, never dropped …`). Report line before the teardown: `fifo_edges=5694 fifo_raised=5002 fifo_armed=1
  kernel_nonstall_registered=5 arm_clears=1 … edges=11365 not_armed=2756 v0[raised=3139] v1[raised=4842] v2[raised=628]
  GR0[vec=0 armed=1 wakes=3200 not_armed=221 raised=2985 live_twins=5] CE2[vec=1 armed=0 wakes=1214 not_armed=1214]
  CE3[vec=2 armed=1 wakes=1257 not_armed=629 raised=628 live_twins=4]`. In the stall window (after the last flip) every GR0 and
  CE3 wake was raised (`Raise(0)` ×283, `Raise(2)` ×188; CE2's 440 are `NotArmed`: no guest event on CE2). All MSIs in the read
  trace are on MSI vector 0 (`0xfeeff00c/0x4962`); the guest's ISR reads the leaf registers (`0xb81000 = 0x3`, `0xb81600`)
  and clears them throughout — the per-engine "vectors" are leaf bits, not MSI vectors.
- **Course:** first window scanout +12.7 s; the last flip (window-0 PUT `0xa80`, LATCH) at kf 275610.148 = 23:45:47.79; then
  no window PUT for 3.3 s, while the D3D process's GR channel `0x15` still rang 13 doorbells, `0x1016` 5 and `0x13` 2 (the last
  at kf 275612.155); **the TDR's state collection (reads of `0x688000…`) at 23:45:49.855 = 2.06 s after the last flip**
  (run 100: 2.12 s after its last flip) — the TDR clock starts at the last flip, not at the last GPU work. Teardown at
  275613.5, recovery refused at the T-space rule (`birth REFUSED` ×1), `SWITCH_TO_VGA`, **0x116** `(…, 0xfffff8002e154930,
  0xffffffffc000009a, 0x4)`, reboot, Code 43 (`no NVIDIA adapter`). No host Xid. The frames stayed black (the stall came
  3.4 s after the first scanout, before the lock screen drew).
- **Stall snapshot (1 s after the last doorbell, and at each twin's free):** for all nine Passthrough twins including
  `0x1016`, **`GPGet == GPPut` and `Get == Put`** — the engines fetched every GPFIFO entry and executed every method
  (no channel is held in an acquire: a held acquire stops `Get` short of `Put`). Every semaphore the last segments name holds
  its release value, e.g. `0x1016`: host `SEM RELEASE va=0x404a000 payload=0x52 memory=0x52`, CE release `va=0x1200eb0a0`
  payload 0 memory 0; `0x15`: `SEM RELEASE va=0x4034000 payload=0x1b6 memory=0x1b6`, 3D report `va=0x1200e7000 payload=0x1a
  memory=0x1a`; `0x13`, `0xe`, `0xf`, `0x11`, `0x1012`, `0x1014` likewise ("a release the memory holds"). VFIO boot3 cannot
  show the same state (it traces BAR0 and GSP RPCs, not guest memory).

**Conclusion (measured, runs 100 and 102, RTX 4070, 2026-10-09):** the GPU did all the work, its fences are in guest memory, and the completion interrupts were
raised on GR0's and CE3's vectors; the guest kept rendering (doorbells) but **stopped presenting**: no flip after the last
latched one, and the TDR fires ~2.1 s after that last flip in both runs 100 and 102. `[inferred]` What times out is the
present/flip path (the KMD's flip queue waiting for the last flip's completion), not render work. The armed-rule relay is
not the fix for this wall (falsifier met); it stays merged (owner-ruled).

## 9. Boot 4 (run 103), stated before the boot (2026-10-09, RTX 4070): H-flipdone

Same binary and flags as run 102 plus `KF3_DISPLAY_TRACE=1` (each window completion kf3 publishes: `TRACE notify chn …
handle … +offset … -> result` and `TRACE release chn … value … -> result`, and `TRACE window N latched`).
**H-flipdone:** the last latched flip's completion (its `SET_SEMAPHORE_RELEASE` value at `SEMAPHORE_CONTROL`'s offset in
context DMA `0xff1fe1b0`, and its notifier per `SET_NOTIFIER_CONTROL`) is not published where/when the KMD reads it.
**Falsifier:** for the last latched flip, `TRACE release chn 1 … -> Ok(())` and `TRACE notify chn 1 … -> Ok(())` appear within
one frame of its latch, with the same offsets and values as the earlier flips that did complete, and the guest still stops
flipping — then kf3 publishes what it publishes for every flip and the wall is in what the KMD expects beyond it.

## 10. Run 103 (binary `kf3-bins/3e9bcdce`, boot 4, RTX 4070, 2026-10-09): H-flipdone FALSIFIED; the stall follows a burst, and the D3D GR twin faults (Xid 31)

`[measured, run 103 at 3e9bcdce, RTX 4070, 2026-10-09 23:51:49-23:54:41 UTC]` files `run103-*` ([harness](run103-harness.log),
[qemu log](run103-qemu.log.gz), [BAR0 trace](run103-trace.log.gz), [frames](run103-frames.tsv), [bugcheck](run103-bugcheck.json)),
[host Xid lines + final host state](host-xid-and-state-20261009.txt). Flags = run 102's + `KF3_DISPLAY_TRACE=1`.
kf3 clock → UTC: second of day = `kf − 190062.364` (window-0 PUT `0x940` at kf 276072.786060 = trace 23:53:30.421645; ⊘ the
kf3 `WTRACE` write lines are capped, so the late `0x611d80` writes in the log are not the trace's last ones — the
alignment uses a PUT value present on both).

**Owner observation (UNCONFIRMED by a frame, consistent with the traces):** the lock screen stayed up a long time (clock 11:52),
the owner interacted with it through the broker window (~01:52-01:54 CEST), saw a short animation, then black. (1) kf3 and
the broker log no input events (the broker log has attach/format lines only; input goes through QEMU's USB tablet, which
the BAR0 trace does not see), so the interaction is placed by its effect: the first flip burst after the idle period.
(2) `[measured, run 103 trace + kf3 log, RTX 4070, 2026-10-09]`:
- first scanout +12.49 s (≈23:52:05.4); initial lock-screen draw 23:52:08-09 (53 flips); frames 23:52:09.3-23:52:44 show the
  lock screen ([frames](run103-frames.tsv); the frame series ended before the interaction);
- **idle lock screen: one flip per 30 s** (23:52:39, 23:53:09: 4 window-0 writes, 1 LATCH, 1 release, 1 notify each; 2-3
  doorbells on `0x11`/`0x13`/`0x15`);
- **burst (the interaction → animation): 23:53:30.40-23:53:31.25**, 26 flips at 60 Hz (106 window-0 writes) with doorbells
  on `0x11`/`0x13`/`0x15`/`0x1014`/`0x1016`; **last latched flip 23:53:31.251** (kf 276073.6156);
- 23:53:31.79-31.81 (kf 276074.156-.176): walks for page-table `SPLIT` tickets of the kernel copy channel `0x80c` UNMAP and,
  20 ms later, re-MAP at new backing pages ranges of the D3D process's space `VasKey(…399696)` — among them
  `va=0x404c000 len=0x2000` (unmapped 276074.156, `0x404d000` re-mapped 276074.176 at another page);
- stall snapshot #4 at 23:53:33.17 (1 s after the last doorbell): **token `0x15` (the D3D GR twin, host `0x3c`): `GPGet=0x2f0
  GPPut=0x3d7` — entries put and NOT fetched; USERD `Get=0x404d490 Put=0x404d504`: the PBDMA stopped inside segment
  GP[0x2ef] (`va=0x404d440`, 49 words), 20 words in** — i.e. in the page `0x404d000` that the walk had just unmapped and
  re-mapped. Every other twin (`0xe`, `0xf`, `0x11`, `0x13`, `0x1010`-`0x1016`): `GPGet == GPPut`, releases in memory.
- **23:53:33 host Xid 31: `channel 0x0000003c … ENGINE GR0_PBDMA0 HUBCLIENT_ESC faulted @ 0x0_04034000 … FAULT_PTE
  ACCESS_TYPE_VIRT_WRITE`** — token `0x15`'s PBDMA writing its host semaphore (`SEM_ADDR 0x4034000`, the release every
  earlier snapshot of `0x15` found in memory). kf3's rows never unmapped `0x4034000` (one MAP at 275991.02, no UNMAP: checked
  with [tools/covers.py](tools/covers.py)); run 101's fault VAs `0x4034000`/`0x4036000` were never unmapped either, and its
  fault also followed walks unmapping other rows of the same 2 MiB region (`0x4000000-0x41fffff`) 6 s earlier.
- **23:53:34.045 the TDR's state collection — 2.79 s after the last flip** (runs 100/102: 2.12/2.06 s: the "TDR clock starts at
  the last flip" reading does not hold here; here the GR twin faulted ~1.8 s after the last flip); teardown, recovery refused
  at the T-space rule (`birth REFUSED` ×1), `SWITCH_TO_VGA`, **0x116** `(…, 0xfffff8034fe24930, 0xffffffffc000009a, 0x4)`,
  reboot, Code 43.

(3) **H-flipdone: FALSIFIED** `[measured, run 103 KF3_DISPLAY_TRACE, 2026-10-09]`: all 83 window-0 completions of the boot are published
`-> Ok(())`, each in the same millisecond as its LATCH (one release at `0xff1fe1b0 + 16·n` with the flip's value, one notifier
at `0xff1fe1a0`), the idle-period flips and the burst's alike; the last latched flip before the stall (276073.6156) got
`release +0x110 value 0x52 -> Ok(())` and `notify +0xb00 -> Ok(())` exactly like the 25 burst flips before it. Every flip of
the burst latched in its own frame (latch 9-16 ms after its PUT, the next PUT ~16 ms later): Windows never had two flips
outstanding, so the overlap case (a flip queued while the previous is still armed, retired by the next VSync, as hardware boot3
does) did not occur in this run and is not tested. **The quiet lock screen shows the display/flip path healthy at low rate,
and the burst shows it healthy at 60 Hz until the stall; the stall is in the render plane** (here: the GR twin's PBDMA faults).

`[inferred]` **H-pde (next, stated, not run):** when kf3 applies a walk's UNMAP rows inside a 2 MiB region of a Passthrough twin's
host space, the host loses the PTEs of OTHER rows of that region that kf3 still holds as mapped (`0x4034000`, `0x4036000`), so a
twin running in that space faults on a VA the guest never unmapped. Falsifier: on the bench host (raw client or fast guest), map
two 4 KiB rows in one 2 MiB region of a host VA space, unmap one through kf3's mirror path, then let a copy engine read and write
the other — no fault. Runs 100 and 102 (stall, no Xid, every twin `GPGet == GPPut`) are not explained by it.

## 11. The recovery wall (queued; analysis for an owner decision — no policy changed)

`[measured, runs 100-103]` every TDR recovery fails the same way: the kernel driver's restart creates a copy channel
(`0xc1d00048:0xff040000`, engine `0xb`, `ProcessID=4`, no context share) that `kf_rm::chanlink::windows_user_work` classifies
**USER WORK → Passthrough** ("a process other than the kernel driver's"), then a GR channel in the same VA space
(`0xc1d0004a:0xff040001`, graphics, no context share → kernel work → Translated) that the T-space rule refuses
(`KernelInUserSpace`, `V3_P1P2_TSPACE.md` §4.2) → RmAlloc `0x40` → StartDevice fails → `0x1B0` live dump → `0x116`.
Cause `[measured]`: the classifier learns ONE kernel-driver process id at the driver's first start — `0x34c` in run 102 (14 channels
declare it) — and the recovery runs in the System process (`ProcessID=4`, 5 channels), so the kernel driver's own copy channel
is taken for user work. `[source, ogkm]` none: this is Windows-KMD behaviour (the recovery thread's process), not RM's.
**Proposal (for the owner; not implemented):** (a) treat `ProcessID=4` (the Windows System process: no user code runs there)
as the kernel driver's process in `windows_user_work` — safety: a hostile guest can declare any ProcessID today; declaring 4
only moves a channel to the Translated route, the stricter one (kayfabe reads its ring and authors every host action), so it
grants nothing; the residual is that a guest-kernel lie keeps user work Translated (slower, never unsafe); or (b) re-learn the
kernel driver's process id at each adapter start (the first kernel channel after an all-free) — weaker, because "first after
an all-free" is guest-timed. The T-space rule itself (a Translated channel never shares a space with a Passthrough one) is
not touched by either.

## 15. Run 106 onward: the interrupt flood, `KF3_DEBUG_IRQ_FLOOD` (branch `claude/debug-irq-flood-20261009`, 2026-10-09)

**STATUS: LIVE, 2026-10-09 (plan committed before the first boot as 647cfbde; results in §15.1-§15.3; six boots, all spent).** Note: §12-§14 (H-pde, run 104, run 105) are on
branch `claude/windows-pde-run104-20261009`; this branch was cut from `claude/windows-reset-20261009` (2e5ddc5c), which ends at §11. The
section numbers follow that evidence branch.

**What is run.** The debug-only flood of `docs/design/V3_DEBUG_IRQ_FLOOD.md` (kf3 `kf3-bins/3448f8a3` = 2e5ddc5c + the flood commit, built
on the bench host with `build_kf3.sh`; the thread, the display read-back words and the status segment exist only with the knob set).
Harness: `wr-run.sh N 3448f8a3 "<EXTRA>"` under `flock -o /tmp/kayfabe-fastguest.lock`, `WR_SHOTS=90`, RTX 4070, with run 104's flags in
EVERY boot, including **`KF3_NO_BATCHED_MAP=1` (batching OFF, as run 104)**; only the flood word changes. The word
`KF3_DEBUG_IRQ_FLOOD=<classes>:<ms>` contains `=`, so `windows_broker.sh:283` passes it unchanged (as run 105's `KF3_PT_NSI_RELAY=0`).

**Matrix (one boot each, stop when decisive, at most 6 boots):** R0 flood off (the baseline: lock screen about 3 s, then black); R1
`all-completion:1000`; R2 `all-completion:100`; R3 `all-completion:10`. If any of R1-R3 is better than R0 (lock screen persists > 10 s),
bisect at that period: `gsp` only, `disp,dispstat` only, `nonstall` only.

**Falsifier (stated before the first boot):** if with `all-completion` at 10 ms the screen still goes black within about 5 s of the lock
screen appearing and the guest tears down as in R0, interrupts alone are not the missing piece (what remains: a completion word, a GSP
message or event, or status values). **Success (decisive):** lock screen persists > 20 s, no bugcheck for 60 s, QGA answers a command.
Not claimed by this experiment: that a flood that helps is a fix (it is a lead; the fix has to come from the real events).

**Per boot, recorded:** first/last non-black screenshot (every 997th byte of the PPM above 16, `irqflood/tools/nonblack.py`), MSI per
second (`tools/tl.py`, as `tl104.py`), whether QGA answered, host Xid count before and after, the bugcheck when readable, the flood's
per-vector raise counts (the status line's `PERTURBING DIAGNOSTIC ON: irq-flood ...` segment).

### 15.1 What was run (six boots, RTX 4070, 2026-10-09, UTC; evidence `irqflood/run106 ... run111`)

Binaries: `kf3-bins/647cfbde` (R0 and the flawed first flood boot), `kf3-bins/b728b480` (the fixed class sets; flood-off code identical to
647cfbde), `kf3-bins/3e9bcdce` (the old binary, one control). All with run 104's flags including `KF3_NO_BATCHED_MAP=1` (**batching OFF in
every boot**), `WR_SHOTS=90` (about 72 s of screenshots from the launch), the BAR0 read trace on. Launcher `irqflood/irqflood-launch.sh`
(= run 105's launcher with the flood word as an extra `WIN_FLAGS` word). Host Xid count: 38 before and after every boot (no new Xid).
Group 11 back on DMA-FQ and 01:00.0 on nvidia after each. A first launch of run 106 with `kf3-bins/3448f8a3` (built in the wrong QEMU tree: no
GSP observer, QEMU refused realize at start, no guest ever booted) was discarded and its directory removed; the QEMU tree used from then on is
a private copy of the observer-patched `readtrace` tree.

| run | binary | flood | lock screen (UTC; s after launch) | what ended it | QGA probes (harness: 240 s wait, +40 s, then the three probes) | bugcheck on the disk |
|---|---|---|---|---|---|---|
| 106 R0 | 647cfbde | off | 10:21:41.3 (+16.4), still on at the last shot 10:22:39.6 (>=58 s) | stall marker +229 s (10:25:13.7); `UnloadingGuestDriver` about 10:25:16 (+231 s); trace silent after | no (3/3 timeout) | none (no dump) |
| 107 control | 3e9bcdce | off | never drawn (non-black fraction <= 0.011: boot spinner only) | stall marker +20.5 s, teardown +23 s | no (3/3 timeout) | none |
| 108 R3, flawed | 647cfbde | all-completion:10, nonstall class wrongly held vectors 132/133 | never drawn; driver init stopped at RPC #494 (vsyncs=0), ISR spinning | never reached a stall marker; `vdr_monitor_info` answered at +345 s with the GPU in Code 43 | 2 of 3 timed out, `vdr_monitor_info` answered: Error 43 | 0x9f |
| 109 R3 | b728b480 | **all-completion:10** | 10:49:58.8 (+16.1), still on at the last shot 10:50:57.5 (>=58 s); the clock tick drawn 10:50:28 | no stall marker through the 240 s wait; D3D11 probe at +286 s OK; **0x116 about 10:55:08 (+325 s), after the D3D12 probe had run for 38 s** | **yes**: D3D11 clear/copy 3 rounds `EXPECTED`, `VC ... status=OK`, `nvidia-smi` OK; D3D12 probe timed out | 0x116 (`100926-4312-01.dmp`) |
| 110 | b728b480 | gsp:10 | never drawn | stall marker +16.5 s, driver unloaded | yes, but `no NVIDIA adapter`, Error 43 | 0x116 |
| 111 | b728b480 | **nonstall:10** | 11:02:30.8 (+17.4), still on at the last shot 11:03:28.2 (>=57 s) | no stall marker through 240 s; D3D11 probe at +285 s OK; **driver torn down at 11:07:11 (+298 s), 9 s after the D3D12 probe's GPU traffic began** | **yes**: D3D11 `EXPECTED` x3; then D3D12 timed out and `vdr` read Error 43, `nvidia-smi` failed | 0x116 |

R1 (`all-completion:1000`) and R2 (`all-completion:100`) were NOT run: the six-boot budget went to R0, one control of the old binary, one flawed boot
(below), R3, and two bisect boots (`gsp`, `nonstall`); `disp` (+`dispstat`) was not bisected and `dispstat` was never used. The order deviates from the
brief (R3 before R1/R2) because the falsifier is stated on R3.

The flawed boot (run 108), `[measured]`: the first `nonstall` class held every row's non-stall vector. The served table repeats the stall vector in the
non-stall column for engines 59-64, 73 and 1 (vectors 64, 72, 129, 131, 132, 133, 134, 148), so one raise of 132/133 left `LEAF(4)` = `0x30`
pending for good: **9067 reads of `0xb81010` per second** (55k reads of the interrupt tree per second, 215 MSI/s), the driver stopped after its 494th RPC.
A stall interrupt with no cause behind it is a level the guest ISR cannot clear. Fixed in `b728b480` (a vector any row uses as a stall vector is in
`errors`, never in `nonstall`); the unit test now encodes it. Run 108 is not evidence about the missing notification.

### 15.2 Result against the falsifier

**The falsifier is NOT met.** With `all-completion:10` (run 109) the lock screen came up as in the baseline and was not followed by the teardown: the
driver stayed up through the idle lock screen for at least 285 s (a D3D11 clear, copy and readback on the real GPU returned the expected pixel, `nvidia-smi`
worked, the monitor status was OK), against 231 s in R0 and 23 s in the old-binary control. So interrupts alone are NOT excluded as the missing piece.
Narrowing: `gsp:10` alone did not help (teardown at +16.5 s, no lock screen), `nonstall:10` alone did (run 111, the same >= 285 s with D3D11 working).

**The success criterion is only partly met.** Lock screen > 20 s: yes (both arms, >= 58 s seen in the 72 s of screenshots; the guest was alive at +285 s).
QGA answered a command: yes. No bugcheck for 60 s: yes at the lock screen (nothing for 280 s), **but both arms bugchecked 0x116 minutes later, once the harness's
D3D12 signal probe ran** (run 109 about 10:55:08, run 111 about 11:07:11, MSI/BAR0 silent afterwards). So the flood postpones the death; it does not cure it.

`[measured]`
- All numbers in the table; per-run files `irqflood/run1NN/{analysis.txt,per-second.txt,qemu-flood-lines.txt,qemu-teardown-lines.txt,bugcheck.json,wr-run1NN.log}`;
  frames `frame-first-lock.png` (clock "10:49" / "11:02") and `frame-last.png` per run.
- Flood status lines: run 109 `PERTURBING DIAGNOSTIC ON: irq-flood nonstall,gsp,disp:10 ticks=51107 raised[v0=50250 v1=50250 v2=50250 v3=50250 v4=50250 v5=50250
  v154=30112 v155=29840]`; run 111 `irq-flood nonstall:10 ticks=34268 raised[v0=33199 ... v5=33199]`; run 110 `irq-flood gsp:10 ticks=8423 raised[v155=574]`. The guest keeps vectors
  154/155 enabled only part of the time (v154 and v155 stopped counting at 30112/29840 ticks in run 109, v155 at 574 in run 110 when the driver unloaded): "only enabled
  vectors" means the GSP class fires only while the guest waits on its queue.
- MSI per second at the lock screen: R0 median 115 (83-142); run 109 median 209 (162-244); run 111 median 211 (174-291): the flood adds about 100 per second (one per
  tick), the guest's own rate is unchanged. Hardware idle: 17-19.
- The flood wrote no completion word (code: only `latch_and_deliver`); the guest's ISR serviced and cleared what was raised: in one sampled second of run 109 (10:52:20) `LEAF(4)` (`0xb81010`)
  read `0xc000000` (vectors 154 and 155) on 218 reads, 698 W1C writes went to `LEAF(0)` and 198 to `LEAF(4)`; vector 155 is pending in about 31% of the `LEAF(4)` reads there (hardware: 24% of the ISR reads, kayfabe without flood: 0.2%).

`[inferred]` (not shown by one boot per arm)
- A lost or missing NON-STALL notification (vectors 0-5: GR0 and the copy-engine notifier vectors of the served table) is what the idle-lock-screen death needs; the GSP interrupt
  by itself is not (run 110). Strength: n = 1 per arm; flood-off boots tore down at about 18-23 s (runs 104, 105, 107) or at 231 s (R0), flood-on boots lived to at least 285 s. The baseline spread is
  wide (R0 itself kept the lock screen for 215 s), so a repeat of R0 and of `nonstall:10` is needed before this is called established.
- The flood works as a wake-up the guest would otherwise miss; it cannot supply what a specific D3D12 wait needs, which is why the D3D12 probe still ends in 0x116. Not tested: that the
  D3D12 probe is the trigger rather than the next idle death (its GPU traffic precedes the teardown by 9 s in run 111; run 109's D3D12 probe made no GPU traffic of its own).
- A perturbed run is not a normal run (about 100 more MSIs per second and their ISR/DPC work); a timing effect of the flood, rather than a notification it supplies, is not excluded.

**Next (in order of cost):** (1) repeat R0 (flood off, `b728b480`) and `nonstall:10` twice each to get the spread; (2) bisect vectors 0-5 (GR0 vector 0 against the CE vectors 1-5) and the period
(100 and 1000 ms) at `nonstall`; (3) find which host non-stall edge the relay does not raise: the `PT-NSI` lines with `live twins 0` and `unraised_no_live` against the hardware pattern of §13.2 (GR0 vec 0 pending 45%,
CE2 57%), then fix that edge on the real path; (4) the D3D12 probe death under `nonstall:10` (which wait does it hang on). The flood stays a diagnostic.
