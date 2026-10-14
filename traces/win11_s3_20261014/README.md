# Win11 S3 sleep/wake on kf3 `0c5fedff` (merge of the S3-resume branch and the user-ring-scan branch), 2026-10-14

STATUS: RESEARCH. Result: **Windows 11 does NOT yet survive S3 sleep + wake on this host. 0/3 clean cycles.**
Measured = a run named here (kf3 log, host `perf kvm` trace, guest event log, screendump). Inferred = said so.

Host: trusted box, RTX 4070, run 2990 (owner's disk `win11-full-b`), kf3 `0c5fedff` (= `7f781318d` + `c287af4f`,
merge commit `0c5fedff`, GPU-free `cargo test -p kf-chan -p kf-qemu -p kf-rm -p kf-gsp`: 1301 passed / 0 failed).
Guest sleeps with `rundll32.exe powrprof.dll,SetSuspendState 0,1,0` (hibernate off; the 2-minute idle timeout did
not fire in 6 min, something keeps the guest awake), QMP `info status` = suspended, QMP `system_wakeup`.

## What worked (measured)
- Windows' suspend is served by the SAME code as Linux's: `GSP fn 47 UNLOADING_GUEST_DRIVER bInPMTransition=true
  newLevel=3`, `GSP phase Running -> Suspending -> Halted`; no `GSP REFUSED` / `UNSERVICED` between fn 47 and the wake
  (run 5: 0 lines after fn 47 until the wedge). The wake is `device reset #N (S3 WAKE) ... KEPT` every time.
- With the Hyper-V timer enlightenments removed the Windows GPU driver DID resume the GSP twice
  (`GSP-RESUME ... no publish`, `Cold -> Booted -> Running`: runs 3 and 5), and Windows re-programmed all
  BARs of 00:06.0 (pio timeline `pio-timeline-nohv-wake-head.txt`, reg 10..24, command 0x406). So the kf3 resume
  code and the FBSR/control answers of `pmsr.rs` are enough for the WINDOWS driver's suspend + resume RPCs
  (the falsifier "GSP REFUSED/UNSERVICED on the unload path" did not occur; the other falsifier, "desktop does not come
  back within 60 s", DID occur, for reasons that are not the GSP, below).

## What failed (measured), per run (kf3 excerpts `kf3-run*.txt`, logs `claude-s3win-var-*.log`)
| run | guest CPU flags | wake outcome |
|---|---|---|
| 1, 2 (cycle 1 and 2, production flags `hv_time,hv_synic,hv_stimer,...`) | full | Windows executes but NO device is ever touched: all 8 vCPUs in the idle HLT at one RIP, 740k HLT exits/s, `kvm_hv_stimer_start_one_shot count=18446744073708147579` (= -1404037 as s64) re-armed every ~13 us, 0 BAR0 writes, every PCI BAR (all devices) still unmapped 3 min after the wake, screen black. Pio timeline: firmware ran 0.07 s, Windows read 00:06.0 vendor/subsystem and then only PM-timer reads. `perf`: 706k `kvm_vcpu_wakeup`/s. |
| 3 (no `hv_time/synic/stimer`) | nohv | BARs restored, `GSP-RESUME`, `Running`, then ~62 s after the wake the display driver tore down every channel and re-created contexts and the guest reset itself (`device reset #3 vCPUs out`, cold). No dump was written; the next boot went to Windows Recovery ("Your device ran into a problem and couldn't be repaired"). |
| p3 (nohv, AutoReboot=0) | nohv | `S3 WAKE ... KEPT`, then 5 s later the guest bugchecked **0x5C HAL_INITIALIZATION_FAILED (0x201, ..., 0xc0000001)** (event 1001, `C:\Windows\Minidump\101426-6843-01.dmp`, Kernel-Power 41 bugcheck 92 = 0x5c) and rebooted: the "desktop came back" was a cold boot, no `GSP-RESUME`. |
| 4 (nohvA, cycle 1) | nohv | `GSP-RESUME`, `Cold -> Booted`, never `Running`; guest spinning (PAUSE exits; 126k HPET counter reads/s at `gpa 0xfed000f0`), black screen, >10 min. |
| 5 (tscB) | nohv + `migratable=off,+invtsc` | `Booted -> Running`, then Windows wedged: 139k/s `pio_read 0xb000` (PM1_STS, always 0x8101 = WAK_STS|PWRBTN_STS|TMR_STS) + 114k/s HPET reads, black screen, agent dead >4 min; the HPET was enabled and its counter advancing (`xp 0xfed000f0` x3), PM1_EN=0, PM1_CNT SCI_EN |
| 6 (tschpC) | tscB + `-machine hpet=off` | `Booted -> Running`, then the same spin with the PM timer in place of the HPET: 137k/s `0xb008` + 136k/s `0xb000` (PM1_STS = 0x8101), black screen |

Hypotheses (stated before each run) and verdicts:
- H-hv "Windows' S3 wake works without hv_time/synic/stimer": **FALSIFIED as a sufficient fix** (runs 3-6: the hv timer
  storm is gone, three other failures appear; one run reached Running + a 62 s reset, one bugchecked 0x5C, three spun in
  a PM1_STS/timer poll). It IS necessary for getting past the stimer storm (runs 1-2 vs 3-6, repeated twice each).
- "The 0xefbe0000 carve-out REFUSED leaf is the cause of the +62 s reset": **FALSIFIED** - the same 3 refusals
  (`CLIPPED/REFUSED ... inside the firmware carve-out`) occur in every normal boot (run 1, 2, 5, the 13:07 boot) with no reset.
- "kf3's GSP resume is what fails": not supported: in runs 3, 5, 6 the GSP reached Running; the wedges/crash are in
  HAL/timer/ACPI code before or beside the display driver. Inferred (not proven): the common factor is QEMU resetting
  every CPU/timer/ACPI device on `system_wakeup` (TSC to 0, HPET/PM timer, PM1 regs, `RESET_TYPE_WAKEUP` in
  `hw/i386/pc.c`) under an i440FX + OVMF + Windows 11 guest; the Linux guest (q35, no hv flags) tolerates it.
- Cost of dropping the enlightenments (measured, run 3 wake window): without `hv_time` Windows reads the PM timer
  (407k pio exits in 25 s = ~16k/s) or the HPET (114-126k exits/s while spinning) instead of the TSC page.

## Not done / next
- A Windows guest that is not S3-capable without a GPU driver ("Graphics" disables S3 with the Basic Display Adapter), so
  the bare QEMU control (same flags, no kf3) could not be run. Next control candidates: q35 instead of i440FX (the Linux
  guest's machine), identifying the PM1_STS/timer spin (kernel stack: a crash dump of the wedged VM), RTC/`-rtc clock=host`,
  QEMU patch keeping the TSC / `RESET_TYPE_WAKEUP` CPU reset.
- Linux sanity of `0c5fedff` (`desktop_s3.sh` x3, `desktop_smoke.sh`): see `linux-sanity.txt` if present.
- VM 2990 left running on `0c5fedff` with the production CPU flags and standby-timeout 0; guest `recoveryenabled No`
  (bcdedit) so killed boots do not enter Automatic Repair.

Scripts used: `claude-s3win-*.{sh,py}` (copied here): cycle (suspend, wake, perf), variant (relaunch with other CPU flags and judge each cycle), pio timeline decoder.
