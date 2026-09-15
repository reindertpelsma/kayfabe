//! ★★★★★ **w393 — THE DEMAND-DRIVEN BAR1/BAR2 MIRROR**: a memslot per guest-touched aperture
//! page, filled on first-touch miss, revalidated on the guest's own TLB invalidate.
//!
//! # What it is — the guest's BAR TLB, made of memslots
//!
//! `docs/design/bar1_passthrough_device_local_host_visible.md` §2.3 names two ways to mirror
//! the guest's BAR page tables into memslots: *"install on PTE publication or on
//! first-touch"*. This is the second, and the owner's direction (2026-09-09, commit
//! `f55984af`) says why it is the one to build: an observation-driven mirror needs BAR2 to
//! stay trapped as the watchpoint for page-table writes; a demand-driven one needs nothing
//! observed at all — **the access IS the notification**. That is what lets BAR2 be
//! RAM-shaped too, and it dissolves *"there is no universal publish trigger"* for the BAR
//! apertures: a miss cannot be missed.
//!
//! The mirror is exactly a TLB:
//!
//! | TLB | this |
//! |---|---|
//! | fill on miss | a trapped BAR1/BAR2 access ([`BarMirror::fill`]) walks the guest's BAR page table (the same walk the trap performed), asks the store what memory backs the frame, and installs a 4 KiB memslot over **that memory**. ⊘ Since w613 `premap_window` installs pages the SAME way with no access at all, so a slot is **not** evidence of an exit — see [`FillOrigin`] and read `TRAP_FILLS`, never a total |
//! | flush on invalidate | the guest's `MMU_INVALIDATE` trigger ([`BarMirror::revalidate`]) re-walks every live entry and drops the ones whose translation or backing changed — the exact GPU boundary the owner ranks first (`publish_trigger_preference_ordering.md`) |
//! | entry = one memory | the slot's pages ARE the store's pages (the page arena) or the join's `memfd` — there is never a second copy, so nothing is carried, revoked or merged |
//!
//! # ★★★ The one invariant, and the two places it is enforced
//!
//! > **A memslot over a framebuffer range exists only while the store serves that range
//! > from the very pages the slot names.**
//!
//! 1. **Before the store moves a range's bytes** (a join install copies local pages into
//!    the join and drops them; a release moves them back or drops them; a device reset
//!    drops everything) the plane calls [`kayfabe_device::FbMirrorPort::quiesce`] — with no
//!    ranked lock held — and this mirror retires every slot over the range and refuses new
//!    fills there until `resume`. A guest store landing in the window traps and is served
//!    by the store, correctly.
//! 2. **After installing a slot** the fill re-resolves the page under the plane lock and
//!    keeps the slot only if the translation and the backing are what it installed over
//!    ([`BarMirror::fill`] phase 3). A quiesce, a revalidate or a competing fill that ran in
//!    between cancels the fill's *pending* ticket, and the slot is removed before anyone
//!    can hit it.
//!
//! ⊘ Every memslot ioctl runs **outside** the plane's `RankedMutex` (R1): the plane is
//! locked only for the page walk + backing lookup, released, and the `mmap`/`ioctl` run
//! lock-free. The mirror's own table is a plain `Mutex`, never held across a syscall.
//!
//! # What is counted, and what a zero means
//!
//! `fills` per window, `distinct_pages` (aperture pages ever covered) and `distinct_frames`
//! (framebuffer frames ever covered) — the acceptance ratio is *misses / distinct frames*;
//! every refusal by name; revalidation runs, kept, removed; quiesce calls and the slots
//! they retired; fills that lost their race. ⚠ `fills=0` under an armed BAR means **nothing
//! touched that BAR** unless the C's own `bar1_touches` says otherwise — read them together.

use std::collections::{HashMap, HashSet};
use std::os::fd::{AsFd, OwnedFd};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use kayfabe_device::{
    FbArenaPage, FbMirrorPort, FbPageArena, FbPageBacking, FbPageExport, FbWindow, RegPlane,
    WindowPageResolution,
};
use kayfabe_linux_raw::{ArenaPage, HostPageSize, SharedPageArena};
use kayfabe_vmm::{BarId, RamRegionId, VmmError};
use kayfabe_vmm_qemu::QemuMachine;

/// ★★★★★ **Why a page got a memslot** — and it is a parameter rather than a guess, because the
/// two origins answer two different questions and one counter served both until w696.
///
/// ⊘ `Trap` is a guest EXIT: the access faulted, we walked, we installed. That is the number
/// goal 2 is about. `Premap` is us installing ahead of the guest, which costs the guest nothing
/// and must never be added to an exit count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FillOrigin {
    /// A trapped guest access — one MMIO exit.
    Trap,
    /// Installed ahead of any access by `premap_window`.
    Premap,
}

/// The page the mirror deals in — the store's `FB_PAGE`, the arena's `ARENA_PAGE`.
const PAGE: u64 = 4096;

/// The export token that names the page arena. Joins are numbered from 1.
const ARENA_TOKEN: u64 = 0;

/// ★★★★★ **§3's THIRD TOKEN SPACE — the one reserved device-local video-memory object.**
///
/// `SINGLE_STORE_PLAN.md` §3: *"the third token space beside `ARENA_TOKEN`/`JoinRegistry`"*.
///
/// # ⊘⊘ Why `u64::MAX` and not `1`, and why the choice is load-bearing
///
/// [`Key::token`] is a bare `u64` and [`key_of`] deliberately erases which arm of
/// [`FbPageBacking`] produced it, so the three spaces must be **disjoint by value**:
///
/// | space | values | minted by |
/// |---|---|---|
/// | the page arena | exactly `ARENA_TOKEN` (`0`) | `ArenaPagePort::export` |
/// | joins | `1 ..` ascending, forever | `JoinRegistry::register` |
/// | **the reserved object** | exactly `DEVICE_TOKEN` | [`key_of`], from the address |
///
/// ⇒ the join space grows upward from `ARENA_TOKEN + 1` and the device token is the top of
/// the range, so a collision needs `2^64 - 1` joins in one boot. ⊘ Stated as a table rather
/// than assumed, because the survey of this file found the `Arena ⇒ ARENA_TOKEN` invariant to
/// be **cross-crate and untyped**: `kayfabe-device` mints the token and this file decides
/// what it means, and nothing but these constants keeps them agreeing. A second arena
/// returning any other value would be looked up in the `JoinRegistry` and refused `JOIN-GONE`.
const DEVICE_TOKEN: u64 = u64::MAX;

/// How many refused fills are printed live per name (the total is uncapped).
const REFUSAL_LIVE: u64 = 4;

/// How many revalidation runs are printed live (the totals are uncapped).
const REVAL_LIVE: u64 = 8;

/// Print a running census every this many fills, so a boot that dies before teardown still
/// carries one.
const CENSUS_EVERY_FILLS: u64 = 512;

/// ★ w617 — the enumeration budget for BAR1, in page-table ENTRIES (see `PREMAP_BUDGET_BAR2`
/// for why that unit matters). ⊘ It has never refused: BAR1's tree is small enough that 2 048
/// entries reach every leaf, which is why the unit error stayed invisible on this arm.
const PREMAP_BUDGET_BAR1: u32 = 2048;

/// ★★★★★ **w626 — BAR2 NEEDS ITS OWN, AND 2 048 WAS WHY IT NEVER RAN.**
///
/// ⊘⊘ `[measured w625a]` the BAR2 enumeration was refused on **all 1 181** invalidates with
/// *"the enumeration did not complete (unbacked page, malformed entry, or exhausted budget)"* —
/// and `decode_subtree`'s own contract settles which without another boot: **it returns `Err`
/// for `BudgetExhausted` and for nothing else.** Every other fault is per-branch and comes back
/// in `SubtreeDecode::faults` with the leaves that did decode.
///
/// ⊘⊘⊘ **AND I GOT THE UNITS WRONG, corrected w627 by the measurement I asked for.** This said
/// *"the budget counts page-table PAGES visited"*. It does not: `decode_subtree` charges
/// `cost = level_shift(level).entries` **per page** — the number of ENTRIES in that page, 512
/// or 1 024 on GA10x. `[measured w626a]` BAR2's tree is **19 pages**, which would have fit a
/// 2 048 *page* budget a hundred times over; at ~512–1 024 entries each it needs **~19 000**,
/// and 2 048 buys two or three pages before it refuses.
///
/// ⇒ The diagnosis was right in mechanism and wrong in units, and the instrument caught it:
/// `bar2_visited=19` beside a budget of 2 048 is a contradiction that had to be explained, not
/// a confirmation. ⚠ **Third time this session I have asserted what a number counts without
/// reading the site that consumes it** — after a doc comment that lied (w607) and an identity
/// assumed in a comment (w617). ⇒ `1 << 20` stays: it is ~50× the measured need, the walk is
/// bounded by the tree rather than by this, and a right-sized 64 KiB-ish value would buy
/// nothing but a second chance to be wrong about the unit.
const PREMAP_BUDGET_BAR2: u32 = 1 << 20;

// ---- refusal names -----------------------------------------------------------------------

/// ★★★ **CUT B — the `premap_why` key for a SHORT enumeration**, so the once-per-reason rule
/// keeps it apart from the refusal reasons beside it. ⊘ A short list and a refused enumeration
/// are different findings with different fixes, and one key for both would print whichever
/// happened first and swallow the other for the rest of the boot.
const PREMAP_SHORT: &str = "ENUMERATION-SHORT";

const R_OUT_OF_BAR: &str = "OUT-OF-BAR";
const R_TRANSLATION: &str = "TRANSLATION-REFUSED";
const R_NO_ADDRESS_MODEL: &str = "NO-ADDRESS-MODEL";
const R_HEAP_PAGE: &str = "HEAP-PAGE";
const R_STORE: &str = "STORE-REFUSED";
const R_JOIN_GONE: &str = "JOIN-GONE";
const R_QUIESCED: &str = "QUIESCED";
const R_PENDING: &str = "PENDING-ELSEWHERE";
const R_COVERED: &str = "ALREADY-COVERED";
/// ★ w611 — the same fact, found BEFORE the plane lock. See the phase-0 check in `fill_now`.
const R_COVERED_EARLY: &str = "ALREADY-COVERED-EARLY";
const R_RACED: &str = "RACED-AND-DROPPED";
/// ★★★ §3 — the store named a page of the reserved object and no device-view port exists.
const R_NO_DEVICE_PORT: &str = "NO-DEVICE-PORT";
/// ★★★ §3 — a device view would have been armed from a vCPU. See the guard in `fill_now`.
const R_ON_VCPU: &str = "DEVICE-VIEW-ON-VCPU";
/// ★★★ §3 — the port refused to arm a view. The sentence is the port's own four-way name;
/// `RM_REFUSED` there means the host BAR1 **aperture** is full, not that vidmem ran out.
const R_VIEW_REFUSED: &str = "DEVICE-VIEW-REFUSED";
const R_INSTALL: &str = "INSTALL-REFUSED";
/// ★★★★★ **THE LANDMINE ARM** — a backing [`key_of`] keyed and this match did not.
///
/// ⊘⊘⊘ This match used to end in `_ => {}`, and `key_of`'s does not: `key_of` is exhaustive,
/// so a new [`FbPageBacking`] arm is a **compile error** there and was a **silent no-op**
/// here. ⇒ a new arm that compiled would have become an unnamed refusal — the mirror would
/// install no slot, count nothing, and say nothing, which reads in a boot log exactly like a
/// window nobody touched. `SINGLE_STORE_PLAN.md` §3 names it as the trap this increment must
/// disarm before adding its own arm.
///
/// ⇒ The `_` is gone. Every arm is spelled, this one is the "keyed but unkeyed" contradiction,
/// and the next arm anyone adds is a compile error in **both** matches.
const R_UNKEYED: &str = "KEYED-BUT-UNKEYED";

// ---- the join export registry ------------------------------------------------------------

/// ★★★★★ **The join registry** — `token → descriptor` for every joined leaf whose backing
/// can be named by a memslot.
///
/// Process-global for the reason `minted_join_ledger` is: the one place a join's `memfd`
/// is in scope is inside `join_one_fb_leaf`, which must not grow a parameter for this. An
/// entry is inserted when the VMM maps a **shared** join and removed by the `MappedFb`'s
/// own `Drop`, so a token the registry does not know is a join that is gone — the fill
/// refuses it by name (`JOIN-GONE`) rather than mapping a reused descriptor number.
#[derive(Debug, Default)]
pub struct JoinRegistry {
    next: AtomicU64,
    fds: Mutex<HashMap<u64, Arc<OwnedFd>>>,
}

impl JoinRegistry {
    /// The process's registry.
    pub fn global() -> &'static JoinRegistry {
        static R: std::sync::OnceLock<JoinRegistry> = std::sync::OnceLock::new();
        R.get_or_init(|| JoinRegistry {
            next: AtomicU64::new(ARENA_TOKEN + 1),
            fds: Mutex::new(HashMap::new()),
        })
    }

    /// Register a duplicate of `fd`; the token names it until [`JoinRegistry::forget`].
    pub fn register(&self, fd: OwnedFd) -> u64 {
        let token = self.next.fetch_add(1, Ordering::Relaxed);
        self.fds
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(token, Arc::new(fd));
        token
    }

    /// Forget a token (the join is being unmapped).
    pub fn forget(&self, token: u64) {
        self.fds
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&token);
    }

    fn get(&self, token: u64) -> Option<Arc<OwnedFd>> {
        self.fds
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&token)
            .cloned()
    }
}

/// The half of a `MappedFb` that names its `memfd`: registered at map time, forgotten at
/// drop. `offset` is the join's byte offset **within** the file.
#[derive(Debug)]
pub struct JoinExport {
    token: u64,
    offset: u64,
}

impl JoinExport {
    /// Register `fd` (duplicated) so a memslot can be placed over the join at `offset`.
    /// `None` when the descriptor could not be duplicated — the join then simply is not
    /// mirrorable, by name (`JOIN_NOT_EXPORTABLE`), and nothing else changes.
    pub fn register(fd: &OwnedFd, offset: u64) -> Option<JoinExport> {
        let dup = fd.try_clone().ok()?;
        Some(JoinExport {
            token: JoinRegistry::global().register(dup),
            offset,
        })
    }

    /// What the store hands out for byte 0 of this join.
    pub fn export(&self) -> FbPageExport {
        FbPageExport {
            token: self.token,
            offset: self.offset,
        }
    }
}

impl Drop for JoinExport {
    fn drop(&mut self) {
        JoinRegistry::global().forget(self.token);
    }
}

// ---- the arena, as the store's port --------------------------------------------------------

/// [`FbPageArena`] over [`SharedPageArena`].
#[derive(Debug)]
pub struct ArenaPort(SharedPageArena);

/// [`FbArenaPage`] over [`ArenaPage`].
#[derive(Debug)]
struct ArenaPagePort(ArenaPage);

impl FbPageArena for ArenaPort {
    fn alloc_at(&mut self, addr: u64) -> Result<Box<dyn FbArenaPage>, &'static str> {
        // ★ w569 — the framebuffer address IS the file offset. See `SharedPageArena::alloc_at`
        // for why that replaced an allocator rather than gaining a parameter.
        self.0
            .alloc_at(addr)
            .map(|p| Box::new(ArenaPagePort(p)) as Box<dyn FbArenaPage>)
    }

    fn read_at(&self, addr: u64, off: u64, buf: &mut [u8]) -> Result<(), &'static str> {
        // ⊘⊘ **w587 — this took a TRANSIENT `ArenaPage` and dropped it, and that was the
        // regression.** `alloc_at` + `Drop` is two mutex acquisitions and — before w587 removed
        // the vestigial free list — one unbounded `Vec` push, **per trapped framebuffer read,
        // on the vCPU inside an MMIO exit.** `[measured w587]` the boot's GSP RPCs went from
        // microseconds to ~60 ms each and then hung.
        //
        // ★ A read needs no handle. `SharedPageArena::read_at` goes straight to the mapping
        // with an arithmetic bound, which is what "the address IS the file offset" buys.
        self.0
            .read_at(addr, off, buf)
            .map_err(|_| ARENA_OUT_OF_PAGE)
    }

    fn reset(&mut self) -> Result<(), &'static str> {
        self.0.punch_all().map_err(|_| ARENA_PUNCH_REFUSED)
    }
}

const ARENA_PUNCH_REFUSED: &str = "the arena refused to return its pages to zero";

const ARENA_OUT_OF_PAGE: &str = "that access falls outside the arena page";

impl FbArenaPage for ArenaPagePort {
    fn read(&self, off: u64, buf: &mut [u8]) -> Result<(), &'static str> {
        self.0.read_into(off, buf).map_err(|_| ARENA_OUT_OF_PAGE)
    }

    fn write(&mut self, off: u64, bytes: &[u8]) -> Result<(), &'static str> {
        self.0.write_from(off, bytes).map_err(|_| ARENA_OUT_OF_PAGE)
    }

    fn export(&self) -> FbPageExport {
        FbPageExport {
            token: ARENA_TOKEN,
            offset: self.0.file_offset(),
        }
    }
}

// ---- the mirror ----------------------------------------------------------------------------

/// One armed aperture: where the BAR sits in guest-physical space.
#[derive(Debug, Clone, Copy)]
struct Arm {
    base: u64,
    len: u64,
}

/// What a slot was installed over — compared on revalidation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Key {
    phys: u64,
    token: u64,
    offset: u64,
    readonly: bool,
}

