//! ★★★ **A simulated host RM for ONE host VA space — the GPU-free proof of the batched path**
//! (owner direction 2026-10-09: "the batching logic must be tested properly WITHOUT a GPU").
//!
//! The model implements what host RM does with OUR map/unmap verbs in one client, read from
//! `ogkm-595.84` (the host runs 595.91.07):
//!
//! - **Two kinds of `hDma`.** The space's `range` is an `NV01_MEMORY_VIRTUAL`, constructed with
//!   `bReserveVaOnAlloc = NV_FALSE` (`virtual_mem.c:349-354`): EVERY map through it allocates its
//!   own VA heap block, and every unmap frees "the block containing the unmapped part's start"
//!   (`virt_mem_allocator_gm107.c:1635-1638` → `gvaspaceFree`, `gpu_vaspace.c:1631-1640`, whose
//!   `eheapGetBlock` is a containment search, `eheap_old.c:1005-1027`). A reservation is an
//!   `NV50_MEMORY_VIRTUAL` (`bReserveVaOnAlloc = NV_TRUE`, `virtual_mem.c:593`) that owns ONE heap
//!   block for its whole VA (a FIXED `eheapAlloc`, `gpu_vaspace.c:1374-1386`, refused with
//!   `NV_ERR_NO_MEMORY` over any existing block); an unmap inside it only invalidates the PTEs of
//!   exactly the unmapped part (`virt_mem_allocator_gm107.c:1578-1633`).
//! - **A FIXED map fails on occupancy** (`VA_ALREADY_MAPPED`; production answers `HeldByHost`): in
//!   the `NV01` range a map needs a free heap block, so it also fails over a reservation.
//! - **`size == 0`** unmaps the ONE mapping that starts exactly at `va`, else `OBJECT_NOT_FOUND`
//!   (`rs_server.c:2436-2452`).
//! - **`size != 0`** removes EVERY mapping of the `hDma` that intersects the range — whoever placed
//!   it — splitting a straddler into its outside remnants, and answers `NV_OK` over gaps
//!   (`rs_server.c:2453-2514`, `2316-2416`; `virtual_mem.c:1697-1804`).
//! - **The consequence the model exists for:** a range unmap that SPLITS a mapping of the `NV01`
//!   range frees the WHOLE original VA block — the remnants RM still lists lose their PTEs (host
//!   Xid 31 `FAULT_PTE`), and freeing a remnant later finds no block (`NV_ASSERT(NULL !=
//!   pMemBlock) @ gpu_vaspace.c:1639`). Both were MEASURED in Windows runs 242/243 (batched on).
//! - **Freeing a memory object** unmaps every mapping of it; **freeing a reservation** unmaps every
//!   mapping inside it and returns its block.
//!
//! Tests may place FOREIGN mappings (what kayfabe's other code maps through the same client and
//! `hDma`); a mirror verb that removes or splits one is a VIOLATION. And a test may arm a GUARD —
//! the VAs whose guest mapping does not change in this refresh — that is checked after EVERY host
//! call: an unchanged VA that is transiently unmapped is a violation (owner, 2026-10-09: "every VA
//! whose guest mapping is UNCHANGED must stay accessible at ALL times").

use crate::apply::{ApplyCfg, DiffRun, apply_entry};
use crate::batch::{BatchedVas, SpaceVerbs};
use crate::ledger::{AP_SYS_COHERENT, AP_VIDMEM, Desired, MapTarget, Mapped, NOT_BATCHED, SkedRow};
use kf_cuda::abi::{KFWR_ACK_APPLIED, KFWR_ACK_HELD};
use kf_host::channel::GuestVaRange;
use kf_host::{MapPerm, VaSpace};
use std::cell::RefCell;
use std::collections::BTreeMap;

/// The model's page.
pub const P: u64 = 0x1000;
/// The space's `NV01_MEMORY_VIRTUAL` range (`bReserveVaOnAlloc = false`).
pub const RANGE: u32 = 0x100;
/// The static guest reservation (`NV50_MEMORY_VIRTUAL`, `bReserveVaOnAlloc = true`).
pub const RESV: u32 = 0x200;
const RAM_OBJ: u32 = 0x10;
const STORE_OBJ: u32 = 0x11;
const FOREIGN_OBJ: u32 = 0x12;

/// Who placed a host mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// The mirror under test (every [`SpaceVerbs`] call).
    Mirror,
    /// Other kayfabe code in the same client + `hDma` (test-placed).
    Foreign(u32),
}

/// One mapping in host RM's per-`hDma` list.
#[derive(Debug, Clone)]
pub struct SimMap {
    /// Start VA.
    pub va: u64,
    /// Bytes.
    pub len: u64,
    /// The `hDma` it was mapped through.
    pub hdma: u32,
    /// The memory object.
    pub obj: u32,
    /// Offset of `va` inside `obj`.
    pub off: u64,
    /// Who placed it.
    pub owner: Owner,
    /// Its PTEs were released while RM still lists it (the `NV01` split hazard).
    pub broken: bool,
}

/// What one model page translates to: `(ram, backing offset)`.
pub type Backing = (bool, u64);

/// Host calls and the stitch's mm syscalls, as production would issue them (the syscall budget).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counters {
    /// `NV_ESC_RM_MAP_MEMORY_DMA` (one per `hDma` piece).
    pub rm_map: u64,
    /// `NV_ESC_RM_UNMAP_MEMORY_DMA` (one per `hDma` piece).
    pub rm_unmap: u64,
    /// `NV_ESC_RM_ALLOC_MEMORY` of a batch's OS descriptor.
    pub rm_desc: u64,
    /// `NV_ESC_RM_ALLOC` of a micro reservation.
    pub rm_reserve: u64,
    /// `NV_ESC_RM_FREE` (batch objects and reservations).
    pub rm_free: u64,
    /// `mmap` calls of the stitch (one `PROT_NONE` reservation + one `MAP_FIXED` per
    /// file-discontiguous piece).
    pub mmap: u64,
    /// `munmap` of a stitched view (one per batch; O(pieces) kernel work).
    pub munmap: u64,
    /// VMAs those `munmap`s tore down.
    pub munmap_vmas: u64,
    /// Bytes pinned through stitched views.
    pub stitched_bytes: u64,
}

