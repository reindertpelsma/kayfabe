# Win11 S3 wake on kf3: the timer storm's root cause, and its fix (2026-11-23)

**STATUS: RESEARCH, 2026-11-23. Root cause of the post-wake hang MEASURED; fix MEASURED 3/3 on one box; a separate
post-wake TDR (below) is NOT fixed.** Hypotheses and falsifiers were committed before the runs: `PREREGISTRATION.md`
(and its two addenda). Measured = a run named here; inferred = said so.

Run: Vast instance 62082978 (RTX 4090, 30 vCPUs, 49 GB, nested KVM, L1 kernel 6.8.0-59, host driver 580.159.04), branch
`claude/win-s3-wake-20261123`, kf3 `1052e2f9` (the probe file was edited on the box after the checkout, so later runs
report the tree `-dirty`; the QEMU binaries were `kf3-bins/1052e2f9/` and, for F1, `qemu-build-f1/`, sha256 prefix
`01f59a74ae25c31f`, built by `r3_armB_F1_stable_tsc/build_f1.sh`). Guest: Windows 11 LTSC 2024 from `windows_lane.sh`, driver
580.88 with GSP forced by `windows_gate.sh` (`WINDOWS_GATE_PASS rev=1052e2f9`, `gate/s3g_windows.log`; deviation: idle
shortened to 120 s with `KF_WINGATE_IDLE_S=120`). Every S3 run boots a fresh child overlay of the gate's overlay on q35 with
`-global ICH9-LPC.disable_s3=0` appended by the probe's wrapper (`win_vm.sh` disables S3 otherwise). Probe: `s3probe.py`
(suspend through a scheduled task in the signed-in session, QMP `query-status` = suspended, `system_wakeup`, tracefs).
Cost: 1.46 USD of credit, about 2 h 15 min of box time; the box was destroyed by its id and its absence verified.

## Results (measured 2026-11-23, kf3 `1052e2f9`, box 62082978)

| run | CPU flags | host clocksource | QEMU | verdict (preregistered definition) |
|---|---|---|---|---|
| R1 `r1_armB_unstable_tsc/` | arm B (`hv-time,hv-synic,hv-stimer,...`) | kvm-clock (L1 TSC "unstable") | stock 10.2.4 + kf3 | 1/1 CLEAN |
| R1t `r1t_armB_stable_tsc/` | arm B | tsc (`tsc=reliable clocksource=tsc`, rebooted) | stock | 0/1: **the 2026-10-14 storm, reproduced** |
| R3 `r3_armB_F1_stable_tsc/` | arm B | tsc | **F1** (keep the Hyper-V MSR block across the wake) | **3/3 CLEAN** |
| R2 `r2_armA_stable_tsc/` | arm A (`host,-vmx`) | tsc | stock | 1/1 CLEAN |

