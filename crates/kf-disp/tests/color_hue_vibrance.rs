// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! PR #92, automated: the NVIDIA app's Hue and Digital Vibrance sliders put SIGNED coefficients
//! in head 0's OCSC0 behind Windows' MIRRORED OLUT (run 4500 froze the console at 0 fps on the
//! refusal "mirrored OLUT behind a signed matrix", a `Fault::Console`). These tests pin, GPU-free:
//!
//! 1. DECODE: for a table of hue / vibrance / boundary configurations, written as register words
//!    into run 99's measured Windows program, `color::{input, pipeline, output}` return `Ok` —
//!    i.e. none of them can produce the `Fault::Console("SDR colour program: ..")` the display
//!    worker raises on an `Err` (`kf-qemu/src/display.rs`, the colour-program closure). The
//!    coefficients decode bit-exactly (the field width comes from the class table).
//! 2. PIXELS: the CPU oracle `color::model::output_rgb` (the kernel's arithmetic, `f32`) agrees
//!    with an INDEPENDENT `f64` reference (`color_support/reference.rs`) for every configuration,
//!    every table shape (both sizes, interpolated or not, mirrored or not) and a colour grid.
//! 3. PROPERTIES (bounded, deterministic): any 21-bit coefficient a hostile guest writes decodes
//!    without an error and with the right sign; random programs agree with the reference; a
//!    mirrored negative channel is black; an unmirrored one is the table's first sample; MIRROR is
//!    inert where nothing is negative.
//!
//! What this does NOT prove: that the real display engine's MIRROR is the odd extension
//! (`[inferred]`, `V3_SDR_COLOR.md` falsifier), that the Windows driver programs these exact
//! matrices (the lane `scripts/bench/windows/hue_vibrance_lane.sh` records the real ones), or that
//! the GPU runs the kernel this way (`color_output_ptx.rs` runs the committed PTX on the host; the
//! hardware example `kf-cuda/examples/color_hue_vibrance.rs` runs it on a GPU).

#[path = "color_support/reference.rs"]
mod reference;
#[path = "color_support/windows.rs"]
mod windows;

use kf_disp::class::for_version;
use kf_disp::color::{
    self, Binding, Experiments, InlineLut, Output, model, negative_side_reachable,
};
use kf_disp::inst::{CtxDma, Target};
use reference::{Config, RefTable, Rng, Span};
use windows::{CORE, VERSIONS, WIN};

/// The production Windows flag set (`KF3_DISPLAY_LUT_MIRROR`, `KF3_DISPLAY_ILUT_OFFSET_256`).
const PROD: Experiments = Experiments {
    lut_mirror: true,
    ilut_offset_256: true,
};

/// Decode the whole program as the display worker does; `Err` = the console would refuse.
fn decode(
    t: &kf_disp::class::ClassTable,
    b: &windows::Words,
) -> Result<(Vec<[i32; 12]>, Output), String> {
    let r = |m: u32| b.get(&m).copied().unwrap_or(0);
    let lut = color::input(t, WIN, r, PROD).map_err(|e| format!("input: {e}"))?;
    let dma = CtxDma {
        target: Target::Vidmem,
        base: 0,
        limit: 0xffff,
        block_linear: false,
        writable: false,
    };
    if let Some(l) = lut {
        l.binding
            .span(Some(&dma))
            .map_err(|e| format!("ILUT span: {e}"))?;
    }
    let tables: [InlineLut; 2] = std::array::from_fn(|_| InlineLut::default());
    let p = color::pipeline(t, WIN, r, &tables).map_err(|e| format!("pipeline: {e}"))?;
    let o = color::output(t, CORE, 0, r, PROD).map_err(|e| format!("output: {e}"))?;
    if let Some(l) = o.lut {
        l.binding
            .span(Some(&dma))
            .map_err(|e| format!("OLUT span: {e}"))?;
    }
    Ok((p.matrices.to_vec(), o))
}

