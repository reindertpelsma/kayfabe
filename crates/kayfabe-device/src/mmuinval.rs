//! ★★★★★ **w326 — THE GUEST'S OWN TLB INVALIDATE, AS A BAR0 REGISTER WE ALREADY RECEIVE.**
//!
//! This is **tier 1** of `blocking_and_completion_model.md` §2 on the MAP side: the exact
//! GPU boundary at which the guest itself declares *"the page tables I just wrote are now
//! live"*. Until w324 this tree believed no such signal existed on the Mode-2 compute path.
//! It does, it always did, and we have been dropping it into the unclaimed-offset census
//! since M5.
//!
//! # 1. ⊘⊘⊘ THE MEASURED ZERO THAT HID IT — and why it was not wrong, only narrow
//!
//! `mode2_address_table.md` §5 (audit S3) measured, on this path:
//! `INVALIDATE_TLB` GSP-RPC fn=200 = **0**; `MEM_OP`/`MMU_TLB_INVALIDATE` pushbuffer method
//! = **0**; `DMA_FILL_PTE_MEM` = **0**. Every one of those numbers is correct. They are also
//! a complete enumeration of the **two transports somebody thought to instrument**, and RM's
//! actual transport on GA106 is **neither**:
//!
//! > `GPU_VREG_WR32(pGpu, NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE, pParams->regVal);`
//! > (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/arch/turing/kern_gmmu_tu102.c:117`)
//!
//! — a plain **BAR0 MMIO store**, with **no GSP branch anywhere in it**
//! (`kgmmuInvalidateTlb_GM107` is the only non-stub HAL variant for every chip we support;
//! its sole early bail is for paravirt guests, `kern_gmmu_gm107.c:129-135`). A GSP-offload
//! CPU-RM takes exactly the same MMIO path as a non-GSP driver.
//!
//! ⇒ ★★ **a census over transports is only as complete as its list of transports**, and a
//! zero from an incomplete list reads identical to a zero from a complete one. That is this
//! tree's `a_census_zero_needs_a_known_positive` class, and this module is the
//! known-positive: it counts a transport nobody had counted.
//!
//! ⊘ **The narrowness is preserved, not deleted.** UVM's invalidate really is a `MEM_OP`
//! pushbuffer method on UVM's own internal channel, in a different VA space; this register
//! does **not** see it. See `guest_invalidate_discipline_and_the_publish_boundary.md` §2.
//!
//! # 2. The registers, derived — never a second row that could drift
//!
//! `GPU_VREG_*` adds `DRF_BASE(NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET)` = `0x00B8_0000`
//! (`ogkm-580: kern_gpu_tu102.c:96-100`, `turing/tu102/dev_vm.h:28`), and the usermode
//! window this device **advertises** to the guest sits at `DRF_BASE(NV_VIRTUAL_FUNCTION)` =
//! `0x0003_0000` above that same base. So the two are one fact and the `PRIV` block is
//! reachable from the row we already publish:
//!
//! ```text
//!   USERMODE base (advertised) = 0x00B8_0000 + 0x0003_0000 = 0x00BB_0000
//!   PRIV base                  = USERMODE base − 0x0003_0000 = 0x00B8_0000
//!   MMU_INVALIDATE             = PRIV base + 0x30B0 = 0x00B8_30B0
//!   MMU_INVALIDATE_PDB         = PRIV base + 0x30A0 = 0x00B8_30A0
//!   MMU_INVALIDATE_UPPER_PDB   = PRIV base + 0x30A4 = 0x00B8_30A4
//! ```
//!
//! (`ogkm-580: turing/tu102/dev_vm.h:120-135`, `ampere/ga100/dev_vm.h:63-122`.)
//!
//! ⊘ **Derived and not a [`crate::ChipProfile`] row**, for exactly [`crate::doorbell`]'s
//! reason: a second independent row stating where we decode could drift from the base we
//! advertised, and the symptom would be a guest committing page tables at an offset this
//! port answers with a defaulted zero — a publication that vanished, with a healthy boot
//! around it.
//!
//! ⚠ **A correction to the briefs that led here.** Both `w326`'s brief and
//! `guest_invalidate_discipline_and_the_publish_boundary.md` say the invalidate register is
//! *"448 KiB below the doorbell we already decode"*. It is **180 KiB** below:
//! `0x00BB_0090 − 0x00B8_30B0 = 0x2_CFE0 = 184 288 bytes`. The conclusion is unaffected —
//! same BAR, same window, already trapped — but the number is wrong in both documents and
//! is corrected here rather than repeated.
//!
//! # 3. ★★★★★ THE COMPLETION IS SPECIFIED BY THE HARDWARE PROTOCOL, AND IT IS AN OBLIGATION
//!
//! RM does not fire and forget. It **spin-polls the same register** until `TRIGGER` reads
//! back false (`kgmmuCheckPendingInvalidates_TU102`, `kern_gmmu_tu102.c:69-71`):
//!
//! ```c
//!     regVal = GPU_VREG_RD32(pGpu, NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE);
//!     if (FLD_TEST_DRF(_VIRTUAL_FUNCTION_PRIV, _MMU_INVALIDATE, _TRIGGER, _FALSE, regVal))
//!         break;
//!     ... status = gpuCheckTimeout(pGpu, pTimeOut);
//! ```
//!
//! ⇒ two consequences, and they point in opposite directions:
//!
//! - ★ **The upside.** The guest is **already stopped** at this point. Publication performed
//!   here cannot race the engine, and needs no fence of ours. It is a quiescence point the
//!   protocol hands us for free — the thing every other rung of this campaign had to
//!   manufacture.
//! - ⚠ **The obligation.** If we ever answer this register with `TRIGGER` set and then fail
//!   to clear it, the guest **spins to its timeout** and then takes an Xid/reset. That is a
//!   guest hang, not a fault, and it is not fail-safe. See [`MmuInvalidateLog::arm`] for the
//!   three mechanisms that make the clear unconditional, and note that the **disarmed**
//!   configuration answers `0` — i.e. `TRIGGER` false, immediately, exactly as the unclaimed
//!   arm did before this module existed.
//!
//! **The timeout, recovered rather than guessed** (it is caller-supplied, and it is
//! `INLINE-SAFE` clause (b)'s bound for this plane): `kgmmuInvalidateTlb_GM107` arms it with
//! `gpuSetTimeout(pGpu, GPU_TIMEOUT_DEFAULT, &params.timeout, …)` (`kern_gmmu_gm107.c:64`),
//! `GPU_TIMEOUT_DEFAULT` is `0` (`gpu_timeout.h:40`) meaning *"use `pTD->defaultus`"*, and
//! `defaultus` comes from `osGetTimeoutParams` (`os.c:1961-2003`):
//!
//! | GPU mode | `defaultus` |
//! |---|---|
//! | `NV_GPU_MODE_GRAPHICS_MODE` | **4 000 000 µs (4 s)** |
//! | `NV_GPU_MODE_COMPUTE_MODE` | **30 000 000 µs (30 s)** |
//!
//! ★ And it is **re-armed dynamically**: `gpuChangeComputeModeRefCount` calls
//! `timeoutInitializeGpuDefault` when the compute refcount crosses 0↔1 (`gpu.c:303-343`), so
//! a `cup3` guest is on **4 s** until it allocates its first compute object and **30 s**
//! after. ⇒ **design against 4 s**, which is the same bound `w317` budgets against, and take
//! this tree's existing 1 % convention as the ceiling: [`INVALIDATE_HOLD_BUDGET_US`].

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

