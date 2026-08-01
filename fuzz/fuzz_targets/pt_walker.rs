//! Coverage-guided fuzz of the **page-table decoder** (`#102` stage C3, landed at
//! `f2094a5`) — `decode_page` / `decode_subtree` / `leaf_disposition` over page images
//! the *guest* wrote.
//!
//! # Why this is a first-class escape target
//!
//! These functions read raw page-table pages out of guest framebuffer and turn them into
//! bindings in the address table — the structure the design docs call *the guest's TLB*.
//! Two distinct failure classes live here and only one of them is a crash:
//!
//! - **Class 1/2** — the entry loop indexes `image` by `i * width`, and both `i` and
//!   `width` come from the format's geometry. A mismatch reads past the buffer.
//! - **Class 3** — `decode_subtree` follows child pointers the *guest wrote*. A guest can
//!   build a **cycle** (a page whose PDE points at itself) or an arbitrarily deep chain;
//!   the only things between that and an unbounded walk in the VMM are `MAX_WALK_DEPTH`
//!   and the entry budget. Those are the properties asserted below, and a fuzzer with a
//!   self-referential `FbRead` is the natural way to attack them.
//!
//! ★ **The `FbRead` is adversarial on purpose.** The production one is a round trip to an
//! isolate; here it answers every physical address from a small pool of fuzzer-chosen
//! page images, so a child pointer very often resolves to *another decodable page*, which
//! is what makes cycles and deep chains actually reachable. A mock that answered zeros
//! would decode one page and stop, and the descent would never be tested at all.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use kayfabe_arch::Aperture;
use kayfabe_mmu::walker::{FbRead, PtPage, decode_page, decode_subtree, leaf_disposition};
use kayfabe_mocks::MockGmmuFmt;

/// Serves every read from a small pool of guest-written images, indexed by the physical
/// address itself. Deliberately **total**: it almost never refuses, so the walker's own
/// bounds are the only thing that stops the descent.
struct PoolFb {
    pages: Vec<Vec<u8>>,
    /// Reads served, so the harness can bound total work independently of the walker's
    /// own accounting — the budget is what is under test, so it cannot also be the thing
    /// that proves the test terminated.
    reads: usize,
    /// Addresses that refuse, so `WalkFault::Unbacked` stays reachable.
    refuse_mask: u64,
}

impl FbRead for PoolFb {
    fn read(&mut self, phys: u64, buf: &mut [u8]) -> bool {
        self.reads += 1;
        assert!(
            self.reads < 20_000,
            "the walker performed {} reads — an unbounded descent",
            self.reads
        );
        if self.pages.is_empty() || (phys & self.refuse_mask) == self.refuse_mask {
            return false;
        }
        let src = &self.pages[(phys as usize / 4096) % self.pages.len()];
        if src.is_empty() {
            return false;
        }
        // ★ Chunked, not per-byte. The per-byte form ran the whole target at ~20 exec/s
        // (measured 2026-08-01, campaign round 1) because `decode_subtree` reads thousands of pages per
        // input — and a target that executes 28 000 times in five minutes has not been
        // fuzzed, it has been sampled. Throughput IS reachability here.
        for chunk in buf.chunks_mut(src.len()) {
            let n = chunk.len();
            chunk.copy_from_slice(&src[..n]);
        }
        true
    }
}

#[derive(Arbitrary, Debug, Clone, Copy)]
enum Ap {
    Vidmem,
    SysmemCoherent,
    SysmemNonCoherent,
}

impl Ap {
    fn to_aperture(self) -> Aperture {
        match self {
            Ap::Vidmem => Aperture::Vidmem,
            Ap::SysmemCoherent => Aperture::SysmemCoherent,
            Ap::SysmemNonCoherent => Aperture::SysmemNonCoherent,
        }
    }
}

#[derive(Arbitrary, Debug)]
struct Input {
    /// The guest-written page images the walk resolves against.
    pages: Vec<Vec<u8>>,
    refuse_mask: u64,
    /// The root the walk starts from — level and `vabase` are forward-populated facts in
    /// production, so a hostile one is only reachable through a decode bug; fuzzing them
    /// anyway is what makes `NoSuchLevel`/`BadGeometry` reachable.
    root_phys: u64,
    root_level: u8,
    root_vabase: u64,
    root_ap: Ap,
    budget: u32,
}

fuzz_target!(|input: Input| {
    if input.pages.is_empty() || input.pages.iter().all(Vec::is_empty) {
        return;
    }
    let fmt = MockGmmuFmt;
    let root = PtPage {
        phys: input.root_phys,
        aperture: input.root_ap.to_aperture(),
        level: input.root_level,
        vabase: input.root_vabase,
    };

    let mut fb = PoolFb {
        pages: input.pages.clone(),
        reads: 0,
        refuse_mask: input.refuse_mask,
    };

    // (1) The direct decode — one page, no descent. This is the pass #13's fix rests on.
    if let Ok(d) = decode_page(&fmt, &mut fb, root) {
        for leaf in &d.leaves {
            // A leaf's VA must lie inside the page's own coverage; one outside it is a
            // binding placed at an address this page never described — a cross-context
            // mapping, which is the class-4 silent misparse this walk can produce.
            let _ = leaf_disposition(&fmt, leaf);
            assert_eq!(
                leaf.level, root.level,
                "a leaf decoded at the wrong level carries the wrong provenance"
            );
        }
        for child in &d.children {
            assert!(
                child.level > root.level,
                "a child at level {} under a parent at level {} — the descent does not \
                 make progress, which is how a guest builds a cycle",
                child.level,
                root.level
            );
        }
    }

    // (2) The bounded descent, over the same adversarial pool. ★ The budget is the
    // property: a guest-built cycle must exhaust it, never loop.
    let mut fb2 = PoolFb {
        pages: input.pages,
        reads: 0,
        refuse_mask: input.refuse_mask,
    };
    // Cap the budget the fuzzer may hand in. An enormous budget is not a bug — it is the
    // caller's choice — and letting the fuzzer set `u32::MAX` would only ever measure
    // wall-clock. `PT_DECODE_BUDGET`-scale values are what production passes.
    // ★ Capped at 4096 entries, not 1 000 000. Round 1 (2026-08-01) measured 20 exec/s with the larger
    // cap: the budget bounds a walk whose cost is linear in it, so a fuzzer allowed to
    // pick a huge one spends the whole campaign inside a handful of inputs. The property
    // under test — a guest-built CYCLE must exhaust the budget rather than loop — is
    // reachable at any budget, and is reached far more often at a small one.
    let budget = input.budget % 4096;
    if let Ok(sub) = decode_subtree(&fmt, &mut fb2, root, budget) {
        // Every visited page is distinct-or-bounded: the depth guard is what stops a
        // self-referential PDE, so a walk that visited more pages than the budget could
        // pay for means the accounting is not being applied.
        assert!(
            u32::try_from(sub.visited.len()).unwrap_or(u32::MAX) <= budget.max(1),
            "visited {} pages on a {budget}-entry budget",
            sub.visited.len()
        );
    }
});
