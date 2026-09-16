//! ★★★★★ **w746 — THE TWO OWNERSHIP-SPLIT GATES THAT LIVE ON THE HOST BACKEND**, and why
//! they are asserted by reading the source that runs.
//!
//! `HostRmBackend` opens `/dev/nvidia*` on every path, so there is no mock that reaches
//! either gate. The honest instrument is therefore the one `guest_ring_census.rs` already
//! uses in this crate: read the body, assert the shape. ⊘ A source-shape test cannot prove a
//! gate *fires*; it proves the gate is *expressible on the path* — which is precisely the
//! failure both of these were written for, and precisely the one w745 hit.
//!
//! # ⊘⊘⊘ WHY GATE ONE EXISTS — a counter that was zero because nothing could reach it
//!
//! `MAP_THROUGH_A_BARE_SPACE` has lived in four `RmBackend` verbs since constraint 26b.
//! `alloc_channel_in` calls **none** of them: it reaches RM through `raw_map_dma` directly.
//! So when w745 pre-registered *"`BARE-SPACE-REFUSED` ≥ 1 confirms the emulated path needed a
//! map into a bare space"*, that row **could not fire whatever happened** — and its measured
//! `0` was read as *"better than predicted: the emulated path did not need a map"*. The map
//! happened; RM refused it `0x51`; ten engine objects died of it, and the whole ownership
//! split refused 4619 times downstream. ⇒ the gate now sits where `Nvos46Parameters` is
//! built, which is the one place in the crate every map is expressible.

use std::path::PathBuf;

/// Strip `//` line comments and `/* */` blocks, so a doc comment that *mentions* a gate is
/// not counted as the gate. ⊘ A local copy of `guest_ring_census.rs`'s, deliberately: a
/// shared helper between two integration-test binaries would be a third crate, and the
/// function is eleven lines with no state.
fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let b = src.as_bytes();
    let (mut i, mut depth) = (0usize, 0usize);
    while i < b.len() {
        if depth == 0 && b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if b[i..].starts_with(b"/*") {
            depth += 1;
            i += 2;
        } else if depth > 0 && b[i..].starts_with(b"*/") {
            depth -= 1;
            i += 2;
        } else {
            if depth == 0 {
                out.push(b[i] as char);
            }
            i += 1;
        }
    }
    out
}

fn rm_body() -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/rm.rs");
    strip_comments(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p:?}: {e}")))
}

/// The one place this crate builds `NVOS46_PARAMETERS`; `raw_map_dma` and
/// `raw_map_dma_flags` both delegate to it.
const THE_ONE_MAP_SITE: &str = "fn raw_map_dma_slice(";

/// ★★★★★ **CONSTRAINT 26/32 — THE SECOND MAP SITE IS UNREACHABLE WITH A BARE SPACE, AND
/// THE ORDER IS WHAT MAKES THAT TRUE.**
///
/// ⊘⊘ **`every_map_in_this_crate_can_refuse_a_bare_space` no longer covers every map**, and
/// saying so is the point. Route K added `BirthConn::map_dma_slice`, a second `NVOS46` site
/// that **cannot** call `is_bare_space`: the ledger lives on `RmConnection` and `BirthConn`
/// deliberately has none. The w746 lesson — *"restated HERE, where `Nvos46Parameters` is
/// built, so the counter cannot be zero by construction again"* — cannot be applied literally.
///
/// ★★★ So the property is bought a different way, and it is **stronger** rather than equal:
/// `BirthConn::map_dma_slice` is reached only through `map_store_slice`, which (a) refuses a
/// bare space **before** the birth dispatch, and (b) reaches the birth branch only when
/// `birth_for_range(h_dma)` answers `Some` — which is proof that `h_dma` is a range **this
/// connection allocated in B**, not merely that it is not bare.
///
/// ⇒ this gate checks the ORDER, because the order is the whole argument. A birth dispatch
/// hoisted above the refusal would let a bare space reach RM through B, where nothing else
/// is watching.
/// ★★★★★ **CONSTRAINT 32 — THE TWO HANDLE SPACES CANNOT COLLIDE, AND IT IS ARITHMETIC.**
///
/// `RmConnection` mints from `FIRST_HANDLE` **upward**; `BirthConn` mints from
/// `BIRTH_HANDLE_BASE` upward, in a **different client**. Two ledgers key on the handle
/// **value** while holding handles from both namespaces — `adopted_spaces` and
/// `birth_ranges` — so a value that occurs in both is not a wrong handle, it is a **silent
/// misroute**: one of our ranges mapped into B, under a foreign `hRoot`, on a descriptor we
/// did not open, and RM answers it because in B's namespace that handle is real.
///
/// ⊘⊘ **The first version of `BIRTH_HANDLE_BASE` was `0xCAFE_B000` — 45 055 allocations
/// above our own base.** That is not a safety margin; it is a collision with a schedule.
/// The base must be **below** ours, so reaching it requires wrapping `u32`.
#[test]
fn the_two_handle_spaces_cannot_collide() {
    let body = rm_body();
    let first = read_hex(&body, "const FIRST_HANDLE: u32 = ");
    let birth = read_hex(&body, "const BIRTH_HANDLE_BASE: u32 = ");
    assert!(
        birth < first,
        "★★★★★ CONSTRAINT 32 — `BIRTH_HANDLE_BASE` ({birth:#010x}) is at or above \
         `FIRST_HANDLE` ({first:#010x}). Our space INCREMENTS, so it will reach B's in \
         {} allocations and then every ledger keyed on a handle value is ambiguous.",
        birth.wrapping_sub(first)
    );
    // ⊘ And not merely below: far enough below that wrapping is the only route.
    let gap = first.wrapping_sub(birth);
    assert!(
        gap > 100_000_000,
        "★★★ CONSTRAINT 32 — the two handle spaces are only {gap} apart. The margin must be \
         a number nobody reaches, not a number nobody has reached YET."
    );
}

