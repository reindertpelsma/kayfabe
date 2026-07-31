//! The `GET_GSP_STATIC_INFO` (fn 65) **reply body**: `GspStaticConfigInfo`, and in
//! particular the FB region table the guest's memory manager cannot start without.
//!
//! ## ★★★ Why this one is not a control, and why that matters here
//!
//! Every other reply this crate encodes is a `GSP_RM_CONTROL` — a header the guest wrote,
//! with a `status` and a `paramsSize` we overwrite. Fn 65 has none of that. The GSP posts
//! the whole `GspStaticConfigInfo` struct at the RPC body and the guest copies it into
//! `pGpu->pGspStaticInfo` unconditionally, so there is no field in which to say *"not
//! served"*. A zero body is not a refusal; it is an answer that says this GPU has no
//! memory. Measured on a stock 580.159.04 guest at `d1ba379`, where fn 65 fell through to
//! the FSM's bare acknowledgement — the request's own zeroed body, echoed:
//!
//! ```text
//! NVRM: memmgrInitBaseFbRegions_FWCLIENT: Missing FB region table in GSP Init arguments.
//! NVRM: ... [NV_ERR_INVALID_PARAMETER] (0x3B) from memmgrInitBaseFbRegions_HAL @ mem_mgr.c:3417
//! NVRM: ... [NV_ERR_INVALID_PARAMETER] (0x3B) from memmgrInitFbRegions @ mem_mgr.c:392
//! NVRM: RmInitNvDevice: *** Cannot pre-initialize the device
//! NVRM: GPU 0000:00:03.0: RmInitAdapter failed! (0x23:0x3b:1206)
//! ```
//!
//! `memmgrInitBaseFbRegions_FWCLIENT` is `ogkm-580:
//! src/nvidia/src/kernel/gpu/mem_mgr/mem_mgr_gsp_client.c:61-67`; the `numFBRegions == 0`
//! branch is the line above.
//!
//! ## ★★ A region is a PROMISE, not a description
//!
//! Every byte inside a region with `reserved == 0` is memory the guest's heap will hand to
//! a client and expect to work (`ogkm-580: mem_mgr_gsp_client.c:104-105` accumulates it
//! into `fbUsableMemSize`; `ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/heap.c:533-618`
//! builds the allocator over it). A region invented to look plausible is a promise the
//! data plane is asked to keep thousands of records later, at an allocation that names
//! nothing about this file. So the rows are a chip-row decision
//! (`kayfabe_device::ChipProfile::fb_regions`), this module is only the layout, and the
//! encoder [refuses](GspStaticInfoError) a table whose geometry does not describe the FB
//! the same device advertises.
//!
//! ## ★ The layout is pinned against real hardware bytes, not against a header read
//!
//! The offsets below are computed from the `ogkm-580` struct declarations, and then
//! **checked against an RTX 3060's own answer**: the C artifact's captured reply
//! (`C: src/qemu/mode2_gspstaticinfo_ga106.h`, 1792 bytes) appears verbatim, exactly once,
//! in `traces/mode2_c_reference/cap1b_coldboot_hermetic_d6` — record 141977, a 4096-byte
//! `GuestWrite` to `0x1_2764_4000`, at payload offset 80 (the element's 48-byte header
//! plus the 32-byte RPC header). `crates/kayfabe-device/tests/gsp_static_info.rs` feeds
//! that capture's five rows back through [`encode_gsp_static_info`] and requires the
//! bytes out to equal the bytes in.

use crate::versions::GspStaticInfoWire;

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

/// `sizeof(GspStaticConfigInfo)` at the 580 shape — the body length the guest's own
/// request declares (`rpc.length` 1824 = 32-byte RPC header + this), and the length the C
/// artifact's captured reply carries.
///
/// A measured constant rather than a sum of the whole struct: the fields past
/// [`FB_LENGTH_OFF`] are all left zero by this port, so their layout is not modelled and
/// writing it out would be a description of nothing. Read from `cap1b_coldboot_hermetic_d6`
/// record 141977 (`rpc.length` at element+56 = 1824) and from the capture log the C header
/// was generated from (`C: docs/research/captures/ga106_gspstaticinfo_580.log:1`,
/// `NVKVM_CAPGSI size=1792`).
pub const GSP_STATIC_CONFIG_INFO_SIZE: usize = 1792;

