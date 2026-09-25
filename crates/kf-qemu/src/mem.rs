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
//! placement of the store, of guest RAM, or of per-BAR scratch (never a hole — a memslot over an
//! unmapped range kills the guest). No host flag is forwarded: every verb is authored here.
//!
//! ## BAR1
//!
//! ★ P5: walked from OUR BAR1 root exactly like BAR2 ([`K_BAR1`]): the guest writes its BAR1 PDEs
//! straight into that page (no RPC — `THE_OGKM_RESIDUE.md` §3) and invalidates it. `[measured
//! p5h]` left on scratch, it swallowed the PMA scrubber's `GP_PUT` (its USERD is mapped through
//! BAR1). ⚠ Each placed view costs host BAR1 aperture (`V3_P4_PORT_MAP.md` Q5 — the budget check
//! is still not built).

use crate::raw_unsafe::{RawRegion, borrow_process_fd};
use kf_host::{CpuViewRelease, HostRm, MapNode, ViewAccess};
use kf_linux_raw::{Backing, CharDevice, GuestWindow, HostOffset, HostPageSize, Notifier, SharedRam};
use kf_mem::cpuwin::{CpuWindow, PraminPool, SlotSource, ViewOps};
use kf_mem::ledger::{Desired, HostVas, MapTarget};
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
/// memory backend's `fd` at `fd_off` (`-1` when the backend has no fd).
#[derive(Debug, Clone, Copy)]
pub struct RamBlock {
    /// Guest-physical base.
    pub gpa: u64,
    /// The host mapping.
    pub mem: RawRegion,
    /// The backend fd, or -1.
    pub fd: i32,
    /// File offset of `gpa`.
    pub fd_off: u64,
}

/// ★ Guest RAM, as QEMU registered it — the VMM's own guest-physical layout, which is NOT the
/// identity once there is a PCI hole.
#[derive(Debug, Default)]
pub struct RamMap {
    blocks: RwLock<Vec<RamBlock>>,
}

impl RamMap {
    /// Register (or replace) the block at `b.gpa`.
    pub fn add(&self, b: RamBlock) {
        if let Ok(mut v) = self.blocks.write() {
            v.retain(|x| x.gpa != b.gpa);
            v.push(b);
            v.sort_by_key(|x| x.gpa);
        }
    }

    /// Unregister the block at `gpa`.
    pub fn del(&self, gpa: u64) {
        if let Ok(mut v) = self.blocks.write() {
            v.retain(|x| x.gpa != gpa);
        }
    }

    /// The ONE backend fd every fd-backed block shares, if any.
    #[must_use]
    pub fn backing_fd(&self) -> Option<i32> {
        self.blocks.read().ok()?.iter().find(|b| b.fd >= 0).map(|b| b.fd)
    }

    /// The block wholly covering `[gpa, gpa+len)`.
    #[must_use]
    pub fn block_for(&self, gpa: u64, len: u64) -> Option<RamBlock> {
        let v = self.blocks.read().ok()?;
        v.iter()
            .find(|b| gpa >= b.gpa && gpa.checked_add(len).is_some_and(|e| e - b.gpa <= b.mem.len() as u64))
            .copied()
    }

    /// ★ P5: the host mapping of guest-RAM memfd bytes `[off, off+len)` — the inverse of
    /// [`Self::file_range`], for reading a sysmem row the reconcile placed (whose offset is a FILE
    /// offset). `None` when no single fd-backed block covers it.
    #[must_use]
    pub fn at_file_offset(&self, off: u64, len: u64) -> Option<(RawRegion, usize)> {
        let v = self.blocks.read().ok()?;
        let first = v.iter().find(|x| x.fd >= 0)?.fd;
        v.iter()
            .filter(|b| b.fd == first)
            .find(|b| off >= b.fd_off && off.checked_add(len).is_some_and(|e| e - b.fd_off <= b.mem.len() as u64))
            .map(|b| (b.mem, (off - b.fd_off) as usize))
    }

