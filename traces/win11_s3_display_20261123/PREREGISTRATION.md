# Win11 S3 wake on kf3: the display window-channel stop, hypotheses and falsifiers written before the run (2026-11-23)

**STATUS: RESEARCH, 2026-11-23. Written and committed BEFORE any hardware run of this directory.** It is the addendum to
`traces/win11_s3_tdr_20261123/PREREGISTRATION.md` for that run's open second cause. Results go in `README.md` beside it;
this file is not edited after the first run except by a dated addendum at the end.

Baseline (measured 2026-11-23, Vast box 62141661, RTX 4090, kf3 `0c16cb72` = F2, runs T1 and T1b,
`traces/win11_s3_tdr_20261123/README.md`): about 12 s after the wake kf3's display engine stops window channel 1 with
`PUT 0x1230 / GET 0xf20` (T1) and `PUT 0x1130 / GET 0xe20` (T1b) "outside the 4096-byte pushbuffer"; the core channel then
parks on that window's interlock and the Windows TDR (event 153) follows 68-70 s after the wake.

## Phase 1, GPU-free (read, not executed on hardware)

Read 2026-11-23 from the T1/T1b logs of that directory, the kf3 sources at `origin/next` `d18026d7` and `third_party/ogkm-580`
(580.159.04, `b81d58e`).

- [read, T1 log `t1_c1_wake_to_unload.txt.gz` line 2037] after the stop the guest kept writing window 1's PUT: `chn 1:
  PUT 0x2130`, engine `(decoded 0xf20, get 0xf20, halted)`. The ring words at the stop (`ring words from decoded-16`) are
  a `NONINC_METHOD` header `0x430002ac` at 0xf20: 192 data words to `NVC67E_SET_INPUT_SCALER_COEFF_VALUE` (0x2ac,
  `clc67e.h:199`), i.e. a 0x304-byte group. In both runs PUT - GET = 0x310 exactly: the guest's own put stood at the
  GET kf3 kept across the wake and it wrote one batch of 0x310 bytes contiguously past the end of the 4 KiB ring, with no
  `JUMP`. [inferred] the guest did NOT restart its channel at 0 at the resume: it continued from the kept GET, as if its
  channel's state survived; and its room check let a 0x310-byte batch through with 0x1e0 (T1b) or 0xe0 (T1) bytes left.
- [read] The ring really is 4 KiB: the guest's `INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER` gave `limit 4095`, `channelPBSize 0`
  for every window (`t1b_kf3_keys.txt`), the class's `DMA_JUMP_OFFSET` is 11:2 and its `PUT_PTR`/`GET_PTR` 9:0
  (`clc67e.h:45-52`), and the five window pushbuffers are consecutive 4 KiB pages (phys 0x279783000 + 4096 * n in T1b), so
  bytes past 0x1000 of window 1 are window 2's ring.
- [read] What kf3 does to the display at a kept wake (`kf-qemu` `device.rs`): `reset_drainer_state(wake)` keeps the life
  (GSP FSM E14) and returns `Life::Kept`; `begin_release` (the display worker's life reset) is skipped for a kept life
  (`reset.rs`), so the engine (each channel's decoded offset and GET), the ports (`ChanPort` put/get/generation) and the
  model are KEPT. But `finish` then runs `restore_shadow()` for both lives, and it rewrites EVERY BAR0 shadow piece to its
  realize image (`piece_image`: zeros plus boot registers), the display aperture included. The guest reads display GET,
  PUT, CHNCTL/CHNSTATUS and the core's ARMED half from the shadow without an exit. After it, only the display's static
  words (caps page, cursor `Free`) and the event registers are republished. The worker republishes a channel's GET only
  when it moves (`published_get[chn] == get && ports.get(chn) == get` -> skip, `display.rs`), so [inferred from the code]
  after every kept wake the guest reads **GET = 0** on every display channel whose engine GET is not 0 (window 1: engine
  0xe20, guest reads 0), until the engine next moves that channel's GET, which needs a PUT it can decode.
- [read, ogkm-580] CPU-RM's display state load/unload (`kern_disp.c:661-692`, `disp_inst_mem.c:417-460`) only (un)maps
  instance memory and the shared page; it does not touch channels. Channel hardware state is physical RM's (here kf3's GSP
  model; on silicon GSP-RM, closed source), so the open sources cannot say whether silicon retains or restores GET/PUT
  across D3cold. [inferred] since the Windows driver neither frees nor re-allocates its display channels across the
  suspend (no `ChannelAllocated` between the wake and the TDR, T1b) and continues writing at the kept GET, it relies on
  the channel's GET/PUT state being there after the resume (retained or restored by GSP-RM). Either way the GET a
  resumed guest reads must be the GET the engine executes from; kf3 shows 0 while it executes from 0xe20.
- [read, `nvkms-dma.c:184-230`, the open driver's analogue] a ring writer whose put is past GET and whose batch does not fit
  before the end only wraps (JUMP to 0) once GET is not 0, and otherwise waits. [inferred] a GET word stuck at 0 is the
  one value that blocks such a writer's wrap; a writer that gives up waiting (a timeout) and writes on would overrun the
  ring exactly as measured. Windows' own room check is closed source: this is a plausible mechanism, not a read one.

## Hypotheses (ranked), each with its falsifier

- **H5 (the kept wake's shadow restore zeroes the display channels' GET words while the engine keeps their GET).**
  Fix candidate **F3** (this branch, commit `22ddc9e7`): `restore_shadow(life)` leaves every display-aperture word as it
  is for a kept life (`wake_keeps`); an ended life is unchanged. Instrumentation (commit `544472a7`, in both builds below):
  at the kept wake one line `display channels (chn, engine PUT, engine GET, GET word before -> after the shadow restore)`,
  then each DMA channel's first PUT after the wake with the engine's decoded offset, the published GET and the GET word
  the guest reads.
  *Predictions:* (a) control build (instrumentation only): the wake line shows window 1 (chn 1) with engine GET != 0 and
  GET word `before` = engine GET, `after` = 0; its first post-wake PUT line shows `GET word the guest reads 0x0` while the
  published GET is not 0; then the stop as in T1/T1b. (b) F3 build: `after` = `before` for every channel, every first
  post-wake PUT line reads a GET word equal to the published GET, no `STOPPED`, no `STALL`, and no event 153 in 3
  consecutive cycles in one boot (CLEAN, definition below).
  *Falsifiers:* (i) in the control build the GET word `after` the restore equals the engine GET (the restore does not
  zero it: H5's premise is wrong); (ii) with F3, a display channel still STOPS past the pushbuffer end while its first
  post-wake PUT line read a correct GET word (the guest overruns whatever GET it reads: then H6); (iii) with F3, no stop
  but event 153 still follows a wake (another cause: H7).
- **H6 (the guest's ring-size or room arithmetic after the resume is wrong regardless of GET).** *Prediction:* F3 still
  stops window 1 at a PUT past 0x1000 with a correct GET word. *Falsifier:* F3 clean (no stop) for 3 cycles.
- **H7 (other display state lost by the restore: CHNCTL/CHNSTATUS or the core ARMED half read as realize values).**
  F3 keeps these too, so F3 cannot separate H7 from H5; *a separate falsifier is not run* (no ablation build).
  [inferred] H7 alone does not explain a PUT that runs past the ring end.
- **H8 (the guest restarts its channel at the resume: PUT from 0).** Already contradicted by T1/T1b (PUT = kept GET +
  0x310). *Falsifier on hardware:* a first post-wake PUT on window 1 below its kept GET without a JUMP-wrap.

## Definition of a CLEAN cycle (unchanged from `traces/win11_s3_tdr_20261123/PREREGISTRATION.md`)

QMP `query-status` reaches `suspended` within 120 s of the suspend request (a scheduled task in the signed-in session);
after `system_wakeup` the guest's ssh answers within 150 s with the SAME `LastBootUpTime`; kf3's log gains `(S3 WAKE) ...
KEPT` and at least one `GSP-RESUME`, and no `device reset #N: vCPUs out`; `nvidia-smi` names the GPU; and after waiting
until at least 120 s past the wake, the guest's System log for the window since this cycle's suspend request holds no
event 41, 1001 or 153. The release-gate reading of `windows-sleep-wake` (docs/RELEASE_CLAIMS.md) is N >= 3 consecutive
CLEAN cycles in one boot. A display `STOPPED` is reported per cycle but is not part of the verdict.

