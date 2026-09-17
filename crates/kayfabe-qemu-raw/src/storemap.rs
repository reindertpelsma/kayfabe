//! ★★★★★ **CONSTRAINT 26 — THE PORT THROUGH WHICH THE SCRATCHPAD DOES ALL GPU-SIDE
//! MAPPING.**
//!
//! > Owner, 2026-09-15: *"All memory is held by the scratchpad, the userspace isolates only
//! > borrow from it."*
//!
//! | | **owns** | **sees** |
//! |---|---|---|
//! | per-proc **isolate** | channel, VA space, compute, control | only its own VA space |
//! | **scratchpad** | the vidmem GPGA object, the tables, bounded kernel channels | all vidmem, all VA spaces |
//!
//! This is the second half of that table, as one object. It holds the scratchpad's isolate
//! handle and **the reservation**, and it is the only route from the VMM to a mapping of
//! guest video memory into a guest address space.
//!
//! ## ⊘ Why it is a sibling of `DeviceViewPort` and not a mode of it
//!
//! [`crate::deviceview::DeviceViewPort`] arms a **CPU** view of the reserved object — an
//! `mmap` this process can `memcpy` through, and an aperture it must give back.
//! [`StoreMapPort`] places a **GPU** mapping in somebody else's address space, consumes no
//! aperture, and is given back by an unmap whose acknowledgement constraint 27's barrier
//! reads. Two resources, two lifetimes, two failure modes; a flag on one type would make
//! the wrong one releasable by the wrong caller.
//!
//! ## ★★★ THE RESTATED ASSERTION — what `RING_NOT_A_JOINED_WINDOW` was for
//!
//! `alloc_channel_declared` refuses a channel birth over an object the birth isolate did not
//! mint by joining a framebuffer leaf. The failure it prevents is exact and is **silent**: a
//! channel over a blank twin fetches zeros, never advances `GP_GET`, and reports no error at
//! all.
//!
//! Under §26 that check cannot be made where it is made today — the per-proc isolate holds
//! neither the object nor the mapping, so it has nothing to check against. ⇒ **the mechanism
//! is deleted and the question is kept**, asked of the only party that can answer it:
//! [`StoreMapPort::is_slice_of_the_store`], which says whether *this* port placed *that*
//! range, out of *the one object*, in *that* address space.
//!
//! ⚠ **It is a live query of the port's own ledger, not a token.** A token crossing the wire
//! would be integers a caller could produce; this is the mapper being asked what it mapped.
//! ⊘ What it is **not**: proof that the GPU can walk to the bytes. Only a submission proves
//! that, which is exactly what `RING_NOT_A_JOINED_WINDOW` never proved either.

use std::sync::atomic::{AtomicU64, Ordering};

use kayfabe_isolate::{BareVaSpace, HostHandle, IsolateId};
use kayfabe_rt::GpuVa;

use crate::scratchpad::SharedIsolate;

/// Why a store mapping did not happen. ⊘ Four names, never one string: *"the scratchpad had
/// no worker"* and *"RM refused the placement"* lead a reader to different files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreMapRefusal {
    /// The scratchpad offered no worker. Its pool is saturated or it is retired.
    NoWorker,
    /// RM refused, verbatim.
    Rm(String),
    /// ★ The address space was never handed over, so this port has no `hDma` for it.
    NotAdopted { space: u64 },
    /// ⚠ The requested slice is outside the reservation. ⊘ Refused **here**, before the
    /// ioctl: RM would map whatever offset it was given, and a run past the object's end is
    /// a guest range pointed at memory the reservation does not cover.
    OutOfRange { offset: u64, len: u64, obj_len: u64 },
    /// ★★★★★ **THE CALLER IS ON A vCPU OR INSIDE A GUEST TRAP.**
    ///
    /// Every verb here is an IPC round trip to another process. `OffTrap::claim` **panics**
    /// on a trap thread, and a panic in the VMM is a guest-visible crash produced by a rule
    /// that exists to prevent a stall. ⇒ declined **by name**, counted, and the leaf is
    /// re-offered by the next publish — which is the route's own iteration and not ours.
    ///
    /// ⊘ Distinct from every other arm: nothing was asked and nothing refused. A boot that
    /// reads this as an RM refusal is reading *"we did not ask"* as *"the driver said no"*.
    OnVcpu,
    /// ⚠ A handle that does not fit RM's 32 bits. ⊘ Refused rather than truncated:
    /// `u32::try_from(..).unwrap_or(0)` would dup **object 0**, which is a legal-looking
    /// handle in somebody else's namespace.
    BadHandle { raw: u64 },
    /// ★★★★★ **w755 — CONSTRAINT 28 REFUSED IT: RM PLACED THE SLICE SOMEWHERE ELSE.**
    ///
    /// ⊘⊘⊘ **This arm exists because the boot that needed it could not say it.**
    /// `[measured w753]` the split-ownership boot reported `map_refused=2154` with
    /// `first_refusal=Rm("Other(19270)")` — an opaque integer that was ALSO another
    /// constant's, so the one number naming the wall named nothing. The refusal was
    /// constraint 28 all along, and `want`/`got` had been discarded at the isolate IPC
    /// boundary (see `kayfabe_isolate_host::proto::WireError::PlacementRefused`).
    ///
    /// ⚠ Split out of [`StoreMapRefusal::Rm`] rather than left inside it, and the
    /// distinction is not cosmetic: `Rm` means *"the driver said no"* and this means
    /// **"the driver said YES, at the wrong address, and we took it back down"** — a
    /// mapping that briefly existed, a guest VA that is still unbacked, and an invariant
    /// of OURS that broke. Reading the second as the first sends the reader to the host.
    ///
    /// ★ Rendered in hex by [`StoreMapRefusal::detail`], because these are GPU VAs and a
    /// 12-digit decimal is not a number anybody compares against a page table by eye.
    Placement {
        /// The guest's own VA, binding.
        want: u64,
        /// Where RM put it instead.
        got: u64,
    },
    /// ★★★★★ **w755d — THIS VA ALREADY HOLDS A DIFFERENT SLICE OF THE STORE.**
    ///
    /// ⊘ Re-offering the *same* slice is idempotent and answers `Ok` without touching RM —
    /// see [`StoreMapPort::map`]. This is the other case: the same guest VA named with a
    /// different `(offset, len)`, which is **two memories at one address**, the state the
    /// single store exists to abolish. Refused by name rather than re-placed, because
    /// whichever mapping won would be invisible to whichever caller lost.
    AlreadyPlacedDifferently {
        /// The guest VA in question.
        at: u64,
        /// The store offset already placed there.
        had_offset: u64,
        /// Its length.
        had_len: u64,
        /// The store offset now being offered.
        want_offset: u64,
        /// Its length.
        want_len: u64,
    },
    /// ★★★★★ **w755g — A RE-POINT WHOSE UNMAP WAS REFUSED, so the new slice was NOT placed.**
    ///
    /// ⊘ Fail-closed by design. The alternative — map the new slice anyway — is two memories
    /// at one address, with the additional property that nobody knows which one the engine
    /// will resolve. ⚠ Distinct from [`StoreMapRefusal::Rm`]: the *map* was never attempted,
    /// and a reader hunting a failed `NVOS46` would find none.
    ReplaceUnmapRefused {
        /// The guest VA in question.
        at: u64,
        /// The store offset that is still placed there.
        had_offset: u64,
        /// Its length.
        had_len: u64,
        /// The store offset that could not be placed.
        want_offset: u64,
        /// Its length.
        want_len: u64,
        /// What the unmap refused with, whole.
        why: String,
    },
}

