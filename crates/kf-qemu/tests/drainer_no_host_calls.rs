//! ★ Drainer verification 2026-10-09 (`docs/design/V3_NONSTALL_THREADS.md` §9, claim (c)): the
//! register drainer makes no host RM call of its own.
//!
//! The drainer reaches the channel plane through `ChanPlane::statement` (the `channels` sink of the
//! served chain, `device.rs`). Every statement that needs the host answers `Deferred` and its host
//! verbs run in a closure on the act thread. This is a SOURCE SCAN (like `no_raw_prints.rs`; the
//! plane needs a real `HostRm`, so it cannot be built GPU-free): in every function that `statement`
//! runs synchronously, no host-session call (`self.rm.` / `me.rm.`), sleep, receive or join may
//! appear outside a `Box::new(|…| …)` act closure. A new synchronous host call added to the drainer
//! side fails this test by name.

use std::fs;

/// Blank out comments and the CONTENTS of string literals (length and newlines kept), so braces in
/// format strings and prose cannot confuse the matcher.
fn strip(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let (mut i, n) = (0, b.len());
    while i < n {
        if b[i] == b'/' && i + 1 < n && b[i + 1] == b'/' {
            while i < n && b[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
        } else if b[i] == b'"' {
            i += 1;
            while i < n && b[i] != b'"' {
                if b[i] == b'\\' && i + 1 < n {
                    if b[i + 1] != b'\n' {
                        out[i + 1] = b' ';
                    }
                    out[i] = b' ';
                    i += 2;
                    continue;
                }
                if b[i] != b'\n' {
                    out[i] = b' ';
                }
                i += 1;
            }
            i += 1;
        } else {
            i += 1;
        }
    }
    String::from_utf8(out).expect("ascii-preserving")
}

/// The index of the delimiter closing the one opened at `open`.
fn matching(s: &[u8], open: usize) -> usize {
    let (o, c) = match s[open] {
        b'{' => (b'{', b'}'),
        b'(' => (b'(', b')'),
        other => panic!("not an opener: {}", other as char),
    };
    let mut depth = 0usize;
    for (k, &ch) in s.iter().enumerate().skip(open) {
        if ch == o {
            depth += 1;
        } else if ch == c {
            depth -= 1;
            if depth == 0 {
                return k;
            }
        }
    }
    panic!("unbalanced at {open}");
}

/// `fn name`'s body, with every act closure (`Box::new(<closure>)`) blanked.
fn sync_part(src: &str, name: &str) -> (String, usize) {
    let s = src.as_bytes();
    let pat = format!("fn {name}(");
    let at = src
        .find(&pat)
        .or_else(|| src.find(&format!("fn {name}<")))
        .unwrap_or_else(|| panic!("`{name}` is gone from chan.rs: update this scan"));
    let open = at + src[at..].find('{').expect("a body");
    let close = matching(s, open);
    let mut body = src.as_bytes()[open..=close].to_vec();
    let mut closures = 0;
    let mut from = 0;
    // Search the body AS BLANKED SO FAR: a `Box::new(` nested in a closure already blanked is gone.
    while let Some(k) = std::str::from_utf8(&body).expect("ascii")[from..].find("Box::new(") {
        let paren = from + k + "Box::new".len();
        let end = matching(&body, paren);
        let head = &body[paren + 1..end.min(paren + 60)];
        if head.contains(&b'|') {
            closures += 1;
            for b in &mut body[paren + 1..end] {
                if *b != b'\n' {
                    *b = b' ';
                }
            }
        }
        from = paren + 1;
    }
    (String::from_utf8(body).expect("ascii"), closures)
}

/// Every function `ChanPlane::statement` runs synchronously on the drainer (its callees that are
/// not act steps).
const SYNC: &[&str] = &[
    "statement",
    "birth",
    "free",
    "free_channels",
    "stop_channel",
    "disable_channels",
    "preempt_group",
    "promote_ctx",
    "evict_ctx",
    "schedule_translated",
    "schedule_flags",
    "defer_quiesced",
    "engine_object",
    "display_sw",
    "number_translated_sw",
    "translated_ht",
    "deferred_api_object",
    "software_object",
    "debugger",
    "encoder_session",
    "translated_of",
    "take_pt",
    "slot",
    "defer",
    "defer_steps",
    "submit_steps",
];

const FORBIDDEN: &[&str] = &[
    "self.rm.",
    "me.rm.",
    ".rm.",
    "thread::sleep",
    ".recv(",
    ".recv_timeout(",
    ".join()",
    "dbfast.deregister(",
    "begin_deregister(",
];

#[test]
fn the_channel_planes_drainer_side_makes_no_host_call_outside_an_act_closure() {
    let raw = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/chan.rs")).unwrap();
    let src = strip(&raw);
    let mut total_closures = 0;
    for name in SYNC {
        let (body, closures) = sync_part(&src, name);
        total_closures += closures;
        for f in FORBIDDEN {
            assert!(
                !body.contains(f),
                "`ChanPlane::{name}` runs on the register drainer and contains `{f}` outside an act \
                 closure (V3_NONSTALL_THREADS.md §9, claim (c))"
            );
        }
    }
    // The scan is not vacuous: the statement paths DO hand their host verbs to act closures.
    assert!(
        total_closures >= 20,
        "only {total_closures} act closures found in the drainer-side functions: the scan lost them"
    );
}

/// The control: the same scan DOES see a host call when it is not inside a closure — the worker's
/// and the act thread's own helpers (`release_twin`, `arm_notifier`) are full of them.
#[test]
fn control_the_scan_sees_host_calls_in_the_act_side_helpers() {
    let raw = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/chan.rs")).unwrap();
    let src = strip(&raw);
    for name in [
        "release_twin",
        "arm_notifier",
        "retire_body",
        "release_encoder_sessions",
    ] {
        let (body, _) = sync_part(&src, name);
        assert!(
            body.contains("self.rm."),
            "`{name}` should show a host call"
        );
    }
}

/// The one exception, named: the BAR0 trace dump (`KF3_BAR0_TRACE`, a default-off diagnostic, once
/// per run) runs on the drainer and arms and releases a CPU view — an RM ioctl. It must stay
/// unreachable unless the diagnostic is on.
#[test]
fn the_one_drainer_side_host_call_is_the_default_off_bar0_trace_dump() {
    let chan = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/chan.rs")).unwrap();
    let (dump, _) = sync_part(&strip(&chan), "bar0trace_dump");
    assert!(
        dump.contains("self.rm.release_cpu_view"),
        "the exception moved: re-read it"
    );
    let device = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/device.rs")).unwrap();
    let callers: Vec<_> = device.match_indices("bar0trace_dump(").collect();
    assert_eq!(callers.len(), 1, "a second caller of the dump appeared");
    let at = callers[0].0;
    let window = &device[at.saturating_sub(400)..at];
    assert!(
        window.contains("take_dump()"),
        "the only caller must be behind the one-shot `take_dump()` of the diagnostic"
    );
}
