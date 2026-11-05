//! `/proc/self/pagemap` and `/proc/self/smaps` readers for a [`MappedRegion`] — **safe code only**.
//!
//! Added 2026-10-31 for the refresh-stress arm (`docs/design/V3_RAW_CLIENT.md` §4.2): to say that a
//! workload's memory is *fragmented*, the arm reads, for every page of its own anonymous region, the
//! page frame number the kernel gave it, and the amount of transparent huge page backing the region
//! got. Neither needs an `unsafe` block: both are file reads. The region's address is the one thing a
//! caller may not hold (`MappedRegion::addr_at` is `pub(crate)`), which is why these are methods of
//! the region and not free functions over an address.
//!
//! ## What a census built on this may and may not conclude
//!
//! * `pfn` is **zero for an unprivileged reader** (the kernel hides it without `CAP_SYS_ADMIN`);
//!   [`PagemapEntry::pfn`] then carries `0` with `present == true`, and a caller must treat
//!   "every present page has pfn 0" as *blind*, not as *contiguous*. [`pfns_visible`] is that test.
//! * A frame number is a *host* frame natively and a *guest-physical* frame inside a guest. That is
//!   the unit the host-side stitch consumes, so the census is in that unit either way.
//! * `anon_huge_bytes` is `None` when `smaps` cannot be read or names no VMA overlapping the region:
//!   a reading that did not happen is not a zero reading (the lesson of `pmd_mapped_bytes`).

use crate::bounds::HostOffset;
use crate::error::RawError;
use crate::mapping_unsafe::MappedRegion;
use crate::page_size::HostPageSize;
use std::os::unix::fs::FileExt;

/// The most entries one [`MappedRegion::pagemap`] call reads (512 KiB of kernel data). A caller
/// with more pages loops; a single call can never be made to read an unbounded range.
pub const PAGEMAP_MAX_ENTRIES: usize = 65_536;

/// One decoded `/proc/self/pagemap` word (`Documentation/admin-guide/mm/pagemap.rst`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PagemapEntry {
    /// Bit 63: the page is present in RAM.
    pub present: bool,
    /// Bit 62: the page is swapped out.
    pub swapped: bool,
    /// Bit 61: a file page or a shared anonymous page.
    pub file_or_shared: bool,
    /// Bits 0-54: the page frame number. Meaningful only when `present`, and zeroed by the kernel
    /// for a reader without `CAP_SYS_ADMIN`.
    pub pfn: u64,
}

/// Decode one raw 64-bit pagemap word.
#[must_use]
pub const fn decode_pagemap_entry(raw: u64) -> PagemapEntry {
    PagemapEntry {
        present: (raw >> 63) & 1 == 1,
        swapped: (raw >> 62) & 1 == 1,
        file_or_shared: (raw >> 61) & 1 == 1,
        pfn: raw & ((1u64 << 55) - 1),
    }
}

/// Whether the frame numbers in `entries` are real: at least one present page has a non-zero frame.
/// All-present-and-all-zero is what an unprivileged read looks like.
#[must_use]
pub fn pfns_visible(entries: &[PagemapEntry]) -> bool {
    entries.iter().any(|e| e.present && e.pfn != 0)
}

/// Sum the `AnonHugePages` of every VMA in `smaps` that overlaps `[lo, hi)`, in bytes.
///
/// Conservative on purpose: two adjacent anonymous VMAs with equal flags merge in the kernel, so the
/// VMA that holds a region may be larger than the region, and a header match on exact bounds would
/// silently measure nothing. Summing every overlapping VMA can only over-count, which makes a
/// "no huge pages" claim stronger, never weaker. `None` when no VMA overlaps (the instrument saw
/// nothing, which is not a zero).
#[must_use]
pub fn anon_huge_bytes_in_range(smaps: &str, lo: u64, hi: u64) -> Option<u64> {
    let mut in_overlap = false;
    let mut seen = false;
    let mut kib = 0u64;
    for line in smaps.lines() {
        if let Some((a, b)) = vma_header(line) {
            in_overlap = a < hi && lo < b;
            seen |= in_overlap;
            continue;
        }
        if in_overlap && let Some(rest) = line.strip_prefix("AnonHugePages:") {
            kib = kib.saturating_add(
                rest.trim()
                    .trim_end_matches("kB")
                    .trim()
                    .parse::<u64>()
                    .unwrap_or(0),
            );
        }
    }
    seen.then(|| kib.saturating_mul(1024))
}

