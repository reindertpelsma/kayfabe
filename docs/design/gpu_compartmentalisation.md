# GPU compartmentalisation — the provider-VM hardening mode

**Owner design, 2026-08-09. ⊘ NOT scheduled.** This is a *second hardening mode* to be built well
after Mode-1 parity. It is written down now for one reason only: **one decision taken today keeps it
cheap, and taking the other decision makes it a rewrite** (§6).

⚠ Nothing here has been measured. It is a design, and the costs in §5 are estimates that **have not
been verified**.

## 1. The problem it solves

Today the **host** runs `ogkm` and the isolate is a host process. Two exposures follow:

1. An exploit in the host driver is reachable through our VMM extension by a malicious guest.
2. ★★ **The GPU has DMA to host RAM.** The IOMMU is programmed *by the NVIDIA driver*, which we do
   not control and cannot audit. ⇒ A GPU-side compromise — malicious firmware, or an RM bug that
   lets a guest steer a DMA target — reaches host memory.

⊘ **There is no cheaper mitigation.** You cannot fix (2) by configuring the IOMMU differently,
because the closed driver programs it. **Putting the driver behind VFIO in a VM is the only
mechanism that yields hardware-enforced DMA containment given a host driver we do not control.**

## 2. The shape

- A **minimal provider VM** — stripped kernel, `ogkm`, the Rust isolate userspace. ⊘ No network, no
  disk.
- The GPU is assigned to it by **VFIO passthrough**; the host has **no NVIDIA driver at all**.
- ★ **The isolates live only in the provider VM.** Every real RM ioctl happens there.
- Guest RAM is registered into the provider VM as memory slots, so `ogkm` can map it for DMA.
- Guest VMMs send RM actions to the provider VMM; results come back as the same **virtual
  references** the guest already receives today.

★★★ **The guest observes no difference, and that is not a coincidence** — kayfabe already gives the
guest only virtual references, never a real fd. A design in which the guest ever held a real
`/dev/nvidia*` fd could not do this at all. **The day-one indirection is what makes this mode
possible.**

## 3. What it buys

- **True DMA containment.** GPU DMA is confined by the IOMMU to the provider VM's memory. A
  compromised GPU reaches the tenants using it — ⊘ not the host, ⊘ not unrelated VMs.
- ★★ **A property vGPU structurally cannot offer.** Under vGPU the **host** runs a privileged
  driver, so a GPU privilege escalation is a **full host takeover**. Here the same escalation yields
  a VM with no network and no disk, still behind KVM + VFIO.
- **Containment of NVIDIA's own bugs.** If `ogkm` or GSP is ever exploitable, the attacker owns the
  provider VM and possibly the GPU — a blast radius no worse than a 1-VM-per-GPU passthrough
  deployment, which is the thing being replaced.
- **The kayfabe property is preserved**: the guest still never touches real hardware, and the RM
  calls that do happen arrive **unprivileged**, so firmware flashing and persistent setting changes
  remain refused by RM itself.

⇒ Best of both: **passthrough's containment with vGPU's sharing**.

## 4. ⚠ What it does NOT buy — state this, do not sell around it

**The provider VM is a shared trust boundary across every tenant on that GPU.** Guest RAM must be
mapped into it for DMA, so a compromised provider VM reads all of them.

⊘ **This cannot be fixed by running one provider VM per tenant** — VFIO assigns the device to
exactly one VM, and the provider talks to the real GPU through MMIO. **One provider VM per GPU is
the only possible topology.**

⇒ The honest description: **vGPU's sharing model with passthrough's containment.** One driver
instance serving many tenants, living in a sacrificial VM instead of the host. That is a real
improvement over the host being the boundary, and it is **not** per-tenant isolation.

## 4b. ★★ Can a guest map a BAR without the VMM mapping it? — ANSWERED, and the answer is no

The natural question: if the provider VM owns the BARs, can guests get native-speed mappings of
them **without** the host VMM also holding a mapping? Researched against Linus master and this
box's headers.

**The hardware is not the constraint — confirmed.** EPT/NPT map GPA → HPA, and MMIO is in the host
physical address space by definition. Shipping proof rather than a spec reading:
`KVM_MEMSLOT_GMEM_ONLY` slots already program EPT/NPT **from a bare PFN with no VA in the path**,
and `__kvm_is_mmio_pfn()` picks the memory type from the PFN. Intel and AMD differ only on memory
type (`vmx_get_mt_mask()` forces UC; SVM defines no `get_mt_mask`) — ⊘ no asymmetry that decides
viability.

⊘ **The constraint is KVM's ABI, and there is no upstream mechanism.**
- `guest_memfd` **cannot** do it, structurally rather than by omission: `kvm_gmem_get_folio()`
  allocates through `__filemap_get_folio_mpol()` and returns `folio_file_pfn()` — page-allocator
  PFNs, which cannot reference a BAR. There are **zero** `dma_buf` references anywhere in KVM.
  ⚠ And upstream is moving the *other* way (`GUEST_MEMFD_FLAG_MMAP`, making gmem mappable).
