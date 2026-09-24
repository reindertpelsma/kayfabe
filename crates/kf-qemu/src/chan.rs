//! ★★★★★ **P5 in the device — the guest KERNEL's copy-engine channels, Translated onto the real
//! engine** (`V3_P5_PORT_MAP.md` §2).
//!
//! | who | does |
//! |---|---|
//! | the **register drainer** (serving the guest's RPCs) | [`ChanPlane::statement`]: births a host twin AT the channel's alloc (a [`HostRing`] in the mirror of the channel's VA space), routes the guest's token to it, records the guest's `GPFIFO_SCHEDULE` |
//! | a **vCPU** | rings the guest token: `Plane::trap_write` stamps RUNG, publishes, wakes a worker — nothing else |
//! | a **worker** (`kf_chan::worker::run`) | `worker_pass` → [`ChanPlane::serve`] → `TranslatedChannel::pump`: read the guest's GP entries/segments through OUR placements, rewrite PHYSICAL operands onto the windows, submit on our ring, author `GP_GET` on completion |
//! | the **host** | runs the work; the ENGINE writes the guest's semaphores through the mirror; our ring's NSI reaches the session's completion fd, which rings the in-flight tokens again |
//!
//! ⊘ **No CPU executor, no forged completion.** The CPU touches exactly two guest words per pump
//! (`GP_PUT` read, `GP_GET` write — `kf_core::channel::CPU_MOVE_MAX_BYTES`), and reads the guest's
//! GP entries and pushbuffer segments (method words, never data) from guest RAM. A pushbuffer in
//! vidmem is REFUSED by name (a GPU read of it is not wired — `V3_P5_PORT_MAP.md` §4 Q3).
//! ⊘ **Nothing here runs on a vCPU.** Births are the drainer's; pumps are the workers'.

use crate::mem::{Mirror, Mirrors, RamMap, resolve_placed};
use crate::raw_unsafe::RawRegion;
use kf_chan::completions::Completions;
use kf_chan::host::{ChanError, GuestUserd, HostRing, Publisher, TranslatedChannel};
use kf_chan::ring::{GuestMemory, TranslatedRing};
use kf_chan::translated::{Target, Window};
use kf_core::{Owner, Plane, VmCaps};
use kf_host::{HostRm, MapNode, ViewAccess};
use kf_linux_raw::{Backing, CachePolicy, HostOffset, HostPageSize, VolatileRegion};
use kf_mem::vasmgr::VasKey;
use kf_rm::chanlink::{ChanAnswer, ChanStatement, ChannelAlloc};
use kf_trap::Route;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

/// `NV_ERR_INVALID_STATE`.
const NV_ERR_INVALID_STATE: u32 = 0x40;
/// `NV_ERR_INSUFFICIENT_RESOURCES`.
const NV_ERR_INSUFFICIENT_RESOURCES: u32 = 0x1A;
/// `NV_ERR_NOT_SUPPORTED`.
const NV_ERR_NOT_SUPPORTED: u32 = 0x56;

/// A CE class id on ANY family — the class tables are generated per family and class ids are
/// unique across them, so this needs no family argument (the rewriter takes a plain `fn`).
fn is_any_ce_class(c: u32) -> bool {
    kf_chip::Family::ALL.iter().any(|f| kf_chip::classes_for(*f).dma_copy.contains(&c))
}

/// `NV2080_ENGINE_TYPE_COPY0..9` = `9..=18`, `COPY10..19` = `0x28..` — a copy engine
/// (`ogkm-580: cl2080_notification.h`).
fn is_copy_engine(engine_type: u32) -> bool {
    (9..=18).contains(&engine_type) || (0x28..0x28 + 10).contains(&engine_type)
}

