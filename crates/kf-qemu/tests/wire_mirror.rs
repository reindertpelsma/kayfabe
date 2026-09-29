//! Compile the repository-owned C seam and compare every carried Rust field's layout.
//! This uses the actual QEMU header, needs no QEMU build, and never opens a GPU.

use kf_qemu::ffi_unsafe::{KF3_ABI, Kf3Identity, Kf3Region};
use std::collections::BTreeMap;
use std::mem::{align_of, offset_of, size_of};
use std::process::Command;

#[test]
fn the_c_header_and_rust_seam_have_identical_layouts() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let header = std::fs::read_to_string(root.join("qemu/hw/misc/kf3/kf3.h")).unwrap();
    let rust = include_str!("../src/ffi_unsafe.rs");
    let names = ["Kf3Identity", "Kf3Region"];
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
            $(value!(format!("{name}.{}", $cfield), format!("offsetof({name}, {})", $cfield), offset_of!($ty, $field));)+
        }};
    }
    layout!(Kf3Identity, [vendor => "vendor", device => "device",
        subsystem_vendor => "subsystem_vendor", subsystem => "subsystem",
        class => "class_code", revision => "revision", pad => "pad", bar0_bytes => "bar0_bytes"]);
    layout!(Kf3Region, [bar => "bar", how => "how", pad => "pad", base => "base", len => "len"]);
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
