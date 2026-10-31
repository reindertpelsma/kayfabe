//! Everything the refresh stress needs from hardware and from the OS, behind **one trait**.
//!
//! The arm's logic (qualification, batching, the single invalidate, timing, the oracle decision, the
//! caps, the verdict) lives in [`super::run`] and calls only this trait; the real implementation is
//! [`crate::hw::HwBackend`] over `kf_host::HostRm`, and the tests use a fake that can be told to
//! misbehave in specific ways. The trait carries **no logic and no policy**: each method is one verb
//! or one OS read, so a backend cannot decide a verdict.
//!
//! ## What is missing from `kf-host`, and how it shows
//!
//! * **`probe_va`** (the page-level-info probe of `VASPACE_GET_PAGE_LEVEL_INFO`) has no `kf-host`
//!   equivalent (V3_RAW_CLIENT.md §2.3.1). [`Backend::probe_free`] therefore returns
//!   [`Probe::Unsupported`] on the real backend and the arm records the unmap postcondition as a
//!   `SKIP`, never as a pass.
//! * **A second (bystander) channel** is not built; the arm does not claim the bystander property.
//! * **`MEM_OP` invalidate** is not built; only `NV2080_CTRL_CMD_DMA_INVALIDATE_TLB` is issued.

use crate::caps::HostFacts;
use crate::clock::Clock;
use crate::outcome::NotRunReason;

use super::census::PageFrame;
use super::params::Shape;

/// Why a backend call did not succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendError {
    /// The platform cannot do this at all (a missing device, a verb the host refuses by design). The arm
    /// maps it to NOTRUN: it decides nothing about kayfabe.
    Unsupported(NotRunReason, String),
    /// The call was attempted and failed. The arm maps it to FAIL.
    Failed(String),
}

impl BackendError {
    /// A failed call.
    #[must_use]
    pub fn failed(s: impl Into<String>) -> Self {
        BackendError::Failed(s.into())
    }
}

/// The answer to "is this VA unmapped?" after the unmap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// The host reports nothing mapped there.
    Free,
    /// The host reports a mapping still present.
    Mapped,
    /// The backend has no such verb. The check is skipped.
    Unsupported,
}

/// Fixed once, before the first refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Setup {
    /// Batches per refresh.
    pub batches: u32,
    /// Pages per batch.
    pub pages_per_batch: u32,
    /// Bytes per page.
    pub leaf_bytes: u64,
    /// Oracle sample pages per refresh = staging slots.
    pub sample: u32,
    /// Whole-batch or per-page maps.
    pub shape: Shape,
}

/// Hardware and OS, one verb or read per method.
pub trait Backend {
    /// The host's answers, gathered once: architecture, driver, page size, copy-engine class, the VA
    /// band, and whether page frame numbers are readable.
    ///
    /// # Errors
    /// [`BackendError::Unsupported`] with `NoRmDevice` when no RM session can be opened.
    fn facts(&mut self) -> Result<HostFacts, BackendError>;

    /// `MemAvailable` from `/proc/meminfo`, bytes.
    ///
    /// # Errors
    /// [`BackendError::Unsupported`] with `ProcUnreadable`.
    fn mem_available_bytes(&mut self) -> Result<u64, BackendError>;

    /// Create the VA space, the copy-engine rig and the staging buffer (OS-described and mapped at a
    /// fixed place after the batch windows). Called once, after the capability check.
    ///
    /// # Errors
    /// As the failing step.
    fn setup(&mut self, s: &Setup) -> Result<(), BackendError>;

    /// Allocate and touch (sequentially) `bytes` of anonymous memory, then give back a seeded random
    /// half of its pages, so the free lists hold holes spread over the machine's RAM. The region is kept
    /// until [`Backend::pressure_end`].
    ///
    /// # Errors
    /// As the failing step.
    fn pressure_begin(&mut self, bytes: u64, seed: u64) -> Result<(), BackendError>;

    /// A fresh anonymous workload region of `pages` pages, transparent huge pages disabled, replacing
    /// any earlier one (which must already have been released).
    ///
    /// # Errors
    /// As the failing step.
    fn alloc_workload(&mut self, pages: u32) -> Result<(), BackendError>;

    /// Touch workload page `page`, filling the whole page with `word` repeated.
    ///
    /// # Errors
    /// As the failing step.
    fn touch(&mut self, page: u32, word: u64) -> Result<(), BackendError>;

    /// The frame number of workload pages `first ..` into `out`.
    ///
    /// # Errors
    /// `ProcUnreadable` when pagemap cannot be read.
    fn pfns(&mut self, first: u32, out: &mut [PageFrame]) -> Result<(), BackendError>;

    /// Bytes of the workload region backed by transparent huge pages, `None` when unmeasurable.
    ///
    /// # Errors
    /// As the failing step.
    fn huge_bytes(&mut self) -> Result<Option<u64>, BackendError>;

    /// Release the pressure region.
    fn pressure_end(&mut self);

    /// Describe batch `batch` of the workload to RM as one OS descriptor (pin its pages).
    ///
    /// # Errors
    /// The host's refusal.
    fn pin_batch(&mut self, batch: u32) -> Result<(), BackendError>;

    /// Map batch `batch` into the GPU VA space at its window, deferring the TLB invalidate. For
    /// [`Shape::Whole`] this is one RM call; for [`Shape::Sliced`] one per page. Every RM call's duration
    /// (read on `clock`) is passed to `call_ns`.
    ///
    /// # Errors
    /// The host's refusal.
    fn map_batch(
        &mut self,
        batch: u32,
        clock: &dyn Clock,
        call_ns: &mut dyn FnMut(u64),
    ) -> Result<(), BackendError>;

    /// The ONE TLB invalidate for the space.
    ///
    /// # Errors
    /// The host's refusal.
    fn invalidate(&mut self) -> Result<(), BackendError>;

    /// Copy workload page `page` through its GPU mapping into staging slot `slot`, and wait for the
    /// completion (fence re-read at a wake). The slot is poisoned first by the backend, so an unwritten
    /// slot can never look like a correct page.
    ///
    /// # Errors
    /// The failing step, including a fence that did not land.
    fn copy_to_staging(&mut self, page: u32, slot: u32) -> Result<(), BackendError>;

    /// Read staging slot `slot` (one page, `words.len()` 64-bit words) with ordinary CPU loads.
    ///
    /// # Errors
    /// As the failing step.
    fn read_staging(&mut self, slot: u32, words: &mut [u64]) -> Result<(), BackendError>;

    /// CPU store: overwrite workload page `page` with `word` repeated (after the map; no RM call).
    ///
    /// # Errors
    /// As the failing step.
    fn rewrite_page(&mut self, page: u32, word: u64) -> Result<(), BackendError>;

    /// Unmap batch `batch` from the GPU VA space.
    ///
    /// # Errors
    /// The host's refusal.
    fn unmap_batch(&mut self, batch: u32) -> Result<(), BackendError>;

    /// After the unmap: does the host say workload page `page`'s VA is free?
    ///
    /// # Errors
    /// As the failing step.
    fn probe_free(&mut self, page: u32) -> Result<Probe, BackendError>;

    /// Free batch `batch`'s OS descriptor (un-pin).
    ///
    /// # Errors
    /// The host's refusal.
    fn free_batch(&mut self, batch: u32) -> Result<(), BackendError>;

    /// Release the workload region. Every descriptor over it must already be freed.
    fn release_workload(&mut self);

    /// Best-effort release of everything (channels, space, staging). Idempotent.
    fn teardown(&mut self);
}
