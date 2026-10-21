use super::*;
use kf_disp::class::{ClassTable, put};
use kf_disp::color::{self, Binding, Experiments, IDENTITY};
use std::collections::BTreeMap;

// CODE READ: OnceLock and environment reads belong to the child, never a parallel test's parent.
fn isolated(name: &str, values: &[(&str, &str)], check: impl FnOnce()) {
    if std::env::var("KF_TEST_COLOR_DEFAULTS_CHILD").as_deref() == Ok(name) {
        check();
        return;
    }
    let mut child = std::process::Command::new(std::env::current_exe().unwrap());
    child.args([
        "--exact",
        &format!("display::production_color_defaults_tests::{name}"),
        "--nocapture",
    ]);
    for key in [
        "KF3_DISPLAY_SDR_COLOR",
        "KF3_DISPLAY_LUT_MIRROR",
        "KF3_DISPLAY_ILUT_OFFSET_256",
    ] {
        child.env_remove(key);
    }
    let output = child
        .env("KF_TEST_COLOR_DEFAULTS_CHILD", name)
        .envs(values.iter().copied())
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("running 1 test"),
        "the child did not reach its named test"
    );
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn field(t: &ClassTable, c: u32, prefix: &str, mirror: bool) -> u32 {
    let ctl = put(
        0,
        t.f(c, &format!("{prefix}_MODE")).unwrap(),
        t.v(c, &format!("{prefix}_MODE_DIRECT10")).unwrap(),
    );
    let ctl = put(ctl, t.f(c, &format!("{prefix}_SIZE")).unwrap(), 1029);
    put(
        ctl,
        t.f(c, &format!("{prefix}_MIRROR")).unwrap(),
        u32::from(mirror),
    )
}

fn input(t: &ClassTable, win: u32, mirror: bool, offset: u32, x: Experiments) -> color::Lut {
    let mut words = BTreeMap::new();
    for (i, value) in IDENTITY.iter().enumerate() {
        words.insert(
            t.v(win, &format!("SET_FMT_COEFFICIENT_C{}{}", i / 4, i % 4))
                .unwrap(),
            *value as u32,
        );
    }
    words.insert(
        t.v(win, "SET_ILUT_CONTROL").unwrap(),
        field(t, win, "SET_ILUT_CONTROL", mirror),
    );
    words.insert(t.v(win, "SET_CONTEXT_DMA_ILUT").unwrap(), 0x123);
    words.insert(t.v(win, "SET_OFFSET_ILUT").unwrap(), offset);
    color::input(t, win, |m| words.get(&m).copied().unwrap_or(0), x)
        .unwrap()
        .unwrap()
}

#[test]
fn default_sdr_pipeline_is_selected() {
    isolated("default_sdr_pipeline_is_selected", &[], || {
        assert!(
            sdr_color_on(),
            "the production colour pipeline must not require an environment flag"
        );
    });
}

#[test]
fn empty_sdr_override_keeps_production_default() {
    isolated(
        "empty_sdr_override_keeps_production_default",
        &[("KF3_DISPLAY_SDR_COLOR", "")],
        || {
            assert!(
                sdr_color_on(),
                "only an explicit 0 selects the old no-colour arm"
            );
        },
    );
}

#[test]
fn default_mirrored_input_lut_decodes() {
    isolated("default_mirrored_input_lut_decodes", &[], || {
        let t = kf_disp::class::for_version("580.65.06").unwrap();
        let lut = input(t, 0xc67e, true, 0, color_experiments());
        assert!(lut.mirror);
        assert_eq!(lut.entries, 1025);
    });
}

