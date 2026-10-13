//! The scrub stage: default-deny rewrite of a capture so it can be kept or shared without the
//! driver's message bodies (`docs/design/V3_GSP_CAPTURE_FORMAT.md` section 8).
//!
//! STATUS: DESIGN-ONLY tool, 2026-10-13 (tested GPU-free on synthetic streams; not run on a
//! module capture, none exists).
//!
//! # What a scrubbed file keeps
//!
//! * the file header (the die, the driver string, the clock) with `flags` bit 0 set, every
//!   `BIND`, `LOSS` and `END` record, and every record header;
//! * of every queue message: the fixed element-header fields the transport reads (checksum,
//!   sequence, element count, the MCTP words where the layout has them) and the 32-byte RPC
//!   envelope; of an RM control also the control header (client, object, cmd, status,
//!   `paramsSize`, flags);
//! * the **body** (the control's `params[]`, a continuation record's payload, any other
//!   function's payload) **only** when the control id is on the allow-list; otherwise it is
//!   replaced by the fill pattern [`FILL`], which the census recognises ("body scrubbed");
//! * every other byte, including the 32 CC bytes of an element header, the zero padding of an
//!   entry, an entry that does not start a message and a message that never completed, is
//!   replaced by zero.
//!
//! The checksum is recomputed so the scrubbed message still verifies; a message that did *not*
//! verify keeps its original residual (the scrub never heals what it did not understand), and
//! sequence numbers, lengths and element counts are untouched, so the oracle sees the same
//! stream. Anything the decoder could not parse is scrubbed as "no body is allowed".
//!
//! # Recognising a scrubbed body
//!
//! Byte `i` of a scrubbed body is `FILL[i % 8]`, counted from the first byte of the control's
//! `params[]` (a continuation record continues the count where the joined body stands), so
//! the joined body of a large control is one unbroken pattern. The census trusts the pattern
//! only in a file that says it was scrubbed (capture `flags` bit 0, observer header
//! `kf_scrubbed`); an unscrubbed file is never searched for it.

use crate::capture::{CaptureError, Dir, Kind, Reader, Record, RecordHeader};
use crate::observer::{self, MAX_LINE, Val};
use crate::stream::message_entries;
use kf_abi::view::RpcEnvelope;
use kf_gsp::element::peek_seq_num;
use kf_gsp::large::{Assembler, Step};
use kf_gsp::{
    GspAbi, IncomingRpc, RpcCommand, RpcFunction, TransportHdr, TxHeader, checksum32,
    decode_message, peek_len,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::io::{self, BufRead, Read, Write};

/// The fill pattern. Byte `i` of a scrubbed body is `FILL[i % 8]`.
pub const FILL: [u8; 8] = *b"KFSCRUB!";

/// Overwrite `buf` with the pattern, as if `buf[0]` were body byte `start`.
pub fn fill(buf: &mut [u8], start: usize) {
    for (j, b) in buf.iter_mut().enumerate() {
        *b = FILL[(start + j) % FILL.len()];
    }
}

/// True if `body` is non-empty and exactly the fill pattern from body byte 0.
#[must_use]
pub fn is_filled(body: &[u8]) -> bool {
    !body.is_empty()
        && body
            .iter()
            .enumerate()
            .all(|(i, &b)| b == FILL[i % FILL.len()])
}

/// The default allow-list: one control id per line (`#` starts a comment).
pub const DEFAULT_ALLOW_LIST: &str = include_str!("../allow_default.txt");

/// Controls that were observed, whose observed bodies pass the structural screen
/// (`docs/design/tier_checks/body_screen.py`), but whose purpose is not documented: off by
/// default, on with `--allow-screened`.
pub const SCREENED_ALLOW_LIST: &str = include_str!("../allow_screened.txt");

/// Control ids whose reply carries data that identifies one board (a GPU UUID, a serial, a
/// VBIOS or InfoROM image, an OEM board record, a fabric or cluster identity). They are
/// refused by [`AllowList::insert`] whatever the list or flag says: there is no override.
/// Each id was read in the ogkm headers; the evidence and the search that produced the list
/// are in `docs/design/V3_GSP_CAPTURE_FORMAT.md` section 8.
pub const BOARD_DATA_CONTROLS: &[(u32, &str)] = &[
    (
        0x2080_013f,
        "GPU_GET_OEM_BOARD_INFO: serialNumber, productPartNumber, memory part ids, build date",
    ),
    (0x2080_014a, "GPU_GET_GID_INFO: the GPU UUID / GID"),
    (
        0x2080_014b,
        "GPU_GET_INFOROM_OBJECT_VERSION: board InfoROM object versions",
    ),
    (
        0x2080_0156,
        "GPU_GET_INFOROM_IMAGE_VERSION: board InfoROM image version",
    ),
    (
        0x2080_015a,
        "GPU_GET_PHYSICAL_BRIDGE_VERSION_INFO: board bridge chip firmware",
    ),
    (0x2080_0169, "GPU_GET_OEM_INFO: OEM info blob"),
    (0x2080_0802, "BIOS_GET_INFO: VBIOS version and OEM version"),
    (0x2080_0803, "BIOS_GET_NBSI: VBIOS NBSI registry strings"),
    (0x2080_0806, "BIOS_GET_NBSI_OBJ: VBIOS NBSI object"),
    (
        0x2080_0808,
        "BIOS_GET_SKU_INFO: board id, chip SKU, project SKU",
    ),
    (0x2080_080e, "BIOS_GET_NBSI_V2: VBIOS NBSI registry strings"),
    (
        0x2080_0810,
        "BIOS_GET_INFO_V2: VBIOS version and OEM version",
    ),
    (
        0x2080_1801,
        "BUS_GET_PCI_INFO: PCI subsystem id (the board vendor and model)",
    ),
    (
        0x2080_3039,
        "NVLINK_SYNC_LINK_MASKS_AND_VBIOS_INFO: VBIOS data",
    ),
    (
        0x2080_3088,
        "NVLINK_GET_LOCAL_DEVICE_INFO: deviceUUID and PCI location",
    ),
    (
        0x0073_011d,
        "SYSTEM_GET_CONNECTOR_TABLE: the board's connector table (from the VBIOS)",
    ),
    (
        0x0073_0245,
        "SPECIFIC_GET_EDID_V2: the attached monitor's EDID (model and serial number)",
    ),
];

/// A set of control ids whose bodies a scrub keeps.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AllowList {
    ids: BTreeSet<u32>,
}

