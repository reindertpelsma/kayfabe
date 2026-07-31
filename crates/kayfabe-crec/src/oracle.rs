//! The guest RAM a recorded trace can answer — and, precisely, the part it cannot.
//!
//! ## What "hermetic" actually buys, and where it runs out
//!
//! `cap1` is hermetic: `m2fwd=off`, so nothing but the emulator ever wrote guest memory
//! and every byte the emulator *read* is in the file. That makes it a sound oracle for
//! **the reads the C performed**. It says nothing about any other address, and this is not
//! a recorder gap — it is a consequence of the C's own defects:
//!
//! - **GSP-D8** — the C addresses the shared region as `sharedMemPhysAddr + offset` and so
//!   **never reads the region's page table**. Our bind reads it (that is the whole point
//!   of [`kayfabe_gsp::RegionMap`]), and the capture cannot answer.
//! - **GSP-D2** — the C never reads the guest's status-queue `readPtr`, because it has no
//!   flow control. Our [`kayfabe_gsp::GspFsm::post`] reads it on every post.
//!
//! ⇒ **The C's guest-RAM read set is a strict subset of ours**, so a hermetic C capture
//! cannot, by itself, close a replay of a correctly-flow-controlled GSP. That is a
//! measured property of this oracle, and the harness reports it rather than papering over
//! it.
//!
//! ## The four answer sources, in order, each counted
//!
//! Every read is answered from exactly one of these and the count is part of the result,
//! so "matched" can never be confused with "invented":
//!
//! 1. [`Answer::Observed`] — bytes the C read (installed transaction by transaction) or
//!    bytes *we* wrote earlier in this run.
//! 2. [`Answer::Lookahead`] — the **nearest later** `GuestRead` in the same capture at
//!    exactly this `(gpa, len)`, and only if it is within [`LOOKAHEAD_LIMIT`] records.
//!    Still the C's own ground truth; needed because our read *order* differs from the
//!    C's (the bind drains the command ring, where the C waits for a doorbell —
//!    `kayfabe_gsp::boot`'s B4).
//!
//!    ★★ **The bound is load-bearing and it was learned the hard way.** Without it the
//!    oracle answered a read of command slot 7 with an observation 157 677 records later
//!    — a *different generation* of the same ring slot — and the run died on a checksum
//!    the guest itself would have rejected. That the failure was *detected* rather than
//!    silent is the point: the element checksum covers the whole run, so a mismatched
//!    generation cannot pass. The two populations in this capture are separated by two
//!    orders of magnitude (the largest sound use is 1 027 records; the first unsound one
//!    is 157 677), and [`OracleRam::max_lookahead`] reports which side of that a run was
//!    actually on.
//! 3. [`Answer::Reconstructed`] — a value the capture cannot contain, supplied under a
//!    **stated assumption** ([`ReconKind`]). Every one is a finding.
//! 4. [`Answer::Unobserved`] — refused, as [`RamRefused`]. The strict default.

use std::collections::BTreeMap;
use std::collections::VecDeque;

use kayfabe_gsp::{GuestRam, RamRefused};

/// Bytes per image page. The guest driver's `RM_PAGE_SIZE`, which is also the msgq
/// element size — a storage detail here, not a protocol constant.
const PAGE: u64 = 4096;

/// The one reason this oracle refuses, as [`RamRefused::why`]: the capture never
/// observed those bytes, so there is no honest answer to give.
///
/// ★ Public so the differential's assertions name the same constant the oracle produces.
/// A test that spelled the sentence out a second time would be asserting against its own
/// copy, and the two could drift without a single red test.
pub const UNOBSERVED: &str = "the capture does not contain those bytes; a replay may not \
                          invent guest memory the recorder never saw";

/// Every `GuestRead` the capture holds, keyed by `(gpa, len)` and carrying each
/// observation's record index — the lookahead pool.
type ReadPool = BTreeMap<(u64, usize), VecDeque<(usize, Vec<u8>)>>;

/// How far ahead in the capture [`Answer::Lookahead`] may reach.
///
/// Not a tuning knob: see the module docs. A guest-RAM address whose next observation is
/// this far away has, in a ring this size, been rewritten in between, and serving it is
/// serving a different generation.
pub const LOOKAHEAD_LIMIT: usize = 4096;

/// Where the bytes for one read came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Answer {
    /// The C read these bytes at or before this point, or we wrote them ourselves.
    Observed,
    /// The C read exactly this `(gpa, len)` **later** in the same capture, within
    /// [`LOOKAHEAD_LIMIT`] records.
    Lookahead,
    /// The capture cannot contain this; it was supplied under a stated assumption.
    Reconstructed(ReconKind),
    /// Nothing can answer it. The read was refused.
    Unobserved,
}

