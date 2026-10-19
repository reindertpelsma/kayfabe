# LX1 GPU-free preparation

STATUS: PREPARED, 2026-10-19. Hardware results are not implied.

MEASURED: preregistration pushed first at 877f01171; new harness contract tests
committed alone EXPECTED RED at 2bf7bf7cc. The absent new API produced 28/28
assertion failures (not a discovered kayfabe product defect). Implemented harness
then passes 32/32 tests, including two added sampler fixtures and two additional
same-resource contracts. Restored-source run passes 32/32, zero ignored.

MEASURED: nine actual source mutations were applied one at a time and restored;
every mutant fails at least two distinct test methods. MUTATIONS.json names
the exact killers: inherited flags 3, mirror inversion 2, disk substitution 2,
private RAM 2, ignored users 3, incomplete group acceptance 2, unsafe restore 3,
ignored lease 2, naive proc-stat splitting 3. These are harness contracts, not
product or physical GPU proofs. No Rust/C product behavior changed.

MEASURED: three shell wrappers pass bash -n; the plan command emits the expected
binary, explicit guest-driver, shared memfd, same disk and mirror flag without
launching anything. The inherited forwarding-gate selftest passes six fixtures.
No cargo build, target directory, host VM boot or GPU workload in Part A.

CODE READ: launch plans follow existing fat-guest memfd/q35 and owner perf6's
immutable binary, disk iothread/cache and identity pattern. The host harness
checks actual flock PID/starttime/inode, refuses concurrent QEMU/brokers and
VFIO GPU clients, records both PCI-function bindings, and restores after QEMU
exit. A failure retains the lease. Runbook documents manual owner stop/restore,
per-binary QEMU resource links and guest reset limitations. The sampler exposes
query errors, clocks, P-state, power, utilisation, PCIe, throttle status, pressure
and per-vCPU runtime/wait on identified processes with UTC timestamps.

MEASURED inventory: owner run 3018 is READY on the requested mirror-OFF perf6
profile; host kernel is 7.0.0-34-generic and its NVIDIA module identifies itself
as open 595.91.07. DRM card0 is amdgpu, card1 NVIDIA; this does not by itself
prove that no desktop client holds NVIDIA. The RTX group contains GPU and audio
functions. Host root/data share a filesystem with approximately 77 GiB free;
dev root had approximately 46 GiB free. No owner's process/file was changed.

CODE READ: all required lane instructions, relevant binding rulings/constraints,
status updates, M1/P1/R1 reports, CUDA/gfx/video/LLM/doorbell/fastguest workflows
and the earlier parity report were consulted. M1's 62-image inventory is an
inherited observation, not repeated LX1 inspection. Its missing tool bundle and
provisioning conflict are addressed in PREREG: provisioning is scheduled only
after owner shutdown under the global hardware lease. The requested
submit_latency directory is absent in available candidate-5/mirror checkouts.

INFERRED next step: copy the prepared tools to the lane host directory, record
the owner restore configuration, stage exact A/B binaries, then execute Part B's
graceful stop and bounded lease acquisition. Verify packages/driver sources and
the same disk under stock QEMU before any Linux mirror or performance claim.