/// ★ The guest's USERD: two 4-byte cursors, reached through a CPU view WE armed at birth (vidmem)
/// or through guest RAM (sysmem). ⊘ Four bytes each way — `CPU_MOVE_MAX_BYTES`, never data.
enum UserdView {
    /// A 4 KiB CPU view of the store page holding USERD (`node` keeps the view's context alive).
    Store { region: VolatileRegion, _node: kf_linux_raw::CharDevice, cookie: u64, at: u64 },
    /// Guest RAM.
    Ram { mem: RawRegion, at: usize },
}

impl UserdView {
    fn load(&self, off: u64) -> Result<u32, String> {
        match self {
            UserdView::Store { region, at, .. } => region.load_u32(HostOffset::new(at + off)).map_err(|e| format!("{e:?}")),
            UserdView::Ram { mem, at } => mem.load_u32(at + off as usize).ok_or_else(|| "guest-RAM USERD load".to_string()),
        }
    }
    fn store(&self, off: u64, v: u32) -> Result<(), String> {
        match self {
            UserdView::Store { region, at, .. } => region.store_u32(HostOffset::new(at + off), v).map_err(|e| format!("{e:?}")),
            UserdView::Ram { mem, at } => mem.store_u32(at + off as usize, v).then_some(()).ok_or_else(|| "guest-RAM USERD store".to_string()),
        }
    }
}

struct Userd<'a>(&'a UserdView);
impl GuestUserd for Userd<'_> {
    fn gp_put(&mut self) -> Result<u32, String> {
        self.0.load(kf_abi::submit::USERD_GP_PUT)
    }
    fn set_gp_get(&mut self, gp_get: u32) -> Result<(), String> {
        self.0.store(kf_abi::submit::USERD_GP_GET, gp_get)
    }
}

/// The guest's memory at a guest VA of the channel's space, through OUR placements.
struct Mem<'a> {
    mirror: &'a Mirror,
    ram: &'a RamMap,
}
impl GuestMemory for Mem<'_> {
    fn read(&mut self, va: u64, out: &mut [u8]) -> Result<(), String> {
        let len = out.len() as u64;
        let (ram, off) = resolve_placed(&self.mirror.rows, va, len).ok_or_else(|| format!("{va:#x}+{len:#x} not placed by us"))?;
        if !ram {
            // ⊘ Q3: a vidmem pushbuffer would need a GPU read (the walker's context lives on the VA
            // thread). RM's CeUtils puts GPFIFO + pushbuffer in SYSMEM by default
            // (`ogkm-580: mem_utils_gm107.c:779-791`), so this is refused by name, not guessed at.
            return Err(format!("{va:#x}: pushbuffer/GPFIFO in VIDMEM (store {off:#x}) — GPU read not wired"));
        }
        let (mem, at) = self.ram.at_file_offset(off, len).ok_or_else(|| format!("{va:#x}: guest-RAM offset {off:#x} unregistered"))?;
        mem.read_into(at, out).then_some(()).ok_or_else(|| format!("{va:#x}: guest-RAM read"))
    }
}

/// ⊘ The `MEM_OP` split is UVM's (P6, `THE_TRANSLATED_PLANE.md` §24.2): RM's CeUtils never
/// invalidates in its pushbuffer. Refused by name until the split goes through the VA thread.
struct NoSplit;
impl Publisher for NoSplit {
    fn invalidated(&mut self, pdb: Option<u64>) -> Result<(), String> {
        Err(format!("MEM_OP invalidate (pdb {pdb:x?}) inside a kernel CE pushbuffer: the split is not wired (P6)"))
    }
}

/// The two windows of a mirror, for the rewriter.
struct Windows<'a>(&'a Mirror);
impl Window for Windows<'_> {
    fn translate(&self, t: Target, phys: u64, len: u64) -> Option<u64> {
        let end = phys.checked_add(len)?;
        match t {
            Target::LocalFb if end <= self.0.fb_len => Some(self.0.fb_base + phys),
            // ⊘ A sysmem operand is a guest-PHYSICAL address; the RAM window is by memfd FILE
            // offset. Identity only when guest RAM is one flat memfd from GPA 0 — resolved per
            // block by the caller's layout, never assumed (see `Slot::ram_window`).
            Target::CoherentSysmem | Target::NonCoherentSysmem => None,
            Target::Peer => None,
            _ => None,
        }
    }
}

