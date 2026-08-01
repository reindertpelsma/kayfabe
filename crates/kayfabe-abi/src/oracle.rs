//! ★★★ The C artifact's captured control table, and the one move it does **not** support:
//! reading a row it recorded **empty** as *"hardware answers zero"*.
//!
//! # The defect this module exists to make unrepeatable
//!
//! `nvidia-gpu-passthrough/src/qemu/mode2_initctrl_ga106.h` is labelled *"GA106 init
//! `GSP_RM_CONTROL` responses (real, captured from host)"* and is the closest thing this
//! port has to an answer sheet. Each row is
//! `{ cmd, status, psize, dlen, ctl_<cmd>[] }`, where `psize` is the reply's size **on the
//! wire** and `dlen` is *how many bytes the recorder kept*.
//!
//! **56 rows. 11 of them — 19.6% — carry `dlen = 0`.** For those the `ctl_` array is
//! literally `{ }`, and the obvious reading is that the reply body was `psize` bytes of
//! nothing. That reading built a wall: `0x20802a08`'s empty row decodes to a fault-method
//! buffer size of **0**, which is a buffer RM allocates and *hardware DMAs into*
//! ([`crate::fmbsize`]).
//!
//! ★★ It survived four rungs because it was **cited**. `crates/kayfabe-device/src/sweep.rs`
//! is gated on every triage row naming a source, and the row named `C:
//! mode2_initctrl_ga106.h:6233` — exactly as demanded. ⊘ **Citing the oracle is not the
//! oracle being right.** A citation gate checks a claim is *sourced*; it can never check
//! that the source says what the claim says it says.
//!
//! # ★★★ The class, MEASURED — all eleven, not a sample
//!
//! `[measured]` 2026-08-01 on an NVIDIA GeForce RTX 3060 (GA106),
//! `GPU-e28d7776-e4f9-704b-d392-d46f187343f8`, vast instance `46494693`, host driver open
//! **580.159.04** — `research_clones/ogkm-580.159.04` (tag `b81d58e`) rebuilt with a chunked
//! whole-body print at `rpcRmApiControl_GSP`'s reply, `RmInitAdapter` driven by opening
//! `/dev/nvidia0`, and widened with `nvidia-smi -q`, which is what reaches `0x20800a4c`.
//! Whole bodies at `traces/real_ga106/rpc_bodies_real_ga106.txt`.
//!
//! | control | C's row | real GA106 | verdict |
//! |---|---|---|---|
//! | `0x20802a06` | `psize 4, dlen 0` | `10 00 00 00` | **contradicted** |
//! | `0x2080017e` | `psize 8, dlen 0` | `00 00 00 02 00 00 00 00` | **contradicted** |
//! | `0x20800af3` | `psize 2, dlen 0` | `01 01` | **contradicted** |
//! | `0x20802a08` | `psize 4, dlen 0` | `00 50 00 00` | **contradicted** |
//! | `0xa06f0103` | `psize 3, dlen 0` | `01 00 00` | **contradicted** |
//! | `0xa06f0104` | `psize 4, dlen 0` | `0b 00 00 00` | **contradicted** |
//! | `0x20800a4b` | `psize 4, dlen 0` | `00 00 01 04` | **contradicted** |
//! | `0x20800aac` | `psize 4, dlen 0` | `00 00 01 00` | **contradicted** |
//! | `0x20800a6c` | `psize 4, dlen 0` | `31 00 00 00` **and** `11 00 00 00` | **contradicted**, and *not a constant* |
//! | `0x20800a4c` | `psize 4, dlen 0` | `00 00 00 00` | ⚠ coincides |
//! | `0x20800a70` | `psize 0, dlen 0` | `<empty>` | ⚠ coincides |
//!
//! **Nine of eleven are wrong.** And the two that are not are the important ones:
//!
//! - `0x20800a70` has `psize = 0`. There is no body to have failed to capture, so the row is
//!   consistent for a reason a reader *can* check without hardware — and that reason is the
//!   whole predicate below.
//! - `0x20800a4c` (`INTERNAL_GPU_GET_SMC_MODE`) genuinely answers zero on this part, because
//!   SMC is disabled. ★★★ **Nothing about the row says so.** It is byte-identical in the
//!   capture to the nine that are wrong. It is the strongest available demonstration that an
//!   empty capture is evidence of *nothing*: here it happens to coincide with the truth, and
//!   a policy of believing it would have been right once and wrong nine times for the same
//!   reason.
//!
//! # The rule, and why it is a predicate rather than a list
//!
//! [`captured_row_evidence`] refuses **`psize > 0 && dlen == 0`** — not "these eleven ids".
//! A list would be a smaller true statement that goes stale the moment a 57th row appears
//! (memory: *gates quantified over a LIST*). The list in [`EMPTY_CAPTURE_ROWS`] exists for a
//! different job: it carries what hardware actually said, so that a citation of one of these
//! rows can be made to carry **content** rather than an address.
//!
//! # ⊘ What this does NOT say
//!
//! - ⊘ **It does not demote the C artifact.** Rows it *did* capture are corroborated by the
//!   same transcript, byte for byte and including all 3712 bytes of `0x20800a2a`
//!   ([`crate::grinfo`]). It remains the only implementation a real driver has accepted end
//!   to end.
//! - ⊘ **It says nothing about TRUNCATED rows** — `dlen > 0` but `dlen < psize`, of which
//!   `0x20800a22` (16 376 of 34 592) is the largest. That is a *different* capture defect,
//!   it is named in [`crate::grstatic`], and no hardware body has been taken for it. The
//!   recipe is now proven and committed; the measurement has not been made.
//! - ⊘ **It is one part, one driver version, one boot.**

