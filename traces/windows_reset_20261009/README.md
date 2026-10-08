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
