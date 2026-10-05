//! Public-source contract for the bounded contiguous SYSRAM ALLOC_MEMORY subset.
//! ABI equality is insufficient: only individually audited compressed-PTE producers
//! and MemoryList consumers have cells. No host-driver or GPU-family assumptions.

#[path = "memory_list_generated.rs"]
mod generated;

#[derive(Debug, Clone, Copy)]
struct Field {
    mask: u32,
    shift: u32,
}
impl Field {
    fn get(self, value: u32) -> u32 {
        (value & self.mask) >> self.shift
    }
}

/// Compiler facts plus an explicitly audited behavioral contract.
#[derive(Debug, Clone, Copy)]
pub struct Cell {
    version: crate::DriverVersion,
    function: u32,
    class: u32,
    size: usize,
    offsets: [usize; 10],
    pte: usize,
    idr_mask: u32,
    reserved_mask: u32,
    count_mask: u32,
    idr_none: u32,
    pitch: u32,
    page_shift: u32,
    page_size: u64,
    allowed_flags: u32,
    physicality: Field,
    contiguous: u32,
    location: Field,
    pci: u32,
    cache: Field,
    caches: [u32; 6],
    mapping: Field,
    mappings: [u32; 3],
}

/// Exact decoded declaration. Addresses are guest DMA addresses, never host PFNs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Declaration {
    /// Owning guest client.
    pub client: u32,
    /// Direct Device or Subdevice parent.
    pub parent: u32,
    /// New memory handle.
    pub handle: u32,
    /// Source-defined SYSTEM MemoryList class.
    pub class: u32,
    /// All admitted original flags, retained for exact idempotence and future policy.
    pub flags: u32,
    /// Base of the supplied guest physical page.
    pub page_base: u64,
    /// Full page-rounded range that must be current writable guest RAM.
    pub span: u64,
    /// Offset of logical byte zero within the supplied page.
    pub adjustment: u32,
    /// Logical descriptor length in bytes.
    pub length: u64,
}

