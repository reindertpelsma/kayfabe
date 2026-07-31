//! `kayfabe-abi-gen` — generate `kayfabe-abi`'s wire layouts from a vendored
//! open-gpu-kernel-modules checkout.
//!
//! ```text
//! cargo run --manifest-path crates/kayfabe-abi/gen/Cargo.toml -- \
//!     <ogkm-root> crates/kayfabe-abi/src/generated
//! ```
//!
//! # The slice, and why it is this one
//!
//! Deliberately small. Every struct here is either (a) a 1:1 producer of a
//! `kayfabe_core::rmgraph::RmEvent` variant, (b) the source of a field on
//! `AllocFacts`, or (c) the GSP boot path's RPC envelope. Nothing is here
//! because it was easy; nothing that the core cannot yet consume is here at all.
//!
//! Breadth is cheap and worthless: a wrong entry in a broad table is invisible
//! until a guest trips it (that is the entire L11 incident list). So the bar for
//! adding a struct is that something above it consumes the struct **today**.

mod ctype;
mod emit;
mod parse;

use ctype::{Layout, lay_out};
use parse::{
    ParseError, find_aggregates, parse_fields, scan_defines, scan_drf_ranges, scan_macro_list,
    strip_comments,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// A struct we want mirrored.
struct StructReq {
    /// Header path relative to the ogkm root.
    header: &'static str,
    /// C typedef name.
    name: &'static str,
    /// If the last member is a flexible array, its element type's alignment —
    /// which the header does not state locally. Declared here WITH a citation,
    /// and cross-checked: the generator refuses if including it would change
    /// `sizeof` relative to the field-only computation, because the emitted Rust
    /// mirror omits the FAM and could then not be `#[repr(C)]`-equal.
    fam_align: Option<usize>,
}

/// A `#define` we want as a typed Rust constant.
struct ConstReq {
    header: &'static str,
    c_name: &'static str,
    rust_name: &'static str,
    rust_ty: &'static str,
    doc: &'static str,
}

/// A DRF **bit range** (`#define NAME hi:lo`) we want as a typed Rust constant.
///
/// Emitted as a [`kayfabe_abi::wire::Drf`], which keeps `hi` and `lo` together so
/// they cannot be transposed at the use site.
struct DrfReq {
    header: &'static str,
    c_name: &'static str,
    rust_name: &'static str,
    doc: &'static str,
}

/// An X-macro enumeration we want as a constant table.
struct MacroListReq {
    header: &'static str,
    prefix: &'static str,
    arity: usize,
    /// Rust name prefix for the emitted constants.
    rust_prefix: &'static str,
    /// Only these names are emitted — the full list is 230 entries and we do not
    /// need 230. Emitting all of them would be breadth without a consumer.
    keep: &'static [&'static str],
    doc: &'static str,
}

/// A small aggregate NVIDIA embeds **by value** in a struct we mirror, declared
/// here as if it were a scalar.
///
/// This is the one place the generator accepts a hand-supplied size, so it is
/// also verified: `verify_against` names the aggregate's own `typedef` in the
/// same header, whose layout the generator computes and compares. A drift
/// between the declaration and the header is a hard error, not a warning.
struct AggregateScalar {
    scalar: ctype::Scalar,
    /// The `typedef union`/`typedef struct` in the header to check against.
    verify_against: &'static str,
    /// Header containing that definition.
    verify_header: &'static str,
}

/// One generated `.rs` file.
struct ModuleReq {
    file: &'static str,
    /// One-line title for the `mod.rs` entry.
    title: &'static str,
    doc: &'static str,
    structs: &'static [StructReq],
    consts: &'static [ConstReq],
    /// DRF bit ranges (`hi:lo`), emitted as `Drf` constants.
    drfs: &'static [DrfReq],
    macro_lists: &'static [MacroListReq],
    /// Extra by-value aggregate types this module's structs embed.
    aggregate_scalars: &'static [AggregateScalar],
    /// The same table flattened for lookup (must list the same `scalar` values).
    aggregate_lookup: &'static [ctype::Scalar],
}

const NVOS_H: &str = "src/common/sdk/nvidia/inc/nvos.h";
const NV_ESCAPE_H: &str = "src/nvidia/arch/nvalloc/unix/include/nv_escape.h";
const CL0000_H: &str = "src/common/sdk/nvidia/inc/class/cl0000.h";
const CL0080_H: &str = "src/common/sdk/nvidia/inc/class/cl0080.h";
const CTRL0080DMA_H: &str = "src/common/sdk/nvidia/inc/ctrl/ctrl0080/ctrl0080dma.h";
const CTRL2080GPU_H: &str = "src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080gpu.h";
const CL9067_H: &str = "src/common/sdk/nvidia/inc/class/cl9067.h";
const CL90F1_H: &str = "src/common/sdk/nvidia/inc/class/cl90f1.h";
const CLA06C_H: &str = "src/common/sdk/nvidia/inc/class/cla06c.h";
const CLC56F_H: &str = "src/common/sdk/nvidia/inc/class/clc56f.h";
const CLC7B5_H: &str = "src/common/sdk/nvidia/inc/class/clc7b5.h";
const CLC7C0_H: &str = "src/common/sdk/nvidia/inc/class/clc7c0.h";
const RPC_HDR_H: &str = "src/nvidia/generated/g_rpc-message-header.h";
const RPC_ENUMS_H: &str = "src/nvidia/inc/kernel/vgpu/rpc_global_enums.h";
/// The RPC **payload** structs. Deliberately a single entry in the slice
/// (`rpc_rc_triggered_v17_02`): the module doc below states that the 213 payload structs
/// are out of scope until something consumes one, and task #111's simulated-fault
/// emission is the first consumer there has been.
const RPC_STRUCTS_H: &str = "src/nvidia/generated/g_rpc-structures.h";
/// The `ROBUST_CHANNEL_*` exception codes — the numbers that appear in a host kernel log
/// as `Xid <n>`.
const NVERROR_H: &str = "src/common/sdk/nvidia/inc/nverror.h";
/// `NV2080_ENGINE_TYPE_*` — the engine vocabulary the RC event routes on.
const CL2080_NOTIFICATION_H: &str = "src/common/sdk/nvidia/inc/class/cl2080_notification.h";

// ── VBIOS / FWSEC ────────────────────────────────────────────────────────────
//
// ★ Two of these are `.c` files, not headers, and that is deliberate: NVIDIA
// declares the entire BIT-table and FWSEC-descriptor vocabulary *inside* the
// implementation files rather than in a header. Scanning the `.c` is therefore
// scanning the **only** authoritative statement of those constants. The scanner
// does not care — it reads `#define` lines out of a text file.
const PCI_EXP_TABLE_H: &str = "src/nvidia/inc/kernel/platform/pci_exp_table.h";
const FWSEC_C: &str = "src/nvidia/src/kernel/gpu/gsp/kernel_gsp_fwsec.c";
const VBIOS_TU102_C: &str = "src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_vbios_tu102.c";
/// ★ The third `.c`, and the one that reads the FWSEC **DMEM payload** rather than
/// the ROM around it. `s_vbiosPatchInterfaceData` walks a
/// `FALCON_APPLICATION_INTERFACE_HEADER_V1` at the descriptor's `InterfaceOffset`
/// and refuses the whole adapter init if it cannot find a `_DMEMMAPPER` entry — so
/// these three typedefs are as load-bearing as the BIT table, and they too are
/// declared inside the implementation file with no header to read them from.
const FRTS_TU102_C: &str = "src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_frts_tu102.c";

/// Headers whose `#define`s feed array-length resolution, globally.
const DEFINE_SOURCES: &[&str] = &["src/common/sdk/nvidia/inc/nvlimits.h", NVOS_H];

