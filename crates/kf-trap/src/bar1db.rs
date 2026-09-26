//! ★★★ **Where the guest put its BAR1 doorbell views — the placement tracker.**
//! (`docs/design/V3_BAR1_DOORBELL.md` §3.)
//!
//! On Hopper+ a client that allocates `HOPPER_USERMODE_A`/`BLACKWELL_USERMODE_A` with
//! `bBar1Mapping` gets its CPU view of the usermode page through **BAR1**, at a VA the guest RM's own
//! BAR1 allocator chose (`usermode_api.c:94-98` selects `pBar1VF`; `mapping_cpu.c:484-516` →
//! `kbusMapFbAperture_GM107` → `dmaAllocMapping_HAL` into `bar1[gfid].pVAS`,
//! `kern_bus_gm107.c:3018,3412-3435`). There is **no fixed offset**, and the allocation never reaches
//! the GSP (the resource has no RPC flag, `resource_list.h:885-904`, flags `RS_FLAGS_ALLOC_NON_PRIVILEGED | RS_FLAGS_ACQUIRE_GPUS_LOCK` only), so the ONLY thing kayfabe sees
//! is the guest's BAR1 PTE write, through the walker, at the guest's BAR1 invalidate.
//!
//! ⇒ This table is fed by the VA-manager thread from walked BAR1 leaves the family's
//! [`kf_chip::usermode::UsermodeMmio`] classifies as the user page, and it is the **only** authority
//! for which BAR1 pages carry a write trap. It holds no guest-chosen VALUE on any vCPU path: the
//! overlay the VMM installs carries its own `vf_rel`, so a trapped write is decoded without a lookup.
//!
//! ⊘ Map may not defer the MAPPING (the trap is placed before the guest's invalidate clears); unmap
//! may not complete while anything still routes through the view (today: nothing but our own trap,
//! so removal is immediate — the guest doorbell module will add a wait here, see
//! `V3_GUEST_DOORBELL_MODULE.md` §2 "Lifecycle and the BAR1 doorbell").

use std::collections::BTreeMap;

/// One BAR1 view of (part of) the usermode page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bar1View {
    /// BAR1 offset (= BAR1 VA) of the view.
    pub base: u64,
    /// Bytes.
    pub len: u64,
    /// Offset of `base` inside the 64 KiB usermode page.
    pub vf_rel: u64,
}

impl Bar1View {
    /// Whether BAR1 offset `off` lies in this view.
    #[must_use]
    pub fn contains(&self, off: u64) -> bool {
        off >= self.base && off - self.base < self.len
    }
}

/// Why a view was refused. By name, never clamped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bar1DbRefusal {
    /// Not whole 4 KiB pages.
    Unaligned(Bar1View),
    /// Past the end of the BAR1 aperture.
    OutsideBar1 {
        /// The view.
        view: Bar1View,
        /// The aperture's bytes.
        bar1_bytes: u64,
    },
    /// Past the end of the usermode page.
    OutsideUsermodePage(Bar1View),
    /// Overlaps a view we already hold.
    Overlap {
        /// The new view.
        view: Bar1View,
        /// The one it overlaps.
        held: Bar1View,
    },
    /// The VMM's overlay pool is exhausted.
    Full {
        /// The capacity.
        cap: usize,
    },
}

/// ★ The table.
#[derive(Debug, Clone)]
pub struct Bar1Doorbells {
    views: BTreeMap<u64, Bar1View>,
    cap: usize,
    usermode_len: u64,
    bar1_bytes: u64,
}

/// A GMMU small page.
const PAGE: u64 = 0x1000;

impl Bar1Doorbells {
    /// A table for a `bar1_bytes` aperture, a `usermode_len`-byte usermode page and at most `cap`
    /// simultaneous views (the VMM's overlay pool).
    #[must_use]
    pub fn new(bar1_bytes: u64, usermode_len: u64, cap: usize) -> Bar1Doorbells {
        Bar1Doorbells { views: BTreeMap::new(), cap, usermode_len, bar1_bytes }
    }

    /// Set the overlay pool's capacity (the C device's `bar1-overlays` property). Views already
    /// held are kept; only later [`Bar1Doorbells::place`] calls see the new cap.
    pub fn set_cap(&mut self, cap: usize) {
        self.cap = cap;
    }

    /// Validate and record `v`. The caller installs the trap only on `Ok`.
    ///
    /// # Errors
    /// [`Bar1DbRefusal`], by name.
    pub fn place(&mut self, v: Bar1View) -> Result<(), Bar1DbRefusal> {
        if v.len == 0 || (v.base | v.len | v.vf_rel) & (PAGE - 1) != 0 {
            return Err(Bar1DbRefusal::Unaligned(v));
        }
        if v.base.checked_add(v.len).is_none_or(|e| e > self.bar1_bytes) {
            return Err(Bar1DbRefusal::OutsideBar1 { view: v, bar1_bytes: self.bar1_bytes });
        }
        if v.vf_rel.checked_add(v.len).is_none_or(|e| e > self.usermode_len) {
            return Err(Bar1DbRefusal::OutsideUsermodePage(v));
        }
        let end = v.base + v.len;
        if let Some(held) = self.views.values().find(|h| h.base < end && v.base < h.base + h.len) {
            return Err(Bar1DbRefusal::Overlap { view: v, held: *held });
        }
        if self.views.len() >= self.cap {
            return Err(Bar1DbRefusal::Full { cap: self.cap });
        }
        self.views.insert(v.base, v);
        Ok(())
    }

