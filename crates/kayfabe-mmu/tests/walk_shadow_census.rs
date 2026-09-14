//! ★★★★★ **THE SHADOW CENSUS, AND THE KNOWN-POSITIVES THAT MAKE ITS ZERO MEAN SOMETHING.**
//!
//! `SINGLE_STORE_PLAN.md` §6 step 1 asks for a live differential between the host walk and the
//! kernel's report, with a census at teardown. The census's headline is a **zero**, and:
//!
//! > ⚠ **A zero disagreement count needs a known-positive.** Perturb one entry deliberately
//! > and confirm the census reports it — a census that has never been shown to fire is this
//! > tree's single most repeated defect, and it has bitten twice in this campaign already.
//!
//! ⇒ Every kind of disagreement is made to fire here, by name, and the two **vacuity** arms
//! are pinned too: a comparison that never ran, and a comparison of two empty sets, must not
//! render as agreement.

use kayfabe_arch::ids::GpuVa;
use kayfabe_arch::{Aperture, PageSize};
use kayfabe_mmu::walkdiff::{PageClass, Run};
use kayfabe_mmu::walker::DecodedLeaf;
use kayfabe_mmu::walkshadow::{
    COMPARED_FLAGS, Disagreement, DisagreementKind, ShadowCensus, compare, leaves_as_runs,
};

const RF_READ_ONLY: u32 = 1 << 3;

fn leaf(va: u64, phys: u64, size: u64) -> DecodedLeaf {
    DecodedLeaf {
        va: GpuVa(va),
        phys,
        aperture: Aperture::Vidmem,
        size: PageSize(size),
        read_only: false,
        level: 4,
    }
}

fn run(va: u64, gpga: u64, len: u64, class: PageClass) -> Run {
    Run {
        va,
        gpga,
        len,
        flags: 0,
        class,
    }
}

/// ⊘ **The control, and it is first.** Identical inputs must produce **no** disagreement — or
/// every assertion below is about a comparison that always fires.
#[test]
fn identical_walks_disagree_about_nothing() {
    let leaves: Vec<_> = (0..8).map(|i| leaf(0x1_0000_0000 + i * 4096, 0x20_0000 + i * 4096, 4096)).collect();
    let (host, dropped) = leaves_as_runs(&leaves);
    assert_eq!(dropped, 0);
    assert!(!host.is_empty(), "the fixture must produce runs, or this proves nothing");
    let d = compare(&host, &host);
    assert!(d.is_empty(), "identical inputs disagreed: {d:?}");
}

/// ★★ **Coalescing must not be a disagreement.** The host emits one run per leaf and the
/// kernel coalesces; if the comparison could not see through that, every boot would report
/// thousands of false differences and the census would be useless.
#[test]
fn a_coalesced_description_equals_a_split_one() {
    let leaves: Vec<_> = (0..8).map(|i| leaf(0x1_0000_0000 + i * 4096, 0x20_0000 + i * 4096, 4096)).collect();
    let (host, _) = leaves_as_runs(&leaves);
    // What a coalescing kernel would emit for the same mappings: one run of eight pages.
    let kernel = vec![run(0x1_0000_0000, 0x20_0000, 8 * 4096, PageClass::P4K)];
    assert!(
        compare(&host, &kernel).is_empty(),
        "one coalesced run must equal eight contiguous leaves — host={host:?}"
    );
}

