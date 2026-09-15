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
    let dup = body
        .find("let duped = self.conn.raw_dup_object(")
        .expect("the scratchpad dups the handed-over space");
    let remember = body
        .find("self.conn.remember_adopted_space(duped);")
        .expect("asserted above");
    let range = body
        .find("match self.conn.raw_alloc_range_over(duped)")
        .expect("the range is built over the dup");
    assert!(
        dup < remember && remember < range,
        "★★★ the adopted space is recorded outside the window between the dup and the range. \
         An adopted space that is not yet recorded is one a concurrent birth would be allowed \
         into."
    );
}
