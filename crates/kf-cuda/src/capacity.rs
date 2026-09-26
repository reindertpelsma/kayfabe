//! ★★★★★ **The walker's capacity, host-managed** (w829 — the no-PM UVM wall).
//!
//! ⊘ **What it replaces.** Every walk entry's table slice and every slot's committed placements
//! used to be ONE uniform `runs_per_pdb` = 16 384 runs. `[measured uw4, vh, GA106]` a CUDA
//! process maps a VA-contiguous ~56 MiB of sysmem whose guest pages are SCATTERED: of 16 378
//! VA-contiguous 4 KiB neighbours, **0** were GPA-contiguous, so every page is its own run. How
//! many runs that is depends on guest-RAM fragmentation, which grows with every process — and
//! faster when each process re-initialises the adapter (no persistence mode). The space crossed
//! 16 384, its walk truncated (`KFWR_R_RUN_CAP`), the split was refused and UVM's copy channel
//! died: on the 5th/6th CUDA process of a fresh boot, on the 1st of an already fragmented one.
//! The table itself was clean (7 objects, 121 free slots) — not a leak: a capacity model.
//!
//! ★ **What it is now.** Two POOLS, carved by this module:
//! - the **walk pool** — each walk entry gets a region sized to what its space needed last time
//!   (the kernel reports the need, uncapped); the diff's scratch is carved from the same offsets
//!   (`4×` runs, `3×` words), so the scratch is bounded by the pool, not by a per-entry constant;
//! - the **slot pool** — each slot (VA-space object) owns a region of committed placements, born
//!   small and GROWN on demand (a device-to-device copy on the walk stream, ordered before the
//!   next walk), freed when the object goes.
//!
//! ⊘ Both pools have a HARD CEILING, named: a space that needs more than the pool can give is
//! refused by name (the walk stays refused, as before) — never a hang, never a write past a
//! region. The ceiling is the pool size, which is accounted in the VM's GPU budget line.
//!
//! ⊘ Pure: no CUDA here. [`crate::walk::WalkKernel`] applies what this module decides.

use crate::abi::{KF_MAX_PDB, KF_MAX_SLOTS, KfLayout};

/// One region of a pool, in runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    /// First run.
    pub off: u32,
    /// Runs.
    pub cap: u32,
}

/// A first-fit allocator over `[0, size)` runs, coalescing on free.
#[derive(Debug, Clone)]
struct Pool {
    size: u32,
    /// Free regions, sorted by offset, never adjacent (coalesced).
    free: Vec<Region>,
}

impl Pool {
    fn new(size: u32) -> Pool {
        Pool { size, free: if size > 0 { vec![Region { off: 0, cap: size }] } else { Vec::new() } }
    }

    fn alloc(&mut self, cap: u32) -> Option<Region> {
        let i = self.free.iter().position(|r| r.cap >= cap)?;
        let r = self.free[i];
        if r.cap == cap {
            self.free.remove(i);
        } else {
            self.free[i] = Region { off: r.off + cap, cap: r.cap - cap };
        }
        Some(Region { off: r.off, cap })
    }

    fn release(&mut self, r: Region) {
        if r.cap == 0 {
            return;
        }
        let i = self.free.partition_point(|f| f.off < r.off);
        self.free.insert(i, r);
        // Coalesce with the right, then the left neighbour.
        if i + 1 < self.free.len() && self.free[i].off + self.free[i].cap == self.free[i + 1].off {
            self.free[i].cap += self.free[i + 1].cap;
            self.free.remove(i + 1);
        }
        if i > 0 && self.free[i - 1].off + self.free[i - 1].cap == self.free[i].off {
            self.free[i - 1].cap += self.free[i].cap;
            self.free.remove(i);
        }
    }

    fn free_runs(&self) -> u64 {
        self.free.iter().map(|r| u64::from(r.cap)).sum()
    }

    fn largest_free(&self) -> u32 {
        self.free.iter().map(|r| r.cap).max().unwrap_or(0)
    }
}

/// Round `n` up to the capacity grain (256 runs = 8 KiB), at least `min`.
fn grain(n: u64, min: u32) -> u64 {
    n.max(u64::from(min)).div_ceil(256) * 256
}

/// What a slot's growth asks the device to do: copy its committed placements from `from` to `to`
/// before the next walk (the counts stay; only the region moves).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Move {
    /// The slot.
    pub slot: u32,
    /// Its old region (freed).
    pub from: Region,
    /// Its new region.
    pub to: Region,
}

/// Counters, for the census line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CapacityStats {
    /// Slots grown.
    pub grows: u64,
    /// Walks re-submitted because a capacity refusal was fixable.
    pub retries: u64,
    /// Capacity refusals that were NOT fixable (the pool ceiling), refused by name.
    pub ceilings: u64,
    /// The largest slot capacity ever held.
    pub largest_slot: u32,
    /// The largest walk region ever planned.
    pub largest_walk: u32,
}

