//! ★★★★★ **THE GUEST'S CPU WINDOWS — BAR2 (and BAR1) as a second apply target, PRAMIN as a pool of
//! pre-armed views** (`V3_P4_PORT_MAP.md` §2.3(b), §2.4, Q2, Q3; `THE_CONSTRAINTS.md` §49.3, §16).
//!
//! A guest BAR aperture is ONE host address range under ONE memslot, created at realize and never
//! unmapped. What each page of it shows is decided by `mmap(MAP_FIXED)` placements inside it:
//! - a **view of the store** (an armed `NV_ESC_RM_MAP_MEMORY` node of the one reserved object), or
//! - the **guest-RAM memfd** at a file offset (a sysmem leaf), or
//! - the window's **scratch** — one small memfd TILE per window, mapped again and again (window
//!   offset `o` shows tile byte `o % T`; `kf_linux_raw::scratch`). ⊘ Never a hole: a memslot over
//!   an unmapped range, or a `PROT_NONE` one, is `KVM_RUN → EFAULT` and kills the guest
//!   (`userfaultfd_is_ruled_out`). The scratch is not a shadow (§18.2): nothing is ever copied or
//!   synced between it and the store, and its contents are undefined (they repeat every `T` bytes).
//!   ⊘ **Corrected 2026-10-03:** *"the worst a guest can do with it is corrupt itself"* was false
//!   while the scratch was one memfd of the whole window. A guest READ of every unmapped page then
//!   allocated the whole window in host RAM, charged to the VMM, because shmem has no zero page.
//!   Tiled, the worst is self-corruption plus at most `T` of host RAM per window: 2 MiB for BAR1 and
//!   BAR2 at their default sizes, 1 MiB for PRAMIN (`V3_P4_PORT_MAP.md` Q3). A sink is one `mmap`
//!   per tile it touches, so a PRAMIN sink (the window is its own tile) stays ONE.
//!
//! ## BAR2 / BAR1: [`CpuWindow`], a [`MapTarget`]
//!
//! The walk of the BAR's page tables (the GPU walker's, never a CPU read) says what the guest's
//! tables express, diffed ON THE GPU against the placements this window confirmed (commit on ack,
//! `kf_cuda::diffmodel`); [`crate::apply`] lands the diff here. A map arms a view of exactly the walked run and places it; an unmap re-points the
//! run to scratch FIRST and only then releases the view's host BAR1 aperture (`0x4F`,
//! `bar1_simultaneous_view_ceiling.md` Q3 — closing the node is not a release). A refusal is
//! returned by name, never clamped: only what landed is acknowledged (so committed), and the VA
//! manager leaves the guest's invalidate armed. ★ This window's `placed` map is the one CPU record
//! that must stay: it holds the armed VIEW HANDLES, and releasing a view's host aperture needs the
//! handle — no diff can carry it.
//!
//! ## PRAMIN: [`PraminPool`]
//!
//! The window is re-pointed INSIDE the trapped window-base write, synchronously: the guest's next
//! PRAMIN access must see the new window and there is no later point to defer to. ★ **Owner ruling
//! 2026-09-25: this is the ONE sanctioned exception to "no syscalls on a vCPU", and it gets ONE host
//! map + ONE `mmap` per window move** — not the 16 per-granule placements of pre-armed views it
//! replaces (`[measured 5dd01b48]` ~400-540 µs worst in the trap, each `mmap` under the driver's
//! per-device semaphore). The 16 slots coalesce into runs (normally ONE: 1 MiB of store, or of
//! guest RAM); a store run costs one `NV_ESC_RM_MAP_MEMORY` on a node opened AHEAD of time plus one
//! `mmap(MAP_FIXED)`; a RAM or scratch run costs one `mmap`. The replaced views are handed to
//! [`ViewOps::retire`], which releases them OFF the vCPU — ⊘ (2026-10-03) only once every slot a
//! view was placed over has been re-placed since, by this re-point or earlier ones; a view a refused
//! run may have left reachable is kept and counted ([`Repointed::kept`], [`PraminPool::kept`]). A
//! slot with no addressable source shows scratch and is **counted and named**.
//!
//! ⊘ Nothing here reads or writes a byte of guest memory.

use crate::ledger::{Desired, MapTarget, Mapped};
use core::sync::atomic::{AtomicU64, Ordering};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

/// ★ The host verbs one window's placements need. Production is kf-qemu's (an armed RM node +
/// `GuestWindow::place_device_view`); tests use a recorder. Every verb is one WE author: no guest
/// value reaches a host flag word.
pub trait ViewOps {
    /// An armed view of a store slice (holds its host aperture until [`ViewOps::release`]).
    type View;

    /// Arm a CPU view of store `[off, off+len)`. ⊘ An RM ioctl: never on a vCPU.
    ///
    /// # Errors
    /// The host's refusal, by name (e.g. `NV_ERR_NO_MEMORY` when the host BAR1 pool is full).
    fn arm_store(&self, off: u64, len: u64) -> Result<Self::View, String>;

    /// Place `view` (all `len` bytes of it) at window offset `at`.
    ///
    /// # Errors
    /// The `mmap` refusal, by name.
    fn place_view(&self, at: u64, len: u64, view: &Self::View) -> Result<(), String>;

    /// Place the guest-RAM memfd's `[file_off, file_off+len)` at window offset `at`.
    ///
    /// # Errors
    /// No guest-RAM object, or the `mmap` refusal, by name.
    fn place_ram(&self, at: u64, len: u64, file_off: u64) -> Result<(), String>;

    /// Re-point `[at, at+len)` to the window's scratch.
    ///
    /// # Errors
    /// The `mmap` refusal, by name.
    fn sink(&self, at: u64, len: u64) -> Result<(), String>;

    /// Give a view's host aperture back.
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn release(&self, view: Self::View) -> Result<(), String>;

    /// Arm a view ON THE vCPU — the PRAMIN re-point's single RM call. Production uses a node
    /// opened ahead of time (so this is exactly one ioctl); the default is [`ViewOps::arm_store`].
    ///
    /// # Errors
    /// As [`ViewOps::arm_store`].
    fn arm_store_in_trap(&self, off: u64, len: u64) -> Result<Self::View, String> {
        self.arm_store(off, len)
    }

    /// Hand a view no longer placed anywhere to be released OFF the vCPU. The default releases
    /// inline (tests); production queues it to a reaper thread.
    fn retire(&self, view: Self::View) {
        let _ = self.release(view);
    }
}

/// One placement WE made in a [`CpuWindow`].
#[derive(Debug)]
struct Held<V> {
    len: u64,
    /// `None` for a guest-RAM placement (nothing to release).
    view: Option<V>,
    /// The store offset a view shows (`None` for guest RAM) — for the seed's retirement report.
    store_off: Option<u64>,
}

/// Counters for one window — never a decision input.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WindowStats {
    /// Store views armed and placed.
    pub views_placed: u64,
    /// Guest-RAM placements.
    pub ram_placed: u64,
    /// Placements re-pointed to scratch.
    pub sunk: u64,
    /// Views whose aperture was given back.
    pub released: u64,
    /// Store bytes currently shown through views.
    pub view_bytes: u64,
    /// Operations refused (first ones named by the VA manager's own refusal list).
    pub refused: u64,
}

/// ★★ **The boot display's seed** (`docs/design/V3_DISPLAY.md` §4.11.2): one view of store
/// `[store_off, store_off + len)` placed at window offset `at` BEFORE the guest's RM has mapped
/// anything — what a real card's BAR1 shows before RM switches it to virtual mode: a physical window
/// onto FB 0, where the firmware's framebuffer is.
///
/// ⊘ Kept OUTSIDE `placed`: it is not a mapping the guest's tables state, so the walk/diff ledger
/// never sees it. It retires at the end of the FIRST batch that changes anything in this window, at
/// any VA ([`CpuWindow::retire_seed`]) — the closest observable equivalent of BAR1 going virtual.
///
/// ★ 2026-10-03 (`v3-gop-unload`, box test B5): it is placed AGAIN when the guest's RM gives BAR1
/// back ([`CpuWindow::reseed`]) — a real card's RM returns BAR1 to PHYSICAL mode at teardown
/// (`kbusTeardownMailbox_GM107`, `ogkm-580: src/nvidia/src/kernel/gpu/bus/arch/maxwell/kern_bus_gm107.c:746-765`),
/// so the firmware console keeps drawing into FB 0 while no RM holds the GPU — and it retires
/// again exactly as the first one did when that RM (or the next) takes BAR1 back.
#[derive(Debug)]
struct Seed<V> {
    at: u64,
    store_off: u64,
    len: u64,
    view: V,
}

