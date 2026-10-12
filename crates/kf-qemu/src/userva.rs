//! VA-worker ownership of restricted Windows read-only command mappings.
//!
//! The shared exclusions are installed BEFORE any RM call and survive uncertain
//! cleanup. Linux Passthrough cannot request a lease without a Windows birth
//! permit. A lease is never retired by GET, a guest free, an error or a timer:
//! its native channel must first have been successfully freed (or never born).

use crate::mem::{Manager, Mirror, Target};
use kf_chan::user_storage::StorageHost;
use kf_chan::user_storage::{BYTES, CpuView, Storage};
use kf_mem::vasmgr::VasKey;
use kf_rm::chanlink::WindowsUserRingPermit;
use std::sync::{Arc, Mutex};

/// Trusted per-space resource budget, not a GPU/die fact or guest ring size.
const SLOTS: usize = 256;
/// Allocator policy below the public 40-bit command-fetch limit.
const BASE: u64 = (1 << 40) - (1 << 32);
/// Bounded pending VA-worker requests for the device.
const REQUESTS: usize = 512;

/// Private ownership shared by channel and VA planes for ONE mirror lifetime.
#[derive(Default, Debug)]
pub struct Regions {
    state: Mutex<RegionState>,
}
#[derive(Default, Debug)]
struct RegionState {
    slots: Vec<(u64, u64)>,
    steers: Vec<(u64, u64, u64)>,
    next: u64,
}

/// In-flight falcon handover admission, retained across parked retries and the
/// native engine-object allocation. Drop removes metadata only, never RM state.
#[derive(Debug)]
pub struct SteerClaim {
    regions: Arc<Regions>,
    id: u64,
}
impl Drop for SteerClaim {
    fn drop(&mut self) {
        if let Ok(mut state) = self.regions.state.lock() {
            state.steers.retain(|&(id, _, _)| id != self.id);
        }
    }
}
impl Regions {
    /// Snapshot exclusions; poisoning refuses all guest mappings below 2^40.
    pub fn ranges(&self) -> Vec<(u64, u64)> {
        self.state
            .lock()
            .map(|s| s.slots.clone())
            .unwrap_or_else(|_| vec![(0, 1 << 40)])
    }
    /// Guest map/unmap paths check complete spans before host actions.
    pub fn check(&self, va: u64, len: u64) -> Result<(), String> {
        let end = va.checked_add(len).ok_or("USER_RING_GUEST_SPAN_OVERFLOW")?;
        let state = self
            .state
            .lock()
            .map_err(|_| "USER_RING_REGIONS_POISONED")?;
        if state.slots.iter().any(|&(a, b)| va < b && a < end) {
            return Err(format!("USER_RING_PRIVATE_OVERLAP {va:#x}+{len:#x}"));
        }
        Ok(())
    }
    /// Atomic against private-slot admission. No metadata lock crosses a host
    /// call, ledger handover, or parked retry. Every channel in this mirror uses
    /// the same metadata; a Linux mirror still has no private GPU mappings.
    pub fn steer(self: &Arc<Self>, va: u64, len: u64) -> Result<Arc<SteerClaim>, String> {
        let end = va
            .checked_add(len)
            .filter(|_| len != 0)
            .ok_or("USER_RING_STEER_SPAN")?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "USER_RING_REGIONS_POISONED")?;
        if state.steers.len() >= REQUESTS
            || state.slots.iter().any(|&(a, b)| va < b && a < end)
            || state.steers.iter().any(|&(_, a, b)| va < b && a < end)
        {
            return Err("USER_RING_STEER_BUSY".into());
        }
        let id = state
            .next
            .checked_add(1)
            .ok_or("USER_RING_STEER_SEQUENCE")?;
        state
            .steers
            .try_reserve(1)
            .map_err(|_| "USER_RING_STEER_ALLOCATION")?;
        state.next = id;
        state.steers.push((id, va, end));
        Ok(Arc::new(SteerClaim {
            regions: self.clone(),
            id,
        }))
    }
    // VA worker checks placed rows, SKED and ledger BEFORE installing exclusion.
    fn claim(&self, va: u64) -> Result<(), String> {
        let end = va.checked_add(BYTES).ok_or("USER_RING_PRIVATE_SPAN")?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "USER_RING_REGIONS_POISONED")?;
        if state.slots.len() >= SLOTS
            || state.slots.iter().any(|&(a, b)| va < b && a < end)
            || state.steers.iter().any(|&(_, a, b)| va < b && a < end)
        {
            return Err("USER_RING_PRIVATE_SLOT_BUSY".into());
        }
        state
            .slots
            .try_reserve(1)
            .map_err(|_| "USER_RING_REGION_ALLOCATION")?;
        state.slots.push((va, end));
        Ok(())
    }
    fn release(&self, va: u64) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "USER_RING_REGIONS_POISONED")?;
        let at = state
            .slots
            .iter()
            .position(|&(a, _)| a == va)
            .ok_or("USER_RING_REGION_MISSING")?;
        state.slots.remove(at);
        Ok(())
    }
}

/// Worker-owned native storage and its permanent mirror lifetime association.
pub struct Lease<V = CpuView> {
    /// CPU access is worker-only; no vCPU takes this mutex.
    pub storage: Mutex<Storage<V>>,
    mirror: Mirror,
    va: u64,
    retired: std::sync::atomic::AtomicBool,
    // 1: bounded clear,2: map on a later turn,0: ready/refused.
    initializing: std::sync::atomic::AtomicU8,
    // Never-born error/cancellation cleanup gets ONE automatic attempt. A
    // failed cleanup remains quarantined, rather than retrying on every turn.
    never_born_cleanup_attempted: std::sync::atomic::AtomicBool,
    reply: Arc<Reply<V>>,
}

