//! ★★★ **The replayable fault packet — `MMU_FAULT_BUFFER` (`0xc369`), 32 bytes.**
//!
//! `docs/design/V3_UVM_GUEST_FAULT_PLANE.md` §3.4. The guest's nvidia-uvm reads these out of the
//! replayable fault buffer it registered (`0x20800a9b`, [`crate::faultbuffer`]) and parses them
//! with `parse_fault_entry_common` (`ogkm-580: kernel-open/nvidia-uvm/uvm_volta_fault_buffer.c`),
//! which every later family's HAL reuses. This module is the byte layout and nothing else: which
//! VALUES go into the fields (the guest's instance block, the per-family access/fault codes) is
//! decided by the caller — `kf_chip::fault` resolves the codes per die group from the derived
//! hardware table, and that crate's tests hold [`FIELDS`] to the `class clc369.h` rows of the same
//! table.
//!
//! # The fields
//!
//! Bit positions are `MW(hi:lo)` over the eight little-endian dwords, exactly as `clc369.h:34-67`
//! states them (`ogkm-580: kernel-open/nvidia-uvm/clc369.h`):
//!
//! | field | bits | dword |
//! |---|---|---|
//! | `INST_APERTURE` | 9:8 | 0 |
//! | `INST_LO` (4 KiB frame) | 31:12 | 0 |
//! | `INST_HI` | 63:32 | 1 |
//! | `ADDR_PHYS_APERTURE` | 65:64 | 2 |
//! | `ADDR_LO` (4 KiB page) | 95:76 | 2 |
//! | `ADDR_HI` | 127:96 | 3 |
//! | `TIMESTAMP_LO` / `_HI` | 159:128 / 191:160 | 4 / 5 |
//! | `ENGINE_ID` | 200:192 | 6 |
//! | `FAULT_TYPE` | 228:224 | 7 |
//! | `REPLAYABLE_FAULT` | 231 | 7 |
//! | `CLIENT` | 238:232 | 7 |
//! | `ACCESS_TYPE` | 243:240 | 7 |
//! | `MMU_CLIENT_TYPE` | 244 | 7 |
//! | `GPC_ID` | 252:248 | 7 |
//! | `PROTECTED_MODE` | 253 | 7 |
//! | `REPLAYABLE_FAULT_EN` | 254 | 7 |
//! | `VALID` | 255 | 7 |
//!
//! # ★ The ordering the guest depends on
//!
//! `fetch_fault_buffer_entries` spins until an entry's `VALID` bit is set and then parses the
//! whole entry; `parse_replayable_entry` clears `VALID` afterwards. So a writer must make dwords
//! 0..7 visible **before** the `VALID` bit — [`FaultEntry::encode_invalid`] and [`VALID_DWORD`] /
//! [`VALID_BIT`] give it the two halves; the fence between them is the writer's.

/// Bytes per entry (`NVC369_BUF_SIZE`).
pub const ENTRY_BYTES: usize = 32;

/// The dword that carries `VALID` (bit 255 = dword 7, bit 31).
pub const VALID_DWORD: usize = 7;

/// `VALID` inside [`VALID_DWORD`].
pub const VALID_BIT: u32 = 1 << 31;

/// `NVC369_BUF_ENTRY_INST_APERTURE_VID_MEM`.
pub const INST_APERTURE_VID_MEM: u32 = 0;
/// `NVC369_BUF_ENTRY_INST_APERTURE_SYS_MEM_COHERENT`.
pub const INST_APERTURE_SYS_MEM_COHERENT: u32 = 2;
/// `NVC369_BUF_ENTRY_INST_APERTURE_SYS_MEM_NONCOHERENT`.
pub const INST_APERTURE_SYS_MEM_NONCOHERENT: u32 = 3;

/// One field: its `clc369.h` name and its `MW(hi:lo)` bit range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    /// The header's name (`NVC369_BUF_ENTRY_…`).
    pub name: &'static str,
    /// High bit, counted from bit 0 of dword 0.
    pub hi: u32,
    /// Low bit.
    pub lo: u32,
}

impl Field {
    /// Width in bits.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.hi - self.lo + 1
    }
}