/// What retiring the seed found inside its range — the guest's first placements there.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SeedRetired {
    /// The seed's window range, `(at, len)`.
    pub range: (u64, u64),
    /// Every placement WE made that overlaps the range when it retired: `(va, len, store offset)`
    /// (`None` = guest RAM). Ascending by VA.
    pub inside: Vec<(u64, u64, Option<u64>)>,
    /// Bytes of the range no placement covered — re-pointed to scratch before the seed went.
    pub sunk: u64,
    /// The seed's release was refused by the host (the aperture leaked; counted).
    pub release_refused: bool,
    /// Which placement of the seed retired: 1 = the one made at realize, N > 1 = the (N-1)th
    /// re-seed ([`CpuWindow::reseed`]).
    pub life: u32,
}

impl SeedRetired {
    /// ★ The boot display's check (`V3_DISPLAY.md` §4.11.9, box test B2): CPU-RM maps a preserved
    /// console at VA 0 as ONE fixed range of 4 KiB PTEs (`kern_bus_gm107.c:1098`, `:1131-1155`), and
    /// the walker must coalesce it — one run, VA 0 → store 0, so ONE host view. `Some(len)` when
    /// exactly that is what the range holds; `None` otherwise (no console, or several runs: each run
    /// is a host BAR1 view of its own).
    #[must_use]
    pub fn console_is_one_run(&self) -> Option<u64> {
        match self.inside.as_slice() {
            [(0, len, Some(0))] => Some(*len),
            _ => None,
        }
    }

    /// The boot log's line for it.
    #[must_use]
    pub fn line(&self) -> String {
        let (at, len) = self.range;
        let verdict = match (self.console_is_one_run(), self.inside.len()) {
            (Some(n), _) => format!("the console is ONE run, VA 0 -> store 0, {n:#x} bytes"),
            (None, 0) => "no placement inside it (no console adopted)".to_string(),
            (None, k) => format!(
                "⚠ {k} placements inside it, first {:?} — the console is NOT one view",
                self.inside[0]
            ),
        };
        format!(
            "boot display seed [{at:#x}, +{len:#x}) retired at the first change: {verdict}; \
             {:#x} bytes re-pointed to scratch{}{}",
            self.sunk,
            if self.release_refused {
                ", the seed's release REFUSED (aperture leaked, counted)"
            } else {
                ""
            },
            if self.life > 1 {
                format!(
                    " (seed life {}: the guest's RM took BAR1 back after giving it up)",
                    self.life
                )
            } else {
                String::new()
            }
        )
    }
}

/// What one range of a [`CpuWindow`] shows now ([`CpuWindow::coverage`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Coverage {
    /// Every placement WE made that overlaps the range: `(va, len, store offset)` (`None` = guest
    /// RAM). Ascending by VA.
    pub inside: Vec<(u64, u64, Option<u64>)>,
    /// The parts of the range no placement covers, `(at, len)`, ascending — scratch, unless the
    /// seed is placed there.
    pub gaps: Vec<(u64, u64)>,
}

impl Coverage {
    /// Bytes of the range no placement covers.
    #[must_use]
    pub fn uncovered(&self) -> u64 {
        self.gaps.iter().map(|&(_, n)| n).sum()
    }
}

/// What [`CpuWindow::reseed`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reseed {
    /// No boot framebuffer was ever seeded in this window (`gop=off`, or BAR2): nothing to do.
    NoBootFramebuffer,
    /// The seed is still placed (the guest gave BAR1 up before ever changing it): nothing to do.
    AlreadyShown,
    /// The seed was placed again; this is its `life`-th placement.
    Placed {
        /// 2 for the first re-seed.
        life: u32,
    },
}

/// ★★★ **A guest BAR aperture as a [`MapTarget`]**: guest BAR VA `v` IS window offset `v`
/// (`[measured cap3 #159733/#159779]`: the BAR2 PTE at `0xfef000` maps VA 0 and the guest's
/// `kbusVerifyBar2` writes land at BAR3 offset 0; `kern_bus_gm107.c:431` asserts
/// `cpuVisibleBase == 0`).
pub struct CpuWindow<V: ViewOps> {
    ops: V,
    bytes: u64,
    placed: RefCell<BTreeMap<u64, Held<V::View>>>,
    stats: RefCell<WindowStats>,
    /// ★ The boot display's seed ([`Seed`]); `None` on every window but a `gop=on` BAR1.
    seed: RefCell<Option<Seed<V::View>>>,
    /// What the seed's LAST retirement found (`None` until it first retired).
    retired: RefCell<Option<SeedRetired>>,
    /// ★ The boot framebuffer's range `(at, store_off, len)`, set by [`CpuWindow::seed`] and kept
    /// for [`CpuWindow::reseed`]. `None`: this window never had a seed.
    boot: Cell<Option<(u64, u64, u64)>>,
    /// How many times the seed has been placed (1 after [`CpuWindow::seed`]).
    lives: Cell<u32>,
    /// Batches that changed anything ([`MapTarget::invalidate`] calls) — what a deferred re-seed
    /// compares to know that the guest took BAR1 back in between.
    changes: Cell<u64>,
}

impl<V: ViewOps> CpuWindow<V> {
    /// A window of `bytes` whose every page currently shows scratch.
    #[must_use]
    pub fn new(ops: V, bytes: u64) -> CpuWindow<V> {
        CpuWindow {
            ops,
            bytes,
            placed: RefCell::new(BTreeMap::new()),
            stats: RefCell::default(),
            seed: RefCell::new(None),
            retired: RefCell::new(None),
            boot: Cell::new(None),
            lives: Cell::new(0),
            changes: Cell::new(0),
        }
    }

    /// ★★ Place the boot display's seed ([`Seed`]): a view of store `[store_off, store_off + len)`
    /// at window offset `at`. Before the guest runs, on a window nothing has been placed in.
    ///
    /// # Errors
    /// By name: a second seed, a window that already holds placements, a range outside the
    /// aperture, or the host's refusal (nothing is left placed then).
    pub fn seed(&self, at: u64, store_off: u64, len: u64) -> Result<(), String> {
        if self.boot.get().is_some() {
            return Err("the window already had its seed".into());
        }
        if !self.placed.borrow().is_empty() {
            return Err("the window already holds placements; a seed is placed first".into());
        }
        if len == 0 || at.checked_add(len).is_none_or(|e| e > self.bytes) {
            return Err(format!(
                "seed {at:#x}+{len:#x}: outside the {:#x}-byte aperture",
                self.bytes
            ));
        }
        self.place_seed(at, store_off, len)?;
        self.boot.set(Some((at, store_off, len)));
        self.lives.set(1);
        Ok(())
    }

    /// Arm one view of store `[store_off, +len)` and place it at `at`; on a refused placement,
    /// scratch goes back and the aperture is returned (nothing stays placed).
    fn place_seed(&self, at: u64, store_off: u64, len: u64) -> Result<(), String> {
        let view = self
            .ops
            .arm_store(store_off, len)
            .map_err(|e| format!("seed {at:#x}+{len:#x} (store @{store_off:#x}): arm: {e}"))?;
        if let Err(e) = self.ops.place_view(at, len, &view) {
            let _ = self.ops.sink(at, len);
            let _ = self.ops.release(view);
            return Err(format!(
                "seed {at:#x}+{len:#x} (store @{store_off:#x}): place: {e}"
            ));
        }
        *self.seed.borrow_mut() = Some(Seed {
            at,
            store_off,
            len,
            view,
        });
        Ok(())
    }