/// A nonblocking allocation reply. Lost replies retain resources in Queue.
pub struct Reply<V = CpuView>(Mutex<ReplyState<V>>);

enum ReplyState<V> {
    Pending,
    Ready(Result<std::sync::Weak<Lease<V>>, String>),
    Taken,
    Canceled { reported: bool },
}

impl<V> Reply<V> {
    /// Act/worker poll; absence never grants a resource or reuse credit.
    pub fn take(&self) -> Result<Option<Result<Arc<Lease<V>>, String>>, String> {
        self.0
            .lock()
            .map(|mut r| match &mut *r {
                ReplyState::Pending | ReplyState::Taken => None,
                ReplyState::Canceled { reported } => {
                    if *reported {
                        None
                    } else {
                        *reported = true;
                        Some(Err("USER_RING_BIRTH_CANCELED".into()))
                    }
                }
                ReplyState::Ready(_) => {
                    let ReplyState::Ready(result) = std::mem::replace(&mut *r, ReplyState::Taken)
                    else {
                        return Some(Err("USER_RING_REPLY_STATE".into()));
                    };
                    Some(result.and_then(|lease| {
                        lease
                            .upgrade()
                            .ok_or_else(|| "USER_RING_REPLY_RETIRED".into())
                    }))
                }
            })
            .map_err(|_| "USER_RING_REPLY_POISONED".into())
    }

    /// Cancel only BEFORE a successful resource handout. This does metadata
    /// work only; the VA worker cleans resources that never had a channel.
    /// Once take() wins, the caller owns native birth/free and must use the
    /// explicit release_after_channel_free path. Cancellation cannot retire it.
    pub fn cancel_before_birth(&self) -> Result<(), String> {
        let mut state = self.0.lock().map_err(|_| "USER_RING_REPLY_POISONED")?;
        match &*state {
            ReplyState::Taken => Err("USER_RING_BIRTH_ALREADY_HANDED_OUT".into()),
            ReplyState::Canceled { .. } => Ok(()),
            ReplyState::Pending | ReplyState::Ready(_) => {
                *state = ReplyState::Canceled { reported: false };
                Ok(())
            }
        }
    }

    fn is_canceled(&self) -> bool {
        self.0
            .lock()
            .is_ok_and(|state| matches!(*state, ReplyState::Canceled { .. }))
    }

    fn publish(&self, result: Result<std::sync::Weak<Lease<V>>, String>) {
        if let Ok(mut state) = self.0.lock()
            && matches!(*state, ReplyState::Pending)
        {
            *state = ReplyState::Ready(result);
        }
        // Cancellation may have won during a host operation. Never overwrite
        // it with a successful handout; Queue still owns the resource.
    }
}

enum Request<V> {
    Allocate {
        key: VasKey,
        mirror: Mirror,
        _permit: WindowsUserRingPermit,
        reply: Arc<Reply<V>>,
    },
    // Enqueued only after successful native channel free, or before birth.
    Release(Arc<Lease<V>>),
}

/// No host verbs outside `serve`, called exclusively by Device::va_loop.
pub struct Queue<V = CpuView> {
    pending: Mutex<Vec<Request<V>>>,
    // Strong ownership even if a birth reply is lost or cleanup is refused.
    held: Mutex<Vec<Arc<Lease<V>>>>,
    // VA-thread scheduling only: one initialization phase between requests.
    init_turn: std::sync::atomic::AtomicBool,
}

impl<V> Default for Queue<V> {
    fn default() -> Self {
        Self {
            pending: Mutex::new(Vec::new()),
            held: Mutex::new(Vec::new()),
            init_turn: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl<V> std::fmt::Debug for Queue<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WindowsPrivateVaQueue")
    }
}

impl<V> Queue<V> {
    /// Bounded request; permit and mirror lifetime are retained before enqueue.
    /// Caller signals Inbox::wake after success; never called by a vCPU.
    pub fn allocate(
        &self,
        key: VasKey,
        mirror: Mirror,
        permit: WindowsUserRingPermit,
    ) -> Result<Arc<Reply<V>>, String> {
        let mut q = self
            .pending
            .lock()
            .map_err(|_| "USER_RING_QUEUE_POISONED")?;
        if q.len() >= REQUESTS {
            return Err("USER_RING_REQUEST_BUDGET".into());
        }
        q.try_reserve(1)
            .map_err(|_| "USER_RING_REQUEST_ALLOCATION")?;
        let reply = Arc::new(Reply(Mutex::new(ReplyState::Pending)));
        // A pending allocation/storage keeps its mirror unrecyclable even if
        // the guest frees its VAS/channel before the allocation reply.
        mirror
            .live
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        q.push(Request::Allocate {
            key,
            mirror,
            _permit: permit,
            reply: reply.clone(),
        });
        Ok(reply)
    }

    /// Native channel successfully freed, or this lease never had a channel.
    /// A failed enqueue retains Queue's strong ownership and mirror live count.
    pub fn release_after_channel_free(&self, lease: Arc<Lease<V>>) -> Result<(), String> {
        if lease
            .initializing
            .load(std::sync::atomic::Ordering::Acquire)
            != 0
        {
            return Err("USER_RING_RELEASE_BEFORE_INIT_REPLY".into());
        }
        let mut q = self
            .pending
            .lock()
            .map_err(|_| "USER_RING_QUEUE_POISONED")?;
        if q.len() >= REQUESTS {
            return Err("USER_RING_REQUEST_BUDGET".into());
        }
        q.try_reserve(1)
            .map_err(|_| "USER_RING_REQUEST_ALLOCATION")?;
        q.push(Request::Release(lease));
        Ok(())
    }

