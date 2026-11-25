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
//! And the source scan proves the second half of "never forwarded": every host control call site in
//! a `crates/kf-*` product crate (any method named `raw_control…`, in any spelling, and the kf-rm
//! `host.control` seams; the `kf-harness` operator tools are not product code) either names a
//! constant id that resolves to a ROW, or is listed with its exact call count in [`DYNAMIC_SITES`]
//! (its id comes from an authored table, never from a guest), or is listed in
//! [`HARNESS_ONLY_SITES`] (a method that no product crate calls). A new served id, a new host call
//! site, one more call at a listed site, or a host id without a row fails here by name.

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

/// `code` with comments and the contents of string and char literals blanked to spaces (newlines
/// and positions kept), so a call named only in a comment or a string never counts.
fn blank(code: &str) -> String {
    let b = code.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    let wipe = |out: &mut Vec<u8>, from: usize, to: usize| {
        for c in out.iter_mut().take(to).skip(from) {
            if *c != b'\n' {
                *c = b' ';
            }
        }
    };
    while i < b.len() {
        let ident_before = i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
        if b[i..].starts_with(b"//") {
            let end = b[i..]
                .iter()
                .position(|&c| c == b'\n')
                .map_or(b.len(), |n| i + n);
            wipe(&mut out, i, end);
            i = end;
        } else if b[i..].starts_with(b"/*") {
            let (mut depth, mut j) = (1, i + 2);
            while j < b.len() && depth > 0 {
                if b[j..].starts_with(b"/*") {
                    depth += 1;
                    j += 2;
                } else if b[j..].starts_with(b"*/") {
                    depth -= 1;
                    j += 2;
                } else {
                    j += 1;
                }
            }
            wipe(&mut out, i, j);
            i = j;
        } else if !ident_before && (b[i..].starts_with(b"r\"") || b[i..].starts_with(b"r#")) {
            let hashes = b[i + 1..].iter().take_while(|&&c| c == b'#').count();
            let open = i + 1 + hashes;
            if b.get(open) != Some(&b'"') {
                i += 1;
                continue;
            }
            let close: Vec<u8> = std::iter::once(b'"')
                .chain(std::iter::repeat_n(b'#', hashes))
                .collect();
            let end = b[open + 1..]
                .windows(close.len())
                .position(|w| w == close.as_slice())
                .map_or(b.len(), |n| open + 1 + n + close.len());
            wipe(&mut out, open + 1, end.saturating_sub(close.len()));
            i = end;
        } else if b[i] == b'"' {
            let mut j = i + 1;
            while j < b.len() && b[j] != b'"' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            wipe(&mut out, i + 1, j.min(b.len()));
            i = j + 1;
        } else if b[i] == b'\'' {
            // A char literal (`'x'`, `'\n'`, `'\u{..}'`); anything else is a lifetime.
            let end = if b.get(i + 1) == Some(&b'\\') {
                b[i + 2..]
                    .iter()
                    .position(|&c| c == b'\'')
                    .map(|n| i + 2 + n)
            } else {
                let ch = code[i + 1..].chars().next().map_or(1, char::len_utf8);
                (b.get(i + 1 + ch) == Some(&b'\'')).then_some(i + 1 + ch)
            };
            match end {
                Some(e) => {
                    wipe(&mut out, i + 1, e);
                    i = e + 1;
                }
                None => i += 1,
            }
        } else {
            i += 1;
        }
    }
    String::from_utf8(out).expect("blanking keeps UTF-8 boundaries")
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// The name of the last `fn` item that starts before `at` in blanked `code` (the call's enclosing
/// function; a call inside a closure belongs to the function that holds the closure).
fn enclosing_fn(code: &str, at: usize) -> String {
    let b = code.as_bytes();
    let mut best = String::from("<top>");
    let mut from = 0;
    while let Some(rel) = code[from..at].find("fn ") {
        let p = from + rel;
        from = p + 3;
        if p > 0 && is_ident(b[p - 1]) {
            continue;
        }
        let name: String = code[p + 3..]
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            best = name;
        }
    }
    best
}

/// The arguments of the call whose `(` is at `open` in blanked `code`, whitespace removed.
fn call_args(code: &str, open: usize) -> Vec<String> {
    let mut depth = 0i32;
    let mut args = vec![String::new()];
    for ch in code[open + 1..].chars() {
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
        if let Some(a) = args.last_mut()
            && !ch.is_whitespace()
        {
            a.push(ch);
        }
    }
    args
}

