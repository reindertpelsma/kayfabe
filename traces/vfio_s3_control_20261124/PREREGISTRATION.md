# VFIO control of the Windows S3 wake storm: hypotheses and falsifiers, written before the run (2026-11-24)

**STATUS: RESEARCH, 2026-11-24. Written and committed BEFORE any VM of this directory was started and before the GPU
was unbound.** Results go in `README.md` beside it; this file is not edited after the first run except by a dated
addendum at the end, written before the run it governs.

## Question (owner, 2026-11-24)

Is the Windows post-wake timer storm of `traces/win11_s3_wake_20261123/` (run R1t, kf3, Vast nested KVM with a stable
L1 TSC) a property of QEMU's S3 wake, independent of kayfabe, as `docs/design/V3_S3_RESUME.md` §7b says? The reference
arm is a real passthrough: the owner's RTX 4070 (01:00.0 and its audio function 01:00.1, IOMMU group 11) on vfio-pci,
STOCK QEMU, no kf3 device, bare-metal host with a stable TSC.

## What #572 predicts (read: `V3_S3_RESUME.md` §7b, `traces/win11_s3_wake_20261123/README.md`)

QEMU's wake (`pc_machine_wakeup`, `RESET_TYPE_WAKEUP`) cold-resets every vCPU, which zeroes the Hyper-V MSR block and
writes the zeros to KVM (`HV_X64_MSR_REFERENCE_TSC = 0`); the guest TSC restarts near 0; Windows does not re-enable the
TSC page and reads the stale one; its one-shot synthetic-timer deadline is "negative"; KVM's `stimer_start` wraps and
re-arms forever; every vCPU waits in HLT. This needs a valid guest TSC page, i.e. a KVM master clock, i.e. a host whose
clocksource is a stable TSC. Nothing in that chain involves the GPU device model.

## Host facts measured in Phase 1 (2026-11-24 ~00:40Z, read-only)

AMD Ryzen 9 7900, kernel 7.0.0-34-generic, `current_clocksource` = `tsc` (available: tsc hpet acpi_pm), dmesg
`Refined TSC clocksource calibration: 3700.007 MHz`, `Switched to clocksource tsc`, no `TSC unstable` line,
`constant_tsc` and `nonstop_tsc` on every CPU, kernel command line without `tsc=`/`clocksource=`. Host driver
595.91.07 on 01:00.0, `snd_hda_intel` on 01:00.1, reset methods `flr bus`, `kernel.sysrq = 176`.

## Arms (run in this order)

All arms: q35 (`pc-q35-10.2`), 8 vCPUs, 8 GiB, OVMF secure boot + swtpm, `-global ICH9-LPC.disable_s3=0`, the GPU and
its audio function through `vfio-pci`, no kf3 device (the binaries of arms A and B have no kf3 code at all), one
throwaway qcow2 overlay per boot over the sealed, read-only, non-proprietary `win11_base` image (Windows 11 Pro 25H2,
NVIDIA 580.88, no applications), offline slirp (`restrict=on`), the image's own TPM and OVMF variables restored into the
work directory. Deviation from the 2026-11-23 runs: those used Windows 11 LTSC 2024; the pc's only LTSC guest
(`ownerwin`) has no disk since 2026-11-15, so the Pro 25H2 base is used. Same NVIDIA driver (580.88).

1. **A** - `-cpu host,-vmx` (the documented user configuration), QEMU = distro `/usr/bin/qemu-system-x86_64`
   (Ubuntu `1:10.2.1+ds-1ubuntu3.2`). One boot, up to 5 cycles.
2. **B** - `-cpu host,-vmx,hv-relaxed,hv-vapic,hv-spinlocks=0x1fff,hv-time,hv-vpindex,hv-synic,hv-stimer,hv-crash`
   (the gate's flags, `win_vm.sh cpu_flags B`), same distro QEMU. A cycle that storms ends its boot (the guest cannot
   come back); a new boot is started for the next cycle; up to 5 cycles in total, at most 5 boots.
3. **B+P** - arm B's flags, QEMU 10.2.4 built on the pc from the pinned tarball (`scripts/bench/qemu.pin`, sha256
   `821b545b...0746` checked) with the optional `hyperv-wake` patch (branch `claude/optional-wake-patch-20261124`,
   `tools/qemu-patches/hyperv-wake.patch`, identical in effect to `WAKE_NEW` of `scripts/release/build_qemu.py`) and no
   kf3 source. One boot, up to 5 cycles.
