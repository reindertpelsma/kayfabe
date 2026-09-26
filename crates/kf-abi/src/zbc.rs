//! ★ v3-gfx — `GF100_ZBC_CLEAR` (`0x9096`) control wire layouts (`ogkm-580: ctrl/ctrl9096.h`).
//!
//! The ZBC (zero-bandwidth clear) table is **GPU-global in hardware**: forwarding a guest's
//! `SET_ZBC_*` would put guest entries in a table every host process shares, with finite slots —
//! a cross-tenant resource. v3 therefore **authors a per-VM table** and never touches the host's
//! (`V3_HEADLESS_GRAPHICS.md` §2.2). This module is only the wire; the table is `kf_rm::zbc`.
//!
//! All the controls below carry `ROUTE_TO_PHYSICAL` (`g_zbc_api_nvoc.c:134-230`), so the guest's
//! CPU-RM forwards them verbatim to "GSP" (us). ⊘ `SET_ZBC_CLEAR_TABLE` (`0x90960104`) is
//! PRIVILEGED (flags `0x48`) and is not modelled — refused.

/// `NV9096_CTRL_CMD_SET_ZBC_COLOR_CLEAR`.
pub const SET_ZBC_COLOR_CLEAR: u32 = 0x9096_0101;
/// `NV9096_CTRL_CMD_SET_ZBC_DEPTH_CLEAR`.
pub const SET_ZBC_DEPTH_CLEAR: u32 = 0x9096_0102;
/// `NV9096_CTRL_CMD_GET_ZBC_CLEAR_TABLE`.
pub const GET_ZBC_CLEAR_TABLE: u32 = 0x9096_0103;
/// `NV9096_CTRL_CMD_SET_ZBC_CLEAR_TABLE` (privileged; not modelled).
pub const SET_ZBC_CLEAR_TABLE: u32 = 0x9096_0104;
/// `NV9096_CTRL_CMD_SET_ZBC_STENCIL_CLEAR`.
pub const SET_ZBC_STENCIL_CLEAR: u32 = 0x9096_0105;
/// `NV9096_CTRL_CMD_GET_ZBC_CLEAR_TABLE_SIZE`.
pub const GET_ZBC_CLEAR_TABLE_SIZE: u32 = 0x9096_0106;
/// `NV9096_CTRL_CMD_GET_ZBC_CLEAR_TABLE_ENTRY`.
pub const GET_ZBC_CLEAR_TABLE_ENTRY: u32 = 0x9096_0107;

/// `sizeof(NV9096_CTRL_SET_ZBC_COLOR_CLEAR_PARAMS)` — `colorFB[4] colorDS[4] format bSkipL2Table`
/// (the `NvBool` pads the struct to 4).
pub const SET_COLOR_PARAMS_SIZE: usize = 40;
/// `sizeof(NV9096_CTRL_SET_ZBC_DEPTH_CLEAR_PARAMS)` — `depth format bSkipL2Table`.
pub const SET_DEPTH_PARAMS_SIZE: usize = 12;
/// `sizeof(NV9096_CTRL_SET_ZBC_STENCIL_CLEAR_PARAMS)` — `stencil format bSkipL2Table`.
pub const SET_STENCIL_PARAMS_SIZE: usize = 12;
/// `sizeof(NV9096_CTRL_GET_ZBC_CLEAR_TABLE_PARAMS)` — `value{40} indexSize indexStart indexEnd
/// indexUsed format valType`.
pub const GET_TABLE_PARAMS_SIZE: usize = 64;
/// `sizeof(NV9096_CTRL_GET_ZBC_CLEAR_TABLE_SIZE_PARAMS)` — `indexStart indexEnd tableType`.
pub const GET_SIZE_PARAMS_SIZE: usize = 12;
/// `sizeof(NV9096_CTRL_GET_ZBC_CLEAR_TABLE_ENTRY_PARAMS)` — `value{40} format index bIndexValid
/// tableType`.
pub const GET_ENTRY_PARAMS_SIZE: usize = 56;

/// `NV9096_CTRL_ZBC_CLEAR_TABLE_TYPE_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TableType {
    /// `_COLOR` (1).
    Color,
    /// `_DEPTH` (2).
    Depth,
    /// `_STENCIL` (3).
    Stencil,
}

impl TableType {
    /// All three, in wire order.
    pub const ALL: [TableType; 3] = [TableType::Color, TableType::Depth, TableType::Stencil];

    /// The wire value.
    #[must_use]
    pub const fn wire(self) -> u32 {
        match self {
            TableType::Color => 1,
            TableType::Depth => 2,
            TableType::Stencil => 3,
        }
    }

    /// From the wire (`None` for INVALID/COUNT/anything else).
    #[must_use]
    pub const fn from_wire(v: u32) -> Option<TableType> {
        match v {
            1 => Some(TableType::Color),
            2 => Some(TableType::Depth),
            3 => Some(TableType::Stencil),
            _ => None,
        }
    }
}

/// One table value, as `GET_ZBC_CLEAR_TABLE_ENTRY` reports it (`value` + `format`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ZbcValue {
    /// `colorFB[4]` (color entries).
    pub color_fb: [u32; 4],
    /// `colorDS[4]` (color entries).
    pub color_ds: [u32; 4],
    /// `depth` (depth entries, FP32 bits).
    pub depth: u32,
    /// `stencil` (stencil entries).
    pub stencil: u32,
    /// `format`.
    pub format: u32,
}

/// Little-endian word `i` of `b`.
#[must_use]
pub fn word(b: &[u8], i: usize) -> Option<u32> {
    b.get(4 * i..4 * i + 4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
}

/// Write word `i` of `b` (no-op past the end — callers size `b` from the constants above).
pub fn put(b: &mut [u8], i: usize, v: u32) {
    if let Some(w) = b.get_mut(4 * i..4 * i + 4) {
        w.copy_from_slice(&v.to_le_bytes());
    }
}

/// Encode the `value` struct (words 0..10) of a GET reply.
pub fn put_value(b: &mut [u8], v: &ZbcValue) {
    for i in 0..4 {
        put(b, i, v.color_fb[i]);
        put(b, 4 + i, v.color_ds[i]);
    }
    put(b, 8, v.depth);
    put(b, 9, v.stencil);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sizes are the C structs' (`ctrl9096.h`), which the real host's own replies confirm:
    /// `[measured vgfx 2026-09-26]` the UMD issues `GET_ZBC_CLEAR_TABLE_SIZE` with paramsSize
    /// `0xc`, `GET_ZBC_CLEAR_TABLE_ENTRY` with `0x38`, `SET_ZBC_COLOR_CLEAR` with `0x28`.
    #[test]
    fn sizes_are_the_structs_the_umd_sends() {
        assert_eq!(GET_SIZE_PARAMS_SIZE, 0xc);
        assert_eq!(GET_ENTRY_PARAMS_SIZE, 0x38);
        assert_eq!(SET_COLOR_PARAMS_SIZE, 0x28);
        for t in TableType::ALL {
            assert_eq!(TableType::from_wire(t.wire()), Some(t));
        }
        assert_eq!(TableType::from_wire(0), None);
        assert_eq!(TableType::from_wire(4), None);
    }
}
