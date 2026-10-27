# Z1 Linux experiments

These are operator scripts. They do not change kayfabe or its fault oracle.
Run the host campaign under `flock -o /tmp/kayfabe-fastguest.lock` after the
preceding campaign releases it. Refuse any existing QEMU and preserve the owner VM.

Prepare an exact-source QEMU with `scripts/bench/build_kf3.sh`, a fresh host-mode
fast image with `scripts/fastguest/build_fast_guest.sh`, and a JSON manifest:

```json
{
  "revision": "<full checkout revision>",
  "driver": "<host-mode guest driver version>",
  "qemu": {"path": "<revision-directory>/qemu-system-x86_64", "sha256": "<hash>"},
  "gates": {
    "kf-gate1": {"path": "<gate binary>", "sha256": "<hash>"}
  }
}
```

The gates map must name gates 1 through 9 and `kf-micro-reserve-probe`. Record all
binary/source hashes before execution. Use one separate target and QEMU build tree
for this revision. Run native gates and `native_baseline.py` before guest tests.
The native script retains the unchanged source-matched deliberate-fault proof.

For two VMs, use `prepare_cuda_guest.sh` to make a CUDA image from the fresh raw
image. The unchanged cup3 and cup8bench instruments and matching host libcuda/JIT
libraries are added to the initrd; the guest driver/kernel remain those of the
base image. `two_vm_share.sh` runs three solo CUDA cases, three concurrent CUDA
pairs, and three concurrent four-arm raw subsets. Every next VM needs its guest
RAM plus 2 GiB headroom. Each has a separate log directory, lock and process name;
all live under the outer shared host lock. Both stores are 4096 MiB on GPU minor 0.
The current experiment expects a 12 GiB GPU with sufficient host BAR1; it is not
an automatic allocator for other hardware. Actual binding appears in each log.

For rootless, stage readable assets and a writable bench directory. An administrator
creates a throwaway user whose only supplementary group is `kvm`. Reuse the audit's
`setpriv` clean-environment pattern, optionally dropping the bounding set as Z1 does.
The rootless launcher refuses root, nonzero effective/permitted capabilities, extra
groups, foreign QEMU processes and insufficient RAM. Its per-process records include
capabilities, limits-related fields and opened device nodes. Neither launch nor the
prebuilt native gates invoke sudo or change host drivers/modules/device permissions.

```bash
BENCH_DIR="$run/bench" KF_FASTGUEST_DIR="$run/fastguest" \
KF_REAL_QEMU="$binary" KF_QEMU_SHA256="$binary_sha" \
    scripts/fastguest/rootless_suite.sh trial 180
```

Run that command as the ordinary user. When a root coordinator already holds the
shared lock, it may set `KF_Z1_LOCK_HELD=1`; the inner per-VM lock remains private.
`rootless_gates.py <manifest>` executes and grades the pinned native binaries without
requiring a Rust toolchain at runtime. The 2026-10-16 audit scripts `as_user.sh`,
`u_suite.sh` and `watch_caps.sh` supplied the setpriv/private-lock/sampling method.

The full suite deliberately causes faults. A separate root kernel-log observer is
needed for attribution on hosts restricting journal/dmesg access; this is test
instrumentation and grants the user no privilege. `observe_rootless.py` starts its
own unprivileged suite, verifies the actual process ancestry/start time/executable,
reuses `native_xid_guard.py` unchanged, and acknowledges each completed arm before
another may start. Its observer receipt requires the same-box native proof first.
The rootless launch works independently of this observer; without one, do not make
a host-fault claim from an unreadable journal. A real refusal stops the experiment.

Keep raw logs private. Publish projected scoreboards, hashes, process/capability
summaries, matrix execution-window overlap, throughput distributions and cleanup.
No fairness, isolation, hostile-kernel latency or Windows result follows from Z1.
