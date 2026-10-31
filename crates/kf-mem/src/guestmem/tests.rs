//! Tests of [`super::RamLayout::locate`] and [`super::translate_blocks`]: a per-byte model oracle
//! (exhaustive on small layouts, seeded-random on larger ones), a second block-walking oracle for
//! huge addresses, the named edge cases, hostile inputs and the fragment cap; then the SEGMENTS:
//! translate through a model hypervisor and compare the bytes the segments reach with the bytes
//! the model says back the range (one block, two blocks, three blocks, a hole, the edges).

use super::testvmm::TestVmm;
use super::*;

fn spec(gpa: u64, len: u64) -> RamBlockSpec {
    RamBlockSpec { gpa, len }
}

fn layout(blocks: &[RamBlockSpec]) -> RamLayout {
    RamLayout::new(blocks).expect("valid layout")
}

/// `(block index, offset in block, len)` of a run.
type Run = (usize, u64, u64);

fn runs(l: &RamLayout, gpa: u64, len: u64) -> Result<Vec<Run>, TranslateError> {
    l.locate(gpa, len)
        .map(|v| v.iter().map(|r| (r.index(), r.off(), r.len())).collect())
}

// ---------------------------------------------------------------- oracles

fn byte_at(blocks: &[RamBlockSpec], gpa: u64) -> Option<(usize, u64)> {
    blocks
        .iter()
        .position(|b| gpa >= b.gpa && gpa - b.gpa < b.len)
        .map(|i| (i, gpa - blocks[i].gpa))
}

/// The model: map EVERY guest-physical byte to `(block, offset)` by linear scan, then run-length
/// encode consecutive bytes of the same block. No shared code with the implementation.
fn oracle_bytes(blocks: &[RamBlockSpec], gpa: u64, len: u64) -> Result<Vec<Run>, TranslateError> {
    if len == 0 {
        return Err(TranslateError::ZeroLength);
    }
    let Some(end) = gpa.checked_add(len) else {
        return Err(TranslateError::Overflow { gpa, len });
    };
    let mut out: Vec<Run> = Vec::new();
    for g in gpa..end {
        let Some((i, o)) = byte_at(blocks, g) else {
            return Err(TranslateError::Unbacked { gpa: g });
        };
        match out.last_mut() {
            Some(l) if l.0 == i && l.1 + l.2 == o => l.2 += 1,
            _ => out.push((i, o, 1)),
        }
    }
    Ok(out)
}

/// A second model for addresses too large to walk byte by byte: repeatedly find the block that
/// holds the next byte by linear scan and take as much of it as the range allows.
fn oracle_blocks(blocks: &[RamBlockSpec], gpa: u64, len: u64) -> Result<Vec<Run>, TranslateError> {
    if len == 0 {
        return Err(TranslateError::ZeroLength);
    }
    let Some(end) = gpa.checked_add(len) else {
        return Err(TranslateError::Overflow { gpa, len });
    };
    let mut out: Vec<Run> = Vec::new();
    let mut at = gpa;
    while at < end {
        let Some(i) = blocks
            .iter()
            .position(|b| at >= b.gpa && at - b.gpa < b.len)
        else {
            return Err(TranslateError::Unbacked { gpa: at });
        };
        let b = blocks[i];
        let take = (b.gpa + b.len).min(end) - at;
        out.push((i, at - b.gpa, take));
        at += take;
    }
    Ok(out)
}

/// Structural invariants of every `Ok` answer.
fn check_ok(out: &[Run], blocks: &[RamBlockSpec], len: u64) {
    assert!(!out.is_empty());
    assert!(out.len() <= MAX_FRAGMENTS);
    let mut sum = 0u64;
    for (k, &(i, o, n)) in out.iter().enumerate() {
        assert!(n > 0, "empty run in {out:?}");
        assert!(
            o.checked_add(n).is_some_and(|e| e <= blocks[i].len),
            "run leaves its block"
        );
        sum = sum.checked_add(n).expect("sum overflows");
        if let Some(&(j, ..)) = out.get(k + 1) {
            assert!(j > i, "runs not in guest-physical order: {out:?}");
        }
    }
    assert_eq!(sum, len);
}