/// `DRF_BASE(NV_VIRTUAL_FUNCTION)` — how far the advertised usermode window sits above the
/// `PRIV` block both live in. See the module docs §2.
pub const USERMODE_ABOVE_PRIV: u64 = 0x0003_0000;

/// `NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE` (`ogkm-580: turing/tu102/dev_vm.h:131`).
pub const MMU_INVALIDATE_OFF: u64 = 0x30B0;

/// `NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE_PDB` (`…:120`).
pub const MMU_INVALIDATE_PDB_OFF: u64 = 0x30A0;

/// `NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE_UPPER_PDB` (`…:128`).
pub const MMU_INVALIDATE_UPPER_PDB_OFF: u64 = 0x30A4;

/// `…_PDB_ADDR_ALIGNMENT` = `0xc` — the PDB address is stored shifted right by 12
/// (`ogkm-580: turing/tu102/dev_vm.h:127`).
pub const PDB_ADDR_ALIGNMENT: u32 = 12;

/// ★★ **`INLINE-SAFE` clause (b)'s bound for this plane**, and it is derived rather than
/// chosen: 1 % of the **4 s** `gpuCheckTimeout` the guest arms before it starts polling
/// (module docs §3). Deliberately the same number as `w317`'s `RETIRED_DRAIN_BUDGET_US`,
/// because it is the same 1 %-of-4 s convention applied to a different 4 s.
///
/// ⊘ It is a **diagnostic** ceiling, not an enforcement: nothing here can interrupt a
/// publication that overruns. What it buys is that an overrun is *named* in the census
/// instead of showing up three rungs later as an unexplained guest stall.
pub const INVALIDATE_HOLD_BUDGET_US: u64 = 40_000;

