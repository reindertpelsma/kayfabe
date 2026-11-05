//! ★★★★★ **BLOCKAGE POINTS — where the guest is already stopped, and therefore where a
//! mapping may be published for free.** The owner's ruling of 2026-09-07, as types and as a
//! census (`docs/design/REQUIREMENTS_TARGET.md` R1; falsifier **C1**).
//!
//! > *"anything thats async blockage gives coverage. so kernel emulated channels, rpc, tlb
//! > invalidate. … what we can't block is passthrough channels and what to avoid is running
//! > any blocking code, or even scheduling it at: bar1/2 traps to gpu phys, dram traps. plus
//! > resume on fault. if we can avoid that its solved."*
//!
//! # 0. ⊘⊘⊘ WHAT THIS REPLACES — the frame, not the mechanism
//!
//! The whole `w387_the_silent_map.md` campaign asked *"how do we OBSERVE every page-table
//! write"* and proved, three times in one day, that the answer is version-dependent and
//! incomplete (invalidate → RPC → aperture). **Coverage is not visibility.** It is the
//! existence of a point at which the guest is **already halted** before the mapping can be
//! consumed. At such a point the work is free and the correctness argument is *by
//! construction* rather than by observation.
//!
//! ⇒ This module does not watch anything. It records **which halt a mapping was published
//! under**, so that *"a mapping was used that no halt covered"* becomes a **countable
//! number** instead of an argument.
//!
//! # 1. The three points, and why each one halts the guest
//!
//! | point | why the guest cannot proceed |
//! |---|---|
//! | [`BlockagePoint::EmulatedDoorbell`] | a guest-kernel/UVM channel's doorbell is `TrapContract::ScheduleAndReturn`; the store is an MMIO vmexit and the vCPU is inside our handler |
//! | [`BlockagePoint::GspRpc`] | `_issueRpcAndWait` (`ogkm-580: rpc.c:1821`) — the guest spins on our reply |
//! | [`BlockagePoint::TlbInvalidate`] | `kgmmuCheckPendingInvalidates_TU102` (`kern_gmmu_tu102.c:59-84`) spin-polls `TRIGGER` at BAR0 `0x00B8_30B0` until it reads false |
//!
//! ⊘ **The fourth plane is deliberately absent and that absence is the model.** A user-proc
//! compute/CE channel is `GuestChannelKind::Passthrough`: its doorbell is
//! `TrapContract::RingAndReturn`, it carries **work only**, and it has **no blockage point
//! by design**. The one case the three points structurally cannot cover — a mapping changed
//! under an already-running passthrough channel — is R1.3's named residual and is measured
//! by `rmladder --late-map-race`, not by this census.
//!
//! # 2. ⊘ THE ANTI-REQUIREMENT IS INHERITED HERE, NOT RESTATED AS ADVICE
//!
//! R1.2 forbids blocking work, **and the scheduling of work**, on BAR1 traps, BAR2 traps to
//! GPU phys, and DRAM traps. Nothing in this module can be armed on such a path, because
//! there is no such arm: [`BlockagePoint::ALL`] is three values, the enum is closed, and a
//! fourth point would be a source edit a reviewer sees. ⚠ That is a *shape* argument, not an
//! enforcement — C4 wants a static gate and this is not it.
//!
//! # 3. ★★★ HOW A ZERO IS KEPT HONEST — the rule this whole tree is scarred by
//!
//! `a_census_zero_needs_a_known_positive`: an absence is not evidence without a
//! known-positive. Three separate mechanisms, because the failure modes are different:
//!
//! - **Arming is measured, never assumed.** [`global_entries`] counts guard *entries* per
//!   point. `uncovered=0` beside `entries=[0 0 0]` is a **never-armed** zero and the
//!   rendered line says so in those words; `uncovered=0` beside a non-zero entry count is a
//!   measured zero. The two are never printed the same way.
//! - **The denominator is always printed.** A ratio with no denominator is not a
//!   measurement (`every_row_verified_over_zero_rows`: a graded verdict once passed over
//!   **zero rows**).
//! - **The counts are exact and only the LIST is capped** — the rule
//!   [`kayfabe_util::coverage`] was built to enforce, applied again here: `render` truncates
//!   the printed VA list and never a count.
//!
//! # 4. Where the counting happens, and why it is not at an enumeration of call sites
//!
//! `arm_the_gate_at_the_sink_not_at_an_enumeration` — measured in this tree, twice. The
//! attribution is taken inside [`crate::AddressTable::bind`], which is the table's **only**
//! entrance, and consumed inside [`crate::AddressTable::resolve`], which is the only point
//! query. A census hung off the four call sites that currently bind would be complete on the
//! day it was written and silently partial the day a fifth arrived.
//!
//! # 5. ⚠ WHAT THIS CANNOT SEE — read before quoting a green line
//!
//! - **It sees OUR table, not the GPU's walk.** A `use` here is *our* resolve of a guest VA
//!   on the guest's behalf. The engine walks the **host** page tables, and this census says
//!   nothing about them (`the_table_is_right_and_the_host_vas_is_empty`). It answers *"did
//!   anything we published arrive without a halt behind it"*, which is exactly C1, and it
//!   answers nothing about whether the host side then materialised.
//! - **It sees guards that were INSTALLED.** A publication path that never enters a
//!   [`BlockageGuard`] is attributed `None` — which is the *safe* direction (it inflates
//!   `uncovered`, it cannot hide one) but it means a red here may indict a missing guard
//!   rather than a missing halt. The entry counts are what separate those two readings.
//! - **It is not an ordering proof.** *"Published before used"* is structural — a resolve
//!   can only hit a row a `bind` already inserted — not something this counts.

