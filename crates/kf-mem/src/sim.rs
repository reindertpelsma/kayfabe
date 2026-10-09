//! ★★★ **A simulated host RM for ONE host VA space — the GPU-free proof of the batched path**
//! (owner direction 2026-10-09: "the batching logic must be tested properly WITHOUT a GPU").
//!
//! The model implements, byte for byte, what host RM does with OUR map/unmap verbs in one client,
//! read from `ogkm-595.84` (the host runs 595.91.07):
//!
//! - **Two kinds of `hDma`.** The space's `range` is an `NV01_MEMORY_VIRTUAL`, constructed with
//!   `bReserveVaOnAlloc = NV_FALSE` (`virtual_mem.c:349-354`): EVERY map through it allocates its
//!   own VA heap block, and every unmap frees "the block containing the unmapped part's start"
//!   (`virt_mem_allocator_gm107.c:1635-1638` → `gvaspaceFree`, `gpu_vaspace.c:1631-1640`, whose
//!   `eheapGetBlock` is a containment search, `eheap_old.c:1005-1027`). A guest reservation is an
//!   `NV50_MEMORY_VIRTUAL` (`bReserveVaOnAlloc = NV_TRUE`, `virtual_mem.c:593`): an unmap only
//!   invalidates the PTEs of exactly the unmapped part (`virt_mem_allocator_gm107.c:1578-1633`).
//! - **A FIXED map fails on occupancy** (`VA_ALREADY_MAPPED`; production answers `HeldByHost`).
//! - **`size == 0`** unmaps the ONE mapping that starts exactly at `va`, else `OBJECT_NOT_FOUND`
//!   (`rs_server.c:2436-2452`).
//! - **`size != 0`** removes EVERY mapping of the `hDma` that intersects the range — whoever placed
//!   it — splitting a straddler into its outside remnants, and answers `NV_OK` over gaps
//!   (`rs_server.c:2453-2514`, `2316-2416`; `virtual_mem.c:1697-1804`).
//! - **The consequence the model exists for:** a range unmap that SPLITS a mapping of the
//!   `NV01` range frees the WHOLE original VA block — the remnants RM still lists lose their PTEs
//!   (a GPU access faults: host Xid 31 `FAULT_PTE`), and freeing a remnant later finds no block
//!   (`NV_ASSERT(NULL != pMemBlock) @ gpu_vaspace.c:1639`). Both were MEASURED on the GPU host in
//!   Windows runs 242/243 (batched path on) and in no run with `KF3_NO_BATCHED_MAP=1`.
//! - **Freeing a memory object** unmaps every mapping of it (`rs_client.c` back-references).
//!
//! Tests may also place FOREIGN mappings — what kayfabe's other code (windows, rings, a twin's
//! USERD view, another twin) maps through the same client and `hDma` — and the model records as a
//! VIOLATION any mirror verb that removes or splits one.

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
/// A guest reservation (`NV50_MEMORY_VIRTUAL`, `bReserveVaOnAlloc = true`).
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

/// The host RM state of one space.
#[derive(Debug)]
pub struct SimRm {
    /// The space (its reservations decide each piece's `hDma`, as production's `dma_pieces`).
    pub space: VaSpace,
    /// Every mapping RM lists, any `hDma`.
    pub maps: Vec<SimMap>,
    /// `NV01` VA heap blocks, `start → end`.
    pub blocks: BTreeMap<u64, u64>,
    /// Batch objects: page → backing.
    pub objs: BTreeMap<u32, Vec<Backing>>,
    next_obj: u32,
    /// Mirror verbs that removed or split a foreign mapping.
    pub violations: Vec<String>,
    /// `gvaspaceFree` found no block (`NV_ASSERT @ gpu_vaspace.c:1639`).
    pub asserts: u32,
    /// Every mirror range unmap, `(va, len)`.
    pub ranges: Vec<(u64, u64)>,
    /// Bytes mirror range unmaps covered where nothing at all was mapped.
    pub gap_bytes: u64,
}

/// The model behind a `&Sim` [`SpaceVerbs`].
#[derive(Debug)]
pub struct Sim(pub RefCell<SimRm>);

