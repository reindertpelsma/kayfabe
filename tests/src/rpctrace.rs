//! A reader for the **hardware** GSP-RM RPC captures in `traces/*.bin` — the recorder
//! that sits inside CPU-RM (`docs/design/rpc_trace_capture.md`, `scripts/rpctrace/`).
//!
//! # ★★★ Why this exists at all, and why it is a *refuser* first
//!
//! `rpc_trace_capture.md` §6.6 lists *"no consumer yet — nothing in `crates/` reads this
//! format"* as an open item. This is that consumer, and it is deliberately built to the
//! same posture as `scripts/rpctrace/decode_rpctrace.py`: **a trace with a hole is refused
//! outright, never reported with a warning somebody scrolls past.** One dropped record
//! shifts every later index, and every property this module's consumers assert is
//! positional in exactly that way.
//!
//! ⊘ There is no `--force` equivalent and no "best effort" parse.
//!
//! # ★★ I did not rebuild the decoder — I mirrored one that is already cross-validated
//!
//! `decode_rpctrace.py` agrees **88/88** with an independent `NV_PRINTF` instrument on the
//! same GPU (`rpc_trace_capture.md` §6.2b), and `successful_boot_demand_ga106.md` §4 is a
//! first-person record of what happens when someone writes their own parser instead: an
//! answer **80 % wrong**. So the offsets here are transcribed from
//! `scripts/rpctrace/nv_rpctrace.h` (the recorder's own header, in this repository) and
//! the agreement with the Python decoder is **asserted from outside**, against the
//! committed `traces/*.json` summaries that decoder produced —
//! `tests/tests/replay_conformance.rs`, `the_rust_reader_agrees_with_the_python_decoder`.
//!
//! ★ Two structural facts hold for **every record of all three captures** and are checked
//! here rather than assumed, because they are what says the 48-byte element header is
//! really 48 bytes:
//!
//! - `cap_len == ELEM_HDR_SIZE + rpc_len` — 1076/1076, 1180/1180, 1112/1112
//!   `[measured]` 2026-08-03.
//! - for a `GSP_RM_CONTROL` element, `rpc_len == RPC_HDR_SIZE + CTRL_HDR_SIZE + paramsSize`
//!   — 618/620, 722/724, 654/656. ⊘ The exception is **not** a decode error and is the
//!   most interesting row in the capture; see [`Control::params_truncated`].
//!
//! # ⊘ What this module deliberately does not do
//!
//! It does not classify a control DATA-vs-ACT (`rpc_trace_capture.md` §4 — that pass is
//! static, over `ogkm`, and its result is `docs/reference/gsp_control_classification.tsv`),
//! and it does not know what any control *means*. It reads bytes and refuses bad ones.

use std::collections::BTreeMap;
use std::path::PathBuf;

// ───────────────────────────── the format, from `nv_rpctrace.h` ─────────────────────────

/// `NV_RPCTRACE_FILE_MAGIC` — ASCII `"NVRT"`, little-endian (`nv_rpctrace.h:57`).
pub const FILE_MAGIC: u32 = 0x5452_564E;
/// `NV_RPCTRACE_REC_MAGIC` — ASCII `"RPCR"` (`nv_rpctrace.h:59`).
pub const REC_MAGIC: u32 = 0x5243_5052;
/// `NV_RPCTRACE_VERSION` (`nv_rpctrace.h:60`).
pub const VERSION: u32 = 1;
/// `sizeof(struct nv_rpctrace_file_hdr)` — 4×u32 + 9×u64 + 2×u32 + 32 (`nv_rpctrace.h:124`).
pub const FILE_HDR_SIZE: usize = 128;
/// `sizeof(struct nv_rpctrace_rec_hdr)` (`nv_rpctrace.h:150`).
pub const REC_HDR_SIZE: usize = 48;

/// `NV_RPCTRACE_FF_OVERFLOWED` (`nv_rpctrace.h:81`).
pub const FF_OVERFLOWED: u32 = 0x0001;
/// `NV_RPCTRACE_FF_DISABLED` (`nv_rpctrace.h:82`).
pub const FF_DISABLED: u32 = 0x0002;

/// `NV_RPCTRACE_F_CC_ENABLED` (`nv_rpctrace.h:67`).
pub const F_CC_ENABLED: u16 = 0x0001;
/// `NV_RPCTRACE_F_LEN_DISAGREE` (`nv_rpctrace.h:72`).
pub const F_LEN_DISAGREE: u16 = 0x0002;
/// `NV_RPCTRACE_F_NOT_SENT` (`nv_rpctrace.h:75`) — composed and captured, never sent.
/// ⊘ A replay must skip these.
pub const F_NOT_SENT: u16 = 0x0004;