#[test]
fn default_mirrored_output_lut_decodes() {
    isolated("default_mirrored_output_lut_decodes", &[], || {
        let t = kf_disp::class::for_version("580.159.04").unwrap();
        let core = 0xc77d;
        let mut words = BTreeMap::new();
        words.insert(
            t.a(core, "HEAD_SET_OLUT_CONTROL", 0).unwrap(),
            field(t, core, "HEAD_SET_OLUT_CONTROL", true),
        );
        words.insert(t.a(core, "HEAD_SET_CONTEXT_DMA_OLUT", 0).unwrap(), 0x456);
        words.insert(
            t.a(core, "HEAD_SET_OLUT_FP_NORM_SCALE", 0).unwrap(),
            u32::MAX,
        );
        words.insert(t.a(core, "HEAD_SET_OCSC0CONTROL", 0).unwrap(), 1);
        for (i, value) in IDENTITY.iter().enumerate() {
            words.insert(
                t.a(
                    core,
                    &format!("HEAD_SET_OCSC0COEFFICIENT_C{}{}", i / 4, i % 4),
                    0,
                )
                .unwrap(),
                *value as u32,
            );
        }
        let out = color::output(
            t,
            core,
            0,
            |m| words.get(&m).copied().unwrap_or(0),
            color_experiments(),
        )
        .unwrap();
        assert!(out.lut.unwrap().mirror);
        assert!(color::mirror_inert([&out.matrix]));
        let mut negative = out.matrix;
        negative[1] = -1;
        assert!(
            !color::mirror_inert([&negative]),
            "negative paths remain refused by the existing composition gate"
        );
    });
}

fn offset_units(win: u32) {
    for version in ["580.65.06", "580.159.04"] {
        let t = kf_disp::class::for_version(version).unwrap();
        let lut = input(t, win, false, 0x21, color_experiments());
        assert_eq!(
            lut.binding,
            Binding::Dma {
                handle: 0x123,
                offset: 0x2100
            }
        );
        let dma = CtxDma {
            target: kf_disp::inst::Target::Vidmem,
            base: 0x10000,
            limit: 0x1ffff,
            block_linear: false,
            writable: false,
        };
        assert_eq!(lut.binding.span(Some(&dma)), Ok(0x12100));
    }
}

#[test]
fn default_input_offsets_have_256_byte_units_turing() {
    isolated(
        "default_input_offsets_have_256_byte_units_turing",
        &[],
        || offset_units(0xc57e),
    );
}

#[test]
fn default_input_offsets_have_256_byte_units_ada() {
    isolated("default_input_offsets_have_256_byte_units_ada", &[], || {
        offset_units(0xc67e)
    });
}

#[test]
fn explicit_zero_restores_each_old_decoder_arm() {
    isolated(
        "explicit_zero_restores_each_old_decoder_arm",
        &[
            ("KF3_DISPLAY_SDR_COLOR", "0"),
            ("KF3_DISPLAY_LUT_MIRROR", "0"),
            ("KF3_DISPLAY_ILUT_OFFSET_256", "0"),
        ],
        || {
            assert!(!sdr_color_on());
            let x = color_experiments();
            assert_eq!(x, Experiments::default());
            let t = kf_disp::class::for_version("580.65.06").unwrap();
            assert_eq!(
                input(t, 0xc67e, false, 0x100, x).binding,
                Binding::Dma {
                    handle: 0x123,
                    offset: 0x100
                }
            );
        },
    );
}

#[test]
fn explicit_one_keeps_existing_production_profile() {
    isolated(
        "explicit_one_keeps_existing_production_profile",
        &[
            ("KF3_DISPLAY_SDR_COLOR", "1"),
            ("KF3_DISPLAY_LUT_MIRROR", "1"),
            ("KF3_DISPLAY_ILUT_OFFSET_256", "1"),
        ],
        || {
            assert!(sdr_color_on());
            let t = kf_disp::class::for_version("580.65.06").unwrap();
            let lut = input(t, 0xc67e, true, 0x21, color_experiments());
            assert!(lut.mirror);
            assert_eq!(
                lut.binding,
                Binding::Dma {
                    handle: 0x123,
                    offset: 0x2100
                }
            );
        },
    );
}
