**STATUS: LIVE, 2026-10-14.** The Linux plane oracle, its bare-metal reference, what it reproduced (the
overlay-flip completion class of G01, twice), what was fixed, and what the owner's current Windows 11 resets
correlate with instead. Branch `claude/linux-oracle-planes-20261014` (from `integration/prod-candidate-2-20261013`
at `bf510bdf`). Labels: **[measured]** = a run named here, evidence under
`traces/linux_planes_oracle_20261014/`; **[read]** = public source (ogkm 595.84, the one `nvidia-drm`/NVKMS the
oracle ran is 580.159.04) or kayfabe code; **[inferred]** = reasoning, never written as evidence.

# V3 display — the Linux plane oracle

## 0. Short answer

1. **Reproduced on Linux [measured].** A stock driver sequence — enable an overlay plane, then flip both planes —
   deadlocked kayfabe's display engine (bf510bdf): the overlay's UPDATE waited forever for a primary UPDATE that had
   already been consumed (the engine paired overlay(k) with primary(k+1)), the next pattern change (the plane's
   disable) parked the core channel for the VM's life, and every later overlay test timed out. Same shape as the
   Windows "plane 1 never completes" TDR (G01). Root cause: the 2026-10-10 rule that an UPDATE naming a channel
   *parked only for its vblank* joins that latch. Fixed in `a34d3003` (rule removed; engine test fails before,
   passes after). The bare-metal AD104 reference completes the same sequence [measured].
2. **Second root cause [measured on the reference, then fixed].** A commit that only *disables* a plane gets its
   DRM event within a frame on real hardware (7-17 ms; DPMS off 34-55 ms; 0 "Flip event timeout"); kayfabe raised
   nothing (3 s nvidia-drm timeout per disable; 45 timeouts in one run). The earlier comment in the engine that
   "real GPUs log the same (#1361)" is falsified by the reference. Fixed in `113a7535`: the flipped-away entry's
   FINISHED notifier raises the channel's AWAKEN (its own WRITE_AWAKEN mode), once per channel per latch. Windows
   flipped plane 1 to NO surface right before the TDRs of runs 400/403 [measured then]; that this was the TDR's
   cause on Windows is **[inferred]**, to be confirmed by the Windows steps in §7.
3. **Caps and the YUV plane.** The real AD104 offers NV12/NV21/NV16/NV61/NV24/NV42/P010/P012/P210 on every plane
   [measured]; kayfabe's authored caps page offered none, so Linux could not exercise a YUV overlay. The page now
   claims semi-planar (`113a7535`). The console composes the window with its own FMT matrix, 8-bit and 16-bit
   semi-planar and packed 4:2:2, bilinear when scaled (`41acc6ac`): NV12, P010 and YUYV overlays are pixel-exact
   on the console [measured, run 4].
4. **The owner's current Win11 resets are not overlay resets [measured].** Runs 2980/2990 (kayfabe bf510bdf, the
   owner's Win11 image): 0 overlay windows ever scanned (only window 0), no display STALL; 11 of 13 GSP reset
   cycles follow a Windows USER-ring scanner refusal (`USER_RING_SCAN Legacy`, `SubDeviceMask(<data-looking word>)`,
   `MethodOverflow`; "no completion/reuse credit") within ~5-25 s, and the guest's channel recovery
   (`NV906F_CTRL_CMD_RESET_CHANNEL`, 0x906f0102) is refused by kayfabe before each adapter reset. A bounded refusal
   diagnostic is added (`113a7535`); §6 has the hypotheses and falsifiers. Not fixed here (not display).

## 1. Why Linux is the oracle

