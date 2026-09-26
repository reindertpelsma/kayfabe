//! ★★★★★ **The Translated channel's runner** — the guest's rewritten work on OUR host channel.
//!
//! [`HostRing`] is one host channel we own, in a given VA space: a circular pushbuffer, a GPFIFO,
//! a USERD, and a fence word. Every batch ends in NVIDIA's own completion tail — a host semaphore
//! release **with `RELEASE_WFI`** of a sequence number, then `NON_STALL_INTERRUPT`
//! (`nvidia-push.c:1047-1059`) — so the event fd wakes the worker and the FENCE says what finished.
//! ⊘ A wake is never a verdict: the non-stall notifiers are GPU-wide (`intr.c:1195-1205`).
//!
//! [`TranslatedChannel::pump`] is what the worker calls on a doorbell or a wake. It never waits:
//! work that must follow a completion (a walk at a `MEM_OP` split) is SUSPENDED behind a fence and
//! resumed by a later pump, when the fence shows it. `[measured w826 gate 1]` NSI wake p50 127 µs.

use crate::ring::{GuestMemory, Next, RingRefusal, TranslatedRing};
use crate::translated::{IsCeClass, Window};
use kf_abi::submit::{ENGINE_TYPE_COPY0, USERD_GP_GET, USERD_GP_PUT, fifo, gp_entry, method_header_inc};
use kf_linux_raw::HostOffset as At;
use std::collections::VecDeque;

/// The bytes one ring occupies in its VA space (pushbuffer + GPFIFO + fence + USERD).
pub const RING_BYTES: u64 = 1 << 20;
/// ★ P6b: every address a ring hands the host engine — pushbuffer segments in GP entries, the
/// GPFIFO, the fence — must be below 2^40 on EVERY family: `GP_ENTRY0_GET 31:2` +
/// `GP_ENTRY1_GET_HI 7:0` in each family's channel class (`ogkm-580 clc46f.h:268-270` Turing,
/// `clc56f.h:270-272` Ampere/Ada, `clc86f.h:173-176` Hopper, `clc96f.h:95-98` / `clca6f.h:56-59`
/// Blackwell), and the fence's `SEM_ADDR_HI` is 8 bits.
pub const RING_VA_LIMIT: u64 = 1 << 40;
const PB_BYTES: u64 = 0xE_0000;
const GPFIFO_OFF: u64 = 0xF_0000;
const GPFIFO_ENTRIES: u32 = 512;
const FENCE_OFF: u64 = 0xF_8000;
const USERD_OFF: u64 = 0xF_C000;
/// Room every ordinary push leaves for one completion tail ([`fence_words`] is 8 words).
const TAIL_BYTES: u64 = 64;
/// GP entries we allow in flight — below the ring size, so a put never laps an unfinished get.
const MAX_IN_FLIGHT: usize = (GPFIFO_ENTRIES as usize) - 16;

/// NVIDIA's completion tail: a host release of `payload` at `fence_va` **with `RELEASE_WFI`**
/// (ordered behind the engine going idle), then the host `NON_STALL_INTERRUPT`.
#[must_use]
pub fn fence_words(fence_va: u64, payload: u32) -> Option<Vec<u32>> {
    Some(vec![
        method_header_inc(0, fifo::SEM_ADDR_LO, 5)?,
        (fence_va & 0xFFFF_FFFC) as u32,
        ((fence_va >> 32) & 0xFF) as u32,
        payload,
        0,
        fifo::SEM_EXECUTE_RELEASE_32BIT | fifo::SEM_EXECUTE_RELEASE_WFI_EN,
        method_header_inc(0, fifo::NON_STALL_INTERRUPT, 1)?,
        0,
    ])
}

/// `a` has reached `b` in wrapping sequence order.
#[must_use]
pub const fn reached(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) >= 0
}

/// The host ring could not take this submission now: retry after a completion frees space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Busy;

