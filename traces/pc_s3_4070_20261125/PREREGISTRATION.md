# Preregistration: Windows S3 sleep/wake under kf3 on a second die and host driver (pc RTX 4070, 2026-11-25)

STATUS: RESEARCH, 2026-11-25. Written and committed before the first boot of this run (pc, 2026-11-24 evening to 2026-11-25 night UTC, kf3 `7bd968c2`,
tag `pcs3`).

## Question
The release row `windows-sleep-wake` rests on 3 consecutive CLEAN S3 cycles in one boot on one die: run D3 of
`traces/win11_s3_display_20261123/` (RTX 4090, AD102, Vast box, host driver 580.159.04, kf3 `a2df9a81`, 2026-11-23). A second die and a second host
driver are wanted. This run uses the owner's pc: RTX 4070 (AD104), host driver 595.91.07, kernel 7.0.0-34, clocksource `tsc`. kf3 reaches the
host GPU through unprivileged RM; **no GPU bind and no VFIO bind** (the desktop stays up).

## Design
- Guest: the sealed non-proprietary `win11_base` image (Windows 11 Pro 25H2, NVIDIA 580.88), one fresh thin child overlay per boot of the
  `prep.qcow2` of `traces/vfio_s3_control_20261124/` (hibernation off, standby and monitor timeouts 0, autologon); backing files are never opened
  for write; each overlay is removed after its boot. q35, 8 vCPU, 8192 MiB, `fb-mb=4096`, `guest-driver=580.65.06`, offline slirp, S3 enabled with
  `-global ICH9-LPC.disable_s3=0`, **display plane on** (`display=on,gop=on`, signed kf-gop of `win11_base`, headless `nvkvm-display-broker
  --backend test`): run `traces/pc_displayexp_20261124/` (pc, 2026-11-24, kf3 `63e46069`) found that a Windows guest without it ends in bugcheck 0x113.
- Binaries, both built on the pc from the pinned QEMU 10.2.4 tarball (sha256 checked against `scripts/bench/qemu.pin`) in a fresh tree, by
  `scripts/bench/build_kf3.sh` (stock) and `scripts/bench/build_kf3_patched.py --with-patch hyperv-wake` (patched copy of that tree), both from
  kf3 `7bd968c2` (the head of `origin/next` when the run started): `kf3-bins/7bd968c2/` and `patched-hyperv-wake/kf3-bins/7bd968c2/`. The older
  shared tree `/workspace/bench/qemu-10.2.4` already contains the hyperv-wake hunk (`grep` finds it in its `hw/i386/pc.c`), so `kf3-bins/63e46069`
  is not a stock binary and is not used.
- CPU flags: arm A `host,-vmx` (the documented configuration); arm B = A plus `hv-relaxed,hv-vapic,hv-spinlocks=0x1fff,hv-time,hv-vpindex,hv-synic,
  hv-stimer,hv-crash` (the flags of the 2026-11-23 gate and of `traces/vfio_s3_control_20261124/`).
- Cycle (`s3kf3pc.py`, which reuses `s3vfio.py`'s cycle): the sleep is requested by a scheduled task in the signed-in session (`SetSuspendState 0,1,0`), not by
  ssh; the host wakes the guest with QMP `system_wakeup`; after ssh returns the harness waits 150 s, then reads the System log from the cycle start and
  `nvidia-smi` and the session in the guest. Before cycle 1 the unattended-sleep timeout is set to 0 (addendum 2 of the 2026-11-23 display probe).
  A QMP event recorder on a second socket keeps every event (SHUTDOWN, GUEST_PANICKED with the bugcheck codes, WAKEUP, SUSPEND, RESET) with host time.
  Per cycle the harness records the sleep trigger time, the wake time, the time to ssh, the nvlddmkm and Kernel-Power events, whether explorer is alive in
  the interactive session and whether `nvidia-smi` names the RTX 4070.
- Pre-run check, written down: `current_clocksource` of the host must read `tsc` (no change of the host's kernel command line; if it does not, report and stop).
- **CLEAN cycle (the rule of `traces/win11_s3_display_20261123/s3disp.py`, plus the QMP events):** suspended within 120 s; ssh back within 150 s of
  `system_wakeup` with the same `LastBootUpTime`; kf3 logged `(S3 WAKE) ... KEPT` and `GSP-RESUME` and no cold reset; **no `nvlddmkm` event 153**; no display
  `STOPPED`, no core `STALL`, no RM unload (`fn 47 ... newLevel=0`), no `USER_RING_REFUSED ... Topology`; no Kernel-Power 41, no 1001, no 6008; `nvidia-smi`
  names the RTX 4070; explorer in the interactive session; no QMP `GUEST_PANICKED`, `SHUTDOWN` or `RESET`. A cycle that is not CLEAN ends its boot.
  A STORM is a cycle where ssh never returns and the vCPUs exit more than 100,000 times a second (`s3vfio.storm_signature`).
- Default `unmapped-reads-zero` (on, PR #580) for every arm. Each boot records whether the host RM logged `mmuWalkMap ... 0x51 NV_ERR_NO_MEMORY` (the open
  finding B1: `dmesg` window of the boot, and the 0x51 lines in the QEMU log).

## Hypotheses and falsifiers
Run order: H1 (5 cycles), H2 (2 cycles), H3 (3 cycles); no new boot after 01:15Z.
- **H1** (arm B plus the optional hyperv-wake patch): the guest survives 5 consecutive S3 cycles in one boot, every one CLEAN.
  **Falsifier: any of the 5 cycles stalls, storms, TDRs (nvlddmkm 153) or fails another clause of the CLEAN rule.**
- **H2** (control, arm B on the stock binary, 2 cycles, one boot each): the unpatched binary storms (the VFIO control of 2026-11-24 stormed 7 of 7 on this pc).
  **Falsifier: the unpatched arm wakes CLEAN in 2 of 2.** If it does not storm, H1's difference to the patch is not shown by this run.
- **H3** (arm A, `host,-vmx`, stock binary, one boot): the guest survives 3 consecutive CLEAN cycles. **Falsifier: arm A storms** (or any non-CLEAN cycle).
- Extra, only if time remains after H3 (stated now so it is not a post-hoc choice): H1 again with `unmapped-reads-zero=off`, up to 3 cycles, to record
  whether 0x51 is absent with it off (it was absent in 3 of 3 boots of the display experiment). It is not part of H1-H3.

## Rules
A value this run reports names its run (pc, 2026-11-25, kf3 `7bd968c2`, tag `pcs3`). Anything not seen in a log is INFERRED and labelled so. A kayfabe bug found
is listed with its evidence, not fixed here beyond a trivial change. Nothing on the pc is deleted except this run's own scratch; if free space on `/workspace`
would fall below 8 GB the run stops. `PC_ACTIVE.log` START/END lines and the lock `/root/prov/pc_gpu.lock` follow the pc convention.
