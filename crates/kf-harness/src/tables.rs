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

