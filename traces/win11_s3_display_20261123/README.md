# Win11 S3 wake on kf3: the post-wake display stop, root cause and fix (2026-11-23)

**STATUS: RESEARCH, 2026-11-23. Root cause of the post-wake window-channel stop MEASURED (control D0) and fixed (F3);
with F3, 3 consecutive CLEAN S3 cycles in one boot (D3), no nvlddmkm event 153.** Hypotheses and falsifiers were
committed before the runs (`PREREGISTRATION.md`, commit `a2df9a81`; addenda 1-3, each committed before the run it
governs). Measured = a run named here; inferred = said so.

Run: Vast instance 62182559 (RTX 4090 24 GB, 30 vCPUs, nested KVM, L1 kernel 6.8.0-59, host driver 580.159.04; the L1
marked its TSC unstable, so it was rebooted once with `tsc=reliable clocksource=tsc` before provisioning; clocksource `tsc`
for every run). Branch `claude/win-s3-display-20261123`. Builds on the box (`build_kf3.sh`, `kf3-bins/<rev>/`): **F3** =
kf3 `a2df9a81` (fix `22ddc9e7` + instrumentation `544472a7` + the preregistration; F3's code is `22ddc9e7`'s), **control**
= `544472a7` (instrumentation only, own worktree, target dir and build dir). Both carry #572 (F1) and #574 (F2). Guest:
Windows 11 LTSC 2024 from `windows_lane.sh`, 580.88 with GSP forced by `windows_gate.sh`:
`WINDOWS_GATE_PASS rev=a2df9a81 ... idle_s=120 ... resets=0` (`gate_verdict.txt`; deviation as on 2026-11-23:
`KF_WINGATE_IDLE_S=120`). Every S3 run boots a fresh child overlay of the gate's overlay `gate_gdisp` on q35, arm B, S3
enabled by the probe's wrapper. Probe: `s3disp.py` (`traces/win11_s3_tdr_20261123/s3tdr.py` plus the display lines; the
suspend is a scheduled task in the signed-in session). Cost: 1.64 USD of credit (44.82 -> 43.18), about 2 h 50 min of box
time; the box was destroyed by its id and its absence verified (`vastai show instances`: no ids).

## Results (measured 2026-11-23, box 62182559)

| run | kf3 | cycles (one boot) | GET word the guest reads after the wake (window 1) | display `STOPPED` / core `STALL` | nvlddmkm 153 | verdict |
|---|---|---|---|---|---|---|
| D1 `d1/` | F3 | 1 | 0x550 = engine GET | 0 / 0 | 0 | probe-FAIL on a Kernel-Boot 153 (addendum 1); hand-judged CLEAN; not counted |
| D2 `d2/` | F3 | 3 | 0xa60, 0xcb0, 0xed0 = engine GET | 0 / 0 | 0 | 2 CLEAN; c3 probe-FAIL: the guest re-slept on its own before the request (addendum 2); not N = 3 |
| **D3 `d3/`** | **F3** | **3** | **0x850, 0xcc0, 0xed0 = engine GET** | **0 / 0** | **0** | **3/3 CLEAN** |
| D0 `d0/` | control | 1 (stopped at FAIL) | **0x0** (engine GET 0xf60) | 1 / 9 | 3 | FAIL |

- **The control (D0) reproduces T1/T1b and shows the mechanism.** At the kept wake the restore line reads, for every
  allocated display DMA channel, GET word `before` = engine GET and `after` = `0x0` (chn 1: `"0xf60", "0xf60", "0xf60",
  "0x0"`). The guest's first post-wake PUT on window 1: `chn 1 PUT 0x1270: engine decoded 0xf60, published GET 0xf60,
  GET word the guest reads 0x0`, then `channel 1 STOPPED at 0xf60: PUT 0x1270 / GET 0xf60 outside the 4096-byte
  pushbuffer`, the core `STALL` on window 1's interlock, three `nvlddmkm` 153 records (15:02:34-15:03:04 guest time, 85-115
  s after the Kernel-Power 107 resume record) and the RM unload (`fn47_unload` 1). PUT - GET = 0x310 again; at the stop the ring holds
  the same 192-word `SET_INPUT_SCALER_COEFF_VALUE` group as T1 (`d0/d0_c1_wake_to_stall.txt`).
- **With F3 (D1-D3)** the restore line reads `after` = `before` for every channel, every first post-wake PUT line reads a
  GET word equal to the published GET, and there is no `STOPPED`, no `STALL`, no RM unload and no nvlddmkm 153 in any of
  7 wakes. D3's third wake left window 1 at GET 0xed0, where a 0x310-byte batch does not fit before the ring end (the
  T1/T1b/D0 position), and no stop followed. [inferred] the guest wrapped with a JUMP, as it does with a correct GET:
  that wake's first-PUT line for window 1 was suppressed by the log's rate limit, so the wrap itself is not logged.
- In every run: suspend 8-16 s after the request (D2 c3 excepted), `(S3 WAKE) ... KEPT`, `GSP-RESUME`, ssh back 18-29 s after
  `system_wakeup` with the same `LastBootUpTime`, `nvidia-smi` names the RTX 4090, no `USER_RING_REFUSED`, no
  `RAM add ... translation CHANGED`, no event 41/1001.

## Verdicts (falsifiers from PREREGISTRATION.md)

- **H5 (the kept wake's shadow restore zeroes the display channels' GET words while the engine keeps their GET):**
  **CONFIRMED.** Prediction (a) hit in D0 (2026-11-23, control `544472a7`; GET word 0 after the restore on every channel; the guest's first PUT read GET 0
  and overran the ring); falsifier (i) not hit. Prediction (b) hit in D3 (3/3 CLEAN) and in D1/D2's wakes; falsifiers
  (ii) and (iii) not hit. [inferred, from `nvkms-dma.c:184-230`] a writer whose batch does not fit before the end waits for
  a non-zero GET before it wraps; with GET stuck at 0 the Windows driver instead wrote on past the end (its room check is
  closed source).
- **H6 (guest ring arithmetic wrong regardless of GET):** **REFUTED** for these runs: F3 clean, including a wake at the
  position that needs a wrap.
- **H7 (CHNCTL/CHNSTATUS/ARMED words lost by the restore):** not separated: F3 keeps them too, and no ablation was run.
- **H8 (the guest restarts its channel at 0 at the resume):** **REFUTED**: every first post-wake PUT continues from the
  kept GET.

## What remains

- `windows-sleep-wake` (docs/RELEASE_CLAIMS.md): one box, one die (AD102), one boot of 3 cycles. The row's flip is the
  release run's to make, not this run's.
- The probe's judging needed two corrections during the run (addenda 1, 2); `s3disp.py` carries them.
- The first-PUT log is rate-limited across the process (`rate_eprintln!`): only the first wake is logged in full.

## Files

`PREREGISTRATION.md` (+ addenda); `s3disp.py`; `gpu_free_test_and_mutants.txt`; `gate_verdict.txt`; per run `dN/`: the
probe log, each cycle's result JSON and kf3 key lines; `d0/d0_c1_wake_to_stall.txt` (the D0 kf3 log from the wake's restore
line to the first STALL report, RPC trace and BAR lines removed). Scrubbed: box PARTUUID, GPU/VM UUIDs, box IP.