    /// Pending work; an idle VA loop signals itself rather than sleeping 50ms.
    pub fn has_work(&self) -> bool {
        self.pending.lock().is_ok_and(|q| !q.is_empty())
            || self.held.lock().is_ok_and(|v| {
                v.iter().any(|l| {
                    l.initializing.load(std::sync::atomic::Ordering::Acquire) != 0
                        || (l.reply.is_canceled()
                            && !l
                                .never_born_cleanup_attempted
                                .load(std::sync::atomic::Ordering::Acquire))
                })
            })
    }

    /// One request OR one bounded initialization step, only AFTER current
    /// reconciliation and only while no invalidate/walk/statement is pending.
    /// No deadline releases GPU state. Native RM latency is logged separately.
    pub fn serve(&self, manager: &Manager, rm: &impl StorageHost<View = V>) {
        let (request, initializing) = self.pop_work();
        if let Some(request) = request {
            match request {
                Request::Allocate {
                    key, mirror, reply, ..
                } => {
                    if Self::abort_canceled_allocate(&mirror, &reply) {
                        return;
                    }
                    let t = std::time::Instant::now();
                    let result = self.build(manager, rm, key, &mirror, reply.clone());
                    eprintln!(
                        "kf3: USER_RING_PREPARE elapsed_us={} ok={}",
                        t.elapsed().as_micros(),
                        result.is_ok()
                    );
                    Self::report_prepare(&mirror, &reply, result);
                }
                Request::Release(lease) => self.retire(&lease, rm),
            }
            // Re-evaluate invalidate/statement priority on the next VA turn.
            // Never stack an initialization map behind this request's RM ops.
            return;
        }
        let Some(lease) = initializing else {
            return;
        };
        self.initialize(&lease, rm);
    }

    // Scheduling is independent of request success/cancellation: request churn
    // cannot starve an older bounded initialization phase.
    fn pop_work(&self) -> (Option<Request<V>>, Option<Arc<Lease<V>>>) {
        let initializing = self.held.lock().ok().and_then(|v| {
            v.iter()
                .find(|l| {
                    l.initializing.load(std::sync::atomic::Ordering::Acquire) != 0
                        || (l.reply.is_canceled()
                            && !l
                                .never_born_cleanup_attempted
                                .load(std::sync::atomic::Ordering::Acquire))
                })
                .cloned()
        });
        let init_turn = self
            .init_turn
            .swap(false, std::sync::atomic::Ordering::Relaxed);
        let request = self.pending.lock().ok().and_then(|mut q| {
            if q.is_empty() || (init_turn && initializing.is_some()) {
                None
            } else {
                Some(q.remove(0))
            }
        });
        if request.is_some() {
            self.init_turn
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        (request, initializing)
    }

    fn initialize(&self, lease: &Arc<Lease<V>>, rm: &impl StorageHost<View = V>) {
        if self.reap_canceled(lease, rm) {
            return;
        }
        let Ok(mut storage) = lease.storage.try_lock() else {
            return;
        };
        let phase = lease
            .initializing
            .load(std::sync::atomic::Ordering::Acquire);
        if phase != 1 && phase != 2 {
            return;
        }
        let t = std::time::Instant::now();
        let result = if phase == 1 {
            storage.clear_next(rm).map(|done| {
                if done {
                    lease
                        .initializing
                        .store(2, std::sync::atomic::Ordering::Release);
                }
                false
            })
        } else {
            storage.map_initialized(rm).map(|()| true)
        };
        let elapsed = t.elapsed().as_micros();
        if phase == 2 || elapsed > 1000 {
            eprintln!("kf3: USER_RING_INIT phase={phase} elapsed_us={elapsed}");
        }
        match result {
            Ok(false) => {}
            Ok(true) => {
                lease
                    .initializing
                    .store(0, std::sync::atomic::Ordering::Release);
                lease.reply.publish(Ok(Arc::downgrade(&lease)));
            }
            Err(error) => {
                lease
                    .initializing
                    .store(0, std::sync::atomic::Ordering::Release);
                drop(storage);
                // Record this attempt before cleanup: cancellation can win
                // during the host call or before the error reply is published.
                // Either way it cannot manufacture a second automatic retry.
                self.cleanup_never_born(lease, rm);
                lease.reply.publish(Err(error));
            }
        }
    }

    fn abort_canceled_allocate(mirror: &Mirror, reply: &Reply<V>) -> bool {
        if !reply.is_canceled() {
            return false;
        }
        // This request has been popped exactly once and performed no host op.
        mirror
            .live
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
        true
    }

    fn reap_canceled(&self, lease: &Arc<Lease<V>>, rm: &impl StorageHost<View = V>) -> bool {
        if !lease.reply.is_canceled()
            || lease
                .never_born_cleanup_attempted
                .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return false;
        }
        // Canceled and Taken are mutually exclusive under the reply mutex:
        // no caller ever received this lease, so a native channel cannot exist.
        lease
            .initializing
            .store(0, std::sync::atomic::Ordering::Release);
        self.retire(lease, rm);
        true
    }

    fn cleanup_never_born(&self, lease: &Arc<Lease<V>>, rm: &impl StorageHost<View = V>) {
        if !lease
            .never_born_cleanup_attempted
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            self.retire(lease, rm);
        }
    }

