//! Every way a raw-OS operation can refuse — one enum, exact variants.
//!
//! `l1_os_shell.md` §11 item 5: *"every `off + len` checked; every `u64 → usize`
//! fallible; `len == 0` refused; page rounding done in checked arithmetic. A rounding
//! overflow is the classic way a bounded object stops being bounded."* Each of those
//! five is a distinct variant here rather than a shared "bad argument", because the
//! test doctrine asserts **exact error variants, never `is_err()`** — a bounds test that
//! passes because the call overflowed somewhere unrelated has tested nothing.

use core::fmt;

/// A refusal from the raw-OS adapter.
///
/// Deliberately carries the *values* that were refused. A host CPU **address** is never
/// among them (§4.2.1 refusal 3): an integer host address in a `Debug` string is still a
/// host address, and error paths are exactly where such a thing gets logged, copied into
/// a message, and eventually reused.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RawError {
    /// A zero-length request. Always a refusal, never a silent no-op: a zero-length
    /// mapping is not a thing the kernel makes, and a zero-length *access* is almost
    /// always a length field the guest controls that arrived as 0.
    ZeroLength {
        /// What was zero (`"mapping length"`, `"read length"`, …).
        what: &'static str,
    },
    /// `offset + len` exceeds the bounded object's own length.
    OutOfRange {
        /// Offset requested, within the object.
        offset: u64,
        /// Length requested from that offset.
        len: u64,
        /// The object's total length — the bound that was exceeded.
        object_len: u64,
    },
    /// `offset + len` overflowed `u64` before any bound could be compared. Kept distinct
    /// from [`RawError::OutOfRange`] because an overflow means the *check itself* would
    /// have been wrong, which is the failure mode a bounds check is supposed to have.
    LengthOverflow {
        /// Offset operand.
        offset: u64,
        /// Length operand.
        len: u64,
    },
    /// A value the host requires to be page- (or word-) aligned was not.
    Misaligned {
        /// What was misaligned (`"mapping length"`, `"volatile load offset"`, …).
        what: &'static str,
        /// The offending value.
        value: u64,
        /// The alignment it had to satisfy.
        required: u64,
    },
    /// A `u64` quantity does not fit this host's `usize`. Reachable on a 32-bit host with
    /// a 64-bit guest geometry; fallible rather than `as`, per §11 item 5.
    TooLargeForHost {
        /// The value that did not fit.
        value: u64,
    },
    /// A placement would overlap one already made in the same [`Reservation`].
    ///
    /// [`Reservation`]: crate::Reservation
    OverlappingPlacement {
        /// Offset of the placement requested.
        offset: u64,
        /// Length of the placement requested.
        len: u64,
        /// Offset of the existing placement it collides with.
        existing_offset: u64,
        /// Length of that existing placement.
        existing_len: u64,
    },
    /// A [`PlacementId`] that this reservation never minted (or that came from another).
    ///
    /// [`PlacementId`]: crate::PlacementId
    UnknownPlacement {
        /// The id presented.
        id: u64,
    },
    /// A write to a region mapped read-only. Refused *before* the store, because the
    /// alternative is a `SIGSEGV` — and a read-only isolate mapping (§11 item 3) is
    /// precisely a place a caller can get this wrong.
    NotWritable,
    /// A syscall failed. `errno` is captured through `std::io::Error::last_os_error`, so
    /// no additional relaxation is needed to read it.
    Syscall {
        /// Which call (`"mmap"`, `"munmap"`).
        call: &'static str,
        /// The raw `errno`, or `None` if the OS reported none.
        errno: Option<i32>,
    },
    /// `sysconf(_SC_PAGESIZE)` reported something that cannot be a host page size — not a
    /// power of two, or outside [4 KiB, 64 KiB]. A loud startup fault, never a silent
    /// misalignment (§5.2 item 1).
    AbsurdPageSize {
        /// What was reported.
        reported: i64,
    },
}

impl fmt::Display for RawError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RawError::ZeroLength { what } => write!(f, "{what} is zero"),
            RawError::OutOfRange {
                offset,
                len,
                object_len,
            } => write!(
                f,
                "offset {offset:#x} + len {len:#x} exceeds the object's {object_len:#x} bytes"
            ),
            RawError::LengthOverflow { offset, len } => {
                write!(f, "offset {offset:#x} + len {len:#x} overflows u64")
            }
            RawError::Misaligned {
                what,
                value,
                required,
            } => write!(f, "{what} {value:#x} is not aligned to {required:#x} bytes"),
            RawError::TooLargeForHost { value } => {
                write!(f, "{value:#x} does not fit this host's usize")
            }
            RawError::OverlappingPlacement {
                offset,
                len,
                existing_offset,
                existing_len,
            } => write!(
                f,
                "placement at {offset:#x}+{len:#x} overlaps the existing placement at \
                 {existing_offset:#x}+{existing_len:#x}"
            ),
            RawError::UnknownPlacement { id } => write!(f, "no placement with id {id}"),
            RawError::NotWritable => write!(f, "the region is mapped read-only"),
            RawError::Syscall { call, errno } => match errno {
                Some(e) => write!(f, "{call} failed (errno {e})"),
                None => write!(f, "{call} failed"),
            },
            RawError::AbsurdPageSize { reported } => write!(
                f,
                "sysconf(_SC_PAGESIZE) reported {reported}, which cannot be a host page \
                 size (expected a power of two in [4096, 65536])"
            ),
        }
    }
}

impl std::error::Error for RawError {}
