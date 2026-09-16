//! ★★★★★ **§6 STEP 2 — THE SWAP'S SUBSTITUTION, CHECKED WITHOUT A GPU.**
//!
//! `walkshadow::substitute` is what puts the walk kernel on the publish path. Its whole
//! safety argument is **totality**: every host leaf gets a kernel-derived value, every kernel
//! byte is used, and any gap refuses by name and falls back. A property like that is either
//! checked here or discovered on a boot, and a boot is a very expensive differ.
//!
//! # ⊘⊘⊘ THE VACUITY THIS FILE HAS TO DEFEAT, STATED FIRST
//!
//! Under agreement the two leaf sets are the **same set**, so a substitution that quietly
//! copied the host leaf through would pass every equality assertion in this file. ⇒ the
//! anti-vacuity test is [`the_value_written_is_computed_from_the_kernel_not_copied_from_the_leaf`]:
//! it hands `substitute` a decode whose leaves carry the WRONG target while the comparison's
//! host side carries the right one, and requires the refusal. A copy-through implementation
//! cannot fail that test, and therefore passing it is what says the value came from the
//! kernel's report.

use kayfabe_arch::ids::GpuVa;
use kayfabe_arch::{Aperture, PageSize};
use kayfabe_mmu::walkdiff::{PageClass, Run};
use kayfabe_mmu::walker::{DecodedLeaf, PageDecode, PtPage, SubtreeDecode};
use kayfabe_mmu::walkshadow::{self, SwapRefusal};

const PAGE: u64 = 4096;

fn leaf(va: u64, phys: u64) -> DecodedLeaf {
    DecodedLeaf {
        va: GpuVa(va),
        phys,
        aperture: Aperture::Vidmem,
        size: PageSize(PAGE),
        read_only: false,
        level: 4,
    }
}

fn page(phys: u64) -> PtPage {
    PtPage {
        phys,
        aperture: Aperture::Vidmem,
        level: 4,
        vabase: 0,
    }
}

/// A decode of one table page holding `n` consecutive 4 KiB leaves from `va0`/`phys0`, plus
/// one child edge, one sparse slot and an invalid count — so the test can assert the page
/// structure is carried through untouched.
fn a_decode(va0: u64, phys0: u64, n: u64) -> SubtreeDecode {
    let leaves: Vec<DecodedLeaf> = (0..n)
        .map(|i| leaf(va0 + i * PAGE, phys0 + i * PAGE))
        .collect();
    let d = PageDecode {
        children: vec![page(0x9000)],
        leaves: leaves.clone(),
        sparse: vec![0xdead_0000],
        invalid: 7,
    };
    SubtreeDecode {
        leaves,
        visited: vec![page(0x8000)],
        decodes: vec![(page(0x8000), d)],
        faults: Vec::new(),
        invalid: 7,
    }
}

/// The comparison's two sides for a decode built by [`a_decode`]: the host's leaves as runs,
/// and one COALESCED kernel run covering all of them — which is the real shape, since the
/// kernel reports runs and the host reports single pages.
fn sides(va0: u64, phys0: u64, n: u64) -> (Vec<Run>, Vec<Run>, usize) {
    let (host, unclassed) = walkshadow::leaves_as_runs(
        &(0..n)
            .map(|i| leaf(va0 + i * PAGE, phys0 + i * PAGE))
            .collect::<Vec<_>>(),
    );
    let kernel = walkshadow::kernel_runs_as_compared(&[Run {
        va: va0,
        gpga: phys0,
        len: n * PAGE,
        flags: 0,
        class: PageClass::P4K,
    }]);
    (host, kernel, unclassed)
}

/// ★★★ **THE HAPPY PATH, AND WHAT IT IS ALLOWED TO CLAIM.** Under agreement the substitution
/// must be TOTAL and must leave the page structure alone. ⊘ It may not claim the values
/// changed — they cannot, and the test that says the value is kernel-derived is below.
#[test]
fn an_agreeing_substitution_is_total_and_leaves_the_page_structure_alone() {
    let d = a_decode(0x1_0000_0000, 0x20_0000, 4);
    let (host, kernel, unclassed) = sides(0x1_0000_0000, 0x20_0000, 4);
    let out = walkshadow::substitute(&d, &host, &kernel, unclassed).expect("agreeing");

    assert_eq!(out.decodes.len(), 1);
    let (p, pd) = &out.decodes[0];
    assert_eq!(*p, page(0x8000), "the page itself is untouched");
    assert_eq!(pd.children, vec![page(0x9000)], "edges are the HOST walk's");
    assert_eq!(pd.sparse, vec![0xdead_0000], "sparse is the HOST walk's");
    assert_eq!(pd.invalid, 7, "the invalid count is the HOST walk's");
    assert_eq!(out.visited, d.visited, "`visited` is the HOST walk's");
    assert_eq!(pd.leaves.len(), 4);
    for (a, b) in pd.leaves.iter().zip(d.decodes[0].1.leaves.iter()) {
        assert_eq!(
            a, b,
            "under agreement the substituted leaf IS the host leaf"
        );
    }
    // ⊘ The flattened view stays consistent with the per-page one, or two consumers of the
    // same decode would disagree about what was found.
    assert_eq!(out.leaves, pd.leaves);
}

