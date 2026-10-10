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

// ★ Review addendum (2026-10-10, `V3_BATCHED_MAP.md` §8.8.9): THE GUEST IS UNTRUSTED. This plane
// reads the guest's rings, pushbuffers and USERD and the rows that back them; a panic on a thread
// that serves it is a denial of service. Guest-derived values (VAs, lengths, ring indices) are only
// added/subtracted/divided with `checked_*` / `saturating_*`, and nothing unwraps or indexes blindly.
// A new panic site fails CI.
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

use crate::mem::{Mirror, Mirrors, RamMap, resolve_placed, resolve_placed_prefix};
use crate::raw_unsafe::RawRegion;
use kf_chan::completions::Completions;
use kf_chan::host::{ChanError, GuestUserd, Publisher, Split, TranslatedChannel};
use kf_chan::ring::{GuestMemory, TranslatedRing};
use kf_chan::translated::{Target, Window};
use kf_core::{Owner, Plane, VmCaps};
use kf_host::{HostRm, MapNode, ViewAccess};
use kf_linux_raw::{Backing, CachePolicy, HostOffset, HostPageSize, VolatileRegion};
use kf_mem::vasmgr::VasKey;
use kf_rm::chanlink::{ChanAnswer, ChanStatement, ChannelAlloc};
use kf_trap::Route;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
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

/// ★ EXPERIMENT `KF3_GSS_NATIVE`: the host seam of `kf_rm::gssnative` over OUR host client: one
/// `NV_ESC_RM_CONTROL` on kayfabe's own host subdevice, params carried unchanged
/// ([`HostRm::raw_control_opaque`]).
struct GssHostRm(&'static HostRm);

impl kf_rm::gssnative::GssHost for GssHostRm {
    fn control(&self, cmd: u32, params: &mut [u8]) -> Result<u32, String> {
        self.0
            .raw_control_opaque(self.0.subdevice(), cmd, params)
            .map_err(|e| format!("{e:?}"))
    }
}

/// ★ P5b: one host act, run on the plane's act thread. `Err((status, why))` refuses by name.
type Act = Box<dyn FnOnce(&ChanPlane) -> Result<String, (u32, String)> + Send>;

/// ★ Review 4 item 1: how much to hand to host RM for a falcon ctx at G of length `ctx_len`: the whole
/// placement row when the steer removed one, else the ctx range itself — a MISSING row (an earlier
/// try removed it and the steer was refused; the mapping stayed in the ledger) never means "nothing
/// of ours there", so the hand-over is attempted all the same and only `Free` lets the alloc go.
fn steer_len(removed_row_len: Option<u64>, ctx_len: u64) -> u64 {
    removed_row_len.unwrap_or(ctx_len)
}

/// ★ Review 3 item 5: the status an act returns after it has put ITSELF back on the act queue
/// ([`ChanPlane::requeue`]): the act loop neither resolves its `Deferred` nor counts a refusal.
const ACT_REQUEUED: u32 = 0xFFFF_FFF0;

thread_local! {
    /// ★ Review 4 item 4: acts the running act asked to be parked ([`delay_act`]); the act loop moves
    /// them into its delay list after the act returns.
    static ACT_DELAYED: std::cell::RefCell<Vec<(std::time::Instant, Act, kf_gsp::Deferred, &'static str)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// ★ Review 4 item 4: from INSIDE an act (on the act thread), park `act` until `not_before`; the act
/// loop runs everything else meanwhile (no sleeping). The act then returns [`ACT_REQUEUED`].
fn delay_act(not_before: std::time::Instant, what: &'static str, act: Act, d: kf_gsp::Deferred) {
    ACT_DELAYED.with(|c| c.borrow_mut().push((not_before, act, d, what)));
}

/// ★★ v3-video — one guest `engine object` alloc on a Passthrough twin, as a RE-QUEUEABLE act.
///
/// STEER THE HOST'S FALCON CONTEXT ONTO THE GUEST'S. In a video channel's VA space guest RM and host
/// RM place buffers with the SAME lowest-free allocator, user buffers and RM-internal ones alike.
/// `[measured vvid vid10, V3_VIDEO_ENGINES.md]` the guest put its falcon ctx at G = 0x12002a000; host RM, allocating the
/// twin's own ctx with this object, found G mirrored and took G+0x1000 — where nvcuvid then mapped a
/// live 4 KiB buffer, which the walker could only report HELD BY HOST: the engine used the wrong page
/// and NVDEC produced untouched frames. So G (the guest's ctx, which no engine ever reads — the twin
/// runs on the host's) is unmapped from the twin first, and host RM takes G itself.
///
/// ★ Review 3 item 5: the host alloc happens ONLY after the hand-over answered `Free`
/// ([`kf_mem::batch::steer_step`]). Before D2 a failed unmap was "logged, best effort" and the alloc
/// went ahead — which re-introduces exactly the wrong-frames bug; D2's `Busy`/`StillOurs`/`Refused`
/// answers were then treated the same. Now each is retried (re-queued behind the other acts, after a
/// 1 ms pause — the act thread never waits on the VA thread) up to [`kf_mem::batch::STEER_MAX_TRIES`]
/// / [`kf_mem::batch::STEER_MAX_AGE`], then the birth is REFUSED by name. Where there is nothing to
/// steer (no falcon ctx, G inside a reserved guest range, the row not mirrored yet) it proceeds as it
/// always did.
#[derive(Clone)]
struct EngineObj {
    client: u32,
    parent: u32,
    handle: u32,
    class: u32,
    copy_engine: Option<u32>,
    chan: kf_host::Channel,
    engine: u32,
    space: kf_host::VaSpace,
    rows: crate::mem::PlacedRows,
    ledger: Option<Arc<kf_mem::batch::BatchedVas<'static>>>,
    kind: kf_chip::classes::Kind,
}

impl EngineObj {
    /// The act for try number `tries`; `steered` = the `(va, row length)` whose placement row an
    /// earlier try already removed (a retry must not read the missing row as "nothing there").
    fn act(
        self,
        tries: u32,
        born: Option<std::time::Instant>,
        steered: Option<(u64, u64)>,
        d: kf_gsp::Deferred,
    ) -> Act {
        Box::new(move |me: &ChanPlane| self.run(me, tries, born, steered, d))
    }

    fn run(
        self,
        me: &ChanPlane,
        tries: u32,
        born: Option<std::time::Instant>,
        steered: Option<(u64, u64)>,
        d: kf_gsp::Deferred,
    ) -> Result<String, (u32, String)> {
        let Self {
            client,
            parent,
            space,
            ref rows,
            ref ledger,
            kind,
            ..
        } = self;
        // ★ Review 4 item 3: the age bound runs from the FIRST EXECUTION of the step, not from its
        // enqueue — a busy act queue must not eat the retry budget.
        let born = born.unwrap_or_else(std::time::Instant::now);
        // ★ Review 4 (lower item): a retry that runs after the channel was freed (or replaced) must
        // not run the host alloc on the old channel.
        if tries > 0 {
            let alive = me
                .pt
                .lock()
                .ok()
                .and_then(|m| m.get(&(client, parent)).map(|v| v.chan.token))
                == Some(self.chan.token);
            if !alive {
                return Err((
                    NV_ERR_INVALID_STATE,
                    format!(
                        "chan {client:#x}:{parent:#x} was freed while its falcon ctx steer was retrying — object birth REFUSED"
                    ),
                ));
            }
        }
        let fc = if matches!(
            kind,
            kf_chip::classes::Kind::VideoEncoder
                | kf_chip::classes::Kind::VideoDecoder
                | kf_chip::classes::Kind::OpticalFlow
        ) {
            me.pt
                .lock()
                .ok()
                .and_then(|m| m.get(&(client, parent)).and_then(|v| v.falcon_ctx))
        } else {
            None
        };
        // ★★ v3-int — HOW THIS COMPOSES WITH THE v3-gfx GUEST-VA RESERVATION
        // (`kf_host::GUEST_VA_RANGES`). The collision above exists only where host RM's
        // allocator may place: with the guest's ranges reserved in the twin's space, host
        // RM's falcon ctx can only land in the host hole `[HOST_HOLE_LO, 1 TiB)` — never
        // at G nor G+0x1000 — so no guest buffer can meet it and there is nothing to steer.
        // ⇒ Steer ONLY when G is outside a live reservation (reservation refused, or
        // `KF3_NO_GUEST_VA_RESERVE`); otherwise the guest's own mapping at G is left alone.
        let fc = fc.filter(|&(va, len)| {
            let reserved = space.guest_reserved(va, len);
            if reserved && tries == 0 {
                eprintln!(
                    "kf3: chan {client:#x}:{parent:#x} falcon ctx G={va:#x}+{len:#x} is inside the reserved guest VA — host RM places its own ctx in the host hole; not steered"
                );
            }
            !reserved
        });
        if tries > 0 && steered.is_some() && fc.is_none() {
            return Err((
                NV_ERR_INVALID_STATE,
                format!(
                    "chan {client:#x}:{parent:#x}: the falcon ctx is no longer known while its steer was retrying — object birth REFUSED"
                ),
            ));
        }
        let mut steer_msg = String::new();
        if let Some((va, len)) = fc {
            // The row goes first: from here G is host RM's. `rows.remove` is also the arbitration
            // `cut_own`'s argument relies on (`V3_BATCHED_MAP.md` §8.8.2). The `rows` write lock is
            // held for the `remove` ONLY (review 3 item 6): the ledger walk below is outside it.
            // ★ Review 4 item 1: a MISSING row is NOT "nothing mirrored there". After a refused first
            // try the guest ctx mapping stays in the ledger (and `falcon_ctx` in `pt`): a later alloc
            // on the same channel finds the row gone, and used to run the host alloc unsteered —
            // the vvid wrong-frames bug. So with no row the ctx RANGE `[va, va+len)` is handed over
            // all the same: `Free` is the proof that nothing of ours covers it (no mapping, no
            // reservation), and only then does the host alloc go ahead.
            let row_len = match steered {
                Some((_, l)) => l,
                None => match rows.write().map(|mut r| r.remove(&va)) {
                    Ok(removed) => steer_len(removed.map(|row| row.0), len),
                    Err(_) => {
                        return Err((
                            NV_ERR_INVALID_STATE,
                            format!(
                                "guest ctx VA {va:#x}: the placement rows are poisoned — the steer cannot be known to have happened; object birth REFUSED"
                            ),
                        ));
                    }
                },
            };
            {
                let rl = row_len;
                // ★ 2026-10-09: by RANGE over the whole row — outside a reservation a row is many host
                // mappings (D1: one per 4 KiB page, or one reservation holding it), so a start-keyed
                // unmap would take only its first. THROUGH the space's ownership ledger
                // (`BatchedVas::hand_to_host`): our mappings only, each through the hDma it was mapped
                // through, the ledger cut. D2: the ledger's locks are taken here (act thread) and held
                // for at most one chunk; the observed bounds are part of the log line.
                let Some(bv) = ledger.as_ref() else {
                    steer_msg = format!(
                        " [guest ctx VA {va:#x}: no ownership ledger for this space — nothing of ours to hand over, host ctx placement unsteered]"
                    );
                    return self.finish(me, steer_msg);
                };
                // One act's hand-over is bounded by a time slice (a hostile row is thousands of
                // hulls); the rest continues in a later act.
                let over = bv.hand_to_host_within(va, rl, kf_mem::batch::STEER_SLICE);
                let (touched, hold_us) = bv.hold_stats();
                let (act_wait_us, act_op_us) = bv.act_stats();
                let holds = format!(
                    " (ledger lock holds: at most {touched} entries, longest {hold_us} us; this thread waited at most {act_wait_us} us for a ledger lock, hand-over took at most {act_op_us} us; try {})",
                    tries.saturating_add(1)
                );
                match kf_mem::batch::steer_step(&over, tries, born.elapsed()) {
                    kf_mem::batch::SteerStep::Done => {
                        steer_msg = format!(
                            " [host ctx steered onto the guest's ctx VA {va:#x}+{len:#x}{holds}]"
                        );
                    }
                    kf_mem::batch::SteerStep::Retry => {
                        eprintln!(
                            "kf3: chan {client:#x}:{parent:#x} falcon ctx steer of {va:#x}+{rl:#x} not complete ({over:?}){holds} — re-queued, the host alloc waits"
                        );
                        let next = self.clone().act(
                            tries.saturating_add(1),
                            Some(born),
                            Some((va, rl)),
                            d.clone(),
                        );
                        // PARKED, not slept on: the act thread runs other acts until it is due.
                        let due = std::time::Instant::now()
                            .checked_add(kf_mem::batch::STEER_RETRY_SPACING)
                            .unwrap_or_else(std::time::Instant::now);
                        delay_act(due, "engine object", next, d);
                        return Err((ACT_REQUEUED, "steer retry".into()));
                    }
                    kf_mem::batch::SteerStep::Refuse => {
                        return Err((
                            NV_ERR_INVALID_STATE,
                            format!(
                                "falcon ctx VA {va:#x}+{rl:#x} could not be handed to host RM after {} tries / {} ms: {over:?}{holds} — object birth REFUSED (a host ctx placed beside the guest's would land on a live guest buffer: wrong frames)",
                                tries.saturating_add(1),
                                born.elapsed().as_millis()
                            ),
                        ));
                    }
                }
            }
        }
        self.finish(me, steer_msg)
    }

    /// The host alloc and the bookkeeping, after the steer is done (or not needed).
    fn finish(&self, me: &ChanPlane, steer: String) -> Result<String, (u32, String)> {
        let Self {
            client,
            parent,
            handle,
            class,
            copy_engine,
            chan,
            engine,
            kind,
            ..
        } = *self;
        let h = kf_chan::passthrough::engine_object(me.rm, chan, engine, class, kind, copy_engine)
            .map_err(|e| (NV_ERR_INVALID_CLASS, e))?;
        if let Ok(mut m) = me.pt.lock()
            && let Some(v) = m.get_mut(&(client, parent))
        {
            v.objects.insert(handle, (h, kind));
        }
        if let Ok(mut m) = me.pt_objs.lock() {
            m.insert((client, handle), (client, parent));
        }
        Ok(format!(
            "{client:#x}:{handle:#x} class {class:#x} ({kind:?}) on twin host {:#x} -> host object {h:#x}{steer}",
            chan.token
        ))
    }
}

/// ★ P5b: a guest USER channel's host twin (Passthrough).
struct PtChan {
    chan: kf_host::Channel,
    /// The guest token (table index = the guest's chid).
    idx: u32,
    /// The channel group it was allocated under.
    tsg: Option<u32>,
    /// ★ v3-int: its declared `hContextShare` — with `tsg`, the host group it joined.
    ctx_share: u32,
    /// The channel's parent (its TSG, or its device).
    parent: u32,
    /// ★ v3-promote: the device it hangs off (== `parent` outside a TSG).
    device: u32,
    /// The host engine (= the guest's `engineType`).
    engine: u32,
    /// Engine objects on the twin: guest handle → (host handle, its class kind).
    objects: HashMap<u32, (u32, kf_chip::classes::Kind)>,
    /// ★ EXPERIMENT `x11-dispsw`: the guest's `GF100_DISP_SW` objects on this channel, twinned on
    /// the twin: guest handle → host handle. Always empty with the switch off. Host RM frees them
    /// with the twin's channel (`HostRm::free` forgets the subtree), so they go with the twin.
    disp_sw: HashMap<u32, u32>,
    /// ★ EXPERIMENT `x11-dispsw` (review 2026-10-03, MEDIUM): the guest's FIFO software-classID
    /// numbering on this channel, mirrored ([`crate::dispsw::SwClassIds`]) — what each display-SW
    /// twin must carry so the guest's `SET_OBJECT` names it. Never advanced with the switch off.
    sw_ids: crate::dispsw::SwClassIds,
    /// ★ EXPERIMENT `x11-dispsw` (review 2026-10-03, LOW): a host display-SW alloc on this twin was
    /// refused, so host CPU-RM's numbering (what the readback asks) may be ahead of host GSP's (what
    /// the guest's `SET_OBJECT` meets) — every later display-SW alloc here is refused by name
    /// ([`crate::dispsw::twin_watched`]). Set once, never cleared: it goes with the twin.
    dispsw_host_refused: bool,
    /// ⚠ EXPERIMENT (2026-10-08, `KF3_WIN_TWIN_DEFAPI_OBJECT=1`, default off; [`twin_defapi_object`]):
    /// the host `NV50_DEFERRED_API` authored on this Windows user-work twin from the guest's own
    /// alloc of that class on the channel — (guest handle, host handle). At most one per twin; freed
    /// by host RM with the twin's channel.
    defapi_host: Option<(u32, u32)>,
    /// ★ v3-promote: the guest's `GPU_PROMOTE_CTX` / `GPU_EVICT_CTX` statements for this channel,
    /// satisfied by the twin (never forwarded).
    ctx: CtxBind,
    /// ★ 2026-10-08: a Windows guest-kernel channel classified per-process USER work at its alloc
    /// (`KF3_WIN_USER_CHANNELS_PASSTHROUGH`); fixed for the twin's life.
    user_work: bool,
    /// ★ P5c: the mirror's live-channel count (released at free).
    live: Arc<AtomicU64>,
    /// ★ P1+P2 inc D: the twin's state word, when this channel was counted as a user of it
    /// (T-mode) — released at free.
    twin: Option<Arc<crate::twin::TwinState>>,
    /// ★ P5c: the guest's error notifier, as the twin's host error context — `None` when the
    /// guest declared none (or it could not be armed, named at birth).
    notifier: Option<PtNotifier>,
    /// ★ v3-chanctl: the guest STOPPED the channel (host: disabled + off the runlist) — its next
    /// `GPFIFO_SCHEDULE(enable)` re-enables the twin first.
    stopped: bool,
    /// ★ v3-chanctl: the guest DISABLED it (`DISABLE_CHANNELS`) — only `bDisable=FALSE` undoes it.
    disabled: bool,
    /// ★ v3-video: the twin's host VA space (the mirror of the guest's) and its placement rows.
    space: kf_host::VaSpace,
    rows: crate::mem::PlacedRows,
    /// ★ Review fix 2026-10-10 (finding 6): the space's ownership ledger ([`crate::mem::Mirror::ledger`]).
    ledger: Option<Arc<kf_mem::batch::BatchedVas<'static>>>,
    /// ★ v3-video: the guest's falcon context buffer `(VA, size)` from its falcon promote — the VA
    /// the host's own falcon context is steered onto (see `ChanPlane::engine_object`).
    falcon_ctx: Option<(u64, u64)>,
    /// ⚠ DIAGNOSTIC (2026-10-09, [`ChanPlane::pt_stall_snapshot_poll`]): where the guest declared
    /// this channel's ring — `(GPFIFO VA, entries, USERD GPA if in guest RAM)`. Read only by the
    /// default-off stall snapshot; never used to act.
    ring_at: (u64, u32, Option<u64>),
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

/// ★ 2026-10-08 (`KF3_ASYNC_PREEMPT`): at most this many completed async preempts wait for the
/// drainer (a guest cannot grow a host allocation by preempting in a loop).
const PREEMPT_DONE_MAX: usize = 64;

/// ★★ 2026-10-08 (after run84) — **a `RUNLIST_PREEMPT_COMPLETE` goes out only after its control's
/// reply.** The GSP answers the async `DISABLE_CHANNELS` first and posts 139 after it, every time
/// (`[measured, vfio-10]` RPCs 6628-6633 and 8100-8126: reply, then event). kayfabe's act pushes the
/// completion when the host verb returns, BEFORE its deferred reply settles; a drainer pass between
/// the two posted the event first (`[measured, run84 at 4d733007]`: the first preempt's event
/// preceded its reply). The drainer that posts is the one that holds replies, so "no reply held"
/// (`held_replies == 0`, read under the GSP lock) means this act's reply is already in the queue.
#[must_use]
pub fn preempt_posts_ready(held_replies: usize) -> bool {
    held_replies == 0
}

/// What a `DISABLE_CHANNELS` list resolves to in THIS VM's plane: twins (`pt`), Translated host rings
/// (`tr`, by host token) and entries naming nothing here (`unknown`).
#[derive(Debug, PartialEq, Eq)]
struct DisableList<T> {
    pt: Vec<((u32, u32), T)>,
    tr: Vec<((u32, u32), u32)>,
    unknown: Vec<(u32, u32)>,
}

/// ★ v3-chanctl (pure since 2026-10-08) — resolve each `(hClient, hChannel)` entry: a twin by its own
/// handle; else a Translated ring; else (★ run82) a CHANNEL GROUP handle — every twin of that client
/// allocated under it (Windows' kernel preempts a process's TSG by its handle); else unknown. Only this
/// VM's maps are consulted, so a handle of another VM (or a freed group) is `unknown`, which the caller
/// refuses whole (nothing half-done).
fn resolve_disable_list<T>(
    list: &[(u32, u32)],
    twin: impl Fn(u32, u32) -> Option<T>,
    group: impl Fn(u32, u32) -> Vec<((u32, u32), T)>,
    translated: impl Fn(u32, u32) -> Option<u32>,
) -> DisableList<T> {
    let mut out = DisableList {
        pt: Vec::new(),
        tr: Vec::new(),
        unknown: Vec::new(),
    };
    for &(c, h) in list {
        if let Some(t) = twin(c, h) {
            out.pt.push(((c, h), t));
        } else if let Some(ht) = translated(c, h) {
            out.tr.push(((c, h), ht));
        } else {
            let members = group(c, h);
            if members.is_empty() {
                out.unknown.push((c, h));
            } else {
                out.pt.extend(members);
            }
        }
    }
    out
}

/// `ROBUST_CHANNEL_PREEMPTIVE_REMOVAL` (`ogkm-580: nverror.h:61`).
const ROBUST_CHANNEL_PREEMPTIVE_REMOVAL: u32 = 45;

impl PtNotifier {
    fn words(&self) -> Option<[u32; 4]> {
        let mut w = [0u32; 4];
        for (i, x) in w.iter_mut().enumerate() {
            *x = self.view.load((i as u64).saturating_mul(4)).ok()?;
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
    /// ★ 2026-10-08 (coordinator's first step on H-ce-intr): wakes NOT raised because no guest twin
    /// was live on this engine (`live == 0`)…
    pub unraised_no_live: AtomicU64,
    /// …and because the served table gives this engine no guest vector (`vector == None`).
    pub unraised_no_vector: AtomicU64,
    /// ★ GR tier (2026-10-07): live Translated kernel-GR rings on this engine whose completions
    /// are the guest's work (their fence tail's `NON_STALL_INTERRUPT` follows the engine's own
    /// writes of the guest's semaphores and `GP_GET`).
    pub tlive: AtomicU64,
    /// Wakes relayed to the guest while a Translated GR-tier ring was live.
    pub trelays: AtomicU64,
    /// ★ 2026-10-07 (`KF3_TRANSLATED_CE_RELAY`): Translated copy-engine pumps on this (guest)
    /// engine that found entries retired after their host fence, not yet relayed.
    pub cpending: AtomicU64,
    /// Relays raised on this engine's guest vector for Translated copy-engine work.
    pub crelays: AtomicU64,
    /// ★ 2026-10-08 (owner ruling §X, `kf_chan::ptnsi`): the guest's non-stall subscription slot
    /// for this engine (`kf_abi::eventnotify::nonstall_slot_*`; `None`: no such row).
    pub slot: Option<usize>,
    /// Host notifier wakes on this engine whose event the guest had not armed (nothing raised).
    pub not_armed: AtomicU64,
    /// ★ 2026-10-09: an edge of THIS engine is in a paced raise still owed (credited to `raised`
    /// when the tick delivers it). Written by the one worker thread only.
    pub owed_late: std::sync::atomic::AtomicBool,
}

/// A CE class id on ANY family — the class tables are generated per family and class ids are
/// unique across them, so this needs no family argument (the rewriter takes a plain `fn`).
fn is_any_ce_class(c: u32) -> bool {
    kf_chip::is_any_dma_copy_class(c)
}

/// A copy engine — `kf_chan::passthrough::is_copy_engine` (both of the header's blocks; the P5
/// copy of this read `0x28..` for COPY10, which is not a copy engine).
fn is_copy_engine(engine_type: u32) -> bool {
    kf_chan::passthrough::is_copy_engine(engine_type)
}

/// ★ Q7: which unforgeable fact made a channel the guest kernel's (for the birth log).
fn kernel_by(a: &ChannelAlloc) -> String {
    let stamp = match a.privilege {
        Some(p) if p.is_kernel() => format!(
            "PRIVILEGE=KERNEL{}",
            if p.uvm_owned { "+UVM_OWNED" } else { "" }
        ),
        Some(p) => format!(
            "privilege={}{}",
            p.level,
            if p.uvm_owned { "+uvm_owned" } else { "" }
        ),
        None => "privilege=undecoded".into(),
    };
    let internal = if kf_rm::chanlink::is_rm_internal_client(a.client) {
        " rm-internal"
    } else {
        ""
    };
    format!("{stamp}{internal}")
}

/// ★ The guest's USERD: two 4-byte cursors, reached through a CPU view WE armed at birth (vidmem)
/// or through guest RAM (sysmem). ⊘ Four bytes each way — `CPU_MOVE_MAX_BYTES`, never data.
enum UserdView {
    /// A 4 KiB CPU view of the store page holding USERD (`node` keeps the view's context alive).
    Store {
        region: VolatileRegion,
        _node: kf_linux_raw::CharDevice,
        cookie: u64,
        at: u64,
    },
    /// Guest RAM.
    Ram { mem: RawRegion, at: usize },
}

impl UserdView {
    fn load(&self, off: u64) -> Result<u32, String> {
        match self {
            UserdView::Store { region, at, .. } => region
                .load_u32(HostOffset::new(at.saturating_add(off)))
                .map_err(|e| format!("{e:?}")),
            UserdView::Ram { mem, at } => mem
                .load_u32(at.saturating_add(off as usize))
                .ok_or_else(|| "guest-RAM USERD load".to_string()),
        }
    }
    fn store(&self, off: u64, v: u32) -> Result<(), String> {
        match self {
            UserdView::Store { region, at, .. } => region
                .store_u32(HostOffset::new(at.saturating_add(off)), v)
                .map_err(|e| format!("{e:?}")),
            UserdView::Ram { mem, at } => mem
                .store_u32(at.saturating_add(off as usize), v)
                .then_some(())
                .ok_or_else(|| "guest-RAM USERD store".to_string()),
        }
    }
}

/// ★ v3-initrace: the physical-RM initialisation of a Translated channel's USERD
/// (`kf_chan::host::zero_userd`). ⊘ A store view covers only the 4 KiB page holding USERD, so a
/// word past that page is refused by name rather than wrapped.
impl kf_chan::host::UserdInit for UserdView {
    fn store_u32(&mut self, off: u64, v: u32) -> Result<(), String> {
        if let UserdView::Store { at, .. } = self
            && at.saturating_add(off).saturating_add(4) > 0x1000
        {
            return Err(format!("USERD +{off:#x} is past the page its view maps"));
        }
        self.store(off, v)
    }
}

/// ★★ 2026-10-08 (`docs/design/V3_USERD_RELAY.md`, `KF3_WIN_USER_CHANNELS_PASSTHROUGH`): a Passthrough
/// twin whose USERD the host cannot adopt (a Windows per-process channel's USERD is a guest-RAM slot at
/// an IOVA wider than a USERD may be). The twin's USERD is a kayfabe-owned video-memory object; a
/// worker relays `GP_PUT` in and `GP_GET` out, four bytes each, never a GP entry or push-buffer word.
struct Relay {
    st: kf_chan::userd_relay::RelayState,
    /// The guest's own USERD slot (guest RAM).
    guest: UserdView,
    /// kayfabe's USERD object (4 KiB, in no GPU VA space), its CPU view, and the view's release
    /// cookie.
    host: VolatileRegion,
    _node: kf_linux_raw::CharDevice,
    cookie: u64,
    mem: u32,
    /// The twin.
    chan: kf_host::Channel,
    /// The guest token (for the log).
    idx: u32,
    /// ⚠ DIAGNOSTIC only (`KF3_RELAY_PB_PEEK=1`, default off): the guest VA space's mirror, the
    /// ring's VA, and how many GP entries were peeked — see [`ChanPlane::relay_peek`].
    mirror: Mirror,
    views: StoreViews,
    gpfifo_va: u64,
    peek: PeekState,
}

/// ⚠ DIAGNOSTIC only (`KF3_RELAY_PB_PEEK=1`): the peek's cursor and bounds for one relayed twin.
#[derive(Default)]
struct PeekState {
    /// The next GP index to peek.
    next: u32,
    /// GP entries peeked so far (bounded by [`PEEK_ENTRY_BUDGET`]).
    entries: u32,
    /// Software-subchannel methods logged so far (bounded by [`PEEK_SW_BUDGET`]).
    sw_logged: u32,
    /// The previous batch's raw entries, re-read at the next step: a change means the guest wrote
    /// an entry AFTER ringing for it.
    prev: Vec<(u32, [u8; 8])>,
    /// ⚠ (after run76) the last one-word semaphore release peeked — (engine form, VA, payload, GP
    /// index) — compared with memory at the twin's release ([`last_fence_release`]).
    fence: Option<(&'static str, u64, u32, u32)>,
}

/// Per relayed twin: at most this many GP entries are peeked.
const PEEK_ENTRY_BUDGET: u32 = 512;
/// Per relayed twin: at most this many software-subchannel methods are logged.
const PEEK_SW_BUDGET: u32 = 64;
/// Per segment: at most this many words are read (and scanned); at most 64 are printed.
const PEEK_SEGMENT_WORDS: usize = 16384;

/// ⚠ DIAGNOSTIC (2026-10-08, run73): one method a push-buffer segment addresses to a SOFTWARE
/// subchannel. `[NVIDIA, open-gpu-doc ga100 dev_ram.ref "Subchannels 5-7 are for software methods"
/// and dev_pbdma.ref NV_PPBDMA_INTR_0_DEVICE]`: any method on subchannels 5-7, SetObject included,
/// is kicked back to software through the PBDMA's DEVICE interrupt; Host-only methods (the
/// `NV_UDMA` range below `0x100`, SetObject excepted) ignore the subchannel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SwMethod {
    /// Index of the header word in the segment.
    word: usize,
    subch: u32,
    /// Byte method address.
    method: u32,
    /// The first datum (the header's own for an immediate), if it was read.
    data: Option<u32>,
}

/// What [`scan_sw_methods`] found in one segment.
#[derive(Debug, Default, PartialEq, Eq)]
struct SwScan {
    hits: Vec<SwMethod>,
    /// Method headers seen, per subchannel.
    per_subch: [u32; 8],
    /// A header word with no defined format stopped the scan here (`method_header_decode` = None).
    undecodable_at: Option<usize>,
}

/// Walk a segment's words header by header (bounded by `words`) and collect every method on a
/// software subchannel. Pure; never follows anything outside `words`.
fn scan_sw_methods(words: &[u32]) -> SwScan {
    let mut out = SwScan::default();
    let mut i = 0usize;
    while let Some(&word) = words.get(i) {
        let Some(h) = kf_abi::submit::method_header_decode(word) else {
            out.undecodable_at = Some(i);
            break;
        };
        let counted = !matches!(
            h.form,
            kf_abi::submit::MethodForm::EndPbSegment | kf_abi::submit::MethodForm::SubDeviceMask
        );
        if counted && (h.arg_words > 0 || h.form == kf_abi::submit::MethodForm::Immediate) {
            if let Some(n) = out.per_subch.get_mut(h.subchannel as usize) {
                *n = n.saturating_add(1);
            }
            let host_only = h.method != kf_abi::submit::SET_OBJECT && h.method < 0x100;
            if h.subchannel >= 5 && !host_only {
                out.hits.push(SwMethod {
                    word: i,
                    subch: h.subchannel,
                    method: h.method,
                    data: if h.form == kf_abi::submit::MethodForm::Immediate {
                        Some(h.immd)
                    } else {
                        words.get(i.saturating_add(1)).copied()
                    },
                });
            }
        }
        if h.form == kf_abi::submit::MethodForm::EndPbSegment {
            break;
        }
        i = i.saturating_add(1).saturating_add(h.arg_words);
    }
    out
}

/// ⚠ DIAGNOSTIC switch (default off, 2026-10-09): `KF3_PT_STALL_SNAPSHOT=1` arms the stall snapshot
/// ([`ChanPlane::pt_stall_snapshot_poll`]); `KF3_PT_STALL_SNAPSHOT_MS` the silence (200..=10000,
/// default 1000). `None` = off.
fn pt_stall_snapshot_ms() -> Option<u64> {
    static ON: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        if std::env::var("KF3_PT_STALL_SNAPSHOT").as_deref() != Ok("1") {
            return None;
        }
        let ms = std::env::var("KF3_PT_STALL_SNAPSHOT_MS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(1000)
            .clamp(200, 10_000);
        eprintln!(
            "kf3: ⚠ DIAGNOSTIC KF3_PT_STALL_SNAPSHOT=1: after {ms} ms with no doorbell to any live Passthrough twin (and at each twin's free), its USERD, ring entries, segment methods and semaphores are read from guest memory and logged (at most {SNAP_MAX} stalls, {SNAP_FREE_MAX} frees per boot)"
        );
        Some(ms)
    })
}

/// Stall snapshots per boot (one per silence).
const SNAP_MAX: u32 = 4;
/// Snapshots at a twin's free per boot.
const SNAP_FREE_MAX: u32 = 32;
/// GPFIFO entries listed per twin.
const SNAP_ENTRIES: u32 = 12;
/// Entries before `GPPut` whose segments are decoded.
const SNAP_SEGMENTS: u32 = 3;
/// Words read per segment.
const SNAP_WORDS: usize = 256;
/// Methods listed per segment.
const SNAP_METHODS: usize = 48;
/// Semaphores read back per twin.
const SNAP_SEMS: usize = 8;

/// ⚠ DIAGNOSTIC: a semaphore a segment names — host `SEM_EXECUTE` (`NVC56F_SEM_*`,
/// `ogkm-595.84: clc56f.h:206-229`), or the 3D/CE release [`last_fence_release`] finds.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SemRef {
    op: &'static str,
    va: u64,
    payload: u64,
    wide: bool,
    /// The GPFIFO entry whose segment named it (`u32::MAX` until the caller sets it).
    gp: u32,
}

/// ⚠ DIAGNOSTIC, pure: a segment's methods as `(subchannel, method byte offset, value)` (at most
/// `max`), and its host semaphore operations. Host methods (`< 0x100`) apply on every subchannel;
/// `SEM_ADDR_LO/HI` (0x5c/0x60), `SEM_PAYLOAD_LO/HI` (0x64/0x68) latch, `SEM_EXECUTE` (0x6c)
/// executes: OPERATION `2:0`, PAYLOAD_SIZE `24:24` (`clc56f.h:206-229`). Stops at the first
/// undecodable header (named) or the segment's end.
struct SnapDecoded {
    methods: Vec<(u32, u32, u32)>,
    sems: Vec<SemRef>,
    stopped_at: Option<usize>,
}

fn snap_decode(words: &[u32], max: usize) -> SnapDecoded {
    use kf_abi::submit::{MethodForm, method_header_decode};
    const OPS: [&str; 8] = [
        "ACQUIRE",
        "RELEASE",
        "ACQ_STRICT_GEQ",
        "ACQ_CIRC_GEQ",
        "ACQ_AND",
        "ACQ_NOR",
        "REDUCTION",
        "OP7",
    ];
    let mut d = SnapDecoded {
        methods: Vec::new(),
        sems: Vec::new(),
        stopped_at: None,
    };
    let (mut lo, mut hi, mut plo, mut phi) = (0u32, 0u32, 0u32, 0u32);
    let mut i = 0usize;
    while let Some(&word) = words.get(i) {
        let Some(h) = method_header_decode(word) else {
            d.stopped_at = Some(i);
            break;
        };
        let args: Vec<(u32, u32)> = match h.form {
            MethodForm::EndPbSegment => break,
            MethodForm::Immediate => vec![(h.method, h.immd)],
            MethodForm::Incrementing => (0..h.arg_words)
                .filter_map(|k| {
                    words
                        .get(i.saturating_add(1).saturating_add(k))
                        .map(|v| (h.method.saturating_add((k as u32).saturating_mul(4)), *v))
                })
                .collect(),
            MethodForm::NonIncrementing => (0..h.arg_words)
                .filter_map(|k| {
                    words
                        .get(i.saturating_add(1).saturating_add(k))
                        .map(|v| (h.method, *v))
                })
                .collect(),
            MethodForm::IncrementOnce => (0..h.arg_words)
                .filter_map(|k| {
                    words.get(i.saturating_add(1).saturating_add(k)).map(|v| {
                        (
                            if k == 0 {
                                h.method
                            } else {
                                h.method.saturating_add(4)
                            },
                            *v,
                        )
                    })
                })
                .collect(),
            _ => Vec::new(),
        };
        for (m, v) in args {
            if d.methods.len() < max {
                d.methods.push((h.subchannel, m, v));
            }
            match m {
                0x5c => lo = v,
                0x60 => hi = v,
                0x64 => plo = v,
                0x68 => phi = v,
                0x6c => {
                    let wide = (v >> 24) & 1 == 1;
                    d.sems.push(SemRef {
                        op: OPS.get((v & 7) as usize).copied().unwrap_or("?"),
                        va: (u64::from(hi & 0xff) << 32) | u64::from(lo & !3),
                        payload: if wide {
                            (u64::from(phi) << 32) | u64::from(plo)
                        } else {
                            u64::from(plo)
                        },
                        wide,
                        gp: u32::MAX,
                    });
                }
                _ => {}
            }
        }
        i = i.saturating_add(1).saturating_add(h.arg_words);
    }
    d
}

/// ⚠ DIAGNOSTIC (2026-10-08, after run76; `KF3_RELAY_PB_PEEK=1` only): the LAST one-word semaphore
/// RELEASE a peeked segment asks the engine for — `(engine, VA, payload)` — so the twin's release can
/// compare the payload with what the memory holds (did the engine write its last fence?). Two
/// incrementing forms, the ones Windows' per-process streams use `[measured, run76 at f649d2c3]`:
/// the 3D/compute `SET_REPORT_SEMAPHORE_A..D` (`0x1b00`, 4 words; `D.OPERATION` 1:0 = RELEASE and
/// `D.STRUCTURE_SIZE` 28 = ONE_WORD; `ogkm-580: clc797.h`), and the copy engine's
/// `SET_SEMAPHORE_A/B` + `SET_SEMAPHORE_PAYLOAD` (`0x240`, 3 words; `clc7b5.h`). Pure; bounded by
/// `words`; anything else is skipped.
fn last_fence_release(words: &[u32]) -> Option<(&'static str, u64, u32)> {
    let mut out = None;
    let mut i = 0usize;
    while let Some(&word) = words.get(i) {
        let Some(h) = kf_abi::submit::method_header_decode(word) else {
            break;
        };
        let args = words.get(i.saturating_add(1)..i.saturating_add(1).saturating_add(h.arg_words));
        if h.form == kf_abi::submit::MethodForm::Incrementing
            && let Some(a) = args
        {
            match (h.method, a) {
                (0x1b00, &[a0, a1, a2, a3]) if a3 & 3 == 0 && a3 & (1 << 28) != 0 => {
                    out = Some(("3d", (u64::from(a0 & 0xff) << 32) | u64::from(a1), a2));
                }
                (0x240, &[a0, a1, a2]) => {
                    out = Some(("ce", (u64::from(a0 & 0x1ffff) << 32) | u64::from(a1), a2));
                }
                _ => {}
            }
        }
        if h.form == kf_abi::submit::MethodForm::EndPbSegment {
            break;
        }
        i = i.saturating_add(1).saturating_add(h.arg_words);
    }
    out
}

/// What an `NV50_DEFERRED_API` alloc on a Passthrough channel becomes (the experiment's decision).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TwinDefapiPlan {
    /// The pre-experiment behaviour: a guest-graph node only (Windows user work), or §U.1's path.
    NoHostObject,
    /// ⚠ `KF3_WIN_TWIN_DEFAPI_OBJECT=1` on a Windows user-work twin: author one host object.
    AuthorHostObject,
}

/// Pure: the experiment applies only with its switch on AND on a Windows user-work twin; a Linux
/// Passthrough channel or a Translated one never gets a host `NV50_DEFERRED_API` from it.
fn twin_defapi_plan(switch_on: bool, user_work: bool) -> TwinDefapiPlan {
    if switch_on && user_work {
        TwinDefapiPlan::AuthorHostObject
    } else {
        TwinDefapiPlan::NoHostObject
    }
}

/// ⚠ EXPERIMENT switch (default off): [`ChanPlane::twin_defapi_act`].
fn twin_defapi_object() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("KF3_WIN_TWIN_DEFAPI_OBJECT").as_deref() == Ok("1"))
}

/// The I/O of one relay step ([`kf_chan::userd_relay::step`]).
struct RelayMem<'a> {
    guest: &'a UserdView,
    host: &'a VolatileRegion,
    rm: &'a HostRm,
    token: u32,
}

