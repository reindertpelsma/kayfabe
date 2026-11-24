// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! (`docs/design/V3_GUEST_INFLUENCE_AUDIT.md` G-5): the control partition, GPU-free.
//!
//! Every control id kayfabe serves (`tools/api_boundaries/served.json`, derived from kayfabe's own
//! source) is in exactly one of two sets:
//! - **ROW**: a `kf_abi::hostabi::HOST_CONTROLS` row — a control kayfabe AUTHORS to the host, with a
//!   measured layout;
//! - **ANSWERED**: answered (or refused) by kayfabe's served chain and never sent to a host —
//!   committed explicitly in `tools/api_boundaries/answered_never_forwarded.txt`.
//!
//! And the source scan proves the second half of "never forwarded": every control id that any
//! `crates/kf-*` call site (outside the `kf-harness` operator tools) hands to a host control
//! ioctl resolves to a ROW, and the call sites whose id is not a constant are exactly the ones
//! listed in [`DYNAMIC_SITES`] (each takes its id from an authored table, never from a guest).
//! A new served id, a new host call site, or a host id without a row fails here by name.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use kf_abi::host_driver::HostDriverVersion;
use kf_abi::hostabi::{HOST_CONTROLS, HostAbi, HostAbiError};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn hex(s: &str) -> u32 {
    u32::from_str_radix(
        s.trim().trim_start_matches("0x").replace('_', "").as_str(),
        16,
    )
    .unwrap_or_else(|e| panic!("{s}: {e}"))
}

/// `ctrl_ids` of served.json (a flat list of quoted hex strings; read without a JSON crate).
fn served() -> BTreeSet<u32> {
    let text = std::fs::read_to_string(root().join("tools/api_boundaries/served.json"))
        .expect("served.json");
    let at = text.find("\"ctrl_ids\"").expect("ctrl_ids");
    let open = at + text[at..].find('[').expect("[");
    let close = open + text[open..].find(']').expect("]");
    text[open + 1..close]
        .split(',')
        .map(|s| s.trim().trim_matches('"'))
        .filter(|s| !s.is_empty())
        .map(hex)
        .collect()
}

fn answered() -> BTreeSet<u32> {
    let text =
        std::fs::read_to_string(root().join("tools/api_boundaries/answered_never_forwarded.txt"))
            .expect("answered_never_forwarded.txt");
    text.lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .map(hex)
        .collect()
}

fn rows() -> BTreeSet<u32> {
    HOST_CONTROLS.iter().map(|c| c.cmd).collect()
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).expect("dir").flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_sources(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Every `crates/kf-*/src` file except the operator tools (`kf-harness`).
fn product_sources() -> Vec<(String, String)> {
    let mut files = Vec::new();
    for e in std::fs::read_dir(root().join("crates"))
        .expect("crates")
        .flatten()
    {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with("kf-") && name != "kf-harness" {
            rust_sources(&e.path().join("src"), &mut files);
        }
    }
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let rel = p
                .strip_prefix(root())
                .unwrap_or(&p)
                .to_string_lossy()
                .replace("/../..", "");
            let text = std::fs::read_to_string(&p).expect("read");
            (rel, text)
        })
        .collect()
}

/// `const NAME: u32 = 0x…;` in one file.
fn file_constants(text: &str) -> BTreeMap<String, u32> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let l = line
            .trim_start()
            .trim_start_matches("pub(crate) ")
            .trim_start_matches("pub ");
        let Some(rest) = l.strip_prefix("const ") else {
            continue;
        };
        let Some((name, val)) = rest.split_once(": u32 = ") else {
            continue;
        };
        let val = val.trim_end_matches(';').trim();
        if val.starts_with("0x") && !val.contains(' ') {
            out.insert(name.to_string(), hex(val));
        }
    }
    out
}

/// Every name's values across the product sources (a name with two values resolves only in the
/// file that defines it).
fn constants(src: &[(String, String)]) -> BTreeMap<String, BTreeSet<u32>> {
    let mut out: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    for (_, text) in src {
        for (k, v) in file_constants(text) {
            out.entry(k).or_default().insert(v);
        }
    }
    out
}