impl AllowList {
    /// The empty list: every body is scrubbed.
    #[must_use]
    pub fn empty() -> AllowList {
        AllowList::default()
    }

    /// The built-in default ([`DEFAULT_ALLOW_LIST`]).
    ///
    /// # Panics
    ///
    /// Never for the shipped file (a test parses it); a malformed build is a bug.
    #[must_use]
    pub fn builtin() -> AllowList {
        AllowList::parse(DEFAULT_ALLOW_LIST).expect("allow_default.txt parses")
    }

    /// The screened-but-unexplained list ([`SCREENED_ALLOW_LIST`]).
    ///
    /// # Panics
    ///
    /// Never for the shipped file (a test parses it).
    #[must_use]
    pub fn screened() -> AllowList {
        AllowList::parse(SCREENED_ALLOW_LIST).expect("allow_screened.txt parses")
    }

    /// Parse a list: one id per line, hex (`0x2080852a`) or decimal, `#` comments, blank lines.
    ///
    /// # Errors
    ///
    /// The first malformed line, or an id on the board-data deny list.
    pub fn parse(text: &str) -> Result<AllowList, String> {
        let mut l = AllowList::empty();
        for (i, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let id = parse_id(line)
                .ok_or_else(|| format!("line {}: not a control id: {line:?}", i + 1))?;
            l.insert(id).map_err(|e| format!("line {}: {e}", i + 1))?;
        }
        Ok(l)
    }

    /// Add one id.
    ///
    /// # Errors
    ///
    /// The id is on [`BOARD_DATA_CONTROLS`].
    pub fn insert(&mut self, id: u32) -> Result<(), String> {
        if let Some((_, what)) = BOARD_DATA_CONTROLS.iter().find(|(c, _)| *c == id) {
            return Err(format!(
                "{id:#010x} returns board data ({what}) and can never be allow-listed"
            ));
        }
        self.ids.insert(id);
        Ok(())
    }

    /// Is this control's body kept?
    #[must_use]
    pub fn contains(&self, id: u32) -> bool {
        self.ids.contains(&id)
    }

    /// Number of ids.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// True if no body is kept.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The ids in ascending order.
    pub fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.ids.iter().copied()
    }
}

