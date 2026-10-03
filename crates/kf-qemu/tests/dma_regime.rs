// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ ABI 15 (`docs/design/V3_VIOMMU.md` §4.2): the device's DMA regime across the C/Rust seam.
//!
//! 1. **The wire table.** The C device's `KF3_DMA_*` values (`kf3.h`) and Rust's reading of them
//!    (`kf_arch::dma::DmaRegime::from_wire`) are ONE table. `tests/wire_mirror.rs` checks the entry
//!    point `kf3_dma_regime(void *, uint32_t)`; the values that cross it are `#define`s that mirror
//!    never reads, and a drift there is silent at every other layer. The dangerous direction is
//!    fail-open: a translating (IOVA) regime arriving as a number Rust reads as admitting.
//! 2. **The C rules** (`kf3_dma.h`): how the sections of the device's DMA address space and the
//!    amd-iommu's properties become a regime. Compiled and RUN here with the system C compiler, the
//!    way `tests/gop_rom_knobs.rs` runs `kf3_gop.h`: kf3.c itself is not compiled by CI.

use kf_arch::dma::DmaRegime;
use std::process::Command;

/// The wire table: each `KF3_DMA_<name>` and the regime Rust must read its value as.
const TABLE: [(&str, DmaRegime); 5] = [
    ("DIRECT", DmaRegime::Direct),
    ("IDENTITY", DmaRegime::Identity),
    ("TRANSLATING", DmaRegime::Translating),
    ("UNTRACKED", DmaRegime::Untracked),
    ("BLOCKED", DmaRegime::Blocked),
];

/// Every `#define KF3_DMA_<name> <n>u` in `header`, in order.
fn defines(header: &str) -> Vec<(String, u32)> {
    header
        .lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("#define KF3_DMA_")?;
            let mut words = rest.split_whitespace();
            let name = words.next()?.to_string();
            let value = words.next()?.trim_end_matches(['u', 'U']).parse().ok()?;
            Some((name, value))
        })
        .collect()
}

/// `Ok` when `header` defines exactly the table's names, each with a value Rust reads as that
/// regime.
fn check(header: &str) -> Result<(), String> {
    let got = defines(header);
    let names: Vec<&str> = got.iter().map(|(n, _)| n.as_str()).collect();
    let want: Vec<&str> = TABLE.iter().map(|(n, _)| *n).collect();
    if names != want {
        return Err(format!(
            "KF3_DMA_* names differ (C left, Rust right): {names:?} vs {want:?}"
        ));
    }
    for ((name, value), (_, regime)) in got.iter().zip(TABLE) {
        let read = DmaRegime::from_wire(*value);
        if read != regime {
            return Err(format!(
                "KF3_DMA_{name} = {value}, which Rust reads as {read:?}, not {regime:?}"
            ));
        }
    }
    Ok(())
}

fn kf3_h() -> String {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(root.join("qemu/hw/misc/kf3/kf3.h")).unwrap()
}

#[test]
fn the_c_dma_regime_values_are_the_ones_rust_reads() {
    let header = kf3_h();
    assert_eq!(check(&header), Ok(()));
    let admitting: Vec<String> = defines(&header)
        .into_iter()
        .filter(|(_, v)| DmaRegime::from_wire(*v).admits())
        .map(|(n, _)| n)
        .collect();
    assert_eq!(
        admitting,
        ["DIRECT", "IDENTITY"],
        "only these two may read a device address as a GPA"
    );
}

/// ⊘ The check above must be able to FAIL: one known-positive per kind of drift it claims to catch.
#[test]
fn a_drifted_dma_regime_table_is_refused() {
    let header = kf3_h();
    assert_eq!(check(&header), Ok(()));
    let drift = [
        // the fail-open swap: a translating guest's value read as identity
        (
            "#define KF3_DMA_TRANSLATING 2u",
            "#define KF3_DMA_TRANSLATING 1u",
        ),
        // a value past the table (Rust reads it as Translating, not Untracked)
        (
            "#define KF3_DMA_UNTRACKED 3u",
            "#define KF3_DMA_UNTRACKED 7u",
        ),
        // a regime dropped from the C side
        ("#define KF3_DMA_BLOCKED 4u", "#define KF3_DMA_GONE 4u"),
    ];
    for (was, now) in drift {
        assert!(header.contains(was), "kf3.h no longer reads `{was}`");
        assert!(
            check(&header.replace(was, now)).is_err(),
            "the drifted `{now}` was accepted"
        );
    }
    let extra = header.replace(
        "#define KF3_DMA_BLOCKED 4u",
        "#define KF3_DMA_BLOCKED 4u\n#define KF3_DMA_SHADOWED 5u",
    );
    assert!(check(&extra).is_err(), "a regime on the C side only");
}