/// One row of `mode2_initctrl_ga106.h` whose reply body was **never captured**.
///
/// ★ `real` is the point of the struct. Without it a reader who wants to know what this
/// control answers has to go to hardware; with it the citation carries the content, which
/// is exactly what the `C:`-citation gate cannot check on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyCaptureRow {
    /// The control id.
    pub cmd: u32,
    /// The reply size on the wire, from the C's own row.
    pub psize: usize,
    /// The line of `src/qemu/mode2_initctrl_ga106.h` the row sits on.
    pub c_line: u32,
    /// ★★★ What a **real GA106** answered — `[measured]` 2026-08-01, see the module docs.
    /// Empty only for `0x20800a70`, whose `psize` is genuinely zero.
    pub real: &'static [u8],
    /// `true` when the empty capture happens to agree with hardware anyway. ⚠ Two rows, and
    /// neither is distinguishable from the nine wrong ones *by anything in the capture*.
    pub coincides: bool,
}

/// Every `dlen = 0` row of the C artifact's GA106 table, with what hardware said.
///
/// ⊘ This is **not** the rule — [`captured_row_evidence`] is. This is the evidence, kept so
/// that a triage row citing one of these can quote a value instead of an address.
pub const EMPTY_CAPTURE_ROWS: &[EmptyCaptureRow] = &[
    EmptyCaptureRow {
        cmd: 0x2080_2a06,
        psize: 4,
        c_line: 6213,
        real: &[0x10, 0x00, 0x00, 0x00],
        coincides: false,
    },
    EmptyCaptureRow {
        cmd: 0x2080_017e,
        psize: 8,
        c_line: 6215,
        real: &[0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00],
        coincides: false,
    },
    EmptyCaptureRow {
        cmd: 0x2080_0af3,
        psize: 2,
        c_line: 6216,
        real: &[0x01, 0x01],
        coincides: false,
    },
    EmptyCaptureRow {
        cmd: 0x2080_2a08,
        psize: 4,
        c_line: 6233,
        real: &[0x00, 0x50, 0x00, 0x00],
        coincides: false,
    },
    EmptyCaptureRow {
        cmd: 0xa06f_0103,
        psize: 3,
        c_line: 6234,
        real: &[0x01, 0x00, 0x00],
        coincides: false,
    },
    EmptyCaptureRow {
        cmd: 0xa06f_0104,
        psize: 4,
        c_line: 6236,
        real: &[0x0b, 0x00, 0x00, 0x00],
        coincides: false,
    },
    // ⚠ Reached only by a CLIENT query (`nvidia-smi -q` → `NV2080_CTRL_GPU_INFO_INDEX_-
    // GPU_SMC_MODE`, `ogkm-580: subdevice_ctrl_gpu_kernel.c:232-266`), never by
    // `RmInitAdapter`. It answers zero because SMC is disabled on this part — a fact about
    // the silicon, not about the capture.
    EmptyCaptureRow {
        cmd: 0x2080_0a4c,
        psize: 4,
        c_line: 6243,
        real: &[0x00, 0x00, 0x00, 0x00],
        coincides: true,
    },
    // ⚠ `psize = 0`: the only row in the table for which "empty" is checkable without
    // hardware, and the reason the predicate keys on `psize` rather than on an id list.
    EmptyCaptureRow {
        cmd: 0x2080_0a70,
        psize: 0,
        c_line: 6244,
        real: &[],
        coincides: true,
    },
    // ⚠ NOT A CONSTANT. The same boot answers `0x31` three times during adapter init and
    // `0x11` afterwards — so even a correct single capture of this row would have been a
    // statement about one moment. See `crate::l2evict`.
    EmptyCaptureRow {
        cmd: 0x2080_0a6c,
        psize: 4,
        c_line: 6245,
        real: &[0x31, 0x00, 0x00, 0x00],
        coincides: false,
    },
    EmptyCaptureRow {
        cmd: 0x2080_0a4b,
        psize: 4,
        c_line: 6257,
        real: &[0x00, 0x00, 0x01, 0x04],
        coincides: false,
    },
    EmptyCaptureRow {
        cmd: 0x2080_0aac,
        psize: 4,
        c_line: 6258,
        real: &[0x00, 0x00, 0x01, 0x00],
        coincides: false,
    },
];

