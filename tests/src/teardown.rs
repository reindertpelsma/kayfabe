//! # ★★ The universal teardown post-condition — a drop guard on the test device
//!
//! `l1_concurrency.md` §12.35; `l1_os_shell.md` §7.8 is the invariant it generalises.
//!
//! §7.8's conservation ledger was *opt-in*: two focused suites went looking for leaks,
//! and `l1_mean.rs` composed the census into one run. Everything else was green because
//! nobody asked. This module makes the question unavoidable — **every** test that builds
//! a device through [`Guarded`] asserts the teardown invariant when the device goes out
//! of scope, so a test that leaks fails *even though its own assertions passed*.
//!
//! ## The invariant, stated exactly
//!
//! At the moment the device is destroyed, for every isolate the mock ever served:
//!
//! ```text
//!   Outstanding(ledger)  ==  Reachable(core state)  ∪  Staged(pending_release)
//! ```
//!
//! read as a **set equality**, not a count (§12.28's form, which `retry_ledger.rs` and
//! `l1_mean.rs` already proved out). Both differences are findings, and they are
//! different findings:
//!
//! | difference | name | what it means |
//! |---|---|---|
//! | `Outstanding − (Reachable ∪ Staged)` | **UNACCOUNTED** | a host object that exists and that nothing can name or is going to free. ★ *This is the owner's case*: core state cleaned, host cleanup **never scheduled** — §12.33's exact shape. |
//! | `(Reachable ∪ Staged) − Outstanding` | **DANGLING** | core state (or the release queue) names something the ledger says was already released — a use-after-free shape, strictly worse than a leak |
//!
//! Plus the three corruption classes, which are never a disposition and are therefore
//! **not declarable**: [`HostLedger::double_free`], [`HostLedger::free_of_unknown`],
//! [`HostLedger::unmap_of_unknown`].
//!
//! ## Why `Staged` is on the accounted side, and why that is the sharp edge
//!
//! An object sitting in a proc's `pending_release` queue is *scheduled*: the next
//! verb-issuing op on that isolate drains it opportunistically, and
//! `SharedDevice::drain_pending_releases` is the backstop for a proc that goes quiet
//! (`l1_os_shell.md` §7.6 T0). A test that frees a VASpace and then simply ends has left
//! a queue, not a leak, and a guard that failed there would be a guard everybody turns
//! off.
//!
//! The distinction that survives is therefore **"queued" versus "never queued"**, and it
//! is exactly the distinction §12.33 found the core could not make: `Spine::refresh` used
//! to remove a vanished component's `Proc` with no staging at all, so from that instant
//! nothing — not core state, not a queue, not a later sweep — could name its host VAS or
//! its backings. That is what UNACCOUNTED catches, and it is why this guard needed
//! [`kayfabe_core::gpu::Proc::staged_releases`] to exist before it could say anything
//! true.
//!
//! ## The opt-out: **name the residue, or it fails**
//!
//! Some tests legitimately end with residue — a condemned owner, an out-of-band retire,
//! anything asserting a refusal. There is deliberately **no `allow_leaks` flag**: a bare
//! skip gets sprinkled around and the guard is dead inside a month. The only way out is
//! [`ResidueClaim`], which names the *exact* expected residue — per isolate, per host
//! class, exact counts — together with a mandatory reason:
//!
//! ```ignore
//! guard.declare_residue(
//!     ResidueClaim::on(IsolateId(pid.0), "§7.0 namespace death: `retire_proc` retired \
//!          the isolate before its staged release could drain, so the disposition of \
//!          record is the session's own death")
//!         .objects(VerbKind::AllocVaSpace, 1)
//!         .objects(VerbKind::AllocSysmem, 1)
//!         .maps(1),
//! );
//! ```
//!
//! Three properties fall out, and they are the requirement:
//!
//! 1. the declaration **is** the documentation of why the state is expected;
//! 2. a residue that **grows past** the declared set still fails — and so does one that
//!    shrinks, so a declaration cannot quietly rot into a lie after a fix;
//! 3. `grep -rn 'ResidueClaim::on'` enumerates every place we knowingly leave state.
//!
//! ## Why an assert in `Drop` is safe here, given §7.8 says the opposite
//!
//! §7.8 says the mean run's census must run *after* the final reap and **never** in a
//! `Drop`, citing §12.14: a held latch not released on unwind turns a failed assert into
//! a hang. That rule is about `l1_mean.rs`'s parked verbs and it still holds there — the
//! mean run keeps its explicit census. It does not generalise to a guard that (a) runs
//! after every thread the test spawned has been joined, because the device outlives them,
//! and (b) **skips itself entirely while the thread is already panicking**, the same
//! discipline `kayfabe_isolate::IsolateBox`'s own `Drop` uses for its R1 assert. Without
//! (b) a panic inside `Drop` during an unwind aborts the process and replaces the real
//! failure's message with a bare abort; with it, the cost is exact and small — a test
//! that is *already failing* is not additionally audited.

