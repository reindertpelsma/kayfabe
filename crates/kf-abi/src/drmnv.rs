// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★ **The nvidia-drm / NVKMS private ABI of the GPU-copy broker rung** (`docs/design/V3_DISPLAY.md`
//! §8.11, `OWNER_RULINGS.md` §L).
//!
//! The rung wraps a VRAM frame object kayfabe allocated (never guest memory) in a dma-buf through
//! the host GPU's DRM render node. Two of the four ioctls it issues are nvidia-drm's own, and one
//! of them carries a pointer to NVKMS's private import struct:
//!
//! | what | C name | where (`ogkm-580:`) |
//! |---|---|---|
//! | `DRM_IOCTL_NVIDIA_GET_DEV_INFO` | `struct drm_nvidia_get_dev_info_params` (36 B) | `kernel-open/nvidia-drm/nvidia-drm-ioctl.h:183-197`, handler `nvidia-drm-drv.c:1029-1064` |
//! | `DRM_IOCTL_NVIDIA_GEM_IMPORT_NVKMS_MEMORY` | `struct drm_nvidia_gem_import_nvkms_memory_params` (32 B) | `nvidia-drm-ioctl.h:166-175` |
//! | its `nvkms_params_ptr` | `struct NvKmsKapiPrivImportMemoryParams` (28 B) | `src/nvidia-modeset/kapi/interface/nvkms-kapi-private.h:37-55` |
//!
//! (`PRIME_HANDLE_TO_FD`, `GEM_CLOSE` and `DMA_BUF_IOCTL_EXPORT_SYNC_FILE` are the kernel's own uapi
//! and live in `kf-linux-raw`, by the ABI-quarantine rule.)
//!
//! ⊘ **None of these is a stable ABI, and a mismatch is SILENT.** The DRM core zero-fills or
//! truncates an argument whose size differs from the kernel's rather than refusing it, so a field
//! inserted mid-struct at some driver tag would be read as another field. The rung is therefore
//! offered only at a host driver tag where `tools/drivermatrix/drmnv.py` COMPILED that tag's own
//! headers and every value equalled the transcription below ([`abi_measured`], over the committed
//! `traces/driver_matrix/drmnv.tsv`); anywhere else it is refused by name. Encoding is explicit
//! byte placement (no `#[repr(C)]`, this crate's convention), and the pointer field is written only
//! inside `kf-linux-raw`'s ioctl, never here.

use crate::host_driver::HostDriverVersion;

/// `DRM_IOCTL_BASE` (`'d'`, `linux: include/uapi/drm/drm.h`).
pub const DRM_IOCTL_BASE: u32 = b'd' as u32;
/// `DRM_COMMAND_BASE` (`drm.h`): driver-private ioctl numbers start here.
pub const DRM_COMMAND_BASE: u32 = 0x40;
/// `DRM_NVIDIA_GEM_IMPORT_NVKMS_MEMORY` (`nvidia-drm-ioctl.h:33`).
pub const NR_GEM_IMPORT_NVKMS_MEMORY: u32 = 0x01;
/// `DRM_NVIDIA_GET_DEV_INFO` (`nvidia-drm-ioctl.h:35`).
pub const NR_GET_DEV_INFO: u32 = 0x03;

/// `_IOWR(type, nr, size)` (`asm-generic/ioctl.h`: dir 3 at bit 30, size at 16, type at 8).
#[must_use]
pub const fn iowr(ty: u32, nr: u32, size: u32) -> u32 {
    (3 << 30) | ((size & 0x3fff) << 16) | (ty << 8) | nr
}

/// `sizeof(struct drm_nvidia_get_dev_info_params)`.
pub const GET_DEV_INFO_SIZE: usize = 36;
/// `sizeof(struct drm_nvidia_gem_import_nvkms_memory_params)`.
pub const GEM_IMPORT_SIZE: usize = 32;
/// `sizeof(struct NvKmsKapiPrivImportMemoryParams)`.
pub const PRIV_IMPORT_SIZE: usize = 28;

