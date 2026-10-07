//! ★★★★★ **w826 — the host's copy of the walk kernel's state, kept current by DELTAS.**
//!
//! `[measured w826 q5]` the publisher reconciled a FULL report on every pass: ~2.4 µs per run
//! per pass, so `--ce-client-guest-ram` (13 000 rows, one invalidate each) went quadratic and
//! reached ~4 000 rows in its budget. The kernel has always been able to emit deltas against
//! the last report the host ACKED; nothing ever acked.
//!
//! This is the pure half: per address space, per page-size class, a non-overlapping range map
//! of the runs the kernel last reported. A RESYNC entry replaces it; a delta entry edits it and
//! names the ranges it touched, which is all the reconcile then looks at.
//!
//! ⊘ **Per CLASS, not per address space.** Under a dual PDE a 4 KiB run and a 64 KiB run can
//! cover the same VA, and the kernel diffs each class separately — one map would let an unmap
//! in one class delete the other's run.

use std::collections::BTreeMap;

/// Where a run's backing lives, as the report said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunWhere {
    /// Video memory: `gpga` is a store offset.
    Vidmem,
    /// System memory: `gpga` is a guest-physical address.
    Sysmem,
    /// Peer memory: never backed here.
    Peer,
}

/// One run as the mirror holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MRun {
    /// Start VA.
    pub va: u64,
    /// Length in bytes.
    pub len: u64,
    /// Backing address (meaning per [`RunWhere`]).
    pub gpga: u64,
    /// Aperture.
    pub at: RunWhere,
}

/// What a delta asks for one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaOp {
    /// The range now maps `run` (MAP or REMAP): whatever was there in this class goes first.
    Put(MRun),
    /// The range `[va, va+len)` is gone from this class.
    Drop { va: u64, len: u64 },
}

/// Page-size classes the report can name (`ps_log2` has four entries).
pub const CLASSES: usize = 4;

/// The mirror of ONE address space.
#[derive(Debug, Default, Clone)]
pub struct VasMirror {
    class: [BTreeMap<u64, MRun>; CLASSES],
}

impl VasMirror {
    /// Replace the whole state (a RESYNC entry). Runs with a class outside [`CLASSES`] are
    /// dropped and counted.
    pub fn resync(&mut self, runs: impl IntoIterator<Item = (usize, MRun)>) -> u64 {
        let mut bad = 0;
        self.class = Default::default();
        for (c, r) in runs {
            match self.class.get_mut(c) {
                Some(m) if r.len > 0 => {
                    m.insert(r.va, r);
                }
                _ => bad += 1,
            }
        }
        bad
    }

    /// Apply one delta op in class `c`; returns the `[a, b)` VA range it touched.
    pub fn apply(&mut self, c: usize, op: DeltaOp) -> Option<(u64, u64)> {
        let m = self.class.get_mut(c)?;
        let (a, b) = match op {
            DeltaOp::Put(r) => (r.va, r.va.checked_add(r.len)?),
            DeltaOp::Drop { va, len } => (va, va.checked_add(len)?),
        };
        if b <= a {
            return None;
        }
        cut(m, a, b);
        if let DeltaOp::Put(r) = op {
            m.insert(r.va, r);
        }
        Some((a, b))
    }

    /// Every run, all classes, overlapping `[a, b)`.
    #[must_use]
    pub fn overlapping(&self, a: u64, b: u64) -> Vec<MRun> {
        let mut out = Vec::new();
        for m in &self.class {
            if let Some((_, r)) = m.range(..a).next_back() {
                if r.va.saturating_add(r.len) > a {
                    out.push(*r);
                }
            }
            out.extend(m.range(a..b).map(|(_, r)| *r));
        }
        out
    }

    /// Every run, all classes.
    #[must_use]
    pub fn all(&self) -> Vec<MRun> {
        self.class.iter().flat_map(|m| m.values().copied()).collect()
    }

    /// Runs held, all classes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.class.iter().map(BTreeMap::len).sum()
    }

    /// Nothing held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Remove `[a, b)` from one class's map, keeping the pieces of any run that stick out.
fn cut(m: &mut BTreeMap<u64, MRun>, a: u64, b: u64) {
    let mut hit: Vec<MRun> = Vec::new();
    if let Some((_, r)) = m.range(..a).next_back() {
        if r.va.saturating_add(r.len) > a {
            hit.push(*r);
        }
    }
    hit.extend(m.range(a..b).map(|(_, r)| *r));
    for r in hit {
        m.remove(&r.va);
        let end = r.va + r.len;
        if r.va < a {
            m.insert(r.va, MRun { len: a - r.va, ..r });
        }
        if end > b {
            let shift = b - r.va;
            m.insert(
                b,
                MRun {
                    va: b,
                    len: end - b,
                    gpga: r.gpga + shift,
                    at: r.at,
                },
            );
        }
    }
}