/// Part 1: every configuration in the table decodes (no `Fault::Console`), exactly, in both driver
/// versions and for every OLUT control shape Windows can arm.
#[test]
fn every_hue_and_vibrance_configuration_decodes_without_a_console_fault() {
    let configs = reference::configurations();
    assert!(
        configs.len() > 250,
        "the table is the sweep, not a sample ({})",
        configs.len()
    );
    let mut decoded = 0;
    for version in VERSIONS {
        let t = for_version(version).unwrap();
        // the measured control word is DIRECT10 / 1029 / MIRROR, no interpolation
        assert_eq!(
            windows::olut_control(t, true, false, true),
            windows::WINDOWS_LUT_CONTROL,
            "the control-word builder reproduces run 99's word"
        );
        for (direct10, interpolate, mirror) in [
            (true, false, true),
            (true, true, true),
            (false, false, true),
            (false, true, true),
            (true, false, false),
            (true, true, false),
        ] {
            let ctl = windows::olut_control(t, direct10, interpolate, mirror);
            for Config { name, matrix } in &configs {
                let mut b = windows::run99(t);
                windows::set_olut_control(&mut b, t, ctl);
                windows::set_ocsc0(&mut b, t, matrix);
                let (window, o) = decode(t, &b).unwrap_or_else(|e| {
                    panic!("{version} {name} ctl={ctl:#x}: refused (a frozen console): {e}")
                });
                assert_eq!(
                    &o.matrix, matrix,
                    "{version} {name}: coefficients decode exactly"
                );
                let l = o.lut.expect("OLUT armed");
                assert_eq!(
                    (l.entries, l.interpolate, l.mirror),
                    (if direct10 { 1025 } else { 257 }, interpolate, mirror),
                    "{version} {name} ctl={ctl:#x}"
                );
                let signed = matrix.iter().any(|&v| v < 0);
                assert_eq!(
                    negative_side_reachable(window.iter(), &o),
                    signed && mirror,
                    "{version} {name}: reachability"
                );
                decoded += 1;
            }
        }
    }
    assert_eq!(decoded, configs.len() * 6 * VERSIONS.len());
}

/// Negative control: the same table WITHOUT `KF3_DISPLAY_LUT_MIRROR` is refused (so the test
/// above can see a refusal: it is not vacuous), with the shipped reason.
#[test]
fn without_the_mirror_experiment_the_windows_program_is_refused() {
    for version in VERSIONS {
        let t = for_version(version).unwrap();
        let mut b = windows::run99(t);
        windows::set_ocsc0(&mut b, t, &reference::pack(reference::hue_w3c(120.0), 0x20));
        let r = |m: u32| b.get(&m).copied().unwrap_or(0);
        let off = Experiments {
            lut_mirror: false,
            ilut_offset_256: true,
        };
        assert_eq!(
            color::output(t, CORE, 0, r, off).map(|_| ()),
            Err("LUT requires matching unmirrored DIRECT8 or DIRECT10 extent")
        );
    }
}

fn model_table<'a>(t: &RefTable<'a>) -> model::Table<'a> {
    model::Table {
        entries: t.entries,
        interpolate: t.interpolate,
        mirror: t.mirror,
    }
}

/// Compare one pixel; push a message on a mismatch; count sharp channels.
fn check(
    what: &str,
    src: [f32; 3],
    m: &[i32; 12],
    t: Option<&RefTable<'_>>,
    bad: &mut Vec<String>,
    sharp: &mut (usize, usize),
) -> [u8; 3] {
    let mt = t.map(model_table);
    let got = model::output_rgb(src, m, mt.as_ref());
    let want: [Span; 3] = reference::output(src.map(f64::from), m, t);
    let exact = reference::output_exact(src.map(f64::from), m, t);
    for c in 0..3 {
        sharp.1 += 1;
        sharp.0 += usize::from(i32::from(got[c]) == exact[c]);
        if !want[c].admits(got[c]) {
            bad.push(format!(
                "{what} src={src:?} c={c}: model {} reference {:?} (exact {exact:?})",
                got[c], want[c]
            ));
        }
    }
    got
}

