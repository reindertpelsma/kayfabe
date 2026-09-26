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

use crate::translated::{CeState, IsCeClass, Piece, Refusal, Release, Window, rewrite};
use kf_abi::submit::{GP_ENTRY_SIZE, gp_entry_decode};
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
    /// The rewriter refused the segment.
    Rewrite {
        /// The guest GP index.
        gp: u32,
        /// Why.
        why: Refusal,
    },
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
}

impl TranslatedRing {
    /// A ring of `entries` GP entries at guest VA `gpfifo_va`, fetched from index `start`.
    ///
    /// # Panics
    /// If `entries == 0` or `start >= entries` — a caller bug, never guest input.
    #[must_use]
    pub fn new(gpfifo_va: u64, entries: u32, start: u32) -> TranslatedRing {
        assert!(entries > 0 && start < entries, "ring of {entries} from {start}");
        TranslatedRing {
            gpfifo_va,
            entries,
            cursor: start,
            st: CeState::default(),
            pending: VecDeque::new(),
            pending_retires: None,
            entries_fetched: 0,
        }
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
            return Err(RingRefusal::PutOutOfRange { gp_put, entries: self.entries });
        }
        loop {
            if let Some(p) = self.pending.pop_front() {
                let retires = if self.pending.is_empty() { self.pending_retires.take() } else { None };
                return Ok(match p {
                    Piece::Words(words) => Next::Submit { words, retires },
                    Piece::Invalidate { pdb } => Next::Walk { pdb, retires },
                });
            }
            if let Some(g) = self.pending_retires.take() {
                // A segment that rewrote to nothing (e.g. only MEM_OP A-C): still retires.
                return Ok(Next::Submit { words: Vec::new(), retires: Some(g) });
            }
            if self.cursor == gp_put {
                return Ok(Next::Idle);
            }
            let gp = self.cursor;
            self.cursor = (self.cursor + 1) % self.entries;
            self.entries_fetched += 1;
            let at = self.gpfifo_va + u64::from(gp) * GP_ENTRY_SIZE;
            let mut raw = [0u8; 8];
            mem.read(at, &mut raw).map_err(|why| RingRefusal::Read { gp, va: at, why })?;
            self.pending_retires = Some(self.cursor);
            let Some(e) = gp_entry_decode(u64::from_le_bytes(raw)) else {
                continue; // a control entry (NOP etc.): nothing to run, still retires
            };
            if e.sync_wait {
                return Err(RingRefusal::SyncWait { gp });
            }
            if e.len_bytes > MAX_SEGMENT_BYTES {
                return Err(RingRefusal::SegmentTooLong { gp, len: e.len_bytes });
            }
            let mut bytes = vec![0u8; e.len_bytes as usize];
            mem.read(e.gpu_va, &mut bytes).map_err(|why| RingRefusal::Read { gp, va: e.gpu_va, why })?;
            let words: Vec<u32> = bytes
                .chunks_exact(4)
                .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            let pieces = rewrite(&words, is_ce, &mut self.st, w).map_err(|why| RingRefusal::Rewrite { gp, why })?;
            self.pending.extend(pieces);
        }
    }
}
