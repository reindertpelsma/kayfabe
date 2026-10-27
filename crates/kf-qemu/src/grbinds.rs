//! Bounded VA-worker GR action queue. First head remains explicitly opt-in.
use std::collections::VecDeque;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

pub(crate) type Reply = Arc<Mutex<Option<Result<u64, u32>>>>;
pub(crate) type Work<H> = Box<dyn FnMut(&H) -> Option<Result<u64, u32>> + Send>;
struct Job<H> {
    work: Work<H>,
    reply: Reply,
    wake: Box<dyn FnOnce() + Send>,
}

const MAX: usize = 256;

pub(crate) fn flag_on(value: Option<&str>) -> bool {
    value == Some("1")
}
pub(crate) fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| flag_on(std::env::var("KF3_GR_BINDS").ok().as_deref()))
}
pub(crate) struct Queue<H = kf_host::HostRm> {
    on: bool,
    jobs: Mutex<VecDeque<Job<H>>>,
    held: AtomicUsize,
    served: AtomicU64,
    refused: AtomicU64,
    bytes: AtomicU64,
}
impl<H> std::fmt::Debug for Queue<H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GrBindQueue")
    }
}
impl<H> Default for Queue<H> {
    fn default() -> Self {
        Self::new(enabled())
    }
}
impl<H> Queue<H> {
    fn new(on: bool) -> Self {
        Self {
            on,
            jobs: Default::default(),
            held: AtomicUsize::new(0),
            served: AtomicU64::new(0),
            refused: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
        }
    }
    pub(crate) fn enqueue(
        &self,
        work: Work<H>,
        reply: Reply,
        wake: impl FnOnce() + Send + 'static,
    ) -> Result<(), u32> {
        if !self.on {
            return Err(0x56);
        }
        let mut jobs = self.jobs.lock().map_err(|_| 0x40u32)?;
        if self.held.load(Ordering::Relaxed) >= MAX {
            self.refused.fetch_add(1, Ordering::Relaxed);
            return Err(0x51);
        }
        jobs.try_reserve(1).map_err(|_| 0x51u32)?;
        self.held.fetch_add(1, Ordering::Relaxed);
        jobs.push_back(Job {
            work,
            reply,
            wake: Box::new(wake),
        });
        Ok(())
    }
    pub(crate) fn has_work(&self) -> bool {
        self.held.load(Ordering::Relaxed) != 0
    }
    // No queue guard crosses a native call or completion callback.
    pub(crate) fn serve(&self, host: &H) {
        if !self.on {
            return;
        }
        let job = self.jobs.lock().ok().and_then(|mut q| q.pop_front());
        if let Some(mut job) = job {
            match (job.work)(host) {
                Some(result) => self.finish(job, result),
                None => {
                    // Admission credit includes this parked job, so it cannot overflow MAX.
                    if let Ok(mut q) = self.jobs.lock() {
                        q.push_back(job);
                    }
                }
            }
        }
    }
    pub(crate) fn reject(&self, status: u32) -> u32 {
        if self.on {
            self.refused.fetch_add(1, Ordering::Relaxed);
        }
        status
    }
    fn finish(&self, job: Job<H>, result: Result<u64, u32>) {
        match result {
            Ok(bytes) => {
                self.served.fetch_add(1, Ordering::Relaxed);
                self.bytes.fetch_add(bytes, Ordering::Relaxed);
            }
            Err(_) => {
                self.refused.fetch_add(1, Ordering::Relaxed);
            }
        }
        if let Ok(mut reply) = job.reply.lock() {
            *reply = Some(result);
        }
        drop(job.work);
        self.held.fetch_sub(1, Ordering::Relaxed);
        (job.wake)();
    }
    pub(crate) fn cancel_all(&self) {
        let jobs = self
            .jobs
            .lock()
            .map(|mut q| q.drain(..).collect::<Vec<_>>());
        if let Ok(jobs) = jobs {
            for job in jobs {
                self.finish(job, Err(0x40));
            }
        }
    }
    pub(crate) fn summary(&self) -> String {
        if !self.on {
            return String::new();
        }
        format!(
            " gr_binds[served={} refused={} bytes={}]",
            self.served.load(Ordering::Relaxed),
            self.refused.load(Ordering::Relaxed),
            self.bytes.load(Ordering::Relaxed)
        )
    }
}

