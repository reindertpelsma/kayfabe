//! # The page-table decode pass — `#102` stage C3
//!
//! Two things live here, and they are the two halves §12.2's ruling splits the world
//! into:
//!
//! - [`IsolateFb`] — **the production [`FbRead`]**. The core holds the address table and
//!   decides *what*; the **isolate** holds bytes and does *it*. This type is the seam
//!   between those sentences and it holds no bytes of its own.
//! - [`plan_pt_decode`] / [`run_pt_decode`] / [`commit_pt_decode`] — the pass, in the
//!   three phases the lock discipline requires, because **the middle one blocks**.
//!
//! ## Why three phases and not one function
//!
//! R1 forbids a blocking call under a ranked lock, and a decode is nothing *but*
//! blocking calls: one round trip to the isolate per page-table page. So the shape is
//! forced, and it is the same shape everything else on this plane has —
//!
//! | phase | lock | what it does |
//! |---|---|---|
//! | [`plan_pt_decode`] | the owner's, rank 1 | drain the dirty pages, resolve each one's level |
//! | [`run_pt_decode`] | **none** | read and decode, over the isolate |
//! | [`commit_pt_decode`] | the owner's, rank 1 again | re-validate (R5), populate, learn |
//!
//! ★ It is also why the latch is already its own pass: stage B had to separate latching
//! from `apply_pushbuffer` for the *other* R3 reason (the page's owner is routinely not
//! the issuing proc). The two separations compose — plan and commit visit owners one
//! lock at a time, exactly as `latch_pt_writes` does.
//!
//! ## What triggers it, and the thing that is NOT true
//!
//! The commit point is the guest's **CE release semaphore** — *"decode each dirtied page
//! DIRECTLY … the release is the guest's own commit point for those PTEs"*
//! (`C: nvkvm_gpu_emul.c:8676-8695`). It is **not** an invalidate — and the two halves of
//! that sentence rest on different things, which this note used to run together:
//!
//! - **In the C: MEASURED.** Both invalidate transports counted ZERO on the GSP-emulated
//!   compute path (`INVALIDATE_TLB` RPC fn=200, and the `MEM_OP`/`MMU_TLB_INVALIDATE`
//!   pushbuffer method), as did `DMA_FILL_PTE_MEM`. That is a run, on hardware, and it is
//!   the audit that refuted the read-at-invalidate model repo-wide.
//! - **In this port: NOT MEASURED.** `#102` stage C2 (§13.4) is an argument that the same
//!   holds here, carried across from the C. It has not been re-run against this code.
//!
//! The design consequence is the same either way and is not softened by the distinction:
//! correctness rests on *witnessing* the write. There is no second chance for a write
//! nobody saw, and nothing here is designed as if there were. ★ The reason to keep the
//! two apart is that this exact claim, stated in one breath, is the one CLAUDE.md
//! repeated for weeks after its own design doc had corrected it.

use std::collections::{BTreeMap, BTreeSet};

use kayfabe_arch::GmmuFmt;
use kayfabe_arch::ids::GpuVa;
use kayfabe_arch::ids::{GpuId, Pdb};
use kayfabe_core::gpu::Proc;
use kayfabe_isolate::{RmError, Worker};
use kayfabe_mmu::reach::ReachFault;
use kayfabe_mmu::walker::{
    DropReason, FbRead, PopulateRefusal, PtPage, SubtreeDecode, WalkFault, decode_subtree,
};

/// ★★★ **THE PRODUCTION [`FbRead`]** — page-table bytes, read out of the isolate's
/// mapping of the fabricated aperture (`eight_blockers_resolved.md` §12.2).
///
/// ## What it reads from, and why that is the isolate
///
/// §12's ruling decomposes a copy by *representability*: an operand naming space we
/// invented is unrepresentable to a real engine, so **we** perform that copy — in the
/// isolate, against the isolate's VRAM-backed mapping of that space. The consequence is
/// the one that unblocked this stage: **every byte in the fabricated aperture was put
/// there by us**, so the guest's page tables are already in the isolate's hands. Nothing
/// needs to store them a second time.
///
/// That is also why this cannot be the core. The rejected design (§11.6 Option 3) was a
/// core-owned store of intercepted payloads, and it failed for three independent reasons,
/// the sharpest being the **orphan leaf**: a fresh page-table page is filled *before* any
/// PDE points at it, so at fill time nothing classifies it as a page table and a
/// payload-keyed store is empty exactly where the decode reads. Under this design the
/// question never arises — the criterion is the **address**, the bytes were ours from the
/// first write, and the decode reads memory rather than replaying observations.
///
/// ## What it is not
///
/// It holds **no bytes**. It is a borrow of a checked-out [`Worker`] plus two counters,
/// and [`FbRead`] has no method that could hand content *in*, so "the core acquired a
/// device-memory store" is not a state this type can be refactored into by accident.
///
/// ★ A transport failure is **kept, not folded into the miss**. `Ok(false)` from the
/// isolate is a fact about the *guest* (its page table names a page outside the aperture
/// — MISS = FAULT); an `Err` is a fact about *us*. Both make the read fail, but a caller
/// that cannot tell them apart will spend a day debugging the guest's page tables when
/// the socket is what broke. [`IsolateFb::transport_error`] is how the pass tells.
pub struct IsolateFb<'a> {
    worker: &'a mut Worker,
    /// The FIRST transport error, kept rather than the last: the one that started it is
    /// the diagnostic, and everything after a broken connection is downstream noise.
    transport: Option<RmError>,
    reads: usize,
    misses: usize,
}

