//! Worker-owned store views for immutable USER command snapshots.
//!
//! Prepare/release run outside placement, RAM, graph and channel metadata guards.
//! Copy reads only the completely prepared views: no native verb or allocation.
//! A release failure retains the view/cookie and blocks replacement. The owner
//! must keep this cache on cleanup error, until explicit retry or session close.
//! No runtime channel owns this adapter yet.

use crate::mem::userread::{Refusal, Store};
use kf_chan::tmode::Span;
use kf_host::{HostRm, MapNode, ViewAccess};
use kf_linux_raw::{Backing, CachePolicy, CharDevice, HostOffset, HostPageSize, VolatileRegion};

const BYTES: u64 = 64 << 10;
const VIEWS: usize = 8;
const SPANS: usize = 64;

/// An arm can fail after the RM mapping exists. Never discard that ownership.
struct ArmError<V> {
    retained: Option<V>,
}

trait Host {
    type View;
    fn arm(&self, off: u64, len: u64) -> Result<Self::View, ArmError<Self::View>>;
    fn release(&self, view: &mut Self::View) -> Result<(), Refusal>;
    fn copy(&self, view: &Self::View, off: u64, bytes: &mut [u8]) -> Result<(), Refusal>;
}

struct View<V> {
    off: u64,
    len: u64,
    ready: bool,
    native: V,
}

/// No automatic cleanup on Drop: explicit release errors require retention by
/// the channel/session owner, just like private USER storage. This type performs
/// no GPU map and can only expose non-secret bytes below the guest store bound.
pub(crate) struct Cache<H: Host = Native> {
    host: H,
    bound: u64,
    views: Vec<View<H::View>>,
}

fn required(bound: u64, spans: &[Span]) -> Result<Vec<(u64, u64)>, Refusal> {
    if spans.len() > SPANS {
        return Err(Refusal::Bounds);
    }
    let mut result = Vec::new();
    result
        .try_reserve_exact(VIEWS)
        .map_err(|_| Refusal::Allocation)?;
    let mut total = 0u64;
    for span in spans {
        total = total.checked_add(span.len).ok_or(Refusal::Bounds)?;
        if total > crate::mem::userread::MAX_BYTES as u64 {
            return Err(Refusal::Bounds);
        }
        if span.ram {
            continue;
        }
        let end = span
            .off
            .checked_add(span.len)
            .filter(|e| *e <= bound)
            .ok_or(Refusal::Bounds)?;
        let mut at = span.off;
        while at < end {
            let off = at & !(BYTES - 1);
            let len = BYTES.min(bound.checked_sub(off).ok_or(Refusal::Bounds)?);
            if !result.contains(&(off, len)) {
                if result.len() == VIEWS {
                    return Err(Refusal::Bounds);
                }
                result.push((off, len));
            }
            at = off.checked_add(len).ok_or(Refusal::Bounds)?;
        }
    }
    Ok(result)
}

impl<H: Host> Cache<H> {
    fn create(host: H, bound: u64) -> Result<Self, Refusal> {
        let mut views = Vec::new();
        views
            .try_reserve_exact(VIEWS)
            .map_err(|_| Refusal::Allocation)?;
        Ok(Self { host, bound, views })
    }

    /// Bounded worker-only cleanup. On error Self still owns every unreleased
    /// native view. Never retry this under a guest-reachable metadata guard.
    #[allow(dead_code)]
    pub(crate) fn release(&mut self) -> Result<(), Refusal> {
        while let Some(view) = self.views.last_mut() {
            // release can revoke the CPU mapping before an RM refusal, so the
            // view must cease to be readable even if cleanup remains pending.
            view.ready = false;
            self.host.release(&mut view.native)?;
            self.views.pop();
        }
        Ok(())
    }
}

