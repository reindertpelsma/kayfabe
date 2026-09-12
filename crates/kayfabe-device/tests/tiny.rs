//! ⊘ **The `TinyFmt` fixture, shared.** Lifted verbatim out of `ce_resolve.rs` at w523 so a
//! second test file can drive a real page-table walk without a second description of what a
//! page-table format is — a second fixture is a second source of truth, and this tree has
//! paid for that before.

#![allow(dead_code)]

use kayfabe_abi::gvaspacepdes::{GMMU_FMT_MAX_LEVELS, PdeLevel, ServerReservedPdes};
use kayfabe_arch::{Aperture, GmmuFmt, GmmuVersion, LevelShift, PageSize, PdeEdge, PteDecode};

pub struct TinyFmt;

pub const E_VALID: u128 = 1;
const E_SPARSE: u128 = 2;
pub const E_SYS: u128 = 4;

/// The address field holds the page frame, so an entry naming byte address `a` stores
/// `a >> 12` at bit 12 — the same shape every real format has, and the reason `decode_entry`
/// shifts it back rather than returning the field.
fn pde(next: u64) -> u128 {
    E_VALID | (u128::from(next >> 12) << 12)
}
fn leaf(phys: u64) -> u128 {
    E_VALID | (u128::from(phys >> 12) << 12)
}
fn leaf_sys(phys: u64) -> u128 {
    E_VALID | E_SYS | (u128::from(phys >> 12) << 12)
}

impl GmmuFmt for TinyFmt {
    fn version(&self) -> GmmuVersion {
        GmmuVersion::Ver2
    }
    fn page_sizes(&self) -> &[PageSize] {
        &[PageSize(2 << 20)]
    }
    fn entry_size(&self, level: u8) -> u8 {
        if level < 2 { 8 } else { 0 }
    }
    fn levels(&self) -> u8 {
        2
    }
    fn level_shift(&self, level: u8) -> Option<LevelShift> {
        match level {
            0 => Some(LevelShift {
                shift: 30,
                entries: 512,
            }),
            1 => Some(LevelShift {
                shift: 21,
                entries: 512,
            }),
            _ => None,
        }
    }
    fn decode_entry(&self, level: u8, raw: u128) -> PteDecode {
        if raw & E_SPARSE != 0 {
            return PteDecode::Sparse;
        }
        if raw & E_VALID == 0 {
            return PteDecode::Invalid;
        }
        let phys = ((raw >> 12) as u64) << 12;
        let aperture = if raw & E_SYS != 0 {
            Aperture::SysmemCoherent
        } else {
            Aperture::Vidmem
        };
        if level == 0 {
            PteDecode::Pde {
                edge: PdeEdge {
                    next: phys,
                    aperture,
                    child_level: 1,
                },
                also: None,
            }
        } else {
            PteDecode::Leaf {
                phys,
                aperture,
                size: PageSize(2 << 20),
                read_only: false,
            }
        }
    }
}
