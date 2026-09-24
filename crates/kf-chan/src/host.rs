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
use kf_abi::submit::{ENGINE_TYPE_COPY0, USERD_GP_PUT, fifo, gp_entry, method_header_inc};
use kf_linux_raw::HostOffset as At;
use std::collections::VecDeque;

const RING_BYTES: u64 = 1 << 20;
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

/// ★ A copy-engine host channel we own. Its completions reach the worker through the SESSION's one
/// completion fd ([`crate::completions::Completions`]), never an fd of its own.
pub struct HostRing {
    cpu: kf_linux_raw::VolatileRegion,
    _node: kf_linux_raw::CharDevice,
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
        let mem = rm.alloc_device_local(RING_BYTES).map_err(|e| format!("ring obj: {e:?}"))?;
        let va = rm
            .map(space, mem, kf_host::MapBacking::Dedicated, 0, RING_BYTES, None, false)
            .map_err(|e| format!("map ring: {e:?}"))?;
        let (node, cpu) = rm
            .map_cpu(mem, RING_BYTES, kf_linux_raw::CachePolicy::Uncached)
            .map_err(|e| format!("cpu ring: {e:?}"))?;
        let chan = rm
            .birth_channel(space, engine, kf_host::RingSpec {
                gp_fifo_va: va + GPFIFO_OFF,
                gp_fifo_entries: GPFIFO_ENTRIES,
                userd_memory: mem,
                userd_offset: USERD_OFF,
                err_notifier: 0,
            })
            .map_err(|e| format!("birth: {e:?}"))?;
        rm.alloc_ce_object(chan, engine).map_err(|e| format!("ce object: {e:?}"))?;
        rm.schedule(chan).map_err(|e| format!("schedule: {e:?}"))?;
        cpu.store_u32(At::new(FENCE_OFF), 0).map_err(|e| format!("{e:?}"))?;
        Ok(HostRing { cpu, _node: node, va, chan, head: 0, put: 0, seq: 0, live: VecDeque::new() })
    }

    /// Nothing pushed is still unfinished (as of the last [`HostRing::completed`]).
    #[must_use]
    pub fn idle(&self) -> bool {
        self.live.is_empty()
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
        let done = self.cpu.load_u32(At::new(FENCE_OFF)).map_err(|e| format!("{e:?}"))?;
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
            self.cpu.store_u32(At::new(start + 4 * i as u64), *w).map_err(|e| format!("{e:?}"))?;
        }
        let entry = gp_entry(self.va + start, n).ok_or("gp entry (ring VA above 2^40?)")?;
        let gp = GPFIFO_OFF + u64::from(self.put % GPFIFO_ENTRIES) * 8;
        self.cpu.store_u32(At::new(gp), entry as u32).map_err(|e| format!("{e:?}"))?;
        self.cpu.store_u32(At::new(gp + 4), (entry >> 32) as u32).map_err(|e| format!("{e:?}"))?;
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
        self.cpu
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

/// Walk the named root and publish its mappings (the reconcile) — the `MEM_OP` split's work.
pub trait Publisher {
    /// Walk `pdb` (`None` = every space) and publish.
    ///
    /// # Errors
    /// A failed walk or a refused publish.
    fn invalidated(&mut self, pdb: Option<u64>) -> Result<(), String>;
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
        }
    }

    /// The host ring (its event fd goes in the worker's epoll).
    #[must_use]
    pub fn host(&self) -> &HostRing {
        &self.host
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
            publisher.invalidated(pdb).map_err(ChanError::Publish)?;
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
                            if let Some(g) = last_retire.take() {
                                self.retire.push_back((seq, g));
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
                }
                // No room for the tail: the regions already live carry fences of their own
                // only if an earlier tail was queued — so this is a real stall, named.
                Err(Busy) => return Err(ChanError::Host("no room for a completion tail".into())),
            }
        }
        Ok(outcome)
    }
}
