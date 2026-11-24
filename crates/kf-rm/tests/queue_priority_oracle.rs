//! ORACLE: the ids, struct layouts and values `kf_rm::queuepriority` serves, read out of the ogkm headers
//! at both pinned trees (580.159.04 and 595.84), so a hand-typed constant cannot drift from NVIDIA's.
//! STATUS: LIVE, 2026-11-18. Skips (named, on stderr) when a tree is absent unless `KF_REQUIRE_ORACLES` is
//! set to anything but empty or `0`, which makes an absent tree a failure (the `stripped` CI job sets it).
//! Trees: `$KF_OGKM_580` or `third_party/ogkm-580`; `$KAYFABE_OGKM_595` or `third_party/ogkm-595`.

use kf_rm::queuepriority as q;
use std::path::{Path, PathBuf};

fn root(env: &str, rel: &str) -> PathBuf {
    std::env::var_os(env).map(PathBuf::from).unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel)
    })
}

fn header(tree: &Path, rel: &str) -> Option<String> {
    std::fs::read_to_string(tree.join("src/common/sdk/nvidia/inc/ctrl").join(rel)).ok()
}

/// The value of `#define NAME (0x..)` or `#define NAME 0x..` (first match, comments cut).
fn define(h: &str, name: &str) -> u32 {
    let l = h
        .lines()
        .find(|l| l.split_whitespace().take(2).eq(["#define", name]))
        .unwrap_or_else(|| panic!("no #define {name}"));
    let v = l
        .split_whitespace()
        .nth(2)
        .unwrap()
        .trim_matches(['(', ')']);
    u32::from_str_radix(v.trim_start_matches("0x"), 16).unwrap_or_else(|_| panic!("{l}"))
}

/// The field names of `typedef struct NAME { ... } NAME;`, in order (all `NvU32` is checked).
fn u32_fields(h: &str, name: &str) -> Vec<String> {
    let start = h
        .find(&format!("typedef struct {name} {{"))
        .unwrap_or_else(|| panic!("no struct {name}"));
    let body = &h[start..];
    let body = &body[body.find('{').unwrap() + 1..body.find('}').unwrap()];
    body.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let l = l
                .strip_prefix("NvU32 ")
                .unwrap_or_else(|| panic!("{name}: {l} is not NvU32"));
            l.trim_end_matches(';').to_string()
        })
        .collect()
}

/// The field names of a struct whose fields are all `NvBool` (one byte each).
fn bool_fields(h: &str, name: &str) -> Vec<String> {
    let start = h
        .find(&format!("typedef struct {name} {{"))
        .unwrap_or_else(|| panic!("no struct {name}"));
    let body = &h[start..];
    let body = &body[body.find('{').unwrap() + 1..body.find('}').unwrap()];
    body.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let l = l
                .strip_prefix("NvBool ")
                .unwrap_or_else(|| panic!("{name}: {l} is not NvBool"));
            l.trim_end_matches(';').to_string()
        })
        .collect()
}

