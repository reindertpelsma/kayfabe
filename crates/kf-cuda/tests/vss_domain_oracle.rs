//! GPU-free oracle for the segmented (VSS) table domains `kf_color.cu` implements.
//!
//! No source states the abscissae of a VSS segment. The kernel infers them: 64 linear
//! ILUT segments over [0,1] and 33 logarithmic OLUT segments, segment 0 = [0,2^-32],
//! segment k = [2^(k-33), 2^(k-32)]. This test checks that inference against an
//! independent reference — SMPTE ST 2084 (PQ), evaluated analytically — using OGKM's own
//! PQ tables (`nvkms-evo3.c`, derived by `scripts/bench/display/derive_tmo_fixture.py`).
//! The CPU math here is a test expectation only; it never runs on the display path.

#![allow(dead_code)] // the shared fixture also carries the CSC matrices
include!("../examples/tmo_fixture_data.inc");

const M1: f64 = 2610.0 / 16384.0;
const M2: f64 = 2523.0 / 4096.0 * 128.0;
const C1: f64 = 3424.0 / 4096.0;
const C2: f64 = 2413.0 / 4096.0 * 32.0;
const C3: f64 = 2392.0 / 4096.0 * 32.0;

/// ST 2084 inverse EOTF: linear luminance (1.0 = 10000 nits) to PQ code.
fn pq_oetf(y: f64) -> f64 {
    let p = y.clamp(0.0, 1.0).powf(M1);
    ((C1 + C2 * p) / (1.0 + C3 * p)).powf(M2)
}

/// ST 2084 EOTF: PQ code to linear luminance (1.0 = 10000 nits).
fn pq_eotf(e: f64) -> f64 {
    let p = e.clamp(0.0, 1.0).powf(1.0 / M2);
    ((p - C1).max(0.0) / (C2 - C3 * p)).powf(1.0 / M1)
}

fn half(bits: u32) -> f64 {
    let (e, m) = ((bits >> 10) & 31, f64::from(bits & 1023));
    assert!(
        bits < 0x7c00,
        "OGKM table entries are finite and nonnegative"
    );
    if e == 0 {
        m * 2f64.powi(-24)
    } else {
        (1.0 + m / 1024.0) * 2f64.powi(e as i32 - 15)
    }
}

/// Sample abscissae: every interval start, then the endpoint.
fn abscissae(log2: &[u32], bounds: impl Fn(usize) -> (f64, f64)) -> Vec<f64> {
    let mut x = Vec::new();
    for (k, &l) in log2.iter().enumerate() {
        let ((lo, hi), n) = (bounds(k), 1 << l);
        x.extend((0..n).map(|j| lo + (hi - lo) * f64::from(j) / f64::from(n)));
    }
    x.push(bounds(log2.len() - 1).1);
    x
}

fn logarithmic(top: i32) -> impl Fn(usize) -> (f64, f64) {
    move |k| {
        let k = k as i32;
        let lo = if k == 0 { 0.0 } else { 2f64.powi(k - 33 + top) };
        (lo, 2f64.powi(k - 32 + top))
    }
}

#[test]
fn the_logarithmic_output_domain_reproduces_ogkms_pq_oetf_table() {
    let x = abscissae(&OETF_P_Q512_SEG_SIZES_LOG2, logarithmic(0));
    assert_eq!(x.len(), OETF_P_Q512_ENTRIES.len(), "intervals + 1 samples");
    let worst = |scale: f64, x: &[f64]| {
        x.iter()
            .zip(OETF_P_Q512_ENTRIES)
            .map(|(&x, e)| (pq_oetf(x / scale) * 65535.0 - f64::from(e)).abs())
            .fold(0.0, f64::max)
    };
    // [0,1] with 1.0 = 10000 nits: within table quantization (OGKM rounds to 4 codes).
    assert!(worst(1.0, &x) < 6.0, "worst {}", worst(1.0, &x));
    // The rejected alternative: a [0,128] domain holding NVKMS's [0,125] units unnormalized.
    let alt = abscissae(&OETF_P_Q512_SEG_SIZES_LOG2, logarithmic(7));
    assert!(
        worst(125.0, &alt) > 100.0,
        "the [0,128]/125 domain must not fit"
    );
}

#[test]
fn the_linear_input_domain_reproduces_ogkms_pq_eotf_table_in_125_units() {
    let x = abscissae(&EOTF_P_Q512_SEG_SIZES_LOG2, |k| {
        (k as f64 / 64.0, (k + 1) as f64 / 64.0)
    });
    assert_eq!(x.len(), EOTF_P_Q512_ENTRIES.len(), "intervals + 1 samples");
    let worst = |scale: f64| {
        x.iter()
            .zip(EOTF_P_Q512_ENTRIES)
            .map(|(&x, e)| {
                let want = pq_eotf(x) * scale;
                (want - half(e)).abs() / want.max(1e-3)
            })
            .fold(0.0, f64::max)
    };
    // NVKMS's comment: "Values are in range [0.0, 125.0], will be scaled back by OLUT",
    // and nvidia-drm divides by 125 (fp_norm) — the OLUT FP_NORM_SCALE then restores [0,1].
    assert!(worst(125.0) < 0.002, "worst {}", worst(125.0));
    assert!(worst(128.0) > 0.02, "a 128-unit scale must not fit");
}

/// The authored extents of OGKM's two VSS programs fit the decoder's bounds:
/// intervals + 1 samples, one spare copied endpoint, at most 1025 entries.
#[test]
fn ogkms_vss_extents_fit_the_fixed_bounds() {
    for (log2, entries, segments) in [
        (
            &EOTF_P_Q512_SEG_SIZES_LOG2[..],
            EOTF_P_Q512_ENTRIES.len(),
            64,
        ),
        (
            &OETF_P_Q512_SEG_SIZES_LOG2[..],
            OETF_P_Q512_ENTRIES.len(),
            33,
        ),
    ] {
        assert_eq!(log2.len(), segments);
        assert!(log2.iter().all(|&l| l <= 7), "3-bit header fields");
        let intervals: u32 = log2.iter().map(|&l| 1 << l).sum();
        let authored = entries as u32 + 1; // NVKMS copies the last entry once more
        assert_eq!(intervals + 1, entries as u32);
        assert!((segments as u32 + 1..=1025).contains(&authored));
    }
}

/// A linear VSS table of 16 intervals per segment indexes exactly as DIRECT10 does for
/// every UNORM8 input, in the kernel's own f32 arithmetic (all terms are powers of two).
#[test]
fn a_uniform_linear_vss_table_indexes_like_direct10() {
    for cs in 0_u32..256 {
        let x = (cs << 2) as f32 / 1024.0;
        let seg = ((x * 64.0) as u32).min(63);
        let (low, high) = (seg as f32 / 64.0, (seg + 1) as f32 / 64.0);
        let fraction = ((x - low) / (high - low)).clamp(0.0, 1.0) * 16.0;
        let local = (fraction as u32).min(15);
        assert_eq!(seg * 16 + local, cs << 2, "cs {cs}");
        assert_eq!(
            fraction, local as f32,
            "no interpolation weight at a DIRECT10 index"
        );
    }
    let cu = include_str!("../../../cuda/display/kf_color.cu");
    for needle in [
        "power2((int)seg - 32)",
        "power2((int)seg - 33)",
        "seg = (U)(x * 64.0f)",
        "vss_lookup(lut, i / 1024.0f, c, interpolate, input_entries, 0)",
        "h > 0x5800u",
    ] {
        assert!(cu.contains(needle), "kernel no longer implements {needle}");
    }
}
