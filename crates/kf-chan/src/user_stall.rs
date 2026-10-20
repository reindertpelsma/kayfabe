//! ★ 2026-10-20 MEASUREMENT ONLY -- `KF3_DIAG_USER_STALL=<ms>` (default OFF; the pure half).
//!
//! STATUS: LIVE, 2026-10-20 (DIAGNOSTIC; never a decision input, never needed to boot).
//!
//! A Windows USER-ring relay publishes the guest's entries onto its PRIVATE ring and the PBDMA
//! consumes them. When the private ring does not drain (`GP_GET != GP_PUT` for longer than the
//! threshold) this module's pieces say WHY the GPU is where it is: the stall detector
//! ([`Detector`], re-armed whenever `GP_GET` moves), the decoder of a pushbuffer's method stream
//! ([`decode`]: SEM_ADDR/PAYLOAD/EXECUTE, SET_REFERENCE, NON_STALL_INTERRUPT, WFI, the copy
//! engine's release methods), the choice of the private entry the PBDMA is inside
//! ([`pick_segment`]) and the bounded text block ([`render`]). No I/O, no clock: the glue
//! (`crates/kf-qemu/src/userdiag.rs`) reads the memory and prints the lines.
//!
//! Hostile guest: everything printed from guest memory is an integer in hex; the decode is bounded
//! by [`MAX_METHODS`], the read by [`MAX_PB_BYTES`], the output by [`MAX_LINES`] lines of at most
//! [`MAX_LINE_CHARS`] characters.

use crate::user_loop::{Peek, Wait};
use kf_abi::submit::{GpEntryKind, MethodForm, gp_entry_classify, method_header_decode};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// The environment variable (milliseconds; unset, empty or `0` = off).
pub const ENV: &str = "KF3_DIAG_USER_STALL";
/// Largest accepted threshold (one hour); larger values are clamped.
pub const MAX_MS: u64 = 3_600_000;
/// Most methods one decode processes.
pub const MAX_METHODS: usize = 64;
/// Most pushbuffer bytes one report reads.
pub const MAX_PB_BYTES: usize = 2048;
/// Most semaphore words one report reads.
pub const MAX_PROBES: usize = 8;
/// Most private-ring rows one report lists.
pub const MAX_RING_ROWS: usize = 8;
/// Most lines one rendered block has.
pub const MAX_LINES: usize = 112;
/// Most characters of one rendered line.
pub const MAX_LINE_CHARS: usize = 320;
/// How often the cumulative counter line may be printed.
pub const PERIOD_MS: u64 = 10_000;

/// What the environment variable selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    /// Off (the default).
    Off,
    /// On, with this threshold in milliseconds.
    On(u64),
    /// Not a number: off, and said so once.
    Invalid,
}

/// Parse the variable's value.
#[must_use]
pub fn parse_setting(value: Option<&str>) -> Setting {
    let Some(v) = value.map(str::trim) else {
        return Setting::Off;
    };
    if v.is_empty() {
        return Setting::Off;
    }
    if !v.bytes().all(|b| b.is_ascii_digit()) {
        return Setting::Invalid;
    }
    match v.parse::<u64>() {
        Ok(0) => Setting::Off,
        Ok(n) => Setting::On(n.min(MAX_MS)),
        // more digits than a u64 holds: certainly beyond the clamp
        Err(_) => Setting::On(MAX_MS),
    }
}

/// The detector's answer for one look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing to report.
    Quiet,
    /// The private ring has not drained for `stalled_ms`: report now (once per condition).
    Fire {
        /// How long `GP_GET` has been unmoved while entries were outstanding.
        stalled_ms: u64,
    },
}

/// "Private ring not draining", re-armed whenever `GP_GET` moves or the ring empties.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Detector {
    get: Option<u32>,
    since_ms: u64,
    fired: bool,
}

impl Detector {
    /// A fresh detector.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            get: None,
            since_ms: 0,
            fired: false,
        }
    }
    /// One look at the private `GP_GET` / `GP_PUT` (ring of `entries`, a power of two) at `now_ms`.
    pub fn observe(
        &mut self,
        now_ms: u64,
        get: u32,
        put: u32,
        entries: u32,
        threshold_ms: u64,
    ) -> Verdict {
        // words outside the ring are not a position; a zero threshold is "off"
        if threshold_ms == 0 || !entries.is_power_of_two() || get >= entries || put >= entries {
            *self = Self::new();
            return Verdict::Quiet;
        }
        // nothing outstanding: the ring drained, the condition (if any) is over
        if get == put {
            *self = Self::new();
            return Verdict::Quiet;
        }
        if self.get != Some(get) {
            // GET moved (or this is the first look): a new condition whose clock starts now
            *self = Self {
                get: Some(get),
                since_ms: now_ms,
                fired: false,
            };
            return Verdict::Quiet;
        }
        if self.fired {
            return Verdict::Quiet;
        }
        let stalled_ms = now_ms.saturating_sub(self.since_ms);
        if stalled_ms >= threshold_ms {
            self.fired = true;
            return Verdict::Fire { stalled_ms };
        }
        Verdict::Quiet
    }
    /// Whether this condition was already reported.
    #[must_use]
    pub const fn fired(&self) -> bool {
        self.fired
    }
}

/// `SEM_EXECUTE.OPERATION` (`clc56f.h` `NVC56F_SEM_EXECUTE_OPERATION_*`, `2:0`), or the legacy
/// `SEMAPHORED.OPERATION` (`4:0`) folded onto it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemOp {
    /// `ACQUIRE` (0): wait until the word equals the payload.
    Acquire,
    /// `RELEASE` (1).
    Release,
    /// `ACQ_STRICT_GEQ` (2).
    AcqStrictGeq,
    /// `ACQ_CIRC_GEQ` (3).
    AcqCircGeq,
    /// `ACQ_AND` (4).
    AcqAnd,
    /// `ACQ_NOR` (5).
    AcqNor,
    /// `REDUCTION` (6).
    Reduction,
    /// The legacy `SEMAPHORED_OPERATION_ACQ_GEQ`.
    AcqGeq,
    /// An operation value the class header does not define.
    Reserved(u32),
}

impl SemOp {
    /// The operation of a `SEM_EXECUTE` word.
    #[must_use]
    pub fn of_execute(raw: u32) -> Self {
        match raw & 0x7 {
            0 => Self::Acquire,
            1 => Self::Release,
            2 => Self::AcqStrictGeq,
            3 => Self::AcqCircGeq,
            4 => Self::AcqAnd,
            5 => Self::AcqNor,
            6 => Self::Reduction,
            v => Self::Reserved(v),
        }
    }
    /// The operation of a legacy `SEMAPHORED` word.
    #[must_use]
    pub fn of_legacy(raw: u32) -> Self {
        match raw & 0x1F {
            0x01 => Self::Acquire,
            0x02 => Self::Release,
            0x04 => Self::AcqGeq,
            0x08 => Self::AcqAnd,
            0x10 => Self::Reduction,
            v => Self::Reserved(v),
        }
    }
    /// Whether the operation waits for a value.
    #[must_use]
    pub fn is_acquire(self) -> bool {
        matches!(
            self,
            Self::Acquire
                | Self::AcqStrictGeq
                | Self::AcqCircGeq
                | Self::AcqAnd
                | Self::AcqNor
                | Self::AcqGeq
        )
    }
    /// The name printed.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Acquire => "ACQUIRE",
            Self::Release => "RELEASE",
            Self::AcqStrictGeq => "ACQ_STRICT_GEQ",
            Self::AcqCircGeq => "ACQ_CIRC_GEQ",
            Self::AcqAnd => "ACQ_AND",
            Self::AcqNor => "ACQ_NOR",
            Self::Reduction => "REDUCTION",
            Self::AcqGeq => "ACQ_GEQ",
            Self::Reserved(_) => "RESERVED",
        }
    }
}

/// One decoded `SEM_EXECUTE` (or legacy `SEMAPHORED`), with the address and payload latched by the
/// methods before it in the same stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exec {
    /// The operation.
    pub op: SemOp,
    /// The method word as written.
    pub raw: u32,
    /// The legacy `SEMAPHORE*` methods (`0x10..0x1c`), not `SEM_*` (`0x5c..0x6c`).
    pub legacy: bool,
    /// The semaphore's GPU VA, when both halves were written earlier in the stream.
    pub addr: Option<u64>,
    /// The payload (32 or 64 bits), when written earlier in the stream.
    pub payload: Option<u64>,
    /// `PAYLOAD_SIZE == 64BIT`.
    pub payload64: bool,
    /// `RELEASE_WFI` enabled.
    pub release_wfi: bool,
    /// `RELEASE_TIMESTAMP` (new family) / 16-byte release (legacy).
    pub timestamp: bool,
    /// `ACQUIRE_SWITCH_TSG` enabled.
    pub acquire_switch: bool,
    /// `REDUCTION` (`30:27`), meaningful for [`SemOp::Reduction`].
    pub reduction: u32,
    /// `REDUCTION_FORMAT == UNSIGNED`.
    pub reduction_unsigned: bool,
}

/// What one method was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `SET_OBJECT` (0x0): binds `class` (`15:0`) on the subchannel.
    SetObject {
        /// The class.
        class: u32,
    },
    /// `SEM_ADDR_LO` (0x5c) or legacy `SEMAPHOREB` (0x14).
    SemAddrLo,
    /// `SEM_ADDR_HI` (0x60) or legacy `SEMAPHOREA` (0x10).
    SemAddrHi,
    /// `SEM_PAYLOAD_LO` (0x64) or legacy `SEMAPHOREC` (0x18).
    SemPayloadLo,
    /// `SEM_PAYLOAD_HI` (0x68).
    SemPayloadHi,
    /// `SEM_EXECUTE` (0x6c) or legacy `SEMAPHORED` (0x1c).
    SemExecute(Exec),
    /// `NON_STALL_INTERRUPT` (0x20).
    NonStallInterrupt,
    /// `SET_REFERENCE` (0x50).
    SetReference,
    /// `WFI` (0x78).
    Wfi,
    /// The copy engine's `SET_SEMAPHORE_A` (0x240).
    CeSemA,
    /// The copy engine's `SET_SEMAPHORE_B` (0x244).
    CeSemB,
    /// The copy engine's `SET_SEMAPHORE_PAYLOAD` (0x248).
    CeSemPayload,
    /// The copy engine's `LAUNCH_DMA` (0x300): `semaphore_type` is `4:3`.
    CeLaunch {
        /// `SEMAPHORE_TYPE`.
        semaphore_type: u32,
    },
    /// Any other method.
    Other,
}

/// One decoded method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decoded {
    /// Index of the word that carries the datum (the header's index is `word - 1` for the first
    /// datum of an inline packet).
    pub word: usize,
    /// Subchannel.
    pub subchannel: u32,
    /// Method byte offset.
    pub method: u32,
    /// The datum.
    pub data: u32,
    /// What it was.
    pub kind: Kind,
}

/// A decoded stream and how the decode ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stream {
    /// The methods, in order, at most the cap.
    pub items: Vec<Decoded>,
    /// Stopped because the method cap was reached with words left.
    pub capped: bool,
    /// The words ended inside a packet.
    pub truncated: bool,
    /// Index of a header word that has no defined format (the decode stopped there).
    pub malformed_at: Option<usize>,
    /// An `END_PB_SEGMENT` header was reached at this word.
    pub ended_at: Option<usize>,
    /// Methods processed (equals `items.len()`).
    pub methods: usize,
}

