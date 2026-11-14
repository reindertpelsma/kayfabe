//! Tests of [`super::GuestMemory::translate`]: a per-byte model oracle (exhaustive on small
//! layouts, seeded-random on larger ones), a second block-walking oracle for huge addresses, the
//! named edge cases, the hostile-input properties and the fragment cap.

use super::*;

fn spec(gpa: u64, hva: u64, len: u64) -> RamBlockSpec {
    RamBlockSpec { gpa, hva, len }
}

fn layout(blocks: &[RamBlockSpec]) -> RamLayout {
    RamLayout::new(blocks).expect("valid layout")
}

fn hb(hva: u64, len: u64) -> HostBlock {
    HostBlock { hva, len }
}

// ---------------------------------------------------------------- oracles

fn byte_hva(blocks: &[RamBlockSpec], gpa: u64) -> Option<u64> {
    blocks
        .iter()
        .find(|b| gpa >= b.gpa && gpa - b.gpa < b.len)
        .map(|b| b.hva + (gpa - b.gpa))
}

/// The model: map EVERY guest-physical byte to its host byte by linear scan, then run-length
/// encode consecutive host bytes. No shared code with the implementation.
fn oracle_bytes(
    blocks: &[RamBlockSpec],
    gpa: u64,
    len: u64,
) -> Result<Vec<HostBlock>, TranslateError> {
    if len == 0 {
        return Err(TranslateError::ZeroLength);
    }
    let Some(end) = gpa.checked_add(len) else {
        return Err(TranslateError::Overflow { gpa, len });
    };
    let mut out: Vec<HostBlock> = Vec::new();
    for g in gpa..end {
        let Some(h) = byte_hva(blocks, g) else {
            return Err(TranslateError::Unbacked { gpa: g });
        };
        match out.last_mut() {
            Some(l) if l.hva + l.len == h => l.len += 1,
            _ => out.push(hb(h, 1)),
        }
    }
    Ok(out)
}

/// A second model for addresses too large to walk byte by byte: repeatedly find the block that
/// holds the next byte by linear scan and take as much of it as the range allows.
fn oracle_blocks(
    blocks: &[RamBlockSpec],
    gpa: u64,
    len: u64,
) -> Result<Vec<HostBlock>, TranslateError> {
    if len == 0 {
        return Err(TranslateError::ZeroLength);
    }
    let Some(end) = gpa.checked_add(len) else {
        return Err(TranslateError::Overflow { gpa, len });
    };
    let mut out: Vec<HostBlock> = Vec::new();
    let mut at = gpa;
    while at < end {
        let Some(b) = blocks.iter().find(|b| at >= b.gpa && at - b.gpa < b.len) else {
            return Err(TranslateError::Unbacked { gpa: at });
        };
        let take = (b.gpa + b.len).min(end) - at;
        let h = b.hva + (at - b.gpa);
        match out.last_mut() {
            Some(l) if l.hva + l.len == h => l.len += take,
            _ => out.push(hb(h, take)),
        }
        at += take;
    }
    Ok(out)
}

/// Structural invariants of every `Ok` answer.
fn check_ok(out: &[HostBlock], len: u64) {
    assert!(!out.is_empty());
    assert!(out.len() <= MAX_FRAGMENTS);
    let mut sum = 0u64;
    for (i, b) in out.iter().enumerate() {
        assert!(b.len > 0, "empty block in {out:?}");
        assert!(b.hva.checked_add(b.len).is_some(), "block overflows: {b:?}");
        sum = sum.checked_add(b.len).expect("sum overflows");
        if let Some(n) = out.get(i + 1) {
            assert_ne!(b.hva + b.len, n.hva, "mergeable neighbours in {out:?}");
        }
    }
    assert_eq!(sum, len);
}

// ------------------------------------------------- oracle comparisons

