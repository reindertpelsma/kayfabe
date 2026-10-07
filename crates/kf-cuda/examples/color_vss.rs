//! Synthetic GPU oracle for segmented (VSS) ILUT/OLUT tables and the OLUT FP
//! normalization. No host display settings or guest memory.
//!
//! ⊘ UNRUN (2026-10-07): written GPU-free while the borrowed host GPU was reserved.
//! It has not executed on hardware; no result may be claimed from it until it has.
//!
//! The expectation is independent of the kernel's table lookup: OGKM's PQ EOTF ILUT
//! followed by its PQ OETF OLUT must reproduce every UNORM8 grey level (an ST 2084
//! round trip, evaluated analytically below) within one code of table interpolation.
#![allow(dead_code)] // the shared OGKM fixture also carries the CSC matrices
use kf_cuda::display::{ColorFixture, ComposeLayer, DisplayGpu};
include!("tmo_fixture_data.inc");

const M1: f64 = 2610.0 / 16384.0;
const M2: f64 = 2523.0 / 4096.0 * 128.0;
const C1: f64 = 3424.0 / 4096.0;
const C2: f64 = 2413.0 / 4096.0 * 32.0;
const C3: f64 = 2392.0 / 4096.0 * 32.0;

fn pq_oetf(y: f64) -> f64 {
    let p = y.clamp(0.0, 1.0).powf(M1);
    ((C1 + C2 * p) / (1.0 + C3 * p)).powf(M2)
}
fn pq_eotf(e: f64) -> f64 {
    let p = e.clamp(0.0, 1.0).powf(1.0 / M2);
    ((p - C1).max(0.0) / (C2 - C3 * p)).powf(1.0 / M1)
}
fn from_half(bits: u32) -> f64 {
    let (e, m) = ((bits >> 10) & 31, f64::from(bits & 1023));
    if e == 0 {
        m * 2f64.powi(-24)
    } else {
        (1.0 + m / 1024.0) * 2f64.powi(e as i32 - 15)
    }
}
/// Nonnegative finite f64 to FP16, round to nearest even (fixture authoring only).
fn to_half(v: f64) -> u16 {
    if v < 2f64.powi(-14) {
        return (v / 2f64.powi(-24)).round_ties_even() as u16;
    }
    let mut e = v.log2().floor() as i32;
    let mut m = (v / 2f64.powi(e) - 1.0) * 1024.0;
    if m < 0.0 {
        e -= 1;
        m = (v / 2f64.powi(e) - 1.0) * 1024.0;
    }
    let m = m.round_ties_even() as u16;
    (((e + 15) as u16) << 10) + m
}

