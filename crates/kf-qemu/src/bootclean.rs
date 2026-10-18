// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★ **Is store `[0, G)` fit to be shown as the boot picture?** (2026-10-18, branch
//! `claude/bootlayer-20261018`.)
//!
//! The boot layer scans store `[0, G)`. That range is the firmware console's framebuffer ONLY while
//! the guest's firmware (OVMF's GOP, the bootloader, a basic-display driver) is what writes it. From
//! the moment a guest RM starts, the same bytes are ordinary video memory: the region table the
//! device serves declares `[0, carve)` as heap (`kf_chip::bar0::fb_layout`) unless the guest reserves
//! a console, and the RM's GPU work writes its structures there. Drawn as pixels they are thin
//! coloured rows at fixed positions (`[measured, run 3081]` the stripes on the boot screen and on the
//! screen after an unload; the RM allocates the same low addresses every life).
//!
//! So the boot layer is shown only while the range is **clean**:
//!
//! - it starts clean (realize zeroed it);
//! - it is **tainted** when a guest RM life begins (fn 72, the first RPC of every RM life — before
//!   the RM can allocate heap), when the seed retires (the guest's CPU writes then go to scratch, so
//!   the store no longer follows its framebuffer), and when a device reset is requested;
//! - it is **cleaned** only by a scrub that COMPLETED: at an RM unload (`bar1phys`) or at a device
//!   reset's memory stage.
//!
//! While tainted the console shows no boot layer: the last frame stays (before any guest scanout was
//! ever shown), or black (after one was lost) — never the RM's data. Once a head is armed the boot
//! layer is not shown at all (`boot_done`), so a guest that arms its display is unaffected.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The shared flag: written by the VA thread, the drainer and the reset sequence, read by the display
/// worker. Lock-free; nothing here ever waits.
#[derive(Debug)]
pub struct BootClean {
    clean: AtomicBool,
    /// ★ Review (2026-10-18): the guest RM life the range belongs to, bumped at fn 72. An unload's
    /// scrub carries the generation it was asked in and may neither zero nor clean a newer life's.
    life: AtomicU64,
}

impl Default for BootClean {
    fn default() -> BootClean {
        BootClean::new()
    }
}

impl BootClean {
    /// Clean: realize zeroed the range and nothing has written it since.
    #[must_use]
    pub const fn new() -> BootClean {
        BootClean {
            clean: AtomicBool::new(true),
            life: AtomicU64::new(0),
        }
    }

    /// Something other than the firmware console may now write the range.
    pub fn taint(&self) {
        self.clean.store(false, Ordering::Release);
    }

    /// A scrub of the range COMPLETED (never called on a refused or unfinished one).
    pub fn clean(&self) {
        self.clean.store(true, Ordering::Release);
    }

    /// The current guest RM life.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.life.load(Ordering::Acquire)
    }

    /// A guest RM life BEGINS (fn 72): the generation moves on and the range is tainted. Returns
    /// the new generation.
    pub fn begin_life(&self) -> u64 {
        // RED STEP (tests first): the generation is not bumped yet
        self.taint();
        self.generation()
    }

    /// A scrub asked in life `life_id` completed: clean the range only if no newer life began since.
    /// Returns whether it cleaned.
    pub fn clean_if(&self, life_id: u64) -> bool {
        // RED STEP (tests first): the generation is not checked yet
        let _ = life_id;
        self.clean();
        true
    }

    /// May the boot layer be shown?
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.clean.load(Ordering::Acquire)
    }
}

/// ★ The VA thread's edge detector: the seed going from placed to retired is a taint, whichever
/// pass of its loop notices; a seed that stays retired, or is placed again, is not.
#[derive(Debug)]
pub struct SeedWatch {
    placed: bool,
}

impl Default for SeedWatch {
    fn default() -> SeedWatch {
        SeedWatch { placed: true }
    }
}

impl SeedWatch {
    /// The window's seed state this pass. `true` when the range must be tainted now.
    pub fn observe(&mut self, placed: bool) -> bool {
        let retired_now = self.placed && !placed;
        self.placed = placed;
        retired_now
    }
}

