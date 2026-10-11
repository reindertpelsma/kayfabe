# Windows OpenGL / CUDA crash, 2026-10-11

**STATUS: LIVE, 2026-10-11.**

## HANDOFF (newest, 0x73011a answered; the crash is NOT fixed by it)
1. `[measured, runs 630 and 631, kf3 `2cedcc67`, production profile (`windows_broker_prod3.sh`), plain baseline, exFAT app disk attached after sign-in]` `0x73011a` is now answered `NV_OK`
   (run 631 `KF3_RPC_TRACE=1`: `RmControl cmd=0x0073011a client=0xc1d0007a object=0xff0a0000 result=0x0`, right after `0x73010c` head 0), cup2/cup3/cup8 still PASS,
   and `kf_glgears.exe` STILL dies: exit -1073741819, Application event 1000, `nvoglv64.dll` 32.0.15.8088, fault offset `0xb6d516` (the same offset as before), screenshots do not differ
   (AE 0). So the falsifier of the lead ("answer it and the crash goes away") **fired**: `0x73011a` refused was not the (only) cause. Evidence: `winprod/run630`, `winprod/run631` and
   `tdropus/glcrash-63{0,1}.log` on the host.
2. The gate admitted the answer (the guest says `guestDriverVersion="580.88"`, the device answers as driver `580.65.06`, the observed branch); no `layout unverified` line, counter 0.
3. **Next divergence, measured (run 631 trace vs the VFIO gears capture, command order only; raw bytes stay on the host):** kayfabe's gears client (0xc1d0007a) issues
   `20801315, 2080012f, 20801315, 2080012f, 730101 x2, 730102 x2, 73010c(h0), 73011a, 73010c x3, 730101`, then **Frees** (and the access violation). The real GPU's gears client
   issues, before the first `730102`, `20801315, 2080012f` x2, then `(730101, 20801315, 2080012f)` x3, then `730101` x5 (8 `730101` calls in all, kayfabe 2), then `730102 x2, 73010c, 73011a, 73010c x3,
   730101, 730102 x2` and repeats the `730102 x2, 73010c, 73011a, 73010c x3, 730101` cycle 5 times, then ZBC `0x9096010x` and the VA/channel allocs. So the ICD takes the exit right
   after the first cycle's final `0x730101` instead of going on to `0x730102 x2`. The three replies that differ between the machines and could steer it (inferred, none proven):
   `0x730101` capsTbl (real `81 2f`, model all-zero), `0x730102` with the CLIENT flag (real 1, model 4), and the number of `730101` calls before `730102` (real 8, kayfabe 2: the ICD
   already behaved differently BEFORE `73011a`, so a reply earlier in the sequence, most likely the `0x730101` table, is the first fork). Not changed: the headers define the
   bits (`NV0073_CTRL_SYSTEM_CAPS_*`) but not that a virtual display must claim `CROSS_BAR_SUPPORTED`/`SINGLE_HEAD_MST` etc. (design choice "no crossbar", `V3_DISPLAY.md` §4.14).
4. Suggested next experiment (needs an owner decision because it changes a design choice; batch it in ONE run, falsifier first): answer `0x730101` with the capability bits the
   model actually implements and the `0x730102` CLIENT flag from the heads whose raster runs; falsifier: the ICD's call sequence still stops after the first cycle. If the sequence
   then follows the real one, bisect which of the two.
5. Linux regression for the display-model change, `[measured, kf3 2cedcc67, host 172.22.1.20, 2026-10-11 03:21-03:38 CEST]`: fast suite **30/30 PASS**, 0 FAIL, 0 CRASH (`FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0`, one revision, one hold), 5 new host Xid (the five intentional negative-control Xid of `--defer-liveness`/`--missing-page-fault`, as in `traces/linux_regression_20261010`); broker smoke `input_proof.sh lr73b` PASS (GRUB key, Cinnamon up, ABS x8, REL under grab (70,30), ABS dropped under grab, keys, broker re-attach, evdev sums). The first broker attempt (`lr73`) failed only because `/workspace/bench/kf3-bins/2cedcc67` did not exist (harness looks there; a symlink fixed it, removed afterwards): not a product result.