/// Parse `0x...` hex or decimal.
#[must_use]
pub fn parse_id(s: &str) -> Option<u32> {
    let s = s.trim();
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u32::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

/// What a scrub did, counted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    /// Message runs presented.
    pub messages: u64,
    /// Runs the decoder could not parse (scrubbed with no body allowed).
    pub undecodable: u64,
    /// Control heads and whole controls, per control id: `(bodies kept, bodies scrubbed)`.
    pub per_cmd: BTreeMap<u32, (u64, u64)>,
    /// Continuation records kept / scrubbed.
    pub continuations: (u64, u64),
    /// Messages of other functions (events, alloc, init ...): payload scrubbed.
    pub other_scrubbed: u64,
    /// Body bytes replaced by the fill pattern.
    pub body_bytes_scrubbed: u64,
    /// Messages whose bytes past the declared length were not all zero (zeroed now).
    pub padding_zeroed: u64,
    /// Capture: entries (or partial messages) dropped to zero because they never formed a
    /// message; records whose payload could not be interpreted.
    pub bytes_zeroed_unframed: u64,
    /// Capture: messages that never completed (dropped by the decoder) and were reduced to
    /// their first entry's header fields.
    pub partial_dropped: u64,
    /// Observer: lines dropped (unreadable, oversized, unknown kind).
    pub lines_dropped: u64,
    /// Observer: keys dropped from header, record and footer lines (not on the key allow-list).
    pub keys_dropped: u64,
    /// Capture: the input ended inside a record (the rest of the file is not written).
    pub truncated: Option<String>,
}

impl Stats {
    /// True if the input had anything the scrub had to drop or could not parse.
    #[must_use]
    pub fn input_was_clean(&self) -> bool {
        self.undecodable == 0
            && self.lines_dropped == 0
            && self.truncated.is_none()
            && self.bytes_zeroed_unframed == 0
            && self.partial_dropped == 0
    }
}

#[derive(Debug, Clone, Copy)]
struct Large {
    allow: bool,
    /// Bytes of the joined payload so far.
    got: usize,
    params_at: usize,
}

/// What to do with one message's payload (the bytes after the 32-byte envelope).
#[derive(Debug, Clone, Copy)]
struct Plan {
    /// Leading payload bytes kept verbatim (the control header).
    keep: usize,
    /// Keep the rest verbatim too (allow-listed body).
    allowed: bool,
    /// Body offset at which the fill pattern starts counting for the first scrubbed byte.
    fill_from: usize,
}

impl Plan {
    fn all_fill() -> Plan {
        Plan {
            keep: 0,
            allowed: false,
            fill_from: 0,
        }
    }
}

type Key = (u64, u64, Dir);

/// The message rewriter. One per scrub run; it carries the large-control state per queue.
pub struct Scrubber {
    abi: GspAbi,
    allow: AllowList,
    asm: HashMap<Key, Assembler>,
    large: HashMap<Key, Large>,
    /// Counters.
    pub stats: Stats,
}

impl std::fmt::Debug for Scrubber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scrubber")
            .field("allow", &self.allow.len())
            .finish_non_exhaustive()
    }
}

impl Scrubber {
    /// A scrubber for the ABI of one decode tag.
    #[must_use]
    pub fn new(abi: GspAbi, allow: AllowList) -> Scrubber {
        Scrubber {
            abi,
            allow,
            asm: HashMap::new(),
            large: HashMap::new(),
            stats: Stats::default(),
        }
    }

    /// The entry stride an observer message was cut with.
    #[must_use]
    pub fn observer_element_size(&self) -> u32 {
        self.abi.driver.gsp_element_size_min()
    }

    fn control_plan(&mut self, cmd: u32, params_at: usize) -> Plan {
        let allowed = self.allow.contains(cmd);
        let e = self.stats.per_cmd.entry(cmd).or_default();
        if allowed {
            e.0 += 1;
        } else {
            e.1 += 1;
        }
        Plan {
            keep: params_at,
            allowed,
            fill_from: 0,
        }
    }

    fn continuation_plan(&mut self, key: Key, payload_len: usize, done: bool) -> Plan {
        let Some(ctx) = self.large.get_mut(&key) else {
            self.stats.other_scrubbed += 1;
            return Plan::all_fill();
        };
        let plan = Plan {
            keep: 0,
            allowed: ctx.allow,
            fill_from: ctx.got.saturating_sub(ctx.params_at),
        };
        ctx.got += payload_len;
        if ctx.allow {
            self.stats.continuations.0 += 1;
        } else {
            self.stats.continuations.1 += 1;
        }
        if done {
            self.large.remove(&key);
        }
        plan
    }