use core::sync::atomic::{AtomicU64, Ordering};

/// ★★★★★ **A point at which the guest is already stopped**, and therefore a point at which a
/// mapping may be published with no cost to the guest and no risk of being late.
///
/// ⊘ **Closed by construction.** A fourth arm is the whole of R1.2's anti-requirement
/// arriving: it would have to name a path — a BAR1 trap, a BAR2 trap, a DRAM write — on
/// which the guest is stopped *because it touched memory* rather than because it *asked us
/// something*. Adding one is a source edit, which is the point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BlockagePoint {
    /// **A guest-KERNEL channel's doorbell.** `GuestChannelKind::Emulated` ⇒
    /// `TrapContract::ScheduleAndReturn`, so the handler is ours and the vCPU is inside an
    /// MMIO vmexit for the whole of it.
    ///
    /// ★ This is the point that carries **UVM's page-table writes and its invalidates**:
    /// UVM writes PDEs/PTEs with CE pushbuffer methods (`uvm_mmu.c` `pde_fill_gpu`,
    /// `uvm_pte_batch.c` → `ce_hal->memcopy`) and invalidates with pushbuffer `MEM_OP`
    /// (`uvm_ampere_host.c:255-265`) — never BAR2, never `0xB830B0`. And a UVM channel is a
    /// guest-kernel channel, so `kayfabe_core::project::ProcBoundary::channel_kind` (anchor
    /// == `SYSTEM_ANCHOR`) classifies it `Emulated`.
    ///
    /// ⚠ **That classification is asserted on a live boot by C3, not assumed here.** *"By
    /// rule"* and *"in this boot"* have diverged in this tree before.
    EmulatedDoorbell,
    /// **A GSP RPC we are answering.** The guest is inside `_issueRpcAndWait` and cannot
    /// proceed until we post a reply, so everything the RPC declares — RM-side maps,
    /// `GPU_PROMOTE_CTX` — can be published before the reply is posted.
    ///
    /// ⚠ **The hang has a budget**: `osGetTimeoutParams` gives **4 s** (GRAPHICS) / **30 s**
    /// (COMPUTE). Overrun is an Xid and a reset, not a slow path. A publication that takes
    /// longer than that has not bought coverage; it has killed the guest.
    ///
    /// ⊘ **It is not universal, and w387 §2 is why**: `bSplitVasManagementServerClientRm`
    /// defaults to true for every GSP client, so most guest maps are *local* and never
    /// become an RPC at all. This point covers what crosses, and the model does not claim
    /// everything crosses.
    GspRpc,
    /// **The guest's own TLB invalidate**, BAR0 `0x00B8_30B0`. `kgmmuCheckPendingInvalidates`
    /// spin-polls `TRIGGER` until it reads false, so answering late is not a latency cost —
    /// it is the guest's whole VM stopped, which is precisely what makes it free to publish
    /// under.
    ///
    /// ⊘ **It is conditional and w387 §1.1 is why**: `NVOS46_FLAGS_DEFER_TLB_INVALIDATION`
    /// (bit 31) is client-settable and skips it. This point covers what fires.
    TlbInvalidate,
}

impl BlockagePoint {
    /// Every point, so a census quantifies over the enum rather than over a hand-written
    /// list that shrinks in one place with nothing going red.
    pub const ALL: [BlockagePoint; 3] = [
        BlockagePoint::EmulatedDoorbell,
        BlockagePoint::GspRpc,
        BlockagePoint::TlbInvalidate,
    ];

