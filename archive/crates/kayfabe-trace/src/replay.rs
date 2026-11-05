//! Stream properties a replay differential rests on: **order**, and **difference**.
//!
//! Both return typed values rather than panicking. A pure crate that asserts is a pure
//! crate a test cannot ask a question of — and `testing_doctrine.md` §2 wants the test to
//! name the exact variant it expects, which it can only do if there is a variant.
//!
//! ## What is compared, and what is not
//!
//! [`diff`] compares the **decoded projection** — the [`crate::TraceEvent`]s, positionally
//! — never raw bytes and never the sequence numbers. `mode2_gsp_port_plan.md` §6.3 is
//! explicit about why: byte-diffing would enshrine the C's zero padding, its uninitialised
//! element tails, and its `rpc.length = 36` for a 32-byte header. Positional comparison
//! also means two streams from two different recorders are comparable even though their
//! [`Seq`] spaces are not.
//!
//! The *admissible-divergence ledger* (§6.3's MUST-DIFFER table, with its C site, its
//! guest-visible consequence and its independent oracle) is deliberately **not** here. Its
//! entries are citations into the C and into ogkm, so it belongs with the harness that
//! owns those citations; this crate owns only the mechanism that finds the difference.

use crate::event::{Record, Seq, TraceEvent};

/// A stream that is not densely, strictly ordered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderingError {
    /// A record's sequence number did not exceed its predecessor's — two records claim
    /// the same position, or the stream went backwards. This is what merging two
    /// recorders' streams produces.
    NotIncreasing {
        /// Index in the slice.
        index: usize,
        /// The previous record's sequence number.
        prev: Seq,
        /// This record's sequence number.
        seq: Seq,
    },
    /// A record is missing: the sequence jumped. A trace with a hole is a trace that
    /// cannot be replayed, so this is a fault and not a warning.
    Gap {
        /// Index in the slice.
        index: usize,
        /// The sequence number this position must carry.
        expected: Seq,
        /// The sequence number it actually carries.
        seq: Seq,
    },
}

/// Check that `records` is one recorder's stream, whole: starting at `from`, dense, and
/// strictly increasing.
///
/// This is the property [`crate::Recorder`] promises, checked rather than assumed —
/// including by the tests that exist to show what happens when the promise does not
/// apply (two recorders, merged).
pub fn check_dense_order(records: &[Record], from: Seq) -> Result<(), OrderingError> {
    let mut expected = from;
    let mut prev: Option<Seq> = None;
    for (index, r) in records.iter().enumerate() {
        if let Some(p) = prev
            && r.seq <= p
        {
            return Err(OrderingError::NotIncreasing {
                index,
                prev: p,
                seq: r.seq,
            });
        }
        if r.seq != expected {
            return Err(OrderingError::Gap {
                index,
                expected,
                seq: r.seq,
            });
        }
        prev = Some(r.seq);
        expected = Seq(expected.0 + 1);
    }
    Ok(())
}

/// Where two decoded projections stopped agreeing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    /// Position in the stream (not a [`Seq`] — see the module docs).
    pub at: usize,
    /// What the reference stream had there, or `None` if it ended first.
    pub expected: Option<TraceEvent>,
    /// What the stream under test had there, or `None` if it ended first.
    pub actual: Option<TraceEvent>,
}

/// Compare two decoded projections, reporting the **first** position where they differ.
/// `None` means identical.
///
/// First rather than all, deliberately: after a divergence the two streams are no longer
/// describing the same run, so every later difference is a consequence rather than a
/// finding, and a list of them buries the one that matters.
#[must_use]
pub fn diff(expected: &[TraceEvent], actual: &[TraceEvent]) -> Option<Divergence> {
    let n = expected.len().max(actual.len());
    for at in 0..n {
        let e = expected.get(at);
        let a = actual.get(at);
        if e != a {
            return Some(Divergence {
                at,
                expected: e.cloned(),
                actual: a.cloned(),
            });
        }
    }
    None
}

/// Compare two streams of [`Record`]s by their decoded projections. `None` means
/// identical.
///
/// Convenience over [`diff`]; the sequence numbers are ignored on purpose.
#[must_use]
pub fn diff_records(expected: &[Record], actual: &[Record]) -> Option<Divergence> {
    let e: Vec<TraceEvent> = expected.iter().map(|r| r.ev.clone()).collect();
    let a: Vec<TraceEvent> = actual.iter().map(|r| r.ev.clone()).collect();
    diff(&e, &a)
}