/// One host-control call site: its line, the method called, its enclosing function and the text
/// of its command-id argument.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Site {
    line: usize,
    method: String,
    func: String,
    arg: String,
}

/// Every host-control call site in `text`: any method whose name starts with `raw_control` (the
/// ioctl choke point `raw_control`, `_exact`, `_status`, `_nested`, `_opaque`, and any new one),
/// called as a method (`x.raw_control(`), by path (`HostRm::raw_control(rm, ..)`, receiver first)
/// or bare, with any whitespace before `(`; plus the kf-rm host seams `host.control(` and
/// `host.device_control(` (command id first). Definitions (`fn raw_control…`) are not calls.
fn host_control_sites(text: &str) -> Vec<Site> {
    let code = blank(text);
    let b = code.as_bytes();
    let line_of = |p: usize| code[..p].matches('\n').count() + 1;
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = code[from..].find("raw_control") {
        let at = from + rel;
        from = at + 1;
        if at > 0 && is_ident(b[at - 1]) {
            continue;
        }
        let name_end = at + code[at..].bytes().take_while(|&c| is_ident(c)).count();
        let after = code[name_end..].trim_start();
        if !after.starts_with('(') {
            continue;
        }
        let before = code[..at].trim_end();
        if before.ends_with("fn")
            && !before[..before.len() - 2]
                .ends_with(|c: char| c == '_' || c.is_ascii_alphanumeric())
        {
            continue;
        }
        let open = code.len() - after.len();
        let arg = if before.ends_with("::") { 2 } else { 1 };
        out.push(Site {
            line: line_of(at),
            method: code[at..name_end].to_string(),
            func: enclosing_fn(&code, at),
            arg: call_args(&code, open).get(arg).cloned().unwrap_or_default(),
        });
    }
    for seam in ["host.control", "host.device_control"] {
        let mut from = 0;
        while let Some(rel) = code[from..].find(seam) {
            let at = from + rel;
            from = at + 1;
            if at > 0 && is_ident(b[at - 1]) {
                continue;
            }
            let end = at + seam.len();
            let after = code[end..].trim_start();
            if !after.starts_with('(') {
                continue;
            }
            let open = code.len() - after.len();
            out.push(Site {
                line: line_of(at),
                method: seam.to_string(),
                func: enclosing_fn(&code, at),
                arg: call_args(&code, open).first().cloned().unwrap_or_default(),
            });
        }
    }
    out
}

/// ★ The host-control call sites whose command is not a constant, keyed by (file, enclosing fn,
/// argument) with the exact number of such calls there, each with where its id comes from. A new
/// one, or one more call under an existing key, fails [`every_host_control_call_site_names_a_row`]
/// until it is listed here (security review of PR #541, F1: keyed by file and argument alone, a new
/// `raw_control(obj, cmd, ..)` anywhere in a listed file passed).
const DYNAMIC_SITES: &[(&str, &str, &str, usize, &str)] = &[
    (
        "crates/kf-host/src/lib.rs",
        "raw_control",
        "cmd",
        2,
        "the choke point: forwards its own caller's id (every caller is a scanned site)",
    ),
    (
        "crates/kf-host/src/lib.rs",
        "raw_control_exact",
        "cmd",
        1,
        "the choke point: forwards its own caller's id (every caller is a scanned site)",
    ),
    (
        "crates/kf-host/src/channel.rs",
        "birth_member_profile",
        "cmd",
        1,
        "NVA06C_CTRL_CMD_BIND or NVA06F_CTRL_CMD_BIND, chosen by `first`",
    ),
    (
        "crates/kf-qemu/src/chan.rs",
        "encoder_session_act",
        "cmd",
        1,
        "GSS_ENC_SESSION_ACQUIRE / _RELEASE (`enc_session_step`)",
    ),
    (
        "crates/kf-qemu/src/rmfacts.rs",
        "control",
        "cmd",
        1,
        "the realize-time host queries of `kf_rm::hostquery` (authored)",
    ),
    (
        "crates/kf-qemu/src/rmfacts.rs",
        "device_control",
        "cmd",
        1,
        "the realize-time host queries of `kf_rm::hostquery` (authored)",
    ),
    (
        "crates/kf-qemu/src/utilhost.rs",
        "read",
        "cmd",
        1,
        "`kf_abi::utilization::GPUMON_CMD`",
    ),
    (
        "crates/kf-host/src/precomp.rs",
        "read_precomp_capabilities",
        "cmd",
        1,
        "the startup PRECOMP plan, compiled per host version (`kf-host/src/precomp.rs`)",
    ),
    (
        "crates/kf-rm/src/hostquery.rs",
        "ask",
        "cmd",
        1,
        "authored hostquery constants",
    ),
    (
        "crates/kf-rm/src/hostquery.rs",
        "query_gr_context_buffers",
        "cmd",
        1,
        "authored hostquery constants",
    ),
    (
        "crates/kf-rm/src/hostquery.rs",
        "query_lce_pce_masks_over",
        "cmd",
        1,
        "authored hostquery constants",
    ),
    (
        "crates/kf-rm/src/hostquery.rs",
        "query_video_caps",
        "cmd",
        1,
        "authored hostquery constants",
    ),
    (
        "crates/kf-rm/src/hostquery.rs",
        "query_gss_replay",
        "row.cmd",
        2,
        "`kf_abi::gssreplay::ROWS`",
    ),
    (
        "crates/kf-rm/src/hostquery.rs",
        "query_fifo_latency",
        "layout.cmd",
        1,
        "`kf_abi::fifoctl::LatencyLayout`",
    ),
    (
        "crates/kf-rm/src/hostquery.rs",
        "query_gr_geometry",
        "map_cmd",
        2,
        "authored GR FS / intr map constants",
    ),
    (
        "crates/kf-host/src/timer.rs",
        "open_timer",
        "kf_abi::timer::REGISTER_OFFSET",
        1,
        "a kf-abi alias, checked below",
    ),
    (
        "crates/kf-qemu/src/rmfacts.rs",
        "channel_budget",
        "kf_abi::chanbudget::CONTROL",
        1,
        "a kf-abi alias, checked below",
    ),
];