/// Read a `u32` hex literal off a `const NAME: u32 = 0x…;` line in the source.
fn read_hex(body: &str, decl: &str) -> u32 {
    let at = body
        .find(decl)
        .unwrap_or_else(|| panic!("★ NON-VACUITY: `{decl}` is gone from rm.rs"));
    let rest = &body[at + decl.len()..];
    let lit: String = rest
        .chars()
        .take_while(|c| c.is_ascii_hexdigit() || *c == 'x' || *c == 'X' || *c == '_')
        .filter(|c| *c != '_')
        .collect();
    let hex = lit.trim_start_matches("0x").trim_start_matches("0X");
    u32::from_str_radix(hex, 16).unwrap_or_else(|e| panic!("parse `{lit}`: {e}"))
}

/// ★★★★★ **CONSTRAINT 27/32 — THE UNMAP FOLLOWS THE MAP'S NAMESPACE.**
///
/// ⊘⊘ **This is constraint 27's failure, not a tidiness one.** If a route-K slice's unmap is
/// issued under our own client naming a handle that lives in B, it fails — and a silently
/// failed unmap is exactly what §27 forbids: the refresh reports the guest's TLB invalidate
/// complete, the guest kernel reuses that physical page for another of its own userspace
/// processes, and the previous process can still reach it through a slice we told the guest
/// was gone. A cross-process leak **inside** the guest, caused by us, invisible to the guest.
///
/// ⇒ `unmap_store_slice` must consult the same reverse index `map_store_slice` does, so the
/// two can never disagree about which namespace a handle lives in.
#[test]
fn the_store_unmap_routes_through_the_same_index_as_the_map() {
    let body = rm_body();
    for f in ["fn map_store_slice(", "fn unmap_store_slice("] {
        let at = body
            .find(f)
            .unwrap_or_else(|| panic!("★ NON-VACUITY: `{f}` is gone from rm.rs"));
        let end = body[at..].find("\n    fn ").map_or(body.len(), |o| at + o);
        assert!(
            body[at..end].contains("birth_for_range(h_dma)"),
            "★★★★★ CONSTRAINT 27/32 — `{f}` does not consult `birth_for_range`. The map and \
             the unmap MUST agree about which client a handle lives in; one that routes and \
             one that does not means every route-K slice is mapped in B and unmapped — or \
             not — in our own client, and §27's barrier completes on an unmap that never \
             landed."
        );
    }
}

#[test]
fn the_birth_dispatch_is_below_the_bare_space_refusal() {
    let body = rm_body();
    let at = body
        .find("fn map_store_slice(")
        .expect("★ NON-VACUITY: `map_store_slice` is gone — this gate gates nothing");
    let end = body[at..]
        .find("\n    fn ")
        .map_or(body.len(), |o| at + o);
    let f = &body[at..end];
    let refusal = f
        .find("if self.conn.is_bare_space(h_dma)")
        .expect(
            "★★★ CONSTRAINT 26 — `map_store_slice` no longer refuses a bare space. It is the \
             only refusal covering the SECOND NVOS46 site, which cannot make one itself.",
        );
    let dispatch = f.find("birth_for_range(h_dma)").expect(
        "★ NON-VACUITY: `map_store_slice` no longer dispatches to a birth client, so route \
         K's map is unreachable and this gate is checking an order that does not exist. If \
         route K was removed, delete this gate in the SAME change.",
    );
    assert!(
        refusal < dispatch,
        "★★★★★ CONSTRAINT 26/32 — the birth dispatch is ABOVE the bare-space refusal \
         (refusal@{refusal}, dispatch@{dispatch}). A bare space would reach RM through \
         client B, where `is_bare_space` cannot be consulted at all because `BirthConn` has \
         no ledger by design. This ordering is the ONLY thing standing between the two."
    );
}

