//! Worker-owned store views for immutable USER command snapshots.
//!
//! Prepare/release run outside placement, RAM, graph and channel metadata guards.
//! Copy reads only the completely prepared views: no native verb or allocation.
//! A release failure retains the view/cookie and blocks replacement. The owner
//! must keep this cache on cleanup error, until explicit retry or session close.
//! The experimental USER worker retains it until confirmed native cleanup.
//! Its private publisher mutex may exclude Free; no drainer/vCPU waits on it.

use crate::mem::userread::{Refusal, Store};
use kf_chan::tmode::Span;
use kf_host::{HostRm, MapNode, ViewAccess};
use kf_linux_raw::{Backing, CachePolicy, CharDevice, HostOffset, HostPageSize, VolatileRegion};

const BYTES: u64 = 64 << 10;
// Trusted CPU-view quota: at most1MiB per channel, independent of fragmentation.
// A source requiring more disjoint64KiB store windows refuses before host actions.
const VIEWS: usize = 16; // SCRATCH d3d11-gap: was BYTES(storage)/64 KiB = 16 with the 1 MiB storage; kept at 16
const SPANS: usize = crate::mem::userread::MAX_BYTES / 4096 + 1;

// ★ 2026-10-15 (perf campaign, `docs/design/V3_WINDOWS_PERF_20261015.md`): the cache KEEPS its views
// across snapshots. Before, `prepare` released every ready view the NEXT snapshot did not need, so a
// channel that alternates between two 64 KiB store windows (a Windows D3D11 channel alternates its
// pushbuffer windows on almost every submission) armed and released one window per snapshot: ~16 000
// openat + RM_MAP_MEMORY + mmap + munmap + RM_UNMAP_MEMORY + close cycles per second on the two workers
// (measured, Heaven D3D11: 106 s of SYSTEM time per worker per 293 s, 20.9 fps). A view is now released
// only when the channel's own limit is reached (least recently used first) or when RM refuses a new
// mapping. The limit is the operator's (`user-view-keep=<N>`, default [`DEFAULT_KEEP`] = the old
// per-channel quota of 16 views = 1 MiB); nothing is derived from the host's BAR1 size or free space.
// Each view stays owned by ONE cache (one channel, one worker at a time under the channel's own
// lock), so no reader can see a view another thread unmapped, and nothing here locks or blocks.
/// The per-channel number of views kept armed (the old quota: 16 x 64 KiB = 1 MiB).
pub(crate) const DEFAULT_KEEP: usize = VIEWS;
static KEEP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(DEFAULT_KEEP);
/// RM refused a mapping and the channel's own LRU eviction + one retry satisfied it.
pub(crate) static ARM_RETRY_OK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// RM refused twice: the cache fell back to today's behaviour (released every view the read does not
/// need, then armed) and that satisfied it.
pub(crate) static ARM_FALLBACK_OK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// RM refused even after the fallback: the read was refused (`Refusal::Store`), as it always was.
pub(crate) static ARM_FAILED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Views armed (RM map + mmap), all channels, since the process started.
pub(crate) static VIEWS_ARMED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Views released because a channel was at its limit (quota full) - a workload that thrashes shows here.
pub(crate) static VIEWS_EVICTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// One line for the shutdown summary (next to `USER_RING_LOOP SUMMARY`): totals by name.
pub(crate) fn summary() -> String {
    use std::sync::atomic::Ordering::Relaxed;
    format!(
        "USER_STORE_VIEWS keep={} armed={} evicted_at_quota={} rm_refused_retry_ok={} rm_refused_fallback_ok={} rm_refused_read_refused={}",
        KEEP.load(Relaxed),
        VIEWS_ARMED.load(Relaxed),
        VIEWS_EVICTED.load(Relaxed),
        ARM_RETRY_OK.load(Relaxed),
        ARM_FALLBACK_OK.load(Relaxed),
        ARM_FAILED.load(Relaxed)
    )
}

/// Realize: the operator's `user-view-keep` (0 = the default).
pub(crate) fn configure(keep: u32) {
    let k = if keep == 0 { DEFAULT_KEEP } else { (keep as usize).min(256) };
    KEEP.store(k, std::sync::atomic::Ordering::Relaxed);
}