/// `INST_APERTURE`.
pub const INST_APERTURE: Field = Field {
    name: "NVC369_BUF_ENTRY_INST_APERTURE",
    hi: 9,
    lo: 8,
};
/// `INST_LO` — bits 31:12 of the instance block's address.
pub const INST_LO: Field = Field {
    name: "NVC369_BUF_ENTRY_INST_LO",
    hi: 31,
    lo: 12,
};
/// `INST_HI` — bits 63:32 of the instance block's address.
pub const INST_HI: Field = Field {
    name: "NVC369_BUF_ENTRY_INST_HI",
    hi: 63,
    lo: 32,
};
/// `ADDR_PHYS_APERTURE` — meaningful only for a physical fault; 0 for a virtual one.
pub const ADDR_PHYS_APERTURE: Field = Field {
    name: "NVC369_BUF_ENTRY_ADDR_PHYS_APERTURE",
    hi: 65,
    lo: 64,
};
/// `ADDR_LO` — bits 31:12 of the faulting address.
pub const ADDR_LO: Field = Field {
    name: "NVC369_BUF_ENTRY_ADDR_LO",
    hi: 95,
    lo: 76,
};
/// `ADDR_HI` — bits 63:32 of the faulting address.
pub const ADDR_HI: Field = Field {
    name: "NVC369_BUF_ENTRY_ADDR_HI",
    hi: 127,
    lo: 96,
};
/// `TIMESTAMP_LO`.
pub const TIMESTAMP_LO: Field = Field {
    name: "NVC369_BUF_ENTRY_TIMESTAMP_LO",
    hi: 159,
    lo: 128,
};
/// `TIMESTAMP_HI`.
pub const TIMESTAMP_HI: Field = Field {
    name: "NVC369_BUF_ENTRY_TIMESTAMP_HI",
    hi: 191,
    lo: 160,
};
/// `ENGINE_ID` — the MMU engine id (for GR: `NV_PFAULT_MMU_ENG_ID_GRAPHICS + VEID`).
pub const ENGINE_ID: Field = Field {
    name: "NVC369_BUF_ENTRY_ENGINE_ID",
    hi: 200,
    lo: 192,
};
/// `FAULT_TYPE` — an `NV_PFAULT_FAULT_TYPE_*` value.
pub const FAULT_TYPE: Field = Field {
    name: "NVC369_BUF_ENTRY_FAULT_TYPE",
    hi: 228,
    lo: 224,
};
/// `REPLAYABLE_FAULT`.
pub const REPLAYABLE_FAULT: Field = Field {
    name: "NVC369_BUF_ENTRY_REPLAYABLE_FAULT",
    hi: 231,
    lo: 231,
};
/// `CLIENT` — the raw MMU client id.
pub const CLIENT: Field = Field {
    name: "NVC369_BUF_ENTRY_CLIENT",
    hi: 238,
    lo: 232,
};
/// `ACCESS_TYPE` — an `NV_PFAULT_ACCESS_TYPE_*` value.
pub const ACCESS_TYPE: Field = Field {
    name: "NVC369_BUF_ENTRY_ACCESS_TYPE",
    hi: 243,
    lo: 240,
};
/// `MMU_CLIENT_TYPE` — `NV_PFAULT_MMU_CLIENT_TYPE_GPC` / `_HUB`.
pub const MMU_CLIENT_TYPE: Field = Field {
    name: "NVC369_BUF_ENTRY_MMU_CLIENT_TYPE",
    hi: 244,
    lo: 244,
};
/// `GPC_ID`.
pub const GPC_ID: Field = Field {
    name: "NVC369_BUF_ENTRY_GPC_ID",
    hi: 252,
    lo: 248,
};
/// `PROTECTED_MODE`.
pub const PROTECTED_MODE: Field = Field {
    name: "NVC369_BUF_ENTRY_PROTECTED_MODE",
    hi: 253,
    lo: 253,
};
/// `REPLAYABLE_FAULT_EN` — UVM asserts it is set on a replayable entry.
pub const REPLAYABLE_FAULT_EN: Field = Field {
    name: "NVC369_BUF_ENTRY_REPLAYABLE_FAULT_EN",
    hi: 254,
    lo: 254,
};
/// `VALID` — written LAST.
pub const VALID: Field = Field {
    name: "NVC369_BUF_ENTRY_VALID",
    hi: 255,
    lo: 255,
};