/// One live pushbuffer region: `[start, start+len)`, freed when fence `seq` completes.
#[derive(Debug, Clone, Copy)]
struct Region {
    start: u64,
    len: u64,
    seq: u32,
}

/// The host resources one [`HostRing`] holds besides its channel.
#[derive(Debug, Clone, Copy)]
struct RingOwned {
    mem: u32,
    cookie: u64,
    space: kf_host::VaSpace,
}

/// ★ A copy-engine host channel we own. Its completions reach the worker through the SESSION's one
/// completion fd ([`crate::completions::Completions`]), never an fd of its own.
pub struct HostRing {
    /// `None` once [`HostRing::release`] has unmapped it.
    cpu: Option<kf_linux_raw::VolatileRegion>,
    _node: kf_linux_raw::CharDevice,
    /// ★ v3-appfix J: what the ring OWNS on the host, so [`HostRing::release`] can give it back —
    /// the device-local object, its CPU view's release cookie, and the space it is mapped in.
    /// ⊘ Before this the ring kept none of them: every retired Translated channel left a 1 MiB
    /// object, its GPU mapping and its host BAR1 CPU view behind (`[measured v3-appfix g5]`
    /// ~4.5 MiB of host BAR1 per guest CUDA process with persistence mode, the 256 MiB aperture
    /// exhausted at ~55 processes: `NV_ESC_RM_MAP_MEMORY … NoMemory`, V3_APP_MATRIX §3 J).
    owned: Option<RingOwned>,
    va: u64,
    chan: kf_host::Channel,
    head: u64,
    put: u32,
    seq: u32,
    live: VecDeque<Region>,
}

impl HostRing {
    /// Build the ring in `space` on host COPY0 (RM places it; its VA must be below 2^40).
    ///
    /// # Errors
    /// Any step's refusal, by name.
    pub fn new(rm: &kf_host::HostRm, space: kf_host::VaSpace) -> Result<HostRing, String> {
        Self::on_engine(rm, space, ENGINE_TYPE_COPY0)
    }

    /// ★ Build the ring on host copy engine `engine` (an `NV2080_ENGINE_TYPE_COPYn` the CALLER
    /// authored — for a guest's CE pushbuffer, an ASYNC copy engine: see `HostRm::ce_is_grce`).
    ///
    /// # Errors
    /// Any step's refusal, by name.
    pub fn on_engine(rm: &kf_host::HostRm, space: kf_host::VaSpace, engine: u32) -> Result<HostRing, String> {
        Self::on_engine_at(rm, space, engine, None)
    }