impl core::fmt::Debug for IsolateFb<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IsolateFb")
            .field("transport", &self.transport)
            .field("reads", &self.reads)
            .field("misses", &self.misses)
            .finish()
    }
}

impl<'a> IsolateFb<'a> {
    /// Read page-table bytes through `worker`'s isolate.
    #[must_use]
    pub fn new(worker: &'a mut Worker) -> Self {
        IsolateFb {
            worker,
            transport: None,
            reads: 0,
            misses: 0,
        }
    }

    /// The first transport failure this source hit, if any. `None` means every refusal
    /// it reported was the aperture honestly not covering an address.
    #[must_use]
    pub fn transport_error(&self) -> Option<RmError> {
        self.transport
    }

    /// How many reads were attempted. Counted so a test can prove the source was
    /// **reached** — a decode that produced no leaves because it was never asked for a
    /// byte looks identical to one that read an empty tree.
    #[must_use]
    pub fn reads(&self) -> usize {
        self.reads
    }

    /// How many of those the aperture did not cover.
    #[must_use]
    pub fn misses(&self) -> usize {
        self.misses
    }
}

impl FbRead for IsolateFb<'_> {
    fn read(&mut self, phys: u64, buf: &mut [u8]) -> bool {
        self.reads += 1;
        match self.worker.fb_read(phys, buf) {
            Ok(true) => true,
            Ok(false) => {
                self.misses += 1;
                false
            }
            Err(e) => {
                self.transport.get_or_insert(e);
                false
            }
        }
    }
}

/// Entries the decode pass may examine in one run, across every task.
///
/// The C bounds the same walk at `300000` (`C: nvkvm_gpu_emul.c:8759`) and then simply
/// stops; here exhausting it is a **loud** [`WalkFault::BudgetExhausted`], because a
/// partial capture presented as a complete one is how a mapping silently goes missing.
pub const PT_DECODE_BUDGET: u32 = 300_000;

/// ★★★★★ Entries the **whole-VAS sweep** may examine — **per address space**, not per run.
///
/// # ⊘⊘ THE SAME NUMBER AS [`PT_DECODE_BUDGET`] IS NOT THE SAME BUDGET
///
/// The C's `300000` (`C: nvkvm_gpu_emul.c:8759`) bounds **one walk of one VAS from its root**.
/// [`PT_DECODE_BUDGET`] spends the identical constant across **every task of a whole run**,
/// where a task is one dirtied page and the run drains a per-doorbell delta. Two different
/// quantities wearing one number: reusing it for a sweep would divide the C's per-VAS budget by
/// the number of address spaces and truncate the walk for a reason that has nothing to do with
/// the walk.
///
/// ⇒ This is charged **per task** by [`run_pt_sweep`], which is what makes it the C's number
/// rather than a coincidence with it.
///
/// ## What it buys, in pages
///
/// A GA10x directory page is 512 entries, so this is ~585 page-table pages per address space —
/// and `decode_subtree` charges what a page *contains*, not what it could contain, so a sparse
/// tree costs less. ⚠ That figure is a **derivation, not a measurement**: how many pages a real
/// `cuCtxCreate` VAS actually has is what [`PtDecodeOutcome::pages_swept`] exists to report, and
/// this constant must be re-derived from that number rather than defended by arithmetic.
///
/// ★ Exhausting it is LOUD ([`kayfabe_mmu::walker::WalkFault::BudgetExhausted`]) **and sets
/// [`kayfabe_core::gpu::PtSweepState::truncated`]**, which forces another sweep. The C carries
/// `m2_gr_pt_trunc` for exactly that reason: a truncated walk that does not force a re-walk
/// silently loses every mapping past the cut.
pub const PT_SWEEP_BUDGET: u32 = 300_000;

/// How many page-table pages one `Vas` may remember the level of.
///
/// The metadata is forward-populated from what the guest's own tables point at, so its
/// size is guest-influenced and needs a bound (boundary-1). At the measured regime's
/// geometry this is ~256 MiB of page tables for a single address space — far past any
/// real working set — and reaching it is reported ([`PtDecodeOutcome::meta_refused`]),
/// never silently absorbed.
pub const MAX_PT_META: usize = 1 << 16;

/// One page-table page the pass will decode **directly** — not by walking to it from a
/// root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtDecodeTask {
    /// The GPU the page lives on.
    pub gpu: GpuId,
    /// The `Vas` that owns it.
    pub pdb: Pdb,
    /// Where it is, what level it is, and what virtual address its entry 0 describes.
    pub page: PtPage,
}

/// What [`plan_pt_decode`] found under the owner's lock.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PtDecodePlan {
    /// Pages to decode, in `(gpu, pdb, phys)` order.
    pub tasks: Vec<PtDecodeTask>,
    /// ★★★ Dirty pages whose **level is not yet known**, and which are therefore
    /// correctly decoded by **nobody, now**.
    ///
    /// This is §12.1(i)'s orphan leaf, and it is not a gap: a page nothing points at yet
    /// has bytes that are *already ours*, sitting in the fabricated aperture. When the
    /// guest links it — which is itself a write to the parent, so the parent becomes
    /// dirty — the descent from that parent reads this page's committed entries out of
    /// the aperture. Deferring costs nothing precisely because the content source is
    /// memory rather than a replay of observed writes; under the rejected design the same
    /// deferral would have lost the payload for good.
    ///
    /// Reported rather than counted so a test can name the page.
    pub deferred: Vec<(GpuId, Pdb, u64)>,
}

