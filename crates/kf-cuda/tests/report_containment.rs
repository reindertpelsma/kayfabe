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

use kf_cuda::abi::{KfMapRun, KfPdbEntry, KfReportHeader, KFWR_MAGIC, KFWR_OP_UNMAP};
use kf_cuda::walk::{Report, ReportError};

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

/// ★★★★★ **Every report the VA manager applies must be a DIFF against the committed
/// placements** (`KFWR_HF_DIFF`, the commit-on-ack protocol, `kf_cuda::diffmodel`).
#[test]
fn a_diff_report_is_accepted() {
    let mut r = report(vec![run(0, 4096, 0), run(0, 4096, KFWR_OP_UNMAP)]);
    r.header.flags = kf_cuda::abi::KFWR_HF_DIFF;
    assert_eq!(r.require_diff(), Ok(()));
}

/// The falsifier: a report from a kernel that does not speak the protocol (the old RESYNC/delta
/// reports) is refused by name — applied as a diff, a full state would re-map everything and a
/// delta would never retire a placement it did not name.
#[test]
fn a_report_without_the_diff_flag_is_refused() {
    let mut r = report(vec![run(0, 4096, 0)]);
    r.header.flags = kf_cuda::abi::KFWR_HF_RESYNC;
    assert!(matches!(r.require_diff(), Err(ReportError::NotDiff { flags: 2 })));
}

/// ★★★★★ w828 — a SYSMEM leaf names guest-PHYSICAL memory, not the store: the store span does
/// not bound it (the guest-RAM map does, in `kf_mem::ledger::desired_from_leaves`). `[measured vh
/// vhA_gpm]` bounding it here refused a valid page of an 8 GiB guest at 0x2_028f_0000 and killed
/// UVM's copy channel.
#[test]
fn a_sysmem_run_is_not_bounded_by_the_store_span() {
    for ap in [kf_cuda::abi::KFWR_AP_SYS_COHERENT, kf_cuda::abi::KFWR_AP_SYS_NONCOHERENT] {
        let mut r0 = run(SPAN + (16 << 20), 0x1_0000, 1);
        r0.flags = u32::from(ap);
        assert_eq!(report(vec![r0]).validate(), Ok(()), "aperture {ap}: guest-physical, not a store offset");
    }
}

/// The falsifier for the one above: the SAME address as a VIDMEM (and a PEER) leaf is still refused.
#[test]
fn a_vidmem_or_peer_run_past_the_span_is_still_refused() {
    for ap in [0u8, 1u8] {
        let mut r0 = run(SPAN + (16 << 20), 0x1_0000, 1);
        r0.flags = u32::from(ap);
        assert!(
            matches!(report(vec![r0]).validate(), Err(ReportError::RunOutsideGpga { .. })),
            "aperture {ap} past the store span must stay refused"
        );
    }
}
