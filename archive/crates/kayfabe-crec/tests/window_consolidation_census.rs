//! ★★★ **How many mappings does a real cold boot's window traffic actually need?** — asked
//! of the oracle rather than estimated.
//!
//! # The question, and why an estimate would not do
//!
//! `kayfabe_vmm_qemu::viewer_install` consolidates adjacent objects into the fewest mappings
//! that cover the coverage, because memslots have a hard ceiling and one per page is not slow
//! but **impossible**. Whether that consolidation is *enough* is an empirical question about
//! the guest's own allocation pattern, and the only honest way to answer it is to count what
//! a real driver did.
//!
//! `cap1_coldboot_hermetic` is 359 062 records of a stock driver cold-booting a real GA106
//! against the C emulator, hermetic (`m2fwd=off m2exec=off m2romregs=off`), so every access
//! in it is the guest's own. This file buckets its framebuffer-window accesses **by host
//! page** and then counts **maximal contiguous runs** of touched pages — which is exactly the
//! consolidation the installer performs, applied to the addresses the guest really used.
//!
//! ## ★★ What the two numbers mean, stated so neither is over-read
//!
//! - **distinct pages touched** — the mapping count of a hypothetical installer that merged
//!   nothing. It is the *upper* bound on objects, since one object may span several pages and
//!   several objects may share one.
//! - **contiguous runs** — the mapping count after consolidation, and therefore the memslot
//!   cost, since a memslot is one contiguous guest-physical range.
//!
//! ⊘ Neither number is the object count. The recorder observes *accesses*, not allocations,
//! so nothing here can say how many objects there were — only how much address space they
//! occupied and how fragmented it was. That is the number the memslot budget cares about,
//! which is why it is the number this file reports; saying it is an object count would be
//! reading it as more than it is.
//!
//! ## ⚠ And the one thing this census cannot settle
//!
//! PRAMIN is a *moving* 1 MiB window: the guest repositions it by writing a base register, so
//! the same window offset names different framebuffer bytes at different times. Bucketing by
//! offset therefore measures the **window's** fragmentation, not the framebuffer's. For the
//! instance window and the framebuffer aperture, which do not move that way, the offset is
//! the address and the count is direct.

use std::collections::BTreeSet;

use kayfabe_crec::format::CKind;
use kayfabe_crec::{cap1_path, load_cap1};
use kayfabe_device::FbWindow;

/// The recorder's PCI slot index → RM's logical index. A 64-bit framebuffer aperture eats
/// two PCI slots, so RM's `BAR2` is PCI `BAR3`.
fn rm_bar_of(rec_bar: u8) -> Option<u8> {
    match rec_bar {
        0 => Some(0),
        1 => Some(1),
        3 => Some(2),
        _ => None,
    }
}

/// Maximal contiguous runs among a sorted set of page numbers — the consolidation, applied.
fn runs(pages: &BTreeSet<u64>) -> usize {
    let mut n = 0usize;
    let mut prev: Option<u64> = None;
    for &p in pages {
        if prev.map(|q| q.saturating_add(1)) != Some(p) {
            n += 1;
        }
        prev = Some(p);
    }
    n
}

