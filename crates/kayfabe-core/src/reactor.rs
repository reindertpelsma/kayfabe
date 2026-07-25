//! ★ The completion-source reactor — a core-owned **PURE** port
//! (`l1_concurrency.md` §6, decision #37).
//!
//! §5 of that doc defines *when* completions are observed and delivered; this module
//! is the machinery that decides *what a readiness signal means*. A reactor joins a
//! dynamic SET of completion sources and dispatches each signal to its owning proc;
//! the knowledge **"source S signalled → which proc → what to do"** is bookkeeping
//! over core state, so it lives here, under the core's normal locks and the same
//! R1–R5 discipline as any other core entry.
//!
//! ## Why this module lives in `kayfabe-core`, not `kayfabe-completion`
//!
//! Dispatch is keyed by [`ProcId`] and [`GpuId`] — the *routing* identities. `ProcId`
//! is defined by this crate (it is the derived label of a dup-connected component of
//! the RM graph); `kayfabe-completion` is the layer *below*, deliberately ignorant of
//! procs — it is pure per-queue policy. Putting the registry there would invert the
//! dependency (`kayfabe-completion` → `kayfabe-core`) and drag the whole graph/projection
//! world under the completion plane. So: the completion **plane** (queues, delivery,
//! fence arms) stays in `kayfabe-completion`; the completion **source routing** is here.
//!
//! ## ★ The hard boundary (§6.2) — state it hard
//!
//! **The core owns the model, and the model is PURE.** In this module there are
//! exactly three things:
//!
//! - [`CompletionSource`] — an **opaque** handle, minted at registration, meaningless
//!   except as identity. Handles come from a monotonic counter and are **never
//!   reused**, so a handle that outlives its registration can never alias a *fresh*
//!   source: the use-after-retire/ABA class (the C's F4 shape) is unrepresentable,
//!   not merely guarded.
//! - [`SourceRegistry`] and its [`SourceRegistry::dispatch`] — the routing knowledge,
//!   as plain owned data (a `BTreeMap`, no interior mutability, all mutation `&mut
//!   self`, per the crate's concurrency contract #17).
//! - [`WakeRequest`] — an **abstract notifiable source**. The core can *request* a
//!   wake; it does not know what a wake IS. There is no trait object here holding a
//!   host handle, and no callback out into the OS.
//!
//! There are **no host descriptors and no syscalls anywhere in this crate** — not in
//! the code, not in the comments. The core says "notifiable source" and "completion
//! source", full stop. This is enforced mechanically: the CI `stable` job greps the
//! pure logic crates for the OS-readiness vocabulary and fails the build on a hit
//! (`.github/workflows/ci.yml`). L1 owns the other side of the seam — the table
//! mapping a [`CompletionSource`] onto a real host readiness primitive, the real
//! wait loop, and the real wake behind [`WakeRequest`]. That loop maps
//! host-handle → [`CompletionSource`] (a pure table lookup), pushes an event onto the
//! executor inbox, and touches ZERO core state; the *dispatch* below runs on the
//! executor under the normal locks (inherited law 9: no core re-entry from the loop
//! thread).
//!
//! ## MISS = FAULT
//!
//! An unknown, stale, or already-deregistered source is a loud [`SourceFault`] —
//! never a guess, never a silent success, never a fallback onto proc 0 or
//! [`GpuId::ZERO`]. Same law as [`crate::rmgraph::RmGraph::gpu_of`] and the address
//! table: the routing tables are *forward-populated* by declared facts, and a miss
//! is a fault, because the alternative is the C's whole bug family.
//!
//! ## Why it earns its abstraction
//!
//! It is what makes the L1 concurrency **deterministically testable**: a test drives
//! [`SourceRegistry::dispatch`] directly, so every completion interleaving §5
//! describes becomes a scripted order with ZERO syscalls and no timing dependence.
//! It also kills the C's F2 shape at the model level — dispatch is per-source →
//! per-proc *by construction*, so there is nowhere in the model to even write an
//! `any_completed`-style cross-proc gate.

use std::collections::BTreeMap;