    /// Dense index into a per-point array. ⊘ `as usize` on the discriminant is deliberately
    /// not used: a reorder of the variants would silently re-key every stored count.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            BlockagePoint::EmulatedDoorbell => 0,
            BlockagePoint::GspRpc => 1,
            BlockagePoint::TlbInvalidate => 2,
        }
    }

    /// The name a diagnostic prints. Stable — a grader greps these.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            BlockagePoint::EmulatedDoorbell => "emulated-doorbell",
            BlockagePoint::GspRpc => "gsp-rpc",
            BlockagePoint::TlbInvalidate => "tlb-invalidate",
        }
    }
}

impl core::fmt::Display for BlockagePoint {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

// ★ One global entry counter per point. This is the **arming state** and it is deliberately
// process-wide: a boot line has to answer *"was this point ever entered on ANY vCPU"*, and a
// thread-local cannot. See [`thread_entries`] for the half a test needs.
static GLOBAL_ENTRIES: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];

std::thread_local! {
    /// The innermost point this thread is currently inside, `None` when it is inside none.
    ///
    /// ⊘ **A single slot, saved and restored by the guard**, not a stack. Nesting is legal
    /// (a doorbell trap can drive an RPC service pass), and the innermost point is the right
    /// answer: it is the halt closest to the publication, and it is the one whose budget the
    /// publication is spending.
    static CURRENT: core::cell::Cell<Option<BlockagePoint>> = const { core::cell::Cell::new(None) };
    /// Per-thread entry counts. ⚠ Exists **only** so a unit test can assert on its own
    /// arming without racing every other test in the binary; the boot line reads
    /// [`global_entries`]. Two counters, two questions, both stated.
    static THREAD_ENTRIES: core::cell::Cell<[u64; 3]> = const { core::cell::Cell::new([0; 3]) };
}

/// ★★★★★ **Which blockage point this thread is inside, if any.**
///
/// [`crate::AddressTable::bind`] reads it and nothing else in this crate does. `None` means
/// *"this publication is happening on no halt we declared"* — the number C1 requires to be
/// zero.
#[must_use]
pub fn current() -> Option<BlockagePoint> {
    CURRENT.with(core::cell::Cell::get)
}

/// How many times each point has been entered, process-wide, indexed by
/// [`BlockagePoint::index`]. **This is the arming evidence** — see the module docs §3.
#[must_use]
pub fn global_entries() -> [u64; 3] {
    [
        GLOBAL_ENTRIES[0].load(Ordering::Relaxed),
        GLOBAL_ENTRIES[1].load(Ordering::Relaxed),
        GLOBAL_ENTRIES[2].load(Ordering::Relaxed),
    ]
}

/// How many times each point has been entered **on this thread**. ⊘ For tests; a boot line
/// wants [`global_entries`].
#[must_use]
pub fn thread_entries() -> [u64; 3] {
    THREAD_ENTRIES.with(core::cell::Cell::get)
}

/// ★★★★★ **Declare that the guest is halted at `point` for the lifetime of this value.**
///
/// RAII, `!Send`/`!Sync` by holding a thread-local slot: the declaration is about *this
/// thread's* guest, and a token that could travel would launder one vCPU's halt into
/// another's. (The same composition [`kayfabe_util::trapwitness::OffTrap`] uses, for the
/// same reason.)
///
/// ⊘ **Entering one is not a permission to do unbounded work.** It is a statement that the
/// guest cannot observe the interval, which is true of all three points and is bounded by
/// the RPC's 4 s/30 s timeout on one of them. `l1_concurrency.md` R1 still applies inside.
#[derive(Debug)]
pub struct BlockageGuard {
    /// What `CURRENT` held before this guard, restored on drop. `Option<Option<..>>` is
    /// avoided: the outer level is always known because the guard always sets one.
    previous: Option<BlockagePoint>,
    /// ⊘ Makes the guard `!Send` + `!Sync` **by construction**, with no `impl` and no
    /// relaxation of the workspace's `unsafe_code` lint — a raw-pointer marker is neither, so
    /// the guarantee costs nothing and the `*_unsafe.rs` surface is untouched. A guard that
    /// crossed to another thread would restore that thread's slot to this thread's previous
    /// value, which is a halt on one vCPU laundering a publication on another.
    _not_send: core::marker::PhantomData<*mut ()>,
}

impl BlockageGuard {
    /// Enter `point`. Counts the entry in both censuses and shadows any outer point until
    /// this value is dropped.
    #[must_use]
    pub fn enter(point: BlockagePoint) -> Self {
        let i = point.index();
        GLOBAL_ENTRIES[i].fetch_add(1, Ordering::Relaxed);
        THREAD_ENTRIES.with(|c| {
            let mut e = c.get();
            e[i] = e[i].saturating_add(1);
            c.set(e);
        });
        let previous = CURRENT.with(|c| c.replace(Some(point)));
        BlockageGuard {
            previous,
            _not_send: core::marker::PhantomData,
        }
    }
}