/// The decode of one task, or the fault that stopped it before it began.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtDecodeResult {
    /// The task this answers.
    pub task: PtDecodeTask,
    /// What the descent produced. `Err` is the whole-result-untrustworthy case only.
    pub decode: Result<SubtreeDecode, WalkFault>,
}

/// What the pass did, once committed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PtDecodeOutcome {
    /// ★★★★★ **w329 — host-published rows this pass REVOKED**, each with everything the shell
    /// needs to release both halves: the framebuffer offset the join is keyed by, and the
    /// `(host_va, memory)` the isolate must unmap and free.
    ///
    /// ⚠ **Non-empty only under [`kayfabe_mmu::reach::PublishedUnbind::RevokeWholeJoins`], and
    /// then it is an OBLIGATION.** The rows are already out of the table; nothing else names
    /// their host objects. See [`commit_pt_decode_revoking`].
    pub revoked: Vec<RevokedLeaf>,
    /// ★ How many of [`Self::revoked`] name a framebuffer offset the SAME settlement also wants
    /// bound — the one lossy sub-case, counted rather than assumed absent. See
    /// [`kayfabe_mmu::reach::ApplyOutcome::revoked_still_desired`].
    pub revoked_still_desired: usize,
    /// ★★★ How many host-published unbinds were kept as refusals because they are
    /// RE-MAPS, not removals — the population a re-point path would serve and this one must
    /// not touch. See [`kayfabe_mmu::reach::ApplyOutcome::remaps_refused`].
    pub remaps_refused: usize,
    /// Leaves forward-populated into a free range.
    pub bound: usize,
    /// Leaves that restated a binding already in the table.
    pub unchanged: usize,
    /// Leaves that re-pointed an existing, unpublished binding.
    pub repointed: usize,
    /// ★★ Leaves decoded faithfully and dropped by **policy**, with the reason. The
    /// 512 MiB whole-framebuffer alias lands here — decoded at the walker, dropped at the
    /// binding site, and both halves separately assertable.
    pub dropped: Vec<(GpuVa, DropReason)>,
    /// Leaves the table refused. Loud.
    pub refusals: Vec<PopulateRefusal>,
    /// ★★★ Branches the decode could not read, carried out of the pass rather than
    /// absorbed — MISS = FAULT. The subtree under each contributed nothing and was **not**
    /// guessed at.
    pub faults: Vec<WalkFault>,
    /// Tasks whose `Vas` had disappeared between the plan and the commit. **R5**: the
    /// commit re-resolves and skips, it does not re-attach a dirty page to whatever
    /// inherited the id — the C's never-pruned-table aliasing class.
    pub vas_gone: usize,
    /// Pages whose level could not be remembered because [`MAX_PT_META`] was reached.
    pub meta_refused: usize,
    /// Pages whose level **was** learned this pass, so a later direct decode of them
    /// knows what they are.
    pub meta_learned: usize,
    /// ★★★ E8 — **which** pages were learned, so the shell can publish them into the
    /// device-global ownership index ([`kayfabe_core::gpu::Spine::pt_learned`]).
    ///
    /// ⊘ Carried out as data rather than published from here, and that is the whole
    /// increment. The index is a rank-0 structure and this function holds a rank-1
    /// `&mut Proc`; reaching up would invert the lock order. The pass therefore *reports*
    /// what it learned and a fourth PUBLISH phase installs it after every ranked lock has
    /// been released — see `SharedDevice::decode_pt_writes`.
    ///
    /// ★ It parallels [`Self::meta_learned`] exactly (one entry per increment), which is
    /// an equality a test can hold the two to.
    pub learned_pages: Vec<(GpuId, Pdb, u64)>,
    /// ★★★ E8 — pages the PUBLISH phase actually installed in the device-global index.
    ///
    /// ⊘ **Not equal to [`Self::meta_learned`], and the gap is the point.** A page is
    /// learned under the owner's rank-1 lock and published under rank 0 with every guard
    /// in between released; anything that died in the gap is dropped by R5. `learned > 0`
    /// with `published == 0` is the address space having gone away mid-pass — a correct
    /// outcome, and one only a separate counter can show.
    ///
    /// ⚠ Written by the shell, not by [`commit_pt_decode`], which cannot reach rank 0.
    pub pages_published: usize,
    /// Publications the index refused — capacity ([`kayfabe_core::gpu::MAX_PT_LEARNED`]),
    /// a dead or re-homed address space (R5), or a page already owned by a *different*
    /// `(proc, pdb)`.
    ///
    /// ★ The last of those is the one worth watching: it means two address spaces claim one
    /// physical page-table page, which is either guest aliasing or a wrong decode, and the
    /// index refuses rather than letting the most recent writer win.
    pub pages_publish_refused: usize,
    /// ★★ The first **transport** failure the byte source hit, if any — set by the shell
    /// from [`IsolateFb::transport_error`] after the execute phase.
    ///
    /// Kept apart from [`PtDecodeOutcome::faults`] on purpose. A `WalkFault::Unbacked` is
    /// a statement about the *guest* (its page table names a page our aperture does not
    /// cover); this is a statement about *us* (the isolate connection broke). They look
    /// identical from inside the walker — both make a read fail — and telling them apart
    /// is the difference between debugging a guest's page tables and debugging a socket.
    pub transport: Option<RmError>,
    /// ★★ Ranges the reachability settlement **removed** from the table — a teardown
    /// (`reachability_on_transition.md` §3.3), a valid→sparse transition (§3.6), or a leaf
    /// that stopped being reachable. Counted, because *"the guest unmapped and we followed"*
    /// is otherwise indistinguishable from *"nothing happened"*.
    pub unbound: usize,
    /// ★★★ Page-table pages **retired** from the shadow: they were reachable and are not any
    /// more, so their level metadata was dropped too. Reported rather than counted so a test
    /// can name the page — this is the half of hole 3 that stops a recycled page's next
    /// contents being misparsed as page-table entries.
    pub retired: Vec<u64>,
    /// ★★ Slots whose ONLY change was the read-only bit. Named, never folded into
    /// `unchanged`: [`kayfabe_mmu::Binding`] carries no rights, so this is a mapping change
    /// this port sees, reports and does not model (`reachability_on_transition.md` §3.4).
    pub protection_changes: Vec<GpuVa>,
    /// ★★★ Leaves in a **reachable** page the guest was never seen to write. They stay a
    /// MISS, which is a fault, which is `resume_from_fault.md` §7 step 5. This is §6.1's
    /// witness rule as a number: a non-zero here is the design working, not failing.
    pub unwitnessed: usize,
    /// Leaves in a **witnessed** page nothing points at yet — the orphan, waiting for its
    /// link. Also a miss, and also on purpose.
    pub unreachable: usize,
    /// Slots the guest declared **sparse** in a reachable, witnessed page (§3.6, hole 6).
    pub sparse: usize,
    /// ★ Refusals from the shadow itself — a root that is not the address space's, or a
    /// bound reached. Kept apart from [`PtDecodeOutcome::faults`] (a statement about the
    /// guest's page tables) and from [`PtDecodeOutcome::transport`] (a statement about us):
    /// this is a statement about our **shadow**, and three different things to debug should
    /// not share one field.
    pub reach_faults: Vec<ReachFault>,
    /// ★★★★★ Leaves desired **only because a sweep admitted their page** — the price of the
    /// relaxation, as a number. See [`kayfabe_mmu::reach::ReachShadow::witness_swept`].
    ///
    /// ⊘ A pass with `bound > 0` and `swept_binds == 0` published nothing the witness rule
    /// would not have published anyway; a pass with `swept_binds == bound` published nothing
    /// else. Only this field can tell those apart, and *"did relaxing the gate do anything"* is
    /// the question the owner's ruling has to be judged on.
    pub swept_binds: usize,
    /// Whole-VAS sweeps that **completed** this pass (a root descent that did not exhaust its
    /// budget).
    pub sweeps_run: usize,
    /// Whole-VAS sweeps that hit [`PT_SWEEP_BUDGET`]. Each one has set
    /// [`kayfabe_core::gpu::PtSweepState::truncated`], so each forces another sweep.
    ///
    /// ⚠ **A truncated sweep is not a smaller sweep.** `decode_subtree` returns `Err` for the
    /// whole task, so a truncated walk contributes **no leaves at all** — it is not a partial
    /// picture, it is no picture. Reading a non-zero here as *"we got most of it"* is the
    /// `dlen=0` mistake in a different plane.
    pub sweeps_truncated: usize,
    /// Page-table pages a completed sweep visited, summed. ★ The measurement
    /// [`PT_SWEEP_BUDGET`] must be re-derived from.
    pub pages_swept: usize,
    /// ★★★★★ **THE THIRD OUTCOME** — leaves that were decoded, reachable, witnessed and
    /// policy-approved and were then displaced inside the settlement's own desired-set map,
    /// so they reached neither `bound` nor `refusals`. See
    /// [`kayfabe_mmu::reach::Settlement::shape_collisions`]; a non-zero here is the answer to
    /// *"the faulting VA is in no bound list and in no refused list — where did it go"*.
    pub shape_collisions: Vec<kayfabe_mmu::reach::ShapeCollision>,
    /// Byte-identical leaves seen twice. Benign, and kept apart from
    /// [`Self::shape_collisions`] so a duplicate cannot read as a contradiction.
    pub duplicate_leaves: usize,
}

