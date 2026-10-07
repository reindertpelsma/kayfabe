//! The guest's page tables, as the harness writes them.

use kf_cuda::synth::{DUAL_HALF_ABSENT, Image, pde, pte, pte_sys, vi0, vi1, vi2, vi3, vis};
use std::collections::HashMap;

/// A lazily-built VER2 (GA10x) page-table tree inside an [`Image`] placed at `base` in the store —
/// the guest kernel's tables, written by the harness exactly as the driver would lay them out.
pub struct Tree {
    /// The image (its bytes go to the store at its origin).
    pub img: Image,
    /// The root (PDB) — a store offset, i.e. a guest FB-physical address.
    pub root: u64,
    tables: HashMap<(u8, u64, usize), u64>,
}

impl Tree {
    /// An empty tree whose image starts at store offset `base`.
    #[must_use]
    pub fn new(base: u64, bytes: usize) -> Tree {
        let mut img = Image::at(base, bytes);
        let root = img.alloc(4 * 8, 4096);
        Tree {
            img,
            root,
            tables: HashMap::new(),
        }
    }
    fn child(
        &mut self,
        level: u8,
        parent: u64,
        idx: usize,
        bytes: u64,
        entry: u64,
        dual: bool,
    ) -> u64 {
        if let Some(&c) = self.tables.get(&(level, parent, idx)) {
            return c;
        }
        let c = self.img.alloc(bytes, 4096);
        if dual {
            // ⊘ 2026-10-07: the big half is INVALID, not `big_pde(0)` (a present table at FB 0;
            // `kf_cuda::synth::DUAL_HALF_ABSENT`).
            self.img.put64(parent + entry, DUAL_HALF_ABSENT);
            self.img.put64(parent + entry + 8, pde(c));
        } else {
            self.img.put64(parent + entry, pde(c));
        }
        self.tables.insert((level, parent, idx), c);
        c
    }
    /// Map the 4 KiB page at `va` to FB-physical `phys`.
    pub fn map4k(&mut self, va: u64, phys: u64) {
        self.leaf4k(va, pte(phys));
    }

    /// Map the 4 KiB page at `va` to guest-physical `gpa` in coherent system memory.
    pub fn map4k_sys(&mut self, va: u64, gpa: u64) {
        self.leaf4k(va, pte_sys(gpa));
    }

    /// ★ v3-roperm: [`Tree::map4k`] / [`Tree::map4k_sys`] with raw PTE permission `bits` OR-ed into
    /// the leaf (the caller derives them from the format descriptor's bit positions).
    pub fn map4k_bits(&mut self, va: u64, at: u64, sys: bool, bits: u64) {
        self.leaf4k(va, if sys { pte_sys(at) } else { pte(at) } | bits);
    }

    /// Unmap the 4 KiB page at `va` (its PTE becomes 0; the tables stay).
    pub fn unmap4k(&mut self, va: u64) {
        self.leaf4k(va, 0);
    }

    /// ★ 2026-10-07 — the replay of Windows run43's directory-level move: the shift-29 level
    /// (PD1) on `va`'s path moves to a new instance at store offset `new_at`. The parent entry in
    /// this image is repointed and the old instance zeroed; the returned bytes are the level's
    /// contents, which the CALLER writes at `new_at` (it may lie outside this image — FB offset 0
    /// in the run). `None` if `va` has no PD1 yet. Afterwards the tree must not be edited under
    /// `va` again (its record still names the old instance).
    pub fn move_pd1(&mut self, va: u64, new_at: u64) -> Option<Vec<u8>> {
        let pd2 = *self.tables.get(&(3, self.root, vi3(va)))?;
        let pd1 = *self.tables.get(&(2, pd2, vi2(va)))?;
        Some(move_level(
            &mut self.img,
            pd2 + 8 * vi2(va) as u64,
            pd1,
            512 * 8,
            pde(new_at),
        ))
    }