/// A stream-ordered bind can target its triggering GR channel or its own TSG.
/// Other logical clients/contexts need a separate target-lifetime adapter.
pub(crate) fn eligible(
    request: kf_abi::grbind::Request,
    key: (u32, u32),
    tsg: Option<u32>,
    engine: u32,
    active: bool,
) -> bool {
    active
        && engine == kf_abi::submit::ENGINE_TYPE_GRAPHICS
        && request.client == key.0
        && (request.channel == key.1 || Some(request.channel) == tsg)
}

/// Snapshot under the private target lifetime guard, not guest-supplied authority.
pub(crate) struct Life<'a> {
    pub allocation: &'a Arc<()>,
    pub channel: Option<kf_host::Channel>,
    pub canceled: bool,
    pub retired: bool,
    pub free_attempted: bool,
    pub dead: bool,
}
pub(crate) fn same_live(allocation: &Arc<()>, channel: kf_host::Channel, now: Life<'_>) -> bool {
    !now.canceled
        && !now.retired
        && !now.free_attempted
        && !now.dead
        && Arc::ptr_eq(allocation, now.allocation)
        && now.channel.is_some_and(|c| {
            c.chan == channel.chan && c.tsg == channel.tsg && c.token == channel.token
        })
}

/// CODE READ (`writable`): bounded full writable coverage of host placement rows.
/// The VA worker serializes ordinary mappings; a steer claim excludes act-thread handovers.
pub(crate) fn writable(rows: &crate::mem::PlacedRows, va: u64, len: u64) -> bool {
    let Some(end) = va.checked_add(len).filter(|e| *e > va) else {
        return false;
    };
    let Ok(rows) = rows.try_read() else {
        return false;
    };
    let mut at = va;
    for _ in 0..32768 {
        let Some((&start, &(bytes, _, _, perm))) = rows.range(..=at).next_back() else {
            return false;
        };
        let Some(next) = start.checked_add(bytes).filter(|n| *n > at) else {
            return false;
        };
        if perm.read_only {
            return false;
        }
        at = next.min(end);
        if at == end {
            return true;
        }
    }
    false
}

/// Select the triggering channel only when its stream proves the requested scope.
pub(crate) fn prefer_trigger(
    request: kf_abi::grbind::Request,
    key: (u32, u32),
    tsg: Option<u32>,
    engine: u32,
    active: bool,
) -> bool {
    eligible(request, key, tsg, engine, active) && request.channel == key.1
}

/// Resolve at most two canonical matches; never guess a member of an ambiguous TSG.
pub(crate) fn unique<T>(candidates: impl IntoIterator<Item = T>) -> Result<Option<T>, u32> {
    let mut candidates = candidates.into_iter();
    let first = candidates.next();
    if candidates.next().is_some() {
        Err(0x56)
    } else {
        Ok(first)
    }
}

