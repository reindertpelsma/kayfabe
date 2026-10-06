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
use kf_abi::submit::{ENGINE_TYPE_COPY0, ENGINE_TYPE_GRAPHICS, USERD_GP_GET, USERD_GP_PUT};
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
const GPFIFO_ENTRIES: u32 = 512;

/// ★ P1+P2 inc D (`docs/design/V3_P1P2_TSPACE.md` §2.4) — **where a ring's regions sit in its one
/// 1 MiB device-local object, and how each is mapped on the GPU.** One object and one CPU view in
/// both layouts (a separate object per region would add host BAR1 views per ring — the v3-appfix J
/// exhaustion).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RingLayout {
    /// The pushbuffer: `[0, pb_bytes)`.
    pub pb_bytes: u64,
    /// The GPFIFO's offset.
    pub gpfifo_off: u64,
    /// The fence word's offset.
    pub fence_off: u64,
    /// USERD's offset.
    pub userd_off: u64,
    /// The T-space layout: three GPU maps, pushbuffer and GPFIFO READ-ONLY, the fence read-write,
    /// USERD in NO GPU map (PBDMA reaches it through the instance block; we write `GP_PUT` through
    /// our CPU view). Else today's one read-write map of the whole object.
    pub tspace: bool,
}

/// Today's layout (one read-write GPU map of the whole object).
pub const LEGACY_LAYOUT: RingLayout = RingLayout {
    pb_bytes: 0xE_0000,
    gpfifo_off: 0xF_0000,
    fence_off: 0xF_8000,
    userd_off: 0xF_C000,
    tspace: false,
};

/// ★ The T-space layout: every region whole 64 KiB granules, so each of the three maps is
/// big-page-congruent (`kf_abi::bringup::nvos46_page_size_flag`): pushbuffer `[0, 0xD_0000)`,
/// GPFIFO `[0xD_0000, 0xE_0000)` (512 entries use 4 KiB of it), fence `[0xE_0000, 0xF_0000)`, USERD
/// `[0xF_0000, 1 MiB)`.
pub const TSPACE_LAYOUT: RingLayout = RingLayout {
    pb_bytes: 0xD_0000,
    gpfifo_off: 0xD_0000,
    fence_off: 0xE_0000,
    userd_off: 0xF_0000,
    tspace: true,
};

impl RingLayout {
    /// The GPU maps of the object: `(offset, length, permission)`.
    #[must_use]
    pub fn maps(&self) -> Vec<(u64, u64, kf_host::MapPerm)> {
        let ro = kf_host::MapPerm {
            read_only: true,
            ..kf_host::MapPerm::READ_WRITE
        };
        if self.tspace {
            vec![
                (0, self.pb_bytes, ro),
                (self.gpfifo_off, self.fence_off - self.gpfifo_off, ro),
                (
                    self.fence_off,
                    self.userd_off - self.fence_off,
                    kf_host::MapPerm::READ_WRITE,
                ),
            ]
        } else {
            vec![(0, RING_BYTES, kf_host::MapPerm::READ_WRITE)]
        }
    }
}
/// ★ Unmap every map in `maps` — all of them, whatever any one answers — and report the first
/// refusal (review fix 2026-10-04: a short-circuit left the later maps live).
///
/// # Errors
/// The first refusal.
pub fn unmap_every<E>(maps: &[u64], mut unmap: impl FnMut(u64) -> Result<(), E>) -> Result<(), E> {
    let mut first = Ok(());
    for &m in maps {
        if let Err(e) = unmap(m)
            && first.is_ok()
        {
            first = Err(e);
        }
    }
    first
}

/// Room every ordinary push leaves for one completion tail ([`fence_words`] is 8 words).
const TAIL_BYTES: u64 = 64;
/// GP entries we allow in flight — below the ring size, so a put never laps an unfinished get.
const MAX_IN_FLIGHT: usize = (GPFIFO_ENTRIES as usize) - 16;

