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
use kf_chan::host::{ChanError, GuestUserd, HostRing, Publisher, Split, TranslatedChannel};
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
/// `NV_ERR_INVALID_ARGUMENT`.
const NV_ERR_INVALID_ARGUMENT: u32 = 0x1F;
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
    /// ★ v3-promote: the device it hangs off (== `parent` outside a TSG).
    device: u32,
    /// The host engine (= the guest's `engineType`).
    engine: u32,
    /// Engine objects on the twin: guest handle → (host handle, its class kind).
    objects: HashMap<u32, (u32, kf_chip::classes::Kind)>,
    /// ★ v3-promote: the guest's `GPU_PROMOTE_CTX` / `GPU_EVICT_CTX` statements for this channel,
    /// satisfied by the twin (never forwarded).
    ctx: CtxBind,
    /// ★ P5c: the mirror's live-channel count (released at free).
    live: Arc<AtomicU64>,
    /// ★ P5c: the guest's error notifier, as the twin's host error context — `None` when the
    /// guest declared none (or it could not be armed, named at birth).
    notifier: Option<PtNotifier>,
    /// ★ v3-chanctl: the guest STOPPED the channel (host: disabled + off the runlist) — its next
    /// `GPFIFO_SCHEDULE(enable)` re-enables the twin first.
    stopped: bool,
    /// ★ v3-chanctl: the guest DISABLED it (`DISABLE_CHANNELS`) — only `bDisable=FALSE` undoes it.
    disabled: bool,
    /// ★ v3-video: the twin's host VA space (the mirror of the guest's).
    space: kf_host::VaSpace,
    /// ★ v3-video: the guest's falcon context buffer `(VA, size)` from its falcon promote — the VA
    /// the host's own falcon context is steered onto (see `ChanPlane::engine_object`).
    falcon_ctx: Option<(u64, u64)>,
}

/// ★★★ v3-promote — **the guest's context-buffer statements, satisfied by the twin** (owner
/// ruling 2026-09-25). The guest's CPU-RM owns and allocates its GR context buffers
/// (`bClientRmAllocatedCtxBuffer` is TRUE on every GSP client, `ogkm-580: gpu_registry.c:153-156`)
/// and declares them to physical RM with `GPU_PROMOTE_CTX`: once per GR object for the
/// PA-initialize half (`kgrobjPromoteContext`, `kernel_graphics_object.c:51-159`) and, in a
/// UVM-owned space, once more at `UVM_REGISTER_CHANNEL` for the VAs (`nvGpuOpsBindChannelResources`,
/// `nv_gpu_ops.c:10854-10904`). Physical RM is us, and the channel runs as a host twin whose OWN
/// context host RM allocated, initialised from its golden image and promoted when we allocated the
/// twin's engine object — so the statement is answered by recording it, never by forwarding it and
/// never by touching the guest's buffers.
///
/// ⊘ **What the guest's CPU-RM reads back from those buffers: nothing** (searched: every
/// `memmgrMemDescBeginTransfer`/`memmgrMemRead` in `kernel/gpu/gr/`, `kernel_channel.c` and
/// `nv_gpu_ops.c`). On `NV_OK` it only flips its own `bKGr*CtxBufferInitialized` flags
/// (`kernel_graphics_object.c:136-151`, `kgrctxMarkCtxBufferInitialized`,
/// `kernel_graphics_context.c:1949-2000`) and — for the UVM bind — `bIsContextBound`
/// (`nv_gpu_ops.c:10901-10904`), which is what its own `kchannelIsSchedulable`
/// (`kernel_channel.c:2200-2206`, called at `:3105` BEFORE the schedule RPC) tests. The one GR
/// buffer CPU-RM does read is the FECS event buffer, and only with a ctxsw-log consumer; CPU-RM
/// itself fills it with `0xde` before enabling (`fecs_event_list.c:1545-1547`) and only reads
/// `magic_lo` against that fill (`:1046-1099`). ⇒ the "stub" is the buffer exactly as the guest
/// left it: we write no byte.
#[derive(Debug, Default, Clone, Copy)]
struct CtxBind {
    /// Buffer ids the guest asked to be initialised (bitmask of `bufferId`).
    initialized: u32,
    /// Buffer ids whose VA the guest promoted (the UVM bind).
    va_bound: u32,
    /// `bIsContextBound`'s mirror: a VA promote succeeded and no evict followed.
    bound: bool,
    /// Promotes / evicts answered.
    promotes: u32,
    evicts: u32,
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
    /// ★ v3-chanctl: after an `NV_OK` to `STOP_CHANNEL` the guest's OWN CPU-RM writes this record
    /// (`ROBUST_CHANNEL_PREEMPTIVE_REMOVAL`, `kchannelNotifyRc_HAL`, `kernel_channel.c:1979`) — that
    /// write is the guest's, not the host's, and must never come back as an `RC_TRIGGERED`.
    guest_stop_write: bool,
}

/// `ROBUST_CHANNEL_PREEMPTIVE_REMOVAL` (`ogkm-580: nverror.h:61`).
const ROBUST_CHANNEL_PREEMPTIVE_REMOVAL: u32 = 45;

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

/// ★ Q7: which unforgeable fact made a channel the guest kernel's (for the birth log).
fn kernel_by(a: &ChannelAlloc) -> String {
    let stamp = match a.privilege {
        Some(p) if p.is_kernel() => format!("PRIVILEGE=KERNEL{}", if p.uvm_owned { "+UVM_OWNED" } else { "" }),
        Some(p) => format!("privilege={}{}", p.level, if p.uvm_owned { "+uvm_owned" } else { "" }),
        None => "privilege=undecoded".into(),
    };
    let internal = if kf_rm::chanlink::is_rm_internal_client(a.client) { " rm-internal" } else { "" };
    format!("{stamp}{internal}")
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

/// ★ P6 (Q3) — CPU views WE arm over the store slices a Translated channel's GPFIFO or pushbuffer
/// lives in (UVM puts its GPFIFO in vidmem by default: `ogkm-580 uvm_channel.c:3386-3391`). The
/// same verb as the USERD view (`NV_ESC_RM_MAP_MEMORY` of the store on the host's BAR1): OUR
/// mapping of OUR object, located through OUR placement rows. ⊘ Never a copy of a guest table, and
/// only method words and GP entries are read through it — never data.
struct StoreViews {
    views: Vec<StoreSpan>,
    /// Views armed over the channel's life (the log's proof of how often it paid for one).
    armed: u64,
}

struct StoreSpan {
    off: u64,
    len: u64,
    region: VolatileRegion,
    _node: kf_linux_raw::CharDevice,
    cookie: u64,
}

/// One view's span: a 64 KiB-aligned slice (a GPFIFO of 1024 entries is 8 KiB).
const VIEW_BYTES: u64 = 64 << 10;
/// Views kept per channel; the oldest is released beyond this (host BAR1 aperture is finite).
const VIEWS_MAX: usize = 8;

impl StoreViews {
    const fn new() -> Self {
        StoreViews { views: Vec::new(), armed: 0 }
    }

    fn view_for(&mut self, rm: &HostRm, store: u32, fb_len: u64, at: u64) -> Result<usize, String> {
        if let Some(i) = self.views.iter().position(|v| at >= v.off && at < v.off + v.len) {
            return Ok(i);
        }
        let off = at & !(VIEW_BYTES - 1);
        let len = VIEW_BYTES.min(fb_len.saturating_sub(off));
        if len == 0 {
            return Err(format!("store offset {at:#x} is past the store ({fb_len:#x})"));
        }
        if self.views.len() >= VIEWS_MAX {
            let old = self.views.remove(0);
            let _ = rm.release_cpu_view(kf_host::CpuViewRelease { h_memory: store, p_linear_address: old.cookie });
        }
        let (node, cookie) = rm
            .arm_cpu_view(MapNode::Gpu, store, off, len, ViewAccess::ReadWrite)
            .map_err(|e| format!("view of store {off:#x}+{len:#x}: {e:?}"))?;
        let region = VolatileRegion::map(Backing::DeviceFile { fd: node.as_fd() }, len, CachePolicy::Uncached, HostPageSize::query())
            .map_err(|e| format!("view mmap: {e:?}"))?;
        self.armed += 1;
        self.views.push(StoreSpan { off, len, region, _node: node, cookie });
        Ok(self.views.len() - 1)
    }

    fn read(&mut self, rm: &HostRm, store: u32, fb_len: u64, off: u64, out: &mut [u8]) -> Result<(), String> {
        let len = out.len() as u64;
        let mut done = 0u64;
        while done < len {
            let at = off + done;
            let i = self.view_for(rm, store, fb_len, at)?;
            let v = &self.views[i];
            let n = (v.off + v.len - at).min(len - done);
            v.region
                .copy_out(HostOffset::new(at - v.off), &mut out[done as usize..(done + n) as usize])
                .map_err(|e| format!("store read {at:#x}: {e:?}"))?;
            done += n;
        }
        Ok(())
    }

    fn release_all(&mut self, rm: &HostRm, store: u32) {
        for v in self.views.drain(..) {
            let _ = rm.release_cpu_view(kf_host::CpuViewRelease { h_memory: store, p_linear_address: v.cookie });
        }
    }
}

/// The guest's memory at a guest VA of the channel's space, through OUR placements.
struct Mem<'a> {
    mirror: &'a Mirror,
    ram: &'a RamMap,
    rm: &'a HostRm,
    store: u32,
    views: &'a mut StoreViews,
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
            let n = avail.min(len - done);
            if !ram {
                // ★ P6 (Q3): a vidmem GPFIFO / pushbuffer (UVM's default GPFIFO) is read through a
                // CPU view WE arm over the store slice our own row placed there.
                let dst = &mut out[done as usize..(done + n) as usize];
                let t0 = crate::prof::on().then(crate::prof::now_ns);
                self.views.read(self.rm, self.store, self.mirror.fb_len, off, dst).map_err(|e| format!("{at_va:#x}: {e}"))?;
                crate::prof::VIEW_READS.fetch_add(1, Ordering::Relaxed);
                crate::prof::VIEW_READ_BYTES.fetch_add(n, Ordering::Relaxed);
                if let Some(t0) = t0 {
                    crate::prof::VIEW_READ_NS.fetch_add(crate::prof::now_ns().saturating_sub(t0), Ordering::Relaxed);
                }
                done += n;
                continue;
            }
            let (mem, at) = self.ram.at_file_offset(off, n).ok_or_else(|| format!("{at_va:#x}: guest-RAM offset {off:#x} unregistered"))?;
            let dst = &mut out[done as usize..(done + n) as usize];
            if !mem.read_into(at, dst) {
                return Err(format!("{at_va:#x}: guest-RAM read"));
            }
            crate::prof::RAM_READ_BYTES.fetch_add(n, Ordering::Relaxed);
            done += n;
        }
        Ok(())
    }
}

