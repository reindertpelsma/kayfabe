//! ★★★★★ **P4 in the device — the memory plane's guest-facing pieces** (`V3_P4_PORT_MAP.md` §2.0-§2.4,
//! build-order rows 1-5).
//!
//! | piece | disposition | who touches it |
//! |---|---|---|
//! | PRAMIN (`BAR0 0x700000`, 1 MiB) | **A**: one RAM range under one memslot, no exit | the vCPU re-points it in the trapped window-base write: ONE host map + ONE `mmap` per move (owner ruling 2026-09-25, [`PraminPool`]); old views released by a reaper thread |
//! | BAR2 (PCI BAR3) | **A**: one RAM range, scratch where nothing is mapped | the VA-manager thread, after the GPU walker walked OUR BAR2 root ([`CpuWindow`] as the reconcile's target) |
//! | BAR1 | **A**: one RAM range, scratch where nothing is mapped | the VA-manager thread, after the GPU walker walked OUR BAR1 root (P5: [`K_BAR1`]) |
//! | the three `MMU_INVALIDATE` registers | **B** | the vCPU arms + wakes ([`InvalidatePort`]); the VA-manager thread walks, reconciles, and only then clears |
//!
//! ## Threads (THE_ARCHITECTURE_v3.md §1)
//!
//! - **vCPU**: [`MemPlane::invalidate_write`] (atomics + one eventfd write) and
//!   [`MemPlane::pramin_write`] (`mmap(MAP_FIXED)` placements of already-armed views: no lock, no
//!   RM ioctl, no allocation — Q2).
//! - **VA manager** ([`crate::device::Device::va_loop`]): the ONE thread that owns the GPU walker
//!   and the [`VaManager`]; it waits in `epoll` on its inbox wake and the walker's completion fd.
//! - **register drainer**: pushes the guest's address-space statements into the inbox
//!   ([`Inbox`]) and delivers their HELD replies once the VA manager has settled them.
//!
//! ## ⊘ What is never done here
//!
//! No CPU read of a guest page table, no mirror of one: every walk is the GPU walker's, the only
//! previous state is OUR ledger. No byte of guest memory is copied: every window page is a
//! placement of the store, of guest RAM, or of the window's scratch (never a hole — a memslot over
//! an unmapped range kills the guest). No host flag is forwarded: every verb is authored here.
//!
//! ⊘ **Scratch is TILED (2026-10-03)**: one small memfd per window ([`ScratchTile`]), mapped again
//! and again, so a guest touching every unmapped page costs the host at most one tile per window
//! (2 MiB for BAR1 and BAR2 at their default sizes, 1 MiB for PRAMIN) instead of the whole window.
//! See [`window_with_scratch`] and `V3_P4_PORT_MAP.md` Q3.
//!
//! ## BAR1
//!
//! ★ P5: walked from OUR BAR1 root exactly like BAR2 ([`K_BAR1`]): the guest writes its BAR1 PDEs
//! straight into that page (no RPC — `THE_OGKM_RESIDUE.md` §3) and invalidates it. `[measured
//! p5h]` left on scratch, it swallowed the PMA scrubber's `GP_PUT` (its USERD is mapped through
//! BAR1). ⚠ Each placed view costs host BAR1 aperture (`V3_P4_PORT_MAP.md` Q5 — the budget check
//! is still not built).

// ★ Review addendum (2026-10-10, `V3_BATCHED_MAP.md` §8.8.9): THE GUEST IS UNTRUSTED and this module
// holds the rows it places (VAs, lengths, backing offsets from its page tables). A panic here is a
// denial of service, so guest-derived values are only added/subtracted with `checked_*` /
// `saturating_*`, and nothing unwraps. A new panic site fails CI.
#![cfg_attr(
    not(test),
    deny(
        clippy::arithmetic_side_effects,
        clippy::indexing_slicing,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable
    )
)]

use crate::raw_unsafe::{BackendFd, RawRegion};
use kf_host::{CpuViewRelease, HostRm, MapNode, ViewAccess};
use kf_linux_raw::{
    Backing, CharDevice, GuestWindow, HostOffset, HostPageSize, Notifier, RawError, ScratchTile,
};
use kf_mem::cpuwin::{CpuWindow, PraminPool, SlotSource, ViewOps};
use kf_mem::ledger::{Desired, HostVas, MapTarget, Mapped, Settle, UsermodeRow};
use kf_mem::vasmgr::{GpuWalker, VaManager, VasKey};
use kf_rm::barpde::{BarAperture, MemStatement};
use kf_trap::pramin::{GRANULE, SLOTS, Target as WinTarget, WindowReg};
use kf_trap::{InvalidatePort, PdbAperture, PortWrite};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};

/// OUR BAR2 aperture's object key in the VA table — outside every guest `(hClient << 32) | hObject`
/// (a guest client handle is never `0xFFFF_FFFF`, RM's reserved value).
pub const K_BAR2: VasKey = VasKey(0xFFFF_FFFF_0000_0002);
/// ★ P5: OUR BAR1 aperture's key (`V3_P4_PORT_MAP.md` §3 row 6). `[measured p5h]` the PMA
/// scrubber's USERD is reached through BAR1 (`bUseBar1` ⇒ `TRANSFER_FLAGS_USE_BAR1`,
/// `mem_utils_gm107.c:1110-1113`), so a BAR1 left on scratch swallowed its `GP_PUT`: 19 doorbells,
/// `GP_PUT` read 0, `scrubberDestruct` timed out.
pub const K_BAR1: VasKey = VasKey(0xFFFF_FFFF_0000_0001);

/// One guest-RAM block QEMU registered: guest-physical `[gpa, gpa+len)` at `mem`, backed by the
/// memory backend's `fd` at `fd_off` (`None` when the backend has no fd).
#[derive(Debug, Clone, Copy)]
pub struct RamBlock {
    /// Guest-physical base.
    pub gpa: u64,
    /// The host mapping.
    pub mem: RawRegion,
    /// The backend fd, if the backend has one.
    pub fd: Option<BackendFd>,
    /// File offset of `gpa`.
    pub fd_off: u64,
}

/// ★ Guest RAM, as QEMU registered it — the VMM's own guest-physical layout, which is NOT the
/// identity once there is a PCI hole.
#[derive(Debug)]
pub struct RamMap {
    blocks: RwLock<Vec<RamBlock>>,
    generation: AtomicU64,
}

impl Default for RamMap {
    fn default() -> Self {
        Self {
            blocks: RwLock::new(Vec::new()),
            generation: AtomicU64::new(1),
        }
    }
}

impl RamMap {
    // Called with the registration write guard held. Zero is terminal: an exhausted
    // counter can never revive a descriptor from an earlier topology incarnation.
    fn revoke(&self) {
        let old = self.generation.load(Ordering::Relaxed);
        self.generation.store(
            if old == 0 {
                0
            } else {
                old.checked_add(1).unwrap_or(0)
            },
            Ordering::Relaxed,
        );
    }

    /// Register (or replace) the block at `b.gpa`.
    pub fn add(&self, b: RamBlock) {
        if let Ok(mut v) = self.blocks.write() {
            self.revoke();
            v.retain(|x| x.gpa != b.gpa);
            v.push(b);
            v.sort_by_key(|x| x.gpa);
        }
    }

    /// Unregister the block at `gpa`.
    pub fn del(&self, gpa: u64) {
        if let Ok(mut v) = self.blocks.write() {
            self.revoke();
            v.retain(|x| x.gpa != gpa);
        }
    }

    /// The ONE backend fd every fd-backed block shares, if any.
    #[must_use]
    pub fn backing_fd(&self) -> Option<BackendFd> {
        self.blocks.read().ok()?.iter().find_map(|b| b.fd)
    }

    /// The block wholly covering `[gpa, gpa+len)`.
    #[must_use]
    pub fn block_for(&self, gpa: u64, len: u64) -> Option<RamBlock> {
        let v = self.blocks.read().ok()?;
        v.iter()
            .find(|b| {
                gpa >= b.gpa
                    && gpa
                        .checked_add(len)
                        .is_some_and(|e| e.saturating_sub(b.gpa) <= b.mem.len() as u64)
            })
            .copied()
    }

    /// ★ P5: the host mapping of guest-RAM memfd bytes `[off, off+len)` — the inverse of
    /// [`Self::file_range`], for reading a sysmem row the reconcile placed (whose offset is a FILE
    /// offset). `None` when no single fd-backed block covers it.
    #[must_use]
    pub fn at_file_offset(&self, off: u64, len: u64) -> Option<(RawRegion, usize)> {
        let v = self.blocks.read().ok()?;
        let first = v.iter().find_map(|x| x.fd)?;
        v.iter()
            .filter(|b| b.fd == Some(first))
            .find(|b| {
                off >= b.fd_off
                    && off
                        .checked_add(len)
                        .is_some_and(|e| e.saturating_sub(b.fd_off) <= b.mem.len() as u64)
            })
            .map(|b| (b.mem, off.saturating_sub(b.fd_off) as usize))
    }

    /// ★ The guest-RAM object's fd and the file offset of `[gpa, gpa+len)`. `None` when no single
    /// fd-backed block covers it. ⊘ Every block must share ONE fd (q35's `memory-backend=ram0`
    /// aliases one memfd above and below the hole); a second fd is refused, never guessed at.
    #[must_use]
    pub fn file_range(&self, gpa: u64, len: u64) -> Option<(BackendFd, u64)> {
        let b = self.block_for(gpa, len)?;
        let first = self.blocks.read().ok()?.iter().find_map(|x| x.fd)?;
        (b.fd == Some(first))
            .then(|| b.fd_off.checked_add(gpa.saturating_sub(b.gpa)))
            .flatten()
            .map(|o| (first, o))
    }

    /// ★★ P1+P2 (`docs/design/V3_P1P2_TSPACE.md` §2.6) — **THE vIOMMU SEAM: the one path from a
    /// device DMA address to a guest-memfd range.** The walker's sysmem leaves, the Translated
    /// rewriter's sysmem operands (both the window arithmetic and the T-mode resolver), and sysmem
    /// USERD and notifiers all go through here. Today a device DMA address IS a guest-physical
    /// address, so this is [`RamMap::file_range`]'s offset; under a guest vIOMMU it does IOVA→GPA
    /// per run FIRST, splitting a run where the mapping is discontiguous and refusing — never
    /// reading through — an unmapped one (`V3_VIOMMU.md` §3). Until that lands kf3 refuses at
    /// realize behind a vIOMMU. ⚠ The PRAMIN plan's closure (`MemPlane::build`) is routed here when
    /// `v3-scratch-bound` (which rewrites that construction) has merged.
    #[must_use]
    pub fn dma_to_file_range(&self, dma: u64, len: u64) -> Option<u64> {
        self.file_range(dma, len).map(|(_, off)| off)
    }
}

/// The checked CPU view authority for MemoryList registration. The QEMU listener
/// supplies only writable real RAM (no ROM or RAM-device/MMIO). No raw span escapes
/// the read guard here; unregister waits for the bounded copy before freeing backing.
pub struct MemoryListRam(pub &'static RamMap);
impl core::fmt::Debug for MemoryListRam {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("MemoryListRam(..)")
    }
}

fn memory_list_block(blocks: &[RamBlock], span: kf_rm::memory_list::RamSpan) -> Option<&RamBlock> {
    if span.length == 0 {
        return None;
    }
    let end = span.base.checked_add(span.length)?;
    let first = blocks.iter().find_map(|b| b.fd)?;
    let mut overlaps = blocks.iter().filter(|b| {
        b.gpa
            .checked_add(b.mem.len() as u64)
            .is_some_and(|e| span.base < e && b.gpa < end)
    });
    let b = overlaps.next()?;
    if overlaps.next().is_some()
        || b.fd != Some(first)
        || span.base < b.gpa
        || end.checked_sub(b.gpa)? > b.mem.len() as u64
    {
        return None;
    }
    b.fd_off
        .checked_add(span.base.saturating_sub(b.gpa))?
        .checked_add(span.length)?;
    Some(b)
}

impl kf_rm::memory_list::GuestRamAuthority for MemoryListRam {
    fn validate(&self, span: kf_rm::memory_list::RamSpan) -> Option<u64> {
        let guard = self.0.blocks.read().ok()?;
        memory_list_block(&guard, span)?;
        let generation = self.0.generation.load(Ordering::Relaxed);
        (generation != 0).then_some(generation)
    }
    fn read(
        &self,
        span: kf_rm::memory_list::RamSpan,
        generation: u64,
        offset: u64,
        bytes: &mut [u8],
    ) -> bool {
        if bytes.len() > kf_rm::memory_list::MAX_ACCESS
            || !offset
                .checked_add(bytes.len() as u64)
                .is_some_and(|end| end <= span.length)
        {
            return false;
        }
        let Ok(guard) = self.0.blocks.read() else {
            return false;
        };
        if generation == 0 || self.0.generation.load(Ordering::Relaxed) != generation {
            return false;
        }
        let Some(b) = memory_list_block(&guard, span) else {
            return false;
        };
        let Some(at) = span
            .base
            .saturating_sub(b.gpa)
            .checked_add(offset)
            .and_then(|v| usize::try_from(v).ok())
        else {
            return false;
        };
        b.mem.read_into(at, bytes)
    }
    fn write(
        &self,
        span: kf_rm::memory_list::RamSpan,
        generation: u64,
        offset: u64,
        bytes: &[u8],
    ) -> bool {
        if bytes.len() > kf_rm::memory_list::MAX_ACCESS
            || !offset
                .checked_add(bytes.len() as u64)
                .is_some_and(|end| end <= span.length)
        {
            return false;
        }
        let Ok(guard) = self.0.blocks.read() else {
            return false;
        };
        if generation == 0 || self.0.generation.load(Ordering::Relaxed) != generation {
            return false;
        }
        let Some(b) = memory_list_block(&guard, span) else {
            return false;
        };
        let Some(at) = span
            .base
            .saturating_sub(b.gpa)
            .checked_add(offset)
            .and_then(|v| usize::try_from(v).ok())
        else {
            return false;
        };
        b.mem.write_from(at, bytes)
    }
}

/// ★ An armed CPU view of a store slice: the node whose `mmap` context RM registered, and the
/// `pLinearAddress` cookie its release is keyed by.
#[derive(Debug)]
pub struct StoreView {
    node: CharDevice,
    cookie: u64,
    /// ★ v3-initrace: the store offset this view shows (the probe's view index).
    off: u64,
}

/// ★★ v3-initrace — **which guest CPU window page shows which store page** (BAR1, BAR2, PRAMIN),
/// kept ONLY while `KF3_COMPLETION_PROBE` is set, so a probe can say which view a guest CPU access
/// to a framebuffer page went through and read what the guest sees there. Updated by
/// [`WindowOps`] as it places and sinks. ⊘ `try_lock` only — a PRAMIN re-point runs on a vCPU, and
/// a contended update is dropped and counted rather than waited for.
pub struct ViewIndex {
    inner: Mutex<ViewIndexInner>,
    /// Updates dropped because the index was busy.
    pub dropped: AtomicU64,
}

#[derive(Default)]
struct ViewIndexInner {
    /// Store page → every (window, window page) showing it.
    by_store: std::collections::BTreeMap<u64, Vec<(&'static str, &'static GuestWindow, u64)>>,
    /// (window, window page) → the store page it shows.
    by_win: std::collections::BTreeMap<(&'static str, u64), u64>,
}

const VIEW_PAGE: u64 = 0x1000;

impl ViewIndex {
    fn with(&self, f: impl FnOnce(&mut ViewIndexInner)) {
        match self.inner.try_lock() {
            Ok(mut g) => f(&mut g),
            Err(_) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn forget(g: &mut ViewIndexInner, name: &'static str, at: u64, len: u64) {
        let pages: Vec<(&'static str, u64)> = g
            .by_win
            .range((name, at & !(VIEW_PAGE - 1))..(name, at.saturating_add(len)))
            .map(|(k, _)| *k)
            .collect();
        for k in pages {
            if let Some(sp) = g.by_win.remove(&k)
                && let Some(v) = g.by_store.get_mut(&sp)
            {
                v.retain(|(n, _, w)| !(*n == k.0 && *w == k.1));
                if v.is_empty() {
                    g.by_store.remove(&sp);
                }
            }
        }
    }

    fn placed(
        &self,
        name: &'static str,
        win: &'static GuestWindow,
        at: u64,
        len: u64,
        store_off: u64,
    ) {
        self.with(|g| {
            Self::forget(g, name, at, len);
            let mut p = 0;
            while p < len {
                let (wp, sp) = (
                    at.saturating_add(p) & !(VIEW_PAGE - 1),
                    store_off.saturating_add(p) & !(VIEW_PAGE - 1),
                );
                g.by_win.insert((name, wp), sp);
                g.by_store.entry(sp).or_default().push((name, win, wp));
                p = p.saturating_add(VIEW_PAGE);
            }
        });
    }

    fn sunk(&self, name: &'static str, at: u64, len: u64) {
        self.with(|g| Self::forget(g, name, at, len));
    }

    /// Every guest window position showing store offset `off`, and the `n` bytes the guest reads
    /// there now (`None`: the read was refused).
    pub fn guest_views(&self, off: u64, n: usize) -> Vec<(&'static str, u64, Option<Vec<u8>>)> {
        let Ok(g) = self.inner.try_lock() else {
            return vec![("(index busy)", 0, None)];
        };
        let Some(v) = g.by_store.get(&(off & !(VIEW_PAGE - 1))) else {
            return Vec::new();
        };
        v.iter()
            .map(|(name, win, wp)| {
                let at = wp.saturating_add(off & (VIEW_PAGE - 1));
                let mut b = vec![0u8; n];
                let got = win.read_into(HostOffset::new(at), &mut b).ok().map(|()| b);
                (*name, at, got)
            })
            .collect()
    }
}

/// The probe's view index — `None` unless `KF3_COMPLETION_PROBE` is set.
#[must_use]
pub fn view_index() -> Option<&'static ViewIndex> {
    static IDX: std::sync::OnceLock<ViewIndex> = std::sync::OnceLock::new();
    crate::chan::completion_probe_ms()?;
    Some(IDX.get_or_init(|| ViewIndex {
        inner: Mutex::new(ViewIndexInner::default()),
        dropped: AtomicU64::new(0),
    }))
}

/// ★ The host verbs of ONE guest window — [`ViewOps`] over the real session.
pub struct WindowOps {
    /// ★ v3-initrace: the window's name, for the probe's view index ("BAR1", "BAR2", "PRAMIN").
    name: &'static str,
    rm: &'static HostRm,
    store: u32,
    win: &'static GuestWindow,
    /// ★ The window's scratch tile (2026-10-03): every hole shows it, at `offset % tile`.
    scratch: &'static ScratchTile,
    ram: &'static RamMap,
    /// PRAMIN only: nodes opened ahead of time and the reaper that releases retired views.
    trap: Option<&'static TrapNodes>,
    /// ★ 2026-10-03: whether this window's sinks and guest-RAM placements carry
    /// [`kf_linux_raw::WINDOW_ADVICE`]. BAR1 and BAR2: yes (VA thread). PRAMIN: no, its verbs run
    /// on the vCPU, which gets ONE `mmap` per move and no `madvise` (owner ruling 2026-09-25); it
    /// needs none, since every move re-places all 16 slots.
    advise: bool,
}

/// ★ Whether a window's verbs carry [`kf_linux_raw::WINDOW_ADVICE`] (2026-10-03): only a window
/// with no trap. A window with a trap (PRAMIN) runs its verbs on the vCPU, which gets ONE `mmap` per
/// move and no `madvise` (owner ruling 2026-09-25); BAR1 and BAR2 run theirs on the VA thread.
#[must_use]
pub const fn window_advises(has_trap: bool) -> bool {
    !has_trap
}

/// ★ Window sinks and guest-RAM placements whose `madvise` the kernel refused (2026-10-03), all
/// windows of the process. Either way the mapping itself landed, so the operation is reported as
/// done: a refused advice costs only the merge (that range stays its own mapping until a later
/// advised sink covers it) or, on a guest-RAM placement, its flags while it lives.
pub static WINDOW_ADVICE_REFUSED: AtomicU64 = AtomicU64::new(0);

/// Count one refused window advice, and log the first of the process.
fn advice_refused(what: &str, at: u64, len: u64, e: &RawError) {
    if WINDOW_ADVICE_REFUSED.fetch_add(1, Ordering::Relaxed) == 0 {
        eprintln!("kf3: {what} @{at:#x}+{len:#x}: advice refused ({e:?}), counted");
    }
}

/// The advice an advised sink applies after its cover; production's is
/// [`kf_linux_raw::advise_window`].
type Advice = fn(&GuestWindow, HostOffset, u64) -> Result<(), RawError>;

/// ★★ Re-point window `[at, at + len)` to scratch: the one sink door of every window (2026-10-03).
///
/// `advise` (BAR1, BAR2, on the VA thread): [`ScratchTile::cover`], then
/// [`kf_linux_raw::advise_window`] over the same range, so the sunk range carries the same flags as
/// the tiling around it and merges back into it; without that, every guest-driven place-then-sink
/// cycle left two more mappings for the VM's life (`kf_linux_raw::scratch`, module docs). Not
/// `advise` (PRAMIN, on the vCPU): the cover alone, ONE `mmap` (the window is its own tile).
///
/// ⊘ **Corrected 2026-10-03 (third review): once the cover landed, a refused advice is counted
/// ([`WINDOW_ADVICE_REFUSED`]) and the sink is `Ok`.** It was returned as a refused sink, so
/// `CpuWindow::unmap` kept a view the guest could no longer reach and the VA manager held the
/// guest's invalidate armed until a re-walk: a guest-visible stall for a flag. The cover alone
/// makes the view unreachable; only the merge waits for a later advised sink over that range.
///
/// `mmap`s: `ceil((at % T + len) / T)`, at most `ceil(len / T) + 1`, plus one retry only after a
/// refusal. Returns that count.
///
/// # Errors
/// The cover's refusal, by name. Never the advice's.
pub fn window_sink(
    win: &GuestWindow,
    scratch: &ScratchTile,
    advise: bool,
    at: u64,
    len: u64,
) -> Result<usize, String> {
    let advice: Advice = kf_linux_raw::advise_window;
    sink_with(win, scratch, advise.then_some(advice), at, len)
}

/// [`window_sink`] with the advice as an argument (`None`: no advice).
fn sink_with(
    win: &GuestWindow,
    scratch: &ScratchTile,
    advice: Option<Advice>,
    at: u64,
    len: u64,
) -> Result<usize, String> {
    let mmaps = scratch
        .cover(win, HostOffset::new(at), len)
        .map_err(|e| format!("mmap scratch @{at:#x}+{len:#x}: {e:?}"))?;
    if let Some(advise) = advice
        && let Err(e) = advise(win, HostOffset::new(at), len)
    {
        advice_refused("window sink", at, len, &e);
    }
    Ok(mmaps)
}

/// ★ The PRAMIN trap's helpers: device nodes opened AHEAD of time (so the trap's map is exactly one
/// ioctl), and a reaper thread that releases retired views and refills the node pool — both OFF
/// the vCPU.
pub struct TrapNodes {
    nodes: Mutex<std::sync::mpsc::Receiver<CharDevice>>,
    retire: Mutex<std::sync::mpsc::Sender<StoreView>>,
    /// Trap-time `openat`s because the pool was empty (should stay 0).
    pub inline_opens: AtomicU64,
    /// Views released by the reaper.
    pub reaped: AtomicU64,
    /// Releases the host refused (the aperture leaked; counted).
    pub reap_refused: AtomicU64,
}

/// Nodes kept ready for the trap. One is taken per store window move and one is returned to the
/// reaper by the view it replaces, so a small pool never drains.
const TRAP_NODES: usize = 4;

impl TrapNodes {
    /// Open the pool and start the reaper thread.
    ///
    /// # Errors
    /// The first `openat` or the thread spawn, by name.
    pub fn start(rm: &'static HostRm, store: u32) -> Result<&'static TrapNodes, String> {
        let (node_tx, node_rx) = std::sync::mpsc::sync_channel::<CharDevice>(TRAP_NODES);
        let (retire_tx, retire_rx) = std::sync::mpsc::channel::<StoreView>();
        let open = move || rm.open_view_node(MapNode::Gpu, ViewAccess::ReadWrite);
        for _ in 0..TRAP_NODES {
            node_tx
                .try_send(open().map_err(|e| format!("PRAMIN node: {e:?}"))?)
                .map_err(|e| format!("{e}"))?;
        }
        let t: &'static TrapNodes = Box::leak(Box::new(TrapNodes {
            nodes: Mutex::new(node_rx),
            retire: Mutex::new(retire_tx),
            inline_opens: AtomicU64::new(0),
            reaped: AtomicU64::new(0),
            reap_refused: AtomicU64::new(0),
        }));
        std::thread::Builder::new()
            .name("kf3-pramin-reaper".into())
            .spawn(move || {
                while let Ok(v) = retire_rx.recv() {
                    let r = rm.release_cpu_view(CpuViewRelease {
                        h_memory: store,
                        p_linear_address: v.cookie,
                    });
                    drop(v.node);
                    if r.is_ok() {
                        t.reaped.fetch_add(1, Ordering::Relaxed);
                    } else {
                        t.reap_refused.fetch_add(1, Ordering::Relaxed);
                    }
                    while let Ok(n) = open() {
                        if node_tx.try_send(n).is_err() {
                            break;
                        }
                    }
                }
            })
            .map_err(|e| format!("PRAMIN reaper thread: {e}"))?;
        Ok(t)
    }
}

impl ViewOps for WindowOps {
    type View = StoreView;