use std::collections::{BTreeMap, BTreeSet};

use kayfabe_core::ProcId;
use kayfabe_core::gpu::{Gpu, Proc};
use kayfabe_isolate::{HostHandle, IsolateId};
use kayfabe_mocks::{HostLedger, RmVerb, SharedRecorder, VerbKind};

use crate::{reachable_maps, reachable_objects};

// =================================================================================
// The view — what the audit needs to see, from either shape of device
// =================================================================================

/// What the teardown audit must be able to walk: every `Proc` the device still holds,
/// **live and retired-but-unreaped**.
///
/// Two implementations, because the suite has two device shapes: the bare
/// [`kayfabe_core::gpu::Gpu`] (single-threaded core tests) and the L1
/// [`kayfabe_rt::device::SharedDevice`] behind its two ranked locks. The audit is written
/// once against this trait so the two can never disagree about what "reachable" means —
/// the same reasoning `Gpu::sync_proc_to_boundary` is extracted under.
///
/// The retired half is not optional. A vacated proc has left the routing maps and the
/// proc set, but it still owns its isolates and its staged release queue until the reap;
/// a walk that missed it would report every correctly-staged corpse as a leak.
pub trait TeardownView {
    /// Visit every **live** proc (including the system proc) with its [`ProcId`].
    fn for_each_live(&self, f: &mut dyn FnMut(ProcId, &Proc));
    /// Visit every **retired-but-unreaped** proc.
    fn for_each_retired(&self, f: &mut dyn FnMut(&Proc));
}

impl TeardownView for Gpu {
    fn for_each_live(&self, f: &mut dyn FnMut(ProcId, &Proc)) {
        f(Gpu::SYSTEM_PROC, &self.system);
        for (&pid, p) in &self.procs {
            f(pid, p);
        }
    }
    fn for_each_retired(&self, f: &mut dyn FnMut(&Proc)) {
        for p in self.spine.retired_procs() {
            f(p);
        }
    }
}

impl<T: TeardownView> TeardownView for std::sync::Arc<T> {
    fn for_each_live(&self, f: &mut dyn FnMut(ProcId, &Proc)) {
        (**self).for_each_live(f);
    }
    fn for_each_retired(&self, f: &mut dyn FnMut(&Proc)) {
        (**self).for_each_retired(f);
    }
}

impl TeardownView for kayfabe_rt::device::SharedDevice {
    fn for_each_live(&self, f: &mut dyn FnMut(ProcId, &Proc)) {
        // One rank-1 lock at a time — `live_pids` takes only the device read guard, and
        // `with_proc` takes exactly one proc lock inside its own scope. Holding two at
        // once is what R3 refuses, so the walk is deliberately two-phase.
        for pid in self.live_pids() {
            self.with_proc(pid, |p| f(pid, p));
        }
    }
    fn for_each_retired(&self, f: &mut dyn FnMut(&Proc)) {
        self.with_retired(|corpses| {
            for p in corpses {
                f(p);
            }
        });
    }
}