/// `DRM_IOCTL_NVIDIA_GET_DEV_INFO` = `DRM_IOWR(0x43, struct drm_nvidia_get_dev_info_params)`.
pub const IOCTL_GET_DEV_INFO: u32 = iowr(
    DRM_IOCTL_BASE,
    DRM_COMMAND_BASE + NR_GET_DEV_INFO,
    GET_DEV_INFO_SIZE as u32,
);
/// `DRM_IOCTL_NVIDIA_GEM_IMPORT_NVKMS_MEMORY` = `DRM_IOWR(0x41, …import_nvkms_memory_params)`.
pub const IOCTL_GEM_IMPORT_NVKMS_MEMORY: u32 = iowr(
    DRM_IOCTL_BASE,
    DRM_COMMAND_BASE + NR_GEM_IMPORT_NVKMS_MEMORY,
    GEM_IMPORT_SIZE as u32,
);

// ── GET_DEV_INFO ─────────────────────────────────────────────────────────────────────────

/// ★ What `DRM_IOCTL_NVIDIA_GET_DEV_INFO` answered (`nvidia-drm-drv.c:1029-1064`).
/// `supports_alloc` is true only when NVKMS is attached to the device, i.e. nvidia-drm runs with
/// `modeset=1`; the three modifier fields are valid only then.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DevInfo {
    /// `gpu_id` — RM's gpu id (whether it equals `CARD_INFO`'s is UNVERIFIED; the PCI address is
    /// the identity kf3 decides on).
    pub gpu_id: u32,
    /// `mig_device`.
    pub mig_device: u32,
    /// `primary_index` — the `card%d` number.
    pub primary_index: u32,
    /// `supports_alloc` — NVKMS memory import/allocation works on this device.
    pub supports_alloc: bool,
    /// `generic_page_kind` — the modifier's `k` (0x06 on Turing+).
    pub generic_page_kind: u32,
    /// `page_kind_generation` — the modifier's `g` (2 on Turing+, `nvidia-drm-drv.c:752-755`).
    pub page_kind_generation: u32,
    /// `sector_layout` — the modifier's `s` (hard-coded 1, `:757-758`).
    pub sector_layout: u32,
    /// `supports_sync_fd`.
    pub supports_sync_fd: bool,
    /// `supports_semsurf`.
    pub supports_semsurf: bool,
}

impl DevInfo {
    /// Decode the ioctl's 36-byte reply.
    #[must_use]
    pub fn decode(b: &[u8; GET_DEV_INFO_SIZE]) -> DevInfo {
        let u = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        DevInfo {
            gpu_id: u(0),
            mig_device: u(4),
            primary_index: u(8),
            supports_alloc: u(12) != 0,
            generic_page_kind: u(16),
            page_kind_generation: u(20),
            sector_layout: u(24),
            supports_sync_fd: u(28) != 0,
            supports_semsurf: u(32) != 0,
        }
    }

    /// ★ The `DRM_FORMAT_MOD_NVIDIA_BLOCK_LINEAR_2D` modifier of an uncompressed 32 bpp surface
    /// in this device's generic kind with `2^h_log2`-GOB blocks — what an ATTACH of a kayfabe VRAM
    /// frame names. `None` without `supports_alloc` (the fields are then zero, not values).
    #[must_use]
    pub fn block_linear_modifier(&self, h_log2: u32) -> Option<u64> {
        self.supports_alloc.then(|| {
            block_linear_2d(
                0,
                self.sector_layout,
                self.page_kind_generation,
                self.generic_page_kind,
                h_log2,
            )
        })
    }
}

/// `DRM_FORMAT_MOD_VENDOR_NVIDIA` (`drm_fourcc.h`).
pub const MOD_VENDOR_NVIDIA: u64 = 0x03;

