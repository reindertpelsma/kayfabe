//! ★★ The real reactor loop — `l1_os_shell.md` §3, decision 8.
//!
//! ```text
//!   wait ──► for each ready token ──► resolve ──► drain ──► push N SourceSignals ──► wake
//!                                                                    │
//!                                              (no core state on this thread — law 9)
//! ```
//!
//! ## ★★ The F1 gate is a QUANTITY, and that is the whole design
//!
//! §3.4's gate is *"as an assert rather than a hope"*: the loop counts its own wakes and a
//! test drives K signals and asserts the loop woke about K times. The reason it must be a
//! quantity and not an absence is the sharp part:
//!
//! > A level-triggered descriptor that never stops being readable **fails a wake-count
//! > assertion loudly**, while it satisfies any absence-shaped rule ("the loop must not
//! > busy-spin") **vacuously** — an infinite loop that never asserts anything is, formally,
//! > not doing the thing the absence forbids.
//!
//! So this loop keeps four counters and one invariant over them:
//!
//! | counter | meaning |
//! |---|---|
//! | [`ReactorStats::wakes`] | waits that came back with at least one ready token |
//! | [`ReactorStats::ready_reports`] | tokens reported, summed |
//! | [`ReactorStats::signals_pushed`] | `SourceSignal`s put on the inbox |
//! | [`ReactorStats::undrained_reports`] | ready tokens that yielded **nothing** |
//!
//! > **The invariant: `signals_pushed` equals the number of signals the producers sent,
//! > exactly** — not "at least", not "roughly". A counter coalesces (N writes then one read
//! > returns N) and the loop pushes that many, so coalescing changes `wakes` and never
//! > `signals_pushed`. `wakes ≤ signals_sent` is therefore a *bound* a spin blows through,
//! > and `signals_pushed == signals_sent` is an *equality* that a lost or duplicated wakeup
//! > breaks. Two different failure modes, two different assertions, neither about a clock.
//!
//! And the loop refuses rather than spins: [`MAX_UNPRODUCTIVE_STREAK`] consecutive waits
//! that reported readiness and produced nothing is [`ReactorFault::UndrainableSource`] —
//! loud, bounded, and with the offending token named.
//!
//! ## Law 9, structurally
//!
//! The loop holds a [`Registrar`] (handle → descriptor), an `InboxSender`, an
//! [`ExecutorWaker`], and counters. It holds **no** reference to the device. See
//! [`crate::sources`] for why the table is here at all, which is a correction to §3.2.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use kayfabe_core::reactor::CompletionSource;
use kayfabe_linux_raw::{Notifier, PollTimeout, Poller, ReadyTokens};
use kayfabe_rt::executor::ExecutorWaker;
use kayfabe_rt::inbox::{CoreEvent, InboxSender};

use crate::sources::{Registrar, Resolved, SourceShape};

/// The token the loop's own control source is watched under.
///
/// `u64::MAX` is safe to reserve because `CompletionSource`'s mint starts at zero and is
/// never recycled: reaching `u64::MAX` would require arming 2^64 sources, and the mint
/// panics on overflow before it could ever collide.
pub const CONTROL_TOKEN: u64 = u64::MAX;

/// How many consecutive unproductive waits the loop tolerates before refusing.
///
/// A **small** number on purpose. One is too few — a terminal source legitimately reports
/// once more between the loop firing it and the `epoll_ctl` that stops it, and a batch may
/// contain two reports for the same token. Anything large turns a loud refusal into a
/// latency bug that only shows up under load.
pub const MAX_UNPRODUCTIVE_STREAK: u64 = 16;

/// Why the loop stopped early.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReactorFault {
    /// ★★ **The F1 gate firing.** A ready token produced no work
    /// [`MAX_UNPRODUCTIVE_STREAK`] waits in a row: the readiness is level-triggered,
    /// nothing the loop can do clears it, and continuing would be a busy-poll.
    ///
    /// Every source class this system arms is either a counter the loop drains or a
    /// terminal fact the loop unwatches, so reaching this means a source of a *third*
    /// shape was armed — [`SourceShape::Foreign`], the one §3.5 rejected.
    UndrainableSource {
        /// The token that would not stop reporting.
        token: u64,
        /// How many waits in a row it produced nothing.
        streak: u64,
    },
    /// The host refused a readiness operation.
    Host(kayfabe_linux_raw::RawError),
}

