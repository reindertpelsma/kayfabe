//! Channel adapters request maintenance without calling native verbs under their guards.
use crate::{
    mem::userread::{Refusal, Store},
    storeview_unsafe::Slot,
};
use kf_chan::tmode::Span;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(crate) trait DeferredStore: Store + Send + Sync {
    fn release(&mut self) -> Result<(), Refusal>;
    fn live(&self) -> u64;
    fn temporary(&self) -> bool {
        false
    }
}
struct State<S> {
    store: S,
    failure: Option<Refusal>,
}
pub(crate) struct Deferred<S> {
    state: Slot<State<S>>,
    request: Slot<Span>,
    retired: AtomicBool,
}
impl<S: DeferredStore> Deferred<S> {
    pub(crate) fn new(store: S) -> Arc<Self> {
        let state = Slot::new();
        *state.claim_empty().expect("new slot").get_mut() = Some(State {
            store,
            failure: None,
        });
        Arc::new(Self {
            state,
            request: Slot::new(),
            retired: AtomicBool::new(false),
        })
    }
    fn request(&self, off: u64, len: usize) {
        if let Some(mut q) = self.request.claim_empty() {
            *q.get_mut() = Some(Span {
                ram: false,
                off,
                len: len as u64,
                perm: kf_host::MapPerm::READ_WRITE,
            });
        }
    }
    /// Mapping access only. A miss records one bounded preparation request.
    pub(crate) fn read(&self, off: u64, out: &mut [u8]) -> Result<(), Refusal> {
        if self.retired.load(Ordering::Acquire) {
            return Err(Refusal::Store);
        }
        let result = if let Some(pin) = self.state.pin() {
            let state = pin.get();
            let result = match state.failure {
                Some(e) => Err(e),
                None => state.store.copy(off, out),
            };
            if result.is_ok() && state.store.temporary() {
                // Empty preparation releases temporary fallback views only.
                // It runs after the caller's guard, rather than keeping a new
                // channel-owned persistent cache after an RM refusal.
                self.request(0, 0);
            }
            result
        } else {
            Err(Refusal::Busy)
        };
        if matches!(result, Err(Refusal::Busy | Refusal::Changed)) {
            self.request(off, out.len());
        }
        result
    }
    pub(crate) fn live(&self) -> u64 {
        self.state.pin().map_or(0, |p| p.get().store.live())
    }
    /// Worker/act maintenance entry. Caller must have dropped ALL channel guards.
    pub(crate) fn maintain(&self) {
        let request = self
            .request
            .claim_idle()
            .and_then(|mut q| q.get_mut().take());
        let Some(span) = request else {
            return;
        };
        if self.retired.load(Ordering::Acquire) {
            return;
        }
        let Some(mut claim) = self.state.claim_idle() else {
            self.request(span.off, span.len as usize);
            return;
        };
        let state = claim.get_mut().as_mut().expect("initialized store");
        if self.retired.load(Ordering::Acquire) {
            let _ = state.store.release();
            return;
        }
        let spans = if span.len == 0 {
            &[][..]
        } else {
            std::slice::from_ref(&span)
        };
        if let Err(e) = state.store.prepare(spans) {
            if !matches!(e, Refusal::Busy | Refusal::Changed) {
                state.failure = Some(e);
            }
        }
        // A concurrent channel free cannot acquire this slot. Finish its cleanup
        // here, outside its guard, before publishing the store again.
        if self.retired.load(Ordering::Acquire) {
            let _ = state.store.release();
        }
    }
    /// Outside ALL caller guards. An ongoing maintainer observes retired and cleans up.
    pub(crate) fn retire(&self) {
        self.retired.store(true, Ordering::Release);
        if let Some(mut claim) = self.state.claim_idle() {
            if let Some(state) = claim.get_mut() {
                if let Err(e) = state.store.release() {
                    state.failure = Some(e);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    struct Fake {
        guarded: Arc<AtomicBool>,
        calls: Arc<AtomicUsize>,
        ready: bool,
        fail: bool,
        pause: Option<Arc<std::sync::Barrier>>,
        releases: Arc<AtomicUsize>,
        temporary: bool,
    }
    impl Store for Fake {
        fn prepare(&mut self, spans: &[Span]) -> Result<(), Refusal> {
            assert!(
                !self.guarded.load(Ordering::Relaxed),
                "native arm under caller guard"
            );
            if spans.is_empty() {
                self.ready = false;
                self.releases.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
            self.calls.fetch_add(1, Ordering::Relaxed);
            if let Some(pause) = &self.pause {
                pause.wait();
                pause.wait();
            }
            if self.fail {
                return Err(Refusal::Store);
            }
            self.ready = true;
            Ok(())
        }
        fn copy(&self, off: u64, out: &mut [u8]) -> Result<(), Refusal> {
            if off.checked_add(out.len() as u64).is_none_or(|e| e > 4096) {
                return Err(Refusal::Bounds);
            }
            if !self.ready {
                return Err(Refusal::Busy);
            }
            out.fill(42);
            Ok(())
        }
    }
    impl DeferredStore for Fake {
        fn release(&mut self) -> Result<(), Refusal> {
            assert!(
                !self.guarded.load(Ordering::Relaxed),
                "native release under caller guard"
            );
            self.ready = false;
            self.releases.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        fn live(&self) -> u64 {
            u64::from(self.ready)
        }
        fn temporary(&self) -> bool {
            self.temporary && self.ready
        }
    }
    #[test]
    fn guarded_miss_only_requests_and_maintenance_runs_after_guard() {
        let guarded = Arc::new(AtomicBool::new(true));
        let calls = Arc::new(AtomicUsize::new(0));
        let d = Deferred::new(Fake {
            guarded: guarded.clone(),
            calls: calls.clone(),
            ready: false,
            fail: false,
            pause: None,
            releases: Arc::new(AtomicUsize::new(0)),
            temporary: false,
        });
        let mut out = [9; 8];
        assert_eq!(d.read(0, &mut out), Err(Refusal::Busy));
        assert_eq!(out, [9; 8]);
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        guarded.store(false, Ordering::Relaxed);
        d.maintain();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(d.read(0, &mut out), Ok(()));
        assert_eq!(out, [42; 8]);
        let pin = d.state.pin().unwrap();
        d.retire();
        assert_eq!(d.read(0, &mut out), Err(Refusal::Store));
        drop(pin);
        d.retire();
        assert_eq!(d.live(), 0);
    }
    #[test]
    fn arm_refusal_is_terminal_and_busy_maintainer_requeues() {
        let d = Deferred::new(Fake {
            guarded: Arc::new(AtomicBool::new(false)),
            calls: Arc::new(AtomicUsize::new(0)),
            ready: false,
            fail: true,
            pause: None,
            releases: Arc::new(AtomicUsize::new(0)),
            temporary: false,
        });
        assert_eq!(d.read(0, &mut [0; 8]), Err(Refusal::Busy));
        let pin = d.state.pin().unwrap();
        d.maintain();
        drop(pin);
        d.maintain();
        assert_eq!(d.read(0, &mut [0; 8]), Err(Refusal::Store));
        d.retire();
    }
    #[test]
    fn retirement_during_arm_never_waits_and_maintainer_releases() {
        let pause = Arc::new(std::sync::Barrier::new(2));
        let releases = Arc::new(AtomicUsize::new(0));
        let d = Deferred::new(Fake {
            guarded: Arc::new(AtomicBool::new(false)),
            calls: Arc::new(AtomicUsize::new(0)),
            ready: false,
            fail: false,
            pause: Some(pause.clone()),
            releases: releases.clone(),
            temporary: false,
        });
        assert_eq!(d.read(0, &mut [0; 8]), Err(Refusal::Busy));
        let worker = d.clone();
        let join = std::thread::spawn(move || worker.maintain());
        pause.wait();
        d.retire();
        assert_eq!(d.read(0, &mut [0; 8]), Err(Refusal::Store));
        pause.wait();
        join.join().unwrap();
        assert_eq!(releases.load(Ordering::Relaxed), 1);
        assert_eq!(d.live(), 0);
    }
    #[test]
    fn successful_fallback_requests_release_outside_the_read() {
        let guarded = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let releases = Arc::new(AtomicUsize::new(0));
        let d = Deferred::new(Fake {
            guarded: guarded.clone(),
            calls: calls.clone(),
            releases: releases.clone(),
            ready: false,
            fail: false,
            pause: None,
            temporary: true,
        });
        assert_eq!(d.read(0, &mut [0; 8]), Err(Refusal::Busy));
        d.maintain();
        guarded.store(true, Ordering::Relaxed);
        assert_eq!(d.read(0, &mut [0; 8]), Ok(()));
        assert_eq!(releases.load(Ordering::Relaxed), 0);
        guarded.store(false, Ordering::Relaxed);
        d.maintain();
        assert_eq!(releases.load(Ordering::Relaxed), 1);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(d.live(), 0);
        d.retire();
    }
}