#[test]
fn every_map_in_this_crate_can_refuse_a_bare_space() {
    let body = rm_body();
    let at = body
        .find(THE_ONE_MAP_SITE)
        .expect("this crate builds NVOS46 in exactly one function");
    let end = body[at..]
        .find("\n    fn ")
        .map(|o| at + o)
        .unwrap_or(body.len());
    let f = &body[at..end];
    assert!(
        f.contains("self.is_bare_space(h_dma)"),
        "★★★★★ CONSTRAINT 26/29 — `raw_map_dma_slice` no longer refuses a bare address \
         space. Every map in this crate funnels through it, INCLUDING `alloc_channel_in`'s \
         ring map, which calls none of the four `RmBackend` verbs that carry the same \
         refusal. Without it here, `MAP_THROUGH_A_BARE_SPACE` is a counter nothing can \
         increment, RM answers the map `0x51` instead, and a boot reads that as memory \
         exhaustion — which is exactly what w745 measured and mis-read."
    );
    assert!(
        f.contains("RmError::Other(MAP_THROUGH_A_BARE_SPACE)"),
        "★★★ …and it must refuse BY NAME. RM's own status for this is indistinguishable \
         from exhaustion; the whole value of the gate is that the log says which."
    );
    // ★★★ NON-VACUITY: the refusal must precede the struct it guards, or it is a check of
    // a request that has already been built.
    let gate = f.find("is_bare_space(h_dma)").expect("asserted above");
    let build = f
        .find("Nvos46Parameters {")
        .expect("the struct is built here");
    assert!(
        gate < build,
        "★★★★★ the bare-space refusal is BELOW the `Nvos46Parameters` it guards"
    );
    // ⊘ And the four verb-level refusals stay. They are not redundant: each names WHICH verb
    // asked, and each refuses before allocating anything. This one is the backstop that
    // makes their question total.
    assert_eq!(
        body.matches("RmError::Other(MAP_THROUGH_A_BARE_SPACE)").count(),
        5,
        "★★ the bare-space refusals moved. FIVE is the ruling: `map_gpu_va`, `unmap_gpu_va`, \
         `map_store_slice`, `unmap_store_slice` — each naming its own verb — plus the \
         backstop in `raw_map_dma_slice` added at w746. ⊘ FOUR means the backstop is gone \
         and the counter is vacuous again."
    );
}