// ------------------------------------------------- oracle comparisons

#[test]
fn oracle_agrees_with_locate_on_every_small_layout_exhaustively() {
    // Per block: gap before it (0..=2) and length (1..=3); 1 to 4 blocks; two starting offsets.
    let mut layouts = 0u64;
    let mut queries = 0u64;
    for n in 1..=4usize {
        let per = 9usize;
        let total = per.pow(u32::try_from(n).unwrap());
        for code in 0..total * 2 {
            let (g0, mut c) = (code % 2, code / 2);
            let mut blocks: Vec<RamBlockSpec> = Vec::new();
            let mut gpa = g0 as u64;
            for i in 0..n {
                let gap = (c % 3) as u64;
                let len = (c / 3 % 3) as u64 + 1;
                c /= per;
                if i > 0 {
                    gpa += gap;
                }
                blocks.push(spec(gpa, len));
                gpa += len;
            }
            let l = RamLayout::new(&blocks).expect("valid");
            layouts += 1;
            for g in 0..=(gpa + 2) {
                for len in 0..=(gpa + 3) {
                    queries += 1;
                    let got = runs(&l, g, len);
                    let want = oracle_bytes(&blocks, g, len);
                    assert_eq!(got, want, "layout {blocks:?} query {g}+{len}");
                    assert_eq!(want, oracle_blocks(&blocks, g, len), "oracles disagree");
                    if let Ok(o) = &got {
                        check_ok(o, &blocks, len);
                    }
                }
            }
        }
    }
    assert!(layouts > 10_000 && queries > 500_000, "{layouts} {queries}");
}

/// xorshift64*: deterministic, no dependency.
pub(super) struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// A random valid layout on an 8-byte "page" grid: tiny blocks, holes, adjacent blocks, unaligned
/// starts.
fn random_layout(r: &mut Rng, base: u64) -> Vec<RamBlockSpec> {
    const P: u64 = 8;
    let n = 1 + r.below(12);
    let mut gpa = base + r.below(3) * P + r.below(P);
    let mut out: Vec<RamBlockSpec> = Vec::new();
    for _ in 0..n {
        let len = (1 + r.below(6)) * P + if r.below(4) == 0 { r.below(P) } else { 0 };
        out.push(spec(gpa, len));
        gpa += len;
        if r.below(3) == 0 {
            gpa += (1 + r.below(4)) * P; // a hole
        }
    }
    out
}

#[test]
fn oracle_agrees_with_locate_on_seeded_random_layouts() {
    let mut r = Rng(0x9E37_79B9_7F4A_7C15);
    let (mut oks, mut errs) = (0u64, 0u64);
    for _ in 0..4000 {
        let blocks = random_layout(&mut r, 0);
        let l = RamLayout::new(&blocks).expect("valid");
        let top = blocks.last().map_or(0, |b| b.gpa + b.len);
        for _ in 0..60 {
            // Queries biased to the interesting places: block edges, +-1, unaligned.
            let pivot = match r.below(3) {
                0 => blocks[r.below(blocks.len() as u64) as usize].gpa,
                1 => {
                    let b = blocks[r.below(blocks.len() as u64) as usize];
                    b.gpa + b.len
                }
                _ => r.below(top + 16),
            };
            let g = pivot.saturating_sub(r.below(5));
            let len = r.below(120);
            let got = runs(&l, g, len);
            assert_eq!(got, oracle_bytes(&blocks, g, len), "{blocks:?} {g}+{len}");
            match &got {
                Ok(o) => {
                    check_ok(o, &blocks, len);
                    oks += 1;
                }
                Err(_) => errs += 1,
            }
        }
    }
    assert!(oks > 20_000 && errs > 20_000, "oks {oks} errs {errs}");
}

