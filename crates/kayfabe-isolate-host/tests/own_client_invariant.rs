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

/// The byte spans of every **function signature's parameter list** in `rm.rs`.
///
/// ★ Needed for exactly the reason [`struct_decl_spans`] is, and the argument is the same
/// one: a line scanner cannot tell `client: u32` (a **parameter**, i.e. a type) from
/// `h_client: self.client.raw()` (a field's **value**, i.e. an ioctl argument). Rust's
/// grammar can — inside a signature's parens the right-hand side of every `:` is a *type*,
/// and a type is never written into an ioctl.
///
/// ⊘ **Only `fn name(` DECLARATIONS**, never call sites, so a struct literal passed as an
/// argument (`foo(Nvos46Parameters { h_client: … })`) is not inside one of these spans and
/// is still gated.
///
/// ⚠ **A runaway span is refused rather than trusted.** Paren matching over raw text can go
/// wrong (a `'('` char literal, a parenthesis inside a string), and a span that swallowed
/// half the file would silently exempt real call sites. So a span is kept only if what
/// follows its closing paren is `->`, `{` or `;` — the three things that can legally follow
/// a signature's parameter list. Anything else means the match went astray, and the span is
/// dropped: the gate then over-reports rather than under-reports, which is the direction a
/// security gate must fail in.
fn fn_signature_spans(code: &str) -> Vec<(usize, usize)> {
    let bytes = code.as_bytes();
    let mut spans = Vec::new();
    let mut search = 0usize;
    while let Some(rel) = code[search..].find("fn ") {
        let at = search + rel;
        search = at + 3;
        let Some(open_rel) = code[at..].find('(') else {
            break;
        };
        let open = at + open_rel;
        // Between `fn ` and `(` there must be nothing but an identifier and (for a generic)
        // angle brackets — otherwise this is not a signature at all.
        let head = &code[at + 3..open];
        if !head
            .chars()
            .all(|c| c.is_alphanumeric() || "_<>, ':&".contains(c))
        {
            continue;
        }
        let mut depth = 0usize;
        for (i, b) in bytes.iter().enumerate().skip(open) {
            match b {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        let tail = code[i + 1..].trim_start();
                        if tail.starts_with("->") || tail.starts_with('{') || tail.starts_with(';')
                        {
                            spans.push((open, i));
                        }
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

/// ★★★★★ **CONSTRAINT 26 — THE ONE SCOPED EXCEPTION, AND IT IS A TYPE RATHER THAN A STRING.**
///
/// > `THE_CONSTRAINTS.md` §26: *"F11 is SCOPED, not eliminated. Someone still names a client
/// > they did not mint — the **scratchpad**, dup'ing the isolate's VA space. The rule is: a
/// > per-proc isolate may never name a foreign client; the scratchpad may, and only for a VA
/// > space the VMM handed it. Enforce as a **newtype**, the way `OwnClient` already does, so
/// > the approved set grows by a TYPE and not by a string on an allowlist."*
///
/// ⊘ **[`APPROVED_RHS`] is NOT widened**, and that is the whole point of this being a
/// separate function. The fields it governs — every client field that is not a `*_src` —
/// keep exactly the two approved forms they had. What grows is the **universe**: `NVOS55`
/// introduced a field whose meaning is *"the OTHER client"*, which the destination rule
/// cannot express at all, and it gets its own rule with its own floor.
///
/// ## ★★ And its approved set is DERIVED from `mod handed_vaspace`, not written here
///
/// `gates_quantified_over_a_list` is a standing lesson in this project. The accessors
/// `HandedVaSpace` exposes are read out of `rm.rs` at test time, so renaming one does not
/// silently un-gate the field, and **adding** one is a deliberate widening of a type whose
/// unforgeability `handed_vaspace_is_unforgeable` checks separately.
fn approved_src_rhs(code: &str) -> BTreeSet<String> {
    let (start, end) = handed_vaspace_module_span(code);
    let module = &code[start..end];
    let mut out = BTreeSet::new();
    let mut search = 0usize;
    while let Some(rel) = module[search..].find("pub(super) fn ") {
        let at = search + rel + "pub(super) fn ".len();
        search = at;
        let Some(paren) = module[at..].find('(') else {
            break;
        };
        let name = module[at..at + paren].trim();
        // Only the accessors — `self`-taking, `-> u32`. A constructor is not an RHS.
        let sig_end = module[at..].find("->").map(|o| at + o).unwrap_or(at);
        if !module[at..sig_end].contains("self") {
            continue;
        }
        if !module[sig_end..].starts_with("-> u32") {
            continue;
        }
        out.insert(name.to_string());
    }
    out
}

/// Does `value` read a [`HandedVaSpace`] accessor — `<something>.<accessor>()`?
fn is_handed_accessor(value: &str, accessors: &BTreeSet<String>) -> bool {
    accessors
        .iter()
        .any(|a| value.ends_with(&format!(".{a}()")) && !value.contains(' '))
}

/// The byte span of `mod handed_vaspace { … }` in `rm.rs`, by brace matching.
/// The byte span of `mod handed_client` inside `rm.rs` — constraint 32's half of the same
/// scoping. ⊘ A separate span from `handed_vaspace`'s on purpose: the two types widen F11 in
/// two different fields (`hClientSrc` vs `hRoot`), and a test that scanned one span for both
/// would pass while the other was deleted.
fn handed_client_module_span(code: &str) -> (usize, usize) {
    let start = code.find("mod handed_client {").expect(
        "★ NON-VACUITY: `mod handed_client` is gone from rm.rs — constraint 32's birth-client \
         exception has no home, and any escape `hRoot` is therefore ungated",
    );
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
    panic!("unbalanced braces scanning `mod handed_client`");
}

fn handed_vaspace_module_span(code: &str) -> (usize, usize) {
    let start = code.find("mod handed_vaspace {").expect(
        "★ NON-VACUITY: `mod handed_vaspace` is gone from rm.rs — constraint 26's scoped \
         exception has no home, and any `*_src` client field is therefore ungated",
    );
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
    panic!("unbalanced braces scanning `mod handed_vaspace`");
}

/// ★ The number of **source**-client initialisers `rm.rs` had when this rule was written.
/// A floor, for [`CLIENT_FIELD_SITES_FLOOR`]'s reason: a scanner that stops matching must
/// turn its zero findings into a red test rather than a vacuous green.
const SRC_CLIENT_SITES_FLOOR: usize = 1;

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

    let params = fn_signature_spans(&code);
    let accessors = approved_src_rhs(&code);
    assert!(
        !accessors.is_empty(),
        "★ NON-VACUITY: `mod handed_vaspace` exposes no `-> u32` accessor, so the scoped \
         source-client rule below has an EMPTY approved set and would refuse the one call \
         site constraint 26 sanctions — or, worse, would be read as having nothing to gate."
    );

    let mut sites = 0usize;
    let mut src_sites = 0usize;
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
        // ★ Nor is a function PARAMETER, for the identical reason — see `fn_signature_spans`.
        if params.iter().any(|(a, b)| offset > *a && offset < *b) {
            continue;
        }
        let inside_own_client = offset > mod_start && offset < mod_end;

        // ★★★★★ **CONSTRAINT 26's SCOPED EXCEPTION.** A `*_src` client field names the
        // OTHER client by definition — `NVOS55_PARAMETERS::hClientSrc` is the whole point of
        // the dup — so the destination rule cannot express it and must not be stretched to.
        // ⊘ The universe is still DERIVED (the suffix is read off the ABI's own names), and
        // the approved set is derived from the newtype, so neither half is a list here.
        if name.ends_with("_src") {
            src_sites += 1;
            if !is_handed_accessor(value, &accessors) {
                bad.push(format!(
                    "  rm.rs (code line {}): `{name}: {value}` ⇒ a SOURCE client that is not \
                     a `HandedVaSpace` accessor",
                    idx + 1
                ));
            }
            continue;
        }

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
        src_sites >= SRC_CLIENT_SITES_FLOOR,
        "★ NON-VACUITY: found {src_sites} SOURCE-client initialiser(s) in rm.rs, floor is \
         {SRC_CLIENT_SITES_FLOOR}. Constraint 26's dup is the one sanctioned place a foreign \
         client may be named; if the scanner stops seeing it, the rule that scopes it is \
         gating nothing. Treat this as RED and fix the scanner — do NOT lower the floor. \
         (If the dup verb was deliberately deleted, delete this rule in the SAME change.)"
    );
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

/// ★★★★★ **CONSTRAINT 26 — `HandedVaSpace` is as unforgeable as `OwnClient`.**
///
/// The scoped rule above accepts `<x>.src_client()` as an `hClientSrc`. That is only worth
/// anything while the **type** on the left cannot be manufactured: a `From<u32>` here would
/// turn the exception into *"any u32, spelled through one extra call"*, which is F11 with a
/// method on it.
///
/// ⊘ Mirrors `own_client_is_unforgeable` deliberately, including the forbidden-name list. If
/// one of them grows a case the other should too, and the duplication is what makes that
/// visible.
#[test]
fn handed_vaspace_is_unforgeable() {
    let code = rm_rs_code_only();
    let (start, end) = handed_vaspace_module_span(&code);
    let module = &code[start..end];

    // Private fields on both types.
    assert!(
        module.contains("pub(super) struct HandedVaSpace {")
            && module.contains("        client: u32,")
            && module.contains("        space: u32,"),
        "★★★ CONSTRAINT 26 REGRESSED — `HandedVaSpace`'s handles are no longer private \
         fields. A public field is a `u32 -> HandedVaSpace` conversion with extra steps, and \
         the one sanctioned foreign client becomes any foreign client."
    );
    assert!(
        module.contains("pub(super) struct ScratchpadRole(());"),
        "★★★ CONSTRAINT 26 REGRESSED — `ScratchpadRole` is no longer an unforgeable unit. It \
         is the half that says *which isolate* may name a foreign client at all; a \
         constructible one lets a per-proc backend build a `HandedVaSpace`."
    );

    // Exactly one door into the role, and it is the isolate-id test.
    assert_eq!(
        module.matches("fn of(").count(),
        1,
        "★★★ CONSTRAINT 26 REGRESSED — `ScratchpadRole` has more than one constructor."
    );
    assert!(
        module.contains("crate::SCRATCHPAD_ISOLATE_PROC"),
        "★★★ CONSTRAINT 26 REGRESSED — `ScratchpadRole::of` no longer checks the isolate id \
         against the scratchpad's proc. Without that test the type proves nothing at all."
    );
    assert!(
        !module.contains("static ") && !module.contains("thread_local"),
        "★★★ CONSTRAINT 26 REGRESSED — the role is being read off process-global state. One \
         process hosts TWO emulated GPUs; a `static` binds to whichever realized first, \
         which is the w637 defect in the one place it would be least visible."
    );

    // Exactly one door into the handed-over value, and it consumes the role.
    assert_eq!(
        module.matches("fn handed_over(").count(),
        1,
        "★★★ CONSTRAINT 26 REGRESSED — `HandedVaSpace` has more than one constructor. The \
         invariant is that *having* one and *being the scratchpad, handed these numbers* are \
         one fact; a second constructor splits them apart."
    );
    assert!(
        module.contains("_role: ScratchpadRole,"),
        "★★★ CONSTRAINT 26 REGRESSED — `handed_over` no longer takes the `ScratchpadRole` BY \
         VALUE. Taking it by reference, or not at all, is what lets the role be re-used or \
         skipped."
    );

    for forbidden in [
        "impl From<u32> for HandedVaSpace",
        "fn new(",
        "derive(Default)",
        "impl Default for HandedVaSpace",
        "fn from_raw(",
        "impl From<u32> for ScratchpadRole",
    ] {
        assert!(
            !module.contains(forbidden),
            "★★★ CONSTRAINT 26 REGRESSED — `mod handed_vaspace` now contains `{forbidden}`, \
             which manufactures the type from a value nobody was handed. That is precisely \
             the direction it exists to make impossible."
        );
    }
}

/// ⊘ **The exception is ONE verb wide.** A second `NV_ESC_RM_DUP_OBJECT` would be a second
/// place a foreign client is named, and the whole argument for scoping F11 rather than
/// eliminating it is that the place is countable.
#[test]
fn there_is_exactly_one_dup_object_escape_in_rm_rs() {
    let code = rm_rs_code_only();
    // ⊘ The ESCAPE, not the import: `use` names it once and that is not a call site.
    let n = code
        .matches("ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_DUP_OBJECT")
        .count();
    assert_eq!(
        n, 1,
        "★★★ CONSTRAINT 26 — `rm.rs` now issues {n} `NV_ESC_RM_DUP_OBJECT` escape(s), \
         expected exactly 1 (`raw_dup_object`). Every one of them names `hClientSrc`; a \
         second is a second scope for an exception whose whole defence is that it is \
         countable."
    );
}

/// ★★★★★ **CONSTRAINT 32 — `HandedClient` is as unforgeable as `HandedVaSpace`.**
///
/// ⊘ **And the reason it needs its own test rather than a second assertion in
/// `handed_vaspace_is_unforgeable`:** the two types widen F11 in *different fields*.
/// `HandedVaSpace` supplies `NVOS55_PARAMETERS::hClientSrc` while the escape is still
/// issued under **our** client on **our** descriptor. `HandedClient` supplies the `hRoot`
/// of the escape itself — every object allocated under it lands in the foreign client's
/// namespace. A `From<u32>` here is not "F11 with a method on it"; it is F11 gone.
///
/// ⊘ Mirrors `handed_vaspace_is_unforgeable` deliberately, forbidden-name list included. If
/// one grows a case the other should too, and the duplication is what makes that visible.
#[test]
fn handed_client_is_unforgeable() {
    let code = rm_rs_code_only();
    let (start, end) = handed_client_module_span(&code);
    let module = &code[start..end];

    assert!(
        module.contains("pub(super) struct HandedClient {")
            && module.contains("        client: u32,")
            && module.contains("        minted_by: u32,"),
        "★★★ CONSTRAINT 32 REGRESSED — `HandedClient`'s handles are no longer private \
         fields. A public field is a `u32 -> HandedClient` conversion with extra steps, and \
         the one sanctioned birth client becomes any client at all as an escape's `hRoot`."
    );

    // Exactly one door into the handed-over value, and it consumes the role.
    assert_eq!(
        module.matches("fn handed_over(").count(),
        1,
        "★★★ CONSTRAINT 32 REGRESSED — `HandedClient` has more than one constructor. The \
         invariant is that *having* one and *being the scratchpad, handed these numbers* are \
         one fact; a second constructor splits them apart."
    );
    assert!(
        module.contains("_role: ScratchpadRole,"),
        "★★★ CONSTRAINT 32 REGRESSED — `handed_over` no longer takes the `ScratchpadRole` BY \
         VALUE. Taking it by reference, or not at all, is what lets the role be re-used or \
         skipped — and a per-proc backend that can build a `HandedClient` can issue escapes \
         under a client it did not mint."
    );
    assert!(
        module.contains("use super::handed_vaspace::ScratchpadRole;"),
        "★★★ CONSTRAINT 32 REGRESSED — `mod handed_client` no longer reuses the ONE \
         `ScratchpadRole`. A second unit type meaning *\"I am the scratchpad\"* is a second \
         place for that answer to drift, and only one of them would be checked against \
         `SCRATCHPAD_ISOLATE_PROC`."
    );

    // ★★★ The minting proc must SURVIVE into the value. Constraint 32's security claim is a
    // statement about `ProcessID`; a value that cannot say which isolate minted it cannot be
    // checked against the descriptor that carried it.
    assert!(
        module.contains("fn minted_by(self) -> u32 {"),
        "★★★ CONSTRAINT 32 REGRESSED — `HandedClient` no longer reports the isolate that \
         minted it. RM stamps `ProcessID` from the CREATING task (client.c:112), so \
         `minted_by` is the only thing that makes *\"stamps land as I's\"* checkable rather \
         than argued. Dropping it is the whole ruling becoming a comment."
    );
    assert!(
        module.contains("minted_by=proc {}"),
        "★★★ CONSTRAINT 32 REGRESSED — `HandedClient`'s `Debug` no longer prints the minting \
         proc. A census line that names a client without naming whose identity it carries \
         cannot answer the one question this route exists to answer."
    );
    assert!(
        !module.contains("static ") && !module.contains("thread_local"),
        "★★★ CONSTRAINT 32 REGRESSED — the birth client is being read off process-global \
         state. One process hosts TWO emulated GPUs; a `static` binds to whichever realized \
         first, which is the w637 defect in the one place it would be least visible."
    );

    for forbidden in [
        "impl From<u32> for HandedClient",
        "fn new(",
        "derive(Default)",
        "impl Default for HandedClient",
        "fn from_raw(",
    ] {
        assert!(
            !module.contains(forbidden),
            "★★★ CONSTRAINT 32 REGRESSED — `mod handed_client` now contains `{forbidden}`, \
             which manufactures the type from a value nobody was handed. That is precisely \
             the direction it exists to make impossible."
        );
    }
}
