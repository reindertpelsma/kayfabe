//! ★ DRIVER-PAIR CONFORMANCE, first slice: what does every measured HOST driver do with every
//! control kayfabe sends it, and does every measured GUEST driver get an ABI table?
//! `STATUS: DESIGN-ONLY slice, 2026-11-20` (`docs/design/V3_DRIVER_PAIR_CONFORMANCE.md`).
//!
//! No GPU, no network, no compiler: the oracle is the committed measurement
//! (`traces/driver_matrix/ranges.tsv`, compiled from ogkm at each tag by `tools/drivermatrix/dm.py`),
//! read through the production tables (`kf_abi::hostabi`, `kf_abi::versions::table_for`). The
//! result is `traces/driver_matrix/pair_table.tsv`: runs of measured host tags with the same
//! verdict, per control. The test FAILS when the committed table is not what the code says now, so
//! an ABI change that makes a pair worse is a red diff, not a silent one.
//!
//! Regenerate: `KF_REGEN_PAIR_TABLE=1 cargo test -p kf-abi --test driver_pair_conformance`.
//!
//! # Verdicts (host axis)
//! - `OK`: the host's layout is the bench's: bytes pass through (or an exact-tag measurement).
//! - `OK(moved ..)`: same fields, other offsets: carried by name.
//! - `DEFAULTED(+f)`: the host has field `f`, the bench does not: zero in requests, dropped from replies.
//! - `CONDITIONAL(ABSENT-ON-HOST -f / REFUSED-IF-FULL a)`: the host lacks bench field `f`, so a request
//!   that sets it is refused and a reply reads 0 for it (a silent default on the consuming side); or an
//!   array is smaller on one side (the number is the smaller capacity) and a full one is refused.
//!   Value-dependent: the all-ones probe shows the worst case, the real request may never reach it.
//! - `REFUSED(reason)`: no encoding at this host; the verb is not sent.
//! - `UNKNOWN(reason)`: the control passes on a contract nobody can derive from ogkm (GSS-legacy ids,
//!   `OWNER_RULINGS` §AH tier 2) at a host tag no probe names. Only hardware can turn it into OK.
//!
//! ⊘ Scope, stated so it is not over-read: layouts cover the fields kayfabe CONSUMES (the spec),
//! not every field of the struct, and nothing here says a carried value MEANS the same thing.

use kf_abi::DriverVersion;
use kf_abi::generated::matrix::MEASURED;
use kf_abi::host_driver::HostDriverVersion;
use kf_abi::hostabi::{Carry, HOST_CONTROLS, HostAbi, HostAbiError, HostControl, HostParams};
use kf_abi::matrix::{Resolved, transcode};
use kf_abi::versions::{BENCH_DRIVER, table_for};
use std::fmt::Write as _;

const COMMITTED: &str = include_str!("../../../traces/driver_matrix/pair_table.tsv");

/// The severity order of a verdict's first token, worst first.
const RANK: [&str; 5] = ["REFUSED", "UNKNOWN", "CONDITIONAL", "DEFAULTED", "OK"];

fn abi_at(v: DriverVersion) -> HostAbi {
    HostAbi::for_host(HostDriverVersion {
        major: v.major,
        minor: v.minor,
        patch: v.patch,
    })
    .expect("a MEASURED tag is a measured host")
}

fn paths(r: &Resolved) -> Vec<&'static str> {
    r.layout.fields.iter().map(|(p, _)| *p).collect()
}