#[test]
fn block_walking_oracle_agrees_on_huge_and_near_u64_max_addresses() {
    let mut r = Rng(0xD1B5_4A32_D192_ED03);
    for case in 0..4000u64 {
        // Layouts at the very top of the address space (the last block may end exactly at
        // u64::MAX) and queries with huge lengths.
        let mut blocks = random_layout(&mut r, 0);
        let shift = u64::MAX - (blocks.last().map_or(0, |b| b.gpa + b.len)) - r.below(64);
        for b in &mut blocks {
            b.gpa += shift;
        }
        if case % 2 == 0 {
            // Make the last block end exactly at u64::MAX.
            let last = blocks.last_mut().expect("non-empty");
            last.len = u64::MAX - last.gpa;
        }
        let l = RamLayout::new(&blocks).expect("valid");
        for _ in 0..40 {
            let b = blocks[r.below(blocks.len() as u64) as usize];
            let g = match r.below(4) {
                0 => b.gpa,
                1 => b.gpa + b.len - 1,
                2 => r.next(),
                _ => u64::MAX - r.below(16),
            };
            let len = match r.below(5) {
                0 => u64::MAX,
                1 => u64::MAX - r.below(1024),
                2 => r.next(),
                3 => 1 + r.below(256),
                _ => u64::MAX.saturating_sub(g).saturating_add(r.below(3)),
            };
            let got = runs(&l, g, len);
            assert_eq!(
                got,
                oracle_blocks(&blocks, g, len),
                "{blocks:?} {g:#x}+{len:#x}"
            );
            if let Ok(o) = &got {
                check_ok(o, &blocks, len);
            }
        }
    }
}

// ------------------------------------------------------ named edge cases

/// q35-like: 2 GiB below the PCI hole, 2 GiB above 4 GiB.
fn q35() -> RamLayout {
    layout(&[spec(0, 0x8000_0000), spec(0x1_0000_0000, 0x8000_0000)])
}

#[test]
fn a_range_inside_one_block_is_one_run() {
    assert_eq!(runs(&q35(), 0x1000, 0x2000), Ok(vec![(0, 0x1000, 0x2000)]));
}

#[test]
fn a_range_exactly_one_whole_block() {
    assert_eq!(runs(&q35(), 0, 0x8000_0000), Ok(vec![(0, 0, 0x8000_0000)]));
}

#[test]
fn a_range_starting_at_a_block_start_and_one_ending_exactly_at_a_block_end() {
    let l = q35();
    assert_eq!(runs(&l, 0x1_0000_0000, 16), Ok(vec![(1, 0, 16)]));
    assert_eq!(
        runs(&l, 0x8000_0000 - 16, 16),
        Ok(vec![(0, 0x8000_0000 - 16, 16)])
    );
}

#[test]
fn the_last_byte_of_ram_is_backed_and_the_next_byte_is_named() {
    let l = q35();
    let last = 0x1_8000_0000 - 1;
    assert_eq!(runs(&l, last, 1), Ok(vec![(1, 0x8000_0000 - 1, 1)]));
    assert_eq!(
        runs(&l, last, 2),
        Err(TranslateError::Unbacked { gpa: 0x1_8000_0000 })
    );
    assert_eq!(
        runs(&l, 0x1_8000_0000, 1),
        Err(TranslateError::Unbacked { gpa: 0x1_8000_0000 })
    );
}