    /// ★ The guest-RAM object's fd and the file offset of `[gpa, gpa+len)`. `None` when no single
    /// fd-backed block covers it. ⊘ Every block must share ONE fd (q35's `memory-backend=ram0`
    /// aliases one memfd above and below the hole); a second fd is refused, never guessed at.
    #[must_use]
    pub fn file_range(&self, gpa: u64, len: u64) -> Option<(i32, u64)> {
        let b = self.block_for(gpa, len)?;
        let first = self.blocks.read().ok()?.iter().find(|x| x.fd >= 0)?.fd;
        (b.fd >= 0 && b.fd == first).then(|| (b.fd, b.fd_off + (gpa - b.gpa)))
    }
}

/// ★ An armed CPU view of a store slice: the node whose `mmap` context RM registered, and the
/// `pLinearAddress` cookie its release is keyed by.
#[derive(Debug)]
pub struct StoreView {
    node: CharDevice,
    cookie: u64,
}

/// ★ The host verbs of ONE guest window — [`ViewOps`] over the real session.
pub struct WindowOps {
    rm: &'static HostRm,
    store: u32,
    win: &'static GuestWindow,
    scratch: &'static SharedRam,
    ram: &'static RamMap,
    /// PRAMIN only: nodes opened ahead of time and the reaper that releases retired views.
    trap: Option<&'static TrapNodes>,
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
            node_tx.try_send(open().map_err(|e| format!("PRAMIN node: {e:?}"))?).map_err(|e| format!("{e}"))?;
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
                    let r = rm.release_cpu_view(CpuViewRelease { h_memory: store, p_linear_address: v.cookie });
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
        Ok(StoreView { node, cookie })
    }

    fn place_view(&self, at: u64, len: u64, v: &StoreView) -> Result<(), String> {
        self.win
            .place_device_view(HostOffset::new(at), len, v.node.as_fd(), true)
            .map_err(|e| format!("mmap view @{at:#x}+{len:#x}: {e:?}"))
    }

    fn place_ram(&self, at: u64, len: u64, file_off: u64) -> Result<(), String> {
        let fd = self
            .ram
            .blocks
            .read()
            .ok()
            .and_then(|v| v.iter().find(|b| b.fd >= 0).map(|b| b.fd))
            .ok_or("no fd-backed guest RAM (the VM needs memory-backend-memfd,share=on)")?;
        self.win
            .place(HostOffset::new(at), len, Backing::SharedFile { fd: borrow_process_fd(fd), offset: file_off })
            .map_err(|e| format!("mmap guest RAM @{at:#x}+{len:#x} (file {file_off:#x}): {e:?}"))
    }

    fn sink(&self, at: u64, len: u64) -> Result<(), String> {
        self.win
            .place(HostOffset::new(at), len, Backing::SharedFile { fd: self.scratch.as_backing_fd(), offset: at })
            .map_err(|e| format!("mmap scratch @{at:#x}+{len:#x}: {e:?}"))
    }

    fn release(&self, v: StoreView) -> Result<(), String> {
        let r = self.rm.release_cpu_view(CpuViewRelease { h_memory: self.store, p_linear_address: v.cookie });
        drop(v.node);
        r.map_err(|e| format!("NV_ESC_RM_UNMAP_MEMORY: {e:?}"))
    }

    /// ★ The trap's ONE RM call: `NV_ESC_RM_MAP_MEMORY` on a node opened ahead of time.
    fn arm_store_in_trap(&self, off: u64, len: u64) -> Result<StoreView, String> {
        let Some(t) = self.trap else { return self.arm_store(off, len) };
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
        Ok(StoreView { node, cookie })
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
    /// A host GPU VA space mirroring a guest one (the Translated plane's target).
    Gpu(GpuMirror),
}

