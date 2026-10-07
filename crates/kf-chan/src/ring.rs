//! ★★★★★ **The Translated channel's ring cursor** — GP entries in, rewritten work out.
//!
//! Pure and GPU-free: the guest's memory is read through [`GuestMemory`], and the output is a
//! sequence of [`Next`] steps the runner turns into host submissions. What this owns:
//!
//! - the guest ring's **fetch cursor** — the next GP index to read, always `< entries`;
//! - the CE register state that carries across segments ([`CeState`]);
//! - the remainder of a segment split at a `MEM_OP` invalidate — work AFTER the split must not
//!   run until the walk and reconcile have published the guest's new tables.
//!
//! ⊘ `GP_GET` is NOT owned here. It is the runner's to author, and only on COMPLETION: a guest
//! that sees `GP_GET` advance may reuse the GPFIFO slot and the pushbuffer behind it.

use crate::census::{Census, GpKind};
use crate::translated::{CeState, IsCeClass, Piece, Refusal, Release, Window, rewrite_counted};
use kf_abi::submit::{GP_ENTRY_SIZE, GpEntryKind, gp_entry_classify, gp_extended_base, gp_opcode};
use std::collections::VecDeque;

/// Largest pushbuffer segment we read for one GP entry. The hardware limit is 2^21 dwords
/// (8 MiB); RM's CeUtils segments are a few hundred bytes and UVM's a few KiB.
pub const MAX_SEGMENT_BYTES: u64 = 256 << 10;

/// Reads the guest's memory at a guest VA in the channel's VA space.
pub trait GuestMemory {
    /// Fill `out` from `va`.
    ///
    /// # Errors
    /// A range that is not mapped, or a failed read — refused by name by the caller.
    fn read(&mut self, va: u64, out: &mut [u8]) -> Result<(), String>;

    /// ★ P1+P2 inc C: the placement rows behind this memory, for the T-mode resolver and its shadow
    /// ([`crate::tmode::Rows`]). `None` (the default) when the reader has none.
    fn rows(&self) -> Option<&dyn crate::tmode::Rows> {
        None
    }
}

/// What the runner must do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Submit these method words on our host channel. `retires = Some(g)` when they finish a
    /// guest GP entry: once they COMPLETE, the guest's `GP_GET` may be authored as `g`.
    Submit {
        /// Rewritten method words.
        words: Vec<u32>,
        /// The guest `GP_GET` this submission completes, if it ends an entry.
        retires: Option<u32>,
    },
    /// The guest invalidated here. Before anything after it runs: wait for everything already
    /// submitted to COMPLETE (the guest may have written page tables with that very work), then
    /// walk `pdb` (`None` = every space) and publish. Then call [`TranslatedRing::next`] again.
    Walk {
        /// The named root, or `None` for `PDB_ALL`.
        pdb: Option<u64>,
        /// The guest `GP_GET` to author once the walk is done, if the split ended its entry.
        retires: Option<u32>,
    },
    /// Nothing to do: the cursor has reached the guest's `GP_PUT`.
    Idle,
    /// ★ P1+P2 inc D (T-mode, `V3_P1P2_TSPACE.md` §3.5): UNBOUND work — the runner binds each item
    /// against the placement rows when it pushes it ([`crate::tmode::push_bound`]).
    Bind {
        /// The items, in order (no split among them).
        ir: Vec<crate::tmode::Ir>,
        /// The guest `GP_GET` they complete, if they end an entry.
        retires: Option<u32>,
    },
}

/// Why the ring stopped. The channel is dead after any of these — refused by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RingRefusal {
    /// The guest's `GP_PUT` is outside its own ring.
    PutOutOfRange {
        /// The value read.
        gp_put: u32,
        /// The ring size.
        entries: u32,
    },
    /// A GP entry or segment could not be read.
    Read {
        /// The guest GP index.
        gp: u32,
        /// The guest VA.
        va: u64,
        /// Why.
        why: String,
    },
    /// A segment longer than [`MAX_SEGMENT_BYTES`].
    SegmentTooLong {
        /// The guest GP index.
        gp: u32,
        /// Its length.
        len: u64,
    },
    /// `GP_ENTRY1_SYNC_WAIT` — not implemented; refused rather than run without its ordering.
    SyncWait {
        /// The guest GP index.
        gp: u32,
    },
    /// ★ P1+P2 inc A (`V3_P1P2_TSPACE.md` §3.7): a control entry that is neither `NOP` nor — on a
    /// family that defines it (Hopper+) — `SET_PB_SEGMENT_EXTENDED_BASE`: `ILLEGAL`, `GP_CRC`,
    /// `PB_CRC`, or an opcode the family's class does not name. Refused by name on a strict ring;
    /// counted and skipped as a NOP (as before 2026-10-04) on the count-only default path.
    ControlEntry {
        /// The guest GP index.
        gp: u32,
        /// `GP_ENTRY1_OPCODE`.
        opcode: u32,
    },
    /// The rewriter refused the segment.
    Rewrite {
        /// The guest GP index.
        gp: u32,
        /// Why.
        why: Refusal,
    },
}