impl Counters {
    /// Every RM ioctl.
    #[must_use]
    pub fn rm_total(&self) -> u64 {
        self.rm_map + self.rm_unmap + self.rm_desc + self.rm_reserve + self.rm_free
    }
}

/// The host RM state of one space.
#[derive(Debug)]
pub struct SimRm {
    /// The space (its static reservations decide each piece's `hDma`, as production's
    /// `dma_pieces`).
    pub space: VaSpace,
    /// Every mapping RM lists, any `hDma`.
    pub maps: Vec<SimMap>,
    /// VA heap blocks, `start → end` (`NV01` per-map blocks and reservation blocks).
    pub blocks: BTreeMap<u64, u64>,
    /// Micro reservations, `handle → (lo, hi)`.
    pub resv: BTreeMap<u32, (u64, u64)>,
    /// Batch objects: page → backing.
    pub objs: BTreeMap<u32, Vec<Backing>>,
    next_obj: u32,
    /// Mirror verbs that removed or split a foreign mapping.
    pub violations: Vec<String>,
    /// Unchanged VAs found unmapped (or re-pointed) after a host call.
    pub transient: Vec<String>,
    /// The VA of each [`SimRm::transient`] entry, in order.
    pub transient_va: Vec<u64>,
    /// ★ Review fix 2026-10-10 (finding 8): host RM refuses every `n`-th mirror UNMAP call (any
    /// verb); `after` = it acted first (all of it) and still answered an error — the superset of
    /// `serverInterUnmapInternal`'s mid-loop `goto done`.
    pub fail_unmaps: Option<UnmapFault>,
    /// Unmap calls refused by [`SimRm::fail_unmaps`].
    pub injected: u64,
    /// The pages that must keep their translation after every host call (the guard).
    pub guard: Vec<(u64, Backing)>,
    /// `gvaspaceFree` found no block (`NV_ASSERT @ gpu_vaspace.c:1639`).
    pub asserts: u32,
    /// Every mirror range unmap, `(va, len)`.
    pub ranges: Vec<(u64, u64)>,
    /// Bytes mirror range unmaps covered where nothing at all was mapped.
    pub gap_bytes: u64,
    /// Every micro reservation is refused (a host that does not accept reservations there).
    pub refuse_reserve: bool,
    /// ★ D1/D3: a micro reservation LONGER than this is refused (a host that accepts small
    /// reservations only, or one that finds a block in a big range).
    pub refuse_reserve_over: Option<u64>,
    /// ★ D1/D3 fault injection (runtime refusals): see [`ExtFault`].
    pub ext: Option<ExtFault>,
    /// What [`SimRm::ext`] injected so far.
    pub ext_hits: ExtHits,
    /// Skip the guard (the workload counts only; the property test always guards).
    pub no_guard: bool,
    /// The next stitched-batch map fails after the descriptor is built.
    pub fail_next_batch_map: bool,
    /// Calls issued.
    pub n: Counters,
    /// ★ Review fix 2026-10-10: guard hits inside intervals the apply declared re-made.
    pub remade_transients: u64,
}

/// ★ D1/D3 (2026-10-10): runtime refusals of the micro-reservation machinery, injected. Every
/// `*_every` is "every n-th call of that kind"; 0 = never.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExtFault {
    /// Every n-th micro reservation ALLOC is refused (before acting).
    pub reserve_every: u64,
    /// Every n-th FREE of a micro reservation is refused (before acting: it stays reserved).
    pub free_every: u64,
    /// Every n-th mirror ROW map (`map_row`, `map_row_in`, SKED) fails before acting — a failure
    /// in the middle of a multi-piece row, so the pieces already placed must be rolled back.
    pub map_every: u64,
    /// Calls seen: reserve, free, map.
    pub seen: [u64; 3],
}

/// What [`ExtFault`] injected: reserve allocs refused, reservation frees refused, row maps failed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ExtHits {
    /// Reservation allocs refused.
    pub reserves: u64,
    /// Reservation frees refused.
    pub frees: u64,
    /// Row maps failed.
    pub maps: u64,
}

impl SimRm {
    /// Whether the `kind`-th (0 reserve, 1 free, 2 map) call is refused by [`SimRm::ext`].
    fn ext_hit(&mut self, kind: usize) -> bool {
        let Some(f) = self.ext.as_mut() else {
            return false;
        };
        let every = [f.reserve_every, f.free_every, f.map_every][kind];
        f.seen[kind] += 1;
        let hit = every != 0 && f.seen[kind].is_multiple_of(every);
        if hit {
            match kind {
                0 => self.ext_hits.reserves += 1,
                1 => self.ext_hits.frees += 1,
                _ => self.ext_hits.maps += 1,
            }
        }
        hit
    }
}

/// ★ Review fix 2026-10-10 (finding 8): refused mirror unmaps, injected.
#[derive(Debug, Clone, Copy)]
pub struct UnmapFault {
    /// Every `every`-th mirror unmap call is refused.
    pub every: u64,
    /// A refused RANGE unmap acted first (all of it), then answered the error.
    pub after: bool,
    /// Calls seen.
    pub seen: u64,
}

/// The model behind a `&Sim` [`SpaceVerbs`].
#[derive(Debug)]
pub struct Sim(pub RefCell<SimRm>);

/// A test space with ONE static guest reservation `[resv_lo, resv_hi)`.
#[must_use]
pub fn space(resv_lo: u64, resv_hi: u64) -> VaSpace {
    VaSpace {
        space: 1,
        range: RANGE,
        guest: [
            GuestVaRange {
                handle: RESV,
                lo: resv_lo,
                hi: resv_hi,
            },
            GuestVaRange::default(),
            GuestVaRange::default(),
        ],
    }
}