    fn arm_store(&self, off: u64, len: u64) -> Result<StoreView, String> {
        let (node, cookie) = self
            .rm
            .arm_cpu_view(MapNode::Gpu, self.store, off, len, ViewAccess::ReadWrite)
            .map_err(|e| format!("NV_ESC_RM_MAP_MEMORY store@{off:#x}+{len:#x}: {e:?}"))?;
        Ok(StoreView { node, cookie, off })
    }

    fn place_view(&self, at: u64, len: u64, v: &StoreView) -> Result<(), String> {
        self.win
            .place_device_view(HostOffset::new(at), len, v.node.as_fd(), true)
            .map_err(|e| format!("mmap view @{at:#x}+{len:#x}: {e:?}"))?;
        if let Some(ix) = view_index() {
            ix.placed(self.name, self.win, at, len, v.off);
        }
        Ok(())
    }

    fn place_ram(&self, at: u64, len: u64, file_off: u64) -> Result<(), String> {
        let fd = self
            .ram
            .blocks
            .read()
            .ok()
            .and_then(|v| v.iter().find_map(|b| b.fd))
            .ok_or("no fd-backed guest RAM (the VM needs memory-backend-memfd,share=on)")?;
        self.win
            .place(
                HostOffset::new(at),
                len,
                Backing::SharedFile {
                    fd: fd.borrow(),
                    offset: file_off,
                },
            )
            .map_err(|e| format!("mmap guest RAM @{at:#x}+{len:#x} (file {file_off:#x}): {e:?}"))?;
        // ★ 2026-10-03: the window's flags (VA thread only). `DONTDUMP` keeps a guest-RAM page
        // out of QEMU's core dump through this alias too (`dump-guest-core=off`). A refusal is
        // counted, not returned: the placement landed, and a RAM mapping never merges with scratch
        // (another file), so a missing flag costs no mapping once it is sunk.
        if self.advise
            && let Err(e) = kf_linux_raw::advise_window(self.win, HostOffset::new(at), len)
        {
            advice_refused(&format!("{} guest RAM", self.name), at, len, &e);
        }
        if let Some(ix) = view_index() {
            ix.sunk(self.name, at, len);
        }
        Ok(())
    }

    /// ★ [`window_sink`]: exactly ONE `mmap` for any PRAMIN run (the window is its own tile, and
    /// no advice on the vCPU); on BAR1 and BAR2 (VA thread) `ceil((at % T + len) / T)` `mmap`s,
    /// at most `ceil(len / T) + 1`, then the window's advice (a refused advice is counted, and the
    /// sink still landed).
    fn sink(&self, at: u64, len: u64) -> Result<(), String> {
        window_sink(self.win, self.scratch, self.advise, at, len)?;
        if let Some(ix) = view_index() {
            ix.sunk(self.name, at, len);
        }
        Ok(())
    }

    fn release(&self, v: StoreView) -> Result<(), String> {
        let r = self.rm.release_cpu_view(CpuViewRelease {
            h_memory: self.store,
            p_linear_address: v.cookie,
        });
        drop(v.node);
        r.map_err(|e| format!("NV_ESC_RM_UNMAP_MEMORY: {e:?}"))
    }

    /// ★ The trap's ONE RM call: `NV_ESC_RM_MAP_MEMORY` on a node opened ahead of time.
    fn arm_store_in_trap(&self, off: u64, len: u64) -> Result<StoreView, String> {
        let Some(t) = self.trap else {
            return self.arm_store(off, len);
        };
        let node = t.nodes.lock().ok().and_then(|rx| rx.try_recv().ok());
        let node = match node {
            Some(n) => n,
            None => {
                t.inline_opens.fetch_add(1, Ordering::Relaxed);
                self.rm
                    .open_view_node(MapNode::Gpu, ViewAccess::ReadWrite)
                    .map_err(|e| format!("PRAMIN node (inline): {e:?}"))?
            }
        };
        let (node, cookie) = self
            .rm
            .arm_cpu_view_on(node, self.store, off, len, ViewAccess::ReadWrite)
            .map_err(|e| format!("NV_ESC_RM_MAP_MEMORY store@{off:#x}+{len:#x}: {e:?}"))?;
        Ok(StoreView { node, cookie, off })
    }

    /// Hand a replaced view to the reaper (off the vCPU); inline only if the reaper is gone.
    fn retire(&self, v: StoreView) {
        let Some(t) = self.trap else {
            let _ = self.release(v);
            return;
        };
        let sent = t.retire.lock().ok().map(|tx| tx.send(v));
        if let Some(Err(std::sync::mpsc::SendError(v))) = sent {
            let _ = self.release(v);
        }
    }
}

/// ★ What one VA-table object maps into: a guest CPU window, or a host GPU VA space.
pub enum Target {
    /// A guest BAR aperture.
    Window(CpuWindow<WindowOps>),
    /// ★ The Hopper+ BAR1 aperture: the window, plus the guest's usermode (doorbell) views
    /// (`V3_BAR1_DOORBELL.md`). Turing … Ada use [`Target::Window`] for BAR1, unchanged.
    Bar1(Bar1Target),
    /// A host GPU VA space mirroring a guest one (the Translated plane's target).
    Gpu(GpuMirror),
}

impl Target {
    /// ★ The guest CPU window behind a BAR target (BAR2, or BAR1 on either arm) — BAR1's is where the
    /// boot display's seed lives (`kf_mem::cpuwin::CpuWindow::seed`, `docs/design/V3_DISPLAY.md`
    /// §4.11.2). `None` for a host GPU VA space.
    #[must_use]
    pub fn cpu_window(&self) -> Option<&CpuWindow<WindowOps>> {
        match self {
            Target::Window(w) => Some(w),
            Target::Bar1(b) => Some(&b.win),
            Target::Gpu(_) => None,
        }
    }
}

/// ★ P5: OUR placements in one mirrored host VA space, `va → (len, offset, ram)` — the rows the
/// reconcile made, recorded AS it makes them, so a Translated channel can find the bytes behind a
/// guest VA (its GPFIFO, a pushbuffer segment) through what WE mapped (`THE_TRANSLATED_PLANE.md`
/// §24.2). ⊘ Never a copy of the guest's tables: every row is one of our own map calls.
///
/// ★ Written INSIDE the apply, before the invalidate's `TRIGGER` is cleared — so a doorbell the
/// guest rings after its invalidate completed always finds the rows that invalidate published.
pub type PlacedRows = std::sync::Arc<RwLock<std::collections::BTreeMap<u64, PlacedRow>>>;

/// Resolve `[va, va+len)` against `rows`: `(ram, offset)` when ONE placement covers it whole.
#[must_use]
pub fn resolve_placed(rows: &PlacedRows, va: u64, len: u64) -> Option<(bool, u64)> {
    let r = rows.read().ok()?;
    let (&start, &(rlen, off, ram, _)) = r.range(..=va).next_back()?;
    let end = va.checked_add(len)?;
    (end <= start.checked_add(rlen)?)
        .then(|| off.checked_add(va.saturating_sub(start)).map(|o| (ram, o)))
        .flatten()
}

/// ★ P5b: the placement covering `va` — `(ram, offset of va, bytes of the row left from va)`.
/// A read that crosses from one of OUR rows into the next (two 4 KiB placements adjacent in VA)
/// walks them piece by piece. `[measured p5bs --concurrency]` a 0x60-byte scrubber segment at
/// `…6fb8` crossed a page into the next row and killed the channel as "not placed by us".
#[must_use]
pub fn resolve_placed_prefix(rows: &PlacedRows, va: u64) -> Option<(bool, u64, u64)> {
    let r = rows.read().ok()?;
    let (&start, &(rlen, off, ram, _)) = r.range(..=va).next_back()?;
    let end = start.checked_add(rlen)?;
    (va < end)
        .then(|| {
            off.checked_add(va.saturating_sub(start))
                .map(|o| (ram, o, end.saturating_sub(va)))
        })
        .flatten()
}

/// ★ 2026-10-07 (Windows Code43, run38: a kernel copy channel died reading GP entry 2 of a ring
/// whose entries 0 and 1 it had read): the rows next to an unplaced `va`, for the death message —
/// the nearest row starting at or below `va` and the next one above, plus the row count. Read
/// once, on the error path only.
#[must_use]
pub fn describe_neighbours(rows: &PlacedRows, va: u64) -> String {
    let Ok(r) = rows.read() else {
        return "rows poisoned".into();
    };
    let below = r
        .range(..=va)
        .next_back()
        .map(|(s, (l, _, ram, _))| format!("{s:#x}+{l:#x}{}", if *ram { " ram" } else { "" }));
    let above = r
        .range(va.saturating_add(1)..)
        .next()
        .map(|(s, (l, _, ram, _))| format!("{s:#x}+{l:#x}{}", if *ram { " ram" } else { "" }));
    format!(
        "rows={} below={} above={}",
        r.len(),
        below.as_deref().unwrap_or("none"),
        above.as_deref().unwrap_or("none")
    )
}

/// One placement row: `(len, backing offset, in guest RAM, the guest leaf's permission)` — see
/// [`PlacedRows`]. ★ P1+P2 inc C (`V3_P1P2_TSPACE.md` §3.4): the permission is what the T-mode
/// resolver refuses a write, release or reduction through.
pub type PlacedRow = (u64, u64, bool, kf_host::MapPerm);

/// ★ P1+P2 review fix (2026-10-04, `V3_P1P2_TSPACE.md` §7.13) — **every change the walk commits
/// to a mirror's placement rows, numbered.** The stale-bind counter asks which change landed while
/// a bound piece's fence was still incomplete ([`kf_chan::tmode::Rows::changed_since`]). ⊘ Bumped
/// while the rows' WRITE lock is held and read under their READ guard ([`resolve_rows_epoch`]), so
/// a resolution and the epoch recorded with it always agree. Bounded: the oldest entries go, and
/// a question reaching past them is answered `Unknown`.
#[derive(Debug, Default)]
pub struct RowsLog {
    epoch: AtomicU64,
    log: Mutex<std::collections::VecDeque<(u64, u64, u64, std::time::Instant)>>,
}

/// Entries a [`RowsLog`] keeps.
const ROWS_LOG_MAX: usize = 4096;

impl RowsLog {
    /// A change to the rows over `[lo, hi)` — call with the rows' write lock HELD.
    pub fn commit(&self, lo: u64, hi: u64) {
        let e = self.epoch.fetch_add(1, Ordering::AcqRel).saturating_add(1);
        if let Ok(mut l) = self.log.lock() {
            if l.len() >= ROWS_LOG_MAX {
                l.pop_front();
            }
            l.push_back((e, lo, hi, std::time::Instant::now()));
        }
    }

    /// The current epoch (the number of commits).
    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    /// ★ 2026-10-07 (diagnostic, error path only): the last `n` commits, as `(epoch, lo, hi, ms
    /// ago)`, newest last — which change took a dead channel's rows away.
    #[must_use]
    pub fn recent(&self, n: usize) -> Vec<(u64, u64, u64, u128)> {
        let Ok(l) = self.log.lock() else {
            return Vec::new();
        };
        let mut v: Vec<_> = l
            .iter()
            .rev()
            .take(n)
            .map(|(e, lo, hi, at)| (*e, *lo, *hi, at.elapsed().as_millis()))
            .collect();
        v.reverse();
        v
    }

    /// The earliest commit after `epoch` touching `[va, va+len)`, and when it landed.
    #[must_use]
    pub fn changed_since(&self, epoch: u64, va: u64, len: u64) -> kf_chan::tmode::Changed {
        use kf_chan::tmode::Changed;
        let Ok(l) = self.log.lock() else {
            return Changed::Unknown;
        };
        let reaches = l
            .front()
            .map_or(self.epoch() <= epoch, |c| c.0 <= epoch.saturating_add(1));
        if !reaches {
            return Changed::Unknown;
        }
        let end = va.saturating_add(len);
        l.iter()
            .find(|c| c.0 > epoch && c.1 < end && va < c.2)
            .map_or(Changed::No, |c| Changed::At(c.3))
    }
}

/// ★ [`kf_chan::tmode::Rows::resolve_epoch`] over a mirror: the spans of `[va, va+len)` and the
/// rows' epoch, both read under ONE read guard (a commit bumps the epoch under the write lock).
pub fn resolve_rows_epoch(
    rows: &PlacedRows,
    log: &RowsLog,
    va: u64,
    len: u64,
) -> (Result<Vec<kf_chan::tmode::Span>, u64>, u64) {
    let Ok(r) = rows.read() else {
        return (Err(va), 0);
    };
    let epoch = log.epoch();
    let spans = kf_chan::tmode::resolve_spans(va, len, |at| {
        let (&start, &(rlen, off, ram, perm)) = r.range(..=at).next_back()?;
        let end = start.checked_add(rlen)?;
        (at < end)
            .then(|| {
                off.checked_add(at.saturating_sub(start))
                    .map(|o| (ram, o, end.saturating_sub(at), perm))
            })
            .flatten()
    });
    (spans, epoch)
}

/// ★ P1+P2 inc A (`docs/design/V3_P1P2_TSPACE.md` §3.4): what [`cut_rows`] changed, so the record
/// can be put back exactly when the host refuses the range unmap.
#[derive(Debug, Default)]
pub struct RowCut {
    /// The rows as they were before the cut (removed whole or trimmed).
    pub original: Vec<(u64, PlacedRow)>,
    /// The keys of the remnants the cut inserted (a straddling row's outside parts).
    pub remnants: Vec<u64>,
}

/// ★ P1+P2 inc A (§3.4) — **remove `[va, end)` from a row record EXACTLY as host RM removes it
/// from the space.** A row wholly inside goes; a row that straddles an edge keeps its outside
/// part(s), at their own VA and backing offset — host RM splits a straddling placement and keeps
/// what lies outside the range ([`kf_host::HostRm::unmap_range`]).
///
/// ⊘ Before this the record removed only the rows whose START lay in the range: a row straddling
/// the start stayed at full length (stale coverage — a reader resolved through a mapping that is
/// gone), and a row starting inside but ending past the range was dropped whole while host RM kept
/// its outside part (missing coverage — a false refusal that kills the reading channel). Once the
/// rows are a translation the engine depends on (the T-space resolver), exactness is a safety
/// property, not a convenience.
pub fn cut_rows(
    rows: &mut std::collections::BTreeMap<u64, PlacedRow>,
    va: u64,
    end: u64,
) -> RowCut {
    let mut cut = RowCut::default();
    if end <= va {
        return cut;
    }
    let mut keys: Vec<u64> = rows
        .range(..va)
        .next_back()
        .filter(|&(&k, &(len, _, _, _))| k.saturating_add(len) > va)
        .map(|(&k, _)| k)
        .into_iter()
        .collect();
    keys.extend(rows.range(va..end).map(|(&k, _)| k));
    for k in keys {
        let Some(row @ (len, off, ram, perm)) = rows.remove(&k) else {
            continue;
        };
        cut.original.push((k, row));
        let row_end = k.saturating_add(len);
        if k < va {
            rows.insert(k, (va.saturating_sub(k), off, ram, perm));
            cut.remnants.push(k);
        }
        if row_end > end {
            rows.insert(
                end,
                (
                    row_end.saturating_sub(end),
                    off.saturating_add(end.saturating_sub(k)),
                    ram,
                    perm,
                ),
            );
            cut.remnants.push(end);
        }
    }
    cut
}

/// ★ P1+P2 inc A (review fix 2026-10-04), a measurement: range unmaps whose row removal before
/// inc A (only the rows STARTING in the range, each whole) would have differed from host RM's
/// exact cut ([`cut_rows`]), i.e. a row straddling an edge (`inca[… rows_inexact=…]` on the status
/// line). ★ 2026-10-10 (§AB, inc A hardwired strict): the removal before inc A is deleted; the cut
/// is always exact, and this counter still names how often the exactness mattered.
pub static ROWS_INEXACT: AtomicU64 = AtomicU64::new(0);

/// Whether the removal before inc A of `[va, end)` would differ from host RM's exact cut: a row
/// straddles the start, or a row starting inside ends past the end.
#[must_use]
pub fn cut_is_inexact(
    rows: &std::collections::BTreeMap<u64, PlacedRow>,
    va: u64,
    end: u64,
) -> bool {
    if end <= va {
        return false;
    }
    let straddles_start = rows
        .range(..va)
        .next_back()
        .is_some_and(|(&k, &(len, _, _, _))| k.saturating_add(len) > va);
    straddles_start
        || rows
            .range(va..end)
            .any(|(&k, &(len, _, _, _))| k.saturating_add(len) > end)
}

/// ★ P1+P2 inc A (review fix 2026-10-04; strict hardwired 2026-10-10): the row removal a range
/// unmap makes — always EXACT ([`cut_rows`]); a cut the removal before inc A would have got wrong
/// is counted in [`ROWS_INEXACT`].
pub fn cut_for(rows: &mut std::collections::BTreeMap<u64, PlacedRow>, va: u64, end: u64) -> RowCut {
    if cut_is_inexact(rows, va, end) {
        ROWS_INEXACT.fetch_add(1, Ordering::Relaxed);
    }
    cut_rows(rows, va, end)
}

/// Undo a [`cut_rows`]: remove its remnants, put the original rows back.
pub fn uncut_rows(rows: &mut std::collections::BTreeMap<u64, PlacedRow>, cut: RowCut) {
    for k in cut.remnants {
        rows.remove(&k);
    }
    rows.extend(cut.original);
}

/// ★★★ P6b: **where OUR rings live** — `[RING_REGION_BASE, RING_VA_LIMIT)`, 4 GiB (4096 one-MiB
/// rings) at the very top of what a GP entry can address.
///
/// ⊘ **Corrected 2026-10-10 (`OWNER_RULINGS.md` §AB rules 2-3), above the P6b text below:** the
/// region exists ONLY in the T-space ([`crate::tspace::TSpace`]), never in a mirrored (Passthrough)
/// space, which holds no ring of ours; so the "guest leaf over the region is refused" guard below
/// applies to no twin any more (a twin has no VMM range to protect). The P6b reasoning for WHERE
/// the region lies still holds for the T-space.
///
/// ⊘ A ring may not go where RM puts it: RM's bottom-up allocator in our host space is the SAME
/// allocator the guest's RM runs in its own space (both start just above the split-VAS server
/// window, `0x1_2000_0000`, `gpu_vaspace.c:421-431`), so `[measured p6b1]` token 3's ring landed
/// at `0x121040000` — where the guest's UVM then mapped tokens 4-6's GPFIFOs, and nine guest maps
/// "succeeded" onto OUR ring (`VA_ALREADY_MAPPED`): a VMM address inside the guest's VA space.
/// ⊘ Nor at the top of the space like the windows: a ring must be below 2^40 on every family
/// (`kf_chan::host::RING_VA_LIMIT`). The top 4 GiB below 2^40 is used by no guest kernel
/// allocator we know of: RM allocates bottom-up from 4.5 GiB, and UVM's own kernel ranges sit far
/// above 2^40 (`uvm_ampere.c:55-60`: 160/256/384 TiB; Hopper/Blackwell higher still). ★ And it is
/// not a guess that must hold: a guest leaf over the region is REFUSED by name
/// ([`MapTarget::reserved`]) — the guest's statement fails, it never aliases a ring.
pub const RING_REGION_BASE: u64 = kf_chan::host::RING_VA_LIMIT - RING_REGION_BYTES;
/// The ring region's size.
pub const RING_REGION_BYTES: u64 = 4 << 30;

/// ★ P6b: the ring slots of one host space — the next never-used slot, and the slots whose ring
/// was RELEASED (object freed, GPU mapping gone: v3-appfix J). A slot is a VA; it returns to the
/// pool only when its ring's release fully succeeded, never on a refusal.
pub type RingSlots = std::sync::Arc<std::sync::Mutex<RingSlotPool>>;

/// See [`RingSlots`].
#[derive(Debug, Default)]
pub struct RingSlotPool {
    next: u64,
    free: Vec<u64>,
}

/// ★ P6b: the VA of a free ring slot in a space (a released one first), or `None` when the region
/// is exhausted.
#[must_use]
pub fn take_ring_slot(slots: &RingSlots) -> Option<u64> {
    let per = RING_REGION_BYTES / kf_chan::host::RING_BYTES;
    let mut p = slots.lock().ok()?;
    if let Some(va) = p.free.pop() {
        return Some(va);
    }
    let i = p.next;
    (i < per).then(|| {
        p.next = p.next.saturating_add(1);
        RING_REGION_BASE.saturating_add(i.saturating_mul(kf_chan::host::RING_BYTES))
    })
}

/// ★ v3-appfix J: return a slot whose ring was fully released (unmapped and freed).
pub fn give_ring_slot(slots: &RingSlots, va: u64) {
    let in_region = va >= RING_REGION_BASE && va < RING_REGION_BASE + RING_REGION_BYTES;
    if in_region
        && let Ok(mut p) = slots.lock()
        && !p.free.contains(&va)
    {
        p.free.push(va);
    }
}

/// ★ A mirrored host VA space: the reconcile's target, and the row record beside it.
pub struct GpuMirror {
    /// The host VA space and its objects.
    pub vas: HostVas<'static>,
    /// Our placements (see [`PlacedRows`]).
    pub rows: PlacedRows,
    /// ★ Review fix 2026-10-04: every change committed to `rows`, numbered ([`RowsLog`]) — shared
    /// with the channel plane's [`Mirror::log`].
    pub log: std::sync::Arc<RowsLog>,
    /// ★ P6b: OUR VMM placements in this space a guest leaf may not overlap. ⊘ 2026-10-10 (§AB
    /// rule 2): empty — a twin holds no window and no ring — except the positive control's window.
    pub reserved: Vec<(u64, u64)>,
    /// ★ `V3_BATCHED_MAP.md`: guest RAM as QEMU registered it — the memfd a batch is stitched from
    /// (`None`: this mirror never batches).
    pub ram: Option<&'static RamMap>,
    /// ★ `V3_BATCHED_MAP.md` §5: the same space, placing batches and booking their objects.
    /// ★ Review fix 2026-10-10 (finding 6): shared with the channel plane's [`Mirror::ledger`], so
    /// the falcon-context steer unmaps through the same ownership ledger.
    pub bv: std::sync::Arc<kf_mem::batch::BatchedVas<'static>>,
    /// ★ Host RM calls this space has cost, for the retire line (per CUDA process).
    pub calls: SpaceCalls,
    /// ★★★ v3-roperm: this space is the guest KERNEL's (a Translated channel was born in it, or
    /// its client is one of the guest RM's internal clients) — it mirrors privileged leaves. Until
    /// then it is a USER twin and WITHHOLDS them (`MapTarget::withholds_privileged`). Shared with
    /// the channel plane's [`Mirror::kernel_vas`].
    pub kernel_vas: std::sync::Arc<crate::twin::TwinState>,
    /// ★★★ v3-cdp: OUR SKED-reflected placements, `va → len` (`MapTarget::map_sked`,
    /// `V3_CDP.md`). ⊘ Kept OUT of [`GpuMirror::rows`]: a SKED page is not memory, so no reader may
    /// resolve a guest VA through it; it is here so an unmap and the retire take it down.
    pub sked: Mutex<std::collections::BTreeMap<u64, u64>>,
}

/// ★ `V3_BATCHED_MAP.md`: what one mirrored space cost in host RM calls over its life — the
/// per-process instrument the retire line prints.
#[derive(Debug, Default)]
pub struct SpaceCalls {
    /// Per-run `NV_ESC_RM_MAP_MEMORY_DMA`.
    pub maps: AtomicU64,
    /// Batches placed (each = ONE `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` + ONE map).
    pub batches: AtomicU64,
    /// Runs those batches carried.
    pub batched_runs: AtomicU64,
    /// Batches refused (their runs then went one by one).
    pub batch_refused: AtomicU64,
    /// `NV_ESC_RM_UNMAP_MEMORY_DMA` calls — per-run or range.
    pub unmaps: AtomicU64,
    /// Of which ranges.
    pub ranges: AtomicU64,
    /// ns inside map verbs (per-run + batch).
    pub map_ns: AtomicU64,
    /// ns inside unmap verbs (incl. the frees they trigger).
    pub unmap_ns: AtomicU64,
}

impl SpaceCalls {
    fn line(&self, frees: u64) -> String {
        let g = |a: &AtomicU64| a.load(Ordering::Relaxed);
        format!(
            "host RM over its life: {} map call(s) ({} per-run, {} batch(es) x2 carrying {} runs, {} batch(es) refused) in {} ms; {} unmap call(s) ({} range(s)) + {} free(s) in {} ms",
            g(&self.maps).saturating_add(g(&self.batches).saturating_mul(2)),
            g(&self.maps),
            g(&self.batches),
            g(&self.batched_runs),
            g(&self.batch_refused),
            g(&self.map_ns) / 1_000_000,
            g(&self.unmaps),
            g(&self.ranges),
            frees,
            g(&self.unmap_ns) / 1_000_000
        )
    }
}

/// ⊘ **Corrected 2026-10-10 (`V3_BATCHED_MAP.md` §8.7.6), above the text it corrects:** the flag
/// turns off the stitched BATCH OBJECTS only ([`GpuMirror::map_batch`] answers `NOT_BATCHED`).
/// The net diff, one host mapping per guest leaf outside reservations, the owned-span range unmaps
/// and the ownership ledger stay on, so it is NOT "the per-run path as before" and it does not
/// reproduce run 244 (that is revision `c6fff2e3` with the flag).
///
/// ★ `KF3_NO_BATCHED_MAP=1` turns batched MAPS off (the per-run path, as before
/// `V3_BATCHED_MAP.md`) — an explicit OPT-OUT for A/B measurement only. Read once. Default: ON.
/// ⊘ (2026-10-09) It no longer turns range unmaps off: an exact range over our own placements is
/// how the apply's net diff removes a changed sub-range without touching unchanged pages.
///
/// ★ STATUS (2026-10-09): the flag is no longer a workaround. Every Windows run since run 114 set
/// it because the batched path froze the guest after sign-in (`[measured, runs 242/243]` host Xid
/// 31 `FAULT_PTE` at `0x4034000`, `gpu_vaspace.c:1639` assertions at exit). Root cause: a range
/// unmap that SPLIT a batch mapping placed through the space's `NV01_MEMORY_VIRTUAL` range — host
/// RM frees the whole VA block there. Fixed in `kf_mem::batch::BatchedVas` (no batch outside a
/// guest reservation; ranges over owned spans only), proven against a host RM model
/// (`kf_mem` `sim` tests); `V3_BATCHED_MAP.md` §8.
fn batching_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KF3_NO_BATCHED_MAP").is_none())
}