impl PtDecodeOutcome {
    /// Did anything go wrong that a caller must look at?
    ///
    /// ★ [`PtDecodeOutcome::unwitnessed`] and [`PtDecodeOutcome::unreachable`] are
    /// deliberately **not** in here. They are the witness and reachability gates doing their
    /// job; a pass that declines to bind an unwitnessed leaf is a correct pass, and folding
    /// them in would make the normal case read as an error.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.refusals.is_empty()
            && self.faults.is_empty()
            && self.reach_faults.is_empty()
            && self.meta_refused == 0
            && self.transport.is_none()
    }
}

/// ★ **PLAN** (owner's lock, rank 1): drain `proc`'s dirtied page-table pages into decode
/// tasks.
///
/// Draining rather than reading is deliberate: a page decoded here and re-written a
/// microsecond later must be dirty *again*, and leaving it set would make the second
/// write indistinguishable from the first. The set is the guest's own signal and it is
/// consumed.
///
/// A page whose level is unknown is dropped from the dirty set and reported in
/// [`PtDecodePlan::deferred`] — see that field for why that is correct and not a loss.
pub fn plan_pt_decode(proc: &mut Proc) -> PtDecodePlan {
    let mut plan = PtDecodePlan::default();
    for (&(gpu, pdb), vas) in &mut proc.vases {
        let dirty = std::mem::take(&mut vas.pt_pages);
        // Level 0 is a DECLARED fact: a PDB *is* its own root page. Everything deeper is
        // learned forward, from a decode that reached it.
        let root = pdb.0 & !0xfff;
        for page in dirty {
            // ★★★ WITNESS, taken HERE and not at the commit — `reachability_on_transition.md`
            // §2.2. The dirty set is *drained* (a page written again must be dirty again), so
            // the record has to be taken at the drain: a page witnessed in this pass and linked
            // in the next would otherwise have no witness left by the time its link arrives, and
            // would never bind. It is taken for DEFERRED pages too, which is the same case —
            // §12.1(i)'s orphan leaf is exactly a page whose write is seen long before its link.
            vas.reach.witness(page);
            let known = if page == root {
                Some(PtPage {
                    phys: root,
                    aperture: kayfabe_arch::Aperture::Vidmem,
                    level: 0,
                    vabase: 0,
                })
            } else {
                vas.pt_meta.get(&page).copied()
            };
            match known {
                Some(p) => {
                    // ★★★★★ **THE C's `m2_gr_vas_dirty`, set at exactly the C's condition**
                    // (`C: nvkvm_gpu_emul.c:583-591`): *a guest write to any TRACKED page-table
                    // page* invalidates the sweep's picture of this address space, so the next
                    // doorbell must re-sweep.
                    //
                    // ⊘ "Tracked" is `known.is_some()` and not "any dirty page", and the
                    // difference is the whole trigger. A page whose level we do not know is
                    // §12.1(i)'s orphan — nothing points at it yet, so it is not part of the
                    // picture a sweep drew and cannot have staled it. Widening this to every
                    // dirty page would arm a sweep on every doorbell forever and the trigger
                    // would stop being a trigger.
                    //
                    // ★★ This is the half that makes the relaxation self-healing: a page that
                    // was mid-update when the sweep read it was BY DEFINITION being written, so
                    // it arrives here, so the torn window closes on the next doorbell.
                    vas.sweep.dirty = true;
                    plan.tasks.push(PtDecodeTask { gpu, pdb, page: p });
                }
                None => plan.deferred.push((gpu, pdb, page)),
            }
        }
    }
    plan
}

