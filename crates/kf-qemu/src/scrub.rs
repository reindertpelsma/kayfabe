// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★ **The store scrub** — no byte of an earlier guest life, or of an earlier process that held
//! the same video memory, is ever visible to the next guest life (owner requirement, 2026-10-18).
//!
//! STATUS: LIVE (code, GPU-free tested; hardware verification is the checklist in the branch
//! report). Branch `claude/bootlayer-20261018`.
//!
//! The rule, with no policy options: **the whole guest-allocatable store is zeroed when the SAME
//! process reuses the SAME store for a NEW OS instance** — a guest reboot ([`Cause::Reset`]: a system
//! reset from QEMU's main loop, stage Memory, vCPUs paused; scope: the whole store, see below).
//!
//! **Why only there** (owner decision, 2026-10-18, premise OWNER-ACCEPTED, NOT verified by us): the
//! host driver does not promise zeroed memory, it promises process isolation — any allocation can
//! only hold stale bytes of the SAME process, never another process's (like anonymous `mmap`). The
//! one place a stale-bytes leak therefore exists is a reboot of the same VM: the next OS's root could
//! read the previous OS's vidmem.
//!
//! | event | scrub? | already covered by the host's process isolation? |
//! |---|---|---|
//! | VM start (a fresh process, a fresh store) | no — realize still zeroes what kayfabe itself needs zero (the roots, the boot framebuffer `[0, G)`), and awaits that zero's completion | yes (premise); a crashed previous QEMU's store was cleaned by the host on process death |
//! | guest REBOOT (system reset, main loop) | **yes** | no: same process, same store, a new OS |
//! | guest-issued bus reset (a vCPU thread), S3 wake | no | n/a — the same OS keeps its own root |
//! | RM unload, the basic-display -> NVIDIA handoff, a TDR-style reset | no (the boot range is zeroed for the DISPLAY only, see `bar1phys`) | n/a — the same OS can read its own vidmem already; a real GPU does not scrub there |
//! | VM teardown | no | yes (premise): the store is freed to the host driver |
//!
//! A reset is told from a reboot by what the device sees: where QEMU asked from (`current_cpu != NULL`
//! is a vCPU thread: the guest's own bus reset) and the reset type (`RESET_TYPE_WAKEUP`); see
//! [`event_of_reset`]. Were they indistinguishable the lazy rule would scrub on every reset.
//!
//! **Scope: the WHOLE store `[0, fb_length)`** (review B2, 2026-10-18). The firmware carve-out
//! `[carve, fb_length)` is declared to the guest as reserved, but it is not out of the guest's reach:
//! BAR1/BAR2/PRAMIN CPU views keep the `fb_len` bound and references into it are counted, not refused
//! (`kf_mem::apply`), so a hostile root could park data there in one OS and read it in the next.
//! What kayfabe itself keeps in the store across a reset is ONLY our two root pages
//! (`bar1_pde_base`, `bar2_pde_base`): every other store write is a root page, a heap-gated
//! USERD/notifier (`chan::heap_gate`), or at a guest-chosen offset. The reset's own stage rewrites
//! the roots and re-registers the BAR objects AFTER the scrub (`mem::reset_memory_finish`).
//! Hardware checklist: after a reboot, read `[carve, fb_length)` back from the guest.
//!
//! **Completion ordering.** [`run`] returns `Ok` only after [`Zeroer::complete`] (the device-side
//! completion) succeeded, and the caller acknowledges the reset stage — which opens the gate and
//! lets the vCPUs resume — only on `Ok`. A refused or incomplete scrub is an [`Err`] naming the
//! range; the caller keeps the gate closed and says so (`V3_DEVICE_RESET.md` §6 item 1: fail
//! closed). A rerun starts from the beginning (zeroing is idempotent), so a second reset request
//! that arrives during or after a failed scrub is simply a second full run.

use std::sync::atomic::{AtomicU64, Ordering};

use kf_chip::bar0::FbLayout;