/// CPU → GSP (`GspMsgQueueSendCommand`).
pub const DIR_REQ: u16 = 0;
/// GSP → CPU (`GspMsgQueueReceiveStatus`).
pub const DIR_REP: u16 = 1;

/// `GSP_MSG_QUEUE_ELEMENT`'s header: `authTag[16] + aad[16] + checkSum + seqNum +
/// elemCount`, with `rpc` aligned to 8 ⇒ 48
/// (`ogkm-580: src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:42-51`).
pub const ELEM_HDR_SIZE: usize = 48;
/// `sizeof(rpc_message_header_v)` (`ogkm-580: g_rpc-message-header.h:41-52`).
pub const RPC_HDR_SIZE: usize = 32;
/// `rpc_gsp_rm_control_v03_00` up to but not including `params[]`
/// (`ogkm-580: g_rpc-structures.h:1506-1520`).
pub const CTRL_HDR_SIZE: usize = 40;

/// `NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL`
/// (`ogkm-580: src/nvidia/inc/kernel/vgpu/rpc_global_enums.h`, 76 = `0x4C`).
pub const GSP_RM_CONTROL: u32 = 0x4C;

/// `NV_OK`.
pub const NV_OK: u32 = 0x0;
/// `NV_ERR_NOT_SUPPORTED` (`ogkm-580: nvstatuscodes.h:115`).
pub const NV_ERR_NOT_SUPPORTED: u32 = 0x56;
/// `NV_ERR_OBJECT_NOT_FOUND` (`ogkm-580: nvstatuscodes.h`).
pub const NV_ERR_OBJECT_NOT_FOUND: u32 = 0x57;

/// `RM_GSS_LEGACY_MASK` — bit 15 of a control id
/// (`ogkm-580: src/nvidia/interface/deprecated/rmapi_deprecated.h:41`).
pub const RM_GSS_LEGACY_MASK: u32 = 0x0000_8000;

// ───────────────────────────────────── refusals ─────────────────────────────────────

/// Every way this reader refuses a file. **One variant per reason**, so a test asserts the
/// exact refusal rather than `is_err()` (`docs/design/testing_doctrine.md` §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceError {
    /// Shorter than the file header.
    ShortFile {
        /// Bytes present.
        len: usize,
    },
    /// The file magic is not [`FILE_MAGIC`].
    BadFileMagic {
        /// What was there.
        got: u32,
    },
    /// The file declares a format version this reader does not speak.
    BadVersion {
        /// What was there.
        got: u32,
    },
    /// The recorder and this reader disagree about `sizeof(nv_rpctrace_file_hdr)`.
    FileHdrSizeMismatch {
        /// What the file declared.
        got: u32,
    },
    /// The recorder and this reader disagree about `sizeof(nv_rpctrace_rec_hdr)`.
    RecHdrSizeMismatch {
        /// What the file declared.
        got: u32,
    },
    /// `NV_RPCTRACE_FF_DISABLED` — the recorder was never armed. ⊘ A file that records
    /// nothing is not a record that nothing happened.
    RecorderDisabled,
    /// ★★★ The ring filled and refused records. The trace is a **prefix**, not a capture,
    /// and one drop shifts every later index in a positional comparison.
    RingOverflowed {
        /// Records refused.
        dropped: u64,
        /// Bytes refused.
        dropped_bytes: u64,
    },
    /// The header declares more record bytes than the file carries.
    Truncated {
        /// Bytes the header declared.
        declared: u64,
        /// Bytes present after the header.
        present: usize,
    },
    /// The file carries bytes past the records the header declares.
    TrailingGarbage {
        /// How many extra.
        extra: usize,
    },
    /// GSP→CPU reads that failed after retries: holes with no bytes behind them.
    RxFailed {
        /// How many.
        n: u64,
    },
    /// A record header does not fit in what remains.
    RecordTruncated {
        /// Record index.
        index: usize,
        /// Byte offset.
        at: usize,
    },
    /// A record's magic is not [`REC_MAGIC`] — the stream is not aligned, so everything
    /// after this point is unreadable.
    BadRecordMagic {
        /// Record index.
        index: usize,
        /// What was there.
        got: u32,
    },
    /// The recorder's own counter skipped: a record is missing.
    NonConsecutiveSeq {
        /// Where the reader was.
        index: usize,
        /// What the record claimed.
        got: u32,
    },
    /// ⊘ **The row that must not exist**: a declared length with no bytes. This is the
    /// `dlen = 0` class the whole recorder was built to make unrepresentable.
    ZeroCapLen {
        /// Record index.
        index: usize,
    },
    /// A record declares more payload than remains in the file.
    PayloadTruncated {
        /// Record index.
        index: usize,
        /// Declared payload.
        cap_len: u32,
        /// Bytes remaining.
        remaining: usize,
    },
    /// The header's record count and the stream's disagree.
    RecordCountMismatch {
        /// Header's count.
        declared: u64,
        /// Records parsed.
        parsed: usize,
    },
    /// The header's payload-byte total and the records' sum disagree.
    PayloadBytesMismatch {
        /// Header's total.
        declared: u64,
        /// Sum over records.
        summed: u64,
    },
}