    fn retire(&self, lease: &Arc<Lease<V>>, rm: &impl StorageHost<View = V>) {
        if lease.retired.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        let Ok(mut storage) = lease.storage.try_lock() else {
            eprintln!("kf3: USER_RING_RELEASE_BUSY retained va={:#x}", lease.va);
            return;
        };
        if let Err(error) = storage.release(rm) {
            eprintln!(
                "kf3: USER_RING_RELEASE_REFUSED retained va={:#x}: {error}",
                lease.va
            );
            return;
        }
        if !storage.released() {
            return;
        }
        // Serialize duplicate retire with this storage lock through removal of
        // the generation's exclusion; an old lease cannot remove a reused slot.
        if let Err(error) = lease.mirror.user_regions.release(lease.va) {
            eprintln!("kf3: USER_RING_REGION_RELEASE_REFUSED: {error}");
            return;
        }
        lease
            .retired
            .store(true, std::sync::atomic::Ordering::Release);
        if let Ok(mut held) = self.held.lock() {
            if let Some(i) = held.iter().position(|v| Arc::ptr_eq(v, lease)) {
                held.remove(i);
                lease
                    .mirror
                    .live
                    .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
            }
        }
    }

    fn build(
        &self,
        manager: &Manager,
        rm: &impl StorageHost<View = V>,
        key: VasKey,
        mirror: &Mirror,
        reply: Arc<Reply<V>>,
    ) -> Result<Arc<Lease<V>>, String> {
        let Some(Target::Gpu(g)) = manager.table.target(key) else {
            return Err("USER_RING_MIRROR_GONE".into());
        };
        if !same_lifetime(g.vas.space, &g.rows, &g.user_regions, mirror) {
            return Err("USER_RING_MIRROR_LIFETIME".into());
        }
        let mut held = self.held.lock().map_err(|_| "USER_RING_OWNER_POISONED")?;
        // Global VM ownership budget, including quarantined failed maps.
        if held.len() >= REQUESTS {
            return Err("USER_RING_STORAGE_BUDGET".into());
        }
        held.try_reserve(1)
            .map_err(|_| "USER_RING_OWNER_ALLOCATION")?;
        let rows = g.rows.read().map_err(|_| "USER_RING_ROWS_POISONED")?;
        let sked = g.sked.lock().map_err(|_| "USER_RING_SKED_POISONED")?;
        let va = (0..SLOTS)
            .find_map(|i| {
                let va = BASE.checked_add((i as u64).checked_mul(BYTES)?)?;
                let end = va.checked_add(BYTES)?;
                let collision = rows
                    .range(..=va)
                    .next_back()
                    .is_some_and(|(&a, &(n, ..))| a.saturating_add(n) > va)
                    || rows.range(va..end).next().is_some()
                    || sked
                        .range(..=va)
                        .next_back()
                        .is_some_and(|(&a, &n)| a.saturating_add(n) > va)
                    || sked.range(va..end).next().is_some()
                    || g.reserved.iter().any(|&(a, b)| va < b && a < end)
                    || mirror
                        .space
                        .guest
                        .iter()
                        .any(|r| r.handle != 0 && va < r.hi && r.lo < end)
                    || !g.bv.own_view(va, end).owned.is_empty();
                (!collision && mirror.user_regions.claim(va).is_ok()).then_some(va)
            })
            .ok_or("USER_RING_NO_PRIVATE_SLOT")?;
        drop(sked);
        drop(rows);
        self.adopt(
            &mut held,
            mirror,
            va,
            reply,
            Storage::prepare(rm, mirror.space, va),
        )
    }

    fn report_prepare(mirror: &Mirror, reply: &Reply<V>, result: Result<Arc<Lease<V>>, String>) {
        if let Err(error) = result {
            mirror
                .live
                .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
            reply.publish(Err(error));
        }
    }

    fn adopt(
        &self,
        held: &mut Vec<Arc<Lease<V>>>,
        mirror: &Mirror,
        va: u64,
        reply: Arc<Reply<V>>,
        prepared: Result<Storage<V>, kf_chan::user_storage::BuildError<V>>,
    ) -> Result<Arc<Lease<V>>, String> {
        let (storage, error) = match prepared {
            Ok(storage) => (storage, None),
            Err(error) => (error.retained, Some(error.reason)),
        };
        if storage.released() {
            mirror.user_regions.release(va)?;
            return Err(error.unwrap_or_else(|| "USER_RING_STORAGE_NOT_LIVE".into()));
        }
        let lease = Arc::new(Lease {
            storage: Mutex::new(storage),
            mirror: mirror.clone(),
            va,
            retired: std::sync::atomic::AtomicBool::new(false),
            initializing: std::sync::atomic::AtomicU8::new(if error.is_none() { 1 } else { 0 }),
            never_born_cleanup_attempted: std::sync::atomic::AtomicBool::new(error.is_some()),
            reply,
        });
        held.push(lease.clone());
        if let Some(error) = error {
            // Retained resource also needs its live-count pin after Err reply.
            mirror
                .live
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            eprintln!("kf3: USER_RING_BUILD_REFUSED retained va={va:#x}: {error}");
            Err(error)
        } else {
            Ok(lease)
        }
    }
}