/// ★ **Unpublish a VA and actually dispose of what it named** — the composed form every
/// test that models *"the guest freed this buffer and kept running"* wants
/// (`l1_os_shell.md` §7.6 T0).
///
/// Exists because the alternative kept being written by hand, and the hand-written
/// version is wrong in a way nothing used to notice: `vas.table.unbind(va)` drops the
/// [`kayfabe_mmu::HostBacking`] on the floor. The GPA block stays out of the arena, the
/// host memory object is never freed and its GPU mapping is never undone — and because
/// the proc stays alive, §7.0's process-boundary backstop never fires either. In a ring
/// buffer that is one leaked object *per iteration*, which is precisely the shape a
/// token-by-token inference loop has (§12.35 found 19 992 of them in `soak_llm_like`).
///
/// [`kayfabe_fwd::unpublish_backing`] returns the [`kayfabe_isolate::Orphans`] instead;
/// this runs their release chain on a worker of the proc's own isolate, with no lock
/// held, exactly as the T0 drain does.
///
/// # Errors
/// Whatever `unpublish_backing` refuses with (`UnknownPdb`, `Address(Miss)`, …).
///
/// # Panics
/// If no worker is available or the release chain is refused — in a single-threaded test
/// either would be a harness bug, and a swallowed failure here would silently restore the
/// leak this function exists to remove.
pub fn unpublish_and_release(
    proc: &mut Proc,
    gpu: kayfabe_arch::ids::GpuId,
    pdb: kayfabe_arch::ids::Pdb,
    va: kayfabe_arch::ids::GpuVa,
) -> Result<(), kayfabe_fwd::FwdFault> {
    let orphans = kayfabe_fwd::unpublish_backing(proc, gpu, pdb, va)?;
    if orphans.is_empty() {
        return Ok(());
    }
    let mut w = proc
        .isolate_mut(gpu)
        .expect("the proc published here, so its isolate is materialized")
        .checkout()
        .expect("a single-threaded test always has an idle worker");
    let out = w.execute(&orphans.release_plan(), &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"));
    proc.isolate_mut(gpu).expect("isolate").checkin(w);
    assert_eq!(
        out,
        Ok(kayfabe_isolate::VerbReply::Released),
        "the release chain must dispose of everything the unpublish named"
    );
    Ok(())
}

// =================================================================================
// The declaration — the ONLY opt-out, and it names what it excuses
// =================================================================================

/// ★ **The declared residue of one isolate**: the exact host objects a test knowingly
/// leaves outstanding at teardown, classified by the verb that minted them, plus the
/// reason it is a disposition rather than a leak.
///
/// Counts are per [`VerbKind`] and are compared for **equality**, not for a bound. That
/// is deliberate in both directions: more residue than declared is a regression, and
/// *less* is a declaration that has outlived its cause — a fix that closes a leak must
/// delete the claim that excused it, or the suite says so. A claim nobody can delete is a
/// claim nobody re-reads.
///
/// See the module docs for why there is no `allow_leaks` escape hatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResidueClaim {
    isolate: IsolateId,
    why: &'static str,
    objects: BTreeMap<VerbKind, usize>,
    maps: usize,
    dangling_objects: usize,
    dangling_maps: usize,
}

impl ResidueClaim {
    /// Declare residue on `isolate`. `why` is **required** and is the whole point: a
    /// declaration that does not say why is an `allow_leaks` flag wearing a struct.
    #[must_use]
    pub fn on(isolate: IsolateId, why: &'static str) -> Self {
        assert!(
            !why.trim().is_empty(),
            "a ResidueClaim must state WHY the residue is a disposition and not a leak"
        );
        ResidueClaim {
            isolate,
            why,
            objects: BTreeMap::new(),
            maps: 0,
            dangling_objects: 0,
            dangling_maps: 0,
        }
    }