/// The second argument of each call `name(` in `text` (the call's command id), with its line.
fn call_ids(text: &str, name: &str, arg: usize) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = text[at..].find(name) {
        let start = at + i;
        at = start + name.len();
        let before = &text[..start];
        if before.ends_with("fn ") || before.ends_with("fn  ") {
            continue;
        }
        let line_start = before.rfind('\n').map_or(0, |n| n + 1);
        if text[line_start..start].trim_start().starts_with("//") {
            continue;
        }
        let mut depth = 0i32;
        let mut args = vec![String::new()];
        for ch in text[at..].chars() {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' if depth == 0 => break,
                ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => {
                    args.push(String::new());
                    continue;
                }
                _ => {}
            }
            if let Some(a) = args.last_mut() {
                a.push(ch);
            }
        }
        let a: String = args
            .get(arg)
            .map(|s| s.split_whitespace().collect())
            .unwrap_or_default();
        out.push((before.matches('\n').count() + 1, a));
    }
    out
}

/// ★ The host-control call sites whose command is not a constant, each with where its id comes
/// from. A new one fails [`every_host_control_call_site_names_a_row`] until it is listed here.
const DYNAMIC_SITES: &[(&str, &str, &str)] = &[
    (
        "crates/kf-host/src/channel.rs",
        "cmd",
        "NVA06C_CTRL_CMD_BIND or NVA06F_CTRL_CMD_BIND, chosen by `first`",
    ),
    (
        "crates/kf-qemu/src/chan.rs",
        "cmd",
        "GSS_ENC_SESSION_ACQUIRE / _RELEASE (`enc_session_step`)",
    ),
    (
        "crates/kf-qemu/src/rmfacts.rs",
        "cmd",
        "the realize-time host queries of `kf_rm::hostquery` (authored)",
    ),
    (
        "crates/kf-qemu/src/utilhost.rs",
        "cmd",
        "`kf_abi::utilization::GPUMON_CMD`",
    ),
    (
        "crates/kf-host/src/precomp.rs",
        "cmd",
        "the startup PRECOMP plan, compiled per host version (`kf-host/src/precomp.rs`)",
    ),
    (
        "crates/kf-rm/src/hostquery.rs",
        "cmd",
        "authored hostquery constants",
    ),
    (
        "crates/kf-rm/src/hostquery.rs",
        "row.cmd",
        "`kf_abi::gssreplay::ROWS`",
    ),
    (
        "crates/kf-rm/src/hostquery.rs",
        "layout.cmd",
        "`kf_abi::fifoctl::LatencyLayout`",
    ),
    (
        "crates/kf-rm/src/hostquery.rs",
        "map_cmd",
        "authored GR FS / intr map constants",
    ),
    (
        "crates/kf-host/src/timer.rs",
        "kf_abi::timer::REGISTER_OFFSET",
        "a kf-abi alias, checked below",
    ),
    (
        "crates/kf-qemu/src/rmfacts.rs",
        "kf_abi::chanbudget::CONTROL",
        "a kf-abi alias, checked below",
    ),
];

#[test]
fn served_controls_are_partitioned_into_rows_and_answered() {
    let (served, rows, answered) = (served(), rows(), answered());
    let both: Vec<String> = answered
        .intersection(&rows)
        .map(|c| format!("{c:#010x}"))
        .collect();
    assert!(
        both.is_empty(),
        "ANSWERED ids that are also host rows: {both:?}"
    );
    let neither: Vec<String> = served
        .iter()
        .filter(|c| !rows.contains(c) && !answered.contains(c))
        .map(|c| format!("{c:#010x}"))
        .collect();
    assert!(
        neither.is_empty(),
        "served control ids in NEITHER set (classify each: a HOST_CONTROLS row, or \
         tools/api_boundaries/answered_never_forwarded.txt): {neither:?}"
    );
    let stale: Vec<String> = answered
        .difference(&served)
        .map(|c| format!("{c:#010x}"))
        .collect();
    assert!(stale.is_empty(), "ANSWERED ids no longer served: {stale:?}");
}

