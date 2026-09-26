//! ★★★★★ Whatever a boot sequence's `on_read` serves, its `may_read` must admit — swept over the
//! whole aperture, for EVERY family that has a model (quantified over `Family::ALL`, so a new
//! family is covered the day its model exists). Ported from the old kayfabe-chips test, now over
//! the shared falcon/FSP models.
use kf_arch::gsp::{ArchBootState, BootContext, GspObservation};
use kf_chip::Family;

/// A discrete die's `MC_GET_ARCH_INFO` implementation (GA106/AD106/TU106 = 6): the `_GA102` group on
/// Ampere, where GA100 (0) is a different one (`kf_chip::Family::gsp_model`).
const DISCRETE_IMPL: u32 = 0x6;

const APERTURE: u64 = 0x0100_0000;
const FB_MB: u64 = 8192;

fn served(f: Family) -> Option<(u64, Vec<u64>)> {
    let model = f.gsp_model(DISCRETE_IMPL, FB_MB).ok()?;
    let ctx = BootContext { obs: GspObservation::default(), boot_args_seen: (false, false) };
    let seq = model.boot_sequence();
    let state = ArchBootState::default();
    let mut n = 0;
    let mut refused = Vec::new();
    for off in (0..APERTURE).step_by(4) {
        if seq.on_read(model.as_ref(), 0, off, &ctx, &state).is_some() {
            n += 1;
            if !seq.may_read(0, off) {
                refused.push(off);
            }
        }
    }
    Some((n, refused))
}

#[test]
fn every_offset_a_sequence_serves_is_one_it_admits_to_serving() {
    for f in Family::ALL {
        let Some((n, refused)) = served(f) else { continue };
        assert!(refused.is_empty(), "{f:?}: on_read SERVES {refused:x?} that may_read REFUSES — dead registers that look alive");
        eprintln!("BOOT-SEQ-READ-GATE {f:?}: serves {n} offset(s)");
    }
}

/// ⊘ Non-vacuity, as KNOWN POSITIVES: Hopper's FSP regime serves the six FSP registers; Blackwell's
/// serves those plus `NV_THERM_I2CS_SCRATCH` — the row difference, measured through the model.
#[test]
fn the_fsp_rows_serve_what_they_declare() {
    assert_eq!(served(Family::Hopper).map(|s| s.0), Some(6), "Hopper: 4 queue regs + EMEMC + EMEMD");
    assert_eq!(served(Family::Blackwell).map(|s| s.0), Some(7), "Blackwell: Hopper's six + the thermal gate");
}

/// ★ 2026-09-26: every family has a model for a discrete die (Turing's is the `_TU102` RISC-V
/// layout); GA100 — Ampere on the `_TU102` HALs with no FWSEC-FRTS — is refused BY NAME.
#[test]
fn every_family_has_a_gsp_model_and_ga100_is_refused_by_name() {
    for f in Family::ALL {
        assert!(f.gsp_model(DISCRETE_IMPL, FB_MB).is_ok(), "{f:?} has no GSP model");
    }
    let ga100 = Family::Ampere.gsp_model(kf_chip::arch::IMPL_GA100, FB_MB).err().expect("GA100 is refused");
    assert!(ga100.what.contains("FRTS"), "{}", ga100.what);
}