/// `NV0080_CTRL_GR_CAPS_TBL_SIZE` (`ogkm-580:
/// src/common/sdk/nvidia/inc/ctrl/ctrl0080/ctrl0080gr.h:78`) — `grCapsBits[]`, the first
/// member.
const GR_CAPS_BITS_SIZE: usize = 23;

/// `NV2080_GPU_MAX_GID_LENGTH` — `gidInfo.data[]`.
const GID_DATA_SIZE: usize = 256;

/// `sizeof(NV2080_CTRL_GPU_GET_GID_INFO_PARAMS)`: `index`, `flags`, `length`, `data[256]`
/// (`ogkm-580: ctrl2080gpu.h:1785-1790`).
const GID_INFO_SIZE: usize = 12 + GID_DATA_SIZE;

/// `sizeof(NV2080_CTRL_BIOS_GET_SKU_INFO_PARAMS)` (`ogkm-580: ctrl2080bios.h:376-386`):
/// `BoardID` u32, `chipSKU[9]`, `chipSKUMod[5]`, u32 `skuConfigVersion` (3 bytes of
/// padding ahead of it), `project[5]`, `projectSKU[5]`, `CDP[6]`, `projectSKUMod[2]`,
/// then u32 `businessCycle` (2 more bytes of padding).
const SKU_INFO_SIZE: usize = 48;

/// `sizeof(NV0080_CTRL_GPU_GET_SRIOV_CAPS_PARAMS)` (`ogkm-580: ctrl0080gpu.h:481-500`):
/// three u32 (padded to 16), six u64, nine `NvBool`, rounded to the struct's 8-byte
/// alignment.
const SRIOV_CAPS_SIZE: usize = 80;

/// `NVGPU_ENGINE_CAPS_MASK_ARRAY_MAX` = `(RM_ENGINE_TYPE_LAST - 1) / 32 + 1`
/// (`ogkm-580: src/nvidia/inc/kernel/gpu/nvbitmask.h:35`), with `RM_ENGINE_TYPE_LAST ==
/// NV2080_ENGINE_TYPE_LAST == 0x54` (`ogkm-580: gpu_engine_type.c:32` and
/// `src/common/sdk/nvidia/inc/class/cl2080_notification.h:373`).
const ENGINE_CAPS_WORDS: usize = (0x54 - 1) / 32 + 1;

/// Round `off` up to `align`, which must be a power of two.
const fn align_up(off: usize, align: usize) -> usize {
    off.next_multiple_of(align)
}

/// Byte offset of `fbRegionInfoParams` — after `grCapsBits[23]`, `gidInfo` and `SKUInfo`,
/// realigned to 8 because the member is `NV_DECLARE_ALIGNED(..., 8)`.
///
/// ⚠ The realignment is four bytes that no field name mentions. Packing the table at 340
/// puts `numFBRegions` where `SKUInfo.businessCycle`'s padding is, and the guest reads
/// zero — i.e. it reproduces the exact failure this module exists to fix, while looking
/// like it served a table.
pub const FB_REGION_INFO_PARAMS_OFF: usize = align_up(
    align_up(GR_CAPS_BITS_SIZE, 4) + GID_INFO_SIZE + SKU_INFO_SIZE,
    8,
);

/// `NV2080_CTRL_CMD_FB_GET_FB_REGION_INFO_MEM_TYPES` — the width of the deprecated
/// `blackList[]` (`ogkm-580: ctrl2080fb.h:862`).
const FB_REGION_MEM_TYPES: usize = 18;

/// `sizeof(NV2080_CTRL_CMD_FB_GET_FB_REGION_FB_REGION_INFO)`: three u64, a u32, three
/// `NvBool`, `blackList[18]`, rounded to the 8-byte alignment the u64 members force
/// (`ogkm-580: ctrl2080fb.h:866-875`).
pub const FB_REGION_ENTRY_SIZE: usize = align_up(24 + 4 + 3 + FB_REGION_MEM_TYPES, 8);

