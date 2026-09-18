//! ★★★★★ §39(c) — a report may not name memory outside the guest's own GPGA span.
//!
//! ⊘ Why this file exists rather than a line in an existing test: the property it checks is
//! not *well-formedness*, it is **escalation**. Every other property `Report::validate`
//! asserts (magic, capacities, slice extents, zero length) describes a report that is
//! internally inconsistent. This one describes a report that is perfectly well formed and
//! would, if acted on, **hand the guest memory that is not its own**.
//!
//! ⚠ Until w760l the validator documented as *"what production consults"* had no such check
//! at all — it was added to the CUDA kernel and to `storemap::map` while the middle layer,
//! the one production actually calls, said nothing about where a run points.

use kayfabe_cuda::abi::{KfMapRun, KfPdbEntry, KfReportHeader, KFWR_MAGIC, KFWR_OP_UNMAP};
use kayfabe_cuda::walk::{Report, ReportError};

const SPAN: u64 = 8 << 20;

fn report(runs: Vec<KfMapRun>) -> Report {
    let mut header = KfReportHeader::default();
    header.magic = KFWR_MAGIC;
    header.pdb_count = 1;
    header.pdb_capacity = 8;
    header.run_count = u32::try_from(runs.len()).unwrap();
    header.run_capacity = 64;
    let mut pdb = KfPdbEntry::default();
    pdb.first_run = 0;
    pdb.run_count = u32::try_from(runs.len()).unwrap();
    Report {
        header,
        pdbs: vec![pdb],
        runs,
        gpga_span: SPAN,
    }
}

fn run(gpga: u64, len: u64, op: u16) -> KfMapRun {
    let mut r = KfMapRun::default();
    r.va = 0x8000_0000;
    r.gpga = gpga;
    r.len = len;
    r.op = op;
    r.pdb_index = 0;
    r
}

/// The CONTROL. Without it the refusals below could be passing for any reason at all.
#[test]
fn a_run_wholly_inside_the_span_is_accepted() {
    let r = report(vec![run(0x30_0000, 4096, 1)]);
    assert_eq!(r.validate(), Ok(()), "a legal run must not be refused");
}

/// ★ The off-by-one, and the one an over-strict fix breaks: the last legal page ends exactly
/// AT the span. Refusing it would silently drop the guest's top page of memory.
#[test]
fn a_run_ending_exactly_at_the_span_is_the_last_legal_page() {
    let r = report(vec![run(SPAN - 4096, 4096, 1)]);
    assert_eq!(r.validate(), Ok(()), "gpga + len == span is the LAST LEGAL page, not an error");
}

#[test]
fn a_run_past_the_span_is_refused_by_name() {
    let r = report(vec![run(SPAN + (16 << 20), 4096, 1)]);
    match r.validate() {
        Err(ReportError::RunOutsideGpga { index, span, .. }) => {
            assert_eq!(index, 0);
            assert_eq!(span, SPAN);
        }
        other => panic!(
            "★★★★★ a run pointing outside the guest's GPGA was ACCEPTED by the validator \
             production consults. Acting on it maps memory that is not the guest's. Got: {other:?}"
        ),
    }
}

/// Starts inside, ends outside — the case a naive `gpga < span` test misses.
#[test]
fn a_run_straddling_the_end_is_refused() {
    let r = report(vec![run(SPAN - 4096, 2 << 20, 1)]);
    assert!(
        matches!(r.validate(), Err(ReportError::RunOutsideGpga { .. })),
        "a run that STARTS inside and ENDS outside must be refused: checking only the start \
         address lets the tail reach past the store"
    );
}

/// An UNMAP retires a VA and carries no gpga, so it is exempt — and must stay exempt, or the
/// walker loses its ability to report removals at all.
#[test]
fn an_unmap_is_exempt_because_it_names_no_memory() {
    let r = report(vec![run(SPAN + (16 << 20), 4096, KFWR_OP_UNMAP)]);
    assert_eq!(
        r.validate(),
        Ok(()),
        "an UNMAP carries no gpga; bounding it would refuse every legitimate removal"
    );
}

/// ★★★★★ **The `acked` offset is DERIVED, and this pins that it is the field we think.**
///
/// ⊘ `WalkKernel::ack` writes one `u64` at `dev.ptr + ACKED_BYTE_OFFSET`. `generation` is the
/// same type and sits immediately before it, so an off-by-one field would overwrite the
/// generation counter with the generation number — a write that looks plausible in a dump and
/// makes the handshake silently self-satisfying.
#[test]
fn the_acked_offset_names_acked_and_not_its_neighbour() {
    use kayfabe_cuda::abi::KfDev;
    assert_eq!(
        core::mem::offset_of!(KfDev, generation),
        0,
        "generation is expected first; if it moved, re-read ack()'s doc before trusting it"
    );
    assert_eq!(
        core::mem::offset_of!(KfDev, acked),
        8,
        "★ `acked` must follow `generation`. A change here is not cosmetic: `ack()` writes at \
         this offset and its neighbour is the counter the kernel compares against."
    );
}