#[test]
fn oracle_agrees_with_translate_on_every_small_layout_exhaustively() {
    // Per block: gap before it (0..=1), length (1..=2), host placement: contiguous with the
    // previous block's host end, far away, or an alias of the previous block's host start.
    let mut layouts = 0u64;
    let mut queries = 0u64;
    for n in 1..=3usize {
        let per: usize = 2 * 2 * 3;
        let total = per.pow(u32::try_from(n).unwrap());
        for code in 0..total * 2 {
            let (g0, mut c) = (code % 2, code / 2);
            let mut blocks: Vec<RamBlockSpec> = Vec::new();
            let mut gpa = g0 as u64;
            let (mut prev_hva, mut prev_len) = (500u64, 0u64);
            for i in 0..n {
                let gap = (c % 2) as u64;
                let len = (c / 2 % 2) as u64 + 1;
                let mode = c / 4 % 3;
                c /= per;
                if i > 0 {
                    gpa += gap;
                }
                let hva = match mode {
                    0 => prev_hva + prev_len,
                    1 => 1000 * (i as u64 + 1),
                    _ => prev_hva,
                };
                blocks.push(spec(gpa, hva, len));
                gpa += len;
                (prev_hva, prev_len) = (hva, len);
            }
            let l = RamLayout::new(&blocks).expect("valid");
            layouts += 1;
            for g in 0..=(gpa + 2) {
                for len in 0..=(gpa + 3) {
                    queries += 1;
                    let got = l.translate(g, len);
                    let want = oracle_bytes(&blocks, g, len);
                    assert_eq!(got, want, "layout {blocks:?} query {g}+{len}");
                    assert_eq!(want, oracle_blocks(&blocks, g, len), "oracles disagree");
                    if let Ok(o) = &got {
                        check_ok(o, len);
                    }
                }
            }
        }
    }
    assert!(layouts > 3000 && queries > 100_000, "{layouts} {queries}");
}

/// xorshift64*: deterministic, no dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// A random valid layout on an 8-byte "page" grid: tiny 1-page blocks, holes, adjacent blocks
/// that are host-contiguous, adjacent blocks that are not, aliases.
fn random_layout(r: &mut Rng, base: u64) -> Vec<RamBlockSpec> {
    const P: u64 = 8;
    let n = 1 + r.below(12);
    let mut gpa = base + r.below(3) * P + r.below(P); // maybe unaligned start
    let mut hva = 0x10_0000 + r.below(64) * P;
    let mut out: Vec<RamBlockSpec> = Vec::new();
    for _ in 0..n {
        let len = (1 + r.below(6)) * P + if r.below(4) == 0 { r.below(P) } else { 0 };
        out.push(spec(gpa, hva, len));
        gpa += len;
        if r.below(3) == 0 {
            gpa += (1 + r.below(4)) * P; // a hole
        }
        match r.below(4) {
            0 => hva += len,                          // host-contiguous with this block
            1 => hva += len + (1 + r.below(8)) * P,   // a gap in the VMM's address space
            2 => hva = hva.saturating_sub(len + P),   // placed below (reversed order)
            _ => hva = 0x20_0000 + r.below(1024) * P, // anywhere (may alias)
        }
    }
    out
}

#[test]
fn oracle_agrees_with_translate_on_seeded_random_layouts() {
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
            let got = l.translate(g, len);
            assert_eq!(got, oracle_bytes(&blocks, g, len), "{blocks:?} {g}+{len}");
            match &got {
                Ok(o) => {
                    check_ok(o, len);
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
            let got = l.translate(g, len);
            assert_eq!(
                got,
                oracle_blocks(&blocks, g, len),
                "{blocks:?} {g:#x}+{len:#x}"
            );
            if let Ok(o) = &got {
                check_ok(o, len);
            }
        }
    }
}

// ------------------------------------------------------ named edge cases

/// q35-like: 2 GiB below the PCI hole, 2 GiB above 4 GiB, mapped separately by the VMM.
fn q35() -> RamLayout {
    layout(&[
        spec(0, 0x7f00_0000_0000, 0x8000_0000),
        spec(0x1_0000_0000, 0x7f00_8000_0000, 0x8000_0000),
    ])
}

#[test]
fn a_range_inside_one_block_is_one_block() {
    let l = q35();
    assert_eq!(
        l.translate(0x1000, 0x2000),
        Ok(vec![hb(0x7f00_0000_1000, 0x2000)])
    );
}

#[test]
fn a_range_exactly_one_whole_block() {
    let l = q35();
    assert_eq!(
        l.translate(0, 0x8000_0000),
        Ok(vec![hb(0x7f00_0000_0000, 0x8000_0000)])
    );
}

#[test]
fn a_range_starting_at_a_block_start_and_one_ending_exactly_at_a_block_end() {
    let l = q35();
    assert_eq!(
        l.translate(0x1_0000_0000, 16),
        Ok(vec![hb(0x7f00_8000_0000, 16)])
    );
    assert_eq!(
        l.translate(0x8000_0000 - 16, 16),
        Ok(vec![hb(0x7f00_7fff_fff0, 16)])
    );
}

