//! Bounded resolved Sysmem USERD bytes, not a CPU-local handle resolver.
//! Realization rejects vIOMMU; resolved DMA addresses name this VM's GPA space.
//! Caller must retain exact channel allocation admission across every access;
//! initial zero is allowed only inside pending-birth commit. RAM guards remain
//! inside MemoryListRam; no pointer escapes and topology cannot revive.

use super::{MemoryListRam, RamMap};
use kf_rm::memory_list::{AccessError, GuestRamAuthority, RamSpan};

#[derive(Clone)]
pub(crate) struct Userd {
    ram: &'static RamMap,
    span: RamSpan,
    generation: u64,
}
impl Userd {
    pub fn new(ram: &'static RamMap, base: u64, size: u64) -> Result<Self, AccessError> {
        if base & 0x1ff != 0 || size < 0x200 || base.checked_add(size).is_none() {
            return Err(AccessError::Bounds);
        }
        let guard = ram.blocks.try_read().map_err(|e| match e {
            std::sync::TryLockError::WouldBlock => AccessError::Busy,
            _ => AccessError::Poisoned,
        })?;
        let generation = ram.generation.load(std::sync::atomic::Ordering::Acquire);
        drop(guard);
        let source = Self {
            ram,
            span: RamSpan {
                base,
                length: 0x200,
            },
            generation,
        };
        source.read(0, &mut [])?;
        Ok(source)
    }
    pub fn read(&self, offset: u64, bytes: &mut [u8]) -> Result<(), AccessError> {
        MemoryListRam(self.ram).try_read(self.span, self.generation, offset, bytes)
    }
    pub fn write(&self, offset: u64, bytes: &[u8]) -> Result<(), AccessError> {
        MemoryListRam(self.ram).try_write(self.span, self.generation, offset, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (&'static RamMap, super::super::RamBlock) {
        let ram = Box::leak(Box::new(RamMap::default()));
        let (mem, fd) = crate::raw_unsafe::test_owned_ram(0x1000);
        let block = super::super::RamBlock {
            gpa: 0x1_0000_0000,
            mem: mem.test_alias(),
            fd: Some(fd),
            fd_off: 0xc000_0000,
        };
        ram.add(block.test_alias());
        (ram, block)
    }
    #[test]
    fn resolved_userd_needs_no_cpu_handle_and_uses_vmm_ram_translation() {
        let (ram, block) = fixture();
        let source = Userd::new(ram, block.gpa + 0x600, 0x200).unwrap();
        source.write(0x8c, &17u32.to_le_bytes()).unwrap();
        let mut bytes = [0; 4];
        source.read(0x8c, &mut bytes).unwrap();
        assert_eq!(u32::from_le_bytes(bytes), 17);
        block.mem.read_into(0x68c, &mut bytes);
        assert_eq!(u32::from_le_bytes(bytes), 17);
        // A same-valued CPU handle/MemoryList is not an input to this authority.
        assert_eq!(source.write(0x1ff, &[1, 2]), Err(AccessError::Bounds));
        assert_eq!(
            Userd::new(ram, block.gpa + 1, 0x200).err(),
            Some(AccessError::Bounds)
        );
        assert_eq!(
            Userd::new(ram, block.gpa, 0x1ff).err(),
            Some(AccessError::Bounds)
        );
        assert_eq!(
            Userd::new(ram, u64::MAX - 0x1ff, 0x200).err(),
            Some(AccessError::Bounds)
        );
        assert_eq!(
            Userd::new(ram, block.gpa + 0x1000, 0x200).err(),
            Some(AccessError::Topology)
        );
    }
    #[test]
    fn topology_contention_retries_but_replacement_never_revives_old_userd() {
        let (ram, block) = fixture();
        let source = Userd::new(ram, block.gpa, 0x200).unwrap();
        let guard = ram.blocks.write().unwrap();
        assert_eq!(source.write(0, &[1]), Err(AccessError::Busy));
        drop(guard);
        ram.del(block.gpa);
        // the replacement maps the same address to other bytes of the file
        ram.add(super::super::RamBlock {
            fd_off: block.fd_off + 0x1000,
            ..block.test_alias()
        });
        assert_eq!(source.write(0, &[1]), Err(AccessError::Topology));
        assert_eq!(source.read(0, &mut [0]), Err(AccessError::Topology));
        // and putting the old translation back never revives it
        ram.del(block.gpa);
        ram.add(block.test_alias());
        assert_eq!(source.read(0, &mut [0]), Err(AccessError::Topology));
    }

    /// The S3 wake of a q35 guest (`traces/win11_s3_tdr_20261123/`): the machine reset toggles a
    /// TSEG/PAM window, so QEMU's flat view removes the low-RAM section and adds a longer one that
    /// shows the same file bytes at the same addresses. A sysmem USERD resolved before the wake
    /// must still read its `GP_PUT` after it; before the fix every RAM add or removal revoked
    /// every lease, the user-ring relay refused `USER_RING_PUT_READ: Topology` and the Windows
    /// driver's channel never ran again (TDR, event 153).
    #[test]
    fn a_wake_resplit_of_the_same_ram_keeps_a_resolved_userd() {
        let (whole, fd) = crate::raw_unsafe::test_owned_ram(0x3000);
        let ram = Box::leak(Box::new(RamMap::default()));
        let gpa = 0x1_0000_0000u64;
        let fd_off = 0xc000_0000u64;
        // before the wake: [gpa+0x1000, gpa+0x3000) (a window covers [gpa, gpa+0x1000))
        ram.add(super::super::RamBlock {
            gpa: gpa + 0x1000,
            mem: whole.test_sub(0x1000, 0x2000),
            fd: Some(fd.clone()),
            fd_off: fd_off + 0x1000,
        });
        let source = Userd::new(ram, gpa + 0x1600, 0x200).unwrap();
        source.write(0x8c, &17u32.to_le_bytes()).unwrap();
        // the wake's reset: the section is removed and the longer one, same linear map, added
        ram.del(gpa + 0x1000);
        ram.add(super::super::RamBlock {
            gpa,
            mem: whole.test_alias(),
            fd: Some(fd.clone()),
            fd_off,
        });
        let mut put = [0u8; 4];
        assert_eq!(source.read(0x8c, &mut put), Ok(()));
        assert_eq!(u32::from_le_bytes(put), 17);
        // the firmware closes the window again: the same bytes, split back
        ram.del(gpa);
        ram.add(super::super::RamBlock {
            gpa: gpa + 0x1000,
            mem: whole.test_sub(0x1000, 0x2000),
            fd: Some(fd.clone()),
            fd_off: fd_off + 0x1000,
        });
        source.write(0x8c, &18u32.to_le_bytes()).unwrap();
        whole.read_into(0x168c, &mut put);
        assert_eq!(u32::from_le_bytes(put), 18);
        // a block that shows OTHER file bytes at the USERD's address ends the lease for good
        ram.del(gpa + 0x1000);
        ram.add(super::super::RamBlock {
            gpa,
            mem: whole.test_alias(),
            fd: Some(fd),
            fd_off: fd_off + 0x10_0000,
        });
        assert_eq!(source.read(0x8c, &mut put), Err(AccessError::Topology));
    }
}