/// The window with guest RAM resolved through the VMM's own layout.
struct SlotWindow<'a> {
    mirror: &'a Mirror,
    ram: &'a RamMap,
}
impl Window for SlotWindow<'_> {
    fn translate(&self, t: Target, phys: u64, len: u64) -> Option<u64> {
        match t {
            Target::CoherentSysmem | Target::NonCoherentSysmem => {
                let (base, rlen) = self.mirror.ram?;
                let (_, off) = self.ram.file_range(phys, len)?;
                (off.checked_add(len)? <= rlen).then(|| base + off)
            }
            other => Windows(self.mirror).translate(other, phys, len),
        }
    }
}

/// One guest kernel channel, Translated.
struct Slot {
    chan: TranslatedChannel,
    key: VasKey,
    mirror: Mirror,
    userd: UserdView,
    guest_idx: u32,
    scheduled: bool,
    dead: Option<String>,
}

/// Per-token counters for the gate (`forwarded=` per token), never a decision input.
#[derive(Debug, Default, Clone)]
pub struct TokenCount {
    /// The guest token (table index).
    pub token: u32,
    /// GP entries fetched and forwarded.
    pub forwarded: u64,
    /// Host submissions.
    pub submissions: u64,
    /// The last `GP_GET` authored.
    pub gp_get: Option<u32>,
    /// Why it died, if it did.
    pub dead: Option<String>,
}

/// ★★★ The channel plane.
pub struct ChanPlane {
    rm: &'static HostRm,
    plane: &'static Plane<'static>,
    store: u32,
    ram: &'static RamMap,
    mirrors: Mirrors,
    /// The session's ONE completion fd + in-flight set (`kf_chan::completions`).
    pub completions: Completions,
    caps: Mutex<VmCaps>,
    /// Host token → channel.
    slots: RwLock<HashMap<u32, Arc<Mutex<Slot>>>>,
    /// `(hClient, hObject)` → host token.
    by_obj: Mutex<HashMap<(u32, u32), u32>>,
    /// The worker eventfd (a schedule that finds work pending wakes one).
    wake: &'static kf_linux_raw::Notifier,
    /// Serves that found the slot lock held — must stay 0 (BUSY excludes).
    pub contended: AtomicU64,
    /// Channels refused-and-poisoned by the plane (§7, kernel).
    pub poisoned: AtomicU64,
    /// Births.
    pub births: AtomicU64,
    stop: AtomicBool,
}

impl ChanPlane {
    /// Build the plane: open the session's completion fd (host ioctls — realize, never a vCPU).
    ///
    /// # Errors
    /// The host's refusal, by name.
    pub fn new(
        rm: &'static HostRm,
        plane: &'static Plane<'static>,
        store: u32,
        ram: &'static RamMap,
        mirrors: Mirrors,
        wake: &'static kf_linux_raw::Notifier,
        tokens: usize,
    ) -> Result<ChanPlane, String> {
        Ok(ChanPlane {
            rm,
            plane,
            store,
            ram,
            mirrors,
            completions: Completions::open(rm, tokens)?,
            // Declared caps: channels are the only twin this plane mints.
            caps: Mutex::new(VmCaps::from_declared(64, 64, 64, 64)),
            slots: RwLock::new(HashMap::new()),
            by_obj: Mutex::new(HashMap::new()),
            wake,
            contended: AtomicU64::new(0),
            poisoned: AtomicU64::new(0),
            births: AtomicU64::new(0),
            stop: AtomicBool::new(false),
        })
    }