/// ★ DIAGNOSTIC (2026-10-07, Windows run30): the most words of a refused segment logged. A
/// refusal kills the channel ([`RingRefusal`] is terminal for the ring), so this runs at most once
/// per channel and prints at most this many guest words: bounded however the guest writes.
const REFUSED_SEGMENT_LOG_WORDS: usize = 32;

/// A rewrite refusal of the segment at GP index `gp`, logged with the segment's first
/// [`REFUSED_SEGMENT_LOG_WORDS`] words. Run30 showed Windows' first submissions on its kernel CE and
/// GR channels refused as `ForeignClass` (subchannel 5 "class 1", subchannel 2 class `0xa140`);
/// which methods the guest sent is the evidence an owner decision on kernel GR work needs.
fn refused_segment(gp: u32, va: u64, words: &[u32], why: Refusal) -> RingRefusal {
    let head: Vec<String> = words
        .iter()
        .take(REFUSED_SEGMENT_LOG_WORDS)
        .map(|w| format!("{w:08x}"))
        .collect();
    eprintln!(
        "kf3: ring REFUSED segment gp {gp} va {va:#x} ({} words) {why:?}; first words: {}",
        words.len(),
        head.join(" ")
    );
    RingRefusal::Rewrite { gp, why }
}

/// ★ One guest kernel channel's ring, as the Translated runner sees it.
#[derive(Debug)]
pub struct TranslatedRing {
    gpfifo_va: u64,
    entries: u32,
    cursor: u32,
    st: CeState,
    pending: VecDeque<Piece>,
    pending_retires: Option<u32>,
    entries_fetched: u64,
    /// ★ P1+P2 inc A (§3.7): address bits 56:40 of every later segment, from the guest's last
    /// `SET_PB_SEGMENT_EXTENDED_BASE` control entry (Hopper+ UVM writes one before a channel's
    /// first push, `ogkm-580: kernel-open/nvidia-uvm/uvm_channel.c:2536-2544`). Zero until then.
    pb_ext_base: u64,
    /// ★ P1+P2 inc A (§3.7, review fix 2026-10-04): the family defines
    /// `SET_PB_SEGMENT_EXTENDED_BASE` (Hopper and later, `clc86f.h:184-189`). Below Hopper opcode 4
    /// names nothing and is handled as every other unnamed control opcode. `false` until the
    /// device says otherwise ([`TranslatedRing::set_extended_base`]).
    ext_base_defined: bool,
    /// ★ P1+P2 inc A (§3.6): the count-only census, when on.
    census: Option<Box<Census>>,
    /// ★ P1+P2 inc C (§3.6): the T-mode shadow and the windows it binds against, when on.
    shadow: Option<Box<(crate::tmode::Shadow, crate::tspace_unsafe::TWindows)>>,
    /// ★ P1+P2 inc D: `Some` in T-mode — the state the T-mode decoder carries, and its unbound IR.
    tmode: Option<Box<(crate::tmode::TState, VecDeque<crate::tmode::Ir>)>>,
}

impl TranslatedRing {
    /// A ring of `entries` GP entries at guest VA `gpfifo_va`, fetched from index `start`.
    ///
    /// # Panics
    /// If `entries == 0` or `start >= entries` — a caller bug, never guest input.
    #[must_use]
    pub fn new(gpfifo_va: u64, entries: u32, start: u32) -> TranslatedRing {
        assert!(
            entries > 0 && start < entries,
            "ring of {entries} from {start}"
        );
        TranslatedRing {
            gpfifo_va,
            entries,
            cursor: start,
            st: CeState::default(),
            pending: VecDeque::new(),
            pending_retires: None,
            entries_fetched: 0,
            pb_ext_base: 0,
            ext_base_defined: false,
            census: None,
            shadow: None,
            tmode: None,
        }
    }