impl Drop for BlockageGuard {
    fn drop(&mut self) {
        CURRENT.with(|c| c.set(self.previous));
    }
}

/// ★★★ **The provenance of ONE row of the address table** — under which halt it became
/// usable, and when.
///
/// ⊘ Stored **inside** the table's own value rather than beside it. A parallel map keyed by
/// the same VA would be a second source of truth about one row, which this tree has a name
/// for (`a_second_source_of_truth_beside_a_complete_value`) and has paid for; keeping it in
/// the value makes *"the two disagree"* unrepresentable rather than merely tested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Publication {
    /// The halt this row was published under, or `None` for *"under no declared halt"* —
    /// the population C1 requires to be empty.
    pub point: Option<BlockagePoint>,
    /// The table generation at which the row was inserted. ⊘ Not an ordering *proof* —
    /// ordering is structural, a resolve cannot hit a row that was not already inserted —
    /// but it is what lets a dump of `uncovered` rows be read in the order they arrived.
    pub at_generation: u64,
}

impl Publication {
    /// Whether this row arrived under a declared halt.
    #[must_use]
    pub const fn covered(self) -> bool {
        self.point.is_some()
    }

    /// The name a diagnostic prints for the point, or `"none"`.
    #[must_use]
    pub const fn point_name(self) -> &'static str {
        match self.point {
            Some(p) => p.name(),
            None => "none",
        }
    }
}

/// ★★★★★ **ONE ADDRESS TABLE'S BLOCKAGE CENSUS** — the counts C1 is graded on.
///
/// # ⊘ Atomics in a crate whose contract is *"plain owned data, no interior mutability"*
///
/// Deliberate, and it is the narrowest exception that works. [`crate::AddressTable::resolve`]
/// is `&self` by decision #17 (concurrent reads are safe and are relied on), so the *use*
/// side cannot be counted without interior mutability, and counting it at the callers
/// instead is exactly the enumeration §4 refuses. `AtomicU64` keeps the table `Send + Sync`
/// — which the crate compile-time-asserts — and no ordering stronger than `Relaxed` is
/// claimed: these are counters, nothing branches on them.
///
/// ⊘ **Per-table and not a global.** A global would make every unit test in the binary race
/// every other one, and a census a test cannot isolate is a census whose red nobody trusts.
/// The whole-boot number is a **sum over tables**, computed by the caller that already walks
/// every VAS.
#[derive(Debug, Default)]
pub struct BlockageCensus {
    /// Successful binds attributed to each point, by [`BlockagePoint::index`].
    binds: [AtomicU64; 3],
    /// Successful binds that happened under **no** declared halt.
    binds_uncovered: AtomicU64,
    /// Successful resolves that hit a row published under each point.
    uses: [AtomicU64; 3],
    /// ★★★★★ **THE C1 NUMBER.** Successful resolves that hit a row published under no
    /// declared halt — *"a mapping used with no prior blockage point"*. Must be zero.
    uses_uncovered: AtomicU64,
    /// Resolves that faulted. ⊘ Counted so `uses == 0` can be told apart from *"nothing ever
    /// asked this table anything"*: a table with misses was consulted.
    use_misses: AtomicU64,
}

impl BlockageCensus {
    /// Record one successful bind under `point`.
    pub(crate) fn note_bind(&self, point: Option<BlockagePoint>) {
        match point {
            Some(p) => &self.binds[p.index()],
            None => &self.binds_uncovered,
        }
        .fetch_add(1, Ordering::Relaxed);
    }

    /// Record one successful resolve that hit a row published under `point`.
    pub(crate) fn note_use(&self, point: Option<BlockagePoint>) {
        match point {
            Some(p) => &self.uses[p.index()],
            None => &self.uses_uncovered,
        }
        .fetch_add(1, Ordering::Relaxed);
    }

    /// Record one resolve that faulted.
    pub(crate) fn note_use_miss(&self) {
        self.use_misses.fetch_add(1, Ordering::Relaxed);
    }

    /// A plain-data copy, so a caller can sum many tables without holding any of them.
    #[must_use]
    pub fn snapshot(&self) -> BlockageCounts {
        BlockageCounts {
            binds: [
                self.binds[0].load(Ordering::Relaxed),
                self.binds[1].load(Ordering::Relaxed),
                self.binds[2].load(Ordering::Relaxed),
            ],
            binds_uncovered: self.binds_uncovered.load(Ordering::Relaxed),
            uses: [
                self.uses[0].load(Ordering::Relaxed),
                self.uses[1].load(Ordering::Relaxed),
                self.uses[2].load(Ordering::Relaxed),
            ],
            uses_uncovered: self.uses_uncovered.load(Ordering::Relaxed),
            use_misses: self.use_misses.load(Ordering::Relaxed),
            uncovered_vas: Vec::new(),
            uncovered_vas_total: 0,
        }
    }
}