#[test]
fn the_scratchpad_cannot_birth_a_channel_in_a_space_it_adopted() {
    // ★★★★★ **CONSTRAINT 30.** Owner, 2026-09-15: *"the reason we also do isolates is to
    // ensure the channel is created in an unprivileged process. If ogkm links the process
    // that created the channel to the privileges of it… the cross guest process isolation is
    // broken. So this has to be asserted."*
    //
    // `[ogkm-580.159.04, src/kernel/gpu/fifo/kernel_channel.c:277-295]` — privilege comes
    // from `pCallContext->secInfo.privLevel` AT CREATION, `rmclientIsAdmin(...)` alone sets
    // `_PRIVILEGED_CHANNEL_TRUE`, and `ProcessID`/`SubProcessID` are copied from the creating
    // client. `NV_ESC_RM_DUP_OBJECT` re-runs none of it.
    let body = rm_body();
    assert!(
        body.contains("RmError::Other(SCRATCHPAD_BIRTH_IN_A_HANDED_SPACE)"),
        "★★★★★ CONSTRAINT 30 — the gate is gone. With it removed, \"the scratchpad births \
         the channel and dups it to the isolate\" is buildable by accident, and every such \
         channel carries the scratchpad's privilege and process identity for its whole life."
    );
    assert!(
        body.contains("ScratchpadRole::of(self.id).is_some() && self.conn.is_adopted_space(space)"),
        "★★★★★ CONSTRAINT 30 — the gate's predicate changed. BOTH halves are load-bearing: \
         `ScratchpadRole::of` because a per-proc isolate birthing in its OWN space is the \
         product path, and `is_adopted_space` because the scratchpad birthing in a space IT \
         created (its `ce_copy` engine, the CUDA walk kernel) is legitimate and must not be \
         refused. A gate on either half alone is the wrong rule."
    );
    // ★★★ The set it reads must be POPULATED, or the predicate is false forever — the purest
    // form of the defect this whole file exists for.
    assert!(
        body.contains("self.conn.remember_adopted_space(duped);"),
        "★★★★★ CONSTRAINT 30 — `adopted_spaces` is never written, so `is_adopted_space` \
         answers `false` for every space and the gate above can never fire. A gate that \
         cannot fire reads exactly like a gate that never had to."
    );
    // ⊘ NON-VACUITY, the other direction: the recording must happen at the DUP, before the
    // range is built, or a failure in between leaves an adopted space nothing knows about.
    //
    // ## ⊘⊘⊘ RESTATED w753 — THE WINDOW IS PER DUP, AND THERE ARE NOW TWO DUPS
    //
    // Constraint 32 added a route-K branch to this function: when the proc handed us a birth
    // client, the dup goes into **B** instead of into our own client. So `adopt_vaspace` now
    // contains two `dup -> remember -> range` sequences.
    //
    // ★★★ **The question is unchanged and is now asked of EVERY sequence.** The previous
    // form used `find` — the FIRST occurrence of each marker — which is not *"the window
    // holds"* but *"the window holds for whichever one happens to be first"*. With one
    // branch those were the same sentence; with two they are not, and the weaker one passes
    // while a second branch records nothing at all.
    //
    // ⇒ every dup is paired with the NEXT remember and the NEXT range after it, and each
    // triple must be in order. A branch that forgot to record goes red because its dup pairs
    // with a `remember` that belongs to the branch after it — and the range test then fails.
    // ⊘ Scoped to `adopt_vaspace`'s OWN body. `rm_body()` is the whole file, and the markers
    // below also appear in the functions that DEFINE them — a gate that scanned the file
    // would count four dups and two records and report a leak that is not there.
    let fn_at = body
        .find("fn adopt_vaspace(&mut self, client: u32, space: u32)")
        .expect("★ NON-VACUITY: `adopt_vaspace` is gone — this gate gates nothing");
    let fn_end = body[fn_at..]
        .find("\n    fn ")
        .map_or(body.len(), |o| fn_at + o);
    let body = &body[fn_at..fn_end];
    // ⊘ `let duped = ` and not the callee's name: the two branches call DIFFERENT dup verbs
    // (`raw_dup_object` in our client, `birth.dup` in B), and a gate keyed on either name
    // would be blind to the other branch — which is exactly the shape that let the first
    // version of this restatement pass while one branch recorded nothing.
    let dups: Vec<usize> = body
        .match_indices("let duped = ")
        .map(|(at, _)| at)
        .collect();
    assert!(
        dups.len() >= 2,
        "★ NON-VACUITY: `adopt_vaspace` contains {} dup(s), expected at least 2 (the \
         cross-client one and constraint 32's route-K one). If route K's branch is gone, \
         delete this half of the gate in the SAME change rather than letting it quantify \
         over one thing and read as if it covered both.",
        dups.len()
    );
    let remembers: Vec<usize> = body
        .match_indices("remember_adopted_space(duped)")
        .map(|(at, _)| at)
        .collect();
    assert_eq!(
        remembers.len(),
        dups.len(),
        "★★★★★ CONSTRAINT 30 — `adopt_vaspace` has {} dup(s) but records {} adopted \
         space(s). Every dup produces a space the scratchpad now holds; one that is not \
         recorded is one a concurrent birth WOULD BE ALLOWED INTO, and the gate above \
         cannot see it because `is_adopted_space` answers `false`.",
        dups.len(),
        remembers.len()
    );
    let ranges: Vec<usize> = body
        .match_indices("alloc_range_over(duped)")
        .map(|(at, _)| at)
        .collect();
    assert_eq!(
        ranges.len(),
        dups.len(),
        "★ NON-VACUITY: {} dup(s) but {} range(s). The window this gate checks is \
         `dup -> remember -> range`; a dup with no range has no window and would pass \
         vacuously.",
        dups.len(),
        ranges.len()
    );
    for (i, &dup) in dups.iter().enumerate() {
        let remember = remembers[i];
        let range = ranges[i];
        assert!(
            dup < remember && remember < range,
            "★★★ dup #{i} in `adopt_vaspace` records its adopted space OUTSIDE the window \
             between the dup and the range (dup@{dup}, remember@{remember}, range@{range}). \
             An adopted space that is not yet recorded is one a concurrent birth would be \
             allowed into — and with two branches, a gate that only checked the first would \
             pass while the second recorded nothing."
        );
    }
}
