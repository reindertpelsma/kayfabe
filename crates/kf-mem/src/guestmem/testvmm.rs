//! A MODEL HYPERVISOR for tests: guest RAM blocks that are plain Rust memory, a layout epoch that
//! moves on add / remove / resize, and a guard that keeps the memory of the layout it saw alive
//! (an `Arc` per block) even after the VMM "frees" the block — the property QEMU's `MemoryRegion`
//! reference gives. No `unsafe`: the segments are made by [`RamSegment::over_cells`].
//!
//! Resizeable blocks keep their base: the memory is allocated at the maximum length once, a resize
//! only changes how much of it is guest RAM (QEMU reserves `max_length` of address space).

use super::{
    GuestMemory, LayoutEpoch, LayoutGuard, RamBlockSpec, RamLayout, TranslateError, Translated,
    translate_blocks,
};
use kf_linux_raw::RamSegment;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// A block's memory: allocated at its maximum length, never moved.
pub(crate) type Mem = Arc<Box<[AtomicU8]>>;

#[derive(Clone)]
struct Blk {
    gpa: u64,
    len: u64,
    mem: Mem,
}

/// The model VMM.
pub(crate) struct TestVmm {
    blocks: Mutex<Vec<Blk>>,
    epoch: AtomicU64,
}

fn cells(n: usize) -> Mem {
    Arc::new((0..n).map(|_| AtomicU8::new(0)).collect())
}

impl TestVmm {
    pub(crate) fn new() -> Self {
        Self {
            blocks: Mutex::new(Vec::new()),
            epoch: AtomicU64::new(1),
        }
    }

    fn bump(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }

    /// Register a block of `len` bytes, sorted into place.
    pub(crate) fn add(&self, gpa: u64, len: u64) {
        self.add_resizeable(gpa, len, len);
    }

    /// Register a block that can later grow to `max` bytes without moving.
    pub(crate) fn add_resizeable(&self, gpa: u64, len: u64, max: u64) {
        let mem = cells(usize::try_from(max).expect("small test block"));
        let mut v = self.blocks.lock().expect("lock");
        v.push(Blk { gpa, len, mem });
        v.sort_by_key(|b| b.gpa);
        drop(v);
        self.bump();
    }

    /// The VMM frees the block at `gpa`. Guards that already hold it keep its memory.
    pub(crate) fn remove(&self, gpa: u64) {
        self.blocks.lock().expect("lock").retain(|b| b.gpa != gpa);
        self.bump();
    }

    /// Resize the block at `gpa` in place (its base does not move).
    pub(crate) fn resize(&self, gpa: u64, new_len: u64) {
        let mut v = self.blocks.lock().expect("lock");
        let b = v.iter_mut().find(|b| b.gpa == gpa).expect("block");
        assert!(new_len as usize <= b.mem.len(), "resize past max_length");
        b.len = new_len;
        drop(v);
        self.bump();
    }

    /// Write byte `f(offset)` into every byte of the block at `gpa`.
    pub(crate) fn fill(&self, gpa: u64, f: impl Fn(usize) -> u8) {
        let v = self.blocks.lock().expect("lock");
        let b = v.iter().find(|b| b.gpa == gpa).expect("block");
        for (i, c) in b.mem.iter().enumerate() {
            c.store(f(i), Ordering::Relaxed);
        }
    }

    /// The byte at guest-physical `gpa`, read straight from the model (the oracle for content).
    pub(crate) fn peek(&self, gpa: u64) -> Option<u8> {
        let v = self.blocks.lock().expect("lock");
        v.iter()
            .find(|b| gpa >= b.gpa && gpa - b.gpa < b.len)
            .and_then(|b| b.mem.get(usize::try_from(gpa - b.gpa).ok()?))
            .map(|c| c.load(Ordering::Relaxed))
    }
}

/// The guard: a snapshot of the layout and a hold (an `Arc`) on each block's memory.
pub(crate) struct TestGuard<'v> {
    vmm: &'v TestVmm,
    epoch: LayoutEpoch,
    layout: RamLayout,
    mems: Vec<Mem>,
}

impl LayoutGuard for TestGuard<'_> {
    fn epoch(&self) -> LayoutEpoch {
        self.epoch
    }

    fn still_current(&self) -> bool {
        self.vmm.epoch() == self.epoch
    }

    fn layout(&self) -> &RamLayout {
        &self.layout
    }

    fn translate(&self, gpa: u64, len: u64) -> Result<Translated<'_>, TranslateError> {
        let mut whole: Vec<RamSegment<'_>> = Vec::new();
        for (m, spec) in self.mems.iter().zip(self.layout.blocks()) {
            let n = usize::try_from(spec.len).map_err(|_| TranslateError::TooFragmented)?;
            let cells = m
                .get(..n)
                .ok_or(TranslateError::Inconsistent { index: whole.len() })?;
            whole.push(
                RamSegment::over_cells(cells)
                    .map_err(|_| TranslateError::Inconsistent { index: whole.len() })?,
            );
        }
        translate_blocks(&self.layout, &whole, self.epoch, gpa, len)
    }
}

impl GuestMemory for TestVmm {
    type Guard<'g> = TestGuard<'g>;

    fn lock_layout(&self) -> Result<TestGuard<'_>, TranslateError> {
        let v = self
            .blocks
            .lock()
            .map_err(|_| TranslateError::LayoutChanged)?;
        let specs: Vec<RamBlockSpec> = v
            .iter()
            .map(|b| RamBlockSpec {
                gpa: b.gpa,
                len: b.len,
            })
            .collect();
        let layout = RamLayout::new(&specs).map_err(|_| TranslateError::Unbacked { gpa: 0 })?;
        let mems = v.iter().map(|b| b.mem.clone()).collect();
        let epoch = LayoutEpoch::new(self.epoch.load(Ordering::SeqCst));
        drop(v);
        Ok(TestGuard {
            vmm: self,
            epoch,
            layout,
            mems,
        })
    }

    fn epoch(&self) -> LayoutEpoch {
        LayoutEpoch::new(self.epoch.load(Ordering::SeqCst))
    }
}
