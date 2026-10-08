# VFIO GSP bootstrap observer

**STATUS: RESEARCH, 2026-10-05; device-agnostic since 2026-10-09 (see the next section).** This
directory implements an opt-in diagnostic
QEMU 10.2.4 observer. Local compilation and GPU-free tests pass; no native GPU
runtime result or complete-prefix claim exists.
It changes neither Kayfabe product semantics nor a guest/GPU queue pointer.

> ★ **2026-10-09 — one observer for vfio-pci AND kf3-gpu** (owner ruling `docs/OWNER_RULINGS.md`
> §X: the same tracer for both, not a look-alike; `docs/design/V3_BAR0_TRACE_MODE.md`). The
> `VFIOPCIDevice`-typed body of `qemu/gsp-observer.c` is now the device-agnostic `GspObserver`
> (`gsp_observer_open(PCIDevice *, path, seconds, errp)`, `gsp_observer_mmio`, `gsp_observer_irq`,
> `gsp_observer_reset`, `gsp_observer_close`, `gsp_observer_is_trigger`); the `vfio_gsp_*`
> entry points the integration patch calls are thin wrappers that keep vfio-pci's own
> preconditions and its fixed 300 s window. The kf3-gpu device calls the same functions behind its
> own `x-gsp-observer=` / `x-gsp-observer-seconds=` properties (`qemu/hw/misc/kf3/kf3.c`; linked
> when its `meson.build` finds this version in `hw/vfio`). Output changes: the header line gains a
> `"device"` key (`"vfio-pci"` or `"kf3-gpu"`), which `decode.py` ignores; the observer still
> reports itself as `vfio-bootstrap-580` so the unchanged decoder accepts both. Everything below
> describes the VFIO side as first written and still holds for it. ⊘ "There is no general MMIO
> log" below is about this observer: the BAR0 access records of a reference run come from QEMU's
> own `vfio_region_read`/`vfio_region_write` trace events, which kf3's trace mode emits too.

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

The implemented profile is compiler-derived from OGKM 580.65.06, commit
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
   Require these plus `x-no-vfio-ioeventfd=on` and `enable-migration=off`
   explicitly. With `x-gsp-observer` present the patch skips mmap for BAR0 only;
   BAR1/BAR2 retain their mappings. Global `x-no-mmap` is unnecessary.

Every guest-memory read must translate through the device address space and
reject anything outside ordinary guest RAM before touching bytes, including
RAM-device mappings, ROM, protected RAM and guest-memfd regions. VFIO BAR mappings
have QEMU's RAM flag too, so the separate RAM-device rejection is essential. Never read
MMIO through the memory helper or log host pointers. Guest-IOMMU configurations
without a validated DMA translation profile are refused.

## Bounds and output requirements

This implementation supports one explicitly selected NVIDIA VFIO
PCI device, one 4096-byte page table, at most 512 unique pages, at most 1 MiB per
queue, and ordinary unencrypted RPC version `0x03000000`. Queue elements and
checksums use the already-tested Windows observer parser. Larger/unknown layouts
are refused; this is not cross-family compatibility evidence.

Callbacks do bounded RAM copies into preallocated memory. They never write a
file, allocate per message, sleep, or wait for the output worker. A bounded SPSC
FIFO sends record bytes to a background writer; FIFO saturation, output cap,
unstable DMA snapshots and malformed records have separate counters. Output is
created exclusively with mode 0600 and no final symlink following. Limits are
32 MiB FIFO, 64 MiB reconstructed KGWT bytes including header, 100,000 records,
and 300 seconds from device realization. JSONL hex and metadata expand the file
beyond 64 MiB (bounded below 192 MiB). The duration is checked on observation
triggers; it does not terminate QEMU. There is no general MMIO log.

Hot unplug is blocked while this profile is active. Normal QEMU exit stops the
producer and drains the writer. A kill can leave a truncated trace. Disk errors
are reported on QEMU stderr; a final flush/close error can occur after the footer
was formatted, so accepting evidence requires both strict decoding and a clean
QEMU exit receipt without the observer-output error. `file_export_complete`
means exported records survived framing/hash checks, never complete GPU history.

Use monotonically increasing observation timestamps, device identity, queue
generation and trigger provenance. Every generation starts with an unknown
prefix. Preserve both transport and RPC sequences; Windows commonly uses RPC
sequence zero, so it is not a unique transaction identifier. Reset/reuse must
not pair an old reply with a new allocation at the same address. The VFIO device
reset hook invalidates attachment; a new valid mailbox chain increments the
generation. Reset/resume paths that do not publish a new mailbox chain are not
covered. Prefix flags are retained even when the first observed sequence is zero.