/// The assumption a reconstruction rests on. There is no `Other`: a reconstruction whose
/// justification cannot be named is not admissible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReconKind {
    /// **GSP-D8.** The region's own page table, synthesised as physically contiguous from
    /// its root address.
    ///
    /// The assumption is not free-floating: the capture *evidences* it for every page the
    /// run touched. The C computed `sharedMemPhysAddr + cmdQueueOffset` and
    /// `+ statQueueOffset`, read and wrote there, and the guest's own `msgqRxLink`
    /// accepted the header at that address — a fragmented region would have made the boot
    /// fail. What the capture cannot establish is the table's contents for pages the run
    /// never touched, and this fills those in the same shape.
    RegionPageTable,
    /// **GSP-D2.** The guest's status-queue `readPtr`, which the C never reads.
    ///
    /// Served as *"the guest has consumed everything we published"* — our own last
    /// published status write pointer. That is an **assumption about the guest**, not an
    /// observation: what the capture proves is only that the C's unbounded posting did
    /// not visibly break this particular run.
    PeerStatusReadPtr,
}

/// A read the capture could not answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Unobserved {
    /// Where.
    pub gpa: u64,
    /// How many bytes.
    pub len: usize,
}

/// A declared reconstruction, registered before a run and reported after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Reconstruction {
    /// The address it answers.
    pub gpa: u64,
    /// How many bytes it answers.
    pub len: usize,
    /// What it assumes.
    pub kind: ReconKind,
}

/// Guest RAM reconstructed from a recorded capture.
pub struct OracleRam {
    /// Sparse page image: `None` for a byte nothing has established.
    image: BTreeMap<u64, Vec<Option<u8>>>,
    /// Every `GuestRead` in the capture, keyed by `(gpa, len)`, as `(record index, bytes)`
    /// in file order — the lookahead pool. Position-aware: a lookahead serves the nearest
    /// observation at or after the replay's current position, never the globally first.
    future: ReadPool,
    /// Where the replay is in the capture, so lookahead can be position-aware.
    position: usize,
    /// The furthest a lookahead actually reached, in records.
    max_lookahead: usize,
    /// Declared reconstructions.
    recon: Vec<Reconstruction>,
    /// Our own status write pointer's address and last published value, once a bind has
    /// resolved them. Only [`ReconKind::PeerStatusReadPtr`] uses this.
    our_stat_write_ptr: Option<u64>,
    our_stat_write_ptr_value: u32,
    /// Whether [`Answer::Lookahead`] is permitted.
    lookahead: bool,

    /// Every write we made, in order: `(gpa, bytes)`. The assertion target.
    pub writes: Vec<(u64, Vec<u8>)>,
    /// How each read was answered, in order.
    pub answers: Vec<(u64, usize, Answer)>,
    /// The reads nothing could answer, in order.
    pub unobserved: Vec<Unobserved>,
}

impl OracleRam {
    /// Build the oracle for one capture.
    ///
    /// `reads` is every `GuestRead` payload in the capture, in file order; the image
    /// starts **empty** and is filled by [`OracleRam::observe`] as the replay advances, so
    /// a read is never silently answered by a later value of the same address unless
    /// `lookahead` explicitly allows it — and then it is counted as such.
    #[must_use]
    pub fn new(reads: Vec<(usize, u64, Vec<u8>)>, lookahead: bool) -> OracleRam {
        let mut pool: ReadPool = ReadPool::new();
        for (at, gpa, bytes) in reads {
            pool.entry((gpa, bytes.len()))
                .or_default()
                .push_back((at, bytes));
        }
        OracleRam {
            image: BTreeMap::new(),
            future: pool,
            position: 0,
            max_lookahead: 0,
            recon: Vec::new(),
            our_stat_write_ptr: None,
            our_stat_write_ptr_value: 0,
            lookahead,
            writes: Vec::new(),
            answers: Vec::new(),
            unobserved: Vec::new(),
        }
    }

    /// Declare a reconstruction. Every one of these is reported as a finding.
    pub fn reconstruct(&mut self, r: Reconstruction) {
        if !self.recon.contains(&r) {
            self.recon.push(r);
        }
    }

    /// The reconstructions in force.
    #[must_use]
    pub fn reconstructions(&self) -> &[Reconstruction] {
        &self.recon
    }

    /// Tell the oracle which address carries our published status write pointer, so
    /// [`ReconKind::PeerStatusReadPtr`] can serve *"the guest kept up"*.
    pub fn bind_pointers(&mut self, our_stat_write_ptr: u64) {
        self.our_stat_write_ptr = Some(our_stat_write_ptr);
    }