use kayfabe_arch::ids::GpuId;
use kayfabe_completion::OsEventRef;
use kayfabe_isolate::WorkerId;

use crate::ProcId;

/// An opaque completion-source handle: identity, and nothing else.
///
/// Deliberately has no public constructor and no readable payload — only
/// [`SourceRegistry::register`] mints one. Its ONE load-bearing property is that the
/// minting counter is **monotonic and never recycled**: a handle held past its
/// source's deregistration stays permanently unresolvable, so a late signal on a
/// retired proc's source cannot be re-bound to whatever was registered next. That is
/// the difference between a loud [`SourceFault`] and a use-after-retire.
///
/// **Scoped to its minting [`SourceRegistry`]** — like [`kayfabe_isolate::HostHandle`]
/// is scoped to one isolate's namespace. There is exactly one registry per device
/// (it lives in [`crate::gpu::Spine`]), so this never bites in practice; it is stated
/// so nobody builds a second registry and assumes handles are comparable across them.
///
/// (`Ord` so the registry iterates deterministically — registration ORDER must never
/// be observable in routing outcomes, decision #27.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CompletionSource(u64);

/// What a completion source *is* — the source classes of §6.1, **all of them from
/// day one** so adding the next one is a list entry, not a redesign.
///
/// Registration is a *declaration* by whoever owns the producing resource (the fwd
/// plane when it arms a host os-event, the isolate pool when it spawns a worker); the
/// registry never infers a kind from a signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceKind {
    /// A host RM os-event armed on behalf of `proc` for target `gpu` — the F3
    /// producer of §5.3. Readiness means "this guest-visible completion fired";
    /// dispatch observes it into that proc's `CompletionQueue`.
    ///
    /// Keyed by `(proc, gpu)` because a proc that spans two GPUs has a *separate*
    /// isolate and a separate completion path per target (MG-5/MG-6) — the pair is
    /// the routing key, never `proc` alone.
    OsEvent {
        /// The proc whose completion queue this event belongs to.
        proc: ProcId,
        /// The GPU target the event was armed on.
        gpu: GpuId,
        /// The guest-visible os-event this source stands for.
        ev: OsEventRef,
    },
    /// One isolate worker's control channel (§7.2): death/HUP detection and any
    /// out-of-band worker signal. Readiness on this class means the worker is GONE —
    /// there is no "the worker said something interesting" case, because the verb
    /// surface is a synchronous round-trip on the caller's own thread (§7.1).
    Worker {
        /// The proc whose isolate owns the worker.
        proc: ProcId,
        /// The GPU target of that isolate (one isolate per `(Proc, GpuId)`, MG-5).
        gpu: GpuId,
        /// Which worker of that isolate's bounded pool.
        worker: WorkerId,
    },
    /// The cross-process signalling seam: a completion owned by `from` must be
    /// reflected into `to` (completion-semaphore passthrough across procs).
    ///
    /// **Future** — nothing registers this yet. It is a source *class* now precisely
    /// so that landing it is a match arm, not a reshaping of the port (§6.1).
    CrossIsolate {
        /// The proc the signal originates in.
        from: ProcId,
        /// The proc the signal must be reflected into.
        to: ProcId,
    },
    /// The notifiable source itself — signalled to make the join re-read the source
    /// set (a source was added or removed) or pick up injected work. Belonging to no
    /// proc is the whole point: it is the one source whose dispatch is *not* routing.
    Notify,
}

impl SourceKind {
    /// The proc this source's signal routes to, if any. `None` for [`Notify`]
    /// (device-global by construction) — never a default-to-proc-0 guess.
    ///
    /// [`Notify`]: SourceKind::Notify
    #[must_use]
    pub fn owner(&self) -> Option<ProcId> {
        match *self {
            SourceKind::OsEvent { proc, .. } | SourceKind::Worker { proc, .. } => Some(proc),
            SourceKind::CrossIsolate { from, .. } => Some(from),
            SourceKind::Notify => None,
        }
    }