/// Everything the loop observed about itself. Every field is an instrument and every
/// instrument has a stated non-vacuity condition — the §14.8 discipline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReactorStats {
    /// Waits that returned at least one ready token. **The F1 quantity.** Non-vacuous only
    /// when `> 0`; bounded above by the number of signals sent plus the control wakes.
    pub wakes: u64,
    /// Waits that returned nothing (a timeout). Not a wake.
    pub idle_waits: u64,
    /// Ready tokens reported, summed over all waits. `wakes ≤ ready_reports` always.
    pub ready_reports: u64,
    /// `SourceSignal`s pushed onto the executor inbox. **Must equal the number of signals
    /// the producers sent** — coalescing moves `wakes`, never this.
    pub signals_pushed: u64,
    /// Control-source reports (a wake request, injected work, or shutdown).
    pub control_reports: u64,
    /// Terminal sources that fired — each exactly once, each then unwatched.
    pub terminal_fired: u64,
    /// Ready tokens that produced no signal at all. The numerator of the F1 refusal.
    pub undrained_reports: u64,
    /// Reports naming a handle that was disarmed and is no longer remembered. Counted, not
    /// faulted: the report resolves to nothing, which is the designed answer.
    pub stale_reports: u64,
    /// Reports naming a **retired but remembered** handle — pushed anyway, so the core's
    /// dispatch answers `SourceFault` (§3.3's designed path, exercised rather than assumed).
    pub retired_reports: u64,
}

#[derive(Debug, Default)]
struct Counters {
    wakes: AtomicU64,
    idle_waits: AtomicU64,
    ready_reports: AtomicU64,
    signals_pushed: AtomicU64,
    control_reports: AtomicU64,
    terminal_fired: AtomicU64,
    undrained_reports: AtomicU64,
    stale_reports: AtomicU64,
    retired_reports: AtomicU64,
}

impl Counters {
    fn snapshot(&self) -> ReactorStats {
        let g = |a: &AtomicU64| a.load(Ordering::SeqCst);
        ReactorStats {
            wakes: g(&self.wakes),
            idle_waits: g(&self.idle_waits),
            ready_reports: g(&self.ready_reports),
            signals_pushed: g(&self.signals_pushed),
            control_reports: g(&self.control_reports),
            terminal_fired: g(&self.terminal_fired),
            undrained_reports: g(&self.undrained_reports),
            stale_reports: g(&self.stale_reports),
            retired_reports: g(&self.retired_reports),
        }
    }
}

/// The half of the reactor everyone but the loop thread holds: arm, disarm, wake, stop,
/// and read the instruments.
///
/// Cloneable and `Send + Sync`; it holds no core state either.
#[derive(Debug, Clone)]
pub struct ReactorHandle {
    registrar: Arc<Registrar>,
    control: Arc<Notifier>,
    stop: Arc<AtomicBool>,
    counters: Arc<Counters>,
}

impl ReactorHandle {
    /// The source registrar (arm / disarm / the descriptor budget).
    #[must_use]
    pub fn registrar(&self) -> &Arc<Registrar> {
        &self.registrar
    }

    /// ★ Signal the loop. §3.6 amends what this *means*: with a kernel-side readiness set,
    /// adding a source needs **no** wake at all (registration from another thread is
    /// immediately effective against a concurrent wait). The wake's real jobs are narrower
    /// and are the reason it still exists:
    ///
    /// 1. a **removal barrier** — make the loop finish its current batch so a deregistered
    ///    descriptor can be closed safely;
    /// 2. **injected work** and shutdown;
    /// 3. **timer re-arm**, when that lands.
    ///
    /// # Errors
    /// [`kayfabe_linux_raw::RawError`] if the control descriptor refuses the write, which
    /// for a non-blocking counter means it saturated.
    ///
    /// # Panics
    /// If called with any ranked lock held (R1) — use the `*_under_lock` door if a core
    /// path ever needs to wake from beneath one.
    pub fn wake(&self) -> Result<(), kayfabe_linux_raw::RawError> {
        self.control.signal()
    }

