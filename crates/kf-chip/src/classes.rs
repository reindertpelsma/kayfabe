//! ★ Engine class SETS per family — GENERATED from ogkm, never hand-picked.
//!
//! `tools/derive_classes.sh` compiles ogkm's `g_gpu_class_list.c` (the per-chip
//! `hal<CHIP>ClassDescriptorList[]` tables) and unions every chip's entries into its family, so a new
//! die of a known family is covered without an edit. Copied from the old `kayfabe-doorbell/
//! classgen.rs` (w824), where it replaced two hand-picked tables that had denied every non-Ampere
//! guest its channel/compute/copy objects (fable A1) and every A100/RTX 50xx guest one layer down
//! (fable HIGH 1).
//!
//! ⊘ The sets are WIDER than a family's own classes: every family lists its predecessors' channel
//! and usermode classes (Hopper lists `AMPERE_CHANNEL_GPFIFO_A`), because the host RM accepts them.

use crate::Family;
use kf_arch::ids::{ClassId, EngineKind};
use kf_arch::ObjectKind;
use kf_abi::generated::classes as nv;

/// The kinds of engine object a guest allocates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// A GPFIFO channel class.
    ChannelGpfifo,
    /// The compute class (GR).
    Compute,
    /// The copy-engine class.
    DmaCopy,
    /// The usermode class — its 64 KiB CPU mapping IS the doorbell page.
    Usermode,
    /// The 3D class (same GR engine as compute).
    ThreeD,
    /// ★ v3-gfx: the 2D class (`FERMI_TWOD_A`) — a GR-engine object graphics UMDs put on their
    /// 3D channel (blits, clears).
    TwoD,
    /// ★ v3-gfx: the inline-to-memory class (`KEPLER_INLINE_TO_MEMORY_B`, Blackwell's own) — a
    /// GR-engine object for pushbuffer-literal uploads.
    InlineToMemory,
    /// ★ A video ENCODER class (`NV*B7_VIDEO_ENCODER`, NVENC) — its own engine and runlist.
    VideoEncoder,
    /// ★ A video DECODER class (`NV*B0_VIDEO_DECODER`, NVDEC) — its own engine and runlist.
    VideoDecoder,
    /// ★ v3-gfxset: the optical-flow class (`NV*FA_VIDEO_OFA`, OFA) — its own engine and runlist;
    /// the engine behind `VK_NV_optical_flow` and the NVOFA SDK.
    OpticalFlow,
}

impl Kind {
    /// Every kind.
    pub const ALL: [Kind; 10] = [
        Kind::ChannelGpfifo,
        Kind::Compute,
        Kind::DmaCopy,
        Kind::Usermode,
        Kind::ThreeD,
        Kind::TwoD,
        Kind::InlineToMemory,
        Kind::VideoEncoder,
        Kind::VideoDecoder,
        Kind::OpticalFlow,
    ];
}

/// One family's engine classes, per kind. ⊘ Slices, not scalars: the whole point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassSet {
    /// The family.
    pub family: Family,
    /// The chips whose class lists were unioned.
    pub chips: &'static [&'static str],
    /// Channel classes.
    pub channel_gpfifo: &'static [u32],
    /// Compute classes.
    pub compute: &'static [u32],
    /// Copy-engine classes.
    pub dma_copy: &'static [u32],
    /// Usermode classes.
    pub usermode: &'static [u32],
    /// 3D classes.
    pub threed: &'static [u32],
    /// 2D classes (GR engine).
    pub twod: &'static [u32],
    /// Inline-to-memory classes (GR engine).
    pub inline_to_memory: &'static [u32],
    /// Video encoder (NVENC) classes — ⊘ EMPTY on a family whose chips list none (Hopper: GH100
    /// has NVDEC and NVJPG but no NVENC, `g_gpu_class_list.c`), derived, never assumed.
    pub video_encoder: &'static [u32],
    /// Video decoder (NVDEC) classes.
    pub video_decoder: &'static [u32],
    /// ★ Optical-flow (OFA) classes — ⊘ EMPTY on Turing (no `NV*FA_VIDEO_OFA` in any TU10x list),
    /// derived, never assumed.
    pub optical_flow: &'static [u32],
}