/// NVIDIA's completion tail — ★ P1+P2 inc C: authored in the address perimeter
/// ([`crate::tspace_unsafe::fence_words`], `V3_P1P2_TSPACE.md` §3.8), re-exported here.
pub use crate::tspace_unsafe::fence_words;

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
#[derive(Debug, Clone)]
struct RingOwned {
    mem: u32,
    cookie: u64,
    space: kf_host::VaSpace,
    /// ★ P1+P2 inc D: every GPU map of the object (one, or the T-space layout's three).
    maps: Vec<u64>,
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
    /// ★ P1+P2 inc D: where the regions sit and how they are mapped.
    layout: RingLayout,
    chan: kf_host::Channel,
    // A graphics-runlist ring owns a real host GR context and routes all of its
    // authored CE work through the channel header's CE subchannel.
    gr_context: Option<(u32, u32)>,
    nvdec_context: Option<(u32, u32, u32)>,
    ce_class: u32,
    head: u64,
    put: u32,
    seq: u32,
    live: VecDeque<Region>,
}

// Only normalized incrementing streams enter a graphics-runlist host ring.
// Caller bounds the segment to half its owned PB before reaching this helper.
fn route_graphics_ce(words: &[u32], ce_class: u32) -> Result<Vec<u32>, String> {
    use kf_abi::submit::{MethodForm, method_header_decode, method_header_inc};
    let sub = kf_abi::generated::classes::NVA06F_SUBCHANNEL_COPY_ENGINE;
    let mut out = words.to_vec();
    let mut at = 0usize;
    while at < words.len() {
        let h = method_header_decode(words[at]).ok_or("GR CE route: invalid header")?;
        if h.form != MethodForm::Incrementing {
            return Err("GR CE route: segment is not normalized incrementing methods".into());
        }
        let end = at
            .checked_add(1)
            .and_then(|a| a.checked_add(h.arg_words))
            .filter(|&e| e <= words.len())
            .ok_or("GR CE route: truncated arguments")?;
        if h.method == kf_abi::submit::SET_OBJECT && h.arg_words != 0 {
            if h.arg_words != 1 || !kf_chip::is_any_dma_copy_class(words[at + 1]) {
                return Err("GR CE route: SET_OBJECT is not one admitted CE class".into());
            }
            // The guest's compatible CE class selects OUR allocated host CE
            // object. It does not name a guest/host handle or allocate an engine.
            out[at + 1] = ce_class;
        }
        out[at] = method_header_inc(sub, h.method, h.arg_words as u32)
            .ok_or("GR CE route: header cannot be authored")?;
        at = end;
    }
    Ok(out)
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
    pub fn on_engine(
        rm: &kf_host::HostRm,
        space: kf_host::VaSpace,
        engine: u32,
    ) -> Result<HostRing, String> {
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
    pub fn on_engine_at(
        rm: &kf_host::HostRm,
        space: kf_host::VaSpace,
        engine: u32,
        at: Option<u64>,
    ) -> Result<HostRing, String> {
        Self::on_engine_layout(rm, space, engine, at, LEGACY_LAYOUT)
    }

    /// ★ P1+P2 inc D: [`HostRing::on_engine_at`] with an explicit [`RingLayout`]. The T-space layout
    /// needs a FIXED `at` (its three maps are placed region by region) and reads every placement
    /// back: a map RM placed elsewhere is refused by name.
    ///
    /// # Errors
    /// Any step's refusal, by name.
    pub fn on_engine_layout(
        rm: &kf_host::HostRm,
        space: kf_host::VaSpace,
        engine: u32,
        at: Option<u64>,
        layout: RingLayout,
    ) -> Result<HostRing, String> {
        if let Some(a) = at
            && a.checked_add(RING_BYTES).is_none_or(|e| e > RING_VA_LIMIT)
        {
            return Err(format!(
                "ring VA {a:#x}+{RING_BYTES:#x} is not below 2^40 (GP entry GET_HI 7:0)"
            ));
        }
        if layout.tspace && at.is_none() {
            return Err("a T-space ring needs a FIXED VA (its ring slot)".into());
        }
        let mem = rm
            .alloc_device_local(RING_BYTES)
            .map_err(|e| format!("ring obj: {e:?}"))?;
        let mut maps: Vec<u64> = Vec::new();
        let unmap_all = |maps: &[u64]| {
            for &m in maps {
                let _ = rm.unmap(space, m, false);
            }
        };
        let mut va = 0u64;
        for (off, len, perm) in layout.maps() {
            let want = at.map(|a| a + off);
            let got = if layout.tspace {
                rm.map_kind(
                    space,
                    mem,
                    kf_host::MapBacking::Dedicated,
                    off,
                    len,
                    want,
                    false,
                    0,
                    perm,
                )
            } else {
                rm.map(
                    space,
                    mem,
                    kf_host::MapBacking::Dedicated,
                    0,
                    len,
                    at,
                    false,
                )
            };
            match got {
                Ok(v) if want.is_none_or(|w| w == v) => {
                    maps.push(v);
                    if off == 0 {
                        va = v;
                    }
                }
                other => {
                    if let Ok(v) = other {
                        maps.push(v);
                    }
                    unmap_all(&maps);
                    let _ = rm.free(mem);
                    let at = at.map(|a| format!(" at {a:#x}")).unwrap_or_default();
                    return Err(match other {
                        Err(e) if !layout.tspace => format!("map ring{at}: {e:?}"),
                        other => format!("map ring{at} region +{off:#x}+{len:#x}: {other:x?}"),
                    });
                }
            }
        }
        // ★ Every failure below gives back what was built (object, mapping, view) — a refused
        // birth must not leak the host aperture any more than a retired one may.
        let undo = |cookie: Option<u64>| {
            if let Some(c) = cookie {
                let _ = rm.release_cpu_view(kf_host::CpuViewRelease {
                    h_memory: mem,
                    p_linear_address: c,
                });
            }
            unmap_all(&maps);
            let _ = rm.free(mem);
        };
        // The CPU view is armed with its release COOKIE kept (`HostRm::map_cpu` drops it, which
        // makes a view unreleasable for the life of the process).
        let (node, cookie) = match rm.arm_cpu_view(
            kf_host::MapNode::Gpu,
            mem,
            0,
            RING_BYTES,
            kf_host::ViewAccess::ReadWrite,
        ) {
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
        let owned = RingOwned {
            mem,
            cookie,
            space,
            maps: maps.clone(),
        };
        let chan = match rm.birth_channel(
            space,
            engine,
            kf_host::RingSpec {
                gp_fifo_va: va + layout.gpfifo_off,
                gp_fifo_entries: GPFIFO_ENTRIES,
                userd_memory: mem,
                userd_offset: layout.userd_off,
                err_notifier: 0,
            },
        ) {
            Ok(c) => c,
            Err(e) => {
                drop(cpu);
                undo(Some(cookie));
                return Err(format!("birth: {e:?}"));
            }
        };
        let mut ring = HostRing {
            cpu: Some(cpu),
            _node: node,
            owned: Some(owned),
            va,
            layout,
            chan,
            gr_context: None,
            nvdec_context: None,
            ce_class: rm.ce_class_id(),
            head: 0,
            put: 0,
            seq: 0,
            live: VecDeque::new(),
        };
        let tail = (|| {
            if let Some(index) = kf_abi::submit::nvdec_index_of_engine_type(engine) {
                let (arch, imp, _) = rm.arch_info();
                let family = kf_chip::Family::from_arch(arch, imp)
                    .map_err(|e| format!("NVDEC family: {e:?}"))?;
                let class = family
                    .classes()
                    .video_decoder
                    .iter()
                    .rev()
                    .copied()
                    .find(|c| rm.supported_class_ids().contains(c))
                    .ok_or("no source-derived NVDEC class in the actual host class list")?;
                let object = rm
                    .alloc_video_object(chan, class, index)
                    .map_err(|e| format!("owned NVDEC context: {e:?}"))?;
                ring.nvdec_context = Some((object, class, engine));
            } else {
                let ce_engine = if engine == ENGINE_TYPE_GRAPHICS {
                    (0..20)
                        .filter_map(crate::passthrough::copy_engine_type)
                        .find(|&e| rm.ce_is_grce(e) == Ok(true))
                        .ok_or("no host graphics copy engine for a graphics-runlist ring")?
                } else {
                    engine
                };
                rm.alloc_ce_object(chan, ce_engine)
                    .map_err(|e| format!("ce object: {e:?}"))?;
                if engine == ENGINE_TYPE_GRAPHICS {
                    ring.gr_context = Some(
                        rm.alloc_compute_object(chan)
                            .map_err(|e| format!("owned GR context: {e:?}"))?,
                    );
                }
            }
            rm.schedule(chan).map_err(|e| format!("schedule: {e:?}"))?;
            ring.cpu()?
                .store_u32(At::new(layout.fence_off), 0)
                .map_err(|e| format!("{e:?}"))
        })();
        if let Err(e) = tail {
            let _ = rm.free_channel(chan);
            ring.release(rm);
            return Err(e);
        }
        Ok(ring)
    }

    fn cpu(&self) -> Result<&kf_linux_raw::VolatileRegion, String> {
        self.cpu
            .as_ref()
            .ok_or_else(|| "host ring already released (no CPU mapping)".to_string())
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
        let view = rm.release_cpu_view(kf_host::CpuViewRelease {
            h_memory: o.mem,
            p_linear_address: o.cookie,
        });
        // ★ Review fix 2026-10-04: EVERY map is unmapped (the T-space layout has three); the first
        // refusal is reported — a refused pushbuffer unmap must not leave the GPFIFO and fence
        // maps live.
        let unmap = unmap_every(&o.maps, |m| rm.unmap(o.space, m, false));
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
        let get = c
            .load_u32(At::new(self.layout.userd_off + USERD_GP_GET))
            .ok()?;
        let put = c
            .load_u32(At::new(self.layout.userd_off + USERD_GP_PUT))
            .ok()?;
        let fence = c.load_u32(At::new(self.layout.fence_off)).ok()?;
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

    /// The owned compute object that constructed this ring's real GR context.
    #[must_use]
    pub fn gr_context(&self) -> Option<(u32, u32)> {
        self.gr_context
    }

    /// Real owned decoder object and engine whose constructor promoted a Falcon context.
    #[must_use]
    pub fn nvdec_context(&self) -> Option<(u32, u32, u32)> {
        self.nvdec_context
    }

    /// Whether this ring already owns a real context on the specified engine.
    #[must_use]
    pub fn owns_context(&self, engine: u32) -> bool {
        (engine == ENGINE_TYPE_GRAPHICS && self.gr_context.is_some())
            || self.nvdec_context.is_some_and(|(_, _, e)| e == engine)
    }

    /// The last fence sequence the engine released.
    ///
    /// # Errors
    /// A failed load.
    pub fn completed(&mut self) -> Result<u32, String> {
        let done = self
            .cpu()?
            .load_u32(At::new(self.layout.fence_off))
            .map_err(|e| format!("{e:?}"))?;
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
        // No codec or CE execution is admitted on the experimental decoder ring.
        // Its private fence remains authored here, never supplied by the guest.
        if self.nvdec_context.is_some() && !words.is_empty() {
            return Err("NVDEC ring: codec/CE submission is not implemented".into());
        }
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
        if n > self.layout.pb_bytes / 2 {
            return Err(format!(
                "segment of {n} bytes exceeds half the host pushbuffer"
            ));
        }
        // T-mode/Translated output is normalized incrementing methods, never a
        // guest GPFIFO. On a GR runlist CE routes through its dedicated subchannel.
        // Validate the entire segment before any store, preserving every datum.
        let routed = if self.gr_context.is_some() {
            Some(route_graphics_ce(words, self.ce_class)?)
        } else {
            None
        };
        let words = routed.as_deref().unwrap_or(words);
        if self.live.len() + usize::from(reserve > 0) >= MAX_IN_FLIGHT {
            return Ok(Err(Busy));
        }
        let need = n + reserve;
        let start = if self.head + need <= self.layout.pb_bytes {
            self.head
        } else {
            0
        };
        let overlaps = self
            .live
            .iter()
            .any(|r| start < r.start + r.len && r.start < start + need);
        if overlaps {
            return Ok(Err(Busy));
        }
        for (i, w) in words.iter().enumerate() {
            self.cpu()?
                .store_u32(At::new(start + 4 * i as u64), *w)
                .map_err(|e| format!("{e:?}"))?;
        }
        // ★ P1+P2 inc C: the GP entry is authored in the address perimeter, bounded to the
        // pushbuffer and below 2^40 (`crate::tspace_unsafe::ring_gp_entry`).
        let entry = crate::tspace_unsafe::ring_gp_entry(self.va, self.layout.pb_bytes, start, n)
            .ok_or("gp entry (outside the pushbuffer, or the ring VA above 2^40)")?;
        let gp = self.layout.gpfifo_off + u64::from(self.put % GPFIFO_ENTRIES) * 8;
        self.cpu()?
            .store_u32(At::new(gp), entry as u32)
            .map_err(|e| format!("{e:?}"))?;
        self.cpu()?
            .store_u32(At::new(gp + 4), (entry >> 32) as u32)
            .map_err(|e| format!("{e:?}"))?;
        self.put = self.put.wrapping_add(1);
        self.head = start + n;
        // Covered by the NEXT fence.
        self.live.push_back(Region {
            start,
            len: n,
            seq: self.seq.wrapping_add(1),
        });
        Ok(Ok(()))
    }

    /// Queue the completion tail for everything pushed so far and ring the doorbell. Returns the
    /// fence sequence that proves it all complete.
    ///
    /// # Errors
    /// `Ok(Err(Busy))` when the tail itself has no room; `Err` for a store/doorbell failure.
    pub fn fence(&mut self, rm: &kf_host::HostRm) -> Result<Result<u32, Busy>, String> {
        let seq = self.seq.wrapping_add(1);
        let words = fence_words(self.va + self.layout.fence_off, seq).ok_or("fence encode")?;
        if let Err(b) = self.push_inner(&words, 0)? {
            return Ok(Err(b));
        }
        self.seq = seq;
        kf_linux_raw::release_fence();
        self.cpu()?
            .store_u32(
                At::new(self.layout.userd_off + USERD_GP_PUT),
                self.put % GPFIFO_ENTRIES,
            )
            .map_err(|e| format!("{e:?}"))?;
        kf_linux_raw::release_fence();
        rm.doorbell(self.chan.token)
            .map_err(|e| format!("doorbell: {e:?}"))?;
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
        u.store_u32(off, 0)
            .map_err(|e| format!("USERD +{off:#x}: {e}"))?;
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
    /// ★ P1+P2 inc D: T-mode refused an item at bind, by name.
    Bind(crate::translated::Refusal),
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
    /// ★ P1+P2 inc D: the T-space windows, `Some` in T-mode (the ring is then
    /// [`TranslatedRing::new_tmode`]).
    tspace: Option<crate::tspace_unsafe::TWindows>,
    /// ★ P1+P2 inc D (§7.13): the stale-bind counter.
    stale: crate::tmode::StaleBind,
    /// The resolutions bound since the last fence (moved into `stale` at the fence).
    bound: Vec<crate::tmode::Bound>,
    /// ★ Review fix 2026-10-04 (§3.5): a host acquire was pushed since the last fence.
    acquire_unfenced: bool,
    /// The fence that covers the latest pushed host acquire.
    acquire_seq: Option<u32>,
    /// ★ The stashed items wait on a walk pending on the space (`Rows::walk_pending`), or on an
    /// acquire's fence; `Some` until they bind (or are refused).
    unresolved: Option<WaitOn>,
    /// Unresolved-operand waits, and refusals, counted.
    pub waits: UnresolvedCounts,
}

/// ★ Review fix 2026-10-04 (§3.5): what a stash with an unresolved operand waits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOn {
    /// A walk pending on the space (`tmode::Rows::walk_pending`) — the device rings the channel
    /// when the VA thread is idle again.
    Walk,
    /// A host acquire ahead of it, until its fence completes (our own completion wakes us).
    Acquire,
}

/// ★ Review fix 2026-10-04 (§3.5): what an operand with no placement row at bind waits on — a
/// walk pending on its space first, else an unfinished host acquire ahead of it — or `None`:
/// nothing can bring the row, so it is unmapped and refused by name.
#[must_use]
pub const fn wait_on(walk_pending: bool, acquire_outstanding: bool) -> Option<WaitOn> {
    if walk_pending {
        Some(WaitOn::Walk)
    } else if acquire_outstanding {
        Some(WaitOn::Acquire)
    } else {
        None
    }
}

/// ★ Unresolved-operand waits and refusals, per channel (the `TSPACE-RETIRE` line).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct UnresolvedCounts {
    /// Waits on a pending walk.
    pub walk: u64,
    /// Waits on an unfinished acquire.
    pub acquire: u64,
    /// Operands refused unmapped (nothing to wait on).
    pub refused: u64,
}

impl UnresolvedCounts {
    fn count(&mut self, on: WaitOn) {
        match on {
            WaitOn::Walk => self.walk += 1,
            WaitOn::Acquire => self.acquire += 1,
        }
    }

    /// `unresolved[walk=… acquire=… refused=…]`.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "unresolved[walk={} acquire={} refused={}]",
            self.walk, self.acquire, self.refused
        )
    }
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
        self.inflight.push_back(ProbeFence {
            seq,
            gp_get,
            releases,
            launches,
            submitted: std::time::Instant::now(),
            completed: None,
        });
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
            tspace: None,
            stale: crate::tmode::StaleBind::default(),
            bound: Vec::new(),
            acquire_unfenced: false,
            acquire_seq: None,
            unresolved: None,
            waits: UnresolvedCounts::default(),
        }
    }

    /// ★ P1+P2 inc D: run in T-mode against the T-space's windows — every item the ring hands out
    /// is bound at push ([`crate::tmode::push_bound`]); `negctl_stale` is the stale-bind counter's
    /// positive control (`KF3_NEGCTL_STALE_BIND`).
    pub fn set_tspace(&mut self, windows: crate::tspace_unsafe::TWindows, negctl_stale: bool) {
        self.tspace = Some(windows);
        self.stale.negctl = negctl_stale;
    }

    /// ★ P1+P2 inc D (§7.13), review fix 2026-10-04: the stale-bind counter and the
    /// unresolved-operand waits, for the `TSPACE-RETIRE` line —
    /// `stale_binds=stale/checked late=… indeterminate=… unresolved[walk= acquire= refused=]`.
    #[must_use]
    pub fn stale_line(&self) -> String {
        format!("{} {}", self.stale.line(), self.waits.line())
    }

    /// The stale-bind counter.
    #[must_use]
    pub fn stale_binds(&self) -> &crate::tmode::StaleBind {
        &self.stale
    }

    /// ★ The channel waits for a walk pending on its space to land before it binds its stash —
    /// the device rings it when the VA thread is idle again.
    #[must_use]
    pub fn waiting_on_walk(&self) -> bool {
        self.unresolved == Some(WaitOn::Walk)
    }

    /// ★ v3-initrace: record every fence's guest releases for the completion probe (diagnostic;
    /// off by default — the device turns it on with `KF3_COMPLETION_PROBE`).
    pub fn set_probe(&mut self, on: bool) {
        self.probe = on.then(Probe::default);
    }

    /// ★ P1+P2 inc A (`V3_P1P2_TSPACE.md` §3.6): count every header, method and GP entry this
    /// channel fetches ([`crate::census`]). Off by default.
    pub fn set_census(&mut self, on: bool) {
        self.ring.set_census(on);
    }

    /// ★ P1+P2 inc A (review fix 2026-10-04): refuse — not only count — what inc A refuses by name
    /// ([`crate::translated::CeState::strict`]); `extended_base`: the family defines
    /// `SET_PB_SEGMENT_EXTENDED_BASE` (Hopper+). See [`TranslatedRing::set_strict`] and
    /// [`TranslatedRing::set_extended_base`].
    pub fn set_inca(&mut self, strict: bool, extended_base: bool) {
        self.ring.set_strict(strict);
        self.ring.set_extended_base(extended_base);
    }

    /// ★ P1+P2 inc A: what the by-name refusals would have refused while not strict.
    #[must_use]
    pub fn inca(&self) -> crate::translated::IncACounts {
        self.ring.inca()
    }

    /// ★ P1+P2 inc C: run the T-mode shadow against `windows` ([`TranslatedRing::set_shadow`]).
    /// `negctl`: the shadow counters' positive control (`KF3_NEGCTL_SHADOW`,
    /// [`crate::tmode::Shadow::negctl_probe`]).
    pub fn set_shadow(&mut self, windows: Option<crate::tspace_unsafe::TWindows>, negctl: bool) {
        self.ring.set_shadow(windows, negctl);
    }

    /// The shadow's counters, when on — the device dumps them at free.
    #[must_use]
    pub fn shadow(&self) -> Option<&crate::tmode::Shadow> {
        self.ring.shadow()
    }

    /// The channel's census, when on — the device dumps it at free.
    #[must_use]
    pub fn census(&self) -> Option<&crate::census::Census> {
        self.ring.census()
    }

    /// ★ v3-initrace: the fences seen complete since the last call (oldest first), each with the
    /// guest releases its work asked for. Empty while the probe is off.
    pub fn take_completed(&mut self) -> Vec<ProbeFence> {
        self.probe
            .as_mut()
            .map(|p| std::mem::take(&mut p.done))
            .unwrap_or_default()
    }

    /// ★ v3-initrace: fences still in flight (the probe's "submitted, never seen complete").
    #[must_use]
    pub fn probe_inflight(&self) -> Vec<ProbeFence> {
        self.probe
            .as_ref()
            .map(|p| p.inflight.iter().cloned().collect())
            .unwrap_or_default()
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

    /// Fence `seq` now covers everything pushed since the last one: its resolutions go to the
    /// stale-bind counter, and an acquire among them is covered by it.
    fn fenced(&mut self, seq: u32) {
        self.stale.record(
            seq,
            std::mem::take(&mut self.bound),
            std::time::Instant::now(),
        );
        if std::mem::take(&mut self.acquire_unfenced) {
            self.acquire_seq = Some(seq);
        }
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
        if let Some(rows) = mem.rows() {
            // ★ P1+P2 inc D (§7.13): what the retired pieces were bound under, checked against
            // the rows' commit log; the rest were seen incomplete NOW.
            self.stale.observe(done, rows, std::time::Instant::now());
        }
        if self.acquire_seq.is_some_and(|s| reached(done, s)) {
            self.acquire_seq = None;
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
                None => self
                    .ring
                    .next(gp_put, mem, is_ce, w)
                    .map_err(ChanError::Ring)?,
            };
            match next {
                Next::Idle => break Pumped::Caught,
                Next::Bind { ir, retires } => {
                    // ★ P1+P2 inc D (§3.5): bound NOW — after any earlier split's walk — and a
                    // piece the ring cannot take yet is stashed UNBOUND, bound again at retry.
                    let (Some(win), Some(rows)) = (self.tspace, mem.rows()) else {
                        return Err(ChanError::Host(
                            "T-mode work without T-space windows or placement rows".into(),
                        ));
                    };
                    let host = &mut self.host;
                    let r = crate::tmode::push_bound(&ir, rows, &win, &mut self.bound, |w| {
                        host.push(w).map(|r| r.is_ok())
                    });
                    let (pieces, rest, unresolved) = match r {
                        Ok(crate::tmode::Pushed::All { pieces }) => (pieces, None, None),
                        Ok(crate::tmode::Pushed::Busy { rest, pieces }) => {
                            (pieces, Some(rest), None)
                        }
                        Ok(crate::tmode::Pushed::Unresolved { rest, pieces, why }) => {
                            (pieces, Some(rest), Some(why))
                        }
                        Err(crate::tmode::PushError::Refused(why)) => {
                            return Err(ChanError::Bind(why));
                        }
                        Err(crate::tmode::PushError::Host(e)) => return Err(ChanError::Host(e)),
                    };
                    let unpushed = rest.as_ref().map_or(0, Vec::len);
                    if ir[..ir.len() - unpushed]
                        .iter()
                        .any(crate::tmode::is_acquire)
                    {
                        self.acquire_unfenced = true;
                    }
                    if pieces > 0 {
                        pushed = true;
                        self.submissions += pieces as u64;
                    }
                    match (rest, unresolved) {
                        (Some(rest), Some(why)) => {
                            // ★ Review fix 2026-10-04 (§3.5): an operand with no row yet. WAIT
                            // while a walk is pending on the space, or while a host acquire ahead
                            // of it is unfinished (another channel's walk may precede the release
                            // it waits for); otherwise the operand is unmapped — refused by name.
                            let acquire = self.acquire_unfenced || self.acquire_seq.is_some();
                            let Some(on) = wait_on(rows.walk_pending(), acquire) else {
                                self.waits.refused += 1;
                                self.unresolved = None;
                                return Err(ChanError::Bind(why));
                            };
                            self.waits.count(on);
                            self.unresolved = Some(on);
                            self.stash = Some(Next::Bind { ir: rest, retires });
                            break Pumped::Waiting;
                        }
                        (Some(rest), None) => {
                            self.unresolved = None;
                            self.stash = Some(Next::Bind { ir: rest, retires });
                            break Pumped::Waiting;
                        }
                        _ => self.unresolved = None,
                    }
                    if retires.is_some() {
                        last_retire = retires;
                    }
                }
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
                            self.fenced(seq);
                            let g0 = last_retire.take();
                            if let Some(g) = g0 {
                                self.retire.push_back((seq, g));
                            }
                            if let Some(p) = self.probe.as_mut() {
                                p.submitted(
                                    seq,
                                    g0,
                                    self.ring.take_releases(),
                                    self.ring.take_launches(),
                                );
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
                    self.fenced(seq);
                    if let Some(g) = last_retire {
                        self.retire.push_back((seq, g));
                    }
                    if let Some(p) = self.probe.as_mut() {
                        p.submitted(
                            seq,
                            last_retire,
                            self.ring.take_releases(),
                            self.ring.take_launches(),
                        );
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

#[cfg(test)]
mod graphics_route_tests {
    use super::route_graphics_ce;
    use kf_abi::submit::{fifo, method_header_decode, method_header_inc};

    #[test]
    fn routing_changes_only_headers_including_native_completion_tail() {
        let mut words = vec![
            method_header_inc(0, 0, 1).unwrap(),
            0xc7b5,
            method_header_inc(2, 0x400, 2).unwrap(),
            0x20010000,
            0xdeadbeef,
        ];
        words.extend(super::fence_words(0x120000000, 0x1234).unwrap());
        let routed = route_graphics_ce(&words, 0xc9b5).unwrap();
        let sub = kf_abi::generated::classes::NVA06F_SUBCHANNEL_COPY_ENGINE;
        let mut at = 0;
        while at < words.len() {
            let before = method_header_decode(words[at]).unwrap();
            let after = method_header_decode(routed[at]).unwrap();
            assert_eq!(after.subchannel, sub);
            assert_eq!(after.method, before.method);
            assert_eq!(after.arg_words, before.arg_words);
            let end = at + 1 + before.arg_words;
            if before.method == 0 {
                assert_eq!(routed[at + 1], 0xc9b5);
            } else {
                assert_eq!(routed[at + 1..end], words[at + 1..end]);
            }
            at = end;
        }
        // Real RELEASE_WFI ordering and notification survive the routing.
        assert!(
            routed.contains(&(fifo::SEM_EXECUTE_RELEASE_32BIT | fifo::SEM_EXECUTE_RELEASE_WFI_EN))
        );
        assert_eq!(routed.len(), words.len());
    }

    #[test]
    fn unsupported_or_partial_stream_is_never_routed() {
        let valid = method_header_inc(0, 0x400, 2).unwrap();
        assert!(route_graphics_ce(&[valid, 1], 0xc9b5).is_err());
        assert!(route_graphics_ce(&[6 << 29], 0xc9b5).is_err());
        assert!(route_graphics_ce(&[3 << 29], 0xc9b5).is_err()); // immediate, not normalized
        assert_eq!(route_graphics_ce(&[], 0xc9b5).unwrap(), Vec::<u32>::new());
        assert!(route_graphics_ce(&[method_header_inc(0, 0, 1).unwrap(), 0xc797], 0xc9b5).is_err());
        assert!(
            route_graphics_ce(&[method_header_inc(0, 0, 2).unwrap(), 0xc7b5, 1], 0xc9b5).is_err()
        );
    }
}
