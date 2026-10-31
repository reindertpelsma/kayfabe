//! ★★★ **The QEMU layout guard** (`docs/design/V3_GUEST_MEMORY.md` §2, owner direction 2026-10-31):
//! the hold that makes a [`RamSegment`] over a QEMU RAM block safe to use.
//!
//! QEMU frees a RAM block through `call_rcu(reclaim_ramblock)` once its `MemoryRegion` is
//! finalized (`system/physmem.c` `qemu_ram_free`; `system/memory.c` `memory_region_destructor_ram`),
//! and a `MemoryRegion` cannot be finalized while somebody holds a reference on its owner
//! (`memory_region_ref` = `object_ref(mr->owner)`). So:
//!
//! - [`QemuLayoutGuard::take`] is the ONLY way to make a guard. It is given the registry's READ
//!   guard (so the blocks it reads are registered right now, which is when taking a reference on
//!   their owners is valid), takes a hold on every block's owner ([`MrHold::hold`]), then releases
//!   the registry lock — a hot-unplug is never blocked behind a long host call, it just cannot free
//!   the memory while the guard lives.
//! - [`QemuLayoutGuard::translate`] makes each [`RamSegment`] from the address of a block THIS
//!   guard holds ([`RamSegment::from_raw`], one `unsafe` block, borrowing `&self`), so a segment
//!   cannot outlive the hold. Nothing here returns or stores an address as an integer.
//! - The guard carries the layout epoch (the registry's generation, bumped by every RAM block add or
//!   removal the memory listener reports) and [`QemuLayoutGuard::still_current`] re-reads it.
//!
//! ⊘ **Why no RCU read lock.** The owner's brief offered "RCU read lock + `MemoryRegion` reference".
//! The reference alone keeps the block alive; the RCU read lock would add a per-thread
//! `rcu_register_thread` (`include/qemu/rcu.h`: "each thread containing read-side critical sections
//! must be registered") and — worse — a read-side section as long as the host RM's pin (seconds for
//! all of guest RAM) would stall every `drain_call_rcu()`/`synchronize_rcu()` caller, among them
//! QMP `device_add`/`device_del` on the main loop with the BQL held (`system/qdev-monitor.c:875`,
//! `:1021`) — the very hot-plug this guard exists for. See the design note.

use super::{MrRefFn, MrUnrefFn, Owner, RawRegion};
use crate::mem::RamBlock;
use core::ptr::NonNull;
use kf_linux_raw::RamSegment;
use kf_mem::guestmem::{
    LayoutEpoch, LayoutGuard, RamBlockSpec, RamLayout, TranslateError, Translated, translate_blocks,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLockReadGuard};

/// How the VMM keeps a RAM block's owner alive. Production: QEMU's `memory_region_ref` /
/// `memory_region_unref` ([`CMrHold`]); tests: a recorder.
pub trait MrHold: Send + Sync + core::fmt::Debug {
    /// Take one hold on the owner of a block. Called with the block registered.
    fn hold(&self, owner: Owner);
    /// Give that hold back.
    fn release(&self, owner: Owner);
}

/// The C verbs `memory_region_ref` / `memory_region_unref`, registered by the device at realize
/// (`Kf3RamOps`).
///
/// **Type invariant:** `hold` and `release` are QEMU's reference-count verbs for the owner pointers
/// `kf3_ram_add` carried: callable from any thread with such a pointer while its block is
/// registered, and `release` callable later with a pointer `hold` took a reference on. The ONLY
/// constructor is `pub(crate)` and has one call site, `kf3_ram_ops` (an FFI entry whose contract is
/// exactly that), so nothing else in the crate can put another function behind it.
#[derive(Clone, Copy, Debug)]
pub struct CMrHold {
    hold: MrRefFn,
    release: MrUnrefFn,
}

impl CMrHold {
    /// Wrap the VMM's two verbs (see the type invariant).
    #[must_use]
    pub(crate) fn new(hold: MrRefFn, release: MrUnrefFn) -> Self {
        CMrHold { hold, release }
    }

    fn call(&self, take: bool, owner: Owner) {
        if !owner.is_some() {
            return;
        }
        let verb = if take { self.hold } else { self.release };
        // SAFETY: the type invariant — `verb` is QEMU's `memory_region_ref` (`take`, only called by
        // `QemuLayoutGuard::take` while the block is registered, under the registry's read lock) or
        // `memory_region_unref` (releasing a reference `hold` took on this very owner).
        unsafe { verb(owner.0) };
    }
}

impl MrHold for CMrHold {
    fn hold(&self, owner: Owner) {
        self.call(true, owner);
    }

    fn release(&self, owner: Owner) {
        self.call(false, owner);
    }
}