/// The test space: `[base, base + pages*P)`, with ONE guest reservation `[resv_lo, resv_hi)`.
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
            objs: BTreeMap::new(),
            next_obj: 0x1000,
            violations: Vec::new(),
            asserts: 0,
            ranges: Vec::new(),
            gap_bytes: 0,
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

    fn occupied(&self, hdma: u32, va: u64, len: u64) -> bool {
        if hdma == RANGE {
            self.blocks
                .iter()
                .any(|(&s, &e)| overlaps(s, e - s, va, len))
        } else {
            self.maps
                .iter()
                .any(|m| m.hdma == hdma && overlaps(m.va, m.len, va, len))
        }
    }

    /// One FIXED map of `[va, va+len)` (all pieces or none). `Err(true)` = occupied.
    fn map(&mut self, va: u64, len: u64, obj: u32, off: u64, owner: Owner) -> Result<(), bool> {
        let pieces = self.pieces(va, len).map_err(|_| false)?;
        if pieces.iter().any(|&(h, pv, pl)| self.occupied(h, pv, pl)) {
            return Err(true);
        }
        for (h, pv, pl) in pieces {
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
        }
        Ok(())
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
        Ok(())
    }

    /// `size != 0` in `hdma`: every intersecting mapping, split at the edges.
    fn unmap_range_in(&mut self, hdma: u32, a: u64, b: u64, by_mirror: bool) {
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
    }

    /// Free `obj`: every mapping of it goes (whole-mapping unmaps).
    fn free(&mut self, obj: u32) -> Result<(), String> {
        if self.objs.remove(&obj).is_none() {
            return Err(format!("free {obj:#x}: no such object"));
        }
        while let Some(m) = self.maps.iter().find(|m| m.obj == obj).cloned() {
            let _ = self.unmap_whole_in(m.hdma, m.va, true);
        }
        Ok(())
    }

    /// What VA page `va` translates to through a live (not broken) mapping of `owner`.
    #[must_use]
    pub fn translate(&self, va: u64, owner: Owner) -> Option<Backing> {
        let m = self.maps.iter().find(|m| {
            !m.broken && m.owner == owner && m.va <= va && va < m.va + m.len
        })?;
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
        self.map(va, len, FOREIGN_OBJ, va, Owner::Foreign(id)).is_ok()
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
}