/// ★ Host-control call sites in product crates that only an operator tool reaches: each names a
/// METHOD whose every caller is in `crates/kf-harness` ([`harness_only_methods_have_no_product_caller`]),
/// so no guest-driven path reaches the id it sends (security review of PR #541, F2; of PR #374,
/// F374-1). Same key as [`DYNAMIC_SITES`], plus the harness-only method.
const HARNESS_ONLY_SITES: &[(&str, &str, &str, usize, &str, &str)] = &[
    (
        "crates/kf-host/src/channel.rs",
        "probe_deferred_channel_update",
        "cmd",
        1,
        "probe_deferred_channel_update",
        "NV5080 deferred-API V2 registration (`encode_deferred_channel_update`, 0x50800103), already \
         at the host's compiler-derived layout; the kf-harness probe is its only caller",
    ),
    (
        "crates/kf-host/src/lib.rs",
        "raw_control_opaque",
        "cmd",
        1,
        "raw_control_opaque",
        "the opaque control (CALLER-CONTRACT: operator tools only); the kf-harness probes are its \
         only callers",
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
    let mut dynamic: BTreeMap<(String, String, String), usize> = BTreeMap::new();
    let mut resolved = 0usize;
    for (rel, text) in &src {
        let local = file_constants(text);
        for Site {
            line, func, arg: a, ..
        } in host_control_sites(text)
        {
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
                None => *dynamic.entry((rel.clone(), func, a)).or_default() += 1,
            }
        }
    }
    assert!(
        unlisted.is_empty(),
        "host control ids with no HOST_CONTROLS row (the G-5 'neither' finding): {unlisted:#?}"
    );
    let listed: BTreeMap<(String, String, String), usize> = DYNAMIC_SITES
        .iter()
        .map(|(f, func, a, n, _)| ((*f, *func, *a), *n))
        .chain(
            HARNESS_ONLY_SITES
                .iter()
                .map(|(f, func, a, n, _, _)| ((*f, *func, *a), *n)),
        )
        .map(|((f, func, a), n)| ((f.to_string(), func.to_string(), a.to_string()), n))
        .collect();
    let new: Vec<_> = dynamic
        .iter()
        .filter(|(k, n)| listed.get(*k) != Some(n))
        .collect();
    assert!(
        new.is_empty(),
        "host control call sites with a non-constant id, not listed with this count in \
         DYNAMIC_SITES or HARNESS_ONLY_SITES (file, fn, argument) -> calls: {new:#?}"
    );
    let stale: Vec<_> = listed
        .keys()
        .filter(|k| !dynamic.contains_key(*k))
        .collect();
    assert!(
        stale.is_empty(),
        "listed sites no longer in the tree: {stale:#?}"
    );
    assert!(
        resolved >= 20,
        "the scan found only {resolved} constant sites: is it still reading the tree?"
    );
}

