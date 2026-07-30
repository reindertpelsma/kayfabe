//! # kayfabe-mmu — the address plane
//!
//! Owns THE ADDRESS TABLE (`mode2_address_table.md`, the one-table-of-truth
//! directive): **one authoritative, forward-populated VA→phys table per VAS, keyed by
//! PDB** — never by client handle (lesson L7: many clients share one VAS; a
//! GSP-managed `hVASpace=0` channel has no client-keyed VAS at all).
//!
//! The rules, as API shape (they are enforced *by construction*, testing strategy
//! `taddr_forward_only`):
//!
//! - **Forward-populate only.** [`AddressTable::bind`] is the only way in — called at
//!   bind time (RPC populate source) or at a CE-PT-write commit point (#13's populate
//!   source). There is no exec-time reverse-resolve entry point in this API.
//! - **MISS = FAULT, with NO deferring case at this layer.** [`AddressTable::resolve`]
//!   returns [`AddressFault::Miss`] on a miss. There is no fallback walk (a torn
//!   multi-level walk = wrong phys = cross-context leak — MISS=FAULT is a *security*
//!   property, arch doc §4.3.5).
//!
//!   ★ The core's miss taxonomy (`kayfabe_core` crate docs) allows one other answer —
//!   DEFER, when the fact is *not yet knowable* — and **no site in this crate qualifies**.
//!   That is worth stating rather than leaving to inference: this table IS the guest's
//!   TLB, and a TLB has no "later". A resolve happens because hardware is about to touch
//!   the address; an unbound VA is a fault on real silicon at that instant, and answering
//!   "wait, it may be declared soon" would be answering a question nobody asked. The
//!   deferring belongs one layer up, in DERIVATION (`Gpu::sync_rpc_mappings` defers a
//!   mapping whose PDB has not been declared) — and deferring there is precisely what
//!   lets this layer be absolute.
//! - **Unmap eager**, map lazy, reclaim deferred ([`AddressTable::unbind`]).
//!
//! ★ **corrected 2026-07-27** (was: *"Also here (skeleton this milestone): the GMMU **walk
//! algorithm**…"*, which overstated — found by the whitepaper's verification pass). What
//! [`walker`] actually contains today is **the seam and nothing behind it**: the [`walker::FbRead`]
//! page-table byte-source trait and the [`walker::WalkResult`] outcome enum. No walk loop, no
//! constructor, no implementor — 41 lines. The algorithm itself is still to be written, and
//! [`walker`]'s own module doc states the requirements it must meet: regime-independent core
//! logic against [`kayfabe_arch::GmmuFmt`], used only at forward-populate commit points
//! (decode-dirtied-PT-pages, #13), never as a resolve fallback.
//!
//! Concurrency (decision #17): plain owned data, no interior mutability;
//! [`AddressTable::resolve`]/[`AddressTable::iter`] are `&self` (concurrent reads
//! safe), bind/unbind are `&mut self` (caller-exclusive). `Send + Sync`
//! compile-time-asserted below; full contract in `kayfabe-core`'s crate docs.

pub mod walker;

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{GpuVa, Pdb};
use kayfabe_isolate::{HostHandle, IsolateId};
use kayfabe_util::IntervalMap;

/// Why a `[offset, offset+len)` byte range inside a host object was refused at
/// construction. Two variants because they are different mistakes and a caller that
/// reports "bad slice" for both has told nobody anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostSliceError {
    /// `len == 0`. A binding backed by no bytes is not a binding; it is a handle with
    /// an opinion.
    Empty,
    /// `offset + len` wraps `u64`. Guest-influenced arithmetic never panics here
    /// (boundary-1 posture) — it refuses.
    Wraps,
}

/// ★★★ **A byte range inside a host memory object** — the arena sub-allocation unit
/// (`gpga_address_space.md` §8.2).
///
/// The fields are **private and there is exactly one constructor**, and that is the
/// whole point of the type: *"an offset with no length"* is not a state you can write
/// down. [`HostSlice::new`] additionally refuses the two ranges that are not ranges —
/// empty, and wrapping `u64` — so a `HostSlice` that exists names real bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct HostSlice {
    offset: u64,
    len: u64,
}

impl HostSlice {
    /// Bytes `[offset, offset+len)` of a host object.
    ///
    /// # Errors
    /// - [`HostSliceError::Empty`] — `len == 0`.
    /// - [`HostSliceError::Wraps`] — `offset + len` does not fit in `u64`.
    pub const fn new(offset: u64, len: u64) -> Result<Self, HostSliceError> {
        if len == 0 {
            return Err(HostSliceError::Empty);
        }
        if offset.checked_add(len).is_none() {
            return Err(HostSliceError::Wraps);
        }
        Ok(HostSlice { offset, len })
    }