/// ★ A hold on QEMU's guest-RAM blocks at one layout epoch. See the module docs.
pub struct QemuLayoutGuard<'m> {
    generation: &'m AtomicU64,
    epoch: LayoutEpoch,
    layout: RamLayout,
    regions: Vec<RawRegion>,
    hold: Arc<dyn MrHold>,
}

impl core::fmt::Debug for QemuLayoutGuard<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("QemuLayoutGuard")
            .field("epoch", &self.epoch)
            .field("blocks", &self.regions.len())
            .finish()
    }
}

impl<'m> QemuLayoutGuard<'m> {
    /// Take the guard: read the blocks the registry holds RIGHT NOW (`blocks` is its read guard),
    /// hold each block's owner, release the registry lock.
    ///
    /// # Errors
    /// [`TranslateError::Inconsistent`] when the registered blocks are not a layout (overlap,
    /// zero length): refused, never guessed at.
    pub fn take(
        blocks: RwLockReadGuard<'m, Vec<RamBlock>>,
        generation: &'m AtomicU64,
        hold: Arc<dyn MrHold>,
    ) -> Result<Self, TranslateError> {
        // `add` and `del` bump the generation under the WRITE lock, so under this read guard the
        // generation and the blocks belong together.
        let epoch = LayoutEpoch::new(generation.load(Ordering::Acquire));
        let specs: Vec<RamBlockSpec> = blocks
            .iter()
            .map(|b| RamBlockSpec {
                gpa: b.gpa,
                len: b.mem.len() as u64,
            })
            .collect();
        let layout = RamLayout::new(&specs).map_err(|e| {
            use kf_mem::guestmem::LayoutError as L;
            TranslateError::Inconsistent {
                index: match e {
                    L::ZeroLength { index }
                    | L::GpaOverflow { index }
                    | L::Unsorted { index }
                    | L::Overlap { index } => index,
                    L::TooManyBlocks => usize::MAX,
                },
            }
        })?;
        let regions: Vec<RawRegion> = blocks.iter().map(|b| b.mem).collect();
        // The blocks are registered (we hold the read lock), so their owners are alive: take the hold.
        for r in regions.iter().filter(|r| r.owner.is_some()) {
            hold.hold(r.owner);
        }
        drop(blocks);
        Ok(QemuLayoutGuard {
            generation,
            epoch,
            layout,
            regions,
            hold,
        })
    }
}

impl Drop for QemuLayoutGuard<'_> {
    fn drop(&mut self) {
        for r in self.regions.iter().filter(|r| r.owner.is_some()) {
            self.hold.release(r.owner);
        }
    }
}

