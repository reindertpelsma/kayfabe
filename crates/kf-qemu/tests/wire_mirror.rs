//! Compile the repository-owned C seam and compare every carried Rust field's layout.
//! This uses the actual QEMU header, needs no QEMU build, and never opens a GPU.

use kf_qemu::ffi_unsafe::{KF3_ABI, Kf3Frame, Kf3Identity, Kf3Region};
use std::collections::{BTreeMap, BTreeSet};
use std::mem::{align_of, offset_of, size_of};
use std::process::Command;

/// C source without its comments (a comment may contain `;`, `(` or an entry point's name).
fn strip_c_comments(src: &str) -> String {
    let mut clean = String::new();
    let mut rest = src;
    while let Some((before, comment)) = rest.split_once("/*") {
        clean.push_str(before);
        rest = comment.split_once("*/").expect("unterminated C comment").1;
    }
    clean.push_str(rest);
    clean
        .lines()
        .map(|line| line.split_once("//").map_or(line, |(code, _)| code))
        .collect::<Vec<_>>()
        .join("\n")
}

fn c_fields(header: &str, name: &str) -> Vec<String> {
    let body = header
        .split(&format!("typedef struct {name} {{"))
        .nth(1)
        .unwrap()
        .split('}')
        .next()
        .unwrap();
    // Drop comments before splitting declarations (a comment may contain ';').
    strip_c_comments(body)
        .split(';')
        .filter(|row| !row.trim().is_empty())
        .flat_map(|row| {
            let variables = row.trim().split_once(char::is_whitespace).unwrap().1;
            variables
                .split(',')
                .map(|field| field.trim().split('[').next().unwrap().to_string())
        })
        .collect()
}

#[test]
fn c_field_census_sees_members_even_when_they_could_fit_in_padding() {
    let header = "typedef struct X { uint8_t a, pad[3]; /* ; */ uint32_t b; uint8_t extra; } X;";
    assert_eq!(c_fields(header, "X"), ["a", "pad", "b", "extra"]);
}

#[test]
fn the_c_header_and_rust_seam_have_identical_layouts() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let header = std::fs::read_to_string(root.join("qemu/hw/misc/kf3/kf3.h")).unwrap();
    let rust = include_str!("../src/ffi_unsafe.rs");
    let names = ["Kf3Identity", "Kf3Region", "Kf3Frame"];
    let declared: Vec<_> = rust
        .split("#[repr(C)]")
        .skip(1)
        .map(|part| {
            part.split("pub struct ")
                .nth(1)
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap()
        })
        .collect();
    assert_eq!(
        declared, names,
        "a new Rust layout needs a C differential row"
    );
    let c_names: Vec<_> = header
        .lines()
        .filter_map(|line| {
            line.strip_prefix("typedef struct ")
                .map(|s| s.split_whitespace().next().unwrap())
        })
        .collect();
    assert_eq!(
        c_names, names,
        "a new C layout needs a Rust differential row"
    );
    let mut expected = BTreeMap::new();
    let mut program = String::from("#include <stdio.h>\n#include \"kf3.h\"\nint main(void) {\n");
    macro_rules! value {
        ($key:expr, $expr:expr, $value:expr) => {{
            program.push_str(&format!(
                "printf(\"{} %zu\\n\", (size_t)({}));\n",
                $key, $expr
            ));
            expected.insert($key.to_string(), $value as usize);
        }};
    }
    macro_rules! layout {
        ($ty:ident, [$($field:ident => $cfield:literal),+ $(,)?]) => {{
            let name = stringify!($ty);
            value!(format!("{name}.size"), format!("sizeof({name})"), size_of::<$ty>());
            value!(format!("{name}.align"), format!("_Alignof({name})"), align_of::<$ty>());
            let body = rust.split(&format!("pub struct {name} {{")).nth(1).unwrap().split('}').next().unwrap();
            let fields: Vec<_> = body.lines().filter_map(|line| line.trim().strip_prefix("pub ")
                .and_then(|rest| rest.split_once(':').map(|(name, _)| name))).collect();
            assert_eq!(fields, [$(stringify!($field)),+], "uncovered Rust field");
            assert_eq!(c_fields(&header, name), [$($cfield),+], "uncovered C field");
            $(value!(format!("{name}.{}", $cfield), format!("offsetof({name}, {})", $cfield), offset_of!($ty, $field));)+
        }};
    }
    layout!(Kf3Identity, [vendor => "vendor", device => "device",
        subsystem_vendor => "subsystem_vendor", subsystem => "subsystem",
        class => "class_code", revision => "revision", pad => "pad", bar0_bytes => "bar0_bytes"]);
    layout!(Kf3Region, [bar => "bar", how => "how", pad => "pad", base => "base", len => "len"]);
    layout!(Kf3Frame, [data => "data", width => "width", height => "height", stride => "stride",
        format => "format", serial => "serial"]);
    value!("abi", "KF3_ABI", KF3_ABI);
    program.push_str("return 0; }\n");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("kf3-wire-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let source = directory.join("layout.c");
    let binary = directory.join("layout");
    std::fs::write(&source, program).unwrap();
    let build = Command::new("cc")
        .args(["-std=c11", "-Wall", "-Werror", "-I"])
        .arg(root.join("qemu/hw/misc/kf3"))
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let output = Command::new(&binary).output().unwrap();
    assert!(output.status.success());
    let actual: BTreeMap<String, usize> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| {
            let (key, value) = line.split_once(' ').unwrap();
            (key.to_string(), value.parse().unwrap())
        })
        .collect();
    assert_eq!(actual, expected, "QEMU/Rust seam ABI drift");
}