impl std::fmt::Display for TraceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for TraceError {}

// ────────────────────────────────────── records ─────────────────────────────────────

/// The file header, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// `NV_VERSION_STRING` of the driver that recorded, e.g. `"580.159.04"`.
    pub driver_version: String,
    /// Ring bytes.
    pub capacity: u64,
    /// Ring bytes that follow the header.
    pub used: u64,
    /// Records in `used`.
    pub n_records: u64,
    /// Sum of `cap_len` over those records.
    pub n_payload_bytes: u64,
    /// Calls with no bytes to record. ⊘ These never became records, so those elements are
    /// **absent** — visible, never silently zero-filled.
    pub n_refused_empty: u64,
}

/// One captured message-queue element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The recorder's own monotonic counter, both directions.
    pub seq: u32,
    /// [`DIR_REQ`] or [`DIR_REP`].
    pub dir: u16,
    /// `NV_RPCTRACE_F_*`.
    pub flags: u16,
    /// `GSP_MSG_QUEUE_ELEMENT.seqNum` — the driver's per-direction counter, which restarts
    /// at 0 on every fresh bring-up. That restart is what cuts a session.
    pub elem_seq: u32,
    /// `ktime_get_ns()`, monotonic.
    pub ts_ns: u64,
    /// `rpc_message_header_v.function`.
    pub rpc_fn: u32,
    /// `rpc_message_header_v.length` — **declared**, and it may disagree with `cap_len`.
    pub rpc_len: u32,
    /// `rpc_message_header_v.rpc_result`. ⚠ On a **request** this is the `0xffffffff`
    /// sentinel `rpcWriteCommonHeader` writes before GSP answers — not an error.
    pub rpc_status: u32,
    /// The transport's own `NV_STATUS`; 0 is `NV_OK`.
    pub outcome: u32,
    /// The complete element, `cap_len` bytes. Present or the record does not exist.
    pub body: Vec<u8>,
}

/// One decoded `GSP_RM_CONTROL` element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Control {
    /// The record's `seq`, so a caller can go back to the element.
    pub seq: u32,
    /// [`DIR_REQ`] or [`DIR_REP`].
    pub dir: u16,
    /// `hClient`.
    pub h_client: u32,
    /// `hObject`.
    pub h_object: u32,
    /// The control command id.
    pub cmd: u32,
    /// The control handler's own status — the `[OUT]` field the guest reads out of the
    /// reply (`ogkm-580: rpc.c:11063-11070`). Zero in a request.
    pub status: u32,
    /// `paramsSize`, as **declared**.
    pub params_size: u32,
    /// `rmapiRpcFlags`.
    pub rmapi_rpc_flags: u32,
    /// The params bytes actually present, `min(params_size, what the element carried)`.
    pub params: Vec<u8>,
    /// ★★★ **`params_size` exceeded the bytes the element carried.**
    ///
    /// `[measured]` 2026-08-03 on all three captures: `0x2080a0a4` (a control carrying
    /// [`RM_GSS_LEGACY_MASK`] and defined **nowhere** in the open 580.159.04 or 575.51.03
    /// trees) declares `paramsSize = 67396` in an element of exactly
    /// `GSP_MSG_QUEUE_ELEMENT_SIZE_MAX = 65536` bytes — so 65448 params bytes are present
    /// against 67396 declared, in **both** directions, and a real GSP answers `NV_OK`.
    ///
    /// ⇒ A conformant emulator may **not** assume `paramsSize <= delivered bytes`. This is
    /// the *inverse* of the `dlen = 0` class: not an absent measurement, but a real driver
    /// declaring more than it sent and a real GSP serving it anyway.
    pub params_truncated: bool,
}

