//! The GSP register model the replay drives — `kf-chip`'s real GA10x family model, NOT a mock arch.
//!
//! ⊘ The old tree's replay wrapped `MockArch` + the GSP model in a local `Ga10xArch`; only `gsp()`
//! was ever used. v3 hands the model to `GspFsm::mmio_*_with` directly.

use kf_arch::gsp::GspModel;

/// The framebuffer size the C artifact booted with when the captures were recorded
/// (`NVKVM_FB_SIZE_MB`, `C: src/qemu/mode2_regs_ga10x.h:62`). ★ A fact about the RECORDING, not a
/// die constant: WPR2's register values in the capture were derived from it, so the replay must
/// build its model at the same size to be comparable.
pub const CAPTURE_FB_SIZE_MB: u64 = 12288;

/// The GA10x GSP model at the capture's size.
///
/// # Panics
/// Never: GA10x's row is built (`kf_chip::Family::gsp_model`).
#[must_use]
pub fn gsp_model() -> Box<dyn GspModel> {
    kf_chip::Family::Ga10x.gsp_model(CAPTURE_FB_SIZE_MB).expect("the GA10x row is built")
}
