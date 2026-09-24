//! ★★★ **THE MEMORY MAP: what the VMM installs, for every byte of every BAR.**
//!
//! ⊘ This file exists because of one owner observation that reframed the whole trap policy:
//!
//! > `[owner, w824]` *"can we then ensure exactly the pages where we need read trap, only there is
//! > no kvm memslot, but surrounded we have kvm memslot coverage read_only and for PRAMIN r/w (no
//! > trap)"*
//!
//! KVM's model is not *"install a trap"*; it is *"is this GPA covered by a memslot?"* Covered ⇒
//! the guest touches memory at full speed. Not covered ⇒ `KVM_EXIT_MMIO`. ⇒ **Every register's
//! disposition is a memslot decision, and a read exit is not a feature you implement — it is a
//! hole you leave.** See `THE_CONSTRAINTS.md` §53 and `THE_BAR0_DISPOSITION_MAP.md`.
//!
//! ## Why the map TILES rather than listing exceptions
//!
//! [`memory_map`] returns regions that **exactly cover** each BAR, end to end, with no gaps and no
//! overlaps ([`MemoryMap::tiles`] proves it). ⊘ A map of *exceptions* would leave each VMM to
//! derive the default for everything else — and one of them to get it wrong. A map that tiles can
//! be installed blindly: the VMM walks it and installs what each row says, and there is nothing
//! left to infer.
//!
//! ⚠ **And the unit is the PAGE, never the register** (`[owner]` *"granular below 4kib is not
//! possible, so then you need to implement the traps for any adjecent register that cannot be
//! aligned out with kvm memslots"*). A [`Disposition::Hole`] therefore costs the whole 4 KiB: every
//! register on that page must be *implemented*, because there is no memory behind any of them.
//! That is why [`holes_for`] is a short, named, per-family list and not a predicate.

use kf_chip::Family;
use crate::trappolicy::{DoorbellPlacement, PRAMIN_BASE, PRAMIN_LEN};
use crate::vmm::Bar;

pub const PAGE: u64 = 0x1000;

/// How one region is served. ⊘ The five of `THE_CONSTRAINTS.md` §53.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// **A** — ordinary r/w memslot. No exit in either direction.
    ///
    /// ⊘ PRAMIN only. It is a window we re-point with an `mmap(MAP_FIXED)` inside the *trapped*
    /// `NV_PBUS_BAR0_WINDOW` write, which lives **outside** PRAMIN and is therefore a `B` region.
    PlainRam,

    /// **B** — read-only memslot: **reads hit DRAM, writes exit.** ★ The default, and almost all
    /// of BAR0. This is what *"no read traps"* means in implementation terms.
    ShadowWriteTrapped,

    /// **C** — read-only memslot over a **live host mapping**, for values we do not author.
    ///
    /// ⊘ Exactly one exists: the usermode/VF page carrying the microsecond counter and the
    /// doorbell. It is the only BAR0 region RM maps to an unprivileged host process, which is why
    /// it is the only one we can alias.
    HostPassthrough,

    /// **D** — **no memslot.** Both reads and writes exit.
    ///
    /// ⚠ The only disposition that costs a read exit, and it is never chosen for performance — it
    /// is forced, by a register whose **read has a side effect the guest verifies**. `why` names
    /// the register so a reader never has to guess which one dragged the page in.
    Hole { why: &'static str },
}

impl Disposition {
    /// Does a read at this region leave the guest?
    #[inline]
    pub fn read_exits(&self) -> bool {
        matches!(self, Disposition::Hole { .. })
    }
    /// Does a write at this region leave the guest?
    #[inline]
    pub fn write_exits(&self) -> bool {
        !matches!(self, Disposition::PlainRam)
    }
}

/// One contiguous span of one BAR, and how to serve it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub bar: Bar,
    pub base: u64,
    pub len: u64,
    pub how: Disposition,
}

/// The complete map for one device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryMap {
    pub regions: Vec<Region>,
    pub bar0_bytes: u64,
    pub bar1_bytes: u64,
    pub bar2_bytes: u64,
}