/// Decode a pushbuffer method stream, at most `max_methods` methods.
#[must_use]
pub fn decode(words: &[u32], max_methods: usize) -> Stream {
    let mut s = Stream::default();
    let mut latch = Latch::default();
    let mut classes = [None::<u32>; 8];
    let mut i = 0usize;
    while i < words.len() {
        let hdr = i;
        let Some(h) = method_header_decode(words[i]) else {
            s.malformed_at = Some(hdr);
            return s;
        };
        i += 1;
        match h.form {
            MethodForm::EndPbSegment => {
                s.ended_at = Some(hdr);
                return s;
            }
            MethodForm::SubDeviceMask => {}
            // sized, never interpreted: a header this decoder cannot size would desynchronise it
            MethodForm::Legacy => {
                i = i.saturating_add(h.arg_words);
                if i > words.len() {
                    s.truncated = true;
                    return s;
                }
            }
            MethodForm::Immediate => {
                if !push(
                    &mut s,
                    &mut latch,
                    &mut classes,
                    max_methods,
                    hdr,
                    h.subchannel,
                    h.method,
                    h.immd,
                ) {
                    return s;
                }
            }
            MethodForm::Incrementing | MethodForm::NonIncrementing | MethodForm::IncrementOnce => {
                for k in 0..h.arg_words {
                    let Some(&data) = words.get(i) else {
                        s.truncated = true;
                        return s;
                    };
                    let step = match h.form {
                        MethodForm::Incrementing => k,
                        MethodForm::IncrementOnce => usize::from(k > 0),
                        _ => 0,
                    };
                    let method = h
                        .method
                        .saturating_add(u32::try_from(step * 4).unwrap_or(u32::MAX));
                    if !push(
                        &mut s,
                        &mut latch,
                        &mut classes,
                        max_methods,
                        i,
                        h.subchannel,
                        method,
                        data,
                    ) {
                        return s;
                    }
                    i += 1;
                }
            }
        }
    }
    s
}

/// The `SEM_*` / `SEMAPHORE*` / copy-engine values latched so far in one decode.
#[derive(Default)]
struct Latch {
    lo: Option<u32>,
    hi: Option<u32>,
    pay_lo: Option<u32>,
    pay_hi: Option<u32>,
}

/// Classify and record one method; `false` when the method cap stopped the decode.
#[allow(clippy::too_many_arguments)]
fn push(
    s: &mut Stream,
    latch: &mut Latch,
    classes: &mut [Option<u32>; 8],
    max_methods: usize,
    word: usize,
    subchannel: u32,
    method: u32,
    data: u32,
) -> bool {
    if s.items.len() >= max_methods {
        s.capped = true;
        return false;
    }
    let kind = classify(latch, classes, subchannel, method, data);
    s.items.push(Decoded {
        word,
        subchannel,
        method,
        data,
        kind,
    });
    s.methods = s.items.len();
    true
}

fn classify(
    latch: &mut Latch,
    classes: &mut [Option<u32>; 8],
    sub: u32,
    method: u32,
    data: u32,
) -> Kind {
    match method {
        // NVC56F_SET_OBJECT_NVCLASS is 15:0; 20:16 is the engine
        0x00 => {
            let class = data & 0xFFFF;
            if let Some(c) = classes.get_mut(sub as usize) {
                *c = Some(class);
            }
            Kind::SetObject { class }
        }
        // legacy SEMAPHOREA (OFFSET_UPPER 7:0) .. SEMAPHORED
        0x10 => {
            latch.hi = Some(data & 0xFF);
            Kind::SemAddrHi
        }
        0x14 => {
            latch.lo = Some(data & !3);
            Kind::SemAddrLo
        }
        0x18 => {
            latch.pay_lo = Some(data);
            Kind::SemPayloadLo
        }
        0x1C => Kind::SemExecute(exec_of(latch, data, true)),
        0x20 => Kind::NonStallInterrupt,
        0x50 => Kind::SetReference,
        // SEM_ADDR_LO_OFFSET 31:2, SEM_ADDR_HI_OFFSET 24:0 (clc86f.h; 7:0 on clc56f)
        0x5C => {
            latch.lo = Some(data & !3);
            Kind::SemAddrLo
        }
        0x60 => {
            latch.hi = Some(data & 0x01FF_FFFF);
            Kind::SemAddrHi
        }
        0x64 => {
            latch.pay_lo = Some(data);
            Kind::SemPayloadLo
        }
        0x68 => {
            latch.pay_hi = Some(data);
            Kind::SemPayloadHi
        }
        0x6C => Kind::SemExecute(exec_of(latch, data, false)),
        0x78 => Kind::Wfi,
        m if m >= 0x100
            && classes
                .get(sub as usize)
                .copied()
                .flatten()
                .is_some_and(is_copy_class) =>
        {
            match m {
                0x240 => Kind::CeSemA,
                0x244 => Kind::CeSemB,
                0x248 => Kind::CeSemPayload,
                0x300 => Kind::CeLaunch {
                    semaphore_type: (data >> 3) & 0x3,
                },
                _ => Kind::Other,
            }
        }
        _ => Kind::Other,
    }
}

fn exec_of(latch: &Latch, raw: u32, legacy: bool) -> Exec {
    let addr = match (latch.lo, latch.hi) {
        (Some(lo), Some(hi)) => Some((u64::from(hi) << 32) | u64::from(lo)),
        _ => None,
    };
    let payload64 = !legacy && raw & (1 << 24) != 0;
    let payload = if payload64 {
        match (latch.pay_lo, latch.pay_hi) {
            (Some(lo), Some(hi)) => Some((u64::from(hi) << 32) | u64::from(lo)),
            _ => None,
        }
    } else {
        latch.pay_lo.map(u64::from)
    };
    Exec {
        op: if legacy {
            SemOp::of_legacy(raw)
        } else {
            SemOp::of_execute(raw)
        },
        raw,
        legacy,
        addr,
        payload,
        payload64,
        // clc56f.h: SEM_EXECUTE_RELEASE_WFI_EN = 1; legacy SEMAPHORED_RELEASE_WFI_EN = 0
        release_wfi: if legacy {
            raw & (1 << 20) == 0
        } else {
            raw & (1 << 20) != 0
        },
        // SEM_EXECUTE_RELEASE_TIMESTAMP_EN = 1 (25); legacy RELEASE_SIZE_16BYTE = 0 (24)
        timestamp: if legacy {
            raw & (1 << 24) == 0
        } else {
            raw & (1 << 25) != 0
        },
        acquire_switch: raw & (1 << 12) != 0,
        reduction: (raw >> 27) & 0xF,
        reduction_unsigned: raw & (1 << 31) != 0,
    }
}

/// The stream's methods as a list (see [`decode`]).
#[must_use]
pub fn decode_methods(words: &[u32]) -> Vec<Decoded> {
    decode(words, MAX_METHODS).items
}

/// Whether `class` is a copy-engine class (`xxB5`).
#[must_use]
pub fn is_copy_class(class: u32) -> bool {
    class > 0xFF && class <= 0xFFFF && class & 0xFF == 0xB5
}

/// What the name of the acquire operation says about whether `current` passes `awaited`
/// (`None` when the name does not say: NOR, REDUCTION, RELEASE, reserved). Inferred from the
/// operation name, never measured.
#[must_use]
pub fn name_says_pass(op: SemOp, current: u64, awaited: u64, payload64: bool) -> Option<bool> {
    let (c, a) = if payload64 {
        (current, awaited)
    } else {
        (current & 0xFFFF_FFFF, awaited & 0xFFFF_FFFF)
    };
    match op {
        SemOp::Acquire => Some(c == a),
        SemOp::AcqStrictGeq | SemOp::AcqGeq => Some(c >= a),
        SemOp::AcqCircGeq => Some(if payload64 {
            (c.wrapping_sub(a) as i64) >= 0
        } else {
            (c.wrapping_sub(a) as u32 as i32) >= 0
        }),
        SemOp::AcqAnd => Some(c & a != 0),
        SemOp::AcqNor | SemOp::Release | SemOp::Reduction | SemOp::Reserved(_) => None,
    }
}

/// A semaphore word the glue should read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeReq {
    /// Index into [`Stream::items`] of the `SEM_EXECUTE`.
    pub item: usize,
    /// GPU VA.
    pub addr: u64,
    /// Bytes to read (4 or 8).
    pub bytes: usize,
}

/// The acquires of a stream whose address is known, at most [`MAX_PROBES`].
#[must_use]
pub fn probe_plan(stream: &Stream) -> Vec<ProbeReq> {
    stream
        .items
        .iter()
        .enumerate()
        .filter_map(|(item, d)| match d.kind {
            Kind::SemExecute(e) if e.op.is_acquire() => e.addr.map(|addr| ProbeReq {
                item,
                addr,
                bytes: if e.payload64 { 8 } else { 4 },
            }),
            _ => None,
        })
        .take(MAX_PROBES)
        .collect()
}

/// The private ring rows to list: `(first slot, count)`.
#[must_use]
pub fn ring_window(get: u32, put: u32, entries: u32) -> (u32, u32) {
    if !entries.is_power_of_two() {
        return (0, 0);
    }
    let mask = entries - 1;
    let outstanding = put.wrapping_sub(get) & mask;
    let count = (3 + outstanding).min(MAX_RING_ROWS as u32).min(entries);
    (get.wrapping_sub(3) & mask, count)
}

/// The private slot whose segment holds `pb_get`, among `rows` (`(slot, entry)`).
#[must_use]
pub fn pick_segment(rows: &[(u32, u64)], get: u32, entries: u32, pb_get: u64) -> Option<u32> {
    if !entries.is_power_of_two() {
        return None;
    }
    let mask = entries - 1;
    // nearest behind GET first (the PBDMA is inside the entry it popped last), then GET and ahead
    let behind = (1..=3u32).map(|d| get.wrapping_sub(d) & mask);
    let ahead = (0..rows.len() as u32).map(|k| get.wrapping_add(k) & mask);
    behind.chain(ahead).find(|slot| {
        rows.iter().any(|&(s, entry)| {
            s == *slot
                && matches!(
                    gp_entry_classify(entry),
                    GpEntryKind::Segment(e)
                        if e.gpu_va <= pb_get && pb_get - e.gpu_va < e.len_bytes
                )
        })
    })
}

/// The semaphore's value, or why it could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Current {
    /// Read through the relay's placement view.
    Value(u64),
    /// Not readable from this thread (`UNREAD(why)`).
    Unread(&'static str),
}

/// One acquire's evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probe {
    /// Index into [`Stream::items`].
    pub item: usize,
    /// The semaphore's current value.
    pub current: Current,
}

/// One private ring row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Slot.
    pub slot: u32,
    /// The 8-byte entry.
    pub entry: u64,
}

/// The private USERD words the report needs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Private {
    /// `GP_GET` (word 34).
    pub gp_get: u32,
    /// `GP_PUT` (word 35).
    pub gp_put: u32,
    /// `REFERENCE` (word 18).
    pub reference: u32,
    /// `GET` + `GET_HI` (words 17, 24).
    pub pb_get: u64,
    /// `PUT` + `PUT_HI` (words 16, 19).
    pub pb_put: u64,
    /// `TOP_LEVEL_GET` + `_HI` (words 22, 23).
    pub top_level_get: u64,
}

/// What the report found about the segment the PBDMA is inside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// No private row's segment holds `PB_GET`.
    NotFound,
    /// The row was found but its pushbuffer could not be read.
    Unread {
        /// The private slot.
        slot: u32,
        /// The segment's GPU VA.
        va: u64,
        /// The segment's length in bytes.
        len_bytes: u64,
        /// Why.
        why: &'static str,
    },
    /// Read and decoded.
    Decoded {
        /// The private slot.
        slot: u32,
        /// The segment's GPU VA.
        va: u64,
        /// The segment's length in bytes.
        len_bytes: u64,
        /// Bytes read (at most [`MAX_PB_BYTES`]).
        read_bytes: usize,
        /// The decode.
        stream: Stream,
        /// The semaphore evidence, for the acquires with a known address.
        probes: Vec<Probe>,
    },
}

