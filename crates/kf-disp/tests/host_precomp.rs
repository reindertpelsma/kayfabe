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
        assert_eq!(d.page.word(*off), (base + off) ^ 0x1234_0000);
        assert!(
            d.provenance
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
        let h = snapshot(t, row, DIE);
        let d = derive(t, row, DIE, &h);
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
        assert_eq!(d.page.word(off), 0);
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
    assert_eq!(
        derive(t, row, DIE, &h).page.words,
        derive(t, row, DIE, &HostFacts::default()).page.words
    );
}
#[test]
fn a_snapshot_from_another_class_cannot_override_this_family() {
    let t = table();
    let row = &kf_chip::display::BLACKWELL_GB20X;
    let h = snapshot(t, &kf_chip::display::ADA, DIE);
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
        assert_eq!(
            get(d.page.word(off), f),
            get((0x640000 + off) ^ 0x1234_0000, f)
        );
    }
}