/// ★ What the boot-layer wiring tells the display, as a trait so the wiring decisions are tested
/// without a display worker. `DisplayPlane` is the production sink.
pub trait BootSink {
    /// A guest RM life began (fn 72): returns the new generation.
    fn begin_life(&self) -> u64;
    /// The range may be written by something other than the firmware console.
    fn taint(&self);
    /// The current generation.
    fn generation(&self) -> u64;
    /// A scrub asked in `life_id` completed; cleans only if still that life. Returns whether it did.
    fn scrubbed_if(&self, life_id: u64) -> bool;
    /// A scrub completed (a reset's memory stage, which owns the whole range).
    fn scrubbed(&self);
}

/// ★ The drainer saw guest RPC `function`: fn 72 (`GSP_SET_SYSTEM_INFO`), the first RPC of every RM
/// life and before any heap use, begins a life and taints the range.
pub fn on_rpc<S: BootSink>(sink: &S, function: kf_gsp::RpcFunction) {
    // RED STEP (tests first)
    let _ = (sink, function);
}

/// ★ A reset's memory stage finished: the boot layer may come back only if the boot framebuffer's
/// zero was accepted AND awaited.
pub fn after_memory_stage<S: BootSink>(sink: &S, boot_zeroed: bool) {
    // RED STEP (tests first)
    let _ = (sink, boot_zeroed);
}

/// What the unload's scrub did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnloadScrub {
    /// Not attempted (the seed was not retired and nothing was tainted, or an RM took BAR1 again).
    NotAsked,
    /// Zeroed and completed.
    Done,
    /// Refused or incomplete.
    Failed,
    /// Skipped because a newer RM life began (fn 72) after this unload.
    StaleLife,
}

/// ★ The unload's scrub finished (or not) for the unload asked in life `life_id`: the boot layer comes
/// back only after a scrub that COMPLETED and only if no newer life began since.
pub fn after_unload_scrub<S: BootSink>(sink: &S, life_id: u64, outcome: UnloadScrub) {
    // RED STEP (tests first)
    let _ = (sink, life_id, outcome);
}

/// ★ Review (2026-10-18): the unload's scrub must wait for the dead RM's twins to be FREED — the
/// act thread frees them asynchronously (`ChanPlane::free_dead_clients`) and a channel still
/// running writes vidmem. The drainer [`FreesBarrier::request`]s a ticket and queues a barrier act
/// behind the frees; the act calls [`FreesBarrier::done`]. Never stuck: a barrier that cannot be
/// queued is marked done at once.
#[derive(Debug, Default)]
pub struct FreesBarrier {
    requested: AtomicU64,
    done: AtomicU64,
}

