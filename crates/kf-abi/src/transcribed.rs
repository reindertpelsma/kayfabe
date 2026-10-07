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
//! # ★ There are TWO reasons a layout lands here, and only one of them is a version
//!
//! The paragraph above describes the first: *the tree we have does not contain the
//! shape we need*. [`Nvos46ParametersPre580`] is that, and vendoring a tag deletes it.
//!
//! [`Nv2080CtrlGpuPromoteCtxParamsHeader`] is the second: *the tree contains the shape
//! and the generator cannot express it*. `NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS` ends in
//! `promoteEntry[16]` — a fixed array of a **nested struct** — which the generator
//! refuses in two independent places on purpose (`gen/src/ctype.rs`'s deliberately
//! closed scalar table; `gen/src/parse.rs`'s `ParseError::NestedAggregate`). Its fix is
//! not a tag, it is teaching the generator struct-typed fields, which would also unblock
//! `NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES` — see
//! [`crate::versions::ControlParams::PageDirNotModelled`].
//!
//! ★★ **The split is chosen so that nothing NUMERIC is hand-computed.** The entry
//! record is all scalars, so it goes through the generator with its full pinning stack
//! ([`crate::generated::ctrl::Nv2080CtrlGpuPromoteCtxBufferEntry`]) — and that is where
//! the risk actually is: its `u32`/`u16`/`u8`/`u8` tail is the exact field the C
//! artifact read four bytes wide (defect D2). The 48-byte *header* is nine plain
//! scalars, transcribed here with the same `LAYOUT` + `RUSTC_OFFSETS` +
//! `const { assert!(offset_of!(..)) }` triple the generated structs get. The array
//! itself is then stride arithmetic over two **generated** constants
//! (`NV2080_CTRL_GPU_PROMOTE_CONTEXT_MAX_ENTRIES`, `…BufferEntry::SIZE`), so the
//! 560-byte total is a product of numbers a compiler checked, not a number anyone typed.
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

