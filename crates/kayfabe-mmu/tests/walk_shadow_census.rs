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

// ═══════════════════════════════════════════════════════════════════════════════════════
// ★★★★★ THE LIVE HALF'S ARMS — a census that only ever DECLINED must say so
// ═══════════════════════════════════════════════════════════════════════════════════════

/// ★★★ **A boot that never compared renders VACUOUS AND NAMES THE REASON.**
///
/// ⊘ This is the arm the live half is most likely to land on, and the one most likely to be
/// misread. The shadow declines on a vCPU thread by design (a synchronous isolate round trip
/// there blocks a vCPU), so a boot whose every sweep ran on a vCPU has **zero disagreements**
/// — and that zero is not agreement. The reason has to be in the line.
#[test]
fn a_census_that_only_declined_is_vacuous_and_says_why() {
    let mut c = ShadowCensus::default();
    for _ in 0..7 {
        c.note_skipped("on_vcpu");
    }
    c.note_skipped("too_many_pages");
    let line = c.render();
    assert!(
        line.contains("VACUOUS"),
        "a census with no comparison must be VACUOUS: {line}"
    );
    assert!(
        line.contains("on_vcpu=7"),
        "the reason must be named and counted: {line}"
    );
    assert!(
        line.contains("too_many_pages=1"),
        "every reason gets its own column: {line}"
    );
    assert!(
        line.contains("kernel_unavailable=8"),
        "a declined sweep is also a sweep on which the two walkers were not compared: {line}"
    );
    assert_eq!(c.total(), 0);
}

/// ⊘ **And the scope caveat is in the line whatever the verdict**, because a reader who only
/// sees `disagreements=0` must still be told what was compared and over what.
#[test]
fn the_line_always_states_its_scope() {
    let empty = ShadowCensus::default().render();
    assert!(empty.contains("compared_flags=0xf"), "{empty}");
    assert!(empty.contains("RELOCATED COPY"), "{empty}");

    let mut c = ShadowCensus::default();
    let leaves: Vec<_> = (0..4)
        .map(|i| leaf(0x1_0000_0000 + i * 4096, 0x20_0000 + i * 4096, 4096))
        .collect();
    let (host, _) = leaves_as_runs(&leaves);
    c.note(&host, &host, 0, &[]);
    let clean = c.render();
    assert!(clean.contains("★★★ AGREEMENT"), "{clean}");
    assert!(clean.contains("compared_flags=0xf"), "{clean}");
    assert!(clean.contains("RELOCATED COPY"), "{clean}");
}

/// ★★ **The image statistics are accumulated and reported**, so the reach clipping and the
/// wire cost are facts a reader can act on rather than numbers nobody kept.
#[test]
fn the_image_statistics_reach_the_line() {
    let mut c = ShadowCensus::default();
    c.note_image(12, 4096 * 13, 5, 2);
    c.note_image(30, 4096 * 31, 1, 0);
    let line = c.render();
    assert!(line.contains("pages_max=30"), "{line}");
    assert!(line.contains(&format!("staged_bytes={}", 4096 * 13 + 4096 * 31)), "{line}");
    assert!(line.contains("absent_edges=6"), "{line}");
    assert!(line.contains("sysmem_edges=2"), "{line}");
}

/// ★★★★★ **THE w731 LIVE DISAGREEMENT, AS A REGRESSION TEST — over the real numbers.**
///
/// `[measured w731, first live boot]` the census came back `compared=65 disagreements=100
/// by_kind[extra_in_kernel=35 len_differs=35 missing_in_kernel=30]`, and the leading pair
/// decoded as **one 4 GiB mapping the kernel had cut in two**:
///
/// ```text
/// host   va=0x120000000 gpga=0x0        len=0x100000000
/// kernel va=0x120000000 gpga=0x0        len=0xefc00000
///      + va=0x20fc00000 gpga=0xefc00000 len=0x10400000
/// ```
///
/// ⊘ Contiguous in VA, contiguous in GPGA, identical flags. This test pins that
/// canonicalisation absorbs exactly that — so a future reader can tell *"the walkers
/// disagree"* apart from *"the comparison put the halves in different sets"*, which is what
/// had actually happened (a proc holds several `Vas` entries sharing one page-directory base;
/// the comparison's unit had been the `Vas` and is now the **root page**).
#[test]
fn the_w731_split_is_absorbed_when_both_halves_are_in_one_set() {
    let host = vec![run(0x1_2000_0000, 0x0, 0x1_0000_0000, PageClass::P2M)];
    let kernel = vec![
        run(0x1_2000_0000, 0x0, 0xefc0_0000, PageClass::P2M),
        run(0x2_0fc0_0000, 0xefc0_0000, 0x1040_0000, PageClass::P2M),
    ];
    let h = kayfabe_mmu::walkdiff::canonical(&host);
    let k = kayfabe_mmu::walkdiff::canonical(&kernel);
    assert_eq!(h, k, "the split and the whole must canonicalise identically");
    assert!(
        compare(&h, &k).is_empty(),
        "★ a run split at a contiguous boundary is NOT a disagreement: {:?}",
        compare(&h, &k)
    );

    // ⊘ And the known-positive for the assertion above: a split whose halves are NOT
    // contiguous in GPGA must still fire, or this test would pass for a comparison that
    // absorbed everything.
    let moved = vec![
        run(0x1_2000_0000, 0x0, 0xefc0_0000, PageClass::P2M),
        run(0x2_0fc0_0000, 0xefc0_0000 + 0x20_0000, 0x1040_0000, PageClass::P2M),
    ];
    let m = kayfabe_mmu::walkdiff::canonical(&moved);
    assert!(
        !compare(&h, &m).is_empty(),
        "a split whose second half points somewhere else MUST disagree"
    );
}