/// ★ `DRM_FORMAT_MOD_NVIDIA_BLOCK_LINEAR_2D(c, s, g, k, h)` (`linux: include/uapi/drm/drm_fourcc.h`,
/// Linux 7.1-rc6 `:1012-1019`), transcribed term by term — including the sector layout's upper
/// two bits at 26:27 (GB20x 8/16 bpp layouts).
#[must_use]
pub const fn block_linear_2d(c: u32, s: u32, g: u32, k: u32, h: u32) -> u64 {
    let v = 0x10u64
        | (h as u64 & 0xf)
        | ((k as u64 & 0xff) << 12)
        | ((g as u64 & 0x3) << 20)
        | ((s as u64 & 0x1) << 22)
        | ((s as u64 & 0x6) << 25)
        | ((c as u64 & 0x7) << 23);
    (MOD_VENDOR_NVIDIA << 56) | (v & 0x00ff_ffff_ffff_ffff)
}

// ── GEM_IMPORT_NVKMS_MEMORY ──────────────────────────────────────────────────────────────

/// `mem_size` @ +0.
pub const GEM_IMPORT_MEM_SIZE_AT: usize = 0;
/// ★ `nvkms_params_ptr` @ +8 — a user pointer, written ONLY by `kf-linux-raw` (the address of its
/// own copy of the private params, scrubbed after the call).
pub const GEM_IMPORT_PTR_AT: usize = 8;
/// `nvkms_params_size` @ +16 — must equal [`PRIV_IMPORT_SIZE`] (`nvkms-kapi.c:1677-1683` checks it
/// exactly; `kf-linux-raw` re-checks it against the buffer it points at).
pub const GEM_IMPORT_PARAMS_SIZE_AT: usize = 16;
/// `handle` @ +24 (OUT).
pub const GEM_IMPORT_HANDLE_AT: usize = 24;

/// ★ The import's argument for an object of `mem_size` bytes, the pointer left ZERO.
#[must_use]
pub fn gem_import_arg(mem_size: u64) -> [u8; GEM_IMPORT_SIZE] {
    let mut b = [0u8; GEM_IMPORT_SIZE];
    b[GEM_IMPORT_MEM_SIZE_AT..GEM_IMPORT_MEM_SIZE_AT + 8].copy_from_slice(&mem_size.to_le_bytes());
    b[GEM_IMPORT_PARAMS_SIZE_AT..GEM_IMPORT_PARAMS_SIZE_AT + 8]
        .copy_from_slice(&(PRIV_IMPORT_SIZE as u64).to_le_bytes());
    b
}

/// The GEM handle the import wrote.
#[must_use]
pub fn gem_import_handle(b: &[u8; GEM_IMPORT_SIZE]) -> u32 {
    let o = GEM_IMPORT_HANDLE_AT;
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// `NvKmsSurfaceMemoryLayoutBlockLinear` (`nvkms-api-types.h:558-561`).
pub const LAYOUT_BLOCK_LINEAR: u32 = 0;
/// `NvKmsSurfaceMemoryLayoutPitch`.
pub const LAYOUT_PITCH: u32 = 1;

/// `memFd` @ +0 of the private params — written by `kf-linux-raw` from a borrowed descriptor that
/// stays open for the whole call.
pub const PRIV_IMPORT_FD_AT: usize = 0;

/// ★ `NvKmsKapiPrivSurfaceParams` for a block-linear object (`nvkms-kapi-private.h:37-49`). NVKMS
/// consults it only for a KMS framebuffer created WITHOUT an explicit modifier
/// (`nvkms-kapi.c:2257-2316`), so it is a per-import constant, never per frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockLinearSurface {
    /// `log2GobsPerBlock.{x, y, z}`.
    pub log2_gobs_per_block: (u32, u32, u32),
    /// `pitchInBlocks` — GOBs per row of the object's widest frame.
    pub pitch_in_blocks: u32,
    /// `genericMemory` — the object is in the generic kind.
    pub generic_memory: bool,
}

