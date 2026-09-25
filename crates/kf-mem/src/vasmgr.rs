//! ★★★★★ **THE VA MANAGER STEP — invalidate → walk → reconcile → map → clear.**
//! (`V3_P4_PORT_MAP.md` §2.1(b)+(c), §3 row 3; `THE_TRANSLATED_PLANE.md` §5 and its `[w824b]`
//! table; `THE_CONSTRAINTS.md` §49.1; `THE_ARCHITECTURE_v3.md` §4.2-§4.3.)
//!
//! The guest's `MMU_INVALIDATE` is the synchronisation point between its page tables and our
//! host VA spaces. The vCPU only arms the trigger and publishes a request
//! ([`kf_trap::InvalidatePort`]); everything else happens here, on the VA-manager thread:
//!
//! 1. **[`VaManager::on_invalidate`]** — look the named PDB up in the [`VasTable`] (identity is the
//!    VA-space OBJECT; the PDB is a mutable, possibly-absent attribute of it, §4.3), and submit ONE
//!    walk of every root that needs it. ⊘ **Never blocks**: [`Walker::submit`] only queues GPU work.
//!    A request that arrives while a walk is in flight waits for the NEXT walk — the one in flight
//!    may have read the tables before the guest's writes that preceded this invalidate.
//! 2. **[`VaManager::on_walk_ready`]** — on the walker's completion fd: take the FULL report,
//!    classify its leaves ([`desired_from_leaves`]), diff them against OUR ledger
//!    ([`plan_reconcile`]), and apply through the space's [`MapTarget`] — deferred unmaps, deferred
//!    maps, ONE invalidate ([`Ledger::apply_to`]).
//! 3. **Only then** `Trigger::complete(seq)` — a compare-and-set, so a request the guest already
//!    abandoned (it timed out and re-issued) is `Superseded` and never clears the later one (§5.5).
//!
//! ## What is never done here
//!
//! ⊘ No CPU read of a guest page table and no mirror of one: the walk is the GPU's
//! (`kf_cuda::WalkKernel`), the only previous state is OUR ledger (`V3_BUILD.md`). ⊘ No blocking:
//! both entry points return as soon as their work is queued or applied. ⊘ No host flag is
//! forwarded: [`MapTarget`] verbs are authored.
//!
//! ## What an invalidate that cannot be honoured does
//!
//! It is **not cleared**, and it is counted and named ([`VaStats::unreconciled`],
//! [`VaStats::refusals`]). The guest then times out (§5.5's tripwire: an overdue trigger is a
//! fault, not a slow path). Clearing it would tell the guest a mapping is live that is not —
//! §49.1's early completion, which is silent corruption rather than a visible failure.
//! ★ Two exceptions, both because we hold nothing to be stale: a PDB no object carries
//! ([`VaStats::named_missed`]) and a batch in which no named space has a root.

use crate::ledger::{Applied, Ledger, MapTarget, clip_leaves, desired_from_leaves, plan_reconcile};
use kf_trap::{ClearOutcome, InvalidateRequest, PdbAperture, Trigger};
use std::collections::{BTreeMap, BTreeSet};

/// The most address spaces one walk may carry — the walk kernel's table (`KF_MAX_PDB`).
pub const MAX_SPACES_PER_WALK: usize = 64;

/// ★ A VA-space OBJECT's identity (`THE_ARCHITECTURE_v3.md` §4.3: *"identity is the object, not
/// the PDB"*) — the guest's resource key, packed by the caller (e.g. `hClient << 32 | hObject`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VasKey(pub u64);

/// Why a root statement was refused. By name, never clamped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootRefusal {
    /// The object is not in the table.
    UnknownObject(VasKey),
    /// A sysmem-rooted directory (UVM can place one there). ⊘ Refused until the walker imports
    /// the guest-RAM window (`V3_P4_PORT_MAP.md` §2.2); walking the store for it would read the
    /// wrong memory.
    SysmemRoot {
        /// The object.
        key: VasKey,
        /// The root it named.
        pdb: u64,
    },
    /// A root outside the store: nothing we could walk.
    OutsideStore {
        /// The object.
        key: VasKey,
        /// The root it named.
        pdb: u64,
    },
}

/// One VA-space object: its (optional) guest root, where its mappings land, and OUR ledger.
#[derive(Debug)]
struct Space<T> {
    root: Option<u64>,
    target: T,
    ledger: Ledger,
}

/// ★ **VA-space object → `Option<root>` + map target + ledger** (`V3_P4_PORT_MAP.md` §2.1(b)).
///
/// ⊘ Not a copy of the guest's tables: a root is ONE number the guest stated
/// (`SET_PAGE_DIRECTORY`, `COPY_SERVER_RESERVED_PDES`, fn 70, or the PDB of an invalidate we
/// were told about), and the ledger records OUR actions.
#[derive(Debug)]
pub struct VasTable<T> {
    spaces: BTreeMap<VasKey, Space<T>>,
    store_bytes: u64,
}

impl<T: MapTarget> VasTable<T> {
    /// An empty table over a store of `store_bytes`.
    #[must_use]
    pub fn new(store_bytes: u64) -> VasTable<T> {
        VasTable { spaces: BTreeMap::new(), store_bytes }
    }

