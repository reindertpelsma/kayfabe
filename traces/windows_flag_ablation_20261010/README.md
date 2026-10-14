# Windows flag-necessity ablation (2026-10-10/11)

**STATUS: LIVE, 2026-10-11 (PARTIAL: lane paused by the coordinator after run 605; 6 of 21 planned runs done; the
rest is queued in `ablate/todo.txt` on the host and resumes on the coordinator's word, after a rebase onto the
channel-twin-cap fix).**

Question (owner): which of the 20 behaviour flags of the Windows production profile does Windows really need to
boot and run, so everything else can be dropped. Each run is the production profile MINUS the named flag(s).

## Method

* Binary `kf3-bins/89721ca4` (playback head = production code), runner `host_tools/tdr-run27.sh` (a copy of
  `tdr-run26.sh`: scripted sign-in, `DWM_OVERLAY_OFF=1`, Edge opened by scripted double-click), launcher
  `host_tools/windows_broker_prod3.sh` (a copy of `windows_broker_prod2.sh` plus `WIN_DROP="KF3_A KF3_B"`: the
  listed flags are not set at all; the runner passes `ABL_DROP` as `WIN_DROP`). No Shorts (`SKIP_SHORTS=1`),
  `HOLD_SECS=180`, zero measurement flags. Driver `host_tools/abl-batch.sh` (pops `NAME|flags` from a todo file,
  queued behind other GPU users), per-run summary `host_tools/abl-sum.sh`. Host runs are 600-605
  (`winprod/runN`, `boundary-kayfabe-N` under `/var/lib/kf-windows-20261005`); the evidence is in `runs/N/`
  (`winprod.log`, `ablate.txt` = QGA: NVIDIA adapter `ConfigManagerErrorCode`, msedge/explorer/dwm, System-log
  nvlddmkm 153 and dxgkrnl 141/4101 counts; `guest-events.txt`; `command.json` = the flags actually set) and in
  `results.txt` (the summariser output, also the refused-GSP-control set diffed against control 600).
* Per run recorded: reaches desktop (explorer in session 1, `after-signin` screen), adapter Code, TDR cycles
  (`GSP phase Running -> Suspending` in qemu.log, sampled at end of hold; the final value is +1 for the shutdown) and
  guest events, host Xid delta, `GSP REFUSED` set versus control, `DEAD:` / panics / `display: STALL` / fail-closed,
  Edge up (msedge processes), screens (unique colours: a dead display is ~80, a desktop ~60k-100k).
* **Falsifier per flag, written before the run:** "dropping X leaves boot + desktop + Edge fine for 180 s". A run
  that is clean falsifies "X is needed for this lane"; a run with adapter Code 43 / no explorer confirms it.

## Baseline noise (full profile, 20 flags)

| run | result |
|---|---|
| 600 control1 | desktop, adapter Code 0, Edge 8 procs, 0 TDR events, 0 Xid, 62 distinct refused GSP controls |
| 601 control2 | same; the end-of-run QGA evidence call timed out once (30 s socket handshake), `guest-events.txt` (read 1 s earlier) is clean, screens normal |

Both clean. **Limits of the lane (read before trusting any "NOT NEEDED"):** one sample per flag; 180 s of an idle
desktop plus Edge's start page. The production TDR (run 500: 3 cycles in 580 s, first ~2 min into Shorts playback)
needs playback load; this lane does not apply it, so a flag whose only role is in TDR recovery or video/3D load
shows as NOT NEEDED here. NOT NEEDED below therefore means "not needed to boot, reach the desktop and open Edge
for 3 minutes". Coordinator note: an idle Windows desktop already holds ~64 live channel twins and the 65th birth is
refused (hardcoded cap). A clean run is not affected by the cap. For the one failing run (604, TIMER_MAP) the failure is at driver start (adapter Code 43
with a smaller GSP-refused set, 45 vs 62 distinct controls: the driver stopped early), long before a desktop's ~64 twins exist;
the twin-cap refusal text was not looked up in qemu.log (not located by grep), so the statement that the cap played no part in run 604 is an
inference from timing, not a measurement. Every later failing run must be checked for it before it counts as a verdict.

## Results so far

