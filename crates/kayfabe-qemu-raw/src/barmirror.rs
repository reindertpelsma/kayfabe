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
//! | fill on miss | a trapped BAR1/BAR2 access ([`BarMirror::fill`]) walks the guest's BAR page table (the same walk the trap performed), asks the store what memory backs the frame, and installs a 4 KiB memslot over **that memory** |
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

/// The page the mirror deals in — the store's `FB_PAGE`, the arena's `ARENA_PAGE`.
const PAGE: u64 = 4096;

/// The export token that names the page arena. Joins are numbered from 1.
const ARENA_TOKEN: u64 = 0;

/// How many refused fills are printed live per name (the total is uncapped).
const REFUSAL_LIVE: u64 = 4;

/// How many revalidation runs are printed live (the totals are uncapped).
const REVAL_LIVE: u64 = 8;

/// Print a running census every this many fills, so a boot that dies before teardown still
/// carries one.
const CENSUS_EVERY_FILLS: u64 = 512;

// ---- refusal names -----------------------------------------------------------------------

const R_OUT_OF_BAR: &str = "OUT-OF-BAR";
const R_TRANSLATION: &str = "TRANSLATION-REFUSED";
const R_NO_ADDRESS_MODEL: &str = "NO-ADDRESS-MODEL";
const R_HEAP_PAGE: &str = "HEAP-PAGE";
const R_STORE: &str = "STORE-REFUSED";
const R_JOIN_GONE: &str = "JOIN-GONE";
const R_QUIESCED: &str = "QUIESCED";
const R_PENDING: &str = "PENDING-ELSEWHERE";
const R_COVERED: &str = "ALREADY-COVERED";
const R_RACED: &str = "RACED-AND-DROPPED";
const R_INSTALL: &str = "INSTALL-REFUSED";

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
    fills: [AtomicU64; 2],
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
    pramin: Mutex<Option<(kayfabe_vmm::RamRegionId, u64)>>,
    /// How many times the latch moved and how many redundant writes were skipped. ⚠ Two
    /// numbers: *"never moved"* and *"wrote the same value forty times"* are different facts.
    pramin_moves: AtomicU64,
    pramin_skipped: AtomicU64,
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
        FbPageBacking::Heap | FbPageBacking::Refused(_) => return None,
    };
    Some(Key {
        phys: r.phys,
        token: e.token,
        offset: e.offset,
        readonly: r.read_only,
    })
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
        if plane
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
            pramin_at_install: AtomicU64::new(u64::MAX),
            pramin_gpa: AtomicU64::new(u64::MAX),
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
    pub fn fill(&self, w: FbWindow, off: u64) {
        if !self.defer_reval || !kayfabe_util::lockwitness::on_vcpu_thread() {
            self.fill_now(w, off);
            return;
        }
        let mut q = self.fills.lock().unwrap_or_else(|e| e.into_inner());
        if q.len() >= FILL_QUEUE_CAP {
            self.fills_dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        q.push_back((w, off));
        self.fills_queued.fetch_add(1, Ordering::Relaxed);
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
            self.fill_now(w, off);
        }
    }

    /// The census for the fill queue, one line.
    #[must_use]
    pub fn fill_census(&self) -> String {
        format!(
            "BAR-MIRROR FILLS queued={} run={} dropped={} (a dropped fill is a page that \
             keeps trapping, never a wrong value)",
            self.fills_queued.load(Ordering::Relaxed),
            self.fills_run.load(Ordering::Relaxed),
            self.fills_dropped.load(Ordering::Relaxed),
        )
    }

    fn fill_now(&self, w: FbWindow, off: u64) {
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

        // ---- 1. RESOLVE (plane lock, released on return) --------------------------------
        let res = match self.plane.window_page_backing(w, page_off, true) {
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
                _ => {}
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
                        "the join's descriptor left the registry between resolve and install",
                    );
                    return;
                }
            }
        };
        let region = match self
            .machine
            .install_file_window(gpa, PAGE, fd, key.offset, key.readonly)
        {
            Ok(r) => r,
            Err(e) => {
                self.cancel(gpa, ticket);
                self.refuse(w, off, refusal_name(&e), &format!("{e:?}"));
                return;
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
            let _ = self.machine.remove_window(region);
            self.refuse(
                w,
                off,
                R_RACED,
                "the page's translation or backing moved while the slot was being installed; \
                 the slot was dropped before it could serve anything",
            );
            return;
        }
        let n = self.census.fills[wi].fetch_add(1, Ordering::Relaxed) + 1;
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

    /// Remove `regions` from the machine, lock-free, and account for them.
    fn retire(&self, regions: Vec<(u64, RamRegionId)>) -> u64 {
        let mut n = 0;
        for (_, r) in regions {
            if self.machine.remove_window(r).is_ok() {
                n += 1;
            }
        }
        n
    }

    /// Take every slot whose frame lies in `[phys, phys+len)` out of the table (the caller
    /// removes them from the machine) and cancel the pendings there.
    fn take_over_frames(t: &mut Table, phys: u64, len: u64) -> Vec<(u64, RamRegionId)> {
        let end = phys.saturating_add(len);
        let gone: Vec<(u64, RamRegionId)> = t
            .slots
            .iter()
            .filter(|(_, s)| s.key.phys >= phys && s.key.phys < end)
            .map(|(g, s)| (*g, s.region))
            .collect();
        for (g, _) in &gone {
            t.slots.remove(g);
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
        let mut gone: Vec<(u64, RamRegionId)> = Vec::new();
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
                gone.push((gpa, s.region));
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
            match self.machine.install_file_window(
                p.base + span_off,
                span_len,
                self.arena.as_backing_fd(),
                base,
                // ⊘ NOT read-only: PRAMIN is the framebuffer, not a register file. `[measured
                // w577]` its writes are 631 458 of 635 162 accesses — a read-only slot would
                // remove the reads and leave the larger half exiting.
                false,
            ) {
                Ok(region) => {
                    *slot = Some((region, base));
                    self.pramin_moves.fetch_add(1, Ordering::Relaxed);
                    // w593 - the prefix, latched once, before any post-install access.
                    let c = self.plane.counters();
                    self.pramin_at_install
                        .store(c.fb_reads + c.fb_writes, Ordering::Relaxed);
                    self.pramin_gpa.store(p.base + span_off, Ordering::Relaxed);
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
        let Some((region, shown)) = *slot else {
            return;
        };
        if shown == base {
            self.pramin_skipped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        match self
            .machine
            .repoint_file_window(region, self.arena.as_backing_fd(), base)
        {
            Ok(()) => {
                *slot = Some((region, base));
                self.pramin_moves.fetch_add(1, Ordering::Relaxed);
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

    pub fn after_write(&self, out: &kayfabe_device::WriteOutcome) {
        // ★ w578 — the latch first: it is synchronous and cheap, and everything below defers.
        if out.claimed {
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

    /// The census, one line per armed window plus one for the mechanism.
    pub fn report(&self, at: &str) {
        self.census.census_lines.fetch_add(1, Ordering::Relaxed);
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
                .map_or_else(|| "none".to_string(), |(_, b)| format!("0x{b:x}"));
            let c = self.plane.counters();
            let total = c.fb_reads + c.fb_writes;
            let pre = self.pramin_at_install.load(Ordering::Relaxed);
            // ★ w595 — where the slot went in, against where BAR0 is NOW.
            let at = self.pramin_gpa.load(Ordering::Relaxed);
            let now = self
                .machine
                .bar_placement(BarId::Bar0)
                .and_then(|p| self.plane.pramin_span().map(|(off, _)| p.base + off));
            let placement = match (at, now) {
                (u64::MAX, _) => " gpa=none".to_string(),
                (a, Some(n)) if a == n => format!(" gpa=0x{a:x} (BAR0 has not moved since)"),
                (a, Some(n)) => format!(
                    " gpa=0x{a:x} but the aperture is NOW at 0x{n:x} => BAR0 MOVED UNDER THE SLOT;                      the slot is serving an address nothing accesses and every exit follows from                      that, not from the slot being wrong"
                ),
                (a, None) => format!(" gpa=0x{a:x} (BAR0 is unplaced now)"),
            };
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
            eprintln!(
                "kayfabe: PRAMIN-SLOT AT {at}: moves={} skipped={} showing={shown} window_accesses={total} {split}{placement} \u{2298} moves+skipped below the guest's latch-write count means a re-point was REFUSED and the guest read the wrong framebuffer.",
                self.pramin_moves.load(Ordering::Relaxed),
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
                    "kayfabe: BAR-MIRROR {} AT {at}: arm=on fills={} distinct_pages={} \
                     distinct_frames={} — ⇒ every fill was ONE trapped access; read fills \
                     beside the C's bar{}_passthrough_misses (a miss the mirror could not fill \
                     is in the refusal list below)",
                    name(w),
                    self.census.fills[i].load(Ordering::Relaxed),
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
             store_migrated={s_mig} store_read_refused={s_rref} store_resets={s_rst} (store numbers at END only)] refused=[{}]{}",
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
        let gone: Vec<(u64, RamRegionId)> = {
            let mut t = self.table.lock().unwrap_or_else(|e| e.into_inner());
            let all: Vec<(u64, RamRegionId)> =
                t.slots.iter().map(|(g, s)| (*g, s.region)).collect();
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