#[test]
fn a_range_crossing_a_hole_names_the_first_unbacked_address_not_the_end_of_the_hole() {
    let l = q35();
    // Starts in the first block, runs through the hole into the second.
    assert_eq!(
        runs(&l, 0x7fff_0000, 0x2_0000_0000),
        Err(TranslateError::Unbacked { gpa: 0x8000_0000 })
    );
    // Starts in the hole.
    assert_eq!(
        runs(&l, 0x9000_0000, 0x1000),
        Err(TranslateError::Unbacked { gpa: 0x9000_0000 })
    );
    assert_eq!(
        runs(&l, 0xFFFF_FFFF, 1),
        Err(TranslateError::Unbacked { gpa: 0xFFFF_FFFF })
    );
    // Starts below everything in a layout that begins above zero.
    let high = layout(&[spec(0x1000, 0x1000)]);
    assert_eq!(
        runs(&high, 0, 0x2000),
        Err(TranslateError::Unbacked { gpa: 0 })
    );
    assert_eq!(
        runs(&high, 0xff0, 0x20),
        Err(TranslateError::Unbacked { gpa: 0xff0 })
    );
}

#[test]
fn a_range_starting_past_all_of_ram_and_an_empty_layout() {
    let l = q35();
    assert_eq!(
        runs(&l, 1 << 40, 1),
        Err(TranslateError::Unbacked { gpa: 1 << 40 })
    );
    let empty = RamLayout::new(&[]).expect("empty is a layout");
    assert_eq!(runs(&empty, 0, 1), Err(TranslateError::Unbacked { gpa: 0 }));
    assert_eq!(empty.total_bytes(), 0);
}

#[test]
fn adjacent_guest_blocks_are_two_runs_never_merged() {
    let l = layout(&[spec(0x0, 0x1000), spec(0x1000, 0x1000)]);
    assert_eq!(
        runs(&l, 0xf00, 0x200),
        Ok(vec![(0, 0xf00, 0x100), (1, 0, 0x100)])
    );
    assert_eq!(
        runs(&l, 0, 0x2000),
        Ok(vec![(0, 0, 0x1000), (1, 0, 0x1000)])
    );
}

#[test]
fn a_range_through_three_blocks_gets_the_middle_one_whole() {
    let l = layout(&[
        spec(0x0, 0x1000),
        spec(0x1000, 0x1000),
        spec(0x2000, 0x1000),
    ]);
    assert_eq!(
        runs(&l, 0xfff, 0x1002),
        Ok(vec![(0, 0xfff, 1), (1, 0, 0x1000), (2, 0, 1)])
    );
}

#[test]
fn unaligned_gpa_and_length() {
    let l = layout(&[spec(0x1003, 0x11)]);
    assert_eq!(runs(&l, 0x1005, 3), Ok(vec![(0, 2, 3)]));
    assert_eq!(runs(&l, 0x1003, 0x11), Ok(vec![(0, 0, 0x11)]));
    assert_eq!(
        runs(&l, 0x1003, 0x12),
        Err(TranslateError::Unbacked { gpa: 0x1014 })
    );
    assert_eq!(
        runs(&l, 0x1002, 1),
        Err(TranslateError::Unbacked { gpa: 0x1002 })
    );
}

#[test]
fn zero_length_is_an_error_even_at_a_backed_address() {
    let l = q35();
    assert_eq!(runs(&l, 0x1000, 0), Err(TranslateError::ZeroLength));
    assert_eq!(runs(&l, u64::MAX, 0), Err(TranslateError::ZeroLength));
}

#[test]
fn overflowing_ranges_are_refused_by_name() {
    let l = q35();
    for (g, n) in [
        (u64::MAX, 1),
        (u64::MAX, u64::MAX),
        (1, u64::MAX),
        (u64::MAX - 4, 5),
        (1 << 63, 1 << 63),
    ] {
        assert_eq!(
            runs(&l, g, n),
            Err(TranslateError::Overflow { gpa: g, len: n }),
            "{g:#x}+{n:#x}"
        );
    }
    assert_eq!(
        runs(&l, u64::MAX - 1, 1),
        Err(TranslateError::Unbacked { gpa: u64::MAX - 1 })
    );
}

