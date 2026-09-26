//! Build the **VBIOS differential oracle** — NVIDIA's own parser, compiled as a
//! test-only executable, one per vendored open-kernel-modules tag.
//!
//! See `oracle/vbios_oracle.c` for what it is and why (including the licensing note).
//! This script only decides *whether* it can be built and hands the paths to the test.
//!
//! # The skip is loud, and the failure polarity is deliberate
//!
//! * A vendored tree **absent** → skip, with a `cargo:warning=` naming the tree and the
//!   env var that would point at it. Nothing else in the workspace is affected.
//! * A vendored tree **present** but the harness fails to compile → **hard error**. That
//!   is not a machine without the oracle, it is an oracle that has rotted, and letting it
//!   degrade to a skip is exactly how a gate stops being able to fire.
//!
//! Neither arm ever substitutes a stand-in. There is no fallback parser, because a
//! fallback parser is the transcription this oracle exists to replace.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The vendored trees, as `(env override, default path, cargo:rustc-env name)`.
///
/// The default paths point OUT of this repository on purpose: the open kernel modules are
/// NVIDIA's (MIT / GPL-2.0) and are not vendored here. Set the env var to relocate.
const TREES: &[(&str, &str, &str)] = &[
    (
        "KAYFABE_OGKM_580",
        "/workspace/nvidia-gpu-passthrough/research_clones/ogkm-580.159.04",
        "KAYFABE_VBIOS_ORACLE_580",
    ),
    (
        "KAYFABE_OGKM_610",
        "/workspace/nvidia-gpu-passthrough/research_clones/ogkm",
        "KAYFABE_VBIOS_ORACLE_610",
    ),
];

/// The parser, relative to a tree root. Both files are compiled UNMODIFIED.
const VBIOS_TU102_C: &str = "src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_vbios_tu102.c";
const FWSEC_C: &str = "src/nvidia/src/kernel/gpu/gsp/kernel_gsp_fwsec.c";
/// The **FWSEC interface walk**, one stage past the parser.
///
/// ★ This file cannot be `#include`d whole the way the two above are: it also
/// contains `s_prepareForFwsec_TU102`, `kgspExecuteFwsec_TU102` and the FRTS
/// entry points, which pull in HAL dispatch, `KernelFalcon`, `memdescMapInternal`
/// and half of `OBJGPU`. So [`extract_interface_slice`] takes a **byte-for-byte
/// range** of it — the three interface typedefs, the FWSEC command structures,
/// and `s_vbiosPatchInterfaceData` itself — and compiles that. Nothing is
/// rewritten; the extraction is delimited by the file's own text and the result
/// is checked for the anchors it must contain, so a slice that silently caught
/// the wrong region is a hard build error rather than a weaker oracle.
const FRTS_TU102_C: &str = "src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_frts_tu102.c";

/// The tree's own build configuration, lifted from `src/nvidia/Makefile`.
///
/// These are not guesses: every `-D` below appears in that Makefile, and they decide real
/// behaviour in the headers the parser reads (`NV_ASSERT_FAILED_USES_STRINGS` picks the
/// assert helper's signature; `PORT_IS_CHECKED_BUILD=0` picks the non-breakpointing
/// assert; `RMCFG_FEATURE_GSP_CLIENT_RM` is what makes `IS_GSP_CLIENT` meaningful).
const DEFINES: &[&str] = &[
    "_LANGUAGE_C",
    "__NO_CTYPE",
    "NVRM",
    "LOCK_VAL_ENABLED=0",
    "PORT_ATOMIC_64_BIT_SUPPORTED=1",
    "PORT_IS_KERNEL_BUILD=1",
    "PORT_IS_CHECKED_BUILD=0",
    "PORT_MODULE_atomic=1",
    "PORT_MODULE_core=1",
    "PORT_MODULE_cpu=1",
    "PORT_MODULE_crypto=1",
    "PORT_MODULE_debug=1",
    "PORT_MODULE_memory=1",
    "PORT_MODULE_safe=1",
    "PORT_MODULE_string=1",
    "PORT_MODULE_sync=1",
    "PORT_MODULE_thread=1",
    "PORT_MODULE_util=1",
    "PORT_MODULE_example=0",
    "PORT_MODULE_mmio=0",
    "PORT_MODULE_time=0",
    "RS_STANDALONE=0",
    "RS_STANDALONE_TEST=0",
    "RS_COMPATABILITY_MODE=1",
    "RS_PROVIDES_API_STATE=0",
    "NV_CONTAINERS_NO_TEMPLATES",
    "INCLUDE_NVLINK_LIB",
    "INCLUDE_NVSWITCH_LIB",
    "NV_PRINTF_STRINGS_ALLOWED=1",
    "NV_ASSERT_FAILED_USES_STRINGS=1",
    "PORT_ASSERT_FAILED_USES_STRINGS=1",
];

/// The include search path, also lifted verbatim from `src/nvidia/Makefile`, as
/// `(root-relative-prefix, subdirectory)` where the prefix is `src/nvidia` or `src/common`.
const INCLUDES: &[(&str, &str)] = &[
    ("src/nvidia", "kernel/inc"),
    ("src/nvidia", "interface"),
    ("src/common", "sdk/nvidia/inc"),
    ("src/common", "sdk/nvidia/inc/hw"),
    ("src/nvidia", "arch/nvalloc/common/inc"),
    ("src/nvidia", "arch/nvalloc/common/inc/gsp"),
    ("src/nvidia", "arch/nvalloc/common/inc/deprecated"),
    ("src/nvidia", "arch/nvalloc/unix/include"),
    ("src/nvidia", "inc"),
    ("src/nvidia", "inc/os"),
    ("src/common", "shared/inc"),
    ("src/common", "shared/msgq/inc"),
    ("src/common", "inc"),
    ("src/common", "uproc/os/libos-v2.0.0/include"),
    ("src/common", "uproc/os/common/include"),
    ("src/common", "inc/swref"),
    ("src/common", "inc/swref/published"),
    ("src/nvidia", "generated"),
    ("src/common", "nvswitch/kernel/inc"),
    ("src/common", "nvswitch/interface"),
    ("src/common", "nvswitch/common/inc"),
    ("src/common", "inc/displayport"),
    ("src/common", "nvlink/interface"),
    ("src/common", "nvlink/inband/interface"),
    ("src/nvidia", "src/mm/uvm/interface"),
    ("src/nvidia", "inc/libraries"),
    ("src/nvidia", "src/libraries"),
    ("src/nvidia", "inc/kernel"),
];