    /// ★★ **BAR1 back to its PHYSICAL view of FB `[0, G)`** — the guest's RM gave BAR1 up (its
    /// teardown, `docs/design/V3_DISPLAY.md` §4.11.13): place the seed again over the boot
    /// framebuffer's range, so the firmware console (efifb, simpledrm, a GOP) keeps reaching FB 0
    /// as it does on a real card, whose RM returns BAR1 to physical mode at teardown
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/bus/arch/maxwell/kern_bus_gm107.c:746-765`). It retires
    /// like the first seed — at the first batch that changes anything — when an RM takes BAR1 again.
    ///
    /// ⊘ Only over a range the guest no longer maps: a placement of the guest's overlapping it is
    /// refused by name (the guest's tables still state that range; the seed would hide them and its
    /// retirement could not put them back). The range was scratch, so the view replaces scratch —
    /// no view is released here, and the scratch-first rule has nothing to order.
    ///
    /// # Errors
    /// By name: a guest placement in the range, or the host's refusal (scratch stays then).
    pub fn reseed(&self) -> Result<Reseed, String> {
        let Some((at, store_off, len)) = self.boot.get() else {
            return Ok(Reseed::NoBootFramebuffer);
        };
        if self.seed.borrow().is_some() {
            return Ok(Reseed::AlreadyShown);
        }
        let cov = self.coverage(at, len);
        if let Some(&(va, n, off)) = cov.inside.first() {
            self.stats.borrow_mut().refused += 1;
            return Err(format!(
                "boot framebuffer [{at:#x}, +{len:#x}): {} placement(s) of the guest's still overlap \
                 it (first {va:#x}+{n:#x} -> {off:x?}) — BAR1's physical view not restored",
                cov.inside.len()
            ));
        }
        if let Err(e) = self.place_seed(at, store_off, len) {
            self.stats.borrow_mut().refused += 1;
            return Err(e);
        }
        let life = self.lives.get().saturating_add(1);
        self.lives.set(life);
        Ok(Reseed::Placed { life })
    }

    /// The boot framebuffer's range `(at, store_off, len)`, once seeded (placed or retired).
    #[must_use]
    pub fn boot_range(&self) -> Option<(u64, u64, u64)> {
        self.boot.get()
    }

    /// Batches that changed anything in this window so far.
    #[must_use]
    pub fn changes(&self) -> u64 {
        self.changes.get()
    }

    /// ★ What `[at, at+len)` shows now: the placements WE made that overlap it, and the gaps no
    /// placement covers (scratch — or the seed, while it is placed).
    #[must_use]
    pub fn coverage(&self, at: u64, len: u64) -> Coverage {
        let end = at.saturating_add(len);
        let mut out = Coverage::default();
        let mut cursor = at;
        for (&va, h) in self.placed.borrow().iter() {
            let h_end = va.saturating_add(h.len);
            if h_end <= at || va >= end {
                continue;
            }
            out.inside.push((va, h.len, h.store_off));
            if va > cursor {
                out.gaps.push((cursor, va - cursor));
            }
            cursor = cursor.max(h_end);
        }
        if cursor < end {
            out.gaps.push((cursor, end - cursor));
        }
        out
    }

    /// The seed's `(at, store_off, len)` while it is placed.
    #[must_use]
    pub fn seeded(&self) -> Option<(u64, u64, u64)> {
        self.seed
            .borrow()
            .as_ref()
            .map(|s| (s.at, s.store_off, s.len))
    }

    /// What the seed's retirement found, once it retired.
    #[must_use]
    pub fn seed_retired(&self) -> Option<SeedRetired> {
        self.retired.borrow().clone()
    }

    /// ★★ **Retire the seed**: the guest's views are already placed (the batch's maps ran first);
    /// every part of the seed's range that no placement covers is re-pointed to scratch; only THEN
    /// is the seed's view released — the scratch-first rule of [`MapTarget::unmap`]. Called by
    /// [`MapTarget::invalidate`] at the end of the first batch that changed anything, and by the
    /// device when it stops (a guest that never loaded its driver). `Ok(None)` with no seed.
    ///
    /// # Errors
    /// A re-point to scratch the host refused: the seed STAYS (it still backs the gap), by name.
    pub fn retire_seed(&self) -> Result<Option<SeedRetired>, String> {
        let Some((at, len)) = self.seed.borrow().as_ref().map(|s| (s.at, s.len)) else {
            return Ok(None);
        };
        let Coverage { inside, gaps } = self.coverage(at, len);
        for &(g, n) in &gaps {
            if let Err(e) = self.ops.sink(g, n) {
                self.stats.borrow_mut().refused += 1;
                return Err(format!(
                    "seed {at:#x}+{len:#x}: re-pointing the uncovered {g:#x}+{n:#x} to scratch: {e} — \
                     the seed stays"
                ));
            }
        }
        let Some(seed) = self.seed.borrow_mut().take() else {
            return Ok(None);
        };
        let release_refused = self.ops.release(seed.view).is_err();
        if release_refused {
            self.stats.borrow_mut().refused += 1;
        }
        let report = SeedRetired {
            range: (at, len),
            inside,
            sunk: gaps.iter().map(|&(_, n)| n).sum(),
            release_refused,
            life: self.lives.get(),
        };
        *self.retired.borrow_mut() = Some(report.clone());
        Ok(Some(report))
    }

    /// The counters.
    #[must_use]
    pub fn stats(&self) -> WindowStats {
        self.stats.borrow().clone()
    }

    /// The window's host verbs.
    #[must_use]
    pub fn ops(&self) -> &V {
        &self.ops
    }

    fn refuse<T>(&self, why: String) -> Result<T, String> {
        self.stats.borrow_mut().refused += 1;
        Err(why)
    }
}

impl<V: ViewOps> MapTarget for CpuWindow<V> {
    fn map(&self, d: &Desired, _defer: bool) -> Result<Mapped, String> {
        let end = d.va.checked_add(d.len).filter(|&e| e <= self.bytes);
        if end.is_none() || d.len == 0 {
            return self.refuse(format!(
                "window map {:#x}+{:#x}: outside the {:#x}-byte aperture (the VA manager clips first)",
                d.va, d.len, self.bytes
            ));
        }
        if self.placed.borrow().contains_key(&d.va) {
            return self.refuse(format!(
                "window map {:#x}: a placement WE made is still there",
                d.va
            ));
        }
        let held = if d.ram {
            if let Err(e) = self.ops.place_ram(d.va, d.len, d.off) {
                return self.refuse(format!(
                    "window map {:#x}+{:#x} (guest RAM @{:#x}): {e}",
                    d.va, d.len, d.off
                ));
            }
            self.stats.borrow_mut().ram_placed += 1;
            Held {
                len: d.len,
                view: None,
                store_off: None,
            }
        } else {
            let view = match self.ops.arm_store(d.off, d.len) {
                Ok(v) => v,
                Err(e) => {
                    return self.refuse(format!(
                        "window map {:#x}+{:#x} (store @{:#x}): arm: {e}",
                        d.va, d.len, d.off
                    ));
                }
            };
            if let Err(e) = self.ops.place_view(d.va, d.len, &view) {
                // Nothing was placed: give the aperture straight back, and put scratch back in
                // case the failed MAP_FIXED left the range in any other state.
                let _ = self.ops.sink(d.va, d.len);
                let _ = self.ops.release(view);
                return self.refuse(format!(
                    "window map {:#x}+{:#x} (store @{:#x}): place: {e}",
                    d.va, d.len, d.off
                ));
            }
            let mut s = self.stats.borrow_mut();
            s.views_placed += 1;
            s.view_bytes += d.len;
            Held {
                len: d.len,
                view: Some(view),
                store_off: Some(d.off),
            }
        };
        self.placed.borrow_mut().insert(d.va, held);
        Ok(Mapped::Placed)
    }

    fn unmap(&self, va: u64, _defer: bool) -> Result<(), String> {
        let Some(len) = self.placed.borrow().get(&va).map(|h| h.len) else {
            return self.refuse(format!("window unmap {va:#x}: no placement of ours there"));
        };
        // ★ Scratch FIRST: from this instant the guest can no longer reach the view, so its
        // aperture may be released.
        if let Err(e) = self.ops.sink(va, len) {
            return self.refuse(format!(
                "window unmap {va:#x}+{len:#x}: re-point to scratch: {e}"
            ));
        }
        let held = self.placed.borrow_mut().remove(&va);
        let mut s = self.stats.borrow_mut();
        s.sunk += 1;
        if let Some(Held {
            view: Some(v), len, ..
        }) = held
        {
            s.view_bytes = s.view_bytes.saturating_sub(len);
            drop(s);
            match self.ops.release(v) {
                Ok(()) => self.stats.borrow_mut().released += 1,
                // ★ The guest can no longer see it (scratch is in place), so the UNMAP happened
                // and is acknowledged APPLIED; only the aperture is leaked, and that is counted.
                // ⊘ Answering FAILED would keep a placement the window no longer holds, and every
                // later diff would re-emit an unmap nothing can satisfy.
                Err(e) => {
                    self.stats.borrow_mut().refused += 1;
                    eprintln!(
                        "kf3: window unmap {va:#x}: view release refused ({e}) — aperture leaked, counted"
                    );
                }
            }
        }
        Ok(())
    }

    /// A CPU view is coherent the moment it is placed: nothing to invalidate.
    ///
    /// ★ The boot display's seed retires HERE ([`CpuWindow::retire_seed`]): the apply calls this
    /// once per batch, after the batch's maps, and only when something changed
    /// (`crate::apply`) — so the first change at any VA retires it, with the guest's new views
    /// already in place. Without a seed this is what it always was.
    fn invalidate(&self) -> Result<(), String> {
        self.changes.set(self.changes.get().wrapping_add(1));
        if let Some(r) = self.retire_seed()? {
            eprintln!("kf3: {}", r.line());
        }
        Ok(())
    }

    fn va_extent(&self) -> Option<u64> {
        Some(self.bytes)
    }
}

/// One store view the PRAMIN window shows, or may still show: the slots it was placed over, the
/// re-point that placed it, and the armed view that holds its host aperture.
#[derive(Debug)]
struct LiveView<V> {
    /// First slot, and how many: the window range `[first * granule, (first + n) * granule)`.
    first: usize,
    n: usize,
    /// The generation (re-point number, from 1) whose placement put it there.
    placed_gen: u64,
    /// Whether it was already counted in [`PraminPool::kept`]: a view is counted ONCE, however
    /// many re-points keep it.
    counted: bool,
    view: V,
}

/// ★ Whether every slot of `[first, first + n)` was landed over (re-placed successfully) by a
/// re-point AFTER generation `placed_gen`. `landed[s]` is the last generation that landed slot `s`
/// (0 = never). Then nothing shows the view placed in `placed_gen` any more: each later landing
/// replaced it on its slot, and a view is never placed twice, so it cannot come back.
fn landed_since(landed: &[u64], first: usize, n: usize, placed_gen: u64) -> bool {
    (first..first.saturating_add(n)).all(|s| landed.get(s).is_some_and(|&g| g > placed_gen))
}

/// The re-point state, under the pool's one mutex.
#[derive(Debug)]
struct PraminState<V> {
    /// Views that may still be shown somewhere in the window.
    placed: Vec<LiveView<V>>,
    /// The current re-point's generation (the first re-point is 1).
    now: u64,
    /// Per slot, the last generation whose placement over it landed (0 = never).
    landed: Vec<u64>,
}

/// What one PRAMIN re-point did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Repointed {
    /// Slots now showing the store.
    pub views: u32,
    /// Slots now showing guest RAM.
    pub ram: u32,
    /// ★ Slots showing scratch because nothing addressable backs them — a named failure.
    pub missed: u32,
    /// Placements (or maps) the kernel refused.
    pub refused: u32,
    /// Host maps (`NV_ESC_RM_MAP_MEMORY`) made on the vCPU — 1 for a store window.
    pub maps: u32,
    /// `mmap` placements made on the vCPU — 1 for a window that is one run.
    pub mmaps: u32,
    /// ★ Previously placed views still held after this re-point (a gauge of VIEWS, not of events):
    /// some slot each was placed over has not been re-placed since, because a placement or sink
    /// there was refused, so the guest may still reach it. Retired by the re-point that lands its
    /// last such slot (2026-10-03). At most one per slot, since a slot's latest landing holds at
    /// most one view.
    pub kept: u32,
}

/// Where a PRAMIN slot must point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotSource {
    /// Store offset (vidmem target).
    Store(u64),
    /// Guest-physical address (sysmem target).
    Ram(u64),
    /// Nothing addressable (a reserved target, an overflowing base).
    Nothing,
}

/// One contiguous run of slots with one source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Run {
    Store { first: usize, n: usize, off: u64 },
    Ram { first: usize, n: usize, gpa: u64 },
    Nothing { first: usize, n: usize },
}

/// Coalesce slots into maximal runs: consecutive slots whose sources continue each other.
fn runs(slots: &[SlotSource], granule: u64) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    for (i, src) in slots.iter().enumerate() {
        let extended = match (out.last_mut(), *src) {
            (Some(Run::Store { n, off, .. }), SlotSource::Store(o))
                if *off + *n as u64 * granule == o =>
            {
                *n += 1;
                true
            }
            (Some(Run::Ram { n, gpa, .. }), SlotSource::Ram(g))
                if *gpa + *n as u64 * granule == g =>
            {
                *n += 1;
                true
            }
            (Some(Run::Nothing { n, .. }), SlotSource::Nothing) => {
                *n += 1;
                true
            }
            _ => false,
        };
        if !extended {
            out.push(match *src {
                SlotSource::Store(off) => Run::Store {
                    first: i,
                    n: 1,
                    off,
                },
                SlotSource::Ram(gpa) => Run::Ram {
                    first: i,
                    n: 1,
                    gpa,
                },
                SlotSource::Nothing => Run::Nothing { first: i, n: 1 },
            });
        }
    }
    out
}

/// ★★★ **The PRAMIN window: one host map + one `mmap` per window move** (owner ruling
/// 2026-09-25; see the module doc).
///
/// ⊘ Re-points are serialised by one mutex, held across the map and the placement: two vCPUs
/// racing on the window-base register must not retire a view the other just placed. The lock
/// guards only this operation (the sanctioned synchronous one); nothing else ever takes it.
pub struct PraminPool<V: ViewOps> {
    ops: V,
    granule: u64,
    ram_offset: Box<dyn Fn(u64, u64) -> Option<u64> + Send + Sync>,
    state: std::sync::Mutex<PraminState<V::View>>,
    /// Re-points done.
    pub repoints: AtomicU64,
    /// ★ Distinct views ever kept past a re-point because a slot they were placed over was not
    /// re-placed (see [`Repointed::kept`]). Each view counts ONCE, however many re-points keep it
    /// (2026-10-03; it counted once per re-point before).
    pub kept: AtomicU64,
    /// Views kept right now ([`Repointed::kept`] of the latest re-point): their host apertures are
    /// still held. At most one per slot.
    pub kept_now: AtomicU64,
    /// Slots that showed scratch because nothing addressable backed them.
    pub missed: AtomicU64,
    /// Placements the kernel refused.
    pub refused: AtomicU64,
    /// Host maps made in the trap.
    pub maps: AtomicU64,
    /// `mmap`s made in the trap.
    pub mmaps: AtomicU64,
    /// Worst re-point wall time, ns (measured by the caller, stored here).
    pub worst_ns: AtomicU64,
    /// Worst single host map (the ioctl) in the trap, ns.
    pub worst_map_ns: AtomicU64,
    /// Worst single `mmap` placement in the trap, ns.
    pub worst_mmap_ns: AtomicU64,
}

fn elapsed_ns(t: std::time::Instant) -> u64 {
    u64::try_from(t.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

impl<V: ViewOps> PraminPool<V> {
    /// A pool over `ops`. `ram_offset(gpa, len)` maps a sysmem target to the guest-RAM memfd.
    #[must_use]
    pub fn new(
        ops: V,
        granule: u64,
        ram_offset: Box<dyn Fn(u64, u64) -> Option<u64> + Send + Sync>,
    ) -> Self {
        PraminPool {
            ops,
            granule,
            ram_offset,
            state: std::sync::Mutex::new(PraminState {
                placed: Vec::new(),
                now: 0,
                landed: Vec::new(),
            }),
            repoints: AtomicU64::new(0),
            kept: AtomicU64::new(0),
            kept_now: AtomicU64::new(0),
            missed: AtomicU64::new(0),
            refused: AtomicU64::new(0),
            maps: AtomicU64::new(0),
            mmaps: AtomicU64::new(0),
            worst_ns: AtomicU64::new(0),
            worst_map_ns: AtomicU64::new(0),
            worst_mmap_ns: AtomicU64::new(0),
        }
    }

    /// ★ The vCPU path: point the window at `slots` (slot `i` = window offset `i * granule`).
    /// One map + one `mmap` per run; the replaced views are retired off the vCPU.
    ///
    /// ⊘ **Corrected 2026-10-03: a replaced view is retired ONLY when every slot it was placed
    /// over has been re-placed successfully since it was placed** — by this re-point or by earlier
    /// ones (each slot remembers the last generation that landed it; [`landed_since`]). Requiring
    /// ONE re-point to land its whole range, as this did first, held for the VM's life a view whose
    /// slots were landed over piecewise (refusals alternating between halves of the window). Until
    /// then this retired every
    /// old view unconditionally, on the comment *"every old view was covered by a MAP_FIXED
    /// placement above"*, which is false when a run's placement or sink is refused. A refused
    /// `MAP_FIXED` either leaves the old mapping in place (a refusal before the old page tables are
    /// cleared, e.g. `ENOMEM` once `vm.max_map_count` is reached: Linux 7.1 `mm/vma.c:2439-2444`,
    /// `:1416-1418`) or leaves a hole; in the first case the guest still reaches the old view. Retiring it then hands its
    /// host BAR1 aperture back to RM while the guest can still map it, and nvidia.ko does not zap
    /// user mappings when RM unmaps (its only revocation is `nv_revoke_gpu_mappings_locked`, on
    /// power-management paths: `ogkm-580: kernel-open/nvidia/nv-mmap.c:786-816`). So such a view
    /// is kept (counted in [`Repointed::kept`]) and retired by the re-point that lands its last
    /// slot still showing it. Kept views are bounded by the slots: a slot shows at most the one view
    /// its latest landing placed. [`CpuWindow::unmap`] already had this rule (scratch FIRST, release
    /// only after it lands).
    ///
    /// A NEW view whose own placement was refused is retired at once: a refused `MAP_FIXED` never
    /// leaves the new mapping behind (a driver's partial mapping is undone, `mm/vma.c:2496-2506`).
    pub fn repoint(&self, slots: &[SlotSource]) -> Repointed {
        let mut out = Repointed::default();
        let g = self.granule;
        let Ok(mut st) = self.state.lock() else {
            out.refused += 1;
            return out;
        };
        st.now += 1;
        let now = st.now;
        if st.landed.len() < slots.len() {
            st.landed.resize(slots.len(), 0);
        }
        let mut fresh: Vec<LiveView<V::View>> = Vec::new();
        let mut doomed: Vec<V::View> = Vec::new();
        for run in runs(slots, g) {
            let (first, n) = match run {
                Run::Store { first, n, .. }
                | Run::Ram { first, n, .. }
                | Run::Nothing { first, n } => (first, n),
            };
            let (at, len) = (first as u64 * g, n as u64 * g);
            let n32 = u32::try_from(n).unwrap_or(u32::MAX);
            // `placed_ok`: whether the window range now shows what this run put there.
            let (placed_ok, r) = match run {
                Run::Store { off, .. } => {
                    out.maps += 1;
                    let t = std::time::Instant::now();
                    let armed = self.ops.arm_store_in_trap(off, len);
                    self.worst_map_ns
                        .fetch_max(elapsed_ns(t), Ordering::Relaxed);
                    match armed {
                        Ok(v) => {
                            out.mmaps += 1;
                            let t = std::time::Instant::now();
                            let r = self.ops.place_view(at, len, &v);
                            self.worst_mmap_ns
                                .fetch_max(elapsed_ns(t), Ordering::Relaxed);
                            if r.is_ok() {
                                out.views += n32;
                                fresh.push(LiveView {
                                    first,
                                    n,
                                    placed_gen: now,
                                    counted: false,
                                    view: v,
                                });
                            } else {
                                doomed.push(v);
                            }
                            (r.is_ok(), r)
                        }
                        Err(e) => {
                            out.missed += n32;
                            out.mmaps += 1;
                            let sunk = self.ops.sink(at, len);
                            (sunk.is_ok(), sunk.and(Err(e)))
                        }
                    }
                }
                Run::Ram { gpa, .. } => {
                    out.mmaps += 1;
                    let r = match (self.ram_offset)(gpa, len) {
                        Some(foff) => self.ops.place_ram(at, len, foff).map(|()| out.ram += n32),
                        None => {
                            out.missed += n32;
                            self.ops.sink(at, len)
                        }
                    };
                    (r.is_ok(), r)
                }
                Run::Nothing { .. } => {
                    out.missed += n32;
                    out.mmaps += 1;
                    let r = self.ops.sink(at, len);
                    (r.is_ok(), r)
                }
            };
            if placed_ok {
                for s in &mut st.landed[first..first + n] {
                    *s = now;
                }
            }
            if r.is_err() {
                out.refused += 1;
            }
        }
        let mut newly_kept = 0u64;
        for mut old in std::mem::take(&mut st.placed) {
            if landed_since(&st.landed, old.first, old.n, old.placed_gen) {
                // Every slot it was placed over shows something placed later: unreachable.
                doomed.push(old.view);
            } else {
                out.kept += 1;
                if !old.counted {
                    old.counted = true;
                    newly_kept += 1;
                }
                fresh.push(old);
            }
        }
        st.placed = fresh;
        // Under the lock, so the gauge is the latest re-point's even when two race.
        self.kept_now.store(u64::from(out.kept), Ordering::Relaxed);
        drop(st);
        for v in doomed {
            self.ops.retire(v);
        }
        self.repoints.fetch_add(1, Ordering::Relaxed);
        self.missed
            .fetch_add(u64::from(out.missed), Ordering::Relaxed);
        self.refused
            .fetch_add(u64::from(out.refused), Ordering::Relaxed);
        self.kept.fetch_add(newly_kept, Ordering::Relaxed);
        self.maps.fetch_add(u64::from(out.maps), Ordering::Relaxed);
        self.mmaps
            .fetch_add(u64::from(out.mmaps), Ordering::Relaxed);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicBool;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Op {
        Arm(u64, u64),
        View(u64, u64, u64),
        Ram(u64, u64, u64),
        Sink(u64, u64),
        Release(u64),
    }

    /// Records every host verb. A view is its store offset.
    #[derive(Default)]
    struct Rec {
        ops: Mutex<Vec<Op>>,
        refuse_arm_at: Option<u64>,
        refuse_place: AtomicBool,
        refuse_sink: AtomicBool,
        /// Refuse every arm while set (flipped by a test mid-way).
        refuse_arms: AtomicBool,
    }

    impl Rec {
        fn refusing_place() -> Rec {
            Rec {
                refuse_place: AtomicBool::new(true),
                ..Rec::default()
            }
        }
        fn releases(&self) -> Vec<u64> {
            self.ops
                .lock()
                .unwrap()
                .iter()
                .filter_map(|o| match o {
                    Op::Release(v) => Some(*v),
                    _ => None,
                })
                .collect()
        }
    }

    impl ViewOps for &Rec {
        type View = u64;
        fn arm_store(&self, off: u64, len: u64) -> Result<u64, String> {
            if self.refuse_arm_at == Some(off) || self.refuse_arms.load(Ordering::Relaxed) {
                return Err("NV_ERR_NO_MEMORY (fake)".into());
            }
            self.ops.lock().unwrap().push(Op::Arm(off, len));
            Ok(off)
        }
        fn place_view(&self, at: u64, len: u64, v: &u64) -> Result<(), String> {
            if self.refuse_place.load(Ordering::Relaxed) {
                return Err("ENOMEM (fake)".into());
            }
            self.ops.lock().unwrap().push(Op::View(at, len, *v));
            Ok(())
        }
        fn place_ram(&self, at: u64, len: u64, file_off: u64) -> Result<(), String> {
            self.ops.lock().unwrap().push(Op::Ram(at, len, file_off));
            Ok(())
        }
        fn sink(&self, at: u64, len: u64) -> Result<(), String> {
            if self.refuse_sink.load(Ordering::Relaxed) {
                return Err("ENOMEM (fake sink)".into());
            }
            self.ops.lock().unwrap().push(Op::Sink(at, len));
            Ok(())
        }
        fn release(&self, v: u64) -> Result<(), String> {
            self.ops.lock().unwrap().push(Op::Release(v));
            Ok(())
        }
    }

    fn d(va: u64, off: u64, len: u64, ram: bool) -> Desired {
        Desired {
            va,
            len,
            off,
            ram,
            kind: 0,
            perm: kf_host::MapPerm::READ_WRITE,
            leaf: 0,
        }
    }

    #[test]
    fn a_map_arms_exactly_the_run_and_places_it_at_its_va() {
        let r = Rec::default();
        let w = CpuWindow::new(&r, 32 << 20);
        w.map(&d(0, 0x2_EFBA_E000, 0x1000, false), true).unwrap();
        assert_eq!(
            *r.ops.lock().unwrap(),
            vec![
                Op::Arm(0x2_EFBA_E000, 0x1000),
                Op::View(0, 0x1000, 0x2_EFBA_E000)
            ]
        );
        assert_eq!(w.stats().view_bytes, 0x1000);
    }

    #[test]
    fn an_unmap_sinks_first_then_releases_the_aperture() {
        let r = Rec::default();
        let w = CpuWindow::new(&r, 32 << 20);
        w.map(&d(0xFEF000, 0x1000_0000, 0x1000, false), true)
            .unwrap();
        r.ops.lock().unwrap().clear();
        w.unmap(0xFEF000, true).unwrap();
        assert_eq!(
            *r.ops.lock().unwrap(),
            vec![Op::Sink(0xFEF000, 0x1000), Op::Release(0x1000_0000)]
        );
        let s = w.stats();
        assert_eq!((s.sunk, s.released, s.view_bytes), (1, 1, 0));
    }

    #[test]
    fn a_guest_ram_row_is_placed_from_the_memfd_and_released_as_nothing() {
        let r = Rec::default();
        let w = CpuWindow::new(&r, 32 << 20);
        w.map(&d(0x2000, 0x4000_0000, 0x1000, true), true).unwrap();
        w.unmap(0x2000, true).unwrap();
        assert_eq!(
            *r.ops.lock().unwrap(),
            vec![
                Op::Ram(0x2000, 0x1000, 0x4000_0000),
                Op::Sink(0x2000, 0x1000)
            ]
        );
    }

    #[test]
    fn refusals_are_named_and_leave_nothing_placed() {
        let r = Rec {
            refuse_arm_at: Some(0x5000),
            ..Rec::default()
        };
        let w = CpuWindow::new(&r, 0x10_0000);
        assert!(
            w.map(&d(0x0F_F000, 0, 0x2000, false), true)
                .unwrap_err()
                .contains("outside")
        );
        assert!(
            w.map(&d(0, 0x5000, 0x1000, false), true)
                .unwrap_err()
                .contains("NV_ERR_NO_MEMORY")
        );
        assert!(w.unmap(0, true).unwrap_err().contains("no placement"));
        assert_eq!(w.stats().refused, 3);
        let r2 = Rec::refusing_place();
        let w2 = CpuWindow::new(&r2, 0x10_0000);
        assert!(w2.map(&d(0, 0x7000, 0x1000, false), true).is_err());
        assert_eq!(
            *r2.ops.lock().unwrap(),
            vec![
                Op::Arm(0x7000, 0x1000),
                Op::Sink(0, 0x1000),
                Op::Release(0x7000)
            ],
            "a failed place gives the aperture straight back"
        );
    }

    #[test]
    fn leaves_above_the_window_are_clipped_not_refused() {
        let (kept, cut) = crate::ledger::clip_leaves(
            &[
                (0, 0x10, 0x1000, 0),
                (0x1F_F000, 0x20, 0x2000, 0),
                (0x200_0000, 0x30, 0x1000, 2),
            ],
            0x20_0000,
        );
        assert_eq!(
            kept,
            vec![(0, 0x10, 0x1000, 0), (0x1F_F000, 0x20, 0x1000, 0)]
        );
        assert_eq!(cut, 0x2000);
    }

    #[test]
    fn the_window_is_its_own_extent() {
        let r = Rec::default();
        assert_eq!(CpuWindow::new(&r, 32 << 20).va_extent(), Some(32 << 20));
    }

    #[test]
    fn a_store_window_is_one_map_and_one_mmap() {
        let r = Rec::default();
        let g = 0x1_0000;
        let pool = PraminPool::new(&r, g, Box::new(|gpa, _| Some(gpa + 7)));
        let slots: Vec<_> = (0..16)
            .map(|i| SlotSource::Store(0x2_0000_0000 + i * g))
            .collect();
        let out = pool.repoint(&slots);
        assert_eq!(
            out,
            Repointed {
                views: 16,
                ram: 0,
                missed: 0,
                refused: 0,
                maps: 1,
                mmaps: 1,
                kept: 0
            }
        );
        assert_eq!(
            *r.ops.lock().unwrap(),
            vec![
                Op::Arm(0x2_0000_0000, 16 * g),
                Op::View(0, 16 * g, 0x2_0000_0000)
            ]
        );
    }

    #[test]
    fn a_move_retires_the_previous_view_after_placing_the_new_one() {
        let r = Rec::default();
        let g = 0x1_0000;
        let pool = PraminPool::new(&r, g, Box::new(|_, _| None));
        let at = |base: u64| {
            (0..16)
                .map(|i| SlotSource::Store(base + i * g))
                .collect::<Vec<_>>()
        };
        pool.repoint(&at(0x10_0000));
        r.ops.lock().unwrap().clear();
        pool.repoint(&at(0x40_0000));
        assert_eq!(
            *r.ops.lock().unwrap(),
            vec![
                Op::Arm(0x40_0000, 16 * g),
                Op::View(0, 16 * g, 0x40_0000),
                Op::Release(0x10_0000)
            ]
        );
    }

    #[test]
    fn a_ram_window_is_one_mmap_and_no_map() {
        let r = Rec::default();
        let g = 0x1_0000;
        let pool = PraminPool::new(&r, g, Box::new(|gpa, _| Some(gpa + 7)));
        let slots: Vec<_> = (0..16)
            .map(|i| SlotSource::Ram(0x9000_0000 + i * g))
            .collect();
        let out = pool.repoint(&slots);
        assert_eq!((out.maps, out.mmaps, out.ram), (0, 1, 16));
        assert_eq!(
            *r.ops.lock().unwrap(),
            vec![Op::Ram(0, 16 * g, 0x9000_0007)]
        );
    }

    #[test]
    fn mixed_slots_coalesce_into_runs_and_name_the_misses() {
        let r = Rec::default();
        let g = 0x1_0000;
        let pool = PraminPool::new(&r, g, Box::new(|gpa, _| Some(gpa + 7)));
        let out = pool.repoint(&[
            SlotSource::Store(0x11_0000),
            SlotSource::Store(0x12_0000),
            SlotSource::Ram(0x9000_0000),
            SlotSource::Nothing,
        ]);
        assert_eq!(
            out,
            Repointed {
                views: 2,
                ram: 1,
                missed: 1,
                refused: 0,
                maps: 1,
                mmaps: 3,
                kept: 0
            }
        );
        assert_eq!(
            *r.ops.lock().unwrap(),
            vec![
                Op::Arm(0x11_0000, 2 * g),
                Op::View(0, 2 * g, 0x11_0000),
                Op::Ram(2 * g, g, 0x9000_0007),
                Op::Sink(3 * g, g)
            ]
        );
    }

    #[test]
    fn a_refused_map_shows_scratch_and_is_counted() {
        let r = Rec {
            refuse_arm_at: Some(0x2_0000),
            ..Rec::default()
        };
        let g = 0x1_0000;
        let pool = PraminPool::new(&r, g, Box::new(|_, _| None));
        let out = pool.repoint(&[SlotSource::Store(0x2_0000), SlotSource::Store(0x3_0000)]);
        assert_eq!((out.missed, out.refused, out.maps), (2, 1, 1));
        assert_eq!(*r.ops.lock().unwrap(), vec![Op::Sink(0, 2 * g)]);
    }

    // ── ★★ the boot display's seed (`docs/design/V3_DISPLAY.md` §4.11.2) ─────────────────────

    /// The 1080p boot framebuffer G, and a BAR1 aperture.
    const G: u64 = 0x7F_0000;
    const BAR1: u64 = 256 << 20;

    fn vid(va: u64, at: u64, len: u64) -> crate::apply::DiffRun {
        crate::apply::DiffRun {
            unmap: false,
            va,
            len,
            at,
            ap: crate::ledger::AP_VIDMEM,
            held: false,
            kind: 0,
            perm: kf_host::MapPerm::READ_WRITE,
            privileged: false,
            leaf: 0,
        }
    }

    fn apply(w: &CpuWindow<&Rec>, runs: &[crate::apply::DiffRun]) -> crate::apply::Applied {
        crate::apply::apply_entry(
            w,
            runs,
            &crate::apply::ApplyCfg {
                store_bytes: 8 << 30,
                grain: 0x1000,
                ram_offset: &|gpa, _| Some(gpa),
                usermode: None,
                per_map_kind: true,
                carve: 8 << 30,
                carve_refuse: false,
            },
        )
    }

    fn seeded(r: &Rec) -> CpuWindow<&Rec> {
        let w = CpuWindow::new(r, BAR1);
        w.seed(0, 0, G).expect("seeded");
        assert_eq!(
            std::mem::take(&mut *r.ops.lock().unwrap()),
            vec![Op::Arm(0, G), Op::View(0, G, 0)],
            "one view of store [0, G) at BAR1 offset 0"
        );
        w
    }

    /// ★ RM preserved the console: its first BAR1 map is FB 0 → VA 0. The guest's view is placed
    /// first, the uncovered rest of [0, G) sinks, and only then does the seed go.
    #[test]
    fn the_console_map_retires_the_seed_place_then_sink_then_release() {
        let r = Rec::default();
        let w = seeded(&r);
        let c = 0x7E_0000;
        let a = apply(&w, &[vid(0, 0, c)]);
        assert!(a.invalidated);
        assert_eq!(
            *r.ops.lock().unwrap(),
            vec![
                Op::Arm(0, c),
                Op::View(0, c, 0),
                Op::Sink(c, G - c),
                Op::Release(0)
            ]
        );
        assert_eq!(w.seeded(), None);
        let s = w.seed_retired().expect("retired");
        assert_eq!(s.console_is_one_run(), Some(c));
        assert_eq!(s.sunk, G - c);
        assert!(s.line().contains("ONE run"), "{}", s.line());
        // a later batch is an ordinary one
        r.ops.lock().unwrap().clear();
        apply(&w, &[vid(0x100_0000, 0x200_0000, 0x1000)]);
        assert_eq!(
            *r.ops.lock().unwrap(),
            vec![
                Op::Arm(0x200_0000, 0x1000),
                Op::View(0x100_0000, 0x1000, 0x200_0000)
            ]
        );
    }

    /// ★ The first BAR1 change lands ELSEWHERE (no console adopted, a REVERSE VAS): the seed still
    /// retires — FB [0, G) must not stay visible where RM thinks its heap is free.
    #[test]
    fn a_first_change_outside_the_seed_retires_it_too() {
        let r = Rec::default();
        let w = seeded(&r);
        apply(&w, &[vid(BAR1 - 0x1000, 0x300_0000, 0x1000)]);
        assert_eq!(
            *r.ops.lock().unwrap(),
            vec![
                Op::Arm(0x300_0000, 0x1000),
                Op::View(BAR1 - 0x1000, 0x1000, 0x300_0000),
                Op::Sink(0, G),
                Op::Release(0)
            ]
        );
        let s = w.seed_retired().unwrap();
        assert_eq!(
            (s.inside.len(), s.sunk, s.console_is_one_run()),
            (0, G, None)
        );
    }

    /// A console the walker hands over in several runs leaves no scratch gap between them, and the
    /// report says it is not one view.
    #[test]
    fn a_multi_run_console_shows_no_gap_and_is_named() {
        let r = Rec::default();
        let w = seeded(&r);
        apply(&w, &[vid(0, 0, 0x1000), vid(0x1000, 0x1000, G - 0x1000)]);
        assert_eq!(
            *r.ops.lock().unwrap(),
            vec![
                Op::Arm(0, 0x1000),
                Op::View(0, 0x1000, 0),
                Op::Arm(0x1000, G - 0x1000),
                Op::View(0x1000, G - 0x1000, 0x1000),
                Op::Release(0)
            ],
            "no sink: the two runs cover [0, G) together"
        );
        let s = w.seed_retired().unwrap();
        assert_eq!(s.inside.len(), 2);
        assert_eq!(s.console_is_one_run(), None);
        assert!(s.line().contains("NOT one view"), "{}", s.line());
    }

    /// No change, no retirement: an empty batch and a batch whose every map was refused leave the
    /// seed showing FB [0, G).
    #[test]
    fn a_batch_that_changes_nothing_keeps_the_seed() {
        let r = Rec {
            refuse_arm_at: Some(0x10_0000),
            ..Rec::default()
        };
        let w = seeded(&r);
        let a = apply(&w, &[]);
        assert!(!a.invalidated);
        let a = apply(&w, &[vid(0x2000, 0x10_0000, 0x1000)]);
        assert_eq!((a.mapped, a.invalidated), (0, false));
        assert_eq!(w.seeded(), Some((0, 0, G)));
        assert!(r.ops.lock().unwrap().is_empty());
    }

    /// A refused re-point to scratch keeps the seed (it still backs the gap), by name.
    #[test]
    fn a_refused_sink_keeps_the_seed() {
        let r = Rec {
            refuse_sink: AtomicBool::new(true),
            ..Rec::default()
        };
        let w = seeded(&r);
        let e = w.retire_seed().expect_err("refused");
        assert!(e.contains("the seed stays"), "{e}");
        assert_eq!(w.seeded(), Some((0, 0, G)));
        assert!(!r.ops.lock().unwrap().contains(&Op::Release(0)));
    }

    /// ⊘ The default: a window without a seed invalidates exactly as before (no host verb at all).
    #[test]
    fn without_a_seed_invalidate_is_a_no_op() {
        let r = Rec::default();
        let w = CpuWindow::new(&r, BAR1);
        w.invalidate().unwrap();
        assert_eq!(w.retire_seed(), Ok(None));
        assert!(r.ops.lock().unwrap().is_empty());
        assert_eq!(w.seed_retired(), None);
    }

    #[test]
    fn a_seed_is_refused_by_name_where_it_cannot_be_placed() {
        let r = Rec::default();
        let w = CpuWindow::new(&r, BAR1);
        assert!(w.seed(BAR1 - 0x1000, 0, G).unwrap_err().contains("outside"));
        assert!(w.seed(0, 0, 0).unwrap_err().contains("outside"));
        w.map(&d(0x10_0000, 0x10_0000, 0x1000, false), true)
            .unwrap();
        assert!(w.seed(0, 0, G).unwrap_err().contains("placements"));
        let r2 = Rec::default();
        let w2 = seeded(&r2);
        assert!(w2.seed(0, 0, G).unwrap_err().contains("already"));
    }

    // ── ★★ BAR1 back to physical mode: the re-seed (`V3_DISPLAY.md` §4.11.13, box test B5) ──────

    fn unvid(va: u64, len: u64) -> crate::apply::DiffRun {
        crate::apply::DiffRun {
            unmap: true,
            ..vid(va, 0, len)
        }
    }

    /// One RM life: the console mapped at VA 0 (the seed retires), then unmapped at teardown.
    fn after_one_life(r: &Rec) -> CpuWindow<&Rec> {
        let w = seeded(r);
        apply(&w, &[vid(0, 0, G)]);
        apply(&w, &[unvid(0, G)]);
        assert_eq!(w.seeded(), None);
        assert_eq!(
            w.coverage(0, G).uncovered(),
            G,
            "torn down: all of [0, G) is scratch"
        );
        r.ops.lock().unwrap().clear();
        w
    }

    /// ★ The guest's RM gave BAR1 up: ONE view of store [0, G) at BAR1 0 again — and when the next
    /// RM life maps its console, that seed retires exactly as the first one did (place, sink,
    /// release), naming its life.
    #[test]
    fn a_reseed_shows_fb0_again_and_the_next_life_retires_it() {
        let r = Rec::default();
        let w = after_one_life(&r);
        assert_eq!(w.reseed(), Ok(Reseed::Placed { life: 2 }));
        assert_eq!(
            std::mem::take(&mut *r.ops.lock().unwrap()),
            vec![Op::Arm(0, G), Op::View(0, G, 0)],
            "one view of store [0, G), replacing scratch: nothing released"
        );
        assert_eq!(w.seeded(), Some((0, 0, G)));
        let c = 0x7E_0000;
        apply(&w, &[vid(0, 0, c)]);
        assert_eq!(
            *r.ops.lock().unwrap(),
            vec![
                Op::Arm(0, c),
                Op::View(0, c, 0),
                Op::Sink(c, G - c),
                Op::Release(0)
            ],
            "the walker's placements win, scratch-first"
        );
        let s = w.seed_retired().expect("retired");
        assert_eq!((s.life, s.console_is_one_run()), (2, Some(c)));
        assert!(s.line().contains("seed life 2"), "{}", s.line());
    }

    /// ⊘ A guest placement still inside [0, G) refuses the re-seed by name: the seed would hide a
    /// mapping the guest's tables state. Nothing is placed, and a later re-seed (once the guest
    /// unmapped it) works.
    #[test]
    fn a_reseed_over_a_live_guest_mapping_is_refused_by_name() {
        let r = Rec::default();
        let w = seeded(&r);
        apply(&w, &[vid(0x10_0000, 0x900_0000, 0x1000)]);
        r.ops.lock().unwrap().clear();
        let e = w.reseed().expect_err("refused");
        assert!(e.contains("still overlap") && e.contains("0x100000"), "{e}");
        assert!(r.ops.lock().unwrap().is_empty());
        assert_eq!(w.seeded(), None);
        apply(&w, &[unvid(0x10_0000, 0x1000)]);
        r.ops.lock().unwrap().clear();
        assert_eq!(w.reseed(), Ok(Reseed::Placed { life: 2 }));
    }

    /// A seed still placed, or a window that never had one (`gop=off`, BAR2): no host verb.
    #[test]
    fn a_reseed_without_a_retired_seed_does_nothing() {
        let r = Rec::default();
        let w = seeded(&r);
        assert_eq!(w.reseed(), Ok(Reseed::AlreadyShown));
        let plain = CpuWindow::new(&r, BAR1);
        assert_eq!(plain.reseed(), Ok(Reseed::NoBootFramebuffer));
        assert!(r.ops.lock().unwrap().is_empty());
    }

    /// A refused host map leaves scratch, counts, keeps the life count, and can be retried.
    #[test]
    fn a_refused_reseed_leaves_scratch_and_can_be_retried() {
        let r = Rec::default();
        let w = after_one_life(&r);
        let refused_before = w.stats().refused;
        r.refuse_arms
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let e = w.reseed().expect_err("refused");
        assert!(e.contains("NV_ERR_NO_MEMORY"), "{e}");
        assert!(
            r.ops.lock().unwrap().is_empty(),
            "nothing placed: scratch stays"
        );
        assert_eq!((w.seeded(), w.stats().refused), (None, refused_before + 1));
        r.refuse_arms
            .store(false, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(w.reseed(), Ok(Reseed::Placed { life: 2 }));
    }

    /// ★ The change counter moves once per batch that changed anything — what a deferred re-seed
    /// compares to learn that the guest took BAR1 back first.
    #[test]
    fn the_change_counter_counts_changing_batches_only() {
        let r = Rec::default();
        let w = seeded(&r);
        assert_eq!(w.changes(), 0);
        apply(&w, &[]);
        assert_eq!(w.changes(), 0);
        apply(&w, &[vid(0, 0, G)]);
        apply(&w, &[unvid(0, G)]);
        assert_eq!(w.changes(), 2);
    }

    #[test]
    fn coverage_names_the_placements_and_the_gaps() {
        let r = Rec::default();
        let w = CpuWindow::new(&r, BAR1);
        w.map(&d(0x1000, 0x5000, 0x1000, false), true).unwrap();
        w.map(&d(0x3000, 0x9000, 0x2000, true), true).unwrap();
        let c = w.coverage(0, 0x8000);
        assert_eq!(
            c.inside,
            vec![(0x1000, 0x1000, Some(0x5000)), (0x3000, 0x2000, None)]
        );
        assert_eq!(
            c.gaps,
            vec![(0, 0x1000), (0x2000, 0x1000), (0x5000, 0x3000)]
        );
        assert_eq!(c.uncovered(), 0x5000);
    }

    /// The 16 slots of a store window at `base`.
    fn store_at(base: u64, g: u64) -> Vec<SlotSource> {
        (0..16).map(|i| SlotSource::Store(base + i * g)).collect()
    }

    /// ★★★ 2026-10-03 (review finding 2): a refused PLACEMENT must not retire the view it was
    /// meant to replace. The refused `MAP_FIXED` can leave the old view mapped, and retiring it
    /// hands its host BAR1 aperture back while the guest can still reach it. Before the fix the
    /// second re-point released `0x10_0000` at once.
    #[test]
    fn a_refused_placement_keeps_the_view_it_did_not_cover() {
        let r = Rec::default();
        let g = 0x1_0000;
        let pool = PraminPool::new(&r, g, Box::new(|_, _| None));
        pool.repoint(&store_at(0x10_0000, g));
        r.refuse_place.store(true, Ordering::Relaxed);
        let out = pool.repoint(&store_at(0x40_0000, g));
        assert_eq!(
            r.releases(),
            vec![0x40_0000],
            "only the NEW view, whose own placement was refused, may go: it was never mapped"
        );
        assert_eq!((out.refused, out.kept, out.views), (1, 1, 0));
        r.refuse_place.store(false, Ordering::Relaxed);
        let out = pool.repoint(&store_at(0x70_0000, g));
        assert_eq!((out.refused, out.kept, out.views), (0, 0, 16));
        assert_eq!(
            r.releases(),
            vec![0x40_0000, 0x10_0000],
            "the kept view goes once a placement covers its whole range"
        );
        assert_eq!(pool.kept.load(Ordering::Relaxed), 1);
    }

    /// ★★ The same rule for a refused SINK (a target with nothing addressable behind it).
    #[test]
    fn a_refused_sink_keeps_the_view_it_did_not_cover() {
        let r = Rec::default();
        let g = 0x1_0000;
        let pool = PraminPool::new(&r, g, Box::new(|_, _| None));
        pool.repoint(&store_at(0x10_0000, g));
        r.refuse_sink.store(true, Ordering::Relaxed);
        let out = pool.repoint(&[SlotSource::Nothing; 16]);
        assert!(r.releases().is_empty(), "the old view is still reachable");
        assert_eq!((out.refused, out.kept, out.missed), (1, 1, 16));
        r.refuse_sink.store(false, Ordering::Relaxed);
        let out = pool.repoint(&[SlotSource::Nothing; 16]);
        assert_eq!((out.refused, out.kept), (0, 0));
        assert_eq!(r.releases(), vec![0x10_0000]);
    }

    /// ★★ Partly covered is not covered: a move whose first half lands and whose second half is
    /// refused keeps the old view (the guest still reaches its second half), and retires it with
    /// the half-placed view's successor once a later move covers both.
    #[test]
    fn a_view_covered_only_in_part_is_kept() {
        let r = Rec::default();
        let g = 0x1_0000;
        let pool = PraminPool::new(&r, g, Box::new(|_, _| None));
        pool.repoint(&store_at(0x10_0000, g));
        r.refuse_sink.store(true, Ordering::Relaxed);
        let mut half: Vec<SlotSource> = (0..8)
            .map(|i| SlotSource::Store(0x40_0000 + i * g))
            .collect();
        half.extend([SlotSource::Nothing; 8]);
        let out = pool.repoint(&half);
        assert!(r.releases().is_empty(), "neither view may go yet");
        assert_eq!((out.views, out.refused, out.kept), (8, 1, 1));
        r.refuse_sink.store(false, Ordering::Relaxed);
        let out = pool.repoint(&store_at(0x70_0000, g));
        assert_eq!((out.refused, out.kept), (0, 0));
        let mut gone = r.releases();
        gone.sort_unstable();
        assert_eq!(gone, vec![0x10_0000, 0x40_0000]);
    }

    #[test]
    fn a_view_is_gone_once_every_slot_was_landed_after_it() {
        // Slot generations: slots 0..8 last landed by re-point 3, 8..16 by re-point 2.
        let mut landed = vec![3u64; 8];
        landed.extend([2u64; 8]);
        assert!(
            landed_since(&landed, 0, 16, 1),
            "landed over piecewise, by 2 and 3"
        );
        assert!(
            !landed_since(&landed, 8, 8, 2),
            "its own landing is not a later one"
        );
        assert!(
            !landed_since(&landed, 0, 16, 2),
            "slots 8..16 still show re-point 2's view"
        );
        assert!(landed_since(&landed, 0, 8, 2));
        assert!(!landed_since(&[0; 16], 0, 1, 0), "never landed");
        assert!(
            !landed_since(&landed, 15, 2, 0),
            "a slot past the window never landed"
        );
    }

    /// ★★★ 2026-10-03 (review finding 1 of the third round): a view whose slots are landed over
    /// PIECEWISE, by several re-points that each refuse a different half, is unreachable once the
    /// last of its slots is landed, and must go then. Requiring one re-point to land its whole
    /// range held it for the VM's life (the review's probe: 200 alternating re-points, the first
    /// view never released, `kept` = 399). And `kept` counts each view ONCE.
    #[test]
    fn a_view_landed_over_piecewise_is_retired_and_kept_views_stay_bounded() {
        let r = Rec::default();
        let g = 0x1_0000;
        let pool = PraminPool::new(&r, g, Box::new(|_, _| None));
        let first = 0x10_0000;
        pool.repoint(&store_at(first, g));
        // Every sink is refused from here on; every store placement lands.
        r.refuse_sink.store(true, Ordering::Relaxed);
        let rounds = 200u64;
        let mut worst_live = 0usize;
        for k in 0..rounds {
            let base = 0x100_0000 + k * 0x10_0000;
            let half = if k % 2 == 0 { 8..16 } else { 0..8 };
            let want: Vec<SlotSource> = (0..16usize)
                .map(|i| {
                    if half.contains(&i) {
                        SlotSource::Store(base + i as u64 * g)
                    } else {
                        SlotSource::Nothing
                    }
                })
                .collect();
            let out = pool.repoint(&want);
            assert_eq!((out.refused, out.kept), (1, 1), "round {k}: {out:?}");
            if k == 1 {
                assert!(
                    r.releases().contains(&first),
                    "every slot of the first view was landed over by rounds 0 and 1: it must go"
                );
            }
            let ops = r.ops.lock().unwrap();
            let armed = ops.iter().filter(|o| matches!(o, Op::Arm(..))).count();
            let released = ops.iter().filter(|o| matches!(o, Op::Release(_))).count();
            worst_live = worst_live.max(armed - released);
        }
        assert_eq!(
            worst_live, 2,
            "live views: the latest half and the one before it, never an accumulation"
        );
        assert_eq!(
            pool.kept.load(Ordering::Relaxed),
            rounds,
            "each view counted once: the first, then every half but the last"
        );
        assert_eq!(pool.kept_now.load(Ordering::Relaxed), 1);
    }

    /// ★ `kept` counts VIEWS: one view held across several refused re-points is one.
    #[test]
    fn a_view_kept_across_many_repoints_is_counted_once() {
        let r = Rec::default();
        let g = 0x1_0000;
        let pool = PraminPool::new(&r, g, Box::new(|_, _| None));
        pool.repoint(&store_at(0x10_0000, g));
        r.refuse_sink.store(true, Ordering::Relaxed);
        for _ in 0..5 {
            assert_eq!(pool.repoint(&[SlotSource::Nothing; 16]).kept, 1);
        }
        assert_eq!(
            pool.kept.load(Ordering::Relaxed),
            1,
            "one view, not five events"
        );
        assert_eq!(pool.kept_now.load(Ordering::Relaxed), 1);
        r.refuse_sink.store(false, Ordering::Relaxed);
        assert_eq!(pool.repoint(&[SlotSource::Nothing; 16]).kept, 0);
        assert_eq!(pool.kept_now.load(Ordering::Relaxed), 0);
        assert_eq!(r.releases(), vec![0x10_0000]);
    }

    /// ★ `CpuWindow` already had the rule the PRAMIN pool lacked: an unmap whose sink is refused
    /// keeps the placement and its view (the guest may still reach it) and retries later.
    #[test]
    fn a_window_unmap_whose_sink_is_refused_keeps_the_view() {
        let r = Rec::default();
        let w = CpuWindow::new(&r, 32 << 20);
        w.map(&d(0x4000, 0x2_0000, 0x1000, false), true).unwrap();
        r.refuse_sink.store(true, Ordering::Relaxed);
        assert!(w.unmap(0x4000, true).is_err());
        assert!(r.releases().is_empty(), "released while still placed");
        r.refuse_sink.store(false, Ordering::Relaxed);
        w.unmap(0x4000, true).expect("the retry lands");
        assert_eq!(r.releases(), vec![0x2_0000]);
    }
}
