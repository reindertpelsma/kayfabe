# Win11 S3 wake on kf3: hypotheses and falsifiers, written before the run (2026-11-23)

**STATUS: RESEARCH, 2026-11-23. Written and committed BEFORE any hardware run of this directory.** Results go in
`README.md` beside it; this file is not edited after the first run except by a dated addendum at the end.

Baseline (measured 2026-10-14, kf3 `0c5fedff`, trusted host, i440FX + OVMF, `traces/win11_s3_20261014/README.md`):
0 clean cycles in 7. With `hv_time,hv_synic,hv_stimer` every vCPU idles in HLT after the wake, ~740k HLT exits/s,
`kvm_hv_stimer_start_one_shot count=18446744073708147579` (-1404037 as s64) re-armed every ~13 us, no device touched.
Without them: a resume and a self-reset 62 s later, a bugcheck 0x5C, three PM1_STS/timer spins. Never run: q35.

## Phase 1, GPU-free (read, not executed)

Read: QEMU 10.2.4 (`scripts/bench/qemu.pin`, tarball sha256-verified through `scripts/ci/qemu_source.py`):
`target/i386/cpu.h`, `target/i386/kvm/kvm.c`, `hw/i386/pc.c`; Linux KVM `arch/x86/kvm/hyperv.c`, `x86.c` (from
memory of mainline 5.15..6.x; the box kernel is named in the README after the run).

- [read] `pc_machine_wakeup` resets every device, the CPUs included, with `RESET_TYPE_WAKEUP`, then
  `cpu_synchronize_all_post_reset` (KVM_PUT_RESET_STATE).
- [read] Every Hyper-V MSR field of `CPUX86State` (`msr_hv_hypercall`, `_guest_os_id`, `_tsc`, `_vapic`, `_synic_*`,
  `_stimer_config/count`, ...) sits before `end_reset_fields`, so the reset zeroes it (SINTs are set back to masked by
  `kvm_arch_reset_vcpu`). `kvm_put_msrs` at `level >= KVM_PUT_RESET_STATE` writes them all to KVM: on the first vCPU
  `HV_X64_MSR_GUEST_OS_ID = 0`, `HV_X64_MSR_HYPERCALL = 0`, `HV_X64_MSR_REFERENCE_TSC = 0` (the TSC page disabled); on
  every vCPU `SCONTROL/SIMP/SIEFP = 0`, SINTs masked, `STIMERn_CONFIG/COUNT = 0`, `APIC_ASSIST_PAGE = 0`.
  A real hypervisor does not sleep with its guest: on a guest S3 its partition state would persist. Hyper-V itself never
  offers S3 to a guest, so Windows' S3 resume path under enlightenments has no native reference.
- [read] `MSR_IA32_TSC = env->tsc = 0` is written too, but KVM treats a host-initiated TSC write of 0 as
  "synchronize" (`kvm_synchronize_tsc`: `data == 0` -> match `cur_tsc_offset`), so the guest TSC is NOT reset to 0 on
  a stable-TSC host; it keeps counting. This contradicts the 2026-10-14 inference "TSC to 0" (that line was inferred).
- [read] KVM `stimer_start` (one-shot): if `time_now >= count` the timer expires at once; else
  `hrtimer_start(ktime_now + 100 * (count - time_now))`. For `count = 2^64 - 1404037` the product wraps, so the hrtimer is
  in the past and fires at once; `kvm_hv_process_stimers` then compares `time_now >= exp_time` as u64 (false), does not
  expire, and calls `stimer_start` again: an endless host loop that never delivers the expiry to the guest. The guest
  sleeps in HLT waiting for a timer interrupt that never comes. KVM only amplifies; the cause is the guest writing an
  absolute deadline that is "negative", i.e. the guest's reference time after the wake disagrees with KVM's.

## Hypotheses (ranked), each with its falsifier

Arm B = `host,-vmx,hv-relaxed,hv-vapic,hv-spinlocks=0x1fff,hv-time,hv-vpindex,hv-synic,hv-stimer,hv-crash` (the
Windows gate's flags, `win_vm.sh cpu_flags B`). Arm A = `host,-vmx` (the documented user configuration's CPU).

- **H1 (the storm mechanism is KVM's wrapped one-shot).** During the storm the trace shows `kvm_hv_stimer_start_one_shot`
  with `count - time_now >= 2^64/100` (u64) and no successful `kvm_hv_stimer_expiration` for that timer.
  *Falsifier:* `count - time_now < 2^64/100` in the storm's events, or expirations with `msg_send_ret 0` for it.
- **H2 (the wake reset wipes partition-wide Hyper-V state that Windows does not re-establish).** After the wake the
  guest writes neither `HV_X64_MSR_REFERENCE_TSC` (0x40000021) nor `HV_X64_MSR_GUEST_OS_ID`/`HYPERCALL`
  (0x40000000/1), so KVM's TSC page stays disabled and the guest keeps reading the stale page in its RAM.
  *Falsifier:* a guest write of 0x40000021 with the enable bit (and of 0x40000000/1) between `system_wakeup` and the first
  `kvm_hv_stimer_start_one_shot` of the storm.
- **H3 (the per-VP SynIC/stimer state is wiped and Windows re-arms the timer without re-enabling the SynIC).**
  *Falsifier:* guest writes of `SCONTROL` (0x40000080) with the enable bit, `SIMP` (0x40000083) and the timer's `SINT` on
  every vCPU after the wake, before the storm starts.
- **H4 (the guest's reference time after the wake is behind KVM's).** The first post-wake one-shot has a "negative"
  `count` while `time_now` is about the VM's uptime in 100 ns units; the TSC page in guest RAM (read with `xp` at the GPA
  the guest gave 0x40000021 at boot) is unchanged across the wake.
  *Falsifier:* the first post-wake `count` is in `[time_now, time_now + 10 s]`, or the page content changes across the wake.