/// The 48-byte **scalar prefix** of `NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS` — everything
/// before `promoteEntry[16]`.
///
/// # Why this is a transcription and not a generated struct
///
/// See the module doc: the params struct's last member is a fixed array of a nested
/// struct, which the generator refuses by design. This is the half that is nine plain
/// scalars; [`crate::generated::ctrl::Nv2080CtrlGpuPromoteCtxBufferEntry`] is the half
/// the generator emits unchanged.
///
/// # ★ It is a PREFIX, not a struct — the same contract as `CHANNEL_ALLOC_PREFIX`
///
/// `size_of::<Self>()` is 48 and `sizeof(NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS)` is 560.
/// The 48 is exactly `offsetof(…, promoteEntry)`, which is what makes the stride
/// arithmetic in [`crate::versions::DriverAbiTable::decode_promote_ctx`] correct.
/// [`Self::PARAMS_SIZE`] states the whole struct's size, composed from two generated
/// numbers rather than written down.
///
/// # Provenance (three citations, and a fourth for the total)
///
/// 1. **ogkm 580.159.04** —
///    `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080gpu.h:988-1000`.
/// 2. **ogkm 610.43.02** — `ogkm-610: .../ctrl2080gpu.h:959-971`. Field for field,
///    alignment for alignment, identical to 580. ★★ That identity is a load-bearing
///    NEGATIVE result: `NV_CHANNEL_ALLOC_PARAMS` diverges at +32 *inside* the supported
///    range, so the house rule is to assume nothing until both tags are read. Both were
///    read here and they agree — **a version seam for this struct would be inventing one
///    that does not exist**, and [`crate::versions::ControlParams::PromoteCtx`]
///    deliberately carries no `MapDmaWire`-style fork.
/// 3. **The C artifact's own snoop offsets**,
///    `nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:2441-2445` — its handler comment
///    independently records `hChanClient@+12`, `entryCount@+40`, `promoteEntry[]@+48`,
///    and its code reads exactly those.
/// 4. **A captured host RPC** —
///    `nvidia-gpu-passthrough/docs/research/captures/ga106_initctrl_580.log:2422` records
///    `cmd=0x2080012b … psize=560`, which is the total [`Self::PARAMS_SIZE`] computes.
///
/// # To delete this
///
/// Teach `kayfabe-abi-gen` struct-typed fields (a scalar-table extension plus an emitter
/// arm for nested `decode`/`encode_into`), add a `StructReq` for
/// `NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS`, and delete this type. That change touches the
/// artefact which *guards* the L11 truncation bug class, so it deliberately does not
/// ride along with a decoder.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Nv2080CtrlGpuPromoteCtxParamsHeader {
    /// `NvU32 engineType` @ +0.
    pub engine_type: u32,
    /// `NvHandle hClient` @ +4 — documented as *"Client Handle for hVirtMemory"*, i.e.
    /// the legacy path's client, NOT the namespace of [`Self::h_object`].
    pub h_client: u32,
    /// `NvU32 ChID` @ +8 — *"Hw Channel — Actually hw index for channel (deprecated)"*.
    pub ch_id: u32,
    /// `NvHandle hChanClient` @ +12 — *"The client handle for hObject"*.
    ///
    /// ★★ This is the namespace [`Self::h_object`] is a handle **in**, and reading it
    /// here is correct: RM sets it from `RES_GET_CLIENT_HANDLE(pChannelDescendant)`
    /// while issuing the control under `RES_GET_CLIENT_HANDLE(pSubdevice)`
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/gr/kernel_graphics_object.c:130-135`), and
    /// the two are not required to be equal. The C artifact's defect is not that it
    /// reads this field — it is that it reads *only* this field and never the envelope's
    /// `hClient`, so it cannot notice a disagreement, let alone refuse one.
    pub h_chan_client: u32,
    /// `NvHandle hObject` @ +16 — *"either a single channel or a channel group"*.
    pub h_object: u32,
    /// `NvHandle hVirtMemory` @ +20 — the **legacy** (pre-`promoteEntry`) path.
    pub h_virt_memory: u32,
    /// `NvU64 virtAddress` @ +24 — the legacy path's VA.
    pub virt_address: u64,
    /// `NvU64 size` @ +32 — the legacy path's length.
    pub size: u64,
    /// `NvU32 entryCount` @ +40 — *guest-declared*, and bounded by
    /// [`crate::generated::ctrl::NV2080_CTRL_GPU_PROMOTE_CONTEXT_MAX_ENTRIES`].
    pub entry_count: u32,
    // +44: 4 bytes of tail padding — `promoteEntry` is 8-aligned.
}

impl Nv2080CtrlGpuPromoteCtxParamsHeader {
    /// The C typedef this is the prefix of.
    pub const C_NAME: &'static str = "NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS";
    /// Bytes of prefix — and, identically, `offsetof(…, promoteEntry)`.
    pub const SIZE: usize = 48;
    /// `alignof`.
    pub const ALIGN: usize = 8;

    /// `sizeof(NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS)` = **560**.
    ///
    /// ★ Computed from [`Self::SIZE`] and two generated numbers, never typed: the
    /// entry's `SIZE` is the generator's, cross-checked against rustc's `size_of` at
    /// compile time, and `MAX_ENTRIES` is scanned out of the header. The one number the
    /// C artifact hand-wrote in this control is the one it got wrong.
    pub const PARAMS_SIZE: usize = Self::SIZE
        + crate::generated::ctrl::NV2080_CTRL_GPU_PROMOTE_CONTEXT_MAX_ENTRIES
            * crate::generated::ctrl::Nv2080CtrlGpuPromoteCtxBufferEntry::SIZE;

    /// The layout, in the shape the oracle tests walk.
    pub const LAYOUT: StructLayout = StructLayout {
        c_name: "NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS",
        size: 48,
        align: 8,
        fields: &[
            Field {
                c_name: "engineType",
                rust_name: "engine_type",
                offset: 0,
                width: 4,
            },
            Field {
                c_name: "hClient",
                rust_name: "h_client",
                offset: 4,
                width: 4,
            },
            Field {
                c_name: "ChID",
                rust_name: "ch_id",
                offset: 8,
                width: 4,
            },
            Field {
                c_name: "hChanClient",
                rust_name: "h_chan_client",
                offset: 12,
                width: 4,
            },
            Field {
                c_name: "hObject",
                rust_name: "h_object",
                offset: 16,
                width: 4,
            },
            Field {
                c_name: "hVirtMemory",
                rust_name: "h_virt_memory",
                offset: 20,
                width: 4,
            },
            Field {
                c_name: "virtAddress",
                rust_name: "virt_address",
                offset: 24,
                width: 8,
            },
            Field {
                c_name: "size",
                rust_name: "size",
                offset: 32,
                width: 8,
            },
            Field {
                c_name: "entryCount",
                rust_name: "entry_count",
                offset: 40,
                width: 4,
            },
        ],
    };

    /// rustc's own offsets, so the transcription is checked against the compiler
    /// exactly like the generated structs are.
    pub const RUSTC_OFFSETS: &'static [(&'static str, usize)] = &[
        (
            "engine_type",
            core::mem::offset_of!(Nv2080CtrlGpuPromoteCtxParamsHeader, engine_type),
        ),
        (
            "h_client",
            core::mem::offset_of!(Nv2080CtrlGpuPromoteCtxParamsHeader, h_client),
        ),
        (
            "ch_id",
            core::mem::offset_of!(Nv2080CtrlGpuPromoteCtxParamsHeader, ch_id),
        ),
        (
            "h_chan_client",
            core::mem::offset_of!(Nv2080CtrlGpuPromoteCtxParamsHeader, h_chan_client),
        ),
        (
            "h_object",
            core::mem::offset_of!(Nv2080CtrlGpuPromoteCtxParamsHeader, h_object),
        ),
        (
            "h_virt_memory",
            core::mem::offset_of!(Nv2080CtrlGpuPromoteCtxParamsHeader, h_virt_memory),
        ),
        (
            "virt_address",
            core::mem::offset_of!(Nv2080CtrlGpuPromoteCtxParamsHeader, virt_address),
        ),
        (
            "size",
            core::mem::offset_of!(Nv2080CtrlGpuPromoteCtxParamsHeader, size),
        ),
        (
            "entry_count",
            core::mem::offset_of!(Nv2080CtrlGpuPromoteCtxParamsHeader, entry_count),
        ),
    ];

    /// Decode the prefix from a little-endian byte image.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`] if `bytes.len() < Self::SIZE`. Never a zero-extended
    /// partial decode.
    pub fn decode(bytes: &[u8]) -> Result<Self, AbiError> {
        if bytes.len() < Self::SIZE {
            return Err(AbiError::Truncated {
                c_name: Self::C_NAME,
                need: Self::SIZE,
                got: bytes.len(),
            });
        }
        Ok(Self {
            engine_type: u32_at(bytes, 0)?,
            h_client: u32_at(bytes, 4)?,
            ch_id: u32_at(bytes, 8)?,
            h_chan_client: u32_at(bytes, 12)?,
            h_object: u32_at(bytes, 16)?,
            h_virt_memory: u32_at(bytes, 20)?,
            virt_address: u64_at(bytes, 24)?,
            size: u64_at(bytes, 32)?,
            entry_count: u32_at(bytes, 40)?,
        })
    }
}

