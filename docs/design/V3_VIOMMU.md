# V3 — the guest's IOMMU (and the host's)

**STATUS: DESIGN-ONLY, 2026-09-25.** Owner: *"just write it down … under kayfabe the security is
slim compared to bare metal GPU, but it's to support any stock OS."* ⇒ vIOMMU support is a
**compatibility** requirement (a stock guest may enable one), not a security feature. Nothing
below is built. Today a guest that puts a vIOMMU in front of kf3 would have its GPU's
system-memory DMA land at the wrong addresses (see §3).

## 1. What a GPU page table holds for system memory

The NVIDIA driver never writes a physical address for sysmem. It pins pages and calls the kernel
DMA API — `dma_map_page_attrs` / `dma_map_sg` (`ogkm-580 kernel-open/nvidia/nv-dma.c:64, :222`,
`nv_dma_map_pages` `:439`, `nv_dma_map_sgt` `:363`) — and writes the returned `dma_addr_t` into
the sysmem PTEs. What that value is depends on the IOMMU mode of the machine the driver runs on:

| IOMMU mode | `dma_addr_t` = | the device can reach |
|---|---|---|
| none, or `iommu=pt` (identity domain) | the physical address | all of RAM |
| translated (DMA / DMA-FQ domain) | an IOVA; the IOMMU translates it to the physical address | only what the kernel `dma_map`'d (+ stale entries until the IOTLB flush) |

⊘ Terminology: `iommu.strict=1` is the **IOTLB invalidation policy**, not the translate-or-not
switch. Lazy mode (DMA-FQ, a common default) defers flushes, so a just-unmapped page stays
reachable briefly; strict closes that window. Either way a device keeps every mapping the driver
legitimately made — and a GPU driver maps a lot.

## 2. The host's IOMMU — covered by construction

kayfabe is an unprivileged host process and never sees an HPA or a host IOVA:
- guest RAM reaches the host GPU as an OS descriptor over the guest memfd (`kf-qemu mem.rs
  guest_ram_object`): host RM pins it and `dma_map`s it through the host kernel;
- the store is host vidmem that host RM maps.

So the host's IOMMU mode needs nothing from us. ⚠ It does set the **stakes of a bug**: with
`iommu=pt`, anything that makes our host channel emit a PHYSICAL address reaches any host RAM
(the subchannel hole, `00f62991`, would have). ⇒ `DENY_PHYSICAL_MODE_CE` on every host channel we
birth is the guarantee that holds on `iommu=pt` hosts too; it must never be dropped.

## 3. The guest's IOMMU — the gap

The guest driver runs the same `dma_map` inside the guest, against our emulated device:

- **No vIOMMU (today's q35 without `intel-iommu`):** `dma_addr_t` = guest-physical. The walker's
  sysmem leaf is a GPA → `RamMap::file_range` → memfd offset → host RM (host kernel's IOVA below).
  This is the translation chain kf3 implements.
- **vIOMMU present** (common: clouds with >255 vCPUs need x2APIC interrupt remapping; any guest
  wanting DMA isolation from its devices): the guest's PTEs hold **guest IOVAs**. kf3 treats them
  as GPAs ⇒ wrong memory.

### Double translation — the design

```
guest PTE (guest IOVA) ──vIOMMU tables (guest-owned)──▶ GPA ──RamMap──▶ memfd offset ──host RM──▶ host IOVA/HPA
```

1. **Address space.** kf3 obtains its DMA address space with `pci_device_iommu_address_space()`
   (QEMU; Cloud Hypervisor: virtio-iommu's equivalent) instead of assuming system memory.
2. **Translate per run, off the vCPU.** The VA-manager thread translates each walked sysmem run
   IOVA → GPA through that address space (`address_space_translate` / the IOMMU region's
   `translate`), splitting a run where the vIOMMU mapping is discontiguous. A run the vIOMMU does
   not map is a **fault** for the guest (as on real hardware: an IOMMU fault, not a read of
   whatever is there) — refused by name, never mapped.
3. **Invalidation is a sync point.** Register an IOMMU notifier: a guest vIOMMU unmap/invalidate
   must unmap our host rows for every run it covers **before** the guest's invalidation completes
   — the same shape as the MMU_INVALIDATE trigger (`THE_THREE_SYNCHRONIZATION_POINTS`). Mapping
   and unmapping stay authored host verbs; nothing guest-chosen reaches a host flag.
4. **Scope.** The CPU windows (BAR1/BAR2/PRAMIN) are guest-physical MMIO and are not behind the
   vIOMMU; only the device's DMA (sysmem leaves, USERD/GPFIFO in guest RAM, semaphores) is.
5. **Per family.** Nothing here depends on the GPU family; the vIOMMU is a VMM property.

### Until it is built

kf3 should **refuse at realize** when it sits behind a vIOMMU (the device's DMA address space is
not system memory), naming the reason, rather than silently mistranslating. ⊘ Not built either.

## 4. What this is and is not

- It adds protection **for the guest against its own (emulated) GPU**. It does not strengthen host
  isolation, which is bounded by host RM + the host IOMMU + `DENY_PHYSICAL_MODE_CE` (§2).
- Without a vIOMMU the device can reach all guest RAM — kf3 pins the whole guest memfd as one OS
  descriptor, the same posture as any emulated DMA-capable device.
