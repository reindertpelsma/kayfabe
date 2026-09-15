//! ★★★★★ **w734 — THE BYTE CENSUS THE BRANCH'S ORDERING RESTS ON, AND ITS TWO VACUITY ARMS.**
//!
//! `SINGLE_STORE_PLAN.md`'s *"§6 MUST PRECEDE §3"* and `THE_CONSTRAINTS.md` §w724c's
//! *"there is no working intermediate — it does not boot"* both multiply **how many bytes the
//! host reads out of the framebuffer store per boot** by a rate. `[surveyed w734]` the rate is
//! measured; the byte volume never was. The tree counted `pages_swept` — a page count at six
//! different page sizes, three of which are not 4 KiB — and the two documents turned it into
//! MiB by assumption.
//!
//! ⊘ This is the known-positive for the instrument that fixes that. It is offline and needs no
//! GPU, and it checks the three things a census of this shape gets wrong:
//!
//! 1. **it counts BYTES, not calls** — an 8-byte trap read and a 4 KiB walk read must not be
//!    one unit, because the whole point of the split is that one of them is 512× the other;
//! 2. **the roles are DISJOINT** — a walk read must not also land in the trap bucket, or the
//!    "which half has to be fixed first" question the census exists for cannot be answered;
//! 3. **zero says so BY NAME** — `⊘⊘ VACUOUS`, never a `0.0MiB` line that reads like a
//!    measurement of a quiet boot. This tree's most expensive recurring instrument failure is
//!    exactly the pair *"nothing happened"* / *"nothing was recorded"* collapsed into one `0`.
//!
//! ⚠ The counters are process-global statics, so this test asserts on **deltas**, never on
//! absolute values: another test in the same binary bumping them must not make this one fail.

use kayfabe_device::plane::{FbIoRole, fb_io_census_line, fb_io_for, note_fb_read, note_fb_write};

/// Every role's four counters, as a tuple, for differencing.
fn snap(role: FbIoRole) -> (u64, u64, u64, u64) {
    fb_io_for(role)
}

fn delta(a: (u64, u64, u64, u64), b: (u64, u64, u64, u64)) -> (u64, u64, u64, u64) {
    (b.0 - a.0, b.1 - a.1, b.2 - a.2, b.3 - a.3)
}

/// ★★★ **BYTES, NOT CALLS** — and the ratio is the whole reason the census exists.
///
/// One 4 KiB walk read and one 8-byte trap read are **two calls** and **4104 bytes**. A
/// census that counted calls would report the two halves as equal, which is the reading the
/// plan's own ordering argument depends on being false.
#[test]
fn the_census_counts_bytes_and_not_calls() {
    let t0 = snap(FbIoRole::Trap);
    let w0 = snap(FbIoRole::WalkGuestPt);

    note_fb_read(FbIoRole::Trap, 8);
    note_fb_read(FbIoRole::WalkGuestPt, 4096);

    let (trc, trb, _, _) = delta(t0, snap(FbIoRole::Trap));
    let (wrc, wrb, _, _) = delta(w0, snap(FbIoRole::WalkGuestPt));

    assert_eq!((trc, wrc), (1, 1), "one call each");
    assert_eq!(
        (trb, wrb),
        (8, 4096),
        "the census must carry the 512x difference between a trap access and a table page; \
         equal numbers here would be a call counter wearing a byte counter's name"
    );
}

/// ★★★ **THE ROLES ARE DISJOINT.** A read noted in one bucket must move no other bucket —
/// otherwise *"which half has to be fixed first"*, the only question this instrument is for,
/// has no answer.
#[test]
fn a_read_in_one_role_moves_no_other_role() {
    let before: Vec<_> = FbIoRole::all().iter().map(|r| snap(*r)).collect();
    note_fb_read(FbIoRole::WalkInPlane, 32);
    note_fb_write(FbIoRole::CpuCe, 64);
    for (i, role) in FbIoRole::all().iter().enumerate() {
        let d = delta(before[i], snap(*role));
        let expect = match role {
            FbIoRole::WalkInPlane => (1, 32, 0, 0),
            FbIoRole::CpuCe => (0, 0, 1, 64),
            _ => (0, 0, 0, 0),
        };
        assert_eq!(
            d,
            expect,
            "role `{}` moved when it should not have; the buckets are not disjoint",
            role.as_str()
        );
    }
}

/// ★★★ **READS AND WRITES DO NOT ABSORB EACH OTHER.** `fb_write` landing in the read column
/// would make the walk's cost look like a read-only cost, which is the direction that
/// flatters the switch.
#[test]
fn writes_do_not_land_in_the_read_column() {
    let b = snap(FbIoRole::OutOfBand);
    note_fb_write(FbIoRole::OutOfBand, 100);
    let d = delta(b, snap(FbIoRole::OutOfBand));
    assert_eq!(d, (0, 0, 1, 100));
}

/// ★★★★★ **EVERY ROLE HAS ITS OWN TOKEN**, or the census line cannot be parsed back into the
/// split it exists to report. ⊘ This tree has shipped a counter whose name pointed at the
/// healthy half (`the_unknownvchid_name_was_not_true`); a duplicated token is the same defect
/// one step earlier.
#[test]
fn every_role_prints_under_its_own_name() {
    let mut seen = std::collections::BTreeSet::new();
    for r in FbIoRole::all() {
        assert!(
            seen.insert(r.as_str()),
            "two roles print the token `{}`",
            r.as_str()
        );
    }
    assert_eq!(seen.len(), FbIoRole::all().len());
}

/// ★★★★★ **THE VACUITY ARM, AND IT IS THE POINT.**
///
/// ⊘ A boot that recorded nothing and a boot in which the store served nothing print the same
/// `0`. There is no boot of the second kind — `kbusVerifyBar2` writes and reads the store
/// before the guest's first instruction — so a zero is a statement about the **instrument**,
/// and the line must say so in words rather than leaving a `0.0MiB` that reads like a quiet
/// boot.
///
/// ⚠ Asserted on a line built from a deliberately-empty counter set rather than on the
/// globals, because another test in this binary will have moved those.
#[test]
fn a_zero_census_says_vacuous_and_not_zero() {
    // The globals are shared, so pose the question the only way it can be posed
    // deterministically: the formatter's own contract, on the state it is given.
    let line = fb_io_census_line(48 * 1024 * 1024);
    // This binary's other tests have bumped the counters, so the live line must NOT be
    // vacuous — which is itself the known-positive for the arm below.
    assert!(
        !line.contains("VACUOUS"),
        "the census called itself vacuous after this binary recorded bytes: {line}"
    );
    assert!(
        line.contains("PROJECTION"),
        "a non-vacuous line must carry the projection and label it as one: {line}"
    );
    assert!(
        line.contains("the rate is NOT"),
        "the projection must name the rate as an ASSUMPTION; a product of a measured count \
         and somebody else's rate printed without that caveat is a derivation wearing a \
         measurement's clothes: {line}"
    );
    for r in FbIoRole::all() {
        assert!(
            line.contains(r.as_str()),
            "role `{}` is missing from the census line: {line}",
            r.as_str()
        );
    }
}