#[derive(Debug, Clone, Copy)]
struct Slot {
    region: RamRegionId,
    window: FbWindow,
    page_off: u64,
    key: Key,
    /// ★★★★★ **§3 — the armed device view this slot's memory came from**, as
    /// [`crate::deviceview::ViewId`]'s inner `u64`. `None` for arena and join slots, which
    /// name a file this process already holds and cost no host BAR1 aperture.
    ///
    /// ⊘ A `u64` and not the [`crate::deviceview::ArmedView`] itself, because this struct is
    /// [`Copy`] and is snapshotted by value under a lock in three places. An armed view is a
    /// **resource**: two copies of one release is a double-release or a leak depending on
    /// which runs. The port keeps the resources; this names one.
    view: Option<u64>,
}

/// ★★★ **A SLOT ON ITS WAY OUT** — what [`BarMirror::retire`] needs to undo, all of it.
///
/// ⊘ It replaced a bare `(gpa, RamRegionId)` tuple. Under §3 a removed slot may owe a second
/// obligation that the region id cannot carry — see [`Slot::view`] — and a tuple that carried
/// only the first would have looked complete.
#[derive(Debug, Clone, Copy)]
struct Retired {
    gpa: u64,
    region: RamRegionId,
    view: Option<u64>,
}

#[derive(Debug, Default)]
struct Table {
    /// Guest-physical page → the slot over it.
    slots: HashMap<u64, Slot>,
    /// Guest-physical page → `(ticket, phys)` of a fill between its install and its commit.
    pending: HashMap<u64, (u64, u64)>,
    /// Framebuffer ranges the plane is moving bytes for right now: `(phys, len)`.
    quiesced: Vec<(u64, u64)>,
    next_ticket: u64,
    /// ★★★ Bumped at the START of every revalidation. A fill that took its ticket before a
    /// revalidation began and commits after it took its snapshot would install a slot the
    /// snapshot never saw, over a translation the invalidate may have retired — so a fill
    /// whose epoch moved between ticket and commit drops its slot (one extra trap on the
    /// next touch, never a stale mapping).
    reval_epoch: u64,
    /// Every aperture page ever covered, per window (0 = BAR1, 1 = BAR2).
    pages_ever: [HashSet<u64>; 2],
    /// Every framebuffer frame ever covered, per window.
    frames_ever: [HashSet<u64>; 2],
    /// Refusals by name.
    refused: HashMap<&'static str, u64>,
    /// Slots live now / the most ever.
    live: u64,
    peak: u64,
}

#[derive(Debug, Default)]
struct Census {
    /// ⊘ **TRAP-DRIVEN fills only** — one guest MMIO exit each. See `FillOrigin`.
    fills: [AtomicU64; 2],
    /// Pages installed AHEAD of any access by `premap_window`. ⊘ Deliberately a second field
    /// and not added to `fills`: a premap install costs the guest no exit, and summing the two
    /// is what made `fills` read as thousands of traps when the trap count was zero.
    premap_fills: [AtomicU64; 2],
    reval_runs: AtomicU64,
    reval_kept: AtomicU64,
    reval_removed: AtomicU64,
    reval_printed: AtomicU64,
    quiesce_calls: AtomicU64,
    quiesce_removed: AtomicU64,
    retire_all_calls: AtomicU64,
    retire_all_removed: AtomicU64,
    census_lines: AtomicU64,
}

/// ★★★★★ **The mirror.** One per device; see the module docs.
#[derive(Debug)]
pub struct BarMirror {
    plane: Arc<RegPlane>,
    machine: QemuMachine,
    arena: SharedPageArena,
    arms: [Option<Arm>; 2],
    /// ★★★★★ w578 — the PRAMIN aperture: one installed slot, and the framebuffer address it
    /// currently shows. `None` when the chip declares no window or the install was refused.
    /// `(region, the framebuffer address it shows, the armed device view behind it)`.
    ///
    /// ⊘ The third field is `None` on the arena arm and `Some` on §3's — and it is what makes
    /// a move release-and-re-arm instead of one `MAP_FIXED`. See [`BarMirror::repoint_pramin`].
    pramin: Mutex<Option<(kayfabe_vmm::RamRegionId, u64, Option<u64>)>>,
    /// How many times the latch moved and how many redundant writes were skipped. ⚠ Two
    /// numbers: *"never moved"* and *"wrote the same value forty times"* are different facts.
    /// ★★★★★ **§3's device-view port**, or `None` on every arm but the single store's.
    ///
    /// ⊘ Held here rather than reached through the plane: arming is lock-free by
    /// requirement, and the plane is the thing whose lock the requirement is about.
    device_port: Option<Arc<crate::deviceview::DeviceViewPort>>,
    /// ★★★★★ **Whether the STORE is the reserved object** (`KAYFABE_FB_STORE=device`).
    ///
    /// ⊘⊘ A **separate** fact from `device_port.is_some()`, and conflating them is a
    /// two-memories defect: the port's gate is `KAYFABE_DEVICE_VIEW`, and a boot may arm the
    /// port while the store is the arena. See [`BarMirror::install_pramin_window`].
    device_store: bool,
    /// ★★★★★ **§3 — views whose slot is gone and whose MAPPING may not be.**
    ///
    /// `(region, view id)`. See [`BarMirror::retire`] for why the two events are not the
    /// same one and why releasing on the first is a silent cross-tenant defect.
    parked: Mutex<Vec<(RamRegionId, u64)>>,
    pramin_moves: AtomicU64,
    pramin_skipped: AtomicU64,
    /// ★★★★★ **w652 — HOW LONG THE REPOINT ACTUALLY TAKES, on the vCPU that trapped.**
    ///
    /// ⊘⊘ **Because "not in SLOW-SITES" is a BOUND, not a measurement.** The door census names
    /// `mmap MAP_FIXED (placing a backing inside a window)` as reached on a vCPU **22 times**
    /// per boot — this is the caller — and `SLOW-SITES` lists only the one trap over 1 ms. So
    /// every repoint is under a millisecond and **its real cost is unmeasured**, which is not
    /// the same as zero. Goal 3 is *"no blocking calls on the vCPU"* and goal 6 is *"every
    /// write trap sub-millisecond"*; the first is currently violated by name and the second may
    /// be satisfied by a comfortable margin or by 900 µs, and nothing here could tell them
    /// apart.
    ///
    /// ⚠ The repoint **cannot simply move to a worker**: the guest writes the window register
    /// and reads through the aperture immediately after, so a deferred move shows it the OLD
    /// framebuffer. That is a correctness break, not a latency trade — which is exactly why
    /// the number has to exist before anyone argues about the design.
    ///
    /// # ★★★★★ RULED SUFFICIENT BY THE OWNER, 2026-09-13 — and the ruling turns on WHEN
    ///
    /// `[measured w653a]` `move_ns[worst=296558 mean=74698]` — **297 µs worst, 75 µs mean.**
    /// Owner: *"297us for a thing that only happens at boot is not bad, thats fine for that
    /// mmap."*
    ///
    /// ⊘ **The load-bearing half of that ruling is "only happens at boot", not "297 µs".** The
    /// PRAMIN aperture is a bring-up path (`pramin_is_a_bringup_aperture_not_a_running_path`:
    /// ogkm-580 picks `TRANSFER_TYPE_BAR0` only under `IS_SIMULATION`), so the 22 doors are
    /// spent before the guest is doing work and cost a running workload nothing.
    ///
    /// ⚠ ⇒ **This ruling expires if the moves stop being a boot-time set.** A `moves` count
    /// that grows with runtime, or a repoint observed after the client starts, is a different
    /// question with the same latency — re-ask it then rather than citing this line. The
    /// census prints `moves` beside `move_ns` so that the precondition is checkable and not
    /// merely remembered.
    pramin_move_ns_total: AtomicU64,
    pramin_move_ns_worst: AtomicU64,
    /// **w593 - PRAMIN traffic that had already happened when the slot went in.**
    ///
    /// `[measured w591, arm C]` with the slot live, `window[SERVED r=2 w=3905]` - **not zero**,
    /// and goal 2 asks for no write traps in PRAMIN. The slot goes in on the guest's FIRST
    /// LATCH WRITE (w580: installing at arm reprograms BAR0 underneath and wedges the machine),
    /// so every access before that moment necessarily exits.
    ///
    /// Whether that PREFIX is the whole 3 905 or a fraction decides what to do next, and the
    /// two answers call for opposite work: a prefix means installing earlier, a remainder means
    /// the slot is not covering something it should. The totals cannot tell them apart.
    pramin_at_install: AtomicU64,
    /// ★★★★★ **w595 — the GPA the PRAMIN slot was installed AT.**
    ///
    /// `[measured w594a]` `before_slot=0 after_slot=2977` — every one of the residual window
    /// exits happened with the slot LIVE, so it is not the pre-install prefix. The next
    /// question, and the only one a total cannot answer: **is the slot still where the guest
    /// is looking?** w580 installs on the first latch write precisely because the guest's
    /// firmware reprograms BAR0 — if it moves again afterwards, the slot sits at an address
    /// nothing accesses and every access exits, which looks exactly like "the slot does not
    /// work".
    pramin_gpa: AtomicU64,
    /// ★★★★★ **w597 — WHEN the residual PRAMIN exits arrive, relative to the window moves.**
    ///
    /// `[measured w595]` `before_slot=0 after_slot=3405 gpa=0xfb700000 (BAR0 has not moved)`.
    /// So the slot is live, at the right address, and 3 405 accesses exit anyway. Three
    /// readings fit that equally well and demand different fixes:
    ///
    ///   - a STEADY leak    => the slot is not covering the range it claims to;
    ///   - a BURST after each re-point => the re-point drops coverage and the guest races it;
    ///   - a single BLOCK   => the slot is being removed once and never restored.
    ///
    /// ⊘ A total cannot separate them, and I have now been wrong twice guessing at this
    /// number's shape. ⇒ Record the access count at each re-point; 22 `u64`s, written once per
    /// move, read once at teardown. The deltas ARE the shape.
    pramin_marks: Mutex<Vec<(u64, u64)>>,
    /// ★★★★★ **w637 — the counter page's slot, installed ONCE and never re-pointed.**
    ///
    /// ⊘ Unlike `pramin`, this names a fixed aperture: the usermode window does not move, so
    /// there is no latch to follow and no re-point to refuse. `Some` means the guest reads the
    /// GPU's real clock with no exit; `None` means it is still being served from a host CPU
    /// clock, which is the state every boot before this landed was in.
    counter_slot: Mutex<Option<RamRegionId>>,
    /// ★★★★★ **w603 — BAR0 placement CHANGES, not a comparison of endpoints.**
    ///
    /// ⊘⊘ `[measured w602]` `window_off_trap=0` — every residual window access is a genuine
    /// guest exit, so the slot really is failing to serve. And `RE-POINT REFUSED` never fires,
    /// so the mapping call always succeeds. A slot that is installed, mapped, and still not
    /// serving is a slot KVM no longer has.
    ///
    /// ⚠ **My w595 check cannot see the cause it was built for.** It compares BAR0's placement
    /// at INSTALL against its placement at TEARDOWN and reports *"BAR0 has not moved"*. If the
    /// guest's firmware moves BAR0 away and back — and w580 recorded that it reprograms BAR0 at
    /// all — QEMU's memory listener tears the region down and rebuilds it, dropping a slot we
    /// installed behind its back, while both endpoints still match. ⇒ **An endpoint comparison
    /// cannot detect a round trip**, and that is the exact shape of the bug it would miss.
    bar0_moves: AtomicU64,
    /// ★★★★★ **w613 — how many BAR1/BAR2 pages were already needed BEFORE the first channel
    /// was born.**
    ///
    /// ⊘ This decides whether the owner's map-at-create ruling can cover the traffic at all.
    /// `[measured w608]` 184 distinct pages account for 3 952 trapped accesses, and mapping
    /// them when a channel is created removes the traps — **but only for pages a channel
    /// knows about.** BAR2 is the INSTANCE window: RM writes instance blocks and page tables
    /// through it during the BAR2 bootstrap, which happens long before any channel exists.
    ///
    /// ⚠ If most of the 184 are pre-birth, map-at-create is the right ruling applied to the
    /// wrong half of the traffic, and I would have built it and measured no change. ⇒ Ask
    /// first. `(bar1, bar2)` distinct pages at the first birth; `u64::MAX` = no birth yet.
    pages_at_first_birth: Mutex<Option<(usize, usize)>>,
    /// ★ w617 — map-at-create: how many premap passes ran, how many pages they asked for, and
    /// how many passes the enumerator refused. ⊘ Three numbers because they fail differently:
    /// `runs=0` means the hook never fired, `refused>0` means BAR1 could not be enumerated, and
    /// `pages=0` with `runs>0` means it enumerated an empty tree — three causes, one symptom.
    premap_runs: AtomicU64,
    premap_pages: AtomicU64,
    premap_refused: AtomicU64,
    /// ★ w620 — the largest leaf any enumeration returned. ⊘ Printed because `4096` here means
    /// the 64 KiB case never arose and the fix below is untested by this boot, while `65536`
    /// means it did — a distinction the page count alone cannot make.
    premap_biggest_leaf: AtomicU64,
    /// ★ w622 — pages the snapshot filtered out before `fill_now` was called. ⊘ This is the
    /// number that used to be `ALREADY-COVERED-EARLY`, moved one layer up where it costs one
    /// `BTreeSet` lookup instead of a mutex acquisition.
    premap_skipped: AtomicU64,
    /// ★ w625 — the distinct refusal reasons seen, so 1 181 identical lines become one each.
    premap_why: Mutex<std::collections::BTreeSet<(bool, &'static str)>>,
    /// ★ w626 — the largest BAR2 page-table tree any enumeration walked, so the budget above
    /// can be set from this rather than from a guess. ⊘ `0` means BAR2 never enumerated at all.
    premap_bar2_visited: AtomicU64,
    /// ★★★★★ **CUT B — enumerations that came back SHORT** (`WindowEnumeration::faults > 0`)
    /// after every allowed arming retry. ⊘ A number that did not exist before cut B: the
    /// faults were dropped inside `window_leaves` and a short list was indistinguishable from
    /// a guest that had mapped less.
    premap_pt_faults: AtomicU64,
    /// ★★★★★ **CUT B — how many times a lock-free caller armed and tried again.** ⊘ The
    /// known-positive for the whole arming path: `0` here with `DEVICE-FB host_read_refused>0`
    /// means the retry never RAN, which is a different defect from a retry that ran and did
    /// not help.
    arm_retries: AtomicU64,
    /// Resolutions that succeeded only because of a retry — cut B working, at this caller.
    arm_retried_ok: AtomicU64,
    /// Resolutions still refused after the last allowed retry.
    arm_gave_up: AtomicU64,
    /// ★★★★★ **CUT C — fills asked for by an access that was REFUSED**, as opposed to one
    /// the archive served. ⊘ A number that could not exist before cut C, because a refused
    /// access asked for nothing at all: see [`BarMirror::fill_after_refusal`].
    fills_from_refusal: AtomicU64,
    /// ★★★ Cut C repairs **declined inside an MMIO trap on the non-deferring arm**, where
    /// [`BarMirror::fill`] would otherwise run `fill_now` — three blocking syscalls — on the
    /// vCPU. ⊘ Counted apart from `fills_dropped`: *"the queue was full"* and *"this caller
    /// may not do the work at all"* are different facts with different fixes, and a reader
    /// who saw one number could not tell which arm was talking.
    fills_refusal_declined: AtomicU64,
    /// BAR0's placement as last seen, for the transition count above.
    bar0_last: AtomicU64,
    table: Mutex<Table>,
    census: Census,
    /// The plane's `UPDATE_BAR_PDE` count at the last check — a moved count is a BAR2 root
    /// change and revalidates the BAR2 half.
    last_bar_pde_updates: AtomicU64,
    /// ★★★★★ **w468 — the deferred-revalidation counters.** The vCPU bumps `reval_req`
    /// and returns; the publication worker runs the walk and raises `reval_done` to the
    /// value it observed. A request is outstanding exactly while `req > done`.
    reval_req: AtomicU64,
    reval_done: AtomicU64,
    /// Which reason armed the outstanding request, for the log line only. Bit 0 =
    /// `mmu-invalidate`, bit 1 = `bar-pde-update`.
    reval_why: AtomicU64,
    /// Whether a publication worker exists to drain the deferral. See [`BarMirror::arm`].
    defer_reval: bool,
    /// ★★★★★ **w472 — the fill queue, SMALL AND LOSSY ON PURPOSE.**
    ///
    /// A fill is a **prefetch**: the plane has already served the access that missed, and
    /// installing the slot only stops the NEXT access to that page from exiting. ⇒ dropping
    /// one is always safe — the page simply keeps trapping until it is requested again. That
    /// is what lets this queue be small and lock-cheap instead of unbounded: the vCPU pushes
    /// under a tiny mutex held for a `push_back`, and never waits for anything.
    fills: Mutex<std::collections::VecDeque<(FbWindow, u64)>>,
    fills_queued: AtomicU64,
    fills_dropped: AtomicU64,
    fills_run: AtomicU64,
}

/// How many pending fills the queue holds before it starts dropping. ⊘ A prefetch queue
/// that grows without bound is a memory leak that pretends to be a cache.
const FILL_QUEUE_CAP: usize = 256;

fn idx(w: FbWindow) -> Option<usize> {
    match w {
        FbWindow::FbAperture => Some(0),
        FbWindow::InstanceWindow => Some(1),
        // ★★★★★ **w578 — PRAMIN IS DELIBERATELY NOT A MIRRORED WINDOW.**
        //
        // The other two are demand-filled page by page because they are GMMU-TRANSLATED: each
        // guest page resolves to an arbitrary framebuffer frame, so each needs its own slot.
        // PRAMIN is **untranslated** — the framebuffer address is the latch plus the offset —
        // so the whole 1 MiB aperture is one contiguous run of framebuffer addresses, and
        // since w569 that is one contiguous run of the arena's file.
        //
        // ⇒ ONE memory slot, re-pointed by ONE `mmap` when the latch moves
        // (`the_bar0_read_surface.md` §3b). Mirroring it would be 256 slots and 42 re-keys a
        // boot to do what one slot and 42 `mmap`s do. See `BarMirror::pramin`.
        FbWindow::Pramin => None,
    }
}

fn name(w: FbWindow) -> &'static str {
    match w {
        FbWindow::FbAperture => "bar1",
        FbWindow::InstanceWindow => "bar2",
        FbWindow::Pramin => "pramin",
    }
}