impl Sim {
    /// A fresh model of `space`.
    #[must_use]
    pub fn new(space: VaSpace) -> Self {
        Sim(RefCell::new(SimRm {
            space,
            maps: Vec::new(),
            blocks: BTreeMap::new(),
            resv: BTreeMap::new(),
            objs: BTreeMap::new(),
            next_obj: 0x1000,
            violations: Vec::new(),
            transient: Vec::new(),
            transient_va: Vec::new(),
            fail_unmaps: None,
            injected: 0,
            guard: Vec::new(),
            asserts: 0,
            ranges: Vec::new(),
            gap_bytes: 0,
            refuse_reserve: false,
            refuse_reserve_over: None,
            ext: None,
            ext_hits: ExtHits::default(),
            no_guard: false,
            fail_next_batch_map: false,
            n: Counters::default(),
            remade_transients: 0,
        }))
    }
}

fn overlaps(a: u64, al: u64, b: u64, bl: u64) -> bool {
    a < b + bl && b < a + al
}

impl SimRm {
    fn pieces(&self, va: u64, len: u64) -> Result<Vec<(u32, u64, u64)>, String> {
        self.space
            .dma_pieces(va, len)
            .map_err(|e| format!("pieces {va:#x}+{len:#x}: {e:?}"))
    }

    /// ★ The guard: every unchanged VA still translates as before. Called after EVERY host call.
    fn check_guard(&mut self, after: &str) {
        let bad: Vec<(u64, String)> = self
            .guard
            .iter()
            .filter(|&&(p, want)| self.translate(p, Owner::Mirror) != Some(want))
            .map(|&(p, want)| {
                (
                    p,
                    format!(
                        "after {after}: UNCHANGED VA {p:#x} transiently unmapped or re-pointed (want {want:x?}, got {:x?})",
                        self.translate(p, Owner::Mirror)
                    ),
                )
            })
            .collect();
        for (p, m) in bad {
            self.transient_va.push(p);
            self.transient.push(m);
        }
    }

    /// ★ Review fix 2026-10-10: whether THIS mirror unmap call is refused ([`SimRm::fail_unmaps`]);
    /// `Some(acted_first)` when it is.
    fn inject_unmap(&mut self) -> Option<bool> {
        let f = self.fail_unmaps.as_mut()?;
        f.seen += 1;
        if f.seen.is_multiple_of(f.every) {
            self.injected += 1;
            Some(f.after)
        } else {
            None
        }
    }

    /// Whether a heap block overlaps `[va, va+len)` (blocks never overlap one another).
    fn block_overlaps(&self, va: u64, len: u64) -> bool {
        self.blocks
            .range(..va + len)
            .next_back()
            .is_some_and(|(_, &e)| e > va)
    }

    fn occupied(&self, hdma: u32, va: u64, len: u64) -> bool {
        if hdma == RANGE {
            self.block_overlaps(va, len)
        } else {
            self.maps
                .iter()
                .any(|m| m.hdma == hdma && overlaps(m.va, m.len, va, len))
        }
    }

    /// FIXED map pieces, all or none, one ioctl each. `Err(true)` = occupied.
    fn map_pieces(
        &mut self,
        pieces: &[(u32, u64, u64)],
        va: u64,
        obj: u32,
        off: u64,
        owner: Owner,
    ) -> Result<(), bool> {
        if pieces.iter().any(|&(h, pv, pl)| self.occupied(h, pv, pl)) {
            self.n.rm_map += 1;
            return Err(true);
        }
        for &(h, pv, pl) in pieces {
            self.n.rm_map += 1;
            if h == RANGE {
                self.blocks.insert(pv, pv + pl);
            }
            self.maps.push(SimMap {
                va: pv,
                len: pl,
                hdma: h,
                obj,
                off: off + (pv - va),
                owner,
                broken: false,
            });
            self.check_guard("map");
        }
        Ok(())
    }

    /// One FIXED map through the space's own routing.
    fn map(&mut self, va: u64, len: u64, obj: u32, off: u64, owner: Owner) -> Result<(), bool> {
        let pieces = self.pieces(va, len).map_err(|_| false)?;
        self.map_pieces(&pieces, va, obj, off, owner)
    }

    /// One FIXED map THROUGH the micro reservation `h` (must lie inside it).
    fn map_through(&mut self, h: u32, va: u64, len: u64, obj: u32, off: u64) -> Result<(), bool> {
        let &(lo, hi) = self.resv.get(&h).ok_or(false)?;
        if va < lo || va + len > hi {
            return Err(false);
        }
        self.map_pieces(&[(h, va, len)], va, obj, off, Owner::Mirror)
    }

    /// `gvaspaceFree(vAddr)`: the block CONTAINING `addr` goes whole; every other `NV01` mapping
    /// that still lies in it loses its PTEs.
    fn vaspace_free(&mut self, addr: u64) {
        let hit = self
            .blocks
            .range(..=addr)
            .next_back()
            .filter(|&(_, &e)| addr < e)
            .map(|(&s, &e)| (s, e));
        let Some((s, e)) = hit else {
            self.asserts += 1;
            return;
        };
        self.blocks.remove(&s);
        for m in self
            .maps
            .iter_mut()
            .filter(|m| m.hdma == RANGE && overlaps(m.va, m.len, s, e - s))
        {
            m.broken = true;
        }
    }

    fn check_owner(&mut self, m: &SimMap, by_mirror: bool, what: &str) {
        if by_mirror && let Owner::Foreign(id) = m.owner {
            self.violations.push(format!(
                "{what}: foreign mapping #{id} {:#x}+{:#x} removed or split by a mirror verb",
                m.va, m.len
            ));
        }
    }

    /// `size == 0` in `hdma` at the exact start `va`.
    fn unmap_whole_in(&mut self, hdma: u32, va: u64, by_mirror: bool) -> Result<(), String> {
        self.n.rm_unmap += u64::from(by_mirror);
        let i = self
            .maps
            .iter()
            .position(|m| m.hdma == hdma && m.va == va)
            .ok_or_else(|| format!("unmap {va:#x}: OBJECT_NOT_FOUND"))?;
        let m = self.maps.remove(i);
        self.check_owner(&m, by_mirror, "whole unmap");
        if hdma == RANGE {
            if m.broken {
                self.asserts += 1;
            } else {
                self.vaspace_free(m.va);
            }
        }
        self.check_guard("whole unmap");
        Ok(())
    }