    /// Byte offset of the slice within its object.
    #[must_use]
    pub const fn offset(self) -> u64 {
        self.offset
    }

    /// Length of the slice in bytes. Never zero.
    ///
    /// ★ No `is_empty` on purpose (clippy's pairing lint is allowed off here): a
    /// `HostSlice` **cannot** be empty — [`HostSliceError::Empty`] refuses that at
    /// construction — so an `is_empty` could only ever answer `false`, and a predicate
    /// that has one possible answer invites callers to branch on a question that is
    /// already settled by the type.
    #[must_use]
    #[allow(clippy::len_without_is_empty)]
    pub const fn len(self) -> u64 {
        self.len
    }

    /// One past the last byte, within the object. Never wraps (refused at construction).
    #[must_use]
    pub const fn end(self) -> u64 {
        self.offset + self.len
    }
}

/// ★★★ **WHICH PART of the host object a binding is backed by — and, inseparably, WHO
/// FREES IT** (`gpga_address_space.md` §8.2/§9.3).
///
/// The two cases are not two spellings of a coordinate pair; they are two **ownership
/// regimes**, which is why this is an exhaustively-matched enum and not an `offset`
/// field:
///
/// - [`HostExtent::Whole`] — the object was allocated for this binding and nothing else
///   refers to it. Reclaiming the binding is what frees it. This is every publication
///   the code makes today.
/// - [`HostExtent::Slice`] — the object is a **reservation arena** that outlives this
///   binding and serves other bindings at other offsets. Reclaiming the binding must
///   **not** free it; the bytes are owed back to the arena, and the arena's own owner
///   frees the object.
///
/// ★ **Why an enum rather than `offset: u64, len: u64` on [`HostBacking`].** With bare
/// coordinates, `offset == 0` is ambiguous — the sole owner of a small object and the
/// first slice of a large arena are the same value — so every reclaim site would have to
/// *re-derive* whether it may free, from information it does not have. `Whole`/`Slice`
/// puts the answer in the type: adding the second case turns every existing free site
/// into a compile error until it says which regime it is in, which is exactly the
/// retrofit §8.2 says to buy now rather than later.
///
/// ★ **Why `Whole` carries no length.** The object *is* the bound range, so its length
/// is the range's length, already held by the table. Copying it here would be a second
/// source of truth that can drift. `Slice` must carry its own because the object is
/// **bigger than the binding** — the coordinates are not recoverable from the range —
/// and because a `HostBacking` travels standalone (trace events, refusal payloads) where
/// no interval length is attached to it. [`AddressTable::bind`] pins `Slice`'s length to
/// the range it backs ([`AddressFault::SliceLenMismatch`]), so the duplication cannot
/// drift silently either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostExtent {
    /// The entire object, owned by this binding alone.
    Whole,
    /// Bytes of an arena object that outlives this binding.
    Slice(HostSlice),
}

/// ★ **The host materialization of one binding — allocation, placement and EXTENT,
/// together** (`l1_concurrency.md` §12.16 gap G1; `gpga_address_space.md` §8.2).
///
/// A published range is backed by THREE host facts: an allocated host memory object
/// ([`HostBacking::memory`]), the GPU VA it is mapped at inside the owning `Vas`'s own
/// host VAS ([`HostBacking::host_va`]), and **which part of that object it is**
/// ([`HostBacking::extent`]). Reclaiming it needs all three — `unmap_gpu_va(vas,
/// host_va)`, then `free(memory)` **only if the binding owns the whole object** — so
/// they are ONE value, not three fields anyone can assemble a subset of.
///
/// **Why a struct rather than a second `Option` on [`Binding`].** With
/// `memory: Option<HostHandle>` beside `host_va: Option<u64>`, the state
/// *"mapped somewhere, owning nothing freeable"* is representable — and that state
/// was precisely the G1 defect: `commit_publish` stored the mapped VA and dropped the
/// `HostHandle` on the floor, so the majority of allocated host bytes existed in no
/// core state and no reclaim path could ever name them. Folding the trio into one
/// `Option<HostBacking>` makes **bound-but-unfreeable unrepresentable**: you cannot
/// write a host VA into a binding without also writing the handle that frees it.
/// (House pattern — `GpaSpace::release(arena)`-by-value: prefer the type over the
/// runtime check.)
///
/// ★★ **The fields are private and the constructors are [`HostBacking::whole`] and
/// [`HostBacking::slice`].** That is the same discipline one level down: a slice cannot
/// be assembled without its owning handle, and an extent cannot be attached to a
/// backing that has no handle to attach it to.
///
/// ★★ **Owner scope is INHERITED, not invented** (§9.3). RM grants *objects*, not
/// ranges: an isolate holding an arena handle can map **any** offset in it, so a slice
/// handed across an isolate boundary is a reach over that isolate's whole reservation.
/// The scope is already carried by [`HostHandle`]'s [`IsolateId`], and
/// `Worker::execute`'s foreign-handle gate already refuses across it — so
/// [`HostBacking::owner`] and [`HostBacking::belongs_to`] delegate to it rather than
/// adding a second, separately-maintained notion of who owns what.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostBacking {
    /// The host object this range is backed by, in the OWNING isolate's handle
    /// namespace (`(Proc, GpuId)`-scoped — a handle from another isolate is a different
    /// object, boundary 2). For [`HostExtent::Slice`] this is the **arena**.
    memory: HostHandle,
    /// The host GPU VA it is mapped at, inside the owning `Vas`'s own host VAS.
    /// Per-Vas host placement is #14's proven fix — see `kayfabe-fwd`.
    host_va: u64,
    /// Which part of `memory`, and therefore who frees it.
    extent: HostExtent,
}