/// Only named, source-audited guest contracts. No closest-version fallback.
#[must_use]
pub fn cell(version: crate::DriverVersion) -> Option<&'static Cell> {
    generated::CELLS.iter().find(|c| c.version == version)
}
impl Cell {
    /// Source-defined RPC number, used only by the independently enabled policy.
    #[must_use]
    pub fn function(&self) -> u32 {
        self.function
    }
    /// Decode the exact one-inline-PFN linear SYSRAM subset. No memory is accessed.
    #[must_use]
    pub fn decode(&self, body: &[u8]) -> Option<Declaration> {
        if body.len() != self.size {
            return None;
        }
        let u32_at = |offset: usize| {
            Some(u32::from_le_bytes(
                body.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
            ))
        };
        let u64_at = |offset: usize| {
            Some(u64::from_le_bytes(
                body.get(offset..offset.checked_add(8)?)?.try_into().ok()?,
            ))
        };
        let [
            client,
            parent,
            handle,
            class,
            flags,
            adjust,
            format,
            length,
            count,
            descriptor,
        ] = self.offsets;
        let flags = u32_at(flags)?;
        let adjustment = u32_at(adjust)?;
        let length = u64_at(length)?;
        let desc = u32_at(descriptor)?;
        if flags & !self.allowed_flags != 0
            || self.physicality.get(flags) != self.contiguous
            || self.location.get(flags) != self.pci
            || !self.caches.contains(&self.cache.get(flags))
            || !self.mappings.contains(&self.mapping.get(flags))
            || u32_at(class)? != self.class
            || u32_at(format)? != self.pitch
            || u32_at(count)? != 1
            || desc & self.idr_mask != self.idr_none
            || desc & self.reserved_mask != 0
            || (desc & self.count_mask) >> self.count_mask.trailing_zeros() != 1
            || u64::from(adjustment) >= self.page_size
            || length == 0
        {
            return None;
        }
        // checked_shl checks the shift amount, not high bits shifted away.
        let page_base = u64_at(self.pte)?.checked_mul(self.page_size)?;
        if self.page_size != 1u64.checked_shl(self.page_shift)? {
            return None;
        }
        let used = u64::from(adjustment).checked_add(length)?;
        let span = used.checked_add(self.page_size.checked_sub(1)?)? & !(self.page_size - 1);
        page_base.checked_add(span)?;
        let d = Declaration {
            client: u32_at(client)?,
            parent: u32_at(parent)?,
            handle: u32_at(handle)?,
            class: self.class,
            flags,
            page_base,
            span,
            adjustment,
            length,
        };
        if d.client == 0
            || d.parent == 0
            || d.handle == 0
            || d.handle == d.client
            || d.handle == d.parent
        {
            return None;
        }
        Some(d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Vec<u8> {
        // Independent public C compiler specimen: no transport-delivered padding.
        let mut b = vec![0; 64];
        for (off, value) in [
            (0, 1u32),
            (4, 2),
            (8, 4),
            (12, 0x81),
            (16, 0x48002000),
            (40, 1),
            (48, 0x10000),
        ] {
            b[off..off + 4].copy_from_slice(&value.to_le_bytes());
        }
        b[32..40].copy_from_slice(&0x7000u64.to_le_bytes());
        b[56..64].copy_from_slice(&2u64.to_le_bytes());
        b
    }
    #[test]
    fn memory_list_all_named_contracts_decode_independent_c_specimen() {
        for c in generated::CELLS {
            let d = c.decode(&request()).unwrap();
            assert_eq!((d.page_base, d.span, d.length), (0x2000, 0x7000, 0x7000));
            assert_eq!(c.function(), 4);
            let mut b = request();
            for off in [28, 44, 52] {
                b[off..off + 4].fill(0xab);
            }
            assert_eq!(c.decode(&b), Some(d), "compiler padding is not reserved");
        }
        for tag in [
            "515.43.04",
            "535.309.01",
            "550.90.12",
            "560.35.03",
            "580.65.07",
            "595.91.08",
        ] {
            assert!(cell(crate::DriverVersion::parse(tag).unwrap()).is_none());
        }
    }
    #[test]
    fn memory_list_lengths_flags_descriptor_and_pointer_arithmetic_refuse() {
        for c in generated::CELLS {
            for size in [0, 55, 56, 63, 65, 72, 4096] {
                let mut b = request();
                b.resize(size, 0);
                assert!(c.decode(&b).is_none());
            }
            for (off, value) in [
                (0, 0u32),
                (4, 0),
                (8, 0),
                (8, 1),
                (8, 2),
                (12, 0x82),
                (16, 1),
                (16, 0x10),
                (16, 0x200),
                (16, 0x6000),
                (16, 0xc0000000),
                (16, 0x40000),
                (16, 0x10000),
                (16, 0x400000),
                (20, 4096),
                (24, 1),
                (40, 0),
                (40, 2),
                (48, 0),
                (48, 0x10001),
                (48, 0x10004),
                (48, 0x20000),
            ] {
                let mut b = request();
                b[off..off + 4].copy_from_slice(&(value as u32).to_le_bytes());
                assert!(c.decode(&b).is_none(), "off={off} value={value}");
            }
            for (off, value) in [
                (32, 0),
                (32, u64::MAX),
                (32, u64::MAX - 4095),
                (56, u64::MAX),
                (56, (u64::MAX >> 12)),
            ] {
                let mut b = request();
                b[off..off + 8].copy_from_slice(&value.to_le_bytes());
                assert!(c.decode(&b).is_none());
            }
        }
    }
    #[test]
    fn memory_list_source_cache_mapping_values_and_adjusted_extent() {
        for c in generated::CELLS {
            for cache in 0..=5u32 {
                for mapping in 0..=2u32 {
                    for register in [0, 1u32] {
                        let mut b = request();
                        let flags = (cache << 12) | (mapping << 30) | (register << 27);
                        b[16..20].copy_from_slice(&flags.to_le_bytes());
                        assert!(c.decode(&b).is_some());
                    }
                }
            }
            let mut b = request();
            b[20..24].copy_from_slice(&4095u32.to_le_bytes());
            b[32..40].copy_from_slice(&2u64.to_le_bytes());
            let d = c.decode(&b).unwrap();
            assert_eq!((d.adjustment, d.span, d.length), (4095, 8192, 2));
        }
    }
}