fn report(what: &str, bad: &[String], sharp: (usize, usize)) {
    assert!(
        bad.is_empty(),
        "{what}: {} mismatches, first: {:#?}",
        bad.len(),
        &bad[..bad.len().min(5)]
    );
    // the f32 oracle equals the unperturbed f64 reference almost everywhere: the interval only
    // absorbs rounding at a decision boundary, it does not make the comparison loose
    assert!(
        sharp.0 * 1000 >= sharp.1 * 998,
        "{what}: only {}/{} channels equal the reference exactly",
        sharp.0,
        sharp.1
    );
    eprintln!(
        "{what}: {}/{} channels equal the unperturbed reference",
        sharp.0, sharp.1
    );
}

/// Part 2: the CPU oracle against the independent reference: every configuration x every table x
/// mirror on/off x interpolation on/off x a 7^3 colour grid, plus OLUT bypass.
#[test]
fn the_output_stage_matches_the_reference_for_every_configuration_and_table() {
    let configs = reference::configurations();
    let tables = reference::tables();
    let grid = reference::grid(&[0, 32, 64, 128, 192, 224, 256]);
    let (mut bad, mut sharp) = (Vec::new(), (0, 0));
    let mut negative = 0;
    for cfg in &configs {
        for src in &grid {
            check(
                &format!("{} bypass", cfg.name),
                *src,
                &cfg.matrix,
                None,
                &mut bad,
                &mut sharp,
            );
        }
        for nt in &tables {
            for (interpolate, mirror) in
                [(false, false), (false, true), (true, false), (true, true)]
            {
                let t = RefTable {
                    entries: &nt.entries,
                    interpolate,
                    mirror,
                };
                for src in &grid {
                    let what = format!(
                        "{} | {} x{} interp={interpolate} mirror={mirror}",
                        cfg.name,
                        nt.name,
                        nt.entries.len()
                    );
                    check(&what, *src, &cfg.matrix, Some(&t), &mut bad, &mut sharp);
                    if mirror {
                        negative += (0..3)
                            .filter(|&c| {
                                reference::pre_table(src.map(f64::from), &cfg.matrix, c) < -1e-3
                            })
                            .count();
                    }
                }
            }
        }
    }
    assert!(
        negative > 100_000,
        "the sweep reaches the negative side ({negative})"
    );
    report("configurations", &bad, sharp);
}

/// The decoded words, not just the matrices: register words -> decode -> oracle pixels equal the
/// reference on the words' own values (catches a decode that drops a bit or a sign).
#[test]
fn register_words_to_pixels_match_the_reference() {
    let tables = reference::tables();
    let lifted = tables
        .iter()
        .find(|t| t.name.starts_with("lifted") && t.entries.len() == 1025)
        .unwrap();
    let grid = reference::grid(&[0, 64, 128, 256]);
    let (mut bad, mut sharp) = (Vec::new(), (0, 0));
    for version in VERSIONS {
        let t = for_version(version).unwrap();
        for cfg in reference::configurations() {
            let mut b = windows::run99(t);
            windows::set_ocsc0(&mut b, t, &cfg.matrix);
            let (_, o) = decode(t, &b).unwrap();
            let l = o.lut.unwrap();
            let rt = RefTable {
                entries: &lifted.entries,
                interpolate: l.interpolate,
                mirror: l.mirror,
            };
            for src in &grid {
                check(
                    &format!("{version} {}", cfg.name),
                    *src,
                    &o.matrix,
                    Some(&rt),
                    &mut bad,
                    &mut sharp,
                );
            }
        }
    }
    report("register words", &bad, sharp);
}

