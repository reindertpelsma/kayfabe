//! ★ Host geometry as **pure functions of the page size** (`l1_os_shell.md` §5.2 item 2).
//!
//! Every derived quantity — round-up, round-down, alignment tests, pages spanned, the
//! widening a write-trap sub-range undergoes, and the composition root's validation of
//! core-supplied geometry — lives here, performs **no syscall**, and takes
//! [`HostPageSize`] as a parameter. That is what makes the test axis possible at all: the
//! same arithmetic runs at 4/16/64 KiB on one host, and it keeps the arithmetic inside the
//! thin waist where the determinism suite can reach it.
//!
//! It is also the half of the crate the `*_unsafe.rs` files *call* rather than duplicate,
//! which is how the house rule holds — see the crate docs.

use crate::error::RawError;
use crate::page_size::HostPageSize;
use core::ops::Range;

/// Round `value` **down** to a page boundary. Total: masking off low bits cannot overflow.
#[must_use]
pub const fn align_down(value: u64, page: HostPageSize) -> u64 {
    value & !page.mask()
}

/// Round `value` **up** to a page boundary.
///
/// Fallible, deliberately: rounding `u64::MAX` up is the rounding overflow §11 item 5
/// names as *"the classic way a bounded object stops being bounded"*, and returning a
/// wrapped 0 there would hand a caller a length that passes every subsequent bound.
///
/// # Errors
/// [`RawError::LengthOverflow`] if rounding up would exceed `u64`.
pub const fn align_up(value: u64, page: HostPageSize) -> Result<u64, RawError> {
    match value.checked_add(page.mask()) {
        Some(v) => Ok(v & !page.mask()),
        None => Err(RawError::LengthOverflow {
            offset: value,
            len: page.bytes(),
        }),
    }
}

/// Is `value` on a page boundary?
#[must_use]
pub const fn is_aligned(value: u64, page: HostPageSize) -> bool {
    value & page.mask() == 0
}

/// Refuse `value` unless it is page-aligned, naming what it was.
///
/// # Errors
/// [`RawError::Misaligned`] with `what`, the value, and the alignment it had to satisfy.
pub const fn require_aligned(
    value: u64,
    page: HostPageSize,
    what: &'static str,
) -> Result<(), RawError> {
    if is_aligned(value, page) {
        Ok(())
    } else {
        Err(RawError::Misaligned {
            what,
            value,
            required: page.bytes(),
        })
    }
}

/// How many host pages `[offset, offset + len)` touches.
///
/// # Errors
/// - [`RawError::ZeroLength`] — `len == 0` spans no pages, and a caller asking is a caller
///   with an empty request it has not noticed.
/// - [`RawError::LengthOverflow`] — the range, or its rounding, exceeds `u64`.
pub const fn pages_spanned(offset: u64, len: u64, page: HostPageSize) -> Result<u64, RawError> {
    if len == 0 {
        return Err(RawError::ZeroLength { what: "page span" });
    }
    let end = match offset.checked_add(len) {
        Some(e) => e,
        None => return Err(RawError::LengthOverflow { offset, len }),
    };
    let first = align_down(offset, page);
    let last = match align_up(end, page) {
        Ok(v) => v,
        Err(e) => return Err(e),
    };
    Ok((last - first) / page.bytes())
}

/// ★ Widen `[offset, offset + len)` to whole host pages — the **write-trap** rounding
/// (§5.2 / §14.4 P7).
///
/// Stated here because the consequence is easy to mis-read: a caller that reasons in
/// 4 KiB gets **more** pages trapped than it asked for on an arm64 host. That is correct
/// (a trap is conservative) and quietly slower, and it is a property of the host page
/// size rather than of the request — which is why the request cannot be expressed without
/// one.
///
/// # Errors
/// [`RawError::ZeroLength`] on an empty range; [`RawError::LengthOverflow`] if widening
/// would exceed `u64`.
pub const fn round_out(offset: u64, len: u64, page: HostPageSize) -> Result<(u64, u64), RawError> {
    if len == 0 {
        return Err(RawError::ZeroLength {
            what: "trap range length",
        });
    }
    let end = match offset.checked_add(len) {
        Some(e) => e,
        None => return Err(RawError::LengthOverflow { offset, len }),
    };
    let start = align_down(offset, page);
    let end = match align_up(end, page) {
        Ok(v) => v,
        Err(e) => return Err(e),
    };
    Ok((start, end - start))
}