`nvidia-drm` → NVKMS (`nvkms-evo3.c`) drives the same NVDisplay classes (C67D/C67E/C67B on Ada) and the same RM
event path Windows' KMD uses: window/core UPDATEs with interlock flags, completion notifiers with AWAKEN, the
AWAKEN interrupt → RM channel event (`kern_disp_0300.c` `_kdispHandleAwakenChnMask`) → NVKMS FLIP_OCCURRED
(`nvkms-evo.c` CompletionNotifierEvent) → the DRM page-flip event [read]. Its source is public, so "what was sent"
and "what was expected" can be read, not guessed; and the same binary can be run on bare metal.

## 2. The tools (all under `scripts/bench/display/`)

| file | what |
|---|---|
| `kfplanes.c` | the guest/bare-metal program: libdrm atomic commits with `PAGE_FLIP_EVENT`, every wait bounded. `list` (connectors, planes, formats, props), `all <n>` (every test below, one `KFPLANES_RESULT` line each), `show <card> <rgb\|nv12\|p010\|yuyv\|nv12-scaled> <s>` (hold a known primary + overlay for a screendump), `ppm` (built `-DKFPLANES_NO_DRM`: the expected frame) |
| `planes_oracle.sh <kf3-rev> <tag>` | trusted host: one Linux desktop-image guest on its own qcow2 child, 4 GiB, fb 2 GiB, kf3 `display=on` without a broker, its own lock/QEMU name/pid; stops the display manager, runs `kfplanes list/all`, then `show` + QMP screendump + `planes_compare.py`; guest dmesg slice, kf3 display flags, host Xid delta, `PLANES_EXIT` |
| `planes_compare.py` | screendump vs reference: mismatches in/out of the overlay, and which YUV→RGB conversion the console matched (709/601/2020, limited/full, U/V order) |
| `planes_hw_reference.sh` | the bare-metal reference inside a rented vast KVM VM (GPU by VFIO, no monitor): the open 580.159.04 modules, one HDMI connector forced on (`status=on`) with the kayfabe guest's own EDID (`drm.edid_firmware`), then the same `kfplanes list/all` |
| `planes_table.py` | the comparison table below from the runs' `all.log` |

Run (trusted host, owner VM may run beside it): `bash planes_oracle.sh <rev> <tag>`; evidence in
`/var/lib/kf-windows-20261005/claude-planes-<tag>/`. It refuses when `nvktap0` is in use.

## 3. Hypotheses and falsifiers (each written before its run; the files are in the run directories)

| run | rev | hypothesis | falsifier | result |
|---|---|---|---|---|
| 1 | bf510bdf | H-PLANE-LINUX: kayfabe completes every Linux overlay flip within a frame like the primary | an overlay series with a timeout, >5% events > 1 frame + 2 ms, or vblank steps > 1, while primary passes | **FALSIFIED** [measured]: argb-both 1 timeout, xrgb-only 4/4 timeouts, 12 STALL reports, the core channel parked for the VM's life, 16 guest "Flip event timeout" |
| 2 | a34d3003 | H-JOIN: the Latch-join rule caused run 1's deadlock; without it every overlay series completes | any overlay series timeout or interlock STALL | **holds for flips** [measured]: every argb/xrgb/yuyv series 120/120, 0 STALL; disables still raise nothing (45 timeouts) → H-DISABLE |
| HW | open 580.159.04 on AD104 | the reference: what real hardware does with the same binary | — | every series completes; a plane disable gets its event in 7-17 ms; DPMS off 34-55 ms; 0 timeouts [measured] |
| 3 | 113a7535 | H-DISABLE: with the flip-away AWAKEN (+ semi-planar caps) a disable completes within a frame; NV12/P010 offered and complete; DPMS off < 0.2 s | any disable without event, any guest Flip event timeout, any overlay timeout/STALL, DPMS off ≥ 1 s, or an extra event making flips complete early (p95 < 8 ms) | **holds** [measured]: every disable 0.7 ms, NV12/P010/YUYV series complete, 0 guest timeouts, DPMS off 1.6 ms, overlay p95 17.1 ms (not early) |
| 4 | 41acc6ac | H-YUV-CONSOLE: the console composes NV12, P010 and YUYV as their FMT matrix says | any CMP OVERLAY_DIFF for nv12/p010/yuyv, any flip timeout/STALL | **holds** [measured]: nv12/p010/yuyv 0 mismatches of 230400 overlay pixels, max error 0 |