/// ★ P6 — the `MEM_OP` split through the VA-manager thread (`THE_TRANSLATED_PLANE.md` §5, §24.2).
/// The channel's work before the invalidate has COMPLETED (the runner fenced it); the first ask
/// queues a walk + reconcile of the named root on the VA thread and answers [`Split::Pending`]; the
/// VA thread's completion rings the channel's token, and the next pump's ask collects the outcome.
/// ⊘ Never a wait on the worker, never a CPU read of a guest table.
struct VaSplit<'a> {
    inbox: &'a crate::mem::Inbox,
    token: u32,
    ticket: &'a mut Option<u64>,
    requested: &'a mut u64,
}
impl Publisher for VaSplit<'_> {
    fn invalidated(&mut self, pdb: Option<u64>) -> Result<Split, String> {
        let Some(t) = *self.ticket else {
            *self.ticket = Some(self.inbox.request_split(self.token, pdb));
            *self.requested += 1;
            return Ok(Split::Pending);
        };
        match self.inbox.split_result(t) {
            None => Ok(Split::Pending),
            Some(r) => {
                *self.ticket = None;
                r.map(|()| Split::Done).map_err(|e| format!("split walk (pdb {pdb:x?}): {e}"))
            }
        }
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
    /// ★ P6: the channel's group (a TSG `GPFIFO_SCHEDULE` names it).
    tsg: Option<u32>,
    /// ★ P6: the split the channel is suspended on, once asked.
    split: Option<u64>,
    /// ★ P6: splits requested over the channel's life.
    splits: u64,
    /// ★ P6 (Q3): store views over a vidmem GPFIFO / pushbuffer.
    views: StoreViews,
    /// ★ P6: the Q7 privilege stamp it was born under (for the log).
    privilege: Option<kf_abi::notifier::ChannelPrivilege>,
    /// ★ v3-chanctl: the guest STOPPED it — our host ring is disabled and off its runlist until
    /// the guest schedules it again.
    stopped: bool,
    /// ★ v3-chanctl: the guest DISABLED it (`DISABLE_CHANNELS`); the pump fetches nothing and our
    /// host ring is disabled until `bDisable=FALSE`.
    disabled: bool,
}

/// ★ v3-promote: where a guest channel hangs in the guest's object tree — every handle whose free
/// takes it (a GSP-client guest frees a subtree with ONE `GSP_RM_FREE` naming its root).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChanScope {
    tsg: Option<u32>,
    parent: u32,
    device: u32,
}