    fn classify(&mut self, key: Key, inc: &IncomingRpc) -> Plan {
        let cmd = RpcCommand::from_incoming(&self.abi.rpc, inc);
        let table = self.abi.driver;
        let ctl = if cmd.function == RpcFunction::RmControl {
            table.decode_rpc_control(&cmd.payload).ok()
        } else {
            None
        };
        let declared = ctl.map(|r| r.params_at + r.params_size as usize);
        let asm = self.asm.entry(key).or_default();
        let step = asm.step(&cmd, declared);
        asm.commit();
        match step {
            Step::NotLarge => {
                self.large.remove(&key);
                match ctl {
                    Some(r) => self.control_plan(r.cmd, r.params_at),
                    None => {
                        self.stats.other_scrubbed += 1;
                        Plan::all_fill()
                    }
                }
            }
            Step::Held => match ctl {
                Some(r) if cmd.function == RpcFunction::RmControl => {
                    let plan = self.control_plan(r.cmd, r.params_at);
                    self.large.insert(
                        key,
                        Large {
                            allow: plan.allowed,
                            got: cmd.payload.len(),
                            params_at: r.params_at,
                        },
                    );
                    plan
                }
                _ => self.continuation_plan(key, cmd.payload.len(), false),
            },
            Step::Complete { .. } => self.continuation_plan(key, cmd.payload.len(), true),
            Step::Refused(_) => {
                self.large.remove(&key);
                self.stats.other_scrubbed += 1;
                Plan::all_fill()
            }
        }
    }

    /// Neutralise the bytes of a message that never completed (the decoder drops them): keep
    /// only what makes the first entry look like a message start (the fixed element-header
    /// fields and the envelope), zero everything else, so the scrubbed file shows the decoder
    /// the same shape and raises the same anomaly, without any of the dropped body.
    pub fn scrub_partial(&mut self, partial: &mut [u8]) {
        let layout = self.abi.element;
        let n = partial.len();
        let mut out = vec![0u8; n];
        let mut copy4 = |off: usize| {
            if off + 4 <= n {
                out[off..off + 4].copy_from_slice(&partial[off..off + 4]);
            }
        };
        copy4(layout.seqnum_off());
        if let Some(o) = layout.elem_count_off() {
            copy4(o);
        }
        if let TransportHdr::Mctp {
            header_off,
            nvdm_off,
            ..
        } = layout.transport()
        {
            copy4(header_off);
            copy4(nvdm_off);
        }
        let hdr = layout.hdr_size();
        let env_end = (hdr + RpcEnvelope::SIZE).min(n);
        if hdr < env_end {
            out[hdr..env_end].copy_from_slice(&partial[hdr..env_end]);
        }
        self.stats.partial_dropped += 1;
        self.stats.bytes_zeroed_unframed += n.saturating_sub(env_end.saturating_sub(hdr)) as u64;
        partial.copy_from_slice(&out);
    }