/// A request paired with the reply that answered it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    /// The request.
    pub req: Control,
    /// The reply.
    pub rep: Control,
    /// Which session (a complete GSP bring-up) it belongs to.
    pub session: usize,
}

/// A parsed capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trace {
    /// The file header.
    pub header: Header,
    /// Every record, in capture order.
    pub records: Vec<Record>,
}

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn u64_at(b: &[u8], off: usize) -> u64 {
    let mut x = [0u8; 8];
    x.copy_from_slice(&b[off..off + 8]);
    u64::from_le_bytes(x)
}

impl Trace {
    /// Parse a capture, or refuse it.
    ///
    /// # Errors
    ///
    /// One [`TraceError`] per reason; see that type. The order of the checks mirrors
    /// `decode_rpctrace.py::verify_and_parse`, so the two instruments refuse the same file
    /// for the same reason.
    #[allow(clippy::too_many_lines)]
    pub fn parse(blob: &[u8]) -> Result<Trace, TraceError> {
        if blob.len() < FILE_HDR_SIZE {
            return Err(TraceError::ShortFile { len: blob.len() });
        }
        let magic = u32_at(blob, 0);
        if magic != FILE_MAGIC {
            return Err(TraceError::BadFileMagic { got: magic });
        }
        let version = u32_at(blob, 4);
        if version != VERSION {
            return Err(TraceError::BadVersion { got: version });
        }
        let file_hdr_size = u32_at(blob, 8);
        if file_hdr_size as usize != FILE_HDR_SIZE {
            return Err(TraceError::FileHdrSizeMismatch { got: file_hdr_size });
        }
        let rec_hdr_size = u32_at(blob, 12);
        if rec_hdr_size as usize != REC_HDR_SIZE {
            return Err(TraceError::RecHdrSizeMismatch { got: rec_hdr_size });
        }
        let capacity = u64_at(blob, 16);
        let used = u64_at(blob, 24);
        let n_records = u64_at(blob, 32);
        let n_payload_bytes = u64_at(blob, 40);
        let n_dropped = u64_at(blob, 48);
        let n_dropped_bytes = u64_at(blob, 56);
        let n_refused_empty = u64_at(blob, 64);
        let n_rx_failed = u64_at(blob, 72);
        let flags = u32_at(blob, 88);
        let drv = &blob[96..128];
        let end = drv.iter().position(|&c| c == 0).unwrap_or(drv.len());
        let driver_version = String::from_utf8_lossy(&drv[..end]).into_owned();

        if flags & FF_DISABLED != 0 {
            return Err(TraceError::RecorderDisabled);
        }
        if n_dropped != 0 || flags & FF_OVERFLOWED != 0 {
            return Err(TraceError::RingOverflowed {
                dropped: n_dropped,
                dropped_bytes: n_dropped_bytes,
            });
        }
        let present = blob.len() - FILE_HDR_SIZE;
        if (present as u64) < used {
            return Err(TraceError::Truncated {
                declared: used,
                present,
            });
        }
        if present as u64 > used {
            return Err(TraceError::TrailingGarbage {
                extra: present - used as usize,
            });
        }
        if n_rx_failed != 0 {
            return Err(TraceError::RxFailed { n: n_rx_failed });
        }

        let mut records = Vec::new();
        let mut off = FILE_HDR_SIZE;
        let stream_end = FILE_HDR_SIZE + used as usize;
        let mut index = 0usize;
        while off < stream_end {
            if off + REC_HDR_SIZE > stream_end {
                return Err(TraceError::RecordTruncated { index, at: off });
            }
            let magic = u32_at(blob, off);
            if magic != REC_MAGIC {
                return Err(TraceError::BadRecordMagic { index, got: magic });
            }
            let dir = u16::from_le_bytes([blob[off + 4], blob[off + 5]]);
            let rflags = u16::from_le_bytes([blob[off + 6], blob[off + 7]]);
            let seq = u32_at(blob, off + 8);
            if seq as usize != index {
                return Err(TraceError::NonConsecutiveSeq { index, got: seq });
            }
            let elem_seq = u32_at(blob, off + 12);
            let ts_ns = u64_at(blob, off + 16);
            let rpc_fn = u32_at(blob, off + 24);
            let rpc_len = u32_at(blob, off + 28);
            let rpc_status = u32_at(blob, off + 32);
            let outcome = u32_at(blob, off + 36);
            let cap_len = u32_at(blob, off + 40);
            if cap_len == 0 {
                return Err(TraceError::ZeroCapLen { index });
            }
            let padded = ((cap_len as usize) + 7) & !7;
            if off + REC_HDR_SIZE + padded > stream_end {
                return Err(TraceError::PayloadTruncated {
                    index,
                    cap_len,
                    remaining: stream_end - off - REC_HDR_SIZE,
                });
            }
            let at = off + REC_HDR_SIZE;
            records.push(Record {
                seq,
                dir,
                flags: rflags,
                elem_seq,
                ts_ns,
                rpc_fn,
                rpc_len,
                rpc_status,
                outcome,
                body: blob[at..at + cap_len as usize].to_vec(),
            });
            off += REC_HDR_SIZE + padded;
            index += 1;
        }
        if records.len() as u64 != n_records {
            return Err(TraceError::RecordCountMismatch {
                declared: n_records,
                parsed: records.len(),
            });
        }
        let summed: u64 = records.iter().map(|r| r.body.len() as u64).sum();
        if summed != n_payload_bytes {
            return Err(TraceError::PayloadBytesMismatch {
                declared: n_payload_bytes,
                summed,
            });
        }

        Ok(Trace {
            header: Header {
                driver_version,
                capacity,
                used,
                n_records,
                n_payload_bytes,
                n_refused_empty,
            },
            records,
        })
    }

