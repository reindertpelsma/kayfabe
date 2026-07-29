//! # `kayfabe-qemu-raw` — the second audited raw crate, **empty on purpose**
//!
//! `l2_qemu_adapter.md` §2.2 / decision Q2. This crate is the future home of the entire
//! QEMU FFI surface: the `extern "C"` entry points the C shim's `MemoryRegionOps`
//! trampolines call, the typed wrappers over the ~20 QEMU functions of §3/§7/§8, and
//! `ForeignMapping` (adoption of a host pointer QEMU owns). All of that is stage **Q2**,
//! and Q2 needs a QEMU source tree, which this machine does not have.
//!
//! ## Why an empty crate exists at all
//!
//! Because the CI gate said to make the decision first, in as many words:
//!
//! > *"There is exactly ONE crate allowed to omit it (`kayfabe-linux-raw`), and adding a
//! > second is a design decision, not a manifest edit."*
//!
//! §2.2 turns that into a scheduling rule — the gate change *"must land before any
//! adapter code"* — for a reason worth restating: the containment gates are the only
//! mechanism behind the claim that the unsound surface is enumerable by `ls`. Changing
//! them from a **constant** to a **named two-element list** is the risky edit, and it is
//! cheapest to make when nothing depends on the answer.
//!
//! ## What the emptiness buys, and what it costs
//!
//! **Buys.** The ratchet's expected count for this crate is **0**, so the first
//! relaxation ever added here turns CI red until somebody itemises it in `ci.yml` — the
//! per-block review §2.2 asks for, enforced from before there is anything to review.
//! Two crate-agnostic gates cover the interim: the surface gate (a file using the
//! keyword must be named `*_unsafe.rs`) and the host-pointer gate.
//!
//! **Costs, stated rather than argued away.** §2.2 says the second crate's blocks should
//! be *"itemised in the ci.yml comment exactly as the first crate's 37 are"*. **There is
//! nothing to itemise yet**, so that half of the decision is deferred, and the honest
//! reading of today's tree is: the workspace now has a permanently allow-listed crate
//! with no contents. If stage Q2 is abandoned, **delete this crate** — an exemption
//! nobody is auditing is worse than no exemption.
//!
//! ## The seam it will implement
//!
//! `kayfabe_vmm_qemu::QemuHost` — defined by the **consumer**, in the safe crate, so the
//! FFI crate has no say in the shape of the port. Its rustdoc carries the normative
//! requirements this crate will have to satisfy, including the two that are load-bearing
//! for correctness rather than tidiness:
//!
//! - `read_region`/`write_region` MUST be a bounded memcpy against the region's own
//!   backing and MUST NOT reach a VMM general read/write-anywhere API;
//! - every topology-transaction method is called only from realize/unrealize, and
//!   `kayfabe_vmm_qemu` enforces that with a latch rather than a comment.
//!
//! ## ★★★ CORRECTED at stage Q2 — the pointer hand-over is NOT what this crate will do
//!
//! This section used to read: *"§5.1's shape is `we mmap one large reservation and hand
//! QEMU the pointer`, via `memory_region_init_ram_ptr`, so Q2 owes `kayfabe-linux-raw` a
//! base-address accessor"*. **That is void.** `host_execution_plane.md` §1 supersedes §5.4:
//! the hypervisor **reserves** the guest-physical window with `memory_region_init_io` — a
//! pure-MMIO BAR it does not back — and *we* install the memslots that shadow it, with the
//! kernel's own ioctl.
//!
//! Three consequences for this crate, all of them subtractions:
//!
//! 1. **No pointer crosses this seam.** The unwrap happens inside
//!    `kayfabe_linux_raw`'s own `kvm_unsafe.rs`, on a safe `&GuestWindow`, so the
//!    host-pointer gate holds as designed and `GuestWindow` still has no base accessor.
//! 2. **No region constructor is called at all.** `memory_region_init_ram_ptr` and the
//!    ROM-device overlay constructor are both gone from the design; what the C shim above
//!    this crate does with the memory API is `memory_region_init_io` + `pci_register_bar`,
//!    once, at realize (`C: src/qemu/virtio_nvgpu_pci.c:108-114`).
//! 3. **The memslot calls are not this crate's either** — they are the kernel's, and they
//!    cross `kayfabe_vmm_qemu::slots::SlotPlane`, which has a real implementation today.
//!
//! What is left for this crate is genuinely small: the `extern "C"` trampolines for the
//! trapped regions' read/write ops, the two field reads
//! (`kayfabe_vmm_qemu::host::QemuHost::bar_base` and the reservation-shape query), the
//! lifecycle calls, and the interrupt write. **It is still empty**, because all of that
//! needs a hypervisor source tree to build against and this machine has none.

#![doc(test(attr(deny(warnings))))]
