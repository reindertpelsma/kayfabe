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

fn main() {
    println!("cargo:rerun-if-changed=oracle/vbios_oracle.c");
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

        let tag = out_env.rsplit('_').next().unwrap_or("unknown");

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