fn check(tree: &Path, label: &str) {
    let fifo = header(tree, "ctrl2080/ctrl2080fifo.h").expect("ctrl2080fifo.h");
    let tsg = header(tree, "ctrla06c.h").expect("ctrla06c.h");
    let chan = header(tree, "ctrla06f/ctrla06fgpfifo.h").expect("ctrla06fgpfifo.h");
    assert_eq!(
        define(&fifo, "NV2080_CTRL_CMD_FIFO_RUNLIST_SET_SCHED_POLICY"),
        q::SET_SCHED_POLICY,
        "{label}"
    );
    assert_eq!(
        define(&tsg, "NVA06C_CTRL_CMD_SET_INTERLEAVE_LEVEL"),
        q::TSG_SET_INTERLEAVE_LEVEL,
        "{label}"
    );
    assert_eq!(
        define(&chan, "NVA06F_CTRL_CMD_SET_INTERLEAVE_LEVEL"),
        q::CHANNEL_SET_INTERLEAVE_LEVEL,
        "{label}"
    );
    assert_eq!(
        define(&tsg, "NVA06C_CTRL_CMD_MAKE_REALTIME"),
        q::MAKE_REALTIME,
        "{label}"
    );
    assert_eq!(
        define(&chan, "NVA06F_CTRL_CMD_RESTART_RUNLIST"),
        q::RESTART_RUNLIST,
        "{label}"
    );
    // `NvBool` is `NvU8` (nvtypes.h), so one byte per field
    assert_eq!(
        bool_fields(&tsg, "NVA06C_CTRL_MAKE_REALTIME_PARAMS"),
        ["bRealtime"],
        "{label}"
    );
    assert_eq!(
        bool_fields(&tsg, "NVA06C_CTRL_MAKE_REALTIME_PARAMS").len(),
        q::MAKE_REALTIME_PARAMS_SIZE,
        "{label}"
    );
    assert_eq!(
        bool_fields(&chan, "NVA06F_CTRL_RESTART_RUNLIST_PARAMS"),
        ["bForceRestart", "bBypassWait"],
        "{label}"
    );
    assert_eq!(
        bool_fields(&chan, "NVA06F_CTRL_RESTART_RUNLIST_PARAMS").len(),
        q::RESTART_RUNLIST_PARAMS_SIZE,
        "{label}"
    );
    // params layouts: field count x 4 bytes
    let p = u32_fields(&fifo, "NV2080_CTRL_FIFO_RUNLIST_SET_SCHED_POLICY_PARAMS");
    assert_eq!(p, ["flags", "schedPolicy"], "{label}");
    assert_eq!(p.len() * 4, q::SCHED_POLICY_PARAMS_SIZE, "{label}");
    assert_eq!(
        u32_fields(&tsg, "NVA06C_CTRL_INTERLEAVE_LEVEL_PARAMS").len() * 4,
        q::INTERLEAVE_PARAMS_SIZE,
        "{label}"
    );
    assert_eq!(
        u32_fields(&chan, "NVA06F_CTRL_INTERLEAVE_LEVEL_PARAMS").len() * 4,
        q::INTERLEAVE_PARAMS_SIZE,
        "{label}"
    );
    // values
    assert_eq!(
        define(&fifo, "NV2080_CTRL_FIFO_RUNLIST_SCHED_POLICY_DEFAULT"),
        0,
        "{label}"
    );
    assert_eq!(
        define(
            &fifo,
            "NV2080_CTRL_FIFO_RUNLIST_SCHED_POLICY_CHANNEL_INTERLEAVED"
        ),
        1,
        "{label}"
    );
    assert_eq!(
        define(
            &fifo,
            "NV2080_CTRL_FIFO_RUNLIST_SCHED_POLICY_CHANNEL_INTERLEAVED_WDDM"
        ),
        q::SCHED_POLICY_MAX,
        "{label}"
    );
    assert!(
        fifo.contains("NV2080_CTRL_CMD_FIFO_RUNLIST_SET_SCHED_POLICY_FLAGS_RESTORE        0:0"),
        "{label}: bit 0:0"
    );
    assert_eq!(
        define(
            &fifo,
            "NV2080_CTRL_CMD_FIFO_RUNLIST_SET_SCHED_POLICY_FLAGS_RESTORE_TRUE"
        ),
        q::FLAGS_RESTORE,
        "{label}"
    );
    assert_eq!(
        define(&tsg, "NVA06C_CTRL_INTERLEAVE_LEVEL_LOW"),
        0,
        "{label}"
    );
    assert_eq!(
        define(&tsg, "NVA06C_CTRL_INTERLEAVE_LEVEL_MEDIUM"),
        1,
        "{label}"
    );
    assert_eq!(
        define(&tsg, "NVA06C_CTRL_INTERLEAVE_LEVEL_HIGH"),
        q::INTERLEAVE_LEVEL_MAX,
        "{label}"
    );
}

#[test]
fn the_served_ids_layouts_and_values_are_the_sdks_at_both_pinned_trees() {
    let required = !matches!(
        std::env::var("KF_REQUIRE_ORACLES").ok().as_deref(),
        None | Some("" | "0")
    );
    for (env, rel, label) in [
        ("KF_OGKM_580", "third_party/ogkm-580", "ogkm-580"),
        ("KAYFABE_OGKM_595", "third_party/ogkm-595", "ogkm-595"),
    ] {
        let tree = root(env, rel);
        if header(&tree, "ctrl2080/ctrl2080fifo.h").is_none() {
            let why = format!(
                "QUEUE-PRIORITY-ORACLE: SKIPPED {label} — no tree at {}",
                tree.display()
            );
            use std::io::Write;
            let _ = writeln!(std::io::stderr(), "{why}");
            assert!(!required, "KF_REQUIRE_ORACLES is set and {why}");
            continue;
        }
        check(&tree, label);
    }
}