/// ★★★ **A plain-data blockage census** — one table's, or the sum of many.
///
/// Every field is a whole count. ⊘ [`Self::uncovered_vas`] is the **only** capped thing here
/// and it is capped by the *producer*, which states its total in
/// [`Self::uncovered_vas_total`]; `a_capture_derived_table_expires` and the `dlen=0` class
/// are both the same mistake — a truncated artefact that reads as a complete one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockageCounts {
    /// Binds attributed to each point, by [`BlockagePoint::index`].
    pub binds: [u64; 3],
    /// Binds under no declared halt.
    pub binds_uncovered: u64,
    /// Uses that hit a row published under each point.
    pub uses: [u64; 3],
    /// ★★★★★ **THE C1 NUMBER** — uses that hit a row published under no declared halt.
    pub uses_uncovered: u64,
    /// Resolves that faulted.
    pub use_misses: u64,
    /// A sample of the VAs of rows published under no declared halt. ⊘ Capped; see
    /// [`Self::uncovered_vas_total`].
    pub uncovered_vas: Vec<u64>,
    /// How many uncovered rows exist in total — the denominator
    /// [`Self::uncovered_vas`] is a sample of. Exact.
    pub uncovered_vas_total: usize,
}

impl BlockageCounts {
    /// Fold `other` into `self`. The uncovered-VA sample is concatenated up to `cap`; the
    /// **total** is summed exactly, so an aggregate can never under-report.
    pub fn add(&mut self, other: &BlockageCounts, cap: usize) {
        // ⊘ Quantified over the enum, not over `0..3`: a fourth point would silently be
        // dropped from every aggregate by a hand-written bound (`gates_quantified_over_a_list`).
        for p in BlockagePoint::ALL {
            let i = p.index();
            self.binds[i] = self.binds[i].saturating_add(other.binds[i]);
            self.uses[i] = self.uses[i].saturating_add(other.uses[i]);
        }
        self.binds_uncovered = self.binds_uncovered.saturating_add(other.binds_uncovered);
        self.uses_uncovered = self.uses_uncovered.saturating_add(other.uses_uncovered);
        self.use_misses = self.use_misses.saturating_add(other.use_misses);
        self.uncovered_vas_total = self
            .uncovered_vas_total
            .saturating_add(other.uncovered_vas_total);
        for va in &other.uncovered_vas {
            if self.uncovered_vas.len() >= cap {
                break;
            }
            self.uncovered_vas.push(*va);
        }
    }

    /// **The denominator**: every mapping that became usable in this boot, covered or not.
    #[must_use]
    pub fn publications(&self) -> u64 {
        self.binds
            .iter()
            .fold(self.binds_uncovered, |a, b| a.saturating_add(*b))
    }

    /// Every resolve that found a row — the denominator [`Self::uses_uncovered`] is a
    /// numerator of.
    #[must_use]
    pub fn uses_total(&self) -> u64 {
        self.uses
            .iter()
            .fold(self.uses_uncovered, |a, b| a.saturating_add(*b))
    }

    /// ★★★★★ **THE VERDICT, and it is three-valued on purpose.**
    ///
    /// ⊘ A `bool` here would be the defect this module was written against: `false` for
    /// *"nothing was ever armed"* and `false` for *"a mapping slipped"* are opposite
    /// findings, and `true` for *"armed, and nothing slipped"* and `true` for *"we never
    /// looked"* is the vacuous green `every_row_verified_over_zero_rows` records.
    #[must_use]
    pub fn verdict(&self, entries: [u64; 3]) -> BlockageVerdict {
        if entries.iter().all(|e| *e == 0) {
            return BlockageVerdict::NeverArmed;
        }
        if self.publications() == 0 {
            return BlockageVerdict::NoPublications;
        }
        if self.uses_uncovered > 0 || self.binds_uncovered > 0 {
            return BlockageVerdict::Uncovered;
        }
        BlockageVerdict::Covered
    }