    /// ★ P1+P2 inc D (`V3_P1P2_TSPACE.md` §3): the same ring in T-MODE — every segment is decoded
    /// into unbound IR ([`crate::tmode::decode`]) and handed out as [`Next::Bind`], never rewritten
    /// in place; nothing the guest wrote is forwarded.
    ///
    /// # Panics
    /// As [`TranslatedRing::new`].
    #[must_use]
    pub fn new_tmode(gpfifo_va: u64, entries: u32, start: u32) -> TranslatedRing {
        let mut r = Self::new(gpfifo_va, entries, start);
        r.tmode = Some(Box::default());
        // T-mode refuses by name whatever the default path counts (§3.7).
        r.st.strict = true;
        r
    }

    /// ★ P1+P2 inc A (review fix 2026-10-04): refuse — not only count — what inc A refuses by name
    /// ([`CeState::strict`]). A T-mode ring is always strict.
    pub fn set_strict(&mut self, strict: bool) {
        self.st.strict = strict || self.tmode.is_some();
    }

    /// ★ P1+P2 inc A (review fix 2026-10-04): the family defines `SET_PB_SEGMENT_EXTENDED_BASE`
    /// (Hopper and later — `kf_chan::ttables::Tier::has_pb_extended_base` of the host CE class).
    pub fn set_extended_base(&mut self, defined: bool) {
        self.ext_base_defined = defined;
    }

    /// ★ P1+P2 inc A: what the by-name refusals would have refused while not strict.
    #[must_use]
    pub fn inca(&self) -> crate::translated::IncACounts {
        self.st.inca
    }

    /// The next T-mode step from the unbound IR, if any: a split, or the run of items before one.
    fn next_t(&mut self) -> Option<Next> {
        let (_, q) = self.tmode.as_deref_mut()?;
        let first = q.pop_front()?;
        if let crate::tmode::Ir::Invalidate { pdb } = first {
            let retires = if q.is_empty() {
                self.pending_retires.take()
            } else {
                None
            };
            return Some(Next::Walk { pdb, retires });
        }
        let mut ir = vec![first];
        while q
            .front()
            .is_some_and(|x| !matches!(x, crate::tmode::Ir::Invalidate { .. }))
        {
            if let Some(x) = q.pop_front() {
                ir.push(x);
            }
        }
        let retires = if q.is_empty() {
            self.pending_retires.take()
        } else {
            None
        };
        Some(Next::Bind { ir, retires })
    }

    /// ★ P1+P2 inc C (`V3_P1P2_TSPACE.md` §3.6): run the T-mode rewriter in SHADOW on every segment
    /// this ring fetches — decode and bind against the memory's placement rows and `windows`, the
    /// output discarded, the verdicts counted ([`crate::tmode::Shadow`]). `None` turns it off.
    pub fn set_shadow(&mut self, windows: Option<crate::tspace_unsafe::TWindows>, negctl: bool) {
        self.shadow = windows.map(|w| Box::new((crate::tmode::Shadow::with_negctl(negctl), w)));
    }

    /// The shadow's counters, when on.
    #[must_use]
    pub fn shadow(&self) -> Option<&crate::tmode::Shadow> {
        self.shadow.as_deref().map(|s| &s.0)
    }

    /// ★ P1+P2 inc A: count every header, method and GP entry this ring fetches
    /// ([`crate::census`]). Off by default.
    pub fn set_census(&mut self, on: bool) {
        self.census = on.then(Box::default);
    }

    /// The census, when on.
    #[must_use]
    pub fn census(&self) -> Option<&Census> {
        self.census.as_deref()
    }

    /// The address bits 56:40 the guest's last `SET_PB_SEGMENT_EXTENDED_BASE` set (0 before one).
    #[must_use]
    pub fn pb_extended_base(&self) -> u64 {
        self.pb_ext_base
    }

    /// GP entries fetched so far (the per-channel `forwarded=` count).
    #[must_use]
    pub fn entries_fetched(&self) -> u64 {
        self.entries_fetched
    }

    /// ★ v3-initrace (diagnostic): the semaphore releases the segments rewritten since the last
    /// call asked for, oldest first (see [`crate::translated::Releases`]).
    pub fn take_releases(&mut self) -> Vec<Release> {
        self.st.releases.take()
    }

    /// ★ v3-initrace (diagnostic): the launches with a physical operand since the last call.
    pub fn take_launches(&mut self) -> Vec<crate::translated::PhysLaunch> {
        self.st.launches.take()
    }