/// ★★★★★ **THE ANTI-VACUITY TEST.** See the module header.
///
/// The decode's leaves say the second page targets `0xbad0_0000`; the comparison's host side
/// and the kernel both say `0x20_1000`. A substitution that **computes** from the kernel sees
/// the difference and refuses; one that **copies the leaf through** cannot.
#[test]
fn the_value_written_is_computed_from_the_kernel_not_copied_from_the_leaf() {
    let mut d = a_decode(0x1_0000_0000, 0x20_0000, 4);
    d.decodes[0].1.leaves[1].phys = 0xbad0_0000;
    d.leaves[1].phys = 0xbad0_0000;
    let (host, kernel, unclassed) = sides(0x1_0000_0000, 0x20_0000, 4);
    let e = walkshadow::substitute(&d, &host, &kernel, unclassed).expect_err("must refuse");
    assert_eq!(e.as_str(), "target_changed", "{e:?}");
    assert_eq!(e, SwapRefusal::TargetChanged { va: 0x1_0000_1000 });
}

/// ⊘ **A DISAGREEMENT REFUSES, AND IT IS THE LOUD ONE.** The whole swap rests on this arm:
/// where the two walkers differ the HOST wins and the caller says so.
#[test]
fn a_disagreement_refuses_and_names_how_many() {
    let d = a_decode(0x1_0000_0000, 0x20_0000, 4);
    let (host, _k, unclassed) = sides(0x1_0000_0000, 0x20_0000, 4);
    // The kernel points one page somewhere else ⇒ canonicalisation cannot merge it away.
    let kernel = walkshadow::kernel_runs_as_compared(&[
        Run {
            va: 0x1_0000_0000,
            gpga: 0x20_0000,
            len: PAGE,
            flags: 0,
            class: PageClass::P4K,
        },
        Run {
            va: 0x1_0000_1000,
            gpga: 0x99_0000,
            len: PAGE,
            flags: 0,
            class: PageClass::P4K,
        },
        Run {
            va: 0x1_0000_2000,
            gpga: 0x20_2000,
            len: 2 * PAGE,
            flags: 0,
            class: PageClass::P4K,
        },
    ]);
    let e = walkshadow::substitute(&d, &host, &kernel, unclassed).expect_err("must refuse");
    assert_eq!(e.as_str(), "disagreed", "{e:?}");
    assert_eq!(
        e,
        SwapRefusal::Disagreed(3),
        "one len_differs and two extra_in_kernel"
    );
}

/// ⊘ **A LEAF THE KERNEL DOES NOT COVER REFUSES BY NAME.** Unreachable after an empty
/// comparison — and checked anyway, because an unchecked invariant is not one.
#[test]
fn a_leaf_outside_every_kernel_run_refuses_rather_than_passing_through() {
    let mut d = a_decode(0x1_0000_0000, 0x20_0000, 4);
    // A fifth leaf that is in the DECODE and in neither side of the comparison.
    d.decodes[0].1.leaves.push(leaf(0x2_0000_0000, 0x77_0000));
    d.leaves.push(leaf(0x2_0000_0000, 0x77_0000));
    let (host, kernel, unclassed) = sides(0x1_0000_0000, 0x20_0000, 4);
    let e = walkshadow::substitute(&d, &host, &kernel, unclassed).expect_err("must refuse");
    assert_eq!(e.as_str(), "leaf_uncovered", "{e:?}");
}

