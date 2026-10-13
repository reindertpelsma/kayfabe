//! KFGSPCAP capture stream to messages: framing checks, ring-entry reassembly, loss handling.
//!
//! The capture module copies raw ring entries and never parses them; this is where entries
//! become messages. Per `(queue_id, dir)` the reassembler keeps the bytes of a message whose
//! entries have not all arrived. A message's entry count is read the way the driver reads it:
//! `elemCount` where the layout has one, otherwise derived from the declared length
//! (`kf_gsp::element::{peek_elem_count, peek_len}`).
//!
//! Assumption (inferred, checkable on the first real capture): a message never straddles the
//! end of the ring except by continuing at entry 0, so consecutive Elements records of a queue
//! are contiguous modulo `msgCount`. A capture where that is false shows up as
//! `discontinuous_entries`, loudly, rather than as silently reassembled garbage.

use crate::analyze::{Analyzer, AnomalyKind, In};
use crate::capture::{CaptureError, Dir, Kind, Reader, Record};
use kf_gsp::{TxHeader, max_elements, peek_elem_count, peek_len};
use std::collections::HashMap;
use std::io::Read;

#[derive(Debug)]
struct Q {
    entry_size: usize,
    ring: u32,
    next_entry: Option<u32>,
    buf: Vec<u8>,
    after_loss: bool,
}

/// Read a KFGSPCAP stream from `r` into the analyzer. `label` names the source.
///
/// # Errors
///
/// A [`CaptureError`] only when the file header itself is unreadable; everything after it
/// becomes anomalies.
pub fn read<R: Read>(an: &mut Analyzer, label: &str, r: R) -> Result<(), CaptureError> {
    let mut rd = Reader::new(r)?;
    let h = rd.header().clone();
    an.begin_source(label, "kfgspcap");
    if h.scrubbed {
        an.mark_scrubbed();
    }
    an.fact("os", format!("{:?}", h.os));
    an.fact("module_version", h.module_version);
    an.fact("pci_vendor_id", format!("{:#06x}", h.pci_vendor_id));
    an.fact("pci_device_id", format!("{:#06x}", h.pci_device_id));
    an.fact("pci_revision_id", format!("{:#04x}", h.pci_revision_id));
    an.fact("driver_version", &h.driver_version);
    an.fact("decode_tag", &h.decode_tag);
    an.fact("ts_hz", h.ts_hz);

    let mut qs: HashMap<(u16, Dir), Q> = HashMap::new();
    let mut expected_counter: u32 = 0;
    let mut last_ts: Option<u64> = None;
    let mut ended = false;
    let mut records = 0u64;
    loop {
        let rec = match rd.next_record() {
            Ok(Some(r)) => r,
            Ok(None) => break,
            Err(e) => {
                an.anomaly(
                    AnomalyKind::StreamTruncated,
                    format!("after {records} records: {e}"),
                );
                break;
            }
        };
        records += 1;
        an.record_read();
        let hd = rec.header;
        if ended {
            an.anomaly(
                AnomalyKind::EndMismatch,
                format!("record {} follows the End record", hd.counter),
            );
        }
        if hd.counter != expected_counter {
            an.anomaly(
                AnomalyKind::CounterGap,
                format!(
                    "expected record counter {expected_counter}, got {}",
                    hd.counter
                ),
            );
        }
        expected_counter = hd.counter.wrapping_add(1);
        if let Some(t) = last_ts
            && hd.ts < t
        {
            an.anomaly(
                AnomalyKind::TimestampRegress,
                format!("record {}: ts {} after {t}", hd.counter, hd.ts),
            );
        }
        last_ts = Some(hd.ts);
        match hd.kind {
            Kind::End => {
                ended = true;
                if u64::from(hd.entry_count) != records - 1 {
                    an.anomaly(
                        AnomalyKind::EndMismatch,
                        format!(
                            "End says {} records were written, {} were read before it",
                            hd.entry_count,
                            records - 1
                        ),
                    );
                }
            }
            Kind::Bind => bind(an, &mut qs, &rec),
            Kind::Elements => elements(an, &mut qs, &rec),
            k => {
                let unknown = hd.entry_count == crate::capture::UNKNOWN_COUNT;
                an.anomaly(
                    AnomalyKind::CaptureLoss,
                    format!(
                        "record {} {k:?} q{} {:?} at entry {}: {} entries lost",
                        hd.counter,
                        hd.queue_id,
                        hd.dir,
                        hd.first_entry,
                        if unknown {
                            "unknown number of".to_string()
                        } else {
                            hd.entry_count.to_string()
                        }
                    ),
                );
                an.declare_loss(u64::from(hd.queue_id), 0, hd.dir);
                if let Some(q) = qs.get_mut(&(hd.queue_id, hd.dir)) {
                    q.after_loss = true;
                    q.buf.clear();
                }
            }
        }
    }
    if !ended {
        an.anomaly(
            AnomalyKind::EndMissing,
            format!("no End record after {records} records (the module did not close cleanly)"),
        );
    }
    for ((qid, dir), q) in &qs {
        if !q.buf.is_empty() {
            an.anomaly(
                AnomalyKind::TrailingBytes,
                format!(
                    "q{qid} {dir:?}: {} bytes of a message that never completed at end of stream",
                    q.buf.len()
                ),
            );
        }
    }
    Ok(())
}