impl kf_chan::userd_relay::RelayIo for RelayMem<'_> {
    fn guest_put(&mut self) -> Result<u32, String> {
        self.guest.load(kf_abi::submit::USERD_GP_PUT)
    }
    fn store_host_put(&mut self, put: u32) -> Result<(), String> {
        self.host
            .store_u32(HostOffset::new(kf_abi::submit::USERD_GP_PUT), put)
            .map_err(|e| format!("{e:?}"))
    }
    fn fence(&mut self) {
        kf_linux_raw::release_fence();
    }
    fn ring(&mut self) -> Result<(), String> {
        self.rm
            .doorbell(self.token)
            .map_err(|e| format!("doorbell: {e:?}"))
    }
    fn host_get(&mut self) -> Result<u32, String> {
        self.host
            .load_u32(HostOffset::new(kf_abi::submit::USERD_GP_GET))
            .map_err(|e| format!("{e:?}"))
    }
    fn store_guest_get(&mut self, get: u32) -> Result<(), String> {
        self.guest.store(kf_abi::submit::USERD_GP_GET, get)
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

/// The 64 KiB-aligned store slice a CPU view of `at` covers, bounded by `bound` — `Err` when `at`
/// lies at or past the bound.
fn view_span(bound: u64, at: u64) -> Result<(u64, u64), String> {
    let off = at & !(VIEW_BYTES - 1);
    let len = VIEW_BYTES.min(bound.saturating_sub(off));
    if len == 0 || at >= bound {
        return Err(format!(
            "store offset {at:#x} is past the store ({bound:#x})"
        ));
    }
    Ok((off, len))
}

/// `(a - b) mod n` on a ring of `n` entries (`a`, `b` taken mod `n` first; `n == 0` is a ring of
/// one). The stall snapshot's GP indices come from the guest's USERD: no `%`, `-` or `+` on them
/// may panic.
fn ring_dist(a: u32, b: u32, n: u32) -> u32 {
    let n = u64::from(n.max(1));
    let (a, b) = (
        u64::from(a).checked_rem(n).unwrap_or(0),
        u64::from(b).checked_rem(n).unwrap_or(0),
    );
    u32::try_from(
        a.saturating_add(n)
            .saturating_sub(b)
            .checked_rem(n)
            .unwrap_or(0),
    )
    .unwrap_or(0)
}

/// ★ Review addendum (hostile guest): the `[done, done + n)` window of a read buffer, or a named
/// error — a read loop's indices come from guest-derived row lengths and never index blindly.
fn out_window(out: &mut [u8], done: u64, n: u64) -> Result<&mut [u8], String> {
    let a = usize::try_from(done).map_err(|_| format!("read offset {done:#x} out of range"))?;
    let b = usize::try_from(done.saturating_add(n))
        .map_err(|_| format!("read end {done:#x}+{n:#x} out of range"))?;
    out.get_mut(a..b)
        .ok_or_else(|| format!("read window {done:#x}+{n:#x} is outside the buffer"))
}

impl StoreViews {
    const fn new() -> Self {
        StoreViews {
            views: Vec::new(),
            armed: 0,
        }
    }

    fn view_for(&mut self, rm: &HostRm, store: u32, fb_len: u64, at: u64) -> Result<usize, String> {
        if let Some(i) = self
            .views
            .iter()
            .position(|v| at >= v.off && at < v.off.saturating_add(v.len))
        {
            return Ok(i);
        }
        let (off, len) = view_span(fb_len, at)?;
        if self.views.len() >= VIEWS_MAX {
            let old = self.views.remove(0);
            let _ = rm.release_cpu_view(kf_host::CpuViewRelease {
                h_memory: store,
                p_linear_address: old.cookie,
            });
        }
        let (node, cookie) = rm
            .arm_cpu_view(MapNode::Gpu, store, off, len, ViewAccess::ReadWrite)
            .map_err(|e| format!("view of store {off:#x}+{len:#x}: {e:?}"))?;
        let region = VolatileRegion::map(
            Backing::DeviceFile { fd: node.as_fd() },
            len,
            CachePolicy::Uncached,
            HostPageSize::query(),
        )
        .map_err(|e| format!("view mmap: {e:?}"))?;
        self.armed = self.armed.saturating_add(1);
        self.views.push(StoreSpan {
            off,
            len,
            region,
            _node: node,
            cookie,
        });
        Ok(self.views.len().saturating_sub(1))
    }

    fn read(
        &mut self,
        rm: &HostRm,
        store: u32,
        fb_len: u64,
        off: u64,
        out: &mut [u8],
    ) -> Result<(), String> {
        let len = out.len() as u64;
        let mut done = 0u64;
        while done < len {
            let at = off.saturating_add(done);
            let i = self.view_for(rm, store, fb_len, at)?;
            let v = self
                .views
                .get(i)
                .ok_or_else(|| format!("store read {at:#x}: no view"))?;
            let n = v
                .off
                .saturating_add(v.len)
                .saturating_sub(at)
                .min(len.saturating_sub(done));
            if n == 0 {
                return Err(format!("store read {at:#x}: no progress"));
            }
            v.region
                .copy_out(
                    HostOffset::new(at.saturating_sub(v.off)),
                    out_window(out, done, n)?,
                )
                .map_err(|e| format!("store read {at:#x}: {e:?}"))?;
            done = done.saturating_add(n);
        }
        Ok(())
    }

    fn release_all(&mut self, rm: &HostRm, store: u32) {
        for v in self.views.drain(..) {
            let _ = rm.release_cpu_view(kf_host::CpuViewRelease {
                h_memory: store,
                p_linear_address: v.cookie,
            });
        }
    }
}

/// The guest's memory at a guest VA of the channel's space, through OUR placements.
struct Mem<'a> {
    mirror: &'a Mirror,
    ram: &'a RamMap,
    rm: &'a HostRm,
    store: u32,
    /// The bound of the CPU store views ([`store_read_bound`]).
    store_len: u64,
    views: &'a mut StoreViews,
    /// ★ Review fix 2026-10-04: the VA thread's inbox — whether a walk is pending
    /// ([`crate::mem::Inbox::walk_busy`]).
    inbox: &'a crate::mem::Inbox,
}

/// ★ P1+P2 review fix (HIGH, 2026-10-04) — **the bound on the CPU store views a Translated
/// channel's reads go through** (its GPFIFO entries and pushbuffer through vidmem rows, the
/// completion probe's read-backs). ⊘ Never the mirror's store-WINDOW length: a T-mode twin carries
/// no window and records `fb_len = 0`, so a bound taken from it made every vidmem fetch fail
/// "past the store (0x0)" — every UVM Translated channel died at its first fetch (UVM's GPFIFO is
/// vidmem on a dGPU, `ogkm-580: kernel-open/nvidia-uvm/uvm_channel.c:3386-3390`). The bound is the
/// carve-out base — a CPU read never touches kayfabe's firmware region either. ★ 2026-10-10: the
/// legacy arm (the mirror's window length) is deleted with `KF3_TSPACE`; a mirror has no window.
const fn store_read_bound(carve: u64) -> u64 {
    carve
}
/// ★ P1+P2 inc C (`docs/design/V3_P1P2_TSPACE.md` §3.4) — the T-mode resolver's view of a
/// mirror: OUR placement rows (never a copy of the guest's tables), each operand resolved under ONE
/// read guard, and guest RAM through the vIOMMU seam ([`RamMap::dma_to_file_range`]).
pub(crate) fn resolve_rows(
    rows: &crate::mem::PlacedRows,
    va: u64,
    len: u64,
) -> Result<Vec<kf_chan::tmode::Span>, u64> {
    let r = rows.read().map_err(|_| va)?;
    kf_chan::tmode::resolve_spans(va, len, |at| {
        let (&start, &(rlen, off, ram, perm)) = r.range(..=at).next_back()?;
        let end = start.checked_add(rlen)?;
        (at < end)
            .then(|| {
                off.checked_add(at.saturating_sub(start))
                    .map(|o| (ram, o, end.saturating_sub(at), perm))
            })
            .flatten()
    })
}

impl kf_chan::tmode::Rows for Mem<'_> {
    fn resolve(&self, va: u64, len: u64) -> Result<Vec<kf_chan::tmode::Span>, u64> {
        resolve_rows(&self.mirror.rows, va, len)
    }
    fn dma_to_file_range(&self, dma: u64, len: u64) -> Option<u64> {
        self.ram.dma_to_file_range(dma, len)
    }
    // ★ Review fix 2026-10-04 (§3.5, §7.13): the rows' commit log and the walk-pending signal.
    fn resolve_epoch(&self, va: u64, len: u64) -> (Result<Vec<kf_chan::tmode::Span>, u64>, u64) {
        crate::mem::resolve_rows_epoch(&self.mirror.rows, &self.mirror.log, va, len)
    }
    fn changed_since(&self, epoch: u64, va: u64, len: u64) -> kf_chan::tmode::Changed {
        self.mirror.log.changed_since(epoch, va, len)
    }
    fn walk_pending(&self) -> bool {
        self.inbox.walk_busy()
    }
}

impl GuestMemory for Mem<'_> {
    fn rows(&self) -> Option<&dyn kf_chan::tmode::Rows> {
        Some(self)
    }
    fn read(&mut self, va: u64, out: &mut [u8]) -> Result<(), String> {
        let len = out.len() as u64;
        // ★ P5b: piece by piece across OUR rows — a segment may span two adjacent placements.
        let mut done = 0u64;
        while done < len {
            let at_va = va.saturating_add(done);
            let (ram, off, avail) =
                resolve_placed_prefix(&self.mirror.rows, at_va).ok_or_else(|| {
                    format!(
                        "{va:#x}+{len:#x}: {at_va:#x} not placed by us ({})",
                        crate::mem::describe_neighbours(&self.mirror.rows, at_va)
                    )
                })?;
            let n = avail.min(len.saturating_sub(done));
            if n == 0 {
                return Err(format!("{at_va:#x}: no progress"));
            }
            if !ram {
                // ★ P6 (Q3): a vidmem GPFIFO / pushbuffer (UVM's default GPFIFO) is read through a
                // CPU view WE arm over the store slice our own row placed there.
                let dst = out_window(out, done, n)?;
                let t0 = crate::prof::on().then(crate::prof::now_ns);
                self.views
                    .read(self.rm, self.store, self.store_len, off, dst)
                    .map_err(|e| format!("{at_va:#x}: {e}"))?;
                crate::prof::VIEW_READS.fetch_add(1, Ordering::Relaxed);
                crate::prof::VIEW_READ_BYTES.fetch_add(n, Ordering::Relaxed);
                if let Some(t0) = t0 {
                    crate::prof::VIEW_READ_NS
                        .fetch_add(crate::prof::now_ns().saturating_sub(t0), Ordering::Relaxed);
                }
                done = done.saturating_add(n);
                continue;
            }
            let (mem, at) = self
                .ram
                .at_file_offset(off, n)
                .ok_or_else(|| format!("{at_va:#x}: guest-RAM offset {off:#x} unregistered"))?;
            let dst = out_window(out, done, n)?;
            if !mem.read_into(at, dst) {
                return Err(format!("{at_va:#x}: guest-RAM read"));
            }
            crate::prof::RAM_READ_BYTES.fetch_add(n, Ordering::Relaxed);
            done = done.saturating_add(n);
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
    /// ★ OWNER_RULINGS §U: the software methods of this channel.
    sw: SwCtx<'a>,
}

/// ★ OWNER_RULINGS §U: what a channel's pump needs to serve its software methods.
struct SwCtx<'a> {
    plane: &'a ChanPlane,
    /// The channel's host token (its software objects are keyed by it).
    ht: u32,
    /// The channel's own VA space (a deferred TLB invalidate walks exactly this).
    space: VasKey,
    st: &'a mut SwSlot,
}

/// ★ OWNER_RULINGS §U: an act's outcome, set by the act thread and taken by the channel's pump.
type SwCell = Arc<Mutex<Option<Result<String, String>>>>;

/// ★ OWNER_RULINGS §U: the software method a channel is stopped at — what was planned, and the
/// action's handle (a VA-thread ticket, or an act's outcome cell).
#[derive(Default)]
struct SwSlot {
    planned: Option<crate::defapi::Planned>,
    ticket: Option<u64>,
    cell: Option<SwCell>,
}
impl Publisher for VaSplit<'_> {
    fn sw_classify(
        &mut self,
        call: &kf_chan::tmode::SwCall,
    ) -> Result<kf_chan::swmethod::SwKind, String> {
        let p = self.sw.plane;
        let reg = p
            .defapi_reg
            .get()
            .ok_or("software method: the deferred-API tables are not attached")?;
        let objs = p
            .sw_objs
            .lock()
            .ok()
            .and_then(|m| m.get(&self.sw.ht).cloned());
        let (kind, planned) = crate::defapi::classify(objs.as_ref(), reg, call)?;
        self.sw.st.planned = planned;
        Ok(kind)
    }

    fn sw_start(
        &mut self,
        call: &kf_chan::tmode::SwCall,
        gate: Option<(kf_chan::host::Gate, u32)>,
    ) -> Result<(), String> {
        let planned = self
            .sw
            .st
            .planned
            .clone()
            .ok_or("software method started with nothing planned")?;
        let p = self.sw.plane;
        p.defapi_triggers.fetch_add(1, Ordering::Relaxed);
        eprintln!(
            "kf3: DEFERRED-API trigger token {:#x} subch {} value {:#x} method {:#x} hApiHandle {:#x} on {:#x}:{:#x} cmd {:#x}: {:?}",
            self.token,
            call.sub,
            call.value,
            call.method,
            call.data,
            planned.key.client,
            planned.key.object,
            planned.entry.cmd,
            planned.entry.decoded
        );
        if let kf_abi::defapi::Bundle::InvalidateTlb { vaspace } = planned.entry.decoded {
            let (g, payload) =
                gate.ok_or("deferred DMA_INVALIDATE_TLB started without its gate")?;
            // ★ §U.2: the guest's hClientVA/hDeviceVA/hVASpace are ignored: our OWN space.
            eprintln!(
                "kf3: DEFERRED-API token {:#x}: DMA_INVALIDATE_TLB (guest hVASpace {vaspace:#x} ignored) -> gate payload {payload} on the channel's own space {:?}",
                self.token, self.sw.space
            );
            self.sw.st.ticket =
                Some(
                    self.inbox
                        .request_gated_split(self.token, self.sw.space, g, payload),
                );
            return Ok(());
        }
        let cell: SwCell = Arc::default();
        self.sw.st.cell = Some(cell.clone());
        p.deferred_ctx_act(planned, cell, self.token)
    }

    fn sw_poll(&mut self, _call: &kf_chan::tmode::SwCall) -> Result<Split, String> {
        let st = &mut *self.sw.st;
        let Some(planned) = st.planned.clone() else {
            return Err("software method polled with nothing planned".into());
        };
        let outcome = if let Some(t) = st.ticket {
            match self.inbox.split_result(t) {
                None => return Ok(Split::Pending),
                Some(r) => r.map(|()| "gate released after the commit".to_string()),
            }
        } else if let Some(c) = &st.cell {
            match c.lock().ok().and_then(|mut g| g.take()) {
                None => return Ok(Split::Pending),
                Some(r) => r,
            }
        } else {
            return Err("software method polled before it was started".into());
        };
        *st = SwSlot::default();
        let tlb = matches!(
            planned.entry.decoded,
            kf_abi::defapi::Bundle::InvalidateTlb { .. }
        );
        if let Some(reg) = self.sw.plane.defapi_reg.get() {
            // The trigger's cleanup, whatever the outcome (`deferred_api.c:653-670`).
            reg.executed(planned.key, planned.entry.handle, tlb && outcome.is_ok());
        }
        match outcome {
            Ok(line) => {
                eprintln!(
                    "kf3: DEFERRED-API token {:#x} hApiHandle {:#x} DONE: {line}",
                    self.token, planned.entry.handle
                );
                Ok(Split::Done)
            }
            Err(e) => Err(format!(
                "deferred API hApiHandle {:#x} cmd {:#x}: {e}",
                planned.entry.handle, planned.entry.cmd
            )),
        }
    }

    fn invalidated(&mut self, pdb: Option<u64>) -> Result<Split, String> {
        let Some(t) = *self.ticket else {
            *self.ticket = Some(self.inbox.request_split(self.token, pdb));
            *self.requested = self.requested.saturating_add(1);
            return Ok(Split::Pending);
        };
        match self.inbox.split_result(t) {
            None => Ok(Split::Pending),
            Some(r) => {
                *self.ticket = None;
                r.map(|()| Split::Done)
                    .map_err(|e| format!("split walk (pdb {pdb:x?}): {e}"))
            }
        }
    }
}

/// ⊘ 2026-10-10 (`OWNER_RULINGS.md` §AB): **no mirror window.** The legacy rewriter translated
/// physical operands through the store and guest-RAM windows every mirror carried (deleted with
/// `KF3_TSPACE`); every Translated ring is now a T-mode ring, which binds its operands against the
/// T-space's windows at push (`kf_chan::tmode::push_bound`) and never consults this one
/// (`TranslatedRing::next` returns before the legacy rewrite). So the pump's `Window` argument
/// translates nothing.
struct NoMirrorWindow;
impl Window for NoMirrorWindow {
    fn translate(&self, _: Target, _: u64, _: u64) -> Option<u64> {
        None
    }
}

/// One guest kernel channel, Translated.
struct Slot {
    chan: TranslatedChannel,
    key: VasKey,
    mirror: Mirror,
    userd: UserdView,
    guest_idx: u32,
    guest_engine: u32,
    ctx: CtxBind,
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
    /// ★ v3-initrace: the completion probe's record (empty unless `KF3_COMPLETION_PROBE`).
    probe: ProbeRec,
    /// ★ P1+P2 inc A: the channel's count-only total already summed into
    /// [`ChanPlane::inca_counted`].
    inca_seen: u64,
    /// ★ GR tier (2026-10-07): born with the kernel-GR tier (its engine's `tlive` counts it).
    gr_tier: bool,
    /// The error notifier the guest declared at allocation (`errorNotifierMem`), kept for the
    /// default-off `KF3_BAR0_TRACE` dump only — a Translated ring arms no notifier.
    err_notifier: Option<kf_arch::fault::ErrorNotifier>,
    /// ★ OWNER_RULINGS §U: the software method the channel is stopped at.
    sw: SwSlot,
}

// Experiment on this branch: kernel GR channels run only authored CE work in
// the private T-space, with a real host-owned GR context. Default off.
// ★ 2026-10-10 (§AB, `KF3_TSPACE` hardwired): KEPT as a switch, deliberately. ON, it changes the
// route of a channel every Linux boot creates — the guest RM's internal kernel GR channel (the
// golden-image channel, today "not born") becomes a Translated ring with an owned host GR
// context — and no Linux run with it exists; ON without `KF3_KERNEL_GR_WORK` (which §S.2 keeps
// default-off "until proven") it is a ring that refuses every GR segment. The two are an owner
// ruling together; the Windows profile sets both.
fn kernel_gr_ce() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KF3_KERNEL_GR_CE").is_some_and(|v| v == "1"))
}

// ★ Owner rulings 2026-10-07 (`OWNER_RULINGS.md` §S items 1-6): the kernel-GR tier. With
// `KF3_KERNEL_GR_CE` and T-mode, a guest-kernel GR channel's host ring also holds host objects of
// the allowlisted graphics classes (`kf_chan::grtables`) after a USER assertion, its decoder
// re-authors their admitted methods, the ENGINE writes the guest's GP_GET, its completions wake
// the plane on GR0 and are relayed to the guest's GR0 vector. Default off.
fn kernel_gr_work() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KF3_KERNEL_GR_WORK").is_some_and(|v| v == "1"))
}

// ★ Owner ruling 2026-10-07 (§S item 4): on a T-mode Translated ring, a SET_OBJECT on a software
// subchannel (5-7) of a value no family lists as a class is accepted silently; any later software
// method there is refused by name. The hardware behaviour is INFERRED, not tested. Default off.
fn sw_subch_inert() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KF3_SW_SUBCH_INERT").is_some_and(|v| v == "1"))
}

// ★ OWNER_RULINGS §U (2026-10-07): on a T-mode Translated ring, a SET_OBJECT on a software
// subchannel names one of the channel's own software objects by number; methods there are served
// by the deferred-API path (class 5080: host-authored equivalents) or refused by name. Default off
// until native-validated; the class-5080 admission (Translated-only) and its registration controls
// are served regardless.
fn deferred_api_trigger() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KF3_DEFERRED_API").is_some_and(|v| v == "1"))
}

// ★ 2026-10-07 (hypothesis 2 after run34; `OWNER_RULINGS.md` §S item 7): a Translated
// COPY-ENGINE ring's completion is relayed to the guest's vector of the ring's own guest engine
// (CEn), exactly as the GR tier relays GR0: only when the pump found entries retired after their
// HOST fence was reached (the guest's GP_GET and semaphores are already written), on the worker,
// never on a vCPU or under a lock a vCPU takes. Default off.
fn translated_ce_relay() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KF3_TRANSLATED_CE_RELAY").is_some_and(|v| v == "1"))
}

// ★ 2026-10-08 (`V3_USERD_RELAY.md` §2.2's follow-up; H-getget after run76): a relayed twin's
// engine-written GP_GET is also written into the guest's slot on every host non-stall wake (before the
// guest's interrupt) and on the worker's park tick — not only at a doorbell. Host-derived only; never
// rings, never reads GP_PUT. Default off (one variable per run).
fn relay_get_refresh() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KF3_RELAY_GET_REFRESH").is_some_and(|v| v == "1"))
}

// ⚠ CONTROL (2026-10-08, owner decision): `KF3_USERD_RELAY_OFF=1` births Windows user-work twins over
// the guest's own sysmem USERD (adoption, no relay). Default off.
fn userd_relay_off() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KF3_USERD_RELAY_OFF").is_some_and(|v| v == "1"))
}

// ⚠ DIAGNOSTIC (2026-10-08, after run84; default off, never shipped): answer a twins-only re-enable
// (`DISABLE_CHANNELS(bDisable=FALSE)`) NV_OK with no host act — see [`ChanPlane::disable_channels`].
fn reenable_noact() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var_os("KF3_ASYNC_PREEMPT_REENABLE_NOACT").is_some_and(|v| v == "1")
    })
}

/// ★ P1+P2 inc D (§7.13) — **`KF3_NEGCTL_STALE_BIND=1`**, the stale-bind counter's POSITIVE
/// CONTROL: every recorded resolution is perturbed, so `stale_binds=` must move on a box run that
/// retires any T-mode work. Default OFF; read once. Never set in production.
fn negctl_stale_bind() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KF3_NEGCTL_STALE_BIND").is_some_and(|v| v != "0"))
}

/// ★ Review fix 2026-10-04 — **`KF3_NEGCTL_HEAP=1`**, the `heap_out=` counter's POSITIVE CONTROL:
/// every birth is counted as if its USERD lay outside the usable heap — counted only, NEVER refused
/// ([`heap_gate`]). Default OFF; read once.
fn negctl_heap() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KF3_NEGCTL_HEAP").is_some_and(|v| v != "0"))
}

/// ★ Review fix 2026-10-04 — **`KF3_NEGCTL_TWIN=1`** (T-mode), the `twin_refused=` counter's
/// POSITIVE CONTROL: every passthrough birth is answered as if its space were a guest-KERNEL space
/// — refused by name, counted. Destructive by design (no guest user channel is born): a dedicated
/// control run only. Default OFF; read once.
fn negctl_twin() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KF3_NEGCTL_TWIN").is_some_and(|v| v != "0"))
}

/// ★ P1+P2 inc A (`docs/design/V3_P1P2_TSPACE.md` §3.6) — **`KF3_TCENSUS=1`** (or
/// `KF3_TSHADOW=1`, which ran the census with the shadow): every Translated channel counts what it
/// fetches and dumps one `TCENSUS` line at free. Default OFF; read once. Count-only: nothing the
/// rewriter emits changes. ⊘ 2026-10-10: the shadow itself ran only beside the legacy rewriter
/// (deleted with `KF3_TSPACE`), so `KF3_TSHADOW` now turns on the census only, and
/// `KF3_NEGCTL_SHADOW` (the shadow's positive control) has nothing left to control.
/// ⚠ AWAITING OWNER CONFIRMATION (default off, `KF3_SW_RUNLIST_HOST_OWNED=1`): software-runlist
/// option (b), host-owned scheduling (`kf_rm::sw_runlist_host`). A Translated (kernel) channel's
/// host ring is scheduled on the host at birth already (`kf_chan::host`); with the flag on, the
/// guest-side gate is opened at birth too, so the channel runs whether or not the guest ever
/// schedules it (Windows schedules its second graphics TSG only through `0x20801111`). The guest's
/// own `GPFIFO_SCHEDULE(false)`, STOP and EVICT still close it.
fn sw_runlist_host_owned() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(kf_rm::sw_runlist_host::enabled)
}

fn tcensus_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        ["KF3_TCENSUS", "KF3_TSHADOW"]
            .iter()
            .any(|k| std::env::var_os(k).is_some_and(|v| v != "0"))
    })
}

/// ★★★ v3-initrace — **`KF3_COMPLETION_PROBE=<ms>`** (default OFF; read once; any non-number
/// means 1000). Records every Translated fence's guest semaphore RELEASES (the rewriter already
/// decodes each method; the words stay forwarded unchanged) and, the moment a pump sees the fence
/// complete, reads each released word back through OUR placements — so a completion the guest
/// never observed can be split into *"the release landed where our rows say"* vs *"it did not"*.
/// A channel whose newest completion is older than `<ms>` while the guest has not moved its
/// `GP_PUT` since, or whose fence is still in flight after `<ms>`, is dumped once with the host
/// ring's cursors, the guest's USERD and the device's interrupt state
/// (`Device::probe_tick`). ⊘ Diagnostic only — never a decision input, never a write.
#[must_use]
pub fn completion_probe_ms() -> Option<u64> {
    static MS: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *MS.get_or_init(|| {
        std::env::var("KF3_COMPLETION_PROBE")
            .ok()
            .map(|v| v.trim().parse().unwrap_or(1000))
    })
}

/// ⊘ **FAULT INJECTION, default off: `KF3_INJECT_STALE_USERD=<n>`** — at every Translated birth,
/// write `GP_PUT = GP_GET = n` into the guest's USERD before the reply, i.e. exactly what an earlier
/// channel on the same chid leaves in that slot (`[measured v3-initrace]` every open after the first
/// finds the previous CeUtils channel's `2`). The reproducer for the adapter-init flake of
/// `V3_DRIVER_MATRIX.md` §6 (`n = 1`). Never set outside a reproduction.
#[must_use]
pub fn inject_stale_userd() -> Option<u32> {
    static V: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        std::env::var("KF3_INJECT_STALE_USERD")
            .ok()
            .and_then(|v| v.trim().parse().ok())
    })
}

/// One release, read back.
#[derive(Debug, Clone)]
struct ReleaseRead {
    r: kf_chan::translated::Release,
    /// Where OUR rows place the word (`ram+off`, `store+off`, `UNPLACED`).
    at: String,
    /// The word there when read (`None`: unplaced or unreadable).
    got: Option<u32>,
}

impl ReleaseRead {
    /// ★ The word read back says the release did NOT happen: neither the payload nor a LATER
    /// value of the same monotonic word (a newer release of the same semaphore — RM's CeUtils and
    /// UVM's trackers only ever count up — may already have overwritten it by the time we read), and
    /// the release was a plain one (a reduction writes `op(old, payload)`, never the payload).
    fn not_landed(&self) -> bool {
        match self.got {
            Some(v) => {
                self.r.kind != kf_chan::translated::ReleaseKind::CeReduction
                    && v != self.r.payload
                    && (v.wrapping_sub(self.r.payload) as i32) <= 0
            }
            None => false,
        }
    }
}

impl std::fmt::Display for ReleaseRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let verdict = match self.got {
            Some(_) if self.r.kind == kf_chan::translated::ReleaseKind::CeReduction => {
                "REDUCTION(value is op(old,payload))"
            }
            Some(v) if v == self.r.payload => "LANDED",
            Some(_) if !self.not_landed() => "OVERTAKEN(a later release of this word landed)",
            Some(_) => "NOT-LANDED",
            None => "UNREAD",
        };
        write!(
            f,
            "{:?} va={:#x} want={:#x} got={} at {} {verdict}",
            self.r.kind,
            self.r.va,
            self.r.payload,
            self.got.map_or("-".to_string(), |v| format!("{v:#x}")),
            self.at
        )
    }
}

/// ★ v3-initrace: the per-channel probe record.
#[derive(Debug, Default)]
struct ProbeRec {
    /// The newest completed fences, each with its releases as read at completion.
    done: std::collections::VecDeque<(kf_chan::host::ProbeFence, Vec<ReleaseRead>)>,
    /// Fence lines printed (the first few always; every one that did not land).
    logged: u32,
    /// The guest `GP_PUT` the pump had read when the newest fence completed, and when.
    put_at_done: Option<(Option<u32>, std::time::Instant)>,
    /// The overdue dumps already printed for the newest completion / the oldest in-flight fence.
    dumped_guest: bool,
    dumped_host: bool,
    /// The last time a serve found the guest's `GP_PUT` moved, and to what.
    put_moved: Option<(u32, std::time::Instant)>,
}

/// Read the 32-bit word at `va` of `mirror` through OUR placements. `views`: read vidmem
/// through a store view (a worker only — it may arm one); `None` names it unread instead.
fn probe_read(
    ram: &RamMap,
    mirror: &Mirror,
    views: Option<(&mut StoreViews, &HostRm, u32, u64)>,
    r: kf_chan::translated::Release,
) -> ReleaseRead {
    match resolve_placed(&mirror.rows, r.va, 4) {
        None => ReleaseRead {
            r,
            at: "UNPLACED".into(),
            got: None,
        },
        Some((true, off)) => {
            let got = ram
                .at_file_offset(off, 4)
                .and_then(|(m, at)| m.load_u32(at));
            ReleaseRead {
                r,
                at: format!("ram+{off:#x}"),
                got,
            }
        }
        Some((false, off)) => {
            let got = views.and_then(|(v, rm, store, bound)| {
                let mut b = [0u8; 4];
                v.read(rm, store, bound, off, &mut b)
                    .ok()
                    .map(|()| u32::from_le_bytes(b))
            });
            ReleaseRead {
                r,
                at: format!("store+{off:#x}"),
                got,
            }
        }
    }
}

