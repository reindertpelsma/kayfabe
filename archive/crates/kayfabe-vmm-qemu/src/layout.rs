//! ★★★ The **stated** guest-RAM layout: which guest-physical ranges are backed by which
//! host file, and at what offset into it.
//!
//! # Why this module exists at all
//!
//! `guest_ram_crossing.md` §5.6 closed step 1 with one line, and it is this module's whole
//! brief:
//!
//! > **a census yields an EXTENT, and an extent is not a LAYOUT.**
//!
//! Step 1 found the hypervisor's guest-RAM block by its properties and learned its **size**.
//! Nothing in that answer says which *guest-physical address* corresponds to which *byte of
//! the file*. On the one command line the bench happens to run — `-m 2048` on `q35` — the
//! two coincide, and `traces/guest_boots/run_w224m_mtree.log` measured exactly that: **12
//! `ram0` ranges, 0 non-identity, nothing at or above 4 GiB**.
//!
//! ⊘ **That measurement settles ONE command line and must not be promoted to a rule.** With
//! `-m 8G` the hypervisor splits RAM around the 4 GiB PCI hole — the boots in
//! `docs/reference/bench_rebuild_notes.md` do it — and the identity breaks at the split. A
//! layout derived from the machine type would be a **second declaration** of a fact the
//! hypervisor already owns, and it would be wrong the first time somebody changes `-m`.
//!
//! ⇒ So the layout is **STATED, never DERIVED**. Every run in this table arrived on the
//! topology listener's own callback, carrying the hypervisor's own numbers. This module
//! stores what it was told and answers only what it was told.
//!
//! # The join, and why it is on the block and not on a number
//!
//! A section reports a region identity (`mr`), which is an address and means nothing across
//! processes, and it reports a backing **file**. Guest RAM is one block among many that
//! answer `is_ram` — video RAM, option ROMs and the SMRAM alias all do — so a section is
//! attributed to guest RAM by joining its backing on `(st_dev, st_ino)` against the block
//! step 1's census adopted.
//!
//! ★ That is the same discipline the census itself was forced into, for the same reason:
//! `procfd.rs` had to stop keying on descriptor **numbers** (they moved between two physical
//! benches) and key on **blocks, inode-joined**. A layout keyed on `mr` would repeat the
//! mistake one layer up — `mr` is stable only within one process's lifetime, and it is not
//! the thing the isolate will `mmap`.
//!
//! # What this module refuses, and why refusal is the whole point
//!
//! [`GuestRamLayout::resolve`] answers a guest-physical range with a file offset **or a
//! named refusal**. There is deliberately no third outcome:
//!
//! - ⊘ **no clamping.** A request that starts inside a run and leaves it is
//!   [`LayoutRefusal::StraddlesRuns`]. Truncating it to the part that fits would hand back a
//!   *shorter* mapping under the *asked-for* name, and the caller — `OS_DESCRIPTOR` over a
//!   DMA range — would pin fewer pages than the guest is about to use.
//! - ⊘ **no best-effort.** A guest-physical address in no stated run is
//!   [`LayoutRefusal::NoStatedRun`], not a guess.
//! - ⊘ **no "probably identity".** There is no fallback that treats an unknown address as
//!   its own offset. That fallback is precisely the `-m 8G` bug, and it would be *silent*.
//! - ⊘ **no cross-block resolution.** RAM backed by some *other* file is
//!   [`LayoutRefusal::OtherBacking`] — a distinct name, because "this is video RAM" and
//!   "this is not memory at all" are different facts and the caller may want to say which.

use std::collections::BTreeMap;

/// ★ A backing **file**, named the way a join can trust: by identity on the filesystem,
/// never by descriptor number and never by the address of a region object.
///
/// Both halves are carried because an inode number is unique only within a device. In
/// practice every `memfd` lives on the one internal `shmem` mount and `dev` is constant —
/// which is exactly why carrying it costs nothing and removes an assumption rather than
/// documenting one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BackingId {
    /// `st_dev` of the backing file.
    pub dev: u64,
    /// `st_ino` of the backing file.
    pub ino: u64,
}

impl BackingId {
    /// The identity of a backing file.
    #[must_use]
    pub const fn new(dev: u64, ino: u64) -> Self {
        BackingId { dev, ino }
    }
}

