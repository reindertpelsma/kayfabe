//! The decoder and the completeness oracle.
//!
//! Sources (the observer's `gsp.jsonl`, KFGSPCAP captures) reduce to one input: a *message*,
//! the byte run of one queue message starting at its element header, plus where it came from.
//! [`Analyzer::message`] decodes it with kayfabe's own transport code
//! (`kf_gsp::element::{peek_seq_num, peek_len, decode_message}` and
//! `kf_abi::versions::DriverAbiTable::decode_rpc_control`), and applies the oracle:
//!
//! 1. sequence numbers are continuous per `(source, queue instance, direction)`, starting at 0;
//! 2. every message parses (checksum, length, envelope, control header, params fit);
//! 3. no trailing bytes (an observer message is exactly its rounded length; a capture stream
//!    ends on a message boundary);
//! 4. every function id is one the ogkm matrix defines at the decode tag.
//!
//! Anything else the source layer can detect (record-counter gaps, loss markers, bad footer)
//! also lands in [`Anomaly`] counters. Counts are exact; only the stored *examples* are capped
//! (a cap may truncate what is printed, never what is computed).

use crate::capture::Dir;
use crate::ogkm::{OgkmIndex, OgkmStatus};
use kf_abi::DriverVersion;
use kf_gsp::element::peek_seq_num;
use kf_gsp::large::{Assembler, Step};
use kf_gsp::{GspAbi, RpcCommand, RpcFunction, decode_message, peek_elem_count, peek_len};
use kf_util::sha256::sha256;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

/// Distinct control bodies: `(cmd, dir, reply status, body)` -> messages (requests: status 0).
pub type Bodies = BTreeMap<(u32, Dir, u32, Vec<u8>), u64>;

/// Examples kept per anomaly kind.
pub const EXAMPLE_CAP: usize = 12;

/// Everything the oracle can report. All of them make [`Report::oracle_ok`] false.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AnomalyKind {
    /// Queue sequence jumped forward: messages are missing.
    SeqGap,
    /// Queue sequence went backwards or repeated.
    SeqRewind,
    /// First message of a queue instance is not sequence 0: its prefix is missing.
    PrefixMissing,
    /// A message failed to parse (see the example text for the exact fault).
    ParseFailure,
    /// Bytes after the end of a message, or a length that does not match the declared one.
    TrailingBytes,
    /// `elemCount` disagrees with the element count derived from the declared length.
    ElemCountMismatch,
    /// The observer's metadata disagrees with the payload it carried.
    MetaMismatch,
    /// The RPC header version is not the one the ABI table expects.
    HeaderVersion,
    /// A function id the ogkm matrix does not define at the decode tag.
    UnknownFunction,
    /// A control declares more params than the message carries.
    ParamsTruncated,
    /// A control message carries more bytes than its declared params.
    ControlExtraBytes,
    /// The observer reported dropped or missing records itself.
    ObserverLoss,
    /// The capture module wrote an explicit loss marker.
    CaptureLoss,
    /// Capture records are not numbered consecutively.
    CounterGap,
    /// A timestamp went backwards.
    TimestampRegress,
    /// The stream ended inside a record or message, or framing was invalid.
    StreamTruncated,
    /// A capture file has no End record.
    EndMissing,
    /// The End record disagrees with the number of records read, or data follows it.
    EndMismatch,
    /// An observer line could not be read as the expected JSON.
    BadLine,
    /// Missing, incomplete or inconsistent observer footer or header.
    FooterProblem,
    /// Capture entries are not contiguous and no loss marker explains it.
    DiscontinuousEntries,
    /// An Elements record before its Bind, or with a payload that is not whole entries.
    BadElements,
    /// A Bind record that is malformed or repeated.
    BadBind,
    /// An entry that does not start a plausible message (the decoder resynchronised).
    DesyncEntry,
}