/// Every field, in bit order — what `kf_chip`'s test holds to the derived header rows.
pub const FIELDS: [Field; 18] = [
    INST_APERTURE,
    INST_LO,
    INST_HI,
    ADDR_PHYS_APERTURE,
    ADDR_LO,
    ADDR_HI,
    TIMESTAMP_LO,
    TIMESTAMP_HI,
    ENGINE_ID,
    FAULT_TYPE,
    REPLAYABLE_FAULT,
    CLIENT,
    ACCESS_TYPE,
    MMU_CLIENT_TYPE,
    GPC_ID,
    PROTECTED_MODE,
    REPLAYABLE_FAULT_EN,
    VALID,
];

/// A value that does not fit its field — refused, never truncated (a truncated `GPC_ID` would
/// index another GPC's uTLB in the guest's fault service).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Overflow {
    /// The field.
    pub field: &'static str,
    /// The value that did not fit.
    pub value: u64,
}

/// One replayable fault entry, as raw field values (the caller has already translated any
/// software enum into the family's hardware codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FaultEntry {
    /// The instance block's address (4 KiB aligned; the low 12 bits are not encoded).
    pub inst_addr: u64,
    /// `INST_APERTURE` (raw 2-bit value).
    pub inst_aperture: u32,
    /// The faulting address (4 KiB aligned; the low 12 bits are not encoded).
    pub addr: u64,
    /// `ADDR_PHYS_APERTURE` (0 for a virtual fault).
    pub addr_phys_aperture: u32,
    /// The GPU timestamp.
    pub timestamp: u64,
    /// `ENGINE_ID`.
    pub engine_id: u32,
    /// `FAULT_TYPE`.
    pub fault_type: u32,
    /// `REPLAYABLE_FAULT`.
    pub replayable: bool,
    /// `CLIENT`.
    pub client: u32,
    /// `ACCESS_TYPE`.
    pub access_type: u32,
    /// `MMU_CLIENT_TYPE`.
    pub mmu_client_type: u32,
    /// `GPC_ID`.
    pub gpc_id: u32,
    /// `PROTECTED_MODE`.
    pub protected_mode: bool,
    /// `REPLAYABLE_FAULT_EN`.
    pub replayable_en: bool,
    /// `VALID`.
    pub valid: bool,
}

fn put(dw: &mut [u32; 8], f: Field, value: u64) -> Result<(), Overflow> {
    let width = f.width();
    if width < 64 && value >> width != 0 {
        return Err(Overflow {
            field: f.name,
            value,
        });
    }
    // Every field of this class lies within one dword (the header splits the 64-bit ones into
    // `_LO`/`_HI` pairs), so a single masked store per field is exact.
    let (w, lo) = ((f.lo / 32) as usize, f.lo % 32);
    debug_assert_eq!(f.hi / 32, f.lo / 32, "{} spans dwords", f.name);
    let mask: u32 = if width == 32 {
        u32::MAX
    } else {
        ((1u32 << width) - 1) << lo
    };
    dw[w] = (dw[w] & !mask) | (((value as u32) << lo) & mask);
    Ok(())
}

fn get(dw: &[u32; 8], f: Field) -> u64 {
    let (w, lo, width) = ((f.lo / 32) as usize, f.lo % 32, f.width());
    let v = dw[w] >> lo;
    u64::from(if width == 32 {
        v
    } else {
        v & ((1u32 << width) - 1)
    })
}

