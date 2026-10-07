// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ `gop=on` and QEMU's own ROM properties (`qemu/hw/misc/kf3/kf3_gop.h`, `docs/design/V3_DISPLAY.md`
//! §4.11.12). kf3.c is not compiled by CI, so its one rule about them lives in a QEMU-free header that
//! this test compiles and RUNS with the system C compiler, the way `tests/wire_mirror.rs` compiles the
//! seam.
//!
//! ⊘ Before 2026-10-03 (`v3-gop`) kf3.c warned and carried on: `gop=on` with `romfile=` or `rombar=0`
//! registered no ROM while the Rust half — realized with `gop=1` — kept the whole boot-display posture,
//! and `romfile=""` was not seen at all. Each of the three rows marked `refused` below was `NULL`
//! (carry on) then.

use std::process::Command;

/// Run `cases` (C expressions calling `kf3_gop_rom_conflict`) and return, per case, the reason it
/// gave or `None`.
fn run(cases: &[&str]) -> Vec<Option<String>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("kf3-gop-knobs-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let mut program = String::from(
        "#include <stdio.h>\n#include \"kf3_gop.h\"\nint main(void) {\n    const char *w;\n",
    );
    for c in cases {
        program.push_str(&format!(
            "    w = {c};\n    printf(\"%s\\n\", w ? w : \"(none)\");\n"
        ));
    }
    program.push_str("    return 0;\n}\n");
    let source = dir.join("knobs.c");
    let binary = dir.join("knobs");
    std::fs::write(&source, program).unwrap();
    let build = Command::new("cc")
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-I"])
        .arg(root.join("qemu/hw/misc/kf3"))
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "kf3_gop.h does not compile alone:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let out = Command::new(&binary).output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success());
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| (l != "(none)").then(|| l.to_owned()))
        .collect()
}

#[test]
fn gop_on_refuses_every_rom_property_that_asks_for_another_rom_or_none() {
    let r = run(&[
        // gop=on, QEMU's defaults (no romfile, rombar -1) and an explicit rombar=1: served
        "kf3_gop_rom_conflict(true, NULL, -1)",
        "kf3_gop_rom_conflict(true, NULL, 1)",
        // refused: another ROM; romfile="" (QEMU's "no option ROM"); rombar=0 (no ROM BAR)
        "kf3_gop_rom_conflict(true, \"efi-virtio.rom\", -1)",
        "kf3_gop_rom_conflict(true, \"\", -1)",
        "kf3_gop_rom_conflict(true, NULL, 0)",
        // gop=off: kf3 serves no ROM, and QEMU's properties mean what they always meant
        "kf3_gop_rom_conflict(false, \"efi-virtio.rom\", 0)",
        "kf3_gop_rom_conflict(false, \"\", -1)",
        "kf3_gop_rom_conflict(false, NULL, 0)",
    ]);
    assert_eq!(r[0], None);
    assert_eq!(r[1], None);
    let refused = |i: usize, word: &str| {
        let why = r[i]
            .as_deref()
            .unwrap_or_else(|| panic!("case {i} must be refused"));
        assert!(why.contains(word), "case {i}: {why}");
    };
    refused(2, "romfile=");
    refused(3, "romfile=\"\"");
    refused(4, "rombar=0");
    assert_eq!(&r[5..], &[None, None, None]);
}