impl AnomalyKind {
    /// Stable name used in TSV and JSON.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            AnomalyKind::SeqGap => "seq_gap",
            AnomalyKind::SeqRewind => "seq_rewind",
            AnomalyKind::PrefixMissing => "prefix_missing",
            AnomalyKind::ParseFailure => "parse_failure",
            AnomalyKind::TrailingBytes => "trailing_bytes",
            AnomalyKind::ElemCountMismatch => "elem_count_mismatch",
            AnomalyKind::MetaMismatch => "meta_mismatch",
            AnomalyKind::HeaderVersion => "header_version",
            AnomalyKind::UnknownFunction => "unknown_function",
            AnomalyKind::ParamsTruncated => "params_truncated",
            AnomalyKind::ControlExtraBytes => "control_extra_bytes",
            AnomalyKind::ObserverLoss => "observer_loss",
            AnomalyKind::CaptureLoss => "capture_loss",
            AnomalyKind::CounterGap => "counter_gap",
            AnomalyKind::TimestampRegress => "timestamp_regress",
            AnomalyKind::StreamTruncated => "stream_truncated",
            AnomalyKind::EndMissing => "end_missing",
            AnomalyKind::EndMismatch => "end_mismatch",
            AnomalyKind::BadLine => "bad_line",
            AnomalyKind::FooterProblem => "footer_problem",
            AnomalyKind::DiscontinuousEntries => "discontinuous_entries",
            AnomalyKind::BadElements => "bad_elements",
            AnomalyKind::BadBind => "bad_bind",
            AnomalyKind::DesyncEntry => "desync_entry",
        }
    }
}

/// Count plus the first few examples of one anomaly kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Anomaly {
    /// Exact number of occurrences.
    pub count: u64,
    /// The first [`EXAMPLE_CAP`] descriptions.
    pub examples: Vec<String>,
}

/// What the observer wrote beside the payload (cross-checked against the payload).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObsMeta {
    /// `queue_sequence`.
    pub queue_sequence: u32,
    /// `rpc_sequence`.
    pub rpc_sequence: u32,
    /// `rpc_function`.
    pub rpc_function: u32,
    /// `rpc_result`.
    pub rpc_result: u32,
    /// `rpc_version`.
    pub rpc_version: u32,
    /// `payload_bytes`.
    pub payload_bytes: u32,
    /// `missing_before` (observer-declared loss ahead of this record).
    pub missing_before: u32,
    /// Flag bit 1: the observer attached mid-stream.
    pub prefix_unknown: bool,
}

/// One message presented to the analyzer.
#[derive(Debug, Clone, Copy)]
pub struct In<'a> {
    /// First half of the queue-instance id (observer: generation; capture: queue id).
    pub a: u64,
    /// Second half (observer: table physical address; capture: 0).
    pub b: u64,
    /// Queue of the pair.
    pub dir: Dir,
    /// Position in the source (line or record number), for messages.
    pub ordinal: u64,
    /// Source timestamp (informational).
    pub ts: u64,
    /// The message run, starting at the element header.
    pub bytes: &'a [u8],
    /// Entry stride the run was cut with.
    pub element_size: u32,
    /// True when `bytes` must be exactly the message rounded to 8 (the observer); false when
    /// it is a whole number of entries with zero padding allowed (the capture format).
    pub exact: bool,
    /// Observer metadata to cross-check.
    pub obs: Option<ObsMeta>,
}

/// Description of one input, for the report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceInfo {
    /// Path or label.
    pub label: String,
    /// `observer-jsonl` or `kfgspcap`.
    pub format: String,
    /// Free key/value facts (device id, driver string, footer counters ...), all non-board.
    pub facts: Vec<(String, String)>,
    /// Records read.
    pub records: u64,
    /// Messages decoded.
    pub messages: u64,
    /// The source says it went through the scrub stage (capture `flags` bit 0, observer header
    /// `kf_scrubbed`): a control body that is exactly the fill pattern is then a scrubbed body.
    pub scrubbed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct StreamKey {
    source: usize,
    a: u64,
    b: u64,
    dir: Dir,
}

