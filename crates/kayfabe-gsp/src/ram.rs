//! The guest-RAM port, and the **self-describing region** the message queues live in.
//!
//! ## Why a port and not `Vmm`
//!
//! `kayfabe-gsp` is one of the crates the CI GPA-accessor gate covers: guest-physical
//! access must funnel through the one core-side classification site
//! (`kayfabe_fwd::guest_read`), so this crate may not call `Vmm::gpa_read` itself.
//! [`GuestRam`] is therefore a narrow port — two methods, no hypervisor vocabulary — that
//! the device shell implements over that site. It is also what makes S0–S5 provable with
//! no GPU and no hypervisor: a test implements it over a `BTreeMap`.
//!
//! ## ★ The region is NOT contiguous, and addressing it linearly is a latent bug
//!
//! The C artifact computes every queue address as `sharedMemPhysAddr + offset`
//! (`C: src/qemu/nvkvm_gpu_emul.c:3437, 2387, 1610`). That is correct **only while the
//! guest's 512 KiB sysmem allocation happens to be physically contiguous**. It is not
//! required to be — and every fact below was re-read at **both** vendored tags and holds
//! identically at both, only the line numbers moved (`ogkm-580`'s `message_queue_cpu.c` is
//! 59 lines shorter, so its numbers run lower). There is no version seam in the region's
//! shape, and nothing here should be version-keyed:
//!
//! - the backing memory is allocated `NV_MEMORY_NONCONTIGUOUS, ADDR_SYSMEM`
//!   (`ogkm-610: src/nvidia/src/kernel/gpu/gsp/message_queue_cpu.c:250-256`,
//!   `ogkm-580: :228-234`);
//! - the first pages of the region are **a page table describing the region itself** — an
//!   array of `RmPhysAddr`, one per 4 KiB page, filled by `memdescGetPhysAddrs`
//!   (`ogkm-610: message_queue_cpu.c:295-316`, `ogkm-580: :273-294`);
//! - `sharedMemPhysAddr = pPageTbl[0]` (`ogkm-610: message_queue_cpu.c:328-329`,
//!   `ogkm-580: :306-307`) — i.e. the table is self-describing: entry 0 is the page the
//!   table itself starts on;
//! - `cmdQueueOffset`/`statQueueOffset` are **byte offsets into the region**, not
//!   addresses (`ogkm-610: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:5484-5485`,
//!   `ogkm-580: :4488-4489`; the previous citation of `:5483-5484` was off by one and
//!   named `pageTableEntryCount` rather than `statQueueOffset`).
//!
//! nouveau's client is the instructive contrast: it allocates one *contiguous* block and
//! fills its table linearly (`nv: r535/gsp.c:1161-1162`), then passes the block's base
//! address directly (`:1190`). Identical numbers, different guarantee — which is exactly
//! how the C got away with it on the bench.
//!
//! This is authorised deviation **GSP-D8**. [`RegionMap`] walks the table; every queue
//! access in this crate resolves through it, so a fragmented region is a test case rather
//! than a field failure.

use crate::fault::{GspFault, RamRefused, RegionError};

/// Guest physical memory, as this crate is allowed to see it.
///
/// `Send` (not `Sync`): an implementation is reached through `&mut`, exactly like
/// `kayfabe_vmm::Vmm`, and its synchronisation is the adapter's.
pub trait GuestRam: Send {
    /// Read `buf.len()` bytes of guest RAM at `gpa`.
    ///
    /// # Errors
    ///
    /// [`RamRefused`] if the range is not readable guest RAM. Never a partial read: an
    /// implementation that cannot fill `buf` entirely must refuse.
    fn read(&mut self, gpa: u64, buf: &mut [u8]) -> Result<(), RamRefused>;

    /// Write `bytes` to guest RAM at `gpa`.
    ///
    /// # Errors
    ///
    /// [`RamRefused`] if the range is not writable guest RAM.
    fn write(&mut self, gpa: u64, bytes: &[u8]) -> Result<(), RamRefused>;
}

/// A guest-declared region, resolved through **its own page table**.
///
/// Constructed only by [`RegionMap::load`], which reads the table out of guest RAM
/// starting at the one address the guest hands us. There is no constructor that takes a
/// base address and a length, because that is the shape of the bug this type exists to
/// remove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionMap {
    page_size: u64,
    pages: Vec<u64>,
}

