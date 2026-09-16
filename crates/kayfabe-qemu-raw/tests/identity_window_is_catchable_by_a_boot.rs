//! ★★★★★ **w734 — THE IDENTITY-WINDOW INVARIANT, AND ITS TWO VACUITY ARMS.**
//!
//! `SINGLE_STORE_PLAN.md` §5's expiry note retires **relocation**, **the staged image + its
//! H2D copy** and **`MAX_REFRESHES = 64`** on one property: after §3 the walk kernel's window
//! is the **identity** of the reserved object. That property is destroyed by advertising more
//! framebuffer than was reserved — the guest's own RM places its page tables at the **top** of
//! what it is told it has, so they land outside the object and every kernel dereference is
//! refused by the bounds check.
//!
//! ⊘ `[measured]` the margin is thin **by design**: `span_pages → 3868.7 MiB` inside
//! `RESERVED_MB=4096` is **94.5 %** of what the guest was told. This is not an inequality with
//! a safety factor; it holds because `derived_from_reservation` moves the tables down with the
//! reservation, and it fails the instant anything advertises independently of it.
//!
//! ⇒ *"Make that catchable by a boot, not a comment."* This is the offline half.

use kayfabe_qemu_raw::scratchpad::{identity_window_reached, identity_window_verdict};

const MIB: u64 = 1024 * 1024;

/// ★★★ The measured case, to the byte: 3868.7 MiB of tables inside a 4096 MiB reservation,
/// advertised at exactly what was reserved.
#[test]
fn the_measured_boot_is_possible_and_reached() {
    let reserved = 4096 * MIB;
    let (ok, line) = identity_window_verdict(reserved, reserved);
    assert!(
        ok,
        "advertising exactly what was reserved must be possible: {line}"
    );
    assert!(line.contains("POSSIBLE"), "{line}");

    // 3868.7 MiB, the measured high-water. ⊘ Spelled as MiB arithmetic rather than as a
    // decimal-kB literal: `3_868_700 * 1024` is 3778 MiB, not 3868.7, and writing it that way
    // once already made this assertion measure a different boot than the one it names.
    let span = 3868 * MIB + 7 * MIB / 10;
    let r = identity_window_reached(span, reserved);
    assert!(r.contains("INSIDE"), "{r}");
    assert!(
        r.contains("94.") && r.contains("%"),
        "the line must carry HOW CLOSE it came — 94.5 % is the fact that makes this an \
         invariant to guard rather than a comfortable inequality: {r}"
    );
}

/// ★★★★★ **THE FAILURE THE CHECK EXISTS FOR** — w730's *"the tables live ~11.8 GiB up a
/// 12 GiB board"*: advertising the compiled 12288 MiB while holding a 4096 MiB reservation.
#[test]
fn advertising_more_than_was_reserved_is_impossible_and_says_why() {
    let (ok, line) = identity_window_verdict(12288 * MIB, 4096 * MIB);
    assert!(!ok, "{line}");
    assert!(line.contains("IMPOSSIBLE"), "{line}");
    assert!(
        line.contains("TOP"),
        "the refusal must name the MECHANISM — the guest's RM puts tables at the top of what \
         it is told it has — or it reads as a rounding worry: {line}"
    );
    assert!(
        line.contains("8192") || line.contains("8192.0"),
        "and it must name the overshoot: {line}"
    );
}

/// ⊘ **ONE BYTE OVER IS OVER.** A boundary that is checked with `<` somewhere and `<=`
/// elsewhere is the defect; pinned in both directions.
#[test]
fn the_boundary_is_exact_in_both_directions() {
    let r = 4096 * MIB;
    assert!(identity_window_verdict(r, r).0, "equal fits");
    assert!(
        !identity_window_verdict(r + 1, r).0,
        "one byte over does not"
    );
    assert!(identity_window_reached(r, r).contains("INSIDE"));
    assert!(identity_window_reached(r + 1, r).contains("OUTSIDE"));
}

/// ★★★★★ **NO RESERVATION IS NOT `IMPOSSIBLE`, AND IT MUST NOT READ AS IT.**
///
/// With the scratchpad gate off nothing is held, the fake framebuffer still backs the guest,
/// and the question does not arise. ⊘ A line that said `IMPOSSIBLE` on every default boot is a
/// check that teaches people to ignore it — §w727 item 3's other half, one subsystem over.
#[test]
fn no_reservation_is_the_question_not_arising() {
    let (ok, line) = identity_window_verdict(12288 * MIB, 0);
    assert!(!ok);
    assert!(line.contains("NO RESERVATION"), "{line}");
    assert!(
        !line.contains("IMPOSSIBLE"),
        "a default boot must not be told its design is impossible; it was never asked: {line}"
    );
}

/// ★★★★★ **A ZERO SPAN IS VACUOUS, NEVER A PASS.** *"The guest stayed well inside"* and
/// *"nothing was ever measured"* print the same `0`, and only one of them is a result. This
/// tree's most expensive recurring instrument failure, in the arm that would flatter the
/// design most.
#[test]
fn a_zero_high_water_is_vacuous_and_not_the_best_possible_result() {
    let r = identity_window_reached(0, 4096 * MIB);
    assert!(r.contains("VACUOUS"), "{r}");
    assert!(
        !r.contains("INSIDE"),
        "an unmeasured boot must not report the identity window's success: {r}"
    );
}

/// ⊘ And the mirror image: a span measured with nothing reserved is also not a verdict.
#[test]
fn a_span_with_no_reservation_is_not_a_verdict() {
    let r = identity_window_reached(1024 * MIB, 0);
    assert!(r.contains("NO RESERVATION"), "{r}");
    assert!(!r.contains("INSIDE") && !r.contains("OUTSIDE"), "{r}");
}
