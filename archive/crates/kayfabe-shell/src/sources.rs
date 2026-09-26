//! The **registrar** — who owns a completion source's descriptor, and the two rules that
//! keep a readiness report from outliving what it names (`l1_os_shell.md` §3.2, §3.3).
//!
//! ## ★★ THE FINDING: §3.2's "the loop needs no table" is not compatible with §3.4(c)
//!
//! §3.2 is emphatic, and its consequences are stated as load-bearing:
//!
//! > *"The naive shell keeps a `HashMap<fd, CompletionSource>` and looks up every
//! > readiness. Do not build it… The loop holds an epoll descriptor and an `InboxSender`.
//! > It has no map, no lock, no `Arc<SharedDevice>` — there is **nothing shared** it could
//! > touch. A reverse table is still needed — but only for teardown … and the loop never
//! > sees it."*
//!
//! That is true of a shell whose loop only *reports* readiness. It is **false** of the
//! shell §3.4(c) prescribes, and the two sections are in the same document:
//!
//! > §3.4(c): *"Every source the reactor watches is a **counter** we own …, the loop
//! > **reads it** (which resets it to zero), and readiness therefore self-clears."*
//!
//! To read the counter the loop must hold the counter. The readiness report carries 64
//! bits of caller-chosen data and nothing else — no descriptor — so mapping *handle →
//! counter* is exactly the table §3.2 says not to build, and the loop is exactly the thread
//! that needs it. There are only three ways out and two of them are worse:
//!
//! 1. put the descriptor **in** the token — impossible, the token is already the handle,
//!    and O3 (never key readiness on a recyclable descriptor) is the reason it is;
//! 2. have someone *else* drain — that is one-shot re-arm (§3.4(b)), explicitly rejected,
//!    or a spin;
//! 3. ★ **give the loop the table**, and pay for it with the *other* discipline this
//!    project already has.
//!
//! So the loop has a table and a lock. What §3.2's argument was really protecting is
//! **inherited law 9** — *"the reactor loop touches ZERO core state, enforced by giving it
//! no reference to the device"* — and that survives intact and structurally: this table
//! maps a handle onto a **descriptor**, and there is no path from a descriptor to core
//! state. The loop still holds no `Arc<SharedDevice>`.
//!
//! What does **not** survive is "no lock". The table's lock has no rank, so
//! `kayfabe_util::lockwitness` is blind to it — which is precisely the hazard §14.8 F1
//! found in the KVM adapter, one component over. It is handled the same way: the critical
//! section is a `BTreeMap` probe and an `Arc` clone, the **drain runs outside it**, and
//! `kayfabe_util::leafwitness` asserts so at every syscall (including `epoll_wait` itself,
//! in the raw crate).
//!
//! ## ★ THE SECOND FINDING: not every source is a counter
//!
//! §3.4(c) says *"every source the reactor watches is a counter we own"*, and §3.1's own
//! table contradicts it two rows up: `Worker` is *"the worker's control channel; readiness
//! means **HUP**"*. A hung-up channel is readable **forever** and yields **nothing** to
//! read — there is no drain that clears it. Under a "drain everything" loop that is an
//! unbounded spin: the exact F1 failure the level-triggered choice was supposed to make
//! impossible, arriving through the one source class the same section defined.
//!
//! The fix is not to abandon level-triggering; it is to notice that readiness has two
//! *shapes*, and to say which each class is:
//!
//! | shape | classes | what readiness means | what clears it |
//! |---|---|---|---|
//! | [`SourceShape::Counter`] | `OsEvent`, `CrossIsolate`, `Notify` | N completions | the drain (N reads' worth, coalesced) |
//! | [`SourceShape::Terminal`] | `Worker`, and `IsolateExit` when it lands | the thing is **gone** | **deregistration** — it never becomes un-gone |
//!
//! A terminal source fires **once**, is unwatched by the loop on the spot, and is pushed
//! once. That is not an exception to the F1 gate; it is what makes the gate hold for a
//! class whose readiness has no counter behind it.

use std::collections::{BTreeMap, VecDeque};
use std::os::fd::{AsFd, OwnedFd};
use std::sync::{Arc, Mutex};

use kayfabe_core::reactor::{CompletionSource, SourceKind};
use kayfabe_linux_raw::{Notifier, Poller, RawError};
use kayfabe_util::leafwitness;