/// Why one address space is being swept. ⊘ Reported, never inferred: *"we swept everything"*
/// and *"we swept the two that had gone stale"* are different facts about the same log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SweepReason {
    /// It has never been swept — the C's *"`chan_vas_n` grew"* case. A fresh address space has
    /// no picture at all, so there is nothing for a dirty bit to invalidate.
    NeverSwept,
    /// Its last sweep hit [`PT_SWEEP_BUDGET`] — the C's `m2_gr_pt_trunc`. ★ Ranked **above**
    /// `Dirty` because a truncated walk produced no leaves, which is a strictly worse picture
    /// than a stale one.
    Truncated,
    /// The guest wrote a page-table page this address space's sweep is tracking.
    Dirty,
}

/// What [`plan_pt_sweep`] decided, under the owner's lock.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PtSweepPlan {
    /// One **root** task per address space that needs a walk, in `(gpu, pdb)` order.
    pub tasks: Vec<PtDecodeTask>,
    /// Why each task is there, positionally aligned with [`Self::tasks`].
    pub reasons: Vec<SweepReason>,
    /// Address spaces whose picture is current, so nothing was walked.
    ///
    /// ⊘ Counted rather than dropped: *"the sweep ran and skipped every VAS"* and *"the sweep
    /// did not run"* produce the same absence of tasks, and only this number separates them.
    pub skipped: usize,
}

/// ★★★★★ **PLAN THE WHOLE-VAS SWEEP** (owner's lock, rank 1) — the C's `enum_gr_sysmem`
/// trigger set, ported (`C: nvkvm_gpu_emul.c:583-591`).
///
/// Seeds **one task at the address space's own page-directory root**, for every address space
/// whose picture is not current. The descent from that root is what
/// [`kayfabe_mmu::reach::ReachShadow::witness_swept`] admits, and the three triggers below are
/// the other half of that function's safety argument — read it before changing them.
///
/// | trigger | the C's | why it cannot be dropped |
/// |---|---|---|
/// | never swept | `chan_vas_n` grew | a new address space has no picture to stale |
/// | truncated | `m2_gr_pt_trunc` | a budget-cut walk produced **no** leaves, not fewer |
/// | dirty | `m2_gr_vas_dirty` | the self-healing half: a torn read comes back as a write |
///
/// ⊘ **Deliberately NOT ported: the C's sparse periodic net.** The C runs a low-rate sweep to
/// bound *"any missed trigger"*. A periodic sweep here would make a missed trigger unobservable
/// — the picture would repair itself on a timer and no log would ever show the trigger was
/// wrong — and this port's whole difficulty has been instruments that hide their own failures.
/// If a trigger is missed, that must be visible as a wall, not smoothed over.
pub fn plan_pt_sweep(proc: &mut Proc) -> PtSweepPlan {
    let mut plan = PtSweepPlan::default();
    for (&(gpu, pdb), vas) in &mut proc.vases {
        // ★★ **THE DIRTY TEST IS MADE HERE TOO, AND THAT IS NOT BELT-AND-BRACES.**
        //
        // `plan_pt_decode` sets the same bit when it *drains* a tracked page, which makes the
        // trigger correct only if the decode pass ran first. That ordering holds in the
        // doorbell path and held nowhere else — a test that called this directly saw a write
        // to a tracked page produce no sweep, which is the failure mode a production caller
        // would hit the first time the two passes were reordered. Asking the undrained set the
        // same question makes the trigger a property of the STATE rather than of the call
        // order. ⊘ Idempotent: both writers set one bit, and the commit is the only clearer.
        let root = pdb.0 & !0xfff;
        if vas
            .pt_pages
            .iter()
            .any(|p| *p == root || vas.pt_meta.contains_key(p))
        {
            vas.sweep.dirty = true;
        }
        let reason = if vas.sweep.sweeps == 0 {
            SweepReason::NeverSwept
        } else if vas.sweep.truncated {
            SweepReason::Truncated
        } else if vas.sweep.dirty {
            SweepReason::Dirty
        } else {
            plan.skipped += 1;
            continue;
        };
        // Level 0 is a DECLARED fact — `plan_pt_decode` says the same thing at the same
        // statement, and for a sweep it is the *only* fact needed to start: everything deeper
        // is handed its level by the parent that pointed at it.
        plan.tasks.push(PtDecodeTask {
            gpu,
            pdb,
            page: PtPage {
                phys: pdb.0 & !0xfff,
                aperture: kayfabe_arch::Aperture::Vidmem,
                level: 0,
                vabase: 0,
            },
        });
        plan.reasons.push(reason);
    }
    plan
}