/// ★★★★★ **THE KNOWN-POSITIVE: every kind fires, by name.**
///
/// ⊘ Not "something was reported" — **which** kind, because the census is read by kind and a
/// count that lands in the wrong column sends a reader to the wrong place.
#[test]
fn every_disagreement_kind_fires_and_is_named() {
    let base: Vec<_> = (0..4).map(|i| leaf(0x1_0000_0000 + i * 4096, 0x20_0000 + i * 4096, 4096)).collect();
    let (host, _) = leaves_as_runs(&base);

    // ── MissingInKernel: the kernel reports nothing at all ──
    let d = compare(&host, &[]);
    assert!(!d.is_empty());
    assert!(
        d.iter().all(|x| x.kind == DisagreementKind::MissingInKernel),
        "a silent kernel must be MISSING, not something else: {d:?}"
    );

    // ── ExtraInKernel: the kernel invents one ──
    let mut k = host.clone();
    k.push(run(0x2_0000_0000, 0x40_0000, 4096, PageClass::P4K));
    let d = compare(&host, &k);
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].kind, DisagreementKind::ExtraInKernel);
    assert_eq!(d[0].va, 0x2_0000_0000);

    // ── GpgaDiffers: ONE entry perturbed, which is what the brief asks for ──
    let mut k = host.clone();
    k[0].gpga ^= 0x1000;
    let d = compare(&host, &k);
    assert_eq!(d.len(), 1, "one perturbed entry, one disagreement: {d:?}");
    assert_eq!(d[0].kind, DisagreementKind::GpgaDiffers);

    // ── LenDiffers ──
    let mut k = host.clone();
    k[0].len = 8192;
    let d = compare(&host, &k);
    assert!(d.iter().any(|x| x.kind == DisagreementKind::LenDiffers), "{d:?}");

    // ── FlagsDiffer: read-only flipped ──
    let mut k = host.clone();
    k[0].flags |= RF_READ_ONLY;
    let d = compare(&host, &k);
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].kind, DisagreementKind::FlagsDiffer);
}

/// ⊘ **A flag only the KERNEL decodes must NOT fire.** `KIND`, volatile, privilege and
/// atomic-disable are outside [`COMPARED_FLAGS`]; comparing them would report a difference
/// between what two decoders LOOKED AT, not between what the guest's tables say — and a boot
/// would drown in false positives.
#[test]
fn a_flag_only_the_kernel_decodes_is_not_a_disagreement() {
    let base: Vec<_> = (0..2).map(|i| leaf(0x1_0000_0000 + i * 4096, 0x20_0000 + i * 4096, 4096)).collect();
    let (host, _) = leaves_as_runs(&base);
    let mut k = host.clone();
    // Bits outside the compared mask: volatile (5), privilege (6), and the KIND field.
    for r in &mut k {
        r.flags |= (1 << 5) | (1 << 6) | (0xAB << 16);
    }
    assert!(
        compare(&host, &k).is_empty(),
        "only {COMPARED_FLAGS:#x} may raise a disagreement"
    );
}

/// ★★★ **The census fires, and its line names the kinds.** A `ShadowCensus` that tallied
/// silently would be a census nobody can act on.
#[test]
fn the_census_counts_by_kind_and_keeps_the_first_few() {
    let base: Vec<_> = (0..4).map(|i| leaf(0x1_0000_0000 + i * 4096, 0x20_0000 + i * 4096, 4096)).collect();
    let (host, dropped) = leaves_as_runs(&base);
    let mut k = host.clone();
    k[0].gpga ^= 0x1000;
    let d = compare(&host, &k);

    let mut c = ShadowCensus::default();
    c.note(&host, &k, dropped, &d);
    assert_eq!(c.total(), 1);
    let line = c.render();
    assert!(line.contains("disagreements=1"), "{line}");
    assert!(line.contains("gpga_differs=1"), "{line}");
    assert!(line.contains("DISAGREEMENTS"), "the verdict must be loud: {line}");
    assert!(
        line.contains(&format!("compared_flags={COMPARED_FLAGS:#x}")),
        "★ the line must state WHICH fields it compared, or a zero reads as more than it is: \
         {line}"
    );
    assert!(!c.first.is_empty(), "the first few must be kept verbatim");
}

/// ⊘⊘ **A CLEAN CENSUS SAYS WHAT IT DOES NOT COVER.** The agreement verdict must name the
/// fields only the kernel decodes, or "zero disagreements" is read as "the two walkers agree
/// about everything" — which is false and is the reading that would license the swap wrongly.
#[test]
fn a_clean_census_still_names_what_it_did_not_compare() {
    let base: Vec<_> = (0..4).map(|i| leaf(0x1_0000_0000 + i * 4096, 0x20_0000 + i * 4096, 4096)).collect();
    let (host, dropped) = leaves_as_runs(&base);
    let mut c = ShadowCensus::default();
    c.note(&host, &host, dropped, &[]);
    let line = c.render();
    assert!(line.contains("AGREEMENT"), "{line}");
    assert!(line.contains("KIND"), "the clean verdict must name KIND: {line}");
    assert!(
        line.contains("Necessary and not sufficient"),
        "the clean verdict must say so in as many words: {line}"
    );
}