/// ★ P5: OUR placements in one mirrored host VA space, `va → (len, offset, ram)` — the rows the
/// reconcile made, recorded AS it makes them, so a Translated channel can find the bytes behind a
/// guest VA (its GPFIFO, a pushbuffer segment) through what WE mapped (`THE_TRANSLATED_PLANE.md`
/// §24.2). ⊘ Never a copy of the guest's tables: every row is one of our own map calls.
///
/// ★ Written INSIDE the apply, before the invalidate's `TRIGGER` is cleared — so a doorbell the
/// guest rings after its invalidate completed always finds the rows that invalidate published.
pub type PlacedRows = std::sync::Arc<RwLock<std::collections::BTreeMap<u64, (u64, u64, bool)>>>;

/// Resolve `[va, va+len)` against `rows`: `(ram, offset)` when ONE placement covers it whole.
#[must_use]
pub fn resolve_placed(rows: &PlacedRows, va: u64, len: u64) -> Option<(bool, u64)> {
    let r = rows.read().ok()?;
    let (&start, &(rlen, off, ram)) = r.range(..=va).next_back()?;
    let end = va.checked_add(len)?;
    (end <= start.checked_add(rlen)?).then(|| (ram, off + (va - start)))
}

/// ★ P5b: the placement covering `va` — `(ram, offset of va, bytes of the row left from va)`.
/// A read that crosses from one of OUR rows into the next (two 4 KiB placements adjacent in VA)
/// walks them piece by piece. `[measured p5bs --concurrency]` a 0x60-byte scrubber segment at
/// `…6fb8` crossed a page into the next row and killed the channel as "not placed by us".
#[must_use]
pub fn resolve_placed_prefix(rows: &PlacedRows, va: u64) -> Option<(bool, u64, u64)> {
    let r = rows.read().ok()?;
    let (&start, &(rlen, off, ram)) = r.range(..=va).next_back()?;
    let end = start.checked_add(rlen)?;
    (va < end).then(|| (ram, off + (va - start), end - va))
}

/// ★ A mirrored host VA space: the reconcile's target, and the row record beside it.
pub struct GpuMirror {
    /// The host VA space and its objects.
    pub vas: HostVas<'static>,
    /// Our placements (see [`PlacedRows`]).
    pub rows: PlacedRows,
}

impl MapTarget for GpuMirror {
    fn map(&self, d: &Desired, defer: bool) -> Result<(), String> {
        self.vas.map(d, defer)?;
        if let Ok(mut r) = self.rows.write() {
            r.insert(d.va, (d.len, d.off, d.ram));
        }
        Ok(())
    }
    fn unmap(&self, va: u64, defer: bool) -> Result<(), String> {
        // ⊘ Forget the row FIRST: a reader must never resolve through a mapping being torn down.
        if let Ok(mut r) = self.rows.write() {
            r.remove(&va);
        }
        self.vas.unmap(va, defer)
    }
    fn invalidate(&self) -> Result<(), String> {
        self.vas.invalidate()
    }
    fn va_extent(&self) -> Option<u64> {
        self.vas.va_extent()
    }
}

/// ★ P5: one mirrored guest VA space as the channel plane sees it — the host space, its two
/// windows (`THE_TRANSLATED_PLANE.md` §2, §6, §12: in EVERY host VAS we create), and our rows.
#[derive(Clone)]
pub struct Mirror {
    /// The host VA space.
    pub space: kf_host::VaSpace,
    /// Guest FB-physical `p` is host VA `fb_base + p` (`p < fb_len`).
    pub fb_base: u64,
    /// The identity window's length (the store's).
    pub fb_len: u64,
    /// Guest-RAM memfd offset `o` is host VA `ram_base + o` (`o < ram_len`); `None` when the space
    /// has no guest-RAM object.
    pub ram: Option<(u64, u64)>,
    /// Our placements.
    pub rows: PlacedRows,
    /// ★ P5b: the guest-RAM host object mapped at `ram` (a sysmem USERD's twin names it).
    pub ram_obj: Option<u32>,
}

/// ★ P5: the mirrors, by VA-space object — written by the VA thread when it creates one, read by
/// the channel plane when a channel names it.
pub type Mirrors = std::sync::Arc<Mutex<std::collections::HashMap<VasKey, Mirror>>>;