#[derive(Debug, Clone, Copy)]
struct SeqState {
    expected: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PairKey {
    source: usize,
    a: u64,
    b: u64,
    rpc_sequence: u32,
    client: u32,
    object: u32,
    cmd: u32,
}

#[derive(Debug, Default)]
struct Pending {
    // `None` = the body was scrubbed: it takes part in pairing but never in the echo comparison.
    reqs: VecDeque<Option<Vec<u8>>>,
    reps: VecDeque<Option<Vec<u8>>>,
}

#[derive(Debug, Default)]
struct ControlAcc {
    requests: u64,
    replies: u64,
    paired: u64,
    echo: u64,
    serialized: u64,
    req_sizes: BTreeMap<u32, u64>,
    rep_sizes: BTreeMap<u32, u64>,
    statuses: BTreeMap<u32, u64>,
    rpc_results: BTreeMap<u32, u64>,
    reply_hashes: HashSet<[u8; 32]>,
    scrubbed: u64,
}

/// Per-function counters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FuncRow {
    /// Name from the matrix at the tag, if defined there.
    pub name: Option<String>,
    /// Messages.
    pub count: u64,
    /// Message bytes (declared `length` + element header).
    pub bytes: u64,
    /// `rpc_result` histogram (replies and events only; requests carry the sentinel).
    pub results: BTreeMap<u32, u64>,
}

/// One row of the per-control census.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlRow {
    /// Control id.
    pub cmd: u32,
    /// Name, if the matrix knows it at the tag.
    pub name: Option<String>,
    /// Request messages.
    pub requests: u64,
    /// Reply messages.
    pub replies: u64,
    /// Requests matched to a reply.
    pub paired: u64,
    /// Requests with no reply left over after FIFO matching.
    pub unanswered: u64,
    /// Replies with no request left over after FIFO matching.
    pub orphan_replies: u64,
    /// Matched pairs whose reply params equal the request params byte for byte.
    pub echo: u64,
    /// Requests with the serialized-params flag.
    pub serialized: u64,
    /// Declared request `paramsSize` histogram.
    pub req_sizes: BTreeMap<u32, u64>,
    /// Declared reply `paramsSize` histogram.
    pub rep_sizes: BTreeMap<u32, u64>,
    /// Reply control `status` histogram.
    pub statuses: BTreeMap<u32, u64>,
    /// Reply envelope `rpc_result` histogram.
    pub rpc_results: BTreeMap<u32, u64>,
    /// Number of distinct reply parameter bodies (sha-256), scrubbed bodies not counted.
    pub distinct_replies: u64,
    /// Request and reply messages whose body was scrubbed (the source went through the scrub
    /// stage and the body is the fill pattern). Such bodies are excluded from `echo` and
    /// `distinct_replies`; every count, size and status is unaffected.
    pub scrubbed: u64,
    /// ogkm answer.
    pub ogkm: OgkmStatus,
}

/// The finished analysis.
#[derive(Debug, Clone)]
pub struct Report {
    /// Decode tag used.
    pub tag: DriverVersion,
    /// Inputs.
    pub sources: Vec<SourceInfo>,
    /// Messages per direction `[cmd, status]`.
    pub messages: [u64; 2],
    /// Oracle anomalies, by kind.
    pub anomalies: BTreeMap<AnomalyKind, Anomaly>,
    /// Anomalies of a kind that followed a declared loss marker (subset of the counts).
    pub declared_loss_followups: u64,
    /// Total messages the sequence oracle knows are missing (sum of forward gaps).
    pub missing_messages: u64,
    /// Messages whose zero padding was not zero (information, not an anomaly).
    pub nonzero_padding: u64,
    /// Messages with a non-zero `rpc.sequence` (the Windows driver sends none).
    pub rpc_sequence_nonzero: u64,
    /// Large controls joined from a head plus continuation records.
    pub large_joined: u64,
    /// Per-function counts, keyed `(dir, function id)`.
    pub functions: BTreeMap<(Dir, u32), FuncRow>,
    /// Per-control census.
    pub controls: Vec<ControlRow>,
    /// Distinct control parameter bodies, only when [`Analyzer::collect_bodies`] was called,
    /// and only from sources that went through the scrub stage (a body that is the fill
    /// pattern is never recorded). Key `(cmd, dir, reply status, body)`, value: messages.
    /// Requests carry status 0.
    pub bodies: Option<Bodies>,
}

