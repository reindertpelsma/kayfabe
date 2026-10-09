//! ★★★ **The act thread as an event loop** — no sleep, no acknowledgement wait: a wait is a
//! continuation the loop resumes from `epoll` (`docs/design/V3_NONSTALL_THREADS.md` §3.C).
//!
//! Owner rule (2026-10-09): the act thread (`kf3-chan-act`) may never stall — every guest RPC whose
//! reply is a host act waits behind it. A **stall** is a sleep, a timed wait, an acknowledgement
//! wait, a contended blocking lock, a blocking write, an unbounded loop; a wait inside an `epoll`
//! that also accepts new requests is not one.
//!
//! ## Shape
//!
//! An act is a [`Job`]: a first [`Cont`]inuation and a `finish` callback. Running a continuation
//! returns a [`Step`]: `Done(result)`, or `Wait(wait, next)` — "resume `next` when `wait` is over":
//! a timer ([`Wait::Timer`], for a retry) or an eventfd ([`Wait::Fd`], for another thread's
//! acknowledgement, with a deadline so a lost one is reported instead of waited for). The loop
//! `epoll`s on its own submission eventfd and on the waited-for fd, so **a new act is ACCEPTED (read
//! from the channel, counted, queued) the moment it is submitted, even while another act waits**.
//!
//! ## Ordering — strict FIFO, deliberately
//!
//! One act is current at a time and the next starts only when it finished: acts take effect in
//! STATEMENT ORDER, which is the documented invariant (a statement's acts for one client / object /
//! channel must not be reordered). This loop therefore removes the *stall* (the thread is never
//! unresponsive, and a wait costs no thread) but NOT the head-of-line wait: an act queued behind a
//! waiting one starts after it. Running independent acts meanwhile needs a per-key independence
//! argument over the whole act set (shared guest chid heap, host groups keyed by client, VA mirrors
//! shared across clients, host RM's own global lock) — a proposal for the owner, §5 of the design
//! document, not built.
//!
//! ⊘ What this cannot remove: a *synchronous step* is still run to completion. A step that makes a
//! host RM ioctl blocks inside `nvidia.ko` for as long as host RM takes; `stall[act_run_max_us=]`
//! measures exactly that.

use crate::stall::{MaxGauge, Role, Stall, set_role};
use kf_linux_raw::{Notifier, PollTimeout, Poller, ReadyTokens};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

/// Poller tag of the submission eventfd.
const TAG_WAKE: u64 = 0;
/// Poller tag of the fd a [`Wait::Fd`] waits on.
const TAG_WAIT: u64 = 1;

/// What a continuation waits for.
pub enum Wait {
    /// A retry timer: resume after this long.
    Timer(Duration),
    /// Another thread's acknowledgement: resume when `fd` becomes readable (the loop drains it), or
    /// after `deadline` — the continuation then sees that its condition still does not hold and
    /// reports the timeout.
    Fd {
        /// The eventfd the other thread signals.
        fd: Arc<Notifier>,
        /// How long to wait for it at most.
        deadline: Duration,
    },
}

/// What running a continuation produced.
pub enum Step<C: ?Sized, T> {
    /// The act is finished with this result.
    Done(T),
    /// Resume `next` when the wait is over. The act thread serves other inputs meanwhile.
    Wait(Wait, Cont<C, T>),
}

/// A continuation: run on the act thread with the context, never blocking.
pub type Cont<C, T> = Box<dyn FnOnce(&C) -> Step<C, T> + Send>;

/// How an act's time was spent, handed to its `finish`.
#[derive(Debug, Clone, Copy)]
pub struct JobTimes {
    /// The act's label.
    pub label: &'static str,
    /// Submission to the first step.
    pub queue_wait: Duration,
    /// Submission to the result.
    pub total: Duration,
    /// Synchronous step time, summed (the time the thread was busy with it).
    pub run: Duration,
    /// Waits the act went through.
    pub waits: u32,
}

/// The callback that receives an act's result (resolves the held reply, logs, wakes the drainer).
pub type Finish<C, T> = Box<dyn FnOnce(&C, T, JobTimes) + Send>;

/// One queued act.
pub struct Job<C: ?Sized, T> {
    label: &'static str,
    step: Cont<C, T>,
    finish: Finish<C, T>,
    queued: Instant,
}

