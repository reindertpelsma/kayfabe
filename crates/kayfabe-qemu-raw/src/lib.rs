//! # `kayfabe-qemu-raw` — the second audited raw crate: the hypervisor FFI surface
//!
//! `l2_qemu_adapter.md` §2.2 / decision Q2. This crate is the **entire** foreign-function
//! surface of the hypervisor adapter: the `extern "C"` entry points a C device shim calls,
//! and [`QemuHost`](kayfabe_vmm_qemu::host::QemuHost) implemented over a table of C function
//! pointers the shim supplies.
//!
//! It stopped being empty on 2026-07-30, at stage **Q2**.
//!
//! ## ★★★ The seam is a TABLE OF PRIMITIVES, not a set of linked symbols — a departure
//! ## from `l2_qemu_adapter.md` §2.3, taken deliberately
//!
//! §2.3 draws the picture as *"the C never calls Rust logic and Rust never calls C logic;
//! both call primitives"* and leaves the mechanism implicit. The obvious mechanism —
//! `extern "C"` **declarations** of the hypervisor's own functions, resolved by the linker
//! when this archive is linked into it — was rejected for three reasons, in order of weight:
//!
//! 1. ★★ **It would make this crate untestable outside a hypervisor.** An archive with
//!    undefined references to `memory_region_init_io` cannot be linked into a test harness,
//!    so `cargo test -p kayfabe-qemu-raw` would need a hypervisor build to run at all. The
//!    whole point of §10's stage table is that Q0 and Q1 need no machine; a raw crate that
//!    silently requires one takes that back.
//! 2. ★★ **It would hard-code the hypervisor's symbol names into Rust**, which is exactly
//!    the coupling `four_axes_of_variation.md` names as a bolt-on risk. A table of function
//!    pointers has no vendor in it: a second hypervisor supplies its own table and this crate
//!    does not change. The names live in the C shim, where the vendor already is.
//! 3. **It would put the hypervisor's version skew inside Rust.** The two releases this was
//!    built against disagree about header paths, one class-initialiser signature, and whether
//!    a property array carries a terminator. All three are C spellings of the same thing, and
//!    they are absorbed by a dozen lines of `#if` in the shim's own compatibility header —
//!    not by a build script probing a source tree.
//!
//! The price, stated rather than argued away: the table is **hand-mirrored** in
//! `qemu/hw/misc/nvkvm/kayfabe_shim.h`, so a field added on one side and not the other is a
//! real hazard. It is answered by two handshakes checked at realize — an ABI number and
//! `sizeof` the table — so a mismatch is a named refusal at startup rather than a jump
//! through a garbage address later. Both arms are exercised.
//!
//! ## Layout
//!
//! ```text
//!   hw/misc/nvkvm/  (C, QOM only)  ──▶  shim_unsafe.rs  ──▶  shim.rs  ──▶  kayfabe-vmm-qemu
//!        the vendor's names           the raw surface       decisions        all the logic
//! ```
//!
//! [`shim`] is safe, holds no addresses, and is where every decision lives — driven end to
//! end by `kayfabe_vmm_qemu::mock_host::MockQemuHost` with no hypervisor present.
//! `shim_unsafe.rs` is one file, and its `ls`-enumerability is the audit.
//!
//! ## ★★ What this stage does NOT build, named rather than sketched
//!
//! - **No interrupt path.** The vector-raising method refuses by name. Message-signalled
//!   interrupts are §7 and need plumbing this stage has none of.
//! - **No trap dispatch.** The device's register region traps and returns a constant; nothing
//!   routes an access into the core. That is §6 / stage Q4.
//! - **No lockless-IO measurement.** The shim marks its regions where the hypervisor offers
//!   the facility (10.2 and later) and the realize-time coverage self-check asserts it, but
//!   the A/B acceptance run of §11's last row was not performed.
//!
//! ## ★ The version floor is now TWO different numbers, and the difference is real
//!
//! `l2_qemu_adapter.md` §3.5 asks for a compile-time floor and a realize-time floor, both
//! 10.2, on the argument that *"the build-time check is a claim about the headers, this is a
//! claim about the binary"*. Building the shim showed those two claims cannot differ here —
//! §2.1 of the same document proves there is no out-of-tree device mechanism, so the shim is
//! compiled *inside* the binary it runs in and the two have one source. What survives is a
//! better distinction: the two floors are about different **subjects**.
//!
//! - the **compile-time** floor is about SYMBOLS — every hypervisor function the shim names
//!   was verified present at **9.2**, so that is where the `#error` sits;
//! - the **realize-time** floor is `kayfabe_vmm_qemu::VERSION_FLOOR`, still **10.2**, and is
//!   about SEMANTICS: the global-lock opt-out, which did not exist before 10.2.
//!
//! Keeping them equal would have made the shim unbuildable on the only hypervisor tree the C
//! oracle itself ran on, and would have hidden the more useful fact: on a 9.2 build the
//! device compiles, registers, realizes its C half, and is then refused **by name** at the
//! Rust half. That refusal was watched happening, which is worth more than a floor nobody
//! could reach.

#![doc(test(attr(deny(warnings))))]

pub mod shim;
pub mod shim_unsafe;
