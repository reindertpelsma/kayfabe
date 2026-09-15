//! ★★★★★ **§3's DEVICE-VIEW PORT — the reserved object, reachable after bring-up.**
//!
//! `SINGLE_STORE_PLAN.md` §3, *"WHAT §3 STILL NEEDS, in order"*, item 1:
//!
//! > A **device-view port** reachable after bring-up (the `WalkShadowPort` shape; ⚠ it and
//! > the walk shadow both want the one `IsolateBox`, so they must share it, not take it).
//!
//! Before this module the crossing existed only as [`crate::scratchpad::probe_device_view`],
//! a **bring-up-only** probe: it ran inside `Scratchpad::bring_up`'s body and nothing could
//! re-enter it afterwards, because the only public route to the isolate was
//! `share_for_walk_shadow`, which *moved* the box and existed only on the
//! `KAYFABE_WALK_SHADOW` arm.
//!
//! # ⊘⊘⊘ WHAT THIS PORT MAY AND MAY NOT BE CALLED FROM — the whole safety argument
//!
//! Arming is [`kayfabe_isolate::Worker::export_device_view`], which is **an IPC round trip to
//! a child process** and asserts `lockwitness::assert_lock_free` on arrival. ⇒ every verb here
//! is **lock-free and off-trap**, and says so with its own witness claim rather than relying
//! on the callee's.
//!
//! ⚠ That is not a style preference. `SINGLE_STORE_PLAN.md` §3's second structural fact:
//!
//! > ★★★ **`page_backing` / `read` / `write` CANNOT ARM.** … all three of those run **under
//! > the plane lock**, and two of them on a vCPU inside an MMIO exit. ⇒ the store can only
//! > report **where** a page lives; a lock-free caller must do the arming.
//!
//! ⇒ **Nothing in `kayfabe-device` may reach this port.** The callers are
//! `BarMirror::fill_now`'s step 2 and the page-table reader's arm-then-retry, both of which
//! are lock-free at entry.
//!
//! # ★★★ THE OWNER'S RULING OF 2026-09-14, ENFORCED BY SHAPE AND NOT BY DISCIPLINE
//!
//! `bar1_passthrough_device_local_host_visible.md` §3.2, decision (b), has three conditions.
//! Two of them are properties of what a holder of the descriptor does with it, so this port
//! **never hands a descriptor out**:
//!
//! 1. **No escape on the node — only `mmap`.** [`DeviceViewPort::arm_mapped`] and
//!    [`DeviceViewPort::with_node`] are the only ways in, and the `BorrowedFd` a caller sees
//!    is alive for exactly one closure call whose only sanctioned use is a mapping verb.
//! 2. **Closed the moment `mmap` returns.** The `OwnedFd` is dropped on the line after the
//!    closure returns, on every path including the error ones.
//! 3. **The crossing is `SCM_RIGHTS`** — inside `ProxyRmBackend::call_for_device_view`.
//!
//! # ⚠ RELEASE IS NOT OPTIONAL, AND `munmap` IS NOT A RELEASE
//!
//! `[measured w722, GA106]` `NV_ESC_RM_UNMAP_MEMORY` returns 224 MiB of host BAR1 per round,
//! 5/5. `munmap` + `close` — *"which is what this tree does today"* — returns **zero** from
//! round 1 on, with `ioctl(2)` answering 0 and `errno == 0`. ⇒ [`ArmedView`] is `#[must_use]`
//! and its `Drop` **screams**, because the failure it guards is silent and ends in refusing
//! every later arm.

use std::os::fd::{AsFd, OwnedFd};
use std::sync::atomic::{AtomicU64, Ordering};

use kayfabe_isolate::{HostHandle, IsolateId};

use crate::scratchpad::SharedIsolate;

/// How the shell turns an isolate-minted export token into a descriptor in **this** process.
///
/// ⊘ The same shape as [`crate::scratchpad::DupFn`] and deliberately a second alias rather
/// than a re-use: this one is stored in an [`std::sync::Arc`] for the port's whole life,
/// where the scratchpad's is borrowed for the length of one bring-up. `Send + Sync` is
/// required here and is not there.
pub type DupArc = dyn Fn(IsolateId, u64) -> Option<OwnedFd> + Send + Sync;

/// Why an arm did not happen. ⊘ Four causes, four names: a single "failed" would make the
/// two that mean *"the aperture is full"* indistinguishable from the two that mean
/// *"the plumbing is wrong"*, and those have opposite fixes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewRefusal {
    /// The isolate offered no worker — it is quiesced, retired, or never had one.
    NoWorker,
    /// RM refused to arm the view. Carries the backend's own sentence.
    ///
    /// ⚠ `NV_ERR_NO_MEMORY` (`0x51`) here means **the host's BAR1 aperture is full**, not
    /// that video memory ran out: `[measured w722]` 8192 MiB reserved fine in the same
    /// process that could not map 254 MiB.
    Rm(String),
    /// The view was armed and this process could not turn its token into a descriptor. The
    /// view is released before this is returned — leaking it would consume aperture for the
    /// rest of the boot with nothing able to see or release it.
    NoDescriptor { token: u64 },
    /// The host mapping itself refused. Carries the raw error's rendering.
    Mmap(String),
}

impl ViewRefusal {
    /// A short, stable token for a census — never the free text.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            ViewRefusal::NoWorker => "NO_WORKER",
            ViewRefusal::Rm(_) => "RM_REFUSED",
            ViewRefusal::NoDescriptor { .. } => "NO_DESCRIPTOR",
            ViewRefusal::Mmap(_) => "MMAP_REFUSED",
        }
    }
}

/// ★★★★★ **THE NAME OF AN ARMED VIEW**, as the port hands it out.
///
/// # ⊘⊘ Why the caller gets a NAME and not the [`ArmedView`] itself
///
/// The consumer of an armed view is `BarMirror`'s slot table, whose `Slot` is [`Copy`] and is
/// snapshotted by value under a lock in three places. An [`ArmedView`] is a **resource** — it
/// cannot be `Copy` and must not be, because two copies of one release is either a
/// double-release or a leak depending on which one runs.
///
/// ⇒ the port keeps the resources and hands out `u64`s. That also puts the leak accounting in
/// exactly one place: [`DeviceViewPort::census_line`]'s outstanding count is the port's own
/// map, not a subtraction of two counters that can each be right while their difference is a
/// fact about neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ViewId(pub u64);

/// ★★★★★ **AN ARMED VIEW OF THE RESERVED OBJECT** — the thing that costs host BAR1.
///
/// # ⊘⊘ It does NOT release itself, and that is deliberate
///
/// Releasing is [`kayfabe_isolate::Worker::release_device_view`], an IPC round trip that
/// asserts lock-free. A `Drop` that performed it would run wherever the value happened to go
/// out of scope — including under the plane lock, where it would panic, and on a vCPU, where
/// it would block one. ⇒ the release is an explicit verb ([`DeviceViewPort::release`]) and
/// this type's `Drop` only **complains**, loudly, once.
#[derive(Debug)]
#[must_use = "an armed view holds host BAR1 aperture until DeviceViewPort::release takes it \
              back; dropping it returns NOTHING to the pool (measured w722) and the failure \
              is silent until every later arm is refused"]