impl LayoutGuard for QemuLayoutGuard<'_> {
    fn epoch(&self) -> LayoutEpoch {
        self.epoch
    }

    fn still_current(&self) -> bool {
        LayoutEpoch::new(self.generation.load(Ordering::Acquire)) == self.epoch
    }

    fn layout(&self) -> &RamLayout {
        &self.layout
    }

    fn translate(&self, gpa: u64, len: u64) -> Result<Translated<'_>, TranslateError> {
        let mut whole: Vec<RamSegment<'_>> = Vec::with_capacity(self.regions.len());
        for (index, r) in self.regions.iter().enumerate() {
            let base = NonNull::new(r.ptr).ok_or(TranslateError::Inconsistent { index })?;
            // SAFETY: `r` is one of `self.regions`: the block's owner has a hold taken in `take`
            // while the block was registered and released only in `drop`, so QEMU cannot finalize
            // the `MemoryRegion` — and `call_rcu(reclaim_ramblock)` cannot run — for as long as
            // `self` is borrowed; the segment borrows `self`. The block is mapped read-write for
            // `r.len` bytes (`RawRegion::adopt`'s contract).
            let seg = unsafe { RamSegment::from_raw(base, r.len, self) }
                .map_err(|_| TranslateError::Inconsistent { index })?;
            whole.push(seg);
        }
        translate_blocks(&self.layout, &whole, self.epoch, gpa, len)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_aligned_ram_owned;
    use super::*;
    use std::sync::{Mutex, RwLock};

    #[derive(Default, Debug)]
    struct Rec {
        log: Mutex<Vec<String>>,
    }
    impl MrHold for Rec {
        fn hold(&self, o: Owner) {
            self.log.lock().unwrap().push(format!("hold {:?}", o.0));
        }
        fn release(&self, o: Owner) {
            self.log.lock().unwrap().push(format!("release {:?}", o.0));
        }
    }

    fn blocks(spec: &[(u64, usize, usize)]) -> RwLock<Vec<RamBlock>> {
        RwLock::new(
            spec.iter()
                .map(|&(gpa, len, tag)| RamBlock {
                    gpa,
                    mem: test_aligned_ram_owned(len, tag),
                    fd: None,
                    fd_off: 0,
                })
                .collect(),
        )
    }

    #[test]
    fn the_guard_holds_every_owner_for_its_life_and_releases_them_when_dropped() {
        let reg = blocks(&[
            (0, 0x2000, 0x10),
            (0x4000, 0x1000, 0x20),
            (0x8000, 0x1000, 0),
        ]);
        let gen_ = AtomicU64::new(7);
        let rec = Arc::new(Rec::default());
        {
            let g = QemuLayoutGuard::take(reg.read().unwrap(), &gen_, rec.clone()).expect("guard");
            assert_eq!(g.epoch(), LayoutEpoch::new(7));
            assert_eq!(g.layout().blocks().len(), 3);
            // The registry lock is NOT held while the guard lives: a writer (QEMU's main loop,
            // removing a block) is never blocked behind a long host call.
            assert!(reg.try_write().is_ok());
            // Held: both owners (the third has none, so it is not counted).
            assert_eq!(rec.log.lock().unwrap().len(), 2);
        }
        let log = rec.log.lock().unwrap().clone();
        assert_eq!(log.len(), 4, "{log:?}");
        assert!(log[0].starts_with("hold") && log[1].starts_with("hold"));
        assert!(log[2].starts_with("release") && log[3].starts_with("release"));
    }

    #[test]
    fn segments_reach_the_block_bytes_across_a_block_edge_and_refuse_everything_else() {
        let reg = blocks(&[
            (0x1000, 0x1000, 1),
            (0x2000, 0x1000, 2),
            (0x9000, 0x1000, 3),
        ]);
        for (i, b) in reg.read().unwrap().iter().enumerate() {
            for k in 0..0x1000usize {
                assert!(b.mem.store(k, (k + i * 3) as u64 & 0xff, 1));
            }
        }
        let gen_ = AtomicU64::new(1);
        let g = QemuLayoutGuard::take(reg.read().unwrap(), &gen_, Arc::new(Rec::default()))
            .expect("guard");
        let t = g.translate(0x1ff0, 0x20).expect("two blocks");
        assert_eq!(t.segments().len(), 2);
        let mut got = [0u8; 0x20];
        t.read_into(&mut got).expect("read");
        for (k, b) in got.iter().enumerate() {
            let want = if k < 0x10 {
                (0xff0 + k) as u8
            } else {
                ((k - 0x10) + 3) as u8
            };
            assert_eq!(*b, want, "byte {k}");
        }
        t.write_from(&[0xEE; 0x20]).expect("write");
        assert_eq!(
            reg.read().unwrap()[0].mem.load_u32(0xffc),
            Some(0xEEEE_EEEE)
        );
        assert_eq!(reg.read().unwrap()[1].mem.load_u32(0), Some(0xEEEE_EEEE));
        // Hole, past the end, zero length, u64::MAX everything: errors, never segments.
        for (gpa, len) in [
            (0x3000u64, 8u64),
            (0x2ff8, 16),
            (0xa000, 1),
            (0x1000, 0),
            (u64::MAX, u64::MAX),
            (u64::MAX, 1),
            (0, u64::MAX),
            (0x1000, u64::MAX),
        ] {
            assert!(g.translate(gpa, len).is_err(), "{gpa:#x}+{len:#x}");
        }
    }

    #[test]
    fn the_epoch_moves_under_a_live_guard_and_the_guard_notices() {
        let reg = blocks(&[(0, 0x1000, 1)]);
        let gen_ = AtomicU64::new(5);
        let g = QemuLayoutGuard::take(reg.read().unwrap(), &gen_, Arc::new(Rec::default()))
            .expect("guard");
        assert!(g.still_current());
        gen_.store(6, Ordering::Release); // a RAM block was added or removed
        assert!(!g.still_current());
        assert_eq!(g.epoch(), LayoutEpoch::new(5));
        // The guard still reads the layout it saw (it holds those blocks).
        assert!(g.translate(0, 8).is_ok());
        gen_.store(0, Ordering::Release); // terminal revoke
        assert!(!g.still_current());
    }

    #[test]
    fn overlapping_blocks_are_not_a_layout_and_nothing_is_held() {
        let reg = blocks(&[(0, 0x2000, 1), (0x1000, 0x1000, 2)]);
        let gen_ = AtomicU64::new(1);
        let rec = Arc::new(Rec::default());
        let e =
            QemuLayoutGuard::take(reg.read().unwrap(), &gen_, rec.clone()).expect_err("overlap");
        assert_eq!(e, TranslateError::Inconsistent { index: 1 });
        assert!(rec.log.lock().unwrap().is_empty(), "held before validating");
    }
}
