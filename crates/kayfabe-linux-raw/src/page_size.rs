//! ★ The host CPU page size — queried, never assumed (`l1_os_shell.md` §5, decision 10).
//!
//! `portability_arm64.md` states the binding rule (`sysconf(_SC_PAGESIZE)`, never a
//! hardcoded `4096`) and admitted it was only prose. This module is what makes it stick.
//!
//! ## First, the distinction that makes a naive grep theatre (§5.1)
//!
//! There are **two** page sizes in this system and they are unrelated:
//!
//! - the **GPU MMU** page size (4 KiB / 64 KiB / 2 MiB leaves) — `kayfabe_arch::PageSize`,
//!   a genuine architectural constant *of the GPU*, correct on every host CPU;
//! - the **host CPU** page size — 4 KiB on x86-64, 16 KiB or 64 KiB on arm64.
//!
//! A workspace-wide grep for `4096` therefore fires constantly on legitimate GPU constants
//! and on test addresses. A gate that fires constantly gets muted, and a muted gate is
//! worse than none — so the enforcement is **typed and targeted**, with grep as a narrow
//! backstop only.
//!
//! ## The type, and the exact reach of its claim
//!
//! [`HostPageSize`] has a private field and exactly two constructors: [`HostPageSize::query`]
//! (one `sysconf`, cached, validated) and [`HostPageSize::forced`] (test-only). Every
//! alignment, mapping-length and geometry API in this crate **takes** one, so a literal
//! `4096` cannot flow into the alignment path — it does not typecheck
//! (`tests/ui/page_size_literal.rs` pins both polarities).
//!
//! > **★ And the reach of that sentence is narrower than it reads (§5.2, §14.4).** The
//! > typed rule **stops at this adapter.** The core's geometry constructors take plain
//! > integers — `GpaSpace::new(window: Range<u64>, arena_len: u64)` — and they *cannot*
//! > take a `HostPageSize`, because a pure crate may not depend on this one. That is the
//! > hexagonal boundary working, not a defect. So a literal `4096` **does** typecheck as
//! > an `arena_len`, and on a 64 KiB host that is a misaligned window.
//! >
//! > The obligation that creates lands on the **composition root**, and there is nowhere
//! > else it can live: it validates every core-supplied geometry against
//! > [`HostPageSize::query`] at construction and refuses loudly. That validation is
//! > [`crate::geometry::validate_gpa_window`]. Stated this way rather than by widening the
//! > newtype's claim: *"no literal reaches the alignment path"* is true of this crate and
//! > false of the core, and a rule quoted one crate beyond where it holds is how an untrue
//! > invariant gets believed.
//!
//! ## The page size as a TEST AXIS (§5.2 item 3 — the headline)
//!
//! Every geometry test in this crate runs over `[4 KiB, 16 KiB, 64 KiB]` via
//! [`test_page_sizes`]. This is not a new idea in this repo: it is exactly what `LockMode`
//! already does — both lock configurations are tested from day one specifically so the late
//! granularity flip is never the untested mode. The page size deserves the same treatment
//! for the same reason.
//!
//! **What a forced run does NOT prove (§5.3), stated so nobody claims it.** It tests our
//! *geometry*, not the *kernel*. On an x86 host the real `mmap` still wants 4 KiB
//! alignment; forcing a **larger** page size is harmless for real mappings (a stricter
//! alignment is still an alignment), forcing a smaller one would not be. Whether a real
//! 16 KiB/64 KiB kernel agrees with our arithmetic is verified only on real arm64
//! hardware — an L3 item, and genuinely residual.

use crate::error::RawError;
use crate::sysconf_unsafe;
use std::sync::OnceLock;

/// The smallest host page size that can be real, and the largest.
///
/// arm64 supports 4/16/64 KiB translation granules; nothing outside that window is a
/// plausible host page size, so a value outside it is a startup fault rather than
/// something to round against.
const MIN_BYTES: u64 = 4 * 1024;
const MAX_BYTES: u64 = 64 * 1024;

/// ★ The host CPU page size. **No literal constructor** — see the module docs.
///
/// `Copy`, one machine word, and passed explicitly to every geometry function rather than
/// read from a global inside them: that is what lets the determinism suite run the same
/// arithmetic at three page sizes on one host (§5.2 item 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostPageSize(u64);

impl HostPageSize {
    /// The real host page size, from one cached `sysconf(_SC_PAGESIZE)`.
    ///
    /// # Panics
    /// If the OS reports something that cannot be a page size (not a power of two, or
    /// outside [4 KiB, 64 KiB]). That is deliberate and is §5.2 item 1's design: an absurd
    /// value is a **loud startup fault**, never a silent misalignment that shows up later
    /// as a mapping the guest and the isolate disagree about.
    #[must_use]
    pub fn query() -> Self {
        static CACHED: OnceLock<u64> = OnceLock::new();
        let bytes = *CACHED.get_or_init(|| {
            let reported = sysconf_unsafe::sc_pagesize();
            validate_reported(reported).unwrap_or_else(|e| {
                panic!("host page size is not usable (l1_os_shell.md §5.2): {e}")
            })
        });
        HostPageSize(bytes)
    }