    /// ★ The drainer's entry: one statement from the served chain (`kf_rm::chanlink`).
    pub fn statement(&self, st: ChanStatement) -> ChanAnswer {
        match st {
            ChanStatement::Alloc(a) => self.birth(a),
            ChanStatement::Schedule { client, object, enable } => {
                let Some(ht) = self.by_obj.lock().ok().and_then(|m| m.get(&(client, object)).copied()) else {
                    return ChanAnswer::NotOurs;
                };
                let Some(slot) = self.slot(ht) else { return ChanAnswer::NotOurs };
                let idx = match slot.lock() {
                    Ok(mut g) => {
                        g.scheduled = enable;
                        g.guest_idx
                    }
                    Err(_) => return ChanAnswer::Refused { status: NV_ERR_INVALID_STATE, why: "slot poisoned".into() },
                };
                eprintln!("kf3: chan {client:#x}:{object:#x} GPFIFO_SCHEDULE enable={enable} (token {idx:#x}, host {ht:#x})");
                // Work the guest queued before scheduling is picked up now.
                if enable && self.plane.ring_internal(idx) {
                    let _ = self.wake.signal();
                }
                ChanAnswer::Done
            }
            ChanStatement::Bind { client, object, engine_type } => {
                let Some(ht) = self.by_obj.lock().ok().and_then(|m| m.get(&(client, object)).copied()) else {
                    return ChanAnswer::NotOurs;
                };
                // ★ The guest's statement that this channel runs on a copy engine. Our twin's host
                // TSG was bound to OUR engine at birth (`HostRm::birth_channel`); a bind to anything
                // but a copy engine contradicts the Translated route and is refused by name.
                if !is_copy_engine(engine_type) {
                    return ChanAnswer::Refused {
                        status: NV_ERR_INVALID_STATE,
                        why: format!("BIND of a Translated CE channel (host {ht:#x}) to engine {engine_type:#x}"),
                    };
                }
                eprintln!("kf3: chan {client:#x}:{object:#x} BIND engine={engine_type:#x} (host {ht:#x})");
                ChanAnswer::Done
            }
            ChanStatement::Token { client, object } => {
                let Some(ht) = self.by_obj.lock().ok().and_then(|m| m.get(&(client, object)).copied()) else {
                    return ChanAnswer::NotOurs;
                };
                match self.slot(ht).and_then(|s| s.lock().ok().map(|g| g.guest_idx)) {
                    // ★ The token is OUR doorbell's vocabulary (we are the host): the table index,
                    // in the VECTOR field the trap masks (`kf_trap::trap`). Its RUNLIST_ID/flag bits
                    // are the guest's own when it computes the token itself (GA10x kernel channels).
                    Some(idx) => ChanAnswer::Token(idx),
                    None => ChanAnswer::NotOurs,
                }
            }
            ChanStatement::Free { client, object } => {
                let Some(ht) = self.by_obj.lock().ok().and_then(|mut m| m.remove(&(client, object))) else {
                    return ChanAnswer::NotOurs;
                };
                self.retire(ht);
                ChanAnswer::Done
            }
        }
    }

    fn slot(&self, ht: u32) -> Option<Arc<Mutex<Slot>>> {
        self.slots.read().ok()?.get(&ht).cloned()
    }