impl HostBacking {
    /// A backing that owns its whole object: reclaiming this binding frees `memory`.
    #[must_use]
    pub const fn whole(memory: HostHandle, host_va: u64) -> Self {
        HostBacking {
            memory,
            host_va,
            extent: HostExtent::Whole,
        }
    }

    /// A backing over `slice` of the arena object `arena`, which **outlives** this
    /// binding: reclaiming this binding must not free `arena`.
    #[must_use]
    pub const fn slice(arena: HostHandle, host_va: u64, slice: HostSlice) -> Self {
        HostBacking {
            memory: arena,
            host_va,
            extent: HostExtent::Slice(slice),
        }
    }

    /// The host object — the whole object, or the arena a slice was cut from.
    #[must_use]
    pub const fn memory(self) -> HostHandle {
        self.memory
    }

    /// The host GPU VA this range is mapped at.
    #[must_use]
    pub const fn host_va(self) -> u64 {
        self.host_va
    }

    /// Which part of [`HostBacking::memory`] this binding covers.
    #[must_use]
    pub const fn extent(self) -> HostExtent {
        self.extent
    }

    /// The slice, if this backing is one. `None` means it owns the whole object.
    #[must_use]
    pub const fn as_slice(self) -> Option<HostSlice> {
        match self.extent {
            HostExtent::Whole => None,
            HostExtent::Slice(s) => Some(s),
        }
    }

    /// ★★ **Does reclaiming this binding free [`HostBacking::memory`]?**
    ///
    /// The single question every reclaim site must ask, asked once, here. `true` only
    /// for [`HostExtent::Whole`]: a slice's object is the arena, which serves other
    /// bindings at other offsets and is freed by its own owner — freeing it on the
    /// first slice's release would pull the backing out from under every sibling slice,
    /// which is the `ALREADY-MAPPED`/use-after-free class one level up.
    #[must_use]
    pub const fn frees_object(self) -> bool {
        matches!(self.extent, HostExtent::Whole)
    }

    /// The isolate whose handle namespace this backing lives in — [`HostHandle`]'s
    /// [`IsolateId`], not a parallel notion (§9.3).
    #[must_use]
    pub const fn owner(self) -> IsolateId {
        self.memory.isolate()
    }

    /// Is this backing usable on `isolate`'s connection? Delegates to
    /// [`HostHandle::belongs_to`], so a slice is scoped exactly as tightly as the arena
    /// object it names and no more loosely.
    #[must_use]
    pub const fn belongs_to(self, isolate: IsolateId) -> bool {
        self.memory.belongs_to(isolate)
    }
}

/// Where a bound VA range points, in core terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    /// Physical/backing address (interpretation depends on `aperture`; for sysmem
    /// this is a guest-physical address).
    pub phys: u64,
    /// Aperture of the backing.
    pub aperture: Aperture,
    /// The host materialization, once the fwd plane has published this range
    /// (`None` = declared by the RPC/CE-capture source only — nothing host-side
    /// exists yet, and nothing host-side needs reclaiming). See [`HostBacking`] for
    /// why this is one `Option` over a pair and not a pair of `Option`s.
    pub host: Option<HostBacking>,
}

impl Binding {
    /// The host GPU VA this range is published at, if it is published at all.
    /// (Convenience over [`Binding::host`]; the #14 gate's predicate.)
    #[must_use]
    pub fn host_va(&self) -> Option<u64> {
        self.host.map(HostBacking::host_va)
    }