impl MapTarget for Target {
    fn map(&self, d: &Desired, defer: bool) -> Result<(), String> {
        match self {
            Target::Window(w) => w.map(d, defer),
            Target::Gpu(g) => g.map(d, defer),
        }
    }
    fn unmap(&self, va: u64, defer: bool) -> Result<(), String> {
        match self {
            Target::Window(w) => w.unmap(va, defer),
            Target::Gpu(g) => g.unmap(va, defer),
        }
    }
    fn invalidate(&self) -> Result<(), String> {
        match self {
            Target::Window(w) => w.invalidate(),
            Target::Gpu(g) => g.invalidate(),
        }
    }
    fn va_extent(&self) -> Option<u64> {
        match self {
            Target::Window(w) => w.va_extent(),
            Target::Gpu(g) => g.va_extent(),
        }
    }
}

/// The manager the VA thread owns.
pub type Manager = VaManager<GpuWalker, Target>;

/// ★ The drainer → VA-manager inbox for the guest's address-space statements, with the two
/// counters that decide when their held replies may go: `received` (the drainer, as it enqueues)
/// and `settled` (the VA thread, once everything received so far is applied and no walk is in
/// flight or pending). A held reply is delivered when they are equal.
#[derive(Debug)]
pub struct Inbox {
    q: Mutex<Vec<MemStatement>>,
    received: AtomicU64,
    settled: AtomicU64,
    /// The VA thread's wake.
    pub wake: Notifier,
}

impl Inbox {
    /// An empty inbox.
    ///
    /// # Errors
    /// No eventfd.
    pub fn new() -> Result<Inbox, String> {
        Ok(Inbox {
            q: Mutex::new(Vec::new()),
            received: AtomicU64::new(0),
            settled: AtomicU64::new(0),
            wake: Notifier::create().map_err(|e| format!("eventfd: {e:?}"))?,
        })
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
        self.q.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
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
        (self.received.load(Ordering::Relaxed), self.settled.load(Ordering::Relaxed))
    }
}

