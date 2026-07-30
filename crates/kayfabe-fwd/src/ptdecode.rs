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
//! (`C: nvkvm_gpu_emul.c:8676-8695`). It is **not** an invalidate: `#102` stage C2
//! measured that there is no read-at-invalidate on this path, in this port or in the C
//! (§13.4), so correctness rests on *witnessing* the write. There is no second chance for
//! a write nobody saw, and nothing here is designed as if there were.

use std::collections::BTreeMap;

use kayfabe_arch::GmmuFmt;
use kayfabe_arch::ids::GpuVa;
use kayfabe_arch::ids::{GpuId, Pdb};
use kayfabe_core::gpu::Proc;
use kayfabe_isolate::{RmError, Worker};
use kayfabe_mmu::walker::{
    DropReason, FbRead, PopulateRefusal, PtPage, SubtreeDecode, WalkFault, decode_subtree, populate,
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
    /// ★★ The first **transport** failure the byte source hit, if any — set by the shell
    /// from [`IsolateFb::transport_error`] after the execute phase.
    ///
    /// Kept apart from [`PtDecodeOutcome::faults`] on purpose. A `WalkFault::Unbacked` is
    /// a statement about the *guest* (its page table names a page our aperture does not
    /// cover); this is a statement about *us* (the isolate connection broke). They look
    /// identical from inside the walker — both make a read fail — and telling them apart
    /// is the difference between debugging a guest's page tables and debugging a socket.
    pub transport: Option<RmError>,
}

impl PtDecodeOutcome {
    /// Did anything go wrong that a caller must look at?
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.refusals.is_empty()
            && self.faults.is_empty()
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
                Some(p) => plan.tasks.push(PtDecodeTask { gpu, pdb, page: p }),
                None => plan.deferred.push((gpu, pdb, page)),
            }
        }
    }
    plan
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
    let mut out = PtDecodeOutcome::default();
    for r in results {
        let Some(vas) = proc.vases.get_mut(&(r.task.gpu, r.task.pdb)) else {
            out.vas_gone += 1;
            continue;
        };
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
        }
        let po = populate(fmt, &mut vas.table, r.task.pdb, &d.leaves);
        out.bound += po.bound;
        out.unchanged += po.unchanged;
        out.repointed += po.repointed;
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
