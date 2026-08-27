# Mode-2 Data-Plane Architecture Decision

**Status:** SETTLED (user + engineering consensus, 2026-06-04). No open architectural
decision remains — what follows is the agreed model; the rest is execution.

**Scope:** how Mode-2 (stock open NVIDIA driver in a KVM guest against the emulated GA106,
compute forwarded to a real host GPU) backs the GPU-addressable memory that compute touches —
context buffers, channel buffers, sysmem DMA targets, and the launch path.

This doc is the canonical reference. Companions:
[mode2_address_virtualization.md](mode2_address_virtualization.md) (the GPU-physical
bookkeeping model), [mode2_uvm_residency.md](mode2_uvm_residency.md) (residency ownership),
[mode2_compute_forwarding.md](mode2_compute_forwarding.md) (the cuCtxCreate bring-up thread),
[mode2_plan.md](mode2_plan.md) (phase plan).

---

## 1. The frame: we ARE GSP

The emulated device plays the role the real GSP plays for the guest's open driver. The guest
kernel talks GSP-RPC to us; we answer. The decisive consequence:

- **GPU-physical address space is OUR bookkeeping.** It is a private contract between the
  guest kernel and QEMU. Proprietary libcuda — running in guest userspace — never observes a
  GPU-physical address. It sees GPU *virtual* addresses and the results of compute.
- Therefore **we do not have to replicate real GSP behavior.** We only have to satisfy the
  *constraints* the open guest driver places on GSP's replies. Those constraints are readable
  from `open-gpu-kernel-modules` source. Where our physical layout differs from a real GSP's,
  nothing in the guest can tell — so it doesn't matter.
