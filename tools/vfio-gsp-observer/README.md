# VFIO GSP bootstrap observer

**STATUS: DESIGN-ONLY, 2026-10-05.** This directory specifies a diagnostic-only
QEMU 10.2.4 observer. No native GPU runtime result or complete-prefix claim exists.
It changes neither Kayfabe product semantics nor a guest/GPU queue pointer.

## Why the existing capture misses initialization

The Windows physical-RAM scanner successfully recorded GSP requests and replies,
but first found the boot queues at request sequence 2707. A later controlled
PnP-restart capture found two allocations at sequences 3714 and 2868. No FIFO
drops after attachment do not recover those prefixes. Starting a fresh observer
while NVIDIA is disabled avoids polling an old live allocation, but the new
allocation still must be discovered by a full-RAM scan after enable. Repeating
that approach cannot prove complete initialization.

QEMU `vfio_region_read/write` with `x-no-mmap=on` records CPU MMIO from boot.
It does not record ordinary guest-RAM writes or GPU DMA, so it does not itself
capture GSP payloads. The observer uses the MMIO events to locate and sample
the published shared-memory queues at causal boundaries.

## Exact source and trigger chain

QEMU upstream tag `v10.2.4` resolves to
`3e0bcba1ca7d6607ca49a988d165f052a3a53323`. The borrowed PC's unmodified
`hw/vfio/pci.c`, `region.c`, and `pci.h` match this checkout byte for byte:

| File | SHA256 |
|---|---|
| pci.c | `4ffc778a881310aaa128f66ec5326fcd53e2240cfccfee32f215fba316993b66` |
| region.c | `49f6e06bdba3e3bfa57650d36d5bb9bc28681cc5c957b087fb3b220314d58240` |
| pci.h | `73498496a8790b41df6becc8e1cb8ae6e5e86bec561a714e282f15e93e4cba74` |

The proposed profile is compiler-derived from OGKM 580.65.06, commit
`307159f2623d3bf45feb9177bd2da52ffbc5ddf9`:

1. `kernel_gsp_tu102.c:kgspProgramLibosBootArgsAddr_TU102` writes the physical/DMA
   address of LibOS init arguments to `NV_PGSP_FALCON_MAILBOX0/1`. Published
   TU102/GA102 headers name BAR0 offsets `0x110040/0x110044`.
2. `kernel_gsp.c:kgspSetupLibosInitArgs_IMPL` creates the `RMARGS` entry in the
   bounded 4096-byte LibOS argument array. Its numeric id is constructed by
   `_kgspGenerateInitArgId`, not by guessing the in-memory string byte order.
   Require contiguous SYSRAM, the compiled struct stride, and valid size.
3. `GSP_ARGUMENTS_CACHED.messageQueueInitArguments` provides the shared page-table
   address, entry count, and command/status offsets. `NvLength` is 64-bit in this
   profile; treating these offsets as adjacent 32-bit fields is incorrect.
4. `message_queue_cpu.c:GspMsgQueuesInit` includes the page table itself as its
   first physical page. Validate every unique aligned page and the entire queue
   geometry. The status header can still be zero before firmware starts: bind
   validated page-table geometry first, validate each directional queue header
   when that direction becomes initialized.
5. Before forwarding mailbox/command-doorbell writes, sample the command queue.
   `kernel_gsp.c:_kgspRpcSendMessage` publishes the message then calls
   `kgspSetCmdQueueHead`; the TU102 implementation writes the published
   `NV_PGSP_QUEUE_HEAD(i)` register (`0x110c00+8*i`). The initial async messages
   are queued before bootstrap, so sample them as soon as RMARGS is located.
6. Sample status messages before userspace INTx/MSI/MSI-X injection, and at GSP
   interrupt-status/acknowledgment MMIO boundaries. QEMU already exposes
   `x-no-kvm-intx`, `x-no-kvm-msi`, `x-no-kvm-msix` and `x-no-kvm-ioeventfd`.
   Require these and `x-no-mmap` explicitly; otherwise IRQFD or mmap can bypass
   the observer. No automatic change to those device options is proposed.

