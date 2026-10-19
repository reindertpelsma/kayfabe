# Linux parity — paused, prerequisite not qualified

STATUS: PAUSED, 2026-10-19 21:13 UTC. Lane LX1, branch
`codex/linux-parity-20261019`, based on candidate-5 f8f9d1a5f.
The report revision is the commit containing this checkpoint; origin equality
is verified after each push and the exact hash recorded in the private handoff.

## Result (interim; no parity conclusion)

- DONE — MEASURED: preregistration, guarded launchers, restore path and per-second sampler prepared; GPU-free contracts pass 37/37.
- DONE — MEASURED: 11 source mutations each fail at least two distinct tests; tests-first commits preserved.
- DONE — MEASURED: original owner 3018 stopped gracefully; binaries/hash/resource links staged; one private stock-QEMU provisioning boot succeeded.
- PARTIAL — MEASURED: noble gfx/CUDA packages installed, matching595 installer/source staged; provisioning then segfaulted and PID1 panicked before qualification.
- NOT DONE — MEASURED: no matching595 guest-module build or loaded-driver/image manifest qualified; no A/B/C/D GPU run.
- NOT DONE — MEASURED: all Linux mirror gates, parity ratios and ten-minute hitch runs remain UNRUN.
- DONE — MEASURED: own panicked GPU-free QEMU exited; initial owner restore3019 reached READY with all KF3 variables matching original.
- DONE — MEASURED: on coordinator pause request,3019 stopped gracefully; all LX1 holders released, GPU NVIDIA-bound, exact PAUSED marker written. Coordinator now controls owner restart.

## Workload × arm table

MEASURED: A = f8f9d1a5f production with no diagnostic/measurement variables;
B = 0d7ba743 + KF3_TSG_MIRROR=1; C = VFIO using A's QEMU. These are planned
GPU arms, not executions. Every cell has n=0; median/spread and both ratios
are UNMEASURED. No zero FPS or equivalence is inferred.

| Workload | A | B | C | A/C; B/C; spread |
|---|---|---|---|---|
| W1 tiny polled/blocking, throughput | UNRUN | UNRUN | UNRUN | UNMEASURED |
| W1 event/graph launch | UNRUN | UNRUN | UNRUN | UNMEASURED |
| W1 H2D/D2H/D2D 4KiB..1GiB | UNRUN | UNRUN | UNRUN | UNMEASURED |
| W1 async allocation/free, managed faults | UNRUN | UNRUN | UNRUN | UNMEASURED |
| W2 cuBLAS fp32/fp16, bandwidth | UNRUN | UNRUN | UNRUN | UNMEASURED |
| W2 fixed-model LLM | UNRUN | UNRUN | UNRUN | UNMEASURED |
| W3 vkmark/all scenes, glmark2 | UNRUN | UNRUN | UNRUN | UNMEASURED |
| W3 Heaven/optional GravityMark | UNRUN | UNRUN | UNRUN | UNMEASURED |
| W4 H264/HEVC NVENC/NVDEC, copies | UNRUN | UNRUN | UNRUN | UNMEASURED |
| W5 five ten-minute mixed loops | UNRUN | UNRUN | UNRUN | UNMEASURED |

## Linux mirror gates

| Required gate | A / B / D mirror OFF | Result |
|---|---|---|
| CUDA ladder/apps/multistream | 0 / 0 / 0 | UNRUN: guest prerequisite failed |
| Vulkan/graphics hashes, old Xid69 shape | 0 / 0 / 0 | UNRUN: same |
| Video content/throughput | 0 / 0 / 0 | UNRUN: same |
| Broker/interactive/cursor retest | 0 / 0 / 0 | UNRUN: same |
| UVM/many-channel/hardware cap | 0 / 0 / 0 | UNRUN: same |
| Complete mirror-OFF equivalence stage | 0 / 0 / 0 | UNRUN: same |

## Prerequisite failure and scope

MEASURED: the only LX1 VM was stock distro QEMU10.2.1 (Debian package
1:10.2.1+ds-1ubuntu3.2), eight vCPUs/12GiB shared memfd RAM, q35, one private
noble disk and original cloud seed, with **no NVIDIA/kf3/vfio device attached**.
Guest kernel6.8.0-142-generic initially had open580.159.04 and matching libcuda.
Signed noble apt supplied graphics tools and CUDA12.0. The apt command ended
139 after package setup/triggers. Services failed, SSH became unavailable, and
at guest uptime363.368s kernel panic reported `Attempted to kill init!`
with exitcode0x8b, PID1 systemd, kernel not tainted. See
[stock-guest-panic.txt](stock-guest-panic.txt). It is an open Linux/systemd panic,
not NVIDIA closed-binary analysis.

