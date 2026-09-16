//! ★★★★★ **w755 — THE INVARIANT THAT IS RIGHT FOR THE PROBE, since the production ones are
//! not.**
//!
//! `crate::placement_probe` builds a **third** `NVOS46` and deliberately returns `Ok` for a
//! mapping RM relocated — the exact behaviour `every_nvos46_site_asserts_its_own_placement`
//! forbids in `rm.rs`, and rightly. It is legitimate here only because the probe's job is to
//! MEASURE what RM does, and only if it actually compares the measurement against the
//! prediction and reports a disagreement.
//!
//! ⊘ Without this file, moving the probe out of `rm.rs` would have bought the production
//! gates' strength back by leaving the probe gated by nothing at all — trading one blind spot
//! for another, which is not what "do not relax a constraint" asks for.

fn probe_src() -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/placement_probe.rs"),
    )
    .expect("the probe module is where this gate says it is")
}

/// ★★★ The probe must **predict**, **measure** and **compare** — all three.
#[test]
fn the_probe_compares_prediction_against_reality() {
    let s = probe_src();
    // ★ NON-VACUITY first: a scan that finds nothing must fail rather than pass.
    assert!(
        s.len() > 2000 && s.contains("pub fn placement_probe"),
        "the probe module is missing or unrecognisable ({} bytes) — this gate would \
         otherwise pass by scanning nothing",
        s.len()
    );
    for (needle, why) in [
        (
            "rm_would_place(",
            "no longer consults the model, so it measures RM without grading anything",
        ),
        (
            "pp_map_fixed(",
            "no longer issues a real map, so it grades the model against nothing",
        ),
        (
            "PP_DISAGREE",
            "no longer reports disagreements, and a differential that cannot say the two \
             sides differ is not a differential",
        ),
        (
            "PP_AGREE",
            "no longer reports agreements — without them a zero disagreement count cannot be \
             told from a probe that compared nothing",
        ),
    ] {
        assert!(
            s.contains(needle),
            "★★★ THE PLACEMENT PROBE STOPPED BEING A DIFFERENTIAL — it {why}. It builds an \
             `NVOS46` that returns `Ok` for a relocated mapping, which is only acceptable \
             while it is measuring."
        );
    }
}

/// ★★★★★ **THE PROBE ENCODES ITS OWN `NVOS46` AND MUST KEEP DOING SO.**
///
/// ⊘ `a_probe_that_shares_the_allocator_is_not_an_observer` — wrong three times in this
/// campaign, the third INVERTED. If `pp_map_fixed` were ever replaced by a call into
/// `RmConnection::raw_map_dma_slice`, the probe would be asking the production encoder to
/// check itself and a defect in its flag selection would produce a matching pair of wrong
/// answers instead of a disagreement.
#[test]
fn the_probe_does_not_share_the_production_encoder() {
    let s = probe_src();
    assert!(
        s.contains("Nvos46Parameters {"),
        "the probe no longer builds its own NVOS46"
    );
    for shared in ["raw_map_dma_slice(", "raw_map_dma(", "map_store_slice("] {
        assert!(
            !s.contains(shared),
            "★★★ the probe now calls `{shared}` — it is no longer an independent observer, \
             and a defect in the production encoder would hide itself as agreement"
        );
    }
}

/// ★★★ **A refused `GET_SURFACE_PHYS_ATTR` must be an ANSWER, not a probe failure.**
///
/// ⊘ `ctrl0041.h` calls the control MODS-only. *"This host will not say"* is one of the three
/// questions the probe exists to answer, so a host that refuses must still produce a row —
/// `a_refusal_counter_read_as_absent_demand` is what happens when it does not.
#[test]
fn a_refused_phys_attr_is_reported_rather_than_fatal() {
    let s = probe_src();
    assert!(
        s.contains("PP_PHYS_ATTR=REFUSED"),
        "the probe no longer distinguishes 'this host refuses to report the address' from a \
         failure — the MODS-only question then has no answer either way"
    );
    assert!(
        s.contains("PP_RESULT=INERT:no-phys-attr"),
        "the probe no longer says that a refused address makes the differential INERT; a \
         `PP_DISAGREE=0` from a probe that could not predict anything reads as success"
    );
}