impl StoreMapRefusal {
    /// The short name a census prints.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            StoreMapRefusal::NoWorker => "NoWorker",
            StoreMapRefusal::Rm(_) => "Rm",
            StoreMapRefusal::NotAdopted { .. } => "NotAdopted",
            StoreMapRefusal::OutOfRange { .. } => "OutOfRange",
            StoreMapRefusal::OnVcpu => "OnVcpu",
            StoreMapRefusal::BadHandle { .. } => "BadHandle",
            StoreMapRefusal::Placement { .. } => "Placement",
            StoreMapRefusal::AlreadyPlacedDifferently { .. } => "AlreadyPlacedDifferently",
            StoreMapRefusal::ReplaceUnmapRefused { .. } => "ReplaceUnmapRefused",
        }
    }

    /// ★ The one-line detail a census prints beside the name — hex for anything that is an
    /// address, because a boot log is read against a page table by eye.
    #[must_use]
    pub fn detail(&self) -> String {
        match self {
            StoreMapRefusal::Placement { want, got } => {
                format!(
                    "CONSTRAINT 28: want={want:#x} got={got:#x} delta={:#x}",
                    got.wrapping_sub(*want)
                )
            }
            other => format!("{other:?}"),
        }
    }
}

/// One placed slice, as this port remembers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Placed {
    /// Offset inside the reserved object.
    offset: u64,
    /// Length in bytes.
    len: u64,
}