/// ★ The capacity state of one walker.
#[derive(Debug, Clone)]
pub struct Capacity {
    walk_pool: u32,
    slots: Pool,
    /// The largest region one space may hold (walk or slot).
    max_cap: u32,
    /// A new slot's first region.
    slot_default: u32,
    /// A walk entry's smallest region.
    walk_min: u32,
    region: Vec<Option<Region>>,
    /// Per slot: what its last walk needed (the next walk's region is at least this).
    hint: Vec<u32>,
    /// The layout of the last planned walk (the next commit's scratch).
    prev_off: [u32; KF_MAX_PDB],
    prev_cap: [u32; KF_MAX_PDB],
    /// Counters.
    pub stats: CapacityStats,
}

impl Capacity {
    /// A walker with `max_slots` slots, a `walk_pool`-run walk pool and a `slot_pool`-run slot
    /// pool; no space may hold more than `max_cap` runs.
    ///
    /// # Errors
    /// A configuration the layout cannot express, by name.
    pub fn new(max_slots: u32, walk_pool: u32, slot_pool: u32, max_cap: u32, slot_default: u32) -> Result<Capacity, String> {
        if max_slots == 0 || max_slots as usize > KF_MAX_SLOTS {
            return Err(format!("max_slots {max_slots} outside 1..={KF_MAX_SLOTS}"));
        }
        if slot_default == 0 || slot_default > max_cap || max_cap > walk_pool || max_cap > slot_pool {
            return Err(format!(
                "slot_default {slot_default} / max_cap {max_cap} must fit both pools (walk {walk_pool}, slot {slot_pool})"
            ));
        }
        Ok(Capacity {
            walk_pool,
            slots: Pool::new(slot_pool),
            max_cap,
            slot_default,
            walk_min: slot_default.min(2048),
            region: vec![None; max_slots as usize],
            hint: vec![0; max_slots as usize],
            prev_off: [0; KF_MAX_PDB],
            prev_cap: [0; KF_MAX_PDB],
            stats: CapacityStats::default(),
        })
    }

    /// The region slot `s` holds, if any.
    #[must_use]
    pub fn slot_region(&self, s: u32) -> Option<Region> {
        self.region.get(s as usize).copied().flatten()
    }

    /// The largest region one space may hold.
    #[must_use]
    pub fn max_cap(&self) -> u32 {
        self.max_cap
    }

    /// The previous walk's region for entry `t` (diagnostics read the table there).
    #[must_use]
    pub fn prev_walk(&self, t: usize) -> Option<Region> {
        (t < KF_MAX_PDB && self.prev_cap[t] > 0).then(|| Region { off: self.prev_off[t], cap: self.prev_cap[t] })
    }

    /// ★ Slot `s`'s object is gone: its region returns to the pool and its hint is forgotten.
    pub fn release(&mut self, s: u32) {
        if let Some(r) = self.region.get_mut(s as usize).and_then(Option::take) {
            self.slots.release(r);
        }
        if let Some(h) = self.hint.get_mut(s as usize) {
            *h = 0;
        }
    }

    /// ★ Plan one walk of `slots` (one per entry, distinct): every slot gets a region (a new one
    /// at `slot_default`), and every entry a walk region at least its slot's capacity and its
    /// last need. Returns the layout the kernels read.
    ///
    /// # Errors
    /// A pool ceiling, by name (nothing is changed by a refused plan except slots it gave a
    /// first region to, which they keep).
    pub fn plan(&mut self, slots: &[u32]) -> Result<KfLayout, String> {
        if slots.len() > KF_MAX_PDB {
            return Err(format!("{} entries; the layout carries {KF_MAX_PDB}", slots.len()));
        }
        for &s in slots {
            let i = s as usize;
            if i >= self.region.len() {
                return Err(format!("slot {s} out of range ({})", self.region.len()));
            }
            if self.region[i].is_none() {
                let r = self.slots.alloc(self.slot_default).ok_or_else(|| {
                    self.stats.ceilings += 1;
                    format!(
                        "slot pool exhausted: slot {s} needs its first {} runs, {} free (largest {}) — \
                         the walker's slot pool is {} runs",
                        self.slot_default,
                        self.slots.free_runs(),
                        self.slots.largest_free(),
                        self.slots.size
                    )
                })?;
                self.region[i] = Some(r);
            }
        }
        let mut lay = KfLayout::default();
        let mut off: u64 = 0;
        for (t, &s) in slots.iter().enumerate() {
            let i = s as usize;
            let scap = self.region[i].map_or(0, |r| r.cap);
            let cap = grain(u64::from(scap.max(self.hint[i])), self.walk_min).min(u64::from(self.max_cap));
            if off + cap > u64::from(self.walk_pool) {
                self.stats.ceilings += 1;
                return Err(format!(
                    "walk pool exhausted: {} entries need more than its {} runs (entry {t}, slot {s}, {cap} runs)",
                    slots.len(),
                    self.walk_pool
                ));
            }
            lay.walk_off[t] = off as u32;
            lay.walk_cap[t] = cap as u32;
            self.stats.largest_walk = self.stats.largest_walk.max(cap as u32);
            off += cap;
        }
        lay.prev_off = self.prev_off;
        lay.prev_cap = self.prev_cap;
        for (i, r) in self.region.iter().enumerate() {
            if let Some(r) = r {
                lay.slot_off[i] = r.off;
                lay.slot_cap[i] = r.cap;
            }
        }
        self.prev_off = lay.walk_off;
        self.prev_cap = lay.walk_cap;
        Ok(lay)
    }

