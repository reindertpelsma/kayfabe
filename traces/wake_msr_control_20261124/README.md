# S3 wake and the Hyper-V MSRs on a plain Linux guest (no kayfabe device, no GPU), 2026-11-24

**STATUS: RESEARCH, 2026-11-24. H1 CONFIRMED; H2 half confirmed: the wipe is there, the preregistered symptoms are
absent for Linux (3/3), plus one latent effect. Measured on one box.** Hypotheses and falsifiers were committed before
the runs: `PREREGISTRATION.md`, with two dated addenda that change the method, not the claims. Measured = a run named here;
inferred = marked as such.

Run: Vast instance 62265107 (AMD Ryzen 7 1800X, 7 vCPUs, 24 GB; nested KVM; L1 kernel 6.8.0-59-generic, `kvm_amd`
nested=1). The L1 marked its TSC unstable ("TSCs unsynchronized"), so the box was rebooted with
`tsc=reliable clocksource=tsc`, as `traces/win11_s3_wake_20261123/` did. After that, `current_clocksource` was `tsc`
(`box/host_info.txt`). Branch `claude/wake-msr-control-20261124`, probe as committed in `360b76de`; the
`no-os-id` seed option (K2 rows) is the one change after that, committed with this README. QEMU: the tarball pinned by
`scripts/bench/qemu.pin` (10.2.4, fetched through `scripts/ci/qemu_source.py`), configured with
`scripts/release/build_qemu.py` `CONFIGURE_FLAGS` and no kf3 device. Two builds: **stock**
(`qemu-system-x86_64` sha256 `9831133937e72efb...`) and **patched** = `build_qemu.py wake-hunk` (`31f27cdd3c674794...`).
The branch `claude/optional-wake-patch-20261124` did not exist; "patched" is the `WAKE_NEW` text. Guest: Ubuntu 24.04
cloud image (sha256 `04e8f246...`, kernel 6.8.0-147-generic), q35 `accel=kvm,smm=on`, OVMF `OVMF_CODE_4M.secboot.fd`
(Ubuntu 22.04 `ovmf` 2022.02-3ubuntu0.22.04.7), 2 vCPUs, 1 GiB, `-global ICH9-LPC.disable_s3=0`, guest kernel
`mem=960M mem_sleep_default=deep`. The guest suspends itself (`echo mem > /sys/power/state`) and the host sends QMP
`system_wakeup` 5 s after `query-status` reports `suspended`. Cost: about 0.09 USD (0.111 USD/h, 00:22Z-01:11Z). The box
was destroyed by its id on 2026-11-24 and its absence was verified with `vastai show instances`; `vast_guard.py --enforce`
ran before and after.

## Results (measured 2026-11-24, box 62265107)

Every row ran 3 suspend/wake cycles, and all 3 reached `suspended`. Each MSR showed the same before/after pattern in all 3
cycles, so cycle 1 stands for the row (all cycles: `summary.txt`, `<row>/c<n>_pre.json`/`_post.json`).