/// ★★★★★ **THE SCRATCHPAD'S MAPPING PORT.** See the module docs.
pub struct StoreMapPort {
    iso: std::sync::Arc<SharedIsolate>,
    /// Which isolate this is, for a census that can say *whose* worker answered.
    id: IsolateId,
    /// ★★★ The reserved object. ⊘ Captured once at construction and never a parameter: a
    /// mapping of a *different* object at a guest's VA is the "two memories at one address"
    /// state the single store exists to abolish, and a per-call object argument is how that
    /// would be spelled.
    obj: HostHandle,
    /// Its length, for the bounds check no ioctl will make for us.
    obj_len: u64,
    /// `per-proc space handle -> the scratchpad's own range over the dup`.
    adopted: std::sync::Mutex<std::collections::BTreeMap<u64, HostHandle>>,
    /// `(scratchpad range, guest VA) -> what was placed there`. **The ledger the restated
    /// ring assertion reads.**
    placed: std::sync::Mutex<std::collections::BTreeMap<(u64, u64), Placed>>,
    adopts: AtomicU64,
    adopt_refused: AtomicU64,
    maps: AtomicU64,
    map_refused: AtomicU64,
    unmaps: AtomicU64,
    unmap_refused: AtomicU64,
    bytes_mapped: AtomicU64,
    /// ★★★ How many times the restated assertion was **asked**, and how many times it said
    /// no. ⊘ Both, because `asked=0` and `refused=0` are the same line otherwise, and the
    /// first means the gate is not wired.
    /// ★★★ w755r — CHANNEL births carried into B: asked, refused, and actually borne.
    ///
    /// ⚠ **Named `chan_*` and kept apart from [`Self::birth_clients`]/[`Self::birth_refused`]
    /// deliberately** — those count birth-CLIENT hand-overs, a different event entirely, and
    /// one counter covering both would make a census unable to say whether a boot failed to
    /// hand a client over or failed to birth a channel in one it had. That is this file's own
    /// `a_refusal_counter_read_as_absent_demand` warning, applied to the counter beside it.
    ///
    /// ⊘ Three and not one: *"we did not ask"*, *"we asked and were refused"* and *"it
    /// worked"* are different facts, and a census that adds them cannot tell a route that
    /// never fired from one that fired and failed — the confusion w755q spent a boot on.
    chan_births: AtomicU64,
    chan_birth_refused: AtomicU64,
    chan_born: AtomicU64,
    asserted: AtomicU64,
    assert_refused: AtomicU64,
    /// ★★★ Calls declined because the caller was on a vCPU or inside a guest trap. ⊘ Its own
    /// counter: *"we did not ask"* and *"RM said no"* are different facts with different
    /// fixes, and a census that adds them cannot tell a stalled publish route from a broken
    /// driver.
    declined_on_vcpu: AtomicU64,
    /// ★★★ **CONSTRAINT 32 — birth clients handed to this port, and refusals.** ⊘ Both,
    /// because `refused=0` alone is the `a_refusal_counter_read_as_absent_demand` shape: it
    /// reads as *"nothing was refused"* when it may mean *"nothing was asked"*.
    birth_clients: AtomicU64,
    /// Birth clients this port refused, or RM did.
    birth_refused: AtomicU64,
    /// ★ Per-proc isolates that have a birth client here, so the census can say whether the
    /// arm reached every proc or only the first.
    birth_procs: std::sync::Mutex<std::collections::BTreeSet<u32>>,
    /// The first refusal's name, so a census can say WHICH of four fired. `None` means none.
    first_refusal: std::sync::Mutex<Option<String>>,
    /// ★★★★★ **w755 — THE DISTINCT REFUSALS, WITH COUNTS. `first_refusal` ALONE IS TOO
    /// THIN FOR A FOUR-FIGURE NUMBER.**
    ///
    /// ⊘ `[measured w753]` `map_refused=2154 first_refusal=[one string]` cannot distinguish
    /// *"one cause, 2 154 times"* from *"2 154 distinct causes"*, and those have completely
    /// different fixes. The first is a single wrong invariant; the second is a path that is
    /// wrong per-page. A census that cannot tell them apart is the
    /// `falsifier_blocker_vs_only_blocker` shape, at the one place this boot is stuck.
    ///
    /// ⚠ **Capped at [`Self::DISTINCT_CAP`] keys**, and the cap is *reported* rather than
    /// silently applied: an unbounded map keyed on a guest-influenced string is a memory
    /// grow the guest can drive. Once full, further distinct refusals increment
    /// [`Self::refusal_kinds_dropped`] — so *"the histogram is complete"* and *"the
    /// histogram is a sample"* are different, visible states.
    refusal_kinds: std::sync::Mutex<std::collections::BTreeMap<String, u64>>,
    /// Distinct refusal strings that did not fit under the cap. ⊘ See [`Self::refusal_kinds`]
    /// — a truncated histogram that does not say it is truncated is worse than none.
    refusal_kinds_dropped: AtomicU64,
    /// ★★★★★ **w755g — RE-POINTS: a VA that already held a DIFFERENT slice.**
    ///
    /// ⊘ Its own counter because `unmaps` alone cannot distinguish *"the guest retired a
    /// leaf"* from *"a VA was re-pointed at a new slice"*, and only the second is the shape
    /// the raw client's `P1 round r+1` and `STALE RACE` exercise deliberately. A zero here
    /// with a passing client means the client never re-pointed; a zero with a FAILING one
    /// means this path is not the reason.
    replaced: AtomicU64,
    /// ★★★★★ **w755 — SLICES WHOSE STORE OFFSET IS LESS ALIGNED THAN THEIR LENGTH IMPLIES.**
    ///
    /// ⊘⊘ This counter exists to make an ARGUMENT CHECKABLE. w755 proposed that
    /// `map_refused` was `nvos46_page_size_flag` ignoring the slice offset; the owner refuted
    /// it from provenance: these slices come out of a **page-table walk**, `len` is the
    /// walk's own page size and `offset` is the frame that PTE names, and a PTE's physical
    /// base is aligned to its own page size **by hardware**. So the case cannot arise.
    ///
    /// ⚠ That is a true argument about another crate's walker, held here as a belief. If it
    /// ever stops being true — a sub-leaf slice, a store offset from a source that is not a
    /// PTE — the page-size predicate becomes load-bearing again and nothing would say so.
    /// ⇒ counted, printed, and **not refused**: a nonzero here is a finding about the
    /// walker, not a reason to drop a mapping the guest needs.
    offset_less_aligned_than_len: AtomicU64,
}