    /// Requests (CPU → GSP).
    #[must_use]
    pub fn requests(&self) -> Vec<&Record> {
        self.records.iter().filter(|r| r.dir == DIR_REQ).collect()
    }

    /// Replies (GSP → CPU).
    #[must_use]
    pub fn replies(&self) -> Vec<&Record> {
        self.records.iter().filter(|r| r.dir == DIR_REP).collect()
    }

    /// The distinct RPC function numbers seen.
    #[must_use]
    pub fn functions(&self) -> BTreeMap<u32, usize> {
        let mut m = BTreeMap::new();
        for r in &self.records {
            *m.entry(r.rpc_fn).or_insert(0) += 1;
        }
        m
    }

    /// ★ Cut the capture into **sessions**, where a session is one complete GSP bring-up.
    ///
    /// `capture.sh` runs `nvidia-smi` twice and persistence mode is off, so RM tears the
    /// GPU down when the last client closes and the message queue's `seqNum` restarts at
    /// 0. A session boundary is therefore a direction's `elem_seq` going **backwards** —
    /// the same rule as `decode_rpctrace.py::split_sessions`, which exists because the
    /// first reading of this capture called 479 restarts "479 retransmits".
    ///
    /// Returns, for each session, the half-open range of record indices.
    #[must_use]
    pub fn sessions(&self) -> Vec<std::ops::Range<usize>> {
        let mut out = Vec::new();
        let mut start = 0usize;
        let mut last: BTreeMap<u16, u32> = BTreeMap::new();
        for (i, r) in self.records.iter().enumerate() {
            if let Some(&prev) = last.get(&r.dir)
                && r.elem_seq < prev
            {
                out.push(start..i);
                start = i;
                last.clear();
            }
            last.insert(r.dir, r.elem_seq);
        }
        if start < self.records.len() {
            out.push(start..self.records.len());
        }
        out
    }

    /// Decode every `GSP_RM_CONTROL` element, both directions, in capture order.
    #[must_use]
    pub fn controls(&self) -> Vec<Control> {
        let mut out = Vec::new();
        for r in &self.records {
            if r.rpc_fn != GSP_RM_CONTROL {
                continue;
            }
            let at = ELEM_HDR_SIZE + RPC_HDR_SIZE;
            if r.body.len() < at + CTRL_HDR_SIZE {
                continue;
            }
            let params_size = u32_at(&r.body, at + 16);
            let params_at = at + CTRL_HDR_SIZE;
            let avail = r.body.len() - params_at;
            let take = (params_size as usize).min(avail);
            out.push(Control {
                seq: r.seq,
                dir: r.dir,
                h_client: u32_at(&r.body, at),
                h_object: u32_at(&r.body, at + 4),
                cmd: u32_at(&r.body, at + 8),
                status: u32_at(&r.body, at + 12),
                params_size,
                rmapi_rpc_flags: u32_at(&r.body, at + 20),
                params: r.body[params_at..params_at + take].to_vec(),
                params_truncated: (params_size as usize) > avail,
            });
        }
        out
    }

