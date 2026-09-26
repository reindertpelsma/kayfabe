//! ★★★★★ **The refusal ledger — every non-`NV_OK` reply this FSM POSTED, keyed by what it
//! answered.**
//!
//! ## Why it exists (v3-refusals, 2026-09-26)
//!
//! `[measured]` v3-mapfix `56032c46`: kf3 answered `NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS` with
//! `0x56`; a blocked CUDA/OpenCL waiter took that as the end of its wait, `clFinish` returned 1 s
//! early, and the guest freed a buffer the host GPU was still writing (host Xid 31). The rule it
//! taught: bare metal returns non-OK **once** in 613 RM records (`0x2080012f`, in `cuInit`), so
//! **every other non-OK status this device returns is a divergence until proven harmless.**
//!
//! Before this ledger the device could not list them. The pieces each saw part of the set:
//!
//! - the unserviced ledger (`kf-rm`) sees only commands **no** policy answered;
//! - the control census (`kf-rm`) sees `GSP_RM_CONTROL` only, records the reply's status
//!   **before** a deferred host act resolves it, and was never printed at runtime;
//! - the object bridge names its first 24 refusals on stderr and then goes quiet.
//!
//! A refusal that happens between guest RM and this FSM and never reaches guest userspace still
//! changes what guest RM does next, and userspace-side recording (nvdiff) cannot see it. So the
//! ledger sits where **every** reply leaves: the two posting sites of [`crate::GspFsm`] (an
//! immediate answer, and a held reply released with its deferred act's final status). A status
//! recorded here is the status the guest actually read.
//!
//! ## ⊘ Bounded, keyed, and truthful about overflow
//!
//! Distinct rows are capped at [`REFUSAL_ROWS_MAX`] (a guest can mint distinct control ids, so an
//! unbounded set is a guest-driven allocation); `distinct` and `total` keep counting past the cap,
//! so a full table is never read as a complete one. Nothing here answers or alters a reply.

/// How many distinct `(function, detail, status)` rows are kept.
pub const REFUSAL_ROWS_MAX: usize = 128;

/// One distinct refusal: an RPC function, what it carried, and the status posted for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefusalRow {
    /// The wire function id (`NV_VGPU_MSG_FUNCTION_*`).
    pub function: u32,
    /// What the command carried, where the FSM can name it without judging it: the control id
    /// for `GSP_RM_CONTROL`, the class for `GSP_RM_ALLOC`; `None` for every other function or a
    /// header too short to decode (⊘ never `0`, which is a real class/control number).
    pub detail: Option<u32>,
    /// The `rpc_result` posted.
    pub status: u32,
    /// How many times this exact row was posted.
    pub count: u64,
    /// `rpc.sequence` of the first posting, to tie the row to a recorder's stream.
    pub first_sequence: u32,
}

/// The ledger. Owned by the FSM; read by the device for its report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefusalLedger {
    rows: Vec<RefusalRow>,
    /// Distinct rows seen — the truth past [`REFUSAL_ROWS_MAX`].
    distinct: u64,
    /// Every refusal posted.
    total: u64,
    /// Rows first seen since the last [`RefusalLedger::take_fresh`], for a one-line log each.
    fresh: Vec<RefusalRow>,
}

impl RefusalLedger {
    /// An empty ledger.
    #[must_use]
    pub fn new() -> RefusalLedger {
        RefusalLedger::default()
    }

    /// Record one posted reply. `status == 0` is not a refusal and is ignored.
    pub fn note(&mut self, function: u32, detail: Option<u32>, status: u32, sequence: u32) {
        if status == 0 {
            return;
        }
        self.total += 1;
        if let Some(r) = self
            .rows
            .iter_mut()
            .find(|r| r.function == function && r.detail == detail && r.status == status)
        {
            r.count += 1;
            return;
        }
        self.distinct += 1;
        let row = RefusalRow { function, detail, status, count: 1, first_sequence: sequence };
        if self.rows.len() < REFUSAL_ROWS_MAX {
            self.rows.push(row);
        }
        // ⊘ `fresh` is drained by the device on every service pass; it is bounded by the same cap
        // so a guest minting ids faster than the drainer runs cannot grow it either.
        if self.fresh.len() < REFUSAL_ROWS_MAX {
            self.fresh.push(row);
        }
    }