/// Preserve non-USER fallback while resolving the USER member of a canonical scope.
pub(crate) fn user_target<T>(
    candidates: impl IntoIterator<Item = (bool, T)>,
) -> Result<Option<T>, u32> {
    unique(
        candidates
            .into_iter()
            .filter(|(user, _)| *user)
            .map(|(_, target)| target),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_abi::grbind::{Kind, Request};
    use kf_host::{
        Channel,
        grbind::{Host, execute},
    };
    #[derive(Default)]
    struct Fake {
        binds: Vec<Request>,
        status: Option<u32>,
    }
    impl Host for Fake {
        fn properties(&mut self, _: u32) -> Result<(u64, u64), u32> {
            Ok((4096, 256))
        }
        fn bind(&mut self, request: Request) -> Result<(), u32> {
            self.binds.push(request);
            self.status.map_or(Ok(()), Err)
        }
    }
    fn request(gfxp: bool) -> Request {
        let mut buffers = [0; 9];
        buffers[0] = 0x10000;
        Request {
            client: 11,
            channel: 21,
            kind: if gfxp {
                Kind::Preemption {
                    flags: 2,
                    gfxp: 1,
                    cilp: 0,
                    buffers,
                }
            } else {
                Kind::Zcull {
                    va: 0x10000,
                    mode: 2,
                }
            },
        }
    }
    fn channel() -> Channel {
        Channel {
            tsg: 71,
            chan: 81,
            token: 91,
            born_user: None,
        }
    }
    fn work(gfxp: bool) -> Work<Mutex<Fake>> {
        Box::new(move |host| {
            Some(execute(
                &mut *host.lock().unwrap(),
                request(gfxp),
                61,
                channel(),
                |v, n| v == 0x10000 && n == 4096,
            ))
        })
    }
    #[test]
    fn queued_gfxp_action_binds_before_publishing_completion() {
        let q = Queue::new(true);
        let host = Mutex::new(Fake::default());
        let reply = Reply::default();
        q.enqueue(work(true), reply.clone(), || {}).unwrap();
        assert!(reply.lock().unwrap().is_none());
        assert!(host.lock().unwrap().binds.is_empty());
        q.serve(&host);
        assert_eq!(*reply.lock().unwrap(), Some(Ok(4096)));
        assert_eq!(host.lock().unwrap().binds[0].channel, 81);
        assert_eq!(q.served.load(Ordering::Relaxed), 1);
        assert_eq!(q.refused.load(Ordering::Relaxed), 0);
        assert_eq!(q.bytes.load(Ordering::Relaxed), 4096);
        assert_eq!(q.summary(), " gr_binds[served=1 refused=0 bytes=4096]");
    }
    #[test]
    fn queued_zcull_action_retains_va_and_credits_only_real_success() {
        let q = Queue::new(true);
        let host = Mutex::new(Fake::default());
        let reply = Reply::default();
        q.enqueue(work(false), reply.clone(), || {}).unwrap();
        q.serve(&host);
        assert_eq!(*reply.lock().unwrap(), Some(Ok(4096)));
        assert_eq!(host.lock().unwrap().binds[0].kind, request(false).kind);
        assert_eq!(q.served.load(Ordering::Relaxed), 1);
        assert_eq!(q.bytes.load(Ordering::Relaxed), 4096);
        assert!(!q.has_work());
        assert_eq!(q.summary(), " gr_binds[served=1 refused=0 bytes=4096]");
    }
    #[test]
    fn busy_lifetime_guard_retains_job_without_reply_or_native_bind() {
        let q = Queue::new(true);
        let host = Mutex::new(Fake::default());
        let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let check = ready.clone();
        let mut bind = work(true);
        let reply = Reply::default();
        q.enqueue(
            Box::new(move |h| {
                if check.load(Ordering::Acquire) {
                    bind(h)
                } else {
                    None
                }
            }),
            reply.clone(),
            || {},
        )
        .unwrap();
        q.serve(&host);
        assert!(reply.lock().unwrap().is_none());
        assert!(host.lock().unwrap().binds.is_empty());
        assert!(q.has_work());
        ready.store(true, Ordering::Release);
        q.serve(&host);
        assert_eq!(*reply.lock().unwrap(), Some(Ok(4096)));
    }
    #[test]
    fn pending_busy_action_does_not_open_the_suffix_or_wake_a_completion() {
        let q = Queue::new(true);
        let reply = Reply::default();
        let wakes = Arc::new(AtomicUsize::new(0));
        let count = wakes.clone();
        q.enqueue(Box::new(|_: &()| None), reply.clone(), move || {
            count.fetch_add(1, Ordering::Relaxed);
        })
        .unwrap();
        q.serve(&());
        q.serve(&());
        assert_eq!(wakes.load(Ordering::Relaxed), 0);
        assert!(reply.lock().unwrap().is_none());
        q.cancel_all();
    }
    #[test]
    fn native_refusal_is_not_a_served_bind_or_credited_bytes() {
        for status in [0x1a, 0x40] {
            let q = Queue::new(true);
            let host = Mutex::new(Fake {
                status: Some(status),
                ..Fake::default()
            });
            let reply = Reply::default();
            q.enqueue(work(true), reply.clone(), || {}).unwrap();
            q.serve(&host);
            assert_eq!(*reply.lock().unwrap(), Some(Err(status)));
            assert_eq!(q.served.load(Ordering::Relaxed), 0);
            assert_eq!(q.bytes.load(Ordering::Relaxed), 0);
            assert_eq!(q.refused.load(Ordering::Relaxed), 1);
            assert_eq!(q.summary(), " gr_binds[served=0 refused=1 bytes=0]");
        }
    }
    #[test]
    fn wake_observes_result_and_released_request_lifetime() {
        let q = Queue::new(true);
        let host = Mutex::new(Fake::default());
        let owner = Arc::new(());
        let weak = Arc::downgrade(&owner);
        let mut bind = work(false);
        let reply = Reply::default();
        let check = reply.clone();
        q.enqueue(
            Box::new(move |h| {
                let _keep = &owner;
                bind(h)
            }),
            reply,
            move || {
                assert_eq!(*check.lock().unwrap(), Some(Ok(4096)));
                assert!(weak.upgrade().is_none());
            },
        )
        .unwrap();
        q.serve(&host);
    }
    #[test]
    fn reset_cancels_pending_native_work_and_releases_held_metadata() {
        let q = Queue::new(true);
        let owner = Arc::new(());
        let weak = Arc::downgrade(&owner);
        let reply = Reply::default();
        q.enqueue(
            Box::new(move |_: &()| {
                let _keep = &owner;
                panic!("canceled job ran")
            }),
            reply.clone(),
            || {},
        )
        .unwrap();
        q.cancel_all();
        assert_eq!(*reply.lock().unwrap(), Some(Err(0x40)));
        assert!(weak.upgrade().is_none());
        assert!(!q.has_work());
        assert_eq!(q.refused.load(Ordering::Relaxed), 1);
        assert_eq!(q.bytes.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn off_and_explicit_zero_leave_the_host_event_sequence_unchanged() {
        for value in [None, Some("0"), Some("true"), Some("2")] {
            let q = Queue::new(flag_on(value));
            let reply = Reply::default();
            assert_eq!(
                q.enqueue(
                    Box::new(|_: &()| panic!("off performed a host action")),
                    reply.clone(),
                    || panic!("off woke completion")
                ),
                Err(0x56)
            );
            q.serve(&());
            assert!(reply.lock().unwrap().is_none());
            assert_eq!(q.summary(), "");
            assert!(!q.has_work());
        }
        assert!(flag_on(Some("1")));
    }
    #[test]
    fn queue_bound_counts_a_busy_action_without_releasing_its_credit() {
        let q = Queue::new(true);
        for _ in 0..MAX {
            q.enqueue(Box::new(|_: &()| None), Reply::default(), || {})
                .unwrap();
        }
        q.serve(&());
        assert_eq!(
            q.enqueue(Box::new(|_: &()| None), Reply::default(), || {}),
            Err(0x51)
        );
        q.cancel_all();
        assert!(!q.has_work());
    }
    #[test]
    fn writable_complete_span_preserves_all_adjacent_placement_rows() {
        let rows = crate::mem::PlacedRows::default();
        for at in [0x10000, 0x11000] {
            rows.write()
                .unwrap()
                .insert(at, (4096, at, false, kf_host::MapPerm::READ_WRITE));
        }
        let before = rows.read().unwrap().clone();
        assert!(writable(&rows, 0x10000, 8192));
        assert!(writable(&rows, 0x10800, 4096));
        assert_eq!(*rows.read().unwrap(), before);
        rows.write().unwrap().remove(&0x11000);
        assert!(!writable(&rows, 0x10000, 8192));
    }
    #[test]
    fn a_read_only_tail_or_missing_middle_is_not_a_writable_buffer() {
        let rows = crate::mem::PlacedRows::default();
        let ro = kf_host::MapPerm {
            read_only: true,
            ..kf_host::MapPerm::READ_WRITE
        };
        rows.write()
            .unwrap()
            .insert(0x10000, (4096, 0, false, kf_host::MapPerm::READ_WRITE));
        rows.write()
            .unwrap()
            .insert(0x11000, (4096, 4096, false, ro));
        assert!(!writable(&rows, 0x10000, 8192));
        assert!(!writable(&rows, u64::MAX - 10, 4096));
        assert!(!writable(&rows, 0x10000, 0));
        rows.write().unwrap().remove(&0x10000);
        assert!(!writable(&rows, 0x10000, 8192));
    }
    #[test]
    fn same_numeric_handle_in_a_foreign_client_is_not_the_target() {
        let r = request(true);
        assert!(eligible(r, (11, 21), None, 1, true));
        assert!(eligible(r, (11, 22), Some(21), 1, true));
        assert!(!eligible(r, (12, 21), None, 1, true));
        assert!(!eligible(r, (11, 22), None, 1, true));
        assert!(!eligible(r, (11, 21), None, 1, false));
    }
    #[test]
    fn zcull_scope_rejects_freed_wrong_engine_and_foreign_contexts() {
        let r = request(false);
        assert!(!eligible(r, (11, 21), None, 9, true));
        assert!(!eligible(r, (11, 21), None, 1, false));
        assert!(!eligible(r, (12, 21), Some(21), 1, true));
        assert!(!eligible(r, (11, 22), Some(23), 1, true));
    }
    #[test]
    fn disabling_gr_binds_does_not_call_a_queued_host_or_credit_results() {
        let q = Queue::new(false);
        let reply = Reply::default();
        let host = Mutex::new(Fake::default());
        assert_eq!(
            q.enqueue(work(true), reply, || panic!("disabled wake")),
            Err(0x56)
        );
        q.serve(&host);
        assert!(host.lock().unwrap().binds.is_empty());
        assert_eq!(q.served.load(Ordering::Relaxed), 0);
        assert_eq!(q.bytes.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn bind_exclusion_extends_without_locks_and_blocks_other_context_steering() {
        let regions = Arc::new(crate::userva::Regions::default());
        let claim = regions.steer(0x10000, 4096).unwrap();
        claim.extend(0x10800, 4096).unwrap();
        assert!(regions.steer(0x11000, 2048).is_err());
        claim.extend(0x20000, 4096).unwrap();
        assert!(regions.steer(0x20000, 4096).is_err());
        drop(claim);
        assert!(regions.steer(0x10000, 4096).is_ok());
        assert!(regions.steer(0x20000, 4096).is_ok());
    }
    #[test]
    fn extending_one_bind_cannot_steal_another_active_handover_span() {
        let regions = Arc::new(crate::userva::Regions::default());
        let first = regions.steer(0x10000, 4096).unwrap();
        let other = regions.steer(0x20000, 4096).unwrap();
        assert!(first.extend(0x20000, 4096).is_err());
        assert!(first.extend(u64::MAX - 1, 4096).is_err());
        drop(other);
        first.extend(0x20000, 4096).unwrap();
        assert!(regions.steer(0x20000, 4096).is_err());
    }
    #[test]
    fn sysmem_read_only_context_buffer_is_rejected_without_rewriting_rows() {
        let rows = crate::mem::PlacedRows::default();
        let ro = kf_host::MapPerm {
            read_only: true,
            ..kf_host::MapPerm::READ_WRITE
        };
        rows.write()
            .unwrap()
            .insert(0x10000, (4096, 8192, true, ro));
        let before = rows.read().unwrap().clone();
        assert!(!writable(&rows, 0x10000, 4096));
        assert_eq!(*rows.read().unwrap(), before);
    }
    #[test]
    fn refused_zcull_never_credits_success_or_bound_bytes() {
        let q = Queue::new(true);
        let host = Mutex::new(Fake {
            status: Some(0x1f),
            ..Fake::default()
        });
        let reply = Reply::default();
        q.enqueue(work(false), reply.clone(), || {}).unwrap();
        q.serve(&host);
        assert_eq!(*reply.lock().unwrap(), Some(Err(0x1f)));
        assert_eq!(q.served.load(Ordering::Relaxed), 0);
        assert_eq!(q.bytes.load(Ordering::Relaxed), 0);
        assert_eq!(q.refused.load(Ordering::Relaxed), 1);
        assert_eq!(q.summary(), " gr_binds[served=0 refused=1 bytes=0]");
    }
    #[test]
    fn cancel_wake_sees_refusal_and_released_target_metadata() {
        let q = Queue::new(true);
        let owner = Arc::new(());
        let weak = Arc::downgrade(&owner);
        let reply = Reply::default();
        let completed = reply.clone();
        q.enqueue(
            Box::new(move |_: &()| {
                let _keep = &owner;
                None
            }),
            reply,
            move || {
                assert_eq!(*completed.lock().unwrap(), Some(Err(0x40)));
                assert!(weak.upgrade().is_none());
            },
        )
        .unwrap();
        q.cancel_all();
    }
    #[test]
    fn a_running_callback_still_owns_admission_credit() {
        let q = Arc::new(Queue::new(true));
        let weak = Arc::downgrade(&q);
        q.enqueue(
            Box::new(move |_: &()| {
                assert_eq!(
                    weak.upgrade()
                        .unwrap()
                        .enqueue(Box::new(|_| None), Reply::default(), || {}),
                    Err(0x51)
                );
                None
            }),
            Reply::default(),
            || {},
        )
        .unwrap();
        for _ in 1..MAX {
            q.enqueue(Box::new(|_| None), Reply::default(), || {})
                .unwrap();
        }
        q.serve(&());
        q.cancel_all();
        assert!(!q.has_work());
    }
    fn closed_work(gfxp: bool, event: u8) -> Work<Mutex<Fake>> {
        let original = Arc::new(());
        let now = if event == 16 {
            Arc::new(())
        } else {
            original.clone()
        };
        let mut native = channel();
        if event == 32 {
            native.chan += 1;
        }
        Box::new(move |host| {
            let live = same_live(
                &original,
                channel(),
                Life {
                    allocation: &now,
                    channel: Some(native),
                    canceled: event == 1,
                    retired: event == 2,
                    free_attempted: event == 4,
                    dead: event == 8,
                },
            );
            Some(if live {
                execute(
                    &mut *host.lock().unwrap(),
                    request(gfxp),
                    61,
                    channel(),
                    |_, _| true,
                )
            } else {
                Err(0x40)
            })
        })
    }
    #[test]
    fn a_deferred_gfxp_request_cannot_bind_after_free_or_numeric_handle_reuse() {
        for event in [1, 2, 4, 8, 16, 32] {
            let q = Queue::new(true);
            let host = Mutex::new(Fake::default());
            let reply = Reply::default();
            q.enqueue(closed_work(true, event), reply.clone(), || {})
                .unwrap();
            q.serve(&host);
            assert_eq!(*reply.lock().unwrap(), Some(Err(0x40)));
            assert!(host.lock().unwrap().binds.is_empty());
            assert!(!q.has_work());
        }
    }
    #[test]
    fn a_held_direct_zcull_reply_observes_a_stale_target_refusal_without_host_io() {
        for event in [1, 2, 4, 8, 16, 32] {
            let q = Queue::new(true);
            let host = Mutex::new(Fake::default());
            let reply = Reply::default();
            let completed = reply.clone();
            let d = kf_gsp::Deferred::new();
            let done = d.clone();
            q.enqueue(closed_work(false, event), reply, move || {
                done.resolve(completed.lock().unwrap().unwrap().err().unwrap_or(0));
            })
            .unwrap();
            q.serve(&host);
            assert_eq!(d.outcome(), Some(0x40));
            assert!(host.lock().unwrap().binds.is_empty());
        }
    }
    #[test]
    fn only_literal_one_enables_the_initial_experimental_head() {
        assert!(!flag_on(None));
        assert!(!flag_on(Some("")));
        assert!(!flag_on(Some(" 1")));
        assert!(!flag_on(Some("2")));
        assert!(flag_on(Some("1")));
        assert!(!flag_on(Some("0")));
    }
    #[test]
    fn extending_into_a_partial_foreign_claim_never_releases_its_exclusion() {
        let regions = Arc::new(crate::userva::Regions::default());
        let bind = regions.steer(0x10000, 4096).unwrap();
        let other = regions.steer(0x20000, 4096).unwrap();
        assert!(bind.extend(0x1f800, 4096).is_err());
        drop(bind);
        assert!(regions.steer(0x20000, 4096).is_err());
        drop(other);
        assert!(regions.steer(0x20000, 4096).is_ok());
    }
    #[test]
    fn tsg_preemption_must_resolve_canonically_instead_of_picking_the_trigger() {
        let r = request(true);
        assert!(!prefer_trigger(r, (11, 22), Some(21), 1, true));
        assert!(prefer_trigger(r, (11, 21), Some(31), 1, true));
    }
    #[test]
    fn tsg_zcull_cannot_infer_a_share_from_its_triggering_stream() {
        let r = request(false);
        assert!(!prefer_trigger(r, (11, 22), Some(21), 1, true));
        assert!(prefer_trigger(r, (11, 21), Some(31), 1, true));
    }
    #[test]
    fn preemption_target_selection_refuses_two_members_but_accepts_one() {
        let r = request(true);
        let members = [(11, 22), (11, 23)];
        let matches = members
            .into_iter()
            .filter(|key| eligible(r, *key, Some(21), 1, true));
        assert_eq!(unique(matches), Err(0x56));
        assert_eq!(unique([(11, 22)]), Ok(Some((11, 22))));
        assert_eq!(unique::<(u32, u32)>([]), Ok(None));
    }
    #[test]
    fn zcull_target_selection_never_collapses_distinct_members_into_a_share() {
        let r = request(false);
        let members = [(11, 22), (11, 23)];
        let matches = members
            .into_iter()
            .filter(|key| eligible(r, *key, Some(21), 1, true));
        assert_eq!(unique(matches), Err(0x56));
        assert_eq!(unique([(11, 23)]), Ok(Some((11, 23))));
    }
    #[test]
    fn mixed_preemption_tsg_cannot_ignore_a_native_non_user_member() {
        let r = request(true);
        let members = [(true, (11, 22)), (false, (11, 23))];
        let candidates = members
            .into_iter()
            .filter(|(_, key)| eligible(r, *key, Some(21), 1, true));
        assert_eq!(user_target(candidates), Err(0x56));
    }
    #[test]
    fn mixed_zcull_tsg_cannot_pick_a_user_share_after_a_non_user_member() {
        let r = request(false);
        let members = [(false, (11, 22)), (true, (11, 23))];
        let candidates = members
            .into_iter()
            .filter(|(_, key)| eligible(r, *key, Some(21), 1, true));
        assert_eq!(user_target(candidates), Err(0x56));
    }
    #[test]
    fn non_user_preemption_target_still_declines_the_user_adapter() {
        let r = request(true);
        let members = [(false, (11, 22)), (true, (12, 23))];
        let candidates = members
            .into_iter()
            .filter(|(_, key)| eligible(r, *key, Some(21), 1, true));
        assert_eq!(user_target(candidates), Ok(None));
    }
    #[test]
    fn non_user_zcull_target_preserves_the_linux_fallback() {
        let r = request(false);
        let members = [(false, (11, 22), 1), (true, (11, 23), 9)];
        let candidates = members
            .into_iter()
            .filter(|(_, key, engine)| eligible(r, *key, Some(21), *engine, true))
            .map(|(user, key, _)| (user, key));
        assert_eq!(user_target(candidates), Ok(None));
    }
    fn busy_pair(first_gfxp: bool) {
        let q = Queue::new(true);
        let host = Mutex::new(Fake::default());
        let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let check = ready.clone();
        let mut first = work(first_gfxp);
        let a = Reply::default();
        let b = Reply::default();
        q.enqueue(
            Box::new(move |h| {
                if check.load(Ordering::Acquire) {
                    first(h)
                } else {
                    None
                }
            }),
            a.clone(),
            || {},
        )
        .unwrap();
        q.enqueue(work(!first_gfxp), b.clone(), || {}).unwrap();
        q.serve(&host);
        q.serve(&host);
        assert!(host.lock().unwrap().binds.is_empty());
        assert!(a.lock().unwrap().is_none());
        assert!(b.lock().unwrap().is_none());
        ready.store(true, Ordering::Release);
        q.serve(&host);
        q.serve(&host);
        assert_eq!(*a.lock().unwrap(), Some(Ok(4096)));
        assert_eq!(*b.lock().unwrap(), Some(Ok(4096)));
        let host = host.lock().unwrap();
        assert_eq!(host.binds[0].kind, request(first_gfxp).kind);
        assert_eq!(host.binds[1].kind, request(!first_gfxp).kind);
        assert!(!q.has_work());
    }
    #[test]
    fn busy_preemption_cannot_be_overtaken_by_zcull_on_the_same_context() {
        busy_pair(true);
    }
    #[test]
    fn busy_zcull_cannot_be_overtaken_by_preemption_on_the_same_context() {
        busy_pair(false);
    }
}
