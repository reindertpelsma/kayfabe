//! Synthetic host facts exercise same-die ownership and the exact derived fallback.
use kf_disp::{
    capsderive::{self, Die, HostFacts, Tier},
    class::{self, get},
    hostcaps,
    regs::Regs,
};
use std::cell::Cell;
const DIE: Die = Die {
    architecture: 0x1b0,
    implementation: 2,
};
fn table() -> &'static class::ClassTable {
    class::for_version("580.65.06").unwrap()
}
fn derive(
    t: &class::ClassTable,
    row: &kf_chip::display::DisplayRow,
    die: Die,
    host: &HostFacts,
) -> capsderive::Derived {
    capsderive::derive(
        t,
        &Regs::for_ip(&t.version, row.ip_version).unwrap(),
        row,
        die,
        true,
        capsderive::shipped_rows(),
        &[],
        "580",
        host,
    )
    .unwrap()
}
fn snapshot(t: &class::ClassTable, row: &kf_chip::display::DisplayRow, die: Die) -> HostFacts {
    HostFacts {
        precomp: Some(
            hostcaps::query(
                t,
                &Regs::for_ip(&t.version, row.ip_version).unwrap(),
                row,
                die,
                |a| Ok(a.iter().map(|x| x ^ 0x1234_0000).collect()),
            )
            .unwrap(),
        ),
    }
}
#[test]
fn host_first_and_last_precomp_words_replace_only_their_t1_words() {
    let t = table();
    let row = &kf_chip::display::BLACKWELL_GB20X;
    let h = snapshot(t, row, DIE);
    let d = derive(t, row, DIE, &h);
    let fallback = derive(t, row, DIE, &HostFacts::default());
    let base = Regs::for_ip(&t.version, row.ip_version)
        .unwrap()
        .f("NV_PDISP_FE_SW")
        .unwrap()
        .1 as u32;
    let offsets: Vec<_> = (0..row.windows)
        .flat_map(|i| {
            ['A', 'B', 'C', 'D', 'E', 'F'].map(move |c| {
                t.a(row.classes.caps, &format!("PRECOMP_WIN_PIPE_HDR_CAP{c}"), i)
                    .unwrap()
            })
        })
        .collect();
    for off in &offsets {
        assert_eq!(d.page.word(*off), fallback.page.word(*off));
        assert_eq!(
            h.precomp
                .as_ref()
                .unwrap()
                .observations_for(t, row, DIE)
                .unwrap()
                .iter()
                .find(|(o, _)| o == off)
                .unwrap()
                .1,
            (base + off) ^ 0x1234_0000
        );
        assert!(
            !d.provenance
                .iter()
                .any(|x| x.offset == *off && x.tier == Tier::Host)
        );
    }
    for &(off, value) in &fallback.page.words {
        if !offsets.contains(&off) {
            assert_eq!(d.page.word(off), value, "outside PRECOMP");
        }
    }
    assert_eq!(d.unmeasured, fallback.unmeasured);
}
#[test]
fn every_family_uses_its_generated_address_and_keeps_inactive_windows_derived() {
    for row in kf_chip::display::ALL {
        let t = table();
        let die = representative(row);
        let h = snapshot(t, row, die);
        let d = derive(t, row, die, &h);
        let fallback = derive(t, row, die, &HostFacts::default());
        assert_eq!(
            d.provenance.iter().filter(|p| p.tier == Tier::Host).count(),
            0
        );
        for i in 0..row.windows {
            let off = t
                .a(row.classes.caps, "PRECOMP_WIN_PIPE_HDR_CAPC", i)
                .unwrap();
            assert_eq!(d.page.word(off), fallback.page.word(off));
        }
        for i in row.windows..32 {
            assert_eq!(
                d.page.word(
                    t.a(row.classes.caps, "PRECOMP_WIN_PIPE_HDR_CAPC", i)
                        .unwrap()
                ),
                0
            );
        }
    }
}
#[test]
fn host_reply_can_remove_a_derived_feature_without_a_hybrid_page() {
    let t = table();
    let row = &kf_chip::display::BLACKWELL_GB20X;
    let h = HostFacts {
        precomp: Some(
            hostcaps::query(
                t,
                &Regs::for_ip(&t.version, row.ip_version).unwrap(),
                row,
                DIE,
                |a| {
                    let mut v = vec![0; a.len()];
                    v[0] = 1;
                    Ok(v)
                },
            )
            .unwrap(),
        ),
    };
    let d = derive(t, row, DIE, &h);
    for i in 0..row.windows {
        let off = t
            .a(row.classes.caps, "PRECOMP_WIN_PIPE_HDR_CAPC", i)
            .unwrap();
        assert_eq!(
            d.page.word(off),
            derive(t, row, DIE, &HostFacts::default()).page.word(off)
        );
    }
}
#[test]
fn a_snapshot_from_another_die_cannot_override_this_die() {
    let t = table();
    let row = &kf_chip::display::BLACKWELL_GB20X;
    let h = snapshot(
        t,
        row,
        Die {
            implementation: 3,
            ..DIE
        },
    );
    assert!(
        h.precomp
            .as_ref()
            .unwrap()
            .observations_for(t, row, DIE)
            .is_none()
    );
    assert_eq!(
        derive(t, row, DIE, &h).page.words,
        derive(t, row, DIE, &HostFacts::default()).page.words
    );
}
#[test]
fn a_snapshot_from_another_class_cannot_override_this_family() {
    let t = table();
    let row = &kf_chip::display::BLACKWELL_GB20X;
    let h = snapshot(
        t,
        &kf_chip::display::ADA,
        representative(&kf_chip::display::ADA),
    );
    assert!(
        h.precomp
            .as_ref()
            .unwrap()
            .observations_for(t, row, DIE)
            .is_none()
    );
    assert_eq!(
        derive(t, row, DIE, &h).page.words,
        derive(t, row, DIE, &HostFacts::default()).page.words
    );
}
#[test]
fn a_changed_guest_layout_keeps_the_entire_derived_fallback() {
    let t = table();
    let row = &kf_chip::display::BLACKWELL_GB20X;
    let h = snapshot(t, row, DIE);
    let altered = class::ClassTable::parse(&format!(
        "{}\nF\tNVCA73_PRECOMP_WIN_PIPE_HDR_CAPC_CSC00_PRECISION\t9\t5\n",
        include_str!("../data/classes-580.65.06.tsv")
    ));
    assert!(
        h.precomp
            .as_ref()
            .unwrap()
            .observations_for(&altered, row, DIE)
            .is_none()
    );
    assert_eq!(
        derive(&altered, row, DIE, &h).page.words,
        derive(&altered, row, DIE, &HostFacts::default()).page.words
    );
}
fn bad_reply(n: i32) {
    let t = table();
    let row = &kf_chip::display::BLACKWELL_GB20X;
    let called = Cell::new(false);
    let r = hostcaps::query(
        t,
        &Regs::for_ip(&t.version, row.ip_version).unwrap(),
        row,
        DIE,
        |a| {
            called.set(true);
            Ok(vec![1; usize::try_from(a.len() as i32 + n).unwrap()])
        },
    );
    assert!(called.get());
    assert!(r.is_err());
}
#[test]
fn short_reply_abandons_the_whole_snapshot() {
    bad_reply(-1);
}
#[test]
fn long_reply_abandons_the_whole_snapshot() {
    bad_reply(1);
}
#[test]
fn an_all_zero_host_page_is_not_a_claim_of_no_capabilities() {
    let t = table();
    let row = &kf_chip::display::BLACKWELL_GB20X;
    let called = Cell::new(false);
    assert!(
        hostcaps::query(
            t,
            &Regs::for_ip(&t.version, row.ip_version).unwrap(),
            row,
            DIE,
            |a| {
                called.set(true);
                Ok(vec![0; a.len()])
            }
        )
        .is_err()
    );
    assert!(called.get());
}
#[test]
fn an_all_ones_host_page_is_not_published() {
    let t = table();
    let row = &kf_chip::display::BLACKWELL_GB20X;
    let called = Cell::new(false);
    assert!(
        hostcaps::query(
            t,
            &Regs::for_ip(&t.version, row.ip_version).unwrap(),
            row,
            DIE,
            |a| {
                called.set(true);
                Ok(vec![u32::MAX; a.len()])
            }
        )
        .is_err()
    );
    assert!(called.get());
}
#[test]
fn host_caps_are_facts_and_do_not_force_a_fvlut_bit() {
    let t = table();
    let row = &kf_chip::display::BLACKWELL_GB20X;
    let h = snapshot(t, row, DIE);
    let d = derive(t, row, DIE, &h);
    let f = t
        .f(row.classes.caps, "PRECOMP_WIN_PIPE_HDR_CAPC_CSC0LUT_FVLUT")
        .unwrap();
    for i in 0..row.windows {
        let off = t
            .a(row.classes.caps, "PRECOMP_WIN_PIPE_HDR_CAPC", i)
            .unwrap();
        assert_eq!(get(d.page.word(off), f), 0);
    }
}