Even with all hooks active, GPU DMA proceeds independently of guest CPU
execution. An interrupt may announce several messages, polling may consume
without a fresh interrupt, and a DMA snapshot can be unstable. Accept only
stable checked records; a first sequence of zero and zero observed gaps are
evidence of coverage, not a mathematical proof of completeness. Save an
uninstrumented health result to detect observer-induced failure/timeout. Each
callback may copy up to roughly 4 MiB of RAM and runs with the BQL, so timing
perturbation is material despite the absence of file I/O or waits in that path.
The inherited `invalid_elements` counter includes empty/stale non-message slots
examined again on later snapshots; it is not a malformed-submitted-RPC count.

## Apply, build and decode

Use a separate QEMU checkout/build directory. This installer hash-checks the five
upstream VFIO source files and `VERSION`; unrelated changes such as kf3 additions
are allowed. It refuses existing observer files. It neither builds nor replaces
an executable. The patch and own source files are separate intentionally.

```sh
python3 tools/vfio-gsp-observer/apply.py /path/to/qemu-10.2.4 --check
python3 tools/vfio-gsp-observer/apply.py /path/to/qemu-10.2.4
# Configure the separate build with the same options as the comparison baseline.
ninja -C /path/to/qemu-10.2.4/build qemu-system-x86_64
```

Add these options only to the selected NVIDIA graphics function, with a fresh
output path whose parent already exists. This first profile rejects guest-IOMMU,
mdev, migration and big-endian configurations.

```text
-device vfio-pci,host=0000:01:00.0,x-gsp-observer=/fresh/run/gsp.jsonl,x-no-kvm-intx=on,x-no-kvm-msi=on,x-no-kvm-msix=on,x-no-kvm-ioeventfd=on,x-no-vfio-ioeventfd=on,enable-migration=off
```

Keep the exact source revision, executable hash, QEMU command, host PCI identity,
Windows/driver/firmware versions, fixture hash, and phase timestamps beside the
trace. Use QMP quit after the bounded workload, preserve stderr and process exit,
then decode. Do not infer readiness merely from file creation: output is buffered.

```sh
python3 tools/vfio-gsp-observer/decode.py /fresh/run/gsp.jsonl --jsonl > /fresh/run/decoded.json
# --all-records preserves the full decoded payloads and generation/trigger metadata.
# --require-query-pair exits 4 unless one unambiguous successful pool pair exists.
```

Trigger ids are 1 bootstrap mailbox, 2 command doorbell, 3 host interrupt before
injection, and 4 GSP interrupt-status/ack MMIO. `source_sha256` covers canonical
KGWT header/records/payloads. `observations_sha256` additionally inserts two
little-endian uint64 words (generation, trigger) before each record. The supplied
decoder checks both, preserves metadata and never pairs across generations.
Checksums detect corruption; they do not establish capture completeness.

## Local validation and provenance

Portable queue parsing and the binary record ABI are copied from this project's
Windows observer at `572411c1`, without NVIDIA implementation code. `decode.py`
starts from that revision and adds VFIO metadata validation/generation pairing.
The new bootstrap path uses compiler-derived public layouts in `layout-580.json`.
`check-layout.py` checks TU102 and GA102 against the pinned OGKM revision.

```sh
tools/vfio-gsp-observer/test.sh
python3 tools/vfio-gsp-observer/check-layout.py /path/to/ogkm-580.65.06
python3 tools/vfio-gsp-observer/tests/test_apply.py /path/to/qemu-10.2.4
ninja -C /path/to/qemu-10.2.4/build libqom.a
python3 tools/vfio-gsp-observer/tests/test_writer.py /path/to/qemu-10.2.4/build
```

On 2026-10-05, GCC and Clang ASan/UBSan core tests passed; the exact upstream
QEMU 10.2.4 x86_64 system target compiled and linked locally. The production
writer test exceeds the FIFO size, verifies 6,500 payloads through wrap/drain,
checks both hashes and strict decoding, refuses metadata corruption/truncation,
and exercises `/dev/full`, both output caps, saturation and the final-stop race.
It uses real QEMU memory predicates against ordinary, RAM-device, ROM, non-RAM,
protected and guest-memfd region states. This is not a live address-space/PCI test.
Installer tests cover hash mismatch, existing-file refusal, successful application
and preservation of unrelated source changes. No GPU runtime result is implied.

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
