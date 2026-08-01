//! Coverage-guided fuzz of **`RegionMap::load`** — the guest's own self-describing page
//! table for the GSP shared region — and of every access that resolves through it.
//!
//! # Why this one is on the list
//!
//! It is the tree's clearest instance of finding class 3: the guest declares
//! `pageTableEntryCount` and `sharedMemPageSize`, and the loader does
//! `Vec::with_capacity(entry_count)` and then `vec![0u8; take * 8]` per batch. Only
//! `max_entries` stands between a hostile `u32` and a multi-gigabyte allocation in the
//! VMM. `max_entries` is itself fuzzed here, because a bound that is only ever tested at
//! its production value is tested at one point.
//!
//! ★★ The table is **self-referential** — page `p > 0` of the table is located through an
//! entry the loader already read — so the guest chooses where the loader reads next. That
//! is a guest-directed read loop inside the VMM, and it is the reason this needs a
//! fuzzer and not a unit test: termination is a property of a walk the guest steers.
//!
//! The harness's `GuestRam` is adversarial in the same way `pt_walker`'s `FbRead` is: it
//! answers almost everything, so the walk keeps going and the loader's own bounds are
//! what has to stop it.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use kayfabe_gsp::{GuestRam, RegionMap};

/// Guest RAM that serves reads from a repeating fuzzer-chosen pattern, and counts them.
struct PatternRam {
    pattern: Vec<u8>,
    reads: usize,
    bytes: u64,
}

impl GuestRam for PatternRam {
    fn read(&mut self, gpa: u64, buf: &mut [u8]) -> Result<(), kayfabe_gsp::RamRefused> {
        self.reads += 1;
        self.bytes += buf.len() as u64;
        // ★ The harness's own bound, independent of the loader's. If this fires, the
        // loader performed unbounded work on a guest declaration — the finding.
        assert!(
            self.reads < 100_000 && self.bytes < (1 << 30),
            "RegionMap::load performed {} reads / {} bytes on one declaration",
            self.reads,
            self.bytes
        );
        if self.pattern.is_empty() {
            return Err(kayfabe_gsp::RamRefused {
                gpa,
                len: buf.len(),
                why: "the fuzz harness holds no pattern to serve",
            });
        }
        for (i, b) in buf.iter_mut().enumerate() {
            *b = self.pattern[(gpa as usize).wrapping_add(i) % self.pattern.len()];
        }
        Ok(())
    }

    fn write(&mut self, _gpa: u64, _bytes: &[u8]) -> Result<(), kayfabe_gsp::RamRefused> {
        Ok(())
    }
}

#[derive(Arbitrary, Debug)]
struct Input {
    pattern: Vec<u8>,
    table_base: u64,
    entry_count: u32,
    page_size: u64,
    max_entries: u32,
    /// Accesses to resolve through the loaded map, all with guest-chosen extents.
    accesses: Vec<(u64, u64)>,
    /// A direct `from_pages` construction, so the second constructor is fuzzed too.
    direct_pages: Vec<u64>,
    direct_page_size: u64,
}

fuzz_target!(|input: Input| {
    let mut ram = PatternRam {
        pattern: input.pattern.clone(),
        reads: 0,
        bytes: 0,
    };

    // ★ Cap `max_entries` at the harness, not at the fuzzer's whim: the property under
    // test is "the loader never exceeds the bound it was given", and a fuzzer allowed to
    // pass `u32::MAX` would only measure how long a 32-GiB allocation takes to OOM the
    // fuzzing box — which is a fact about the caller's choice of bound, not about the
    // loader. Production passes a small constant.
    let max_entries = input.max_entries % 4096;

    if let Ok(map) = RegionMap::load(
        &mut ram,
        input.table_base,
        input.entry_count,
        input.page_size,
        max_entries,
    ) {
        // The map's own accounting must agree with the declaration it accepted.
        assert!(
            map.len() <= u64::from(max_entries).saturating_mul(input.page_size),
            "a region longer than the entries it was allowed"
        );
        assert!(!map.is_empty());

        for &(offset, len) in input.accesses.iter().take(32) {
            // ★★ `runs` is where an out-of-region access must be REFUSED rather than
            // clamped — every queue access in the GSP crate resolves through it, so a
            // range that escapes here is an out-of-bounds guest-RAM access performed by
            // the VMM on the guest's instruction (class 1).
            if let Ok(runs) = map.runs(offset, len) {
                let total: u64 = runs.iter().map(|(_, n)| *n as u64).sum();
                assert_eq!(
                    total, len,
                    "runs({offset:#x}, {len}) decomposed into {total} bytes"
                );
                assert!(
                    offset.checked_add(len).is_some_and(|e| e <= map.len()),
                    "runs accepted {offset:#x}+{len} against a {}-byte region",
                    map.len()
                );
            }
            let mut buf = vec![0u8; (len % 4096) as usize];
            let _ = map.read(&mut ram, offset, &mut buf);
            let _ = map.read_u32(&mut ram, offset);
        }
    }

    // The list constructor: same bounds, different entry point, and the one a caller
    // holding its own table uses.
    if let Ok(map) = RegionMap::from_pages(
        input.direct_page_size,
        input.direct_pages.iter().copied().take(4096).collect(),
    ) {
        assert!(!map.is_empty());
        for &(offset, len) in input.accesses.iter().take(16) {
            let _ = map.runs(offset, len);
        }
    }
});
