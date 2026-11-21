// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ 2026-11-18 (boundary fuzzing, `docs/design/V3_BOUNDARY_FUZZING.md`, finding BF-1, fixed): what one
//! refused guest-kernel CE launch may cost the host before it is refused.
//!
//! `tmode::bind` refuses a launch whose operand needs more than [`MAX_PIECES`] pieces, and a
//! multi-line launch whose operand is not one span. Both checks need the operand's spans, so the
//! number of spans RESOLVED before the refusal is what bounds the work and the allocation, and the
//! guest kernel chooses it: a multi-line footprint is `pitch × lines` (two `u32`s), and the rows are
//! its own mappings. Invariant: a launch resolves at most `MAX_PIECES + 1` spans per side, whatever its
//! footprint.
use kf_abi::submit::{ce, method_header_inc};
use kf_chan::tmode::{
    MAX_PIECES, MAX_SPANS, PushError, Pushed, Rows, Span, bind, decode, push_bound, resolve_spans,
};
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

impl Discontiguous {
    fn new(pages: u64) -> Discontiguous {
        Discontiguous {
            pages,
            spans: Cell::new(0),
            lookups: Cell::new(0),
        }
    }
}
impl Rows for Discontiguous {
    fn resolve(&self, va: u64, len: u64) -> Result<Vec<Span>, kf_chan::tmode::Unresolved> {
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
        let n = match &r {
            Ok(s) => s.len(),
            Err(_) => self.lookups.get() as usize,
        };
        self.spans.set(self.spans.get().max(n));
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
fn a_refused_multi_line_launch_resolves_a_bounded_number_of_spans() {
    let lines = 64 * MAX_PIECES as u32;
    let rows = Discontiguous::new(u64::from(lines));
    let r = bind_all(&multi_line(lines), &rows);
    assert!(
        matches!(r, Err(Refusal::TooManySpans { va: VA, .. })),
        "{r:?}"
    );
    assert!(
        rows.lookups.get() <= MAX_SPANS as u64 + 1,
        "refused ({r:?}) after {} row lookups: the work is the guest's footprint, not the cap",
        rows.lookups.get()
    );
}

/// ★ The cap is exactly `MAX_SPANS`: a single-line copy over `MAX_PIECES + 1` discontiguous pages
/// resolves (both its spans fit) and is refused by its PIECES, as before BF-1; one page more is
/// refused by its SPANS, at the cap.
#[test]
fn the_span_cap_sits_exactly_above_what_a_bind_could_accept() {
    for (pages, want_spans) in [(MAX_PIECES as u64 + 1, false), (MAX_SPANS as u64 + 1, true)] {
        let rows = Discontiguous::new(pages);
        let mut pb = m(SUB, 0, &[0xc7b5]);
        pb.extend(m(
            SUB,
            ce::OFFSET_IN_UPPER,
            &[(VA >> 32) as u32, VA as u32, 0, 0x20_0000],
        ));
        pb.extend(m(SUB, ce::LINE_LENGTH_IN, &[(PAGE * pages) as u32]));
        pb.extend(m(SUB, ce::SET_DST_PHYS_MODE, &[0]));
        pb.extend(m(SUB, ce::LAUNCH_DMA, &[0x182 | ce::LAUNCH_DST_PHYSICAL]));
        let r = bind_all(&pb, &rows);
        if want_spans {
            assert!(
                matches!(r, Err(Refusal::TooManySpans { .. })),
                "{pages} pages: {r:?}"
            );
        } else {
            assert_eq!(
                r,
                Err(Refusal::TooManyPieces {
                    pieces: pages as usize
                }),
                "{pages} pages"
            );
        }
    }
}

/// ★ BF-1 must not turn a WAIT into a refusal: a large operand whose rows are contiguous (one span)
/// but end before the operand does is `VirtualUnresolved`, which the runner hands back unbound to
/// be re-bound after the walk (`Pushed::Unresolved`); and so is a gap after `MAX_PIECES` spans.
#[test]
fn a_large_bind_that_needs_a_walk_is_still_waited_not_refused() {
    struct Contiguous(u64);
    impl Rows for Contiguous {
        fn resolve(&self, va: u64, len: u64) -> Result<Vec<Span>, kf_chan::tmode::Unresolved> {
            resolve_spans(va, len, |at| {
                let k = at.checked_sub(VA)?;
                (k < self.0).then(|| (true, k, self.0 - k, MapPerm::READ_WRITE))
            })
        }
        fn dma_to_file_range(&self, dma: u64, len: u64) -> Option<u64> {
            (dma.checked_add(len)? <= RAM.1).then_some(dma)
        }
    }
    let lines = 64 * MAX_PIECES as u32;
    // mapped: all but the last line, as one contiguous run
    let contiguous = Contiguous(PAGE * u64::from(lines - 1));
    let gap_after_many = Discontiguous::new(MAX_PIECES as u64);
    for (name, rows, pb) in [
        ("contiguous", &contiguous as &dyn Rows, multi_line(lines)),
        (
            "discontiguous",
            &gap_after_many as &dyn Rows,
            multi_line(MAX_PIECES as u32 + 1),
        ),
    ] {
        let r = bind_all(&pb, rows);
        assert!(
            matches!(r, Err(Refusal::VirtualUnresolved { .. })),
            "{name}: {r:?}"
        );
        let w = TWindows::new(FB, RAM, LIMIT).unwrap();
        let ir = decode(&pb, is_ce, &mut Default::default(), None).unwrap();
        let pushed = push_bound(&ir, rows, &w, &mut Vec::new(), |_| Ok(true));
        assert!(
            matches!(pushed, Ok(Pushed::Unresolved { .. })),
            "{name}: waited, not refused: {pushed:?}"
        );
    }
    // and the over-cap launch is refused by the runner, not waited on
    let over = Discontiguous::new(u64::from(lines));
    let w = TWindows::new(FB, RAM, LIMIT).unwrap();
    let ir = decode(&multi_line(lines), is_ce, &mut Default::default(), None).unwrap();
    let pushed = push_bound(&ir, &over, &w, &mut Vec::new(), |_| Ok(true));
    assert!(
        matches!(
            pushed,
            Err(PushError::Refused(Refusal::TooManySpans { .. }))
        ),
        "{pushed:?}"
    );
}