    /// `size != 0` in `hdma`: every intersecting mapping, split at the edges.
    fn unmap_range_in(&mut self, hdma: u32, a: u64, b: u64, by_mirror: bool) {
        self.n.rm_unmap += u64::from(by_mirror);
        let mut hit: Vec<usize> = self
            .maps
            .iter()
            .enumerate()
            .filter(|(_, m)| m.hdma == hdma && overlaps(m.va, m.len, a, b - a))
            .map(|(i, _)| i)
            .collect();
        hit.sort_unstable();
        let mut taken: Vec<SimMap> = hit.iter().rev().map(|&i| self.maps.remove(i)).collect();
        taken.sort_by_key(|m| m.va);
        for m in taken {
            self.check_owner(&m, by_mirror, "range unmap");
            let (ps, pe) = (m.va.max(a), (m.va + m.len).min(b));
            if m.va < ps {
                self.maps.push(SimMap {
                    len: ps - m.va,
                    ..m.clone()
                });
            }
            if pe < m.va + m.len {
                self.maps.push(SimMap {
                    va: pe,
                    len: m.va + m.len - pe,
                    off: m.off + (pe - m.va),
                    ..m.clone()
                });
            }
            if hdma == RANGE {
                if m.broken {
                    self.asserts += 1;
                } else {
                    self.vaspace_free(ps);
                }
            }
        }
        self.check_guard("range unmap");
    }

    /// Free `h`: a batch object (every mapping of it goes) or a micro reservation (every mapping
    /// inside it goes, its block is returned).
    fn free(&mut self, h: u32) -> Result<(), String> {
        self.n.rm_free += 1;
        if self.resv.contains_key(&h) && self.ext_hit(1) {
            return Err(format!("free {h:#x}: refused (injected)"));
        }
        if self.objs.remove(&h).is_some() {
            while let Some(m) = self.maps.iter().find(|m| m.obj == h).cloned() {
                self.n.rm_unmap -= 1; // part of the free, not a separate ioctl
                let _ = self.unmap_whole_in(m.hdma, m.va, true);
            }
        } else if let Some((lo, _)) = self.resv.remove(&h) {
            self.maps.retain(|m| m.hdma != h);
            self.blocks.remove(&lo);
        } else {
            return Err(format!("free {h:#x}: no such object"));
        }
        self.check_guard("free");
        Ok(())
    }

    /// A FIXED `NV50_MEMORY_VIRTUAL` over `[va, va+len)`: one heap block, refused over any block
    /// or static reservation.
    fn reserve(&mut self, va: u64, len: u64) -> Result<u32, String> {
        self.n.rm_reserve += 1;
        let statics = self
            .space
            .guest
            .iter()
            .any(|g| g.handle != 0 && overlaps(g.lo, g.hi - g.lo, va, len));
        if self.refuse_reserve
            || self.refuse_reserve_over.is_some_and(|m| len > m)
            || self.ext_hit(0)
            || statics
            || self.block_overlaps(va, len)
        {
            return Err(format!("reserve {va:#x}+{len:#x}: NV_ERR_NO_MEMORY"));
        }
        let h = self.next_obj;
        self.next_obj += 1;
        self.blocks.insert(va, va + len);
        self.resv.insert(h, (va, va + len));
        Ok(h)
    }

    /// What VA page `va` translates to through a live (not broken) mapping of `owner`.
    #[must_use]
    pub fn translate(&self, va: u64, owner: Owner) -> Option<Backing> {
        let m = self
            .maps
            .iter()
            .find(|m| !m.broken && m.owner == owner && m.va <= va && va < m.va + m.len)?;
        let o = m.off + (va - m.va);
        Some(match m.obj {
            RAM_OBJ => (true, o),
            STORE_OBJ | FOREIGN_OBJ => (false, o),
            h => *self.objs.get(&h)?.get(usize::try_from(o / P).ok()?)?,
        })
    }

    /// Mirror mappings whose PTEs are gone while RM still lists them.
    #[must_use]
    pub fn broken_mirror(&self) -> Vec<(u64, u64)> {
        self.maps
            .iter()
            .filter(|m| m.broken && m.owner == Owner::Mirror)
            .map(|m| (m.va, m.len))
            .collect()
    }

    /// Place a FOREIGN mapping (other kayfabe code, same client + `hDma`).
    pub fn place_foreign(&mut self, id: u32, va: u64, len: u64) -> bool {
        let n = self.n;
        let ok = self
            .map(va, len, FOREIGN_OBJ, va, Owner::Foreign(id))
            .is_ok();
        self.n = n;
        ok
    }

    /// Its owner removes a foreign mapping (whole-mapping unmaps of its pieces).
    pub fn remove_foreign(&mut self, id: u32) {
        while let Some(m) = self
            .maps
            .iter()
            .find(|m| m.owner == Owner::Foreign(id))
            .cloned()
        {
            let _ = self.unmap_whole_in(m.hdma, m.va, false);
        }
    }

    /// Whether foreign `id` still covers exactly `[va, va+len)`, unbroken.
    #[must_use]
    pub fn foreign_intact(&self, id: u32, va: u64, len: u64) -> bool {
        let mine: Vec<&SimMap> = self
            .maps
            .iter()
            .filter(|m| m.owner == Owner::Foreign(id))
            .collect();
        let bytes: u64 = mine.iter().map(|m| m.len).sum();
        bytes == len
            && mine
                .iter()
                .all(|m| !m.broken && m.va >= va && m.va + m.len <= va + len)
    }

    fn stitch(&mut self, rows: &[Desired]) -> Result<(u32, u64, u64), String> {
        let first = rows.first().ok_or("empty batch")?;
        let mut pages: Vec<Backing> = Vec::new();
        let mut next = first.va;
        let mut pieces = 0u64;
        let mut last_end: Option<u64> = None;
        for d in rows {
            if !d.ram || d.va != next || d.perm != first.perm || d.kind != first.kind {
                return Err(
                    "batch rows not one VA-contiguous same-kind same-perm RAM range".into(),
                );
            }
            next = d.va + d.len;
            if last_end != Some(d.off) {
                pieces += 1;
            }
            last_end = Some(d.off + d.len);
            pages.extend((0..d.len / P).map(|i| (true, d.off + i * P)));
        }
        // The stitch: one PROT_NONE reservation + one MAP_FIXED per file-discontiguous piece; the
        // descriptor; the view's munmap (on the reaper — still the VA thread's mm syscalls).
        self.n.mmap += 1 + pieces;
        self.n.munmap += 1;
        self.n.munmap_vmas += pieces;
        self.n.rm_desc += 1;
        self.n.stitched_bytes += next - first.va;
        let h = self.next_obj;
        self.next_obj += 1;
        self.objs.insert(h, pages);
        Ok((h, first.va, next - first.va))
    }
}