/// Counters of the loop (relaxed atomics; the status line reads them).
#[derive(Debug, Default)]
pub struct ActStats {
    /// Acts read from the submission channel (accepted).
    pub accepted: AtomicU64,
    /// Acts finished.
    pub finished: AtomicU64,
    /// Waits entered (timers and acknowledgements).
    pub waits: AtomicU64,
    /// Acts queued or current now.
    pub depth: AtomicU64,
    /// The largest `depth` seen.
    pub depth_max: AtomicU64,
    /// Time spent in each wait (a head-of-line delay for the acts behind, not a stall).
    pub wait_time: MaxGauge,
}

/// The submitting half: any thread, never blocks (an unbounded channel and one eventfd write).
pub struct ActQueue<C: ?Sized, T> {
    tx: mpsc::Sender<Job<C, T>>,
    wake: Arc<Notifier>,
}

impl<C: ?Sized, T> Clone for ActQueue<C, T> {
    fn clone(&self) -> Self {
        ActQueue {
            tx: self.tx.clone(),
            wake: Arc::clone(&self.wake),
        }
    }
}

impl<C: ?Sized, T> ActQueue<C, T> {
    /// Queue an act. `false`: the loop is gone.
    pub fn submit(&self, label: &'static str, step: Cont<C, T>, finish: Finish<C, T>) -> bool {
        let ok = self
            .tx
            .send(Job {
                label,
                step,
                finish,
                queued: Instant::now(),
            })
            .is_ok();
        let _ = self.wake.signal();
        ok
    }

    /// Wake the loop without a job (a stop request).
    pub fn poke(&self) {
        let _ = self.wake.signal();
    }
}

/// The loop's half.
pub struct ActLoop<C: ?Sized, T> {
    rx: mpsc::Receiver<Job<C, T>>,
    wake: Arc<Notifier>,
    /// Counters.
    pub stats: Arc<ActStats>,
}

/// The act being waited on.
struct Waiting<C: ?Sized, T> {
    label: &'static str,
    finish: Finish<C, T>,
    queued: Instant,
    first_run: Instant,
    run: Duration,
    waits: u32,
    cont: Option<Cont<C, T>>,
    wait: Wait,
    since: Instant,
    watched: bool,
}

/// A queue and its loop, as [`channel`] returns them.
pub type Channel<C, T> = (ActQueue<C, T>, ActLoop<C, T>);

/// A fresh queue and its loop.
///
/// # Errors
/// The submission eventfd could not be created.
pub fn channel<C: ?Sized, T>() -> Result<Channel<C, T>, String> {
    let (tx, rx) = mpsc::channel();
    let wake = Arc::new(Notifier::create().map_err(|e| format!("act eventfd: {e:?}"))?);
    Ok((
        ActQueue {
            tx,
            wake: Arc::clone(&wake),
        },
        ActLoop {
            rx,
            wake,
            stats: Arc::new(ActStats::default()),
        },
    ))
}

impl<C: ?Sized, T> ActLoop<C, T> {
    fn accept(&self, queue: &mut VecDeque<Job<C, T>>) {
        while let Ok(j) = self.rx.try_recv() {
            self.stats.accepted.fetch_add(1, Ordering::Relaxed);
            queue.push_back(j);
        }
    }

    fn depth(&self, n: usize) {
        let n = n as u64;
        self.stats.depth.store(n, Ordering::Relaxed);
        self.stats.depth_max.fetch_max(n, Ordering::Relaxed);
    }

    /// Run until `stop` is set and the loop is poked. `stall` receives the act gauges.
    pub fn run(self, ctx: &C, stop: &AtomicBool, stall: &Stall) {
        set_role(Role::Act);
        let Ok(poller) = Poller::create() else {
            return;
        };
        if poller.watch(self.wake.as_source_fd(), TAG_WAKE).is_err() {
            return;
        }
        let mut queue: VecDeque<Job<C, T>> = VecDeque::new();
        let mut cur: Option<Waiting<C, T>> = None;
        while !stop.load(Ordering::Acquire) {
            self.accept(&mut queue);
            self.depth(queue.len() + usize::from(cur.is_some()));
            let Some(mut w) = cur.take() else {
                // Nothing current: start the next act, or wait for one.
                if let Some(job) = queue.pop_front() {
                    cur = self.start(ctx, job, stall);
                    self.depth(queue.len() + usize::from(cur.is_some()));
                } else {
                    let mut ready = ReadyTokens::new();
                    let _ = poller.wait(&mut ready, PollTimeout::Blocking);
                    let _ = self.wake.drain();
                }
                continue;
            };
            // An act is waiting: epoll on the submission fd (a new act is accepted at the loop top)
            // and on what it waits for, until the wait is over.
            let (fd, left) = match &w.wait {
                Wait::Timer(d) => (None, d.saturating_sub(w.since.elapsed())),
                Wait::Fd { fd, deadline } => (
                    Some(Arc::clone(fd)),
                    deadline.saturating_sub(w.since.elapsed()),
                ),
            };
            if let Some(fd) = &fd
                && !w.watched
            {
                w.watched = poller.watch(fd.as_source_fd(), TAG_WAIT).is_ok();
            }
            let mut over = left.is_zero();
            if !over {
                let ms = u32::try_from(left.as_millis() + 1).unwrap_or(u32::MAX);
                let mut ready = ReadyTokens::new();
                let _ = poller.wait(&mut ready, PollTimeout::Millis(ms));
                if ready.iter().any(|t| t == TAG_WAKE) {
                    let _ = self.wake.drain();
                }
                over = ready.iter().any(|t| t == TAG_WAIT)
                    || w.since.elapsed()
                        >= match &w.wait {
                            Wait::Timer(d) => *d,
                            Wait::Fd { deadline, .. } => *deadline,
                        };
            }
            if !over {
                cur = Some(w);
                continue;
            }
            if let Some(fd) = &fd {
                if w.watched {
                    let _ = poller.unwatch(fd.as_source_fd());
                }
                let _ = fd.drain();
            }
            self.stats.wait_time.since(w.since);
            cur = self.resume(ctx, w, stall);
        }
    }

