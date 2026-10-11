# Windows playback TDR, 2026-10-10 (branch `claude/playback-tdr-20261010`, from combined line 3aea0a79)

# PROGRESS H1 (2026-10-10 night, agent overlay-h1; newest first; [measured] / [inferred])

STATUS: LIVE. Work on branch `claude/overlay-h1-20261010` (from 575d5b20).

* 5. [measured, runs 417 and 418, binary 72784ba7, ZERO flags, tdr-run24.sh ETW=1 OVLDUMP=1, hold 241 s each] the stuck-flip class is GONE in both: 0 `STALL Window .. waiting for
  its interlock group`, 0 OVLDUMP triggers, TDR cycles 0/0/0/0/0 (boot/sign-in/Edge/Shorts-load/hold), 0 channel exceptions, 0 scanout REFUSED. The overlay is active in both
  (window 4 1295x985 RGB plane from +219 s / +187 s, then the NV12 video overlay `fmt 0x38` 156 / 141 console lines). Prior baseline with 18da7c3c: stall in 6 of 8 runs
  (408/409/411/413/415/416; 410 and 414 did not). Two clean runs is not yet three; run 419 is queued. `4` extra `STALL Window 4 ... ready, waiting for a vblank` reports in 417
  (parked 1012 ms while ticks and flips flow: the report's parked clock is per channel stage, not per UPDATE; benign, no TDR). The kf_overlayprobe verdict of 417/418 is NOT
  valid: the QGA staging of the exe timed out and left a truncated file (exit -1073741819, no output; stage.txt TimeoutError, sizes 803160/821160 of 839160); the same flake hit
  413 (821160) and 414. Probe verdicts of 413/415/416 (full exe, 839160 in 415): no DIRECT flag (flags 0x2) before any TDR.
* 4. [measured + inferred, runs 415/416 (18da7c3c, OVLDUMP: guest-memory dump at the first window-4 STALL report); engine replay tests] CAUSE of H1 (the
  first half measured, the pairing rule inferred):
  - [measured, dump 415] at the stall ALL vCPUs are in HLT (3 samples), the VidSch workers wait for scheduler events, DWM/Edge wait on events: nobody spins, nobody is
    blocked in a trapped BAR0 write (H-VCPU false); [measured, VFIO reference + run 414 trace] the driver does not wait for GET per flip (H-GET false for the
    flip path); the stuck window-4 UPDATE (decoded from the stalled guest's pushbuffer in the dump) is the plane's FIRST FLIP C: surface, notifier, semaphore, and
    `SET_WINDOW_INTERLOCK_FLAGS`=window 0 with `UPDATE`.INTERLOCK_WITH_WIN_IMM. The UPDATEs it names are the NEXT frame's, which the guest sends only after this
    present completes (DWM is idle). In run 414 and on hardware C pairs with window 0's and window 4's immediate channel's UPDATEs KICKED JUST BEFORE it
    (PUT order in the trace: w4 enable batches, w0, imm0, w4, imm4, w4=C).
  - [measured, run 416 WTRACE + engine ring] kayfabe paired them with EARLIER window-4 UPDATEs: (a) the display worker applied "every channel whose PUT moved" in CHANNEL
    NUMBER order at whatever moment it woke (up to ms after the writes, which are 25-1000 us apart), so window 0 (chn 1) was processed before window 4's earlier
    UPDATEs and joined their group; (b) the engine held window 4's surface-less UPDATE B for the vblank, so window 0's UPDATE joined B instead of waiting for C. Replay
    test `the_overlay_enable_sequence_pairs_in_arrival_order` (arrival order + (b) fix: C pairs and latches at the vblank) and
    `batched_in_channel_number_order_the_same_writes_leave_the_overlay_flip_unpaired` (the old worker order: C unpaired).
  - Fix 72784ba7: (1) `PutLog` (kf-disp ports): vCPU PUT writes are logged lock-free in arrival order and the worker steps the engine in that order (the latest-PUT pass
    remains only to resynchronise after a log overflow); (2) a window UPDATE that scans no surface before and names none after is not a flip: no vblank wait.
    FALSIFIER (written before the run): with 72784ba7, zero flags, runs 417-419 (overlay active) must show no `display: STALL Window 4` and the Edge phase TDR count 0;
    one stall in three falsifies it (then the pairing rule (2) is wrong or incomplete, and the dump of that run says what the guest waits for).
* 3. [measured, run 414, binary 18da7c3c, `KF3_BAR0_READ_TRACE=1` display ranges + `WIN_TRACE=1`, runner `tdr-run24.sh` = tdr-run23 + `OVLDUMP`]
  with BAR0 READS trapped and traced the overlay path WORKS: window 4 (1295x985 RGB, Edge plane 1) is in use from +207 s, 28261 window-4 PUT
  writes, 19534 window-0, 6957 imm-4, 0 TDR in all phases (boot/sign-in/Edge/Shorts/hold 92 s), 0 STALL reports. In runs 408/409/411/413 (no read
  trace) the stall H1 occurred every time (4/4). So H1 is a TIMING-dependent race, not a missing capability: the read exits slow the guest enough to hide it.
* 2. [measured, VFIO reference `boot3` (real RTX 4070 + Windows to desktop, traced with the same tracer), 3949 window-0 PUT writes] H-GET is FALSIFIED
  for the flip path: the driver reads window GET (`0x690004`) only 115 times, in bursts at the ring wrap (values 0xf30..0xfe0 then 0x0; flow-control room
  checks), never once per UPDATE. Per flip the driver does (window 0, one flip per vblank): read ARMED `+0x22c`, PUT x3, read ARMED `+0x2ec`, PUT of the
  window-imm channel, read core `0x680220`, PUT of the window (the UPDATE). Real values: ARMED 0x22c = 0xcf, 0x2ec = 0xa0, core 0x680220 = varying
  0xb8..0x100. kayfabe serves 0 for all three (it mirrors only the CORE's ARMED half; the windows' ARMED halves and core 0x220 read 0). [inferred] harmless for
  window 0 (works); not excluded for plane 1. The same per-window sequence is seen for window 4 in run 414 (read `0x694a2c`, PUT x3, read `0x694aec`, imm-4 PUT,
  read core `0x680220`, PUT), with the imm UPDATE BEFORE the window UPDATE.
* 1. [measured, runs 413/414, probe BEFORE any TDR (tdr_cycles=0)] kf_overlayprobe: `CheckOverlaySupport` NV12/YUY2/P010 = hr 0, flags 0x2 (SCALING only), no
  DIRECT (0x1), exit 5; with `--ignore-support` the swap chain / DirectComposition setup fails (exit 5). So the missing DIRECT flag is NOT a post-TDR effect
  (run 411's conclusion stands for the pre-TDR state too). Whether native Windows reports DIRECT: see `traces/windows_overlay_native_20261010/README.md` when it lands.
  (Run 412 never ran: its queue entry was replaced by 413.) Stall shape in 413 identical to 411 (window-4 UPDATE 0x1000 naming {chn 1, chn 37}, then core UPDATE and
  cursor updates keep flowing, window 0 flips cease): [measured] the display thread is alive; the FLIP-SUBMISSION thread is blocked between the kicks.

# HANDOFF (2026-10-10 ~22:20 CEST; a fresh agent resumes from here; sections "Progress 1-4" below are the evidence)

STATUS: LIVE. Product code on the branch, head = see `git log` (18da7c3c + docs). Labels: [measured] / [inferred].

## H0. What is fixed, what is not
* FIXED, hardware-verified (run 400-412): a console-only refusal never stops the guest display (`Fault::Console` vs `Fault::Gpu`); console YUV
  overlay composition (NV12-class formats derived from the class names, plane 1 following luma when unprogrammed, BT.709 limited, nearest scale,
  depth order incl. the SDR colour path); correct colours seen by the owner (run 403). P2 items (colour pipeline of the YUV window, clip, 10-bit): not done.
* PARTLY FIXED, NOT VALIDATED: the overlay (window 4 / MPO) flip-completion deadlock (guest flip queue 547 after a present whose plane 0 completes and
  plane 1 never does). Engine fix 17eb8b2f (an UPDATE naming a channel parked for its vblank joins that latch) is right but INSUFFICIENT: run 408/409/411
  (all with it) still stall in a different order, see H1.
* OPEN, P0: the stall of H1. No hardware run with the fix has yet had a clean overlay phase (run 410: 0 TDR but short; 400/401/403 had overlay, 385/402/404 none).

## H1. The stall that remains [measured: runs 408, 409, 411; STALL reports + ring + per-channel state + effects ring]
Sequence in the engine ring (run 411, seq 2617-2635; same shape in 408 and 409): core UPDATE; core UPDATE naming chn 5; window 0 UPDATE naming window 4;
window 4 UPDATE naming the core (group {core, window 0, window 4} latches); window 4 UPDATE naming nothing + window-0-imm UPDATE; window-imm-4 UPDATE
naming window 4 -> latch {5, 37}; THEN window 4: `SET_WINDOW_INTERLOCK_FLAGS` = 1 (names window 0) + `UPDATE` 0x1000 (interlock with its immediate channel):
parked forever waiting for {chn 1, chn 37}. Per-channel state at the stall (run 409): chn 1 PUT == decoded (nothing new kicked), chn 37 PUT == decoded
(its last UPDATE 0x2 is the OLD one), chn 5 GET stands before its UPDATE (published GET != PUT). So the guest has kicked window 4 and has NOT kicked the two
channels the UPDATE names. ETW (408/409): the stuck present (id 510) handed plane 0 (completes) and plane 1 = Edge's RGB MPO plane (1295x986 at 22,13,
`Opaque`), first appearance of plane 1; the guest flip queue declares the TDR 2.25 s later. No YUV involved (the video was not yet playing).
nvkms (`nvkms-evo3.c` nvEvoUpdateC3) pushes the CORE first and the windows after it, so hardware waits for named channels that have not yet issued their UPDATE
(R0); the Windows driver therefore must kick the partners; it does not.
Hypotheses (falsifier in brackets):
* H-GET: the Windows driver waits for the window channel's GET (or idle status) to pass its UPDATE before kicking the partner channels; real hardware advances
  GET past an accepted UPDATE, kayfabe holds GET before it until the latch (engine doc comment: "GET stands before the UPDATE"). [Falsifier: the working
  pattern (first-kicked channel names nothing, later ones name it) kicks partners while the first channel's GET still stands before its UPDATE; so H-GET
  needs a driver path that differs for the stuck update. Test: publish GET past an accepted UPDATE (engine + `Item::Get` path), rerun Linux broker lane +
  fast suite (NVKMS idle checks!) and the overlay run; or read the pusher thread's stack in the WATCHDOG dump: `dumpcfg/plugins/windows/waitunwind.py`,
  `tdrctx.py` (host-only; diagnosis only): is it spinning on a read of the channel's user area?]
* H-WAIT-NOTIFIER: the driver waits for plane 1's previous-flip notifier (FINISHED) before kicking the partners; kayfabe writes FINISHED at flip-away
  (the stuck update). The effects ring (18da7c3c) lists the notifier writes: compare with plane 1's notifier words. [Falsifier: a FINISHED write for the
  previous plane-1 flip exists before the stall.]
* H-VCPU: the guest pusher is blocked in a trapped BAR0 write. [Falsifier: dump stack of the thread shows no kayfabe-trapped access.]
Do NOT add timeouts that release the stuck update, nor a window-count cap (owner rulings; the owner wants MPO supported, MPO-off is diagnostic only).

## H2. Runs (hold in seconds; TDR per phase boot/sign-in/Edge/Shorts-load/hold)
| run | binary | note | TDR | outcome |
|---|---|---|---|---|
| 400 | f34d8937 | isolation fix only | 0/0/0/0/4 (hold 624-844 s) | overlay up, console video black (no YUV), flip-queue TDRs on plane 1 |
| 401 | da6be258 | + console YUV | 0/0/0/0/2 (hold 732 s) | overlay drawn, magenta/green (chroma plane at offset 0) |
| 402 | cc378992 | chroma follows luma | 0/0/2/1/- | TDRs in Edge phase, then NO overlay; video correct via DWM; bars clip correct |
| 403 | 91b2890a | + STALL report | 0/0/0/0/2 (hold 72 s) | owner: colours correct; overlay on top of a PowerShell window (z-order, P2) |
| 404 | 91b2890a | OverlayTestMode=5 + dwm restart (DIAGNOSTIC) | 0/0/0/0/0 (900 s hold, Shorts scrolling) | MPO off = clean |
| 408 | 17eb8b2f | + join fix | 0/0/2/1/.. | stalls remain (H1) |
| 409 | 7d766cbd | per-channel state | 0/0/2/.. | H1 evidence |
| 410 | 18da7c3c | + effects ring | 0/0/0/0 (120 s hold) | no stall that time (race) |
| 411 | 18da7c3c | probe after Edge | 0/0/2/1 | probe: exit 5 (no overlay support after the TDRs) |
| 412 | 18da7c3c | probe BEFORE Edge (runner tdr-run22.sh) | see below | |
Host tooling added (host-only, `tdrhunt/`, copies of the idea in this README): `tdr-run17..22.sh` (guest-side playback check `kfplay.ps1`, bars clip `bars.mp4`,
staging over QGA `stage()`, interactive-session tasks `usertask()`, `DWM_OVERLAY_OFF=1`, `OCCLUDE=1`, `PROBE_SCNS="steady occlude recreate"` with
`kf_overlayprobe.exe` built by `scripts/bench/windows/appmatrix/build_tools.sh`). TRAP: never edit a runner script while a run uses it (bash reads it
incrementally; run 400 died that way): copy to a new name. The probe staged and ran on the first try (exit codes work).



STATUS: LIVE, 2026-10-10. Progress log; newest HANDOFF at the top when I stop.

## Progress
* 1. Code finding [measured, code]: in `display.rs` `ScanState::start`, when `dp.sdr_color` is set (Windows SDR colour pipeline) any entry in
  `planned.refused` returned `Err("colour frame has refused windows")`; the caller set `self.failed = true`; the display loop's step 6
  does `if scan.failed { queue.clear(); engine.halt_scanout(); }` and `halt_scanout` halts every guest display channel for the VM's life.
  The non-colour path always left the refused window out and carried on.
* Fix: `Fault::Console` vs `Fault::Gpu` (`ScanState::fault`). Console-only refusals (window with no console format; colour program the
  console cannot build) leave the window out / skip the copy, complete the flips behind the copy (`finish`), never set `failed`, and are
  counted (`console_windows_left_out`, `console_copies_skipped` in the device status line, first 16 refusals in the log). GPU failures keep
  `failed` (no forged completion for GPU work). YUV composition for the console was not added: it needs a new CUDA kernel + PTX
  (planar storage), not simple. Test `a_console_only_refusal_completes_the_flip_and_never_halts_the_display`.
  Note: the test exercises the new `fault` policy; the old inline code had no testable seam (it needs the GPU), so "fails on old code" is by construction (old behaviour = `Fault::Gpu` for the refused case, asserted in the test as the stopping branch).

## Progress 2 (YUV console composition; run 400 = f34d8937 with only the isolation fix)
* [measured, run 400] 0 TDR through boot/sign-in/Edge/Shorts-load (first ~6 min of the hold also 0 at the time of writing); 16x
  `scanout REFUSED window 4 ... FORMAT 0x38` and the owner sees the video area BLACK on the console while the rest of the page is fine.
  So the isolation fix works (flips continue), and the black area is the deterministic console-copy gap, not a race.
* [measured, ogkm clc67e.h] FORMAT 0x38 = `Y8___V8U8_N420` (semi-planar 4:2:0, luma plane then interleaved V,U; NV21). Siblings
  `Y8___U8V8_N422` (0x36), `Y8___U8V8_N444` (0x35). SET_PARAMS.SWAP_UV flips the chroma order. Planes: SET_CONTEXT_DMA_ISO(b),
  SET_OFFSET(b), SET_PLANAR_STORAGE(b), b = 0 luma, 1 chroma.
* [measured, harness] the runners' `PLAYBACK frames differ` uses QMP `screendump device kf0` = kayfabe's CONSOLE copy, which could not contain the
  YUV overlay: the check was blind to the video by construction (explains the false 'identical' verdicts; 385 showed video because
  it simply had no refusal: [inferred] the overlay plane was not in use or composed otherwise there; run 385 has 0 `REFUSED`).
* Implemented (commit f72b0d34, da6be258): `ScanFormats.yuv` (derived from the class names), `Scanout.{iso1,offset1,pitch1,swap_uv}`,
  `scanout::plan_yuv` (each plane bounded against its own context DMA; destination clipped), `cuda/display/kf_yuv.cu` + generated PTX
  (own module; `make_yuv_ptx.sh`), `DisplayGpu::compose_yuv` (bounds re-derived), composed after the RGB layers and before the cursor.
  BT.709 limited range, 8.8 fixed point, nearest-neighbour scale, opaque. NOT done (stated): the window's own CSC00/ILUT are ignored for the
  YUV window; no 10/12-bit or 3-plane formats (still left out by name, console-only); depth ordering (YUV after RGB).
  Tests: `tests/yuv_kernel.rs` (the kernel BODY compiled for the host, compared byte-for-byte with `yuv_reference` over pitch/BL, 420/422/444,
  both orders, scale up/down, frame-edge clip), NV12 known answers, `plan_yuv` bounds incl. hostile planes, `YuvLayer::check`.
* Guest-side check added to the runner (tdr-run17.sh, host-only): `kfplay.ps1` runs in the interactive session (scheduled task /it),
  5 captures of the player column 3 s apart with pixel-diff counts, GPU Engine utilisation of msedge by engine type (VideoDecode = NVDEC),
  full PNGs left in C:\kf. Run 400's first attempt failed (`schtasks /run`: Element not found); run 401 discovers the interactive user.

## Progress 3 (runs 400-402; measured vs inferred)
* [measured, run 401 da6be258] the console now DRAWS the overlay (owner photo + `shorts-playing-2.png`): window 4 armed state logged:
  448x796 block-linear (7 GOBs, bh 1), format 0x38, SWAP_UV=1 (so the effective chroma order is U,V = NV12), out (826,180) 448x795,
  depth 11, ISO(1)=OFFSET(1)=0 (never programmed), PLANAR_STORAGE(1)=7. The picture was luma-correct but magenta/green: [measured] the
  luma tile structure is right and a 2x-stretched copy of the luma image's top half appears in the chroma channels, i.e. the kernel read
  the chroma plane from the luma plane's first byte (offset1 = 0). Fix cc378992: an unprogrammed chroma plane follows the luma surface
  (offset + whole-surface luma size, 256-aligned; 358 400 bytes here). It is NOT a U/V-order or matrix bug; the order logic (format order
  XOR SWAP_UV) is unchanged. [unverified on hardware until an overlay run with cc378992+: run 402 had no overlay.]
* [measured] ETW from the WATCHDOG 0x117 of run 400 (cluster at hold_t 624): the guest's flip queue (event 547, pid 4) declared the TDR
  at 17:51:50.159; the last present (id 10097) handed plane 0 and plane 1 (YCBCR_STUDIO_G22_LEFT_P709, 448x796 at 826,180) at
  17:51:48.050; plane 0 completed (505) at .067, plane 1 NEVER completed; no render/video packet was open (178 without 180: none) and
  no host Xid. So it is the same class as 264-266 (a flip handed and never reported), here on the overlay plane. The 0x117 dump's
  param2 is an nvlddmkm address. Run 401: same shape at hold_t 732 (its ETW window ends earlier).
* [measured] overlay presence correlates with the Edge phase: runs 400/401 (overlay active, console black/magenta before the fixes) had 0 TDR
  through Shorts-load and a TDR cluster 10-12 min into the hold; runs 385 and 402 had 2-3 TDRs in the Edge phase, then NO overlay
  window (0 YUV/REFUSED lines), correct video on the console from DWM composition. [inferred, H-MPO] a TDR makes Windows drop MPO
  for the session; not proven (the registry/ETW MPO state was not read).
* [measured] harness: `PLAYBACK frames differ` was blind by construction (console copy without the overlay). The in-guest check
  (`kfplay.ps1`) reports GPU Engine use by msedge: run 401 `videodecode sum=4.74 max=1.25` = NVDEC ran for Shorts (hardware decode IS in use).
  Its crop-diff output was empty in 400/401 (PowerShell alias `diff` shadowed the function; fixed) and 0/15000 in 402 (the clip was paused:
  the Shorts play icon is visible in `shorts-playing-2.png`; [inferred] the scripted PLAY click toggled it).
* Colour bars: `tdrhunt/bars.mp4` (10 stripes, BT.709 limited, exact RGB), staged in the guest, opened in Edge via an /it task: run 402 shows
  the bars correctly from DWM composition (no overlay), so it did not exercise the YUV kernel.
* Added a bounded STALL report (91b2890a): an UPDATE parked > 1 s is named once (stage, group, head, acquire and the semaphore value it
  reads), to see which wait the overlay flip is stuck on in the next overlay run.

## Progress 4 (the overlay flip that never completes: root cause and engine fix; measured vs inferred)
* [measured, ETW of runs 400 and 403] both TDR declarations (flip-queue 547) follow a present whose PLANE 0 completed (505) and whose PLANE 1
  (the video overlay, window 4) never did; run 403: plane 1 had been flipped to NO surface at presents 5212-5214 and was RE-ENABLED
  at present 5215 (run 400: the same last-present shape). No render/video packet was open (178 without 180: none), no Xid.
* [measured, run 403, STALL report 7750480b] chn 5 (window 4) update 0x1000 parked 1016 ms waiting for its interlock group
  `{chn 1, chn 37}`; chn 37 (window-immediate 4) update 0x2 waiting for `{chn 5}`; chn 1 (window 0) NOT parked at an UPDATE; no acquire.
  An earlier line: chn 1 and chn 5 both ready and waiting for head 0's vblank.
* [inferred from those two, consistent with the 21 us between the two 259 events of present 5215] the guest pushes window 0's UPDATE first
  (it names nothing) and window 4's (names window 0 and its immediate channel) microseconds later. The engine parked window 0's update for
  the vblank (Stage::Latch); window 4's update then waited for an UPDATE on window 0 that never comes (the driver waits for plane 1's completion
  before sending more), while window 0 latched alone: plane 0 completed, plane 1 never. nvkms (`nvkms-evo3.c:2829-2905`) confirms the
  hardware rule that an interlocked UPDATE waits for the named channels' updates and latches with them; a parked update is still PENDING until
  its latch.
* Fix 17eb8b2f (`engine.rs` `ready_group`/`group_ready`): a channel parked for its vblank or acquire counts as pending for the closure; an UPDATE
  naming it joins its latch group (with the channels it latches with). Test
  `an_update_naming_a_channel_already_parked_for_the_vblank_joins_its_latch`: FAILS on the old code (only window 0 latches at the vblank,
  window 4 and its immediate channel stay waiting), passes now; the run-293 one-sided tests and the other 118 engine/disp tests pass.
* Also pushed: an always-on ring of recent interlock events and a STALL report with queue/copy/tick context (bounded, loud, read-only).
* Hardware validation of 17eb8b2f: run 408 (queued; 20-minute hold).
* MPO decision (owner question): [measured] nothing in our logs or the open sources names a derived capability the guest reads to turn the overlay
  off: the guest asks nothing we log for the window assignment; the family row's `windows` count and the caps page's `SYS_CAPB_WINDOW_EXISTS`
  bits are the real die's (derived); Windows' MPO decision lives in the closed driver. Therefore no knob is proposed: the product answer
  is the flip-completion fix. `OverlayTestMode=5` stays a guest-side diagnostic (run 404).