/// The number of rows in `mode2_initctrl_ga106.h`, `[measured]` by counting the table at
/// `nvidia-gpu-passthrough` rev `8baf4f2` on 2026-08-01. Pinned so that
/// [`EMPTY_CAPTURE_ROWS`]'s share is a stated fraction rather than a bare count.
pub const CAPTURED_ROWS_TOTAL: usize = 56;

/// What a captured row is evidence **of**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapturedEvidence {
    /// The reply is genuinely empty, because the control's `psize` is zero. Checkable from
    /// the row alone.
    NoBodyExists,
    /// The recorder kept `kept` of `psize` bytes. ⚠ `kept < psize` is a **truncation**, and
    /// the caller is told the number rather than being handed a padded body.
    Body {
        /// Bytes the recorder kept.
        kept: usize,
        /// Bytes the reply had on the wire.
        psize: usize,
    },
}

/// Why a captured row cannot be read as a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleRowError {
    /// ★★★ `psize > 0` and `dlen == 0`: the reply had a body and **the recorder kept none
    /// of it**. The row is a record that a control was asked, and nothing more.
    ///
    /// ⊘ It is emphatically *not* `Ok(vec![0; psize])`. Nine of the eleven rows in this
    /// state are contradicted by hardware, and the two that are not are indistinguishable
    /// from the nine — so a zero-filled body is a fabricated reply wearing the oracle's
    /// name.
    BodyNeverCaptured {
        /// The control whose body is missing.
        cmd: u32,
        /// How many bytes the reply had that were not kept.
        psize: usize,
    },
    /// `dlen > psize` — the row is internally inconsistent and describes no reply.
    KeptMoreThanExists {
        /// The control.
        cmd: u32,
        /// Bytes claimed kept.
        dlen: usize,
        /// Bytes the reply is said to have.
        psize: usize,
    },
}

/// ★★★ **The rule.** Decide what a row of the C artifact's captured table is evidence of.
///
/// This is the named refusal the four-rung `0x20802a08` defect needed and did not have. It
/// is a predicate on `(psize, dlen)`, not a lookup in [`EMPTY_CAPTURE_ROWS`], so a row added
/// to the table tomorrow is covered tomorrow.
///
/// # Errors
/// [`OracleRowError::BodyNeverCaptured`] when `psize > 0 && dlen == 0` — an empty capture is
/// evidence of nothing, never evidence of emptiness.
/// [`OracleRowError::KeptMoreThanExists`] when `dlen > psize`.
pub const fn captured_row_evidence(
    cmd: u32,
    psize: usize,
    dlen: usize,
) -> Result<CapturedEvidence, OracleRowError> {
    if dlen > psize {
        return Err(OracleRowError::KeptMoreThanExists { cmd, dlen, psize });
    }
    if psize == 0 {
        return Ok(CapturedEvidence::NoBodyExists);
    }
    if dlen == 0 {
        return Err(OracleRowError::BodyNeverCaptured { cmd, psize });
    }
    Ok(CapturedEvidence::Body { kept: dlen, psize })
}