/// One `extern "C"` signature as Rust spells it: the parameter types and the return type.
#[derive(Debug)]
struct RustSig {
    params: Vec<String>,
    ret: String,
}

/// `a: T, b: U,\n) -> R` (everything after the opening parenthesis) → its types.
fn rust_sig(after_paren: &str) -> RustSig {
    let (params, ret) = after_paren
        .rsplit_once(')')
        .expect("unclosed parameter list");
    RustSig {
        params: params
            .split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(|p| {
                p.split_once(':')
                    .expect("an unnamed parameter")
                    .1
                    .trim()
                    .to_string()
            })
            .collect(),
        ret: ret
            .trim()
            .strip_prefix("->")
            .map_or("()", str::trim)
            .to_string(),
    }
}

/// Every entry point the archive exports (`no_mangle` in `ffi_unsafe.rs`), by name.
fn rust_entry_points(src: &str) -> BTreeMap<String, RustSig> {
    src.split("(no_mangle)]")
        .skip(1)
        .map(|item| {
            let head = item
                .split_once('{')
                .expect("an entry point without a body")
                .0;
            let (name, rest) = head.split_once('(').unwrap();
            let name = name.rsplit("fn ").next().unwrap().trim().to_string();
            (name, rust_sig(rest))
        })
        .collect()
}

/// Every C callback type alias (`pub type XFn = … extern "C" fn(…)`) in `raw_unsafe.rs`.
fn rust_callbacks(src: &str) -> BTreeMap<String, RustSig> {
    src.split("pub type ")
        .skip(1)
        .filter_map(|item| {
            let (name, def) = item.split_once('=')?;
            let (_, rest) = def.split_once(';')?.0.split_once("extern \"C\" fn(")?;
            Some((name.trim().to_string(), rust_sig(rest)))
        })
        .collect()
}

/// A Rust FFI type in C. ⊘ Unknown spellings panic: the mirror never guesses a C type.
fn c_type(rust: &str) -> String {
    let t = rust.trim();
    if let Some(pointee) = t.strip_prefix("*mut") {
        return format!("{} *", c_type(pointee));
    }
    if let Some(pointee) = t.strip_prefix("*const") {
        return format!("const {} *", c_type(pointee));
    }
    // A nullable C callback: `Option<path::XFn>` is kf3.h's `Kf3XFn`.
    if let Some(alias) = t.strip_prefix("Option<").and_then(|s| s.strip_suffix('>')) {
        return format!("Kf3{}", alias.rsplit("::").next().unwrap());
    }
    let t = t.rsplit("::").next().unwrap();
    match t {
        "u8" => "uint8_t",
        "u16" => "uint16_t",
        "u32" => "uint32_t",
        "u64" => "uint64_t",
        "i32" => "int32_t",
        "i64" => "int64_t",
        "usize" => "size_t",
        "c_void" | "()" => "void",
        "c_char" => "char",
        "Kf3Identity" | "Kf3Region" | "Kf3Frame" => t,
        other => panic!("the mirror cannot spell the Rust FFI type `{other}` in C — add it"),
    }
    .to_string()
}

fn c_params(sig: &RustSig) -> String {
    if sig.params.is_empty() {
        return "void".into();
    }
    sig.params
        .iter()
        .map(|p| c_type(p))
        .collect::<Vec<_>>()
        .join(", ")
}

/// kf3.h, and C that RE-DECLARES every Rust entry point and callback type from its Rust signature,
/// plus the two name censuses (C, Rust) for entry points and for callback types.
struct Seam {
    header: String,
    redeclared: String,
    c_entry: BTreeSet<String>,
    rust_entry: BTreeSet<String>,
    c_callbacks: BTreeSet<String>,
    rust_callbacks: BTreeSet<String>,
}