/// A loud, bounded line for the first few events of a kind and then every power of two.
fn note(kind: &str, n: u64, why: &str) {
    if n <= 8 || n.is_power_of_two() {
        eprintln!("kf3: USER_STORE_VIEW {kind} #{n}: {why}");
    }
}

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
    /// The cache's use tick of the last snapshot that required this view (LRU order).
    used: u64,
    native: V,
}

/// No automatic cleanup on Drop: explicit release errors require retention by
/// the channel/session owner, just like private USER storage. This type performs
/// no GPU map and can only expose non-secret bytes below the guest store bound.
pub(crate) struct Cache<H: Host = Native> {
    host: H,
    bound: u64,
    views: Vec<View<H::View>>,
    /// Views this channel keeps armed (the operator's limit, read once at creation).
    keep: usize,
    tick: u64,
    /// This channel's views armed / released at its limit (printed when the channel is released).
    armed: u64,
    evicted: u64,
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
        let keep = KEEP.load(std::sync::atomic::Ordering::Relaxed);
        Ok(Self { host, bound, views, keep, tick: 0, armed: 0, evicted: 0 })
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

    /// `(armed, evicted_at_quota, keep)` of this channel.
    pub(crate) fn stats(&self) -> (u64, u64, usize) {
        (self.armed, self.evicted, self.keep)
    }

    /// Release the least recently used ready view that `required` does not name. A refused release
    /// keeps ownership (the view stays, not ready, and blocks replacement until retried).
    fn evict_lru(&mut self, required: &[(u64, u64)]) -> Result<(), Refusal> {
        let victim = self
            .views
            .iter()
            .enumerate()
            .filter(|(_, v)| !(v.ready && required.contains(&(v.off, v.len))))
            .min_by_key(|(_, v)| v.used)
            .map(|(i, _)| i)
            .ok_or(Refusal::Bounds)?;
        let view = self.views.get_mut(victim).ok_or(Refusal::Store)?;
        view.ready = false;
        self.host.release(&mut view.native)?;
        self.views.remove(victim);
        Ok(())
    }

    /// Today's behaviour, kept as the last resort when RM keeps refusing a mapping: release every
    /// ready view the current read does not need.
    fn evict_all_unrequired(&mut self, required: &[(u64, u64)]) -> Result<(), Refusal> {
        while self.evict_lru(required).is_ok() {}
        Ok(())
    }

    /// One arm attempt. `Ok(true)`: armed and kept. `Ok(false)`: RM refused and nothing exists. `Err`:
    /// a mapping exists but could not be mapped - it is RETAINED (never discarded) and the read is refused.
    fn try_arm(&mut self, off: u64, len: u64) -> Result<bool, Refusal> {
        match self.host.arm(off, len) {
            Ok(native) => {
                self.views.push(View { off, len, ready: true, used: self.tick, native });
                self.armed += 1;
                VIEWS_ARMED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(true)
            }
            Err(error) => match error.retained {
                Some(native) => {
                    self.views.push(View { off, len, ready: false, used: self.tick, native });
                    Err(Refusal::Store)
                }
                None => Ok(false),
            },
        }
    }

    /// Arm one window; on an RM refusal evict this channel's own LRU view and retry once, then fall
    /// back to releasing every view the read does not need and retry once more. Counted and logged
    /// by name.
    fn arm_with_fallback(
        &mut self,
        off: u64,
        len: u64,
        required: &[(u64, u64)],
    ) -> Result<(), Refusal> {
        use std::sync::atomic::Ordering::Relaxed;
        if self.try_arm(off, len)? {
            return Ok(());
        }
        // 1. evict our own least recently used view and retry once
        if self.evict_lru(required).is_ok() && self.try_arm(off, len)? {
            let n = ARM_RETRY_OK.fetch_add(1, Relaxed) + 1;
            note(
                "arm refused by RM; evicted this channel's LRU view and the retry succeeded",
                n,
                &format!("window {off:#x}+{len:#x}, {} views kept", self.views.len()),
            );
            return Ok(());
        }
        // 2. today's behaviour: release everything the read does not need, then arm
        let _ = self.evict_all_unrequired(required);
        if self.try_arm(off, len)? {
            let n = ARM_FALLBACK_OK.fetch_add(1, Relaxed) + 1;
            note("arm refused twice by RM; fell back to arm/release for this read (satisfied)", n, &format!("window {off:#x}+{len:#x}"));
            return Ok(());
        }
        let n = ARM_FAILED.fetch_add(1, Relaxed) + 1;
        note("arm refused by RM even after the fallback; the read is refused", n, &format!("window {off:#x}+{len:#x}"));
        Err(Refusal::Store)
    }
}