impl Report {
    /// True iff the oracle found nothing at all.
    #[must_use]
    pub fn oracle_ok(&self) -> bool {
        self.anomalies.values().all(|a| a.count == 0)
    }

    /// Total anomaly count.
    #[must_use]
    pub fn anomaly_total(&self) -> u64 {
        self.anomalies.values().map(|a| a.count).sum()
    }

    /// Controls the census saw that are not paired one-to-one (information for pairing).
    #[must_use]
    pub fn unbalanced_controls(&self) -> u64 {
        self.controls
            .iter()
            .filter(|c| c.unanswered > 0 || c.orphan_replies > 0)
            .count() as u64
    }
}

/// The analyzer: feed it messages, then [`Analyzer::finish`].
pub struct Analyzer {
    abi: GspAbi,
    tag: DriverVersion,
    ogkm: OgkmIndex,
    sources: Vec<SourceInfo>,
    seq: HashMap<StreamKey, SeqState>,
    declared_loss: HashSet<StreamKey>,
    anomalies: BTreeMap<AnomalyKind, Anomaly>,
    declared_followups: u64,
    missing_messages: u64,
    nonzero_padding: u64,
    rpc_sequence_nonzero: u64,
    messages: [u64; 2],
    functions: BTreeMap<(Dir, u32), FuncRow>,
    controls: BTreeMap<u32, ControlAcc>,
    pending: HashMap<PairKey, Pending>,
    large: HashMap<StreamKey, Assembler>,
    large_joined: u64,
    tap: Option<MessageTap>,
    record_tap: Option<MessageRecordTap>,
    bodies: Option<Bodies>,
}

/// A callback that sees every message run exactly as it reached the analyzer (tests use it to
/// compare reassembled bytes with the bytes that were sent).
pub type MessageTap = Box<dyn FnMut(Dir, &[u8])>;

/// A diagnostic tap with source and queue-instance context. The bytes remain
/// host-only; exporters must select metadata explicitly, never copy payloads.
pub type MessageRecordTap = Box<dyn for<'a> FnMut(usize, &In<'a>)>;

fn dir_ix(d: Dir) -> usize {
    match d {
        Dir::Cmd => 0,
        Dir::Status => 1,
    }
}

fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

impl std::fmt::Debug for Analyzer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Analyzer")
            .field("tag", &self.tag)
            .finish_non_exhaustive()
    }
}

impl Analyzer {
    /// New analyzer for one decode tag. `abi` must be the bundle for that tag.
    #[must_use]
    pub fn new(abi: GspAbi, tag: DriverVersion, ogkm: OgkmIndex) -> Analyzer {
        Analyzer {
            abi,
            tag,
            ogkm,
            sources: Vec::new(),
            seq: HashMap::new(),
            declared_loss: HashSet::new(),
            anomalies: BTreeMap::new(),
            declared_followups: 0,
            missing_messages: 0,
            nonzero_padding: 0,
            rpc_sequence_nonzero: 0,
            messages: [0, 0],
            functions: BTreeMap::new(),
            controls: BTreeMap::new(),
            pending: HashMap::new(),
            large: HashMap::new(),
            large_joined: 0,
            tap: None,
            record_tap: None,
            bodies: None,
        }
    }

    /// Keep every distinct control body of a **scrubbed** source for [`Report::bodies`].
    /// Bodies of a source without the scrubbed flag are never kept: only a scrub decides which
    /// bytes may leave the trusted host, and its fill pattern is never recorded as a body.
    pub fn collect_bodies(&mut self) {
        self.bodies = Some(BTreeMap::new());
    }

    /// Install a [`MessageTap`].
    pub fn set_tap(&mut self, tap: MessageTap) {
        self.tap = Some(tap);
    }

    /// Install a diagnostic tap retaining source/queue identity across restarts.
    pub fn set_record_tap(&mut self, tap: MessageRecordTap) {
        self.record_tap = Some(tap);
    }