## Run plan and stop rule (time-box: 3 box-hours, 5 USD, ONE box; caps `kf-s3wake-` in vast_caps.json)

1. One 24 GB RTX 4090-class box (`gpu_ram` about 24564), `vms_enabled=true`, KVM image, global search by total price;
   `df -h /`; `vast_guard.py --enforce` before and after; host clocksource recorded; if the L1 TSC is unstable, reboot with
   `tsc=reliable clocksource=tsc` (as in T0/T1/T1b).
2. `windows_lane.sh` then `windows_gate.sh` (arm B, `KF_WINGATE_IDLE_S=120`) at this branch's F3 revision. No S3 claim on
   a red gate.
3. **D1 (F3):** arm B, q35, S3 enabled, 3 cycles in one boot (`s3disp.py`, `--cycles 3`). Judges H5 (b), H6, H7.
4. **D0 (control, only if box time and money remain):** the same box and gate overlay, 1 cycle with the
   instrumentation-only build `544472a7` (built by `build_kf3.sh` in its own worktree and target dir). Judges H5 (a)
   and falsifier (i).
5. Stop, compare, write the README. Destroy the box by its own id and verify its absence.

## Addendum 1, 2026-11-23 (after D1 cycle 1, before any further run)

D1 (F3 `a2df9a81`, box 62182559) cycle 1 was judged FAIL by the probe for `event 153`, but the only 153 record is
`Microsoft-Windows-Kernel-Boot` (a boot-time record), dated 22:43 on the guest's clock before the clock step that the
resume logs (`Kernel-General` 1 at 15:01:28): the boot's records fall "after" the window start. No nvlddmkm record, no
display `STOPPED`/`STALL`, no RM unload (`fn 47 ... newLevel=0`: 0). The probe's 153 rule therefore misjudges. From here
on (`s3disp.py`, this commit) an event 153 counts only when its provider is `nvlddmkm` (the TDR the gate is about);
the other 153 records are kept in each cycle's result (`event_153_other`). Everything else in the CLEAN definition is
unchanged. D1 cycle 1 is reported as probe-FAIL and hand-judged against this rule, and does not count toward N >= 3.
**D2:** a fresh boot of the same gate overlay, F3 build, 3 cycles, with the corrected rule. Then D0 as planned.