    /// The rows, first-seen order, capped at [`REFUSAL_ROWS_MAX`].
    #[must_use]
    pub fn rows(&self) -> &[RefusalRow] {
        &self.rows
    }

    /// Distinct rows seen, including any past the cap.
    #[must_use]
    pub fn distinct(&self) -> u64 {
        self.distinct
    }

    /// Every refusal posted.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.total
    }

    /// The rows first seen since the previous call.
    pub fn take_fresh(&mut self) -> Vec<RefusalRow> {
        std::mem::take(&mut self.fresh)
    }

    /// One compact line: `fn/detail=status×count`, rows in first-seen order.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut s = format!("total={} distinct={} [", self.total, self.distinct);
        for (i, r) in self.rows.iter().enumerate() {
            if i > 0 {
                s.push(' ');
            }
            s.push_str(&r.key());
            s.push_str(&format!("x{}", r.count));
        }
        s.push(']');
        s
    }
}

impl RefusalRow {
    /// `fn<id>/<detail>=<status>` — the key as printed.
    #[must_use]
    pub fn key(&self) -> String {
        match self.detail {
            Some(d) => format!("fn{}/{:#010x}={:#x}", self.function, d, self.status),
            None => format!("fn{}={:#x}", self.function, self.status),
        }
    }
}

kf_util::assert_send_sync!(RefusalRow, RefusalLedger);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_is_not_a_refusal() {
        let mut l = RefusalLedger::new();
        l.note(76, Some(0x2080_1702), 0, 1);
        assert_eq!((l.total(), l.distinct(), l.rows().len()), (0, 0, 0));
        assert!(l.take_fresh().is_empty());
    }

    #[test]
    fn rows_are_keyed_on_function_detail_and_status() {
        let mut l = RefusalLedger::new();
        l.note(76, Some(0x2080_1702), 0x56, 1);
        l.note(76, Some(0x2080_1702), 0x56, 2);
        l.note(76, Some(0x2080_1702), 0x1f, 3); // same control, another status: another row
        l.note(76, Some(0x2080_0122), 0x56, 4);
        l.note(103, Some(0xc86f), 0x56, 5);
        l.note(21, None, 0x56, 6);
        assert_eq!(l.total(), 6);
        assert_eq!(l.distinct(), 5);
        assert_eq!(l.rows()[0].count, 2);
        assert_eq!(l.rows()[0].first_sequence, 1);
        let fresh = l.take_fresh();
        assert_eq!(fresh.len(), 5, "each distinct row is fresh exactly once");
        assert!(l.take_fresh().is_empty());
        l.note(76, Some(0x2080_1702), 0x56, 7);
        assert!(l.take_fresh().is_empty(), "a repeat is not fresh");
        assert_eq!(
            l.summary(),
            "total=7 distinct=5 [fn76/0x20801702=0x56x3 fn76/0x20801702=0x1fx1 fn76/0x20800122=0x56x1 \
             fn103/0x0000c86f=0x56x1 fn21=0x56x1]"
        );
    }

    #[test]
    fn the_cap_bounds_storage_but_not_the_counts() {
        let mut l = RefusalLedger::new();
        for i in 0..(REFUSAL_ROWS_MAX as u32 + 10) {
            l.note(76, Some(i), 0x56, i);
        }
        assert_eq!(l.rows().len(), REFUSAL_ROWS_MAX);
        assert_eq!(l.distinct(), REFUSAL_ROWS_MAX as u64 + 10, "distinct is the truth past the cap");
        assert_eq!(l.take_fresh().len(), REFUSAL_ROWS_MAX);
    }
}
