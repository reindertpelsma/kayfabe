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