## Layout, gate and tests of the change (code: `kf-disp`, branch `claude/gl-73011a-20261011`)
- `kf_disp::model::GET_DISPLAY_MODE` (`0x0073011a`), 28 bytes `[displayId, width, height, total_w, total_h, 0, refresh_hz]` (observed once, field meanings inferred),
  values from the head's ARMED core state (`Engine::head_timings`: visible raster = `BLANK_START - BLANK_END`, totals = `HEAD_SET_RASTER_SIZE`, refresh = pixel clock over totals rounded
  to whole Hz), published by the display worker into `Ports::head_timing` (a seqlock slot, no vCPU lock). A display no head lights answers with the monitor's authored preferred mode
  (inferred; not observed). Gate: params size 28 and guest branch in `DISPLAY_MODE_OBSERVED_BRANCHES` (`580.65.06`, i.e. Windows 580.88); otherwise `NOT_SUPPORTED` by name, counter
  `display_mode_unverified`, logged once.
- Tests (GPU-free): `an_armed_head_reports_its_programmed_mode`, `display_mode_control_follows_the_core_channels_programmed_mode_end_to_end` (1920x1080@60 and 1280x720@60 vectors),
  `display_mode_follows_the_heads_armed_mode_in_the_observed_layout` (hostile sizes, unknown ids, unverified branch). Written together with the code, not before it: the failing-first
  step was not done separately (the GPU run's refusal in run 615 is the failing baseline).


## VFIO REFERENCE RESULT (2026-10-11, real RTX 4070; supersedes HANDOFF item 4 "next step" and decides item 3's lead)
Measured on the real GPU (Windows 11 baseline, NVIDIA 580.88, vfio-pci, GSP observer; the monitor was attached, so the display controls answered as with a head):
- `kf_glgears.exe` RUNS on real hardware: RTX 4070 GL 4.6, RESULT OK, ~2200-2350 fps over 40 s, two screenshots 3 s apart differ (spinning gears), no Application event 1000.
  `cup2.exe` (QGA exec) PASS. Gears process = 5 RM clients, 206 controls, 43 allocs; the first client allocs 0x0000, 0x0080, 0x2080, 0x0073, 0x9096 (same order as kayfabe run 615).
- **`0x0073011a` is answered by the real GSP with `NV_OK`** (5 of 5 calls, 28-byte params, issued right after `0x73010c` head 0 returned a display id). kayfabe refuses it (0x56) and the
  ICD then crashes: this is the measured divergence, the H-GL lead of item 3 is confirmed as the only gears-path control that differs (inference: it is also the crash cause; the falsifier
  stays "answer it and the crash goes away"). The command is not in the open ogkm headers (`ctrl0073system.h` has `GET_SRM_STATUS 0x730119` and `HDCP_REVOCATION_CHECK 0x73011b`, nothing in between).
  Observed (measured, field names inferred; raw bytes are host-only, `/var/lib/kf-windows-20261005/vfio-gl-20261011/summary.md`): the reply is derived from the display head's current mode
  (it changed with the Windows mode: 1920x1080 and 1280x720, both at 60 Hz) and carries the request's display id; kayfabe's display engine holds the armed raster, so the answer can be
  DERIVED from it (never captured).
- `0x2080012f` (GPU_QUERY_ECC_STATUS): real status `0x56`, 5 of 5. Same as kayfabe: not a divergence (the falsifier's second suspect is out).
- Other display-common answers that differ from kayfabe's model (open layouts, `ctrl0073system.h`): `0x730101` GET_CAPS_V2 real capsTbl `81 2f` (AA_FOS_GAMMA_COMP, KSV_SRM_VALIDATION; SINGLE_HEAD_MST,
  SINGLE_HEAD_DUAL_SST, HDMI_2_0, CROSS_BAR, GLITCHLESS_MODESET) vs the model's all-zero table; `0x730102` GET_NUM_HEADS with `flags = NV0073_CTRL_SYSTEM_GET_NUM_HEADS_CLIENT` real 1 (flags 0: 4) vs the model's
  constant; `0x73010c` GET_ACTIVE real: head 0 -> display id 0x100, heads 1..3 -> 0. Not shown to matter for the crash.
- Every other gears control (ZBC `0x9096010x`, channel-group `0xa06c01xx`, `0x2080012b`, `0x80170e`, ...) was answered OK by the real GSP; kayfabe never reached them (crash first).
- Provenance: harness `scripts/bench/windows/glcrash/vfio_gl_reference.sh` (+ `session_prep.sh`, `vfio_gl_user.ps1`, `vfio_gl_mode.ps1`, analysis `vfio_ctrl_*.py`), host dir `/var/lib/kf-windows-20261005/vfio-gl-20261011/`
  (boot1 fresh overlay, boot2 reuse; observer 64 MiB cap, so keep the boot-to-GL window short). Host restored afterwards (nvidia 595.91.07, group 11 DMA-FQ, no qemu).

## HANDOFF (top)
1. **CUDA is FIXED on the baseline guest (measured, runs 613/614/615 at `9e4e2ffb`, production profile):** `cuCtxCreate -> CTX OK`, cup2/cup3/cup8 PASS
   (raw lines below). Cause of the 999 (measured in run 501's qemu.log, causal link inferred): the 65th live channel twin was refused by a
   hardcoded `VmCaps::from_declared(64, ...)` (`OverDeclaredCap { cap: 64, asked: 65 }`, then `GSP REFUSED fn103/0x0000c56f=0x1a`) while the guest had been told
   2048 per runlist. Fix = the **channel budget** (`docs/design/V3_CHANNEL_BUDGET.md`): derived from the host's unprivileged FIFO_GET_INFO,
   told to the guest AND enforced per runlist, property `channel-budget`, counter `birth_refused_cap`. On this host: 1789 per runlist (Derived; limit 2045).
   **Not run:** the control (old binary, same baseline with >64 live channels) that would show the pre-fix failure on a fresh guest; run 501's log is the evidence.
2. **OpenGL is NOT the same bug.** `kf_glgears.exe` STILL crashes at `nvoglv64.dll+0xb6d516` (Application event 1000, exit -1073741819) on the fresh
   baseline with budget 1789 and `birth_refused_cap=0` (runs 614, 615). The channel refusals in run 501 were coincident, not the GL cause.
3. **GL lead (measured, run 615 with KF3_RPC_TRACE=1, qemu.log lines 10913-13330):** the gears process (client 0xc1d0007a) allocs client/device/subdevice/`0x0073`
   display-common, then controls `0x20801315` ok, `0x2080012f` (GPU_QUERY_ECC_STATUS) `result=none` (unserviced -> 0x56; bare metal also answers
   0x56 for it, `V3_REFUSAL_AUDIT.md`: not a divergence), `0x730101` x2, `0x730102` x2, `0x73010c` ok, **`0x0073011a` `result=none` (refused 0x56)**, `0x730101`, then
   Frees and the access violation. No VA space, channel or GL class is ever allocated, so the crash is in ICD initialisation right after the display-common
   queries. `0x73011a` is not in the open ogkm headers (sits between `GET_SRM_STATUS 0x730119` and `HDCP_REVOCATION_CHECK 0x73011b`), no VFIO reference capture
   contains it (the vfio-8/9/10 boot captures end before an ICD runs). The WER minidump (analysed on the host only, `/var/lib/kf-windows-20261005/glcrash/`) shows a
   NULL-based read, i.e. the ICD used an object it never got. **Inferred, not proven:** the refused `0x73011a` (or the ECC one) leaves that object NULL.
   Falsifier: answer `0x73011a` as the real GPU does and the crash goes away; if it does not, the next suspect is `0x2080012f`'s reply shape.
4. **Next step (needs the owner's GPU time): a VFIO reference capture** of kf_glgears on the real GPU with the GSP observer running through the launch
   (`vfio_dvi_reference_click.sh` style, `gsp_ctrl_scan.py 0073011a,2080012f`) gives the real request size, reply bytes and status for both. Then answer them from
   `kf_disp::model` / the host (derive, never capture; no echo guess). Until then the layout of `0x73011a` is unknown, so nothing is guessed.
5. Open: opengl matrix (13) + Minecraft depend on item 3; CUDA matrix not run (lane launcher issue, below); Linux fast suite 30/30 + broker smoke on `9e4e2ffb` not run.

## Raw CUDA lines (run 614, `9e4e2ffb`, baseline guest + exFAT app disk attached after sign-in, interactive task as `vast`)
cup2: `ok cuInit(0) | devices=1 | name=NVIDIA GeForce RTX 4070 | compute=8.9 | totalMem=3835 MiB | ok cuCtxCreate(&ctx,0,d) | CTX OK | MEMALLOC OK 0x609400000 | CE rv=0xabcd1234 want=0xabcd1234 -> PASS`
cup3: `... CTX OK | PTX_TARGET=sm_86 | MODULE OK | FUNC OK | LAUNCH OK | SYNC OK | KERNEL rv=43 want=43 -> PASS`
cup8: `... CTX OK | CUP8 matmul N=2048 | LAUNCH OK | SYNC OK | CUP8 RESULT N=2048 bad=0 maxerr=0 C[0]=2048(exp 2048) -> PASS | CUP8 VERDICT: PASS`

## Reference numbers
- Idle Windows desktop live channel twins: about 55-58 after sign-in, 64 at the run-501 refusals (replay of BORN/`USERD relay released` lines of the 501 log; the replay
  overcounts by >=1 because not every free prints a line). The count tracks processes (frees drop it, e.g. 64 -> 14 around the in-session reboot): no monotonic leak seen.
- The budget holds nothing: `crates/kf-qemu/tests/channel_budget_lazy.rs` (no realize/prewarm path calls a host channel birth; a high budget has 0 live until guest births, frees return it).

## Why run 613 gave no verdict, and runs 620/621
- 613: my in-guest script defined `function R`, but `R` is the built-in alias of `Invoke-History`, so every result line threw ("Cannot locate the history for command line")
  and `gl-result.txt` stayed empty. The scheduled task DID run as `vast` in the interactive session (the +20 s screenshot of run 614 shows the PowerShell window with the cup output).
  Fixed (`Rw`) in `scripts/bench/windows/glcrash/kfgl.ps1`. The CUDA programs are console programs; QGA exec would also do.
- 620/621: launched by ME (the app-matrix lane `run_windows_apps.sh --run-base 620`), whose kayfabe launcher defaults to the legacy windows_broker.sh flag profile
  (BAR0_TRACE/MAPLOG/RPC_TRACE/METHOD_TRACE, none of CAPS/CTRL/IMP/PREEMPT_BIND/ZCULL/HOST_OWNED/USER_CHANNELS_PASSTHROUGH/ASYNC_PREEMPT) and has no sign-in/overlay-off: not the production
  profile, no evidence for or against the budget. Lane stopped, QEMU 621 killed, IOMMU group 11 back to DMA-FQ, driver nvidia, GPU free (02:30 CEST).
  Gap to record in V3_FEATURE_GAPS: the app-matrix lane's kayfabe launcher uses the legacy profile (pass `--broker .../windows_broker_prod2.sh`), has no password retry / overlay-off, and attaches the ISO (needs exFAT usb-storage).

## Log (newest first)
- `9e4e2ffb`: hostabi row for `NV2080_CTRL_CMD_FIFO_GET_INFO` (the budget's host query was refused at host driver 595.91.07 by the ABI gate: run 612 failed at realize, by name).
- `c9ef3457`: channel-budget property (ABI 26).
- `a1786956`: first fix (cap = 2048 x runlists), superseded.
- Runs: 610 (script bug: D: not ready), 612 (realize refused: hostabi row missing), 613 (R alias bug), 614 (CUDA PASS, GL crash), 615 (RPC trace + WER dump).
- Runner: `scripts/bench/windows/glcrash/tdr-run28.sh` (copy of tdr-run27 + GLTEST block: attach exFAT app disk, WER minidump, in-guest `kfgl.ps1`).