/// ★★★★★ **THE TWO VACUITY ARMS.** A zero from a shadow that never ran, and a zero from two
/// empty sets, must not render as agreement — that is the exact defect the brief names.
#[test]
fn a_shadow_that_never_ran_is_vacuous_and_says_so() {
    let c = ShadowCensus::default();
    let line = c.render();
    assert!(line.contains("VACUOUS"), "{line}");
    assert!(
        !line.contains("AGREEMENT"),
        "an unarmed shadow must never render as agreement: {line}"
    );

    let mut c2 = ShadowCensus::default();
    c2.note(&[], &[], 0, &[]);
    let line2 = c2.render();
    assert!(line2.contains("VACUOUS"), "two empty sets are vacuous, not agreeing: {line2}");
}

/// ⊘ A refresh on which the kernel could not run is counted **separately**. Folding it into
/// `compared` would let a boot where the kernel never ran report perfect agreement.
#[test]
fn a_refresh_the_kernel_could_not_run_is_not_an_agreeing_one() {
    let mut c = ShadowCensus::default();
    c.note_unavailable();
    c.note_unavailable();
    let line = c.render();
    assert!(line.contains("kernel_unavailable=2"), "{line}");
    assert!(line.contains("VACUOUS"), "nothing was compared: {line}");
}

/// ⊘ A leaf whose size has no page-size class is **dropped and counted**, never filed under
/// 4 KiB — which would turn a real disagreement into an equal comparison.
#[test]
fn an_unclassed_leaf_is_dropped_and_counted_rather_than_defaulted() {
    let leaves = vec![leaf(0x1_0000_0000, 0x20_0000, 4096), leaf(0x2_0000_0000, 0x40_0000, 1234)];
    let (runs, dropped) = leaves_as_runs(&leaves);
    assert_eq!(dropped, 1, "the odd size must be dropped");
    assert_eq!(runs.len(), 1);
    let mut c = ShadowCensus::default();
    c.note(&runs, &runs, dropped, &[]);
    assert!(
        c.render().contains("host_unclassed=1"),
        "the census must say the comparison lost a member: {}",
        c.render()
    );
}

/// ⊘ Sanity: the disagreement record carries both sides, so a census line is actionable
/// without re-running the boot.
#[test]
fn a_disagreement_carries_both_sides() {
    let base = vec![leaf(0x1_0000_0000, 0x20_0000, 4096)];
    let (host, _) = leaves_as_runs(&base);
    let mut k = host.clone();
    k[0].gpga = 0x99_0000;
    let d: Vec<Disagreement> = compare(&host, &k);
    assert_eq!(d[0].host.0, 0x20_0000);
    assert_eq!(d[0].kernel.0, 0x99_0000);
    assert_eq!(d[0].class, PageClass::P4K);
}

// ═══════════════════════════════════════════════════════════════════════════════════════
// ★★★★★ THE CANONICALISATION, OVER REAL DRIVER TABLES
// ═══════════════════════════════════════════════════════════════════════════════════════

/// Parse `cuda/walk/corpus/*_leaves.txt` — the host walker's committed decode — into leaves,
/// grouped per image.
fn corpus_images(file: &str) -> Vec<(String, Vec<DecodedLeaf>)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("the manifest is two levels below the root")
        .join("cuda/walk/corpus")
        .join(file);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} is part of this repository: {e}", path.display()));
    let hex = |s: &str| u64::from_str_radix(s, 16).expect("a hex field");
    let mut out: Vec<(String, Vec<DecodedLeaf>)> = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("image") => out.push((it.next().unwrap_or("?").to_string(), Vec::new())),
            Some("leaf") => {
                let va = hex(it.next().expect("va"));
                let phys = hex(it.next().expect("phys"));
                let size = hex(it.next().expect("size"));
                let ap = it.next().expect("aperture").parse::<u8>().expect("0..3");
                let ro = it.next().expect("read_only") == "1";
                let aperture = match ap {
                    0 => Aperture::Vidmem,
                    1 => Aperture::Peer,
                    2 => Aperture::SysmemCoherent,
                    _ => Aperture::SysmemNonCoherent,
                };
                out.last_mut()
                    .expect("a leaf before any image line")
                    .1
                    .push(DecodedLeaf {
                        va: GpuVa(va),
                        phys,
                        aperture,
                        size: PageSize(size),
                        read_only: ro,
                        level: 4,
                    });
            }
            _ => {}
        }
    }
    out
}

