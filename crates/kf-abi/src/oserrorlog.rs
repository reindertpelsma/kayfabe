// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! Axis A: the GSP **`OS_ERROR_LOG`** event — the message that makes a GSP-client guest print an
//! `Xid` line (`docs/design/V3_APP_MATRIX.md`, release item "managed memory fails loudly").
//!
//! This module owns exactly one thing — the bytes of `rpc_os_error_log_v` — for the same reason
//! [`crate::rc`] owns `rpc_rc_triggered_v17_02` (decision #2, the quarantine): the layout is an
//! NVIDIA struct, so no crate above may state a field offset for it. ⊘ It is a separate module and
//! not a second struct in `rc.rs`, because `rc.rs` documents that it owns exactly one.
//!
//! # What the guest does with it
//!
//! `_kgspRpcOsErrorLog` (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:769-806`,
//! dispatched at `:1510-1511`):
//!
//! 1. if `chid != INVALID_CHID`, it resolves `kfifoGetChidMgr(runlistId)` and the channel with
//!    that chid — the runlist matters only with per-runlist channel RAM
//!    (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_fifo.c:1449-1480`);
//! 2. it sets `pPreviousChannelInError` to that channel (or `NULL`) and calls
//!    `nvErrorLog2_va(…, "%s", errString)` — the string is **data, never a format**;
//! 3. that reaches `krcReportXid`, which prints
//!    `NVRM: Xid (PCI:…): <exceptType>, pid=<the channel's ProcessID>, name=<its client's comm>, <errString>`
//!    — or, with no channel, the same line without `pid=`/`name=`
//!    (`ogkm-580: src/nvidia/src/kernel/gpu/rc/kernel_rc.c:297-412`). The print is gated on
//!    `RmLogonRC`, which is 1 by default on Linux (`ogkm-580: kernel-open/nvidia/nv-reg.h:955`).
//!
//! ⇒ A GSP-client guest prints an RC `Xid` **only** on this event: `RC_TRIGGERED` notifies the
//! channel and prints nothing. That is why a host RC kayfabe forwards as `RC_TRIGGERED` alone left
//! the guest's dmesg empty while bare metal's shows `Xid 31`.
//!
//! # ★ The wire layout is a version seam
//!
//! [`RPC_OS_ERROR_LOG_V`] carries three layouts across the driver matrix's tags (L0 has no
//! `preemptiveRemovalPreviousXid`; L1 appends it at +268; L2 puts it at +4 and moves `runlistId`,
//! `chid` and `errString` down by 4). The encoder takes every offset **by field name** from the
//! version's own layout and refuses a version outside the matrix — a borrowed neighbour's
//! layout would print a garbage Xid number or string.

use std::fmt;

use crate::DriverVersion;
use crate::generated::matrix::{RPC_EVENTS_NV_VGPU_MSG_EVENT_OS_ERROR_LOG, RPC_OS_ERROR_LOG_V};
use crate::matrix::{LayoutError, Resolved};

/// `NV_VGPU_MSG_EVENT_OS_ERROR_LOG` — the event id, identical at every tag of the driver matrix
/// (`ogkm-580: src/nvidia/inc/kernel/vgpu/rpc_global_enums.h:258`). `everywhere_u32` makes a
/// future sweep in which it varies a BUILD failure here, never a stale id.
pub const FUNCTION: u32 = RPC_EVENTS_NV_VGPU_MSG_EVENT_OS_ERROR_LOG.everywhere_u32();

/// `INVALID_CHID` (`ogkm-580: src/nvidia/generated/g_kernel_fifo_nvoc.h:108`) — "this error has no
/// channel": the receiver skips the channel lookup and the Xid prints without `pid=`/`name=`.
///
/// ★ Hand-written, with its citation: the driver matrix does not carry it. It is the value the sender uses when it cannot PROVE which guest channel the error
/// belongs to — never a guessed chid that might name another process's channel.
pub const INVALID_CHID: u32 = 0xFFFF_FFFF;

/// `ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT` (`ogkm-580: src/common/sdk/nvidia/inc/nverror.h:49`) —
/// Xid 31, the exception a host twin's unserviced GPU MMU fault is RC'd with.
pub const ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT: u32 = 31;

