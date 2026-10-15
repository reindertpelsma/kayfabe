// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! Bounded, metadata-only sequence export for native/observer comparison.
//! Order is the capture's observation order, not a global GPU execution order.
//! Physical queue addresses become local instance numbers; handles, timestamps,
//! payload bytes and board/person identifiers are never emitted. A published
//! field layout explains an encoding, not a reply's correctness.

use crate::analyze::In;
use crate::capture::Dir;
use kf_gsp::element::peek_seq_num;
use kf_gsp::{GspAbi, decode_message, peek_len};
use std::collections::BTreeMap;
use std::io::{self, Write};

/// Fixed bounds: never silently truncate an export and call it complete.
pub const MAX_ROWS: usize = 100_000;
const MAX_INSTANCES: usize = 1024;

/// Streaming exporter; the caller must also run the completeness oracle.
pub struct Sequence<W> {
    abi: GspAbi,
    writer: W,
    instances: BTreeMap<(usize, u64, u64), usize>,
    rows: usize,
    max_rows: usize,
}

impl<W: Write> Sequence<W> {
    /// Emit the schema header. A smaller row budget is useful for bounded probes.
    pub fn new(abi: GspAbi, mut writer: W, max_rows: usize) -> io::Result<Self> {
        if max_rows == 0 || max_rows > MAX_ROWS {
            return Err(io::Error::other("invalid sequence row bound"));
        }
        writeln!(
            writer,
            "source\tinstance\tordinal\tdir\telement_sequence\tfunction\trpc_result\trpc_result_private\trpc_sequence\tkind\tcontrol_or_class\tcontrol_status\tdeclared_params_bytes\tdecode"
        )?;
        Ok(Self {
            abi,
            writer,
            instances: BTreeMap::new(),
            rows: 0,
            max_rows,
        })
    }

    /// One raw message in observed order, with no raw bytes in the output.
    /// Parse errors remain visible rows; the analyzer separately rejects them.
    pub fn message(&mut self, source: usize, m: &In<'_>) -> io::Result<()> {
        if self.rows >= self.max_rows {
            return Err(io::Error::other("sequence row limit exceeded"));
        }
        let key = (source, m.a, m.b);
        let next = self.instances.len() + 1;
        if !self.instances.contains_key(&key) && next > MAX_INSTANCES {
            return Err(io::Error::other("sequence instance limit exceeded"));
        }
        let instance = *self.instances.entry(key).or_insert(next);
        self.rows += 1;
        write!(
            self.writer,
            "{}\t{instance}\t{}\t{}\t",
            source + 1,
            m.ordinal,
            if m.dir == Dir::Cmd { "cmd" } else { "status" }
        )?;
        let decoded = peek_seq_num(&self.abi.element, m.bytes).and_then(|seq| {
            let len = peek_len(
                &self.abi.element,
                m.bytes,
                m.element_size,
                self.abi.element_size_max,
            )?;
            decode_message(&self.abi.element, m.bytes, len, seq, &self.abi.driver)
        });
        let Ok(rpc) = decoded else {
            return writeln!(self.writer, "-\t-\t-\t-\t-\tunknown\t-\t-\t-\tbad_frame");
        };
        let e = rpc.envelope;
        write!(
            self.writer,
            "{}\t{:#x}\t{:#x}\t{:#x}\t{}\t",
            rpc.seq_num, e.function, e.rpc_result, e.rpc_result_private, e.sequence
        )?;
        if e.function == self.abi.rpc.codes.gsp_rm_control {
            if let Ok(req) = self.abi.driver.decode_rpc_control(&rpc.payload) {
                let off = self.abi.driver.rm_control_wire().status_off;
                let status = rpc
                    .payload
                    .get(off..off + 4)
                    .and_then(|b| b.try_into().ok())
                    .map(u32::from_le_bytes);
                if let Some(status) = status {
                    return writeln!(
                        self.writer,
                        "control\t{:#x}\t{status:#x}\t{}\tok",
                        req.cmd, req.params_size
                    );
                }
            }
            writeln!(self.writer, "control\t-\t-\t-\tbad_control")
        } else if e.function == self.abi.rpc.codes.gsp_rm_alloc {
            if let Ok(req) = self.abi.driver.decode_rpc_alloc(&rpc.payload) {
                // The existing alloc view exposes no status field. Do not
                // duplicate its wire offset here; envelope result stays visible.
                writeln!(
                    self.writer,
                    "alloc\t{:#x}\t-\t{}\tok",
                    req.class, req.params_size
                )
            } else {
                writeln!(self.writer, "alloc\t-\t-\t-\tbad_alloc")
            }
        } else {
            writeln!(
                self.writer,
                "{}\t-\t-\t-\tok",
                if e.function == self.abi.rpc.codes.continuation_record {
                    "continuation"
                } else {
                    "other"
                }
            )
        }
    }

    /// Flush all metadata; an I/O failure is a failed export.
    pub fn finish(&mut self) -> io::Result<()> {
        self.writer.flush()
    }

    /// The writer, for consumers that own the resulting buffer.
    pub fn into_inner(self) -> W {
        self.writer
    }
}
