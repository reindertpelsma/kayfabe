//! Layouts that are **hand-transcribed**, and the reason each one has to be.
//!
//! # Why this module exists at all
//!
//! The vendored open-kernel-modules tree is a **single snapshot** — 610.43.02
//! (`research_clones/ogkm/version.mk:1`). `mode2_abi_agnostic_layer.md` §2.4
//! flagged this as a finding and it bites here immediately: a struct that
//! *changed* at some driver boundary has only its newest form in the tree, so
//! the older form cannot be generated from anything we have.
//!
//! Hand-transcription is exactly the practice codegen exists to retire (L11), so
//! every entry here is a **defect with a fix**, not a design. The fix is
//! mechanical and stated per entry: vendor the corresponding ogkm tag and delete
//! the entry. Until then each one carries its independent citations and is
//! pinned by the same oracle tests as the generated code, so it is at least
//! transcription *under supervision*.
//!
//! # What is NOT here
//!
//! The pre-550.54.04 `NVOS47_PARAMETERS` (40 bytes, no `size` field). It is a
//! real layout, but no driver version this project supports is that old — 575
//! and 580 are the HW-validated pair (`multi_driver_validated`) — so writing it
//! down would be breadth with no consumer, and an untested transcription is
//! worse than an absent one. [`crate::versions`] therefore refuses any driver
//! older than 550.54.04 outright rather than quietly decoding it wrong.

use crate::wire::{AbiError, Field, StructLayout, u32_at, u64_at};

/// `NVOS46_PARAMETERS` as it was **before driver 580.65.06** — 56 bytes.
///
/// # Provenance (transcribed, three citations)
///
/// 1. **gVisor nvproxy**, `gvisor/pkg/abi/nvgpu/frontend.go:625-639`
///    (`NVOS46_PARAMETERS`: `Client, Device, Dma, Memory, Offset, Length, Flags,
///    Pad0[4], DmaOffset, Status, Pad1[4]`), which nvproxy replaces with
///    `NVOS46_PARAMETERS_V580` starting at driver 580.65.06
///    (`gvisor/pkg/sentry/devices/nvproxy/version.go:1057-1059`).
/// 2. **The C artifact's runtime profile**,
///    `nvidia-gpu-passthrough/src/common/nvkvm_abi.h:66,76` —
///    `.nvos46_size = 56, .nvos46_status_off = 48` for the 535 and 570/575
///    profiles, versus `:86` `.nvos46_size = 64, .nvos46_status_off = 56` for
///    580.
/// 3. **The C artifact's parity test**,
///    `nvidia-gpu-passthrough/tests/abi_parity/abi_parity_test.go:68-71`, which
///    asserts 56 with a comment naming `DmaOffset` at +40 and `Status` at +48.
///
/// # The delta to the generated form
///
/// [`crate::generated::nvos::Nvos46Parameters`] (610.43.02) adds `flags2` at +36
/// and `kindOverride` at +40, pushing `dmaOffset` to +48 and `status` to +56 and
/// `sizeof` to 64. Everything at or before `flags` is identical, which is why
/// [`crate::view::MapMemoryDma`] can be one version-independent shape.
///
/// # To delete this
///
/// Vendor ogkm tag `575.51.02` (or any tag in `[550.54.04, 580.65.06)`) and add
/// it to the generator's input set. Nothing else about this file is load-bearing.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Nvos46ParametersPre580 {
    /// `NvHandle hClient` @ +0.
    pub h_client: u32,
    /// `NvHandle hDevice` @ +4.
    pub h_device: u32,
    /// `NvHandle hDma` @ +8.
    pub h_dma: u32,
    /// `NvHandle hMemory` @ +12.
    pub h_memory: u32,
    /// `NvU64 offset` @ +16.
    pub offset: u64,
    /// `NvU64 length` @ +24.
    pub length: u64,
    /// `NvV32 flags` @ +32.
    pub flags: u32,
    // +36: 4 bytes of padding before the 8-aligned `dmaOffset`.
    /// `NvU64 dmaOffset` @ +40 — `[OUT]` the GPU VA the mapping landed at.
    pub dma_offset: u64,
    /// `NvV32 status` @ +48 — `[OUT]`.
    pub status: u32,
    // +52: 4 bytes of tail padding to the struct's 8-byte alignment.
}

impl Nvos46ParametersPre580 {
    /// The C typedef name (unchanged across the version boundary — NVIDIA grew
    /// the struct in place, which is precisely why a size-blind decoder is
    /// dangerous here).
    pub const C_NAME: &'static str = "NVOS46_PARAMETERS";
    /// `sizeof`, per all three citations above.
    pub const SIZE: usize = 56;
    /// `alignof`.
    pub const ALIGN: usize = 8;

