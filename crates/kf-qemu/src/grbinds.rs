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