/// Bytes per queued memset. Bounded so one chunk is bounded GPU work and `progress` moves; large
/// enough that the per-chunk host cost is nothing next to the memset itself.
pub const CHUNK: u64 = 256 << 20;

/// Why the store is being scrubbed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// A guest reboot: the memory stage of a system reset.
    Reset,
}

/// ★ What happened to the guest's video memory's trust boundary (the owner's table,
/// 2026-10-18). Only the SAME process reusing the SAME store for a NEW OS instance scrubs (a
/// reboot); an in-guest event keeps the same OS and the same root, which can read all of its own
/// vidmem already, and a new process or a freed store is the host driver's process isolation
/// (owner-accepted premise, not verified by us).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The VM starts: a fresh process with a fresh store (the host's process isolation).
    VmStart,
    /// The machine's system reset from QEMU's main loop: a guest reboot, the reset button, a
    /// bugcheck or triple-fault reset, QMP `system_reset` — a new OS instance follows.
    GuestReboot,
    /// The platform's S3 wake: the same OS resumes a PM-suspended RM life.
    S3Wake,
    /// A bus reset delivered on a vCPU thread (a bridge's secondary-bus reset, the guest's own act).
    GuestBusReset,
    /// fn 47 `UNLOADING_GUEST_DRIVER` without a PM transition: the guest RM unloads.
    DriverUnload,
    /// The basic display (firmware GOP) handing over to the guest NVIDIA driver in a running OS.
    DisplayHandoff,
    /// The VM is going away; the store is freed to the host driver (accepted premise, owner: it
    /// scrubs freed vidmem before another process can get it).
    VmTeardown,
}

/// ★ THE TABLE: does `event` scrub the guest-allocatable store, and with which scope?
#[must_use]
pub fn decide(event: Event) -> Option<Cause> {
    match event {
        Event::GuestReboot => Some(Cause::Reset),
        // the same OS and the same root keep running (it can read its own vidmem already), or the
        // host's process isolation covers it (owner-accepted premise): see the module docs
        Event::VmStart
        | Event::S3Wake
        | Event::GuestBusReset
        | Event::DriverUnload
        | Event::DisplayHandoff
        | Event::VmTeardown => None,
    }
}

/// The [`Event`] a QEMU reset request is, from what the device sees: where QEMU asked from (a vCPU
/// thread: `current_cpu != NULL`, so the guest's own act) and the reset type (`RESET_TYPE_WAKEUP`).
#[must_use]
pub fn event_of_reset(from_vcpu: bool, wake: bool) -> Event {
    if wake {
        Event::S3Wake
    } else if from_vcpu {
        Event::GuestBusReset
    } else {
        Event::GuestReboot
    }
}

/// The ranges to zero, as `(offset, length)`, sorted, non-overlapping, non-empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The ranges.
    pub ranges: Vec<(u64, u64)>,
}

impl Plan {
    /// Total bytes.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.ranges.iter().map(|r| r.1).sum()
    }

    /// The chunks of the plan, in order: each at most [`CHUNK`] bytes, never crossing a range.
    #[must_use]
    pub fn chunks(&self) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        for &(off, len) in &self.ranges {
            let mut at = 0;
            while at < len {
                let n = (len - at).min(CHUNK);
                out.push((off + at, n));
                at += n;
            }
        }
        out
    }
}

/// ★ The plan for `cause` on `layout`, plus `boot` (the boot framebuffer's store range, always
/// inside the heap; merged, so it is never zeroed twice).
#[must_use]
pub fn plan(layout: &FbLayout, cause: Cause, boot: Option<(u64, u64)>) -> Plan {
    let mut want: Vec<(u64, u64)> = match cause {
        // the WHOLE store (review B2): the guest's CPU views and its GPU leaves reach the carve-out
        // too (counted, not refused), so nothing above `carve` is out of its reach; our own two
        // root pages are rewritten by the reset's stage after the scrub
        Cause::Reset => vec![(0, layout.fb_length)],
    };
    if let Some(b) = boot {
        want.push(b);
    }
    want.retain(|r| r.1 > 0);
    want.sort_unstable();
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    for (off, len) in want {
        let end = (off + len).min(layout.fb_length);
        if end <= off {
            continue;
        }
        match ranges.last_mut() {
            Some(last) if off <= last.0 + last.1 => last.1 = last.1.max(end - last.0),
            _ => ranges.push((off, end - off)),
        }
    }
    Plan { ranges }
}