    /// Install one of the C's observations of guest memory. Called as the replay reaches
    /// each transaction, never ahead of it.
    pub fn observe(&mut self, gpa: u64, bytes: &[u8]) {
        self.put(gpa, bytes);
    }

    /// Tell the oracle where the replay is in the capture. Lookahead is measured from
    /// here.
    pub fn seek(&mut self, position: usize) {
        self.position = position;
    }

    /// The furthest any lookahead in this run reached, in records. Reported so a reader
    /// can see which side of [`LOOKAHEAD_LIMIT`] the run actually lived on.
    #[must_use]
    pub fn max_lookahead(&self) -> usize {
        self.max_lookahead
    }

    /// How many reads each source answered.
    #[must_use]
    pub fn answer_census(&self) -> Vec<(Answer, usize)> {
        let mut out: Vec<(Answer, usize)> = Vec::new();
        for (_, _, a) in &self.answers {
            match out.iter_mut().find(|(k, _)| k == a) {
                Some((_, c)) => *c += 1,
                None => out.push((*a, 1)),
            }
        }
        out.sort_unstable();
        out
    }

    fn put(&mut self, gpa: u64, bytes: &[u8]) {
        for (i, b) in bytes.iter().enumerate() {
            let at = gpa + i as u64;
            let page = self
                .image
                .entry(at / PAGE)
                .or_insert_with(|| vec![None; PAGE as usize]);
            page[(at % PAGE) as usize] = Some(*b);
        }
    }

    fn get(&self, gpa: u64, buf: &mut [u8]) -> bool {
        for (i, slot) in buf.iter_mut().enumerate() {
            let at = gpa + i as u64;
            match self
                .image
                .get(&(at / PAGE))
                .and_then(|p| p[(at % PAGE) as usize])
            {
                Some(b) => *slot = b,
                None => return false,
            }
        }
        true
    }
}

impl GuestRam for OracleRam {
    fn read(&mut self, gpa: u64, buf: &mut [u8]) -> Result<(), RamRefused> {
        if self.get(gpa, buf) {
            self.answers.push((gpa, buf.len(), Answer::Observed));
            return Ok(());
        }
        if self.lookahead
            && let Some(pool) = self.future.get_mut(&(gpa, buf.len()))
            && let Some(k) = pool.iter().position(|(at, _)| *at >= self.position)
            && pool[k].0 - self.position <= LOOKAHEAD_LIMIT
        {
            let (at, bytes) = pool.remove(k).expect("k came from this deque");
            self.max_lookahead = self.max_lookahead.max(at - self.position);
            buf.copy_from_slice(&bytes);
            self.put(gpa, &bytes);
            self.answers.push((gpa, buf.len(), Answer::Lookahead));
            return Ok(());
        }
        if let Some(kind) = self
            .recon
            .iter()
            .find(|r| r.gpa == gpa && r.len == buf.len())
            .map(|r| r.kind)
        {
            match kind {
                ReconKind::RegionPageTable => {
                    // A physically contiguous table rooted at its own address: entry `i`
                    // is `gpa + i * 4096`, which is exactly what the C's
                    // `sharedMemPhysAddr + offset` addressing assumes.
                    for (i, chunk) in buf.chunks_exact_mut(8).enumerate() {
                        chunk.copy_from_slice(&(gpa + i as u64 * PAGE).to_le_bytes());
                    }
                }
                ReconKind::PeerStatusReadPtr => {
                    buf.copy_from_slice(&self.our_stat_write_ptr_value.to_le_bytes());
                }
            }
            let filled = buf.to_vec();
            self.put(gpa, &filled);
            self.answers
                .push((gpa, buf.len(), Answer::Reconstructed(kind)));
            return Ok(());
        }
        self.answers.push((gpa, buf.len(), Answer::Unobserved));
        self.unobserved.push(Unobserved {
            gpa,
            len: buf.len(),
        });
        Err(RamRefused {
            gpa,
            len: buf.len(),
            why: UNOBSERVED,
        })
    }

    fn write(&mut self, gpa: u64, bytes: &[u8]) -> Result<(), RamRefused> {
        self.put(gpa, bytes);
        self.writes.push((gpa, bytes.to_vec()));
        if self.our_stat_write_ptr == Some(gpa)
            && let Ok(b) = <[u8; 4]>::try_from(bytes)
        {
            self.our_stat_write_ptr_value = u32::from_le_bytes(b);
        }
        Ok(())
    }
}

kayfabe_util::assert_send_sync!(Answer, ReconKind, Reconstruction, Unobserved);