/// The three BAR0 offsets, for one chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidateRegs {
    /// `MMU_INVALIDATE` — the trigger and the scope bits.
    pub trigger: u64,
    /// `MMU_INVALIDATE_PDB` — aperture + the low 28 bits of the shifted PDB address.
    pub pdb: u64,
    /// `MMU_INVALIDATE_UPPER_PDB` — the high 20 bits.
    pub upper_pdb: u64,
}

/// ★★★ **Where this chip's MMU invalidate is, DERIVED from the base the chip advertises** —
/// or `None` if the chip names no usermode register group.
///
/// ⊘ `None` is a refusal to classify, not a default offset: a chip that advertised no
/// usermode window told the driver it has no such aperture, and inventing one here would
/// decode an address the guest was never given. Same rule as [`crate::doorbell::doorbell_reg`].
#[must_use]
pub fn invalidate_regs(chip: &crate::ChipProfile) -> Option<InvalidateRegs> {
    let usermode = chip
        .chip_info
        .reg_bases
        .iter()
        .find(|r| r.index == kayfabe_abi::chipinfo::reg_base::USERMODE)
        .map(|r| u64::from(r.offset))?;
    // ⊘ Checked, not assumed: a chip whose usermode base is BELOW the PRIV delta would
    // underflow into a wildly wrong offset, and the arithmetic that produced it would look
    // as principled as the correct one.
    let priv_base = usermode.checked_sub(USERMODE_ABOVE_PRIV)?;
    Some(InvalidateRegs {
        trigger: priv_base + MMU_INVALIDATE_OFF,
        pdb: priv_base + MMU_INVALIDATE_PDB_OFF,
        upper_pdb: priv_base + MMU_INVALIDATE_UPPER_PDB_OFF,
    })
}

/// One decoded write to `MMU_INVALIDATE`.
///
/// ⊘ Every field is kept even where this port does not act on it. A scope bit we discard is
/// a scope bit we cannot later discover we needed, and this register's fields are how the
/// guest says *which* mappings it means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Invalidate {
    /// The whole 32-bit word, undecoded, so a bit this port does not model survives into
    /// the log.
    pub raw: u32,
    /// Bit 31. `false` ⇒ this write armed scope bits without committing anything.
    pub trigger: bool,
    /// Bit 0 — invalidate the whole VA range of the named PDB.
    pub all_va: bool,
    /// Bit 1 — **every** PDB, i.e. the whole GPU. When set, RM does *not* write the PDB
    /// registers at all (`kern_gmmu_tu102.c:112-115`), so [`Self::pdb`] is stale by design.
    pub all_pdb: bool,
    /// Bit 2 — the BAR VA spaces only, not a GPU VA space.
    pub hubtlb_only: bool,
    /// Bits 16:15 — `0 = ALL_TLBS`, `1 = LINK_TLBS`, `2 = NON_LINK_TLBS`.
    pub inval_scope: u32,
    /// Bits 5:3 — `REPLAY`. CPU-RM only ever writes `CANCEL_*` here, and only on the vGPU
    /// guest path, so a non-zero value on our path is itself news.
    pub replay: u32,
    /// The page-directory base this invalidate names, reassembled from the two PDB
    /// registers latched immediately before the trigger. ⊘ Meaningless when
    /// [`Self::all_pdb`] is set.
    pub pdb: u64,
    /// `MMU_INVALIDATE_PDB_APERTURE`: `0 = VID_MEM`, `1 = SYS_MEM`.
    pub pdb_aperture: u32,
}

