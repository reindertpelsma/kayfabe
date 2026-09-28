# Doorbell baseline and ioeventfd prerequisites

**STATUS: RESEARCH / MECHANISM PROBE, 2026-09-28.** Owner selected a non-nested baseline and
host-side/ioeventfd work first, optional guest helper afterward. A modified guest NVIDIA driver
remains a later option. No production doorbell route is changed here; no new GPU ratio is measured.

## What exists, and what this adds

`qemu/hw/misc/kf3/kf3.c` already has a default-off `dummy-bar` probe: lockless MMIO, BQL MMIO,
and a pure-MMIO page with an undrained, non-DATAMATCH eventfd. That tests a synthetic trap floor,
not token delivery to a real GPU, and not the read-only memslot used for passthrough reads.

`tools/probes/kvm_ioeventfd_readonly.c` fills that specific gap without a GPU. It creates a tiny
x86 real-mode KVM guest with a read-only RAM memslot. The guest writes tokens A, B and an unknown
token, then reads the page's original sentinel. Four cases cover no routes, two token-matched
eventfds, A deassigned, and A reassigned. It checks **exact fallback values and order**, eventfd
counts, guest readback and unchanged backing RAM. All four passed both an optimized build and
ASan/UBSan on this workspace's Linux 7.0.0-34-generic KVM host (itself virtualized).

```sh
cc -O2 -Wall -Wextra -Werror tools/probes/kvm_ioeventfd_readonly.c -o /tmp/kf-ioeventfd-probe
timeout 10 /tmp/kf-ioeventfd-probe
```

Exit 0 means the four cases passed, 77 means KVM access/capability is unavailable, other exits mean
failure. This is not a hardware-notification latency test, a concurrent lifetime proof, a security
proof or evidence for 70% GPU throughput. It never opens an NVIDIA device or changes a host module.
The authoritative interface is [Linux KVM API: KVM_IOEVENTFD](https://docs.kernel.org/virt/kvm/api.html#kvm-ioeventfd).

## Non-nested baseline protocol

Read-only SSH to the documented physical RTX 4070 host `172.22.1.20` failed with **Network is
unreachable** on 2026-09-28, including a direct retry from the main session. No workload, desktop,
module or configuration was changed. Do not substitute another Vast KVM VM and label it non-nested.

When access is restored:

1. Read `/root/CODEX-HANDOVER.md` on the development machine for shared-host exclusions. Check the
   physical host's active desktop, CUDA/QEMU jobs, disk space and virtualization status. Do not stop
   another session or change its driver. If occupied, arrange an idle measurement window.
2. Record source SHA, exact QEMU binary, guest/host kernels and drivers, GPU, nesting, clocks/power,
   CPU placement, model snapshot, and competing processes. First pass the stock host CUDA control
   and Kayfabe correctness smoke. Host-driver compatibility is a prerequisite, not an assumed fact.
3. Reuse `scripts/bench/llm_parity.sh` and its identical host/guest runners. Compare eager and graph
   lanes, cold and warm timings, with identical model/prompt and output hashes. Use repeated paired
   runs rather than comparing unrelated boxes. A guest kernel transition is not itself a VM exit.
4. Record doorbells/token and hardware VM exits separately. The current harness finds QEMU with
   `pgrep ... | head -1`; **do not use that on a shared multi-VM host without binding collection to
   the benchmark's exact PID**. Otherwise another VM's counters could be attributed to this run.
5. Measure the existing dummy trap floor separately. Then compare actual application throughput,
   CPU use and tail latency; an empty eventfd handler cannot stand in for GPU notification.

The owner's ~0.70x expectation and possible Windows batching advantage are useful hypotheses.
Neither is a pass criterion or a result. Windows needs its own submission/doorbell counts; this
Linux probe does not validate a Windows path.

## Acceptance criteria before a production ioeventfd path

- Use value-specific DATAMATCH registrations for owned live tokens. Plain eventfd counters discard
  the written value. Unknown tokens and non-passthrough channels must retain the established path.
- Preserve read passthrough, store width/address matching, per-GPU routing, moved BAR1 aliases and
  guest userspace/kernel distinctions. Test mismatched widths and addresses explicitly.
- Bound fd/device registration counts; registration failure must leave a correct trapped path.
- Bind each worker event to a channel **generation**, not just a recyclable numeric token. Order
  deassignment, outstanding-event draining/quiescence and channel destruction; test parallel rings,
  channel free/reuse, hot-unplug and worker teardown. The single-vCPU probe does not establish this.
- Treat eventfd as a work notification: coalescing is permitted only with a progress/ordering
  argument and tests for a new PUT arriving while the worker handles an earlier notification.
- Report at least three distinct quantities: guest/vCPU blocked time, submission-to-hardware
  notification latency, and GPU work completion latency. Test both deep queued workloads and idle,
  synchronous single-launch workloads. Include median/p99, CPU cost and correctness checks.
- Keep the fast path optional until the whole-device regression bar and those adversarial/lifecycle
  tests pass. No new privileged host doorbell module is implied by using existing KVM ioeventfd.