impl FaultEntry {
    /// The eight dwords, `VALID` as the entry says.
    ///
    /// # Errors
    /// [`Overflow`] — a value wider than its field, refused by name.
    pub fn encode(&self) -> Result<[u32; 8], Overflow> {
        let mut dw = [0u32; 8];
        put(&mut dw, INST_APERTURE, u64::from(self.inst_aperture))?;
        put(&mut dw, INST_LO, (self.inst_addr >> 12) & 0xF_FFFF)?;
        put(&mut dw, INST_HI, self.inst_addr >> 32)?;
        put(
            &mut dw,
            ADDR_PHYS_APERTURE,
            u64::from(self.addr_phys_aperture),
        )?;
        put(&mut dw, ADDR_LO, (self.addr >> 12) & 0xF_FFFF)?;
        put(&mut dw, ADDR_HI, self.addr >> 32)?;
        put(&mut dw, TIMESTAMP_LO, self.timestamp & 0xFFFF_FFFF)?;
        put(&mut dw, TIMESTAMP_HI, self.timestamp >> 32)?;
        put(&mut dw, ENGINE_ID, u64::from(self.engine_id))?;
        put(&mut dw, FAULT_TYPE, u64::from(self.fault_type))?;
        put(&mut dw, REPLAYABLE_FAULT, u64::from(self.replayable))?;
        put(&mut dw, CLIENT, u64::from(self.client))?;
        put(&mut dw, ACCESS_TYPE, u64::from(self.access_type))?;
        put(&mut dw, MMU_CLIENT_TYPE, u64::from(self.mmu_client_type))?;
        put(&mut dw, GPC_ID, u64::from(self.gpc_id))?;
        put(&mut dw, PROTECTED_MODE, u64::from(self.protected_mode))?;
        put(&mut dw, REPLAYABLE_FAULT_EN, u64::from(self.replayable_en))?;
        put(&mut dw, VALID, u64::from(self.valid))?;
        Ok(dw)
    }

    /// The eight dwords with `VALID` CLEAR, whatever [`Self::valid`] says — the first half of the
    /// write; the second half is `dw[VALID_DWORD] | VALID_BIT`, after a release fence.
    ///
    /// # Errors
    /// As [`Self::encode`].
    pub fn encode_invalid(&self) -> Result<[u32; 8], Overflow> {
        let mut dw = self.encode()?;
        dw[VALID_DWORD] &= !VALID_BIT;
        Ok(dw)
    }

    /// Decode eight dwords — the inverse of [`Self::encode`] (`parse_fault_entry_common`'s reads).
    #[must_use]
    pub fn decode(dw: &[u32; 8]) -> FaultEntry {
        FaultEntry {
            inst_aperture: get(dw, INST_APERTURE) as u32,
            inst_addr: (get(dw, INST_LO) << 12) | (get(dw, INST_HI) << 32),
            addr_phys_aperture: get(dw, ADDR_PHYS_APERTURE) as u32,
            addr: (get(dw, ADDR_LO) << 12) | (get(dw, ADDR_HI) << 32),
            timestamp: get(dw, TIMESTAMP_LO) | (get(dw, TIMESTAMP_HI) << 32),
            engine_id: get(dw, ENGINE_ID) as u32,
            fault_type: get(dw, FAULT_TYPE) as u32,
            replayable: get(dw, REPLAYABLE_FAULT) != 0,
            client: get(dw, CLIENT) as u32,
            access_type: get(dw, ACCESS_TYPE) as u32,
            mmu_client_type: get(dw, MMU_CLIENT_TYPE) as u32,
            gpc_id: get(dw, GPC_ID) as u32,
            protected_mode: get(dw, PROTECTED_MODE) != 0,
            replayable_en: get(dw, REPLAYABLE_FAULT_EN) != 0,
            valid: get(dw, VALID) != 0,
        }
    }
}

/// The eight dwords as the 32 little-endian bytes the buffer holds.
#[must_use]
pub fn to_bytes(dw: &[u32; 8]) -> [u8; ENTRY_BYTES] {
    let mut b = [0u8; ENTRY_BYTES];
    for (i, w) in dw.iter().enumerate() {
        b[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes());
    }
    b
}