    /// The ABI bundle (sources need the element size).
    #[must_use]
    pub fn abi(&self) -> &GspAbi {
        &self.abi
    }

    /// Start a new source; later messages belong to it.
    pub fn begin_source(&mut self, label: &str, format: &str) {
        self.sources.push(SourceInfo {
            label: label.to_string(),
            format: format.to_string(),
            ..SourceInfo::default()
        });
    }

    /// Mark the current source as having gone through the scrub stage.
    pub fn mark_scrubbed(&mut self) {
        if let Some(s) = self.sources.last_mut() {
            s.scrubbed = true;
        }
        self.fact("scrubbed", true);
    }

    /// Attach a non-board fact to the current source.
    pub fn fact(&mut self, key: &str, value: impl ToString) {
        if let Some(s) = self.sources.last_mut() {
            s.facts.push((key.to_string(), value.to_string()));
        }
    }

    /// Count a record read from the current source.
    pub fn record_read(&mut self) {
        if let Some(s) = self.sources.last_mut() {
            s.records += 1;
        }
    }

    /// Record an anomaly. Counts are exact; examples are capped.
    pub fn anomaly(&mut self, kind: AnomalyKind, detail: String) {
        let a = self.anomalies.entry(kind).or_default();
        a.count += 1;
        if a.examples.len() < EXAMPLE_CAP {
            let src = self.sources.len().saturating_sub(1);
            a.examples.push(format!("src{src}: {detail}"));
        }
    }

    /// A loss marker (or equivalent) for a queue: the next gap on it is "declared".
    pub fn declare_loss(&mut self, a: u64, b: u64, dir: Dir) {
        let key = StreamKey {
            source: self.sources.len().saturating_sub(1),
            a,
            b,
            dir,
        };
        self.declared_loss.insert(key);
    }

    fn fail(&mut self, m: &In<'_>, what: &str, detail: impl std::fmt::Debug) {
        self.anomaly(
            AnomalyKind::ParseFailure,
            format!(
                "rec{} {:?} q({:#x},{:#x}): {what}: {detail:?}",
                m.ordinal, m.dir, m.a, m.b
            ),
        );
    }

    fn check_seq(&mut self, key: StreamKey, seq: u32, ordinal: u64) {
        let declared = self.declared_loss.remove(&key);
        match self.seq.get_mut(&key) {
            None => {
                if seq != 0 {
                    self.anomaly(
                        AnomalyKind::PrefixMissing,
                        format!(
                            "rec{ordinal} {:?} q({:#x},{:#x}): first sequence is {seq}, not 0",
                            key.dir, key.a, key.b
                        ),
                    );
                    self.missing_messages += u64::from(seq);
                    if declared {
                        self.declared_followups += 1;
                    }
                }
                self.seq.insert(
                    key,
                    SeqState {
                        expected: seq.wrapping_add(1),
                    },
                );
            }
            Some(st) => {
                let expected = st.expected;
                let delta = seq.wrapping_sub(expected);
                if delta == 0 {
                    st.expected = seq.wrapping_add(1);
                    return;
                }
                // A forward jump resynchronises on the number seen; a backwards or repeated
                // number leaves the high-water mark alone, so a late message is one rewind
                // (and the gap that preceded it), not a second gap afterwards.
                if delta < 0x8000_0000 {
                    st.expected = seq.wrapping_add(1);
                }
                let (kind, text) = if delta < 0x8000_0000 {
                    self.missing_messages += u64::from(delta);
                    (
                        AnomalyKind::SeqGap,
                        format!("expected {expected}, got {seq} ({delta} missing)"),
                    )
                } else {
                    (
                        AnomalyKind::SeqRewind,
                        format!("expected {expected}, got {seq} (backwards or repeated)"),
                    )
                };
                if declared {
                    self.declared_followups += 1;
                }
                self.anomaly(
                    kind,
                    format!(
                        "rec{ordinal} {:?} q({:#x},{:#x}): {text}{}",
                        key.dir,
                        key.a,
                        key.b,
                        if declared {
                            " [after a declared loss]"
                        } else {
                            ""
                        }
                    ),
                );
            }
        }
    }