    /// The next step, given the guest's current `GP_PUT`.
    ///
    /// # Errors
    /// [`RingRefusal`], by name; the ring must not be pumped again.
    pub fn next(
        &mut self,
        gp_put: u32,
        mem: &mut dyn GuestMemory,
        is_ce: IsCeClass,
        w: &dyn Window,
    ) -> Result<Next, RingRefusal> {
        if gp_put >= self.entries {
            return Err(RingRefusal::PutOutOfRange {
                gp_put,
                entries: self.entries,
            });
        }
        loop {
            if let Some(n) = self.next_t() {
                return Ok(n);
            }
            if let Some(p) = self.pending.pop_front() {
                let retires = if self.pending.is_empty() {
                    self.pending_retires.take()
                } else {
                    None
                };
                return Ok(match p {
                    Piece::Words(words) => Next::Submit { words, retires },
                    Piece::Invalidate { pdb } => Next::Walk { pdb, retires },
                });
            }
            if let Some(g) = self.pending_retires.take() {
                // A segment that rewrote to nothing (e.g. only MEM_OP A-C): still retires.
                return Ok(Next::Submit {
                    words: Vec::new(),
                    retires: Some(g),
                });
            }
            if self.cursor == gp_put {
                return Ok(Next::Idle);
            }
            let gp = self.cursor;
            self.cursor = (self.cursor + 1) % self.entries;
            self.entries_fetched += 1;
            let at = self.gpfifo_va + u64::from(gp) * GP_ENTRY_SIZE;
            let mut raw = [0u8; 8];
            mem.read(at, &mut raw)
                .map_err(|why| RingRefusal::Read { gp, va: at, why })?;
            self.pending_retires = Some(self.cursor);
            let mut e = match gp_entry_classify(u64::from_le_bytes(raw)) {
                GpEntryKind::Segment(e) => {
                    if let Some(c) = self.census.as_deref_mut() {
                        c.gp(GpKind::Segment);
                    }
                    e
                }
                GpEntryKind::Control { opcode, operand } => {
                    if let Some(c) = self.census.as_deref_mut() {
                        c.gp(GpKind::Control(opcode));
                    }
                    match opcode {
                        // Nothing to run; still retires.
                        gp_opcode::NOP => continue,
                        gp_opcode::SET_PB_SEGMENT_EXTENDED_BASE if self.ext_base_defined => {
                            self.pb_ext_base = gp_extended_base(operand);
                            continue;
                        }
                        // ★ Count-only (not strict): skipped as a NOP, as before inc A.
                        _ if !self.st.strict => {
                            self.st.inca.control_entries += 1;
                            continue;
                        }
                        _ => return Err(RingRefusal::ControlEntry { gp, opcode }),
                    }
                }
            };
            // ★ §3.7: a segment entry carries address bits 39:0; 56:40 are the channel's
            // extended base (zero before Hopper, and before the guest set one). Applied when
            // strict; on the count-only default path counted, and read at 39:0 as before inc A.
            if self.st.strict {
                e.gpu_va |= self.pb_ext_base;
            } else if self.pb_ext_base != 0 {
                self.st.inca.ext_base_unapplied += 1;
            }
            if e.sync_wait {
                return Err(RingRefusal::SyncWait { gp });
            }
            if e.len_bytes > MAX_SEGMENT_BYTES {
                return Err(RingRefusal::SegmentTooLong {
                    gp,
                    len: e.len_bytes,
                });
            }
            let mut bytes = vec![0u8; e.len_bytes as usize];
            mem.read(e.gpu_va, &mut bytes)
                .map_err(|why| RingRefusal::Read {
                    gp,
                    va: e.gpu_va,
                    why,
                })?;
            let words: Vec<u32> = bytes
                .chunks_exact(4)
                .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            if let Some((st, q)) = self.tmode.as_deref_mut() {
                // ★ T-mode: decoded to unbound IR, bound by the runner at push (§3.5).
                let ir = crate::tmode::decode(&words, is_ce, st, self.census.as_deref_mut())
                    .map_err(|why| refused_segment(gp, e.gpu_va, &words, why))?;
                q.extend(ir);
                continue;
            }
            if let (Some(sh), Some(rows)) = (self.shadow.as_deref_mut(), mem.rows()) {
                // ★ Before today's rewrite, on the same words: what T-mode would do here.
                sh.0.observe(&words, is_ce, rows, &sh.1);
            }
            let pieces =
                rewrite_counted(&words, is_ce, &mut self.st, w, self.census.as_deref_mut())
                    .map_err(|why| refused_segment(gp, e.gpu_va, &words, why))?;
            self.pending.extend(pieces);
        }
    }
}
