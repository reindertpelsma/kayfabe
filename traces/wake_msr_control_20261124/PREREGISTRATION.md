# S3 wake and the Hyper-V MSRs on a plain Linux guest: hypotheses and falsifiers, written before the run (2026-11-24)

**STATUS: RESEARCH, 2026-11-24. Written and committed BEFORE any run of this directory.** Results go in `README.md` beside
it; this file is not edited after the first run except by a dated addendum at the end.

Question (owner, 2026-11-24): does QEMU's own S3 wake wipe the Hyper-V synthetic MSRs on a guest that has nothing to do with
kayfabe (no kf3 device, no GPU)? If yes, the Windows post-wake timer storm (`traces/win11_s3_wake_20261123/README.md`) is a
QEMU/KVM behaviour that kayfabe only met, not a kayfabe bug.

## What the source says (read 2026-11-24, not executed)

QEMU 10.2.4, the tarball pinned by `scripts/bench/qemu.pin` (sha256 checked here, `821b545b...`):

- [read] `hw/i386/pc.c` `pc_machine_wakeup`: `cpu_synchronize_all_states(); pc_machine_reset(machine, RESET_TYPE_WAKEUP);
  cpu_synchronize_all_post_reset();`, i.e. a CPU reset of every vCPU and then a `KVM_PUT_RESET_STATE` write to KVM.
- [read] `target/i386/cpu.h`: every `msr_hv_*` field (`msr_hv_hypercall` at line 1984 to `msr_hv_tsc_emulation_status` at
  2006) lies before `end_reset_fields` (2062), so the reset zeroes it.
- [read] `target/i386/kvm/kvm.c` `kvm_put_msrs`, block `if (level >= KVM_PUT_RESET_STATE)` (line 4019): writes
  `HV_X64_MSR_GUEST_OS_ID`, `HYPERCALL`, `REFERENCE_TSC` (first vCPU only), and on every vCPU `SCONTROL`, `SIEFP`, `SIMP`,
  the SINTs and `STIMERn_CONFIG/COUNT`, from those zeroed fields; `MSR_IA32_TSC = env->tsc` (0) too.

So the code predicts the wipe. This run measures it.

## Variants (one Vast KVM box, nested; one variant at a time, 3 suspend/wake cycles each)

Guest: Ubuntu 24.04 cloud image (distribution kernel), q35, `accel=kvm,smm=on`, SeaBIOS, 2 vCPUs, 1 GiB, kernel command line
`mem=960M` so the guest pages at 0x3D000000.. are guest-physical RAM the kernel never uses (the pages the probe hands to
KVM below). S3 enabled with `-global ICH9-LPC.disable_s3=0` (the Windows arm's machine options otherwise: `smm=on`,
`ICH9-LPC.disable_s4=1`, `kvm-pit.lost_tick_policy=discard`, `-rtc base=utc`). Deviation from the Windows arm, stated: SeaBIOS
instead of OVMF and no TPM (the wake path in QEMU, `pc_machine_wakeup`, does not depend on the firmware; the guest's resume
does).

Arm B = `host,-vmx,hv-relaxed,hv-vapic,hv-spinlocks=0x1fff,hv-time,hv-vpindex,hv-synic,hv-stimer,hv-crash`, exactly
`scripts/bench/windows/win_vm.sh` `cpu_flags B`. Arm A = `host,-vmx`.