    /// ★ Declare `objects` + `maps` **DANGLING** entries — core state naming something
    /// the ledger has no live record of.
    ///
    /// Narrower than it looks, and deliberately unpleasant to reach for. In product code
    /// this class is a use-after-free and there is no legitimate declaration of it. The
    /// only honest use is a test that has reached **past** the core: one that hand-rolls
    /// a release chain straight onto a worker (proving the objects *can* be freed, with
    /// no core-side unpublish to match), or one that fabricates a `HostHandle` the mock
    /// never minted in order to set up a state the protocol cannot reach. Both are
    /// harness bypasses, and saying so in the claim is the price of using one.
    #[must_use]
    pub fn dangling(mut self, objects: usize, maps: usize) -> Self {
        self.dangling_objects += objects;
        self.dangling_maps += maps;
        self
    }

    /// Expect exactly `n` outstanding objects minted by `kind` (`AllocVaSpace`,
    /// `AllocSysmem`, `AllocChannel`, `AllocEngineObject`, `Alloc`).
    #[must_use]
    pub fn objects(mut self, kind: VerbKind, n: usize) -> Self {
        *self.objects.entry(kind).or_default() += n;
        self
    }

    /// Expect exactly `n` outstanding GPU mappings on this isolate.
    #[must_use]
    pub fn maps(mut self, n: usize) -> Self {
        self.maps += n;
        self
    }
}

// =================================================================================
// The audit
// =================================================================================

/// Classify every handle the recorder's log ever minted, by the verb that minted it.
/// One pass, so the audit is O(log) rather than O(log × residue).
#[must_use]
fn classify(rec: &SharedRecorder) -> BTreeMap<HostHandle, VerbKind> {
    let mut m = BTreeMap::new();
    for (_iso, v) in &rec.lock().expect("recorder").log {
        let (h, k) = match v {
            RmVerb::AllocVaSpace { handle } => (*handle, VerbKind::AllocVaSpace),
            RmVerb::AllocSysmem { handle, .. } => (*handle, VerbKind::AllocSysmem),
            RmVerb::AllocChannel { handle, .. } => (*handle, VerbKind::AllocChannel),
            RmVerb::AllocEngineObject { handle, .. } => (*handle, VerbKind::AllocEngineObject),
            RmVerb::Alloc { handle, .. } => (*handle, VerbKind::Alloc),
            _ => continue,
        };
        m.insert(h, k);
    }
    m
}

/// Everything one proc has **queued** for release but not yet disposed of, keyed the way
/// the ledger keys it. The other half of `Accounted`, and the half §12.33 proved the core
/// could not produce.
fn staged(proc: &Proc) -> (BTreeSet<HostHandle>, BTreeSet<(HostHandle, u64)>) {
    let mut objs = BTreeSet::new();
    let mut maps = BTreeSet::new();
    for (_gpu, o) in proc.staged_releases() {
        objs.extend(o.free.iter().copied());
        maps.extend(o.unmap.iter().copied());
    }
    (objs, maps)
}

/// How many individual handles a violation message names before it says "…and N more".
/// A stress test can leave six figures of residue, and a panic message that scrolls the
/// terminal for a page hides the one line that matters (the by-class summary).
const MAX_NAMED: usize = 12;

/// `{a, b, c, …and N more}` — bounded rendering of a residue set.
fn brief<T: core::fmt::Debug>(set: &BTreeSet<T>) -> String {
    let shown: Vec<String> = set
        .iter()
        .take(MAX_NAMED)
        .map(|h| format!("{h:?}"))
        .collect();
    if set.len() > MAX_NAMED {
        format!(
            "{{{}, …and {} more}}",
            shown.join(", "),
            set.len() - MAX_NAMED
        )
    } else {
        format!("{{{}}}", shown.join(", "))
    }
}

/// One isolate's accounted sets: reachable-from-core-state ∪ staged-for-release.
#[derive(Default)]
struct Accounted {
    objects: BTreeSet<HostHandle>,
    maps: BTreeSet<(HostHandle, u64)>,
}