/// ★★ One **stated** run: a guest-physical range, and the byte offset into the backing file
/// at which its first byte lives.
///
/// The name is `file_offset`, not `offset` — the value that matters to the eventual `mmap`
/// is an offset into the *file*, and a section reports an offset into a *region*. The two
/// differ whenever the hypervisor placed the region at a non-zero offset into its
/// descriptor, which is a thing it is allowed to do and which no caller should have to
/// remember to add.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StatedRun {
    /// Guest-physical base of the run.
    pub gpa: u64,
    /// Length of the run in bytes.
    pub len: u64,
    /// Byte offset into the backing file of the run's first byte.
    pub file_offset: u64,
}

impl StatedRun {
    /// The guest-physical address one past the run's last byte, widened so a run that ends
    /// at the top of the address space does not wrap.
    #[must_use]
    pub fn gpa_end(&self) -> u128 {
        u128::from(self.gpa) + u128::from(self.len)
    }

    /// Whether this run is **identity-mapped** — its file offset equals its guest-physical
    /// base.
    ///
    /// ⊘ Reported for the boot log only. Nothing in this module *acts* on it: the whole
    /// point is that identity is an observation about one run, never a rule.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.gpa == self.file_offset
    }
}

/// Why a guest-physical range could not be answered. Every arm is a **name**, because the
/// alternative to a name here is a plausible number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutRefusal {
    /// The address is in no run the hypervisor stated for this backing file.
    NoStatedRun {
        /// The guest-physical address asked for.
        gpa: u64,
    },
    /// The range begins inside a stated run and leaves it. ⊘ Not clamped: see the module
    /// documentation.
    StraddlesRuns {
        /// The guest-physical address asked for.
        gpa: u64,
        /// The length asked for.
        len: u64,
        /// How many bytes the run containing `gpa` actually had left.
        available: u64,
    },
    /// The address is stated RAM, but it is backed by a **different** file.
    OtherBacking {
        /// The guest-physical address asked for.
        gpa: u64,
    },
    /// A zero-length request. Refused rather than answered, because the answer would be a
    /// mapping nobody can use and the caller almost certainly computed it.
    EmptyRange {
        /// The guest-physical address asked for.
        gpa: u64,
    },
    /// A range that leaves the 64-bit guest-physical space.
    OutOfSpace {
        /// The guest-physical address asked for.
        gpa: u64,
        /// The length asked for.
        len: u64,
    },
}

impl LayoutRefusal {
    /// The refusal as a stable, printable name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            LayoutRefusal::NoStatedRun { .. } => "NoStatedRun",
            LayoutRefusal::StraddlesRuns { .. } => "StraddlesRuns",
            LayoutRefusal::OtherBacking { .. } => "OtherBacking",
            LayoutRefusal::EmptyRange { .. } => "EmptyRange",
            LayoutRefusal::OutOfSpace { .. } => "OutOfSpace",
        }
    }
}

/// ★★★ The hypervisor's own statement of where guest RAM is, accumulated one topology
/// callback at a time.
///
/// Keyed by guest-physical base, so the natural iteration order is the **GPA order** step 3
/// wants for `OS_DESCRIPTOR` placement, and so a delete — which the listener issues by
/// `(gpa, len)` — is a single lookup.
#[derive(Debug, Default, Clone)]
pub struct GuestRamLayout {
    stated: BTreeMap<u64, (BackingId, StatedRun)>,
    /// ★★★ Every run that was **ever** stated in this process's lifetime, never withdrawn.
    ///
    /// ⊘ Not a cache and never consulted by [`GuestRamLayout::resolve`] — resolution answers
    /// from the LIVE table only, because a withdrawn range is one the hypervisor has stopped
    /// backing and answering for it would be the memory-plane equivalent of a stale mapping.
    ///
    /// It exists because the layout is **transient** and every report of it samples an
    /// instant. `[measured 2026-08-10, boots w225c/w225d]`: at memory-plane attach the flat
    /// view is empty (the listener sits on the device's bus-master address space, which the
    /// guest has not enabled yet); at the exit notifier it is empty **again**, because
    /// teardown replays `region_del` over every range. The live table was correct in
    /// between, and both of the instants a device can easily reach show zero.
    ///
    /// ★ So this is what makes the boot log carry evidence at all: "what was stated at some
    /// point in this run" is a question with a stable answer, and it is the one an operator
    /// reading a finished log can actually be given.
    ever: BTreeMap<u64, (BackingId, StatedRun)>,
    census: LayoutCensus,
}

