//! Audited slot lifetime perimeter for the shared store view.
//!
//! EMPTY/ARMING/ARMED and the reader count share ONE atomic word. No pointer
//! escapes: a safe caller receives a borrow tied to its RAII pin. Native mapping
//! calls belong to the backend, outside this protocol.
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};

const EMPTY: usize = 0;
const ARMING: usize = 1;
const ARMED: usize = 2;
const READER: usize = 4;

/// A non-waiting, bounded ownership transition. Shared with the Loom model.
pub(crate) fn pin_state(state: usize) -> Option<usize> {
    (state & 3 == ARMED)
        .then(|| state.checked_add(READER))
        .flatten()
}

trait Atomic: Sized {
    fn new(value: usize) -> Self;
    fn load(&self, order: Ordering) -> usize;
    fn compare_exchange(
        &self,
        old: usize,
        new: usize,
        yes: Ordering,
        no: Ordering,
    ) -> Result<usize, usize>;
    fn fetch_sub(&self, value: usize, order: Ordering) -> usize;
    fn store(&self, value: usize, order: Ordering);
}
macro_rules! atomic_impl {
    ($ty:ty) => {
        impl Atomic for $ty {
            fn new(value: usize) -> Self {
                Self::new(value)
            }
            fn load(&self, order: Ordering) -> usize {
                self.load(order)
            }
            fn compare_exchange(
                &self,
                old: usize,
                new: usize,
                yes: Ordering,
                no: Ordering,
            ) -> Result<usize, usize> {
                self.compare_exchange(old, new, yes, no)
            }
            fn fetch_sub(&self, value: usize, order: Ordering) -> usize {
                self.fetch_sub(value, order)
            }
            fn store(&self, value: usize, order: Ordering) {
                self.store(value, order);
            }
        }
    };
}
atomic_impl!(AtomicUsize);
#[cfg(test)]
atomic_impl!(loom::sync::atomic::AtomicUsize);
/// Exactly the same ownership code is instantiated with native and Loom atomics.
struct State<A: Atomic>(A);
impl<A: Atomic> State<A> {
    fn new() -> Self {
        Self(A::new(EMPTY))
    }
    fn pin(&self) -> bool {
        let old = self.0.load(Ordering::Relaxed);
        let Some(next) = pin_state(old) else {
            return false;
        };
        self.0
            .compare_exchange(old, next, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }
    fn claim(&self, from: usize) -> bool {
        self.0
            .compare_exchange(from, ARMING, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }
    fn unpin(&self) {
        self.0.fetch_sub(READER, Ordering::Release);
    }
    fn publish(&self, occupied: bool) {
        self.0
            .store(if occupied { ARMED } else { EMPTY }, Ordering::Release);
    }
}
pub(crate) struct Slot<T> {
    state: State<AtomicUsize>,
    value: UnsafeCell<Option<T>>,
}
// SAFETY: ownership is transferred only by the atomic protocol; T may move to
// another thread only when no borrow exists. All exposed borrows require a pin.
unsafe impl<T: Send> Send for Slot<T> {}
// SAFETY: shared access exposes only &T while pinned; mutation requires the
// exclusive ARMING state and zero readers. T must itself allow shared access.
unsafe impl<T: Send + Sync> Sync for Slot<T> {}

impl<T> Slot<T> {
    pub(crate) fn new() -> Self {
        Self {
            state: State::new(),
            value: UnsafeCell::new(None),
        }
    }
    pub(crate) fn pin(&self) -> Option<Pin<'_, T>> {
        self.state.pin().then(|| Pin { slot: self })
    }
    pub(crate) fn claim_empty(&self) -> Option<Exclusive<'_, T>> {
        self.claim(EMPTY)
    }
    pub(crate) fn claim_idle(&self) -> Option<Exclusive<'_, T>> {
        self.claim(ARMED)
    }
    fn claim(&self, from: usize) -> Option<Exclusive<'_, T>> {
        self.state.claim(from).then(|| Exclusive { slot: self })
    }
    pub(crate) fn get_mut(&mut self) -> &mut Option<T> {
        self.value.get_mut()
    }
    pub(crate) fn reset_empty(&mut self) {
        assert!(self.value.get_mut().is_none());
        self.state.publish(false);
    }
}

