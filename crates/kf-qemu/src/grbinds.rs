//! Bounded VA-worker GR action queue. First head remains explicitly opt-in.
use std::collections::VecDeque;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

pub(crate) type Reply = Arc<Mutex<Option<Result<u64, u32>>>>;
type Work<H> = Box<dyn FnMut(&H) -> Option<Result<u64, u32>> + Send>;
struct Job<H> {
    work: Work<H>,
    reply: Reply,
    wake: Box<dyn FnOnce() + Send>,
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
    // The baseline still refuses execution; RED tests precede the production callback dispatch.
    pub(crate) fn serve(&self, _host: &H) {
        let job = self.jobs.lock().ok().and_then(|mut q| q.pop_front());
        if let Some(job) = job {
            self.finish(job, Err(0x56));
        }
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