impl<H: Host> Store for Cache<H> {
    fn prepare(&mut self, spans: &[Span]) -> Result<(), Refusal> {
        // Validate the whole request before any native mutation or eviction.
        let required = required(self.bound, spans)?;
        let mut i = 0;
        while i < self.views.len() {
            let view = self.views.get_mut(i).ok_or(Refusal::Store)?;
            if view.ready && required.contains(&(view.off, view.len)) {
                i += 1;
                continue;
            }
            view.ready = false;
            self.host.release(&mut view.native)?;
            self.views.remove(i);
        }
        for (off, len) in required {
            if self
                .views
                .iter()
                .any(|v| v.ready && v.off == off && v.len == len)
            {
                continue;
            }
            if self.views.len() >= VIEWS {
                return Err(Refusal::Bounds);
            }
            match self.host.arm(off, len) {
                Ok(native) => self.views.push(View {
                    off,
                    len,
                    ready: true,
                    native,
                }),
                Err(error) => {
                    if let Some(native) = error.retained {
                        self.views.push(View {
                            off,
                            len,
                            ready: false,
                            native,
                        });
                    }
                    return Err(Refusal::Store);
                }
            }
        }
        Ok(())
    }

    fn copy(&self, off: u64, bytes: &mut [u8]) -> Result<(), Refusal> {
        if bytes.len() > crate::mem::userread::MAX_BYTES {
            return Err(Refusal::Bounds);
        }
        let end = off
            .checked_add(bytes.len() as u64)
            .filter(|e| *e <= self.bound)
            .ok_or(Refusal::Bounds)?;
        // Complete preflight prevents partial output for a missing cached view.
        let mut at = off;
        while at < end {
            let view = self
                .views
                .iter()
                .find(|v| v.ready && at >= v.off && at < v.off + v.len)
                .ok_or(Refusal::Store)?;
            at = (view.off + view.len).min(end);
        }
        let mut done = 0usize;
        while done < bytes.len() {
            let at = off.checked_add(done as u64).ok_or(Refusal::Bounds)?;
            let view = self
                .views
                .iter()
                .find(|v| v.ready && at >= v.off && at < v.off + v.len)
                .ok_or(Refusal::Store)?;
            let n = usize::try_from((view.off + view.len - at).min(end - at))
                .map_err(|_| Refusal::Bounds)?;
            let dest = bytes.get_mut(done..done + n).ok_or(Refusal::Bounds)?;
            self.host.copy(&view.native, at - view.off, dest)?;
            done += n;
        }
        Ok(())
    }
}

pub(crate) struct Native {
    rm: &'static HostRm,
    store: u32,
}

struct NativeView {
    cpu: Option<VolatileRegion>,
    _node: CharDevice,
    cookie: Option<u64>,
}

impl Cache<Native> {
    #[allow(dead_code)] // Runtime USER source adapter is the next caller.
    pub(crate) fn new(rm: &'static HostRm, store: u32, bound: u64) -> Result<Self, Refusal> {
        Self::create(Native { rm, store }, bound)
    }
}