#[test]
fn huge_lengths_are_unbacked_not_allocated() {
    let l = q35();
    assert_eq!(
        runs(&l, 0, u64::MAX),
        Err(TranslateError::Unbacked { gpa: 0x8000_0000 })
    );
    assert_eq!(
        runs(&l, 0x1_0000_0000, u64::MAX - 0x1_0000_0000),
        Err(TranslateError::Unbacked { gpa: 0x1_8000_0000 })
    );
}

#[test]
fn a_block_ending_exactly_at_u64_max_translates_up_to_its_last_byte() {
    let l = layout(&[spec(u64::MAX - 0xfff, 0xfff)]);
    assert_eq!(runs(&l, u64::MAX - 0xfff, 0xfff), Ok(vec![(0, 0, 0xfff)]));
    // One byte more would end at 2^64: an overflow (the exclusive end is not representable).
    assert_eq!(
        runs(&l, u64::MAX - 0xfff, 0x1000),
        Err(TranslateError::Overflow {
            gpa: u64::MAX - 0xfff,
            len: 0x1000
        })
    );
}

// ----------------------------------------------------- layout validation

#[test]
fn layout_validation_refuses_by_name() {
    assert_eq!(
        RamLayout::new(&[spec(0, 0)]),
        Err(LayoutError::ZeroLength { index: 0 })
    );
    assert_eq!(
        RamLayout::new(&[spec(0, 8), spec(u64::MAX, 2)]),
        Err(LayoutError::GpaOverflow { index: 1 })
    );
    assert_eq!(
        RamLayout::new(&[spec(0x2000, 8), spec(0x1000, 8)]),
        Err(LayoutError::Unsorted { index: 1 })
    );
    assert_eq!(
        RamLayout::new(&[spec(0, 0x1001), spec(0x1000, 8)]),
        Err(LayoutError::Overlap { index: 1 })
    );
    // Same start is an overlap, not "unsorted".
    assert_eq!(
        RamLayout::new(&[spec(0, 8), spec(0, 8)]),
        Err(LayoutError::Overlap { index: 1 })
    );
    let many: Vec<RamBlockSpec> = (0..=MAX_LAYOUT_BLOCKS as u64)
        .map(|i| spec(i * 2, 1))
        .collect();
    assert_eq!(RamLayout::new(&many), Err(LayoutError::TooManyBlocks));
    let ok: Vec<RamBlockSpec> = (0..MAX_LAYOUT_BLOCKS as u64)
        .map(|i| spec(i * 2, 1))
        .collect();
    assert!(RamLayout::new(&ok).is_ok());
}

#[test]
fn layout_errors_and_translate_errors_have_names() {
    assert!(
        LayoutError::Overlap { index: 3 }
            .to_string()
            .contains("RAM block 3")
    );
    assert!(
        TranslateError::Unbacked { gpa: 0x8000_0000 }
            .to_string()
            .contains("0x80000000")
    );
    assert!(
        TranslateError::Overflow { gpa: 1, len: 2 }
            .to_string()
            .contains("overflows")
    );
    assert!(
        TranslateError::LayoutChanged
            .to_string()
            .starts_with("LAYOUT_CHANGED")
    );
}

// -------------------------------------------------------- fragment cap

/// `n` one-page blocks, adjacent in guest-physical space.
fn fragmented(n: u64) -> RamLayout {
    let blocks: Vec<RamBlockSpec> = (0..n).map(|i| spec(i * 0x1000, 0x1000)).collect();
    layout(&blocks)
}

#[test]
fn exactly_the_cap_is_served_and_one_more_is_refused() {
    let l = fragmented(MAX_FRAGMENTS as u64 + 1);
    let n = MAX_FRAGMENTS as u64;
    let ok = l.locate(0, n * 0x1000).expect("exactly the cap");
    assert_eq!(ok.len(), MAX_FRAGMENTS);
    assert_eq!(
        l.locate(0, (n + 1) * 0x1000).map(|v| v.len()),
        Err(TranslateError::TooFragmented)
    );
    // The cap counts blocks needed by THIS range, not the layout: a range deep in it is fine.
    assert_eq!(l.locate(1 << 12, 0x1000).expect("one run").len(), 1);
}

