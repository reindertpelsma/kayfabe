# Win11 S3 wake on kf3: the post-wake TDR, first cause fixed, second cause found (2026-11-23)

**STATUS: RESEARCH, 2026-11-23. One root cause MEASURED and fixed (F2: the RAM-topology revocation that killed every
user-ring relay born before the wake); the TDR is NOT gone: a second cause (display window channel 1 stopped by kf3
after the wake) MEASURED in 2 of 2 F2 cycles, root cause not settled.** Hypotheses and falsifiers were committed before
the run (`PREREGISTRATION.md`, commit `a023a5143`). Measured = a run named here; inferred = said so.

Run: Vast instance 62141661 (RTX 4090 24 GB, 30 vCPUs, 49 GB, nested KVM, L1 kernel 6.8.0-59, host driver 580.159.04; the
L1 marked its TSC unstable, so it was rebooted once with `tsc=reliable clocksource=tsc` before provisioning, clocksource
`tsc` for every run). Branch `claude/win-s3-tdr-20261123`. Builds on the box: **F2** = kf3 `0c16cb72`
(`kf3-bins/0c16cb72/`), **control** = `origin/next` `90a658c3` (`kf3-bins/90a658c3/`, built by `build_kf3.sh` in its own
worktree and target dir); both carry the #572 wake hunk (F1). Guest: Windows 11 LTSC 2024 from `windows_lane.sh`, 580.88
with GSP forced by `windows_gate.sh`: `WINDOWS_GATE_PASS rev=0c16cb72 ... idle_s=120 resets=0` (deviation as on 2026-11-23:
`KF_WINGATE_IDLE_S=120`). Every S3 run boots a fresh child overlay of the gate's overlay `gate_tdr` on q35, arm B, with
S3 enabled by the probe's wrapper. Probe: `s3tdr.py` (the 2026-11-23 probe with its cycle replaced: judged 120 s after the
wake, events since this cycle's suspend request, 153 fails the cycle). Cost: 0.72 USD of credit (45.57 -> 44.85), about
1 h 35 min of box time; the box was destroyed by its id and its absence verified (`vastai show instances-v1`: no ids).

## Results (measured 2026-11-23, box 62141661)

| run | kf3 | wake -> first event 153 (guest clock: event 107 -> 153) | `USER_RING_REFUSED ... Topology` after the wake | display channel 1 `STOPPED` | verdict |
|---|---|---|---|---|---|
| T0 `t0/` | control `90a658c3` | 24 s | 6 relays (hosts 0x49, 0x2004b, 0x3a, **0x37**, 0x3b, 0x2003d) | no | FAIL (153) |
| T1 `t1/` | F2 `0c16cb72` | 68 s | 0 | yes: `PUT 0x1230 / GET 0xf20 outside the 4096-byte pushbuffer` | FAIL (153; judged by hand, `t1/t1_c1_guest_events_manual.txt`) |
| T1b `t1b/` | F2 `0c16cb72` | 70 s | 0 | yes: `PUT 0x1130 / GET 0xe20 ...` | FAIL (153) |

Each FAIL stopped the run after cycle 1 (consecutive-cycle rule), so no run reached N >= 3. In every run: suspend 9 s after
the request, `(S3 WAKE) ... KEPT`, `GSP-RESUME`, ssh back after 11-14 s with the same `LastBootUpTime`, `nvidia-smi` names
the RTX 4090, the guest RM unloads (`fn 47 ... newLevel=0`) after the TDR and comes back; no event 41/1001.

- **RAM topology at the wake (F2 build, T1/T1b):** the firmware already re-splits the low-RAM section at boot (`RAM del
  gpa=0x100000` / `RAM add gpa=0x100000 len=0x7bf00000` / `... len=0x7ff00000`, TSEG toggling), and the wake adds more
  (`RAM del gpa=0x50000`, `RAM add gpa=0x0 len=0x93000 fd_off=0x0: translation kept`, ...). Every line says
  `translation kept (generation 1)`; no `translation CHANGED`. Before F2 each of those lines moved the generation
  (`traces/.../t0`: six relays refused `Topology` at their first access after the wake, among them host 0x37 =
  `0xc1d00029:0xff04000e`, the channel the driver RESET_CHANNELed in R2 of `traces/win11_s3_wake_20261123/`).
- **The second cause (T1, T1b):** about 12 s after the wake kf3's display engine halts window channel 1: the guest's PUT
  is GET + 0x310 past a GET near the end of the 4 KiB pushbuffer the guest itself declared (`ChannelAllocated { kind:
  Window, instance: 1, ... limit: 4095 }` at boot, `t1b/t1b_kf3_keys.txt`), with no wrap. 10 s later the core channel parks
  (`display: STALL Core 0 (chn 0) update 0x0 parked 1016 ms: waiting for its interlock group ... bits 0x2`), the window
  never completes its update, and the driver's TDR follows ~70 s after the wake. No display channel is allocated again
  between the wake and the TDR, and `INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER` (`0x20800a58`) is not re-sent. In T0 and R2 the
  relays died first and no display stop was logged.

## Verdicts (falsifiers from PREREGISTRATION.md)

- **H1 (RAM-topology revocation kills the relays born before the wake):** the revocation is **CONFIRMED** as a real defect
  (T0: 6 Topology refusals at the wake; T1/T1b with F2: 0, and the wake's RAM changes all `translation kept`; falsifiers
  (a) and (b) not hit). As the cause of the TDR: **falsifier (c) HIT** (T1, T1b: event 153 with no Topology refusal). F2
  removes one trigger (the TDR moves from 24 s to 68-70 s after the wake) but not the TDR.
- **H2 (an UNSERVICED code-76 RPC starves the guest):** **REFUTED** as the cause: code 76 is `GSP_RM_CONTROL`, the
  post-wake UNSERVICED set is the boot set (read, PREREGISTRATION.md), and its presence did not change between T0 and
  T1/T1b while the failure mode did. [inferred] answering them would not stop either TDR mechanism seen here.
- **H3 (other kept-channel state diverges):** not supported by these runs: with F2 no relay refused or died, and the one
  stall seen is in the display engine. Not separately tested.
- **H4 (display re-programming race after the wake):** **its prediction HIT** in T1 and T1b (display `STOPPED` + core
  `STALL` before the TDR). Mechanism not settled. [inferred] the guest's window-channel PUT no longer agrees with the
  GET kf3 kept across the wake (`display KEPT` in the wake reset): the guest acts as if its channel restarted or wrapped at
  the resume while kf3 continues from the pre-suspend GET. Next step (GPU-free first): read what the Windows display path
  does with a window channel at D0 entry (GET/PUT re-init, the channel's `offset`) and what kf3's kept display state does
  with it; on hardware, log the window channel's GET/PUT at the suspend and the first guest PUT write after the wake.
- The 34-45 s delay of 2026-11-23 is, with the control build, 24 s here; with F2 68-70 s. Not explained further.

## Files

`PREREGISTRATION.md`; `s3tdr.py` (its `%`-format bug crashed T1 after the wake of cycle 1; fixed before T0 and T1b, which
ran the fixed file); `gpu_free_prefix_failure.txt` (F2's tests failing before the fix with `Err(Topology)`, and the
mutants); per run: the probe log, `c1_result.json`, the kf3 key lines; `t1/t1_c1_wake_to_unload.txt.gz` (the T1 kf3 log
from the suspend to the unload, BAR1/BAR0 trace lines removed); `t1b/t1b_c1_display_stop_window.txt` (the lines around the
display stop). Scrubbed: box PARTUUID, VM/GPU UUIDs, SMBIOS serial, box IP.