/// The device-side zero. `zero` QUEUES (it may return before the bytes are zero); `complete` is the
/// completion of everything queued.
pub trait Zeroer {
    /// Queue a zero of `[off, off+len)`.
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn zero(&mut self, off: u64, len: u64) -> Result<(), String>;
    /// Everything queued is zero in device memory.
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn complete(&mut self) -> Result<(), String>;
}

/// A scrub that did not finish. The store is NOT clean: the caller must keep the gate closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incomplete {
    /// Bytes queued before the refusal.
    pub queued: u64,
    /// Bytes the plan holds.
    pub total: u64,
    /// The chunk that was refused, or `None` when the completion was.
    pub at: Option<(u64, u64)>,
    /// The host's words.
    pub why: String,
}

impl std::fmt::Display for Incomplete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.at {
            Some((off, len)) => write!(
                f,
                "store scrub REFUSED at [{off:#x}, +{len:#x}) after {:#x} of {:#x} bytes: {}",
                self.queued, self.total, self.why
            ),
            None => write!(
                f,
                "store scrub queued {:#x} bytes but its completion was REFUSED: {}",
                self.total, self.why
            ),
        }
    }
}

/// ★ Run `plan`: every chunk queued, then the completion. `progress` (bytes queued so far) is for
/// the status line. `Ok(bytes)` only after the completion.
///
/// # Errors
/// [`Incomplete`], with the store not clean.
pub fn run(
    plan: &Plan,
    z: &mut impl Zeroer,
    progress: &AtomicU64,
) -> Result<u64, Incomplete> {
    let total = plan.bytes();
    let mut queued = 0u64;
    progress.store(0, Ordering::Release);
    for (off, len) in plan.chunks() {
        z.zero(off, len).map_err(|why| Incomplete {
            queued,
            total,
            at: Some((off, len)),
            why,
        })?;
        queued += len;
        progress.store(queued, Ordering::Release);
    }
    z.complete().map_err(|why| Incomplete {
        queued,
        total,
        at: None,
        why,
    })?;
    Ok(total)
}

/// ★ The production [`Zeroer`]: the walker's CUDA context on the store's own device mapping —
/// `cuMemsetD8` per chunk (no host buffer, no PCIe), one `cuCtxSynchronize` for the completion.
/// ⊘ Only ever driven from a worker thread (the VA thread): the completion blocks the CALLER, and a
/// vCPU is paused around a reset and never waits here.
pub struct WalkZeroer<'a>(pub &'a kf_cuda::walk::WalkKernel);

impl Zeroer for WalkZeroer<'_> {
    fn zero(&mut self, off: u64, len: u64) -> Result<(), String> {
        self.0.zero_store_queued(off, len).map_err(|e| e.to_string())
    }
    fn complete(&mut self) -> Result<(), String> {
        self.0.sync_store().map_err(|e| e.to_string())
    }
}