pub(crate) struct Pin<'a, T> {
    slot: &'a Slot<T>,
}
impl<T> Pin<'_, T> {
    pub(crate) fn get(&self) -> &T {
        // SAFETY: construction atomically increments the ARMED reader count.
        // Neither EMPTY nor ARMING can be acquired until this pin is dropped.
        // ARMED publication always holds Some; the lifetime is bounded by &self.
        unsafe {
            (&*self.slot.value.get())
                .as_ref()
                .expect("ARMED slot holds a value")
        }
    }
}
impl<T> Drop for Pin<'_, T> {
    fn drop(&mut self) {
        self.slot.state.unpin();
    }
}

pub(crate) struct Exclusive<'a, T> {
    slot: &'a Slot<T>,
}
impl<T> Exclusive<'_, T> {
    pub(crate) fn get_mut(&mut self) -> &mut Option<T> {
        // SAFETY: ARMING was acquired only from EMPTY or ARMED with zero readers.
        // One claimant exists; readers cannot enter ARMING. Borrow cannot escape
        // this exclusive guard, and publishing happens only when it drops.
        unsafe { &mut *self.slot.value.get() }
    }
}
impl<T> Drop for Exclusive<'_, T> {
    fn drop(&mut self) {
        // SAFETY: this guard exclusively owns ARMING until the Release below.
        let occupied = unsafe { (&*self.slot.value.get()).is_some() };
        self.slot.state.publish(occupied);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pin_excludes_reclamation_and_unwind_releases_it() {
        let slot = Slot::new();
        *slot.claim_empty().unwrap().get_mut() = Some(42);
        let pin = slot.pin().unwrap();
        assert_eq!(*pin.get(), 42);
        assert!(slot.claim_idle().is_none());
        assert!(slot.claim_empty().is_none());
        drop(pin);
        let mut owner = slot.claim_idle().unwrap();
        assert!(slot.pin().is_none());
        assert_eq!(owner.get_mut().take(), Some(42));
        drop(owner);
        assert!(slot.pin().is_none());
        assert!(slot.claim_empty().is_some());
    }
    #[test]
    fn reader_count_refuses_overflow() {
        assert_eq!(pin_state(usize::MAX - 1), None);
        assert_eq!(pin_state(ARMING), None);
        assert_eq!(pin_state(EMPTY), None);
    }
    #[test]
    fn loom_two_readers_and_eviction_use_actual_state_machine() {
        loom::model(|| {
            struct Model {
                state: State<loom::sync::atomic::AtomicUsize>,
                value: loom::cell::UnsafeCell<usize>,
            }
            let m = loom::sync::Arc::new(Model {
                state: State::new(),
                value: loom::cell::UnsafeCell::new(0),
            });
            assert!(m.state.claim(EMPTY));
            m.value.with_mut(|p| {
                // SAFETY: model starts with exclusive EMPTY -> ARMING ownership.
                unsafe {
                    *p = 7;
                }
            });
            m.state.publish(true);
            let mut jobs = Vec::new();
            for _ in 0..2 {
                let m = m.clone();
                jobs.push(loom::thread::spawn(move || {
                    if m.state.pin() {
                        let p = m.value.get();
                        // SAFETY: real protocol acquired a reader pin; Loom tracks
                        // this access until p is dropped before releasing the pin.
                        assert!([7, 9].contains(unsafe { p.deref() }));
                        loom::thread::yield_now();
                        drop(p);
                        m.state.unpin();
                    }
                }));
            }
            let e = m.clone();
            jobs.push(loom::thread::spawn(move || {
                if e.state.claim(ARMED) {
                    e.value.with_mut(|p| {
                        // SAFETY: actual protocol acquired ARMING with zero readers.
                        unsafe {
                            *p = 9;
                        }
                    });
                    e.state.publish(true);
                }
            }));
            for j in jobs {
                j.join().unwrap();
            }
            assert!(m.state.claim(ARMED)); // no leaked pins on thread exit
        });
    }
}
