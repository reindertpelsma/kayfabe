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

use kf_cuda::abi::*;
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
    let kf_max_reset = extract_define(&h, "KF_MAX_RESET");
    let kf_max_pdb_l = extract_define(&h, "KF_MAX_PDB_L");
    let kf_max_slots = extract_define(&h, "KF_MAX_SLOTS");

    let mut prog = String::new();
    prog.push_str("#include <stdint.h>\n#include <stddef.h>\n#include <stdio.h>\n");
    prog.push_str(&format!(
        "#define KF_DIRS {kf_dirs}\n#define KF_MAX_PDB {kf_max_pdb}\n#define KF_MAX_RESET {kf_max_reset}\n\
         #define KF_MAX_PDB_L {kf_max_pdb_l}\n#define KF_MAX_SLOTS {kf_max_slots}\n"
    ));
    // The report ABI lives in the header; the launch ABI lives in the .cu.
    for name in [
        "KfReportHeader",
        "KfPdbEntry",
        "KfMapRun",
        "KfScope",
        "KfSlot",
        "KfAck",
        "KfLayout",
    ] {
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
        "KfSlot",
        "KfAck",
        "KfLayout",
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
        ("KfArgs", "walk"),
        ("KfArgs", "com"),
        ("KfArgs", "slot"),
        ("KfArgs", "pdbs"),
        ("KfArgs", "slots"),
        ("KfArgs", "npdb"),
        ("KfArgs", "key_perm"),
        ("KfArgs", "ack"),
        ("KfArgs", "ack_code"),
        ("KfArgs", "scratch"),
        ("KfArgs", "iscratch"),
        ("KfArgs", "hdr"),
        ("KfArgs", "rpdb"),
        ("KfArgs", "rrun"),
        ("KfArgs", "lay"),
        ("KfDev", "need"),
        ("KfLayout", "walk_cap"),
        ("KfLayout", "prev_off"),
        ("KfLayout", "prev_cap"),
        ("KfLayout", "slot_off"),
        ("KfLayout", "slot_cap"),
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
        ("KfDev", "committed"),
        ("KfDev", "max_slots"),
        ("KfDev", "tbl_run_count"),
        ("KfDev", "diff_count"),
        ("KfDev", "diff_vflags"),
        ("KfDev", "entry_refuse"),
        ("KfAck", "nrun"),
        ("KfAck", "reset"),
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
    // ★ `v3-sec-rawaddr`: `KfWin`/`KfArgs` carry device addresses and are declared only inside
    // the perimeter; their layouts are exported as data for exactly this comparison.
    check("size KfWin", KF_WIN_LAYOUT.size);
    check("size KfDev", size_of::<KfDev>());
    check("size KfArgs", KF_ARGS_LAYOUT.size);
    check("size KfReportHeader", size_of::<KfReportHeader>());
    check("size KfPdbEntry", size_of::<KfPdbEntry>());
    check("size KfMapRun", size_of::<KfMapRun>());
    check("size KfScope", size_of::<KfScope>());
    check("size KfSlot", size_of::<KfSlot>());
    check("size KfAck", size_of::<KfAck>());
    check("size KfLayout", size_of::<KfLayout>());

    macro_rules! off {
        ($t:ty, $f:ident, $k:literal) => {
            check($k, std::mem::offset_of!($t, $f));
        };
    }
    let args_off = |f: &str| {
        KF_ARGS_LAYOUT
            .fields
            .iter()
            .find(|(n, _)| *n == f)
            .unwrap_or_else(|| panic!("KF_ARGS_LAYOUT has no field `{f}`"))
            .1
    };
    assert_eq!(
        KF_ARGS_LAYOUT.fields.len(),
        18,
        "every KfArgs field is in the exported layout"
    );
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
    check("off KfArgs.win", args_off("win"));
    check("off KfArgs.fmt", args_off("fmt"));
    check("off KfArgs.dev", args_off("dev"));
    check("off KfArgs.walk", args_off("walk"));
    check("off KfArgs.com", args_off("com"));
    check("off KfArgs.slot", args_off("slot"));
    check("off KfArgs.pdbs", args_off("pdbs"));
    check("off KfArgs.slots", args_off("slots"));
    check("off KfArgs.npdb", args_off("npdb"));
    check("off KfArgs.key_perm", args_off("key_perm"));
    check("off KfArgs.ack", args_off("ack"));
    check("off KfArgs.ack_code", args_off("ack_code"));
    check("off KfArgs.scratch", args_off("scratch"));
    check("off KfArgs.iscratch", args_off("iscratch"));
    check("off KfArgs.hdr", args_off("hdr"));
    check("off KfArgs.rpdb", args_off("rpdb"));
    check("off KfArgs.rrun", args_off("rrun"));
    check("off KfArgs.lay", args_off("lay"));
    off!(KfDev, need, "off KfDev.need");
    off!(KfLayout, walk_cap, "off KfLayout.walk_cap");
    off!(KfLayout, prev_off, "off KfLayout.prev_off");
    off!(KfLayout, prev_cap, "off KfLayout.prev_cap");
    off!(KfLayout, slot_off, "off KfLayout.slot_off");
    off!(KfLayout, slot_cap, "off KfLayout.slot_cap");
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
    off!(KfDev, committed, "off KfDev.committed");
    off!(KfDev, max_slots, "off KfDev.max_slots");
    off!(KfDev, tbl_run_count, "off KfDev.tbl_run_count");
    off!(KfDev, diff_count, "off KfDev.diff_count");
    off!(KfDev, diff_vflags, "off KfDev.diff_vflags");
    off!(KfDev, entry_refuse, "off KfDev.entry_refuse");
    off!(KfAck, nrun, "off KfAck.nrun");
    off!(KfAck, reset, "off KfAck.reset");
    off!(KfDev, entries_visited, "off KfDev.entries_visited");
    off!(KfDev, sparse_slots, "off KfDev.sparse_slots");

    let _ = std::fs::remove_dir_all(&dir);
}

