//! Relay observability lines of the Windows USER-ring relay (`docs/design/V3_RELAY_LIFECYCLE_LOG.md`).
//!
//! Pure formatting and bookkeeping, no I/O: the pump prints what these functions return, so the format and the
//! once-only / bounded rules are testable GPU-free. Every line is emitted at most once per relay event (bind,
//! retire, exit), never per entry and never per guest-controlled count.

use std::sync::atomic::{AtomicU32, Ordering};

use kf_chip::classes::Kind;

use crate::user_lifecycle::{Bound, Lifecycle};

/// Sentinel of an unknown process id / engine in the atomics (a real value is a `u32`).
const NONE: u64 = u64::MAX;

/// What a relay is, for the log lines: the declared process id and engine of its channel (both already in
/// `CHAN-FACTS`; nothing is captured anew) and the set of engine-object kinds allocated on it. Shared between the
/// channel's twin record (which notes a kind when an engine object is kept: wait-free) and the pump (which reads it
/// when it prints).
#[derive(Debug)]
pub struct RelayIdent {
    pid: u64,
    engine: u64,
    /// One bit per [`Kind`] (its position in [`Kind::ALL`]).
    kinds: AtomicU32,
}

/// Which final line: the relay retired, or it was still alive at device stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum At {
    /// The relay was released (channel free).
    Retire,
    /// The device stopped while the relay was alive.
    Exit,
}

impl At {
    /// The `at=` value.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            At::Retire => "retire",
            At::Exit => "exit",
        }
    }
}

fn kind_bit(kind: Kind) -> u32 {
    let at = Kind::ALL.iter().position(|k| *k == kind).unwrap_or(0);
    u32::try_from(at)
        .ok()
        .and_then(|n| 1u32.checked_shl(n))
        .unwrap_or(0)
}

fn opt_hex(v: u64) -> String {
    if v == NONE {
        "none".to_owned()
    } else {
        format!("{v:#x}")
    }
}

impl RelayIdent {
    /// An identity from the channel's declared process id and engine type.
    #[must_use]
    pub fn new(pid: Option<u32>, engine: Option<u32>) -> Self {
        Self {
            pid: pid.map_or(NONE, u64::from),
            engine: engine.map_or(NONE, u64::from),
            kinds: AtomicU32::new(0),
        }
    }
    /// An engine object of this kind was kept on the channel (idempotent, wait-free, bounded: one bit per kind).
    pub fn note_class(&self, kind: Kind) {
        self.kinds.fetch_or(kind_bit(kind), Ordering::Relaxed);
    }
    /// `pid=.. classes=.. engine=..`: hex with a `0x` prefix, `none` when unknown; the classes are the
    /// [`Kind`] names in [`Kind::ALL`] order joined by `+`.
    #[must_use]
    pub fn fields(&self) -> String {
        let bits = self.kinds.load(Ordering::Relaxed);
        let names: Vec<String> = Kind::ALL
            .iter()
            .filter(|k| bits & kind_bit(**k) != 0)
            .map(|k| format!("{k:?}"))
            .collect();
        let classes = if names.is_empty() {
            "none".to_owned()
        } else {
            names.join("+")
        };
        format!(
            "pid={} classes={classes} engine={}",
            opt_hex(self.pid),
            opt_hex(self.engine)
        )
    }
}

/// The per-relay final line (`at=retire` or `at=exit`): the pre-existing `USER_RING_LIFECYCLE <key> mode=.. Counts {..}`
/// line with the identity and the bind facts appended after the closing brace. `None` for a relay that forwarded
/// nothing (as before).
///
/// `bound=yes` when the scan saw the software object bound and the channel left Scan; `scan_entries` is then the entries
/// forwarded in Scan before the bind, and for `bound=never` the entries forwarded in Scan over the relay's whole life.
#[must_use]
pub fn lifecycle_line(
    key: (u32, u32),
    at: At,
    life: &Lifecycle,
    ident: &RelayIdent,
) -> Option<String> {
    let c = life.counts();
    if c.entries_scan
        .saturating_add(c.entries_pointer)
        .saturating_add(c.entries_window)
        == 0
    {
        return None;
    }
    let (bound, scan_entries) = match life.bound() {
        Some(b) => ("yes", b.scan_entries),
        None => ("never", c.entries_scan),
    };
    Some(format!(
        "kf3: USER_RING_LIFECYCLE {key:x?} mode={:?} {c:?} {} bound={bound} scan_entries={scan_entries} at={}",
        life.mode(),
        ident.fields(),
        at.name()
    ))
}

/// The one-shot `USER_RING_BOUND` line for a bind (drained from [`crate::user_lifecycle::Notice::Bound`]).
#[must_use]
pub fn bound_line(key: (u32, u32), b: &Bound, ident: &RelayIdent) -> String {
    format!(
        "kf3: USER_RING_BOUND {key:x?} to={:?} scan_entries={} {}",
        b.to,
        b.scan_entries,
        ident.fields()
    )
}

/// The bounded in-run `USER_RING_SCAN_LONG` line (from [`crate::user_lifecycle::Notice::ScanLong`]).
#[must_use]
pub fn scan_long_line(
    key: (u32, u32),
    threshold: u64,
    scan_entries: u64,
    ident: &RelayIdent,
) -> String {
    let _ = (key, threshold, scan_entries, ident);
    String::new()
}