/// ★★★ The disposition-**D** pages, per family. **This list is the entire read-trap surface.**
///
/// ⊘ Derived in `THE_BAR0_DISPOSITION_MAP.md` §1 from ogkm (§50 level 2) and nouveau (level 5),
/// and cross-checked against nova, which declares no read auto-increment at all. Each entry is a
/// register whose **read advances a hardware cursor** — a falcon PIO data port — where the driver
/// then **asserts the cursor moved**, so no shadow can satisfy it.
///
/// ★ **Empty for Turing, Ampere and Ada** — the current product target has no read exits anywhere.
pub fn holes_for(family: Family) -> &'static [(u64, &'static str)] {
    match family {
        // ⊘ GSP boots here via the sysmem libos message queue, never through falcon PIO. GSP's own
        // EMEM port at 0x110ac4 is CrashCat-only and gated shut by serving FALCON_DEBUGINFO = 0.
        Family::Ga10x | Family::Ad10x => &[],

        // FSP comes up first out of chip reset and RM asks IT to boot GSP, over MCTP/NVDM packets
        // carried in FSP's EMEM. `_kfspReadPacket_GH100` reads NV_PFSP_EMEMD in a burst and then
        // asserts EMEMC advanced by exactly packetSize/4.
        Family::Gh100 => &[(0x008F_2000, "NV_PFSP_EMEMD 0x8F2ac4 — FSP boot handshake, AINCR burst")],

        // ⚠ Blackwell is split by die group: discrete parts use FSP like Hopper; the integrated
        // GB10B/GB20B parts have no FSP and put the identical protocol behind SEC2.
        Family::Gb20x => &[
            (0x008F_2000, "NV_PFSP_EMEMD 0x8F2ac4 — FSP boot handshake (discrete)"),
            (0x0084_0000, "NV_PSEC_EMEMD 0x840ac4 — SEC2 boot handshake (integrated GB10B/GB20B)"),
        ],
    }
}

/// The usermode/VF page — the **C** region. ⊘ `DRF_BASE(NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET)` is
/// `0xB80000`; the counter pair sits at `+0x30080/84` and the doorbell at `+0x30090`, i.e. page
/// `0xbb0000`.
pub const VF_USERMODE_PAGE: u64 = 0x00BB_0000;

/// ★★★ Build the complete map. Every byte of every BAR gets exactly one row.
pub fn memory_map(
    family: Family,
    doorbell: DoorbellPlacement,
    bar0_bytes: u64,
    bar1_bytes: u64,
    bar2_bytes: u64,
) -> MemoryMap {
    // ---- BAR0: start from "all B", then carve in the exceptions, in address order. ----
    let mut cuts: Vec<(u64, u64, Disposition)> = Vec::new();
    cuts.push((PRAMIN_BASE, PRAMIN_LEN, Disposition::PlainRam));
    cuts.push((VF_USERMODE_PAGE, PAGE, Disposition::HostPassthrough));
    for (page, why) in holes_for(family) {
        cuts.push((*page, PAGE, Disposition::Hole { why }));
    }
    cuts.retain(|(b, l, _)| *b + *l <= bar0_bytes);
    cuts.sort_by_key(|(b, _, _)| *b);

    let mut regions = Vec::new();
    let mut at = 0u64;
    for (base, len, how) in cuts {
        // ⊘ Overlapping exceptions would silently drop one. They are page-aligned and disjoint by
        // construction; assert it rather than trust it.
        assert!(base >= at, "memory-map cuts overlap at {base:#x} (cursor {at:#x})");
        if base > at {
            regions.push(Region { bar: Bar(0), base: at, len: base - at, how: Disposition::ShadowWriteTrapped });
        }
        regions.push(Region { bar: Bar(0), base, len, how });
        at = base + len;
    }
    if at < bar0_bytes {
        regions.push(Region { bar: Bar(0), base: at, len: bar0_bytes - at, how: Disposition::ShadowWriteTrapped });
    }

    // ---- BAR1: plain RAM throughout, except the doorbell page where the doorbell lives there. ----
    match doorbell {
        DoorbellPlacement::Bar0 { .. } => {
            regions.push(Region { bar: Bar(1), base: 0, len: bar1_bytes, how: Disposition::PlainRam });
        }
        DoorbellPlacement::Bar1 { page_base } => {
            let end = page_base + 0x1_0000;
            if page_base > 0 {
                regions.push(Region { bar: Bar(1), base: 0, len: page_base, how: Disposition::PlainRam });
            }
            // ★ The ONE page of BAR1 that may trap, and only writes — the ring is a write.
            regions.push(Region { bar: Bar(1), base: page_base, len: 0x1_0000, how: Disposition::ShadowWriteTrapped });
            if bar1_bytes > end {
                regions.push(Region { bar: Bar(1), base: end, len: bar1_bytes - end, how: Disposition::PlainRam });
            }
        }
    }

    // ---- BAR2: plain RAM, always, in every configuration. ----
    regions.push(Region { bar: Bar(2), base: 0, len: bar2_bytes, how: Disposition::PlainRam });

    MemoryMap { regions, bar0_bytes, bar1_bytes, bar2_bytes }
}

impl MemoryMap {
    /// ★ Does the map **tile** each BAR — cover it exactly, no gaps, no overlaps?
    ///
    /// ⊘ This is the property that lets a VMM install the map blindly. A gap is not a harmless
    /// omission: an uncovered span is an accidental [`Disposition::Hole`], i.e. a **read exit we
    /// never decided to have** — precisely the failure this whole design exists to prevent.
    pub fn tiles(&self) -> Result<(), String> {
        for (bar, total) in [(0u8, self.bar0_bytes), (1, self.bar1_bytes), (2, self.bar2_bytes)] {
            let mut rs: Vec<&Region> = self.regions.iter().filter(|r| r.bar.0 == bar).collect();
            rs.sort_by_key(|r| r.base);
            let mut at = 0u64;
            for r in &rs {
                if r.base != at {
                    return Err(format!(
                        "BAR{bar}: {} at {:#x}..{:#x} — an uncovered span is an ACCIDENTAL read exit",
                        if r.base > at { "GAP" } else { "OVERLAP" }, at, r.base
                    ));
                }
                at = r.base + r.len;
            }
            if at != total {
                return Err(format!("BAR{bar}: covered {at:#x} of {total:#x}"));
            }
        }
        Ok(())
    }