/// How many entries the message starting at `first` (one entry) occupies, read the way the
/// driver reads it: `elemCount` where the layout has one, else derived from the declared length.
///
/// # Errors
///
/// `(count, limit)` when the entry does not start a plausible message (count 0, or more entries
/// than the guest's staging buffer holds); 
pub(crate) fn message_entries(
    layout: &kf_gsp::ElementLayout,
    size_max: u32,
    first: &[u8],
    es: usize,
) -> Result<u32, (u32, u32)> {
    let count = match peek_elem_count(layout, first) {
        Ok(Some(n)) => n,
        _ => match peek_len(layout, first, es as u32, size_max) {
            Ok(l) => l.elements(),
            Err(_) => 0,
        },
    };
    let limit = max_elements(es as u32, size_max);
    if count == 0 || count > limit {
        Err((count, limit))
    } else {
        Ok(count)
    }
}

fn bind(an: &mut Analyzer, qs: &mut HashMap<(u16, Dir), Q>, rec: &Record) {
    let hd = rec.header;
    let t = match TxHeader::decode(&rec.payload) {
        Ok(t) if rec.payload.len() == TxHeader::BYTES => t,
        other => {
            an.anomaly(
                AnomalyKind::BadBind,
                format!(
                    "record {}: bind payload is {} bytes ({other:?})",
                    hd.counter,
                    rec.payload.len()
                ),
            );
            return;
        }
    };
    if t.msg_size < 16 || t.msg_count == 0 || t.msg_size > 1 << 20 {
        an.anomaly(
            AnomalyKind::BadBind,
            format!(
                "record {}: msgSize {} msgCount {} is not a ring",
                hd.counter, t.msg_size, t.msg_count
            ),
        );
        return;
    }
    if qs.contains_key(&(hd.queue_id, hd.dir)) {
        an.anomaly(
            AnomalyKind::BadBind,
            format!(
                "record {}: q{} {:?} bound twice",
                hd.counter, hd.queue_id, hd.dir
            ),
        );
    }
    qs.insert(
        (hd.queue_id, hd.dir),
        Q {
            entry_size: t.msg_size as usize,
            ring: t.msg_count,
            next_entry: None,
            buf: Vec::new(),
            after_loss: false,
        },
    );
}

fn elements(an: &mut Analyzer, qs: &mut HashMap<(u16, Dir), Q>, rec: &Record) {
    let hd = rec.header;
    let Some(q) = qs.get_mut(&(hd.queue_id, hd.dir)) else {
        an.anomaly(
            AnomalyKind::BadElements,
            format!(
                "record {}: q{} {:?} has no Bind",
                hd.counter, hd.queue_id, hd.dir
            ),
        );
        return;
    };
    if rec.payload.len() != hd.entry_count as usize * q.entry_size || hd.first_entry >= q.ring {
        an.anomaly(
            AnomalyKind::BadElements,
            format!(
                "record {}: {} payload bytes for {} entries of {} at index {} of {}",
                hd.counter,
                rec.payload.len(),
                hd.entry_count,
                q.entry_size,
                hd.first_entry,
                q.ring
            ),
        );
        return;
    }
    if let Some(n) = q.next_entry
        && n != hd.first_entry
    {
        if !q.after_loss {
            an.anomaly(
                AnomalyKind::DiscontinuousEntries,
                format!(
                    "record {} q{} {:?}: starts at entry {}, previous record ended at {n}",
                    hd.counter, hd.queue_id, hd.dir, hd.first_entry
                ),
            );
        }
        q.buf.clear();
    }
    q.after_loss = false;
    q.next_entry =
        Some(((u64::from(hd.first_entry) + u64::from(hd.entry_count)) % u64::from(q.ring)) as u32);
    q.buf.extend_from_slice(&rec.payload);

    let layout = an.abi().element;
    let size_max = an.abi().element_size_max;
    let es = q.entry_size;
    loop {
        if q.buf.len() < es {
            break;
        }
        let count = match message_entries(&layout, size_max, &q.buf[..es], es) {
            Ok(n) => n,
            Err((count, limit)) => {
                an.anomaly(
                    AnomalyKind::DesyncEntry,
                    format!(
                        "record {} q{} {:?}: entry does not start a message (element count {count}, limit {limit}); dropped one entry",
                        hd.counter, hd.queue_id, hd.dir
                    ),
                );
                q.buf.drain(..es);
                continue;
            }
        };
        let need = count as usize * es;
        if q.buf.len() < need {
            break;
        }
        let msg: Vec<u8> = q.buf.drain(..need).collect();
        an.message(In {
            a: u64::from(hd.queue_id),
            b: 0,
            dir: hd.dir,
            ordinal: u64::from(hd.counter),
            ts: hd.ts,
            bytes: &msg,
            element_size: es as u32,
            exact: false,
            obs: None,
        });
    }
}