- ★ **The exact mechanism exists as an unmerged RFC**: `KVM: Support vfio_dmabuf backed MMIO region`
  (Xu Yilun, 2025-05-29) adds `KVM_MEM_VFIO_DMABUF`, replacing `userspace_addr` with a
  `struct dma_buf_attachment *`, expressly to *"eliminate userspace mapping"*. **Last posted
  2025-05-29; nothing newer.** ⚠ And it is gated on `kvm_arch_has_private_mem()` — **a plain
  VMX/SVM VM gets `-EINVAL`.** Kernel 6.19 merged only the PCI/TSM layer beneath it; TDISP /
  private MMIO is explicitly the next phase.

⚠ **And it is not an out-of-tree module.** KVM does not *fundamentally* assume an HVA —
`__kvm_mmu_faultin_pfn()` already branches to a PFN-only backend — but there is no extension point:
the flag check is closed in `check_memory_region_flags`, the dispatch is `static`, and master gates
internals behind `EXPORT_SYMBOL_FOR_MODULES(sym, "kvm-intel,kvm-amd")` — **a named allowlist an
out-of-tree `.ko` cannot link against.** ⇒ **A KVM patch, not a module.** The RFC's delta is ~200
lines, plus dropping the CoCo gate, which is the part upstream has not agreed to.

⚠ Also measured, and it kills the obvious workaround: **mmu_notifiers are HVA-indexed**
(`hva_start = max(range->start, slot->userspace_addr)`), so the VMA is the *invalidation channel*,
not merely an address supply. ⊘ "Map it, then `munmap`" cannot work.

### ⇒ What this changes, which is less than it sounds
★★ **The design's main claim does not depend on it.** GPU → host DMA containment comes from the
**IOMMU**, which is indifferent to whether any process holds an HVA for a BAR. The BAR question is
about a *different and lesser* risk: host → GPU MMIO.

★ And the correct strength of the claim is weaker than "unmappable by the host": `ioremap` and
`pci_resource_start` are always available to the **kernel** (`CONFIG_IO_STRICT_DEVMEM` restricts
only `/dev/mem`, and only for idle ranges). TEE-IO's own changelog calls host-inaccessibility of
private MMIO *"not that critical but nice to have"*. ⇒ Claim only: **"not reachable through the VMM
process's address space."**

### ★ The tractable near-term lever
`vfio/pci: Add mmap() for DMABUFs` (v5, 2026-07-15) lets a primary process vend **range-limited,
revocable** BAR handles by fd to subordinate processes instead of sharing the whole device fd.
⊘ Still a VMA, so it does **not** answer the question above — but it is the realistic way to shrink
VMM BAR authority in this topology, and it is on its way upstream rather than stalled.

## 5. Costs, all `unverified`

- ★ **Every RM ioctl gains a round trip**: guest vmexit → guest VMM → provider VM → `ogkm` → back.
  Not merely a VM entry — the provider VM's vCPU must also be **scheduled**.
- ⚠ ★★ **But the C measured that the control path is the entire tax and the data path is free** —
  `docs/PARITY_PLAN.md` and the C-era parity harness record GEMM 1.00×, LLM 0.97×, DMA 0.93×
  byte-exact, ~0 % compute/DMA overhead, with allocation named as the remaining target. ⇒ This design taxes
  **exactly the path that was already the bottleneck** and leaves steady-state compute untouched,
  because mappings are native once established. **Survivable for inference; painful for
  allocation-heavy workloads** — many small contexts, frequent map/unmap. ⊘ Benchmark that shape
  before committing.
- **You give up host compute entirely.** No CUDA on the host, by construction.
- The extra VM is real: a kernel, a driver, a userspace, and their lifecycle.
- ⚠ **The hard engineering piece**: mapping GPU BAR pages into a guest at native speed. The provider
  VM owns the BARs via VFIO, but **only the host can install memslots in a guest** ⇒ the host stays
  in the *memory-plumbing* path even though it is out of the *driver* path. Not per-call, but this
  is where the design will fight the kernel.

## 6. ★★★ The one decision to protect TODAY

**Keep the isolate boundary "messages + explicitly registered memory regions". ⊘ Never "pass an
fd".**

The control plane is already portable — `kayfabe-isolate-host` talks over `UnixDatagram::pair()` /
`UnixStream::pair()`, so socketpair → vsock is a **transport swap, not a redesign**. Every fd newly
handed across that boundary is a piece of this design that would have to be unbuilt.

⇒ Hold that line and the provider VM stays a **deployment option**. Break it and it becomes a
rewrite. ⊘ This is the only part of this document that constrains work before Mode-1 parity.

## 7. Positioning

★ It is a **second mode**, not a replacement: opt-in hardening for multi-tenant hosting, with a
documented latency cost and no host compute. The default deployment stays as it is.
See `PRODUCT_POSITIONING.md` §2 for why hostile-guest isolation is the thing being sold.