    /// Rewrite one message run in place. `run` starts at the element header; for a capture it
    /// is the whole entry run, for the observer the message rounded to 8 bytes.
    ///
    /// The run keeps its length. The sequence of calls for one queue must be the stream order
    /// (the large-control state depends on it).
    pub fn scrub_run(&mut self, key: Key, run: &mut [u8], element_size: u32) {
        self.stats.messages += 1;
        let layout = self.abi.element;
        let table = self.abi.driver;
        let n = run.len();
        let hdr = layout.hdr_size();
        let Ok(seq) = peek_seq_num(&layout, run) else {
            self.stats.undecodable += 1;
            run.fill(0);
            return;
        };
        let len = peek_len(&layout, run, element_size, self.abi.element_size_max).ok();
        let extent = len.map_or(n, |l| (l.msg_len() as usize).min(n));
        // What the original folds to. A valid message folds to 0; a corrupt one keeps its
        // residual, so the scrub does not turn a checksum failure into a pass.
        let residual = checksum32(run, extent);
        let decoded = len.and_then(|l| decode_message(&layout, run, l, seq, &table).ok());
        let plan = match decoded {
            Some(inc) => self.classify(key, &inc),
            None => {
                self.stats.undecodable += 1;
                Plan::all_fill()
            }
        };

        let mut out = vec![0u8; n];
        let mut copy4 = |off: usize| {
            if off + 4 <= n {
                out[off..off + 4].copy_from_slice(&run[off..off + 4]);
            }
        };
        copy4(layout.seqnum_off());
        if let Some(o) = layout.elem_count_off() {
            copy4(o);
        }
        if let TransportHdr::Mctp {
            header_off,
            nvdm_off,
            ..
        } = layout.transport()
        {
            copy4(header_off);
            copy4(nvdm_off);
        }
        let env_end = (hdr + RpcEnvelope::SIZE).min(extent);
        if hdr < env_end {
            out[hdr..env_end].copy_from_slice(&run[hdr..env_end]);
        }
        let p0 = hdr + RpcEnvelope::SIZE;
        for j in 0..extent.saturating_sub(p0) {
            out[p0 + j] = if plan.allowed || j < plan.keep {
                run[p0 + j]
            } else {
                self.stats.body_bytes_scrubbed += 1;
                FILL[(plan.fill_from + j - plan.keep) % FILL.len()]
            };
        }
        if run[extent..].iter().any(|&b| b != 0) {
            self.stats.padding_zeroed += 1;
        }
        let co = layout.checksum_off();
        if co + 4 <= n {
            let fold = checksum32(&out, extent);
            out[co..co + 4].copy_from_slice(&(fold ^ residual).to_le_bytes());
        }
        run.copy_from_slice(&out);
    }
}

// ---------------------------------------------------------------------------------------------
// KFGSPCAP

struct Slot {
    header: RecordHeader,
    payload: Vec<u8>,
    /// Payload bytes that belong to a message not yet rewritten (the slot cannot be written).
    pending: usize,
}

#[derive(Debug, Clone, Copy)]
struct Seg {
    slot: u64,
    off: usize,
    len: usize,
}

struct Q {
    entry_size: usize,
    ring: u32,
    next_entry: Option<u32>,
    after_loss: bool,
    buf: Vec<u8>,
    segs: VecDeque<Seg>,
}

struct Out<W: Write> {
    w: W,
    slots: VecDeque<Slot>,
    base: u64,
}

impl<W: Write> Out<W> {
    fn push(&mut self, header: RecordHeader, payload: Vec<u8>, pending: usize) -> u64 {
        self.slots.push_back(Slot {
            header,
            payload,
            pending,
        });
        self.base + self.slots.len() as u64 - 1
    }

    fn slot(&mut self, abs: u64) -> &mut Slot {
        let ix = usize::try_from(abs - self.base).unwrap_or(usize::MAX);
        &mut self.slots[ix]
    }

    /// Write every leading slot that has no unfinished message in it.
    fn flush(&mut self) -> io::Result<()> {
        while self.slots.front().is_some_and(|s| s.pending == 0) {
            if let Some(s) = self.slots.pop_front() {
                self.base += 1;
                self.w.write_all(&s.header.encode())?;
                self.w.write_all(&s.payload)?;
            }
        }
        Ok(())
    }
}

/// Remove the first `n` bytes of the queue's pending message from `segs` and return them.
fn take_front(segs: &mut VecDeque<Seg>, mut n: usize) -> Vec<Seg> {
    let mut got = Vec::new();
    while n > 0 {
        let Some(front) = segs.front_mut() else { break };
        let t = front.len.min(n);
        got.push(Seg {
            slot: front.slot,
            off: front.off,
            len: t,
        });
        front.off += t;
        front.len -= t;
        n -= t;
        if front.len == 0 {
            segs.pop_front();
        }
    }
    got
}

fn zero_segs<W: Write>(out: &mut Out<W>, segs: &[Seg], stats: &mut Stats) {
    for s in segs {
        let slot = out.slot(s.slot);
        slot.payload[s.off..s.off + s.len].fill(0);
        slot.pending -= s.len;
        stats.bytes_zeroed_unframed += s.len as u64;
    }
}

/// Write `bytes` back over the byte ranges `segs` (in order) and release those ranges.
fn scatter<W: Write>(out: &mut Out<W>, segs: &[Seg], bytes: &[u8]) {
    let mut at = 0;
    for s in segs {
        let slot = out.slot(s.slot);
        slot.payload[s.off..s.off + s.len].copy_from_slice(&bytes[at..at + s.len]);
        slot.pending -= s.len;
        at += s.len;
    }
}