/// ★ D3 (2026-10-10, `V3_BATCHED_MAP.md` §8.8): micro reservations are the DEFAULT, with no flag.
/// They batch guest-RAM rows OUTSIDE the guest reservations (Windows process VAs,
/// `[1 MiB, 4.5 GiB)`) through a small FIXED `NV50_MEMORY_VIRTUAL` made over exactly each batch's VA,
/// and place every big-leaf row there through one (`kf_mem::batch` rule 2), so a later partial
/// unmap is exact. Why default ON: (i) a row beyond `MAX_LEAF_PIECES` grains is otherwise refused
/// by name — a correctness hole (the 8 GiB flat FB alias of a guest-kernel space is one);
/// (ii) the reserve probe (`kf-micro-reserve-probe reserve`, gate 10 of `scripts/bench/v3_gates.sh`)
/// `[measured]` passed on the trusted host at `6fafcc6e`. A reservation host RM refuses at run time
/// is a clean fallback (4 KiB grain / per run, nothing lost); one it accepts is always used.
///
/// ⊘ The text this corrects: *"`KF3_BATCH_MICRO_RESERVE=1` … DEFAULT OFF until the hardware
/// experiment shows host RM accepts such reservations there; off, rows there are placed one host
/// mapping per guest leaf"* — that flag is GONE (setting it now does nothing).
///
/// `KF3_NEGCTL_NO_MICRO_RESERVE=1` turns them off: a NEGATIVE CONTROL for A/B and bisecting a host
/// that misbehaves with reservations, never a launcher setting (off, an over-bound row is refused
/// by name and every big-leaf row costs one host mapping per 4 KiB page). Named NEGCTL, not DIAG,
/// because it changes behaviour (`V3_FLAG_INVENTORY.md` §8). Read once.
fn micro_reserve_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("KF3_NEGCTL_NO_MICRO_RESERVE").as_deref() != Ok("1"))
}