impl Invalidate {
    /// Decode a word, given the PDB halves latched before it.
    #[must_use]
    pub fn decode(raw: u32, pdb_lo: u32, pdb_hi: u32) -> Self {
        // `_PDB_ADDR` is 31:4 and holds the LOW 28 bits of `pdbAddress >> 12`;
        // `_UPPER_PDB_ADDR` is 19:0 and holds the next 20 (`kgmmuSetPdbToInvalidate_TU102`,
        // `kern_gmmu_tu102.c:143-153`). Reassembled in that order and shifted back.
        let lo28 = u64::from(pdb_lo >> 4);
        let hi20 = u64::from(pdb_hi & 0x000F_FFFF);
        Self {
            raw,
            trigger: raw & (1 << 31) != 0,
            all_va: raw & 0b1 != 0,
            all_pdb: raw & 0b10 != 0,
            hubtlb_only: raw & 0b100 != 0,
            inval_scope: (raw >> 15) & 0b11,
            replay: (raw >> 3) & 0b111,
            pdb: ((hi20 << 28) | lo28) << PDB_ADDR_ALIGNMENT,
            pdb_aperture: (pdb_lo >> 1) & 0b1,
        }
    }
}

/// What [`MmuInvalidateLog::note_trigger`] decided the caller should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerAction {
    /// ⊘ **Disarmed** — record it and answer the guest immediately, exactly as the
    /// unclaimed arm did before this module existed. This is the byte-comparable control.
    Observed,
    /// ★ **Armed** — the caller must publish, then call [`MmuInvalidateLog::complete`].
    /// Until it does, reads of the trigger register answer with `TRIGGER` set and the guest
    /// spins in `kgmmuCheckPendingInvalidates`.
    Publish,
}

/// Everything this register has told us, as one value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MmuInvalidateSnapshot {
    /// Writes to `MMU_INVALIDATE` — including ones that did **not** set `TRIGGER`.
    pub writes: u64,
    /// Writes that set `TRIGGER`, i.e. actual commits. **This is the number the publish
    /// plane's cost is proportional to.**
    pub triggers: u64,
    /// Of [`Self::triggers`], how many carried `ALL_PDB` (whole-GPU scope).
    pub all_pdb: u64,
    /// Of [`Self::triggers`], how many carried `ALL_VA`.
    pub all_va: u64,
    /// Of [`Self::triggers`], how many carried `HUBTLB_ONLY` — a **BAR** VA space, not a
    /// GPU one. ⊘ These are not compute-path publications and a publish plane keyed on
    /// this register must not treat them as such.
    pub hubtlb_only: u64,
    /// Reads of `MMU_INVALIDATE` — RM's completion poll. Two per invalidate is the floor
    /// (one *before* to check nothing is pending, one after); many more means we held
    /// `TRIGGER` and the guest spun.
    pub polls: u64,
    /// Writes to the two PDB registers.
    pub pdb_writes: u64,
    /// Distinct PDBs named across the boot. Bounded by [`MAX_PDBS`].
    pub distinct_pdbs: usize,
    /// ★ [`Self::triggers`] as it stood when the **first doorbell** was rung — everything
    /// after this is the compute phase. Lets a boot report *"invalidates per submission"*
    /// without correlating two logs.
    pub triggers_at_first_doorbell: u64,
    /// Doorbells rung, so the ratio the trigger-vs-doorbell decision turns on is computed
    /// from two numbers taken by the same observer over the same interval.
    pub doorbells: u64,
    /// The longest a single armed publication held `TRIGGER` set, in microseconds.
    pub worst_hold_us: u64,
    /// Publications whose hold exceeded [`INVALIDATE_HOLD_BUDGET_US`].
    pub over_budget: u64,
    /// ⚠ Non-zero means a trigger arrived while one was still pending. RM serialises these
    /// under its own lock, so a non-zero here is either a second RM client or our own
    /// completion having gone missing.
    pub reentrant: u64,
    /// Whether a publication is outstanding right now.
    pub pending: bool,
}

/// How many distinct PDBs are remembered. Bounded for the same reason the unclaimed census
/// is: an unbounded set on a guest-driven path is a guest-driven allocation.
pub const MAX_PDBS: usize = 64;

#[derive(Debug, Default)]
struct Inner {
    pdb_lo: u32,
    pdb_hi: u32,
    pdbs: BTreeSet<u64>,
    /// When the currently-pending publication armed, as a monotonic microsecond stamp
    /// supplied by the caller (this crate models no clock of its own).
    pending_since_us: Option<u64>,
    snap: MmuInvalidateSnapshot,
}

