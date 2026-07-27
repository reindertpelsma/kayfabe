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
    ParseError, find_aggregates, parse_fields, scan_defines, scan_macro_list, strip_comments,
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
const RPC_HDR_H: &str = "src/nvidia/generated/g_rpc-message-header.h";
const RPC_ENUMS_H: &str = "src/nvidia/inc/kernel/vgpu/rpc_global_enums.h";

/// Headers whose `#define`s feed array-length resolution, globally.
const DEFINE_SOURCES: &[&str] = &["src/common/sdk/nvidia/inc/nvlimits.h", NVOS_H];

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
        macro_lists: &[],
        aggregate_scalars: &[],
        aggregate_lookup: &[],
    },
    ModuleReq {
        file: "classes.rs",
        title: "Per-class alloc-param structs and their class IDs.",
        doc: "\
The two alloc-param structs the core's `AllocFacts` actually reads today.

- `NV0000_ALLOC_PARAMETERS` (`NV01_ROOT`) carries `processID`, the decision-#14
  client-kind discriminator (`l1_concurrency.md` §12.27).
- `NV0080_ALLOC_PARAMETERS` (`NV01_DEVICE_0`) carries `deviceId`, the multi-GPU
  routing fact (`multi_gpu_and_mig.md`).

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
        ],
        macro_lists: &[],
        aggregate_scalars: &[],
        aggregate_lookup: &[],
    },
    ModuleReq {
        file: "ctrl.rs",
        title: "RM control commands and their payload structs.",
        doc: "\
The one RM control command in the slice: `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY`.

This is where a VAS's page-directory base is declared, i.e. where the data-plane
identity (`Pdb`) is born — `RmEvent::SetPageDir`. It arrives inside an
`NVOS54_PARAMETERS` as the control payload.

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
        structs: &[StructReq {
            header: CTRL0080DMA_H,
            name: "NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS",
            fam_align: None,
        }],
        consts: &[ConstReq {
            header: CTRL0080DMA_H,
            c_name: "NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY",
            rust_name: "NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY",
            rust_ty: "u32",
            doc: "`NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` — the control command whose\npayload is [`Nv0080CtrlDmaSetPageDirectoryParams`].",
        }],
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
        structs: &[StructReq {
            header: RPC_HDR_H,
            name: "rpc_message_header_v03_00",
            // `rpc_generic_union` is a union over the RPC payload structs, whose
            // arms include `NV_DECLARE_ALIGNED(NvU64 …, 8)` members, so 8 is the
            // conservative bound. The generator asserts that using it does not
            // change `sizeof` versus the field-only computation (it does not:
            // the named fields already end at 32, a multiple of 8).
            fam_align: Some(8),
        }],
        consts: &[],
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
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let [_, ogkm, out] = args.as_slice() else {
        eprintln!("usage: kayfabe-abi-gen <ogkm-root> <out-dir>");
        return ExitCode::FAILURE;
    };
    match run(Path::new(ogkm), Path::new(out)) {
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

fn run(ogkm: &Path, out: &Path) -> Result<Vec<PathBuf>, String> {
    let version = read_version(ogkm)?;

    // Global `#define` table for array lengths.
    let mut defines: BTreeMap<String, usize> = BTreeMap::new();
    for h in DEFINE_SOURCES {
        let clean = strip_comments(&read(ogkm, h)?);
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
            verify_aggregate_scalar(ogkm, ag, &defines, m.aggregate_lookup)?;
        }
        let mut layouts = Vec::new();
        for sr in m.structs {
            layouts.push(build_struct(ogkm, sr, &defines, m.aggregate_lookup)?);
        }
        let mut consts: Vec<(String, String, String, String)> = Vec::new();
        for cr in m.consts {
            let clean = strip_comments(&read(ogkm, cr.header)?);
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
        for ml in m.macro_lists {
            let clean = strip_comments(&read(ogkm, ml.header)?);
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
    ogkm: &Path,
    ag: &AggregateScalar,
    defines: &BTreeMap<String, usize>,
    extra: &'static [ctype::Scalar],
) -> Result<(), String> {
    let clean = strip_comments(&read(ogkm, ag.verify_header)?);
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
    ogkm: &Path,
    sr: &StructReq,
    defines: &BTreeMap<String, usize>,
    extra: &'static [ctype::Scalar],
) -> Result<Layout, String> {
    let clean = strip_comments(&read(ogkm, sr.header)?);
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

fn read(ogkm: &Path, rel: &str) -> Result<String, String> {
    let p = ogkm.join(rel);
    std::fs::read_to_string(&p).map_err(|e| format!("read {}: {e}", p.display()))
}

fn read_version(ogkm: &Path) -> Result<String, String> {
    let mk = read(ogkm, "version.mk")?;
    for l in mk.lines() {
        if let Some(v) = l.trim().strip_prefix("NVIDIA_VERSION") {
            return Ok(v.trim_start_matches([' ', '=']).trim().to_string());
        }
    }
    Err("NVIDIA_VERSION not found in version.mk".to_string())
}