pub struct ArmedView {
    view: kayfabe_isolate::DeviceView,
    /// Set by [`DeviceViewPort::release`] so `Drop` can tell a released view from a leaked
    /// one. ⊘ A bool and not the absence of the value, because the view's fields stay
    /// readable for a census after the release.
    released: bool,
}

impl ArmedView {
    /// Where in the reserved object this view starts.
    #[must_use]
    pub fn offset(&self) -> u64 {
        self.view.offset
    }

    /// The page-rounded length the driver registered — what an `mmap` of the node must use.
    #[must_use]
    pub fn mmap_len(&self) -> u64 {
        self.view.mmap_len
    }
}

/// How many leak lines [`ArmedView::drop`] has printed. ⊘ Capped — see the `Drop` impl.
static LEAK_LINES: AtomicU64 = AtomicU64::new(0);

/// How many leak lines are worth printing before the count is the only useful part.
const LEAK_LINES_MAX: u64 = 3;

impl Drop for ArmedView {
    fn drop(&mut self) {
        // ⊘⊘ **CAPPED, and the cap is not tidiness.** Every view the port still holds at
        // process teardown arrives here at once — an ordinary shutdown with a hundred live
        // memslots would print a hundred alarming lines, which is a log telling the truth and
        // nobody reading it, and worse, it trains a reader to skip the line that matters. The
        // port's `outstanding=` is the number; this is the first few instances of it.
        if !self.released
            && !std::thread::panicking()
            && LEAK_LINES.fetch_add(1, Ordering::Relaxed) < LEAK_LINES_MAX
        {
            eprintln!(
                "kayfabe: DEVICE-VIEW ⊘⊘⊘ LEAKED a view at offset {:#x} len {:#x} — it was \
                 dropped without DeviceViewPort::release. `[measured w722]` munmap+close \
                 returns ZERO host BAR1 aperture; only NV_ESC_RM_UNMAP_MEMORY does. This \
                 aperture is gone for the life of the isolate and every later arm is that \
                 much closer to NV_ERR_NO_MEMORY — which is reported with ioctl(2) returning \
                 0 and errno 0, i.e. silently. ⚠ At most {LEAK_LINES_MAX} of these are \
                 printed; DEVICE-VIEW-PORT's `outstanding=` is the total.",
                self.view.offset, self.view.mmap_len,
            );
        }
    }
}

/// ★★★★★ **THE PORT.** One isolate, one reserved object, arms and releases views of it.
///
/// ⊘ `Debug` is written by hand rather than derived: the `dup` closure is a trait object and
/// has none, and a `Debug` that skipped it silently would be the shape this tree keeps
/// paying for. The field is named in the output as what it is.
pub struct DeviceViewPort {
    iso: std::sync::Arc<SharedIsolate>,
    id: IsolateId,
    /// The reserved object. ⊘ Captured once at construction rather than passed per call:
    /// *"a probe over a different object would prove a different thing"*, and so would an arm.
    obj: HostHandle,
    dup: std::sync::Arc<DupArc>,
    /// ★★★ **THE ARMED VIEWS THIS PORT IS HOLDING**, by the name it handed out.
    ///
    /// ⊘ The map IS the outstanding set: its length is the authoritative *"how much host
    /// BAR1 are we holding"*, where `armed - released` is an arithmetic that is wrong the
    /// moment a release is refused.
    views: std::sync::Mutex<std::collections::BTreeMap<u64, ArmedView>>,
    next_id: AtomicU64,
    armed: AtomicU64,
    released: AtomicU64,
    refused: AtomicU64,
    /// Releases naming an id this port does not hold. ⊘ Counted rather than panicked: it is
    /// a caller's bookkeeping bug, and it is ALSO what a leak looks like from the other side
    /// — so a non-zero here and a non-zero outstanding are read together.
    double_released: AtomicU64,
    bytes_armed: AtomicU64,
    arm_us: AtomicU64,
    rel_us: AtomicU64,
    /// The first refusal's name, packed as an index+1 so `0` means *"none"*. ⊘ A census that
    /// reports only a total cannot say WHICH of four causes fired, and the four have
    /// different fixes.
    first_refusal: std::sync::Mutex<Option<String>>,
}

impl DeviceViewPort {
    /// Build the port. ⊘ Not public beyond the crate's own wiring:
    /// [`crate::scratchpad::Scratchpad::share_for_device_views`] is the only constructor
    /// path, because it is the only place that knows the reservation is held.
    #[must_use]
    pub fn new(
        iso: std::sync::Arc<SharedIsolate>,
        id: IsolateId,
        obj: HostHandle,
        dup: std::sync::Arc<DupArc>,
    ) -> DeviceViewPort {
        DeviceViewPort {
            iso,
            id,
            obj,
            dup,
            views: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            // ⊘ Ids start at 1 so `0` is never a live view — a `Slot`'s `Option<u64>` that
            // lost its `Some` would otherwise name the first view ever armed.
            next_id: AtomicU64::new(1),
            armed: AtomicU64::new(0),
            released: AtomicU64::new(0),
            refused: AtomicU64::new(0),
            double_released: AtomicU64::new(0),
            bytes_armed: AtomicU64::new(0),
            arm_us: AtomicU64::new(0),
            rel_us: AtomicU64::new(0),
            first_refusal: std::sync::Mutex::new(None),
        }
    }