    /// The host memory object backing this range, if it is published at all — the
    /// handle a reclaim path must `free`. Its existence is G1's whole point.
    #[must_use]
    pub fn host_memory(&self) -> Option<HostHandle> {
        self.host.map(HostBacking::memory)
    }
}

/// A resolution failure. Every variant is LOUD: callers propagate, they never guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressFault {
    /// No binding covers the VA — the guest's own TLB would have faulted too.
    Miss {
        /// The PDB whose table missed.
        pdb: Pdb,
        /// The faulting VA.
        va: GpuVa,
    },
    /// A bind attempted to overlap a live binding without an eager unbind first
    /// (the `ALREADY-MAPPED` collision class made loud).
    Overlap {
        /// The PDB whose table refused the bind.
        pdb: Pdb,
        /// Start of the attempted range.
        va: GpuVa,
    },
    /// A bind named a malformed range — zero length, or `va + len` wraps `u64`. The
    /// `va`/`len` are guest-controlled (a hostile `MapMemoryDma` or a hostile CE
    /// PT-write dst), so this is a clean loud fault, never an arithmetic panic
    /// (boundary-1 posture; regression-banked).
    Malformed {
        /// The PDB whose table refused the bind.
        pdb: Pdb,
        /// Start of the attempted (malformed) range.
        va: GpuVa,
    },
    /// ★★★ **A host-backed binding whose host GPU VA is not the VA it is bound at**
    /// (`#102`, `eight_blockers_resolved.md` §1).
    ///
    /// The address plane's one identity law: *every binding with a host backing satisfies
    /// `host_va == the VA it is bound at`*. It has to hold because the guest's own
    /// commands are what dereference these addresses — a forwarded pushbuffer names the
    /// guest VA and the host MMU resolves it in the host VAS. A binding that records
    /// "published, but over there" is a mapping the guest can never reach, and it fails
    /// **later and elsewhere**, as `Xid 31 FAULT_PDE` inside a copy engine.
    ///
    /// Refused at [`AddressTable::bind`] — the table's only entrance — so the state is
    /// not merely detectable, it never enters the table.
    HostVaMismatch {
        /// The PDB whose table refused the bind.
        pdb: Pdb,
        /// The VA the range is being bound at (what the host VA must equal).
        va: GpuVa,
        /// The host VA the binding actually carried.
        host_va: u64,
    },
    /// ★★ **A [`HostExtent::Slice`] whose length is not the length of the range it
    /// backs** (`gpga_address_space.md` §8.2).
    ///
    /// The slice's `len` is deliberately redundant with the bound range's `len` — it has
    /// to be, because a [`HostBacking`] travels standalone. Redundancy that is never
    /// checked is just drift waiting to happen, and the drift is not benign: the slice
    /// coordinates are what an arena's free path will use to work out which bytes came
    /// back, so a slice claiming fewer bytes than it holds silently strands the
    /// remainder, and one claiming more hands the next requester bytes that are still
    /// mapped. Refused at [`AddressTable::bind`] — the table's only entrance.
    ///
    /// Near neighbour of [`AddressFault::Malformed`] and deliberately distinct: that one
    /// means the **guest's range** is nonsense, this one means **our own bookkeeping**
    /// disagrees with itself.
    SliceLenMismatch {
        /// The PDB whose table refused the bind.
        pdb: Pdb,
        /// The VA the range is being bound at.
        va: GpuVa,
        /// The length of the range being bound.
        len: u64,
        /// The length the slice claimed.
        slice_len: u64,
    },
}

/// The forward-populated VA→backing table of ONE VAS (identified by its PDB).
///
/// This *is* the guest's GMMU TLB from the emulator's point of view: populated at
/// the guest's own publication points, invalidated by the guest's own invalidate
/// discipline, faulting where real hardware would fault.
#[derive(Debug, Default)]
pub struct AddressTable {
    map: IntervalMap<Binding>,
}