    /// Decode and check one message.
    #[allow(clippy::too_many_lines)]
    pub fn message(&mut self, m: In<'_>) {
        let source = self.sources.len().saturating_sub(1);
        let key = StreamKey {
            source,
            a: m.a,
            b: m.b,
            dir: m.dir,
        };
        self.messages[dir_ix(m.dir)] += 1;
        if let Some(t) = self.record_tap.as_mut() {
            t(source, &m);
        }
        if let Some(t) = self.tap.as_mut() {
            t(m.dir, m.bytes);
        }
        if let Some(s) = self.sources.last_mut() {
            s.messages += 1;
        }
        let layout = self.abi.element;
        let table = self.abi.driver;

        if let Some(o) = m.obs
            && o.missing_before > 0
        {
            self.anomaly(
                AnomalyKind::ObserverLoss,
                format!(
                    "rec{} {:?}: observer declares {} missing before this record",
                    m.ordinal, m.dir, o.missing_before
                ),
            );
        }

        let seq = match peek_seq_num(&layout, m.bytes) {
            Ok(s) => s,
            Err(e) => {
                self.fail(&m, "no sequence number", e);
                return;
            }
        };
        self.check_seq(key, seq, m.ordinal);

        let len = match peek_len(&layout, m.bytes, m.element_size, self.abi.element_size_max) {
            Ok(l) => l,
            Err(e) => {
                self.fail(&m, "length", e);
                return;
            }
        };
        match peek_elem_count(&layout, m.bytes) {
            Ok(Some(n)) if n != len.elements() => self.anomaly(
                AnomalyKind::ElemCountMismatch,
                format!(
                    "rec{} {:?}: elemCount {n} but length {} needs {}",
                    m.ordinal,
                    m.dir,
                    len.rpc_length(),
                    len.elements()
                ),
            ),
            _ => {}
        }
        let msg_len = len.msg_len() as usize;
        if m.exact {
            let want = msg_len.next_multiple_of(8);
            if m.bytes.len() != want {
                self.anomaly(
                    AnomalyKind::TrailingBytes,
                    format!(
                        "rec{} {:?}: {} bytes carried, declared length rounds to {want}",
                        m.ordinal,
                        m.dir,
                        m.bytes.len()
                    ),
                );
            }
        } else {
            let want = len.elements() as usize * m.element_size as usize;
            if m.bytes.len() != want {
                self.anomaly(
                    AnomalyKind::TrailingBytes,
                    format!(
                        "rec{} {:?}: run is {} bytes, {} elements of {} expected",
                        m.ordinal,
                        m.dir,
                        m.bytes.len(),
                        len.elements(),
                        m.element_size
                    ),
                );
            }
            if m.bytes
                .get(msg_len..)
                .is_some_and(|t| t.iter().any(|&x| x != 0))
            {
                self.nonzero_padding += 1;
            }
        }

        let inc = match decode_message(&layout, m.bytes, len, seq, &table) {
            Ok(i) => i,
            Err(e) => {
                self.fail(&m, "decode", e);
                return;
            }
        };
        let env = inc.envelope;

        if env.header_version != self.abi.rpc.header_version {
            self.anomaly(
                AnomalyKind::HeaderVersion,
                format!(
                    "rec{}: header_version {:#x}, expected {:#x}",
                    m.ordinal, env.header_version, self.abi.rpc.header_version
                ),
            );
        }
        if let Some(o) = m.obs {
            let mut bad = Vec::new();
            if o.queue_sequence != seq {
                bad.push(format!(
                    "queue_sequence {} vs payload {seq}",
                    o.queue_sequence
                ));
            }
            if o.rpc_sequence != env.sequence {
                bad.push(format!(
                    "rpc_sequence {} vs payload {}",
                    o.rpc_sequence, env.sequence
                ));
            }
            if o.rpc_function != env.function {
                bad.push(format!(
                    "rpc_function {} vs payload {}",
                    o.rpc_function, env.function
                ));
            }
            if o.rpc_result != env.rpc_result {
                bad.push(format!(
                    "rpc_result {:#x} vs payload {:#x}",
                    o.rpc_result, env.rpc_result
                ));
            }
            if o.rpc_version != env.header_version {
                bad.push(format!(
                    "rpc_version {:#x} vs payload {:#x}",
                    o.rpc_version, env.header_version
                ));
            }
            if o.payload_bytes as usize != m.bytes.len() {
                bad.push(format!(
                    "payload_bytes {} vs {}",
                    o.payload_bytes,
                    m.bytes.len()
                ));
            }
            if !bad.is_empty() {
                self.anomaly(
                    AnomalyKind::MetaMismatch,
                    format!("rec{}: {}", m.ordinal, bad.join("; ")),
                );
            }
        }
        if env.sequence != 0 {
            self.rpc_sequence_nonzero += 1;
        }

        let name = self.ogkm.rpc_name(env.function).map(str::to_string);
        if name.is_none() {
            self.anomaly(
                AnomalyKind::UnknownFunction,
                format!(
                    "rec{} {:?}: function {:#x} is not defined at ogkm {}.{}.{:02}",
                    m.ordinal, m.dir, env.function, self.tag.major, self.tag.minor, self.tag.patch
                ),
            );
        }
        let row = self.functions.entry((m.dir, env.function)).or_default();
        row.name = name;
        row.count += 1;
        row.bytes += u64::from(len.msg_len());
        if m.dir == Dir::Status {
            *row.results.entry(env.rpc_result).or_default() += 1;
        }

        // Large controls: the head carries the first `maxRpcSize` bytes, `CONTINUATION_RECORD`s
        // carry the rest. `kf_gsp::large::Assembler` is the transport's own joiner.
        let cmd = RpcCommand::from_incoming(&self.abi.rpc, &inc);
        let declared = if cmd.function == RpcFunction::RmControl {
            table
                .decode_rpc_control(&cmd.payload)
                .ok()
                .map(|r| r.params_at + r.params_size as usize)
        } else {
            None
        };
        let asm = self.large.entry(key).or_default();
        let step = asm.step(&cmd, declared);
        asm.commit();
        match step {
            Step::NotLarge => {
                if cmd.function == RpcFunction::ContinuationRecord {
                    self.fail(
                        &m,
                        "continuation record with no open large message",
                        cmd.sequence,
                    );
                } else if cmd.function == RpcFunction::RmControl {
                    self.control(&m, env.sequence, env.rpc_result, &cmd.payload);
                }
            }
            Step::Held => {}
            Step::Complete { whole, fragments } => {
                self.large_joined += 1;
                let _ = fragments;
                self.control(&m, whole.sequence, env.rpc_result, &whole.payload);
            }
            Step::Refused(why) => self.fail(&m, "large RPC", why.to_string()),
        }
    }

