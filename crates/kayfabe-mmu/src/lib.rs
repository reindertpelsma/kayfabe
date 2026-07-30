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
use kayfabe_isolate::HostHandle;
use kayfabe_util::IntervalMap;

/// ★ **The host materialization of one binding — allocation and placement, together**
/// (`l1_concurrency.md` §12.16, gap G1).
///
/// A published range is backed by TWO host facts: an allocated host memory object
/// (`memory`) and the GPU VA it is mapped at inside the owning `Vas`'s own host VAS
/// (`host_va`). Reclaiming it needs BOTH — `unmap_gpu_va(vas, host_va)` then
/// `free(memory)` — so they are ONE value, not two fields.
///
/// **Why a struct rather than a second `Option` on [`Binding`].** With
/// `memory: Option<HostHandle>` beside `host_va: Option<u64>`, the state
/// *"mapped somewhere, owning nothing freeable"* is representable — and that state
/// was precisely the G1 defect: `commit_publish` stored the mapped VA and dropped the
/// `HostHandle` on the floor, so the majority of allocated host bytes existed in no
/// core state and no reclaim path could ever name them. Folding the pair into one
/// `Option<HostBacking>` makes **bound-but-unfreeable unrepresentable**: you cannot
/// write a host VA into a binding without also writing the handle that frees it.
/// (House pattern — `GpaSpace::release(arena)`-by-value: prefer the type over the
/// runtime check.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostBacking {
    /// The host memory object this range was allocated from, in the OWNING isolate's
    /// handle namespace (`(Proc, GpuId)`-scoped — a handle from another isolate is a
    /// different object, boundary 2).
    pub memory: HostHandle,
    /// The host GPU VA it is mapped at, inside the owning `Vas`'s own host VAS.
    /// Per-Vas host placement is #14's proven fix — see `kayfabe-fwd`.
    pub host_va: u64,
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
        self.host.map(|h| h.host_va)
    }

    /// The host memory object backing this range, if it is published at all — the
    /// handle a reclaim path must `free`. Its existence is G1's whole point.
    #[must_use]
    pub fn host_memory(&self) -> Option<HostHandle> {
        self.host.map(|h| h.memory)
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
            && h.host_va != va.0
        {
            return Err(AddressFault::HostVaMismatch {
                pdb,
                va,
                host_va: h.host_va,
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
    /// # Errors
    /// [`AddressFault::HostVaMismatch`] naming the offending range.
    pub fn audit_identity(&self, pdb: Pdb) -> Result<(), AddressFault> {
        for (va, _len, b) in self.map.iter() {
            if let Some(h) = b.host
                && h.host_va != va
            {
                return Err(AddressFault::HostVaMismatch {
                    pdb,
                    va: GpuVa(va),
                    host_va: h.host_va,
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
            host: Some(HostBacking {
                memory: mem,
                host_va: va,
            }),
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
                    host: Some(HostBacking {
                        memory: mem,
                        // Published one page away from where it is bound — the exact
                        // state that reads as "mapped" everywhere in core state and
                        // resolves to nothing on the host GPU.
                        host_va: rogue_va + 0x1000,
                    }),
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
}