/// The queue's incomplete message will never be decoded (loss, discontinuity, end of stream):
/// reduce it to the header fields of its first entry ([`Scrubber::scrub_partial`]).
fn drop_partial<W: Write>(sc: &mut Scrubber, out: &mut Out<W>, q: &mut Q) {
    if q.buf.is_empty() {
        return;
    }
    let mut b = std::mem::take(&mut q.buf);
    sc.scrub_partial(&mut b);
    let segs: Vec<Seg> = q.segs.drain(..).collect();
    scatter(out, &segs, &b);
}

/// Scrub a KFGSPCAP stream: read `r`, write the scrubbed capture to `w`.
///
/// Record headers, counters, timestamps, `BIND`, loss and `END` records are copied; each
/// `ELEMENTS` payload is rewritten message by message ([`Scrubber::scrub_run`]) with the same
/// reassembly rule the decoder uses (`stream.rs`), so message boundaries cannot differ.
///
/// # Errors
///
/// An unreadable file header, or a failed write. A file that ends inside a record is not an
/// error: the complete records are written, `stats.truncated` says so.
pub fn scrub_capture<R: Read, W: Write>(sc: &mut Scrubber, r: R, w: W) -> Result<(), CaptureError> {
    let mut rd = Reader::new(r)?;
    let mut fh = rd.header().clone();
    fh.scrubbed = true;
    let mut out = Out {
        w,
        slots: VecDeque::new(),
        base: 0,
    };
    out.w.write_all(&fh.encode()?)?;
    let mut qs: HashMap<(u16, Dir), Q> = HashMap::new();
    loop {
        let rec = match rd.next_record() {
            Ok(Some(rec)) => rec,
            Ok(None) => break,
            Err(CaptureError::Io(e)) => return Err(CaptureError::Io(e)),
            Err(e) => {
                sc.stats.truncated = Some(e.to_string());
                break;
            }
        };
        match rec.header.kind {
            Kind::Bind => bind(sc, &mut out, &mut qs, rec),
            Kind::Elements => elements(sc, &mut out, &mut qs, rec),
            k => {
                if k.is_loss()
                    && let Some(q) = qs.get_mut(&(rec.header.queue_id, rec.header.dir))
                {
                    q.after_loss = true;
                    drop_partial(sc, &mut out, q);
                }
                // loss and End records carry no payload; whatever a malformed one carries is dropped
                let mut payload = rec.payload;
                sc.stats.bytes_zeroed_unframed += payload.len() as u64;
                payload.fill(0);
                out.push(rec.header, payload, 0);
            }
        }
        out.flush()?;
    }
    for q in qs.values_mut() {
        drop_partial(sc, &mut out, q);
    }
    out.flush()?;
    out.w.flush()?;
    Ok(())
}

fn bind<W: Write>(
    sc: &mut Scrubber,
    out: &mut Out<W>,
    qs: &mut HashMap<(u16, Dir), Q>,
    rec: Record,
) {
    let hd = rec.header;
    let mut payload = rec.payload;
    let geometry = TxHeader::decode(&payload).ok().filter(|t| {
        payload.len() == TxHeader::BYTES
            && t.msg_size >= 16
            && t.msg_count != 0
            && t.msg_size <= 1 << 20
    });
    let ok = if let Some(t) = geometry {
        qs.insert(
            (hd.queue_id, hd.dir),
            Q {
                entry_size: t.msg_size as usize,
                ring: t.msg_count,
                next_entry: None,
                after_loss: false,
                buf: Vec::new(),
                segs: VecDeque::new(),
            },
        );
        true
    } else {
        false
    };
    if !ok {
        // not a ring header: nothing in it is a geometry worth keeping
        sc.stats.bytes_zeroed_unframed += payload.len() as u64;
        payload.fill(0);
    }
    out.push(hd, payload, 0);
}