/// ★★★ **EXECUTE THE SWEEP** (no lock) — [`run_pt_decode`] with the budget charged **per
/// task** instead of across the run.
///
/// That one difference is the whole function, and it is not a tuning knob: see
/// [`PT_SWEEP_BUDGET`] for why sharing [`PT_DECODE_BUDGET`] across address spaces would divide
/// the C's per-walk bound by a guest-chosen number.
///
/// ⚠ It is therefore *not* a global bound on work. `tasks.len() * budget` is, and the task
/// count is bounded by the address-space count, which is bounded by the proc's own limits.
#[must_use]
pub fn run_pt_sweep(
    fmt: &dyn GmmuFmt,
    fb: &mut dyn kayfabe_mmu::walker::FbRead,
    tasks: &[PtDecodeTask],
    budget_per_vas: u32,
) -> Vec<PtDecodeResult> {
    tasks
        .iter()
        .map(|&task| PtDecodeResult {
            task,
            decode: decode_subtree(fmt, fb, task.page, budget_per_vas),
        })
        .collect()
}

/// How a commit admits the pages a pass decoded — the ONE axis on which the sweep differs from
/// the dirty-set drain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admit {
    /// The default and the design: a leaf binds iff its page is reachable **and** the guest was
    /// seen to write it. Residue cannot bind.
    Witnessed,
    /// ★★★★★ The relaxation: pages reached by a descent from the address space's own installed
    /// root are admitted, witnessed or not. **Only [`commit_pt_sweep`] may pass this**, and
    /// only for results produced by [`run_pt_sweep`] from a root task — see
    /// [`kayfabe_mmu::reach::ReachShadow::witness_swept`] for the accepted residual.
    Swept,
}

/// ★★★ **EXECUTE** (no lock — this is the phase that blocks): decode each task's page and
/// the subtree under it, reading through `fb`.
///
/// `budget` is shared across the whole run and consumed in task order, so one hostile
/// address space cannot be starved by another *silently*: the task that runs out gets a
/// loud [`WalkFault::BudgetExhausted`] and the ones after it get the same.
///
/// # Panics
/// Through `fb`, if the caller is holding a ranked lock — [`Worker::fb_read`] asserts R1,
/// which is the point of routing the read through a worker at all.
#[must_use]
pub fn run_pt_decode(
    fmt: &dyn GmmuFmt,
    fb: &mut dyn FbRead,
    tasks: &[PtDecodeTask],
    budget: u32,
) -> Vec<PtDecodeResult> {
    let mut left = budget;
    let mut out = Vec::with_capacity(tasks.len());
    for &task in tasks {
        let decode = decode_subtree(fmt, fb, task.page, left);
        if let Ok(d) = &decode {
            // Charge what this task actually looked at, so a sparse tree costs what it
            // contains. `visited` is pages; the entries examined is the sum of their
            // level widths, which is what `decode_subtree` itself spends.
            let spent: u32 = d
                .visited
                .iter()
                .map(|p| fmt.level_shift(p.level).map_or(1, |g| g.entries))
                .sum();
            left = left.saturating_sub(spent);
        }
        out.push(PtDecodeResult { task, decode });
    }
    out
}

/// ★★★★★ **w329 — one revoked publication, addressed.** [`kayfabe_mmu::reach::RevokedPublication`]
/// plus the `(gpu, pdb)` that identifies which address space it left, which the shell needs and
/// the pure crate does not have: the host unmap must name the host VAS the map was made in, and
/// that is a property of the `Vas`, not of the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevokedLeaf {
    /// The GPU whose `Vas` held the row.
    pub gpu: GpuId,
    /// The address space's page-directory base.
    pub pdb: Pdb,
    /// The guest VA the row was bound at.
    pub va: GpuVa,
    /// The row's tabled length.
    pub len: u64,
    /// ★ The framebuffer offset — the key `kayfabe_device::FbStore::release_join` takes. Both
    /// halves of the release are keyed off this one number, which is what makes them one unit.
    pub phys: u64,
    /// The host GPU VA to unmap, inside that `Vas`'s own host VAS.
    pub host_va: u64,
    /// The host object to free. `frees_object()` was true at the qualification, so this is a
    /// whole object and never an arena slice.
    pub memory: kayfabe_isolate::HostHandle,
}