impl AddressTable {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Forward-populate `[va, va+len)` → `binding` (bind-time RPC or CE-PT-write
    /// commit point). Overlap with a live binding is a loud fault: the guest must
    /// have unbound first (unmap eager); silently replacing would hide the #14
    /// collision class.
    pub fn bind(
        &mut self,
        pdb: Pdb,
        va: GpuVa,
        len: u64,
        binding: Binding,
    ) -> Result<(), AddressFault> {
        // ★★★ #102 — ADDRESS IDENTITY, at the table's only entrance. Checked before the
        // range is inserted, so a binding that claims a host publication somewhere other
        // than its own VA is not merely reportable — it is never in the table for a
        // resolve to hand out, and never in the ring gate for a doorbell to pass on.
        if let Some(h) = binding.host
            && h.host_va() != va.0
        {
            return Err(AddressFault::HostVaMismatch {
                pdb,
                va,
                host_va: h.host_va(),
            });
        }
        // ★★ §8.2 — EXTENT AGREEMENT. A slice's own length must be the length of the
        // range it backs; see [`AddressFault::SliceLenMismatch`] for why the redundancy
        // is checked rather than trusted. `Whole` has nothing to disagree with.
        if let Some(s) = binding.host.and_then(HostBacking::as_slice)
            && s.len() != len
        {
            return Err(AddressFault::SliceLenMismatch {
                pdb,
                va,
                len,
                slice_len: s.len(),
            });
        }
        self.map.insert(va.0, len, binding).map_err(|e| match e {
            kayfabe_util::IntervalError::Overlap { .. } => AddressFault::Overlap { pdb, va },
            // Zero-length / u64-wrapping range from hostile guest input: loud, not a panic.
            kayfabe_util::IntervalError::Empty | kayfabe_util::IntervalError::Wraps => {
                AddressFault::Malformed { pdb, va }
            }
        })
    }

    /// Eagerly drop the binding starting at `va`. Returns the dropped binding so the
    /// caller can retire its host backing (reclaim deferred).
    ///
    /// ★ G1 (§12.16): the returned [`Binding::host`] is what makes "retire its host
    /// backing" an executable sentence rather than an aspiration — it names both the
    /// mapping to undo and the object to free.
    pub fn unbind(&mut self, va: GpuVa) -> Option<(u64, Binding)> {
        self.map.remove_at(va.0)
    }

    /// Resolve `va` to its binding + offset within it. **MISS = FAULT** — there is no
    /// fallback and never will be (see crate docs).
    pub fn resolve(&self, pdb: Pdb, va: GpuVa) -> Result<(Binding, u64), AddressFault> {
        match self.map.lookup(va.0) {
            Some((start, _len, b)) => Ok((*b, va.0 - start)),
            None => Err(AddressFault::Miss { pdb, va }),
        }
    }

    /// Iterate bindings as `(va, len, &binding)` in ascending VA order.
    pub fn iter(&self) -> impl Iterator<Item = (u64, u64, &Binding)> {
        self.map.iter()
    }

    /// ★★★ **The range algebra's one primitive** (`#102` stage C2,
    /// `eight_blockers_resolved.md` §12.3): partition `[va, va+len)` into the maximal
    /// runs over which this table's answer is CONSTANT — each either covered by exactly
    /// one binding (`Some`, with the offset already *inside* that binding, so a caller
    /// never recomputes it) or a hole (`None`).
    ///
    /// This exists because §12.3's ruling is *"the operand ranges must be PARTITIONED,
    /// not classified whole"*: one privileged copy-engine request can cover fabricated
    /// and real memory at once, and classifying it by its start address answers for the
    /// wrong half of it. [`AddressTable::resolve`] is the point query; this is the
    /// range query, and it is deliberately here rather than reimplemented over
    /// [`AddressTable::iter`] at each call site — a partition that is not TOTAL is
    /// silently a dropped sub-copy.
    ///
    /// # Guarantees (all pinned by test)
    /// - Ascending, contiguous, non-overlapping, **no zero-length span**.
    /// - The spans cover the *effective* range EXACTLY — nothing implicit is dropped.
    /// - Total on hostile input: never panics, never allocates unboundedly.
    ///
    /// ★ **A wrapping range is CLIPPED at the top of the address space, never wrapped.**
    /// `va + len` is computed in `u128`; the effective end is `min(va+len, 2^64)`. A
    /// copy that ran off the top and resumed at address 0 is not something a real engine
    /// does, and honouring the wrap would let a hostile length reach a mapping at the
    /// BOTTOM of the space from a request aimed at the top. The clipped surplus
    /// addresses nothing, so it needs no span. (`len == 0`, likewise, yields no spans:
    /// an empty request is empty, not a fault — the guest is allowed to ask for nothing.)
    #[must_use]
    pub fn spans(&self, va: GpuVa, len: u64) -> Vec<(u64, u64, Option<Binding>)> {
        let start = u128::from(va.0);
        let end = (start + u128::from(len)).min(1u128 << 64);
        let mut out = Vec::new();
        let mut at = start;
        while at < end {
            let here = at as u64;
            match self.map.lookup(here) {
                Some((b_start, b_len, b)) => {
                    // The covered run ends at the binding's end or the request's, first.
                    let b_end = u128::from(b_start) + u128::from(b_len);
                    let run_end = b_end.min(end);
                    out.push((here, (run_end - at) as u64, Some(*b)));
                    at = run_end;
                }
                None => {
                    // A hole: it runs until the next binding that STARTS inside the
                    // request, or to the end of the request. Derived from the map rather
                    // than probed byte-by-byte — a per-byte scan of a 4 GiB copy is the
                    // same shape as the C's O(n) overlay scan that ate 42% of a run.
                    let next = self
                        .map
                        .iter()
                        .map(|(s, _, _)| u128::from(s))
                        .find(|&s| s > at)
                        .unwrap_or(end)
                        .min(end);
                    out.push((here, (next - at) as u64, None));
                    at = next;
                }
            }
        }
        out
    }

    /// ★★★ #102 — the identity law as a **whole-table walk**: the first host-backed
    /// binding whose host VA is not the VA it is bound at, or `None` if the table is
    /// clean.
    ///
    /// [`AddressTable::bind`] already refuses such a binding, so in a correct build this
    /// always answers `None`. It exists anyway, and is not redundant: `bind` proves the
    /// law about *every write that went through it*, and this proves it about the
    /// *table*. Those differ the moment anything reaches the map another way — a future
    /// bulk-populate path, a deserialize, a merge. A law with only an entry check is one
    /// refactor away from being a law about nothing.
    ///
    /// ★ It walks **both** entrance laws, for the same reason it walks the first one:
    /// [`AddressTable::bind`] also pins a slice's length to its range
    /// ([`AddressFault::SliceLenMismatch`]), and a law with only an entry check is one
    /// bulk-populate path away from being a law about nothing.
    ///
    /// # Errors
    /// [`AddressFault::HostVaMismatch`] or [`AddressFault::SliceLenMismatch`] naming the
    /// offending range.
    pub fn audit_identity(&self, pdb: Pdb) -> Result<(), AddressFault> {
        for (va, len, b) in self.map.iter() {
            let Some(h) = b.host else { continue };
            if h.host_va() != va {
                return Err(AddressFault::HostVaMismatch {
                    pdb,
                    va: GpuVa(va),
                    host_va: h.host_va(),
                });
            }
            if let Some(s) = h.as_slice()
                && s.len() != len
            {
                return Err(AddressFault::SliceLenMismatch {
                    pdb,
                    va: GpuVa(va),
                    len,
                    slice_len: s.len(),
                });
            }
        }
        Ok(())
    }
}

// The concurrency contract, compile-time-asserted (decision #17).
kayfabe_util::assert_send_sync!(
    AddressTable,
    Binding,
    HostBacking,
    HostExtent,
    HostSlice,
    HostSliceError,
    AddressFault,
    walker::WalkResult
);

#[cfg(test)]
mod tests {
    use super::*;