#[test]
fn a_changed_guest_register_address_uses_the_entire_derived_fallback() {
    let t = table();
    let row = &kf_chip::display::BLACKWELL_GB20X;
    let h = snapshot(t, row, DIE);
    let altered = class::ClassTable::parse(&format!(
        "{}\nA\tNVCA73_PRECOMP_WIN_PIPE_HDR_CAPC\t2944\t32\n",
        include_str!("../data/classes-580.65.06.tsv")
    ));
    assert!(
        h.precomp
            .as_ref()
            .unwrap()
            .observations_for(&altered, row, DIE)
            .is_none()
    );
    assert_eq!(
        derive(&altered, row, DIE, &h).page.words,
        derive(&altered, row, DIE, &HostFacts::default()).page.words
    );
}

#[test]
fn a_snapshot_from_a_different_architecture_is_not_reused() {
    let t = table();
    let row = &kf_chip::display::BLACKWELL_GB20X;
    let h = snapshot(
        t,
        &kf_chip::display::ADA,
        Die {
            architecture: 0x190,
            ..DIE
        },
    );
    assert!(
        h.precomp
            .as_ref()
            .unwrap()
            .observations_for(t, row, DIE)
            .is_none()
    );
    assert_eq!(
        derive(t, row, DIE, &h).page.words,
        derive(t, row, DIE, &HostFacts::default()).page.words
    );
}