// The transcription vs rustc, at COMPILE time — the same gate the generated structs
// get. A transcription that nothing checks is a rumour.
const _: () = {
    assert!(
        core::mem::size_of::<Nv2080CtrlGpuPromoteCtxParamsHeader>()
            == Nv2080CtrlGpuPromoteCtxParamsHeader::SIZE
    );
    assert!(
        core::mem::align_of::<Nv2080CtrlGpuPromoteCtxParamsHeader>()
            == Nv2080CtrlGpuPromoteCtxParamsHeader::ALIGN
    );
    assert!(core::mem::offset_of!(Nv2080CtrlGpuPromoteCtxParamsHeader, engine_type) == 0);
    assert!(core::mem::offset_of!(Nv2080CtrlGpuPromoteCtxParamsHeader, h_client) == 4);
    assert!(core::mem::offset_of!(Nv2080CtrlGpuPromoteCtxParamsHeader, ch_id) == 8);
    assert!(core::mem::offset_of!(Nv2080CtrlGpuPromoteCtxParamsHeader, h_chan_client) == 12);
    assert!(core::mem::offset_of!(Nv2080CtrlGpuPromoteCtxParamsHeader, h_object) == 16);
    assert!(core::mem::offset_of!(Nv2080CtrlGpuPromoteCtxParamsHeader, h_virt_memory) == 20);
    assert!(core::mem::offset_of!(Nv2080CtrlGpuPromoteCtxParamsHeader, virt_address) == 24);
    assert!(core::mem::offset_of!(Nv2080CtrlGpuPromoteCtxParamsHeader, size) == 32);
    assert!(core::mem::offset_of!(Nv2080CtrlGpuPromoteCtxParamsHeader, entry_count) == 40);
    // ★ The one arithmetic fact this file states, asserted against the captured host
    // RPC's own `psize` (see the type's provenance note 4).
    assert!(Nv2080CtrlGpuPromoteCtxParamsHeader::PARAMS_SIZE == 560);
};
