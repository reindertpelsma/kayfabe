# Doorbell baseline and ioeventfd prerequisites

**STATUS: RESEARCH / MECHANISM PROBE, 2026-09-28.** Owner selected a non-nested baseline and
host-side/ioeventfd work first, optional guest helper afterward. A modified guest NVIDIA driver
remains a later option. No production doorbell route is changed here; no new GPU ratio is measured.

**Owner refinement, 2026-09-28:** no timer-based batching and no intentional delay. Ring as soon as
the worker can act; allow only incidental coalescing of notifications already pending when it wakes.
CUDA already batches work. Host ring count need not equal guest doorbell count, but the last
notification must never disappear. This refinement does not select an unmeasured fast path as the
default or waive lifecycle/order checks.

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

## Why passthrough is the initial performance target

The production classifier in `crates/kf-qemu/src/chan.rs` births ordinary user channels as
passthrough and guest-kernel physical-address copy-engine channels as translated. The latter's
rewriter, real GPU submissions, fence-based retirement and wake/requeue protocol already exist;
this experiment does not replace them. Kernel channels are explicitly forbidden from the
emulated route (`kf_core::channel::Submission::kernel_channels_are_never_emulated`). Therefore
"translated and emulated are both kernel traffic" is not the routing rule.

High-frequency application submissions make passthrough the first target. Lower translated trap
volume is a workload hypothesis, not a guarantee: allocations, scrubbing, migration and future
UVM fault service can put kernel CE latency on an application's critical path. Keep those paths
prompt and measure them separately. Existing correctness tests and hardware gates establish a
working implementation, not universal driver coverage or proof that every workload is fast.
Translated completions must still follow actual GPU work; notifications alone are never proof
of completion. The worker's existing timed park is a shutdown recheck, not a batching delay:
ready work is handled immediately and eventfd readiness interrupts the wait.

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
  Do not add a debounce timer, batching deadline, or repeated drain-until-quiet loop before acting.
  A proposed first implementation drains one ordinary per-channel eventfd counter **before** the
  host ring/queue inspection, then acts immediately. Arrivals after that drain remain pending for
  another pass. Keep this separate from the existing semaphore-style worker wake eventfd.
- Report at least three distinct quantities: guest/vCPU blocked time, submission-to-hardware
  notification latency, and GPU work completion latency. Test both deep queued workloads and idle,
  synchronous single-launch workloads. Include median/p99, CPU cost and correctness checks.
- Keep the fast path optional until the whole-device regression bar and those adversarial/lifecycle
  tests pass. No new privileged host doorbell module is implied by using existing KVM ioeventfd.

## Proposed composition, not a production implementation

Keep the existing table-driven MMIO handler as the slow path. Register DATAMATCH only for live
passthrough routes initially. Unmatched values (including noncanonical values the existing decoder
masks), unregistered channels and registration-capacity failures continue through normal MMIO
handling. A table-unknown token stays unknown/no-op; fallback is not permission to ring a guessed
host channel. Translated/emulated channels retain their current stamped RUNG/BUSY_RUNG protocol;
moving those directly to eventfd would need an explicit ordering proof.

The optional guest helper is an additional opportunistic layer: an eligible passthrough write can
go straight to the real host doorbell; its fallback writes the classic guest page, which may then
match ioeventfd or reach the table handler. Do not introduce shared guest/host pending flags merely
to suppress an occasional redundant ring. Those flags add another lost-wakeup/lifetime protocol.

Drain-before-act is the simple correctness baseline: an arrival before the drain is covered by the
subsequent action; an arrival afterward leaves a notification owed. An implementation must preserve
the required memory ordering from queue/PUT publication to the actual host MMIO write, and retain
the channel generation through dispatch. Clearing/draining **after** the action can discard a new
notification the preceding ring did not cover. If a translated worker reaches its execution budget
with work remaining, it must requeue independently of whether the eventfd counter is zero.