fn ns_since(t: std::time::Instant) -> u64 {
    u64::try_from(t.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

impl GpuMirror {
    /// A mirror over `vas`, batching through `ram`'s memfd when it has one.
    #[must_use]
    pub fn new(
        vas: HostVas<'static>,
        (rows, log): (PlacedRows, std::sync::Arc<RowsLog>),
        reserved: Vec<(u64, u64)>,
        ram: Option<&'static RamMap>,
        kernel_vas: std::sync::Arc<crate::twin::TwinState>,
    ) -> Self {
        Self::with_ledger(
            std::sync::Arc::new(new_ledger(vas)),
            (rows, log),
            reserved,
            ram,
            kernel_vas,
        )
    }

    /// [`GpuMirror::new`] over a ledger the caller shares (the channel plane's [`Mirror::ledger`]).
    #[must_use]
    pub fn with_ledger(
        bv: std::sync::Arc<kf_mem::batch::BatchedVas<'static>>,
        (rows, log): (PlacedRows, std::sync::Arc<RowsLog>),
        reserved: Vec<(u64, u64)>,
        ram: Option<&'static RamMap>,
        kernel_vas: std::sync::Arc<crate::twin::TwinState>,
    ) -> Self {
        GpuMirror {
            vas: bv.vas,
            rows,
            log,
            reserved,
            ram,
            bv,
            calls: SpaceCalls::default(),
            kernel_vas,
            sked: Mutex::new(std::collections::BTreeMap::new()),
        }
    }

    /// ★ Review fix 2026-10-10 (finding 4): the rows follow the LEDGER over `[va, end)` — every part
    /// of it no mapping of ours covers any more is cut from the rows (host RM removed it before an
    /// error, or the batch layer removed a stray there before a map). A reader never resolves
    /// through a mapping that is gone, and the rows never claim more than the host holds.
    fn rows_follow_ledger(&self, va: u64, end: u64) {
        let owned = self.bv.own_view(va, end).owned;
        let Ok(mut rows) = self.rows.write() else {
            return;
        };
        let mut cur = va;
        let mut changed = false;
        for (s, e) in owned.into_iter().chain(std::iter::once((end, end))) {
            if s > cur && !cut_rows(&mut rows, cur, s).original.is_empty() {
                changed = true;
            }
            cur = cur.max(e);
        }
        if changed {
            self.log.commit(va, end);
        }
    }

    /// Batch objects freed so far.
    fn frees(&self) -> u64 {
        self.bv.frees.load(Ordering::Relaxed)
    }

    /// The retire line's cost summary.
    pub fn calls_line(&self) -> String {
        self.calls.line(self.frees())
    }

    /// ★ Retire-time teardown: every row, as few ranges as are VA-contiguous; then every batch
    /// object. Returns `(rows, refused, host calls)`.
    fn unmap_all_rows(&self) -> (usize, usize, u64) {
        let rows: Vec<(u64, u64)> = self
            .rows
            .read()
            .map(|r| r.iter().map(|(&va, &(len, _, _, _))| (va, len)).collect())
            .unwrap_or_default();
        let before = self
            .calls
            .unmaps
            .load(Ordering::Relaxed)
            .saturating_add(self.frees());
        let mut refused = 0usize;
        // ★★★ v3-cdp: the SKED-reflected placements first (whole-mapping unmaps; never batched).
        let sked: Vec<u64> = self
            .sked
            .lock()
            .map(|m| m.keys().copied().collect())
            .unwrap_or_default();
        for &va in &sked {
            if self.unmap(va, true).is_err() {
                refused = refused.saturating_add(1);
            }
        }
        let mut k = 0;
        while k < rows.len() {
            let mut j = k.saturating_add(1);
            // (`j` and `k` index `rows`, both bounded by `rows.len()`; the VA arithmetic is checked.)
            while j < rows.len()
                && rows
                    .get(j.saturating_sub(1))
                    .zip(rows.get(j))
                    .is_some_and(|(a, b)| a.0.checked_add(a.1) == Some(b.0))
            {
                j = j.saturating_add(1);
            }
            let run = rows.get(k..j).unwrap_or_default();
            let end = run.last().map_or(0, |&(v, l)| v.saturating_add(l));
            let va = run.first().map_or(0, |&(v, _)| v);
            if run.len() < 2 || self.unmap_range(va, end.saturating_sub(va), true).is_err() {
                for &(va, _) in run {
                    if self.unmap(va, true).is_err() {
                        refused = refused.saturating_add(1);
                    }
                }
            }
            k = j;
        }
        // Whatever batch objects remain (a refused unmap left a piece): freeing one unmaps its
        // pieces (`rs_client.c:1342-1395`) — the space is being retired, nothing may keep them.
        let _ = self.bv.drain();
        // ★ Review fix 2026-10-10 (finding 5): anything of ours the drain could not release (a
        // mapping, a micro reservation or a batch object whose free host RM refused) makes the
        // space unclean — it is freed whole, never recycled into another guest VA space.
        let leftovers = self.bv.leftovers();
        if leftovers != 0 {
            eprintln!(
                "kf3: mem retire {:#x}: {leftovers} mapping(s)/reservation(s)/object(s) of ours could not be released — the space is freed, not recycled",
                self.vas.space.space
            );
        }
        let after = self
            .calls
            .unmaps
            .load(Ordering::Relaxed)
            .saturating_add(self.frees());
        (
            rows.len().saturating_add(sked.len()),
            refused.saturating_add(leftovers),
            after.saturating_sub(before),
        )
    }
}

impl MapTarget for GpuMirror {
    fn withholds_privileged(&self) -> bool {
        // ★ P1+P2 inc D: ONE atomic load of the per-twin state word (`crate::twin`).
        !self.kernel_vas.is_kernel()
    }
    fn gpu_space(&self) -> bool {
        true
    }
    fn map(&self, d: &Desired, defer: bool) -> Result<Mapped, String> {
        let t = std::time::Instant::now();
        // ★ 2026-10-09: through the batch layer, which records every host mapping of ours
        // (`kf_mem::batch::OwnMaps`) — a range unmap covers exactly those, never a gap.
        let m = self.bv.map(d, defer);
        self.calls.maps.fetch_add(1, Ordering::Relaxed);
        self.calls.map_ns.fetch_add(ns_since(t), Ordering::Relaxed);
        // ★ Review fix 2026-10-10: a stray the batch layer removed there first is no row.
        self.rows_follow_ledger(d.va, d.va.saturating_add(d.len));
        let m = m?;
        // ★ P6b ruling (a): only a mapping WE placed is a row a reader may resolve through.
        if m == Mapped::Placed
            && let Ok(mut r) = self.rows.write()
        {
            r.insert(d.va, (d.len, d.off, d.ram, d.perm));
            self.log.commit(d.va, d.va.saturating_add(d.len));
        }
        Ok(m)
    }
    fn map_batch(&self, rows: &[Desired], defer: bool) -> Result<(), String> {
        let (Some(ram), true) = (self.ram, batching_enabled()) else {
            return Err(kf_mem::ledger::NOT_BATCHED.into());
        };
        let fd = ram
            .backing_fd()
            .ok_or("no fd-backed guest RAM to stitch a batch from")?;
        let t = std::time::Instant::now();
        let placed = self.bv.place(fd.borrow(), rows, defer);
        self.calls.map_ns.fetch_add(ns_since(t), Ordering::Relaxed);
        if let (Some(a), Some(b)) = (rows.first(), rows.last()) {
            self.rows_follow_ledger(a.va, b.va.saturating_add(b.len));
        }
        if let Err(e) = placed {
            // ★ 2026-10-09: `NOT_BATCHED` = rows outside a guest reservation, placed per run by
            // design (`BatchedVas` rule 2) — not a refused batch.
            if e != kf_mem::ledger::NOT_BATCHED {
                self.calls.batch_refused.fetch_add(1, Ordering::Relaxed);
            }
            return Err(e);
        }
        self.calls.batches.fetch_add(1, Ordering::Relaxed);
        self.calls
            .batched_runs
            .fetch_add(rows.len() as u64, Ordering::Relaxed);
        if let Ok(mut r) = self.rows.write() {
            for d in rows {
                r.insert(d.va, (d.len, d.off, d.ram, d.perm));
                self.log.commit(d.va, d.va.saturating_add(d.len));
            }
        }
        Ok(())
    }
    fn reserved(&self) -> Vec<(u64, u64)> {
        self.reserved.clone()
    }
    fn map_sked(&self, s: &kf_mem::ledger::SkedRow, defer: bool) -> Result<Mapped, String> {
        let t = std::time::Instant::now();
        let m = self.bv.map_sked(s, defer);
        self.calls.maps.fetch_add(1, Ordering::Relaxed);
        self.calls.map_ns.fetch_add(ns_since(t), Ordering::Relaxed);
        let m = m?;
        // ★ Only a mapping WE placed is ours to take down; never a row a reader resolves through.
        if m == Mapped::Placed
            && let Ok(mut k) = self.sked.lock()
        {
            k.insert(s.va, s.len);
        }
        Ok(m)
    }
    fn unmap(&self, va: u64, defer: bool) -> Result<(), String> {
        // ★★★ v3-cdp: a SKED-reflected placement of ours: a whole-mapping unmap (never batched).
        let sked = self.sked.lock().ok().and_then(|mut k| k.remove(&va));
        if let Some(len) = sked {
            let t = std::time::Instant::now();
            let r = self.bv.unmap_run(va, None, defer);
            self.calls.unmaps.fetch_add(1, Ordering::Relaxed);
            self.calls
                .unmap_ns
                .fetch_add(ns_since(t), Ordering::Relaxed);
            if r.is_err()
                && let Ok(mut k) = self.sked.lock()
            {
                k.insert(va, len);
            }
            return r;
        }
        // ⊘ Forget the row FIRST: a reader must never resolve through a mapping being torn down.
        // ★ v3-video: NO row = nothing of OURS is mapped there — a leaf the host held (host RM's
        // own buffer) or one the channel plane handed to host RM (a steered falcon context). A
        // host unmap there would name host RM's mapping, which is not ours to remove (`[measured
        // vvid vid11]` refused `Other(87)`, leaving the space unsettled): answered with no host call.
        let row = match self.rows.write() {
            Ok(mut r) => match r.remove(&va) {
                Some(row) => {
                    self.log.commit(va, va.saturating_add(row.0));
                    row
                }
                None => {
                    eprintln!(
                        "kf3: mem unmap {va:#x}: no placement of ours there (host-held or handed to host RM) — no host call"
                    );
                    return Ok(());
                }
            },
            Err(_) => return Err(format!("unmap {va:#x}: placement rows poisoned")),
        };
        let t = std::time::Instant::now();
        let r = self.bv.unmap_run(va, Some(row.0), defer);
        self.calls.unmaps.fetch_add(1, Ordering::Relaxed);
        self.calls
            .unmap_ns
            .fetch_add(ns_since(t), Ordering::Relaxed);
        if r.is_err() {
            // ★ Review fix 2026-10-10 (finding 4): the row is back for the part still ours.
            if let Ok(mut rows) = self.rows.write() {
                rows.insert(va, row);
                self.log.commit(va, va.saturating_add(row.0));
            }
            self.rows_follow_ledger(va, va.saturating_add(row.0));
        }
        r
    }
    fn unmap_range(&self, va: u64, len: u64, defer: bool) -> Result<(), String> {
        // ★ 2026-10-09: NOT gated by `KF3_NO_BATCHED_MAP` — an exact range over OUR placements
        // (`BatchedVas::unmap_range`: owned spans only, never a split in the NV01 range) is how the
        // apply's net diff unmaps a changed sub-range of a placement without touching its
        // unchanged pages. The opt-out disables batched MAPS only.
        let end = va.checked_add(len).ok_or("unmap range overflows")?;
        // ⊘ Belt and braces: RM removes EVERY mapping of ours in the range, so it must never reach
        // one of our VMM placements (a guest row over one is refused at map time — this re-checks).
        if let Some(&(a, b)) = self.reserved.iter().find(|&&(a, b)| va < b && a < end) {
            return Err(format!(
                "unmap range {va:#x}+{len:#x} reaches OUR placement [{a:#x}, {b:#x}) — refused"
            ));
        }
        // ⊘ Forget the rows FIRST (as `unmap`); put them back if the host refuses, so the per-run
        // fallback still knows each run's length. ★ P1+P2 inc A: cut EXACTLY at both edges, as host
        // RM does ([`cut_rows`]); a cut the removal before inc A would have got wrong is counted
        // ([`cut_for`]).
        let cut = self
            .rows
            .write()
            .map(|mut r| {
                let cut = cut_for(&mut r, va, end);
                if !cut.original.is_empty() {
                    self.log.commit(va, end);
                }
                cut
            })
            .unwrap_or_default();
        // ★★★ v3-cdp: the range takes any SKED-reflected placement of ours inside it down too.
        let sked_removed: Vec<(u64, u64)> = self
            .sked
            .lock()
            .map(|mut k| {
                let keys: Vec<u64> = k.range(va..end).map(|(&v, _)| v).collect();
                keys.into_iter()
                    .filter_map(|v| k.remove(&v).map(|l| (v, l)))
                    .collect()
            })
            .unwrap_or_default();
        let t = std::time::Instant::now();
        let r = self.bv.unmap_range(va, len, defer);
        self.calls.unmaps.fetch_add(1, Ordering::Relaxed);
        self.calls.ranges.fetch_add(1, Ordering::Relaxed);
        if r.is_err() {
            if let Ok(mut rows) = self.rows.write() {
                if !cut.original.is_empty() {
                    self.log.commit(va, end);
                }
                uncut_rows(&mut rows, cut);
            }
            // ★ Review fix 2026-10-10 (finding 4): ⊘ the rows used to be put back WHOLE although
            // the ledger had been cut for every span host RM unmapped before the error — a reader
            // then resolved through a mapping that was gone, and the per-run fallback found "no
            // mapping of ours" at a run's start while its tail was still mapped. Now the rows
            // follow the ledger.
            self.rows_follow_ledger(va, end);
            if let Ok(mut k) = self.sked.lock() {
                let still: Vec<(u64, u64)> = sked_removed
                    .into_iter()
                    .filter(|&(v, l)| !self.bv.own_view(v, v.saturating_add(l)).owned.is_empty())
                    .collect();
                k.extend(still);
            }
        }
        self.calls
            .unmap_ns
            .fetch_add(ns_since(t), Ordering::Relaxed);
        r
    }
    fn invalidate(&self) -> Result<(), String> {
        self.vas.invalidate()
    }
    fn va_extent(&self) -> Option<u64> {
        self.vas.va_extent()
    }
    fn own_view(&self, va: u64, end: u64) -> Option<kf_mem::ledger::OwnView> {
        Some(self.bv.own_view(va, end))
    }
    fn begin_refresh(&self) {
        self.bv.begin_refresh();
    }
}

/// ★ The batch / ownership ledger of one mirrored space (`kf_mem::batch::BatchedVas`), with the
/// process's micro-reservation policy.
#[must_use]
pub fn new_ledger(vas: HostVas<'static>) -> kf_mem::batch::BatchedVas<'static> {
    kf_mem::batch::BatchedVas::with_low_reserve(vas, micro_reserve_enabled())
}

/// ★ P5: one mirrored guest VA space as the channel plane sees it — the host space and our rows.
/// ⊘ 2026-10-10 (`OWNER_RULINGS.md` §AB rule 2): it carries NO window and NO ring. Every mapping
/// in it is a row derived from a guest page-table leaf; the legacy store/RAM windows and ring
/// region (P5 §12, "in EVERY host VAS we create", audit S1-21) are deleted with `KF3_TSPACE`.
/// The only other mapping that can exist is the positive control's ([`negctl_twin_window`]).
#[derive(Clone)]
pub struct Mirror {
    /// The host VA space.
    pub space: kf_host::VaSpace,
    /// ⚠ CONTROL ONLY: the store window `KF3_NEGCTL_TWIN_WINDOW=1` maps (`(base, len)`), recorded
    /// so the log line names it and the box-log gate's `WINDOWS=NONE` fails. `None` in production.
    pub negctl_window: Option<(u64, u64)>,
    /// Our placements.
    pub rows: PlacedRows,
    /// ★ P5b: the guest-RAM host object (a sysmem USERD's twin names it; never mapped here).
    pub ram_obj: Option<u32>,
    /// ★ P5c: host channels (twins, Translated rings) born in this space and not yet freed — the
    /// channel plane counts them. A retired space with any is NEVER recycled (a live engine in a
    /// space handed to another guest VA space would be a cross-tenant hazard).
    pub live: std::sync::Arc<AtomicU64>,
    /// ★★★ v3-roperm: the space is the guest KERNEL's — set by the channel plane when it births a
    /// Translated channel here; the memory plane's [`GpuMirror`] reads it to decide whether a
    /// privileged guest leaf may be mirrored.
    pub kernel_vas: std::sync::Arc<crate::twin::TwinState>,
    /// ★ Review fix 2026-10-04: the commit log of `rows` ([`RowsLog`]), shared with the walker's
    /// [`GpuMirror::log`].
    pub log: std::sync::Arc<RowsLog>,
    /// ★ Review fix 2026-10-10 (finding 6): the space's ownership ledger, shared with the walker's
    /// [`GpuMirror::bv`] — so the channel plane's falcon-context steer unmaps OUR mappings only,
    /// through the `hDma` each was mapped through, and the ledger never keeps a stale entry.
    /// `None` until the walker's mirror exists.
    pub ledger: Option<std::sync::Arc<kf_mem::batch::BatchedVas<'static>>>,
}

/// ★★★ v3-roperm: a mirror's starting classification — KERNEL for one of the guest RM's own
/// internal clients (a handle range no guest process can hold, `kf_rm::chanlink::is_rm_internal_client`),
/// USER (withholding privileged leaves) for everything else until a Translated channel is born in it.
#[must_use]
pub fn kernel_vas_for(key: VasKey) -> std::sync::Arc<crate::twin::TwinState> {
    let client = u32::try_from(key.0 >> 32).unwrap_or(0);
    std::sync::Arc::new(crate::twin::TwinState::for_kernel(
        kf_rm::chanlink::is_rm_internal_client(client),
    ))
}

/// ★ P5c: a retired mirror's host space, its rows unmapped — ready to be the next guest VA space's
/// mirror. `[measured c3, 61fc7ac8]` the legacy build (space + the 8 GiB store window + the 2 GiB
/// guest-RAM window) cost ~54 ms of host RM time; the raw client's `--concurrency` allocates and
/// frees 2400 VA spaces (7 s on bare metal). ⊘ 2026-10-10 (§AB rule 2): a spare holds nothing of
/// ours (no window, no ring) — only the positive control's window when it ran.
#[derive(Debug, Clone)]
pub struct Spare {
    space: kf_host::VaSpace,
    /// ⚠ CONTROL ONLY ([`Mirror::negctl_window`]): so a recycled twin's record says what its space
    /// actually holds.
    negctl_window: Option<(u64, u64)>,
    ram_obj: Option<u32>,
}

/// Spares kept; a retirement beyond this frees the host space instead.
const SPARES_MAX: usize = 32;

/// ★ Build once, behind a precondition that can be TRANSIENTLY absent. `pre` is read outside the
/// cell and its value is what `build` gets, so the build never re-reads what `pre` saw (no
/// check-then-use gap), and a `None` from `pre` returns `not_yet` WITHOUT touching the cell — the
/// next call looks again. Only `build`'s outcome, success or refusal, is remembered.
fn once_after<P, T: Clone>(
    cell: &std::sync::OnceLock<Result<T, String>>,
    pre: impl FnOnce() -> Option<P>,
    not_yet: &str,
    build: impl FnOnce(P) -> Result<T, String>,
) -> Result<T, String> {
    if let Some(done) = cell.get() {
        return done.clone();
    }
    let p = pre().ok_or_else(|| not_yet.to_owned())?;
    cell.get_or_init(|| build(p)).clone()
}

/// ★ w827: spares built by [`prewarm`] before the guest runs. `[measured w827 vh2, 58e03230]` a
/// raw-client process names THREE VA spaces (the floor arm `--timer`: one took the single prewarmed
/// spare, two paid `create_mirror` at 67-90 ms each — its RAM window map is 65-86 ms — INSIDE a held
/// page-directory reply: `rpc_held` 191 ms of a 2.4 s process). Built on the VA thread while it is
/// idle, off every vCPU, from our own objects; nothing guest-visible.
pub const PREWARM_SPARES: u64 = 3;

/// ★ P5: the mirrors, by VA-space object — written by the VA thread when it creates one, read by
/// the channel plane when a channel names it.
pub type Mirrors = std::sync::Arc<Mutex<std::collections::HashMap<VasKey, Mirror>>>;

impl MapTarget for Target {
    fn map(&self, d: &Desired, defer: bool) -> Result<Mapped, String> {
        match self {
            Target::Window(w) => w.map(d, defer),
            Target::Bar1(b) => b.map(d, defer),
            Target::Gpu(g) => g.map(d, defer),
        }
    }
    // ★ `V3_BATCHED_MAP.md`: forwarded EXPLICITLY — a trait default here would silently answer
    // `NOT_BATCHED` for every space (`[measured bm1]`: 3 groups formed, 0 batched, 12 291 verbs).
    fn map_batch(&self, rows: &[Desired], defer: bool) -> Result<(), String> {
        match self {
            Target::Window(w) => w.map_batch(rows, defer),
            Target::Bar1(b) => b.win.map_batch(rows, defer),
            Target::Gpu(g) => g.map_batch(rows, defer),
        }
    }
    fn unmap_range(&self, va: u64, len: u64, defer: bool) -> Result<(), String> {
        match self {
            Target::Window(w) => w.unmap_range(va, len, defer),
            Target::Bar1(b) => b.win.unmap_range(va, len, defer),
            Target::Gpu(g) => g.unmap_range(va, len, defer),
        }
    }
    fn reserved(&self) -> Vec<(u64, u64)> {
        match self {
            Target::Window(w) => w.reserved(),
            Target::Bar1(b) => b.win.reserved(),
            Target::Gpu(g) => g.reserved(),
        }
    }
    fn unmap(&self, va: u64, defer: bool) -> Result<(), String> {
        match self {
            Target::Window(w) => w.unmap(va, defer),
            Target::Bar1(b) => b.unmap(va, defer),
            Target::Gpu(g) => g.unmap(va, defer),
        }
    }
    fn invalidate(&self) -> Result<(), String> {
        match self {
            Target::Window(w) => w.invalidate(),
            Target::Bar1(b) => b.win.invalidate(),
            Target::Gpu(g) => g.invalidate(),
        }
    }
    fn va_extent(&self) -> Option<u64> {
        match self {
            Target::Window(w) => w.va_extent(),
            Target::Bar1(b) => b.win.va_extent(),
            Target::Gpu(g) => g.va_extent(),
        }
    }
    fn settle(&self) -> Settle {
        match self {
            Target::Bar1(b) => b.settle(),
            Target::Window(_) | Target::Gpu(_) => Settle::Live,
        }
    }
    // ★★★ v3-cdp: forwarded EXPLICITLY. A GPU mirror places a SKED-reflected page; a CPU window
    // keeps the trait default — a refusal by name (it cannot express a message-kind mapping).
    fn map_sked(&self, s: &kf_mem::ledger::SkedRow, defer: bool) -> Result<Mapped, String> {
        match self {
            Target::Window(w) => w.map_sked(s, defer),
            Target::Bar1(b) => b.win.map_sked(s, defer),
            Target::Gpu(g) => g.map_sked(s, defer),
        }
    }
    fn map_usermode(&self, u: &UsermodeRow) -> Result<Mapped, String> {
        match self {
            // ⊘ BAR2 is RM's own kernel aperture; a usermode view there has no reader. The trait
            // default (not mirrored, satisfied) is the answer, as for a GPU VA space.
            Target::Window(w) => w.map_usermode(u),
            Target::Bar1(b) => b.map_usermode(u),
            // ★ A GPU VA view of the doorbell: NOT MIRRORED (the trait default, `V3_BAR1_DOORBELL.md` §5).
            Target::Gpu(g) => g.map_usermode(u),
        }
    }
    // ★ v3-roperm: forwarded EXPLICITLY (a trait default here would mirror privileged leaves into
    // every user twin). The CPU windows are the guest kernel's apertures: they mirror them.
    fn withholds_privileged(&self) -> bool {
        match self {
            Target::Window(_) | Target::Bar1(_) => false,
            Target::Gpu(g) => g.withholds_privileged(),
        }
    }
    // ★ P1+P2 inc A: forwarded EXPLICITLY — only a host GPU VA space is bounded by the carve-out;
    // the CPU windows are the guest kernel's own views (count-only, §4.3).
    fn gpu_space(&self) -> bool {
        match self {
            Target::Window(_) | Target::Bar1(_) => false,
            Target::Gpu(g) => g.gpu_space(),
        }
    }
    // ★ Review fix 2026-10-10: forwarded EXPLICITLY — the trait default (`None`) would keep pages
    // no mapping of ours covers and parts of mappings host RM cannot split (findings 1, 3).
    fn own_view(&self, va: u64, end: u64) -> Option<kf_mem::ledger::OwnView> {
        match self {
            Target::Window(w) => w.own_view(va, end),
            Target::Bar1(b) => b.win.own_view(va, end),
            Target::Gpu(g) => g.own_view(va, end),
        }
    }
    // ★ Review item 4: forwarded EXPLICITLY (the default renews nothing).
    fn begin_refresh(&self) {
        match self {
            Target::Window(w) => w.begin_refresh(),
            Target::Bar1(b) => b.win.begin_refresh(),
            Target::Gpu(g) => g.begin_refresh(),
        }
    }
}

/// ★ The default BAR1 doorbell-overlay pool (the C device's `bar1-overlays` property, which
/// overrides it at registration). ⊘ User CPU maps are `ALLOW_DISCONTIG` ⇒ never reused
/// (`mapping_cpu.c:484`, `kern_bus_gm107.c:3043-3047`), so this is roughly one per guest process
/// holding a CUDA context, plus UVM's one kernel view. One more is refused by name.
pub const BAR1_OVERLAY_SLOTS: usize = 64;

/// ★ The BAR1 overlay verb the C device registered, its pool size, the completions its main-loop
/// bottom half posts back, and the counters.
#[derive(Debug)]
pub struct Bar1Overlay {
    hook: std::sync::OnceLock<crate::raw_unsafe::OverlayHook>,
    /// The C device's pool size.
    pub cap: std::sync::atomic::AtomicUsize,
    /// `(seq, rc)` posted by the main loop, drained by the VA thread in [`Bar1Target`]'s settle.
    /// ⊘ Held only for a push or a `take` — never across a wait, never by a vCPU.
    done: Mutex<Vec<(u64, i32)>>,
    /// Overlays installed (confirmed live by the main loop).
    pub installed: AtomicU64,
    /// Overlays removed (confirmed).
    pub removed: AtomicU64,
    /// Views refused (by the tracker or the C device), each named in the VA stats.
    pub refused: AtomicU64,
    /// ★ 2026-09-26: doorbell writes (`+0x90`) that arrived through a BAR1 view (T1 evidence).
    pub rings: AtomicU64,
}

impl Default for Bar1Overlay {
    fn default() -> Self {
        Bar1Overlay {
            hook: std::sync::OnceLock::new(),
            cap: std::sync::atomic::AtomicUsize::new(BAR1_OVERLAY_SLOTS),
            done: Mutex::new(Vec::new()),
            installed: AtomicU64::new(0),
            removed: AtomicU64::new(0),
            refused: AtomicU64::new(0),
            rings: AtomicU64::new(0),
        }
    }
}

impl Bar1Overlay {
    /// Register the C device's verb and its pool size (once).
    pub fn set(&self, h: crate::raw_unsafe::OverlayHook, cap: usize) -> bool {
        if self.hook.set(h).is_err() {
            return false;
        }
        self.cap.store(cap, Ordering::Release);
        true
    }

    /// ★ The main loop reports change `seq` applied with `rc` (0 = live/removed). The caller then
    /// wakes the VA thread; this never blocks beyond one uncontended push.
    pub fn post(&self, seq: u64, rc: i32) {
        if let Ok(mut d) = self.done.lock() {
            d.push((seq, rc));
        }
    }

    fn take(&self) -> Vec<(u64, i32)> {
        self.done
            .lock()
            .map(|mut d| std::mem::take(&mut *d))
            .unwrap_or_default()
    }
}

/// One queued overlay change.
#[derive(Debug, Clone, Copy)]
struct InFlight {
    install: bool,
    view: kf_trap::bar1db::Bar1View,
}

/// ★★★ **The Hopper+ BAR1 target** (`V3_BAR1_DOORBELL.md` §3): ordinary leaves go to the window; a
/// usermode-page view becomes a write-trapped overlay at exactly the BAR1 offset the guest's own
/// PTEs put it, and is removed when its UNMAP arrives. VA-manager thread only.
///
/// ★ **Asynchronous (ruling 2026-09-26 (5))**: the verb QUEUES the change for QEMU's main loop and
/// returns; [`MapTarget::settle`] answers `Pending` until the main loop posts it back, and the VA
/// manager defers only the clears of the invalidates that named BAR1. ⇒ the overlay is live before
/// the clear on map, and gone before the clear on unmap — with no thread ever waiting on the BQL.
pub struct Bar1Target {
    /// The BAR1 window.
    pub win: CpuWindow<WindowOps>,
    /// Views live or being installed (a removed view leaves at once — its removal is in flight).
    db: std::cell::RefCell<kf_trap::bar1db::Bar1Doorbells>,
    overlay: std::sync::Arc<Bar1Overlay>,
    inflight: std::cell::RefCell<std::collections::BTreeMap<u64, InFlight>>,
    next_seq: std::cell::Cell<u64>,
    /// Bases whose install failed after its run was acknowledged: their UNMAP retires quietly.
    dead: std::cell::RefCell<std::collections::BTreeSet<u64>>,
}

impl Bar1Target {
    /// The window, a tracker over `bar1_bytes` and a `usermode_len`-byte page, and the verb.
    #[must_use]
    pub fn new(
        win: CpuWindow<WindowOps>,
        bar1_bytes: u64,
        usermode_len: u64,
        overlay: std::sync::Arc<Bar1Overlay>,
    ) -> Bar1Target {
        Bar1Target {
            win,
            db: std::cell::RefCell::new(kf_trap::bar1db::Bar1Doorbells::new(
                bar1_bytes,
                usermode_len,
                BAR1_OVERLAY_SLOTS,
            )),
            overlay,
            inflight: std::cell::RefCell::new(std::collections::BTreeMap::new()),
            next_seq: std::cell::Cell::new(1),
            dead: std::cell::RefCell::new(std::collections::BTreeSet::new()),
        }
    }

    /// The views currently trapped or being installed (the guest doorbell module's replay set).
    #[must_use]
    pub fn views(&self) -> Vec<kf_trap::bar1db::Bar1View> {
        self.db.borrow().views().copied().collect()
    }

    fn refuse<T>(&self, why: String) -> Result<T, String> {
        self.overlay.refused.fetch_add(1, Ordering::Relaxed);
        Err(why)
    }

    fn submit(&self, install: bool, v: kf_trap::bar1db::Bar1View) -> Result<(), String> {
        let Some(hook) = self.overlay.hook.get() else {
            return Err(format!(
                "BAR1 doorbell view {:#x}+{:#x}: the C device registered no overlay verb — the view cannot trap, and it is NEVER backed by guest RAM",
                v.base, v.len
            ));
        };
        let seq = self.next_seq.get();
        hook.submit(seq, u32::from(install), v.base, v.len, v.vf_rel).map_err(|e| {
            format!("BAR1 doorbell view {:#x}+{:#x}: the C device refused to queue the change (errno {e})", v.base, v.len)
        })?;
        self.next_seq.set(seq.saturating_add(1));
        self.inflight
            .borrow_mut()
            .insert(seq, InFlight { install, view: v });
        Ok(())
    }

    /// An ordinary BAR1 leaf. ⊘ Refused by name under a live (or installing) doorbell view: the
    /// overlay would shadow it and turn the guest's stores to that memory into doorbell writes.
    /// Under a view whose REMOVAL is in flight it is placed: it becomes visible when the removal
    /// lands, which is before the invalidate that states it clears.
    fn map(&self, d: &Desired, defer: bool) -> Result<Mapped, String> {
        let end = d.va.saturating_add(d.len);
        if let Some(v) = self
            .db
            .borrow()
            .views()
            .find(|v| v.base < end && d.va < v.base.saturating_add(v.len))
        {
            return self.refuse(format!(
                "BAR1 leaf {:#x}+{:#x} lies under the live doorbell view {:#x}+{:#x}; refused until that view is unmapped",
                d.va, d.len, v.base, v.len
            ));
        }
        self.win.map(d, defer)
    }

    fn map_usermode(&self, u: &UsermodeRow) -> Result<Mapped, String> {
        let v = kf_trap::bar1db::Bar1View {
            base: u.va,
            len: u.len,
            vf_rel: u.vf_rel,
        };
        {
            let mut db = self.db.borrow_mut();
            db.set_cap(self.overlay.cap.load(Ordering::Acquire));
            if let Err(e) = db.place(v) {
                drop(db);
                return self.refuse(format!("BAR1 doorbell view refused: {e:?}"));
            }
        }
        if let Err(e) = self.submit(true, v) {
            self.db.borrow_mut().remove(v.base);
            return self.refuse(e);
        }
        self.dead.borrow_mut().remove(&v.base);
        Ok(Mapped::Placed)
    }

    fn unmap(&self, va: u64, defer: bool) -> Result<(), String> {
        if self.dead.borrow_mut().remove(&va) {
            return Ok(()); // its install failed (named then): nothing of ours is there
        }
        let Some(v) = self.db.borrow().at_base(va) else {
            return self.win.unmap(va, defer);
        };
        // ⊘ Unmap may not complete while anything routes through the view: the removal is queued
        // here, and `settle` holds the invalidate's clear until the main loop confirms it.
        if let Err(e) = self.submit(false, v) {
            return self.refuse(e);
        }
        self.db.borrow_mut().remove(va);
        Ok(())
    }

    /// Drain the main loop's completions; `Pending` while any change is still queued.
    fn settle(&self) -> Settle {
        let mut failures = Vec::new();
        for (seq, rc) in self.overlay.take() {
            let Some(f) = self.inflight.borrow_mut().remove(&seq) else {
                continue;
            };
            match (f.install, rc) {
                (true, 0) => {
                    // ★ 2026-09-26 (T1 evidence, `V3_BAR1_DOORBELL.md` §7): one bounded line per
                    // view, on the VA thread — the heartbeat misses an arm shorter than its period.
                    let n = self
                        .overlay
                        .installed
                        .fetch_add(1, Ordering::Relaxed)
                        .saturating_add(1);
                    if n <= 32 {
                        eprintln!(
                            "kf3: bar1db view LIVE #{n}: BAR1 {:#x}+{:#x} (usermode page {:#x}) now traps its doorbell",
                            f.view.base, f.view.len, f.view.vf_rel
                        );
                    }
                }
                (false, 0) => {
                    let n = self
                        .overlay
                        .removed
                        .fetch_add(1, Ordering::Relaxed)
                        .saturating_add(1);
                    if n <= 32 {
                        eprintln!(
                            "kf3: bar1db view REMOVED #{n}: BAR1 {:#x}+{:#x} (doorbells through BAR1 views so far: {})",
                            f.view.base,
                            f.view.len,
                            self.overlay.rings.load(Ordering::Relaxed)
                        );
                    }
                }
                (true, e) => {
                    // ⊘ Its run was already acknowledged APPLIED: the view is retired here and its
                    // later UNMAP retires quietly (`dead`). Named; the invalidate stays armed.
                    if self.db.borrow().at_base(f.view.base) == Some(f.view) {
                        self.db.borrow_mut().remove(f.view.base);
                    }
                    self.dead.borrow_mut().insert(f.view.base);
                    self.overlay.refused.fetch_add(1, Ordering::Relaxed);
                    failures.push(format!(
                        "BAR1 doorbell overlay {:#x}+{:#x} (page {:#x}): install FAILED in QEMU (errno {e}) — the view does not trap",
                        f.view.base, f.view.len, f.view.vf_rel
                    ));
                }
                (false, e) => {
                    self.overlay.refused.fetch_add(1, Ordering::Relaxed);
                    failures.push(format!(
                        "BAR1 doorbell overlay {:#x}: removal FAILED in QEMU (errno {e})",
                        f.view.base
                    ));
                }
            }
        }
        if !failures.is_empty() {
            Settle::Failed(failures.join("; "))
        } else if self.inflight.borrow().is_empty() {
            Settle::Live
        } else {
            Settle::Pending
        }
    }
}

/// The manager the VA thread owns.
pub type Manager = VaManager<GpuWalker, Target>;

/// ★ What a split walks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitTarget {
    /// A guest `MEM_OP` invalidate's root (`None` = `PDB_ALL`).
    Pdb(Option<u64>),
    /// ★ OWNER_RULINGS §U.2: the triggering channel's OWN space (a deferred TLB invalidate).
    Space(kf_mem::vasmgr::VasKey),
}

/// ★ The drainer → VA-manager inbox for the guest's address-space statements, with the two
/// counters that decide when their held replies may go: `received` (the drainer, as it enqueues)
/// and `settled` (the VA thread, once everything received so far is applied and no walk is in
/// flight or pending). A held reply is delivered when they are equal.
#[derive(Debug)]
pub struct Inbox {
    /// ★ w827: guest L2 cache-op requests per [`kf_trap::cacheop::CacheOp`] — bumped by the vCPU
    /// (lock-free) after it stores the busy word; the VA thread performs the host op and publishes
    /// idle. See `ChanDevice::serve_cache_ops`.
    pub cache_req: [AtomicU64; kf_trap::cacheop::CacheOp::COUNT],
    /// ★ w828: per op, the request count the host has FINISHED — stored by the VA thread after the
    /// host verb returned (a completion is a host event), read by the vCPU to answer a Hopper+
    /// `…_COMPLETED` token register (`kf_trap::cacheop::completed_word`).
    pub cache_done: [AtomicU64; kf_trap::cacheop::CacheOp::COUNT],
    q: Mutex<Vec<MemStatement>>,
    received: AtomicU64,
    settled: AtomicU64,
    /// The VA thread's wake.
    pub wake: Notifier,
    /// ★ P6: `MEM_OP` splits a Translated channel asked for — `(ticket, guest token, target)`.
    splits: Mutex<Vec<(u64, u32, SplitTarget)>>,
    /// ★ OWNER_RULINGS §U.2: the host gate a split opens once it is committed — ticket →
    /// `(the ring's gate, the payload its acquire waits for)`.
    gates: Mutex<std::collections::HashMap<u64, (kf_chan::host::Gate, u32)>>,
    /// ★ P6: tickets the VA thread took and has not finished — ticket → guest token.
    split_tokens: Mutex<std::collections::HashMap<u64, u32>>,
    /// ★ P6: finished splits waiting for their channel's next pump.
    split_results: Mutex<std::collections::HashMap<u64, Result<(), String>>>,
    next_ticket: AtomicU64,
    /// ★ 2026-10-03 (B5, `V3_DISPLAY.md` §4.11.13): how many times the guest's RM gave BAR1 up
    /// (the drainer bumps it; the VA thread re-seeds the boot framebuffer when it moved).
    bar1_physical: AtomicU64,
    /// ★ P1+P2 review fix (2026-10-04, §3.5): the VA thread has a walk in flight or pending, or an
    /// armed invalidate — published by it at the end of every loop ([`Inbox::set_walk_busy`]).
    walk_busy: std::sync::atomic::AtomicBool,
    /// Guest tokens of Translated channels waiting for a pending walk to land.
    walk_waiters: Mutex<Vec<u32>>,
}

impl Inbox {
    /// An empty inbox.
    ///
    /// # Errors
    /// No eventfd.
    pub fn new() -> Result<Inbox, String> {
        Ok(Inbox {
            cache_req: std::array::from_fn(|_| AtomicU64::new(0)),
            cache_done: std::array::from_fn(|_| AtomicU64::new(0)),
            q: Mutex::new(Vec::new()),
            received: AtomicU64::new(0),
            settled: AtomicU64::new(0),
            wake: Notifier::create().map_err(|e| format!("eventfd: {e:?}"))?,
            splits: Mutex::new(Vec::new()),
            gates: Mutex::new(std::collections::HashMap::new()),
            split_tokens: Mutex::new(std::collections::HashMap::new()),
            split_results: Mutex::new(std::collections::HashMap::new()),
            next_ticket: AtomicU64::new(1),
            bar1_physical: AtomicU64::new(0),
            walk_busy: std::sync::atomic::AtomicBool::new(false),
            walk_waiters: Mutex::new(Vec::new()),
        })
    }

    /// ★ Review fix 2026-10-04 (§3.5) — **a walk the guest asked for has not landed yet**: the VA
    /// thread says so ([`Inbox::set_walk_busy`]), a statement is unsettled, or a split is queued
    /// or running. A T-mode operand with no row then waits for it instead of being refused.
    #[must_use]
    pub fn walk_busy(&self) -> bool {
        self.walk_busy.load(Ordering::Acquire)
            || !self.all_settled()
            || self.splits.lock().is_ok_and(|q| !q.is_empty())
            || self.split_tokens.lock().is_ok_and(|m| !m.is_empty())
    }

    /// The VA thread, at the end of every loop: whether it has a walk in flight or pending.
    pub fn set_walk_busy(&self, busy: bool) {
        self.walk_busy.store(busy, Ordering::Release);
    }

    /// A worker: channel `token` waits for the pending walk to land (rung by the VA thread when it
    /// is idle — at most one loop later, so a registration racing the take is never lost).
    pub fn wait_walk(&self, token: u32) {
        if let Ok(mut w) = self.walk_waiters.lock()
            && !w.contains(&token)
        {
            w.push(token);
        }
        let _ = self.wake.signal();
    }

    /// The VA thread, idle: the tokens to ring.
    pub fn take_walk_waiters(&self) -> Vec<u32> {
        self.walk_waiters
            .lock()
            .map(|mut w| std::mem::take(&mut *w))
            .unwrap_or_default()
    }

    /// ★ 2026-10-03 (B5), the drainer: the guest's RM gave BAR1 up — ask the VA thread to show the
    /// boot framebuffer's physical view again (`kf_mem::cpuwin::CpuWindow::reseed`). ⊘ Never waits.
    pub fn request_bar1_physical(&self) {
        self.bar1_physical.fetch_add(1, Ordering::AcqRel);
        let _ = self.wake.signal();
    }

    /// The VA thread: how many such requests were made so far.
    #[must_use]
    pub fn bar1_physical_requests(&self) -> u64 {
        self.bar1_physical.load(Ordering::Acquire)
    }

    /// ★ P6, a WORKER: channel `token` reached a `MEM_OP` invalidate of `pdb` and its prior work
    /// completed — ask the VA thread to walk + reconcile. Returns the ticket to poll. ⊘ Never waits.
    pub fn request_split(&self, token: u32, pdb: Option<u64>) -> u64 {
        let t = self.next_ticket.fetch_add(1, Ordering::Relaxed);
        if kf_mem::maplog::on() {
            eprintln!(
                "kf3: maplog t={:.6} SPLIT-REQUEST ticket={t} by channel token {token:#x} pdb={pdb:x?}",
                kf_mem::maplog::t()
            );
        }
        if let Ok(mut q) = self.splits.lock() {
            q.push((t, token, SplitTarget::Pdb(pdb)));
        }
        let _ = self.wake.signal();
        t
    }

    /// ★★ OWNER_RULINGS §U.2, a WORKER: channel `token` reached a deferred `DMA_INVALIDATE_TLB`;
    /// its prior work completed and the host ring now holds an acquire of `payload` on `gate`.
    /// Ask the VA thread to walk the channel's OWN space `key` (the guest's handles in the entry are
    /// ignored) and — only after the diff is committed and the host invalidate landed — store
    /// `payload` ([`Inbox::finish_split`]). Returns the ticket to poll. ⊘ Never waits.
    pub fn request_gated_split(
        &self,
        token: u32,
        key: kf_mem::vasmgr::VasKey,
        gate: kf_chan::host::Gate,
        payload: u32,
    ) -> u64 {
        let t = self.next_ticket.fetch_add(1, Ordering::Relaxed);
        eprintln!(
            "kf3: GATE ticket={t} by channel token {token:#x}: walk {key:?}, then release payload {payload}"
        );
        if let Ok(mut g) = self.gates.lock() {
            g.insert(t, (gate, payload));
        }
        if let Ok(mut q) = self.splits.lock() {
            q.push((t, token, SplitTarget::Space(key)));
        }
        let _ = self.wake.signal();
        t
    }

    /// ★ P6, the VA thread: the splits requested since the last call, `(ticket, target)`.
    pub fn take_split_requests(&self) -> Vec<(u64, SplitTarget)> {
        let taken = self
            .splits
            .lock()
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default();
        if let Ok(mut m) = self.split_tokens.lock() {
            for (t, tok, _) in &taken {
                m.insert(*t, *tok);
            }
        }
        taken.into_iter().map(|(t, _, p)| (t, p)).collect()
    }

    /// ★ P6, the VA thread: `ticket` finished. Returns the guest token to ring.
    ///
    /// ★★ §U.2: a gated split's gate is RELEASED here — on the VA thread, after the walk's diff was
    /// applied and its host invalidate landed (the caller takes finished splits only after
    /// `on_walk_ready`), and BEFORE the outcome is published, so the channel that polls `Ok` finds
    /// its gate open. A failed split never releases it (the channel dies at its poll); a release
    /// that fails (the ring is gone) turns the outcome into that failure.
    pub fn finish_split(&self, ticket: u64, r: Result<(), String>) -> Option<u32> {
        let gate = self.gates.lock().ok().and_then(|mut g| g.remove(&ticket));
        let r = match (r, gate) {
            (Ok(()), Some((g, payload))) => g
                .release(payload)
                .map(|()| {
                    eprintln!(
                        "kf3: GATE ticket={ticket} RELEASED payload {payload} after the commit"
                    );
                })
                .map_err(|e| format!("gate release: {e}")),
            (r, _) => r,
        };
        if let Ok(mut m) = self.split_results.lock() {
            m.insert(ticket, r);
        }
        self.split_tokens
            .lock()
            .ok()
            .and_then(|mut m| m.remove(&ticket))
    }

    /// ★ P6, a WORKER: `ticket`'s outcome, once (`None`: still running).
    pub fn split_result(&self, ticket: u64) -> Option<Result<(), String>> {
        self.split_results
            .lock()
            .ok()
            .and_then(|mut m| m.remove(&ticket))
    }

    /// The drainer: enqueue one statement and wake the VA thread.
    pub fn push(&self, s: MemStatement) {
        if let Ok(mut q) = self.q.lock() {
            q.push(s);
        }
        self.received.fetch_add(1, Ordering::AcqRel);
        let _ = self.wake.signal();
    }

    /// The VA thread: take everything queued.
    pub fn take(&self) -> Vec<MemStatement> {
        self.q
            .lock()
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default()
    }

    /// Whether every statement received has been settled — held replies may be delivered.
    #[must_use]
    pub fn all_settled(&self) -> bool {
        self.settled.load(Ordering::Acquire) >= self.received.load(Ordering::Acquire)
    }

    /// The VA thread: `n` statements (counted from the start) are settled.
    pub fn settle(&self, n: u64) -> bool {
        self.settled.fetch_max(n, Ordering::AcqRel) < n
    }

    /// `(received, settled)`, for the boot log.
    #[must_use]
    pub fn counts(&self) -> (u64, u64) {
        (
            self.received.load(Ordering::Relaxed),
            self.settled.load(Ordering::Relaxed),
        )
    }
}

/// Counters for the boot log — never a decision input.
#[derive(Debug, Default)]
pub struct MemCounters {
    /// Trigger writes that armed.
    pub invalidates: AtomicU64,
    /// ★ w827: guest L2 cache ops served as host `FB_FLUSH_GPU_CACHE`.
    pub cache_ops: AtomicU64,
    /// ★ v3-refusals: guest sysmembars (`INTERNAL_BUS_FLUSH_WITH_SYSMEMBAR`) served as the host's.
    pub sysmembars: AtomicU64,
    /// ★ v3-refusals: roots withdrawn by `DMA_UNSET_PAGE_DIRECTORY`.
    pub root_unsets: AtomicU64,
    /// PRAMIN re-points whose window showed any scratch slot.
    pub pramin_miss_writes: AtomicU64,
    /// The last window base that missed (for the log).
    pub pramin_last_miss: AtomicU64,
    /// fn 70 entries written into our roots.
    pub bar_pdes: AtomicU64,
    /// Page-directory statements applied as roots.
    pub roots: AtomicU64,
    /// ★ P6b: of those, roots that MOVED (walked at the next sync point, not at the statement).
    pub root_moves: AtomicU64,
    /// Statements refused by name.
    pub refused: AtomicU64,
    /// ★ P5c: mirrors created, and the ns their host verbs cost (space + two windows), summed/max.
    pub mirrors: AtomicU64,
    /// Sum of mirror-creation ns.
    pub mirror_ns: AtomicU64,
    /// ★ P5c: mirrors that reused a retired host space (no host verb).
    pub mirrors_reused: AtomicU64,
    /// ★ P5c: mirrors retired (the guest freed the VA space), recycled or freed.
    pub mirrors_retired: AtomicU64,
    /// ★ P5c: retired with live channels — kept, never recycled.
    pub mirrors_kept_live: AtomicU64,
    /// The slowest mirror creation, ns.
    pub mirror_ns_max: AtomicU64,
    /// ★ Cold-box fix: spare spaces built before the guest ran ([`prewarm`]).
    pub prewarmed: AtomicU64,
}

// ★ Owner ruling 2026-09-25: a PRAMIN re-point is ONE `mmap` on the vCPU. With a tiled scratch
// that holds only while PRAMIN is its own tile, since every run inside one tile is one piece.
const _: () = assert!(
    kf_linux_raw::scratch_tile_len(kf_trap::trappolicy::PRAMIN_LEN)
        == kf_trap::trappolicy::PRAMIN_LEN,
    "PRAMIN must be its own scratch tile, or its trap's sink becomes more than one mmap"
);

/// ★★★ **One guest window and its scratch, exactly as realize builds them** (2026-10-03).
///
/// The window is `len` bytes of host range under one memslot; before anyone can see it, every page
/// of it shows the window's [`ScratchTile`], so it is never a hole. The tile is
/// [`kf_linux_raw::scratch_tile_len`]`(len)` bytes, mapped again and again (offset `o` shows tile
/// byte `o % tile`), so a guest that touches every unmapped page makes the host allocate at most one
/// tile, not the window. Until 2026-10-03 the scratch was one memfd of the whole window and a guest
/// READ of every page allocated all of it (`V3_P4_PORT_MAP.md` Q3).
///
/// `name` is the memfd's creation name: an operator finds it as `memfd:<name>` in
/// `/proc/<pid>/fd` and reads its allocation with `stat -L -c %b`.
///
/// # Errors
/// The window's `mmap`, the tile's memfd, or a cover placement, by name.
pub fn window_with_scratch(
    len: u64,
    page: HostPageSize,
    what: &str,
    name: &std::ffi::CStr,
) -> Result<(GuestWindow, ScratchTile), String> {
    let w =
        GuestWindow::create(len, page).map_err(|e| format!("{what} window of {len:#x}: {e:?}"))?;
    let s = ScratchTile::for_window(name, &w)
        .map_err(|e| format!("{what} scratch tile for {len:#x}: {e:?}"))?;
    // ⊘ Scratch over the WHOLE window before anyone can see it: never a hole. ★ With the window's
    // flags (2026-10-03), set BEFORE QEMU registers the window (kf3.c runs kf3_realize first): its
    // own `ram_block_add` advice then finds them set and changes nothing, and every later sink
    // carries the same flags and merges back (`kf_linux_raw::WINDOW_ADVICE`).
    let mmaps = s
        .cover_advised(&w, HostOffset::ZERO, len)
        .map_err(|e| format!("{what} scratch placement: {e:?}"))?;
    eprintln!(
        "kf3: {what} scratch: tile {:#x} x {mmaps} over {len:#x} (host RAM bound {:#x})",
        s.tile_len(),
        s.tile_len()
    );
    Ok((w, s))
}

/// ★★★ **The memory plane's shared half** — what the vCPU and the VA thread both reach.
pub struct MemPlane {
    /// The three `MMU_INVALIDATE` registers.
    pub port: InvalidatePort,
    /// The PRAMIN window's views.
    pub pramin: PraminPool<WindowOps>,
    /// The PRAMIN trap's node pool and reaper.
    pub pramin_trap: &'static TrapNodes,
    /// The PRAMIN window-base register of this family.
    pub pramin_reg: WindowReg,
    pramin_want: AtomicU64,
    /// The PRAMIN window (1 MiB).
    pub pramin_win: &'static GuestWindow,
    /// The BAR1 window.
    pub bar1_win: &'static GuestWindow,
    /// The BAR2 (PCI BAR3) window.
    pub bar2_win: &'static GuestWindow,
    /// The statements' inbox.
    pub inbox: std::sync::Arc<Inbox>,
    /// OUR BAR1 root (store offset).
    pub bar1_root: u64,
    /// OUR BAR2 root (store offset).
    pub bar2_root: u64,
    /// Counters.
    pub counters: MemCounters,
    /// Guest RAM as QEMU registered it.
    pub(crate) ram: &'static RamMap,
    /// ★ The guest-RAM host object — an OS descriptor over the WHOLE guest memfd (gate 3's
    /// recipe), created once, on the VA thread, the first time a space needs it: `(handle, len)`.
    ram_obj: std::sync::OnceLock<Result<(u32, u64), String>>,
    /// ★ P5: every mirrored space, for the channel plane.
    pub mirrors: Mirrors,
    /// The store's length (the identity window's).
    pub fb_len: u64,
    /// ★ P5c: retired host spaces ready for reuse (VA thread only).
    spares: Mutex<Vec<Spare>>,
}

impl MemPlane {
    /// Build the windows, their scratch and the PRAMIN pool (views armed HERE, off the vCPU).
    ///
    /// # Errors
    /// Any refusal, by name — the VM must not start with a window that is a hole.
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        rm: &'static HostRm,
        family: kf_chip::Family,
        store: u32,
        layout: &kf_chip::bar0::FbLayout,
        bar1_bytes: u64,
        bar2_bytes: u64,
        ram: &'static RamMap,
        inbox: std::sync::Arc<Inbox>,
        mirrors: Mirrors,
    ) -> Result<(MemPlane, WindowOps, WindowOps), String> {
        let page = HostPageSize::query();
        // The windows live for the VM: leaked, like every other piece the vCPU paths borrow.
        let window = |len: u64,
                      what: &str,
                      name: &std::ffi::CStr|
         -> Result<(&'static GuestWindow, &'static ScratchTile), String> {
            let (w, s) = window_with_scratch(len, page, what, name)?;
            Ok((Box::leak(Box::new(w)), Box::leak(Box::new(s))))
        };
        let pramin_len = kf_trap::trappolicy::PRAMIN_LEN;
        let (pramin_win, pramin_scratch) = window(pramin_len, "PRAMIN", c"kf3-scratch-pramin")?;
        let (bar1_win, bar1_scratch) = window(bar1_bytes, "BAR1", c"kf3-scratch-bar1")?;
        let (bar2_win, bar2_scratch) = window(bar2_bytes, "BAR2", c"kf3-scratch-bar2")?;
        eprintln!(
            "kf3: scratch host-RAM bound {:#x} for this device (PRAMIN {:#x} + BAR1 {:#x} + BAR2 \
             {:#x}); whole-window scratch would have allowed {:#x}",
            pramin_scratch
                .tile_len()
                .saturating_add(bar1_scratch.tile_len())
                .saturating_add(bar2_scratch.tile_len()),
            pramin_scratch.tile_len(),
            bar1_scratch.tile_len(),
            bar2_scratch.tile_len(),
            pramin_len
                .saturating_add(bar1_bytes)
                .saturating_add(bar2_bytes),
        );
        // ★ PRAMIN (the one window with a trap) runs on the vCPU: no advice there.
        let ops = |name, win, scratch, trap: Option<&'static TrapNodes>| WindowOps {
            name,
            rm,
            store,
            win,
            scratch,
            ram,
            trap,
            advise: window_advises(trap.is_some()),
        };
        let trap = TrapNodes::start(rm, store)?;
        let pramin = PraminPool::new(
            ops("PRAMIN", pramin_win, pramin_scratch, Some(trap)),
            GRANULE,
            Box::new(move |gpa, len| ram.file_range(gpa, len).map(|(_, off)| off)),
        );
        let regs = kf_trap::InvalidateRegs::from_usermode_base(kf_trap::memmap::VF_USERMODE_PAGE)
            .ok_or("MMU_INVALIDATE registers: the usermode base is below the PRIV delta")?;
        Ok((
            MemPlane {
                port: InvalidatePort::new(regs),
                pramin,
                pramin_trap: trap,
                pramin_reg: WindowReg::for_family(family),
                pramin_want: AtomicU64::new(0),
                pramin_win,
                bar1_win,
                bar2_win,
                inbox,
                bar1_root: layout.bar1_pde_base,
                bar2_root: layout.bar2_pde_base,
                counters: MemCounters::default(),
                ram,
                ram_obj: std::sync::OnceLock::new(),
                mirrors,
                fb_len: layout.fb_length,
                spares: Mutex::new(Vec::new()),
            },
            ops("BAR1", bar1_win, bar1_scratch, None),
            ops("BAR2", bar2_win, bar2_scratch, None),
        ))
    }

    /// ★ The guest-RAM object: an OS descriptor over the whole guest memfd, so a sysmem leaf at
    /// memfd offset `o` is object offset `o` — the same numbering `RamMap::file_range` gives the
    /// VA manager. Created ONCE, on the VA thread (never a vCPU: it pins every page).
    ///
    /// ⚠ It pins all of guest RAM on the host for the VM's life — the same posture as gate 3's
    /// Translated channel, and the reason `memory-backend-memfd,share=on` is required.
    ///
    /// ⊘ **"No fd-backed block" is never remembered** — only the build's own outcome is. The fd
    /// is read ONCE, outside the cell, and handed to the build ([`once_after`]): QEMU's listener
    /// re-renders guest RAM at machine reset (a region deleted, then added again), and a second
    /// lookup inside the cell caught that gap once in 60 boots (`traces/v3_cifix/`, the first
    /// `--timer` arm at `3f67ed95`): the cached "no RAM" refused every sysmem leaf of that VM, and
    /// its CeUtils self-test timed out.
    ///
    /// # Errors
    /// No fd-backed guest RAM yet (not remembered), the mapping, or the host's refusal (by name,
    /// and remembered).
    pub fn guest_ram_object(&self, rm: &'static HostRm) -> Result<(u32, u64), String> {
        once_after(
            &self.ram_obj,
            || self.ram.backing_fd(),
            "no fd-backed guest RAM block (memory-backend-memfd,share=on?)",
            |fd| {
                let borrowed = fd.borrow();
                let len = borrowed
                    .try_clone_to_owned()
                    .map(std::fs::File::from)
                    .and_then(|f| f.metadata())
                    .map_err(|e| format!("guest memfd size: {e}"))?
                    .len();
                let view = kf_linux_raw::MappedRegion::map(
                    Backing::SharedFile {
                        fd: borrowed,
                        offset: 0,
                    },
                    len,
                    kf_linux_raw::HostProt::ReadWrite,
                    kf_linux_raw::CachePolicy::WriteBack,
                    HostPageSize::query(),
                )
                .map_err(|e| format!("guest memfd view of {len:#x}: {e:?}"))?;
                let view: &'static kf_linux_raw::MappedRegion = Box::leak(Box::new(view));
                let obj = rm
                    .alloc_os_descriptor(view, HostOffset::new(0), len)
                    .map_err(|e| format!("guest-RAM OS descriptor of {len:#x}: {e:?}"))?;
                eprintln!("kf3: guest-RAM object {obj:#x} over {len:#x} bytes of the guest memfd");
                Ok((obj, len))
            },
        )
    }

    /// Whether [`Self::guest_ram_object`] has an outcome to remember yet (built, or refused by
    /// the host). `false` after a "no RAM registered yet" answer, which is never cached.
    #[must_use]
    pub fn guest_ram_object_settled(&self) -> bool {
        self.ram_obj.get().is_some()
    }

    /// The PRAMIN plan for a window-base word: what each slot must show.
    fn pramin_plan(&self, raw: u32) -> [SlotSource; SLOTS] {
        let w = self.pramin_reg.decode(raw);
        let mut out = [SlotSource::Nothing; SLOTS];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = match (w.target, w.slot_addr(i)) {
                (WinTarget::Vidmem, Some(a)) => SlotSource::Store(a),
                (WinTarget::SysCoherent | WinTarget::SysNonCoherent, Some(a)) => SlotSource::Ram(a),
                _ => SlotSource::Nothing,
            };
        }
        out
    }

    /// ★ **vCPU, in the trap**: the guest wrote the window base. Re-point PRAMIN: one host map +
    /// one `mmap` (owner ruling 2026-09-25 — the ONE sanctioned vCPU syscall). ⊘ No lock: two racing writers each re-check the latest word after placing, so the
    /// last to finish always leaves the latest window in place.
    pub fn pramin_write(&self, raw: u32) {
        let t0 = std::time::Instant::now();
        self.pramin_want
            .store(u64::from(raw) | (1 << 32), Ordering::Release);
        loop {
            let want = self.pramin_want.load(Ordering::Acquire);
            let r = self.pramin.repoint(&self.pramin_plan(want as u32));
            if r.missed > 0 || r.refused > 0 {
                self.counters
                    .pramin_miss_writes
                    .fetch_add(1, Ordering::Relaxed);
                self.counters
                    .pramin_last_miss
                    .store(want & 0xFFFF_FFFF, Ordering::Relaxed);
            }
            if self.pramin_want.load(Ordering::Acquire) == want {
                break;
            }
        }
        let ns = u64::try_from(t0.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.pramin.worst_ns.fetch_max(ns, Ordering::Relaxed);
    }

    /// ★ **vCPU, in the trap**: a write to one of the three invalidate registers. Returns the
    /// word the BAR0 read shadow must now hold, or `None` when `off` is not one of them.
    /// ⊘ Atomics and at most one eventfd write.
    pub fn invalidate_write(&self, off: u64, val: u32) -> Option<u32> {
        match self.port.write(off, val) {
            PortWrite::NotOurs => None,
            PortWrite::Latched => self.port.read(off),
            PortWrite::Publish(_) => {
                self.counters.invalidates.fetch_add(1, Ordering::Relaxed);
                let v = self.port.read(off);
                let _ = self.inbox.wake.signal();
                v
            }
        }
    }
}