/// ★★ **The composition root's obligation** (§5.2, §14.4 P7): validate a core-supplied
/// GPA window against the *real* host page size, loudly, at construction.
///
/// The typed page-size rule stops at this adapter. `GpaSpace::new(window, arena_len)`
/// takes plain `u64`s and cannot take a [`HostPageSize`] — a pure crate may not depend on
/// this one, which is the hexagonal boundary working rather than a defect. So a literal
/// `4096` **does** typecheck as an `arena_len`, and on a 64 KiB host that is a misaligned
/// window that no type can catch. This function is the only place that can, and the
/// composition root must call it before it builds the core's geometry.
///
/// A startup fault, in the same spirit as [`HostPageSize::query`]'s own validation — never
/// a silent round, because rounding here would move a window the core has already carved
/// arenas out of.
///
/// # Errors
/// - [`RawError::ZeroLength`] — an empty window, or a zero arena length.
/// - [`RawError::Misaligned`] — the window start, the window end or the arena length is
///   not a multiple of the host page size. Each names itself, so the fault says which.
pub fn validate_gpa_window(
    window: &Range<u64>,
    arena_len: u64,
    page: HostPageSize,
) -> Result<(), RawError> {
    if window.start >= window.end {
        return Err(RawError::ZeroLength { what: "gpa window" });
    }
    if arena_len == 0 {
        return Err(RawError::ZeroLength {
            what: "gpa arena length",
        });
    }
    require_aligned(window.start, page, "gpa window start")?;
    require_aligned(window.end, page, "gpa window end")?;
    require_aligned(arena_len, page, "gpa arena length")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page_size::test_page_sizes;

    /// ★ The test axis in its simplest form: the round-trip laws must hold at every host
    /// page size, not at the one this runner happens to have.
    #[test]
    fn alignment_laws_hold_at_every_host_page_size() {
        for page in test_page_sizes() {
            let p = page.bytes();
            assert_eq!(align_down(0, page), 0);
            assert_eq!(align_up(0, page), Ok(0));
            assert!(is_aligned(0, page));

            assert_eq!(align_down(p - 1, page), 0);
            assert_eq!(align_up(p - 1, page), Ok(p));
            assert!(!is_aligned(p - 1, page));

            assert_eq!(align_down(p, page), p);
            assert_eq!(align_up(p, page), Ok(p), "already aligned must not move");
            assert!(is_aligned(p, page));

            assert_eq!(align_down(p + 1, page), p);
            assert_eq!(align_up(p + 1, page), Ok(2 * p));
        }
    }

    /// The rounding overflow, by exact variant, at every page size.
    #[test]
    fn align_up_refuses_to_wrap() {
        for page in test_page_sizes() {
            assert_eq!(
                align_up(u64::MAX, page),
                Err(RawError::LengthOverflow {
                    offset: u64::MAX,
                    len: page.bytes()
                })
            );
        }
    }

    #[test]
    fn require_aligned_names_what_was_misaligned() {
        let page = HostPageSize::forced(65536).expect("64 KiB");
        assert_eq!(require_aligned(0x1_0000, page, "window start"), Ok(()));
        assert_eq!(
            require_aligned(0x1000, page, "window start"),
            Err(RawError::Misaligned {
                what: "window start",
                value: 0x1000,
                required: 65536
            })
        );
    }

    /// ★ The property that only shows up off the x86 axis: one 4 KiB request spans ONE
    /// page everywhere, but a request that straddles a 4 KiB boundary spans two pages on
    /// a 4 KiB host and one page on a 16/64 KiB host.
    #[test]
    fn pages_spanned_is_a_function_of_the_host_page_size() {
        let [k4, k16, k64] = test_page_sizes();
        assert_eq!(pages_spanned(0, 4096, k4), Ok(1));
        assert_eq!(pages_spanned(0, 4096, k16), Ok(1));
        assert_eq!(pages_spanned(0, 4096, k64), Ok(1));

        assert_eq!(pages_spanned(4095, 2, k4), Ok(2));
        assert_eq!(pages_spanned(4095, 2, k16), Ok(1));
        assert_eq!(pages_spanned(4095, 2, k64), Ok(1));

        assert_eq!(pages_spanned(0, 65537, k64), Ok(2));
        assert_eq!(
            pages_spanned(0, 0, k4),
            Err(RawError::ZeroLength { what: "page span" })
        );
    }

    /// ★ §14.4 P7's consequence, asserted rather than described: a 4 KiB write-trap
    /// request becomes a 64 KiB trap on a 64 KiB host. More pages trapped than asked for
    /// — correct, and quietly slower.
    #[test]
    fn round_out_widens_a_trap_request_to_whole_host_pages() {
        let [k4, k16, k64] = test_page_sizes();
        assert_eq!(round_out(0x1000, 0x1000, k4), Ok((0x1000, 0x1000)));
        assert_eq!(round_out(0x1000, 0x1000, k16), Ok((0, 0x4000)));
        assert_eq!(round_out(0x1000, 0x1000, k64), Ok((0, 0x1_0000)));
        assert_eq!(
            round_out(0, 0, k4),
            Err(RawError::ZeroLength {
                what: "trap range length"
            })
        );
    }

    /// ★★ The composition-root validation: the core's `GpaSpace::new` geometry, checked
    /// against the *host*. The 4 KiB-shaped window that every existing core test uses is
    /// exactly what a 64 KiB host must refuse.
    #[test]
    fn a_core_geometry_that_is_fine_at_4k_is_refused_at_64k() {
        let [k4, _k16, k64] = test_page_sizes();
        let window = 0x1000..0x11000;
        assert_eq!(validate_gpa_window(&window, 0x1000, k4), Ok(()));
        assert_eq!(
            validate_gpa_window(&window, 0x1000, k64),
            Err(RawError::Misaligned {
                what: "gpa window start",
                value: 0x1000,
                required: 65536
            })
        );
    }

    #[test]
    fn the_validation_names_which_of_the_three_quantities_was_wrong() {
        let k64 = HostPageSize::forced(65536).expect("64 KiB");
        assert_eq!(validate_gpa_window(&(0..0x2_0000), 0x1_0000, k64), Ok(()));
        assert_eq!(
            validate_gpa_window(&(0..0x2_0001), 0x1_0000, k64),
            Err(RawError::Misaligned {
                what: "gpa window end",
                value: 0x2_0001,
                required: 65536
            })
        );
        assert_eq!(
            validate_gpa_window(&(0..0x2_0000), 0x1000, k64),
            Err(RawError::Misaligned {
                what: "gpa arena length",
                value: 0x1000,
                required: 65536
            })
        );
    }

    #[test]
    fn the_validation_refuses_an_empty_window_and_a_zero_arena() {
        let k4 = HostPageSize::forced(4096).expect("4 KiB");
        assert_eq!(
            validate_gpa_window(&(0x1000..0x1000), 0x1000, k4),
            Err(RawError::ZeroLength { what: "gpa window" })
        );
        assert_eq!(
            validate_gpa_window(&(0..0x1000), 0, k4),
            Err(RawError::ZeroLength {
                what: "gpa arena length"
            })
        );
    }
}
