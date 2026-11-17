//! ★ The committed utilisation layouts (`src/utilization_generated.rs`, receipts in
//! `traces/util_layout_20261117/`) are what a FRESH compile of `tools/util-probe/probe.c` prints against the two
//! PINNED oracle trees (`third_party/ogkm-580` = 580.159.04 and `third_party/ogkm-595` = 595.84, fetched by
//! `scripts/ci/fetch_oracles.sh`), and the Rust is what the receipts emit. `STATUS: LIVE, 2026-11-17`
//! (`docs/design/V3_UTILIZATION_FLAG.md`).
//!
//! The same gate as `kf-chip/tests/hwref_is_current.rs`: with `KF_REQUIRE_ORACLES` unset a missing tree or
//! compiler SKIPS (printed, not a pass); set to anything but empty or `0` it FAILS. The other 16 receipts are
//! probes of tags this job does not hold; they are checked by hash (`generate.py emit`), not recompiled.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

fn announce(line: &str) {
    let _ = writeln!(std::io::stderr(), "{line}");
}

fn tree(var: &str, dir: &str, marker: &str) -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    [
        std::env::var_os(var).map(PathBuf::from),
        Some(root.join("third_party").join(dir)),
    ]
    .into_iter()
    .flatten()
    .find(|p| p.join(marker).exists())
}

fn required(value: Option<&str>) -> bool {
    !matches!(value, None | Some("" | "0"))
}

#[test]
fn the_required_flag_fails_closed_on_a_typo() {
    assert!(!required(None));
    assert!(!required(Some("")));
    assert!(!required(Some("0")));
    for yes in ["1", "true", "yes", "00", " 0"] {
        assert!(required(Some(yes)), "{yes:?}");
    }
}

#[test]
fn the_committed_layouts_are_a_fresh_derivation_from_the_pinned_oracles() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let req = required(std::env::var("KF_REQUIRE_ORACLES").ok().as_deref());
    let marker = "src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080perf.h";
    let o580 = tree("KF_OGKM_580", "ogkm-580", marker);
    let o595 = tree("KAYFABE_OGKM_595", "ogkm-595", marker);
    let tools = Command::new("gcc").arg("--version").output().is_ok()
        && Command::new("python3").arg("--version").output().is_ok();
    let missing = if o580.is_none() {
        Some("no ogkm-580 tree (set KF_OGKM_580 or run scripts/ci/fetch_oracles.sh)")
    } else if o595.is_none() {
        Some("no ogkm-595 tree (set KAYFABE_OGKM_595 or run scripts/ci/fetch_oracles.sh)")
    } else if !tools {
        Some("no gcc or python3")
    } else {
        None
    };
    if let Some(why) = missing {
        if req {
            panic!("KF_REQUIRE_ORACLES is set and the oracle cannot run: {why}");
        }
        announce(&format!(
            "ORACLE-GATE: SKIPPED utilization_layout_is_current: {why} — NOT a pass"
        ));
        return;
    }
    announce(&format!(
        "ORACLE-GATE: RAN utilization_layout_is_current required={req}"
    ));
    let out = Command::new("python3")
        .arg(root.join("tools/util-probe/generate.py"))
        .arg("check")
        .arg("--ogkm-580")
        .arg(o580.expect("checked"))
        .arg("--ogkm-595")
        .arg(o595.expect("checked"))
        .output()
        .expect("python3 runs");
    assert!(
        out.status.success(),
        "the committed utilisation layouts are stale: run `python3 tools/util-probe/generate.py all --work DIR`:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