/// ★★★ **The log.** Lock-free on the read path that matters, because the guest polls the
/// trigger register in a tight loop and every poll is an MMIO trap.
///
/// ⊘ Held on [`crate::RegPlane`] beside `unserviced`/`bar_pdes`/`gvas_pub` for their two
/// reasons — reading it must not take the FSM's lock behind a doorbell, and it survives
/// `set_policy`.
#[derive(Debug)]
pub struct MmuInvalidateLog {
    /// ★★★★★ **The pending flag lives OUTSIDE the mutex, and that is the whole completion
    /// guarantee's first leg.** The guest's poll reads exactly this atomic; it never takes
    /// a lock, so no lock this process could hold — and no thread that could die holding
    /// one — can leave the guest spinning. See [`Self::arm`].
    pending: AtomicBool,
    /// Whether the armed behaviour is on at all. ⊘ Separate from `pending`: "disarmed" and
    /// "armed with nothing outstanding" answer the guest identically and are different facts.
    armed: AtomicBool,
    inner: Mutex<Inner>,
    /// Counters the poll path bumps, outside the mutex for the same reason `pending` is.
    polls: AtomicU64,
}

impl Default for MmuInvalidateLog {
    fn default() -> Self {
        Self::new()
    }
}

impl MmuInvalidateLog {
    /// A disarmed log — it records and answers `0`, which is byte-identical to the
    /// unclaimed arm this replaces.
    #[must_use]
    pub fn new() -> Self {
        Self {
            pending: AtomicBool::new(false),
            armed: AtomicBool::new(false),
            inner: Mutex::new(Inner::default()),
            polls: AtomicU64::new(0),
        }
    }

    /// ★★★ **Arm the completion behaviour.**
    ///
    /// Once armed, a `TRIGGER` write leaves the register reading `TRIGGER = TRUE` until
    /// [`Self::complete`] runs. **Three mechanisms make that clear unconditional**, and no
    /// two of them are sufficient:
    ///
    /// 1. **The worker completes in a `Drop` guard, not on the success path.** A publication
    ///    that panics, returns early or is refused still clears — see the shell's
    ///    `PublishGuard`. An `if ok { clear() }` would make a guest hang the punishment for
    ///    any of our own errors.
    /// 2. **The flag is an atomic outside every lock**, so a poisoned mutex or a thread that
    ///    died holding one cannot hold it set. (`pending` above.)
    /// 3. **The doorbell path is not the only clearer.** Arming is refused unless a worker
    ///    exists to drain the queue; if the queue refuses an offer, the caller publishes
    ///    inline and clears in the same trap, which is the pre-w326 behaviour and is never
    ///    worse than it.
    ///
    /// ⚠ **This is the first guest-visible MMIO read whose value depends on completed
    /// work.** `publication_off_the_bql.md` §5.2 measured that no such read existed and
    /// concluded deferral was invisible to the guest. That finding is now **scoped, not
    /// refuted**: it remains true of all seven pre-existing arms, and this eighth one is
    /// deliberate. It is also the *only* one, and it is a read the guest is **already
    /// blocking on**, which is why it is safe to make it mean something.
    pub fn arm(&self) {
        self.armed.store(true, Ordering::Release);
    }

    /// Is the armed behaviour on?
    #[must_use]
    pub fn is_armed(&self) -> bool {
        self.armed.load(Ordering::Acquire)
    }

    /// Latch a write to one of the two PDB registers. Returns `true` if it was one.
    pub fn note_pdb_write(&self, regs: InvalidateRegs, off: u64, val: u32) -> bool {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if off == regs.pdb {
            g.pdb_lo = val;
        } else if off == regs.upper_pdb {
            g.pdb_hi = val;
        } else {
            return false;
        }
        g.snap.pdb_writes += 1;
        true
    }

