//! SPR1: sparse is a placement without a backing address. Tests precede ABI support.

use kf_cuda::abi::{KFWR_OP_MAP, KFWR_OP_UNMAP, KfMapRun};
use kf_cuda::diffmodel::{self, AckCode, Committed};

// Reserved run bit proposed for ABI 7; local so EXPECTED RED compiles on ABI 6.
const SPARSE: u32 = 1 << 7;
const PAGE: u64 = 4096;

fn run(va: u64, len: u64, sparse: bool) -> KfMapRun {
    KfMapRun {
        va,
        len,
        gpga: 0,
        flags: if sparse { SPARSE } else { 0 },
        op: KFWR_OP_MAP,
        pdb_index: 0,
    }
}

fn placed(runs: &[KfMapRun]) -> Committed {
    diffmodel::commit(&Committed::default(), runs, &vec![AckCode::Applied; runs.len()])
}

#[test]
fn old_vidmem_becoming_sparse_is_a_change() {
    let old = placed(&[run(PAGE, PAGE, false)]);
    let d = diffmodel::diff(&old, &[run(PAGE, PAGE, true)], 8);
    assert_eq!(d.runs.len(), 2);
    assert_eq!(d.runs[0].op, KFWR_OP_UNMAP);
    assert_eq!(d.runs[1].flags, SPARSE);
}

#[test]
fn sparse_becoming_vidmem_is_a_change() {
    let old = placed(&[run(PAGE, PAGE, true)]);
    let d = diffmodel::diff(&old, &[run(PAGE, PAGE, false)], 8);
    assert_eq!(d.runs.len(), 2);
    assert_eq!(d.runs[0].op, KFWR_OP_UNMAP);
    assert_eq!(d.runs[1].flags, 0);
}

#[test]
fn sparse_has_no_backing_or_permission_key() {
    assert_ne!(diffmodel::host_key(SPARSE), diffmodel::host_key(0));
    assert_eq!(diffmodel::host_key(SPARSE), diffmodel::host_key(SPARSE | 0x00ff_0078));
}

#[test]
fn sparse_subrange_keeps_no_linear_offset() {
    let old = placed(&[run(PAGE, PAGE, true)]);
    let d = diffmodel::diff(&old, &[run(0, 3 * PAGE, true)], 8);
    assert_eq!(d.runs.len(), 2);
    assert!(d.runs.iter().all(|r| r.op == KFWR_OP_MAP && r.gpga == 0));
    assert_eq!(d.runs[1].va, 2 * PAGE);
}

#[test]
fn adjacent_sparse_statements_cover_one_placement() {
    let old = placed(&[run(0, 2 * PAGE, true)]);
    let walk = [run(0, PAGE, true), run(PAGE, PAGE, true)];
    assert!(diffmodel::diff(&old, &walk, 8).runs.is_empty());
}

#[test]
fn adjacent_sparse_coverage_coalesces_without_backing() {
    let walk = [run(0, PAGE, true), run(PAGE, PAGE, true)];
    assert_eq!(diffmodel::coverage(&walk), diffmodel::coverage(&[run(0, 2 * PAGE, true)]));
}

#[test]
fn refused_sparse_map_is_retried() {
    let walk = [run(PAGE, PAGE, true)];
    let d = diffmodel::diff(&Committed::default(), &walk, 8);
    let old = diffmodel::commit(&Committed::default(), &d.runs, &[AckCode::Failed]);
    assert!(old.is_empty());
    assert_eq!(diffmodel::diff(&old, &walk, 8).runs, d.runs);
}

#[test]
fn acknowledged_sparse_is_quiet_and_plain_invalid_retires_it() {
    let walk = [run(PAGE, PAGE, true)];
    let old = placed(&walk);
    assert!(diffmodel::diff(&old, &walk, 8).runs.is_empty());
    let d = diffmodel::diff(&old, &[], 8);
    assert_eq!(d.runs.len(), 1);
    assert_eq!(d.runs[0].op, KFWR_OP_UNMAP);
    assert!(diffmodel::commit(&old, &d.runs, &[AckCode::Applied]).is_empty());
}