/// `start-end perms ...` -> `(start, end)`; `None` for any other line (a field line has a `:` in
/// its first word, a header does not).
fn vma_header(line: &str) -> Option<(u64, u64)> {
    let first = line.split(' ').next()?;
    if first.contains(':') {
        return None;
    }
    let (a, b) = first.split_once('-')?;
    Some((
        u64::from_str_radix(a, 16).ok()?,
        u64::from_str_radix(b, 16).ok()?,
    ))
}

impl MappedRegion {
    /// Read the pagemap word of pages `first_page ..` of this region into `out` (one per page).
    ///
    /// # Errors
    /// [`RawError::OutOfRange`] / [`RawError::LengthOverflow`] when the pages leave the region;
    /// [`RawError::ZeroLength`] for an empty `out`; [`RawError::TooLargeForHost`] for more than
    /// [`PAGEMAP_MAX_ENTRIES`]; [`RawError::Syscall`] when `/proc/self/pagemap` cannot be opened or
    /// read (a sandbox without `/proc`).
    pub fn pagemap(&self, first_page: u64, out: &mut [PagemapEntry]) -> Result<(), RawError> {
        if out.is_empty() {
            return Err(RawError::ZeroLength {
                what: "pagemap read",
            });
        }
        if out.len() > PAGEMAP_MAX_ENTRIES {
            return Err(RawError::TooLargeForHost {
                value: out.len() as u64,
            });
        }
        let page = HostPageSize::query().bytes();
        let off = first_page
            .checked_mul(page)
            .ok_or(RawError::LengthOverflow {
                offset: first_page,
                len: page,
            })?;
        let len = (out.len() as u64) * page;
        let addr = self.addr_at(HostOffset::new(off), len)?;
        let io = |e: std::io::Error| RawError::Syscall {
            call: "read(/proc/self/pagemap)",
            errno: e.raw_os_error(),
        };
        let file = std::fs::File::open("/proc/self/pagemap").map_err(io)?;
        let mut raw = vec![0u8; out.len() * 8];
        file.read_exact_at(&mut raw, addr / page * 8).map_err(io)?;
        let (words, _) = raw.as_chunks::<8>();
        for (slot, word) in out.iter_mut().zip(words) {
            *slot = decode_pagemap_entry(u64::from_le_bytes(*word));
        }
        Ok(())
    }

