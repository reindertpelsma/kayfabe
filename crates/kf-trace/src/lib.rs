//! ★ **v3 `kf-trace` — the WIRE-PLANE record vocabulary and its differential.** ORACLE-ONLY.
//!
//! What a device looks like from outside: MMIO reads/writes, guest-RAM reads/writes, interrupt
//! raises, clock reads. It is the vocabulary the C artifact's §6 captures decode into
//! (`traces/mode2_c_reference/`), so a replay of the stock driver's cold boot can be diffed against
//! it positionally. Copied from the old `kayfabe-trace` MINUS its core-plane variants (address
//! binds, isolate verbs, joins) — v3 has none of those planes, so its trace cannot name them.
//!
//! ⊘ Never linked into the product: it is a test/oracle dependency of `kf-crec`.

pub mod replay;

use kf_arch::ids::Gpa;

pub use replay::{Divergence, OrderingError, check_dense_order, diff, diff_records};

/// A record's position in the totally-ordered stream.
///
/// See [`crate::Recorder`] for exactly what that order does and does not guarantee —
/// it is a property of the recorder, not of this number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Seq(pub u64);

impl core::fmt::Display for Seq {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// One sequenced observation: the unit a sink stores and a differential compares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Position in this recorder's total order.
    pub seq: Seq,
    /// What happened.
    pub ev: TraceEvent,
}

/// A PCI base address register index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Bar(pub u8);

/// The width of one register access.
///
/// A closed enum rather than the port plan's `size: u8`, so that a recorder replaying a
/// captured stream turns a corrupt width into a **decode refusal** instead of carrying a
/// nonsense number into the comparison. Same information, one fewer way to be wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Width {
    /// One byte.
    B1,
    /// Two bytes.
    B2,
    /// Four bytes — the overwhelmingly common register access.
    B4,
    /// Eight bytes.
    B8,
}

impl Width {
    /// Decode a captured byte count. `None` for anything a register access cannot be.
    #[must_use]
    pub fn from_bytes(n: u8) -> Option<Width> {
        match n {
            1 => Some(Width::B1),
            2 => Some(Width::B2),
            4 => Some(Width::B4),
            8 => Some(Width::B8),
            _ => None,
        }
    }

    /// The byte count.
    #[must_use]
    pub fn bytes(self) -> u8 {
        match self {
            Width::B1 => 1,
            Width::B2 => 2,
            Width::B4 => 4,
            Width::B8 => 8,
        }
    }
}

/// ★ One observation in the replay stream.
///
/// See the module docs for the two families and why they share one enum. Every variant
/// is something a *differential* can compare field by field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceEvent {
    // ───────────────────────────── the wire plane (§6.1) ─────────────────────────────
    /// A register read, carrying **the value that was served** — not the value the
    /// register nominally holds. That distinction is the whole point of recording reads.
    MmioRead {
        /// Which BAR trapped.
        bar: Bar,
        /// Offset within the BAR.
        off: u64,
        /// Access width.
        size: Width,
        /// The value served to the guest.
        val: u64,
    },
    /// A register write.
    MmioWrite {
        /// Which BAR trapped.
        bar: Bar,
        /// Offset within the BAR.
        off: u64,
        /// Access width.
        size: Width,
        /// The value the guest wrote.
        val: u64,
    },
    /// A guest-RAM read, carrying **the bytes it returned**. A replay that cannot answer
    /// a DMA read is not hermetic (§6.1), so the bytes are part of the record.
    GuestRead {
        /// Guest-physical address read.
        gpa: Gpa,
        /// The bytes returned.
        bytes: Vec<u8>,
    },
    /// A guest-RAM write — an assertion target: this is what the guest would observe.
    GuestWrite {
        /// Guest-physical address written.
        gpa: Gpa,
        /// The bytes written.
        bytes: Vec<u8>,
    },
    /// An interrupt raise — an assertion target.
    IrqRaise {
        /// How it was delivered.
        spec: IrqSpec,
    },
    /// A clock observation. Recorded so a replay is deterministic without reading a real
    /// clock: the stream *is* the timebase.
    Clock {
        /// Nanoseconds on the recorded timebase.
        ns: u64,
    },
}

/// An interrupt to inject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IrqSpec {
    /// MSI-X vector index. **The only variant the core emits** (`Msix(0)`, the
    /// `COMPLETION_VECTOR` edge) and therefore the only one every backend must support.
    Msix(u16),
    /// Legacy INTx line level.
    ///
    /// ★ **Backend-conditional, by design.** A cloud-hypervisor/rust-vmm adapter's
    /// legacy INTx path is a userspace IOAPIC that exists only on x86-64, so on
    /// **CH/aarch64 this variant is unimplementable** — such an adapter MUST return
    /// [`VmmError::Unsupported`] rather than silently dropping the injection. That is
    /// harmless today precisely because the core never emits it; it is written down so
    /// the first adapter to meet it treats it as a contract, not as a bug in the trait.
    IntxLevel(bool),
}

/// A fault's variant name — the part of an error a trace can compare across implementations
/// without comparing payloads that legitimately differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FaultTag(pub &'static str);

impl core::fmt::Display for FaultTag {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0)
    }
}