#[test]
fn the_last_byte_of_ram_is_backed_and_the_next_byte_is_named() {
    let l = q35();
    let last = 0x1_8000_0000 - 1;
    assert_eq!(l.translate(last, 1), Ok(vec![hb(0x7f00_ffff_ffff, 1)]));
    assert_eq!(
        l.translate(last, 2),
        Err(TranslateError::Unbacked { gpa: 0x1_8000_0000 })
    );
    assert_eq!(
        l.translate(0x1_8000_0000, 1),
        Err(TranslateError::Unbacked { gpa: 0x1_8000_0000 })
    );
}

#[test]
fn the_first_byte_of_the_next_block_after_a_hole() {
    let l = q35();
    assert_eq!(
        l.translate(0x1_0000_0000, 1),
        Ok(vec![hb(0x7f00_8000_0000, 1)])
    );
    // The byte just below the hole's far end is in the hole.
    assert_eq!(
        l.translate(0xFFFF_FFFF, 1),
        Err(TranslateError::Unbacked { gpa: 0xFFFF_FFFF })
    );
}

#[test]
fn a_range_crossing_a_hole_names_the_first_unbacked_address_not_the_end_of_the_hole() {
    let l = q35();
    // Starts in the first block, runs through the hole into the second.
    assert_eq!(
        l.translate(0x7fff_0000, 0x2_0000_0000),
        Err(TranslateError::Unbacked { gpa: 0x8000_0000 })
    );
    // Starts in the hole.
    assert_eq!(
        l.translate(0x9000_0000, 0x1000),
        Err(TranslateError::Unbacked { gpa: 0x9000_0000 })
    );
    // Starts below everything in a layout that begins above zero.
    let high = layout(&[spec(0x1000, 0x5000, 0x1000)]);
    assert_eq!(
        high.translate(0, 0x2000),
        Err(TranslateError::Unbacked { gpa: 0 })
    );
    assert_eq!(
        high.translate(0xff0, 0x20),
        Err(TranslateError::Unbacked { gpa: 0xff0 })
    );
}

#[test]
fn a_range_starting_past_all_of_ram_and_an_empty_layout() {
    let l = q35();
    assert_eq!(
        l.translate(1 << 40, 1),
        Err(TranslateError::Unbacked { gpa: 1 << 40 })
    );
    let empty = RamLayout::new(&[]).expect("empty is a layout");
    assert_eq!(
        empty.translate(0, 1),
        Err(TranslateError::Unbacked { gpa: 0 })
    );
    assert_eq!(empty.total_bytes(), 0);
}

#[test]
fn adjacent_guest_blocks_that_are_contiguous_in_the_vmm_are_one_block() {
    let l = layout(&[
        spec(0x0, 0x10_0000, 0x1000),
        spec(0x1000, 0x10_1000, 0x1000),
    ]);
    assert_eq!(l.translate(0xf00, 0x200), Ok(vec![hb(0x10_0f00, 0x200)]));
    assert_eq!(l.translate(0, 0x2000), Ok(vec![hb(0x10_0000, 0x2000)]));
}

#[test]
fn adjacent_guest_blocks_that_are_not_contiguous_in_the_vmm_are_two_blocks_in_order() {
    let l = layout(&[
        spec(0x0, 0x10_0000, 0x1000),
        spec(0x1000, 0x90_0000, 0x1000),
    ]);
    assert_eq!(
        l.translate(0xf00, 0x200),
        Ok(vec![hb(0x10_0f00, 0x100), hb(0x90_0000, 0x100)])
    );
}

#[test]
fn adjacent_guest_blocks_with_the_host_order_reversed_are_two_blocks() {
    // The second block sits immediately BELOW the first in the VMM: not mergeable.
    let l = layout(&[
        spec(0x0, 0x10_1000, 0x1000),
        spec(0x1000, 0x10_0000, 0x1000),
    ]);
    assert_eq!(
        l.translate(0xff0, 0x20),
        Ok(vec![hb(0x10_1ff0, 0x10), hb(0x10_0000, 0x10)])
    );
}

#[test]
fn a_range_through_three_blocks_gets_the_middle_one_whole() {
    let l = layout(&[
        spec(0x0, 0x1_0000, 0x1000),
        spec(0x1000, 0x9_0000, 0x1000),
        spec(0x2000, 0x5_0000, 0x1000),
    ]);
    assert_eq!(
        l.translate(0xfff, 0x1002),
        Ok(vec![hb(0x1_0fff, 1), hb(0x9_0000, 0x1000), hb(0x5_0000, 1)])
    );
}