    /// ★ P6b: [`HostRing::on_engine`] at a VA the CALLER chose (`at`, FIXED), or RM's choice for
    /// `None`. ⊘ In a space that mirrors a GUEST's VA space the caller must choose: RM's
    /// bottom-up choice is the same allocator the guest's own RM uses, so it lands on guest VAs
    /// (`[measured p6b1]` token 3's ring at `0x121040000`, where the guest's UVM mapped tokens
    /// 4-6's GPFIFOs next) — a VMM address inside the guest's VA space.
    ///
    /// # Errors
    /// Any step's refusal, by name; `at` not below [`RING_VA_LIMIT`] - [`RING_BYTES`].
    pub fn on_engine_at(rm: &kf_host::HostRm, space: kf_host::VaSpace, engine: u32, at: Option<u64>) -> Result<HostRing, String> {
        if let Some(a) = at
            && a.checked_add(RING_BYTES).is_none_or(|e| e > RING_VA_LIMIT)
        {
            return Err(format!("ring VA {a:#x}+{RING_BYTES:#x} is not below 2^40 (GP entry GET_HI 7:0)"));
        }
        let mem = rm.alloc_device_local(RING_BYTES).map_err(|e| format!("ring obj: {e:?}"))?;
        let va = match rm.map(space, mem, kf_host::MapBacking::Dedicated, 0, RING_BYTES, at, false) {
            Ok(va) => va,
            Err(e) => {
                let _ = rm.free(mem);
                return Err(format!("map ring{}: {e:?}", at.map(|a| format!(" at {a:#x}")).unwrap_or_default()));
            }
        };
        // ★ Every failure below gives back what was built (object, mapping, view) — a refused
        // birth must not leak the host aperture any more than a retired one may.
        let undo = |cookie: Option<u64>| {
            if let Some(c) = cookie {
                let _ = rm.release_cpu_view(kf_host::CpuViewRelease { h_memory: mem, p_linear_address: c });
            }
            let _ = rm.unmap(space, va, false);
            let _ = rm.free(mem);
        };
        // The CPU view is armed with its release COOKIE kept (`HostRm::map_cpu` drops it, which
        // makes a view unreleasable for the life of the process).
        let (node, cookie) = match rm.arm_cpu_view(kf_host::MapNode::Gpu, mem, 0, RING_BYTES, kf_host::ViewAccess::ReadWrite) {
            Ok(v) => v,
            Err(e) => {
                undo(None);
                return Err(format!("cpu ring: {e:?}"));
            }
        };
        let cpu = match kf_linux_raw::VolatileRegion::map(
            kf_linux_raw::Backing::DeviceFile { fd: node.as_fd() },
            RING_BYTES,
            kf_linux_raw::CachePolicy::Uncached,
            kf_linux_raw::HostPageSize::query(),
        ) {
            Ok(r) => r,
            Err(e) => {
                undo(Some(cookie));
                return Err(format!("cpu ring mmap: {e:?}"));
            }
        };
        let owned = RingOwned { mem, cookie, space };
        let chan = match rm.birth_channel(space, engine, kf_host::RingSpec {
            gp_fifo_va: va + GPFIFO_OFF,
            gp_fifo_entries: GPFIFO_ENTRIES,
            userd_memory: mem,
            userd_offset: USERD_OFF,
            err_notifier: 0,
        }) {
            Ok(c) => c,
            Err(e) => {
                drop(cpu);
                undo(Some(cookie));
                return Err(format!("birth: {e:?}"));
            }
        };
        let mut ring = HostRing { cpu: Some(cpu), _node: node, owned: Some(owned), va, chan, head: 0, put: 0, seq: 0, live: VecDeque::new() };
        let tail = rm
            .alloc_ce_object(chan, engine)
            .map_err(|e| format!("ce object: {e:?}"))
            .and_then(|_| rm.schedule(chan).map_err(|e| format!("schedule: {e:?}")))
            .and_then(|()| ring.cpu()?.store_u32(At::new(FENCE_OFF), 0).map_err(|e| format!("{e:?}")));
        if let Err(e) = tail {
            let _ = rm.free_channel(chan);
            ring.release(rm);
            return Err(e);
        }
        Ok(ring)
    }

    fn cpu(&self) -> Result<&kf_linux_raw::VolatileRegion, String> {
        self.cpu.as_ref().ok_or_else(|| "host ring already released (no CPU mapping)".to_string())
    }

    /// ★ v3-appfix J: give back what the ring owns besides its channel — its CPU view (host BAR1
    /// aperture), its GPU mapping and its device-local object. Call it AFTER the channel is freed
    /// (the channel's GPFIFO and USERD live in the object). Idempotent; returns what it released.
    ///
    /// ⊘ Order: the CPU mapping is removed FIRST (never left over BAR1 pages RM is about to
    /// reassign), then `NV_ESC_RM_UNMAP_MEMORY`, then the GPU unmap, then the free. A ring that
    /// is used after this refuses every store/load by name (no mapping).
    pub fn release(&mut self, rm: &kf_host::HostRm) -> Option<String> {
        let o = self.owned.take()?;
        // Drop the CPU mapping (munmap) before the view's aperture is given back.
        self.cpu = None;
        let view = rm.release_cpu_view(kf_host::CpuViewRelease { h_memory: o.mem, p_linear_address: o.cookie });
        let unmap = rm.unmap(o.space, self.va, false);
        let free = rm.free(o.mem);
        Some(format!(
            "ring {:#x} released: cpu unmapped, view {} unmap {} free {}",
            self.va,
            if view.is_ok() { "ok" } else { "REFUSED" },
            if unmap.is_ok() { "ok" } else { "REFUSED" },
            if free.is_ok() { "ok" } else { "REFUSED" }
        ))
    }

