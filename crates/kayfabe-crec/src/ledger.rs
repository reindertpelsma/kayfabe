//! Sorting a divergence into **expected** or **finding**.
//!
//! `mode2_gsp_port_plan.md` §6.3 puts every assertion in exactly one of three classes, and
//! the class is *declared in a table, not inferred*. This module is the reader of that
//! table ([`kayfabe_gsp::LEDGER`]) and nothing else: it owns no policy of its own, and it
//! has **no catch-all**. A divergence no rule below matches comes back
//! [`Verdict::Unexplained`], which is the whole value of the exercise — an unexplained
//! divergence is a finding, and one that has been quietly reclassified is not.
//!
//! ## Two rules that keep this honest
//!
//! 1. **A rule may only fire on evidence that is in the divergence itself.** Every
//!    predicate below reads the two decoded notes and the transaction's driving register.
//!    None of them reads a record index, so none of them can be tuned to one capture.
//! 2. **`Expected` is not `Pass`.** A ledger row asserts the C is the thing that is wrong
//!    *at that site*. It says nothing about whether we produced the right answer, which is
//!    what the row's `independent_oracle` field is for.

use kayfabe_arch::gsp::GspReg;
use kayfabe_gsp::{Divergence as LedgerRow, LEDGER, Observation};
use kayfabe_trace::Divergence;

use crate::replay::{Note, ReplayResult, Txn};

/// What one divergence is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// A row of [`kayfabe_gsp::LEDGER`] predicted it, and the C is the thing that is
    /// wrong.
    Expected(&'static LedgerRow),
    /// Nothing in the ledger accounts for it. **A finding.**
    Unexplained,
}

impl Verdict {
    /// The ledger id, or `"—"`.
    #[must_use]
    pub fn id(&self) -> &'static str {
        match self {
            Verdict::Expected(d) => d.id,
            Verdict::Unexplained => "—",
        }
    }
}

/// One classified divergence, with everything a reader needs to check the classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classified {
    /// Which transaction.
    pub txn: usize,
    /// The driving register.
    pub reg: Option<GspReg>,
    /// Position within the transaction's projection.
    pub at: usize,
    /// What the C did there, decoded.
    pub c: Option<Note>,
    /// What we did there, decoded.
    pub rust: Option<Note>,
    /// The verdict.
    pub verdict: Verdict,
    /// ★ Is this divergence at or after [`ReplayResult::closure_limit`] — i.e. after the
    /// capture stopped being able to carry the replay?
    ///
    /// Beyond that point the C keeps going and we do not, so *every* position differs and
    /// the difference is a consequence of the oracle's reach rather than a fact about the
    /// implementation. Counted, never interpreted.
    pub beyond_closure: bool,
}

fn row(id: &str) -> &'static LedgerRow {
    LEDGER
        .iter()
        .find(|d| d.id == id)
        .expect("the ledger carries every id this classifier names")
}

/// Classify one divergence.
///
/// The order of the rules does not matter: they are disjoint by construction, because
/// each one names a different shape.
#[must_use]
pub fn classify(txn: &Txn, c: Option<&Note>, rust: Option<&Note>) -> Verdict {
    // ── GSP-D1 — `rpc.length = 36` for a bare 32-byte header ──────────────────────
    // Two elements that agree on everything a guest matches on (slot, seqNum, function,
    // sequence, result) and differ **only** in the declared length, with the C's being the
    // longer one. That is precisely `C:1586`.
    if let (
        Some(Note::Decoded(Observation::ElementPosted {
            slot: cs,
            seq_num: cq,
            function: cf,
            sequence: cx,
            rpc_result: cr,
            rpc_length: cl,
            ..
        })),
        Some(Note::Decoded(Observation::ElementPosted {
            slot: rs,
            seq_num: rq,
            function: rf,
            sequence: rx,
            rpc_result: rr,
            rpc_length: rl,
            ..
        })),
    ) = (c, rust)
        && (cs, cq, cf, cx, cr) == (rs, rq, rf, rx, rr)
        && cl != rl
    {
        return Verdict::Expected(row("GSP-D1"));
    }

    // ── GSP-D2 — the C posts without flow control ─────────────────────────────────
    // We refused this transaction with `QueueFull` and therefore published nothing where
    // the C published. The refusal is on the transaction, not on the position, which is
    // why this rule reads it there.
    if matches!(txn.refusal, Some(kayfabe_gsp::GspFault::QueueFull { .. })) {
        return Verdict::Expected(row("GSP-D2"));
    }

    // ── GSP-D5 — the teardown STARTCPU misclassified as a re-acquire ──────────────
    // A GSP `CPUCTL` STARTCPU on which the C re-published the queue (a tx header, an
    // element, a write pointer) and we did not, because E2 dropped the binding by value.
    if txn.reg == Some(GspReg::GspFalconCpuctl)
        && txn.write
        && txn
            .transitions
            .iter()
            .any(|t| matches!(t, kayfabe_gsp::Transition::E2 | kayfabe_gsp::Transition::E3))
        && rust.is_none()
        && matches!(
            c,
            Some(Note::Decoded(
                Observation::TxHeaderPublished { .. }
                    | Observation::ElementPosted { .. }
                    | Observation::WritePtrAdvanced { .. }
            ))
        )
    {
        return Verdict::Expected(row("GSP-D5"));
    }

    // ── GSP-D6 — continuation elements skipped ────────────────────────────────────
    // A doorbell on which the two sides acknowledged a *different* command read pointer:
    // the C advances past continuation elements without reading them, so its ack runs
    // ahead of the elements it actually consumed.
    if txn
        .reg
        .is_some_and(|r| matches!(r, GspReg::GspQueueHead(_)))
        && let (
            Some(Note::Decoded(Observation::ReadPtrAcked { value: cv, .. })),
            Some(Note::Decoded(Observation::ReadPtrAcked { value: rv, .. })),
        ) = (c, rust)
        && cv != rv
    {
        return Verdict::Expected(row("GSP-D6"));
    }

    Verdict::Unexplained
}