/// ★★★ Why the section counts are kept, and why they are three numbers and not one.
///
/// `[measured 2026-08-10, boot w225a, rev fbc8cd7]` the first armed boot of this module
/// reported **0 runs out of 0 stated sections**, and 0-of-0 does not say which of three very
/// different things happened: the listener never fired, it fired and nothing classified as
/// RAM, or it fired with RAM that carried no backing identity. Each has a different fix and
/// two of them are not in this file at all.
///
/// ⊘ A single "stated" count is exactly the instrument that cannot tell them apart — the
/// shape this project has now named twice (`a_wall_that_can_carry_no_name`, and the census
/// that reported two memfds as three). So the report carries the whole funnel, and every
/// stage of it is counted at the boundary it describes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LayoutCensus {
    /// Every section the listener handed us, whatever it turned out to be.
    pub seen: u64,
    /// Of those, the ones that classified as plain RAM.
    pub ram: u64,
    /// Of those, the ones that also named a backing file — i.e. the ones that stated a run.
    pub backed: u64,
    /// How many statements were **withdrawn** by a delete callback. ★ The number that tells
    /// "nothing was ever stated" apart from "everything was stated and then torn down",
    /// which are the two readings of an empty live table and have nothing in common.
    pub forgotten: u64,
}

impl GuestRamLayout {
    /// An empty layout — nothing stated, so everything is refused.
    #[must_use]
    pub fn new() -> Self {
        GuestRamLayout {
            stated: BTreeMap::new(),
            ever: BTreeMap::new(),
            census: LayoutCensus::default(),
        }
    }

    /// ★ Count one section the listener reported, at the boundary where the two facts that
    /// decide its fate are both known. Called for **every** section, including the ones this
    /// module then ignores — that is the point.
    pub fn saw(&mut self, is_ram: bool, has_backing: bool) {
        self.census.seen += 1;
        if is_ram {
            self.census.ram += 1;
            if has_backing {
                self.census.backed += 1;
            }
        }
    }

    /// The three-stage funnel. See [`LayoutCensus`].
    #[must_use]
    pub fn census(&self) -> LayoutCensus {
        self.census
    }

    /// Record what the hypervisor just said about one section.
    ///
    /// A repeated statement for the same base **replaces** the old one. That is the
    /// hypervisor re-flattening its view, not a conflict: the listener's contract is that a
    /// range is deleted before it is re-added, and the last statement is the current one.
    pub fn state(&mut self, backing: BackingId, run: StatedRun) {
        self.stated.insert(run.gpa, (backing, run));
        self.ever.insert(run.gpa, (backing, run));
    }

    /// Forget the run based at `gpa`, if any. Mirrors the listener's delete callback.
    pub fn forget(&mut self, gpa: u64) {
        if self.stated.remove(&gpa).is_some() {
            self.census.forgotten += 1;
        }
    }

    /// Forget everything.
    pub fn clear(&mut self) {
        self.stated.clear();
    }

    /// How many sections have been stated, over all backing files.
    #[must_use]
    pub fn stated_sections(&self) -> usize {
        self.stated.len()
    }

    /// ★ The runs stated for one backing file, in **guest-physical order**, with adjacent
    /// sections coalesced.
    ///
    /// Two sections coalesce only when they are contiguous in **both** axes — the second
    /// begins where the first ends in guest-physical space *and* at the byte after it in the
    /// file. ⊘ A pair contiguous in only one axis is left as two runs, because a single
    /// descriptor over such a pair would be a mapping whose second half is somewhere else.
    ///
    /// This is why the hypervisor's twelve reported `ram0` sections become the handful of
    /// runs a caller actually wants: the flat view is sliced wherever *anything* changes,
    /// including things that have no bearing on where the bytes are.
    #[must_use]
    pub fn contiguous_runs(&self, backing: BackingId) -> Vec<StatedRun> {
        Self::coalesce(&self.stated, backing)
    }