    /// The one-line verdict a grader greps.
    ///
    /// ★★★ `cap` bounds the **printed** VA list and nothing else. Every count on this line
    /// is exact at every `cap`, including `0` — the rule
    /// [`kayfabe_util::coverage`] exists to enforce, restated because this is a second
    /// producer of it.
    #[must_use]
    pub fn render(&self, entries: [u64; 3], cap: usize) -> String {
        use core::fmt::Write as _;
        let mut s = String::new();
        let v = self.verdict(entries);
        let _ = write!(s, "BLOCKAGE-COVERAGE {} armed=[", v.name());
        for (i, p) in BlockagePoint::ALL.iter().enumerate() {
            // ⊘ Keyed by `p.index()`, never by the loop counter. They agree today; a reorder
            // of `ALL` would silently re-key every column and the line would still look
            // well-formed — `a_count_cannot_see_a_substitution`, at the renderer.
            let _ = write!(
                s,
                "{}{}={}",
                if i == 0 { "" } else { " " },
                p.name(),
                entries[p.index()]
            );
        }
        let _ = write!(s, "] publications={} by=[", self.publications());
        for (i, p) in BlockagePoint::ALL.iter().enumerate() {
            let _ = write!(
                s,
                "{}{}={}",
                if i == 0 { "" } else { " " },
                p.name(),
                self.binds[p.index()]
            );
        }
        let _ = write!(
            s,
            " none={}] uses={} uses_by=[",
            self.binds_uncovered,
            self.uses_total()
        );
        for (i, p) in BlockagePoint::ALL.iter().enumerate() {
            let _ = write!(
                s,
                "{}{}={}",
                if i == 0 { "" } else { " " },
                p.name(),
                self.uses[p.index()]
            );
        }
        let _ = write!(
            s,
            "] ⇒ USES_UNCOVERED={} use_misses={} uncovered_rows={}",
            self.uses_uncovered, self.use_misses, self.uncovered_vas_total
        );
        // ⊘ The list, and its own truncation marker. An address absent from a capped list is
        // not thereby covered, and the line has to say so or a reader will read it as a set.
        let shown = self.uncovered_vas.len().min(cap);
        let _ = write!(s, " uncovered_vas=[");
        for (i, va) in self.uncovered_vas.iter().take(shown).enumerate() {
            let _ = write!(s, "{}0x{va:x}", if i == 0 { "" } else { " " });
        }
        let _ = write!(s, "]");
        if shown < self.uncovered_vas_total {
            let _ = write!(
                s,
                " ⚠SHOWING {shown} of {} — an address absent from this list is NOT thereby covered",
                self.uncovered_vas_total
            );
        }
        if matches!(v, BlockageVerdict::NeverArmed) {
            s.push_str(
                " ⊘ NO BLOCKAGE POINT WAS EVER ENTERED — every zero on this line is an \
                 UNMEASURED zero, not a clean one",
            );
        }
        if matches!(v, BlockageVerdict::NoPublications) {
            s.push_str(" ⊘ NOTHING WAS EVER PUBLISHED — the ratio has no denominator");
        }
        s
    }
}

/// The three-valued reading of a [`BlockageCounts`]. See [`BlockageCounts::verdict`] for why
/// it is not a `bool`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockageVerdict {
    /// ★ Points were entered, mappings were published, and **every** use hit a row a halt
    /// covered. This is C1 passing.
    Covered,
    /// ⊘ At least one mapping was published, or used, under no declared halt. C1 failing —
    /// the count and the VAs are on the line.
    Uncovered,
    /// ⊘ **Not a pass.** No blockage point was ever entered, so every zero is unmeasured.
    NeverArmed,
    /// ⊘ **Not a pass.** Points were entered but nothing was ever published, so the
    /// predicate quantifies over the empty set — `every_row_verified_over_zero_rows`.
    NoPublications,
}

impl BlockageVerdict {
    /// Every reading, so a consumer quantifies over the enum.
    pub const ALL: [BlockageVerdict; 4] = [
        BlockageVerdict::Covered,
        BlockageVerdict::Uncovered,
        BlockageVerdict::NeverArmed,
        BlockageVerdict::NoPublications,
    ];

    /// ★ **Is this reading a PASS?** ⊘ Exactly one of the four is, and the other three are
    /// three different reasons it is not — which is why the caller asks by name instead of
    /// matching and deciding for itself what each arm implies.
    #[must_use]
    pub const fn is_pass(self) -> bool {
        matches!(self, BlockageVerdict::Covered)
    }

    /// The name a diagnostic prints.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            BlockageVerdict::Covered => "COVERED",
            BlockageVerdict::Uncovered => "⊘UNCOVERED",
            BlockageVerdict::NeverArmed => "⊘NEVER-ARMED",
            BlockageVerdict::NoPublications => "⊘NO-PUBLICATIONS",
        }
    }
}