/// Run `cases` (C expressions over `kf3_dma.h` that yield an unsigned value) and return each value.
fn run_c(cases: &[&str]) -> Vec<u32> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("kf3-dma-rules-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let mut program =
        String::from("#include <stdio.h>\n#include \"kf3_dma.h\"\nint main(void) {\n");
    for c in cases {
        program.push_str(&format!("    printf(\"%u\\n\", (unsigned)({c}));\n"));
    }
    program.push_str("    return 0;\n}\n");
    let source = dir.join("rules.c");
    let binary = dir.join("rules");
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
        "kf3_dma.h does not compile alone:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let out = Command::new(&binary).output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success());
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| l.parse().unwrap())
        .collect()
}

/// `kf3_dma_classify(untracked, iommu_sections, ram_sections)`, read through Rust's wire table.
#[test]
fn the_c_classifier_needs_positive_evidence_for_identity() {
    let rows: [(&str, DmaRegime); 8] = [
        // a translating vIOMMU: its IOMMU region is in view (with or without RAM beside it)
        ("kf3_dma_classify(false, 1, 0)", DmaRegime::Translating),
        ("kf3_dma_classify(false, 2, 5)", DmaRegime::Translating),
        // a pass-through view of system memory, and nothing translating: identity
        ("kf3_dma_classify(false, 0, 3)", DmaRegime::Identity),
        // ⊘ neither: the half-way point of a mode switch (one region off, the other not yet on)
        ("kf3_dma_classify(false, 0, 0)", DmaRegime::Blocked),
        // untracked wins over whatever the sections say — the model does not follow the guest
        ("kf3_dma_classify(true, 0, 3)", DmaRegime::Untracked),
        ("kf3_dma_classify(true, 1, 0)", DmaRegime::Untracked),
        ("kf3_dma_classify(true, 0, 0)", DmaRegime::Untracked),
        // the largest counts still classify (no narrowing on the way)
        (
            "kf3_dma_classify(false, 0, 0xffffffffu)",
            DmaRegime::Identity,
        ),
    ];
    let cases: Vec<&str> = rows.iter().map(|(c, _)| *c).collect();
    let got = run_c(&cases);
    for ((case, want), wire) in rows.iter().zip(got) {
        assert_eq!(DmaRegime::from_wire(wire), *want, "{case} = {wire}");
    }
}

/// A switch the way every vIOMMU makes it — one region disabled, then the other enabled, each its
/// own commit — never shows IDENTITY on the way into translation, nor on the way out of it before
/// the RAM view is actually back.
#[test]
fn a_mode_switch_passes_through_blocked_never_a_false_identity() {
    // (iommu, ram) after each commit: identity -> translating -> identity.
    let steps = [(0, 4), (0, 0), (1, 0), (0, 0), (0, 4)];
    let cases: Vec<String> = steps
        .iter()
        .map(|(i, r)| format!("kf3_dma_classify(false, {i}, {r})"))
        .collect();
    let refs: Vec<&str> = cases.iter().map(String::as_str).collect();
    let got: Vec<DmaRegime> = run_c(&refs).into_iter().map(DmaRegime::from_wire).collect();
    assert_eq!(
        got,
        [
            DmaRegime::Identity,
            DmaRegime::Blocked,
            DmaRegime::Translating,
            DmaRegime::Blocked,
            DmaRegime::Identity,
        ]
    );
    let admitted: Vec<bool> = got.iter().map(|r| r.admits()).collect();
    assert_eq!(admitted, [true, false, false, false, true]);
}

/// `kf3_dma_amd_untracked(found, ambiguous, readable, dma_remap, dma_translation)`.
#[test]
fn an_amd_iommu_qemu_does_not_follow_is_untracked_and_unreadable_state_refuses() {
    let rows: [(&str, bool); 8] = [
        // no amd-iommu at all: nothing to say (the sections decide)
        (
            "kf3_dma_amd_untracked(false, false, true, false, true)",
            false,
        ),
        // QEMU's default (dma-remap=off) with translation advertised: the guest may translate
        // while the model passes through
        (
            "kf3_dma_amd_untracked(true, false, true, false, true)",
            true,
        ),
        // dma-remap=on: the model follows the guest's device-table entries, the sections decide
        (
            "kf3_dma_amd_untracked(true, false, true, true, true)",
            false,
        ),
        // dma-translation=off: the guest is told not to translate
        (
            "kf3_dma_amd_untracked(true, false, true, false, false)",
            false,
        ),
        (
            "kf3_dma_amd_untracked(true, false, true, true, false)",
            false,
        ),
        // ⊘ fail-closed: two of them, or properties this rule cannot read
        (
            "kf3_dma_amd_untracked(false, true, true, true, false)",
            true,
        ),
        (
            "kf3_dma_amd_untracked(true, false, false, true, false)",
            true,
        ),
        (
            "kf3_dma_amd_untracked(true, true, false, false, false)",
            true,
        ),
    ];
    let cases: Vec<&str> = rows.iter().map(|(c, _)| *c).collect();
    for ((case, want), got) in rows.iter().zip(run_c(&cases)) {
        assert_eq!(got != 0, *want, "{case}");
    }
}