    /// Forget the view starting at `base` (the caller removes the trap first).
    pub fn remove(&mut self, base: u64) -> Option<Bar1View> {
        self.views.remove(&base)
    }

    /// The view starting exactly at `base`.
    #[must_use]
    pub fn at_base(&self, base: u64) -> Option<Bar1View> {
        self.views.get(&base).copied()
    }

    /// ★ The view covering BAR1 offset `off`, and the usermode-page offset `off` names.
    #[must_use]
    pub fn resolve(&self, off: u64) -> Option<(Bar1View, u64)> {
        let (_, v) = self.views.range(..=off).next_back()?;
        v.contains(off).then(|| (*v, v.vf_rel + (off - v.base)))
    }

    /// ★ The structural gate for BAR1: a write at `off` may trap iff a view covers it.
    #[must_use]
    pub fn may_trap_write(&self, off: u64) -> bool {
        self.resolve(off).is_some()
    }

    /// Every view held, in BAR1 order — what a late-loading guest doorbell module is replayed.
    pub fn views(&self) -> impl Iterator<Item = &Bar1View> {
        self.views.values()
    }

    /// How many views are held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.views.len()
    }

    /// Whether none is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> Bar1Doorbells {
        Bar1Doorbells::new(256 << 20, 0x1_0000, 4)
    }

    #[test]
    fn a_placed_view_resolves_the_doorbell_and_nothing_else_traps() {
        let mut d = t();
        // A GH100-shaped view: RM's BAR1 allocator chose 0x0123_0000 for pBar1VF (64 KiB).
        d.place(Bar1View { base: 0x0123_0000, len: 0x1_0000, vf_rel: 0 }).unwrap();
        assert_eq!(d.resolve(0x0123_0090).map(|(_, o)| o), Some(0x90), "the doorbell");
        assert_eq!(d.resolve(0x0123_0080).map(|(_, o)| o), Some(0x80), "TIME_0");
        assert!(d.may_trap_write(0x0123_FFFC));
        assert!(!d.may_trap_write(0x0124_0000), "one past the view");
        assert!(!d.may_trap_write(0x0122_FFFC), "one before it");
        // ⊘ The old fixed page is NOT special any more.
        assert!(!d.may_trap_write(0x9_0090));
    }

    #[test]
    fn a_discontiguous_view_keeps_each_pieces_page_offset() {
        let mut d = t();
        // ALLOW_DISCONTIG (mapping_cpu.c:484): the 64 KiB may arrive as two runs.
        d.place(Bar1View { base: 0x40_0000, len: 0x8000, vf_rel: 0 }).unwrap();
        d.place(Bar1View { base: 0x80_0000, len: 0x8000, vf_rel: 0x8000 }).unwrap();
        assert_eq!(d.resolve(0x80_0010).map(|(_, o)| o), Some(0x8010));
        assert_eq!(d.resolve(0x40_0090).map(|(_, o)| o), Some(0x90));
    }

    #[test]
    fn refusals_are_named() {
        let mut d = t();
        assert!(matches!(d.place(Bar1View { base: 0x1010, len: 0x1000, vf_rel: 0 }), Err(Bar1DbRefusal::Unaligned(_))));
        assert!(matches!(
            d.place(Bar1View { base: (256 << 20) - 0x1000, len: 0x2000, vf_rel: 0 }),
            Err(Bar1DbRefusal::OutsideBar1 { .. })
        ));
        assert!(matches!(
            d.place(Bar1View { base: 0, len: 0x2000, vf_rel: 0xF000 }),
            Err(Bar1DbRefusal::OutsideUsermodePage(_))
        ));
        d.place(Bar1View { base: 0x10_0000, len: 0x1_0000, vf_rel: 0 }).unwrap();
        assert!(matches!(
            d.place(Bar1View { base: 0x10_8000, len: 0x1000, vf_rel: 0 }),
            Err(Bar1DbRefusal::Overlap { .. })
        ));
        for i in 1..4u64 {
            d.place(Bar1View { base: 0x100_0000 * i, len: 0x1000, vf_rel: 0 }).unwrap();
        }
        assert_eq!(d.place(Bar1View { base: 0x800_0000, len: 0x1000, vf_rel: 0 }), Err(Bar1DbRefusal::Full { cap: 4 }));
    }

    #[test]
    fn remove_ends_the_trap() {
        let mut d = t();
        d.place(Bar1View { base: 0x2_0000, len: 0x1_0000, vf_rel: 0 }).unwrap();
        assert!(d.remove(0x2_0000).is_some());
        assert!(!d.may_trap_write(0x2_0090));
        assert!(d.is_empty());
        // The same VA may be placed again (a new process got the recycled BAR1 VA).
        d.place(Bar1View { base: 0x2_0000, len: 0x1_0000, vf_rel: 0 }).unwrap();
    }
}
