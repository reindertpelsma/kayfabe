// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! Public NVKMS documents S5.14 CSC coefficients (580.159.04 b81d58e,
//! nvkms-evo3.c:374-432). Report the precision of the already declared matrices;
//! the diagnostic native CAPC word is not a product input or expected whole word.
use kf_disp::capsderive::{Die, HostFacts, derive, shipped_rows};
use kf_disp::class::{ClassTable, get};
use kf_disp::regs::Regs;

fn page(t: &ClassTable, row: &kf_chip::display::DisplayRow) -> kf_disp::capsderive::Derived {
    let r = Regs::for_ip(&t.version, row.ip_version).unwrap();
    derive(
        t,
        &r,
        row,
        Die {
            architecture: 0,
            implementation: 0,
        },
        true,
        shipped_rows(),
        &[],
        "580",
        &HostFacts::default(),
    )
    .unwrap()
}

#[test]
fn declared_csc_stages_report_public_precision_in_every_display_cell() {
    for version in [
        "580.65.06",
        "580.94.02",
        "580.159.04",
        "590.48.01",
        "595.91.07",
    ] {
        let t = kf_disp::class::for_version(version).unwrap();
        for row in kf_chip::display::ALL {
            let d = page(t, row);
            let c = row.classes.caps;
            for i in 0..t.v(c, "PRECOMP_WIN_PIPE_HDR_CAPC__SIZE_1").unwrap() {
                let word = d.page.word(t.a(c, "PRECOMP_WIN_PIPE_HDR_CAPC", i).unwrap());
                for stage in ["CSC00", "CSC01"] {
                    let f = t
                        .f(c, &format!("PRECOMP_WIN_PIPE_HDR_CAPC_{stage}_PRECISION"))
                        .unwrap();
                    assert_eq!(
                        get(word, f),
                        if i < row.windows { 14 } else { 0 },
                        "{version} NV{c:04X} window{i} {stage}"
                    );
                }
            }
        }
    }
}

#[test]
fn precision_uses_generated_addresses_and_fields_including_the_final_window() {
    let raw = include_str!("../data/classes-580.65.06.tsv");
    let t = ClassTable::parse(&format!(
        "{raw}\nA\tNVCA73_PRECOMP_WIN_PIPE_HDR_CAPC\t2944\t32\nF\tNVCA73_PRECOMP_WIN_PIPE_HDR_CAPC_CSC00_PRECISION\t9\t5\nF\tNVCA73_PRECOMP_WIN_PIPE_HDR_CAPC_CSC01_PRECISION\t25\t21\n"
    ));
    let row = &kf_chip::display::BLACKWELL_GB20X;
    let d = page(&t, row);
    for i in 0..32 {
        let word = d.page.word(2944 + i * 32);
        let expected = if i < row.windows { 14 } else { 0 };
        assert_eq!(get(word, (9, 5)), expected, "CSC00 window{i}");
        assert_eq!(get(word, (25, 21)), expected, "CSC01 window{i}");
    }
}

#[test]
fn precision_does_not_advertise_blackwell_function_luts_or_unity_clamping() {
    for version in [
        "580.65.06",
        "580.94.02",
        "580.159.04",
        "590.48.01",
        "595.91.07",
    ] {
        let t = kf_disp::class::for_version(version).unwrap();
        let row = &kf_chip::display::BLACKWELL_GB20X;
        let d = page(t, row);
        let c = row.classes.caps;
        for i in 0..row.windows {
            let word = d.page.word(t.a(c, "PRECOMP_WIN_PIPE_HDR_CAPC", i).unwrap());
            for field in [
                "CSC0LUT_FVLUT",
                "CSC0LUT_DIRECT",
                "CSC00_UNITY_CLAMP",
                "CSC01_UNITY_CLAMP",
            ] {
                let f = t
                    .f(c, &format!("PRECOMP_WIN_PIPE_HDR_CAPC_{field}"))
                    .unwrap();
                assert_eq!(get(word, f), 0, "{version} window{i} {field}");
            }
        }
    }
}