/// One `OS_ERROR_LOG` event, in field terms.
///
/// ⊘ No `Default`: a zeroed event is `Xid 0` on channel 0 of runlist 0 — a well-formed message
/// about the wrong thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OsErrorLog<'a> {
    /// `exceptType` — the `ROBUST_CHANNEL_*` code; the number the guest prints as `Xid`.
    pub except_type: u32,
    /// `runlistId` — the runlist whose channel-id manager `chid` is in. Read by the receiver only
    /// with per-runlist channel RAM, and only when `chid` is not [`INVALID_CHID`].
    pub runlist_id: u32,
    /// `chid` — the guest's own channel id, or [`INVALID_CHID`].
    pub chid: u32,
    /// `errString` — the text after the Xid's attribution. At most [`ERR_STRING_MAX`] bytes, no
    /// NUL; the encoder writes the terminator.
    pub text: &'a str,
}

/// `errString[0x100]` holds a NUL-terminated string: 255 bytes of text at most.
/// ⚠ Every layout in the driver matrix has a 256-byte `errString`, and [`OsErrorLog::encode`] still reads the
/// width from the layout, so a future layout with a different width refuses instead of overrunning.
pub const ERR_STRING_MAX: usize = 255;

/// Why an `OS_ERROR_LOG` could not be encoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OsErrorLogError {
    /// The version is not in the driver matrix, the struct is absent there, or a needed field is.
    Layout(LayoutError),
    /// The text does not fit `errString` with its terminator.
    TextTooLong {
        /// The text's length in bytes.
        len: usize,
        /// The most `errString` holds at this version.
        max: usize,
    },
    /// The text carries a NUL, which would cut the guest's copy short silently.
    TextHasNul {
        /// The byte index of the first NUL.
        at: usize,
    },
}

impl fmt::Display for OsErrorLogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Layout(e) => e.fmt(f),
            Self::TextTooLong { len, max } => write!(
                f,
                "OS_ERROR_LOG text is {len} bytes; errString holds {max} plus its terminator"
            ),
            Self::TextHasNul { at } => {
                write!(f, "OS_ERROR_LOG text carries a NUL at byte {at}")
            }
        }
    }
}

impl std::error::Error for OsErrorLogError {}

impl From<LayoutError> for OsErrorLogError {
    fn from(e: LayoutError) -> Self {
        Self::Layout(e)
    }
}