#[test]
fn a_refused_cap_returns_no_partial_answer_and_bounded_memory() {
    let l = fragmented(MAX_LAYOUT_BLOCKS as u64);
    let r = l.locate(0, MAX_LAYOUT_BLOCKS as u64 * 0x1000);
    assert_eq!(r.map(|v| v.len()), Err(TranslateError::TooFragmented));
}

// ----------------------------------------------------- hostile input

#[test]
fn hostile_inputs_never_panic_overflow_or_exceed_the_bound() {
    let layouts = [
        q35(),
        fragmented(MAX_FRAGMENTS as u64 + 5),
        RamLayout::new(&[]).expect("empty"),
        layout(&[spec(u64::MAX - 0xfff, 0xfff)]),
        layout(&[spec(0, u64::MAX)]),
    ];
    let vals = [
        0u64,
        1,
        2,
        0xfff,
        0x1000,
        0x7fff_ffff,
        0x8000_0000,
        0xffff_ffff,
        0x1_0000_0000,
        1 << 47,
        1 << 62,
        1 << 63,
        (1 << 63) + 1,
        u64::MAX / 2,
        u64::MAX - 0xfff,
        u64::MAX - 1,
        u64::MAX,
    ];
    let mut r = Rng(0x1234_5678_9ABC_DEF1);
    let mut cases: Vec<(u64, u64)> = Vec::new();
    for &g in &vals {
        for &n in &vals {
            cases.push((g, n));
        }
    }
    for _ in 0..20_000 {
        cases.push((r.next(), r.next()));
    }
    for l in &layouts {
        for &(g, n) in &cases {
            match l.locate(g, n) {
                Ok(o) => {
                    assert!(!o.is_empty() && o.len() <= MAX_FRAGMENTS);
                    assert!(
                        o.capacity() <= 2 * MAX_FRAGMENTS,
                        "capacity {}",
                        o.capacity()
                    );
                    let sum: u64 = o.iter().map(BlockRun::len).sum();
                    assert_eq!(sum, n);
                    for run in &o {
                        let b = l.blocks()[run.index()];
                        assert!(run.off().checked_add(run.len()).is_some_and(|e| e <= b.len));
                    }
                }
                Err(TranslateError::Overflow { gpa, len }) => {
                    assert_eq!((gpa, len), (g, n));
                    assert!(g.checked_add(n).is_none());
                }
                Err(TranslateError::Unbacked { gpa }) => {
                    assert!(gpa >= g && gpa - g < n, "named address outside the range");
                }
                Err(TranslateError::ZeroLength) => assert_eq!(n, 0),
                Err(TranslateError::TooFragmented) => {}
                Err(e) => panic!("unexpected {e:?}"),
            }
        }
    }
}

#[test]
fn the_unbacked_address_is_the_first_one_for_every_prefix() {
    let blocks = [spec(0x100, 0x40), spec(0x180, 0x40)];
    let l = layout(&blocks);
    for g in 0x80..0x1e0u64 {
        for n in 1..0x120u64 {
            let want = (g..g + n).find(|&x| byte_at(&blocks, x).is_none());
            match (runs(&l, g, n), want) {
                (Ok(_), None) => {}
                (Err(TranslateError::Unbacked { gpa }), Some(w)) => assert_eq!(gpa, w),
                (got, want) => panic!("{g:#x}+{n:#x}: {got:?} vs first unbacked {want:?}"),
            }
        }
    }
}

// ---------------------------------------------------------- the SEGMENTS

/// The byte the model VMM puts at block-relative offset `i` of the block at `gpa`.
fn pattern(gpa: u64, i: usize) -> u8 {
    (gpa as usize / 0x100 + i * 7 + 1) as u8
}

