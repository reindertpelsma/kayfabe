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
    armed: AtomicU64,
    released: AtomicU64,
    refused: AtomicU64,
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
            armed: AtomicU64::new(0),
            released: AtomicU64::new(0),
            refused: AtomicU64::new(0),
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
    ) -> Result<Result<(T, ArmedView), (E, ())>, ViewRefusal> {
        let t0 = std::time::Instant::now();
        let armed = self.iso.with_worker(|worker| {
            worker.export_device_view(self.obj, offset, len, write)
        });
        let view = match armed {
            None => return Err(self.note_refusal(ViewRefusal::NoWorker)),
            Some(Err(e)) => return Err(self.note_refusal(ViewRefusal::Rm(format!("{e:?}")))),
            Some(Ok(v)) => v,
        };
        self.arm_us
            .fetch_add(t0.elapsed().as_micros() as u64, Ordering::Relaxed);
        let Some(fd) = (self.dup)(self.id, view.token) else {
            let token = view.token;
            self.release_inner(ArmedView {
                view,
                released: false,
            });
            return Err(self.note_refusal(ViewRefusal::NoDescriptor { token }));
        };
        let mmap_len = view.mmap_len;
        let out = f(fd.as_fd(), mmap_len);
        // ★★★ **CONDITION 2, and it is this line.** The descriptor is closed the instant the
        // mapping exists — not at the end of the scope, not on the success path only.
        drop(fd);
        let mut armed = ArmedView {
            view,
            released: false,
        };
        match out {
            Ok(t) => {
                self.armed.fetch_add(1, Ordering::Relaxed);
                self.bytes_armed.fetch_add(mmap_len, Ordering::Relaxed);
                Ok(Ok((t, armed)))
            }
            Err(e) => {
                // ⊘ The caller could not use the mapping, so the aperture goes straight back.
                // Holding it would be a leak with a plausible-looking cause.
                armed.released = self.release_inner_ref(&mut armed);
                Ok(Err((e, ())))
            }
        }
    }

    /// ★★★★★ **ARM A VIEW AND MAP IT INTO A HOST WINDOW OF THIS PROCESS.**
    ///
    /// The shape the **store** needs: a CPU mapping of a run of the reserved object that this
    /// process can `memcpy` through. Returns the window beside the view, and **both** must be
    /// kept together — dropping the window unmaps, and only [`DeviceViewPort::release`]
    /// returns the aperture.
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
    ) -> Result<(kayfabe_linux_raw::GuestWindow, ArmedView), ViewRefusal> {
        use kayfabe_linux_raw::{GuestWindow, HostOffset, HostPageSize};
        let out = self.with_node(offset, len, writable, |fd, mmap_len| {
            let win = GuestWindow::create(mmap_len, HostPageSize::query())
                .map_err(|e| format!("{e:?}"))?;
            win.place_device_view(HostOffset::ZERO, mmap_len, fd, writable)
                .map_err(|e| format!("{e:?}"))?;
            Ok(win)
        })?;
        match out {
            Ok((win, view)) => Ok((win, view)),
            Err((why, ())) => Err(self.note_refusal(ViewRefusal::Mmap(why))),
        }
    }

    /// ★★★★★ **GIVE THE APERTURE BACK.** The only thing that does.
    ///
    /// # Panics
    /// Through `Worker::release_device_view`'s `assert_lock_free`.
    pub fn release(&self, mut view: ArmedView) {
        view.released = self.release_inner_ref(&mut view);
    }

    fn release_inner(&self, mut view: ArmedView) {
        view.released = self.release_inner_ref(&mut view);
    }

    fn release_inner_ref(&self, view: &mut ArmedView) -> bool {
        if view.released {
            return true;
        }
        let t0 = std::time::Instant::now();
        let out = self
            .iso
            .with_worker(|worker| worker.release_device_view(&view.view));
        self.rel_us
            .fetch_add(t0.elapsed().as_micros() as u64, Ordering::Relaxed);
        match out {
            Some(Ok(())) => {
                self.released.fetch_add(1, Ordering::Relaxed);
                true
            }
            // ⊘⊘ A refused release is NOT a released view, and saying so is the whole point:
            // the aperture is gone either way, and a counter that called it released would
            // make the leak invisible in exactly the census meant to catch it.
            Some(Err(e)) => {
                self.note_refusal(ViewRefusal::Rm(format!("release: {e:?}")));
                false
            }
            None => {
                self.note_refusal(ViewRefusal::NoWorker);
                false
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
    /// ⊘ `armed` and `released` are printed separately and never as a difference: *"how many
    /// are outstanding"* and *"how many were leaked"* are the same arithmetic and different
    /// facts, and only the caller's own bookkeeping can tell them apart.
    #[must_use]
    pub fn census_line(&self) -> String {
        let armed = self.armed.load(Ordering::Relaxed);
        let released = self.released.load(Ordering::Relaxed);
        let refused = self.refused.load(Ordering::Relaxed);
        let first = self
            .first_refusal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        format!(
            "DEVICE-VIEW-PORT armed={armed} released={released} refused={refused} \
             bytes_armed={:.1}MiB arm_us_total={} rel_us_total={} first_refusal=[{}] \
             ⇒ {} ({} view{} still hold host BAR1)",
            self.bytes_armed.load(Ordering::Relaxed) as f64 / (1024.0 * 1024.0),
            self.arm_us.load(Ordering::Relaxed),
            self.rel_us.load(Ordering::Relaxed),
            first.unwrap_or_else(|| "none".to_string()),
            if armed == 0 && refused == 0 {
                "⊘⊘ VACUOUS — the port exists and NOTHING ever asked it for a view. That is \
                 not 'no views were needed'; it is an unmeasured port."
            } else if refused > 0 {
                "⚠ at least one arm was REFUSED — read first_refusal before reading anything \
                 else, because NV_ERR_NO_MEMORY here means the host BAR1 aperture is full"
            } else {
                "★ every arm succeeded"
            },
            armed.saturating_sub(released),
            if armed.saturating_sub(released) == 1 {
                ""
            } else {
                "s"
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