    /// Register an object and where its mappings land. It has no root yet.
    pub fn insert(&mut self, key: VasKey, target: T) {
        self.spaces.insert(key, Space { root: None, target, ledger: Ledger::default() });
    }

    /// Forget an object, returning its target and ledger (the caller tears the mappings down).
    pub fn remove(&mut self, key: VasKey) -> Option<(T, Ledger)> {
        self.spaces.remove(&key).map(|s| (s.target, s.ledger))
    }

    /// ★ The guest stated a root for `key`. Returns whether it CHANGED — a changed root with no
    /// invalidate behind it needs a walk ([`VaManager::schedule_walk`], `V3_P4_PORT_MAP.md` Q10).
    ///
    /// # Errors
    /// [`RootRefusal`], by name.
    pub fn set_root(&mut self, key: VasKey, pdb: u64, aperture: PdbAperture) -> Result<bool, RootRefusal> {
        let store = self.store_bytes;
        let s = self.spaces.get_mut(&key).ok_or(RootRefusal::UnknownObject(key))?;
        if aperture == PdbAperture::Sysmem {
            return Err(RootRefusal::SysmemRoot { key, pdb });
        }
        if pdb >= store {
            return Err(RootRefusal::OutsideStore { key, pdb });
        }
        let changed = s.root != Some(pdb);
        s.root = Some(pdb);
        Ok(changed)
    }

    /// The guest withdrew the root (`UNSET_PAGE_DIRECTORY`). Our mappings stay until a walk says
    /// otherwise — ⊘ nothing here may unmap on a guess.
    pub fn clear_root(&mut self, key: VasKey) {
        if let Some(s) = self.spaces.get_mut(&key) {
            s.root = None;
        }
    }

    /// The root of `key`, if stated.
    #[must_use]
    pub fn root(&self, key: VasKey) -> Option<u64> {
        self.spaces.get(&key).and_then(|s| s.root)
    }

    /// Where `key`'s mappings land.
    #[must_use]
    pub fn target(&self, key: VasKey) -> Option<&T> {
        self.spaces.get(&key).map(|s| &s.target)
    }

    /// OUR ledger for `key`.
    #[must_use]
    pub fn ledger(&self, key: VasKey) -> Option<&Ledger> {
        self.spaces.get(&key).map(|s| &s.ledger)
    }

    /// Every object whose root is `pdb` (two objects may share one: RM's server VAS and a
    /// client's, before a `SET_PAGE_DIRECTORY` splits them).
    #[must_use]
    pub fn keys_for_pdb(&self, pdb: u64) -> Vec<VasKey> {
        self.spaces.iter().filter(|(_, s)| s.root == Some(pdb)).map(|(&k, _)| k).collect()
    }

    /// Every object that has a root — what `ALL_PDB` names.
    #[must_use]
    pub fn rooted(&self) -> Vec<VasKey> {
        self.spaces.iter().filter(|(_, s)| s.root.is_some()).map(|(&k, _)| k).collect()
    }

    /// Objects held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.spaces.len()
    }

    /// No objects.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.spaces.is_empty()
    }
}

/// One space as a walk reported it: its root and the COMPLETE list of its leaves,
/// `(va, at, len, aperture)` in the shape [`desired_from_leaves`] takes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WalkedSpace {
    /// The root walked.
    pub pdb: u64,
    /// Every leaf — a full state, never a delta.
    pub leaves: Vec<(u64, u64, u64, u8)>,
}

/// A finished walk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WalkDone {
    /// One entry per root walked.
    pub spaces: Vec<WalkedSpace>,
    /// GPU time, when the walker measures it.
    pub gpu_us: u64,
}

/// ★ **The GPU walker, as the VA manager needs it** — two verbs, neither of which may block.
/// Production is [`GpuWalker`] (the PTX walk kernel on its own stream, completing through an
/// eventfd); tests use an in-memory fake.
pub trait Walker {
    /// Queue a walk of `pdbs` (ascending, unique, at most [`MAX_SPACES_PER_WALK`]). Must return as
    /// soon as the work is queued.
    ///
    /// # Errors
    /// The walker's refusal, by name.
    fn submit(&mut self, pdbs: &[u64]) -> Result<(), String>;

    /// The walk's result if it has finished; `Ok(None)` if not yet. Must not block.
    ///
    /// # Errors
    /// The walk failed or its report was refused (malformed, truncated, not full), by name.
    fn poll(&mut self) -> Result<Option<WalkDone>, String>;
}

/// ★★★ **The production walker**: [`kf_cuda::WalkKernel`] over the imported store.
///
/// `submit` queues the walk on the kernel's stream (no `cuCtxSynchronize`); `poll` is
/// `try_collect`, then the report must pass `validate` (§39(c) containment included),
/// `require_full` (no delta) and must not be truncated — any failure is a named refusal.
pub struct GpuWalker {
    /// The walk kernel (owns its CUDA context and completion fd).
    pub kernel: kf_cuda::WalkKernel,
    /// The store's device pointer in the walker's context.
    pub store_ptr: u64,
    /// The store's length.
    pub store_bytes: u64,
}

impl Walker for GpuWalker {
    fn submit(&mut self, pdbs: &[u64]) -> Result<(), String> {
        self.kernel.submit(self.store_ptr, self.store_bytes, pdbs).map_err(|e| e.to_string())
    }