/// 32 little-endian bytes as eight dwords.
#[must_use]
pub fn from_bytes(b: &[u8; ENTRY_BYTES]) -> [u32; 8] {
    let mut dw = [0u32; 8];
    for (i, w) in dw.iter_mut().enumerate() {
        *w = u32::from_le_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]]);
    }
    dw
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> FaultEntry {
        FaultEntry {
            inst_addr: 0x0000_0012_3456_7000,
            inst_aperture: INST_APERTURE_SYS_MEM_COHERENT,
            addr: 0x0000_7f12_3456_7000,
            addr_phys_aperture: 0,
            timestamp: 0x1122_3344_5566_7788,
            engine_id: 64 + 3,
            fault_type: 2,
            replayable: true,
            client: 0x2b,
            access_type: 1,
            mmu_client_type: 0,
            gpc_id: 5,
            protected_mode: false,
            replayable_en: true,
            valid: true,
        }
    }

    /// The fields tile the 256 bits without overlapping, and each stays inside one dword.
    #[test]
    fn the_fields_do_not_overlap_and_stay_in_one_dword() {
        let mut used = [false; 256];
        for f in FIELDS {
            assert_eq!(f.hi / 32, f.lo / 32, "{} spans dwords", f.name);
            for b in f.lo..=f.hi {
                assert!(!used[b as usize], "{} overlaps bit {b}", f.name);
                used[b as usize] = true;
            }
        }
        assert!(used[255], "VALID is the top bit");
    }

    /// ★ Round trip: decode(encode(x)) == x for a packet that fills every field.
    #[test]
    fn a_packet_round_trips() {
        let e = sample();
        let dw = e.encode().expect("fits");
        assert_eq!(FaultEntry::decode(&dw), e);
        assert_eq!(FaultEntry::decode(&from_bytes(&to_bytes(&dw))), e);
    }

    /// The positions a guest parser reads, dword by dword (`clc369.h:36-67`), for one packet.
    #[test]
    fn the_bits_land_where_the_header_puts_them() {
        let dw = sample().encode().expect("fits");
        assert_eq!(dw[0], 0x3456_7000 | (2 << 8), "INST_LO | INST_APERTURE");
        assert_eq!(dw[1], 0x12, "INST_HI");
        assert_eq!(dw[2], 0x3456_7000, "ADDR_LO, phys aperture 0");
        assert_eq!(dw[3], 0x7f12, "ADDR_HI");
        assert_eq!(dw[4], 0x5566_7788);
        assert_eq!(dw[5], 0x1122_3344);
        assert_eq!(dw[6], 67, "ENGINE_ID");
        let d7 = 2 | (1 << 7) | (0x2b << 8) | (1 << 16) | (5 << 24) | (1 << 30) | (1 << 31);
        assert_eq!(dw[7], d7);
    }

    /// ★ The VALID bit is the ONLY difference between the two halves of a write.
    #[test]
    fn the_invalid_half_differs_only_in_valid() {
        let e = sample();
        let full = e.encode().expect("fits");
        let first = e.encode_invalid().expect("fits");
        for i in 0..8 {
            if i == VALID_DWORD {
                assert_eq!(first[i] | VALID_BIT, full[i]);
                assert_eq!(first[i] & VALID_BIT, 0);
            } else {
                assert_eq!(first[i], full[i]);
            }
        }
    }

    /// ⊘ A value wider than its field is refused by name, never truncated.
    #[test]
    fn an_overwide_value_is_refused() {
        let mut e = sample();
        e.gpc_id = 32;
        assert_eq!(
            e.encode(),
            Err(Overflow {
                field: "NVC369_BUF_ENTRY_GPC_ID",
                value: 32
            })
        );
        let mut e = sample();
        e.engine_id = 512;
        assert!(e.encode().is_err(), "ENGINE_ID is 9 bits");
        let mut e = sample();
        e.access_type = 16;
        assert!(e.encode().is_err(), "ACCESS_TYPE is 4 bits");
    }

    /// The low 12 bits of both addresses are not part of the packet.
    #[test]
    fn page_offsets_are_not_encoded() {
        let mut e = sample();
        e.addr |= 0xfff;
        e.inst_addr |= 0xabc;
        let d = FaultEntry::decode(&e.encode().expect("fits"));
        assert_eq!(d.addr, sample().addr);
        assert_eq!(d.inst_addr, sample().inst_addr);
    }
}