/// The verdict for a struct that crosses by name between the bench and the host layout.
fn carried_verdict(strukt: &'static str, abi: &HostAbi, carry: &Carry) -> String {
    let Carry::Carried { bench, host } = carry else {
        return "OK".to_owned();
    };
    let (bp, hp) = (paths(bench), paths(host));
    let host_only: Vec<&str> = hp.iter().copied().filter(|p| !bp.contains(p)).collect();
    let moved: Vec<String> = bench
        .layout
        .fields
        .iter()
        .filter_map(|(p, f)| {
            host.layout
                .field(p)
                .filter(|g| g != f)
                .map(|_| (*p).to_owned())
        })
        .collect();
    // The request direction: an all-ones body shows which bench fields the host cannot hold.
    let probe = vec![0x01u8; bench.size()];
    let mut cond: Vec<String> = Vec::new();
    match transcode(bench, host, &probe, &[]) {
        Ok((_, dropped)) => cond.extend(
            dropped
                .iter()
                .map(|d| format!("ABSENT-ON-HOST -{d}: request refused if set, reply reads 0")),
        ),
        Err(e) => cond.push(format!("REFUSED-IF-FULL {e}")),
    }
    // A reply that fills the host's struct: the carry back may not fit the bench's arrays.
    let reply = vec![0x01u8; host.size()];
    if let Err(e) = transcode(host, bench, &reply, &[]) {
        cond.push(format!("REFUSED-IF-FULL(reply) {e}"));
    }
    // An all-zero request is the authored default: it must always be expressible.
    if let Err(e) = abi.carry_out(strukt, carry, &vec![0u8; bench.size()]) {
        return format!("REFUSED(zero request: {e})");
    }
    let mut parts = Vec::new();
    if !cond.is_empty() {
        parts.push(format!("CONDITIONAL({})", cond.join("; ")));
    }
    if !host_only.is_empty() {
        parts.push(format!("DEFAULTED(+{})", host_only.join(",+")));
    }
    if parts.is_empty() {
        parts.push(if moved.is_empty() && bench.size() == host.size() {
            "OK".to_owned()
        } else {
            format!(
                "OK(moved {}; size {}->{})",
                moved.join(","),
                bench.size(),
                host.size()
            )
        });
    }
    parts.join(" ")
}

fn refused(e: &HostAbiError) -> String {
    let kind = match e {
        HostAbiError::Unmeasured { .. } => "Unmeasured",
        HostAbiError::Layout(_) => "Layout",
        HostAbiError::Unexpressible { .. } => "Unexpressible",
        HostAbiError::Transcode { .. } => "Transcode",
        HostAbiError::UnlistedControl { .. } => "UnlistedControl",
        HostAbiError::NoHeader { .. } => "NoHeader",
        HostAbiError::Size { .. } => "Size",
    };
    // The first sentence is the reason; the rest of the Display line is advice.
    let text = e.to_string();
    let first = text.split(';').next().unwrap_or(&text);
    format!("REFUSED({kind}: {first})")
}

/// What host `h` does with control `c`: the whole classification, one function.
fn classify_host(h: DriverVersion, c: &HostControl) -> String {
    let abi = abi_at(h);
    let carry = abi.control_carry(c.cmd);
    let exact = |versions: &[DriverVersion]| versions.contains(&h) || h == BENCH_DRIVER;
    match (&c.params, carry) {
        (_, Err(e)) => refused(&e),
        (HostParams::Measured(r), Ok(Some(k))) => carried_verdict(r.name, &abi, &k),
        (HostParams::Renamed { before, .. }, Ok(Some(k))) => carried_verdict(before.name, &abi, &k),
        (HostParams::Measured(_) | HostParams::Renamed { .. }, Ok(None)) => {
            "REFUSED(internal: a measured row passed through without a layout)".to_owned()
        }
        (HostParams::MeasuredOpaque { versions, size, .. }, Ok(_)) => {
            if exact(versions) {
                format!("OK(opaque {size} B, exact tag measured)")
            } else {
                format!(
                    "UNKNOWN(no ogkm oracle for this id; 580-interval contract, {h} not probed)"
                )
            }
        }
        (HostParams::BranchSized { branches, .. }, Ok(_)) => {
            if h == BENCH_DRIVER || branches.iter().any(|b| b.branch.contains(h)) {
                "OK(branch-sized, exact tag measured)".to_owned()
            } else {
                format!(
                    "UNKNOWN(no ogkm oracle for this id; 580-interval contract, {h} not probed)"
                )
            }
        }
        (HostParams::NoHeader { .. }, Ok(_)) => {
            if h == BENCH_DRIVER {
                "OK(no header, bench)".to_owned()
            } else {
                format!(
                    "UNKNOWN(no ogkm oracle for this id; 580-interval contract, {h} not probed)"
                )
            }
        }
    }
}