fn key_of(r: &WindowPageResolution) -> Option<Key> {
    let e = match r.backing {
        FbPageBacking::Joined(e) | FbPageBacking::Arena(e) => e,
        // ★★★★★ **§3.** The store named an ADDRESS in the reserved object, not a file this
        // process holds — see [`FbPageBacking::Device`] for why it cannot name a token. The
        // offset IS the framebuffer address, under the arena's own contract that the two
        // coincide, and `phys` beside it is the same number: kept as two fields anyway,
        // because `Key` equality is what revalidation compares and a key that derived one
        // from the other could not notice the day they stop agreeing.
        FbPageBacking::Device { at } => FbPageExport {
            token: DEVICE_TOKEN,
            offset: at,
        },
        FbPageBacking::Heap | FbPageBacking::Refused(_) => return None,
    };
    Some(Key {
        phys: r.phys,
        token: e.token,
        offset: e.offset,
        readonly: r.read_only,
    })
}

/// ★★★★★ **CUT B — THE BOUNDED ARM-THEN-RETRY, AS A PURE FUNCTION.**
///
/// `attempt` produces a value and says whether it is **good**; `arm` says whether it changed
/// anything. The loop runs `attempt` once, then at most `retries` more times, and only while
/// `arm` returns `true`. Returns the last value and how many retries were spent.
///
/// # ⊘⊘⊘ Why this is a free function and not two loops at the two call sites
///
/// The two callers — `resolve_arming` and `premap_window` — need the same three properties and
/// **cannot be built in a `cargo test`**: a `BarMirror` needs a `QemuMachine`. A loop that can
/// only be checked by booting is a loop nobody checks. ⇒ the part that can be wrong on its own
/// lives here, where the inline tests below fire each property:
///
/// 1. **A FIXED TRIP COUNT** (`THE_CONSTRAINTS.md` §20 invariant 1). `attempt`'s input is a
///    guest-authored page-table pointer; a loop that ended when the walk succeeded would let
///    the guest's own tables choose how long this thread runs.
/// 2. **`arm` returning false ENDS IT.** *"Nothing was armed"* covers *"the drain declined"*
///    (a vCPU), *"the aperture refused"* and *"there was no port"*, and retrying helps in none
///    of them — ⚠ and the first is a **vCPU inside an MMIO exit**, which is the thread that
///    must not spin.
/// 3. **The value comes back either way.** A caller that got only `Err` on give-up could not
///    report what it last saw, and `window_leaves`' failing shape is an `Ok` that is SHORT.
fn arm_then_retry<T>(
    retries: u32,
    mut attempt: impl FnMut() -> (T, bool),
    mut arm: impl FnMut() -> bool,
) -> (T, u32) {
    let (mut value, mut good) = attempt();
    let mut used = 0u32;
    // ⊘ `for`, never `while !good`: the bound is the loop's own, not the data's.
    for _ in 0..retries {
        if good || !arm() {
            break;
        }
        used += 1;
        let (v, g) = attempt();
        value = v;
        good = g;
    }
    (value, used)
}

fn refusal_name(e: &VmmError) -> &'static str {
    match e {
        VmmError::Unsupported(s) => s,
        VmmError::HostRefused { what, .. } => what,
        _ => R_INSTALL,
    }
}

impl BarMirror {
    /// Build the mirror for whichever of BAR1/BAR2 the hypervisor declares unbacked (the
    /// QOM `bar1-passthrough` / `bar2-passthrough` arms). `None` — printed by name — when
    /// neither is, or when the arena could not be created.
    ///
    /// ⊘ Lock-free context required (an `mmap` and a `memfd_create`): the composition
    /// root's `attach_ram`, on the hypervisor's own thread.
    /// `defer_reval` is the shim's `DoorbellAsyncArm::defers()`. ⊘ It is not a preference:
    /// the deferred walk is drained by the publication worker, and on the `off` control
    /// there is no worker, so deferring there would leave every stale slot live forever.
    /// The control keeps the pre-w468 inline walk, which is what `off` means.
    pub fn arm(
        plane: Arc<RegPlane>,
        machine: QemuMachine,
        defer_reval: bool,
        device_port: Option<Arc<crate::deviceview::DeviceViewPort>>,
    ) -> Option<Arc<BarMirror>> {
        let mut arms = [None, None];
        for (i, bar, w) in [(0usize, BarId::Bar1, "bar1"), (1, BarId::Bar2, "bar2")] {
            let unbacked = machine.bar_is_unbacked_reservation(bar);
            let placement = machine.bar_placement(bar);
            match (unbacked, placement) {
                (true, Some(p)) => {
                    arms[i] = Some(Arm {
                        base: p.base,
                        len: p.len,
                    });
                    eprintln!(
                        "kayfabe: BAR-MIRROR {w}: ARMED — the QOM row answers unbacked, so \
                         this archive may shadow sub-ranges of it; base=0x{:x} len=0x{:x}. \
                         Every trapped access to it is a FILL (one memslot over the store's \
                         own page for that frame); every later access to that page takes no \
                         VM exit.",
                        p.base, p.len
                    );
                }
                (true, None) => eprintln!(
                    "kayfabe: BAR-MIRROR {w}: ⊘ NOT ARMED — the QOM row answers unbacked but \
                     the machine has no placement for it"
                ),
                (false, _) => eprintln!(
                    "kayfabe: BAR-MIRROR {w}: OFF (the control) — the QOM row traps; every \
                     access to it takes a VM exit and is served by the archive, byte for byte \
                     as before w393"
                ),
            }
        }
        if arms.iter().all(Option::is_none) {
            return None;
        }
        // ★★★★★ **§3 — THE `device` ARM'S PRECONDITIONS, CHECKED HERE TOO.**
        //
        // ⊘ `defer_reval` is only knowable at this point — the mirror is built at
        // `attach_ram` — so the half of the rule that depends on it is enforced here, with
        // the SAME function the realize-time half uses. Two moments, one statement of the
        // rule: a second spelling would be a second chance for the two to disagree.
        let store_arm = match crate::deviceview::selected_fb_store() {
            Ok(a) => a,
            Err(why) => {
                eprintln!("kayfabe: BAR-MIRROR ⊘⊘⊘ NOT ARMED — {why}");
                return None;
            }
        };
        if let Err(why) =
            crate::deviceview::enforce_device_store(store_arm, device_port.is_some(), defer_reval)
        {
            eprintln!("kayfabe: BAR-MIRROR ⊘⊘⊘ NOT ARMED — {why}");
            return None;
        }
        // ★ w578 — sized to THIS chip's framebuffer, because since w569 a page's file offset
        // is its framebuffer ADDRESS. A sparse memfd makes the extent free; residency is still
        // bounded by the store's own ceiling, which is where that limit belongs.
        let arena = match SharedPageArena::create_for(
            plane.chip().fb_length,
            HostPageSize::query(),
        ) {
            Ok(a) => a,
            Err(e) => {
                eprintln!(
                    "kayfabe: BAR-MIRROR ⊘⊘ NOT ARMED — the page arena could not be created \
                     ({e:?}); the store keeps heap pages and every access traps"
                );
                return None;
            }
        };
        if store_arm.is_device() {
            eprintln!(
                "kayfabe: BAR-MIRROR §3: the page arena is NOT installed — the single store \
                 holds no pages of ours, so there is nothing to arena. Every framebuffer page \
                 names an address in the reserved object and this mirror arms a CPU view per \
                 memslot. ⊘ This function used to read the store's refusal of an arena as \
                 \"every access traps\" and return None, which would leave the device arm with \
                 NO MIRROR AT ALL."
            );
        } else if plane
            .install_fb_page_arena(Box::new(ArenaPort(arena.clone())))
            .is_err()
        {
            eprintln!(
                "kayfabe: BAR-MIRROR ⊘⊘ NOT ARMED — the store refused the page arena; every \
                 access traps"
            );
            return None;
        }
        let m = Arc::new(BarMirror {
            last_bar_pde_updates: AtomicU64::new(plane.bar_pde_counts().0),
            reval_req: AtomicU64::new(0),
            reval_done: AtomicU64::new(0),
            reval_why: AtomicU64::new(0),
            defer_reval,
            device_port,
            device_store: store_arm.is_device(),
            parked: Mutex::new(Vec::new()),
            fills: Mutex::new(std::collections::VecDeque::new()),
            fills_queued: AtomicU64::new(0),
            fills_dropped: AtomicU64::new(0),
            fills_run: AtomicU64::new(0),
            plane,
            machine,
            arena,
            arms,
            pramin: Mutex::new(None),
            pramin_moves: AtomicU64::new(0),
            pramin_move_ns_total: AtomicU64::new(0),
            pramin_move_ns_worst: AtomicU64::new(0),
            pramin_at_install: AtomicU64::new(u64::MAX),
            pramin_gpa: AtomicU64::new(u64::MAX),
            pramin_marks: Mutex::new(Vec::new()),
            counter_slot: Mutex::new(None),
            bar0_moves: AtomicU64::new(0),
            pages_at_first_birth: Mutex::new(None),
            premap_runs: AtomicU64::new(0),
            premap_pages: AtomicU64::new(0),
            premap_refused: AtomicU64::new(0),
            premap_biggest_leaf: AtomicU64::new(0),
            premap_skipped: AtomicU64::new(0),
            premap_why: Mutex::new(std::collections::BTreeSet::new()),
            premap_pt_faults: AtomicU64::new(0),
            arm_retries: AtomicU64::new(0),
            arm_retried_ok: AtomicU64::new(0),
            arm_gave_up: AtomicU64::new(0),
            fills_from_refusal: AtomicU64::new(0),
            fills_refusal_declined: AtomicU64::new(0),
            premap_bar2_visited: AtomicU64::new(0),
            bar0_last: AtomicU64::new(u64::MAX),
            pramin_skipped: AtomicU64::new(0),
            table: Mutex::new(Table::default()),
            census: Census::default(),
        });
        let (floor, ceiling) = m.machine.slot_range();
        eprintln!(
            "kayfabe: BAR-MIRROR armed: page arena {} MiB (sparse memfd, one mapping), slot \
             numbers {floor}..{ceiling} ({} available to this device). ⊘ A fill is ONE trap \
             per page; a revalidation runs on every guest MMU_INVALIDATE trigger and on \
             every BAR2 root publication.",
            SharedPageArena::LEN >> 20,
            ceiling - floor,
        );
        Some(m)
    }

    fn arm_for(&self, w: FbWindow) -> Option<Arm> {
        self.arms[idx(w)?]
    }

    /// Whether this window is armed at all — the cheap check the trap path makes.
    #[must_use]
    pub fn is_armed(&self, w: FbWindow) -> bool {
        self.arm_for(w).is_some()
    }

    fn refuse(&self, w: FbWindow, off: u64, why: &'static str, detail: &str) {
        let n = {
            let mut t = self.table.lock().unwrap_or_else(|e| e.into_inner());
            let c = t.refused.entry(why).or_insert(0);
            *c += 1;
            *c
        };
        if n <= REFUSAL_LIVE {
            eprintln!(
                "kayfabe: BAR-MIRROR {} ⊘ FILL REFUSED [{why}] #{n} at aperture +0x{off:x}: \
                 {detail} (printed {n} of {REFUSAL_LIVE}; the total is in the census)",
                name(w)
            );
        }
    }

    fn quiesced_covers(t: &Table, phys: u64) -> bool {
        t.quiesced
            .iter()
            .any(|(p, l)| phys >= *p && phys < p.saturating_add(*l))
    }

