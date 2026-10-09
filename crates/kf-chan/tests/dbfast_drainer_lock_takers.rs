//! ★ Drainer verification 2026-10-09 (`docs/design/V3_NONSTALL_THREADS.md` §9, items D1-D2): which
//! functions of the doorbell fast path take which lock.
//!
//! The drainer's half of `DbFast` is `service_ready` / `on_ready` (→ `deliver_tags`, `sync`,
//! `drain_and_deliver`). The registry lock `reg` is held across `KVM_IOEVENTFD` (an SRCU grace
//! period) by the main loop and the act thread, so the drainer must never take it; `tx` is the act
//! thread's sender. A SOURCE SCAN of `dbfast.rs` (the behavioural proof is
//! `the_drainer_serves_doorbells_while_another_thread_sits_in_kvm_ioeventfd` in the unit tests):
//! `side` is taken only by the drainer's two entries, and `reg` / `lock_reg` / `tx` only by the
//! act-thread and main-loop functions.

use std::fs;

/// Comments and string contents blanked, so prose cannot match.
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
                    out[i] = b' ';
                    if b[i + 1] != b'\n' {
                        out[i + 1] = b' ';
                    }
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

/// The enclosing `fn` of every occurrence of `pat` (whitespace-insensitive), sorted, deduplicated.
fn takers(src: &str, pat: &str) -> Vec<String> {
    let src = strip(src);
    let mut flat = String::new();
    let mut origin = Vec::new();
    for (i, c) in src.char_indices() {
        if !c.is_whitespace() {
            flat.push(c);
            origin.push(i);
        }
    }
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(k) = flat[from..].find(pat) {
        let at = from + k;
        let before = &src[..origin[at]];
        let name = before
            .rmatch_indices("fn ")
            .find_map(|(p, _)| {
                let id: String = before[p + 3..]
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                let ok = p == 0 || !before.as_bytes()[p - 1].is_ascii_alphanumeric();
                (ok && !id.is_empty()).then_some(id)
            })
            .unwrap_or_else(|| "?".into());
        found.push(name);
        from = at + pat.len();
    }
    found.sort();
    found.dedup();
    found
}

fn production() -> String {
    let src = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/dbfast.rs")).unwrap();
    src.split("\n#[cfg(test)]").next().unwrap().to_string()
}

#[test]
fn the_side_table_is_the_drainers_alone() {
    let src = production();
    assert_eq!(
        takers(&src, ".side.lock("),
        ["deliver_tags", "service_ready"],
        "something other than the drainer's two entries takes the drainer-only table"
    );
}

#[test]
fn the_registry_lock_and_the_sender_are_never_taken_by_a_drainer_function() {
    let src = production();
    let drainer_side = [
        "service_ready",
        "on_ready",
        "deliver_tags",
        "sync",
        "drain_and_deliver",
        "status",
        "poller",
    ];
    for pat in [".reg.lock(", ".reg.try_lock(", ".lock_reg(", ".tx.lock("] {
        for taker in takers(&src, pat) {
            assert!(
                !drainer_side.contains(&taker.as_str()),
                "`{taker}` is on the drainer and takes `{pat}` (held across KVM_IOEVENTFD by other threads)"
            );
        }
    }
    // …and the scan sees them where they are.
    assert_eq!(
        takers(&src, ".lock_reg("),
        ["begin_deregister", "register", "site_add", "site_del"]
    );
    assert_eq!(takers(&src, ".tx.lock("), ["send"]);
}
