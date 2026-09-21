//! Host descriptors and teardown order — §9.
//!
//! ## Two descriptors, two lifetimes
//!
//! | descriptor | lives for | owns |
//! |---|---|---|
//! | **A** | the VM | the single store, the host register mappings, the walker's GPU context |
//! | **B** | one guest **driver instance** | every twin — devices, VA spaces, channels, engine objects, events, mappings |
//!
//! ## ⊘⊘⊘ `close()` IS NOT A SYNCHRONOUS FREE, and relying on it leaves the hazard it should close
//!
//! §9 names two mechanisms that defeat it:
//! - *"`release` runs on the **last file reference**, not on `close`. Every in-flight call on that
//!   descriptor and every mapping made through it holds one."* ⇒ `close` returns while teardown
//!   has not started.
//! - *"The driver's close path **defers the whole cleanup to a kernel thread** when its
//!   interruptible wait fails — which **a pending signal causes**. VMM threads carry signals
//!   routinely."* ⇒ the deferred path is the **normal case under load**, not an edge case.
//!
//! ★ **Why this is not hygiene:** *"a still-scheduled host channel whose ring lives in guest RAM
//! the guest has since reused is **a live DMA engine writing into another guest's memory**."* The
//! guarantee must be *the engine is stopped before the memory is reused*, and only the explicit
//! free gives that ordering.

/// The teardown steps, **in the order §9 requires**. ⊘ The order is the product here; a set would
/// lose it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Step {
    /// Our own order first: stop the workers touching twins.
    QuiesceWorkers = 0,
    /// Reset the privileged ring.
    ResetRing = 1,
    /// Drain the VA manager — it owns every `mmap`/`munmap`.
    DrainVaManager = 2,
    /// Unmap every mapping made through the descriptor. ⊘ Each one holds a file reference, so
    /// skipping this makes `close` a no-op.
    UnmapAll = 3,
    /// Join every in-flight call. Same reason.
    JoinInFlight = 4,
    /// ⊘ **Block signals.** A pending signal is what pushes the driver's close path onto a kernel
    /// thread, turning the free asynchronous.
    BlockSignals = 5,
    /// ⊘⊘⊘ **Reset the walker's previous-state snapshot.** §9: it *"lives across a driver
    /// instance; if it survives while every mapping is dropped, the next instance's first diff
    /// reports 'unchanged' and maps NOTHING — a second boot that faults for reasons the first did
    /// not."*
    ResetWalkerState = 6,
    /// The explicit, synchronous free of the client tree — ordered by the host driver's own
    /// dependency rules, preempting and unbinding channels.
    FreeClientTree = 7,
    /// ★ Only now.
    CloseDescriptor = 8,
}

pub const ORDER: [Step; 9] = [
    Step::QuiesceWorkers,
    Step::ResetRing,
    Step::DrainVaManager,
    Step::UnmapAll,
    Step::JoinInFlight,
    Step::BlockSignals,
    Step::ResetWalkerState,
    Step::FreeClientTree,
    Step::CloseDescriptor,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeardownError {
    /// A step ran before one that must precede it.
    OutOfOrder { attempted: Step, expected: Step },
    /// ⊘ `close` was reached without the explicit free. §9's central refusal.
    ClosedWithoutExplicitFree,
}

/// Drives one descriptor's teardown and refuses any order but §9's.
#[derive(Debug, Default)]
pub struct Teardown {
    next: usize,
}

impl Teardown {
    pub fn new() -> Teardown {
        Teardown { next: 0 }
    }

    pub fn run(&mut self, s: Step) -> Result<(), TeardownError> {
        let expected = ORDER[self.next.min(ORDER.len() - 1)];
        if s != expected {
            // ⊘ Reaching close early is called out by name, because it is the specific mistake
            // §9 exists to prevent — not a generic ordering slip.
            if s == Step::CloseDescriptor && self.next < ORDER.len() - 1 {
                return Err(TeardownError::ClosedWithoutExplicitFree);
            }
            return Err(TeardownError::OutOfOrder { attempted: s, expected });
        }
        self.next += 1;
        Ok(())
    }

    pub fn complete(&self) -> bool {
        self.next == ORDER.len()
    }
}

/// The GPU walker's cross-instance state.
///
/// ⊘⊘ Modelled here rather than in the walker because the **bug is a lifetime bug**: the snapshot
/// outliving the mappings it describes.
#[derive(Debug, Default)]
pub struct WalkerState {
    snapshot_generation: u64,
    has_snapshot: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Diff {
    /// Mappings to apply.
    Changed(u64),
    /// ⊘ Nothing to do. **Correct only if a snapshot genuinely described the same state.**
    Unchanged,
}

impl WalkerState {
    /// Take a diff against the previous snapshot.
    pub fn diff(&mut self, live_generation: u64) -> Diff {
        if self.has_snapshot && self.snapshot_generation == live_generation {
            return Diff::Unchanged;
        }
        self.snapshot_generation = live_generation;
        self.has_snapshot = true;
        Diff::Changed(live_generation)
    }

    /// §9's mandatory teardown step. ⊘ Without it the next driver instance starts with a snapshot
    /// describing mappings that no longer exist.
    pub fn reset(&mut self) {
        self.has_snapshot = false;
        self.snapshot_generation = 0;
    }
}