impl RegionMap {
    /// Load a region's page table, following the table across its own pages.
    ///
    /// `table_base` is the guest's `sharedMemPhysAddr`, which *is* `pPageTbl[0]` — the
    /// physical address of the page the table starts on. `entry_count` is its
    /// `pageTableEntryCount`. `max_entries` bounds a hostile declaration before any
    /// allocation happens.
    ///
    /// The walk is self-referential and that is the point: entries `0..k` (as many as fit
    /// in one page) are read linearly from `table_base`, and any further page **of the
    /// table** is located through an entry already read. A contiguous region and a
    /// fragmented one take the same code path.
    ///
    /// # Errors
    ///
    /// [`GspFault::RegionMalformed`] for an impossible declaration,
    /// [`GspFault::GuestRam`] if the table itself cannot be read.
    pub fn load(
        ram: &mut dyn GuestRam,
        table_base: u64,
        entry_count: u32,
        page_size: u64,
        max_entries: u32,
    ) -> Result<RegionMap, GspFault> {
        if page_size == 0 || !page_size.is_power_of_two() {
            return Err(RegionError::BadPageSize { page_size }.into());
        }
        if entry_count == 0 {
            return Err(RegionError::NoEntries.into());
        }
        if entry_count > max_entries {
            return Err(RegionError::TooManyEntries {
                declared: entry_count,
                max: max_entries,
            }
            .into());
        }

        let per_page = usize::try_from(page_size / 8).unwrap_or(usize::MAX);
        let want = entry_count as usize;
        let mut pages: Vec<u64> = Vec::with_capacity(want);
        let mut table_page = table_base;
        // Which page of the *table* we are reading. Page 0 is at `table_base`; page p>0
        // is at `pages[p]`, which we have necessarily already read (the table occupies
        // the region's first pages, so its own pages are its lowest-indexed entries).
        let mut table_page_index = 0usize;
        while pages.len() < want {
            let take = per_page.min(want - pages.len());
            let mut buf = vec![0u8; take * 8];
            ram.read(table_page, &mut buf)?;
            // The global index of this batch's **first** entry, captured before the loop
            // runs: `pages.len()` advances inside it, so reading it per-iteration would
            // double-count `i` and misname the offending entry for every `i > 0`.
            let batch_base = pages.len();
            for (i, chunk) in buf.chunks_exact(8).enumerate() {
                let raw = u64::from_le_bytes(chunk.try_into().unwrap_or([0; 8]));
                if raw & (page_size - 1) != 0 {
                    return Err(RegionError::UnalignedEntry {
                        index: batch_base + i,
                        value: raw,
                    }
                    .into());
                }
                pages.push(raw);
            }
            table_page_index += 1;
            if pages.len() < want {
                // The next page of the table is described by the entry we just read.
                table_page = *pages.get(table_page_index).ok_or(RegionError::NoEntries)?;
            }
        }

        Ok(RegionMap { page_size, pages })
    }

    /// Build a map directly from a page list. For tests and for a caller that already
    /// holds the table; the bounds are the same ones [`RegionMap::load`] enforces.
    ///
    /// # Errors
    ///
    /// [`GspFault::RegionMalformed`] for a bad page size, an empty list, or a misaligned
    /// entry.
    pub fn from_pages(page_size: u64, pages: Vec<u64>) -> Result<RegionMap, GspFault> {
        if page_size == 0 || !page_size.is_power_of_two() {
            return Err(RegionError::BadPageSize { page_size }.into());
        }
        if pages.is_empty() {
            return Err(RegionError::NoEntries.into());
        }
        for (index, &value) in pages.iter().enumerate() {
            if value & (page_size - 1) != 0 {
                return Err(RegionError::UnalignedEntry { index, value }.into());
            }
        }
        Ok(RegionMap { page_size, pages })
    }

    /// The region's total size in bytes.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.pages.len() as u64 * self.page_size
    }

    /// Always false — a [`RegionMap`] cannot be constructed with zero pages.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// The physically contiguous runs `[offset, offset+len)` decomposes into.
    ///
    /// One entry per page crossing; a contiguous region still yields one run per page,
    /// deliberately, so the fragmented case is never a *different* code path.
    ///
    /// # Errors
    ///
    /// [`GspFault::RegionOutOfRange`] if the range leaves the region.
    pub fn runs(&self, offset: u64, len: u64) -> Result<Vec<(u64, usize)>, GspFault> {
        let end = offset.checked_add(len).ok_or(GspFault::RegionOutOfRange {
            offset,
            len,
            region_len: self.len(),
        })?;
        if end > self.len() {
            return Err(GspFault::RegionOutOfRange {
                offset,
                len,
                region_len: self.len(),
            });
        }
        let mut out = Vec::new();
        let mut at = offset;
        while at < end {
            let page = (at / self.page_size) as usize;
            let within = at % self.page_size;
            let take = (self.page_size - within).min(end - at);
            let base = *self.pages.get(page).ok_or(GspFault::RegionOutOfRange {
                offset,
                len,
                region_len: self.len(),
            })?;
            out.push((base + within, usize::try_from(take).unwrap_or(0)));
            at += take;
        }
        Ok(out)
    }

    /// Read `buf.len()` bytes from `offset` within the region.
    ///
    /// # Errors
    ///
    /// [`GspFault::RegionOutOfRange`] or [`GspFault::GuestRam`].
    pub fn read(
        &self,
        ram: &mut dyn GuestRam,
        offset: u64,
        buf: &mut [u8],
    ) -> Result<(), GspFault> {
        let mut done = 0usize;
        for (gpa, take) in self.runs(offset, buf.len() as u64)? {
            ram.read(gpa, &mut buf[done..done + take])?;
            done += take;
        }
        Ok(())
    }

    /// Write `bytes` at `offset` within the region.
    ///
    /// # Errors
    ///
    /// [`GspFault::RegionOutOfRange`] or [`GspFault::GuestRam`].
    pub fn write(&self, ram: &mut dyn GuestRam, offset: u64, bytes: &[u8]) -> Result<(), GspFault> {
        let mut done = 0usize;
        for (gpa, take) in self.runs(offset, bytes.len() as u64)? {
            ram.write(gpa, &bytes[done..done + take])?;
            done += take;
        }
        Ok(())
    }

    /// Read one little-endian `u32` at `offset`.
    ///
    /// # Errors
    ///
    /// As [`RegionMap::read`].
    pub fn read_u32(&self, ram: &mut dyn GuestRam, offset: u64) -> Result<u32, GspFault> {
        let mut b = [0u8; 4];
        self.read(ram, offset, &mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    /// Write one little-endian `u32` at `offset`.
    ///
    /// # Errors
    ///
    /// As [`RegionMap::write`].
    pub fn write_u32(
        &self,
        ram: &mut dyn GuestRam,
        offset: u64,
        value: u32,
    ) -> Result<(), GspFault> {
        self.write(ram, offset, &value.to_le_bytes())
    }
}

kayfabe_util::assert_send_sync!(RegionMap);
kayfabe_util::assert_send!(dyn GuestRam);