/// The census: every divergence in the run, classified, plus the totals.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Census {
    /// Every divergence, in transaction order.
    pub items: Vec<Classified>,
}

impl Census {
    /// How many divergences each ledger row accounted for, and how many nothing did —
    /// **before the closure limit**, which is the only region where a difference is
    /// evidence about the implementation rather than about the oracle's reach.
    #[must_use]
    pub fn by_id(&self) -> Vec<(&'static str, usize)> {
        let mut out: Vec<(&'static str, usize)> = Vec::new();
        for it in self.items.iter().filter(|i| !i.beyond_closure) {
            let id = it.verdict.id();
            match out.iter_mut().find(|(n, _)| *n == id) {
                Some((_, c)) => *c += 1,
                None => out.push((id, 1)),
            }
        }
        out.sort_unstable();
        out
    }

    /// The findings: unexplained divergences **before** the closure limit.
    #[must_use]
    pub fn unexplained(&self) -> Vec<&Classified> {
        self.items
            .iter()
            .filter(|i| i.verdict == Verdict::Unexplained && !i.beyond_closure)
            .collect()
    }

    /// Everything at or beyond the closure limit, as a count. Reported, never read as a
    /// finding.
    #[must_use]
    pub fn beyond_closure(&self) -> usize {
        self.items.iter().filter(|i| i.beyond_closure).count()
    }
}

/// Diff each transaction with [`kayfabe_trace::diff`] and classify every divergence.
///
/// ★ **The per-transaction diff is not a weaker diff.** It is the same positional
/// comparison, resynchronised at the point the *guest* next drives the device — which is
/// an input we replay rather than something we produce. Without it a single divergence
/// makes every later position differ and the census degenerates to "1".
///
/// Within a transaction the walk is: diff, classify, then re-diff the **suffix** after
/// the divergence — so every position is examined and none is skipped.
#[must_use]
pub fn census(res: &ReplayResult) -> Census {
    let mut items = Vec::new();
    for (n, t) in res.txns.iter().enumerate() {
        let ce = &res.c.events[t.c.0..t.c.1];
        let re = &res.rust.events[t.r.0..t.r.1];
        let mut base = 0usize;
        while base <= ce.len().max(re.len()) {
            let Some(d) =
                kayfabe_trace::diff(ce.get(base..).unwrap_or(&[]), re.get(base..).unwrap_or(&[]))
            else {
                break;
            };
            let at = base + d.at;
            let c_note = res
                .c
                .notes
                .get(t.c.0 + at)
                .filter(|_| at < ce.len())
                .cloned();
            let r_note = res
                .rust
                .notes
                .get(t.r.0 + at)
                .filter(|_| at < re.len())
                .cloned();
            items.push(Classified {
                txn: n,
                reg: t.reg,
                at,
                verdict: classify(t, c_note.as_ref(), r_note.as_ref()),
                c: c_note,
                rust: r_note,
                beyond_closure: res.closure_limit.is_some_and(|l| n >= l),
            });
            base = at + 1;
        }
    }
    Census { items }
}

/// Sanity: [`kayfabe_trace::Divergence`] is what [`census`] is built on, so a change to
/// its shape is a compile error here rather than a silent behaviour change.
const _: fn(&Divergence) -> usize = |d| d.at;