/// How many **retired** sources the registrar remembers so a readiness report already in
/// flight across a deregistration still resolves to its handle (§3.3: *"a readiness report
/// in flight across a deregistration is normal and safe … it pushes them anyway, and
/// dispatch faults"*).
///
/// Bounded, because retirement is guest-driven and an unbounded memory of dead handles is
/// the G10 shape. Evicting the oldest costs only the *diagnosis*: an evicted late report
/// becomes a counted [`crate::ReactorStats::stale_readiness`] instead of a
/// `SourceFault`, and both are refusals that mutate nothing.
pub const MAX_REMEMBERED_RETIRED: usize = 256;

/// The fraction of the process's descriptor budget the reactor may spend on guest-armed
/// sources (§3.8). The rest is the reserve: guest RAM windows, isolate channels, the KVM
/// descriptors, and whatever the process we are embedded in needs.
///
/// A **fraction**, not a subtracted constant, because `RLIMIT_NOFILE` is legitimately
/// `RLIM_INFINITY` on some deployments and legitimately 1024 on others.
pub const SOURCE_BUDGET_NUMERATOR: u64 = 1;
/// Denominator of [`SOURCE_BUDGET_NUMERATOR`].
pub const SOURCE_BUDGET_DENOMINATOR: u64 = 4;

/// An absolute ceiling on armed sources, independent of how generous `RLIMIT_NOFILE` is.
///
/// §3.8 asks for *"a per-`(proc, gpu)` bound and a device-global bound, both named
/// constants"*. This is the device-global one; the per-`(proc, gpu)` half belongs in the
/// core's `SourceRegistry::register` (which must grow a `Result` to express it) and is
/// **not built here** — see the crate docs' "what this stage did not build".
pub const MAX_ARMED_SOURCES: u64 = 8192;

/// What went wrong arming or disarming a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReactorError {
    /// The device-global armed-source cap, or the descriptor budget derived from the
    /// process's real `RLIMIT_NOFILE`, whichever is lower. **Contained**: the guest's arm
    /// fails and nothing else notices.
    SourceBudgetExhausted {
        /// How many sources are armed.
        armed: u64,
        /// The bound that was hit.
        bound: u64,
    },
    /// This handle is already armed. A second arm for one handle would leave a descriptor
    /// with no owner, so it is a refusal rather than a replace.
    AlreadyArmed,
    /// This handle names nothing armed (MISS = FAULT, same law as the core registry).
    NotArmed,
    /// The host refused, carrying its own error.
    Host(RawError),
}

/// Whether a source's readiness is a **quantity** that a read clears, or a **fact** that
/// nothing clears. See the module docs for why this distinction had to be invented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceShape {
    /// Readiness is a count; the loop drains it and pushes that many signals.
    Counter,
    /// Readiness means the thing is gone. The loop pushes **one** signal and stops
    /// watching — because nothing will ever make it un-ready.
    Terminal,
    /// ★ A readiness primitive **we did not create and cannot drain**.
    ///
    /// This is the shape §3.5 option (a) — *"pass the [nvidia] descriptor to us"* —
    /// **rejected**: it puts an ioctl-capable capability into the process that parses
    /// hostile guest bytes, which is *"the security posture inverted"*. It exists here so
    /// the refusal is a **mechanism with a test**, not a paragraph: a source of this shape
    /// that keeps reporting ready drives the loop into
    /// [`crate::ReactorFault::UndrainableSource`] within a bounded number of iterations.
    /// Nothing in this system arms one.
    Foreign,
}

impl SourceShape {
    /// The shape a core [`SourceKind`] has. Total, so a new kind is a compile error here
    /// rather than a silent default onto `Counter` — which for a terminal class would be
    /// an unbounded spin (see the module docs).
    #[must_use]
    pub fn of(kind: SourceKind) -> Self {
        match kind {
            SourceKind::OsEvent { .. } | SourceKind::CrossIsolate { .. } | SourceKind::Notify => {
                SourceShape::Counter
            }
            SourceKind::Worker { .. } => SourceShape::Terminal,
        }
    }
}