impl core::fmt::Debug for StoreMapPort {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StoreMapPort")
            .field("id", &self.id)
            .field("obj", &self.obj)
            .field("obj_len", &self.obj_len)
            .field("adopts", &self.adopts.load(Ordering::Relaxed))
            .field("maps", &self.maps.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl StoreMapPort {
    /// Build the port. ⊘ `crate::scratchpad::Scratchpad::share_for_store_maps` is the only
    /// constructor path, because it is the only place that knows the reservation is held.
    #[must_use]
    pub fn new(
        iso: std::sync::Arc<SharedIsolate>,
        id: IsolateId,
        obj: HostHandle,
        obj_len: u64,
    ) -> StoreMapPort {
        StoreMapPort {
            iso,
            id,
            obj,
            obj_len,
            adopted: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            placed: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            adopts: AtomicU64::new(0),
            adopt_refused: AtomicU64::new(0),
            maps: AtomicU64::new(0),
            map_refused: AtomicU64::new(0),
            unmaps: AtomicU64::new(0),
            unmap_refused: AtomicU64::new(0),
            bytes_mapped: AtomicU64::new(0),
            chan_births: AtomicU64::new(0),
            chan_birth_refused: AtomicU64::new(0),
            chan_born: AtomicU64::new(0),
            asserted: AtomicU64::new(0),
            assert_refused: AtomicU64::new(0),
            declined_on_vcpu: AtomicU64::new(0),
            birth_clients: AtomicU64::new(0),
            birth_refused: AtomicU64::new(0),
            birth_procs: std::sync::Mutex::new(std::collections::BTreeSet::new()),
            first_refusal: std::sync::Mutex::new(None),
            refusal_kinds: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            refusal_kinds_dropped: AtomicU64::new(0),
            replaced: AtomicU64::new(0),
            offset_less_aligned_than_len: AtomicU64::new(0),
        }
    }

    /// ★★★★★ **TAKE A PER-PROC ISOLATE'S BARE ADDRESS SPACE.** Idempotent: a space already
    /// adopted answers with the range it already has, because duping twice would give the
    /// scratchpad two references to one space and `0x19` on the second range.
    ///
    /// # Errors
    /// [`StoreMapRefusal`], by name.
    ///
    /// # Panics
    /// Through `Worker::with_rm`'s `assert_lock_free`, if a caller reaches this holding a
    /// ranked lock. That is the invariant, not a bug to be caught.
    pub fn adopt(&self, bare: BareVaSpace) -> Result<HostHandle, StoreMapRefusal> {
        let key = bare.space.raw();
        if let Some(h) = self
            .adopted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&key)
        {
            return Ok(*h);
        }
        let Ok(space) = u32::try_from(bare.space.raw()) else {
            return Err(self.note(StoreMapRefusal::BadHandle {
                raw: bare.space.raw(),
            }));
        };
        let off = self.off_vcpu()?;
        let out = self
            .iso
            .with_worker(|worker| worker.with_rm(&off, |rm| rm.adopt_vaspace(bare.client, space)));
        match out {
            None => Err(self.note(StoreMapRefusal::NoWorker)),
            Some(Err(e)) => {
                self.adopt_refused.fetch_add(1, Ordering::Relaxed);
                Err(self.note(StoreMapRefusal::Rm(format!("{e:?}"))))
            }
            Some(Ok(range)) => {
                self.adopts.fetch_add(1, Ordering::Relaxed);
                self.adopted
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(key, range);
                Ok(range)
            }
        }
    }

    /// ★★★★★ **CONSTRAINT 32 — HAND A BIRTH CLIENT TO THE SCRATCHPAD.**
    ///
    /// The descriptors and the handle came from a per-proc isolate's
    /// [`kayfabe_isolate::RmBackend::mint_birth_client`]; this puts them where every later
    /// store-mapping escape for that proc will find them.
    ///
    /// ⊘ **Idempotent by REFUSAL, not by replacement.** A second hand-over for one proc is
    /// refused at the far side (`BIRTH_CLIENT_ALREADY_HELD`) rather than overwriting, because
    /// replacing drops a live connection — closing I's descriptors and orphaning every range
    /// already placed through it — while callers holding a range handle from the old one
    /// carry on naming it. This side therefore asks **once**, and the counter says so.
    ///
    /// # Errors
    /// [`StoreMapRefusal`], by name.
    ///
    /// # Panics
    /// Through `Worker::with_rm`'s `assert_lock_free`, if a caller reaches this holding a
    /// ranked lock. That is the invariant, not a bug to be caught.
    pub fn adopt_birth_client(
        &self,
        minted: kayfabe_isolate::MintedBirthClient,
        minted_by_proc: u32,
    ) -> Result<(), StoreMapRefusal> {
        let off = self.off_vcpu()?;
        let isolate_client = minted.isolate_client;
        let client = u64::from(minted.client);
        let (ctl, node) = (minted.ctl, minted.node);
        let out = self.iso.with_worker(move |worker| {
            worker.with_rm(&off, move |rm| {
                rm.adopt_birth_client(client, isolate_client, minted_by_proc, ctl, node)
            })
        });
        match out {
            None => {
                self.birth_refused.fetch_add(1, Ordering::Relaxed);
                Err(self.note(StoreMapRefusal::NoWorker))
            }
            Some(Err(e)) => {
                self.birth_refused.fetch_add(1, Ordering::Relaxed);
                Err(self.note(StoreMapRefusal::Rm(format!("{e:?}"))))
            }
            Some(Ok(())) => {
                self.birth_clients.fetch_add(1, Ordering::Relaxed);
                self.birth_procs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(minted_by_proc);
                Ok(())
            }
        }
    }

    /// ★★★ **CONSTRAINT 32's CENSUS ROW** — handed, refused, and **how many procs**.
    ///
    /// ⊘ Three numbers, because two of them answer different questions and the third catches
    /// the failure that reads as success: one proc's birth client working while the others'
    /// silently fall back to the cross-client path is a partial arm that looks like a flaky
    /// GPU.
    #[must_use]
    pub fn birth_census(&self) -> (u64, u64, usize) {
        (
            self.birth_clients.load(Ordering::Relaxed),
            self.birth_refused.load(Ordering::Relaxed),
            self.birth_procs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
        )
    }

    /// ★★★★★ **w755r — THE CHANNEL-BIRTH CENSUS: asked, refused, borne.**
    ///
    /// ⊘⊘⊘ **This row exists because of what w755q could not say.** That boot reported
    /// `P1 … NEVER RETIRED` and every plumbing counter green, and it took reading three
    /// different log streams to establish that the route under test had **never executed**.
    /// A route with no census of its own cannot distinguish *"it ran and failed"* from
    /// *"it never ran"*, and those have opposite fixes.
    ///
    /// ⇒ Read it as a triple, never as one number:
    /// - `asked = 0` — **the route never fired.** Nothing downstream of it has been tested,
    ///   whatever else the boot says. Look at the USERD discriminant and at whether a birth
    ///   party was installed at realize.
    /// - `asked > 0, borne = 0` — it fired and RM (or this port) refused every time. The
    ///   refusals name themselves; read those.
    /// - `borne > 0` — channels are being born in B over the guest's own ring AND USERD.
    ///   ★ This is the first number in the campaign that means the guest's cursor is the one
    ///   hardware reads.
    #[must_use]
    pub fn chan_birth_census(&self) -> (u64, u64, u64) {
        (
            self.chan_births.load(Ordering::Relaxed),
            self.chan_birth_refused.load(Ordering::Relaxed),
            self.chan_born.load(Ordering::Relaxed),
        )
    }

    /// Does this port already hold a birth client for `proc`?
    #[must_use]
    pub fn has_birth_client(&self, proc: u32) -> bool {
        self.birth_procs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&proc)
    }