    /// A page size chosen by the caller — **test builds only** (§5.2 item 1/3).
    ///
    /// This is the mechanism behind the test axis, and the mechanism the integration
    /// harness's future `KAYFABE_FORCE_HOST_PAGE_SIZE` knob will call. It is `cfg`- and
    /// feature-gated so it cannot exist in a shipping build: a forced page size that
    /// disagrees with the kernel's is exactly the misalignment the type exists to prevent.
    ///
    /// # Errors
    /// [`RawError::AbsurdPageSize`] under the same validation as [`HostPageSize::query`].
    #[cfg(any(test, feature = "force-host-page-size"))]
    pub fn forced(bytes: u64) -> Result<Self, RawError> {
        validate_bytes(bytes).map(HostPageSize)
    }

    /// The page size in bytes.
    #[must_use]
    pub const fn bytes(self) -> u64 {
        self.0
    }

    /// `bytes() - 1` — the low-bit mask an alignment test uses. Exposed because computing
    /// it at each call site is exactly the arithmetic this type exists to centralise.
    #[must_use]
    pub const fn mask(self) -> u64 {
        self.0 - 1
    }
}

/// Validate what `sysconf` reported. Split from [`validate_bytes`] so the `i64` the OS
/// hands back is preserved verbatim in the error — a negative or zero report is a
/// different fault from a nonsensical positive one, and the number is the evidence.
fn validate_reported(reported: i64) -> Result<u64, RawError> {
    let bytes = u64::try_from(reported).map_err(|_| RawError::AbsurdPageSize { reported })?;
    validate_bytes(bytes).map_err(|_| RawError::AbsurdPageSize { reported })
}

/// A power of two in `[MIN_BYTES, MAX_BYTES]`, or a refusal.
fn validate_bytes(bytes: u64) -> Result<u64, RawError> {
    if bytes.is_power_of_two() && (MIN_BYTES..=MAX_BYTES).contains(&bytes) {
        Ok(bytes)
    } else {
        Err(RawError::AbsurdPageSize {
            reported: i64::try_from(bytes).unwrap_or(i64::MAX),
        })
    }
}

/// ★ The test axis: every geometry test runs over all three (§5.2 item 3).
#[cfg(test)]
pub(crate) fn test_page_sizes() -> [HostPageSize; 3] {
    [
        HostPageSize::forced(4 * 1024).expect("4 KiB is a valid host page size"),
        HostPageSize::forced(16 * 1024).expect("16 KiB is a valid host page size"),
        HostPageSize::forced(64 * 1024).expect("64 KiB is a valid host page size"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real host must report something usable, and the answer must be stable
    /// (`OnceLock`) — a page size that changed mid-process would invalidate every
    /// mapping already placed against it.
    #[test]
    fn query_reports_a_real_power_of_two_and_caches_it() {
        let a = HostPageSize::query();
        let b = HostPageSize::query();
        assert_eq!(a, b);
        assert!(a.bytes().is_power_of_two(), "{a:?}");
        assert!((MIN_BYTES..=MAX_BYTES).contains(&a.bytes()), "{a:?}");
        assert_eq!(a.mask(), a.bytes() - 1);
    }

    #[test]
    fn the_three_test_page_sizes_are_the_arm64_granules() {
        let sizes = test_page_sizes().map(HostPageSize::bytes);
        assert_eq!(sizes, [4096, 16384, 65536]);
    }

    /// Both polarities of the validation, by exact variant. The interesting refusals are
    /// the *plausible* ones — 2048 and 131072 are powers of two, and 12288 is a multiple
    /// of 4096; none of them is a translation granule any host has.
    #[test]
    fn absurd_page_sizes_are_refused_by_that_exact_variant() {
        for bad in [0_u64, 1, 2048, 12288, 131_072, u64::MAX] {
            assert_eq!(
                HostPageSize::forced(bad),
                Err(RawError::AbsurdPageSize {
                    reported: i64::try_from(bad).unwrap_or(i64::MAX)
                }),
                "{bad} must be refused"
            );
        }
        for good in [4096_u64, 16384, 65536] {
            assert_eq!(
                HostPageSize::forced(good).map(HostPageSize::bytes),
                Ok(good)
            );
        }
    }

    /// A negative or zero `sysconf` report is the "the OS could not answer" shape, and it
    /// must surface the number the OS actually gave, not a lossy conversion of it.
    #[test]
    fn a_negative_sysconf_report_is_preserved_verbatim_in_the_error() {
        assert_eq!(
            validate_reported(-1),
            Err(RawError::AbsurdPageSize { reported: -1 })
        );
        assert_eq!(
            validate_reported(0),
            Err(RawError::AbsurdPageSize { reported: 0 })
        );
    }
}