fn seam() -> Seam {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let header = std::fs::read_to_string(root.join("qemu/hw/misc/kf3/kf3.h")).unwrap();
    let exports = rust_entry_points(include_str!("../src/ffi_unsafe.rs"));
    let aliases = rust_callbacks(include_str!("../src/raw_unsafe.rs"));
    let clean = strip_c_comments(&header);
    let ident = |at: usize| -> String {
        clean[at..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect()
    };
    // `kf3_x(` not preceded by an identifier character: a declaration (comments are gone).
    let c_entry = clean
        .match_indices("kf3_")
        .filter(|(at, _)| {
            !clean[..*at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        .map(|(at, _)| (at, ident(at)))
        .filter(|(at, name)| clean[at + name.len()..].trim_start().starts_with('('))
        .map(|(_, name)| name)
        .collect();
    // `(*Kf3XFn)(`: a function-pointer typedef.
    let c_callbacks = clean
        .match_indices("(*Kf3")
        .map(|(at, _)| ident(at + 2))
        .collect();
    // The callback types the exports take (`Option<…::XFn>` → `Kf3XFn`), each from its Rust alias.
    let mut rust_callbacks = BTreeSet::new();
    let mut redeclared = String::new();
    for sig in exports.values() {
        for p in &sig.params {
            let Some(path) = p.strip_prefix("Option<").and_then(|s| s.strip_suffix('>')) else {
                continue;
            };
            let alias = path.rsplit("::").next().unwrap();
            let c = c_type(p);
            if rust_callbacks.insert(c.clone()) {
                let a = aliases
                    .get(alias)
                    .unwrap_or_else(|| panic!("no Rust alias `{alias}` for `{c}`"));
                redeclared.push_str(&format!(
                    "typedef {} (*{c})({});\n",
                    c_type(&a.ret),
                    c_params(a)
                ));
            }
        }
    }
    for (name, sig) in &exports {
        redeclared.push_str(&format!(
            "{} {name}({});\n",
            c_type(&sig.ret),
            c_params(sig)
        ));
    }
    Seam {
        header,
        redeclared,
        c_entry,
        rust_entry: exports.keys().cloned().collect(),
        c_callbacks,
        rust_callbacks,
    }
}

/// Compile (no link) `program` against kf3.h: `Err` carries the compiler's diagnostics.
fn c_accepts(program: &str) -> Result<(), String> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("kf3-seam-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let source = directory.join("seam.c");
    std::fs::write(&source, program).unwrap();
    let build = Command::new("cc")
        .args(["-std=c11", "-Wall", "-Werror", "-c", "-I"])
        .arg(root.join("qemu/hw/misc/kf3"))
        .arg(&source)
        .arg("-o")
        .arg(directory.join("seam.o"))
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&directory);
    if build.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&build.stderr).into_owned())
    }
}

/// ★ ABI 10 (`v3-mc22`, 2026-09-30): the ENTRY POINTS, not only the constant and the layouts.
/// `v3-display2` and `v3-ioeventfd` each shipped an ABI 9 naming a DIFFERENT function set, and a
/// constant-and-layout mirror is green for both. Here every Rust export and every callback alias
/// it takes is re-declared in C from its Rust signature after `#include "kf3.h"`: C refuses a
/// redeclaration whose type differs (`conflicting types`), so a swapped, retyped, added or dropped
/// parameter fails — and the name sets must be equal, so an entry point on one side only fails.
#[test]
fn the_c_header_and_rust_seam_declare_the_same_entry_points() {
    let s = seam();
    assert!(
        s.rust_entry.len() >= 24 && s.rust_entry.contains("kf3_abi_version"),
        "the Rust census found too little: {:?}",
        s.rust_entry
    );
    assert_eq!(
        s.c_entry, s.rust_entry,
        "an entry point on one side of the seam only (C left, Rust right)"
    );
    assert_eq!(
        s.c_callbacks, s.rust_callbacks,
        "a callback type on one side of the seam only (C left, Rust right)"
    );
    let program = format!("#include \"kf3.h\"\n{}", s.redeclared);
    if let Err(e) = c_accepts(&program) {
        panic!("the Rust signatures disagree with kf3.h:\n{e}\n--- redeclared:\n{program}");
    }
    assert!(s.header.contains(&format!("#define KF3_ABI {KF3_ABI}\n")));
}

/// ⊘ The check above must be able to FAIL: a known-positive per kind of drift it claims to catch.
#[test]
fn a_signature_that_drifted_from_kf3_h_is_refused() {
    let s = seam();
    let good = format!("#include \"kf3.h\"\n{}", s.redeclared);
    assert_eq!(c_accepts(&good), Ok(()));
    let drift = [
        // two parameters swapped (both integers: a caller would compile and pass them crossed)
        (
            "void kf3_doorbell_site(void *, uint64_t, uint32_t);",
            "void kf3_doorbell_site(void *, uint32_t, uint64_t);",
        ),
        // a narrowed return
        (
            "int64_t kf3_doorbell_page_offset(void *);",
            "int32_t kf3_doorbell_page_offset(void *);",
        ),
        // a callback parameter retyped
        (
            "typedef int32_t (*Kf3IoeventfdFn)(void *, uint64_t, uint32_t, uint64_t, int32_t, uint32_t);",
            "typedef int32_t (*Kf3IoeventfdFn)(void *, uint64_t, uint64_t, uint64_t, int32_t, uint32_t);",
        ),
        // a parameter added
        (
            "int32_t kf3_display_frame(void *, Kf3Frame *);",
            "int32_t kf3_display_frame(void *, Kf3Frame *, uint32_t);",
        ),
    ];
    for (was, now) in drift {
        assert!(
            good.contains(was),
            "the redeclaration no longer reads `{was}`:\n{good}"
        );
        assert!(
            c_accepts(&good.replace(was, now)).is_err(),
            "C accepted the drifted `{now}`"
        );
    }
}