#[test]
fn host_aliases_are_allowed_and_answered_independently() {
    let l = layout(&[
        spec(0x0, 0x10_0000, 0x1000),
        spec(0x4000, 0x10_0000, 0x1000),
    ]);
    assert_eq!(l.translate(0x4000, 8), Ok(vec![hb(0x10_0000, 8)]));
    assert_eq!(l.translate(0, 8), Ok(vec![hb(0x10_0000, 8)]));
}

#[test]
fn unaligned_gpa_and_length() {
    let l = layout(&[spec(0x1003, 0x7007, 0x11)]);
    assert_eq!(l.translate(0x1005, 3), Ok(vec![hb(0x7009, 3)]));
    assert_eq!(l.translate(0x1003, 0x11), Ok(vec![hb(0x7007, 0x11)]));
    assert_eq!(
        l.translate(0x1003, 0x12),
        Err(TranslateError::Unbacked { gpa: 0x1014 })
    );
    assert_eq!(
        l.translate(0x1002, 1),
        Err(TranslateError::Unbacked { gpa: 0x1002 })
    );
}

#[test]
fn zero_length_is_an_error_even_at_a_backed_address() {
    let l = q35();
    assert_eq!(l.translate(0x1000, 0), Err(TranslateError::ZeroLength));
    assert_eq!(l.translate(u64::MAX, 0), Err(TranslateError::ZeroLength));
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
            l.translate(g, n),
            Err(TranslateError::Overflow { gpa: g, len: n }),
            "{g:#x}+{n:#x}"
        );
    }
    // The largest representable range ending at u64::MAX does not overflow (and is unbacked).
    assert_eq!(
        l.translate(u64::MAX - 1, 1),
        Err(TranslateError::Unbacked { gpa: u64::MAX - 1 })
    );
}

#[test]
fn huge_lengths_are_unbacked_not_allocated() {
    let l = q35();
    assert_eq!(
        l.translate(0, u64::MAX),
        Err(TranslateError::Unbacked { gpa: 0x8000_0000 })
    );
    assert_eq!(
        l.translate(0x1_0000_0000, u64::MAX - 0x1_0000_0000),
        Err(TranslateError::Unbacked { gpa: 0x1_8000_0000 })
    );
}

#[test]
fn a_block_ending_exactly_at_u64_max_translates_up_to_its_last_byte() {
    let l = layout(&[spec(u64::MAX - 0xfff, 0x4000, 0xfff)]);
    assert_eq!(
        l.translate(u64::MAX - 0xfff, 0xfff),
        Ok(vec![hb(0x4000, 0xfff)])
    );
    // One byte more would end at 2^64: an overflow (the exclusive end is not representable).
    assert_eq!(
        l.translate(u64::MAX - 0xfff, 0x1000),
        Err(TranslateError::Overflow {
            gpa: u64::MAX - 0xfff,
            len: 0x1000
        })
    );
    assert_eq!(
        l.translate(u64::MAX - 0xfff, 0xfff),
        Ok(vec![hb(0x4000, 0xfff)])
    );
}

#[test]
fn a_host_range_ending_exactly_at_u64_max_merges_without_overflow() {
    let l = layout(&[
        spec(0, u64::MAX - 0x2000, 0x1000),
        spec(0x1000, u64::MAX - 0x1000, 0x1000),
    ]);
    // hva + len of the first == hva of the second; the merged block ends at u64::MAX (exclusive).
    assert_eq!(
        l.translate(0, 0x2000),
        Ok(vec![hb(u64::MAX - 0x2000, 0x2000)])
    );
}

#[test]
fn export_fd_defaults_to_none() {
    assert!(q35().export_fd(0, 16).is_none());
    assert!(q35().export_fd(u64::MAX, u64::MAX).is_none());
}

// ----------------------------------------------------- layout validation