| row | QEMU | CPU | seeded by the probe | GUEST_OS_ID 0x40000000 | HYPERCALL 0x40000001 | REFERENCE_TSC 0x40000021 | SCONTROL / SIEFP / SIMP (both vCPUs) | STIMER0_CONFIG / COUNT (both vCPUs) |
|---|---|---|---|---|---|---|---|---|
| K-stock | stock | arm B | all | 0x8100000000000001 -> **0** | (write #GP under OVMF, see below) | 0x3d001001 -> **0** | 1 / 0x3d002001 / 0x3d003001 -> **0 / 0 / 0** | 0x20000 / 0x12345678 -> **0 / 0** |
| K2-stock | stock | arm B | all but OS_ID/HYPERCALL | - | - | 0x3d001001 -> **0** | as above -> **0 / 0 / 0** | as above -> **0 / 0** |
| K2-patch | patched | arm B | all but OS_ID/HYPERCALL | - | - | 0x3d001001 -> 0x3d001001 | kept | kept |
| K-patch | patched | arm B | all | guest did not come back (below) | | | host wrote the kept values back | host wrote the kept values back |
| H-stock | stock | arm B + `kvm=off` | none (Linux runs as a Hyper-V guest) | 0x81800006080c0000 -> **0**, stays 0 (c2, c3 pre = 0) | 0x51a3001 -> **0x51a3000** (enable lost) | 0x4a04001 -> 0x4a04001 (Linux re-wrote it) | 0 (Linux enables no SynIC without VMBus) | 0 |
| H-patch | patched | arm B + `kvm=off` | none | kept | kept (0x51a6001) | kept (re-written too) | 0 | 0 |
| A-stock | stock | arm A `host,-vmx` | all (every write fails) | EIO before and after | EIO | EIO | EIO | EIO |

SeaBIOS cross-check (`seabios/`, cycle 1 of `K-stock-try1`, `-try2`, `-seabios-try3`): the same wipe, plus `HYPERCALL`
0x3d000001 -> 0x3d000000. [read, KVM `kvm_hv_set_msr_pw`] A write of `GUEST_OS_ID` = 0 clears the hypercall enable bit,
and KVM ignores a `HYPERCALL` write while `GUEST_OS_ID` is 0.

Host-side evidence (tracefs on the L1, `<row>/c<n>_trace.txt.gz`; `summary.txt` lists it per cycle):
- **K-stock, K2-stock, every cycle:** right after the `system_wakeup` marker, QEMU (`host 1`) wrote `SCONTROL=0`, `SIEFP=0`,
  `SIMP=0`, SINT0-15 = masked, `STIMER0-3_CONFIG=0` and `_COUNT=0` on both vCPUs (56 host writes per wake). After the wake
  and before the guest's read, the guest wrote **no** Hyper-V MSR (0 `kvm_msr` writes in 0x40000000-0x400001ff). The zeros
  the guest read are therefore QEMU's.
- **K-patch, K2-patch:** the same host writes carry the pre-suspend values (`SCONTROL=0x1`, `SIEFP=0x3d002001`,
  `STIMER0 count 305419896`, ...).
- **H-stock / H-patch, every cycle:** after the wake the Linux guest writes `VP_ASSIST` (0x40000073) on both vCPUs, then
  `HYPERCALL` (enable set) and `REFERENCE_TSC` (enable set). It never writes `GUEST_OS_ID`. On stock its `HYPERCALL`
  write does not take (read back without the enable bit).
- **T (TSC), all rows:** at every wake `kvm_write_tsc_offset` moves both vCPUs' offset by about minus the guest TSC
  accumulated since the previous reset (-97.3e9 cycles = -27.0 s at 3.6 GHz in cycle 1; about -61e9 = -17 s in cycles 2-3,
  the cycle spacing). [inferred, arithmetic] The guest TSC restarts near 0 at each wake. The wake hunk does not change this.
- No `kvm_hv_stimer_start_one_shot` after any wake in any row (no timer storm), and no guest kernel log line matching the
  preregistered symptom pattern.

Symptom checks for H2 (preregistered list; H-stock, 3/3 cycles). Back on ssh 6.3 s after `system_wakeup`. Clocksource
`hyperv_clocksource_tsc_page` before and after. Guest `CLOCK_MONOTONIC` +0.40-0.56 s across the cycle (suspended time is
excluded). `CLOCK_REALTIME` advanced by the same amount as `CLOCK_BOOTTIME` (difference 0.00 s). No symptom log lines,
no stimer events. **No preregistered symptom.**

Latent effect (not on the preregistered list, so reported apart): after the first wake on stock, the Linux Hyper-V guest
runs with `GUEST_OS_ID` = 0 and its hypercall page disabled, and it stays so (cycles 2 and 3 start from 0). [inferred,
from KVM's code] Any Hyper-V hypercall it makes afterwards goes to KVM's own hypercall handler and fails. With these flags
Linux uses no hypercall on its hot paths (no `hv-tlbflush`/`hv-ipi`, no VMBus), so nothing visible followed.

K-patch: the guest never answered ssh after its first wake. Its serial log shows `rcu_preempt detected stalls` on CPU 0
and "NMIs are not reaching exc_nmi()", and the host trace has 3 `kvm_hv_hypercall` events from vCPU 1 after the wake.
[inferred, from KVM's code; Addendum 2] The cause is the probe's own `GUEST_OS_ID` seed, which the wake hunk preserved.
While it is set, KVM routes the Linux KVM guest's PV `vmcall`s (IPIs, kicks) to its Hyper-V handler. A Linux guest that
picks KVM never writes `GUEST_OS_ID` itself. Not tested: whether the same guest stalls without a suspend when the seed is
left in place long enough. K2-patch, the same run without that seed, came back 3/3.

Not measured, and why: the content of the TSC page. Under OVMF the guest's `/dev/mem` read of the page died with SIGILL,
and HMP `xp` at GPA 0x3d001000 returned all-ones. The probe's `HYPERCALL` write at 0x3d000000 also got #GP under OVMF but
not under SeaBIOS. [inferred] Under OVMF those GPAs are not backed by a KVM memory slot; the reason was not looked at.

## Verdicts

- **H1: CONFIRMED** (K-stock 3/3, K2-stock 3/3, SeaBIOS 3 x cycle 1). Stock QEMU 10.2.4's S3 wake zeroes
  `REFERENCE_TSC`, `GUEST_OS_ID` (and with it the hypercall enable), `SCONTROL`, `SIEFP`, `SIMP` and `STIMER0_CONFIG/COUNT`
  on a guest with no kayfabe device. The guest reads zeros, and the host trace shows QEMU writing them. With the wake
  hunk they survive (K2-patch 3/3 read back, H-patch 3/3; K-patch host writes). Falsifier not met.
- **H2: wipe CONFIRMED, symptom FALSIFIED for Linux** (H-stock 3/3). On stock QEMU, Linux re-establishes `REFERENCE_TSC`,
  `HYPERCALL` and VP assist in its resume path, so its clock is fine. It does not re-write `GUEST_OS_ID`, so its hypercall
  enable is lost (latent; see above).
- **C (control): passes.** Arm A: every Hyper-V MSR access fails before and after. The probe reports no Hyper-V value
  without the flags, and the cycles complete. QEMU still writes `STIMER*` = 0 from the host at the wake (KVM accepts
  host writes of 0).
- **T:** the guest TSC restarts near 0 at every wake, on stock and patched alike (offsets measured 2026-11-24; "near 0"
  inferred).

## What it implies for the Windows case

[inferred] The wipe belongs to QEMU's wake path (`pc_machine_wakeup` -> CPU reset -> `KVM_PUT_RESET_STATE`), on any
guest with Hyper-V enlightenments. It is reproduced here with no kf3 device, no GPU and no kayfabe code in the QEMU binary,
so the 2026-11-23 Windows post-wake timer storm is not caused by kayfabe. Whether a guest survives the wipe depends on
what its resume path re-writes. Linux re-writes the TSC page MSR, so a reset TSC gives it a valid page again. Windows
(2026-11-23 R1t) re-writes `SIMP`/`SINT` but not `REFERENCE_TSC`, so it reads a stale page against a TSC that restarted
near 0, its deadlines land "before zero", and its vCPUs idle forever, on a stable-TSC host. The wake hunk is a QEMU-side
fix (a hypervisor does not sleep with its guest), not a kayfabe workaround. The pc VFIO control with a Windows guest
(separate) is the remaining check on stock QEMU without kayfabe.

## Files

`PREREGISTRATION.md` (claims, falsifiers, addenda 1-2). `guest_msr.py` (guest side). `wakeprobe.py` (host: QEMU,
QMP, tracefs). `box_setup.sh` (builds, image, seed). `analyze.py` -> `summary.txt` (every cycle, `python3 analyze.py .`).
Per row: `qemu_argv.json`, `result.json`, `c<n>_seed.json`, `c<n>_pre.json`/`_post.json`, `c<n>_dmesg.txt`,
`c<n>_trace.txt.gz`, `serial.log`, `guest_boot_info.txt`. `seabios/`: the SeaBIOS attempts (only cycle 1 of each
reaches `suspended`, Addendum 1); `dbg-A-seabios` shows the ACPI PM I/O regions missing after the wake. `debug/`: the
OVMF trials (`dbg-A-ovmf` blocked on the `/dev/mem` read, `dbg-A-ovmf2` 2/2). `box/`: host facts (`host_info.txt`, root
PARTUUID scrubbed) and the run logs.

Probe errata: `K-stock-try1`/`-try2` launched the guest helper with output to `/dev/null` and, in try1, did not save the
helper's files from a cycle that did not suspend; fixed before `-seabios-try3`. The `trace_counts` in `result.json` count
all events of the cycle, the pre-suspend seed writes included; the per-wake facts are in `summary.txt`.