/// ★ Run the teardown post-condition over `view` and return every violation, most
/// structural first. Empty = the invariant held.
///
/// Pure and standalone so it is testable *as a function* (the guard's own teeth are
/// proved by calling this on a deliberately-leaking scenario, not by trusting a `Drop`),
/// and so a test may call it mid-run at a quiesce point of its own choosing.
#[must_use]
pub fn audit_teardown(
    view: &dyn TeardownView,
    rec: &SharedRecorder,
    declared: &[ResidueClaim],
) -> Vec<String> {
    let ledger: HostLedger = rec.lock().expect("recorder").ledger();
    let class = classify(rec);
    let mut out: Vec<String> = Vec::new();

    // ---- (1) Corruption. Never a disposition, never declarable. ----
    if !ledger.double_free.is_empty() {
        out.push(format!(
            "★★ DOUBLE FREE — a host handle was released twice (or a live handle re-minted): \
             {:?}. This is corruption, not a leak, and no ResidueClaim can excuse it.",
            ledger.double_free
        ));
    }
    if !ledger.free_of_unknown.is_empty() {
        out.push(format!(
            "★★ FREE OF UNKNOWN — a free of a handle that was never minted in that isolate's \
             namespace, i.e. a cross-namespace reach (boundary 2): {:?}",
            ledger.free_of_unknown
        ));
    }
    if !ledger.unmap_of_unknown.is_empty() {
        out.push(format!(
            "★★ UNMAP OF UNKNOWN — an unmap with no matching map: {:?}",
            ledger.unmap_of_unknown
        ));
    }

    // ---- (2) Accounted(core) — reachable ∪ staged, over live AND retired procs. ----
    let mut accounted: BTreeMap<IsolateId, Accounted> = BTreeMap::new();
    let mut absorb = |proc: &Proc| {
        // ★ N3 — bucketed by the handle's OWN recorded namespace ([`HostHandle::isolate`],
        // an `(proc, GPU)` [`IsolateId`]), not by the proc. A proc that spans two GPUs owns
        // two isolates with two disjoint RM client namespaces, and this audit used to add
        // both to one bucket — so an object accounted against the *wrong* one of a proc's
        // isolates balanced perfectly. It no longer can: the ledger keys on the isolate
        // that ISSUED the verb, this keys on the isolate that MINTED the handle, and the
        // set equality below is now a statement that those two agree.
        for h in reachable_objects(proc) {
            accounted.entry(h.isolate()).or_default().objects.insert(h);
        }
        for (vas, va) in reachable_maps(proc) {
            accounted
                .entry(vas.isolate())
                .or_default()
                .maps
                .insert((vas, va));
        }
        let (sobj, smap) = staged(proc);
        for h in sobj {
            accounted.entry(h.isolate()).or_default().objects.insert(h);
        }
        for (vas, va) in smap {
            accounted
                .entry(vas.isolate())
                .or_default()
                .maps
                .insert((vas, va));
        }
    };
    view.for_each_live(&mut |_pid, p| absorb(p));
    view.for_each_retired(&mut |p| absorb(p));

    // ---- (3) The set equality, per isolate. ----
    let mut isolates: BTreeSet<IsolateId> = ledger.leaked.keys().copied().collect();
    isolates.extend(ledger.leaked_maps.keys().copied());
    isolates.extend(accounted.keys().copied());

    type ObjMapSets = (BTreeSet<HostHandle>, BTreeSet<(HostHandle, u64)>);
    let mut unaccounted: BTreeMap<IsolateId, ObjMapSets> = BTreeMap::new();
    let mut dangling: BTreeMap<IsolateId, ObjMapSets> = BTreeMap::new();
    for iso in isolates {
        let outstanding = ledger.leaked_on(iso);
        let outstanding_maps = ledger.leaked_maps.get(&iso).cloned().unwrap_or_default();
        let acc = accounted.remove(&iso).unwrap_or_default();

        let d: BTreeSet<_> = acc.objects.difference(&outstanding).copied().collect();
        let dm: BTreeSet<_> = acc.maps.difference(&outstanding_maps).copied().collect();
        if !d.is_empty() || !dm.is_empty() {
            dangling.insert(iso, (d, dm));
        }
        let un: BTreeSet<_> = outstanding.difference(&acc.objects).copied().collect();
        let un_maps: BTreeSet<_> = outstanding_maps.difference(&acc.maps).copied().collect();
        if !un.is_empty() || !un_maps.is_empty() {
            unaccounted.insert(iso, (un, un_maps));
        }
    }

    // ---- (4) The two differences versus what was DECLARED — exact, in both
    //      directions. More residue than declared is a regression; LESS is a
    //      declaration that has outlived its cause and must be deleted, not loosened.
    let mut claims: BTreeMap<IsolateId, &ResidueClaim> = BTreeMap::new();
    for c in declared {
        assert!(
            claims.insert(c.isolate, c).is_none(),
            "two ResidueClaims for {:?} — fold them into one so the declared totals are \
             readable in one place",
            c.isolate
        );
    }
    let all: BTreeSet<IsolateId> = unaccounted
        .keys()
        .copied()
        .chain(dangling.keys().copied())
        .chain(claims.keys().copied())
        .collect();
    for iso in all {
        let (objs, maps) = unaccounted.get(&iso).cloned().unwrap_or_default();
        let (dobjs, dmaps) = dangling.get(&iso).cloned().unwrap_or_default();
        let mut found: BTreeMap<VerbKind, usize> = BTreeMap::new();
        for h in &objs {
            *found
                .entry(class.get(h).copied().unwrap_or(VerbKind::Alloc))
                .or_default() += 1;
        }
        let claim = claims.get(&iso).copied();
        let (want_objs, want_maps, want_d, want_dm) = match claim {
            Some(c) => (
                c.objects.clone(),
                c.maps,
                c.dangling_objects,
                c.dangling_maps,
            ),
            None => (BTreeMap::new(), 0, 0, 0),
        };

        if (dobjs.len(), dmaps.len()) != (want_d, want_dm) {
            out.push(format!(
                "★★ DANGLING on {iso:?} — core state (or its release queue) names {} host \
                 object(s) and {} mapping(s) the ledger has no live record of; {want_d} + \
                 {want_dm} were declared. In product code this is a use-after-free shape, \
                 strictly worse than a leak.\n      objects: {}\n      mappings: {}\n      \
                 claim: {:?}",
                dobjs.len(),
                dmaps.len(),
                brief(&dobjs),
                brief(&dmaps),
                claim.map(|c| c.why),
            ));
        }

        if found != want_objs || maps.len() != want_maps {
            match claim {
                None => out.push(format!(
                    "★★ UNACCOUNTED on {iso:?} — {} host object(s) and {} mapping(s) are \
                     OUTSTANDING and NOTHING can name them: not core state, not a \
                     `pending_release` queue. Nothing will ever free them because nothing can \
                     address them.\n      by class: {found:?}\n      objects: {}\n      \
                     mappings: {}\n      If this is a real disposition (§7.0 namespace death, \
                     a condemned owner, a refusal under test) then DECLARE it — \
                     `ResidueClaim::on({iso:?}, \"why\")` — and the declaration becomes the \
                     documentation. There is no `allow_leaks` flag.",
                    objs.len(),
                    maps.len(),
                    brief(&objs),
                    brief(&maps),
                )),
                Some(c) => out.push(format!(
                    "★ DECLARED RESIDUE MISMATCH on {iso:?} — the claim says {want_objs:?} + \
                     {want_maps} mapping(s), the run left {found:?} + {} mapping(s).\n      \
                     claim: \"{}\"\n      objects: {}\n      mappings: {}\n      A residue that \
                     GREW is a regression; one that SHRANK means the claim outlived its cause \
                     and must be deleted, not loosened.",
                    maps.len(),
                    c.why,
                    brief(&objs),
                    brief(&maps),
                )),
            }
        }
    }
    out
}