Every run: host Xid delta 0, the owner's VM alive [measured].

## 4. The comparison (bare metal vs kayfabe, `planes_table.py`)

Latency is commit → DRM event. 120 flips per series (run 4: 60). "-" = not run (run 1 died at the xrgb test).

| test | HW AD104 (bare metal, 580.159.04 open) | kf bf510bdf (before) | kf a34d3003 | kf 113a7535 | kf 41acc6ac |
|---|---|---|---|---|---|
| vblank-wait | FAIL err -95 | FAIL err -95 | FAIL err -95 | FAIL err -95 | FAIL err -95 |
| vblank-event | FAIL  | FAIL  | FAIL  | FAIL  | FAIL  |
| primary-flip | 120/120 ev, 0 tmo, p95 17.1 ms, max 17.1 ms | 120/120 ev, 0 tmo, p95 17.0 ms, max 17.1 ms | 120/120 ev, 0 tmo, p95 17.1 ms, max 17.1 ms | 120/120 ev, 0 tmo, p95 17.1 ms, max 17.1 ms | 60/60 ev, 0 tmo, p95 17.1 ms, max 17.1 ms |
| overlay-argb-only | 120/120 ev, 0 tmo, p95 16.7 ms, max 17.5 ms | 120/120 ev, 0 tmo, p95 17.0 ms, max 27.4 ms | 120/120 ev, 0 tmo, p95 17.0 ms, max 26.5 ms | 120/120 ev, 0 tmo, p95 17.1 ms, max 27.7 ms | 60/60 ev, 0 tmo, p95 17.1 ms, max 27.7 ms |
| disable after overlay-argb-only | event 1 16.5 ms | event 0  | event 0  | event 1 0.7 ms | event 1 0.7 ms |
| overlay-argb-both | 120/120 ev, 0 tmo, p95 19.8 ms, max 35.0 ms | 119/120 ev, 1 tmo, p95 17.1 ms, max 17.1 ms | 120/120 ev, 0 tmo, p95 17.0 ms, max 28.4 ms | 120/120 ev, 0 tmo, p95 17.2 ms, max 25.8 ms | 60/60 ev, 0 tmo, p95 17.2 ms, max 25.6 ms |
| disable after overlay-argb-both | event 1 9.5 ms | event 0  | event 0  | event 1 0.7 ms | event 1 0.7 ms |
| overlay-xrgb-only | 120/120 ev, 0 tmo, p95 16.7 ms, max 27.4 ms | 0/4 ev, 4 tmo, p95 0.0 ms, max 0.0 ms | 120/120 ev, 0 tmo, p95 17.0 ms, max 23.4 ms | 120/120 ev, 0 tmo, p95 17.1 ms, max 25.6 ms | 60/60 ev, 0 tmo, p95 17.1 ms, max 25.6 ms |
| disable after overlay-xrgb-only | event 1 16.6 ms | event 0  | event 0  | event 1 0.7 ms | event 1 0.7 ms |
| overlay-nv12-only | 120/120 ev, 0 tmo, p95 16.7 ms, max 17.2 ms | SKIP overlay plane does not list the format | SKIP overlay plane does not list the format | 120/120 ev, 0 tmo, p95 17.1 ms, max 23.7 ms | 60/60 ev, 0 tmo, p95 17.2 ms, max 24.1 ms |
| disable after overlay-nv12-only | event 1 16.6 ms | - | - | event 1 0.7 ms | event 1 0.7 ms |
| overlay-nv12-both | 120/120 ev, 0 tmo, p95 22.0 ms, max 32.4 ms | SKIP overlay plane does not list the format | SKIP overlay plane does not list the format | 120/120 ev, 0 tmo, p95 17.1 ms, max 23.6 ms | 60/60 ev, 0 tmo, p95 17.2 ms, max 23.5 ms |
| disable after overlay-nv12-both | event 1 7.5 ms | - | - | event 1 0.7 ms | event 1 0.7 ms |
| overlay-p010-only | 120/120 ev, 0 tmo, p95 19.9 ms, max 21.4 ms | SKIP overlay plane does not list the format | SKIP overlay plane does not list the format | 120/120 ev, 0 tmo, p95 17.1 ms, max 23.5 ms | 60/60 ev, 0 tmo, p95 17.1 ms, max 22.7 ms |
| disable after overlay-p010-only | event 1 16.5 ms | - | - | event 1 0.7 ms | event 1 0.7 ms |
| overlay-yuyv-only | 120/120 ev, 0 tmo, p95 16.7 ms, max 17.5 ms | - | 120/120 ev, 0 tmo, p95 17.0 ms, max 34.7 ms | 120/120 ev, 0 tmo, p95 17.0 ms, max 26.7 ms | 60/60 ev, 0 tmo, p95 17.1 ms, max 25.5 ms |
| disable after overlay-yuyv-only | event 1 16.4 ms | - | event 0  | event 1 0.7 ms | event 1 0.8 ms |
| overlay-nv12-scaled | SKIP enable commit refused rc=-22 | - | SKIP overlay plane does not list the format | SKIP enable commit refused rc=-22 | SKIP enable commit refused rc=-22 |
| disable after overlay-nv12-scaled | commit rc=-22 (nothing to disable) | - | - | commit rc=-22 (nothing to disable) | commit rc=-22 (nothing to disable) |
| overlay-argb-scaled | SKIP enable commit refused rc=-22 | - | SKIP enable commit refused rc=-22 | SKIP enable commit refused rc=-22 | SKIP enable commit refused rc=-22 |
| disable after overlay-argb-scaled | commit rc=-22 (nothing to disable) | - | commit rc=-22 (nothing to disable) | commit rc=-22 (nothing to disable) | commit rc=-22 (nothing to disable) |
| overlay-nv12-toggle | 60/60 ev, 0 tmo, p95 23.2 ms, max 26.6 ms | - | SKIP overlay plane does not list the format | 60/60 ev, 0 tmo, p95 16.1 ms, max 26.7 ms | 30/30 ev, 0 tmo, p95 16.8 ms, max 23.4 ms |
| disable after overlay-nv12-toggle | commit rc=-22 (nothing to disable) | - | - | commit rc=-22 (nothing to disable) | commit rc=-22 (nothing to disable) |
| overlay-argb-toggle | 60/60 ev, 0 tmo, p95 23.0 ms, max 32.3 ms | - | 4/8 ev, 4 tmo, p95 20.4 ms, max 20.4 ms | 60/60 ev, 0 tmo, p95 16.8 ms, max 25.5 ms | 30/30 ev, 0 tmo, p95 16.8 ms, max 23.4 ms |
| disable after overlay-argb-toggle | commit rc=-22 (nothing to disable) | - | commit rc=-22 (nothing to disable) | commit rc=-22 (nothing to disable) | commit rc=-22 (nothing to disable) |
| overlay-argb-cycle (disables) | 8/8 disable events | - | 0/8 disable events | 8/8 disable events | 8/8 disable events |
| overlay-argb-cycle | 80/80 ev, 0 tmo, p95 32.4 ms, max 34.0 ms | - | 80/80 ev, 0 tmo, p95 32.4 ms, max 33.6 ms | 80/80 ev, 0 tmo, p95 32.5 ms, max 32.9 ms | 80/80 ev, 0 tmo, p95 32.3 ms, max 32.8 ms |
| overlay-nv12-cycle (disables) | 8/8 disable events | - | - | 8/8 disable events | 8/8 disable events |
| overlay-nv12-cycle | 80/80 ev, 0 tmo, p95 31.9 ms, max 33.3 ms | - | SKIP no overlay plane with the format | 80/80 ev, 0 tmo, p95 32.8 ms, max 32.9 ms | 80/80 ev, 0 tmo, p95 32.4 ms, max 32.5 ms |
| cursor-atomic | 120/120 ev, 0 tmo, p95 20.6 ms, max 23.2 ms | - | 120/120 ev, 0 tmo, p95 0.7 ms, max 0.8 ms | 120/120 ev, 0 tmo, p95 0.7 ms, max 0.8 ms | 60/60 ev, 0 tmo, p95 0.7 ms, max 0.8 ms |
| gamma-lut | 60/60 ev, 0 tmo, p95 11.4 ms, max 16.5 ms | - | 60/60 ev, 0 tmo, p95 0.7 ms, max 0.7 ms | 60/60 ev, 0 tmo, p95 0.7 ms, max 0.8 ms | 30/30 ev, 0 tmo, p95 0.7 ms, max 0.8 ms |
| degamma-lut | 60/60 ev, 0 tmo, p95 13.2 ms, max 16.5 ms | - | 60/60 ev, 0 tmo, p95 0.7 ms, max 0.7 ms | 60/60 ev, 0 tmo, p95 0.7 ms, max 0.7 ms | 30/30 ev, 0 tmo, p95 0.8 ms, max 0.8 ms |
| ctm | 60/60 ev, 0 tmo, p95 3.4 ms, max 9.6 ms | - | 60/60 ev, 0 tmo, p95 0.7 ms, max 0.7 ms | 60/60 ev, 0 tmo, p95 0.7 ms, max 0.7 ms | 30/30 ev, 0 tmo, p95 0.8 ms, max 0.8 ms |
| primary-async-legacy | 120/120 ev, 0 tmo, p95 0.6 ms, max 27.4 ms | - | 120/120 ev, 0 tmo, p95 17.0 ms, max 17.0 ms | 120/120 ev, 0 tmo, p95 17.1 ms, max 17.2 ms | 60/60 ev, 0 tmo, p95 17.1 ms, max 17.2 ms |
| primary-async-atomic | 120/120 ev, 0 tmo, p95 0.6 ms, max 0.6 ms | - | 120/120 ev, 0 tmo, p95 17.0 ms, max 25.6 ms | 120/120 ev, 0 tmo, p95 17.1 ms, max 17.2 ms | 60/60 ev, 0 tmo, p95 17.1 ms, max 17.2 ms |
| out-fence | 60/60 signalled, late 23, max 31.7 ms | - | 60/60 signalled, late 0, max 16.9 ms | 60/60 signalled, late 0, max 17.1 ms | 30/30 signalled, late 0, max 17.0 ms |
| in-fence | 30/30 ev, early 0, after-signal max 4.9 ms | - | 30/30 ev, early 0, after-signal max 10.0 ms | 30/30 ev, early 0, after-signal max 14.1 ms | 15/15 ev, early 0, after-signal max 10.1 ms |
| dpms-cycle | off max 55.0 ms, on max 44.5 ms | - | off max 3067.3 ms, on max 5.3 ms | off max 1.6 ms, on max 4.7 ms | off max 1.6 ms, on max 4.3 ms |
| flip-after-dpms | 30/30 ev, 0 tmo, p95 27.4 ms, max 28.2 ms | - | 30/30 ev, 0 tmo, p95 16.8 ms, max 16.8 ms | 30/30 ev, 0 tmo, p95 17.0 ms, max 17.0 ms | 30/30 ev, 0 tmo, p95 17.0 ms, max 17.1 ms |

