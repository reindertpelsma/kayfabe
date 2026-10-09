//! ★ Drainer verification 2026-10-09 (`docs/design/V3_NONSTALL_THREADS.md` §9, claim (e)): who takes
//! the GSP mutex (`Device::gsp`).
//!
//! The drainer holds it for the whole of every register write it applies, so any other taker is a
//! thread the drainer can wait for. A SOURCE SCAN of every file of the crate (the field is private to
//! `device.rs`, so a taker in another file would have to be a method of `Device` declared there; the
//! scan looks anyway): the blocking `lock()` is taken by the drainer's functions and by
//! `seal_shadow` (realize, before the guest runs); the status thread takes it with `try_lock()` only.

use std::fs;
use std::path::Path;

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

/// `(enclosing fn name, method)` of every `<field>.<method>(` call in `src`, whitespace-insensitive
/// (rustfmt splits `self\n.gsp\n.lock()`), for the methods in `methods`.
fn takers(src: &str, field: &str, methods: &[&str]) -> Vec<(String, String)> {
    let src = strip(src);
    // Whitespace removed, remembering where each kept byte came from.
    let mut flat = String::new();
    let mut origin = Vec::new();
    for (i, c) in src.char_indices() {
        if !c.is_whitespace() {
            flat.push(c);
            origin.push(i);
        }
    }
    let mut found = Vec::new();
    for m in methods {
        let pat = format!(".{field}.{m}(");
        let mut from = 0;
        while let Some(k) = flat[from..].find(&pat) {
            let at = from + k;
            let o = origin[at];
            let before = &src[..o];
            let name = before
                .rmatch_indices("fn ")
                .find_map(|(p, _)| {
                    let rest = &before[p + 3..];
                    let id: String = rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    let ok_prefix = p == 0 || !before.as_bytes()[p - 1].is_ascii_alphanumeric();
                    (ok_prefix && !id.is_empty()).then_some(id)
                })
                .unwrap_or_else(|| "?".into());
            found.push((name, (*m).to_string()));
            from = at + pat.len();
        }
    }
    found.sort();
    found
}

fn every_source_file(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            every_source_file(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn the_gsp_mutex_is_taken_by_the_drainer_realize_and_the_status_thread_only() {
    let mut files = Vec::new();
    every_source_file(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let mut blocking: Vec<(String, String)> = Vec::new();
    let mut nonblocking: Vec<(String, String)> = Vec::new();
    for f in &files {
        let src = fs::read_to_string(f).unwrap();
        // Test modules are not production takers.
        let src = src
            .split("\n#[cfg(test)]")
            .next()
            .unwrap_or_default()
            .to_string();
        for t in takers(&src, "gsp", &["lock", "read", "write"]) {
            blocking.push((
                format!("{}::{}", f.file_name().unwrap().to_string_lossy(), t.0),
                t.1,
            ));
        }
        for t in takers(&src, "gsp", &["try_lock", "try_read", "try_write"]) {
            nonblocking.push((
                format!("{}::{}", f.file_name().unwrap().to_string_lossy(), t.0),
                t.1,
            ));
        }
    }
    let names = |v: &[(String, String)]| -> Vec<String> {
        let mut n: Vec<String> = v.iter().map(|(f, _)| f.clone()).collect();
        n.sort();
        n.dedup();
        n
    };
    assert_eq!(
        names(&blocking),
        [
            "device.rs::apply_register",
            "device.rs::deliver_hotplug",
            "device.rs::deliver_rc",
            "device.rs::release_settled",
            "device.rs::seal_shadow",
        ],
        "a new blocking taker of the GSP mutex: the drainer can now wait for it"
    );
    assert_eq!(
        names(&nonblocking),
        ["device.rs::status_line"],
        "a new try_lock taker of the GSP mutex"
    );
}

/// The scan itself works: it finds a blocking taker split across lines and ignores prose.
#[test]
fn control_the_takers_scan_sees_a_split_chain_and_ignores_comments() {
    let src = "fn a(&self) {\n    let g = self\n        .gsp\n        .lock();\n}\n\
               // self.gsp.lock() in prose\nfn b(&self) { let _ = \"self.gsp.lock()\"; }\n\
               fn c(&self) { self.gsp.try_lock(); }";
    assert_eq!(
        takers(src, "gsp", &["lock"]),
        vec![("a".to_string(), "lock".to_string())]
    );
    assert_eq!(
        takers(src, "gsp", &["try_lock"]),
        vec![("c".to_string(), "try_lock".to_string())]
    );
}