/// Every `crates/kf-*/src` file, the operator tools (`kf-harness`) included.
fn all_kf_sources() -> Vec<(String, String)> {
    let mut files = Vec::new();
    for e in std::fs::read_dir(root().join("crates"))
        .expect("crates")
        .flatten()
    {
        if e.file_name().to_string_lossy().starts_with("kf-") {
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
            (rel, std::fs::read_to_string(&p).expect("read"))
        })
        .collect()
}

/// Calls of method `name` in `text` (`.name(`, `::name(`, any whitespace before `(`), definitions
/// excluded, as `(line, enclosing fn)`.
fn calls_of(text: &str, name: &str) -> Vec<(usize, String)> {
    let code = blank(text);
    let b = code.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = code[from..].find(name) {
        let at = from + rel;
        from = at + 1;
        let end = at + name.len();
        if (at > 0 && is_ident(b[at - 1])) || b.get(end).is_some_and(|&c| is_ident(c)) {
            continue;
        }
        if !code[end..].trim_start().starts_with('(') {
            continue;
        }
        let before = code[..at].trim_end();
        if !(before.ends_with('.') || before.ends_with("::")) {
            continue;
        }
        out.push((
            code[..at].matches('\n').count() + 1,
            enclosing_fn(&code, at),
        ));
    }
    out
}

/// ★ Security review of PR #374, F374-1, and of PR #541, F2: a harness-only method (the opaque
/// control that once carried guest bytes, the deferred-API registration probe) has no caller in any
/// product crate. Re-adding a guest-bytes forward through `raw_control_opaque` in kf-qemu fails here.
#[test]
fn harness_only_methods_have_no_product_caller() {
    let mut outside = Vec::new();
    let mut harness_calls = 0usize;
    for (rel, text) in all_kf_sources() {
        for (_, _, _, _, method, _) in HARNESS_ONLY_SITES {
            for (line, func) in calls_of(&text, method) {
                if rel.starts_with("crates/kf-harness/") {
                    harness_calls += 1;
                } else {
                    outside.push(format!("{rel}:{line} fn {func} calls {method}"));
                }
            }
        }
    }
    assert!(
        outside.is_empty(),
        "harness-only host-control methods called from a product crate: {outside:#?}"
    );
    assert!(
        harness_calls >= HARNESS_ONLY_SITES.len(),
        "★ NON-VACUITY: only {harness_calls} kf-harness call(s) found"
    );
}

/// The scanner's own known positives (review of PR #541, F2 mutants S2, S3, S5, and F1's S1): each
/// spelling of a host-control call is seen, with its enclosing function; comments, strings and
/// definitions are not calls.
#[test]
fn the_scanner_sees_every_spelling_of_a_host_control_call() {
    let planted = "fn a(&self) { self.rm.raw_control(obj, cmd, &mut p) }\n\
         fn b(&self) { self.raw_control_exact(o, 0x5080_0103, &mut p) }\n\
         fn c(rm: &HostRm) { HostRm::raw_control(rm, o, guest_cmd, &mut p); }\n\
         fn d(&self) { self.rm.raw_control (o, spaced, p) }\n\
         fn e(&self) { self.rm\n    .raw_control_opaque(o, cmd, p) }\n\
         fn f(&self) { let _ = \"x.raw_control(o, in_string, p)\"; } // y.raw_control(o, in_comment, p)\n\
         pub fn raw_control_new(&self, o: u32, cmd: u32) {}\n\
         fn g(host: &dyn H) { host.control (cmd, p) }\n";
    let got: Vec<(String, String, String)> = host_control_sites(planted)
        .into_iter()
        .map(|s| (s.func, s.method, s.arg))
        .collect();
    let want: Vec<(String, String, String)> = [
        ("a", "raw_control", "cmd"),
        ("b", "raw_control_exact", "0x5080_0103"),
        ("c", "raw_control", "guest_cmd"),
        ("d", "raw_control", "spaced"),
        ("e", "raw_control_opaque", "cmd"),
        ("g", "host.control", "cmd"),
    ]
    .into_iter()
    .map(|(f, m, a)| (f.to_string(), m.to_string(), a.to_string()))
    .collect();
    assert_eq!(got, want);
    assert_eq!(
        calls_of("fn x() { a.raw_control_opaque (o, c, p); Y::raw_control_opaque(r, o, c, p) }\nfn raw_control_opaque() {}", "raw_control_opaque").len(),
        2
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