/// The member declarations of an extracted `struct … { … };`, as `(name, array length)`.
///
/// # Panics
/// On a declaration this reader does not understand (a nested struct, a bitfield) — the report
/// structs have neither, and one appearing must break this test rather than be skipped.
fn fields_of(def: &str) -> Vec<(String, Option<usize>)> {
    let open = def.find('{').expect("a struct body");
    let close = def.rfind('}').expect("a struct body");
    let mut body = String::new();
    let mut rest = &def[open + 1..close];
    while let Some(at) = rest.find("/*") {
        body.push_str(&rest[..at]);
        let end = rest[at..].find("*/").expect("a closed comment");
        rest = &rest[at + end + 2..];
    }
    body.push_str(rest);
    let body: Vec<&str> = body
        .lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .collect();
    body.join("\n")
        .split(';')
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(|d| {
            assert!(
                !d.contains('{') && !d.contains(':'),
                "`{d}`: a nested struct or a bitfield — the report is documented to have neither"
            );
            let last = d.split_whitespace().last().expect("a declarator");
            match last.split_once('[') {
                Some((n, len)) => (
                    n.to_string(),
                    Some(
                        len.trim_end_matches(']')
                            .parse()
                            .expect("a literal array length"),
                    ),
                ),
                None => (last.to_string(), None),
            }
        })
        .collect()
}

/// One report field as a number, keyed `T.f` (or `T.f[k]` per array element).
trait Flat {
    fn flat(&self, key: &str, out: &mut std::collections::BTreeMap<String, u64>);
}
impl Flat for u16 {
    fn flat(&self, key: &str, out: &mut std::collections::BTreeMap<String, u64>) {
        out.insert(key.to_string(), u64::from(*self));
    }
}
impl Flat for u32 {
    fn flat(&self, key: &str, out: &mut std::collections::BTreeMap<String, u64>) {
        out.insert(key.to_string(), u64::from(*self));
    }
}
impl Flat for u64 {
    fn flat(&self, key: &str, out: &mut std::collections::BTreeMap<String, u64>) {
        out.insert(key.to_string(), *self);
    }
}
impl<const N: usize> Flat for [u8; N] {
    fn flat(&self, key: &str, out: &mut std::collections::BTreeMap<String, u64>) {
        for (k, v) in self.iter().enumerate() {
            out.insert(format!("{key}[{k}]"), u64::from(*v));
        }
    }
}

