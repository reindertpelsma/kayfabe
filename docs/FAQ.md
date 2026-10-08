# kayfabe FAQ

**STATUS: LIVE, 2026-10-08 (non-stall interrupt entry added the same day).** Questions the VM community asks first, answered from the design and the
evidence in this repository. A statement carries its evidence or says it is inferred.

## Does kayfabe need an IOMMU? Is VM isolation weaker than with VFIO?

**Short answer:** kayfabe's isolation does not depend on an IOMMU, and that is not a weakness in the
way it would be for VFIO, because the guest never drives the physical GPU.

**Why VFIO needs one.** With VFIO the guest's driver programs the real device and can write any bus
address into a descriptor or register. The device then DMAs wherever it is told. The IOMMU is the only
hardware that stops a malicious guest driver from pointing the GPU at arbitrary host memory, and with
one device per VM its domain is a per-VM domain.

**What kayfabe does instead.**
- The guest talks to an emulated device. A guest-chosen address is an input that kayfabe checks
  against that VM's own memory or store, then maps through its own tables. Host RM, the trusted
  driver on the host, builds the GPU page tables from the memory objects kayfabe registered. A guest
  value never reaches the device as a raw bus address.
- The VM's work runs on host channels that are unprivileged and live in a GPU virtual address space
  holding only that VM's mappings. The GPU's own memory management unit enforces the address space.
  Physical addressing needs a privileged channel: [measured, physical-operand oracle at 8c084ab7
  (RTX 4070, driver 595.91.07), 2026-10-08] on a USER-privilege channel the copy engine refuses a
  physical operand, source and destination, with a channel reset, while a virtual operand is
  delivered; the 3D, compute and video classes expose no physical-operand method in OGKM's class
  headers (a reading, not a measurement). Details: `traces/phys_operand_oracle_20261008/`.
- All VMs share one physical GPU, so they share one IOMMU domain for it. An IOMMU cannot tell VM A's
  DMA from VM B's here, so it could not provide VM-to-VM isolation even if we leaned on it. The
  isolation between VMs is the per-channel address space.

**What the host IOMMU still does.** The GPU's DMA on the host goes through the host IOMMU where the
host runs in translated mode (it does on the reference host, `DMA-FQ`). That limits the effect of a
wrong bus address, such as a bug in the driver or the hardware, to pages already mapped for that GPU.
It protects the host from any process using the GPU, kayfabe included, and it is configuration
dependent (many hosts run identity mode). We do not rely on it, and it is not part of the argument.

**Would a guest IOMMU (vIOMMU) help?** Not for security. The guest kernel works in CPU physical
addresses and converts to an IOVA only when it writes a PTE or sends a command to the device; kayfabe
converts it back at once. Translation happens at the two ends only and protects almost no code path;
the "device" it would guard against is kayfabe itself. It would be a compatibility feature for guests
that ask for DMA remapping. Windows 11 does not require one. Cost and design: `design/V3_VIOMMU.md`.

**Is the IOMMU a strong boundary against a malicious device even on bare metal?** Only against DMA to
pages the driver did not map. Drivers generally trust what the device writes into pages that are
mapped for it (rings, message queues, notifiers) and parse it, and GPU drivers are not designed
against a hostile device (inferred; Linux's own model treats internal PCI devices as trusted and has a
separate bounce-buffer path for untrusted external ones). That is why our own hostile party is the
guest and why kayfabe's input handling, bounds checks and authored host actions carry the weight.

**What the boundary is made of.** Hardware: the GPU's memory management unit and the channel
privilege. Software: kayfabe's checks and host RM. So a bug in kayfabe's address resolution matters
more than it would under VFIO, which is why that code gets hostile-input tests and review
(`docs/OWNER_RULINGS.md` §V and the review notes in `docs/STATUS_AND_HANDOFF.md`).

**Paravirtual projects in general.** Other paravirtual GPU projects (the owner reports virtio-nvgpu
says the same of itself; not re-checked here) and nvkvm-pv have no IOMMU-based isolation, and for them
it likewise does not decide the boundary: the guest does not own the device.

## Can one VM's GPU work wake another VM? Does a VM learn when other tenants use the GPU?

**Short answer:** yes, a little, on purpose — the same as for processes on one bare-metal GPU. Decided by
the owner on 2026-10-08 (`docs/OWNER_RULINGS.md` §X).

**What happens.** A "non-stall" interrupt (a completion signal a program asks for, for example after a
copy) is raised by the physical GPU and serviced by the host driver. The host driver's notification is
GPU-wide: it says "an engine of this kind finished something", not whose work it was, and the host driver
itself wakes every client registered on that engine. kayfabe passes the notification on to every VM whose
guest registered for that event (a non-stall event its own driver allocated, which kayfabe records), and
to no other VM. It never drops one: a lost completion interrupt leaves a program waiting for work that
already finished, which is a correctness bug.

**What a tenant can do to others (accepted).** By running GPU work that asks for interrupts, a tenant makes
the other VMs that registered for the same engine's events take extra interrupts. Each is spurious for them:
the waiting program checks its own completion value, finds it unchanged and waits again. This is a minor
denial of service, accepted for correctness, and it is the host driver's own behaviour between processes.
An optional pacing setting (`KF3_PT_NSI_MIN_INTERVAL_US`, off by default) can cap how often one guest
vector is raised; it delays an interrupt, it never drops one.

**What a guest learns (accepted).** A guest that registered for an engine's events can tell, from when its
interrupts arrive, that some tenant used that kind of engine and roughly how often. It does not learn which
tenant, what the work was, or any data. [measured, `traces/rawclient_ce_interrupt_20261008/`,
`bare_tenant_noise_*`] any unprivileged process on the host can already observe this: with another VM on
the GPU, an idle raw client saw these notifications in every observation window.

**What is not shared.** Memory, address spaces, channels and results stay per VM; see the IOMMU answer
above. Design: `design/the_three_channel_kinds.md` §1.2.