### 4.1 Verdict per feature

| feature | verdict | note |
|---|---|---|
| primary flips, vsync | exercised, matches | p95 17.1 ms both |
| overlay flips (RGB, plane-only and with the primary) | exercised, **differed → fixed** (a34d3003) | the enable + both-planes deadlock |
| overlay plane disable / enable cycles (MPO life cycle) | exercised, **differed → fixed** (113a7535) | no event on disable |
| YUV overlay formats offered (NV12/P010/…) | exercised, **differed → fixed** (113a7535) | caps page semi-planar |
| YUV overlay flips (NV12, P010, YUYV) | exercised, matches (after the caps fix) | |
| YUV overlay on the kf3 console | exercised, **fixed** (41acc6ac) | P010/YUYV were left out; FMT matrix now honoured |
| overlay scaling | exercised, matches | refused (EINVAL) by NVKMS on both: the overlay window has no precomp scaler [measured on HW] |
| multiple windows per head | exercised, matches | one primary + one overlay per head on both (2 windows/head) |
| cursor plane (atomic, moving every flip) | exercised, **differs (early)** | HW completes at the vblank (p95 20.6 ms); kayfabe in 0.7 ms — a commit with a core-channel UPDATE latches at once (the `core_latch_at_vblank` experiment, off). Completion early, never late: no hang; precise item D-EARLY below |
| gamma / degamma LUT, CTM | exercised, **differs (early)** | same D-EARLY (HW: LUT notifier pending ~a frame, EBUSY on the next commit) |
| async (tearing) flips | exercised, **differs by ruling** | HW 0.6 ms; kayfabe paces tearing flips to the vblank tick (`OWNER_RULINGS` §M D1, `display-max-fps`) |
| out-fence | exercised, matches | HW signals later (23/60 > 1 frame) than kayfabe (0/60) |
| in-fence (sw_sync) | exercised, matches | no early completion on either |
| DPMS / modeset cycle | exercised, **differed → fixed** (113a7535) | off: HW 34-55 ms, kayfabe 3.07 s before (disable without event), 1.6 ms after (D-EARLY: no vblank wait) |
| vblank ioctl / vblank events | not exercisable | `drmWaitVBlank` → EOPNOTSUPP on both: nvidia-drm on kernels with `NV_DRM_CRTC_STATE_HAS_NO_VBLANK` never calls `drm_vblank_init` [read, `nvidia-drm-drv.c`]; flip events carry sequence 0 |
| vblank-synchronous flips | exercised, matches | one flip per 16.7 ms frame in steady state on both |
| hotplug / EDID | partly | EDID read, header/checksum valid [measured]; a hotplug event cannot be generated from inside the guest (QEMU-side), not exercised |
| HDR metadata / Colorspace / max bpc props | not comparable | connector type differs (HW forced HDMI-A has `HDR_OUTPUT_METADATA`/`Colorspace`; kayfabe's monitor is DVI-D, which has none) |
| 3-plane YUV | not exercisable | not offered on the AD104 either (CAPA.SUPPORT_PLANAR clear) |

**D-EARLY (precise item, not fixed):** an update group that contains the core channel latches immediately
(`engine.rs` `group_ready`: `has_core` → park only when `core_latch_at_vblank`), so commits that touch the core
(cursor, LUT/CTM, plane disable, DPMS) complete before the vblank that real hardware waits for. No hang can come
from it; a guest that paces on these events runs faster than the display. Decide with an A/B of the existing
`core_latch_at_vblank` experiment on the Linux broker lane and one Windows run before changing the default.

## 5. The two engine root causes, in detail

### 5.1 The vblank-parked join (a34d3003)

NVKMS gives every window of an update the same interlock set — the clique of the update's windows, itself
included (`nvEvoUpdateC3` / `UpdateWindowC3`) [read]. The measured kayfabe ring of run 1 (`kf3-display-lines`):
the enable (`imm7` UPDATE, then window 7 naming only its immediate channel) was parked for the vblank; the first
both-planes flip pushed window 6 naming {6, 7}; the 2026-10-10 rule let it join the parked enable; window 7's own
flip-0 UPDATE then named window 6, whose update was gone → parked. Every later flip paired window 7(k) with window
6(k+1) (events still arrived, the overlay showed the previous frame), until the plane's disable (core + window 7 +
imm 7, all naming window 7) closed the cycle around the stuck update — core parked forever. The rule's premise,
"window 0's UPDATE names nothing" (runs 400/403), was inferred; the measured ring of run 411 shows window 0 naming
window 4. Test `linux_overlay_enable_then_flip_pairs_each_flip_with_its_own_partner` replays the measured
sequence (fails on the old engine: the overlay waits for chn 1); the superseded test is rewritten in place.