fn ctl_label(c: &HostControl) -> String {
    if c.name.is_empty() {
        format!("{:#010x}", c.cmd)
    } else {
        format!("{} {:#010x}", c.name, c.cmd)
    }
}

/// Runs of consecutive measured tags with the same verdict, as `item \t first \t last \t verdict`.
fn runs(item: &str, kind: &str, verdicts: &[(DriverVersion, String)], out: &mut String) {
    let mut i = 0;
    while i < verdicts.len() {
        let mut j = i;
        while j + 1 < verdicts.len() && verdicts[j + 1].1 == verdicts[i].1 {
            j += 1;
        }
        let _ = writeln!(
            out,
            "{kind}\t{item}\t{}\t{}\t{}",
            verdicts[i].0, verdicts[j].0, verdicts[i].1
        );
        i = j + 1;
    }
}

fn guest_verdict(v: DriverVersion) -> String {
    match table_for(v) {
        Ok(_) => "OK".to_owned(),
        Err(e) => format!("REFUSED({e})"),
    }
}

fn first_token(verdict: &str) -> &'static str {
    RANK.iter()
        .copied()
        .find(|k| verdict.starts_with(k))
        .unwrap_or_else(|| panic!("verdict {verdict:?} has no known class"))
}

/// The worst class among a set of verdicts.
fn worst(classes: impl Iterator<Item = &'static str>) -> &'static str {
    classes
        .min_by_key(|c| RANK.iter().position(|r| r == c).expect("class"))
        .unwrap_or("OK")
}

fn render() -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# kind\titem\tfirst_host\tlast_host\tverdict   (derived by crates/kf-abi/tests/driver_pair_conformance.rs from \
         traces/driver_matrix/ranges.tsv; regenerate with KF_REGEN_PAIR_TABLE=1; docs/design/V3_DRIVER_PAIR_CONFORMANCE.md)"
    );
    let _ = writeln!(
        out,
        "# tags\t{}",
        MEASURED
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    );
    // Guest axis: does the tag get an ABI table at all?
    let guest: Vec<(DriverVersion, String)> =
        MEASURED.iter().map(|v| (*v, guest_verdict(*v))).collect();
    runs("abi_table", "guest", &guest, &mut out);
    // Host axis, per control.
    let mut host_class: Vec<(DriverVersion, Vec<&'static str>)> =
        MEASURED.iter().map(|v| (*v, Vec::new())).collect();
    for c in HOST_CONTROLS {
        let verdicts: Vec<(DriverVersion, String)> = MEASURED
            .iter()
            .map(|v| (*v, classify_host(*v, c)))
            .collect();
        for ((_, cls), (_, vd)) in host_class.iter_mut().zip(&verdicts) {
            cls.push(first_token(vd));
        }
        runs(&ctl_label(c), "host", &verdicts, &mut out);
    }
    // Per-host roll-up, and the pair roll-up (the axes are independent: kayfabe authors both sides).
    for (v, cls) in &host_class {
        let count = |k: &str| cls.iter().filter(|c| **c == k).count();
        let _ = writeln!(
            out,
            "summary\thost {v}\t{v}\t{v}\tcontrols={} OK={} DEFAULTED={} CONDITIONAL={} UNKNOWN={} REFUSED={} worst={}",
            cls.len(),
            count("OK"),
            count("DEFAULTED"),
            count("CONDITIONAL"),
            count("UNKNOWN"),
            count("REFUSED"),
            worst(cls.iter().copied())
        );
    }
    out
}