/// Counters for the boot log — never a decision input.
#[derive(Debug, Default)]
pub struct MemCounters {
    /// Trigger writes that armed.
    pub invalidates: AtomicU64,
    /// PRAMIN re-points whose window showed any scratch slot.
    pub pramin_miss_writes: AtomicU64,
    /// The last window base that missed (for the log).
    pub pramin_last_miss: AtomicU64,
    /// fn 70 entries written into our roots.
    pub bar_pdes: AtomicU64,
    /// Page-directory statements applied as roots.
    pub roots: AtomicU64,
    /// Statements refused by name.
    pub refused: AtomicU64,
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
    /// The store's device pointer in the walker's context (for fn 70's write).
    pub store_ptr: u64,
    /// Counters.
    pub counters: MemCounters,
    /// Guest RAM as QEMU registered it.
    ram: &'static RamMap,
    /// ★ The guest-RAM host object — an OS descriptor over the WHOLE guest memfd (gate 3's
    /// recipe), created once, on the VA thread, the first time a space needs it: `(handle, len)`.
    ram_obj: std::sync::OnceLock<Result<(u32, u64), String>>,
    /// ★ P5: every mirrored space, for the channel plane.
    pub mirrors: Mirrors,
    /// The store's length (the identity window's).
    pub fb_len: u64,
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
        store_ptr: u64,
        layout: &kf_chip::bar0::FbLayout,
        bar1_bytes: u64,
        bar2_bytes: u64,
        ram: &'static RamMap,
        inbox: std::sync::Arc<Inbox>,
        mirrors: Mirrors,
    ) -> Result<(MemPlane, WindowOps, WindowOps), String> {
        let page = HostPageSize::query();
        let window = |len: u64, what: &str| -> Result<(&'static GuestWindow, &'static SharedRam), String> {
            let w = GuestWindow::create(len, page).map_err(|e| format!("{what} window of {len:#x}: {e:?}"))?;
            let s = SharedRam::create(len).map_err(|e| format!("{what} scratch memfd of {len:#x}: {e:?}"))?;
            // ⊘ Scratch over the WHOLE window before anyone can see it: never a hole.
            w.place(HostOffset::new(0), len, Backing::SharedFile { fd: s.as_backing_fd(), offset: 0 })
                .map_err(|e| format!("{what} scratch placement: {e:?}"))?;
            Ok((Box::leak(Box::new(w)), Box::leak(Box::new(s))))
        };
        let pramin_len = kf_trap::trappolicy::PRAMIN_LEN;
        let (pramin_win, pramin_scratch) = window(pramin_len, "PRAMIN")?;
        let (bar1_win, bar1_scratch) = window(bar1_bytes, "BAR1")?;
        let (bar2_win, bar2_scratch) = window(bar2_bytes, "BAR2")?;
        let ops = |win, scratch, trap| WindowOps { rm, store, win, scratch, ram, trap };
        let trap = TrapNodes::start(rm, store)?;
        let pramin = PraminPool::new(
            ops(pramin_win, pramin_scratch, Some(trap)),
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
                store_ptr,
                counters: MemCounters::default(),
                ram,
                ram_obj: std::sync::OnceLock::new(),
                mirrors,
                fb_len: layout.fb_length,
            },
            ops(bar1_win, bar1_scratch, None),
            ops(bar2_win, bar2_scratch, None),
        ))
    }

    /// ★ The guest-RAM object: an OS descriptor over the whole guest memfd, so a sysmem leaf at
    /// memfd offset `o` is object offset `o` — the same numbering `RamMap::file_range` gives the
    /// VA manager. Created ONCE, on the VA thread (never a vCPU: it pins every page).
    ///
    /// ⚠ It pins all of guest RAM on the host for the VM's life — the same posture as gate 3's
    /// Translated channel, and the reason `memory-backend-memfd,share=on` is required.
    ///
    /// # Errors
    /// No fd-backed guest RAM, the mapping, or the host's refusal — by name, and remembered.
    pub fn guest_ram_object(&self, rm: &'static HostRm) -> Result<(u32, u64), String> {
        self.ram_obj
            .get_or_init(|| {
                let fd = self.ram.backing_fd().ok_or("no fd-backed guest RAM block (memory-backend-memfd,share=on?)")?;
                let borrowed = crate::raw_unsafe::borrow_process_fd(fd);
                let len = borrowed
                    .try_clone_to_owned()
                    .map(std::fs::File::from)
                    .and_then(|f| f.metadata())
                    .map_err(|e| format!("guest memfd size: {e}"))?
                    .len();
                let view = kf_linux_raw::MappedRegion::map(
                    Backing::SharedFile { fd: borrowed, offset: 0 },
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
            })
            .clone()
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
        self.pramin_want.store(u64::from(raw) | (1 << 32), Ordering::Release);
        loop {
            let want = self.pramin_want.load(Ordering::Acquire);
            let r = self.pramin.repoint(&self.pramin_plan(want as u32));
            if r.missed > 0 || r.refused > 0 {
                self.counters.pramin_miss_writes.fetch_add(1, Ordering::Relaxed);
                self.counters.pramin_last_miss.store(want & 0xFFFF_FFFF, Ordering::Relaxed);
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
            let w = m.walker().kernel.write_at(plane.store_ptr + root, &p.entry.to_le_bytes());
            if let Err(e) = w {
                plane.counters.refused.fetch_add(1, Ordering::Relaxed);
                return format!("fn70 {:?} entry={:#x}: GPU write into our root @{root:#x} REFUSED: {e}", p.bar, p.entry);
            }
            plane.counters.bar_pdes.fetch_add(1, Ordering::Relaxed);
            m.schedule_walk(if p.bar == BarAperture::Bar2 { K_BAR2 } else { K_BAR1 }, trigger);
            format!("fn70 {:?} entry={:#x} shift={} -> our root @{root:#x}, walk scheduled", p.bar, p.entry, p.level_shift)
        }
        MemStatement::PageDir(s) => {
            let key = VasKey((u64::from(s.client.0) << 32) | u64::from(s.vaspace.0));
            let ap = match s.pdb_aperture {
                Some(kf_arch::Aperture::Vidmem) => PdbAperture::Vidmem,
                Some(kf_arch::Aperture::SysmemCoherent | kf_arch::Aperture::SysmemNonCoherent) => PdbAperture::Sysmem,
                other => {
                    plane.counters.refused.fetch_add(1, Ordering::Relaxed);
                    return format!("pagedir {key:?} pdb={:#x}: aperture {other:?} refused", s.pdb.0);
                }
            };
            if m.table.target(key).is_none() {
                match rm.alloc_vaspace() {
                    Ok(space) => {
                        // ⊘ A space without the RAM object refuses every sysmem leaf and leaves the
                        // trigger armed — measured p4b4: a ~22 s guest stall per boot.
                        let ram_obj = match plane.guest_ram_object(rm) {
                            Ok(o) => Some(o),
                            Err(e) => {
                                eprintln!("kf3: {key:?}: {e} — its sysmem leaves will be refused");
                                None
                            }
                        };
                        // ★ P5: the two windows, in EVERY mirrored space (§12) — GROWS_DOWN, away
                        // from the guest's bottom-up VAs (§24.2). A space whose windows refuse is
                        // still a mirror (its virtual rows work); a Translated channel naming it
                        // refuses by name at birth.
                        let fb_base = rm.map_window(space, store, plane.fb_len, true);
                        let ram_base = ram_obj.map(|(o, len)| rm.map_window(space, o, len, true).map(|b| (b, len)));
                        let rows = PlacedRows::default();
                        let line = match (&fb_base, &ram_base) {
                            (Ok(fb), Some(Ok((rb, rl)))) => {
                                if let Ok(mut mm) = plane.mirrors.lock() {
                                    mm.insert(key, Mirror { space, fb_base: *fb, fb_len: plane.fb_len, ram: Some((*rb, *rl)), rows: rows.clone(), ram_obj: ram_obj.map(|(o, _)| o) });
                                }
                                format!("windows fb={fb:#x}+{:#x} ram={rb:#x}+{rl:#x}", plane.fb_len)
                            }
                            (Ok(fb), None) => {
                                if let Ok(mut mm) = plane.mirrors.lock() {
                                    mm.insert(key, Mirror { space, fb_base: *fb, fb_len: plane.fb_len, ram: None, rows: rows.clone(), ram_obj: None });
                                }
                                format!("windows fb={fb:#x} ram=NONE")
                            }
                            (fb, ram) => format!("windows REFUSED fb={fb:?} ram={ram:?}"),
                        };
                        eprintln!("kf3: {key:?} mirror space={:#x}: {line}", space.space);
                        m.table.insert(key, Target::Gpu(GpuMirror { vas: HostVas { rm, space, store, ram_obj: ram_obj.map(|(o, _)| o) }, rows }))
                    }
                    Err(e) => {
                        plane.counters.refused.fetch_add(1, Ordering::Relaxed);
                        return format!("pagedir {key:?}: host VA space refused: {e:?}");
                    }
                }
            }
            match m.table.set_root(key, s.pdb.0, ap) {
                Ok(changed) => {
                    plane.counters.roots.fetch_add(1, Ordering::Relaxed);
                    if changed {
                        m.schedule_walk(key, trigger);
                    }
                    format!("pagedir {key:?} root={:#x} changed={changed}", s.pdb.0)
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

    /// The measured bases, at 12 GiB, are inside what is armed, and a 1 MiB window from each is
    /// wholly covered.
    #[test]
    fn the_bar2_key_is_no_guest_key() {
        assert_eq!(K_BAR2.0 >> 32, 0xFFFF_FFFF);
    }
}