    const PDB: Pdb = Pdb(0x340_1000);

    /// testing strategy §2.4 `taddr_miss_is_fault`: a lookup miss is a loud fault,
    /// never an opportunistic walk.
    #[test]
    fn taddr_miss_is_fault() {
        let mut t = AddressTable::new();
        t.bind(
            PDB,
            GpuVa(0x2_0020_0000),
            0x10000,
            Binding {
                phys: 0x8000_0000,
                aperture: Aperture::SysmemCoherent,
                host: None,
            },
        )
        .unwrap();
        // In range: resolves with offset.
        let (b, off) = t.resolve(PDB, GpuVa(0x2_0020_4000)).unwrap();
        assert_eq!((b.phys, off), (0x8000_0000, 0x4000));
        // Out of range: FAULT, carrying the identity needed for a loud diagnostic.
        assert_eq!(
            t.resolve(PDB, GpuVa(0x2_0030_0000)),
            Err(AddressFault::Miss {
                pdb: PDB,
                va: GpuVa(0x2_0030_0000)
            })
        );
    }

    /// `taddr_unmap_eager`: a removed range faults immediately; rebinding after an
    /// eager unbind succeeds; rebinding over a live range is a loud overlap fault.
    #[test]
    fn taddr_unmap_eager_and_overlap_loud() {
        let mut t = AddressTable::new();
        let bind = Binding {
            phys: 0x1000,
            aperture: Aperture::Vidmem,
            host: None,
        };
        t.bind(PDB, GpuVa(0x1000), 0x1000, bind).unwrap();
        assert_eq!(
            t.bind(PDB, GpuVa(0x1800), 0x1000, bind),
            Err(AddressFault::Overlap {
                pdb: PDB,
                va: GpuVa(0x1800)
            }),
            "re-point without unbind must be loud"
        );
        assert!(t.unbind(GpuVa(0x1000)).is_some());
        assert!(matches!(
            t.resolve(PDB, GpuVa(0x1000)),
            Err(AddressFault::Miss { .. })
        ));
        t.bind(PDB, GpuVa(0x1800), 0x1000, bind).unwrap();
    }