fn changed_window_count(windows: u32) {
    let t = table();
    let row = &kf_chip::display::BLACKWELL_GB20X;
    let h = snapshot(t, row, DIE);
    let changed = kf_chip::display::DisplayRow { windows, ..*row };
    assert!(
        h.precomp
            .as_ref()
            .unwrap()
            .observations_for(t, &changed, DIE)
            .is_none()
    );
    assert_eq!(
        derive(t, &changed, DIE, &h).page.words,
        derive(t, &changed, DIE, &HostFacts::default()).page.words
    );
}

#[test]
fn fewer_virtual_windows_do_not_import_hidden_host_windows() {
    changed_window_count(4);
}
#[test]
fn more_virtual_windows_do_not_make_a_partly_host_page() {
    changed_window_count(12);
}

fn representative(row: &kf_chip::display::DisplayRow) -> Die {
    Die {
        architecture: match row.classes.caps {
            0xc573 => 0x160,
            0xc673 => 0x170,
            0xc773 => 0x190,
            0xca73 => 0x1b0,
            _ => panic!("unknown test row"),
        },
        implementation: 2,
    }
}
fn extreme(die: Die) -> HostFacts {
    let t = table();
    let row = &kf_chip::display::BLACKWELL_GB20X;
    HostFacts {
        precomp: Some(
            hostcaps::query(
                t,
                &Regs::for_ip(&t.version, row.ip_version).unwrap(),
                row,
                die,
                |a| Ok(vec![0xffff_fffe; a.len()]),
            )
            .unwrap(),
        ),
    }
}
fn all_fields(window: u32) {
    let t = table();
    let row = &kf_chip::display::BLACKWELL_GB20X;
    for implementation in [2, 3, 5, 6, 7] {
        let die = Die {
            implementation,
            ..DIE
        };
        let d = derive(t, row, die, &extreme(die));
        for line in include_str!(
            "../../../traces/blackwell_host_caps_review_20261022/tier_audit/precomp_fields.tsv"
        )
        .lines()
        .skip(1)
        {
            let f: Vec<_> = line.split('\t').collect();
            let reg = format!("PRECOMP_WIN_PIPE_HDR_{}", f[0]);
            let name = format!("{reg}_{}", f[1]);
            let off = t.a(row.classes.caps, &reg, window).unwrap();
            let bits = t.f(row.classes.caps, &name).unwrap();
            let expected: u32 = f[if window.is_multiple_of(2) { 3 } else { 4 }]
                .parse()
                .unwrap();
            assert_eq!(
                get(d.page.word(off), bits),
                expected,
                "GB20{implementation} {name}"
            );
        }
    }
}
#[test]
fn main_layer_fields_reject_every_host_override() {
    all_fields(0);
}
#[test]
fn overlay_layer_fields_reject_every_host_override() {
    all_fields(7);
}
fn unsupported_rows(window: u32, policy: &str) {
    let t = table();
    let row = &kf_chip::display::BLACKWELL_GB20X;
    let mut rows = capsderive::shipped_rows().to_vec();
    for line in include_str!(
        "../../../traces/blackwell_host_caps_review_20261022/tier_audit/precomp_fields.tsv"
    )
    .lines()
    .skip(1)
    {
        let f: Vec<_> = line.split('\t').collect();
        if f[5] != policy {
            continue;
        }
        rows.push(capsderive::LayerRow {
            overlay: !window.is_multiple_of(2),
            register: format!("PRECOMP_WIN_PIPE_HDR_{}", f[0]),
            field: f[1].into(),
            value: "1".into(),
            optional: false,
            basis: "synthetic unsupported claim".into(),
        });
    }
    let d = capsderive::derive(
        t,
        &Regs::for_ip(&t.version, row.ip_version).unwrap(),
        row,
        DIE,
        true,
        &rows,
        &[],
        "580",
        &HostFacts::default(),
    )
    .unwrap();
    for r in rows
        .iter()
        .filter(|r| r.basis == "synthetic unsupported claim")
    {
        let off = t.a(row.classes.caps, &r.register, window).unwrap();
        let f = t
            .f(row.classes.caps, &format!("{}_{}", r.register, r.field))
            .unwrap();
        assert_eq!(get(d.page.word(off), f), 0, "{}", r.field);
    }
}
#[test]
fn unresolved_main_fields_are_unclaimed_even_if_a_row_attempts_to_enable_them() {
    unsupported_rows(0, "unresolved");
}
#[test]
fn unresolved_overlay_fields_are_unclaimed_even_if_a_row_attempts_to_enable_them() {
    unsupported_rows(7, "unresolved");
}
#[test]
fn unsupported_main_features_stay_unclaimed() {
    unsupported_rows(0, "T1-unclaimed");
}
#[test]
fn unsupported_overlay_features_stay_unclaimed() {
    unsupported_rows(7, "T1-unclaimed");
}
fn inferred(stage: &str) {
    let t = table();
    let row = &kf_chip::display::BLACKWELL_GB20X;
    let d = derive(t, row, DIE, &extreme(DIE));
    let off = t
        .a(row.classes.caps, "PRECOMP_WIN_PIPE_HDR_CAPC", 0)
        .unwrap();
    assert_eq!(
        get(
            d.page.word(off),
            t.f(
                row.classes.caps,
                &format!("PRECOMP_WIN_PIPE_HDR_CAPC_{stage}_PRECISION")
            )
            .unwrap()
        ),
        14
    );
    assert!(
        d.provenance
            .iter()
            .find(|p| p.offset == off)
            .unwrap()
            .why
            .contains("inferred")
    );
}
#[test]
fn csc00_precision_retains_its_inferred_label() {
    inferred("CSC00");
}
#[test]
fn csc01_precision_retains_its_inferred_label() {
    inferred("CSC01");
}
fn unknown(implementation: u32) {
    let t = table();
    let row = &kf_chip::display::BLACKWELL_GB20X;
    let calls = Cell::new(0);
    assert!(
        hostcaps::query(
            t,
            &Regs::for_ip(&t.version, row.ip_version).unwrap(),
            row,
            Die {
                implementation,
                ..DIE
            },
            |a| {
                calls.set(calls.get() + 1);
                Ok(vec![1; a.len()])
            }
        )
        .is_err()
    );
    assert_eq!(calls.get(), 0);
}
#[test]
fn an_unknown_future_die_cannot_borrow_the_blackwell_row() {
    unknown(8);
}
#[test]
fn a_second_unknown_die_is_also_refused_before_io() {
    unknown(9);
}