    fn control(&mut self, m: &In<'_>, rpc_sequence: u32, rpc_result: u32, payload: &[u8]) {
        let table = self.abi.driver;
        let req = match table.decode_rpc_control(payload) {
            Ok(r) => r,
            Err(e) => {
                self.fail(m, "control header", e);
                return;
            }
        };
        let wire = table.rm_control_wire();
        let status = u32_at(payload, wire.status_off).unwrap_or(0);
        let observed = payload.len().saturating_sub(req.params_at);
        let declared = req.params_size as usize;
        if observed < declared {
            self.anomaly(
                AnomalyKind::ParamsTruncated,
                format!(
                    "rec{} {:?} cmd {:#x}: declares {declared} param bytes, message carries {observed}",
                    m.ordinal, m.dir, req.cmd
                ),
            );
        } else if observed > declared {
            self.anomaly(
                AnomalyKind::ControlExtraBytes,
                format!(
                    "rec{} {:?} cmd {:#x}: declares {declared} param bytes, message carries {observed}",
                    m.ordinal, m.dir, req.cmd
                ),
            );
        }
        let n = declared.min(observed);
        let params = payload[req.params_at..req.params_at + n].to_vec();
        // A scrubbed body is the fill pattern in a source that says it was scrubbed; it is
        // counted as such and never decoded as if it were the driver's bytes.
        let scrubbed =
            self.sources.last().is_some_and(|s| s.scrubbed) && crate::scrub::is_filled(&params);
        let params: Option<Vec<u8>> = if scrubbed { None } else { Some(params) };
        if let (Some(b), Some(p)) = (self.bodies.as_mut(), params.as_ref())
            && !p.is_empty()
            && self.sources.last().is_some_and(|s| s.scrubbed)
        {
            let st = if m.dir == Dir::Status { status } else { 0 };
            *b.entry((req.cmd, m.dir, st, p.clone())).or_default() += 1;
        }
        let acc = self.controls.entry(req.cmd).or_default();
        if scrubbed {
            acc.scrubbed += 1;
        }
        let pk = PairKey {
            source: self.sources.len().saturating_sub(1),
            a: m.a,
            b: m.b,
            rpc_sequence,
            client: req.client,
            object: req.object,
            cmd: req.cmd,
        };
        let pend = self.pending.entry(pk).or_default();
        match m.dir {
            Dir::Cmd => {
                acc.requests += 1;
                if kf_abi::rpc_params_are_serialized(req.rmapi_rpc_flags) {
                    acc.serialized += 1;
                }
                *acc.req_sizes.entry(req.params_size).or_default() += 1;
                if let Some(rep) = pend.reps.pop_front() {
                    acc.paired += 1;
                    if rep.is_some() && rep == params {
                        acc.echo += 1;
                    }
                } else {
                    pend.reqs.push_back(params);
                }
            }
            Dir::Status => {
                acc.replies += 1;
                *acc.rep_sizes.entry(req.params_size).or_default() += 1;
                *acc.statuses.entry(status).or_default() += 1;
                *acc.rpc_results.entry(rpc_result).or_default() += 1;
                if let Some(p) = &params {
                    acc.reply_hashes.insert(sha256(p));
                }
                if let Some(rq) = pend.reqs.pop_front() {
                    acc.paired += 1;
                    if rq.is_some() && rq == params {
                        acc.echo += 1;
                    }
                } else {
                    pend.reps.push_back(params);
                }
            }
        }
    }

