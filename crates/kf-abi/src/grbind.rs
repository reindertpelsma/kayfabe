//! Bounded GR bind intent. Decoding/authoring is added by the tests-first adapter change.

use crate::DriverVersion;

/// A logical guest target, never a host handle authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    /// Logical owner client.
    pub client: u32,
    /// Logical channel or TSG.
    pub channel: u32,
    /// Scalar mode and buffer intent.
    pub kind: Kind,
}

/// SDK bind variants; the maximum pointer array is a public ABI bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Zcull save/restore state.
    Zcull { va: u64, mode: u32 },
    /// PM save/restore state.
    Pm { va: u64, mode: u32 },
    /// Graphics/compute preemption state.
    Preemption {
        flags: u32,
        gfxp: u32,
        cilp: u32,
        buffers: [u64; 9],
    },
}

/// Refusal seam for a deferred union member. No runtime caller yet.
pub fn decode(_v: DriverVersion, _cmd: u32, _bytes: &[u8]) -> Option<Request> {
    None
}
