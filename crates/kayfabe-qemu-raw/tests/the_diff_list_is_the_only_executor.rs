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

/// ★★★★★ **w759 — `apply_ops` RETURNS RM'S ANSWER, AND THE BIND PATH USES IT.**
///
/// ⊘⊘⊘ **This gate exists because its absence cost a live regression.** When
/// `map_store_slice_for_leaf` was routed through `apply_ops`, the returned VA was replaced
/// with `at` — the store offset. Both are `u64`, so **nothing failed to compile and no test
/// failed**; `[measured w758]` the guest went from `P1/P2/P3 ✔ VERIFIED` back to
/// `NEVER RETIRED` on the next boot, and the only evidence was a boot four minutes long.
///
/// ⚠ The distinction is not pedantic: `placed_as_asked` exists because **RM may place a
/// mapping somewhere other than where it was asked**. A caller handed back its own request can
/// never detect that — it is guaranteed to agree with itself.
#[test]
fn the_bind_path_uses_the_address_rm_chose() {
    let storemap = code_of("src/storemap.rs");
    assert!(
        storemap.contains("pub placed: Vec<u64>"),
        "★★★★★ `AppliedOps::placed` is gone. Without it a caller cannot learn where RM put a \
         mapping, and the only address available is the one it asked for — which always \
         agrees with itself and therefore checks nothing."
    );
    assert!(
        storemap.contains("done.placed.push(self.map("),
        "★★★ `apply_ops` no longer records the VA `map` returned. A `placed` vector that is \
         never filled is worse than none: the caller's `.first()` then refuses on every map."
    );

    let shim = code_of("src/shim.rs");
    // ★★★ The bind path must consume `placed`, and must NOT re-substitute the request.
    assert!(
        shim.contains("done.placed.first()"),
        "★★★★★ the bind path no longer takes RM's answer out of `AppliedOps`. This is the \
         exact regression of w758, and it is invisible to the compiler because the request \
         and the answer are both `u64`."
    );
    assert!(
        !shim.contains(".map(|_| at)"),
        "★★★★★ the bind path discards `apply_ops`' result and substitutes `at` — the store \
         OFFSET — as the host VA. `[measured w758]` this took the guest from \
         `P1/P2/P3 ✔ VERIFIED` to `NEVER RETIRED`, with nothing red in the workspace suite."
    );
}

/// ★★★★★ **§39(c) — `apply_ops` REFUSES THE WHOLE BATCH BEFORE IT MOVES ANYTHING.**
///
/// ⊘ `map()` already refuses an out-of-range slice one at a time, but that failure arrives
/// MID-APPLY: some of the batch is unmapped, some mapped, and the caller is handed a
/// half-applied diff to reason about. A report naming memory outside the store is not one to
/// partially honour — it is one we have caught asking for memory that is not the guest's.
///
/// This gate pins the ORDER, which is the whole property: the containment loop must sit above
/// the first `self.unmap(`, or the refusal happens after mappings have already been retired.
#[test]
fn apply_ops_bounds_the_whole_batch_before_it_mutates() {
    let src = std::fs::read_to_string("src/storemap.rs").expect("storemap.rs is readable");
    let body = {
        let at = src
            .find("pub fn apply_ops(")
            .expect("`apply_ops` exists — the gates above pin that");
        &src[at..]
    };
    let guard = body
        .find("self.obj_len")
        .expect(
            "★★★★★ `apply_ops` no longer bounds its ops against the store's length. A diff \
             naming memory outside the guest's own store would then be applied, handing the \
             guest memory that is not its own — the escalation §39 exists to refuse.",
        );
    let first_unmap = body.find("self.unmap(").expect("`apply_ops` no longer unmaps");
    let first_map = body.find("self.map(").expect("`apply_ops` no longer maps");
    assert!(
        guard < first_unmap && guard < first_map,
        "★★★★★ the containment check runs AFTER `apply_ops` has begun mutating (guard at {guard}, \
         first unmap at {first_unmap}, first map at {first_map}). A batch carrying one \
         out-of-store run would then be half-applied before anything objected."
    );
}
