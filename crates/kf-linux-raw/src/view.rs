//! [`RegionView`] — "the thing at offset X, N bytes long", as an object.
//!
//! This module is ordinary safe Rust and holds **no** pointer: a view is a borrow of a
//! [`MappedRegion`] plus a validated sub-range, and every access it serves is re-checked
//! against its own length and then delegated. That is deliberate — the composable
//! arithmetic is the part most likely to be wrong, so it lives where it can be read and
//! unit-tested without an audit.
//!
//! ## Why this type is the point of the design (`l1_os_shell.md` §4.2.1)
//!
//! The prohibitive rule — *"no host CPU address ever crosses a crate boundary"* — is
//! correct and, alone, weak: a rule that only forbids gets routed around the moment
//! someone needs to express "the second half of this region". [`RegionView`] is what they
//! reach for instead. It is cheap, it composes ([`RegionView::slice`] of a view is another
//! view), and **every step of the composition is bounds-checked by construction** because
//! there is no other way to perform it. A caller cannot accumulate an unchecked offset,
//! because [`crate::HostOffset`] has no `Add`; and cannot escape into a raw address,
//! because no accessor here or on [`MappedRegion`] returns one.

use crate::bounds::{self, HostOffset};
use crate::error::RawError;
use crate::mapping_unsafe::MappedRegion;

/// A bounded sub-range of a [`MappedRegion`], borrowed from it.
///
/// Carries its length with its base, as §4.2.1 requires: there is no constructor that
/// takes a length separately from the thing it bounds, and the borrow makes a view that
/// outlives its region a compile error rather than a use-after-`munmap`
/// (`tests/ui/view_outlives_region.rs`).
#[derive(Debug, Clone, Copy)]
pub struct RegionView<'r> {
    region: &'r MappedRegion,
    offset: HostOffset,
    len: u64,
}

impl<'r> RegionView<'r> {
    /// Validate `[offset, offset + len)` against `region` and bind it.
    ///
    /// Crate-private: a view is obtained through [`MappedRegion::slice`], so there is
    /// exactly one door and it is the checked one.
    pub(crate) fn new(
        region: &'r MappedRegion,
        offset: HostOffset,
        len: u64,
    ) -> Result<Self, RawError> {
        bounds::checked_span(region.len_bytes(), offset, len, "slice length")?;
        Ok(RegionView {
            region,
            offset,
            len,
        })
    }

    /// The view's length in bytes — the bound its own accessors check against.
    #[must_use]
    pub fn len_bytes(&self) -> u64 {
        self.len
    }

    /// Copy `dst.len()` bytes from `offset` **within this view** into `dst`.
    ///
    /// # Errors
    /// As [`bounds::checked_span`], against the *view's* length — an offset that is inside
    /// the underlying region but outside the view is [`RawError::OutOfRange`], which is
    /// the whole reason to hold a view rather than a region.
    pub fn read_into(&self, offset: HostOffset, dst: &mut [u8]) -> Result<(), RawError> {
        let absolute = self.absolute(offset, dst.len() as u64, "read length")?;
        self.region.read_into(absolute, dst)
    }

    /// Copy `src` into the view at `offset`.
    ///
    /// # Errors
    /// [`RawError::NotWritable`] if the underlying region is read-only — checked here so a
    /// view cannot smuggle a write past the region's own refusal; otherwise as
    /// [`RegionView::read_into`].
    pub fn write_from(&self, offset: HostOffset, src: &[u8]) -> Result<(), RawError> {
        if !self.region.is_writable() {
            return Err(RawError::NotWritable);
        }
        let absolute = self.absolute(offset, src.len() as u64, "write length")?;
        self.region.write_from(absolute, src)
    }

    /// A narrower view. Composition re-checks: the sub-range is validated against **this**
    /// view's length, never against the region's.
    ///
    /// # Errors
    /// As [`bounds::checked_span`], against this view's length.
    pub fn slice(&self, offset: HostOffset, len: u64) -> Result<RegionView<'r>, RawError> {
        let absolute = self.absolute(offset, len, "slice length")?;
        Ok(RegionView {
            region: self.region,
            offset: absolute,
            len,
        })
    }

    /// Translate a view-relative offset to a region-relative one, having first proved the
    /// whole access is inside the view.
    ///
    /// The order matters and is the reason this is one function: checking against the
    /// view's bound *before* adding the view's base is what stops a large `offset` from
    /// being validated against the region instead.
    fn absolute(
        &self,
        offset: HostOffset,
        len: u64,
        what: &'static str,
    ) -> Result<HostOffset, RawError> {
        bounds::checked_span(self.len, offset, len, what)?;
        self.offset.checked_add(offset.get())
    }
}