    fn poll(&mut self) -> Result<Option<WalkDone>, String> {
        let Some(c) = self.kernel.try_collect().map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        let r = &c.report;
        r.validate().map_err(|e| format!("walk report refused: {e}"))?;
        r.require_full().map_err(|e| format!("walk report refused: {e}"))?;
        if r.truncated() {
            return Err(format!(
                "walk report TRUNCATED (flags={:#x}, runs {} of {}): not the whole state of any \
                 space, never reconciled against",
                r.header.flags, r.runs.len(), r.header.run_count
            ));
        }
        let mut spaces: Vec<WalkedSpace> =
            r.pdbs.iter().map(|p| WalkedSpace { pdb: p.pdb, leaves: Vec::new() }).collect();
        for m in &r.runs {
            // `validate` proved every `pdb_index` names an entry.
            if let Some(s) = spaces.get_mut(usize::from(m.pdb_index)) {
                s.leaves.push((m.va, m.gpga, m.len, m.aperture()));
            }
        }
        Ok(Some(WalkDone { spaces, gpu_us: c.gpu_us }))
    }
}

fn ns_since(t: std::time::Instant) -> u64 {
    u64::try_from(t.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// Why a walk is wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Want {
    /// The guest invalidated; clear its trigger once reconciled.
    Invalidate(InvalidateRequest, std::time::Instant),
    /// A root changed with no invalidate behind it (Q10); nothing to clear.
    Root(VasKey),
}

/// The walk in flight and what it answers.
#[derive(Debug)]
struct Batch {
    /// Each want, with the objects it named and when it arrived.
    wants: Vec<(Want, Vec<VasKey>, std::time::Instant)>,
    /// Every object to reconcile.
    keys: BTreeSet<VasKey>,
    /// When the walk was submitted.
    submitted: std::time::Instant,
}

/// What the VA manager has done, cumulatively. Every refusal is counted AND named.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VaStats {
    /// Walks queued.
    pub walks_submitted: u64,
    /// Walks whose report was reconciled.
    pub walks_reconciled: u64,
    /// Walks the walker refused (at submit or at completion).
    pub walks_refused: u64,
    /// Spaces reconciled cleanly.
    pub spaces_reconciled: u64,
    /// Host maps placed.
    pub mapped: u64,
    /// Host maps removed.
    pub unmapped: u64,
    /// Host TLB invalidates issued (ONE per space per reconcile that changed anything).
    pub host_invalidates: u64,
    /// Triggers cleared by us.
    pub cleared: u64,
    /// Completions that found the trigger re-armed by a later write (§5.5) — never cleared.
    pub superseded: u64,
    /// Completions that found the trigger already idle.
    pub already_idle: u64,
    /// ★ Invalidates naming a PDB no object carries (counted; the trigger is cleared — we hold
    /// nothing for it).
    pub named_missed: u64,
    /// ★ Invalidates NOT cleared because a space they named could not be reconciled.
    pub unreconciled: u64,
    /// The first 16 refusals, verbatim.
    pub refusals: Vec<String>,
    /// Worst wall time of one [`Walker::submit`] call, in ns — the cost on the manager's thread.
    pub submit_ns_max: u64,
    /// ★ P4: walked bytes above a CPU window's extent ([`MapTarget::va_extent`]) — real in the
    /// guest's tables, no CPU address to show them at.
    pub clipped_bytes: u64,
    /// ★ P5c timing (the instrument behind the mapping-plane throughput fix): per phase, summed ns.
    pub timing: VaTiming,
}

/// ★ P5c: where an invalidate's wall time goes, summed over every one completed — never a decision
/// input. `arrive→clear = wait (queued behind a walk) + walk (submit→collected) + reconcile + apply`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VaTiming {
    /// Invalidates cleared (the denominator of `inval_ns`).
    pub invals: u64,
    /// Sum over cleared invalidates of arrival → clear.
    pub inval_ns: u64,
    /// The slowest arrival → clear.
    pub inval_ns_max: u64,
    /// Walks collected (the denominator of the next four).
    pub walks: u64,
    /// Sum of submit → collected (wall, on this thread's clock).
    pub walk_ns: u64,
    /// Sum of the walker's own GPU time.
    pub gpu_us: u64,
    /// Sum of leaf classification + diff against the ledger.
    pub plan_ns: u64,
    /// Sum of the host verbs (maps, unmaps, the invalidate).
    pub apply_ns: u64,
    /// Leaves in the last walk (all spaces).
    pub leaves_last: u64,
    /// Host verbs issued (maps + unmaps + invalidates).
    pub host_calls: u64,
}

impl VaStats {
    fn refuse(&mut self, why: String) {
        if self.refusals.len() < 16 {
            self.refusals.push(why);
        }
    }
    fn outcome(&mut self, o: ClearOutcome) {
        match o {
            ClearOutcome::Cleared => self.cleared += 1,
            ClearOutcome::Superseded => self.superseded += 1,
            ClearOutcome::AlreadyIdle => self.already_idle += 1,
        }
    }
}

/// What one [`VaManager::on_walk_ready`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reconciled {
    /// Whether a walk was actually collected (a wake with nothing finished is `false`).
    pub collected: bool,
    /// Per space: what the apply did.
    pub applied: Vec<(VasKey, Applied)>,
    /// Invalidate sequences completed, with the outcome.
    pub completed: Vec<(u64, ClearOutcome)>,
    /// Invalidate sequences left armed because a space they named failed.
    pub unreconciled: Vec<u64>,
}