/// Everything one report says. Counters are the relay's own.
#[derive(Debug, Clone)]
pub struct Report {
    /// The channel key `(client, object)`.
    pub key: (u32, u32),
    /// The host token.
    pub host: u32,
    /// How long the private `GP_GET` has been unmoved.
    pub stalled_ms: u64,
    /// The threshold.
    pub threshold_ms: u64,
    /// The guest's `GP_PUT`.
    pub guest_put: Option<u32>,
    /// The guest's `GP_GET`.
    pub guest_get: Option<u32>,
    /// The private USERD words.
    pub private: Private,
    /// What the relay is parked on.
    pub wait: Option<Wait>,
    /// Doorbells counted on the channel, and how long ago the count last changed.
    pub doorbells: u64,
    /// See `doorbells`.
    pub doorbell_ago_ms: Option<u64>,
    /// The last look that rang the relay, and how long ago.
    pub wake: Option<(u64, Peek)>,
    /// How long ago the relay last had a turn.
    pub turn_ago_ms: Option<u64>,
    /// Private descriptors issued / fetched (the relay's receipts).
    pub issued: u64,
    /// See `issued`.
    pub fetched: u64,
    /// Markers issued / seen processed.
    pub marks_issued: u64,
    /// See `marks_issued`.
    pub marks_seen: u64,
    /// The relay's guest-ring cursor.
    pub cursor: u32,
    /// Turns the relay has had (its own counter).
    pub turns: u64,
    /// Entries the lifecycle has forwarded, by mode.
    pub entries: (u64, u64, u64),
    /// The private rows around `GP_GET`.
    pub rows: Vec<Row>,
    /// The segment.
    pub segment: Segment,
    /// Cumulative diagnostic counters (`StallStats::summary`).
    pub totals: String,
}

/// Render the report: at most [`MAX_LINES`] lines of at most [`MAX_LINE_CHARS`] characters, every
/// integer in hex.
#[must_use]
pub fn render(r: &Report) -> Vec<String> {
    let mut out = Out::default();
    let k = format!("{:#x}:{:#x}", r.key.0, r.key.1);
    let p = format!("{PREFIX} {k}");
    out.push(format!(
        "{PREFIX} BEGIN {k} host={:#x} stalled_ms={:#x} threshold_ms={:#x}",
        r.host, r.stalled_ms, r.threshold_ms
    ));
    out.push(format!("{p} TOTALS {}", r.totals));
    out.push(format!(
        "{p} GUEST guest_put={} guest_get={} relay_cursor={:#x}",
        hex_or(r.guest_put, "UNREAD"),
        hex_or(r.guest_get, "UNREAD"),
        r.cursor
    ));
    let v = &r.private;
    out.push(format!(
        "{p} PRIVATE gp_get={:#x} gp_put={:#x} reference={:#x} pb_get={:#x} pb_put={:#x} top_level_get={:#x}",
        v.gp_get, v.gp_put, v.reference, v.pb_get, v.pb_put, v.top_level_get
    ));
    out.push(format!(
        "{p} RELAY wait={} last_doorbell_ago_ms={} doorbells={:#x} last_wake={} last_wake_ago_ms={} last_turn_ago_ms={}",
        wait_text(r.wait),
        hex_or(r.doorbell_ago_ms, "unknown"),
        r.doorbells,
        r.wake.map_or("none", |(_, w)| peek_name(w)),
        hex_or(r.wake.map(|(t, _)| t), "none"),
        hex_or(r.turn_ago_ms, "unknown"),
    ));
    out.push(format!(
        "{p} COUNTERS issued={:#x} fetched={:#x} marks_issued={:#x} marks_seen={:#x} turns={:#x} entries_scan={:#x} entries_pointer={:#x} entries_window={:#x}",
        r.issued, r.fetched, r.marks_issued, r.marks_seen, r.turns, r.entries.0, r.entries.1, r.entries.2
    ));
    let held = match &r.segment {
        Segment::NotFound => None,
        Segment::Unread { slot, .. } | Segment::Decoded { slot, .. } => Some(*slot),
    };
    for row in r.rows.iter().take(MAX_RING_ROWS) {
        let mut line = format!("{p} RING slot={:#x} entry={:#x}", row.slot, row.entry);
        match gp_entry_classify(row.entry) {
            GpEntryKind::Segment(e) => line.push_str(&format!(
                " va={:#x} len={:#x} subroutine={:#x} sync_wait={:#x}",
                e.gpu_va,
                e.len_bytes,
                u8::from(e.subroutine),
                u8::from(e.sync_wait)
            )),
            GpEntryKind::Control { opcode, operand } => {
                line.push_str(&format!(" control opcode={opcode:#x} operand={operand:#x}"));
            }
        }
        if row.slot == v.gp_get {
            line.push_str(" <== GP_GET");
        }
        if held == Some(row.slot) {
            line.push_str(" <== SEGMENT_ROW");
        }
        out.push(line);
    }
    match &r.segment {
        Segment::NotFound => out.push(format!(
            "{p} SEGMENT none: no listed private row holds pb_get={:#x}",
            v.pb_get
        )),
        Segment::Unread {
            slot,
            va,
            len_bytes,
            why,
        } => out.push(format!(
            "{p} SEGMENT slot={slot:#x} va={va:#x} len_bytes={len_bytes:#x} UNREAD({})",
            clean(why)
        )),
        Segment::Decoded {
            slot,
            va,
            len_bytes,
            read_bytes,
            stream,
            probes,
        } => {
            let off = v.pb_get.checked_sub(*va).filter(|o| o < len_bytes);
            let pb_word = off.map(|o| usize::try_from(o / 4).unwrap_or(usize::MAX));
            let beyond = pb_word.is_some_and(|w| w >= read_bytes / 4);
            out.push(format!(
                "{p} SEGMENT slot={slot:#x} va={va:#x} len_bytes={len_bytes:#x} read_bytes={read_bytes:#x} pb_get_off={} methods={:#x} capped={:#x} truncated={:#x} malformed_at={} ended_at={}",
                match (off, beyond) {
                    (None, _) => "outside".to_owned(),
                    (Some(o), true) => format!("{o:#x}(beyond_read_window)"),
                    (Some(o), false) => format!("{o:#x}"),
                },
                stream.methods,
                u8::from(stream.capped),
                u8::from(stream.truncated),
                hex_or(stream.malformed_at.map(|w| w as u64), "none"),
                hex_or(stream.ended_at.map(|w| w as u64), "none"),
            ));
            let mark = pb_word
                .filter(|_| !beyond)
                .and_then(|w| stream.items.iter().position(|i| i.word >= w));
            let (mut others, mut first_other, mut any_acquire) = (0usize, Vec::new(), false);
            for (idx, d) in stream.items.iter().enumerate() {
                let at = format!("+{:#x}", d.word * 4);
                let text = match d.kind {
                    Kind::Other => {
                        others += 1;
                        if first_other.len() < 4 {
                            first_other.push(format!("{:#x}", d.method));
                        }
                        continue;
                    }
                    Kind::SetObject { class } => {
                        format!("SET_OBJECT sub={:#x} class={class:#x}", d.subchannel)
                    }
                    Kind::SemAddrLo => format!("SEM_ADDR_LO value={:#x}", d.data),
                    Kind::SemAddrHi => format!("SEM_ADDR_HI value={:#x}", d.data),
                    Kind::SemPayloadLo => format!("SEM_PAYLOAD_LO value={:#x}", d.data),
                    Kind::SemPayloadHi => format!("SEM_PAYLOAD_HI value={:#x}", d.data),
                    Kind::NonStallInterrupt => "NON_STALL_INTERRUPT".to_owned(),
                    Kind::SetReference => format!("SET_REFERENCE value={:#x}", d.data),
                    Kind::Wfi => format!("WFI scope={:#x}", d.data),
                    Kind::CeSemA => format!("CE_SET_SEMAPHORE_A value={:#x}", d.data),
                    Kind::CeSemB => format!("CE_SET_SEMAPHORE_B value={:#x}", d.data),
                    Kind::CeSemPayload => format!("CE_SET_SEMAPHORE_PAYLOAD value={:#x}", d.data),
                    Kind::CeLaunch { semaphore_type } => {
                        format!(
                            "CE_LAUNCH_DMA semaphore_type={semaphore_type:#x} raw={:#x}",
                            d.data
                        )
                    }
                    Kind::SemExecute(e) => {
                        let acq = e.op.is_acquire();
                        any_acquire |= acq;
                        let mut t = format!(
                            "SEM_EXECUTE {} raw={:#x} addr={} {}={} wfi={:#x} payload64={:#x} timestamp={:#x} acquire_switch={:#x}",
                            e.op.name(),
                            e.raw,
                            e.addr.map_or("unknown".to_owned(), |a| format!("{a:#x}")),
                            if acq { "awaited" } else { "payload" },
                            e.payload
                                .map_or("unknown".to_owned(), |a| format!("{a:#x}")),
                            u8::from(e.release_wfi),
                            u8::from(e.payload64),
                            u8::from(e.timestamp),
                            u8::from(e.acquire_switch),
                        );
                        if let SemOp::Reserved(v) = e.op {
                            t.push_str(&format!(" op_value={v:#x}"));
                        }
                        if acq {
                            let cur = probes.iter().find(|pr| pr.item == idx).map(|pr| pr.current);
                            match (e.addr, cur) {
                                (None, _) => t.push_str(" current=NOADDR"),
                                (Some(_), None) => t.push_str(" current=UNPROBED"),
                                (Some(_), Some(Current::Unread(why))) => {
                                    t.push_str(&format!(" current=UNREAD({})", clean(why)));
                                }
                                (Some(_), Some(Current::Value(c))) => {
                                    t.push_str(&format!(" current={c:#x}"));
                                    let verdict = e
                                        .payload
                                        .and_then(|a| name_says_pass(e.op, c, a, e.payload64));
                                    t.push_str(match verdict {
                                        Some(true) => " name_says_pass=yes",
                                        Some(false) => " name_says_pass=no",
                                        None => " name_says_pass=n/a",
                                    });
                                }
                            }
                        }
                        t
                    }
                };
                out.push(format!(
                    "{p} M {at} {text}{}",
                    if mark == Some(idx) { " <== PB_GET" } else { "" }
                ));
            }
            if others != 0 {
                out.push(format!(
                    "{p} OTHER methods={others:#x} first=[{}]",
                    first_other.join(",")
                ));
            }
            if any_acquire {
                out.push(format!(
                    "{p} NOTE name_says_pass is inferred from the operation name, never measured; current is read from memory through the relay's placement view, not by the GPU"
                ));
            }
        }
    }
    out.finish(format!("{p} END"))
}

const PREFIX: &str = "kf3: USER_RING_STALL_DIAG";

/// Bounded output: at most [`MAX_LINES`] lines (the last one is always the END line), each at most
/// [`MAX_LINE_CHARS`] printable-ASCII characters.
#[derive(Default)]
struct Out {
    lines: Vec<String>,
    omitted: usize,
}

impl Out {
    fn push(&mut self, line: String) {
        // one slot is kept for the "omitted" note and one for END
        if self.lines.len() + 2 >= MAX_LINES {
            self.omitted += 1;
            return;
        }
        self.lines.push(clip(line));
    }
    fn finish(mut self, end: String) -> Vec<String> {
        if self.omitted != 0 {
            let n = self.omitted;
            self.lines.push(format!("{PREFIX} OMITTED lines={n:#x}"));
        }
        self.lines.push(clip(end));
        self.lines
    }
}

/// Printable ASCII only, one line, at most [`MAX_LINE_CHARS`] characters.
fn clip(line: String) -> String {
    let mut s: String = line
        .chars()
        .map(|c| if (' '..='~').contains(&c) { c } else { '?' })
        .take(MAX_LINE_CHARS)
        .collect();
    if s.chars().count() == MAX_LINE_CHARS {
        s.truncate(MAX_LINE_CHARS - 1);
        s.push('~');
    }
    s
}

/// A reason string made of the characters of a name (the glue passes literals only).
fn clean(why: &str) -> String {
    why.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(32)
        .collect()
}