impl ClassSet {
    /// The classes of kind `k`.
    #[must_use]
    pub fn of_kind(&self, k: Kind) -> &'static [u32] {
        match k {
            Kind::ChannelGpfifo => self.channel_gpfifo,
            Kind::Compute => self.compute,
            Kind::DmaCopy => self.dma_copy,
            Kind::Usermode => self.usermode,
            Kind::ThreeD => self.threed,
            Kind::TwoD => self.twod,
            Kind::InlineToMemory => self.inline_to_memory,
            Kind::VideoEncoder => self.video_encoder,
            Kind::VideoDecoder => self.video_decoder,
            Kind::OpticalFlow => self.optical_flow,
        }
    }

    /// Which kind `class` is on this family, if listed.
    #[must_use]
    pub fn kind_of(&self, class: u32) -> Option<Kind> {
        Kind::ALL.iter().copied().find(|k| self.of_kind(*k).contains(&class))
    }

    /// ★ Classify a guest-allocated class for the object graph — the ONE classifier, for every
    /// family: the family-invariant resource classes, then this family's generated engine sets.
    /// ⊘ An unlisted id is `ObjectKind::Unknown`, never a guess.
    #[must_use]
    pub fn classify(&self, class: ClassId) -> ObjectKind {
        match class.0 {
            nv::NV01_ROOT | nv::NV01_ROOT_CLIENT => return ObjectKind::Client,
            nv::NV01_DEVICE_0 => return ObjectKind::Device,
            nv::NV20_SUBDEVICE_0 => return ObjectKind::Subdevice,
            nv::NV01_EVENT_KERNEL_CALLBACK_EX => return ObjectKind::Event,
            nv::FERMI_VASPACE_A => return ObjectKind::VaSpace,
            nv::KEPLER_CHANNEL_GROUP_A => return ObjectKind::Tsg,
            nv::FERMI_CONTEXT_SHARE_A => return ObjectKind::CtxShare,
            nv::NV2081_BINAPI => return ObjectKind::Other,
            _ => {}
        }
        match self.kind_of(class.0) {
            // A channel class is GR until its engine object refines it (the params carry the
            // engine type, which a class id cannot).
            Some(Kind::ChannelGpfifo) => ObjectKind::Channel { engine: EngineKind::GrCompute },
            Some(Kind::Compute) => ObjectKind::EngineObject { engine: EngineKind::GrCompute },
            Some(Kind::ThreeD | Kind::TwoD | Kind::InlineToMemory) => {
                ObjectKind::EngineObject { engine: EngineKind::GrGraphics }
            }
            Some(Kind::DmaCopy) => ObjectKind::EngineObject { engine: EngineKind::Ce },
            Some(Kind::VideoEncoder) => ObjectKind::EngineObject { engine: EngineKind::NvEnc },
            Some(Kind::VideoDecoder) => ObjectKind::EngineObject { engine: EngineKind::NvDec },
            // ★ v3-gfxset: OFA is an engine kayfabe routes (a passthrough twin) and never interprets
            // — `EngineKind::Other`'s own definition; no census or RC route distinguishes it.
            Some(Kind::OpticalFlow) => ObjectKind::EngineObject { engine: EngineKind::Other },
            Some(Kind::Usermode) | None => ObjectKind::Unknown,
        }
    }
}