    /// ★ Entry-level verdict on a collected report: slot `s`'s WALK needed `need` runs (it was
    /// cut at its region). Records the need; `Ok(true)` when a re-walk can hold it.
    ///
    /// # Errors
    /// The ceiling, by name.
    pub fn walk_needed(&mut self, s: u32, need: u32) -> Result<bool, String> {
        let want = grain(u64::from(need) + u64::from(need) / 4, self.walk_min);
        if u64::from(need) > u64::from(self.max_cap) {
            self.stats.ceilings += 1;
            return Err(format!(
                "slot {s}: its space maps {need} runs; one space may hold {} (the walk pool's ceiling)",
                self.max_cap
            ));
        }
        let h = &mut self.hint[s as usize];
        *h = (*h).max(want.min(u64::from(self.max_cap)) as u32);
        Ok(true)
    }

    /// ★ Entry-level verdict: slot `s`'s diff needed `need` committed placements (placements +
    /// maps) and its region holds fewer. Grows the region (at 1.5×, capped) and returns the move
    /// the device must make before the next walk; `Ok(None)` when it already fits.
    ///
    /// # Errors
    /// The ceiling (one space, or the pool), by name.
    pub fn slot_needed(&mut self, s: u32, need: u32) -> Result<Option<Move>, String> {
        let i = s as usize;
        let Some(old) = self.region.get(i).copied().flatten() else {
            return Err(format!("slot {s} has no region"));
        };
        if need <= old.cap {
            return Ok(None);
        }
        if need > self.max_cap {
            self.stats.ceilings += 1;
            return Err(format!("slot {s}: {need} committed placements; one space may hold {}", self.max_cap));
        }
        let cap = grain(u64::from(need) + u64::from(need) / 2, self.slot_default).min(u64::from(self.max_cap)) as u32;
        // ⊘ Allocate the new region BEFORE releasing the old one: the copy reads the old region,
        // so the two must not overlap.
        let Some(to) = self.slots.alloc(cap) else {
            self.stats.ceilings += 1;
            return Err(format!(
                "slot pool exhausted: slot {s} grows {} → {cap} runs, {} free (largest {}) of {}",
                old.cap,
                self.slots.free_runs(),
                self.slots.largest_free(),
                self.slots.size
            ));
        };
        self.slots.release(old);
        self.region[i] = Some(to);
        let h = &mut self.hint[i];
        *h = (*h).max(cap);
        self.stats.grows += 1;
        self.stats.largest_slot = self.stats.largest_slot.max(cap);
        Ok(Some(Move { slot: s, from: old, to }))
    }

    /// Free runs in the slot pool.
    #[must_use]
    pub fn slot_pool_free(&self) -> u64 {
        self.slots.free_runs()
    }