    /// ★★ Pair every control request with the reply that answered it, and attribute each
    /// pair to a session.
    ///
    /// The pairing is **positional** — the transport is synchronous, so the reply to a
    /// control is the next control element in the other direction. That is an assumption,
    /// so [`Trace::pair_controls`] returns `None` the moment it does not hold, and
    /// `tests/tests/replay_conformance.rs` asserts it holds for all three captures
    /// (0 alternation violations, 0 `cmd` mismatches, 0 handle mismatches across
    /// 310 / 362 / 328 pairs, `[measured]` 2026-08-03).
    #[must_use]
    pub fn pair_controls(&self) -> Option<Vec<Pair>> {
        let bounds: Vec<(u32, u32)> = self
            .sessions()
            .iter()
            .map(|r| (self.records[r.start].seq, self.records[r.end - 1].seq))
            .collect();
        let session_of = |seq: u32| bounds.iter().position(|&(a, b)| a <= seq && seq <= b);

        let ctrls = self.controls();
        let mut out = Vec::new();
        let mut pending: Option<Control> = None;
        for c in ctrls {
            if c.dir == DIR_REQ {
                if pending.is_some() {
                    return None; // two requests with no reply between them
                }
                pending = Some(c);
            } else {
                let req = pending.take()?;
                if req.cmd != c.cmd || req.h_client != c.h_client || req.h_object != c.h_object {
                    return None; // the reply is not to that request
                }
                let session = session_of(c.seq)?;
                out.push(Pair {
                    req,
                    rep: c,
                    session,
                });
            }
        }
        if pending.is_some() {
            return None; // a request nothing answered
        }
        Some(out)
    }
}

// ───────────────────────────────── the committed captures ────────────────────────────

/// One committed hardware capture: which board, which driver, and where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Board {
    /// Short tag used in assertion messages.
    pub tag: &'static str,
    /// The marketing name of the part, as `nvidia-smi` reported it on the capture run.
    pub part: &'static str,
    /// The die.
    pub arch: &'static str,
    /// The **open** kernel-module version the capture was recorded against.
    pub driver: &'static str,
    /// File name under `traces/`.
    pub file: &'static str,
}

/// ★★ The three committed captures — **two architectures and two driver versions, with
/// two of the three boards sharing a driver**, which is what lets version and architecture
/// be separated instead of confounded (`rpc_trace_capture.md` §7.2, §8).
pub const BOARDS: [Board; 3] = [
    Board {
        tag: "ga106",
        part: "RTX 3060",
        arch: "GA106",
        driver: "580.159.04",
        file: "rpctrace_ga106_boot1.bin",
    },
    Board {
        tag: "ga102",
        part: "RTX 3090",
        arch: "GA102",
        driver: "575.51.03",
        file: "ga102_boot1.bin",
    },
    Board {
        tag: "ad102",
        part: "RTX 4090",
        arch: "AD102",
        driver: "575.51.03",
        file: "ad102_boot1.bin",
    },
];

/// The repository's `traces/` directory.
#[must_use]
pub fn traces_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../traces")
}

impl Board {
    /// Where this board's capture lives.
    #[must_use]
    pub fn path(&self) -> PathBuf {
        traces_dir().join(self.file)
    }

    /// The `decode_rpctrace.py --json` summary committed beside the capture — the
    /// **other** instrument, used to cross-check this reader rather than to feed it.
    #[must_use]
    pub fn summary_path(&self) -> PathBuf {
        traces_dir().join(self.file.replace(".bin", ".json"))
    }

    /// Read and parse this board's capture.
    ///
    /// # Panics
    ///
    /// If the file is missing or refused. ⊘ Deliberately a panic and not a skip: these
    /// captures are **committed**, so their absence is a broken checkout, and a
    /// conformance suite that silently skipped its own oracle is the failure mode this
    /// project has already been bitten by four times.
    #[must_use]
    pub fn load(&self) -> Trace {
        let blob = std::fs::read(self.path())
            .unwrap_or_else(|e| panic!("{} is committed at {:?}: {e}", self.tag, self.path()));
        Trace::parse(&blob)
            .unwrap_or_else(|e| panic!("{} is a well-formed capture, got {e:?}", self.tag))
    }
}
