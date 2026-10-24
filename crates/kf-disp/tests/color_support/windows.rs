// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! The colour program Windows armed in runs 98/99 (RTX 4070, 2026-10-09; window 0 = class C67E,
//! head 0 of core C77D), rebuilt as register words through the PUBLIC class-table API, so an
//! integration test can drive `kf_disp::color::{input, pipeline, output}` exactly as the display
//! worker does (`kf-qemu/src/display.rs`: every `Err` of those three becomes
//! `Fault::Console("SDR colour program: ..")`). Same words as the unit test fixture
//! `color::tests::run99_windows_program`; LUT entry words are not register state.
#![allow(dead_code)]

use kf_disp::class::{ClassTable, put};
use kf_disp::color::IDENTITY;
use std::collections::HashMap;

/// Window and core classes of the measured program (run 99, 2026-10-09).
pub const WIN: u32 = 0xc67e;
/// Core class (C77D, Ada).
pub const CORE: u32 = 0xc77d;
/// The guest driver versions whose class tables the decoder carries.
pub const VERSIONS: [&str; 2] = ["580.65.06", "580.159.04"];

/// Register words by method address.
pub type Words = HashMap<u32, u32>;

fn addr(t: &ClassTable, c: u32, name: &str, head: Option<u32>) -> u32 {
    head.map_or_else(|| t.v(c, name), |h| t.a(c, name, h))
        .unwrap_or_else(|| panic!("class {c:#x} has no method {name}"))
}

/// Set one method's word.
pub fn set(b: &mut Words, t: &ClassTable, c: u32, name: &str, head: Option<u32>, v: u32) {
    b.insert(addr(t, c, name, head), v);
}

/// The measured OLUT/ILUT control word (runs 98/99, RTX 4070, 2026-10-09): DIRECT10, 1029 entries,
/// MIRROR=1, no interpolation.
pub const WINDOWS_LUT_CONTROL: u32 = 0x0004_050a;

/// Run 99's program (identity OCSC0 with the 0x20 bias, mirrored ILUT and OLUT).
#[must_use]
pub fn run99(t: &ClassTable) -> Words {
    let mut b = HashMap::new();
    for (i, v) in IDENTITY.iter().enumerate() {
        set(
            &mut b,
            t,
            WIN,
            &format!("SET_FMT_COEFFICIENT_C{}{}", i / 4, i % 4),
            None,
            *v as u32,
        );
        set(
            &mut b,
            t,
            WIN,
            &format!("SET_CSC11COEFFICIENT_C{}{}", i / 4, i % 4),
            None,
            *v as u32,
        );
    }
    set(
        &mut b,
        t,
        WIN,
        "SET_ILUT_CONTROL",
        None,
        WINDOWS_LUT_CONTROL,
    );
    set(&mut b, t, WIN, "SET_CONTEXT_DMA_ILUT", None, 0xff1f_e313);
    set(&mut b, t, WIN, "SET_OFFSET_ILUT", None, 0x21);
    set(&mut b, t, WIN, "SET_TMO_CONTROL", None, 1);
    set(&mut b, t, WIN, "SET_CSC11CONTROL", None, 1);
    set(
        &mut b,
        t,
        WIN,
        "SET_CSC0LUT_SEGMENT_SIZE",
        None,
        0x0005_0020,
    );
    set(
        &mut b,
        t,
        WIN,
        "SET_CSC1LUT_SEGMENT_SIZE",
        None,
        0x0002_003f,
    );
    set(
        &mut b,
        t,
        CORE,
        "HEAD_SET_OLUT_CONTROL",
        Some(0),
        WINDOWS_LUT_CONTROL,
    );
    set(
        &mut b,
        t,
        CORE,
        "HEAD_SET_OLUT_FP_NORM_SCALE",
        Some(0),
        u32::MAX,
    );
    set(
        &mut b,
        t,
        CORE,
        "HEAD_SET_CONTEXT_DMA_OLUT",
        Some(0),
        0xff1f_e144,
    );
    set(&mut b, t, CORE, "HEAD_SET_OFFSET_OLUT", Some(0), 0);
    set(&mut b, t, CORE, "HEAD_SET_OCSC0CONTROL", Some(0), 1);
    for (i, v) in IDENTITY.iter().enumerate() {
        let v = if i % 4 == 3 { 0x20 } else { *v as u32 };
        set(
            &mut b,
            t,
            CORE,
            &format!("HEAD_SET_OCSC0COEFFICIENT_C{}{}", i / 4, i % 4),
            Some(0),
            v,
        );
    }
    b
}

/// Write `m` into head 0's OCSC0, each coefficient encoded into its class-table field (the field
/// width is derived, not assumed).
pub fn set_ocsc0(b: &mut Words, t: &ClassTable, m: &[i32; 12]) {
    for (i, v) in m.iter().enumerate() {
        let n = format!("HEAD_SET_OCSC0COEFFICIENT_C{}{}", i / 4, i % 4);
        let f = t.f(CORE, &format!("{n}_VALUE")).expect("OCSC0 field");
        set(b, t, CORE, &n, Some(0), put(0, f, *v as u32));
    }
}

/// Write a raw word into one OCSC0 coefficient (hostile-guest fuzzing: any 32 bits).
pub fn set_ocsc0_raw(b: &mut Words, t: &ClassTable, i: usize, word: u32) {
    let n = format!("HEAD_SET_OCSC0COEFFICIENT_C{}{}", i / 4, i % 4);
    set(b, t, CORE, &n, Some(0), word);
}

/// An OLUT control word: DIRECT8 (261) or DIRECT10 (1029), interpolation and MIRROR as asked,
/// built from the class table's fields.
#[must_use]
pub fn olut_control(t: &ClassTable, direct10: bool, interpolate: bool, mirror: bool) -> u32 {
    let n = "HEAD_SET_OLUT_CONTROL";
    let f = |s: &str| t.f(CORE, &format!("{n}_{s}")).expect("OLUT field");
    let mode = t
        .v(
            CORE,
            &format!("{n}_MODE_{}", if direct10 { "DIRECT10" } else { "DIRECT8" }),
        )
        .expect("OLUT mode");
    let mut w = put(0, f("MODE"), mode);
    w = put(w, f("SIZE"), if direct10 { 1029 } else { 261 });
    w = put(w, f("INTERPOLATE"), u32::from(interpolate));
    put(w, f("MIRROR"), u32::from(mirror))
}

/// Replace head 0's OLUT control word.
pub fn set_olut_control(b: &mut Words, t: &ClassTable, w: u32) {
    set(b, t, CORE, "HEAD_SET_OLUT_CONTROL", Some(0), w);
}