impl OsErrorLog<'_> {
    /// The encoded body for a guest driver of `version` — exactly `sizeof(rpc_os_error_log_v)`
    /// at that version, zero-filled first (so `preemptiveRemovalPreviousXid`, where the layout
    /// has it, and the unused tail of `errString` are zero rather than whatever a buffer held).
    ///
    /// # Errors
    /// [`OsErrorLogError::Layout`] for a version outside the driver matrix (or a layout missing a
    /// field), and
    /// the two text refusals.
    pub fn encode(&self, version: DriverVersion) -> Result<Vec<u8>, OsErrorLogError> {
        let r = Resolved::of(&RPC_OS_ERROR_LOG_V, version)?;
        let text = self.text.as_bytes();
        if let Some(at) = text.iter().position(|b| *b == 0) {
            return Err(OsErrorLogError::TextHasNul { at });
        }
        let err_string = r.need("errString")?;
        let width = err_string.bytes().unwrap_or(0);
        let max = width.saturating_sub(1).min(ERR_STRING_MAX);
        if text.len() > max {
            return Err(OsErrorLogError::TextTooLong {
                len: text.len(),
                max,
            });
        }
        let mut buf = vec![0u8; r.size()];
        for (path, value) in [
            ("exceptType", self.except_type),
            ("runlistId", self.runlist_id),
            ("chid", self.chid),
        ] {
            let at = r.need(path)?.off();
            buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        let at = err_string.off();
        // The terminator is the zero the fill left at `at + text.len()` (`text.len() < width`).
        buf[at..at + text.len()].copy_from_slice(text);
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matrix::Unmeasured;

    fn rd(b: &[u8], o: usize) -> u32 {
        u32::from_le_bytes(b[o..o + 4].try_into().expect("4 bytes"))
    }

    fn ev(text: &str) -> OsErrorLog<'_> {
        OsErrorLog {
            except_type: ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT,
            runlist_id: 0x0000_0003,
            chid: 0x0000_0123,
            text,
        }
    }

    /// The event id is the matrix's, and it is the one the guest dispatches on.
    #[test]
    fn the_function_is_os_error_log() {
        assert_eq!(FUNCTION, 0x1006);
    }

    /// ★ One encode per driver-matrix run, at BOTH ends of the run: every field lands where THAT
    /// version's layout puts it, the size is that version's `sizeof`, and nothing else is written.
    #[test]
    fn every_matrix_layout_encodes_by_field_name() {
        let text = "kayfabe: test";
        for run in RPC_OS_ERROR_LOG_V.runs {
            let layout = run.value.expect("present at every matrix tag");
            for v in [run.first, run.last] {
                let b = ev(text).encode(v).expect("in the matrix");
                assert_eq!(b.len(), layout.size(), "{v}: sizeof");
                let at = |p: &str| layout.field(p).expect(p).off();
                assert_eq!(rd(&b, at("exceptType")), 31, "{v}");
                assert_eq!(rd(&b, at("runlistId")), 3, "{v}");
                assert_eq!(rd(&b, at("chid")), 0x123, "{v}");
                let s = at("errString");
                assert_eq!(&b[s..s + text.len()], text.as_bytes(), "{v}");
                assert_eq!(b[s + text.len()], 0, "{v}: the terminator");
                if let Some(p) = layout.field("preemptiveRemovalPreviousXid") {
                    assert_eq!(rd(&b, p.off()), 0, "{v}: no previous Xid is claimed");
                }
                let written: usize = 12 + text.len();
                let nonzero = b.iter().filter(|x| **x != 0).count();
                assert!(nonzero <= written, "{v}: only the four fields are written");
            }
        }
    }

    /// The three layouts are really different — so a version-blind encoder WOULD be wrong.
    /// Offsets as `ogkm-580: src/nvidia/generated/g_rpc-structures.h:1579-1586` declares them at
    /// 580.159.04 (L2), and the matrix's L0 (535.309.01).
    #[test]
    fn the_seam_is_where_the_matrix_says() {
        let v580 = DriverVersion::parse("580.159.04").expect("parses");
        let b = ev("x").encode(v580).expect("in the matrix");
        assert_eq!(b.len(), 272);
        assert_eq!(rd(&b, 0), 31, "exceptType @0");
        assert_eq!(rd(&b, 4), 0, "preemptiveRemovalPreviousXid @4");
        assert_eq!(rd(&b, 8), 3, "runlistId @8");
        assert_eq!(rd(&b, 12), 0x123, "chid @12");
        assert_eq!(b[16], b'x', "errString @16");
        let v535 = DriverVersion::parse("535.309.01").expect("parses");
        let b = ev("x").encode(v535).expect("in the matrix");
        assert_eq!(b.len(), 268);
        assert_eq!(rd(&b, 4), 3, "runlistId @4");
        assert_eq!(rd(&b, 8), 0x123, "chid @8");
        assert_eq!(b[12], b'x', "errString @12");
    }

    /// A version outside the matrix is refused by name — never a neighbour's layout.
    #[test]
    fn a_version_outside_the_matrix_is_refused() {
        let v = DriverVersion {
            major: 580,
            minor: 159,
            patch: 5,
        };
        assert_eq!(
            ev("x").encode(v),
            Err(OsErrorLogError::Layout(LayoutError::Unmeasured(
                Unmeasured {
                    version: v,
                    item: "rpc_os_error_log_v",
                }
            )))
        );
    }

    /// 255 bytes fit with the terminator; 256 do not, and nothing is half-written.
    #[test]
    fn the_text_bound_is_the_terminated_width() {
        let v = DriverVersion::parse("580.159.04").expect("parses");
        let ok = "k".repeat(ERR_STRING_MAX);
        let b = ev(&ok).encode(v).expect("255 bytes fit");
        assert_eq!(b[16 + 254], b'k');
        assert_eq!(
            b[16 + 255],
            0,
            "the terminator is the last byte of errString"
        );
        let long = "k".repeat(ERR_STRING_MAX + 1);
        assert_eq!(
            ev(&long).encode(v),
            Err(OsErrorLogError::TextTooLong { len: 256, max: 255 })
        );
        assert_eq!(
            ev("a\0b").encode(v),
            Err(OsErrorLogError::TextHasNul { at: 1 })
        );
    }

    /// `INVALID_CHID` reaches the wire as all-ones, so the receiver's `chid != INVALID_CHID`
    /// test skips the channel lookup.
    #[test]
    fn invalid_chid_round_trips() {
        let v = DriverVersion::parse("580.159.04").expect("parses");
        let e = OsErrorLog {
            chid: INVALID_CHID,
            runlist_id: 0,
            ..ev("x")
        };
        let b = e.encode(v).expect("in the matrix");
        assert_eq!(rd(&b, 12), 0xFFFF_FFFF);
    }
}