    fn leaf4k(&mut self, va: u64, leaf: u64) {
        let pd2 = self.child(3, self.root, vi3(va), 512 * 8, 8 * vi3(va) as u64, false);
        let pd1 = self.child(2, pd2, vi2(va), 512 * 8, 8 * vi2(va) as u64, false);
        let pd0 = self.child(1, pd1, vi1(va), 256 * 16, 8 * vi1(va) as u64, false);
        let small = self.child(0, pd0, vi0(va), 512 * 8, 16 * vi0(va) as u64, true);
        self.img.put64(small + 8 * vis(va) as u64, leaf);
    }
}

/// ★ A VER3 (Hopper, Blackwell) tree — PD4 → PD3 → PD2 → PD1 → PD0 (dual) → PT — built exactly as the
/// family's driver lays it out, so the walk kernel's VER3 decode can be proved on ANY GPU (the walker
/// decodes guest bytes; the host's own MMU never walks these tables).
pub struct Tree3 {
    /// The image.
    pub img: Image,
    /// The root (PD4) — a store offset.
    pub root: u64,
    tables: HashMap<(u8, u64, usize), u64>,
}

impl Tree3 {
    /// An empty tree whose image starts at store offset `base`.
    #[must_use]
    pub fn new(base: u64, bytes: usize) -> Tree3 {
        let mut img = Image::at(base, bytes);
        let root = img.alloc(2 * 8, 4096);
        Tree3 {
            img,
            root,
            tables: HashMap::new(),
        }
    }

    fn child(&mut self, level: u8, parent: u64, idx: usize, bytes: u64, stride: u64) -> u64 {
        if let Some(&c) = self.tables.get(&(level, parent, idx)) {
            return c;
        }
        let c = self.img.alloc(bytes, 4096);
        let at = parent + stride * idx as u64;
        if level == 0 {
            let [lo, hi] = kf_cuda::synth::ver3::dual_small(c);
            self.img.put64(at, lo);
            self.img.put64(at + 8, hi);
        } else {
            self.img.put64(at, kf_cuda::synth::ver3::pde(c));
        }
        self.tables.insert((level, parent, idx), c);
        c
    }

    /// [`Tree::move_pd1`] for VER3: the shift-29 level (PD1, `ver3::idx(va)[3]` indexes it).
    pub fn move_pd1(&mut self, va: u64, new_at: u64) -> Option<Vec<u8>> {
        let [i4, i3, i2, ..] = kf_cuda::synth::ver3::idx(va);
        let pd3 = *self.tables.get(&(4, self.root, i4))?;
        let pd2 = *self.tables.get(&(3, pd3, i3))?;
        let pd1 = *self.tables.get(&(2, pd2, i2))?;
        Some(move_level(
            &mut self.img,
            pd2 + 8 * i2 as u64,
            pd1,
            512 * 8,
            kf_cuda::synth::ver3::pde(new_at),
        ))
    }

    fn leaf4k(&mut self, va: u64, leaf: u64) {
        let [i4, i3, i2, i1, i0, it] = kf_cuda::synth::ver3::idx(va);
        // RM's VER3 PDEs carry no valid bit at any level (aperture is the validity).
        let pd3 = self.child(4, self.root, i4, 512 * 8, 8);
        let pd2 = self.child(3, pd3, i3, 512 * 8, 8);
        let pd1 = self.child(2, pd2, i2, 512 * 8, 8);
        let pd0 = self.child(1, pd1, i1, 256 * 16, 8);
        let pt = self.child(0, pd0, i0, 512 * 8, 16);
        self.img.put64(pt + 8 * it as u64, leaf);
    }

    /// Map the 4 KiB page at `va` to FB-physical `phys`.
    pub fn map4k(&mut self, va: u64, phys: u64) {
        self.leaf4k(va, kf_cuda::synth::ver3::pte(phys));
    }

    /// Map the 4 KiB page at `va` to guest-physical `gpa` in coherent system memory.
    pub fn map4k_sys(&mut self, va: u64, gpa: u64) {
        self.leaf4k(va, kf_cuda::synth::ver3::pte_sys(gpa));
    }

