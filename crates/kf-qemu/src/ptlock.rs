//! 2026-10-20 -- **A mutex whose release hands off** (`kf_chan::handoff`, `docs/design/V3_USER_RING_EVENT_LOOP_20261014.md` section 10).
//!
//! [`PtLock`] is the `pt` map's lock: the same `lock` / `try_lock` as a std `Mutex`, and its guard, AFTER it has released the lock, tells the
//! [`HandoffHub`] ([`HandoffHub::released`]): the epoch moves and every relay that lost a `try_lock` and registered is rung. No lock is taken
//! by the guard's drop beyond the registry's own list (touched only while a loser is registered), no timer, nothing blocks.

use kf_chan::handoff::Handoff;
use std::ops::{Deref, DerefMut};
use std::sync::{LockResult, Mutex, MutexGuard, PoisonError, TryLockError, TryLockResult};

/// The registry plus the way to ring a relay (`Plane::ring_internal` + the worker eventfd in production).
pub(crate) struct HandoffHub {
    h: Handoff,
    ring: Box<dyn Fn(u32) + Send + Sync>,
}

impl HandoffHub {
    pub(crate) fn new(ring: impl Fn(u32) + Send + Sync + 'static) -> Self {
        Self {
            h: Handoff::new(),
            ring: Box::new(ring),
        }
    }
    /// A hub that rings nothing (tests).
    #[cfg(test)]
    pub(crate) fn detached() -> Self {
        Self::new(|_| {})
    }
    /// The epoch to read BEFORE an attempt.
    pub(crate) fn prepare(&self) -> u64 {
        self.h.prepare()
    }
    /// A failed attempt that began at `seen`: register `who`; `true` = a holder released meanwhile, retry now.
    pub(crate) fn lost(&self, who: u32, seen: u64) -> bool {
        self.h.lost(who, seen)
    }
    /// A holder released: ring the registered losers. Call AFTER the lock is released.
    pub(crate) fn released(&self) {
        self.h.released(|who| (self.ring)(who));
    }
    /// Registered, not yet rung losers (the strand diagnostic prints it).
    pub(crate) fn waiters(&self) -> u32 {
        self.h.waiters()
    }
    /// One bounded line for the loop summary.
    pub(crate) fn summary(&self) -> String {
        self.h.summary()
    }
}

pub(crate) struct PtLock<T> {
    inner: Mutex<T>,
    hub: std::sync::Arc<HandoffHub>,
}

pub(crate) struct PtGuard<'a, T> {
    g: Option<MutexGuard<'a, T>>,
    hub: &'a HandoffHub,
}

impl<T> PtLock<T> {
    pub(crate) fn new(value: T, hub: std::sync::Arc<HandoffHub>) -> Self {
        Self {
            inner: Mutex::new(value),
            hub,
        }
    }
    fn wrap<'a>(&'a self, g: MutexGuard<'a, T>) -> PtGuard<'a, T> {
        PtGuard {
            g: Some(g),
            hub: &self.hub,
        }
    }
    pub(crate) fn lock(&self) -> LockResult<PtGuard<'_, T>> {
        match self.inner.lock() {
            Ok(g) => Ok(self.wrap(g)),
            Err(e) => Err(PoisonError::new(self.wrap(e.into_inner()))),
        }
    }
    #[cfg(test)]
    pub(crate) fn is_poisoned(&self) -> bool {
        self.inner.is_poisoned()
    }
    pub(crate) fn try_lock(&self) -> TryLockResult<PtGuard<'_, T>> {
        match self.inner.try_lock() {
            Ok(g) => Ok(self.wrap(g)),
            Err(TryLockError::WouldBlock) => Err(TryLockError::WouldBlock),
            Err(TryLockError::Poisoned(e)) => Err(TryLockError::Poisoned(PoisonError::new(
                self.wrap(e.into_inner()),
            ))),
        }
    }
}