/// Terse [`ConstReq`] for the VBIOS module, where the Rust name is always the C
/// name (these constants have no NVIDIA-namespace prefix to strip and renaming
/// them would only break the grep from generated code back to the driver).
const fn vbios_const(
    header: &'static str,
    c_name: &'static str,
    rust_ty: &'static str,
    doc: &'static str,
) -> ConstReq {
    ConstReq {
        header,
        c_name,
        rust_name: c_name,
        rust_ty,
        doc,
    }
}

/// Terse [`DrfReq`], same naming rule as [`vbios_const`].
const fn vbios_drf(header: &'static str, c_name: &'static str, doc: &'static str) -> DrfReq {
    DrfReq {
        header,
        c_name,
        rust_name: c_name,
        doc,
    }
}

const MODULES: &[ModuleReq] = &[
    ModuleReq {
        file: "nvos.rs",
        title: "The frontend RM ioctl parameter structs (`NVOS*`) — the `RmEvent` seam.",
        doc: "\
The frontend RM ioctl parameter structs — the seven verbs the core's `RmEvent`
enum is made of.

| C typedef | escape | `RmEvent` |
|---|---|---|
| `NVOS00_PARAMETERS` | `NV_ESC_RM_FREE` | `Free` |
| `NVOS21_PARAMETERS` | `NV_ESC_RM_ALLOC` (v1) | `Alloc` |
| `NVOS64_PARAMETERS` | `NV_ESC_RM_ALLOC` (v2) | `Alloc` |
| `NVOS54_PARAMETERS` | `NV_ESC_RM_CONTROL` | carries `SetPageDir` |
| `NVOS55_PARAMETERS` | `NV_ESC_RM_DUP_OBJECT` | `Dup` |
| `NVOS46_PARAMETERS` | `NV_ESC_RM_MAP_MEMORY_DMA` | `MapMemoryDma` |
| `NVOS47_PARAMETERS` | `NV_ESC_RM_UNMAP_MEMORY_DMA` | `Unmap` |

★ `NVOS46_PARAMETERS` as generated here is the **580.65.06-and-later** layout
(64 bytes, with `flags2` + `kindOverride`). The pre-580.65.06 layout is 56 bytes
and is NOT in this file, because the vendored ogkm tree is a single snapshot
(610.43.02) — see `crate::transcribed`.",
        structs: &[
            StructReq {
                header: NVOS_H,
                name: "NVOS00_PARAMETERS",
                fam_align: None,
            },
            StructReq {
                header: NVOS_H,
                name: "NVOS21_PARAMETERS",
                fam_align: None,
            },
            StructReq {
                header: NVOS_H,
                name: "NVOS46_PARAMETERS",
                fam_align: None,
            },
            StructReq {
                header: NVOS_H,
                name: "NVOS47_PARAMETERS",
                fam_align: None,
            },
            StructReq {
                header: NVOS_H,
                name: "NVOS54_PARAMETERS",
                fam_align: None,
            },
            StructReq {
                header: NVOS_H,
                name: "NVOS55_PARAMETERS",
                fam_align: None,
            },
            StructReq {
                header: NVOS_H,
                name: "NVOS64_PARAMETERS",
                fam_align: None,
            },
        ],
        consts: &[
            ConstReq {
                header: NV_ESCAPE_H,
                c_name: "NV_ESC_RM_FREE",
                rust_name: "NV_ESC_RM_FREE",
                rust_ty: "u32",
                doc: "`NV_ESC_RM_FREE` — ioctl NR carrying [`Nvos00Parameters`].",
            },
            ConstReq {
                header: NV_ESCAPE_H,
                c_name: "NV_ESC_RM_CONTROL",
                rust_name: "NV_ESC_RM_CONTROL",
                rust_ty: "u32",
                doc: "`NV_ESC_RM_CONTROL` — ioctl NR carrying [`Nvos54Parameters`].",
            },
            ConstReq {
                header: NV_ESCAPE_H,
                c_name: "NV_ESC_RM_ALLOC",
                rust_name: "NV_ESC_RM_ALLOC",
                rust_ty: "u32",
                doc: "`NV_ESC_RM_ALLOC` — ioctl NR carrying [`Nvos21Parameters`] or\n[`Nvos64Parameters`], discriminated by the ioctl's own size field.",
            },
            ConstReq {
                header: NV_ESCAPE_H,
                c_name: "NV_ESC_RM_DUP_OBJECT",
                rust_name: "NV_ESC_RM_DUP_OBJECT",
                rust_ty: "u32",
                doc: "`NV_ESC_RM_DUP_OBJECT` — ioctl NR carrying [`Nvos55Parameters`].",
            },
            ConstReq {
                header: NV_ESCAPE_H,
                c_name: "NV_ESC_RM_MAP_MEMORY_DMA",
                rust_name: "NV_ESC_RM_MAP_MEMORY_DMA",
                rust_ty: "u32",
                doc: "`NV_ESC_RM_MAP_MEMORY_DMA` — ioctl NR carrying [`Nvos46Parameters`].",
            },
            ConstReq {
                header: NV_ESCAPE_H,
                c_name: "NV_ESC_RM_UNMAP_MEMORY_DMA",
                rust_name: "NV_ESC_RM_UNMAP_MEMORY_DMA",
                rust_ty: "u32",
                doc: "`NV_ESC_RM_UNMAP_MEMORY_DMA` — ioctl NR carrying [`Nvos47Parameters`].",
            },
        ],
        drfs: &[],
        macro_lists: &[],
        aggregate_scalars: &[],
        aggregate_lookup: &[],
    },
    ModuleReq {
        file: "classes.rs",
        title: "Per-class alloc-param structs and their class IDs.",
        doc: "\
The alloc-param structs the core's `AllocFacts` actually reads, and the class IDs
that select them.

- `NV0000_ALLOC_PARAMETERS` (`NV01_ROOT`) carries `processID`, the decision-#14
  client-kind discriminator (`l1_concurrency.md` §12.27).
- `NV0080_ALLOC_PARAMETERS` (`NV01_DEVICE_0`) carries `deviceId`, the multi-GPU
  routing fact (`multi_gpu_and_mig.md`).
- `NV_CHANNEL_GROUP_ALLOCATION_PARAMETERS` (`KEPLER_CHANNEL_GROUP_A`) and
  `NV_CTXSHARE_ALLOCATION_PARAMETERS` (`FERMI_CONTEXT_SHARE_A`) each carry an
  `hVASpace`, the two indirect halves of a channel's VAS resolution
  (`kayfabe_core::project::resolve_channel_vas`: own handle -> CtxShare's ->
  parent TSG's).

★★ `NV_CHANNEL_ALLOC_PARAMS` (`AMPERE_CHANNEL_GPFIFO_A`) is DELIBERATELY NOT
generated here, and the reason is a measured version divergence rather than a
generator limitation. At 610.43.02 the struct carries `hHandleVASpace` at +32,
inserted directly after `hVASpace`; at 580.159.04 — the driver this project's
bench actually runs (`versions::BENCH_DRIVER`) — that field does not exist and
`hUserdMemory[]` starts at +32 instead (`ogkm-610: alloc_channel.h:296-347` vs
`ogkm-580: alloc_channel.h:296-342` — the typedef opens at `:296` in BOTH trees;
only 610's body is one member longer). A generated 610 mirror would therefore
mis-read EVERY field from +32 onward for the guest we run. The three fields
`AllocFacts` needs (`flags` @20, `hContextShare` @24, `hVASpace` @28) are
byte-identical in both trees, so `versions::CHANNEL_ALLOC_PREFIX` decodes exactly
that prefix — the same contract, and for the same kind of reason, as
`CLIENT_ALLOC_PREFIX`.

A class whose alloc params carry nothing `AllocFacts` models
(`FERMI_VASPACE_A`, the engine objects) gets a class ID here and no struct: the
ID is the consumer, and mirroring params nothing reads would be breadth.

Every other class's alloc params is deferred: the class table is its own
milestone and a half-populated one is worse than none, because a missing entry
reads as `None` = \"class not in this version\" rather than \"nobody has done it\".",
        structs: &[
            StructReq {
                header: CL0000_H,
                name: "NV0000_ALLOC_PARAMETERS",
                fam_align: None,
            },
            StructReq {
                header: CL0080_H,
                name: "NV0080_ALLOC_PARAMETERS",
                fam_align: None,
            },
            StructReq {
                header: NVOS_H,
                name: "NV_CHANNEL_GROUP_ALLOCATION_PARAMETERS",
                fam_align: None,
            },
            StructReq {
                header: NVOS_H,
                name: "NV_CTXSHARE_ALLOCATION_PARAMETERS",
                fam_align: None,
            },
        ],
        consts: &[
            ConstReq {
                header: CL0000_H,
                c_name: "NV01_ROOT",
                rust_name: "NV01_ROOT",
                rust_ty: "u32",
                doc: "`NV01_ROOT` — the client-root class; its alloc params are\n[`Nv0000AllocParameters`].",
            },
            ConstReq {
                header: NVOS_H,
                c_name: "NV01_ROOT_CLIENT",
                rust_name: "NV01_ROOT_CLIENT",
                rust_ty: "u32",
                doc: "`NV01_ROOT_CLIENT` — the modern client-root class. Same alloc params as\n[`NV01_ROOT`]; RM treats the two as one resource kind.",
            },
            ConstReq {
                header: CL0080_H,
                c_name: "NV01_DEVICE_0",
                rust_name: "NV01_DEVICE_0",
                rust_ty: "u32",
                doc: "`NV01_DEVICE_0` — the Device class; its alloc params are\n[`Nv0080AllocParameters`] and carry the GPU routing target.",
            },
            ConstReq {
                header: CL90F1_H,
                c_name: "FERMI_VASPACE_A",
                rust_name: "FERMI_VASPACE_A",
                rust_ty: "u32",
                doc: "`FERMI_VASPACE_A` — the VASpace class. Its alloc params\n(`NV_VASPACE_ALLOCATION_PARAMETERS`: `index`, `flags`, `vaSize`, `vaBase`,\n`pasid`) carry NOTHING `AllocFacts` models, so no struct is mirrored for it —\nthe VAS's data-plane identity arrives later, on\n`NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY`.",
            },
            ConstReq {
                header: CLA06C_H,
                c_name: "KEPLER_CHANNEL_GROUP_A",
                rust_name: "KEPLER_CHANNEL_GROUP_A",
                rust_ty: "u32",
                doc: "`KEPLER_CHANNEL_GROUP_A` — the TSG class; its alloc params are\n[`NvChannelGroupAllocationParameters`] and declare the VASpace every channel in\nthe group inherits.",
            },
            ConstReq {
                header: CL9067_H,
                c_name: "FERMI_CONTEXT_SHARE_A",
                rust_name: "FERMI_CONTEXT_SHARE_A",
                rust_ty: "u32",
                doc: "`FERMI_CONTEXT_SHARE_A` — the CtxShare (subcontext) class; its alloc\nparams are [`NvCtxshareAllocationParameters`] and declare a VASpace a channel\ncan reach indirectly.",
            },
            ConstReq {
                header: CLC56F_H,
                c_name: "AMPERE_CHANNEL_GPFIFO_A",
                rust_name: "AMPERE_CHANNEL_GPFIFO_A",
                rust_ty: "u32",
                doc: "`AMPERE_CHANNEL_GPFIFO_A` — the GPFIFO channel class on GA10x.\n\n★ There is exactly ONE channel class per architecture: a GR channel and a CE\nchannel are the SAME `hClass` and differ only by `NV_CHANNEL_ALLOC_PARAMS.\nengineType`, which `kayfabe_core::rmgraph::RmEvent::Alloc` has nowhere to put.\nThe engine therefore reaches the core only through the engine-object refinement\n(`kayfabe_core::project`), never through the class ID.",
            },
            ConstReq {
                header: CLC7C0_H,
                c_name: "AMPERE_COMPUTE_B",
                rust_name: "AMPERE_COMPUTE_B",
                rust_ty: "u32",
                doc: "`AMPERE_COMPUTE_B` — the compute engine object a CUDA process allocates\non its GR channel. Declares no `AllocFacts`; its whole protocol content is the\nedge (channel -> engine object) that refines the channel's `EngineKind`.",
            },
            ConstReq {
                header: CLC7B5_H,
                c_name: "AMPERE_DMA_COPY_B",
                rust_name: "AMPERE_DMA_COPY_B",
                rust_ty: "u32",
                doc: "`AMPERE_DMA_COPY_B` — the copy-engine object on a CE channel. Same shape\nas [`AMPERE_COMPUTE_B`]: no declared facts, and the only thing that tells the\ncore this channel is a CE channel at all.",
            },
        ],
        drfs: &[],
        macro_lists: &[],
        aggregate_scalars: &[],
        aggregate_lookup: &[],
    },
    ModuleReq {
        file: "ctrl.rs",
        title: "RM control commands and their payload structs.",
        doc: "\
Two RM controls: `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` and the entry record of
`NV2080_CTRL_CMD_GPU_PROMOTE_CTX`.

This is where a VAS's page-directory base is declared, i.e. where the data-plane
identity (`Pdb`) is born — `RmEvent::SetPageDir`. It arrives inside an
`NVOS54_PARAMETERS` as the control payload.

★★ `NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS` itself is NOT generated, and the reason is a
generator limitation rather than a version fact: its last member is
`promoteEntry[16]`, a fixed array of a NESTED struct, which
`crate::gen`'s closed scalar table and `ParseError::NestedAggregate` both refuse by
design. The decomposition is `crate::transcribed::Nv2080CtrlGpuPromoteCtxParamsHeader`
(the 48-byte scalar prefix, hand-transcribed under the same `LAYOUT`/`RUSTC_OFFSETS`
pinning) plus stride arithmetic over the entry emitted here. The entry is where the
risk actually lives — its `u32`/`u16`/`u8`/`u8` tail is the field the C artifact read
32 bits wide — and it is fully machine-pinned.

★ `NV2080_CTRL_GPU_PROMOTE_CONTEXT_MAX_ENTRIES` is generated as a constant for a
specific reason: the C artifact's handler clamped the guest's `entryCount` to a
hand-written `64` (its own comment said `20`) where the header says `16`, and read
1536 bytes past the struct. The number the transcription got wrong is the one number
this port refuses to write by hand.

★ Version caveat, stated because it is real: the prefix through `hVASpace` is
confirmed by three independent oracles (ogkm 610.43.02; the C emulator's snoop
offsets at `src/qemu/nvkvm_gpu_emul.c:2528-2536`, which read `physAddress@+0`,
`numEntries@+8`, `flags@+12`, `hVASpace@+16`; and NVIDIA's own field order). The
tail (`chId`, `subDeviceId`, `pasid`) is confirmed only by ogkm 610.43.02 — no
other oracle models this struct at all. `pasid` in particular looks like a recent
addition, in the same family as the `NV_VASPACE_ALLOCATION_PARAMETERS` `+Pasid`
growth the C artifact records at 580. If a 575 guest sends a shorter payload,
`decode` refuses **loudly** rather than reading past it, which is the correct
failure.",
        structs: &[
            StructReq {
                header: CTRL0080DMA_H,
                name: "NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS",
                fam_align: None,
            },
            StructReq {
                header: CTRL2080GPU_H,
                name: "NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ENTRY",
                fam_align: None,
            },
        ],
        consts: &[
            ConstReq {
                header: CTRL0080DMA_H,
                c_name: "NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY",
                rust_name: "NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY",
                rust_ty: "u32",
                doc: "`NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` — the control command whose\npayload is [`Nv0080CtrlDmaSetPageDirectoryParams`].",
            },
            ConstReq {
                header: CTRL2080GPU_H,
                c_name: "NV2080_CTRL_CMD_GPU_PROMOTE_CTX",
                rust_name: "NV2080_CTRL_CMD_GPU_PROMOTE_CTX",
                rust_ty: "u32",
                doc: "`NV2080_CTRL_CMD_GPU_PROMOTE_CTX` — the control that declares where a\ngraphics/compute context's buffers live. `ROUTE_TO_PHYSICAL`, so CPU-RM compiles\nits implementation out entirely and it exists only in GSP firmware — i.e. in the\nthing this port fakes.",
            },
            ConstReq {
                header: CTRL2080GPU_H,
                c_name: "NV2080_CTRL_GPU_PROMOTE_CONTEXT_MAX_ENTRIES",
                rust_name: "NV2080_CTRL_GPU_PROMOTE_CONTEXT_MAX_ENTRIES",
                rust_ty: "usize",
                doc: "`NV2080_CTRL_GPU_PROMOTE_CONTEXT_MAX_ENTRIES` — the length of\n`promoteEntry[]`, **16**, identical at 580.159.04 and 610.43.02.\n\n★ Generated rather than written down: the C artifact hand-wrote `64` here (with a\ncomment claiming `20`) and read 1536 bytes past the 560-byte struct out of\nguest-writable memory.",
            },
        ],
        drfs: &[],
        macro_lists: &[],
        aggregate_scalars: &[],
        aggregate_lookup: &[],
    },
    ModuleReq {
        file: "rpc.rs",
        title: "The GSP-RPC envelope and its function/event IDs.",
        doc: "\
The GSP-RPC envelope and the function/event IDs the boot path needs.

`kayfabe-gsp`'s doc comment names this crate as its dependency for \"RPC
decode/encode against `kayfabe-abi`'s generated message layouts\"; this is the
envelope half. The per-function *payload* structs (`g_rpc-structures.h`, 213 of
them) are deliberately NOT here — the boot FSM has not been ported, so nothing
consumes them yet, and `mode2_abi_agnostic_layer.md` §5 residual 1 is explicit
that codegen gives shapes and never protocol.",
        structs: &[
            StructReq {
                header: RPC_HDR_H,
                name: "rpc_message_header_v03_00",
                // `rpc_generic_union` is a union over the RPC payload structs, whose
                // arms include `NV_DECLARE_ALIGNED(NvU64 …, 8)` members, so 8 is the
                // conservative bound. The generator asserts that using it does not
                // change `sizeof` versus the field-only computation (it does not:
                // the named fields already end at 32, a multiple of 8).
                fam_align: Some(8),
            },
            StructReq {
                header: RPC_STRUCTS_H,
                name: "rpc_rc_triggered_v17_02",
                // The flexible tail is `NvU8 rcJournalBuffer[]`, so alignment 1. We
                // emit an EMPTY journal (`rcJournalBufferSize = 0`) — see
                // `crate::rc` for why an invented journal record would be worse than
                // none — but the alignment still has to be declared for the mirror to
                // be `#[repr(C)]`-equal, and the generator refuses it if including it
                // would move `sizeof`.
                fam_align: Some(1),
            },
        ],
        consts: &[
            ConstReq {
                header: NVERROR_H,
                c_name: "ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT",
                rust_name: "ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT",
                rust_ty: "u32",
                doc: "`ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT` — the `exceptType` an MMU\nfault carries, and the number a host kernel log prints as **`Xid 31`**.\n\nIt is generated rather than written down for the ordinary reason, and for one extra:\nthis repository's own design docs quote `Xid 31` dozens of times as a number READ OUT\nOF A HOST LOG. Typing `31` into a Rust source would put a second, uncheckable copy of\nit beside those readings.",
            },
            ConstReq {
                header: CL2080_NOTIFICATION_H,
                c_name: "NV2080_ENGINE_TYPE_GRAPHICS",
                rust_name: "NV2080_ENGINE_TYPE_GRAPHICS",
                rust_ty: "u32",
                doc: "`NV2080_ENGINE_TYPE_GRAPHICS` — the `nv2080EngineType` of a GR\ncontext, compute or graphics.",
            },
            ConstReq {
                header: CL2080_NOTIFICATION_H,
                c_name: "NV2080_ENGINE_TYPE_COPY0",
                rust_name: "NV2080_ENGINE_TYPE_COPY0",
                rust_ty: "u32",
                doc: "`NV2080_ENGINE_TYPE_COPY0` — the base of the copy-engine range.\n\n★ The BASE, and the fault emitter deliberately does not add an instance to it: see\n`crate::rc::EngineRoute` for why a copy engine gets **no** RC event today rather than\na guessed instance.",
            },
        ],
        drfs: &[],
        macro_lists: &[
            MacroListReq {
                header: RPC_ENUMS_H,
                prefix: "X",
                arity: 3,
                rust_prefix: "NV_VGPU_MSG_FUNCTION_",
                keep: &[
                    "NOP",
                    "SET_GUEST_SYSTEM_INFO",
                    "ALLOC_ROOT",
                    "ALLOC_MEMORY",
                    "FREE",
                    "MAP_MEMORY_DMA",
                    "UNMAP_MEMORY_DMA",
                    "DUP_OBJECT",
                    "UNLOADING_GUEST_DRIVER",
                    "GSP_RM_ALLOC",
                    "GSP_RM_CONTROL",
                    // ★ Added 2026-07-31 for the production `GspAbi` assembly
                    // (`kayfabe_device::abi::FUNCTIONS`). These four were already
                    // named by the GSP boot FSM's `FunctionCodes`, but only in a
                    // test harness that transcribed the numbers by hand — which is
                    // a second copy of an NVIDIA constant outside the Axis-A
                    // quarantine. The list grows because the consumer grew, which
                    // is the legitimate direction; shortening it is the one that
                    // weakens a proof with no red test.
                    "GET_GSP_STATIC_INFO",
                    "CONTINUATION_RECORD",
                    "GSP_SET_SYSTEM_INFO",
                    "SET_REGISTRY",
                    // ★ Added 2026-07-31 (task #127) for the same reason and by the
                    // same rule: `RmRpcSetGuestSystemInfo` tail-calls
                    // `NV_RM_RPC_SET_GUEST_SYSTEM_INFO_EXT` and returns ITS status
                    // (`ogkm-580: rpc.c:8825-8832`), so a port that answers fn 1 and
                    // not fn 64 fails `RmInitAdapter` one line further on. The
                    // consumer is `kayfabe_device::guestsysinfo`.
                    "SET_GUEST_SYSTEM_INFO_EXT",
                    // ★ Added 2026-07-31 (task #127c) — `kgspInitGspTraceCrashBuffer`
                    // sends it from inside `kgspInitRm_IMPL` and asserts on the status
                    // (`ogkm-580: kernel_gsp.c:3396-3402, 4239`). The consumer is
                    // `kayfabe_device::inert`, which acknowledges it and does nothing —
                    // deliberately, and with the reason written down there.
                    "INIT_GSP_TRACE_CRASH_BUFFER",
                ],
                doc: "RPC function IDs (`rpc_global_enums.h`, `X(RM, NAME, id)`).",
            },
            MacroListReq {
                header: RPC_ENUMS_H,
                prefix: "E",
                arity: 2,
                rust_prefix: "NV_VGPU_MSG_EVENT_",
                keep: &[
                    "FIRST_EVENT",
                    "GSP_INIT_DONE",
                    "GSP_RUN_CPU_SEQUENCER",
                    "POST_EVENT",
                    "RC_TRIGGERED",
                ],
                doc: "GSP→CPU event IDs (`rpc_global_enums.h`, `E(NAME, id)`).",
            },
        ],
        // `rpc_message_header_v03_00` embeds `rpc_message_rpc_union_field_v u;`
        // BY VALUE. That alias resolves to the union
        // `rpc_message_rpc_union_field_v03_00 { NvU32 spare; NvU32 cpuRmGfid; }`
        // in the same header, which the generator lays out and compares against
        // the declaration below — so this is a checked declaration, not a guess.
        aggregate_scalars: &[AggregateScalar {
            scalar: ctype::Scalar {
                c_name: "rpc_message_rpc_union_field_v",
                size: 4,
                align: 4,
                rust: "u32",
                prim: "u32",
            },
            verify_against: "rpc_message_rpc_union_field_v03_00",
            verify_header: RPC_HDR_H,
        }],
        aggregate_lookup: &[ctype::Scalar {
            c_name: "rpc_message_rpc_union_field_v",
            size: 4,
            align: 4,
            rust: "u32",
            prim: "u32",
        }],
    },
    ModuleReq {
        file: "vbios.rs",
        title: "The VBIOS ROM / BIT-table / FWSEC-descriptor vocabulary — the synthetic-ROM seam.",
        doc: "\
The constants a **synthetic VBIOS** must be built out of, so that the guest's own
`kgspExtractVbiosFromRom_TU102` → `kgspParseFwsecUcodeFromVbiosImg` path accepts
it. Consumed by `crate::vbios`, which is the builder.

★ Three of the five sources are `.c` files. NVIDIA declares the BIT-table and
FWSEC-descriptor vocabulary inside `kernel_gsp_fwsec.c` itself, the ROM
code-type constants inside `kernel_gsp_vbios_tu102.c`, and the FWSEC
**application interface table** — the three typedefs `s_vbiosPatchInterfaceData`
walks inside the DMEM payload — inside `kernel_gsp_frts_tu102.c`. There is no
header to read any of them from, so the implementation file **is** the
authoritative statement and is what the generator scans.

# Why this is structure and not secrets

The driver performs **no cryptographic verification** of what it parses here.
`kernel_gsp_fwsec.c:993` copies the signature blob out of the image with a plain
`portMemCopy`; `kernel_gsp_frts_tu102.c:355` checks only that the pointer is
non-`NULL`; `:397` hands one signature to the falcon. Everything that *looks*
cryptographic in this path is a magic (`BIT_HEADER_SIGNATURE` = `\"BIT\\0\"`), a
size (`BCRT30_RSA3K_SIG_SIZE` = 384), a one-byte checksum (the BIT header), or a
bounds check (`portSafeAddU32`, offsets `<= biosSize`). Verification happens on
the falcon — and in Mode 2 we *are* the falcon. So a generated image needs a
signature blob of the right **size** at the declared **offset**, and its contents
are never inspected by anything outside our control.

# Why a generated image and not a dumped one

A dumped ROM describes the **host's** card. We emulate a *different* device — our
own FB size, our own straps, our own PCI identity — so a dumped image can
silently disagree with the registers the device answers. An image generated from
the same profile that drives those registers cannot disagree, by construction.

# Version stability (measured, not assumed)

Both vendored ogkm tags — 580.159.04 and 610.43.02 — carry `kernel_gsp_fwsec.c`,
`kernel_gsp_vbios_tu102.c`, `kernel_gsp_frts_tu102.c`, `pci_exp_table.h` and
`dev_bus.h` **byte-identically** (`diff` is empty on all five; the fifth,
`kernel_gsp_frts_tu102.c`, was diffed on 2026-07-31 when the interface-table
structs below were added). So every constant in this module has the same value at
both, and `crate::vbios::VbiosWire` has exactly one variant. That is a
measurement, and the generator re-checks it every time it is run against a tree.",
        structs: &[
            // ── The FWSEC application interface table (`kernel_gsp_frts_tu102.c`) ──
            //
            // Read by `s_vbiosPatchInterfaceData`, which is where a driver that has
            // ALREADY accepted the ROM stops if the DMEM payload does not describe
            // itself: `failed to find required interface entry for FWSEC cmd 0x15`.
            // Mirrored rather than transcribed because every one of these sizes is a
            // stride the walk uses (`interfaceOffset + sizeof(hdr)`, then
            // `curOffset + sizeof(entry)` per entry, then
            // `dmemOffset + sizeof(mapper)`), and a stride that is one byte wrong
            // reads the next field.
            StructReq {
                header: FRTS_TU102_C,
                name: "FALCON_APPLICATION_INTERFACE_HEADER_V1",
                fam_align: None,
            },
            StructReq {
                header: FRTS_TU102_C,
                name: "FALCON_APPLICATION_INTERFACE_ENTRY_V1",
                fam_align: None,
            },
            StructReq {
                header: FRTS_TU102_C,
                name: "FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3",
                fam_align: None,
            },
            // The command payloads `s_prepareForFwsec_TU102` builds and hands to
            // `s_vbiosPatchInterfaceData` as `pCmdBuffer`/`cmdBufferSize`. Their
            // sizes are what `cmd_in_buffer_size` has to be able to hold, and what
            // gets `portMemCopy`d into DMEM at `cmd_in_buffer_offset`.
            StructReq {
                header: FRTS_TU102_C,
                name: "FWSECLIC_READ_VBIOS_DESC",
                fam_align: None,
            },
            StructReq {
                header: FRTS_TU102_C,
                name: "FWSECLIC_FRTS_REGION_DESC",
                fam_align: None,
            },
        ],
        consts: &[
            // ── The FWSEC application interface table (`kernel_gsp_frts_tu102.c`) ──
            vbios_const(
                FRTS_TU102_C,
                "FALCON_APPLICATION_INTERFACE_ENTRY_ID_DMEMMAPPER",
                "u32",
                "`0x4` — the one entry `id` `s_vbiosPatchInterfaceData` is looking\n\
                 for. An interface table with `entryCount >= 2` but no entry\n\
                 carrying this id fails with `failed to find required interface\n\
                 entry`, which is a REFUSAL of the whole adapter init.",
            ),
            vbios_const(
                FRTS_TU102_C,
                "FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3_CMD_FRTS",
                "u32",
                "`0x15` — the command written into the mapper's `init_cmd` for the\n\
                 FRTS (WPR2 carve-out) request. The first of the two FWSEC commands\n\
                 the driver issues, and the one in the failure message.",
            ),
            vbios_const(
                FRTS_TU102_C,
                "FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3_CMD_SB",
                "u32",
                "`0x19` — the second command (`SB`), issued through the same table.\n\
                 A table that only satisfies FRTS would stop the boot one step later.",
            ),
            // ── The PCI expansion-ROM container (`s_locateExpansionRoms`) ────
            vbios_const(
                PCI_EXP_TABLE_H,
                "PCI_EXP_ROM_SIGNATURE",
                "u16",
                "`0xAA55` at image offset 0 — what `IS_VALID_PCI_ROM_SIG` accepts,\n\
                 and the byte pattern whose absence produced `did not find valid ROM\n\
                 signature`. Placing it at offset 0 is what lets a generated image\n\
                 skip the IFR/ROM-directory path entirely.",
            ),
            vbios_const(
                PCI_EXP_TABLE_H,
                "OFFSETOF_PCI_EXP_ROM_SIG",
                "usize",
                "Offset of the ROM signature within an expansion-ROM image.",
            ),
            vbios_const(
                PCI_EXP_TABLE_H,
                "OFFSETOF_PCI_EXP_ROM_PCI_DATA_STRUCT_PTR",
                "usize",
                "Offset of the `u16` pointer to this image's PCIR structure.",
            ),
            vbios_const(
                PCI_EXP_TABLE_H,
                "PCI_DATA_STRUCT_SIGNATURE",
                "u32",
                "`\"PCIR\"` — the PCI Data Structure magic `IS_VALID_PCI_DATA_SIG` accepts.",
            ),
            vbios_const(
                PCI_EXP_TABLE_H,
                "OFFSETOF_PCI_DATA_STRUCT_SIG",
                "usize",
                "`PCIR` magic, within the PCI Data Structure.",
            ),
            vbios_const(
                PCI_EXP_TABLE_H,
                "OFFSETOF_PCI_DATA_STRUCT_VENDOR_ID",
                "usize",
                "PCI vendor ID (`u16`), then device ID at +2.",
            ),
            vbios_const(
                PCI_EXP_TABLE_H,
                "OFFSETOF_PCI_DATA_STRUCT_LEN",
                "usize",
                "Length of the PCI Data Structure (`u16`) — also what positions the\n\
                 NPDE extension, at `(pcir + len + 0xF) & ~0xF`.",
            ),
            vbios_const(
                PCI_EXP_TABLE_H,
                "OFFSETOF_PCI_DATA_STRUCT_CLASS_CODE",
                "usize",
                "3-byte PCI class code.",
            ),
            vbios_const(
                PCI_EXP_TABLE_H,
                "OFFSETOF_PCI_DATA_STRUCT_IMAGE_LEN",
                "usize",
                "Image length (`u16`) in 512-byte blocks.",
            ),
            vbios_const(
                PCI_EXP_TABLE_H,
                "OFFSETOF_PCI_DATA_STRUCT_CODE_TYPE",
                "usize",
                "Code type (`u8`) — selects BASE vs EXT in `s_locateExpansionRoms`.",
            ),
            vbios_const(
                PCI_EXP_TABLE_H,
                "OFFSETOF_PCI_DATA_STRUCT_LAST_IMAGE",
                "usize",
                "Last-image indicator (`u8`); bit 7 terminates the walk.",
            ),
            vbios_const(
                PCI_EXP_TABLE_H,
                "PCI_LAST_IMAGE",
                "u8",
                "`NVBIT(7)` — the bit that ends `s_locateExpansionRoms`' `for(;;)`.\n\
                 Without it the walk runs off the end of the image.",
            ),
            vbios_const(
                PCI_EXP_TABLE_H,
                "PCI_ROM_IMAGE_BLOCK_SIZE",
                "usize",
                "512 — the unit `IMAGE_LEN` and `SUBIMAGE_LEN` count in, and therefore\n\
                 the granularity a generated image must be padded to.",
            ),
            vbios_const(
                PCI_EXP_TABLE_H,
                "NV_PCI_DATA_EXT_SIG",
                "u32",
                "`\"NPDE\"` — NVIDIA's PCI Data Extension magic.",
            ),
            vbios_const(
                PCI_EXP_TABLE_H,
                "NV_PCI_DATA_EXT_REV_11",
                "u16",
                "NPDE revision 1.1 — one of the two revisions the walk honours.",
            ),
            vbios_const(
                PCI_EXP_TABLE_H,
                "OFFSETOF_PCI_DATA_EXT_STRUCT_SIG",
                "usize",
                "`NPDE` magic, within the extension.",
            ),
            vbios_const(
                PCI_EXP_TABLE_H,
                "OFFSETOF_PCI_DATA_EXT_STRUCT_REV",
                "usize",
                "NPDE revision (`u16`).",
            ),
            vbios_const(
                PCI_EXP_TABLE_H,
                "OFFSETOF_PCI_DATA_EXT_STRUCT_LEN",
                "usize",
                "NPDE length (`u16`) — gates whether `LAST_IMAGE` below is even read.",
            ),
            vbios_const(
                PCI_EXP_TABLE_H,
                "OFFSETOF_PCI_DATA_EXT_STRUCT_SUBIMAGE_LEN",
                "usize",
                "NPDE sub-image length (`u16`, blocks) — **overrides** `IMAGE_LEN`\n\
                 when the NPDE is present, and is what actually advances the walk.",
            ),
            vbios_const(
                PCI_EXP_TABLE_H,
                "OFFSETOF_PCI_DATA_EXT_STRUCT_LAST_IMAGE",
                "usize",
                "NPDE last-image indicator (`u8`).",
            ),
            // ── ROM code types (`kernel_gsp_vbios_tu102.c`) ──────────────────
            vbios_const(
                VBIOS_TU102_C,
                "NV_BCRT_HASH_INFO_BASE_CODE_TYPE_VBIOS_BASE",
                "u8",
                "Code type of the **base** VBIOS image; its block size becomes\n\
                 `baseRomSize` in the `expansionRomOffset` computation.",
            ),
            vbios_const(
                VBIOS_TU102_C,
                "NV_BCRT_HASH_INFO_BASE_CODE_TYPE_VBIOS_EXT",
                "u8",
                "Code type of an **extended** VBIOS image; the first one's offset\n\
                 becomes `extRomOffset`. `expansionRomOffset = extRomOffset -\n\
                 baseRomSize`, and is 0 when either is absent.",
            ),
            // ── The BIT table (`kernel_gsp_fwsec.c`) ─────────────────────────
            vbios_const(
                FWSEC_C,
                "BIT_HEADER_ID",
                "u16",
                "`0xB8FF` — the `u16` `s_vbiosFindBitHeader` scans the whole image for.",
            ),
            vbios_const(
                FWSEC_C,
                "BIT_HEADER_SIGNATURE",
                "u32",
                "`\"BIT\\0\"` — the `u32` at `bitAddr + 2` that confirms a candidate.",
            ),
            vbios_const(
                FWSEC_C,
                "BIT_HEADER_SIZE_OFFSET",
                "usize",
                "Offset of `HeaderSize` — the byte count the header checksum covers.",
            ),
            vbios_const(
                FWSEC_C,
                "BIT_TOKEN_V1_00_SIZE_6",
                "u8",
                "Token stride below which the 6-byte token format is used (`u16` DataPtr).",
            ),
            vbios_const(
                FWSEC_C,
                "BIT_TOKEN_V1_00_SIZE_8",
                "u8",
                "Token stride at or above which the 8-byte format is used — the one a\n\
                 generated image wants, because its `DataPtr` is a full `u32`.",
            ),
            vbios_const(
                FWSEC_C,
                "BIT_TOKEN_BIOSDATA",
                "u8",
                "`0x42` — the token carrying the VBIOS version the driver reports.",
            ),
            vbios_const(
                FWSEC_C,
                "BIT_TOKEN_FALCON_DATA",
                "u8",
                "`0x70` — the token that points at the falcon ucode table. THE one\n\
                 that matters: without it FWSEC is never found.",
            ),
            vbios_const(
                FWSEC_C,
                "BIT_DATA_BIOSDATA_VERSION_2",
                "u8",
                "`DataVersion` accepted for a BIOSDATA token.",
            ),
            vbios_const(
                FWSEC_C,
                "BIT_DATA_BIOSDATA_BINVER_SIZE_5",
                "u16",
                "The BIOSDATA token's `DataSize` must be **strictly greater** than this\n\
                 for the version to be read (`bitToken.DataSize > 5`).",
            ),
            vbios_const(
                FWSEC_C,
                "BIT_DATA_FALCON_DATA_V2_SIZE_4",
                "u16",
                "Minimum `DataSize` of a falcon-data token; its payload is one `u32`\n\
                 `FalconUcodeTablePtr`.",
            ),
            // ── The falcon ucode table ───────────────────────────────────────
            vbios_const(
                FWSEC_C,
                "FALCON_UCODE_TABLE_HDR_V1_VERSION",
                "u8",
                "Required `Version` of the falcon ucode table header, else skipped.",
            ),
            vbios_const(
                FWSEC_C,
                "FALCON_UCODE_TABLE_HDR_V1_SIZE_6",
                "u8",
                "Minimum `HeaderSize`; also the stride from header to first entry.",
            ),
            vbios_const(
                FWSEC_C,
                "FALCON_UCODE_TABLE_ENTRY_V1_SIZE_6",
                "u8",
                "Minimum `EntrySize`; the entry is `2b1d` = appId, targetId, descPtr.",
            ),
            vbios_const(
                FWSEC_C,
                "FALCON_UCODE_ENTRY_APPID_FIRMWARE_SEC_LIC",
                "u8",
                "★ `0x05`. Read the skip condition carefully: an entry with this appId\n\
                 matches **regardless of `bUseDebugFwsec`**, because it short-circuits\n\
                 the `&&` before the debug/prod test is reached. That makes it the one\n\
                 appId a generated image can rely on without knowing whether the\n\
                 emulated GPU will read as debug-fused or prod-fused.",
            ),
            vbios_const(
                FWSEC_C,
                "FALCON_UCODE_ENTRY_APPID_FWSEC_DBG",
                "u8",
                "`0x45` — matched only when `kgspIsDebugModeEnabled_HAL` says debug.",
            ),
            vbios_const(
                FWSEC_C,
                "FALCON_UCODE_ENTRY_APPID_FWSEC_PROD",
                "u8",
                "`0x85` — matched only when it says prod.",
            ),
            // ── The FWSEC ucode descriptor ───────────────────────────────────
            vbios_const(
                FWSEC_C,
                "NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_FLAGS_VERSION_AVAILABLE",
                "u32",
                "The `vDesc` flag bit that must be set, else the entry is skipped with\n\
                 `unexpected ucode desc version missing`.",
            ),
            vbios_const(
                FWSEC_C,
                "NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_VERSION_V2",
                "u32",
                "Descriptor version 2 — the 60-byte boot-with-loader shape.",
            ),
            vbios_const(
                FWSEC_C,
                "NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_VERSION_V3",
                "u32",
                "Descriptor version 3 — the 44-byte boot-from-HS shape, the one with\n\
                 `PKCDataOffset`/`SignatureCount` and the one a modern part uses.",
            ),
            vbios_const(
                FWSEC_C,
                "FALCON_UCODE_DESC_V2_SIZE_60",
                "usize",
                "`sizeof` the V2 descriptor (`15d`); the declared `vDesc` size must be\n\
                 at least this for V2 to be accepted.",
            ),
            vbios_const(
                FWSEC_C,
                "FALCON_UCODE_DESC_V3_SIZE_44",
                "usize",
                "`sizeof` the V3 descriptor (`9d1w2b2w`). ★ Doubly load-bearing: the\n\
                 declared `vDesc` size must be `>=` it, **and** `signaturesTotalSize =\n\
                 descSize - 44` — so the signature blob's length is defined entirely by\n\
                 how much larger than 44 the descriptor claims to be.",
            ),
            vbios_const(
                FWSEC_C,
                "BCRT30_RSA3K_SIG_SIZE",
                "usize",
                "384 — the per-signature size the bounds checks use (`sigDataOffset +\n\
                 sigSize <= size`). A size constant, not a key: nothing verifies the\n\
                 384 bytes themselves anywhere in this path.",
            ),
        ],
        drfs: &[
            vbios_drf(
                FWSEC_C,
                "NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_FLAGS_VERSION",
                "Whether a descriptor version is present at all. Must be\n\
                 `_AVAILABLE`, else `s_vbiosParseFwsecUcodeDescFromBit` skips the entry.",
            ),
            vbios_drf(
                FWSEC_C,
                "NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_VERSION",
                "The descriptor version (2 or 3) inside the `vDesc` word.",
            ),
            vbios_drf(
                FWSEC_C,
                "NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_SIZE",
                "The descriptor size inside the `vDesc` word — 16 bits, which is the\n\
                 hard ceiling on `44 + signatureCount * 384` and therefore on how many\n\
                 signatures a descriptor can carry.",
            ),
        ],
        macro_lists: &[],
        aggregate_scalars: &[],
        aggregate_lookup: &[],
    },
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let [_, ogkm_root, out] = args.as_slice() else {
        eprintln!("usage: kayfabe-abi-gen <ogkm-root> <out-dir>");
        return ExitCode::FAILURE;
    };
    match run(Path::new(ogkm_root), Path::new(out)) {
        Ok(files) => {
            for f in files {
                println!("wrote {}", f.display());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("kayfabe-abi-gen: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(ogkm_root: &Path, out: &Path) -> Result<Vec<PathBuf>, String> {
    let version = read_version(ogkm_root)?;

    // Global `#define` table for array lengths.
    let mut defines: BTreeMap<String, usize> = BTreeMap::new();
    for h in DEFINE_SOURCES {
        let clean = strip_comments(&read(ogkm_root, h)?);
        defines.extend(scan_defines(&clean));
    }

    std::fs::create_dir_all(out).map_err(|e| format!("create {}: {e}", out.display()))?;
    let mut written = Vec::new();
    let mut mod_entries = Vec::new();

    for m in MODULES {
        // A declared by-value aggregate must match its own definition in the
        // header. This is the only hand-supplied size in the pipeline, so it is
        // the only one that needs an independent check.
        for ag in m.aggregate_scalars {
            verify_aggregate_scalar(ogkm_root, ag, &defines, m.aggregate_lookup)?;
        }
        let mut layouts = Vec::new();
        for sr in m.structs {
            layouts.push(build_struct(ogkm_root, sr, &defines, m.aggregate_lookup)?);
        }
        let mut consts: Vec<(String, String, String, String)> = Vec::new();
        for cr in m.consts {
            let clean = strip_comments(&read(ogkm_root, cr.header)?);
            let d = scan_defines(&clean);
            let v = d.get(cr.c_name).ok_or_else(|| {
                format!(
                    "`{}` not found as an integral #define in {}",
                    cr.c_name, cr.header
                )
            })?;
            let doc = format!("{}\n\nogkm `{}`.", cr.doc, cr.header);
            consts.push((
                cr.rust_name.to_string(),
                cr.rust_ty.to_string(),
                format!("{v:#x}"),
                doc,
            ));
        }
        for dr in m.drfs {
            let clean = strip_comments(&read(ogkm_root, dr.header)?);
            let d = scan_drf_ranges(&clean);
            let (hi, lo) = d.get(dr.c_name).copied().ok_or_else(|| {
                format!(
                    "`{}` not found as a `hi:lo` DRF range in {}",
                    dr.c_name, dr.header
                )
            })?;
            let doc = format!(
                "{}\n\n`{}` = `{hi}:{lo}` — ogkm `{}`.",
                dr.doc, dr.c_name, dr.header
            );
            consts.push((
                dr.rust_name.to_string(),
                "Drf".to_string(),
                format!("Drf::new({hi}, {lo})"),
                doc,
            ));
        }
        for ml in m.macro_lists {
            let clean = strip_comments(&read(ogkm_root, ml.header)?);
            let all = scan_macro_list(&clean, ml.prefix, ml.arity);
            if all.is_empty() {
                return Err(format!(
                    "macro list `{}(…)` in {} scanned EMPTY — a silently-empty scan must never \
                     pass for success",
                    ml.prefix, ml.header
                ));
            }
            for want in ml.keep {
                let v = all
                    .iter()
                    .find(|(n, _)| n == want)
                    .map(|(_, v)| *v)
                    .ok_or_else(|| format!("`{}{}` not in {}", ml.prefix, want, ml.header))?;
                let doc = format!(
                    "{}\n\n`{}({want}, {v:#x})` — ogkm `{}`.",
                    ml.doc, ml.prefix, ml.header
                );
                consts.push((
                    format!("{}{want}", ml.rust_prefix),
                    "u32".to_string(),
                    format!("{v:#x}"),
                    doc,
                ));
            }
        }

        let text = emit::module(m.doc, &version, &layouts, &consts);
        let path = out.join(m.file);
        std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))?;
        written.push(path);
        mod_entries.push((m.file.trim_end_matches(".rs"), m.title));
    }

    // The `mod.rs` that ties the generated modules together.
    let mut modrs = String::new();
    modrs.push_str("// @generated by kayfabe-abi-gen — DO NOT EDIT BY HAND.\n");
    modrs.push_str(&format!(
        "//!\n//! Generated wire layouts from open-gpu-kernel-modules {version}.\n\
         //!\n//! See `crate::generated`'s parent module doc for the generation strategy and\n\
         //! `crates/kayfabe-abi/gen/src/main.rs` for the slice manifest.\n\n"
    ));
    for (m, doc) in &mod_entries {
        modrs.push_str(&format!("/// {doc}\npub mod {m};\n"));
    }
    modrs.push_str(&format!(
        "\n/// The ogkm checkout these modules were generated from.\npub const OGKM_VERSION: &str = {version:?};\n"
    ));
    let modpath = out.join("mod.rs");
    std::fs::write(&modpath, modrs).map_err(|e| format!("write {}: {e}", modpath.display()))?;
    written.push(modpath);

    // Format the output with the same rustfmt the repo's CI checks
    // (`cargo fmt --all --check`). Emitting text that rustfmt would rewrite
    // means the first `cargo fmt` produces a diff nobody authored, which
    // destroys the property this whole strategy rests on: that the committed
    // generated file is byte-identical to what the generator produces.
    rustfmt(&written)?;

    Ok(written)
}

fn rustfmt(files: &[PathBuf]) -> Result<(), String> {
    let mut cmd = std::process::Command::new("rustfmt");
    cmd.arg("--edition").arg("2024");
    for f in files {
        cmd.arg(f);
    }
    let out = cmd.output().map_err(|e| {
        format!("running rustfmt (it must be on PATH — the output is committed): {e}")
    })?;
    if !out.status.success() {
        return Err(format!(
            "rustfmt failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

fn verify_aggregate_scalar(
    ogkm_root: &Path,
    ag: &AggregateScalar,
    defines: &BTreeMap<String, usize>,
    extra: &'static [ctype::Scalar],
) -> Result<(), String> {
    let clean = strip_comments(&read(ogkm_root, ag.verify_header)?);
    let aggs =
        find_aggregates(&clean).map_err(|e: ParseError| format!("{}: {e}", ag.verify_header))?;
    let found = aggs.get(ag.verify_against).ok_or_else(|| {
        format!(
            "`{}` not found in {} — cannot verify the declared aggregate scalar `{}`",
            ag.verify_against, ag.verify_header, ag.scalar.c_name
        )
    })?;
    if found.len() != 1 {
        return Err(format!(
            "`{}` is defined {} times in {} — refusing to verify against one silently",
            ag.verify_against,
            found.len(),
            ag.verify_header
        ));
    }
    let def = &found[0];
    let (fields, flex) = parse_fields(&def.body, def.body_line, defines)
        .map_err(|e| format!("{}: {}: {e}", ag.verify_header, ag.verify_against))?;
    if flex.is_some() {
        return Err(format!(
            "`{}` has a flexible array member; not a by-value scalar",
            ag.verify_against
        ));
    }
    let (size, align) = if def.is_union {
        ctype::lay_out_union(&fields, extra)
            .map_err(|e| format!("{}: {}: {e}", ag.verify_header, ag.verify_against))?
    } else {
        let l = lay_out(
            ag.verify_against,
            ag.verify_header,
            def.line,
            &fields,
            None,
            extra,
        )
        .map_err(|e| format!("{}: {}: {e}", ag.verify_header, ag.verify_against))?;
        (l.size, l.align)
    };
    if size != ag.scalar.size || align != ag.scalar.align {
        return Err(format!(
            "declared aggregate scalar `{}` says size={} align={}, but `{}` in {} lays out as \
             size={size} align={align}",
            ag.scalar.c_name, ag.scalar.size, ag.scalar.align, ag.verify_against, ag.verify_header
        ));
    }
    Ok(())
}

fn build_struct(
    ogkm_root: &Path,
    sr: &StructReq,
    defines: &BTreeMap<String, usize>,
    extra: &'static [ctype::Scalar],
) -> Result<Layout, String> {
    let clean = strip_comments(&read(ogkm_root, sr.header)?);
    let aggs = find_aggregates(&clean).map_err(|e: ParseError| format!("{}: {e}", sr.header))?;
    let found = aggs
        .get(sr.name)
        .ok_or_else(|| format!("`{}` not found in {}", sr.name, sr.header))?;
    if found.len() != 1 {
        return Err(format!(
            "`{}` is defined {} times in {} — refusing to pick one silently",
            sr.name,
            found.len(),
            sr.header
        ));
    }
    let agg = &found[0];
    let (fields, flexible) = parse_fields(&agg.body, agg.body_line, defines)
        .map_err(|e| format!("{}: {}: {e}", sr.header, sr.name))?;

    let flex = match (&flexible, sr.fam_align) {
        (Some((name, _elem)), Some(a)) => Some((name.clone(), a)),
        (Some((name, elem)), None) => {
            return Err(format!(
                "{}: `{}` ends in a flexible array member `{name}[]` of type `{elem}` but the \
                 manifest declares no `fam_align` — the element's alignment is not stated locally \
                 and must not be guessed",
                sr.header, sr.name
            ));
        }
        (None, Some(_)) => {
            return Err(format!(
                "{}: `{}` has no flexible array member but the manifest declares `fam_align`",
                sr.header, sr.name
            ));
        }
        (None, None) => None,
    };

    let with_fam = lay_out(sr.name, sr.header, agg.line, &fields, flex.clone(), extra)
        .map_err(|e| format!("{}: {}: {e}", sr.header, sr.name))?;

    if flex.is_some() {
        // The emitted Rust mirror OMITS the flexible array member, so rustc will
        // compute the field-only size. If the FAM's alignment would change
        // `sizeof`, a plain `#[repr(C)]` mirror is not equal to the C type and we
        // must not pretend otherwise.
        let fields_only = lay_out(sr.name, sr.header, agg.line, &fields, None, extra)
            .map_err(|e| format!("{}: {}: {e}", sr.header, sr.name))?;
        if fields_only.size != with_fam.size {
            return Err(format!(
                "{}: `{}`: the flexible array member's alignment changes sizeof ({} vs {}); the \
                 emitted #[repr(C)] mirror omits the FAM and would be wrong",
                sr.header, sr.name, fields_only.size, with_fam.size
            ));
        }
        return Ok(fields_only);
    }
    Ok(with_fam)
}

fn read(ogkm_root: &Path, rel: &str) -> Result<String, String> {
    let p = ogkm_root.join(rel);
    std::fs::read_to_string(&p).map_err(|e| format!("read {}: {e}", p.display()))
}

fn read_version(ogkm_root: &Path) -> Result<String, String> {
    let mk = read(ogkm_root, "version.mk")?;
    for l in mk.lines() {
        if let Some(v) = l.trim().strip_prefix("NVIDIA_VERSION") {
            return Ok(v.trim_start_matches([' ', '=']).trim().to_string());
        }
    }
    Err("NVIDIA_VERSION not found in version.mk".to_string())
}