(R1's first attempt, `r1_attempt1*`: the suspend was requested from the ssh session, which is not the interactive session,
and Windows never suspended; the probe's QMP status parser was also wrong. Fixed, not counted.)

- **R1t (the storm):** the TSC page at GPA 0xd000 was VALID before the suspend (sequence 1, scale `0x012346222f5c28f5` =
  2.24998 GHz, offset `0xfffffffffec42d4d` = -2.0698 s) and read **byte-identical while suspended and after the wake**: KVM
  stopped maintaining it. At the wake every vCPU's TSC offset moved by -588,320,593,649 cycles = **-261.48 s, the guest's
  uptime** (`time_now` 261.08 s): the guest TSC restarted near 0. The guest wrote no `HV_X64_MSR_REFERENCE_TSC`,
  `GUEST_OS_ID` or `HYPERCALL` after the wake (only `SIMP` and `SINT3` on all 8 vCPUs). 1.67 s after the TSC jump the first
  one-shot was `count = 18446744073705572754` (-0.3979 s as s64) with `time_now = 2610799925` (261.08 s): `count - time_now`
  wraps when multiplied by 100, and the two timers re-armed 276625 and 274953 times in 4 s. The guest never answered ssh;
  exits were MSR/CR8/VINTR/hypercall at five RIPs (`c1_wedge_sample.json`).
  [inferred, arithmetic] stale page time at that moment = -2.0698 s + 1.671 s = -0.399 s, the deadline Windows wrote.
- **R3 (F1):** `kayfabe-F1: S3 wake kept the Hyper-V MSR block of 8 vCPU(s)` at each wake; after the first wake the page
  read sequence 5 with a new offset (`0xabe7806e`, KVM recomputed it for the restarted TSC); first post-wake one-shots
  `count - time_now` = 11092, 4164, 13964 (1.1, 0.4, 1.4 ms); ssh back after 59, 14 and 17 s with the same
  `LastBootUpTime`; `(S3 WAKE) ... KEPT` and `GSP-RESUME` each cycle; `nvidia-smi` names the RTX 4090; no event 41/1001.
- **R1:** with the TSC page invalid (all zeros: no KVM master clock on a kvm-clock L1), Windows reads `TIME_REF_COUNT`,
  which is always KVM's, and the wake is clean without F1. So whether stock QEMU hangs depends on the HOST: a stable-TSC
  host (the 2026-10-14 trusted host; any bare-metal host with an invariant TSC) hangs, a typical nested Vast box does not.

## Verdicts

- H1 (storm = KVM's wrapped one-shot): **CONFIRMED** (R1t 2026-11-23, `1052e2f9`: `wraps_x100` true, the same count re-armed ~70k/s per timer).
- H2 (partition-wide Hyper-V state wiped, not re-established): **CONFIRMED** (R1t: no guest write of 0x40000021/0/1 after
  the wake; page frozen) and F1, which restores it, makes the wake clean (R3).
- H3 (per-VP SynIC not re-enabled): **REFUTED as stated** (Windows re-writes `SIMP` and `SINT3` after the wake; R1, R1t).
- H4 (guest reference time behind KVM's): **CONFIRMED** (R1t numbers above).
- H5 (TSC not reset by the wake): **REFUTED** on this host (R1t: the TSC went back by the uptime). [read] KVM matches a host
  write of 0 to `cur_tsc_offset`; why that offset gave a TSC near 0 here is not settled.
- H6 (q35 arm A may be clean): **not falsified** (R2 1/1 CLEAN; one cycle only).
- F1: **CONFIRMED** 3/3 on one box, one die, one driver.

## What is NOT fixed (measured 2026-11-23, `1052e2f9`: event 153 in R2 and in R3's cycles; the unload in kf3's log after every resume of R1, R2, R3)

About 34-45 s after each wake the Windows driver logs three `nvlddmkm` event 153 records ("Resetting / Reset / Restarting
TDR occurred on GPUID:5", `r2_armA_stable_tsc/r2_event153.txt`) and kf3 logs `fn 47 ... bInPMTransition=false newLevel=0`
(the guest RM unloads) followed by a new GSP life: the driver recovers from a GPU timeout and the desktop and
`nvidia-smi` work afterwards. The preregistered CLEAN definition did not include event 153, so the cycles above count as
clean by it; a release gate that treats 153 as a failure would fail them. Lead for the next run (not analysed): between the
wake and the unload kf3 logs repeated `GSP rpc UNSERVICED { code: 76 }` (GSP_RM_CONTROL) lines
(`r2_armA_stable_tsc/r2_wake_to_unload.txt.gz`).

## Files

`PREREGISTRATION.md`; `s3probe.py` (the probe); `f1_keep_hv_on_wake.py` (the F1 edit as run; the product hunk is
`scripts/release/build_qemu.py` `WAKE_NEW`, identical minus the diagnostic `fprintf` and the comment); per run: `*.out`
(probe log), `*/c*_result_small.json` (all facts per cycle), `*_kf3_excerpt.txt`; `gate/` (the lane's and the gate's logs).

Probe errata (found after the runs): `summarize` matched `msg send N` while KVM prints `msg send result N`, so every
`expirations_ok/_fail` count in the result files is a parse miss (all counted as `_fail`); fixed in `s3probe.py` after the
runs. No verdict used it. The R3 cycles 2 and 3 have no `tsc_page` entry because the boot-time trace that names the page's
GPA was cleared by cycle 1.