impl<T> Deref for PtGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.g.as_deref().expect("guard alive")
    }
}
impl<T> DerefMut for PtGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.g.as_deref_mut().expect("guard alive")
    }
}
impl<T> Drop for PtGuard<'_, T> {
    fn drop(&mut self) {
        // release FIRST, then tell: the loser's retry must find the lock free
        drop(self.g.take());
        self.hub.released();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn lock_with_ring() -> (
        PtLock<u32>,
        Arc<HandoffHub>,
        Arc<std::sync::Mutex<Vec<u32>>>,
    ) {
        let rung = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = Arc::clone(&rung);
        let hub = Arc::new(HandoffHub::new(move |who| sink.lock().unwrap().push(who)));
        (PtLock::new(0, Arc::clone(&hub)), hub, rung)
    }

    /// The guard releases the lock FIRST and only then tells the hub: a registered loser is rung exactly once, and its retry finds the
    /// lock free.
    #[test]
    fn a_registered_loser_is_rung_after_the_holder_releases() {
        let (lock, hub, rung) = lock_with_ring();
        let g = lock.lock().unwrap();
        let seen = hub.prepare();
        assert!(matches!(lock.try_lock(), Err(TryLockError::WouldBlock)));
        assert!(
            !hub.lost(5, seen),
            "no release since the attempt began: stay registered"
        );
        assert!(
            rung.lock().unwrap().is_empty(),
            "nothing rung while the lock is held"
        );
        drop(g);
        assert_eq!(
            *rung.lock().unwrap(),
            vec![5],
            "the holder rang the registered relay after releasing"
        );
        assert!(lock.try_lock().is_ok(), "the retry finds the lock free");
        assert!(
            hub.summary().contains("registered=1 rung=1"),
            "{}",
            hub.summary()
        );
    }

    /// A holder that released between the loser's attempt and its registration: the epoch tells the loser to retry at once.
    #[test]
    fn a_release_before_the_registration_tells_the_loser_to_retry_at_once() {
        let (lock, hub, rung) = lock_with_ring();
        let seen = hub.prepare();
        drop(lock.lock().unwrap());
        assert!(
            hub.lost(9, seen),
            "a holder released since the attempt began"
        );
        assert!(
            rung.lock().unwrap().is_empty(),
            "the registry was empty at the release: the loser retries itself"
        );
    }

    /// A hub whose ring records whether the lock was FREE at ring time (a woken loser retries at once: it must win).
    fn lock_whose_ring_probes_the_lock() -> (Arc<PtLock<u32>>, Arc<AtomicU64>, Arc<AtomicU64>) {
        let slot: Arc<std::sync::OnceLock<Arc<PtLock<u32>>>> = Arc::new(std::sync::OnceLock::new());
        let (free, busy) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
        let (s2, f2, b2) = (Arc::clone(&slot), Arc::clone(&free), Arc::clone(&busy));
        let hub = Arc::new(HandoffHub::new(move |_who| {
            let l = s2.get().expect("lock set");
            if l.try_lock().is_ok() {
                f2.fetch_add(1, Ordering::SeqCst);
            } else {
                b2.fetch_add(1, Ordering::SeqCst);
            }
        }));
        let lock = Arc::new(PtLock::new(0, hub));
        let _ = slot.set(Arc::clone(&lock));
        (lock, free, busy)
    }

    /// The ring comes AFTER the release: a loser rung by the holder finds the lock free (one loser).
    #[test]
    fn the_ring_finds_the_lock_free_one_loser() {
        let (lock, free, busy) = lock_whose_ring_probes_the_lock();
        let g = lock.lock().unwrap();
        let seen = lock.hub.prepare();
        assert!(matches!(lock.try_lock(), Err(TryLockError::WouldBlock)));
        assert!(!lock.hub.lost(1, seen));
        drop(g);
        assert_eq!(
            (free.load(Ordering::SeqCst), busy.load(Ordering::SeqCst)),
            (1, 0),
            "the ring happened while the lock was still held"
        );
    }

    /// The same for several losers and for a guard taken through `try_lock`.
    #[test]
    fn the_ring_finds_the_lock_free_for_every_loser() {
        let (lock, free, busy) = lock_whose_ring_probes_the_lock();
        let g = lock.try_lock().unwrap();
        let seen = lock.hub.prepare();
        for who in [3, 4, 5] {
            assert!(!lock.hub.lost(who, seen));
        }
        drop(g);
        assert_eq!(
            (free.load(Ordering::SeqCst), busy.load(Ordering::SeqCst)),
            (3, 0),
            "a loser was rung while the lock was still held"
        );
    }

    /// The wrapper is a std mutex otherwise: exclusion, deref, mutation, poisoning.
    #[test]
    fn the_wrapper_is_a_mutex() {
        let (lock, _hub, _rung) = lock_with_ring();
        {
            let mut g = lock.lock().unwrap();
            *g += 41;
            assert!(matches!(lock.try_lock(), Err(TryLockError::WouldBlock)));
        }
        assert_eq!(*lock.lock().unwrap(), 41);
        let lock = Arc::new(lock);
        let l2 = Arc::clone(&lock);
        let _ = std::thread::spawn(move || {
            let _g = l2.lock().unwrap();
            panic!("poison it");
        })
        .join();
        assert!(lock.lock().is_err() && matches!(lock.try_lock(), Err(TryLockError::Poisoned(_))));
        assert_eq!(
            *lock.lock().unwrap_or_else(PoisonError::into_inner),
            41,
            "the poisoned guard still derefs"
        );
    }

    /// Real threads: a loser that fails to get the lock either acquires it, is told by the epoch to retry, or is RUNG by the holder: it is never
    /// left registered and unrung.
    #[test]
    fn real_threads_never_strand_a_registered_loser() {
        let rung = Arc::new(AtomicU64::new(0));
        let r2 = Arc::clone(&rung);
        let hub = Arc::new(HandoffHub::new(move |_| {
            r2.fetch_add(1, Ordering::SeqCst);
        }));
        let lock = Arc::new(PtLock::new(0u32, Arc::clone(&hub)));
        for i in 0..3000u32 {
            rung.store(0, Ordering::SeqCst);
            let g = lock.lock().unwrap();
            let seen = hub.prepare();
            let (l, h) = (Arc::clone(&lock), Arc::clone(&hub));
            let loser = std::thread::spawn(move || {
                if i % 3 == 0 {
                    std::thread::yield_now();
                }
                match l.try_lock() {
                    Ok(_g) => "acquired",
                    Err(_) if h.lost(1, seen) => "retry",
                    Err(_) => "registered",
                }
            });
            if i % 5 == 0 {
                std::thread::yield_now();
            }
            drop(g);
            let outcome = loser.join().unwrap();
            if outcome == "registered" {
                // registered before the holder's look: the holder rang it; (the join happens after the drop, so the ring is done)
                assert!(
                    rung.load(Ordering::SeqCst) >= 1,
                    "iteration {i}: registered and never rung"
                );
            }
            // a stale registration (the loser retried or acquired) is harmless: drain it
            hub.released();
        }
    }
}
