//! # `kayfabe-chips` — the per-generation HOST class profiles the raw client forwards with
//!
//! ★ **Slimmed 2026-10-15 (`archive/README.md`).** This crate used to be the pre-v3 arch-impl
//! crate: one `impl Arch` + `impl GspModel` per GPU generation (`ad10x`, `ga10x`, `gb20x`,
//! `gh100`). None of that is reachable from the raw client (`kayfabe-rm-ladder`) or the isolate,
//! so those modules and their tests moved to `archive/prev3-modules/kayfabe-chips/`, and the
//! generated class tables the host profiles intersect with, [`classgen`], were copied here from
//! the archived `kayfabe-doorbell` (their only live consumer).
//!
//! What remains is the HOST-forwarding class axis ([`host_classes`], task `#156`): the three
//! roles (channel, usermode, copy-engine object) and the compute class RM binds per generation,
//! the host-derived [`DerivedHostClasses`] that replaced the GA10x pin on every raw-client
//! connection, and the pinned fallback ([`pinned_host_classes`]). v3's counterpart is
//! `kf_chip::host_classes`.

pub mod classgen;
pub mod host_classes;

pub use host_classes::{
    Ad10xHostClasses, DerivedHostClasses, Ga10xHostClasses, Gb20xHostClasses, Gh100HostClasses,
    HostLacksKind, host_classes_for_arch, pinned_host_classes,
};