| flag | ablation run | result | verdict | evidence / perf note | next step |
|---|---|---|---|---|---|
| KF3_DISPLAY_SDR_COLOR + KF3_DISPLAY_LUT_MIRROR + KF3_DISPLAY_ILUT_OFFSET_256 (tied; the two are dead code without SDR_COLOR) | 602 (flags_set=17) | clean: desktop, Code 0, Edge, 0 TDR, 0 Xid, refused set identical to control | **NOT NEEDED** for boot/desktop/Edge | Without SDR_COLOR the authored caps page is used (replaced by the captured page anyway under CAPS_PROBE) and no colour program is decoded: the guest's LUT/matrix/TMO programs are not applied (colour-fidelity feature, not a liveness one). Control 600 shows exactly one mirrored-LUT colour program being accepted, so the guest does send one. Perf: saves the per-frame GPU colour kernels (not measured here). | Drop from the production profile (colour correctness is the owner's call: if wanted, it stays as the product feature and the pair is hardwired with it; then the profile keeps all three). A run with a long hold plus a colour-heavy scene is the open check. |
| KF3_ASYNC_PREEMPT | 603 | clean; one extra refusal vs control: `fn76/0x2080110b=0x1f` (FIFO_DISABLE_CHANNELS async refused) | **NOT NEEDED** in this lane; AMBIGUOUS for load | Dropping turns a served guest call into a refusal (a real fidelity loss); the earlier TDR-at-TdrDelay (runs 78/79) was later attributed elsewhere (inventory), but the TDR recovery path only appears under playback load. Runs 100-103 serve 2 async preempts, 183-383 us each. | Keep until a playback-load ablation (production run with Shorts, drop only this flag) is clean; then hardwire (the correct behaviour is to serve it) rather than drop. |
| KF3_TIMER_MAP | 604 | **FAIL: adapter Code 43**, no explorer/Edge, display dead (80 unique colours), 1 GSP cycle at boot, refused set smaller (45 vs 62: the driver gave up early) | **NEEDED** (measured, run 604) | Falsifier met (clean desktop) is NOT met: adapter at Code 43. Previously "unknown/inferred". | Hardwire per inventory: on whenever `open_timer` succeeds (host `TIMER_GET_REGISTER_OFFSET`, layout match from the compiled matrix). |
| KF3_TRANSLATED_CE_RELAY | 605 | clean, refused set identical to control | **NOT NEEDED** in this lane; AMBIGUOUS | Inventory: CE2 (paging channel's engine) reaches the guest only through this relay under §X (run 102); the paging path may only matter under memory pressure / playback load. | Not droppable yet: repeat under the playback-load profile (Shorts) before deciding; if still clean, the relay is dead for Windows and the flag goes. |
| controls (full profile) | 600, 601 | clean | baseline | see above | n/a |

## Not yet run (queued in `ablate/todo.txt`, order as written)

SW_RUNLIST_PROBE, MEMORY_LIST_PROBE, ZCULL_BIND_PROBE, GFX_POOL_PROBE, KERNEL_GR_WORK (unknown or inferred ones);
CTRL_PROBE+IMP_ENABLE (pair, then each alone), then the necessity controls DEFERRED_API, DISPLAY_CAPS_PROBE,
KERNEL_GR_CE, and PREEMPT_BIND_PROBE, SW_RUNLIST_HOST_OWNED, WIN_TWIN_DEFAPI_OBJECT,
WIN_USER_CHANNELS_PASSTHROUGH, LUT_MIRROR+ILUT_OFFSET_256 (with SDR_COLOR kept). For these the inventory's
earlier measured/unknown verdict stands until their run lands. The run TIMER_MAP shows the lane detects necessity.

## Code hygiene fixes (branch `claude/flag-ablation-20261010`, separate commits, `cargo test -p kf-rm -p kf-host` green, `cargo check -p kf-qemu` green)

* `efdb5c84` kf-rm: `KF3_GFX_POOL_PROBE` logged one uncapped `eprintln!` per query; now the first 16 then powers of two, with a test.
* `7f7f7f17` kf-host: `KF3_WIN_USER_CHANNELS_PASSTHROUGH` was read with `std::env::var` on every mirror-space alloc
  (VA thread); now a `OnceLock`.
* `2f85ba69` kf-qemu: `KF3_DEFERRED_API` logged two uncapped lines per trigger (trigger + DMA_INVALIDATE_TLB) and one
  per completion (`DONE`); all capped (first 16, powers of two) and numbered. (The `grep`s of old run logs for
  "DEFERRED-API trigger ... DONE" still work for the first 16.)
* Sweep of the 20 flags' code for other per-event env reads or logs: every other flag is read once (`OnceLock`, or at
  realize / policy-chain build: `lib.rs:487/570/583`, `display.rs:397/1785`, `device.rs:641`, `chan.rs:1018/1567/1577/1595/1605`,
  `chanlink.rs:248`, `display.rs:151`, `color_experiments()`). Per-event logs that remain are capped by the existing
  "first 16 then powers of two" pattern (`display_ctrl_probe` LOG_CAP 32, `fecstrace` 8). `KF3_ASYNC_PREEMPT_REENABLE_NOACT`
  (not one of the 20) is also a once-cached read.
  These fixes are NOT in the binary `89721ca4` that the ablation runs, so the ablation measures the flags, not the fixes.