/// The staleness check, separated so a test can hand it a mutated file.
fn is_current(committed: &str, fresh: &str) -> bool {
    committed == fresh
}

// -------------------------------------------------------------------------------------------
// the gate
// -------------------------------------------------------------------------------------------

#[test]
fn the_committed_pair_table_is_what_the_code_says_now() {
    let fresh = render();
    if std::env::var_os("KF_REGEN_PAIR_TABLE").is_some_and(|v| v == "1") {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../traces/driver_matrix/pair_table.tsv");
        std::fs::write(&p, &fresh).expect("write pair_table.tsv");
        return;
    }
    assert!(
        is_current(COMMITTED, &fresh),
        "traces/driver_matrix/pair_table.tsv is stale: an ABI table changed. Review the diff \
         (`KF_REGEN_PAIR_TABLE=1 cargo test -p kf-abi --test driver_pair_conformance`, then \
         `git diff traces/driver_matrix/pair_table.tsv`): every changed line is a pair that got better or worse."
    );
}

#[test]
fn a_stale_table_is_detected_both_ways() {
    let fresh = render();
    // A changed verdict and a dropped row are both stale.
    let weakened = fresh.replacen("REFUSED", "OK", 1);
    assert!(
        !is_current(&weakened, &fresh),
        "a weakened verdict went unnoticed"
    );
    let dropped: String = fresh.lines().skip(3).map(|l| format!("{l}\n")).collect();
    assert!(
        !is_current(&dropped, &fresh),
        "a dropped row went unnoticed"
    );
    assert!(is_current(&fresh, &fresh));
}

#[test]
fn an_empty_or_one_byte_different_table_is_stale() {
    let fresh = render();
    assert!(!is_current("", &fresh));
    assert!(!is_current(&format!("{fresh}\n"), &fresh));
    let mut flipped = fresh.clone();
    flipped.replace_range(0..1, "!");
    assert!(!is_current(&flipped, &fresh));
}

// -------------------------------------------------------------------------------------------
// known cells: each pins one verdict class to a fact in the measured matrix
// -------------------------------------------------------------------------------------------

fn tag(s: &str) -> DriverVersion {
    DriverVersion::parse(s).unwrap_or_else(|| panic!("{s}"))
}

fn control(name: &str) -> &'static HostControl {
    HOST_CONTROLS
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("{name} is not a host control"))
}

fn gss(cmd: u32) -> &'static HostControl {
    HOST_CONTROLS
        .iter()
        .find(|c| c.cmd == cmd)
        .unwrap_or_else(|| panic!("{cmd:#x} is not a host control"))
}

/// `NVOS46` is 56 bytes with no `kindOverride` below 580.65.06 (`hostabi.rs` module doc): the bench
/// request may set it, the host cannot be asked: refused only when set.
#[test]
fn a_field_the_host_lacks_is_refused_only_when_a_request_sets_it() {
    let abi = abi_at(tag("575.57.08"));
    let k = abi
        .carry(&kf_abi::generated::matrix::NVOS46_PARAMETERS)
        .expect("carries");
    let v = carried_verdict("NVOS46_PARAMETERS", &abi, &k);
    assert!(v.starts_with("CONDITIONAL(ABSENT-ON-HOST"), "{v}");
    assert!(v.contains("kindOverride"), "{v}");
    // and the bench itself is a pass-through
    let b = abi_at(BENCH_DRIVER);
    let k = b
        .carry(&kf_abi::generated::matrix::NVOS46_PARAMETERS)
        .expect("carries");
    assert_eq!(carried_verdict("NVOS46_PARAMETERS", &b, &k), "OK");
}

