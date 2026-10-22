#![allow(dead_code)]
mod atomic {
    use std::cell::RefCell;
    pub use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::atomic::{AtomicU64 as Real, AtomicUsize};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    pub static TAIL: AtomicUsize = AtomicUsize::new(usize::MAX);
    thread_local! {
        pub static AFTER_TAIL: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
    }
    pub struct AtomicU64 {
        value: Real,
        id: usize,
    }
    impl AtomicU64 {
        pub fn new(value: u64) -> Self {
            Self {
                value: Real::new(value),
                id: NEXT.fetch_add(1, Ordering::SeqCst),
            }
        }
        pub fn load(&self, order: Ordering) -> u64 {
            let snapshot = self.value.load(order);
            if self.id == TAIL.load(Ordering::SeqCst) {
                let hook = AFTER_TAIL.with(|h| h.borrow_mut().take());
                if let Some(hook) = hook {
                    hook();
                }
            }
            snapshot
        }
        pub fn store(&self, value: u64, order: Ordering) {
            self.value.store(value, order);
        }
        pub fn fetch_max(&self, value: u64, order: Ordering) -> u64 {
            self.value.fetch_max(value, order)
        }
        pub fn compare_exchange_weak(
            &self,
            current: u64,
            new: u64,
            success: Ordering,
            failure: Ordering,
        ) -> Result<u64, u64> {
            self.value
                .compare_exchange_weak(current, new, success, failure)
        }
    }
}
fn main() {
    use ring::{PrivRing, Push, RegWrite};
    use std::sync::Arc;
    let n: u64 = std::env::args().nth(1).unwrap().parse().unwrap();
    let ring = Arc::new(PrivRing::new());
    atomic::TAIL.store(ring::CAPACITY * 2 + 1, atomic::Ordering::SeqCst);
    let nested = Arc::clone(&ring);
    atomic::AFTER_TAIL.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            for seq in 0..n {
                assert_eq!(nested.push(RegWrite::default()), Push::Queued(seq));
                assert_eq!(nested.peek().unwrap().0, seq);
                nested.commit();
            }
        }))
    });
    // The outer producer resumes with its old tail after another producer and
    // the real consumer have completely pushed/consumed n entries.
    assert_eq!(ring.push(RegWrite::default()), Push::Queued(n));
    assert!(!ring.is_poisoned());
    assert_eq!(ring.peek().unwrap().0, n);
    ring.commit();
    assert_eq!(ring.occupancy(), 0);
}