    /// Finish: resolve pairing leftovers and produce the report.
    #[must_use]
    pub fn finish(self) -> Report {
        let mut unanswered: BTreeMap<u32, u64> = BTreeMap::new();
        let mut orphans: BTreeMap<u32, u64> = BTreeMap::new();
        for (k, p) in &self.pending {
            if !p.reqs.is_empty() {
                *unanswered.entry(k.cmd).or_default() += p.reqs.len() as u64;
            }
            if !p.reps.is_empty() {
                *orphans.entry(k.cmd).or_default() += p.reps.len() as u64;
            }
        }
        let controls = self
            .controls
            .into_iter()
            .map(|(cmd, a)| ControlRow {
                cmd,
                name: self.ogkm.controls_at_tag.get(&cmd).cloned(),
                requests: a.requests,
                replies: a.replies,
                paired: a.paired,
                unanswered: unanswered.get(&cmd).copied().unwrap_or(0),
                orphan_replies: orphans.get(&cmd).copied().unwrap_or(0),
                echo: a.echo,
                serialized: a.serialized,
                req_sizes: a.req_sizes,
                rep_sizes: a.rep_sizes,
                statuses: a.statuses,
                rpc_results: a.rpc_results,
                distinct_replies: a.reply_hashes.len() as u64,
                scrubbed: a.scrubbed,
                ogkm: self.ogkm.control(cmd),
            })
            .collect();
        Report {
            tag: self.tag,
            sources: self.sources,
            messages: self.messages,
            anomalies: self.anomalies,
            declared_loss_followups: self.declared_followups,
            missing_messages: self.missing_messages,
            nonzero_padding: self.nonzero_padding,
            rpc_sequence_nonzero: self.rpc_sequence_nonzero,
            large_joined: self.large_joined,
            functions: self.functions,
            controls,
            bodies: self.bodies,
        }
    }
}