    /// ★★★★★ **MAP `[offset, offset+len)` OF THE ONE OBJECT AT THE GUEST'S OWN VA.**
    ///
    /// # Errors
    /// [`StoreMapRefusal`], by name. ⚠ `Rm` here includes `PlacementRefused` — constraint
    /// 28's assertion, made inside the isolate at the one `NVOS46` site — so a mapping RM
    /// relocated arrives as a refusal rather than as an `Ok` naming the wrong address.
    pub fn map(
        &self,
        vas: HostHandle,
        offset: u64,
        len: u64,
        at: GpuVa,
    ) -> Result<u64, StoreMapRefusal> {
        // ⊘ The bounds check is OURS. RM maps whatever offset it is handed; a run past the
        // reservation's end is a guest range pointed at memory this object does not cover,
        // and it would fault later, somewhere else, as somebody else's bug.
        if offset.checked_add(len).is_none_or(|end| end > self.obj_len) {
            self.map_refused.fetch_add(1, Ordering::Relaxed);
            return Err(self.note(StoreMapRefusal::OutOfRange {
                offset,
                len,
                obj_len: self.obj_len,
            }));
        }
        // ★★★ w755 — see `Self::offset_less_aligned_than_len`. A count, never a refusal.
        if len > 0 && len.is_power_of_two() && offset % len != 0 {
            self.offset_less_aligned_than_len
                .fetch_add(1, Ordering::Relaxed);
        }
        // ★★★★★ **w755d — THE LEDGER IS CONSULTED BEFORE THE MAP, NOT ONLY WRITTEN AFTER
        // IT.**
        //
        // ⊘⊘⊘ `[measured w755k, route-K boot]` `maps=45 map_refused=11` with
        // `refusals=[11x Rm("NoMemory")]`, and **45 + 11 = 56**, the exact number of map
        // attempts that boot made. `unmaps=0` — nothing is ever taken down — so eleven of
        // the fifty-six were the route re-publishing a leaf this port had ALREADY placed.
        // RM then answered `0x51` on a FIXED map, which
        // `[C: src/qemu/nvkvm_gpu_emul.c:7935]` records as *"the VA is ALREADY mapped in the
        // host VASpace"* and NOT as capacity.
        //
        // ⚠ This ledger existed the whole time and was **write-only**: `placed` was inserted
        // on success and read by `is_slice_of_the_store`, never by the mapper. A publish
        // route that legitimately re-offers a leaf (which is its documented behaviour on a
        // declined vCPU attempt) therefore paid an IPC round trip and an RM refusal per
        // repeat.
        //
        // ★ Re-offering the SAME slice is idempotent and answers the VA already held. A
        // re-offer naming a DIFFERENT `(offset, len)` at the same VA is NOT idempotent — it
        // is two memories at one address, the state the single store exists to abolish — so
        // it is refused by name rather than silently re-placed.
        {
            let placed = self
                .placed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(prev) = placed.get(&(vas.raw(), at.0)) {
                if prev.offset == offset && prev.len == len {
                    return Ok(at.0);
                }
                let prev = *prev;
                drop(placed);
                // ★★★★★ **w755g — THE RE-POINT: UNMAP THE OLD SLICE, THEN MAP THE NEW.**
                //
                // ⊘⊘⊘ This arm used to refuse `AlreadyPlacedDifferently`. That is right for
                // *two memories at one address* and **wrong for a legitimate re-point**, and
                // the two were indistinguishable only because **nothing ever unmapped**:
                // `StoreMapPort::unmap` was fully implemented and had ZERO production callers
                // (`[measured w755k]` `maps=45 unmaps=0 outstanding=45`).
                //
                // ⚠ **It is also the leak constraint 27 exists to prevent, and this file's own
                // sibling named it before anyone wrote this code** (`rm.rs:7318`): *"the
                // refresh reports the guest's TLB invalidate complete, the guest kernel reuses
                // the physical page for another of its own userspace processes, and the old
                // process can still reach it through a slice we told the guest was gone — a
                // cross-process leak INSIDE the guest, caused by us, invisible to the guest."*
                // We had the never-issued form of exactly that.
                //
                // ★ **UNMAP FIRST, and the order is the constraint, not a preference.** §27
                // requires unmaps ordered before maps within one refresh. Doing it in this one
                // call makes the window in which both could be live **not exist**, rather than
                // making it small.
                //
                // ⊘ **FAIL-CLOSED.** If the unmap is refused we do NOT map: a second slice
                // placed over a live one is the two-memories-at-one-address state, and
                // "the old one is probably gone" is the assumption this whole defect was.
                self.replaced.fetch_add(1, Ordering::Relaxed);
                if let Err(e) = self.unmap(vas, at) {
                    self.map_refused.fetch_add(1, Ordering::Relaxed);
                    return Err(self.note(StoreMapRefusal::ReplaceUnmapRefused {
                        at: at.0,
                        had_offset: prev.offset,
                        had_len: prev.len,
                        want_offset: offset,
                        want_len: len,
                        why: format!("{e:?}"),
                    }));
                }
            }
        }
        let off = self.off_vcpu()?;
        let obj = self.obj;
        let out = self.iso.with_worker(|worker| {
            worker.with_rm(&off, |rm| rm.map_store_slice(vas, obj, offset, len, at))
        });
        match out {
            None => Err(self.note(StoreMapRefusal::NoWorker)),
            Some(Err(e)) => {
                self.map_refused.fetch_add(1, Ordering::Relaxed);
                // ★★★★★ w755 — constraint 28 gets its OWN arm. See `StoreMapRefusal::Placement`.
                Err(self.note(match e {
                    kayfabe_isolate::RmError::PlacementRefused { want, got } => {
                        StoreMapRefusal::Placement { want, got }
                    }
                    other => StoreMapRefusal::Rm(format!("{other:?}")),
                }))
            }
            Some(Ok(va)) => {
                self.maps.fetch_add(1, Ordering::Relaxed);
                self.bytes_mapped.fetch_add(len, Ordering::Relaxed);
                self.placed
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert((vas.raw(), at.0), Placed { offset, len });
                Ok(va)
            }
        }
    }