    /// True if this source belongs to `proc` in the sense [`SourceRegistry::deregister_proc`]
    /// means it: **either end** of a cross-isolate seam counts, because retiring
    /// either party makes the seam unroutable. Deliberately NOT `owner() == Some(p)`
    /// — a seam left half-registered after one side retired is exactly the dangling
    /// route this port exists to make impossible.
    #[must_use]
    pub fn belongs_to(&self, proc: ProcId) -> bool {
        match *self {
            SourceKind::OsEvent { proc: p, .. } | SourceKind::Worker { proc: p, .. } => p == proc,
            SourceKind::CrossIsolate { from, to } => from == proc || to == proc,
            SourceKind::Notify => false,
        }
    }
}

/// What the executor must DO about a signalled source — a typed action, so the
/// mapping from readiness to consequence is exhaustive and reviewable in one place.
///
/// Deliberately a *description*, not an effect: `dispatch` takes `&self` and performs
/// nothing. The caller applies it under the locks it already holds, which is what
/// keeps this whole port testable with no machinery at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Dispatch {
    /// Observe `ev` into `proc`'s completion queue for target `gpu` (§5.3's F3 edge).
    Observe {
        /// Owning proc.
        proc: ProcId,
        /// Target GPU.
        gpu: GpuId,
        /// The completion to observe.
        ev: OsEventRef,
    },
    /// A worker of `proc`'s `gpu` isolate died. The consequence is the §5.4 teardown
    /// path (refuse in-flight, surface the refusal, retire) — never a silent respawn:
    /// a worker that died mid-verb may have left host state the core cannot reason
    /// about.
    WorkerDied {
        /// Owning proc.
        proc: ProcId,
        /// Target GPU whose isolate lost the worker.
        gpu: GpuId,
        /// The worker slot that died.
        worker: WorkerId,
    },
    /// Reflect a completion across the process seam (future, see
    /// [`SourceKind::CrossIsolate`]).
    CrossSignal {
        /// Originating proc.
        from: ProcId,
        /// Receiving proc.
        to: ProcId,
    },
    /// The notifiable source fired: re-read the source set / pick up injected work.
    /// Routes to no proc, touches no proc state.
    Wake,
}

/// MISS = FAULT: a source that is not registered **right now**.
///
/// There is exactly one variant on purpose. "Never registered", "already
/// deregistered" and "its proc retired" are indistinguishable *by design* — handles
/// are never reused, so all three are the same fact ("this identity names nothing")
/// and any attempt to tell them apart would require keeping dead routing state alive,
/// which is the bug rather than the diagnosis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceFault {
    /// The unresolvable handle, echoed back for diagnostics.
    pub source: CompletionSource,
}

/// ★ The abstract notifiable source, as a value.
///
/// The source set is joined by something outside the core; when the set changes, that
/// something must be told to re-read it. The core cannot perform that signal — it does
/// not know what a wake IS (§6.2) — so it *returns a request for one*, and the
/// `#[must_use]` is the teeth: an adapter that changes the source set and forgets to
/// signal the join does not compile clean, and the workspace builds with
/// `-D warnings`. A dropped wake is a hung reactor with a source nobody is watching;
/// making that a compile-time complaint is cheaper than making it a debugging session.
///
/// Requests are **also latched inside the registry** ([`SourceRegistry::take_pending_wake`]).
/// That is what lets a core path which changes the set deep inside a larger operation
/// (proc retire inside `Spine::apply`) avoid threading a wake up through every
/// signature: the adapter drains the latch once after any core entry. Use
/// [`WakeRequest::latched`] to discharge the `#[must_use]` at such a site — it says
/// "already recorded; the drain will pick it up" out loud, which `let _ =` does not.
///
/// ## The teeth are real, both polarities pinned
///
/// Ignoring a set-changing op's result is a **compile error** under the workspace's
/// `-D warnings` (the lint sees through the returned tuple):
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use kayfabe_core::reactor::{SourceKind, SourceRegistry};
/// let mut reg = SourceRegistry::new();
/// reg.register(SourceKind::Notify); // no wake signalled, no wake latched-by-hand
/// ```
///
/// ...while discharging it compiles clean:
///
/// ```
/// #![deny(unused_must_use)]
/// use kayfabe_core::reactor::{SourceKind, SourceRegistry};
/// let mut reg = SourceRegistry::new();
/// let (_s, wake) = reg.register(SourceKind::Notify);
/// wake.latched();
/// ```
#[must_use = "a source-set change must signal the notifiable source, or the reactor \
              keeps joining a stale set (l1_concurrency.md §6.2); discharge it with \
              `WakeRequest::latched()` if the registry's pending-wake latch will \
              carry it instead"]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WakeRequest(());

