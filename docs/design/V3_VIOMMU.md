# V3 vIOMMU — a guest IOMMU in front of the kf3 device (design only)

**STATUS: DESIGN-ONLY, 2026-10-08.** Nothing is built. Prompted by the owner's question whether Windows
needs an IOMMU and what a vIOMMU would cost. Today kayfabe presents no vIOMMU: every bus address the
guest hands the device is a guest-physical address (GPA), and Windows 11 runs without one (its
hardware requirements are Secure Boot and TPM 2.0; Kernel DMA Protection and the DMA part of
virtualization-based security are optional features that use an IOMMU only when firmware advertises
one, and WDDM's "IoMmu" memory model is for GPUs sharing the CPU address space, while discrete NVIDIA
GPUs use the GpuMmu model). Not needed for any current goal; recorded so the cost is not re-derived.

## 1. What changes

With a guest IOMMU (QEMU `intel-iommu` on q35, which supplies the ACPI DMAR table Windows uses) the
device's DMA addresses are IOVAs and the guest's IOMMU tables map IOVA to GPA. Every place kayfabe
consumes a guest bus address gets one extra step, IOVA to GPA, before the existing GPA to host-memory
step. The `USERD` address check that failed in Windows run 61 is a HOST IOMMU property (the GPU's
group sits in a translated `DMA-FQ` domain on the host) and is unrelated to a guest vIOMMU.

## 2. Where the step goes (inventory 2026-10-08: reading, not a run)

- CPU-side reads and writes of guest memory: the `GuestRam` trait (`kf-gsp/src/ram.rs`,
  `read(gpa, ..)`, `write(gpa, ..)`), referenced from about 20 files. An IOVA-aware wrapper at this
  one trait covers the RPC queues, ring reads, and message queues.
- GPU-side: where the VA manager resolves a PTE's system-memory address to a host mapping (the
  memory ledger, `kf-mem/src/ledger.rs`, and `kf-qemu/src/mem.rs`) and the memory-list descriptors
  (`kf-rm/src/memory_list.rs`).
- A contiguous IOVA range maps to scattered GPAs: reads and writes split per page, and "one row per
  contiguous range" splits into runs.
- The translation itself is QEMU's: the PCI device's IOMMU address space and an address-space
  translate call, behind a thread-safe central function with a small cache.

## 3. Invalidation: two different barriers (owner, 2026-10-08)

- The normal GMMU invalidate is the guest's statement that it has finished editing a VA space's PTEs;
  kayfabe uses it as the commit point of the diff.
- An IOMMU invalidate is the guest's statement that it changed IOVA to GPA translations. The GPU PTEs
  are unchanged (they hold IOVAs) but now mean something else, so the GMMU path alone would never
  notice. The IOMMU invalidation is scoped to IOVA map relocations only.
- Design: each host mapping records the IOVA ranges and the GPA runs it was resolved through (a
  reverse index). An invalidation of a domain/range finds the affected mappings; the VA-manager
  thread re-translates them and compares with the recorded runs (equal: nothing; different: remap,
  then a host-side GPU TLB invalidate of that VA space). Comparing results instead of trusting the
  event means a missed or coalesced invalidation cannot leave a stale mapping for long. A generation
  counter per IOMMU domain, checked at each normal PTE commit, is a cheap staleness test.
- Stale window: between the guest's invalidation completing and the re-map the host GPU may still DMA
  to the old GPA page. That page is guest RAM, so it harms only the guest. A strict version would
  hold the invalidation's completion as the register path holds `MMU_INVALIDATE`, but QEMU's
  emulation completes it on the vCPU, which would need a QEMU patch; the lazy form is the first cut.

## 4. Effort and first step

About 4 days to 2 weeks (an estimate; a first working cut in a few days). The first step is a one-day
spike: a `GuestRam` wrapper that is the identity when no vIOMMU is present, the QEMU translate call,
and a Linux boot with `intel_iommu=on` to see which consumers still fail. Testing: Linux in strict
and passthrough modes; Windows with the DMAR table present. New trap surface: the IOMMU's own
registers, which belong to QEMU's `intel-iommu` and not to kf3.
