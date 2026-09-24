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
    /// ★★★★★ **§18 — the guest-RAM object does not exist**, and `why` says whether it was
    /// never asked for (no guest RAM armed), is being built right now, or RM refused it.
    /// ⊘ Never degraded to a per-page pin: that path is the one this object retires.
    NoGuestRamObject { why: String },
    /// RM refused, verbatim.
    Rm(String),
    /// ★ The address space was never handed over, so this port has no `hDma` for it.
    NotAdopted { space: u64 },
    /// ★★★★★ **w757 — a VA space was offered for release and something still holds it.**
    ///
    /// ⊘ `why` names WHICH of the three conditions failed, and `count` how much of it is
    /// left. A single *"still held"* would send a reader to check all three — the
    /// one-refusal-for-several-causes shape this tree keeps paying for.
    VasStillHeld {
        /// The space, raw.
        vas: u64,
        /// `"mappings"` or `"table-referenced"`. ⊘ The third condition — *no channel uses
        /// it* — cannot appear here: it is the witness's precondition, so a caller without it
        /// never reaches this function.
        why: &'static str,
        /// How many of `why` remain.
        count: usize,
    },
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
            StoreMapRefusal::NoGuestRamObject { .. } => "NoGuestRamObject",
            StoreMapRefusal::Rm(_) => "Rm",
            StoreMapRefusal::NotAdopted { .. } => "NotAdopted",
            StoreMapRefusal::OutOfRange { .. } => "OutOfRange",
            StoreMapRefusal::OnVcpu => "OnVcpu",
            StoreMapRefusal::BadHandle { .. } => "BadHandle",
            StoreMapRefusal::Placement { .. } => "Placement",
            StoreMapRefusal::AlreadyPlacedDifferently { .. } => "AlreadyPlacedDifferently",
            StoreMapRefusal::ReplaceUnmapRefused { .. } => "ReplaceUnmapRefused",
            // ⊘ The CONDITION is part of the name, so a census row distinguishes a space held
            // by mappings from one held by its table without anyone opening the payload.
            StoreMapRefusal::VasStillHeld { why, .. } => match *why {
                "mappings" => "VasStillHeld(mappings)",
                _ => "VasStillHeld(table)",
            },
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

/// ★★★★★ §18 — WHICH ground truth a diff list's runs are slices of. Named by the caller at
/// the one door ([`StoreMapPort::apply_ops`]), so the diff list stays the only executor for
/// BOTH objects rather than growing a second mutator for the second one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliceOf {
    /// The ONE reserved video-memory object (GPGA); a run's `gpga` is an offset into it.
    Store,
    /// The ONE guest-RAM object; a run's `gpga` field carries the memfd FILE OFFSET, and
    /// `bytes` is the block's extent (used only to build the object the first time).
    GuestRam { bytes: u64 },
}

/// The resolved object behind a [`SliceOf`]. Private: callers name the ground truth, never a
/// handle.
#[derive(Debug, Clone, Copy)]
struct SliceTarget {
    obj: HostHandle,
    len: u64,
    ram: bool,
}

/// ★★★★★ §18 — where the one guest-RAM object stands.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RamObject {
    /// Nobody has asked for it yet.
    NotAsked,
    /// A publication is building it right now; a concurrent caller is told so, by name,
    /// rather than building a second object.
    Asking,
    /// Held: the scratchpad's `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` over the whole block.
    Held { obj: HostHandle, len: u64 },
    /// Refused once; never re-asked (a retry per publish is the per-row pin storm again,
    /// one level up).
    Refused(String),
}

/// ★ The bounds rule for a slice of the guest-RAM object, kept a free function so it is
/// testable with no GPU: `[file_offset, file_offset + len)` must lie inside `[0, obj_len)`,
/// and a zero-length slice is refused rather than mapped as nothing.
///
/// # Errors
/// [`StoreMapRefusal::OutOfRange`], carrying the numbers.
pub fn ram_slice_in_bounds(
    file_offset: u64,
    len: u64,
    obj_len: u64,
) -> Result<(), StoreMapRefusal> {
    if len == 0 || file_offset.checked_add(len).is_none_or(|end| end > obj_len) {
        return Err(StoreMapRefusal::OutOfRange {
            offset: file_offset,
            len,
            obj_len,
        });
    }
    Ok(())
}