/// A host readiness primitive the registrar owns for the lifetime of one armed source.
#[derive(Debug)]
pub enum HostSource {
    /// A counter descriptor **we** created (§3.5(b): the isolate relays into it, so every
    /// nvidia descriptor stays inside the sandbox and the drain semantics are ours to
    /// define).
    Counter(Arc<Notifier>),
    /// A channel whose peer end is held by the thing whose death we are watching for.
    /// Readiness is its HUP.
    Channel(std::io::PipeReader),
    /// A descriptor we did not create — see [`SourceShape::Foreign`].
    Foreign(OwnedFd),
}

impl HostSource {
    fn shape(&self) -> SourceShape {
        match self {
            HostSource::Counter(_) => SourceShape::Counter,
            HostSource::Channel(_) => SourceShape::Terminal,
            HostSource::Foreign(_) => SourceShape::Foreign,
        }
    }

    fn watch_on(&self, poller: &Poller, token: u64) -> Result<(), RawError> {
        match self {
            HostSource::Counter(n) => poller.watch(n.as_source_fd(), token),
            HostSource::Channel(c) => poller.watch(c.as_fd(), token),
            HostSource::Foreign(f) => poller.watch(f.as_fd(), token),
        }
    }

    fn unwatch_on(&self, poller: &Poller) -> Result<(), RawError> {
        match self {
            HostSource::Counter(n) => poller.unwatch(n.as_source_fd()),
            HostSource::Channel(c) => poller.unwatch(c.as_fd()),
            HostSource::Foreign(f) => poller.unwatch(f.as_fd()),
        }
    }
}

/// One armed source, as the loop needs to see it.
#[derive(Debug)]
struct Armed {
    source: CompletionSource,
    host: Arc<HostSource>,
}

/// What a lookup found. Deliberately three-way: "armed", "retired but remembered" and
/// "nothing at all" have different consequences and none of them is an error to swallow.
#[derive(Debug)]
pub enum Resolved {
    /// Live: drain it (or fire it, if terminal).
    Live {
        /// The handle this token names.
        source: CompletionSource,
        /// The readiness primitive behind it, cloned out **under** the table lock so the
        /// drain can run outside it.
        host: Arc<HostSource>,
        /// Whether the readiness is a quantity or a fact.
        shape: SourceShape,
    },
    /// Deregistered, but recently enough that we still know which handle it was — so the
    /// signal is pushed and the core's dispatch faults on it (§3.3's designed answer).
    Retired(CompletionSource),
    /// Not armed and not remembered.
    Unknown,
}

/// The reverse table: `CompletionSource` → the descriptor that signals it.
///
/// **Its lock has no rank** (see the module docs). The critical section is a `BTreeMap`
/// probe and an `Arc` clone; every syscall — the drain, the `epoll_ctl`, the `close` —
/// happens with the guard dropped, and `leafwitness` is what says so.
#[derive(Debug)]
pub struct Registrar {
    poller: Arc<Poller>,
    table: Mutex<Table>,
    /// The descriptor budget, computed once at construction from the real `RLIMIT_NOFILE`.
    bound: u64,
}

#[derive(Debug, Default)]
struct Table {
    armed: BTreeMap<u64, Armed>,
    retired: VecDeque<(u64, CompletionSource)>,
    armed_ever: u64,
}

impl Registrar {
    /// Build a registrar over `poller`, deriving the source cap from the process's real
    /// descriptor limit (§3.8: *"so the core's constant cannot be set optimistically past
    /// reality"*).
    ///
    /// # Errors
    /// [`ReactorError::Host`] if the limit cannot be read.
    ///
    /// # Panics
    /// If called with any ranked lock or any adapter leaf lock held (R1).
    pub fn new(poller: Arc<Poller>) -> Result<Self, ReactorError> {
        let budget = kayfabe_linux_raw::descriptor_budget().map_err(ReactorError::Host)?;
        let derived = budget
            .saturating_mul(SOURCE_BUDGET_NUMERATOR)
            .saturating_div(SOURCE_BUDGET_DENOMINATOR);
        Ok(Self::with_bound(poller, derived.min(MAX_ARMED_SOURCES)))
    }

    /// A registrar with the bound stated explicitly rather than derived.
    ///
    /// Two callers: a deployment that knows better than `RLIMIT_NOFILE` what share of the
    /// process's descriptors it may spend, and the suite — because §3.8 is explicit that
    /// *"the refusal path must be exercised … an unexercised refusal path is where the C's
    /// worst behaviour lived"*, and exercising it against a derived bound of several
    /// hundred would make the test about descriptor churn rather than about the refusal.
    #[must_use]
    pub fn with_bound(poller: Arc<Poller>, bound: u64) -> Self {
        Registrar {
            poller,
            table: Mutex::new(Table::default()),
            bound,
        }
    }