impl SpaceVerbs for &Sim {
    fn map_row(&self, d: &Desired, _defer: bool) -> Result<Mapped, String> {
        if self.0.borrow_mut().ext_hit(2) {
            return Err(format!("map {:#x}+{:#x}: refused (injected)", d.va, d.len));
        }
        let obj = if d.ram { RAM_OBJ } else { STORE_OBJ };
        match self
            .0
            .borrow_mut()
            .map(d.va, d.len, obj, d.off, Owner::Mirror)
        {
            Ok(()) => Ok(Mapped::Placed),
            Err(true) => Ok(Mapped::HeldByHost),
            Err(false) => Err(format!("map {:#x}+{:#x}: refused", d.va, d.len)),
        }
    }
    fn map_sked_row(&self, s: &SkedRow, _defer: bool) -> Result<Mapped, String> {
        if self.0.borrow_mut().ext_hit(2) {
            return Err(format!("map SKED {:#x}: refused (injected)", s.va));
        }
        match self
            .0
            .borrow_mut()
            .map(s.va, s.len, STORE_OBJ, s.off, Owner::Mirror)
        {
            Ok(()) => Ok(Mapped::Placed),
            Err(true) => Ok(Mapped::HeldByHost),
            Err(false) => Err(format!("map SKED {:#x}: refused", s.va)),
        }
    }
    fn map_scattered(
        &self,
        _ram_fd: std::os::fd::BorrowedFd<'_>,
        rows: &[Desired],
        _defer: bool,
    ) -> Result<u32, String> {
        let mut rm = self.0.borrow_mut();
        let (h, va, len) = rm.stitch(rows)?;
        match rm.map(va, len, h, 0, Owner::Mirror) {
            Ok(()) => Ok(h),
            Err(_) => {
                let _ = rm.free(h);
                Err(format!("batch {va:#x}: map refused"))
            }
        }
    }
    fn unmap_whole(&self, va: u64, _defer: bool) -> Result<(), String> {
        let mut rm = self.0.borrow_mut();
        if rm.inject_unmap().is_some() {
            return Err(format!("unmap {va:#x}: refused (injected)"));
        }
        let h = rm.space.dma_for(va, 1).map_err(|e| format!("{e:?}"))?;
        rm.unmap_whole_in(h, va, true)
    }
    fn unmap_row(&self, va: u64, len: u64, _defer: bool) -> Result<(), String> {
        let mut rm = self.0.borrow_mut();
        if rm.inject_unmap().is_some() {
            return Err(format!("unmap row {va:#x}: refused (injected)"));
        }
        let mut first = Ok(());
        for (h, pv, _) in rm.pieces(va, len)? {
            if let Err(e) = rm.unmap_whole_in(h, pv, true) {
                first = first.and(Err(e));
            }
        }
        first
    }
    fn unmap_range(&self, va: u64, len: u64, _defer: bool) -> Result<(), String> {
        let mut rm = self.0.borrow_mut();
        let injected = rm.inject_unmap();
        if injected == Some(false) {
            return Err(format!("unmap range {va:#x}+{len:#x}: refused (injected)"));
        }
        rm.ranges.push((va, len));
        let gap = (0..len / P)
            .filter(|i| {
                let p = va + i * P;
                !rm.maps.iter().any(|m| m.va <= p && p < m.va + m.len)
            })
            .count() as u64;
        rm.gap_bytes += gap * P;
        for (h, pv, pl) in rm.pieces(va, len)? {
            rm.unmap_range_in(h, pv, pv + pl, true);
        }
        if injected == Some(true) {
            return Err(format!(
                "unmap range {va:#x}+{len:#x}: refused AFTER acting (injected)"
            ));
        }
        Ok(())
    }
    fn free(&self, handle: u32) -> Result<(), String> {
        self.0.borrow_mut().free(handle)
    }
    fn splits_safely(&self, va: u64, len: u64) -> bool {
        self.0.borrow().space.guest_reserved(va, len)
    }
    fn reserve(&self, va: u64, len: u64) -> Result<u32, String> {
        self.0.borrow_mut().reserve(va, len)
    }
    fn map_row_in(&self, h: u32, d: &Desired, _defer: bool) -> Result<Mapped, String> {
        if self.0.borrow_mut().ext_hit(2) {
            return Err(format!("map {:#x} in {h:#x}: refused (injected)", d.va));
        }
        let obj = if d.ram { RAM_OBJ } else { STORE_OBJ };
        match self.0.borrow_mut().map_through(h, d.va, d.len, obj, d.off) {
            Ok(()) => Ok(Mapped::Placed),
            Err(true) => Ok(Mapped::HeldByHost),
            Err(false) => Err(format!(
                "map {:#x} in {h:#x}: outside the reservation",
                d.va
            )),
        }
    }
    fn map_sked_in(&self, h: u32, s: &SkedRow, _defer: bool) -> Result<Mapped, String> {
        if self.0.borrow_mut().ext_hit(2) {
            return Err(format!(
                "map SKED {:#x} in {h:#x}: refused (injected)",
                s.va
            ));
        }
        match self
            .0
            .borrow_mut()
            .map_through(h, s.va, s.len, STORE_OBJ, s.off)
        {
            Ok(()) => Ok(Mapped::Placed),
            Err(true) => Ok(Mapped::HeldByHost),
            Err(false) => Err(format!("map SKED {:#x} in {h:#x}: refused", s.va)),
        }
    }
    fn map_scattered_in(
        &self,
        h: u32,
        _ram_fd: std::os::fd::BorrowedFd<'_>,
        rows: &[Desired],
        _defer: bool,
    ) -> Result<u32, String> {
        let mut rm = self.0.borrow_mut();
        let (obj, va, len) = rm.stitch(rows)?;
        let fail = core::mem::take(&mut rm.fail_next_batch_map);
        match (fail, rm.map_through(h, va, len, obj, 0)) {
            (false, Ok(())) => Ok(obj),
            (true, Ok(())) => {
                // Injected: the map "failed" — RM rolled it back (the mapping is not listed).
                rm.maps.retain(|m| m.obj != obj);
                let _ = rm.free(obj);
                Err(format!("batch {va:#x} in {h:#x}: map refused (injected)"))
            }
            (_, Err(_)) => {
                let _ = rm.free(obj);
                Err(format!("batch {va:#x} in {h:#x}: map refused"))
            }
        }
    }
    fn unmap_in(&self, h: u32, va: u64, size: u64, _defer: bool) -> Result<(), String> {
        let mut rm = self.0.borrow_mut();
        let injected = rm.inject_unmap();
        if size == 0 {
            if injected.is_some() {
                return Err(format!("unmap {va:#x} in {h:#x}: refused (injected)"));
            }
            rm.unmap_whole_in(h, va, true)
        } else {
            if injected == Some(false) {
                return Err(format!(
                    "unmap {va:#x}+{size:#x} in {h:#x}: refused (injected)"
                ));
            }
            rm.ranges.push((va, size));
            rm.unmap_range_in(h, va, va + size, true);
            if injected == Some(true) {
                return Err(format!(
                    "unmap {va:#x}+{size:#x} in {h:#x}: refused AFTER acting (injected)"
                ));
            }
            Ok(())
        }
    }
}