/// ★★★★★ **THE REPORT DECODERS, DIFFERENTIALLED FIELD BY FIELD AGAINST THE C COMPILER**
/// (`STATUS_AND_HANDOFF.md` §4 item 6).
///
/// The struct test above samples offsets; this one takes **every** member of `KfReportHeader`,
/// `KfPdbEntry` and `KfMapRun` from `kf_walk.h` itself (by parsing the extracted definitions, so a
/// member added in C is checked without anyone remembering to add it here), and has `g++`:
/// - fill each struct with a byte pattern in which no two bytes are equal,
/// - print the bytes, and each member's `offsetof`, `sizeof` and **value as C reads it**.
///
/// Then the Rust side decodes those bytes with the struct's ONE decoder (`abi.rs`,
/// `report_codec!`) and must agree member for member — offset, width and value — with no member
/// missing on either side, and `encode` must give back C's bytes exactly. A field read at the
/// wrong offset, at the wrong width or in the wrong byte order is a named failure here, not a
/// wrong mapping on a GPU.
#[test]
fn every_report_field_decodes_to_what_the_c_compiler_reads() {
    use std::collections::{BTreeMap, BTreeSet};
    let root = repo_root();
    let h = std::fs::read_to_string(root.join("cuda/walk/kf_walk.h")).expect("the .h");
    let names = ["KfReportHeader", "KfPdbEntry", "KfMapRun"];

    let mut prog = String::from("#include <stdint.h>\n#include <stddef.h>\n#include <stdio.h>\n");
    let mut c_fields: BTreeMap<&str, Vec<(String, Option<usize>)>> = BTreeMap::new();
    for t in names {
        let def = extract_struct(&h, t);
        c_fields.insert(t, fields_of(&def));
        prog.push_str(&def);
        prog.push('\n');
    }
    prog.push_str("int main(void){\n");
    for t in names {
        prog.push_str(&format!(
            "{{ {t} x; unsigned char *p = (unsigned char *)&x;\n\
             for (size_t i = 0; i < sizeof x; i++) p[i] = (unsigned char)(i * 37u + 11u);\n\
             printf(\"size {t} %zu\\n\", sizeof x);\n\
             printf(\"bytes {t} \"); for (size_t i = 0; i < sizeof x; i++) printf(\"%02x\", p[i]); printf(\"\\n\");\n"
        ));
        for (f, n) in &c_fields[t] {
            prog.push_str(&format!(
                "printf(\"off {t}.{f} %zu\\n\", offsetof({t}, {f}));\n\
                 printf(\"width {t}.{f} %zu\\n\", sizeof x.{f});\n"
            ));
            match n {
                None => prog.push_str(&format!(
                    "printf(\"val {t}.{f} %llu\\n\", (unsigned long long)x.{f});\n"
                )),
                Some(n) => prog.push_str(&format!(
                    "for (int k = 0; k < {n}; k++) printf(\"val {t}.{f}[%d] %llu\\n\", k, (unsigned long long)x.{f}[k]);\n"
                )),
            }
        }
        prog.push_str("}\n");
    }
    prog.push_str("return 0;}\n");

    let dir = std::env::temp_dir().join(format!("kf_report_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temp dir");
    let src = dir.join("report.cpp");
    let bin = dir.join("report");
    std::fs::write(&src, &prog).expect("write the probe");
    let out = Command::new("g++")
        .args(["-std=c++14", "-O0", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .expect("g++ must be present — a check that cannot RUN is not a check that passed");
    assert!(
        out.status.success(),
        "the extracted report structs did not compile — the extraction, not the layout, is what \
         broke:\n{}\n--- program ---\n{prog}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = Command::new(&bin).output().expect("run the probe");
    assert!(run.status.success(), "the probe did not run");
    let text = String::from_utf8_lossy(&run.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    let mut c_bytes: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut c_num: BTreeMap<String, u64> = BTreeMap::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (kind, key, v) = (
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
        );
        if kind == "bytes" {
            let b = (0..v.len() / 2)
                .map(|i| u8::from_str_radix(&v[i * 2..i * 2 + 2], 16).expect("hex"))
                .collect();
            c_bytes.insert(key.to_string(), b);
        } else {
            c_num.insert(format!("{kind} {key}"), v.parse().expect("a decimal"));
        }
    }

    // The Rust side: the ONE decoder, over C's bytes; each member's offset and width.
    macro_rules! rust_side {
        ($t:ident; $($f:ident),+ $(,)?) => {{
            let bytes = &c_bytes[stringify!($t)];
            let v = $t::decode(bytes);
            assert_eq!(&v.encode()[..], &bytes[..], "{}: encode(decode(C's bytes)) is C's bytes", stringify!($t));
            assert_eq!(c_num[concat!("size ", stringify!($t))], $t::BYTES as u64, "{}: sizeof", stringify!($t));
            let mut vals = BTreeMap::new();
            let mut layout = BTreeMap::new();
            $(
                let key = concat!(stringify!($t), ".", stringify!($f));
                Flat::flat(&v.$f, key, &mut vals);
                layout.insert(key.to_string(), (std::mem::offset_of!($t, $f), std::mem::size_of_val(&v.$f)));
            )+
            (vals, layout)
        }};
    }
    let sides = [
        (
            "KfReportHeader",
            rust_side!(KfReportHeader; magic, version, flags, generation, acked_generation, pdb_count,
                pdb_capacity, run_count, run_capacity, entries_visited, refusals, refuse_mask,
                sparse_slots, ps_log2),
        ),
        (
            "KfPdbEntry",
            rust_side!(KfPdbEntry; pdb, first_run, run_count, vas_flags, reserved, reserved2),
        ),
        (
            "KfMapRun",
            rust_side!(KfMapRun; va, gpga, len, flags, op, pdb_index),
        ),
    ];
    for (t, (vals, layout)) in sides {
        let c_members: BTreeSet<String> = c_fields[t]
            .iter()
            .map(|(f, _)| format!("{t}.{f}"))
            .collect();
        let rust_members: BTreeSet<String> = layout.keys().cloned().collect();
        assert_eq!(
            c_members, rust_members,
            "★ {t}: the header and the Rust decoder list different members"
        );
        for (key, (off, width)) in &layout {
            assert_eq!(
                c_num[&format!("off {key}")],
                *off as u64,
                "★ REPORT SKEW: offset of {key}"
            );
            assert_eq!(
                c_num[&format!("width {key}")],
                *width as u64,
                "★ REPORT SKEW: width of {key}"
            );
        }
        let c_vals: BTreeMap<String, u64> = c_num
            .iter()
            .filter_map(|(k, v)| {
                k.strip_prefix("val ")
                    .filter(|k| k.starts_with(&format!("{t}.")))
                    .map(|k| (k.to_string(), *v))
            })
            .collect();
        assert_eq!(
            c_vals, vals,
            "★ {t}: the Rust decoder reads different values from the bytes C wrote than C does"
        );
    }
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
    assert_descriptor_matches("kf_format_ver2", &kf_format_ver2(), "VER2");
}

/// ★ Hopper and Blackwell's format, pinned the same way (w826: every family first-class).
#[test]
fn the_rust_ver3_descriptor_matches_the_cu_byte_for_byte() {
    assert_descriptor_matches(
        "kf_format_ver3_untested",
        &kf_cuda::abi::kf_format_ver3(),
        "VER3",
    );
}

fn assert_descriptor_matches(cu_fn: &str, rust: &kf_cuda::abi::KfFormat, tag: &str) {
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
        "KF_TBL_VER3",
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
    prog.push_str(&extract_fn(&cu, &format!("static KfFormat {cu_fn}(void)")));
    prog.push_str(&format!(
        "\nint main(void){{ KfFormat F = {cu_fn}(); const unsigned char *p = \
         (const unsigned char *)&F; for (size_t i = 0; i < sizeof F; i++) printf(\"%02x\", \
         p[i]); printf(\"\\n\"); return 0; }}\n"
    ));

    let dir = std::env::temp_dir().join(format!("kf_fmt_{tag}_{}", std::process::id()));
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

    // ★ T17: the ENCODER the launches use (`KfFormat::encode`, every field at its offset, padding
    // zero by construction) — no longer the compiler's raw bytes of the value.
    let rust_bytes = rust.encode();
    let rust_hex: String = rust_bytes.iter().map(|b| format!("{b:02x}")).collect();

    if c_hex != rust_hex {
        // ⊘ Name the FIRST differing byte and the field it lands in. "the bytes differ" is
        // not actionable over a 200-byte struct with forty fields.
        let cb: Vec<u8> = (0..c_hex.len() / 2)
            .map(|i| u8::from_str_radix(&c_hex[i * 2..i * 2 + 2], 16).unwrap_or(0))
            .collect();
        let rb = &rust_bytes[..];
        let at = cb
            .iter()
            .zip(rb.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(cb.len().min(rb.len()));
        panic!(
            "★ THE {tag} DESCRIPTOR DIFFERS at byte {at}: the .cu says {:#04x}, abi.rs says \
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
    let ptx = std::str::from_utf8(kf_cuda::WALK_PTX).expect("the PTX is text");
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
        declared, KF_ARGS_LAYOUT.size,
        "★ THE COMMITTED PTX AND THE RUST MIRROR DISAGREE ABOUT KfArgs. The PTX's own \
         `.param` says {declared} bytes and abi.rs says {}. Regenerate the PTX \
         (`python3 cuda/walk/make_ptx.py kf_walk.cu kf_walk.ptx compute_75` in cuda/walk) or \
         fix the mirror — but do not ship them disagreeing: the kernel takes this struct BY \
         VALUE.",
        KF_ARGS_LAYOUT.size
    );
}

/// ⊘ The PTX must stay **PTX-only and Turing-targeted**. `make check-ptx` asserts the *test
/// binary* contains no SASS; this asserts the *committed artifact* is what we think it is,
/// which is the half that ships.
#[test]
fn the_committed_ptx_is_turing_targeted() {
    let ptx = std::str::from_utf8(kf_cuda::WALK_PTX).expect("the PTX is text");
    assert!(
        ptx.contains(".target sm_75"),
        "`THE_CONSTRAINTS.md` §21: the PTX must be built for compute_75 so the driver JITs \
         forward onto any Turing-or-later part. The committed artifact does not say so."
    );
    for sym in [
        "_Z15kf_begin_kernel6KfArgs",
        "_Z14kf_walk_kernel6KfArgs",
        "_Z13kf_diff_slots6KfArgs",
        "_Z12kf_diff_emit6KfArgs",
        "_Z16kf_commit_kernel6KfArgs",
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
        u64::from(kf_cuda::abi::KFWR_MAGIC),
        "★ the report magic differs. It spells \"KFWR\" as BYTES, so as a u32 literal the \
         characters look reversed — which is exactly how it was got wrong once."
    );
    assert_eq!(
        parse("KFWR_HF_TRUNCATED"),
        u64::from(kf_cuda::abi::KFWR_HF_TRUNCATED)
    );
    assert_eq!(
        parse("KFWR_HF_RESYNC"),
        u64::from(kf_cuda::abi::KFWR_HF_RESYNC)
    );
    assert_eq!(parse("KFWR_HF_DIFF"), u64::from(kf_cuda::abi::KFWR_HF_DIFF));
    assert_eq!(parse("KFWR_RF_HELD"), u64::from(kf_cuda::abi::KFWR_RF_HELD));
    assert_eq!(
        parse("KFWR_RF_READ_ONLY"),
        u64::from(kf_cuda::abi::KFWR_RF_READ_ONLY)
    );
    assert_eq!(
        parse("KFWR_RF_ATOMIC_DISABLE"),
        u64::from(kf_cuda::abi::KFWR_RF_ATOMIC_DISABLE)
    );
    assert_eq!(
        parse("KFWR_RF_VOLATILE"),
        u64::from(kf_cuda::abi::KFWR_RF_VOLATILE)
    );
    assert_eq!(
        parse("KFWR_RF_PRIVILEGE"),
        u64::from(kf_cuda::abi::KFWR_RF_PRIVILEGE)
    );
    // ★ The flags word's named RANGES (`KFWR_RF_*_{SHIFT,MASK}`), which every Rust reader goes
    // through (`kf_cuda::abi::RF_*`) instead of restating a shift.
    for (n, r) in [
        ("KFWR_RF_AP_SHIFT", kf_cuda::abi::KFWR_RF_AP_SHIFT),
        ("KFWR_RF_AP_MASK", kf_cuda::abi::KFWR_RF_AP_MASK),
        ("KFWR_RF_PS_SHIFT", kf_cuda::abi::KFWR_RF_PS_SHIFT),
        ("KFWR_RF_PS_MASK", kf_cuda::abi::KFWR_RF_PS_MASK),
        ("KFWR_RF_KIND_SHIFT", kf_cuda::abi::KFWR_RF_KIND_SHIFT),
        ("KFWR_RF_KIND_MASK", kf_cuda::abi::KFWR_RF_KIND_MASK),
    ] {
        assert_eq!(parse(n), u64::from(r), "{n} differs");
    }
    assert_eq!(
        (kf_cuda::abi::RF_AP.shift, kf_cuda::abi::RF_AP.mask),
        (
            kf_cuda::abi::KFWR_RF_AP_SHIFT,
            kf_cuda::abi::KFWR_RF_AP_MASK
        )
    );
    assert_eq!(
        (kf_cuda::abi::RF_PS.shift, kf_cuda::abi::RF_PS.mask),
        (
            kf_cuda::abi::KFWR_RF_PS_SHIFT,
            kf_cuda::abi::KFWR_RF_PS_MASK
        )
    );
    assert_eq!(
        (kf_cuda::abi::RF_KIND.shift, kf_cuda::abi::RF_KIND.mask),
        (
            kf_cuda::abi::KFWR_RF_KIND_SHIFT,
            kf_cuda::abi::KFWR_RF_KIND_MASK
        )
    );
    // ⊘ The class mask has no header macro: the .cu spells it `(flags >> KFWR_RF_PS_SHIFT) & 3u`
    // in `kf_pcls` and `kf_ps_bytes_of`. Held to the .cu's own text, so a change there is loud.
    let cu = std::fs::read_to_string(repo_root().join("cuda/walk/kf_walk.cu")).expect("the .cu");
    let pcls = "(flags >> KFWR_RF_PS_SHIFT) & 3u";
    assert!(
        cu.contains(&format!("kf_pcls(uint32_t flags) {{ return {pcls}; }}")),
        "kf_walk.cu's kf_pcls no longer reads `{pcls}` — re-derive kf_cuda::abi::KF_PS_CLASS_MASK"
    );
    assert_eq!(kf_cuda::abi::KF_PS_CLASS_MASK, 3);
    assert_eq!(
        (kf_cuda::abi::RF_CLASS.shift, kf_cuda::abi::RF_CLASS.mask),
        (
            kf_cuda::abi::KFWR_RF_PS_SHIFT,
            kf_cuda::abi::KF_PS_CLASS_MASK
        )
    );
    // ★ v3-roperm: the header spells the key sets as ORs of the names; each must be the same set
    // abi.rs builds.
    let or_of = |name: &str| -> u32 {
        let mut body = extract_define(&h, name);
        // `KFWR_RF_KEY_PERM_ALL` is continued on the next line (`\` + newline).
        if body.trim_end().ends_with('\\') {
            let at = h.find(&format!("#define {name} ")).expect("the define");
            let rest = &h[at..];
            let second = rest.lines().nth(1).expect("the continuation line");
            body = format!("{}{}", body.trim_end().trim_end_matches('\\'), second);
        }
        let body = body.trim().trim_start_matches('(').trim_end_matches(')');
        body.split('|')
            .map(|n| u32::try_from(parse(n.trim())).expect("a flag"))
            .fold(0, |a, b| a | b)
    };
    assert_eq!(
        or_of("KFWR_RF_KEY_PERM_ALL"),
        kf_cuda::abi::KFWR_RF_KEY_PERM_ALL,
        "KFWR_RF_KEY_PERM_ALL differs"
    );
    assert_eq!(
        or_of("KFWR_RF_KEY_PERM_DEFAULT"),
        kf_cuda::abi::KFWR_RF_KEY_PERM_DEFAULT,
        "KFWR_RF_KEY_PERM_DEFAULT differs"
    );
    assert_eq!(
        kf_cuda::abi::KFWR_RF_KEY_PERM_DEFAULT & kf_cuda::abi::KFWR_RF_ATOMIC_DISABLE,
        0,
        "ATOMIC_DISABLE joins the key only under KF3_CARRY_ATOMIC_DISABLE"
    );
    assert_eq!(
        parse("KFWR_V_PARTIAL"),
        u64::from(kf_cuda::abi::KFWR_V_PARTIAL)
    );
    assert_eq!(
        parse("KFWR_V_OVERFLOW"),
        u64::from(kf_cuda::abi::KFWR_V_OVERFLOW)
    );
    assert_eq!(
        parse("KFWR_V_REFUSED"),
        u64::from(kf_cuda::abi::KFWR_V_REFUSED)
    );
    assert_eq!(
        parse("KFWR_ACK_APPLIED"),
        u64::from(kf_cuda::abi::KFWR_ACK_APPLIED)
    );
    assert_eq!(
        parse("KFWR_ACK_HELD"),
        u64::from(kf_cuda::abi::KFWR_ACK_HELD)
    );
    assert_eq!(parse("KF_MAX_RESET"), kf_cuda::abi::KF_MAX_RESET as u64);
    assert_eq!(
        parse("KF_ABI_VERSION"),
        u64::from(kf_cuda::abi::KF_ABI_VERSION)
    );
    assert_eq!(parse("KF_TBL_VER2"), u64::from(kf_cuda::abi::KF_TBL_VER2));
    assert_eq!(parse("KF_TBL_VER3"), u64::from(kf_cuda::abi::KF_TBL_VER3));
    assert_eq!(parse("KF_MAX_PDB"), kf_cuda::abi::KF_MAX_PDB as u64);
    assert_eq!(parse("KF_MAX_SCOPE"), kf_cuda::abi::KF_MAX_SCOPE as u64);
    for (n, r) in [
        ("KFWR_AP_VIDMEM", kf_cuda::abi::AP_VID),
        ("KFWR_AP_PEER", kf_cuda::abi::AP_PEER),
        ("KFWR_AP_SYSCOH", kf_cuda::abi::AP_SYS),
        ("KFWR_AP_SYSNONCOH", kf_cuda::abi::AP_SYS_NC),
        ("KFWR_PS_4K", kf_cuda::abi::PS_4K),
        ("KFWR_PS_64K", kf_cuda::abi::PS_64K),
        ("KFWR_PS_2M", kf_cuda::abi::PS_2M),
        ("KFWR_PS_512M", kf_cuda::abi::PS_512M),
    ] {
        assert_eq!(parse(n), u64::from(r), "{n} differs");
    }
}

/// ★★ **T17 — every host-to-device launch struct encodes to exactly the C compiler's bytes**
/// (`v3-sec-rawaddr`, 2026-10-04). `KfDev`, `KfLayout` and `KfAck` reach the GPU only through
/// their `encode()` (each field at its `offset_of!`, padding zero by construction). A `g++` program
/// zeroes each struct as the `.cu` does, assigns every field (every array element) a distinct value,
/// and prints its bytes; the Rust encoder of the same values must produce them byte for byte — a
/// field the encoder skipped, misplaced or narrowed differs here.
#[test]
fn every_launch_struct_encodes_to_the_c_compilers_bytes() {
    let root = repo_root();
    let cu = std::fs::read_to_string(root.join("cuda/walk/kf_walk.cu")).expect("the .cu");
    let h = std::fs::read_to_string(root.join("cuda/walk/kf_walk.h")).expect("the header");
    // A distinct value per (field, element), truncated by the field's width on both sides.
    let v = |k: u64, j: u64| {
        (k << 40) ^ j.wrapping_mul(0x9E37_79B9).wrapping_add(0x5A5A_A5A5) ^ 0x3C00_0000_0000_00C3
    };
    let arr = |k: u64| -> [u32; KF_MAX_PDB] { core::array::from_fn(|j| v(k, j as u64) as u32) };
    let dev = KfDev {
        generation: v(0, 0),
        committed: v(1, 0),
        runs_per_pdb: v(2, 0) as u32,
        entry_budget: v(3, 0) as u32,
        run_capacity: v(4, 0) as u32,
        pdb_capacity: v(5, 0) as u32,
        max_pdbs: v(6, 0) as u32,
        max_slots: v(7, 0) as u32,
        tbl_run_count: arr(8),
        diff_count: arr(9),
        diff_vflags: arr(10),
        entry_refuse: arr(11),
        entries_visited: v(12, 0),
        refusals: v(13, 0) as u32,
        refuse_mask: v(14, 0) as u32,
        hdr_flags: v(15, 0) as u32,
        walk_trunc: v(16, 0) as u32,
        walk_abort: v(17, 0) as u32,
        sparse_slots: v(18, 0) as u32,
        need: arr(19),
    };
    let slots = |k: u64| -> [u32; KF_MAX_SLOTS] { core::array::from_fn(|j| v(k, j as u64) as u32) };
    let lay = KfLayout {
        walk_off: arr(20),
        walk_cap: arr(21),
        prev_off: arr(22),
        prev_cap: arr(23),
        slot_off: slots(24),
        slot_cap: slots(25),
    };
    let ack = KfAck {
        generation: v(26, 0),
        nrun: v(27, 0) as u32,
        nreset: v(28, 0) as u32,
        reset: core::array::from_fn(|j| v(29, j as u64) as u32),
    };
    // The C side: the same values, assigned field by field into a zeroed struct.
    let mut set = String::new();
    let mut scalar =
        |var: &str, f: &str, k: u64| set.push_str(&format!("{var}.{f} = {}ULL;\n", v(k, 0)));
    for (f, k) in [
        ("generation", 0),
        ("committed", 1),
        ("runs_per_pdb", 2),
        ("entry_budget", 3),
        ("run_capacity", 4),
        ("pdb_capacity", 5),
        ("max_pdbs", 6),
        ("max_slots", 7),
        ("entries_visited", 12),
        ("refusals", 13),
        ("refuse_mask", 14),
        ("hdr_flags", 15),
        ("walk_trunc", 16),
        ("walk_abort", 17),
        ("sparse_slots", 18),
    ] {
        scalar("D", f, k);
    }
    for (f, k) in [("generation", 26), ("nrun", 27), ("nreset", 28)] {
        scalar("A", f, k);
    }
    let mut array = |var: &str, f: &str, k: u64, n: usize| {
        for j in 0..n {
            set.push_str(&format!("{var}.{f}[{j}] = {}ULL;\n", v(k, j as u64)));
        }
    };
    for (f, k) in [
        ("tbl_run_count", 8),
        ("diff_count", 9),
        ("diff_vflags", 10),
        ("entry_refuse", 11),
        ("need", 19),
    ] {
        array("D", f, k, KF_MAX_PDB);
    }
    for (f, k) in [
        ("walk_off", 20),
        ("walk_cap", 21),
        ("prev_off", 22),
        ("prev_cap", 23),
    ] {
        array("L", f, k, KF_MAX_PDB);
    }
    for (f, k) in [("slot_off", 24), ("slot_cap", 25)] {
        array("L", f, k, KF_MAX_SLOTS);
    }
    array("A", "reset", 29, KF_MAX_RESET);

    let mut prog = String::from(
        "#include <stdint.h>\n#include <stddef.h>\n#include <stdio.h>\n#include <string.h>\n",
    );
    for (d, src) in [
        ("KF_DIRS", &cu),
        ("KF_MAX_PDB", &h),
        ("KF_MAX_RESET", &h),
        ("KF_MAX_PDB_L", &h),
        ("KF_MAX_SLOTS", &h),
    ] {
        prog.push_str(&format!("#define {d} {}\n", extract_define(src, d)));
    }
    for name in ["KfAck", "KfLayout"] {
        prog.push_str(&extract_struct(&h, name));
        prog.push('\n');
    }
    prog.push_str(&extract_struct(&cu, "KfDev"));
    prog.push('\n');
    prog.push_str(
        "static void dump(const void *p, size_t n){const unsigned char *b=(const unsigned char*)p;\
         for(size_t i=0;i<n;i++)printf(\"%02x\",b[i]);printf(\"\\n\");}\n\
         int main(void){KfDev D; KfLayout L; KfAck A; memset(&D,0,sizeof D); memset(&L,0,sizeof L); \
         memset(&A,0,sizeof A);\n",
    );
    prog.push_str(&set);
    prog.push_str("dump(&D,sizeof D); dump(&L,sizeof L); dump(&A,sizeof A); return 0;}\n");
    let dir = std::env::temp_dir().join(format!("kf_t17_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temp dir");
    let (src, bin) = (dir.join("t17.cpp"), dir.join("t17"));
    std::fs::write(&src, &prog).expect("write the probe");
    let out = Command::new("g++")
        .args(["-std=c++14", "-O0", "-w", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .expect("g++ must be present — a check that cannot RUN is not a check that passed");
    assert!(
        out.status.success(),
        "{}\n{prog}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = Command::new(&bin).output().expect("run the probe");
    let text = String::from_utf8_lossy(&run.stdout).to_string();
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3, "{text}");
    assert_eq!(
        lines[0],
        hex(&dev.encode()),
        "KfDev: the encoder and the C compiler disagree"
    );
    assert_eq!(
        lines[1],
        hex(&lay.encode()),
        "KfLayout: the encoder and the C compiler disagree"
    );
    assert_eq!(
        lines[2],
        hex(&ack.encode()),
        "KfAck: the encoder and the C compiler disagree"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
