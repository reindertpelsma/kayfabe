//! Completions and interrupts — §8.
//!
//! ## The completion rule is per ROUTE, and two of the three are "nothing"
//!
//! | route | completion |
//! |---|---|
//! | `Passthrough` | **nothing.** We do not inspect the channel and do not care when it finishes |
//! | `Translated` | **nothing.** The semaphore address was forwarded; **the GPU writes it** |
//! | `Emulated` | forged at the end of our own call — there was no GPU work |
//!
//! ⊘⊘ **"Forge" is licensed ONLY where there was no work.** §8: *"A completion written for work
//! that did not happen is how a scrub becomes a leak."* That is not a slogan — it is this
//! campaign's most expensive measured defect, and [`Completion::forge`] refuses to express it.
//!
//! ## Interrupts carry no identity (§8)
//!
//! *"Interrupt registration is per ENGINE, not per channel and not per semaphore: the event object
//! requires a subdevice notifier, the index names an engine, and the channel class has no
//! completion event at all."* ⇒ An interrupt is a **broadcast wake to every waiter on that
//! engine**.
//!
//! ★ **The waiter owns the race** — RM's own code does this three independent ways — so we fire on
//! *genuinely new completed work* and need **not** fire retroactively.
//! ⚠ **The converse binds: once armed, a later release MUST produce an interrupt. There is no
//! level-triggered fallback.**

use kf_trap::token::Route;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// What, if anything, we owe the guest when a unit of work retires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completion {
    /// Nothing is owed — the GPU already wrote it, or there is nothing to write.
    Nothing,
    /// Write the payload ourselves, because no GPU work existed.
    Forge,
}

impl Completion {
    /// ★ The whole rule, in one total function over the route.
    ///
    /// ⊘ `did_gpu_work` is not advisory. §8 licenses a forge **only** where there was no work, so
    /// an `Emulated` route that somehow DID reach hardware must not forge — that is precisely the
    /// scrub-becomes-a-leak shape, and the type refuses it rather than trusting the caller to
    /// remember.
    pub fn for_route(route: Route, did_gpu_work: bool) -> Completion {
        match route {
            // We never looked at the channel; its completion is not ours to write.
            Route::Passthrough => Completion::Nothing,
            // The semaphore address was FORWARDED. The GPU writes it. Writing it ourselves would
            // be a second author for one value.
            Route::Translated => Completion::Nothing,
            Route::Emulated if !did_gpu_work => Completion::Forge,
            // ⊘ Emulated but work reached the GPU ⇒ the GPU owns the write. Forging here is the
            // leak.
            Route::Emulated => Completion::Nothing,
            // An unknown route is never completed. It should never have been served.
            Route::Unknown => Completion::Nothing,
        }
    }
}

/// One engine's interrupt arming state.
///
/// §8: *"An interrupt is a broadcast wake to every waiter on that engine, carrying no identity."*
/// ⇒ There is nothing per-channel here, deliberately.
#[derive(Debug, Default)]
pub struct EngineIrq {
    /// Set when the guest has unmasked/armed this engine.
    armed: AtomicBool,
    /// Pending edge, held while masked. §5.5's interrupt-masking hazard lives here: the ISR
    /// writes the mask then reads pending, so the two must be one consistent state.
    pending: AtomicBool,
    /// Work retired since the last edge we delivered. Fires on *new* work only.
    retired: AtomicU64,
    delivered: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Raise {
    /// Deliver an interrupt for this engine now.
    Deliver,
    /// Nothing new, or masked. ⊘ Held, not dropped — see [`EngineIrq::unmask`].
    Hold,
}

impl EngineIrq {
    pub const fn new() -> EngineIrq {
        EngineIrq {
            armed: AtomicBool::new(false),
            pending: AtomicBool::new(false),
            retired: AtomicU64::new(0),
            delivered: AtomicU64::new(0),
        }
    }

    pub fn arm(&self) {
        self.armed.store(true, Ordering::Release);
    }

    /// Work retired on this engine.
    ///
    /// ⚠ §8's binding converse: *"once armed, a later release MUST produce an interrupt. There is
    /// no level-triggered fallback."* ⇒ When masked we set `pending` and [`Self::unmask`] must
    /// deliver it. Dropping the edge here would hang a waiter forever with no way to recover.
    pub fn retire(&self) -> Raise {
        self.retired.fetch_add(1, Ordering::AcqRel);
        if !self.armed.load(Ordering::Acquire) {
            // Not armed: the waiter is not listening, and §8 says we need not fire
            // retroactively — but we must remember, because arming later is allowed.
            self.pending.store(true, Ordering::Release);
            return Raise::Hold;
        }
        self.delivered.fetch_add(1, Ordering::AcqRel);
        Raise::Deliver
    }

    /// The guest unmasked. ⊘ **Re-evaluate, do not merely clear.** §5.5: the ISR writes the mask
    /// then reads pending, so an edge that arrived while masked must surface now or be lost.
    pub fn unmask(&self) -> Raise {
        self.armed.store(true, Ordering::Release);
        if self.pending.swap(false, Ordering::AcqRel) {
            self.delivered.fetch_add(1, Ordering::AcqRel);
            Raise::Deliver
        } else {
            Raise::Hold
        }
    }

    pub fn mask(&self) {
        self.armed.store(false, Ordering::Release);
    }

    /// ⚠ §8: *"The channels we care about most POLL — UVM spins, and the scrubber's blocking waits
    /// loop on the semaphore word."* ⇒ A zero here is not necessarily a defect; it is why the
    /// interrupt plane is not the completion plane.
    #[inline]
    pub fn delivered(&self) -> u64 {
        self.delivered.load(Ordering::Acquire)
    }
    #[inline]
    pub fn retired_count(&self) -> u64 {
        self.retired.load(Ordering::Acquire)
    }
}
