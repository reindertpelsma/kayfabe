//! # `kayfabe-chips` — what is left of the pre-v3 arch-impl crate: the host class profiles
//!
//! **STATUS: LIVE, 2026-10-31.** ⊘ Correction, folded above the text it corrects: this crate used
//! to hold one `impl Arch` + `impl GspModel` per GPU generation (`ga10x`, `ad10x`, `gb20x`,
//! `gh100`, 3 995 lines) next to [`host_classes`]. Nothing live used the chip modules: the
//! frozen grader (`kayfabe-rm-ladder`) and its closure take only [`host_classes`]
//! (`DerivedHostClasses`, `pinned_host_classes`, `CLASSLIST_V2_SIZE`, `decode_classlist`). The
//! four modules and their tests moved to `archive/crates/kayfabe-chips-modules/` (not a
//! workspace member, not built), which is what let `kayfabe-mocks`, `kayfabe-gsp`,
//! `kayfabe-trace`, `kayfabe-completion` and `kayfabe-mmu` follow into `archive/crates/`.
//! The module-level documentation of the old crate, including the Ada/Hopper/Blackwell
//! findings, is kept verbatim in `archive/crates/kayfabe-chips-modules/src/lib.rs`.
//!
//! What remains is [`host_classes`]: the HOST-forwarding class axis (`#156`), one table of
//! three roles (channel, usermode, copy object) per generation, plus the class-list decoder
//! the raw client uses to derive the profile from the host's own class list. v3's
//! counterpart is `kf_arch::HostClasses` / `kf_rm`'s derived class sets.

pub mod host_classes;

pub use host_classes::{
    Ad10xHostClasses, DerivedHostClasses, Ga10xHostClasses, Gb20xHostClasses, Gh100HostClasses,
    HostLacksKind, host_classes_for_arch, pinned_host_classes,
};