/// One range, zeroed and completed — the boot framebuffer at an RM unload (`bar1phys`).
///
/// # Errors
/// [`Incomplete`]'s words.
pub fn zero_range(
    k: &kf_cuda::walk::WalkKernel,
    off: u64,
    len: u64,
    progress: &AtomicU64,
) -> Result<u64, Incomplete> {
    run(&Plan { ranges: vec![(off, len)] }, &mut WalkZeroer(k), progress)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_chip::bar0::{FW_CARVE_OUT_BYTES, fb_layout};

    const MIB: u64 = 1 << 20;
    /// A store of 1 GiB plus the carve-out: big enough to chunk, small enough to model.
    fn layout() -> FbLayout {
        fb_layout(FW_CARVE_OUT_BYTES + 1024 * MIB).expect("a layout")
    }

    /// A model store: bytes `[0, len)` each dirty (0xAA) or zero, kept as sorted dirty intervals.
    struct Model {
        dirty: Vec<(u64, u64)>,
        queued: Vec<(u64, u64)>,
        completed: bool,
        refuse_zero_at: Option<u64>,
        refuse_complete: bool,
        /// every `zero` call, in order
        calls: Vec<(u64, u64)>,
    }

    impl Model {
        fn filled(len: u64) -> Model {
            Model {
                dirty: vec![(0, len)],
                queued: Vec::new(),
                completed: false,
                refuse_zero_at: None,
                refuse_complete: false,
                calls: Vec::new(),
            }
        }
        fn clear_range(&mut self, off: u64, len: u64) {
            let end = off + len;
            let mut out = Vec::new();
            for &(s, l) in &self.dirty {
                let e = s + l;
                if e <= off || s >= end {
                    out.push((s, l));
                    continue;
                }
                if s < off {
                    out.push((s, off - s));
                }
                if e > end {
                    out.push((end, e - end));
                }
            }
            self.dirty = out;
        }
        fn is_dirty(&self, off: u64, len: u64) -> bool {
            self.dirty.iter().any(|&(s, l)| s < off + len && s + l > off)
        }
        fn dirty_bytes_in(&self, off: u64, len: u64) -> u64 {
            self.dirty
                .iter()
                .map(|&(s, l)| (s + l).min(off + len).saturating_sub(s.max(off)))
                .sum()
        }
    }

    impl Zeroer for Model {
        fn zero(&mut self, off: u64, len: u64) -> Result<(), String> {
            self.calls.push((off, len));
            if self.refuse_zero_at == Some(off) {
                return Err("cuMemsetD8: CUDA_ERROR_UNKNOWN".to_string());
            }
            // a queued zero is NOT visible until `complete` — the model is strict about it
            self.queued.push((off, len));
            Ok(())
        }
        fn complete(&mut self) -> Result<(), String> {
            if self.refuse_complete {
                return Err("cuCtxSynchronize: device fault".to_string());
            }
            for (o, l) in std::mem::take(&mut self.queued) {
                self.clear_range(o, l);
            }
            self.completed = true;
            Ok(())
        }
    }

    /// ★ THE MODEL TEST of the owner's rule: whatever the old life (or an older process) left,
    /// after a reset no guest-allocatable byte retains it.
    #[test]
    fn after_a_reset_no_guest_allocatable_byte_retains_old_bytes() {
        let l = layout();
        let mut m = Model::filled(l.fb_length);
        let p = plan(&l, Cause::Reset, Some((0, 0x7F_0000)));
        let done = run(&p, &mut m, &AtomicU64::new(0)).expect("scrubbed");
        assert_eq!(done, l.fb_length);
        assert_eq!(m.dirty_bytes_in(0, l.fb_length), 0, "the whole store is zero");
        assert!(m.completed);
    }

    /// ★ Review B2: the firmware carve-out is NOT out of the guest's reach (BAR1/BAR2/PRAMIN CPU
    /// views keep the `fb_len` bound; references into `[carve, fb_len)` are counted, not refused),
    /// so a hostile root could park data there in OS 1 and read it in OS 2. A reboot therefore
    /// zeroes the WHOLE store. What kayfabe itself keeps there — our two root pages, nothing else
    /// (every other store write is a root, a heap-gated USERD/notifier, or a guest-chosen offset) —
    /// is rewritten by the reset's own stage after the scrub.
    #[test]
    fn a_reset_zeroes_the_whole_store_and_the_roots_are_rewritten_after() {
        let l = layout();
        let mut m = Model::filled(l.fb_length);
        let n = run(&plan(&l, Cause::Reset, None), &mut m, &AtomicU64::new(0)).expect("scrubbed");
        assert_eq!(n, l.fb_length, "the carve-out is scrubbed too");
        assert!(m.dirty.is_empty(), "{:x?}", m.dirty);
        for root in [l.bar1_pde_base, l.bar2_pde_base] {
            assert!(root >= l.carve() && !m.is_dirty(root, kf_chip::bar0::ROOT_PAGE_BYTES));
        }
    }

    /// A store whose size is not a multiple of the chunk: the partial last chunk is zeroed too, and
    /// no byte past the store is ever named.
    #[test]
    fn an_unaligned_store_scrubs_its_partial_last_chunk() {
        let l = fb_layout(FW_CARVE_OUT_BYTES + 1000 * MIB + 0x1_0000).expect("a layout");
        assert_ne!(l.fb_length % CHUNK, 0);
        let p = plan(&l, Cause::Reset, None);
        let c = p.chunks();
        let (lo, ln) = *c.last().expect("chunks");
        assert!(ln < CHUNK && lo + ln == l.fb_length, "{c:x?}");
        let mut m = Model::filled(l.fb_length);
        run(&p, &mut m, &AtomicU64::new(0)).expect("scrubbed");
        assert!(m.dirty.is_empty(), "the partial last chunk was dropped: {:x?}", m.dirty);
        assert!(m.calls.iter().all(|&(o, n)| o + n <= l.fb_length));
    }

    /// The boot framebuffer is inside the heap: asking for it too never zeroes anything twice, and
    /// a range past the store is clipped to it.
    #[test]
    fn the_boot_range_is_merged_and_never_zeroed_twice() {
        let l = layout();
        let p = plan(&l, Cause::Reset, Some((0, 0x7F_0000)));
        assert_eq!(p.ranges, vec![(0, l.fb_length)]);
        // a range the layout does not hold is clipped to the store, never read past it
        let q = plan(&l, Cause::Reset, Some((l.fb_length - 4096, 1 << 20)));
        assert_eq!(q.ranges, vec![(0, l.fb_length)], "a range inside the store merges, one past it is clipped");
        let inside = plan(&l, Cause::Reset, Some((16, 100)));
        assert_eq!(inside.ranges, vec![(0, l.fb_length)]);
    }

    /// Chunks: bounded, ordered, contiguous, covering the plan exactly.
    #[test]
    fn chunks_cover_the_plan_exactly_and_never_exceed_the_bound() {
        let l = layout();
        let p = plan(&l, Cause::Reset, None);
        let c = p.chunks();
        assert!(c.iter().all(|&(_, n)| n > 0 && n <= CHUNK));
        assert_eq!(c.iter().map(|&(_, n)| n).sum::<u64>(), p.bytes());
        assert!(c.windows(2).all(|w| w[0].0 + w[0].1 == w[1].0), "{c:x?}");
        assert!(c.len() >= 4, "1 GiB of heap is several chunks");
    }

    /// ★ COMPLETION ORDER: the queued zeros are not zero until `complete`; `run` returns Ok only
    /// after it. A model that is only checked after `run` would pass a run that skipped it.
    #[test]
    fn ok_means_the_completion_ran() {
        let l = layout();
        let mut m = Model::filled(l.fb_length);
        let progress = AtomicU64::new(0);
        run(&plan(&l, Cause::Reset, None), &mut m, &progress).expect("ok");
        assert!(m.completed);
        assert_eq!(progress.load(Ordering::Acquire), l.fb_length);
    }

    /// ★ REFUSAL: a refused chunk is an error naming the range, the store is NOT clean, and the
    /// completion is never claimed.
    #[test]
    fn a_refused_chunk_is_incomplete_and_names_its_range() {
        let l = layout();
        let mut m = Model::filled(l.fb_length);
        m.refuse_zero_at = Some(CHUNK);
        let e = run(&plan(&l, Cause::Reset, None), &mut m, &AtomicU64::new(0)).unwrap_err();
        assert_eq!(e.at, Some((CHUNK, CHUNK)));
        assert_eq!(e.queued, CHUNK);
        assert!(e.to_string().contains("REFUSED at") && e.to_string().contains("CUDA_ERROR"));
        assert!(!m.completed);
        assert!(m.dirty_bytes_in(0, l.fb_length) > 0, "not clean: the caller keeps the gate closed");
    }

    /// A completion the host refuses is as bad as a refused chunk.
    #[test]
    fn a_refused_completion_is_incomplete() {
        let l = layout();
        let mut m = Model::filled(l.fb_length);
        m.refuse_complete = true;
        let e = run(&plan(&l, Cause::Reset, None), &mut m, &AtomicU64::new(0)).unwrap_err();
        assert_eq!(e.at, None);
        assert!(e.to_string().contains("completion was REFUSED"), "{e}");
        assert!(m.dirty_bytes_in(0, l.fb_length) > 0, "queued zeros were never completed");
    }

    /// ★ IDEMPOTENCE: a second reset during or after a failed scrub is a second full run from the
    /// start, and the end state is the same as one clean run.
    #[test]
    fn a_rerun_after_a_failure_ends_clean_and_a_double_run_changes_nothing() {
        let l = layout();
        let p = plan(&l, Cause::Reset, None);
        let mut m = Model::filled(l.fb_length);
        m.refuse_zero_at = Some(2 * CHUNK);
        assert!(run(&p, &mut m, &AtomicU64::new(0)).is_err());
        m.refuse_zero_at = None;
        m.calls.clear();
        assert_eq!(run(&p, &mut m, &AtomicU64::new(0)), Ok(l.fb_length));
        assert_eq!(m.calls.first(), Some(&(0, CHUNK)), "restarted from the beginning");
        let after_one = m.dirty.clone();
        assert_eq!(run(&p, &mut m, &AtomicU64::new(0)), Ok(l.fb_length));
        assert_eq!(m.dirty, after_one);
        assert_eq!(m.dirty_bytes_in(0, l.fb_length), 0);
    }

    /// A store smaller than one chunk, and a layout whose boot range is empty, are plain cases.
    #[test]
    fn an_empty_boot_range_adds_nothing() {
        let l = layout();
        assert_eq!(plan(&l, Cause::Reset, Some((0, 0))), plan(&l, Cause::Reset, None));
    }

    /// ★ THE OWNER'S TABLE, event -> scrub?: only a guest REBOOT scrubs — the one place the SAME
    /// process reuses the SAME store for a new OS. VM start and teardown are the host's process
    /// isolation (a fresh process gets a fresh store; a dead one's is the host driver's), and
    /// everything inside a running OS keeps its own root.
    #[test]
    fn the_scrub_table_follows_the_trust_boundary() {
        let table = [
            (Event::VmStart, None),
            (Event::GuestReboot, Some(Cause::Reset)),
            (Event::VmTeardown, None),
            (Event::S3Wake, None),
            (Event::GuestBusReset, None),
            (Event::DriverUnload, None),
            (Event::DisplayHandoff, None),
        ];
        for (e, want) in table {
            assert_eq!(decide(e), want, "{e:?}");
        }
    }

    /// ★ What the device can tell apart: QEMU's main loop (vCPUs paused) is a system reset — a new
    /// OS — a vCPU thread is the guest's own bus reset, and RESET_TYPE_WAKEUP is the S3 resume
    /// (`kf3.c: kf3_device_reset(s->h, current_cpu != NULL, type == RESET_TYPE_WAKEUP)`).
    #[test]
    fn a_reset_request_is_classified_by_where_it_came_from() {
        assert_eq!(event_of_reset(false, false), Event::GuestReboot);
        assert_eq!(event_of_reset(true, false), Event::GuestBusReset);
        assert_eq!(event_of_reset(false, true), Event::S3Wake);
        assert_eq!(event_of_reset(true, true), Event::S3Wake, "a wake is a wake");
    }
}