/// ★ GENERATED by `tools/derive_classes.sh --rust` from ogkm 610.43.02 — regenerate, never edit.
/// §50 level 2: every id is what the C compiler resolved from `g_gpu_class_list.c` +
/// `src/common/sdk/nvidia/inc/class/*.h`; every set is the union over the named chips.
pub const FAMILIES: [ClassSet; 5] = [
    ClassSet {
        family: Family::Turing,
        chips: &["TU102", "TU104", "TU106", "TU116", "TU117"],
        channel_gpfifo: &[0xC36F /* VOLTA_CHANNEL_GPFIFO_A */, 0xC46F /* TURING_CHANNEL_GPFIFO_A */],
        compute: &[0xC5C0 /* TURING_COMPUTE_A */],
        dma_copy: &[0xC5B5 /* TURING_DMA_COPY_A */],
        usermode: &[0xC361 /* VOLTA_USERMODE_A */, 0xC461 /* TURING_USERMODE_A */],
        threed: &[0xC597 /* TURING_A */],
        twod: &[0x902D /* FERMI_TWOD_A */],
        inline_to_memory: &[0xA140 /* KEPLER_INLINE_TO_MEMORY_B */],
        video_encoder: &[0xB4B7 /* NVB4B7_VIDEO_ENCODER */, 0xC4B7 /* NVC4B7_VIDEO_ENCODER */],
        video_decoder: &[0xC4B0 /* NVC4B0_VIDEO_DECODER */],
        optical_flow: &[],
    },
    ClassSet {
        family: Family::Ampere,
        chips: &["GA100", "GA102", "GA103", "GA104", "GA106", "GA107"],
        channel_gpfifo: &[0xC36F /* VOLTA_CHANNEL_GPFIFO_A */, 0xC46F /* TURING_CHANNEL_GPFIFO_A */, 0xC56F /* AMPERE_CHANNEL_GPFIFO_A */],
        compute: &[0xC6C0 /* AMPERE_COMPUTE_A */, 0xC7C0 /* AMPERE_COMPUTE_B */],
        dma_copy: &[0xC6B5 /* AMPERE_DMA_COPY_A */, 0xC7B5 /* AMPERE_DMA_COPY_B */],
        usermode: &[0xC361 /* VOLTA_USERMODE_A */, 0xC461 /* TURING_USERMODE_A */, 0xC561 /* AMPERE_USERMODE_A */],
        threed: &[0xC697 /* AMPERE_A */, 0xC797 /* AMPERE_B */],
        twod: &[0x902D /* FERMI_TWOD_A */],
        inline_to_memory: &[0xA140 /* KEPLER_INLINE_TO_MEMORY_B */],
        video_encoder: &[0xC7B7 /* NVC7B7_VIDEO_ENCODER */],
        video_decoder: &[0xC6B0 /* NVC6B0_VIDEO_DECODER */, 0xC7B0 /* NVC7B0_VIDEO_DECODER */],
        optical_flow: &[0xC6FA /* NVC6FA_VIDEO_OFA */, 0xC7FA /* NVC7FA_VIDEO_OFA */],
    },
    ClassSet {
        family: Family::Ada,
        chips: &["AD102", "AD103", "AD104", "AD106", "AD107"],
        channel_gpfifo: &[0xC36F /* VOLTA_CHANNEL_GPFIFO_A */, 0xC46F /* TURING_CHANNEL_GPFIFO_A */, 0xC56F /* AMPERE_CHANNEL_GPFIFO_A */],
        compute: &[0xC9C0 /* ADA_COMPUTE_A */],
        dma_copy: &[0xC7B5 /* AMPERE_DMA_COPY_B */],
        usermode: &[0xC361 /* VOLTA_USERMODE_A */, 0xC461 /* TURING_USERMODE_A */, 0xC561 /* AMPERE_USERMODE_A */],
        threed: &[0xC997 /* ADA_A */],
        twod: &[0x902D /* FERMI_TWOD_A */],
        inline_to_memory: &[0xA140 /* KEPLER_INLINE_TO_MEMORY_B */],
        video_encoder: &[0xC9B7 /* NVC9B7_VIDEO_ENCODER */],
        video_decoder: &[0xC9B0 /* NVC9B0_VIDEO_DECODER */],
        optical_flow: &[0xC9FA /* NVC9FA_VIDEO_OFA */],
    },
    ClassSet {
        family: Family::Hopper,
        chips: &["GH100"],
        channel_gpfifo: &[0xC36F /* VOLTA_CHANNEL_GPFIFO_A */, 0xC46F /* TURING_CHANNEL_GPFIFO_A */, 0xC56F /* AMPERE_CHANNEL_GPFIFO_A */, 0xC86F /* HOPPER_CHANNEL_GPFIFO_A */],
        compute: &[0xCBC0 /* HOPPER_COMPUTE_A */],
        dma_copy: &[0xC8B5 /* HOPPER_DMA_COPY_A */],
        usermode: &[0xC361 /* VOLTA_USERMODE_A */, 0xC461 /* TURING_USERMODE_A */, 0xC561 /* AMPERE_USERMODE_A */, 0xC661 /* HOPPER_USERMODE_A */],
        threed: &[0xCB97 /* HOPPER_A */],
        twod: &[0x902D /* FERMI_TWOD_A */],
        inline_to_memory: &[0xA140 /* KEPLER_INLINE_TO_MEMORY_B */],
        video_encoder: &[],
        video_decoder: &[0xB8B0 /* NVB8B0_VIDEO_DECODER */],
        optical_flow: &[0xB8FA /* NVB8FA_VIDEO_OFA */],
    },
    ClassSet {
        family: Family::Blackwell,
        chips: &["GB100", "GB102", "GB10B", "GB110", "GB112", "GB202", "GB203", "GB205", "GB206", "GB207", "GB20B", "GB20C", "GR100", "GR102"],
        channel_gpfifo: &[0xC36F /* VOLTA_CHANNEL_GPFIFO_A */, 0xC46F /* TURING_CHANNEL_GPFIFO_A */, 0xC56F /* AMPERE_CHANNEL_GPFIFO_A */, 0xC86F /* HOPPER_CHANNEL_GPFIFO_A */, 0xC96F /* BLACKWELL_CHANNEL_GPFIFO_A */, 0xCA6F /* BLACKWELL_CHANNEL_GPFIFO_B */],
        compute: &[0xCDC0 /* BLACKWELL_COMPUTE_A */, 0xCEC0 /* BLACKWELL_COMPUTE_B */],
        dma_copy: &[0xC9B5 /* BLACKWELL_DMA_COPY_A */, 0xCAB5 /* BLACKWELL_DMA_COPY_B */],
        usermode: &[0xC361 /* VOLTA_USERMODE_A */, 0xC461 /* TURING_USERMODE_A */, 0xC561 /* AMPERE_USERMODE_A */, 0xC661 /* HOPPER_USERMODE_A */, 0xC761 /* BLACKWELL_USERMODE_A */],
        threed: &[0xCD97 /* BLACKWELL_A */, 0xCE97 /* BLACKWELL_B */],
        twod: &[0x902D /* FERMI_TWOD_A */],
        inline_to_memory: &[0xCD40 /* BLACKWELL_INLINE_TO_MEMORY_A */],
        video_encoder: &[0xCEB7 /* NVCEB7_VIDEO_ENCODER */, 0xCFB7 /* NVCFB7_VIDEO_ENCODER */, 0xD1B7 /* NVD1B7_VIDEO_ENCODER */],
        video_decoder: &[0xCDB0 /* NVCDB0_VIDEO_DECODER */, 0xCEB0 /* NVCEB0_VIDEO_DECODER */, 0xCFB0 /* NVCFB0_VIDEO_DECODER */, 0xD1B0 /* NVD1B0_VIDEO_DECODER */, 0xD2B0 /* NVD2B0_VIDEO_DECODER */],
        optical_flow: &[0xCDFA /* NVCDFA_VIDEO_OFA */, 0xCEFA /* NVCEFA_VIDEO_OFA */, 0xCFFA /* NVCFFA_VIDEO_OFA */, 0xD1FA /* NVD1FA_VIDEO_OFA */, 0xD2FA /* NVD2FA_VIDEO_OFA */],
    },
];