impl<H: Host> Store for Cache<H> {
    fn prepare(&mut self, spans: &[Span]) -> Result<(), Refusal> {
        // Validate the whole request before any native mutation or eviction.
        let required = required(self.bound, spans)?;
        self.tick = self.tick.wrapping_add(1);
        // A view whose mapping failed half-way (or whose release failed) is retried first, as before.
        let mut i = 0;
        while i < self.views.len() {
            let view = self.views.get_mut(i).ok_or(Refusal::Store)?;
            if view.ready {
                if required.contains(&(view.off, view.len)) {
                    view.used = self.tick;
                }
                i += 1;
                continue;
            }
            self.host.release(&mut view.native)?;
            self.views.remove(i);
        }
        let limit = self.keep.max(required.len());
        for (off, len) in required.iter().copied() {
            if self
                .views
                .iter()
                .any(|v| v.ready && v.off == off && v.len == len)
            {
                continue;
            }
            if self.views.len() >= limit {
                // at the channel's limit: release the least recently used view this read does not
                // need (one exists: `required` has at most `limit` windows and one is missing)
                self.evict_lru(&required)?;
                self.evicted += 1;
                VIEWS_EVICTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            self.arm_with_fallback(off, len, &required)?;
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
        let cpu = view.cpu.as_ref().ok_or(Refusal::Store)?;
        let t0 = crate::lifecycle::on().then(std::time::Instant::now);
        // EXPERIMENT ONLY (`KF3_BAR1_STREAM_LOAD=1`, scratch, never for merge): MOVNTDQA streaming loads.
        static STREAM: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let stream = *STREAM.get_or_init(|| std::env::var("KF3_BAR1_STREAM_LOAD").is_ok_and(|v| v == "1"));
        let r = if stream {
            cpu.copy_out_wide(HostOffset::new(off), bytes, 1)
        } else {
            cpu.copy_out(HostOffset::new(off), bytes)
        }
        .map_err(|_| Refusal::Store);
        if let Some(t0) = t0 {
            crate::lifecycle::note_vram(
                bytes.len(),
                u64::try_from(t0.elapsed().as_nanos()).unwrap_or(u64::MAX),
            );
        }
        r
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
        /// the next N arms are refused by RM (no mapping exists afterwards)
        refuse_arms: Cell<u32>,
    }
    impl Host for Fake {
        type View = u64;
        fn arm(&self, off: u64, _: u64) -> Result<u64, ArmError<u64>> {
            self.events.borrow_mut().push(("arm", off));
            if self.refuse_arms.get() > 0 {
                self.refuse_arms.set(self.refuse_arms.get() - 1);
                return Err(ArmError { retained: None });
            }
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
    fn counts(c: &Cache<Fake>) -> (usize, usize) {
        let ev = c.host.events.borrow();
        (
            ev.iter().filter(|e| e.0 == "arm").count(),
            ev.iter().filter(|e| e.0 == "release").count(),
        )
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
    fn actual_cache_prepares_full_fragmented_ram_and_contiguous_store_snapshots() {
        use crate::mem::{PlacedRows, RamBlock, RamMap, RowsLog, userread};
        let bytes = kf_chan::user_storage::GP_OFFSET as usize;
        for in_ram in [true, false] {
            let ram = RamMap::default();
            let (mem, fd) = crate::raw_unsafe::test_owned_ram(bytes + 4096);
            ram.add(RamBlock { gpa: 0x2000, mem, fd: Some(fd), fd_off: 0 });
            assert!(mem.write_from(4, &[7]));
            assert!(mem.write_from(bytes + 3, &[11]));
            let generation = ram.try_generation().unwrap();
            let rows = PlacedRows::default();
            for offset in (0..bytes + 4096).step_by(4096) {
                rows.write().unwrap().insert(0x100000 + offset as u64,
                    (4096, offset as u64, in_ram, kf_host::MapPerm::READ_WRITE));
            }
            let mut cache = Cache::create(Fake::default(), bytes as u64 + BYTES).unwrap();
            let log = RowsLog::default();
            let source = userread::snapshot(&rows, &log, &ram, generation, 0x100004, bytes, &mut cache).unwrap();
            assert_eq!(source.bytes.len(), bytes);
            if in_ram {
                assert!(cache.host.events.borrow().is_empty());
                assert_eq!((source.bytes[0], source.bytes[bytes - 1]), (7, 11));
            } else {
                assert_eq!(cache.views.len(), 15);
                assert_eq!((source.bytes[0], source.bytes[bytes - 1]), (0, 14));
            }
            source.admit(|| ()).unwrap();
            rows.write().unwrap().remove(&(0x100000 + bytes as u64));
            assert_eq!(source.admit(|| ()), Err(Refusal::Missing(0x100000 + bytes as u64)));
            cache.release().unwrap();
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
        let mut c = Cache::create(Fake::default(), BYTES * (VIEWS as u64 + 1)).unwrap();
        let spans: Vec<_> = (0..VIEWS as u64 + 1).map(|i| span(i * BYTES, 4)).collect();
        assert_eq!(c.prepare(&spans), Err(Refusal::Bounds));
        for s in [
            span(u64::MAX, 4),
            span(c.bound - 1, 2),
            span(0, crate::mem::userread::MAX_BYTES as u64 + 1),
        ] {
            assert_eq!(c.prepare(&[s]), Err(Refusal::Bounds));
        }
        assert!(c.host.events.borrow().is_empty());
        c.prepare(&[span(0, 4), span(0, 4)]).unwrap();
        assert_eq!(c.views.len(), 1);
    }
    #[test]
    fn failed_release_keeps_ownership_and_never_reuses_revoked_cpu_view() {
        // Views are released only at the channel's limit: fill it, then ask for one more window.
        let mut c = Cache::create(Fake::default(), BYTES * 64).unwrap();
        c.keep = 2;
        c.prepare(&[span(0, 4)]).unwrap();
        c.prepare(&[span(BYTES, 4)]).unwrap();
        c.host.fail_release.set(true);
        assert_eq!(c.prepare(&[span(2 * BYTES, 4)]), Err(Refusal::Store));
        // the LRU view (window 0) was being released; ownership is kept, it is no longer readable
        assert_eq!(c.views.len(), 2);
        assert!(c.views.iter().any(|v| !v.ready && v.native == 0));
        assert_eq!(c.copy(0, &mut [0; 4]), Err(Refusal::Store));
        assert!(!c.host.events.borrow().contains(&("arm", 2 * BYTES)));
        c.host.fail_release.set(false);
        c.prepare(&[span(2 * BYTES, 4)]).unwrap();
        assert!(c.views.iter().any(|v| v.native == 2 * BYTES && v.ready));
        assert!(c.views.iter().all(|v| v.native != 0));
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

    // ---- the churn case of the perf campaign (2026-10-15): reproduced first, then fixed ----
    #[test]
    fn alternating_windows_are_armed_once_not_per_snapshot() {
        // A D3D11 channel alternates between two 64 KiB store windows on nearly every submission.
        // The old cache released the window the next snapshot did not need: 2 arms + 1 release per pair.
        let mut c = Cache::create(Fake::default(), BYTES * 64).unwrap();
        for _ in 0..1000 {
            c.prepare(&[span(0, 64)]).unwrap();
            c.prepare(&[span(3 * BYTES, 64)]).unwrap();
        }
        assert_eq!(counts(&c), (2, 0), "each window armed once, nothing released");
        let mut out = [0u8; 4];
        c.copy(3 * BYTES, &mut out).unwrap();
    }
    #[test]
    fn many_channels_many_slices_arm_each_slice_once() {
        // N channels x M distinct windows each (M <= keep): total arms == N x M however long it runs.
        let (n, m) = (24usize, 6u64);
        let mut chans: Vec<Cache<Fake>> =
            (0..n).map(|_| Cache::create(Fake::default(), BYTES * 64).unwrap()).collect();
        for round in 0..400u64 {
            for c in chans.iter_mut() {
                c.prepare(&[span((round % m) * BYTES, 32)]).unwrap();
            }
        }
        let arms: usize = chans.iter().map(|c| counts(c).0).sum();
        let releases: usize = chans.iter().map(|c| counts(c).1).sum();
        assert_eq!((arms, releases), (n * m as usize, 0));
    }
    #[test]
    fn a_working_set_beyond_the_limit_releases_least_recently_used_first() {
        let mut c = Cache::create(Fake::default(), BYTES * 64).unwrap();
        c.keep = 4;
        for i in 0..7u64 {
            c.prepare(&[span(i * BYTES, 4)]).unwrap();
        }
        assert_eq!(c.views.len(), 4, "held at the limit");
        let released: Vec<u64> = c.host.events.borrow().iter().filter(|e| e.0 == "release").map(|e| e.1).collect();
        assert_eq!(released, vec![0, BYTES, 2 * BYTES], "oldest first");
        // touching window 3 makes it the newest: the next eviction takes window 4, not 3
        c.prepare(&[span(3 * BYTES, 4)]).unwrap();
        c.prepare(&[span(7 * BYTES, 4)]).unwrap();
        assert!(c.views.iter().any(|v| v.off == 3 * BYTES));
        assert!(!c.views.iter().any(|v| v.off == 4 * BYTES));
    }
    #[test]
    fn a_snapshot_larger_than_the_limit_is_still_satisfied_in_full() {
        let mut c = Cache::create(Fake::default(), BYTES * 64).unwrap();
        c.keep = 2;
        let wide: Vec<_> = (0..6u64).map(|i| span(i * BYTES, 4)).collect();
        c.prepare(&wide).unwrap();
        assert_eq!(c.views.len(), 6);
        for i in 0..6u64 {
            c.copy(i * BYTES, &mut [0; 4]).unwrap();
        }
        // the next small snapshot trims back to the limit, least recently used first
        c.prepare(&[span(5 * BYTES, 4)]).unwrap();
        assert_eq!(c.views.len(), 6, "trimming happens only when a new window needs a slot");
    }
    #[test]
    fn an_rm_refusal_evicts_the_channels_own_lru_view_and_retries_once() {
        let mut c = Cache::create(Fake::default(), BYTES * 64).unwrap();
        c.prepare(&[span(0, 4)]).unwrap();
        c.prepare(&[span(BYTES, 4)]).unwrap();
        c.host.events.borrow_mut().clear();
        let before = ARM_RETRY_OK.load(std::sync::atomic::Ordering::Relaxed);
        c.host.refuse_arms.set(1);
        c.prepare(&[span(2 * BYTES, 4)]).unwrap();
        // refused arm, LRU window 0 released, retry armed window 2; window 1 untouched
        assert_eq!(*c.host.events.borrow(), [("arm", 2 * BYTES), ("release", 0), ("arm", 2 * BYTES)]);
        assert!(c.views.iter().any(|v| v.off == BYTES && v.ready));
        assert!(c.views.iter().any(|v| v.off == 2 * BYTES && v.ready));
        assert!(ARM_RETRY_OK.load(std::sync::atomic::Ordering::Relaxed) > before);
    }
    #[test]
    fn two_rm_refusals_fall_back_to_arm_release_for_that_read() {
        let mut c = Cache::create(Fake::default(), BYTES * 64).unwrap();
        for i in 0..3u64 {
            c.prepare(&[span(i * BYTES, 4)]).unwrap();
        }
        c.host.events.borrow_mut().clear();
        let before = ARM_FALLBACK_OK.load(std::sync::atomic::Ordering::Relaxed);
        c.host.refuse_arms.set(2);
        c.prepare(&[span(9 * BYTES, 4)]).unwrap();
        // today's behaviour for this read: everything it does not need is released, then it is armed
        assert_eq!(c.views.len(), 1);
        assert_eq!(c.views[0].off, 9 * BYTES);
        assert!(c.views[0].ready);
        assert!(ARM_FALLBACK_OK.load(std::sync::atomic::Ordering::Relaxed) > before);
        c.copy(9 * BYTES, &mut [0; 4]).unwrap();
    }
    #[test]
    fn three_rm_refusals_refuse_the_read_by_name_and_leave_a_consistent_cache() {
        let mut c = Cache::create(Fake::default(), BYTES * 64).unwrap();
        c.prepare(&[span(0, 4)]).unwrap();
        let before = ARM_FAILED.load(std::sync::atomic::Ordering::Relaxed);
        c.host.refuse_arms.set(3);
        assert_eq!(c.prepare(&[span(5 * BYTES, 4)]), Err(Refusal::Store));
        assert!(ARM_FAILED.load(std::sync::atomic::Ordering::Relaxed) > before);
        assert_eq!(c.copy(5 * BYTES, &mut [0; 4]), Err(Refusal::Store), "never readable");
        // the channel is not poisoned: the next read arms normally
        c.prepare(&[span(5 * BYTES, 4)]).unwrap();
        c.copy(5 * BYTES, &mut [0; 4]).unwrap();
        c.release().unwrap();
        assert!(c.views.is_empty());
    }

    // ---- ONE real-mmap stress (perf campaign 2026-10-15): the backend maps a REAL memfd per view, so a
    // read through a released view would SIGSEGV instead of passing silently ----
    mod stress {
        use super::*;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};

        fn pattern(x: u64) -> u8 {
            ((x ^ (x >> 13)).wrapping_mul(0x9E37_79B1) >> 7) as u8
        }
        fn rng(state: &mut u64) -> u64 {
            *state ^= *state << 13;
            *state ^= *state >> 7;
            *state ^= *state << 17;
            *state
        }
        #[derive(Default)]
        struct Counters {
            rm_maps: AtomicU64,
            rm_unmaps: AtomicU64,
            live_rm: AtomicUsize,
            live_regions: AtomicUsize,
            arm_refused: AtomicU64,
            release_refused: AtomicU64,
            partial: AtomicU64,
            reads: AtomicU64,
            bytes: AtomicU64,
        }
        struct StubView {
            region: Option<VolatileRegion>,
        }
        struct MemfdHost {
            c: Arc<Counters>,
            rng: std::cell::Cell<u64>,
            fail_permille: u64,
        }
        impl MemfdHost {
            fn roll(&self, permille: u64) -> bool {
                let mut r = self.rng.get();
                let v = rng(&mut r) % 1000;
                self.rng.set(r);
                v < permille
            }
        }
        impl Host for MemfdHost {
            type View = StubView;
            fn arm(&self, off: u64, len: u64) -> Result<StubView, ArmError<StubView>> {
                if self.roll(self.fail_permille) {
                    self.c.arm_refused.fetch_add(1, Relaxed);
                    return Err(ArmError { retained: None });
                }
                let ram = kf_linux_raw::SharedRam::create_named(c"kf-stress", len).map_err(|_| ArmError { retained: None })?;
                let page = HostPageSize::query();
                let fill = kf_linux_raw::MappedRegion::map(
                    Backing::SharedFile { fd: ram.as_backing_fd(), offset: 0 },
                    len,
                    kf_linux_raw::HostProt::ReadWrite,
                    CachePolicy::WriteBack,
                    page,
                )
                .map_err(|_| ArmError { retained: None })?;
                let bytes: Vec<u8> = (0..len).map(|i| pattern(off + i)).collect();
                fill.write_from(HostOffset::new(0), &bytes).map_err(|_| ArmError { retained: None })?;
                drop(fill);
                let view = VolatileRegion::map_read_only(
                    Backing::SharedFile { fd: ram.as_backing_fd(), offset: 0 },
                    len,
                    CachePolicy::WriteBack,
                    page,
                )
                .map_err(|_| ArmError { retained: None })?;
                self.c.rm_maps.fetch_add(1, Relaxed);
                self.c.live_rm.fetch_add(1, Relaxed);
                if self.roll(self.fail_permille / 4) {
                    // the RM mapping exists but the CPU mapping failed: ownership must be retained
                    drop(view);
                    self.c.partial.fetch_add(1, Relaxed);
                    return Err(ArmError { retained: Some(StubView { region: None }) });
                }
                self.c.live_regions.fetch_add(1, Relaxed);
                Ok(StubView { region: Some(view) })
            }
            fn release(&self, view: &mut StubView) -> Result<(), Refusal> {
                if self.roll(self.fail_permille / 2) {
                    self.c.release_refused.fetch_add(1, Relaxed);
                    return Err(Refusal::Store);
                }
                if view.region.take().is_some() {
                    self.c.live_regions.fetch_sub(1, Relaxed);
                }
                self.c.rm_unmaps.fetch_add(1, Relaxed);
                self.c.live_rm.fetch_sub(1, Relaxed);
                Ok(())
            }
            fn copy(&self, view: &StubView, off: u64, bytes: &mut [u8]) -> Result<(), Refusal> {
                self.c.reads.fetch_add(1, Relaxed);
                self.c.bytes.fetch_add(bytes.len() as u64, Relaxed);
                view.region.as_ref().ok_or(Refusal::Store)?.copy_out(HostOffset::new(off), bytes).map_err(|_| Refusal::Store)
            }
        }
        fn maps_with_name() -> usize {
            std::fs::read_to_string("/proc/self/maps").unwrap().lines().filter(|l| l.contains("kf-stress")).count()
        }
        fn fds() -> usize {
            std::fs::read_dir("/proc/self/fd").unwrap().count()
        }

        #[test]
        fn eight_threads_arm_evict_and_read_real_mappings_with_injected_failures() {
            let secs: u64 = std::env::var("KF3_STRESS_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
            let seed: u64 = std::env::var("KF3_STRESS_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or_else(|| {
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64 | 1
            });
            let threads: usize = std::env::var("KF3_STRESS_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
            eprintln!("userstore stress: seed={seed} threads={threads} secs={secs}");
            let (maps0, fds0) = (maps_with_name(), fds());
            let c = Arc::new(Counters::default());
            let bound = BYTES * 256;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
            let handles: Vec<_> = (0..threads)
                .map(|t| {
                    let c = c.clone();
                    std::thread::spawn(move || {
                        let mut r = seed ^ ((t as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
                        let host = MemfdHost { c: c.clone(), rng: std::cell::Cell::new(r | 1), fail_permille: 30 };
                        let mut cache = Cache::create(host, bound).unwrap();
                        cache.keep = 2 + (t % 15);
                        let hostile = t == 0; // thread 0 hammers distinct windows (no working set)
                        let mut snaps = 0u64;
                        while std::time::Instant::now() < deadline {
                            let n_spans = 1 + rng(&mut r) % 3;
                            let mut spans = Vec::new();
                            for _ in 0..n_spans {
                                let len = 1 + rng(&mut r) % (BYTES * 3 / 2);
                                let win = if hostile { rng(&mut r) % 250 } else { (t as u64 * 7 + rng(&mut r) % 5) % 250 };
                                let off = (win * BYTES + rng(&mut r) % BYTES).min(bound - len);
                                spans.push(span(off, len));
                            }
                            match cache.prepare(&spans) {
                                Ok(()) => {
                                    for sp in &spans {
                                        let mut buf = vec![0u8; sp.len as usize];
                                        cache.copy(sp.off, &mut buf).expect("a prepared span must be readable");
                                        for (i, b) in buf.iter().enumerate() {
                                            assert_eq!(*b, pattern(sp.off + i as u64), "seed={seed} thread={t} snapshot={snaps}: wrong byte at {:#x}", sp.off + i as u64);
                                        }
                                    }
                                }
                                Err(Refusal::Store | Refusal::Bounds) => {}
                                Err(e) => panic!("seed={seed} thread={t}: unexpected {e:?}"),
                            }
                            snaps += 1;
                        }
                        // stop injecting, then release everything the cache still owns
                        cache.host.fail_permille = 0;
                        cache.release().expect("release with no injection");
                        assert!(cache.views.is_empty());
                        snaps
                    })
                })
                .collect();
            let snaps: u64 = handles.into_iter().map(|h| h.join().expect("a stress thread panicked (seed logged above)")).sum();
            let (maps, unmaps) = (c.rm_maps.load(Relaxed), c.rm_unmaps.load(Relaxed));
            eprintln!(
                "userstore stress: snapshots={snaps} reads={} bytes={} rm_maps={maps} rm_unmaps={unmaps} arm_refused={} release_refused={} partial={}",
                c.reads.load(Relaxed), c.bytes.load(Relaxed), c.arm_refused.load(Relaxed), c.release_refused.load(Relaxed), c.partial.load(Relaxed)
            );
            assert_eq!(maps, unmaps, "arms - releases must be zero once every cache released");
            assert_eq!(c.live_rm.load(Relaxed), 0);
            assert_eq!(c.live_regions.load(Relaxed), 0);
            assert_eq!(maps_with_name(), maps0, "no leaked mapping");
            assert!(fds() <= fds0 + 1, "descriptors back to baseline");
            assert!(snaps > 0 && c.arm_refused.load(Relaxed) > 0 && c.release_refused.load(Relaxed) > 0 && c.partial.load(Relaxed) > 0, "the injections ran");
        }
    }
}