### 5.2 The disable's AWAKEN (113a7535)

The KAPI programs a completion notifier only for a non-NULL surface (`nvkms-kapi.c`), NVKMS programs the disabled
window with that NULL notifier (`EvoFlipC3Common`), and RM raises a window's event only from its AWAKEN interrupt
[read]. Yet real hardware delivers the disable's event [measured]. The one notifier such a latch writes is the
flipped-away entry's FINISHED, in that entry's own `WRITE_AWAKEN` mode — the engine now raises the channel's AWAKEN
there when no new BEGUN raises it in the same latch. Test `a_plane_disable_raises_one_awaken_through_the_flipped_away_entry`
(fails on the old engine) also pins exactly one AWAKEN for a normal flip. **[inferred]** that real hardware raises
it through the flip-away write rather than through a notifier-independent update-completion AWAKEN: both produce the
same Linux behaviour; the Windows run in §7 is the next discriminator.

## 6. Windows: what the traces show around the resets, and what Linux exercised

**The owner's current Win11 runs (bf510bdf, 19 production flags, CAPS_PROBE on).** [measured, host logs of runs
2970/2980/2990, read-only] The display shows window 0 only (no `[window 4` composition line, no YUV line, no
interlock STALL; one benign "ready, waiting for a vblank" report). The sequence before the first reset of run 2990
(boot 23:43): `USER_RING_REFUSED 0xc1d00091:0xff040013 host 0x5c: USER_RING_SCAN Legacy; no completion/reuse
credit` (t≈39.8 s) → `GSP REFUSED fn76/0x906f0102` (the guest's `NV906F_CTRL_CMD_RESET_CHANNEL`, refused 0x56) →
the display windows freed → `fn 47 UNLOADING_GUEST_DRIVER` at t=55.8 s → GSP Running→Suspending→Halted→… (the
reset). Run 2980: 6 resets, 5 refusals (4 Legacy, 1 MethodOverflow), RESET_CHANNEL refused 16 times; run 2990: 11
resets over two boots, 11 refusals (7 Legacy, 3 SubDeviceMask, 1 MethodOverflow). The refused "headers" are
data-shaped (e.g. `SubDeviceMask(0x107af0e0)`, `SubDeviceMask(0x0053004f)`). Run 2950 (ec45aad9, same image, 41
min): 0 USER-ring births, 0 resets [measured]; what the owner did in it is unknown.

★ ANSWERED 2026-10-14 (`V3_USER_RING_SCAN_RACE.md`): **H-DESYNC is falsified** — the refused words are data
the relay read too early, not packets it sized wrongly: the guest fills pushbuffers behind a `SYNC_WAIT` entry the
GPU has not reached, and the relay scanned them as soon as `GP_PUT` named them (15 of 19 traced refusals rescan
clean 1 ms later; the other 4 carry a packet from an earlier stale read). A second cause, the relay not reading a
`GP_PUT` advanced without a doorbell, produced the remaining resets. Both fixed on `claude/user-ring-scan-20261014`.
H-ESCALATE stays consistent but unacted (G17). The two hypotheses below are kept as written.

Hypotheses (not tested here; for the user-ring owner):
- **H-DESYNC:** the scanner's packet sizing drifts from the hardware's (a header it sizes differently, or a
  snapshot that is not the bytes the GPU fetches), so it reads data as headers. Falsifier: the new diagnostic line
  (`at word N/M raw=… last_header=… carried_in=… words[…]`) shows the refused word at a legitimate header position
  after a correctly sized previous packet (then it is a real legacy/mask packet, and the scanner must model it).
- **H-ESCALATE:** the refused RESET_CHANNEL turns a per-channel recovery into an adapter TDR. Falsifier: a reset
  cluster without a preceding 906f0102 refusal (the log is first-time-only; the refusal counter is the measure).

**The earlier overlay TDRs (runs 400-418, G01).** The class methods involved — window/imm interlock flags, a
window UPDATE with `INTERLOCK_WITH_WIN_IMM`, the immediate channel's `INTERLOCK_WITH_WINDOW`, core UPDATEs naming a
window, notifier BEGUN/FINISHED with `MODE_WRITE_AWAKEN`, the AWAKEN_WIN interrupt, a window flipped to NO surface
(presents 5212-5214 of run 403) and re-enabled — are all exercised by the Linux tests above [measured]. Not
exercised on Linux: Windows' semaphore acquire/release on window flips (Linux uses none here), the YCbCr format 0x38
with `SWAP_UV` under the captured caps page (Linux now uses the same format through the authored page), the window-4
assignment (Linux: windows 6/7 of head 3), and Windows' own MPO policy.

## 7. Windows validation steps (for the owner; not run here)

Build: kf3 for the head of `claude/linux-oracle-planes-20261014` (the revision is in the final report). Then:
1. Launch a NEW Win11 run (not the current one) with that binary and the owner's usual 19 flags.
2. Desktop idle 5 min, then the Start menu 10x, Edge + YouTube (a normal video, then Shorts scrolling) 15 min.
3. Record from the host log: GSP reset cycles (`Running -> Suspending`), `USER_RING_REFUSED` lines (each now ends
   with `at word … raw=… last_header=… carried_in=… words[…]` — copy them all), `GSP REFUSED fn76/0x906f0102`,
   `display: STALL`, `[window 4` composition lines, `YUV window … composed`, `scanout REFUSED`.
4. Expected if the display fixes matter for Windows: 0 interlock STALL, and when window 4 is used, its flips and
   its disable complete (no flip-queue TDR). Expected if H-DESYNC/H-ESCALATE hold: resets still follow
   USER_RING_REFUSED + 906f0102 — then the diagnostic lines decide H-DESYNC.
5. A/B for the MPO question (G02): the same steps with `OverlayTestMode=5` — if the reset count is unchanged, MPO
   is not the trigger in this image (as the owner already observed).

## 7a. Linux validation of the branch [measured, `edf2759d` = product `41acc6ac`]

GPU-free `cargo test --no-fail-fast` over every `kf-*` crate: 2986 passed, 0 failed. `v3_gates.sh`: 9/9 +
gate10 PASS, 12/12 births USER. Host-mode fast suite: 30/30. Linux desktop with the broker
(`desktop_smoke.sh`): Cinnamon + Xorg, NVIDIA renderer, glxgears ~60 FPS, 0 guest Flip event timeouts, host Xid
delta 0; one 1 s "ready, waiting for a vblank" STALL report (acquire satisfied, recovered). Evidence:
`traces/linux_planes_oracle_20261014/validation-edf2759d/`.

## 8. What remains

- ★ 2026-10-14: answered and fixed in `V3_USER_RING_SCAN_RACE.md` (G16); G17 analysed there. ~~The Win11 resets
  of §6 (user-ring scanner refusals, RESET_CHANNEL refused): not display; diagnostic only here.~~
- D-EARLY (core-group latch timing), §4.1.
- The console still ignores the colour stages after the FMT matrix (ILUT/CSC/TMO) for YUV windows; 3-plane YUV is
  left out by name; HDR/PQ YUV (L04) is not qualified.
- The caps-page MPO policy (G02) was not changed: no measurement here shows the window count matters.
