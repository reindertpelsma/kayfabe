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