impl Host for Native {
    type View = NativeView;
    fn arm(&self, off: u64, len: u64) -> Result<NativeView, ArmError<NativeView>> {
        let (node, cookie) = self
            .rm
            .arm_cpu_view(MapNode::Gpu, self.store, off, len, ViewAccess::ReadOnly)
            .map_err(|_| ArmError { retained: None })?;
        match VolatileRegion::map_read_only(
            Backing::DeviceFile { fd: node.as_fd() },
            len,
            CachePolicy::Uncached,
            HostPageSize::query(),
        ) {
            Ok(cpu) => Ok(NativeView {
                cpu: Some(cpu),
                _node: node,
                cookie: Some(cookie),
            }),
            Err(_) => Err(ArmError {
                retained: Some(NativeView {
                    cpu: None,
                    _node: node,
                    cookie: Some(cookie),
                }),
            }),
        }
    }
    fn release(&self, view: &mut NativeView) -> Result<(), Refusal> {
        view.cpu.take();
        if let Some(cookie) = view.cookie {
            self.rm
                .release_cpu_view(kf_host::CpuViewRelease {
                    h_memory: self.store,
                    p_linear_address: cookie,
                })
                .map_err(|_| Refusal::Store)?;
            view.cookie = None;
        }
        Ok(())
    }
    fn copy(&self, view: &NativeView, off: u64, bytes: &mut [u8]) -> Result<(), Refusal> {
        view.cpu
            .as_ref()
            .ok_or(Refusal::Store)?
            .copy_out(HostOffset::new(off), bytes)
            .map_err(|_| Refusal::Store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    #[derive(Default)]
    struct Fake {
        events: RefCell<Vec<(&'static str, u64)>>,
        fail_release: Cell<bool>,
        partial_arm: Cell<bool>,
    }
    impl Host for Fake {
        type View = u64;
        fn arm(&self, off: u64, _: u64) -> Result<u64, ArmError<u64>> {
            self.events.borrow_mut().push(("arm", off));
            if self.partial_arm.replace(false) {
                Err(ArmError {
                    retained: Some(off),
                })
            } else {
                Ok(off)
            }
        }
        fn release(&self, view: &mut u64) -> Result<(), Refusal> {
            self.events.borrow_mut().push(("release", *view));
            if self.fail_release.get() {
                Err(Refusal::Store)
            } else {
                Ok(())
            }
        }
        fn copy(&self, view: &u64, _: u64, bytes: &mut [u8]) -> Result<(), Refusal> {
            self.events.borrow_mut().push(("copy", *view));
            bytes.fill((view / BYTES) as u8);
            Ok(())
        }
    }
    fn span(off: u64, len: u64) -> Span {
        Span {
            ram: false,
            off,
            len,
            perm: kf_host::MapPerm::READ_WRITE,
        }
    }
    #[test]
    fn cross_view_copy_is_prepared_and_missing_span_has_no_side_effect() {
        let mut c = Cache::create(Fake::default(), BYTES * 16).unwrap();
        c.prepare(&[span(BYTES - 4, 8)]).unwrap();
        c.host.events.borrow_mut().clear();
        let mut out = [9; 8];
        c.copy(BYTES - 4, &mut out).unwrap();
        assert_eq!(out, [0, 0, 0, 0, 1, 1, 1, 1]);
        assert_eq!(*c.host.events.borrow(), [("copy", 0), ("copy", BYTES)]);
        c.host.events.borrow_mut().clear();
        let mut out = [9; 8];
        assert_eq!(c.copy(BYTES * 2 - 4, &mut out), Err(Refusal::Store));
        assert_eq!(out, [9; 8]);
        assert!(c.host.events.borrow().is_empty());
    }
    #[test]
    fn complete_budget_and_store_bounds_are_checked_before_host_actions() {
        let mut c = Cache::create(Fake::default(), BYTES * 16).unwrap();
        let spans: Vec<_> = (0..9).map(|i| span(i * BYTES, 4)).collect();
        assert_eq!(c.prepare(&spans), Err(Refusal::Bounds));
        for s in [
            span(u64::MAX, 4),
            span(BYTES * 16 - 1, 2),
            span(0, BYTES + 1),
        ] {
            assert_eq!(c.prepare(&[s]), Err(Refusal::Bounds));
        }
        assert!(c.host.events.borrow().is_empty());
        c.prepare(&[span(0, 4), span(0, 4)]).unwrap();
        assert_eq!(c.views.len(), 1);
    }
    #[test]
    fn failed_release_keeps_ownership_and_never_reuses_revoked_cpu_view() {
        let mut c = Cache::create(Fake::default(), BYTES * 16).unwrap();
        c.prepare(&[span(0, 4)]).unwrap();
        c.host.fail_release.set(true);
        assert_eq!(c.prepare(&[span(BYTES, 4)]), Err(Refusal::Store));
        assert_eq!(c.views.len(), 1);
        assert_eq!(c.views[0].native, 0);
        assert_eq!(c.copy(0, &mut [0; 4]), Err(Refusal::Store));
        assert!(!c.host.events.borrow().contains(&("arm", BYTES)));
        c.host.fail_release.set(false);
        c.prepare(&[span(BYTES, 4)]).unwrap();
        assert_eq!(c.views.len(), 1);
        assert_eq!(c.views[0].native, BYTES);
        c.release().unwrap();
        assert!(c.views.is_empty());
    }
    #[test]
    fn mmap_failure_preserves_native_mapping_until_confirmed_cleanup() {
        let mut c = Cache::create(Fake::default(), BYTES * 16).unwrap();
        c.host.partial_arm.set(true);
        assert_eq!(c.prepare(&[span(0, 4)]), Err(Refusal::Store));
        assert_eq!(c.views.len(), 1);
        assert!(!c.views[0].ready);
        c.host.fail_release.set(true);
        assert_eq!(c.release(), Err(Refusal::Store));
        assert_eq!(c.views.len(), 1);
        c.host.fail_release.set(false);
        c.release().unwrap();
        assert!(c.views.is_empty());
    }
}