impl ChanScope {
    /// Does freeing `(client, object)` free the channel `key` (`(hClient, hChannel)`)?
    fn freed_by(self, key: (u32, u32), client: u32, object: u32) -> bool {
        key.0 == client && (object == client || key.1 == object || self.tsg == Some(object) || self.parent == object || self.device == object)
    }
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
    /// ★ P6: the VA thread's inbox — a Translated channel's `MEM_OP` split goes through it.
    inbox: std::sync::Arc<crate::mem::Inbox>,
    /// The session's ONE completion fd + in-flight set (`kf_chan::completions`).
    pub completions: Completions,
    caps: Mutex<VmCaps>,
    /// Host token → channel.
    slots: RwLock<HashMap<u32, Arc<Mutex<Slot>>>>,
    /// `(hClient, hObject)` → host token.
    by_obj: Mutex<HashMap<(u32, u32), u32>>,
    /// ★ v3-promote: a Translated channel's group / parent / device (what a guest free can name).
    scopes: Mutex<HashMap<(u32, u32), ChanScope>>,
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
    /// ★ w827: guest debugger session `(hClient, hDebugger)` → its host twin: `(guest device, host
    /// session, host GR object it is bound to)`.
    dbg: Mutex<HashMap<(u32, u32), (u32, u32, u32)>>,
    /// ★ w827: guest Devices `(hClient, hDevice)` whose CUDA limit is on, and whether OUR host
    /// device's is — the host limit is on iff any guest Device's is (the guest kernel sends only
    /// per-Device edges, so the union is exactly the guest's own state).
    cuda_limit: Mutex<(std::collections::BTreeSet<(u32, u32)>, bool)>,
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
    /// ★ w827: every act's time, summed (the act thread's busy time).
    pub act_total_us: AtomicU64,
    /// Passthrough twins born.
    pub pt_births: AtomicU64,
    /// ★ v3-video: host NVENC session slots held per guest client (acquired on OUR host client;
    /// released with the guest's release or its client's free).
    enc_sessions: Mutex<HashMap<u32, u32>>,
    /// ★ v3-video: guest TSG `(hClient, hTsg)` → `(host group, live members)` — the guest's TSG
    /// membership mirrored, so its channels share ONE host GR context as on hardware. Touched by
    /// acts only (serialised on the act thread).
    groups: Mutex<HashMap<(u32, u32), (u32, u32)>>,
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
        inbox: std::sync::Arc<crate::mem::Inbox>,
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
        // ★ The video engines the served table advertises (it lists only the host's own): one
        // non-stall event each, announced on the vector the guest was told
        // (`authored::engine_notification_rows`) — its `gkflcnServiceNotificationInterrupt` wakes
        // the guest's NVENC/NVDEC OS events from it.
        for i in 0..kf_abi::submit::NVENC_SIZE {
            let kind = kf_rm::authored::EngineKind::VideoEncode(i);
            if let (Some(et), Some(_)) = (kf_abi::submit::engine_type_nvenc(i), kf_rm::authored::non_stall_vector_for(intr_table, kind)) {
                kinds.push((kind, kf_host::event::notifier_nvenc(i), et));
            }
        }
        for i in 0..kf_abi::submit::NVDEC_SIZE {
            let kind = kf_rm::authored::EngineKind::VideoDecode(i);
            if let (Some(et), Some(_)) = (kf_abi::submit::engine_type_nvdec(i), kf_rm::authored::non_stall_vector_for(intr_table, kind)) {
                kinds.push((kind, kf_host::event::notifier_nvdec(i), et));
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
            inbox,
            completions,
            // Declared caps: channels are the only twin this plane mints.
            caps: Mutex::new(VmCaps::from_declared(64, 64, 64, 64)),
            slots: RwLock::new(HashMap::new()),
            by_obj: Mutex::new(HashMap::new()),
            scopes: Mutex::new(HashMap::new()),
            wake,
            contended: AtomicU64::new(0),
            poisoned: AtomicU64::new(0),
            births: AtomicU64::new(0),
            host_ce,
            stop: AtomicBool::new(false),
            family,
            pt: Mutex::new(HashMap::new()),
            pt_objs: Mutex::new(HashMap::new()),
            dbg: Mutex::new(HashMap::new()),
            cuda_limit: Mutex::new((std::collections::BTreeSet::new(), false)),
            acts: Mutex::new(None),
            release,
            engines,
            acts_run: AtomicU64::new(0),
            acts_refused: AtomicU64::new(0),
            act_worst_us: AtomicU64::new(0),
            act_total_us: AtomicU64::new(0),
            pt_births: AtomicU64::new(0),
            groups: Mutex::new(HashMap::new()),
            enc_sessions: Mutex::new(HashMap::new()),
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
                    self.act_total_us.fetch_add(us, Ordering::Relaxed);
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
                // ★ P6: a TSG schedule (`0xa06c0101`) over Translated members (nvidia-uvm's
                // channels live in groups): every member's slot, as its own schedule.
                let members: Vec<u32> = self
                    .by_obj
                    .lock()
                    .map(|m| m.iter().filter(|(k, _)| k.0 == client).map(|(_, v)| *v).collect::<Vec<u32>>())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|ht| self.slot(*ht).is_some_and(|s| s.lock().is_ok_and(|g| g.tsg == Some(object))))
                    .collect();
                if !members.is_empty() {
                    for ht in members {
                        if let ChanAnswer::Refused { status, why } = self.schedule_translated(client, object, ht, enable) {
                            return ChanAnswer::Refused { status, why };
                        }
                    }
                    return ChanAnswer::Done;
                }
                // ★ P5b: a user channel (or its group) — the twin's own schedule, as an act.
                let twins: Vec<((u32, u32), kf_host::Channel)> = self
                    .pt
                    .lock()
                    .map(|m| {
                        m.iter()
                            .filter(|(k, v)| k.0 == client && (k.1 == object || v.tsg == Some(object)))
                            .map(|(k, v)| (*k, v.chan))
                            .collect()
                    })
                    .unwrap_or_default();
                if twins.is_empty() {
                    return ChanAnswer::NotOurs;
                }
                self.defer(
                    "schedule",
                    Box::new(move |me: &ChanPlane| {
                        let mut restarted = 0;
                        // ★ v3-video: twins of one guest TSG share ONE host group — schedule it once.
                        let mut groups_done = std::collections::HashSet::new();
                        for (k, c) in &twins {
                            // ★ v3-chanctl: a STOPPED twin is re-enabled before it is scheduled
                            // (STOP disabled it; "it has to be scheduled, bound and enabled again",
                            // `ctrla06fgpfifo.h:219-221`) — unless the guest ALSO disabled it with
                            // DISABLE_CHANNELS, which only its own `bDisable=FALSE` undoes.
                            let (stopped, disabled) =
                                me.pt.lock().ok().and_then(|m| m.get(k).map(|v| (v.stopped, v.disabled))).unwrap_or((false, false));
                            if enable && stopped {
                                if !disabled {
                                    me.rm
                                        .disable_channels(&[*c], false, false, false)
                                        .map_err(|e| (NV_ERR_INVALID_STATE, format!("host {:#x} re-enable after STOP: {e:?}", c.token)))?;
                                }
                                restarted += 1;
                                if let Ok(mut m) = me.pt.lock()
                                    && let Some(v) = m.get_mut(k)
                                {
                                    v.stopped = false;
                                    if let Some(n) = v.notifier.as_mut() {
                                        // The record as the guest left it is the new baseline.
                                        n.guest_stop_write = false;
                                        n.at_arm = n.words().unwrap_or(n.at_arm);
                                    }
                                }
                            }
                            if groups_done.insert(c.tsg) {
                                me.rm.schedule_enable(*c, enable).map_err(|e| (NV_ERR_INVALID_STATE, format!("host {:#x}: {e:?}", c.token)))?;
                            }
                        }
                        Ok(format!("{client:#x}:{object:#x} GPFIFO_SCHEDULE enable={enable} on {} twin(s) ({restarted} restarted after STOP)", twins.len()))
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
            ChanStatement::EngineObject { client, parent, handle, class, copy_engine } => self.engine_object(client, parent, handle, class, copy_engine),
            ChanStatement::Debugger { client, parent, handle, app_client, obj3d } => self.debugger(client, parent, handle, app_client, obj3d),
            ChanStatement::DebuggerExceptionMask { client, object, mask } => {
                let Some(h) = self.dbg.lock().ok().and_then(|m| m.get(&(client, object)).map(|v| v.1)) else {
                    return ChanAnswer::NotOurs;
                };
                self.defer(
                    "debugger exception mask",
                    Box::new(move |me: &ChanPlane| {
                        me.rm.debugger_set_exception_mask(h, mask).map_err(|e| (NV_ERR_INVALID_STATE, format!("SET_EXCEPTION_MASK {mask:#x} on host session {h:#x}: {e:?}")))?;
                        Ok(format!("{client:#x}:{object:#x} SET_EXCEPTION_MASK {mask:#x} on host session {h:#x}"))
                    }),
                )
            }
            ChanStatement::CtxswPreemption { client, channel, flags, gfxp, cilp } => {
                // The guest's channel, or every twin of the guest's TSG — GR twins only: the mode
                // is a GR context property (a CE twin in the group has none).
                let twins: Vec<kf_host::Channel> = self
                    .pt
                    .lock()
                    .map(|m| {
                        m.iter()
                            .filter(|(k, v)| k.0 == client && (k.1 == channel || v.tsg == Some(channel)) && v.engine == kf_abi::submit::ENGINE_TYPE_GRAPHICS)
                            .map(|(_, v)| v.chan)
                            .collect()
                    })
                    .unwrap_or_default();
                if twins.is_empty() {
                    return ChanAnswer::NotOurs;
                }
                self.defer(
                    "ctxsw preemption",
                    Box::new(move |me: &ChanPlane| {
                        for c in &twins {
                            me.rm
                                .set_ctxsw_preemption_mode(*c, flags, gfxp, cilp)
                                .map_err(|e| (NV_ERR_NOT_SUPPORTED, format!("twin host {:#x} SET_CTXSW_PREEMPTION_MODE: {e:?}", c.token)))?;
                        }
                        Ok(format!("{client:#x}:{channel:#x} SET_CTXSW_PREEMPTION_MODE flags={flags:#x} gfxp={gfxp} cilp={cilp} on {} GR twin(s)", twins.len()))
                    }),
                )
            }
            ChanStatement::Timeslice { client, object, us } => {
                let twins: Vec<kf_host::Channel> = self
                    .pt
                    .lock()
                    .map(|m| m.iter().filter(|(k, v)| k.0 == client && v.tsg == Some(object)).map(|(_, v)| v.chan).collect())
                    .unwrap_or_default();
                if twins.is_empty() {
                    return ChanAnswer::NotOurs;
                }
                self.defer(
                    "timeslice",
                    Box::new(move |me: &ChanPlane| {
                        for c in &twins {
                            me.rm.set_timeslice(*c, us).map_err(|e| (NV_ERR_INVALID_ARGUMENT, format!("twin host {:#x} SET_TIMESLICE {us}: {e:?}", c.token)))?;
                        }
                        Ok(format!("{client:#x}:{object:#x} SET_TIMESLICE {us} us on {} twin group(s)", twins.len()))
                    }),
                )
            }
            ChanStatement::CudaLimit { client, device, enable } => {
                let flip = self.cuda_limit.lock().ok().and_then(|mut g| {
                    if enable {
                        g.0.insert((client, device));
                    } else {
                        g.0.remove(&(client, device));
                    }
                    let want = !g.0.is_empty();
                    (want != g.1).then(|| {
                        g.1 = want;
                        want
                    })
                });
                match flip {
                    None => ChanAnswer::Done,
                    Some(want) => self.defer(
                        "cuda limit",
                        Box::new(move |me: &ChanPlane| {
                            me.rm.perf_cuda_limit(want).map_err(|e| (NV_ERR_INVALID_STATE, format!("host PERF_CUDA_LIMIT_SET_CONTROL({want}): {e:?}")))?;
                            Ok(format!("{client:#x}:{device:#x} CUDA limit {enable} -> host device limit {want}"))
                        }),
                    ),
                }
            }
            // ⊘ The internal DISABLE names no Device (it is sent on the guest's internal device);
            // the Device's own free, which follows it, is what removes its row (see `free`).
            ChanStatement::CudaLimitDisable => ChanAnswer::Done,
            ChanStatement::Free { client, object } => self.free(client, object),
            ChanStatement::PromoteCtx { chan_client, object, engine_type, initialize, with_va, entries, falcon_ctx } => {
                if let (Some(fc), Ok(mut m)) = (falcon_ctx, self.pt.lock())
                    && let Some(v) = m.get_mut(&(chan_client, object))
                {
                    v.falcon_ctx = Some(fc);
                }
                self.promote_ctx(chan_client, object, engine_type, initialize, with_va, entries)
            }
            ChanStatement::EvictCtx { chan_client, object, engine_type } => self.evict_ctx(chan_client, object, engine_type),
            ChanStatement::Stop { client, object, immediate } => self.stop_channel(client, object, immediate),
            ChanStatement::DisableChannels { client, disable, only_scheduling, rewind_gp_put, list } => {
                self.disable_channels(client, disable, only_scheduling, rewind_gp_put, list.as_slice())
            }
            ChanStatement::Preempt { client, object, wait } => self.preempt_group(client, object, wait),
            ChanStatement::EncoderSession { client, acquire } => self.encoder_session(client, acquire),
        }
    }

    /// The Translated slot of `(client, object)`, if the plane owns one: its host token.
    fn translated_of(&self, client: u32, object: u32) -> Option<u32> {
        self.by_obj.lock().ok().and_then(|m| m.get(&(client, object)).copied())
    }

    /// ★★★ v3-chanctl — **`STOP_CHANNEL`, served as authored host verbs on the twin.**
    ///
    /// The guest's promise (`ctrla06fgpfifo.h:216-231`): the channel is disabled, unbound and off
    /// its runlist, NOT RUNNING (a preempt that fails RCs it). The host act on OUR channel:
    /// `DISABLE_CHANNELS{bDisable, bOnlyDisableScheduling=FALSE}` — RM's own words for exactly
    /// "none of the listed channels are running in hardware and will not run until …"
    /// (`ctrl2080fifo.h:309-316`) — then `GPFIFO_SCHEDULE(bEnable=FALSE)` (off the runlist). Both
    /// are unprivileged (`NON_PRIVILEGED` in their export flags; see `kf_host`). The reply waits
    /// for both. ⊘ NOT the host's own `STOP_CHANNEL`: host CPU-RM would then write the twin's error
    /// notifier (`kchannelNotifyRc_HAL`) — the GUEST's record, since the twin's error context is
    /// it — a host CPU write into guest memory that the guest's CPU-RM is about to make itself.
    /// A preempt failure is a host RC, reported through the RC plane like any other.
    ///
    /// Per-twin scope (owner ruling 2026-09-25): the twin is its own host group, so this stops
    /// exactly the channel the guest named — a group sibling keeps running, as on hardware (a
    /// channel STOP is channel-scoped). A later `GPFIFO_SCHEDULE(enable)` re-enables the twin
    /// (`stopped` is cleared there); nothing else is left behind.
    fn stop_channel(&self, client: u32, object: u32, immediate: bool) -> ChanAnswer {
        if let Some(ht) = self.translated_of(client, object) {
            return self.defer(
                "stop translated",
                Box::new(move |me: &ChanPlane| {
                    let slot = me.slot(ht).ok_or_else(|| (NV_ERR_INVALID_STATE, format!("host {ht:#x}: slot gone")))?;
                    let host = {
                        let mut g = slot.lock().map_err(|_| (NV_ERR_INVALID_STATE, "slot poisoned".to_string()))?;
                        g.scheduled = false;
                        g.stopped = true;
                        g.chan.host().channel()
                    };
                    me.rm
                        .disable_channels(&[host], true, false, false)
                        .and_then(|()| me.rm.schedule_enable(host, false))
                        .map_err(|e| (NV_ERR_INVALID_STATE, format!("host ring {ht:#x} stop: {e:?}")))?;
                    Ok(format!("{client:#x}:{object:#x} STOP_CHANNEL(bImmediate={immediate}): Translated ring host {ht:#x} disabled + preempted + off its runlist"))
                }),
            );
        }
        let Some(chan) = self.pt.lock().ok().and_then(|m| m.get(&(client, object)).map(|v| v.chan)) else {
            return ChanAnswer::NotOurs;
        };
        self.defer(
            "stop",
            Box::new(move |me: &ChanPlane| {
                me.rm
                    .disable_channels(&[chan], true, false, false)
                    .map_err(|e| (NV_ERR_INVALID_STATE, format!("host {:#x} DISABLE_CHANNELS: {e:?}", chan.token)))?;
                me.rm.schedule_enable(chan, false).map_err(|e| (NV_ERR_INVALID_STATE, format!("host {:#x} GPFIFO_SCHEDULE(false): {e:?}", chan.token)))?;
                if let Ok(mut m) = me.pt.lock()
                    && let Some(v) = m.get_mut(&(client, object))
                {
                    v.stopped = true;
                    if let Some(n) = v.notifier.as_mut() {
                        n.guest_stop_write = true;
                    }
                }
                Ok(format!(
                    "{client:#x}:{object:#x} STOP_CHANNEL(bImmediate={immediate}): twin host {:#x} disabled + preempted (DISABLE_CHANNELS) + off its runlist (GPFIFO_SCHEDULE false)",
                    chan.token
                ))
            }),
        )
    }

    /// ★★★ v3-chanctl — **`DISABLE_CHANNELS`, served as the same host verb over the twins.**
    ///
    /// Every entry names the calling client (the link refused any other). Each entry maps to its
    /// twin (Passthrough) or our host ring (Translated); ONE host `DISABLE_CHANNELS` with the
    /// guest's `bDisable` / `bOnlyDisableScheduling` / `bRewindGpPut` over OUR channels — the
    /// flags are the guest's intent about its own channels; the channels and the client are ours.
    /// ⊘ `bRewindGpPut` on a Translated channel is refused by name: our ring's `GP_PUT` is not the
    /// guest's (the guest's cursor is in its USERD, which the pump reads).
    /// ⊘ A list naming a channel the plane does not own is refused whole (nothing is half-done);
    /// a list naming none of ours is not ours.
    fn disable_channels(&self, client: u32, disable: bool, only_scheduling: bool, rewind: bool, list: &[(u32, u32)]) -> ChanAnswer {
        if list.is_empty() {
            // Vacuously true: RM disables nothing (the only in-tree caller never sends it).
            return ChanAnswer::Done;
        }
        let mut pt = Vec::new();
        let mut tr = Vec::new();
        let mut unknown = Vec::new();
        {
            let Ok(m) = self.pt.lock() else {
                return ChanAnswer::Refused { status: NV_ERR_INVALID_STATE, why: "twins poisoned".into() };
            };
            for &(c, h) in list {
                if let Some(v) = m.get(&(c, h)) {
                    pt.push(((c, h), v.chan));
                } else if let Some(ht) = self.translated_of(c, h) {
                    tr.push(((c, h), ht));
                } else {
                    unknown.push((c, h));
                }
            }
        }
        if pt.is_empty() && tr.is_empty() {
            return ChanAnswer::NotOurs;
        }
        if !unknown.is_empty() {
            return ChanAnswer::Refused {
                status: NV_ERR_INVALID_STATE,
                why: format!("DISABLE_CHANNELS names {} channel(s) with no host twin ({unknown:x?}); none disabled", unknown.len()),
            };
        }
        if rewind && !tr.is_empty() {
            return ChanAnswer::Refused {
                status: NV_ERR_INVALID_ARGUMENT,
                why: format!("DISABLE_CHANNELS bRewindGpPut on Translated channel(s) {tr:x?}: our host ring's GP_PUT is not the guest's"),
            };
        }
        self.defer(
            "disable channels",
            Box::new(move |me: &ChanPlane| {
                let mut hosts: Vec<kf_host::Channel> = pt.iter().map(|(_, c)| *c).collect();
                let mut slots = Vec::new();
                for (_, ht) in &tr {
                    let slot = me.slot(*ht).ok_or_else(|| (NV_ERR_INVALID_STATE, format!("host {ht:#x}: slot gone")))?;
                    let host = {
                        let mut g = slot.lock().map_err(|_| (NV_ERR_INVALID_STATE, "slot poisoned".to_string()))?;
                        if disable {
                            // The pump stops fetching BEFORE the host verb.
                            g.disabled = true;
                        }
                        g.chan.host().channel()
                    };
                    hosts.push(host);
                    slots.push(slot);
                }
                me.rm
                    .disable_channels(&hosts, disable, only_scheduling, rewind)
                    .map_err(|e| (NV_ERR_INVALID_STATE, format!("host DISABLE_CHANNELS over {} channel(s): {e:?}", hosts.len())))?;
                if let Ok(mut m) = me.pt.lock() {
                    for (k, _) in &pt {
                        if let Some(v) = m.get_mut(k) {
                            v.disabled = disable;
                        }
                    }
                }
                if !disable {
                    for s in &slots {
                        let idx = s.lock().ok().map(|mut g| {
                            g.disabled = false;
                            g.guest_idx
                        });
                        // Work the guest queued while disabled is picked up now.
                        if let Some(i) = idx
                            && me.plane.ring_internal(i)
                        {
                            let _ = me.wake.signal();
                        }
                    }
                }
                Ok(format!(
                    "{client:#x} DISABLE_CHANNELS(bDisable={disable}, bOnlyDisableScheduling={only_scheduling}, bRewindGpPut={rewind}) over {} twin(s) + {} Translated ring(s)",
                    pt.len(),
                    tr.len()
                ))
            }),
        )
    }

    /// ★★★ v3-chanctl — **`NVA06C` `PREEMPT` of a guest channel group, served per twin.**
    ///
    /// Each guest channel is its own host group (a twin, or our Translated ring), so the group
    /// preempt is `NVA06C PREEMPT(bWait=1)` on every member's host group, in turn, and the reply
    /// is held until the last completes (`bWait=0` is served the same way: waiting is the stronger
    /// promise, and the held reply is off every lock). Per-twin scope (owner ruling 2026-09-25):
    /// after the reply every member has been context-switched out, which is all a group preempt
    /// promises (it does not disable — a member with work may run again, on hardware too); the
    /// difference (members preempted one after another, not atomically) is timing only and not
    /// observable to guest userspace. A group the plane owns no member of is not ours.
    fn preempt_group(&self, client: u32, object: u32, wait: bool) -> ChanAnswer {
        let pt: Vec<kf_host::Channel> = self
            .pt
            .lock()
            .map(|m| m.iter().filter(|(k, v)| k.0 == client && v.tsg == Some(object)).map(|(_, v)| v.chan).collect())
            .unwrap_or_default();
        // ⊘ Group membership from the scopes table, never a slot lock on the drainer.
        let scopes = self.scopes.lock().map(|m| m.clone()).unwrap_or_default();
        let tr: Vec<u32> = self
            .by_obj
            .lock()
            .map(|m| m.iter().filter(|(k, _)| k.0 == client && scopes.get(k).is_some_and(|sc| sc.tsg == Some(object))).map(|(_, v)| *v).collect())
            .unwrap_or_default();
        if pt.is_empty() && tr.is_empty() {
            return ChanAnswer::NotOurs;
        }
        self.defer(
            "preempt",
            Box::new(move |me: &ChanPlane| {
                for c in &pt {
                    me.rm.preempt(*c).map_err(|e| (NV_ERR_INVALID_STATE, format!("twin host {:#x} PREEMPT: {e:?}", c.token)))?;
                }
                for ht in &tr {
                    let host = me
                        .slot(*ht)
                        .and_then(|s| s.lock().ok().map(|g| g.chan.host().channel()))
                        .ok_or_else(|| (NV_ERR_INVALID_STATE, format!("host {ht:#x}: slot gone")))?;
                    me.rm.preempt(host).map_err(|e| (NV_ERR_INVALID_STATE, format!("Translated ring host {ht:#x} PREEMPT: {e:?}")))?;
                }
                Ok(format!("{client:#x}:{object:#x} PREEMPT(bWait={wait}) — {} twin(s) + {} Translated ring(s) preempted (host bWait=1)", pt.len(), tr.len()))
            }),
        )
    }

    /// ★★★ `GPU_PROMOTE_CTX`, satisfied by the twin ([`CtxBind`]).
    ///
    /// ⊘ `[measured pr3]` the PA-initialize promote arrives BEFORE the GR object's own alloc RPC:
    /// `kgrobjConstruct` promotes inside its constructor (`kernel_graphics_object.c:225`) and the
    /// resource's alloc reaches us only after it, so at that moment the twin holds no host object
    /// yet. That promote is answered from the twin alone (a GR twin exists): the host context it
    /// stands for is created by the SAME guest alloc's engine-object act right after, and if that
    /// act fails the guest's alloc fails and its object — with its "initialized" flags — is torn
    /// down (`kernel_graphics_object.c:352-356`). No host work, so no act: answered inline.
    /// A promote that carries VAs (the UVM bind, after the object exists) is an ACT ordered behind
    /// any queued engine-object act, and requires the host GR object to exist.
    /// ⊘ Nothing is sent to the host and no guest byte is read or written.
    fn promote_ctx(&self, client: u32, object: u32, engine_type: u32, initialize: u32, with_va: u32, entries: u32) -> ChanAnswer {
        let Some((engine, ht)) = self.pt.lock().ok().and_then(|m| m.get(&(client, object)).map(|v| (v.engine, v.chan.token))) else {
            // A Translated (kernel CE) channel has no GR context and never promotes; a kernel GR
            // channel (RM's golden-image channel) is not born (P7) — both stay the FSM's refusal.
            eprintln!("kf3: chan {client:#x}:{object:#x} GPU_PROMOTE_CTX: no passthrough twin — not ours (entries={entries})");
            return ChanAnswer::NotOurs;
        };
        // ★ A VIDEO twin's promote is the falcon context (`_kflcnPromoteContext`,
        // `kernel_falcon.c:184-276`: `entryCount = 0`, the guest VA only). Host RM allocated and
        // promoted the twin's own falcon context with its engine object, so it is satisfied by the
        // twin — nothing sent, no guest byte touched.
        if kf_abi::submit::is_video_engine_type(engine) && engine_type == engine {
            let Ok(mut m) = self.pt.lock() else {
                return ChanAnswer::Refused { status: NV_ERR_INVALID_STATE, why: "twins poisoned".into() };
            };
            let Some(v) = m.get_mut(&(client, object)) else { return ChanAnswer::NotOurs };
            v.ctx.promotes += 1;
            eprintln!(
                "kf3: chan {client:#x}:{object:#x} GPU_PROMOTE_CTX (video falcon ctx, engine {engine:#x}) SATISFIED BY TWIN host {ht:#x}: entries={entries} host_objects={} — not forwarded",
                v.objects.len()
            );
            return ChanAnswer::Done;
        }
        if engine != kf_abi::submit::ENGINE_TYPE_GRAPHICS || engine_type != engine {
            return ChanAnswer::Refused {
                status: NV_ERR_INVALID_ARGUMENT,
                why: format!("GPU_PROMOTE_CTX engine {engine_type:#x} for a twin born on engine {engine:#x} (host {ht:#x})"),
            };
        }
        if with_va == 0 {
            let Ok(mut m) = self.pt.lock() else {
                return ChanAnswer::Refused { status: NV_ERR_INVALID_STATE, why: "twins poisoned".into() };
            };
            let Some(v) = m.get_mut(&(client, object)) else { return ChanAnswer::NotOurs };
            v.ctx.initialized |= initialize;
            v.ctx.promotes += 1;
            eprintln!(
                "kf3: chan {client:#x}:{object:#x} GPU_PROMOTE_CTX (initialize) SATISFIED BY TWIN host {ht:#x}: entries={entries} init_ids={initialize:#x} host_objects={} — not forwarded, no guest byte touched",
                v.objects.len()
            );
            return ChanAnswer::Done;
        }
        self.defer(
            "promote ctx",
            Box::new(move |me: &ChanPlane| {
                let mut m = me.pt.lock().map_err(|_| (NV_ERR_INVALID_STATE, "twins poisoned".to_string()))?;
                let v = m
                    .get_mut(&(client, object))
                    .ok_or_else(|| (NV_ERR_INVALID_STATE, format!("{client:#x}:{object:#x}: twin freed before its promote")))?;
                let host_ctx: Vec<u32> = v
                    .objects
                    .values()
                    .filter(|(_, k)| matches!(k, kf_chip::classes::Kind::Compute | kf_chip::classes::Kind::ThreeD))
                    .map(|(h, _)| *h)
                    .collect();
                if host_ctx.is_empty() {
                    return Err((
                        NV_ERR_INVALID_STATE,
                        format!(
                            "{client:#x}:{object:#x} GPU_PROMOTE_CTX: twin host {ht:#x} holds no GR engine object, so host RM has no context to stand for the guest's"
                        ),
                    ));
                }
                v.ctx.initialized |= initialize;
                v.ctx.va_bound |= with_va;
                v.ctx.bound |= with_va != 0;
                v.ctx.promotes += 1;
                Ok(format!(
                    "chan {client:#x}:{object:#x} GPU_PROMOTE_CTX SATISFIED BY TWIN host {ht:#x} (host GR object(s) {host_ctx:x?}): entries={entries} init_ids={initialize:#x} va_ids={with_va:#x} bound={} — not forwarded, no guest byte touched",
                    v.ctx.bound
                ))
            }),
        )
    }

    /// `GPU_EVICT_CTX` — the unbind half. The guest's `nvGpuOpsStopChannel` sends it after
    /// `STOP_CHANNEL` and then clears `bIsContextBound` (`nv_gpu_ops.c:10956-10983`); `NV_OK`
    /// asserts the context is switched out. The twin is taken off the host runlist (its own
    /// `GPFIFO_SCHEDULE` disable, an authored verb), so that promise is the host's, and the
    /// binding is recorded UNBOUND.
    fn evict_ctx(&self, client: u32, object: u32, engine_type: u32) -> ChanAnswer {
        let Some((engine, chan)) = self.pt.lock().ok().and_then(|m| m.get(&(client, object)).map(|v| (v.engine, v.chan))) else {
            return ChanAnswer::NotOurs;
        };
        if engine != kf_abi::submit::ENGINE_TYPE_GRAPHICS || engine_type != engine {
            return ChanAnswer::Refused {
                status: NV_ERR_INVALID_ARGUMENT,
                why: format!("GPU_EVICT_CTX engine {engine_type:#x} for a twin born on engine {engine:#x} (host {:#x})", chan.token),
            };
        }
        self.defer(
            "evict ctx",
            Box::new(move |me: &ChanPlane| {
                me.rm.schedule_enable(chan, false).map_err(|e| (NV_ERR_INVALID_STATE, format!("host {:#x} disable: {e:?}", chan.token)))?;
                let ctx = me.pt.lock().ok().and_then(|mut m| {
                    m.get_mut(&(client, object)).map(|v| {
                        v.ctx.bound = false;
                        v.ctx.va_bound = 0;
                        v.ctx.evicts += 1;
                        v.ctx
                    })
                });
                Ok(format!("chan {client:#x}:{object:#x} GPU_EVICT_CTX: twin host {:#x} off the runlist, context UNBOUND ({ctx:?})", chan.token))
            }),
        )
    }

    fn schedule_translated(&self, client: u32, object: u32, ht: u32, enable: bool) -> ChanAnswer {
        let Some(slot) = self.slot(ht) else { return ChanAnswer::NotOurs };
        // ★ v3-chanctl: a STOPPED Translated channel's host ring was disabled and taken off its
        // runlist — re-enable it (host verbs: an act) before the pump may fetch again.
        if enable && slot.try_lock().is_ok_and(|g| g.stopped) {
            return self.defer(
                "restart translated",
                Box::new(move |me: &ChanPlane| {
                    let slot = me.slot(ht).ok_or_else(|| (NV_ERR_INVALID_STATE, format!("host {ht:#x}: slot gone")))?;
                    let (host, disabled) = slot
                        .lock()
                        .map(|g| (g.chan.host().channel(), g.disabled))
                        .map_err(|_| (NV_ERR_INVALID_STATE, "slot poisoned".to_string()))?;
                    if !disabled {
                        me.rm.disable_channels(&[host], false, false, false).map_err(|e| (NV_ERR_INVALID_STATE, format!("host ring {ht:#x} re-enable: {e:?}")))?;
                    }
                    me.rm.schedule_enable(host, true).map_err(|e| (NV_ERR_INVALID_STATE, format!("host ring {ht:#x} schedule: {e:?}")))?;
                    let idx = slot.lock().map(|mut g| {
                        g.stopped = false;
                        g.scheduled = true;
                        g.guest_idx
                    });
                    if let Ok(i) = idx
                        && me.plane.ring_internal(i)
                    {
                        let _ = me.wake.signal();
                    }
                    Ok(format!("{client:#x}:{object:#x} GPFIFO_SCHEDULE enable=true: Translated ring host {ht:#x} restarted after STOP"))
                }),
            );
        }
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
    fn engine_object(&self, client: u32, parent: u32, handle: u32, class: u32, copy_engine: Option<u32>) -> ChanAnswer {
        let Some((chan, engine, space)) = self.pt.lock().ok().and_then(|m| m.get(&(client, parent)).map(|v| (v.chan, v.engine, v.space))) else {
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
                // ★★ v3-video — STEER THE HOST'S FALCON CONTEXT ONTO THE GUEST'S. In a video
                // channel's VA space guest RM and host RM place buffers with the SAME lowest-free
                // allocator, user buffers and RM-internal ones alike. `[measured vvid vid10]` the
                // guest put its falcon ctx at G = 0x12002a000; host RM, allocating the twin's own ctx
                // with this object, found G mirrored and took G+0x1000 — where nvcuvid then mapped a
                // live 4 KiB buffer, which the walker could only report HELD BY HOST: the engine
                // used the wrong page and NVDEC produced untouched frames. So G (the guest's ctx,
                // which no engine ever reads — the twin runs on the host's) is unmapped from the
                // twin first, and host RM takes G itself. Best effort: a failed unmap is logged.
                let fc = if matches!(kind, kf_chip::classes::Kind::VideoEncoder | kf_chip::classes::Kind::VideoDecoder) {
                    me.pt.lock().ok().and_then(|m| m.get(&(client, parent)).and_then(|v| v.falcon_ctx))
                } else {
                    None
                };
                let steer = fc.map(|(va, len)| match me.rm.unmap(space, va, false) {
                    Ok(()) => format!(" [host ctx steered onto the guest's ctx VA {va:#x}+{len:#x}]"),
                    Err(e) => format!(" [guest ctx VA {va:#x} not unmapped ({e:?}) — host ctx placement unsteered]"),
                });
                let h = kf_chan::passthrough::engine_object(me.rm, chan, engine, class, kind, copy_engine).map_err(|e| (NV_ERR_INVALID_CLASS, e))?;
                if let Ok(mut m) = me.pt.lock()
                    && let Some(v) = m.get_mut(&(client, parent))
                {
                    v.objects.insert(handle, (h, kind));
                }
                if let Ok(mut m) = me.pt_objs.lock() {
                    m.insert((client, handle), (client, parent));
                }
                Ok(format!("{client:#x}:{handle:#x} class {class:#x} ({kind:?}) on twin host {:#x} -> host object {h:#x}{}", chan.token, steer.unwrap_or_default()))
            }),
        )
    }

    /// ★ A free the plane may own: a Translated channel, a passthrough twin (by channel, its group,
    /// its device, or its client), or an engine object on one. Tokens stop routing to a twin NOW
    /// (atomics, no host call); the host frees run as ONE act whose outcome the reply waits for.
    /// ★ w827 — **a `GT200_DEBUGGER` session, twinned on the host.** `cuCtxCreate` allocates one
    /// per context, bound to its own GR object, then sets its exception mask; refusing the alloc
    /// failed `cuCtxCreate` with `CUDA_ERROR_INVALID_VALUE` on every CUDA rung (`[measured vh
    /// w827, 238a88f6]`, guest `GspRmAlloc failed … hClass=0x000083de … status=0x56`).
    ///
    /// The guest's `hClass3dObject` must name a compute/3D object on one of ITS OWN client's
    /// passthrough twins (`hAppClient` is the allocating client — a session over another client's
    /// context is refused by name, as `DISABLE_CHANNELS` refuses another client's channel); the
    /// host session is allocated with params WE author, bound to that twin's host GR object. So the
    /// host session watches exactly the host context the guest's own context runs as.
    fn debugger(&self, client: u32, parent: u32, handle: u32, app_client: u32, obj3d: u32) -> ChanAnswer {
        if app_client != client {
            return ChanAnswer::Refused {
                status: NV_ERR_INVALID_ARGUMENT,
                why: format!("GT200_DEBUGGER in client {client:#x} over another client's ({app_client:#x}) object"),
            };
        }
        let host_obj = self.pt_objs.lock().ok().and_then(|m| m.get(&(client, obj3d)).copied()).and_then(|key| {
            self.pt.lock().ok().and_then(|m| m.get(&key).and_then(|v| v.objects.get(&obj3d).copied()))
        });
        let host_obj = match host_obj {
            Some((h, kf_chip::classes::Kind::Compute | kf_chip::classes::Kind::ThreeD)) => h,
            Some((_, k)) => {
                return ChanAnswer::Refused { status: NV_ERR_INVALID_ARGUMENT, why: format!("GT200_DEBUGGER over {client:#x}:{obj3d:#x}, a {k:?} object — not GR") };
            }
            None => {
                return ChanAnswer::Refused {
                    status: NV_ERR_INVALID_ARGUMENT,
                    why: format!("GT200_DEBUGGER over {client:#x}:{obj3d:#x}: no passthrough twin holds that object"),
                };
            }
        };
        self.defer(
            "debugger",
            Box::new(move |me: &ChanPlane| {
                let h = me.rm.alloc_debugger(host_obj).map_err(|e| (NV_ERR_INVALID_STATE, format!("GT200_DEBUGGER over host object {host_obj:#x}: {e:?}")))?;
                if let Ok(mut m) = me.dbg.lock() {
                    m.insert((client, handle), (parent, h, host_obj));
                }
                Ok(format!("{client:#x}:{handle:#x} GT200_DEBUGGER over {obj3d:#x} -> host session {h:#x} (host GR object {host_obj:#x})"))
            }),
        )
    }

    fn free(&self, client: u32, object: u32) -> ChanAnswer {
        // ★ w827: debugger sessions go FIRST — freed by name, by their device, or by their client,
        // or because the GR object they are bound to is about to go with its twin.
        let doomed_twin_objs: Vec<u32> = self
            .pt
            .lock()
            .map(|m| {
                m.iter()
                    .filter(|(k, v)| ChanScope { tsg: v.tsg, parent: v.parent, device: v.device }.freed_by(**k, client, object))
                    .flat_map(|(_, v)| v.objects.values().map(|o| o.0).collect::<Vec<_>>())
                    .collect()
            })
            .unwrap_or_default();
        let freed_obj = self
            .pt_objs
            .lock()
            .ok()
            .and_then(|m| m.get(&(client, object)).copied())
            .and_then(|key| self.pt.lock().ok().and_then(|m| m.get(&key).and_then(|v| v.objects.get(&object).map(|o| o.0))));
        let debuggers: Vec<u32> = self
            .dbg
            .lock()
            .map(|mut m| {
                let keys: Vec<(u32, u32)> = m
                    .iter()
                    .filter(|((c, h), (dev, _, gr))| {
                        (*c == client && (*h == object || object == client || *dev == object))
                            || doomed_twin_objs.contains(gr)
                            || freed_obj == Some(*gr)
                    })
                    .map(|(k, _)| *k)
                    .collect();
                keys.into_iter().filter_map(|k| m.remove(&k).map(|v| v.1)).collect()
            })
            .unwrap_or_default();
        // ★ w827: a freed Device (or client) takes its CUDA-limit row; the host limit goes off with
        // the last one.
        let limit_off = self
            .cuda_limit
            .lock()
            .ok()
            .and_then(|mut g| {
                let before = g.0.len();
                g.0.retain(|(c, d)| !(*c == client && (*d == object || object == client)));
                (g.0.len() != before && g.0.is_empty() && g.1).then(|| {
                    g.1 = false;
                })
            })
            .is_some();
        self.free_channels(client, object, debuggers, limit_off)
    }

    fn free_channels(&self, client: u32, object: u32, debuggers: Vec<u32>, limit_off: bool) -> ChanAnswer {
        // Translated channels: by object, or every one of the client's.
        // ★ v3-promote: a free of the channel, its group, its parent, its DEVICE or its client
        // takes the channel with it (the guest frees a subtree with ONE RPC) — for Translated
        // channels too, which were matched by handle or client only, so a group or device free
        // left the host ring pumping after the guest considered the channel gone.
        let scopes = self.scopes.lock().map(|m| m.clone()).unwrap_or_default();
        let translated: Vec<u32> = self
            .by_obj
            .lock()
            .map(|mut m| {
                let keys: Vec<(u32, u32)> = m
                    .keys()
                    .copied()
                    .filter(|k| {
                        let sc = scopes.get(k).copied().unwrap_or(ChanScope { tsg: None, parent: k.1, device: k.1 });
                        sc.freed_by(*k, client, object)
                    })
                    .collect();
                keys.into_iter().filter_map(|k| m.remove(&k)).collect()
            })
            .unwrap_or_default();
        if let Ok(mut m) = self.scopes.lock() {
            m.retain(|k, sc| !sc.freed_by(*k, client, object));
        }
        let twins = self.take_pt(|k, v| ChanScope { tsg: v.tsg, parent: v.parent, device: v.device }.freed_by(*k, client, object));
        // Engine objects freed on their own (their twin still lives).
        let obj = if twins.is_empty() {
            self.pt_objs.lock().ok().and_then(|mut m| m.remove(&(client, object))).and_then(|key| {
                self.pt.lock().ok().and_then(|mut m| m.get_mut(&key).and_then(|v| v.objects.remove(&object).map(|o| o.0)))
            })
        } else {
            None
        };
        let sessions = client == object && self.enc_sessions.lock().is_ok_and(|m| m.get(&client).is_some_and(|n| *n > 0));
        if translated.is_empty() && twins.is_empty() && obj.is_none() && debuggers.is_empty() && !limit_off && !sessions {
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
                if client == object {
                    line.extend(me.release_encoder_sessions(client));
                }
                for h in debuggers {
                    let r = me.rm.free(h);
                    line.push(format!("debugger session host {h:#x} {}", if r.is_ok() { "freed" } else { "FREE REFUSED" }));
                }
                if limit_off {
                    let r = me.rm.perf_cuda_limit(false);
                    line.push(format!("host CUDA limit off ({})", if r.is_ok() { "ok" } else { "REFUSED" }));
                }
                for ht in translated {
                    me.retire(ht);
                    line.push(format!("translated host {ht:#x}"));
                }
                for ((c, h), t) in twins {
                    let r = me.release_twin(c, t.tsg, t.chan);
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
                    line.push(format!("passthrough {c:#x}:{h:#x} token {:#x} host {:#x} objects={} ctx={:?} {}", t.idx, t.chan.token, t.objects.len(), t.ctx, if r.is_ok() { "freed" } else { "FREE REFUSED" }));
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

    /// ★ v3-video: a guest NVENC session acquire/release, carried to the HOST's GPU-wide slot
    /// accounting (`kf_abi::gssreplay::GSS_ENC_SESSION_ACQUIRE`): an act (a host ioctl), the reply
    /// held until it lands. A release with nothing held is answered without a host call.
    fn encoder_session(&self, client: u32, acquire: bool) -> ChanAnswer {
        self.defer(
            "encoder session",
            Box::new(move |me: &ChanPlane| {
                let held = me.enc_sessions.lock().map(|m| m.get(&client).copied().unwrap_or(0)).unwrap_or(0);
                if !acquire && held == 0 {
                    return Ok(format!("{client:#x} NVENC session release with none held — no host call"));
                }
                let cmd = if acquire { kf_abi::gssreplay::GSS_ENC_SESSION_ACQUIRE } else { kf_abi::gssreplay::GSS_ENC_SESSION_RELEASE };
                let mut p = [0u8; kf_abi::gssreplay::ENC_SESSION_PARAMS_SIZE];
                me.rm.raw_control(me.rm.subdevice(), cmd, &mut p).map_err(|e| {
                    let st = match e {
                        kf_host::RmError::Other(s) if s < 0x4B00 => s,
                        _ => NV_ERR_INVALID_STATE,
                    };
                    (st, format!("{client:#x} host NVENC session {}: {e:?}", if acquire { "acquire" } else { "release" }))
                })?;
                let now = me.enc_sessions.lock().map(|mut m| {
                    let c = m.entry(client).or_insert(0);
                    if acquire { *c += 1 } else { *c = c.saturating_sub(1) }
                    *c
                });
                Ok(format!("{client:#x} NVENC session {} on the host (held now {now:?})", if acquire { "ACQUIRED" } else { "released" }))
            }),
        )
    }

    /// ★ v3-video: release every host NVENC slot a freed guest client still held.
    fn release_encoder_sessions(&self, client: u32) -> Vec<String> {
        let n = self.enc_sessions.lock().ok().and_then(|mut m| m.remove(&client)).unwrap_or(0);
        (0..n)
            .map(|_| {
                let mut p = [0u8; kf_abi::gssreplay::ENC_SESSION_PARAMS_SIZE];
                let r = self.rm.raw_control(self.rm.subdevice(), kf_abi::gssreplay::GSS_ENC_SESSION_RELEASE, &mut p);
                format!("NVENC session of freed client {client:#x} released on the host ({})", if r.is_ok() { "ok" } else { "REFUSED" })
            })
            .collect()
    }

    /// ★ v3-video: free a Passthrough twin — a member of a shared host group frees its channel,
    /// and the group goes with its LAST member; an ungrouped twin frees channel and group.
    fn release_twin(&self, client: u32, guest_tsg: Option<u32>, chan: kf_host::Channel) -> Result<(), kf_host::RmError> {
        let Some(k) = guest_tsg.map(|t| (client, t)) else { return self.rm.free_channel(chan) };
        let r = self.rm.free_member(chan);
        let last = self.groups.lock().map_or(true, |mut m| match m.get_mut(&k) {
            Some(g) if g.1 > 1 => {
                g.1 -= 1;
                false
            }
            _ => {
                m.remove(&k);
                true
            }
        });
        if last { r.and(self.rm.free(chan.tsg)) } else { r }
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
        if passthrough
            && !is_copy_engine(engine)
            && engine != kf_abi::submit::ENGINE_TYPE_GRAPHICS
            && !kf_abi::submit::is_video_engine_type(engine)
        {
            return refuse(
                NV_ERR_NOT_SUPPORTED,
                format!("user channel on engine type {engine:#x}: only a copy engine, GR0 or a video engine has a passthrough twin"),
            );
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
                    // ★ v3-video: a member of a guest TSG joins the host group standing for it.
                    let gkey = a.tsg.map(|t| (a.client, t));
                    let join = gkey.and_then(|k| me.groups.lock().ok().and_then(|m| m.get(&k).map(|g| g.0)));
                    let chan = match kf_chan::passthrough::birth_twin_in(me.rm, space, g, join) {
                        Ok(c) => {
                            if let (Some(k), Ok(mut m)) = (gkey, me.groups.lock()) {
                                m.entry(k).and_modify(|g| g.1 += 1).or_insert((c.tsg, 1));
                            }
                            c
                        }
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
                        let _ = me.release_twin(a.client, a.tsg, chan);
                        live.fetch_sub(1, Ordering::AcqRel);
                        if let Some(n) = notifier {
                            me.release_notifier(n);
                        }
                        return Err((NV_ERR_INSUFFICIENT_RESOURCES, format!("token {idx:#x}: {e}")));
                    }
                    let rc = if notifier.is_some() { "armed" } else { "none" };
                    if let Ok(mut m) = me.pt.lock() {
                        m.insert((a.client, a.handle), PtChan {
                            chan,
                            idx,
                            tsg: a.tsg,
                            parent: a.parent,
                            device: a.device,
                            engine,
                            objects: HashMap::new(),
                            ctx: CtxBind::default(),
                            live,
                            notifier,
                            stopped: false,
                            disabled: false,
                            space,
                            falcon_ctx: None,
                        });
                    }
                    me.pt_births.fetch_add(1, Ordering::Relaxed);
                    me.engine_live(engine, true);
                    let _ = me.take_ledger(idx);
                    Ok(format!(
                        "chan {:#x}:{:#x} BORN Passthrough: token {idx:#x} -> host {:#x} in {key:?} gpfifo={:#x}x{} userd={userd:?} engine={engine:#x} declared_kernel_pid={} {} rc={rc}",
                        a.client,
                        a.handle,
                        chan.token,
                        a.gpfifo_va,
                        g.entries,
                        a.declared_kernel_pid,
                        kernel_by(&a)
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
                // ★ P6b: OUR ring goes in OUR region of the space, never where RM's allocator (the
                // guest's own allocator) would put it — `crate::mem::RING_REGION_BASE`.
                let at = crate::mem::take_ring_slot(&mirror.rings)
                    .ok_or_else(|| fail((NV_ERR_INSUFFICIENT_RESOURCES, "host ring: the space's ring region is exhausted".to_string())))?;
                let host = HostRing::on_engine_at(me.rm, mirror.space, me.host_ce, Some(at))
                    .map_err(|e| fail((NV_ERR_INSUFFICIENT_RESOURCES, format!("host ring: {e}"))))?;
                let ht = host.channel().token;
                let ring_va = host.va();
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
                let slot = Slot {
                    chan,
                    key,
                    mirror,
                    userd,
                    guest_idx: idx,
                    scheduled: false,
                    dead: None,
                    serves: 0,
                    last_put: None,
                    tsg: a.tsg,
                    split: None,
                    splits: 0,
                    views: StoreViews::new(),
                    privilege: a.privilege,
                    stopped: false,
                    disabled: false,
                };
                if let Ok(mut s) = me.slots.write() {
                    s.insert(ht, Arc::new(Mutex::new(slot)));
                }
                if let Ok(mut m) = me.by_obj.lock() {
                    m.insert((a.client, a.handle), ht);
                }
                if let Ok(mut m) = me.scopes.lock() {
                    m.insert((a.client, a.handle), ChanScope { tsg: a.tsg, parent: a.parent, device: a.device });
                }
                me.births.fetch_add(1, Ordering::Relaxed);
                Ok(format!(
                    "chan {:#x}:{:#x} BORN Translated: token {idx:#x} -> host {ht:#x} in {key:?} gpfifo={:#x}x{entries} userd={:?} engine={engine:#x} tsg={:x?} kernel_by={} ring_va={ring_va:#x}",
                    a.client,
                    a.handle,
                    a.gpfifo_va,
                    a.userd,
                    a.tsg,
                    kernel_by(&a)
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
                let mut n = PtNotifier { ctx, view, reported: false, at_arm: [0; 4], guest_stop_write: false };
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
                // ★ v3-chanctl: the guest's own post-STOP write is re-baselined, never reported.
                if n.guest_stop_write && w != n.at_arm && w[2] == ROBUST_CHANNEL_PREEMPTIVE_REMOVAL {
                    n.at_arm = w;
                    continue;
                }
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
        let Ok(mut g) = slot.lock() else { return };
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
        let armed = g.views.armed;
        g.views.release_all(self.rm, self.store);
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
            "kf3: chan token {:#x} (host {ht:#x}) RETIRED, forwarded={} submissions={} splits={}/{} serves={} last_put={:?} gp_get={:?} store_views={armed} privilege={:?} dead={:?}",
            g.guest_idx,
            g.chan.counts().0,
            g.chan.counts().1,
            g.chan.counts().2,
            g.splits,
            g.serves,
            g.last_put,
            g.chan.last_gp_get(),
            g.privilege,
            g.dead
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
        if g.dead.is_some() || !g.scheduled || g.disabled || g.stopped || self.stop.load(Ordering::Acquire) {
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
        let mut mem = Mem { mirror: &mirror, ram: self.ram, rm: self.rm, store: self.store, views: &mut g.views };
        let win = SlotWindow { mirror: &mirror, ram: self.ram };
        let mut split = VaSplit { inbox: &self.inbox, token: g.guest_idx, ticket: &mut g.split, requested: &mut g.splits };
        let r = g.chan.pump(self.rm, &self.completions, &mut mem, &mut Userd(&g.userd), &mut split, is_any_ce_class, &win);
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

#[cfg(test)]
mod scope_tests {
    use super::ChanScope;

    /// ★ v3-promote: a guest free of ANY handle the channel hangs under takes its twin — the
    /// channel, its group, its parent, its device or its client — and nothing else does (another
    /// client's identical handles, a sibling channel, an unrelated object).
    #[test]
    fn a_group_device_or_client_free_takes_every_twin_under_it() {
        let (c, dev, tsg, ch, sib) = (0xc1d0_000b, 0x5c00_0001, 0xcafe_0010, 0xcafe_0013, 0xcafe_0014);
        let member = ChanScope { tsg: Some(tsg), parent: tsg, device: dev };
        assert!(member.freed_by((c, ch), c, ch), "the channel itself");
        assert!(member.freed_by((c, ch), c, tsg), "its group");
        assert!(member.freed_by((c, ch), c, dev), "its DEVICE (a group member's parent is the group)");
        assert!(member.freed_by((c, ch), c, c), "its client");
        assert!(!member.freed_by((c, ch), c, sib), "a sibling's free leaves it");
        assert!(!member.freed_by((c, ch), 0xc1d0_000c, tsg), "another client's same handle");
        assert!(!member.freed_by((c, ch), c, 0xdead_0001), "an unrelated object");
        let bare = ChanScope { tsg: None, parent: dev, device: dev };
        assert!(bare.freed_by((c, ch), c, dev));
        assert!(!bare.freed_by((c, ch), c, tsg));
    }
}