/// ★★★★★ **THE CANONICAL FORM IS STABLE ON A REAL DRIVER'S TABLES.**
///
/// ⊘ My other fixtures are eight contiguous 4 KiB pages — a shape that cannot expose a
/// coalescing bug, because there is nothing to get wrong. `real_ga106.bin` is `[w725]`'s
/// capture of an actual GA106's page tables: **fragmented, multi-class, with sysmem and
/// vidmem apertures side by side and non-zero `KIND` on 99.7 % of leaves**.
///
/// Three properties, each of which a live shadow depends on:
/// 1. **Idempotence** — `canonical(canonical(x)) == canonical(x)`, so re-canonicalising a
///    description cannot change it. Without this a comparison could disagree with itself.
/// 2. **Self-agreement** — the comparison finds nothing between a description and itself,
///    on real data rather than on a fixture I designed.
/// 3. ★★ **Order-independence** — the same leaves, shuffled, canonicalise identically. The
///    host walk emits depth-first and the kernel emits per-thread; if canonical form depended
///    on emission order, every boot would report false disagreements.
#[test]
fn the_canonical_form_is_stable_on_real_ga106_tables() {
    let images = corpus_images("real_leaves.txt");
    assert!(
        images.len() >= 5,
        "the real-GA106 corpus must have images, or this test is about nothing: {}",
        images.len()
    );
    let total: usize = images.iter().map(|(_, l)| l.len()).sum();
    assert!(
        total >= 1000,
        "the real corpus must be substantial — got {total} leaves; a handful would make the \
         properties below vacuous"
    );

    let mut compared = 0usize;
    for (name, leaves) in &images {
        let (runs, dropped) = leaves_as_runs(leaves);
        assert_eq!(
            dropped, 0,
            "image {name}: a real driver's leaf had no page-size class, so the comparison \
             would silently lose it"
        );
        if runs.is_empty() {
            continue;
        }
        compared += 1;

        // 1. idempotence
        let again = kayfabe_mmu::walkdiff::canonical(&runs);
        assert_eq!(again, runs, "image {name}: canonical form is not idempotent");

        // 2. self-agreement
        assert!(
            compare(&runs, &runs).is_empty(),
            "image {name}: a description disagreed with itself"
        );

        // 3. order-independence — reverse the leaves and canonicalise again
        let mut rev = leaves.clone();
        rev.reverse();
        let (rev_runs, _) = leaves_as_runs(&rev);
        assert_eq!(
            rev_runs, runs,
            "★ image {name}: canonical form depends on EMISSION ORDER. The host walk emits \
             depth-first and the kernel emits per-thread, so this would make every boot \
             report false disagreements."
        );
    }
    assert!(
        compared >= 5,
        "only {compared} images produced runs; the assertions above ran on almost nothing"
    );
}

/// ⊘ The same three properties over the synthetic hostile corpus, which contains shapes a
/// real driver does not produce (cycles, out-of-range pointers, straddling tables). A
/// canonicalisation that only survives well-formed input is one a hostile guest can break.
#[test]
fn the_canonical_form_is_stable_on_the_hostile_corpus_too() {
    let images = corpus_images("rust_leaves.txt");
    assert!(images.len() >= 10, "got {}", images.len());
    let mut nonempty = 0usize;
    for (name, leaves) in &images {
        let (runs, _) = leaves_as_runs(leaves);
        if runs.is_empty() {
            continue;
        }
        nonempty += 1;
        assert_eq!(kayfabe_mmu::walkdiff::canonical(&runs), runs, "image {name}");
        assert!(compare(&runs, &runs).is_empty(), "image {name}");
    }
    assert!(nonempty >= 5, "only {nonempty} hostile images produced runs");
}