/// `NV2080_CTRL_CMD_FB_GET_FB_REGION_INFO_MAX_ENTRIES` (`ogkm-580: ctrl2080fb.h:877`).
/// Also the bound RM itself checks against, as `MAX_FB_REGIONS`
/// (`ogkm-580: src/nvidia/generated/g_mem_mgr_nvoc.h:440`, and the `ct_assert` tying the
/// two together at `mem_mgr_ctrl.c:655`).
pub const FB_REGION_MAX_ENTRIES: usize = 16;

/// `sizeof(NV2080_CTRL_CMD_FB_GET_FB_REGION_INFO_PARAMS)`: `numFBRegions`, four bytes of
/// alignment padding, then `fbRegion[16]`.
pub const FB_REGION_INFO_PARAMS_SIZE: usize = 8 + FB_REGION_MAX_ENTRIES * FB_REGION_ENTRY_SIZE;

/// Byte offset of `fb_length` — `fbRegionInfoParams`, `sriovCaps`, `sriovMaxGfid`,
/// `engineCaps[]`, `poisonFuseEnabled`, realigned to 8.
///
/// ★ This is the FB size stated a **second** time: `NV_USABLE_FB_SIZE_IN_MB` states it
/// first, and RM takes `fbTotalMemSizeMb` from here while taking `fbAddrSpaceSizeMb` from
/// the last region's limit (`ogkm-580: mem_mgr_gsp_client.c:118-120`). Three statements of
/// one fact; [`encode_gsp_static_info`] refuses the two it can see disagree.
pub const FB_LENGTH_OFF: usize = align_up(
    FB_REGION_INFO_PARAMS_OFF
        + FB_REGION_INFO_PARAMS_SIZE
        + SRIOV_CAPS_SIZE
        + 4
        + ENGINE_CAPS_WORDS * 4
        + 1,
    8,
);

/// One FB region, as the guest's RM will file it.
///
/// Deliberately **not** `#[repr(C)]`: [`encode_gsp_static_info`] is the only thing that
/// knows the wire layout, so a row is free to be ergonomic. The field docs are
/// `ogkm-580: ctrl2080fb.h:820-856`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FbRegion {
    /// First valid address in the region.
    pub base: u64,
    /// Last valid address in the region — `limit - base + 1` is its size.
    pub limit: u64,
    /// How much of the region RM must leave alone. ★ **Non-zero makes the whole region a
    /// reserved one**: RM sets `bRsvdRegion` on any non-zero value and adds the size to
    /// `reservedMemSize` instead of `fbUsableMemSize`, whatever the number was
    /// (`ogkm-580: mem_mgr_gsp_client.c:96-106`). There is no partial reservation.
    pub reserved: u64,
    /// Relative bandwidth; higher is faster. Only compared between regions.
    pub performance: u32,
    /// Whether compressed kinds may be allocated here.
    pub support_compressed: bool,
    /// Whether ISO kinds (display, cursor, video) may be allocated here.
    pub support_iso: bool,
    /// Whether only `NVOS32_ALLOC_FLAGS_PROTECTED` allocations may land here.
    pub protected: bool,
}

/// The static facts this port is willing to state about one GPU.
///
/// ⊘ Deliberately not "the struct". Every field of `GspStaticConfigInfo` this type does
/// not name is left **zero**, which is this port saying it does not advertise the thing —
/// and the next guest-side failure names which one it wanted. Growing this type is how a
/// rung is climbed; pre-filling it from a capture is how a failure is moved somewhere it
/// cannot be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GspStaticInfo<'a> {
    /// `fbRegionInfoParams.fbRegion[]`, in ascending address order.
    pub fb_regions: &'a [FbRegion],
    /// `fb_length` — the framebuffer this device advertises, in bytes.
    pub fb_length: u64,
}