/// Hex of up to 16 bytes, as little-endian words where whole.
fn hex16(b: &[u8]) -> String {
    b.chunks(4)
        .map(|c| {
            if let Ok(w) = <[u8; 4]>::try_from(c) {
                format!("{:08x}", u32::from_le_bytes(w))
            } else {
                format!("{c:02x?}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// ★★ v3-initrace — one operand of a completed launch, read back. A VIRTUAL operand is resolved
/// through OUR placements (the host space's rows — exactly what the engine's MMU used) to the
/// store or guest RAM; a PHYSICAL one is read where the rewriter pointed the engine.
fn probe_side(
    ram: &RamMap,
    mirror: &Mirror,
    views: Option<(&mut StoreViews, &HostRm, u32, u64)>,
    o: kf_chan::translated::Operand,
) -> (String, Option<Vec<u8>>) {
    match o {
        kf_chan::translated::Operand::Physical(t, p, n) => probe_operand(ram, views, t, p, n),
        kf_chan::translated::Operand::Virtual(va, n) => {
            let len = n.min(16);
            match resolve_placed(&mirror.rows, va, len) {
                None => (format!("VA {va:#x}+{n:#x} UNPLACED by us"), None),
                Some((true, off)) => {
                    let n16 = usize::try_from(len).unwrap_or(16);
                    let host = ram.at_file_offset(off, len).and_then(|(m, at)| {
                        let mut b = vec![0u8; n16];
                        m.read_into(at, &mut b).then_some(b)
                    });
                    (
                        format!(
                            "VA {va:#x}+{n:#x} -> ram+{off:#x} host[{}]",
                            host.as_deref().map_or("unread".to_string(), hex16)
                        ),
                        host,
                    )
                }
                Some((false, off)) => {
                    let (s, b) = probe_operand(ram, views, Target::LocalFb, off, n);
                    (format!("VA {va:#x} -> {s}"), b)
                }
            }
        }
    }
}

/// ★★ v3-initrace — one PHYSICAL operand of a completed launch, read back: its first ≤ 16 bytes as
/// the HOST holds them (the store for `LOCAL_FB`, guest RAM for sysmem — what the engine read or
/// wrote) and, for a framebuffer operand, every guest CPU window position (BAR1 / BAR2 / PRAMIN)
/// showing that page with what the guest reads THERE (`mem::view_index`). A guest write that
/// landed in a window's scratch, or in a view of another store page, shows as a disagreement.
fn probe_operand(
    ram: &RamMap,
    views: Option<(&mut StoreViews, &HostRm, u32, u64)>,
    t: Target,
    phys: u64,
    len: u64,
) -> (String, Option<Vec<u8>>) {
    let n = usize::try_from(len.min(16)).unwrap_or(16);
    match t {
        Target::LocalFb => {
            let host = views.and_then(|(v, rm, store, bound)| {
                let mut b = vec![0u8; n];
                v.read(rm, store, bound, phys, &mut b).ok().map(|()| b)
            });
            let guest: Vec<String> = crate::mem::view_index()
                .map(|ix| ix.guest_views(phys, n))
                .unwrap_or_default()
                .into_iter()
                .map(|(w, at, b)| {
                    format!(
                        "{w}@{at:#x}={}",
                        b.as_deref().map_or("unreadable".to_string(), hex16)
                    )
                })
                .collect();
            let guest = if guest.is_empty() {
                "no guest CPU view of this page now".to_string()
            } else {
                guest.join(", ")
            };
            (
                format!(
                    "FB+{phys:#x}+{len:#x} host[{}] guest-views[{guest}]",
                    host.as_deref().map_or("unread".to_string(), hex16)
                ),
                host,
            )
        }
        Target::CoherentSysmem | Target::NonCoherentSysmem => {
            let host = ram
                .dma_to_file_range(phys, n as u64)
                .and_then(|off| ram.at_file_offset(off, n as u64))
                .and_then(|(m, at)| {
                    let mut b = vec![0u8; n];
                    m.read_into(at, &mut b).then_some(b)
                });
            (
                format!(
                    "{t:?}+{phys:#x}+{len:#x} host[{}]",
                    host.as_deref().map_or("unread".to_string(), hex16)
                ),
                host,
            )
        }
        Target::Peer => (format!("Peer+{phys:#x}"), None),
    }
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
        key.0 == client
            && (object == client
                || key.1 == object
                || self.tsg == Some(object)
                || self.parent == object
                || self.device == object)
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

/// ★ P1+P2 inc A / audit S1-43: the FB ranges a channel allocation names — its USERD (the
/// declared size, at least `NV_RAMUSERD_CHAN_SIZE`: the engine's footprint and what
/// [`kf_chan::host::zero_userd`] may write) and its 16-byte error notifier — each inside ONE usable
/// heap region of the layout we declared ([`kf_chip::bar0::FbLayout::in_usable_heap`]). ⊘ Never the
/// firmware carve-out, where kayfabe's own BAR1/BAR2 roots live; before this check host RM bounded
/// these offsets only by the store object. Sysmem ranges are bounded by the guest-RAM lookup at
/// their use. ⚠ A preserved console region is reserved in the layout fn 72 produces, which this
/// plane does not see: a USERD there is guest memory, accepted. Enforced through [`heap_gate`].
fn heap_bounds(
    layout: &kf_chip::bar0::FbLayout,
    userd: Option<kf_arch::UserdMem>,
    notifier: Option<kf_arch::fault::ErrorNotifier>,
) -> Result<(), String> {
    if let Some(kf_arch::UserdMem::Framebuffer { base, size }) = userd {
        let len = size.max(kf_abi::submit::USERD_SIZE);
        if !layout.in_usable_heap(base, len) {
            return Err(format!(
                "FB USERD {base:#x}+{len:#x} is outside the usable heap (carve-out at {:#x}, store {:#x})",
                layout.carve(),
                layout.fb_length
            ));
        }
    }
    if let Some(kf_arch::fault::ErrorNotifier::Framebuffer { off }) = notifier
        && !layout.in_usable_heap(off, 16)
    {
        return Err(format!(
            "FB error notifier {off:#x}+0x10 is outside the usable heap (carve-out at {:#x})",
            layout.carve()
        ));
    }
    Ok(())
}

/// What [`heap_gate`] decided for one birth.
#[derive(Debug, Clone, PartialEq, Eq)]
enum HeapGate {
    /// Every guest-named FB range lies inside the usable heap.
    Inside,
    /// Outside, and strict: refused by name before any host call.
    Refuse(String),
    /// Outside, count-only (the default path until box step 1): counted, named, born as before
    /// inc A.
    Count(String),
}

/// ★ P1+P2 inc A / S1-43 (review fix 2026-10-04): [`heap_bounds`], refusing only when `strict`;
/// `negctl` (`KF3_NEGCTL_HEAP`, the counter's positive control) counts EVERY birth and refuses none.
fn heap_gate(
    strict: bool,
    negctl: bool,
    layout: &kf_chip::bar0::FbLayout,
    userd: Option<kf_arch::UserdMem>,
    notifier: Option<kf_arch::fault::ErrorNotifier>,
) -> HeapGate {
    match heap_bounds(layout, userd, notifier) {
        Err(why) if negctl => HeapGate::Count(format!("{why} (positive control)")),
        Ok(()) if negctl => HeapGate::Count("KF3_NEGCTL_HEAP positive control".into()),
        Ok(()) => HeapGate::Inside,
        Err(why) if strict => HeapGate::Refuse(why),
        Err(why) => HeapGate::Count(why),
    }
}

/// ★★★ The channel plane.
pub struct ChanPlane {
    /// ★ GR tier (2026-10-07): GR-tier pumps that retired engine-written work since the worker
    /// last relayed it to the guest's GR0 vector ([`ChanPlane::take_gr_relay`]).
    gr_relay: AtomicU64,
    /// ★ DIAGNOSTIC (`KF3_BAR0_TRACE`, default off; owner-approved 2026-10-07): the bounded BAR0
    /// trace of the window after a guest-kernel channel's `GPFIFO_SCHEDULE` ([`crate::bar0trace`]).
    pub bar0trace: crate::bar0trace::Bar0Trace,
    rm: &'static HostRm,
    plane: &'static Plane<'static>,
    store: u32,
    ram: &'static RamMap,
    /// ★ P1+P2 / S1-43: the framebuffer layout this device declared — a guest-named FB USERD or
    /// notifier must lie inside one of its usable heap regions ([`ChanPlane::heap_bounds`]).
    layout: kf_chip::bar0::FbLayout,
    /// ★ P1+P2 inc D: the T-space (always, since 2026-10-10), built by the VA thread at prewarm.
    tspace: crate::tspace::TSpaceCell,
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
    /// ★ OWNER_RULINGS §U: each Translated channel's software objects, by host token.
    sw_objs: Mutex<HashMap<u32, crate::defapi::SwObjs>>,
    /// ★ OWNER_RULINGS §U: the VM's deferred-API tables (the object seat's), once attached.
    defapi_reg: std::sync::OnceLock<Arc<kf_rm::defapi::Registry>>,
    /// Deferred-API triggers started.
    pub defapi_triggers: AtomicU64,
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
    /// ★ 2026-09-26: the served FIFO table — which runlist an engine's channels are on (the
    /// Blackwell token index carries it, `kf_trap::tokenindex`).
    engine_table: Vec<kf_abi::inittables::FifoDeviceEntry>,
    stop: AtomicBool,
    /// The host family (its generated class sets check a guest engine-object class).
    family: kf_chip::Family,
    /// ★ P5b: the guest's user channels' twins, by `(hClient, hChannel)`.
    pt: Mutex<HashMap<(u32, u32), PtChan>>,
    /// ★ 2026-10-08 (`V3_USERD_RELAY.md`): relayed-USERD twins, by host token.
    relays: Mutex<HashMap<u32, Arc<Mutex<Relay>>>>,
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
    /// ★ 2026-10-11: births refused because the guest's channel budget (`channel-budget`, the count
    /// it was told per runlist) is spent on the birth's runlist. Named and always on: the status line's `birth_refused_cap`.
    pub birth_refused_cap: AtomicU64,
    /// Slowest act, µs.
    pub act_worst_us: AtomicU64,
    /// ★ w827: every act's time, summed (the act thread's busy time).
    pub act_total_us: AtomicU64,
    /// Passthrough twins born.
    pub pt_births: AtomicU64,
    /// ★ 2026-10-08 (owner ruling §X): host `FIFO_EVENT_MTHD` edges seen by the relay.
    pub pt_fifo_edges: AtomicU64,
    /// …of which raised a guest vector (at once, or late through optional pacing).
    pub pt_fifo_raised: AtomicU64,
    /// ★ 2026-10-09: one bit per guest vector — a paced raise still owed there carries a
    /// `FIFO_EVENT_MTHD` edge (credited to `pt_fifo_raised` when the tick delivers it).
    pt_fifo_owed: [AtomicU64; kf_chan::ptnsi::VECTORS / 64],
    /// ★ v3-video: host NVENC session slots held per guest client (acquired on OUR host client;
    /// released with the guest's release or its client's free).
    enc_sessions: Mutex<HashMap<u32, u32>>,
    /// ★ v3-video: guest TSG `(hClient, hTsg, hContextShare)` → `(host group, live members)` — the
    /// guest's TSG membership mirrored, so its channels share ONE host GR context as on hardware.
    /// Touched by acts only (serialised on the act thread).
    /// ★★ v3-int: keyed PER CONTEXT SHARE. Every member of a host group is born on the group's
    /// legacy subcontext (`HostRm::birth_member`, `hContextShare = 0`), which is exact for CUDA
    /// (one ctxshare per TSG) and wrong for a TSG with several subcontexts: `[measured vint
    /// int_gfx, ada6855a]` the Vulkan render's graphics (VEID0) and async-compute channels were
    /// merged onto one legacy subcontext and the compute channel took Xid 69 (class error, 3D
    /// class `c797`) — fence never signalled. Per-ctxshare groups keep CUDA's one-group shape and
    /// restore v3-gfx's measured shape for Vulkan (a separate host group per subcontext).
    groups: Mutex<HashMap<(u32, u32, u32), (u32, u32)>>,
    /// ★ 2026-10-09 (`crate::latejoin`): which guest TSGs the guest has scheduled — written at
    /// statement time by the drainer, read by the birth act (`crate::latejoin`).
    guest_sched: Mutex<crate::latejoin::GuestTsgSched>,
    /// ★ Per guest token: doorbells the vCPU trap rang INLINE, and how many reached the host's
    /// doorbell (the `DOORBELL-LEDGER` line at free; atomics only — the vCPU writes them).
    rung: Box<[AtomicU64]>,
    rang: Box<[AtomicU64]>,
    /// ★ `KF3_MAPLOG` only: per guest token, when its last inline doorbell was rung (µs on the
    /// `kf_mem::maplog` clock; 0 = never, or maplog off).
    rung_at_us: Box<[AtomicU64]>,
    /// ★ 2026-10-08 (owner ruling §X, `kf_chan::ptnsi`): host non-stall edges → the guest vectors
    /// of the events this VM's guest armed; never dropped (`KF3_PT_NSI_*`, read once at realize).
    pub nsi: kf_chan::ptnsi::Relay,
    /// The guest's non-stall subscriptions (`kf_rm::osevent::NonstallArms`), set once at realize
    /// from the served chain's os-event registry; read lock-free by the workers.
    nsi_arms: std::sync::OnceLock<std::sync::Arc<kf_rm::osevent::NonstallArms>>,
    /// The same registry (a shared handle), for its overflow counts in the report.
    nsi_log: std::sync::OnceLock<kf_rm::osevent::OsEventLog>,
    /// The index into [`ChanPlane::engines`] of GR0 — the fallback vector for a host
    /// `FIFO_EVENT_MTHD` edge when every vector carries an armed engine.
    host_notify_engine: Option<usize>,
    /// The plane's monotonic origin (the relay's pacing clock).
    t0: std::time::Instant,
    /// ★ P5c: the host robust-channel event fd — one dataless `NV01_EVENT_OS_EVENT` per twin's
    /// context DMA (notify index 0: `krcErrorSendEventNotificationsCtxDma_FWCLIENT` walks exactly
    /// those, `kernel_rc_notification.c:380-400`), all on this fd. A worker's poller watches it.
    pub rc_ev: kf_host::EventFd,
    /// ★ P5c: RC events waiting for the register drainer (which owns the GSP queue).
    rc_queue: Mutex<Vec<RcEvent>>,
    /// ★ 2026-10-08 (`KF3_ASYNC_PREEMPT`): `(hClient, pRunlistPreemptEvent)` of async
    /// `DISABLE_CHANNELS` whose HOST disable+preempt returned — posted by the drainer as
    /// `RUNLIST_PREEMPT_COMPLETE` ([`ChanPlane::take_preempt_done`]). Bounded.
    preempt_done: Mutex<Vec<(u32, u64)>>,
    /// Twins born with the guest's notifier armed as their host error context.
    pub rc_armed: AtomicU64,
    /// Twins whose declared notifier could NOT be armed (named at birth) — their faults are silent.
    pub rc_unarmed: AtomicU64,
    /// ★ P1+P2 inc A / S1-43: births whose guest-named FB USERD or notifier lay outside the usable
    /// heap ([`ChanPlane::heap_bounds`]) — refused when strict (`crate::tspace::inca_strict`),
    /// counted and let through as before inc A otherwise.
    pub heap_out: AtomicU64,
    /// ★ P1+P2 inc A, count-only (review fix 2026-10-04): what Translated channels' by-name
    /// refusals WOULD have refused while not strict (`kf_chan::translated::IncACounts`), summed
    /// over every channel as it runs.
    pub inca_counted: AtomicU64,
    /// ★ P1+P2 inc D (§4.2): births the per-twin state refused — a passthrough channel in a
    /// guest-KERNEL space, or a Translated one in a space with live user channels. 0 on stock
    /// drivers (UVM's channels live in UVM's own VA space).
    pub twin_refused: AtomicU64,
    /// ★ P1+P2 inc D: Translated births refused because the T-space was not built.
    pub tspace_refused: AtomicU64,
    /// ★ Review fix 2026-10-04 (§4.2): Translated births refused in a space whose only user
    /// channels are being freed (their host free not finished, or refused).
    pub twin_freeing_refused: AtomicU64,
    /// ★ 2026-09-30: the doorbell fast path (`kf_chan::dbfast`, `V3_DOORBELL_IOEVENTFD.md`): every
    /// born channel — Passthrough AND Translated — registers its guest token; every free removes it
    /// before the twin goes.
    dbfast: &'static kf_chan::dbfast::DbFast,
    /// The guest's token layout (the DATAMATCH value of a channel `(runlist, chid)`); `None` ⇒ no
    /// registration is ever made and every doorbell stays trapped.
    token_fmt: Option<kf_trap::tokenindex::GuestTokenFormat>,
    /// RC records seen on a twin's notifier (host-written).
    pub rc_seen: AtomicU64,
    /// Wakes of the RC fd.
    pub rc_wakes: AtomicU64,
    /// ★ EXPERIMENT `x11-dispsw`: the display-SW twins' counters (all zero with the switch off).
    pub dispsw: crate::dispsw::DispSwCounters,
}

/// ★ **What a guest free of `object` takes from a twin that still lives — pure** (review
/// 2026-10-03, MEDIUM): its engine object (`false`) or, x11-dispsw only, its display-SW twin
/// (`true`), with the host handle to free; `None` if the twin holds neither.
fn take_twin_object(
    objects: &mut HashMap<u32, (u32, kf_chip::classes::Kind)>,
    disp_sw: &mut HashMap<u32, u32>,
    object: u32,
) -> Option<(u32, bool)> {
    objects
        .remove(&object)
        .map(|o| (o.0, false))
        .or_else(|| disp_sw.remove(&object).map(|h| (h, true)))
}

/// The plane's passthrough twins, by `(client, channel handle)` ([`ChanPlane`]'s `pt`).
type PtMap = HashMap<(u32, u32), PtChan>;

/// The plane's object index, `(client, object handle) → (client, channel handle)` ([`ChanPlane`]'s
/// `pt_objs`).
type ObjIndex = HashMap<(u32, u32), (u32, u32)>;

/// ★ x11-dispsw: what the act checks before a display-SW twin ([`crate::dispsw::twin_one`]), read
/// from the plane's channel map on the act thread, where every display-SW alloc and free runs in
/// order (review 2026-10-03, LOW: this had no test, and a body of zeros kept CI green while the
/// caps never fired). `chan`: the twins on channel `key`'s twin. `vm`: the twins in the VM — the
/// map holds every twinned channel of every guest client, and a channel, group, device or client
/// free takes its twin out at statement time, so a freed channel's twins stop counting at once.
/// `host_refused_before`: the twin's mark ([`dispsw_mark_host_refused`]). A poisoned map counts as
/// full (`usize::MAX`), so the caps refuse.
fn dispsw_live(pt: &Mutex<PtMap>, key: (u32, u32)) -> crate::dispsw::Live {
    pt.lock().map_or(
        crate::dispsw::Live {
            chan: usize::MAX,
            vm: usize::MAX,
            host_refused_before: false,
        },
        |m| crate::dispsw::Live {
            chan: m.get(&key).map_or(0, |v| v.disp_sw.len()),
            vm: m.values().map(|v| v.disp_sw.len()).sum(),
            host_refused_before: m.get(&key).is_some_and(|v| v.dispsw_host_refused),
        },
    )
}

/// ★ x11-dispsw (review 2026-10-03, LOW): a host display-SW alloc on channel `key`'s twin was
/// refused ([`crate::dispsw::twin_watched`]) — mark the twin, so every later display-SW alloc on it
/// is refused by name. Only while the map still holds THAT twin (`chan`); `true` when marked.
fn dispsw_mark_host_refused(pt: &Mutex<PtMap>, key: (u32, u32), chan: kf_host::Channel) -> bool {
    pt.lock().is_ok_and(|mut m| match m.get_mut(&key) {
        Some(v) if v.chan == chan => {
            v.dispsw_host_refused = true;
            true
        }
        _ => false,
    })
}

/// ★ x11-dispsw (review 2026-10-03, MEDIUM; its test, LOW): the guest numbered another `ENG_SW`
/// object under channel `key` ([`ChanPlane::software_object`]) — that twin's mirror advances, and
/// no other channel's. `None`: no twin holds the channel; `Some(None)`: the mirror lost the count.
fn register_other_sw(pt: &Mutex<PtMap>, key: (u32, u32)) -> Option<Option<u16>> {
    pt.lock()
        .ok()
        .and_then(|mut m| m.get_mut(&key).map(|v| v.sw_ids.register()))
}

/// The act-queue label of the display-SW undo ([`attach_dispsw_withdraw`]).
const DISPSW_WITHDRAW: &str = "display-SW withdraw";

/// ★ x11-dispsw: the undo's wiring (review 2026-10-03, LOW). When the display-SW act resolved OK
/// and another link then refused the alloc, `d`'s owner runs this once
/// ([`kf_gsp::Deferred::on_orphaned`]); it queues the withdraw ([`withdraw_kept`]) on the act FIFO
/// `tx`, so it runs after the act that set `kept` (the host object the act kept; 0: none).
fn attach_dispsw_withdraw(
    d: &kf_gsp::Deferred,
    tx: std::sync::mpsc::Sender<(Act, kf_gsp::Deferred, &'static str)>,
    kept: Arc<AtomicU32>,
    key: (u32, u32),
    handle: u32,
) {
    d.on_orphaned(move || {
        let undo: Act = Box::new(move |me: &ChanPlane| {
            let h = kept.load(Ordering::Acquire);
            Ok(withdraw_kept(
                me.rm,
                &me.pt,
                &me.pt_objs,
                key,
                handle,
                h,
                &me.dispsw,
            ))
        });
        // Nobody waits on this cell: the guest already has the refusal.
        let _ = tx.send((undo, kf_gsp::Deferred::new(), DISPSW_WITHDRAW));
    });
}

/// ★ x11-dispsw: the undo's body ([`attach_dispsw_withdraw`]) — take the display-SW twin the act
/// kept (`h`; 0: it kept none) out of channel `key`'s twin and the object index, only while they
/// still name it ([`crate::dispsw::withdraw_disp_sw`]), and free it on the host. The log line.
fn withdraw_kept<H: crate::dispsw::DispSwHost>(
    host: &H,
    pt: &Mutex<PtMap>,
    pt_objs: &Mutex<ObjIndex>,
    key: (u32, u32),
    handle: u32,
    h: u32,
    c: &crate::dispsw::DispSwCounters,
) -> String {
    let client = key.0;
    let taken = h != 0
        && match (pt.lock(), pt_objs.lock()) {
            (Ok(mut pt), Ok(mut objs)) => crate::dispsw::withdraw_disp_sw(
                pt.get_mut(&key).map(|v| &mut v.disp_sw),
                &mut objs,
                key,
                handle,
                h,
            ),
            _ => false,
        };
    if !taken {
        return format!("{client:#x}:{handle:#x}: nothing of this act's left to withdraw");
    }
    c.withdrawn.fetch_add(1, Ordering::Relaxed);
    let freed = crate::dispsw::release_one(host, h, c);
    format!(
        "{client:#x}:{handle:#x} GF100_DISP_SW: another link refused the alloc after the act — its twin, host object {h:#x}, withdrawn and {freed}"
    )
}

/// ★ 2026-10-08 (owner ruling §X): the relay's settings (`kf_chan::ptnsi`), read once at realize.
/// - `KF3_PT_NSI_RELAY=0` — the falsifier mode: host `FIFO_EVENT_MTHD` edges are counted and NOT
///   raised (engine-notifier edges still are). Default on.
/// - `KF3_PT_NSI_MIN_INTERVAL_US` — optional, loss-free pacing per guest vector (an edge inside the
///   interval is owed and raised by the worker's tick). Default `0` = OFF; clamped to 1 s.
fn nsi_relay_from_env() -> kf_chan::ptnsi::Relay {
    kf_chan::ptnsi::Relay::new(
        kf_chan::ptnsi::min_interval_ns_from(
            std::env::var("KF3_PT_NSI_MIN_INTERVAL_US").ok().as_deref(),
        ),
        std::env::var("KF3_PT_NSI_RELAY").map_or(true, |v| v.trim() != "0"),
    )
}

/// ★ 2026-10-08 (owner ruling §X): the guest's non-stall subscription slot for an engine kind.
fn engine_slot(kind: kf_rm::authored::EngineKind) -> Option<usize> {
    use kf_abi::eventnotify as ev;
    use kf_rm::authored::EngineKind as K;
    match kind {
        K::Graphics(0) => Some(ev::NONSTALL_SLOT_GR0),
        K::Copy(i) => ev::nonstall_slot_ce(i),
        K::VideoEncode(i) => ev::nonstall_slot_nvenc(i),
        K::VideoDecode(i) => ev::nonstall_slot_nvdec(i),
        K::OpticalFlow(i) => ev::nonstall_slot_ofa(i),
        _ => None,
    }
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
        layout: kf_chip::bar0::FbLayout,
        tspace: crate::tspace::TSpaceCell,
        ram: &'static RamMap,
        mirrors: Mirrors,
        inbox: std::sync::Arc<crate::mem::Inbox>,
        wake: &'static kf_linux_raw::Notifier,
        release: &'static kf_linux_raw::Notifier,
        tokens: usize,
        family: kf_chip::Family,
        intr_table: &[kf_abi::inittables::IntrTableEntry],
        engine_table: &[kf_abi::inittables::FifoDeviceEntry],
        channel_budget: u32,
        dbfast: &'static kf_chan::dbfast::DbFast,
        token_fmt: Option<kf_trap::tokenindex::GuestTokenFormat>,
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
        let lce = host_ce.saturating_sub(kf_abi::submit::ENGINE_TYPE_COPY0);
        completions.also(rm, kf_host::event::notifier_ce(lce))?;
        // ★ GR tier: a kernel-GR ring's fence tail NSI wakes FIFO_EVENT_MTHD, and NOT the GR0
        // notifier (measured 2026-10-07 by `kf-gr-tier` at 01870988, `completion_edges`:
        // `traces/windows_code43_walls_20261007/gr-tier-native-run32.log`), so nothing is added.
        eprintln!(
            "kf3: channel plane: Translated rings on host COPY{lce} (engine {host_ce:#x}); completions on FIFO_EVENT_MTHD + CE{lce}"
        );
        // ★ P5b §2.7: one non-stall event per HOST engine a twin can run on — GR0 and every copy
        // engine the host has (`CE_GET_CAPS_V2` answers for it). Realize-time host ioctls, never a
        // vCPU. A GRCE's completions announce on GR0's vector (it has no row of its own).
        let mut engines = Vec::new();
        let mut kinds = vec![(
            kf_rm::authored::EngineKind::Graphics(0),
            NV2080_NOTIFIERS_GR0,
            kf_abi::submit::ENGINE_TYPE_GRAPHICS,
        )];
        for i in 0..20u32 {
            if let Some(et) = kf_chan::passthrough::copy_engine_type(i)
                && rm.ce_is_grce(et).is_ok()
            {
                kinds.push((
                    kf_rm::authored::EngineKind::Copy(i),
                    kf_host::event::notifier_ce(i),
                    et,
                ));
            }
        }
        // ★ The video engines the served table advertises (it lists only the host's own): one
        // non-stall event each, announced on the vector the guest was told
        // (`authored::engine_notification_rows`) — its `gkflcnServiceNotificationInterrupt` wakes
        // the guest's NVENC/NVDEC OS events from it.
        for i in 0..kf_abi::submit::NVENC_SIZE {
            let kind = kf_rm::authored::EngineKind::VideoEncode(i);
            if let (Some(et), Some(_)) = (
                kf_abi::submit::engine_type_nvenc(i),
                kf_rm::authored::non_stall_vector_for(intr_table, kind),
            ) {
                kinds.push((kind, kf_host::event::notifier_nvenc(i), et));
            }
        }
        for i in 0..kf_abi::submit::NVDEC_SIZE {
            let kind = kf_rm::authored::EngineKind::VideoDecode(i);
            if let (Some(et), Some(_)) = (
                kf_abi::submit::engine_type_nvdec(i),
                kf_rm::authored::non_stall_vector_for(intr_table, kind),
            ) {
                kinds.push((kind, kf_host::event::notifier_nvdec(i), et));
            }
        }
        // ★ v3-gfxset: the optical-flow engine, the same way (its `gkflcnServiceNotificationInterrupt`
        // wakes the guest's OFA OS events — VK_NV_optical_flow's completion).
        for i in 0..kf_abi::submit::OFA_SIZE {
            let kind = kf_rm::authored::EngineKind::OpticalFlow(i);
            if let (Some(et), Some(_)) = (
                kf_abi::submit::engine_type_ofa(i),
                kf_rm::authored::non_stall_vector_for(intr_table, kind),
            ) {
                kinds.push((kind, kf_host::event::notifier_ofa(i), et));
            }
        }
        for (kind, notify, engine_type) in kinds {
            let ev = rm
                .open_event_fd()
                .map_err(|e| format!("{} event fd: {e:?}", kind.name()))?;
            rm.alloc_os_event(rm.subdevice(), notify, true, &ev)
                .map_err(|e| format!("{} os event: {e:?}", kind.name()))?;
            rm.arm_repeat(notify)
                .map_err(|e| format!("{} notify: {e:?}", kind.name()))?;
            let vector = kf_rm::authored::non_stall_vector_for(intr_table, kind);
            engines.push(EngineEvent {
                name: kind.name(),
                ev,
                vector,
                engine_type,
                live: AtomicU64::new(0),
                wakes: AtomicU64::new(0),
                raised: AtomicU64::new(0),
                unraised_no_live: AtomicU64::new(0),
                unraised_no_vector: AtomicU64::new(0),
                tlive: AtomicU64::new(0),
                trelays: AtomicU64::new(0),
                cpending: AtomicU64::new(0),
                crelays: AtomicU64::new(0),
                slot: engine_slot(kind),
                not_armed: AtomicU64::new(0),
                owed_late: std::sync::atomic::AtomicBool::new(false),
            });
        }
        eprintln!(
            "kf3: interrupt plane: host non-stall events -> guest vectors [{}]",
            engines
                .iter()
                .map(|e| format!("{}->{:x?}", e.name, e.vector))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let nsi = nsi_relay_from_env();
        let host_notify_engine = engines
            .iter()
            .position(|e| e.engine_type == kf_abi::submit::ENGINE_TYPE_GRAPHICS);
        eprintln!(
            "kf3: non-stall relay (owner ruling 2026-10-08): every host edge -> the guest vector of every event this guest ARMED, never dropped; FIFO_EVENT_MTHD -> a vector no armed engine shares, else {} vector {:?} (relay={}, pacing={}; KF3_PT_NSI_RELAY / KF3_PT_NSI_MIN_INTERVAL_US)",
            host_notify_engine
                .and_then(|i| engines.get(i))
                .map_or("no GR0", |e| e.name.as_str()),
            host_notify_engine
                .and_then(|i| engines.get(i))
                .and_then(|e| e.vector),
            if nsi.relays_fifo() {
                "on"
            } else {
                "OFF (FIFO_EVENT_MTHD edges counted only)"
            },
            match nsi.pacer().interval_ns() {
                0 => "off".to_string(),
                ns => format!("{} us, loss-free", ns / 1000),
            }
        );
        // ★ P5c: the RC fd (realize-time host ioctls, never a vCPU).
        let rc_ev = rm
            .open_event_fd()
            .map_err(|e| format!("RC event fd: {e:?}"))?;
        Ok(ChanPlane {
            gr_relay: AtomicU64::new(0),
            bar0trace: crate::bar0trace::Bar0Trace::from_env(),
            rm,
            plane,
            store,
            layout,
            tspace,
            ram,
            mirrors,
            inbox,
            completions,
            // Declared caps: channels are the only twin this plane mints, and the cap is the number
            // the guest was TOLD per runlist (`kf_abi::chanbudget`, the `channel-budget` property, §9.1)
            // — a hardcoded 64 refused the 65th live channel of a Windows desktop (OpenGL ICD crash, CUDA 999).
            caps: Mutex::new(VmCaps::from_declared(channel_budget, 64, 64, 64)),
            slots: RwLock::new(HashMap::new()),
            by_obj: Mutex::new(HashMap::new()),
            sw_objs: Mutex::new(HashMap::new()),
            defapi_reg: std::sync::OnceLock::new(),
            defapi_triggers: AtomicU64::new(0),
            scopes: Mutex::new(HashMap::new()),
            wake,
            contended: AtomicU64::new(0),
            poisoned: AtomicU64::new(0),
            births: AtomicU64::new(0),
            host_ce,
            engine_table: engine_table.to_vec(),
            stop: AtomicBool::new(false),
            family,
            pt: Mutex::new(HashMap::new()),
            relays: Mutex::new(HashMap::new()),
            pt_objs: Mutex::new(HashMap::new()),
            dbg: Mutex::new(HashMap::new()),
            cuda_limit: Mutex::new((std::collections::BTreeSet::new(), false)),
            acts: Mutex::new(None),
            release,
            engines,
            acts_run: AtomicU64::new(0),
            acts_refused: AtomicU64::new(0),
            birth_refused_cap: AtomicU64::new(0),
            act_worst_us: AtomicU64::new(0),
            act_total_us: AtomicU64::new(0),
            pt_births: AtomicU64::new(0),
            pt_fifo_edges: AtomicU64::new(0),
            pt_fifo_raised: AtomicU64::new(0),
            pt_fifo_owed: std::array::from_fn(|_| AtomicU64::new(0)),
            groups: Mutex::new(HashMap::new()),
            guest_sched: Mutex::new(crate::latejoin::GuestTsgSched::default()),
            enc_sessions: Mutex::new(HashMap::new()),
            rung: (0..tokens).map(|_| AtomicU64::new(0)).collect(),
            rang: (0..tokens).map(|_| AtomicU64::new(0)).collect(),
            rung_at_us: (0..tokens).map(|_| AtomicU64::new(0)).collect(),
            nsi,
            nsi_arms: std::sync::OnceLock::new(),
            nsi_log: std::sync::OnceLock::new(),
            host_notify_engine,
            t0: std::time::Instant::now(),
            rc_ev,
            rc_queue: Mutex::new(Vec::new()),
            preempt_done: Mutex::new(Vec::new()),
            rc_armed: AtomicU64::new(0),
            rc_unarmed: AtomicU64::new(0),
            heap_out: AtomicU64::new(0),
            inca_counted: AtomicU64::new(0),
            twin_refused: AtomicU64::new(0),
            tspace_refused: AtomicU64::new(0),
            twin_freeing_refused: AtomicU64::new(0),
            rc_seen: AtomicU64::new(0),
            rc_wakes: AtomicU64::new(0),
            dispsw: crate::dispsw::DispSwCounters::default(),
            dbfast,
            token_fmt,
        })
    }

    /// ★ **Act thread**: give the channel just born on token `idx` — `(runlist, chid)` in the guest's
    /// own numbering — its doorbell fast path. Called AFTER its token word is installed (until the
    /// KVM placement lands, the trap serves it) and with NO plane lock held (the fast path may wait
    /// on the drainer). The value is refused unless the device's own index finds `idx` from it — a
    /// value naming another slot would deliver another channel's doorbells. Returns the birth
    /// line's `fast=` field.
    fn fast_register(&self, idx: u32, runlist: u32, chid: u32) -> String {
        if !self.dbfast.enabled() {
            return "fast=off".into();
        }
        let Some(v) = self.token_fmt.and_then(|f| f.value(runlist, chid)) else {
            return format!("fast=unencodable(runlist {runlist}, chid {chid:#x})");
        };
        if self.plane.token_index.of_doorbell(v) != Some(idx) {
            return format!("fast=refused(value {v:#010x} names another slot)");
        }
        match self.dbfast.register(idx, v) {
            kf_chan::dbfast::RegOutcome::Off => "fast=off".into(),
            kf_chan::dbfast::RegOutcome::Refused => {
                format!("fast=refused(value {v:#010x}: no eventfd)")
            }
            kf_chan::dbfast::RegOutcome::Registered {
                placed,
                refused,
                sites,
                ..
            } => format!("fast=on value={v:#010x} placed={placed}/{sites} refused={refused}"),
        }
    }

    /// The `DOORBELL-LEDGER` fields a removed fast-path registration contributes.
    fn fast_fields(l: Option<kf_chan::dbfast::FastLedger>) -> String {
        match l {
            None => " fast=0".into(),
            Some(l) => format!(
                " fast={} fast_wakes={} fast_forwarded={} fast_absorbed={} fast_failed={} fast_sites={} fast_refused={}",
                l.doorbells, l.wakes, l.forwarded, l.absorbed, l.failed, l.sites, l.refused
            ),
        }
    }

    /// ★ P5b: start the ACT thread — every host act a statement implies runs here, in statement
    /// order, off the drainer and off every lock the drainer holds.
    ///
    /// # Errors
    /// The spawn.
    pub fn start(&'static self) -> Result<(), String> {
        let (tx, rx) = std::sync::mpsc::channel::<(Act, kf_gsp::Deferred, &'static str)>();
        std::thread::Builder::new()
            .name(kf_rm::gssnative::ACT_THREAD.into())
            .spawn(move || {
                let mut q = crate::actq::ActQueue::new(rx);
                while let Some((act, d, what)) = q.next_act() {
                    let t0 = std::time::Instant::now();
                    let r = act(self);
                    // Retries the act parked (a steer waiting for a map in flight to end).
                    for (when, a, dd, w) in
                        ACT_DELAYED.with(|c| std::mem::take(&mut *c.borrow_mut()))
                    {
                        q.delay(when, (a, dd, w));
                    }
                    let us = u64::try_from(t0.elapsed().as_micros()).unwrap_or(u64::MAX);
                    self.act_worst_us.fetch_max(us, Ordering::Relaxed);
                    self.act_total_us.fetch_add(us, Ordering::Relaxed);
                    self.acts_run.fetch_add(1, Ordering::Relaxed);
                    match r {
                        Ok(line) => {
                            // EXPERIMENT KF3_GSS_NATIVE: an act that says nothing is quiet (its own
                            // counters and once-lines speak; no per-RPC print).
                            if !line.is_empty() {
                                eprintln!("kf3: act {what}: {line} ({us} us, off the GSP lock)");
                            }
                            d.resolve(0);
                        }
                        // ★ Review 3 item 5: the act put itself back on the queue; its reply is NOT given.
                        Err((ACT_REQUEUED, _)) => continue,
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

    /// ★ Realize (2026-10-08, owner ruling §X): the guest's non-stall subscriptions, from the
    /// served chain's os-event registry. Set once; until then no event counts as armed.
    pub fn set_os_events(&self, log: &kf_rm::osevent::OsEventLog) {
        let _ = self.nsi_arms.set(log.nonstall_arms());
        let _ = self.nsi_log.set(log.clone());
    }

    /// Whether the guest armed subscription `slot` (lock-free).
    fn nsi_armed(&self, slot: Option<usize>) -> bool {
        match (slot, self.nsi_arms.get()) {
            (Some(s), Some(a)) => a.armed(s),
            _ => false,
        }
    }

    /// The relay's pacing clock: ns since the plane was built.
    fn nsi_now_ns(&self) -> u64 {
        u64::try_from(self.t0.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    /// ★ **Worker**, on a REAL host `FIFO_EVENT_MTHD` edge (2026-10-08, owner ruling §X): if the
    /// guest armed `FIFO_EVENT_MTHD`, raise the vector `kf_chan::ptnsi::host_notify_vector` picks —
    /// one whose guest service fires the guest's own HOST notifier and, if possible, no engine
    /// event the guest armed (GR0's otherwise) — via `deliver`. Never dropped: unarmed is counted,
    /// paced is owed. With `KF3_PT_NSI_RELAY=0` the edge is counted only (the falsifier run).
    pub fn nsi_fifo_edge(&self, deliver: impl FnOnce(u32)) -> kf_chan::ptnsi::Verdict {
        let n = self
            .pt_fifo_edges
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        let armed = self.nsi_armed(Some(kf_abi::eventnotify::NONSTALL_SLOT_FIFO_EVENT_MTHD));
        let vector = if armed {
            kf_chan::ptnsi::host_notify_vector(
                self.engines
                    .iter()
                    .map(|e| (e.vector, self.nsi_armed(e.slot))),
                self.host_notify_engine
                    .and_then(|i| self.engines.get(i))
                    .and_then(|e| e.vector),
            )
        } else {
            None
        };
        let verdict = self.nsi.edge(
            kf_chan::ptnsi::EdgeKind::Fifo,
            armed,
            vector,
            self.nsi_now_ns(),
        );
        match verdict {
            kf_chan::ptnsi::Verdict::Raise(v) => {
                self.pt_fifo_raised.fetch_add(1, Ordering::Relaxed);
                deliver(v);
            }
            kf_chan::ptnsi::Verdict::Owed => {
                if let Some(v) = vector.map(|v| v as usize)
                    && let Some(w) = self.pt_fifo_owed.get(v / 64)
                {
                    w.fetch_or(1 << (v % 64), Ordering::Relaxed);
                }
            }
            _ => {}
        }
        if n <= 16 || n.is_power_of_two() {
            eprintln!(
                "kf3: PT-NSI host FIFO_EVENT_MTHD edge #{n}: {verdict:?} (guest armed={armed}, vector {vector:?})"
            );
        }
        verdict
    }

    /// ★ **Worker**, on engine `e`'s own host notifier (2026-10-08, owner ruling §X): raise the
    /// engine's guest vector if the guest armed that engine's notifier — whoever's work it was
    /// (the notifier is GPU-wide, as RM's own delivery is). Never dropped.
    pub fn nsi_engine_edge(
        &self,
        e: &EngineEvent,
        deliver: impl FnOnce(u32),
    ) -> kf_chan::ptnsi::Verdict {
        let armed = self.nsi_armed(e.slot);
        let verdict = self.nsi.edge(
            kf_chan::ptnsi::EdgeKind::Engine,
            armed,
            e.vector,
            self.nsi_now_ns(),
        );
        match verdict {
            kf_chan::ptnsi::Verdict::Raise(v) => {
                e.raised.fetch_add(1, Ordering::Relaxed);
                deliver(v);
            }
            kf_chan::ptnsi::Verdict::NotArmed => {
                e.not_armed.fetch_add(1, Ordering::Relaxed);
            }
            kf_chan::ptnsi::Verdict::Owed => {
                e.owed_late.store(true, Ordering::Relaxed);
            }
            _ => {}
        }
        verdict
    }

    /// ★ **Worker tick** (every loop; at least every millisecond while something is owed): raise
    /// what the optional pacing owes. Returns whether anything is still owed. With pacing off
    /// (the default) it is one branch.
    ///
    /// ★ 2026-10-09: a late raise is credited to the source(s) whose edge it carries — `pt_fifo_raised`
    /// if a `FIFO_EVENT_MTHD` edge was owed on its vector, and `raised` of every engine whose own
    /// edge was owed there (the edge side marks them; edges, tick and marks all run on the one
    /// worker thread). One late raise can carry both kinds, so the per-source counts may sum above
    /// the vector's `raised`; per vector, `late` stays exact.
    pub fn nsi_tick(&self, mut deliver: impl FnMut(u32)) -> bool {
        if self.nsi.pacer().interval_ns() == 0 {
            return false;
        }
        self.nsi.flush(self.nsi_now_ns(), |v| {
            let i = v as usize;
            if let Some(w) = self.pt_fifo_owed.get(i / 64)
                && w.fetch_and(!(1 << (i % 64)), Ordering::Relaxed) & (1 << (i % 64)) != 0
            {
                self.pt_fifo_raised.fetch_add(1, Ordering::Relaxed);
            }
            for e in self.engines.iter().filter(|e| e.vector == Some(v)) {
                if e.owed_late.swap(false, Ordering::Relaxed) {
                    e.raised.fetch_add(1, Ordering::Relaxed);
                }
            }
            deliver(v);
        })
    }

    /// The `PT-NSI` report: the relay's counters, the guest's armed subscriptions and every engine
    /// that woke or is armed.
    #[must_use]
    pub fn pt_summary(&self) -> String {
        let o = Ordering::Relaxed;
        let arms = self.nsi_arms.get();
        let count = |slot: Option<usize>| match (slot, arms) {
            (Some(s), Some(a)) => a.count(s),
            _ => 0,
        };
        let per: Vec<String> = self
            .engines
            .iter()
            .filter(|e| e.wakes.load(o) > 0 || count(e.slot) > 0)
            .map(|e| {
                format!(
                    "{}[vec={:?} armed={} wakes={} not_armed={} raised={} live_twins={}]",
                    e.name,
                    e.vector,
                    count(e.slot),
                    e.wakes.load(o),
                    e.not_armed.load(o),
                    e.raised.load(o),
                    e.live.load(o)
                )
            })
            .collect();
        format!(
            "fifo_edges={} fifo_raised={} fifo_armed={} sticky={:#x} kernel_nonstall_registered={} arm_clears={} os_event_overflowed={} kernel_nonstall_overflowed={} {} {}",
            self.pt_fifo_edges.load(o),
            self.pt_fifo_raised.load(o),
            count(Some(kf_abi::eventnotify::NONSTALL_SLOT_FIFO_EVENT_MTHD)),
            arms.map_or(0, |a| a.sticky()),
            arms.map_or(0, |a| a.kernel_registered.load(o)),
            arms.map_or(0, |a| a.clears.load(o)),
            self.nsi_log
                .get()
                .map_or(0, kf_rm::osevent::OsEventLog::overflowed),
            self.nsi_log
                .get()
                .map_or(0, kf_rm::osevent::OsEventLog::kernel_overflowed),
            self.nsi.summary(),
            per.join(" ")
        )
    }

    /// ★ **vCPU**: a doorbell on guest token `idx` was rung inline; `reached` = the host store
    /// succeeded. Two relaxed atomics — the only thing this plane does on a vCPU.
    pub fn note_inline(&self, idx: u32, reached: bool) {
        if let Some(c) = self.rung.get(idx as usize) {
            c.fetch_add(1, Ordering::Relaxed);
        }
        if kf_mem::maplog::on()
            && let Some(c) = self.rung_at_us.get(idx as usize)
        {
            // ⊘ Diagnostic only (and it writes a line from the vCPU — never on in production).
            let t = kf_mem::maplog::t();
            c.store((t * 1e6) as u64, Ordering::Relaxed);
            let n = self
                .rung
                .get(idx as usize)
                .map_or(0, |r| r.load(Ordering::Relaxed));
            eprintln!("kf3: maplog t={t:.6} DOORBELL chid {idx:#x} #{n} reached={reached}");
        }
        if reached && let Some(c) = self.rang.get(idx as usize) {
            c.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// `(rung, reached)` for token `idx`, reset (a freed token's successor starts at zero).
    fn take_ledger(&self, idx: u32) -> (u64, u64) {
        let r = self
            .rung
            .get(idx as usize)
            .map_or(0, |c| c.swap(0, Ordering::Relaxed));
        let f = self
            .rang
            .get(idx as usize)
            .map_or(0, |c| c.swap(0, Ordering::Relaxed));
        (r, f)
    }

    /// ★ 2026-10-08 (`KF3_ASYNC_PREEMPT`): the completed async preempts not yet posted (drained) —
    /// ⊘ none while any reply is still held ([`preempt_posts_ready`]): the event must follow its
    /// control's reply, as the GSP posts it.
    pub fn take_preempt_done(&self, held_replies: usize) -> Vec<(u32, u64)> {
        if !preempt_posts_ready(held_replies) {
            return Vec::new();
        }
        self.preempt_done
            .lock()
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default()
    }

    /// ★ 2026-10-08: give back preempt completions the queue could not take yet (front first).
    pub fn requeue_preempt_done(&self, back: Vec<(u32, u64)>) {
        if let Ok(mut q) = self.preempt_done.lock() {
            let mut v = back;
            v.append(&mut q);
            v.truncate(PREEMPT_DONE_MAX);
            *q = v;
        }
    }

    /// ★ GR tier: the guest vector to raise because a GR-tier pump retired engine-written work
    /// since the last call — `(vector, engine name, relay number, live GR-tier rings)`, or `None`.
    /// Called by the worker right after [`ChanPlane::serve`].
    pub fn take_gr_relay(&self) -> Option<(u32, &str, u64, u64)> {
        if self.gr_relay.swap(0, Ordering::AcqRel) == 0 {
            return None;
        }
        let e = self
            .engines
            .iter()
            .find(|e| e.engine_type == kf_abi::submit::ENGINE_TYPE_GRAPHICS)?;
        let v = e.vector?;
        e.raised.fetch_add(1, Ordering::Relaxed);
        let n = e.trelays.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        Some((v, e.name.as_str(), n, e.tlive.load(Ordering::Relaxed)))
    }

    /// ★ 2026-10-07 (`KF3_TRANSLATED_CE_RELAY`): for each guest copy engine whose Translated rings
    /// retired fenced work since the last call, `f(vector, engine name, relay number)`. Called by the
    /// worker right after [`ChanPlane::serve`]; an engine with no guest vector is counted, not raised.
    pub fn for_each_ce_relay(&self, mut f: impl FnMut(u32, &str, u64)) {
        for e in &self.engines {
            if e.cpending.swap(0, Ordering::AcqRel) == 0 {
                continue;
            }
            let Some(v) = e.vector else { continue };
            e.raised.fetch_add(1, Ordering::Relaxed);
            let n = e.crelays.fetch_add(1, Ordering::Relaxed).saturating_add(1);
            f(v, e.name.as_str(), n);
        }
    }

    /// ★ GR tier: a Translated GR-tier ring on `engine_type` came up / went away.
    fn engine_tlive(&self, engine_type: u32, up: bool) {
        if let Some(e) = self.engines.iter().find(|e| e.engine_type == engine_type) {
            // Only a slot that counted itself up (`Slot::gr_tier`) counts down: never below 0.
            if up {
                e.tlive.fetch_add(1, Ordering::Relaxed);
            } else {
                e.tlive.fetch_sub(1, Ordering::Relaxed);
            }
        }
    }

    fn engine_live(&self, engine_type: u32, up: bool) {
        if let Some(e) = self.engines.iter().find(|e| e.engine_type == engine_type) {
            if up {
                e.live.fetch_add(1, Ordering::Relaxed);
            } else {
                let _ = e
                    .live
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(1));
            }
        }
    }

    /// Queue `act` and answer [`ChanAnswer::Deferred`] — the drainer returns at once.
    fn defer(&self, what: &'static str, act: Act) -> ChanAnswer {
        let d = kf_gsp::Deferred::new();
        self.defer_cell(what, act, d)
    }

    /// [`Self::defer`] with the caller's own cell (an act that must reach its own `Deferred`).
    fn defer_cell(&self, what: &'static str, act: Act, d: kf_gsp::Deferred) -> ChanAnswer {
        let sent = self
            .acts
            .lock()
            .ok()
            .and_then(|a| a.as_ref().map(|tx| tx.send((act, d.clone(), what)).is_ok()));
        if sent == Some(true) {
            ChanAnswer::Deferred(d)
        } else {
            ChanAnswer::Refused {
                status: NV_ERR_INVALID_STATE,
                why: format!("{what}: the act thread is not running"),
            }
        }
    }

    /// ★ EXPERIMENT `KF3_GSS_NATIVE` (`docs/design/V3_GSS_NATIVE.md`): a non-privileged subdevice-level
    /// GSS-legacy control, carried to OUR host subdevice as an act — the host call is the act
    /// thread's (statement order, off the drainer, off every lock the drainer holds), the reply is
    /// held on the cell, and a call that blocks on the host RM API lock shows in `acts` /
    /// `act_worst_us`. The act resolves the cell itself (the host's status, not `0`) and says
    /// nothing per call (`kf_rm::gssnative` counts, and says each first answer once).
    pub fn gss_forward(
        &self,
        req: kf_rm::gssnative::GssRequest,
        stats: &Arc<kf_rm::gssnative::Stats>,
    ) -> kf_rm::gssnative::GssAnswer {
        let d = kf_gsp::Deferred::new();
        let (cell, stats) = (d.clone(), stats.clone());
        let act: Act = Box::new(move |me: &ChanPlane| {
            kf_rm::gssnative::execute(&stats, &GssHostRm(me.rm), req, &cell);
            Ok(String::new())
        });
        match self.defer_cell("gss-native control", act, d) {
            ChanAnswer::Deferred(d) => kf_rm::gssnative::GssAnswer::Deferred(d),
            ChanAnswer::Refused { why, .. } => kf_rm::gssnative::GssAnswer::Refused { why },
            _ => kf_rm::gssnative::GssAnswer::Refused {
                why: "unexpected answer".into(),
            },
        }
    }

    /// The passthrough twins matching `f`, removed.
    fn take_pt(&self, f: impl Fn(&(u32, u32), &PtChan) -> bool) -> Vec<((u32, u32), PtChan)> {
        let Ok(mut m) = self.pt.lock() else {
            return Vec::new();
        };
        let keys: Vec<(u32, u32)> = m.iter().filter(|(k, v)| f(k, v)).map(|(k, _)| *k).collect();
        keys.into_iter()
            .filter_map(|k| m.remove(&k).map(|v| (k, v)))
            .collect()
    }

    /// A `GPFIFO_SCHEDULE` statement on `object` (a channel or a TSG): the guest's own schedule,
    /// carried to its twins' host groups. ⊘ Never blocks: the host verb is an act.
    fn schedule_statement(&self, client: u32, object: u32, enable: bool) -> ChanAnswer {
        if let Some(ht) = self
            .by_obj
            .lock()
            .ok()
            .and_then(|m| m.get(&(client, object)).copied())
        {
            return self.schedule_translated(client, object, ht, enable);
        }
        // ★ P6: a TSG schedule (`0xa06c0101`) over Translated members (nvidia-uvm's
        // channels live in groups): every member's slot, as its own schedule.
        let members: Vec<u32> = self
            .by_obj
            .lock()
            .map(|m| {
                m.iter()
                    .filter(|(k, _)| k.0 == client)
                    .map(|(_, v)| *v)
                    .collect::<Vec<u32>>()
            })
            .unwrap_or_default()
            .into_iter()
            .filter(|ht| {
                self.slot(*ht)
                    .is_some_and(|s| s.lock().is_ok_and(|g| g.tsg == Some(object)))
            })
            .collect();
        if !members.is_empty() {
            for ht in members {
                if let ChanAnswer::Refused { status, why } =
                    self.schedule_translated(client, object, ht, enable)
                {
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
                let run = || -> Result<String, (u32, String)> {
                    let mut restarted = 0u32;
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
                            restarted = restarted.saturating_add(1);
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
                        crate::latejoin::schedule_group_once(me.rm, &mut groups_done, *c, enable)
                            .map_err(|e| (NV_ERR_INVALID_STATE, e))?;
                    }
                    Ok(format!("{client:#x}:{object:#x} GPFIFO_SCHEDULE enable={enable} on {} twin(s) ({restarted} restarted after STOP)", twins.len()))
                };
                let result = run();
                // ★ 2026-10-09: a schedule the host refused is not the guest's scheduled state.
                if result.is_err()
                    && let Ok(mut s) = me.guest_sched.lock()
                {
                    s.forget(client, object);
                }
                result
            }),
        )
    }

    /// ★ The drainer's entry: one statement from the served chain (`kf_rm::chanlink`). ⊘ Never
    /// blocks: a statement that implies a host act answers [`ChanAnswer::Deferred`] and the act
    /// runs on the act thread (P5b).
    pub fn statement(&self, st: ChanStatement) -> ChanAnswer {
        match st {
            ChanStatement::Alloc(a) => self.birth(a),
            ChanStatement::Schedule {
                client,
                object,
                enable,
            } => {
                let answer = self.schedule_statement(client, object, enable);
                // ★ 2026-10-09 (late TSG joiners): the guest's own scheduled state, recorded at
                // STATEMENT time (statement order is the act thread's order, so a twin whose birth
                // act is still queued sees it). A refused statement is not a schedule.
                if !matches!(answer, ChanAnswer::Refused { .. })
                    && let Ok(mut s) = self.guest_sched.lock()
                {
                    s.record(client, object, enable);
                }
                answer
            }
            ChanStatement::Bind {
                client,
                object,
                engine_type,
            } => {
                if let Some(ht) = self
                    .by_obj
                    .lock()
                    .ok()
                    .and_then(|m| m.get(&(client, object)).copied())
                {
                    if engine_type == kf_abi::submit::ENGINE_TYPE_GRAPHICS
                        || kf_abi::submit::nvdec_index_of_engine_type(engine_type).is_some()
                        || kf_abi::submit::nvenc_index_of_engine_type(engine_type).is_some()
                        || kf_abi::submit::ofa_index_of_engine_type(engine_type).is_some()
                    {
                        return self.defer("bind translated context", Box::new(move |me: &ChanPlane| {
                            let slot = me.slot(ht).ok_or_else(|| (NV_ERR_INVALID_STATE, "GR slot gone".into()))?;
                            let g = slot.lock().map_err(|_| (NV_ERR_INVALID_STATE, "GR slot poisoned".into()))?;
                            if g.guest_engine != engine_type || !g.chan.host().owns_context(engine_type) || g.dead.is_some() {
                                return Err((NV_ERR_INVALID_STATE, "BIND has no matching owned context".into()));
                            }
                            Ok(format!("{client:#x}:{object:#x} BIND satisfied by owned context on host {ht:#x}"))
                        }));
                    }
                    // ★ The guest's statement that this channel runs on a copy engine. Our twin's
                    // host TSG was bound to OUR engine at birth; a bind to anything but a copy
                    // engine contradicts the Translated route and is refused by name.
                    if !is_copy_engine(engine_type) {
                        return ChanAnswer::Refused {
                            status: NV_ERR_INVALID_STATE,
                            why: format!(
                                "BIND of a Translated CE channel (host {ht:#x}) to engine {engine_type:#x}"
                            ),
                        };
                    }
                    eprintln!(
                        "kf3: chan {client:#x}:{object:#x} BIND engine={engine_type:#x} (host {ht:#x})"
                    );
                    return ChanAnswer::Done;
                }
                // ★ P5b: the twin's TSG was bound to the guest's engine at birth; the guest's own
                // BIND must name that engine (the runlist the guest computes its token from).
                let twin = self
                    .pt
                    .lock()
                    .ok()
                    .and_then(|m| m.get(&(client, object)).map(|v| (v.engine, v.chan.token)));
                match twin {
                    None => ChanAnswer::NotOurs,
                    Some((e, ht)) if e == engine_type => {
                        eprintln!(
                            "kf3: chan {client:#x}:{object:#x} BIND engine={engine_type:#x} (passthrough host {ht:#x})"
                        );
                        ChanAnswer::Done
                    }
                    Some((e, ht)) => ChanAnswer::Refused {
                        status: NV_ERR_INVALID_STATE,
                        why: format!(
                            "BIND to engine {engine_type:#x} of a twin born on {e:#x} (host {ht:#x})"
                        ),
                    },
                }
            }
            ChanStatement::Token { client, object } => {
                if let Some(ht) = self
                    .by_obj
                    .lock()
                    .ok()
                    .and_then(|m| m.get(&(client, object)).copied())
                {
                    return match self
                        .slot(ht)
                        .and_then(|s| s.lock().ok().map(|g| g.guest_idx))
                    {
                        // ★ The token is OUR doorbell's vocabulary (we are the host): the table
                        // index, in the VECTOR field the trap masks (`kf_trap::trap`).
                        Some(idx) => ChanAnswer::Token(idx),
                        None => ChanAnswer::NotOurs,
                    };
                }
                match self
                    .pt
                    .lock()
                    .ok()
                    .and_then(|m| m.get(&(client, object)).map(|v| v.idx))
                {
                    Some(idx) => ChanAnswer::Token(idx),
                    None => ChanAnswer::NotOurs,
                }
            }
            ChanStatement::EngineObject {
                client,
                parent,
                handle,
                class,
                copy_engine,
            } => self.engine_object(client, parent, handle, class, copy_engine),
            ChanStatement::Debugger {
                client,
                parent,
                handle,
                app_client,
                obj3d,
            } => self.debugger(client, parent, handle, app_client, obj3d),
            ChanStatement::DisplaySw {
                client,
                parent,
                handle,
            } => {
                // ★ §U: the guest numbered it on its channel, whatever happens to it next.
                self.number_translated_sw(client, parent, kf_rm::chanlink::GF100_DISP_SW, None);
                self.display_sw(client, parent, handle)
            }
            ChanStatement::SoftwareObject {
                client,
                parent,
                class,
            } => {
                self.number_translated_sw(client, parent, class, None);
                self.software_object(client, parent, class)
            }
            ChanStatement::DeferredApiObject {
                client,
                parent,
                handle,
            } => self.deferred_api_object(client, parent, handle),
            ChanStatement::DebuggerExceptionMask {
                client,
                object,
                mask,
            } => {
                let Some(h) = self
                    .dbg
                    .lock()
                    .ok()
                    .and_then(|m| m.get(&(client, object)).map(|v| v.1))
                else {
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
            ChanStatement::CtxswPreemption {
                client,
                channel,
                flags,
                gfxp,
                cilp,
            } => {
                // The guest's channel, or every twin of the guest's TSG — GR twins only: the mode
                // is a GR context property (a CE twin in the group has none).
                // ★ v3-gfxset: …unless the target HAS no GR twin. `[measured diag1, RTX 3070]` the
                // Vulkan UMD (ffmpeg's device: 2 transfer queues) sets CILP on each BARE copy channel
                // it creates (`hChannel` = the channel, its TSG implicit, `kernel_channel.c:354-381`);
                // on bare metal GSP answers NV_OK, here the GR-only filter left nothing, the answer
                // fell through to 0x56, and the UMD freed the channel, retried and failed
                // `vkCreateDevice` (VK_ERROR_INITIALIZATION_FAILED). Such a target's own twins get the
                // same authored verb, on their own host groups, and the HOST decides — never a
                // forged OK. A target with no twin at all is still not ours.
                let pick = |gr_only: bool| -> Vec<kf_host::Channel> {
                    let mut out: Vec<kf_host::Channel> = self
                        .pt
                        .lock()
                        .map(|m| {
                            m.iter()
                                .filter(|(k, v)| {
                                    k.0 == client
                                        && (k.1 == channel || v.tsg == Some(channel))
                                        && (!gr_only
                                            || v.engine == kf_abi::submit::ENGINE_TYPE_GRAPHICS)
                                })
                                .map(|(_, v)| v.chan)
                                .collect()
                        })
                        .unwrap_or_default();
                    out.sort_by_key(|c| c.tsg);
                    out.dedup_by_key(|c| c.tsg); // one call per HOST group, as the mode is per group
                    out
                };
                let mut twins = pick(true);
                if twins.is_empty() {
                    twins = pick(false);
                }
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
                        Ok(format!("{client:#x}:{channel:#x} SET_CTXSW_PREEMPTION_MODE flags={flags:#x} gfxp={gfxp} cilp={cilp} on {} host group(s)", twins.len()))
                    }),
                )
            }
            ChanStatement::ZcullBind {
                client,
                channel,
                va,
                mode,
            } => {
                // GR twins only (zcull is GR context state), the channel or its whole group.
                let twins: Vec<kf_host::Channel> = self
                    .pt
                    .lock()
                    .map(|m| {
                        m.iter()
                            .filter(|(k, v)| {
                                k.0 == client
                                    && (k.1 == channel || v.tsg == Some(channel))
                                    && v.engine == kf_abi::submit::ENGINE_TYPE_GRAPHICS
                            })
                            .map(|(_, v)| v.chan)
                            .collect()
                    })
                    .unwrap_or_default();
                if twins.is_empty() {
                    return ChanAnswer::NotOurs;
                }
                self.defer(
                    "zcull bind",
                    Box::new(move |me: &ChanPlane| {
                        for c in &twins {
                            me.rm
                                .zcull_bind(*c, va, mode)
                                .map_err(|e| (NV_ERR_NOT_SUPPORTED, format!("twin host {:#x} ZCULL_BIND va={va:#x} mode={mode}: {e:?}", c.token)))?;
                        }
                        Ok(format!("{client:#x}:{channel:#x} ZCULL_BIND va={va:#x} mode={mode} on {} GR twin(s)", twins.len()))
                    }),
                )
            }
            ChanStatement::Timeslice { client, object, us } => {
                let mut twins: Vec<kf_host::Channel> = self
                    .pt
                    .lock()
                    .map(|m| {
                        m.iter()
                            .filter(|(k, v)| k.0 == client && v.tsg == Some(object))
                            .map(|(_, v)| v.chan)
                            .collect()
                    })
                    .unwrap_or_default();
                // A guest TSG can contain Translated channels as well as passthrough
                // twins. Resolve its members in this client's namespace, then use
                // only our owned host channels. No host call or wait holds these locks.
                let translated: Vec<u32> = self
                    .by_obj
                    .lock()
                    .map(|m| {
                        m.iter()
                            .filter(|(k, _)| k.0 == client)
                            .map(|(_, ht)| *ht)
                            .collect()
                    })
                    .unwrap_or_default();
                if twins.is_empty() && translated.is_empty() {
                    return ChanAnswer::NotOurs;
                }
                self.defer(
                    "timeslice",
                    Box::new(move |me: &ChanPlane| {
                        // Slot resolution runs on the act thread as well: it must
                        // not contend with a pump while the GSP drainer is locked.
                        for ht in translated {
                            if let Some(slot) = me.slot(ht)
                                && let Ok(g) = slot.lock()
                                && g.tsg == Some(object)
                                && g.dead.is_none()
                            {
                                twins.push(g.chan.host().channel());
                            }
                        }
                        if twins.is_empty() {
                            return Err((NV_ERR_INVALID_STATE, format!(
                                "{client:#x}:{object:#x} SET_TIMESLICE: no live owned group member"
                            )));
                        }
                        let mut groups_done = std::collections::HashSet::new();
                        for c in &twins {
                            if !groups_done.insert(c.tsg) {
                                continue;
                            }
                            me.rm.set_timeslice(*c, us).map_err(|e| {
                                (
                                    NV_ERR_INVALID_ARGUMENT,
                                    format!("twin host {:#x} SET_TIMESLICE {us}: {e:?}", c.token),
                                )
                            })?;
                        }
                        Ok(format!(
                            "{client:#x}:{object:#x} SET_TIMESLICE {us} us on {} twin group(s)",
                            groups_done.len()
                        ))
                    }),
                )
            }
            // ★ 2026-10-07 (Windows run34): `SET_CHANNEL_PROPERTIES` engine timeslice on ONE guest
            // channel — the host's `SET_TIMESLICE` on that channel's own host group (passthrough
            // twin or Translated ring), the same unprivileged verb as the TSG arm above. Resolved
            // in this client's namespace only; no host call or wait holds these locks.
            ChanStatement::ChannelTimeslice {
                client,
                channel,
                us,
            } => {
                let twin: Option<kf_host::Channel> = self
                    .pt
                    .lock()
                    .ok()
                    .and_then(|m| m.get(&(client, channel)).map(|v| v.chan));
                let translated: Option<u32> = self
                    .by_obj
                    .lock()
                    .ok()
                    .and_then(|m| m.get(&(client, channel)).copied());
                if twin.is_none() && translated.is_none() {
                    return ChanAnswer::NotOurs;
                }
                self.defer(
                    "channel timeslice",
                    Box::new(move |me: &ChanPlane| {
                        let c = twin
                            .or_else(|| {
                                let slot = me.slot(translated?)?;
                                let g = slot.lock().ok()?;
                                g.dead.is_none().then(|| g.chan.host().channel())
                            })
                            .ok_or_else(|| {
                                (
                                    NV_ERR_INVALID_STATE,
                                    format!(
                                        "{client:#x}:{channel:#x} SET_CHANNEL_PROPERTIES timeslice: no live owned host channel"
                                    ),
                                )
                            })?;
                        me.rm.set_timeslice(c, us).map_err(|e| {
                            (
                                NV_ERR_INVALID_ARGUMENT,
                                format!(
                                    "twin host {:#x} SET_TIMESLICE {us} (SET_CHANNEL_PROPERTIES): {e:?}",
                                    c.token
                                ),
                            )
                        })?;
                        Ok(format!(
                            "{client:#x}:{channel:#x} SET_CHANNEL_PROPERTIES ENGINE_TIMESLICE {us} us on host group {:#x} (twin {:#x})",
                            c.tsg, c.token
                        ))
                    }),
                )
            }
            ChanStatement::CudaLimit {
                client,
                device,
                enable,
            } => {
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
            ChanStatement::PromoteCtx {
                chan_client,
                object,
                engine_type,
                initialize,
                with_va,
                entries,
                falcon_ctx,
            } => {
                if let (Some(fc), Ok(mut m)) = (falcon_ctx, self.pt.lock())
                    && let Some(v) = m.get_mut(&(chan_client, object))
                {
                    v.falcon_ctx = Some(fc);
                }
                self.promote_ctx(
                    chan_client,
                    object,
                    engine_type,
                    initialize,
                    with_va,
                    entries,
                )
            }
            ChanStatement::EvictCtx {
                chan_client,
                object,
                engine_type,
            } => self.evict_ctx(chan_client, object, engine_type),
            ChanStatement::Stop {
                client,
                object,
                immediate,
            } => self.stop_channel(client, object, immediate),
            ChanStatement::DisableChannels {
                client,
                disable,
                only_scheduling,
                rewind_gp_put,
                list,
                preempt_event,
            } => self.disable_channels(
                client,
                disable,
                only_scheduling,
                rewind_gp_put,
                list.as_slice(),
                preempt_event,
            ),
            ChanStatement::Preempt {
                client,
                object,
                wait,
            } => self.preempt_group(client, object, wait),
            ChanStatement::EncoderSession { client, acquire } => {
                self.encoder_session(client, acquire)
            }
        }
    }

    /// The Translated slot of `(client, object)`, if the plane owns one: its host token.
    fn translated_of(&self, client: u32, object: u32) -> Option<u32> {
        self.by_obj
            .lock()
            .ok()
            .and_then(|m| m.get(&(client, object)).copied())
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
        let Some(chan) = self
            .pt
            .lock()
            .ok()
            .and_then(|m| m.get(&(client, object)).map(|v| v.chan))
        else {
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
    fn disable_channels(
        &self,
        client: u32,
        disable: bool,
        only_scheduling: bool,
        rewind: bool,
        list: &[(u32, u32)],
        preempt_event: Option<u64>,
    ) -> ChanAnswer {
        if list.is_empty() {
            // Vacuously true: RM disables nothing (the only in-tree caller never sends it).
            return ChanAnswer::Done;
        }
        let DisableList { pt, tr, unknown } = {
            let Ok(m) = self.pt.lock() else {
                return ChanAnswer::Refused {
                    status: NV_ERR_INVALID_STATE,
                    why: "twins poisoned".into(),
                };
            };
            resolve_disable_list(
                list,
                |c, h| m.get(&(c, h)).map(|v| v.chan),
                |c, h| {
                    m.iter()
                        .filter(|(k, v)| k.0 == c && v.tsg == Some(h))
                        .map(|(k, v)| (*k, v.chan))
                        .collect()
                },
                |c, h| self.translated_of(c, h),
            )
        };
        if pt.is_empty() && tr.is_empty() {
            return ChanAnswer::NotOurs;
        }
        if !unknown.is_empty() {
            return ChanAnswer::Refused {
                status: NV_ERR_INVALID_STATE,
                why: format!(
                    "DISABLE_CHANNELS names {} channel(s) with no host twin ({unknown:x?}); none disabled",
                    unknown.len()
                ),
            };
        }
        if rewind && !tr.is_empty() {
            return ChanAnswer::Refused {
                status: NV_ERR_INVALID_ARGUMENT,
                why: format!(
                    "DISABLE_CHANNELS bRewindGpPut on Translated channel(s) {tr:x?}: our host ring's GP_PUT is not the guest's"
                ),
            };
        }
        // ⚠ DIAGNOSTIC (2026-10-08, after run84; `KF3_ASYNC_PREEMPT_REENABLE_NOACT=1`, default off, NEVER
        // shipped): the coordinator's bisect — a re-enable of twins only (no Translated ring) is answered
        // NV_OK with NO host act, so the twins stay disabled on the host. It is a status the host did not
        // earn (a diagnostic, not a behaviour): it tells whether the host re-enable act itself is what
        // the guest's later timeout follows (run83 refused it and survived; run84 served it and TDR'd).
        if !disable && tr.is_empty() && reenable_noact() {
            let toks: Vec<String> = pt
                .iter()
                .map(|(k, c)| format!("{:#x}:{:#x}->host {:#x}", k.0, k.1, c.token))
                .collect();
            eprintln!(
                "kf3: ⚠ DIAGNOSTIC KF3_ASYNC_PREEMPT_REENABLE_NOACT: {client:#x} DISABLE_CHANNELS(bDisable=false) over twin(s) [{}] answered NV_OK with NO host act (the twins stay disabled on the host){}",
                toks.join(", "),
                self.relay_snapshots(&pt)
            );
            return ChanAnswer::Done;
        }
        self.defer(
            "disable channels",
            Box::new(move |me: &ChanPlane| {
                // ⚠ DIAGNOSTIC (after run84): which twins this act names, and each relay's cursors
                // before the host verb (host-derived reads of 4 bytes each; nothing forwarded).
                let before = me.relay_snapshots(&pt);
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
                // ★ 2026-10-08 (`KF3_ASYNC_PREEMPT`): the host's disable+preempt RETURNED — the host's
                // real completion — so the guest's RUNLIST_PREEMPT_COMPLETE may be posted now (by the
                // drainer, after this act's held reply). Never before, never without it.
                if let Some(ev) = preempt_event
                    && disable
                    && let Ok(mut q) = me.preempt_done.lock()
                    && q.len() < PREEMPT_DONE_MAX
                {
                    q.push((client, ev));
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
                let after = me.relay_snapshots(&pt);
                let toks: Vec<String> = pt
                    .iter()
                    .map(|(k, c)| format!("{:#x}:{:#x}->host {:#x}", k.0, k.1, c.token))
                    .collect();
                Ok(format!(
                    "{client:#x} DISABLE_CHANNELS(bDisable={disable}, bOnlyDisableScheduling={only_scheduling}, bRewindGpPut={rewind}) over {} twin(s) [{}] + {} Translated ring(s){}; relays before{before} after{after}",
                    pt.len(),
                    toks.join(", "),
                    tr.len(),
                    if preempt_event.is_some() {
                        " — async preempt: host preempt completed, RUNLIST_PREEMPT_COMPLETE queued (posted only after this reply)"
                    } else {
                        ""
                    }
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
            .map(|m| {
                m.iter()
                    .filter(|(k, v)| k.0 == client && v.tsg == Some(object))
                    .map(|(_, v)| v.chan)
                    .collect()
            })
            .unwrap_or_default();
        // ⊘ Group membership from the scopes table, never a slot lock on the drainer.
        let scopes = self.scopes.lock().map(|m| m.clone()).unwrap_or_default();
        let tr: Vec<u32> = self
            .by_obj
            .lock()
            .map(|m| {
                m.iter()
                    .filter(|(k, _)| {
                        k.0 == client && scopes.get(k).is_some_and(|sc| sc.tsg == Some(object))
                    })
                    .map(|(_, v)| *v)
                    .collect()
            })
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
    fn promote_ctx(
        &self,
        client: u32,
        object: u32,
        engine_type: u32,
        initialize: u32,
        with_va: u32,
        entries: u32,
    ) -> ChanAnswer {
        if let Some(ht) = self.translated_of(client, object) {
            return self.defer("promote translated context", Box::new(move |me: &ChanPlane| {
                let slot = me.slot(ht).ok_or_else(|| (NV_ERR_INVALID_STATE, "GR slot gone".into()))?;
                let mut g = slot.lock().map_err(|_| (NV_ERR_INVALID_STATE, "GR slot poisoned".into()))?;
                let context = g.chan.host().gr_context();
                let video = g.chan.host().video_context();
                if g.guest_engine != engine_type || !g.chan.host().owns_context(engine_type) || g.dead.is_some()
                    || (video.is_some() && entries != 0)
                {
                    return Err((NV_ERR_INVALID_ARGUMENT, "GPU_PROMOTE_CTX has no matching owned context or invalid video entries".into()));
                }
                // Owner ruling B: the actual host context was created at birth,
                // before this statement. No guest PA, VA or context byte is used.
                g.ctx.initialized |= initialize;
                g.ctx.va_bound |= with_va;
                g.ctx.bound |= with_va != 0;
                g.ctx.promotes = g.ctx.promotes.saturating_add(1);
                Ok(format!("{client:#x}:{object:#x} GPU_PROMOTE_CTX satisfied by Translated host {ht:#x} GR={context:x?} VIDEO={video:x?} entries={entries} init_ids={initialize:#x} va_ids={with_va:#x} bound={} — no guest buffer touched", g.ctx.bound))
            }));
        }
        let Some((engine, ht)) = self
            .pt
            .lock()
            .ok()
            .and_then(|m| m.get(&(client, object)).map(|v| (v.engine, v.chan.token)))
        else {
            // A channel without an owned host context stays the FSM's refusal.
            // Experimental kernel GR/NVDEC/NVENC paths require successful birth first.
            eprintln!(
                "kf3: chan {client:#x}:{object:#x} GPU_PROMOTE_CTX: no passthrough twin — not ours (entries={entries})"
            );
            return ChanAnswer::NotOurs;
        };
        // ★ A VIDEO twin's promote is the falcon context (`_kflcnPromoteContext`,
        // `kernel_falcon.c:184-276`: `entryCount = 0`, the guest VA only). Host RM allocated and
        // promoted the twin's own falcon context with its engine object, so it is satisfied by the
        // twin — nothing sent, no guest byte touched.
        if kf_abi::submit::is_video_engine_type(engine) && engine_type == engine {
            let Ok(mut m) = self.pt.lock() else {
                return ChanAnswer::Refused {
                    status: NV_ERR_INVALID_STATE,
                    why: "twins poisoned".into(),
                };
            };
            let Some(v) = m.get_mut(&(client, object)) else {
                return ChanAnswer::NotOurs;
            };
            v.ctx.promotes = v.ctx.promotes.saturating_add(1);
            eprintln!(
                "kf3: chan {client:#x}:{object:#x} GPU_PROMOTE_CTX (video falcon ctx, engine {engine:#x}) SATISFIED BY TWIN host {ht:#x}: entries={entries} host_objects={} — not forwarded",
                v.objects.len()
            );
            return ChanAnswer::Done;
        }
        if engine != kf_abi::submit::ENGINE_TYPE_GRAPHICS || engine_type != engine {
            return ChanAnswer::Refused {
                status: NV_ERR_INVALID_ARGUMENT,
                why: format!(
                    "GPU_PROMOTE_CTX engine {engine_type:#x} for a twin born on engine {engine:#x} (host {ht:#x})"
                ),
            };
        }
        if with_va == 0 {
            let Ok(mut m) = self.pt.lock() else {
                return ChanAnswer::Refused {
                    status: NV_ERR_INVALID_STATE,
                    why: "twins poisoned".into(),
                };
            };
            let Some(v) = m.get_mut(&(client, object)) else {
                return ChanAnswer::NotOurs;
            };
            v.ctx.initialized |= initialize;
            v.ctx.promotes = v.ctx.promotes.saturating_add(1);
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
                // ★ v3-gfx: a graphics object's promote ARRIVES BEFORE its alloc — the guest's CPU-RM
                // maps the context buffers into the channel's VA space and promotes them inside
                // `_kgrAlloc` (`kernel_graphics_object.c:224`), and only then RPCs the object.
                // `[measured vgfx 2026-09-26, gfx3]` every 3D channel's promote hit an empty twin.
                // The stub stands (owner ruling #3): host RM builds the twin's OWN context when that
                // engine object is allocated on it, a moment later; the guest's buffers are never
                // read or written either way. So an empty twin is "pending", not a refusal.
                let pending = host_ctx.is_empty();
                v.ctx.initialized |= initialize;
                v.ctx.va_bound |= with_va;
                v.ctx.bound |= with_va != 0;
                v.ctx.promotes = v.ctx.promotes.saturating_add(1);
                Ok(format!(
                    "chan {client:#x}:{object:#x} GPU_PROMOTE_CTX SATISFIED BY TWIN host {ht:#x} (host GR object(s) {host_ctx:x?}{}): entries={entries} init_ids={initialize:#x} va_ids={with_va:#x} bound={} — not forwarded, no guest byte touched",
                    if pending { " — none yet: the engine object this promote precedes births the host context" } else { "" },
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
        if let Some(ht) = self.translated_of(client, object) {
            return self.defer("evict translated context", Box::new(move |me: &ChanPlane| {
                let slot = me.slot(ht).ok_or_else(|| (NV_ERR_INVALID_STATE, "GR slot gone".into()))?;
                let host = {
                    let g = slot.lock().map_err(|_| (NV_ERR_INVALID_STATE, "GR slot poisoned".into()))?;
                    if g.guest_engine != engine_type || !g.chan.host().owns_context(engine_type)
                    {
                        return Err((NV_ERR_INVALID_ARGUMENT, "GPU_EVICT_CTX has no matching owned context".into()));
                    }
                    g.chan.host().channel()
                };
                me.rm.schedule_enable(host, false).map_err(|e| (NV_ERR_INVALID_STATE, format!("GR host {ht:#x} evict: {e:?}")))?;
                let mut g = slot.lock().map_err(|_| (NV_ERR_INVALID_STATE, "GR slot poisoned".into()))?;
                g.scheduled = false;
                g.ctx.bound = false;
                g.ctx.va_bound = 0;
                g.ctx.evicts = g.ctx.evicts.saturating_add(1);
                Ok(format!("{client:#x}:{object:#x} GPU_EVICT_CTX: Translated host {ht:#x} off runlist, context UNBOUND"))
            }));
        }
        let Some((engine, chan)) = self
            .pt
            .lock()
            .ok()
            .and_then(|m| m.get(&(client, object)).map(|v| (v.engine, v.chan)))
        else {
            return ChanAnswer::NotOurs;
        };
        if engine != kf_abi::submit::ENGINE_TYPE_GRAPHICS || engine_type != engine {
            return ChanAnswer::Refused {
                status: NV_ERR_INVALID_ARGUMENT,
                why: format!(
                    "GPU_EVICT_CTX engine {engine_type:#x} for a twin born on engine {engine:#x} (host {:#x})",
                    chan.token
                ),
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
                        v.ctx.evicts = v.ctx.evicts.saturating_add(1);
                        v.ctx
                    })
                });
                Ok(format!("chan {client:#x}:{object:#x} GPU_EVICT_CTX: twin host {:#x} off the runlist, context UNBOUND ({ctx:?})", chan.token))
            }),
        )
    }

    fn schedule_translated(&self, client: u32, object: u32, ht: u32, enable: bool) -> ChanAnswer {
        let Some(slot) = self.slot(ht) else {
            return ChanAnswer::NotOurs;
        };
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
            Err(_) => {
                return ChanAnswer::Refused {
                    status: NV_ERR_INVALID_STATE,
                    why: "slot poisoned".into(),
                };
            }
        };
        eprintln!(
            "kf3: chan {client:#x}:{object:#x} GPFIFO_SCHEDULE enable={enable} (token {idx:#x}, host {ht:#x})"
        );
        if enable {
            self.bar0trace.schedule_served(crate::bar0trace::TraceChan {
                client,
                object,
                host: ht,
            });
        }
        // Work the guest queued before scheduling is picked up now.
        if enable && self.plane.ring_internal(idx) {
            let _ = self.wake.signal();
        }
        ChanAnswer::Done
    }

    /// ★ P5b: an engine object under a PASSTHROUGH twin — allocated on the twin with the guest's
    /// class (checked against the HOST family's generated set for the twin's engine) and params
    /// we author. Under a Translated channel it is a graph node only (our ring owns its object).
    fn engine_object(
        &self,
        client: u32,
        parent: u32,
        handle: u32,
        class: u32,
        copy_engine: Option<u32>,
    ) -> ChanAnswer {
        let Some((chan, engine, space, rows, ledger)) = self.pt.lock().ok().and_then(|m| {
            m.get(&(client, parent))
                .map(|v| (v.chan, v.engine, v.space, v.rows.clone(), v.ledger.clone()))
        }) else {
            return ChanAnswer::NotOurs;
        };
        let Some(kind) = kf_chip::classes_for(self.family).kind_of(class) else {
            return ChanAnswer::Refused {
                status: NV_ERR_INVALID_CLASS,
                why: format!(
                    "class {class:#x} is not an engine class of the host family {:?}",
                    self.family
                ),
            };
        };
        let d = kf_gsp::Deferred::new();
        let step = EngineObj {
            client,
            parent,
            handle,
            class,
            copy_engine,
            chan,
            engine,
            space,
            rows,
            ledger,
            kind,
        };
        self.defer_cell("engine object", step.act(0, None, None, d.clone()), d)
    }

    /// ★ EXPERIMENT `x11-dispsw` (default off; carried only when the device property is on,
    /// `kf_rm::chanlink::ChannelPolicy::with_display_sw_twins`): the guest's `GF100_DISP_SW` object
    /// under its channel, twinned under that channel's PASSTHROUGH twin with params WE author
    /// (`kf_host::HostRm::alloc_disp_sw`: head 0, displayMask 0, caps 0 — the guest's own params
    /// never reach this plane), carrying the guest's own FIFO software classID — or refused by name.
    /// Every decision is `crate::dispsw`'s; this wires it to the maps and the host session:
    /// 1. Statement time ([`crate::dispsw::admit`], no host call): the channel's mirrored numbering
    ///    advances (the guest numbered the object whatever we answer); no twin, a handle already
    ///    twinned, or a lost mirror is refused here.
    /// 2. The act ([`dispsw_live`], then [`crate::dispsw::twin_watched`]): a twin marked by an
    ///    earlier host refusal is refused, then the caps, the host alloc, its number read back and
    ///    made the guest's or refused; a host alloc refused here marks the twin
    ///    ([`dispsw_mark_host_refused`]); then [`crate::dispsw::keep_disp_sw`] /
    ///    [`crate::dispsw::settle_keep`].
    /// 3. The undo ([`attach_dispsw_withdraw`]): when the act kept a twin and the object seat then
    ///    refused the alloc (a handle the guest already uses for a non-twinned object), an act
    ///    queued behind it withdraws exactly that twin ([`withdraw_kept`]).
    fn display_sw(&self, client: u32, parent: u32, handle: u32) -> ChanAnswer {
        let key = (client, parent);
        let admitted = match (self.pt.lock(), self.pt_objs.lock()) {
            (Ok(mut pt), Ok(objs)) => {
                let v = pt.get_mut(&key);
                let chan = v.as_ref().map(|v| v.chan);
                crate::dispsw::admit(
                    v.map(|v| &mut v.sw_ids),
                    &objs,
                    client,
                    handle,
                    &self.dispsw,
                )
                .map(|n| (chan, n))
            }
            _ => Err((NV_ERR_INVALID_STATE, "plane maps poisoned".into())),
        };
        // Lock order here and in the act: `pt`, then `pt_objs` (no site holds `pt_objs` while it
        // takes `pt`). `admit` refuses a channel with no twin, so `chan` is `Some` with every `Ok`.
        let (chan, expect) = match admitted {
            Ok((Some(chan), n)) => (chan, n),
            Ok((None, _)) => {
                return ChanAnswer::Refused {
                    status: crate::dispsw::NV_ERR_NOT_SUPPORTED,
                    why: format!(
                        "GF100_DISP_SW {client:#x}:{handle:#x} under {parent:#x}: no twin"
                    ),
                };
            }
            Err((status, why)) => {
                return ChanAnswer::Refused {
                    status,
                    why: format!("GF100_DISP_SW {client:#x}:{handle:#x} under {parent:#x}: {why}"),
                };
            }
        };
        // The host object the act kept (0: none) — read by the undo, which runs after the act.
        let kept = Arc::new(AtomicU32::new(0));
        let kept_by_act = kept.clone();
        let answer = self.defer(
            "display-SW twin",
            Box::new(move |me: &ChanPlane| {
                let live = dispsw_live(&me.pt, key);
                let (twinned, host_refused) =
                    crate::dispsw::twin_watched(me.rm, chan, expect, live, &me.dispsw);
                let marked = host_refused
                    && !live.host_refused_before
                    && dispsw_mark_host_refused(&me.pt, key, chan);
                let (h, how) = twinned.map_err(|(st, why)| {
                    let mark = if marked {
                        "; this twin now takes no more display-SW objects (its host numbering is no longer known)"
                    } else {
                        ""
                    };
                    (st, format!("GF100_DISP_SW {client:#x}:{handle:#x} on twin host {:#x}: {why}{mark}", chan.token))
                })?;
                let keep = match (me.pt.lock(), me.pt_objs.lock()) {
                    (Ok(mut pt), Ok(mut objs)) => crate::dispsw::keep_disp_sw(
                        pt.get_mut(&key).map(|v| &mut v.disp_sw),
                        &mut objs,
                        key,
                        handle,
                        h,
                    ),
                    _ => crate::dispsw::DispSwKeep::ChannelGone,
                };
                if keep == crate::dispsw::DispSwKeep::Kept {
                    kept_by_act.store(h, Ordering::Release);
                }
                let what = crate::dispsw::settle_keep(me.rm, keep, h, &me.dispsw)
                    .map_err(|(st, why)| (st, format!("GF100_DISP_SW {client:#x}:{handle:#x}: {why}")))?;
                Ok(format!(
                    "{client:#x}:{handle:#x} GF100_DISP_SW on twin host {:#x} -> host object {h:#x} (authored: head 0, displayMask 0, caps 0; {how}) [{what}]",
                    chan.token
                ))
            }),
        );
        if let ChanAnswer::Deferred(d) = &answer
            && let Some(tx) = self.acts.lock().ok().and_then(|a| a.clone())
        {
            attach_dispsw_withdraw(d, tx, kept, key, handle);
        }
        answer
    }

    /// ★ EXPERIMENT `x11-dispsw` (review 2026-10-03, MEDIUM): the guest allocated another `ENG_SW`
    /// object under a channel (`kf_rm::chanlink::OTHER_ENG_SW_CHANNEL_CLASSES`). Never twinned — but
    /// the guest numbered it, so the twin's mirror advances and the channel's next display-SW twin
    /// is repaid up to the guest's number. Observed only: the link ignores the answer.
    /// ★ OWNER_RULINGS §U: attach the VM's deferred-API tables (the object seat's registry).
    pub fn set_deferred_api(&self, reg: Arc<kf_rm::defapi::Registry>) {
        let _ = self.defapi_reg.set(reg);
    }

    /// ★ §U: the host token of the Translated channel `(client, parent)`, if it is one.
    fn translated_ht(&self, client: u32, parent: u32) -> Option<u32> {
        self.by_obj
            .lock()
            .ok()
            .and_then(|m| m.get(&(client, parent)).copied())
    }

    /// ★ §U: the guest gave an `ENG_SW` child of `(client, parent)` the channel's next software
    /// classID: mirror it when that channel is one of our Translated ones.
    fn number_translated_sw(
        &self,
        client: u32,
        parent: u32,
        class: u32,
        handle: Option<u32>,
    ) -> Option<u16> {
        let ht = self.translated_ht(client, parent)?;
        let mut m = self.sw_objs.lock().ok()?;
        let o = m.get_mut(&ht)?;
        let n = o.number(class, handle);
        eprintln!(
            "kf3: chan {client:#x}:{parent:#x} (host {ht:#x}): class {class:#x} handle {handle:x?} took software classID {n:?}"
        );
        n
    }

    /// ★★ OWNER_RULINGS §U.1: an `NV50_DEFERRED_API_CLASS` alloc — admitted (and numbered) under a
    /// Translated channel, refused `NV_ERR_NOT_SUPPORTED` under a Passthrough twin.
    fn deferred_api_object(&self, client: u32, parent: u32, handle: u32) -> ChanAnswer {
        let (passthrough, user_work) = self
            .pt
            .lock()
            .ok()
            .and_then(|m| m.get(&(client, parent)).map(|v| (true, v.user_work)))
            .unwrap_or((false, false));
        if passthrough {
            // The guest numbered it anyway: keep the twin's display-SW mirror equal.
            let _ = register_other_sw(&self.pt, (client, parent));
        }
        if user_work {
            // ★★ 2026-10-08 (§V, `KF3_WIN_USER_CHANNELS_PASSTHROUGH`): Windows allocates a 5080 on
            // EVERY channel, its per-process ones included; refusing it fails the D3D device
            // creation. On a Passthrough twin nothing ever reads the ring, so the object is a
            // guest-graph node only: no host object, no trigger. A `0x200` the guest's bytes
            // carry reaches the host engine as a software method with no host object behind it,
            // and the host RCs that channel (fail closed). `[measured, runs 47-57]` Windows' deferred
            // triggers all ran on its kernel GR channel.
            if twin_defapi_plan(twin_defapi_object(), user_work) == TwinDefapiPlan::AuthorHostObject
            {
                return self.twin_defapi_act(client, parent, handle);
            }
            eprintln!(
                "kf3: chan {client:#x}:{parent:#x}: NV50_DEFERRED_API {handle:#x} ADMITTED as a guest-graph object on a Windows user-work Passthrough twin (never triggered by kayfabe; §V)"
            );
            return ChanAnswer::Done;
        }
        let ht = self.translated_ht(client, parent);
        let verdict = {
            let mut m = self.sw_objs.lock();
            let objs = match (&mut m, ht) {
                (Ok(m), Some(h)) => m.get_mut(&h),
                _ => None,
            };
            crate::defapi::admit(objs, passthrough, client, parent, handle)
        };
        match verdict {
            crate::defapi::Admission::Admitted(n) => {
                eprintln!(
                    "kf3: chan {client:#x}:{parent:#x}: NV50_DEFERRED_API {handle:#x} ADMITTED on Translated host {:#x}, software classID {n:?} (OWNER_RULINGS §U.1)",
                    ht.unwrap_or(0)
                );
                ChanAnswer::Done
            }
            crate::defapi::Admission::Refused(status, why) => ChanAnswer::Refused { status, why },
        }
    }

    /// ⚠ EXPERIMENT (2026-10-08, `KF3_WIN_TWIN_DEFAPI_OBJECT=1`, default off — an owner decision is
    /// pending: OWNER_RULINGS §U.1 says the class is Translated-only). `[measured, run73 at 3f23995a]`
    /// every D3D device copy channel of Windows 580.88 binds its `NV50_DEFERRED_API` to SOFTWARE
    /// subchannel 5 (`SET_OBJECT` data `0x5080`) early in its stream; on an unprivileged host twin with
    /// no such object, host RM answers that software method with an RC (Xid 32, PBDMA `DEVICE`).
    ///
    /// What this does: one host `NV50_DEFERRED_API` under the twin's host channel, AUTHORED from the
    /// guest's own alloc of that class on that channel (no params: `RS_OPTIONAL(NV5080_ALLOC_PARAMS)`,
    /// `resource_list.h:1524-1528`; the guest's params never reach the host), so the guest's
    /// `SET_OBJECT` on its software subchannel finds an object of that class on the twin (RM matches a
    /// software `SET_OBJECT` by external class or software classID, `kernel_channel.c:3433-3436`).
    /// What it does NOT do: register any deferred entry on the host object (the guest's
    /// `NV5080_CTRL_CMD_DEFERRED_API*` controls stay guest-graph only, §U.4 undecided), so a `0x200`
    /// trigger on that subchannel finds nothing registered and host RM fails it (fail closed, an RC of
    /// that twin only); no host action is ever taken from a guest word. At most one per twin.
    fn twin_defapi_act(&self, client: u32, parent: u32, handle: u32) -> ChanAnswer {
        self.defer(
            "deferred-API twin object",
            Box::new(move |me: &ChanPlane| {
                let chan = {
                    let m = me
                        .pt
                        .lock()
                        .map_err(|_| (NV_ERR_NOT_SUPPORTED, "pt poisoned".to_string()))?;
                    let v = m
                        .get(&(client, parent))
                        .ok_or((NV_ERR_NOT_SUPPORTED, "the twin is gone".to_string()))?;
                    if let Some((g, h)) = v.defapi_host {
                        return Ok(format!(
                            "{client:#x}:{handle:#x}: twin host {:#x} already carries the host NV50_DEFERRED_API {h:#x} (for guest {g:#x}); this one is guest-graph only",
                            v.chan.token
                        ));
                    }
                    v.chan
                };
                let h = me.rm.alloc_deferred_api(chan).map_err(|e| {
                    (
                        NV_ERR_NOT_SUPPORTED,
                        format!(
                            "EXPERIMENT KF3_WIN_TWIN_DEFAPI_OBJECT: the host refused the authored NV50_DEFERRED_API ({e:?})"
                        ),
                    )
                })?;
                if let Ok(mut m) = me.pt.lock()
                    && let Some(v) = m.get_mut(&(client, parent))
                {
                    v.defapi_host = Some((handle, h));
                }
                Ok(format!(
                    "EXPERIMENT KF3_WIN_TWIN_DEFAPI_OBJECT: {client:#x}:{handle:#x} NV50_DEFERRED_API on Windows user-work twin host {:#x} -> host object {h:#x} (authored, no params, NOTHING registered: a 0x200 on it fails closed)",
                    chan.token
                ))
            }),
        )
    }

    /// ★★ OWNER_RULINGS §U: a deferred INITIALIZE/PROMOTE/EVICT — on the plane's act thread (it
    /// takes other channels' slot locks and, for an evict, a host verb); its outcome goes to `cell`
    /// and the triggering channel's token `ring` is rung. Never a wait on the worker.
    ///
    /// # Errors
    /// The act thread is not running.
    fn deferred_ctx_act(
        &self,
        planned: crate::defapi::Planned,
        cell: SwCell,
        ring: u32,
    ) -> Result<(), String> {
        let act: Act = Box::new(move |me: &ChanPlane| {
            let r = me.deferred_ctx(&planned);
            if let Ok(mut c) = cell.lock() {
                *c = Some(r.clone().map_err(|(st, why)| format!("{why} ({st:#x})")));
            }
            if me.plane.ring_internal(ring) {
                let _ = me.wake.signal();
            }
            r
        });
        let sent = self.acts.lock().ok().and_then(|a| {
            a.as_ref().map(|tx| {
                tx.send((act, kf_gsp::Deferred::new(), "deferred API context"))
                    .is_ok()
            })
        });
        if sent == Some(true) {
            Ok(())
        } else {
            Err("deferred API context: the act thread is not running".into())
        }
    }

    /// The act's body: resolve the bundle's `(hChanClient, hObject)` to our Translated channels
    /// (the channel, or every member of the TSG it names), judge by the direct path's predicate,
    /// and perform the effect.
    fn deferred_ctx(&self, planned: &crate::defapi::Planned) -> Result<String, (u32, String)> {
        let (chan_client, object) = match planned.entry.decoded {
            kf_abi::defapi::Bundle::InitializeCtx {
                chan_client,
                object,
                ..
            }
            | kf_abi::defapi::Bundle::PromoteCtx {
                chan_client,
                object,
                ..
            }
            | kf_abi::defapi::Bundle::EvictCtx {
                chan_client,
                object,
                ..
            } => (chan_client, object),
            _ => return Err((NV_ERR_INVALID_ARGUMENT, "not a context command".into())),
        };
        let named: Vec<u32> = {
            let scopes = self
                .scopes
                .lock()
                .map_err(|_| (NV_ERR_INVALID_STATE, "scopes poisoned".to_string()))?;
            let by_obj = self
                .by_obj
                .lock()
                .map_err(|_| (NV_ERR_INVALID_STATE, "by_obj poisoned".to_string()))?;
            scopes
                .iter()
                .filter(|((c, h), sc)| {
                    *c == chan_client && (*h == object || sc.tsg == Some(object))
                })
                .filter_map(|(k, _)| by_obj.get(k).copied())
                .collect()
        };
        if named.is_empty()
            && let Some(r) =
                self.deferred_ctx_passthrough(&planned.entry.decoded, chan_client, object)
        {
            return r;
        }
        let mut targets = Vec::new();
        for ht in &named {
            let Some(slot) = self.slot(*ht) else { continue };
            let g = slot
                .lock()
                .map_err(|_| (NV_ERR_INVALID_STATE, "slot poisoned".to_string()))?;
            targets.push(crate::defapi::Target {
                ht: *ht,
                engine: g.guest_engine,
                owns_context: g.chan.host().owns_context(g.guest_engine),
                dead: g.dead.is_some(),
            });
        }
        let (effect, hit) = crate::defapi::ctx_verdict(&planned.entry.decoded, &targets)
            .map_err(|why| (NV_ERR_INVALID_ARGUMENT, why))?;
        for ht in &hit {
            let Some(slot) = self.slot(*ht) else { continue };
            match effect {
                crate::defapi::CtxEffect::Satisfied => {
                    let mut g = slot
                        .lock()
                        .map_err(|_| (NV_ERR_INVALID_STATE, "slot poisoned".to_string()))?;
                    g.ctx.promotes = g.ctx.promotes.saturating_add(1);
                }
                crate::defapi::CtxEffect::Evict => {
                    let host = slot
                        .lock()
                        .map_err(|_| (NV_ERR_INVALID_STATE, "slot poisoned".to_string()))?
                        .chan
                        .host()
                        .channel();
                    self.rm.schedule_enable(host, false).map_err(|e| {
                        (NV_ERR_INVALID_STATE, format!("host {ht:#x} evict: {e:?}"))
                    })?;
                    let mut g = slot
                        .lock()
                        .map_err(|_| (NV_ERR_INVALID_STATE, "slot poisoned".to_string()))?;
                    g.scheduled = false;
                    g.ctx.bound = false;
                    g.ctx.va_bound = 0;
                    g.ctx.evicts = g.ctx.evicts.saturating_add(1);
                }
            }
        }
        Ok(format!(
            "{:?} on {chan_client:#x}:{object:#x} -> Translated host(s) {hit:x?}: {} (owner ruling B / §U; no guest buffer touched)",
            planned.entry.decoded,
            match effect {
                crate::defapi::CtxEffect::Satisfied =>
                    "satisfied by each twin's own host context, created and golden-initialised by host RM at its birth",
                crate::defapi::CtxEffect::Evict => "host ring(s) off the runlist, context UNBOUND",
            }
        ))
    }

    /// ★★ 2026-10-08 (§V): a deferred INITIALIZE/PROMOTE/EVICT whose target is a Windows user-work
    /// Passthrough twin (the channel, or every member of the TSG it names). INITIALIZE and PROMOTE are
    /// satisfied by the twin's own host context (ruling B, as for Translated twins); EVICT takes the
    /// twin's host channel off its runlist (the direct path's act, `evict_ctx`). `None` when no
    /// user-work twin matches (the caller's Translated path then decides).
    fn deferred_ctx_passthrough(
        &self,
        bundle: &kf_abi::defapi::Bundle,
        chan_client: u32,
        object: u32,
    ) -> Option<Result<String, (u32, String)>> {
        let hit: Vec<((u32, u32), kf_host::Channel, u32)> = self
            .pt
            .lock()
            .ok()?
            .iter()
            .filter(|((c, h), v)| {
                *c == chan_client && v.user_work && (*h == object || v.tsg == Some(object))
            })
            .map(|(k, v)| (*k, v.chan, v.engine))
            .collect();
        if hit.is_empty() {
            return None;
        }
        let evict = matches!(bundle, kf_abi::defapi::Bundle::EvictCtx { .. });
        for (k, chan, engine) in &hit {
            if *engine != kf_abi::submit::ENGINE_TYPE_GRAPHICS {
                continue;
            }
            if evict && let Err(e) = self.rm.schedule_enable(*chan, false) {
                return Some(Err((
                    NV_ERR_INVALID_STATE,
                    format!("Passthrough twin host {:#x} evict: {e:?}", chan.token),
                )));
            }
            if let Ok(mut m) = self.pt.lock()
                && let Some(v) = m.get_mut(k)
            {
                if evict {
                    v.ctx.bound = false;
                    v.ctx.va_bound = 0;
                    v.ctx.evicts = v.ctx.evicts.saturating_add(1);
                } else {
                    v.ctx.promotes = v.ctx.promotes.saturating_add(1);
                }
            }
        }
        Some(Ok(format!(
            "{bundle:?} on {chan_client:#x}:{object:#x} -> Windows user-work Passthrough twin(s) {:x?}: {} (owner ruling B / §U / §V)",
            hit.iter().map(|h| h.1.token).collect::<Vec<_>>(),
            if evict {
                "host channel(s) off the runlist, context UNBOUND"
            } else {
                "satisfied by each twin's own host context"
            }
        )))
    }

    fn software_object(&self, client: u32, parent: u32, class: u32) -> ChanAnswer {
        if let Some(n) = register_other_sw(&self.pt, (client, parent)) {
            self.dispsw.other_sw.fetch_add(1, Ordering::Relaxed);
            eprintln!(
                "kf3: chan {client:#x}:{parent:#x}: class {class:#x} took the guest's software classID {} on this channel (no host twin takes one; its next display-SW twin is repaid to match)",
                n.map_or_else(
                    || "(past 65535: the mirror lost it)".to_string(),
                    |n| n.to_string()
                )
            );
        }
        ChanAnswer::NotOurs
    }

    /// ★ x11-dispsw: the status line's `dispsw[...]` (`""` with the switch off) — `live=` is the
    /// maps' own count, so it cannot drift from them.
    #[must_use]
    pub fn dispsw_status(&self, on: bool) -> String {
        if !on {
            return String::new();
        }
        let live = self
            .pt
            .lock()
            .map_or(0, |m| m.values().map(|v| v.disp_sw.len()).sum());
        self.dispsw.status(true, live)
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
    fn debugger(
        &self,
        client: u32,
        parent: u32,
        handle: u32,
        app_client: u32,
        obj3d: u32,
    ) -> ChanAnswer {
        if app_client != client {
            return ChanAnswer::Refused {
                status: NV_ERR_INVALID_ARGUMENT,
                why: format!(
                    "GT200_DEBUGGER in client {client:#x} over another client's ({app_client:#x}) object"
                ),
            };
        }
        let host_obj = self
            .pt_objs
            .lock()
            .ok()
            .and_then(|m| m.get(&(client, obj3d)).copied())
            .and_then(|key| {
                self.pt
                    .lock()
                    .ok()
                    .and_then(|m| m.get(&key).and_then(|v| v.objects.get(&obj3d).copied()))
            });
        let host_obj = match host_obj {
            Some((h, kf_chip::classes::Kind::Compute | kf_chip::classes::Kind::ThreeD)) => h,
            Some((_, k)) => {
                return ChanAnswer::Refused {
                    status: NV_ERR_INVALID_ARGUMENT,
                    why: format!(
                        "GT200_DEBUGGER over {client:#x}:{obj3d:#x}, a {k:?} object — not GR"
                    ),
                };
            }
            None => {
                return ChanAnswer::Refused {
                    status: NV_ERR_INVALID_ARGUMENT,
                    why: format!(
                        "GT200_DEBUGGER over {client:#x}:{obj3d:#x}: no passthrough twin holds that object"
                    ),
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
        // ★ 2026-10-09: a freed TSG (or client) is no longer scheduled — its handle may come back
        // as a new group, whose first channel must not be scheduled from the old one's state.
        if let Ok(mut s) = self.guest_sched.lock() {
            s.forget(client, object);
        }
        // ★ §U: a freed 5080 names nothing any more; a freed channel's numbering goes with it.
        if let (Ok(mut m), Ok(scopes)) = (self.sw_objs.lock(), self.scopes.lock()) {
            m.retain(|_, o| {
                let gone = scopes
                    .get(&o.owner)
                    .is_some_and(|sc| sc.freed_by(o.owner, client, object))
                    || (o.owner.0 == client && (o.owner.1 == object || object == client));
                if !gone && o.owner.0 == client {
                    o.forget(object);
                }
                !gone
            });
        }
        // ★ w827: debugger sessions go FIRST — freed by name, by their device, or by their client,
        // or because the GR object they are bound to is about to go with its twin.
        let doomed_twin_objs: Vec<u32> = self
            .pt
            .lock()
            .map(|m| {
                m.iter()
                    .filter(|(k, v)| {
                        ChanScope {
                            tsg: v.tsg,
                            parent: v.parent,
                            device: v.device,
                        }
                        .freed_by(**k, client, object)
                    })
                    .flat_map(|(_, v)| v.objects.values().map(|o| o.0).collect::<Vec<_>>())
                    .collect()
            })
            .unwrap_or_default();
        let freed_obj = self
            .pt_objs
            .lock()
            .ok()
            .and_then(|m| m.get(&(client, object)).copied())
            .and_then(|key| {
                self.pt.lock().ok().and_then(|m| {
                    m.get(&key)
                        .and_then(|v| v.objects.get(&object).map(|o| o.0))
                })
            });
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
                keys.into_iter()
                    .filter_map(|k| m.remove(&k).map(|v| v.1))
                    .collect()
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

    fn free_channels(
        &self,
        client: u32,
        object: u32,
        debuggers: Vec<u32>,
        limit_off: bool,
    ) -> ChanAnswer {
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
                        let sc = scopes.get(k).copied().unwrap_or(ChanScope {
                            tsg: None,
                            parent: k.1,
                            device: k.1,
                        });
                        sc.freed_by(*k, client, object)
                    })
                    .collect();
                keys.into_iter().filter_map(|k| m.remove(&k)).collect()
            })
            .unwrap_or_default();
        if let Ok(mut m) = self.scopes.lock() {
            m.retain(|k, sc| !sc.freed_by(*k, client, object));
        }
        let twins = self.take_pt(|k, v| {
            ChanScope {
                tsg: v.tsg,
                parent: v.parent,
                device: v.device,
            }
            .freed_by(*k, client, object)
        });
        // Engine objects freed on their own (their twin still lives) — and, x11-dispsw only, a
        // display-SW twin (`true`).
        let obj = if twins.is_empty() {
            self.pt_objs
                .lock()
                .ok()
                .and_then(|mut m| m.remove(&(client, object)))
                .and_then(|key| {
                    self.pt.lock().ok().and_then(|mut m| {
                        m.get_mut(&key)
                            .and_then(|v| take_twin_object(&mut v.objects, &mut v.disp_sw, object))
                    })
                })
        } else {
            None
        };
        // ★ x11-dispsw: display-SW twins that go WITH their channel's twin (host RM frees them with
        // it) leave the maps with it, so `live=` drops at once; a refused channel free is counted
        // as leaving them behind on the host (`free_refused`, `crate::dispsw::released_with_channel`).
        let sessions = client == object
            && self
                .enc_sessions
                .lock()
                .is_ok_and(|m| m.get(&client).is_some_and(|n| *n > 0));
        if translated.is_empty()
            && twins.is_empty()
            && obj.is_none()
            && debuggers.is_empty()
            && !limit_off
            && !sessions
        {
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
            // ★ P1+P2 inc D, review fix 2026-10-04 (§4.2): in statement order the twin's user
            // moves from live to FREEING; the act ends it only once host RM freed the channel.
            for (_, t) in &twins {
                if let Some(tw) = &t.twin {
                    tw.user_freeing();
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
                    // ★ The fast path goes FIRST: its placements removed, its eventfd's last count
                    // delivered (to the token word, retired on the drainer at the statement — so
                    // absorbed), acknowledged — only then may the twin go and the token be reborn.
                    // ⚠ DIAGNOSTIC (`KF3_PT_STALL_SNAPSHOT=1`, bounded per boot): the twin's state
                    // before it goes — a TDR teardown frees the stalled twins here.
                    if pt_stall_snapshot_ms().is_some() {
                        static FREES: AtomicU32 = AtomicU32::new(0);
                        if FREES.fetch_add(1, Ordering::Relaxed) < SNAP_FREE_MAX {
                            eprintln!("kf3: PT-SNAP BEGIN at free of tok={:#x} (maplog t={:.6})", t.idx, kf_mem::maplog::t());
                            me.pt_snapshot_twin(c, h, &t);
                            eprintln!("kf3: PT-SNAP END");
                        }
                    }
                    let fast = me.dbfast.deregister(t.idx);
                    let r = me.release_twin(c, t.tsg, t.ctx_share, t.chan);
                    t.live.fetch_sub(1, Ordering::AcqRel);
                    // ★ P1+P2 inc D, review fix 2026-10-04 (§4.2): the twin's user ends only if
                    // host RM really freed the channel. A refused free leaves it counted — the
                    // space can then never become a guest-KERNEL space, where privileged leaves
                    // would be mirrored under a channel that may still run.
                    if let Some(tw) = &t.twin {
                        if r.is_ok() {
                            tw.user_released();
                        } else {
                            line.push(format!(
                                "twin {:#x} stays counted (its host free was refused): its space never turns kernel",
                                t.idx
                            ));
                        }
                    }
                    // The error context goes AFTER its channel (host RM refuses freeing a context
                    // DMA a live channel names as its error context).
                    if let Some(n) = t.notifier {
                        me.release_notifier(n);
                    }
                    me.engine_live(t.engine, false);
                    // ★ The per-token hardware ledger (`run_fast_guest.sh` gates on it): every ring
                    // of a Passthrough token IS a host doorbell, so `emulated` is only the rings
                    // that failed to reach the host — never a CPU executor.
                    // `rung`/`forwarded` cover BOTH transports (the grader's rule is per token and
                    // must not lose the doorbells the trap no longer sees); `trap=`/`fast=` split them.
                    let (trap_rung, trap_reached) = me.take_ledger(t.idx);
                    let (f_rung, f_fwd) = fast.map_or((0, 0), |l| (l.doorbells.saturating_sub(l.absorbed), l.forwarded));
                    let (rung, reached) = (trap_rung.saturating_add(f_rung), trap_reached.saturating_add(f_fwd));
                    eprintln!(
                        "kf3: DOORBELL-LEDGER tok={:#010x} route=passthrough rung={rung} emulated={} forwarded={reached} host={:#x} trap={trap_rung}{}",
                        t.idx,
                        rung.saturating_sub(reached.min(rung)),
                        t.chan.token,
                        Self::fast_fields(fast)
                    );
                    crate::dispsw::released_with_channel(r.is_ok(), t.disp_sw.len(), &me.dispsw);
                    let disp_sw = if t.disp_sw.is_empty() { String::new() } else { format!(" disp_sw={}", t.disp_sw.len()) };
                    line.push(format!("passthrough {c:#x}:{h:#x} token {:#x} host {:#x} objects={}{disp_sw} ctx={:?} {}", t.idx, t.chan.token, t.objects.len(), t.ctx, if r.is_ok() { "freed" } else { "FREE REFUSED" }));
                }
                if let Some((h, disp_sw)) = obj {
                    if disp_sw {
                        line.push(format!("display-SW object host {h:#x} {}", crate::dispsw::release_one(me.rm, h, &me.dispsw)));
                    } else {
                        let r = me.rm.free(h);
                        line.push(format!("engine object host {h:#x} {}", if r.is_ok() { "freed" } else { "FREE REFUSED" }));
                    }
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
                let held = me
                    .enc_sessions
                    .lock()
                    .map(|m| m.get(&client).copied().unwrap_or(0))
                    .unwrap_or(0);
                if !acquire && held == 0 {
                    return Ok(format!(
                        "{client:#x} NVENC session release with none held — no host call"
                    ));
                }
                let cmd = if acquire {
                    kf_abi::gssreplay::GSS_ENC_SESSION_ACQUIRE
                } else {
                    kf_abi::gssreplay::GSS_ENC_SESSION_RELEASE
                };
                let mut p = [0u8; kf_abi::gssreplay::ENC_SESSION_PARAMS_SIZE];
                me.rm
                    .raw_control(me.rm.subdevice(), cmd, &mut p)
                    .map_err(|e| {
                        let st = match e {
                            kf_host::RmError::Other(s) if s < 0x4B00 => s,
                            _ => NV_ERR_INVALID_STATE,
                        };
                        (
                            st,
                            format!(
                                "{client:#x} host NVENC session {}: {e:?}",
                                if acquire { "acquire" } else { "release" }
                            ),
                        )
                    })?;
                let now = me.enc_sessions.lock().map(|mut m| {
                    let c = m.entry(client).or_insert(0);
                    if acquire {
                        *c = c.saturating_add(1)
                    } else {
                        *c = c.saturating_sub(1)
                    }
                    *c
                });
                Ok(format!(
                    "{client:#x} NVENC session {} on the host (held now {now:?})",
                    if acquire { "ACQUIRED" } else { "released" }
                ))
            }),
        )
    }

    /// ★ v3-video: release every host NVENC slot a freed guest client still held.
    fn release_encoder_sessions(&self, client: u32) -> Vec<String> {
        let n = self
            .enc_sessions
            .lock()
            .ok()
            .and_then(|mut m| m.remove(&client))
            .unwrap_or(0);
        (0..n)
            .map(|_| {
                let mut p = [0u8; kf_abi::gssreplay::ENC_SESSION_PARAMS_SIZE];
                let r = self.rm.raw_control(
                    self.rm.subdevice(),
                    kf_abi::gssreplay::GSS_ENC_SESSION_RELEASE,
                    &mut p,
                );
                format!(
                    "NVENC session of freed client {client:#x} released on the host ({})",
                    if r.is_ok() { "ok" } else { "REFUSED" }
                )
            })
            .collect()
    }

    /// ★ v3-video: free a Passthrough twin — a member of a shared host group frees its channel,
    /// and the group goes with its LAST member; an ungrouped twin frees channel and group.
    fn release_twin(
        &self,
        client: u32,
        guest_tsg: Option<u32>,
        ctx_share: u32,
        chan: kf_host::Channel,
    ) -> Result<(), kf_host::RmError> {
        let Some(k) = guest_tsg.map(|t| (client, t, ctx_share)) else {
            let r = self.rm.free_channel(chan);
            self.drop_relay(chan.token);
            return r;
        };
        let r = self.rm.free_member(chan);
        self.drop_relay(chan.token);
        let last = self
            .groups
            .lock()
            .map_or(true, |mut m| match m.get_mut(&k) {
                Some(g) if g.1 > 1 => {
                    g.1 = g.1.saturating_sub(1);
                    false
                }
                _ => {
                    m.remove(&k);
                    true
                }
            });
        if last {
            r.and(self.rm.free(chan.tsg))
        } else {
            r
        }
    }

    /// ★ 2026-10-08 (`V3_USERD_RELAY.md` §2): kayfabe's own USERD for a relayed twin — a 4 KiB
    /// device-local object (in no GPU VA space) and one CPU view of it. Everything made is given back on
    /// failure.
    fn relay_userd(&self) -> Result<(u32, kf_linux_raw::CharDevice, u64, VolatileRegion), String> {
        let mem = self
            .rm
            .alloc_device_local(0x1000)
            .map_err(|e| format!("USERD object: {e:?}"))?;
        let (node, cookie) =
            match self
                .rm
                .arm_cpu_view(MapNode::Gpu, mem, 0, 0x1000, ViewAccess::ReadWrite)
            {
                Ok(v) => v,
                Err(e) => {
                    let _ = self.rm.free(mem);
                    return Err(format!("USERD view: {e:?}"));
                }
            };
        match VolatileRegion::map(
            Backing::DeviceFile { fd: node.as_fd() },
            0x1000,
            CachePolicy::Uncached,
            HostPageSize::query(),
        ) {
            Ok(region) => Ok((mem, node, cookie, region)),
            Err(e) => {
                let _ = self.rm.release_cpu_view(kf_host::CpuViewRelease {
                    h_memory: mem,
                    p_linear_address: cookie,
                });
                let _ = self.rm.free(mem);
                Err(format!("USERD mmap: {e:?}"))
            }
        }
    }

    /// ★ 2026-10-08: remove the relay of host token `token` (if any) and give back its USERD — after
    /// the host channel is gone; a worker step holds the relay's lock, so none is mid-store.
    fn drop_relay(&self, token: u32) {
        let Some(r) = self.relays.lock().ok().and_then(|mut m| m.remove(&token)) else {
            return;
        };
        let Ok(mut g) = r.lock() else { return };
        let (mem, cookie) = (g.mem, g.cookie);
        // ⚠ DIAGNOSTIC (after run76; only when the peek ran): did the engine write the last fence
        // release the guest asked of this twin? Read 4 bytes at its VA through the mirror's rows.
        if let Some((kind, fva, payload, gp)) = g.peek.fence {
            let g = &mut *g;
            let mirror = g.mirror.clone();
            let store_len = store_read_bound(self.layout.carve());
            let mut m = Mem {
                mirror: &mirror,
                ram: self.ram,
                rm: self.rm,
                store: self.store,
                store_len,
                views: &mut g.views,
                inbox: &self.inbox,
            };
            let mut b = [0u8; 4];
            let now = m.read(fva, &mut b).map(|()| u32::from_le_bytes(b));
            eprintln!(
                "kf3: chan token {:#x} (host {token:#x}) FENCE-AT-RELEASE (diagnostic) last {kind} release GP[{gp:#x}] va={fva:#x} payload={payload:#x} memory={now:x?} — {}",
                g.idx,
                match now {
                    Ok(v) if v == payload => "the engine wrote it",
                    Ok(_) =>
                        "NOT the payload (the engine did not write it, or a later one overwrote it)",
                    Err(_) => "unreadable",
                }
            );
        }
        // Diagnostic: the guest slot's cursors at release — a GP_PUT ahead of the relay's means the
        // guest queued work it never rang for.
        eprintln!(
            "kf3: chan token {:#x} (host {token:#x}) USERD relay released: forwarded={} refused={} gets={} last_put={} guest[GP_PUT={:x?} GP_GET={:x?}] host[GP_PUT={:x?} GP_GET={:x?}]",
            g.idx,
            g.st.forwarded,
            g.st.refused,
            g.st.gets,
            g.st.host_put,
            g.guest.load(kf_abi::submit::USERD_GP_PUT).ok(),
            g.guest.load(kf_abi::submit::USERD_GP_GET).ok(),
            g.host
                .load_u32(HostOffset::new(kf_abi::submit::USERD_GP_PUT))
                .ok(),
            g.host
                .load_u32(HostOffset::new(kf_abi::submit::USERD_GP_GET))
                .ok()
        );
        drop(g);
        drop(r);
        let _ = self.rm.release_cpu_view(kf_host::CpuViewRelease {
            h_memory: mem,
            p_linear_address: cookie,
        });
        let _ = self.rm.free(mem);
    }

    /// ★ 2026-10-08 (`V3_USERD_RELAY.md` §2.1-2.2): one relay step for host token `ht`, on a worker.
    /// `None` when `ht` is not a relayed twin.
    fn relay_serve(&self, ht: u32) -> Option<bool> {
        let r = self.relays.lock().ok()?.get(&ht).cloned()?;
        // ⊘ A BLOCKING lock (2026-10-08, run77 at 492fb3f0): the token's BUSY state keeps two steps
        // apart, but the GP_GET refresh (`relay_refresh_all`, on a host wake or the park tick) also
        // takes this lock for two 4-byte accesses — a `try_lock` here then CONSUMED the doorbell
        // (run77: the compositor's guest GP_PUT 0x12 never forwarded, host stuck at 0xf, TDR). The
        // refresh never waits (it skips a held relay); a step waits at most for its two accesses —
        // on a worker, never a vCPU, never under a lock a vCPU takes.
        let Ok(mut g) = r.lock() else {
            self.poisoned.fetch_add(1, Ordering::Relaxed);
            return Some(false);
        };
        let g = &mut *g;
        let mut io = RelayMem {
            guest: &g.guest,
            host: &g.host,
            rm: self.rm,
            token: g.chan.token,
        };
        match kf_chan::userd_relay::step(&mut g.st, &mut io) {
            Ok(kf_chan::userd_relay::Outcome::Forwarded(p)) => {
                self.relay_peek(g, p);
                if g.st.forwarded <= 8 || g.st.forwarded.is_power_of_two() {
                    eprintln!(
                        "kf3: chan token {:#x} (host {ht:#x}) USERD relay: GP_PUT {p:#x} forwarded and rung (#{}) — no GP entry or push-buffer word read",
                        g.idx, g.st.forwarded
                    );
                }
                Some(true)
            }
            Ok(kf_chan::userd_relay::Outcome::Unchanged) => Some(false),
            Ok(kf_chan::userd_relay::Outcome::Refused(p)) => {
                if g.st.refused <= 4 {
                    eprintln!(
                        "kf3: chan token {:#x} (host {ht:#x}) USERD relay REFUSED a guest GP_PUT {p:#x} outside its {}-entry ring (not stored, not rung; #{})",
                        g.idx, g.st.entries, g.st.refused
                    );
                }
                Some(false)
            }
            Err(e) => {
                eprintln!(
                    "kf3: chan token {:#x} (host {ht:#x}) USERD relay step failed: {e}",
                    g.idx
                );
                Some(false)
            }
        }
    }

    /// ⚠ DIAGNOSTIC (2026-10-08, after run84): for each named twin that is relayed, its cursors as
    /// ` [host H: guest PUT/GET, relay host_put, engine GET]` — four-byte loads of the guest's slot and
    /// the twin's USERD, nothing stored or rung. A relay a doorbell step holds is reported `busy`.
    fn relay_snapshots(&self, pt: &[((u32, u32), kf_host::Channel)]) -> String {
        let Ok(m) = self.relays.lock() else {
            return " [relays poisoned]".into();
        };
        let mut out = String::new();
        for (_, c) in pt {
            let Some(r) = m.get(&c.token) else { continue };
            let Ok(g) = r.try_lock() else {
                out.push_str(&format!(" [host {:#x}: busy]", c.token));
                continue;
            };
            let mut io = RelayMem {
                guest: &g.guest,
                host: &g.host,
                rm: self.rm,
                token: g.chan.token,
            };
            use kf_chan::userd_relay::RelayIo;
            let gput = io.guest_put();
            let hget = io.host_get();
            let gget = g.guest.load(kf_abi::submit::USERD_GP_GET);
            out.push_str(&format!(
                " [host {:#x}: guest PUT={:x?} GET={:x?} relay host_put={:#x} engine GET={:x?}]",
                c.token, gput, gget, g.st.host_put, hget
            ));
        }
        out
    }

    /// ★ 2026-10-08 (`KF3_RELAY_GET_REFRESH=1`, default off; [`kf_chan::userd_relay::refresh`]): on a
    /// worker, write every relayed twin's engine-written `GP_GET` into its guest slot when it moved —
    /// called on an engine's host non-stall wake (before the guest's interrupt is raised) and on the
    /// worker's park tick. A relay whose lock a doorbell step holds is skipped (that step writes it
    /// back). Returns `(relays seen, stores made)`; nothing at all with the switch off.
    pub fn relay_refresh_all(&self, why: &str) -> (u32, u32) {
        if !relay_get_refresh() {
            return (0, 0);
        }
        let rs: Vec<_> = match self.relays.lock() {
            Ok(m) if !m.is_empty() => m.iter().map(|(k, v)| (*k, v.clone())).collect(),
            _ => return (0, 0),
        };
        let (mut seen, mut stored) = (0u32, 0u32);
        for (ht, r) in rs {
            let Ok(mut g) = r.try_lock() else { continue };
            seen = seen.saturating_add(1);
            let g = &mut *g;
            let mut io = RelayMem {
                guest: &g.guest,
                host: &g.host,
                rm: self.rm,
                token: g.chan.token,
            };
            match kf_chan::userd_relay::refresh(&mut g.st, &mut io) {
                Ok(kf_chan::userd_relay::Refresh::Stored(v)) => {
                    stored = stored.saturating_add(1);
                    let n = g.st.refreshed;
                    if n <= 8 || n.is_power_of_two() {
                        eprintln!(
                            "kf3: chan token {:#x} (host {ht:#x}) USERD relay: GP_GET {v:#x} refreshed on {why} (#{n}; engine-written, outside a doorbell)",
                            g.idx
                        );
                    }
                }
                Ok(kf_chan::userd_relay::Refresh::Same) => {}
                Ok(kf_chan::userd_relay::Refresh::Refused(v)) => {
                    if g.st.get_refused <= 4 {
                        eprintln!(
                            "kf3: chan token {:#x} (host {ht:#x}) USERD relay: engine GP_GET {v:#x} outside its {}-entry ring — NOT stored (#{})",
                            g.idx, g.st.entries, g.st.get_refused
                        );
                    }
                }
                Err(e) => eprintln!(
                    "kf3: chan token {:#x} (host {ht:#x}) USERD relay refresh failed: {e}",
                    g.idx
                ),
            }
        }
        (seen, stored)
    }

    /// ⚠⚠ DIAGNOSTIC (2026-10-08, `KF3_RELAY_PB_PEEK=1`, default off — the one place a relayed
    /// twin's guest bytes are READ, never executed or forwarded by kayfabe): log the GP entries a relayed
    /// twin was rung for (v2, run73: EVERY entry of each batch up to [`PEEK_ENTRY_BUDGET`], runs of zero
    /// entries coalesced, each segment scanned for software-subchannel methods ([`scan_sw_methods`]),
    /// and the previous batch re-read for late writes), so a host RC of that twin
    /// (`[measured, run71 at 2959ed5f]` Xid 32, PBDMA `DEVICE` interrupt) can be traced to a method, and
    /// so the owner's physical-operand question has a sample from a per-process channel. Bounded per
    /// twin; reads go through the mirror's own rows (an unmapped VA is reported, never followed).
    fn relay_peek(&self, g: &mut Relay, new_put: u32) {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if !*ON.get_or_init(|| std::env::var("KF3_RELAY_PB_PEEK").as_deref() == Ok("1")) {
            return;
        }
        let entries = g.st.entries.max(1);
        let mirror = g.mirror.clone();
        let store_len = store_read_bound(self.layout.carve());
        let mut mem = Mem {
            mirror: &mirror,
            ram: self.ram,
            rm: self.rm,
            store: self.store,
            store_len,
            views: &mut g.views,
            inbox: &self.inbox,
        };
        let tok = g.idx;
        // ★ run73 (v2): first re-read the previous batch — an entry that changed was written by the
        // guest AFTER it rang for it (then the engine may have fetched the old bytes).
        for (i, old) in std::mem::take(&mut g.peek.prev) {
            let mut e = [0u8; 8];
            let gpva = g.gpfifo_va.saturating_add(u64::from(i).saturating_mul(8));
            if mem.read(gpva, &mut e).is_ok() && e != old {
                eprintln!(
                    "kf3: chan token {tok:#x} RELAY-PEEK (diagnostic) GP[{i:#x}] CHANGED after its doorbell: was {:016x} now {:016x}",
                    u64::from_le_bytes(old),
                    u64::from_le_bytes(e)
                );
            }
        }
        // Every entry in [next, new_put), bounded per twin; zero entries are counted, not listed.
        let mut idx = g.peek.next.checked_rem(entries).unwrap_or(0);
        let mut zeros: Option<(u32, u32)> = None;
        let flush_zeros = |z: &mut Option<(u32, u32)>| {
            if let Some((a, n)) = z.take() {
                eprintln!(
                    "kf3: chan token {tok:#x} RELAY-PEEK (diagnostic) GP[{a:#x}..+{n:#x}] all-zero entries (NOP control entries)"
                );
            }
        };
        while g.peek.entries < PEEK_ENTRY_BUDGET && idx != new_put.checked_rem(entries).unwrap_or(0)
        {
            let mut e = [0u8; 8];
            let gpva = g.gpfifo_va.saturating_add(u64::from(idx).saturating_mul(8));
            g.peek.entries = g.peek.entries.saturating_add(1);
            match mem.read(gpva, &mut e) {
                Err(why) => {
                    flush_zeros(&mut zeros);
                    eprintln!(
                        "kf3: chan token {tok:#x} RELAY-PEEK (diagnostic) GP[{idx:#x}] @{gpva:#x}: unreadable ({why})"
                    );
                }
                Ok(()) if e == [0u8; 8] => {
                    g.peek.prev.push((idx, e));
                    zeros = match zeros {
                        Some((a, n)) => Some((a, n.saturating_add(1))),
                        None => Some((idx, 1)),
                    };
                }
                Ok(()) => {
                    flush_zeros(&mut zeros);
                    g.peek.prev.push((idx, e));
                    let lo = u32::from_le_bytes([e[0], e[1], e[2], e[3]]);
                    let hi = u32::from_le_bytes([e[4], e[5], e[6], e[7]]);
                    let va = u64::from(lo & !3) | (u64::from(hi & 0xff) << 32);
                    let words = (hi >> 10) & 0x1f_ffff;
                    if words == 0 {
                        eprintln!(
                            "kf3: chan token {tok:#x} RELAY-PEEK (diagnostic) GP[{idx:#x}] lo={lo:#010x} hi={hi:#010x} CONTROL entry opcode={:#x} sync={}",
                            lo & 0xff,
                            hi >> 31
                        );
                    } else {
                        let n = (words as usize).min(PEEK_SEGMENT_WORDS);
                        let mut buf = vec![0u8; n.saturating_mul(4)];
                        match mem.read(va, &mut buf) {
                            Ok(()) => {
                                let w: Vec<u32> = buf
                                    .as_chunks::<4>()
                                    .0
                                    .iter()
                                    .map(|c| u32::from_le_bytes(*c))
                                    .collect();
                                let scan = scan_sw_methods(&w);
                                if let Some((k, fva, p)) = last_fence_release(&w) {
                                    g.peek.fence = Some((k, fva, p, idx));
                                }
                                let body = w
                                    .iter()
                                    .take(64)
                                    .map(|x| format!("{x:08x}"))
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                eprintln!(
                                    "kf3: chan token {tok:#x} RELAY-PEEK (diagnostic) GP[{idx:#x}] lo={lo:#010x} hi={hi:#010x} va={va:#x} words={words} level={} sync={} subch_headers={:?} sw_methods={} undecodable_at={:?}: {body}",
                                    (hi >> 9) & 1,
                                    hi >> 31,
                                    scan.per_subch,
                                    scan.hits.len(),
                                    scan.undecodable_at
                                );
                                for h in scan.hits {
                                    if g.peek.sw_logged >= PEEK_SW_BUDGET {
                                        break;
                                    }
                                    g.peek.sw_logged = g.peek.sw_logged.saturating_add(1);
                                    eprintln!(
                                        "kf3: chan token {tok:#x} RELAY-PEEK (diagnostic) SW-SUBCH METHOD GP[{idx:#x}] word {} subch {} method {:#x} data {:x?} — a software method (PBDMA DEVICE on the twin)",
                                        h.word, h.subch, h.method, h.data
                                    );
                                }
                            }
                            Err(why) => eprintln!(
                                "kf3: chan token {tok:#x} RELAY-PEEK (diagnostic) GP[{idx:#x}] lo={lo:#010x} hi={hi:#010x} va={va:#x} words={words}: segment unreadable ({why})"
                            ),
                        }
                    }
                }
            }
            idx = idx.saturating_add(1).checked_rem(entries).unwrap_or(0);
        }
        flush_zeros(&mut zeros);
        g.peek.next = idx;
    }

    /// ⚠ DIAGNOSTIC (default off, 2026-10-09, `KF3_PT_STALL_SNAPSHOT=1`; record
    /// `traces/windows_reset_20261009/`): **the stall snapshot**. Called by the drainer (never a
    /// vCPU) about every 250 ms. When no doorbell has reached any live Passthrough twin for
    /// `KF3_PT_STALL_SNAPSHOT_MS` (default 1000, 200..=10000) after at least one did, it reads, once
    /// per silence and at most [`SNAP_MAX`] times per boot, every live twin's state from GUEST
    /// memory ([`Self::pt_snapshot_twin`]). Post-mortem only: it logs; nothing it reads reaches a
    /// host action. With the switch off it returns at its first load.
    pub fn pt_stall_snapshot_poll(&self) {
        let Some(silence) = pt_stall_snapshot_ms() else {
            return;
        };
        static STATE: Mutex<(u64, Option<std::time::Instant>, bool, u32)> =
            Mutex::new((0, None, false, 0));
        let tokens: Vec<u32> = match self.pt.lock() {
            Ok(m) => m.values().map(|t| t.idx).collect(),
            Err(_) => return,
        };
        let total: u64 = tokens
            .iter()
            .filter_map(|&i| self.rung.get(i as usize))
            .map(|c| c.load(Ordering::Relaxed))
            .sum();
        let Ok(mut st) = STATE.lock() else { return };
        let (last, since, armed, taken) = &mut *st;
        let now = std::time::Instant::now();
        if total != *last || since.is_none() {
            *armed = *armed || (total > *last && since.is_some());
            *last = total;
            *since = Some(now);
            return;
        }
        let quiet = since.map_or(0, |t| now.duration_since(t).as_millis());
        if !*armed || *taken >= SNAP_MAX || quiet < u128::from(silence) {
            return;
        }
        *armed = false;
        *taken = taken.saturating_add(1);
        let n = *taken;
        drop(st);
        self.pt_snapshot_all(&format!(
            "stall #{n}: no doorbell to any of {} live Passthrough twin(s) for {quiet} ms (doorbells so far {total})",
            tokens.len()
        ));
    }

    /// ⚠ DIAGNOSTIC: [`Self::pt_snapshot_twin`] for every live twin, under one `pt` lock.
    fn pt_snapshot_all(&self, why: &str) {
        let Ok(m) = self.pt.lock() else { return };
        eprintln!(
            "kf3: PT-SNAP BEGIN {why} (maplog t={:.6})",
            kf_mem::maplog::t()
        );
        let mut twins: Vec<_> = m.iter().collect();
        twins.sort_by_key(|(_, t)| t.idx);
        for ((c, h), t) in twins {
            self.pt_snapshot_twin(*c, *h, t);
        }
        eprintln!("kf3: PT-SNAP END");
    }

    /// ⚠ DIAGNOSTIC: one twin's state, read from guest memory — its USERD (`NVC56F` USERD layout,
    /// `ogkm-595.84: clc56f.h:49-63`), the GPFIFO entries around `GPGet`/`GPPut`, the push-buffer
    /// methods of the last [`SNAP_SEGMENTS`] entries before `GPPut` (decoded by
    /// [`snap_decode`]), and the current value at every semaphore those methods name. Bounded:
    /// [`SNAP_ENTRIES`] entries, [`SNAP_WORDS`] words per segment, [`SNAP_SEMS`] semaphores. Every
    /// read goes through the guest's declared USERD GPA (checked against guest RAM) or OUR placement
    /// rows (an unplaced or vidmem VA is named, never followed).
    fn pt_snapshot_twin(&self, c: u32, h: u32, t: &PtChan) {
        let tok = t.idx;
        let (gpfifo_va, entries, userd_gpa) = t.ring_at;
        let db = self
            .rung
            .get(tok as usize)
            .map_or(0, |x| x.load(Ordering::Relaxed));
        let db_at = self
            .rung_at_us
            .get(tok as usize)
            .map_or(0, |x| x.load(Ordering::Relaxed));
        let mut u = [0u8; 0x90];
        let userd = userd_gpa
            .ok_or_else(|| "USERD not in guest RAM".to_string())
            .and_then(|gpa| {
                let off = self
                    .ram
                    .dma_to_file_range(gpa, u.len() as u64)
                    .ok_or_else(|| format!("USERD GPA {gpa:#x} outside guest RAM"))?;
                let (mem, at) = self
                    .ram
                    .at_file_offset(off, u.len() as u64)
                    .ok_or_else(|| format!("USERD offset {off:#x} unregistered"))?;
                if mem.read_into(at, &mut u) {
                    Ok(())
                } else {
                    Err("USERD read".to_string())
                }
            });
        let w = |o: usize| {
            u.get(o..o.saturating_add(4))
                .and_then(|b| <[u8; 4]>::try_from(b).ok())
                .map_or(0, u32::from_le_bytes)
        };
        let (gp_get, gp_put) = (w(0x88), w(0x8c));
        match &userd {
            Ok(()) => eprintln!(
                "kf3: PT-SNAP tok={tok:#x} chan {c:#x}:{h:#x} host={:#x} engine={:#x} user_work={} doorbells={db} last_doorbell_us={db_at} USERD@{:#x}: Put={:#x} Get={:#x} Reference={:#x} PutHi={:#x} TopLevelGet={:#x} TopLevelGetHi={:#x} GetHi={:#x} GPGet={gp_get:#x} GPPut={gp_put:#x} ring={gpfifo_va:#x}x{entries} — {}",
                t.chan.token,
                t.engine,
                t.user_work,
                userd_gpa.unwrap_or(0),
                w(0x40),
                w(0x44),
                w(0x48),
                w(0x4c),
                w(0x58),
                w(0x5c),
                w(0x60),
                if gp_get == gp_put {
                    "GPGet == GPPut: the engine fetched every entry the guest put"
                } else {
                    "GPGet != GPPut: entries put and NOT fetched"
                }
            ),
            Err(e) => eprintln!(
                "kf3: PT-SNAP tok={tok:#x} chan {c:#x}:{h:#x} host={:#x} engine={:#x} doorbells={db} last_doorbell_us={db_at} USERD unreadable ({e}) ring={gpfifo_va:#x}x{entries}",
                t.chan.token, t.engine
            ),
        }
        if entries == 0 || userd.is_err() {
            return;
        }
        let n = entries.max(1);
        // entries [GPGet - 2, GPPut + 2), at most SNAP_ENTRIES; the segments of the last
        // SNAP_SEGMENTS before GPPut are decoded
        let first = ring_dist(gp_get, 2.min(n.saturating_sub(1)), n);
        let span = ring_dist(gp_put, first, n)
            .saturating_add(2)
            .min(SNAP_ENTRIES);
        let mut sems: Vec<SemRef> = Vec::new();
        for k in 0..span {
            let i = u32::try_from(
                u64::from(first)
                    .saturating_add(u64::from(k))
                    .checked_rem(u64::from(n))
                    .unwrap_or(0),
            )
            .unwrap_or(0);
            let mut e = [0u8; 8];
            let gpva = gpfifo_va.saturating_add(u64::from(i).saturating_mul(8));
            if let Err(why) = self.snap_read(&t.rows, gpva, &mut e) {
                eprintln!("kf3: PT-SNAP tok={tok:#x} GP[{i:#x}] @{gpva:#x}: {why}");
                continue;
            }
            let lo = u32::from_le_bytes([e[0], e[1], e[2], e[3]]);
            let hi = u32::from_le_bytes([e[4], e[5], e[6], e[7]]);
            let va = u64::from(lo & !3) | (u64::from(hi & 0xff) << 32);
            let words = (hi >> 10) & 0x1f_ffff;
            let pending = ring_dist(i, gp_get, n) < ring_dist(gp_put, gp_get, n);
            let to_put = ring_dist(gp_put, i, n);
            let decode = to_put <= SNAP_SEGMENTS && to_put > 0;
            let mut line = format!(
                "kf3: PT-SNAP tok={tok:#x} GP[{i:#x}] {} lo={lo:#010x} hi={hi:#010x} va={va:#x} words={words}",
                if pending { "PENDING" } else { "fetched" }
            );
            if decode && words > 0 {
                let mut buf = vec![0u8; (words as usize).min(SNAP_WORDS).saturating_mul(4)];
                match self.snap_read(&t.rows, va, &mut buf) {
                    Ok(()) => {
                        let ws: Vec<u32> = buf
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(|c| u32::from_le_bytes(*c))
                            .collect();
                        let d = snap_decode(&ws, SNAP_METHODS);
                        line.push_str(&format!(
                            " methods[{}{}]: {}",
                            d.methods.len(),
                            d.stopped_at
                                .map_or(String::new(), |w| format!(", undecodable at word {w}")),
                            d.methods
                                .iter()
                                .map(|(s, m, v)| format!("s{s}:{m:#x}={v:#x}"))
                                .collect::<Vec<_>>()
                                .join(" ")
                        ));
                        for s in d.sems {
                            if sems.len() < SNAP_SEMS {
                                sems.push(s);
                            }
                        }
                        if let Some((k, fva, p)) = last_fence_release(&ws)
                            && sems.len() < SNAP_SEMS
                        {
                            sems.push(SemRef {
                                op: k,
                                va: fva,
                                payload: u64::from(p),
                                wide: false,
                                gp: i,
                            });
                        }
                        for s in sems.iter_mut().filter(|s| s.gp == u32::MAX) {
                            s.gp = i;
                        }
                    }
                    Err(why) => line.push_str(&format!(" segment unreadable ({why})")),
                }
            }
            eprintln!("{line}");
        }
        for s in &sems {
            let mut b = [0u8; 8];
            let now = self
                .snap_read(&t.rows, s.va, b.split_at_mut(if s.wide { 8 } else { 4 }).0)
                .map(|()| u64::from_le_bytes(b));
            eprintln!(
                "kf3: PT-SNAP tok={tok:#x} SEM {} (GP[{:#x}]) va={:#x} payload={:#x} memory={} — {}",
                s.op,
                s.gp,
                s.va,
                s.payload,
                now.as_ref()
                    .map_or_else(|e| format!("unreadable ({e})"), |v| format!("{v:#x}")),
                match (&now, s.op.starts_with("ACQ")) {
                    (Ok(v), true) if *v >= s.payload =>
                        "an acquire the memory already satisfies (>=)",
                    (Ok(_), true) =>
                        "an ACQUIRE NOT YET SATISFIED (memory < payload): who releases it?",
                    (Ok(v), false) if *v == s.payload => "a release the memory holds",
                    (Ok(_), false) =>
                        "a release NOT in memory (not executed, or overwritten later)",
                    (Err(_), _) => "",
                }
            );
        }
    }

    /// ⚠ DIAGNOSTIC: read guest memory at a VA through OUR placement rows, guest-RAM rows only (a
    /// vidmem row is named, not read: the snapshot arms no CPU view of the store).
    fn snap_read(
        &self,
        rows: &crate::mem::PlacedRows,
        va: u64,
        out: &mut [u8],
    ) -> Result<(), String> {
        let len = out.len() as u64;
        let mut done = 0u64;
        while done < len {
            let at_va = va.saturating_add(done);
            let (ram, off, avail) = crate::mem::resolve_placed_prefix(rows, at_va)
                .ok_or_else(|| format!("{at_va:#x} not placed by us"))?;
            if !ram {
                return Err(format!(
                    "{at_va:#x} is a vidmem row (not read by the snapshot)"
                ));
            }
            let n = avail.min(len.saturating_sub(done));
            if n == 0 {
                return Err(format!("{at_va:#x}: no progress"));
            }
            let (mem, at) = self
                .ram
                .at_file_offset(off, n)
                .ok_or_else(|| format!("{at_va:#x}: guest-RAM offset {off:#x} unregistered"))?;
            if !mem.read_into(at, out_window(out, done, n)?) {
                return Err(format!("{at_va:#x}: guest-RAM read"));
            }
            done = done.saturating_add(n);
        }
        Ok(())
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
            eprintln!(
                "kf3: chan {:#x}:{:#x} birth REFUSED: {why} (decl {a:x?})",
                a.client, a.handle
            );
            ChanAnswer::Refused { status, why }
        };
        // ★★ 2026-10-08 (OWNER_RULINGS §V, `KF3_WIN_USER_CHANNELS_PASSTHROUGH`): a Windows
        // guest-kernel channel the link classified as per-process USER work at its alloc
        // (`kf_rm::chanlink::windows_user_work`) takes the Passthrough route; every kernel-only
        // route below is for the others.
        let passthrough = !a.kernel_client || a.user_work;
        let kernel_work = a.kernel_client && !a.user_work;
        // ★ 2026-10-10 (§AB): `KF3_TSPACE` is hardwired, so no route below is gated on it.
        // `KF3_KERNEL_NVDEC_CTX`/`_NVENC_CTX`/`_OFA_CTX` are deleted and hardwired ON (the
        // Windows profile's values; measured required, runs 21-24); `KF3_KERNEL_GR_CE` stays a
        // switch ([`kernel_gr_ce`] says why).
        let kernel_gr =
            kernel_work && engine == kf_abi::submit::ENGINE_TYPE_GRAPHICS && kernel_gr_ce();
        let kernel_nvdec =
            kernel_work && kf_abi::submit::nvdec_index_of_engine_type(engine).is_some();
        let kernel_nvenc =
            kernel_work && kf_abi::submit::nvenc_index_of_engine_type(engine).is_some();
        let kernel_ofa = kernel_work && kf_abi::submit::ofa_index_of_engine_type(engine).is_some();
        if kernel_work
            && !is_copy_engine(engine)
            && !kernel_gr
            && !kernel_nvdec
            && !kernel_nvenc
            && !kernel_ofa
        {
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
                format!(
                    "user channel on engine type {engine:#x}: only a copy engine, GR0 or a video engine has a passthrough twin"
                ),
            );
        }
        // ★ P1+P2 inc A / S1-43: a guest-named FB USERD or error notifier inside the usable heap,
        // checked here — before any host call — for both routes. ★ Review fix 2026-10-04: REFUSED
        // when strict ([`crate::tspace::INCA_STRICT`], hardwired ON 2026-10-10 with the T-space);
        // the count-only arm is reached only by its unit test.
        match heap_gate(
            crate::tspace::INCA_STRICT,
            negctl_heap(),
            &self.layout,
            a.userd,
            a.error_notifier,
        ) {
            HeapGate::Inside => {}
            HeapGate::Refuse(why) => {
                self.heap_out.fetch_add(1, Ordering::Relaxed);
                return refuse(NV_ERR_INVALID_ARGUMENT, why);
            }
            HeapGate::Count(why) => {
                self.heap_out.fetch_add(1, Ordering::Relaxed);
                eprintln!(
                    "kf3: chan {:#x}:{:#x} HEAP-OUT (count-only, inc A): {why}",
                    a.client, a.handle
                );
            }
        }
        let Some(vas) = a.vaspace else {
            return refuse(
                NV_ERR_INVALID_STATE,
                format!(
                    "no VA space resolved (hVASpace={:#x}, parent {:#x})",
                    a.h_vaspace, a.parent
                ),
            );
        };
        // ★ v3-gfx: the VA space's OWN client (a dup'd space is keyed by its original).
        let key = VasKey((u64::from(a.vaspace_client) << 32) | u64::from(vas));
        let Some(mirror) = self.mirrors.lock().ok().and_then(|m| m.get(&key).cloned()) else {
            return refuse(
                NV_ERR_INVALID_STATE,
                format!("VA space {key:?} has no mirror (no page-directory statement named it)"),
            );
        };
        let Some(chid) = a.chid else {
            return refuse(
                NV_ERR_INVALID_STATE,
                format!(
                    "no guest chid in flags {:#x} (USERD_INDEX not fixed)",
                    a.flags
                ),
            );
        };
        // ★ 2026-09-26: the index is per family (`kf_trap::tokenindex`) — on Blackwell chids are per
        // runlist, and the runlist is the one the served FIFO table gives this engine (what the
        // guest's own token carries in RUNLIST_ID). Through Hopper the runlist is ignored.
        let runlist =
            kf_rm::authored::runlist_of_engine_type(&self.engine_table, engine).unwrap_or(0);
        let Some(idx) = self.plane.token_index.of_channel(runlist, chid) else {
            return refuse(
                NV_ERR_INSUFFICIENT_RESOURCES,
                format!("chid {chid:#x} (runlist {runlist}) outside the token table"),
            );
        };
        if (idx as usize) >= self.plane.tokens.len() {
            return refuse(
                NV_ERR_INSUFFICIENT_RESOURCES,
                format!("chid {chid:#x} outside the token table"),
            );
        }
        if passthrough {
            // ★★ 2026-10-08 (`V3_USERD_RELAY.md`): a Windows user-work twin with a guest-RAM USERD is
            // born over kayfabe's own USERD and relayed; the guest's slot is only read (`GP_PUT`) and
            // written (`GP_GET`) by a worker. Its guest-RAM view is made here, before any host verb.
            // ⚠ CONTROL (2026-10-08, owner decision; `KF3_USERD_RELAY_OFF=1`, default off): no relay —
            // the Windows user-work twin ADOPTS the guest's sysmem USERD like any Passthrough twin.
            // Only meaningful with the GPU's IOMMU group in identity mode (host RM refuses a USERD
            // whose DMA address is wider than PTR_HI allows; the birth then fails by name, as run61).
            let relay_guest = if userd_relay_off() {
                None
            } else if a.user_work && matches!(a.userd, Some(kf_arch::UserdMem::Sysmem { .. })) {
                match self.userd_view(a.userd) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        return refuse(
                            NV_ERR_NOT_SUPPORTED,
                            format!("USERD relay: guest slot: {e}"),
                        );
                    }
                }
            } else {
                None
            };
            // ★ The guest's USERD, adopted AT CREATION (RM zeroes it — `rm_takes_a_guest_userd`):
            // a store slice, or guest RAM through the mirror's RAM object.
            let userd = match a.userd {
                Some(kf_arch::UserdMem::Framebuffer { base, .. }) => {
                    kf_chan::passthrough::UserdAt::Store {
                        store: self.store,
                        off: base,
                    }
                }
                Some(kf_arch::UserdMem::Sysmem { base, .. }) => {
                    let (Some(ram), Some((_, off))) = (
                        mirror.ram_obj,
                        self.ram.dma_to_file_range(base, 0x200).map(|o| ((), o)),
                    ) else {
                        return refuse(
                            NV_ERR_NOT_SUPPORTED,
                            format!(
                                "sysmem USERD at {base:#x}: no guest-RAM object or memfd offset"
                            ),
                        );
                    };
                    kf_chan::passthrough::UserdAt::Ram { ram, off }
                }
                other => {
                    return refuse(
                        NV_ERR_NOT_SUPPORTED,
                        format!("USERD not declared as a physical descriptor ({other:?})"),
                    );
                }
            };
            // ★ P5c: where the guest's error notifier record is, as an object + offset the host
            // can name — so the twin's RC record is written there by the HOST (its GSP), natively.
            let err_at: Option<(u32, u64, kf_arch::UserdMem)> = match a.error_notifier {
                Some(kf_arch::fault::ErrorNotifier::Sysmem { gpa }) => {
                    match (
                        mirror.ram_obj,
                        self.ram.dma_to_file_range(gpa, 16).map(|o| ((), o)),
                    ) {
                        (Some(ram), Some((_, off))) => Some((
                            ram,
                            off,
                            kf_arch::UserdMem::Sysmem {
                                base: gpa,
                                size: 16,
                            },
                        )),
                        _ => {
                            self.rc_unarmed.fetch_add(1, Ordering::Relaxed);
                            eprintln!(
                                "kf3: chan {:#x}:{:#x} RC-UNARMED: sysmem notifier @{gpa:#x} has no guest-RAM object/offset",
                                a.client, a.handle
                            );
                            None
                        }
                    }
                }
                Some(kf_arch::fault::ErrorNotifier::Framebuffer { off }) => Some((
                    self.store,
                    off,
                    kf_arch::UserdMem::Framebuffer {
                        base: off,
                        size: 16,
                    },
                )),
                Some(kf_arch::fault::ErrorNotifier::Unreachable) => {
                    self.rc_unarmed.fetch_add(1, Ordering::Relaxed);
                    eprintln!(
                        "kf3: chan {:#x}:{:#x} RC-UNARMED: the declared notifier is in an aperture we cannot name",
                        a.client, a.handle
                    );
                    None
                }
                None => None,
            };
            let g0 = kf_chan::passthrough::GuestChannel {
                gpfifo_va: a.gpfifo_va,
                entries: a.entries.max(1),
                userd,
                engine,
                err_ctx: 0,
            };
            let space = mirror.space;
            let relay_mirror = mirror.clone();
            let rows = mirror.rows.clone();
            let ledger = mirror.ledger.clone();
            // ★ P5c: counted NOW (on the drainer, in statement order), so a VA-space free that
            // follows can never recycle the space under a birth still queued.
            let live = mirror.live.clone();
            // ★★ P1+P2 inc D (§4.2), hardwired 2026-10-10: a passthrough birth moves the twin to
            // User(n+1), in statement order, and is REFUSED by name in a guest-KERNEL space —
            // never waited on.
            let twin = {
                if negctl_twin() || mirror.kernel_vas.try_user().is_err() {
                    self.twin_refused.fetch_add(1, Ordering::Relaxed);
                    return refuse(
                        NV_ERR_INVALID_STATE,
                        format!(
                            "twin state: VA space {key:?} is a guest-KERNEL space — a channel the guest created non-kernel never runs in it (V3_P1P2_TSPACE.md §4.2){}",
                            if negctl_twin() {
                                " [KF3_NEGCTL_TWIN positive control]"
                            } else {
                                ""
                            }
                        ),
                    );
                }
                Some(mirror.kernel_vas.clone())
            };
            live.fetch_add(1, Ordering::AcqRel);
            return self.defer(
                "birth passthrough",
                Box::new(move |me: &ChanPlane| {
                    let notifier = err_at.and_then(|(obj, off, at)| me.arm_notifier(a.client, a.handle, obj, off, at));
                    // ★ 2026-10-08: the relayed twin's own USERD (refused by name, fail closed, if it
                    // cannot be made — never a twin without its relay).
                    let relay_host = match relay_guest.as_ref().map(|_| me.relay_userd()) {
                        Some(Ok(h)) => Some(h),
                        Some(Err(e)) => {
                            live.fetch_sub(1, Ordering::AcqRel);
                            if let Some(t) = &twin {
                                t.user_done();
                            }
                            if let Some(n) = notifier {
                                me.release_notifier(n);
                            }
                            return Err((NV_ERR_INSUFFICIENT_RESOURCES, format!("USERD relay: {e}")));
                        }
                        None => None,
                    };
                    let userd_at = relay_host.as_ref().map_or(g0.userd, |h| kf_chan::passthrough::UserdAt::Store { store: h.0, off: 0 });
                    let g = kf_chan::passthrough::GuestChannel { err_ctx: notifier.as_ref().map_or(0, |n| n.ctx), userd: userd_at, ..g0 };
                    // ★ v3-video: a member of a guest TSG joins the host group standing for it.
                    let gkey = a.tsg.map(|t| (a.client, t, a.ctx_share));
                    let join = gkey.and_then(|k| me.groups.lock().ok().and_then(|m| m.get(&k).map(|g| g.0)));
                    let chan = match kf_chan::passthrough::birth_twin_in(me.rm, space, g, join) {
                        Ok(c) => {
                            if let (Some(k), Ok(mut m)) = (gkey, me.groups.lock()) {
                                m.entry(k).and_modify(|g| g.1 = g.1.saturating_add(1)).or_insert((c.tsg, 1));
                            }
                            c
                        }
                        Err(e) => {
                            live.fetch_sub(1, Ordering::AcqRel);
                            if let Some(t) = &twin {
                                t.user_done();
                            }
                            if let Some(n) = notifier {
                                me.release_notifier(n);
                            }
                            if let Some((mem, _node, cookie, region)) = relay_host {
                                drop(region);
                                let _ = me.rm.release_cpu_view(kf_host::CpuViewRelease { h_memory: mem, p_linear_address: cookie });
                                let _ = me.rm.free(mem);
                            }
                            return Err((NV_ERR_INSUFFICIENT_RESOURCES, e));
                        }
                    };
                    let owner = if a.kernel_client && !a.user_work { Owner::Kernel } else { Owner::User };
                    // ★ 2026-10-09, hardwired 2026-10-10 (`crate::latejoin`, hardware-verified): a twin
                    // born into a guest TSG the guest has ALREADY scheduled is in a host group nobody
                    // else schedules (the guest sent its one schedule before the channel existed).
                    // The schedule of its own host group, authored from the guest's own scheduled
                    // state. A refusal is the birth's refusal, by name.
                    let late = crate::latejoin::schedule_late_joiner(me.rm, &me.guest_sched, a.client, a.tsg, chan);
                    let alloc = late
                        .clone()
                        .map(|_| ())
                        .and_then(|()| me.caps.lock().map_err(|_| "caps poisoned".to_string()))
                        .and_then(|mut c| me.plane.allocate_channel(&mut c, idx, if relay_host.is_some() { Route::Translated } else { Route::Passthrough }, chan.token, owner).map_err(|e| {
                            // ★ 2026-10-11: LOUD and counted by reason (a refused birth is a guest crash: the OpenGL ICD dereferences it).
                            let rl = me.plane.token_index.runlist_of(me.plane.token_index.clamp(idx));
                            let n = me.birth_refused_cap.fetch_add(1, Ordering::Relaxed) + 1;
                            eprintln!("kf3: ⚠ CHANNEL BUDGET EXHAUSTED on guest runlist {rl}: {e:?} (refusal #{n}; the `channel-budget` property, docs/design/V3_CHANNEL_BUDGET.md)");
                            format!("{e:?}")
                        }));
                    if let Err(e) = alloc {
                        let _ = me.release_twin(a.client, a.tsg, a.ctx_share, chan);
                        live.fetch_sub(1, Ordering::AcqRel);
                        if let Some(t) = &twin {
                            t.user_done();
                        }
                        if let Some(n) = notifier {
                            me.release_notifier(n);
                        }
                        if let Some((mem, _node, cookie, region)) = relay_host {
                            drop(region);
                            let _ = me.rm.release_cpu_view(kf_host::CpuViewRelease { h_memory: mem, p_linear_address: cookie });
                            let _ = me.rm.free(mem);
                        }
                        return Err((NV_ERR_INSUFFICIENT_RESOURCES, format!("token {idx:#x}: {e}")));
                    }
                    let rc = if notifier.is_some() { "armed" } else { "none" };
                    if let Ok(mut m) = me.pt.lock() {
                        m.insert((a.client, a.handle), PtChan {
                            chan,
                            idx,
                            tsg: a.tsg,
                            ctx_share: a.ctx_share,
                            parent: a.parent,
                            device: a.device,
                            engine,
                            objects: HashMap::new(),
                            disp_sw: HashMap::new(),
                            sw_ids: crate::dispsw::SwClassIds::default(),
                            dispsw_host_refused: false,
                            defapi_host: None,
                            ctx: CtxBind::default(),
                            user_work: a.user_work,
                            live,
                            twin,
                            notifier,
                            stopped: false,
                            disabled: false,
                            space,
                            rows,
                            ledger,
                            falcon_ctx: None,
                            ring_at: (
                                a.gpfifo_va,
                                g.entries,
                                match a.userd {
                                    Some(kf_arch::UserdMem::Sysmem { base, .. }) => Some(base),
                                    _ => None,
                                },
                            ),
                        });
                    }
                    me.pt_births.fetch_add(1, Ordering::Relaxed);
                    me.engine_live(engine, true);
                    let relayed = match (relay_host, relay_guest) {
                        (Some((mem, node, cookie, host)), Some(guest)) => {
                            if let Ok(mut m) = me.relays.lock() {
                                m.insert(chan.token, Arc::new(Mutex::new(Relay {
                                    st: kf_chan::userd_relay::RelayState::new(g.entries),
                                    guest,
                                    host,
                                    _node: node,
                                    cookie,
                                    mem,
                                    chan,
                                    idx,
                                    mirror: relay_mirror.clone(),
                                    views: StoreViews::new(),
                                    gpfifo_va: a.gpfifo_va,
                                    peek: PeekState::default(),
                                })));
                            }
                            true
                        }
                        _ => false,
                    };
                    let _ = me.take_ledger(idx);
                    // ★ After the token word and the twin: until its placement lands, the trap
                    // serves this token; after, its eventfd does. No lock is held here.
                    let fast = if relayed { "fast=off (USERD relay: doorbells go to a worker)".to_string() } else { me.fast_register(idx, runlist, chid) };
                    Ok(format!(
                        "chan {:#x}:{:#x} BORN Passthrough: token {idx:#x} -> host {:#x} in {key:?} gpfifo={:#x}x{} userd={userd:?} engine={engine:#x} declared_kernel_pid={} {}{} rc={rc} late_joiner_schedule={:?} {fast}",
                        a.client,
                        a.handle,
                        chan.token,
                        a.gpfifo_va,
                        g.entries,
                        a.declared_kernel_pid,
                        kernel_by(&a),
                        if a.user_work { " WINDOWS-USER-WORK (§V, ProcessID/subcontext)" } else { "" },
                        late.as_ref().ok()
                    ))
                }),
            );
        }
        let entries = a.entries.max(1);
        // ★★★ OWNER_RULINGS §AB rule 4 (2026-10-10): the T-space maps all of guest RAM and the
        // guest store, so a Translated channel runs in it only with a [`crate::tspace::Privileged`]
        // witness, made from the alloc's facts. Today's routes send only guest-kernel work here
        // (`passthrough` above takes every other channel), so this refusal cannot fire; it keeps
        // an unprivileged Translated channel out of the T-space if a route ever sends one.
        let Some(privileged) = crate::tspace::Privileged::of(a.kernel_client, a.user_work) else {
            self.tspace_refused.fetch_add(1, Ordering::Relaxed);
            return refuse(
                NV_ERR_INVALID_STATE,
                crate::tspace::UNPRIVILEGED_TRANSLATED.to_string(),
            );
        };
        // ★★★ P1+P2 inc D (`V3_P1P2_TSPACE.md` §2.3, §4.2), hardwired 2026-10-10 (§AB): the
        // channel runs in the T-space — refused by name if it is not built (never a fallback to
        // mirror windows: there are none, S1-21) — and its space moves Unclassified -> Kernel,
        // refused while user channels live there. Statement order, before the channel can run.
        {
            match self.tspace.get() {
                Some(Ok(_)) => {}
                built => {
                    self.tspace_refused.fetch_add(1, Ordering::Relaxed);
                    let why = match built {
                        Some(Err(e)) => e.clone(),
                        _ => "prewarm has not run: guest RAM is not registered yet".to_string(),
                    };
                    return refuse(NV_ERR_INVALID_STATE, format!("tspace: not built ({why})"));
                }
            }
            match mirror.kernel_vas.try_kernel() {
                Ok(true) => eprintln!(
                    "kf3: {key:?} is a guest-KERNEL space (Translated chan {:#x}:{:#x}): privileged leaves are mirrored here",
                    a.client, a.handle
                ),
                Ok(false) => {}
                Err(e) => {
                    // ★ Review fix 2026-10-04: a space whose only user channels are being freed
                    // (or whose host free was refused) is counted apart.
                    if matches!(e, crate::twin::TwinRefusal::KernelWhileUsersFree(_)) {
                        self.twin_freeing_refused.fetch_add(1, Ordering::Relaxed);
                    } else {
                        self.twin_refused.fetch_add(1, Ordering::Relaxed);
                    }
                    return refuse(
                        NV_ERR_INVALID_STATE,
                        format!(
                            "twin state: VA space {key:?} ({e:?}) — a Translated channel never runs in a space a user channel runs in (V3_P1P2_TSPACE.md §4.2)"
                        ),
                    );
                }
            }
        }
        mirror.live.fetch_add(1, Ordering::AcqRel);
        // ★★★ v3-roperm: a guest-KERNEL channel lives here (`kernel_channel`: facts guest
        // userspace cannot produce), so this space is the kernel's and mirrors privileged leaves
        // from its next walk on — set above by `try_kernel`, in statement order. ⚠ A privileged
        // leaf walked BEFORE this birth was withheld (never committed) and is placed at the
        // space's next walk (its next invalidate or split). `[measured 5fead67d]` no privileged
        // leaf was ever walked in a space that turned kernel this way (UVM's): they live in
        // RM-internal clients' spaces, which are kernel from creation (`kernel_vas_for`).
        // ⊘ 2026-10-10: the legacy `force_kernel` arm (no refusal) is deleted with `KF3_TSPACE`.
        self.defer(
            "birth translated",
            Box::new(move |me: &ChanPlane| {
                let fail = |e: (u32, String)| {
                    mirror.live.fetch_sub(1, Ordering::AcqRel);
                    e
                };
                let userd = me.userd_view(a.userd).map_err(|e| fail((NV_ERR_NOT_SUPPORTED, e)))?;
                // ★ v3-initrace: what the guest's USERD held when we took the channel — a slot
                // an earlier channel on the same chid used still holds that channel's cursors.
                let userd_at_birth = (userd.load(kf_abi::submit::USERD_GP_PUT).ok(), userd.load(kf_abi::submit::USERD_GP_GET).ok());
                if let Some(v) = inject_stale_userd() {
                    // ⊘ FAULT INJECTION (`KF3_INJECT_STALE_USERD`, default off): leave the cursors a
                    // previous channel on this chid would have left.
                    let _ = userd.store(kf_abi::submit::USERD_GP_PUT, v);
                    let _ = userd.store(kf_abi::submit::USERD_GP_GET, v);
                    eprintln!("kf3: chan {:#x}:{:#x} INJECTED stale USERD GP_PUT=GP_GET={v} (KF3_INJECT_STALE_USERD)", a.client, a.handle);
                }
                // ★★★★★ v3-initrace: physical RM's allocation-time USERD initialisation — we are the
                // physical RM (`kf_chan::host::UserdInit`). Before the reply, so no guest cursor can
                // exist yet; without it the ring's first pump read an earlier channel's GP_PUT.
                let declared = match a.userd {
                    Some(kf_arch::UserdMem::Framebuffer { size, .. } | kf_arch::UserdMem::Sysmem { size, .. }) => size,
                    _ => kf_abi::submit::USERD_SIZE,
                };
                let mut userd = userd;
                let zeroed = kf_chan::host::zero_userd(&mut userd, declared)
                    .map_err(|e| fail((NV_ERR_INSUFFICIENT_RESOURCES, format!("USERD initialisation: {e}"))))?;
                // ★★★ P1+P2 inc D, hardwired 2026-10-10 (§AB rules 2-4): the ring is born in the
                // privileged T-space at a T-space ring slot, in the T-space layout — the guest's
                // mirror gets nothing of ours. No T-space: refused by name, never a mirror ring.
                let t = match me.tspace.get() {
                    Some(Ok(t)) => t,
                    _ => return Err(fail((NV_ERR_INVALID_STATE, "tspace: not built".to_string()))),
                };
                let mut host = t
                    .ring(me.rm, if kernel_gr || kernel_nvdec || kernel_nvenc || kernel_ofa { engine } else { me.host_ce }, privileged)
                    .map_err(|e| fail((NV_ERR_INSUFFICIENT_RESOURCES, e)))?;
                let ht = host.channel().token;
                let ring_va = host.va();
                // ★★★ GR tier (owner rulings 2026-10-07): USER assertion, then the allowlisted host
                // objects — refused by name, the ring freed, the birth refused, on any failure.
                let gr_tier = kernel_gr && kernel_gr_work();
                let gr_gp_get = if gr_tier {
                    let at = match a.userd {
                        Some(kf_arch::UserdMem::Framebuffer { base, .. }) => {
                            t.windows(privileged)
                                .fb(base.saturating_add(kf_abi::submit::USERD_GP_GET), 4)
                        }
                        Some(kf_arch::UserdMem::Sysmem { base, .. }) => me
                            .ram
                            .dma_to_file_range(base.saturating_add(kf_abi::submit::USERD_GP_GET), 4)
                            .and_then(|off| t.windows(privileged).ram(off, 4)),
                        _ => None,
                    };
                    let admitted = host.admit_gr_tier(me.rm).and_then(|objects| {
                        at.map(|at| (objects, at)).ok_or_else(|| {
                            "GR tier REFUSED: the guest USERD's GP_GET word is in no T-space window".to_string()
                        })
                    });
                    match admitted {
                        Ok((_, at)) => Some(at),
                        Err(e) => {
                            let _ = me.rm.free_channel(host.channel());
                            let released = host.release(me.rm).is_some_and(|l| !l.contains("REFUSED"));
                            t.give_ring(ring_va, released);
                            return Err(fail((NV_ERR_INSUFFICIENT_RESOURCES, e)));
                        }
                    }
                } else {
                    None
                };
                let gr_cfg = kf_chan::tmode::GrConfig {
                    tier: gr_tier,
                    inert_sw_subch: sw_subch_inert(),
                    deferred_api: deferred_api_trigger(),
                };
                let mut chan = {
                    let mut ring = TranslatedRing::new_tmode(a.gpfifo_va, entries, 0);
                    ring.set_gr(gr_cfg);
                    let mut c = TranslatedChannel::new(ring, host, idx);
                    c.set_tspace(t.windows(privileged), negctl_stale_bind());
                    c
                };
                if let Some(at) = gr_gp_get
                    && let Err(e) = chan.set_gpu_gp_get(at)
                {
                    let _ = me.rm.free_channel(chan.host().channel());
                    let released = chan.release_host(me.rm).is_some_and(|l| !l.contains("REFUSED"));
                    t.give_ring(ring_va, released);
                    return Err(fail((NV_ERR_INSUFFICIENT_RESOURCES, format!("GR tier REFUSED: {e}"))));
                }
                chan.set_probe(completion_probe_ms().is_some());
                chan.set_census(tcensus_on());
                // ★ P1+P2 inc A (review fix 2026-10-04): strict (hardwired 2026-10-10); the GP
                // extended base exists from Hopper's `NVC86F` on (`clc86f.h:184-189`) — kayfabe
                // presents the host family. ⊘ The T-mode SHADOW (`KF3_TSHADOW`'s shadow half,
                // `KF3_NEGCTL_SHADOW`) ran only beside the legacy rewriter, which no ring runs
                // any more: it is not armed (`KF3_TSHADOW` still turns the census on).
                chan.set_inca(
                    crate::tspace::INCA_STRICT,
                    matches!(me.family, kf_chip::Family::Hopper | kf_chip::Family::Blackwell),
                );
                let alloc = me
                    .caps
                    .lock()
                    .map_err(|_| "caps poisoned".to_string())
                    .and_then(|mut c| me.plane.allocate_channel(&mut c, idx, Route::Translated, ht, Owner::Kernel).map_err(|e| format!("{e:?}")));
                if let Err(e) = alloc {
                    let mut chan = chan;
                    let _ = me.rm.free_channel(chan.host().channel());
                    // ★ v3-appfix J: and the ring's own object, mapping and CPU view.
                    let released = chan.release_host(me.rm).is_some_and(|l| !l.contains("REFUSED"));
                    t.give_ring(ring_va, released);
                    return Err(fail((NV_ERR_INSUFFICIENT_RESOURCES, format!("token {idx:#x}: {e}"))));
                }
                let slot = Slot {
                    chan,
                    key,
                    mirror,
                    userd,
                    guest_idx: idx,
                    guest_engine: engine,
                    ctx: CtxBind::default(),
                    // ⚠ Host-owned scheduling (default off, awaiting owner confirmation).
                    scheduled: sw_runlist_host_owned(),
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
                    probe: ProbeRec::default(),
                    inca_seen: 0,
                    gr_tier,
                    err_notifier: a.error_notifier,
                    sw: SwSlot::default(),
                };
                // ★ §U: a fresh software numbering (a host token may be reused).
                if let Ok(mut m) = me.sw_objs.lock() {
                    m.insert(ht, crate::defapi::SwObjs::new((a.client, a.handle)));
                }
                if gr_tier {
                    me.engine_tlive(engine, true);
                }
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
                // ★ The kernel channels' doorbells (CeUtils, UVM) take the fast path too: only the
                // transport changes — the drainer stamps RUNG and wakes a worker, as the trap did.
                let fast = me.fast_register(idx, runlist, chid);
                Ok(format!(
                    "chan {:#x}:{:#x} BORN Translated: token {idx:#x} -> host {ht:#x} in {key:?} gpfifo={:#x}x{entries} userd={:?} engine={engine:#x} tsg={:x?} kernel_by={} ring_va={ring_va:#x} userd_at_birth(GP_PUT,GP_GET)={userd_at_birth:?} zeroed={zeroed}B gr_tier={gr_tier} gp_get_by_engine={} sw_subch_inert={} scheduled_at_birth={} {fast}",
                    a.client,
                    a.handle,
                    a.gpfifo_va,
                    a.userd,
                    a.tsg,
                    kernel_by(&a),
                    gr_gp_get.is_some(),
                    gr_cfg.inert_sw_subch,
                    sw_runlist_host_owned()
                ))
            }),
        )
    }

    /// ★ P5c (act thread): the twin's host error context over the guest's notifier record, its
    /// RC event on the plane's RC fd, and a read view of the record. `None` (named, counted) when any
    /// step refuses — the twin is then born without one, and its faults stay silent.
    fn arm_notifier(
        &self,
        client: u32,
        handle: u32,
        obj: u32,
        off: u64,
        at: kf_arch::UserdMem,
    ) -> Option<PtNotifier> {
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
                let mut n = PtNotifier {
                    ctx,
                    view,
                    reported: false,
                    at_arm: [0; 4],
                    guest_stop_write: false,
                };
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
            let _ = self.rm.release_cpu_view(kf_host::CpuViewRelease {
                h_memory: self.store,
                p_linear_address: *cookie,
            });
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
                let Some(n) = t.notifier.as_mut() else {
                    continue;
                };
                if n.reported {
                    continue;
                }
                // `NvNotification {timeStamp:8, info32:4, info16:2, status:2}`: status is the high
                // half of word 3 and is written last.
                let Some(w) = n.words() else { continue };
                // ★ v3-chanctl: the guest's own post-STOP write is re-baselined, never reported.
                if n.guest_stop_write && w != n.at_arm && w[2] == ROBUST_CHANNEL_PREEMPTIVE_REMOVAL
                {
                    n.at_arm = w;
                    continue;
                }
                if w != n.at_arm && (w[3] >> 16) != 0 {
                    n.reported = true;
                    found.push(RcEvent {
                        // ★ 2026-09-26: the GUEST chid (the index carries the runlist on Blackwell).
                        chid: self.plane.token_index.chid_of(t.idx),
                        engine: t.engine,
                        except_type: w[2],
                        host_token: t.chan.token,
                    });
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
                if kf_mem::maplog::on() {
                    eprintln!(
                        "kf3: maplog t={:.6} RC-SEEN twin host {:#x} guest chid {:#x} engine {:#x} except_type={:#x} doorbells rung={} last@{}",
                        kf_mem::maplog::t(),
                        e.host_token,
                        e.chid,
                        e.engine,
                        e.except_type,
                        self.rung
                            .get(e.chid as usize)
                            .map_or(0, |c| c.load(Ordering::Relaxed)),
                        self.last_rung(e.chid)
                    );
                }
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
        self.rc_queue
            .lock()
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default()
    }

    /// ★ P5c (drainer): put back events the GSP queue could not take now (retried next pass).
    pub fn requeue_rc(&self, back: Vec<RcEvent>) {
        if let Ok(mut q) = self.rc_queue.lock() {
            let mut back = back;
            back.append(&mut q);
            *q = back;
        }
    }

    /// ★ DIAGNOSTIC (`KF3_BAR0_TRACE`, default off; drainer, once per run): the first 64 words of
    /// the traced channel's USERD and the four words of its error notifier, as they are when the
    /// guest's teardown starts. Reads only; nothing is written or forwarded.
    pub fn bar0trace_dump(&self, t: crate::bar0trace::TraceChan) -> Vec<String> {
        let tag = format!(
            "kf3: BAR0-TRACE dump chan {:#x}:{:#x} (host {:#x})",
            t.client, t.object, t.host
        );
        let Some(slot) = self.slot(t.host) else {
            return vec![format!("{tag}: no Translated slot (already retired?)")];
        };
        let Ok(g) = slot.lock() else {
            return vec![format!("{tag}: slot poisoned")];
        };
        let words = |f: &dyn Fn(u64) -> Result<u32, String>, n: u64| -> String {
            (0..n)
                .map(|i| {
                    f(i.saturating_mul(4))
                        .map_or_else(|e| format!("?({e})"), |v| format!("{v:08x}"))
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        let mut out = vec![format!(
            "{tag}: USERD[0..64] (token {:#x}, scheduled={} serves={} last_put={:?}): {}",
            g.guest_idx,
            g.scheduled,
            g.serves,
            g.last_put,
            words(&|o| g.userd.load(o), 64)
        )];
        let notifier = match g.err_notifier {
            None => "none declared".to_string(),
            Some(kf_arch::fault::ErrorNotifier::Sysmem { gpa }) => self
                .ram
                .dma_to_file_range(gpa, 16)
                .and_then(|off| self.ram.at_file_offset(off, 16))
                .map_or_else(
                    || format!("sysmem @{gpa:#x}: no guest RAM"),
                    |(mem, at)| {
                        format!(
                            "sysmem @{gpa:#x}: {}",
                            words(
                                &|o| mem
                                    .load_u32(at.saturating_add(o as usize))
                                    .ok_or_else(|| "load".to_string()),
                                4
                            )
                        )
                    },
                ),
            Some(kf_arch::fault::ErrorNotifier::Framebuffer { off }) => {
                match self.userd_view(Some(kf_arch::UserdMem::Framebuffer {
                    base: off,
                    size: 16,
                })) {
                    Ok(v) => {
                        let s = format!("framebuffer +{off:#x}: {}", words(&|o| v.load(o), 4));
                        if let UserdView::Store { cookie, .. } = &v {
                            let _ = self.rm.release_cpu_view(kf_host::CpuViewRelease {
                                h_memory: self.store,
                                p_linear_address: *cookie,
                            });
                        }
                        s
                    }
                    Err(e) => format!("framebuffer +{off:#x}: no view ({e})"),
                }
            }
            Some(kf_arch::fault::ErrorNotifier::Unreachable) => {
                "in an aperture we cannot name".to_string()
            }
        };
        out.push(format!("{tag}: error notifier {notifier}"));
        out
    }

    /// The guest's USERD, reached through a CPU view WE arm now (off the vCPU).
    fn userd_view(&self, u: Option<kf_arch::UserdMem>) -> Result<UserdView, String> {
        match u {
            Some(kf_arch::UserdMem::Framebuffer { base, .. }) => {
                let page = base & !0xFFF;
                let (node, cookie) = self
                    .rm
                    .arm_cpu_view(
                        MapNode::Gpu,
                        self.store,
                        page,
                        0x1000,
                        ViewAccess::ReadWrite,
                    )
                    .map_err(|e| format!("USERD view of store {page:#x}: {e:?}"))?;
                let region = VolatileRegion::map(
                    Backing::DeviceFile { fd: node.as_fd() },
                    0x1000,
                    CachePolicy::Uncached,
                    HostPageSize::query(),
                )
                .map_err(|e| format!("USERD mmap: {e:?}"))?;
                Ok(UserdView::Store {
                    region,
                    _node: node,
                    cookie,
                    at: base.saturating_sub(page),
                })
            }
            Some(kf_arch::UserdMem::Sysmem { base, .. }) => {
                // ★ P1+P2 §2.6, review fix 2026-10-04: through the vIOMMU seam — the guest's DMA
                // address to a guest-memfd range, then that range's host mapping — never a GPA
                // lookup of a device address. (Equal to the block lookup it replaces for the one
                // fd-backed guest RAM kf3 supports.)
                let (mem, at) = self
                    .ram
                    .dma_to_file_range(base, 0x200)
                    .and_then(|off| self.ram.at_file_offset(off, 0x200))
                    .ok_or_else(|| format!("USERD at guest DMA address {base:#x}: no guest RAM"))?;
                Ok(UserdView::Ram { mem, at })
            }
            other => Err(format!(
                "USERD not declared as a physical descriptor ({other:?})"
            )),
        }
    }

    fn retire(&self, ht: u32) {
        let Some(slot) = self.slots.write().ok().and_then(|mut s| s.remove(&ht)) else {
            return;
        };
        // ★ The fast path goes FIRST, with NO slot lock held (it waits for the drainer's ack, and a
        // statement on the drainer may lock a slot): placements removed, the eventfd's last count
        // handed to the still-live token (its worker sees `scheduled == false`), acknowledged.
        let fast = match slot.lock() {
            Ok(g) => Some(g.guest_idx),
            Err(_) => None,
        }
        .and_then(|idx| self.dbfast.deregister(idx));
        let Ok(mut g) = slot.lock() else { return };
        if g.gr_tier {
            self.engine_tlive(g.guest_engine, false);
        }
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
            eprintln!(
                "kf3: chan token {:#x} (host {ht:#x}) STRANDED: still BUSY after 200 ms",
                g.guest_idx
            );
        }
        let freed_chan = self.rm.free_channel(g.chan.host().channel()).is_ok();
        // ★ v3-appfix J: the ring's 1 MiB object, its GPU mapping and its host BAR1 CPU view went
        // with NOTHING before — ~4.5 MiB of host BAR1 per guest CUDA process with persistence
        // mode, the 256 MiB aperture gone at ~55 processes. Only after the channel is freed (its
        // GPFIFO and USERD live in the object); the ring's VA slot is reused only when every step
        // of the release succeeded.
        let ring_va = g.chan.host().va();
        let ring_line = if freed_chan {
            g.chan.release_host(self.rm)
        } else {
            None
        };
        let released = ring_line.as_deref().is_some_and(|l| !l.contains("REFUSED"));
        // ★ P1+P2 inc D: every Translated ring is a T-space ring slot (hardwired 2026-10-10); one
        // whose release did not fully succeed leaks.
        if let Some(Ok(t)) = self.tspace.get() {
            t.give_ring(ring_va, released);
        }
        eprintln!(
            "kf3: TSPACE-RETIRE tok={:#x} host={ht:#x} key={:?} ring_va={ring_va:#x} released={released} {}",
            g.guest_idx,
            g.key,
            g.chan.stale_line()
        );
        if let Some(l) = ring_line.as_deref().filter(|l| l.contains("REFUSED")) {
            eprintln!("kf3: chan token {:#x} (host {ht:#x}) {l}", g.guest_idx);
        }
        if !freed_chan {
            eprintln!(
                "kf3: chan token {:#x} (host {ht:#x}): host channel free REFUSED — its ring is KEPT (the channel still names it)",
                g.guest_idx
            );
        }
        g.mirror.live.fetch_sub(1, Ordering::AcqRel);
        let armed = g.views.armed;
        g.views.release_all(self.rm, self.store);
        if let UserdView::Store { cookie, .. } = &g.userd {
            let _ = self.rm.release_cpu_view(kf_host::CpuViewRelease {
                h_memory: self.store,
                p_linear_address: *cookie,
            });
        }
        eprintln!(
            "kf3: DOORBELL-LEDGER tok={:#010x} route=translated emulated={} forwarded={} host={ht:#x}{}",
            g.guest_idx,
            u64::from(g.dead.is_some() && g.chan.counts().0 == 0),
            g.chan.counts().0,
            Self::fast_fields(fast)
        );
        if completion_probe_ms().is_some() {
            for (f, reads) in &g.probe.done {
                let now: Vec<String> = f
                    .releases
                    .iter()
                    .map(|r| probe_read(self.ram, &g.mirror, None, *r).to_string())
                    .collect();
                eprintln!(
                    "kf3: PROBE-RETIRE tok={:#x} fence seq={} gp_get={:?} submit->seen={}us seen {}ms before retire; at completion [{}]; at retire [{}]",
                    g.guest_idx,
                    f.seq,
                    f.gp_get,
                    f.completed
                        .map_or(0, |c| c.duration_since(f.submitted).as_micros()),
                    f.completed.map_or(0, |c| c.elapsed().as_millis()),
                    reads
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; "),
                    now.join("; ")
                );
            }
            for f in g.chan.probe_inflight() {
                eprintln!(
                    "kf3: PROBE-RETIRE tok={:#x} fence seq={} STILL IN FLIGHT at retire ({}ms) releases={:?}",
                    g.guest_idx,
                    f.seq,
                    f.submitted.elapsed().as_millis(),
                    f.releases
                );
            }
        }
        if let Some(sh) = g.chan.shadow() {
            // ★ P1+P2 inc C (`V3_P1P2_TSPACE.md` §3.6): what T-mode would have done on this channel.
            eprintln!(
                "kf3: TSHADOW tok={:#x} host={ht:#x} key={:?} {}",
                g.guest_idx,
                g.key,
                sh.line()
            );
        }
        if let Some(c) = g.chan.census() {
            // ★ P1+P2 inc A (`V3_P1P2_TSPACE.md` §3.6): one census line per Translated channel.
            eprintln!(
                "kf3: TCENSUS tok={:#x} host={ht:#x} key={:?} privilege={:?} {}",
                g.guest_idx,
                g.key,
                g.privilege,
                c.line()
            );
        }
        eprintln!(
            "kf3: chan token {:#x} (host {ht:#x}) RETIRED, forwarded={} submissions={} splits={}/{} serves={} last_put={:?} gp_get={:?} store_views={armed} privilege={:?} dead={:?} inca=[{}] gr_tier={} gr[methods={} inert_binds={} objects={:x?}] gp_get_by_engine={:?}",
            g.guest_idx,
            g.chan.counts().0,
            g.chan.counts().1,
            g.chan.counts().2,
            g.splits,
            g.serves,
            g.last_put,
            g.chan.last_gp_get(),
            g.privilege,
            g.dead,
            g.chan.inca().line(),
            g.gr_tier,
            g.chan.gr_counts().0,
            g.chan.gr_counts().1,
            g.chan.host().gr_objects(),
            g.chan.gpu_gp_get()
        );
    }

    /// ★ A WORKER's entry (`HostOps::run_translated`): pump the channel behind `ht`. Never waits.
    /// Returns whether anything reached the GPU.
    pub fn serve(&self, ht: u32) -> bool {
        if let Some(r) = self.relay_serve(ht) {
            return r;
        }
        let Some(slot) = self.slot(ht) else {
            return false;
        };
        // ★ The token's BUSY state is the exclusion; this lock must never be contended.
        let Ok(mut g) = slot.try_lock() else {
            self.contended.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        let g = &mut *g;
        g.serves = g.serves.saturating_add(1);
        if g.dead.is_some()
            || !g.scheduled
            || g.disabled
            || g.stopped
            || self.stop.load(Ordering::Acquire)
        {
            return false;
        }
        g.last_put = g.userd.load(kf_abi::submit::USERD_GP_PUT).ok();
        // Diagnostic (first 3 serves that find nothing to do): which words of the USERD page are
        // non-zero — a GP_PUT that landed elsewhere in the page shows up here. Worker thread only.
        if g.last_put == Some(0) && g.serves <= 3 {
            if let UserdView::Store { region, at, .. } = &g.userd {
                let nz: Vec<String> = (0..0x1000u64)
                    .step_by(4)
                    .filter_map(|o| {
                        region
                            .load_u32(HostOffset::new(o))
                            .ok()
                            .filter(|v| *v != 0)
                            .map(|v| format!("+{o:#x}={v:#x}"))
                    })
                    .take(16)
                    .collect();
                eprintln!(
                    "kf3: chan token {:#x}: GP_PUT=0 at USERD+{at:#x}+0x8c; non-zero words in its page: [{}]",
                    g.guest_idx,
                    nz.join(" ")
                );
            }
        }
        let before = g.chan.counts().1;
        let mirror = g.mirror.clone();
        let probe = completion_probe_ms().is_some();
        if probe
            && let Some(p) = g.last_put
            && g.probe.put_moved.is_none_or(|(q, _)| q != p)
        {
            g.probe.put_moved = Some((p, std::time::Instant::now()));
        }
        // ★ Review fix 2026-10-04 (HIGH): the CPU store views' bound is the STORE's readable
        // extent, never the mirror's window length (0 on a T-mode twin) — [`store_read_bound`].
        let store_len = store_read_bound(self.layout.carve());
        let mut mem = Mem {
            mirror: &mirror,
            ram: self.ram,
            rm: self.rm,
            store: self.store,
            store_len,
            views: &mut g.views,
            inbox: &self.inbox,
        };
        let mut split = VaSplit {
            inbox: &self.inbox,
            token: g.guest_idx,
            ticket: &mut g.split,
            requested: &mut g.splits,
            sw: SwCtx {
                plane: self,
                ht,
                space: g.key,
                st: &mut g.sw,
            },
        };
        let gp_before = g.chan.last_gp_get();
        let r = g.chan.pump(
            self.rm,
            &self.completions,
            &mut mem,
            &mut Userd(&g.userd),
            &mut split,
            is_any_ce_class,
            &NoMirrorWindow,
        );
        if !g.gr_tier
            && translated_ce_relay()
            && is_copy_engine(g.guest_engine)
            && g.chan.last_gp_get() != gp_before
        {
            // ★ The pump authored GP_GET only for entries whose host fence was REACHED (never a
            // forged completion): relay that to the guest's own CE vector.
            if let Some(e) = self
                .engines
                .iter()
                .find(|e| e.engine_type == g.guest_engine)
            {
                e.cpending.fetch_add(1, Ordering::Release);
            }
        }
        if g.gr_tier && g.chan.last_gp_get() != gp_before {
            // The engine wrote GP_GET (and the guest's semaphores) before this fence's NSI.
            self.gr_relay.fetch_add(1, Ordering::Release);
        }
        // ★ Review fix 2026-10-04 (§3.5): a stash waiting for a pending walk is rung by the VA
        // thread once it is idle again.
        if g.chan.waiting_on_walk() {
            self.inbox.wait_walk(g.guest_idx);
        }
        // ★ P1+P2 inc A: what this pump counted (count-only), summed for the status line.
        let inca = g.chan.inca().total();
        if inca > g.inca_seen {
            self.inca_counted
                .fetch_add(inca.saturating_sub(g.inca_seen), Ordering::Relaxed);
            g.inca_seen = inca;
        }
        if probe {
            for f in g.chan.take_completed() {
                let reads: Vec<ReleaseRead> = f
                    .releases
                    .iter()
                    .map(|r| {
                        probe_read(
                            self.ram,
                            &mirror,
                            Some((&mut g.views, self.rm, self.store, store_len)),
                            *r,
                        )
                    })
                    .collect();
                let bad = reads.iter().any(ReleaseRead::not_landed);
                let dt = f
                    .completed
                    .map_or(0, |c| c.duration_since(f.submitted).as_micros());
                // ★ v3-initrace: the data the launches moved, host side vs the guest's CPU views.
                let mut data = Vec::new();
                let mut copy_bad = false;
                for l in &f.launches {
                    let (src, sb) = match l.src {
                        Some(o) => probe_side(
                            self.ram,
                            &mirror,
                            Some((&mut g.views, self.rm, self.store, store_len)),
                            o,
                        ),
                        None => ("-".to_string(), None),
                    };
                    let (dst, db) = match l.dst {
                        Some(o) => probe_side(
                            self.ram,
                            &mirror,
                            Some((&mut g.views, self.rm, self.store, store_len)),
                            o,
                        ),
                        None => ("-".to_string(), None),
                    };
                    let remap = l.launch & kf_abi::submit::ce::LAUNCH_REMAP_ENABLE != 0;
                    let differ = !remap && matches!((&sb, &db), (Some(a), Some(b)) if a != b);
                    copy_bad |= differ;
                    data.push(format!(
                        "launch={:#x} src {src} dst {dst}{}",
                        l.launch,
                        if differ { " ⊘ DST != SRC" } else { "" }
                    ));
                }
                if g.probe.logged < 8 || bad || copy_bad {
                    g.probe.logged = g.probe.logged.saturating_add(1);
                    eprintln!(
                        "kf3: PROBE t={:.6} tok={:#x} fence seq={} gp_get={:?} submit->seen-complete={dt}us put={:?} releases=[{}]{} data=[{}]",
                        kf_mem::maplog::t(),
                        g.guest_idx,
                        f.seq,
                        f.gp_get,
                        g.last_put,
                        reads
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join("; "),
                        if bad {
                            " ⊘ A RELEASE DID NOT LAND WHERE OUR ROWS PLACE IT"
                        } else {
                            ""
                        },
                        data.join("; ")
                    );
                }
                g.probe.put_at_done = Some((
                    g.last_put,
                    f.completed.unwrap_or_else(std::time::Instant::now),
                ));
                g.probe.dumped_guest = false;
                if g.probe.done.len() >= 4 {
                    g.probe.done.pop_front();
                }
                g.probe.done.push_back((f, reads));
            }
        }
        if let Err(e) = r {
            let why = match e {
                ChanError::Ring(r) => format!("ring: {r:?}"),
                ChanError::Host(h) => format!("host: {h}"),
                ChanError::Userd(u) => format!("userd: {u}"),
                ChanError::Publish(p) => format!("split: {p}"),
                // ★ P1+P2 inc D: T-mode refused an item at bind, by name.
                ChanError::Bind(r) => format!("tspace bind: {} {r:x?}", r.reason()),
                // ★ §U: a software method refused, or its host-authored action failed.
                ChanError::Sw(w) => format!("software method: {w}"),
            };
            eprintln!(
                "kf3: chan token {:#x} ({:?}) DEAD: {why}",
                g.guest_idx, g.key
            );
            // ★ 2026-10-07 (Windows Code43, run41: a kernel copy channel died with an EMPTY rows
            // map although the walker had mapped its ring into that space): is the channel's
            // mirror still the plane's mirror for its key? Error path only; never waits.
            let plane_now = self.mirrors.try_lock().ok().map(|m| {
                m.get(&g.key).map(|now| {
                    (
                        std::sync::Arc::ptr_eq(&now.rows, &g.mirror.rows),
                        now.space.space,
                        now.rows.read().map_or(usize::MAX, |r| r.len()),
                    )
                })
            });
            eprintln!(
                "kf3: chan token {:#x} death mirror: own space={:#x} rows={} log_epoch={} last row commits (epoch, lo, hi, ms ago)={:x?}; plane's mirror for the key now (same rows, space, rows): {plane_now:?}",
                g.guest_idx,
                g.mirror.space.space,
                g.mirror.rows.read().map_or(usize::MAX, |r| r.len()),
                g.mirror.log.epoch(),
                g.mirror.log.recent(8)
            );
            g.dead = Some(why);
            self.completions.clear(g.guest_idx);
        }
        g.chan.counts().1 > before
    }

    /// ★ `KF3_MAPLOG` (diagnostic): the passthrough twins in host space `space`, each as
    /// `chid:engine=rung` — the doorbells the guest has rung on it so far (a relaxed read).
    #[must_use]
    pub fn pt_doorbells(&self, space: u32) -> String {
        // ⊘ try_lock: the VA thread must never wait on a lock the act thread may hold.
        let Ok(m) = self.pt.try_lock() else {
            return "[pt busy]".into();
        };
        let mut v: Vec<(u32, u32, u64)> = m
            .values()
            .filter(|t| t.space.space == space)
            .map(|t| {
                (
                    t.idx,
                    t.engine,
                    self.rung
                        .get(t.idx as usize)
                        .map_or(0, |c| c.load(Ordering::Relaxed)),
                )
            })
            .collect();
        v.sort_unstable();
        format!(
            "[{}]",
            v.iter()
                .map(|(i, e, n)| format!("{i:#x}:{e:#x}={n}@{}", self.last_rung(*i)))
                .collect::<Vec<_>>()
                .join(" ")
        )
    }

    /// `KF3_MAPLOG`: when guest token `idx` last rang (seconds on the maplog clock), or `-`.
    fn last_rung(&self, idx: u32) -> String {
        match self
            .rung_at_us
            .get(idx as usize)
            .map_or(0, |c| c.load(Ordering::Relaxed))
        {
            0 => "-".into(),
            us => format!("{:.6}", us as f64 / 1e6),
        }
    }

    /// ★ v3-initrace (`KF3_COMPLETION_PROBE`, the probe thread — never a vCPU, never the drainer):
    /// every Translated channel whose oldest in-flight fence is older than `overdue` (the HOST has
    /// not completed it), or whose newest completion is older than `overdue` while the guest's
    /// `GP_PUT` has not moved since (the guest never came back for more), is dumped ONCE per
    /// condition. Guest-RAM words are re-read now; a vidmem word is named, not read (no view is
    /// armed off a worker). ⊘ `try_lock` only: a slot a worker holds is skipped this tick.
    pub fn probe_tick(&self, overdue: std::time::Duration) -> Vec<String> {
        let Ok(s) = self.slots.read() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (&ht, slot) in s.iter() {
            let Ok(mut g) = slot.try_lock() else { continue };
            let g = &mut *g;
            let infl = g.chan.probe_inflight();
            let host_late = infl
                .first()
                .is_some_and(|f| f.submitted.elapsed() > overdue);
            let put_now = g.userd.load(kf_abi::submit::USERD_GP_PUT).ok();
            let get_now = g.userd.load(kf_abi::submit::USERD_GP_GET).ok();
            let guest_silent = g
                .probe
                .put_at_done
                .is_some_and(|(p, at)| at.elapsed() > overdue && put_now == p);
            let why = if host_late && !g.probe.dumped_host {
                g.probe.dumped_host = true;
                "HOST-FENCE-OVERDUE (submitted, never seen complete)"
            } else if guest_silent && !g.probe.dumped_guest {
                g.probe.dumped_guest = true;
                "GUEST-SILENT-AFTER-COMPLETION (its GP_PUT has not moved since our fence completed)"
            } else {
                continue;
            };
            let mirror = g.mirror.clone();
            let cursors = g.chan.host_cursors();
            let (fwd, subs, walks) = g.chan.counts();
            let mut lines = vec![format!(
                "kf3: PROBE-DUMP t={:.6} tok={:#x} host={ht:#x} {why}: key={:?} forwarded={fwd} submissions={subs} walks={walks} serves={} guest USERD GP_PUT={put_now:?} GP_GET={get_now:?} (authored {:?}) put_moved={} our ring GP_GET/GP_PUT/fence={cursors:?} dead={:?} scheduled={} stopped={} disabled={}",
                kf_mem::maplog::t(),
                g.guest_idx,
                g.key,
                g.serves,
                g.chan.last_gp_get(),
                g.probe
                    .put_moved
                    .map_or("never".to_string(), |(p, at)| format!(
                        "{p} {}ms ago",
                        at.elapsed().as_millis()
                    )),
                g.dead,
                g.scheduled,
                g.stopped,
                g.disabled
            )];
            for f in &infl {
                lines.push(format!(
                    "kf3: PROBE-DUMP   in-flight fence seq={} gp_get={:?} submitted {}ms ago releases={:?}",
                    f.seq,
                    f.gp_get,
                    f.submitted.elapsed().as_millis(),
                    f.releases
                ));
            }
            for (f, reads) in &g.probe.done {
                let now: Vec<String> = f
                    .releases
                    .iter()
                    .map(|r| probe_read(self.ram, &mirror, None, *r).to_string())
                    .collect();
                let data: Vec<String> = f
                    .launches
                    .iter()
                    .map(|l| {
                        let side = |x: Option<kf_chan::translated::Operand>| {
                            x.map_or("-".to_string(), |o| {
                                probe_side(self.ram, &mirror, None, o).0
                            })
                        };
                        format!(
                            "launch={:#x} src {} dst {}",
                            l.launch,
                            side(l.src),
                            side(l.dst)
                        )
                    })
                    .collect();
                lines.push(format!(
                    "kf3: PROBE-DUMP   completed fence seq={} gp_get={:?} submit->seen={}us seen {}ms ago; at completion [{}]; NOW [{}] data NOW [{}]",
                    f.seq,
                    f.gp_get,
                    f.completed.map_or(0, |c| c.duration_since(f.submitted).as_micros()),
                    f.completed.map_or(0, |c| c.elapsed().as_millis()),
                    reads.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "),
                    now.join("; "),
                    data.join("; ")
                ));
            }
            out.extend(lines);
        }
        out
    }

    /// Whether the channel behind `ht` can take work (a dead one cannot: §7 then poisons).
    #[must_use]
    pub fn alive(&self, ht: u32) -> bool {
        // ★ 2026-10-08 (`V3_USERD_RELAY.md`): a relayed twin is served by the worker too; it has no
        // Translated slot (its ring is the guest's, never pumped), so its liveness is the relay's.
        // [measured, run67 at 8288ff8e] without this the plane judged it untranslatable and never
        // ran a relay step (gets=0).
        if self.relays.lock().is_ok_and(|m| m.contains_key(&ht)) {
            return true;
        }
        self.slot(ht)
            .is_some_and(|s| s.try_lock().map_or(true, |g| g.dead.is_none()))
    }

    /// `forwarded=` per token, for the boot log.
    #[must_use]
    pub fn counts(&self) -> Vec<TokenCount> {
        let Ok(s) = self.slots.read() else {
            return Vec::new();
        };
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
mod dispsw_tests {
    use super::{
        CtxBind, DISPSW_WITHDRAW, PtChan, PtMap, attach_dispsw_withdraw, dispsw_live,
        dispsw_mark_host_refused, register_other_sw, take_twin_object, withdraw_kept,
    };
    use crate::dispsw::{
        DispSwCounters, DispSwHost, Live, NV_ERR_INSUFFICIENT_RESOURCES, PER_CHANNEL_CAP, twin_one,
    };
    use kf_chip::classes::Kind;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    const A: u32 = 0xc1d0_0001;
    const B: u32 = 0xc1d0_0002;
    const CH1: u32 = 0x5c00_0010;
    const CH2: u32 = 0x5c00_0011;

    /// A twin as the plane holds it, with `disp_sw` (guest handle → host object) on it.
    fn twin(token: u32, disp_sw: &[(u32, u32)]) -> PtChan {
        PtChan {
            chan: kf_host::Channel {
                tsg: token,
                chan: token,
                token,
                born_user: None,
            },
            idx: token,
            tsg: None,
            ctx_share: 0,
            parent: 0,
            device: 0,
            engine: 0,
            objects: HashMap::new(),
            disp_sw: disp_sw.iter().copied().collect(),
            sw_ids: crate::dispsw::SwClassIds::default(),
            dispsw_host_refused: false,
            defapi_host: None,
            ctx: CtxBind::default(),
            user_work: false,
            live: Arc::new(AtomicU64::new(0)),
            twin: None,
            notifier: None,
            stopped: false,
            disabled: false,
            space: kf_host::VaSpace {
                space: 0,
                range: 0,
                guest: [kf_host::channel::GuestVaRange::default(); 3],
            },
            rows: crate::mem::PlacedRows::default(),
            ledger: None,
            falcon_ctx: None,
            ring_at: (0, 0, None),
        }
    }

    /// Two clients, three twinned channels — client A's `CH1` and `CH2`, and client B's `CH1` (the
    /// same handle in another client): 3 + 1 + 2 display-SW twins.
    fn plane_map() -> Mutex<PtMap> {
        let mut m = PtMap::new();
        m.insert(
            (A, CH1),
            twin(1, &[(0x70, 0xa0), (0x71, 0xa1), (0x72, 0xa2)]),
        );
        m.insert((A, CH2), twin(2, &[(0x73, 0xa3)]));
        m.insert((B, CH1), twin(3, &[(0x70, 0xb0), (0x71, 0xb1)]));
        Mutex::new(m)
    }

    fn live(chan: usize, vm: usize) -> Live {
        Live {
            chan,
            vm,
            host_refused_before: false,
        }
    }

    /// A host that frees (recorded) and must never be asked to allocate or read back.
    #[derive(Default)]
    struct FreesOnly {
        frees: RefCell<Vec<u32>>,
    }

    impl DispSwHost for FreesOnly {
        type Chan = ();
        fn alloc(&self, (): ()) -> Result<u32, String> {
            panic!("no host alloc expected")
        }
        fn class_id(&self, (): (), _: u32) -> Result<u16, String> {
            panic!("no host readback expected")
        }
        fn free(&self, object: u32) -> Result<(), String> {
            self.frees.borrow_mut().push(object);
            Ok(())
        }
    }

    /// ★ The caps' input (review 2026-10-03, LOW: a body of zeros kept CI green and the caps never
    /// fired). Per channel, keyed by client AND handle (B's `CH1` is not A's); the VM sums every
    /// client's channels; a freed channel's twins stop counting at once (its free takes the twin out
    /// of the map at statement time); a channel no twin holds counts 0 on itself and still sees the
    /// VM; a poisoned map is full. Then the count drives the cap: a channel holding
    /// [`PER_CHANNEL_CAP`] is refused with no host call.
    #[test]
    fn the_live_count_sums_every_client_and_drops_a_freed_channel() {
        let pt = plane_map();
        assert_eq!(dispsw_live(&pt, (A, CH1)), live(3, 6));
        assert_eq!(dispsw_live(&pt, (A, CH2)), live(1, 6));
        assert_eq!(dispsw_live(&pt, (B, CH1)), live(2, 6));
        assert_eq!(dispsw_live(&pt, (B, CH2)), live(0, 6), "no twin there");
        // The twin's host-refusal mark is handed to the act with the counts — its own only.
        if let Some(v) = pt.lock().expect("map").get_mut(&(A, CH2)) {
            v.dispsw_host_refused = true;
        }
        assert!(dispsw_live(&pt, (A, CH2)).host_refused_before);
        assert!(!dispsw_live(&pt, (A, CH1)).host_refused_before);
        assert!(!dispsw_live(&pt, (B, CH1)).host_refused_before);
        let freed = pt.lock().expect("map").remove(&(A, CH1));
        assert!(freed.is_some());
        assert_eq!(dispsw_live(&pt, (A, CH1)), live(0, 3));
        assert_eq!(dispsw_live(&pt, (B, CH1)), live(2, 3));
        // Full channel: the count it reads is what refuses the next alloc, before any host call.
        let full: Vec<(u32, u32)> = (0..PER_CHANNEL_CAP as u32)
            .map(|i| (0x100 + i, 0xc00 + i))
            .collect();
        pt.lock().expect("map").insert((A, CH1), twin(4, &full));
        let at_cap = dispsw_live(&pt, (A, CH1));
        assert_eq!(at_cap, live(PER_CHANNEL_CAP, PER_CHANNEL_CAP + 3));
        let c = DispSwCounters::default();
        let e = twin_one(&FreesOnly::default(), (), 1, at_cap, &c).expect_err("capped");
        assert_eq!(e.0, NV_ERR_INSUFFICIENT_RESOURCES, "{}", e.1);
        assert_eq!(c.capped.load(Ordering::Relaxed), 1);
        // Poisoned: counted full, so the caps refuse.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _held = pt.lock().expect("map");
            panic!("poison the map (expected in this test)");
        }));
        assert!(pt.is_poisoned());
        assert_eq!(dispsw_live(&pt, (B, CH1)), live(usize::MAX, usize::MAX));
    }

    /// ★ The host-refusal mark (review 2026-10-03, LOW) lands on its own twin only — never on
    /// another client's same handle, never on a twin re-created under the key (another host
    /// channel), never on a gone one.
    #[test]
    fn the_host_refusal_mark_lands_on_its_own_twin_only() {
        let pt = plane_map();
        let marked = |key: (u32, u32)| {
            pt.lock()
                .expect("map")
                .get(&key)
                .is_some_and(|v| v.dispsw_host_refused)
        };
        let ch = pt.lock().expect("map")[&(A, CH1)].chan;
        assert!(dispsw_mark_host_refused(&pt, (A, CH1), ch));
        assert!(marked((A, CH1)));
        assert!(!marked((B, CH1)) && !marked((A, CH2)));
        let other = pt.lock().expect("map")[&(A, CH2)].chan;
        assert!(
            !dispsw_mark_host_refused(&pt, (B, CH1), other),
            "a different host channel under the key"
        );
        assert!(!marked((B, CH1)));
        assert!(!dispsw_mark_host_refused(&pt, (B, CH2), ch), "no twin");
    }

    /// ★ Another `ENG_SW` object (review 2026-10-03, MEDIUM; its test, LOW): the guest numbered it on
    /// ONE channel, so that twin's mirror advances and no other's — the next display-SW twin there
    /// must carry the number after it.
    #[test]
    fn another_eng_sw_object_advances_only_its_own_channels_mirror() {
        let pt = plane_map();
        assert_eq!(register_other_sw(&pt, (A, CH1)), Some(Some(1)));
        assert_eq!(register_other_sw(&pt, (A, CH1)), Some(Some(2)));
        assert_eq!(
            register_other_sw(&pt, (B, CH1)),
            Some(Some(1)),
            "B's own count"
        );
        assert_eq!(register_other_sw(&pt, (B, CH2)), None, "no twin");
        let mut m = pt.lock().expect("map");
        assert_eq!(
            m.get_mut(&(A, CH1)).map(|v| v.sw_ids.register()),
            Some(Some(3))
        );
        assert_eq!(
            m.get_mut(&(A, CH2)).map(|v| v.sw_ids.register()),
            Some(Some(1))
        );
    }

    /// ★ The undo's body (review 2026-10-03, LOW): it takes the kept twin out of the twin's map and
    /// the object index, frees it once and counts it; a second run, an act that kept nothing
    /// (`0`), or a later twin under the same handle is left alone.
    #[test]
    fn the_undo_withdraws_the_kept_twin_and_frees_it_once() {
        let pt = plane_map();
        let objs = Mutex::new(HashMap::from([
            ((A, 0x70), (A, CH1)),
            ((B, 0x70), (B, CH1)),
        ]));
        let host = FreesOnly::default();
        let c = DispSwCounters::default();
        let line = withdraw_kept(&host, &pt, &objs, (A, CH1), 0x70, 0xa0, &c);
        assert!(line.contains("withdrawn and freed"), "{line}");
        assert_eq!(*host.frees.borrow(), vec![0xa0]);
        assert_eq!(c.withdrawn.load(Ordering::Relaxed), 1);
        assert!(
            !pt.lock().expect("map")[&(A, CH1)]
                .disp_sw
                .contains_key(&0x70)
        );
        assert!(!objs.lock().expect("objs").contains_key(&(A, 0x70)));
        assert!(
            objs.lock().expect("objs").contains_key(&(B, 0x70)),
            "B's is B's"
        );
        for (h, why) in [(0xa0, "twice"), (0, "kept nothing")] {
            let line = withdraw_kept(&host, &pt, &objs, (A, CH1), 0x70, h, &c);
            assert!(line.contains("nothing of this act's"), "{why}: {line}");
        }
        // A later twin under the same handle is not this act's.
        if let Some(v) = pt.lock().expect("map").get_mut(&(A, CH1)) {
            v.disp_sw.insert(0x70, 0xa9);
        }
        let line = withdraw_kept(&host, &pt, &objs, (A, CH1), 0x70, 0xa0, &c);
        assert!(line.contains("nothing of this act's"), "{line}");
        assert_eq!(*host.frees.borrow(), vec![0xa0]);
        assert_eq!(c.withdrawn.load(Ordering::Relaxed), 1);
    }

    /// ★ The undo's wiring (review 2026-10-03, LOW): attached to the act's cell, it queues NOTHING
    /// until the cell's owner runs it (an act that succeeded under a refused reply,
    /// `kf_gsp`'s `settle_deferred`), then exactly one withdraw on the act queue, and never twice.
    #[test]
    fn an_orphaned_display_sw_act_queues_exactly_one_withdraw() {
        let d = kf_gsp::Deferred::new();
        let (tx, rx) = std::sync::mpsc::channel();
        attach_dispsw_withdraw(&d, tx, Arc::new(AtomicU32::new(0xa0)), (A, CH1), 0x70);
        assert!(rx.try_recv().is_err(), "nothing until orphaned");
        d.resolve(0);
        assert!(d.run_orphan_undo());
        let (_act, _cell, what) = rx.try_recv().expect("the withdraw is queued");
        assert_eq!(what, DISPSW_WITHDRAW);
        assert!(!d.run_orphan_undo(), "once");
        assert!(rx.try_recv().is_err());
    }

    /// ★ A guest free of a display-SW object on a LIVE twin is ours (`Some(_, true)`), so the plane
    /// frees its host object — without the `disp_sw` lookup the free answered NotOurs and the
    /// host object lived until its channel went. An engine object is found first and stays
    /// `false`; a handle the twin does not hold is `None`. (The rest of the display-SW decisions
    /// and their tests are `crate::dispsw`'s.)
    #[test]
    fn a_guest_free_takes_the_display_sw_twin_from_a_live_channel() {
        let mut objects: HashMap<u32, (u32, Kind)> = HashMap::new();
        let mut ds: HashMap<u32, u32> = HashMap::new();
        objects.insert(0x10, (0xa1, Kind::ThreeD));
        ds.insert(0x20, 0xb2);
        assert_eq!(
            take_twin_object(&mut objects, &mut ds, 0x20),
            Some((0xb2, true))
        );
        assert!(ds.is_empty(), "taken, so a second free finds nothing");
        assert_eq!(take_twin_object(&mut objects, &mut ds, 0x20), None);
        assert_eq!(
            take_twin_object(&mut objects, &mut ds, 0x10),
            Some((0xa1, false))
        );
        assert_eq!(take_twin_object(&mut objects, &mut ds, 0x30), None);
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
        let (c, dev, tsg, ch, sib) = (
            0xc1d0_000b,
            0x5c00_0001,
            0xcafe_0010,
            0xcafe_0013,
            0xcafe_0014,
        );
        let member = ChanScope {
            tsg: Some(tsg),
            parent: tsg,
            device: dev,
        };
        assert!(member.freed_by((c, ch), c, ch), "the channel itself");
        assert!(member.freed_by((c, ch), c, tsg), "its group");
        assert!(
            member.freed_by((c, ch), c, dev),
            "its DEVICE (a group member's parent is the group)"
        );
        assert!(member.freed_by((c, ch), c, c), "its client");
        assert!(
            !member.freed_by((c, ch), c, sib),
            "a sibling's free leaves it"
        );
        assert!(
            !member.freed_by((c, ch), 0xc1d0_000c, tsg),
            "another client's same handle"
        );
        assert!(
            !member.freed_by((c, ch), c, 0xdead_0001),
            "an unrelated object"
        );
        let bare = ChanScope {
            tsg: None,
            parent: dev,
            device: dev,
        };
        assert!(bare.freed_by((c, ch), c, dev));
        assert!(!bare.freed_by((c, ch), c, tsg));
    }
}

#[cfg(test)]
mod heap_tests {
    use super::heap_bounds;
    use kf_arch::UserdMem;
    use kf_arch::fault::ErrorNotifier;

    /// ★ P1+P2 inc A / S1-43 (`docs/design/V3_P1P2_TSPACE.md` §7 test 17): a birth whose FB USERD
    /// or FB notifier names the firmware carve-out (kayfabe's root pages), the store's end or 2^40
    /// is refused before any host call; the USERD bound uses the DECLARED size when it is larger
    /// than `NV_RAMUSERD_CHAN_SIZE`; sysmem descriptors are not this check's business.
    #[test]
    fn userd_and_notifier_bounded_to_the_usable_heap() {
        let l = kf_chip::bar0::fb_layout(12 << 30).expect("layout");
        let carve = l.carve();
        let fb = |base, size| Some(UserdMem::Framebuffer { base, size });
        let nfb = |off| Some(ErrorNotifier::Framebuffer { off });
        assert_eq!(
            heap_bounds(&l, fb(0x10_0000, 0x200), nfb(0x20_0000)),
            Ok(())
        );
        assert_eq!(heap_bounds(&l, fb(carve - 0x200, 0x200), None), Ok(()));
        for base in [
            carve,
            l.bar1_pde_base,
            l.bar2_pde_base,
            l.fb_length - 8,
            1 << 40,
        ] {
            assert!(
                heap_bounds(&l, fb(base, 0x200), None).is_err(),
                "USERD {base:#x}"
            );
            assert!(
                heap_bounds(&l, None, nfb(base)).is_err(),
                "notifier {base:#x}"
            );
        }
        // The declared size counts: 512 bytes fit below the carve-out, 4 KiB do not.
        assert!(heap_bounds(&l, fb(carve - 0x200, 0x1000), None).is_err());
        // A declared size smaller than the engine's footprint is bounded at the footprint.
        assert!(heap_bounds(&l, fb(carve - 0x10, 0x10), None).is_err());
        // Sysmem: bounded at its guest-RAM lookup, not here.
        assert_eq!(
            heap_bounds(
                &l,
                Some(UserdMem::Sysmem {
                    base: 1 << 40,
                    size: 0x200
                }),
                Some(ErrorNotifier::Sysmem { gpa: 1 << 40 })
            ),
            Ok(())
        );
    }

    /// ★ Review fix 2026-10-04 (`V3_P1P2_TSPACE.md` §8): the S1-43 bound REFUSES only when strict
    /// (hardwired ON 2026-10-10; was `KF3_INCA_REFUSE=1` / `KF3_TSPACE=1`); the count-only arm COUNTS and
    /// proceeds as before inc A; a birth inside the heap is neither.
    #[test]
    fn the_heap_bound_refuses_only_when_strict() {
        use super::{HeapGate, heap_gate};
        let l = kf_chip::bar0::fb_layout(12 << 30).expect("layout");
        let out = Some(UserdMem::Framebuffer {
            base: l.carve(),
            size: 0x200,
        });
        let inside = Some(UserdMem::Framebuffer {
            base: 0x10_0000,
            size: 0x200,
        });
        assert!(matches!(
            heap_gate(true, false, &l, out, None),
            HeapGate::Refuse(_)
        ));
        assert!(matches!(
            heap_gate(false, false, &l, out, None),
            HeapGate::Count(_)
        ));
        assert_eq!(heap_gate(true, false, &l, inside, None), HeapGate::Inside);
        assert_eq!(heap_gate(false, false, &l, inside, None), HeapGate::Inside);
        // The positive control (`KF3_NEGCTL_HEAP`): every birth counted, none refused — strict
        // or not, inside the heap or not.
        for strict in [false, true] {
            for u in [out, inside] {
                assert!(matches!(
                    heap_gate(strict, true, &l, u, None),
                    HeapGate::Count(_)
                ));
            }
        }
    }
}

#[cfg(test)]
mod store_bound_tests {
    use super::{store_read_bound, view_span};

    /// ★ Review fix 2026-10-04 (HIGH): a twin records NO store window, yet its Translated
    /// channel's GPFIFO and pushbuffer are read through vidmem rows — the CPU view bound is the
    /// carve-out base (a heap offset reads; the carve-out does not).
    #[test]
    fn a_tmode_twin_reads_vidmem_rows_through_the_store_bound() {
        let l = kf_chip::bar0::fb_layout(12 << 30).expect("layout");
        let carve = l.carve();
        let twin_fb_len = 0; // what a twin holds: no window
        let bound = store_read_bound(carve);
        assert_eq!(view_span(bound, 0x10_0040), Ok((0x10_0000, 0x1_0000)));
        assert_eq!(
            view_span(bound, carve - 4).map(|(o, n)| o + n),
            Ok(carve),
            "the last heap page reads, up to the carve-out"
        );
        assert!(
            view_span(bound, carve).is_err(),
            "the carve-out never reads"
        );
        assert!(view_span(bound, l.bar1_pde_base).is_err());
        // ⊘ The defect: the window length as the bound refuses every vidmem read.
        assert!(view_span(twin_fb_len, 0x10_0040).is_err());
    }
}

#[cfg(test)]
mod rows_tests {
    use super::resolve_rows;
    use kf_host::MapPerm;

    /// ★ P1+P2 inc C (`V3_P1P2_TSPACE.md` §3.4): an operand resolves through OUR placement rows —
    /// adjacent rows contiguous in VA and backing with one permission merge into one span; a
    /// change of backing or permission starts a new one; the first uncovered byte is named.
    #[test]
    fn an_operand_resolves_through_our_rows_and_names_its_first_hole() {
        let ro = MapPerm {
            read_only: true,
            ..MapPerm::READ_WRITE
        };
        let rows = crate::mem::PlacedRows::default();
        if let Ok(mut r) = rows.write() {
            r.insert(0x1000, (0x1000, 0x10_0000, false, MapPerm::READ_WRITE));
            r.insert(0x2000, (0x1000, 0x10_1000, false, MapPerm::READ_WRITE)); // contiguous
            r.insert(0x3000, (0x1000, 0x10_2000, false, ro)); // same backing run, read-only
            r.insert(0x4000, (0x1000, 0x50_0000, true, MapPerm::READ_WRITE)); // guest RAM
        }
        let spans = resolve_rows(&rows, 0x1800, 0x3000).unwrap();
        let shape: Vec<(bool, u64, u64, bool)> = spans
            .iter()
            .map(|s| (s.ram, s.off, s.len, s.perm.read_only))
            .collect();
        assert_eq!(
            shape,
            vec![
                (false, 0x10_0800, 0x1800, false),
                (false, 0x10_2000, 0x1000, true),
                (true, 0x50_0000, 0x800, false)
            ]
        );
        assert_eq!(resolve_rows(&rows, 0x4800, 0x1000), Err(0x5000));
        assert_eq!(resolve_rows(&rows, 0x800, 0x10), Err(0x800));
    }
}

#[cfg(test)]
mod sw_scan_tests {
    use super::{SwMethod, scan_sw_methods};

    /// ★ run73 diagnostic: the segments peeked in run72 (`[measured, run72 at 8de8ef26]`) carry no
    /// software-subchannel method — the 3D report-semaphore pair on subchannel 0 and the CE
    /// semaphore + LAUNCH_DMA on subchannel 4 are engine methods.
    #[test]
    fn run72_first_segments_have_no_software_method() {
        let gr = [
            0x2004_06c0,
            1,
            0x2024_4000,
            1,
            0x1000_0004,
            0x2004_06c0,
            1,
            0x2024_4010,
            1,
            4,
        ];
        let s = scan_sw_methods(&gr);
        assert!(s.hits.is_empty());
        assert_eq!(s.per_subch[0], 2);
        assert_eq!(s.undecodable_at, None);
        let ce = [
            0x2003_8090,
            1,
            0x2035_3000,
            1,
            0x2001_80c0,
            8,
            0x2003_8090,
            1,
            0x2035_3010,
            1,
            0x2001_80c0,
            0x10,
        ];
        let s = scan_sw_methods(&ce);
        assert!(s.hits.is_empty());
        assert_eq!(s.per_subch[4], 4);
    }

    /// A method on subchannel 5 (the deferred-API trigger shape Windows' kernel channels use,
    /// `[measured, run72 at 8de8ef26, 2026-10-08]` "subch 5 value 0x1 method 0x200") and a SetObject on subchannel 6 are
    /// software methods; a Host-only method on subchannel 7 (NOP, `0x8`) is not.
    #[test]
    fn software_subchannel_methods_are_found_and_host_methods_are_not() {
        // INC subch 5 method 0x200 count 1, data 0x40000002; IMMD subch 6 SET_OBJECT data 0x123;
        // INC subch 7 method 0x8 (NOP, Host-only) count 1.
        let words = [
            0x2001_a080,
            0x4000_0002,
            0x8000_0000 | (0x123 << 16) | (6 << 13),
            0x2001_e002,
            0,
        ];
        let s = scan_sw_methods(&words);
        assert_eq!(
            s.hits,
            vec![
                SwMethod {
                    word: 0,
                    subch: 5,
                    method: 0x200,
                    data: Some(0x4000_0002)
                },
                SwMethod {
                    word: 2,
                    subch: 6,
                    method: 0,
                    data: Some(0x123)
                },
            ]
        );
        assert_eq!(s.per_subch[7], 1);
    }

    /// ⚠ The after-run76 fence finder: the kernel-inserted epilogue of a Windows per-process GR
    /// segment (`[measured, run76 at f649d2c3]` token 0x15: a one-word release of the fence value,
    /// then a four-word timestamp report) yields the ONE-WORD release; a copy-engine semaphore yields
    /// its own; the last one in the segment wins.
    #[test]
    fn the_fence_finder_takes_the_last_one_word_release() {
        use super::last_fence_release;
        let gr = [
            0x2004_06c0,
            0x0000_0001,
            0x200e_7000,
            0x0000_000c,
            0x1000_0004,
            0x2004_06c0,
            0x0000_0001,
            0x200e_7020,
            0x0000_0000,
            0x0000_0004,
        ];
        assert_eq!(last_fence_release(&gr), Some(("3d", 0x1_200e_7000, 0xc)));
        let ce = [
            0x2003_8090,
            0x0000_0001,
            0x200e_b000,
            0x0000_0009,
            0x2001_80c0,
            0x0000_0008,
        ];
        assert_eq!(last_fence_release(&ce), Some(("ce", 0x1_200e_b000, 9)));
        // An acquire (OPERATION = 1) is not a release.
        assert_eq!(
            last_fence_release(&[0x2004_06c0, 1, 0x1000, 5, 0x1000_0001]),
            None
        );
        // Truncated: the header promises more words than were read — nothing, never a read past.
        assert_eq!(last_fence_release(&[0x2004_06c0, 1, 0x1000]), None);
        assert_eq!(last_fence_release(&[]), None);
    }

    /// Hostile input: a count that runs past the words read never reads past them, and an
    /// undefined header stops the scan by name.
    #[test]
    fn scan_is_bounded_and_stops_at_an_undefined_header() {
        let s = scan_sw_methods(&[0x3fff_a080]); // INC subch 5, count 0x1fff, no data words
        assert_eq!(s.hits.len(), 1);
        assert_eq!(s.hits[0].data, None);
        let s = scan_sw_methods(&[0xc000_0000, 0x2001_a080, 1]); // RESERVED6 first
        assert_eq!(s.undecodable_at, Some(0));
        assert!(s.hits.is_empty());
    }
}

#[cfg(test)]
mod twin_defapi_tests {
    use super::{TwinDefapiPlan, twin_defapi_plan};

    /// ⚠ EXPERIMENT `KF3_WIN_TWIN_DEFAPI_OBJECT`: off by default (the switch is read from the
    /// environment, unset in production launchers), and even on it authors a host object only for a
    /// Windows user-work twin.
    #[test]
    fn a_host_deferred_api_object_needs_the_switch_and_a_user_work_twin() {
        assert_eq!(twin_defapi_plan(false, false), TwinDefapiPlan::NoHostObject);
        assert_eq!(twin_defapi_plan(false, true), TwinDefapiPlan::NoHostObject);
        assert_eq!(twin_defapi_plan(true, false), TwinDefapiPlan::NoHostObject);
        assert_eq!(
            twin_defapi_plan(true, true),
            TwinDefapiPlan::AuthorHostObject
        );
    }
}

/// ★ 2026-10-08 (after run84) — the async preempt's ordering and the disable list's resolution, with
/// fake host completions (no GPU): a model drainer that holds replies until their acts settle,
/// releases them in order, then posts queued preempt completions through [`preempt_posts_ready`] —
/// the same pass order as `Device::drainer_loop` (`release_settled`, then `deliver_preempt_complete`).
#[cfg(test)]
mod preempt_order_tests {
    use super::{DisableList, preempt_posts_ready, resolve_disable_list};

    /// One control on the model GSP: its reply waits for its act (`settled`).
    struct Held {
        name: &'static str,
        settled: bool,
    }

    #[derive(Default)]
    struct Drainer {
        held: Vec<Held>,
        /// Completions the act thread queued (as `ChanPlane::preempt_done`).
        done: Vec<&'static str>,
        /// What the guest's message queue receives, in order.
        wire: Vec<String>,
    }

    impl Drainer {
        fn request(&mut self, name: &'static str) {
            self.held.push(Held {
                name,
                settled: false,
            });
        }
        /// The act ran on the host: the completion is queued BEFORE the act's reply settles (the
        /// window run84 hit).
        fn act_queues(&mut self, ev: &'static str) {
            self.done.push(ev);
        }
        fn act_settles(&mut self, name: &'static str) {
            for h in &mut self.held {
                if h.name == name {
                    h.settled = true;
                }
            }
        }
        /// One drainer pass: release settled replies in order (a held one blocks those behind it),
        /// then post completions only when the gate allows.
        fn pass(&mut self) {
            while self.held.first().is_some_and(|h| h.settled) {
                let h = self.held.remove(0);
                self.wire.push(format!("reply {}", h.name));
            }
            if preempt_posts_ready(self.held.len()) {
                for ev in self.done.drain(..) {
                    self.wire.push(format!("event {ev}"));
                }
            }
        }
    }

    #[test]
    fn the_event_follows_its_reply_even_when_the_act_queues_it_first() {
        let mut d = Drainer::default();
        d.request("disable A");
        d.act_queues("A");
        d.pass(); // run84's window: the act queued, its reply not settled yet
        assert!(
            d.wire.is_empty(),
            "nothing may reach the guest before the reply"
        );
        d.act_settles("disable A");
        d.pass();
        assert_eq!(d.wire, ["reply disable A", "event A"]);
    }

    #[test]
    fn an_enable_sent_before_the_disable_completed_never_puts_the_event_before_the_disable_reply() {
        let mut d = Drainer::default();
        d.request("disable A");
        d.request("enable A");
        d.act_queues("A");
        d.pass();
        d.act_settles("enable A"); // out of order: still blocked behind the disable's reply
        d.pass();
        assert!(d.wire.is_empty());
        d.act_settles("disable A");
        d.pass();
        let pos = |s: &str| d.wire.iter().position(|w| w == s).unwrap();
        assert!(pos("reply disable A") < pos("event A"));
        assert!(pos("reply disable A") < pos("reply enable A"));
        assert_eq!(d.wire.len(), 3);
    }

    #[test]
    fn two_preempts_each_post_after_their_replies_in_order() {
        let mut d = Drainer::default();
        d.request("disable A");
        d.act_queues("A");
        d.act_settles("disable A");
        d.pass();
        d.request("disable B");
        d.act_queues("B");
        d.pass();
        d.act_settles("disable B");
        d.pass();
        assert_eq!(
            d.wire,
            ["reply disable A", "event A", "reply disable B", "event B"]
        );
    }

    #[test]
    fn no_completion_without_a_host_act() {
        let mut d = Drainer::default();
        d.request("disable A");
        d.act_settles("disable A"); // the act failed: nothing queued
        d.pass();
        assert_eq!(d.wire, ["reply disable A"]);
        assert!(preempt_posts_ready(0));
        assert!(!preempt_posts_ready(1));
    }

    /// This VM's plane: client 0x31 has twin 0x40 under group 0xe0 (token 0x13) and client 0x32 a
    /// Translated ring 0x41 (host 0x80c). Nothing else exists here.
    fn resolve(list: &[(u32, u32)]) -> DisableList<u32> {
        let twins = [((0x31u32, 0x40u32), Some(0xe0u32), 0x13u32)];
        resolve_disable_list(
            list,
            |c, h| twins.iter().find(|t| t.0 == (c, h)).map(|t| t.2),
            |c, h| {
                twins
                    .iter()
                    .filter(|t| t.0.0 == c && t.1 == Some(h))
                    .map(|t| (t.0, t.2))
                    .collect()
            },
            |c, h| ((c, h) == (0x32, 0x41)).then_some(0x80c),
        )
    }

    #[test]
    fn a_group_handle_resolves_to_its_twins_and_an_unknown_one_is_named() {
        let r = resolve(&[(0x31, 0xe0)]);
        assert_eq!(r.pt, [((0x31, 0x40), 0x13)]);
        assert!(r.tr.is_empty() && r.unknown.is_empty());
        // ENABLE of a group this plane never saw (freed, or never born): unknown, refused whole.
        let r = resolve(&[(0x31, 0xe0), (0x31, 0xe1)]);
        assert_eq!(r.unknown, [(0x31, 0xe1)]);
        assert_eq!(r.pt.len(), 1);
    }

    #[test]
    fn another_vms_handles_never_resolve_here() {
        // Another VM's guest RM hands out the same handle shapes; this plane's maps hold only its own
        // twins, so a foreign (client, group) pair is unknown — never a twin of this VM.
        let r = resolve(&[(0x33, 0xe0)]);
        assert!(r.pt.is_empty() && r.tr.is_empty());
        assert_eq!(r.unknown, [(0x33, 0xe0)]);
        // The same group handle under a different client is not this client's group.
        let r = resolve(&[(0x32, 0xe0)]);
        assert_eq!(r.unknown, [(0x32, 0xe0)]);
        // A Translated ring resolves by its own handle only.
        let r = resolve(&[(0x32, 0x41)]);
        assert_eq!(r.tr, [((0x32, 0x41), 0x80c)]);
    }
}

#[cfg(test)]
mod snap_tests {
    use super::{SNAP_METHODS, snap_decode};
    use kf_abi::submit::{method_header_inc, method_header_non_inc};

    #[test]
    fn the_stall_snapshot_decodes_host_semaphores_and_is_bounded() {
        // SEM_ADDR_LO/HI, PAYLOAD_LO/HI, EXECUTE(ACQ_CIRC_GEQ, 64-bit) on subchannel 3 (host methods
        // apply on every subchannel), then a 32-bit RELEASE of the same address
        let mut w = vec![
            method_header_inc(3, 0x5c, 5).unwrap(),
            0x1234_5678,
            0x12,
            7,
            1,
            3 | (1 << 24),
        ];
        w.extend([method_header_non_inc(0, 0x64, 1).unwrap(), 9]);
        w.extend([method_header_inc(0, 0x6c, 1).unwrap(), 1]);
        let d = snap_decode(&w, SNAP_METHODS);
        assert_eq!(d.sems.len(), 2);
        assert_eq!(
            (
                d.sems[0].op,
                d.sems[0].va,
                d.sems[0].payload,
                d.sems[0].wide
            ),
            ("ACQ_CIRC_GEQ", 0x12_1234_5678, (1 << 32) | 7, true)
        );
        assert_eq!(
            (
                d.sems[1].op,
                d.sems[1].va,
                d.sems[1].payload,
                d.sems[1].wide
            ),
            ("RELEASE", 0x12_1234_5678, 9, false)
        );
        assert_eq!(d.methods.len(), 7);
        assert_eq!(d.methods[0], (3, 0x5c, 0x1234_5678));
        assert!(d.stopped_at.is_none());
        // bounded: never more than `max` methods listed, whatever the segment holds
        let many: Vec<u32> = std::iter::once(method_header_non_inc(0, 0x100, 4000).unwrap())
            .chain(std::iter::repeat_n(0, 4000))
            .collect();
        assert_eq!(snap_decode(&many, 5).methods.len(), 5);
    }

    /// ★ Review 4 item 1 — a missing row hands the ctx range over, it is not "nothing there".
    #[test]
    fn a_missing_steer_row_still_hands_the_ctx_range_over() {
        assert_eq!(
            crate::chan::steer_len(Some(0x20_0000), 0x1_0000),
            0x20_0000,
            "the whole row when one was removed"
        );
        assert_eq!(
            crate::chan::steer_len(None, 0x1_0000),
            0x1_0000,
            "the ctx range when the row is gone"
        );
    }
}