/// ★★★★★ **The VA manager** — one per GPU, on one thread (§3: *"the VA manager is a thread rather
/// than a lock so that 'who may map' is a structural answer"*).
pub struct VaManager<W: Walker, T: MapTarget> {
    /// The VA-space objects.
    pub table: VasTable<T>,
    walker: W,
    store_bytes: u64,
    ram_offset: Box<dyn Fn(u64, u64) -> Option<u64> + Send>,
    pending: Vec<Want>,
    inflight: Option<Batch>,
    /// Counters and named refusals.
    pub stats: VaStats,
}

impl<W: Walker, T: MapTarget> VaManager<W, T> {
    /// A manager over a store of `store_bytes`. `ram_offset(gpa, len)` is the VMM's guest-RAM
    /// layout (the memfd offset of a sysmem leaf), `None` where it backs nothing contiguously.
    pub fn new(walker: W, store_bytes: u64, ram_offset: Box<dyn Fn(u64, u64) -> Option<u64> + Send>) -> Self {
        VaManager {
            table: VasTable::new(store_bytes),
            walker,
            store_bytes,
            ram_offset,
            pending: Vec::new(),
            inflight: None,
            stats: VaStats::default(),
        }
    }

    /// The walker (for its completion fd and counters).
    #[must_use]
    pub fn walker(&self) -> &W {
        &self.walker
    }

    /// Whether a walk is in flight.
    #[must_use]
    pub fn in_flight(&self) -> bool {
        self.inflight.is_some()
    }

    /// Requests waiting for the next walk.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// ★ A published invalidate. **Never blocks**: it queues, and submits a walk if none is in
    /// flight. `trigger` is the port's, for a request that can be cleared without a walk.
    pub fn on_invalidate(&mut self, req: InvalidateRequest, trigger: &Trigger) {
        self.pending.push(Want::Invalidate(req, std::time::Instant::now()));
        self.pump(trigger);
    }

    /// A root changed with no invalidate behind it (Q10): walk `key` at the next opportunity.
    pub fn schedule_walk(&mut self, key: VasKey, trigger: &Trigger) {
        self.pending.push(Want::Root(key));
        self.pump(trigger);
    }