    /// ★ v3-initrace (diagnostic): our ring's USERD `GP_GET` (what the host engine fetched),
    /// `GP_PUT` (what we published) and the fence word — `None` once released.
    pub fn cursors(&self) -> Option<(u32, u32, u32)> {
        let c = self.cpu.as_ref()?;
        let get = c.load_u32(At::new(USERD_OFF + USERD_GP_GET)).ok()?;
        let put = c.load_u32(At::new(USERD_OFF + USERD_GP_PUT)).ok()?;
        let fence = c.load_u32(At::new(FENCE_OFF)).ok()?;
        Some((get, put, fence))
    }

    /// Nothing pushed is still unfinished (as of the last [`HostRing::completed`]).
    #[must_use]
    pub fn idle(&self) -> bool {
        self.live.is_empty()
    }

    /// The host VA RM placed the ring at (its GPFIFO is at `va + 0xF_0000`).
    #[must_use]
    pub fn va(&self) -> u64 {
        self.va
    }

    /// The host channel.
    #[must_use]
    pub fn channel(&self) -> kf_host::Channel {
        self.chan
    }

    /// The last fence sequence the engine released.
    ///
    /// # Errors
    /// A failed load.
    pub fn completed(&mut self) -> Result<u32, String> {
        let done = self.cpu()?.load_u32(At::new(FENCE_OFF)).map_err(|e| format!("{e:?}"))?;
        while self.live.front().is_some_and(|r| reached(done, r.seq)) {
            self.live.pop_front();
        }
        Ok(done)
    }

    /// Copy `words` into the pushbuffer and queue one GP entry for them (no doorbell yet).
    ///
    /// # Errors
    /// `Ok(Err(Busy))` when there is no free space until a completion; `Err` for a store failure.
    pub fn push(&mut self, words: &[u32]) -> Result<Result<(), Busy>, String> {
        self.push_inner(words, TAIL_BYTES)
    }

    /// `push`, requiring `reserve` bytes of room beyond the words — so ordinary work always leaves
    /// space for the completion tail that will free it (without it a full pushbuffer with no tail
    /// queued never completes, and never frees).
    fn push_inner(&mut self, words: &[u32], reserve: u64) -> Result<Result<(), Busy>, String> {
        let n = 4 * words.len() as u64;
        if n == 0 {
            return Ok(Ok(()));
        }
        if n > PB_BYTES / 2 {
            return Err(format!("segment of {n} bytes exceeds half the host pushbuffer"));
        }
        if self.live.len() + usize::from(reserve > 0) >= MAX_IN_FLIGHT {
            return Ok(Err(Busy));
        }
        let need = n + reserve;
        let start = if self.head + need <= PB_BYTES { self.head } else { 0 };
        let overlaps = self.live.iter().any(|r| start < r.start + r.len && r.start < start + need);
        if overlaps {
            return Ok(Err(Busy));
        }
        for (i, w) in words.iter().enumerate() {
            self.cpu()?.store_u32(At::new(start + 4 * i as u64), *w).map_err(|e| format!("{e:?}"))?;
        }
        let entry = gp_entry(self.va + start, n).ok_or("gp entry (ring VA above 2^40?)")?;
        let gp = GPFIFO_OFF + u64::from(self.put % GPFIFO_ENTRIES) * 8;
        self.cpu()?.store_u32(At::new(gp), entry as u32).map_err(|e| format!("{e:?}"))?;
        self.cpu()?.store_u32(At::new(gp + 4), (entry >> 32) as u32).map_err(|e| format!("{e:?}"))?;
        self.put = self.put.wrapping_add(1);
        self.head = start + n;
        // Covered by the NEXT fence.
        self.live.push_back(Region { start, len: n, seq: self.seq.wrapping_add(1) });
        Ok(Ok(()))
    }