    /// ★★★★★ **ARM A VIEW AND DO ONE THING WITH ITS NODE**, then close the node.
    ///
    /// `f` is handed the `/dev/nvidia<N>` descriptor and the length the driver will accept.
    /// The **only** sanctioned thing to do with it is a mapping verb —
    /// `GuestWindow::place_device_view` or `QemuMachine::install_device_window`.
    ///
    /// On success the caller owns the returned [`ArmedView`] and **must** hand it back to
    /// [`DeviceViewPort::release`]. On `f` returning `Err`, the view is released here and the
    /// error is returned, because a caller that could not use the mapping has no reason to
    /// hold the aperture.
    ///
    /// # ⊘ Why a closure and not `-> OwnedFd`
    ///
    /// Condition 2 of the owner's ruling is *"closed the moment `mmap` returns"*. Handing an
    /// `OwnedFd` out makes that a property of every caller's control flow; handing a
    /// `BorrowedFd` into a closure makes it a property of **this function**, which is the
    /// only place it can be checked once.
    ///
    /// # Errors
    /// [`ViewRefusal`], by name. Nothing is left armed on any refusal path.
    ///
    /// # Panics
    /// Through `Worker::export_device_view`'s `assert_lock_free`, if a caller reaches this
    /// while holding a ranked lock. That is the invariant, not a bug to be caught.
    pub fn with_node<T, E>(
        &self,
        offset: u64,
        len: u64,
        write: bool,
        f: impl FnOnce(std::os::fd::BorrowedFd<'_>, u64) -> Result<T, E>,
    ) -> Result<Result<(T, ViewId), E>, ViewRefusal> {
        let t0 = std::time::Instant::now();
        let armed = self
            .iso
            .with_worker(|worker| worker.export_device_view(self.obj, offset, len, write));
        let view = match armed {
            None => return Err(self.note_refusal(ViewRefusal::NoWorker)),
            Some(Err(e)) => return Err(self.note_refusal(ViewRefusal::Rm(format!("{e:?}")))),
            Some(Ok(v)) => v,
        };
        self.arm_us
            .fetch_add(t0.elapsed().as_micros() as u64, Ordering::Relaxed);
        let mut held = ArmedView {
            view,
            released: false,
        };
        let Some(fd) = (self.dup)(self.id, held.view.token) else {
            let token = held.view.token;
            // ⊘ Released BEFORE the refusal is returned: the view exists in the isolate
            // whether or not this process can see it, and leaking it would consume aperture
            // for the rest of the boot with nothing able to name or release it.
            self.release_held(&mut held);
            return Err(self.note_refusal(ViewRefusal::NoDescriptor { token }));
        };
        let mmap_len = held.view.mmap_len;
        let out = f(fd.as_fd(), mmap_len);
        // ★★★ **CONDITION 2 OF THE OWNER'S RULING, AND IT IS THIS LINE.** The descriptor is
        // closed the instant the mapping exists — not at the end of the scope, not on the
        // success path only.
        drop(fd);
        match out {
            Ok(t) => {
                self.armed.fetch_add(1, Ordering::Relaxed);
                self.bytes_armed.fetch_add(mmap_len, Ordering::Relaxed);
                let id = self.next_id.fetch_add(1, Ordering::Relaxed);
                self.views
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(id, held);
                Ok(Ok((t, ViewId(id))))
            }
            Err(e) => {
                // ⊘ The caller could not use the mapping, so the aperture goes straight back.
                // Holding it would be a leak with a plausible-looking cause.
                self.release_held(&mut held);
                Ok(Err(e))
            }
        }
    }

    /// ★★★★★ **ARM A VIEW AND MAP IT INTO A HOST WINDOW OF THIS PROCESS.**
    ///
    /// The shape the **store** needs: a CPU mapping of a run of the reserved object that this
    /// process can `memcpy` through. The window and the [`ViewId`] must be kept together —
    /// dropping the window unmaps, and only [`DeviceViewPort::release`] returns the aperture.
    ///
    /// ⊘ `writable` is the VMA's protection — what **this process** may do — and is
    /// independent of any guest slot tier.
    ///
    /// # Errors
    /// [`ViewRefusal`], by name.
    ///
    /// # Panics
    /// As [`DeviceViewPort::with_node`].
    ///
    /// ⊘ `host-isolates` only: `GuestWindow` lives in `kayfabe-linux-raw`, which is the
    /// optional half of this crate. [`DeviceViewPort::with_node`] is unconditional, because
    /// the memslot path's mapping verb is the hypervisor's, not this one.
    #[cfg(feature = "host-isolates")]
    pub fn arm_mapped(
        &self,
        offset: u64,
        len: u64,
        writable: bool,
    ) -> Result<(kayfabe_linux_raw::GuestWindow, ViewId), ViewRefusal> {
        use kayfabe_linux_raw::{GuestWindow, HostOffset, HostPageSize};
        let out = self.with_node(offset, len, writable, |fd, mmap_len| {
            let win = GuestWindow::create(mmap_len, HostPageSize::query())
                .map_err(|e| format!("{e:?}"))?;
            win.place_device_view(HostOffset::ZERO, mmap_len, fd, writable)
                .map_err(|e| format!("{e:?}"))?;
            Ok(win)
        })?;
        out.map_err(|why| self.note_refusal(ViewRefusal::Mmap(why)))
    }

    /// ★★★★★ **GIVE THE APERTURE BACK.** The only thing that does.
    ///
    /// An id the port does not hold is a **no-op and is counted**, not a panic: a double
    /// release is a bookkeeping bug in a caller, and aborting the VMM over it would turn a
    /// leak-shaped defect into a guest-visible crash.
    ///
    /// # Panics
    /// Through `Worker::release_device_view`'s `assert_lock_free`.
    pub fn release(&self, id: ViewId) {
        let held = self
            .views
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&id.0);
        match held {
            Some(mut v) => self.release_held(&mut v),
            None => {
                self.double_released.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// How many armed views this port is holding right now — the authoritative host-BAR1
    /// outstanding count.
    #[must_use]
    pub fn outstanding(&self) -> usize {
        self.views
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// Release everything this port still holds, and say how many there were.
    ///
    /// ⊘ For teardown, and for a store that is being replaced. Not a `Drop`: releasing is an
    /// IPC round trip that asserts lock-free, and a `Drop` would perform it wherever the
    /// value happened to go out of scope.
    ///
    /// # Panics
    /// As [`DeviceViewPort::release`].
    pub fn release_all(&self) -> usize {
        let all: Vec<ArmedView> = {
            let mut g = self
                .views
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *g).into_values().collect()
        };
        let n = all.len();
        for mut v in all {
            self.release_held(&mut v);
        }
        n
    }

    fn release_held(&self, view: &mut ArmedView) {
        if view.released {
            return;
        }
        let t0 = std::time::Instant::now();
        let out = self
            .iso
            .with_worker(|worker| worker.release_device_view(&view.view));
        self.rel_us
            .fetch_add(t0.elapsed().as_micros() as u64, Ordering::Relaxed);
        match out {
            Some(Ok(())) => {
                view.released = true;
                self.released.fetch_add(1, Ordering::Relaxed);
            }
            // ⊘⊘ A refused release is NOT a released view, and the flag stays `false` so the
            // `Drop` still screams: the aperture is gone either way, and a counter that
            // called it released would make the leak invisible in exactly the census meant
            // to catch it.
            Some(Err(e)) => {
                self.note_refusal(ViewRefusal::Rm(format!("release: {e:?}")));
            }
            None => {
                self.note_refusal(ViewRefusal::NoWorker);
            }
        }
    }

    fn note_refusal(&self, why: ViewRefusal) -> ViewRefusal {
        self.refused.fetch_add(1, Ordering::Relaxed);
        let mut first = self
            .first_refusal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if first.is_none() {
            *first = Some(format!("{}: {why:?}", why.name()));
        }
        why
    }

    /// ★★★ **THE CENSUS LINE.** One line, and it carries the per-view tax by name.
    ///
    /// ⊘ `outstanding` is read from the port's own map, **never** as `armed - released`: a
    /// refused release increments neither, and the difference of two right numbers would be
    /// a fact about neither.
    #[must_use]
    pub fn census_line(&self) -> String {
        let armed = self.armed.load(Ordering::Relaxed);
        let released = self.released.load(Ordering::Relaxed);
        let refused = self.refused.load(Ordering::Relaxed);
        let outstanding = self.outstanding();
        let first = self
            .first_refusal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        format!(
            "DEVICE-VIEW-PORT armed={armed} released={released} outstanding={outstanding} \
             refused={refused} double_released={dr} bytes_armed={mib:.1}MiB \
             arm_us_total={au} rel_us_total={ru} first_refusal=[{first}] ⇒ {verdict}",
            dr = self.double_released.load(Ordering::Relaxed),
            mib = self.bytes_armed.load(Ordering::Relaxed) as f64 / (1024.0 * 1024.0),
            au = self.arm_us.load(Ordering::Relaxed),
            ru = self.rel_us.load(Ordering::Relaxed),
            first = first.unwrap_or_else(|| "none".to_string()),
            verdict = if armed == 0 && refused == 0 {
                "⊘⊘ VACUOUS — the port exists and NOTHING ever asked it for a view. That is \
                 not `no views were needed`; it is an unmeasured port."
            } else if refused > 0 {
                "⚠ at least one arm or release was REFUSED — read first_refusal before \
                 anything else, because NV_ERR_NO_MEMORY here means the host BAR1 APERTURE \
                 is full, not that video memory ran out"
            } else {
                "★ every arm and every release succeeded"
            },
        )
    }
}

impl core::fmt::Debug for DeviceViewPort {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DeviceViewPort")
            .field("isolate", &self.id)
            .field("object", &self.obj)
            .field("dup", &"<closure: token → descriptor in this process>")
            .field("armed", &self.armed.load(Ordering::Relaxed))
            .field("released", &self.released.load(Ordering::Relaxed))
            .field("refused", &self.refused.load(Ordering::Relaxed))
            .finish()
    }
}

#[cfg(feature = "host-isolates")]
mod byteport {
    use super::{DeviceViewPort, ViewId};
    use std::sync::atomic::{AtomicU64, Ordering};

    // ═══════════════════════════════════════════════════════════════════════════════════════
    // ★★★★★ §3 CUT B — THE STORE'S BYTE PORT, OVER THE DEVICE-VIEW PORT.
    // ═══════════════════════════════════════════════════════════════════════════════════════

    /// ★★★ **How much of the reserved object one arm covers.** 64 KiB.
    ///
    /// # ⊘ Why not one page
    ///
    /// An arm is an **IPC round trip** — `[measured w736]` mean **382 µs**, across nine live arms
    /// — and a page-table walk reads its pages 4 KiB at a time. Arming per page would pay that
    /// round trip once per page; arming a 64 KiB run pays it once per sixteen, and page-table
    /// pages are exactly the thing a guest's own allocator clusters.
    ///
    /// ⚠ **And it is not free in the direction that matters.** Host BAR1 is the scarce resource
    /// (`THE_CONSTRAINTS.md` §22 item 3: ~254 MiB usable, ONE global pool shared with our own CUDA
    /// context), so a 16× coarser grain is a 16× larger aperture bill for the same working set.
    /// `[measured w734]` the walk touches **128 distinct frames** ⇒ 0.5 MiB at page grain, at most
    /// 8 MiB at this one — **3.1 % of the pool** against a measured 1.4 % for BAR1 itself. That is
    /// the whole of the argument, and it is a measurement rather than a feeling.
    ///
    /// ⊘ It is **not** a per-die fact and nothing may derive a chip property from it
    /// (constraint 12): it is a cost trade between two measured numbers on this host.
    pub const ARM_GRAIN: u64 = 64 * 1024;

    /// ★★★ **The BUDGET — how many runs may be armed at once.** 256 × 64 KiB = **16 MiB**.
    ///
    /// `THE_CONSTRAINTS.md` §22, *"what is actually left"* item 1: *"**Enforce a budget.** A guest
    /// *may* legally fill its aperture, and nothing stops it. Left unbounded it can starve our
    /// CUDA context and with it the walker. ⇒ A **checked** bound, not an assumed one."*
    ///
    /// ⊘ At the cap the oldest run is **released and re-used**, never refused: a refusal here
    /// would wedge a boot on a guest that is behaving legally, and the release verb
    /// (`NV_ESC_RM_UNMAP_MEMORY`) is exactly what makes recycling possible at all. `[measured
    /// w722]` `munmap` + `close` returns **nothing** to the pool, which is why eviction goes
    /// through [`DeviceViewPort::release`] and not through dropping the window.
    pub const ARMED_RUNS_CAP: usize = 256;

    /// How many wanted-but-unarmed runs the demand set may hold. ⊘ A bound and not a hope: the
    /// set is written **under the plane lock** by whatever the guest's own page tables made us
    /// read, so an unbounded one is a guest-driven host allocation — the exact amplification
    /// shape `THE_CONSTRAINTS.md` §w720h says the single store deletes.
    pub const WANT_SET_CAP: usize = 4096;

    /// How many runs one [`DeviceFbPort::drain`] may arm. ⊘ A **fixed trip count**: a drain runs
    /// on a worker the rest of the device is waiting on, and 64 × 382 µs ≈ 24 ms is already the
    /// most a single pass should hold that thread for. What it cannot finish stays wanted and is
    /// reported as `deferred`, which is a number rather than a silent truncation.
    pub const DRAIN_ARMS_MAX: u32 = 64;

    /// The largest host-side access this port will try to serve across armed runs, in runs.
    /// ⊘ `decode_page` reads a whole page-table page — 512 × 16 B = **8 KiB** at the widest
    /// format — so two runs is already generous; the bound exists so the loop below has a fixed
    /// trip count regardless of what a caller asks for.
    const MAX_SPAN_RUNS: u64 = 4;

    /// One armed, mapped run of the reserved object.
    #[derive(Debug)]
    struct ArmedRun {
        /// The mapping. ⊘ Dropping it `munmap`s and returns **nothing** to the BAR1 pool; the
        /// aperture comes back only through [`DeviceViewPort::release`], which is why the
        /// [`ViewId`] travels beside it and never apart from it.
        win: kayfabe_linux_raw::GuestWindow,
        /// The port's name for the armed view.
        view: ViewId,
        /// How long the run is, as the driver accepted it — never as we asked for it.
        len: u64,
    }

    /// ★★★★★ **CUT B item 1 — BYTES OF THE RESERVED OBJECT, FOR A STORE THAT HOLDS NO DESCRIPTOR.**
    ///
    /// [`kayfabe_device::DeviceFb`] names a framebuffer page as an **address** in the one reserved
    /// object and can go no further: `read`/`write`/`page_backing` all run under the plane lock
    /// and arming is an IPC round trip that asserts lock-free. This is the other half — a table of
    /// armed runs that a locked caller may `memcpy` through, and a demand set that a **lock-free**
    /// caller drains.
    ///
    /// # ⊘⊘⊘ THE LOCK DISCIPLINE, WHICH IS THE WHOLE OF THE SAFETY ARGUMENT
    ///
    /// | this type's mutex | held across | why |
    /// |---|---|---|
    /// | `runs` | a `memcpy` | ⊘ never an IPC round trip. An arm or a release performed under it would block a vCPU that is reading under the plane lock — the same stall through a lock instead of through a socket |
    /// | `want` | an insert | nothing |
    ///
    /// ⇒ every arm and every release below happens with **both** mutexes released, and the map is
    /// re-entered only to publish or to take a victim. That is why eviction removes the victim
    /// from the map **first** and releases it afterwards.
    pub struct DeviceFbBytePort {
        port: std::sync::Arc<DeviceViewPort>,
        /// How long the reserved object is. ⊘ Carried rather than asked of the port, because an
        /// arm past the end of the object is refused by RM with a status that reads like every
        /// other refusal — and *"the guest named framebuffer that was never reserved"* is the
        /// IDENTITY-WINDOW finding, not an aperture one.
        obj_len: u64,
        /// Armed runs, keyed by their base in the object. Non-overlapping by construction: every
        /// key is [`ARM_GRAIN`]-aligned and every run is one grain long.
        runs: std::sync::Mutex<std::collections::BTreeMap<u64, ArmedRun>>,
        /// Arm order, for the cap's eviction. ⊘ FIFO and not LRU: an LRU needs a write on every
        /// **read**, i.e. under the plane lock on a vCPU, to buy an eviction policy nothing has
        /// measured a need for.
        order: std::sync::Mutex<std::collections::VecDeque<u64>>,
        /// The demand set — runs wanted and not armed.
        want: std::sync::Mutex<std::collections::BTreeSet<u64>>,
        served_read: AtomicU64,
        served_write: AtomicU64,
        wanted_read: AtomicU64,
        wanted_write: AtomicU64,
        want_dropped: AtomicU64,
        drains: AtomicU64,
        declined: AtomicU64,
        armed: AtomicU64,
        arm_refused: AtomicU64,
        evicted: AtomicU64,
        outside_object: AtomicU64,
        span_too_wide: AtomicU64,
        /// Arms refused because the budget was full and no run could be evicted. ⊘ Counted
        /// apart from an RM refusal: one is OUR bound and the other is the host's aperture,
        /// and reading them as one would send somebody to measure the wrong pool.
        budget_refused: AtomicU64,
        /// ★ w743 — the pre-flight's own counters, kept APART from `wanted_read`/`served_read`.
        /// ⊘ A probe that misses also calls `want`, so it bumps `wanted_*` too; without these
        /// three there would be no way to tell a demand raised by an *attempted access* from one
        /// raised by a *question*, and the whole of w743's claim is about which came first.
        probes: AtomicU64,
        probe_ready: AtomicU64,
        probe_missed: AtomicU64,
        probe_unservable: AtomicU64,
        first_arm_refusal: std::sync::Mutex<Option<String>>,
    }

    impl core::fmt::Debug for DeviceFbBytePort {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("DeviceFbBytePort")
                .field("obj_len", &self.obj_len)
                .field("armed", &self.armed.load(Ordering::Relaxed))
                .field("evicted", &self.evicted.load(Ordering::Relaxed))
                .field("served_read", &self.served_read.load(Ordering::Relaxed))
                .finish_non_exhaustive()
        }
    }

    impl DeviceFbBytePort {
        /// Build the byte port over an armed device-view port and a reservation of `obj_len`
        /// bytes.
        #[must_use]
        pub fn new(port: std::sync::Arc<DeviceViewPort>, obj_len: u64) -> DeviceFbBytePort {
            DeviceFbBytePort {
                port,
                obj_len,
                runs: std::sync::Mutex::new(std::collections::BTreeMap::new()),
                order: std::sync::Mutex::new(std::collections::VecDeque::new()),
                want: std::sync::Mutex::new(std::collections::BTreeSet::new()),
                served_read: AtomicU64::new(0),
                served_write: AtomicU64::new(0),
                wanted_read: AtomicU64::new(0),
                wanted_write: AtomicU64::new(0),
                want_dropped: AtomicU64::new(0),
                drains: AtomicU64::new(0),
                declined: AtomicU64::new(0),
                armed: AtomicU64::new(0),
                arm_refused: AtomicU64::new(0),
                evicted: AtomicU64::new(0),
                outside_object: AtomicU64::new(0),
                span_too_wide: AtomicU64::new(0),
                budget_refused: AtomicU64::new(0),
                probes: AtomicU64::new(0),
                probe_ready: AtomicU64::new(0),
                probe_missed: AtomicU64::new(0),
                probe_unservable: AtomicU64::new(0),
                first_arm_refusal: std::sync::Mutex::new(None),
            }
        }

        /// The `[first, last]` run bases an access of `len` bytes at `at` touches, or `None` when
        /// it is out of the object or spans more runs than [`MAX_SPAN_RUNS`].
        fn span(&self, at: u64, len: u64) -> Option<(u64, u64)> {
            if len == 0 {
                return None;
            }
            let end = at.checked_add(len)?;
            if end > self.obj_len {
                self.outside_object.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            let first = at & !(ARM_GRAIN - 1);
            let last = (end - 1) & !(ARM_GRAIN - 1);
            if (last - first) / ARM_GRAIN >= MAX_SPAN_RUNS {
                self.span_too_wide.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            Some((first, last))
        }

        /// Release one run's aperture. ⊘ Called with **no** mutex of this type held: it is an IPC
        /// round trip.
        fn release_run(&self, run: ArmedRun) {
            let ArmedRun { win, view, .. } = run;
            // ★★★ ORDER: the port's release returns the host BAR1 aperture, and the `munmap` is
            // the `Drop` of `win` on the line after. ⊘ `osUnmapPciMemoryUser` is an EMPTY function
            // in RM (`ogkm os.c:1275-1282`), so `NV_ESC_RM_UNMAP_MEMORY` does not touch the VMA —
            // which is exactly why this process must keep the mapping alive until it has finished
            // with it and drop it itself. Nothing else may use these bytes in between: the run is
            // already out of `runs`, so no reader can find it.
            self.port.release(view);
            drop(win);
            self.evicted.fetch_add(1, Ordering::Relaxed);
        }
    }

    impl kayfabe_device::DeviceFbPort for DeviceFbBytePort {
        fn read_armed(&self, at: u64, buf: &mut [u8]) -> bool {
            let Some((first, last)) = self.span(at, buf.len() as u64) else {
                return false;
            };
            let runs = self
                .runs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // ⊘⊘ **CHECKED WHOLE BEFORE ANY BYTE MOVES.** A partial fill would leave the caller a
            // buffer that is half this page and half whatever it held — and `FbRead::read_in`
            // answers a `bool`, so nothing downstream could tell. ⇒ all-or-nothing, decided before
            // the first copy.
            let mut base = first;
            // ★ A fixed trip count: `span` refused anything wider than `MAX_SPAN_RUNS`.
            while base <= last {
                let Some(run) = runs.get(&base) else {
                    return false;
                };
                // ⊘ The run's length is the DRIVER's, page-rounded, and may differ from what we
                // asked for. Checked here rather than assumed, because a short run would make the
                // copy below read past the mapping.
                let want_end = (at + buf.len() as u64).min(base + ARM_GRAIN);
                if want_end > base + run.len {
                    return false;
                }
                base += ARM_GRAIN;
            }
            let mut done = 0usize;
            let mut base = first;
            while base <= last {
                let run = match runs.get(&base) {
                    Some(r) => r,
                    // Unreachable: the loop above proved every key present under this same guard.
                    None => return false,
                };
                let start = at.max(base);
                let stop = (at + buf.len() as u64).min(base + ARM_GRAIN);
                let n = (stop - start) as usize;
                if run
                    .win
                    .read_into(
                        kayfabe_linux_raw::HostOffset::new(start - base),
                        &mut buf[done..done + n],
                    )
                    .is_err()
                {
                    return false;
                }
                done += n;
                base += ARM_GRAIN;
            }
            let ok = done == buf.len();
            if ok {
                self.served_read.fetch_add(1, Ordering::Relaxed);
            }
            ok
        }

        fn write_armed(&self, at: u64, bytes: &[u8]) -> bool {
            let Some((first, last)) = self.span(at, bytes.len() as u64) else {
                return false;
            };
            let runs = self
                .runs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut base = first;
            while base <= last {
                let Some(run) = runs.get(&base) else {
                    return false;
                };
                let want_end = (at + bytes.len() as u64).min(base + ARM_GRAIN);
                if want_end > base + run.len {
                    return false;
                }
                base += ARM_GRAIN;
            }
            let mut done = 0usize;
            let mut base = first;
            while base <= last {
                let Some(run) = runs.get(&base) else {
                    return false;
                };
                let start = at.max(base);
                let stop = (at + bytes.len() as u64).min(base + ARM_GRAIN);
                let n = (stop - start) as usize;
                if run
                    .win
                    .write_from(
                        kayfabe_linux_raw::HostOffset::new(start - base),
                        &bytes[done..done + n],
                    )
                    .is_err()
                {
                    // ⚠ A partial write is possible here and is NOT silently forgiven: `false`
                    // makes the store refuse by name, which is the only honest answer — the run
                    // is real memory and some of it may already have changed.
                    return false;
                }
                done += n;
                base += ARM_GRAIN;
            }
            let ok = done == bytes.len();
            if ok {
                self.served_write.fetch_add(1, Ordering::Relaxed);
            }
            ok
        }

        fn probe_or_want(&self, at: u64, len: u64, by: kayfabe_device::DeviceFbWant) -> bool {
            self.probes.fetch_add(1, Ordering::Relaxed);
            // ⊘ An access this port can NEVER serve — outside the reserved object, or wider
            // than `MAX_SPAN_RUNS` — is not a want, for `want`'s own reason: putting it in
            // the set would make every later drain spend an IPC round trip rediscovering it.
            // It is still `false`, because it is still not servable, and `span`'s own
            // `outside_object` / `span_too_wide` counters are what name which.
            let Some((first, last)) = self.span(at, len.max(1)) else {
                self.probe_unservable.fetch_add(1, Ordering::Relaxed);
                return false;
            };
            {
                let runs = self
                    .runs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let mut base = first;
                let mut all = true;
                // ★ A fixed trip count: `span` refused anything wider than `MAX_SPAN_RUNS`.
                while base <= last {
                    match runs.get(&base) {
                        // ⊘ The run's length is the DRIVER's, page-rounded, and may be
                        // SHORT of the grain. Checked with exactly `read_armed`'s test, so
                        // a `true` here and a served access cannot disagree.
                        Some(run) => {
                            let want_end = (at + len.max(1)).min(base + ARM_GRAIN);
                            if want_end > base + run.len {
                                all = false;
                                break;
                            }
                        }
                        None => {
                            all = false;
                            break;
                        }
                    }
                    base += ARM_GRAIN;
                }
                if all {
                    self.probe_ready.fetch_add(1, Ordering::Relaxed);
                    return true;
                }
            }
            // ⚠ The `runs` lock is DROPPED above before this: `want` takes the `want` mutex,
            // and holding two of this type's mutexes at once is the one ordering fact the
            // type's own table forbids.
            self.probe_missed.fetch_add(1, Ordering::Relaxed);
            kayfabe_device::DeviceFbPort::want(self, at, len, by);
            false
        }

        fn want(&self, at: u64, len: u64, by: kayfabe_device::DeviceFbWant) {
            match by {
                kayfabe_device::DeviceFbWant::Read => &self.wanted_read,
                kayfabe_device::DeviceFbWant::Write => &self.wanted_write,
            }
            .fetch_add(1, Ordering::Relaxed);
            // ⊘ The span refusal is NOT a want: an access outside the reserved object cannot be
            // armed however many times it is asked for, and putting it in the set would make every
            // later drain spend an IPC round trip discovering that again.
            let Some((first, last)) = self.span(at, len.max(1)) else {
                return;
            };
            let mut w = self
                .want
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut base = first;
            while base <= last {
                if w.len() >= WANT_SET_CAP {
                    // ⊘ DROPPED, and counted. A dropped want is a page that keeps missing, never a
                    // wrong value — the same contract `BarMirror::fill`'s full queue has.
                    self.want_dropped.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                w.insert(base);
                base += ARM_GRAIN;
            }
        }

        fn drain(&self) -> kayfabe_device::DeviceFbDrained {
            use kayfabe_device::DeviceFbDrained;
            // ★★★ **CUT B item 5 — DECLINE BY NAME**, exactly as `WalkShadowDecider::decide` does,
            // and asked FIRST so nothing is locked on the way to finding out.
            if kayfabe_util::lockwitness::on_vcpu_thread() || kayfabe_util::trapwitness::in_trap() {
                self.declined.fetch_add(1, Ordering::Relaxed);
                return DeviceFbDrained::declined();
            }
            self.drains.fetch_add(1, Ordering::Relaxed);
            // ⊘ Taken out of the set under the lock and armed OUTSIDE it: arming is an IPC round
            // trip, and a `want` call from a vCPU under the plane lock must never wait on it.
            let batch: Vec<u64> = {
                let mut w = self
                    .want
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let take: Vec<u64> = w.iter().take(DRAIN_ARMS_MAX as usize).copied().collect();
                for k in &take {
                    w.remove(k);
                }
                take
            };
            let mut out = DeviceFbDrained::default();
            for base in batch {
                // Already armed by another drain between the want and here — free, and not a
                // refusal.
                if self
                    .runs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .contains_key(&base)
                {
                    continue;
                }
                // ★★★ THE BUDGET, ENFORCED — §22's *"a checked bound, not an assumed one"*. The
                // victim leaves the map under the lock and is released with the lock DROPPED.
                let at_cap;
                let victim = {
                    let mut runs = self
                        .runs
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    at_cap = runs.len() >= ARMED_RUNS_CAP;
                    if at_cap {
                        let mut order = self
                            .order
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        // ★ A fixed trip count: the queue is at most `ARMED_RUNS_CAP` long and
                        // every iteration pops one entry.
                        let mut found = None;
                        for _ in 0..=ARMED_RUNS_CAP {
                            match order.pop_front() {
                                Some(k) => {
                                    if let Some(r) = runs.remove(&k) {
                                        found = Some(r);
                                        break;
                                    }
                                }
                                None => break,
                            }
                        }
                        found
                    } else {
                        None
                    }
                };
                match victim {
                    Some(v) => self.release_run(v),
                    None if at_cap => {
                        // ⊘⊘ **THE BUDGET IS CHECKED, NOT HOPED FOR** (§22, *"a checked bound,
                        // not an assumed one"*). Reaching here means the cap is full and the
                        // arm-order queue could not name a victim — the two structures having
                        // drifted apart, which nothing should be able to do. ⇒ REFUSED rather
                        // than armed anyway: an arm past the cap is host BAR1 aperture this
                        // port would then never release, and `[measured w722]` that failure is
                        // silent until every later arm returns zero.
                        self.budget_refused.fetch_add(1, Ordering::Relaxed);
                        out.refused += 1;
                        continue;
                    }
                    None => {}
                }
                let len = ARM_GRAIN.min(self.obj_len.saturating_sub(base));
                if len == 0 {
                    self.outside_object.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                // ⚠ `writable: true` — the VMA's protection, i.e. what THIS process may do. The
                // store serves host writes through the same run (`write_armed`), and a read-only
                // mapping would refuse them with an `EACCES` that reads as a plumbing fault.
                match self.port.arm_mapped(base, len, true) {
                    Ok((win, view)) => {
                        let run_len = win.len_bytes();
                        let mut runs = self
                            .runs
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        runs.insert(
                            base,
                            ArmedRun {
                                win,
                                view,
                                len: run_len,
                            },
                        );
                        self.order
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push_back(base);
                        self.armed.fetch_add(1, Ordering::Relaxed);
                        out.armed += 1;
                    }
                    Err(why) => {
                        self.arm_refused.fetch_add(1, Ordering::Relaxed);
                        let mut first = self
                            .first_arm_refusal
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if first.is_none() {
                            *first = Some(format!("{}: {why:?} at 0x{base:x}", why.name()));
                        }
                        out.refused += 1;
                    }
                }
            }
            out.deferred = u32::try_from(
                self.want
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .len(),
            )
            .unwrap_or(u32::MAX);
            out
        }

        fn census_line(&self) -> String {
            let armed = self.armed.load(Ordering::Relaxed);
            let refused = self.arm_refused.load(Ordering::Relaxed);
            let declined = self.declined.load(Ordering::Relaxed);
            let drains = self.drains.load(Ordering::Relaxed);
            let sr = self.served_read.load(Ordering::Relaxed);
            let sw = self.served_write.load(Ordering::Relaxed);
            let first = self
                .first_arm_refusal
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            let verdict = if drains == 0 && declined == 0 {
                "⊘⊘ VACUOUS — the byte port was installed and NOTHING ever drained it. That is not \
                 `no bytes were wanted`; it is an unmeasured port, and the first place to look is \
                 whether any lock-free caller calls `RegPlane::arm_fb_demand` at all."
            } else if armed == 0 && declined > 0 {
                "⊘⊘⊘ EVERY DRAIN WAS DECLINED — the demand is arriving on vCPU threads and nothing \
                 off-vCPU ever drains. The retry is at the wrong caller."
            } else if refused > 0 {
                "⚠ AT LEAST ONE ARM WAS REFUSED — read first_arm_refusal: NV_ERR_NO_MEMORY here is \
                 the HOST BAR1 APERTURE being full, not video memory, and it is shared with our \
                 own CUDA context."
            } else if sr + sw == 0 {
                "⊘ RUNS WERE ARMED AND NOTHING WAS EVER SERVED THROUGH THEM — the arming and the \
                 serving disagree about an address, which is a defect here and not in the store."
            } else {
                "★ every arm succeeded and armed runs served host-side accesses"
            };
            format!(
                "DEVICE-FB-PORT drains={drains} armed={armed} arm_refused={refused} \
                 declined_on_vcpu={declined} evicted={ev} outstanding={out} served_read={sr} \
                 served_write={sw} wanted_read={wr} wanted_write={ww} want_dropped={wd} \
                 still_wanted={sw2} outside_object={oo} span_too_wide={stw} \
                 budget_refused={br} probes={pr} probe_ready={prr} probe_missed={prm} \
                 probe_unservable={pru} first_arm_refusal=[{first}] ⇒ {verdict}",
                br = self.budget_refused.load(Ordering::Relaxed),
                pr = self.probes.load(Ordering::Relaxed),
                prr = self.probe_ready.load(Ordering::Relaxed),
                prm = self.probe_missed.load(Ordering::Relaxed),
                pru = self.probe_unservable.load(Ordering::Relaxed),
                ev = self.evicted.load(Ordering::Relaxed),
                out = self
                    .runs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .len(),
                wr = self.wanted_read.load(Ordering::Relaxed),
                ww = self.wanted_write.load(Ordering::Relaxed),
                wd = self.want_dropped.load(Ordering::Relaxed),
                sw2 = self
                    .want
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .len(),
                oo = self.outside_object.load(Ordering::Relaxed),
                stw = self.span_too_wide.load(Ordering::Relaxed),
                first = first.unwrap_or_else(|| "none".to_string()),
            )
        }
    }

    /// ⊘⊘⊘ **THERE IS DELIBERATELY NO `Drop` HERE, AND THE REASON IS A PANIC.**
    ///
    /// The obvious teardown — release every run in `Drop` — is an **IPC round trip performed
    /// wherever the value happens to go out of scope**, and this value lives inside
    /// `kayfabe_device::DeviceFb`, inside `PlaneMem`, **behind the plane's mutex**. A
    /// `RegPlane::set_fb` that replaced the store would drop the last `Arc` *under that lock*, and
    /// `Worker::release_device_view`'s `assert_lock_free` would abort the VMM.
    ///
    /// ⇒ the aperture comes back through [`DeviceViewPort::release_all`], which the composition
    /// root owns and can call from a lock-free teardown — the same argument [`ArmedView`]'s own
    /// `Drop` makes one type over, for the same reason.
    const _: () = ();
}

#[cfg(feature = "host-isolates")]
pub use byteport::{ARMED_RUNS_CAP, ARM_GRAIN, DRAIN_ARMS_MAX, DeviceFbBytePort, WANT_SET_CAP};

// ═══════════════════════════════════════════════════════════════════════════════════════
// ★★★★★ §3's GATE — which store backs the guest's video memory.
// ═══════════════════════════════════════════════════════════════════════════════════════

/// ★★★★★ **THE SWITCH.** `arena` (the default) | `device`.
///
/// | value | what backs a guest framebuffer page |
/// |---|---|
/// | unset / `arena` | **the default.** A sparse host memfd. Byte for byte what shipped. |
/// | `device` | the one reserved device-local RM object — `THE_CONSTRAINTS.md` §22's *"if the guest says this is now in vidmem, its in vidmem"*. |
///
/// # ⊘ Why a gate at all, when the design's answer is `device`
///
/// `SINGLE_STORE_PLAN.md`'s sequencing rule: *"If deletion rides along, the tree is broken
/// across a long stretch with **no working intermediate and no way to bisect** which half
/// broke the raw client."* The switch is the same hazard one increment earlier. With the gate
/// defaulting to `arena`, every commit on the way keeps a working tree and a red bisects to
/// one cut rather than to "the switch".
///
/// ⊘ A value naming neither arm is **refused**, not defaulted — both directions of a silent
/// default are wrong and they are wrong in opposite ways.
///
/// # ★★★ THIS GATE'S EXPIRY CONDITION (§w724g), stated where the rule requires it
///
/// > **Deleted when the GUEST SUITE — all 30 arms, not one workload — passes on `device`.**
/// > At that point `arena` is the dead half and `SparseFb`, the page arena, the join and the
/// > demand-fill mirror go with it (`SINGLE_STORE_PLAN.md` §7, *"the deletions — only now"*).
///
/// ⊘ It is **not** deleted because a boot came back clean, and not because the raw client
/// grades `(P)`: §w727's *"the minimum is a measurement, and it is one workload"* applies to
/// this gate exactly as it does to `BAR1_MIN`.
pub const FB_STORE_ENV: &str = "KAYFABE_FB_STORE";

/// Which store this boot installs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FbStoreArm {
    /// A sparse host memfd. The shipped behaviour.
    Arena,
    /// The one reserved device-local object.
    Device,
}

impl FbStoreArm {
    /// The value as written in the environment.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            FbStoreArm::Arena => "arena",
            FbStoreArm::Device => "device",
        }
    }

    /// Whether this arm serves guest video memory out of the reserved object.
    #[must_use]
    pub fn is_device(self) -> bool {
        matches!(self, FbStoreArm::Device)
    }
}

/// Which arm `value` names — the pure half, and the only statement of the default.
///
/// # Errors
/// [`kayfabe_isolate::Status`]-shaped refusal text if `value` names neither arm. **Absent is
/// not an error**; it is [`FbStoreArm::Arena`].
pub fn fb_store_from(value: Option<&str>) -> Result<FbStoreArm, &'static str> {
    match value {
        None | Some("arena") => Ok(FbStoreArm::Arena),
        Some("device") => Ok(FbStoreArm::Device),
        Some(_) => Err(
            "KAYFABE_FB_STORE does not name a store: the only values are `arena` (the \
             default, a sparse host memfd) and `device` (the one reserved device-local RM \
             object). It is not defaulted, because both directions of a typo are wrong in \
             opposite ways: defaulted to `arena` it runs the control arm on a boot the \
             operator believes is armed, and defaulted to `device` it puts the guest's whole \
             framebuffer on a path whose host-side half is not built yet.",
        ),
    }
}

/// Which arm this boot runs.
///
/// # Errors
/// Whatever [`fb_store_from`] refused with.
pub fn selected_fb_store() -> Result<FbStoreArm, &'static str> {
    let raw = std::env::var_os(FB_STORE_ENV);
    let value = raw
        .as_ref()
        .map(|v| v.to_str().unwrap_or("\u{fffd}invalid"));
    fb_store_from(value)
}