impl core::fmt::Display for BlockageVerdict {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⊘ **EVERY VERDICT IS DISTINGUISHABLE IN PRINT, and exactly one is a pass.** A renderer
    /// that spelled two readings the same way would make a grader's `grep` report a boot that
    /// never armed as a boot that passed — the whole failure this type is shaped against.
    #[test]
    fn every_verdict_prints_differently_and_exactly_one_is_a_pass() {
        let mut seen: Vec<&str> = Vec::new();
        let mut passes = 0;
        for v in BlockageVerdict::ALL {
            assert!(!seen.contains(&v.name()), "{v} shares a printed name");
            seen.push(v.name());
            if v.is_pass() {
                passes += 1;
            }
        }
        assert_eq!(seen.len(), BlockageVerdict::ALL.len());
        assert_eq!(passes, 1, "★ exactly one of the four readings is a pass");
        assert!(BlockageVerdict::Covered.is_pass());
    }

    /// ⊘ Names are distinct and stable — a grader greps them, and two points sharing a word
    /// would make every boot line ambiguous about the split the model turns on.
    #[test]
    fn every_points_name_is_distinct_and_the_index_is_a_bijection() {
        let mut seen: Vec<&str> = Vec::new();
        let mut idx: Vec<usize> = Vec::new();
        for p in BlockagePoint::ALL {
            assert!(!seen.contains(&p.name()), "{p} shares a name");
            assert!(!idx.contains(&p.index()), "{p} shares an index");
            assert!(p.index() < BlockagePoint::ALL.len());
            seen.push(p.name());
            idx.push(p.index());
        }
        assert_eq!(seen.len(), BlockagePoint::ALL.len());
    }

    /// ★★★ The guard sets the point, restores the previous one, and **nests innermost-wins**.
    #[test]
    fn a_guard_shadows_and_restores() {
        assert_eq!(current(), None, "a fresh test thread is inside no point");
        {
            let _outer = BlockageGuard::enter(BlockagePoint::GspRpc);
            assert_eq!(current(), Some(BlockagePoint::GspRpc));
            {
                let _inner = BlockageGuard::enter(BlockagePoint::EmulatedDoorbell);
                assert_eq!(
                    current(),
                    Some(BlockagePoint::EmulatedDoorbell),
                    "★ the INNERMOST halt is the answer: it is the one closest to the \
                     publication and the one whose budget the publication spends"
                );
            }
            assert_eq!(
                current(),
                Some(BlockagePoint::GspRpc),
                "the outer is restored"
            );
        }
        assert_eq!(current(), None);
    }

    /// ★★★★★ **THE ARMING EVIDENCE IS ITSELF CHECKED — by this test and nothing else.**
    /// Entering a guard moves the per-thread counter for that point and no other, which is
    /// the property a boot line leans on when it distinguishes a zero somebody looked at
    /// from a zero nobody did. ⊘ Nothing here is a claim about a guest; it is a claim about
    /// the counter, and this test is the only thing behind it.
    ///
    /// ⊘ Asserted on the THREAD counter, not the global one: the global is shared with every
    /// other test in this binary and a delta over it is a race, not an assertion.
    #[test]
    fn entering_a_guard_arms_exactly_one_point() {
        let before = thread_entries();
        {
            let _g = BlockageGuard::enter(BlockagePoint::TlbInvalidate);
        }
        let after = thread_entries();
        assert_eq!(
            after[BlockagePoint::TlbInvalidate.index()],
            before[BlockagePoint::TlbInvalidate.index()] + 1
        );
        assert_eq!(
            after[BlockagePoint::EmulatedDoorbell.index()],
            before[BlockagePoint::EmulatedDoorbell.index()],
            "⊘ arming one point must not arm another — a census that moves every counter \
             together cannot attribute anything"
        );
        assert_eq!(
            after[BlockagePoint::GspRpc.index()],
            before[BlockagePoint::GspRpc.index()]
        );
    }

    /// ★★★★★ **THE VERDICT IS THREE-VALUED, AND THE TWO ZEROS ARE DIFFERENT FINDINGS.**
    ///
    /// This is the whole of §3: a bare `uses_uncovered == 0` is satisfied by a boot where
    /// nothing was armed, by a boot where nothing was published, and by a boot that actually
    /// passed. All three are asserted here, so a future edit that collapses them goes red.
    #[test]
    fn a_never_armed_zero_and_a_measured_zero_do_not_read_the_same() {
        let empty = BlockageCounts::default();
        assert_eq!(empty.verdict([0, 0, 0]), BlockageVerdict::NeverArmed);
        assert_eq!(empty.verdict([7, 0, 0]), BlockageVerdict::NoPublications);

        let mut clean = BlockageCounts::default();
        clean.binds[BlockagePoint::GspRpc.index()] = 3;
        clean.uses[BlockagePoint::GspRpc.index()] = 9;
        assert_eq!(clean.verdict([0, 4, 0]), BlockageVerdict::Covered);

        let mut dirty = clean.clone();
        dirty.uses_uncovered = 1;
        assert_eq!(dirty.verdict([0, 4, 0]), BlockageVerdict::Uncovered);

        // ⊘ And a publication under no halt is a red on its own, even if nothing used it —
        // the mapping became usable outside every halt, which is the hazard, not the touch.
        let mut published_blind = clean.clone();
        published_blind.binds_uncovered = 1;
        assert_eq!(
            published_blind.verdict([0, 4, 0]),
            BlockageVerdict::Uncovered
        );
    }