fn same_lifetime(
    space: kf_host::VaSpace,
    rows: &crate::mem::PlacedRows,
    regions: &Arc<Regions>,
    mirror: &Mirror,
) -> bool {
    space == mirror.space
        && Arc::ptr_eq(rows, &mirror.rows)
        && Arc::ptr_eq(regions, &mirror.user_regions)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exclusions_are_complete_spans_and_survive_no_receipt() {
        let r = Regions::default();
        r.claim(BASE).unwrap();
        assert!(r.check(BASE - 4, 8).is_err());
        assert!(r.check(BASE + BYTES - 4, 8).is_err());
        assert!(r.check(BASE, 4).is_err());
        assert!(r.check(u64::MAX - 3, 8).is_err());
        assert!(r.check(BASE - 4, 4).is_ok());
        assert!(r.check(BASE + BYTES, 4).is_ok());
        assert!(r.claim(BASE).is_err());
        assert_eq!(r.ranges(), [(BASE, BASE + BYTES)]);
        r.release(BASE).unwrap();
        assert!(r.check(BASE, BYTES).is_ok());
    }
    #[test]
    fn linux_default_owns_no_private_ranges_and_budget_is_bounded() {
        let r = Regions::default();
        assert!(r.ranges().is_empty());
        for i in 0..SLOTS {
            r.claim(BASE + i as u64 * BYTES).unwrap();
        }
        assert!(r.claim(BASE + SLOTS as u64 * BYTES).is_err());
        assert_eq!(r.ranges().len(), SLOTS);
    }
    #[test]
    fn steer_and_private_admission_are_atomic_and_retry_guarded() {
        let r = Arc::new(Regions::default());
        let guard = r.steer(BASE - 4, 8).unwrap();
        assert!(r.claim(BASE).is_err());
        let parked = guard.clone();
        drop(guard);
        assert!(r.claim(BASE).is_err());
        assert!(r.steer(BASE - 4, 8).is_err());
        drop(parked);
        r.claim(BASE).unwrap();
        assert!(r.steer(BASE - 4, 8).is_err());
        r.release(BASE).unwrap();
        assert!(r.steer(BASE - 4, 8).is_ok());
        assert!(r.steer(u64::MAX - 3, 8).is_err());
        assert!(r.steer(BASE, 0).is_err());
    }
    #[derive(Default)]
    struct Fake {
        failures: std::cell::RefCell<Vec<&'static str>>,
        calls: std::cell::RefCell<Vec<&'static str>>,
        on_op: std::cell::RefCell<Option<(&'static str, Box<dyn FnOnce()>)>>,
    }
    impl Fake {
        fn op(&self, name: &'static str) -> Result<(), String> {
            self.calls.borrow_mut().push(name);
            let callback = {
                let mut callback = self.on_op.borrow_mut();
                if callback.as_ref().is_some_and(|(op, _)| *op == name) {
                    callback.take().map(|(_, f)| f)
                } else {
                    None
                }
            };
            if let Some(callback) = callback {
                callback();
            }
            if self.failures.borrow().contains(&name) {
                Err(name.into())
            } else {
                Ok(())
            }
        }
    }
    impl StorageHost for Fake {
        type View = ();
        fn reserve(&self, _: kf_host::VaSpace, _: u64) -> Result<u32, String> {
            self.op("reserve")?;
            Ok(1)
        }
        fn alloc(&self) -> Result<u32, String> {
            self.op("alloc")?;
            Ok(2)
        }
        fn view(&self, _: u32) -> Result<(), String> {
            self.op("view")
        }
        fn clear(&self, _: &()) -> Result<(), String> {
            self.op("clear")
        }
        fn clear_range(&self, _: &(), _: u64, _: u64) -> Result<(), String> {
            self.op("clear")
        }
        fn map_ro(
            &self,
            _: u32,
            _: u32,
            va: u64,
        ) -> Result<u64, kf_chan::user_storage::MapFailure> {
            self.op("map")
                .map(|_| va)
                .map_err(|reason| kf_chan::user_storage::MapFailure {
                    reason,
                    unexpected: None,
                })
        }
        fn revoke_view(&self, _: u32, _: &mut ()) -> Result<(), String> {
            self.op("revoke")
        }
        fn unmap(&self, _: u32, _: u64) -> Result<(), String> {
            self.op("unmap")
        }
        fn free_reservation(&self, _: u32) -> Result<(), String> {
            self.op("free_reservation")
        }
        fn free(&self, _: u32) -> Result<(), String> {
            self.op("free")
        }
    }
    fn mirror() -> Mirror {
        Mirror {
            space: kf_host::VaSpace {
                space: 1,
                range: 2,
                guest: Default::default(),
            },
            user_regions: Arc::default(),
            negctl_window: None,
            rows: Default::default(),
            ram_obj: None,
            live: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            kernel_vas: crate::mem::kernel_vas_for(VasKey(7)),
            log: Arc::default(),
            ledger: None,
        }
    }
    fn request(q: &Queue<()>, m: &Mirror) -> Arc<Reply<()>> {
        let permit = kf_rm::chanlink::windows_user_ring_permit(&kf_rm::chanlink::UserWorkFacts {
            flag: true,
            windows: true,
            kernel_stamped: true,
            engine: Some(kf_abi::submit::ENGINE_TYPE_GRAPHICS),
            ctx_share: 1,
            ctx_share_known: true,
            process_id: Some(200),
            kernel_pid: Some(100),
            ..Default::default()
        })
        .unwrap();
        let reply = q.allocate(VasKey(7), m.clone(), permit).unwrap();
        // Simulate the VA-worker pop, then exercise its actual adoption/report helpers.
        assert!(matches!(
            q.pending.lock().unwrap().remove(0),
            Request::Allocate { .. }
        ));
        m.user_regions.claim(BASE).unwrap();
        reply
    }
    fn adopt_ready(q: &Queue<()>, m: &Mirror, host: &Fake) -> Arc<Lease<()>> {
        let reply = request(q, m);
        let built = Storage::allocate(host, m.space, BASE);
        let lease = q
            .adopt(&mut q.held.lock().unwrap(), m, BASE, reply, built)
            .unwrap_or_else(|e| panic!("{e}"));
        // Native allocation fixture is ready; no channel has ever been born.
        lease
            .initializing
            .store(0, std::sync::atomic::Ordering::Release);
        lease
    }
    #[test]
    fn construction_failure_retains_pin_only_while_resources_survive() {
        for retained in [false, true] {
            let q = Queue::<()>::default();
            let m = mirror();
            let host = Fake::default();
            host.failures.borrow_mut().push("alloc");
            if retained {
                host.failures.borrow_mut().push("free_reservation");
            }
            let reply = request(&q, &m);
            let result = q.adopt(
                &mut q.held.lock().unwrap(),
                &m,
                BASE,
                reply.clone(),
                Storage::prepare(&host, m.space, BASE),
            );
            Queue::report_prepare(&m, &reply, result);
            assert!(reply.take().unwrap().unwrap().is_err());
            assert_eq!(
                m.live.load(std::sync::atomic::Ordering::Acquire),
                u64::from(retained)
            );
            assert_eq!(q.held.lock().unwrap().len(), usize::from(retained));
            assert_eq!(m.user_regions.ranges().len(), usize::from(retained));
            if retained {
                let lease = q.held.lock().unwrap()[0].clone();
                host.failures.borrow_mut().clear();
                q.retire(&lease, &host);
                assert!(m.user_regions.ranges().is_empty());
                assert_eq!(m.live.load(std::sync::atomic::Ordering::Acquire), 0);
            }
        }
    }
    #[test]
    fn failed_cleanup_requires_retry_and_old_release_cannot_remove_reused_slot() {
        for failure in ["revoke", "unmap", "free_reservation", "free"] {
            let q = Queue::<()>::default();
            let m = mirror();
            let host = Fake::default();
            let old = adopt_ready(&q, &m, &host);
            host.failures.borrow_mut().push(failure);
            q.retire(&old, &host);
            assert_eq!(m.live.load(std::sync::atomic::Ordering::Acquire), 1);
            assert!(m.user_regions.claim(BASE).is_err());
            assert!(!old.retired.load(std::sync::atomic::Ordering::Acquire));
            host.failures.borrow_mut().clear();
            q.retire(&old, &host);
            assert_eq!(m.live.load(std::sync::atomic::Ordering::Acquire), 0);
            let new = adopt_ready(&q, &m, &host);
            q.retire(&old, &host);
            assert_eq!(m.live.load(std::sync::atomic::Ordering::Acquire), 1);
            assert_eq!(q.held.lock().unwrap().len(), 1);
            assert!(m.user_regions.check(BASE, BYTES).is_err());
            q.retire(&new, &host);
            assert_eq!(m.live.load(std::sync::atomic::Ordering::Acquire), 0);
        }
    }
    #[test]
    fn reused_native_handles_cannot_admit_a_stale_mirror() {
        let old = mirror();
        let new = mirror();
        assert!(same_lifetime(old.space, &old.rows, &old.user_regions, &old));
        assert_eq!(old.space, new.space);
        assert!(!same_lifetime(
            new.space,
            &new.rows,
            &new.user_regions,
            &old
        ));
        assert!(!same_lifetime(
            new.space,
            &old.rows,
            &new.user_regions,
            &old
        ));
        assert!(!same_lifetime(
            new.space,
            &new.rows,
            &old.user_regions,
            &old
        ));
    }

    #[test]
    fn cancellation_before_prepare_returns_only_the_pending_pin() {
        let q = Queue::<()>::default();
        let m = mirror();
        let reply = request(&q, &m);
        // request() also claims the fixture slot; no native allocation exists.
        m.user_regions.release(BASE).unwrap();
        reply.cancel_before_birth().unwrap();
        assert!(Queue::abort_canceled_allocate(&m, &reply));
        assert_eq!(m.live.load(std::sync::atomic::Ordering::Acquire), 0);
        assert!(reply.take().unwrap().unwrap().is_err());
        assert!(reply.take().unwrap().is_none());
        assert!(q.held.lock().unwrap().is_empty());
    }

    #[test]
    fn cancellation_reaps_prepared_or_ready_storage_without_handout() {
        for ready in [false, true] {
            let q = Queue::<()>::default();
            let m = mirror();
            let host = Fake::default();
            let lease = adopt_ready(&q, &m, &host);
            if ready {
                lease.reply.publish(Ok(Arc::downgrade(&lease)));
            } else {
                lease
                    .initializing
                    .store(1, std::sync::atomic::Ordering::Release);
            }
            lease.reply.cancel_before_birth().unwrap();
            // Polling the cancellation does not swallow the worker's cleanup.
            assert!(lease.reply.take().unwrap().unwrap().is_err());
            assert!(q.has_work());
            assert!(q.reap_canceled(&lease, &host));
            assert!(!q.has_work());
            assert!(q.held.lock().unwrap().is_empty());
            assert_eq!(m.live.load(std::sync::atomic::Ordering::Acquire), 0);
            assert!(m.user_regions.ranges().is_empty());
        }
    }

    #[test]
    fn successful_handout_disarms_automatic_cancellation() {
        let q = Queue::<()>::default();
        let m = mirror();
        let host = Fake::default();
        let lease = adopt_ready(&q, &m, &host);
        lease.reply.publish(Ok(Arc::downgrade(&lease)));
        let handed = lease.reply.take().unwrap().unwrap().unwrap();
        assert!(Arc::ptr_eq(&handed, &lease));
        assert!(lease.reply.cancel_before_birth().is_err());
        assert!(!q.reap_canceled(&lease, &host));
        assert!(!q.has_work());
        assert_eq!(m.live.load(std::sync::atomic::Ordering::Acquire), 1);
        assert!(m.user_regions.check(BASE, BYTES).is_err());
        // Only confirmed never-born/native-freed cleanup may now retire it.
        q.retire(&handed, &host);
        assert_eq!(m.live.load(std::sync::atomic::Ordering::Acquire), 0);
    }

    #[test]
    fn canceled_cleanup_failure_quarantines_without_retry_spin() {
        let q = Queue::<()>::default();
        let m = mirror();
        let host = Fake::default();
        let lease = adopt_ready(&q, &m, &host);
        lease.reply.publish(Ok(Arc::downgrade(&lease)));
        lease.reply.cancel_before_birth().unwrap();
        host.failures.borrow_mut().push("unmap");
        assert!(q.reap_canceled(&lease, &host));
        assert!(!q.has_work());
        assert_eq!(q.held.lock().unwrap().len(), 1);
        assert_eq!(m.live.load(std::sync::atomic::Ordering::Acquire), 1);
        assert!(m.user_regions.claim(BASE).is_err());
        assert!(lease.reply.cancel_before_birth().is_ok());
        assert!(!q.reap_canceled(&lease, &host), "no repeated native retry");
        host.failures.borrow_mut().clear();
        q.retire(&lease, &host);
        assert_eq!(m.live.load(std::sync::atomic::Ordering::Acquire), 0);
    }

    #[test]
    fn cancellation_during_or_after_phase_error_does_not_retry_failed_cleanup() {
        for phase in [1, 2] {
            for cancel_at in ["phase", "cleanup", "late"] {
                let q = Queue::<()>::default();
                let m = mirror();
                let host = Fake::default();
                let reply = request(&q, &m);
                let lease = q
                    .adopt(
                        &mut q.held.lock().unwrap(),
                        &m,
                        BASE,
                        reply.clone(),
                        Storage::prepare(&host, m.space, BASE),
                    )
                    .unwrap();
                if phase == 2 {
                    let mut storage = lease.storage.lock().unwrap();
                    while !storage.clear_next(&host).unwrap() {}
                    lease
                        .initializing
                        .store(2, std::sync::atomic::Ordering::Release);
                }
                let phase_op = if phase == 1 { "clear" } else { "map" };
                host.failures.borrow_mut().extend([phase_op, "revoke"]);
                if cancel_at != "late" {
                    let reply = reply.clone();
                    *host.on_op.borrow_mut() = Some((
                        if cancel_at == "phase" {
                            phase_op
                        } else {
                            "revoke"
                        },
                        Box::new(move || reply.cancel_before_birth().unwrap()),
                    ));
                }
                q.initialize(&lease, &host);
                if cancel_at == "late" {
                    reply.cancel_before_birth().unwrap();
                }
                assert!(reply.take().unwrap().unwrap().is_err());
                assert!(!q.has_work(), "phase={phase} cancel_at={cancel_at}");
                assert!(!q.reap_canceled(&lease, &host));
                assert_eq!(
                    host.calls
                        .borrow()
                        .iter()
                        .filter(|&&s| s == "revoke")
                        .count(),
                    1
                );
                assert_eq!(m.live.load(std::sync::atomic::Ordering::Acquire), 1);
                assert!(m.user_regions.claim(BASE).is_err());
                host.failures.borrow_mut().clear();
                q.retire(&lease, &host);
                assert_eq!(m.live.load(std::sync::atomic::Ordering::Acquire), 0);
            }
        }
    }

    #[test]
    fn canceled_request_churn_preserves_the_next_initialization_turn() {
        let q = Queue::<()>::default();
        let m = mirror();
        let host = Fake::default();
        let reply = request(&q, &m);
        let lease = q
            .adopt(
                &mut q.held.lock().unwrap(),
                &m,
                BASE,
                reply,
                Storage::prepare(&host, m.space, BASE),
            )
            .unwrap();
        let canceled = Arc::new(Reply(Mutex::new(ReplyState::Pending)));
        canceled.cancel_before_birth().unwrap();
        // Release requests have the same scheduler eligibility as allocation;
        // exercise an actual canceled Allocate record with the existing permit.
        let permit = kf_rm::chanlink::windows_user_ring_permit(&kf_rm::chanlink::UserWorkFacts {
            flag: true,
            windows: true,
            kernel_stamped: true,
            engine: Some(kf_abi::submit::ENGINE_TYPE_GRAPHICS),
            ctx_share: 1,
            ctx_share_known: true,
            process_id: Some(200),
            kernel_pid: Some(100),
            ..Default::default()
        })
        .unwrap();
        for _ in 0..3 {
            m.live.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            q.pending.lock().unwrap().push(Request::Allocate {
                key: VasKey(7),
                mirror: m.clone(),
                _permit: permit,
                reply: canceled.clone(),
            });
        }
        let (Some(Request::Allocate { mirror, reply, .. }), _) = q.pop_work() else {
            panic!()
        };
        assert!(Queue::abort_canceled_allocate(&mirror, &reply));
        let (next, initializing) = q.pop_work();
        assert!(
            next.is_none(),
            "canceled request must still yield to initialization"
        );
        assert!(Arc::ptr_eq(&initializing.unwrap(), &lease));
        host.calls.borrow_mut().clear();
        q.initialize(&lease, &host);
        assert_eq!(&*host.calls.borrow(), &["clear"]);
        assert_eq!(q.pending.lock().unwrap().len(), 2);
    }

    fn channel_declaration() -> kf_rm::chanlink::ChannelAlloc {
        use kf_rm::chanlink::*;
        ChannelAlloc {
            client: 1,
            parent: 2,
            handle: 3,
            class: 0xc56f,
            gpfifo_va: 0x200000,
            entries: 32768,
            h_vaspace: 7,
            vaspace: Some(7),
            vaspace_client: 1,
            chid: Some(1),
            flags: 0,
            engine_type: Some(kf_abi::submit::ENGINE_TYPE_GRAPHICS),
            userd: None,
            userd_source: None,
            kernel_client: true,
            privilege: None,
            declared_kernel_pid: true,
            tsg: Some(2),
            ctx_share: 8,
            device: 9,
            error_notifier: None,
            process_id: Some(200),
            user_work: true,
            user_ring_permit: windows_user_ring_permit(&UserWorkFacts {
                flag: true,
                windows: true,
                kernel_stamped: true,
                engine: Some(kf_abi::submit::ENGINE_TYPE_GRAPHICS),
                ctx_share: 8,
                ctx_share_known: true,
                process_id: Some(200),
                kernel_pid: Some(100),
                ..Default::default()
            })
            .ok(),
        }
    }

    #[test]
    fn pending_windows_birth_free_reuse_and_old_commit_are_exact() {
        use crate::userbirth::Book;
        for object in [1, 2, 3, 9] {
            let q = Queue::<()>::default();
            let m = mirror();
            let a = channel_declaration();
            let book = Book::default();
            let old = book.allocate(&q, &a, m.clone()).unwrap();
            assert!(book.allocate(&q, &a, m.clone()).is_err());
            assert_eq!(book.cancel(20, object).unwrap(), 0);
            assert!(book.is_current(&old));
            assert_eq!(book.cancel(1, object).unwrap(), 1);
            let new = book.allocate(&q, &a, m.clone()).unwrap();
            assert!(!book.is_current(&old));
            assert!(book.is_current(&new));
            let ran = std::cell::Cell::new(false);
            assert!(
                book.commit(&old, |_| {
                    ran.set(true);
                    Ok(())
                })
                .is_err()
            );
            assert!(!ran.get());
            assert!(book.poll(&old).is_err());
            assert!(book.poll(&new).unwrap().is_none());
            assert!(
                book.commit(&new, |_| {
                    ran.set(true);
                    Ok(())
                })
                .is_err(),
                "cannot attach before lease handout"
            );
            assert!(!ran.get());
        }
    }

    #[test]
    fn pending_birth_handout_still_needs_an_exact_commit() {
        use crate::userbirth::Book;
        let q = Queue::<()>::default();
        let m = mirror();
        let a = channel_declaration();
        let book = Book::default();
        let birth = book.allocate(&q, &a, m.clone()).unwrap();
        let Request::Allocate { reply, .. } = q.pending.lock().unwrap().remove(0) else {
            panic!()
        };
        m.user_regions.claim(BASE).unwrap();
        let host = Fake::default();
        let lease = q
            .adopt(
                &mut q.held.lock().unwrap(),
                &m,
                BASE,
                reply,
                Storage::allocate(&host, m.space, BASE),
            )
            .unwrap();
        lease
            .initializing
            .store(0, std::sync::atomic::Ordering::Release);
        lease.reply.publish(Ok(Arc::downgrade(&lease)));
        assert!(Arc::ptr_eq(
            &book.poll(&birth).unwrap().unwrap().unwrap(),
            &lease
        ));
        assert_eq!(book.cancel(1, 3).unwrap(), 1);
        assert!(
            !q.reap_canceled(&lease, &host),
            "handout stays caller-owned"
        );
        assert!(book.commit(&birth, |_| Ok(())).is_err());
        assert_eq!(m.live.load(std::sync::atomic::Ordering::Acquire), 1);
        q.retire(&lease, &host); // caller confirms no native channel was born
    }

    #[test]
    fn committed_birth_identity_is_unique_and_orphan_drops_cancel_pending_storage() {
        use crate::userbirth::Book;
        let q = Queue::<()>::default();
        let m = mirror();
        let a = channel_declaration();
        let wakes = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let wake_counter = wakes.clone();
        let book = Book::with_wake(move || {
            wake_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        });
        let birth = book.allocate(&q, &a, m.clone()).unwrap();
        assert_eq!(wakes.load(std::sync::atomic::Ordering::Relaxed), 1);
        let Request::Allocate { reply, .. } = q.pending.lock().unwrap().remove(0) else {
            panic!()
        };
        drop(birth);
        assert_eq!(wakes.load(std::sync::atomic::Ordering::Relaxed), 2);
        assert!(Queue::abort_canceled_allocate(&m, &reply));
        assert_eq!(m.live.load(std::sync::atomic::Ordering::Acquire), 0);
        let birth = book.allocate(&q, &a, m.clone()).unwrap();
        let Request::Allocate { reply, .. } = q.pending.lock().unwrap().remove(0) else {
            panic!()
        };
        m.user_regions.claim(BASE).unwrap();
        let host = Fake::default();
        let lease = q
            .adopt(
                &mut q.held.lock().unwrap(),
                &m,
                BASE,
                reply,
                Storage::allocate(&host, m.space, BASE),
            )
            .unwrap();
        lease
            .initializing
            .store(0, std::sync::atomic::Ordering::Release);
        lease.reply.publish(Ok(Arc::downgrade(&lease)));
        book.poll(&birth).unwrap().unwrap().unwrap();
        let identity = std::cell::RefCell::new(None);
        book.commit(&birth, |allocation| {
            *identity.borrow_mut() = Some(allocation.clone());
            Ok(())
        })
        .unwrap();
        assert!(!book.is_current(&birth));
        assert_eq!(book.cancel(1, 3).unwrap(), 0);
        assert!(book.commit(&birth, |_| Ok(())).is_err());
        drop(birth);
        assert!(!q.reap_canceled(&lease, &host));
        assert!(identity.borrow().is_some());
        q.retire(&lease, &host);
    }

    #[test]
    fn pending_birth_without_windows_permit_allocates_nothing() {
        let q = Queue::<()>::default();
        let m = mirror();
        let mut a = channel_declaration();
        a.user_ring_permit = None;
        let book = crate::userbirth::Book::default();
        assert!(book.allocate(&q, &a, m.clone()).is_err());
        assert!(q.pending.lock().unwrap().is_empty());
        assert_eq!(m.live.load(std::sync::atomic::Ordering::Acquire), 0);
        assert!(m.user_regions.ranges().is_empty());
    }
}