fn vmm_with(blocks: &[(u64, u64)]) -> TestVmm {
    let v = TestVmm::new();
    for &(gpa, len) in blocks {
        v.add(gpa, len);
        v.fill(gpa, |i| pattern(gpa, i));
    }
    v
}

/// What the segments reach equals what the model says is at those guest-physical addresses.
fn assert_reads_match(vmm: &TestVmm, gpa: u64, len: u64) {
    let guard = vmm.lock_layout().expect("layout");
    let t = guard.translate(gpa, len).expect("translate");
    assert_eq!(t.total_len(), len);
    assert_eq!(t.epoch(), guard.epoch());
    let mut got = vec![0u8; len as usize];
    t.read_into(&mut got).expect("read");
    for (k, b) in got.iter().enumerate() {
        assert_eq!(
            Some(*b),
            vmm.peek(gpa + k as u64),
            "byte {k} of {gpa:#x}+{len:#x}"
        );
    }
}

#[test]
fn segments_reach_the_bytes_the_model_names_across_one_two_and_three_blocks() {
    let vmm = vmm_with(&[
        (0x1000, 0x1000),
        (0x2000, 0x800),
        (0x2800, 0x1000),
        (0x8000, 0x400),
    ]);
    // One block.
    assert_reads_match(&vmm, 0x1100, 0x40);
    // Whole first block; edges.
    assert_reads_match(&vmm, 0x1000, 0x1000);
    assert_reads_match(&vmm, 0x1fff, 1);
    // Two blocks (0x1000 | 0x2000).
    assert_reads_match(&vmm, 0x1ff0, 0x20);
    // Three blocks.
    assert_reads_match(&vmm, 0x1ff0, 0x1800);
    // The very last byte of RAM.
    assert_reads_match(&vmm, 0x83ff, 1);
    // Segments: one per block touched, never merged.
    let g = vmm.lock_layout().expect("layout");
    let t = g.translate(0x1ff0, 0x1800).expect("three");
    let lens: Vec<u64> = t.segments().iter().map(|s| s.len()).collect();
    assert_eq!(lens, vec![0x10, 0x800, 0xff0]);
}

#[test]
fn a_hole_names_the_first_unbacked_byte_and_returns_no_segment() {
    let vmm = vmm_with(&[(0x1000, 0x1000), (0x3000, 0x1000)]);
    let g = vmm.lock_layout().expect("layout");
    assert_eq!(
        g.translate(0x1ff0, 0x1000).map(|t| t.total_len()),
        Err(TranslateError::Unbacked { gpa: 0x2000 })
    );
    assert_eq!(
        g.translate(0, 8).map(|t| t.total_len()),
        Err(TranslateError::Unbacked { gpa: 0 })
    );
}

#[test]
fn hostile_u64_max_requests_through_the_guard_are_errors_never_segments() {
    let vmm = vmm_with(&[(0x1000, 0x1000), (0x2000, 0x1000)]);
    let g = vmm.lock_layout().expect("layout");
    for (gpa, len) in [
        (u64::MAX, u64::MAX),
        (u64::MAX, 1),
        (1, u64::MAX),
        (0x1000, u64::MAX),
        (u64::MAX - 1, 2),
        (0x1000, 0),
        (0x2fff, 2),
    ] {
        assert!(g.translate(gpa, len).is_err(), "{gpa:#x}+{len:#x}");
    }
    // The only OK answers in that list were refused; the backed range still works.
    assert_eq!(
        g.translate(0x1000, 0x2000).expect("both").segments().len(),
        2
    );
}