/// ★ P1+P2 review fix (2026-10-04) — **the host verbs a twin path may call**: [`HostRm`] in kf3, a
/// recorder in the tests, so "no window in any twin" is tested on the paths themselves (create,
/// prewarm, reuse, retire) rather than on a predicate. ★ 2026-10-10 (§AB rule 2): these are the
/// ONLY host verbs the twin life cycle issues outside the walker's guest-leaf rows
/// ([`GpuMirror`]'s `MapTarget`), and the only mapping verb among them is the positive control's.
pub trait TwinHost {
    /// A host VA space for a twin ([`HostRm::alloc_vaspace`]).
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn alloc_space(&self) -> Result<kf_host::VaSpace, String>;
    /// A store window in `space`, `GROWS_DOWN`. ⊘ ONLY the positive control
    /// ([`negctl_twin_window`]) calls it: §AB rule 2 forbids any window in a twin.
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn map_window(&self, space: kf_host::VaSpace, memory: u32, len: u64) -> Result<u64, String>;
}

impl TwinHost for HostRm {
    fn alloc_space(&self) -> Result<kf_host::VaSpace, String> {
        self.alloc_vaspace().map_err(|e| format!("{e:?}"))
    }
    fn map_window(&self, space: kf_host::VaSpace, memory: u32, len: u64) -> Result<u64, String> {
        HostRm::map_window(self, space, memory, len, true).map_err(|e| format!("{e:?}"))
    }
}

