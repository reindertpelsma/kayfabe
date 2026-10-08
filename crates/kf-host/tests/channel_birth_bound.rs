// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★★ **The channel-birth bound, checked in the source** (`crate::birth`;
//! `docs/design/THE_CONSTRAINTS.md` §30).
//!
//! The adversarial review of `v3-sec-nonpriv` (2026-10-03) showed a mutant that every test
//! passed: `birth_member` with the `CAP_SYS_ADMIN` bracket removed and the reply check replaced
//! by `Ok`. The behaviour is now tested in `src/birth.rs` against a simulated root thread. This
//! file pins the **shape** those tests cannot see, in the style of
//! `kayfabe-isolate-host/tests/own_client_invariant.rs`:
//!
//! 1. every function in this crate that builds an `NV_ESC_RM_ALLOC` request calls
//!    `admit_alloc_class` before it (the bound sits on every alloc path, including the public
//!    `raw_alloc`, `raw_alloc_via` and `raw_alloc_nested`);
//! 2. the token that admits a channel class is made in exactly one place, `born_user`, and
//!    cannot be copied or defaulted into existence;
//! 3. `born_user` issues inside the bracket and checks the reply after it, freeing on refusal;
//! 4. `birth_member`, the one caller, goes through `born_user` on the live thread, and the
//!    channel role is unwrapped nowhere else.
//!
//! ⊘ These read source text. They pin a shape and say nothing about what RM does; the box
//! census (`scripts/bench/box/merge_check.sh`, the `channel birth` lines) is what reads RM.

use std::path::{Path, PathBuf};

fn src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every `.rs` file under `src/`, as `(relative path, code)`: comments and the contents of string
/// and char literals blanked to spaces (positions kept), and `#[cfg(test)]` modules removed.
fn sources() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![src_dir()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).expect("read src") {
            let p = e.expect("entry").path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let raw = std::fs::read_to_string(&p).expect("read");
                let rel = p
                    .strip_prefix(src_dir())
                    .expect("under src")
                    .display()
                    .to_string();
                out.push((rel, without_test_modules(&blank(&raw))));
            }
        }
    }
    out.sort();
    assert!(
        out.len() >= 3,
        "★ NON-VACUITY: only {} source files found under {}",
        out.len(),
        src_dir().display()
    );
    out
}

