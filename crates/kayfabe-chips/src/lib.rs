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
//!
//! ## [`gb20x`] — consumer Blackwell (GB202). The generation that BREAKS the family rule
//!
//! Blackwell is the member that falsifies *"a later generation is the previous one's
//! offsets"* in two places, both read out of `ogkm-580` rather than assumed:
//!
//! - **the work-submit token changes.** `kfifoGenerateWorkSubmitTokenHal` gains a
//!   GB202-specific arm that sets a third field, `NV_VIRTUAL_FUNCTION_DOORBELL_
//!   RUNLIST_DOORBELL = _ENABLE` at bit 30. Every GB202 token has that bit set, and
//!   [`ga10x::decode_work_submit_token`]'s refusal mask rejects **all** of them. This is
//!   the seam `execution_plane_increments.md` §2.1 names as unable to fail loudly, and it
//!   is the first generation where reusing it is actively wrong.
//! - **`NV_THERM_I2CS_SCRATCH` moves** from `0x000200bc` to `0x00ad00bc`, and it is the
//!   register RM polls for FSP-boot-complete *before it may send any FSP packet at all*.
//!
//! ★ Its boot regime is [`gh100`]'s (GB202 binds `kgspBootstrap_GH100` and the whole
//! `kfsp*_GH100` transport), so the seam task #121 built is what made this generation a
//! new file rather than an edit to an existing one. Adding it changed **zero** lines of
//! `kayfabe_gsp::seq`, `kayfabe_device::ga10x`, [`ad10x`] or [`gh100`].
//!
//! ⊘ **No Blackwell board has run any of it.** The one part of the generation with a
//! hardware measurement behind it is [`host_classes::Gb20xHostClasses`], and the
//! measurement is the Mode-1 sibling's, not this port's.
//! [`Arch`]: kayfabe_arch::Arch
//! [`GspModel`]: kayfabe_arch::gsp::GspModel
//! [`GspReg`]: kayfabe_arch::gsp::GspReg
//! [`Gh100GspModel`]: gh100::Gh100GspModel

pub mod ad10x;
pub mod ga10x;
// ★ #Blackwell — consumer Blackwell (GB202). See the module docs: the first generation
// whose work-submit token this crate could NOT inherit.
pub mod gb20x;
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
pub use gb20x::{Gb20xArch, Gb20xFspBoot, Gb20xGspModel};
pub use gh100::{Gh100Arch, Gh100GspModel};
pub use host_classes::{
    Ad10xHostClasses, Ga10xHostClasses, Gb20xHostClasses, Gh100HostClasses, pinned_host_classes, host_classes_for_arch,
};