Every guest-memory read must translate through the device address space and
reject anything outside ordinary guest RAM before touching bytes. Never read
MMIO through the memory helper or log host pointers. Guest-IOMMU configurations
without a validated DMA translation profile are refused.

## Bounds and output requirements

The proposed first implementation supports one explicitly selected NVIDIA VFIO
PCI device, one 4096-byte page table, at most 512 unique pages, at most 1 MiB per
queue, and ordinary unencrypted RPC version `0x03000000`. Queue elements and
checksums use the already-tested Windows observer parser. Larger/unknown layouts
are refused; this is not cross-family compatibility evidence.

Callbacks do bounded RAM copies into preallocated memory. They never write a
file, allocate per message, sleep, or wait for the output worker. A bounded SPSC
FIFO sends record bytes to a background writer; FIFO saturation, output cap,
unstable DMA snapshots and malformed records have separate counters. Output is
created exclusively, has an explicit byte/record cap, and is drained on clean
device teardown. Disk errors mark the result failed. A killed QEMU can leave a
truncated trace; strict decoding must refuse it rather than count it complete.

Use monotonically increasing observation timestamps, device identity, queue
generation and trigger provenance. Every generation starts with an unknown
prefix. Preserve both transport and RPC sequences; Windows commonly uses RPC
sequence zero, so it is not a unique transaction identifier. Reset/reuse must
not pair an old reply with a new allocation at the same address.

Even with all hooks active, GPU DMA proceeds independently of guest CPU
execution. An interrupt may announce several messages, polling may consume
without a fresh interrupt, and a DMA snapshot can be unstable. Accept only
stable checked records; a first sequence of zero and zero observed gaps are
evidence of coverage, not a mathematical proof of completeness. Save an
uninstrumented health result to detect observer-induced failure/timeout.

## Matched-run comparison

Use at least three fresh native/VFIO overlays and three fresh Kayfabe overlays
from the same Windows fixture, with identical driver hash, firmware policy,
memory/CPU topology, auxiliary VGA, network, registry and workload sequence.
Pin and record the exact QEMU/Kayfabe binaries and all experiment flags. The
`b431aeaf` L baseline has GOP off, auxiliary VGA on, and all six diagnostic flags
enabled; its TMO constructor mode refuses every display method deliberately.

Separate boot, explicit StartDevice/restart, steady idle, and graphics trigger
phases. Compare within each lane first, then compare stable between-lane
differences. Report occurrence frequency and ordering variability; never flatten
all repeated calls into a set and call absence meaningful in a partial prefix.

Normalize observation timestamps to phase origins, queue generation/address to
allocation ordinals, object/client handles to lifecycle-relative ids, and known
guest physical addresses to typed symbolic regions. Keep raw data alongside the
normalization. Do not normalize away result codes, declared sizes, capability
bits, engine/class ids, alignments, topology, or unknown bytes merely because
they differ. Pair repeated sequence-zero controls by generation, operation,
object lifetime, order and uniquely matched request fields; flag ambiguity.

`KF3_RPC_TRACE=1` currently logs request ids and policy results, not full request
and reply bytes or every posted asynchronous/deferred message. A future Kayfabe
payload recorder belongs at accepted command extraction and actual status-queue
posting in `kf-gsp/src/boot.rs`, with the same nonblocking bounds, not only at
`kf-rm::ControlCensus::respond`. `rpc_diff.py` is a count/refusal census, not a
payload differential tool. `KF3_DISPLAY_TRACE=1` adds display method/latched/copy
observations but cannot substitute for capabilities read directly by Windows.

For display comparison, derive the caps window from `NV_PDISP_FE_SW` and each
VM's actual BAR0 assignment, then decode through the matching compiler-derived
`NVC773_*` rows. Keep full named capability words, per-head/window counts, masks
and repeated phase snapshots. Monitor reads introduced by the harness belong
to a separately marked observation phase. Native allocations, GSP display RPCs,
direct caps-page reads, and display pushbuffer methods are distinct boundaries;
GSP tracing alone covers only one of them.