impl WakeRequest {
    /// Discharge this request against the registry's pending-wake latch: the change
    /// happened inside a larger core operation, and whoever drives the reactor will
    /// pick it up via [`SourceRegistry::take_pending_wake`]. A no-op by construction —
    /// its whole job is to be an *explicit, greppable* consumption of the `#[must_use]`.
    pub fn latched(self) {}
}

/// ★ The device-global registry of completion sources + the dispatch that routes them.
///
/// Plain owned data — a `BTreeMap` and two counters. No interior mutability, all
/// mutation `&mut self`, `Send + Sync` (concurrency contract #17). Lives in
/// [`crate::gpu::Spine`], i.e. under the device lock, rank 0 of the L1 lock order:
/// registration/deregistration are spine ops, dispatch is a spine read.
///
/// Sources are added and removed **dynamically, as a plain set** — no fixed slots and
/// no rebuild-the-world on change (§6.1). Iteration is ascending by handle, so any
/// derived state stays a deterministic function of the *content* of the set, never of
/// the order it was filled in (decision #27).
///
/// **Known deferral (honest):** there is no cap on the number of registered sources.
/// Nothing guest-reachable registers one today (the os-event arming path lands in a
/// later stage), so a boundary-1 "no unbounded allocation from guest input" bound
/// would be an invented API shape guessing at that path's error surface. When arming
/// becomes guest-driven it needs the [`kayfabe_completion::MAX_OUTSTANDING_COMPLETIONS`]
/// treatment — a named constant and a loud refusal — and `register` grows a `Result`.
#[derive(Debug, Default)]
pub struct SourceRegistry {
    /// Registered sources, ascending by handle (the determinism contract).
    sources: BTreeMap<CompletionSource, SourceKind>,
    /// The monotonic mint. **Never decremented, never recycled** — see
    /// [`CompletionSource`].
    next: u64,
    /// Latched wake, drained by [`SourceRegistry::take_pending_wake`].
    pending_wake: bool,
}

