//! ★★★ **F11: `hClient` is never guest-derived, and never a client we did not mint.**
//!
//! `guest_blast_radius.md` §4 F11 is the finding this file exists for. Its short form:
//!
//! * `surrender_privilege` drops **capabilities, not uid** — the user-namespace map is the
//!   single line `0 <outer_uid> 1` (`crates/kayfabe-linux-raw/src/sandbox_unsafe.rs:596-617`),
//!   so on a **root VMM** the isolate's euid *as the host kernel sees it* is 0;
//! * RM keys a real check on that euid, and it is an **OR** — a matching euid **alone**
//!   passes (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/os.c:3844-3868`, driven from
//!   `_rmclientUserClientSecurityCheck`, `ogkm-580: src/nvidia/src/kernel/rmapi/client.c:447-512`),
//!   and it is on by default (`ogkm-580: src/nvidia/generated/g_system_nvoc.c:103`);
//! * ⇒ a local unprivileged process fails that check against a root-owned RM client and
//!   **we pass it**. RM's cross-user client-handle protection does not stand between this
//!   isolate and the host's root GPU clients.
//!
//! That widening is **latent and not live** for exactly one reason: *we have no way to name
//! a client we did not mint.* Before task #133 that reason was **one line** —
//! `RmConnection::raw_alloc` took a `root: u32` and every caller happened to pass
//! `self.client`. Nothing said so, and nothing would have gone red.
//!
//! ## What is STRUCTURAL and what is TESTED — the split is the honest part
//!
//! **Structural (a wrong call site is `error[E0…]`, not a red test).** `rm.rs` has a private
//! `mod own_client` whose `OwnClient(u32)` has a private field and exactly one constructor,
//! `OwnClient::allocate_root`, which *performs* the `NV01_ROOT_CLIENT` allocation. So *"an
//! `OwnClient` exists"* and *"this process minted that client"* are **one statement**, and
//! `raw_alloc` no longer has a client parameter at all. A caller cannot express the wrong
//! thing.
//!
//! **Tested (this file).** The ABI parameter blocks in `kayfabe-abi` type their client
//! fields as plain `u32`, and typing them is a crate-wide change deliberately not made
//! here. So a **new struct literal** in `rm.rs` could still write `h_client: 0xdead_beef`
//! and compile. This file is that residue's gate, and it is the weaker half — say so rather
//! than let the module docs imply the whole invariant is compiler-enforced.
//!
//! ## ★★ The universe is DERIVED, not listed
//!
//! `gates_quantified_over_a_list` is a standing lesson in this project: shortening the list
//! weakens the gate with **zero red tests**. So the set of field names that count as *"an RM
//! client field"* is **read out of `kayfabe-abi`** at test time — every `pub <name>:` whose
//! name mentions `client`, `root` or `owner` — rather than written here. Adding
//! `h_client_src` to an ABI struct and then using it in `rm.rs` turns this gate red without
//! anyone remembering it exists.
//!
//! ⊘ **No GPU, no driver, no run stands behind any of this.** These tests read source text.
//! They pin a shape; they say nothing about what RM does, and F11's underlying question —
//! whether the euid widening is exploitable at all — remains `[unknown]`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root is two levels above this crate")
}