/// ★ The production glue of `kf_qemu::mem::GpuMirror`'s [`MapTarget`] impl, over the model: a row
/// record beside [`BatchedVas`], the per-run unmap through `unmap_run`, the range through
/// [`BatchedVas::unmap_range`], and the retire teardown (`unmap_all_rows`). Every host verb goes
/// through the real [`BatchedVas`].
pub struct SimMirror<'a> {
    /// The real batch / ownership code, over the model.
    pub bv: BatchedVas<'static, &'a Sim>,
    /// Our rows (`PlacedRows`), `va → len`.
    pub rows: RefCell<BTreeMap<u64, u64>>,
    /// `KF3_NO_BATCHED_MAP` unset.
    pub batching: bool,
}

impl<'a> SimMirror<'a> {
    /// A fresh mirror over `sim`; `low_reserve` batches the unreserved range through micro
    /// reservations.
    #[must_use]
    pub fn new(sim: &'a Sim, batching: bool, low_reserve: bool) -> Self {
        SimMirror {
            bv: BatchedVas::with_low_reserve(sim, low_reserve),
            rows: RefCell::new(BTreeMap::new()),
            batching,
        }
    }

    /// The model under this mirror.
    #[must_use]
    pub fn sim(&self) -> &'a Sim {
        self.bv.vas
    }

    /// `GpuMirror::unmap_all_rows` + `drain` (the retire).
    pub fn retire(&self) -> usize {
        let rows: Vec<(u64, u64)> = self.rows.borrow().iter().map(|(&v, &l)| (v, l)).collect();
        let mut refused = 0;
        let mut k = 0;
        while k < rows.len() {
            let mut j = k + 1;
            while j < rows.len() && rows[j - 1].0 + rows[j - 1].1 == rows[j].0 {
                j += 1;
            }
            let (va, end) = (rows[k].0, rows[j - 1].0 + rows[j - 1].1);
            if j - k < 2 || self.unmap_range(va, end - va, true).is_err() {
                for &(va, _) in &rows[k..j] {
                    if self.unmap(va, true).is_err() {
                        refused += 1;
                    }
                }
            }
            k = j;
        }
        let _ = self.bv.drain();
        refused
    }
}

impl MapTarget for SimMirror<'_> {
    fn gpu_space(&self) -> bool {
        true
    }
    fn map(&self, d: &Desired, defer: bool) -> Result<Mapped, String> {
        let m = self.bv.map(d, defer);
        // ★ Review fix 2026-10-10: what the batch layer found there was a stray — never a row a
        // reader may resolve through; the rows follow the ledger.
        restore_owned_rows(&mut self.rows.borrow_mut(), &self.bv, d.va, d.va + d.len);
        if m == Ok(Mapped::Placed) {
            self.rows.borrow_mut().insert(d.va, d.len);
        }
        m
    }
    fn map_batch(&self, rows: &[Desired], defer: bool) -> Result<(), String> {
        if !self.batching {
            return Err(NOT_BATCHED.into());
        }
        let res = self
            .bv
            .place(std::os::fd::AsFd::as_fd(&std::io::stdin()), rows, defer);
        if let (Some(a), Some(b)) = (rows.first(), rows.last()) {
            restore_owned_rows(&mut self.rows.borrow_mut(), &self.bv, a.va, b.va + b.len);
        }
        res?;
        let mut r = self.rows.borrow_mut();
        for d in rows {
            r.insert(d.va, d.len);
        }
        Ok(())
    }
    fn unmap(&self, va: u64, defer: bool) -> Result<(), String> {
        let Some(len) = self.rows.borrow_mut().remove(&va) else {
            return Ok(());
        };
        let r = self.bv.unmap_run(va, Some(len), defer);
        if r.is_err() {
            self.rows.borrow_mut().insert(va, len);
            restore_owned_rows(&mut self.rows.borrow_mut(), &self.bv, va, va + len);
        }
        r
    }
    fn unmap_range(&self, va: u64, len: u64, defer: bool) -> Result<(), String> {
        // ★ Not gated by `batching`: an exact range over our own placements is how a changed
        // sub-range of a placement is unmapped (`crate::apply` net diff) — not a batch.
        let end = va + len;
        // `cut_rows` (strict): rows wholly inside go, straddlers keep their outside parts.
        let saved = self.rows.borrow().clone();
        cut_sim_rows(&mut self.rows.borrow_mut(), va, end);
        let res = self.bv.unmap_range(va, len, defer);
        if res.is_err() {
            // ★ Review fix 2026-10-10 (finding 4): the rows follow the LEDGER, never the
            // pre-call state — a span host RM did unmap before the error stays cut.
            *self.rows.borrow_mut() = saved;
            restore_owned_rows(&mut self.rows.borrow_mut(), &self.bv, va, end);
        }
        res
    }
    fn invalidate(&self) -> Result<(), String> {
        Ok(())
    }
    fn own_view(&self, va: u64, end: u64) -> Option<crate::ledger::OwnView> {
        Some(self.bv.own_view(va, end))
    }
}