    /// Ask the loop to stop and wake it. Idempotent; the loop finishes its current batch.
    ///
    /// # Errors
    /// As [`ReactorHandle::wake`].
    ///
    /// # Panics
    /// If called with any ranked lock held (R1).
    pub fn shutdown(&self) -> Result<(), kayfabe_linux_raw::RawError> {
        self.stop.store(true, Ordering::Release);
        self.control.signal()
    }

    /// The instruments.
    #[must_use]
    pub fn stats(&self) -> ReactorStats {
        self.counters.snapshot()
    }
}

/// The loop itself. Owned by exactly one thread; everything else uses a [`ReactorHandle`].
pub struct Reactor {
    registrar: Arc<Registrar>,
    poller: Arc<Poller>,
    control: Arc<Notifier>,
    stop: Arc<AtomicBool>,
    counters: Arc<Counters>,
    inbox: InboxSender,
    waker: Arc<dyn ExecutorWaker>,
    ready: ReadyTokens,
    /// Consecutive unproductive waits, and which token drove them.
    streak: u64,
    streak_token: u64,
}

impl core::fmt::Debug for Reactor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Reactor")
            .field("armed", &self.registrar.armed())
            .field("stats", &self.counters.snapshot())
            .finish_non_exhaustive()
    }
}

impl Reactor {
    /// Build a reactor over `registrar`, pushing into `inbox` and waking `waker`.
    ///
    /// # Errors
    /// [`ReactorFault::Host`] if the control source cannot be created or watched.
    ///
    /// # Panics
    /// If called with any ranked lock held (R1).
    pub fn new(
        registrar: Arc<Registrar>,
        inbox: InboxSender,
        waker: Arc<dyn ExecutorWaker>,
    ) -> Result<(Self, ReactorHandle), ReactorFault> {
        let poller = Arc::clone(registrar.poller());
        let control = Arc::new(Notifier::create().map_err(ReactorFault::Host)?);
        poller
            .watch(control.as_source_fd(), CONTROL_TOKEN)
            .map_err(ReactorFault::Host)?;
        let counters = Arc::new(Counters::default());
        let stop = Arc::new(AtomicBool::new(false));
        let handle = ReactorHandle {
            registrar: Arc::clone(&registrar),
            control: Arc::clone(&control),
            stop: Arc::clone(&stop),
            counters: Arc::clone(&counters),
        };
        Ok((
            Reactor {
                registrar,
                poller,
                control,
                stop,
                counters,
                inbox,
                waker,
                ready: ReadyTokens::new(),
                streak: 0,
                streak_token: CONTROL_TOKEN,
            },
            handle,
        ))
    }

    /// The instruments (the loop thread's own view).
    #[must_use]
    pub fn stats(&self) -> ReactorStats {
        self.counters.snapshot()
    }

    /// Run until shutdown is requested, blocking in the wait between batches.
    ///
    /// # Errors
    /// [`ReactorFault`] — the loop stops rather than spinning or guessing.
    ///
    /// # Panics
    /// If entered with any ranked lock or any adapter leaf lock held (R1); the raw layer
    /// asserts both at the wait itself.
    pub fn run(&mut self) -> Result<(), ReactorFault> {
        self.run_with(PollTimeout::Blocking, u64::MAX)
    }