    /// ★★★ The same, over every run **ever** stated rather than the live ones. See the
    /// `ever` field: this is the form a finished boot log can be given, because both instants
    /// a device can easily report at are empty for unrelated reasons.
    ///
    /// ⊘ Evidence only. Nothing resolves against it.
    #[must_use]
    pub fn contiguous_runs_ever(&self, backing: BackingId) -> Vec<StatedRun> {
        Self::coalesce(&self.ever, backing)
    }

    fn coalesce(
        from: &BTreeMap<u64, (BackingId, StatedRun)>,
        backing: BackingId,
    ) -> Vec<StatedRun> {
        let mut out: Vec<StatedRun> = Vec::new();
        for (_, run) in from.values().filter(|(b, _)| *b == backing) {
            match out.last_mut() {
                Some(prev)
                    if prev.gpa_end() == u128::from(run.gpa)
                        && u128::from(prev.file_offset) + u128::from(prev.len)
                            == u128::from(run.file_offset) =>
                {
                    prev.len += run.len;
                }
                _ => out.push(*run),
            }
        }
        out
    }

    /// ★★★ Every backing file that has stated at least one run, with how many sections it
    /// stated and how many bytes they cover — in `(dev, ino)` order.
    ///
    /// ⊘ This exists because "0 runs for the block we adopted" is a **join** failure, and a
    /// join failure is undiagnosable from either side alone: the count says the join missed,
    /// and only the other side's key says why. `[measured 2026-08-10, boot w225c]` the funnel
    /// reported `76 -> 10 -> 8` and zero runs for the adopted block, which narrows the fault
    /// to the key and says nothing about which half of it.
    ///
    /// ★ It is the same lesson as the descriptor census one layer down: the cure for "no
    /// match" is never a better search, it is **printing what was there**.
    #[must_use]
    pub fn backings_seen(&self) -> Vec<(BackingId, usize, u128)> {
        let mut out: Vec<(BackingId, usize, u128)> = Vec::new();
        for (b, r) in self.ever.values() {
            match out.iter_mut().find(|(k, _, _)| k == b) {
                Some(row) => {
                    row.1 += 1;
                    row.2 += u128::from(r.len);
                }
                None => out.push((*b, 1, u128::from(r.len))),
            }
        }
        out.sort_by_key(|(b, _, _)| (b.dev, b.ino));
        out
    }

    /// ★★★ Answer a guest-physical range with the file offset of its first byte, or refuse
    /// **by name**.
    ///
    /// The returned run is exactly the range asked for — same base, same length — so a
    /// caller cannot accidentally use a longer run's length.
    ///
    /// # Errors
    /// Every arm of [`LayoutRefusal`]. There is no success-with-a-caveat.
    pub fn resolve(
        &self,
        backing: BackingId,
        gpa: u64,
        len: u64,
    ) -> Result<StatedRun, LayoutRefusal> {
        if len == 0 {
            return Err(LayoutRefusal::EmptyRange { gpa });
        }
        let end = u128::from(gpa) + u128::from(len);
        if end > u128::from(u64::MAX) + 1 {
            return Err(LayoutRefusal::OutOfSpace { gpa, len });
        }
        // ★ Resolved against the COALESCED runs, not the raw sections. A DMA range that
        // spans two adjacent sections of one contiguous block is not a straddle — the bytes
        // really are contiguous in the file — and refusing it would refuse the ordinary
        // case. Coalescing already proved the contiguity in both axes.
        let runs = self.contiguous_runs(backing);
        let Some(run) = runs
            .iter()
            .find(|r| r.gpa <= gpa && u128::from(gpa) < r.gpa_end())
        else {
            // ⊘ Distinguish "some other file backs this address" from "nothing does". Both
            // are refusals; only one of them means the caller asked about video RAM.
            let other = self
                .stated
                .values()
                .any(|(_, r)| r.gpa <= gpa && u128::from(gpa) < r.gpa_end());
            return Err(if other {
                LayoutRefusal::OtherBacking { gpa }
            } else {
                LayoutRefusal::NoStatedRun { gpa }
            });
        };
        if end > run.gpa_end() {
            let available = u64::try_from(run.gpa_end() - u128::from(gpa)).unwrap_or(u64::MAX);
            return Err(LayoutRefusal::StraddlesRuns {
                gpa,
                len,
                available,
            });
        }
        Ok(StatedRun {
            gpa,
            len,
            file_offset: run.file_offset + (gpa - run.gpa),
        })
    }
}