- The earlier "GSP-internal / unobservable wall" framing was **wrong**. GSP can only map what
  the kernel *tells* it to map; every mapping is communicated over RPC. What we had was a
  *parse gap* (we hadn't decoded the request), not a fundamental observability wall.

## 2. Production constraint: stock driver, unprivileged QEMU

- **Production runs the stock, unmodified NVIDIA driver in the guest.** This is non-negotiable:
  it is what lets a closed KMD (Windows, the proprietary Linux KMD) load against the emulated
  device. Any guest-kernel shim (e.g. `docs/kernel_patches/mode2_uvm_complete_proof.patch`) is
  **debug-only** — a diagnostic to crack a wall faster, never shipped.
- **QEMU is unprivileged in production** and issues only *unprivileged* NVIDIA ioctls on the
  host. Debugging may use privileged probes (e.g. to read host GR buffer layout), but no
  production code path may depend on a privileged host call.
- Priority order for all Mode-2 work: **correctness (crashes) → security → performance.**

## 3. The partition rule (decidable per page)

Every GPU-addressable page falls into exactly one bucket, decided by whether the guest kernel
maps it into guest *userspace*:

| Bucket | Examples | Handling |
| --- | --- | --- |
| **Mapped into guest userspace** (~99% of transactions, the hot path) | sysmem DMA buffers, userspace-visible context/channel buffers, push buffers, USERD-adjacent userspace mappings | **Forward unprivileged** — back the guest GPA with real host GPU/host memory via the Mode-1 GPA-window double-mmap + KVM memslot. Zero-copy, no per-access trap. |
| **Kernel-only** | doorbell / PDB setup, RM control ioctls, instance blocks, page-directory setup | **Trap** to QEMU and emulate / forward as a control transaction. Not on the per-access hot path. |

The rule is decidable from the guest driver source: if the driver maps the buffer to
userspace, we forward it; if it keeps it kernel-private, we trap it.

## 4. Sysmem DMA: SOLVED (Mode-1-proven os-descriptor path)

When guest userspace wants the GPU to DMA into its own memory, the **guest kernel has already
done the hard part** — it pinned the userspace pages and translated userspace-VA → GPA. QEMU
finishes it, unprivileged:

```
guest GPA
  -> iterate KVM user memory regions -> VMM virtual address (the pinned host page)
  -> NV01_MEMORY_SYSTEM_OS_DESCRIPTOR (class 0x00DE) over that VMM VA
  -> RM_MAP_MEMORY_DMA into the host context's VASpace AT THE GUEST'S GPU-VA
```

This is the exact mechanism Mode-1 already proves. **Discipline:** revoke the os-descriptor on
unmap, so a freed guest page can never be reached through a stale GPU-VA.

## 5. Vidmem GR-context (golden context): separate, smaller case

The RM-internal GR golden-context buffer is *not* the sysmem hot path and has no userspace
handle. It is FECS-lazy: the host shadow loads it when its channel first runs. Back the guest's
GPU-VA for it with **host vidmem** via `RM_MAP_MEMORY_DMA` (the `nvkvm_m2_host_alloc_map_vidmem`
primitive + KVM memslot). One-time setup, not per-access.

## 6. Performance: parity assessment

Engineering judgment, Mode-2 vs host:

- **Compute throughput — PARITY.** Kernels run on the real host GPU; memslot-backed data means
  no copy, no trap. Same as Mode-1's measured ~0% overhead.
- **DMA / HtoD / DtoH — PARITY.** Same memslot mechanism.
- **Launch / submission latency — PARITY iff the doorbell/USERD page is memslot-backed** (host
  GPU's doorbell page mapped into the guest BAR) rather than MMIO-emulated. Today's bring-up
  scaffolding TRAPS the BAR (every access → QEMU); that must converge to a memslot for parity.
- **Completion — parity** via memslot semaphore-poll; a small vIRQ-injection tax if
  interrupt-based (same in both modes).
- **Structural risk — dynamic UVM.** Mode-1 runs our cooperative driver with static mappings
  we control. Mode-2 runs the stock driver doing its own UVM demand-paging / migration / fault
  buffers; our reverse-driver page-table translation must be re-shadowed on every remap.
  Static/resident device memory = one-time setup (no hot-path tax). UVM-thrash /
  oversubscription workloads = a per-event tax Mode-1 avoids. GSP-RPC + interrupt reflection add
  control/setup-path latency, not throughput.
- **Bottom line:** throughput parity reachable; launch near-parity with the doorbell
  memslot-backed; the inherent cost of "run the unmodified driver" is shadowing its dynamic
  address-space management. The UVM-thrash delta is unmeasurable until a data plane runs.

## 7. Current bring-up status & the next concrete step

- cuCtxCreate crashes in libcuda compute-context finalization (SIGSEGV at .so offset
  `0x466560`, `rbp=0`), right after the `AMPERE_COMPUTE_B` (0xc7c0) RM_ALLOC. Host-vs-guest gdb
  of the *same* libcuda fn proves this is **guest-specific corruption** inside the
  `0x47acc0 → 0x497b50` RM_ALLOC path (host preserves rbp across the call 8×; guest → rbp=0),
  **not a libcuda bug**. Likely a variable-size stack alloc sized from un-backed GR-context GPU
  state. Ruled out: forge gaps (filled, no change), value smash, exceptions, wrong reads,
  writeback divergence. **Black-box libcuda RE is exhausted.**
- The clean "read host GR buffer layout" shortcut is **privilege-blocked**:
  `GR_GET_CTX_BUFFER_INFO` (0x20801219) and `GET_SURFACE_PHYS_ATTR` (0x410103) both return
  `0x1b` INSUFFICIENT_PERMISSIONS to unprivileged QEMU.
- The FB→host overlay foundation is committed but inert (no ranges backed yet).

**NEXT STEP — the source pass (replaces black-box gdb):** read `open-gpu-kernel-modules`
(`kernel_graphics_object.c`, `kernel_graphics_context.c`, `kernel_channel.c`, `kgraphics_*`) to
extract, definitively:

1. the compute-context buffer **forward-list** — which buffers exist and which are mapped into
   guest userspace (→ forward) vs kernel-only (→ trap);
2. the **GSP-response constraints** the guest driver requires from our replies.

Then back the userspace-mapped buffers via the GPA-window double-mmap (overlay foundation
already in place), forge per the extracted constraints, and re-test `cup2`. No more guessing.
