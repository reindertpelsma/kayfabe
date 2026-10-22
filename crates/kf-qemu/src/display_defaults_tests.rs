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
        "KF3_DISPLAY_CAPS_PROBE",
        "KF3_DISPLAY_TMO_CONSTRUCTOR_PROBE",
        "KF3_DISPLAY_ILUT_CONSTRUCTOR_PROBE",
        "KF3_DISPLAY_TMO_SURFACE_CONSTRUCTOR_PROBE",
        "KF3_DISPLAY_OLUT_CONSTRUCTOR_PROBE",
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
fn default_sdr_pipeline_requires_opt_in() {
    isolated("default_sdr_pipeline_requires_opt_in", &[], || {
        assert!(
            !sdr_color_on(),
            "the authored SDR page is unqualified as a default (review B2, G05)"
        );
    });
}

#[test]
fn empty_sdr_override_stays_off() {
    isolated(
        "empty_sdr_override_stays_off",
        &[("KF3_DISPLAY_SDR_COLOR", "")],
        || {
            assert!(
                !sdr_color_on(),
                "only exact 1 enables the research colour path"
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

// CODE READ: exercise the exact pre-GPU configuration used by DisplayPlane::build,
// including its selection call-site, rather than just testing the environment helper.
fn config_for(row: &kf_chip::display::DisplayRow, version: &str) -> DisplayColorConfig {
    let t = kf_disp::class::for_version(version).unwrap();
    let regs = Regs::for_ip(version, row.ip_version).unwrap();
    display_color_config(row, t, &regs).unwrap()
}

#[test]
fn default_build_caps_are_plain_on_every_derived_family() {
    isolated(
        "default_build_caps_are_plain_on_every_derived_family",
        &[],
        || {
            for version in ["580.65.06", "580.159.04"] {
                let t = kf_disp::class::for_version(version).unwrap();
                for row in kf_chip::display::ALL {
                    let regs = Regs::for_ip(version, row.ip_version).unwrap();
                    let c = config_for(row, version);
                    assert!(!c.sdr_color);
                    assert!(!c.constructor_probe);
                    assert_eq!(
                        c.caps,
                        kf_disp::caps::page(t, &regs, row.classes.caps, row.heads, row.windows)
                            .unwrap()
                    );
                }
            }
        },
    );
}

#[test]
fn non_one_sdr_value_does_not_enable_processing() {
    isolated(
        "non_one_sdr_value_does_not_enable_processing",
        &[("KF3_DISPLAY_SDR_COLOR", "on")],
        || {
            assert!(!config_for(&kf_chip::display::ADA, "580.65.06").sdr_color);
        },
    );
}

#[test]
fn explicit_sdr_build_uses_the_authored_page_without_caps_probe() {
    isolated(
        "explicit_sdr_build_uses_the_authored_page_without_caps_probe",
        &[("KF3_DISPLAY_SDR_COLOR", "1")],
        || {
            let row = &kf_chip::display::ADA;
            let t = kf_disp::class::for_version("580.65.06").unwrap();
            let regs = Regs::for_ip("580.65.06", row.ip_version).unwrap();
            let c = config_for(row, "580.65.06");
            assert!(c.sdr_color);
            assert!(!c.constructor_probe);
            let authored = kf_disp::caps::sdr_page(
                t,
                &regs,
                row.classes.caps,
                row.classes.core,
                row.heads,
                row.windows,
            )
            .unwrap();
            assert_eq!(c.caps, authored);
            assert_ne!(
                c.caps,
                kf_disp::caps::page(t, &regs, row.classes.caps, row.heads, row.windows).unwrap()
            );
        },
    );
}

macro_rules! constructor_tests {
    ($name:ident, $reject:ident, $flag:literal, $author:ident, $ilut:expr, $surface:expr, $olut:expr) => {
        #[test]
        fn $name() {
            isolated(stringify!($name), &[($flag, "1")], || {
                for version in ["580.65.06", "580.159.04"] {
                    let t = kf_disp::class::for_version(version).unwrap();
                    for row in kf_chip::display::ALL {
                        let regs = Regs::for_ip(version, row.ip_version).unwrap();
                        let c = config_for(row, version);
                        assert!(!c.sdr_color, "standalone probes need no SDR_COLOR=0");
                        assert!(c.constructor_probe, "build must choose the refusing engine");
                        assert_eq!(
                            (
                                c.ilut_constructor_probe,
                                c.tmo_surface_constructor_probe,
                                c.olut_constructor_probe
                            ),
                            ($ilut, $surface, $olut)
                        );
                        assert_eq!(
                            c.caps,
                            kf_disp::caps::$author(
                                t,
                                &regs,
                                row.classes.caps,
                                row.heads,
                                row.windows
                            )
                            .unwrap()
                        );
                    }
                }
            });
        }
        #[test]
        fn $reject() {
            isolated(
                stringify!($reject),
                &[($flag, "1"), ("KF3_DISPLAY_SDR_COLOR", "1")],
                || {
                    let row = &kf_chip::display::ADA;
                    let t = kf_disp::class::for_version("580.65.06").unwrap();
                    let regs = Regs::for_ip("580.65.06", row.ip_version).unwrap();
                    let err = display_color_config(row, t, &regs)
                        .err()
                        .expect("incompatible modes must refuse before GPU bring-up");
                    assert!(
                        err.contains(
                            "SDR processing cannot be combined with constructor-only probes"
                        ),
                        "{err}"
                    );
                },
            );
        }
    };
}

constructor_tests!(
    tmo_probe_with_default_sdr,
    tmo_probe_refuses_explicit_sdr,
    "KF3_DISPLAY_TMO_CONSTRUCTOR_PROBE",
    constructor_probe_page,
    false,
    false,
    false
);
constructor_tests!(
    ilut_probe_with_default_sdr,
    ilut_probe_refuses_explicit_sdr,
    "KF3_DISPLAY_ILUT_CONSTRUCTOR_PROBE",
    ilut_constructor_probe_page,
    true,
    false,
    false
);
constructor_tests!(
    tmo_surface_probe_with_default_sdr,
    tmo_surface_probe_refuses_explicit_sdr,
    "KF3_DISPLAY_TMO_SURFACE_CONSTRUCTOR_PROBE",
    tmo_surface_constructor_probe_page,
    true,
    true,
    false
);
constructor_tests!(
    olut_probe_with_default_sdr,
    olut_probe_refuses_explicit_sdr,
    "KF3_DISPLAY_OLUT_CONSTRUCTOR_PROBE",
    olut_constructor_probe_page,
    true,
    true,
    true
);

#[test]
fn caps_probe_with_default_sdr_replaces_plain_page_without_processing() {
    isolated(
        "caps_probe_with_default_sdr_replaces_plain_page_without_processing",
        &[("KF3_DISPLAY_CAPS_PROBE", "1")],
        || {
            let c = config_for(&kf_chip::display::ADA, "580.65.06");
            assert!(!c.sdr_color);
            assert!(!c.constructor_probe);
            assert_eq!(
                Some(c.caps),
                caps_probe_page(CAPS_PROBE_AT.1, CAPS_PROBE_AT.0)
            );
        },
    );
}

#[test]
fn caps_probe_with_explicit_sdr_discards_authored_page_but_keeps_processing() {
    isolated(
        "caps_probe_with_explicit_sdr_discards_authored_page_but_keeps_processing",
        &[
            ("KF3_DISPLAY_CAPS_PROBE", "1"),
            ("KF3_DISPLAY_SDR_COLOR", "1"),
        ],
        || {
            let c = config_for(&kf_chip::display::ADA, "580.65.06");
            assert!(
                c.sdr_color,
                "the existing production arm still processes colour"
            );
            assert_eq!(
                Some(c.caps),
                caps_probe_page(CAPS_PROBE_AT.1, CAPS_PROBE_AT.0)
            );
        },
    );
}

#[test]
fn caps_probe_with_constructor_keeps_blanket_refusal() {
    isolated(
        "caps_probe_with_constructor_keeps_blanket_refusal",
        &[
            ("KF3_DISPLAY_CAPS_PROBE", "1"),
            ("KF3_DISPLAY_TMO_CONSTRUCTOR_PROBE", "1"),
        ],
        || {
            let c = config_for(&kf_chip::display::ADA, "580.65.06");
            assert!(!c.sdr_color);
            assert!(c.constructor_probe);
            assert_eq!(
                Some(c.caps),
                caps_probe_page(CAPS_PROBE_AT.1, CAPS_PROBE_AT.0)
            );
        },
    );
}

#[test]
fn caps_probe_on_another_class_keeps_its_authored_page() {
    isolated(
        "caps_probe_on_another_class_keeps_its_authored_page",
        &[("KF3_DISPLAY_CAPS_PROBE", "1")],
        || {
            let row = &kf_chip::display::TURING;
            let t = kf_disp::class::for_version("580.65.06").unwrap();
            let regs = Regs::for_ip("580.65.06", row.ip_version).unwrap();
            let c = config_for(row, "580.65.06");
            assert!(!c.sdr_color);
            assert_eq!(
                c.caps,
                kf_disp::caps::page(t, &regs, row.classes.caps, row.heads, row.windows).unwrap()
            );
        },
    );
}