/// `GpuMirror`'s `cut_rows` over the model's `va → len` rows: rows wholly inside `[va, end)` go,
/// straddlers keep their outside parts.
pub fn cut_sim_rows(r: &mut BTreeMap<u64, u64>, va: u64, end: u64) {
    let keys: Vec<(u64, u64)> = r
        .iter()
        .filter(|&(&k, &l)| k < end && va < k + l)
        .map(|(&k, &l)| (k, l))
        .collect();
    for (k, l) in keys {
        r.remove(&k);
        if k < va {
            r.insert(k, va - k);
        }
        if k + l > end {
            r.insert(end, k + l - end);
        }
    }
}

/// ★ Review fix 2026-10-10 (finding 4) — `GpuMirror::unmap_range`'s refusal path over the model:
/// with the rows put back as they were, cut every part of `[va, end)` the ledger no longer holds
/// (host RM unmapped it before the error), so a reader never resolves through a mapping that is
/// gone and the rows never claim more than the host holds.
pub fn restore_owned_rows<V: SpaceVerbs>(
    r: &mut BTreeMap<u64, u64>,
    bv: &BatchedVas<'_, V>,
    va: u64,
    end: u64,
) {
    let owned = bv.own_view(va, end).owned;
    let mut cur = va;
    for (s, e) in owned.into_iter().chain(std::iter::once((end, end))) {
        if s > cur {
            cut_sim_rows(r, cur, s);
        }
        cur = cur.max(e);
    }
}

/// Deterministic xorshift64*.
pub struct Rng(pub u64);
impl Rng {
    /// Next value.
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// `[0, n)`.
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// A walker-committed placement: the diff model's ledger (`kf_cuda::diffmodel`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Committed {
    /// Bytes.
    pub len: u64,
    /// Acknowledged HELD (not ours on the host).
    pub held: bool,
    /// Guest RAM (else the store).
    pub ram: bool,
    /// Backing offset of the first page.
    pub off: u64,
    /// The MAP run that placed it (its UNMAP carries the same `at`/flags, as the walker's does:
    /// `kf_walk.cu` emits the committed placement itself with `op = UNMAP`).
    pub run: DiffRun,
}

/// The apply configuration of the model (identity guest-RAM layout, 4 KiB grain).
#[must_use]
pub fn cfg(ram_offset: &dyn Fn(u64, u64) -> Option<u64>) -> ApplyCfg<'_> {
    ApplyCfg {
        store_bytes: 1 << 40,
        grain: P,
        ram_offset,
        usermode: None,
        per_map_kind: true,
        carve: 1 << 40,
        carve_refuse: false,
    }
}

/// One MAP run of 4 KiB leaves.
#[must_use]
pub fn map_run(va: u64, len: u64, ram: bool, off: u64, read_only: bool) -> DiffRun {
    DiffRun {
        unmap: false,
        va,
        len,
        at: off,
        ap: if ram { AP_SYS_COHERENT } else { AP_VIDMEM },
        held: false,
        kind: 0,
        perm: MapPerm {
            read_only,
            ..MapPerm::READ_WRITE
        },
        privileged: false,
        leaf: 0,
    }
}

/// One UNMAP run of a committed placement — the placement itself, `op = UNMAP` (as the walker).
#[must_use]
pub fn unmap_run(va: u64, c: &Committed) -> DiffRun {
    DiffRun {
        unmap: true,
        va,
        len: c.len,
        held: c.held,
        ..c.run
    }
}

/// The translation a page has per the walker's committed placements (non-held):
/// `(ram, off, read-only, kind)`.
///
/// ★ Review fix 2026-10-10 (finding 3): ⊘ the LEAF SIZE was part of this key ("a leaf-size
/// change is a change of the guest's mapping"), so the guard could not see sixteen 4 KiB leaves
/// re-expressed as one 64 KiB leaf over the same pages being transiently unmapped. The owner's
/// rule is about the TRANSLATION; the leaf is not part of it.
fn backing_of(c: &BTreeMap<u64, Committed>, p: u64) -> Option<(bool, u64, bool, u8)> {
    c.range(..=p)
        .next_back()
        .filter(|&(&v, x)| p < v + x.len && !x.held)
        .map(|(&v, x)| (x.ram, x.off + (p - v), x.run.perm.read_only, x.run.kind))
}