    fn start(&self, ctx: &C, job: Job<C, T>, stall: &Stall) -> Option<Waiting<C, T>> {
        let t0 = Instant::now();
        stall
            .act_queue_wait
            .record(t0.saturating_duration_since(job.queued));
        let w = Waiting {
            label: job.label,
            finish: job.finish,
            queued: job.queued,
            first_run: t0,
            run: Duration::ZERO,
            waits: 0,
            cont: None,
            wait: Wait::Timer(Duration::ZERO),
            since: t0,
            watched: false,
        };
        self.step(ctx, w, job.step, stall)
    }

    fn resume(&self, ctx: &C, mut w: Waiting<C, T>, stall: &Stall) -> Option<Waiting<C, T>> {
        let cont = w.cont.take()?;
        self.step(ctx, w, cont, stall)
    }

    /// Run one continuation; finish the act or park it on its next wait.
    fn step(
        &self,
        ctx: &C,
        mut w: Waiting<C, T>,
        cont: Cont<C, T>,
        stall: &Stall,
    ) -> Option<Waiting<C, T>> {
        let t0 = Instant::now();
        let r = cont(ctx);
        let d = t0.elapsed();
        stall.act_run.record(d);
        w.run += d;
        match r {
            Step::Done(t) => {
                let total = w.queued.elapsed();
                stall.act_total.record(total);
                self.stats.finished.fetch_add(1, Ordering::Relaxed);
                (w.finish)(
                    ctx,
                    t,
                    JobTimes {
                        label: w.label,
                        queue_wait: w.first_run.saturating_duration_since(w.queued),
                        total,
                        run: w.run,
                        waits: w.waits,
                    },
                );
                None
            }
            Step::Wait(wait, next) => {
                self.stats.waits.fetch_add(1, Ordering::Relaxed);
                w.waits += 1;
                w.wait = wait;
                w.cont = Some(next);
                w.since = Instant::now();
                w.watched = false;
                Some(w)
            }
        }
    }
}

