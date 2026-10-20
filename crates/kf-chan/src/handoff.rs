//! 2026-10-20 -- **The lock HOLDER hands off to the loser** (`docs/design/V3_USER_RING_EVENT_LOOP_20261014.md` section 10, owner design).
//!
//! A relay whose `try_lock` fails (the `pt` map, the USERD / RM-graph admission, a source mapping swap) does not retry on a budget and does not
//! wait for a timer. It REGISTERS here before giving up; whoever held the lock RINGS the registered relays after releasing it
//! ([`Handoff::released`]). The race this closes -- the loser checked the lock, the holder released and looked at the (empty) waiter list, the
//! loser registered afterwards -- is closed by an EVENT COUNT: the loser reads [`Handoff::prepare`] BEFORE its attempt, and after registering asks
//! [`Handoff::changed_since`]; a holder bumps the epoch BEFORE it looks at the list, so either the holder sees the registration (and rings) or the
//! loser sees the epoch move (and retries at once). Both orders are sequentially consistent (`SeqCst`).
//!
//! Pure data and atomics, no I/O: the ring is a closure the holder supplies. The retries are driven by holder events, never by a clock.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// The registry of relays waiting for a lock holder.
#[derive(Debug, Default)]
pub struct Handoff {
    epoch: AtomicU64,
    waiters: AtomicU32,
    list: Mutex<Vec<u32>>,
    registered: AtomicU64,
    rung: AtomicU64,
    retried: AtomicU64,
}

impl Handoff {
    /// An empty registry.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            epoch: AtomicU64::new(0),
            waiters: AtomicU32::new(0),
            list: Mutex::new(Vec::new()),
            registered: AtomicU64::new(0),
            rung: AtomicU64::new(0),
            retried: AtomicU64::new(0),
        }
    }

    /// The loser, BEFORE its attempt: the epoch the attempt starts at.
    #[must_use]
    pub fn prepare(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    /// The loser, after a failed attempt: register `who` (idempotent).
    pub fn register(&self, who: u32) {
        if let Ok(mut l) = self.list.lock()
            && !l.contains(&who)
        {
            l.push(who);
            self.waiters.fetch_add(1, Ordering::SeqCst);
            self.registered.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The loser, AFTER registering: did a holder release since the attempt began? (`true` = retry now.)
    #[must_use]
    pub fn changed_since(&self, seen: u64) -> bool {
        self.epoch.load(Ordering::SeqCst) != seen
    }

    /// The loser: a failed attempt that began at `seen`. Registers; `true` = retry now (the holder may have released already).
    #[must_use]
    pub fn lost(&self, who: u32, seen: u64) -> bool {
        self.register(who);
        let again = self.changed_since(seen);
        if again {
            self.retried.fetch_add(1, Ordering::Relaxed);
        }
        again
    }

    /// The holder, AFTER releasing the lock: the first half of [`Handoff::released`].
    pub fn bump(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }

    /// Registered waiters (a cheap check).
    #[must_use]
    pub fn waiters(&self) -> u32 {
        self.waiters.load(Ordering::SeqCst)
    }

    /// Drain the registered waiters.
    #[must_use]
    pub fn take(&self) -> Vec<u32> {
        self.list
            .lock()
            .map(|mut l| {
                self.waiters.store(0, Ordering::SeqCst);
                std::mem::take(&mut *l)
            })
            .unwrap_or_default()
    }

    /// The holder, AFTER releasing the lock: bump the epoch, then ring every registered waiter.
    pub fn released(&self, mut ring: impl FnMut(u32)) {
        self.bump();
        if self.waiters() != 0 {
            for who in self.take() {
                self.rung.fetch_add(1, Ordering::Relaxed);
                ring(who);
            }
        }
    }

    /// One bounded line for the loop summary: registrations, rings by a holder, immediate retries (the lost-registration race answered).
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "handoff(registered={} rung={} retried={})",
            self.registered.load(Ordering::Relaxed),
            self.rung.load(Ordering::Relaxed),
            self.retried.load(Ordering::Relaxed)
        )
    }
}
