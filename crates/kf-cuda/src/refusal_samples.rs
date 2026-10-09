//! ★★★ **REFUSAL SAMPLES, PRINTED** — observation only (ABI 6, 2026-10-09).
//!
//! A walk report carries a COUNT and a MASK of refusals (`refusals=103 refuse_mask=0x400` on a
//! real Windows guest, run 223: `KFWR_R_MISALIGNED_LEAF`), which cannot say whether those leaves
//! are valid-but-misaligned or garbage. The kernel therefore records the first
//! [`KF_REFUSAL_SAMPLES`] refusals that name a guest entry (`va`, the raw entry as read, the
//! descriptor's level, the decoded target, the size it had to be aligned to, the refusal bit),
//! and this module prints them: ONE bounded line per sample, the first
//! [`REFUSAL_SAMPLE_LINE_CAP`] samples of the process, then nothing.
//!
//! ⊘ **Nothing here changes a decision.** The samples are read from the header after the fact;
//! no mapping, no refusal and no ack depends on them. The count the device wrote is clamped to
//! the array again here ([`crate::abi::KfReportHeader::refusal_samples`]).
//!
//! ⊘ **Logging, not a stall.** `eprintln!` is the exempt kind of call under constraint 35 (a
//! bounded handful of short lines, never inside a lock a vCPU takes); the per-process cap is
//! what keeps it bounded when a guest keeps spelling the same bad leaf in every walk.

use crate::abi::{
    KF_REFUSAL_SAMPLES, KFWR_R_FOREIGN_AP, KFWR_R_LEAF_OOB, KFWR_R_MISALIGNED_LEAF, KFWR_R_OOB,
    KFWR_R_UNALIGNED, KfRefusalSample,
};
use crate::walk::Report;
use std::sync::atomic::{AtomicU32, Ordering};

/// Sample lines printed per process, in total.
pub const REFUSAL_SAMPLE_LINE_CAP: u32 = 32;

/// Lines printed so far by this process.
static PRINTED: AtomicU32 = AtomicU32::new(0);

/// The refusal's name, for the line (`bit=0x400(MISALIGNED_LEAF)`); an unknown bit prints `?`.
#[must_use]
pub fn bit_name(bit: u32) -> &'static str {
    match bit {
        KFWR_R_OOB => "OOB",
        KFWR_R_UNALIGNED => "UNALIGNED",
        KFWR_R_FOREIGN_AP => "FOREIGN_AP",
        KFWR_R_MISALIGNED_LEAF => "MISALIGNED_LEAF",
        KFWR_R_LEAF_OOB => "LEAF_OOB",
        _ => "?",
    }
}

/// One sample as the log line. `space` is the walk entry's page-directory base when the report
/// still names it.
#[must_use]
pub fn format_sample(space: Option<u64>, s: &KfRefusalSample) -> String {
    let space = space.map_or_else(|| format!("entry{}", s.entry), |p| format!("{p:#x}"));
    format!(
        "kf3: walk refusal sample: space={space} va={:#x} raw={:#018x} level={} gpga={:#x} \
         ps={:#x} bit={:#x}({})",
        s.va,
        s.raw,
        s.level,
        s.gpga,
        s.ps_bytes,
        s.bit,
        bit_name(s.bit)
    )
}

/// Take up to `want` lines from `budget` (never past `cap`), atomically and saturating.
fn take(budget: &AtomicU32, cap: u32, want: usize) -> usize {
    let mut took = 0;
    for _ in 0..want {
        if budget
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < cap).then_some(n + 1)
            })
            .is_err()
        {
            break;
        }
        took += 1;
    }
    took
}

/// The lines to print for `report`, charged to `budget` (cap `cap`). Pure apart from the budget.
/// At most [`KF_REFUSAL_SAMPLES`] per report, whatever count the device wrote.
#[must_use]
pub fn sample_lines(report: &Report, budget: &AtomicU32, cap: u32) -> Vec<String> {
    let samples = report.header.refusal_samples();
    debug_assert!(samples.len() <= KF_REFUSAL_SAMPLES);
    let n = take(budget, cap, samples.len());
    samples[..n]
        .iter()
        .map(|s| format_sample(report.pdbs.get(usize::from(s.entry)).map(|p| p.pdb), s))
        .collect()
}

/// Print `report`'s refusal samples to stderr (one line each), within the per-process cap.
pub fn log_report_samples(report: &Report) {
    if report.header.sample_count == 0 {
        return;
    }
    for line in sample_lines(report, &PRINTED, REFUSAL_SAMPLE_LINE_CAP) {
        eprintln!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::{KfPdbEntry, KfReportHeader};

    fn report(count: u32, entries: usize) -> Report {
        let mut header = KfReportHeader {
            sample_count: count,
            sample_total: count,
            ..KfReportHeader::default()
        };
        for (i, s) in header.samples.iter_mut().enumerate() {
            let i = i as u64;
            *s = KfRefusalSample {
                va: 0x7000_0000_0000 + i * 0x1000,
                raw: 0x1234_0000_0000_0000 | i,
                gpga: 0x1000 * (i + 1),
                ps_bytes: 0x10000,
                bit: KFWR_R_MISALIGNED_LEAF,
                level: 5,
                entry: 0,
            };
        }
        Report {
            header,
            pdbs: vec![
                KfPdbEntry {
                    pdb: 0x00ab_c000,
                    ..KfPdbEntry::default()
                };
                entries
            ],
            runs: Vec::new(),
            gpga_span: 0,
        }
    }

    #[test]
    fn a_sample_is_one_line_with_every_field() {
        let r = report(1, 1);
        let b = AtomicU32::new(0);
        let lines = sample_lines(&r, &b, 32);
        assert_eq!(lines.len(), 1);
        assert_eq!(
            lines[0],
            "kf3: walk refusal sample: space=0xabc000 va=0x700000000000 raw=0x1234000000000000 \
             level=5 gpga=0x1000 ps=0x10000 bit=0x400(MISALIGNED_LEAF)"
        );
        assert!(!lines[0].contains('\n'));
    }

    #[test]
    fn a_count_past_the_array_is_clamped_not_trusted() {
        let r = report(u32::MAX, 1);
        let b = AtomicU32::new(0);
        assert_eq!(sample_lines(&r, &b, 1000).len(), KF_REFUSAL_SAMPLES);
        assert_eq!(r.header.refusal_samples().len(), KF_REFUSAL_SAMPLES);
    }

    #[test]
    fn the_process_cap_is_32_lines_across_walks() {
        let b = AtomicU32::new(0);
        let r = report(8, 1);
        let mut total = 0;
        for _ in 0..10 {
            total += sample_lines(&r, &b, REFUSAL_SAMPLE_LINE_CAP).len();
        }
        assert_eq!(total, 32, "10 walks x 8 samples, capped at 32 lines");
        assert_eq!(b.load(Ordering::Relaxed), 32, "the counter saturates at the cap");
    }

    #[test]
    fn an_entry_the_report_no_longer_names_prints_its_index() {
        let r = report(1, 0);
        let b = AtomicU32::new(0);
        let lines = sample_lines(&r, &b, 32);
        assert!(lines[0].contains("space=entry0 "), "{}", lines[0]);
    }

    #[test]
    fn no_samples_prints_nothing() {
        let r = report(0, 1);
        let b = AtomicU32::new(0);
        assert!(sample_lines(&r, &b, 32).is_empty());
        assert_eq!(b.load(Ordering::Relaxed), 0);
    }
}
