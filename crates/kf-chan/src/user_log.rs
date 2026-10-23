//! Relay observability lines of the Windows USER-ring relay (`docs/design/V3_RELAY_LIFECYCLE_LOG.md`).
//!
//! Pure formatting and bookkeeping, no I/O: the pump prints what these functions return, so the format and the
//! once-only / bounded rules are testable GPU-free. Every line is emitted at most once per relay event (bind,
//! retire, exit), never per entry and never per guest-controlled count.

use crate::user_lifecycle::{Bound, Lifecycle};

/// What a relay is, for the log lines: the declared process id and engine of its channel (both already in
/// `CHAN-FACTS`; nothing is captured anew) and the set of engine-object kinds allocated on it. Shared between the
/// channel's twin record (which notes a kind when an engine object is kept: wait-free) and the pump (which reads it
/// when it prints).
#[derive(Debug)]
pub struct RelayIdent {
    _stub: (),
}

/// Which final line: the relay retired, or it was still alive at device stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum At {
    /// The relay was released (channel free).
    Retire,
    /// The device stopped while the relay was alive.
    Exit,
}

impl RelayIdent {
    /// An identity from the channel's declared process id and engine type.
    #[must_use]
    pub fn new(pid: Option<u32>, engine: Option<u32>) -> Self {
        let _ = (pid, engine);
        Self { _stub: () }
    }
    /// An engine object of this kind was kept on the channel (idempotent, wait-free, bounded: one bit per kind).
    pub fn note_class(&self, kind: kf_chip::classes::Kind) {
        let _ = kind;
    }
    /// ` pid=.. classes=.. engine=..` without the leading space.
    #[must_use]
    pub fn fields(&self) -> String {
        String::new()
    }
}

/// The per-relay final line (`at=retire` or `at=exit`): the pre-existing `USER_RING_LIFECYCLE <key> mode=.. Counts {..}`
/// line with the identity and the bind facts appended. `None` for a relay that forwarded nothing.
#[must_use]
pub fn lifecycle_line(
    key: (u32, u32),
    at: At,
    life: &Lifecycle,
    ident: &RelayIdent,
) -> Option<String> {
    let _ = (key, at, life, ident);
    None
}

/// The one-shot `USER_RING_BOUND` line for a bind (drained from [`crate::user_lifecycle::Notice::Bound`]).
#[must_use]
pub fn bound_line(key: (u32, u32), b: &Bound, ident: &RelayIdent) -> String {
    let _ = (key, b, ident);
    String::new()
}