    /// ★ Birth AT ALLOCATION (§7). Only the guest-KERNEL copy-engine channels are Translated; any
    /// other channel is NOT ours yet (user channels are Passthrough, P5 step 5) and the chain answers
    /// as before.
    fn birth(&self, a: ChannelAlloc) -> ChanAnswer {
        let engine = a.engine_type.unwrap_or(0);
        if !a.kernel_client || !is_copy_engine(engine) {
            eprintln!(
                "kf3: chan {:#x}:{:#x} class={:#x} engine={engine:#x} kernel={} vaspace={:x?} chid={:x?} — not a kernel CE channel: not born (Passthrough is P5 step 5)",
                a.client, a.handle, a.class, a.kernel_client, a.vaspace, a.chid
            );
            return ChanAnswer::NotOurs;
        }
        let refuse = |status: u32, why: String| {
            eprintln!("kf3: chan {:#x}:{:#x} birth REFUSED: {why} (decl {a:x?})", a.client, a.handle);
            ChanAnswer::Refused { status, why }
        };
        let Some(vas) = a.vaspace else {
            return refuse(NV_ERR_INVALID_STATE, format!("no VA space resolved (hVASpace={:#x}, parent {:#x})", a.h_vaspace, a.parent));
        };
        let key = VasKey((u64::from(a.client) << 32) | u64::from(vas));
        let Some(mirror) = self.mirrors.lock().ok().and_then(|m| m.get(&key).cloned()) else {
            return refuse(NV_ERR_INVALID_STATE, format!("VA space {key:?} has no mirror (no page-directory statement named it)"));
        };
        let Some(chid) = a.chid else {
            return refuse(NV_ERR_INVALID_STATE, format!("no guest chid in flags {:#x} (USERD_INDEX not fixed)", a.flags));
        };
        let idx = chid & self.plane.token_mask;
        if idx != chid || (idx as usize) >= self.plane.tokens.len() {
            return refuse(NV_ERR_INSUFFICIENT_RESOURCES, format!("chid {chid:#x} outside the token table"));
        }
        let userd = match self.userd_view(a.userd) {
            Ok(u) => u,
            Err(e) => return refuse(NV_ERR_NOT_SUPPORTED, e),
        };
        let t0 = std::time::Instant::now();
        let host = match HostRing::new(self.rm, mirror.space) {
            Ok(h) => h,
            Err(e) => return refuse(NV_ERR_INSUFFICIENT_RESOURCES, format!("host ring: {e}")),
        };
        let ht = host.channel().token;
        let entries = a.entries.max(1);
        let chan = TranslatedChannel::new(TranslatedRing::new(a.gpfifo_va, entries, 0), host, idx);
        let alloc = self
            .caps
            .lock()
            .map_err(|_| "caps poisoned".to_string())
            .and_then(|mut c| self.plane.allocate_channel(&mut c, idx, Route::Translated, ht, Owner::Kernel).map_err(|e| format!("{e:?}")));
        if let Err(e) = alloc {
            return refuse(NV_ERR_INSUFFICIENT_RESOURCES, format!("token {idx:#x}: {e}"));
        }
        let slot = Slot { chan, key, mirror, userd, guest_idx: idx, scheduled: false, dead: None };
        if let Ok(mut s) = self.slots.write() {
            s.insert(ht, Arc::new(Mutex::new(slot)));
        }
        if let Ok(mut m) = self.by_obj.lock() {
            m.insert((a.client, a.handle), ht);
        }
        self.births.fetch_add(1, Ordering::Relaxed);
        eprintln!(
            "kf3: chan {:#x}:{:#x} BORN Translated: token {idx:#x} -> host {ht:#x} in {key:?} gpfifo={:#x}x{} userd={:?} engine={engine:#x} ({} us)",
            a.client,
            a.handle,
            a.gpfifo_va,
            entries,
            a.userd,
            t0.elapsed().as_micros()
        );
        ChanAnswer::Done
    }

    /// The guest's USERD, reached through a CPU view WE arm now (off the vCPU).
    fn userd_view(&self, u: Option<kf_arch::UserdMem>) -> Result<UserdView, String> {
        match u {
            Some(kf_arch::UserdMem::Framebuffer { base, .. }) => {
                let page = base & !0xFFF;
                let (node, cookie) = self
                    .rm
                    .arm_cpu_view(MapNode::Gpu, self.store, page, 0x1000, ViewAccess::ReadWrite)
                    .map_err(|e| format!("USERD view of store {page:#x}: {e:?}"))?;
                let region = VolatileRegion::map(Backing::DeviceFile { fd: node.as_fd() }, 0x1000, CachePolicy::Uncached, HostPageSize::query())
                    .map_err(|e| format!("USERD mmap: {e:?}"))?;
                Ok(UserdView::Store { region, _node: node, cookie, at: base - page })
            }
            Some(kf_arch::UserdMem::Sysmem { base, .. }) => {
                let b = self.ram.block_for(base, 0x200).ok_or_else(|| format!("USERD at guest-physical {base:#x}: no RAM block"))?;
                Ok(UserdView::Ram { mem: b.mem, at: (base - b.gpa) as usize })
            }
            other => Err(format!("USERD not declared as a physical descriptor ({other:?})")),
        }
    }

