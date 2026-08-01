//! # `kayfabe-chips` — the arch-impl crate, and the measurement it was created to take
//!
//! `mode2_gsp_port_plan.md` §3.5's table names the home of an [`Arch`] implementation as
//! *"`kayfabe-arch` + an arch-impl crate"*. Before this crate the workspace had no such
//! member: the one non-mock [`Arch`]/[`GspModel`] pair in the tree lives inside
//! `kayfabe-crec`, the C-trace differential harness, because that is where it was first
//! needed. So *"adding a generation is `impl Arch for <Gen>` in an adapter crate"* had
//! never been done — the adapter crate did not exist.
//!
//! This crate exists to take that measurement, and it holds two generations that answer
//! it in **opposite directions**. They are reported separately on purpose: averaging them
//! would hide the interesting one.
//!
//! ## [`ad10x`] — Ada. The claim SURVIVES, and this is the easy member of the universe
//!
//! Every register in [`GspReg`]'s vocabulary is at the same offset on AD10x as on GA10x,
//! and every encoding is the same value, because Ada's GSP boot HAL dispatches to the
//! `_TU102`/`_GA102` implementations for the whole sequence. The generation costs a
//! `struct`, a `VBIOS_PROFILES` row and nothing else.
//!
//! ★ **That result is weak on its own and is labelled as such.** An experiment that
//! selects the easiest member of its universe produces a green with no red available to
//! it — the same defect as a gate quantified over a shortened list.
//!
//! ## [`gh100`] — Hopper. The hard member, and it is where the seam was actually built
//!
//! Hopper's registers are *also* mostly at the same offsets (see [`gh100`]'s constants,
//! all read from `ogkm-580`'s `hopper/gh100/` headers). It was never the offsets. It was
//! that **the boot sequence had no seam**: `GspFsm::mmio_write` spelled the
//! falcon/secure-booter ordering out in `match` arms over the [`GspReg`] enum, inside
//! `kayfabe-gsp` — a logic crate — so a generation whose boot is driven by an FSP command
//! queue could be expressed only by editing the ordering GA10x boots on.
//!
//! ★★★ **Task #121 (2026-07-31) reframed and then fixed that.** Arch-specific boot code
//! is legitimate and has to exist somewhere; the defect was that it was *unhooked*. The
//! ordering now lives behind `kayfabe_arch::BootSequence`, and
//! [`gh100::Gh100FspBoot`] is a second one, added **alongside**:
//!
//! - it lives in this crate, at this crate's offsets — `NV_PFSP_EMEMC`/`EMEMD` and the
//!   FSP command queue, none of which any [`GspReg`] variant names;
//! - it declares **three** boot stages where the falcon regime declares five, because one
//!   FSP command does what four falcon writes do;
//! - landing it changed **zero lines** of `kayfabe_gsp::seq::FalconSecureBooterBoot`, of
//!   `kayfabe_device::ga10x` or of [`ad10x`].
//!
//! [`gh100::ARCH_LOCAL_BOOT_EVENTS`] carries the four boot events with no `GspReg` to
//! hang on, three now served through the seam and **one still unmodelled and saying so**.
//!
//! [`Arch`]: kayfabe_arch::Arch
//! [`GspModel`]: kayfabe_arch::gsp::GspModel
//! [`GspReg`]: kayfabe_arch::gsp::GspReg
//! [`Gh100GspModel`]: gh100::Gh100GspModel

pub mod ad10x;
pub mod ga10x;
pub mod gh100;
// ★ #156 — the HOST-forwarding class axis. Not per-chip files, because the axis is one
// table of three roles and splitting it across three modules would hide that AD10x's
// answer is IDENTICAL to GA10x's, which is the interesting half of the measurement.
pub mod host_classes;

pub use ad10x::{Ad10xArch, Ad10xGspModel};
// ★★ `ga10x` is the ONE module here that is linked into the shipped QEMU archive, and it
// is deliberately NOT `MockArch`-composed like its two neighbours — see its module docs.
pub use ga10x::{
    Ga10xArch, Ga10xGmmu, Ga10xPushbuffer, Ga10xUserd, UnbuiltGmmu, UnbuiltPushbuffer, UnbuiltUserd,
};
pub use gh100::{Gh100Arch, Gh100GspModel};
pub use host_classes::{Ad10xHostClasses, Ga10xHostClasses, Gh100HostClasses, pinned_host_classes};
