// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! 2026-10-23: production serving, exact coverage, and explicit provisional inheritance.
use kf_chip::DieId;
use kf_disp::capsderive::{self, Derived, Die, HostFacts, Tier};
use kf_disp::regs::Regs;
use kf_rm::consttab::{self, BranchScope, DieScope, Row, SourceTag};
use kf_rm::precomp::{self, Context, INHERITANCE, Inheritance};

fn rows() -> Vec<Row> {
    consttab::table().unwrap().to_vec()
}

fn die(name: &str) -> DieId {
    DieId::from_name(name).unwrap()
}

fn base(row: &kf_chip::display::DisplayRow, id: DieId) -> Derived {
    let table = kf_disp::class::for_version("580.65.06").unwrap();
    capsderive::derive(
        table,
        &Regs::for_ip(&table.version, row.ip_version).unwrap(),
        row,
        Die {
            architecture: id.architecture,
            implementation: id.implementation,
        },
        true,
        capsderive::shipped_rows(),
        capsderive::shipped_measured(),
        "580",
        &HostFacts::default(),
    )
    .unwrap()
}

fn run_with(
    id: DieId,
    host: &str,
    rows: &[Row],
    inheritance: &[Inheritance],
) -> Result<Derived, kf_disp::caps::Missing> {
    let table = kf_disp::class::for_version("580.65.06").unwrap();
    let row = kf_chip::display::display_for(id.architecture, id.implementation)
        .unwrap_or(&kf_chip::display::BLACKWELL_GB20X);
    let regs = Regs::for_ip(&table.version, row.ip_version).unwrap();
    let initial = base(
        row,
        if id.name().is_some() {
            id
        } else {
            die("GB202")
        },
    );
    let context = Context {
        table,
        regs: &regs,
        row,
        die: Die {
            architecture: id.architecture,
            implementation: id.implementation,
        },
        host_driver: host,
    };
    precomp::apply_in(initial, &context, rows, inheritance)
}

fn run(name: &str, rows: &[Row]) -> Result<Derived, kf_disp::caps::Missing> {
    run_with(die(name), "595.91.07", rows, INHERITANCE)
}

fn key(offset: u32) -> String {
    format!("display.precomp.0xca73.{offset:#06x}")
}

fn measured_words(name: &str) {
    let all = rows();
    let page = run(name, &all).unwrap();
    let own: Vec<_> = all
        .iter()
        .filter(|r| r.key.starts_with("display.precomp.") && r.die == DieScope::Die(die(name)))
        .collect();
    assert_eq!(own.len(), 48);
    for row in own {
        let offset = u32::from_str_radix(
            row.key.rsplit('.').next().unwrap().trim_start_matches("0x"),
            16,
        )
        .unwrap();
        assert_eq!(page.page.word(offset), row.words[0], "{name} {offset:#x}");
    }
    let before = base(&kf_chip::display::BLACKWELL_GB20X, die(name));
    let outside = |d: &Derived| {
        d.page
            .words
            .iter()
            .copied()
            .filter(|(o, _)| !(*o >= 0x780 && *o <= 0x87c))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        outside(&page),
        outside(&before),
        "topology and non-PRECOMP facts stay authored"
    );
    assert_eq!(page.unmeasured, before.unmeasured);
}

fn provenance(name: &str) {
    let d = run(name, &rows()).unwrap();
    let measured: Vec<_> = d
        .provenance
        .iter()
        .filter(|p| p.tier == Tier::Measured && p.offset >= 0x780 && p.offset <= 0x87c)
        .collect();
    assert_eq!(measured.len(), 48);
    assert!(measured.iter().all(|p| {
        p.why
            .contains(&format!("measured:{name},595.91.07,2026-10-23"))
            && p.why.contains("evidence=traces/")
            && !p.why.contains("inherited-with-reason")
    }));
}

fn inherited(name: &str) {
    let all = rows();
    let own = run(name, &all).unwrap();
    let donor = run("GB202", &all).unwrap();
    assert_eq!(own.page, donor.page);
    let marked: Vec<_> = own
        .provenance
        .iter()
        .filter(|p| p.why.contains("inherited-with-reason:"))
        .collect();
    assert_eq!(marked.len(), 48);
    assert!(
        marked
            .iter()
            .all(|p| p.why.contains(&format!("{name}<-GB202"))
                && p.why.contains("measured:GB202,595.91.07,2026-10-23")
                && p.why.contains("DISPv0502")
                && p.why.contains("328"))
    );
}

fn partial(name: &str, offset: u32) {
    let mut all = rows();
    all.retain(|r| !(r.die == DieScope::Die(die(name)) && r.key == key(offset)));
    let error = run(name, &all).unwrap_err();
    assert!(error.0.contains("PRECOMP"), "{error:?}");
    assert!(
        error.0.contains(&key(offset)),
        "the precise missing word must be named"
    );
}

// Synthetic prospective measurements exercise precedence, not published data.
fn prospective(target: &str) -> Vec<Row> {
    rows()
        .into_iter()
        .filter_map(|mut r| {
            if !r.key.starts_with("display.precomp.") || r.die != DieScope::Die(die("GB202")) {
                return None;
            }
            r.die = DieScope::Die(die(target));
            r.source = SourceTag::Measured {
                die: target.into(),
                driver: "595.91.07".into(),
                date: "2026-10-23".into(),
            };
            r.evidence = "traces/synthetic_prospective_measurement.txt".into();
            if r.key == key(0x780) {
                r.words[0] ^= 1;
            }
            Some(r)
        })
        .collect()
}