#[test]
fn writing_through_segments_lands_in_the_model_memory_across_a_block_edge() {
    let vmm = vmm_with(&[(0x1000, 0x100), (0x1100, 0x100)]);
    let g = vmm.lock_layout().expect("layout");
    let t = g.translate(0x10f8, 16).expect("two");
    t.write_from(&[0xAB; 16]).expect("write");
    for a in 0x10f8..0x1108u64 {
        assert_eq!(vmm.peek(a), Some(0xAB), "{a:#x}");
    }
    assert_eq!(vmm.peek(0x10f7), Some(pattern(0x1000, 0xf7)));
    assert_eq!(vmm.peek(0x1108), Some(pattern(0x1100, 8)));
    // A length that is not the range's length is refused, nothing is written.
    assert!(t.write_from(&[0u8; 15]).is_err());
    let mut small = [0u8; 4];
    assert!(t.read_into(&mut small).is_err());
}

#[test]
fn a_segment_cannot_be_narrowed_out_of_bounds_and_the_adapter_cannot_lie() {
    use kf_linux_raw::HostOffset;
    let vmm = vmm_with(&[(0x1000, 0x100)]);
    let g = vmm.lock_layout().expect("layout");
    let t = g.translate(0x1010, 0x20).expect("one");
    let seg = &t.segments()[0];
    assert_eq!(seg.len(), 0x20);
    for (o, l) in [
        (0u64, 0x21u64),
        (0x20, 1),
        (u64::MAX, 2),
        (1, u64::MAX),
        (0, 0),
    ] {
        assert!(seg.slice(HostOffset::new(o), l).is_err(), "{o:#x}+{l:#x}");
    }
    assert!(seg.slice(HostOffset::new(0x10), 0x10).is_ok());
    // An adapter whose per-block segment is the wrong length is refused by name.
    let layout = layout(&[spec(0, 0x100)]);
    let cells: Vec<std::sync::atomic::AtomicU8> = (0..0x80)
        .map(|_| std::sync::atomic::AtomicU8::new(0))
        .collect();
    let short = kf_linux_raw::RamSegment::over_cells(&cells).expect("segment");
    assert_eq!(
        translate_blocks(&layout, &[short], LayoutEpoch::new(1), 0, 8).map(|t| t.total_len()),
        Err(TranslateError::Inconsistent { index: 0 })
    );
    assert_eq!(
        translate_blocks(&layout, &[], LayoutEpoch::new(1), 0, 8).map(|t| t.total_len()),
        Err(TranslateError::Inconsistent { index: 0 })
    );
}

#[test]
fn the_epoch_moves_on_add_remove_and_resize_and_a_guard_notices() {
    let vmm = TestVmm::new();
    vmm.add_resizeable(0x1000, 0x1000, 0x4000);
    vmm.fill(0x1000, |i| i as u8);
    let e0 = vmm.epoch();
    let g = vmm.lock_layout().expect("layout");
    assert_eq!(g.epoch(), e0);
    assert!(g.still_current());
    let t = g.translate(0x1000, 0x10).expect("t");
    assert_eq!(t.epoch(), e0);
    vmm.resize(0x1000, 0x2000);
    assert!(!g.still_current());
    assert!(vmm.epoch() > e0);
    // The held guard still reads the block it saw (the hold keeps the memory); the base did not move.
    let mut b = [0u8; 4];
    g.translate(0x1000, 4)
        .expect("t")
        .read_into(&mut b)
        .expect("read");
    assert_eq!(b, [0, 1, 2, 3]);
    // A fresh guard sees the grown block, same bytes at the same offsets.
    let g2 = vmm.lock_layout().expect("layout");
    assert_eq!(g2.layout().blocks()[0].len, 0x2000);
    let mut c = [0u8; 4];
    g2.translate(0x1ffc, 4)
        .expect("t")
        .read_into(&mut c)
        .expect("read");
    assert_eq!(c, [0xfc, 0xfd, 0xfe, 0xff]);
    vmm.remove(0x1000);
    assert!(!g2.still_current());
    // Removed: the held guard still reads (the hold), a new one finds nothing.
    assert!(g2.translate(0x1000, 4).is_ok());
    let g3 = vmm.lock_layout().expect("empty layout");
    assert!(g3.translate(0x1000, 4).is_err());
}