/// ★ Apply `runs` (one refresh) and commit by the acknowledgements, as the walker does — with the
/// GUARD armed: every page mapped before the refresh whose guest mapping is the SAME after it
/// (same backing, same permissions, same kind — whether or not a run of the diff names it) and
/// that the host maps that way right now is UNCHANGED and must translate as before after EVERY
/// host call of the refresh.
///
/// ★ Review fix 2026-10-10 (finding 8): the commit is the walker's — WHOLE runs, an APPLIED UNMAP
/// removed, an APPLIED/HELD MAP inserted — and a MAP that would land over a placement still
/// committed is recorded as a VIOLATION ("WALKER SLOT OVERLAP"): the real slot would then hold two
/// placements over one VA (`kf_cuda::diffmodel::commit` keeps both), which this `va → placement`
/// map used to overwrite silently. A transient inside an interval the apply declares re-made
/// ([`crate::apply::Applied::remade`]: the one inexact case, an `NV01` mapping the guest split)
/// is moved to [`SimRm::remade_transients`] — counted, never silent.
pub fn apply_and_commit(
    m: &SimMirror<'_>,
    runs: &[DiffRun],
    committed: &mut BTreeMap<u64, Committed>,
) -> crate::apply::Applied {
    // The guest's view after the refresh: the committed set with the UNMAPs removed and the MAPs
    // added (what the walker will commit if everything lands).
    let mut after = committed.clone();
    for r in runs.iter().filter(|r| r.unmap) {
        after.remove(&r.va);
    }
    for r in runs.iter().filter(|r| !r.unmap) {
        after.insert(
            r.va,
            Committed {
                len: r.len,
                held: false,
                ram: r.ap == AP_SYS_COHERENT,
                off: r.at,
                run: *r,
            },
        );
    }
    let guard_on = !m.sim().0.borrow().no_guard;
    if guard_on {
        let rm = m.sim().0.borrow();
        let guard: Vec<(u64, Backing)> = committed
            .iter()
            .filter(|(_, c)| !c.held)
            .flat_map(|(&v, c)| (0..c.len / P).map(move |i| v + i * P))
            .filter(|&p| backing_of(committed, p) == backing_of(&after, p))
            .filter_map(|p| backing_of(committed, p).map(|(r, o, ..)| (p, (r, o))))
            .filter(|&(p, want)| rm.translate(p, Owner::Mirror) == Some(want))
            .collect();
        drop(rm);
        m.sim().0.borrow_mut().guard = guard;
    }
    let id = |gpa: u64, _len: u64| Some(gpa);
    let out = apply_entry(m, runs, &cfg(&id));
    {
        let mut rm = m.sim().0.borrow_mut();
        rm.guard.clear();
        let (mut kept_t, mut kept_va) = (Vec::new(), Vec::new());
        let t = core::mem::take(&mut rm.transient);
        let tv = core::mem::take(&mut rm.transient_va);
        for (msg, va) in t.into_iter().zip(tv) {
            if out.remade.iter().any(|&(s, e)| s <= va && va < e) {
                rm.remade_transients += 1;
            } else {
                kept_t.push(msg);
                kept_va.push(va);
            }
        }
        rm.transient = kept_t;
        rm.transient_va = kept_va;
    }
    let acked = |i: usize| out.codes[i] == KFWR_ACK_APPLIED || out.codes[i] == KFWR_ACK_HELD;
    for (i, r) in runs.iter().enumerate().filter(|(_, r)| r.unmap) {
        if acked(i) {
            committed.remove(&r.va);
        }
    }
    for (i, r) in runs.iter().enumerate().filter(|(_, r)| !r.unmap) {
        if acked(i) {
            let end = r.va + r.len;
            let clash = committed
                .range(..end)
                .next_back()
                .filter(|&(&v, x)| v + x.len > r.va)
                .map(|(&v, x)| (v, x.len));
            if let Some((v, l)) = clash {
                m.sim().0.borrow_mut().violations.push(format!(
                    "WALKER SLOT OVERLAP: map {:#x}+{:#x} acknowledged over the committed placement {v:#x}+{l:#x} (codes {:?})",
                    r.va, r.len, out.codes
                ));
            }
            committed.insert(
                r.va,
                Committed {
                    len: r.len,
                    held: out.codes[i] == KFWR_ACK_HELD,
                    ram: r.ap == AP_SYS_COHERENT,
                    off: r.at,
                    run: *r,
                },
            );
        }
    }
    out
}
/// ★ The invariants, after every step: no foreign mapping touched by a mirror verb; no unchanged
/// VA transiently unmapped (checked after every host call); no `gvaspaceFree` assertion; no mirror
/// mapping whose PTEs are gone; every foreign mapping intact; and the host's mirror-owned
/// translation of every page EQUALS what the walker committed (no stale, no missing).
///
/// # Errors
/// The first broken invariant, by name.
pub fn check(
    sim: &Sim,
    committed: &BTreeMap<u64, Committed>,
    foreign: &BTreeMap<u32, (u64, u64)>,
    lo: u64,
    hi: u64,
) -> Result<(), String> {
    let rm = sim.0.borrow();
    if let Some(v) = rm.violations.first() {
        return Err(format!("VIOLATION {v}"));
    }
    if let Some(v) = rm.transient.first() {
        return Err(format!("TRANSIENT {v}"));
    }
    if rm.asserts != 0 {
        return Err(format!(
            "{} gvaspaceFree(NULL pMemBlock) assertion(s) (gpu_vaspace.c:1639)",
            rm.asserts
        ));
    }
    // ★ D1 (2026-10-10): outside every VA-reserving hDma (the NV01 range) a mapping of ours is ONE
    // 4 KiB page — the unit no partial change can split. Anything bigger there is a mapping host
    // RM could not partly unmap exactly (the apply would have to re-make it).
    if let Some(m) = rm
        .maps
        .iter()
        .find(|m| m.owner == Owner::Mirror && m.hdma == RANGE && m.len != P)
    {
        return Err(format!(
            "NV01 mapping {:#x}+{:#x} is bigger than one 4 KiB page (D1: split-exact placement)",
            m.va, m.len
        ));
    }
    if rm.remade_transients != 0 {
        return Err(format!(
            "{} UNCHANGED page(s) transiently unmapped inside a declared re-make (D1: must be 0)",
            rm.remade_transients
        ));
    }
    let broken = rm.broken_mirror();
    if !broken.is_empty() {
        return Err(format!(
            "mirror mapping(s) {broken:x?} lost their PTEs while still listed (FAULT_PTE)"
        ));
    }
    for (&id, &(va, len)) in foreign {
        if !rm.foreign_intact(id, va, len) {
            return Err(format!("foreign #{id} {va:#x}+{len:#x} not intact"));
        }
    }
    let mut p = lo;
    while p < hi {
        let want = committed
            .range(..=p)
            .next_back()
            .filter(|&(&v, c)| p < v + c.len && !c.held)
            .map(|(&v, c)| (c.ram, c.off + (p - v)));
        let got = rm.translate(p, Owner::Mirror);
        if want != got {
            return Err(format!(
                "page {p:#x}: walker committed {want:x?}, host mirror translates {got:x?}"
            ));
        }
        p += P;
    }
    Ok(())
}

#[cfg(test)]
mod adversarial;
#[cfg(test)]
mod tests;