/// Cut the interface-walk region out of `kernel_gsp_frts_tu102.c`, verbatim.
///
/// The region runs from the `typedef struct` that declares
/// `FALCON_APPLICATION_INTERFACE_HEADER_V1` to the closing brace of
/// `s_vbiosPatchInterfaceData`. Between them the file contains only typedefs,
/// `#define`s and comments — no other function — so the cut is a contiguous span
/// of the original bytes with nothing added and nothing removed.
///
/// Both ends are located by the file's OWN text rather than by line number: line
/// numbers move between driver releases and a slice that quietly caught the wrong
/// region would produce an oracle that agrees with us for the wrong reason. Every
/// step below returns `Err` with what it was looking for instead.
fn extract_interface_slice(src: &str) -> Result<&str, String> {
    let hdr_close = "} FALCON_APPLICATION_INTERFACE_HEADER_V1;";
    let hdr_end = src
        .find(hdr_close)
        .ok_or_else(|| format!("no `{hdr_close}` in the file"))?;
    let start = src[..hdr_end]
        .rfind("typedef struct")
        .ok_or("no `typedef struct` opening the interface header")?;

    let name = "s_vbiosPatchInterfaceData";
    let fn_at = src.find(name).ok_or_else(|| format!("no `{name}`"))?;
    // The definition's body opens with a brace in column 0, which is also the
    // only place a brace can appear in column 0 inside a function definition.
    let rel = src[fn_at..]
        .find("\n{\n")
        .ok_or_else(|| format!("`{name}` has no column-0 body"))?;
    let body = fn_at + rel + 1;
    let mut depth = 0usize;
    let mut end = None;
    for (i, ch) in src[body..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(body + i + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end.ok_or_else(|| format!("`{name}`'s body never closes"))?;
    let slice = &src[start..end];

    // The anchors the slice MUST contain. A cut that lands on the wrong region
    // still compiles — it would simply define fewer things — and the harness
    // would then fail to build with a confusing error somewhere else.
    for anchor in [
        "FALCON_APPLICATION_INTERFACE_HEADER_V1",
        "FALCON_APPLICATION_INTERFACE_ENTRY_V1",
        "FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3",
        "FALCON_APPLICATION_INTERFACE_ENTRY_ID_DMEMMAPPER",
        "FWSECLIC_FRTS_CMD",
        "s_vbiosPatchInterfaceData",
        "too few interface entires",
        "failed to find required interface entry",
    ] {
        if !slice.contains(anchor) {
            return Err(format!("the extracted slice does not contain `{anchor}`"));
        }
    }
    // …and what it must NOT, so a future refactor that moves another function
    // into this span is caught rather than dragging the whole HAL in.
    for forbidden in ["s_prepareForFwsec_TU102", "kgspExecuteFwsec_TU102"] {
        if slice.contains(forbidden) {
            return Err(format!("the extracted slice reaches `{forbidden}`"));
        }
    }
    Ok(slice)
}

// =====================================================================================
// The GMMU page-table format oracle — NVIDIA's own encoder, per-chip HAL and all
// =====================================================================================
//
// See `oracle/gmmu_fmt_oracle.c` for what it is and why. Everything below only decides
// WHICH driver code to compile, and it decides it by reading the driver rather than by
// carrying a list.

/// The chip whose HAL bindings the oracle is built with.
///
/// ★ This is the one thing here that is a CHOICE, and it is the same choice the shipped
/// port already made: `kayfabe_chips::Ga10xArch` is a GA106 (`crates/kayfabe-chips/
/// src/ga10x.rs`). Naming it here rather than deriving it is correct — nothing in the
/// driver can tell us which chip we are pretending to be.
const GMMU_ORACLE_CHIP: &str = "GA106";

/// The driver's own per-chip dispatch table and the header that declares its entry points.
const NVOC_KERN_GMMU_C: &str = "src/nvidia/generated/g_kern_gmmu_nvoc.c";
const NVOC_KERN_GMMU_H: &str = "src/nvidia/generated/g_kern_gmmu_nvoc.h";

/// Where a `kgmmu*` implementation can live. Searched recursively for the symbol the
/// dispatch table named.
const GMMU_IMPL_ROOT: &str = "src/nvidia/src/kernel/gpu/mmu";

/// The format abstraction itself — `gmmuFmtEntryIsPte`, `gmmuFmtGetPde`, the address-field
/// selectors and the level geometry. Always compiled; not chip-dependent.
const GMMU_SUPPORT_C: &[&str] = &[
    "src/nvidia/src/libraries/mmu/gmmu_fmt.c",
    "src/nvidia/src/libraries/mmu/mmu_fmt.c",
];

/// The **halified** entry points the oracle drives, each with the `-D` macro the harness
/// reads it through. These are exactly the calls `kgmmuConstructEngine_IMPL` makes to
/// build a format (`src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:77-99, 640-647`).
const GMMU_HALIFIED: &[(&str, &str)] = &[
    ("kgmmuFmtInitLevels", "OGKM_HAL_FMT_INIT_LEVELS"),
    ("kgmmuFmtInitPde", "OGKM_HAL_FMT_INIT_PDE"),
    ("kgmmuFmtInitPdeMulti", "OGKM_HAL_FMT_INIT_PDE_MULTI"),
    ("kgmmuFmtInitPte", "OGKM_HAL_FMT_INIT_PTE"),
    (
        "kgmmuFmtInitPdeApertures",
        "OGKM_HAL_FMT_INIT_PDE_APERTURES",
    ),
    (
        "kgmmuFmtInitPteApertures",
        "OGKM_HAL_FMT_INIT_PTE_APERTURES",
    ),
    (
        "kgmmuFmtInitPteComptagLine",
        "OGKM_HAL_FMT_INIT_PTE_COMPTAGLINE",
    ),
    ("kgmmuFmtFamiliesInit", "OGKM_HAL_FMT_FAMILIES_INIT"),
];

/// Entry points that are NOT in the dispatch table because they have exactly one
/// implementation: the generated header binds them with a plain `#define`.
const GMMU_DEFINED: &[(&str, &str)] = &[("kgmmuFmtInitCaps", "OGKM_HAL_FMT_INIT_CAPS")];

/// Implementation files that may be compiled WHOLE.
///
/// ★★ This is a **safety** declaration, not the universe. Which file is compiled is
/// derived from the symbol the driver's own dispatch table named; this list only records
/// which of those files have been read and found to be pure bit-packing — no `OBJGPU`
/// dereference, no `pRmApi->Control`, nothing that would need the object model mocked. A
/// derived file that is on neither this list nor [`GMMU_SLICE_ONLY`] is a **hard build
/// error** that names the file, because the alternative is an oracle that quietly grew a
/// mock maze.
const GMMU_WHOLE_FILE_OK: &[&str] = &[
    "kern_gmmu_fmt_gm10x.c",
    "kern_gmmu_fmt_gm20x.c",
    "kern_gmmu_fmt_gp10x.c",
    "kern_gmmu_fmt_tu10x.c",
    "kern_gmmu_fmt_ga10x.c",
    "kern_gmmu_gm200.c",
];

/// Implementation files that must be **sliced**: they hold a leaf-like function the oracle
/// needs AND a great deal of object-model code it must not drag in.
///
/// `kern_gmmu_gv100.c` is 2 630 lines of fault-buffer, interrupt-ownership and shadow-
/// buffer management around one 25-line function. The slice is byte-for-byte the driver's
/// own definition, located by its own text, with a two-line include prologue that is ours.
const GMMU_SLICE_ONLY: &[&str] = &["kern_gmmu_gv100.c"];

// =====================================================================================
// ★★★ The WORK-SUBMIT TOKEN oracle — increment E3's second instrument
// =====================================================================================
//
// See `oracle/worksubmit_token_oracle.c`. Everything here decides WHICH driver code is
// compiled, and it decides it by reading the driver.

/// The chip whose HAL binding the token oracle is built with — the same GA106 the shipped
/// `kayfabe_chips::Ga10xArch` is, named here for [`GMMU_ORACLE_CHIP`]'s reason: nothing in
/// the driver can tell us which chip we are pretending to be.
const TOKEN_ORACLE_CHIP: &str = "GA106";

/// The dispatch table that binds the token encoder per chip, and the header declaring it.
const NVOC_KERNEL_FIFO_C: &str = "src/nvidia/generated/g_kernel_fifo_nvoc.c";

/// Where a `kfifo*` implementation can live. Searched recursively for the symbol the
/// dispatch table named — so a driver release that moves or rebinds the encoder is
/// followed, or the build fails saying so.
const TOKEN_IMPL_ROOT: &str = "src/nvidia/src/kernel/gpu/fifo";

/// The halified entry point the oracle drives.
const TOKEN_HAL_FUNC: &str = "kfifoGenerateWorkSubmitTokenHal";

/// The channel-state accessors the encoder's own runlist gate reads, and the setter the
/// harness drives them with. Both live in `kernel_channel.c` beside 4 000 lines the
/// oracle must not compile, so both are sliced.
const TOKEN_KCHANNEL_C: &str = "src/nvidia/src/kernel/gpu/fifo/kernel_channel.c";
const TOKEN_KCHANNEL_FNS: &[&str] = &["kchannelIsRunlistSet", "kchannelSetRunlistSet"];

/// The two lines the whole increment turns on. If a driver release stops writing the
/// token with these, this oracle must **fail to build** rather than quietly judge our
/// decoder against a function that no longer encodes anything.
const TOKEN_SLICE_ANCHORS: &[&str] = &[
    "_VF_DOORBELL, _RUNLIST_ID",
    "_VF_DOORBELL, _VECTOR",
    "kchannelGetRunlistId",
    "kchannelIsRunlistSet",
];

// =====================================================================================
// ★★★ The PUSHBUFFER / USERD ABI oracle — increment E4's instrument
// =====================================================================================
//
// See `oracle/pushbuffer_abi_oracle.c`. Everything here decides WHICH driver code is
// compiled, and it decides it by reading the driver.

/// The chip whose HAL binding the pushbuffer/USERD oracle is built with — the same GA106
/// the shipped `kayfabe_chips::Ga10xArch` is, named here for [`GMMU_ORACLE_CHIP`]'s
/// reason: nothing in the driver can tell us which chip we are pretending to be.
const PUSHBUFFER_ORACLE_CHIP: &str = "GA106";

/// The halified entry point that decides how large a channel's USERD is.
///
/// ★★ **This is not decoration, and it is the reason the oracle exists at all for USERD.**
/// `kfifoGetUserdSizeAlign` is halified two ways in `g_kernel_fifo_nvoc.c`; only
/// `T234D`/`T264D` get their own arm and **GA106 falls to the fallback**, which is
/// *Maxwell's* `_GM107`. So an Ampere channel's USERD is sized out of
/// `published/maxwell/gm107/dev_ram.h`, and `published/ampere/ga102/dev_ram.h` — the
/// obvious place to look — contains no `NV_RAMUSERD` at all. Deriving the binding is what
/// keeps that choice the driver's; `a_table_does_not_decide_behaviour` is the memory that
/// names this exact mistake.
const USERD_SIZE_HAL_FUNC: &str = "kfifoGetUserdSizeAlign";

/// The class headers the oracle's own `#include`s reach, checked for presence before the
/// build is attempted so an absent one is a **skip naming the file** rather than a wall of
/// preprocessor errors.
const PUSHBUFFER_CLASS_HEADERS: &[&str] = &[
    "src/common/sdk/nvidia/inc/class/clc56f.h",
    "src/common/sdk/nvidia/inc/class/clc7b5.h",
    "src/common/sdk/nvidia/inc/nvmisc.h",
];

/// The generated header carrying the `SF_*` field accessors — the macros that turn a
/// `dev_ram.h` field extent into a **byte offset**.
const PUSHBUFFER_SF_HEADER: &str = "src/nvidia/generated/g_gpu_access_nvoc.h";

/// The two lines the USERD half of the increment turns on. If a release stops sizing
/// USERD with them, this oracle must **fail to build** rather than quietly judge our model
/// against a function that no longer sizes anything.
const USERD_SLICE_ANCHORS: &[&str] = &["NV_RAMUSERD_BASE_SHIFT", "pSize"];

/// Cut the `SF_*` accessor block out of the driver's generated header, byte for byte.
///
/// Same construction as [`extract_interface_slice`]: both ends are located by the file's
/// own text, and the result is checked for the anchors it must contain. The block is a
/// contiguous run of `#define SF_…` / `#undef SF_…` lines, so the cut is the run itself.
fn extract_sf_accessors(src: &str) -> Result<String, String> {
    let needle = "#define SF_OFFSET(";
    let at = src
        .find(needle)
        .ok_or_else(|| format!("no `{needle}` in the generated header"))?;
    // Walk back to the first line of the contiguous SF block…
    let mut start = src[..at].rfind('\n').map_or(0, |r| r + 1);
    while let Some(r) = src[..start.saturating_sub(1)].rfind('\n') {
        let prev_start = r + 1;
        let line = src[prev_start..start].trim_start();
        if !line.starts_with("#define SF_") && !line.starts_with("#undef  SF_") {
            break;
        }
        start = prev_start;
    }
    // …and forward to the last one.
    let mut end = start;
    for line in src[start..].split_inclusive('\n') {
        let t = line.trim_start();
        if t.starts_with("#define SF_") || t.starts_with("#undef  SF_") {
            end += line.len();
        } else {
            break;
        }
    }
    let slice = &src[start..end];
    for anchor in ["SF_OFFSET", "SF_SHIFT", "SF_MASK"] {
        if !slice.contains(anchor) {
            return Err(format!(
                "the extracted SF block does not contain `{anchor}`"
            ));
        }
    }
    Ok(slice.to_string())
}

// =====================================================================================
// ★★★ The USERD-INDEX / chid oracle — the `vchid_from_userd_flags` instrument
// =====================================================================================
//
// See `oracle/userd_chid_oracle.c`. Everything here decides WHICH driver code is compiled,
// and it decides it by reading the driver.
//
// ★★ Why this one compiles FOUR spans out of THREE files. The writer's divisor
// (`NVBIT(DRF_SIZE(_USERD_INDEX_VALUE))`, a flag-field WIDTH) and the reader's multiplier
// (`pGlobalChIDHeap->ownerGranularity`, set from `RM_PAGE_SIZE / userdBar1Size`) are two
// different numbers reached by two unrelated routes. They are equal on GA106. An oracle
// that compiled only one of them would be assuming exactly the thing that could be wrong,
// so both are compiled and the round trip is what says whether they agree.

/// The chip whose HAL bindings this oracle is built with — the same GA106 the shipped
/// `kayfabe_chips::Ga10xArch` is, named here for [`GMMU_ORACLE_CHIP`]'s reason.
const USERD_CHID_ORACLE_CHIP: &str = "GA106";

/// The dispatch table that binds the READER per chip.
const NVOC_KERNEL_CHANNEL_C: &str = "src/nvidia/generated/g_kernel_channel_nvoc.c";

/// The halified entry point that EXTRACTS the two subfields out of the flags word.
const KCHANNEL_ALLOC_HWID_HAL_FUNC: &str = "kchannelAllocHwID";

/// The file holding the recombination and the granularity, both sliced as statement spans.
const KERNEL_FIFO_C: &str = "src/nvidia/src/kernel/gpu/fifo/kernel_fifo.c";

/// The generated header that declares which member holds each halified `kfifo*`.
const NVOC_KERNEL_FIFO_H: &str = "src/nvidia/generated/g_kernel_fifo_nvoc.h";

/// The name of the USERD-isolation predicate the **granularity span itself** calls.
///
/// ★★ `ogkm` is VERSIONED, not the spec — and this is that rule biting in a place that
/// costs a segfault rather than a compile error. The span is byte-identical between the
/// two vendored trees except for one call: `580.159.04` asks
/// `kfifoIsPreAllocatedUserDEnabled`, a plain `static inline` reading a PDB property;
/// `610.43.02` asks `kfifoUserdNeedsIsolation`, which is **halified** and therefore
/// dispatches through a function pointer that is `NULL` in a `calloc`'d `KernelFifo`. The
/// 610 oracle built clean and then crashed on its first case.
///
/// So the predicate is read out of the driver's own text rather than named here, and the
/// caller binds it when — and only when — the driver's own header says it is halified.
fn span_isolation_predicate(span: &str) -> Result<String, String> {
    let decl = "NvBool bIsolationEnabled = ";
    let at = span
        .find(decl)
        .ok_or_else(|| format!("the granularity span has no `{decl}…`"))?;
    let rest = &span[at + decl.len()..];
    let name: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() || !rest[name.len()..].trim_start().starts_with('(') {
        return Err(format!(
            "the granularity span's isolation predicate is not a call: `{}`",
            rest.lines().next().unwrap_or_default()
        ));
    }
    Ok(name)
}

/// Cut the **writer** span out of `kernel_channel.c`, verbatim.
///
/// The span runs from the `numChannelsPerUserd` declaration — the divisor, computed from a
/// flag field's own width — through the `_PAGE_VALUE` `FLD_SET_DRF_NUM` that closes the
/// four-statement write. Between them the file contains nothing else, so the cut is a
/// contiguous run of the original bytes.
///
/// Both ends are located by the file's OWN text, never by line number, for
/// [`extract_interface_slice`]'s reason.
fn extract_userd_writer_span(src: &str) -> Result<&str, String> {
    let decl = "NvU32 numChannelsPerUserd";
    let at = src.find(decl).ok_or_else(|| {
        format!("no `{decl}` in the file — CPU-RM no longer splits the chid here")
    })?;
    let start = src[..at].rfind('\n').map_or(0, |r| r + 1);
    let last = "_CHANNEL_USERD_INDEX_PAGE_VALUE";
    let at2 = src[start..]
        .find(last)
        .map(|r| start + r)
        .ok_or_else(|| format!("no `{last}` after the divisor"))?;
    let end = src[at2..]
        .find(';')
        .map(|r| at2 + r + 1)
        .ok_or("the `_PAGE_VALUE` write has no `;`")?;
    let slice = &src[start..end];
    for anchor in [
        "NVBIT(DRF_SIZE(NVOS04_FLAGS_CHANNEL_USERD_INDEX_VALUE))",
        "_CHANNEL_USERD_INDEX_FIXED, _FALSE",
        "_CHANNEL_USERD_INDEX_PAGE_FIXED, _TRUE",
        "ChID % numChannelsPerUserd",
        "ChID / numChannelsPerUserd",
    ] {
        if !slice.contains(anchor) {
            return Err(format!("the writer span does not contain `{anchor}`"));
        }
    }
    // …and what it must NOT: the span is four statements, not the RPC around them.
    for forbidden in ["NV_RM_RPC_ALLOC_CHANNEL", "internalFlags"] {
        if slice.contains(forbidden) {
            return Err(format!("the writer span reaches `{forbidden}`"));
        }
    }
    Ok(slice)
}

/// Cut the **recombination** span out of `kernel_fifo.c`, verbatim.
///
/// ★★★ The span must be the **non-VF** arm. `kfifoChidMgrAllocChid_IMPL` holds two
/// structurally identical `if (bForceUserdPage)` blocks — one multiplying by
/// `ppVirtualChIDHeap[gfid]->ownerGranularity` (SR-IOV) and one by
/// `pGlobalChIDHeap->ownerGranularity` (everything else, including the GSP-client path we
/// are). Cutting the wrong one would compile, run, and be judged against a heap this
/// harness never populates. So the cut is anchored on `pGlobalChIDHeap` and the slice is
/// **refused** if it mentions the virtual heap at all.
fn extract_chid_recombine_span(src: &str) -> Result<&str, String> {
    let needle = "pChidMgr->pGlobalChIDHeap->ownerGranularity";
    let at = src.find(needle).ok_or_else(|| {
        format!("no `{needle}` — this release no longer recombines the chid from the global heap")
    })?;
    let open = "if (bForceUserdPage)";
    let start = src[..at]
        .rfind(open)
        .ok_or_else(|| format!("no `{open}` before the recombination"))?;
    let start = src[..start].rfind('\n').map_or(0, |r| r + 1);
    let tail = "offsetAlign = internalIdx;";
    let at2 = src[at..]
        .find(tail)
        .map(|r| at + r)
        .ok_or_else(|| format!("no `{tail}` closing the `else if` arm"))?;
    let end = src[at2..]
        .find('}')
        .map(|r| at2 + r + 1)
        .ok_or("the `else if` arm never closes")?;
    let slice = &src[start..end];
    for anchor in [
        "bForceUserdPage",
        "NV_ASSERT_OR_RETURN(!bForceInternalIdx, NV_ERR_INVALID_STATE)",
        "userdPageIdx",
        "ownerGranularity",
        "internalIdx",
        "NVOS32_ALLOC_FLAGS_FIXED_ADDRESS_ALLOCATE",
    ] {
        if !slice.contains(anchor) {
            return Err(format!(
                "the recombination span does not contain `{anchor}`"
            ));
        }
    }
    if slice.contains("ppVirtualChIDHeap") {
        return Err(
            "the recombination span caught the SR-IOV arm (`ppVirtualChIDHeap`), whose heap \
             this harness never populates"
                .to_string(),
        );
    }
    Ok(slice)
}

/// Cut the **granularity** span out of `kernel_fifo.c`, verbatim.
///
/// Three statements: size a USERD through the halified `kfifoGetUserdSizeAlign`, ask
/// whether isolation is on, and hand `RM_PAGE_SIZE / userdBar1Size` to the eheap. The
/// number the whole increment turns on is the third argument of the third statement, and
/// it is never read here — the driver's own `eheapSetOwnerIsolation` stores it and the
/// driver's own recombination reads it back.
fn extract_userd_granularity_span(src: &str) -> Result<&str, String> {
    let first = "kfifoGetUserdSizeAlign_HAL(pKernelFifo, &userdBar1Size, NULL);";
    let at = src
        .find(first)
        .ok_or_else(|| format!("no `{first}` — the USERD size no longer reaches the eheap"))?;
    let start = src[..at].rfind('\n').map_or(0, |r| r + 1);
    let setter = "eheapSetOwnerIsolation";
    let at2 = src[start..]
        .find(setter)
        .map(|r| start + r)
        .ok_or_else(|| format!("no `{setter}` after the USERD size"))?;
    let at3 = src[at2..]
        .find("RM_PAGE_SIZE")
        .map(|r| at2 + r)
        .ok_or("the isolation setter is no longer handed `RM_PAGE_SIZE / …`")?;
    let end = src[at3..]
        .find(';')
        .map(|r| at3 + r + 1)
        .ok_or("the isolation call has no `;`")?;
    let slice = &src[start..end];
    for anchor in [
        "kfifoGetUserdSizeAlign_HAL",
        "userdBar1Size",
        "eheapSetOwnerIsolation",
        "RM_PAGE_SIZE / userdBar1Size",
    ] {
        if !slice.contains(anchor) {
            return Err(format!("the granularity span does not contain `{anchor}`"));
        }
    }
    Ok(slice)
}

/// Read the symbol the driver's own dispatch table binds `func` to for `chip`.
///
/// ★★★ Why this is derived and not listed. `kgmmuFmtInitPteComptagLine` looks like it
/// should follow `kgmmuFmtInitPte` to `_GP10X`; the driver actually binds GA106 to
/// `_TU10X`. That was got wrong by hand on the first attempt at this harness, and the
/// compiler caught it only because the symbol did not exist. A binding that merely moved
/// between two REAL symbols would have compiled and produced a confidently wrong oracle.
///
/// The dispatch table's own `/* ChipHal: … */` comments enumerate the chips each branch
/// serves, so the choice is a lookup, not an inference. The `else` arm with no such
/// comment is the fallback and is used only when no branch names the chip.
fn nvoc_hal_symbol(src: &str, func: &str, chip: &str) -> Result<String, String> {
    let marker = format!("    // {func} -- halified");
    let at = src
        .find(&marker)
        .ok_or_else(|| format!("no `{marker}` block in the dispatch table"))?;
    let block_end = src[at..].find("\n\n").map_or(src.len(), |r| at + r);
    let block = &src[at..block_end];

    let assign = format!("pThis->__{func}__ = &");
    let mut fallback: Option<&str> = None;
    let mut chosen: Option<&str> = None;
    let mut cursor = 0usize;
    let mut prev_end = 0usize;
    while let Some(rel) = block[cursor..].find(&assign) {
        let a = cursor + rel;
        let sym_start = a + assign.len();
        let sym_end = sym_start
            + block[sym_start..]
                .find(';')
                .ok_or_else(|| format!("`{func}`: an assignment with no `;`"))?;
        let symbol = block[sym_start..sym_end].trim();
        // The guard is everything between the previous assignment and this one: the
        // `if`/`else if` and, when the branch is chip-conditional, its ChipHal comment.
        let guard = &block[prev_end..a];
        match guard.rfind("/* ChipHal:") {
            None => fallback = Some(symbol),
            Some(c) => {
                let list = &guard[c + "/* ChipHal:".len()..];
                let list = &list[..list.find("*/").unwrap_or(list.len())];
                if chosen.is_none() && list.split('|').any(|c| c.trim() == chip) {
                    chosen = Some(symbol);
                }
            }
        }
        prev_end = sym_end;
        cursor = sym_end;
    }
    chosen.or(fallback).map(str::to_string).ok_or_else(|| {
        format!("`{func}`: the block binds nothing for `{chip}` and has no fallback")
    })
}

/// ★★ Refuse a chip name the dispatch table has never heard of.
///
/// [`nvoc_hal_symbol`] falls back to the `else` arm when no branch names the chip, which
/// is the right answer for a chip the table really does handle by default — and a silent
/// disaster for a typo. `GA106` misspelled resolves every binding to the Blackwell
/// implementations and the oracle then judges our Ampere decoder against a format for a
/// different generation, green wherever the two happen to agree. So the name must appear
/// in the driver's own enumeration at least once before any lookup is trusted.
fn chip_is_known(src: &str, chip: &str) -> bool {
    let mut from = 0usize;
    while let Some(rel) = src[from..].find("/* ChipHal:") {
        let at = from + rel + "/* ChipHal:".len();
        let end = src[at..].find("*/").map_or(src.len(), |r| at + r);
        if src[at..end].split('|').any(|c| c.trim() == chip) {
            return true;
        }
        from = end;
    }
    false
}

/// The format-abstraction header, and the exact call shapes `gmmu_fmt_oracle.c` is written
/// against.
///
/// ★★ **`ogkm` is VERSIONED, not the spec** — and this is that rule biting in the build.
/// The vendored `580.159.04` tree declares
/// `gmmuFmtPtePhysAddrFld(const GMMU_FMT_PTE *, const GMMU_APERTURE)`; the `610.43.02`
/// tree declares `(const GMMU_FMT_PTE *, const MMU_FMT_LEVEL *, const GMMU_APERTURE,
/// const GMMU_PEER_TYPE)`. That is not the oracle rotting, it is two driver releases with
/// two different abstractions, and the harness can only be written against one of them at
/// a time.
///
/// ⊘ So a tree whose shape does not match is **skipped, loudly, naming the arity** rather
/// than hard-erroring — this is the one case where "present but unbuildable" is a fact
/// about the *tree* and not about the oracle. It is still a SKIP the census counts and the
/// floor protects; it is never silent, and nothing stands in for it.
const GMMU_FMT_HEADER: &str = "src/nvidia/inc/libraries/mmu/gmmu_fmt.h";
const GMMU_EXPECTED_SHAPES: &[(&str, usize)] = &[
    ("gmmuFmtEntryIsPte", 3),
    ("gmmuFmtGetPde", 3),
    ("gmmuFmtPtePhysAddrFld", 2),
    ("gmmuFmtPdePhysAddrFld", 2),
];

/// How many parameters a function is DECLARED with in `header`, or `None` if it is absent.
fn decl_arity(header: &str, func: &str) -> Option<usize> {
    let mut from = 0usize;
    while let Some(rel) = header[from..].find(func) {
        let at = from + rel;
        from = at + func.len();
        let bytes = header.as_bytes();
        if at > 0 && (bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_') {
            continue;
        }
        let mut i = from;
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'(' {
            continue;
        }
        let mut depth = 0usize;
        let mut j = i;
        while j < bytes.len() {
            match bytes[j] {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        if j >= bytes.len() {
            continue;
        }
        let params = &header[i + 1..j];
        // Only a DECLARATION (`…);`) — a definition or a call site says nothing about the
        // interface the harness compiles against.
        let tail = header[j + 1..].trim_start();
        if !tail.starts_with(';') {
            continue;
        }
        if params.trim().is_empty() || params.trim() == "void" {
            return Some(0);
        }
        // Commas at depth 0 only.
        let mut n = 1usize;
        let mut d = 0i32;
        for c in params.chars() {
            match c {
                '(' | '[' => d += 1,
                ')' | ']' => d -= 1,
                ',' if d == 0 => n += 1,
                _ => {}
            }
        }
        return Some(n);
    }
    None
}

/// Every `kgmmu*` symbol a source file declares `extern` and therefore expects the linker
/// to resolve from another file.
///
/// ★ Used to close the source set TRANSITIVELY rather than by hand: the sliced
/// `kgmmuFmtFamiliesInit_GV100` declares `extern … kgmmuFmtFamiliesInit_GM200` in its own
/// body, so the file that defines GM200 is pulled in because the driver's own text asked
/// for it. A hand-written companion list would be one more thing to forget.
fn declared_externs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.trim_start().strip_prefix("extern ") else {
            continue;
        };
        let Some(paren) = rest.find('(') else {
            continue;
        };
        let name: String = rest[..paren]
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if name.starts_with("kgmmu") && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// Read the symbol a single-implementation entry point is `#define`d to.
fn nvoc_define_symbol(hdr: &str, func: &str) -> Result<String, String> {
    let needle = format!("#define {func}(");
    let at = hdr
        .find(&needle)
        .ok_or_else(|| format!("no `{needle}` in the generated header"))?;
    let line = &hdr[at..hdr[at..].find('\n').map_or(hdr.len(), |r| at + r)];
    let rhs = line
        .rfind(')')
        .and_then(|_| line.split_once(')'))
        .map(|(_, r)| r)
        .ok_or_else(|| format!("`{func}`: malformed define `{line}`"))?;
    let rhs = rhs.trim();
    let sym: String = rhs
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if sym.is_empty() || !sym.starts_with(func) {
        return Err(format!(
            "`{func}`: define does not name an implementation: `{line}`"
        ));
    }
    Ok(sym)
}

/// Byte offset of the occurrence of `symbol` that is a **definition**, if there is one.
///
/// ★ Structural, not textual. The first attempt at this looked for the symbol at the start
/// of a line — true of `NV_STATUS\nkgmmuFmtFamiliesInit_GV100(` and false of
/// `void kgmmuFmtInitLevels_GA10X(`, which is the very function this oracle exists for, so
/// it found nothing at all. The rule here is the one that actually distinguishes the three
/// cases: an occurrence is a definition when it is a whole word, is followed by a
/// parenthesised list, and that list is followed by `{`. A *declaration* is followed by
/// `;`; a *call* is followed by `;`, `)` or an operator.
fn find_definition_offset(text: &str, symbol: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let is_word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut from = 0usize;
    while let Some(rel) = text[from..].find(symbol) {
        let at = from + rel;
        from = at + symbol.len();
        // Whole word?
        if at > 0 && is_word(bytes[at - 1]) {
            continue;
        }
        let mut i = from;
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'(' {
            continue;
        }
        // Walk to the matching close paren.
        let mut depth = 0usize;
        let mut j = i;
        while j < bytes.len() {
            match bytes[j] {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        if j >= bytes.len() {
            continue;
        }
        // …then the next non-space byte must open a body.
        let mut k = j + 1;
        while k < bytes.len() && (bytes[k] as char).is_whitespace() {
            k += 1;
        }
        if k < bytes.len() && bytes[k] == b'{' {
            return Some(at);
        }
    }
    None
}

/// Find the file that DEFINES `symbol`, under `root`, recursively.
fn find_definition_file(root: &Path, symbol: &str) -> Result<PathBuf, String> {
    let mut hits = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "c") {
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                if find_definition_offset(&text, symbol).is_some() {
                    hits.push(path);
                }
            }
        }
    }
    match hits.len() {
        0 => Err(format!(
            "no file under {} defines `{symbol}` at the start of a line",
            root.display()
        )),
        1 => Ok(hits.remove(0)),
        _ => Err(format!(
            "`{symbol}` is defined in more than one file: {hits:?} — the oracle cannot \
             tell which one the driver links"
        )),
    }
}

/// Cut one function definition, byte-for-byte, out of a file that also holds code the
/// oracle must not compile. Same construction as [`extract_interface_slice`].
fn extract_function(src: &str, symbol: &str) -> Result<String, String> {
    let at = find_definition_offset(src, symbol)
        .ok_or_else(|| format!("no definition of `{symbol}`"))?;
    // Start of the symbol's own line…
    let line_start = src[..at].rfind('\n').map_or(0, |r| r + 1);
    // …and, when the return type sits alone on the line above (`NV_STATUS\nname(…)`),
    // that line too. A previous line carrying `;`, `}`, `*/` or nothing is the END of
    // something else and is not part of this definition.
    let start = if src[line_start..at].trim().is_empty() {
        let prev_start = src[..line_start.saturating_sub(1)]
            .rfind('\n')
            .map_or(0, |r| r + 1);
        let prev = src[prev_start..line_start].trim();
        if !prev.is_empty()
            && !prev.ends_with(';')
            && !prev.ends_with('}')
            && !prev.ends_with("*/")
            && !prev.starts_with("//")
        {
            prev_start
        } else {
            line_start
        }
    } else {
        line_start
    };
    let rel = src[at..]
        .find("\n{\n")
        .ok_or_else(|| format!("`{symbol}` has no column-0 body"))?;
    let body = at + rel + 1;
    let mut depth = 0usize;
    let mut end = None;
    for (i, ch) in src[body..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(body + i + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end.ok_or_else(|| format!("`{symbol}`'s body never closes"))?;
    let slice = &src[start..end];
    // A cut that landed on the wrong region still compiles; check the anchor it must have.
    if !slice.contains(symbol) {
        return Err(format!("the extracted slice does not contain `{symbol}`"));
    }
    Ok(slice.to_string())
}

fn main() {
    println!("cargo:rerun-if-changed=oracle/vbios_oracle.c");
    println!("cargo:rerun-if-changed=oracle/gmmu_fmt_oracle.c");
    println!("cargo:rerun-if-changed=oracle/worksubmit_token_oracle.c");
    println!("cargo:rerun-if-changed=oracle/pushbuffer_abi_oracle.c");
    println!("cargo:rerun-if-changed=oracle/userd_chid_oracle.c");
    for (env_var, _, _) in TREES {
        println!("cargo:rerun-if-env-changed={env_var}");
    }

    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let harness = Path::new("oracle/vbios_oracle.c")
        .canonicalize()
        .expect("oracle/vbios_oracle.c is part of this crate");

    for (env_var, default, out_env) in TREES {
        let root = PathBuf::from(std::env::var(env_var).unwrap_or_else(|_| (*default).to_string()));
        let tag = out_env.rsplit('_').next().unwrap_or("unknown");

        // The GMMU format oracle is built from a DIFFERENT set of driver files, so it gets
        // its own presence check: a tree that can serve one and not the other must skip
        // exactly one of them, and say which.
        build_gmmu_oracle(&cc, &out_dir, &root, tag, env_var);
        build_token_oracle(&cc, &out_dir, &root, tag, env_var);
        build_pushbuffer_oracle(&cc, &out_dir, &root, tag, env_var);
        build_userd_chid_oracle(&cc, &out_dir, &root, tag, env_var);

        if !root.join(VBIOS_TU102_C).is_file()
            || !root.join(FWSEC_C).is_file()
            || !root.join(FRTS_TU102_C).is_file()
        {
            println!(
                "cargo:warning=VBIOS ORACLE SKIPPED: no open-kernel-modules tree at {} \
                 (set {env_var} to relocate). The tests that run NVIDIA's REAL VBIOS parser \
                 over our generated image will announce themselves as SKIPPED and assert \
                 NOTHING. Nothing is vendored into this repository to stand in for it.",
                root.display()
            );
            continue;
        }

        // The interface-walk slice, cut fresh from THIS tree and written beside
        // the binary it is compiled into — one per tag, never shared, so the two
        // tags cannot silently be judged against one tree's driver code.
        let frts_src = std::fs::read_to_string(root.join(FRTS_TU102_C))
            .unwrap_or_else(|e| panic!("VBIOS ORACLE: cannot read {FRTS_TU102_C}: {e}"));
        let slice = extract_interface_slice(&frts_src).unwrap_or_else(|e| {
            panic!(
                "VBIOS ORACLE: could not extract the FWSEC interface walk from {}: {e}.\n\
                 The tree IS present, so this is the ORACLE having rotted against a driver \
                 whose text moved — degrading it to a skip is how a gate stops being able \
                 to fire. Re-read the file and fix the anchors.",
                root.join(FRTS_TU102_C).display()
            )
        });
        // ★ A per-tag SUBDIRECTORY with the SAME basename, not a tagged filename.
        // The driver's own `NV_PRINTF` puts `__FILE__`'s basename in every
        // diagnostic, and the oracle reports those verbatim — so a name carrying
        // the tag makes the 580 and 610 runs emit *different text* for identical
        // behaviour, and `the_two_vendored_ogkm_tags_agree_on_our_image` reports a
        // disagreement that is entirely an artefact of this build script. Seen
        // happening while planting a bite; the clean image emits no diagnostics at
        // all, so nothing would have caught it until a real failure did.
        let slice_dir = out_dir.join(tag);
        std::fs::create_dir_all(&slice_dir).expect("slice dir");
        let slice_path = slice_dir.join("frts_interface_slice.inc");
        std::fs::write(&slice_path, slice).expect("write the interface slice");

        let bin = out_dir.join(format!("vbios_oracle_{tag}"));
        let mut cmd = Command::new(&cc);
        cmd.arg("-std=gnu11").arg("-O1").arg("-g");
        // A *test* harness: leave the assertion machinery on and do not let the compiler
        // assume the parser is well-behaved. `-fno-strict-aliasing` matters — the parser's
        // `s_romImgReadGeneric` reads a `union { NvU32 word[2]; NvU8 byte[8]; }` through
        // both members, which the kernel build also compiles with aliasing off.
        cmd.arg("-fno-strict-aliasing");
        cmd.arg("-o").arg(&bin);
        cmd.arg(format!(
            "-DOGKM_VBIOS_TU102_C=\"{}\"",
            root.join(VBIOS_TU102_C).display()
        ));
        cmd.arg(format!(
            "-DOGKM_FWSEC_C=\"{}\"",
            root.join(FWSEC_C).display()
        ));
        cmd.arg(format!(
            "-DOGKM_FRTS_INTERFACE_SLICE=\"{}\"",
            slice_path.display()
        ));
        for d in DEFINES {
            cmd.arg(format!("-D{d}"));
        }
        cmd.arg("-include")
            .arg(root.join("src/common/sdk/nvidia/inc/cpuopsys.h"));
        for (prefix, sub) in INCLUDES {
            cmd.arg("-I").arg(root.join(prefix).join(sub));
        }
        cmd.arg(&harness);

        let out = cmd
            .output()
            .unwrap_or_else(|e| panic!("VBIOS ORACLE: could not run the C compiler `{cc}`: {e}"));
        assert!(
            out.status.success(),
            "VBIOS ORACLE FAILED TO BUILD against {}.\n\
             The tree IS present, so this is NOT a machine without the oracle — it is the \
             oracle having rotted, and degrading it to a skip is how a gate stops being able \
             to fire. Compiler output:\n{}\n{}",
            root.display(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );

        println!("cargo:rustc-env={out_env}={}", bin.display());
    }
}

/// Build the GMMU format oracle for one vendored tree.
///
/// Failure polarity matches the VBIOS oracle exactly and for the same reason: **absent
/// tree → skip with a warning; present tree that will not build → hard error.** An oracle
/// that degrades to a skip is how a gate stops being able to fire.
fn build_gmmu_oracle(cc: &str, out_dir: &Path, root: &Path, tag: &str, env_var: &str) {
    let out_env = format!("KAYFABE_GMMU_ORACLE_{tag}");
    let dispatch = root.join(NVOC_KERN_GMMU_C);
    let header = root.join(NVOC_KERN_GMMU_H);
    if !dispatch.is_file() || !header.is_file() || !root.join(GMMU_IMPL_ROOT).is_dir() {
        println!(
            "cargo:warning=GMMU ORACLE SKIPPED: no open-kernel-modules tree at {} \
             (set {env_var} to relocate). The tests that judge our GA10x page-table decoder \
             against NVIDIA's OWN encoder will announce themselves as SKIPPED and assert \
             NOTHING. Nothing is vendored into this repository to stand in for it.",
            root.display()
        );
        return;
    }

    let fail = |what: &str| -> ! {
        panic!(
            "GMMU ORACLE: {what}\n\
             The tree at {} IS present, so this is NOT a machine without the oracle — it is \
             the oracle having rotted against a driver whose text moved, and degrading it to \
             a skip is how a gate stops being able to fire.",
            root.display()
        )
    };

    let dispatch_src = std::fs::read_to_string(&dispatch)
        .unwrap_or_else(|e| fail(&format!("cannot read {NVOC_KERN_GMMU_C}: {e}")));
    let header_src = std::fs::read_to_string(&header)
        .unwrap_or_else(|e| fail(&format!("cannot read {NVOC_KERN_GMMU_H}: {e}")));

    // Does this tree's format abstraction have the shape the harness is written against?
    let fmt_header = std::fs::read_to_string(root.join(GMMU_FMT_HEADER))
        .unwrap_or_else(|e| fail(&format!("cannot read {GMMU_FMT_HEADER}: {e}")));
    for (func, want) in GMMU_EXPECTED_SHAPES {
        let got = decl_arity(&fmt_header, func);
        if got != Some(*want) {
            // ★ The reason travels to the TEST, not just to the build log. A skip nobody
            // can read at test time is indistinguishable from a tree that was never there.
            println!(
                "cargo:rustc-env=KAYFABE_GMMU_ORACLE_SKIP_{tag}=`{func}` is declared with {} \
                 parameter(s) in {}; oracle/gmmu_fmt_oracle.c is written against {want}",
                got.map_or("no".to_string(), |n| n.to_string()),
                root.join(GMMU_FMT_HEADER).display(),
            );
            println!(
                "cargo:warning=GMMU ORACLE SKIPPED for the tree at {}: its `{func}` is \
                 declared with {} parameters, and oracle/gmmu_fmt_oracle.c is written \
                 against {want}. This is NOT the oracle rotting — it is a driver release \
                 whose GMMU abstraction differs (580.159.04 and 610.43.02 genuinely \
                 disagree here). The tests judged against THIS tree announce themselves as \
                 SKIPPED and assert NOTHING; nothing is substituted for them.",
                root.display(),
                got.map_or("no".to_string(), |n| n.to_string()),
            );
            return;
        }
    }

    if !chip_is_known(&dispatch_src, GMMU_ORACLE_CHIP) {
        fail(&format!(
            "`{GMMU_ORACLE_CHIP}` appears in no `/* ChipHal: … */` list in the dispatch \
             table, so every binding below would silently take the `else` arm — which is a \
             DIFFERENT GENERATION's format, green wherever the two happen to agree."
        ));
    }

    // 1. Ask the driver which implementation this chip gets, for every entry point.
    let mut bindings: Vec<(&str, String)> = Vec::new();
    for (func, macro_name) in GMMU_HALIFIED {
        let sym =
            nvoc_hal_symbol(&dispatch_src, func, GMMU_ORACLE_CHIP).unwrap_or_else(|e| fail(&e));
        bindings.push((macro_name, sym));
    }
    for (func, macro_name) in GMMU_DEFINED {
        let sym = nvoc_define_symbol(&header_src, func).unwrap_or_else(|e| fail(&e));
        bindings.push((macro_name, sym));
    }

    // 2. Find the file that defines each of those symbols, classify it, and close the set
    //    over the `extern`s those files declare.
    let impl_root = root.join(GMMU_IMPL_ROOT);
    let slice_dir = out_dir.join(tag);
    std::fs::create_dir_all(&slice_dir).expect("slice dir");
    let mut sources: Vec<PathBuf> = Vec::new();
    let mut pending: Vec<(String, String)> = bindings
        .iter()
        .map(|(m, s)| ((*m).to_string(), s.clone()))
        .collect();
    let mut resolved: Vec<String> = Vec::new();
    while let Some((why, sym)) = pending.pop() {
        if resolved.contains(&sym) {
            continue;
        }
        resolved.push(sym.clone());
        let file = find_definition_file(&impl_root, &sym)
            .unwrap_or_else(|e| fail(&format!("binding {why}={sym}: {e}")));
        let base = file
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|e| fail(&format!("cannot read {}: {e}", file.display())));
        let added = if GMMU_WHOLE_FILE_OK.contains(&base.as_str()) {
            file.clone()
        } else if GMMU_SLICE_ONLY.contains(&base.as_str()) {
            // Cut the one function out, byte-for-byte, and give it a short include
            // prologue. The prologue is OURS and is the whole of what this harness adds;
            // everything after it is the driver's own text, unmodified.
            let body = extract_function(&text, &sym)
                .unwrap_or_else(|e| fail(&format!("slicing {sym} out of {}: {e}", file.display())));
            let path = slice_dir.join(format!("{sym}_slice.c"));
            let prologue = format!(
                "/* GENERATED by tests/build.rs — a byte-for-byte slice of\n\
                 \x20* {}\n\
                 \x20* (NVIDIA, SPDX-License-Identifier: MIT). Only the two directives below \
                 are ours.\n\
                 \x20*/\n\
                 #define NVOC_KERN_GMMU_H_PRIVATE_ACCESS_ALLOWED\n\
                 #include \"gpu/mmu/kern_gmmu.h\"\n\n",
                file.display()
            );
            std::fs::write(&path, format!("{prologue}{body}\n")).expect("write the gmmu slice");
            // A slice links against whatever it declares `extern`.
            for dep in declared_externs(&body) {
                pending.push((format!("an `extern` in {sym}"), dep));
            }
            path
        } else {
            fail(&format!(
                "the driver binds {why} to `{sym}`, which is defined in {} — a file that is \
                 on neither GMMU_WHOLE_FILE_OK nor GMMU_SLICE_ONLY. READ IT before adding \
                 it: the whole reason this oracle is cheap is that every function in it is \
                 pure bit-packing with no OBJGPU entanglement, and a file that needs the \
                 object model mocked must be SLICED or the target must shrink. A smaller \
                 true oracle beats a larger fake one.",
                file.display()
            ));
        };
        if !sources.contains(&added) {
            sources.push(added);
        }
    }
    for support in GMMU_SUPPORT_C {
        let p = root.join(support);
        if !p.is_file() {
            fail(&format!("{support} is missing from the tree"));
        }
        sources.push(p);
    }

    // 3. Compile.
    let harness = Path::new("oracle/gmmu_fmt_oracle.c")
        .canonicalize()
        .expect("oracle/gmmu_fmt_oracle.c is part of this crate");
    let bin = out_dir.join(format!("gmmu_fmt_oracle_{tag}"));
    let mut cmd = Command::new(cc);
    cmd.arg("-std=gnu11").arg("-O1").arg("-g");
    cmd.arg("-fno-strict-aliasing");
    cmd.arg("-o").arg(&bin);
    for (macro_name, sym) in &bindings {
        cmd.arg(format!("-D{macro_name}={sym}"));
    }
    cmd.arg(format!("-DOGKM_CHIP_NAME=\"{GMMU_ORACLE_CHIP}\""));
    for d in DEFINES {
        cmd.arg(format!("-D{d}"));
    }
    cmd.arg("-include")
        .arg(root.join("src/common/sdk/nvidia/inc/cpuopsys.h"));
    for (prefix, sub) in INCLUDES {
        cmd.arg("-I").arg(root.join(prefix).join(sub));
    }
    cmd.arg(&harness);
    for s in &sources {
        cmd.arg(s);
    }

    let out = cmd
        .output()
        .unwrap_or_else(|e| fail(&format!("could not run the C compiler `{cc}`: {e}")));
    if !out.status.success() {
        fail(&format!(
            "the harness did not compile. Compiler output:\n{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        ));
    }

    println!("cargo:rustc-env={out_env}={}", bin.display());
}

/// Build the **work-submit token oracle** for one vendored tree.
///
/// Failure polarity is the house rule, for the third time and the same reason: an absent
/// tree is a **skip** with a `cargo:warning=`; a present tree that will not build is a
/// **hard error**. An oracle that degrades to a skip is how a gate stops being able to
/// fire — and this one guards the increment whose wrong answer is silent, so a skip that
/// nobody noticed would be worse here than anywhere else in the workspace.
fn build_token_oracle(cc: &str, out_dir: &Path, root: &Path, tag: &str, env_var: &str) {
    let out_env = format!("KAYFABE_TOKEN_ORACLE_{tag}");
    let dispatch = root.join(NVOC_KERNEL_FIFO_C);
    if !dispatch.is_file()
        || !root.join(TOKEN_IMPL_ROOT).is_dir()
        || !root.join(TOKEN_KCHANNEL_C).is_file()
    {
        println!(
            "cargo:warning=TOKEN ORACLE SKIPPED: no open-kernel-modules tree at {} \
             (set {env_var} to relocate). The test that judges `Ga10xArch::decode_doorbell` \
             against NVIDIA's OWN work-submit-token encoder will announce itself as SKIPPED \
             and assert NOTHING. Nothing is vendored here to stand in for it.",
            root.display()
        );
        return;
    }

    let dispatch_src = std::fs::read_to_string(&dispatch)
        .unwrap_or_else(|e| panic!("TOKEN ORACLE: cannot read {}: {e}", dispatch.display()));

    // ★★ Refuse a chip name this table has never heard of — `nvoc_hal_symbol` would
    // otherwise fall through to the `else` arm and bind the BLACKWELL encoder, judging our
    // Ampere decoder against a different generation's field layout. That failure mode is
    // not hypothetical; it is the one `chip_is_known` was written for on the GMMU oracle.
    assert!(
        chip_is_known(&dispatch_src, TOKEN_ORACLE_CHIP),
        "TOKEN ORACLE: `{TOKEN_ORACLE_CHIP}` appears in no `/* ChipHal: … */` comment in \
         {} — the name is wrong, and every binding would silently resolve to the fallback.",
        dispatch.display()
    );
    let symbol = nvoc_hal_symbol(&dispatch_src, TOKEN_HAL_FUNC, TOKEN_ORACLE_CHIP)
        .unwrap_or_else(|e| panic!("TOKEN ORACLE: {e}"));

    let impl_file = find_definition_file(&root.join(TOKEN_IMPL_ROOT), &symbol)
        .unwrap_or_else(|e| panic!("TOKEN ORACLE: {e}"));
    let impl_src = std::fs::read_to_string(&impl_file)
        .unwrap_or_else(|e| panic!("TOKEN ORACLE: cannot read {}: {e}", impl_file.display()));
    let token_fn = extract_function(&impl_src, &symbol)
        .unwrap_or_else(|e| panic!("TOKEN ORACLE: {} — {e}", impl_file.display()));

    // ★★★ The slice carries the IMPL FILE'S OWN `#include` lines, byte for byte, and not
    // a list of headers this script chose. `NV_CTRL_VF_DOORBELL_VECTOR` and `_RUNLIST_ID`
    // — the two bit positions the whole increment is about — reach the compiler ONLY
    // through those lines. Naming `published/ampere/ga100/dev_ctrl.h` here instead would
    // have put the chip-generation choice back into our hands, which is the transcription
    // this oracle exists to remove.
    let includes: String = impl_src
        .lines()
        .filter(|l| l.trim_start().starts_with("#include"))
        .map(|l| format!("{l}\n"))
        .collect();
    assert!(
        includes.contains("dev_ctrl.h"),
        "TOKEN ORACLE: {} no longer includes a `dev_ctrl.h`, so the slice would not see \
         NV_CTRL_VF_DOORBELL_* at all and the encoder could not compile against the \
         driver's own field positions.",
        impl_file.display()
    );

    let mut kchannel_slices = String::new();
    let kchannel_src = std::fs::read_to_string(root.join(TOKEN_KCHANNEL_C))
        .unwrap_or_else(|e| panic!("TOKEN ORACLE: cannot read {TOKEN_KCHANNEL_C}: {e}"));
    for f in TOKEN_KCHANNEL_FNS {
        kchannel_slices.push_str(
            &extract_function(&kchannel_src, f)
                .unwrap_or_else(|e| panic!("TOKEN ORACLE: {TOKEN_KCHANNEL_C} — {e}")),
        );
        kchannel_slices.push('\n');
    }

    // The anchors the compiled driver code MUST still contain. A slice that landed on the
    // wrong region, or a release that moved the encoding out of this function, is a build
    // error naming the anchor rather than a green test.
    let all = format!("{token_fn}\n{kchannel_slices}");
    for anchor in TOKEN_SLICE_ANCHORS {
        assert!(
            all.contains(anchor),
            "TOKEN ORACLE: the driver code sliced for `{symbol}` does not contain \
             `{anchor}`. Either the cut is wrong or this release no longer encodes the \
             token here — either way the oracle must not be believed."
        );
    }
    // …and what it must NOT: the whole point of slicing rather than compiling the file.
    for forbidden in ["kfifoUpdateUsermodeDoorbell", "kfifoRunlistGetBaseShift"] {
        assert!(
            !token_fn.contains(forbidden),
            "TOKEN ORACLE: the slice reaches `{forbidden}` — it caught more than the \
             encoder."
        );
    }

    // Per-tag subdirectory with a stable basename, for `build_gmmu_oracle`'s reason: the
    // driver's own `NV_PRINTF` puts `__FILE__`'s basename in every diagnostic.
    let dir = out_dir.join(tag);
    std::fs::create_dir_all(&dir).expect("slice dir");
    let token_path = dir.join("worksubmit_token_slice.inc");
    std::fs::write(&token_path, format!("{includes}\n{token_fn}\n"))
        .expect("write the token slice");
    let kchannel_path = dir.join("kchannel_runlist_slice.inc");
    std::fs::write(&kchannel_path, &kchannel_slices).expect("write the kchannel slice");

    let harness = Path::new("oracle/worksubmit_token_oracle.c")
        .canonicalize()
        .expect("oracle/worksubmit_token_oracle.c is part of this crate");
    let bin = out_dir.join(format!("worksubmit_token_oracle_{tag}"));
    let mut cmd = Command::new(cc);
    cmd.arg("-std=gnu11").arg("-O1").arg("-g");
    cmd.arg("-fno-strict-aliasing");
    cmd.arg("-o").arg(&bin);
    cmd.arg(format!(
        "-DOGKM_WORKSUBMIT_TOKEN_SLICE=\"{}\"",
        token_path.display()
    ));
    cmd.arg(format!(
        "-DOGKM_KCHANNEL_RUNLIST_SLICE=\"{}\"",
        kchannel_path.display()
    ));
    cmd.arg(format!("-DOGKM_WORKSUBMIT_TOKEN_FN={symbol}"));
    cmd.arg(format!("-DOGKM_WORKSUBMIT_TOKEN_FN_NAME=\"{symbol}\""));
    cmd.arg(format!(
        "-DOGKM_WORKSUBMIT_TOKEN_CHIP=\"{TOKEN_ORACLE_CHIP}\""
    ));
    for d in DEFINES {
        cmd.arg(format!("-D{d}"));
    }
    cmd.arg("-include")
        .arg(root.join("src/common/sdk/nvidia/inc/cpuopsys.h"));
    for (prefix, sub) in INCLUDES {
        cmd.arg("-I").arg(root.join(prefix).join(sub));
    }
    cmd.arg(&harness);

    let out = cmd
        .output()
        .unwrap_or_else(|e| panic!("TOKEN ORACLE: could not run the C compiler `{cc}`: {e}"));
    assert!(
        out.status.success(),
        "TOKEN ORACLE FAILED TO BUILD against {}.\n\
         The tree IS present, so this is NOT a machine without the oracle — it is the \
         oracle having rotted, and degrading it to a skip is how a gate stops being able \
         to fire. Compiler output:\n{}\n{}",
        root.display(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    println!("cargo:rustc-env={out_env}={}", bin.display());
}

/// Build the **pushbuffer / USERD ABI oracle** for one vendored tree — increment `E4`.
///
/// Failure polarity is the house rule, for the fourth time and the same reason: an absent
/// tree is a **skip** with a `cargo:warning=`; a present tree that will not build is a
/// **hard error**. An oracle that degrades to a skip is how a gate stops being able to
/// fire.
fn build_pushbuffer_oracle(cc: &str, out_dir: &Path, root: &Path, tag: &str, env_var: &str) {
    let out_env = format!("KAYFABE_PUSHBUFFER_ORACLE_{tag}");
    let dispatch = root.join(NVOC_KERNEL_FIFO_C);
    let sf_header = root.join(PUSHBUFFER_SF_HEADER);
    let missing: Vec<&str> = PUSHBUFFER_CLASS_HEADERS
        .iter()
        .copied()
        .filter(|h| !root.join(h).is_file())
        .collect();
    if !dispatch.is_file()
        || !sf_header.is_file()
        || !root.join(TOKEN_IMPL_ROOT).is_dir()
        || !missing.is_empty()
    {
        println!(
            "cargo:warning=PUSHBUFFER ORACLE SKIPPED: no usable open-kernel-modules tree at \
             {} (set {env_var} to relocate; missing class headers: {missing:?}). The tests \
             that judge `Ga10xUserd` / `Ga10xPushbuffer` against NVIDIA's OWN field \
             definitions announce themselves as SKIPPED and assert NOTHING. Nothing is \
             vendored here to stand in for them.",
            root.display()
        );
        return;
    }

    let fail = |what: &str| -> ! {
        panic!(
            "PUSHBUFFER ORACLE: {what}\n\
             The tree at {} IS present, so this is NOT a machine without the oracle — it is \
             the oracle having rotted against a driver whose text moved, and degrading it to \
             a skip is how a gate stops being able to fire.",
            root.display()
        )
    };

    let dispatch_src = std::fs::read_to_string(&dispatch)
        .unwrap_or_else(|e| fail(&format!("cannot read {}: {e}", dispatch.display())));

    // ★★ Refuse a chip name this table has never heard of — `nvoc_hal_symbol` would
    // otherwise fall through to the `else` arm silently. Here that arm is ALSO the right
    // answer for GA106, which makes the check more important rather than less: without it,
    // a typo'd chip would produce the identical binding and the oracle would look healthy
    // for a chip that does not exist.
    if !chip_is_known(&dispatch_src, PUSHBUFFER_ORACLE_CHIP) {
        fail(&format!(
            "`{PUSHBUFFER_ORACLE_CHIP}` appears in no `/* ChipHal: … */` comment in {} — \
             the name is wrong.",
            dispatch.display()
        ));
    }
    let symbol = nvoc_hal_symbol(&dispatch_src, USERD_SIZE_HAL_FUNC, PUSHBUFFER_ORACLE_CHIP)
        .unwrap_or_else(|e| fail(&e));

    let impl_file =
        find_definition_file(&root.join(TOKEN_IMPL_ROOT), &symbol).unwrap_or_else(|e| fail(&e));
    let impl_src = std::fs::read_to_string(&impl_file)
        .unwrap_or_else(|e| fail(&format!("cannot read {}: {e}", impl_file.display())));
    let userd_fn = extract_function(&impl_src, &symbol)
        .unwrap_or_else(|e| fail(&format!("{} — {e}", impl_file.display())));

    for anchor in USERD_SLICE_ANCHORS {
        if !userd_fn.contains(anchor) {
            fail(&format!(
                "the driver code sliced for `{symbol}` does not contain `{anchor}`. Either \
                 the cut is wrong or this release no longer sizes USERD here — either way \
                 the oracle must not be believed."
            ));
        }
    }

    // ★★★ The slice carries the IMPL FILE'S OWN `published/…` include lines, byte for
    // byte, and not a header this script chose. `NV_RAMUSERD_BASE_SHIFT` — and, through
    // the same header, `NV_RAMUSERD_GP_GET`/`_GP_PUT` — reach the compiler ONLY through
    // those lines. Naming `published/maxwell/gm107/dev_ram.h` here instead would put the
    // chip-generation choice back into our hands, which is the transcription this oracle
    // exists to remove. (It is Maxwell's, on an Ampere chip; see USERD_SIZE_HAL_FUNC.)
    let published: String = impl_src
        .lines()
        .filter(|l| l.trim_start().starts_with("#include") && l.contains("published/"))
        .map(|l| format!("{l}\n"))
        .collect();
    if !published.contains("dev_ram.h") {
        fail(&format!(
            "{} no longer includes a `published/…/dev_ram.h`, so the slice would not see \
             NV_RAMUSERD_* at all and USERD could not be sized against the driver's own \
             field definitions.",
            impl_file.display()
        ));
    }

    let sf_src = std::fs::read_to_string(&sf_header)
        .unwrap_or_else(|e| fail(&format!("cannot read {PUSHBUFFER_SF_HEADER}: {e}")));
    let sf_slice = extract_sf_accessors(&sf_src).unwrap_or_else(|e| {
        fail(&format!(
            "slicing the SF accessors out of {PUSHBUFFER_SF_HEADER}: {e}"
        ))
    });

    // Per-tag subdirectory with a stable basename, for `build_gmmu_oracle`'s reason.
    let dir = out_dir.join(tag);
    std::fs::create_dir_all(&dir).expect("slice dir");
    let sf_path = dir.join("sf_accessor_slice.inc");
    std::fs::write(&sf_path, &sf_slice).expect("write the SF slice");
    let userd_path = dir.join("userd_size_slice.inc");
    std::fs::write(&userd_path, format!("{published}\n{userd_fn}\n"))
        .expect("write the USERD slice");

    let harness = Path::new("oracle/pushbuffer_abi_oracle.c")
        .canonicalize()
        .expect("oracle/pushbuffer_abi_oracle.c is part of this crate");
    let bin = out_dir.join(format!("pushbuffer_abi_oracle_{tag}"));
    let mut cmd = Command::new(cc);
    cmd.arg("-std=gnu11").arg("-O1").arg("-g");
    cmd.arg("-fno-strict-aliasing");
    cmd.arg("-o").arg(&bin);
    cmd.arg(format!(
        "-DOGKM_SF_ACCESSOR_SLICE=\"{}\"",
        sf_path.display()
    ));
    cmd.arg(format!(
        "-DOGKM_USERD_SIZE_SLICE=\"{}\"",
        userd_path.display()
    ));
    cmd.arg(format!("-DOGKM_USERD_SIZE_FN={symbol}"));
    cmd.arg(format!("-DOGKM_USERD_SIZE_FN_NAME=\"{symbol}\""));
    cmd.arg(format!(
        "-DOGKM_PUSHBUFFER_CHIP=\"{PUSHBUFFER_ORACLE_CHIP}\""
    ));
    for d in DEFINES {
        cmd.arg(format!("-D{d}"));
    }
    cmd.arg("-include")
        .arg(root.join("src/common/sdk/nvidia/inc/cpuopsys.h"));
    for (prefix, sub) in INCLUDES {
        cmd.arg("-I").arg(root.join(prefix).join(sub));
    }
    cmd.arg(&harness);

    let out = cmd
        .output()
        .unwrap_or_else(|e| fail(&format!("could not run the C compiler `{cc}`: {e}")));
    if !out.status.success() {
        fail(&format!(
            "the harness did not compile. Compiler output:\n{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        ));
    }

    println!("cargo:rustc-env={out_env}={}", bin.display());
}

/// Build the **USERD-index / chid oracle** for one vendored tree — the instrument for
/// `Arch::vchid_from_userd_flags`.
///
/// Failure polarity is the house rule, for the fifth time and the same reason: an absent
/// tree is a **skip** with a `cargo:warning=`; a present tree that will not build is a
/// **hard error**. An oracle that degrades to a skip is how a gate stops being able to
/// fire — and this one guards a decode whose wrong answer is silent (it routes a guest's
/// channel to *another channel*, and on the Mode-2 path we are the GSP, so there is no
/// second party to notice).
fn build_userd_chid_oracle(cc: &str, out_dir: &Path, root: &Path, tag: &str, env_var: &str) {
    let out_env = format!("KAYFABE_USERD_CHID_ORACLE_{tag}");
    let chan_dispatch = root.join(NVOC_KERNEL_CHANNEL_C);
    let fifo_dispatch = root.join(NVOC_KERNEL_FIFO_C);
    // The driver's own containers, compiled WHOLE: `ownerGranularity` must be stored by
    // NVIDIA's `eheapSetOwnerIsolation` and read back by NVIDIA's recombination, with
    // nothing of ours in between. `eheap_old.c` links against `btree.c`.
    let eheap = root.join("src/nvidia/src/libraries/containers/eheap/eheap_old.c");
    let btree = root.join("src/nvidia/src/libraries/containers/btree/btree.c");
    if !chan_dispatch.is_file()
        || !fifo_dispatch.is_file()
        || !root.join(TOKEN_IMPL_ROOT).is_dir()
        || !root.join(TOKEN_KCHANNEL_C).is_file()
        || !root.join(KERNEL_FIFO_C).is_file()
        || !eheap.is_file()
        || !btree.is_file()
    {
        println!(
            "cargo:warning=USERD-CHID ORACLE SKIPPED: no usable open-kernel-modules tree at \
             {} (set {env_var} to relocate). The tests that judge \
             `Ga10xArch::vchid_from_userd_flags` against NVIDIA's OWN writer, reader and \
             chid recombination announce themselves as SKIPPED and assert NOTHING. Nothing \
             is vendored here to stand in for them.",
            root.display()
        );
        return;
    }

    let fail = |what: &str| -> ! {
        panic!(
            "USERD-CHID ORACLE: {what}\n\
             The tree at {} IS present, so this is NOT a machine without the oracle — it is \
             the oracle having rotted against a driver whose text moved, and degrading it to \
             a skip is how a gate stops being able to fire.",
            root.display()
        )
    };

    let chan_dispatch_src = std::fs::read_to_string(&chan_dispatch)
        .unwrap_or_else(|e| fail(&format!("cannot read {}: {e}", chan_dispatch.display())));
    let fifo_dispatch_src = std::fs::read_to_string(&fifo_dispatch)
        .unwrap_or_else(|e| fail(&format!("cannot read {}: {e}", fifo_dispatch.display())));

    // ★★ Refuse a chip name a table has never heard of, in BOTH tables — `nvoc_hal_symbol`
    // would otherwise take the `else` arm silently. Here that arm happens to be the right
    // answer for GA106 in both cases, which makes the check more important rather than
    // less: without it a typo'd chip produces the identical binding and the oracle looks
    // healthy for a chip that does not exist (`a_table_does_not_decide_behaviour`).
    for (src, path) in [
        (&chan_dispatch_src, &chan_dispatch),
        (&fifo_dispatch_src, &fifo_dispatch),
    ] {
        if !chip_is_known(src, USERD_CHID_ORACLE_CHIP) {
            fail(&format!(
                "`{USERD_CHID_ORACLE_CHIP}` appears in no `/* ChipHal: … */` comment in {} — \
                 the name is wrong.",
                path.display()
            ));
        }
    }

    // 1. The READER, as the driver's own dispatch table binds it for this chip.
    let reader_sym = nvoc_hal_symbol(
        &chan_dispatch_src,
        KCHANNEL_ALLOC_HWID_HAL_FUNC,
        USERD_CHID_ORACLE_CHIP,
    )
    .unwrap_or_else(|e| fail(&e));
    let reader_file =
        find_definition_file(&root.join(TOKEN_IMPL_ROOT), &reader_sym).unwrap_or_else(|e| fail(&e));
    let reader_src = std::fs::read_to_string(&reader_file)
        .unwrap_or_else(|e| fail(&format!("cannot read {}: {e}", reader_file.display())));
    let reader_fn = extract_function(&reader_src, &reader_sym)
        .unwrap_or_else(|e| fail(&format!("{} — {e}", reader_file.display())));
    for anchor in [
        "_CHANNEL_USERD_INDEX_PAGE_FIXED",
        "_CHANNEL_USERD_INDEX_PAGE_VALUE",
        "_CHANNEL_USERD_INDEX_VALUE",
        "_CHANNEL_USERD_INDEX_FIXED",
        "NV_ERR_INVALID_STATE",
        "kfifoChidMgrAllocChid",
    ] {
        if !reader_fn.contains(anchor) {
            fail(&format!(
                "the reader sliced for `{reader_sym}` does not contain `{anchor}`. Either \
                 the cut is wrong or this release no longer extracts the USERD index here \
                 — either way the oracle must not be believed."
            ));
        }
    }
    // ★ The slice carries the reader file's OWN `#include` lines, byte for byte.
    // `NVOS04_FLAGS_CHANNEL_USERD_INDEX_*` reach the compiler only through them.
    let reader_includes: String = reader_src
        .lines()
        .filter(|l| l.trim_start().starts_with("#include"))
        .map(|l| format!("{l}\n"))
        .collect();

    // 2. The USERD SIZE, likewise derived — it is the numerator's divisor, and on GA106 the
    //    driver binds *Maxwell's* implementation (see `USERD_SIZE_HAL_FUNC`).
    let userd_sym = nvoc_hal_symbol(
        &fifo_dispatch_src,
        USERD_SIZE_HAL_FUNC,
        USERD_CHID_ORACLE_CHIP,
    )
    .unwrap_or_else(|e| fail(&e));
    let userd_file =
        find_definition_file(&root.join(TOKEN_IMPL_ROOT), &userd_sym).unwrap_or_else(|e| fail(&e));
    let userd_src = std::fs::read_to_string(&userd_file)
        .unwrap_or_else(|e| fail(&format!("cannot read {}: {e}", userd_file.display())));
    let userd_fn = extract_function(&userd_src, &userd_sym)
        .unwrap_or_else(|e| fail(&format!("{} — {e}", userd_file.display())));
    for anchor in USERD_SLICE_ANCHORS {
        if !userd_fn.contains(anchor) {
            fail(&format!(
                "the driver code sliced for `{userd_sym}` does not contain `{anchor}`."
            ));
        }
    }
    let userd_published: String = userd_src
        .lines()
        .filter(|l| l.trim_start().starts_with("#include") && l.contains("published/"))
        .map(|l| format!("{l}\n"))
        .collect();
    if !userd_published.contains("dev_ram.h") {
        fail(&format!(
            "{} no longer includes a `published/…/dev_ram.h`, so USERD could not be sized \
             against the driver's own field definitions.",
            userd_file.display()
        ));
    }

    // 3. The three STATEMENT spans, each cut by the file's own text.
    let kchannel_src = std::fs::read_to_string(root.join(TOKEN_KCHANNEL_C))
        .unwrap_or_else(|e| fail(&format!("cannot read {TOKEN_KCHANNEL_C}: {e}")));
    let writer_span = extract_userd_writer_span(&kchannel_src).unwrap_or_else(|e| {
        fail(&format!(
            "slicing the writer out of {TOKEN_KCHANNEL_C}: {e}"
        ))
    });
    let fifo_src = std::fs::read_to_string(root.join(KERNEL_FIFO_C))
        .unwrap_or_else(|e| fail(&format!("cannot read {KERNEL_FIFO_C}: {e}")));
    let recombine_span = extract_chid_recombine_span(&fifo_src).unwrap_or_else(|e| {
        fail(&format!(
            "slicing the recombination out of {KERNEL_FIFO_C}: {e}"
        ))
    });
    let granularity_span = extract_userd_granularity_span(&fifo_src).unwrap_or_else(|e| {
        fail(&format!(
            "slicing the granularity out of {KERNEL_FIFO_C}: {e}"
        ))
    });

    // 3b. ★★ The isolation predicate the granularity span calls, bound if the driver's own
    //     header says it is halified. See `span_isolation_predicate`: the 610 tree's
    //     predicate dispatches through a function pointer, and an unbound one is a NULL
    //     call at run time — an oracle that builds clean and segfaults.
    let pred = span_isolation_predicate(granularity_span).unwrap_or_else(|e| fail(&e));
    let fifo_hdr = std::fs::read_to_string(root.join(NVOC_KERNEL_FIFO_H))
        .unwrap_or_else(|e| fail(&format!("cannot read {NVOC_KERNEL_FIFO_H}: {e}")));
    // NVOC's OWN statement of which member holds the pointer — not a guessed `__x__`.
    let fnptr_define = format!("#define {pred}_FNPTR(pKernelFifo) pKernelFifo->__{pred}__");
    let isolation_binding: Option<(String, String)> = if fifo_hdr.contains(&fnptr_define) {
        let sym = nvoc_hal_symbol(&fifo_dispatch_src, &pred, USERD_CHID_ORACLE_CHIP)
            .unwrap_or_else(|e| fail(&format!("binding the isolation predicate `{pred}`: {e}")));
        Some((format!("__{pred}__"), sym))
    } else if fifo_hdr.contains(&format!("static inline NvBool {pred}(struct KernelFifo *")) {
        // A plain inline: nothing to bind, and the span calls it directly.
        None
    } else {
        fail(&format!(
            "the granularity span calls `{pred}`, which {NVOC_KERNEL_FIFO_H} declares \
             neither as a halified entry point (no `{pred}_FNPTR` define) nor as a plain \
             `static inline NvBool {pred}(struct KernelFifo *)`. Binding it wrong is a NULL \
             call at RUN time, so this refuses at BUILD time instead."
        ));
    };

    // ★ A statement span cannot carry its own `#include` lines — it is compiled INSIDE a
    // function body. So the two source files' `#include` lines travel as separate
    // file-scope fragments, still byte for byte and still theirs.
    let chan_includes: String = kchannel_src
        .lines()
        .filter(|l| l.trim_start().starts_with("#include"))
        .map(|l| format!("{l}\n"))
        .collect();
    let fifo_includes: String = fifo_src
        .lines()
        .filter(|l| l.trim_start().starts_with("#include"))
        .map(|l| format!("{l}\n"))
        .collect();

    // Per-tag subdirectory with a stable basename, for `build_gmmu_oracle`'s reason: the
    // driver's own `NV_PRINTF` puts `__FILE__`'s basename in every diagnostic.
    let dir = out_dir.join(tag);
    std::fs::create_dir_all(&dir).expect("slice dir");
    let write = |name: &str, body: &str| -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap_or_else(|e| fail(&format!("writing {name}: {e}")));
        p
    };
    let p_chan_inc = write("userd_chid_kernel_channel_includes.inc", &chan_includes);
    let p_fifo_inc = write("userd_chid_kernel_fifo_includes.inc", &fifo_includes);
    let p_writer = write("userd_chid_writer_slice.inc", &format!("{writer_span}\n"));
    let p_reader = write(
        "userd_chid_reader_slice.inc",
        &format!("{reader_includes}\n{reader_fn}\n"),
    );
    let p_recomb = write(
        "userd_chid_recombine_slice.inc",
        &format!("{recombine_span}\n"),
    );
    let p_gran = write(
        "userd_chid_granularity_slice.inc",
        &format!("{granularity_span}\n"),
    );
    let p_usize = write(
        "userd_chid_userd_size_slice.inc",
        &format!("{userd_published}\n{userd_fn}\n"),
    );

    let harness = Path::new("oracle/userd_chid_oracle.c")
        .canonicalize()
        .expect("oracle/userd_chid_oracle.c is part of this crate");
    let bin = out_dir.join(format!("userd_chid_oracle_{tag}"));
    let mut cmd = Command::new(cc);
    cmd.arg("-std=gnu11").arg("-O1").arg("-g");
    cmd.arg("-fno-strict-aliasing");
    cmd.arg("-o").arg(&bin);
    for (macro_name, path) in [
        ("OGKM_KERNEL_CHANNEL_INCLUDES", &p_chan_inc),
        ("OGKM_KERNEL_FIFO_INCLUDES", &p_fifo_inc),
        ("OGKM_USERD_WRITER_SLICE", &p_writer),
        ("OGKM_KCHANNEL_ALLOC_HWID_SLICE", &p_reader),
        ("OGKM_CHID_RECOMBINE_SLICE", &p_recomb),
        ("OGKM_USERD_GRANULARITY_SLICE", &p_gran),
        ("OGKM_USERD_SIZE_SLICE", &p_usize),
    ] {
        cmd.arg(format!("-D{macro_name}=\"{}\"", path.display()));
    }
    cmd.arg(format!("-DOGKM_KCHANNEL_ALLOC_HWID_FN={reader_sym}"));
    cmd.arg(format!(
        "-DOGKM_KCHANNEL_ALLOC_HWID_FN_NAME=\"{reader_sym}\""
    ));
    cmd.arg(format!("-DOGKM_USERD_SIZE_FN={userd_sym}"));
    cmd.arg(format!(
        "-DOGKM_USERD_CHID_CHIP=\"{USERD_CHID_ORACLE_CHIP}\""
    ));
    if let Some((member, sym)) = &isolation_binding {
        cmd.arg(format!("-DOGKM_USERD_ISOLATION_MEMBER={member}"));
        cmd.arg(format!("-DOGKM_USERD_ISOLATION_FN={sym}"));
    }
    for d in DEFINES {
        cmd.arg(format!("-D{d}"));
    }
    cmd.arg("-include")
        .arg(root.join("src/common/sdk/nvidia/inc/cpuopsys.h"));
    for (prefix, sub) in INCLUDES {
        cmd.arg("-I").arg(root.join(prefix).join(sub));
    }
    cmd.arg(&harness);
    cmd.arg(&eheap);
    cmd.arg(&btree);

    let out = cmd
        .output()
        .unwrap_or_else(|e| fail(&format!("could not run the C compiler `{cc}`: {e}")));
    if !out.status.success() {
        fail(&format!(
            "the harness did not compile. Compiler output:\n{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        ));
    }

    println!("cargo:rustc-env={out_env}={}", bin.display());
}
