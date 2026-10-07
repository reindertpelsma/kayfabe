# Mode-2 memory model & the path to host parity

User architecture proposal (2026-06-03) + assessment. Primary goal: **host parity**
— the guest's CUDA/graphics run at native speed. The memory model is the blueprint
for that, and the trap-minimization principle is exactly what makes parity possible.

## Address spaces (and what is "real" vs "bookkeeping")
- **Guest userspace VA** — real; what apps use.
- **GPA** (guest physical) — real; guest RAM + the PCI BAR windows live here.
- **BAR window** — bookkeeping/aperture: a GPA range that the PCIe device decodes;
  CPU MMIO to it is real, but the *address* is just "where the aperture sits".
- **GPU virtual (GMMU VA)** — real; the address the GPU engines actually issue.
- **GPU physical (FB offset)** — bookkeeping in Mode-2: emulated sparse FB today;
  becomes the **host GPU's** FB when we forward compute to a real GPU.

The GMMU page tables (rooted at a PDB) translate GPU-VA → GPU-phys (FB, VID
aperture) **or** → GPA (sysmem, SYS aperture). That walk is the core primitive
(already built: nvkvm_walk_pdb, aperture-aware, 4K/64K/2M pages, FB+sysmem tables).

## The core job (same as Mode-1)
For every guest mapping that the GPU must reach, translate down to a real backing
and install it so access is **direct** (no trap). Two access paths, not one:
1. **CPU → BAR MMIO** (registers, doorbell/USERD): CPU-VA → GPA(BAR) → MMIO.
2. **GPU engines → GMMU**: GPU-VA → (FB | GPA) via the page tables.

## The hot path (99%) — MUST be trap-free (this is parity)
Guest userspace maps a context's doorbell/USERD/GPFIFO and submits work by writing
memory — no syscall, no fault. For parity this requires the guest's channel to be
**backed by a real host-GPU channel**, with that channel's USERD/doorbell MMIO
**direct-mapped into the guest's GPA** (via the unprivileged stub's forwarded mmap,
exactly as Mode-1 does — keeps QEMU unprivileged). Then the guest's doorbell write
hits real hardware with zero VMM involvement. Compute/DMA throughput is then at
host parity (Mode-1 already measures ~0% overhead on compute/DMA for this reason;
only the control path has tax).

> Bring-up vs parity: TODAY (Mode-2 bring-up) we TRAP the doorbell and *emulate*
> the copy engine to prove the stock driver runs end-to-end. That is the scaffold.
> The PARITY version replaces emulation with host-GPU-backed, direct-mapped
> contexts. Both share the GMMU-walk translation core; only the backing changes.

## Direct-mappable data (read-native, trap-narrowly)
A GPU-physical range can be backed by a host RAM page the guest reads natively
(as MMIO) — no read trap — iff one of:
- **constant** (chip ID, VBIOS/ROM, static config), or
- **deferred-consistent** (stats/counters where a ms-stale value is harmless), or
- **we can update it atomically** when the driver acts on the device.
Combine freely with writable doorbells on the same page: **don't trap reads, trap
only writes** (page-protection split). Maximize this; trap is the expensive case.

## Page tables (PDB) — prefer shadow-on-invalidate over write-trapping
The proposal's "trap writes, reads-always-latest, allow partial, atomic pointer
swap" is the classic shadow-page-table contract and is correct. **Key refinement:
do NOT trap every PTE/PDE write — that is the hottest write stream during context
setup.** Instead use **shadow-on-invalidate**: let the guest write page tables
freely into (emulated FB | sysmem); re-walk them lazily at the natural sync points
the GMMU *already requires* — the **TLB invalidate** (a register write / RPC the
driver must issue after changing mappings) and the **doorbell** (work submit).
Those are orders of magnitude rarer than PTE writes. This is exactly the "read the
PDB at the point we set the mapping" idea, generalized: invalidation is the trap
boundary, not the write. (We already re-walk on the doorbell today.)

## DMA
GPU-context DMA ranges with SYS-aperture PTEs resolve to GPAs → host VMM VA. To
forward to a real host GPU, register those GPAs (guest RAM, pinned) into the host
GPU's VASpace/IOMMU (RM OS-descriptor of the guest-RAM HVA — the Mode-1 path), so
the host GPU DMAs straight into guest RAM. "Validate VA range" = bind the qemu fd's
CPU RAM to the host GPU context. No copy, no bounce.

## When we DO trap (the irreducible <1%)
- A **kernel-level doorbell that means "do a privileged HW action"** (syscall-like)
  — e.g. channel schedule/teardown, GSP RPC. Trap, forward to host RM (unprivileged
  ioctl), return.
- A **read we cannot pre-satisfy** (a value only the real HW/GSP knows at read time
  and that isn't deferred-consistent). Trap and fetch.
Everything else: direct-map.

## Net
The proposal matches both Mode-1's proven design and standard GPU-virt practice.
Adopt it as the parity blueprint. The one substantive change from the literal
proposal: **page tables are shadowed on TLB-invalidate/doorbell, not on every
write** (cheaper, race-free because invalidation is the architectural sync point).
The translation core needed for all of this is the GMMU walk already implemented
for Mode-2 bring-up; parity reuses it to install host-backed direct mappings.