fn exact_wins(name: &str) {
    let mut all = rows();
    let prospective = prospective(name);
    let expected = prospective
        .iter()
        .find(|r| r.key == key(0x780))
        .unwrap()
        .words[0];
    all.extend(prospective);
    let d = run(name, &all).unwrap();
    assert_eq!(d.page.word(0x780), expected);
    assert!(
        !d.provenance
            .iter()
            .any(|p| p.why.contains("inherited-with-reason"))
    );
}

fn partial_own_never_borrows(name: &str) {
    let mut all = rows();
    all.push(prospective(name).remove(0));
    assert!(
        run(name, &all).is_err(),
        "a partial exact measurement cannot borrow donor holes"
    );
}

fn zero_stays_explicit(name: &str) {
    let mut all = rows();
    all.iter_mut()
        .find(|r| r.die == DieScope::Die(die(name)) && r.key == key(0x780))
        .unwrap()
        .words = vec![0];
    let d = run(name, &all).unwrap();
    assert!(d.page.words.contains(&(0x780, 0)));
    assert!(
        d.provenance
            .iter()
            .any(|p| p.offset == 0x780 && p.tier == Tier::Measured)
    );
}

macro_rules! cases {
    ($helper:ident; $($test:ident => $name:literal),+ $(,)?) => {
        $(#[test] fn $test() { $helper($name); })+
    };
}
cases!(measured_words; gb202_real_display_page => "GB202", gb206_real_display_page => "GB206");
cases!(provenance; gb202_provenance => "GB202", gb206_provenance => "GB206");
cases!(inherited; gb203_explicit_inheritance => "GB203", gb205_explicit_inheritance => "GB205",
    gb207_explicit_inheritance => "GB207");
cases!(exact_wins; gb203_exact_precedes_inherited => "GB203", gb205_exact_precedes_inherited => "GB205");
cases!(partial_own_never_borrows; gb203_partial_is_not_patched => "GB203",
    gb205_partial_is_not_patched => "GB205");
cases!(zero_stays_explicit; gb202_zero_is_complete => "GB202", gb206_zero_is_complete => "GB206");

#[test]
fn gb202_missing_first_word_refuses() {
    partial("GB202", 0x780);
}
#[test]
fn gb206_missing_last_word_refuses() {
    partial("GB206", 0x87c);
}
#[test]
fn gb202_wrong_host_branch_refuses() {
    assert!(run_with(die("GB202"), "580.159.04", &rows(), INHERITANCE).is_err());
}
#[test]
fn gb206_wrong_host_branch_refuses() {
    assert!(run_with(die("GB206"), "596.01", &rows(), INHERITANCE).is_err());
}
#[test]
fn gb208_has_no_silent_family_row() {
    assert!(run_with(DieId::new(0x1b0, 8), "595.91.07", &rows(), INHERITANCE).is_err());
}
#[test]
fn gb209_has_no_silent_family_row() {
    assert!(run_with(DieId::new(0x1b0, 9), "595.91.07", &rows(), INHERITANCE).is_err());
}
#[test]
fn ada_is_unchanged() {
    let id = die("AD104");
    assert_eq!(
        run("AD104", &rows()).unwrap(),
        base(&kf_chip::display::ADA, id)
    );
}
#[test]
fn ampere_is_unchanged() {
    let id = die("GA106");
    assert_eq!(
        run("GA106", &rows()).unwrap(),
        base(&kf_chip::display::AMPERE, id)
    );
}
#[test]
fn gb203_empty_inheritance_reason_refuses() {
    let rules = [Inheritance {
        die: "GB203",
        donor: "GB202",
        branch: 595,
        reason: "",
    }];
    assert!(run_with(die("GB203"), "595.91.07", &rows(), &rules).is_err());
}
#[test]
fn gb205_empty_inheritance_reason_refuses() {
    let rules = [Inheritance {
        die: "GB205",
        donor: "GB202",
        branch: 595,
        reason: "",
    }];
    assert!(run_with(die("GB205"), "595.91.07", &rows(), &rules).is_err());
}
#[test]
fn gb203_ada_donor_refuses_even_with_fabricated_rows() {
    let mut all = rows();
    all.extend(prospective("AD104"));
    let rules = [Inheritance {
        die: "GB203",
        donor: "AD104",
        branch: 595,
        reason: "synthetic",
    }];
    assert!(run_with(die("GB203"), "595.91.07", &all, &rules).is_err());
}
#[test]
fn gb205_ampere_donor_refuses_even_with_fabricated_rows() {
    let mut all = rows();
    all.extend(prospective("GA106"));
    let rules = [Inheritance {
        die: "GB205",
        donor: "GA106",
        branch: 595,
        reason: "synthetic",
    }];
    assert!(run_with(die("GB205"), "595.91.07", &all, &rules).is_err());
}
#[test]
fn gb202_bad_word_extent_refuses() {
    let mut all = rows();
    all.iter_mut()
        .find(|r| r.key == key(0x780) && r.die == DieScope::Die(die("GB202")))
        .unwrap()
        .size = 8;
    assert!(run("GB202", &all).is_err());
}
#[test]
fn gb206_bad_word_extent_refuses() {
    let mut all = rows();
    all.iter_mut()
        .find(|r| r.key == key(0x780) && r.die == DieScope::Die(die("GB206")))
        .unwrap()
        .words
        .push(0);
    assert!(run("GB206", &all).is_err());
}
#[test]
fn inherited_scope_is_exactly_the_owner_approved_set() {
    assert_eq!(
        INHERITANCE
            .iter()
            .map(|r| (r.die, r.donor, r.branch))
            .collect::<Vec<_>>(),
        [
            ("GB203", "GB202", 595),
            ("GB205", "GB202", 595),
            ("GB207", "GB202", 595)
        ]
    );
    assert!(
        rows()
            .iter()
            .filter(|r| r.key.starts_with("display.precomp."))
            .all(|r| r.branch == BranchScope::Major(595))
    );
}
