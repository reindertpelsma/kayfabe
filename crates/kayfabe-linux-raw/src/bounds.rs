//! The domain-typed offset and the one checked-span computation everything uses.
//!
//! `l1_os_shell.md` §4.2.1. This module is ordinary safe Rust and is where the *whole*
//! bounds argument lives; the `*_unsafe.rs` files call [`checked_span`] themselves,
//! immediately before dereferencing, so no block in this crate relies on a caller having
//! checked anything (the house rule — see the crate docs).

use crate::error::RawError;

/// ★ An offset **within a bounded object** — never an address, and never a guest address.
///
/// This is the constructive half of §4.2.1's rule. A caller must be able to say *"the
/// thing at offset X"*, or they will route around the rule the first time they need to
/// express it; what they must **not** be able to do is say it with unchecked integer
/// arithmetic on something pointer-shaped. So the type exists, it is cheap, and it
/// deliberately has:
///
/// - **no `Add`/`Sub`/`AddAssign` impl** — `off + 1` does not compile; the only way to
///   move an offset is [`HostOffset::checked_add`], which returns a `Result`
///   (`tests/ui/offset_arithmetic.rs`);
/// - **no conversion to or from `Gpa`** in either direction, by `From`, `as` or
///   arithmetic — these are different address spaces with different consequences for
///   being wrong (§4.2.1 refusal 4), and the compiler should be the one that says so;
/// - **no meaning without a base.** An offset is only ever interpreted by the object it
///   is handed to, which knows its own length. A length is never passed separately from
///   the base it belongs to.
///
/// [`HostOffset::get`] returning a `u64` is not a hole in refusal 3: an *offset* is not
/// an address, it is safe to log, and the object it indexes is the thing that bounds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct HostOffset(u64);

impl HostOffset {
    /// The start of any bounded object.
    pub const ZERO: HostOffset = HostOffset(0);

    /// An offset of `bytes` into whatever object it is later handed to.
    #[must_use]
    pub const fn new(bytes: u64) -> Self {
        HostOffset(bytes)
    }

    /// The offset, in bytes.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Advance by `delta`, refusing an overflow rather than wrapping.
    ///
    /// # Errors
    /// [`RawError::LengthOverflow`] if `self + delta` overflows `u64`.
    pub const fn checked_add(self, delta: u64) -> Result<Self, RawError> {
        match self.0.checked_add(delta) {
            Some(v) => Ok(HostOffset(v)),
            None => Err(RawError::LengthOverflow {
                offset: self.0,
                len: delta,
            }),
        }
    }
}

/// ★ **The one bounds check.** Validate `[offset, offset + len)` against an object of
/// `object_len` bytes and return it as `(start, len)` in `usize` — the form a dereference
/// needs, computed only once the range is known to be inside the object.
///
/// Every refusal is a distinct variant so a test can assert which one fired (the doctrine:
/// exact variants, never `is_err()`), and the order is deliberate: **overflow is checked
/// before the bound**, because an overflowed sum compared against a bound is a check that
/// silently succeeds — §11 item 5's "the classic way a bounded object stops being bounded".
///
/// # Errors
/// - [`RawError::ZeroLength`] — `len == 0`.
/// - [`RawError::LengthOverflow`] — `offset + len` overflows `u64`.
/// - [`RawError::OutOfRange`] — the range is not inside the object.
/// - [`RawError::TooLargeForHost`] — the range does not fit this host's `usize`.
pub(crate) fn checked_span(
    object_len: u64,
    offset: HostOffset,
    len: u64,
    what: &'static str,
) -> Result<(usize, usize), RawError> {
    if len == 0 {
        return Err(RawError::ZeroLength { what });
    }
    let offset = offset.get();
    let end = offset
        .checked_add(len)
        .ok_or(RawError::LengthOverflow { offset, len })?;
    if end > object_len {
        return Err(RawError::OutOfRange {
            offset,
            len,
            object_len,
        });
    }
    let start = usize::try_from(offset).map_err(|_| RawError::TooLargeForHost { value: offset })?;
    let len = usize::try_from(len).map_err(|_| RawError::TooLargeForHost { value: len })?;
    Ok((start, len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_offset_has_no_unchecked_arithmetic_and_refuses_overflow() {
        assert_eq!(HostOffset::ZERO.get(), 0);
        assert_eq!(HostOffset::new(8).checked_add(8), Ok(HostOffset::new(16)));
        assert_eq!(
            HostOffset::new(u64::MAX).checked_add(1),
            Err(RawError::LengthOverflow {
                offset: u64::MAX,
                len: 1
            })
        );
    }

    #[test]
    fn checked_span_accepts_exactly_the_inside_of_the_object() {
        assert_eq!(
            checked_span(16, HostOffset::ZERO, 16, "read length"),
            Ok((0, 16))
        );
        assert_eq!(
            checked_span(16, HostOffset::new(15), 1, "read length"),
            Ok((15, 1))
        );
    }

    #[test]
    fn checked_span_refuses_zero_length_by_that_exact_variant() {
        assert_eq!(
            checked_span(16, HostOffset::ZERO, 0, "read length"),
            Err(RawError::ZeroLength {
                what: "read length"
            })
        );
    }

    #[test]
    fn checked_span_refuses_one_byte_past_the_end() {
        assert_eq!(
            checked_span(16, HostOffset::new(16), 1, "read length"),
            Err(RawError::OutOfRange {
                offset: 16,
                len: 1,
                object_len: 16
            })
        );
        assert_eq!(
            checked_span(16, HostOffset::new(8), 9, "read length"),
            Err(RawError::OutOfRange {
                offset: 8,
                len: 9,
                object_len: 16
            })
        );
    }

    /// ★ The check that makes the bound a bound: an overflowing `offset + len` must be
    /// refused as an **overflow**, not wrap around into a range that looks in-bounds.
    #[test]
    fn checked_span_refuses_an_overflowing_sum_before_it_compares_the_bound() {
        assert_eq!(
            checked_span(16, HostOffset::new(u64::MAX), 2, "read length"),
            Err(RawError::LengthOverflow {
                offset: u64::MAX,
                len: 2
            })
        );
    }
}