#[test]
fn layout_validation_refuses_by_name() {
    assert_eq!(
        RamLayout::new(&[spec(0, 0, 0)]),
        Err(LayoutError::ZeroLength { index: 0 })
    );
    assert_eq!(
        RamLayout::new(&[spec(0, 0, 8), spec(u64::MAX, 0, 2)]),
        Err(LayoutError::GpaOverflow { index: 1 })
    );
    assert_eq!(
        RamLayout::new(&[spec(0, u64::MAX, 2)]),
        Err(LayoutError::HvaOverflow { index: 0 })
    );
    assert_eq!(
        RamLayout::new(&[spec(0x2000, 0, 8), spec(0x1000, 0x100, 8)]),
        Err(LayoutError::Unsorted { index: 1 })
    );
    assert_eq!(
        RamLayout::new(&[spec(0, 0, 0x1001), spec(0x1000, 0x5000, 8)]),
        Err(LayoutError::Overlap { index: 1 })
    );
    // Same start is an overlap, not "unsorted".
    assert_eq!(
        RamLayout::new(&[spec(0, 0, 8), spec(0, 0x100, 8)]),
        Err(LayoutError::Overlap { index: 1 })
    );
    let many: Vec<RamBlockSpec> = (0..=MAX_LAYOUT_BLOCKS as u64)
        .map(|i| spec(i * 2, i * 2, 1))
        .collect();
    assert_eq!(RamLayout::new(&many), Err(LayoutError::TooManyBlocks));
    let ok: Vec<RamBlockSpec> = (0..MAX_LAYOUT_BLOCKS as u64)
        .map(|i| spec(i * 2, i * 2, 1))
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
}

// -------------------------------------------------------- fragment cap

/// `n` one-page blocks, adjacent in guest-physical space, each placed at a host address that
/// leaves a gap (so no two are mergeable).
fn fragmented(n: u64) -> RamLayout {
    let blocks: Vec<RamBlockSpec> = (0..n)
        .map(|i| spec(i * 0x1000, 0x10_0000 + i * 0x3000, 0x1000))
        .collect();
    layout(&blocks)
}

#[test]
fn exactly_the_cap_is_served_and_one_more_is_refused() {
    let l = fragmented(MAX_FRAGMENTS as u64 + 1);
    let n = MAX_FRAGMENTS as u64;
    let ok = l.translate(0, n * 0x1000).expect("exactly the cap");
    assert_eq!(ok.len(), MAX_FRAGMENTS);
    check_ok(&ok, n * 0x1000);
    assert_eq!(
        l.translate(0, (n + 1) * 0x1000),
        Err(TranslateError::TooFragmented)
    );
    // The cap counts blocks needed by THIS range, not the layout: a range deep in it is fine.
    let tail = l.translate(1 << 12, 0x1000).expect("one fragment");
    assert_eq!(tail.len(), 1);
}

#[test]
fn the_cap_does_not_count_merged_neighbours() {
    // 3 * cap one-page blocks that are all host-contiguous: ONE block.
    let n = 3 * MAX_FRAGMENTS as u64;
    let blocks: Vec<RamBlockSpec> = (0..n)
        .map(|i| spec(i * 0x1000, 0x10_0000 + i * 0x1000, 0x1000))
        .collect();
    let l = layout(&blocks);
    assert_eq!(
        l.translate(0, n * 0x1000),
        Ok(vec![hb(0x10_0000, n * 0x1000)])
    );
}

#[test]
fn a_refused_cap_returns_no_partial_answer_and_bounded_memory() {
    let l = fragmented(MAX_LAYOUT_BLOCKS as u64);
    let r = l.translate(0, MAX_LAYOUT_BLOCKS as u64 * 0x1000);
    assert_eq!(r, Err(TranslateError::TooFragmented));
}

// ----------------------------------------------------- hostile input

#[test]
fn hostile_inputs_never_panic_overflow_or_exceed_the_bound() {
    let layouts = [
        q35(),
        fragmented(MAX_FRAGMENTS as u64 + 5),
        RamLayout::new(&[]).expect("empty"),
        layout(&[spec(u64::MAX - 0xfff, u64::MAX - 0xfff, 0xfff)]),
        layout(&[spec(0, 0, u64::MAX)]),
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
            match l.translate(g, n) {
                Ok(o) => {
                    check_ok(&o, n);
                    assert!(
                        o.capacity() <= 2 * MAX_FRAGMENTS,
                        "capacity {}",
                        o.capacity()
                    );
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
            }
        }
    }
}

#[test]
fn the_unbacked_address_is_the_first_one_for_every_prefix() {
    // For each start inside a layout with a hole, the named address is exactly the first byte
    // the byte-model finds unbacked.
    let blocks = [spec(0x100, 0x1000, 0x40), spec(0x180, 0x2000, 0x40)];
    let l = layout(&blocks);
    for g in 0x80..0x1e0u64 {
        for n in 1..0x120u64 {
            let want = (g..g + n).find(|&x| byte_hva(&blocks, x).is_none());
            match (l.translate(g, n), want) {
                (Ok(_), None) => {}
                (Err(TranslateError::Unbacked { gpa }), Some(w)) => assert_eq!(gpa, w),
                (got, want) => panic!("{g:#x}+{n:#x}: {got:?} vs first unbacked {want:?}"),
            }
        }
    }
}
