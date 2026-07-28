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
//! ## ★ One thing Q2 will need that does not exist yet
//!
//! §5.1's shape is *"**we** mmap one large reservation and hand QEMU the pointer"*, via
//! `memory_region_init_ram_ptr`. `kayfabe_linux_raw::GuestWindow` has **no accessor for
//! its base address** — by design: the host-pointer gate confines every such type to
//! `*_unsafe.rs`, and `GuestWindow`'s base is a private field. So Q2 owes
//! `kayfabe-linux-raw` one new relaxation (a base-address accessor inside
//! `window_unsafe.rs`, moving that crate's ratchet off 37) *or* a `GuestWindow`
//! constructor that hands the mapping over. Neither is a Q0/Q1 item; it is recorded here
//! because §5.1 reads as though the pointer were already reachable.

#![doc(test(attr(deny(warnings))))]