/// ★ Merge guest-RAM rows `(va, file_offset, len)` into maximal runs that are contiguous in
/// BOTH the guest VA and the file, so one FIXED map covers what the guest mapped as many
/// pages. Rows must arrive in VA order (the address table iterates in VA order); an
/// out-of-order or overlapping row simply starts a new run.
#[must_use]
pub fn coalesce_ram_rows(rows: &[(u64, u64, u64)]) -> Vec<(u64, u64, u64)> {
    let mut out: Vec<(u64, u64, u64)> = Vec::new();
    for &(va, off, len) in rows {
        if len == 0 {
            continue;
        }
        if let Some(last) = out.last_mut()
            && last.0.checked_add(last.2) == Some(va)
            && last.1.checked_add(last.2) == Some(off)
            && let Some(n) = last.2.checked_add(len)
        {
            last.2 = n;
            continue;
        }
        out.push((va, off, len));
    }
    out
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
    ///
    /// ⊘⊘⊘ **w825 — keyed by the WHOLE handle, isolate included, never by `.raw()`.**
    /// Each per-proc isolate is its own RM client, so handle NUMBERS repeat across procs.
    /// `[measured w825base --cross-client-leak]` two guest clients' spaces were both
    /// `0xcafe0004`; keyed by the number, proc 2 was handed proc 1's range, both host
    /// channels ran in ONE address space, B's FIXED map at the shared VA replaced A's, and
    /// A's next copy landed in B's object — a cross-client write the isolation rung caught.
    adopted: std::sync::Mutex<std::collections::BTreeMap<HostHandle, HostHandle>>,
    /// `(scratchpad range, guest VA) -> what was placed there`. **The ledger the restated
    /// ring assertion reads.**
    placed: std::sync::Mutex<std::collections::BTreeMap<(u64, u64), Placed>>,
    /// ★★★★★ **§18 — the SECOND ground truth: guest RAM as ONE RM object.** Built once,
    /// lazily, off-vCPU, from the scratchpad's spawn-time guest-RAM grant. See
    /// [`StoreMapPort::guest_ram_object`].
    ram: std::sync::Mutex<RamObject>,
    /// `(scratchpad range, guest VA) -> slice of the guest-RAM object placed there`.
    /// ⊘ A SEPARATE ledger from `placed`: `is_slice_of_the_store` and `slice_offset` read
    /// `placed` as "a slice of the GPGA object", and a RAM slice answering yes there would be
    /// a ring adopted over bytes that are not the store.
    ram_placed: std::sync::Mutex<std::collections::BTreeMap<(u64, u64), Placed>>,
    ram_maps: AtomicU64,
    ram_map_refused: AtomicU64,
    ram_bytes_mapped: AtomicU64,
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
    /// ★★★ w755u — doorbells carried to the channel's own isolate: asked, refused, rung.
    /// ⊘ Kept apart from the birth counters for their reason: a boot must be able to say
    /// whether the BIRTH or the RING is what stopped, and one counter cannot.
    /// ★★★ w757 — how many diff lists have been applied. ⊘ Its own counter: `maps` counts
    /// individual slices, and a boot must be able to say whether ONE list moved many slices or
    /// many lists moved none.
    /// ★★★ w757 — VA spaces released, and release attempts refused. ⊘ Both, for
    /// `a_refusal_counter_read_as_absent_demand`'s reason: `refused=0` alone cannot tell
    /// *"nothing was refused"* from *"nothing was asked"*.
    released: AtomicU64,
    release_refused: AtomicU64,
    applied: AtomicU64,
    /// ★★★ w758 — asked-for work DECLINED before it left this process (on a vCPU, inside a
    /// trap). ⊘ Its own counter: *"we did not ask"* and *"we were refused"* have different
    /// fixes, and a census that adds them reports a route that fired when none did.
    chan_declined: AtomicU64,
    chan_doorbells: AtomicU64,
    chan_doorbell_refused: AtomicU64,
    chan_rung: AtomicU64,
    /// ★ w783 — engine-object allocs routed here because the channel lives here.
    chan_engine_objects: AtomicU64,
    /// ★ w783 — of those, how many the scratchpad's RM refused. ⊘ A measured zero beside a
    /// nonzero `chan_engine_objects` is the reading that says the third ForeignHandle is
    /// closed; the pair is kept because `forwarded=0 refused=8` is how it was found.
    chan_engine_object_refused: AtomicU64,
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
    /// ★★★★★ §39(c): how large the guest's GPGA is. The single store IS the whole of guest
    /// vidmem, so the object's length is exactly the bound a reported leaf must lie inside.
    /// ⊘ Exposed so the WALK path can refuse an out-of-store run when the report is PARSED,
    /// rather than discovering it one `map()` at a time after the diff is already built.
    #[must_use]
    pub fn gpga_span(&self) -> u64 {
        self.obj_len
    }

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
            ram: std::sync::Mutex::new(RamObject::NotAsked),
            ram_placed: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            ram_maps: AtomicU64::new(0),
            ram_map_refused: AtomicU64::new(0),
            ram_bytes_mapped: AtomicU64::new(0),
            adopts: AtomicU64::new(0),
            adopt_refused: AtomicU64::new(0),
            maps: AtomicU64::new(0),
            map_refused: AtomicU64::new(0),
            unmaps: AtomicU64::new(0),
            unmap_refused: AtomicU64::new(0),
            bytes_mapped: AtomicU64::new(0),
            released: AtomicU64::new(0),
            release_refused: AtomicU64::new(0),
            applied: AtomicU64::new(0),
            chan_declined: AtomicU64::new(0),
            chan_doorbells: AtomicU64::new(0),
            chan_doorbell_refused: AtomicU64::new(0),
            chan_rung: AtomicU64::new(0),
            chan_engine_objects: AtomicU64::new(0),
            chan_engine_object_refused: AtomicU64::new(0),
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
        let key = bare.space;
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
    fn map(
        &self,
        target: SliceTarget,
        vas: HostHandle,
        offset: u64,
        len: u64,
        at: GpuVa,
    ) -> Result<u64, StoreMapRefusal> {
        // ⊘ The bounds check is OURS. RM maps whatever offset it is handed; a run past the
        // reservation's end is a guest range pointed at memory this object does not cover,
        // and it would fault later, somewhere else, as somebody else's bug.
        if offset.checked_add(len).is_none_or(|end| end > target.len) {
            self.count_map_refused(target);
            return Err(self.note(StoreMapRefusal::OutOfRange {
                offset,
                len,
                obj_len: target.len,
            }));
        }
        // ★★★ w755 — see `Self::offset_less_aligned_than_len`. A count, never a refusal.
        if !target.ram && len > 0 && len.is_power_of_two() && offset % len != 0 {
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
            // ★ w825 (§18) — TWO ledgers, one per ground truth, and a VA holds at most one
            // slice across BOTH: the guest may move a VA between guest RAM and vidmem, and RM
            // refuses a FIXED map over a live one (0x51). So "already placed" is asked of the
            // SAME object's ledger, and the re-point below fires for a slice of EITHER.
            let key = (vas.raw(), at.0);
            let (same_ledger, other_ledger) = if target.ram {
                (&self.ram_placed, &self.placed)
            } else {
                (&self.placed, &self.ram_placed)
            };
            let same = same_ledger
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&key)
                .copied();
            let other = other_ledger
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&key)
                .copied();
            if let (Some(prev), None) = (same, other) {
                if prev.offset == offset && prev.len == len {
                    return Ok(at.0);
                }
            }
            if let Some(prev) = same.or(other) {
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
                    self.count_map_refused(target);
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
        let obj = target.obj;
        let out = self.iso.with_worker(|worker| {
            worker.with_rm(&off, |rm| rm.map_store_slice(vas, obj, offset, len, at))
        });
        match out {
            None => Err(self.note(StoreMapRefusal::NoWorker)),
            Some(Err(e)) => {
                self.count_map_refused(target);
                // ★★★★★ w755 — constraint 28 gets its OWN arm. See `StoreMapRefusal::Placement`.
                Err(self.note(match e {
                    kayfabe_isolate::RmError::PlacementRefused { want, got } => {
                        StoreMapRefusal::Placement { want, got }
                    }
                    other => StoreMapRefusal::Rm(format!("{other:?}")),
                }))
            }
            Some(Ok(va)) => {
                if target.ram {
                    self.ram_maps.fetch_add(1, Ordering::Relaxed);
                    self.ram_bytes_mapped.fetch_add(len, Ordering::Relaxed);
                } else {
                    self.maps.fetch_add(1, Ordering::Relaxed);
                    self.bytes_mapped.fetch_add(len, Ordering::Relaxed);
                }
                self.ledger(target.ram)
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert((vas.raw(), at.0), Placed { offset, len });
                Ok(va)
            }
        }
    }

    /// The ledger for one ground truth: `placed` for the store, `ram_placed` for guest RAM.
    fn ledger(
        &self,
        ram: bool,
    ) -> &std::sync::Mutex<std::collections::BTreeMap<(u64, u64), Placed>> {
        if ram { &self.ram_placed } else { &self.placed }
    }

    fn count_map_refused(&self, target: SliceTarget) {
        if target.ram {
            self.ram_map_refused.fetch_add(1, Ordering::Relaxed);
        } else {
            self.map_refused.fetch_add(1, Ordering::Relaxed);
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
    /// ★★★★★ **§18 — THE ONE GUEST-RAM OBJECT.** Maps the scratchpad's spawn-time guest-RAM
    /// grant whole and describes it to RM as ONE `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR`, once per
    /// VM. `bytes` is the block's extent as the composition root adopted it.
    ///
    /// ⊘ Asked ONCE: a refusal is remembered and returned by name on every later call. The
    /// per-row pin path re-asked on every publish and `[measured w825base --engines]` served
    /// ~99 000 worker requests doing so while the guest's RM replies queued behind them.
    ///
    /// # Errors
    /// [`StoreMapRefusal::NoGuestRamObject`] / [`StoreMapRefusal::OnVcpu`] /
    /// [`StoreMapRefusal::NoWorker`], by name.
    pub fn guest_ram_object(&self, bytes: u64) -> Result<(HostHandle, u64), StoreMapRefusal> {
        {
            let mut g = self
                .ram
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match &*g {
                RamObject::Held { obj, len } => return Ok((*obj, *len)),
                RamObject::Asking => {
                    return Err(StoreMapRefusal::NoGuestRamObject {
                        why: "being built by another publication right now".to_string(),
                    });
                }
                RamObject::Refused(why) => {
                    return Err(StoreMapRefusal::NoGuestRamObject { why: why.clone() });
                }
                RamObject::NotAsked => {}
            }
            if bytes == 0 {
                return Err(StoreMapRefusal::NoGuestRamObject {
                    why: "no guest-RAM block was adopted (KAYFABE_GUEST_RAM unset?)".to_string(),
                });
            }
            *g = RamObject::Asking;
        }
        let off = match self.off_vcpu() {
            Ok(o) => o,
            Err(e) => {
                // ⊘ Not a refusal of the object: we did not ask. Back to NotAsked.
                *self
                    .ram
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = RamObject::NotAsked;
                return Err(e);
            }
        };
        let t0 = std::time::Instant::now();
        let out = self.iso.with_worker(|worker| {
            worker.with_rm(&off, |rm| {
                let grant = kayfabe_isolate::GuestRamGrant::originated_by_the_vmm(
                    0,
                    bytes,
                    kayfabe_vmm::Prot::ReadWrite,
                );
                let mapped = rm.map_guest_ram(grant)?;
                match rm.describe_guest_ram(mapped) {
                    Ok(obj) => Ok(obj),
                    Err(e) => {
                        let _ = rm.unmap_guest_ram(mapped);
                        Err(e)
                    }
                }
            })
        });
        let (next, ret) = match out {
            None => (
                RamObject::NotAsked,
                Err(self.note(StoreMapRefusal::NoWorker)),
            ),
            Some(Err(e)) => {
                let why = format!("RM refused the OS_DESCRIPTOR over {bytes:#x} bytes: {e:?}");
                (
                    RamObject::Refused(why.clone()),
                    Err(self.note(StoreMapRefusal::NoGuestRamObject { why })),
                )
            }
            Some(Ok(obj)) => (RamObject::Held { obj, len: bytes }, Ok((obj, bytes))),
        };
        eprintln!(
            "kayfabe: GUEST-RAM-OBJECT bytes={bytes:#x} in {} ms → {} ⇒ §18: guest RAM is ONE RM \
             object in the scratchpad; every sysmem row is a FIXED slice of it at the guest's \
             own VA. ⊘ Asked once per VM.",
            t0.elapsed().as_millis(),
            match &ret {
                Ok((obj, _)) => format!("★★★★★ HELD obj={obj:?}"),
                Err(e) => format!("⊘⊘ REFUSED {e:?}"),
            }
        );
        *self
            .ram
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = next;
        ret
    }

    /// ★★★★★ **§18 — map `[file_offset, file_offset+len)` of the guest-RAM object FIXED at
    /// `at` in the adopted range `vas`.** The sysmem twin of the store's `map`: same verb
    /// (`map_store_slice`, which takes the object as a parameter), same birth-client dup,
    /// same checked placement, its own ledger.
    ///
    /// Idempotent: an identical placement is answered from the ledger with no IPC — which is
    /// what lets every publish re-offer every row without a request storm.
    ///
    /// # Errors
    /// [`StoreMapRefusal`], by name.
    pub fn map_ram_slice(
        &self,
        vas: HostHandle,
        bytes: u64,
        file_offset: u64,
        len: u64,
        at: GpuVa,
    ) -> Result<u64, StoreMapRefusal> {
        // ⊘ The zero-length / overflow rule is checked here, before the batch: `apply_ops`'
        // containment check cannot see a zero-length run as wrong.
        let (_, obj_len) = self.guest_ram_object(bytes)?;
        if let Err(e) = ram_slice_in_bounds(file_offset, len, obj_len) {
            self.ram_map_refused.fetch_add(1, Ordering::Relaxed);
            return Err(self.note(e));
        }
        let op = kayfabe_mmu::walkdiff::MapOp::Map(kayfabe_mmu::walkdiff::Run {
            va: at.0,
            gpga: file_offset,
            len,
            flags: 0,
            class: kayfabe_mmu::walkdiff::PageClass::P4K,
        });
        let done = self.apply_ops(vas, core::slice::from_ref(&op), SliceOf::GuestRam { bytes })?;
        done.placed
            .first()
            .copied()
            .ok_or_else(|| self.note_applied_nothing())
    }

    /// ★ Whether `[va, va+len)` is already covered by ONE placed guest-RAM slice in `vas`
    /// that maps it to `[file_offset, file_offset+len)`. Answered from the ledger alone.
    ///
    /// ⊘ This is what keeps coalescing from tearing a live slice down: rows already covered
    /// are dropped BEFORE runs are merged, so a run that grows is mapped as its new tail,
    /// never re-mapped whole under an engine that may be using its head.
    #[must_use]
    pub fn ram_covers(&self, vas: HostHandle, va: u64, len: u64, file_offset: u64) -> bool {
        let g = self
            .ram_placed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        g.range((vas.raw(), 0)..=(vas.raw(), va))
            .next_back()
            .is_some_and(|(&(_, start), p)| {
                let Some(delta) = va.checked_sub(start) else {
                    return false;
                };
                delta.checked_add(len).is_some_and(|e| e <= p.len)
                    && p.offset.checked_add(delta) == Some(file_offset)
            })
    }

    /// ★★★★★ **w825 — the RAM slices this port holds in `vas` that the guest's CURRENT rows
    /// no longer back**, i.e. the handle-ledger half of v3's *"diff the live tables against
    /// the ledger"*. `live` is every resolved guest-RAM row `(va, file_offset, len)` of the
    /// space; a slice is stale unless every byte of it is covered by live rows that map it to
    /// the SAME file offsets. ⊘ The caller must pass the COMPLETE row set — a capped list
    /// would read as stale everything past the cap.
    #[must_use]
    pub fn ram_stale(&self, vas: HostHandle, live: &[(u64, u64, u64)]) -> Vec<(u64, u64)> {
        let mut rows: Vec<(u64, u64, u64)> = live.to_vec();
        rows.sort_unstable();
        let g = self
            .ram_placed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        g.range((vas.raw(), 0)..=(vas.raw(), u64::MAX))
            .filter(|((_, start), p)| !ram_slice_backed(*start, p.offset, p.len, &rows))
            .map(|(&(_, start), p)| (start, p.len))
            .collect()
    }

    /// Unmap one guest-RAM slice through [`Self::apply_ops`] — the one mapper.
    ///
    /// # Errors
    /// [`StoreMapRefusal`], by name.
    pub fn unmap_ram_slice(
        &self,
        vas: HostHandle,
        bytes: u64,
        va: u64,
        len: u64,
    ) -> Result<(), StoreMapRefusal> {
        let op = kayfabe_mmu::walkdiff::MapOp::Unmap(kayfabe_mmu::walkdiff::Run {
            va,
            gpga: 0,
            len,
            flags: 0,
            class: kayfabe_mmu::walkdiff::PageClass::P4K,
        });
        self.apply_ops(vas, core::slice::from_ref(&op), SliceOf::GuestRam { bytes })
            .map(|_| ())
    }

    /// `(ram_maps, ram_map_refused, ram_bytes_mapped, ram_slices_held)` — the §18 census.
    #[must_use]
    pub fn ram_census(&self) -> (u64, u64, u64, usize) {
        (
            self.ram_maps.load(Ordering::Relaxed),
            self.ram_map_refused.load(Ordering::Relaxed),
            self.ram_bytes_mapped.load(Ordering::Relaxed),
            self.ram_placed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
        )
    }

    fn unmap(&self, vas: HostHandle, at: GpuVa) -> Result<(), StoreMapRefusal> {
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
                self.ram_placed
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
        // ⊘⊘ **w758 — `off_vcpu` FAILING IS *WE DID NOT ASK*, NOT *WE WERE REFUSED*.**
        // The counter used to bump here, so the census printed *"ASKED AND REFUSED EVERY TIME
        // — the route fired"* for a route that never left this process. Same conflation the
        // suite header carried at w756c, one layer down.
        let off = self.off_vcpu().map_err(|_r| {
            self.chan_declined.fetch_add(1, Ordering::Relaxed);
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
            .get(&host_vas)
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
                rm.birth_guest_channel_in_b(range, engine, ring, err_notifier)
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

    /// ★★★ w759 — a `Map` op that placed nothing. ⊘ Its own refusal rather than a default:
    /// substituting the requested address is what made w758's regression invisible.
    pub(crate) fn note_applied_nothing(&self) -> StoreMapRefusal {
        self.note(StoreMapRefusal::Rm(
            "apply_ops: a Map op placed no VA — RM answered success with no address".into(),
        ))
    }

    /// ★★★★★ **w757 — EXECUTE THE DIFF LIST, AND BE THE ONLY THING THAT EXECUTES IT.**
    ///
    /// > Owner, 2026-09-18: *"So ensure its executed, and ensure the diff list is the only
    /// > thing executing it."*
    ///
    /// ⊘⊘⊘ **The second half is the one with teeth, and it is enforced by PRIVACY rather than
    /// by discipline.** [`StoreMapPort::map`] and [`StoreMapPort::unmap`] are now **private**:
    /// this is the only way to change what is mapped, so *"the diff list is the only thing
    /// that executes"* is a property of the module's surface, not a rule a reviewer has to
    /// remember. A gate pins the caller count as well, because privacy stops at the module
    /// boundary and this file is large.
    ///
    /// # ★★★ Why a chokepoint is worth more than it costs
    ///
    /// `[measured w755u]` `maps=76 unmaps=0 replaced=0`: mappings only ever accumulated,
    /// because the bind path mapped directly and nothing else ever removed anything. With
    /// every change flowing through one list, *"what is mapped"* has exactly one author — and
    /// the ordering rule below can be stated once rather than at every call site.
    ///
    /// ⚠ **UNMAPS BEFORE MAPS, within one application.** §27's rule, and it is not a
    /// preference: a `Remap` is an unmap plus a map at the same VA, and running the map first
    /// leaves two slices live at one address — the two-memories-at-one-address state the
    /// single store exists to abolish. Ordering here makes the window **not exist** rather
    /// than making it small.
    ///
    /// # Errors
    /// The first refusal, with the op that caused it. ⊘ Stops on the first failure rather than
    /// continuing: a half-applied delta is a state nobody can describe, and continuing would
    /// make the ledger disagree with the guest's tables in a way no later diff could repair.
    ///
    /// # Panics
    /// Through `Worker::with_rm`'s `assert_lock_free`, if a caller reaches this holding a
    /// ranked lock.
    pub fn apply_ops(
        &self,
        vas: HostHandle,
        ops: &[kayfabe_mmu::walkdiff::MapOp],
        of: SliceOf,
    ) -> Result<AppliedOps, StoreMapRefusal> {
        use kayfabe_mmu::walkdiff::MapOp;
        // ★★★★★ §18 (w825) — the ground truth the runs are slices of, resolved ONCE here so
        // the bound below and every map below use the same object.
        let target = match of {
            SliceOf::Store => SliceTarget {
                obj: self.obj,
                len: self.obj_len,
                ram: false,
            },
            SliceOf::GuestRam { bytes } => {
                let (obj, len) = self.guest_ram_object(bytes)?;
                SliceTarget {
                    obj,
                    len,
                    ram: true,
                }
            }
        };
        let mut done = AppliedOps::default();
        // ⊘ Two passes over one list, not a sort: the list's own order is meaningful within
        // each kind (the differ emits coalesced runs), and sorting would discard it.
        // ★★★★★ §39(c) CONTAINMENT, CHECKED OVER THE WHOLE BATCH BEFORE ANY MAP HAPPENS.
        //
        // `map()` already refuses an out-of-range slice one at a time (`OutOfRange`), but that
        // is a per-op failure discovered MID-APPLY: by then some of the batch has been unmapped
        // and some mapped, and the caller is left to reason about a half-applied diff. A report
        // that names memory outside the store is not a report to partially honour — it is one
        // we have caught asking for memory that is not the guest's, and the whole thing is
        // refused before a single mapping moves.
        //
        // ⊘ This is the layer the WALK path was missing. The CUDA kernel refuses such a leaf at
        // both emit chokepoints (KFWR_R_LEAF_OOB) and `map()` bounds again underneath; this is
        // the one in the middle, and it is here rather than in `walkshadow` because THIS is
        // where the store's own length lives — a check that has to be handed its bound from
        // somewhere else is a check someone can forget to hand.
        for op in ops {
            let r = match op {
                MapOp::Map(r) | MapOp::Remap(r) => r,
                MapOp::Unmap(_) => continue,
            };
            if r.gpga > target.len || r.len > target.len - r.gpga {
                self.count_map_refused(target);
                return Err(StoreMapRefusal::OutOfRange {
                    offset: r.gpga,
                    len: r.len,
                    obj_len: target.len,
                });
            }
        }
        for op in ops {
            if let MapOp::Unmap(r) | MapOp::Remap(r) = op {
                self.unmap(vas, GpuVa(r.va))?;
                done.unmapped += 1;
            }
        }
        for op in ops {
            match op {
                MapOp::Map(r) | MapOp::Remap(r) => {
                    // ⊘ The store offset IS the GPGA: under the identity window a guest
                    // framebuffer address is an offset into the one reserved object. That is
                    // the whole reason the window must stay identity.
                    // ⊘ RM's ANSWER is kept, never the request. See `AppliedOps::placed`.
                    let (off, len, at) = (r.gpga, r.len, GpuVa(r.va));
                    done.placed.push(self.map(target, vas, off, len, at)?);
                    done.mapped += 1;
                }
                MapOp::Unmap(_) => {}
            }
        }
        done.ops = ops.len();
        self.applied.fetch_add(1, Ordering::Relaxed);
        Ok(done)
    }

    /// ★★★★★ **w757 — RELEASE A VA SPACE, AND ONLY WHEN ALL THREE CONDITIONS HOLD.**
    ///
    /// > Owner, 2026-09-18: *"Ensure va space is only released if it contains 0 mappings, its
    /// > table is no longer referenced and no channel uses it (vmm coordinated)."*
    ///
    /// ⊘ **The port re-checks its own two conditions even though the caller may have looked.**
    /// The VMM's witness is an assertion at an instant and can go stale between mint and use;
    /// the port's two cannot, because since w757 the port is the **only author of mappings**.
    /// ⇒ checking here is not belt-and-braces, it is the only check that is still true when it
    /// runs.
    ///
    /// ⚠ **Refusals are per-condition and NOT folded into one name.** *"Something still holds
    /// it"* would send a reader to look at all three; this tree has paid for one-refusal-for-
    /// several-causes repeatedly.
    ///
    /// # Errors
    /// [`StoreMapRefusal::VasStillHeld`] naming which condition failed, with its count.
    ///
    /// # Panics
    /// Through `Worker::with_rm`'s `assert_lock_free`, if a caller reaches this holding a
    /// ranked lock.
    pub fn release_vas(
        &self,
        witness: kayfabe_isolate::NoChannelHoldsVas,
    ) -> Result<(), StoreMapRefusal> {
        let vas = witness.vas();
        // ⊘ A witness for a DIFFERENT space must not release this one. The witness carries its
        // subject precisely so this check can exist.
        let placed = self
            .placed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let still_mapped = placed.keys().filter(|(v, _)| *v == vas.raw()).count();
        drop(placed);
        if still_mapped != 0 {
            self.release_refused.fetch_add(1, Ordering::Relaxed);
            return Err(self.note(StoreMapRefusal::VasStillHeld {
                vas: vas.raw(),
                why: "mappings",
                count: still_mapped,
            }));
        }
        // ★★★★★ **w758 — CONDITION 2 IS AN ACTION, NOT A PRECONDITION. My first version had
        // it backwards and the result was a function that could never release anything real.**
        //
        // ⊘⊘⊘ It refused when the space was still in `adopted` — but **being adopted is the
        // normal state** of every space this port ever mapped through, and nothing removes
        // entries from that ledger. So every real space was refused `table-referenced`
        // forever, and the only spaces that reached `Ok(())` were ones this port had never
        // adopted — i.e. it succeeded exactly when it had nothing to do.
        //
        // ⚠ And the success path did **no work at all**: no worker call, no RM free, no ledger
        // removal — while bumping a `released` counter. A counter that counts a no-op is worse
        // than no counter; this is the *"the passthrough verb is BUILT and ORPHANED"* shape,
        // authored in the same hour it was warned about.
        //
        // ⇒ *"Its table is no longer referenced"* is what this function must MAKE TRUE: drop
        // our own range inside B, then forget the space. Conditions 1 and 3 gate; 2 is the
        // work.
        let range = self
            .adopted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&vas)
            .copied();
        if let Some(range) = range {
            let off = self.off_vcpu()?;
            let out = self
                .iso
                .with_worker(move |worker| worker.with_rm(&off, move |rm| rm.free(range)));
            match out {
                // ⊘ NoWorker is *"we could not ask"* — the space stays adopted and the caller
                // may retry. Reporting it as a release would leak the range silently.
                None => {
                    self.release_refused.fetch_add(1, Ordering::Relaxed);
                    return Err(self.note(StoreMapRefusal::NoWorker));
                }
                Some(Err(e)) => {
                    self.release_refused.fetch_add(1, Ordering::Relaxed);
                    return Err(self.note(StoreMapRefusal::Rm(format!("{e:?}"))));
                }
                Some(Ok(())) => {}
            }
            // ⊘ Removed only AFTER RM agreed: a ledger that forgets a range RM still holds is
            // a leak this port can no longer even name.
            self.adopted
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&vas);
            // ★ w825 — freeing the range took every guest-RAM slice in it down with it; the
            // ledger forgets them here so a later map is not answered from a dead entry.
            // ⊘ Not counted as "holding" the space above: nothing unmaps RAM slices on a
            // guest unmap yet, so counting them would refuse every release forever.
            let r = range.raw();
            self.ram_placed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .retain(|(v, _), _| *v != r);
        }
        // Condition 3 is the witness's existence: it cannot be constructed with a channel
        // still using the space.
        self.released.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// ★★★★★ **w755u — SCHEDULE AND RING A CHANNEL THIS PORT'S ISOLATE OWNS.**
    ///
    /// `[measured w755t]` the birth moved to B and the doorbell did not follow: every one was
    /// refused `ForeignHandle { handle: 0xb1470006, worker_isolate: iso2 }`. ⊘ The gate was
    /// right; the verb was in the wrong process — w755q's defect, one verb later.
    ///
    /// ⚠ **The order is the ruling, not tidiness.** `GPFIFO_SCHEDULE` first, then the ring:
    /// the host-side runlist submit is lazy, and a doorbell rung on a channel that is not on
    /// the runlist has its submission **dropped silently** — nothing faults and the guest
    /// waits forever. That is `NEVER RETIRED` with `Xid = 0`, character for character.
    ///
    /// ⊘ Both verbs are plain [`kayfabe_isolate::RmBackend`] methods that already cross the
    /// socket, so this needs **no new protocol request**: what was missing was never a
    /// transport, only a caller on the right side of it.
    ///
    /// # Errors
    /// [`kayfabe_fwd::FwdFault`], by name.
    ///
    /// # Panics
    /// Through `Worker::with_rm`'s `assert_lock_free`, if a caller reaches this holding a
    /// ranked lock. That is the invariant, not a bug to be caught.
    pub fn doorbell_over_the_store(
        &self,
        chan: HostHandle,
        token: u64,
        schedule: bool,
    ) -> Result<(), kayfabe_fwd::FwdFault> {
        self.chan_doorbells.fetch_add(1, Ordering::Relaxed);
        let refused = |e: kayfabe_isolate::RmError| {
            self.chan_doorbell_refused.fetch_add(1, Ordering::Relaxed);
            kayfabe_fwd::FwdFault::Rm { err: e, on: None }
        };
        let off = self.off_vcpu().map_err(|_| {
            refused(kayfabe_isolate::RmError::Other(
                kayfabe_isolate::STORE_BIRTH_REFUSED,
            ))
        })?;
        let out = self.iso.with_worker(move |worker| {
            worker.with_rm(&off, move |rm| {
                if schedule {
                    rm.schedule(chan)?;
                }
                rm.ring_doorbell(token)
            })
        });
        match out {
            None => Err(refused(kayfabe_isolate::RmError::Other(
                kayfabe_isolate::STORE_BIRTH_REFUSED,
            ))),
            Some(Err(e)) => {
                eprintln!(
                    "kayfabe: STORE-DOORBELL ⊘⊘ REFUSED by the scratchpad chan={chan:?} \
                     token={token:#x} schedule={schedule} — {e:?}"
                );
                Err(refused(e))
            }
            Some(Ok(())) => {
                self.chan_rung.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        }
    }

    /// ★★★★★ **w783 — ALLOCATE AN ENGINE OBJECT ON A CHANNEL THIS PORT'S ISOLATE OWNS.**
    ///
    /// `[measured w783, thin guest r4]` the engine-object forward was still running on the
    /// per-proc worker after the birth moved to B, so **every** one of them was refused:
    ///
    /// ```text
    ///   ENGINE-OBJECT class=0xc7c0 client=0xc1d0000b parent=0xcafe001b params=16B
    ///     -> REFUSED ForeignHandle { handle: HostHandle(iso4294967295/gpu0:0xb1470008),
    ///                                worker_isolate: iso1/gpu0 }  [seen=8 forwarded=0 refused=8]
    /// ```
    ///
    /// ⊘ The gate was right and the verb was in the wrong process — w755q's defect, now a
    /// third time. See [`kayfabe_fwd::StoreChannelBirth::engine_object_over_the_store`] for
    /// what `forwarded=0` cost: `P3 rpc-bind`'s `FAULT_PDE`, and w780's regression of three
    /// green rows to `NEVER RETIRED`.
    ///
    /// ⊘ [`kayfabe_isolate::RmBackend::alloc_engine_object`] already crosses the socket, so
    /// like the doorbell this needs **no new protocol request** — what was missing was a
    /// caller on the right side of one.
    ///
    /// # Errors
    /// [`kayfabe_fwd::FwdFault`], by name.
    ///
    /// # Panics
    /// Through `Worker::with_rm`'s `assert_lock_free`, if a caller reaches this holding a
    /// ranked lock. That is the invariant, not a bug to be caught.
    pub fn engine_object_over_the_store(
        &self,
        chan: HostHandle,
        class: kayfabe_rt::ClassId,
        params: &[u8],
    ) -> Result<HostHandle, kayfabe_fwd::FwdFault> {
        self.chan_engine_objects.fetch_add(1, Ordering::Relaxed);
        let refused = |e: kayfabe_isolate::RmError| {
            self.chan_engine_object_refused
                .fetch_add(1, Ordering::Relaxed);
            kayfabe_fwd::FwdFault::Rm { err: e, on: None }
        };
        let off = self.off_vcpu().map_err(|_| {
            refused(kayfabe_isolate::RmError::Other(
                kayfabe_isolate::STORE_BIRTH_REFUSED,
            ))
        })?;
        let blob = params.to_vec();
        let out = self.iso.with_worker(move |worker| {
            worker.with_rm(&off, move |rm| rm.alloc_engine_object(chan, class, &blob))
        });
        match out {
            None => Err(refused(kayfabe_isolate::RmError::Other(
                kayfabe_isolate::STORE_BIRTH_REFUSED,
            ))),
            Some(Err(e)) => {
                eprintln!(
                    "kayfabe: STORE-ENGINE-OBJECT ⊘⊘ REFUSED by the scratchpad chan={chan:?} \
                     class={:#x} params={}B — {e:?}",
                    class.0,
                    params.len()
                );
                Err(refused(e))
            }
            Some(Ok(obj)) => {
                eprintln!(
                    "kayfabe: STORE-ENGINE-OBJECT ✔ chan={chan:?} class={:#x} → object={obj:?} \
                     ⇒ the host channel now has its engine context; on real silicon host RM \
                     builds and self-promotes its OWN golden context at this alloc",
                    class.0
                );
                Ok(obj)
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
        let (cda, cdr, cdg) = (
            self.chan_doorbells.load(Ordering::Relaxed),
            self.chan_doorbell_refused.load(Ordering::Relaxed),
            self.chan_rung.load(Ordering::Relaxed),
        );
        // ★★★★★ **w755u's ROW — and it exists because `born>0` was NOT enough.**
        // `[measured w755t]` 11 channels were born in B and every doorbell on them was
        // refused `ForeignHandle`, so the birth census read green while nothing reached the
        // GPU. ⇒ the BIRTH and the RING need separate verdicts, or a boot cannot say which
        // half stopped.
        let chan_ring_verdict = if cda == 0 {
            "⊘⊘ NEVER ASKED — no doorbell was carried to a channel in another isolate. With \
             born>0 above, that means the doorbells are still running on the per-proc worker \
             and being refused ForeignHandle; nothing reached the GPU"
        } else if cdg == 0 {
            "⊘⊘ CARRIED AND REFUSED EVERY TIME — read the named refusals above"
        } else if cdr > 0 {
            "⚠ PARTIAL — some rang and some did not; the ones that did not were dropped \
             silently, which is `NEVER RETIRED` with no fault"
        } else {
            "★★★ EVERY DOORBELL REACHED THE CHANNEL'S OWN ISOLATE — scheduled, then rung"
        };
        let (cea, cer) = (
            self.chan_engine_objects.load(Ordering::Relaxed),
            self.chan_engine_object_refused.load(Ordering::Relaxed),
        );
        // ★★★★★ **w783's ROW — and it exists for w755u's reason, one verb earlier.**
        // `[measured w783]` the birth census read green and the doorbell census read green
        // while `ENGINE-OBJECT … forwarded=0 refused=8` sat in the log above them: not one
        // host channel had ever received its engine class object. A channel that is born and
        // rung but carries no engine object is a channel hardware faults on
        // (`FAULT_PDE`), so this needs its own verdict too.
        let chan_obj_verdict = if cea == 0 {
            "⊘⊘ NEVER ASKED — no engine object was carried to a channel in another isolate. \
             With born>0 above, that means the engine-object forward is still running on the \
             per-proc worker and being refused ForeignHandle, and every host channel is \
             running with NO engine context"
        } else if cer == cea {
            "⊘⊘ CARRIED AND REFUSED EVERY TIME — read the named refusals above"
        } else if cer > 0 {
            "⚠ PARTIAL — some channels have their engine context and some do not; the ones \
             that do not fault FAULT_PDE the first time an engine walks their context"
        } else {
            "★★★ EVERY ENGINE OBJECT REACHED THE CHANNEL'S OWN ISOLATE — host RM built the \
             context (golden ctx included, on real silicon)"
        };
        format!(
            "STORE-ENGINE-OBJECT asked={cea} refused={cer} ⇒ {chan_obj_verdict}\n\
             STORE-DOORBELL asked={cda} refused={cdr} rung={cdg} ⇒ {chan_ring_verdict}\n\
             STORE-BIRTH asked={cba} refused={cbr} born={cbb} ⇒ {chan_birth_verdict}\n\
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

    fn store_slice_covering(&self, vas: HostHandle, at: GpuVa) -> Option<(u64, u64, u64)> {
        let g = self
            .placed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        g.range((vas.raw(), 0)..=(vas.raw(), at.0))
            .next_back()
            .filter(|((_, start), p)| at.0 < start.saturating_add(p.len))
            .map(|(&(_, start), p)| (start, p.offset, p.len))
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
        StoreMapPort::birth_over_the_store(self, host_vas, engine, ring, err_notifier)
    }

    fn doorbell_over_the_store(
        &self,
        chan: HostHandle,
        token: u64,
        schedule: bool,
    ) -> Result<(), kayfabe_fwd::FwdFault> {
        StoreMapPort::doorbell_over_the_store(self, chan, token, schedule)
    }

    fn engine_object_over_the_store(
        &self,
        chan: HostHandle,
        class: kayfabe_rt::ClassId,
        params: &[u8],
    ) -> Result<HostHandle, kayfabe_fwd::FwdFault> {
        StoreMapPort::engine_object_over_the_store(self, chan, class, params)
    }
}

/// ★★★ w757 — what one [`StoreMapPort::apply_ops`] did.
///
/// ⊘ Counts, not a bool: `ops=12 mapped=0 unmapped=0` is a list that arrived and changed
/// nothing, which is a different fact from a list that never arrived.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AppliedOps {
    /// How many ops the list carried.
    pub ops: usize,
    /// Slices mapped (`Map` + `Remap`).
    pub mapped: usize,
    /// Slices unmapped (`Unmap` + `Remap`).
    pub unmapped: usize,
    /// ★★★★★ **w759 — THE VAs RM ACTUALLY PLACED, in map order.**
    ///
    /// ⊘⊘⊘ **This field exists because leaving it out caused a live regression.** The bind
    /// path needs the address RM *chose*, not the one we asked for, and when that path was
    /// routed through `apply_ops` the returned value was replaced with the store OFFSET. Both
    /// are `u64`, so nothing failed to compile and nothing failed a test — `[measured w758]`
    /// the guest went from `P1/P2/P3 ✔ VERIFIED` back to `NEVER RETIRED` on the next boot.
    ///
    /// ⚠ **`placed_as_asked` is the whole reason RM's answer is not the request.** A map that
    /// lands elsewhere is a real outcome this tree checks for; a caller handed back its own
    /// request can never see it.
    pub placed: Vec<u64>,
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

/// Whether `[start, start+len)` mapped to `[off, off+len)` is fully covered by `rows`
/// (sorted `(va, file_offset, len)`), each row agreeing on the offset. See
/// [`StoreMapPort::ram_stale`].
#[must_use]
pub fn ram_slice_backed(start: u64, off: u64, len: u64, rows: &[(u64, u64, u64)]) -> bool {
    let Some(end) = start.checked_add(len) else {
        return false;
    };
    let mut at = start;
    for &(va, foff, rlen) in rows {
        if at >= end {
            break;
        }
        let Some(rend) = va.checked_add(rlen) else {
            return false;
        };
        if rend <= at {
            continue;
        }
        if va > at {
            return false;
        }
        let delta = at - va;
        if foff.checked_add(delta) != off.checked_add(at - start) {
            return false;
        }
        at = rend.min(end);
    }
    at >= end
}

// ════════════════════════════════════════════════════════════════════════════════════════════
// ★★★★★ w826 — THE v3 PUBLISHER: walk the guest's roots IN PLACE on the GPU, reconcile the
// result against OUR OWN handle ledger. No CPU read of a page table, no address table.
// ════════════════════════════════════════════════════════════════════════════════════════════

/// One mapping a walk says the guest's tables currently express, as a slice of one of the two
/// ground truths: the reserved store (`ram == false`, `off` = GPGA offset) or the guest-RAM
/// object (`ram == true`, `off` = memfd file offset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Desired {
    /// Guest VA.
    pub va: u64,
    /// Bytes.
    pub len: u64,
    /// Offset into the object named by `ram`.
    pub off: u64,
    /// Which ground truth.
    pub ram: bool,
}

/// Sort and merge half-open `[a, b)` ranges; touching ranges merge; empty ones are dropped.
#[must_use]
pub fn merge_ranges(mut r: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    r.retain(|(a, b)| b > a);
    r.sort_unstable();
    let mut out: Vec<(u64, u64)> = Vec::with_capacity(r.len());
    for (a, b) in r {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// What [`plan_reconcile`] asks for. Unmaps are applied FIRST.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReconcilePlan {
    /// `(va, len)` of ledger slices to take down.
    pub unmap: Vec<(u64, u64)>,
    /// Desired runs to map.
    pub map: Vec<Desired>,
    /// Ledger slices left exactly as they are.
    pub kept: usize,
}

/// ★★★★★ **w826 — a server row, made mappable and made SUBORDINATE to the walk.**
///
/// A `GPU_PROMOTE_CTX` row is what RM asked US to map; the walk is what the guest's own
/// tables say. Two rules, both measured `[w826 m2 cup3: cuCtxCreate 719]`:
/// 1. **Whole pages.** RM refuses a mapping that is not a page multiple (`0x20409d000+0x8600`
///    → `Other(19305)`); the C rounds every promote mapping (`nvkvm_gpu_emul.c:7920`). A row
///    that is 64 KiB-aligned on both sides takes the C's 64 KiB round-up; any other row rounds
///    to 4 KiB. A row whose VA and backing disagree inside a page cannot be expressed → none.
/// 2. **The walk wins where both speak.** A row overlapping walked runs contributes only the
///    pieces the walk left empty — otherwise the two fight over one slice and every pass
///    unmaps and remaps it (`mapped=1 unmapped=1`, measured).
///
/// Returns `(va, len, backing offset)` pieces, page-aligned, ascending.
#[must_use]
pub fn server_row_pieces(
    va: u64,
    len: u64,
    phys: u64,
    walked: &[(u64, u64)],
) -> Vec<(u64, u64, u64)> {
    const PAGE: u64 = 0x1000;
    const BIG: u64 = 0x1_0000;
    if len == 0 || (va % PAGE) != (phys % PAGE) {
        return Vec::new();
    }
    let lead = va % PAGE;
    let (va, phys, len) = (va - lead, phys - lead, len + lead);
    let grain = if va % BIG == 0 && phys % BIG == 0 { BIG } else { PAGE };
    let Some(end) = len.checked_next_multiple_of(grain).and_then(|l| va.checked_add(l)) else {
        return Vec::new();
    };
    let mut cover: Vec<(u64, u64)> = walked
        .iter()
        .filter(|(w, l)| *l > 0 && *w < end && w.saturating_add(*l) > va)
        .map(|&(w, l)| (w, w.saturating_add(l)))
        .collect();
    cover.sort_unstable();
    let mut out = Vec::new();
    let mut at = va;
    for (s, e) in cover {
        if s > at {
            out.push((at, s.min(end) - at, phys + (at - va)));
        }
        at = at.max(e);
        if at >= end {
            break;
        }
    }
    if at < end {
        out.push((at, end - at, phys + (at - va)));
    }
    out
}

/// ★★★★★ **The pure half of the reconcile — no GPU, no isolate, fully testable.**
///
/// `ledger` is every slice this port holds in one VA space, `(va, len, off, ram)`; `desired`
/// is the COMPLETE state a walk reported for that space. A ledger slice is kept iff desired
/// runs of the SAME ground truth back every byte of it at the same offsets; every other slice
/// is unmapped. A desired run already backed by kept slices is left alone; otherwise every kept
/// slice that overlaps it is unmapped too and the run is mapped whole — a FIXED map over a live
/// slice is refused by RM, so overlap is resolved here, not discovered there.
#[must_use]
pub fn plan_reconcile(ledger: &[(u64, u64, u64, bool)], desired: &[Desired]) -> ReconcilePlan {
    let rows = |ram: bool| -> Vec<(u64, u64, u64)> {
        let mut v: Vec<(u64, u64, u64)> = desired
            .iter()
            .filter(|d| d.ram == ram)
            .map(|d| (d.va, d.off, d.len))
            .collect();
        v.sort_unstable();
        v
    };
    let (want_store, want_ram) = (rows(false), rows(true));
    let mut plan = ReconcilePlan::default();
    let mut kept: Vec<(u64, u64, u64, bool)> = Vec::new();
    for &(va, len, off, ram) in ledger {
        let want = if ram { &want_ram } else { &want_store };
        if ram_slice_backed(va, off, len, want) {
            kept.push((va, len, off, ram));
        } else {
            plan.unmap.push((va, len));
        }
    }
    kept.sort_unstable();
    let kept_rows = |ram: bool| -> Vec<(u64, u64, u64)> {
        kept.iter()
            .filter(|k| k.3 == ram)
            .map(|&(va, len, off, _)| (va, off, len))
            .collect()
    };
    let (have_store, have_ram) = (kept_rows(false), kept_rows(true));
    let mut dropped: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
    for d in desired {
        let have = if d.ram { &have_ram } else { &have_store };
        if ram_slice_backed(d.va, d.off, d.len, have) {
            continue;
        }
        for &(kva, klen, _, _) in &kept {
            let overlaps = kva < d.va.saturating_add(d.len) && d.va < kva.saturating_add(klen);
            if overlaps && dropped.insert(kva) {
                plan.unmap.push((kva, klen));
            }
        }
        plan.map.push(*d);
    }
    plan.kept = kept.len() - dropped.len();
    plan
}

/// What [`StoreMapPort::reconcile`] did.
#[derive(Debug, Default, Clone)]
pub struct Reconciled {
    /// Slices left in place.
    pub kept: usize,
    /// Slices taken down.
    pub unmapped: usize,
    /// Runs mapped.
    pub mapped: usize,
    /// Operations RM refused.
    pub refused: usize,
    /// The first refusal, by name.
    pub first_refusal: Option<String>,
}

impl StoreMapPort {
    /// ★★★★★ **Walk the guest's roots IN PLACE** — the scratchpad's walk kernel reads the
    /// tables where they live, inside the one reserved object. `pdbs` are GPGA offsets,
    /// strictly ascending, at most `KF_MAX_PDB`. ⊘ Never acked: every report is a full
    /// RESYNC, so [`Self::reconcile`] always sees complete state and trusts no delta.
    ///
    /// # Errors
    /// The refusal, by name.
    pub fn walk_in_place(
        &self,
        pdbs: &[u64],
        ack: u64,
    ) -> Result<kayfabe_mmu::walkreport::Report, String> {
        let off = self
            .off_vcpu()
            .map_err(|r| format!("declined: {r:?}"))?;
        let bytes = self
            .iso
            .with_worker(|worker| {
                worker
                    .with_rm(&off, |rm| rm.walk_shadow_run(pdbs, ack))
                    .map_err(|e| format!("walk refused: {e:?}"))
            })
            .unwrap_or_else(|| Err("the scratchpad isolate offered no worker".to_string()))?;
        kayfabe_mmu::walkreport::Report::parse(&bytes).map_err(|e| format!("report: {e:?}"))
    }

    /// Every slice this port holds in `vas`, from both ledgers, as `(va, len, off, ram)`.
    #[must_use]
    pub fn ledger_of(&self, vas: HostHandle) -> Vec<(u64, u64, u64, bool)> {
        let mut out = Vec::new();
        for (ledger, ram) in [(&self.placed, false), (&self.ram_placed, true)] {
            let g = ledger
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for (&(_, va), p) in g.range((vas.raw(), 0)..=(vas.raw(), u64::MAX)) {
                out.push((va, p.len, p.offset, ram));
            }
        }
        out
    }

    /// ★★★★★ **Make the host VA space say exactly what the walk said** — through
    /// [`Self::apply_ops`], the one mapper. Unmaps first, then maps.
    pub fn reconcile(&self, vas: HostHandle, desired: &[Desired], ram_bytes: u64) -> Reconciled {
        self.apply_plan(vas, &plan_reconcile(&self.ledger_of(vas), desired), ram_bytes)
    }

    /// Slices of `vas` overlapping `[a, b)`, from both ledgers. ⊘ Each ledger is
    /// non-overlapping by construction, so at most ONE slice starting below `a` can reach it.
    fn ledger_overlapping(&self, vas: HostHandle, a: u64, b: u64) -> Vec<(u64, u64, u64, bool)> {
        let mut out = Vec::new();
        for (ledger, ram) in [(&self.placed, false), (&self.ram_placed, true)] {
            let g = ledger
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some((&(_, va), p)) = g.range((vas.raw(), 0)..(vas.raw(), a)).next_back() {
                if va.saturating_add(p.len) > a {
                    out.push((va, p.len, p.offset, ram));
                }
            }
            for (&(_, va), p) in g.range((vas.raw(), a)..(vas.raw(), b)) {
                out.push((va, p.len, p.offset, ram));
            }
        }
        out
    }

    /// ★★★★★ **w826 — the DELTA half of [`StoreMapPort::reconcile`].** Plan only the VA ranges
    /// a walk delta `touched`, CLOSED over every desired run and ledger slice overlapping them
    /// (a run that crosses the edge of a touched range pulls its whole extent in, until
    /// nothing new is pulled). `desired_in(a, b)` answers the desired runs overlapping
    /// `[a, b)`. `[measured w826 q5]` the full reconcile cost ~1.9 µs per run per pass, so a
    /// 13 000-row arm went quadratic; this costs what the delta touched.
    pub fn reconcile_scoped(
        &self,
        vas: HostHandle,
        touched: &[(u64, u64)],
        desired_in: &dyn Fn(u64, u64) -> Vec<Desired>,
        ram_bytes: u64,
    ) -> Reconciled {
        let mut ranges = merge_ranges(touched.to_vec());
        let (mut desired, mut ledger);
        loop {
            desired = Vec::new();
            ledger = Vec::new();
            for &(a, b) in &ranges {
                desired.extend(desired_in(a, b));
                ledger.extend(self.ledger_overlapping(vas, a, b));
            }
            desired.sort_unstable_by_key(|d| (d.va, d.len, d.off, d.ram));
            desired.dedup_by_key(|d| (d.va, d.len, d.off, d.ram));
            ledger.sort_unstable();
            ledger.dedup();
            let mut next = ranges.clone();
            next.extend(desired.iter().map(|d| (d.va, d.va.saturating_add(d.len))));
            next.extend(ledger.iter().map(|l| (l.0, l.0.saturating_add(l.1))));
            let next = merge_ranges(next);
            if next == ranges {
                break;
            }
            ranges = next;
        }
        self.apply_plan(vas, &plan_reconcile(&ledger, &desired), ram_bytes)
    }

    fn apply_plan(&self, vas: HostHandle, plan: &ReconcilePlan, ram_bytes: u64) -> Reconciled {
        use kayfabe_mmu::walkdiff::{MapOp, PageClass, Run};
        let mut out = Reconciled {
            kept: plan.kept,
            ..Reconciled::default()
        };
        let run = |va: u64, len: u64, off: u64| Run {
            va,
            gpga: off,
            len,
            flags: 0,
            class: PageClass::P4K,
        };
        for &(va, len) in &plan.unmap {
            match self.apply_ops(vas, &[MapOp::Unmap(run(va, len, 0))], SliceOf::Store) {
                Ok(_) => out.unmapped += 1,
                Err(e) => {
                    out.refused += 1;
                    out.first_refusal.get_or_insert_with(|| format!("unmap {va:#x}: {e:?}"));
                }
            }
        }
        for d in &plan.map {
            let of = if d.ram {
                SliceOf::GuestRam { bytes: ram_bytes }
            } else {
                SliceOf::Store
            };
            match self.apply_ops(vas, &[MapOp::Map(run(d.va, d.len, d.off))], of) {
                Ok(_) => out.mapped += 1,
                Err(e) => {
                    out.refused += 1;
                    out.first_refusal
                        .get_or_insert_with(|| format!("map {:#x}+{:#x}: {e:?}", d.va, d.len));
                }
            }
        }
        out
    }
}