    /// ★★★★★ **TAKE ONE SLICE BACK DOWN — and the `Result` is constraint 27's barrier.**
    ///
    /// ⚠ The ledger entry is removed **only on success**. A refused unmap leaves the range
    /// recorded as placed, which is what makes [`StoreMapPort::outstanding`] an honest
    /// answer to *"is anything still mapped?"* and therefore an honest input to the
    /// invalidate's completion.
    ///
    /// # Errors
    /// [`StoreMapRefusal`], by name.
    pub fn unmap(&self, vas: HostHandle, at: GpuVa) -> Result<(), StoreMapRefusal> {
        let off = self.off_vcpu()?;
        let out = self
            .iso
            .with_worker(|worker| worker.with_rm(&off, |rm| rm.unmap_store_slice(vas, at)));
        match out {
            None => Err(self.note(StoreMapRefusal::NoWorker)),
            Some(Err(e)) => {
                self.unmap_refused.fetch_add(1, Ordering::Relaxed);
                Err(self.note(match e {
                    kayfabe_isolate::RmError::PlacementRefused { want, got } => {
                        StoreMapRefusal::Placement { want, got }
                    }
                    other => StoreMapRefusal::Rm(format!("{other:?}")),
                }))
            }
            Some(Ok(())) => {
                self.unmaps.fetch_add(1, Ordering::Relaxed);
                self.placed
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&(vas.raw(), at.0));
                Ok(())
            }
        }
    }

    /// ★★★★★ **THE RESTATED ASSERTION — "IS THIS RING A SLICE OF THE ONE OBJECT?"**
    ///
    /// See the module docs for why this replaces `RING_NOT_A_JOINED_WINDOW` rather than
    /// joining it. It answers `true` only if **this** port placed a slice at exactly `at` in
    /// `vas`, out of the one reserved object, covering at least `len` bytes.
    ///
    /// ⊘ **`at` must match exactly, not merely be covered.** A ring that starts inside
    /// somebody else's slice is a ring whose own extent nobody has stated, and the failure
    /// being prevented — a channel that fetches zeros and never advances `GP_GET` — is
    /// precisely the one that a *partly* correct mapping still produces.
    #[must_use]
    pub fn is_slice_of_the_store(&self, vas: HostHandle, at: GpuVa, len: u64) -> bool {
        self.asserted.fetch_add(1, Ordering::Relaxed);
        let ok = self
            .placed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&(vas.raw(), at.0))
            .is_some_and(|p| p.len >= len);
        if !ok {
            self.assert_refused.fetch_add(1, Ordering::Relaxed);
        }
        ok
    }

    /// ★★★★★ **w755r, ROUTE K INCREMENT 7 — CARRY THE GUEST'S CHANNEL BIRTH INTO B.**
    ///
    /// `[measured w755q]` the per-proc isolate refused this shape **11 times**
    /// (`USERD_IN_STORE_NEEDS_BIRTH_IN_B`) while `birth_in_b` was **never entered**: the two
    /// halves were in different processes and nothing carried the request across. This is
    /// the crossing.
    ///
    /// ⊘ `host_vas` is the **per-proc isolate's** space handle, and this port's `adopted`
    /// ledger is the only place the corresponding range **inside B** is written down
    /// (`adopt_space` → `remember_birth_range`). ⇒ resolved here rather than by the caller,
    /// for the reason `the_handover_rederived_a_routing_key` names: a hand-over that
    /// re-derives a key its caller already holds gets one of the two wrong eventually.
    ///
    /// # Errors
    /// [`kayfabe_fwd::FwdFault`], by name — a missing adoption and an RM refusal are
    /// different facts with different fixes, so they are different variants and not one
    /// string.
    ///
    /// # Panics
    /// Through `Worker::with_rm`'s `assert_lock_free`, if a caller reaches this holding a
    /// ranked lock. That is the invariant, not a bug to be caught.
    pub fn birth_over_the_store(
        &self,
        host_vas: HostHandle,
        engine: kayfabe_isolate::ChannelEngine,
        ring: kayfabe_isolate::AdoptedGuestRing,
        err_notifier: Option<kayfabe_isolate::GuestRamGrant>,
    ) -> Result<(HostHandle, u64), kayfabe_fwd::FwdFault> {
        self.chan_births.fetch_add(1, Ordering::Relaxed);
        let off = self.off_vcpu().map_err(|r| {
            self.chan_birth_refused.fetch_add(1, Ordering::Relaxed);
            kayfabe_fwd::FwdFault::Rm {
                err: kayfabe_isolate::RmError::Other(kayfabe_isolate::STORE_BIRTH_REFUSED),
                on: None,
            }
        })?;
        // ★ The range inside B, from the ledger that placed every slice for this proc.
        let Some(range) = self
            .adopted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&host_vas.raw())
            .copied()
        else {
            self.chan_birth_refused.fetch_add(1, Ordering::Relaxed);
            eprintln!(
                "kayfabe: STORE-BIRTH ⊘⊘ REFUSED host_vas={:#x} — this port never adopted \
                 that space, so there is no range inside B to birth in. ⊘ A store-slice USERD \
                 in a space this port did not adopt means the ring was not placed by us \
                 either, and a channel born over it would fetch from nothing.",
                host_vas.raw(),
            );
            return Err(kayfabe_fwd::FwdFault::Rm {
                err: kayfabe_isolate::RmError::Other(kayfabe_isolate::NO_STORE_BIRTH_PARTY),
                on: None,
            });
        };
        let out = self.iso.with_worker(move |worker| {
            worker.with_rm(&off, move |rm| {
                rm.birth_guest_channel_in_b(
                    range,
                    engine,
                    ring,
                    err_notifier,
                )
            })
        });
        match out {
            None => {
                self.chan_birth_refused.fetch_add(1, Ordering::Relaxed);
                Err(kayfabe_fwd::FwdFault::Rm {
                    err: kayfabe_isolate::RmError::Other(kayfabe_isolate::STORE_BIRTH_REFUSED),
                    on: None,
                })
            }
            Some(Err(e)) => {
                self.chan_birth_refused.fetch_add(1, Ordering::Relaxed);
                eprintln!("kayfabe: STORE-BIRTH ⊘⊘ REFUSED by the scratchpad — {e:?}");
                Err(kayfabe_fwd::FwdFault::Rm { err: e, on: None })
            }
            Some(Ok(born)) => {
                self.chan_born.fetch_add(1, Ordering::Relaxed);
                eprintln!(
                    "kayfabe: STORE-BIRTH ✔ BORN IN B host_vas={:#x} range={:#x} \
                     engine={engine:?} → channel={:?} \
                     token={:#x} ⇒ the channel \
                     carries the GUEST'S OWN ring AND USERD, so hardware reads the guest's \
                     cursor and we are never in the GP_PUT path",
                    host_vas.raw(),
                    range.raw(),
                    born.0,
                    born.1,
                );
                Ok(born)
            }
        }
    }

    /// Where in the reserved object the slice at `at` lives, if this port placed one.
    #[must_use]
    pub fn slice_offset(&self, vas: HostHandle, at: GpuVa) -> Option<u64> {
        self.placed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&(vas.raw(), at.0))
            .map(|p| p.offset)
    }

    /// The reserved object this port maps out of — what a binding must name as its arena.
    #[must_use]
    pub fn object(&self) -> HostHandle {
        self.obj
    }

    /// How many slices are still mapped. ⊘ The ledger's length, not `maps - unmaps`: that
    /// arithmetic is wrong the moment one unmap is refused.
    #[must_use]
    pub fn outstanding(&self) -> usize {
        self.placed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// ★★★★★ **DECLINE BY NAME RATHER THAN PANIC.** See [`StoreMapRefusal::OnVcpu`].
    ///
    /// ⊘ Checked **before** `OffTrap::claim`, never after: `claim` asserts, and an assert is
    /// not a refusal — it is a VMM abort, which is a guest-visible crash caused by the rule
    /// that exists to stop a guest-visible stall.
    fn off_vcpu(&self) -> Result<kayfabe_util::trapwitness::OffTrap, StoreMapRefusal> {
        if kayfabe_util::lockwitness::on_vcpu_thread() || kayfabe_util::trapwitness::in_trap() {
            self.declined_on_vcpu.fetch_add(1, Ordering::Relaxed);
            return Err(self.note(StoreMapRefusal::OnVcpu));
        }
        Ok(kayfabe_util::trapwitness::OffTrap::claim(
            "a scratchpad store mapping",
        ))
    }

    fn note(&self, why: StoreMapRefusal) -> StoreMapRefusal {
        let detail = why.detail();
        {
            let mut first = self
                .first_refusal
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if first.is_none() {
                *first = Some(detail.clone());
            }
        }
        // ★★★★★ w755 — and the histogram, because one string cannot characterise 2 154
        // refusals. See `Self::refusal_kinds`.
        let mut kinds = self
            .refusal_kinds
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(n) = kinds.get_mut(&detail) {
            *n += 1;
        } else if kinds.len() < Self::DISTINCT_CAP {
            kinds.insert(detail, 1);
        } else {
            self.refusal_kinds_dropped.fetch_add(1, Ordering::Relaxed);
        }
        why
    }

    /// How many distinct refusal strings [`Self::refusal_kinds`] will hold before it starts
    /// counting drops instead. ⊘ Bounded because the key is guest-influenced.
    pub const DISTINCT_CAP: usize = 24;

    /// ★ The distinct refusals seen, most frequent first, with the drop count appended when
    /// the histogram is a sample rather than a census.
    #[must_use]
    pub fn refusal_histogram(&self) -> String {
        let kinds = self
            .refusal_kinds
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if kinds.is_empty() {
            return "none".to_string();
        }
        let mut rows: Vec<(&String, &u64)> = kinds.iter().collect();
        rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let mut out = rows
            .iter()
            .map(|(k, n)| format!("{n}x {k}"))
            .collect::<Vec<_>>()
            .join(" | ");
        let dropped = self.refusal_kinds_dropped.load(Ordering::Relaxed);
        if dropped > 0 {
            out.push_str(&format!(
                " | ⚠ SAMPLE not census: {dropped} further distinct refusals exceeded the                  {} key cap",
                Self::DISTINCT_CAP
            ));
        }
        out
    }

    /// One line for a boot log. ⊘ Printed even when every number is zero: *"the port never
    /// ran"* and *"nobody looked"* are different facts and only the line's presence
    /// distinguishes them.
    #[must_use]
    pub fn census_line(&self) -> String {
        let adopts = self.adopts.load(Ordering::Relaxed);
        let maps = self.maps.load(Ordering::Relaxed);
        let asserted = self.asserted.load(Ordering::Relaxed);
        let assert_refused = self.assert_refused.load(Ordering::Relaxed);
        let verdict = if adopts == 0 && maps == 0 {
            "⊘⊘ VACUOUS — the scratchpad mapped NOTHING. Under KAYFABE_VAS_OWNER=scratchpad \
             this means the publish path never reached the port, NOT that there was nothing \
             to map"
        } else if assert_refused > 0 {
            "⚠ a ring was offered that this port had NOT placed — read RING-NOT-A-SLICE"
        } else if self.map_refused.load(Ordering::Relaxed) > 0 {
            "⚠ mappings were refused"
        } else {
            "★ every adopt and every map succeeded"
        };
        let (bc, br, bp) = self.birth_census();
        // ★★★★★ **CONSTRAINT 32's ROW, AND IT NAMES ITS OWN VACUITY.** ⊘ `birth_refused=0`
        // alone is `a_refusal_counter_read_as_absent_demand`: it reads as "nothing was
        // refused" when it may mean "nothing was asked". So the row carries `handed`,
        // `refused` AND `procs`, and says in words which of the three states it is in.
        let birth_verdict = if bc == 0 && br == 0 {
            "⊘⊘ NOT ASKED — this is the `scratchpad` arm, or the publish path never reached \
             the mint. It is NOT evidence that route K failed"
        } else if bc == 0 {
            "⊘⊘⊘ EVERY BIRTH CLIENT REFUSED — every dup below took the CROSS-CLIENT path and \
             RM's `InsufficientPermissions` is this line's consequence, not a new finding"
        } else if br > 0 {
            "⚠ PARTIAL — some procs have a birth client and some do not. The ones that do not \
             are silently on the cross-client path; this is NOT a flaky GPU"
        } else {
            "★ every birth client was handed over"
        };
        let (cba, cbr, cbb) = self.chan_birth_census();
        // ★★★★★ **w755r's ROW, AND IT NAMES ITS OWN VACUITY TOO** — for the reason w755q
        // paid for: a boot reported the gate red while the route under test had never run,
        // and no single counter could have said so.
        let chan_birth_verdict = if cba == 0 {
            "⊘⊘⊘ NEVER FIRED — no guest channel declared a store-slice USERD, or no birth \
             party was installed. ⚠ Nothing downstream of this route has been tested by this \
             boot, whatever else it says"
        } else if cbb == 0 {
            "⊘⊘ ASKED AND REFUSED EVERY TIME — the route fired; read the named refusals above"
        } else if cbr > 0 {
            "⚠ PARTIAL — some guest channels were born in B and some were refused. The \
             refused ones have a USERD of nobody's and will never advance GP_GET"
        } else {
            "★★★ EVERY STORE-USERD CHANNEL WAS BORN IN B — hardware reads the GUEST'S cursor"
        };
        format!(
            "STORE-BIRTH asked={cba} refused={cbr} born={cbb} ⇒ {chan_birth_verdict}\n\
             STORE-MAP-K handed={bc} refused={br} procs={bp} ⇒ {birth_verdict}\n\
             STORE-MAP iso={:?} obj={:?} obj_len={} adopts={adopts} adopt_refused={} \
             maps={maps} map_refused={} unmaps={} unmap_refused={} bytes_mapped={} \
             outstanding={} asserted={asserted} assert_refused={assert_refused} \
             declined_on_vcpu={} ragged_offset={} replaced={} first_refusal=[{}] refusals=[{}] \
             ⇒ {verdict}",
            self.id,
            self.obj,
            self.obj_len,
            self.adopt_refused.load(Ordering::Relaxed),
            self.map_refused.load(Ordering::Relaxed),
            self.unmaps.load(Ordering::Relaxed),
            self.unmap_refused.load(Ordering::Relaxed),
            self.bytes_mapped.load(Ordering::Relaxed),
            self.outstanding(),
            self.declined_on_vcpu.load(Ordering::Relaxed),
            self.offset_less_aligned_than_len.load(Ordering::Relaxed),
            self.replaced.load(Ordering::Relaxed),
            self.first_refusal
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_deref()
                .unwrap_or("none"),
            self.refusal_histogram(),
        )
    }
}