/// ★ The 28-byte private params (`memFd` left zero for `kf-linux-raw` to fill).
#[must_use]
pub fn priv_import_params(bl: &BlockLinearSurface) -> [u8; PRIV_IMPORT_SIZE] {
    let mut b = [0u8; PRIV_IMPORT_SIZE];
    let mut put = |o: usize, v: u32| b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    put(4, LAYOUT_BLOCK_LINEAR);
    put(8, bl.log2_gobs_per_block.0);
    put(12, bl.log2_gobs_per_block.1);
    put(16, bl.log2_gobs_per_block.2);
    put(20, bl.pitch_in_blocks);
    b[24] = u8::from(bl.generic_memory);
    b
}

// ── the probed interval (2026-10-03, drmnv.py) ────────────────────────────────────────────────────────────────

/// The committed probe output (`tools/drivermatrix/drmnv.py`, run 2026-10-03 over the 29 tags of
/// `tools/drivermatrix/tags.txt`).
pub static MEASURED_TSV: &str = include_str!("../../../traces/driver_matrix/drmnv.tsv");

/// ★ What every measured tag must answer for the rung to be offered there: exactly the values the
/// encoders above were written against (`ogkm-580: 580.159.04`). Each item is one `printf` of the
/// probe.
pub static TRANSCRIBED: &[(&str, &str)] = &[
    ("sizeof:drm_nvidia_get_dev_info_params", "36"),
    ("offsetof:drm_nvidia_get_dev_info_params.gpu_id", "0"),
    ("offsetof:drm_nvidia_get_dev_info_params.mig_device", "4"),
    ("offsetof:drm_nvidia_get_dev_info_params.primary_index", "8"),
    (
        "offsetof:drm_nvidia_get_dev_info_params.supports_alloc",
        "12",
    ),
    (
        "offsetof:drm_nvidia_get_dev_info_params.generic_page_kind",
        "16",
    ),
    (
        "offsetof:drm_nvidia_get_dev_info_params.page_kind_generation",
        "20",
    ),
    (
        "offsetof:drm_nvidia_get_dev_info_params.sector_layout",
        "24",
    ),
    (
        "offsetof:drm_nvidia_get_dev_info_params.supports_sync_fd",
        "28",
    ),
    (
        "offsetof:drm_nvidia_get_dev_info_params.supports_semsurf",
        "32",
    ),
    ("sizeof:drm_nvidia_gem_import_nvkms_memory_params", "32"),
    (
        "offsetof:drm_nvidia_gem_import_nvkms_memory_params.mem_size",
        "0",
    ),
    (
        "sizeof:drm_nvidia_gem_import_nvkms_memory_params.mem_size",
        "8",
    ),
    (
        "offsetof:drm_nvidia_gem_import_nvkms_memory_params.nvkms_params_ptr",
        "8",
    ),
    (
        "sizeof:drm_nvidia_gem_import_nvkms_memory_params.nvkms_params_ptr",
        "8",
    ),
    (
        "offsetof:drm_nvidia_gem_import_nvkms_memory_params.nvkms_params_size",
        "16",
    ),
    (
        "sizeof:drm_nvidia_gem_import_nvkms_memory_params.nvkms_params_size",
        "8",
    ),
    (
        "offsetof:drm_nvidia_gem_import_nvkms_memory_params.handle",
        "24",
    ),
    ("sizeof:NvKmsKapiPrivImportMemoryParams", "28"),
    ("offsetof:NvKmsKapiPrivImportMemoryParams.memFd", "0"),
    (
        "offsetof:NvKmsKapiPrivImportMemoryParams.surfaceParams.layout",
        "4",
    ),
    (
        "sizeof:NvKmsKapiPrivImportMemoryParams.surfaceParams.layout",
        "4",
    ),
    (
        "offsetof:NvKmsKapiPrivImportMemoryParams.surfaceParams.blockLinear.log2GobsPerBlock.x",
        "8",
    ),
    (
        "offsetof:NvKmsKapiPrivImportMemoryParams.surfaceParams.blockLinear.log2GobsPerBlock.y",
        "12",
    ),
    (
        "offsetof:NvKmsKapiPrivImportMemoryParams.surfaceParams.blockLinear.log2GobsPerBlock.z",
        "16",
    ),
    (
        "offsetof:NvKmsKapiPrivImportMemoryParams.surfaceParams.blockLinear.pitchInBlocks",
        "20",
    ),
    (
        "offsetof:NvKmsKapiPrivImportMemoryParams.surfaceParams.blockLinear.genericMemory",
        "24",
    ),
    (
        "sizeof:NvKmsKapiPrivImportMemoryParams.surfaceParams.blockLinear.genericMemory",
        "1",
    ),
    ("ioctl:DRM_IOCTL_NVIDIA_GET_DEV_INFO", "0xc0246443"),
    (
        "ioctl:DRM_IOCTL_NVIDIA_GEM_IMPORT_NVKMS_MEMORY",
        "0xc0206441",
    ),
    ("enum:NvKmsSurfaceMemoryLayoutBlockLinear", "0"),
    ("enum:NvKmsSurfaceMemoryLayoutPitch", "1"),
];