    /// Every region whose reads leave the guest. ★ The whole read-trap surface, in one call.
    pub fn read_exit_regions(&self) -> Vec<&Region> {
        self.regions.iter().filter(|r| r.how.read_exits()).collect()
    }

    /// How many 4 KiB pages lose their free reads. ⚠ The cost the owner named: a hole is
    /// page-granular, so this counts pages we must fully **implement**, not registers.
    pub fn read_exit_pages(&self) -> u64 {
        self.read_exit_regions().iter().map(|r| r.len.div_ceil(PAGE)).sum()
    }

    /// The disposition covering `(bar, offset)`, if the map covers it.
    pub fn disposition_at(&self, bar: Bar, offset: u64) -> Option<Disposition> {
        self.regions
            .iter()
            .find(|r| r.bar == bar && (r.base..r.base + r.len).contains(&offset))
            .map(|r| r.how)
    }
}

/// ★★★ **Install the map through the VMM seam.** The one translation from *disposition* to
/// *memslot*, written once so QEMU and Cloud Hypervisor cannot each invent their own.
///
/// ⊘ **The `Hole` arm is the important one, and it is the arm that does NOTHING.** A read exit is
/// produced by *not installing a memslot* — so the code that creates kayfabe's only read traps is
/// a `continue`. That is the whole design in one line: mapping code, not trapping code.
///
/// ⊘⊘⊘ **`bar_base` IS NOT OPTIONAL BOOKKEEPING — it was a real hole in this seam.** A [`Region`]
/// carries a **BAR-relative** offset, while [`crate::vmm::VmmOps::install_memslot`] takes a
/// **guest-physical address**. Nothing in the seam knew where a BAR is programmed, so the two
/// could not be connected at all. ⚠ Found by writing the test below, not by reading the code: the
/// first version silently treated BAR-relative offsets as GPAs, and BAR1 (256 MiB from 0) then
/// "covered" a BAR0 hole at `0x8F2000`. ⇒ **Two address spaces that are both plain `u64` will be
/// confused**, and only something that checks a concrete address catches it.
///
/// `bar_base` returns the GPA a BAR is currently mapped at, or `None` if the guest has not
/// programmed it yet — an unprogrammed BAR installs nothing, which is correct rather than an error.
///
/// `host_for` supplies the backing for one region; returning `None` leaves it uncovered, which is
/// itself a read exit, so a caller that cannot back a region must know that is what it is asking
/// for. ⚠ Regions are installed in address order and the caller gets the slots back in that order.
pub fn install(
    map: &MemoryMap,
    vmm: &dyn crate::vmm::VmmOps,
    mut bar_base: impl FnMut(crate::vmm::Bar) -> Option<u64>,
    mut host_for: impl FnMut(&Region) -> Option<crate::vmm::HostMapping>,
) -> Result<Vec<(Region, crate::vmm::SlotId)>, crate::vmm::VmmError> {
    // ⊘ Refuse to install a map that does not tile. A gap would become a read exit nobody chose,
    // and the point of failing here is that the VMM never sees a half-installed device.
    debug_assert!(map.tiles().is_ok(), "{:?}", map.tiles());

    let mut installed = Vec::new();
    let mut regions: Vec<&Region> = map.regions.iter().collect();
    regions.sort_by_key(|r| (r.bar.0, r.base));

    for r in regions {
        let readonly = match r.how {
            // A — plain RAM: no exit in either direction.
            Disposition::PlainRam => false,
            // B and C — reads from memory, writes exit. THE default.
            Disposition::ShadowWriteTrapped | Disposition::HostPassthrough => true,
            // ★ D — install NOTHING. This `continue` is the entire read-trap implementation.
            Disposition::Hole { .. } => continue,
        };
        let Some(base) = bar_base(r.bar) else { continue }; // BAR not yet programmed by the guest
        // ⊘⊘⊘ `[fable w825]` This was `else { continue }`. By this function's OWN doc an
        // uncovered span is an ACCIDENTAL READ EXIT — so silently skipping a region the map says
        // must be backed was a check that reported nothing and gated nothing. ⇒ Refuse by name;
        // the VMM must not come up with a hole it did not choose.
        let Some(host) = host_for(r) else {
            return Err(crate::vmm::VmmError::Unbacked { bar: r.bar.0, base: r.base, len: r.len });
        };
        let gpa = base.checked_add(r.base).ok_or(crate::vmm::VmmError::BadGpa)?;
        installed.push((*r, vmm.install_memslot(gpa, r.len, host, readonly)?));
    }
    Ok(installed)
}