    /// ★★★★★ **THE FILL** — called after a trapped BAR1/BAR2 access the archive served.
    ///
    /// Three phases, and the plane lock is held only inside the first and the third:
    /// 1. resolve the page (walk + backing, materialising the store's page if needed);
    /// 2. reserve a ticket in the table, resolve the descriptor, install the memslot —
    ///    lock-free;
    /// 3. re-resolve, and commit the slot only if nothing moved and the ticket survived.
    /// ★★★★★ **w472 — THE FRONT DOOR. On a vCPU this only enqueues.**
    ///
    /// `[measured w471]` the three syscalls one fill makes — `mmap` to create the window,
    /// `mmap MAP_FIXED` to place the backing, `KVM_SET_USER_MEMORY_REGION` to install the
    /// slot — are three of the five blocking doors a vCPU thread reached, out of 1239
    /// reaches in one boot. None of them may run inside an MMIO exit.
    ///
    /// ⊘ Deferring is safe *because a fill is a prefetch*: the plane already served the
    /// access that missed, and `fill` only stops the NEXT access to that page from exiting.
    /// Nothing the guest can observe depends on when it happens, or on whether it happens
    /// at all — which is also why a full queue DROPS rather than blocks.
    /// Returns **whether a worker wake is owed** — `true` only when this call actually queued
    /// a fill for the worker to drain.
    ///
    /// # ⊘⊘⊘ THE WAKE WAS UNCONDITIONAL, AND IT WAS 93 % OF THE WORKER'S WORK (w674)
    ///
    /// `[measured w674a, one `cuDeviceGet` that took 20 s]`
    /// `PUBQUEUE by_kind[doorbell=17 mirror_fill=11414 invalidate=306 gsp_submit=420
    /// rpc_bind=105]` — **11 414 of 12 262 queue jobs were mirror-fill wakes** — beside this
    /// type's own census on the same boot: `BAR-MIRROR FILLS queued=0 run=0 dropped=0`.
    ///
    /// ★ Both numbers are right, and together they name the defect. This function has two
    /// paths: off the deferred arm it calls `fill_now` **synchronously and queues nothing**;
    /// only the deferred arm pushes. `queued=0` proves the synchronous path was always taken —
    /// so every one of those 11 414 wakes told the worker to drain a queue **nothing had been
    /// put into**, and each wake costs a full worker pass (dbtable rebuild, birth drain,
    /// publish, mirror drain). ⇒ ~93 % of the passes behind that 20 s were for no work at all.
    ///
    /// ⚠ It survived because the two are separate censuses that were never read side by side,
    /// and because `MirrorFill` is also used as a generic "wake the worker" token elsewhere —
    /// so a large count looked like the lane being busy rather than the lane spinning.
    ///
    /// ⊘ `#[must_use]`: an ignored return puts the unconditional wake straight back, and the
    /// only symptom would be the worker being busy — which is what this looked like for a
    /// whole session.
    #[must_use = "the caller must wake the worker IF AND ONLY IF this returns true; ignoring                   it restores the unconditional wake that cost 93% of the worker's passes"]
    pub fn fill(&self, w: FbWindow, off: u64) -> bool {
        if !self.defer_reval || !kayfabe_util::lockwitness::on_vcpu_thread() {
            // ⊘ Done HERE, synchronously. Nothing is queued, so no wake is owed.
            self.fill_now(w, off, FillOrigin::Trap);
            return false;
        }
        let mut q = self.fills.lock().unwrap_or_else(|e| e.into_inner());
        if q.len() >= FILL_QUEUE_CAP {
            // ⊘ Dropped, not queued — and a dropped fill is a page that keeps trapping, never
            // a wrong value. No work is pending, so no wake is owed.
            self.fills_dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        q.push_back((w, off));
        self.fills_queued.fetch_add(1, Ordering::Relaxed);
        true
    }

    /// ★★★★★ **CUT C — THE REPAIR PATH, ASKED FOR BY THE ACCESS THAT FAILED.**
    ///
    /// # ⊘⊘⊘ THE DEFECT THIS EXISTS FOR — the recovery was gated on the success it repairs
    ///
    /// `[established from the source and confirmed by the w738 boot, w739]` both shell call
    /// sites of [`BarMirror::fill`] are gated on the access having **worked**:
    /// `Regs::read` fills only on `ReadOutcome::Fb` (*the archive served it*) and
    /// `Regs::write` only on `out.fb_landed.is_some()` (*the store took the bytes*).
    ///
    /// ⇒ Under the arena store that is harmless, because a trapped access to a mapped page
    /// always succeeds and the fill is a pure prefetch. **Under the single store it is a
    /// deadlock**: the FIRST access to any BAR1/BAR2 page is refused — no memslot covers it
    /// and no CPU view of its page tables is armed — a refused access queues nothing, so
    /// `fill_now` never runs, so `resolve_arming` never arms, so the page is never covered,
    /// so the next access is refused for the same reason. `[measured w738]`
    /// `BAR-MIRROR FILLS queued=0 run=0 dropped=0` beside `BAR2 (translated): … 14 REFUSED
    /// by name` and `named=0`: fourteen refusals, not one repair attempted.
    ///
    /// ★★★ This is the tree's named class one turn further on — not *"a diagnostic gated on
    /// the drop it hunts"* but **a repair gated on the failure it repairs**, and the symptom
    /// is a counter reading `0` for *"nothing needed fixing"* on a boot where everything did.
    ///
    /// # ⚠ WHY IT IS GATED ON THE BYTE PORT AND NOT ARMED UNCONDITIONALLY
    ///
    /// On the arena arm a refusal means the guest's own tables do not map that offset, and
    /// re-walking it on a worker would resolve to the same refusal, every time, for as long
    /// as the guest keeps poking it — real work, queued forever, to discover a fact that has
    /// not changed. ⊘ So the caller asks [`kayfabe_device::plane::RegPlane::fb_has_demand_port`]
    /// **first**, which is `false` on the arena arm and makes this unreachable there: the
    /// default arm is byte-identical by construction rather than by inspection, and
    /// `two_worlds_split::the_refused_access_repair_gate_is_false_on_the_arena_arm_and_true_on_the_device_arm`
    /// pins the predicate on both arms.
    ///
    /// ⊘ **It does not recover the bytes of the write that was refused.** Those are gone —
    /// the guest was not told and cannot be. What this buys is that the NEXT access to that
    /// page can land, which is the difference between a boot that repairs itself and one that
    /// refuses the same page until it dies.
    ///
    /// Returns whether a worker wake is owed, exactly as [`BarMirror::fill`] does.
    #[must_use = "the caller must wake the worker IF AND ONLY IF this returns true"]
    pub fn fill_after_refusal(&self, w: FbWindow, off: u64) -> bool {
        self.fills_from_refusal.fetch_add(1, Ordering::Relaxed);
        // ⊘⊘⊘ **NEVER INLINE FROM AN MMIO EXIT.** [`BarMirror::fill`] falls through to
        // `fill_now` **synchronously** when the deferring arm is off — and `fill_now` makes
        // the three blocking syscalls w471 measured on a vCPU (`mmap`, `mmap MAP_FIXED`,
        // `KVM_SET_USER_MEMORY_REGION`). This caller is a vCPU inside an MMIO exit **by
        // construction**: it is the refusal of that very access.
        //
        // ⇒ constraint 4 (*"all traps sub-millisecond"*) and constraint 6 (*"every MMIO trap
        // only posts to a queue"*) both forbid it, and no number of microseconds changes
        // that. ★ The repair is a **prefetch**: declining costs the next access to this page
        // and never a wrong value — the same contract the full-queue drop already has.
        //
        // ⚠ The deferring arm (the shipped one) does not reach this: `fill` queues there.
        if kayfabe_util::lockwitness::on_vcpu_thread() || kayfabe_util::trapwitness::in_trap() {
            if !self.defer_reval {
                self.fills_refusal_declined.fetch_add(1, Ordering::Relaxed);
                return false;
            }
        }
        self.fill(w, off)
    }


    /// ★★★★★ **w472 — the worker's half.** Runs every queued fill. ⊘ Never call from a vCPU.
    pub fn drain_fills(&self) {
        loop {
            let Some((w, off)) = ({
                let mut q = self.fills.lock().unwrap_or_else(|e| e.into_inner());
                q.pop_front()
            }) else {
                return;
            };
            self.fills_run.fetch_add(1, Ordering::Relaxed);
            self.fill_now(w, off, FillOrigin::Trap);
        }
    }

    /// The census for the fill queue, one line.
    #[must_use]
    pub fn fill_census(&self) -> String {
        let from_refusal = self.fills_from_refusal.load(Ordering::Relaxed);
        let refusal_declined = self.fills_refusal_declined.load(Ordering::Relaxed);
        format!(
            "BAR-MIRROR FILLS queued={} run={} dropped={} from_refusal={from_refusal} \
             refusal_declined={refusal_declined} (a \
             dropped fill is a page that keeps trapping, never a wrong value){}",
            self.fills_queued.load(Ordering::Relaxed),
            self.fills_run.load(Ordering::Relaxed),
            self.fills_dropped.load(Ordering::Relaxed),
            if from_refusal == 0 {
                " ⊘ from_refusal=0 — cut C's repair path was NEVER ASKED. On the arena arm \
                 that is correct and expected (no byte port, the gate is false by \
                 construction); on the `device` arm it means a refused BAR1/BAR2 access \
                 never reached the gate, which is a DIFFERENT defect from one that reached \
                 it and could not fix the page."
            } else {
                " ★ from_refusal>0 — a refused access asked for its own repair, which before \
                 cut C nothing did."
            }
        )
    }

    /// ★★★★★ **CUT B item 3 — THE RETRY BOUND, AND IT IS THE TREE'S DEPTH.**
    ///
    /// A point walk (`bar1_translate`) reads **one page-table page per level**, and each
    /// attempt faults at the shallowest page it cannot read. So one attempt per level is all
    /// that can ever be needed, and `kayfabe_mmu::walker::MAX_WALK_DEPTH` is the format-bounded
    /// cap on levels — the same bound §20's first invariant uses, and for the same reason: a
    /// **fixed trip count**, never a loop that ends when the walk succeeds.
    ///
    /// ⊘ It is deliberately not *"retry until nothing is armed"*: that phrasing makes the
    /// guest's own tables choose how long this thread runs.
    const ARM_RETRIES: u32 = kayfabe_mmu::walker::MAX_WALK_DEPTH as u32;

    /// ★★★★★ **CUT B item 3 — RESOLVE, ARMING WHAT THE STORE COULD NOT READ.**
    ///
    /// `SINGLE_STORE_PLAN.md` cut B item 3: *"`bar1_translate` / `bar2_translate` /
    /// `window_leaves` run under the lock; the frame they missed survives only in the store's
    /// `want` set … The retry belongs at `fill_now`'s and `premap`'s **entry**, both
    /// lock-free, in a bounded loop."*
    ///
    /// ⚠ **This is the lock-free half and it must stay that way.** `window_page_backing` takes
    /// and releases the plane's locks inside itself; `arm_fb_demand` is called with nothing
    /// held, and declines by name if this thread is a vCPU.
    fn resolve_arming(
        &self,
        w: FbWindow,
        page_off: u64,
    ) -> Result<kayfabe_device::WindowPageResolution, kayfabe_device::WindowRefusal> {
        let (out, used) = arm_then_retry(
            Self::ARM_RETRIES,
            || {
                // ⊘ `window_page_backing` takes and releases BOTH plane guards inside itself,
                // so nothing is held when `arm` runs below.
                let r = self.plane.window_page_backing(w, page_off, true);
                let ok = r.is_ok();
                (r, ok)
            },
            // ★ LOCK-FREE HERE, and it has to be: this is an IPC round trip to the scratchpad
            // isolate, and it declines by name if this thread is a vCPU.
            || self.plane.arm_fb_demand().progressed(),
        );
        self.arm_retries.fetch_add(u64::from(used), Ordering::Relaxed);
        if used > 0 {
            if out.is_ok() {
                self.arm_retried_ok.fetch_add(1, Ordering::Relaxed);
            } else {
                self.arm_gave_up.fetch_add(1, Ordering::Relaxed);
            }
        }
        out
    }

    fn fill_now(&self, w: FbWindow, off: u64, origin: FillOrigin) {
        let Some(arm) = self.arm_for(w) else {
            return;
        };
        let Some(wi) = idx(w) else {
            return;
        };
        let page_off = off & !(PAGE - 1);
        if page_off >= arm.len {
            self.refuse(w, off, R_OUT_OF_BAR, "the offset is past the BAR's own length");
            return;
        }
        let gpa = arm.base + page_off;

        // ---- 0. ALREADY COVERED? ★★★★★ **ASKED FIRST, w611.**
        //
        // ⊘⊘ This question used to be asked in phase 2, **after** phase 1 had taken the PLANE
        // LOCK, walked the address model and MATERIALISED a store page (`window_page_backing`'s
        // `true`). `[measured w603a, one boot]` **3 572 of 3 952 fills — 90.4 % — end here**,
        // so that ordering spent 3 572 plane-lock acquisitions and address-model walks per boot
        // to discover work that was already done.
        //
        // ⚠ The plane lock is the one the vCPU also wants; `[w516]` and `[w522]` are both about
        // exactly this lock, and the second measured every trap's wait against it. A worker
        // taking it 3 572 times for nothing is not free merely because it is off the vCPU.
        //
        // ★ Hoisting is SOUND, not just cheaper, and the reason is architectural: a slot that
        // exists but has gone stale is the REVALIDATOR's job (`revalidate[runs=… removed=…]`),
        // never the fill path's. The fill path asks *"is this page covered"*, and the answer
        // does not depend on anything phase 1 computes. ⇒ The old order asked a question whose
        // answer it already had, using the most expensive lock in the device to not find out.
        //
        // ⊘ Counted separately from the late arm so the hoist is falsifiable: `COVERED-EARLY`
        // should absorb essentially all of `ALREADY-COVERED`, and if the late arm stays large
        // the races are arriving between here and phase 2 and the hoist bought nothing.
        {
            let t = self.table.lock().unwrap_or_else(|e| e.into_inner());
            if t.slots.contains_key(&gpa) {
                drop(t);
                self.refuse(
                    w,
                    off,
                    R_COVERED_EARLY,
                    "a slot already covers this page; answered before taking the plane lock",
                );
                return;
            }
        }

        // ---- 1. RESOLVE (plane lock, released on return) --------------------------------
        // ★★★★★ **CUT B item 3 — and it is `resolve_arming`, not `window_page_backing`.** Under
        // the single store the walk this performs reads the guest's BAR page tables out of the
        // reserved object, where a page with no armed CPU view refuses **by name**. Arming is
        // lock-free and this is a lock-free caller; see `resolve_arming`.
        let res = match self.resolve_arming(w, page_off) {
            Ok(r) => r,
            Err(kayfabe_device::WindowRefusal::NoAddressModel) => {
                self.refuse(w, off, R_NO_ADDRESS_MODEL, "the window has no address model");
                return;
            }
            Err(kayfabe_device::WindowRefusal::Translated { why, .. }) => {
                self.refuse(w, off, R_TRANSLATION, why);
                return;
            }
        };
        let Some(key) = key_of(&res) else {
            match res.backing {
                FbPageBacking::Heap => self.refuse(
                    w,
                    off,
                    R_HEAP_PAGE,
                    "the store holds this frame on the heap and could not move it to the arena",
                ),
                FbPageBacking::Refused(why) => self.refuse(w, off, R_STORE, why),
                // ⊘ Unreachable by `key_of`'s own shape — it answers `Some` for exactly these
                // two — and spelled out anyway, because the `_` that used to stand here is the
                // reason a new arm could be added and wired nowhere. Refused **by name**
                // rather than `unreachable!()`: this runs on a guest MMIO exit, and a panic
                // here is a guest-reachable abort of the VMM for a contradiction that costs
                // one un-mirrored page.
                FbPageBacking::Joined(_)
                | FbPageBacking::Arena(_)
                | FbPageBacking::Device { .. } => self.refuse(
                    w,
                    off,
                    R_UNKEYED,
                    "the store named a memslottable backing and `key_of` refused to key it \u{2014}                      the two matches on `FbPageBacking` disagree, which is a defect in this                      file and not in the store",
                ),
            }
            return;
        };

        // ---- 2. RESERVE + INSTALL (no ranked lock) ---------------------------------------
        let (ticket, epoch) = {
            let mut t = self.table.lock().unwrap_or_else(|e| e.into_inner());
            if t.slots.contains_key(&gpa) {
                drop(t);
                self.refuse(
                    w,
                    off,
                    R_COVERED,
                    "a slot already covers this page — the access raced its install",
                );
                return;
            }
            if Self::quiesced_covers(&t, key.phys) {
                drop(t);
                self.refuse(
                    w,
                    off,
                    R_QUIESCED,
                    "the plane is moving this frame's bytes (a join install or release)",
                );
                return;
            }
            if t.pending.contains_key(&gpa) {
                drop(t);
                self.refuse(w, off, R_PENDING, "another vCPU is filling this page");
                return;
            }
            t.next_ticket += 1;
            let ticket = t.next_ticket;
            t.pending.insert(gpa, (ticket, key.phys));
            (ticket, t.reval_epoch)
        };
        // ★★★★★ **§3 — THE THIRD BACKING, AND IT IS THE ONLY ONE THAT COSTS HOST BAR1.**
        //
        // ⊘ Arena and join slots name a file this process already holds: the `dup` is local,
        // free, and nothing has to be given back. A device page is different in every one of
        // those respects — the node is armed by an **IPC round trip** to the isolate that
        // owns the reserved object, it consumes host BAR1 aperture, and the aperture comes
        // back only through `NV_ESC_RM_UNMAP_MEMORY`.
        //
        // ⚠ **This is the lock-free step, and it has to be.** `Worker::export_device_view`
        // asserts `assert_lock_free` on arrival; `SINGLE_STORE_PLAN.md` §3's structural fact 2
        // is that `page_backing`/`read`/`write` cannot arm, which is why the store handed us
        // an ADDRESS rather than a token. Phase 1's plane lock was taken and released above;
        // nothing is held here.
        let (region, view) = if key.token == DEVICE_TOKEN {
            // ★★★★★ **THE vCPU GUARD, AND IT IS NOT BELT-AND-BRACES.**
            //
            // ⊘⊘ `assert_lock_free` inside `Worker::export_device_view` asks *"what does this
            // thread HOLD"*, not *"where am I"* — and its `assert_not_on_vcpu` half only
            // **reports** unless `KAYFABE_VCPU_BLOCK_FATAL` is set. ⇒ an IPC round trip
            // reached from a vCPU here would simply block one inside an MMIO exit, silently,
            // and be discovered later as latency with nothing pointing at the cause.
            //
            // ★ `enforce_device_store` refuses the one configuration that routes here on a
            // vCPU (`defer_reval` off) at startup, so this arm should be unreachable. It is
            // spelled out anyway and **refuses by name** rather than asserting: a second route
            // onto this path is a change somebody will make, and the cost of catching it here
            // is one un-mirrored page against a blocked vCPU.
            if kayfabe_util::lockwitness::on_vcpu_thread() {
                self.cancel(gpa, ticket);
                self.refuse(
                    w,
                    off,
                    R_ON_VCPU,
                    "arming a device view is an IPC round trip to the scratchpad isolate and \
                     this is a vCPU inside an MMIO exit; refused rather than blocking it",
                );
                return;
            }
            let Some(port) = self.device_port.as_ref() else {
                self.cancel(gpa, ticket);
                self.refuse(
                    w,
                    off,
                    R_NO_DEVICE_PORT,
                    "the store named a page of the reserved object and there is no \
                     device-view port to arm it through — the two gates disagree",
                );
                return;
            };
            match port.with_node(key.offset, PAGE, true, |fd, mmap_len| {
                // ⊘ `install_device_page`, NOT `install_device_window`: that verb's `native`
                // argument is the READ-ONLY sub-range, and a framebuffer page needs a
                // WRITABLE slot — a read-only one would trap every guest store, which reads
                // as "the switch landed and performance collapsed" rather than as the wrong
                // verb. See its doc.
                //
                // ⚠ `mmap_len` is the DRIVER's page-rounded length, which is what the node
                // will accept; `PAGE` is what we asked for. They agree on a 4 KiB host page
                // and the driver's number is the one that must be mapped.
                self.machine
                    .install_device_page(gpa, mmap_len, fd, key.readonly)
            }) {
                Ok(Ok((r, id))) => (r, Some(id.0)),
                Ok(Err(e)) => {
                    // ⊘ The view was already released by the port — the caller could not use
                    // the mapping, so the aperture went straight back.
                    self.cancel(gpa, ticket);
                    self.refuse(w, off, refusal_name(&e), &format!("{e:?}"));
                    return;
                }
                Err(v) => {
                    self.cancel(gpa, ticket);
                    self.refuse(w, off, R_VIEW_REFUSED, v.name());
                    return;
                }
            }
        } else {
            let join_fd;
            let fd = if key.token == ARENA_TOKEN {
                self.arena.as_backing_fd()
            } else {
                match JoinRegistry::global().get(key.token) {
                    Some(f) => {
                        join_fd = f;
                        join_fd.as_fd()
                    }
                    None => {
                        self.cancel(gpa, ticket);
                        self.refuse(
                            w,
                            off,
                            R_JOIN_GONE,
                            "the join's descriptor left the registry between resolve and \
                             install",
                        );
                        return;
                    }
                }
            };
            match self
                .machine
                .install_file_window(gpa, PAGE, fd, key.offset, key.readonly)
            {
                Ok(r) => (r, None),
                Err(e) => {
                    self.cancel(gpa, ticket);
                    self.refuse(w, off, refusal_name(&e), &format!("{e:?}"));
                    return;
                }
            }
        };

        // ---- 3. RE-RESOLVE + COMMIT ------------------------------------------------------
        let still = self
            .plane
            .window_page_backing(w, page_off, false)
            .ok()
            .and_then(|r| key_of(&r))
            .is_some_and(|k| k == key);
        let kept = {
            let mut t = self.table.lock().unwrap_or_else(|e| e.into_inner());
            let mine = t.pending.remove(&gpa).is_some_and(|(tk, _)| tk == ticket);
            if mine && still && !Self::quiesced_covers(&t, key.phys) && t.reval_epoch == epoch {
                t.slots.insert(
                    gpa,
                    Slot {
                        region,
                        window: w,
                        page_off,
                        key,
                        view,
                    },
                );
                t.live += 1;
                t.peak = t.peak.max(t.live);
                t.pages_ever[wi].insert(gpa);
                t.frames_ever[wi].insert(key.phys);
                true
            } else {
                false
            }
        };
        if !kept {
            // ⊘ Through `retire`, not a bare `remove_window`: a device page's view must be
            // parked and released only once its mapping is actually gone, and a second
            // spelling of that sequence is a second chance to get it wrong.
            let _ = self.retire(vec![Retired {
                gpa,
                region,
                view,
            }]);
            self.refuse(
                w,
                off,
                R_RACED,
                "the page's translation or backing moved while the slot was being installed; \
                 the slot was dropped before it could serve anything",
            );
            return;
        }
        // ★★★★★ **TRAP-DRIVEN ONLY — w696.**
        //
        // ⊘⊘⊘ This counter used to be bumped by BOTH callers, while the module doc 960 lines up
        // said *"every fill was ONE trapped access"*. That sentence was TRUE before `premap`
        // existed and premap broke it without touching it. `[measured w695m/w696ctl]` the
        // identity is exact and damning: bar1 1728 + bar2 211 = **1939** = `premap[filled=1939]`
        // (CUDA), and 5841 + 294 = **6135** = `premap[filled=6135]` (raw client). ⇒ EVERY fill
        // in both workloads was a premap install and the trap count was **zero** — but the
        // number read as thousands of traps, and it was reported as a goal-2 regression twice.
        //
        // ★ Goal 2 asks "how many exits did the guest take", and only this arm answers it.
        // Premap installs are counted by `premap_pages`, which already exists and is honest.
        let n = match origin {
            FillOrigin::Trap => self.census.fills[wi].fetch_add(1, Ordering::Relaxed) + 1,
            FillOrigin::Premap => {
                self.census.premap_fills[wi].fetch_add(1, Ordering::Relaxed) + 1
            }
        };
        if n % CENSUS_EVERY_FILLS == 0 {
            self.report("RUNNING");
        }
    }

    fn cancel(&self, gpa: u64, ticket: u64) {
        let mut t = self.table.lock().unwrap_or_else(|e| e.into_inner());
        if t.pending.get(&gpa).is_some_and(|(tk, _)| *tk == ticket) {
            t.pending.remove(&gpa);
        }
    }

    /// Remove `gone` from the machine, lock-free, and account for them.
    ///
    /// # ★★★★★ §3 — REMOVING THE WINDOW IS NOT RELEASING THE VIEW, AND THE GAP IS DANGEROUS
    ///
    /// `QemuMachine::remove_window` clears the memslots and **parks** the mapping: an accessor
    /// on another thread may still hold a clone of the `Arc` and be reading through it, so the
    /// `munmap` happens later. For an arena or join slot that deferral is invisible.
    ///
    /// For a **device view** it is not. RM's `osUnmapPciMemoryUser` is an empty function
    /// (`ogkm os.c:1275-1282`), so `NV_ESC_RM_UNMAP_MEMORY` returns the host BAR1 aperture to
    /// the pool **without touching the VMA**. Releasing while a mapping is still live leaves
    /// PTEs pointing at BAR1 space RM has already handed to the next mapping — ours, the
    /// host's own CUDA context, or another VM's. Silent, and cross-tenant.
    ///
    /// ⇒ the view is **parked** here and released only once
    /// `QemuMachine::reclaim_released_windows` names its region. [`Self::drain_view_releases`]
    /// is what does it, and it is called from lock-free points only.
    fn retire(&self, gone: Vec<Retired>) -> u64 {
        let mut n = 0;
        for r in gone {
            if self.machine.remove_window(r.region).is_ok() {
                n += 1;
            }
            if let Some(v) = r.view {
                self.parked
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push((r.region, v));
            }
        }
        self.drain_view_releases();
        n
    }

    /// ★★★★★ **§3 — GIVE BACK THE APERTURE OF EVERY VIEW WHOSE MAPPING IS NOW GONE.**
    ///
    /// ⊘ Must be lock-free and off-trap: releasing is an IPC round trip to the isolate that
    /// asserts exactly that. Called from [`Self::retire`] and from the reclaim tick.
    ///
    /// ⚠ A parked view whose region never gets collected stays parked, and that is the safe
    /// direction: a leaked aperture refuses later arms loudly, while an early release
    /// corrupts another tenant's mapping silently. The census prints `parked=` so the leak is
    /// visible rather than inferred.
    pub fn drain_view_releases(&self) {
        let Some(port) = self.device_port.as_ref() else {
            return;
        };
        // ⊘ **The emptiness check comes FIRST, and that is not a micro-optimisation.** The
        // device-view port can be armed on the `arena` arm too (`KAYFABE_DEVICE_VIEW=probe`
        // with the default store), and nothing is ever parked there. Calling
        // `reclaim_released_windows` regardless would run the retirement collector on a path
        // that never ran it before — a second variable on a boot that is supposed to differ
        // from the control in exactly one thing.
        if self
            .parked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
        {
            return;
        }
        let freed = self.machine.reclaim_released_windows();
        if freed.is_empty() {
            return;
        }
        let ready: Vec<u64> = {
            let mut p = self.parked.lock().unwrap_or_else(|e| e.into_inner());
            let (ready, keep): (Vec<_>, Vec<_>) =
                std::mem::take(&mut *p).into_iter().partition(|(r, _)| freed.contains(r));
            *p = keep;
            ready.into_iter().map(|(_, v)| v).collect()
        };
        for v in ready {
            port.release(crate::deviceview::ViewId(v));
        }
    }

    /// Take every slot whose frame lies in `[phys, phys+len)` out of the table (the caller
    /// removes them from the machine) and cancel the pendings there.
    fn take_over_frames(t: &mut Table, phys: u64, len: u64) -> Vec<Retired> {
        let end = phys.saturating_add(len);
        let gone: Vec<Retired> = t
            .slots
            .iter()
            .filter(|(_, s)| s.key.phys >= phys && s.key.phys < end)
            .map(|(g, s)| Retired {
                gpa: *g,
                region: s.region,
                view: s.view,
            })
            .collect();
        for r in &gone {
            t.slots.remove(&r.gpa);
        }
        t.live = t.live.saturating_sub(gone.len() as u64);
        gone
    }

    /// ★★★ Wait for every fill in flight over `[phys, phys+len)` to reach its commit —
    /// where it will find the range quiesced and DROP its slot. Cancelling the pendings
    /// instead would leave a just-installed slot live for the interval between the cancel
    /// and the fill's own drop, which is exactly the window a quiesce exists to close.
    /// Bounded: a fill's phase 2 is two `mmap`s and one ioctl, so the wait is microseconds;
    /// past the bound this says so loudly and proceeds rather than hanging the plane.
    fn drain_pending_over(&self, phys: u64, len: u64) {
        let end = phys.saturating_add(len);
        for i in 0..20_000u32 {
            let busy = {
                let t = self.table.lock().unwrap_or_else(|e| e.into_inner());
                t.pending.values().any(|(_, p)| *p >= phys && *p < end)
            };
            if !busy {
                return;
            }
            if i == 19_999 {
                eprintln!(
                    "kayfabe: BAR-MIRROR ⚠⚠ QUIESCE WAITED 2 s for a fill over fb 0x{phys:x}+0x{len:x} \
                     that never committed — proceeding; if a slot outlives this, the fill's \
                     own commit will drop it (quiesced range), but the interval was not zero"
                );
                return;
            }
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
    }

    /// ★★★★★ **THE FLUSH** — on the guest's own `MMU_INVALIDATE` trigger, or a BAR2 root
    /// publication: re-walk every live entry and drop the ones that no longer say what they
    /// said when installed. Kept entries cost one page walk each; dropped ones cost the
    /// ioctl. ⊘ Never a wholesale drop: a slot whose translation is unchanged after an
    /// invalidate is still exactly right, and the guest's next touch would only re-create it.
    pub fn revalidate(&self, why: &'static str) {
        let snapshot: Vec<(u64, Slot)> = {
            let mut t = self.table.lock().unwrap_or_else(|e| e.into_inner());
            // ★ The epoch moves BEFORE the snapshot is taken (see `Table::reval_epoch`).
            t.reval_epoch += 1;
            t.slots.iter().map(|(g, s)| (*g, *s)).collect()
        };
        let mut kept = 0u64;
        let mut gone: Vec<Retired> = Vec::new();
        for (gpa, s) in snapshot {
            let same = self
                .plane
                .window_page_backing(s.window, s.page_off, false)
                .ok()
                .and_then(|r| key_of(&r))
                .is_some_and(|k| k == s.key);
            if same {
                kept += 1;
                continue;
            }
            let mut t = self.table.lock().unwrap_or_else(|e| e.into_inner());
            if t.slots.get(&gpa).is_some_and(|cur| cur.region == s.region) {
                t.slots.remove(&gpa);
                t.live = t.live.saturating_sub(1);
                gone.push(Retired {
                    gpa,
                    region: s.region,
                    view: s.view,
                });
            }
        }
        let removed = self.retire(gone);
        let runs = self.census.reval_runs.fetch_add(1, Ordering::Relaxed) + 1;
        self.census.reval_kept.fetch_add(kept, Ordering::Relaxed);
        self.census
            .reval_removed
            .fetch_add(removed, Ordering::Relaxed);
        if removed > 0 && self.census.reval_printed.fetch_add(1, Ordering::Relaxed) < REVAL_LIVE
        {
            eprintln!(
                "kayfabe: BAR-MIRROR REVALIDATE #{runs} [{why}]: kept={kept} removed={removed} \
                 — the guest declared its BAR page tables live and {removed} slot(s) named a \
                 translation or a backing that is no longer what it was (printed only when \
                 something was removed, at most {REVAL_LIVE} times; totals in the census)",
            );
        }
    }

    /// The trap-path hook for a completed register write: an invalidate trigger
    /// revalidates everything; a moved `UPDATE_BAR_PDE` count revalidates too (BAR2's root
    /// is a value the guest can re-publish at any time).
    /// ⊘⊘⊘ **w468 — THIS RUNS ON THE vCPU, SO IT MUST NOT WALK ANYTHING.** Until w468 it
    /// called [`BarMirror::revalidate`] inline, which is one page walk per live slot plus a
    /// memslot ioctl per drop — with BAR1 passthrough armed that is tens of thousands of
    /// slots and tens of milliseconds, *inside the guest's MMIO exit*. It now only records
    /// the request; [`BarMirror::revalidate_pending`] does the work on the publication
    /// worker, and the invalidate's completion is withheld until it has.
    ///
    /// ★ Why deferring is safe: the guest is spinning on the invalidate's completion, and
    /// that completion is not written until the worker has revalidated. A stale slot can
    /// therefore only be observed by a guest thread that raced its own invalidate — and it
    /// names a translation that same guest had legitimately mapped a moment earlier, so the
    /// exposure is guest self-corruption, never cross-process leakage. A translation to
    /// memory the guest does not own cannot appear this way: a *new* key is only installed
    /// on a fresh fault, which takes the ownership check.
    /// ★★★★★ **w578 — re-point the PRAMIN slot when the guest moves the latch.**
    ///
    /// ⊘ Synchronous, on the vCPU, deliberately: RM writes the latch and uses the window
    /// immediately, with no completion to defer behind. It is ONE `mmap` over a slot that does
    /// not change — the hypervisor is not told, because nothing it knows has moved.
    ///
    /// ⊘ A re-point to the address already shown is SKIPPED and counted. The guest writes this
    /// register as a read-modify-write, so the same value arrives repeatedly, and each
    /// redundant `MAP_FIXED` would shoot down every vCPU's TLB for no change.
    fn repoint_pramin(&self) {
        let Some(base) = self.plane.pramin_fb_base() else {
            return;
        };
        let mut slot = self.pramin.lock().unwrap_or_else(|e| e.into_inner());
        // ★★★★★ **w580 — INSTALLED ON FIRST USE, not at arm.**
        //
        // ⊘⊘ w579 installed it when the mirror armed, off `bar_placement(Bar0)`. `[measured
        // w579]` the guest's firmware then REPROGRAMS BAR0, and the device refused every
        // later base-address write — *"the reservation BAR moved after a memslot was
        // installed"* — so the boot never reached the client at all.
        //
        // ★ The first latch write is the right moment and needs no new signal: the guest
        // cannot aim a window in a BAR it has not placed, so by the time this runs BAR0's
        // base is final and is the one the guest is actually using.
        // ⊘⊘⊘ **w582 PARKED THIS. w585 UN-PARKS IT, and the reason it was parked was wrong.**
        //
        // `[measured w581]` with the aperture placed and heap pages migrated, PRAMIN's traps go
        // to **ZERO** — `window[SERVED r=0 w=0]`, exactly the target — and the guest still
        // failed in `_kgspBootGspRm`. w582 read that as *"the mapping works, so the mapping is
        // not what is wrong"* and went looking for a semantic difference.
        //
        // ⊘ **That reading was the mistake, and it cost three commits.** Zero traps proves the
        // slot INTERCEPTS the access. It says nothing about whether the slot shows the same
        // BYTES the trap path would have — and it did not:
        //
        //   - **w584**: `fresh_page` handed `alloc_at` a frame NUMBER where it wanted a byte
        //     ADDRESS, so every store page since w569 was refused and fell back to the heap.
        //     The slot therefore mapped a file that the store had never written.
        //   - **w585**: even once that was fixed, the store answered a frame it had no page
        //     for with ZEROS, and zero-filled each arena page at creation — so it ignored what
        //     the slot wrote and erased what the slot held.
        //
        // ⇒ Three separate reasons the slot and the trap were two different memories, none of
        // them about *which* framebuffer a window names.
        //
        // ⊘ And the hypothesis w582 parked this ON is **refuted**: `Bar0Window::target()` has
        // **zero call sites** in the tree. There is no TARGET field being ignored, because
        // nothing reads one. ⚠ I wrote that hypothesis as *"UNTESTED"* and then treated it as
        // the reason to stop — an untested hypothesis is not a blocker, and checking this one
        // cost one `grep`.
        if slot.is_none() {
            // ★★★★★ **w587 — THE ARM KNOB, so one binary grades both sides.**
            //
            // ⊘ w586 landed FOUR unmeasured commits at once and the boot regressed to (E).
            // Without a knob, separating "the PRAMIN slot" from "the store residency fixes"
            // costs a rebuild per arm and a claim about which build was which. With one, the
            // two arms differ by an environment variable and nothing else — which is the only
            // form of this comparison that is worth anything.
            //
            // ★ Default ON: the slot is the goal, not the experiment. `=0` is the control.
            if std::env::var("KAYFABE_PRAMIN_SLOT").is_ok_and(|v| v == "0") {
                eprintln!(
                    "kayfabe: PRAMIN-WINDOW ⊘ ARM OFF (KAYFABE_PRAMIN_SLOT=0) — the aperture \
                     keeps trapping, byte for byte as before. This is the CONTROL arm."
                );
                return;
            }
            let (Some((span_off, span_len)), Some(p)) =
                (self.plane.pramin_span(), self.machine.bar_placement(BarId::Bar0))
            else {
                return;
            };
            match self.install_pramin_window(p.base + span_off, span_len, base) {
                Ok((region, view)) => {
                    *slot = Some((region, base, view));
                    self.pramin_moves.fetch_add(1, Ordering::Relaxed);
                    // w593 - the prefix, latched once, before any post-install access.
                    let c = self.plane.counters();
                    self.pramin_at_install
                        .store(c.pramin_reads + c.pramin_writes, Ordering::Relaxed);
                    self.pramin_gpa.store(p.base + span_off, Ordering::Relaxed);
                    self.mark_pramin(base);
                    eprintln!(
                        "kayfabe: PRAMIN-WINDOW installed at gpa=0x{:x} len=0x{span_len:x} \
                         showing fb 0x{base:x}, on the guest's FIRST latch write. ⊘ ONE slot: \
                         reads AND writes through this aperture resolve in the guest, and a \
                         later move is ONE mmap over the same slot.",
                        p.base + span_off
                    );
                }
                Err(e) => eprintln!(
                    "kayfabe: PRAMIN-WINDOW ⊘ NOT INSTALLED ({e:?}) — the aperture keeps \
                     trapping, byte for byte as before. ⚠ A REFUSAL, not a fallback."
                ),
            }
            return;
        }
        let Some((region, shown, view)) = *slot else {
            return;
        };
        if shown == base {
            self.pramin_skipped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let t0 = std::time::Instant::now();
        // ★★★★★ **§3 — A DEVICE VIEW CANNOT BE RE-POINTED, SO THE MOVE IS RELEASE-AND-RE-ARM.**
        //
        // `SINGLE_STORE_PLAN.md` §3 item 4: *"today one re-pointable slot over the arena's
        // file (`repoint_file_window`). A device view cannot be re-pointed — each arming is
        // its own fd at offset 0 — so PRAMIN becomes release-and-re-arm, **~0.7 ms
        // measured**, on the vCPU, which is the one sanctioned expensive trap (constraint 4)
        // and is inside its budget."*
        //
        // ⊘ `nvidia_mmap_helper` refuses any `vm_pgoff` but zero, so *what* a node shows was
        // fixed by the `NV_ESC_RM_MAP_MEMORY` that armed it. There is no offset to move.
        //
        // ⚠ **THE GAP IS REAL AND IS NAMED.** `repoint_file_window` is one `MAP_FIXED` that
        // replaces the backing atomically; this is a window removal followed by an install,
        // and between them PRAMIN has no slot. A sibling vCPU accessing PRAMIN in that
        // interval traps — and on this arm the trap path reaches the single store and is
        // refused by name. ⊘ Said rather than discovered: it is bounded by one install and it
        // is the price of a backing that cannot be re-pointed.
        let outcome = if view.is_some() {
            let Some(p) = self.machine.bar_placement(BarId::Bar0) else {
                return;
            };
            let Some((span_off, span_len)) = self.plane.pramin_span() else {
                return;
            };
            let gpa = p.base + span_off;
            let _ = self.retire(vec![Retired {
                gpa,
                region,
                view,
            }]);
            match self.install_pramin_window(gpa, span_len, base) {
                Ok((r2, v2)) => {
                    *slot = Some((r2, base, v2));
                    self.pramin_moves.fetch_add(1, Ordering::Relaxed);
                    self.mark_pramin(base);
                    let ns = u64::try_from(t0.elapsed().as_nanos()).unwrap_or(u64::MAX);
                    self.pramin_move_ns_total.fetch_add(ns, Ordering::Relaxed);
                    self.pramin_move_ns_worst.fetch_max(ns, Ordering::Relaxed);
                    return;
                }
                Err(why) => {
                    // ⊘ The old slot is already gone. PRAMIN traps from here on and the trap
                    // path refuses by name — which is loud, and is the honest state: there is
                    // no window, rather than a window showing the wrong framebuffer.
                    *slot = None;
                    eprintln!(
                        "kayfabe: PRAMIN-WINDOW ⊘⊘ RE-ARM REFUSED to fb 0x{base:x} ({why}); \
                         the old view was released and there is now NO slot. Every PRAMIN \
                         access traps and the single store refuses it by name."
                    );
                    let ns = u64::try_from(t0.elapsed().as_nanos()).unwrap_or(u64::MAX);
                    self.pramin_move_ns_total.fetch_add(ns, Ordering::Relaxed);
                    self.pramin_move_ns_worst.fetch_max(ns, Ordering::Relaxed);
                    return;
                }
            }
        } else {
            self.machine
                .repoint_file_window(region, self.arena.as_backing_fd(), base)
        };
        // ⊘ Timed around the syscall ONLY, and recorded on both outcomes: a refused repoint
        // still spent the time, and excluding it would flatter the worst case.
        let ns = u64::try_from(t0.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.pramin_move_ns_total.fetch_add(ns, Ordering::Relaxed);
        self.pramin_move_ns_worst.fetch_max(ns, Ordering::Relaxed);
        match outcome {
            Ok(()) => {
                *slot = Some((region, base, view));
                self.pramin_moves.fetch_add(1, Ordering::Relaxed);
                self.mark_pramin(base);
            }
            // ⚠ A refused re-point leaves the slot showing what it showed. That is WRONG for
            // the guest — it will read the old framebuffer — so it is said, not swallowed.
            Err(e) => eprintln!(
                "kayfabe: PRAMIN-WINDOW ⊘⊘ RE-POINT REFUSED to fb 0x{base:x} ({e:?}); the \
                 aperture still shows 0x{shown:x} and the guest's next access through it is \
                 WRONG. This is the one failure on this path that cannot be contained."
            ),
        }
    }

    /// ★★★ **PRAMIN's WINDOW, ON WHICHEVER ARM IS RUNNING** — one place, two backings.
    ///
    /// ⊘ NOT read-only on either arm: PRAMIN is the framebuffer, not a register file.
    /// `[measured w577]` its writes are 631 458 of 635 162 accesses, so a read-only slot
    /// would remove the reads and leave the larger half exiting.
    ///
    /// # Errors
    /// The refusal's rendering, for the caller to print with its own context.
    fn install_pramin_window(
        &self,
        gpa: u64,
        span_len: u64,
        base: u64,
    ) -> Result<(RamRegionId, Option<u64>), String> {
        // ⊘⊘⊘ **KEYED ON THE STORE ARM, NOT ON THE PORT'S PRESENCE — and the difference is a
        // TWO-MEMORIES DEFECT.** The device-view port is armed by `KAYFABE_DEVICE_VIEW`, which
        // is a **different gate** from `KAYFABE_FB_STORE`: a boot may legitimately run
        // `KAYFABE_DEVICE_VIEW=probe` with the default `arena` store — that is exactly what
        // w734's census boot did. Asking *"is there a port?"* would put PRAMIN on the reserved
        // object while every other framebuffer path served the arena memfd, i.e. two memories
        // for one address, silently, on the control arm.
        let device = crate::deviceview::backing_is_device(
            if self.device_store {
                crate::deviceview::FbStoreArm::Device
            } else {
                crate::deviceview::FbStoreArm::Arena
            },
            self.device_port.is_some(),
        );
        match device.then(|| self.device_port.as_ref()).flatten() {
            Some(port) => {
                match port.with_node(base, span_len, true, |fd, mmap_len| {
                    self.machine.install_device_page(gpa, mmap_len, fd, false)
                }) {
                    Ok(Ok((r, id))) => Ok((r, Some(id.0))),
                    Ok(Err(e)) => Err(format!("{e:?}")),
                    Err(v) => Err(format!("{}: {v:?}", v.name())),
                }
            }
            None => self
                .machine
                .install_file_window(gpa, span_len, self.arena.as_backing_fd(), base, false)
                .map(|r| (r, None))
                .map_err(|e| format!("{e:?}")),
        }
    }

    /// ★ w597 — one mark per accepted PRAMIN placement. O(1), bounded by the move count.
    fn mark_pramin(&self, base: u64) {
        // ⊘⊘ w607 — PRAMIN's OWN counters. This sampled `fb_reads + fb_writes`, the union of
        // BAR1 + BAR2 + PRAMIN, and every number this instrument produced was therefore about
        // traffic through apertures it does not govern. See `Counters::pramin_reads`.
        let c = self.plane.counters();
        let mut m = self.pramin_marks.lock().unwrap_or_else(|e| e.into_inner());
        if m.len() < 64 {
            m.push((c.pramin_reads + c.pramin_writes, base));
        }
    }

    /// `(deltas between successive placements, exits after the last one)` — the SHAPE of the
    /// residual PRAMIN traffic. See [`BarMirror::pramin_marks`].
    fn pramin_shape(&self, total: u64) -> String {
        let m = self.pramin_marks.lock().unwrap_or_else(|e| e.into_inner());
        if m.is_empty() {
            return "shape=none (never placed)".to_string();
        }
        let mut d: Vec<u64> = m.windows(2).map(|w| w[1].0.saturating_sub(w[0].0)).collect();
        d.push(total.saturating_sub(m.last().map_or(0, |x| x.0)));
        let zero = d.iter().filter(|n| **n == 0).count();
        let max = d.iter().copied().max().unwrap_or(0);
        // ★★★ w599 — the BASE each interval was showing. `[measured w597]` 16 of 22 intervals
        // leak nothing and two account for 3 205 of 3 228 exits, so the leak follows particular
        // WINDOW POSITIONS rather than the slot mechanism. Which positions is the next fact,
        // and a delta list cannot carry it.
        let leaky: Vec<String> = d
            .iter()
            .enumerate()
            .filter(|(_, n)| **n > 0)
            .map(|(i, n)| format!("fb0x{:x}:{n}", m.get(i).map_or(0, |x| x.1)))
            .collect();
        format!(
            "shape[placements={} per_placement_exits={:?} zero_intervals={zero} worst={max}              leaky_bases=[{}]] => {}",
            m.len(),
            &d[..d.len().min(24)],
            leaky.join(" "),
            if zero * 2 > d.len() {
                "BURSTY - most intervals leak nothing, so the exits follow particular placements rather than leaking steadily"
            } else {
                "STEADY - every interval leaks, so the slot is not covering the range it claims"
            }
        )
    }

    /// ★★★★★ **w617 — MAP AT CREATE, for BAR1. The owner's ruling, implemented.**
    ///
    /// > *"if a channel is created inheriting a va base, then you can map at create, of
    /// > existing known va maps, and only return from rpc if channel is usuable."*
    ///
    /// `[measured w608]` BAR1+BAR2 cost **3 952 trapped accesses for 185 distinct pages** — a
    /// factor of 21 — because every fill is on DEMAND and the guest re-touches a page many
    /// times before its slot lands. `[measured w616]` **100 % of BAR1's 66 pages are needed
    /// only AFTER the first channel birth**, so a birth is the right moment and the ruling is
    /// aimed at real traffic rather than at bring-up.
    ///
    /// ⊘ **BAR1 ONLY, and refused by name for BAR2 rather than approximated.**
    /// `RegPlane::window_leaves` enumerates BAR1 because its directory is a chip constant that
    /// names a PAGE; BAR2's root is a raw PDE ENTRY the guest republishes, and the entry-rooted
    /// subtree decode that would enumerate it **does not exist**. ⇒ BAR2's 99 post-birth pages
    /// stay on demand until it does. Half the win, honestly bounded, beats a whole win computed
    /// from a root that might be the wrong level — *"a wrong root yields a plausible, WRONG
    /// list of leaves"*.
    ///
    /// ⚠ **Filling INLINE here is the ruling, not a shortcut.** This runs off the vCPU in the
    /// birth drain, and the guest is blocked on the RPC that caused the birth — which is one of
    /// the owner's three sanctioned synchronization points. *"Only return from rpc if channel
    /// is usable"* means the cost belongs here. The drain's own budget reports an overrun, so
    /// the price is visible rather than hidden.
    /// ★ w624 — both apertures. BAR2 became enumerable when `decode_subtree_from_entry`
    /// landed; before that `window_leaves` refused it by name and BAR2's 121 pages stayed on
    /// demand. ⊘ Ordered BAR1 first so a BAR2 regression cannot be mistaken for a BAR1 one.
    pub fn premap_bars(&self) {
        self.premap_bar1();
        self.premap_window(FbWindow::InstanceWindow);
    }

    pub fn premap_bar1(&self) {
        self.premap_window(FbWindow::FbAperture);
    }

    fn premap_window(&self, win: FbWindow) {
        // ⊘⊘⊘ **w618 FAILED ITS CRITERION; w620 FIXES THE TWO REAL DEFECTS AND MOVES THE
        // MOMENT. Default ON again, `KAYFABE_PREMAP_BAR1=0` is the control.**
        //
        // ⚠ **My w618 self-diagnosis was WRONG and is corrected here.** I wrote that this
        // *"maps the wrong set"* and needs the channel's own VA maps. It does not: BAR1 is its
        // own VAS keyed by `bar1_pde_base`, its leaves ARE its known maps, a channel has no
        // "own" BAR1 subset, and going from a VAS row to an aperture offset would be the
        // reverse resolution `mode2_address_table.md` forbids. The 253 extra pages were
        // UNTOUCHED, not wrong. ⇒ Two other things were wrong, both real:
        //   (a) one 4 KiB fill per leaf while GA10x leaves are 64 KiB — fixed below;
        //   (b) the MOMENT. See `premap_bar1`'s caller.
        //
        // ⊘ Kept as a record of what the old arm measured:
        //
        // `[measured w617a, a boot that graded (P)]`, against the criterion fixed before it ran:
        //
        //     ALREADY-COVERED-EARLY   2 574 -> 11 480     4.5x WORSE
        //     bar1 distinct_pages        66 ->    319     253 pages the guest never asked for
        //     bar1 fills                166 ->    679
        //     premap runs=30 pages=7811               260 pages per birth for a 66-page set
        //     BAR1 writes                            NOT reduced
        //     BAR2 2 / 1 453                          unchanged - the one prediction that held
        //
        // ★ The ruling is right and **this implementation maps the wrong set**. `window_leaves`
        // enumerates every leaf BAR1 has a PTE for — the aperture's whole mapped VA range — not
        // the pages the CHANNEL needs. So it installed 253 slots nobody touches, re-enumerated
        // the same tree at each of 30 births, and did not remove the traps it was aimed at.
        //
        // ⚠ The owner's words were *"of existing known va maps"* — the maps a channel INHERITS,
        // not every leaf in the window. I read that as "everything currently mapped", which is
        // a strictly larger set and the wrong one.
        //
        // ⇒ Kept behind a knob rather than deleted, because the machinery is right and only its
        // INPUT is wrong: a corrected version needs the channel's own VA maps at birth, and it
        // will want to be graded against this arm. **A change that fails a pre-registered
        // criterion is turned off, never tuned until it goes green.**
        if std::env::var("KAYFABE_PREMAP_BAR1").is_ok_and(|v| v == "0") {
            return;
        }
        let Some(arm) = self.arm_for(win) else {
            return;
        };
        let budget = if win == FbWindow::InstanceWindow {
            PREMAP_BUDGET_BAR2
        } else {
            PREMAP_BUDGET_BAR1
        };
        // ★★★★★ **CUT B item 4 — THE PREMAP REFUSAL STOPS BEING TERMINAL, AND SO DOES THE
        // SILENT-EMPTY ONE BESIDE IT.**
        //
        // `SINGLE_STORE_PLAN.md` cut B item 4: *"`window_leaves` refuses the whole subtree at
        // the first unbacked page and the caller prints once and returns — `that aperture
        // stays on demand-fill`, which under `device` means the trap fires and there is
        // nothing to serve it."*
        //
        // ⊘⊘⊘ **AND THE MECHANISM IS WORSE THAN THAT SENTENCE, measured from the source
        // (w737).** `decode_subtree` returns `Err` for **budget exhaustion and nothing else**:
        // an unreadable page-table page is a per-branch `WalkFault` and the walk continues. So
        // the failing shape is not a refusal at all — it is `Ok` with a **SHORT leaf list**,
        // and an unreadable ROOT gives `Ok` with an **EMPTY** one. That reads as *"the guest
        // has mapped nothing"*, publishes nothing, and counts nothing. ⇒ `WindowEnumeration`
        // now carries `faults`, and this loop retries on **either** shape.
        //
        // ⚠ A fixed trip count, for `resolve_arming`'s reason: each pass arms the frontier it
        // could not read, so a tree converges in at most its own depth — and the depth is
        // format-bounded, never guest-bounded.
        let (enumerated, attempt) = arm_then_retry(
            Self::ARM_RETRIES,
            || {
                let got = self.plane.window_leaves(win, budget);
                // ⊘ "Good" is `Ok` AND no fault: a short list is the failing shape here, and
                // an `is_ok()` test would call it success.
                let good = got.as_ref().is_ok_and(|e| e.faults == 0);
                (got, good)
            },
            || self.plane.arm_fb_demand().progressed(),
        );
        self.arm_retries.fetch_add(u64::from(attempt), Ordering::Relaxed);
        if enumerated.as_ref().is_ok_and(|e| e.faults > 0) {
            self.premap_pt_faults.fetch_add(1, Ordering::Relaxed);
        }
        let kayfabe_device::WindowEnumeration {
            leaves,
            visited,
            faults,
        } = match enumerated {
            Ok(l) => l,
            Err(e) => {
                self.premap_refused.fetch_add(1, Ordering::Relaxed);
                // ⊘⊘ **NAME IT, w625.** `[measured w624a]` `premap[refused=1181]` — the BAR2
                // enumeration was refused on EVERY invalidate, and the count alone cannot say
                // which of three things happened: no published root (`BAR2_UNROOTED`), a
                // `level_shift` matching no format row (`BAR2_UNKNOWN_ROOT_LEVEL`), or the
                // subtree walk itself faulting (`WINDOW_ENUMERATION_REFUSED`). Three causes,
                // one symptom, three different fixes — the shape this session has now paid for
                // four times. ⚠ Printed ONCE per distinct reason, not per refusal: 1 181
                // identical lines would be the log telling the truth and nobody reading it.
                let why = match e {
                    kayfabe_device::WindowRefusal::NoAddressModel => "no address model",
                    kayfabe_device::WindowRefusal::Translated { why, .. } => why,
                };
                let mut seen = self.premap_why.lock().unwrap_or_else(|x| x.into_inner());
                if seen.insert((win == FbWindow::InstanceWindow, why)) {
                    eprintln!(
                        "kayfabe: PREMAP ⊘ {} enumeration REFUSED [{why}] — that aperture stays \
                         on demand-fill. First occurrence only; the total is `premap[refused=]`.",
                        if win == FbWindow::InstanceWindow { "bar2" } else { "bar1" }
                    );
                }
                return;
            }
        };
        // ★★★★★ **w622 — TAKE THE TABLE LOCK ONCE PER ENUMERATION, NOT ONCE PER PAGE.**
        //
        // `[measured w620a]` BAR1 reached ZERO traps at a cost of **567 312 `fill_now` calls**
        // across 1 181 invalidates — each one acquiring the table lock for its phase-0
        // "already covered?" check, and 83 882+ of them answering yes. ⊘ The enumeration cannot
        // be skipped when the tree is unchanged, because **we cannot tell**: BAR1's page tables
        // are written by the guest through BAR2, which is slot-served and therefore invisible,
        // so the invalidate is the only signal we get and it is the one we already use.
        //
        // ⇒ What CAN be removed is the per-page locking. Snapshot the covered set once, filter
        // against it, and call `fill_now` only for pages that are actually missing. 567 312
        // acquisitions become 1 181.
        //
        // ⚠ The snapshot can go stale between the lock and the fill — another thread may
        // install a page we are about to ask for. That is HARMLESS and already handled:
        // `fill_now` re-checks under the lock and refuses `ALREADY-COVERED-EARLY`. The snapshot
        // is an optimisation, never an authority, and the race it loses costs one redundant
        // call rather than a wrong slot.
        let covered: std::collections::BTreeSet<u64> = {
            let t = self.table.lock().unwrap_or_else(|e| e.into_inner());
            t.slots.keys().copied().collect()
        };
        let mut asked = 0u64;
        let mut skipped = 0u64;
        let mut biggest = 0u64;
        for leaf in leaves {
            // ★ The window OFFSET is the leaf's VA within the aperture, VERIFIED rather than
            // assumed this time: `window_leaves` roots the decode at `vabase: 0` and
            // `bar1_translate` walks the identical root with the identical `vabase`, so both
            // speak the same coordinate. ⊘ w617 asserted this in a comment without checking,
            // and it happened to be right — which is worse than being wrong, because it made
            // the real defect look like a coordinate problem.
            let off = leaf.va.0;
            // ⊘⊘⊘ **A LEAF IS NOT A PAGE, AND THIS WAS THE w617 DEFECT.** `fill_now` installs
            // exactly one 4 KiB slot, and `[GA10X_PAGE_SIZES]` offers 4 KiB, **64 KiB**, 2 MiB
            // and 512 MiB. RM caps BAR1 mappings at the big page size and forces 64 KiB on a
            // BAR1 of 256 MiB or less, so any vidmem object ≥ 64 KiB — a USERD pool, a
            // pushbuffer — is ONE leaf covering SIXTEEN pages, of which w617 filled the first.
            // The other fifteen kept demand-filling exactly as before, which is precisely the
            // "premapping did not reduce the traps" I could not explain.
            let size = leaf.size.0.max(PAGE);
            biggest = biggest.max(size);
            for page in (off..off.saturating_add(size)).step_by(PAGE as usize) {
                if page >= arm.len {
                    break;
                }
                if covered.contains(&(arm.base + page)) {
                    skipped += 1;
                    continue;
                }
                asked += 1;
                self.fill_now(win, page, FillOrigin::Premap);
            }
        }
        // ⊘⊘ **A SHORT ENUMERATION IS SAID, ONCE, BY NAME.** Before cut B this number did not
        // exist and the list came back short in silence — the empty-artefact class this tree
        // has now paid for four times. ⚠ On the arena arm it should be ZERO: `SparseFb::read`
        // answers every in-range address, so a fault there is a real page-table finding and
        // not an unarmed page.
        if faults > 0 {
            let mut seen = self.premap_why.lock().unwrap_or_else(|x| x.into_inner());
            if seen.insert((win == FbWindow::InstanceWindow, PREMAP_SHORT)) {
                eprintln!(
                    "kayfabe: PREMAP ⊘⊘ {} enumeration came back SHORT — {faults} branch(es) \
                     could not be decoded after {attempt} arming retr(ies), so the leaf list \
                     below is a SUBSET of what the guest mapped and those pages stay on \
                     demand-fill. ⊘ This is not `the guest mapped nothing`. First occurrence \
                     only; the total is `premap[pt_faults=]`, and `FB-DEMAND` says whether \
                     arming was declined, refused or never drained.",
                    if win == FbWindow::InstanceWindow { "bar2" } else { "bar1" }
                );
            }
        }
        self.premap_biggest_leaf.fetch_max(biggest, Ordering::Relaxed);
        self.premap_pages.fetch_add(asked, Ordering::Relaxed);
        self.premap_skipped.fetch_add(skipped, Ordering::Relaxed);
        self.premap_runs.fetch_add(1, Ordering::Relaxed);
        if win == FbWindow::InstanceWindow {
            self.premap_bar2_visited
                .fetch_max(visited as u64, Ordering::Relaxed);
        }
    }

    /// ★★★★★ **w637 — place the host's usermode page over the guest's counter page.**
    ///
    /// One mapping, two permissions, which is the owner's design and the whole point:
    ///   - the VMA is **writable**, because ringing the host doorbell with a translated token
    ///     must be one dword store inline on the vCPU, with no IPC to block on;
    ///   - the slot is **read-only**, so the guest's doorbell store at `+0x90` still EXITS and
    ///     we get to translate the token at all.
    ///
    /// ⊘ ONE page of the 64 KiB mapping is slotted. `TIME_0`, `TIME_1` and the doorbell all
    /// live in page 0 and `[measured]` the other fifteen take zero reads; the `mmap` is 64 KiB
    /// only because the driver refuses any other length. The rest stays `observe`, which keeps
    /// trapping — the honest default for hardware nobody asked to expose.
    ///
    /// ⚠ Idempotent by the `Some` check, because the caller is a worker loop that runs every
    /// tick and this is a once-only placement.
    ///
    /// Returns `true` only when a slot was newly installed.
    pub fn install_counter_page(&self, fd: std::os::fd::BorrowedFd<'_>, mmap_len: u64) -> bool {
        let mut slot = self.counter_slot.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() {
            return false;
        }
        let (Some((off, len)), Some(p)) = (
            self.plane.usermode_page_span(),
            self.machine.bar_placement(BarId::Bar0),
        ) else {
            return false;
        };
        let gpa = p.base + off;
        match self
            .machine
            .install_device_window(gpa, mmap_len, fd, Some(gpa..gpa + len), true)
        {
            Ok(region) => {
                *slot = Some(region);
                eprintln!(
                    "kayfabe: COUNTER-PAGE installed at gpa=0x{gpa:x} mmap_len=0x{mmap_len:x} \
                     slot=0x{len:x} \u{2299} the guest now reads the GPU's OWN clock with no exit, \
                     and its doorbell store still traps because the slot is read-only."
                );
                true
            }
            Err(e) => {
                eprintln!(
                    "kayfabe: COUNTER-PAGE \u{2298} NOT INSTALLED ({e:?}) — the page keeps trapping \
                     and the counter keeps being answered from a host CPU clock. \u{26a0} A REFUSAL, \
                     not a fallback."
                );
                false
            }
        }
    }

    /// ★ w613 — called once, at the first channel birth. See `pages_at_first_birth`.
    pub fn note_first_channel_birth(&self) {
        let mut g = self
            .pages_at_first_birth
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if g.is_none() {
            let t = self.table.lock().unwrap_or_else(|e| e.into_inner());
            *g = Some((t.pages_ever[0].len(), t.pages_ever[1].len()));
        }
    }

    pub fn after_write(&self, out: &kayfabe_device::WriteOutcome) {
        // ★ w578 — the latch first: it is synchronous and cheap, and everything below defers.
        if out.claimed {
            // ★ w603 — one atomic compare per claimed write, on a path already taken. See
            // `bar0_moves`: an endpoint comparison cannot see BAR0 move away and back.
            let now = self
                .machine
                .bar_placement(BarId::Bar0)
                .map_or(u64::MAX, |p| p.base);
            let was = self.bar0_last.swap(now, Ordering::Relaxed);
            if was != u64::MAX && was != now {
                self.bar0_moves.fetch_add(1, Ordering::Relaxed);
            }
            self.repoint_pramin();
        }
        let mut why = 0u64;
        if out.invalidate.as_ref().is_some_and(|inv| inv.trigger) {
            why |= 1;
        }
        let u = self.plane.bar_pde_counts().0;
        if self.last_bar_pde_updates.swap(u, Ordering::Relaxed) != u {
            why |= 2;
        }
        if why == 0 {
            return;
        }
        // ⊘ **THE w468 A/B IS SETTLED AND ITS LOSING ARM IS GONE.**
        // `KAYFABE_MIRROR_REVAL_INLINE` compared the inline walk against the deferred one in
        // a single binary. `[measured w469, one binary, two arms]` deferred: `bar0+0xb830b0`
        // **absent from the slow-site table entirely**; inline: the **top** site at 53 hits,
        // worst 42.5 ms. Both arms passed the client. There is nothing left to compare.
        //
        // ⚠ `defer_reval` STAYS, and it is not the same thing: it is false when no publication
        // worker exists to drain the queue, and deferring there would leave every stale slot
        // live forever. A configuration is not an experiment.
        if !self.defer_reval {
            self.revalidate(if why == 1 { "mmu-invalidate" } else { "bar-pde-update" });
            return;
        }
        self.reval_why.fetch_or(why, Ordering::Relaxed);
        self.reval_req.fetch_add(1, Ordering::Release);
    }

    /// ★★★★★ **w468 — the worker's half.** Runs the walk `after_write` deferred, if any is
    /// outstanding. ⊘ Never call from a vCPU thread.
    pub fn revalidate_pending(&self) {
        let req = self.reval_req.load(Ordering::Acquire);
        if req == self.reval_done.load(Ordering::Relaxed) {
            return;
        }
        let why = match self.reval_why.swap(0, Ordering::Relaxed) {
            1 => "mmu-invalidate",
            2 => "bar-pde-update",
            _ => "mmu-invalidate+bar-pde-update",
        };
        self.revalidate(why);
        // ★ Raise to the value read BEFORE the walk: a request that arrived during it is
        // still outstanding and the next pass runs again.
        self.reval_done.store(req, Ordering::Release);
    }

    /// ★★★ **w734 — THE HIGHEST FRAMEBUFFER ADDRESS THE GUEST CAUSED A PAGE TO EXIST AT**,
    /// in bytes: the identity-window invariant's known-positive
    /// (`crate::scratchpad::identity_window_reached`).
    ///
    /// ⊘ It is the arena's index high-water × the page size, and it means what it says only
    /// because of [`kayfabe_device::FbPageArena::alloc_at`]'s contract — *"framebuffer address
    /// `frame` is placed at fd offset `frame`"*. ⇒ the arena's own span IS the guest's answer
    /// to *"how high did you go"*, and it consults nothing the verdict it checks computed.
    ///
    /// ⚠ `0` is **vacuous**, not *"the guest stayed low"*: it means no page was ever
    /// arena-backed. The consumer says so by name.
    #[must_use]
    pub fn arena_span_bytes(&self) -> u64 {
        let (_, _, _, issued) = self.arena.census();
        issued.saturating_mul(PAGE)
    }

    /// The census, one line per armed window plus one for the mechanism.
    pub fn report(&self, at: &str) {
        self.census.census_lines.fetch_add(1, Ordering::Relaxed);
        // ★★★★★ **§3's OWN LINE, printed on BOTH arms.** ⊘ Separate from the slot census
        // because *"how many memslots are live"* and *"how much host BAR1 aperture are we
        // holding"* are bounded by completely different things — §w724c's two terms — and a
        // reader who saw only the first would not know which one bit.
        {
            let parked = self
                .parked
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .len();
            match self.device_port.as_ref() {
                Some(port) => eprintln!(
                    "kayfabe: FB-STORE-DEVICE AT {at}: {} parked_releases={parked} ⇒ {}",
                    port.census_line(),
                    if parked == 0 {
                        "★ no view is waiting on a mapping that has not been collected"
                    } else {
                        "⚠ views whose SLOT is gone and whose MAPPING may not be. They are                          held deliberately: releasing early leaves PTEs pointing at BAR1                          space RM has re-handed out, which is silent and cross-tenant. A                          number that never falls is a leak and refuses later arms loudly."
                    }
                ),
                None => eprintln!(
                    "kayfabe: FB-STORE-DEVICE AT {at}: ⊘ ARENA ARM — no device-view port, no                      armed views, the framebuffer is a host memfd. This is the control and                      the default ({}=arena).",
                    crate::deviceview::FB_STORE_ENV
                ),
            }
        }
        // ★★★★★ **w587 — PRAMIN's move counters, printed. They existed since w577 and nothing
        // read them**, which is the w584 failure exactly: a number that was correct the whole
        // time and had no emitter. `[measured w586a]` `moves=42` in `BAR0-READS` says the guest
        // re-aims this window 42 times a boot, so whether the slot FOLLOWED it is a fact about
        // whether the guest read the right framebuffer — and it was unanswerable.
        //
        // ⊘ `moves` counts INSTALL + every accepted re-point; `skipped` counts latch writes
        // that named the base already shown. `moves + skipped` should equal the latch writes,
        // and a gap is re-points that were REFUSED — the one failure on this path that cannot
        // be contained, since the guest then reads the wrong framebuffer with no fault.
        {
            let shown = self
                .pramin
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .map_or_else(|| "none".to_string(), |(_, b, _)| format!("0x{b:x}"));
            let c = self.plane.counters();
            let total = c.pramin_reads + c.pramin_writes;
            let pre = self.pramin_at_install.load(Ordering::Relaxed);
            // ★ w595 — where the slot went in, against where BAR0 is NOW.
            let at = self.pramin_gpa.load(Ordering::Relaxed);
            let now = self
                .machine
                .bar_placement(BarId::Bar0)
                .and_then(|p| self.plane.pramin_span().map(|(off, _)| p.base + off));
            let placement = match (at, now) {
                (u64::MAX, _) => " gpa=none".to_string(),
                (a, Some(n)) if a == n => format!(
                    " gpa=0x{a:x} (BAR0 ends where it started, and it CHANGED {} time(s) in \
                     between{})",
                    self.bar0_moves.load(Ordering::Relaxed),
                    if self.bar0_moves.load(Ordering::Relaxed) > 0 {
                        " => it moved away and back, so QEMU rebuilt the region and our slot went                          with it. The endpoint comparison this line used to print could not see that"
                    } else {
                        ""
                    }
                ),
                (a, Some(n)) => format!(
                    " gpa=0x{a:x} but the aperture is NOW at 0x{n:x} => BAR0 MOVED UNDER THE SLOT;                      the slot is serving an address nothing accesses and every exit follows from                      that, not from the slot being wrong"
                ),
                (a, None) => format!(" gpa=0x{a:x} (BAR0 is unplaced now)"),
            };
            let shape = self.pramin_shape(total);
            let split = if pre == u64::MAX {
                "\u{2298} the slot was never installed, so ALL of it is pre-install by definition".to_string()
            } else {
                format!(
                    "before_slot={pre} after_slot={} => {}",
                    total.saturating_sub(pre),
                    if total.saturating_sub(pre) == 0 {
                        "\u{2605} every exit was the PREFIX before the first latch write - installing earlier is the whole remaining fix"
                    } else {
                        "\u{2298} traffic is STILL EXITING with the slot live - the slot is not covering something it should, which is a different bug from the prefix"
                    }
                )
            };
            let moves = self.pramin_moves.load(Ordering::Relaxed);
            let worst = self.pramin_move_ns_worst.load(Ordering::Relaxed);
            // A mean over ZERO moves is not 0, it is undefined - and printing 0 would read as
            // "the repoint is free" for a boot in which it never ran.
            let mean = if moves == 0 {
                "n/a (no move)".to_string()
            } else {
                format!("{}", self.pramin_move_ns_total.load(Ordering::Relaxed) / moves)
            };
            eprintln!(
                "kayfabe: PRAMIN-SLOT AT {at}: moves={moves} skipped={} showing={shown} window_accesses={total} {split}{placement} {shape} move_ns[worst={worst} mean={mean}] \u{2605} THE REPOINT IS A BLOCKING DOOR ON THE vCPU (goal 3) AND PART OF A WRITE TRAP (goal 6): `worst` is what both are actually worth, and it was UNMEASURED before w652 - absent from SLOW-SITES only proves it is under 1ms. \u{2298} moves+skipped below the guest's latch-write count means a re-point was REFUSED and the guest read the wrong framebuffer.",
                self.pramin_skipped.load(Ordering::Relaxed),
            );
        }
        let (live, peak, pages, frames, refused) = {
            let t = self.table.lock().unwrap_or_else(|e| e.into_inner());
            let mut r: Vec<(&'static str, u64)> = t.refused.iter().map(|(k, v)| (*k, *v)).collect();
            r.sort_unstable();
            (
                t.live,
                t.peak,
                [t.pages_ever[0].len(), t.pages_ever[1].len()],
                [t.frames_ever[0].len(), t.frames_ever[1].len()],
                r,
            )
        };
        for (i, w) in [FbWindow::FbAperture, FbWindow::InstanceWindow]
            .into_iter()
            .enumerate()
        {
            match self.arms[i] {
                Some(_) => eprintln!(
                    "kayfabe: BAR-MIRROR {} AT {at}: arm=on TRAP_FILLS={} premap_fills={} \
                     distinct_pages={} distinct_frames={} — ⇒ **TRAP_FILLS is the goal-2 \
                     number**: one guest MMIO exit each. ⊘ `premap_fills` are installed AHEAD \
                     of any access and cost the guest NO exit; they were summed into the same \
                     counter until w696, which made a ZERO trap count read as thousands. Read \
                     TRAP_FILLS beside the C's bar{}_passthrough_misses (a miss the mirror \
                     could not fill is in the refusal list below)",
                    name(w),
                    self.census.fills[i].load(Ordering::Relaxed),
                    self.census.premap_fills[i].load(Ordering::Relaxed),
                    pages[i],
                    frames[i],
                    i + 1,
                ),
                None => eprintln!(
                    "kayfabe: BAR-MIRROR {} AT {at}: arm=off (the control) — every access \
                     trapped",
                    name(w)
                ),
            }
        }
        // ★ w613 — the pre-birth share, printed beside the totals it qualifies.
        let birth = {
            let g = self
                .pages_at_first_birth
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            match *g {
                None => " pre_birth_pages=NO-BIRTH (no channel was ever born, so every page here is bring-up)".to_string(),
                Some((b1, b2)) => format!(
                    " pre_birth_pages=[bar1={b1} bar2={b2}] of [bar1={} bar2={}] => {}",
                    pages[0],
                    pages[1],
                    if b1 + b2 >= (pages[0] + pages[1]) * 3 / 4 {
                        "MOST pages were needed BEFORE any channel existed - map-at-create cannot cover them, and would measure no change"
                    } else {
                        "most pages arrive AFTER the first birth - map-at-create is aimed at the right traffic"
                    }
                ),
            }
        };
        let premap = format!(
            " premap[runs={} filled={} skipped={} refused={} biggest_leaf={} bar2_visited={} \
             pt_faults={}] arm[retries={} retried_ok={} gave_up={}]",
            self.premap_runs.load(Ordering::Relaxed),
            self.premap_pages.load(Ordering::Relaxed),
            self.premap_skipped.load(Ordering::Relaxed),
            self.premap_refused.load(Ordering::Relaxed),
            self.premap_biggest_leaf.load(Ordering::Relaxed),
            self.premap_bar2_visited.load(Ordering::Relaxed),
            self.premap_pt_faults.load(Ordering::Relaxed),
            self.arm_retries.load(Ordering::Relaxed),
            self.arm_retried_ok.load(Ordering::Relaxed),
            self.arm_gave_up.load(Ordering::Relaxed),
        );
        let (a_live, a_peak, a_recycled, a_issued) = self.arena.census();
        // ⊘ w585 — the STORE's census, not the allocator's. They answer different questions:
        // the allocator says how many pages it handed out, the store says how many it asked
        // for and was REFUSED. w584 lived entirely in the gap between them.
        // ⚠ **Only at teardown.** `fb_arena_census` takes the plane's memory lock, and this
        // report is emitted periodically during the boot — the same "an instrument is on a hot
        // path unless someone checked" mistake w586 made one crate over, and the same shape as
        // w516's *"the vCPU read a counter by locking every proc"*. The numbers are cumulative,
        // so the only emission that carries information is the last one.
        let (s_ref, s_mig, s_rref, s_rst) = if at.contains("END") {
            self.plane.fb_arena_census()
        } else {
            (0, 0, 0, 0)
        };
        let refused_s: Vec<String> = refused.iter().map(|(k, v)| format!("{k}={v}")).collect();
        eprintln!(
            "kayfabe: BAR-MIRROR MECHANISM AT {at}: slots live={live} peak={peak} \
             revalidate[runs={} kept={} removed={}] quiesce[calls={} removed={}] \
             retire_all[calls={} removed={}] arena[pages live={a_live} peak={a_peak} \
             allocations={a_recycled} span_pages={a_issued} store_refused={s_ref} \
             store_migrated={s_mig} store_read_refused={s_rref} store_resets={s_rst} (store numbers at END only)]{birth}{premap} refused=[{}]{}",
            self.census.reval_runs.load(Ordering::Relaxed),
            self.census.reval_kept.load(Ordering::Relaxed),
            self.census.reval_removed.load(Ordering::Relaxed),
            self.census.quiesce_calls.load(Ordering::Relaxed),
            self.census.quiesce_removed.load(Ordering::Relaxed),
            self.census.retire_all_calls.load(Ordering::Relaxed),
            self.census.retire_all_removed.load(Ordering::Relaxed),
            refused_s.join(" "),
            if refused.is_empty() {
                " ⊘ no refusal fired"
            } else {
                ""
            },
        );
    }
}

impl FbMirrorPort for BarMirror {
    fn revalidate_pending(&self) {
        BarMirror::revalidate_pending(self);
    }

    fn drain_fills(&self) {
        BarMirror::drain_fills(self);
    }


    fn quiesce(&self, phys: u64, len: u64) {
        let gone = {
            let mut t = self.table.lock().unwrap_or_else(|e| e.into_inner());
            t.quiesced.push((phys, len));
            Self::take_over_frames(&mut t, phys, len)
        };
        let n = self.retire(gone);
        self.drain_pending_over(phys, len);
        self.census.quiesce_calls.fetch_add(1, Ordering::Relaxed);
        self.census.quiesce_removed.fetch_add(n, Ordering::Relaxed);
    }

    fn resume(&self, phys: u64, len: u64) {
        let mut t = self.table.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = t.quiesced.iter().position(|(p, l)| *p == phys && *l == len) {
            t.quiesced.swap_remove(i);
        }
    }

    fn retire_all(&self, why: &'static str) {
        let gone: Vec<Retired> = {
            let mut t = self.table.lock().unwrap_or_else(|e| e.into_inner());
            let all: Vec<Retired> = t
                .slots
                .iter()
                .map(|(g, s)| Retired {
                    gpa: *g,
                    region: s.region,
                    view: s.view,
                })
                .collect();
            t.slots.clear();
            t.pending.clear();
            t.live = 0;
            all
        };
        let count = gone.len();
        let n = self.retire(gone);
        self.census.retire_all_calls.fetch_add(1, Ordering::Relaxed);
        self.census.retire_all_removed.fetch_add(n, Ordering::Relaxed);
        eprintln!(
            "kayfabe: BAR-MIRROR RETIRE-ALL [{why}]: {n} of {count} slot(s) removed — every \
             BAR page traps again until its next touch"
        );
    }
}

#[cfg(test)]
mod arena_unit_tests {
    use super::*;
    use kayfabe_device::fbwin::{FbStore, SparseFb};

    const FB_PAGE: u64 = 4096;

    /// ★★★★★ **THE STORE AND THE REAL ARENA MUST AGREE ON UNITS (w584).**
    ///
    /// ⊘⊘ `SharedPageArena::alloc_at` takes a framebuffer BYTE ADDRESS. Between w569 and w584
    /// the store passed it a FRAME NUMBER at four of five call sites. `alloc_at` refuses a
    /// misaligned address, so every store-side page creation was refused `ARENA_MISALIGNED` —
    /// unless the frame number happened to be a multiple of 4096, i.e. the address a multiple
    /// of 16 MiB — and fell back to the heap.
    ///
    /// ★ It survived the whole suite and a PASSING boot, because the heap fallback is
    /// functionally CORRECT for the trapping path: right bytes, wrong location. It breaks
    /// exactly one thing — a memory slot over the framebuffer, which can only see the file —
    /// so it surfaced as PRAMIN failing, nowhere near its cause.
    ///
    /// ⚠ Invisible to every existing test because they drive a MOCK arena, which accepts any
    /// value. **A unit error between two crates can only be caught by a test that spans both**,
    /// so this one uses the real `SharedPageArena` through the real `ArenaPort`.
    #[test]
    fn the_store_allocates_at_addresses_the_real_arena_accepts() {
        const FB: u64 = 64 << 20;
        let arena = SharedPageArena::create_for(FB, HostPageSize::query()).expect("arena");
        let mut fb = SparseFb::new(FB);
        fb.install_page_arena(Box::new(ArenaPort(arena.clone())))
            .expect("the store takes the arena");

        // Frames whose NUMBER is not a multiple of 4096 — every one was refused under w569.
        for frame in [0u64, 1, 2, 17, 255, 4095, 9001] {
            fb.write(frame * FB_PAGE, &[0xAB; 8]).expect("write");
        }

        let (refusals, _migrations) = fb.arena_census();
        assert_eq!(
            refusals, 0,
            "the store asked the arena for {refusals} page(s) it REFUSED — a unit mismatch: \
             `alloc_at` takes a framebuffer BYTE ADDRESS and the store passed a FRAME NUMBER, \
             so every page landed on the heap. Correct bytes, wrong location, invisible to any \
             memory slot over the framebuffer."
        );
        // ⊘ Non-vacuity: zero refusals means nothing if nothing was asked.
        assert!(fb.resident_pages() > 0, "no page was created");

        // ★ And the bytes read back — a page in the right place with the wrong contents is a
        // different failure, ruled out here because this is the cheapest place to do it.
        let mut buf = [0u8; 8];
        fb.read(0, &mut buf);
        assert_eq!(buf, [0xAB; 8], "the arena page holds what the store wrote");
    }

    /// ★★★★★ **THE FILE IS THE MEMORY — a write that never trapped must still be READ (w585).**
    ///
    /// ⊘ This is what a memory slot over the arena does: the guest writes a framebuffer
    /// address with **no exit**, so the bytes land in the `memfd` and the store never learns
    /// the frame exists. Simulated here by writing through the arena directly, which is the
    /// same memory the slot would map.
    ///
    /// Before w585 the store answered that read with ZEROS, because "no page" meant "nobody
    /// wrote it". That is the read half of the two-memories bug, and it is why PRAMIN over a
    /// slot could not work no matter how correctly the slot was placed.
    #[test]
    fn a_frame_written_only_through_the_file_reads_back_through_the_store() {
        const FB: u64 = 64 << 20;
        let arena = SharedPageArena::create_for(FB, HostPageSize::query()).expect("arena");
        let mut fb = SparseFb::new(FB);
        fb.install_page_arena(Box::new(ArenaPort(arena.clone())))
            .expect("install");

        // The untrapped write: straight into the file at the framebuffer address.
        const ADDR: u64 = 777 * FB_PAGE;
        let mut page = arena.alloc_at(ADDR).expect("the arena places it by address");
        page.write_from(0x40, &[0xC5; 16]).expect("file write");
        drop(page);

        let mut buf = [0u8; 16];
        fb.read(ADDR + 0x40, &mut buf);
        assert_eq!(
            buf,
            [0xC5; 16],
            "the store answered a frame it has no page for from the wrong memory. With an arena \
             installed every frame inside the framebuffer is implicitly resident IN THE FILE — \
             a memory slot lets the guest write it untrapped, so zeros here are an invention."
        );
        assert_eq!(fb.arena_read_refusals(), 0, "the arena refused the read");
    }

    /// ★★★★★ **A TRAPPED WRITE MUST NOT ERASE THE REST OF THE PAGE (w585).**
    ///
    /// ⊘ The write half of the same bug. `fresh_page` used to zero-fill a newly allocated
    /// arena page; under address indexing that page already held whatever the guest had put
    /// there through the slot, so the **first trapped write to it destroyed everything else in
    /// it**. Not a stale read — an active erase, by the store, of live guest memory.
    #[test]
    fn a_trapped_write_does_not_erase_what_the_file_already_held() {
        const FB: u64 = 64 << 20;
        let arena = SharedPageArena::create_for(FB, HostPageSize::query()).expect("arena");
        let mut fb = SparseFb::new(FB);
        fb.install_page_arena(Box::new(ArenaPort(arena.clone())))
            .expect("install");

        const ADDR: u64 = 1234 * FB_PAGE;
        let mut page = arena.alloc_at(ADDR).expect("place by address");
        page.write_from(0x800, &[0x77; 32]).expect("file write");
        drop(page);

        // Now the guest traps a write elsewhere in the SAME page — this is what creates the
        // store's page, and what used to zero the whole thing.
        fb.write(ADDR + 0x10, &[0x11; 4]).expect("trapped write");

        let mut far = [0u8; 32];
        fb.read(ADDR + 0x800, &mut far).expect("read the far half");
        assert_eq!(
            far,
            [0x77; 32],
            "the trapped write erased 4 KiB of guest memory it was not addressed to"
        );
        let mut near = [0u8; 4];
        fb.read(ADDR + 0x10, &mut near).expect("read the written half");
        assert_eq!(near, [0x11; 4], "the trapped write itself did not land");
    }

    /// ★★★★★ **A DEVICE RESET MUST REACH THE FILE (w585).**
    ///
    /// ⊘ `device_reset` clears the page map so one guest's framebuffer cannot be read by the
    /// next. Once the file is the memory, that clear wipes **nothing** — and a guard that
    /// still reads as a guard is worse than none. The arena is punched instead.
    #[test]
    fn a_device_reset_returns_the_file_to_zero() {
        const FB: u64 = 64 << 20;
        let arena = SharedPageArena::create_for(FB, HostPageSize::query()).expect("arena");
        let mut fb = SparseFb::new(FB);
        fb.install_page_arena(Box::new(ArenaPort(arena.clone())))
            .expect("install");

        const ADDR: u64 = 4000 * FB_PAGE;
        fb.write(ADDR, &[0xEE; 64]).expect("the first guest writes");
        let mut buf = [0u8; 64];
        fb.read(ADDR, &mut buf).expect("read");
        assert_eq!(buf, [0xEE; 64], "non-vacuity: the bytes were there to leak");

        fb.device_reset();

        // ⊘ Asked through the FILE, not through the store — the store's map is empty either
        // way, and the map being empty is precisely the thing that used to look like proof.
        let page = arena.alloc_at(ADDR).expect("place by address");
        let mut leaked = [0u8; 64];
        page.read_into(0, &mut leaked).expect("file read");
        assert_eq!(
            leaked,
            [0u8; 64],
            "the previous device life's framebuffer bytes are still in the file, readable by \
             the next guest through the same memory slot"
        );
    }
}

#[cfg(test)]
mod arm_then_retry_tests {
    //! ★★★★★ **CUT B's RETRY LOOP, and each of its three properties fired.**
    //!
    //! ⊘ These exist because the two production call sites cannot be built in a `cargo test` —
    //! a [`BarMirror`] needs a `QemuMachine` — so the part that can be wrong on its own was
    //! made a free function. ⚠ Every assertion here is a **known-positive**: each one fails if
    //! the loop stops doing the thing, rather than merely not crashing.

    use super::arm_then_retry;
    use std::cell::Cell;

    /// ★ **THE HAPPY PATH: it retries exactly as far as it has to, and no further.**
    #[test]
    fn it_stops_the_moment_the_attempt_is_good() {
        let n = Cell::new(0u32);
        let (v, used) = arm_then_retry(
            8,
            || {
                n.set(n.get() + 1);
                (n.get(), n.get() == 3)
            },
            || true,
        );
        assert_eq!(v, 3);
        assert_eq!(used, 2, "one initial attempt plus TWO retries, then it stops");
        assert_eq!(n.get(), 3, "and the attempt ran three times, not eight");
    }

    /// ⊘⊘⊘ **THE FIXED TRIP COUNT — §20's first structural invariant, fired.**
    ///
    /// The attempt never succeeds and the arm always claims progress: a loop that ended on the
    /// data would run forever, and a guest's own page tables are what feed the data.
    #[test]
    fn an_attempt_that_never_succeeds_costs_exactly_the_bound() {
        let n = Cell::new(0u32);
        let (_, used) = arm_then_retry(
            5,
            || {
                n.set(n.get() + 1);
                ((), false)
            },
            || true,
        );
        assert_eq!(used, 5, "★ the bound, exactly — not one more");
        assert_eq!(
            n.get(),
            6,
            "one initial attempt plus five retries. ⚠ If this ever becomes unbounded the \
             symptom is a WEDGED BOOT with no error, because every iteration looks like work."
        );
    }

    /// ★★★★★ **CUT B ITEM 5 — AN ARM THAT DID NOTHING ENDS THE LOOP, AND IT ENDS IT AT ONCE.**
    ///
    /// ⊘ *"Nothing was armed"* covers three states — the drain **declined** (a vCPU inside an
    /// MMIO exit), the aperture **refused**, and there is **no port**. Retrying helps in none
    /// of them, and the first is on the one thread that must never spin.
    #[test]
    fn an_arm_that_changes_nothing_ends_the_loop_immediately() {
        let attempts = Cell::new(0u32);
        let arms = Cell::new(0u32);
        let (_, used) = arm_then_retry(
            16,
            || {
                attempts.set(attempts.get() + 1);
                ((), false)
            },
            || {
                arms.set(arms.get() + 1);
                false
            },
        );
        assert_eq!(used, 0);
        assert_eq!(attempts.get(), 1, "the initial attempt, and nothing after it");
        assert_eq!(
            arms.get(),
            1,
            "★★★ THE KNOWN-POSITIVE: the arm is asked ONCE. A loop that could not tell \
             `armed nothing` from `try again` would have asked sixteen times — on a vCPU, \
             sixteen IPC round trips inside one MMIO exit."
        );
    }

    /// ⊘ **THE LAST VALUE COMES BACK ON GIVE-UP.** `window_leaves`' failing shape is an `Ok`
    /// that is SHORT, so a caller that got only a refusal could not report what it saw — and
    /// premap would print nothing and publish nothing.
    #[test]
    fn the_value_survives_a_give_up() {
        let n = Cell::new(0u32);
        let (v, used) = arm_then_retry(
            2,
            || {
                n.set(n.get() + 1);
                (format!("attempt {}", n.get()), false)
            },
            || true,
        );
        assert_eq!(used, 2);
        assert_eq!(v, "attempt 3", "the LAST value, not the first and not a default");
    }

    /// ⊘ A bound of zero is a caller that does not want a retry, and it must still attempt
    /// once. ⚠ Stated because `0` is the value a future gate would use to turn cut B off, and
    /// an off switch that skipped the attempt would break the arena arm.
    #[test]
    fn a_bound_of_zero_still_attempts_once() {
        let n = Cell::new(0u32);
        let (_, used) = arm_then_retry(
            0,
            || {
                n.set(n.get() + 1);
                ((), false)
            },
            || panic!("the arm must not be asked when no retry is allowed"),
        );
        assert_eq!(used, 0);
        assert_eq!(n.get(), 1);
    }
}
