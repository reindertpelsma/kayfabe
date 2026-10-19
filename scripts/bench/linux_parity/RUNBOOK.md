# LX1 host runbook

STATUS: RESEARCH, 2026-10-19. CODE READ: these scripts change benchmark host
bindings only; no kayfabe product code. Hardware qualification is still required.
The preregistration defines arms, correctness gates and stop conditions.

All mutable files are below the lane root. Copy this directory there, keep the
configuration private, and replace the unique run name for every boot. The
example's PCI values are placeholders to verify against the actual IOMMU group;
`amd_desktop_verified` must reflect actual display/process evidence.

1. Record the live owner launcher, log, QEMU identity/argv/environment. Copy its
   launcher byte-for-byte into the lane directory for later restoration. Verify
   the perf6 profile, including mirror OFF. Stop the owner only with the approved
   stop flag; wait for its EXIT marker and QEMU plus broker disappearance. A stop
   refusal means STOP; do not send a signal or substitute QMP quit.
2. Acquire the shared lock for the entire session:
   `flock -w 60 -o /tmp/kayfabe-fastguest.lock python3 host.py lease`.
   Keep that process alive in a durable host session. It records the actual
   flock PID/starttime and checks `/proc/locks`. Never overwrite lease.json or
   break a held lock. If the bounded wait fails, report contention.
3. Stage immutable A/B binaries in the configuration's bins directory; compare
   REV/hash with preregistration. Both binary directories require pc-bios and
   qemu-bundle links into the verified QEMU build resources. Do not relink an
   existing other-lane directory. Record actual symlink targets/hashes privately.
4. Provision the lane disk using stock QEMU, under that same lease:
   `python3 host.py run --arm P --config provision.json`.
   P has no NVIDIA device and is not a performance arm. SSH is forwarded only
   on loopback port 2229. Use the copied benchmark key privately. Preserve the
   existing NoCloud seed in the lane directory and attach it identically across
   P/A/B/C/D; omitting it can trigger cloud-init's new-instance work.
   pinned kernel and exact driver/module pairing; no distro switch or silent
   package fallback. Clean guest poweroff, verify QEMU exit, freeze manifests.
5. Before each gate/run, write its private config with the same guest disk and
   workload manifest. `python3 host.py plan --arm A --config a.json` is GPU-free
   and prints the complete QEMU command. Run `launch-kayfabe.sh A a.json` (or B/D).
   Scripts clear every inherited KF3 variable; B sets mirror 1, D sets mirror 0,
   A sets none. Unlike perf6's Windows-specific lifecycle meters, Linux A has no
   measurement flags. All arms share q35, eight host vCPUs, 12 GiB shared memfd
   RAM, disk iothread/cache policy, guest UUID, network and SeaBIOS firmware.
   Guest-driver= is explicit; unset was a prior Linux launcher failure.
6. VFIO: verify the AMD desktop and no remaining NVIDIA device users. Run
   `launch-vfio.sh c.json`. It refuses any QEMU/broker/GPU client or incomplete
   group, records each original driver, loads vfio-pci, sets per-function
   driver_override, unbinds and probes. No global module unload, firmware access
   or reset command. Linux clean poweroff between arms; record reset_method,
   dmesg and health after rebind. Reset reliability is unmeasured until exercised.
7. Run `sample-host.sh PID 600 > run/host-samples.jsonl` with the exact owned
   QEMU PID. Samples carry UTC/monotonic clocks, pressure, per-vCPU schedstat and
   NVML. Errors are recorded; missing NVML during VFIO is expected and must be
   complemented by guest NVML sampling. Stop this owned finite sampler normally
   or signal only its verified PID. Clock calibration: bracket a guest
   `time.time_ns()` with host realtime_ns ten times and keep the minimum-RTT
   offset with half-RTT uncertainty. Guest per-frame timestamps need that mapping.
8. Shutdown with guest poweroff (or QMP system_powerdown as an ACPI request),
   never QMP quit on a refusal. The foreground launcher waits and then restores
   VFIO's recorded bindings in finally. If it is interrupted with live QEMU,
   restoration refuses and the outer lease stays held. For recovery *after*
   verified QEMU exit: `python3 host.py restore --config run/bindings.json`.
   Restore never executes beneath a live QEMU. Confirm both NVIDIA and audio
   drivers, clear driver_override, check nvidia-smi and attributed Xid journals.
9. After all owned QEMUs/brokers have exited, create the lane's RELEASE file.
   Guardian verifies idle, removes only its own marker and releases flock.
   Restore the observed owner perf6 copy under the shared lock, with next unused
   run, mirror OFF and original binary. Wait READY and compare actual exe/env.
   The intentional owner lease is not an LX1 leftover. Never start a second owner.

MEASURED limits: contract tests use fake process/PCI/lease data and inspect plans;
no test detaches a GPU, boots QEMU or establishes physical reset reliability.
There is no automatic owner shutdown/restart in this code. The operator must
follow the explicit owner flag/READY/EXIT protocol. Sampler runs are bounded;
no firmware, mitigation, power, clock, timeout or product poll setting is changed.