    /// ★★★ `#102` — **`audit_identity` fires.**
    ///
    /// [`AddressTable::bind`] refuses a host-backed binding whose host VA is not the VA
    /// it is bound at, which means **no caller of the public API can build a table this
    /// walk fails on** — so the walk cannot be fired from an integration test, and a
    /// control that cannot be fired is not a control. It is fired here, from inside the
    /// crate, by writing the private map directly.
    ///
    /// That is not cheating: it is *exactly* the situation the walk exists for. `bind`
    /// proves the law about every write that went through it; the walk proves it about
    /// the table. They differ the moment anything else reaches `self.map` — a bulk
    /// populate path, a deserialize, a merge — which is a plausible next commit, not a
    /// hypothetical.
    ///
    /// **Instrument check, performed 2026-07-30:** with `audit_identity`'s body replaced
    /// by `Ok(())`, this test fails on the negative assertion. Restored.
    #[test]
    fn taddr_audit_identity_catches_a_binding_that_bypassed_the_entrance() {
        let mem = kayfabe_isolate::HostHandle::new(
            kayfabe_isolate::IsolateId::new(1, kayfabe_arch::ids::GpuId::ZERO),
            9,
        );
        let honest = |va: u64| Binding {
            phys: 0x8000_0000 + va,
            aperture: Aperture::SysmemCoherent,
            host: Some(HostBacking::whole(mem, va)),
        };

        let mut t = AddressTable::new();
        for k in 0..4u64 {
            let va = 0x2_0020_0000 + k * 0x10000;
            t.bind(PDB, GpuVa(va), 0x10000, honest(va)).unwrap();
        }
        assert_eq!(t.audit_identity(PDB), Ok(()), "a clean table audits clean");

        // Bypass the entrance, the way a future populate path would.
        let rogue_va = 0x2_0080_0000u64;
        t.map
            .insert(
                rogue_va,
                0x1000,
                Binding {
                    phys: 0x1234_0000,
                    aperture: Aperture::Vidmem,
                    // Published one page away from where it is bound — the exact
                    // state that reads as "mapped" everywhere in core state and
                    // resolves to nothing on the host GPU.
                    host: Some(HostBacking::whole(mem, rogue_va + 0x1000)),
                },
            )
            .expect("the private map takes it — that is the whole premise");
        assert_eq!(
            t.audit_identity(PDB),
            Err(AddressFault::HostVaMismatch {
                pdb: PDB,
                va: GpuVa(rogue_va),
                host_va: rogue_va + 0x1000,
            }),
            "the walk names the offending range, so the diagnostic is actionable"
        );
    }

    fn mem(raw: u64) -> kayfabe_isolate::HostHandle {
        kayfabe_isolate::HostHandle::new(
            kayfabe_isolate::IsolateId::new(1, kayfabe_arch::ids::GpuId::ZERO),
            raw,
        )
    }

    /// §8.2 — the two ranges that are not ranges are refused **at construction**, by
    /// their exact variant. `Empty` and `Wraps` are different mistakes.
    ///
    /// **Instrument check, performed 2026-07-30 — WATCHED IT FAIL.** With both validation
    /// blocks deleted from [`HostSlice::new`]:
    ///
    /// ```text
    /// assertion `left == right` failed
    ///   left: Ok(HostSlice { offset: 4096, len: 0 })
    ///  right: Err(Empty)
    /// ```
    ///
    /// Restored.
    #[test]
    fn hostslice_refuses_the_two_non_ranges() {
        assert_eq!(HostSlice::new(0x1000, 0), Err(HostSliceError::Empty));
        assert_eq!(HostSlice::new(u64::MAX, 1), Err(HostSliceError::Wraps));
        assert_eq!(HostSlice::new(u64::MAX - 1, 2), Err(HostSliceError::Wraps));
        // …and the last representable range is accepted, so `end()` cannot overflow.
        assert_eq!(
            HostSlice::new(u64::MAX - 1, 1).map(HostSlice::end),
            Ok(u64::MAX)
        );
        let s = HostSlice::new(0x2_0000, 0x1000).expect("a real range");
        assert_eq!((s.offset(), s.len(), s.end()), (0x2_0000, 0x1000, 0x2_1000));
    }

