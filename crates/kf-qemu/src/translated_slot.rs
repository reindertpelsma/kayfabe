//! Translated slot exclusion, isolated from the GPU adapter for GPU-free tests.
//!
//! This first extraction preserves the existing mutex behavior, including the
//! contended worker return and the drainer's blocking metadata/edit accesses.

use std::sync::{LockResult, Mutex, MutexGuard, TryLockResult};

#[derive(Clone, Copy)]
pub(crate) struct Identity {
    pub(crate) idx: u32,
    pub(crate) tsg: Option<u32>,
}

pub(crate) struct Cell<T> {
    inner: Mutex<T>,
    identity: Identity,
    _ring: Box<dyn Fn(u32) + Send + Sync>,
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
            _ring: Box::new(ring),
        }
    }

    pub(crate) fn lock(&self) -> LockResult<MutexGuard<'_, T>> {
        self.inner.lock()
    }

    pub(crate) fn try_lock(&self) -> TryLockResult<MutexGuard<'_, T>> {
        self.inner.try_lock()
    }

    pub(crate) fn enter(&self) -> TryLockResult<MutexGuard<'_, T>> {
        self.inner.try_lock()
    }

    pub(crate) fn identity(&self) -> Option<Identity> {
        self.inner.lock().ok().map(|_g| self.identity)
    }

    pub(crate) fn edit<R>(&self, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        self.inner.lock().ok().map(|mut g| f(&mut *g))
    }
}

#[cfg(test)]
#[path = "translated_slot_tests.rs"]
mod tests;