/// ★ `KF3_NEGCTL_TWIN_WINDOW=1` — the POSITIVE CONTROL of "no window in any twin" (review fix
/// 2026-10-04): every new twin and every prewarmed spare ALSO gets a store window, recorded in its
/// record, so its log line names it and the box-log gate's `WINDOWS=NONE` must fail. It
/// reintroduces exactly the S1-21 defect, and it is the ONE path that breaks `OWNER_RULINGS.md`
/// §AB rule 2 — kept (2026-10-10) because the owner keeps every measurement and control flag; never
/// set it outside a dedicated control run. Default OFF; read once.
#[must_use]
pub fn negctl_twin_window() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KF3_NEGCTL_TWIN_WINDOW").is_some_and(|v| v != "0"))
}

/// ★ A T-mode twin's record — its [`Mirror`] and the VMM ranges a guest leaf must avoid — both
/// derived from what was actually MAPPED in its space, never asserted.
pub struct TwinRecord {
    /// The mirror the channel plane sees.
    pub mirror: Mirror,
    /// Our placements in the space a guest leaf may not overlap.
    pub reserved: Vec<(u64, u64)>,
}

/// ★★ P1+P2 inc D (`V3_P1P2_TSPACE.md` §4.1) — **a fresh T-mode twin**: the host space and nothing
/// of ours — no window, no ring region, so no VMM range a guest leaf must avoid (guest leaves at
/// `[RING_REGION_BASE, 2^40)` are accepted) — and always recorded: its record never hangs on a
/// window. `negctl` (the positive control) maps one store window of `store_len` and records it.
pub fn tmode_twin(
    host: &dyn TwinHost,
    key: VasKey,
    space: kf_host::VaSpace,
    ram_obj: Option<u32>,
    store: u32,
    store_len: u64,
    negctl: bool,
) -> TwinRecord {
    let fb = negctl
        .then(|| host.map_window(space, store, store_len).ok())
        .flatten();
    twin_record(key, space, fb.map(|b| (b, store_len)), ram_obj)
}

/// ★ A prewarmed spare: a plain space (the space and its reservations cost RM calls), with no
/// window to pay for — `negctl` as for [`tmode_twin`].
pub fn tmode_spare(
    host: &dyn TwinHost,
    space: kf_host::VaSpace,
    ram_obj: Option<u32>,
    store: u32,
    store_len: u64,
    negctl: bool,
) -> Spare {
    let fb = negctl
        .then(|| host.map_window(space, store, store_len).ok())
        .flatten();
    Spare {
        space,
        negctl_window: fb.map(|b| (b, store_len)),
        ram_obj,
    }
}

/// ★ The prewarm path's spare: a host space from `host`, then [`tmode_spare`].
///
/// # Errors
/// The host's refusal of the space, by name.
pub fn prewarm_spare(
    host: &dyn TwinHost,
    ram_obj: Option<u32>,
    store: u32,
    store_len: u64,
    negctl: bool,
) -> Result<Spare, String> {
    let space = host.alloc_space()?;
    Ok(tmode_spare(host, space, ram_obj, store, store_len, negctl))
}

/// ★ A recycled spare as a twin — no host verb; the record says what the space holds (a spare
/// holds nothing of ours, unless the positive control ran).
#[must_use]
pub fn tmode_reuse(key: VasKey, sp: &Spare) -> TwinRecord {
    twin_record(key, sp.space, sp.negctl_window, sp.ram_obj)
}

/// ★ The retire path's spare: a retired mirror whose rows were all unmapped keeps its host space
/// for the next guest VA space, carrying exactly what the mirror held besides its rows (nothing,
/// unless the positive control ran). `None` when an unmap was refused: the space is not clean and
/// is freed instead.
#[must_use]
pub fn retire_spare(mi: &Mirror, refused: usize) -> Option<Spare> {
    (refused == 0).then_some(Spare {
        space: mi.space,
        negctl_window: mi.negctl_window,
        ram_obj: mi.ram_obj,
    })
}

fn twin_record(
    key: VasKey,
    space: kf_host::VaSpace,
    negctl_window: Option<(u64, u64)>,
    ram_obj: Option<u32>,
) -> TwinRecord {
    TwinRecord {
        mirror: Mirror {
            space,
            negctl_window,
            rows: PlacedRows::default(),
            ram_obj,
            live: Default::default(),
            kernel_vas: kernel_vas_for(key),
            log: std::sync::Arc::default(),
            ledger: None,
        },
        // ⊘ A twin has no ring region and no window: only the positive control's window, when
        // it mapped one, is a VMM range here.
        reserved: negctl_window
            .map(|(b, l)| vec![(b, b.saturating_add(l))])
            .unwrap_or_default(),
    }
}

/// ★ The windows a mirror carries, as its log line names them — DERIVED from the record (review
/// fix 2026-10-04: the T-mode lines printed a literal `windows=none` whatever was mapped).
#[must_use]
pub fn windows_text(m: &Mirror) -> String {
    window_words(m.negctl_window)
}

/// [`windows_text`] for a spare.
#[must_use]
pub fn spare_windows_text(sp: &Spare) -> String {
    window_words(sp.negctl_window)
}

fn window_words(w: Option<(u64, u64)>) -> String {
    match w {
        None => "windows=none".into(),
        Some((b, l)) => format!("windows fb={b:#x}+{l:#x} ram=NONE"),
    }
}

/// ★ P1+P2 inc D: record a T-mode twin — its [`TwinRecord`] in the channel plane's table and the
/// walker's, the two sharing its rows and its state word.
fn record_twin(
    plane: &MemPlane,
    key: VasKey,
    rec: TwinRecord,
    m: &mut Manager,
    rm: &'static HostRm,
    store: u32,
) {
    let TwinRecord {
        mut mirror,
        reserved,
    } = rec;
    let (space, ram_obj, rows, log, kernel_vas) = (
        mirror.space,
        mirror.ram_obj,
        mirror.rows.clone(),
        mirror.log.clone(),
        mirror.kernel_vas.clone(),
    );
    let ledger = std::sync::Arc::new(new_ledger(HostVas {
        rm,
        space,
        store,
        ram_obj,
    }));
    mirror.ledger = Some(ledger.clone());
    if let Ok(mut mm) = plane.mirrors.lock() {
        mm.insert(key, mirror);
    }
    m.table.insert(
        key,
        Target::Gpu(GpuMirror::with_ledger(
            ledger,
            (rows, log),
            reserved,
            Some(plane.ram),
            kernel_vas,
        )),
    );
}

/// ★ Build a fresh mirror for `key`: a host space and nothing of ours in it (§AB rule 2; the P5
/// §12 windows are deleted). Returns the log line on refusal.
fn create_mirror(
    m: &mut Manager,
    plane: &MemPlane,
    rm: &'static HostRm,
    store: u32,
    key: VasKey,
) -> Result<(), String> {
    let t_mirror = std::time::Instant::now();
    let us = |t: std::time::Instant| t.elapsed().as_micros();
    let t_step = std::time::Instant::now();
    let space = match TwinHost::alloc_space(rm) {
        Ok(space) => space,
        Err(e) => {
            plane.counters.refused.fetch_add(1, Ordering::Relaxed);
            return Err(format!("pagedir {key:?}: host VA space refused: {e}"));
        }
    };
    // ⊘ A space without the RAM object refuses every sysmem leaf and leaves the trigger armed —
    // measured p4b4: a ~22 s guest stall per boot.
    let vas_us = us(t_step);
    let t_step = std::time::Instant::now();
    let ram_obj = match plane.guest_ram_object(rm) {
        Ok(o) => Some(o),
        Err(e) => {
            eprintln!("kf3: {key:?}: {e} — its sysmem leaves will be refused");
            None
        }
    };
    let ram_obj_us = us(t_step);
    // ★★ P1+P2 inc D (`V3_P1P2_TSPACE.md` §4.1), hardwired 2026-10-10 (§AB rule 2): a twin carries
    // NO window and NO ring — only rows derived from the guest's own page tables — and its record
    // never hangs on a window. ★ Review fix 2026-10-04: the record is built by [`tmode_twin`]
    // (tested against a host that counts window maps) and the line is derived from it
    // ([`windows_text`]).
    let rec = tmode_twin(
        rm,
        key,
        space,
        ram_obj.map(|(o, _)| o),
        store,
        plane.fb_len,
        negctl_twin_window(),
    );
    let windows = windows_text(&rec.mirror);
    record_twin(plane, key, rec, m, rm, store);
    let ns = u64::try_from(t_mirror.elapsed().as_nanos()).unwrap_or(u64::MAX);
    plane.counters.mirrors.fetch_add(1, Ordering::Relaxed);
    plane.counters.mirror_ns.fetch_add(ns, Ordering::Relaxed);
    plane
        .counters
        .mirror_ns_max
        .fetch_max(ns, Ordering::Relaxed);
    eprintln!(
        "kf3: {key:?} mirror space={:#x}: {windows} rings=none ({} us: vaspace {vas_us} ram_obj {ram_obj_us})",
        space.space,
        ns / 1000
    );
    Ok(())
}

/// ★★★ **Cold-box fix — the first mirror's one-time cost, paid BEFORE the guest runs.**
///
/// `[measured pr1]` the first mirror of a boot took 7.6 s on a freshly provisioned box (2-7 s on
/// every later boot, `mirror space=0xcafe000c` in every `fast_*_qemu.log`; later mirrors ~0.1 s),
/// and it is built INSIDE the guest's held `COPY_SERVER_RESERVED_PDES` (`0x90f10106`) reply —
/// whose GSP RPC timeout is 6 s (`[pr1]` Xid 119 *"Timeout after 6s of waiting for RPC response
/// … 0x90f10106"*, then the PMA scrubber's construction fails `0x40`). The one-time part is the
/// guest-RAM OS descriptor (host RM pins every page of the guest memfd) plus the first host VA
/// space (and, before 2026-10-10, its two window maps). Here, on the VA thread as soon as QEMU has
/// registered an fd-backed RAM block (realize's listener replay — long before the guest's driver
/// loads), we build the guest-RAM object and ONE spare space, so the first page-directory
/// statement takes the spare (`mirrors_reused`) instead of paying. ⊘ 2026-10-10 (§AB rule 2): the
/// spare holds no window.
///
/// Returns `None` while guest RAM is not registered yet (the caller retries next tick; the
/// `OnceLock` in [`MemPlane::guest_ram_object`] must never cache a "no RAM yet" refusal), else
/// the log line. ⊘ Nothing here is guest-visible or guest-chosen: our space.
pub fn prewarm(plane: &MemPlane, rm: &'static HostRm, store: u32) -> Option<String> {
    plane.ram.backing_fd()?;
    let t0 = std::time::Instant::now();
    let ram_obj = plane.guest_ram_object(rm);
    // The block went away between the two lookups (QEMU re-rendering guest RAM): nothing was
    // cached, so try again next tick instead of spending a spare on a space with no RAM window.
    if ram_obj.is_err() && !plane.guest_ram_object_settled() {
        return None;
    }
    let ram_obj_us = t0.elapsed().as_micros();
    let t1 = std::time::Instant::now();
    // ★ P1+P2 inc D (§4.1), hardwired 2026-10-10 (§AB rule 2): spares are plain spaces — still
    // prewarmed (the space and its reservations cost RM calls), with no window to pay for. ★ Review
    // fix 2026-10-04: the line names what the spare's record holds ([`spare_windows_text`]).
    let sp = match prewarm_spare(
        rm,
        ram_obj.ok().map(|(o, _)| o),
        store,
        plane.fb_len,
        negctl_twin_window(),
    ) {
        Ok(sp) => sp,
        Err(e) => {
            return Some(format!(
                "prewarm: host VA space refused: {e} (ram_obj {ram_obj_us} us)"
            ));
        }
    };
    let vas_us = t1.elapsed().as_micros();
    let windows = spare_windows_text(&sp);
    let space = sp.space.space;
    if let Ok(mut v) = plane.spares.lock() {
        v.push(sp);
    }
    plane.counters.prewarmed.fetch_add(1, Ordering::Relaxed);
    Some(format!(
        "prewarm: spare host space {space:#x} ready before the guest runs — {windows} (vaspace {vas_us} us, ram_obj {ram_obj_us} us, total {} us)",
        t0.elapsed().as_micros()
    ))
}

/// ★ P5c: the guest freed VA space `key` — unmap OUR rows (deferred, one invalidate) and keep the
/// host space as a spare (it holds nothing of ours, §AB rule 2). ⊘ A space a live channel still
/// runs in is never recycled (nor freed): it is kept, named.
fn retire_mirror(m: &mut Manager, plane: &MemPlane, rm: &'static HostRm, key: VasKey) -> String {
    let mirror = plane.mirrors.lock().ok().and_then(|mut mm| mm.remove(&key));
    let Some(target) = m.remove(key) else {
        return format!("retire {key:?}: no mirror (no page-directory statement named it)");
    };
    plane
        .counters
        .mirrors_retired
        .fetch_add(1, Ordering::Relaxed);
    let Target::Gpu(g) = target else {
        return format!("retire {key:?}: not a GPU mirror — kept");
    };
    let live = mirror
        .as_ref()
        .map_or(0, |mi| mi.live.load(Ordering::Acquire));
    if live > 0 {
        plane
            .counters
            .mirrors_kept_live
            .fetch_add(1, Ordering::Relaxed);
        return format!(
            "retire {key:?}: {live} live channel(s) still run in host space {:#x} — KEPT, never recycled",
            g.vas.space.space
        );
    }
    // ★ Our placements in this space, from the space's own row record (what we PLACED, never a
    // copy of the guest's tables); the walker's slot for it is released by `m.remove`.
    // ★ V3_BATCHED_MAP: VA-contiguous rows go as one range each; every batch object is freed.
    let t_unmap = std::time::Instant::now();
    let (nrows, mut refused, calls) = g.unmap_all_rows();
    let host_calls = calls.saturating_add(u64::from(nrows > 0));
    if nrows > 0 && g.invalidate().is_err() {
        refused = refused.saturating_add(1);
    }
    let unmap_us = t_unmap.elapsed().as_micros();
    let recycled = match mirror.as_ref().and_then(|mi| retire_spare(mi, refused)) {
        Some(sp) => plane
            .spares
            .lock()
            .ok()
            .filter(|v| v.len() < SPARES_MAX)
            .map(|mut v| v.push(sp))
            .is_some(),
        None => false,
    };
    if !recycled {
        // A refused unmap means the space is not clean — freed.
        rm.free_vaspace(g.vas.space);
    }
    format!(
        "retire {key:?}: {nrows} row(s) unmapped ({refused} refused) in {unmap_us} us, {host_calls} host call(s), host space {:#x} {} — {}",
        g.vas.space.space,
        if recycled { "kept as a spare" } else { "freed" },
        g.calls_line()
    )
}

/// Diagnostics: every VA-space object the plane holds, `key=slot@root:rows`.
fn vas_census(m: &Manager) -> String {
    let objs = m.table.objects();
    let mut out = format!("{} objects:", objs.len());
    for (k, root, slot) in objs {
        let rows = match m.table.target(k) {
            Some(Target::Gpu(g)) => g.rows.read().map_or(usize::MAX, |r| r.len()),
            _ => 0,
        };
        out.push_str(&format!(
            " {:#x}=s{}@{}:{rows}",
            k.0,
            slot.map_or(-1, i64::from),
            root.map_or("-".to_string(), |r| format!("{r:#x}"))
        ));
    }
    out
}