// =================================================================================
// The guard
// =================================================================================

/// ★★ **The drop guard**: wrap a test's device in this and the teardown post-condition
/// runs when it goes out of scope — see the module docs for the invariant, the opt-out
/// and the `Drop` discipline.
///
/// [`Deref`](core::ops::Deref)s to the wrapped device, so a retrofit is a change to the
/// harness helper and nothing else in the test body.
pub struct Guarded<D: TeardownView> {
    /// `None` only between [`Guarded::map`] taking the device and the old guard's
    /// `Drop` — the one moment there is nothing to audit.
    device: Option<D>,
    rec: SharedRecorder,
    what: &'static str,
    declared: Vec<ResidueClaim>,
}

impl<D: TeardownView> Guarded<D> {
    /// Arm the guard on `device`. `what` names the test/harness in the failure message.
    #[must_use]
    pub fn new(what: &'static str, device: D, rec: SharedRecorder) -> Self {
        Guarded {
            device: Some(device),
            rec,
            what,
            declared: Vec::new(),
        }
    }

    /// ★ Move the guard onto a **transformed** device — `Gpu` → `Arc<SharedDevice>`
    /// being the case the suite needs, since `SharedDevice::new` takes the `Gpu` by
    /// value.
    ///
    /// Deliberately the only way a guarded device changes shape: there is no
    /// `into_inner()`, because an un-guarding escape hatch is exactly the `allow_leaks`
    /// flag this design refuses. Declarations travel with the guard.
    #[must_use]
    pub fn map<E: TeardownView>(mut self, f: impl FnOnce(D) -> E) -> Guarded<E> {
        let d = self.device.take().expect("armed");
        Guarded {
            device: Some(f(d)),
            rec: self.rec.clone(),
            what: self.what,
            declared: core::mem::take(&mut self.declared),
        }
    }