/// ⊘ **THE OTHER HALF OF TOTALITY.** Every host leaf having a kernel value does not say every
/// kernel byte was used; a decode that is a strict SUBSET of the compared set must refuse.
#[test]
fn kernel_bytes_the_decode_never_places_refuse() {
    let d = a_decode(0x1_0000_0000, 0x20_0000, 2);
    // Compared over FOUR pages; the decode holds two.
    let (host, kernel, unclassed) = sides(0x1_0000_0000, 0x20_0000, 4);
    let e = walkshadow::substitute(&d, &host, &kernel, unclassed).expect_err("must refuse");
    assert_eq!(e.as_str(), "bytes_unplaced", "{e:?}");
    assert_eq!(
        e,
        SwapRefusal::BytesUnplaced {
            kernel: 4 * PAGE,
            host: 2 * PAGE
        }
    );
}

/// ⊘ **AN UNCLASSED HOST LEAF POISONS THE WHOLE COMPARISON**, so it refuses before anything
/// is written: agreement over a set that lost members is not agreement over the decode.
#[test]
fn an_unclassed_host_leaf_refuses_before_anything_is_written() {
    let d = a_decode(0x1_0000_0000, 0x20_0000, 4);
    let (host, kernel, _u) = sides(0x1_0000_0000, 0x20_0000, 4);
    let e = walkshadow::substitute(&d, &host, &kernel, 1).expect_err("must refuse");
    assert_eq!(e.as_str(), "host_unclassed", "{e:?}");
}

/// ⊘ **AN APERTURE CODE NOTHING DECODES IS REFUSED, NEVER DEFAULTED.** Filing an unknown
/// code under vidmem would bind a system-memory target into the framebuffer's aperture —
/// which is constraint 22's *"sysmem is sysmem"* direction, the one nobody watches.
#[test]
fn an_undecodable_aperture_code_refuses_rather_than_defaulting_to_vidmem() {
    let d = a_decode(0x1_0000_0000, 0x20_0000, 1);
    let (host, _k, unclassed) = sides(0x1_0000_0000, 0x20_0000, 1);
    // Code 5 is inside `RF_AP_MASK` and outside every `Aperture`.
    let kernel = vec![Run {
        va: 0x1_0000_0000,
        gpga: 0x20_0000,
        len: PAGE,
        flags: 5,
        class: PageClass::P4K,
    }];
    let e = walkshadow::substitute(&d, &host, &kernel, unclassed).expect_err("must refuse");
    assert_eq!(e.as_str(), "bad_aperture", "{e:?}");
}

/// ⊘ Every refusal has a distinct census column, or two different walls land in one number.
#[test]
fn every_swap_refusal_has_its_own_name() {
    let names = [
        SwapRefusal::Disagreed(1).as_str(),
        SwapRefusal::HostUnclassed(1).as_str(),
        SwapRefusal::SharedRoot { tasks: 2 }.as_str(),
        SwapRefusal::LeafUncovered { va: 0 }.as_str(),
        SwapRefusal::BytesUnplaced { kernel: 0, host: 0 }.as_str(),
        SwapRefusal::BadAperture { code: 9 }.as_str(),
        SwapRefusal::TargetChanged { va: 0 }.as_str(),
    ];
    let uniq: std::collections::BTreeSet<_> = names.iter().collect();
    assert_eq!(
        uniq.len(),
        names.len(),
        "two refusals share a name: {names:?}"
    );
}

/// ★★★ **THE CENSUS TELLS A SWAP BOOT THAT DECIDED NOTHING FROM A SHADOW BOOT.**
///
/// ⊘ The defect this forecloses: a boot whose shadow agreed 65 times and whose swap decided
/// **zero** times prints `★★★ AGREEMENT` and reads as the swap being proven, while every
/// published leaf came from the host walk exactly as before.
#[test]
fn an_armed_swap_that_decided_nothing_renders_vacuous_beside_an_agreeing_census() {
    let mut c = walkshadow::ShadowCensus::default();
    c.note(&[], &[], 0, &[]);
    c.host_runs = 5;
    let shadow = c.render();
    assert!(shadow.contains("SWAP DISARMED"), "{shadow}");

    c.note_swap_armed();
    let armed = c.render();
    assert!(
        armed.contains("AGREEMENT"),
        "the agreement verdict is unchanged: {armed}"
    );
    assert!(
        armed.contains("SWAP VACUOUS"),
        "an armed swap that decided nothing must say so beside it: {armed}"
    );

    c.note_decided();
    c.note_fell_back("shared_root");
    let live = c.render();
    assert!(live.contains("SWAP LIVE"), "{live}");
    assert!(live.contains("decided=1"), "{live}");
    assert!(live.contains("fell_back[shared_root=1]"), "{live}");
}