| id | QEMU | CPU | guest kernel's view | who writes the Hyper-V MSRs |
|---|---|---|---|---|
| K-stock | stock 10.2.4 | arm B | KVM guest (Linux picks KVM: the KVM leaves at 0x40000100 win) | only the probe, before the suspend |
| K-patch | 10.2.4 + `build_qemu.py wake-hunk` | arm B | as above | as above |
| A-stock | stock 10.2.4 | arm A (control) | KVM guest, no Hyper-V | nobody (the writes must #GP) |
| H-stock | stock 10.2.4 | arm B + `kvm=off` | Hyper-V guest (hyperv clocksource, hypercall page) | the guest kernel, including its resume hooks |
| H-patch | patched | arm B + `kvm=off` | as above | as above |

The optional-patch branch `claude/optional-wake-patch-20261124` did not exist when this was written; "patched" is
`scripts/release/build_qemu.py wake-hunk` (the `WAKE_NEW` text) on the same tarball, nothing else changed.

In K-*, the probe writes from the guest (`/dev/cpu/N/msr`, the `msr` module) before the first suspend: on CPU 0
`GUEST_OS_ID (0x40000000) = 0x8100000000000001`, `HYPERCALL (0x40000001) = 0x3D000001`, `REFERENCE_TSC (0x40000021) =
0x3D001001`; on every CPU `SCONTROL (0x40000080) = 1`, `SIEFP (0x40000082) = (0x3D002000 + cpu*0x2000) | 1`,
`SIMP (0x40000083) = (0x3D003000 + cpu*0x2000) | 1`, `STIMER0_COUNT (0x400000B1) = 0x12345678 + cpu`, `STIMER0_CONFIG (0x400000B0) = 0x20000` (SINT 2,
not enabled, so no timer runs). Linux as a KVM guest has no code that touches these MSRs, so any change across the wake is
the hypervisor's. The process that writes `mem` to `/sys/power/state` reads them again the moment that write returns.

Host side, independent of the guest's timing (tracefs on the box): `kvm_hv_synic_set_msr` and `kvm_hv_stimer_set_config` /
`_set_count` (they carry `host 1` for a write by QEMU), `kvm_msr` filtered to 0x40000000-0x400001ff (guest reads and writes,
with values), `kvm_write_tsc_offset`, and the `kvm_hv_stimer_*` timer events. There is no tracepoint for a host write of
`REFERENCE_TSC`/`GUEST_OS_ID`/`HYPERCALL`; for those the evidence is KVM's answer to the guest's read (`kvm_msr` shows the
value KVM returned) plus the absence of a guest write between the wake and that read.

## Hypotheses, each with its falsifier

- **H1 (stock QEMU's wake zeroes the Hyper-V MSRs; the wake hunk keeps them).** K-stock: after each wake every MSR listed
  above reads 0 (SINTs back to masked), and the trace shows QEMU's host writes of 0 to `SCONTROL`/`SIEFP`/`SIMP` and
  `STIMER0_CONFIG/COUNT` on every vCPU at the wake. K-patch: every MSR reads its pre-suspend value.
  *Falsifier:* in K-stock any listed MSR keeps its pre-suspend non-zero value after a wake (then the premise "the wake wipes
  them" is wrong for this QEMU/KVM pair); or in K-patch any of them reads 0 after a wake (then the hunk does not do what it
  claims).
- **H2 (with the Windows gate's flags a Linux guest shows the wipe AND a visible symptom on a stable-TSC host).**
  H-stock: the host trace shows the zero writes at the wake, and at least one symptom: the guest is not back on ssh within
  90 s; or the clocksource differs from before the suspend; or the guest `CLOCK_MONOTONIC` advanced by more than the host's
  monotonic time across the cycle plus 2 s, or went backwards; or the guest `CLOCK_REALTIME - host realtime` changed by
  more than 2 s; or the guest kernel log of the cycle has a line matching `clocksource|watchdog|TSC.*unstable|soft lockup|
  rcu.*stall|hrtimer: interrupt took`; or more than 10000 `kvm_hv_stimer_start_one_shot` events in a 1 s window.
  *Falsifier:* the zero writes are absent (H2 fails with H1), or H-stock shows none of these symptoms in 3 of 3 cycles (then
  the wipe is harmless for Linux; Linux re-writes what it needs on resume, which the trace will show).
  *Expectation, written down so it can be wrong:* Linux re-arms its Hyper-V state in its own resume hooks (`hv_resume`,
  the hyperv clocksource resume), so H2's symptom half may well be falsified. That would mean the wipe is QEMU's for every
  guest, and the hang is the interaction of the wipe with a guest (Windows) that does not re-write `REFERENCE_TSC` on S3.
- **C (control, A-stock).** Without Hyper-V flags the probe's writes fail (#GP, `EIO` from `/dev/cpu/N/msr`) and every read
  fails the same way before and after; the cycles complete. *Falsifier:* the probe reports a Hyper-V MSR value on arm A
  (the probe is broken; then no other row counts).
- **T (the TSC at the wake; re-measures 2026-11-23 H5 on a Linux guest).** Reported, not ranked: the change of
  `kvm_write_tsc_offset` at each wake in cycles of guest TSC, against minus the guest's TSC before the suspend.

## Rules for the run

Stable host TSC required (`current_clocksource` = `tsc`, no "TSC unstable" in `dmesg`); if not, reboot the box with
`tsc=reliable clocksource=tsc` as `traces/win11_s3_wake_20261123/` did, and say so. A cycle whose guest does not reach
`suspended` within 60 s is reported as "no suspend" and does not count for or against H1. Measured and inferred are kept
apart in the README of the 2026-11-24 run. Cap: 1.5 USD and 2 box-hours.

## Addendum 1 (2026-11-24 ~00:57Z, written before the OVMF runs' results were read)

Changes to the method, with the reason. The hypotheses and falsifiers above are unchanged.

- **Firmware: OVMF instead of SeaBIOS** (`OVMF_CODE_4M.secboot.fd` + `OVMF_VARS_4M.fd`, `cfi.pflash01 secure=on`, as
  `win_vm.sh` uses). With SeaBIOS + `smm=on`, only the first S3 of a boot reaches QEMU: after the wake QEMU's memory map
  has no `acpi-evt`/`acpi-cnt`/`acpi-tmr` I/O regions any more (HMP `info mtree -f`, run `dbg-A-seabios`, arm A, so not
  Hyper-V related): the wake reset disables the ICH9 PMBASE and SeaBIOS's resume path does not restore it, so the guest's
  second `SLP_EN` write goes nowhere and Linux "resumes" in 12 ms. OVMF restores it (run `dbg-A-ovmf2`: 2/2 cycles
  reached `suspended`, regions present after each wake). Disclosed: the SeaBIOS runs `K-stock-try2`/`-seabios-try3`
  already showed cycle 1's values before this addendum; they are kept in the evidence and reported as such.
- **The K probe re-seeds the MSRs before every cycle** (before: once after boot), since a wipe in cycle 1 would leave
  nothing to wipe in cycle 2.
- **The TSC page is read by the host** (HMP `xp /3gx` at the page's GPA, before the suspend, while suspended, after the
  wake): under OVMF the guest's `/dev/mem` read of that page dies with SIGILL, so the guest-side copy is reported as an
  error there. For H-* the GPA is the guest kernel's last `REFERENCE_TSC` write in the boot trace.

## Addendum 2 (2026-11-24 ~01:05Z, written after K-stock/K-patch/A-stock/H-* had run, before K2-* ran)

K-patch's guest never came back after its first wake (CPU 0 RCU stall; the host trace shows `kvm_hv_hypercall` events
from vCPU 1 after the wake). [inferred, from KVM's code] With `GUEST_OS_ID` non-zero KVM treats every `vmcall` of the guest
as a Hyper-V hypercall, and a Linux KVM guest issues KVM PV hypercalls (IPIs, kicks); the probe's own `GUEST_OS_ID` seed,
which the wake hunk keeps, is what breaks such a guest, not the hunk. Two variants are added to answer H1 for every other
MSR with a guest that survives: **K2-stock / K2-patch** = K-stock / K-patch with the seed minus `GUEST_OS_ID` and
`HYPERCALL` (`guest_msr.py seed no-os-id`). Same falsifiers as H1 for the MSRs seeded.

## Addendum 3 (2026-11-24, editorial, after the runs)

The last sentence of "Rules for the run" gained the words "of the 2026-11-24 run" so the claim-ledger gate
(`scripts/ci/debt.py claims`) finds a date in that block. Nothing else in this file changed after the runs.