/// ★★★★★ **CONSTRAINT 26 — THE PORT *IS* THE ORACLE.**
///
/// ⊘ The seam and the ledger are the same object deliberately: an oracle that read a copy of
/// the mapper's state would be a second source of truth for *"what is mapped"*, and the one
/// question it exists to answer is exactly that. See [`kayfabe_fwd::RingSliceOracle`].
impl kayfabe_fwd::RingSliceOracle for StoreMapPort {
    fn is_slice_of_the_store(&self, vas: HostHandle, at: GpuVa, len: u64) -> bool {
        StoreMapPort::is_slice_of_the_store(self, vas, at, len)
    }
}

/// ★★★★★ **w755r, CONSTRAINT 32 — THE PORT IS ALSO THE BIRTH PARTY, for the oracle's reason.**
///
/// The port already owns the per-proc-space → adopted-range ledger — it is the same ledger
/// that places every store slice — and `birth_in_b` is keyed on exactly that range. A second
/// object holding a copy would be a second source of truth for *"which range is this proc's
/// inside B"*, which is the shape `a_second_source_of_truth_beside_a_complete_value` names.
impl kayfabe_fwd::StoreChannelBirth for StoreMapPort {
    fn birth_over_the_store(
        &self,
        host_vas: HostHandle,
        engine: kayfabe_isolate::ChannelEngine,
        ring: kayfabe_isolate::AdoptedGuestRing,
        err_notifier: Option<kayfabe_isolate::GuestRamGrant>,
    ) -> Result<(HostHandle, u64), kayfabe_fwd::FwdFault> {
        StoreMapPort::birth_over_the_store(
            self,
            host_vas,
            engine,
            ring,
            err_notifier,
        )
    }
}

