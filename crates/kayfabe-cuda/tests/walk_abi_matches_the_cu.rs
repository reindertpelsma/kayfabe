//! ★★★★★ **THE LAUNCH ABI, DIFFERENTIALLED AGAINST THE `.cu` THAT OWNS IT.**
//!
//! `kf_walk_kernel` takes `KfArgs` **by value**, so the whole contract between
//! `kayfabe-cuda` and the committed PTX is a **byte layout**. `abi.rs` states that layout a
//! second time, in Rust. This test is what stops the second statement from drifting.
//!
//! # How it works, and why this way
//!
//! It extracts the struct definitions from `cuda/walk/kf_walk.cu` **by name** (`struct
//! KfFormat { … };`), emits a small C++ program that prints every `sizeof` and `offsetof`,
//! compiles it with **`g++`**, runs it, and compares against the Rust types.
//!
//! - ⊘ **Not `bindgen`**: that would make the C the *only* statement, and the failure this
//!   guards — a Rust/PTX skew — would then be invisible until a kernel decoded garbage field
//!   offsets and it looked like a page-table bug (`THE_CONSTRAINTS.md` §21). Two statements
//!   plus a differential is what makes the skew **loud**.
//! - ⊘ **Not a line range.** An earlier draft sliced the `.cu` by line number; that rots on
//!   the first edit above the seam and would silently start checking a different program.
//!   Extraction is keyed on the struct's own name and **fails loudly** when a name is absent.
//! - ⊘ **No GPU, no CUDA toolkit, no `nvcc`.** The structs are plain C, so `g++` parses them.
//!   ⇒ this runs wherever `cargo test` runs, which is the only place a layout gate is worth
//!   having.
//!
//! ⚠ **A missing `g++` is a FAILURE, not a skip.** A check that cannot run is not a check that
//! passed — and this workspace already builds C for other reasons, so a toolchain without it
//! could not have built the tree anyway.

use kayfabe_cuda::abi::*;
use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `<root>/crates/kayfabe-cuda`.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("the manifest lives two levels below the workspace root")
        .to_path_buf()
}

/// Pull `struct <name> { … };` out of a C++ source, keyed on the name.
///
/// # Panics
/// If the struct is absent — which is the point: a rename in the `.cu` must break this test
/// rather than quietly reduce what it checks.
fn extract_struct(src: &str, name: &str) -> String {
    // ⊘⊘ **THE TYPEDEF FORM HAS TO COME ALONG.** `kf_walk.h` spells these as
    // `typedef struct KfMapRun { … } KfMapRun;`. Extracting from `struct` alone yields
    // `struct KfMapRun { … } KfMapRun;` — which in C++ declares a **variable** named
    // `KfMapRun` that then shadows the type, so every later use fails to name a type. The
    // first draft did exactly that and the probe would not compile; keeping the `typedef`
    // keyword is what makes the extracted text mean what it means in its own file.
    let needle = format!("struct {name} {{");
    let start = src.find(&needle).unwrap_or_else(|| {
        panic!(
            "cuda/walk/kf_walk.cu has no `struct {name} {{` — it was \
                                   renamed or moved, and this differential would otherwise \
                                   have gone on passing over a struct that no longer exists"
        )
    });
    let mut depth = 0usize;
    for (i, c) in src[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    // include the trailing `;`
                    let end = start + i + 1;
                    let rest = &src[end..];
                    let semi = rest.find(';').expect("a struct definition ends in `;`");
                    let body = &src[start..end + semi + 1];
                    let typedef = src[..start].trim_end().ends_with("typedef");
                    return if typedef {
                        format!("typedef {body}")
                    } else {
                        body.to_string()
                    };
                }
            }
            _ => {}
        }
    }
    panic!("`struct {name}` in kf_walk.cu has unbalanced braces");
}

/// The `#define`s the extracted structs depend on, taken from the `.cu` rather than restated.
fn extract_define(src: &str, name: &str) -> String {
    for line in src.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix(&format!("#define {name} ")) {
            return rest.split("/*").next().unwrap_or(rest).trim().to_string();
        }
    }
    panic!("cuda/walk/kf_walk.cu has no `#define {name}`");
}

