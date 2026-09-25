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
//! ⊘ **Nothing here runs on a vCPU.** Pumps are the workers'. ★ P5b: every host ACT a statement
//! implies (a birth, an engine object, a schedule, a free) runs on the plane's own ACT thread
//! (`kf3-chan-act`), never on the register drainer — which serves statements under the GSP lock
//! (owner invariant: no blocking under a lock). The statement's reply is HELD (`kf_gsp::Deferred`)
//! until the act resolves it: the reply still IS the act, the drainer just does not wait for it.
//!
//! ## ★ P5b — guest USER channels are PASSTHROUGH twins
//!
//! A user channel names only VIRTUAL addresses in a VA space the memory plane mirrors, so its twin
//! is born over the guest's own GPFIFO VA and USERD (`kf_chan::passthrough`), its engine objects
//! follow the guest's own allocs (the guest's class, our params), its schedule the guest's own
//! `GPFIFO_SCHEDULE`, and its token is `Route::Passthrough` — the vCPU rings the twin INLINE
//! (`Action::RingHostInline`). ⊘ Nothing here reads its pushbuffer, GP entries or cursors.
//!
//! ## ★ P5b §2.7 — completion interrupts
//!
//! One host event fd per host ENGINE (GR0, every copy engine), dataless + non-stall + REPEAT. Its
//! readiness in a worker's poller latches the guest's non-stall vector for that engine (read back
//! out of the served `intr_table`, `kf_rm::authored::non_stall_vector_for`) and writes the MSI-X
//! irqfd — `Device::latch_and_deliver`. ⊘ Never forged, never inline: a host NSI is the only
//! trigger, and it carries no channel identity (RM's waiters re-check their semaphores).

use crate::mem::{Mirror, Mirrors, RamMap, resolve_placed_prefix};
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
/// `NV_ERR_INVALID_CLASS` (`ogkm-580: nvstatuscodes.h:63`).
const NV_ERR_INVALID_CLASS: u32 = 0x22;
/// `NV2080_NOTIFIERS_GR0` = `NV2080_NOTIFIERS_GRAPHICS` (`ogkm-580: cl2080_notification.h:48,185`).
const NV2080_NOTIFIERS_GR0: u32 = 12;

/// ★ P5b: one host act, run on the plane's act thread. `Err((status, why))` refuses by name.
type Act = Box<dyn FnOnce(&ChanPlane) -> Result<String, (u32, String)> + Send>;

/// ★ P5b: a guest USER channel's host twin (Passthrough).
struct PtChan {
    chan: kf_host::Channel,
    /// The guest token (table index = the guest's chid).
    idx: u32,
    /// The channel group it was allocated under.
    tsg: Option<u32>,
    /// The channel's parent (its TSG, or its device).
    parent: u32,
    /// The host engine (= the guest's `engineType`).
    engine: u32,
    /// Engine objects on the twin: guest handle → host handle.
    objects: HashMap<u32, u32>,
    /// ★ P5c: the mirror's live-channel count (released at free).
    live: Arc<AtomicU64>,
    /// ★ P5c: the guest's error notifier, as the twin's host error context — `None` when the
    /// guest declared none (or it could not be armed, named at birth).
    notifier: Option<PtNotifier>,
}

/// ★ P5c: a twin's error context: the host `NV01_CONTEXT_DMA` over the guest's notifier record
/// (the host's RC path writes the record there natively) and a read view of that record's 16 bytes
/// (the RC plane reads `info32`/`status` to name the event it forwards). ⊘ Read only — the CPU never
/// writes the record.
struct PtNotifier {
    ctx: u32,
    view: UserdView,
    /// Already forwarded to the guest (one RC per channel life).
    reported: bool,
    /// The record's four words when it was armed: only a record the HOST changed is an event (a
    /// guest may initialise its notifier to anything).
    at_arm: [u32; 4],
}

impl PtNotifier {
    fn words(&self) -> Option<[u32; 4]> {
        let mut w = [0u32; 4];
        for (i, x) in w.iter_mut().enumerate() {
            *x = self.view.load(4 * i as u64).ok()?;
        }
        Some(w)
    }
}

/// ★ P5c: one host robust-channel event, bound for the guest as `RC_TRIGGERED`.
#[derive(Debug, Clone, Copy)]
pub struct RcEvent {
    /// The guest's chid (the token-table index).
    pub chid: u32,
    /// The guest's declared `nv2080EngineType` for the channel.
    pub engine: u32,
    /// `info32` of the record the host wrote — the `ROBUST_CHANNEL_*` code.
    pub except_type: u32,
    /// The twin's host token (for the log).
    pub host_token: u32,
}

/// ★ P5b §2.7: one host engine's non-stall event, and the guest vector it is announced on.
pub struct EngineEvent {
    /// The engine (`kf_rm::authored::EngineKind` naming).
    pub name: String,
    /// The host event fd (a worker's poller watches it).
    pub ev: kf_host::EventFd,
    /// The guest vector (`None`: the served table has none — counted, never raised).
    pub vector: Option<u32>,
    /// The host engine (`NV2080_ENGINE_TYPE_*`).
    pub engine_type: u32,
    /// Live guest twins on this engine: a wake with none is not the guest's work (our own
    /// Translated rings, the GPU walker's copies, another host tenant) and raises nothing.
    pub live: AtomicU64,
    /// Wakes seen.
    pub wakes: AtomicU64,
    /// Wakes raised to the guest.
    pub raised: AtomicU64,
}