    /// Queue the completion tail for everything pushed so far and ring the doorbell. Returns the
    /// fence sequence that proves it all complete.
    ///
    /// # Errors
    /// `Ok(Err(Busy))` when the tail itself has no room; `Err` for a store/doorbell failure.
    pub fn fence(&mut self, rm: &kf_host::HostRm) -> Result<Result<u32, Busy>, String> {
        let seq = self.seq.wrapping_add(1);
        let words = fence_words(self.va + FENCE_OFF, seq).ok_or("fence encode")?;
        if let Err(b) = self.push_inner(&words, 0)? {
            return Ok(Err(b));
        }
        self.seq = seq;
        kf_linux_raw::release_fence();
        self.cpu()?
            .store_u32(At::new(USERD_OFF + USERD_GP_PUT), self.put % GPFIFO_ENTRIES)
            .map_err(|e| format!("{e:?}"))?;
        kf_linux_raw::release_fence();
        rm.doorbell(self.chan.token).map_err(|e| format!("doorbell: {e:?}"))?;
        Ok(Ok(seq))
    }
}

/// `NV2080_NOTIFIERS_FIFO_EVENT_MTHD` — the host NSI method's edge.
pub const FIFO_EVENT_MTHD: u32 = 35;

/// The guest's USERD for this channel: its produce cursor and the consume cursor we author.
pub trait GuestUserd {
    /// The guest's current `GP_PUT`.
    ///
    /// # Errors
    /// A failed read.
    fn gp_put(&mut self) -> Result<u32, String>;
    /// Author the guest's `GP_GET`.
    ///
    /// # Errors
    /// A failed write.
    fn set_gp_get(&mut self, gp_get: u32) -> Result<(), String>;
}

/// ★★★★★ v3-initrace — **a Translated channel's USERD, as physical RM initialises it.**
///
/// The guest's CPU-RM clears a channel's USERD only when it is in SYSMEM (or on full SR-IOV):
/// *"Clear Userd if it is in FB for SRIOV environment … or if in SYSMEM"*
/// (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:2344-2356`, the memset is
/// `kfifoSetupUserD_GM107`, `kernel_fifo_gm107.c:797-808`: `NV_RAMUSERD_CHAN_SIZE` bytes of zero).
/// An FB-resident USERD is the PHYSICAL RM's to initialise at allocation — host RM does exactly
/// that to a passthrough twin's USERD (`rm_takes_a_guest_userd_and_zeroes_it`). For a Translated
/// channel the physical RM is us, and until v3-initrace nothing did it.
///
/// ⊘⊘⊘ **What that cost, measured.** RM's allocator hands a new channel the same FB USERD slot an
/// earlier channel used (every re-open of `/dev/nvidia0` puts the new CeUtils channel's USERD where
/// the last one's was, cursors `GP_PUT = GP_GET = 2` still in it). The ring cursor starts at 0, so
/// the first pump — rung by `GPFIFO_SCHEDULE`, before the guest has submitted anything — read the
/// stale `GP_PUT` as queued work, fetched the guest's still-zero GP entries as NOPs, fenced them and
/// authored `GP_GET` to the stale value. With a stale `GP_PUT = 1` the guest's real entry 0 then
/// arrives with `GP_PUT = 1` = the cursor, so it is **never fetched**: `memmgrMemSet` times out,
/// `RmInitAdapter failed (0x25:0x65)` — the adapter-init flake of `V3_DRIVER_MATRIX.md` §6, whose
/// evidence (`forwarded=1 serves=3 last_put=1 GP_GET=1`, one host non-stall) this reproduces exactly
/// (`KF3_INJECT_STALE_USERD=1`). A stale value ≥ 2 only "worked" by fetching the whole ring round.
pub trait UserdInit {
    /// Store one little-endian word `off` bytes from the channel's USERD base.
    ///
    /// # Errors
    /// A failed store.
    fn store_u32(&mut self, off: u64, v: u32) -> Result<(), String>;
}