#[test]
fn every_host_control_call_site_names_a_row() {
    let src = product_sources();
    let consts = constants(&src);
    let rows = rows();
    let mut unlisted = Vec::new();
    let mut dynamic = BTreeSet::new();
    let mut resolved = 0usize;
    for (rel, text) in &src {
        let mut sites = Vec::new();
        for name in ["raw_control(", "raw_control_nested(", "raw_control_opaque("] {
            sites.extend(call_ids(text, &format!(".{name}"), 1));
            sites.extend(call_ids(text, &format!("|{name}"), 1));
        }
        for name in ["host.control(", "host.device_control("] {
            sites.extend(call_ids(text, name, 0));
        }
        let local = file_constants(text);
        for (line, a) in sites {
            let short = a.rsplit("::").next().unwrap_or(&a);
            let id = if a.starts_with("0x") {
                Some(hex(&a))
            } else if let Some(v) = local.get(short) {
                Some(*v)
            } else {
                consts
                    .get(short)
                    .filter(|s| s.len() == 1)
                    .and_then(|s| s.iter().next().copied())
            };
            match id {
                Some(id) if rows.contains(&id) => resolved += 1,
                Some(id) => unlisted.push(format!("{rel}:{line} {a} = {id:#010x}")),
                None => {
                    dynamic.insert((rel.clone(), a));
                }
            }
        }
    }
    assert!(
        unlisted.is_empty(),
        "host control ids with no HOST_CONTROLS row (the G-5 'neither' finding): {unlisted:#?}"
    );
    let listed: BTreeSet<(String, String)> = DYNAMIC_SITES
        .iter()
        .map(|(f, a, _)| ((*f).to_string(), (*a).to_string()))
        .collect();
    let new: Vec<_> = dynamic.difference(&listed).collect();
    assert!(
        new.is_empty(),
        "host control call sites with a non-constant id, not listed: {new:#?}"
    );
    assert!(
        resolved >= 20,
        "the scan found only {resolved} constant sites: is it still reading the tree?"
    );
}

#[test]
fn the_tabled_host_controls_are_rows() {
    let rows = rows();
    for r in kf_abi::gssreplay::ROWS {
        assert!(
            rows.contains(&r.cmd),
            "gssreplay row {:#010x} has no host row",
            r.cmd
        );
    }
    for c in [
        kf_abi::gssreplay::GSS_ENC_SESSION_ACQUIRE,
        kf_abi::gssreplay::GSS_ENC_SESSION_RELEASE,
        kf_abi::utilization::GPUMON_CMD,
        kf_abi::timer::REGISTER_OFFSET,
        kf_abi::chanbudget::CONTROL,
    ] {
        assert!(rows.contains(&c), "{c:#010x} has no host row");
    }
}

/// ★ The per-tag picture of `control_carry` for an id with no row (pass-through or refused by
/// name), over every measured host tag. Today an unlisted id passes through at the bench and inside
/// `[580.65.06, 581)`; the two tests above are what makes that harmless (no call site sends one).
#[test]
fn an_unlisted_id_passes_through_only_inside_the_encoded_interval() {
    let probe = 0x2080_dead;
    assert!(kf_abi::hostabi::host_control(probe).is_none());
    let mut through = Vec::new();
    for v in kf_abi::generated::matrix::MEASURED {
        let h = HostAbi::for_host(HostDriverVersion {
            major: v.major,
            minor: v.minor,
            patch: v.patch,
        })
        .expect("measured");
        match h.control_carry(probe) {
            Ok(None) => through.push(format!("{}.{}.{}", v.major, v.minor, v.patch)),
            Err(HostAbiError::UnlistedControl { .. }) => {}
            other => panic!("{v:?}: {other:?}"),
        }
    }
    assert!(through.iter().all(|t| t.starts_with("580.")), "{through:?}");
    assert_eq!(through.len(), 9, "{through:?}");
}