#[test]
fn the_rust_mirror_matches_the_cu_byte_for_byte() {
    let root = repo_root();
    let cu = std::fs::read_to_string(root.join("cuda/walk/kf_walk.cu"))
        .expect("cuda/walk/kf_walk.cu is part of this repository");
    let h = std::fs::read_to_string(root.join("cuda/walk/kf_walk.h"))
        .expect("cuda/walk/kf_walk.h is part of this repository");

    // ⊘ `KF_DIRS` and `KF_MAX_PDB` are read from the source rather than restated here, so a
    // change to either breaks the COMPILE below rather than silently producing two layouts
    // that differ only in an array bound.
    let kf_dirs = extract_define(&cu, "KF_DIRS");
    let kf_max_pdb = extract_define(&h, "KF_MAX_PDB");

    let mut prog = String::new();
    prog.push_str("#include <stdint.h>\n#include <stddef.h>\n#include <stdio.h>\n");
    prog.push_str(&format!(
        "#define KF_DIRS {kf_dirs}\n#define KF_MAX_PDB {kf_max_pdb}\n"
    ));
    // The report ABI lives in the header; the launch ABI lives in the .cu.
    for name in ["KfReportHeader", "KfPdbEntry", "KfMapRun", "KfScope"] {
        prog.push_str(&extract_struct(&h, name));
        prog.push('\n');
    }
    for name in ["KfField", "KfDir", "KfFormat", "KfWin", "KfDev", "KfArgs"] {
        prog.push_str(&extract_struct(&cu, name));
        prog.push('\n');
    }
    prog.push_str("int main(void){\n");
    for t in [
        "KfField",
        "KfDir",
        "KfFormat",
        "KfWin",
        "KfDev",
        "KfArgs",
        "KfReportHeader",
        "KfPdbEntry",
        "KfMapRun",
        "KfScope",
    ] {
        prog.push_str(&format!("printf(\"size {t} %zu\\n\", sizeof({t}));\n"));
    }
    for (t, f) in [
        ("KfFormat", "abi_version"),
        ("KfFormat", "table_version"),
        ("KfFormat", "dir"),
        ("KfFormat", "big_va_lo"),
        ("KfFormat", "root_align"),
        ("KfFormat", "first_dir"),
        ("KfFormat", "valid_bit"),
        ("KfFormat", "pte_ap_map"),
        ("KfFormat", "addr_local"),
        ("KfFormat", "big_addr_sys"),
        ("KfFormat", "bit_volatile"),
        ("KfFormat", "pcf"),
        ("KfFormat", "ps_log2"),
        ("KfArgs", "win"),
        ("KfArgs", "fmt"),
        ("KfArgs", "dev"),
        ("KfArgs", "tbl"),
        ("KfArgs", "pdbs"),
        ("KfArgs", "npdb"),
        ("KfArgs", "scopes"),
        ("KfArgs", "nscope"),
        ("KfArgs", "hdr"),
        ("KfArgs", "rpdb"),
        ("KfArgs", "rrun"),
        ("KfReportHeader", "generation"),
        ("KfReportHeader", "run_count"),
        ("KfReportHeader", "entries_visited"),
        ("KfReportHeader", "refuse_mask"),
        ("KfReportHeader", "sparse_slots"),
        ("KfReportHeader", "ps_log2"),
        ("KfMapRun", "gpga"),
        ("KfMapRun", "len"),
        ("KfMapRun", "flags"),
        ("KfMapRun", "op"),
        ("KfMapRun", "pdb_index"),
        ("KfPdbEntry", "first_run"),
        ("KfPdbEntry", "reserved2"),
        ("KfDev", "acked"),
        ("KfDev", "tbl_pdb"),
        ("KfDev", "entries_visited"),
        ("KfDev", "sparse_slots"),
    ] {
        prog.push_str(&format!(
            "printf(\"off {t}.{f} %zu\\n\", offsetof({t}, {f}));\n"
        ));
    }
    prog.push_str("return 0;}\n");

    let dir = std::env::temp_dir().join(format!("kf_abi_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temp dir");
    let src = dir.join("abi.cpp");
    let bin = dir.join("abi");
    std::fs::write(&src, &prog).expect("write the probe");

    let out = Command::new("g++")
        .args(["-std=c++14", "-O0", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .expect(
            "g++ must be present: this differential is the only thing standing between the \
             Rust mirror and the kernel's real layout, and a check that cannot RUN is not a \
             check that passed",
        );
    assert!(
        out.status.success(),
        "the extracted structs did not compile — the extraction, not the layout, is what \
         broke:\n{}\n--- program ---\n{prog}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = Command::new(&bin).output().expect("run the probe");
    assert!(run.status.success(), "the probe did not run");
    let text = String::from_utf8_lossy(&run.stdout);

    let mut got = std::collections::BTreeMap::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let kind = it.next().unwrap_or("");
        let key = it.next().unwrap_or("");
        let val: usize = it.next().unwrap_or("0").parse().unwrap_or(0);
        got.insert(format!("{kind} {key}"), val);
    }
    let check = |k: &str, rust: usize| {
        let c = *got
            .get(k)
            .unwrap_or_else(|| panic!("the probe printed nothing for `{k}`"));
        assert_eq!(
            c, rust,
            "★ ABI SKEW on `{k}`: the .cu says {c}, abi.rs says {rust}. The kernel takes \
             KfArgs BY VALUE, so this is not a cosmetic difference — every field after the \
             first mismatch would be read at the wrong offset."
        );
    };

    check("size KfField", size_of::<KfField>());
    check("size KfDir", size_of::<KfDir>());
    check("size KfFormat", size_of::<KfFormat>());
    check("size KfWin", size_of::<KfWin>());
    check("size KfDev", size_of::<KfDev>());
    check("size KfArgs", size_of::<KfArgs>());
    check("size KfReportHeader", size_of::<KfReportHeader>());
    check("size KfPdbEntry", size_of::<KfPdbEntry>());
    check("size KfMapRun", size_of::<KfMapRun>());
    check("size KfScope", size_of::<KfScope>());

    macro_rules! off {
        ($t:ty, $f:ident, $k:literal) => {
            check($k, std::mem::offset_of!($t, $f));
        };
    }
    off!(KfFormat, abi_version, "off KfFormat.abi_version");
    off!(KfFormat, table_version, "off KfFormat.table_version");
    off!(KfFormat, dir, "off KfFormat.dir");
    off!(KfFormat, big_va_lo, "off KfFormat.big_va_lo");
    off!(KfFormat, root_align, "off KfFormat.root_align");
    off!(KfFormat, first_dir, "off KfFormat.first_dir");
    off!(KfFormat, valid_bit, "off KfFormat.valid_bit");
    off!(KfFormat, pte_ap_map, "off KfFormat.pte_ap_map");
    off!(KfFormat, addr_local, "off KfFormat.addr_local");
    off!(KfFormat, big_addr_sys, "off KfFormat.big_addr_sys");
    off!(KfFormat, bit_volatile, "off KfFormat.bit_volatile");
    off!(KfFormat, pcf, "off KfFormat.pcf");
    off!(KfFormat, ps_log2, "off KfFormat.ps_log2");
    off!(KfArgs, win, "off KfArgs.win");
    off!(KfArgs, fmt, "off KfArgs.fmt");
    off!(KfArgs, dev, "off KfArgs.dev");
    off!(KfArgs, tbl, "off KfArgs.tbl");
    off!(KfArgs, pdbs, "off KfArgs.pdbs");
    off!(KfArgs, npdb, "off KfArgs.npdb");
    off!(KfArgs, scopes, "off KfArgs.scopes");
    off!(KfArgs, nscope, "off KfArgs.nscope");
    off!(KfArgs, hdr, "off KfArgs.hdr");
    off!(KfArgs, rpdb, "off KfArgs.rpdb");
    off!(KfArgs, rrun, "off KfArgs.rrun");
    off!(KfReportHeader, generation, "off KfReportHeader.generation");
    off!(KfReportHeader, run_count, "off KfReportHeader.run_count");
    off!(
        KfReportHeader,
        entries_visited,
        "off KfReportHeader.entries_visited"
    );
    off!(
        KfReportHeader,
        refuse_mask,
        "off KfReportHeader.refuse_mask"
    );
    off!(
        KfReportHeader,
        sparse_slots,
        "off KfReportHeader.sparse_slots"
    );
    off!(KfReportHeader, ps_log2, "off KfReportHeader.ps_log2");
    off!(KfMapRun, gpga, "off KfMapRun.gpga");
    off!(KfMapRun, len, "off KfMapRun.len");
    off!(KfMapRun, flags, "off KfMapRun.flags");
    off!(KfMapRun, op, "off KfMapRun.op");
    off!(KfMapRun, pdb_index, "off KfMapRun.pdb_index");
    off!(KfPdbEntry, first_run, "off KfPdbEntry.first_run");
    off!(KfPdbEntry, reserved2, "off KfPdbEntry.reserved2");
    off!(KfDev, acked, "off KfDev.acked");
    off!(KfDev, tbl_pdb, "off KfDev.tbl_pdb");
    off!(KfDev, entries_visited, "off KfDev.entries_visited");
    off!(KfDev, sparse_slots, "off KfDev.sparse_slots");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Pull a whole function definition out of a C++ source, keyed on its signature line.
fn extract_fn(src: &str, sig: &str) -> String {
    let start = src.find(sig).unwrap_or_else(|| {
        panic!(
            "cuda/walk/kf_walk.cu has no `{sig}` — it was renamed, and the descriptor \
                differential would otherwise have gone on passing over a function that no \
                longer exists"
        )
    });
    let open = start + src[start..].find('{').expect("a function body");
    let mut depth = 0usize;
    for (i, c) in src[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return src[start..open + i + 1].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("`{sig}` has unbalanced braces");
}

/// ★★★★★ **THE SETUP DATA ITSELF, DIFFERENTIALLED** — not the layout, the ~120 NUMBERS.
///
/// The layout test above proves the two sides agree about *where* the fields are.
/// ⊘ **It says nothing about what is in them**, and `abi.rs::kf_format_ver2` is a hand
/// transcription of `kf_walk.cu`'s own `kf_format_ver2()`: five directory levels, four
/// aperture maps, four bit-field specs and the permission bits. A single wrong `lo` there
/// decodes every address at the wrong offset and looks exactly like a page-table bug — the
/// failure `THE_CONSTRAINTS.md` §21 asks be made loud.
///
/// ⇒ This compiles the `.cu`'s **own function**, dumps the descriptor it builds as bytes, and
/// compares them against the bytes the Rust builder produces. A transcription error is then a
/// red test rather than a wrong mapping on a GPU.
///
/// ⚠ §21's eventual answer is that the descriptor is *derived* from `kayfabe_mmu`'s `GmmuFmt`
/// impls rather than transcribed at all. That is increment 6's; until then this is what makes
/// the transcription safe to rely on.
#[test]
fn the_rust_ver2_descriptor_matches_the_cu_byte_for_byte() {
    let root = repo_root();
    let cu = std::fs::read_to_string(root.join("cuda/walk/kf_walk.cu")).expect("the .cu");
    let h = std::fs::read_to_string(root.join("cuda/walk/kf_walk.h")).expect("the .h");
    let kf_dirs = extract_define(&cu, "KF_DIRS");

    let mut prog = String::new();
    prog.push_str(
        "#include <stdint.h>\n#include <stddef.h>\n#include <string.h>\n#include <stdio.h>\n",
    );
    prog.push_str(&format!("#define KF_DIRS {kf_dirs}\n"));
    for d in [
        "KF_PS_NONE",
        "KF_ABI_VERSION",
        "KF_TBL_VER2",
        "KFWR_AP_VIDMEM",
        "KFWR_AP_PEER",
        "KFWR_AP_SYSCOH",
        "KFWR_AP_SYSNONCOH",
        "KFWR_PS_4K",
        "KFWR_PS_64K",
        "KFWR_PS_2M",
        "KFWR_PS_512M",
    ] {
        // ⊘ Looked up in BOTH files rather than guessed: which one owns a constant is the
        // .cu's business, and a test that assumed would break on a harmless move. ⚠ It is
        // still a FAILURE if neither has it — a constant that vanished must not default.
        let v = if cu.contains(&format!("#define {d} ")) {
            extract_define(&cu, d)
        } else {
            extract_define(&h, d)
        };
        prog.push_str(&format!("#define {d} {v}\n"));
    }
    for name in ["KfField", "KfDir", "KfFormat"] {
        prog.push_str(&extract_struct(&cu, name));
        prog.push('\n');
    }
    // ⊘ The two helpers the builder calls come along too, extracted the same way. Restating
    // them here would be a fourth transcription of the same arithmetic — `entries` is
    // `1 << (va_hi - va_lo + 1)`, and getting THAT wrong is exactly the class this test is
    // for.
    prog.push_str(&extract_fn(
        &cu,
        "static KfField kf_f(uint8_t lo, uint8_t bits, uint8_t shift)",
    ));
    prog.push('\n');
    prog.push_str(&extract_fn(
        &cu,
        "static void kf_set_dir(KfDir *d, int active, uint8_t va_lo, uint8_t va_hi,",
    ));
    prog.push('\n');
    prog.push_str(&extract_fn(&cu, "static KfFormat kf_format_ver2(void)"));
    prog.push_str(
        "\nint main(void){ KfFormat F = kf_format_ver2(); const unsigned char *p = \
         (const unsigned char *)&F; for (size_t i = 0; i < sizeof F; i++) printf(\"%02x\", \
         p[i]); printf(\"\\n\"); return 0; }\n",
    );

    let dir = std::env::temp_dir().join(format!("kf_fmt_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temp dir");
    let src = dir.join("fmt.cpp");
    let bin = dir.join("fmt");
    std::fs::write(&src, &prog).expect("write the probe");
    let out = Command::new("g++")
        .args(["-std=c++14", "-O0", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .expect("g++ must be present — a check that cannot RUN is not a check that passed");
    assert!(
        out.status.success(),
        "the extracted descriptor builder did not compile — the extraction, not the \
         transcription, is what broke:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = Command::new(&bin).output().expect("run the probe");
    let c_hex = String::from_utf8_lossy(&run.stdout).trim().to_string();

    let rust = kf_format_ver2();
    let rust_hex: String = kayfabe_cuda::driver_unsafe::view_bytes(&rust)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    if c_hex != rust_hex {
        // ⊘ Name the FIRST differing byte and the field it lands in. "the bytes differ" is
        // not actionable over a 200-byte struct with forty fields.
        let cb: Vec<u8> = (0..c_hex.len() / 2)
            .map(|i| u8::from_str_radix(&c_hex[i * 2..i * 2 + 2], 16).unwrap_or(0))
            .collect();
        let rb = kayfabe_cuda::driver_unsafe::view_bytes(&rust);
        let at = cb
            .iter()
            .zip(rb.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(cb.len().min(rb.len()));
        panic!(
            "★ THE VER2 DESCRIPTOR DIFFERS at byte {at}: the .cu says {:#04x}, abi.rs says \
             {:#04x}.\n  .cu  = {c_hex}\n  rust = {rust_hex}\nOne wrong number here decodes \
             every address at the wrong offset and reads as a page-table bug.",
            cb.get(at).copied().unwrap_or(0),
            rb.get(at).copied().unwrap_or(0),
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// ★★★ **The committed PTX declares the parameter size the mirror must produce.**
///
/// ⊘ This is a *third*, independent statement of `sizeof(KfArgs)` — and it is the one that
/// actually matters, because it is what the driver will read at launch. The `g++` differential
/// above says "Rust agrees with the `.cu`"; this says "and the PTX we ship agrees with both".
/// A `.cu` edited without regenerating the PTX fails here and nowhere else.
#[test]
fn the_committed_ptx_declares_the_same_kfargs_size() {
    let ptx = std::str::from_utf8(kayfabe_cuda::WALK_PTX).expect("the PTX is text");
    let needle = "_Z14kf_walk_kernel6KfArgs_param_0[";
    let at = ptx.find(needle).unwrap_or_else(|| {
        panic!(
            "the committed PTX has no `{needle}` — either the kernel was renamed or the PTX \
             was not regenerated from the .cu (cuda/walk/make_ptx.py)"
        )
    });
    let rest = &ptx[at + needle.len()..];
    let end = rest.find(']').expect("the .param declaration is closed");
    let declared: usize = rest[..end].parse().expect("a decimal size");
    assert_eq!(
        declared,
        size_of::<KfArgs>(),
        "★ THE COMMITTED PTX AND THE RUST MIRROR DISAGREE ABOUT KfArgs. The PTX's own \
         `.param` says {declared} bytes and abi.rs says {}. Regenerate the PTX \
         (`python3 cuda/walk/make_ptx.py kf_walk.cu kf_walk.ptx compute_75` in cuda/walk) or \
         fix the mirror — but do not ship them disagreeing: the kernel takes this struct BY \
         VALUE.",
        size_of::<KfArgs>()
    );
}

/// ⊘ The PTX must stay **PTX-only and Turing-targeted**. `make check-ptx` asserts the *test
/// binary* contains no SASS; this asserts the *committed artifact* is what we think it is,
/// which is the half that ships.
#[test]
fn the_committed_ptx_is_turing_targeted() {
    let ptx = std::str::from_utf8(kayfabe_cuda::WALK_PTX).expect("the PTX is text");
    assert!(
        ptx.contains(".target sm_75"),
        "`THE_CONSTRAINTS.md` §21: the PTX must be built for compute_75 so the driver JITs \
         forward onto any Turing-or-later part. The committed artifact does not say so."
    );
    for sym in [
        "_Z15kf_begin_kernelP5KfDev",
        "_Z14kf_walk_kernel6KfArgs",
        "_Z14kf_diff_kernel6KfArgs",
    ] {
        assert!(
            ptx.contains(sym),
            "the committed PTX has no entry `{sym}`; the Rust side resolves it by name and \
             would refuse at bring-up"
        );
    }
}

/// ★★★ **THE `#define`s, DIFFERENTIALLED** — the class the struct test cannot see.
///
/// ⊘⊘ `KFWR_MAGIC` was transcribed **byte-reversed** and a real boot is what caught it: the
/// kernel returned a correct report (`runs=1 entries=1796 refusals=0 sparse=1`) and this
/// crate's validator refused it as *"not a walk report"*. The layout differential was green
/// throughout — a `#define` is not a field, so it was outside what that test quantifies over.
///
/// ★ The lesson is the shape, not the constant: **a differential is only as wide as the thing
/// it enumerates.** This one enumerates the constants; if a new `KFWR_*` the Rust side mirrors
/// is added, add it here too.
#[test]
fn the_report_constants_match_the_header() {
    let root = repo_root();
    let h = std::fs::read_to_string(root.join("cuda/walk/kf_walk.h")).expect("the .h");
    let parse = |name: &str| -> u64 {
        let v = extract_define(&h, name);
        // `0x5257464Bu`, `(1u << 0)`, `0u` — the three spellings the header actually uses.
        let v = v.trim().trim_end_matches('u');
        if let Some(rest) = v.strip_prefix("(1u << ") {
            let sh: u32 = rest.trim_end_matches(')').trim().parse().expect("a shift");
            return 1u64 << sh;
        }
        if let Some(hex) = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
            return u64::from_str_radix(hex.trim_end_matches('u'), 16).expect("hex");
        }
        v.parse().expect("a decimal")
    };
    assert_eq!(
        parse("KFWR_MAGIC"),
        u64::from(kayfabe_cuda::abi::KFWR_MAGIC),
        "★ the report magic differs. It spells \"KFWR\" as BYTES, so as a u32 literal the \
         characters look reversed — which is exactly how it was got wrong once."
    );
    assert_eq!(
        parse("KFWR_HF_TRUNCATED"),
        u64::from(kayfabe_cuda::abi::KFWR_HF_TRUNCATED)
    );
    assert_eq!(
        parse("KFWR_HF_RESYNC"),
        u64::from(kayfabe_cuda::abi::KFWR_HF_RESYNC)
    );
    assert_eq!(
        parse("KF_ABI_VERSION"),
        u64::from(kayfabe_cuda::abi::KF_ABI_VERSION)
    );
    assert_eq!(
        parse("KF_TBL_VER2"),
        u64::from(kayfabe_cuda::abi::KF_TBL_VER2)
    );
    assert_eq!(
        parse("KF_TBL_VER3"),
        u64::from(kayfabe_cuda::abi::KF_TBL_VER3)
    );
    assert_eq!(parse("KF_MAX_PDB"), kayfabe_cuda::abi::KF_MAX_PDB as u64);
    assert_eq!(
        parse("KF_MAX_SCOPE"),
        kayfabe_cuda::abi::KF_MAX_SCOPE as u64
    );
    for (n, r) in [
        ("KFWR_AP_VIDMEM", kayfabe_cuda::abi::AP_VID),
        ("KFWR_AP_PEER", kayfabe_cuda::abi::AP_PEER),
        ("KFWR_AP_SYSCOH", kayfabe_cuda::abi::AP_SYS),
        ("KFWR_AP_SYSNONCOH", kayfabe_cuda::abi::AP_SYS_NC),
        ("KFWR_PS_4K", kayfabe_cuda::abi::PS_4K),
        ("KFWR_PS_64K", kayfabe_cuda::abi::PS_64K),
        ("KFWR_PS_2M", kayfabe_cuda::abi::PS_2M),
        ("KFWR_PS_512M", kayfabe_cuda::abi::PS_512M),
    ] {
        assert_eq!(parse(n), u64::from(r), "{n} differs");
    }
}