/// ★ Zero the channel's USERD — `min(declared, NV_RAMUSERD_CHAN_SIZE)` bytes, whole words — as
/// physical RM does before the allocation's reply (so the guest can never have written a cursor
/// yet). Returns the bytes zeroed.
///
/// # Errors
/// The first failed store, by name.
pub fn zero_userd(u: &mut dyn UserdInit, declared: u64) -> Result<u64, String> {
    let len = declared.min(kf_abi::submit::USERD_SIZE) & !3;
    for off in (0..len).step_by(4) {
        u.store_u32(off, 0).map_err(|e| format!("USERD +{off:#x}: {e}"))?;
    }
    Ok(len)
}

/// Where a `MEM_OP` split's walk stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Split {
    /// Walked and published: the work after the split may run.
    Done,
    /// Requested (or still running) on the thread that owns the walker. The channel stays
    /// SUSPENDED; the walker's completion rings the channel's token and the next pump asks again.
    Pending,
}

/// Walk the named root and publish its mappings (the reconcile) — the `MEM_OP` split's work.
///
/// ⊘ Never a wait: a publisher whose walk runs elsewhere (the VA-manager thread) answers
/// [`Split::Pending`] and is asked again on a later pump, never blocked on.
pub trait Publisher {
    /// Walk `pdb` (`None` = every space) and publish — or report that it is under way.
    ///
    /// # Errors
    /// A failed walk or a refused publish.
    fn invalidated(&mut self, pdb: Option<u64>) -> Result<Split, String>;
}

/// Why a Translated channel stopped. It is dead after any of these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChanError {
    /// The guest's ring was refused.
    Ring(RingRefusal),
    /// Our host ring failed.
    Host(String),
    /// The guest's USERD could not be read or written.
    Userd(String),
    /// The walk/publish at a split failed.
    Publish(String),
}

/// What a pump did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pumped {
    /// Everything up to the guest's `GP_PUT` is submitted (or already retired).
    Caught,
    /// Waiting for a completion: a split's walk, or host ring space. Pump again on the next wake.
    Waiting,
}

/// ★ v3-initrace (diagnostic, `KF3_COMPLETION_PROBE`): one host fence of a Translated channel —
/// the guest semaphore releases its work asked for, and when it was submitted / seen complete.
#[derive(Debug, Clone)]
pub struct ProbeFence {
    /// Our fence sequence.
    pub seq: u32,
    /// The guest `GP_GET` it retires, if it ends a guest entry.
    pub gp_get: Option<u32>,
    /// The releases the rewritten segments asked for (the words were forwarded unchanged).
    pub releases: Vec<crate::translated::Release>,
    /// The launches with a physical operand, as the guest wrote them.
    pub launches: Vec<crate::translated::PhysLaunch>,
    /// When the host doorbell for it was rung.
    pub submitted: std::time::Instant,
    /// When a pump first read the fence as reached.
    pub completed: Option<std::time::Instant>,
}

/// Completed fences kept for the probe between takes (older ones are dropped).
const PROBE_KEEP: usize = 16;

/// ★ One guest kernel channel executed on the real engine through our host ring.
pub struct TranslatedChannel {
    ring: TranslatedRing,
    host: HostRing,
    token: u32,
    retire: VecDeque<(u32, u32)>,
    suspended: Option<(u32, Option<u64>, Option<u32>)>,
    stash: Option<Next>,
    walks: u64,
    submissions: u64,
    last_gp_get: Option<u32>,
    /// ★ v3-initrace: `Some` only while the completion probe is on.
    probe: Option<Probe>,
}