/// ★★★★★ **THE PRECONDITIONS OF THE `device` ARM, CHECKED AT REALIZE AND REFUSED BY NAME.**
///
/// `Ok(())` on the `arena` arm whatever the rest of the configuration is.
///
/// # ⊘ Why each of these refuses rather than degrades
///
/// §w727's third rule: *"**Refuse at startup, loudly, never silently clamp.** A guest booted
/// with a BAR too small for its driver fails somewhere unrecognisable."* Each precondition
/// below, unmet, produces a boot that fails later and elsewhere:
///
/// 1. **No device-view port.** Every page the store names would be refused `NO-DEVICE-PORT`
///    by the mirror, one per access, and the boot would look like a translation failure.
/// 2. **`defer_reval` off.** `BarMirror::fill` runs `fill_now` **synchronously on the vCPU**
///    on that arm (`!self.defer_reval || !on_vcpu_thread()`), and `fill_now`'s device branch
///    is an IPC round trip to the isolate. `assert_lock_free` would not catch it —
///    `assert_not_on_vcpu` only *reports* unless `KAYFABE_VCPU_BLOCK_FATAL` is set — so the
///    vCPU would simply block, silently, inside an MMIO exit. ⊘ That is constraint 4
///    violated by construction, and it would be measured as latency rather than named.
///
/// # Errors
/// The refusal text, when `device` is armed and a precondition is not met.
pub fn enforce_device_store(
    arm: FbStoreArm,
    have_port: bool,
    defer_reval: bool,
) -> Result<(), &'static str> {
    if !arm.is_device() {
        return Ok(());
    }
    if !have_port {
        return Err(
            "KAYFABE_FB_STORE=device and there is NO DEVICE-VIEW PORT. The single store names \
             every framebuffer page as an address in the reserved object, and only the port \
             can arm a CPU view of one — so every guest memslot would be refused \
             `NO-DEVICE-PORT` and the boot would look like a translation failure. Set \
             KAYFABE_SCRATCHPAD=on and KAYFABE_DEVICE_VIEW=probe, and check the SCRATCHPAD \
             census line above for which step refused.",
        );
    }
    if !defer_reval {
        return Err(
            "KAYFABE_FB_STORE=device with the deferred revalidation arm OFF. On that arm \
             `BarMirror::fill` runs `fill_now` synchronously ON THE vCPU, and the single \
             store's install step is an IPC round trip to the scratchpad isolate — a vCPU \
             blocked inside an MMIO exit, which `assert_not_on_vcpu` only REPORTS. Refused \
             here rather than measured as latency later.",
        );
    }
    Ok(())
}

/// ★★★★★ **WHICH BACKING A FRAMEBUFFER APERTURE GETS — the rule, as a pure function.**
///
/// # ⊘⊘⊘ THE DEFECT THIS EXISTS TO MAKE UNREPRESENTABLE — found in review, w735
///
/// The device-view **port** and the single **store** are two different gates:
/// [`crate::scratchpad::DEVICE_VIEW_ENV`] arms the first, [`FB_STORE_ENV`] the second. A boot
/// may legitimately run the port armed with the default `arena` store — that is exactly what
/// w734's census boot did.
///
/// ⇒ any site that chooses a backing by asking *"is there a port?"* puts **that** aperture on
/// the reserved object while every other framebuffer path serves the arena memfd: **two
/// memories for one address**, silently, on the *control* arm. `BarMirror::install_pramin_window`
/// was written that way and caught in review before it booted.
///
/// ⇒ the rule is written once, here, and the `&&` is the whole of it: **the store decides, and
/// the port is only the ability to act on that decision.**
#[must_use]
pub fn backing_is_device(store: FbStoreArm, have_port: bool) -> bool {
    store.is_device() && have_port
}
