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
//! ## [`gh100`] — Hopper. The claim FAILS, and the failure is in a logic crate
//!
//! Hopper's registers are *also* mostly at the same offsets (see [`gh100`]'s constants,
//! all read from `ogkm-580`'s `hopper/gh100/` headers). It is not the offsets. It is that
//! **the boot sequence the FSM implements does not exist on this generation**, and the
//! FSM's `mmio_write` dispatcher spells that sequence out in `match` arms over the
//! [`GspReg`] enum — in `kayfabe-gsp`, a logic crate, whose whole stated contract is that
//! it contains no generation-specific behaviour.
//!
//! [`Gh100GspModel`] is therefore written to be **structurally honest rather than
//! green**: it declines (returns `None` from `decode_reg`) for the two SEC2 Booter
//! registers, because on this generation those registers carry no boot meaning at all,
//! and it names in [`gh100::MISSING_TRANSITIONS`] the boot events that have no `GspReg`
//! to hang on. The crate's test asserts that the FSM cannot be driven past
//! `BootPhase::FwsecRan` by any [`GspReg`] write on this model — i.e. it *pins the
//! refutation*, so a later change that fixes the seam turns a test red rather than
//! passing silently.
//!
//! [`Arch`]: kayfabe_arch::Arch
//! [`GspModel`]: kayfabe_arch::gsp::GspModel
//! [`GspReg`]: kayfabe_arch::gsp::GspReg
//! [`Gh100GspModel`]: gh100::Gh100GspModel

pub mod ad10x;
pub mod gh100;

pub use ad10x::{Ad10xArch, Ad10xGspModel};
pub use gh100::{Gh100Arch, Gh100GspModel};