/// The probe's per-channel record: fences in flight, and completed ones not yet taken.
#[derive(Debug, Default)]
struct Probe {
    inflight: VecDeque<ProbeFence>,
    done: Vec<ProbeFence>,
}

impl Probe {
    fn submitted(
        &mut self,
        seq: u32,
        gp_get: Option<u32>,
        releases: Vec<crate::translated::Release>,
        launches: Vec<crate::translated::PhysLaunch>,
    ) {
        self.inflight.push_back(ProbeFence { seq, gp_get, releases, launches, submitted: std::time::Instant::now(), completed: None });
    }
    fn reached(&mut self, done: u32) {
        while self.inflight.front().is_some_and(|f| reached(done, f.seq)) {
            if let Some(mut f) = self.inflight.pop_front() {
                f.completed = Some(std::time::Instant::now());
                if self.done.len() >= PROBE_KEEP {
                    self.done.remove(0);
                }
                self.done.push(f);
            }
        }
    }
}

impl TranslatedChannel {
    /// A Translated channel over the guest ring `ring`, executing on `host`, known to the doorbell
    /// plane as `token`.
    #[must_use]
    pub fn new(ring: TranslatedRing, host: HostRing, token: u32) -> TranslatedChannel {
        TranslatedChannel {
            ring,
            host,
            token,
            retire: VecDeque::new(),
            suspended: None,
            stash: None,
            walks: 0,
            submissions: 0,
            last_gp_get: None,
            probe: None,
        }
    }

    /// ★ v3-initrace: record every fence's guest releases for the completion probe (diagnostic;
    /// off by default — the device turns it on with `KF3_COMPLETION_PROBE`).
    pub fn set_probe(&mut self, on: bool) {
        self.probe = on.then(Probe::default);
    }

    /// ★ v3-initrace: the fences seen complete since the last call (oldest first), each with the
    /// guest releases its work asked for. Empty while the probe is off.
    pub fn take_completed(&mut self) -> Vec<ProbeFence> {
        self.probe.as_mut().map(|p| std::mem::take(&mut p.done)).unwrap_or_default()
    }

    /// ★ v3-initrace: fences still in flight (the probe's "submitted, never seen complete").
    #[must_use]
    pub fn probe_inflight(&self) -> Vec<ProbeFence> {
        self.probe.as_ref().map(|p| p.inflight.iter().cloned().collect()).unwrap_or_default()
    }

    /// ★ v3-initrace: our host ring's own `GP_GET`/`GP_PUT` (USERD) and the fence word — the
    /// host side of the ordering the probe prints.
    pub fn host_cursors(&mut self) -> Option<(u32, u32, u32)> {
        self.host.cursors()
    }

    /// The host ring (its event fd goes in the worker's epoll).
    #[must_use]
    pub fn host(&self) -> &HostRing {
        &self.host
    }

    /// ★ v3-appfix J: release the host ring's object, mapping and CPU view — AFTER the host
    /// channel is freed. See [`HostRing::release`].
    pub fn release_host(&mut self, rm: &kf_host::HostRm) -> Option<String> {
        self.host.release(rm)
    }

    /// `(guest GP entries fetched, host submissions, walks at splits)`.
    #[must_use]
    pub fn counts(&self) -> (u64, u64, u64) {
        (self.ring.entries_fetched(), self.submissions, self.walks)
    }

    /// The last `GP_GET` authored to the guest.
    #[must_use]
    pub fn last_gp_get(&self) -> Option<u32> {
        self.last_gp_get
    }

    fn author(&mut self, userd: &mut dyn GuestUserd, g: u32) -> Result<(), ChanError> {
        userd.set_gp_get(g).map_err(ChanError::Userd)?;
        self.last_gp_get = Some(g);
        Ok(())
    }

