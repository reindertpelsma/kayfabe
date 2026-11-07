STATUS: DESIGN-ONLY, 2026-11-07. J3 feasibility tooling; no nested VM, GPU or Vast run was performed.

# Nested kayfabe feasibility probe

**Owner intent (2026-11-07):** run a small thin Linux guest inside a full Linux kayfabe guest,
using prebuilt kayfabe binaries in the outer guest. On a Vast VM, the proposed outer guest is
L2 and the inner guest is L3. This is an experiment, not a support or performance claim.

**Hypothesis, NOT MEASURED:** the inner raw RM client's GPU work can traverse both kayfabe
instances and produce checked output on the real GPU. **Falsifier, before hardware:** a missing
accelerator, failed boot, missing result, mismatched bytes, failed completion or inability of the
inner QEMU's host CUDA/RM calls to use the outer guest's GPU falsifies feasibility for that
configuration. A successful CPU-accelerator dry check does not establish GPU feasibility.

## Probe contract (CODE READ)

`scripts/bench/box/nested_probe.sh` emits exactly seven `NESTED_PROBE key=value` observations:

| key | vocabulary | source |
|---|---|---|
| `kvm_device` | `yes`, `no` | existence of `/dev/kvm`; existence does not imply permission |
| `kvm_intel_nested` | `yes`, `no`, `unknown` | `kvm_intel/parameters/nested` |
| `kvm_amd_nested` | `yes`, `no`, `unknown` | `kvm_amd/parameters/nested` |
| `cpu_vmx`, `cpu_svm` | `yes`, `no` | exact CPU flag tokens in cpuinfo |
| `kvm_start` | `yes`, `no` | zero exit from `qemu -accel kvm -machine none -S`, monitor `quit` |
| `tcg` | `yes`, `no` | `tcg` advertised by `qemu -accel help`, with a clean exit |

**CODE READ:** both QEMU probe calls have a five-second limit. The KVM dry check initializes
an accelerator with no CPU execution and no GPU device; it is not a tiny guest boot. TCG
availability reports the accelerator listing; neither probe boots a guest. The sysfs parameters
and CPU flags are observations; the selected accelerator depends on the dry check. Missing sysfs files are
`unknown`, not proof that nesting is disabled.

**CODE READ (`scripts/ci/test_nested.py`, `Probe` and `StageIntegration`):** the fixtures substitute
`KF_NESTED_DEV_ROOT`, `KF_NESTED_SYS_ROOT`, `KF_NESTED_CPUINFO` and `QEMU_BIN`. No real `/dev/kvm`
or GPU is opened by those tests.

## Stage contract (CODE READ; hardware behavior NOT MEASURED)

`scripts/bench/box/nested_gate.sh <tag> <prov_dir> <full-revision>` calls `nested_gate.py`.
The caller supplies a prepared, running outer Linux guest over SSH. This stage does not create
or boot the outer guest. The coordinator must keep that guest alive after provisioning and
keep the bench exclusively assigned to this experiment; the existing CUDA ladder does not
promise to leave such a guest running. No second GPU workload should run alongside it.

**CODE READ:** the stage streams six regular, non-symlink files into a fresh private
`/tmp/kf-nested.*` directory in the outer guest: the revision's QEMU binary, the fastguest kernel,
initrd and driver receipt, and the two stage helpers. Nothing is built in the outer guest.
The remote directory is removed by its owning shell after the inner runner exits. The SSH
agent is not forwarded. These are isolated bench SSH settings, including ephemeral host-key
handling, not an SSH configuration for a user's production machine.

**Required preparation, NOT MEASURED here:**

- Host artifacts: `BENCH_DIR/kf3-bins/<first-eight-revision-hex>/qemu-system-x86_64` and
  `BENCH_DIR/fastguest/{vmlinuz,initrd.cpio.gz,driver.STAGED}`. `KF_NESTED_BINS` and
  `KF_FASTGUEST_DIR` may select other prepared directories. The receipt needs exactly one
  numeric `version=` value. The operator must verify that artifacts match the requested
  revision; a directory name and a printed SHA-256 are provenance aids, not build attestation.
- Outer guest: Python 3, Bash, tar, timeout, QEMU runtime libraries/firmware for the prebuilt
  binary, the installed stock NVIDIA driver and working CUDA driver libraries. Its NVIDIA
  device nodes must permit the SSH user to execute the nested QEMU. The normal apps image alone
  is not evidence that these dependencies are complete.
- SSH defaults: `ubuntu@127.0.0.1:2222`, identity `BENCH_DIR/guest_key`.
  `KF_NESTED_SSH_HOST`, `KF_NESTED_SSH_PORT`, `KF_NESTED_SSH_KEY` override them for a direct
  script invocation. `BENCH_DIR` defaults to `/workspace/bench`.
- The 1 GiB inner guest RAM and 1 GiB inner framebuffer are conservative **DESIGN choices,
  NOT MEASURED** minima. The outer guest must have sufficient RAM, VRAM and BAR headroom for
  its own driver, nested QEMU, kayfabe walker and the inner guest. The experiment must report
  a refusal rather than silently increasing its footprint.

**CODE READ:** a successful KVM dry probe selects KVM with `-cpu host`; otherwise advertised
TCG selects TCG with `-cpu max`. There is no automatic second attempt after a failed KVM boot.
The inner workload is exactly `KF_ARMS=--ce-client`, one raw-client selection with three
internal checks: a copy with checked data/completion, an occupied-address control and an
unmapped-address control. It does not run the 30-arm suite or load libcuda in the inner client.
This is a tiny raw GPU/CE check of the path used by CUDA; it does **not** prove CUDA kernel,
libcuda, UVM, graphics, fault containment, recursive isolation or general nested support.
The nested QEMU itself still needs CUDA for kayfabe's walker.