    /// ★★★★★ **The commit point.** Decode a write to `MMU_INVALIDATE` and say what the
    /// caller must do.
    ///
    /// `now_us` is a monotonic stamp from the caller's clock — this crate models none.
    pub fn note_trigger(&self, raw: u32, now_us: u64) -> (Invalidate, TriggerAction) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let inv = Invalidate::decode(raw, g.pdb_lo, g.pdb_hi);
        g.snap.writes += 1;
        if !inv.trigger {
            // ⊘ A write that armed scope bits without committing. Recorded and NOT acted
            // on: publishing here would publish before the guest said the tables were
            // ready, which is the one ordering this whole plane exists to respect.
            return (inv, TriggerAction::Observed);
        }
        g.snap.triggers += 1;
        g.snap.all_pdb += u64::from(inv.all_pdb);
        g.snap.all_va += u64::from(inv.all_va);
        g.snap.hubtlb_only += u64::from(inv.hubtlb_only);
        if !inv.all_pdb && g.pdbs.len() < MAX_PDBS {
            g.pdbs.insert(inv.pdb);
            g.snap.distinct_pdbs = g.pdbs.len();
        }
        if !self.is_armed() {
            return (inv, TriggerAction::Observed);
        }
        if self.pending.swap(true, Ordering::AcqRel) {
            // A trigger arrived with one already outstanding. RM serialises these under its
            // own lock, so this is news either way; the newer one subsumes the older (both
            // are "publish what is dirty"), and the single pending flag already says so.
            g.snap.reentrant += 1;
        }
        g.pending_since_us = Some(now_us);
        (inv, TriggerAction::Publish)
    }

    /// ★★★★★ **The completion.** Clears `TRIGGER` so the guest's poll returns.
    ///
    /// Idempotent on purpose: the `Drop` guard and an explicit success path may both call
    /// it, and a completion that could be double-sent must be one that costs nothing the
    /// second time.
    pub fn complete(&self, now_us: u64) {
        if !self.pending.swap(false, Ordering::AcqRel) {
            return;
        }
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(since) = g.pending_since_us.take() {
            let held = now_us.saturating_sub(since);
            g.snap.worst_hold_us = g.snap.worst_hold_us.max(held);
            if held > INVALIDATE_HOLD_BUDGET_US {
                g.snap.over_budget += 1;
            }
        }
    }

    /// ★★★ **What the guest reads.** `TRIGGER` set iff a publication is outstanding.
    ///
    /// ⊘ Lock-free by construction — see [`Self::pending`]'s note. The guest polls this in
    /// a spin loop; a lock here would put every vCPU behind whatever the worker is doing,
    /// which is the exact cost this rung exists to remove.
    #[must_use]
    pub fn read_trigger(&self) -> u64 {
        self.polls.fetch_add(1, Ordering::Relaxed);
        if self.pending.load(Ordering::Acquire) {
            1 << 31
        } else {
            0
        }
    }

    /// Is a publication outstanding?
    #[must_use]
    pub fn pending(&self) -> bool {
        self.pending.load(Ordering::Acquire)
    }

    /// Note that a doorbell was rung, so the invalidate-per-doorbell ratio comes from one
    /// observer over one interval.
    pub fn note_doorbell(&self) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if g.snap.doorbells == 0 {
            g.snap.triggers_at_first_doorbell = g.snap.triggers;
        }
        g.snap.doorbells += 1;
    }

    /// Everything, as one value.
    #[must_use]
    pub fn snapshot(&self) -> MmuInvalidateSnapshot {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        MmuInvalidateSnapshot {
            polls: self.polls.load(Ordering::Relaxed),
            pending: self.pending.load(Ordering::Acquire),
            ..g.snap.clone()
        }
    }

    /// One line for a boot log.
    ///
    /// ⊘ Printed even when every number is zero. *"The register never fired"* and *"nobody
    /// looked"* are different facts and only the line's presence distinguishes them — this
    /// tree's `⊘ABSENT-UNMEASURED, not 0` rule, applied at the source rather than in the
    /// grader.
    #[must_use]
    pub fn census(&self) -> String {
        let s = self.snapshot();
        // ⊘ The two ratios the trigger-vs-doorbell decision turns on, computed here so a
        // grader cannot divide the wrong pair. Guarded: a zero denominator prints `n/a` and
        // never `0`, because "no doorbells were rung" is not "the ratio is zero".
        let per_db = if s.doorbells == 0 {
            "n/a".to_string()
        } else {
            format!("{:.4}", s.triggers as f64 / s.doorbells as f64)
        };
        let all_pdb_frac = if s.triggers == 0 {
            "n/a".to_string()
        } else {
            format!("{:.4}", s.all_pdb as f64 / s.triggers as f64)
        };
        format!(
            "MMUINVAL armed={} writes={} triggers={} all_pdb={} all_pdb_frac={} all_va={} \
             hubtlb_only={} gpu_vas={} polls={} pdb_writes={} distinct_pdbs={} \
             doorbells={} triggers_per_doorbell={} triggers_at_first_doorbell={} \
             worst_hold_us={} over_budget={} reentrant={} pending={}{}",
            self.is_armed(),
            s.writes,
            s.triggers,
            s.all_pdb,
            all_pdb_frac,
            s.all_va,
            s.hubtlb_only,
            s.triggers.saturating_sub(s.hubtlb_only),
            s.polls,
            s.pdb_writes,
            s.distinct_pdbs,
            s.doorbells,
            per_db,
            s.triggers_at_first_doorbell,
            s.worst_hold_us,
            s.over_budget,
            s.reentrant,
            s.pending,
            if s.pending {
                " ⚠⚠ TRIGGER STILL SET AT TEARDOWN — the guest is spinning on a completion \
                 that never came. This is a HANG, not a fault."
            } else {
                ""
            },
        )
    }
}

