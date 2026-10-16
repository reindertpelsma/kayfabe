//! Shared physical-store view prototype. No guest VA or host pointer is accepted.
//! Reads never arm, reclaim, allocate or wait; maintenance runs outside all caller guards.
use crate::storeview_unsafe::Slot;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

pub(crate) const WINDOW: u64 = 64 << 10;
pub(crate) const ACCESS: usize = 256;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Error {
    Bounds,
    Retry,
    Arm,
    Release,
    Read,
    Write,
    Allocation,
}
/// Partial arms retain ownership, including an RM map whose CPU mmap failed.
pub(crate) struct ArmError<V> {
    pub(crate) retained: Option<V>,
}
/// Worker-only native operations. Read/write must be bounded mapping access only.
pub(crate) trait Backend: Send + Sync {
    type View: Send + Sync;
    fn arm(&self, off: u64, len: u64) -> Result<Self::View, ArmError<Self::View>>;
    fn release(&self, view: &mut Self::View) -> Result<(), Error>;
    fn read(&self, view: &Self::View, off: u64, out: &mut [u8]) -> Result<(), Error>;
    fn write(&self, view: &mut Self::View, off: u64, bytes: &[u8]) -> Result<(), Error>;
}
pub(crate) struct Owner {
    live: AtomicUsize,
    active: AtomicBool,
}
impl Default for Owner {
    fn default() -> Self {
        Self {
            live: AtomicUsize::new(0),
            active: AtomicBool::new(true),
        }
    }
}
impl Owner {
    pub(crate) fn retire(&self) {
        self.active.store(false, Ordering::Release);
    }
    pub(crate) fn live(&self) -> usize {
        self.live.load(Ordering::Relaxed)
    }
}
struct Charge<'a> {
    total: &'a AtomicUsize,
    owner: &'a Owner,
    committed: bool,
}
impl Drop for Charge<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.total.fetch_sub(1, Ordering::Relaxed);
            self.owner.live.fetch_sub(1, Ordering::Relaxed);
        }
    }
}
struct Entry<V> {
    view: V,
    ready: bool,
    owner: Arc<Owner>,
    hot: AtomicBool,
}
#[derive(Default)]
pub(crate) struct Counters {
    pub(crate) arms: AtomicU64,
    pub(crate) releases: AtomicU64,
    pub(crate) arm_refused: AtomicU64,
    pub(crate) retry_ok: AtomicU64,
    pub(crate) fallback: AtomicU64,
    pub(crate) release_refused: AtomicU64,
    pub(crate) retries: AtomicU64,
}
/// FULL has no slot table; creation is an explicit realize-time operation.
pub(crate) struct Full<B: Backend> {
    backend: B,
    view: Option<B::View>,
    bound: u64,
}
fn range(bound: u64, off: u64, len: usize) -> Result<u64, Error> {
    if len > ACCESS {
        return Err(Error::Bounds);
    }
    off.checked_add(len as u64)
        .filter(|e| *e <= bound)
        .ok_or(Error::Bounds)
}
impl<B: Backend> Full<B> {
    pub(crate) fn new(backend: B, bound: u64) -> Result<Self, (B, ArmError<B::View>)> {
        match backend.arm(0, bound) {
            Ok(view) => Ok(Self {
                backend,
                view: Some(view),
                bound,
            }),
            Err(e) => Err((backend, e)),
        }
    }
    pub(crate) fn read(&self, off: u64, out: &mut [u8]) -> Result<(), Error> {
        range(self.bound, off, out.len())?;
        self.backend
            .read(self.view.as_ref().ok_or(Error::Read)?, off, out)
    }
    pub(crate) fn write(&mut self, off: u64, bytes: &[u8]) -> Result<(), Error> {
        range(self.bound, off, bytes.len())?;
        self.backend
            .write(self.view.as_mut().ok_or(Error::Write)?, off, bytes)
    }
    pub(crate) fn release(&mut self) -> Result<(), Error> {
        if let Some(v) = &mut self.view {
            self.backend.release(v)?;
        }
        self.view = None;
        Ok(())
    }
}
impl<B: Backend> Drop for Full<B> {
    fn drop(&mut self) {
        if self.release().is_err() {
            // A native backend may retain a cookie until session close. Never
            // silently drop it after a refused explicit release.
            if let Some(v) = self.view.take() {
                std::mem::forget(v);
            }
        }
    }
}
/// Dense offset-indexed table, bounded by the store rather than channel quotas.
pub(crate) struct Windowed<B: Backend> {
    backend: B,
    bound: u64,
    slots: Vec<Slot<Entry<B::View>>>,
    budget: usize,
    live: AtomicUsize,
    clock: AtomicUsize,
    pub(crate) counters: Counters,
}
impl<B: Backend> Windowed<B> {
    pub(crate) fn new(backend: B, bound: u64, budget: usize) -> Result<Self, Error> {
        if bound == 0 || budget == 0 {
            return Err(Error::Bounds);
        }
        let n = usize::try_from((bound - 1) / WINDOW + 1).map_err(|_| Error::Bounds)?;
        let mut slots = Vec::new();
        slots.try_reserve_exact(n).map_err(|_| Error::Allocation)?;
        slots.resize_with(n, Slot::new);
        Ok(Self {
            backend,
            bound,
            slots,
            budget,
            live: AtomicUsize::new(0),
            clock: AtomicUsize::new(0),
            counters: Counters::default(),
        })
    }
    fn slot(&self, off: u64) -> Result<&Slot<Entry<B::View>>, Error> {
        if off >= self.bound {
            return Err(Error::Bounds);
        }
        self.slots
            .get(usize::try_from(off / WINDOW).map_err(|_| Error::Bounds)?)
            .ok_or(Error::Bounds)
    }
    pub(crate) fn live(&self) -> usize {
        self.live.load(Ordering::Relaxed)
    }
    /// Both spans are pinned before copying, so a missing second span changes no output.
    pub(crate) fn read(&self, off: u64, out: &mut [u8]) -> Result<(), Error> {
        let end = range(self.bound, off, out.len())?;
        if out.is_empty() {
            return Ok(());
        }
        let a = self.slot(off)?.pin().ok_or(Error::Retry)?;
        let first = a.get();
        if !first.ready {
            return Err(Error::Retry);
        }
        let n = out.len().min((WINDOW - off % WINDOW) as usize);
        let second = if n < out.len() {
            Some(self.slot(end - 1)?.pin().ok_or(Error::Retry)?)
        } else {
            None
        };
        if second.as_ref().is_some_and(|p| !p.get().ready) {
            return Err(Error::Retry);
        }
        first.hot.store(true, Ordering::Relaxed);
        self.backend
            .read(&first.view, off % WINDOW, &mut out[..n])?;
        if let Some(b) = second {
            b.get().hot.store(true, Ordering::Relaxed);
            self.backend.read(&b.get().view, 0, &mut out[n..])?;
        }
        Ok(())
    }
    pub(crate) fn write(&self, off: u64, bytes: &[u8]) -> Result<(), Error> {
        let end = range(self.bound, off, bytes.len())?;
        if bytes.is_empty() {
            return Ok(());
        }
        // CPU writers exclude readers as well as other writers. A backend's
        // plain data copy must never race another host CPU access.
        let mut a = self.slot(off)?.claim_idle().ok_or(Error::Retry)?;
        let first = a.get_mut().as_mut().ok_or(Error::Write)?;
        if !first.ready {
            return Err(Error::Retry);
        }
        let n = bytes.len().min((WINDOW - off % WINDOW) as usize);
        let mut second = if n < bytes.len() {
            Some(self.slot(end - 1)?.claim_idle().ok_or(Error::Retry)?)
        } else {
            None
        };
        if second
            .as_mut()
            .is_some_and(|p| p.get_mut().as_ref().is_none_or(|e| !e.ready))
        {
            return Err(Error::Retry);
        }
        first.hot.store(true, Ordering::Relaxed);
        self.backend
            .write(&mut first.view, off % WINDOW, &bytes[..n])?;
        if let Some(mut b) = second {
            self.backend.write(
                &mut b.get_mut().as_mut().ok_or(Error::Write)?.view,
                0,
                &bytes[n..],
            )?;
        }
        Ok(())
    }
    /// One bounded clock pass. No occupied reader is waited for. A requester
    /// can reclaim its own views or surplus from a more represented owner.
    pub(crate) fn evict(&self, requester: &Arc<Owner>) -> Result<bool, Error> {
        let start = self.clock.fetch_add(1, Ordering::Relaxed);
        for step in 0..self.slots.len() {
            let idx = start.wrapping_add(step) % self.slots.len();
            let Some(mut claim) = self.slots[idx].claim_idle() else {
                continue;
            };
            let e = claim.get_mut().as_mut().ok_or(Error::Release)?;
            let own = Arc::ptr_eq(&e.owner, requester) || !e.owner.active.load(Ordering::Acquire);
            if !own && e.owner.live() <= requester.live().max(1) {
                continue;
            }
            if e.ready && e.hot.swap(false, Ordering::Relaxed) {
                continue;
            }
            // Reserve a victim owner's surplus atomically. Independent evictors
            // must not both spend the same final surplus and remove its last view.
            let surplus_reserved = if own {
                false
            } else {
                let count = e.owner.live();
                if count <= requester.live().max(1)
                    || e.owner
                        .live
                        .compare_exchange(count, count - 1, Ordering::Relaxed, Ordering::Relaxed)
                        .is_err()
                {
                    continue;
                }
                true
            };
            // Revocation before release means a retained failed cleanup can
            // never be read even if mmap was removed before RM_UNMAP refused.
            e.ready = false;
            if self.backend.release(&mut e.view).is_err() {
                if surplus_reserved {
                    e.owner.live.fetch_add(1, Ordering::Relaxed);
                }
                self.counters
                    .release_refused
                    .fetch_add(1, Ordering::Relaxed);
                return Err(Error::Release);
            }
            if !surplus_reserved {
                e.owner.live.fetch_sub(1, Ordering::Relaxed);
            }
            claim.get_mut().take();
            self.live.fetch_sub(1, Ordering::Relaxed);
            self.counters.releases.fetch_add(1, Ordering::Relaxed);
            self.clock.store(idx.wrapping_add(1), Ordering::Relaxed);
            return Ok(true);
        }
        Ok(false)
    }
    /// Maintenance only: do not call under placement/topology/metadata guards.
    /// A read hit is not disturbed. An arming contender immediately retries.
    pub(crate) fn prepare(&self, owner: &Arc<Owner>, off: u64) -> Result<(), Error> {
        let slot = self.slot(off)?;
        if let Some(p) = slot.pin() {
            if p.get().ready {
                return Ok(());
            }
            drop(p);
            let mut cleanup = slot.claim_idle().ok_or(Error::Retry)?;
            let e = cleanup.get_mut().as_mut().ok_or(Error::Release)?;
            if e.ready {
                return Err(Error::Retry);
            }
            self.backend
                .release(&mut e.view)
                .map_err(|_| Error::Release)?;
            e.owner.live.fetch_sub(1, Ordering::Relaxed);
            cleanup.get_mut().take();
            self.live.fetch_sub(1, Ordering::Relaxed);
            self.counters.releases.fetch_add(1, Ordering::Relaxed);
        }
        let Some(mut claim) = slot.claim_empty() else {
            return Err(Error::Retry);
        };
        let old = self.live.load(Ordering::Relaxed);
        if old >= self.budget {
            drop(claim);
            // Second-chance pass may only clear hot bits; the next turn tries again.
            self.evict(owner)?;
            return Err(Error::Retry);
        }
        if self
            .live
            .compare_exchange(old, old + 1, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            return Err(Error::Retry);
        }
        owner.live.fetch_add(1, Ordering::Relaxed);
        let mut charge = Charge {
            total: &self.live,
            owner,
            committed: false,
        };
        let base = off / WINDOW * WINDOW;
        let len = WINDOW.min(self.bound - base);
        let attempt = self.backend.arm(base, len);
        let attempt = match attempt {
            Err(ArmError { retained: None }) => {
                self.counters.arm_refused.fetch_add(1, Ordering::Relaxed);
                // No other guard is held, only ownership of our unreadable slot.
                let _ = self.evict(owner)?;
                let retry = self.backend.arm(base, len);
                if retry.is_ok() {
                    self.counters.retry_ok.fetch_add(1, Ordering::Relaxed);
                }
                retry
            }
            other => other,
        };
        let (view, ready) = match attempt {
            Ok(v) => (v, true),
            Err(ArmError { retained: Some(v) }) => (v, false),
            Err(ArmError { retained: None }) => {
                return Err(Error::Arm);
            }
        };
        self.counters.arms.fetch_add(1, Ordering::Relaxed);
        charge.committed = true;
        *claim.get_mut() = Some(Entry {
            view,
            ready,
            owner: owner.clone(),
            hot: AtomicBool::new(true),
        });
        if ready { Ok(()) } else { Err(Error::Arm) }
    }
    /// Exclusive final cleanup. On error failed native resources stay owned.
    pub(crate) fn release(&mut self) -> Result<(), Error> {
        let mut failed = false;
        for s in &mut self.slots {
            let v = s.get_mut();
            if let Some(e) = v {
                e.ready = false;
                if self.backend.release(&mut e.view).is_err() {
                    failed = true;
                    continue;
                }
                e.owner.live.fetch_sub(1, Ordering::Relaxed);
                self.live.fetch_sub(1, Ordering::Relaxed);
                self.counters.releases.fetch_add(1, Ordering::Relaxed);
                *v = None;
                s.reset_empty();
            }
        }
        if failed { Err(Error::Release) } else { Ok(()) }
    }
}
impl<B: Backend> Drop for Windowed<B> {
    fn drop(&mut self) {
        if self.release().is_err() {
            for s in &mut self.slots {
                if let Some(e) = s.get_mut().take() {
                    std::mem::forget(e);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "storeview_tests.rs"]
mod tests;
