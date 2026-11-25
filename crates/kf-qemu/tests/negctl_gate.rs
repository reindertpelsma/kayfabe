//! The `KF3_NEGCTL_*` control switches exist only in a `negctl` build (`STATUS: LIVE, 2026-11-25`).
//!
//! Policy: the release binary has no switch that undoes a boundary. Each switch is an environment read in a small
//! function that comes in two forms, `#[cfg(feature = "negctl")]` (the read) and `#[cfg(not(feature = "negctl"))]`
//! (a constant). This test reads the product crates' source and fails when a `KF3_NEGCTL_` name appears in code (a
//! string literal) outside an item or statement that carries the `negctl` gate, so a new switch added without the gate
//! cannot reach the release binary. Names in comments are not code and are allowed.
//!
//! It is a source scan: it says no read is un-gated, not that the gate is spelled right. The compiled form is checked
//! beside each function (`negctl_seam_tests`, which const-evaluates every control to its safe value in a default
//! build) and in `kf-qemu`'s lib tests (the test binary of a default build holds no `KF3_NEGCTL_` string).

use std::path::{Path, PathBuf};

const NAME: &str = "KF3_NEGCTL_";
const GATE: &str = "cfg(feature=\"negctl\")";

/// The end of a string or character literal that starts at `i`, or `None` when none starts there.
fn literal_end(c: &[char], i: usize) -> Option<usize> {
    let at = |k: usize| c.get(k).copied();
    // raw string: r"..." / r#"..."# / br"..."
    let mut j = i;
    if at(j) == Some('b') {
        j += 1;
    }
    if at(j) == Some('r') && !(i > 0 && (c[i - 1].is_alphanumeric() || c[i - 1] == '_')) {
        let mut hashes = 0;
        let mut k = j + 1;
        while at(k) == Some('#') {
            hashes += 1;
            k += 1;
        }
        if at(k) == Some('"') {
            let mut m = k + 1;
            while m < c.len() {
                if c[m] == '"' && (0..hashes).all(|h| at(m + 1 + h) == Some('#')) {
                    return Some(m + 1 + hashes);
                }
                m += 1;
            }
            return Some(c.len());
        }
    }
    match at(i)? {
        '"' => {
            let mut m = i + 1;
            while m < c.len() {
                match c[m] {
                    '\\' => m += 2,
                    '"' => return Some(m + 1),
                    _ => m += 1,
                }
            }
            Some(c.len())
        }
        '\'' => {
            if at(i + 1) == Some('\\') {
                let mut m = i + 3;
                while m < c.len() && c[m] != '\'' {
                    m += 1;
                }
                Some(m + 1)
            } else if at(i + 2) == Some('\'') {
                Some(i + 3)
            } else {
                None // a lifetime
            }
        }
        _ => None,
    }
}

/// `text` with every comment replaced by spaces (newlines kept, so line numbers survive).
fn strip_comments(text: &str) -> Vec<char> {
    let c: Vec<char> = text.chars().collect();
    let mut out = c.clone();
    let mut i = 0;
    while i < c.len() {
        if let Some(end) = literal_end(&c, i) {
            i = end.max(i + 1);
        } else if c[i] == '/' && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' {
                out[i] = ' ';
                i += 1;
            }
        } else if c[i] == '/' && c.get(i + 1) == Some(&'*') {
            let mut depth = 0;
            while i < c.len() {
                if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                    depth += 1;
                    out[i] = ' ';
                    out[i + 1] = ' ';
                    i += 2;
                } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    out[i] = ' ';
                    out[i + 1] = ' ';
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if c[i] != '\n' {
                        out[i] = ' ';
                    }
                    i += 1;
                }
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Blanks every item or statement that follows a `#[cfg(feature = "negctl")]` attribute.
fn strip_gated(code: &mut [char]) {
    let c: Vec<char> = code.to_vec();
    let mut i = 0;
    while i < c.len() {
        if let Some(end) = literal_end(&c, i) {
            i = end.max(i + 1);
            continue;
        }
        if c[i] != '#' {
            i += 1;
            continue;
        }
        // an attribute: `#[ ... ]`
        let mut j = i + 1;
        if c.get(j) == Some(&'!') {
            j += 1;
        }
        if c.get(j) != Some(&'[') {
            i += 1;
            continue;
        }
        let mut depth = 0;
        let mut k = j;
        while k < c.len() {
            if let Some(end) = literal_end(&c, k) {
                k = end;
                continue;
            }
            match c[k] {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            k += 1;
        }
        let attr: String = c[j + 1..k.min(c.len())]
            .iter()
            .filter(|ch| !ch.is_whitespace())
            .collect();
        i = k + 1;
        if attr != GATE {
            continue;
        }
        // the gated item or statement: ends at a `;` or at the `}` that closes its first brace block,
        // at nesting depth 0 (further attributes before it are part of it)
        let start = i;
        let mut depth = 0i32;
        let mut m = i;
        while m < c.len() {
            if let Some(end) = literal_end(&c, m) {
                m = end;
                continue;
            }
            match c[m] {
                '(' | '[' => depth += 1,
                ')' | ']' => depth -= 1,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        m += 1;
                        break;
                    }
                }
                ';' if depth == 0 => {
                    m += 1;
                    break;
                }
                _ => {}
            }
            m += 1;
        }
        for x in code.iter_mut().take(m.min(c.len())).skip(start) {
            if *x != '\n' {
                *x = ' ';
            }
        }
        i = m;
    }
}