kayfabe_util::assert_send_sync!(MmuInvalidateLog, Invalidate, MmuInvalidateSnapshot);

#[cfg(test)]
mod tests {
    use super::*;

    /// ★★★ The offsets, against the vendor's own headers. A derivation nobody checks is a
    /// guess with arithmetic in front of it.
    #[test]
    fn the_offsets_are_the_ones_the_vendor_headers_name() {
        let regs = invalidate_regs(&crate::ga10x::GA106).expect("GA106 advertises USERMODE");
        assert_eq!(regs.trigger, 0x00B8_30B0, "NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE");
        assert_eq!(regs.pdb, 0x00B8_30A0, "…_MMU_INVALIDATE_PDB");
        assert_eq!(regs.upper_pdb, 0x00B8_30A4, "…_MMU_INVALIDATE_UPPER_PDB");
    }

    /// ⚠ The brief and two design docs say "448 KiB below the doorbell". It is 180 KiB, and
    /// this test is where that gets corrected instead of repeated.
    #[test]
    fn the_distance_to_the_doorbell_is_180_kib_not_448() {
        let regs = invalidate_regs(&crate::ga10x::GA106).expect("GA106 advertises USERMODE");
        let db = crate::doorbell::doorbell_reg(&crate::ga10x::GA106).expect("a doorbell");
        assert_eq!(db, 0x00BB_0090);
        assert_eq!(db - regs.trigger, 0x2_CFE0);
        assert_eq!((db - regs.trigger) / 1024, 179, "180 KiB, not 448");
    }

    /// ★★★★★ The exact word the C's `cap3_matmul_forwarding` recorded at the commit point,
    /// decoded. `gmmu_publication_discipline.md` quotes the six-step sequence verbatim;
    /// this pins our decoder against it, so a future field-shuffle fails here rather than
    /// on a bench.
    #[test]
    fn the_word_the_c_captured_decodes_the_way_the_c_read_it() {
        // MmioWr bar0 0xb830a0 = 0x02efba50 ; 0xb830a4 = 0x0 ; 0xb830b0 = 0x80010001
        let inv = Invalidate::decode(0x8001_0001, 0x02ef_ba50, 0x0);
        assert!(inv.trigger, "bit 31");
        assert!(inv.all_va, "ALL_VA — the C recorded ALL_VA=1 on every one of 308");
        assert!(!inv.all_pdb, "ALL_PDB=0 on every one of 308 — it always names ONE pdb");
        assert!(!inv.hubtlb_only, "a GPU VA space, not a BAR one");
        assert_eq!(inv.inval_scope, 2, "NON_LINK_TLBS");
        assert_eq!(inv.replay, 0, "CPU-RM never writes REPLAY_START");
        assert_eq!(inv.pdb_aperture, 0, "VID_MEM");
        // `_PDB_ADDR` is 31:4, so the guest's word `0x02efba50` carries `0x02efba5` — one
        // nibble SHORTER than the word looks. ⚠ Written wrong the first time (`0x2efba50`),
        // and the assertion caught it: an off-by-one-nibble PDB is a plausible-looking
        // address that names a different VA space, which is the failure mode this whole
        // plane would be worst at diagnosing later.
        assert_eq!(inv.pdb, 0x02efba5_u64 << 12);
        assert_eq!(inv.pdb, 0x2_EFBA_5000);
    }

    /// ⊘ A write that does not set `TRIGGER` is not a commit and must not publish.
    #[test]
    fn a_write_without_the_trigger_bit_publishes_nothing() {
        let log = MmuInvalidateLog::new();
        log.arm();
        let (inv, act) = log.note_trigger(0x0001_0001, 0);
        assert!(!inv.trigger);
        assert_eq!(act, TriggerAction::Observed);
        assert!(!log.pending(), "nothing may be outstanding");
        assert_eq!(log.read_trigger(), 0);
    }