fn rm_rs() -> String {
    let p = repo_root().join("crates/kayfabe-isolate-host/src/rm.rs");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// `rm.rs` with every comment line removed.
///
/// ★ Load-bearing, not tidiness: this file's own subject matter means `rm.rs`'s **prose**
/// is full of the exact strings being gated (`h_client: <some other u32>` appears in
/// `mod own_client`'s docs as the thing it does *not* close). A scanner that read comments
/// would fire on the documentation of the invariant it is checking.
fn rm_rs_code_only() -> String {
    rm_rs()
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("//") || t.starts_with("*") || t.starts_with("/*"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every field name in `kayfabe-abi` that could carry an RM **client** handle, read out of
/// the ABI source rather than written down here.
///
/// The match is deliberately loose (`client` / `root` / `owner` anywhere in the name): a
/// gate that over-approximates the universe fails **closed**. A new ABI field this test has
/// never heard of is covered the day it is declared.
fn client_field_names() -> BTreeSet<String> {
    let abi = repo_root().join("crates/kayfabe-abi/src");
    let mut names = BTreeSet::new();
    let mut stack = vec![abi];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read kayfabe-abi/src") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("read an abi source file");
            for line in text.lines() {
                let t = line.trim();
                let Some(rest) = t.strip_prefix("pub ") else {
                    continue;
                };
                let Some((name, _)) = rest.split_once(':') else {
                    continue;
                };
                let name = name.trim();
                if name.is_empty()
                    || !name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                {
                    continue;
                }
                if name.contains("client") || name.contains("root") || name.contains("owner") {
                    names.insert(name.to_string());
                }
            }
        }
    }
    names
}

/// The byte spans of every `struct … { … }` **declaration** in `rm.rs`.
///
/// ★ Needed because a declaration and an initialiser are the same shape to a line scanner:
/// `client: OwnClient,` (a field's *type*) and `h_client: self.client.raw(),` (a field's
/// *value*) both match `name: rhs`. The first is not an RM escape and gating it is
/// nonsense — this gate is about what gets **written into an ioctl argument**, and a type
/// is never that. Found by brace matching rather than by guessing at the right-hand side,
/// because "does this look like a type?" is exactly the kind of heuristic that goes wrong
/// silently.
fn struct_decl_spans(code: &str) -> Vec<(usize, usize)> {
    let bytes = code.as_bytes();
    let mut spans = Vec::new();
    let mut search = 0usize;
    while let Some(rel) = code[search..].find("struct ") {
        let at = search + rel;
        search = at + 7;
        // A tuple struct (`struct OwnClient(u32);`) has no brace block to skip.
        let Some(stop) = code[at..].find(['{', ';', '(']) else {
            break;
        };
        if code.as_bytes()[at + stop] != b'{' {
            continue;
        }
        let open = at + stop;
        let mut depth = 0usize;
        for (i, b) in bytes.iter().enumerate().skip(open) {
            match b {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        spans.push((open, i));
                        search = i;
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    spans
}

/// The byte span of `mod own_client { … }` in `rm.rs`, by brace matching.
///
/// The one legitimate `hRoot = 0` in the crate lives inside it — a root-client allocation
/// is the single escape with no owning client, which is exactly why it is the single
/// constructor. Everywhere else, `0` would be a bug.
fn own_client_module_span(code: &str) -> (usize, usize) {
    let start = code
        .find("mod own_client {")
        .expect("★ NON-VACUITY: `mod own_client` is gone from rm.rs — F11's invariant has no home");
    let open = start + code[start..].find('{').expect("an opening brace");
    let bytes = code.as_bytes();
    let mut depth = 0usize;
    for (i, b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return (start, i);
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced braces scanning `mod own_client`");
}

/// The RHS forms an RM client field is allowed to be filled from, and nothing else.
///
/// Both are an [`OwnClient`] unwrapped at the ABI boundary. That the list is short is the
/// point: any *other* expression is a client this code did not mint, or cannot prove it
/// minted, and either way it is the thing F11 says must not become possible.
const APPROVED_RHS: &[&str] = &["self.client.raw()", "self.conn.client.raw()"];

/// ★ The number of client-field initialisers `rm.rs` had when this gate was written.
///
/// A **floor**, and a literal on purpose — for the reason `scripts/run_full_suite.sh`
/// spells out at length: a count derived from the thing it checks moves silently when that
/// thing moves. If the scanner stops matching (a rename, a refactor, a formatting change
/// that puts the field and its value on different lines) this floor is what turns the
/// resulting **zero findings** into a red test instead of a vacuous green.
const CLIENT_FIELD_SITES_FLOOR: usize = 9;

#[test]
fn every_rm_escape_in_rm_rs_stamps_the_isolates_own_client() {
    let code = rm_rs_code_only();
    let (mod_start, mod_end) = own_client_module_span(&code);
    let decls = struct_decl_spans(&code);
    let fields = client_field_names();
    assert!(
        fields.contains("h_client") && fields.contains("h_root") && fields.contains("owner"),
        "★ NON-VACUITY: the derived client-field universe lost a name it must contain \
         — the scan of kayfabe-abi is broken, not the tree. Got: {fields:?}"
    );

    let mut sites = 0usize;
    let mut bad = Vec::new();
    for (idx, line) in code.lines().enumerate() {
        let t = line.trim();
        let Some((name, value)) = t.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if !fields.contains(name) {
            continue;
        }
        let value = value.trim().trim_end_matches(',').trim();
        // Byte offset of this line, to decide whether it is inside `mod own_client`.
        let offset = code.lines().take(idx).map(|l| l.len() + 1).sum::<usize>();
        // A field *declaration* names a type, never an ioctl argument. Not our subject.
        if decls.iter().any(|(a, b)| offset > *a && offset < *b) {
            continue;
        }
        let inside_own_client = offset > mod_start && offset < mod_end;

        sites += 1;
        let ok = if inside_own_client {
            // The root-client allocation: no owning client exists yet, by construction.
            value == "0" || APPROVED_RHS.contains(&value)
        } else {
            APPROVED_RHS.contains(&value)
        };
        if !ok {
            bad.push(format!(
                "  rm.rs (code line {}): `{name}: {value}`",
                idx + 1
            ));
        }
    }

    assert!(
        sites >= CLIENT_FIELD_SITES_FLOOR,
        "★ NON-VACUITY: found only {sites} RM client-field initialiser(s) in rm.rs, floor is \
         {CLIENT_FIELD_SITES_FLOOR}. This gate has stopped seeing the thing it gates — treat \
         it as RED and fix the scanner, do NOT lower the floor."
    );
    assert!(
        bad.is_empty(),
        "★★★ F11 VIOLATED — an RM escape in rm.rs names a client that is not this isolate's \
         own minted `OwnClient` ({} site(s)):\n{}\n\nWhy this is not a style rule: on a root \
         VMM the isolate's kernel-visible euid is 0, and RM's cross-user client check is an \
         OR on euid (`ogkm-580: os.c:3844-3868`), so a client handle we did not mint is one \
         we would be *permitted* to drive. Fill the field from `self.client.raw()` / \
         `self.conn.client.raw()`, or read `mod own_client`'s docs before widening this.\n\
         Approved forms: {APPROVED_RHS:?}",
        bad.len(),
        bad.join("\n"),
    );
}

#[test]
fn raw_alloc_takes_no_caller_supplied_client() {
    let code = rm_rs_code_only();
    let at = code
        .find("fn raw_alloc(")
        .expect("★ NON-VACUITY: `fn raw_alloc` is gone — this gate no longer gates anything");
    let sig_end = at + code[at..].find(')').expect("a closing paren");
    let sig = &code[at..sig_end];
    assert!(
        !sig.contains("root"),
        "★★★ F11 REGRESSED — `raw_alloc` has regained a caller-supplied client parameter:\n\
         {sig}\n\nThis is the exact one-line shape F11 names. The client must be stamped by \
         the function from `self.client`, so that a call site cannot express a foreign \
         client at all."
    );
}

#[test]
fn own_client_is_unforgeable() {
    let code = rm_rs_code_only();
    let (start, end) = own_client_module_span(&code);
    let module = &code[start..end];

    // The field must be private: `OwnClient(u32)`, never `OwnClient(pub u32)`.
    assert!(
        module.contains("struct OwnClient(u32);"),
        "★★★ F11 REGRESSED — `OwnClient`'s wrapped handle is no longer a private field. \
         A public field is a `u32 -> OwnClient` conversion with extra steps."
    );

    // Exactly one constructor, and it is the allocation itself.
    let ctor = module.matches("-> Result<Self, RmError>").count();
    assert_eq!(
        ctor, 1,
        "★★★ F11 REGRESSED — `mod own_client` exposes {ctor} constructor-shaped functions, \
         expected exactly 1 (`allocate_root`). The invariant is that *having* an `OwnClient` \
         and *having minted it* are one fact; a second constructor splits them back apart."
    );
    assert!(
        module.contains("pub(super) fn allocate_root("),
        "★ NON-VACUITY: `allocate_root` is not where this gate thinks it is"
    );

    // The conversions that would re-open the hole, by name.
    for forbidden in [
        "impl From<u32> for OwnClient",
        "fn new(",
        "derive(Default)",
        "impl Default for OwnClient",
        "fn from_raw(",
    ] {
        assert!(
            !module.contains(forbidden),
            "★★★ F11 REGRESSED — `mod own_client` now contains `{forbidden}`, which \
             manufactures an `OwnClient` from a value nobody minted. That is precisely the \
             `u32 -> OwnClient` direction the type exists to make impossible."
        );
    }
}