impl SpaceVerbs for &Sim {
    fn map_row(&self, d: &Desired, _defer: bool) -> Result<Mapped, String> {
        let obj = if d.ram { RAM_OBJ } else { STORE_OBJ };
        match self.0.borrow_mut().map(d.va, d.len, obj, d.off, Owner::Mirror) {
            Ok(()) => Ok(Mapped::Placed),
            Err(true) => Ok(Mapped::HeldByHost),
            Err(false) => Err(format!("map {:#x}+{:#x}: refused", d.va, d.len)),
        }
    }
    fn map_sked_row(&self, s: &SkedRow, _defer: bool) -> Result<Mapped, String> {
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
        let first = rows.first().ok_or("empty batch")?;
        let mut pages: Vec<Backing> = Vec::new();
        let mut next = first.va;
        for d in rows {
            if !d.ram || d.va != next || d.perm != first.perm || d.kind != first.kind {
                return Err("batch rows not one VA-contiguous same-kind same-perm RAM range".into());
            }
            next = d.va + d.len;
            pages.extend((0..d.len / P).map(|i| (true, d.off + i * P)));
        }
        let mut rm = self.0.borrow_mut();
        let h = rm.next_obj;
        rm.next_obj += 1;
        rm.objs.insert(h, pages);
        match rm.map(first.va, next - first.va, h, 0, Owner::Mirror) {
            Ok(()) => Ok(h),
            Err(_) => {
                let _ = rm.free(h);
                Err(format!("batch {:#x}: map refused", first.va))
            }
        }
    }
    fn unmap_whole(&self, va: u64, _defer: bool) -> Result<(), String> {
        let mut rm = self.0.borrow_mut();
        let h = rm.space.dma_for(va, 1).map_err(|e| format!("{e:?}"))?;
        rm.unmap_whole_in(h, va, true)
    }
    fn unmap_row(&self, va: u64, len: u64, _defer: bool) -> Result<(), String> {
        let mut rm = self.0.borrow_mut();
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
        Ok(())
    }
    fn free(&self, handle: u32) -> Result<(), String> {
        self.0.borrow_mut().free(handle)
    }
    fn splits_safely(&self, va: u64, len: u64) -> bool {
        self.0.borrow().space.guest_reserved(va, len)
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
    /// A fresh mirror over `sim`.
    #[must_use]
    pub fn new(sim: &'a Sim, batching: bool) -> Self {
        SimMirror {
            bv: BatchedVas::new(sim),
            rows: RefCell::new(BTreeMap::new()),
            batching,
        }
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
        let m = self.bv.map(d, defer)?;
        if m == Mapped::Placed {
            self.rows.borrow_mut().insert(d.va, d.len);
        }
        Ok(m)
    }
    fn map_batch(&self, rows: &[Desired], defer: bool) -> Result<(), String> {
        if !self.batching {
            return Err(NOT_BATCHED.into());
        }
        self.bv
            .place(std::os::fd::AsFd::as_fd(&std::io::stdin()), rows, defer)?;
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
        }
        r
    }
    fn unmap_range(&self, va: u64, len: u64, defer: bool) -> Result<(), String> {
        if !self.batching {
            return Err(NOT_BATCHED.into());
        }
        let end = va + len;
        // `cut_rows` (strict): rows wholly inside go, straddlers keep their outside parts.
        let saved = self.rows.borrow().clone();
        {
            let mut r = self.rows.borrow_mut();
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
        let res = self.bv.unmap_range(va, len, defer);
        if res.is_err() {
            *self.rows.borrow_mut() = saved;
        }
        res
    }
    fn invalidate(&self) -> Result<(), String> {
        Ok(())
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

/// One MAP run.
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
    }
}

/// One UNMAP run of a committed placement.
#[must_use]
pub fn unmap_run(va: u64, c: &Committed) -> DiffRun {
    DiffRun {
        unmap: true,
        va,
        len: c.len,
        at: 0,
        ap: 0,
        held: c.held,
        kind: 0,
        perm: MapPerm::READ_WRITE,
        privileged: false,
    }
}

/// Apply `runs` and commit by the acknowledgements, as the walker does.
pub fn apply_and_commit(
    target: &dyn MapTarget,
    runs: &[DiffRun],
    committed: &mut BTreeMap<u64, Committed>,
) -> crate::apply::Applied {
    let id = |gpa: u64, _len: u64| Some(gpa);
    let out = apply_entry(target, runs, &cfg(&id));
    for (r, &code) in runs.iter().zip(&out.codes) {
        if r.unmap {
            if code == KFWR_ACK_APPLIED || code == KFWR_ACK_HELD {
                committed.remove(&r.va);
            }
        } else if code == KFWR_ACK_APPLIED || code == KFWR_ACK_HELD {
            committed.insert(
                r.va,
                Committed {
                    len: r.len,
                    held: code == KFWR_ACK_HELD,
                    ram: r.ap == AP_SYS_COHERENT,
                    off: r.at,
                },
            );
        }
    }
    out
}

/// ★ The invariants, after every step: no foreign mapping touched by a mirror verb; no `gvaspaceFree`
/// assertion; no mirror mapping whose PTEs are gone; every foreign mapping intact; and the host's
/// mirror-owned translation of every page EQUALS what the walker committed (no stale, no missing).
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
    if rm.asserts != 0 {
        return Err(format!(
            "{} gvaspaceFree(NULL pMemBlock) assertion(s) (gpu_vaspace.c:1639)",
            rm.asserts
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
mod tests {
    use super::*;

    /// The model space: 96 pages; pages 32..64 are a guest reservation (`NV50`), the rest go
    /// through the `NV01` range — the Windows shape (`[1 MiB, 4.5 GiB)` is unreserved) and the
    /// Linux/CUDA shape (above 4.5 GiB, reserved) in one space, with two straddle edges.
    const BASE: u64 = 0x100_0000;
    const PAGES: u64 = 96;
    const RESV_LO: u64 = BASE + 32 * P;
    const RESV_HI: u64 = BASE + 64 * P;

    fn pg(i: u64) -> u64 {
        BASE + i * P
    }

    fn fresh() -> Sim {
        Sim::new(space(RESV_LO, RESV_HI))
    }

    /// Map `rows` = `(page, pages, backing page)` of guest RAM through apply; all must be APPLIED.
    fn map_ram(m: &SimMirror<'_>, c: &mut BTreeMap<u64, Committed>, rows: &[(u64, u64, u64)]) {
        let runs: Vec<DiffRun> = rows
            .iter()
            .map(|&(p, n, b)| map_run(pg(p), n * P, true, b * P, false))
            .collect();
        let out = apply_and_commit(m, &runs, c);
        assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
    }

    fn unmap_pages(m: &SimMirror<'_>, c: &mut BTreeMap<u64, Committed>, pages: &[u64]) {
        let runs: Vec<DiffRun> = pages
            .iter()
            .map(|&p| unmap_run(pg(p), &c[&pg(p)]))
            .collect();
        let out = apply_and_commit(m, &runs, c);
        assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
    }

    /// ★★★ THE ROOT-CAUSE REGRESSION (Windows runs 242/243, host Xid 31 `FAULT_PTE` at
    /// `0x4034000`, `gpu_vaspace.c:1639` at exit): four scattered guest-RAM rows in the UNRESERVED
    /// part of the space (through the `NV01` range) are placed, and ONE middle row is unmapped. The
    /// other three must stay mapped. Before the fix the four went as one batch mapping, the single
    /// unmap became a range that SPLIT it, and host RM freed the whole VA block.
    #[test]
    fn unmapping_one_row_of_several_outside_a_reservation_keeps_the_others_mapped() {
        let sim = fresh();
        let m = SimMirror::new(&sim, true);
        let mut c = BTreeMap::new();
        map_ram(&m, &mut c, &[(4, 1, 900), (5, 1, 17), (6, 1, 333), (7, 1, 5)]);
        check(&sim, &c, &BTreeMap::new(), BASE, pg(PAGES)).unwrap();
        unmap_pages(&m, &mut c, &[5]);
        check(&sim, &c, &BTreeMap::new(), BASE, pg(PAGES)).unwrap();
        // ... and the teardown frees cleanly (no assertion, no leak).
        unmap_pages(&m, &mut c, &[4, 6, 7]);
        check(&sim, &c, &BTreeMap::new(), BASE, pg(PAGES)).unwrap();
        assert!(sim.0.borrow().maps.is_empty());
    }

    /// Inside a reservation (`NV50`, the Linux/CUDA shape) a partial unmap is exact: the batch is
    /// kept (ONE host map for the four rows) and its piece unmaps alone.
    #[test]
    fn inside_a_reservation_rows_still_batch_and_a_piece_unmaps_alone() {
        let sim = fresh();
        let m = SimMirror::new(&sim, true);
        let mut c = BTreeMap::new();
        map_ram(&m, &mut c, &[(40, 1, 900), (41, 1, 17), (42, 1, 333), (43, 1, 5)]);
        assert_eq!(
            sim.0.borrow().maps.len(),
            1,
            "one batched host mapping for four rows"
        );
        unmap_pages(&m, &mut c, &[41]);
        check(&sim, &c, &BTreeMap::new(), BASE, pg(PAGES)).unwrap();
        unmap_pages(&m, &mut c, &[40, 42, 43]);
        check(&sim, &c, &BTreeMap::new(), BASE, pg(PAGES)).unwrap();
        assert!(sim.0.borrow().maps.is_empty() && sim.0.borrow().objs.is_empty());
    }

    /// A range over two own placements with a FOREIGN mapping in the gap between them must take
    /// the two placements and never the foreign one.
    #[test]
    fn a_range_never_reaches_a_foreign_mapping_between_own_placements() {
        for (lo, label) in [(4u64, "NV01 range"), (40, "NV50 reservation")] {
            let sim = fresh();
            let m = SimMirror::new(&sim, true);
            let mut c = BTreeMap::new();
            map_ram(&m, &mut c, &[(lo, 1, 10), (lo + 1, 1, 11)]);
            assert!(sim.0.borrow_mut().place_foreign(1, pg(lo + 2), P));
            map_ram(&m, &mut c, &[(lo + 3, 1, 12), (lo + 4, 1, 13)]);
            let foreign = BTreeMap::from([(1u32, (pg(lo + 2), P))]);
            let r = m.bv.unmap_range(pg(lo), 5 * P, true);
            for p in [lo, lo + 1, lo + 3, lo + 4] {
                c.remove(&pg(p));
            }
            m.rows.borrow_mut().clear();
            assert!(r.is_ok(), "{label}: {r:?}");
            check(&sim, &c, &foreign, BASE, pg(PAGES))
                .unwrap_or_else(|e| panic!("{label}: {e}"));
            assert_eq!(sim.0.borrow().gap_bytes, 0, "{label}: no byte of a gap is unmapped");
        }
    }

    /// A range that ends exactly where a foreign mapping starts leaves it alone.
    #[test]
    fn a_range_ending_at_a_foreign_mapping_leaves_it() {
        let sim = fresh();
        let m = SimMirror::new(&sim, true);
        let mut c = BTreeMap::new();
        assert!(sim.0.borrow_mut().place_foreign(7, pg(42), 2 * P));
        map_ram(&m, &mut c, &[(40, 1, 1), (41, 1, 9)]);
        unmap_pages(&m, &mut c, &[40, 41]);
        check(&sim, &c, &BTreeMap::from([(7, (pg(42), 2 * P))]), BASE, pg(PAGES)).unwrap();
    }

    /// Sparse unmaps (every other page) around foreign pages: one verdict each, foreign intact.
    #[test]
    fn sparse_unmaps_around_foreign_pages() {
        for lo in [2u64, 34] {
            let sim = fresh();
            let m = SimMirror::new(&sim, true);
            let mut c = BTreeMap::new();
            let mut foreign = BTreeMap::new();
            map_ram(&m, &mut c, &[(lo, 1, 3), (lo + 2, 1, 4), (lo + 4, 1, 5)]);
            for (id, p) in [(1u32, lo + 1), (2, lo + 3)] {
                assert!(sim.0.borrow_mut().place_foreign(id, pg(p), P));
                foreign.insert(id, (pg(p), P));
            }
            unmap_pages(&m, &mut c, &[lo, lo + 2, lo + 4]);
            check(&sim, &c, &foreign, BASE, pg(PAGES)).unwrap();
        }
    }

    /// Whole-space retire with foreign windows present: every mirror mapping goes, every batch
    /// object is freed, the foreign windows stay.
    #[test]
    fn retire_with_foreign_windows_takes_only_ours() {
        let sim = fresh();
        let m = SimMirror::new(&sim, true);
        let mut c = BTreeMap::new();
        let foreign = BTreeMap::from([(1u32, (pg(0), 2 * P)), (2, (pg(50), 3 * P)), (3, (pg(90), P))]);
        for (&id, &(va, len)) in &foreign {
            assert!(sim.0.borrow_mut().place_foreign(id, va, len));
        }
        map_ram(&m, &mut c, &[(2, 1, 7), (3, 1, 70), (4, 2, 700), (30, 1, 1), (31, 1, 2), (32, 1, 3), (33, 1, 4)]);
        map_ram(&m, &mut c, &[(45, 1, 8), (46, 1, 80), (47, 1, 800), (53, 1, 9), (54, 1, 90), (63, 1, 5), (64, 1, 6)]);
        check(&sim, &c, &foreign, BASE, pg(PAGES)).unwrap();
        assert_eq!(m.retire(), 0);
        c.clear();
        check(&sim, &c, &foreign, BASE, pg(PAGES)).unwrap();
        let rm = sim.0.borrow();
        assert!(rm.maps.iter().all(|x| x.owner != Owner::Mirror) && rm.objs.is_empty());
    }

    /// ★★★ **The property test**: thousands of random walker diffs — maps (scattered, contiguous,
    /// vidmem, read-only), remaps, partial unmaps, straddles of the reservation edges, whole-space
    /// retires — through the REAL apply + [`BatchedVas`] code against the model, with foreign
    /// mappings sprinkled into gaps. Every invariant of [`check`] after EVERY step.
    #[test]
    fn property_batched_mirror_against_the_host_rm_model() {
        let mut failures = Vec::new();
        for seed in 1..=300u64 {
            if let Err(e) = run_seed(seed, 60, true) {
                failures.push(format!("seed {seed}: {e}"));
            }
        }
        let kinds = ["VIOLATION", "FAULT_PTE", "assertion", "translates", "retire", "survived"];
        let histogram: Vec<(&str, usize)> = kinds
            .iter()
            .map(|k| (*k, failures.iter().filter(|f| f.contains(k)).count()))
            .collect();
        assert!(
            failures.is_empty(),
            "{} of 300 seeds failed {histogram:?}; first: {}",
            failures.len(),
            failures[0]
        );
    }

    /// The same property on the per-run path (`KF3_NO_BATCHED_MAP=1`, the A/B opt-out): no batch,
    /// no range — the reference the batched path must equal.
    #[test]
    fn property_per_run_path_against_the_host_rm_model() {
        for seed in 1..=100u64 {
            run_seed(seed, 60, false).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        }
    }

    fn run_seed(seed: u64, steps: usize, batching: bool) -> Result<(), String> {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let sim = fresh();
        let mut m = SimMirror::new(&sim, batching);
        let mut c: BTreeMap<u64, Committed> = BTreeMap::new();
        let mut foreign: BTreeMap<u32, (u64, u64)> = BTreeMap::new();
        let mut next_id = 1u32;
        let busy = |c: &BTreeMap<u64, Committed>, f: &BTreeMap<u32, (u64, u64)>, p: u64| {
            let va = pg(p);
            c.range(..=va).next_back().is_some_and(|(&v, x)| va < v + x.len)
                || f.values().any(|&(v, l)| v <= va && va < v + l)
        };
        for step in 0..steps {
            let op = rng.below(100);
            let what;
            if op < 45 {
                // MAP a window of uncommitted pages (foreign pages may be inside: HeldByHost).
                let start = rng.below(PAGES);
                let want = 1 + rng.below(12);
                let mut n = 0;
                while n < want
                    && start + n < PAGES
                    && !c
                        .range(..=pg(start + n))
                        .next_back()
                        .is_some_and(|(&v, x)| pg(start + n) < v + x.len)
                {
                    n += 1;
                }
                let mut runs = Vec::new();
                let mut p = start;
                let ro = rng.below(100) < 15;
                while p < start + n {
                    let k = (1 + rng.below(4)).min(start + n - p);
                    let ram = rng.below(100) < 85;
                    let back = rng.below(4096);
                    runs.push(map_run(pg(p), k * P, ram, back * P, ro && rng.below(2) == 0));
                    p += k;
                }
                what = format!("map {} run(s) from page {start}", runs.len());
                apply_and_commit(&m, &runs, &mut c);
            } else if op < 70 {
                // UNMAP (sparse or contiguous) the placements wholly inside a window.
                let start = rng.below(PAGES);
                let n = 1 + rng.below(16);
                let sparse = rng.below(3) == 0;
                let runs: Vec<DiffRun> = c
                    .range(pg(start)..pg(start + n))
                    .filter(|&(&v, x)| v + x.len <= pg(start + n))
                    .enumerate()
                    .filter(|(i, _)| !sparse || i % 2 == 0)
                    .map(|(_, (&v, x))| unmap_run(v, x))
                    .collect();
                what = format!("unmap {} run(s) in pages {start}+{n}", runs.len());
                apply_and_commit(&m, &runs, &mut c);
            } else if op < 85 {
                // REMAP: unmap the placements inside a window and map new backing over them.
                let start = rng.below(PAGES);
                let n = 1 + rng.below(10);
                let old: Vec<(u64, Committed)> = c
                    .range(pg(start)..pg(start + n))
                    .filter(|&(&v, x)| v + x.len <= pg(start + n))
                    .map(|(&v, &x)| (v, x))
                    .collect();
                let mut runs: Vec<DiffRun> = old.iter().map(|(v, x)| unmap_run(*v, x)).collect();
                for (v, x) in &old {
                    let mut q = *v;
                    while q < v + x.len {
                        let k = ((1 + rng.below(3)) * P).min(v + x.len - q);
                        runs.push(map_run(q, k, rng.below(100) < 85, rng.below(4096) * P, false));
                        q += k;
                    }
                }
                what = format!("remap {} placement(s) in pages {start}+{n}", old.len());
                apply_and_commit(&m, &runs, &mut c);
            } else if op < 93 {
                // A FOREIGN mapping lands in a gap.
                let p = rng.below(PAGES);
                let n = 1 + rng.below(3);
                if (p..p + n).all(|q| q < PAGES && !busy(&c, &foreign, q))
                    && sim.0.borrow_mut().place_foreign(next_id, pg(p), n * P)
                {
                    foreign.insert(next_id, (pg(p), n * P));
                    next_id += 1;
                }
                what = format!("foreign at page {p}+{n}");
            } else if op < 97 {
                // Its owner removes a foreign mapping.
                let key = foreign.keys().nth(rng.below(foreign.len() as u64 + 1) as usize).copied();
                if let Some(id) = key {
                    sim.0.borrow_mut().remove_foreign(id);
                    foreign.remove(&id);
                }
                what = "foreign removed".into();
            } else {
                // The guest frees the VA space: retire, then a fresh mirror on the SAME host space.
                let refused = m.retire();
                if refused != 0 {
                    return Err(format!("step {step}: retire refused {refused} row(s)"));
                }
                c.clear();
                check(&sim, &c, &foreign, BASE, pg(PAGES))
                    .map_err(|e| format!("step {step} (retire): {e}"))?;
                if sim.0.borrow().maps.iter().any(|x| x.owner == Owner::Mirror) {
                    return Err(format!("step {step}: a mirror mapping survived the retire"));
                }
                m = SimMirror::new(&sim, batching);
                what = "retire".into();
            }
            check(&sim, &c, &foreign, BASE, pg(PAGES))
                .map_err(|e| format!("step {step} ({what}): {e}"))?;
        }
        Ok(())
    }
}