    /// ★ Advance as far as possible without waiting. Call on the doorbell and on every wake.
    ///
    /// # Errors
    /// [`ChanError`]; the channel must not be pumped again.
    #[allow(clippy::too_many_arguments)]
    pub fn pump(
        &mut self,
        rm: &kf_host::HostRm,
        done_edge: &crate::completions::Completions,
        mem: &mut dyn GuestMemory,
        userd: &mut dyn GuestUserd,
        publisher: &mut dyn Publisher,
        is_ce: IsCeClass,
        w: &dyn Window,
    ) -> Result<Pumped, ChanError> {
        let done = self.host.completed().map_err(ChanError::Host)?;
        if let Some(p) = self.probe.as_mut() {
            p.reached(done);
        }
        let mut newest = None;
        while self.retire.front().is_some_and(|&(s, _)| reached(done, s)) {
            newest = self.retire.pop_front().map(|(_, g)| g);
        }
        if let Some(g) = newest {
            self.author(userd, g)?;
        }
        if self.host.idle() && self.retire.is_empty() && self.suspended.is_none() {
            // Nothing of ours is in flight: completions need not ring us. Only the BUSY owner
            // clears, so this cannot race a mark by another thread.
            done_edge.clear(self.token);
        }
        if let Some((seq, pdb, retires)) = self.suspended {
            if !reached(done, seq) {
                return Ok(Pumped::Waiting);
            }
            if publisher.invalidated(pdb).map_err(ChanError::Publish)? == Split::Pending {
                // ★ The walk runs on the VA thread; its completion rings this token again.
                return Ok(Pumped::Waiting);
            }
            self.walks += 1;
            self.suspended = None;
            if let Some(g) = retires {
                self.author(userd, g)?;
            }
        }
        let gp_put = userd.gp_put().map_err(ChanError::Userd)?;
        let mut pushed = false;
        let mut last_retire = None;
        let outcome = loop {
            let next = match self.stash.take() {
                Some(n) => n,
                None => self.ring.next(gp_put, mem, is_ce, w).map_err(ChanError::Ring)?,
            };
            match next {
                Next::Idle => break Pumped::Caught,
                Next::Submit { ref words, retires } => {
                    if self.host.push(words).map_err(ChanError::Host)?.is_err() {
                        self.stash = Some(next);
                        break Pumped::Waiting;
                    }
                    if !words.is_empty() {
                        pushed = true;
                        self.submissions += 1;
                    }
                    if retires.is_some() {
                        last_retire = retires;
                    }
                }
                Next::Walk { pdb, retires } => {
                    // Everything before the split must COMPLETE before the walk: the guest may
                    // have written the very page tables it is invalidating with that work.
                    done_edge.mark(self.token); // BEFORE the doorbell — see `completions`
                    match self.host.fence(rm).map_err(ChanError::Host)? {
                        Ok(seq) => {
                            let g0 = last_retire.take();
                            if let Some(g) = g0 {
                                self.retire.push_back((seq, g));
                            }
                            if let Some(p) = self.probe.as_mut() {
                                p.submitted(seq, g0, self.ring.take_releases(), self.ring.take_launches());
                            }
                            self.suspended = Some((seq, pdb, retires));
                            return Ok(Pumped::Waiting);
                        }
                        Err(Busy) => {
                            self.stash = Some(Next::Walk { pdb, retires });
                            break Pumped::Waiting;
                        }
                    }
                }
            }
        };
        if pushed || last_retire.is_some() {
            done_edge.mark(self.token); // BEFORE the doorbell — see `completions`
            match self.host.fence(rm).map_err(ChanError::Host)? {
                Ok(seq) => {
                    if let Some(g) = last_retire {
                        self.retire.push_back((seq, g));
                    }
                    if let Some(p) = self.probe.as_mut() {
                        p.submitted(seq, last_retire, self.ring.take_releases(), self.ring.take_launches());
                    }
                }
                // No room for the tail: the regions already live carry fences of their own
                // only if an earlier tail was queued — so this is a real stall, named.
                Err(Busy) => return Err(ChanError::Host("no room for a completion tail".into())),
            }
        }
        Ok(outcome)
    }
}