    /// The device-global bound on armed sources actually in force.
    #[must_use]
    pub fn bound(&self) -> u64 {
        self.bound
    }

    /// How many sources are armed right now.
    #[must_use]
    pub fn armed(&self) -> u64 {
        let (t, _h) = self.lock();
        t.armed.len() as u64
    }

    /// ★ Arm `source`: take ownership of `host`, and start watching it.
    ///
    /// The order is the mirror of §3.3's deregistration rule and matters for the same
    /// reason: the descriptor is **stored before it is watched**, so the loop can never
    /// see a token whose descriptor the table does not yet own.
    ///
    /// # Errors
    /// [`ReactorError::SourceBudgetExhausted`] past the cap — a **contained** refusal;
    /// [`ReactorError::AlreadyArmed`]; [`ReactorError::Host`].
    ///
    /// # Panics
    /// If called with any ranked lock or any adapter leaf lock held (R1) — this performs
    /// an `epoll_ctl`.
    pub fn arm(&self, source: CompletionSource, host: HostSource) -> Result<(), ReactorError> {
        let token = source.as_token();
        // ---- PLAN (the table lock; no syscall) --------------------------------------
        let entry = {
            let (mut t, _h) = self.lock();
            let armed = t.armed.len() as u64;
            if armed >= self.bound {
                return Err(ReactorError::SourceBudgetExhausted {
                    armed,
                    bound: self.bound,
                });
            }
            if t.armed.contains_key(&token) {
                return Err(ReactorError::AlreadyArmed);
            }
            let host = Arc::new(host);
            t.armed.insert(
                token,
                Armed {
                    source,
                    host: Arc::clone(&host),
                },
            );
            t.armed_ever += 1;
            host
        };
        // ---- EXECUTE (every lock dropped) -------------------------------------------
        leafwitness::assert_leaf_free("arming a completion source");
        match entry.watch_on(&self.poller, token) {
            Ok(()) => Ok(()),
            Err(e) => {
                // ---- COMMIT, refused. Undo the bookkeeping so a failed arm leaves no
                // descriptor the loop can never be told about. The `Arc` we still hold is
                // what keeps the descriptor alive until this scope ends.
                let (mut t, _h) = self.lock();
                t.armed.remove(&token);
                Err(ReactorError::Host(e))
            }
        }
    }

    /// ★ Disarm `source`: **deregister, then close** (§3.3), and remember the handle so a
    /// report already in flight still resolves.
    ///
    /// > *"Closing a descriptor removes it from the readiness set only if it was the last
    /// > reference; a duplicated one silently stays and keeps firing. The order is a rule,
    /// > not an optimisation."*
    ///
    /// # Errors
    /// [`ReactorError::NotArmed`] (MISS = FAULT); [`ReactorError::Host`].
    ///
    /// # Panics
    /// If called with any ranked lock or any adapter leaf lock held (R1).
    pub fn disarm(&self, source: CompletionSource) -> Result<(), ReactorError> {
        let token = source.as_token();
        let entry = {
            let (mut t, _h) = self.lock();
            let Some(a) = t.armed.remove(&token) else {
                return Err(ReactorError::NotArmed);
            };
            t.retired.push_back((token, a.source));
            while t.retired.len() > MAX_REMEMBERED_RETIRED {
                t.retired.pop_front();
            }
            a.host
        };
        leafwitness::assert_leaf_free("disarming a completion source");
        // 1. deregister…
        let removed = entry.unwatch_on(&self.poller);
        // 2. …then close, which is this `drop` — and only if we hold the last reference.
        //    A concurrent loop iteration holding a clone keeps it open past here, which is
        //    the safe direction: a descriptor that outlives its epoll registration reports
        //    nothing.
        drop(entry);
        removed.map_err(ReactorError::Host)
    }