/// Why the rung is refused at a host driver tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrmNvRefusal {
    /// The tag is not in the probe output at all.
    NotMeasured(String),
    /// The probe could not measure the tag (its reason).
    Unmeasurable(String, String),
    /// A value differs from the transcription: (tag, item, probed value, transcribed).
    Differs(String, String, String, String),
    /// A transcribed item is absent from the tag's rows.
    Missing(String, String),
}

impl core::fmt::Display for DrmNvRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DrmNvRefusal::NotMeasured(t) => write!(
                f,
                "GPU-copy rung: the nvidia-drm ABI is not measured at host driver {t} \
                 (tools/drivermatrix/drmnv.py)"
            ),
            DrmNvRefusal::Unmeasurable(t, why) => write!(
                f,
                "GPU-copy rung: the nvidia-drm ABI could not be measured at host driver {t} ({why})"
            ),
            DrmNvRefusal::Differs(t, item, got, want) => write!(
                f,
                "GPU-copy rung: the nvidia-drm ABI differs at host driver {t}: {item} is {got}, \
                 the encoder was written for {want}"
            ),
            DrmNvRefusal::Missing(t, item) => write!(
                f,
                "GPU-copy rung: the nvidia-drm ABI at host driver {t} lacks {item}"
            ),
        }
    }
}

/// ★ Is the rung's private ABI equal to its transcription at host driver `v`, as `drmnv.py`
/// compiled that tag's own headers (run 2026-10-03) — exact tag,
/// as every other host-ABI fact in this crate?
///
/// # Errors
/// [`DrmNvRefusal`], by name.
pub fn abi_measured(v: HostDriverVersion) -> Result<(), DrmNvRefusal> {
    abi_measured_in(MEASURED_TSV, &v.to_string())
}

