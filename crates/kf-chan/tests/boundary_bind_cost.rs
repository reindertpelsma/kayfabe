// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ 2026-11-18 (boundary fuzzing, `docs/design/V3_BOUNDARY_FUZZING.md`, finding BF-1): what one
//! refused guest-kernel CE launch may cost the host before it is refused.
//!
//! `tmode::bind` refuses a launch whose operand needs more than [`MAX_PIECES`] pieces, and a
//! multi-line launch whose operand is not one span. Both checks need the operand's spans, so the
//! number of spans RESOLVED before the refusal is what bounds the work and the allocation, and the
//! guest kernel chooses it: a multi-line footprint is `pitch × lines` (two `u32`s), and the rows are
//! its own mappings. Invariant: a launch resolves at most `MAX_PIECES + 1` spans per side, whatever its
//! footprint.
use kf_abi::submit::{ce, method_header_inc};
use kf_chan::tmode::{MAX_PIECES, Rows, Span, bind, decode, resolve_spans};
use kf_chan::translated::Refusal;
use kf_chan::tspace_unsafe::TWindows;
use kf_host::MapPerm;
use std::cell::Cell;

const FB: (u64, u64) = (0x1_2000_0000, 0x2_EFBE_0000);
const RAM: (u64, u64) = (0x4_1000_0000, 0x2_0000_0000);
const LIMIT: u64 = (1 << 40) - (4 << 30);
const SUB: u32 = 4;
const VA: u64 = 0x5_0000_0000;
const PAGE: u64 = 0x1000;

/// `pages` discontiguous 4 KiB rows from [`VA`] (every other RAM page), counting the spans and the
/// row lookups every resolve makes.
struct Discontiguous {
    pages: u64,
    spans: Cell<usize>,
    lookups: Cell<u64>,
}
impl Rows for Discontiguous {
    fn resolve(&self, va: u64, len: u64) -> Result<Vec<Span>, u64> {
        let r = resolve_spans(va, len, |at| {
            self.lookups.set(self.lookups.get() + 1);
            let k = at.checked_sub(VA)? / PAGE;
            (k < self.pages).then(|| {
                (
                    true,
                    k * 2 * PAGE + (at - VA) % PAGE,
                    PAGE - (at - VA) % PAGE,
                    MapPerm::READ_WRITE,
                )
            })
        });
        if let Ok(s) = &r {
            self.spans.set(self.spans.get().max(s.len()));
        }
        r
    }
    fn dma_to_file_range(&self, dma: u64, len: u64) -> Option<u64> {
        (dma.checked_add(len)? <= RAM.1).then_some(dma)
    }
}

fn m(sub: u32, method: u32, args: &[u32]) -> Vec<u32> {
    let mut v = vec![method_header_inc(sub, method, args.len() as u32).unwrap()];
    v.extend_from_slice(args);
    v
}

fn is_ce(c: u32) -> bool {
    c == 0xc7b5
}

/// A multi-line virtual-to-physical copy whose source spans `lines` pages at a 4 KiB pitch.
fn multi_line(lines: u32) -> Vec<u32> {
    let mut pb = m(SUB, 0, &[0xc7b5]);
    pb.extend(m(
        SUB,
        ce::OFFSET_IN_UPPER,
        &[(VA >> 32) as u32, VA as u32, 0, 0x20_0000],
    ));
    // PITCH_IN, PITCH_OUT, LINE_LENGTH_IN, LINE_COUNT
    pb.extend(m(SUB, 0x410, &[PAGE as u32, PAGE as u32, 0x800, lines]));
    pb.extend(m(SUB, ce::SET_DST_PHYS_MODE, &[0]));
    pb.extend(m(
        SUB,
        ce::LAUNCH_DMA,
        &[0x182 | ce::LAUNCH_MULTI_LINE_ENABLE | ce::LAUNCH_DST_PHYSICAL],
    ));
    pb
}

fn bind_all(pb: &[u32], rows: &dyn Rows) -> Result<Vec<u32>, Refusal> {
    let w = TWindows::new(FB, RAM, LIMIT).unwrap();
    let ir = decode(pb, is_ce, &mut Default::default(), None)?;
    let mut out = Vec::new();
    for it in &ir {
        bind(it, rows, &w, &mut out)?;
    }
    Ok(out)
}

#[test]
#[ignore = "known defect: BF-1 (spans resolved before the MAX_PIECES cap), fix tracked in docs/design/V3_BOUNDARY_FUZZING.md"]
fn a_refused_multi_line_launch_resolves_a_bounded_number_of_spans() {
    let lines = 64 * MAX_PIECES as u32;
    let rows = Discontiguous {
        pages: u64::from(lines),
        spans: Cell::new(0),
        lookups: Cell::new(0),
    };
    let t = std::time::Instant::now();
    let r = bind_all(&multi_line(lines), &rows);
    let dt = t.elapsed();
    assert!(
        r.is_err(),
        "a discontiguous multi-line source is refused: {r:?}"
    );
    assert!(
        rows.spans.get() <= MAX_PIECES + 1 && rows.lookups.get() <= MAX_PIECES as u64 + 1,
        "refused ({r:?}) after resolving {} spans with {} row lookups in {dt:?}: the work is the guest's footprint, \
         not the cap",
        rows.spans.get(),
        rows.lookups.get()
    );
}

/// BF-1 pinned as it is today (`V3_REPLAY_TESTS.md` step 5): the refusal comes only after one span per
/// line was resolved, so the work grows with the guest's `LINE_COUNT`. The fix deletes this pin and
/// removes the `#[ignore]` above.
#[test]
fn known_defect_bf1_a_refused_multi_line_launch_resolves_its_whole_footprint() {
    let lines = 4 * MAX_PIECES as u32;
    let rows = Discontiguous {
        pages: u64::from(lines),
        spans: Cell::new(0),
        lookups: Cell::new(0),
    };
    assert_eq!(
        bind_all(&multi_line(lines), &rows),
        Err(Refusal::MultiLineDiscontiguous)
    );
    assert_eq!(rows.spans.get(), lines as usize);
}
