//! The register read shadow — §5.5.
//!
//! **Reads are not trapped.** The guest reads BAR0 from a page of host memory installed as a
//! guest-physical memslot, so a write must land in that page **synchronously, in the trap**,
//! before anything is queued. §5.5: *"a deferred write leaves the guest reading stale state. The
//! sharp case is interrupt masking: the ISR writes the mask then reads pending, and a deferred
//! mask delivers an interrupt the guest already masked."*
//!
//! ⚠ **A plain store is wrong for four of the five kinds** — hence [`WriteSemantics`].

use core::sync::atomic::{AtomicU64, Ordering};

/// §5.5's table. ⊘ **This cannot be generated from the published access codes, and that is
/// MEASURED**: the interrupt-pending register (write-1-to-clear) and its enable-set port carry
/// the **same** code, and the invalidate trigger reads as plain read-write. ⇒ read/write-only-ness
/// is generated; this is a **hand-maintained per-family overlay of about a dozen names**, and
/// **it must fail the build when a register in the generated set has no entry.**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteSemantics {
    /// Store.
    Plain,
    /// `fetch_and(!bits)`. ⊘ A load-store here **loses a worker's concurrent set** — the worker
    /// is setting pending bits from the host edge while the guest's ISR clears the ones it saw.
    W1c,
    /// The readable effect is on a **different** register; this one's own shadow must not move.
    WriteOnlyPort,
    /// The guest writes 1 and **spin-reads until 0** ⇒ cleared **after the work is done**, by
    /// whoever did the work. See [`Trigger`].
    Trigger,
    /// An auto-incrementing image port ⇒ a synchronous store into a shadow image (§5.4). ⊘ These
    /// must never enter the privileged ring: Turing/GA100 firmware load is **16 000–65 000
    /// back-to-back writes** and would overflow any ring that exists.
    DataPort,
}

/// One shadowed register cell.
#[derive(Debug, Default)]
pub struct Cell(AtomicU64);

impl Cell {
    pub const fn new() -> Cell {
        Cell(AtomicU64::new(0))
    }
    #[inline]
    pub fn read(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }

    /// Apply a guest write under its semantics. ★ Called **on the vCPU, in the trap**, before
    /// anything is queued.
    pub fn apply(&self, sem: WriteSemantics, value: u64) {
        match sem {
            WriteSemantics::Plain => self.0.store(value, Ordering::Release),
            // ⊘ fetch_and, NEVER load-then-store.
            WriteSemantics::W1c => {
                self.0.fetch_and(!value, Ordering::AcqRel);
            }
            // ⊘ The readable effect is elsewhere; touching this cell would invent a value the
            // hardware does not have.
            WriteSemantics::WriteOnlyPort => {}
            // The trigger's shadow is owned by [`Trigger`], not by a bare store: it must carry
            // the ring position so the clear can name what it completes.
            WriteSemantics::Trigger => {}
            WriteSemantics::DataPort => {}
        }
    }
}

/// ★★★ A trigger register, and the reason it is not a bool.
///
/// §5.5: *"the guest writes 1 and spin-reads until 0"*. The drainer **hands the trigger to the
/// thread that does the work and moves on** — it must never wait, because an invalidate trigger
/// completes only when the VA manager has applied the diff, which waits on the GPU walker and on
/// host mapping calls taken under the host driver's own global lock. ⊘ If the drainer blocked
/// there, *"everything queued behind it stalls"*.
///
/// ## ⊘⊘⊘ And the clear must NAME what it completes, or a slow invalidate silently corrupts
///
/// §5.5 spells out the corruption, and it is not hypothetical — it is the guest driver's
/// documented behaviour:
///
/// > The guest's invalidate spin has a timeout of a few seconds, after which **the driver proceeds
/// > anyway, with stale translations and no error**. So: the guest times out on invalidate *A* and
/// > continues; it later issues invalidate *B*; our work for *A* finishes and clears the trigger;
/// > the guest reads zero and concludes *B* is done. **It is not.**
///
/// ⇒ The shadow carries the **ring position of the write it belongs to**, and the clear is a
/// **compare-and-set against that position — never a blind store.**
#[derive(Debug, Default)]
pub struct Trigger {
    /// 0 = idle. Otherwise `seq + 1` of the ring write that armed it.
    armed_at: AtomicU64,
    issued: AtomicU64,
    completed: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearOutcome {
    /// We cleared the trigger we were given.
    Cleared,
    /// ⊘ **A LATER write has re-armed it.** Our completion is stale and MUST NOT clear — doing so
    /// would tell the guest that *B* finished when only *A* did. This is the silent-corruption
    /// path, refused by name.
    Superseded,
    /// Already idle; nothing to do.
    AlreadyIdle,
}

impl Trigger {
    pub const fn new() -> Trigger {
        Trigger { armed_at: AtomicU64::new(0), issued: AtomicU64::new(0), completed: AtomicU64::new(0) }
    }

