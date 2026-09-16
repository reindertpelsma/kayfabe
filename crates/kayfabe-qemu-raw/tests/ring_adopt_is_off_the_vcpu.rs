//! ★★★★★ **w754 — THE RING ADOPT AND ITS PAGE-TABLE SETTLEMENT MAY NOT RUN ON A vCPU.**
//!
//! # The measurement this file is paid for
//!
//! `[measured w752, vast 51210329, GA106; traces/w752_inplace_repoint/]`
//!
//! ```text
//! worst_trap=24999us at=bar0+0x110c00   cpu_of_that_trap=23979us   (96 % CPU)
//! LOCKCOST rank0 worst_wait=8790us worst_hold=748us
//! SLOW-SITES bar0+0xbb0090=5(worst 8802us) bar0+0xb81608=3(worst 6903us)
//!            bar0+0x110c00=2(worst 24999us) …
//! ```
//!
//! ⊘ The register is a **bystander**. `NV_PGSP_QUEUE_HEAD`'s servicing has been deferred since
//! w432 and its arming has survived a guest reset since w472b; what still ran inside the guest's
//! store was `Regs::adopt_pending_channel_rings`, whose first act is three guest page-table
//! settlement passes (`witness_executor_fb_pages` → `decode_cpu_pt_writes` →
//! `sweep_cpu_pt_tables`). It ran on **whichever** register write noticed
//! `pending_latch_epoch()` move, and `0x110c00` is simply the one the guest writes most during
//! driver init.
//!
//! # ⚠ Why this is a SOURCE-ORDER test and not a runtime one
//!
//! The property is *"on the arm that has a worker, this body runs on the worker, BEFORE the
//! birth drain that consumes it"*. Both halves are facts about **which line calls what, in
//! which order**, on a thread a unit test cannot create without a GPU, an isolate and a guest
//! that has latched a passthrough birth. A runtime test able to reach it would need all of
//! that — and the version of this bug that shipped was invisible to every test that existed,
//! precisely because nothing asserted the call graph.
//!
//! ⊘ The house already uses this form where a type cannot carry the obligation:
//! `pubqueue::the_revocation_type_has_no_route_into_the_queue` reads its own source. This is
//! the same instrument aimed at an ordering rather than an absence.
//!
//! ★ The live half is the boot's `RING-ADOPT ran= off_vcpu= on_vcpu=` census, whose
//! known-positive is an arm of the same binary run with `KAYFABE_MATERIALIZE_INLINE=1` — which
//! must put every pass back **on** the vCPU and the 25 ms back at `bar0+0x110c00`.

/// The shim's own source, read at compile time.
const SHIM: &str = include_str!("../src/shim.rs");

/// ★★★ **The worker calls the adopt, and calls it BEFORE the birth drain.**
///
/// ⊘ Fail-closed on three separate ways to break it: the call disappearing, the call moving
/// after the drain, and the call being made with `on_vcpu = true` from the worker (which would
/// make the census report a violation that is not happening, and hide one that is).
#[test]
fn the_worker_adopts_rings_before_it_drains_births() {
    let loop_start = SHIM
        .find("fn doorbell_publish_loop(")
        .expect("the doorbell worker's loop still exists");
    // ⊘ Bounded to the worker function by the NEXT top-level `fn` after it, so a matching
    // string somewhere else in a 20 000-line file cannot satisfy this test.
    let rest = &SHIM[loop_start + 1..];
    let loop_end = rest.find("\nfn ").map_or(rest.len(), |i| loop_start + 1 + i);
    let body = &SHIM[loop_start..loop_end];

    let adopt = body
        .find("port.adopt_pending_channel_rings(false);")
        .expect(
            "the doorbell worker must call `adopt_pending_channel_rings(false)`. Without it \
             the ring adopt and its page-table settlement have NO caller on the arm that has a \
             worker, and every passthrough channel is born without an adopted ring — silently",
        );
    let drain = body
        .find("report_channel_birth_drain(")
        .expect("the worker still drains channel births");
    assert!(
        adopt < drain,
        "the adopt must run BEFORE the birth drain: the drain births the host channel over the \
         ring this pass joins, so an adopt after it has nothing left to prepare for. That \
         ordering is the function's own documented obligation and on the deferring arm it was \
         not true of ANY single thread before w754"
    );
    assert!(
        !body.contains("port.adopt_pending_channel_rings(true)"),
        "the worker is not a vCPU and must not claim to be one: the `on_vcpu` flag feeds the \
         RING-ADOPT census, and a worker passing `true` would report a constraint-4 violation \
         that is not happening while hiding one that is"
    );
}

/// ★★★ **The vCPU call site survives — and ONLY for the arm that has no worker.**
///
/// ⚠ Both halves fail closed, and they fail in opposite directions:
///   * the call vanishing entirely breaks the `KAYFABE_DOORBELL_ASYNC=off` arm, where nothing
///     else would ever adopt a ring (`no_worker_still_drains.rs` is this file's sibling and
///     was paid for by exactly that regression);
///   * the call becoming unconditional again puts the 25 ms settlement back inside a guest
///     MMIO store.
#[test]
fn the_vcpu_adopts_rings_only_when_there_is_no_worker() {
    let site = SHIM
        .find("if latch_changed && inline_because_no_worker {")
        .expect(
            "the vCPU's ring adopt must be gated on there being no doorbell worker. An \
             unconditional call is the w752 shape: `worst_trap=24999us at=bar0+0x110c00`, \
             96 % CPU, crossing no `assert_lock_free` door",
        );
    let after = &SHIM[site..];
    let end = after.find('}').expect("the gated block closes");
    assert!(
        after[..end].contains("self.adopt_pending_channel_rings();"),
        "the gate must guard the adopt itself, not something else that happens to sit near it"
    );
    // ⊘ And the gate is ONE decision with ONE spelling. Three sites turn on it; three
    // hand-written copies of `MATERIALIZE_INLINE … || !defers()` is how an arm comes to be
    // half-moved, which is this exact defect's history.
    assert_eq!(
        SHIM.matches("let inline_because_no_worker = *MATERIALIZE_INLINE")
            .count(),
        1,
        "the no-worker decision must be computed once per trap and reused"
    );
}

/// ⊘ **The census must be able to say `on_vcpu`, or its zero is unreadable.**
///
/// A counter that no reachable line increments reports zero forever and reads exactly like a
/// clean boot — this campaign's most-repeated failure. The vCPU call site passes `true` and the
/// worker passes `false`; both spellings must exist in the source, or one of the two arms of
/// the live known-positive is impossible by construction.
#[test]
fn both_arms_of_the_ring_adopt_census_are_reachable() {
    assert!(
        SHIM.contains("self.doorbell_port.adopt_pending_channel_rings(true);"),
        "no caller passes `on_vcpu = true`, so `RING-ADOPT on_vcpu=0` would be VACUOUS — \
         indistinguishable from the property actually holding"
    );
    assert!(
        SHIM.contains("port.adopt_pending_channel_rings(false);"),
        "no caller passes `on_vcpu = false`, so `off_vcpu=0` cannot be read as `the move did \
         not take` either"
    );
    for field in [
        "RING-ADOPT ran=",
        "off_vcpu=",
        "on_vcpu=",
        "nothing_pending=",
    ] {
        assert!(
            SHIM.contains(field),
            "the RING-ADOPT census lost the field `{field}`; a boot log's fields are an \
             interface the moment a harness greps them"
        );
    }
}