/// The class set of `f`.
///
/// # Panics
/// Never: [`FAMILIES`] is exhaustive over [`Family`] (a test pins it).
#[must_use]
pub fn classes_for(f: Family) -> &'static ClassSet {
    FAMILIES.iter().find(|c| c.family == f).expect("FAMILIES is exhaustive")
}

#[cfg(test)]
mod gfx_kinds {
    use super::*;

    /// ★ v3-gfx: every family's generated set carries the 2D and inline-to-memory GR classes the
    /// graphics UMDs allocate (`[measured vgfx 2026-09-26]` the host's own Vulkan/EGL run allocs
    /// `FERMI_TWOD_A` ×7 and `KEPLER_INLINE_TO_MEMORY_B` ×7), and an id listed by several
    /// families always has ONE kind there — `engine_class_kind` searches across families.
    #[test]
    fn graphics_gr_classes_are_generated_per_family_and_one_id_has_one_kind() {
        for f in &FAMILIES {
            assert!(f.twod.contains(&0x902D), "{:?} has no FERMI_TWOD_A", f.family);
            assert!(!f.inline_to_memory.is_empty(), "{:?} has no inline-to-memory class", f.family);
        }
        let mut seen: std::collections::BTreeMap<u32, Kind> = std::collections::BTreeMap::new();
        for f in &FAMILIES {
            for k in Kind::ALL {
                for &id in f.of_kind(k) {
                    assert_eq!(*seen.entry(id).or_insert(k), k, "{id:#x} is {k:?} in {:?} and something else elsewhere", f.family);
                }
            }
        }
        assert_eq!(classes_for(Family::Blackwell).kind_of(0xCD40), Some(Kind::InlineToMemory));
        // ★ v3-gfxset: OFA is generated where the chips list it, and only there
        assert_eq!(classes_for(Family::Ampere).kind_of(0xC7FA), Some(Kind::OpticalFlow));
        assert_eq!(classes_for(Family::Ada).kind_of(0xC9FA), Some(Kind::OpticalFlow));
        assert!(classes_for(Family::Turing).optical_flow.is_empty(), "no TU10x chip lists an OFA class");
        assert_eq!(classes_for(Family::Ampere).kind_of(0x902D), Some(Kind::TwoD));
    }
}