4. **B0** (in the same bind, if the stop-new-boots time allows; it is judged only if B storms and B+P is clean) - arm
   B's flags on the same 10.2.4 build WITHOUT the patch (same configure line), up to 2 cycles: separates the patch from
   the 10.2.1 vs 10.2.4 difference. Binaries (built 2026-11-24 on the pc by `build_qemu_pair.sh`): stock 10.2.4 sha256
   `1398ac7f44d468f9...`, patched `5164ee64968a9c53...`; neither lists a kf3 device in `-device help`.

Each cycle: the suspend is requested by a scheduled task in the signed-in user's interactive session
(`rundll32.exe powrprof.dll,SetSuspendState 0,1,0`, as `s3probe.py` of 2026-11-23); QMP `query-status` must read
`suspended`; then QMP `system_wakeup`. 30 s between cycles in one boot.

## Definitions (fixed before the run)

- **STORM**: after `system_wakeup` the guest's ssh does not answer within 150 s, and the host trace of the first 4 s
  after the wake shows a `kvm_hv_stimer_start_one_shot` with `(count - time_now) mod 2^64 >= 2^64/100` re-armed at
  least 10,000 times (the R1t signature), or (if the trace misses it) the 2 s exit sample after 60 s shows more than
  100,000 exits/s with ssh dead.
- **CLEAN**: QMP `suspended` within 120 s of the request; ssh answers within 150 s of the wake with the SAME
  `LastBootUpTime`; no System event 41, 1001 or 6008 since the suspend request; `nvidia-smi` in the guest names the
  RTX 4070; `explorer.exe` runs in the interactive session after the wake.
- **FAIL-OTHER**: neither (no suspend, a new boot, a bugcheck, a dead guest without the storm signature, ...).
- **Event 153**: `nvlddmkm` (provider `nvlddmkm`) System event 153 counted from the suspend request until 150 s after
  ssh returns (kf3 had three ~34-70 s after every wake before #575). Recorded per cycle; not part of CLEAN (as in #572).

## Hypotheses and falsifiers

- **HV1 (the storm is a QEMU-wake artifact, not kayfabe's).** Arm B on stock QEMU with a passed-through GPU storms.
  **Falsifier (owner's, stated 2026-11-24):** arm B wakes CLEAN in at least 3 cycles on this stable-TSC host. Then the
  premise is wrong and the storm is kayfabe-caused or kayfabe-specific. A mixed result (some STORM, some CLEAN) is
  reported as such; one STORM with the R1t signature is enough to show the mechanism exists without kayfabe.
- **HV2 (the documented flags are safe).** Arm A wakes CLEAN. *Falsifier:* any arm-A cycle is STORM or FAIL-OTHER.
- **HV3 (the patch fixes it without kayfabe).** Arm B+P wakes CLEAN. *Falsifier:* a B+P cycle is STORM.
- **HV4 (mechanism as in R1t).** In a B storm the TSC page GPA read with `xp` is byte-identical before suspend, while
  suspended and after the wake; the wake moves `kvm_write_tsc_offset` by about minus the guest's uptime; the guest
  writes no `0x40000021`/`0x40000000`/`0x40000001` after the wake. *Falsifier:* the page changes across the wake, or the
  guest re-writes `REFERENCE_TSC` with the enable bit after the wake.
- **HV5 (post-wake TDR).** Open question, no prediction: does the real 4070 under VFIO log `nvlddmkm` 153 after a
  clean wake? kf3 did (3 per wake) before #575 and not after. A VFIO 153 would say the TDR is (at least partly) the
  driver's or QEMU's wake; zero is the expected bare behaviour [inferred].

## Host evidence (per cycle)

tracefs (`kvm_msr` filtered to Hyper-V MSR writes, `kvm_write_tsc_offset`, `kvm_hv_synic_set_msr`,
`kvm_hv_stimer_*`, `kvm_hv_hypercall`), the TSC page through HMP `xp`, KVM debugfs exit rates, filtered host `dmesg`
(vfio, AER, NVRM, kvm), guest System events 41/1001/6008/1074/153/14/4101, `nvidia-smi` in the guest.

## Stop rules and the host

- Restore by 16:00Z at the latest (owner window to 16:30Z): stop starting new boots at 15:15Z.
- A bind ABORT (a module in use, a D-state child) stops the session with the restore of `pcsession.py` (the 2026-11-18
  safe bind, 24 tests); a wedge goes to the SysRq s,u,b ladder (memory note of 2026-11-18), never a normal reboot.
- After the restore: nvidia 595.91.07 bound to 01:00.0, `snd_hda_intel` on 01:00.1, gdm active with the monitor on
  `card*-DP-3` connected, `nvidia-smi` answers, `nvidia_uvm` live (the settle wait of `pcsession.py`).