/// Hand-computed values (no reference code): the four cases the hardware example checks.
#[test]
fn hand_computed_hue_and_vibrance_pixels() {
    let lifted: Vec<[u16; 3]> = (0..1025_u32)
        .map(|i| [(16384 + i * 32) as u16; 3])
        .collect();
    let t = |interpolate, mirror| model::Table {
        entries: &lifted,
        interpolate,
        mirror,
    };
    let neg = [65536, -32768, 0, 0, 0, 65536, 0, 0, 0, 0, 65536, 0];
    // R' = 0.25 - 0.375 = -0.125: mirrored -> -(0.25 + 0.0625) -> 0; unmirrored -> first sample 64
    assert_eq!(
        model::output_rgb([0.25, 0.75, 0.0], &neg, Some(&t(false, true))),
        [0, 160, 64]
    );
    assert_eq!(
        model::output_rgb([0.25, 0.75, 0.0], &neg, Some(&t(false, false))),
        [64, 160, 64]
    );
    let vib = reference::pack(reference::saturation(1.5), 0);
    assert_eq!(
        model::output_rgb([1.0, 0.0, 0.0], &vib, Some(&t(true, true))),
        [192, 0, 0]
    );
    assert_eq!(
        model::output_rgb([1.0, 0.0, 0.0], &vib, Some(&t(true, false))),
        [192, 64, 64]
    );
    // grey is a fixed point of both constructions (rows sum to 1, within the encoding's rounding)
    for m in [
        reference::hue_w3c(77.0),
        reference::hue_ycbcr(77.0),
        reference::saturation(2.5),
    ] {
        for row in m {
            assert!((row.iter().sum::<f64>() - 1.0).abs() < 1e-3, "{row:?}");
        }
    }
}

/// Part 3a, hostile guest: ANY 32-bit word in any OCSC0 coefficient decodes (no error, no panic) to
/// the sign-extended 21-bit field; the extra bits are dropped. Bounded: 4096 random programs per
/// driver version, plus the field's corners.
#[test]
fn any_coefficient_word_decodes_to_its_sign_extended_field() {
    let mut rng = Rng(0x0009_20c5_c0de_u64);
    for version in VERSIONS {
        let t = for_version(version).unwrap();
        let f = t.f(CORE, "HEAD_SET_OCSC0COEFFICIENT_C00_VALUE").unwrap();
        assert_eq!(
            f.0 - f.1 + 1,
            21,
            "{version}: the S5.14 field is 21 bits ({f:?})"
        );
        let corners = [
            0,
            1,
            0xf_ffff,
            0x10_0000,
            0x1f_ffff,
            0x20_0000,
            u32::MAX,
            0x8000_0000,
        ];
        for case in 0..4096 + corners.len() {
            let mut b = windows::run99(t);
            let words: [u32; 12] = std::array::from_fn(|i| {
                corners
                    .get(case)
                    .map_or_else(|| rng.next() as u32, |&c| c.rotate_left(i as u32 % 2))
            });
            for (i, w) in words.iter().enumerate() {
                windows::set_ocsc0_raw(&mut b, t, i, *w);
            }
            let (_, o) = decode(t, &b).unwrap_or_else(|e| panic!("{version} {words:x?}: {e}"));
            let want = words.map(|w| reference::sign_extend_21(w >> f.1));
            assert_eq!(o.matrix, want, "{version} {words:x?}");
        }
    }
}