    /// ★★★ **THE PRINT CAP TRUNCATES THE LIST AND NEVER THE VERDICT** — the same property
    /// `kayfabe_util::coverage` proves for its own renderer, proved again here because this
    /// is a second producer and the rule is not inherited by being nearby.
    #[test]
    fn the_print_cap_truncates_the_list_and_never_the_counts() {
        let mut c = BlockageCounts::default();
        c.binds[0] = 5;
        c.binds_uncovered = 40;
        c.uses_uncovered = 40;
        c.uncovered_vas = (0..40u64).map(|i| 0x1000 * i).collect();
        c.uncovered_vas_total = 40;

        let wide = c.render([1, 0, 0], 100);
        let narrow = c.render([1, 0, 0], 4);
        for line in [&wide, &narrow] {
            assert!(
                line.contains("USES_UNCOVERED=40"),
                "count truncated: {line}"
            );
            assert!(
                line.contains("uncovered_rows=40"),
                "total truncated: {line}"
            );
            assert!(line.contains("⊘UNCOVERED"), "verdict truncated: {line}");
        }
        assert!(
            narrow.contains("SHOWING 4 of 40"),
            "★ a capped list must say so IN the line: an address absent from a truncated \
             sample is not thereby covered, and a reader scanning for a VA would conclude \
             the opposite. line={narrow}"
        );
        assert!(
            !wide.contains("SHOWING"),
            "an uncapped list must not claim truncation"
        );
    }

    /// ⊘ The never-armed line **says** it is unmeasured, in words, on the line itself. A
    /// grader that only reads counts would otherwise record a clean boot.
    #[test]
    fn the_never_armed_line_declares_itself_unmeasured() {
        let line = BlockageCounts::default().render([0, 0, 0], 8);
        assert!(line.contains("⊘NEVER-ARMED"), "{line}");
        assert!(line.contains("UNMEASURED zero"), "{line}");
        assert!(
            line.contains("publications=0"),
            "the denominator is always printed: {line}"
        );
    }

    /// ★★ Aggregation sums the counts exactly and the VA list only up to `cap`, while the
    /// **total** is still summed exactly — so a whole-boot line can never under-report the
    /// population its sample is drawn from.
    #[test]
    fn aggregation_sums_counts_exactly_and_only_samples_the_list() {
        let mut a = BlockageCounts {
            uncovered_vas: vec![1, 2, 3],
            uncovered_vas_total: 3,
            uses_uncovered: 3,
            ..Default::default()
        };
        let b = BlockageCounts {
            uncovered_vas: vec![4, 5, 6],
            uncovered_vas_total: 3,
            uses_uncovered: 3,
            ..Default::default()
        };
        a.add(&b, 4);
        assert_eq!(a.uses_uncovered, 6);
        assert_eq!(a.uncovered_vas_total, 6, "the TOTAL is exact");
        assert_eq!(a.uncovered_vas.len(), 4, "the SAMPLE is capped");
    }

    /// ⊘ Non-vacuity for the census helpers themselves: `note_bind`/`note_use` must land in
    /// the arm the point names, or every attribution on the boot line is a coincidence.
    #[test]
    fn the_census_attributes_to_the_named_point() {
        let c = BlockageCensus::default();
        c.note_bind(Some(BlockagePoint::TlbInvalidate));
        c.note_bind(None);
        c.note_use(Some(BlockagePoint::TlbInvalidate));
        c.note_use(None);
        c.note_use_miss();
        let s = c.snapshot();
        assert_eq!(s.binds[BlockagePoint::TlbInvalidate.index()], 1);
        assert_eq!(s.binds[BlockagePoint::GspRpc.index()], 0);
        assert_eq!(s.binds_uncovered, 1);
        assert_eq!(s.uses[BlockagePoint::TlbInvalidate.index()], 1);
        assert_eq!(s.uses_uncovered, 1);
        assert_eq!(s.use_misses, 1);
        assert_eq!(s.publications(), 2);
        assert_eq!(s.uses_total(), 2);
    }
}
