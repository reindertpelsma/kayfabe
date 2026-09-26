//! ★★★ What makes this crate's ABI-quarantine exemption NARROW instead of a hole.
//!
//! The Axis-A gate lets exactly three crates declare a C layout, and its own failure text
//! says a third entry is a design decision. The argument for this one is that its layouts
//! are **not foreign** — their other half is `qemu/hw/misc/nvkvm/kayfabe_shim.h`, a file in
//! this repository. That argument is only worth anything if it stays true, and a comment
//! cannot keep it true.
//!
//! So: **every C-layout type in this crate must have a declared counterpart in that header,
//! and every structure in that header must have one here.** Both directions, because the two
//! failures are different and only one of them is loud on its own:
//!
//! - a Rust type with no counterpart is a layout we invented and nothing mirrors — the exact
//!   thing the gate exists to stop, arriving through the exemption;
//! - a header structure with no Rust counterpart is the hand-mirroring hazard the crate docs
//!   admit to. It would compile on both sides and diverge silently until the `sizeof`
//!   handshake caught it at somebody's runtime, which is later and further away than here.
//!
//! ★★ **The counterpart is found BY NAME, and there is no map.**
//!
//! An earlier draft carried a `(Rust type, C structure)` table so the two sides could be
//! spelled differently. That was withdrawn: a hand-maintained map is the
//! [[gates_quantified_over_a_list]] shape one level down — shortening it weakens the proof
//! with zero red tests, and it is a thing to get wrong on the way to being right. Requiring
//! **identical names** costs one rename and buys a proof that a *shell gate* can re-derive
//! without parsing Rust, which is what let the quarantine become a predicate instead of an
//! exemption list.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("the crate lives at crates/<name>")
        .to_path_buf()
}

fn header() -> String {
    let p = repo_root().join("qemu/hw/misc/nvkvm/kayfabe_shim.h");
    std::fs::read_to_string(&p).unwrap_or_else(|e| {
        panic!(
            "★ the other half of this crate's ABI is {} and it is not readable: {e}. The \
             Axis-A exemption for this crate rests on that file existing in this repository; \
             if it has moved, the exemption's argument has moved with it",
            p.display()
        )
    })
}

/// Every `#[repr(C)]` type declared in this crate's sources, by name.
fn c_layout_types() -> Vec<String> {
    let src = repo_root().join("crates/kayfabe-qemu-raw/src");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&src).expect("the crate has a src/") {
        let path = entry.expect("readable").path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("readable");
        let lines: Vec<&str> = text.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            // ★ Assembled from fragments, never written out — the Axis-A gate greps every
            // `.rs` file for this attribute, so a scanner that spelled it in order to look
            // for it would be reported as its own offender. `gate_scope.rs` does the same
            // for the `unsafe_code` keyword, and for the same reason. (Spelling it that way
            // is not pedantry either: the surface gate greps for the bare word, so this very
            // line would trip THAT gate — which is the same lesson twice on one file.)
            let attr = concat!("#[repr", "(C");
            if !l.trim_start().starts_with(attr) {
                continue;
            }
            // The declaration is the next line that names a struct, skipping the derives and
            // documentation that conventionally sit between the attribute and the item.
            let name = lines[i + 1..]
                .iter()
                .find_map(|c| c.split("struct ").nth(1))
                .and_then(|rest| {
                    rest.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                        .next()
                })
                .unwrap_or_else(|| {
                    panic!(
                        "a C-layout attribute in {} at line {} that is not on a struct",
                        path.display(),
                        i + 1
                    )
                });
            out.push(name.to_string());
        }
    }
    out.sort();
    out
}

/// Every structure the header declares, by name.
fn header_structs() -> Vec<String> {
    header()
        .lines()
        .filter_map(|l| l.trim().strip_prefix("typedef struct "))
        .filter_map(|rest| rest.split_whitespace().next())
        .map(|n| n.trim_end_matches('{').to_string())
        .collect()
}

#[test]
fn every_c_layout_type_in_this_crate_is_declared_in_our_own_header() {
    let h = header();
    let found = c_layout_types();
    assert!(
        !found.is_empty(),
        "\u{2605} SUSPECT THE INSTRUMENT: this crate is the hypervisor FFI surface and must \
         have C-layout types. Finding none means the scan broke, not that the crate is clean"
    );
    for rust in &found {
        assert!(
            h.contains(&format!("}} {rust};")),
            "\u{2605} `{rust}` is a C layout our own header does not declare, so it is \
             FOREIGN by default and stays quarantined. The Axis-A rule admits a C layout \
             outside the quarantine crates only when it is provably OWN-WIRE: a structure of \
             the SAME NAME in a repository-local header, mirrored in both directions. Add the \
             header structure, or do not use a C layout"
        );
    }
}

#[test]
fn every_structure_in_our_own_header_is_mirrored_here() {
    let declared = header_structs();
    let found = c_layout_types();
    assert!(
        declared.len() >= 5,
        "\u{2605} SUSPECT THE INSTRUMENT: the header declares only {} structures. The seam \
         has five, so finding fewer means the scan broke rather than the header shrank",
        declared.len()
    );
    for name in &declared {
        assert!(
            found.contains(name),
            "\u{2605} the header declares `{name}` and no Rust type mirrors it. A structure \
             that exists on one side only compiles on both and diverges silently until the \
             `sizeof` handshake catches it at somebody\'s runtime \u{2014} which is later, \
             and further away, than here"
        );
    }
}