impl FreesBarrier {
    /// A ticket: frees were queued and a barrier will follow.
    pub fn request(&self) -> u64 {
        self.requested.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// The barrier behind ticket `n` ran: every free queued before it has run.
    pub fn done(&self, n: u64) {
        self.done.fetch_max(n, Ordering::AcqRel);
    }

    /// Every free asked for so far has completed.
    #[must_use]
    pub fn is_open(&self) -> bool {
        // RED STEP (tests first)
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_starts_clean_and_only_a_completed_scrub_cleans_it_again() {
        let b = BootClean::new();
        assert!(b.is_clean(), "realize zeroed it");
        b.taint();
        assert!(!b.is_clean());
        b.taint();
        assert!(!b.is_clean(), "tainting twice is still tainted");
        b.clean();
        assert!(b.is_clean());
    }

    #[test]
    fn the_seed_retiring_is_one_edge_and_only_that() {
        let mut w = SeedWatch::default();
        assert!(!w.observe(true), "placed from the start");
        assert!(w.observe(false), "retired: the first BAR1 change");
        assert!(!w.observe(false), "staying retired is not a new edge");
        assert!(!w.observe(true), "placed again (reset, unload) is not a taint");
        assert!(w.observe(false), "the next RM life retires it again");
    }

    /// A sink that records every call.
    #[derive(Default)]
    struct Rec {
        calls: std::cell::RefCell<Vec<String>>,
        boot: BootClean,
    }
    impl BootSink for Rec {
        fn begin_life(&self) -> u64 {
            self.calls.borrow_mut().push("begin_life".into());
            self.boot.begin_life()
        }
        fn taint(&self) {
            self.calls.borrow_mut().push("taint".into());
            self.boot.taint();
        }
        fn generation(&self) -> u64 {
            self.boot.generation()
        }
        fn scrubbed_if(&self, life_id: u64) -> bool {
            self.calls.borrow_mut().push(format!("scrubbed_if {life_id}"));
            self.boot.clean_if(life_id)
        }
        fn scrubbed(&self) {
            self.calls.borrow_mut().push("scrubbed".into());
            self.boot.clean();
        }
    }

    /// ★ Review (M4d): fn 72 — and only fn 72 — begins a life and taints the range.
    #[test]
    fn fn_72_begins_a_life_and_nothing_else_does() {
        let r = Rec::default();
        on_rpc(&r, kf_gsp::RpcFunction::GspSetSystemInfo);
        assert_eq!(*r.calls.borrow(), ["begin_life"]);
        assert!(!r.boot.is_clean(), "tainted at fn 72");
        assert_eq!(r.boot.generation(), 1);
        let q = Rec::default();
        on_rpc(&q, kf_gsp::RpcFunction::UnloadingGuestDriver);
        on_rpc(&q, kf_gsp::RpcFunction::Free);
        assert!(q.calls.borrow().is_empty(), "no other RPC touches the gate");
        assert!(q.boot.is_clean());
    }

    /// ★ Review (2): the generation moves on at every fn 72, and a scrub asked in an older life
    /// can neither clean a newer life's range.
    #[test]
    fn a_stale_scrub_cannot_clean_a_newer_lifes_range() {
        let b = BootClean::new();
        let old = b.begin_life();
        let new = b.begin_life();
        assert!(new > old);
        assert!(!b.clean_if(old), "the old unload must not clear the new life's taint");
        assert!(!b.is_clean());
        assert!(b.clean_if(new));
        assert!(b.is_clean());
    }

    /// ★ Review (M4a): the boot layer comes back only after an unload scrub that COMPLETED, in the
    /// life it was asked in.
    #[test]
    fn the_unload_scrub_cleans_only_when_it_completed_in_its_own_life() {
        for (outcome, cleans) in [
            (UnloadScrub::Done, true),
            (UnloadScrub::Failed, false),
            (UnloadScrub::StaleLife, false),
            (UnloadScrub::NotAsked, false),
        ] {
            let r = Rec::default();
            let life_id = r.boot.begin_life();
            after_unload_scrub(&r, life_id, outcome);
            assert_eq!(r.boot.is_clean(), cleans, "{outcome:?}");
        }
        // a scrub that completed but belongs to an older life leaves the newer taint
        let r = Rec::default();
        let old = r.boot.begin_life();
        r.boot.begin_life();
        after_unload_scrub(&r, old, UnloadScrub::Done);
        assert!(!r.boot.is_clean());
    }

    /// ★ A reset's memory stage cleans only if the boot framebuffer's zero was accepted.
    #[test]
    fn the_memory_stage_cleans_only_after_an_accepted_zero() {
        let r = Rec::default();
        r.boot.taint();
        after_memory_stage(&r, false);
        assert!(!r.boot.is_clean(), "a refused zero must not bring the boot layer back");
        after_memory_stage(&r, true);
        assert!(r.boot.is_clean());
    }

    /// ★ Review (5): the barrier is open only once every ticket's act ran, in any order.
    #[test]
    fn the_frees_barrier_opens_when_every_ticket_ran() {
        let b = FreesBarrier::default();
        assert!(b.is_open(), "nothing asked");
        let a = b.request();
        let c = b.request();
        assert!(!b.is_open());
        b.done(a);
        assert!(!b.is_open(), "the second ticket is still out");
        b.done(c);
        assert!(b.is_open());
        b.done(a);
        assert!(b.is_open(), "a late duplicate never closes it");
    }
}