    /// vCPU, in the trap: the guest wrote 1. Record WHICH write this is.
    pub fn arm(&self, ring_seq: u64) {
        self.armed_at.store(ring_seq + 1, Ordering::Release);
        self.issued.fetch_add(1, Ordering::AcqRel);
    }

    /// ★ P4 (w826): vCPU, in the trap — arm under a sequence this trigger MINTS, and return it.
    ///
    /// ⊘ Why not [`Trigger::arm`] with the ring's sequence: the ring hands out its position only
    /// once the write is PUSHED, and a pushed write can be drained, walked, reconciled and
    /// `complete`d before the vCPU gets back to `arm` — which would find the trigger idle
    /// ([`ClearOutcome::AlreadyIdle`]) and then arm it forever. Minting the sequence here lets
    /// the trap arm FIRST and publish second, so the completion can never overtake the arm.
    ///
    /// `fetch_max`, not `store`: two vCPUs racing here must leave the trigger naming the LATER
    /// write, or completing the earlier one would clear the later one early (§5.5's corruption).
    pub fn arm_next(&self) -> u64 {
        let seq = self.issued.fetch_add(1, Ordering::AcqRel);
        self.armed_at.fetch_max(seq + 1, Ordering::AcqRel);
        seq
    }

    /// The guest's spin-read. Non-zero means "still working".
    #[inline]
    pub fn read(&self) -> u64 {
        u64::from(self.armed_at.load(Ordering::Acquire) != 0)
    }

    /// The owning thread finished the work for `ring_seq`.
    ///
    /// ⊘ **Compare-and-set against the position, never a blind store.**
    pub fn complete(&self, ring_seq: u64) -> ClearOutcome {
        let want = ring_seq + 1;
        match self.armed_at.compare_exchange(want, 0, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => {
                self.completed.fetch_add(1, Ordering::AcqRel);
                ClearOutcome::Cleared
            }
            Err(0) => ClearOutcome::AlreadyIdle,
            // A later arm() replaced our position. Our work is done but it is not the work the
            // guest is currently waiting on.
            Err(_) => {
                self.completed.fetch_add(1, Ordering::AcqRel);
                ClearOutcome::Superseded
            }
        }
    }

    /// §5.5: the drainer keeps *issued* and *completed*; doorbell workers fence on **completed**,
    /// and only for the polled class.
    #[inline]
    pub fn issued(&self) -> u64 {
        self.issued.load(Ordering::Acquire)
    }
    #[inline]
    pub fn completed(&self) -> u64 {
        self.completed.load(Ordering::Acquire)
    }

    /// ⚠ §5.5's tripwire: *"a trigger outstanding past a fraction of the guest's own timeout
    /// budget"* is a fault, not a slow path — because past the guest's timeout it proceeds with
    /// stale translations and no error, and we would never know.
    #[inline]
    pub fn is_overdue(&self, armed_nanos_ago: u64, guest_timeout_nanos: u64) -> bool {
        self.armed_at.load(Ordering::Acquire) != 0 && armed_nanos_ago > guest_timeout_nanos / 2
    }
}