- **H5 (the TSC is not reset by the wake).** *Falsifier:* `kvm_write_tsc_offset` at the wake moves the offset so that the
  guest TSC restarts near 0 (offset change of about minus the pre-suspend guest TSC).
- **H6 (q35 control, arm A).** The 2026-10-14 arm-A failures (0x5C, PM1_STS spins) were i440FX; on q35 + arm A a cycle
  may be clean. Not a mechanism claim. *Falsifier:* a q35 arm-A cycle fails (no desktop/ssh within 150 s, a new boot,
  a bugcheck event, or a cold kf3 reset).

## Fix candidate (only if H2 or H3 holds)

**F1:** keep the Hyper-V synthetic state across `RESET_TYPE_WAKEUP` in QEMU (partition-wide and per-VP MSRs, not the
architectural CPU state): what a hypervisor that does not sleep with its guest would do. Prediction: arm B with F1
survives the cycle. *Falsifier:* with F1 the first post-wake `count` is still outside `[time_now, time_now + 10 s]`, or
the guest still does not return within 150 s.

## Definition of a CLEAN cycle (fixed before the run)

QMP `query-status` reaches `suspended` within 120 s of the suspend request; after `system_wakeup` the guest's ssh answers
within 150 s with the SAME `LastBootUpTime` as before the suspend; kf3's log gains `(S3 WAKE) ... KEPT` and at least one
`GSP-RESUME` and no `device reset #N: vCPUs out` (cold reset); `nvidia-smi` in the guest names the GPU; no event 41/1001
in the guest's System log for the cycle window.

## Run plan and stop rule (time-box: 3 box-hours, 5 USD, ONE box)

1. `windows_lane.sh` then `windows_gate.sh` (arm B) on a fresh 24 GB-class box: a sealed Windows 11 with the 580.88
   driver on kf3 (the gate's overlay). If the gate does not PASS, stop: no S3 claim on a red box.
2. R1: arm B, q35, `-global ICH9-LPC.disable_s3=0` (the harness otherwise disables S3), 1 cycle, traced (`s3probe.py`).
   Judges H1-H5.
3. R2: arm A, same overlay family, 1-3 cycles. Judges H6.
4. R3 (only if H2 or H3 holds and box time remains): F1 built on the box, arm B, up to 3 cycles.
5. Stop, compare, write the README. No unplanned walls in this lease.

## Addendum 1 (2026-11-23, before any run; the box was rented and is provisioning)

- **R2b (only if box time remains after R1-R3):** arm B without `hv-stimer` (`hv-time`, `hv-synic` and the rest kept), via
  a later `-cpu` in the wrapper. Windows then takes its clock interrupts from the LAPIC timer while still reading the TSC
  page. *Prediction if H2/H4 hold through the stimer only:* the cycle is clean. *Falsifier of "the stimer is the only
  casualty":* the R2b cycle fails.
- Order of runs: R1, then R3 if its condition holds, then R2, then R2b.
- [read, KVM] the 13 us re-arm is KVM's own loop (`kvm_hv_process_stimers` -> `stimer_start`), not the guest: a single
  guest write of `count = -1404037` explains the whole storm, so the guest's deadline is a fixed value 0.14 s "before
  reference time zero".

## Addendum 2 (2026-11-23 ~04:28Z, after R1, BEFORE the outcome of R1t is read)

- [measured R1, box 62082978, kf3 `1052e2f9`, arm B, q35, stock QEMU] **1/1 CLEAN.** The box's L1 kernel marked its TSC
  unstable (`tsc: Marking TSC unstable due to TSCs unsynchronized`, clocksource kvm-clock); the guest's TSC page at GPA
  0xd000 read all zeros before the suspend, while suspended and after the wake (KVM publishes sequence 0 = invalid when it
  has no master clock), so Windows read the reference time through the `TIME_REF_COUNT` MSR, which is always KVM's.
  First post-wake one-shots: `count - time_now` = 4775 and 14663 (0.5 and 1.5 ms). After the wake Windows re-wrote `SIMP`
  and `SINT3` on all 8 vCPUs (H3's falsifier hit for SIMP/SINT; `SCONTROL` was not in the filtered writes).
- New, sharper form of H2/H4 (**H2'**): the storm needs a VALID TSC page before the suspend. The wake reset writes
  `REFERENCE_TSC = 0`, KVM stops maintaining the page, the guest keeps reading the stale page in its RAM, and its
  reference time diverges from KVM's `TIME_REF_COUNT` fallback used by the stimers.
- **R1t:** the same box rebooted with `tsc=reliable clocksource=tsc` (L1 clocksource tsc, so KVM can keep a master clock
  and publish a valid page). Prediction under H2': the TSC page reads non-zero before the suspend, and after the wake the
  cycle fails with a post-wake one-shot outside `[time_now, time_now + 10 s]` (or a hang). *Falsifier of H2':* R1t with a
  valid page before the suspend is CLEAN. If the page is still all zero, R1t says nothing about H2' (stated as such).
- **R3 (F1) runs after R1t** under the same boot if R1t fails with a valid page: prediction CLEAN.