**CODE READ:** PASS requires the client's exact `ALL ARMS MET` summary, exactly one ready
marker, client exit 0 and DONE, in order, plus QEMU exit 0. An explicit checked client failure
is `NESTED_FAIL class=CHECK`. Missing/malformed/duplicate evidence, missing artifacts, SSH or
QEMU failure, no usable accelerator and time exhaustion produce `NESTED_RETRY class=...`.
A host/harness fault never becomes a product PASS. The payload independently requires all five
ordered, passing rows (`install`, `probe`, `boot`, `raw_ce`, `cleanup`), one PASS verdict and
one matching exit line. Raw guest output is prefixed/escaped in bounded diagnostic tails and
cannot author a row. The full remote-stage stream is retained in `<prov_dir>/<tag>_nested.log`.
**CODE READ:** row seconds are observed host wall time; `boot` ends when the ready marker is
observed, and `raw_ce` includes the subsequent check and poweroff/reaping. Polling resolution
is 200 ms; these are not guest-only kernel timings.

**Budget policy (DESIGN choices, NOT MEASURED):** `KF_NESTED_KVM_S=600`,
`KF_NESTED_TCG_S=1800`, `KF_NESTED_BUDGET_S=2400`. Require
`0 < KVM < TCG < total <= 7200`. The deadline includes artifact preparation/transfer and the
probe. Refuse to start the VM unless its entire accelerator budget remains. When the workload
budget expires, TERM the owned process group and reap before grading. As in the fastguest
lane, GPU VM teardown is never SIGKILLed; TERM cleanup can exceed the execution deadline and
cannot turn an overrun into PASS. A VM ignoring TERM remains a hardware-coordinator cleanup
blocker: the stage deliberately waits rather than abandoning ownership. This is not a hard
upper bound on cleanup. Outer-guest clocks must be synchronized with the host for the absolute
transfer-inclusive deadline. SSH/outer-guest loss prevents this harness from proving remote
cleanup; the coordinator must inspect the outer guest before another GPU run.

## Payload and report (CODE READ)

`KF_NESTED` is an optional `0|1` environment-contract flag in `vast_report.py`, absent/0 by
default. `1` is accepted only with `KF_TIER=large`; small, middle, adhoc and unknown tiers refuse
it at preflight. `KF_STAGES=nested` is refused. The flag is consumed before stage subprocesses.
There is no Worker deployment or admission change in this lane, and no Worker currently
supplies this opt-in as a result of this change. `KF_WINAPPS` was absent in the starting tree;
J3 follows the existing optional-boolean contract rather than assuming that flag exists.

**CODE READ:** the optional stage runs after `cuda_guest`/optional apps and before Windows.
It requires 45 minutes remaining at the payload deadline (**INFERRED margin**, not a timing
result). Its suite is `nested`; harness faults are failing `nested_retry_<class>` suite rows,
as with the apps stage, so no retry counts as a passing release run. `vast_run_report.py` copies
these suite rows and parses only known `NESTED_ROW` names into tests of suite `nested` in the
Linux subrun. `NOT_RUN` rows become skipped tests. Linux subrun failure retains the retry cause
in its suite name. This attribution describes the Linux stage; it does not claim that the
inner driver's version equals the outer guest's recorded driver version. No perf row is invented.

## Hardware checklist for the coordinator (NOT RUN)

1. Record exact code/artifact revisions and hashes, host/outer/inner kernels and drivers,
   CPU flags, nested parameters, GPU family and requested memory budgets; run the probe on a
   Vast box and **inside the outer Linux kayfabe guest**. Preserve both logs. Falsifier: a dry
   KVM failure disproves usable inner KVM for that configuration, regardless of vmx/svm flags.
2. On an otherwise idle box, verify the tiny raw CE selection directly in the prepared outer
   guest first. Falsifier: a failing outer baseline prevents attribution to the extra depth.
3. Boot the inner thin guest once with KVM where available, then separately under an explicitly
   prepared TCG-only configuration. Measure boot-ready, raw-client and complete shutdown times,
   including cleanup; do not quote the proposed budgets as measurements. Falsifier: a missing
   ready/DONE marker or exceeded budget means no boot-feasibility result for that mode.
4. Verify whether inner QEMU's RM and CUDA walker can use the outer guest's virtual GPU and
   whether inner raw-client copy/completion reaches the physical GPU through both instances.
   Collect both QEMU logs, serial logs, host/outer driver diagnostics and channel/doorbell
   evidence. Falsifier: refused nested mappings, failed checked output, unexplained completion,
   GPU reset or broken outer GPU progress prevents a protocol-forwarding claim.
5. Verify synchronized clocks, runtime library/firmware installation, artifact hashes, memory
   and BAR headroom, no new host/outer Xid/reset, normal outer GPU use after the inner exits,
   and that only the stage's processes/directories were removed. Test timeout and lost-SSH
   cleanup deliberately before enabling the release opt-in. Keep it disabled until these pass.

## GPU-free validation commands (CODE READ; fakes only)

`python3 -m unittest discover -s scripts/ci -p test_nested.py` exercises fake roots, fake QEMU,
fake SSH, the real artifact-transfer shell, the payload's fake stage runner and report parsing.
`python3 scripts/ci/nested_mutants.py` runs isolated mutants; each needs at least two failing
test methods. Neither command connects to a box or boots a VM. Results and limits are recorded
in the J3 lane report; this document records no hardware measurements.
