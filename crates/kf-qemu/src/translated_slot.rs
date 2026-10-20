//! Translated slot exclusion and lock-holder handoff.
//!
//! A worker reads the release epoch before trying the slot. On contention it
//! registers a coalesced obligation, then rechecks the epoch before giving up.
//! Every guard unlocks before advancing the epoch and ringing that obligation.
//! Either the holder sees the registration or the loser sees the release. No
//! registry mutex, allocation, retry budget or timer carries correctness.

use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LockResult, Mutex, MutexGuard, PoisonError, TryLockError, TryLockResult};

#[derive(Clone, Copy)]
pub(crate) struct Identity {
    pub(crate) idx: u32,
    pub(crate) tsg: Option<u32>,
}

#[derive(Default)]
struct Retry {
    epoch: AtomicU64,
    again: AtomicBool,
}

impl Retry {
    fn prepare(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    fn lost(&self, seen: u64) -> bool {
        self.register();
        self.changed_since(seen)
    }

    fn register(&self) {
        self.again.store(true, Ordering::SeqCst);
    }

    fn changed_since(&self, seen: u64) -> bool {
        self.epoch.load(Ordering::SeqCst) != seen
    }

    fn bump(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }

    fn take(&self) -> bool {
        self.again.swap(false, Ordering::SeqCst)
    }

    fn released(&self) -> bool {
        self.bump();
        self.take()
    }
}

pub(crate) struct Cell<T> {
    inner: Mutex<T>,
    identity: Identity,
    retry: Retry,
    ring: Box<dyn Fn(u32) + Send + Sync>,
}

pub(crate) struct Guard<'a, T> {
    g: Option<MutexGuard<'a, T>>,
    cell: &'a Cell<T>,
}

impl<T> Cell<T> {
    pub(crate) fn new(
        value: T,
        identity: Identity,
        ring: impl Fn(u32) + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner: Mutex::new(value),
            identity,
            retry: Retry::default(),
            ring: Box::new(ring),
        }
    }

    fn wrap<'a>(&'a self, g: MutexGuard<'a, T>) -> Guard<'a, T> {
        Guard {
            g: Some(g),
            cell: self,
        }
    }

    /// Blocking acquisition is for the act thread only.
    pub(crate) fn lock(&self) -> LockResult<Guard<'_, T>> {
        self.inner
            .lock()
            .map(|g| self.wrap(g))
            .map_err(|e| PoisonError::new(self.wrap(e.into_inner())))
    }

    /// Diagnostics may skip a busy slot, but their releases still hand off.
    pub(crate) fn try_lock(&self) -> TryLockResult<Guard<'_, T>> {
        match self.inner.try_lock() {
            Ok(g) => Ok(self.wrap(g)),
            Err(TryLockError::WouldBlock) => Err(TryLockError::WouldBlock),
            Err(TryLockError::Poisoned(e)) => Err(TryLockError::Poisoned(PoisonError::new(
                self.wrap(e.into_inner()),
            ))),
        }
    }

    /// A worker records its lost attempt before returning to the plane.
    pub(crate) fn enter(&self) -> TryLockResult<Guard<'_, T>> {
        let seen = self.retry.prepare();
        let result = self.try_lock();
        if matches!(result, Err(TryLockError::WouldBlock)) && self.retry.lost(seen) {
            (self.ring)(self.identity.idx);
        }
        result
    }

    /// Birth-time routing does not depend on the pump's mutable state.
    pub(crate) fn identity(&self) -> Option<Identity> {
        (!self.inner.is_poisoned()).then_some(self.identity)
    }
}

impl<T> Deref for Guard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.g.as_deref().expect("guard alive")
    }
}

impl<T> DerefMut for Guard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.g.as_deref_mut().expect("guard alive")
    }
}

impl<T> Drop for Guard<'_, T> {
    fn drop(&mut self) {
        drop(self.g.take());
        if self.cell.retry.released() {
            (self.cell.ring)(self.cell.identity.idx);
        }
    }
}

#[cfg(test)]
#[path = "translated_slot_tests.rs"]
mod tests;