fn hex_or<T: Into<u64>>(v: Option<T>, none: &str) -> String {
    v.map_or_else(|| none.to_owned(), |v| format!("{:#x}", v.into()))
}

fn wait_text(w: Option<Wait>) -> String {
    match w {
        None => "none".to_owned(),
        Some(Wait::Marker { target }) => format!("Marker(target={target:#x})"),
        Some(Wait::Popped { target }) => format!("Popped(target={target:#x})"),
        Some(Wait::Contended) => "Contended".to_owned(),
    }
}

/// The diagnostic's cumulative counters (VM wide).
#[derive(Debug, Default)]
pub struct StallStats {
    /// Conditions reported.
    pub conditions: AtomicU64,
    /// Acquires listed.
    pub acquires: AtomicU64,
    /// Acquires whose semaphore could not be read.
    pub unread: AtomicU64,
    /// Acquires whose current value passes by the name of the operation.
    pub would_pass: AtomicU64,
    /// Conditions whose segment was not found / not readable.
    pub no_segment: AtomicU64,
    /// Looks skipped because a lock was busy.
    pub skipped_busy: AtomicU64,
}

impl StallStats {
    /// Whether anything was counted.
    #[must_use]
    pub fn any(&self) -> bool {
        self.fields().iter().any(|(_, c)| c.load(Relaxed) != 0)
    }
    fn fields(&self) -> [(&'static str, &AtomicU64); 6] {
        [
            ("conditions", &self.conditions),
            ("acquires", &self.acquires),
            ("unread", &self.unread),
            ("would_pass", &self.would_pass),
            ("no_segment", &self.no_segment),
            ("skipped_busy", &self.skipped_busy),
        ]
    }
    /// One bounded fragment (hex), for the lifecycle counters line.
    #[must_use]
    pub fn summary(&self) -> String {
        if !self.any() {
            return String::new();
        }
        self.fields()
            .iter()
            .map(|(n, c)| format!("diag_user_stall_{n}={:#x}", c.load(Relaxed)))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Per-channel diagnostic state: the detector and the facts the report quotes.
#[derive(Debug, Clone, Copy, Default)]
pub struct ChannelDiag {
    /// The stall detector.
    pub detector: Detector,
    wake: Option<(u64, Peek)>,
    turn_ms: Option<u64>,
    db_count: u64,
    db_ms: Option<u64>,
    db_seen: bool,
}

impl ChannelDiag {
    /// A fresh state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            detector: Detector::new(),
            wake: None,
            turn_ms: None,
            db_count: 0,
            db_ms: None,
            db_seen: false,
        }
    }
    /// A look rang the relay.
    pub fn note_wake(&mut self, now_ms: u64, why: Peek) {
        self.wake = Some((now_ms, why));
    }
    /// The relay had a turn.
    pub fn note_turn(&mut self, now_ms: u64) {
        self.turn_ms = Some(now_ms);
    }
    /// The doorbell count read at `now_ms`.
    pub fn note_doorbells(&mut self, now_ms: u64, count: u64) {
        if !self.db_seen {
            // the first look cannot say when the last doorbell was rung
            self.db_seen = true;
            self.db_count = count;
        } else if count != self.db_count {
            self.db_count = count;
            self.db_ms = Some(now_ms);
        }
    }
    /// `(doorbells, ms since the count last changed)`.
    #[must_use]
    pub fn doorbells(&self, now_ms: u64) -> (u64, Option<u64>) {
        (self.db_count, self.db_ms.map(|t| now_ms.saturating_sub(t)))
    }
    /// `(ms since the last ring, its reason)`.
    #[must_use]
    pub fn last_wake(&self, now_ms: u64) -> Option<(u64, Peek)> {
        self.wake.map(|(t, w)| (now_ms.saturating_sub(t), w))
    }
    /// Ms since the last turn.
    #[must_use]
    pub fn last_turn(&self, now_ms: u64) -> Option<u64> {
        self.turn_ms.map(|t| now_ms.saturating_sub(t))
    }
}

/// The name printed for a look's reason.
#[must_use]
pub fn peek_name(why: Peek) -> &'static str {
    match why {
        Peek::Spin => "Spin",
        Peek::Event => "Event",
        Peek::Backstop => "Backstop",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_abi::submit::{gp_entry, method_header_inc, method_header_non_inc};

    // ---- helpers: the method numbers below are the ogkm headers', NOT kf_abi's constants ----
    // clc56f.h / clc86f.h: SEM_ADDR_LO 0x5c, SEM_ADDR_HI 0x60, SEM_PAYLOAD_LO 0x64,
    // SEM_PAYLOAD_HI 0x68, SEM_EXECUTE 0x6c, NON_STALL_INTERRUPT 0x20, SET_REFERENCE 0x50, WFI 0x78,
    // SEMAPHOREA 0x10, SEMAPHOREB 0x14, SEMAPHOREC 0x18, SEMAPHORED 0x1c.
    // clc6b5.h: SET_SEMAPHORE_A 0x240, _B 0x244, _PAYLOAD 0x248, LAUNCH_DMA 0x300 (SEMAPHORE_TYPE 4:3).
    fn inc(sub: u32, method: u32, data: &[u32]) -> Vec<u32> {
        let mut v = vec![method_header_inc(sub, method, data.len() as u32).unwrap()];
        v.extend_from_slice(data);
        v
    }
    fn non_inc(sub: u32, method: u32, data: &[u32]) -> Vec<u32> {
        let mut v = vec![method_header_non_inc(sub, method, data.len() as u32).unwrap()];
        v.extend_from_slice(data);
        v
    }
    /// `IMMD_DATA_METHOD`: sec_op 4, data in 28:16.
    fn immd(sub: u32, method: u32, data: u32) -> u32 {
        (4 << 29) | (data << 16) | (sub << 13) | (method / 4)
    }
    fn sem(addr: u64, payload: u64, exec: u32) -> Vec<u32> {
        inc(
            0,
            0x5c,
            &[
                addr as u32,
                (addr >> 32) as u32,
                payload as u32,
                (payload >> 32) as u32,
                exec,
            ],
        )
    }
    fn exec_of(it: &Decoded) -> Exec {
        match it.kind {
            Kind::SemExecute(e) => e,
            k => panic!("not a SEM_EXECUTE: {k:?}"),
        }
    }

    // ------------------------------------------------------------------ the switch
    #[test]
    fn the_switch_parses_milliseconds_and_everything_else_is_off_or_loud() {
        let t = [
            (None, Setting::Off),
            (Some(""), Setting::Off),
            (Some("0"), Setting::Off),
            (Some("2000"), Setting::On(2000)),
            (Some(" 2000 "), Setting::On(2000)),
            (Some("1"), Setting::On(1)),
            (Some("99999999999"), Setting::On(MAX_MS)),
            (Some("abc"), Setting::Invalid),
            (Some("-5"), Setting::Invalid),
            (Some("1.5"), Setting::Invalid),
            (Some("0x10"), Setting::Invalid),
            (Some("2000ms"), Setting::Invalid),
        ];
        for (v, want) in t {
            assert_eq!(parse_setting(v), want, "value {v:?}");
        }
        assert_eq!(ENV, "KF3_DIAG_USER_STALL");
    }

    // ------------------------------------------------------------------ the detector
    #[test]
    fn an_empty_private_ring_is_quiet_forever() {
        let mut d = Detector::new();
        for t in [0, 1000, 5000, 1_000_000] {
            assert_eq!(d.observe(t, 7, 7, 512, 100), Verdict::Quiet);
        }
        assert!(!d.fired());
    }

    #[test]
    fn it_fires_once_after_the_threshold_with_the_stalled_time() {
        let mut d = Detector::new();
        assert_eq!(d.observe(1000, 5, 7, 512, 2000), Verdict::Quiet);
        assert_eq!(d.observe(2999, 5, 7, 512, 2000), Verdict::Quiet);
        assert_eq!(
            d.observe(3000, 5, 7, 512, 2000),
            Verdict::Fire { stalled_ms: 2000 }
        );
        assert!(d.fired());
        assert_eq!(
            d.observe(9000, 5, 7, 512, 2000),
            Verdict::Quiet,
            "once per condition"
        );
        assert_eq!(d.observe(90_000, 5, 7, 512, 2000), Verdict::Quiet);
    }

    #[test]
    fn the_condition_is_re_armed_when_get_moves() {
        let mut d = Detector::new();
        assert_eq!(d.observe(0, 5, 7, 512, 100), Verdict::Quiet);
        assert_eq!(
            d.observe(100, 5, 7, 512, 100),
            Verdict::Fire { stalled_ms: 100 }
        );
        // GET moved by one: a new condition whose clock starts now
        assert_eq!(d.observe(150, 6, 7, 512, 100), Verdict::Quiet);
        assert!(!d.fired());
        assert_eq!(d.observe(249, 6, 7, 512, 100), Verdict::Quiet);
        assert_eq!(
            d.observe(250, 6, 7, 512, 100),
            Verdict::Fire { stalled_ms: 100 }
        );
    }

    #[test]
    fn draining_to_empty_resets_the_clock() {
        let mut d = Detector::new();
        assert_eq!(d.observe(0, 5, 7, 512, 100), Verdict::Quiet);
        assert_eq!(d.observe(90, 7, 7, 512, 100), Verdict::Quiet); // drained
        assert_eq!(d.observe(1000, 7, 9, 512, 100), Verdict::Quiet); // new work, new clock
        assert_eq!(d.observe(1099, 7, 9, 512, 100), Verdict::Quiet);
        assert_eq!(
            d.observe(1100, 7, 9, 512, 100),
            Verdict::Fire { stalled_ms: 100 }
        );
    }

    #[test]
    fn put_advancing_while_get_is_stuck_does_not_reset_the_clock() {
        let mut d = Detector::new();
        assert_eq!(d.observe(0, 5, 6, 512, 100), Verdict::Quiet);
        assert_eq!(d.observe(60, 5, 9, 512, 100), Verdict::Quiet);
        assert_eq!(
            d.observe(100, 5, 12, 512, 100),
            Verdict::Fire { stalled_ms: 100 }
        );
    }

    #[test]
    fn the_outstanding_count_wraps_with_the_ring() {
        let mut d = Detector::new();
        // GET 510, PUT 1 on a 512-entry ring: three entries outstanding
        assert_eq!(d.observe(0, 510, 1, 512, 10), Verdict::Quiet);
        assert_eq!(
            d.observe(10, 510, 1, 512, 10),
            Verdict::Fire { stalled_ms: 10 }
        );
        // GET 1, PUT 510 is also outstanding (509 of them), not "empty"
        let mut e = Detector::new();
        assert_eq!(e.observe(0, 1, 510, 512, 10), Verdict::Quiet);
        assert_eq!(
            e.observe(10, 1, 510, 512, 10),
            Verdict::Fire { stalled_ms: 10 }
        );
    }

    #[test]
    fn words_outside_the_ring_and_a_zero_threshold_never_fire() {
        let mut d = Detector::new();
        assert_eq!(d.observe(0, 600, 3, 512, 1), Verdict::Quiet);
        assert_eq!(d.observe(100, 600, 3, 512, 1), Verdict::Quiet);
        assert_eq!(d.observe(0, 3, 600, 512, 1), Verdict::Quiet);
        assert_eq!(d.observe(100, 3, 600, 512, 1), Verdict::Quiet);
        let mut z = Detector::new();
        assert_eq!(z.observe(0, 1, 3, 512, 0), Verdict::Quiet);
        assert_eq!(z.observe(100, 1, 3, 512, 0), Verdict::Quiet);
    }

    #[test]
    fn a_clock_that_runs_backwards_is_quiet_not_a_panic() {
        let mut d = Detector::new();
        assert_eq!(d.observe(1000, 1, 3, 512, 10), Verdict::Quiet);
        assert_eq!(d.observe(5, 1, 3, 512, 10), Verdict::Quiet);
    }

    // ------------------------------------------------------------------ the operation table
    #[test]
    fn sem_execute_operations_follow_the_header_table() {
        // clc56f.h:216-222
        let t = [
            (0, SemOp::Acquire, true, "ACQUIRE"),
            (1, SemOp::Release, false, "RELEASE"),
            (2, SemOp::AcqStrictGeq, true, "ACQ_STRICT_GEQ"),
            (3, SemOp::AcqCircGeq, true, "ACQ_CIRC_GEQ"),
            (4, SemOp::AcqAnd, true, "ACQ_AND"),
            (5, SemOp::AcqNor, true, "ACQ_NOR"),
            (6, SemOp::Reduction, false, "REDUCTION"),
            (7, SemOp::Reserved(7), false, "RESERVED"),
        ];
        for (v, op, acq, name) in t {
            // only bits 2:0 select the operation
            for noise in [0u32, 1 << 12, 1 << 20, 1 << 24, 0xFFFF_FFF8] {
                assert_eq!(SemOp::of_execute(v | noise), op, "raw {:#x}", v | noise);
            }
            assert_eq!(op.is_acquire(), acq, "{name}");
            assert_eq!(op.name(), name);
        }
    }

    #[test]
    fn legacy_semaphored_operations_follow_the_header_table() {
        // clc56f.h:84-91: ACQUIRE 1, RELEASE 2, ACQ_GEQ 4, ACQ_AND 8, REDUCTION 0x10
        assert_eq!(SemOp::of_legacy(1), SemOp::Acquire);
        assert_eq!(SemOp::of_legacy(2), SemOp::Release);
        assert_eq!(SemOp::of_legacy(4), SemOp::AcqGeq);
        assert_eq!(SemOp::of_legacy(8), SemOp::AcqAnd);
        assert_eq!(SemOp::of_legacy(0x10), SemOp::Reduction);
        assert_eq!(SemOp::of_legacy(0), SemOp::Reserved(0));
        assert_eq!(SemOp::of_legacy(3), SemOp::Reserved(3));
        // bits above 4:0 do not change the operation
        assert_eq!(SemOp::of_legacy(0x1000 | 0x0010_0000 | 1), SemOp::Acquire);
        assert!(SemOp::AcqGeq.is_acquire());
        assert!(!SemOp::Reduction.is_acquire());
    }

    // ------------------------------------------------------------------ the decoder
    #[test]
    fn an_acquire_is_decoded_with_its_address_payload_and_flags() {
        let addr = 0x0000_0001_2345_6780u64;
        let words = sem(addr, 0x1122_3344, 0);
        let s = decode(&words, MAX_METHODS);
        assert_eq!(s.methods, 5);
        assert_eq!(s.items.len(), 5);
        let kinds: Vec<_> = s.items.iter().map(|i| i.method).collect();
        assert_eq!(kinds, [0x5c, 0x60, 0x64, 0x68, 0x6c]);
        assert_eq!(s.items[0].kind, Kind::SemAddrLo);
        assert_eq!(s.items[1].kind, Kind::SemAddrHi);
        assert_eq!(s.items[2].kind, Kind::SemPayloadLo);
        assert_eq!(s.items[3].kind, Kind::SemPayloadHi);
        let e = exec_of(&s.items[4]);
        assert_eq!(e.op, SemOp::Acquire);
        assert_eq!(e.addr, Some(addr));
        assert_eq!(e.payload, Some(0x1122_3344));
        assert!(!e.payload64 && !e.release_wfi && !e.timestamp && !e.acquire_switch);
        assert!(!e.legacy);
        assert_eq!(e.raw, 0);
        // word indexes name the datum words (the header is word 0)
        assert_eq!(
            s.items.iter().map(|i| i.word).collect::<Vec<_>>(),
            [1, 2, 3, 4, 5]
        );
        assert!(!s.capped && !s.truncated && s.malformed_at.is_none() && s.ended_at.is_none());
    }

    #[test]
    fn every_operation_value_decodes_through_the_stream() {
        for op in 0..8u32 {
            let s = decode(&sem(0x1000, 5, op), MAX_METHODS);
            let e = exec_of(&s.items[4]);
            assert_eq!(e.op, SemOp::of_execute(op));
        }
    }

    #[test]
    fn the_execute_flag_bits_are_the_header_fields() {
        // RELEASE_WFI 20:20 (EN = 1), PAYLOAD_SIZE 24:24 (64BIT = 1), RELEASE_TIMESTAMP 25:25,
        // ACQUIRE_SWITCH_TSG 12:12, REDUCTION 30:27, REDUCTION_FORMAT 31:31 (UNSIGNED = 1)
        let raw = 1 | (1 << 12) | (1 << 20) | (1 << 24) | (1 << 25) | (5 << 27) | (1 << 31);
        let e = exec_of(&decode(&sem(0x1000, 0x1_0000_0002, raw), 64).items[4]);
        assert_eq!(e.op, SemOp::Release);
        assert!(e.acquire_switch && e.release_wfi && e.payload64 && e.timestamp);
        assert_eq!(e.reduction, 5);
        assert!(e.reduction_unsigned);
        assert_eq!(
            e.payload,
            Some(0x1_0000_0002),
            "64-bit payload joins HI and LO"
        );
        // none of them set
        let e = exec_of(&decode(&sem(0x1000, 2, 1), 64).items[4]);
        assert!(!e.acquire_switch && !e.release_wfi && !e.payload64 && !e.timestamp);
        assert_eq!(e.reduction, 0);
        assert!(!e.reduction_unsigned);
    }

    #[test]
    fn a_32_bit_payload_ignores_the_high_word() {
        let e = exec_of(&decode(&sem(0x1000, 0xDEAD_BEEF_0000_0007, 0), 64).items[4]);
        assert_eq!(e.payload, Some(7));
        assert!(!e.payload64);
    }

    #[test]
    fn the_address_latch_masks_the_fields() {
        // SEM_ADDR_LO_OFFSET is 31:2, SEM_ADDR_HI_OFFSET is 24:0 (clc86f.h:143-145)
        let words = inc(0, 0x5c, &[0x1234_5677, 0xFFFF_FFFF, 9, 0, 0]);
        let e = exec_of(&decode(&words, 64).items[4]);
        assert_eq!(e.addr, Some((0x01FF_FFFFu64 << 32) | 0x1234_5674));
    }

    #[test]
    fn a_half_written_address_is_unknown_not_invented() {
        // LO only, then an execute in its own packet
        let mut w = inc(0, 0x5c, &[0x1000]);
        w.extend(inc(0, 0x6c, &[0]));
        let e = exec_of(&decode(&w, 64).items[1]);
        assert_eq!(e.addr, None);
        assert_eq!(e.payload, None);
        // address written, payload never written: the address is known, the payload is not
        let mut w = inc(0, 0x5c, &[0x1000, 0]);
        w.extend(inc(0, 0x6c, &[0]));
        let e = exec_of(&decode(&w, 64).items[2]);
        assert_eq!(e.addr, Some(0x1000));
        assert_eq!(e.payload, None);
    }

    #[test]
    fn the_latch_survives_to_a_second_execute() {
        let mut w = sem(0x4000, 9, 0);
        w.extend(inc(0, 0x6c, &[1]));
        let s = decode(&w, 64);
        let (a, b) = (exec_of(&s.items[4]), exec_of(&s.items[5]));
        assert_eq!((a.op, b.op), (SemOp::Acquire, SemOp::Release));
        assert_eq!(b.addr, Some(0x4000));
        assert_eq!(b.payload, Some(9));
    }

    #[test]
    fn the_host_methods_are_named() {
        let mut w = inc(0, 0x50, &[0x4b46_6dd5]);
        w.extend(inc(0, 0x20, &[0]));
        w.extend(inc(0, 0x78, &[1]));
        let s = decode(&w, 64);
        assert_eq!(s.items[0].kind, Kind::SetReference);
        assert_eq!(s.items[0].data, 0x4b46_6dd5);
        assert_eq!(s.items[1].kind, Kind::NonStallInterrupt);
        assert_eq!(s.items[2].kind, Kind::Wfi);
        assert_eq!(s.items[2].data, 1);
    }

    #[test]
    fn set_object_binds_the_class_without_the_engine_bits() {
        // NVC56F_SET_OBJECT_NVCLASS is 15:0, ENGINE 20:16 (clc56f.h:69-70)
        let s = decode(&inc(3, 0x0, &[0x001F_C9B5]), 64);
        assert_eq!(s.items[0].kind, Kind::SetObject { class: 0xC9B5 });
        assert_eq!(s.items[0].subchannel, 3);
    }

    #[test]
    fn copy_engine_semaphore_methods_need_a_bound_copy_class() {
        let mut w = inc(4, 0x0, &[0xC6B5]);
        w.extend(inc(4, 0x240, &[0x1]));
        w.extend(inc(4, 0x244, &[0x2000]));
        w.extend(inc(4, 0x248, &[0x77]));
        w.extend(inc(4, 0x300, &[1 << 3])); // SEMAPHORE_TYPE 4:3 = 1
        let s = decode(&w, 64);
        assert_eq!(s.items[1].kind, Kind::CeSemA);
        assert_eq!(s.items[2].kind, Kind::CeSemB);
        assert_eq!(s.items[3].kind, Kind::CeSemPayload);
        assert_eq!(s.items[4].kind, Kind::CeLaunch { semaphore_type: 1 });
        // the same methods on a subchannel bound to a graphics class, or to nothing, are Other
        let mut g = inc(2, 0x0, &[0xC997]);
        g.extend(inc(2, 0x240, &[1]));
        g.extend(inc(5, 0x244, &[1]));
        let s = decode(&g, 64);
        assert_eq!(s.items[1].kind, Kind::Other);
        assert_eq!(s.items[2].kind, Kind::Other);
    }

    #[test]
    fn copy_classes_are_the_xxb5_family() {
        for c in [
            0xA0B5u32, 0xC0B5, 0xC3B5, 0xC5B5, 0xC6B5, 0xC7B5, 0xC8B5, 0xC9B5, 0x85B5,
        ] {
            assert!(is_copy_class(c), "{c:#x}");
        }
        for c in [
            0xC997u32, 0xC9C0, 0xC86F, 0xC56F, 0x0, 0xB5, 0xC6B4, 0x1_C6B5,
        ] {
            assert!(!is_copy_class(c), "{c:#x}");
        }
    }

    #[test]
    fn immediate_non_incrementing_and_one_increment_forms_are_walked() {
        // IMMD_DATA_METHOD: the datum is in the header, no words follow
        let mut w = vec![immd(0, 0x20, 0x123)];
        // NON_INC: all words at one method
        w.extend(non_inc(0, 0x64, &[11, 22]));
        // ONE_INC (sec_op 5): first at the method, the rest at method + 4
        let mut oi = vec![(5u32 << 29) | (3 << 16) | (0x5c / 4)];
        oi.extend([0xA0, 0xB0, 0xC0]);
        w.extend(oi);
        let s = decode(&w, 64);
        let got: Vec<(u32, u32)> = s.items.iter().map(|i| (i.method, i.data)).collect();
        assert_eq!(
            got,
            [
                (0x20, 0x123),
                (0x64, 11),
                (0x64, 22),
                (0x5c, 0xA0),
                (0x60, 0xB0),
                (0x60, 0xC0)
            ]
        );
        assert_eq!(s.items[0].kind, Kind::NonStallInterrupt);
        assert!(!s.truncated && s.malformed_at.is_none());
    }

    #[test]
    fn legacy_formats_are_sized_and_skipped_without_desync() {
        // NVC56F_DMA_NOP is 0: the legacy format with a zero count
        let mut w = vec![0u32, 0u32];
        // GRP0_USE_TERT (sec_op 0), COUNT_OLD 28:18 = 2, ADDRESS_OLD 12:2: two data words follow
        w.push((2 << 18) | (0x40 / 4 * 4));
        w.extend([0xAAAA_AAAA, 0xBBBB_BBBB]);
        w.extend(inc(0, 0x20, &[0]));
        let s = decode(&w, 64);
        assert_eq!(s.items.len(), 1, "{:?}", s);
        assert_eq!(s.items[0].kind, Kind::NonStallInterrupt);
        assert_eq!(s.items[0].word, 6);
    }

    #[test]
    fn a_reserved_header_stops_the_decode_by_index() {
        let mut w = inc(0, 0x20, &[0]);
        w.push(6 << 29); // RESERVED6: no format
        w.extend(inc(0, 0x78, &[0]));
        let s = decode(&w, 64);
        assert_eq!(s.malformed_at, Some(2));
        assert_eq!(s.items.len(), 1);
    }

    #[test]
    fn end_pb_segment_stops_the_decode() {
        let mut w = inc(0, 0x20, &[0]);
        w.push(7 << 29);
        w.extend(inc(0, 0x78, &[0]));
        let s = decode(&w, 64);
        assert_eq!(s.ended_at, Some(2));
        assert_eq!(s.items.len(), 1);
    }

    #[test]
    fn a_packet_cut_short_is_reported_not_padded() {
        let w = vec![method_header_inc(0, 0x5c, 5).unwrap(), 1, 2];
        let s = decode(&w, 64);
        assert!(s.truncated);
        assert_eq!(s.items.len(), 2);
        assert!(!s.capped);
    }

    #[test]
    fn the_method_cap_is_exact() {
        let w64 = inc(0, 0x64, &vec![0; 64]);
        let s = decode(&w64, 64);
        assert_eq!(
            (s.items.len(), s.capped),
            (64, false),
            "exactly 64 methods fit"
        );
        let w65 = inc(0, 0x64, &vec![0; 65]);
        let s = decode(&w65, 64);
        assert_eq!((s.items.len(), s.methods, s.capped), (64, 64, true));
        // across packets too
        let mut many = Vec::new();
        for _ in 0..100 {
            many.extend(inc(0, 0x20, &[0]));
        }
        let s = decode(&many, MAX_METHODS);
        assert_eq!((s.items.len(), s.capped), (MAX_METHODS, true));
        let s = decode(&many, 3);
        assert_eq!((s.items.len(), s.capped), (3, true));
        assert_eq!(decode_methods(&many).len(), MAX_METHODS);
    }

    #[test]
    fn legacy_semaphore_methods_decode_onto_the_same_shape() {
        // SEMAPHOREA (0x10) OFFSET_UPPER 7:0, B (0x14) OFFSET_LOWER 31:2, C (0x18) PAYLOAD, D (0x1c)
        let d = 1 | (1 << 24); // ACQUIRE, SIZE_4BYTE
        let w = inc(0, 0x10, &[0x3, 0x1000_0008, 0x55, d]);
        let s = decode(&w, 64);
        assert_eq!(s.items[0].kind, Kind::SemAddrHi);
        assert_eq!(s.items[1].kind, Kind::SemAddrLo);
        assert_eq!(s.items[2].kind, Kind::SemPayloadLo);
        let e = exec_of(&s.items[3]);
        assert!(e.legacy);
        assert_eq!(e.op, SemOp::Acquire);
        assert_eq!(e.addr, Some((3u64 << 32) | 0x1000_0008));
        assert_eq!(e.payload, Some(0x55));
        assert!(!e.payload64);
        // RELEASE_WFI 20:20: EN = 0 in the legacy header, so bit clear means WFI
        let w = inc(0, 0x1c, &[2]);
        assert!(exec_of(&decode(&w, 64).items[0]).release_wfi);
        let w = inc(0, 0x1c, &[2 | (1 << 20)]);
        assert!(!exec_of(&decode(&w, 64).items[0]).release_wfi);
    }

    #[test]
    fn hostile_streams_never_panic_and_stay_inside_their_bounds() {
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for round in 0..4000 {
            let n = (next() % 700) as usize;
            let words: Vec<u32> = (0..n)
                .map(|_| {
                    let r = next();
                    // bias towards plausible headers so packets actually open
                    if r & 3 == 0 {
                        (r >> 8) as u32
                    } else {
                        method_header_inc(
                            ((r >> 3) % 8) as u32,
                            ((r >> 8) % 0x300 * 4) as u32 & !3,
                            ((r >> 30) % 70 + 1) as u32,
                        )
                        .unwrap_or(0x2000_0000)
                    }
                })
                .collect();
            let cap = (next() % 70) as usize;
            let s = decode(&words, cap);
            assert!(s.items.len() <= cap, "round {round}");
            assert_eq!(s.methods, s.items.len());
            assert!(s.items.iter().all(|i| i.word < words.len().max(1)));
            assert!(
                s.items.windows(2).all(|p| p[0].word < p[1].word),
                "word order, round {round}"
            );
            let _ = probe_plan(&s);
        }
    }

    // ------------------------------------------------------------------ what the name says
    #[test]
    fn the_name_says_pass_table() {
        use SemOp::*;
        let t: [(SemOp, u64, u64, bool, Option<bool>); 18] = [
            (Acquire, 5, 5, false, Some(true)),
            (Acquire, 4, 5, false, Some(false)),
            (Acquire, 6, 5, false, Some(false)),
            (AcqStrictGeq, 5, 5, false, Some(true)),
            (AcqStrictGeq, 6, 5, false, Some(true)),
            (AcqStrictGeq, 4, 5, false, Some(false)),
            (AcqGeq, 4, 5, false, Some(false)),
            (AcqCircGeq, 1, 0xFFFF_FFFF, false, Some(true)),
            (AcqCircGeq, 0xFFFF_FFFE, 1, false, Some(false)),
            (AcqCircGeq, 1, 0xFFFF_FFFF_FFFF_FFFF, true, Some(true)),
            (AcqCircGeq, 0x8000_0000_0000_0000, 0, true, Some(false)),
            (AcqAnd, 0b0110, 0b0100, false, Some(true)),
            (AcqAnd, 0b0110, 0b1001, false, Some(false)),
            (AcqNor, 0, 0, false, None),
            (Release, 1, 1, false, None),
            (Reduction, 1, 1, false, None),
            (Reserved(7), 1, 1, false, None),
            // a 32-bit acquire compares 32 bits: the high halves are not part of the value
            (Acquire, 0xFFFF_FFFF_0000_0005, 5, false, Some(true)),
        ];
        for (op, cur, aw, p64, want) in t {
            assert_eq!(
                name_says_pass(op, cur, aw, p64),
                want,
                "{op:?} cur={cur:#x} awaited={aw:#x} p64={p64}"
            );
        }
    }

    // ------------------------------------------------------------------ the probe plan
    #[test]
    fn the_probe_plan_lists_acquires_with_a_known_address_only() {
        let mut w = sem(0x1000, 5, 0); // acquire
        w.extend(sem(0x2000, 6, 1)); // release: not probed
        w.extend(sem(0x3000, 0x1_0000_0007, 3 | (1 << 24))); // 64-bit circ acquire
        w.extend(inc(0, 0x6c, &[2])); // strict geq on the latched 0x3000 (64-bit flag not set here)
        let s = decode(&w, MAX_METHODS);
        let plan = probe_plan(&s);
        assert_eq!(
            plan,
            [
                ProbeReq {
                    item: 4,
                    addr: 0x1000,
                    bytes: 4
                },
                ProbeReq {
                    item: 14,
                    addr: 0x3000,
                    bytes: 8
                },
                ProbeReq {
                    item: 15,
                    addr: 0x3000,
                    bytes: 4
                },
            ]
        );
        // an execute whose address was never written is not planned
        let s = decode(&inc(0, 0x6c, &[0]), 64);
        assert!(probe_plan(&s).is_empty());
    }

    #[test]
    fn the_probe_plan_is_bounded() {
        let mut w = Vec::new();
        for i in 0..12u64 {
            w.extend(sem(0x1000 + i * 4, 1, 0));
        }
        let s = decode(&w, 64);
        assert_eq!(s.items.len(), 60);
        let plan = probe_plan(&s);
        assert_eq!(plan.len(), MAX_PROBES);
        assert_eq!(plan[0].addr, 0x1000);
        assert_eq!(
            plan[MAX_PROBES - 1].addr,
            0x1000 + (MAX_PROBES as u64 - 1) * 4
        );
    }

    // ------------------------------------------------------------------ which private entry
    #[test]
    fn the_ring_window_covers_three_behind_get_up_to_put_and_wraps() {
        assert_eq!(ring_window(100, 102, 512), (97, 5));
        assert_eq!(ring_window(100, 100, 512), (97, 3));
        assert_eq!(ring_window(100, 150, 512), (97, MAX_RING_ROWS as u32));
        assert_eq!(ring_window(1, 3, 512), (510, 5));
        assert_eq!(ring_window(0, 0, 512), (509, 3));
        assert_eq!(ring_window(510, 2, 512), (507, 7));
        // a ring smaller than the window lists each slot once
        let (_, c) = ring_window(2, 3, 4);
        assert!(c <= 4);
    }

    fn seg_entry(va: u64, len: u64) -> u64 {
        gp_entry(va, len).unwrap()
    }

    #[test]
    fn the_segment_is_the_row_that_holds_pb_get_nearest_behind_get_first() {
        let rows = [
            (97u32, seg_entry(0x1000, 0x100)),
            (98, seg_entry(0x1000, 0x400)), // overlaps 97's range
            (99, seg_entry(0x9000, 0x40)),
            (100, seg_entry(0x5000, 0x20)),
            (101, seg_entry(0x6000, 0x20)),
        ];
        // GET is 100: the PBDMA is inside slot 99 (GET - 1) when PB_GET lies there
        assert_eq!(pick_segment(&rows, 100, 512, 0x9010), Some(99));
        // both 97 and 98 hold 0x1050; the nearer behind GET wins
        assert_eq!(pick_segment(&rows, 100, 512, 0x1050), Some(98));
        // a row at or ahead of GET (fetched, not yet counted) is a candidate too
        assert_eq!(pick_segment(&rows, 100, 512, 0x5010), Some(100));
        // the end of a segment is exclusive
        assert_eq!(pick_segment(&rows, 100, 512, 0x9040), None);
        assert_eq!(pick_segment(&rows, 100, 512, 0x9000), Some(99));
        assert_eq!(pick_segment(&rows, 100, 512, 0xdead_0000), None);
    }

    #[test]
    fn control_entries_never_hold_pb_get() {
        // LENGTH == 0: a control entry (NOP opcode)
        let rows = [(9u32, 0u64), (10, seg_entry(0x2000, 0x40))];
        assert_eq!(pick_segment(&rows, 11, 512, 0), None);
        assert_eq!(pick_segment(&rows, 11, 512, 0x2000), Some(10));
        assert_eq!(pick_segment(&[], 11, 512, 0x2000), None);
    }

    #[test]
    fn the_segment_lookup_wraps_around_the_ring() {
        let rows = [
            (511u32, seg_entry(0x7000, 0x40)),
            (0, seg_entry(0x8000, 0x40)),
        ];
        // GET 0: slot 511 is GET - 1
        assert_eq!(pick_segment(&rows, 0, 512, 0x7020), Some(511));
    }

    // ------------------------------------------------------------------ rendering
    fn incident_report() -> Report {
        // the 2026-10-20 incident's shape: a 500 B guest pushbuffer at 0x304720000, PB_GET at +0x40,
        // two entries outstanding, REFERENCE one marker behind
        // eight NOP words, SET_OBJECT, the acquire, SET_REFERENCE, NON_STALL_INTERRUPT: PB_GET (+0x40
        // = word 16) lies after the acquire, which the PBDMA is blocked in
        let mut pb = vec![0u32; 8];
        pb.extend(inc(4, 0x0, &[0xC6B5]));
        pb.extend(sem(0x0000_0001_2036_1000, 0x2a, 3 | (1 << 12)));
        pb.extend(inc(0, 0x50, &[0x4b46_6dd5]));
        pb.extend(inc(0, 0x20, &[0]));
        let stream = decode(&pb, MAX_METHODS);
        let acq = stream
            .items
            .iter()
            .position(|i| matches!(i.kind, Kind::SemExecute(_)))
            .unwrap();
        Report {
            key: (0xc1d0_0001, 0x5c00_0010),
            host: 0x1c0,
            stalled_ms: 0x7d3,
            threshold_ms: 0x7d0,
            guest_put: Some(0x1ab),
            guest_get: Some(0x1a7),
            private: Private {
                gp_get: 0x1a8,
                gp_put: 0x1aa,
                reference: 0x4b46_6dd4,
                pb_get: 0x3_0472_0040,
                pb_put: 0x1_2036_1d40,
                top_level_get: 0x3_0472_0000,
            },
            wait: Some(Wait::Marker { target: 0x1234 }),
            doorbells: 0x77,
            doorbell_ago_ms: Some(0x1f40),
            wake: Some((0x30, Peek::Backstop)),
            turn_ago_ms: Some(0x32),
            issued: 0x1aa,
            fetched: 0x1a8,
            marks_issued: 0x20,
            marks_seen: 0x1f,
            cursor: 0x1ab,
            turns: 0x400,
            entries: (0, 0x123, 0),
            rows: vec![
                Row {
                    slot: 0x1a7,
                    entry: gp_entry(0x3_0472_0000, 500).unwrap(),
                },
                Row {
                    slot: 0x1a8,
                    entry: gp_entry(0x1_2036_1d18, 40).unwrap(),
                },
                Row {
                    slot: 0x1a9,
                    entry: gp_entry(0x3_ffff_0000, 8).unwrap(),
                },
            ],
            segment: Segment::Decoded {
                slot: 0x1a7,
                va: 0x3_0472_0000,
                len_bytes: 500,
                read_bytes: 500,
                stream,
                probes: vec![Probe {
                    item: acq,
                    current: Current::Value(0x29),
                }],
            },
            totals: "stall_conditions=0x1".into(),
        }
    }

    fn all(lines: &[String]) -> String {
        lines.join("\n")
    }

    #[test]
    fn the_incident_block_names_every_fact_the_owner_asked_for() {
        let text = all(&render(&incident_report()));
        for needle in [
            "0xc1d00001:0x5c000010",
            "host=0x1c0",
            "stalled_ms=0x7d3",
            "guest_put=0x1ab",
            "guest_get=0x1a7",
            "gp_get=0x1a8",
            "gp_put=0x1aa",
            "reference=0x4b466dd4",
            "pb_get=0x304720040",
            "top_level_get=0x304720000",
            "wait=Marker(target=0x1234)",
            "last_wake=Backstop",
            "doorbells=0x77",
            "issued=0x1aa",
            "fetched=0x1a8",
            "ACQ_CIRC_GEQ",
            "addr=0x120361000",
            "awaited=0x2a",
            "current=0x29",
            "SET_REFERENCE",
            "NON_STALL_INTERRUPT",
            "SEM_ADDR_LO",
            "SEM_ADDR_HI",
            "SEM_PAYLOAD_LO",
            "SEM_PAYLOAD_HI",
            "SET_OBJECT",
            "class=0xc6b5",
            "<== PB_GET",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
        }
    }

    #[test]
    fn the_pb_get_marker_sits_on_the_first_method_at_or_after_pb_get() {
        // PB_GET = segment + 0x40 -> word 16; the marker is on the first item whose word >= 16
        let mut r = incident_report();
        let Segment::Decoded { stream, .. } = &r.segment else {
            panic!()
        };
        let first_at_or_after = stream.items.iter().find(|i| i.word >= 16).unwrap().word;
        let lines = render(&r);
        let marked: Vec<_> = lines.iter().filter(|l| l.contains("<== PB_GET")).collect();
        assert_eq!(marked.len(), 1, "{marked:?}");
        assert!(
            marked[0].contains(&format!("+{:#x}", first_at_or_after * 4)),
            "{}",
            marked[0]
        );
        // PB_GET outside the segment: no marker, and the segment header says so
        r.private.pb_get = 0x9_9999_0000;
        let text = all(&render(&r));
        assert!(!text.contains("<== PB_GET"));
    }

    #[test]
    fn an_unreadable_semaphore_says_so_and_a_vidmem_one_says_vidmem() {
        let mut r = incident_report();
        let Segment::Decoded { probes, .. } = &mut r.segment else {
            panic!()
        };
        probes[0].current = Current::Unread("vidmem");
        let text = all(&render(&r));
        assert!(text.contains("current=UNREAD(vidmem)"), "{text}");
        let Segment::Decoded { probes, .. } = &mut r.segment else {
            panic!()
        };
        probes.clear();
        let text = all(&render(&r));
        assert!(text.contains("current=UNPROBED"), "{text}");
    }

    #[test]
    fn name_says_pass_is_printed_for_a_read_acquire_and_labelled_as_inferred() {
        let text = all(&render(&incident_report()));
        // current 0x29 < awaited 0x2a on a circular GEQ: does not pass
        assert!(text.contains("name_says_pass=no"), "{text}");
        assert!(text.contains("inferred"), "{text}");
    }

    #[test]
    fn a_missing_or_unreadable_segment_is_a_line_not_a_gap() {
        let mut r = incident_report();
        r.segment = Segment::NotFound;
        let text = all(&render(&r));
        assert!(text.contains("SEGMENT none"), "{text}");
        r.segment = Segment::Unread {
            slot: 0x1a7,
            va: 0x3_0472_0000,
            len_bytes: 500,
            why: "Missing",
        };
        let text = all(&render(&r));
        assert!(text.contains("UNREAD(Missing)"), "{text}");
        assert!(text.contains("slot=0x1a7"), "{text}");
    }

    #[test]
    fn rows_are_decoded_and_the_get_row_and_the_holding_row_are_marked() {
        let text = all(&render(&incident_report()));
        assert!(text.contains("va=0x304720000"), "{text}");
        assert!(text.contains("len=0x1f4"), "{text}");
        assert!(text.contains("va=0x120361d18"), "{text}");
        assert!(text.contains("<== GP_GET"), "{text}");
        assert!(text.contains("<== SEGMENT_ROW"), "{text}");
    }

    #[test]
    fn every_number_in_the_block_is_hex() {
        for r in [incident_report()] {
            for line in render(&r) {
                for tok in line.split_whitespace() {
                    let Some((_, v)) = tok.split_once('=') else {
                        continue;
                    };
                    let v = v.trim_matches(|c: char| "()[]{},".contains(c));
                    if v.starts_with(|c: char| c.is_ascii_digit()) {
                        assert!(
                            v.starts_with("0x") && v[2..].chars().all(|c| c.is_ascii_hexdigit()),
                            "decimal-looking value {v:?} in {line:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_worst_case_block_is_bounded_in_lines_and_characters() {
        let mut r = incident_report();
        // 64 interesting methods: 12 full semaphore ops, then padding with NSI
        let mut pb = Vec::new();
        for i in 0..20u64 {
            pb.extend(sem(0x1_0000_0000 + i * 4, i, (i % 8) as u32));
        }
        let stream = decode(&pb, MAX_METHODS);
        assert_eq!(stream.items.len(), MAX_METHODS);
        r.segment = Segment::Decoded {
            slot: 1,
            va: 0x1_0000_0000,
            len_bytes: 4096,
            read_bytes: MAX_PB_BYTES,
            probes: probe_plan(&stream)
                .iter()
                .map(|p| Probe {
                    item: p.item,
                    current: Current::Value(u64::MAX),
                })
                .collect(),
            stream,
        };
        r.rows = (0..64)
            .map(|s| Row {
                slot: s,
                entry: u64::MAX,
            })
            .collect();
        r.totals = "x=0xffffffffffffffff ".repeat(200);
        let lines = render(&r);
        assert!(lines.len() <= MAX_LINES, "{} lines", lines.len());
        for l in &lines {
            assert!(
                l.chars().count() <= MAX_LINE_CHARS,
                "{} chars: {l}",
                l.chars().count()
            );
            assert!(!l.contains('\n'));
        }
    }

    #[test]
    fn rendering_prints_only_printable_ascii() {
        let text = all(&render(&incident_report()));
        assert!(
            text.chars().all(|c| c == '\n' || (' '..='~').contains(&c)),
            "{text}"
        );
    }

    #[test]
    fn a_report_without_any_facts_still_renders_its_frame() {
        let mut r = incident_report();
        r.guest_put = None;
        r.guest_get = None;
        r.wait = None;
        r.doorbell_ago_ms = None;
        r.wake = None;
        r.turn_ago_ms = None;
        r.rows.clear();
        r.segment = Segment::NotFound;
        let text = all(&render(&r));
        for needle in [
            "guest_put=UNREAD",
            "wait=none",
            "last_wake=none",
            "last_doorbell_ago_ms=unknown",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
        }
    }

    // ------------------------------------------------------------------ the per-channel facts
    #[test]
    fn the_channel_remembers_its_last_wake_and_turn() {
        let mut d = ChannelDiag::new();
        assert_eq!(d.last_wake(10), None);
        assert_eq!(d.last_turn(10), None);
        d.note_wake(100, Peek::Spin);
        d.note_wake(250, Peek::Event);
        d.note_turn(260);
        assert_eq!(d.last_wake(300), Some((50, Peek::Event)));
        assert_eq!(d.last_turn(300), Some(40));
        // a clock that ran backwards reads as zero, not as a huge number
        assert_eq!(d.last_wake(10), Some((0, Peek::Event)));
    }

    #[test]
    fn the_doorbell_time_is_when_the_count_was_seen_to_change() {
        let mut d = ChannelDiag::new();
        assert_eq!(d.doorbells(5), (0, None));
        d.note_doorbells(100, 7); // first look: the time of the ring is unknown
        assert_eq!(d.doorbells(150), (7, None));
        d.note_doorbells(200, 7);
        assert_eq!(d.doorbells(250), (7, None));
        d.note_doorbells(300, 9); // changed between 200 and 300
        assert_eq!(d.doorbells(350), (9, Some(50)));
        d.note_doorbells(400, 9);
        assert_eq!(d.doorbells(450), (9, Some(150)));
    }

    // ------------------------------------------------------------------ counters
    #[test]
    fn the_counters_are_silent_until_something_is_counted_and_then_hex() {
        use std::sync::atomic::Ordering::Relaxed;
        let s = StallStats::default();
        assert!(!s.any());
        assert_eq!(s.summary(), "");
        s.conditions.fetch_add(0x11, Relaxed);
        s.acquires.fetch_add(3, Relaxed);
        s.unread.fetch_add(1, Relaxed);
        s.would_pass.fetch_add(2, Relaxed);
        s.no_segment.fetch_add(4, Relaxed);
        s.skipped_busy.fetch_add(5, Relaxed);
        assert!(s.any());
        let t = s.summary();
        for needle in [
            "diag_user_stall_conditions=0x11",
            "diag_user_stall_acquires=0x3",
            "diag_user_stall_unread=0x1",
            "diag_user_stall_would_pass=0x2",
            "diag_user_stall_no_segment=0x4",
            "diag_user_stall_skipped_busy=0x5",
        ] {
            assert!(t.contains(needle), "missing {needle:?} in {t:?}");
        }
        assert!(t.len() < 300);
    }

    // ------------------------------------------------------------------ second killers
    #[test]
    fn a_setting_is_never_on_at_zero_and_never_above_the_clamp() {
        for v in ["0", "00", "000", " 0 "] {
            assert_eq!(parse_setting(Some(v)), Setting::Off, "{v:?}");
        }
        for v in [
            "1",
            "50",
            "2000",
            "3600000",
            "3600001",
            "18446744073709551615",
            "184467440737095516160",
        ] {
            match parse_setting(Some(v)) {
                Setting::On(ms) => assert!((1..=MAX_MS).contains(&ms), "{v}: {ms}"),
                other => panic!("{v:?} -> {other:?}"),
            }
        }
        assert_eq!(parse_setting(Some("3600000")), Setting::On(MAX_MS));
        assert_eq!(parse_setting(Some("3600001")), Setting::On(MAX_MS));
    }

    #[test]
    fn the_detector_fires_at_every_ring_position_not_only_the_small_ones() {
        for (get, put) in [(0u32, 1u32), (255, 256), (511, 0), (300, 100), (1, 0)] {
            let mut d = Detector::new();
            assert_eq!(
                d.observe(10, get, put, 512, 5),
                Verdict::Quiet,
                "{get}->{put}"
            );
            assert_eq!(
                d.observe(15, get, put, 512, 5),
                Verdict::Fire { stalled_ms: 5 },
                "{get}->{put}"
            );
        }
        // a smaller ring: 8 entries, GET 7 PUT 0 is one outstanding entry
        let mut d = Detector::new();
        assert_eq!(d.observe(0, 7, 0, 8, 1), Verdict::Quiet);
        assert_eq!(d.observe(1, 7, 0, 8, 1), Verdict::Fire { stalled_ms: 1 });
        // an entry count that is not a power of two is not a ring
        let mut d = Detector::new();
        assert_eq!(d.observe(0, 1, 2, 500, 1), Verdict::Quiet);
        assert_eq!(d.observe(9, 1, 2, 500, 1), Verdict::Quiet);
    }

    #[test]
    fn a_class_numbered_b5_is_not_a_copy_engine() {
        let mut w = inc(4, 0x0, &[0xB5]);
        w.extend(inc(4, 0x240, &[1]));
        w.extend(inc(4, 0x300, &[8]));
        let s = decode(&w, 64);
        assert_eq!(s.items[0].kind, Kind::SetObject { class: 0xB5 });
        assert_eq!(s.items[1].kind, Kind::Other);
        assert_eq!(s.items[2].kind, Kind::Other);
    }

    #[test]
    fn copy_engine_methods_follow_the_subchannel_binding_not_the_stream_order() {
        // sub 4 is a copy engine; sub 1 is rebound from copy to graphics: only sub 4 stays decoded
        let mut w = inc(4, 0x0, &[0xC6B5]);
        w.extend(inc(1, 0x0, &[0xC7B5]));
        w.extend(inc(1, 0x0, &[0xC997]));
        w.extend(inc(4, 0x248, &[5]));
        w.extend(inc(1, 0x248, &[6]));
        let s = decode(&w, 64);
        assert_eq!(s.items[3].kind, Kind::CeSemPayload);
        assert_eq!(s.items[4].kind, Kind::Other);
        // the launch word's SEMAPHORE_TYPE is bits 4:3 only
        let s = decode(
            &[w[..2].to_vec(), inc(4, 0x300, &[0xFFFF_FFE7])].concat(),
            64,
        );
        assert_eq!(s.items[1].kind, Kind::CeLaunch { semaphore_type: 0 });
        let s = decode(&[w[..2].to_vec(), inc(4, 0x300, &[0x18])].concat(), 64);
        assert_eq!(s.items[1].kind, Kind::CeLaunch { semaphore_type: 3 });
    }

    #[test]
    fn the_segment_end_is_exclusive_even_when_a_neighbour_row_would_take_it() {
        let rows = [
            (98u32, seg_entry(0x1000, 0x40)),
            (99, seg_entry(0x5000, 0x40)),
        ];
        assert_eq!(pick_segment(&rows, 100, 512, 0x1040), None);
        assert_eq!(pick_segment(&rows, 100, 512, 0x103c), Some(98));
        assert_eq!(pick_segment(&rows, 100, 512, 0x0fff), None);
    }

    #[test]
    fn a_row_behind_get_wins_over_a_row_at_or_ahead_of_it() {
        let rows = [
            (99u32, seg_entry(0x1000, 0x100)),
            (100, seg_entry(0x1000, 0x100)),
            (101, seg_entry(0x1000, 0x100)),
        ];
        assert_eq!(pick_segment(&rows, 100, 512, 0x1010), Some(99));
        let rows = [
            (100u32, seg_entry(0x1000, 0x100)),
            (101, seg_entry(0x1000, 0x100)),
        ];
        assert_eq!(pick_segment(&rows, 100, 512, 0x1010), Some(100));
        // three behind is the farthest looked at
        let rows = [
            (97u32, seg_entry(0x1000, 0x100)),
            (96, seg_entry(0x2000, 0x100)),
        ];
        assert_eq!(pick_segment(&rows, 100, 512, 0x1010), Some(97));
        assert_eq!(pick_segment(&rows, 100, 512, 0x2010), None);
    }

    #[test]
    fn the_ring_window_always_lists_the_entry_before_get_and_never_exceeds_its_bounds() {
        for entries in [4u32, 8, 64, 512] {
            for get in 0..entries {
                for put in [0, 1, get, get.wrapping_add(1) % entries, entries - 1] {
                    let (start, count) = ring_window(get, put, entries);
                    assert!(
                        start < entries
                            && count <= entries
                            && count as usize <= MAX_RING_ROWS
                            && count >= 3.min(entries)
                    );
                    let slots: Vec<u32> = (0..count).map(|i| (start + i) & (entries - 1)).collect();
                    assert!(
                        slots.contains(&(get.wrapping_sub(1) & (entries - 1))),
                        "{entries} {get} {put}"
                    );
                    if put != get && count >= 4 {
                        assert!(slots.contains(&get), "{entries} {get} {put}");
                    }
                    let mut u = slots.clone();
                    u.sort_unstable();
                    u.dedup();
                    assert_eq!(u.len(), slots.len(), "each slot once");
                }
            }
        }
        assert_eq!(ring_window(1, 2, 500), (0, 0), "not a ring");
    }

    #[test]
    fn name_says_pass_is_rendered_for_each_acquire_flavour() {
        fn line_for(op: u32, cur: u64, awaited: u64) -> String {
            let mut r = incident_report();
            let stream = decode(&sem(0x1000, awaited, op), MAX_METHODS);
            r.private.pb_get = 0;
            r.segment = Segment::Decoded {
                slot: 0x1a7,
                va: 0x1000,
                len_bytes: 64,
                read_bytes: 64,
                probes: vec![Probe {
                    item: 4,
                    current: Current::Value(cur),
                }],
                stream,
            };
            all(&render(&r))
        }
        assert!(line_for(0, 7, 7).contains("current=0x7 name_says_pass=yes"));
        assert!(line_for(0, 8, 7).contains("current=0x8 name_says_pass=no"));
        assert!(line_for(2, 8, 7).contains("name_says_pass=yes"));
        assert!(line_for(2, 6, 7).contains("name_says_pass=no"));
        assert!(line_for(3, 0xffff_ffff, 1).contains("name_says_pass=no"));
        assert!(line_for(3, 1, 0xffff_ffff).contains("name_says_pass=yes"));
        assert!(line_for(4, 6, 4).contains("name_says_pass=yes"));
        assert!(line_for(5, 6, 4).contains("name_says_pass=n/a"));
        // a RELEASE is not an acquire: no current, no verdict, a payload not an awaited value
        let text = line_for(1, 6, 4);
        assert!(
            !text.contains("name_says_pass")
                && !text.contains("current=")
                && text.contains("payload=0x4")
        );
    }

    #[test]
    fn legacy_release_size_and_sub_device_masks_are_decoded_exactly() {
        // legacy SEMAPHORED RELEASE_SIZE (24:24): 16BYTE = 0 is a timestamped release, 4BYTE = 1 is not
        assert!(exec_of(&decode(&inc(0, 0x1c, &[2]), 64).items[0]).timestamp);
        assert!(!exec_of(&decode(&inc(0, 0x1c, &[2 | (1 << 24)]), 64).items[0]).timestamp);
        // the new family: RELEASE_TIMESTAMP (25:25) EN = 1
        assert!(!exec_of(&decode(&inc(0, 0x6c, &[1]), 64).items[0]).timestamp);
        assert!(exec_of(&decode(&inc(0, 0x6c, &[1 | (1 << 25)]), 64).items[0]).timestamp);
        // a sub-device-mask header (sec_op 0, TERT_OP != 0) is header-only
        let mut w = vec![1u32 << 16];
        w.extend(inc(0, 0x20, &[0]));
        let s = decode(&w, 64);
        assert_eq!(s.items.len(), 1);
        assert_eq!(s.items[0].kind, Kind::NonStallInterrupt);
        assert_eq!(s.items[0].word, 2);
    }

    #[test]
    fn a_legacy_packet_longer_than_the_words_left_is_truncated() {
        let w = vec![(5u32 << 18) | 0x40, 1, 2];
        let s = decode(&w, 64);
        assert!(s.truncated);
        assert!(s.items.is_empty());
    }

    #[test]
    fn the_output_buffer_is_bounded_and_always_ends_with_the_end_line() {
        let mut o = Out::default();
        for i in 0..500 {
            o.push(format!("line {i}"));
        }
        let lines = o.finish("END".to_owned());
        assert_eq!(lines.len(), MAX_LINES);
        assert_eq!(lines.last().unwrap(), "END");
        assert!(
            lines[MAX_LINES - 2].contains("OMITTED lines=0x"),
            "{:?}",
            lines[MAX_LINES - 2]
        );
        assert_eq!(lines[0], "line 0");
        // exactly as many lines as fit: nothing is omitted
        let mut o = Out::default();
        for i in 0..MAX_LINES - 2 {
            o.push(format!("line {i}"));
        }
        let lines = o.finish("END".to_owned());
        assert_eq!(lines.len(), MAX_LINES - 1);
        assert!(!lines.iter().any(|l| l.contains("OMITTED")));
    }

    #[test]
    fn one_line_is_clipped_to_the_character_cap_and_to_printable_ascii() {
        let long = "x".repeat(MAX_LINE_CHARS * 3);
        let c = clip(long);
        assert_eq!(c.chars().count(), MAX_LINE_CHARS);
        assert!(c.ends_with('~'));
        assert_eq!(clip("a\nb\u{1b}[0m\u{e9}".to_owned()), "a?b?[0m?");
        let exact = "y".repeat(MAX_LINE_CHARS - 1);
        assert_eq!(clip(exact.clone()), exact);
        // hostile text in the report's free-form fields is neutralised
        let mut r = incident_report();
        r.totals = "t=0x1\nINJECTED\u{7}".to_owned();
        let text = all(&render(&r));
        assert!(!text.contains("\nINJECTED") && text.contains("t=0x1?INJECTED?"));
        r.segment = Segment::Unread {
            slot: 1,
            va: 0,
            len_bytes: 4,
            why: "a b\nc",
        };
        assert!(all(&render(&r)).contains("UNREAD(abc)"));
    }

    #[test]
    fn a_doorbell_count_that_goes_down_is_a_change_too() {
        let mut d = ChannelDiag::new();
        d.note_doorbells(10, 50);
        d.note_doorbells(20, 3); // the token was reused
        assert_eq!(d.doorbells(30), (3, Some(10)));
    }

    #[test]
    fn each_counter_alone_makes_the_summary_non_empty() {
        use std::sync::atomic::Ordering::Relaxed;
        let each: [fn(&StallStats) -> &AtomicU64; 6] = [
            |s| &s.conditions,
            |s| &s.acquires,
            |s| &s.unread,
            |s| &s.would_pass,
            |s| &s.no_segment,
            |s| &s.skipped_busy,
        ];
        for f in each {
            let s = StallStats::default();
            assert!(!s.any());
            f(&s).fetch_add(1, Relaxed);
            assert!(s.any());
            assert!(s.summary().contains("=0x1"));
            assert_eq!(
                s.summary().matches("=0x").count(),
                6,
                "every field is printed"
            );
        }
    }

    #[test]
    fn peek_reasons_have_stable_names() {
        assert_eq!(peek_name(Peek::Spin), "Spin");
        assert_eq!(peek_name(Peek::Event), "Event");
        assert_eq!(peek_name(Peek::Backstop), "Backstop");
    }
}