/// A CE class id on ANY family — the class tables are generated per family and class ids are
/// unique across them, so this needs no family argument (the rewriter takes a plain `fn`).
fn is_any_ce_class(c: u32) -> bool {
    kf_chip::Family::ALL.iter().any(|f| kf_chip::classes_for(*f).dma_copy.contains(&c))
}

/// A copy engine — `kf_chan::passthrough::is_copy_engine` (both of the header's blocks; the P5
/// copy of this read `0x28..` for COPY10, which is not a copy engine).
fn is_copy_engine(engine_type: u32) -> bool {
    kf_chan::passthrough::is_copy_engine(engine_type)
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
        // ★ P5b: piece by piece across OUR rows — a segment may span two adjacent placements.
        let mut done = 0u64;
        while done < len {
            let at_va = va + done;
            let (ram, off, avail) = resolve_placed_prefix(&self.mirror.rows, at_va)
                .ok_or_else(|| format!("{va:#x}+{len:#x}: {at_va:#x} not placed by us"))?;
            if !ram {
                // ⊘ Q3: a vidmem pushbuffer would need a GPU read (the walker's context lives on the
                // VA thread). RM's CeUtils puts GPFIFO + pushbuffer in SYSMEM by default
                // (`ogkm-580: mem_utils_gm107.c:779-791`), so this is refused by name, not guessed.
                return Err(format!("{at_va:#x}: pushbuffer/GPFIFO in VIDMEM (store {off:#x}) — GPU read not wired"));
            }
            let n = avail.min(len - done);
            let (mem, at) = self.ram.at_file_offset(off, n).ok_or_else(|| format!("{at_va:#x}: guest-RAM offset {off:#x} unregistered"))?;
            let dst = &mut out[done as usize..(done + n) as usize];
            if !mem.read_into(at, dst) {
                return Err(format!("{at_va:#x}: guest-RAM read"));
            }
            done += n;
        }
        Ok(())
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
    /// Serves (doorbells + completion rings that reached this channel).
    serves: u64,
    /// The last `GP_PUT` the pump read (for the log).
    last_put: Option<u32>,
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
    /// Serves that reached the channel.
    pub serves: u64,
    /// The guest's `GP_PUT` at the last serve.
    pub last_put: Option<u32>,
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
    /// ★ The HOST async copy engine our rings run on (authored: the first copy engine the host
    /// says is not a GRCE).
    pub host_ce: u32,
    stop: AtomicBool,
    /// The host family (its generated class sets check a guest engine-object class).
    family: kf_chip::Family,
    /// ★ P5b: the guest's user channels' twins, by `(hClient, hChannel)`.
    pt: Mutex<HashMap<(u32, u32), PtChan>>,
    /// ★ P5b: guest engine object `(hClient, hObject)` → its channel's key.
    pt_objs: Mutex<HashMap<(u32, u32), (u32, u32)>>,
    /// ★ P5b: the act thread's queue (`None` until [`ChanPlane::start`]).
    acts: Mutex<Option<std::sync::mpsc::Sender<(Act, kf_gsp::Deferred, &'static str)>>>,
    /// The register drainer's wake: an act that resolved a held reply signals it.
    release: &'static kf_linux_raw::Notifier,
    /// ★ P5b §2.7: the per-engine host non-stall events.
    pub engines: Vec<EngineEvent>,
    /// Acts run, refused, and the slowest (µs) — the log's proof that births left the lock.
    pub acts_run: AtomicU64,
    /// Acts refused.
    pub acts_refused: AtomicU64,
    /// Slowest act, µs.
    pub act_worst_us: AtomicU64,
    /// Passthrough twins born.
    pub pt_births: AtomicU64,
    /// ★ Per guest token: doorbells the vCPU trap rang INLINE, and how many reached the host's
    /// doorbell (the `DOORBELL-LEDGER` line at free; atomics only — the vCPU writes them).
    rung: Box<[AtomicU64]>,
    rang: Box<[AtomicU64]>,
    /// ★ P5c: the host robust-channel event fd — one dataless `NV01_EVENT_OS_EVENT` per twin's
    /// context DMA (notify index 0: `krcErrorSendEventNotificationsCtxDma_FWCLIENT` walks exactly
    /// those, `kernel_rc_notification.c:380-400`), all on this fd. A worker's poller watches it.
    pub rc_ev: kf_host::EventFd,
    /// ★ P5c: RC events waiting for the register drainer (which owns the GSP queue).
    rc_queue: Mutex<Vec<RcEvent>>,
    /// Twins born with the guest's notifier armed as their host error context.
    pub rc_armed: AtomicU64,
    /// Twins whose declared notifier could NOT be armed (named at birth) — their faults are silent.
    pub rc_unarmed: AtomicU64,
    /// RC records seen on a twin's notifier (host-written).
    pub rc_seen: AtomicU64,
    /// Wakes of the RC fd.
    pub rc_wakes: AtomicU64,
}

impl ChanPlane {
    /// Build the plane: open the session's completion fd (host ioctls — realize, never a vCPU).
    ///
    /// # Errors
    /// The host's refusal, by name.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        rm: &'static HostRm,
        plane: &'static Plane<'static>,
        store: u32,
        ram: &'static RamMap,
        mirrors: Mirrors,
        wake: &'static kf_linux_raw::Notifier,
        release: &'static kf_linux_raw::Notifier,
        tokens: usize,
        family: kf_chip::Family,
        intr_table: &[kf_abi::inittables::IntrTableEntry],
    ) -> Result<ChanPlane, String> {
        // ★ Authored, never the guest's engine number: the first HOST copy engine that is not a
        // graphics CE. ⊘ A GRCE shares the GR runlist and routes subchannels 0-3 to GR — RM's
        // CeUtils pushes on subchannel 0, and `[measured p5f]` our ring on host COPY0 faulted
        // CTXNOTVALID (Xid 32) on the first guest segment.
        let host_ce = (0..10u32)
            .filter_map(kf_abi::submit::engine_type_copy)
            .find(|&et| rm.ce_is_grce(et) == Ok(false))
            .ok_or("no async (non-GRCE) copy engine on the host — a Translated ring has nowhere to run")?;
        let completions = Completions::open(rm, tokens)?;
        let lce = host_ce - kf_abi::submit::ENGINE_TYPE_COPY0;
        completions.also(rm, kf_host::event::notifier_ce(lce))?;
        eprintln!("kf3: channel plane: Translated rings on host COPY{lce} (engine {host_ce:#x}); completions on FIFO_EVENT_MTHD + CE{lce}");
        // ★ P5b §2.7: one non-stall event per HOST engine a twin can run on — GR0 and every copy
        // engine the host has (`CE_GET_CAPS_V2` answers for it). Realize-time host ioctls, never a
        // vCPU. A GRCE's completions announce on GR0's vector (it has no row of its own).
        let mut engines = Vec::new();
        let mut kinds = vec![(kf_rm::authored::EngineKind::Graphics(0), NV2080_NOTIFIERS_GR0, kf_abi::submit::ENGINE_TYPE_GRAPHICS)];
        for i in 0..20u32 {
            if let Some(et) = kf_chan::passthrough::copy_engine_type(i)
                && rm.ce_is_grce(et).is_ok()
            {
                kinds.push((kf_rm::authored::EngineKind::Copy(i), kf_host::event::notifier_ce(i), et));
            }
        }
        for (kind, notify, engine_type) in kinds {
            let ev = rm.open_event_fd().map_err(|e| format!("{} event fd: {e:?}", kind.name()))?;
            rm.alloc_os_event(rm.subdevice(), notify, true, &ev).map_err(|e| format!("{} os event: {e:?}", kind.name()))?;
            rm.arm_repeat(notify).map_err(|e| format!("{} notify: {e:?}", kind.name()))?;
            let vector = kf_rm::authored::non_stall_vector_for(intr_table, kind);
            engines.push(EngineEvent {
                name: kind.name(),
                ev,
                vector,
                engine_type,
                live: AtomicU64::new(0),
                wakes: AtomicU64::new(0),
                raised: AtomicU64::new(0),
            });
        }
        eprintln!(
            "kf3: interrupt plane: host non-stall events -> guest vectors [{}]",
            engines.iter().map(|e| format!("{}->{:x?}", e.name, e.vector)).collect::<Vec<_>>().join(" ")
        );
        // ★ P5c: the RC fd (realize-time host ioctls, never a vCPU).
        let rc_ev = rm.open_event_fd().map_err(|e| format!("RC event fd: {e:?}"))?;
        Ok(ChanPlane {
            rm,
            plane,
            store,
            ram,
            mirrors,
            completions,
            // Declared caps: channels are the only twin this plane mints.
            caps: Mutex::new(VmCaps::from_declared(64, 64, 64, 64)),
            slots: RwLock::new(HashMap::new()),
            by_obj: Mutex::new(HashMap::new()),
            wake,
            contended: AtomicU64::new(0),
            poisoned: AtomicU64::new(0),
            births: AtomicU64::new(0),
            host_ce,
            stop: AtomicBool::new(false),
            family,
            pt: Mutex::new(HashMap::new()),
            pt_objs: Mutex::new(HashMap::new()),
            acts: Mutex::new(None),
            release,
            engines,
            acts_run: AtomicU64::new(0),
            acts_refused: AtomicU64::new(0),
            act_worst_us: AtomicU64::new(0),
            pt_births: AtomicU64::new(0),
            rung: (0..tokens).map(|_| AtomicU64::new(0)).collect(),
            rang: (0..tokens).map(|_| AtomicU64::new(0)).collect(),
            rc_ev,
            rc_queue: Mutex::new(Vec::new()),
            rc_armed: AtomicU64::new(0),
            rc_unarmed: AtomicU64::new(0),
            rc_seen: AtomicU64::new(0),
            rc_wakes: AtomicU64::new(0),
        })
    }

    /// ★ P5b: start the ACT thread — every host act a statement implies runs here, in statement
    /// order, off the drainer and off every lock the drainer holds.
    ///
    /// # Errors
    /// The spawn.
    pub fn start(&'static self) -> Result<(), String> {
        let (tx, rx) = std::sync::mpsc::channel::<(Act, kf_gsp::Deferred, &'static str)>();
        std::thread::Builder::new()
            .name("kf3-chan-act".into())
            .spawn(move || {
                while let Ok((act, d, what)) = rx.recv() {
                    let t0 = std::time::Instant::now();
                    let r = act(self);
                    let us = u64::try_from(t0.elapsed().as_micros()).unwrap_or(u64::MAX);
                    self.act_worst_us.fetch_max(us, Ordering::Relaxed);
                    self.acts_run.fetch_add(1, Ordering::Relaxed);
                    match r {
                        Ok(line) => {
                            eprintln!("kf3: act {what}: {line} ({us} us, off the GSP lock)");
                            d.resolve(0);
                        }
                        Err((status, why)) => {
                            self.acts_refused.fetch_add(1, Ordering::Relaxed);
                            eprintln!("kf3: act {what} REFUSED ({status:#x}): {why} ({us} us)");
                            d.resolve(status);
                        }
                    }
                    // The held reply may go now: the drainer posts it.
                    let _ = self.release.signal();
                }
            })
            .map_err(|e| format!("act thread: {e}"))?;
        if let Ok(mut a) = self.acts.lock() {
            *a = Some(tx);
        }
        Ok(())
    }

    /// ★ **vCPU**: a doorbell on guest token `idx` was rung inline; `reached` = the host store
    /// succeeded. Two relaxed atomics — the only thing this plane does on a vCPU.
    pub fn note_inline(&self, idx: u32, reached: bool) {
        if let Some(c) = self.rung.get(idx as usize) {
            c.fetch_add(1, Ordering::Relaxed);
        }
        if reached && let Some(c) = self.rang.get(idx as usize) {
            c.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// `(rung, reached)` for token `idx`, reset (a freed token's successor starts at zero).
    fn take_ledger(&self, idx: u32) -> (u64, u64) {
        let r = self.rung.get(idx as usize).map_or(0, |c| c.swap(0, Ordering::Relaxed));
        let f = self.rang.get(idx as usize).map_or(0, |c| c.swap(0, Ordering::Relaxed));
        (r, f)
    }

    fn engine_live(&self, engine_type: u32, up: bool) {
        if let Some(e) = self.engines.iter().find(|e| e.engine_type == engine_type) {
            if up {
                e.live.fetch_add(1, Ordering::Relaxed);
            } else {
                let _ = e.live.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(1));
            }
        }
    }

    /// Queue `act` and answer [`ChanAnswer::Deferred`] — the drainer returns at once.
    fn defer(&self, what: &'static str, act: Act) -> ChanAnswer {
        let d = kf_gsp::Deferred::new();
        let sent = self.acts.lock().ok().and_then(|a| a.as_ref().map(|tx| tx.send((act, d.clone(), what)).is_ok()));
        if sent == Some(true) {
            ChanAnswer::Deferred(d)
        } else {
            ChanAnswer::Refused { status: NV_ERR_INVALID_STATE, why: format!("{what}: the act thread is not running") }
        }
    }

    /// The passthrough twins matching `f`, removed.
    fn take_pt(&self, f: impl Fn(&(u32, u32), &PtChan) -> bool) -> Vec<((u32, u32), PtChan)> {
        let Ok(mut m) = self.pt.lock() else { return Vec::new() };
        let keys: Vec<(u32, u32)> = m.iter().filter(|(k, v)| f(k, v)).map(|(k, _)| *k).collect();
        keys.into_iter().filter_map(|k| m.remove(&k).map(|v| (k, v))).collect()
    }

    /// ★ The drainer's entry: one statement from the served chain (`kf_rm::chanlink`). ⊘ Never
    /// blocks: a statement that implies a host act answers [`ChanAnswer::Deferred`] and the act
    /// runs on the act thread (P5b).
    pub fn statement(&self, st: ChanStatement) -> ChanAnswer {
        match st {
            ChanStatement::Alloc(a) => self.birth(a),
            ChanStatement::Schedule { client, object, enable } => {
                if let Some(ht) = self.by_obj.lock().ok().and_then(|m| m.get(&(client, object)).copied()) {
                    return self.schedule_translated(client, object, ht, enable);
                }
                // ★ P5b: a user channel (or its group) — the twin's own schedule, as an act.
                let twins: Vec<kf_host::Channel> = self
                    .pt
                    .lock()
                    .map(|m| {
                        m.iter()
                            .filter(|(k, v)| k.0 == client && (k.1 == object || v.tsg == Some(object)))
                            .map(|(_, v)| v.chan)
                            .collect()
                    })
                    .unwrap_or_default();
                if twins.is_empty() {
                    return ChanAnswer::NotOurs;
                }
                self.defer(
                    "schedule",
                    Box::new(move |me: &ChanPlane| {
                        for c in &twins {
                            me.rm.schedule_enable(*c, enable).map_err(|e| (NV_ERR_INVALID_STATE, format!("host {:#x}: {e:?}", c.token)))?;
                        }
                        Ok(format!("{client:#x}:{object:#x} GPFIFO_SCHEDULE enable={enable} on {} twin(s)", twins.len()))
                    }),
                )
            }
            ChanStatement::Bind { client, object, engine_type } => {
                if let Some(ht) = self.by_obj.lock().ok().and_then(|m| m.get(&(client, object)).copied()) {
                    // ★ The guest's statement that this channel runs on a copy engine. Our twin's
                    // host TSG was bound to OUR engine at birth; a bind to anything but a copy
                    // engine contradicts the Translated route and is refused by name.
                    if !is_copy_engine(engine_type) {
                        return ChanAnswer::Refused {
                            status: NV_ERR_INVALID_STATE,
                            why: format!("BIND of a Translated CE channel (host {ht:#x}) to engine {engine_type:#x}"),
                        };
                    }
                    eprintln!("kf3: chan {client:#x}:{object:#x} BIND engine={engine_type:#x} (host {ht:#x})");
                    return ChanAnswer::Done;
                }
                // ★ P5b: the twin's TSG was bound to the guest's engine at birth; the guest's own
                // BIND must name that engine (the runlist the guest computes its token from).
                let twin = self.pt.lock().ok().and_then(|m| m.get(&(client, object)).map(|v| (v.engine, v.chan.token)));
                match twin {
                    None => ChanAnswer::NotOurs,
                    Some((e, ht)) if e == engine_type => {
                        eprintln!("kf3: chan {client:#x}:{object:#x} BIND engine={engine_type:#x} (passthrough host {ht:#x})");
                        ChanAnswer::Done
                    }
                    Some((e, ht)) => ChanAnswer::Refused {
                        status: NV_ERR_INVALID_STATE,
                        why: format!("BIND to engine {engine_type:#x} of a twin born on {e:#x} (host {ht:#x})"),
                    },
                }
            }
            ChanStatement::Token { client, object } => {
                if let Some(ht) = self.by_obj.lock().ok().and_then(|m| m.get(&(client, object)).copied()) {
                    return match self.slot(ht).and_then(|s| s.lock().ok().map(|g| g.guest_idx)) {
                        // ★ The token is OUR doorbell's vocabulary (we are the host): the table
                        // index, in the VECTOR field the trap masks (`kf_trap::trap`).
                        Some(idx) => ChanAnswer::Token(idx),
                        None => ChanAnswer::NotOurs,
                    };
                }
                match self.pt.lock().ok().and_then(|m| m.get(&(client, object)).map(|v| v.idx)) {
                    Some(idx) => ChanAnswer::Token(idx),
                    None => ChanAnswer::NotOurs,
                }
            }
            ChanStatement::EngineObject { client, parent, handle, class } => self.engine_object(client, parent, handle, class),
            ChanStatement::Free { client, object } => self.free(client, object),
        }
    }

    fn schedule_translated(&self, client: u32, object: u32, ht: u32, enable: bool) -> ChanAnswer {
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

    /// ★ P5b: an engine object under a PASSTHROUGH twin — allocated on the twin with the guest's
    /// class (checked against the HOST family's generated set for the twin's engine) and params
    /// we author. Under a Translated channel it is a graph node only (our ring owns its object).
    fn engine_object(&self, client: u32, parent: u32, handle: u32, class: u32) -> ChanAnswer {
        let Some((chan, engine)) = self.pt.lock().ok().and_then(|m| m.get(&(client, parent)).map(|v| (v.chan, v.engine))) else {
            return ChanAnswer::NotOurs;
        };
        let Some(kind) = kf_chip::classes_for(self.family).kind_of(class) else {
            return ChanAnswer::Refused {
                status: NV_ERR_INVALID_CLASS,
                why: format!("class {class:#x} is not an engine class of the host family {:?}", self.family),
            };
        };
        self.defer(
            "engine object",
            Box::new(move |me: &ChanPlane| {
                let h = kf_chan::passthrough::engine_object(me.rm, chan, engine, class, kind).map_err(|e| (NV_ERR_INVALID_CLASS, e))?;
                if let Ok(mut m) = me.pt.lock()
                    && let Some(v) = m.get_mut(&(client, parent))
                {
                    v.objects.insert(handle, h);
                }
                if let Ok(mut m) = me.pt_objs.lock() {
                    m.insert((client, handle), (client, parent));
                }
                Ok(format!("{client:#x}:{handle:#x} class {class:#x} ({kind:?}) on twin host {:#x} -> host object {h:#x}", chan.token))
            }),
        )
    }

    /// ★ A free the plane may own: a Translated channel, a passthrough twin (by channel, its group,
    /// its device, or its client), or an engine object on one. Tokens stop routing to a twin NOW
    /// (atomics, no host call); the host frees run as ONE act whose outcome the reply waits for.
    fn free(&self, client: u32, object: u32) -> ChanAnswer {
        // Translated channels: by object, or every one of the client's.
        let translated: Vec<u32> = self
            .by_obj
            .lock()
            .map(|mut m| {
                let keys: Vec<(u32, u32)> = m.keys().copied().filter(|k| k.0 == client && (object == client || k.1 == object)).collect();
                keys.into_iter().filter_map(|k| m.remove(&k)).collect()
            })
            .unwrap_or_default();
        let twins = self.take_pt(|k, v| k.0 == client && (object == client || k.1 == object || v.tsg == Some(object) || v.parent == object));
        // Engine objects freed on their own (their twin still lives).
        let obj = if twins.is_empty() {
            self.pt_objs.lock().ok().and_then(|mut m| m.remove(&(client, object))).and_then(|key| {
                self.pt.lock().ok().and_then(|mut m| m.get_mut(&key).and_then(|v| v.objects.remove(&object)))
            })
        } else {
            None
        };
        if translated.is_empty() && twins.is_empty() && obj.is_none() {
            return ChanAnswer::NotOurs;
        }
        if !twins.is_empty() {
            if let Ok(mut m) = self.pt_objs.lock() {
                m.retain(|_, key| !twins.iter().any(|(k, _)| k == key));
            }
            // ★ The guest's token stops ringing the twin before its host free (a Passthrough token
            // is never BUSY: no worker ever claims one).
            if let Ok(mut c) = self.caps.lock() {
                for (_, t) in &twins {
                    let _ = self.plane.free_channel(&mut c, t.idx);
                }
            }
        }
        for ht in &translated {
            // Stop the pump before the act frees its twin (the slot lock is the worker's).
            if let Some(s) = self.slot(*ht)
                && let Ok(mut g) = s.try_lock()
            {
                g.scheduled = false;
            }
        }
        self.defer(
            "free",
            Box::new(move |me: &ChanPlane| {
                let mut line = Vec::new();
                for ht in translated {
                    me.retire(ht);
                    line.push(format!("translated host {ht:#x}"));
                }
                for ((c, h), t) in twins {
                    let r = me.rm.free_channel(t.chan);
                    t.live.fetch_sub(1, Ordering::AcqRel);
                    // The error context goes AFTER its channel (host RM refuses freeing a context
                    // DMA a live channel names as its error context).
                    if let Some(n) = t.notifier {
                        me.release_notifier(n);
                    }
                    me.engine_live(t.engine, false);
                    // ★ The per-token hardware ledger (`run_fast_guest.sh` gates on it): every ring
                    // of a Passthrough token IS a host doorbell, so `emulated` is only the rings
                    // that failed to reach the host — never a CPU executor.
                    let (rung, reached) = me.take_ledger(t.idx);
                    eprintln!(
                        "kf3: DOORBELL-LEDGER tok={:#010x} route=passthrough rung={rung} emulated={} forwarded={reached} host={:#x}",
                        t.idx,
                        rung - reached.min(rung),
                        t.chan.token
                    );
                    line.push(format!("passthrough {c:#x}:{h:#x} token {:#x} host {:#x} objects={} {}", t.idx, t.chan.token, t.objects.len(), if r.is_ok() { "freed" } else { "FREE REFUSED" }));
                }
                if let Some(h) = obj {
                    let r = me.rm.free(h);
                    line.push(format!("engine object host {h:#x} {}", if r.is_ok() { "freed" } else { "FREE REFUSED" }));
                }
                // ⊘ A host free that refuses leaks a host object; the guest's object is gone
                // either way, so the guest is answered OK and the leak is named here.
                Ok(format!("{client:#x}:{object:#x}: {}", line.join("; ")))
            }),
        )
    }

    fn slot(&self, ht: u32) -> Option<Arc<Mutex<Slot>>> {
        self.slots.read().ok()?.get(&ht).cloned()
    }

    /// ★ Birth AT ALLOCATION (§7). A guest-KERNEL copy-engine channel is Translated; a guest USER
    /// channel on a copy engine or GR0 is Passthrough (P5b). Everything the drainer can check
    /// without the host is checked HERE (a refusal is immediate); the host verbs are an act.
    fn birth(&self, a: ChannelAlloc) -> ChanAnswer {
        let engine = a.engine_type.unwrap_or(0);
        let refuse = |status: u32, why: String| {
            eprintln!("kf3: chan {:#x}:{:#x} birth REFUSED: {why} (decl {a:x?})", a.client, a.handle);
            ChanAnswer::Refused { status, why }
        };
        let passthrough = !a.kernel_client;
        if a.kernel_client && !is_copy_engine(engine) {
            eprintln!(
                "kf3: chan {:#x}:{:#x} class={:#x} engine={engine:#x} kernel=true vaspace={:x?} chid={:x?} — a KERNEL non-CE channel: not born (kernel GR is P7)",
                a.client, a.handle, a.class, a.vaspace, a.chid
            );
            return ChanAnswer::NotOurs;
        }
        if passthrough && !is_copy_engine(engine) && engine != kf_abi::submit::ENGINE_TYPE_GRAPHICS {
            return refuse(NV_ERR_NOT_SUPPORTED, format!("user channel on engine type {engine:#x}: only a copy engine or GR0 has a passthrough twin"));
        }
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
        if passthrough {
            // ★ The guest's USERD, adopted AT CREATION (RM zeroes it — `rm_takes_a_guest_userd`):
            // a store slice, or guest RAM through the mirror's RAM object.
            let userd = match a.userd {
                Some(kf_arch::UserdMem::Framebuffer { base, .. }) => kf_chan::passthrough::UserdAt::Store { store: self.store, off: base },
                Some(kf_arch::UserdMem::Sysmem { base, .. }) => {
                    let (Some(ram), Some((_, off))) = (mirror.ram_obj, self.ram.file_range(base, 0x200)) else {
                        return refuse(NV_ERR_NOT_SUPPORTED, format!("sysmem USERD at {base:#x}: no guest-RAM object or memfd offset"));
                    };
                    kf_chan::passthrough::UserdAt::Ram { ram, off }
                }
                other => return refuse(NV_ERR_NOT_SUPPORTED, format!("USERD not declared as a physical descriptor ({other:?})")),
            };
            // ★ P5c: where the guest's error notifier record is, as an object + offset the host
            // can name — so the twin's RC record is written there by the HOST (its GSP), natively.
            let err_at: Option<(u32, u64, kf_arch::UserdMem)> = match a.error_notifier {
                Some(kf_arch::fault::ErrorNotifier::Sysmem { gpa }) => match (mirror.ram_obj, self.ram.file_range(gpa, 16)) {
                    (Some(ram), Some((_, off))) => Some((ram, off, kf_arch::UserdMem::Sysmem { base: gpa, size: 16 })),
                    _ => {
                        self.rc_unarmed.fetch_add(1, Ordering::Relaxed);
                        eprintln!("kf3: chan {:#x}:{:#x} RC-UNARMED: sysmem notifier @{gpa:#x} has no guest-RAM object/offset", a.client, a.handle);
                        None
                    }
                },
                Some(kf_arch::fault::ErrorNotifier::Framebuffer { off }) => {
                    Some((self.store, off, kf_arch::UserdMem::Framebuffer { base: off, size: 16 }))
                }
                Some(kf_arch::fault::ErrorNotifier::Unreachable) => {
                    self.rc_unarmed.fetch_add(1, Ordering::Relaxed);
                    eprintln!("kf3: chan {:#x}:{:#x} RC-UNARMED: the declared notifier is in an aperture we cannot name", a.client, a.handle);
                    None
                }
                None => None,
            };
            let g0 = kf_chan::passthrough::GuestChannel { gpfifo_va: a.gpfifo_va, entries: a.entries.max(1), userd, engine, err_ctx: 0 };
            let space = mirror.space;
            // ★ P5c: counted NOW (on the drainer, in statement order), so a VA-space free that
            // follows can never recycle the space under a birth still queued.
            let live = mirror.live.clone();
            live.fetch_add(1, Ordering::AcqRel);
            return self.defer(
                "birth passthrough",
                Box::new(move |me: &ChanPlane| {
                    let notifier = err_at.and_then(|(obj, off, at)| me.arm_notifier(a.client, a.handle, obj, off, at));
                    let g = kf_chan::passthrough::GuestChannel { err_ctx: notifier.as_ref().map_or(0, |n| n.ctx), ..g0 };
                    let chan = match kf_chan::passthrough::birth_twin(me.rm, space, g) {
                        Ok(c) => c,
                        Err(e) => {
                            live.fetch_sub(1, Ordering::AcqRel);
                            if let Some(n) = notifier {
                                me.release_notifier(n);
                            }
                            return Err((NV_ERR_INSUFFICIENT_RESOURCES, e));
                        }
                    };
                    let owner = if a.kernel_client { Owner::Kernel } else { Owner::User };
                    let alloc = me
                        .caps
                        .lock()
                        .map_err(|_| "caps poisoned".to_string())
                        .and_then(|mut c| me.plane.allocate_channel(&mut c, idx, Route::Passthrough, chan.token, owner).map_err(|e| format!("{e:?}")));
                    if let Err(e) = alloc {
                        let _ = me.rm.free_channel(chan);
                        live.fetch_sub(1, Ordering::AcqRel);
                        if let Some(n) = notifier {
                            me.release_notifier(n);
                        }
                        return Err((NV_ERR_INSUFFICIENT_RESOURCES, format!("token {idx:#x}: {e}")));
                    }
                    let rc = if notifier.is_some() { "armed" } else { "none" };
                    if let Ok(mut m) = me.pt.lock() {
                        m.insert((a.client, a.handle), PtChan { chan, idx, tsg: a.tsg, parent: a.parent, engine, objects: HashMap::new(), live, notifier });
                    }
                    me.pt_births.fetch_add(1, Ordering::Relaxed);
                    me.engine_live(engine, true);
                    let _ = me.take_ledger(idx);
                    Ok(format!(
                        "chan {:#x}:{:#x} BORN Passthrough: token {idx:#x} -> host {:#x} in {key:?} gpfifo={:#x}x{} userd={userd:?} engine={engine:#x} declared_kernel_pid={} rc={rc}",
                        a.client, a.handle, chan.token, a.gpfifo_va, g.entries, a.declared_kernel_pid
                    ))
                }),
            );
        }
        let entries = a.entries.max(1);
        mirror.live.fetch_add(1, Ordering::AcqRel);
        self.defer(
            "birth translated",
            Box::new(move |me: &ChanPlane| {
                let fail = |e: (u32, String)| {
                    mirror.live.fetch_sub(1, Ordering::AcqRel);
                    e
                };
                let userd = me.userd_view(a.userd).map_err(|e| fail((NV_ERR_NOT_SUPPORTED, e)))?;
                let host = HostRing::on_engine(me.rm, mirror.space, me.host_ce).map_err(|e| fail((NV_ERR_INSUFFICIENT_RESOURCES, format!("host ring: {e}"))))?;
                let ht = host.channel().token;
                let chan = TranslatedChannel::new(TranslatedRing::new(a.gpfifo_va, entries, 0), host, idx);
                let alloc = me
                    .caps
                    .lock()
                    .map_err(|_| "caps poisoned".to_string())
                    .and_then(|mut c| me.plane.allocate_channel(&mut c, idx, Route::Translated, ht, Owner::Kernel).map_err(|e| format!("{e:?}")));
                if let Err(e) = alloc {
                    let _ = me.rm.free_channel(chan.host().channel());
                    return Err(fail((NV_ERR_INSUFFICIENT_RESOURCES, format!("token {idx:#x}: {e}"))));
                }
                let slot = Slot { chan, key, mirror, userd, guest_idx: idx, scheduled: false, dead: None, serves: 0, last_put: None };
                if let Ok(mut s) = me.slots.write() {
                    s.insert(ht, Arc::new(Mutex::new(slot)));
                }
                if let Ok(mut m) = me.by_obj.lock() {
                    m.insert((a.client, a.handle), ht);
                }
                me.births.fetch_add(1, Ordering::Relaxed);
                Ok(format!(
                    "chan {:#x}:{:#x} BORN Translated: token {idx:#x} -> host {ht:#x} in {key:?} gpfifo={:#x}x{entries} userd={:?} engine={engine:#x}",
                    a.client, a.handle, a.gpfifo_va, a.userd
                ))
            }),
        )
    }

    /// ★ P5c (act thread): the twin's host error context over the guest's notifier record, its
    /// RC event on the plane's RC fd, and a read view of the record. `None` (named, counted) when any
    /// step refuses — the twin is then born without one, and its faults stay silent.
    fn arm_notifier(&self, client: u32, handle: u32, obj: u32, off: u64, at: kf_arch::UserdMem) -> Option<PtNotifier> {
        let refuse = |why: String| {
            self.rc_unarmed.fetch_add(1, Ordering::Relaxed);
            eprintln!("kf3: chan {client:#x}:{handle:#x} RC-UNARMED: {why}");
        };
        let ctx = match self.rm.alloc_context_dma(obj, off, 16) {
            Ok(c) => c,
            Err(e) => {
                refuse(format!("context DMA over object {obj:#x}+{off:#x}: {e:?}"));
                return None;
            }
        };
        if let Err(e) = self.rm.alloc_os_event(ctx, 0, false, &self.rc_ev) {
            let _ = self.rm.free(ctx);
            refuse(format!("RC event on context DMA {ctx:#x}: {e:?}"));
            return None;
        }
        match self.userd_view(Some(at)) {
            Ok(view) => {
                self.rc_armed.fetch_add(1, Ordering::Relaxed);
                let mut n = PtNotifier { ctx, view, reported: false, at_arm: [0; 4] };
                n.at_arm = n.words().unwrap_or([0; 4]);
                Some(n)
            }
            Err(e) => {
                let _ = self.rm.free(ctx);
                refuse(format!("notifier read view: {e}"));
                None
            }
        }
    }

    /// Free a twin's error context (its event goes with it) and release its view.
    fn release_notifier(&self, n: PtNotifier) {
        let _ = self.rm.free(n.ctx);
        if let UserdView::Store { cookie, .. } = &n.view {
            let _ = self.rm.release_cpu_view(kf_host::CpuViewRelease { h_memory: self.store, p_linear_address: *cookie });
        }
    }

    /// ★ P5c — a WORKER, on the RC fd's readiness: which twins' notifier records did the host just
    /// write? Each newly-written record (non-zero `status`, `kernel_rc_notification.c:326-338`
    /// writes `0xffff` last) becomes ONE queued [`RcEvent`] for the drainer. ⊘ Reads 16 bytes per
    /// armed twin; writes nothing. Returns how many were queued.
    pub fn rc_scan(&self) -> usize {
        self.rc_wakes.fetch_add(1, Ordering::Relaxed);
        let mut found = Vec::new();
        if let Ok(mut m) = self.pt.lock() {
            for t in m.values_mut() {
                let Some(n) = t.notifier.as_mut() else { continue };
                if n.reported {
                    continue;
                }
                // `NvNotification {timeStamp:8, info32:4, info16:2, status:2}`: status is the high
                // half of word 3 and is written last.
                let Some(w) = n.words() else { continue };
                if w != n.at_arm && (w[3] >> 16) != 0 {
                    n.reported = true;
                    found.push(RcEvent { chid: t.idx, engine: t.engine, except_type: w[2], host_token: t.chan.token });
                }
            }
        }
        let k = found.len();
        if k > 0 {
            self.rc_seen.fetch_add(k as u64, Ordering::Relaxed);
            for e in &found {
                eprintln!(
                    "kf3: RC host twin {:#x} (guest chid {:#x}, engine {:#x}) wrote its notifier: except_type={:#x} (Xid {}) — forwarding RC_TRIGGERED",
                    e.host_token, e.chid, e.engine, e.except_type, e.except_type
                );
            }
            if let Ok(mut q) = self.rc_queue.lock() {
                q.extend(found);
            }
            let _ = self.release.signal();
        }
        k
    }

    /// ★ P5c (drainer): the RC events waiting to be posted.
    pub fn take_rc(&self) -> Vec<RcEvent> {
        self.rc_queue.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
    }

    /// ★ P5c (drainer): put back events the GSP queue could not take now (retried next pass).
    pub fn requeue_rc(&self, back: Vec<RcEvent>) {
        if let Ok(mut q) = self.rc_queue.lock() {
            let mut back = back;
            back.append(&mut q);
            *q = back;
        }
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
        // §5.2: free waits out BUSY. The slot lock is held, so no worker is inside the pump — but
        // one may still hold the TOKEN for a moment after it: retry on the act thread (never a
        // lock the drainer holds), bounded, and name a token that stays stranded.
        let mut freed = false;
        for _ in 0..200 {
            if let Ok(mut c) = self.caps.lock()
                && self.plane.free_channel(&mut c, g.guest_idx)
            {
                freed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        if !freed {
            eprintln!("kf3: chan token {:#x} (host {ht:#x}) STRANDED: still BUSY after 200 ms", g.guest_idx);
        }
        let _ = self.rm.free_channel(g.chan.host().channel());
        g.mirror.live.fetch_sub(1, Ordering::AcqRel);
        if let UserdView::Store { cookie, .. } = &g.userd {
            let _ = self.rm.release_cpu_view(kf_host::CpuViewRelease { h_memory: self.store, p_linear_address: *cookie });
        }
        eprintln!(
            "kf3: DOORBELL-LEDGER tok={:#010x} route=translated emulated={} forwarded={} host={ht:#x}",
            g.guest_idx,
            u64::from(g.dead.is_some() && g.chan.counts().0 == 0),
            g.chan.counts().0
        );
        eprintln!(
            "kf3: chan token {:#x} (host {ht:#x}) RETIRED, forwarded={} serves={} last_put={:?} gp_get={:?}",
            g.guest_idx,
            g.chan.counts().0,
            g.serves,
            g.last_put,
            g.chan.last_gp_get()
        );
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
        g.serves += 1;
        if g.dead.is_some() || !g.scheduled || self.stop.load(Ordering::Acquire) {
            return false;
        }
        g.last_put = g.userd.load(kf_abi::submit::USERD_GP_PUT).ok();
        // Diagnostic (first 3 serves that find nothing to do): which words of the USERD page are
        // non-zero — a GP_PUT that landed elsewhere in the page shows up here. Worker thread only.
        if g.last_put == Some(0) && g.serves <= 3 {
            if let UserdView::Store { region, at, .. } = &g.userd {
                let nz: Vec<String> = (0..0x1000u64)
                    .step_by(4)
                    .filter_map(|o| region.load_u32(HostOffset::new(o)).ok().filter(|v| *v != 0).map(|v| format!("+{o:#x}={v:#x}")))
                    .take(16)
                    .collect();
                eprintln!("kf3: chan token {:#x}: GP_PUT=0 at USERD+{at:#x}+0x8c; non-zero words in its page: [{}]", g.guest_idx, nz.join(" "));
            }
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
                Some(TokenCount {
                    token: g.guest_idx,
                    forwarded: fwd,
                    submissions: subs,
                    gp_get: g.chan.last_gp_get(),
                    dead: g.dead.clone(),
                    serves: g.serves,
                    last_put: g.last_put,
                })
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
