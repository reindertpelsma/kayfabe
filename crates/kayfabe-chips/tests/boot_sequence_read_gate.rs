//! ★★★★★ **A `BootSequence` that serves a read must ADMIT to serving it (w567).**
//!
//! `RegPlane::read_inner` refuses an offset before the sequence is ever asked for a value:
//!
//! ```text
//! if self.model.decode_reg(bar, off).is_none()
//!     && !self.model.boot_sequence().may_read(bar, off)
//! { return ReadOutcome::Unclaimed; }
//! ```
//!
//! ⊘⊘ So `on_read` without `may_read` is **dead code that looks alive**. `Gh100FspBoot`
//! shipped in exactly that state: six FSP registers served by `on_read`, and a `may_read` left
//! at its `false` default. The failure has no symptom — an unclaimed BAR0 read answers `0`, so
//! the guest's FSP queue reads "drained" by accident and its EMEM cursor reads zero forever.
//!
//! ⚠ The existing guard, `kayfabe-device/tests/state_free_read_filter.rs`, builds a plane over
//! **GA106 only** — whose sequence overrides neither method. It was vacuous for the one
//! implementation that could trip it. This test quantifies over the SEQUENCES instead, so a
//! fourth generation is covered the day it is written rather than the day it boots.

use kayfabe_arch::gsp::{ArchBootState, BootContext, BootSequence, GspObservation};
use kayfabe_arch::GspModel;

/// Every `BootSequence` this workspace ships, with a name for the failure message.
fn sequences() -> Vec<(&'static str, &'static dyn GspModel)> {
    // ⊘ Leaked deliberately: these are zero-sized and the test needs `&'static dyn`. A leak
    // in a test binary that exits is not a leak anyone pays for.
    let gh: &'static kayfabe_chips::gh100::Gh100GspModel =
        Box::leak(Box::new(kayfabe_chips::gh100::Gh100GspModel::default()));
    let ad: &'static kayfabe_chips::ad10x::Ad10xGspModel =
        Box::leak(Box::new(kayfabe_chips::ad10x::Ad10xGspModel::default()));
    vec![("Gh100FspBoot", gh as &dyn GspModel), ("FalconSecureBooterBoot (via Ad10x)", ad)]
}

/// ★★★★★ **Whatever `on_read` answers, `may_read` must admit — over the whole aperture.**
///
/// ⊘ Swept rather than spot-checked: the defect is an OMISSION, and an omission has no token
/// to grep for. The sweep is the only thing that finds an offset somebody served and forgot to
/// declare.
#[test]
fn every_offset_a_sequence_serves_is_one_it_admits_to_serving() {
    const APERTURE: u64 = 0x0100_0000;
    let obs = GspObservation::default();
    let ctx = BootContext {
        obs,
        boot_args_seen: (false, false),
    };
    for (name, model) in sequences() {
        let seq = model.boot_sequence();
        let state = ArchBootState::default();
        let mut served = 0u64;
        for off in (0..APERTURE).step_by(4) {
            let answered = seq.on_read(model, 0, off, &ctx, &state).is_some();
            if answered {
                served += 1;
                assert!(
                    seq.may_read(0, off),
                    "{name}: on_read SERVES {off:#x} and may_read REFUSES it. The plane gates \
                     on may_read, so this register is dead code that looks alive — the guest \
                     reads a defaulted zero with no fault and no counter."
                );
            }
        }
        // ⊘ And the converse, as non-vacuity in the other direction: a sequence that admitted
        // to offsets it does not serve would make the plane take a lock to be told nothing.
        eprintln!("BOOT-SEQ-READ-GATE {name}: serves {served} offset(s)");
    }
}

/// ⊘ **Non-vacuity.** At least one sequence must actually serve something, or the sweep above
/// passes by finding nothing — the shape this repository calls a census zero with no
/// known-positive.
#[test]
fn at_least_one_sequence_serves_a_read_at_all() {
    let obs = GspObservation::default();
    let ctx = BootContext {
        obs,
        boot_args_seen: (false, false),
    };
    let total: u64 = sequences()
        .into_iter()
        .map(|(_, model)| {
            let seq = model.boot_sequence();
            let state = ArchBootState::default();
            (0..0x0100_0000u64)
                .step_by(4)
                .filter(|&off| seq.on_read(model, 0, off, &ctx, &state).is_some())
                .count() as u64
        })
        .sum();
    assert!(
        total > 0,
        "no sequence serves any read, so the gate test above proved nothing"
    );
}