/// ★ A bounded retry as continuations: call `check` now; if it holds, run `ready`; otherwise wait
/// `every` and try again, at most `tries` more times, then run `give_up`. The act thread's
/// replacement for `for _ in 0..n { if ok { break } sleep(every) }`.
pub fn retry<C: ?Sized + 'static, T: 'static>(
    ctx: &C,
    every: Duration,
    tries: u32,
    check: Box<dyn FnMut(&C) -> bool + Send>,
    ready: Cont<C, T>,
    give_up: Cont<C, T>,
) -> Step<C, T> {
    let mut check = check;
    if check(ctx) {
        return ready(ctx);
    }
    if tries == 0 {
        return give_up(ctx);
    }
    Step::Wait(
        Wait::Timer(every),
        Box::new(move |c: &C| retry(c, every, tries - 1, check, ready, give_up)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Ctx {
        log: Mutex<Vec<String>>,
    }

    type Out = String;

    type Rig = (
        ActQueue<Ctx, Out>,
        Arc<ActStats>,
        Arc<AtomicBool>,
        Arc<Ctx>,
        &'static Stall,
        std::thread::JoinHandle<()>,
    );

    fn spawn() -> Rig {
        let (q, l) = channel::<Ctx, Out>().unwrap();
        let stats = Arc::clone(&l.stats);
        let stop = Arc::new(AtomicBool::new(false));
        let ctx = Arc::new(Ctx {
            log: Mutex::new(Vec::new()),
        });
        let stall = Stall::leak();
        let (s2, c2) = (Arc::clone(&stop), Arc::clone(&ctx));
        let h = std::thread::spawn(move || l.run(&*c2, &s2, stall));
        (q, stats, stop, ctx, stall, h)
    }

    fn finish() -> Finish<Ctx, Out> {
        Box::new(|c: &Ctx, t: Out, _| c.log.lock().unwrap().push(format!("done:{t}")))
    }

    fn done(name: &'static str) -> Cont<Ctx, Out> {
        Box::new(move |c: &Ctx| {
            c.log.lock().unwrap().push(format!("ran:{name}"));
            Step::Done(name.to_string())
        })
    }

    fn until(f: impl Fn() -> bool, ms: u64) -> bool {
        let t = Instant::now();
        while t.elapsed() < Duration::from_millis(ms) {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        f()
    }

    fn stop(q: &ActQueue<Ctx, Out>, stop: &AtomicBool, h: std::thread::JoinHandle<()>) {
        stop.store(true, Ordering::Release);
        q.poke();
        h.join().unwrap();
    }

    /// ★ FALSIFIER of the act-thread stall (C): the first act is mid-retire (waiting on a 400 ms
    /// timer); a second is submitted. It is ACCEPTED within a few ms — the loop is in `epoll`, not
    /// asleep — and runs, in order, after the first. (The old retire slept in a loop: nothing was
    /// read from the queue for the whole wait — see the control below.)
    #[test]
    fn a_second_act_is_accepted_within_milliseconds_while_the_first_waits_on_a_timer() {
        let (q, stats, stop_flag, ctx, _st, h) = spawn();
        let first: Cont<Ctx, Out> = Box::new(|_| {
            Step::Wait(
                Wait::Timer(Duration::from_millis(400)),
                Box::new(|c: &Ctx| {
                    c.log.lock().unwrap().push("ran:first".into());
                    Step::Done("first".into())
                }),
            )
        });
        assert!(q.submit("first", first, finish()));
        assert!(until(|| stats.waits.load(Ordering::Relaxed) == 1, 500));
        let t = Instant::now();
        assert!(q.submit("second", done("second"), finish()));
        assert!(
            until(|| stats.accepted.load(Ordering::Relaxed) == 2, 100),
            "the second act was not accepted while the first waited"
        );
        let accepted_in = t.elapsed();
        assert!(
            accepted_in < Duration::from_millis(100),
            "accepted after {accepted_in:?}"
        );
        assert_eq!(
            stats.finished.load(Ordering::Relaxed),
            0,
            "strict FIFO: the first act is still waiting, so the second has not run"
        );
        assert_eq!(stats.depth.load(Ordering::Relaxed), 2);
        assert!(until(|| stats.finished.load(Ordering::Relaxed) == 2, 2000));
        assert_eq!(
            *ctx.log.lock().unwrap(),
            vec!["ran:first", "done:first", "ran:second", "done:second"],
            "statement order is preserved"
        );
        stop(&q, &stop_flag, h);
    }

    /// The control, i.e. the old behaviour: an act that SLEEPS inside its step (what `retire` did,
    /// 200 × 1 ms) accepts nothing for as long as it sleeps. This is the shape the test above fails
    /// on, and the reason waits are continuations.
    #[test]
    fn control_a_step_that_sleeps_accepts_nothing_meanwhile() {
        let (q, stats, stop_flag, _ctx, st, h) = spawn();
        let sleeper: Cont<Ctx, Out> = Box::new(|_| {
            std::thread::sleep(Duration::from_millis(300));
            Step::Done("slept".into())
        });
        assert!(q.submit("sleeper", sleeper, finish()));
        std::thread::sleep(Duration::from_millis(50));
        assert!(q.submit("next", done("next"), finish()));
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            stats.accepted.load(Ordering::Relaxed),
            1,
            "while the step sleeps the second act is not even read"
        );
        assert!(until(|| stats.finished.load(Ordering::Relaxed) == 2, 2000));
        assert!(
            st.act_run.max_ns() >= 250_000_000,
            "act_run_max_us shows the sleep: {} ns",
            st.act_run.max_ns()
        );
        stop(&q, &stop_flag, h);
    }

    /// An acknowledgement wait resumes as soon as the other thread signals, long before its deadline,
    /// and a lost acknowledgement resumes at the deadline.
    #[test]
    fn an_ack_wait_resumes_on_the_signal_and_at_the_deadline_when_none_comes() {
        let (q, stats, stop_flag, ctx, _st, h) = spawn();
        let fd = Arc::new(Notifier::create().unwrap());
        let (fd2, order) = (
            Arc::clone(&fd),
            Arc::new(Mutex::new(Vec::<&'static str>::new())),
        );
        let o2 = Arc::clone(&order);
        let ack: Cont<Ctx, Out> = Box::new(move |_| {
            Step::Wait(
                Wait::Fd {
                    fd: fd2,
                    deadline: Duration::from_secs(10),
                },
                Box::new(move |_| {
                    o2.lock().unwrap().push("acked");
                    Step::Done("ack".into())
                }),
            )
        });
        assert!(q.submit("ack", ack, finish()));
        assert!(until(|| stats.waits.load(Ordering::Relaxed) == 1, 500));
        let t = Instant::now();
        let s = std::thread::spawn({
            let fd = Arc::clone(&fd);
            move || {
                std::thread::sleep(Duration::from_millis(30));
                fd.signal().unwrap();
            }
        });
        assert!(until(|| stats.finished.load(Ordering::Relaxed) == 1, 2000));
        s.join().unwrap();
        assert!(
            t.elapsed() < Duration::from_secs(1),
            "resumed on the signal"
        );
        // A lost ack: resumes at the deadline.
        let lost = Arc::new(Notifier::create().unwrap());
        let l2 = Arc::clone(&lost);
        let never: Cont<Ctx, Out> = Box::new(move |_| {
            Step::Wait(
                Wait::Fd {
                    fd: l2,
                    deadline: Duration::from_millis(60),
                },
                Box::new(|_| Step::Done("timed out".into())),
            )
        });
        let t = Instant::now();
        assert!(q.submit("never", never, finish()));
        assert!(until(|| stats.finished.load(Ordering::Relaxed) == 2, 2000));
        let e = t.elapsed();
        assert!(
            e >= Duration::from_millis(55) && e < Duration::from_secs(1),
            "{e:?}"
        );
        assert_eq!(*order.lock().unwrap(), vec!["acked"]);
        assert!(
            ctx.log
                .lock()
                .unwrap()
                .contains(&"done:timed out".to_string())
        );
        stop(&q, &stop_flag, h);
    }

    /// The bounded retry: ready on the n-th try, no thread sleeps, and a give-up when it never holds.
    #[test]
    fn retry_waits_on_timers_and_gives_up_after_its_tries() {
        let (q, stats, stop_flag, ctx, st, h) = spawn();
        let n = Arc::new(Mutex::new(0u32));
        let n2 = Arc::clone(&n);
        let job: Cont<Ctx, Out> = Box::new(move |c: &Ctx| {
            retry(
                c,
                Duration::from_millis(2),
                50,
                Box::new(move |_| {
                    let mut k = n2.lock().unwrap();
                    *k += 1;
                    *k >= 4
                }),
                done("ready"),
                done("gave up"),
            )
        });
        assert!(q.submit("retry-ok", job, finish()));
        let job2: Cont<Ctx, Out> = Box::new(|c: &Ctx| {
            retry(
                c,
                Duration::from_millis(1),
                3,
                Box::new(|_| false),
                done("never ready"),
                done("gave up"),
            )
        });
        assert!(q.submit("retry-never", job2, finish()));
        assert!(until(|| stats.finished.load(Ordering::Relaxed) == 2, 2000));
        assert_eq!(*n.lock().unwrap(), 4);
        let log = ctx.log.lock().unwrap().clone();
        assert!(log.contains(&"ran:ready".to_string()), "{log:?}");
        assert!(log.contains(&"ran:gave up".to_string()), "{log:?}");
        assert_eq!(stats.waits.load(Ordering::Relaxed), 3 + 3);
        assert!(
            st.act_run.max_ns() < 50_000_000,
            "every step was short: {} ns",
            st.act_run.max_ns()
        );
        stop(&q, &stop_flag, h);
    }

    /// A stop request ends the loop even while an act waits.
    #[test]
    fn stop_ends_the_loop_while_an_act_waits() {
        let (q, stats, stop_flag, _ctx, _st, h) = spawn();
        let long: Cont<Ctx, Out> = Box::new(|_| {
            Step::Wait(
                Wait::Timer(Duration::from_secs(60)),
                Box::new(|_| Step::Done("late".into())),
            )
        });
        assert!(q.submit("long", long, finish()));
        assert!(until(|| stats.waits.load(Ordering::Relaxed) == 1, 500));
        let t = Instant::now();
        stop(&q, &stop_flag, h);
        assert!(t.elapsed() < Duration::from_secs(2));
    }
}