    fn retire(&self, ht: u32) {
        let Some(slot) = self.slots.write().ok().and_then(|mut s| s.remove(&ht)) else { return };
        let Ok(g) = slot.lock() else { return };
        if let Ok(mut c) = self.caps.lock() {
            // §5.2: free waits out BUSY; the slot lock is held, so no worker is inside it.
            let _ = self.plane.free_channel(&mut c, g.guest_idx);
        }
        let _ = self.rm.free_channel(g.chan.host().channel());
        if let UserdView::Store { cookie, .. } = &g.userd {
            let _ = self.rm.release_cpu_view(kf_host::CpuViewRelease { h_memory: self.store, p_linear_address: *cookie });
        }
        eprintln!("kf3: chan token {:#x} (host {ht:#x}) RETIRED, forwarded={}", g.guest_idx, g.chan.counts().0);
    }

    /// ★ A WORKER's entry (`HostOps::run_translated`): pump the channel behind `ht`. Never waits.
    /// Returns whether anything reached the GPU.
    pub fn serve(&self, ht: u32) -> bool {
        let Some(slot) = self.slot(ht) else { return false };
        // ★ The token's BUSY state is the exclusion; this lock must never be contended.
        let Ok(mut g) = slot.try_lock() else {
            self.contended.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        let g = &mut *g;
        if g.dead.is_some() || !g.scheduled || self.stop.load(Ordering::Acquire) {
            return false;
        }
        let before = g.chan.counts().1;
        let mirror = g.mirror.clone();
        let mut mem = Mem { mirror: &mirror, ram: self.ram };
        let win = SlotWindow { mirror: &mirror, ram: self.ram };
        let r = g.chan.pump(self.rm, &self.completions, &mut mem, &mut Userd(&g.userd), &mut NoSplit, is_any_ce_class, &win);
        if let Err(e) = r {
            let why = match e {
                ChanError::Ring(r) => format!("ring: {r:?}"),
                ChanError::Host(h) => format!("host: {h}"),
                ChanError::Userd(u) => format!("userd: {u}"),
                ChanError::Publish(p) => format!("split: {p}"),
            };
            eprintln!("kf3: chan token {:#x} ({:?}) DEAD: {why}", g.guest_idx, g.key);
            g.dead = Some(why);
            self.completions.clear(g.guest_idx);
        }
        g.chan.counts().1 > before
    }

    /// Whether the channel behind `ht` can take work (a dead one cannot: §7 then poisons).
    #[must_use]
    pub fn alive(&self, ht: u32) -> bool {
        self.slot(ht).is_some_and(|s| s.try_lock().map_or(true, |g| g.dead.is_none()))
    }

    /// `forwarded=` per token, for the boot log.
    #[must_use]
    pub fn counts(&self) -> Vec<TokenCount> {
        let Ok(s) = self.slots.read() else { return Vec::new() };
        let mut v: Vec<TokenCount> = s
            .values()
            .filter_map(|slot| {
                let g = slot.try_lock().ok()?;
                let (fwd, subs, _) = g.chan.counts();
                Some(TokenCount { token: g.guest_idx, forwarded: fwd, submissions: subs, gp_get: g.chan.last_gp_get(), dead: g.dead.clone() })
            })
            .collect();
        v.sort_by_key(|c| c.token);
        v
    }

    /// Stop serving (device teardown).
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
}
