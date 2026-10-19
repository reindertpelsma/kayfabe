# R1 predeclared hypotheses (2026-10-19)

STATUS: RESEARCH. No GPU/VM experiments authorized in this lane.

- H1 (INFERRED): the existing serial CpuIntr implements the public register model for every family. FALSIFIER: any differential state/read/raise mismatch, or a missing/double message in the single-edge exhaustive cases. Production defects require a separate EXPECTED RED commit before a fix.
- H2 (INFERRED): naive deferred writes are not equivalent. FALSIFIER: exhaustive counterexamples and randomized replay find no state or delivery difference when W1C/TRIGGER cross a latch. Preserve counterexamples as tests; do not change the hardware contract.
- H3 (INFERRED): ordered mask-only replay with synchronous barriers preserves eventual messages by the next drain. FALSIFIER: a pending/enable/latch schedule misses a synchronous message or differs in final state. This is NOT a claim of coherent untrapped reads or guaranteed drains.
- H4 (INFERRED): RAM stores lose register alias semantics/order. FALSIFIER: a plain-memory representation implements W1C, both SET/CLEAR aliases and immediate readback without intercepting the stores.
- H5 (INFERRED): KVM coalesced MMIO avoids some userspace returns, not hardware exits; generic QEMU flushing is incompatible with the vCPU blocking rule. FALSIFIER: the source handles writes without a hardware exit, or all flushing/callback paths are demonstrably off-vCPU and lock-free on the vCPU side.
- H6 (INFERRED): existing traces identify the traffic mix but do not prove read freedom or all masked-pending transitions. FALSIFIER: complete ordered reads/writes and pending/enable before/after state with loss counters exist in the retained corpus.
- H7 (INFERRED): ioeventfd is already available but not enabled in the measured trap arm. FALSIFIER: source lacks registration/consumer or ledger/launch data establishes fast-path use.
- H8 (INFERRED): AVIC effectiveness and an external KVM device module require more than a CPU flag. FALSIFIER: active VM evidence proves accelerated injection, or the host exports a supported complete registration/lifetime interface.
- H9 (INFERRED): a capability-limited, no_std replay implementation can keep locks/allocations/syscalls unspellable. FALSIFIER: adding a lock compiles in the no_std gate or the compiled reachable symbol audit admits a lock/allocation/syscall.

Owner addition: identify execution thread and all vCPU-path locks/BQL/futex/mutex/allocations/syscalls for every option. Reject waiting for a worker or lock. Do not wire a replay experiment into the device.