    /// Run for at most `max_waits` waits, each bounded by `timeout`.
    ///
    /// The bounded form exists for tests, and the bound is a **count of waits**, never a
    /// wall-clock deadline: `testing_doctrine.md`'s rule is to measure the structure.
    ///
    /// # Errors
    /// [`ReactorFault`].
    ///
    /// # Panics
    /// If entered with any ranked lock or any adapter leaf lock held (R1).
    pub fn run_with(&mut self, timeout: PollTimeout, max_waits: u64) -> Result<(), ReactorFault> {
        for _ in 0..max_waits {
            let n = self
                .poller
                .wait(&mut self.ready, timeout)
                .map_err(ReactorFault::Host)?;
            if n == 0 {
                self.counters.idle_waits.fetch_add(1, Ordering::SeqCst);
                if self.stop.load(Ordering::Acquire) {
                    return Ok(());
                }
                continue;
            }
            self.counters.wakes.fetch_add(1, Ordering::SeqCst);
            self.counters
                .ready_reports
                .fetch_add(n as u64, Ordering::SeqCst);

            let mut produced = 0u64;
            let mut culprit = CONTROL_TOKEN;
            // The batch is copied out of the readiness buffer first: `handle` borrows the
            // registrar and the counters, and re-borrowing `self.ready` inside the loop
            // would pin the buffer for the whole batch.
            let batch: Vec<u64> = self.ready.iter().collect();
            for token in batch {
                let made = self.handle_token(token);
                if made == 0 {
                    culprit = token;
                    self.counters
                        .undrained_reports
                        .fetch_add(1, Ordering::SeqCst);
                }
                produced += made;
            }
            if produced > 0 {
                self.waker.wake();
            }

            // ★ The F1 refusal, as a bounded quantity rather than a hope.
            if produced == 0 {
                if culprit == self.streak_token {
                    self.streak += 1;
                } else {
                    self.streak = 1;
                    self.streak_token = culprit;
                }
                if self.streak >= MAX_UNPRODUCTIVE_STREAK {
                    return Err(ReactorFault::UndrainableSource {
                        token: culprit,
                        streak: self.streak,
                    });
                }
            } else {
                self.streak = 0;
            }

            if self.stop.load(Ordering::Acquire) {
                return Ok(());
            }
        }
        Ok(())
    }

    /// Handle one ready token, returning how many units of work it produced. Zero is what
    /// the F1 streak counts.
    fn handle_token(&self, token: u64) -> u64 {
        if token == CONTROL_TOKEN {
            self.counters.control_reports.fetch_add(1, Ordering::SeqCst);
            // Drained even when we are about to stop: a control source left readable is a
            // source the *next* wait reports again, which is the spin this loop refuses.
            let n = self.control.drain().unwrap_or(0);
            // A control wake is real work — it is what a removal barrier IS — so it counts
            // as progress even though it pushes nothing.
            return n.max(1);
        }
        match self.registrar.resolve(token) {
            Resolved::Live {
                source,
                host,
                shape,
            } => match shape {
                SourceShape::Counter => {
                    // ★ The drain runs with the registrar's table lock DROPPED — `resolve`
                    // cloned the `Arc` out and returned. The leaf witness in the raw layer
                    // is what turns that from a comment into a mechanism.
                    let n = match &*host {
                        crate::sources::HostSource::Counter(c) => c.drain().unwrap_or(0),
                        // A live entry's shape and its payload are derived from the same
                        // value, so this is unreachable; answering zero rather than
                        // panicking keeps the loop's failure mode "refuse loudly after a
                        // bounded streak" instead of "abort a thread nothing joins".
                        _ => 0,
                    };
                    for _ in 0..n {
                        self.push(source);
                    }
                    n
                }
                SourceShape::Terminal => {
                    // ★ Fires ONCE. Readiness here is a fact, not a quantity — nothing
                    // clears it, so the loop stops watching instead of draining.
                    self.counters.terminal_fired.fetch_add(1, Ordering::SeqCst);
                    self.registrar.quiesce_terminal(token);
                    self.push(source);
                    1
                }
                // Nothing arms one of these. Producing zero drives the streak straight into
                // the F1 refusal, which is the entire point of the shape existing.
                SourceShape::Foreign => 0,
            },
            Resolved::Retired(source) => {
                // §3.3's designed answer: push it anyway and let dispatch fault. The
                // handle is permanently unresolvable, so this cannot route onto anything.
                self.counters.retired_reports.fetch_add(1, Ordering::SeqCst);
                self.push(source);
                1
            }
            Resolved::Unknown => {
                self.counters.stale_reports.fetch_add(1, Ordering::SeqCst);
                0
            }
        }
    }

    fn push(&self, source: CompletionSource) {
        self.inbox.send(CoreEvent::SourceSignal(source));
        self.counters.signals_pushed.fetch_add(1, Ordering::SeqCst);
    }
}

kayfabe_util::assert_send_sync!(ReactorStats, ReactorFault, ReactorHandle);
kayfabe_util::assert_send!(Reactor);