    /// ★ v3-roperm: [`Tree3::map4k`] / [`Tree3::map4k_sys`] with raw PCF permission `bits` OR-ed
    /// into the leaf (the caller derives them from the format descriptor's bit positions).
    pub fn map4k_bits(&mut self, va: u64, at: u64, sys: bool, bits: u64) {
        let leaf = if sys {
            kf_cuda::synth::ver3::pte_sys(at)
        } else {
            kf_cuda::synth::ver3::pte(at)
        };
        self.leaf4k(va, leaf | bits);
    }

    /// Unmap the 4 KiB page at `va` (its PTE becomes 0; the tables stay).
    pub fn unmap4k(&mut self, va: u64) {
        self.leaf4k(va, 0);
    }
}

/// Move one directory level: copy out `bytes` of the instance at `old`, zero it, and write
/// `new_entry` at `parent_entry`. Both offsets are absolute store offsets inside `img`.
fn move_level(img: &mut Image, parent_entry: u64, old: u64, bytes: u64, new_entry: u64) -> Vec<u8> {
    let lo = usize::try_from(old - img.origin).expect("a store offset fits usize");
    let hi = lo + usize::try_from(bytes).expect("a table size fits usize");
    let out = img.mem[lo..hi].to_vec();
    img.mem[lo..hi].fill(0);
    img.put64(parent_entry, new_entry);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(img: &Image, off: u64) -> u64 {
        let o = usize::try_from(off - img.origin).unwrap();
        u64::from_le_bytes(img.mem[o..o + 8].try_into().unwrap())
    }

    /// The level-move helper (run43's replay, kf-gate9) moves exactly the PD1 instance: the parent
    /// names the new address, the old instance is zero, and the returned bytes are its contents.
    #[test]
    fn move_pd1_repoints_the_parent_and_returns_the_level() {
        let va = 0x2000_0000u64;
        let mut t = Tree::new(0x10_0000, 1 << 20);
        t.map4k(va, 0x80_0000);
        let pd2 = t.tables[&(3, t.root, vi3(va))];
        let pd1 = t.tables[&(2, pd2, vi2(va))];
        let pd0 = t.tables[&(1, pd1, vi1(va))];
        let level = t.move_pd1(va, 0).expect("PD1 exists");
        assert_eq!(level.len(), 4096);
        let o = 8 * vi1(va);
        assert_eq!(
            u64::from_le_bytes(level[o..o + 8].try_into().unwrap()),
            pde(pd0)
        );
        assert_eq!(word(&t.img, pd2 + 8 * vi2(va) as u64), pde(0));
        assert_eq!(
            word(&t.img, pd1 + o as u64),
            0,
            "the old instance is zeroed"
        );
        // ⊘ pde(0) is a PRESENT entry (aperture VIDEO), not an empty one.
        assert_ne!(pde(0), 0);
    }

    #[test]
    fn ver3_move_pd1_repoints_the_parent() {
        let va = 0x2000_0000u64;
        let mut t = Tree3::new(0x10_0000, 1 << 20);
        t.map4k(va, 0x80_0000);
        let [i4, i3, i2, ..] = kf_cuda::synth::ver3::idx(va);
        let pd3 = t.tables[&(4, t.root, i4)];
        let pd2 = t.tables[&(3, pd3, i3)];
        let level = t.move_pd1(va, 0).expect("PD1 exists");
        assert!(level.iter().any(|&b| b != 0));
        assert_eq!(
            word(&t.img, pd2 + 8 * i2 as u64),
            kf_cuda::synth::ver3::pde(0)
        );
    }

    /// ⊘ 2026-10-07: a dual PDE's unused big half is INVALID, never `big_pde(0)` (a present
    /// big-page table at FB 0 to the hardware and, now, to the walk kernel).
    #[test]
    fn the_unused_big_half_is_invalid() {
        let va = 0x2000_0000u64;
        let mut t = Tree::new(0x10_0000, 1 << 20);
        t.map4k(va, 0x80_0000);
        let pd2 = t.tables[&(3, t.root, vi3(va))];
        let pd1 = t.tables[&(2, pd2, vi2(va))];
        let pd0 = t.tables[&(1, pd1, vi1(va))];
        assert_eq!(word(&t.img, pd0 + 16 * vi0(va) as u64), DUAL_HALF_ABSENT);
        assert_eq!(DUAL_HALF_ABSENT, 0);
    }
}
