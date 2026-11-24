# VFIO control of the Windows S3 wake storm (2026-11-24)

**STATUS: RESEARCH, 2026-11-24. MEASURED on the owner's pc (bare metal, stable TSC) with a real RTX 4070 passed through
(vfio-pci), stock QEMU, no kf3 device: arm B storms 5 of 5 (and 2 of 2 on stock 10.2.4), arm A wakes clean 5 of 5, arm B
with the `hyperv-wake` patch wakes clean 5 of 5; no `nvlddmkm` 153 in any clean wake.** Hypotheses, definitions and the
falsifier were committed before the run (`PREREGISTRATION.md`, commit `cda273be`). Measured = a file named here; inferred
= said so.

Run: 2026-11-24 00:54Z-02:10Z, pc (AMD Ryzen 9 7900, kernel 7.0.0-34-generic, clocksource `tsc`, no `TSC unstable`,
host driver 595.91.07), GPU 01:00.0 + 01:00.1 (IOMMU group 11) on vfio-pci through the safe bind of the 2026-11-18
session (`pcsession.py`, unchanged copy). Repository revision of the tools: `cda273be` (branch
`claude/vfio-s3-control-20261124`). Guest: the sealed, non-proprietary `win11_base` image (Windows 11 Pro 25H2, NVIDIA
580.88, no applications; one throwaway overlay per boot over a prep overlay whose only changes are hibernation off and
standby/monitor timeouts 0: autologon was already on), q35 `pc-q35-10.2`, 8 vCPUs, 8 GiB, OVMF secure boot + swtpm,
offline slirp, `-global ICH9-LPC.disable_s3=0`. Sleep: a scheduled task in the signed-in session
(`SetSuspendState 0,1,0`), wake: QMP `system_wakeup` (`s3vfio.py`, which reuses the 2026-11-23 probe's trace helpers).

QEMU binaries: arms A and B the distro `/usr/bin/qemu-system-x86_64` (Ubuntu `1:10.2.1+ds-1ubuntu3.2`, no kf3 code); B+P
and B0 built on the pc from the pinned 10.2.4 tarball (`build_qemu_pair.sh`, sha256 checked against
`scripts/bench/qemu.pin`), one with `hyperv-wake.patch` (sha256 of the binary `5164ee64968a9c53...`), one without
(`1398ac7f44d468f9...`); `-device help` of all three lists `vfio-pci` and no kf3 device; every argv has no kf3 device
(`run/*/argv_scrubbed.txt`).

## Results (measured 2026-11-24)

| arm | CPU flags | QEMU | cycles | verdict | ssh back after wake | post-wake `nvlddmkm` 153 |
|---|---|---|---|---|---|---|
| A | `host,-vmx` | distro 10.2.1 | 5 in one boot | **5/5 CLEAN** | 7-14 s | 0 in each |
| B | `host,-vmx,hv-relaxed,hv-vapic,hv-spinlocks=0x1fff,hv-time,hv-vpindex,hv-synic,hv-stimer,hv-crash` | distro 10.2.1 | 5, one boot each | **5/5 STORM** | never (150 s) | n/a (guest dead) |
| B+P | as B | 10.2.4 + `hyperv-wake` | 5 in one boot | **5/5 CLEAN** | 8-15 s | 0 in each |
| B0 | as B | 10.2.4, no patch | 2, one boot each | **2/2 STORM** | never | n/a |

Per cycle (`summarize_run.py run`, from `run/*/c*_result.json`):

| arm | boot | cycle | verdict | exits/s 60 s after the wake | TSC page across the wake | guest TSC offset change at the wake (cycles) |
|---|---|---|---|---|---|---|
| A | A_b1 | 1-5 | CLEAN | - | no page (no `hv-time`) | -465e9, -1200e9, -1955e9, -2690e9, -3430e9 |
| B | B_b1..B_b5 | 1..5 | STORM | 4474207, 4455102, 5356967, 1022391, 3627290 | byte-identical before, while suspended, after | -453e9 .. -457e9 |
| B+P | BP_b1 | 1 | CLEAN | - | changed (KVM republished it) | -457e9 |
| B+P | BP_b1 | 2-5 | CLEAN | - | not read (the boot trace naming its GPA is cleared by cycle 1, as in 2026-11-23) | -739e9, -759e9, -736e9, -722e9 |
| B0 | B0_b1, B0_b2 | 1, 2 | STORM | 5376012, 1017600 | byte-identical | -454e9, -453e9 |

Every CLEAN cycle: QMP `suspended` within 2-3 s, same `LastBootUpTime` after the wake, `nvidia-smi` in the guest
`NVIDIA GeForce RTX 4070, 580.88`, `explorer.exe` in the interactive session, System events only 42 (entering sleep),
107 (resumed), 1 (time change / returned from low power): no 41, 1001, 6008, 14, 4101 or 153 between the suspend request and
150 s after ssh returned. Every STORM cycle: the guest never answered ssh; 60 s after the wake the vCPUs exited 1.0-5.4
million times per second, exit reasons `write_cr8`, `msr`, `vintr`, `hlt` (`run/*/c*_wedge_sample.json`), the R1t exit mix
of 2026-11-23.

Post-wake Hyper-V timer deadlines (`storm_deadlines.py run`, first ~0.75-1.8 s of `kvm_hv_stimer_*` after the wake):

| arms | guest one-shot deadlines written (absolute, s) | KVM reference time (s) | expiries in the window |
|---|---|---|---|
| B (5 cycles), B0 (2) | 3.99-4.12 | 126.5-127.6 | 6465-6627 in ~0.75 s, every one delivered (`send result 0`) |
| B+P (5) | time_now + 0.003 .. + 1.1 | 127.6, 327.2, 532.5, 731.4, 926.5 | 4148-4493 in ~1.75 s |

## Verdicts

- **HV1 (the storm is a QEMU-wake artifact, not kayfabe's): CONFIRMED.** Stock QEMU (distro 10.2.1 and upstream 10.2.4),
  a real RTX 4070 through vfio-pci, no kf3 device, stable host TSC: 7 of 7 arm-B wakes storm. The owner's falsifier (arm B
  clean in at least 3 cycles) did not occur: 0 clean of 7.
- **HV2 (the documented `host,-vmx` flags are safe): not falsified**, 5/5 clean.
- **HV3 (the patch fixes it without kayfabe): CONFIRMED** 5/5 clean, against B0 2/2 storm on the same 10.2.4 source and
  configure line, so the difference is the patch, not 10.2.1 versus 10.2.4.
- **HV4 (mechanism as in R1t): CONFIRMED for the root cause, with a different KVM symptom.** As in R1t: the TSC page is
  byte-identical before the suspend, while suspended and after the wake; the wake moves every vCPU's TSC offset by about
  minus the guest's uptime (-453e9 cycles = -122.6 s at 3.7 GHz [arithmetic]); the guest re-writes `SIMP`/`SINT` but not
  `REFERENCE_TSC`, `GUEST_OS_ID` or `HYPERCALL` after the wake (`hv_writes_after_wake` in `run/B_b1/c1_result.json`).
  Different from R1t: the guest's stale reference time here is POSITIVE (about +4 s; the page's offset field is
  `0x1f5be73` = +3.29 s), so its deadlines lie about 122 s in the past and KVM expires each one at once: an expiry storm,
  every expiry delivered, rather than R1t's wrapped deadline that never fires (R1t's page offset gave -0.40 s). [inferred]
  Which of the two a wake shows depends only on the sign of the stale page time at the wake; both leave the guest
  unresponsive. Hypothesis H1 of 2026-11-23 ("the u64 wrap") is therefore one form of the symptom, not the cause.