fn elements<W: Write>(
    sc: &mut Scrubber,
    out: &mut Out<W>,
    qs: &mut HashMap<(u16, Dir), Q>,
    rec: Record,
) {
    let hd = rec.header;
    let mut payload = rec.payload;
    let Some(q) = qs.get_mut(&(hd.queue_id, hd.dir)) else {
        sc.stats.bytes_zeroed_unframed += payload.len() as u64;
        payload.fill(0);
        out.push(hd, payload, 0);
        return;
    };
    if payload.len() != hd.entry_count as usize * q.entry_size || hd.first_entry >= q.ring {
        sc.stats.bytes_zeroed_unframed += payload.len() as u64;
        payload.fill(0);
        out.push(hd, payload, 0);
        return;
    }
    if q.next_entry.is_some_and(|n| n != hd.first_entry) {
        // discontinuity: the partial message before it is never decoded
        drop_partial(sc, out, q);
    }
    q.after_loss = false;
    q.next_entry =
        Some(((u64::from(hd.first_entry) + u64::from(hd.entry_count)) % u64::from(q.ring)) as u32);
    let len = payload.len();
    q.buf.extend_from_slice(&payload);
    let abs = out.push(hd, payload, len);
    if len > 0 {
        q.segs.push_back(Seg {
            slot: abs,
            off: 0,
            len,
        });
    }

    let layout = sc.abi.element;
    let size_max = sc.abi.element_size_max;
    let es = q.entry_size;
    let key_q = u64::from(hd.queue_id);
    loop {
        if q.buf.len() < es {
            break;
        }
        let count = match message_entries(&layout, size_max, &q.buf[..es], es) {
            Ok(n) => n,
            Err(_) => {
                // an entry that starts no message: dropped by the decoder, zeroed here
                q.buf.drain(..es);
                let segs = take_front(&mut q.segs, es);
                zero_segs(out, &segs, &mut sc.stats);
                continue;
            }
        };
        let need = count as usize * es;
        if q.buf.len() < need {
            break;
        }
        let mut msg: Vec<u8> = q.buf.drain(..need).collect();
        sc.scrub_run((key_q, 0, hd.dir), &mut msg, es as u32);
        let segs = take_front(&mut q.segs, need);
        scatter(out, &segs, &msg);
    }
}

// ---------------------------------------------------------------------------------------------
// observer gsp.jsonl

const HEADER_KEYS: &[&str] = &[
    "schema",
    "kind",
    "capture_complete",
    "selection",
    "observer",
    "device",
    "magic",
    "version",
    "header_bytes",
    "record_header_bytes",
    "qpc_frequency",
    "started_qpc",
    "flags",
    "reserved",
    "reserved2",
];
const RECORD_KEYS: &[&str] = &[
    "kind",
    "generation",
    "trigger",
    "magic",
    "header_bytes",
    "payload_bytes",
    "direction",
    "qpc",
    "table_pa",
    "queue_sequence",
    "rpc_sequence",
    "rpc_function",
    "rpc_result",
    "flags",
    "missing_before",
    "rpc_version",
    "reserved",
];
const FOOTER_KEYS: &[&str] = &[
    "kind",
    "file_export_complete",
    "capture_complete",
    "records",
    "source_records",
    "omitted_records",
    "source_bytes",
    "exported_bytes",
    "driver_stats",
];

fn put_json(v: &Val, o: &mut String) -> bool {
    use std::fmt::Write as _;
    match v {
        Val::Int(n) => {
            let _ = write!(o, "{n}");
            true
        }
        Val::Bool(b) => {
            o.push_str(if *b { "true" } else { "false" });
            true
        }
        // The key allow-list names the string-valued keys; the value must still be a short
        // printable word, otherwise the key is dropped.
        Val::Str(s) if s.len() <= 64 && s.bytes().all(|b| (0x20..0x7f).contains(&b)) => {
            let _ = write!(o, "\"{s}\"");
            true
        }
        Val::Str(_) => false,
        Val::Arr(a) => {
            o.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    o.push(',');
                }
                if !matches!(x, Val::Int(_)) || !put_json(x, o) {
                    return false;
                }
            }
            o.push(']');
            true
        }
        Val::Obj(m) => {
            // only counters and flags (driver_stats)
            o.push('{');
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    o.push(',');
                }
                if !k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    || !matches!(x, Val::Int(_) | Val::Bool(_))
                {
                    return false;
                }
                let _ = write!(o, "\"{k}\":");
                put_json(x, o);
            }
            o.push('}');
            true
        }
    }
}