/// A VSS table: four header entries of 3-bit log2 interval counts, the samples, and
/// NVKMS's extra copy of the last sample (`nvkms-evo3.c` `EvoSetupPQEotfBaseLutC5`).
fn vss(log2: &[u32], samples: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for entry in 0..4 {
        let header = log2
            .iter()
            .enumerate()
            .skip(entry * 16)
            .take(16)
            .fold(0_u64, |h, (i, &l)| h | (u64::from(l) << ((i % 16) * 3)));
        bytes.extend_from_slice(&header.to_le_bytes());
    }
    for &v in samples.iter().chain(samples.last()) {
        for c in [v, v, v, 0] {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
    }
    bytes
}
fn direct(samples: impl Fn(u32) -> u16) -> Vec<u8> {
    let mut bytes = vec![0; 32];
    for i in 0..1025 {
        for c in [samples(i.min(1023)); 3].into_iter().chain([0]) {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
    }
    bytes
}
fn grey(pixels: &[u8]) -> Vec<u32> {
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| {
            assert!(p[0] == p[1] && p[1] == p[2] && p[3] == 255, "grey {p:?}");
            u32::from(p[0])
        })
        .collect()
}

fn main() -> Result<(), String> {
    let bdf = std::env::args()
        .nth(1)
        .ok_or("usage: color_vss <host PCI BDF>")?;
    let mut gpu = DisplayGpu::bring_up_on(&bdf).map_err(|e| e.to_string())?;
    let layer = ComposeLayer {
        src: 0,
        extent: 1024,
        block_linear: false,
        pitch: 1024,
        block_height_log2: 0,
        x0_bytes: 0,
        y0: 0,
        width: 256,
        rows: 1,
        ox: 0,
        oy: 0,
        flags: 4,
        a_s: 255,
        b_s: 0,
        a_d: 0,
        b_d: 0,
    };
    let unit = [65536, 0, 0, 0, 0, 65536, 0, 0, 0, 0, 65536, 0];
    let surface: Vec<u8> = (0..=255_u8).flat_map(|v| [v, v, v, 255]).collect();
    let eotf: Vec<u16> = EOTF_P_Q512_ENTRIES.iter().map(|&e| e as u16).collect();
    let eotf_drm: Vec<u16> = EOTF_P_Q512_ENTRIES
        .iter()
        .map(|&e| to_half(from_half(e) / 125.0))
        .collect();
    let oetf: Vec<u16> = OETF_P_Q512_ENTRIES.iter().map(|&e| e as u16).collect();
    let ilut = vss(&EOTF_P_Q512_SEG_SIZES_LOG2, &eotf);
    let ilut_drm = vss(&EOTF_P_Q512_SEG_SIZES_LOG2, &eotf_drm);
    let olut = vss(&OETF_P_Q512_SEG_SIZES_LOG2, &oetf);
    let fixture = ColorFixture {
        surface: &surface,
        input: &ilut,
        output: &olut,
        layer: &layer,
        matrix: &unit,
        size: (256, 1),
        interpolate: true,
        mutate_input: None,
        rearm: false,
        tone: None,
        input_segmented: true,
        output_segmented: true,
        norm: u32::MAX / 125,
    };
    let run = |gpu: &mut DisplayGpu, f: &ColorFixture<'_>| {
        gpu.selftest_color(f).map_err(|e| e.to_string())
    };
    let round_trip = |pixels: &[u8]| {
        for (cs, got) in grey(pixels).into_iter().enumerate() {
            let x = f64::from((cs as u32) << 2) / 1024.0;
            let want = ((pq_oetf(pq_eotf(x)) * 256.0) as u32).min(255);
            assert!(
                got.abs_diff(want) <= 1,
                "grey {cs}: got {got}, ST 2084 {want}"
            );
        }
    };

    // NVKMS's own HDR program: ILUT in [0,125] units, OLUT input scaled by 1/125.
    round_trip(&run(&mut gpu, &fixture)?);
    println!("VSS_GPU nvkms_pq_round_trip PASS 256 greys within 1 code, norm=1/125");

    // nvidia-drm's PQ degamma (ILUT divided by 125) with the default normalization.
    let drm = ColorFixture {
        input: &ilut_drm,
        norm: u32::MAX,
        ..fixture
    };
    round_trip(&run(&mut gpu, &drm)?);
    println!("VSS_GPU drm_pq_round_trip PASS 256 greys within 1 code, norm=1.0");

    // Linear VSS with 16 intervals per segment must equal DIRECT10 on the same data.
    let curve = |i: u32| to_half((f64::from(i) / 1024.0).powf(2.2));
    let identity = direct(|i| (i << 6) as u16);
    let uniform = vss(&[4; 64], &(0..1024).map(curve).collect::<Vec<_>>());
    let direct10 = direct(curve);
    let linear = ColorFixture {
        input: &uniform,
        output: &identity,
        output_segmented: false,
        norm: u32::MAX,
        ..fixture
    };
    let a = run(&mut gpu, &linear)?;
    let b = run(
        &mut gpu,
        &ColorFixture {
            input: &direct10,
            input_segmented: false,
            ..linear
        },
    )?;
    assert_eq!(a, b, "uniform VSS and DIRECT10 differ");
    println!("VSS_GPU uniform_equals_direct10 PASS 256 greys identical");

    // The normalization applies to a DIRECT OLUT too: 1.0 scaled by one half.
    let white = direct(|_| 0x3c00);
    let half = run(
        &mut gpu,
        &ColorFixture {
            input: &white,
            input_segmented: false,
            output: &identity,
            output_segmented: false,
            interpolate: false,
            norm: u32::MAX / 2,
            ..fixture
        },
    )?;
    assert!(grey(&half).iter().all(|&v| v == 128), "{:?}", grey(&half));
    println!("VSS_GPU direct_olut_norm PASS 1.0 * 0.5 selects index 512");

    // Hostile tables: refused by GPU validation of the snapshot, before any pixel use.
    let refuse = |gpu: &mut DisplayGpu, f: &ColorFixture<'_>, why: &str| {
        let e = gpu.selftest_color(f).unwrap_err().to_string();
        assert!(e.contains(why), "wrong refusal: {e}");
    };
    let mut wide = ilut.clone();
    wide[..32].fill(0xff); // every header field 7: 8192 intervals in 508 entries
    refuse(
        &mut gpu,
        &ColorFixture {
            input: &wide,
            ..fixture
        },
        "header exceeds",
    );
    let mut wide_out = olut.clone();
    wide_out[..32].fill(0xff);
    refuse(
        &mut gpu,
        &ColorFixture {
            output: &wide_out,
            ..fixture
        },
        "header exceeds",
    );
    for bad in [0x7c00_u16, 0x7e00, 0x8001, 0x5801] {
        let mut t = ilut.clone();
        t[(4 + 100) * 8..(4 + 100) * 8 + 2].copy_from_slice(&bad.to_le_bytes());
        refuse(
            &mut gpu,
            &ColorFixture {
                input: &t,
                ..fixture
            },
            "segmented input LUT",
        );
    }
    println!("VSS_GPU hostile PASS header overrun, infinity, NaN, negative, >128 refused");
    Ok(())
}
