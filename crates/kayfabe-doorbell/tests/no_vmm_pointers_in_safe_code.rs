//! ★★★ NO VMM ADDRESS IN A STRUCT SAFE CODE CAN REACH — enforced over the source.
//!
//! `[owner, 2026-09-21]` *"no vmm pointer in safe code, unsafe must protect against memory bugs in
//! safe, for all low level calls, so also no vmm pointers in nvidia structs that exist in safe
//! code."*
//!
//! ⊘ An NVIDIA parameter block is a `#[repr(C)]` struct safe Rust can hold, index and mutate. If a
//! field of one is a host address that `unsafe` code later dereferences, an **ordinary bug in safe
//! code** — a wrong index, an off-by-one, a stale clone — **becomes memory unsafety.** That
//! inverts the boundary: `unsafe` must be sound no matter what safe code does.
//!
//! ⚠ This is a *source* gate. It cannot prove absence — a `u64` field could hold an address under
//! any name. It catches the shapes that have actually appeared, and it is one layer, not the
//! argument.

const FILES: [(&str, &str); 6] = [
    ("hostverb.rs", include_str!("../src/hostverb.rs")),
    ("leaf.rs", include_str!("../src/leaf.rs")),
    ("plane.rs", include_str!("../src/plane.rs")),
    ("trap.rs", include_str!("../src/trap.rs")),
    ("caps.rs", include_str!("../src/caps.rs")),
    ("channel.rs", include_str!("../src/channel.rs")),
];

/// ⊘⊘ **PRECISION MATTERS MORE THAN REACH HERE.** The first version of this gate matched
/// `": usize,"` and fired on `n_tokens: usize` and on the validation function's own parameters —
/// counts, not addresses. ⚠ A gate that cries wolf is a gate someone switches off, and this tree
/// has already written that down once tonight. ⇒ Match what is an address **by type** or **by
/// name**, and look only at STRUCT FIELDS.
const POINTER_TYPED: [&str; 3] = ["*const ", "*mut ", "NonNull<"];
const ADDRESS_NAMED: [&str; 6] = ["addr:", "ptr:", "pointer:", "host_va:", "vmm_addr:", "hva:"];

/// Is this line a struct field declaration (`name: Type,`) rather than a fn signature?
fn is_field(t: &str) -> bool {
    t.ends_with(',')
        && t.contains(':')
        && !t.contains('(')
        && !t.starts_with("fn ")
        && !t.starts_with("pub fn ")
        && !t.starts_with("pub unsafe fn ")
}

#[test]
fn no_safe_struct_field_is_pointer_shaped() {
    let mut bad = Vec::new();
    for (name, src) in FILES {
        for (i, line) in src.lines().enumerate() {
            let t = line.trim();
            if t.starts_with("//") || !is_field(t) {
                continue;
            }
            let typed = POINTER_TYPED.iter().any(|p| t.contains(p));
            let named = ADDRESS_NAMED.iter().any(|p| t.starts_with(p));
            if !(typed || named) {
                continue;
            }
            // ⊘ ONE declared exception: `VmmAddr`'s own private fields. It has no safe
            // constructor (creation is `unsafe fn new`) and no safe accessor returning the
            // integer, so safe code can move it but cannot fabricate or read it.
            if name == "hostverb.rs" && (t.starts_with("addr:") || t.starts_with("len:")) {
                continue;
            }
            bad.push(format!("{name}:{}: {t}", i + 1));
        }
    }
    assert!(
        bad.is_empty(),
        "⊘ POINTER-SHAPED FIELD REACHABLE FROM SAFE CODE — a safe-code bug here becomes memory \
         unsafety:\n  {}",
        bad.join("\n  ")
    );
}

#[test]
fn a_host_verb_signature_cannot_carry_a_guest_flag_word() {
    // ★ §9: "our host-verb signatures do not accept a guest flag word, so a guest-chosen bit
    // cannot reach RM's interpretation of it." ⊘ Enforced by the SIGNATURE, not by a check in the
    // body -- a verb taking `flags: u32` forwards by construction, and validation in the body
    // would not change that, because the next caller just passes the guest's word.
    let src = include_str!("../src/hostverb.rs");
    let verb = src
        .split("pub enum HostVerb")
        .nth(1)
        .and_then(|s| s.split("\n}").next())
        .expect("HostVerb enum");
    for banned in ["flags", "flag:", "guest_word", "raw:"] {
        assert!(
            !verb.contains(banned),
            "⊘ HostVerb carries `{banned}` — that is forwarding a guest value by construction"
        );
    }
}