fn line_out(
    obj: &[(String, Val)],
    keys: &[&str],
    extra: &[(&str, String)],
    stats: &mut Stats,
) -> String {
    let mut o = String::from("{");
    let mut first = true;
    let mut emit = |o: &mut String, k: &str, f: &dyn Fn(&mut String) -> bool| {
        let mark = o.len();
        if !first {
            o.push(',');
        }
        o.push('"');
        o.push_str(k);
        o.push_str("\":");
        if f(o) {
            first = false;
            true
        } else {
            o.truncate(mark);
            false
        }
    };
    for (k, v) in obj {
        if extra.iter().any(|(e, _)| e == k) {
            continue; // replaced below (payload_hex), not dropped
        }
        if !keys.contains(&k.as_str()) {
            stats.keys_dropped += 1;
            continue;
        }
        if !emit(&mut o, k, &|o| put_json(v, o)) {
            stats.keys_dropped += 1;
        }
    }
    for (k, raw) in extra {
        emit(&mut o, k, &|o| {
            o.push_str(raw);
            true
        });
    }
    o.push_str("}\n");
    o
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if !b.len().is_multiple_of(2) {
        return None;
    }
    let nib = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    b.as_chunks::<2>()
        .0
        .iter()
        .map(|p| Some(nib(p[0])? << 4 | nib(p[1])?))
        .collect()
}

fn find<'a>(o: &'a [(String, Val)], k: &str) -> Option<&'a Val> {
    o.iter().find(|(n, _)| n == k).map(|(_, v)| v)
}

/// Scrub an observer `gsp.jsonl`: read `r`, write the scrubbed export to `w`.
///
/// Header, record and footer lines keep only the keys on explicit lists (counters, flags and
/// short printable words); a record's `payload_hex` is rewritten with the same length. The
/// footer's digests are dropped (they would no longer match). A line that cannot be read, or
/// a record that lacks a field, is not copied.
///
/// # Errors
///
/// I/O errors, and a record line before any header line (the scrubbed marker lives in the
/// header, and a file without one would not announce that it is scrubbed).
pub fn scrub_observer<R: BufRead, W: Write>(
    sc: &mut Scrubber,
    mut r: R,
    mut w: W,
) -> io::Result<()> {
    let es = sc.observer_element_size();
    let mut line = Vec::new();
    let mut have_header = false;
    loop {
        line.clear();
        let n = (&mut r)
            .take(MAX_LINE as u64 + 1)
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        if line.len() > MAX_LINE {
            sc.stats.lines_dropped += 1;
            let mut skip = Vec::new();
            loop {
                skip.clear();
                let k = (&mut r).take(65_536).read_until(b'\n', &mut skip)?;
                if k == 0 || skip.ends_with(b"\n") {
                    break;
                }
            }
            continue;
        }
        let text = line.strip_suffix(b"\n").unwrap_or(&line);
        let Ok(obj) = observer::parse_object(text) else {
            sc.stats.lines_dropped += 1;
            continue;
        };
        let kind = match find(&obj, "kind") {
            Some(Val::Str(s)) => s.as_str(),
            _ => "",
        };
        match kind {
            "header" if !have_header => {
                have_header = true;
                let s = line_out(
                    &obj,
                    HEADER_KEYS,
                    &[("kf_scrubbed", "1".to_string())],
                    &mut sc.stats,
                );
                w.write_all(s.as_bytes())?;
            }
            "record" => {
                if !have_header {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "record line before the header line: refusing to write an unmarked file",
                    ));
                }
                let (
                    Some(Val::Str(hex)),
                    Some(Val::Int(generation)),
                    Some(Val::Int(pa)),
                    Some(Val::Int(dir)),
                ) = (
                    find(&obj, "payload_hex"),
                    find(&obj, "generation"),
                    find(&obj, "table_pa"),
                    find(&obj, "direction"),
                )
                else {
                    sc.stats.lines_dropped += 1;
                    continue;
                };
                let (Some(mut bytes), true) = (unhex(hex), *dir <= 1) else {
                    sc.stats.lines_dropped += 1;
                    continue;
                };
                let d = if *dir == 0 { Dir::Cmd } else { Dir::Status };
                sc.scrub_run((*generation, *pa, d), &mut bytes, es);
                let hex_out: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                let s = line_out(
                    &obj,
                    RECORD_KEYS,
                    &[("payload_hex", format!("\"{hex_out}\""))],
                    &mut sc.stats,
                );
                w.write_all(s.as_bytes())?;
            }
            "footer" => {
                let s = line_out(&obj, FOOTER_KEYS, &[], &mut sc.stats);
                w.write_all(s.as_bytes())?;
            }
            _ => sc.stats.lines_dropped += 1,
        }
    }
    w.flush()
}