- **HV5 (post-wake TDR):** 0 `nvlddmkm` 153 in 10 clean VFIO wakes (A 5, B+P 5). kf3 logged three per wake before #575 and
  none after. The real driver's wake under QEMU does not TDR [measured on one die, one guest driver].

## Finding outside the question: a udev load/unload loop of the host NVIDIA modules during a VFIO bind (measured)

At 01:01:49Z, 7 minutes after the bind, a root ssh session (source not identified; this run's tools never load the nvidia
module while bound) loaded `nvidia`; with the GPU on vfio-pci it probed no device (`NVRM: GPU 0000:01:00.0 is already
bound to vfio-pci`) and unloaded. The stock rules `/usr/lib/udev/rules.d/71-nvidia.rules` (lines 10-19: on `add` of
`/bus/pci/drivers/nvidia` run `modprobe nvidia-modeset`/`nvidia-drm`/`nvidia-uvm`, on `remove` run `modprobe -r` of
each) then fed themselves: every `modprobe nvidia-drm` loaded `nvidia` again (a new `add`), failed, and unloaded it (a new
`remove`). The journal holds 62,495 failed `modprobe` lines from udev between 01:01Z and the restore, about 320 per minute,
and the udev queue was never empty. The GPU itself stayed on vfio-pci and the runs were not touched by it [inferred from
the guest results above, which match their predictions].
At the restore (`pcsession.restore_vfio`, every step rc=0, `nvidia-smi` answered) the queued events kept loading and
unloading the now-working driver (`nvidia-drm` "Unloading driver" 263 times in dmesg), gdm could not hold the card, and
`pcsession`'s settle wait would only have timed out. Repair (02:14Z-02:27Z, each step a `PC_ACTIVE.log` line): hold
`/dev/nvidiactl`, the 4070's DRM card and `/dev/nvidia-uvm` open so the queued `modprobe -r` fail (no more kernel events:
`udevadm monitor --kernel` silent); restart `systemd-udevd` to drop the stale queue (`udevadm settle` rc 0); re-trigger
`drm` and the two GPU functions; release the holders (they made the card busy for logind); restart gdm. After it: nvidia
595.91.07 on 01:00.0, `snd_hda_intel` on 01:00.1, gnome-shell on card1, `card1-DP-3 connected enabled`, `nvidia_uvm`
live, host `cuInit` 0, `vfio`/`vfio_iommu_type1` unloaded (`run/host_final.txt`; `run/host_before.txt` for the original).
Lessons for `pcsession.py` [not implemented here]: nothing may load the nvidia module on the pc while a VFIO bind holds
the GPU (other agents' `nvidia-smi` included); the restore should check that the udev queue is empty before reloading,
and treat a non-empty queue with nvidia add/remove events as this loop, not as a wedge. The 2026-11-18 "late
`modprobe -r nvidia-uvm`" finding is plausibly the same rule set with a shorter backlog [inferred].

## Files

`PREREGISTRATION.md`; `s3vfio.py` (prep + session driver), `pcsession.py` (the 2026-11-18 safe bind, unchanged, sha256
`3a4e74fb...`), `build_qemu_pair.sh`, `hyperv-wake.patch` (as applied; from `claude/optional-wake-patch-20261124`),
`summarize_run.py`, `storm_deadlines.py`; `run/`: `s3probe.log` (the session log), `bind.txt`, `restore.txt`,
`host_before.txt`, `host_after.txt` (written by the session at 02:20Z, before the repair), `host_final.txt` (after it),
`results.json.gz`, per boot `c*_result.json`, `c*_wedge_sample.json`, `argv_scrubbed.txt`, `powercfg_a.txt`,
`dmesg_filtered.txt`, and gzipped `c1_wake_trace_head` for A_b1, B_b1, BP_b1, B0_b1 (`B_b1/c1_boot_trace_tail` too).
Not committed (on the coordinator, `/workspace/bench/vfio_s3_control_20261124_full/`): the other cycles' raw wake traces.

Probe errata: the inherited `summarize` counts `kvm_hv_stimer_expiration` with `msg send result`, while this kernel prints
`send result`, so `expirations_ok/_fail` in the result files are parse misses (no verdict used them; `storm_deadlines.py`
reads the right field). `stimer_first` is empty in the storm cycles because the guest armed its timers through
`set_count` with auto-enable, which KVM traces as `kvm_hv_stimer_start_periodic`/`set_count`, not `start_one_shot`; the
preregistered storm definition matched through its exit-rate branch (more than 100,000 exits/s with ssh dead).