    /// Bytes of this region the kernel backs with anonymous transparent huge pages, from
    /// `/proc/self/smaps` ([`anon_huge_bytes_in_range`]); `None` when it could not be read.
    #[must_use]
    pub fn anon_huge_bytes(&self) -> Option<u64> {
        let addr = self.addr_at(HostOffset::ZERO, self.len_bytes()).ok()?;
        let text = std::fs::read_to_string("/proc/self/smaps").ok()?;
        anon_huge_bytes_in_range(&text, addr, addr + self.len_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CachePolicy;
    use crate::mapping_unsafe::{Backing, HostProt, RegionAdvice};

    #[test]
    fn decodes_the_documented_bits() {
        let e = decode_pagemap_entry(1 << 63 | 0x1234);
        assert!(e.present && !e.swapped && !e.file_or_shared);
        assert_eq!(e.pfn, 0x1234);
        let e = decode_pagemap_entry(1 << 62);
        assert!(!e.present && e.swapped);
        // Bits 55-60 are flags, not frame bits.
        assert_eq!(decode_pagemap_entry(0x7F << 55 | 7).pfn, 7);
        assert!(decode_pagemap_entry(1 << 61).file_or_shared);
    }

    #[test]
    fn all_zero_present_pages_are_blind_not_contiguous() {
        let hidden = [decode_pagemap_entry(1 << 63); 4];
        assert!(!pfns_visible(&hidden));
        let seen = [
            decode_pagemap_entry(1 << 63),
            decode_pagemap_entry(1 << 63 | 9),
        ];
        assert!(pfns_visible(&seen));
        assert!(!pfns_visible(&[]));
    }

    const SMAPS: &str = "\
10000-20000 rw-p 00000000 00:00 0
Size:                 64 kB
AnonHugePages:         0 kB
30000-50000 rw-p 00000000 00:00 0
Size:                128 kB
AnonHugePages:      2048 kB
VmFlags: rd wr mr mw me ac
60000-70000 r--p 00000000 08:01 12 /lib/x
AnonHugePages:         4 kB
";

    #[test]
    fn smaps_sum_covers_every_overlapping_vma_and_none_is_not_zero() {
        assert_eq!(anon_huge_bytes_in_range(SMAPS, 0x10000, 0x20000), Some(0));
        assert_eq!(
            anon_huge_bytes_in_range(SMAPS, 0x30000, 0x50000),
            Some(2048 * 1024)
        );
        // A region straddling two VMAs sums both (a merged VMA must not hide a huge page).
        assert_eq!(
            anon_huge_bytes_in_range(SMAPS, 0x1F000, 0x31000),
            Some(2048 * 1024)
        );
        // Nothing overlaps: blind, not zero.
        assert_eq!(anon_huge_bytes_in_range(SMAPS, 0x80000, 0x90000), None);
        assert_eq!(anon_huge_bytes_in_range("", 0, 1), None);
    }

    #[test]
    fn field_lines_are_never_taken_for_headers() {
        assert_eq!(vma_header("Size: 1-2 kB"), None);
        assert_eq!(vma_header("zz-yy rw-p"), None);
        assert_eq!(vma_header("1000-2000 rw-p 0"), Some((0x1000, 0x2000)));
    }

    #[test]
    fn a_real_region_reads_back_present_pages_and_no_huge_pages() {
        let page = HostPageSize::query();
        let n = 64u64;
        let r = MappedRegion::map(
            Backing::PrivateAnonymous,
            n * page.bytes(),
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
            page,
        )
        .expect("map");
        r.advise(HostOffset::ZERO, n * page.bytes(), RegionAdvice::NoHugePage)
            .expect("nohugepage");
        let mut e = vec![PagemapEntry::default(); n as usize];
        r.pagemap(0, &mut e).expect("pagemap before touch");
        assert!(
            e.iter().all(|x| !x.present),
            "untouched anonymous pages are absent"
        );
        for i in 0..n {
            r.write_from(HostOffset::new(i * page.bytes()), &[0xA5])
                .expect("touch");
        }
        r.pagemap(0, &mut e).expect("pagemap after touch");
        assert!(e.iter().all(|x| x.present && !x.swapped));
        assert_eq!(r.anon_huge_bytes(), Some(0));
        // DONTNEED gives the pages back.
        r.advise(HostOffset::ZERO, 8 * page.bytes(), RegionAdvice::DontNeed)
            .expect("dontneed");
        r.pagemap(0, &mut e).expect("pagemap after discard");
        assert!(e[..8].iter().all(|x| !x.present));
        assert!(e[8..].iter().all(|x| x.present));
    }

    #[test]
    fn bounds_are_refused_by_name() {
        let page = HostPageSize::query();
        let r = MappedRegion::map(
            Backing::PrivateAnonymous,
            4 * page.bytes(),
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
            page,
        )
        .expect("map");
        let mut e = vec![PagemapEntry::default(); 5];
        assert!(matches!(
            r.pagemap(0, &mut e),
            Err(RawError::OutOfRange { .. })
        ));
        assert!(matches!(
            r.pagemap(u64::MAX, &mut e[..1]),
            Err(RawError::LengthOverflow { .. })
        ));
        assert!(matches!(
            r.pagemap(0, &mut []),
            Err(RawError::ZeroLength { .. })
        ));
        let mut big = vec![PagemapEntry::default(); PAGEMAP_MAX_ENTRIES + 1];
        assert!(matches!(
            r.pagemap(0, &mut big),
            Err(RawError::TooLargeForHost { .. })
        ));
        assert!(matches!(
            r.advise(HostOffset::new(1), page.bytes(), RegionAdvice::DontNeed),
            Err(RawError::Misaligned { .. })
        ));
        assert!(matches!(
            r.advise(HostOffset::ZERO, 0, RegionAdvice::NoHugePage),
            Err(RawError::ZeroLength { .. })
        ));
        assert!(matches!(
            r.advise(HostOffset::ZERO, 8 * page.bytes(), RegionAdvice::DontNeed),
            Err(RawError::OutOfRange { .. })
        ));
    }
}