kayfabe_util::assert_send_sync!(StoreMapPort);

/// ★★★ **THE PER-GPU REGISTRY — how the publish path reaches this device's port.**
///
/// ⊘⊘ **Keyed by `GpuId`, and that is the w637 fix rather than the w637 defect.** One process
/// hosts two emulated GPUs as two device instances; a bare `static` port would bind to
/// whichever realized first and device 1's leaves would be mapped into device 0's
/// reservation — silently, because both are valid handles. A map keyed on the GPU cannot do
/// that.
///
/// ⊘ **`Weak`**, so the registry never keeps a retired scratchpad's isolate alive: the
/// `Scratchpad` owns the `Arc` and drops it in `retire`, and a lookup afterwards answers
/// `None` rather than handing out a port whose isolate has been told to go away.
///
/// ⚠ It exists because `join_one_fb_leaf` is reached from **three different types**
/// (`SharedDoorbell`, `PublishContext`, `Regs`) and threading a handle through all three
/// would be three places the port's identity lives. The registry is one.
static STORE_MAP_PORTS: std::sync::Mutex<
    std::collections::BTreeMap<u32, std::sync::Weak<StoreMapPort>>,
> = std::sync::Mutex::new(std::collections::BTreeMap::new());

/// Publish this device's store-map port. Called once, at realize, beside the reservation.
pub fn register_store_map_port(gpu: kayfabe_rt::GpuId, port: &std::sync::Arc<StoreMapPort>) {
    STORE_MAP_PORTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(gpu.0, std::sync::Arc::downgrade(port));
}

/// This device's store-map port, or `None` when none was built or the scratchpad has retired.
#[must_use]
pub fn store_map_port(gpu: kayfabe_rt::GpuId) -> Option<std::sync::Arc<StoreMapPort>> {
    STORE_MAP_PORTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&gpu.0)
        .and_then(std::sync::Weak::upgrade)
}