/// [`abi_measured`] over any probe output (tests feed it mutated rows).
///
/// # Errors
/// [`DrmNvRefusal`].
pub fn abi_measured_in(tsv: &str, tag: &str) -> Result<(), DrmNvRefusal> {
    let rows: Vec<(&str, &str)> = tsv
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let mut it = l.splitn(3, '\t');
            match (it.next(), it.next(), it.next()) {
                (Some(t), Some(k), Some(v)) if t == tag => Some((k, v)),
                _ => None,
            }
        })
        .collect();
    let Some(&(_, status)) = rows.iter().find(|(k, _)| *k == "status") else {
        return Err(DrmNvRefusal::NotMeasured(tag.to_string()));
    };
    if status != "MEASURED" {
        return Err(DrmNvRefusal::Unmeasurable(
            tag.to_string(),
            status.to_string(),
        ));
    }
    for (item, want) in TRANSCRIBED {
        match rows.iter().find(|(k, _)| k == item) {
            None => return Err(DrmNvRefusal::Missing(tag.to_string(), (*item).to_string())),
            Some((_, got)) if got != want => {
                return Err(DrmNvRefusal::Differs(
                    tag.to_string(),
                    (*item).to_string(),
                    (*got).to_string(),
                    (*want).to_string(),
                ));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★ The ioctl numbers, pinned without a GPU, and derived from the struct sizes the gate
    /// measures (a size change moves the number, which the probe then also measures).
    #[test]
    fn the_ioctl_numbers_are_the_drm_iowr_of_the_measured_sizes() {
        assert_eq!(IOCTL_GET_DEV_INFO, 0xC024_6443);
        assert_eq!(IOCTL_GEM_IMPORT_NVKMS_MEMORY, 0xC020_6441);
        let want = |item: &str| {
            TRANSCRIBED
                .iter()
                .find(|(k, _)| *k == item)
                .map(|(_, v)| u32::from_str_radix(v.trim_start_matches("0x"), 16).unwrap())
                .unwrap()
        };
        assert_eq!(
            IOCTL_GET_DEV_INFO,
            want("ioctl:DRM_IOCTL_NVIDIA_GET_DEV_INFO")
        );
        assert_eq!(
            IOCTL_GEM_IMPORT_NVKMS_MEMORY,
            want("ioctl:DRM_IOCTL_NVIDIA_GEM_IMPORT_NVKMS_MEMORY")
        );
    }

    /// ★ The design's vector: Turing … GB20x, 32 bpp, uncompressed, 16-GOB blocks.
    #[test]
    fn the_block_linear_modifier_matches_drm_fourcc() {
        assert_eq!(block_linear_2d(0, 1, 2, 0x06, 4), 0x0300_0000_0060_6014);
        let info = DevInfo {
            supports_alloc: true,
            generic_page_kind: 0x06,
            page_kind_generation: 2,
            sector_layout: 1,
            ..DevInfo::default()
        };
        assert_eq!(info.block_linear_modifier(4), Some(0x0300_0000_0060_6014));
        assert_eq!(
            DevInfo::default().block_linear_modifier(4),
            None,
            "without supports_alloc the fields are not values"
        );
        // the sector layout's upper bits (GB20x 8/16 bpp) land at 26:27, the compression at 23:25
        assert_eq!(block_linear_2d(0, 2, 0, 0, 0) & (3 << 26), 1 << 26);
        assert_eq!(block_linear_2d(7, 0, 0, 0, 0) & (7 << 23), 7 << 23);
        assert_eq!(block_linear_2d(0, 0, 0, 0, 0xff), 0x0300_0000_0000_001f);
    }

    /// Every field lands at its transcribed offset (a distinct value planted per field).
    #[test]
    fn the_encoders_place_every_field_at_its_offset() {
        let mut b = [0u8; GET_DEV_INFO_SIZE];
        for i in 0..9 {
            b[i * 4..i * 4 + 4].copy_from_slice(&(0x10 + i as u32).to_le_bytes());
        }
        let d = DevInfo::decode(&b);
        assert_eq!(
            (d.gpu_id, d.mig_device, d.primary_index, d.generic_page_kind),
            (0x10, 0x11, 0x12, 0x14)
        );
        assert_eq!((d.page_kind_generation, d.sector_layout), (0x15, 0x16));
        assert!(d.supports_alloc && d.supports_sync_fd && d.supports_semsurf);
        let a = gem_import_arg(10 << 20);
        assert_eq!(&a[0..8], &(10u64 << 20).to_le_bytes());
        assert_eq!(&a[8..16], &[0; 8], "the pointer is never written here");
        assert_eq!(&a[16..24], &28u64.to_le_bytes());
        let mut r = a;
        r[24..28].copy_from_slice(&7u32.to_le_bytes());
        assert_eq!(gem_import_handle(&r), 7);
        let p = priv_import_params(&BlockLinearSurface {
            log2_gobs_per_block: (0, 4, 0),
            pitch_in_blocks: 120,
            generic_memory: true,
        });
        assert_eq!(&p[0..4], &[0; 4], "memFd is filled by kf-linux-raw");
        assert_eq!(&p[4..8], &LAYOUT_BLOCK_LINEAR.to_le_bytes());
        assert_eq!(&p[12..16], &4u32.to_le_bytes());
        assert_eq!(&p[20..24], &120u32.to_le_bytes());
        assert_eq!(p[24], 1);
        assert_eq!(&p[25..], &[0; 3]);
    }

    /// ★ The gate over the committed measurement: offered at the tags the rung was read at, refused
    /// by name at a tag that is not measured; and a mutated row (a field moved) is refused — the
    /// known-positive that the comparison can fire.
    #[test]
    fn the_rung_is_offered_only_at_tags_measured_equal_to_the_transcription() {
        // `[measured 2026-10-03, drmnv.py over tags.txt]` equal at 575.51.02 … 615.71.09 (the
        // header renamed to `nv_drm_common_ioctl.h` at 590.48.01, the layout unchanged)
        for t in [
            "575.51.02",
            "580.159.04",
            "590.48.01",
            "610.43.02",
            "615.71.09",
        ] {
            assert_eq!(abi_measured_in(MEASURED_TSV, t), Ok(()), "{t}");
        }
        // ⊘ before 575 `drm_nvidia_get_dev_info_params` has no `mig_device`: every later field
        // sits 4 bytes lower, which the DRM core would have zero-filled into a silent misread
        let e = abi_measured_in(MEASURED_TSV, "570.148.08").unwrap_err();
        assert!(matches!(e, DrmNvRefusal::Unmeasurable(..)), "{e:?}");
        assert!(e.to_string().contains("mig_device"), "{e}");
        assert!(matches!(
            abi_measured_in(MEASURED_TSV, "580.159.03"),
            Err(DrmNvRefusal::NotMeasured(_))
        ));
        let moved = MEASURED_TSV.replace(
            "580.159.04\toffsetof:drm_nvidia_get_dev_info_params.supports_alloc\t12",
            "580.159.04\toffsetof:drm_nvidia_get_dev_info_params.supports_alloc\t16",
        );
        assert_ne!(moved, MEASURED_TSV, "the row to mutate exists");
        let e = abi_measured_in(&moved, "580.159.04").unwrap_err();
        assert!(matches!(e, DrmNvRefusal::Differs(..)), "{e:?}");
        assert!(e.to_string().contains("supports_alloc is 16"), "{e}");
        let unmeasured = "x\tstatus\tUNMEASURED compile: no header\n";
        assert!(matches!(
            abi_measured_in(unmeasured, "x"),
            Err(DrmNvRefusal::Unmeasurable(..))
        ));
        let dropped: String = MEASURED_TSV
            .lines()
            .filter(|l| !l.contains("580.159.04\tenum:NvKmsSurfaceMemoryLayoutPitch"))
            .map(|l| format!("{l}\n"))
            .collect();
        assert!(matches!(
            abi_measured_in(&dropped, "580.159.04"),
            Err(DrmNvRefusal::Missing(..))
        ));
    }

    /// Every probe item the encoders depend on is transcribed, and the probe printed each one at
    /// 580.159.04 (a transcription the probe never prints could never be checked).
    #[test]
    fn every_transcribed_item_is_one_the_probe_prints() {
        for (item, _) in TRANSCRIBED {
            assert!(
                MEASURED_TSV.contains(&format!("580.159.04\t{item}\t")),
                "{item} is not in the measurement"
            );
        }
    }
}