    /// Start the next walk if none is in flight.
    fn pump(&mut self, trigger: &Trigger) {
        if self.inflight.is_some() || self.pending.is_empty() {
            return;
        }
        let wants = core::mem::take(&mut self.pending);
        let mut batch =
            Batch { wants: Vec::with_capacity(wants.len()), keys: BTreeSet::new(), submitted: std::time::Instant::now() };
        let mut vacuous: Vec<u64> = Vec::new();
        for w in wants {
            let keys = match w {
                Want::Invalidate(r, _) if r.inval.all_pdb => self.table.rooted(),
                // ⊘ We hold no sysmem-rooted space (`set_root` refuses them), so a sysmem PDB
                // names nothing of ours — a miss, like an unknown vidmem PDB.
                Want::Invalidate(r, _) if r.inval.pdb_aperture == PdbAperture::Sysmem => Vec::new(),
                Want::Invalidate(r, _) => self.table.keys_for_pdb(r.inval.pdb),
                Want::Root(k) => self.table.root(k).map(|_| vec![k]).unwrap_or_default(),
            };
            if let Want::Invalidate(r, _) = w {
                if keys.is_empty() {
                    if !r.inval.all_pdb {
                        self.stats.named_missed += 1;
                    }
                    vacuous.push(r.seq);
                    continue;
                }
            }
            batch.keys.extend(keys.iter().copied());
            let at = match w {
                Want::Invalidate(_, at) => at,
                Want::Root(_) => std::time::Instant::now(),
            };
            batch.wants.push((w, keys, at));
        }
        // Nothing of ours is named: nothing can be stale, so the clear is honest now.
        for seq in vacuous {
            self.stats.outcome(trigger.complete(seq));
        }
        if batch.keys.is_empty() {
            return;
        }
        let pdbs: Vec<u64> = batch
            .keys
            .iter()
            .filter_map(|&k| self.table.root(k))
            .collect::<BTreeSet<u64>>()
            .into_iter()
            .collect();
        if pdbs.len() > MAX_SPACES_PER_WALK {
            self.refuse_batch(
                &batch,
                format!("{} roots in one walk; the walk kernel's table holds {MAX_SPACES_PER_WALK}", pdbs.len()),
            );
            return;
        }
        let t0 = std::time::Instant::now();
        let r = self.walker.submit(&pdbs);
        let ns = u64::try_from(t0.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.stats.submit_ns_max = self.stats.submit_ns_max.max(ns);
        match r {
            Ok(()) => {
                self.stats.walks_submitted += 1;
                batch.submitted = std::time::Instant::now();
                self.inflight = Some(batch);
            }
            Err(e) => {
                self.stats.walks_refused += 1;
                self.refuse_batch(&batch, format!("walk submit: {e}"));
            }
        }
    }

    /// Every invalidate in `batch` stays armed; counted and named.
    fn refuse_batch(&mut self, batch: &Batch, why: String) {
        for (w, _, _) in &batch.wants {
            if matches!(w, Want::Invalidate(..)) {
                self.stats.unreconciled += 1;
            }
        }
        self.stats.refuse(why);
    }

    /// ★★★ **The walker's completion fd became readable.** Collect, reconcile every named
    /// space, and only then clear the triggers whose spaces all reconciled. Never blocks; a wake
    /// with nothing finished changes nothing.
    pub fn on_walk_ready(&mut self, trigger: &Trigger) -> Reconciled {
        let mut out = Reconciled::default();
        if self.inflight.is_none() {
            return out;
        }
        let done = match self.walker.poll() {
            Ok(None) => return out,
            Ok(Some(d)) => d,
            Err(e) => {
                self.stats.walks_refused += 1;
                if let Some(b) = self.inflight.take() {
                    for (w, _, _) in &b.wants {
                        if let Want::Invalidate(r, _) = w {
                            self.stats.unreconciled += 1;
                            out.unreconciled.push(r.seq);
                        }
                    }
                }
                self.stats.refuse(format!("walk: {e}"));
                out.collected = true;
                self.pump(trigger);
                return out;
            }
        };
        out.collected = true;
        let Some(batch) = self.inflight.take() else {
            return out;
        };
        self.stats.walks_reconciled += 1;
        let tm = &mut self.stats.timing;
        tm.walks += 1;
        tm.walk_ns += ns_since(batch.submitted);
        tm.gpu_us += done.gpu_us;
        tm.leaves_last = done.spaces.iter().map(|s| s.leaves.len() as u64).sum();
        let walked: BTreeMap<u64, &WalkedSpace> = done.spaces.iter().map(|s| (s.pdb, s)).collect();
        let mut failed: BTreeSet<VasKey> = BTreeSet::new();
        for &key in &batch.keys {
            let store = self.store_bytes;
            let Some(space) = self.table.spaces.get_mut(&key) else {
                continue; // removed while the walk ran: nothing of ours left to be stale
            };
            let Some(root) = space.root else {
                continue; // root withdrawn meanwhile; mappings stay until a walk says otherwise
            };
            let Some(ws) = walked.get(&root) else {
                // The root changed after the walk was submitted: this walk does not describe it.
                failed.insert(key);
                self.pending.push(Want::Root(key));
                self.stats.refuse(format!("{key:?}: root {root:#x} changed during the walk; re-walking"));
                continue;
            };
            // ★ A CPU window shows only the VAs its BAR decodes (`MapTarget::va_extent`).
            let clipped;
            let leaves: &[(u64, u64, u64, u8)] = match space.target.va_extent() {
                Some(extent) => {
                    let (kept, cut) = clip_leaves(&ws.leaves, extent);
                    self.stats.clipped_bytes += cut;
                    clipped = kept;
                    &clipped
                }
                None => &ws.leaves,
            };
            let t_plan = std::time::Instant::now();
            let desired = match desired_from_leaves(leaves.iter().copied(), store, &*self.ram_offset) {
                Ok(d) => d,
                Err(e) => {
                    failed.insert(key);
                    self.stats.refuse(format!("{key:?} root {root:#x}: leaf refused: {e:?}"));
                    continue;
                }
            };
            let plan = plan_reconcile(&space.ledger.rows(), &desired);
            let t_apply = std::time::Instant::now();
            self.stats.timing.plan_ns += u64::try_from((t_apply - t_plan).as_nanos()).unwrap_or(u64::MAX);
            let a = space.ledger.apply_to(&space.target, &plan);
            self.stats.timing.apply_ns += ns_since(t_apply);
            self.stats.timing.host_calls += (a.mapped + a.unmapped + a.refused) as u64 + u64::from(a.invalidated);
            self.stats.mapped += a.mapped as u64;
            self.stats.unmapped += a.unmapped as u64;
            self.stats.host_invalidates += u64::from(a.invalidated);
            if a.refused > 0 {
                failed.insert(key);
                self.stats.refuse(format!(
                    "{key:?} root {root:#x}: host refused {}: {}",
                    a.refused,
                    a.first_refusal.clone().unwrap_or_default()
                ));
            } else {
                self.stats.spaces_reconciled += 1;
            }
            out.applied.push((key, a));
        }
        // ★ THE CLEAR IS LAST: every map above has landed and its space's ONE invalidate ran.
        for (w, keys, at) in &batch.wants {
            let Want::Invalidate(r, _) = w else { continue };
            if keys.iter().any(|k| failed.contains(k)) {
                self.stats.unreconciled += 1;
                out.unreconciled.push(r.seq);
            } else {
                let o = trigger.complete(r.seq);
                self.stats.outcome(o);
                let ns = ns_since(*at);
                let tm = &mut self.stats.timing;
                tm.invals += 1;
                tm.inval_ns += ns;
                tm.inval_ns_max = tm.inval_ns_max.max(ns);
                out.completed.push((r.seq, o));
            }
        }
        self.pump(trigger);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::Desired;
    use kf_trap::{Invalidate, InvalidatePort, InvalidateRegs, PortWrite};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    const STORE: u64 = 256 << 20;
    const PDB_A: u64 = 0x0100_0000;
    const PDB_B: u64 = 0x0180_0000;
    const K_A: VasKey = VasKey(0xA);
    const K_B: VasKey = VasKey(0xB);

    /// The guest's tables, as the fake walker "walks" them: root → leaves. Shared so a test can
    /// change them between invalidates, as the guest would.
    type Tables = Rc<RefCell<BTreeMap<u64, Vec<(u64, u64, u64, u8)>>>>;

    struct FakeWalker {
        tables: Tables,
        /// Snapshot taken AT SUBMIT — what a real walk would have read.
        queued: Option<Vec<WalkedSpace>>,
        /// Polls that answer "not yet" before the result.
        not_ready: u32,
        submits: Vec<Vec<u64>>,
        fail_poll: Option<String>,
    }

    impl Walker for FakeWalker {
        fn submit(&mut self, pdbs: &[u64]) -> Result<(), String> {
            assert!(self.queued.is_none(), "one walk at a time");
            assert!(pdbs.windows(2).all(|w| w[0] < w[1]), "ascending and unique");
            self.submits.push(pdbs.to_vec());
            let t = self.tables.borrow();
            self.queued = Some(
                pdbs.iter()
                    .map(|&p| WalkedSpace { pdb: p, leaves: t.get(&p).cloned().unwrap_or_default() })
                    .collect(),
            );
            Ok(())
        }
        fn poll(&mut self) -> Result<Option<WalkDone>, String> {
            if self.not_ready > 0 {
                self.not_ready -= 1;
                return Ok(None);
            }
            if let Some(e) = self.fail_poll.take() {
                self.queued = None;
                return Err(e);
            }
            Ok(self.queued.take().map(|spaces| WalkDone { spaces, gpu_us: 1 }))
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Op {
        Map(u64, u64, u64),
        Unmap(u64),
        Invalidate,
    }

    /// Records every host op, and whether the guest's trigger was STILL BUSY when it ran —
    /// the §49.1 ordering, observed from the host side.
    struct FakeHost {
        ops: Rc<RefCell<Vec<(Op, bool)>>>,
        port: Arc<InvalidatePort>,
        refuse_map_at: Option<u64>,
    }

    impl MapTarget for FakeHost {
        fn map(&self, d: &Desired, _defer: bool) -> Result<(), String> {
            if self.refuse_map_at == Some(d.va) {
                return Err(format!("map {:#x}: refused (fake)", d.va));
            }
            self.ops.borrow_mut().push((Op::Map(d.va, d.off, d.len), self.port.trigger().read() != 0));
            Ok(())
        }
        fn unmap(&self, va: u64, _defer: bool) -> Result<(), String> {
            self.ops.borrow_mut().push((Op::Unmap(va), self.port.trigger().read() != 0));
            Ok(())
        }
        fn invalidate(&self) -> Result<(), String> {
            self.ops.borrow_mut().push((Op::Invalidate, self.port.trigger().read() != 0));
            Ok(())
        }
    }

    struct Rig {
        m: VaManager<FakeWalker, FakeHost>,
        port: Arc<InvalidatePort>,
        tables: Tables,
        ops: Rc<RefCell<Vec<(Op, bool)>>>,
    }

    fn rig() -> Rig {
        let port = Arc::new(InvalidatePort::new(InvalidateRegs::from_usermode_base(0xBB_0000).unwrap()));
        let tables: Tables = Rc::default();
        let ops: Rc<RefCell<Vec<(Op, bool)>>> = Rc::default();
        let w = FakeWalker { tables: tables.clone(), queued: None, not_ready: 0, submits: vec![], fail_poll: None };
        let mut m = VaManager::new(w, STORE, Box::new(|_, _| None));
        for k in [K_A, K_B] {
            m.table.insert(k, FakeHost { ops: ops.clone(), port: port.clone(), refuse_map_at: None });
        }
        m.table.set_root(K_A, PDB_A, PdbAperture::Vidmem).unwrap();
        m.table.set_root(K_B, PDB_B, PdbAperture::Vidmem).unwrap();
        Rig { m, port, tables, ops }
    }

    /// The guest's sequence through the PORT (PDB lo, PDB hi, TRIGGER) — what a vCPU does.
    fn guest_invalidate(port: &InvalidatePort, pdb: u64, all_pdb: bool) -> InvalidateRequest {
        let r = port.regs();
        let (lo, hi) = Invalidate::encode_pdb(pdb, PdbAperture::Vidmem);
        port.write(r.pdb, lo);
        port.write(r.upper_pdb, hi);
        let word = kf_trap::mmuinval::TRIGGER_BIT | 1 | if all_pdb { 0b10 } else { 0 };
        match port.write(r.trigger, word) {
            PortWrite::Publish(req) => req,
            other => panic!("trigger did not publish: {other:?}"),
        }
    }

    fn busy(port: &InvalidatePort) -> bool {
        port.read(port.regs().trigger).unwrap() & kf_trap::mmuinval::TRIGGER_BIT != 0
    }

    /// ★★★ THE P4 PROPERTY: the trigger reads busy until the host mapping is committed, and every
    /// host op of the reconcile ran while it was still busy.
    #[test]
    fn the_trigger_clears_only_after_the_host_mapping_is_committed() {
        let mut r = rig();
        r.tables.borrow_mut().insert(PDB_A, vec![(0x20_0000_0000, 0x0200_0000, 0x1_0000, 0)]);
        let req = guest_invalidate(&r.port, PDB_A, false);
        r.m.on_invalidate(req, r.port.trigger());
        assert!(busy(&r.port), "busy after submit: the walk has not even run");
        assert!(r.ops.borrow().is_empty(), "nothing mapped before the walk completes");
        assert_eq!(r.m.walker().submits, vec![vec![PDB_A]]);

        let out = r.m.on_walk_ready(r.port.trigger());
        assert_eq!(out.completed, vec![(req.seq, ClearOutcome::Cleared)]);
        assert!(!busy(&r.port), "cleared after the reconcile");
        let ops = r.ops.borrow();
        assert_eq!(
            ops.iter().map(|(o, _)| o.clone()).collect::<Vec<_>>(),
            vec![Op::Map(0x20_0000_0000, 0x0200_0000, 0x1_0000), Op::Invalidate],
            "one deferred map, then ONE invalidate"
        );
        assert!(ops.iter().all(|(_, b)| *b), "every host op ran while the guest still saw busy");
    }

    #[test]
    fn a_wake_before_the_walk_finishes_changes_nothing() {
        let mut r = rig();
        r.tables.borrow_mut().insert(PDB_A, vec![(0x1000_0000, 0x0200_0000, 0x1000, 0)]);
        r.m.walker.not_ready = 2;
        let req = guest_invalidate(&r.port, PDB_A, false);
        r.m.on_invalidate(req, r.port.trigger());
        for _ in 0..2 {
            let out = r.m.on_walk_ready(r.port.trigger());
            assert!(!out.collected);
            assert!(busy(&r.port));
        }
        assert!(r.m.on_walk_ready(r.port.trigger()).collected);
        assert!(!busy(&r.port));
    }

    #[test]
    fn an_unknown_pdb_is_counted_and_cleared_without_a_walk() {
        let mut r = rig();
        let req = guest_invalidate(&r.port, 0x0900_0000, false);
        r.m.on_invalidate(req, r.port.trigger());
        assert_eq!(r.m.stats.named_missed, 1);
        assert!(r.m.walker().submits.is_empty(), "nothing of ours to walk");
        assert!(!busy(&r.port), "we hold nothing for it, so nothing can be stale");
    }

    /// §5.5: A in flight, B arrives (the guest gave up on A). A's completion must NOT clear;
    /// B gets its OWN walk — submitted after its arm — and only that clears.
    #[test]
    fn a_rearmed_trigger_is_superseded_and_the_next_walk_clears_it() {
        let mut r = rig();
        r.tables.borrow_mut().insert(PDB_A, vec![(0x1000_0000, 0x0200_0000, 0x1000, 0)]);
        let a = guest_invalidate(&r.port, PDB_A, false);
        r.m.on_invalidate(a, r.port.trigger());
        // The guest rewrites its tables, then invalidates again while A's walk is in flight.
        r.tables.borrow_mut().insert(PDB_A, vec![(0x1000_0000, 0x0300_0000, 0x1000, 0)]);
        let b = guest_invalidate(&r.port, PDB_A, false);
        r.m.on_invalidate(b, r.port.trigger());
        assert_eq!(r.m.pending(), 1, "B waits for the NEXT walk");

        let out = r.m.on_walk_ready(r.port.trigger());
        assert_eq!(out.completed, vec![(a.seq, ClearOutcome::Superseded)]);
        assert!(busy(&r.port), "A's completion must not clear B");
        assert!(r.m.in_flight(), "B's walk was submitted at once");

        let out = r.m.on_walk_ready(r.port.trigger());
        assert_eq!(out.completed, vec![(b.seq, ClearOutcome::Cleared)]);
        assert!(!busy(&r.port));
        assert_eq!(r.m.walker().submits.len(), 2);
        let ledger = r.m.table.ledger(K_A).unwrap();
        assert_eq!(ledger.resolve(0x1000_0000, 0x1000), Some((false, 0x0300_0000)), "B's tables won");
    }

    #[test]
    fn requests_arriving_during_a_walk_coalesce_into_one_follow_up_walk() {
        let mut r = rig();
        let a = guest_invalidate(&r.port, PDB_A, false);
        r.m.on_invalidate(a, r.port.trigger());
        for _ in 0..3 {
            let x = guest_invalidate(&r.port, PDB_B, false);
            r.m.on_invalidate(x, r.port.trigger());
        }
        r.m.on_walk_ready(r.port.trigger());
        r.m.on_walk_ready(r.port.trigger());
        assert_eq!(r.m.walker().submits, vec![vec![PDB_A], vec![PDB_B]]);
        assert!(!busy(&r.port));
        assert_eq!(r.m.stats.cleared, 1);
        assert_eq!(r.m.stats.superseded, 3, "A and the two earlier B writes");
    }

    #[test]
    fn all_pdb_walks_and_reconciles_every_rooted_space() {
        let mut r = rig();
        r.tables.borrow_mut().insert(PDB_A, vec![(0x1000_0000, 0x0200_0000, 0x1000, 0)]);
        r.tables.borrow_mut().insert(PDB_B, vec![(0x2000_0000, 0x0210_0000, 0x2000, 0)]);
        let req = guest_invalidate(&r.port, 0, true);
        r.m.on_invalidate(req, r.port.trigger());
        assert_eq!(r.m.walker().submits, vec![vec![PDB_A, PDB_B]]);
        let out = r.m.on_walk_ready(r.port.trigger());
        assert_eq!(out.applied.len(), 2);
        assert_eq!(r.m.stats.named_missed, 0);
        assert!(!busy(&r.port));
        assert_eq!(r.m.stats.host_invalidates, 2, "ONE per space");
    }

    #[test]
    fn a_remap_unmaps_the_old_run_and_maps_the_new_one() {
        let mut r = rig();
        r.tables.borrow_mut().insert(PDB_A, vec![(0x1000_0000, 0x0200_0000, 0x1000, 0), (0x1100_0000, 0x0210_0000, 0x1000, 0)]);
        let q = guest_invalidate(&r.port, PDB_A, false);
        r.m.on_invalidate(q, r.port.trigger());
        r.m.on_walk_ready(r.port.trigger());
        r.ops.borrow_mut().clear();
        r.tables.borrow_mut().insert(PDB_A, vec![(0x1000_0000, 0x0200_0000, 0x1000, 0), (0x1100_0000, 0x0220_0000, 0x1000, 0)]);
        let q = guest_invalidate(&r.port, PDB_A, false);
        r.m.on_invalidate(q, r.port.trigger());
        let out = r.m.on_walk_ready(r.port.trigger());
        let a = &out.applied[0].1;
        assert_eq!((a.mapped, a.unmapped), (1, 1));
        assert_eq!(
            r.ops.borrow().iter().map(|(o, _)| o.clone()).collect::<Vec<_>>(),
            vec![Op::Unmap(0x1100_0000), Op::Map(0x1100_0000, 0x0220_0000, 0x1000), Op::Invalidate]
        );
        assert!(!busy(&r.port));
    }

    #[test]
    fn a_failed_walk_leaves_the_trigger_armed_and_names_why() {
        let mut r = rig();
        r.m.walker.fail_poll = Some("device fault (fake)".into());
        let q = guest_invalidate(&r.port, PDB_A, false);
        r.m.on_invalidate(q, r.port.trigger());
        let out = r.m.on_walk_ready(r.port.trigger());
        assert_eq!(out.unreconciled, vec![q.seq]);
        assert!(busy(&r.port), "an unreconciled invalidate must never read complete");
        assert_eq!(r.m.stats.walks_refused, 1);
        assert!(r.m.stats.refusals[0].contains("device fault"));
    }

    #[test]
    fn a_host_refusal_leaves_the_trigger_armed() {
        let mut r = rig();
        r.m.table.insert(
            K_A,
            FakeHost { ops: r.ops.clone(), port: r.port.clone(), refuse_map_at: Some(0x1000_0000) },
        );
        r.m.table.set_root(K_A, PDB_A, PdbAperture::Vidmem).unwrap();
        r.tables.borrow_mut().insert(PDB_A, vec![(0x1000_0000, 0x0200_0000, 0x1000, 0)]);
        let q = guest_invalidate(&r.port, PDB_A, false);
        r.m.on_invalidate(q, r.port.trigger());
        let out = r.m.on_walk_ready(r.port.trigger());
        assert_eq!(out.unreconciled, vec![q.seq]);
        assert!(busy(&r.port));
        assert!(r.m.stats.refusals[0].contains("refused (fake)"));
    }

    #[test]
    fn a_leaf_outside_the_store_is_refused_and_the_trigger_stays_armed() {
        let mut r = rig();
        r.tables.borrow_mut().insert(PDB_A, vec![(0x1000_0000, STORE - 0x1000, 0x2000, 0)]);
        let q = guest_invalidate(&r.port, PDB_A, false);
        r.m.on_invalidate(q, r.port.trigger());
        let out = r.m.on_walk_ready(r.port.trigger());
        assert_eq!(out.unreconciled, vec![q.seq]);
        assert!(r.ops.borrow().is_empty(), "nothing mapped");
        assert!(r.m.stats.refusals[0].contains("OutsideStore"));
    }

    #[test]
    fn roots_are_refused_by_name() {
        let mut r = rig();
        assert!(matches!(
            r.m.table.set_root(K_A, 0x1000, PdbAperture::Sysmem),
            Err(RootRefusal::SysmemRoot { .. })
        ));
        assert!(matches!(
            r.m.table.set_root(K_A, STORE, PdbAperture::Vidmem),
            Err(RootRefusal::OutsideStore { .. })
        ));
        assert!(matches!(
            r.m.table.set_root(VasKey(99), 0x1000, PdbAperture::Vidmem),
            Err(RootRefusal::UnknownObject(_))
        ));
        assert_eq!(r.m.table.set_root(K_A, PDB_A, PdbAperture::Vidmem), Ok(false), "unchanged");
        assert_eq!(r.m.table.set_root(K_A, 0x0110_0000, PdbAperture::Vidmem), Ok(true), "changed");
    }

    /// Q10: a root change with no invalidate schedules a walk; nothing is cleared (nothing armed).
    #[test]
    fn a_root_change_schedules_a_walk_without_touching_the_trigger() {
        let mut r = rig();
        r.tables.borrow_mut().insert(PDB_B, vec![(0x3000_0000, 0x0200_0000, 0x1000, 0)]);
        r.m.schedule_walk(K_B, r.port.trigger());
        assert_eq!(r.m.walker().submits, vec![vec![PDB_B]]);
        let out = r.m.on_walk_ready(r.port.trigger());
        assert!(out.completed.is_empty());
        assert_eq!(out.applied[0].1.mapped, 1);
        assert_eq!(r.port.trigger().issued(), 0);
    }
}