/// ★ **COMMIT** (owner's lock, rank 1): forward-populate the decoded leaves and remember
/// what was learned.
///
/// **R5 — re-validate after re-acquiring.** The lock was released for the whole of the
/// execute phase, so the `Vas` a result names may be gone; it is re-resolved by
/// `(gpu, pdb)` and a miss is counted, never re-homed onto a survivor.
pub fn commit_pt_decode(
    fmt: &dyn GmmuFmt,
    proc: &mut Proc,
    results: &[PtDecodeResult],
) -> PtDecodeOutcome {
    commit_pt_decode_as(fmt, proc, results, Admit::Witnessed)
}

/// ★★★★★ **w329 — [`commit_pt_decode`] with the host-published unbind policy as a parameter.**
///
/// The pass is otherwise identical; see [`kayfabe_mmu::reach::apply_settlement_as`] for the four
/// conditions a row must meet to be revoked and for what is still refused.
///
/// ⚠ **The caller MUST dispose of [`PtDecodeOutcome::revoked`].** Those rows are already out of
/// the table and their host objects are reachable through nothing else; ignoring the field is not
/// "the old behaviour", it is a leak of exactly the objects the old behaviour kept a row for.
pub fn commit_pt_decode_revoking(
    fmt: &dyn GmmuFmt,
    proc: &mut Proc,
    results: &[PtDecodeResult],
    revoke: kayfabe_mmu::reach::PublishedUnbind,
) -> PtDecodeOutcome {
    commit_pt_decode_with(fmt, proc, results, Admit::Witnessed, revoke)
}

/// ★★★★★ **w329 — [`commit_pt_sweep`] with the host-published unbind policy as a parameter.**
/// Same relationship to [`commit_pt_sweep`] that [`commit_pt_decode_revoking`] has to
/// [`commit_pt_decode`], and the same obligation on the caller.
pub fn commit_pt_sweep_revoking(
    fmt: &dyn GmmuFmt,
    proc: &mut Proc,
    results: &[PtDecodeResult],
    revoke: kayfabe_mmu::reach::PublishedUnbind,
) -> PtDecodeOutcome {
    commit_pt_sweep_inner(fmt, proc, results, revoke)
}

/// ★★★★★ **COMMIT THE SWEEP** — [`commit_pt_decode_as`] with [`Admit::Swept`], plus the
/// per-address-space bookkeeping that arms the next sweep.
///
/// ⚠ `results` must come from [`run_pt_sweep`] over [`plan_pt_sweep`]'s **root** tasks.
/// Admitting a *dirty-page* task's descent as swept would mark pages the guest merely happens
/// to point at from a leaf table as root-reachable, which is not the claim
/// [`kayfabe_mmu::reach::ReachShadow::witness_swept`] is licensed to make.
pub fn commit_pt_sweep(
    fmt: &dyn GmmuFmt,
    proc: &mut Proc,
    results: &[PtDecodeResult],
) -> PtDecodeOutcome {
    commit_pt_sweep_inner(fmt, proc, results, kayfabe_mmu::reach::PublishedUnbind::Refuse)
}

fn commit_pt_sweep_inner(
    fmt: &dyn GmmuFmt,
    proc: &mut Proc,
    results: &[PtDecodeResult],
    revoke: kayfabe_mmu::reach::PublishedUnbind,
) -> PtDecodeOutcome {
    // ★ The state update runs BEFORE the admit/settle, because a truncated task contributes no
    // leaves and must still leave `truncated` set — a bookkeeping step folded into the success
    // path would arm nothing on exactly the case that needs re-arming.
    for r in results {
        let Some(vas) = proc.vases.get_mut(&(r.task.gpu, r.task.pdb)) else {
            continue;
        };
        match &r.decode {
            Ok(d) => {
                vas.sweep.sweeps += 1;
                vas.sweep.truncated = false;
                // ⊘ Cleared HERE and not at the plan. A dirty bit cleared when the sweep was
                // *scheduled* would lose every write that landed during the walk — which is
                // precisely the torn-read window this bit exists to close.
                vas.sweep.dirty = false;
                vas.sweep.last_pages = d.visited.len();
            }
            Err(_) => {
                vas.sweep.truncated = true;
            }
        }
    }
    let mut out = commit_pt_decode_with(fmt, proc, results, Admit::Swept, revoke);
    for r in results {
        match &r.decode {
            Ok(d) => {
                out.sweeps_run += 1;
                out.pages_swept += d.visited.len();
            }
            Err(_) => out.sweeps_truncated += 1,
        }
    }
    out
}

/// The body of [`commit_pt_decode`], with the admission rule as a parameter. See [`Admit`].
pub fn commit_pt_decode_as(
    fmt: &dyn GmmuFmt,
    proc: &mut Proc,
    results: &[PtDecodeResult],
    admit: Admit,
) -> PtDecodeOutcome {
    commit_pt_decode_with(
        fmt,
        proc,
        results,
        admit,
        kayfabe_mmu::reach::PublishedUnbind::Refuse,
    )
}