/// ★★★ The census. Every number below was produced by this harness against the committed
/// capture before it was written down (2026-07-31); each is exact, so a change in the
/// classifier's extents or in the capture turns it red rather than drifting.
#[test]
fn the_cold_boots_window_traffic_consolidates_into_far_fewer_mappings_than_pages() {
    let trace = match load_cap1() {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => panic!("cap1 at {:?} did not decode: {e:?}", cap1_path()),
        Err(e) => panic!("cap1 is missing at {:?} ({e})", cap1_path()),
    };
    let chip = &kayfabe_device::ga10x::GA106;
    // The host page this project's boxes use. Named rather than queried, because the census
    // is a property of the capture and must not change with the machine reading it.
    const PAGE: u64 = 4096;

    let mut pages: std::collections::BTreeMap<&'static str, BTreeSet<u64>> =
        std::collections::BTreeMap::new();
    let mut accesses: std::collections::BTreeMap<&'static str, u64> =
        std::collections::BTreeMap::new();
    for r in trace.records() {
        if !matches!(r.kind, CKind::MmioWrite | CKind::MmioRead) {
            continue;
        }
        let Some(bar) = rm_bar_of(r.bar) else {
            continue;
        };
        // ★★ The PRODUCTION classifier, not a reimplementation of it.
        let Some(w) = chip.fb_window(bar, r.a) else {
            continue;
        };
        pages.entry(w.name()).or_default().insert(r.a / PAGE);
        *accesses.entry(w.name()).or_default() += 1;
    }

    let inst = pages
        .get(FbWindow::InstanceWindow.name())
        .expect("the instance window is touched during a cold boot");
    let pram = pages
        .get(FbWindow::Pramin.name())
        .expect("PRAMIN is touched during a cold boot");
    let fbap = pages
        .get(FbWindow::FbAperture.name())
        .expect("the framebuffer aperture is touched twice");

    eprintln!(
        "GPGA-CONSOLIDATION cap1_coldboot_hermetic: \
         instance window {} accesses / {} pages / {} runs; \
         PRAMIN {} accesses / {} pages / {} runs; \
         fb aperture {} accesses / {} pages / {} runs",
        accesses[FbWindow::InstanceWindow.name()],
        inst.len(),
        runs(inst),
        accesses[FbWindow::Pramin.name()],
        pram.len(),
        runs(pram),
        accesses[FbWindow::FbAperture.name()],
        fbap.len(),
        runs(fbap),
    );

    // ── The instance window: the bulk of the traffic. ──
    assert_eq!(
        inst.len(),
        87,
        "distinct host pages the instance window touches"
    );
    assert_eq!(
        runs(inst),
        5,
        "★★★ 87 touched pages consolidate into 5 contiguous runs — so the memslot cost of \
         the instance window over a whole cold boot is FIVE, not 87 and not 178 397"
    );

    // ── PRAMIN. ──
    assert_eq!(pram.len(), 20, "distinct host pages PRAMIN touches");
    assert_eq!(
        runs(pram),
        1,
        "★★ every PRAMIN access in the whole cold boot falls in ONE contiguous run of \
         pages — one memslot covers the window's entire used extent"
    );

    // ── The framebuffer aperture: barely used at boot. ──
    assert_eq!(fbap.len(), 1);
    assert_eq!(runs(fbap), 1);

    // ── ★★★ The conclusion, as an assertion rather than a paragraph. ──
    let total_runs = runs(inst) + runs(pram) + runs(fbap);
    let total_pages = inst.len() + pram.len() + fbap.len();
    let total_accesses: u64 = accesses.values().sum();
    assert_eq!(total_pages, 108, "pages touched across all three windows");
    assert_eq!(
        total_accesses, 212_389,
        "★ the traffic this census is over — the same accesses `fb_window_census.rs` \
         buckets, counted here by page instead of by window"
    );
    assert_eq!(
        total_runs,
        7,
        "★★★ ALL THREE WINDOWS of a real cold boot consolidate into SEVEN mappings. The \
         device's memslot budget is {}, so the whole framebuffer-window plane costs {:.1}% \
         of it — the consolidation is not merely helpful, it makes the budget a non-issue",
        kayfabe_vmm_qemu_budget(),
        100.0 * total_runs as f64 / f64::from(kayfabe_vmm_qemu_budget())
    );
    assert!(
        total_accesses > 200_000,
        "★ NON-VACUITY: the census must be over the real traffic. It saw {total_accesses} \
         window accesses; a few hundred would mean the classifier stopped matching and every \
         count above became a count of nothing"
    );
    // ★ And the compression really is enormous — which is the whole argument for the unit
    // being the mapping rather than the page.
    // ★ 212 389 accesses over 7 mappings is ~30 000 accesses per mapping — which is the
    // whole argument for the unit being the mapping rather than the page, in one number.
    assert!(
        total_accesses / total_runs as u64 > 25_000,
        "{total_accesses} accesses over {total_runs} mappings"
    );
}

/// The adapter's memslot budget, retyped here rather than depended on.
///
/// ★ `kayfabe-crec` must not grow a dependency on an adapter crate for one integer, and this
/// test's own suite would not catch a drift — so the number is asserted against the adapter's
/// own constant in `crates/kayfabe-vmm-qemu/tests/viewer_install.rs`, where the dependency
/// already exists, rather than being trusted here.
const fn kayfabe_vmm_qemu_budget() -> u32 {
    64
}