/// A `GspStaticConfigInfo` this port will not put on the wire.
///
/// Every variant is a table whose geometry contradicts something the same device has
/// already told the same guest. The alternative to refusing is a heap built over memory
/// nothing backs, whose first symptom is a fault at an address that names none of this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GspStaticInfoError {
    /// No regions at all — which is the defect this module exists to fix, so producing it
    /// from the encoder side is refused rather than shipped.
    NoRegions,
    /// More regions than `fbRegion[]` holds.
    TooManyRegions {
        /// How many rows were offered.
        len: usize,
        /// The array bound, [`FB_REGION_MAX_ENTRIES`].
        max: usize,
    },
    /// A region whose `limit` is below its `base`, i.e. a region of negative size.
    RegionInverted {
        /// Which row.
        index: usize,
        /// Its `base`.
        base: u64,
        /// Its `limit`.
        limit: u64,
    },
    /// Two regions that are not in ascending, non-overlapping order. RM indexes the last
    /// row for `fbAddrSpaceSizeMb` (`ogkm-580: mem_mgr_gsp_client.c:119-120`) and the heap
    /// walks them in order, so "ascending" is not a tidiness rule.
    RegionsOutOfOrder {
        /// The row that starts too low.
        index: usize,
        /// Where it starts.
        base: u64,
        /// Where its predecessor ended.
        prev_limit: u64,
    },
    /// The regions do not describe the framebuffer this device advertises: the last
    /// region's `limit + 1` is not `fb_length`. RM reads the two independently and
    /// believes both.
    RegionsDoNotSpanFb {
        /// `last.limit + 1`.
        top: u64,
        /// The `fb_length` in the same reply.
        fb_length: u64,
    },
    /// This port does not encode this driver version's `GspStaticConfigInfo`.
    UnsupportedWire {
        /// The shape asked for.
        wire: GspStaticInfoWire,
    },
}

impl core::fmt::Display for GspStaticInfoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoRegions => write!(
                f,
                "an empty FB region table is the answer 'this GPU has no memory'; RM \
                 rejects it as NV_ERR_INVALID_PARAMETER"
            ),
            Self::TooManyRegions { len, max } => {
                write!(f, "{len} FB regions; fbRegion[] holds {max}")
            }
            Self::RegionInverted { index, base, limit } => write!(
                f,
                "FB region {index} has limit {limit:#x} below base {base:#x}; limit is the \
                 last valid address, not a size"
            ),
            Self::RegionsOutOfOrder {
                index,
                base,
                prev_limit,
            } => write!(
                f,
                "FB region {index} starts at {base:#x}, at or below the previous region's \
                 limit {prev_limit:#x}; RM reads the table in order"
            ),
            Self::RegionsDoNotSpanFb { top, fb_length } => write!(
                f,
                "the FB regions end at {top:#x} but fb_length is {fb_length:#x}; RM takes \
                 fbAddrSpaceSizeMb from the one and fbTotalMemSizeMb from the other"
            ),
            Self::UnsupportedWire { wire } => write!(
                f,
                "this port does not encode the {wire:?} shape of GspStaticConfigInfo"
            ),
        }
    }
}

impl core::error::Error for GspStaticInfoError {}

