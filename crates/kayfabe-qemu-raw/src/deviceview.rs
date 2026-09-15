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

impl Drop for ArmedView {
    fn drop(&mut self) {
        if !self.released && !std::thread::panicking() {
            eprintln!(
                "kayfabe: DEVICE-VIEW ⊘⊘⊘ LEAKED a view at offset {:#x} len {:#x} — it was \
                 dropped without DeviceViewPort::release. `[measured w722]` munmap+close \
                 returns ZERO host BAR1 aperture; only NV_ESC_RM_UNMAP_MEMORY does. This \
                 aperture is gone for the life of the isolate and every later arm is that \
                 much closer to NV_ERR_NO_MEMORY — which is reported with ioctl(2) returning \
                 0 and errno 0, i.e. silently.",
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