/// A field only the host has is defaulted, never silently filled: `hHandleVASpace` arrives at 610.
#[test]
fn a_field_only_the_host_has_is_defaulted() {
    let abi = abi_at(tag("610.43.02"));
    let k = abi
        .carry(&kf_abi::generated::matrix::NV_CHANNEL_ALLOC_PARAMS)
        .expect("carries");
    let v = carried_verdict("NV_CHANNEL_ALLOC_PARAMS", &abi, &k);
    assert!(v.contains("DEFAULTED(+"), "{v}");
    assert!(v.contains("hHandleVASpace"), "{v}");
}

/// A GSS-legacy id has no ogkm name: at a host no probe names it is UNKNOWN, at an exact probed
/// tag it is OK, and outside the 580 interval without a probe it is REFUSED.
#[test]
fn a_gss_legacy_row_is_unknown_unprobed_ok_probed_and_refused_outside() {
    let a028 = gss(0x2080_a028);
    let unprobed = classify_host(tag("580.82.07"), a028);
    assert!(unprobed.starts_with("UNKNOWN("), "{unprobed}");
    let probed = classify_host(tag("595.91.07"), a028);
    assert!(probed.starts_with("OK("), "{probed}");
    let outside = classify_host(tag("575.57.08"), a028);
    assert!(outside.starts_with("REFUSED(NoHeader"), "{outside}");
    // and the bench host is OK by construction
    assert!(classify_host(BENCH_DRIVER, a028).starts_with("OK("));
}

/// A refusal in `control_carry` is a REFUSED cell, never converted to OK by the classifier.
#[test]
fn every_refusal_of_the_production_carry_is_a_refused_cell() {
    for c in HOST_CONTROLS {
        for v in MEASURED {
            let refused_by_code = abi_at(*v).control_carry(c.cmd).is_err();
            let verdict = classify_host(*v, c);
            assert_eq!(
                refused_by_code,
                verdict.starts_with("REFUSED"),
                "{} at {v}: {verdict}",
                ctl_label(c)
            );
        }
    }
}

/// A renamed struct (MSENC caps became NVENC at 610.43.02) is carried, not refused.
#[test]
fn a_renamed_struct_is_carried_across_the_rename() {
    let c = control("NV0080_CTRL_CMD_MSENC_GET_CAPS_V2");
    for t in ["580.159.04", "610.43.02"] {
        let v = classify_host(tag(t), c);
        assert!(!v.starts_with("REFUSED"), "{t}: {v}");
    }
}

/// The two axes are independent: a guest tag's verdict does not depend on the host, and every
/// guest verdict is a class the table knows.
#[test]
fn every_measured_guest_tag_has_a_named_verdict_and_the_bench_is_ok() {
    assert_eq!(guest_verdict(BENCH_DRIVER), "OK");
    for v in MEASURED {
        let g = guest_verdict(*v);
        assert!(g == "OK" || g.starts_with("REFUSED("), "{v}: {g}");
    }
    let unmeasured = guest_verdict(tag("580.159.05"));
    assert!(unmeasured.starts_with("REFUSED("), "{unmeasured}");
}

/// Every host row is classified at every measured tag, and the table says so.
#[test]
fn every_host_control_has_a_cell_at_every_measured_tag() {
    let fresh = render();
    for c in HOST_CONTROLS {
        let label = ctl_label(c);
        let mut seen = 0;
        let mut last_end: Option<DriverVersion> = None;
        for l in fresh.lines().filter(|l| l.starts_with("host\t")) {
            let cols: Vec<&str> = l.split('\t').collect();
            if cols[1] != label {
                continue;
            }
            let (first, last) = (tag(cols[2]), tag(cols[3]));
            let i = MEASURED.binary_search(&first).expect("measured");
            let j = MEASURED.binary_search(&last).expect("measured");
            // runs are contiguous and ordered
            match last_end {
                None => assert_eq!(i, 0, "{label} does not start at the first tag"),
                Some(e) => assert_eq!(MEASURED.binary_search(&e).unwrap() + 1, i, "{label} gap"),
            }
            seen += j - i + 1;
            last_end = Some(last);
        }
        assert_eq!(seen, MEASURED.len(), "{label}");
    }
}