    /// ★ Arm a **counter** source and hand back the write end.
    ///
    /// This is §3.5(b)'s seam in one signature: the reactor owns the readable side, and
    /// the returned handle is what the **isolate's relay thread** writes when one of its
    /// own nvidia descriptors fires. Every nvidia descriptor stays inside the sandbox; the
    /// reactor watches only descriptors we created, *"which is also what makes §3.4(c)'s
    /// drain semantics ours to define."*
    ///
    /// # Errors
    /// As [`Registrar::arm`], plus [`ReactorError::Host`] if the counter cannot be created.
    ///
    /// # Panics
    /// If called with any ranked lock or any adapter leaf lock held (R1).
    pub fn arm_counter(&self, source: CompletionSource) -> Result<Arc<Notifier>, ReactorError> {
        let n = Arc::new(Notifier::create().map_err(ReactorError::Host)?);
        self.arm(source, HostSource::Counter(Arc::clone(&n)))?;
        Ok(n)
    }

    /// ★ Arm a **terminal** source and hand back the peer end of its channel.
    ///
    /// Readiness is the peer's HUP, so *dropping the returned writer is the death signal*
    /// — which is exactly what `SourceKind::Worker` means (§3.1: *"readiness means the
    /// worker is GONE"*) and is why no message ever needs to be sent on it.
    ///
    /// # Errors
    /// As [`Registrar::arm`], plus [`ReactorError::Host`] if the channel cannot be created.
    ///
    /// # Panics
    /// If called with any ranked lock or any adapter leaf lock held (R1).
    pub fn arm_channel(
        &self,
        source: CompletionSource,
    ) -> Result<std::io::PipeWriter, ReactorError> {
        let (r, w) = std::io::pipe().map_err(|e| {
            ReactorError::Host(RawError::Syscall {
                call: "pipe2",
                errno: e.raw_os_error(),
            })
        })?;
        self.arm(source, HostSource::Channel(r))?;
        Ok(w)
    }

    /// Disarm every armed source, returning how many went. The shutdown path (§7.9).
    ///
    /// # Panics
    /// If called with any ranked lock or any adapter leaf lock held (R1).
    pub fn disarm_all(&self) -> usize {
        let sources: Vec<CompletionSource> = {
            let (t, _h) = self.lock();
            t.armed.values().map(|a| a.source).collect()
        };
        sources.iter().filter(|s| self.disarm(**s).is_ok()).count()
    }

    /// How many sources have ever been armed — the **non-vacuity** half of every
    /// assertion about the table.
    #[must_use]
    pub fn armed_ever(&self) -> u64 {
        let (t, _h) = self.lock();
        t.armed_ever
    }

    /// Resolve a readiness token. The `Arc` is cloned out **under** the lock and the drain
    /// happens **outside** it — the §14.8 F2 shape, one component over.
    ///
    /// Public because the three-way answer is the observable §3.3 contract, and a test that
    /// could not tell [`Resolved::Retired`] from [`Resolved::Unknown`] could not tell a
    /// *designed* fault from a *forgotten* one.
    #[must_use]
    pub fn resolve(&self, token: u64) -> Resolved {
        let (t, _h) = self.lock();
        if let Some(a) = t.armed.get(&token) {
            return Resolved::Live {
                source: a.source,
                host: Arc::clone(&a.host),
                shape: a.host.shape(),
            };
        }
        for (tok, src) in &t.retired {
            if *tok == token {
                return Resolved::Retired(*src);
            }
        }
        Resolved::Unknown
    }

    /// Stop watching a terminal source that has fired, without forgetting it: the handle
    /// stays armed (only the executor's teardown disarms it), but the kernel stops telling
    /// us about a fact that will never change.
    pub(crate) fn quiesce_terminal(&self, token: u64) {
        let host = {
            let (t, _h) = self.lock();
            t.armed.get(&token).map(|a| Arc::clone(&a.host))
        };
        if let Some(h) = host {
            leafwitness::assert_leaf_free("unwatching a fired terminal source");
            // A `NotFound` here means a peer already did it — idempotent by design, since
            // the loop may batch two reports for one terminal source.
            let _ = h.unwatch_on(&self.poller);
        }
    }

    /// The readiness set this registrar arms into.
    #[must_use]
    pub fn poller(&self) -> &Arc<Poller> {
        &self.poller
    }

    fn lock(&self) -> (std::sync::MutexGuard<'_, Table>, leafwitness::Held) {
        let g = self
            .table
            .lock()
            .expect("the registrar table is never poisoned");
        let h = leafwitness::Held::enter();
        (g, h)
    }
}

kayfabe_util::assert_send_sync!(ReactorError, SourceShape, HostSource, Registrar);