impl SourceRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `kind`, minting a fresh never-before-used [`CompletionSource`].
    ///
    /// Returns the handle **and** a [`WakeRequest`]: the join is watching a set that
    /// no longer matches reality until it re-reads.
    pub fn register(&mut self, kind: SourceKind) -> (CompletionSource, WakeRequest) {
        let source = CompletionSource(self.next);
        // Monotonic and never recycled. u64 at any plausible arm/retire rate does not
        // wrap in the lifetime of the universe, but the core does not do silent
        // arithmetic wrap: an exhausted mint is a state we would rather not reach at
        // all than reach quietly.
        self.next = self
            .next
            .checked_add(1)
            .expect("completion-source mint exhausted");
        self.sources.insert(source, kind);
        (source, self.wake())
    }

    /// Deregister one source. MISS = FAULT: deregistering something that is not
    /// registered is a loud [`SourceFault`], not a no-op — a double-deregister means
    /// the caller's model of the set disagrees with the set.
    pub fn deregister(&mut self, source: CompletionSource) -> Result<WakeRequest, SourceFault> {
        if self.sources.remove(&source).is_none() {
            return Err(SourceFault { source });
        }
        Ok(self.wake())
    }

    /// ★ Deregister **every** source of `proc`, across **every** [`GpuId`] and both
    /// ends of any cross-isolate seam ([`SourceKind::belongs_to`]).
    ///
    /// This is the load-bearing half of the retire path. Without it, a source
    /// signalled after its proc retired would route to a proc that no longer exists —
    /// the C's F4 use-after-retire species. With it, the handle resolves to nothing
    /// and the signal is a loud [`SourceFault`].
    ///
    /// A proc with no sources is legal (retire is called unconditionally), and the
    /// wake is returned **unconditionally**: a spurious wake costs one re-read of an
    /// unchanged set, a missing wake costs a lost edge. Conservative is correct here.
    pub fn deregister_proc(&mut self, proc: ProcId) -> WakeRequest {
        self.sources.retain(|_, kind| !kind.belongs_to(proc));
        self.wake()
    }

    /// What `source` is. MISS = FAULT for the same reason [`Self::dispatch`] faults:
    /// an unresolvable identity has no honest answer.
    pub fn kind(&self, source: CompletionSource) -> Result<SourceKind, SourceFault> {
        self.sources
            .get(&source)
            .copied()
            .ok_or(SourceFault { source })
    }

    /// ★ The routing decision: *"source S signalled → which proc → what to do"*.
    ///
    /// Pure (`&self`, performs nothing) and total over the registered set. Unknown,
    /// stale, or retired-proc handles fault — see [`SourceFault`].
    pub fn dispatch(&self, source: CompletionSource) -> Result<Dispatch, SourceFault> {
        Ok(match self.kind(source)? {
            SourceKind::OsEvent { proc, gpu, ev } => Dispatch::Observe { proc, gpu, ev },
            SourceKind::Worker { proc, gpu, worker } => Dispatch::WorkerDied { proc, gpu, worker },
            SourceKind::CrossIsolate { from, to } => Dispatch::CrossSignal { from, to },
            SourceKind::Notify => Dispatch::Wake,
        })
    }

    /// Every registered source, **ascending by handle** (determinism contract).
    pub fn iter(&self) -> impl Iterator<Item = (CompletionSource, SourceKind)> + '_ {
        self.sources.iter().map(|(&s, &k)| (s, k))
    }

    /// How many sources are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sources.len()
    }

    /// True if no source is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// Drain the pending-wake latch: `Some` if the source set changed since the last
    /// drain. The reactor's driver calls this after any core entry and signals the
    /// notifiable source once — coalescing N set changes into one wake, which is
    /// sound because the wake means "re-read the set", not "one thing happened".
    ///
    /// ```
    /// use kayfabe_core::reactor::{SourceKind, SourceRegistry};
    ///
    /// let mut reg = SourceRegistry::new();
    /// let (_s, wake) = reg.register(SourceKind::Notify);
    /// wake.latched();
    /// assert!(reg.take_pending_wake().is_some(), "a set change wakes the join");
    /// assert!(reg.take_pending_wake().is_none(), "the latch drains");
    /// ```
    pub fn take_pending_wake(&mut self) -> Option<WakeRequest> {
        core::mem::take(&mut self.pending_wake).then_some(WakeRequest(()))
    }

    /// Latch + mint a wake request (every set-changing op ends here).
    fn wake(&mut self) -> WakeRequest {
        self.pending_wake = true;
        WakeRequest(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GPU0: GpuId = GpuId::ZERO;
    const GPU1: GpuId = GpuId(1);
    const PA: ProcId = ProcId(1);
    const PB: ProcId = ProcId(2);

    /// ★ Handles are minted from a monotonic counter and **never reused**, so a
    /// handle outliving its registration can never alias a fresh source: the
    /// stale-handle/ABA (use-after-retire) class is unrepresentable, not guarded.
    #[test]
    fn source_handles_are_never_reused_and_stale_handles_fault() {
        let mut reg = SourceRegistry::new();
        let kind = SourceKind::OsEvent {
            proc: PA,
            gpu: GPU0,
            ev: OsEventRef(0xE0),
        };

        let (first, w) = reg.register(kind);
        w.latched();
        reg.deregister(first).expect("registered").latched();

        // Re-registering the IDENTICAL kind mints a DIFFERENT identity.
        let (second, w) = reg.register(kind);
        w.latched();
        assert_ne!(
            first, second,
            "a recycled handle would re-bind stale signals"
        );

        // ...and the old handle stays permanently unresolvable.
        assert_eq!(
            reg.dispatch(first),
            Err(SourceFault { source: first }),
            "the stale handle must never resolve onto the fresh source"
        );
        assert_eq!(
            reg.dispatch(second),
            Ok(Dispatch::Observe {
                proc: PA,
                gpu: GPU0,
                ev: OsEventRef(0xE0)
            })
        );

        // Deregistering twice is a fault, not a shrug.
        assert_eq!(reg.deregister(first), Err(SourceFault { source: first }));

        // Exhaustively: no handle value is ever handed out twice, across a churn
        // that would trivially recycle a free-list-backed mint.
        let mut seen = std::collections::BTreeSet::new();
        seen.insert(first);
        seen.insert(second);
        for _ in 0..256 {
            let (s, w) = reg.register(SourceKind::Notify);
            w.latched();
            assert!(seen.insert(s), "handle {s:?} was minted twice");
            reg.deregister(s).expect("just registered").latched();
        }
    }

    /// Routing correctness: each [`SourceKind`] dispatches to ITS typed [`Dispatch`],
    /// carrying the exact identities it was declared with (no field shuffle, no
    /// `GpuId::ZERO` substitution).
    #[test]
    fn each_source_kind_dispatches_to_its_typed_action() {
        let mut reg = SourceRegistry::new();
        let (os, w) = reg.register(SourceKind::OsEvent {
            proc: PA,
            gpu: GPU1,
            ev: OsEventRef(0xBEEF),
        });
        w.latched();
        let (worker, w) = reg.register(SourceKind::Worker {
            proc: PB,
            gpu: GPU0,
            worker: WorkerId(3),
        });
        w.latched();
        let (cross, w) = reg.register(SourceKind::CrossIsolate { from: PA, to: PB });
        w.latched();
        let (notify, w) = reg.register(SourceKind::Notify);
        w.latched();

        assert_eq!(
            reg.dispatch(os),
            Ok(Dispatch::Observe {
                proc: PA,
                gpu: GPU1,
                ev: OsEventRef(0xBEEF)
            })
        );
        assert_eq!(
            reg.dispatch(worker),
            Ok(Dispatch::WorkerDied {
                proc: PB,
                gpu: GPU0,
                worker: WorkerId(3)
            })
        );
        assert_eq!(
            reg.dispatch(cross),
            Ok(Dispatch::CrossSignal { from: PA, to: PB })
        );
        assert_eq!(reg.dispatch(notify), Ok(Dispatch::Wake));

        // `kind` round-trips the declaration it was given.
        assert_eq!(
            reg.kind(worker),
            Ok(SourceKind::Worker {
                proc: PB,
                gpu: GPU0,
                worker: WorkerId(3)
            })
        );
    }

    /// MISS = FAULT: never-registered and already-deregistered handles both fault,
    /// and the fault names the offending handle — never a guess, never a silent
    /// success, never a fallback onto proc 0 / `GpuId::ZERO`.
    #[test]
    fn unknown_or_deregistered_source_faults_and_never_guesses() {
        let mut reg = SourceRegistry::new();
        // A live source of proc A exists, so a "fall back to the only proc we know"
        // implementation would have somewhere tempting to land.
        let (live, w) = reg.register(SourceKind::OsEvent {
            proc: PA,
            gpu: GPU0,
            ev: OsEventRef(1),
        });
        w.latched();

        let (dead, w) = reg.register(SourceKind::Worker {
            proc: PB,
            gpu: GPU1,
            worker: WorkerId(0),
        });
        w.latched();
        reg.deregister(dead).expect("registered").latched();

        assert_eq!(reg.dispatch(dead), Err(SourceFault { source: dead }));
        assert_eq!(reg.kind(dead), Err(SourceFault { source: dead }));
        assert_eq!(reg.deregister(dead), Err(SourceFault { source: dead }));

        // Emptying the registry does not make a lookup start guessing either: with a
        // single source left, a "fall back to the only thing we have" implementation
        // would be indistinguishable from correct on every other assertion.
        let mut lone = SourceRegistry::new();
        let (only, w) = lone.register(SourceKind::CrossIsolate { from: PA, to: PB });
        w.latched();
        lone.deregister(only).expect("registered").latched();
        assert!(lone.is_empty());
        assert_eq!(lone.dispatch(only), Err(SourceFault { source: only }));

        // The live source is undisturbed by any of the above.
        assert_eq!(
            reg.dispatch(live),
            Ok(Dispatch::Observe {
                proc: PA,
                gpu: GPU0,
                ev: OsEventRef(1)
            })
        );
        assert_eq!(reg.len(), 1);
    }

    /// ★ `deregister_proc` is scoped to THAT proc and spans EVERY GPU: multi-GPU is a
    /// first-class axis (a proc holds a separate isolate/arena/completion path per
    /// target, MG-5/MG-6), so retiring it must clear both targets. A *different* proc
    /// holding sources with the SAME numeric worker/event ids on the other GPU is
    /// untouched — the routing key is the whole `(proc, gpu, …)` tuple, never a
    /// numeric coincidence.
    #[test]
    fn deregister_proc_spans_all_gpus_and_spares_other_procs() {
        let mut reg = SourceRegistry::new();
        let mut reg_of = |k| {
            let (s, w) = reg.register(k);
            w.latched();
            s
        };

        // Proc A on BOTH GPUs.
        let a0 = reg_of(SourceKind::OsEvent {
            proc: PA,
            gpu: GPU0,
            ev: OsEventRef(7),
        });
        let a1 = reg_of(SourceKind::OsEvent {
            proc: PA,
            gpu: GPU1,
            ev: OsEventRef(7),
        });
        let aw1 = reg_of(SourceKind::Worker {
            proc: PA,
            gpu: GPU1,
            worker: WorkerId(2),
        });
        // Proc B on both GPUs with IDENTICAL numeric ids (the #14 shape on the GPU
        // axis: identical values, distinct identities).
        let b0 = reg_of(SourceKind::OsEvent {
            proc: PB,
            gpu: GPU0,
            ev: OsEventRef(7),
        });
        let b1 = reg_of(SourceKind::OsEvent {
            proc: PB,
            gpu: GPU1,
            ev: OsEventRef(7),
        });
        let bw1 = reg_of(SourceKind::Worker {
            proc: PB,
            gpu: GPU1,
            worker: WorkerId(2),
        });
        // The seam: BOTH ends must go when EITHER end retires.
        let seam = reg_of(SourceKind::CrossIsolate { from: PB, to: PA });
        // And a device-global source that belongs to no proc.
        let notify = reg_of(SourceKind::Notify);

        assert_eq!(reg.len(), 8);
        reg.deregister_proc(PA).latched();

        for gone in [a0, a1, aw1, seam] {
            assert_eq!(
                reg.dispatch(gone),
                Err(SourceFault { source: gone }),
                "{gone:?} belonged to the retired proc (or its seam)"
            );
        }
        for kept in [b0, b1, bw1, notify] {
            assert!(
                reg.dispatch(kept).is_ok(),
                "{kept:?} belongs to another proc / to the device"
            );
        }
        assert_eq!(reg.len(), 4);
        // Nothing in the survivors names the retired proc.
        assert!(
            reg.iter().all(|(_, k)| !k.belongs_to(PA)),
            "no residue of the retired proc anywhere in the set"
        );
        // Retiring a proc with no sources left is legal and idempotent.
        reg.deregister_proc(PA).latched();
        assert_eq!(reg.len(), 4);
    }

    /// Protocol-not-order (decision #27): the registry's OBSERVABLE routing is a
    /// function of the source *set*, not of the order it was registered in. Handle
    /// VALUES legitimately differ between orders (they are a mint, not a fact); the
    /// dispatch outcomes and the iterated content must not.
    #[test]
    fn dispatch_is_registration_order_independent() {
        let kinds = [
            SourceKind::OsEvent {
                proc: PA,
                gpu: GPU0,
                ev: OsEventRef(0x10),
            },
            SourceKind::Worker {
                proc: PB,
                gpu: GPU1,
                worker: WorkerId(1),
            },
            SourceKind::Notify,
            SourceKind::CrossIsolate { from: PA, to: PB },
            SourceKind::OsEvent {
                proc: PB,
                gpu: GPU1,
                ev: OsEventRef(0x10),
            },
        ];

        // Every rotation + the reverse of each: enough distinct permutations to catch
        // any order sensitivity without a proptest dependency in a unit module.
        let mut orders: Vec<Vec<usize>> = Vec::new();
        for shift in 0..kinds.len() {
            let fwd: Vec<usize> = (0..kinds.len())
                .map(|i| (i + shift) % kinds.len())
                .collect();
            let mut rev = fwd.clone();
            rev.reverse();
            orders.push(fwd);
            orders.push(rev);
        }

        let mut reference: Option<(Vec<Dispatch>, Vec<SourceKind>)> = None;
        for order in &orders {
            let mut reg = SourceRegistry::new();
            // Registered in THIS order...
            let mut handles = vec![None; kinds.len()];
            for &i in order {
                let (s, w) = reg.register(kinds[i]);
                w.latched();
                handles[i] = Some(s);
            }
            // ...but read back in the CANONICAL (kind-index) order.
            let dispatched: Vec<Dispatch> = (0..kinds.len())
                .map(|i| reg.dispatch(handles[i].expect("registered")).expect("live"))
                .collect();
            // Ascending iteration content (handle values excluded — see doc above).
            let iterated: Vec<SourceKind> = {
                let mut ks: Vec<SourceKind> = reg.iter().map(|(_, k)| k).collect();
                // Ascending-by-handle IS registration order here; the *content* is
                // what must match, so compare as a canonicalized multiset.
                ks.sort_unstable();
                ks
            };
            match &reference {
                None => reference = Some((dispatched, iterated)),
                Some((d0, i0)) => {
                    assert_eq!(&dispatched, d0, "order {order:?} changed dispatch outcomes");
                    assert_eq!(&iterated, i0, "order {order:?} changed the set content");
                }
            }
        }
        // Iteration itself is ascending by handle, always.
        let mut reg = SourceRegistry::new();
        let mut minted = Vec::new();
        for k in kinds {
            let (s, w) = reg.register(k);
            w.latched();
            minted.push(s);
        }
        let seen: Vec<CompletionSource> = reg.iter().map(|(s, _)| s).collect();
        let mut sorted = seen.clone();
        sorted.sort_unstable();
        assert_eq!(seen, sorted, "iteration must be deterministic + ascending");
    }

    /// Every set-changing op yields a [`WakeRequest`] and latches one — the join is
    /// never left watching a stale set. Reads do NOT latch (a spurious wake per
    /// lookup would be a self-inflicted storm).
    #[test]
    fn every_set_changing_op_yields_a_wake_request() {
        let mut reg = SourceRegistry::new();
        assert!(reg.take_pending_wake().is_none(), "fresh registry: no wake");

        let (s, wake) = reg.register(SourceKind::Notify);
        wake.latched();
        assert!(reg.take_pending_wake().is_some(), "register wakes");
        assert!(reg.take_pending_wake().is_none(), "the latch drains once");

        // Reads are silent.
        let _ = reg.dispatch(s);
        let _ = reg.kind(s);
        let _ = reg.len();
        let _ = reg.iter().count();
        assert!(reg.take_pending_wake().is_none(), "reads must not wake");

        // Deregister wakes; a FAILED deregister does not (nothing changed).
        reg.deregister(s).expect("registered").latched();
        assert!(reg.take_pending_wake().is_some(), "deregister wakes");
        assert!(reg.deregister(s).is_err());
        assert!(
            reg.take_pending_wake().is_none(),
            "a refused deregister changed nothing, so it must not wake"
        );

        // Proc deregistration always wakes — conservatively, even when it removed
        // nothing (a missing wake is a lost edge; a spurious one costs a re-read).
        reg.deregister_proc(PA).latched();
        assert!(reg.take_pending_wake().is_some(), "deregister_proc wakes");

        // N changes coalesce into ONE wake ("re-read the set", not "one thing
        // happened").
        for _ in 0..4 {
            let (_, w) = reg.register(SourceKind::Notify);
            w.latched();
        }
        assert!(reg.take_pending_wake().is_some());
        assert!(reg.take_pending_wake().is_none());
    }
}