/// The lines of `text` that hold a `KF3_NEGCTL_` name in code that the `negctl` gate does not cover.
fn ungated(text: &str) -> Vec<(usize, String)> {
    let mut code = strip_comments(text);
    strip_gated(&mut code);
    let code: String = code.into_iter().collect();
    code.lines()
        .enumerate()
        .filter(|(_, l)| l.contains(NAME))
        .map(|(n, l)| (n + 1, l.trim().to_string()))
        .collect()
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn files(dir: &Path, suffixes: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            files(&p, suffixes, out);
        } else if p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| suffixes.contains(&e))
        {
            out.push(p);
        }
    }
}

#[test]
fn every_negctl_name_in_the_product_crates_sits_inside_the_feature_gate() {
    let crates = root().join("crates");
    let mut all = Vec::new();
    let mut dirs: Vec<_> = std::fs::read_dir(&crates)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    dirs.sort();
    for d in dirs {
        // the v3 product crates; `kayfabe-*` is the frozen pre-v3 tree and is out of scope
        if d.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("kf-"))
        {
            files(&d.join("src"), &["rs"], &mut all);
            if d.join("build.rs").is_file() {
                all.push(d.join("build.rs"));
            }
        }
    }
    assert!(
        all.len() > 100,
        "scanned only {} files: the scan is looking in the wrong place",
        all.len()
    );
    let mut bad = Vec::new();
    for f in &all {
        let text = std::fs::read_to_string(f).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        for (n, l) in ungated(&text) {
            bad.push(format!(
                "{}:{n}: {l}",
                f.strip_prefix(root()).unwrap_or(f).display()
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "a KF3_NEGCTL_ name is in code that a release build compiles; put the read behind #[cfg(feature = \"negctl\")] \
         with a constant twin under #[cfg(not(feature = \"negctl\"))]:\n{}",
        bad.join("\n")
    );
}

#[test]
fn the_c_sources_name_no_negctl_switch() {
    let mut all = Vec::new();
    for d in ["qemu/hw/misc/kf3", "broker", "cuda/walk", "firmware"] {
        files(&root().join(d), &["c", "h", "cu", "cuh"], &mut all);
    }
    let bad: Vec<_> = all
        .iter()
        .filter(|f| std::fs::read_to_string(f).is_ok_and(|t| t.contains(NAME)))
        .map(|f| f.display().to_string())
        .collect();
    assert!(
        bad.is_empty(),
        "C has no feature gate, so no C source may name a control switch: {bad:?}"
    );
}

// ---- the scan itself ----

const READ: &str = "fn f() -> bool { std::env::var_os(\"KF3_NEGCTL_X\").is_some() }\n";

#[test]
fn an_unguarded_read_is_found() {
    assert_eq!(ungated(READ).len(), 1);
    let two = format!(
        "{READ}// KF3_NEGCTL_Y in a comment\n/// KF3_NEGCTL_Z in a doc\nconst S: &str = \"KF3_NEGCTL_W\";\n"
    );
    assert_eq!(
        ungated(&two).len(),
        2,
        "the comments are not code, the literal is"
    );
}

#[test]
fn a_gate_on_the_wrong_feature_or_the_negated_gate_does_not_cover_a_read() {
    for attr in [
        "#[cfg(feature = \"other\")]",
        "#[cfg(not(feature = \"negctl\"))]",
        "#[cfg(test)]",
        "#[inline]",
    ] {
        assert_eq!(ungated(&format!("{attr}\n{READ}")).len(), 1, "{attr}");
    }
}

#[test]
fn a_gated_item_or_statement_is_covered_and_the_code_after_it_is_not() {
    for attr in [
        "#[cfg(feature = \"negctl\")]",
        "#[cfg( feature = \"negctl\" )]",
    ] {
        assert!(ungated(&format!("{attr}\n{READ}")).is_empty(), "{attr}");
        assert!(ungated(&format!("{attr}\nconst S: &str = \"KF3_NEGCTL_A\";\n")).is_empty());
        assert!(
            ungated(&format!(
                "fn g() {{\n    {attr}\n    eprintln!(\n        \"KF3_NEGCTL_A: x;y\"\n    );\n}}\n"
            ))
            .is_empty()
        );
        assert!(ungated(&format!("fn g() {{\n    {attr}\n    {{\n        if x {{ eprintln!(\"KF3_NEGCTL_A\"); }}\n    }}\n}}\n")).is_empty());
        assert_eq!(
            ungated(&format!("{attr}\n{READ}{READ}")).len(),
            1,
            "only the item after the attribute is gated"
        );
    }
    let tricky =
        "#[cfg(feature = \"negctl\")]\nfn a() -> char { '}' }\nconst B: &str = \"KF3_NEGCTL_B\";\n";
    assert_eq!(
        ungated(tricky).len(),
        1,
        "a brace in a char literal does not end the gated fn early"
    );
    let raw = "#[cfg(feature = \"negctl\")]\nfn a() -> &'static str { r#\"}\"# }\nconst B: &str = \"KF3_NEGCTL_B\";\n";
    assert_eq!(
        ungated(raw).len(),
        1,
        "a brace in a raw string does not end the gated fn early"
    );
}
