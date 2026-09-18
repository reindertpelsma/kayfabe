//! ★★★★★ **w757 — THE DIFF LIST IS THE ONLY THING THAT CHANGES WHAT IS MAPPED.**
//!
//! > Owner, 2026-09-18: *"So ensure its executed, and ensure the diff list is the only thing
//! > executing it."*
//!
//! Privacy carries most of this — `StoreMapPort::map` and `::unmap` are private, so no other
//! module can call them. ⊘ But privacy stops at the module boundary and `storemap.rs` is a
//! thousand lines, so a second in-module caller could appear without any type error. That is
//! what this gate is for.
//!
//! ⚠ **Why it matters, measured:** `[w755u]` `maps=76 unmaps=0 replaced=0`. Mappings only ever
//! accumulated because the bind path mapped directly and nothing removed anything — and two
//! VAs came to name one store offset, which is why the guest's worker threads read each
//! other's patterns. One author for *"what is mapped"* is the fix, and a chokepoint that
//! anything may bypass is not one.

/// Source with `//` comments stripped, so a mention in prose cannot satisfy or trip a gate.
///
/// ⊘ This tree has been bitten both ways: a gate that passed on a sentence in a comment, and a
/// census that counted a doc reference as a call site.
fn code_of(path: &str) -> String {
    let raw = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    raw.lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn only_apply_ops_may_map_or_unmap_a_store_slice() {
    let src = code_of("src/storemap.rs");

    // ★ The mutators must be PRIVATE. `pub fn map` would let any module in this crate bypass
    //   the chokepoint without tripping anything below.
    assert!(
        !src.contains("pub fn map(") && !src.contains("pub fn unmap("),
        "★★★★★ `StoreMapPort::map`/`unmap` is `pub` again. The diff list stops being the only \
         executor the moment another module can call the mutator directly — which is how the \
         tree reached `maps=76 unmaps=0 replaced=0`."
    );

    // ★★★ And exactly one caller each, inside `apply_ops`.
    let maps = src.matches("self.map(").count();
    let unmaps = src.matches("self.unmap(").count();
    assert_eq!(
        (maps, unmaps),
        (1, 2),
        "★★★★★ THE MUTATOR CALL COUNT MOVED. The ruling is ONE `self.map(` (in `apply_ops`) \
         and TWO `self.unmap(` — one in `apply_ops`, one in `map`'s own re-point branch, \
         which is an unmap-then-map at a single VA and cannot be hoisted out without \
         re-opening the window §27 closes. ⊘ A THIRD caller means something changes what is \
         mapped without going through the diff list, and the counters will not say so: they \
         count slices, not authors."
    );

    // ⊘ And `apply_ops` must still be the public door, or the two above are vacuous.
    assert!(
        src.contains("pub fn apply_ops("),
        "★★★ `apply_ops` is gone or no longer public — there is then no way to execute a diff \
         list at all, and the assertions above would pass on a port that can do nothing."
    );
}

/// ★★★ **UNMAPS BEFORE MAPS, and it is §27's rule rather than a preference.**
///
/// A `Remap` is an unmap plus a map at one VA. Running the map first leaves two slices live at
/// one address — the two-memories-at-one-address state the single store exists to abolish.
/// ⊘ Ordering makes the window **not exist**; it does not make it small.
#[test]
fn apply_ops_unmaps_before_it_maps() {
    let src = code_of("src/storemap.rs");
    let body = {
        let start = src
            .find("pub fn apply_ops(")
            .expect("`apply_ops` exists — the gate above pins that");
        let rest = &src[start..];
        let end = rest.find("\n    pub fn ").unwrap_or(rest.len());
        &rest[..end]
    };
    let unmap_at = body.find("self.unmap(").expect(
        "`apply_ops` no longer unmaps at all. A diff list that cannot remove a mapping can \
         only accumulate, which is the defect this whole chokepoint exists to fix.",
    );
    let map_at = body
        .find("self.map(")
        .expect("`apply_ops` no longer maps at all");
    assert!(
        unmap_at < map_at,
        "★★★★★ `apply_ops` maps before it unmaps. A `Remap` is an unmap plus a map at ONE VA, \
         so this order leaves two slices live at one address — exactly the state the single \
         store abolishes. §27 requires unmaps ordered first within one application."
    );
}