/// Part 3b: random programs, random tables: the oracle agrees with the reference, and the three
/// structural properties hold. 20000 bounded cases, fixed seed (a failure prints its case).
#[test]
fn random_programs_agree_with_the_reference_and_keep_the_mirror_properties() {
    let mut rng = Rng(0x0dd_5ee_d92);
    let (mut bad, mut sharp) = (Vec::new(), (0, 0));
    let (mut neg_mirrored, mut neg_plain) = (0, 0);
    for case in 0..20_000 {
        let n = if rng.coin() { 1025 } else { 257 };
        // random tables: monotone ramps, random noise, constant
        let kind = rng.below(3);
        let base = rng.below(65536) as u16;
        let entries: Vec<[u16; 3]> = (0..n)
            .map(|i| match kind {
                0 => [
                    ((i * 65535) / (n - 1)) as u16,
                    base,
                    (65535 - (i * 65535) / (n - 1)) as u16,
                ],
                1 => [rng.next() as u16, rng.next() as u16, rng.next() as u16],
                _ => [base; 3],
            })
            .collect();
        let t = RefTable {
            entries: &entries,
            interpolate: rng.coin(),
            mirror: rng.coin(),
        };
        // coefficients: mostly within +-4 (realistic), sometimes the whole field
        let m: [i32; 12] = std::array::from_fn(|_| {
            if rng.below(8) == 0 {
                reference::sign_extend_21(rng.next() as u32)
            } else {
                (rng.below(8 * 65536) as i32) - 4 * 65536
            }
        });
        let src: [f32; 3] = std::array::from_fn(|_| rng.below(257) as f32 / 256.0);
        let what = format!(
            "case {case} n={n} kind={kind} interp={} mirror={} m={m:?}",
            t.interpolate, t.mirror
        );
        let got = check(&what, src, &m, Some(&t), &mut bad, &mut sharp);
        let mt = model_table(&t);
        for c in 0..3 {
            let v = reference::pre_table(src.map(f64::from), &m, c);
            if v < -reference::EPS {
                if t.mirror {
                    // odd extension of a UNORM table behind the final clamp: black
                    neg_mirrored += 1;
                    if got[c] != 0 {
                        bad.push(format!(
                            "{what}: mirrored negative channel {c} is {} not 0",
                            got[c]
                        ));
                    }
                } else {
                    // unmirrored: the first sample, whatever the interpolation
                    neg_plain += 1;
                    let first = reference::quantise(f64::from(entries[0][c]) / 65536.0);
                    if i32::from(got[c]) != first {
                        bad.push(format!(
                            "{what}: unmirrored negative channel {c} is {} not {first}",
                            got[c]
                        ));
                    }
                }
            }
        }
        // MIRROR is inert when no channel is negative
        if (0..3).all(|c| reference::pre_table(src.map(f64::from), &m, c) >= 0.0) {
            let flipped = model::Table {
                mirror: !mt.mirror,
                ..mt
            };
            if model::output_rgb(src, &m, Some(&flipped)) != got {
                bad.push(format!("{what}: MIRROR changed a nonnegative pixel"));
            }
        }
    }
    assert!(
        neg_mirrored > 1000 && neg_plain > 1000,
        "{neg_mirrored} {neg_plain}"
    );
    report("random programs", &bad, sharp);
}

/// NaN and infinities cannot reach the output stage (the compose kernel flags a non-finite value
/// and the frame is not published), but the arithmetic is still total on them: no panic, a
/// defined byte. Pinned so a refactor cannot turn a hostile value into a crash.
#[test]
fn non_finite_inputs_give_defined_bytes() {
    let lifted: Vec<[u16; 3]> = (0..1025_u32)
        .map(|i| [(16384 + i * 32) as u16; 3])
        .collect();
    for mirror in [false, true] {
        let t = model::Table {
            entries: &lifted,
            interpolate: true,
            mirror,
        };
        let m = reference::pack(reference::saturation(1.5), 0x20);
        // NaN is not negative: it samples the first entry (64)
        assert_eq!(model::output_rgb([f32::NAN; 3], &m, Some(&t)), [64, 64, 64]);
        let inf = model::output_rgb([f32::INFINITY, 0.0, 0.0], &m, Some(&t));
        let ninf = model::output_rgb([f32::NEG_INFINITY, 0.0, 0.0], &m, Some(&t));
        assert_eq!(inf[0], 192, "+inf saturates to the last sample");
        assert_eq!(ninf[0], if mirror { 0 } else { 64 });
    }
    let _ = Binding::Vidmem(0);
}
