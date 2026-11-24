# Preregistration: does the kf3 display plane decide the Windows power-off? (pc, 2026-11-24)

STATUS: RESEARCH, 2026-11-24. Written and committed before any boot of this experiment.

## Question
On 2026-11-24 (PR #595, kf3 `63e46069`, pc, win11_base Windows 11 Pro 25H2 with NVIDIA 580.88, no apps) 7 of 8 Windows boots
without a kf3 display plane (`-vga none -display none`, no `display=on,gop=on`) powered themselves off 61-110 s after the start, and
the 3 polled boots logged `nvlddmkm` event 153 every 8-10 s. The same image on the same pc with `display=on,gop=on` (2026-11-18 clone
boots, kf3 `6bfa2dbc`) logged no `nvlddmkm` event in its System log at 125 s uptime. Hypothesis **H2a**: the display plane being off is
the cause of the event-153 loop and the power-off.

## Design
Same binary (`kf3-bins/63e46069/qemu-system-x86_64`), same host, die and guest driver, same throwaway child overlay of the sealed
`win11_base` chain (the #595 `prep.qcow2` as backing file; the backing file is never opened for write), same QEMU arguments as the
#595 boot r8 (arm B CPU flags, 8 vCPU, 8192 MiB, `fb-mb=4096`, `guest-driver=580.65.06`, S3 off, network off). Arms, interleaved in
this order so that a drift in time or host state hits all arms alike:

| arm | kf3 device | display backend |
|---|---|---|
| A (control) | `kf3-gpu,...` as #595 | `-vga none -display none`, no broker |
| B | A plus `display=on,gop=on,gop-efi=<signed kf-gop>,display-broker=<sock>,display-broker-uid=0` | headless `nvkvm-display-broker --backend test` (what the 2026-11-18 clone boots used, `propimage_check.py`), `-display none` |
| B-off | B plus `unmapped-reads-zero=off` | as B |

Order: A1 B1 A2 B2 A3 B3, then B-off 1, 2, 3 while time remains (hard stop 20:50Z). Each boot is polled over ssh every 10 s until 300 s
after the QEMU start (or until QEMU exits), with a QMP event recorder on a second QMP socket (GUEST_PANICKED with the Hyper-V
crash arguments, SHUTDOWN with reason, RESET, STOP/RESUME, POWERDOWN). Per poll: LastBootUpTime, System-log counts for
nvlddmkm 153/141, Kernel-Power 41, 1074, 6008, BugCheck 1001; every 60 s the NVIDIA adapter's ConfigManagerErrorCode and
`Get-PnpDevice` status. Host side: `dmesg` lines with `mmuWalkMap` / `NV_ERR_NO_MEMORY` / Xid during the boot, and the QEMU log
lines naming a `0x51`.

## Predictions and falsifier
- H2a predicts: arm A repeats the failure (153 events, power-off in most boots); arm B shows no event 153 and stays alive at 300 s.
- **Falsifier of H2a: an arm B boot also shows `nvlddmkm` 153 or dies (QEMU exits or ssh never answers again) before 300 s.**
  One such boot out of three is enough to reject "the display plane being off is THE cause" (it could remain a factor).
- If arm A does NOT fail (all 3 alive, no 153), the control did not reproduce and arm B says nothing about H2a: reported as such.
- F1 question (not a hypothesis test, recorded either way): the earlier data are 4 of 4 `mmuWalkMap ... NV_ERR_NO_MEMORY` with the
  default fill ON and 0 of 4 with `unmapped-reads-zero=off`, all display-off. Here: does it also appear with display ON?

## Rules
Measured values name run, date and revision. Anything not seen in a log is INFERRED and labelled so. No GPU bind, no nvidia tools beyond
reading `dmesg` and `nvidia-smi --query` for the idle check, nothing on the pc deleted except this experiment's own scratch.
