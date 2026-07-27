//! Every way a raw-OS operation can refuse — one enum, exact variants.
//!
//! `l1_os_shell.md` §11 item 5: *"every `off + len` checked; every `u64 → usize`
//! fallible; `len == 0` refused; page rounding done in checked arithmetic. A rounding
//! overflow is the classic way a bounded object stops being bounded."* Each of those
//! five is a distinct variant here rather than a shared "bad argument", because the
//! test doctrine asserts **exact error variants, never `is_err()`** — a bounds test that
//! passes because the call overflowed somewhere unrelated has tested nothing.

use crate::cache::CachePolicy;
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
    /// ★ A mapping asked for a cache policy the **backing cannot attain** — the #111
    /// class, made loud.
    ///
    /// Page-cache-backed memory (anonymous, `memfd`, `tmpfs`, a regular file) is mapped
    /// write-back and there is no `mmap` flag that changes it. A caller that asks for
    /// write-combining or uncached over such a backing does not get it; without this
    /// refusal it gets **write-back and no diagnostic**, and the symptom surfaces days
    /// later as a coherence bug or a 60× bandwidth cliff. See [`CachePolicy`].
    ///
    /// [`CachePolicy`]: crate::CachePolicy
    CachePolicyUnattainable {
        /// What the call site asked for.
        requested: CachePolicy,
        /// The only policy this backing can actually have.
        attainable: CachePolicy,
        /// The backing that decides it (`"anonymous memory"`, `"a shared file"`).
        backing: &'static str,
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
    /// ★ The host cannot do this **at all** — a capability refusal, not an argument
    /// refusal.
    ///
    /// Distinct from every variant above because those describe a request that was wrong
    /// and this describes one that was fine and cannot be served here: a kernel whose KVM
    /// API version these structs are not written for, a cache attribute the platform does
    /// not offer. A caller retries a bad argument; a caller of this one must refuse
    /// **loudly at realize**, which is the §4.4.1 / §9.3 pattern for every deployment fact
    /// no type and no CI grep can observe.
    Unsupported {
        /// What was asked for.
        what: &'static str,
        /// Why this host cannot serve it, in operator-readable terms.
        detail: &'static str,
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
            RawError::CachePolicyUnattainable {
                requested,
                attainable,
                backing,
            } => write!(
                f,
                "{backing} is always {attainable}; a mapping of it cannot be {requested}, \
                 and asking silently yields {attainable}"
            ),
            RawError::NotWritable => write!(f, "the region is mapped read-only"),
            RawError::Syscall { call, errno } => match errno {
                Some(e) => write!(f, "{call} failed (errno {e})"),
                None => write!(f, "{call} failed"),
            },
            RawError::Unsupported { what, detail } => {
                write!(f, "this host cannot provide {what}: {detail}")
            }
            RawError::AbsurdPageSize { reported } => write!(
                f,
                "sysconf(_SC_PAGESIZE) reported {reported}, which cannot be a host page \
                 size (expected a power of two in [4096, 65536])"
            ),
        }
    }
}

impl std::error::Error for RawError {}

/// `errno` for the call that just failed, as a [`RawError::Syscall`].
///
/// Lives here — in the safe half — rather than beside the syscalls, because it is read
/// through `std::io::Error::last_os_error()` and needs no relaxation of its own. Every
/// `*_unsafe.rs` file calls it, so the `errno` capture is written once and cannot drift
/// into a per-file variant that forgets to read it.
pub(crate) fn last_syscall_error(call: &'static str) -> RawError {
    RawError::Syscall {
        call,
        errno: std::io::Error::last_os_error().raw_os_error(),
    }
}
