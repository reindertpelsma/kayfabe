//! The guest's page tables, as the harness writes them.

use kf_cuda::synth::{Image, big_pde, pde, pte, pte_sys, vi0, vi1, vi2, vi3, vis};
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
        Tree { img, root, tables: HashMap::new() }
    }
    fn child(&mut self, level: u8, parent: u64, idx: usize, bytes: u64, entry: u64, dual: bool) -> u64 {
        if let Some(&c) = self.tables.get(&(level, parent, idx)) {
            return c;
        }
        let c = self.img.alloc(bytes, 4096);
        if dual {
            self.img.put64(parent + entry, big_pde(0));
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
        Tree3 { img, root, tables: HashMap::new() }
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
}