/// ★ Apply one statement on the VA thread. Returns a line for the boot log.
///
/// - **fn 70**: write the 8-byte entry into entry 0 of OUR root on the GPU (the root is ours, so
///   this is not a guest table we store), then walk it (Q10: a root that changed with no
///   invalidate behind it is walked now).
/// - **page directory**: the root becomes an attribute of that VA-space object; a new object
///   gets a host VA space (the Translated plane's mirror) the first time it is named.
pub fn apply_statement(
    m: &mut Manager,
    plane: &MemPlane,
    rm: &'static HostRm,
    store: u32,
    st: MemStatement,
    trigger: &kf_trap::Trigger,
) -> String {
    match st {
        MemStatement::BarPde(p) => {
            let root = match p.bar {
                BarAperture::Bar1 => plane.bar1_root,
                BarAperture::Bar2 => plane.bar2_root,
            };
            // ★ §13: a store-bounded write — no device pointer leaves the walker.
            let w = m.walker().kernel.write_store(root, &p.entry.to_le_bytes());
            if let Err(e) = w {
                plane.counters.refused.fetch_add(1, Ordering::Relaxed);
                return format!(
                    "fn70 {:?} entry={:#x}: GPU write into our root @{root:#x} REFUSED: {e}",
                    p.bar, p.entry
                );
            }
            plane.counters.bar_pdes.fetch_add(1, Ordering::Relaxed);
            m.schedule_walk(
                if p.bar == BarAperture::Bar2 {
                    K_BAR2
                } else {
                    K_BAR1
                },
                trigger,
            );
            format!(
                "fn70 {:?} entry={:#x} shift={} -> our root @{root:#x}, walk scheduled",
                p.bar, p.entry, p.level_shift
            )
        }
        // ★★★ v3-refusals: the guest's sysmembar IS the host GPU's — the authored, unprivileged
        // `FB_FLUSH_GPU_CACHE(FB_FLUSH_YES)` (the verb that serves a Hopper+ guest's token-register
        // sysmembar). The statement settles only after it returns, so the held `NV_OK` is posted
        // after the flush (`kf_rm::sysmembar`). A refused host verb is NAMED; the reply stays
        // `NV_OK`, exactly as the Hopper+ register path releases its register — the guest's
        // alternative is the same unflushed state with no name on it.
        MemStatement::Sysmembar => {
            let t = std::time::Instant::now();
            let r = rm.fb_flush();
            let n = plane
                .counters
                .sysmembars
                .fetch_add(1, Ordering::Relaxed)
                .saturating_add(1);
            match r {
                Ok(()) => format!(
                    "sysmembar #{n}: host FB_FLUSH_GPU_CACHE(FB_FLUSH_YES) in {} us",
                    t.elapsed().as_micros()
                ),
                Err(e) => {
                    plane.counters.refused.fetch_add(1, Ordering::Relaxed);
                    format!("sysmembar #{n}: host FB_FLUSH_GPU_CACHE REFUSED: {e:?}")
                }
            }
        }
        // ★★ v3-refusals: the guest withdrew this object's page directory — stop reading it
        // (UVM frees it right after the held reply). ⊘ No unmap: the rows stay until a walk or
        // the object's retirement says otherwise (`VasTable::clear_root`).
        MemStatement::UnsetPageDir { client, vaspace } => {
            let key = VasKey((u64::from(client) << 32) | u64::from(vaspace));
            let had = m.table.root(key);
            m.table.clear_root(key);
            plane.counters.root_unsets.fetch_add(1, Ordering::Relaxed);
            format!(
                "unset pagedir {key:?}: root {} withdrawn — no walk reads it again",
                had.map_or("(none)".into(), |r| format!("{r:#x}"))
            )
        }
        MemStatement::Retire { client, vaspace } => {
            let line = retire_mirror(
                m,
                plane,
                rm,
                VasKey((u64::from(client) << 32) | u64::from(vaspace)),
            );
            if std::env::var_os("KF_VAS_CENSUS").is_some() {
                eprintln!("kf3: census {line}");
            }
            line
        }
        MemStatement::PageDir(s) => {
            let key = VasKey((u64::from(s.client.0) << 32) | u64::from(s.vaspace.0));
            let ap = match s.pdb_aperture {
                Some(kf_arch::Aperture::Vidmem) => PdbAperture::Vidmem,
                Some(kf_arch::Aperture::SysmemCoherent | kf_arch::Aperture::SysmemNonCoherent) => {
                    PdbAperture::Sysmem
                }
                other => {
                    plane.counters.refused.fetch_add(1, Ordering::Relaxed);
                    return format!(
                        "pagedir {key:?} pdb={:#x}: aperture {other:?} refused",
                        s.pdb.0
                    );
                }
            };
            if m.table.target(key).is_none() {
                let reused = plane.spares.lock().ok().and_then(|mut v| v.pop());
                if let Some(sp) = reused.as_ref() {
                    // ★ P1+P2 inc D: a recycled spare is a plain space (§AB rule 2); classified
                    // afresh.
                    // ★ Review fix 2026-10-04: recorded with what its space holds and LOGGED.
                    let rec = tmode_reuse(key, sp);
                    let windows = windows_text(&rec.mirror);
                    record_twin(plane, key, rec, m, rm, store);
                    plane
                        .counters
                        .mirrors_reused
                        .fetch_add(1, Ordering::Relaxed);
                    eprintln!(
                        "kf3: {key:?} mirror space={:#x}: {windows} rings=none (recycled)",
                        sp.space.space
                    );
                } else if let Err(line) = create_mirror(m, plane, rm, store, key) {
                    return line;
                }
            }
            match m.table.set_root(key, s.pdb.0, ap) {
                Ok(change) => {
                    plane.counters.roots.fetch_add(1, Ordering::Relaxed);
                    // ★ P6b ruling (c): a FIRST root is walked now (Q10); a MOVED root at the next
                    // synchronisation point naming the space — its entries are written by the
                    // guest only after this statement's reply (`RootChange`'s rustdoc).
                    if change.walk_now() {
                        m.schedule_walk(key, trigger);
                    }
                    if let kf_mem::vasmgr::RootChange::Moved { .. } = change {
                        plane.counters.root_moves.fetch_add(1, Ordering::Relaxed);
                    }
                    if change.walk_now() && std::env::var_os("KF_VAS_CENSUS").is_some() {
                        eprintln!("kf3: census at {key:?} First: {}", vas_census(m));
                    }
                    format!("pagedir {key:?} root={:#x} {change:x?}", s.pdb.0)
                }
                Err(e) => {
                    plane.counters.refused.fetch_add(1, Ordering::Relaxed);
                    format!("pagedir {key:?}: root refused: {e:?}")
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The last commits of a rows log, newest last.
    #[test]
    fn a_rows_log_names_its_last_commits() {
        let log = RowsLog::default();
        assert!(log.recent(4).is_empty());
        for i in 0..6u64 {
            log.commit(i * 0x1000, i * 0x1000 + 0x1000);
        }
        let r: Vec<(u64, u64, u64)> = log.recent(2).iter().map(|c| (c.0, c.1, c.2)).collect();
        assert_eq!(r, vec![(5, 0x4000, 0x5000), (6, 0x5000, 0x6000)]);
        assert_eq!(log.epoch(), 6);
    }

    /// The death message names the rows around an unplaced VA, or says there are none.
    #[test]
    fn neighbours_of_an_unplaced_va_are_named() {
        let rows: PlacedRows = PlacedRows::default();
        assert_eq!(
            describe_neighbours(&rows, 0x2000),
            "rows=0 below=none above=none"
        );
        {
            let mut w = rows.write().unwrap();
            w.insert(0x1000, (0x1000, 0, true, kf_host::MapPerm::READ_WRITE));
            w.insert(0x5000, (0x2000, 0, false, kf_host::MapPerm::READ_WRITE));
        }
        assert_eq!(
            describe_neighbours(&rows, 0x2010),
            "rows=2 below=0x1000+0x1000 ram above=0x5000+0x2000"
        );
        assert!(resolve_placed_prefix(&rows, 0x2010).is_none());
    }

    /// ★ `traces/v3_cifix/`: a "not registered yet" answer is returned, never cached; the first
    /// time the precondition holds, the build runs with exactly the value it saw, and its result
    /// is what every later call gets — even if the precondition is absent again by then.
    #[test]
    fn a_not_yet_precondition_is_never_remembered() {
        let cell = std::sync::OnceLock::new();
        let r: Result<u32, String> = once_after(
            &cell,
            || None::<u32>,
            "no RAM yet",
            |_| panic!("no build without the precondition"),
        );
        assert_eq!(r, Err("no RAM yet".to_owned()));
        assert!(
            cell.get().is_none(),
            "the transient refusal must not be cached"
        );
        let r = once_after(
            &cell,
            || Some(7u32),
            "no RAM yet",
            |p| {
                assert_eq!(p, 7, "the build gets the value the precondition saw");
                Ok(p * 2)
            },
        );
        assert_eq!(r, Ok(14));
        let r = once_after(
            &cell,
            || None::<u32>,
            "no RAM yet",
            |_| panic!("built twice"),
        );
        assert_eq!(r, Ok(14), "a settled build outlives a later gap");
    }

    /// A refusal the BUILD makes (the host said no) is an outcome: remembered, never retried.
    #[test]
    fn a_build_refusal_is_remembered() {
        let cell = std::sync::OnceLock::new();
        let r: Result<u32, String> = once_after(
            &cell,
            || Some(1u32),
            "no RAM yet",
            |_| Err("host refused".to_owned()),
        );
        assert_eq!(r, Err("host refused".to_owned()));
        let r = once_after(&cell, || Some(1u32), "no RAM yet", |_| panic!("retried"));
        assert_eq!(r, Err("host refused".to_owned()));
    }

    /// The measured bases, at 12 GiB, are inside what is armed, and a 1 MiB window from each is
    /// wholly covered.
    /// ★ v3-appfix J: a released ring's slot is reused before a fresh one, only slots inside the
    /// region are accepted back, and a slot is never handed out twice.
    #[test]
    fn a_released_ring_slot_is_reused_and_never_doubled() {
        let slots = RingSlots::default();
        let a = take_ring_slot(&slots).expect("first slot");
        let b = take_ring_slot(&slots).expect("second slot");
        assert_eq!(a, RING_REGION_BASE);
        assert_eq!(b, RING_REGION_BASE + kf_chan::host::RING_BYTES);
        give_ring_slot(&slots, a);
        give_ring_slot(&slots, a);
        give_ring_slot(&slots, 0x1000);
        assert_eq!(
            take_ring_slot(&slots),
            Some(a),
            "the released slot comes back first"
        );
        assert_eq!(
            take_ring_slot(&slots),
            Some(RING_REGION_BASE + 2 * kf_chan::host::RING_BYTES),
            "a double give and a foreign VA are not slots"
        );
    }

    /// ★ P1+P2 inc A (`V3_P1P2_TSPACE.md` §7 test 14): a range that cuts a row at its START and
    /// one that cuts a row at its END both leave exactly host RM's remaining coverage — and a
    /// refused unmap puts the record back as it was.
    #[test]
    fn placed_rows_track_host_unmap_at_both_edges() {
        use std::collections::BTreeMap;
        const RW: kf_host::MapPerm = kf_host::MapPerm::READ_WRITE;
        const RO: kf_host::MapPerm = kf_host::MapPerm {
            read_only: true,
            ..kf_host::MapPerm::READ_WRITE
        };
        let base: BTreeMap<u64, PlacedRow> = [
            (0x1000, (0x3000, 0x10_0000, false, RO)), // [0x1000, 0x4000)
            (0x4000, (0x1000, 0x20_0000, true, RW)),  // [0x4000, 0x5000)
            (0x8000, (0x4000, 0x30_0000, false, RW)), // [0x8000, 0xC000)
        ]
        .into_iter()
        .collect();
        // Cut [0x2000, 0x9000): row 1 straddles the start, row 2 is inside, row 3 straddles the end.
        let mut rows = base.clone();
        let cut = cut_rows(&mut rows, 0x2000, 0x9000);
        let want: BTreeMap<u64, PlacedRow> = [
            (0x1000, (0x1000, 0x10_0000, false, RO)), // the part before, same backing and perm
            (0x9000, (0x3000, 0x30_1000, false, RW)), // the part after, its backing advanced
        ]
        .into_iter()
        .collect();
        assert_eq!(rows, want);
        // Nothing the host removed is still resolvable; everything it kept is.
        assert_eq!(resolve_in(&rows, 0x1800), Some((false, 0x10_0800)));
        assert_eq!(
            resolve_in(&rows, 0x2000),
            None,
            "stale coverage at the start"
        );
        assert_eq!(resolve_in(&rows, 0x4800), None);
        assert_eq!(resolve_in(&rows, 0x8800), None);
        assert_eq!(
            resolve_in(&rows, 0x9800),
            Some((false, 0x30_1800)),
            "missing coverage at the end"
        );
        uncut_rows(&mut rows, cut);
        assert_eq!(rows, base, "a refused unmap restores the record");
        // A range inside one row splits it in two.
        let mut rows = base.clone();
        cut_rows(&mut rows, 0x9000, 0xA000);
        assert_eq!(rows.get(&0x8000), Some(&(0x1000, 0x30_0000, false, RW)));
        assert_eq!(rows.get(&0xA000), Some(&(0x2000, 0x30_2000, false, RW)));
        // An empty range changes nothing.
        let mut rows = base.clone();
        cut_rows(&mut rows, 0x2000, 0x2000);
        assert_eq!(rows, base);
        // ★ Strict hardwired 2026-10-10: the range unmap's cut is always the exact one; a cut the
        // removal before inc A would have got wrong (a row straddling an edge) is counted.
        let mut rows = base.clone();
        assert!(cut_is_inexact(&rows, 0x2000, 0x9000));
        let before = ROWS_INEXACT.load(Ordering::Relaxed);
        let cut = cut_for(&mut rows, 0x2000, 0x9000);
        assert_eq!(rows, want, "always the exact cut");
        assert!(ROWS_INEXACT.load(Ordering::Relaxed) > before, "counted");
        uncut_rows(&mut rows, cut);
        assert_eq!(rows, base, "a refused unmap restores the record");
        // A range no row straddles is exact, and not counted as inexact.
        assert!(!cut_is_inexact(&base, 0x4000, 0x5000));
        let mut a = base.clone();
        let mut b = base.clone();
        cut_for(&mut a, 0x4000, 0x5000);
        cut_rows(&mut b, 0x4000, 0x5000);
        assert_eq!(a, b);
    }

    /// `(ram, offset)` of the row covering `va` (the readers' lookup, over a plain map).
    fn resolve_in(
        rows: &std::collections::BTreeMap<u64, PlacedRow>,
        va: u64,
    ) -> Option<(bool, u64)> {
        let (&start, &(len, off, ram, _)) = rows.range(..=va).next_back()?;
        (va < start + len).then(|| (ram, off + (va - start)))
    }

    /// ★ Review fix 2026-10-04 (§7.13) — **the rows' commit log**: every commit bumps the epoch
    /// and names the range it changed; `changed_since` finds the FIRST commit after an epoch that
    /// overlaps a range and when it landed, says `No` when none did, and `Unknown` once the log no
    /// longer reaches back to the epoch. A resolution reads the epoch under the same guard.
    #[test]
    fn the_rows_commit_log_answers_what_changed_since_a_bind() {
        use kf_chan::tmode::Changed;
        let log = RowsLog::default();
        let rows = PlacedRows::default();
        if let Ok(mut r) = rows.write() {
            r.insert(
                0x1000,
                (0x1000, 0x10_0000, false, kf_host::MapPerm::READ_WRITE),
            );
            log.commit(0x1000, 0x2000);
        }
        let (spans, epoch) = resolve_rows_epoch(&rows, &log, 0x1800, 0x100);
        assert_eq!((spans.map(|s| s.len()), epoch), (Ok(1), 1));
        assert_eq!(log.changed_since(1, 0x1800, 0x100), Changed::No);
        log.commit(0x9000, 0xA000); // elsewhere
        assert_eq!(log.changed_since(1, 0x1800, 0x100), Changed::No);
        log.commit(0x1000, 0x2000); // this row, epoch 3
        let Changed::At(t) = log.changed_since(1, 0x1800, 0x100) else {
            panic!("a change after the bind")
        };
        assert!(t <= std::time::Instant::now());
        assert_eq!(log.changed_since(3, 0x1800, 0x100), Changed::No);
        // Past the bounded log's reach: Unknown, never a guess.
        let full = RowsLog::default();
        for k in 0..(ROWS_LOG_MAX as u64 + 10) {
            full.commit(k << 12, (k + 1) << 12);
        }
        assert_eq!(full.changed_since(2, 0, 0x1000), Changed::Unknown);
        assert_eq!(
            full.changed_since(full.epoch(), 0, 0x1000),
            Changed::No,
            "nothing after the newest epoch"
        );
    }

    /// ★ Review fix 2026-10-04 (§3.5): a walk is pending while the VA thread says so or a split is
    /// queued; a channel waiting for one is rung once, when the VA thread takes the waiters.
    #[test]
    fn a_walk_waiter_is_registered_once_and_taken() {
        let inbox = Inbox::new().expect("inbox");
        assert!(!inbox.walk_busy());
        inbox.set_walk_busy(true);
        assert!(inbox.walk_busy());
        inbox.set_walk_busy(false);
        let t = inbox.request_split(7, None);
        assert!(inbox.walk_busy(), "a queued split is a pending walk");
        let _ = inbox.take_split_requests();
        assert!(inbox.walk_busy(), "a running split too");
        let _ = inbox.finish_split(t, Ok(()));
        assert!(!inbox.walk_busy());
        inbox.wait_walk(5);
        inbox.wait_walk(5);
        inbox.wait_walk(9);
        assert_eq!(inbox.take_walk_waiters(), vec![5, 9]);
        assert!(inbox.take_walk_waiters().is_empty());
    }

    /// ★★ OWNER_RULINGS §U.2: a gated split opens its gate only on success, only at its finish (on
    /// the VA thread, after the commit), and before its outcome is published; a failed split never
    /// opens it; a released ring turns the release into a failure.
    #[test]
    fn a_gated_split_releases_its_gate_only_after_a_successful_finish() {
        let inbox = Inbox::new().expect("inbox");
        let key = kf_mem::vasmgr::VasKey(0x1234);
        let g = kf_chan::host::Gate::scratch().expect("scratch gate");
        let t = inbox.request_gated_split(7, key, g.clone(), 5);
        assert_eq!(g.value(), Some(0), "requested: closed");
        let taken = inbox.take_split_requests();
        assert_eq!(taken, vec![(t, SplitTarget::Space(key))]);
        assert_eq!(g.value(), Some(0), "taken (walking): still closed");
        assert_eq!(inbox.split_result(t), None, "no outcome before the finish");
        assert_eq!(inbox.finish_split(t, Ok(())), Some(7));
        assert_eq!(g.value(), Some(5), "released at the finish");
        assert_eq!(inbox.split_result(t), Some(Ok(())));
        // A failed split never opens its gate.
        let t2 = inbox.request_gated_split(7, key, g.clone(), 6);
        let _ = inbox.take_split_requests();
        let _ = inbox.finish_split(t2, Err("walk refused".into()));
        assert_eq!(g.value(), Some(5));
        assert!(inbox.split_result(t2).unwrap().is_err());
        // A ring released before its gate opened: the outcome is that failure.
        let t3 = inbox.request_gated_split(7, key, g.clone(), 7);
        let _ = inbox.take_split_requests();
        g.revoke_for_test();
        let _ = inbox.finish_split(t3, Ok(()));
        assert!(
            inbox
                .split_result(t3)
                .unwrap()
                .unwrap_err()
                .contains("released")
        );
    }

    // ★★★ 2026-10-10 (`OWNER_RULINGS.md` §AB; `docs/design/V3_WINDOW_EXPOSURE_REVIEW.md`,
    // `crate::exposure`) — **the owner invariants over the real placement paths**: in a mirror
    // (where Passthrough channels run) every mapping has a guest-leaf origin; whole-RAM windows,
    // store windows and the ring region exist only in the privileged T-space.

    const STORE: u32 = 0x1;
    const RAM_OBJ: u32 = 0x20;
    const STORE_12G: u64 = 12 << 30;
    const RAM_8G: u64 = 8 << 30;

    /// ★ A fake host for EVERY host verb the twin life cycle ([`TwinHost`]) and the T-space build
    /// ([`crate::tspace::TSpaceHost`]) issue. It keeps a ledger of what each verb placed, per
    /// space, the origin derived from the verb and the memory it names — never from the caller.
    #[derive(Default)]
    struct LedgerHost {
        next_space: std::cell::Cell<u32>,
        /// `(space, placement)` for every mapping or reservation a verb placed.
        placed: std::cell::RefCell<Vec<(u32, crate::exposure::Placement)>>,
        /// Spaces a bare (T-space) allocation made.
        bare: std::cell::RefCell<Vec<u32>>,
        /// Twin window maps (`TwinHost::map_window`): `(space, memory, len)`.
        maps: std::cell::RefCell<Vec<(u32, u32, u64)>>,
        /// Where the next bottom-up T-space window lands.
        next_low: std::cell::Cell<u64>,
    }
    impl LedgerHost {
        fn new_space(&self) -> kf_host::VaSpace {
            let n = self.next_space.get() + 0x10;
            self.next_space.set(n);
            space(n)
        }
        fn origin(memory: u32) -> crate::exposure::Origin {
            match memory {
                STORE => crate::exposure::Origin::StoreWindow,
                RAM_OBJ => crate::exposure::Origin::RamWindow,
                _ => crate::exposure::Origin::Unnamed,
            }
        }
        fn record(&self, s: u32, lo: u64, len: u64, origin: crate::exposure::Origin) {
            self.placed.borrow_mut().push((
                s,
                crate::exposure::Placement {
                    lo,
                    hi: lo.saturating_add(len),
                    origin,
                },
            ));
        }
        fn in_space(&self, s: u32) -> Vec<crate::exposure::Placement> {
            self.placed
                .borrow()
                .iter()
                .filter(|(x, _)| *x == s)
                .map(|(_, p)| *p)
                .collect()
        }
    }
    impl TwinHost for LedgerHost {
        fn alloc_space(&self) -> Result<kf_host::VaSpace, String> {
            Ok(self.new_space())
        }
        fn map_window(&self, s: kf_host::VaSpace, memory: u32, len: u64) -> Result<u64, String> {
            // Where RM placed a `GROWS_DOWN` window.
            let base = 0x1_fffe_0000_0000;
            self.maps.borrow_mut().push((s.space, memory, len));
            self.record(s.space, base, len, Self::origin(memory));
            Ok(base)
        }
    }
    impl crate::tspace::TSpaceHost for LedgerHost {
        fn alloc_bare(&self) -> Result<kf_host::VaSpace, String> {
            let s = self.new_space();
            self.bare.borrow_mut().push(s.space);
            self.next_low.set(0x1_2000_0000);
            Ok(s)
        }
        fn reserve(&self, s: kf_host::VaSpace, at: u64, len: u64) -> Result<u32, String> {
            let origin = if at == RING_REGION_BASE {
                crate::exposure::Origin::RingRegion
            } else {
                crate::exposure::Origin::Unnamed
            };
            self.record(s.space, at, len, origin);
            Ok(0x99)
        }
        fn map_window(
            &self,
            s: kf_host::VaSpace,
            memory: u32,
            len: u64,
            _: bool,
            _: kf_host::MapPerm,
            _: bool,
        ) -> Result<u64, String> {
            let base = self.next_low.get();
            self.next_low
                .set((base + len + (16 << 20)).next_multiple_of(2 << 20));
            self.record(s.space, base, len, Self::origin(memory));
            Ok(base)
        }
        fn map_fixed_4k(
            &self,
            s: kf_host::VaSpace,
            memory: u32,
            _: u64,
            len: u64,
            at: u64,
            _: kf_host::MapPerm,
        ) -> Result<u64, String> {
            self.next_low
                .set((at + len + (16 << 20)).next_multiple_of(2 << 20));
            self.record(s.space, at, len, Self::origin(memory));
            Ok(at)
        }
        fn free_space(&self, _: kf_host::VaSpace) {}
    }
    fn space(n: u32) -> kf_host::VaSpace {
        kf_host::VaSpace {
            space: n,
            range: n + 1,
            guest: Default::default(),
        }
    }
    /// A guest process's VA space (a user client) and one of the guest RM's internal clients'.
    fn user_key() -> VasKey {
        VasKey((0xc1d0_002b_u64 << 32) | 5)
    }
    fn rm_internal_key() -> VasKey {
        VasKey((0xc1e0_0007_u64 << 32) | 1)
    }

    /// A guest leaf as the walker places it through `GpuMirror`'s `MapTarget::map`: a row.
    fn place_leaf(rec: &TwinRecord, va: u64, len: u64, off: u64, ram: bool) {
        let perm = kf_host::MapPerm::READ_WRITE;
        rec.mirror
            .rows
            .write()
            .expect("rows")
            .insert(va, (len, off, ram, perm));
    }

    /// Every mapping a twin record says its space holds: its rows (guest leaves) and kayfabe's
    /// placements (the control's window, its VMM ranges).
    fn record_placements(rec: &TwinRecord) -> Vec<crate::exposure::Placement> {
        let mut v: Vec<_> = rec
            .mirror
            .rows
            .read()
            .expect("rows")
            .iter()
            .map(|(&va, &(len, ..))| crate::exposure::Placement {
                lo: va,
                hi: va + len,
                origin: crate::exposure::Origin::GuestLeaf,
            })
            .collect();
        v.extend(crate::exposure::mirror_placements(
            &rec.mirror,
            &rec.reserved,
        ));
        v
    }

    /// ★ The audit: every space the fake host saw, judged by its kind (the T-space is the space
    /// the build reported; every other space is a mirror), plus every twin record. Returns the
    /// violations, named.
    fn audit(
        h: &LedgerHost,
        tspace: Option<u32>,
        records: &[&TwinRecord],
    ) -> Vec<(u32, crate::exposure::Placement)> {
        use crate::exposure::{SpaceKind, violations};
        let mut spaces: Vec<u32> = h.placed.borrow().iter().map(|(s, _)| *s).collect();
        spaces.extend(records.iter().map(|r| r.mirror.space.space));
        spaces.sort_unstable();
        spaces.dedup();
        let mut out = Vec::new();
        for s in spaces {
            let kind = if Some(s) == tspace {
                SpaceKind::PrivilegedTSpace
            } else {
                SpaceKind::Mirror
            };
            out.extend(violations(kind, &h.in_space(s)).into_iter().map(|p| (s, p)));
        }
        for r in records {
            out.extend(
                violations(SpaceKind::Mirror, &record_placements(r))
                    .into_iter()
                    .map(|p| (r.mirror.space.space, p)),
            );
        }
        out
    }

    /// The whole life of a VM's spaces, through the real path functions: the T-space build
    /// (`tspace::prewarm`'s), a prewarmed spare ([`prewarm`]'s), a created twin
    /// ([`create_mirror`]'s: a space, then [`tmode_twin`]), a recycled spare ([`apply_statement`]'s
    /// [`tmode_reuse`]), a retired twin ([`retire_mirror`]'s [`retire_spare`]) and its reuse. Guest
    /// leaves are placed in every twin, one of them over `[RING_REGION_BASE, 2^40)` (a guest leaf
    /// there is the guest's, now that no twin reserves the region).
    fn life(h: &LedgerHost, key: VasKey, negctl: bool) -> (Option<u32>, Vec<TwinRecord>) {
        let t = crate::tspace::TSpace::build(
            h,
            STORE,
            STORE_12G - 0x1042_0000,
            (RAM_OBJ, RAM_8G),
            false,
        )
        .expect("T-space built");
        let tspace = h.bare.borrow().first().copied();
        assert!(
            t.line()
                .starts_with(&format!("tspace space={:#x} ", tspace.unwrap_or(0)))
        );
        let spare = prewarm_spare(h, Some(RAM_OBJ), STORE, STORE_12G, negctl).expect("spare");
        let created = tmode_twin(
            h,
            key,
            TwinHost::alloc_space(h).expect("space"),
            Some(RAM_OBJ),
            STORE,
            STORE_12G,
            negctl,
        );
        place_leaf(&created, 0x7f00_0000_0000, 0x20_0000, 0x40_0000, false);
        place_leaf(&created, RING_REGION_BASE, 0x1000, 0x1000, true);
        let recycled = tmode_reuse(key, &spare);
        place_leaf(&recycled, 0x1_0000_0000, 0x1000, 0x2000, true);
        assert!(
            retire_spare(&created.mirror, 1).is_none(),
            "a refused unmap frees the space"
        );
        let retired = retire_spare(&created.mirror, 0).expect("kept as a spare");
        let reborn = tmode_reuse(key, &retired);
        (tspace, vec![created, recycled, reborn])
    }

    /// ★★ The invariants hold on every path, for a user key and a guest-kernel key: no mirror
    /// carries a window, a ring region or any kayfabe placement (the fake host saw no map in any
    /// mirror space, and no record names one); the whole-RAM window, the store window and the
    /// ring region exist, and only in the T-space; the T-space is never a mirror.
    #[test]
    fn owner_invariant_holds_on_every_tmode_mirror_path() {
        use crate::exposure::Origin;
        for key in [user_key(), rm_internal_key()] {
            let h = LedgerHost::default();
            let (tspace, recs) = life(&h, key, false);
            let refs: Vec<&TwinRecord> = recs.iter().collect();
            assert_eq!(audit(&h, tspace, &refs), vec![], "{key:?}");
            assert!(h.maps.borrow().is_empty(), "{:?}", h.maps.borrow());
            for rec in &recs {
                assert_ne!(
                    Some(rec.mirror.space.space),
                    tspace,
                    "a mirror is never the T-space"
                );
                assert!(rec.reserved.is_empty(), "no VMM range in a twin");
                assert_eq!(windows_text(&rec.mirror), "windows=none");
                assert!(
                    record_placements(rec)
                        .iter()
                        .all(|p| p.origin == Origin::GuestLeaf),
                    "only guest leaves"
                );
            }
            // The T-space holds exactly the windows and the ring region — and they are there.
            let ts = h.in_space(tspace.expect("T-space"));
            for o in [Origin::RingRegion, Origin::StoreWindow, Origin::RamWindow] {
                assert!(
                    ts.iter().any(|p| p.origin == o),
                    "{o:?} in the T-space: {ts:?}"
                );
            }
            // Its store window ends at the carve-out (never kayfabe's firmware region).
            let store_end = ts
                .iter()
                .filter(|p| p.origin == Origin::StoreWindow)
                .map(|p| p.hi)
                .max();
            let store_lo = ts
                .iter()
                .filter(|p| p.origin == Origin::StoreWindow)
                .map(|p| p.lo)
                .min();
            assert_eq!(
                store_end.zip(store_lo).map(|(e, l)| e - l),
                Some(STORE_12G - 0x1042_0000)
            );
        }
    }

    /// ⊘ Deliberate violations — the check must fail on each: (1) the positive control
    /// (`KF3_NEGCTL_TWIN_WINDOW`) maps a store window in each new twin and spare: named on the
    /// created, recycled and retired-then-reused paths, in the ledger AND in the records; (2) a
    /// whole-RAM window planted in a twin (what the deleted P5 path did); (3) a planted ring
    /// region in a clean twin's record; (4) the T-space judged as a mirror.
    #[test]
    fn owner_invariant_check_catches_a_planted_window() {
        use crate::exposure::{Origin, SpaceKind, violations};
        let key = user_key();
        // (1)
        let h = LedgerHost::default();
        let (tspace, recs) = life(&h, key, true);
        let refs: Vec<&TwinRecord> = recs.iter().collect();
        let v = audit(&h, tspace, &refs);
        assert_eq!(
            h.maps.borrow().len(),
            2,
            "one window per new twin and spare"
        );
        // Two ledger maps (created twin, prewarmed spare) + three records naming one each.
        assert_eq!(v.len(), 5, "{v:?}");
        assert!(v.iter().all(|(s, p)| p.origin == Origin::StoreWindow
            && p.hi - p.lo == STORE_12G
            && Some(*s) != tspace));
        for rec in &recs {
            assert!(windows_text(&rec.mirror).starts_with("windows fb=0x1fffe00000000+"));
        }
        // (2)
        let h = LedgerHost::default();
        let (tspace, recs) = life(&h, key, false);
        let twin = recs[0].mirror.space;
        TwinHost::map_window(&h, twin, RAM_OBJ, RAM_8G).expect("planted");
        let refs: Vec<&TwinRecord> = recs.iter().collect();
        let v = audit(&h, tspace, &refs);
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!((v[0].0, v[0].1.origin), (twin.space, Origin::RamWindow));
        // (3)
        let clean = tmode_twin(
            &h,
            key,
            space(0x400),
            Some(RAM_OBJ),
            STORE,
            STORE_12G,
            false,
        );
        let planted = TwinRecord {
            mirror: clean.mirror.clone(),
            reserved: vec![(RING_REGION_BASE, kf_chan::host::RING_VA_LIMIT)],
        };
        let v = audit(&LedgerHost::default(), None, &[&planted]);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].1.origin, Origin::RingRegion);
        // (4) The T-space's own placements are violations if it were a mirror.
        let ts = h.in_space(tspace.expect("T-space"));
        assert_eq!(violations(SpaceKind::Mirror, &ts).len(), ts.len());
        assert!(!ts.is_empty());
    }

    /// ★ P1+P2 inc D (§7 test 12), review fix 2026-10-04 — **no twin path maps a window**: a
    /// created twin, a prewarmed spare and a recycled spare each hold no window and no VMM range,
    /// and are named `windows=none` by a line DERIVED from the record. ⊘ The positive control
    /// (`KF3_NEGCTL_TWIN_WINDOW`) maps one window per new twin and spare, and the record and its
    /// line NAME it — what the box-log gate's `WINDOWS=NONE` must catch.
    #[test]
    fn no_tmode_twin_path_maps_a_window() {
        let key = user_key();
        let h = LedgerHost::default();
        let created = tmode_twin(&h, key, space(0x10), Some(RAM_OBJ), STORE, STORE_12G, false);
        let spare = tmode_spare(&h, space(0x12), Some(RAM_OBJ), STORE, STORE_12G, false);
        let recycled = tmode_reuse(key, &spare);
        assert!(h.maps.borrow().is_empty(), "{:?}", h.maps.borrow());
        assert_eq!(spare_windows_text(&spare), "windows=none");
        for rec in [&created, &recycled] {
            assert_eq!(rec.mirror.negctl_window, None);
            assert!(
                rec.reserved.is_empty(),
                "no VMM range a guest leaf must avoid"
            );
            assert_eq!(windows_text(&rec.mirror), "windows=none");
            assert!(
                !rec.mirror.kernel_vas.is_kernel(),
                "a user client's space starts unclassified"
            );
        }
        // ⊘ The positive control.
        let h = LedgerHost::default();
        let created = tmode_twin(&h, key, space(0x10), Some(RAM_OBJ), STORE, STORE_12G, true);
        let spare = tmode_spare(&h, space(0x12), Some(RAM_OBJ), STORE, STORE_12G, true);
        let recycled = tmode_reuse(key, &spare);
        assert_eq!(
            h.maps.borrow().len(),
            2,
            "one window per new twin and spare"
        );
        for rec in [&created, &recycled] {
            assert!(windows_text(&rec.mirror).starts_with("windows fb=0x1fffe00000000+"));
            assert_eq!(
                rec.reserved,
                vec![(0x1_fffe_0000_0000, 0x1_fffe_0000_0000 + STORE_12G)]
            );
        }
        assert!(spare_windows_text(&spare).starts_with("windows fb="));
    }

    #[test]
    fn the_bar2_key_is_no_guest_key() {
        assert_eq!(K_BAR2.0 >> 32, 0xFFFF_FFFF);
    }

    /// ★★★ The owner's question of 2026-10-03 (*"the memfd is only scratch — isn't that a DoS
    /// target?"*), as a test of the PRODUCTION constructor: a guest that reads and then writes every
    /// page of a fully unmapped BAR-sized window makes the host allocate at most one tile. Before
    /// 2026-10-03 the same reads allocated the whole window
    /// (`scratch.rs::the_old_whole_window_scratch_allocates_the_whole_window_on_reads_alone`).
    #[test]
    fn a_window_built_for_the_vm_bounds_its_scratch_to_one_tile() {
        let page = HostPageSize::query();
        let len = 64 << 20;
        let bound = kf_linux_raw::scratch_tile_len(len);
        assert_eq!(bound, 2 << 20);
        let (w, s) =
            window_with_scratch(len, page, "BAR1", c"kf3-scratch-test-bar1").expect("window");
        let step = page.bytes();
        let mut b = [0u8; 1];
        let mut sum = 0u64;
        let mut at = 0;
        while at < len {
            w.read_into(HostOffset::new(at), &mut b).expect("read");
            sum += u64::from(b[0]);
            at += step;
        }
        std::hint::black_box(sum);
        let after_reads = s.allocated_bytes().expect("fstat");
        assert!(after_reads > 0, "the reads reached the scratch");
        assert!(
            after_reads <= bound,
            "reads allocated {after_reads} > {bound}"
        );
        let mut at = 0;
        while at < len {
            w.write_from(HostOffset::new(at), &[0x5A]).expect("write");
            at += step;
        }
        let after_writes = s.allocated_bytes().expect("fstat");
        assert!(
            after_writes <= bound,
            "writes allocated {after_writes} > {bound}"
        );
    }

    /// Mappings of this process whose file is the memfd `name`.
    fn mappings_of(name: &std::ffi::CStr) -> usize {
        let want = format!("/memfd:{} (deleted)", name.to_str().expect("utf-8"));
        std::fs::read_to_string("/proc/self/maps")
            .expect("/proc/self/maps")
            .lines()
            .filter(|l| l.ends_with(&want))
            .count()
    }

    /// The `(Size in KiB, VmFlags)` of every mapping of memfd `name`, from `/proc/self/smaps`.
    fn vmflags_of(name: &std::ffi::CStr) -> Vec<(u64, String)> {
        let want = format!("/memfd:{} (deleted)", name.to_str().expect("utf-8"));
        let smaps = std::fs::read_to_string("/proc/self/smaps").expect("/proc/self/smaps");
        let (mut ours, mut size) = (false, 0);
        let mut out = Vec::new();
        for l in smaps.lines() {
            // A mapping's header line starts with `start-end`; every field line with `Key:`.
            let header = l
                .split_whitespace()
                .next()
                .is_some_and(|f| !f.ends_with(':'));
            if header {
                ours = l.ends_with(&want);
            } else if let Some(kb) = l.strip_prefix("Size:") {
                size = kb
                    .trim()
                    .trim_end_matches("kB")
                    .trim()
                    .parse()
                    .expect("Size");
            } else if let Some(flags) = l.strip_prefix("VmFlags:")
                && ours
            {
                out.push((size, flags.trim().to_string()));
            }
        }
        out
    }

    /// Whether VmFlags line `f` carries flag `x`.
    fn has_flag(f: &str, x: &str) -> bool {
        f.split_whitespace().any(|w| w == x)
    }

    /// ★★★ 2026-10-03 (review finding 1), through the PRODUCTION doors: `window_with_scratch`, then
    /// QEMU 10.2.4's `ram_block_add` advice (`system/physmem.c:2294-2304`), then guest-driven
    /// place-then-sink cycles at distinct offsets through `window_sink` as BAR1 and BAR2 call it.
    /// The scratch must be back to `ceil(W / T)` mappings after every sink.
    ///
    /// ★ Under BOTH `dump-guest-core` settings (third review): with `=off` QEMU's own advice adds
    /// `MADV_DONTDUMP` and would hide a plain initial cover; with `=on` (the default) only
    /// `window_with_scratch`'s own advice puts `VM_DONTDUMP` on the tiling, so this is the case
    /// that fails if that cover ever goes back to a plain one. Before the fix each cycle left two
    /// more mappings (`kf_linux_raw` `scratch.rs::an_unadvised_sink_beside_qemus_advice_never_merges_back`).
    #[test]
    fn a_bar_sink_merges_back_after_qemu_has_advised_the_window() {
        use kf_linux_raw::WindowAdvice;
        let page = HostPageSize::query();
        let pg = page.bytes();
        let len: u64 = 64 << 20;
        for dump_guest_core in [true, false] {
            let name = if dump_guest_core {
                c"kf3-scratch-test-advised-bar1-dump"
            } else {
                c"kf3-scratch-test-advised-bar1-nodump"
            };
            let (w, s) = window_with_scratch(len, page, "BAR1", name).expect("window");
            let canonical = usize::try_from(len / s.tile_len()).expect("small");
            for a in [
                WindowAdvice::DontDump,
                WindowAdvice::HugePage,
                WindowAdvice::DontFork,
            ] {
                if a == WindowAdvice::DontDump && dump_guest_core {
                    continue; // QEMU advises DONTDUMP only with dump-guest-core=off
                }
                // QEMU ignores a refusal (THP compiled out is EINVAL); so do we.
                let _ = w.advise(HostOffset::ZERO, len, a);
            }
            assert_eq!(mappings_of(name), canonical, "QEMU's advice split nothing");
            let stand_in = kf_linux_raw::SharedRam::create(4 * pg).expect("a stand-in view");
            let cycles = 256u64;
            let mut worst = 0;
            for k in 0..cycles {
                let at = (11 * k + 3) * pg;
                w.place(
                    HostOffset::new(at),
                    4 * pg,
                    Backing::SharedFile {
                        fd: stand_in.as_backing_fd(),
                        offset: 0,
                    },
                )
                .expect("place");
                window_sink(&w, &s, window_advises(false), at, 4 * pg).expect("sink");
                worst = worst.max(mappings_of(name));
            }
            assert_eq!(
                (worst, mappings_of(name)),
                (canonical, canonical),
                "dump-guest-core={dump_guest_core}: after {cycles} place-then-sink cycles the \
                 scratch does not merge back"
            );
            for (_, f) in vmflags_of(name) {
                assert!(
                    has_flag(&f, "dd") && has_flag(&f, "dc"),
                    "dump-guest-core={dump_guest_core}: a BAR scratch mapping without the \
                     window's advice: {f}"
                );
            }
        }
    }

    /// A refusing stand-in for [`kf_linux_raw::advise_window`] (`ENOMEM`, what a merge's
    /// allocation under memory-cgroup pressure returns).
    fn refused_advice(_: &GuestWindow, _: HostOffset, _: u64) -> Result<(), RawError> {
        Err(RawError::Syscall {
            call: "madvise",
            errno: Some(12),
        })
    }

    /// A BAR window's verbs over a real [`GuestWindow`] and its scratch, with no host RM: a "view"
    /// is a stand-in memfd placed at the view's offset, and every sink goes through the production
    /// sink with its advice refused.
    struct RefusedAdviceOps {
        w: GuestWindow,
        s: ScratchTile,
        view: kf_linux_raw::SharedRam,
        released: std::cell::RefCell<Vec<u64>>,
    }

    impl ViewOps for &RefusedAdviceOps {
        type View = u64;
        fn arm_store(&self, off: u64, _len: u64) -> Result<u64, String> {
            Ok(off)
        }
        fn place_view(&self, at: u64, len: u64, _v: &u64) -> Result<(), String> {
            self.w
                .place(
                    HostOffset::new(at),
                    len,
                    Backing::SharedFile {
                        fd: self.view.as_backing_fd(),
                        offset: 0,
                    },
                )
                .map_err(|e| format!("{e:?}"))
        }
        fn place_ram(&self, _: u64, _: u64, _: u64) -> Result<(), String> {
            Err("no guest RAM in this test".into())
        }
        fn sink(&self, at: u64, len: u64) -> Result<(), String> {
            sink_with(&self.w, &self.s, Some(refused_advice), at, len).map(|_| ())
        }
        fn release(&self, v: u64) -> Result<(), String> {
            self.released.borrow_mut().push(v);
            Ok(())
        }
    }

    /// ★★ 2026-10-03 (third review, finding 6): once a sink's cover landed, the guest can no longer
    /// reach the view behind it, and a refused advice costs only the merge. Returned as a refused
    /// sink, it made `CpuWindow::unmap` keep the view (and its host aperture) and left the guest's
    /// invalidate armed until a re-walk. Now the unmap lands, the view is released, and the
    /// refusal is counted.
    #[test]
    fn a_sink_whose_advice_is_refused_still_unmaps_and_is_counted() {
        let page = HostPageSize::query();
        let pg = page.bytes();
        let len = 64 << 20;
        let (w, s) = window_with_scratch(len, page, "BAR1", c"kf3-scratch-test-advice-refused")
            .expect("window");
        let view_name = c"kf3-test-advice-refused-view";
        let ops = RefusedAdviceOps {
            w,
            s,
            view: kf_linux_raw::SharedRam::create_named(view_name, 4 * pg).expect("a stand-in"),
            released: std::cell::RefCell::default(),
        };
        let win = CpuWindow::new(&ops, len);
        let d = Desired {
            va: 8 * pg,
            len: 4 * pg,
            off: 0x2_0000,
            ram: false,
            kind: 0,
            perm: kf_host::MapPerm::READ_WRITE,
            leaf: 0,
        };
        win.map(&d, false).expect("map");
        assert_eq!(mappings_of(view_name), 1, "the view is placed");
        let before = WINDOW_ADVICE_REFUSED.load(Ordering::Relaxed);
        win.unmap(8 * pg, false)
            .expect("the unmap lands: the cover already made the view unreachable");
        assert_eq!(
            mappings_of(view_name),
            0,
            "the window no longer shows the view"
        );
        assert_eq!(
            *ops.released.borrow(),
            vec![0x2_0000],
            "its aperture was released"
        );
        assert!(
            WINDOW_ADVICE_REFUSED.load(Ordering::Relaxed) > before,
            "the refused advice is counted"
        );
        let st = win.stats();
        assert_eq!((st.sunk, st.released, st.refused), (1, 1, 0));
    }

    /// ★ PRAMIN keeps its one-`mmap` sink (owner ruling 2026-09-25): the production PRAMIN window is
    /// its own tile, so every slot run of the trap's re-point is one placement.
    ///
    /// ★ And it makes NO `madvise` (third review): the PRAMIN `WindowOps` is built with
    /// `advise: window_advises(trap.is_some())`, so the decision is asserted here, and its effect
    /// read back from the kernel: the run the trap's sink re-placed carries no `VM_DONTCOPY` (`dc`),
    /// while the rest of the window keeps the initial cover's. Before, `advise=false` was passed
    /// by hand, and an advised PRAMIN (three `madvise`s per vCPU re-point) would have gone unseen.
    #[test]
    fn the_pramin_window_is_its_own_tile_and_a_run_is_one_mmap() {
        assert!(
            !window_advises(true),
            "a window with a trap (PRAMIN, on the vCPU) must not madvise"
        );
        assert!(window_advises(false), "BAR1 and BAR2 (VA thread) advise");
        let page = HostPageSize::query();
        let len = kf_trap::trappolicy::PRAMIN_LEN;
        let name = c"kf3-scratch-test-pramin";
        let (w, s) = window_with_scratch(len, page, "PRAMIN", name).expect("window");
        assert_eq!(s.tile_len(), len);
        // The door the PRAMIN trap goes through, with the production decision: one mmap per run.
        assert_eq!(
            window_sink(&w, &s, window_advises(true), GRANULE, 3 * GRANULE),
            Ok(1)
        );
        let unadvised: Vec<u64> = vmflags_of(name)
            .into_iter()
            .filter(|(_, f)| !has_flag(f, "dc"))
            .map(|(kb, _)| kb)
            .collect();
        assert_eq!(
            unadvised,
            vec![3 * GRANULE / 1024],
            "exactly the sunk run carries no advice (no madvise on the vCPU)"
        );
        assert_eq!(s.cover(&w, HostOffset::new(GRANULE), 3 * GRANULE), Ok(1));
        assert_eq!(s.cover(&w, HostOffset::ZERO, len), Ok(1));
    }
}

#[cfg(test)]
mod memory_list_tests {
    use super::*;
    use kf_rm::memory_list::{GuestRamAuthority, MAX_ACCESS, RamSpan};
    fn setup() -> (MemoryListRam, RamBlock) {
        let (mem, fd) = crate::raw_unsafe::test_owned_ram(0x8000);
        let block = RamBlock {
            gpa: 0x2000,
            mem,
            fd: Some(fd),
            fd_off: 0,
        };
        let ram = Box::leak(Box::new(RamMap::default()));
        ram.add(block);
        (MemoryListRam(ram), block)
    }
    #[test]
    fn memory_list_live_ram_copy_boundary_and_topology_revocation() {
        let (ram, block) = setup();
        let span = RamSpan {
            base: 0x3000,
            length: 0x7000,
        };
        let epoch = ram.validate(span).unwrap();
        assert!(ram.write(span, epoch, span.length - 2, &[9, 7]));
        let mut result = [0; 2];
        assert!(ram.read(span, epoch, span.length - 2, &mut result));
        assert_eq!(result, [9, 7]);
        assert!(!ram.write(span, epoch, span.length - 1, &[1, 2]));
        assert!(!ram.write(span, epoch, 0, &vec![0; MAX_ACCESS + 1]));
        assert!(!ram.read(span, 0, 0, &mut result));
        ram.0.del(block.gpa);
        assert!(!ram.read(span, epoch, 0, &mut result));
        ram.0.add(block);
        assert!(ram.validate(span).is_some());
        assert!(!ram.write(span, epoch, 0, &[1]));
        let current = ram.validate(span).unwrap();
        ram.0.del(0xffff0000); // even unrelated/no-op topology notifications revoke
        assert!(!ram.read(span, current, 0, &mut result));
    }
    #[test]
    fn memory_list_whole_span_no_holes_overlap_foreign_fd_or_overflow() {
        let (ram, block) = setup();
        for span in [
            RamSpan {
                base: 0x2000,
                length: 0,
            },
            RamSpan {
                base: 0x2000,
                length: 0x8001,
            },
            RamSpan {
                base: 0x1fff,
                length: 2,
            },
            RamSpan {
                base: u64::MAX,
                length: 2,
            },
        ] {
            assert!(ram.validate(span).is_none());
        }
        ram.0.add(RamBlock {
            gpa: 0x4000,
            ..block
        });
        assert!(
            ram.validate(RamSpan {
                base: 0x3000,
                length: 0x3000
            })
            .is_none()
        );
        ram.0.del(0x4000);
        ram.0.add(RamBlock {
            gpa: 0xa000,
            ..block
        });
        assert!(
            ram.validate(RamSpan {
                base: 0x9000,
                length: 0x2000
            })
            .is_none(),
            "two adjacent blocks still refused"
        );
        let (_, fd) = crate::raw_unsafe::test_owned_ram(1);
        ram.0.add(RamBlock {
            gpa: 0x20000,
            fd: Some(fd),
            ..block
        });
        assert!(
            ram.validate(RamSpan {
                base: 0x20000,
                length: 1
            })
            .is_none()
        );
        ram.0.add(RamBlock {
            gpa: 0x2000,
            fd: None,
            ..block
        });
        assert!(
            ram.validate(RamSpan {
                base: 0x2000,
                length: 1
            })
            .is_none()
        );
    }
    #[test]
    fn memory_list_epoch_exhaustion_is_terminal() {
        let (ram, block) = setup();
        let span = RamSpan {
            base: 0x2000,
            length: 4096,
        };
        ram.0.generation.store(u64::MAX, Ordering::Relaxed);
        assert_eq!(ram.validate(span), Some(u64::MAX));
        ram.0.add(block);
        assert_eq!(ram.validate(span), None);
        ram.0.del(block.gpa);
        ram.0.add(block);
        assert_eq!(ram.validate(span), None);
    }
}
