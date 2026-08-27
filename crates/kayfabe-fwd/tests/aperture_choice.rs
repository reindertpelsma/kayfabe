//! ★★★★★ **THE APERTURE IS WHAT THE GUEST SETTLED** — the owner's ruling, 2026-08-27, as
//! executable statements. `docs/design/copy_placement_policy.md` §1.
//!
//! Pure logic, no GPU: the *choice* of backing arm is a decision in the logic core, and it is
//! the half of the vidmem lane a bench cannot check. What a bench measures is whether the
//! chosen arm is fast; what these check is whether it is the arm the guest asked for.

use kayfabe_arch::Aperture;
use kayfabe_fwd::{DeclaredPlacement as D, FbLeafBacking as B, backing_for, honours_declaration};

/// ★★★★★ **THE WHOLE RULING, AS A TABLE.** Exhaustive over every `DeclaredPlacement`.
///
/// ⊘ Written as a table rather than five `assert_eq!`s so that adding a variant to
/// `DeclaredPlacement` or `Aperture` without deciding its backing is a **compile** failure
/// here, not a silently-unexercised arm.
#[test]
fn every_declaration_gets_the_arm_the_guest_asked_for() {
    let table: &[(D, B, &str)] = &[
        (
            D::Aperture(Aperture::Vidmem),
            B::Vidmem,
            "the guest's PTEs say device memory, so the leaf is device memory",
        ),
        (
            D::Aperture(Aperture::SysmemCoherent),
            B::Joined,
            "declared sysmem stays sysmem",
        ),
        (
            D::Aperture(Aperture::SysmemNonCoherent),
            B::Joined,
            "declared sysmem stays sysmem, coherency notwithstanding",
        ),
        (
            D::Aperture(Aperture::Peer),
            B::Joined,
            "⚠ placeholder — peer memory is on ANOTHER GPU and neither arm expresses it",
        ),
        (
            D::Managed,
            B::Joined,
            "⊘ the UVM exception: residency migrates, so we assert no aperture at all",
        ),
        (
            D::Undeclared,
            B::Joined,
            "⊘ conservative: 'not found' must never become 'assume device memory'",
        ),
    ];
    for (declared, want, why) in table {
        assert_eq!(backing_for(*declared), *want, "{declared:?}: {why}");
    }
}

/// ★★★ **THE CHOICE HONOURS ITSELF** — for every declaration, what `backing_for` picks must
/// satisfy `honours_declaration`.
///
/// ⊘ Not a tautology: the two are written independently — one is a `match` producing an arm,
/// the other a `match` checking a pair — so this catches the two drifting apart, which is
/// exactly what happens when someone edits one arm and not the other.
#[test]
fn the_chosen_arm_always_honours_the_declaration_that_chose_it() {
    for d in [
        D::Aperture(Aperture::Vidmem),
        D::Aperture(Aperture::SysmemCoherent),
        D::Aperture(Aperture::SysmemNonCoherent),
        D::Aperture(Aperture::Peer),
        D::Managed,
        D::Undeclared,
    ] {
        assert!(
            honours_declaration(d, backing_for(d)),
            "{d:?}: backing_for chose an arm its own conformance predicate rejects"
        );
    }
}

/// ★★★★★ **THE PREDICATE CAN SEE A VIOLATION** — the known-positive, and the reason this file
/// is worth more than the two above.
///
/// A conformance check that returns `true` for everything passes both tests above and is
/// worthless. `#12` — the second-context hang that cost a campaign week — **was** an aperture
/// mismatch, so the number that must read zero is only meaningful if a nonzero is reachable
/// at all. These are the two reachable violations.
#[test]
fn a_backing_that_contradicts_the_declaration_is_detected() {
    assert!(
        !honours_declaration(D::Aperture(Aperture::Vidmem), B::Joined),
        "★ the guest declared VIDMEM and we backed sysmem — this is the #12 class and it \
         MUST be visible"
    );
    assert!(
        !honours_declaration(D::Aperture(Aperture::SysmemCoherent), B::Vidmem),
        "★ and the mirror image: declared sysmem, backed device-local. A fabricated aperture \
         in either direction is a defect, not a trade-off"
    );
}

/// ⊘⊘ **THE PREDICATE'S BLIND SPOT, ASSERTED RATHER THAN COMMENTED.**
///
/// `Managed` and `Undeclared` are honoured *vacuously* — neither carries a declaration to
/// contradict. That is correct, and it means a census over `honours_declaration` **cannot**
/// distinguish *"correctly backed"* from *"we never read the declaration"*.
///
/// ⚠ So a decoder that silently stopped decoding — every leaf falling to `Undeclared` — would
/// read as **100 % conformance**. This test exists so that blind spot is a stated property
/// with a name, not a footnote someone discovers after trusting the number. Any census MUST
/// report the `Undeclared` population separately.
#[test]
fn an_undeclared_leaf_is_vacuously_conformant_in_both_arms() {
    for b in [B::Vidmem, B::Joined] {
        assert!(
            honours_declaration(D::Undeclared, b),
            "an undeclared leaf asserted nothing, so nothing can contradict it"
        );
        assert!(
            honours_declaration(D::Managed, b),
            "UVM residency is the host driver's; we assert no aperture to be wrong about"
        );
    }
}

/// ★★★ **UVM NEVER PICKS VIDMEM** — stated on its own because it is the one exception and an
/// exception that quietly stopped applying would be invisible in the table above.
///
/// `mode2_uvm_residency.md`, DECIDED 2026-06-04: a guest managed VA is backed by a host
/// `cudaMallocManaged` allocation and **host UVM owns residency**. Pinning it to `Vidmem`
/// would assert an aperture for memory whose whole nature is that its aperture moves.
#[test]
fn the_uvm_exception_never_asserts_an_aperture() {
    assert_eq!(
        backing_for(D::Managed),
        B::Joined,
        "★ managed memory must NOT be pinned to device memory — residency is the host \
         driver's to move, and claiming an aperture for it is the fabrication this ruling \
         forbids"
    );
}