MEASURED: no completed matching595 source-module build/installation was verified.
Installer integrity and OGKM source provenance were verified separately
([HARDWARE_PROGRESS.md](HARDWARE_PROGRESS.md)); they do not qualify this image.
The launch itself succeeded (cloud-init/SSH at approximately5s); this was a
later provisioning failure, not a failed QEMU launch silently retried.
Only the exact LX1 PID/starttime/executable/name was sent TERM after its private
guest had panicked; QEMU flushed/exited0. This is not a clean Linux shutdown
claim. No force signal, GPU reset, VFIO transition or timeout change occurred.
The failed disk is preserved privately and must not be used for performance.

INFERRED: this run cannot establish a kayfabe GPU defect, Linux/VFIO parity,
TSG benefit or absence/presence of Linux100ms hitches. H1–H5/H7 are untested;
H6's healthy-image prerequisite is unmet. Root cause of the CPU-only crash is
unknown; no driver/kernel/CPU/disk policy was improvised as a performance fix.
Host I/O pressure was high during provisioning, but correlation does not
explain the SIGSEGV. Its falsifier (persistent failure with normal I/O) was
not tested before stopping the VM; no causal conclusion follows.

MEASURED GPU-free follow-up during coordinator pause: read-only/no-replay/noexec
inspection checked1,222 regular files against seven installed-package manifests
(systemd663, systemd-sysv6, shared6, libc283, libc-bin30, sudo51, apt183), zero
mismatches. Symlink paths were skipped rather than resolving guest absolute
links on the host. Three empty crash files and one352,138-byte sshd crash file
are present; contents remain private. The checks weaken H8 file damage, but
do not authenticate every guest byte or diagnose transient memory/CPU faults.
Initial inspections hit the documented partition-device event race; both
disconnected their own NBD without a mount. M1's explicit rereadpt + udev wait
recipe then completed, followed by unmount/disconnect. No guest program was
executed on the host, and no GPU lock/VM boot occurred in this follow-up.

## Tests and mutations

MEASURED: first new-API EXPECTED RED28 failures →32 passes; cloud-seed proof
32 passes/3 failures →35 passes; idempotent-restore proof35 passes/2 failures
→37 passes. All final37 pass, zero ignored. Three shell wrappers pass bash -n;
the existing forwarding selftest passes six fixtures. No product Rust/C change,
cargo build, v3/fast/app hardware gate or product-test count is claimed.

| Source mutant | Distinct killing tests |
|---|---:|
| retain inherited KF3 flags | 3 |
| invert mirror switch | 2 |
| substitute another disk | 2 |
| private RAM instead of shared | 2 |
| ignore active users | 3 |
| ignore incomplete IOMMU group | 2 |
| unsafe driver restore | 3 |
| ignore lease identity | 2 |
| naive proc-stat splitting | 3 |
| omit cloud seed | 3 |
| retain stale override on idempotent bind | 2 |

MEASURED: [MUTATIONS.json](MUTATIONS.json) lists exact killers. These tests use
fake sysfs/processes; physical VFIO rollback/reset and sampler accuracy remain
unqualified. The last defect was in unexecuted harness code: already-bound
GPU/audio drivers retained a stale override. Independent fake-sysfs tests failed
first and now clear it; reverting the fix fails both. No production default
or security policy changed and nothing was merged.

## Current host state, decisions and remaining work

MEASURED: full retained same-boot journal Xid528→528 (delta0) through initial
restore; ring-based owner helper21→21. GPU remains nvidia595.91.07, audio
snd_hda_intel, host kernel7.0.0-34-generic. No GPU workload ran, so this is not
a GPU stability qualification. Final pause at21:07:15.580457UTC verified no
QEMU/broker/own lease holder, NVIDIA binding and free shared flock, then wrote
exact `LX1 PAUSED host released at 2026-10-19T21:07:15.580457+00:00`.
Owner run3019 exited0 at21:06:59UTC. **Do not restart the owner from LX1:**
the coordinator explicitly owns that action and must schedule the next slot.
No further host/GPU availability is presumed from the PAUSED marker alone.

INFERRED ranked next steps/owner decisions:
1. Schedule with the coordinator before another host stop; keep the owner's
   Minecraft session undisturbed. Recommendation: no automatic LX1 restart.
2. Establish a healthy CPU-only Linux provisioning baseline and resolve the
   stock-QEMU userspace crash before GPU qualification. Recommendation: retain
   the failed private disk/logs, then agree a bounded same-config reproduction
   or a fresh known-good image; no timer/mitigation/driver guessing.
3. Once a matching595 image is proven under stock QEMU, freeze manifests and
   pass mirror CUDA/Vulkan/video/broker gates before any parity run.
4. Only then execute the preregistered interleaved n>=5 matrix, with independent
   per-frame timestamps and matched VFIO guest telemetry;580 is a separate matrix.

MEASURED cleanup: dev target was never created and is absent; no lane build,
collector, GPU lease or VM remains. The latest offline mount/NBD was released.
Private failed image/cache/source/logs intentionally remain under the lane host
directory for diagnosis. The original Windows disk and launcher were unchanged;
all commits are lane-only with required trailers and secret/path/address scans.
The eight-hour deadline remains04:45:32UTC on2026-10-20, including this pause.