    /// The layout, in the same shape the generated structs use so the oracle
    /// tests can walk generated and transcribed layouts identically.
    pub const LAYOUT: StructLayout = StructLayout {
        c_name: "NVOS46_PARAMETERS",
        size: 56,
        align: 8,
        fields: &[
            Field {
                c_name: "hClient",
                rust_name: "h_client",
                offset: 0,
                width: 4,
            },
            Field {
                c_name: "hDevice",
                rust_name: "h_device",
                offset: 4,
                width: 4,
            },
            Field {
                c_name: "hDma",
                rust_name: "h_dma",
                offset: 8,
                width: 4,
            },
            Field {
                c_name: "hMemory",
                rust_name: "h_memory",
                offset: 12,
                width: 4,
            },
            Field {
                c_name: "offset",
                rust_name: "offset",
                offset: 16,
                width: 8,
            },
            Field {
                c_name: "length",
                rust_name: "length",
                offset: 24,
                width: 8,
            },
            Field {
                c_name: "flags",
                rust_name: "flags",
                offset: 32,
                width: 4,
            },
            Field {
                c_name: "dmaOffset",
                rust_name: "dma_offset",
                offset: 40,
                width: 8,
            },
            Field {
                c_name: "status",
                rust_name: "status",
                offset: 48,
                width: 4,
            },
        ],
    };

    /// rustc's own offsets, so the transcription is checked against the compiler
    /// exactly like the generated structs are.
    pub const RUSTC_OFFSETS: &'static [(&'static str, usize)] = &[
        (
            "h_client",
            core::mem::offset_of!(Nvos46ParametersPre580, h_client),
        ),
        (
            "h_device",
            core::mem::offset_of!(Nvos46ParametersPre580, h_device),
        ),
        (
            "h_dma",
            core::mem::offset_of!(Nvos46ParametersPre580, h_dma),
        ),
        (
            "h_memory",
            core::mem::offset_of!(Nvos46ParametersPre580, h_memory),
        ),
        (
            "offset",
            core::mem::offset_of!(Nvos46ParametersPre580, offset),
        ),
        (
            "length",
            core::mem::offset_of!(Nvos46ParametersPre580, length),
        ),
        (
            "flags",
            core::mem::offset_of!(Nvos46ParametersPre580, flags),
        ),
        (
            "dma_offset",
            core::mem::offset_of!(Nvos46ParametersPre580, dma_offset),
        ),
        (
            "status",
            core::mem::offset_of!(Nvos46ParametersPre580, status),
        ),
    ];

    /// Decode from a little-endian byte image.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`] if `bytes.len() < Self::SIZE`.
    pub fn decode(bytes: &[u8]) -> Result<Self, AbiError> {
        if bytes.len() < Self::SIZE {
            return Err(AbiError::Truncated {
                c_name: Self::C_NAME,
                need: Self::SIZE,
                got: bytes.len(),
            });
        }
        Ok(Self {
            h_client: u32_at(bytes, 0)?,
            h_device: u32_at(bytes, 4)?,
            h_dma: u32_at(bytes, 8)?,
            h_memory: u32_at(bytes, 12)?,
            offset: u64_at(bytes, 16)?,
            length: u64_at(bytes, 24)?,
            flags: u32_at(bytes, 32)?,
            dma_offset: u64_at(bytes, 40)?,
            status: u32_at(bytes, 48)?,
        })
    }

    /// Write back only the declared fields, leaving padding and any tail as
    /// found — see the generated `encode_into` for why that rule exists.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`] if `bytes.len() < Self::SIZE`.
    pub fn encode_into(&self, bytes: &mut [u8]) -> Result<(), AbiError> {
        let len = bytes.len();
        if len < Self::SIZE {
            return Err(AbiError::Truncated {
                c_name: Self::C_NAME,
                need: Self::SIZE,
                got: len,
            });
        }
        let mut put = |off: usize, src: &[u8]| -> Result<(), AbiError> {
            bytes
                .get_mut(off..off + src.len())
                .ok_or(AbiError::Truncated {
                    c_name: Self::C_NAME,
                    need: Self::SIZE,
                    got: len,
                })?
                .copy_from_slice(src);
            Ok(())
        };
        put(0, &self.h_client.to_le_bytes())?;
        put(4, &self.h_device.to_le_bytes())?;
        put(8, &self.h_dma.to_le_bytes())?;
        put(12, &self.h_memory.to_le_bytes())?;
        put(16, &self.offset.to_le_bytes())?;
        put(24, &self.length.to_le_bytes())?;
        put(32, &self.flags.to_le_bytes())?;
        put(40, &self.dma_offset.to_le_bytes())?;
        put(48, &self.status.to_le_bytes())?;
        Ok(())
    }
}

// The transcription vs rustc, at COMPILE time — the same gate the generated
// structs get. A transcription that nothing checks is a rumour.
const _: () = {
    assert!(core::mem::size_of::<Nvos46ParametersPre580>() == Nvos46ParametersPre580::SIZE);
    assert!(core::mem::align_of::<Nvos46ParametersPre580>() == Nvos46ParametersPre580::ALIGN);
    assert!(core::mem::offset_of!(Nvos46ParametersPre580, h_client) == 0);
    assert!(core::mem::offset_of!(Nvos46ParametersPre580, h_device) == 4);
    assert!(core::mem::offset_of!(Nvos46ParametersPre580, h_dma) == 8);
    assert!(core::mem::offset_of!(Nvos46ParametersPre580, h_memory) == 12);
    assert!(core::mem::offset_of!(Nvos46ParametersPre580, offset) == 16);
    assert!(core::mem::offset_of!(Nvos46ParametersPre580, length) == 24);
    assert!(core::mem::offset_of!(Nvos46ParametersPre580, flags) == 32);
    assert!(core::mem::offset_of!(Nvos46ParametersPre580, dma_offset) == 40);
    assert!(core::mem::offset_of!(Nvos46ParametersPre580, status) == 48);
};
