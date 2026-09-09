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
    fn alloc(&mut self) -> Result<Box<dyn FbArenaPage>, &'static str> {
        self.0.alloc().map(|p| Box::new(ArenaPagePort(p)) as Box<dyn FbArenaPage>)
    }
}

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
    table: Mutex<Table>,
    census: Census,
    /// The plane's `UPDATE_BAR_PDE` count at the last check — a moved count is a BAR2 root
    /// change and revalidates the BAR2 half.
    last_bar_pde_updates: AtomicU64,
}

fn idx(w: FbWindow) -> Option<usize> {
    match w {
        FbWindow::FbAperture => Some(0),
        FbWindow::InstanceWindow => Some(1),
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
    pub fn arm(plane: Arc<RegPlane>, machine: QemuMachine) -> Option<Arc<BarMirror>> {
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
        let arena = match SharedPageArena::create(HostPageSize::query()) {
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
            plane,
            machine,
            arena,
            arms,
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
    pub fn fill(&self, w: FbWindow, off: u64) {
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
        let ticket = {
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
            ticket
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
            if mine && still && !Self::quiesced_covers(&t, key.phys) {
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
        t.pending.retain(|_, (_, p)| !(*p >= phys && *p < end));
        gone
    }

    /// ★★★★★ **THE FLUSH** — on the guest's own `MMU_INVALIDATE` trigger, or a BAR2 root
    /// publication: re-walk every live entry and drop the ones that no longer say what they
    /// said when installed. Kept entries cost one page walk each; dropped ones cost the
    /// ioctl. ⊘ Never a wholesale drop: a slot whose translation is unchanged after an
    /// invalidate is still exactly right, and the guest's next touch would only re-create it.
    pub fn revalidate(&self, why: &'static str) {
        let snapshot: Vec<(u64, Slot)> = {
            let t = self.table.lock().unwrap_or_else(|e| e.into_inner());
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
    pub fn after_write(&self, out: &kayfabe_device::WriteOutcome) {
        if let Some(inv) = &out.invalidate {
            if inv.trigger {
                self.revalidate("mmu-invalidate");
            }
        }
        let u = self.plane.bar_pde_counts().0;
        if self.last_bar_pde_updates.swap(u, Ordering::Relaxed) != u {
            self.revalidate("bar-pde-update");
        }
    }

    /// The census, one line per armed window plus one for the mechanism.
    pub fn report(&self, at: &str) {
        self.census.census_lines.fetch_add(1, Ordering::Relaxed);
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
        let refused_s: Vec<String> = refused.iter().map(|(k, v)| format!("{k}={v}")).collect();
        eprintln!(
            "kayfabe: BAR-MIRROR MECHANISM AT {at}: slots live={live} peak={peak} \
             revalidate[runs={} kept={} removed={}] quiesce[calls={} removed={}] \
             retire_all[calls={} removed={}] arena[pages live={a_live} peak={a_peak} \
             recycled={a_recycled} issued={a_issued}] refused=[{}]{}",
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
    fn quiesce(&self, phys: u64, len: u64) {
        let gone = {
            let mut t = self.table.lock().unwrap_or_else(|e| e.into_inner());
            t.quiesced.push((phys, len));
            Self::take_over_frames(&mut t, phys, len)
        };
        let n = self.retire(gone);
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
