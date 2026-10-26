//! Compile the actual CUDA leaf classifier as C++ and compare with the Rust classifier.

use kf_cuda::abi::{KfFormat, kf_format_ver2, kf_format_ver3};
use kf_mem::sparse::{LeafState, classify_leaf};
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

fn definition(src: &str, name: &str) -> String {
    let start = src.find(name).expect("named CUDA definition");
    let body = src[start..].find('{').unwrap() + start;
    let mut depth = 0;
    for (i, ch) in src[body..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return src[start..body + i + 1].to_owned();
                }
            }
            _ => {}
        }
    }
    panic!("unterminated definition {name}")
}

fn classifier() -> &'static PathBuf {
    static EXE: OnceLock<PathBuf> = OnceLock::new();
    EXE.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let src = std::fs::read_to_string(root.join("cuda/walk/kf_walk.cu")).unwrap();
        let mut cpp = String::from(
            "#include <stdint.h>\n#include <stdlib.h>\n#include <stdio.h>\n\
             #define KF_DIRS 5\n#define KF_TBL_VER2 2\n\
             #define __device__\n#define __forceinline__ inline\n",
        );
        for name in ["struct KfField {", "struct KfDir {", "struct KfFormat {"] {
            cpp.push_str(&definition(&src, name));
            cpp.push_str(";\n");
        }
        for name in [
            "uint64_t kf_field(",
            "bool kf_valid(",
            "bool kf_slot_sparse(",
        ] {
            cpp.push_str(&definition(&src, name));
            cpp.push('\n');
        }
        cpp.push_str(
            "int main(int argc, char **v) {\n\
             KfFormat f = {}; f.table_version = strtoul(v[1], 0, 0);\n\
             f.valid_bit = 0; f.bit_volatile = 3; f.bit_privilege = 5;\n\
             f.pcf.lo = 3; f.pcf.bits = 5; f.pcf_sparse = 1;\n\
             printf(\"%d\\n\", kf_slot_sparse(f, strtoull(v[2], 0, 0))); }\n",
        );
        let dir = std::env::temp_dir().join(format!("kf-spr1-decode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("decode.cpp");
        let exe = dir.join("decode");
        std::fs::write(&file, cpp).unwrap();
        let out = Command::new("g++")
            .args(["-std=c++14", "-o"])
            .arg(&exe)
            .arg(&file)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        exe
    })
}

fn agrees(f: KfFormat, raw: u64) {
    let out = Command::new(classifier())
        .arg(f.table_version.to_string())
        .arg(raw.to_string())
        .output()
        .unwrap();
    assert!(out.status.success());
    let actual = String::from_utf8(out.stdout).unwrap().trim() == "1";
    assert_eq!(
        actual,
        classify_leaf(&f, raw) == LeafState::Sparse,
        "raw={raw:#x}"
    );
}

#[test]
fn nv4k_is_not_a_sparse_leaf() {
    agrees(kf_format_ver2(), 0x28);
}

#[test]
fn nv4k_with_stale_payload_is_not_a_sparse_leaf() {
    agrees(kf_format_ver2(), 0x1234_0028);
}

#[test]
fn valid_volatile_is_not_a_sparse_leaf_on_either_format() {
    agrees(kf_format_ver2(), 9);
    agrees(kf_format_ver3(), 9);
}

#[test]
fn sparse_and_plain_invalid_match_on_both_formats() {
    for f in [kf_format_ver2(), kf_format_ver3()] {
        for raw in [0, 8, 0x20, 0x18] {
            agrees(f, raw);
        }
    }
}