    /// ★★ §8.2 — **`bind` refuses a slice whose length is not its range's.**
    ///
    /// **Instrument check, performed 2026-07-30 — WATCHED IT FAIL.** With the
    /// `SliceLenMismatch` block short-circuited in [`AddressTable::bind`]:
    ///
    /// ```text
    /// assertion `left == right` failed: our own bookkeeping must not disagree with itself
    ///   left: Ok(())
    ///  right: Err(SliceLenMismatch { pdb: Pdb(54530048), va: GpuVa(8592031744),
    ///                                len: 65536, slice_len: 4096 })
    /// ```
    ///
    /// Restored.
    #[test]
    fn taddr_bind_refuses_a_slice_that_disagrees_with_its_range() {
        let mut t = AddressTable::new();
        let va = GpuVa(0x2_0020_0000);
        let arena = mem(7);
        let short = HostSlice::new(0x8000, 0x1000).expect("real");
        assert_eq!(
            t.bind(
                PDB,
                va,
                0x10000,
                Binding {
                    phys: 0x8000_0000,
                    aperture: Aperture::Vidmem,
                    host: Some(HostBacking::slice(arena, va.0, short)),
                },
            ),
            Err(AddressFault::SliceLenMismatch {
                pdb: PDB,
                va,
                len: 0x10000,
                slice_len: 0x1000,
            }),
            "our own bookkeeping must not disagree with itself"
        );
        // …and the refusal is not a blanket one: the same slice at its own length binds.
        let honest = HostSlice::new(0x8000, 0x10000).expect("real");
        assert_eq!(
            t.bind(
                PDB,
                va,
                0x10000,
                Binding {
                    phys: 0x8000_0000,
                    aperture: Aperture::Vidmem,
                    host: Some(HostBacking::slice(arena, va.0, honest)),
                },
            ),
            Ok(())
        );
        assert_eq!(t.audit_identity(PDB), Ok(()));
    }

    /// ★★ The whole-table walk carries the slice law too, fired the only way it can be
    /// fired — by writing the private map, exactly as the `#102` arm above does.
    ///
    /// **Instrument check, performed 2026-07-30 — WATCHED IT FAIL.** With the
    /// `SliceLenMismatch` block short-circuited in `audit_identity`:
    ///
    /// ```text
    /// assertion `left == right` failed
    ///   left: Ok(())
    ///  right: Err(SliceLenMismatch { pdb: Pdb(54530048), va: GpuVa(8598323200),
    ///                                len: 4096, slice_len: 8192 })
    /// ```
    ///
    /// Restored.
    #[test]
    fn taddr_audit_identity_catches_a_slice_that_bypassed_the_entrance() {
        let mut t = AddressTable::new();
        let va = 0x2_0080_0000u64;
        t.map
            .insert(
                va,
                0x1000,
                Binding {
                    phys: 0x1234_0000,
                    aperture: Aperture::Vidmem,
                    host: Some(HostBacking::slice(
                        mem(9),
                        va,
                        HostSlice::new(0, 0x2000).expect("real"),
                    )),
                },
            )
            .expect("the private map takes it — that is the whole premise");
        assert_eq!(
            t.audit_identity(PDB),
            Err(AddressFault::SliceLenMismatch {
                pdb: PDB,
                va: GpuVa(va),
                len: 0x1000,
                slice_len: 0x2000,
            })
        );
    }

    /// §8.2/§9.3 — the ownership regime and the owner scope are both readable from the
    /// backing, and the scope is [`HostHandle`]'s, not a second one.
    #[test]
    fn host_backing_states_who_frees_it_and_whose_it_is() {
        let a = kayfabe_isolate::IsolateId::new(1, kayfabe_arch::ids::GpuId::ZERO);
        let b = kayfabe_isolate::IsolateId::new(2, kayfabe_arch::ids::GpuId::ZERO);
        let arena = kayfabe_isolate::HostHandle::new(a, 0x5c00_0001);

        let whole = HostBacking::whole(arena, 0x1000);
        assert!(whole.frees_object(), "sole owner: its release frees it");
        assert_eq!(whole.as_slice(), None);
        assert_eq!(whole.extent(), HostExtent::Whole);

        let s = HostSlice::new(0x4000, 0x1000).expect("real");
        let slice = HostBacking::slice(arena, 0x1000, s);
        assert!(
            !slice.frees_object(),
            "a slice never frees the arena it was cut from"
        );
        assert_eq!(slice.as_slice(), Some(s));
        assert_eq!(slice.extent(), HostExtent::Slice(s));

        // The scope is inherited: same answer as the handle's own gate, both ways.
        for backing in [whole, slice] {
            assert_eq!(backing.owner(), a);
            assert!(backing.belongs_to(a));
            assert!(
                !backing.belongs_to(b),
                "a slice must not be nameable from another isolate"
            );
            assert_eq!(backing.belongs_to(b), backing.memory().belongs_to(b));
        }
    }
}