    /// One line: pools, use, growth.
    #[must_use]
    pub fn census(&self) -> String {
        format!(
            "walk pool {} runs, slot pool {} runs ({} free), max per space {}, slots held {}, grows {}, retries {}, ceilings {}, largest slot {}, largest walk {}",
            self.walk_pool,
            self.slots.size,
            self.slots.free_runs(),
            self.max_cap,
            self.region.iter().filter(|r| r.is_some()).count(),
            self.stats.grows,
            self.stats.retries,
            self.stats.ceilings,
            self.stats.largest_slot,
            self.stats.largest_walk
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap() -> Capacity {
        Capacity::new(8, 1 << 16, 1 << 16, 1 << 15, 1024).expect("config")
    }

    #[test]
    fn a_new_slot_gets_the_default_and_the_walk_region_covers_it() {
        let mut c = cap();
        let lay = c.plan(&[3, 5]).expect("plan");
        assert_eq!(c.slot_region(3), Some(Region { off: 0, cap: 1024 }));
        assert_eq!(c.slot_region(5), Some(Region { off: 1024, cap: 1024 }));
        for t in 0..2 {
            assert!(lay.walk_cap[t] >= 1024, "walk_cap {} < slot cap", lay.walk_cap[t]);
        }
        assert_eq!(lay.walk_off[1], lay.walk_off[0] + lay.walk_cap[0], "entries are packed, disjoint");
        assert_eq!(lay.slot_cap[3], 1024);
        assert_eq!(lay.slot_off[5], 1024);
    }

    #[test]
    fn the_previous_layout_is_carried_for_the_commit() {
        let mut c = cap();
        let a = c.plan(&[1]).expect("plan");
        let _ = c.walk_needed(1, 20_000).expect("fits");
        let b = c.plan(&[2, 1]).expect("plan");
        assert_eq!(b.prev_off, a.walk_off);
        assert_eq!(b.prev_cap, a.walk_cap);
        assert!(b.walk_cap[1] >= 25_000, "the need (+25%) sizes the next walk: {}", b.walk_cap[1]);
    }

    /// ★ The w829 case in miniature: a space whose walk outgrows its region is re-walked into a
    /// region that holds it, and its slot grows to hold the diff — the move keeps the counts.
    #[test]
    fn a_space_past_the_old_uniform_cap_is_held_after_one_growth() {
        let mut c = Capacity::new(8, 1 << 20, 2 << 20, 1 << 20, 1024).expect("config");
        let _ = c.plan(&[0]).expect("plan");
        assert!(c.walk_needed(0, 30_000).expect("fits"));
        let m = c.slot_needed(0, 30_000).expect("grows").expect("a move");
        assert_eq!(m.from, Region { off: 0, cap: 1024 });
        assert!(m.to.cap >= 30_000);
        assert!(m.to.off >= 1024 || m.to.off + m.to.cap <= m.from.off, "the move never overlaps its source");
        let lay = c.plan(&[0]).expect("plan");
        assert!(lay.walk_cap[0] >= lay.slot_cap[0], "walk region >= slot region (the staging bound)");
        assert_eq!(c.stats.grows, 1);
        assert_eq!(c.slot_needed(0, 30_000).expect("fits"), None, "already big enough: no move");
    }

    #[test]
    fn the_ceilings_refuse_by_name() {
        let mut c = cap();
        let _ = c.plan(&[0]).expect("plan");
        let e = c.walk_needed(0, (1 << 15) + 1).expect_err("past max_cap");
        assert!(e.contains("one space may hold"), "{e}");
        let e = c.slot_needed(0, (1 << 15) + 1).expect_err("past max_cap");
        assert!(e.contains("one space may hold"), "{e}");
        // Exhaust the slot pool: 64 × 1024 = 65 536 = the pool.
        let mut c = Capacity::new(128, 1 << 20, 64 * 1024, 32 * 1024, 1024).expect("config");
        for s in 0..64 {
            let _ = c.plan(&[s]).expect("fits");
        }
        let e = c.plan(&[64]).expect_err("pool full");
        assert!(e.contains("slot pool exhausted"), "{e}");
        assert!(c.stats.ceilings >= 1);
        // The walk pool: two entries of 40 000 runs do not fit 65 536.
        let mut c = Capacity::new(8, 1 << 16, 1 << 17, 1 << 16, 1024).expect("config");
        let _ = c.plan(&[0, 1]).expect("plan");
        let _ = c.walk_needed(0, 40_000);
        let _ = c.walk_needed(1, 40_000);
        let e = c.plan(&[0, 1]).expect_err("walk pool");
        assert!(e.contains("walk pool exhausted"), "{e}");
    }

    /// ★ Teardown releases exactly what the object owned: the region returns, coalesced, and a
    /// churn of create/grow/release (the per-process lifecycle) never leaks a run.
    #[test]
    fn release_returns_exactly_what_the_slot_owned_across_a_churn() {
        let size = 2 << 20;
        let mut c = Capacity::new(128, 1 << 20, size, 1 << 20, 1024).expect("config");
        for round in 0..50u32 {
            // A "process": 7 objects, one of which (the CUDA space) grows past 16 384.
            let slots: Vec<u32> = (0..7).map(|k| (round * 7 + k) % 128).collect();
            let _ = c.plan(&slots).expect("plan");
            let big = slots[5];
            let _ = c.walk_needed(big, 20_000 + round * 100).expect("fits");
            let _ = c.slot_needed(big, 20_000 + round * 100).expect("grows");
            let _ = c.plan(&slots).expect("plan after growth");
            for &s in &slots {
                c.release(s);
            }
            assert_eq!(c.slot_pool_free(), u64::from(size), "round {round}: every run came back");
            assert_eq!(c.slots.free.len(), 1, "round {round}: coalesced back to one region");
        }
    }
}