    /// ★ The **only** opt-out: declare exactly what residue this test expects to leave,
    /// and why. See [`ResidueClaim`]; `grep -rn 'ResidueClaim::on'` enumerates every one.
    pub fn declare_residue(&mut self, claim: ResidueClaim) -> &mut Self {
        self.declared.push(claim);
        self
    }

    /// The recorder behind the ledger (tests already hold their own clone; this is for
    /// harness helpers that build the guard and want to hand one back).
    #[must_use]
    pub fn recorder(&self) -> &SharedRecorder {
        &self.rec
    }

    /// Run the audit **now**, at a point of the test's own choosing, and panic on any
    /// violation. The `Drop` path calls exactly this.
    pub fn assert_teardown_invariant(&self) {
        let Some(d) = self.device.as_ref() else {
            return;
        };
        let v = audit_teardown(d, &self.rec, &self.declared);
        assert!(
            v.is_empty(),
            "\n★★ TEARDOWN POST-CONDITION FAILED — {} ({} violation(s))\n\
             This test's own assertions passed; the device still leaked. \
             (`l1_concurrency.md` §12.35)\n\n{}\n",
            self.what,
            v.len(),
            v.join("\n\n")
        );
    }
}

impl<D: TeardownView> core::ops::Deref for Guarded<D> {
    type Target = D;
    fn deref(&self) -> &D {
        self.device.as_ref().expect("armed")
    }
}

impl<D: TeardownView> core::ops::DerefMut for Guarded<D> {
    fn deref_mut(&mut self) -> &mut D {
        self.device.as_mut().expect("armed")
    }
}

impl<D: TeardownView> core::fmt::Debug for Guarded<D> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Guarded")
            .field("what", &self.what)
            .field("declared", &self.declared)
            .finish_non_exhaustive()
    }
}

impl<D: TeardownView> Drop for Guarded<D> {
    /// # Panics
    /// If the teardown invariant does not hold — unless this thread is **already**
    /// panicking, in which case the audit is skipped so the real failure keeps its
    /// message (a panic in `Drop` during an unwind aborts the process). Same discipline
    /// as `kayfabe_isolate::IsolateBox`'s R1 assert, and for the same reason.
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        self.assert_teardown_invariant();
    }
}