/// The row for `cmd`, if the C artifact recorded it with no body at all.
#[must_use]
pub fn empty_capture_row(cmd: u32) -> Option<&'static EmptyCaptureRow> {
    EMPTY_CAPTURE_ROWS.iter().find(|r| r.cmd == cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_eleven_are_all_here_and_the_share_is_stated() {
        assert_eq!(EMPTY_CAPTURE_ROWS.len(), 11);
        assert_eq!(CAPTURED_ROWS_TOTAL, 56);
        // 11/56 — stated as the multiplication so the fraction cannot drift from the count.
        assert!(EMPTY_CAPTURE_ROWS.len() * 5 < CAPTURED_ROWS_TOTAL);
        let mut ids: Vec<u32> = EMPTY_CAPTURE_ROWS.iter().map(|r| r.cmd).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), EMPTY_CAPTURE_ROWS.len(), "an id appears once");
    }

    #[test]
    fn every_empty_capture_row_is_refused_by_name_unless_no_body_exists() {
        // ★★ Quantified over the whole list, so a row added without a decision fails here.
        for r in EMPTY_CAPTURE_ROWS {
            let got = captured_row_evidence(r.cmd, r.psize, 0);
            if r.psize == 0 {
                assert_eq!(got, Ok(CapturedEvidence::NoBodyExists), "{:#x}", r.cmd);
            } else {
                assert_eq!(
                    got,
                    Err(OracleRowError::BodyNeverCaptured {
                        cmd: r.cmd,
                        psize: r.psize
                    }),
                    "{:#x} must be UNMEASURED, not zero",
                    r.cmd
                );
            }
        }
    }

    #[test]
    fn the_refusal_is_not_the_only_witness_that_zeros_would_have_been_wrong() {
        // ★★★ Deliberately does NOT go through `captured_row_evidence`. If the only
        // evidence that "the empty row is wrong" were the function that refuses empty rows,
        // then "refused" and "answered zeros" would be indistinguishable and a mutation that
        // deleted the refusal would survive. This inspects the bodies `[measured]`
        // 2026-08-01 on a real RTX 3060 directly — the ones
        // `tests/real_ga106_bodies.rs::every_claimed_hardware_body_is_the_body_in_the_trace`
        // pins against `traces/real_ga106/rpc_bodies_real_ga106.txt`.
        let contradicted: Vec<u32> = EMPTY_CAPTURE_ROWS
            .iter()
            .filter(|r| !r.real.iter().all(|&b| b == 0) || r.real.is_empty() && r.psize > 0)
            .map(|r| r.cmd)
            .collect();
        assert_eq!(
            contradicted.len(),
            9,
            "nine of the eleven empty rows are contradicted by a non-zero hardware body: {contradicted:#x?}"
        );
        // …and the two survivors are exactly the ones flagged as coincidences, each for a
        // reason recorded in the module docs.
        let coinciding: Vec<u32> = EMPTY_CAPTURE_ROWS
            .iter()
            .filter(|r| r.coincides)
            .map(|r| r.cmd)
            .collect();
        assert_eq!(coinciding, vec![0x2080_0a4c, 0x2080_0a70]);
        for r in EMPTY_CAPTURE_ROWS {
            assert_eq!(
                r.coincides,
                r.real.iter().all(|&b| b == 0),
                "{:#x}: `coincides` must be derivable from the measured body, not asserted",
                r.cmd
            );
            assert_eq!(r.real.len(), r.psize, "{:#x}: body is psize bytes", r.cmd);
        }
    }

    #[test]
    fn a_truncated_row_is_a_body_with_its_truncation_stated() {
        // `0x20800a22`: `psize 34592, dlen 16376` — a real row, a different defect, and it
        // must NOT be refused. What it must not do is claim to be 34 592 bytes.
        assert_eq!(
            captured_row_evidence(0x2080_0a22, 34592, 16376),
            Ok(CapturedEvidence::Body {
                kept: 16376,
                psize: 34592
            })
        );
    }

    #[test]
    fn a_full_row_is_evidence_and_0x20800a2a_is_one() {
        // ★ The other half of the class result: `0x20800a2a` carries `dlen == psize` and is
        // byte-identical to hardware for all 3712 bytes. See `crate::grinfo`.
        assert_eq!(
            captured_row_evidence(0x2080_0a2a, 3712, 3712),
            Ok(CapturedEvidence::Body {
                kept: 3712,
                psize: 3712
            })
        );
        assert!(empty_capture_row(0x2080_0a2a).is_none());
    }

    #[test]
    fn an_inconsistent_row_describes_no_reply() {
        assert_eq!(
            captured_row_evidence(0xdead_beef, 4, 8),
            Err(OracleRowError::KeptMoreThanExists {
                cmd: 0xdead_beef,
                dlen: 8,
                psize: 4
            })
        );
    }
}