/// Blank comments and literal contents, so braces and words inside them never count.
fn blank(s: &str) -> String {
    let b: Vec<char> = s.chars().collect();
    let mut o = b.clone();
    let mut i = 0;
    let n = b.len();
    while i < n {
        if b[i] == '/' && i + 1 < n && b[i + 1] == '/' {
            while i < n && b[i] != '\n' {
                o[i] = ' ';
                i += 1;
            }
        } else if b[i] == '/' && i + 1 < n && b[i + 1] == '*' {
            let mut depth = 0;
            while i < n {
                if b[i] == '/' && i + 1 < n && b[i + 1] == '*' {
                    depth += 1;
                    o[i] = ' ';
                    o[i + 1] = ' ';
                    i += 2;
                } else if b[i] == '*' && i + 1 < n && b[i + 1] == '/' {
                    depth -= 1;
                    o[i] = ' ';
                    o[i + 1] = ' ';
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if b[i] != '\n' {
                        o[i] = ' ';
                    }
                    i += 1;
                }
            }
        } else if b[i] == 'r' && i + 1 < n && (b[i + 1] == '"' || b[i + 1] == '#') {
            // Raw string r"..." / r#"..."#.
            let mut j = i + 1;
            let mut hashes = 0;
            while j < n && b[j] == '#' {
                hashes += 1;
                j += 1;
            }
            if j < n && b[j] == '"' {
                j += 1;
                loop {
                    if j >= n {
                        break;
                    }
                    if b[j] == '"' && (0..hashes).all(|k| j + 1 + k < n && b[j + 1 + k] == '#') {
                        j += 1 + hashes;
                        break;
                    }
                    if b[j] != '\n' {
                        o[j] = ' ';
                    }
                    j += 1;
                }
                i = j;
            } else {
                i += 1;
            }
        } else if b[i] == '"' {
            i += 1;
            while i < n && b[i] != '"' {
                if b[i] == '\\' && i + 1 < n {
                    o[i] = ' ';
                    if b[i + 1] != '\n' {
                        o[i + 1] = ' ';
                    }
                    i += 2;
                    continue;
                }
                if b[i] != '\n' {
                    o[i] = ' ';
                }
                i += 1;
            }
            i += 1;
        } else if b[i] == '\'' {
            // A char literal ('x', '\n', '\u{..}') — never a lifetime.
            if i + 2 < n && b[i + 1] != '\\' && b[i + 2] == '\'' {
                o[i + 1] = ' ';
                i += 3;
            } else if i + 1 < n && b[i + 1] == '\\' {
                let mut j = i + 1;
                while j < n && b[j] != '\'' {
                    o[j] = ' ';
                    j += 1;
                }
                i = j + 1;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    o.into_iter().collect()
}

/// The index just past the `}` matching the `{` at `open`.
fn matching_brace(s: &str, open: usize) -> usize {
    let b = s.as_bytes();
    assert_eq!(b[open], b'{');
    let mut depth = 0usize;
    for (k, &c) in b.iter().enumerate().skip(open) {
        if c == b'{' {
            depth += 1;
        } else if c == b'}' {
            depth -= 1;
            if depth == 0 {
                return k + 1;
            }
        }
    }
    panic!("unbalanced braces from byte {open}");
}

fn without_test_modules(code: &str) -> String {
    let mut s = code.to_string();
    while let Some(at) = s.find("#[cfg(test)]") {
        let open = at + s[at..].find('{').expect("a test module body");
        let end = matching_brace(&s, open);
        s.replace_range(at..end, "");
    }
    s
}

/// Every `fn` item in `code`: `(name, body)`. Bodiless declarations are skipped.
fn functions(code: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = code[from..].find("fn ") {
        let at = from + rel;
        from = at + 3;
        if at > 0 {
            let prev = code.as_bytes()[at - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                continue;
            }
        }
        let name: String = code[at + 3..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() {
            continue;
        }
        let rest = &code[at..];
        let (Some(brace), semi) = (rest.find('{'), rest.find(';')) else {
            continue;
        };
        if semi.is_some_and(|s| s < brace) {
            continue;
        }
        let open = at + brace;
        let end = matching_brace(code, open);
        out.push((name, code[open..end].to_string()));
    }
    out
}

fn all_functions() -> Vec<(String, String, String)> {
    sources()
        .into_iter()
        .flat_map(|(file, code)| {
            functions(&code)
                .into_iter()
                .map(move |(name, body)| (file.clone(), name, body))
        })
        .collect()
}

fn one_fn(file: &str, name: &str) -> String {
    let hits: Vec<_> = all_functions()
        .into_iter()
        .filter(|(f, n, _)| f == file && n == name)
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "★ NON-VACUITY: expected exactly one `fn {name}` in src/{file}, found {}",
        hits.len()
    );
    hits.into_iter().next().expect("one").2
}

/// ★★★ (1) The bound sits on EVERY alloc path. The universe is derived: every function body
/// that names `NV_ESC_RM_ALLOC` (the only way to build an alloc request), not a list of names.
#[test]
fn every_alloc_request_is_admitted_first() {
    let sites: Vec<_> = all_functions()
        .into_iter()
        .filter(|(_, _, body)| body.contains("NV_ESC_RM_ALLOC"))
        .collect();
    assert!(
        sites.len() >= 4,
        "★ NON-VACUITY: {} function(s) build an NV_ESC_RM_ALLOC request; expected at least 4 \
         (allocate_root, raw_alloc_exact, raw_alloc_via_exact, raw_alloc_nested). The scanner \
         is broken — treat as RED, do not lower the floor.",
        sites.len()
    );
    let mut bad = Vec::new();
    for (file, name, body) in &sites {
        let alloc = body.find("NV_ESC_RM_ALLOC").expect("filtered");
        match body.find("admit_alloc_class(") {
            Some(a) if a < alloc => {}
            _ => bad.push(format!("src/{file}: fn {name}")),
        }
    }
    assert!(
        bad.is_empty(),
        "★★★ CHANNEL-BIRTH BOUND BREACHED — these build an NV_ESC_RM_ALLOC request without \
         calling `birth::admit_alloc_class` first, so a channel class could reach host RM \
         without CAP_SYS_ADMIN cleared and without RM's reply being checked:\n  {}",
        bad.join("\n  ")
    );
}

/// ★★★ (2) The admitting token is unforgeable and made in one place.
#[test]
fn the_birth_token_is_made_only_by_born_user() {
    let srcs = sources();
    let birth = &srcs
        .iter()
        .find(|(f, _)| f == "birth.rs")
        .expect("★ NON-VACUITY: src/birth.rs is gone")
        .1;
    assert!(
        birth.contains("pub(crate) struct InsideBirthPath {\n    _private: (),\n}"),
        "★★★ `InsideBirthPath` no longer has exactly one private field; a public or tuple field \
         lets any code in the crate admit a channel class"
    );
    let def = birth
        .find("pub(crate) struct InsideBirthPath")
        .expect("definition");
    let derives = &birth[birth[..def].rfind("#[derive(").unwrap_or(def)..def];
    for forbidden in ["Clone", "Copy", "Default"] {
        assert!(
            !derives.contains(forbidden),
            "★★★ `InsideBirthPath` derives {forbidden}: an issuing closure could keep a token \
             past the bracket"
        );
    }
    for (file, code) in &srcs {
        for forbidden in [
            "impl Clone for InsideBirthPath",
            "impl Copy for InsideBirthPath",
            "impl Default for InsideBirthPath",
        ] {
            assert!(!code.contains(forbidden), "★★★ src/{file}: `{forbidden}`");
        }
        let made = code.matches("InsideBirthPath { _private").count();
        let expect = usize::from(file == "birth.rs");
        assert_eq!(
            made, expect,
            "★★★ src/{file} constructs `InsideBirthPath` {made} time(s); the one construction \
             must be in `born_user`"
        );
    }
    assert!(
        one_fn("birth.rs", "born_user").contains("InsideBirthPath { _private"),
        "★★★ the token is constructed somewhere other than `born_user`"
    );
}

/// ★★★ (3) `born_user`: bracket, then the reply check, then free on refusal — in that order.
#[test]
fn born_user_brackets_the_call_and_checks_the_reply() {
    let body = one_fn("birth.rs", "born_user");
    let bracket = body
        .find("with_effective_cap_cleared_on(ops, CAP_SYS_ADMIN, call)")
        .expect(
            "★★★ `born_user` no longer issues the call inside \
             `with_effective_cap_cleared_on(ops, CAP_SYS_ADMIN, call)`",
        );
    let issue = body
        .find("issue(&token, params)")
        .expect("★★★ `born_user` no longer hands the token to `issue`");
    let check = body
        .find("birth_privilege(request_flags, params)")
        .expect("★★★ `born_user` no longer reads RM's reply with `birth_privilege`");
    let free = body
        .find("free(handle)")
        .expect("★★★ `born_user` no longer frees a refused channel");
    assert!(
        issue < bracket && bracket < check && check < free,
        "★★★ `born_user` is out of order: the call must be made inside the bracket, and the \
         reply checked (and a refused channel freed) after it returns"
    );
    // The bracket is skipped only by the named negative control, which the reply check still
    // follows.
    assert_eq!(
        body.matches("call()").count(),
        1,
        "★★★ `born_user` calls `issue` outside the bracket somewhere other than the negative \
         control"
    );
    assert!(body.contains("if skip_bracket {"));
}

/// ★★★ (4) The one caller: `birth_member` goes through `born_user` on the live thread, and the
/// channel class is unwrapped nowhere else in the crate.
#[test]
fn birth_member_is_the_one_caller_and_uses_the_live_thread() {
    let body = one_fn("channel.rs", "birth_member");
    assert!(
        body.contains("crate::birth::born_user(") && body.contains("capability::ThisThread"),
        "★★★ `birth_member` no longer allocates through `born_user` on the calling thread"
    );
    let unwraps: Vec<_> = all_functions()
        .into_iter()
        .filter(|(_, _, b)| b.contains("gpfifo_channel()"))
        .map(|(f, n, _)| format!("src/{f}: fn {n}"))
        .collect();
    assert_eq!(
        unwraps,
        vec!["src/channel.rs: fn birth_member".to_string()],
        "★★★ the channel class is named outside `birth_member`"
    );
    let callers: Vec<_> = all_functions()
        .into_iter()
        .filter(|(_, n, b)| n != "born_user" && b.contains("born_user("))
        .map(|(f, n, _)| format!("src/{f}: fn {n}"))
        .collect();
    assert_eq!(
        callers,
        vec!["src/channel.rs: fn birth_member".to_string()],
        "a second caller of `born_user` — check it uses the live thread, then widen this"
    );
}

/// The scanner's own known-positive: a function that builds an alloc request without the
/// admission is caught, and one with it passes. (A scanner that has only ever reported zero has
/// not shown it can report one.)
#[test]
fn the_scanner_catches_a_planted_unadmitted_alloc() {
    let planted = blank(
        "fn bad(&self) { let s = \"{ not a brace }\"; let req = NV_ESC_RM_ALLOC; }\n\
         fn good(&self) { birth::admit_alloc_class(c, None)?; let req = NV_ESC_RM_ALLOC; }\n\
         // fn commented(&self) { NV_ESC_RM_ALLOC }\n",
    );
    let fns = functions(&planted);
    let names: Vec<_> = fns.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["bad", "good"]);
    let flagged: Vec<_> = fns
        .iter()
        .filter(|(_, b)| {
            let a = b.find("NV_ESC_RM_ALLOC").expect("alloc");
            !b.find("admit_alloc_class(").is_some_and(|x| x < a)
        })
        .map(|(n, _)| n.as_str())
        .collect();
    assert_eq!(flagged, ["bad"]);
}

/// ★ Review 2026-10-08 (finding 2): the belt-off value is a TYPE now, and only the physical-operand
/// oracle may name it. A production caller that wrote `PhysicalCeBelt::Off` (or the two booleans of
/// the old signature, swapped) would turn off `DENY_PHYSICAL_MODE_CE` on production channels.
#[test]
fn only_the_oracle_names_the_belt_off_and_the_passthrough_birth_names_deny() {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).expect("read dir") {
            let p = e.expect("entry").path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for c in std::fs::read_dir(&crates).expect("crates dir") {
        let src = c.expect("entry").path().join("src");
        if src.is_dir() {
            walk(&src, &mut files);
        }
    }
    let code = |p: &Path| -> String {
        std::fs::read_to_string(p)
            .expect("read")
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut naming_off: Vec<String> = files
        .iter()
        .filter(|p| code(p).contains("PhysicalCeBelt::Off"))
        .map(|p| {
            p.strip_prefix(&crates)
                .expect("under crates")
                .display()
                .to_string()
        })
        .collect();
    naming_off.sort();
    assert_eq!(
        naming_off,
        vec!["kf-harness/src/bin/kf-phys-oracle.rs".to_string()],
        "★★★ another source file names `PhysicalCeBelt::Off`: production channels must keep the belt"
    );
    // non-vacuity: the production Passthrough birth does name the belt, and it says Deny
    let pt = crates.join("kf-chan/src/passthrough.rs");
    assert!(
        code(&pt).contains("PhysicalCeBelt::Deny"),
        "★ the Passthrough birth no longer passes `PhysicalCeBelt::Deny`"
    );
}