    /// ★★★★★ **The completion contract, both directions.**
    #[test]
    fn the_guest_reads_trigger_set_until_the_publication_completes() {
        let log = MmuInvalidateLog::new();
        log.arm();
        assert_eq!(log.read_trigger(), 0, "nothing outstanding");
        let (_, act) = log.note_trigger(0x8001_0001, 1_000);
        assert_eq!(act, TriggerAction::Publish);
        assert_eq!(log.read_trigger(), 1 << 31, "★ the guest MUST spin here");
        assert_eq!(log.read_trigger(), 1 << 31, "…and keep spinning");
        log.complete(3_000);
        assert_eq!(log.read_trigger(), 0, "★★★ and now it proceeds");
        let s = log.snapshot();
        assert_eq!(s.worst_hold_us, 2_000);
        assert_eq!(s.over_budget, 0);
        assert_eq!(s.triggers, 1);
    }

    /// ★★★ **Idempotent completion** — the `Drop` guard and a success path may both fire.
    #[test]
    fn completing_twice_costs_nothing_and_does_not_corrupt_the_hold() {
        let log = MmuInvalidateLog::new();
        log.arm();
        log.note_trigger(0x8000_0001, 0);
        log.complete(5_000);
        log.complete(9_999_999);
        assert_eq!(log.snapshot().worst_hold_us, 5_000, "the second call is a no-op");
        assert!(!log.pending());
    }

    /// ⊘⊘ **THE DISARMED ARM IS THE CONTROL, and it must be byte-identical to the
    /// unclaimed arm it replaces**: `TRIGGER` never reads set, so the guest never spins.
    #[test]
    fn disarmed_answers_zero_forever_and_can_never_hang_the_guest() {
        let log = MmuInvalidateLog::new();
        for _ in 0..8 {
            let (_, act) = log.note_trigger(0x8001_0001, 0);
            assert_eq!(act, TriggerAction::Observed);
            assert_eq!(log.read_trigger(), 0, "⊘ a disarmed plane can never hang a guest");
        }
        let s = log.snapshot();
        assert_eq!(s.triggers, 8, "…and it still MEASURES");
        assert_eq!(s.polls, 8);
    }

    /// The budget is a diagnostic and it must actually fire.
    #[test]
    fn a_hold_over_the_budget_is_named() {
        let log = MmuInvalidateLog::new();
        log.arm();
        log.note_trigger(0x8000_0001, 0);
        log.complete(INVALIDATE_HOLD_BUDGET_US + 1);
        assert_eq!(log.snapshot().over_budget, 1);
    }

    /// ★ The ratio the whole trigger-vs-doorbell decision turns on, from one observer.
    #[test]
    fn the_ratio_is_computed_from_two_numbers_this_log_took_itself() {
        let log = MmuInvalidateLog::new();
        log.note_trigger(0x8000_0001, 0);
        log.note_trigger(0x8000_0001, 0);
        log.note_doorbell();
        log.note_trigger(0x8000_0001, 0);
        log.note_doorbell();
        let s = log.snapshot();
        assert_eq!(s.triggers, 3);
        assert_eq!(s.doorbells, 2);
        assert_eq!(s.triggers_at_first_doorbell, 2, "two arrived before any submission");
        assert!(log.census().contains("triggers_per_doorbell=1.5000"));
    }

    /// ⊘ A zero denominator prints `n/a`, never `0` — "no doorbells" is not "ratio zero".
    #[test]
    fn an_absent_denominator_is_named_and_never_defaulted_to_zero() {
        let log = MmuInvalidateLog::new();
        let c = log.census();
        assert!(c.contains("triggers_per_doorbell=n/a"), "{c}");
        assert!(c.contains("all_pdb_frac=n/a"), "{c}");
    }

    /// ★★★ `ALL_PDB` means RM did **not** write the PDB registers, so the latched value is
    /// stale by design and must not be reported as this invalidate's target.
    #[test]
    fn an_all_pdb_invalidate_names_no_single_pdb() {
        let log = MmuInvalidateLog::new();
        log.note_pdb_write(
            InvalidateRegs { trigger: 0xB830B0, pdb: 0xB830A0, upper_pdb: 0xB830A4 },
            0xB830A0,
            0x02ef_ba50,
        );
        let (inv, _) = log.note_trigger(0x8000_0003, 0);
        assert!(inv.all_pdb);
        assert_eq!(log.snapshot().distinct_pdbs, 0, "⊘ a stale latch is not a target");
    }
}