fn unknown_derive(implementation: u32) {
    let t = table();
    let row = &kf_chip::display::BLACKWELL_GB20X;
    assert!(
        capsderive::derive(
            t,
            &Regs::for_ip(&t.version, row.ip_version).unwrap(),
            row,
            Die {
                implementation,
                ..DIE
            },
            true,
            capsderive::shipped_rows(),
            &[],
            "580",
            &HostFacts::default()
        )
        .is_err()
    );
}
#[test]
fn an_unknown_die_cannot_derive_a_named_family_page() {
    unknown_derive(8);
}
#[test]
fn a_second_unknown_die_cannot_derive_a_named_family_page() {
    unknown_derive(9);
}

#[test]
fn every_virtual_head_keeps_distinct_main_and_overlay_scaler_roles() {
    let t = table();
    let row = &kf_chip::display::BLACKWELL_GB20X;
    let d = derive(t, row, DIE, &extreme(DIE));
    let field = t
        .f(row.classes.caps, "PRECOMP_WIN_PIPE_HDR_CAPA_SCLR_PRESENT")
        .unwrap();
    for window in 0..row.windows {
        let offset = t
            .a(row.classes.caps, "PRECOMP_WIN_PIPE_HDR_CAPA", window)
            .unwrap();
        assert_eq!(
            get(d.page.word(offset), field),
            u32::from(window.is_multiple_of(2)),
            "window {window}"
        );
    }
}