/// The body of [`commit_pt_decode_as`], with the host-published unbind policy as a second
/// parameter (w329). ⊘ Private: every public entry states which policy it takes, so a caller
/// cannot acquire a release obligation without naming it.
fn commit_pt_decode_with(
    fmt: &dyn GmmuFmt,
    proc: &mut Proc,
    results: &[PtDecodeResult],
    admit: Admit,
    revoke: kayfabe_mmu::reach::PublishedUnbind,
) -> PtDecodeOutcome {
    let mut out = PtDecodeOutcome::default();
    // Address spaces this pass observed something for, so `settle` runs once each rather
    // than once per decoded page — `reachability_on_transition.md` §4: a closure and a
    // desired-set recompute are O(the shadow), and a shadow that emitted a transition
    // halfway through a pass would be answering with a tree the guest had not finished
    // writing.
    let mut touched: BTreeSet<(GpuId, Pdb)> = BTreeSet::new();
    for r in results {
        let Some(vas) = proc.vases.get_mut(&(r.task.gpu, r.task.pdb)) else {
            out.vas_gone += 1;
            continue;
        };
        // ★★★ HOLE 5's standing audit, run before anything is believed. A shadow whose
        // root is not this address space's page-directory base is answering reachability
        // for a tree that is not installed; the `Vas` keying makes that unreachable, and
        // this is the check that the keying held.
        if let Err(e) = vas.reach.audit_root(r.task.pdb) {
            out.reach_faults.push(e);
            continue;
        }
        let d = match &r.decode {
            Ok(d) => d,
            Err(e) => {
                out.faults.push(*e);
                continue;
            }
        };
        out.faults.extend(d.faults.iter().copied());
        // Learn the metadata chain forward. The root is a declared fact and is not stored.
        let root = r.task.pdb.0 & !0xfff;
        for p in &d.visited {
            if p.phys == root {
                continue;
            }
            if vas.pt_meta.contains_key(&p.phys) {
                continue;
            }
            if vas.pt_meta.len() >= MAX_PT_META {
                out.meta_refused += 1;
                continue;
            }
            vas.pt_meta.insert(p.phys, *p);
            out.meta_learned += 1;
            // E8: report it upward for the PUBLISH phase. Recorded at the same statement
            // that learns it, so the two can never drift.
            out.learned_pages.push((r.task.gpu, r.task.pdb, p.phys));
        }
        // OBSERVE — content in, nothing decided yet.
        for (page, decode) in &d.decodes {
            // ★★★★★ THE RELAXATION, applied at the only statement that knows the page was
            // reached BY A DESCENT FROM THIS ADDRESS SPACE'S OWN ROOT — which is the entire
            // licence `witness_swept` has. Marking pages from a caller that did not walk from
            // the root would be a different, unlicensed claim.
            //
            // ⊘ Taken before `observe`, so a page the shadow REFUSES (capacity) is still not
            // admitted: a refused page holds no slots, so nothing of it can bind either way,
            // and the admission is then a note about a page we hold nothing for.
            if admit == Admit::Swept {
                vas.reach.witness_swept(page.phys);
            }
            if let Err(e) = vas.reach.observe(*page, decode) {
                out.reach_faults.push(e);
            }
        }
        touched.insert((r.task.gpu, r.task.pdb));
    }

    // SETTLE + APPLY, once per address space.
    for key in touched {
        let Some(vas) = proc.vases.get_mut(&key) else {
            // The `Vas` cannot vanish between the loops — no lock is released here — but
            // re-resolving costs nothing and the alternative is an `expect` that would be
            // the only unproven assumption in the pass.
            out.vas_gone += 1;
            continue;
        };
        let s = vas.reach.settle(fmt);
        // ★★ HOLE 3's second half. A page that fell out of the tree must stop being a page
        // table *to us*: `_mmuWalkPdeRelease` frees the backing store right after clearing
        // the parent, so the next thing written there is not a page table at all. Dropping
        // the level means a later write to it is DEFERRED by `plan_pt_decode` rather than
        // decoded, which is the honest answer to "we no longer know what this page is".
        for phys in &s.retired {
            vas.pt_meta.remove(phys);
        }
        let po = kayfabe_mmu::reach::apply_settlement_as(
            fmt,
            &mut vas.table,
            &mut vas.reach,
            key.1,
            &s,
            revoke,
        );
        // ★★★ The revoked rows are carried up WITH the key that identifies the address space
        // they left, because a `(host_va, memory)` pair with no `(gpu, pdb)` cannot be unmapped:
        // the unmap needs the host VAS the map was made in, and that is a property of the `Vas`.
        out.revoked.extend(po.revoked.iter().map(|r| RevokedLeaf {
            gpu: key.0,
            pdb: key.1,
            va: r.va,
            len: r.len,
            phys: r.phys,
            host_va: r.host.host_va(),
            memory: r.host.memory(),
        }));
        out.revoked_still_desired += po.revoked_still_desired;
        out.remaps_refused += po.remaps_refused;
        out.bound += po.bound;
        out.unchanged += po.unchanged;
        out.repointed += po.repointed;
        out.unbound += po.unbound;
        out.retired.extend(s.retired);
        out.protection_changes.extend(s.protection_changes);
        out.unwitnessed += s.unwitnessed;
        out.unreachable += s.unreachable;
        out.sparse += s.sparse;
        out.swept_binds += s.swept_binds;
        out.shape_collisions
            .extend(s.shape_collisions.iter().copied());
        out.duplicate_leaves += s.duplicate_leaves;
        out.dropped.extend(po.dropped);
        out.refusals.extend(po.refusals);
    }
    out
}

/// A `Vas`'s learned page-table metadata, for a caller that wants to inspect it without
/// reaching into core state. Ordered by physical address.
#[must_use]
pub fn pt_meta_of(proc: &Proc, gpu: GpuId, pdb: Pdb) -> BTreeMap<u64, PtPage> {
    proc.vases
        .get(&(gpu, pdb))
        .map(|v| v.pt_meta.clone())
        .unwrap_or_default()
}

kayfabe_util::assert_send_sync!(PtDecodeTask, PtDecodePlan, PtDecodeResult, PtDecodeOutcome);