/// Encode a `GspStaticConfigInfo` reply body.
///
/// The result is always [`GSP_STATIC_CONFIG_INFO_SIZE`] bytes, zero-filled outside the
/// fields [`GspStaticInfo`] names — RM copies the whole struct into its own storage and
/// reads fields this port has not reached yet, so a short body would leave them holding
/// the request's bytes rather than zero.
///
/// # Errors
///
/// Any [`GspStaticInfoError`]: a region table that contradicts itself or the advertised
/// `fb_length`, or a driver version whose struct shape this port does not encode.
pub fn encode_gsp_static_info(
    info: &GspStaticInfo<'_>,
    wire: GspStaticInfoWire,
) -> Result<Vec<u8>, GspStaticInfoError> {
    if wire != GspStaticInfoWire::Pre610 {
        return Err(GspStaticInfoError::UnsupportedWire { wire });
    }
    let regions = info.fb_regions;
    if regions.is_empty() {
        return Err(GspStaticInfoError::NoRegions);
    }
    if regions.len() > FB_REGION_MAX_ENTRIES {
        return Err(GspStaticInfoError::TooManyRegions {
            len: regions.len(),
            max: FB_REGION_MAX_ENTRIES,
        });
    }
    for (i, r) in regions.iter().enumerate() {
        if r.limit < r.base {
            return Err(GspStaticInfoError::RegionInverted {
                index: i,
                base: r.base,
                limit: r.limit,
            });
        }
        if i > 0 {
            let prev_limit = regions[i - 1].limit;
            if r.base <= prev_limit {
                return Err(GspStaticInfoError::RegionsOutOfOrder {
                    index: i,
                    base: r.base,
                    prev_limit,
                });
            }
        }
    }
    let top = regions[regions.len() - 1].limit.wrapping_add(1);
    if top != info.fb_length {
        return Err(GspStaticInfoError::RegionsDoNotSpanFb {
            top,
            fb_length: info.fb_length,
        });
    }

    let mut body = vec![0u8; GSP_STATIC_CONFIG_INFO_SIZE];
    let n = u32::try_from(regions.len()).unwrap_or(u32::MAX);
    let p = FB_REGION_INFO_PARAMS_OFF;
    body[p..p + 4].copy_from_slice(&n.to_le_bytes());
    for (i, r) in regions.iter().enumerate() {
        let o = p + 8 + i * FB_REGION_ENTRY_SIZE;
        body[o..o + 8].copy_from_slice(&r.base.to_le_bytes());
        body[o + 8..o + 16].copy_from_slice(&r.limit.to_le_bytes());
        body[o + 16..o + 24].copy_from_slice(&r.reserved.to_le_bytes());
        body[o + 24..o + 28].copy_from_slice(&r.performance.to_le_bytes());
        body[o + 28] = u8::from(r.support_compressed);
        body[o + 29] = u8::from(r.support_iso);
        body[o + 30] = u8::from(r.protected);
        // `blackList[]` stays zero: it is documented DEPRECATED in favour of `supportISO`
        // (`ogkm-580: ctrl2080fb.h:855-856`), and the RTX 3060 capture leaves all 18 bytes
        // of every one of its five rows at zero.
    }
    body[FB_LENGTH_OFF..FB_LENGTH_OFF + 8].copy_from_slice(&info.fb_length.to_le_bytes());
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal well-formed table: one usable region spanning a 1 MiB framebuffer.
    fn one_region(fb: u64) -> [FbRegion; 1] {
        [FbRegion {
            base: 0,
            limit: fb - 1,
            reserved: 0,
            performance: 6,
            support_compressed: true,
            support_iso: true,
            protected: false,
        }]
    }

    #[test]
    fn the_offsets_are_the_ones_the_580_struct_produces() {
        // Literals, deliberately: reading these back off the constants under test would
        // make the assertions agree with the arithmetic rather than with the struct.
        assert_eq!(FB_REGION_INFO_PARAMS_OFF, 344);
        assert_eq!(FB_REGION_ENTRY_SIZE, 56);
        assert_eq!(FB_REGION_INFO_PARAMS_SIZE, 904);
        assert_eq!(FB_LENGTH_OFF, 1352);
        assert_eq!(GSP_STATIC_CONFIG_INFO_SIZE, 1792);
        // The alignment hole ahead of the region table is four bytes nothing names.
        assert_eq!(FB_REGION_INFO_PARAMS_OFF - (24 + 268 + 48), 4);
    }

    #[test]
    fn the_body_is_the_whole_struct_and_zero_outside_what_we_state() {
        let fb = 1 << 20;
        let body = encode_gsp_static_info(
            &GspStaticInfo {
                fb_regions: &one_region(fb),
                fb_length: fb,
            },
            GspStaticInfoWire::Pre610,
        )
        .expect("encodes");
        assert_eq!(body.len(), 1792);
        assert!(
            body[..344].iter().all(|b| *b == 0),
            "grCaps/gid/SKU untouched"
        );
        assert!(
            body[1248..1352].iter().all(|b| *b == 0),
            "sriov/engineCaps untouched"
        );
        assert!(
            body[1360..].iter().all(|b| *b == 0),
            "the tail past fb_length"
        );
        assert_eq!(u32::from_le_bytes(body[344..348].try_into().unwrap()), 1);
        assert_eq!(u64::from_le_bytes(body[1352..1360].try_into().unwrap()), fb);
    }

    #[test]
    fn an_empty_table_is_refused_rather_than_encoded() {
        let e = encode_gsp_static_info(
            &GspStaticInfo {
                fb_regions: &[],
                fb_length: 1 << 20,
            },
            GspStaticInfoWire::Pre610,
        )
        .expect_err("refuses");
        assert_eq!(e, GspStaticInfoError::NoRegions);
    }

    #[test]
    fn more_regions_than_the_array_holds_is_refused() {
        let mut rows = [FbRegion {
            base: 0,
            limit: 0,
            reserved: 0,
            performance: 0,
            support_compressed: false,
            support_iso: false,
            protected: false,
        }; 17];
        for (i, r) in rows.iter_mut().enumerate() {
            r.base = (i as u64) << 20;
            r.limit = r.base + 0xf_ffff;
        }
        let e = encode_gsp_static_info(
            &GspStaticInfo {
                fb_regions: &rows,
                fb_length: 17 << 20,
            },
            GspStaticInfoWire::Pre610,
        )
        .expect_err("refuses");
        assert_eq!(e, GspStaticInfoError::TooManyRegions { len: 17, max: 16 });
    }

    #[test]
    fn an_inverted_region_is_refused() {
        let rows = [FbRegion {
            base: 0x2000,
            limit: 0x1000,
            reserved: 0,
            performance: 0,
            support_compressed: false,
            support_iso: false,
            protected: false,
        }];
        let e = encode_gsp_static_info(
            &GspStaticInfo {
                fb_regions: &rows,
                fb_length: 0x1001,
            },
            GspStaticInfoWire::Pre610,
        )
        .expect_err("refuses");
        assert_eq!(
            e,
            GspStaticInfoError::RegionInverted {
                index: 0,
                base: 0x2000,
                limit: 0x1000
            }
        );
    }

    #[test]
    fn overlapping_regions_are_refused() {
        let rows = [
            FbRegion {
                base: 0,
                limit: 0xffff,
                reserved: 0,
                performance: 0,
                support_compressed: false,
                support_iso: false,
                protected: false,
            },
            FbRegion {
                base: 0x8000,
                limit: 0x1_ffff,
                reserved: 0,
                performance: 0,
                support_compressed: false,
                support_iso: false,
                protected: false,
            },
        ];
        let e = encode_gsp_static_info(
            &GspStaticInfo {
                fb_regions: &rows,
                fb_length: 0x2_0000,
            },
            GspStaticInfoWire::Pre610,
        )
        .expect_err("refuses");
        assert_eq!(
            e,
            GspStaticInfoError::RegionsOutOfOrder {
                index: 1,
                base: 0x8000,
                prev_limit: 0xffff
            }
        );
    }

    #[test]
    fn a_table_that_does_not_span_the_advertised_fb_is_refused() {
        let fb = 1 << 20;
        let e = encode_gsp_static_info(
            &GspStaticInfo {
                fb_regions: &one_region(fb),
                fb_length: fb * 2,
            },
            GspStaticInfoWire::Pre610,
        )
        .expect_err("refuses");
        assert_eq!(
            e,
            GspStaticInfoError::RegionsDoNotSpanFb {
                top: fb,
                fb_length: fb * 2
            }
        );
    }

    #[test]
    fn a_shape_this_port_has_never_seen_on_a_wire_is_refused_by_name() {
        let fb = 1 << 20;
        let e = encode_gsp_static_info(
            &GspStaticInfo {
                fb_regions: &one_region(fb),
                fb_length: fb,
            },
            GspStaticInfoWire::From610_43_02,
        )
        .expect_err("refuses");
        assert_eq!(
            e,
            GspStaticInfoError::UnsupportedWire {
                wire: GspStaticInfoWire::From610_43_02
            }
        );
    }
}
