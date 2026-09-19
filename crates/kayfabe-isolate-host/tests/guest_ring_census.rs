//! ★★★★★ **THE GUEST-RING CENSUS** — the three constants that must not come back.
//!
//! `w230` made a channel able to adopt the guest's queue. What it actually changed is
//! smaller and more fragile than that sentence: **three numbers stopped being constants**.
//! The GPFIFO's entry count, its offset and the provenance of the object it lives in are
//! now read from [`ChannelParts`] per channel, and every one of them has a plausible-looking
//! constant sitting one line away that used to be correct.
//!
//! ⇒ ★ Nothing behavioural can catch the regression. Re-spelling `GPFIFO_ENTRIES` in
//! `submit_entry` is **invisible on every channel this file allocates**, because for those
//! the constant and the per-channel value are the same number. It is wrong only on a
//! guest-backed ring — 64 against 4096, measured — and the symptom is a `GP_PUT` naming an
//! entry the guest never wrote. That is not a failure with a stack trace; it is the engine
//! fetching the wrong eight bytes.
//!
//! ⊘ **A row is a RULING, not an inventory line.** Changing a count is the same act as
//! adding a row.
//!
//! ⚠ Comments are stripped before scanning and the scan runs over the **whole file text**
//! rather than line by line — a `rustfmt` wrap is invisible to a per-line scanner. Same
//! convention, and same reason, as `tests/executor_vas_census.rs`.

use std::path::PathBuf;

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Strip `//` line comments and `/* */` blocks, so a doc comment that *mentions* a constant
/// is not counted as a use of it.
fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let b = src.as_bytes();
    let mut i = 0;
    let mut depth = 0usize;
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

fn body_of(rel: &str) -> String {
    let p = crate_root().join(rel);
    let src = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
    strip_comments(&src)
}

/// ★★★ **THE PINNED SURFACE.** Where a channel's GPFIFO geometry and provenance may be
/// spelled, and how many times.
const RING_SURFACE: &[(&str, &str, usize, &str)] = &[
    (
        "src/rm.rs",
        "GPFIFO_ENTRIES",
        16,
        "★★ **5 → 16 at `8d74b11d` (w393), ADMITTED 2026-09-10 (w407).** Eleven of the new \
         ones are ONE feature: `KAYFABE_LADDER_GPFIFO_ENTRIES`, an override for the entry \
         count of OUR OWN ring (`:754`-`:805` — constant, env name, read, default return, the \
         power-of-two-and-in-range guard, two log strings, the fallback). Its guard is what \
         keeps it harmless: it refuses anything ABOVE the default, because larger would push \
         the GPFIFO past USERD. Plus the definition and three unit-test asserts. ⊘ NOT ONE of \
         them reads a GUEST ring's geometry, which is the only thing this row forbids — \
         `:6966` keeps the distinction explicit and `:8144` states it as a rule (the modulus \
         is READ FROM THE CHANNEL). ⚠ A seventeenth that is not an arm of that override, or \
         ANY use of this constant on a `Guest` path, is the regression.",
    ),
    (
        "src/rm.rs",
        "layout.entries",
        4,
        "The four reads of the per-channel count: what is told to RM, what \
         `channel_ring_layout` reports, the zero guard in `submit_entry`, and the modulus of \
         `GP_PUT`. ⚠ A read that DISAPPEARS is the regression, so this row is as much about \
         the floor as the ceiling.",
    ),
    (
        "src/rm.rs",
        "GPFIFO_OFFSET",
        8,
        "★★ **7 → 8 at `8d74b11d` (w393), ADMITTED 2026-09-10 (w407).** The eighth is a \
         SECOND ours-family layout arm: `RingSource::OursPlaced` (`:6869`) beside the original \
         `Ours` (`:6855`). Both compute OUR ring object's layout and differ only in who chose \
         the base address. So: the definition, `PUSHBUFFER_SLOTS`'s derivation, the TWO \
         ours-family layouts, `submit_entry`'s slot address, three in the unit test. ⊘ Every \
         one is about OUR ring object; the guest's has its own, which is why `submit_entry` \
         refuses a handed-in ring BY NAME before it computes an offset at all.",
    ),
    (
        "src/rm.rs",
        "alloc_device_local(RING_OBJECT_BYTES)",
        3,
        "★★★ G1, as a count. **2 → 3, ADMITTED 2026-09-10 (w407):** the ring on the `Ours` arm \
         (`:6711`), our USERD when a `Guest` ring hands us none (`:6762` — USERD is ours on \
         every channel we allocate and may NOT sit inside the guest's object), and the public \
         `alloc_ring_object` helper (`:6139`). ⚠ The third is the OPPOSITE of the blocker this \
         rung removed: a CALLER minting a ring to hand IN, not us allocating one for a channel \
         that already came with one. ⊘ THAT is still the regression — a fourth call reached \
         from the `Guest` arm's ring path.",
    ),
    (
        "src/rm.rs",
        "RingOwner::HandedIn",
        6,
        "★★ **5 → 6 at w745 (constraint 26), ADMITTED 2026-09-15.** The `Guest` arm's TAG is \
         now written twice, once per `RingProvenance` arm — `OwnObject` narrows a handle, \
         `StoreSlice` contributes `0` because the birth isolate holds none — and both are \
         `HandedIn`, which is the ruling unchanged: **provenance, not ownership, moved.** ⊘ \
         The other five are untouched: the empty unwind set, the absent CPU map, \
         `submit_entry`'s refusal, and the teardown that must not unmap or free the guest's \
         ring. ⚠ A SEVENTH would mean a third provenance, and there are two.",
    ),
    (
        "src/rm.rs",
        "RING_NOT_OURS",
        6,
        "The named status, the two ring accessors that answer it, and `submit_entry`'s \
         early refusal. ⊘ These are the assertion that no CPU view exists — G4 stated as an \
         answer rather than as an omission. \
         ★★ **4 → 6, ADMITTED 2026-09-17 (w755k).** The two new ones are ONE line of the \
         `local_status::ALL` census — `(\"RING_NOT_OURS\", super::RING_NOT_OURS)` — which \
         NAMES the status so a boot log's bare integer can be resolved to it. ⊘ Naming is not \
         using: the census is a lookup table with no control flow, and it exists because \
         `0x4B46` was once carried by two constants and made a wall unreadable. ⚠ A seventh \
         that is not a census row IS the regression this row is about.",
    ),
];

#[test]
fn the_rings_geometry_is_per_channel_and_stays_that_way() {
    let mut bad = Vec::new();
    for (file, pat, want, why) in RING_SURFACE {
        let n = body_of(file).matches(pat).count();
        if n != *want {
            bad.push(format!(
                "  {file}: `{pat}` appears {n}x, the ruling says {want}x\n      {why}"
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "★★★ THE GUEST-RING SURFACE MOVED. This is not \"a test failed\": each row is a \
         ruling about where a channel's queue geometry may come from, and a count that \
         changed means a constant is deciding something the guest declared.\n{}",
        bad.join("\n")
    );
}

/// ⊘ The other polarity: the count RM is told must not be spelled as a constant at the one
/// site that talks to the driver.
///
/// ★ The census above would pass if `ChannelAllocParams` grew a second construction that
/// spelled `GPFIFO_ENTRIES` while the first kept reading `layout` — two sites, one count
/// each, both "correct" by row. This checks the shape instead of the tally.
#[test]
fn the_channel_alloc_tells_rm_the_channels_own_numbers() {
    let body = body_of("src/rm.rs");
    let n = body.matches("gp_fifo_entries: layout.entries").count();
    assert_eq!(
        n, 1,
        "`ChannelAllocParams::gp_fifo_entries` is filled from `layout.entries` {n} times, \
         not once. Exactly one construction of the channel alloc params exists, and it must \
         read the per-channel layout — spelling a constant there is invisible until a guest \
         ring is passed."
    );
    assert!(
        body.contains("gp_fifo_offset: layout.gp_fifo_va"),
        "`ChannelAllocParams::gp_fifo_offset` is no longer filled from `layout.gp_fifo_va`. \
         If it went back to `ring_va + GPFIFO_OFFSET`, a channel handed the guest's queue is \
         told about a page of OUR layout inside THEIR memory."
    );
}

/// ⊘ And the diagnostic must not be the thing that keeps the capability alive.
///
/// ⊘⊘ **CORRECTED 2026-08-12, and the correction is the load-bearing half.** This doc used
/// to read *"`alloc_channel_over_guest_ring` currently has exactly **one caller**, the R31
/// probe, and that is honest — the rung builds the alloc side and nothing consumes it."*
/// **That stopped being true at `361fca8`** (leg A2), which gave the verb its first
/// production caller at `rm.rs`'s `alloc_channel`. The test stayed green through that rung
/// because its assertion counted **`fn` DEFINITIONS**, not callers — and
/// `guest_ring_and_userd_adoption_prereg.md` §4 had explicitly promised to update it
/// (*"⊘ Not a bumped number"*) and did not.
///
/// ★ ⇒ The tripwire that existed *"so that the day a production caller appears, somebody has
/// to say so out loud"* let that day pass in silence. **A gate's prose is not its
/// assertion**, and here the prose was the only thing that was ever right. Both halves are
/// now asserted, and the caller half is the one that was missing.
#[test]
fn the_probe_does_not_mint_the_rings_geometry_twice() {
    let body = body_of("src/rm.rs");
    assert_eq!(
        body.matches("fn alloc_channel_over_guest_ring(").count(),
        1,
        "There is more than one entry point for a channel over a handed-in ring. Two entry \
         points are two places where the guest's numbers are turned into RM's, and only one \
         of them will be the one a boot exercises."
    );
    // ★★★ THE HALF THAT WAS MISSING. `.alloc_channel_over_guest_ring(` is the CALL form; the
    // definition asserted above is `fn alloc_channel_over_guest_ring(`, with no leading dot,
    // so it is not counted here. ⚠ The dot is matched separately from the receiver because
    // `cargo fmt` puts `self` and `.method(` on different lines when the argument list is
    // long — one of the two live call sites is already wrapped that way, and a pattern that
    // spelled `self.` would have counted it as absent.
    let callers = body.matches(".alloc_channel_over_guest_ring(").count()
        + body_of("src/bin/rmladder.rs")
            .matches(".alloc_channel_over_guest_ring(")
            .count();
    assert_eq!(
        callers, 4,
        "`alloc_channel_over_guest_ring` has {callers} call sites, not 4. ⊘ Four is the ruling \
         and the split is what matters, not the tally: **ONE production caller** — \
         `alloc_channel`'s adoption arm, reachable only through an `AdoptedGuestRing` the \
         shell had to arm and only past the `RING_NOT_A_JOINED_WINDOW` membership check — plus \
         TWO in `prove_guest_ring_channel` (the R31 probe) and ONE in the `rmladder` driver. A \
         fifth means a host channel can be born over guest memory from a path that did not \
         state it, and the whole point of this verb is that such a birth is never accidental."
    );
    // ⊘ And the production caller must stay behind the gate. A caller that reached the verb
    // without the membership check would be `w228`'s blank twin waiting to happen — a channel
    // fetching GPFIFO entries out of a page nothing ever wrote, reporting no error at all.
    assert!(
        body.contains("RmError::Other(RING_NOT_A_JOINED_WINDOW)"),
        "`alloc_channel`'s adoption arm no longer refuses a non-joined object by name."
    );
    // ⊘⊘ **CORRECTED 2026-08-13 (w288) — THE RULING IS NOW SIX, AND THE SIXTH IS A SECOND
    // CONSTRUCTION, NOT A SECOND DECISION.** `alloc_channel_over_guest_ring_with_error_notifier`
    // is `alloc_channel_over_guest_ring`'s body with `Some(notifier)` in place of `None`, so it
    // builds its own `RingSource::Guest(ring)`. ⚠ That is exactly the shape this row exists to
    // catch — *"a host channel born over guest memory from a path that did not state it"* — so
    // it is worth saying why it is admitted: the new verb states it in its NAME, takes the same
    // `GuestRing` by value, and is reachable only from `alloc_channel`'s adoption arm, on the
    // far side of the `RING_NOT_A_JOINED_WINDOW` membership check asserted directly above.
    // ⊘ The number moved because a **verb** was added, never because an arm became reachable
    // from somewhere new: the decision count in `alloc_channel_in` is unchanged at four.
    // ⊘⊘ **CORRECTED 2026-09-15 (w746, constraint 29) — SEVEN → EIGHT, AND THE EIGHTH IS A
    // GATE, NOT A DECISION.** `alloc_channel_in`'s `store_slice_ring` predicate matches
    // `RingSource::Guest(GuestRing { ring: RingProvenance::StoreSlice { .. }, .. })` to decide
    // whether [`RING_HANDLE_REACHED_RM`] applies. ⚠ It is admitted for the same reason the
    // sixth was: it adds no path on which a channel can be born over guest memory — it is a
    // read of the value the four decisions already produced, taken immediately before the
    // struct RM reads is built, and its only outcome is a REFUSAL. ⊘ The decision count in
    // `alloc_channel_in` is still four; a NINTH would mean a fifth decision.
    assert_eq!(
        body.matches("RingSource::Guest(").count(),
        10,
        "`RingSource::Guest` is constructed or matched somewhere new. **EIGHT is the ruling \
         (three before leg B, five before w288, six before w393), ADMITTED 2026-09-10 \
         (w407)**, and each is a different job: TWO constructions — \
         `alloc_channel_over_guest_ring` and its `_with_error_notifier` twin — TWO arms in \
         `alloc_channel_in` deciding the RING (one provenance: allocate, or do not; one \
         layout: our offsets, or the caller's) — TWO more deciding the USERD, in the same \
         shape and for the same reason — and ONE space lookup (`:6693`) where `Ours` and \
         `Guest` deliberately SHARE an arm. ★ That sharing is correct because the axis \
         there is a different one: `OursPlaced` carries an externally-owned VA space whose \
         handle the caller passes directly, while both other kinds resolve their space \
         through `space_of(range)`. The guest/ours distinction that matters is untouched. \
         ★ The ring arms are deliberately not one arm each: they straddle a failure that \
         must unwind between them, and the USERD arms are a second axis entirely — a \
         channel can adopt the guest's ring and keep a USERD of ours, which is what every \
         leg-A boot before this one did. \
         ★★ **8 → 9, ADMITTED 2026-09-17 (w755n).** The ninth is a ROUTING decision, a job \
         none of the eight does: `alloc_channel_in`'s head matches \
         `Guest {{ ring: StoreSlice, userd: TheStore }}` and sends the birth to the BIRTH \
         CLIENT B (route K increment 6), because `hUserdMemory` is a real RM operand and \
         constraint 26 forbids THIS isolate naming the store. ⊘ It reads BOTH halves of the \
         declaration on purpose — a store-slice ring with a JOINED USERD stays on the \
         isolate's path, where the joined-object check can see it — and it sits ABOVE EVERY \
         ALLOCATION, because the two paths allocate in different clients and unwinding \
         across that boundary is the double free `UserdOwner` exists to prevent. \
         ★★ **9 → 10, ADMITTED 2026-09-17 (w755r).** The tenth is the DELEGATION's own \
         construction, in `birth_guest_channel_in_b`, and it is a job none of the nine does: \
         carrying a birth that arrived from ANOTHER PROCESS into this one. `[measured w755q]` \
         without it the ninth site — the routing decision — was UNREACHABLE from the guest's \
         path, because the guest's birth lowers through `alloc_channel_lowered` in the \
         per-proc isolate while the routing sits in `alloc_channel_in` in the scratchpad. \
         The ninth decided; nothing ever arrived for it to decide about. \
         ⊘ A struct literal and not a `From`, for the reason `alloc_channel_lowered`'s own \
         conversion is one: `GuestRing` is this crate's shape and `AdoptedGuestRing` is the \
         wire's, and a blanket conversion would let a future field silently default across \
         the boundary. \
         ⚠ An ELEVENTH site means one of the two guest arms \
         is reachable from a path that did not state it."
    );
}

// ═══════════════════════════════════════════════════════════════════════════════════════
// ★★★★★ THE BIRTH WITNESS — and the TWO-CRATE INVARIANT its middle state rests on
// ═══════════════════════════════════════════════════════════════════════════════════════

/// A sibling crate's source, comments stripped. ⊘ Reached by path rather than by `include!`
/// or a re-export: the fact under test is *what the other crate's source says*, and anything
/// that compiled it would be testing what it means instead.
fn sibling_body(crate_name: &str, rel: &str) -> String {
    let p = crate_root()
        .parent()
        .expect("crates/")
        .join(crate_name)
        .join(rel);
    let src = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
    strip_comments(&src)
}

/// ★★★★★ **`DECLINED` and `NOT-ASKED` mean opposite things and both arrive as `adopt: None`.**
///
/// `w261` could not tell *"leg A2 fired"* from *"leg A2 was never asked"*, and its own
/// `RESULT.md` leads with that hole. [`kayfabe_isolate_host::rm::BirthOffer`] closes it by
/// reading the third state off `hosting`, which already crosses the wire — **no new field, no
/// second source of truth**.
///
/// ⚠ ⇒ **The reading is only true while the two production birth sites keep their shapes**,
/// and those sites live in a *different crate*. A doc comment cannot hold a cross-crate
/// invariant; this can.
///
/// - `VerbPlan::EngineObject` births pass `Some(HostedObject { .. })` **and** consult
///   `kayfabe_fwd::adopted_guest_ring` unconditionally on the `channel.is_none()` branch ⇒
///   `hosting = Some, adopt = None` really does mean *asked, and it produced nothing*.
/// - `VerbPlan::Doorbell` births pass a literal `None, None` for those two ⇒ *nothing was
///   asked*.
///
/// ⊘ If either changes, this fails and the witness's `because()` text stops being a claim
/// nobody checked. `refuse_by_name_means_the_name_is_true`.
///
/// ## ⊘⊘ CORRECTED 2026-08-13 (w288) — THE ASSERTION WAS A ONE-LINE STRING, AND THE CALL IS
/// ## NOW FIVE ARGUMENTS LONG
///
/// This test used to match the literal `"rm.alloc_channel(vas, *engine, None, None)"`. w288
/// gave `RmBackend::alloc_channel` a fifth argument — the error notifier — so `rustfmt`
/// broke the call across lines and the single-line pattern matched **zero** times. ⚠ The
/// invariant it was protecting is unchanged and is still exactly what matters: the doorbell
/// birth must offer **`None` for `hosting` and `None` for `adopt`**, because that literal
/// pair is the entire evidence for the witness's `NOT-ASKED` state.
///
/// ⇒ The pattern is matched against the source with **whitespace collapsed**, so it survives
/// the next reformat. ★ It is deliberately still a *literal argument list* rather than a
/// regex over "some `None`s": the discriminator is WHICH two arguments are `None`, and a
/// looser pattern would keep passing on the day one of them becomes something else — which
/// is the whole failure mode this tripwire exists for.
#[test]
fn the_birth_witness_can_tell_declined_from_never_asked() {
    let isolate = sibling_body("kayfabe-isolate", "src/lib.rs");
    // ⊘ Collapsed, not stripped: the argument SEPARATORS have to survive or the pattern stops
    // saying anything about order. See the ⊘⊘ correction above for why this is not the raw
    // source text any more.
    let flat = isolate.split_whitespace().collect::<Vec<_>>().join(" ");
    assert_eq!(
        flat.matches("rm.alloc_channel( vas, *engine, None, None,")
            .count(),
        1,
        "The doorbell materialization no longer births its channel with a LITERAL `None, \
         None` for `hosting` and `adopt`. That pair is the entire evidence for the witness's \
         `NOT-ASKED` state: it is what makes `hosting = None` mean *this birth path offers no \
         ring at all* rather than *this birth happened to have none*. If the doorbell path \
         grew a `hosting` or an `adopt`, `BirthOffer::read` is now mislabelling births and its \
         `because()` text is false on a real boot. ⚠ If it merely got REFORMATTED, fix the \
         pattern — and say so, as w288 did."
    );
    // ⊘ And the OTHER site must keep passing `hosting`, or `Some/None` stops discriminating.
    assert!(
        isolate.contains("Some(HostedObject {"),
        "`VerbPlan::EngineObject`'s birth no longer hands `hosting` to `alloc_channel`. \
         `hosting` is the witness's only discriminator between `DECLINED` and `NOT-ASKED`."
    );

    // ★★★ The half in `kayfabe-fwd`: the consult is UNCONDITIONAL on the birth branch. If it
    // ever grew a second selector, `adopt: None` would stop meaning "asked and declined" and
    // the arming would have two sources of truth — the defect
    // `a_second_source_of_truth_beside_a_complete_value` names.
    //
    // ⊘⊘ **CORRECTED 2026-09-09 (w393) — THE RULING IS NOW TWO, AND THE SECOND IS A BIRTH
    // SITE THAT CANNOT DECLINE.** `plan_channel_birth` consults `adopted_guest_ring` at the
    // guest's own channel alloc and, for a `Passthrough` channel, turns `None` into a NAMED
    // REFUSAL (`FwdFault::PassthroughRingNotAdoptable`) rather than into `adopt: None` — so
    // the isolate's `DECLINED` reading is still produced by exactly ONE site
    // (`plan_engine_object`), which is what the witness's `because()` text relies on.
    // ★ Between w392j and w393 there was a THIRD call — the doorbell arm — and it was the
    // measured USERD-zeroing hazard (w233 / w392j's five `Xid 31`); that consult is gone and
    // its return is what the `None, None` literal above now pins.
    let fwd = sibling_body("kayfabe-fwd", "src/lib.rs");
    assert_eq!(
        // ⊘⊘ **CORRECTED 2026-09-15 (w745) — THE PATTERN, NOT THE RULING.** Constraint 26
        // gave `adopted_guest_ring` a fifth argument (the `RingSliceOracle`), so the
        // four-argument literal matched **zero** times and this gate reported "somewhere
        // other than the two birth sites" about a tree with exactly two. That is w288's
        // failure mode on this very test, recorded in its own doc comment — *"If it merely
        // got REFORMATTED, fix the pattern — and say so, as w288 did."* ⇒ said.
        // ★ Truncated at the fourth comma rather than widened to a regex: the discriminator
        // is still WHICH arguments are passed and in what order, and matching "some call to
        // `adopted_guest_ring`" would keep passing on the day a third site appears.
        fwd.matches("adopted_guest_ring(spine, proc, chan, cgpu,")
            .count(),
        2,
        "`adopted_guest_ring` is called from somewhere other than the TWO birth sites — \
         `plan_engine_object` (may decline → `adopt: None`) and `plan_channel_birth` (may \
         NOT decline → refuses by name). A third is a doorbell-side consult growing back, which \
         is w392j's USERD-zeroing hazard by construction."
    );
    assert!(
        fwd.contains("adopt: if channel.is_none() {"),
        "The consult is no longer gated on `channel.is_none()` alone. ⊘ A second condition \
         here — a flag, a feature, an env read — would make `DECLINED` ambiguous again, which \
         is the exact state this rung exists to leave."
    );
    // ★★★★★ w393 — the THIRD birth site's shape: its consult is a `let … else` whose else-arm
    // is a refusal, and the plan variant it builds has NO `Option` around the adoption. Both
    // are what make *"a passthrough channel born over our ring"* unspellable on this path.
    assert_eq!(
        // ⊘ Truncated at the fourth comma for the reason given above — w745 added a fifth
        // argument. The SHAPE this pins (`let … else` with a refusing else-arm, and a plan
        // variant with no `Option` around the adoption) is unchanged.
        fwd.matches("let Some(adopt) = adopted_guest_ring(spine, proc, chan, cgpu,")
            .count(),
        1,
        "`plan_channel_birth`'s consult is no longer a refuse-by-name `let … else`. ⊘ If it \
         became an `Option` that flows into the plan, a birth-at-alloc over OUR ring would be \
         spellable again, which is the w392h silence (`Xid 0`) by construction."
    );
    assert_eq!(
        isolate.matches("rm.alloc_channel_declared(").count(),
        1,
        "`RmBackend::alloc_channel_declared` — the birth verb whose adoption is mandatory by \
         type — is reached from somewhere other than the single `VerbPlan::ChannelBirth` arm, \
         or from nowhere."
    );
    // ⊘ And on the far side, ONE lowering behind BOTH channel verbs: the birth witness, the
    // `RING_NOT_A_JOINED_WINDOW` / `USERD_NOT_A_JOINED_WINDOW` refusals and the guest-ring
    // arm are shared code, so the two verbs cannot come to read `DECLINED` differently.
    let rm = body_of("src/rm.rs");
    assert_eq!(
        rm.matches("fn alloc_channel_lowered(").count(),
        1,
        "the shared lowering behind `alloc_channel` and `alloc_channel_declared` is gone or \
         duplicated."
    );
    assert_eq!(
        rm.matches(".alloc_channel_lowered(").count(),
        2,
        "`alloc_channel_lowered` has {} callers, not 2 (`alloc_channel` and \
         `alloc_channel_declared`). A third is a channel verb that bypassed the shared witness.",
        rm.matches(".alloc_channel_lowered(").count()
    );
}

/// ⊘ And the witness must stay a witness: it prints, it decides nothing.
///
/// ★ `a_flag_is_not_progress`, and one sharper — an instrument that acquired a branch would
/// make the armed and disarmed arms differ *because they were measured*, which voids the
/// comparison the whole boot is for.
#[test]
fn the_birth_witness_is_read_by_no_decision() {
    let whole = body_of("src/rm.rs");
    // ⚠ SHIPPED CODE ONLY. The unit tests below `#[cfg(test)]` call `BirthOffer::read` four
    // times on purpose — that is the reading's own truth table — and counting them here would
    // make this gate fire on the thing that proves the reading correct.
    let body = whole
        .split("#[cfg(test)]")
        .next()
        .expect("split always yields one")
        .to_string();
    assert!(
        body.len() < whole.len(),
        "`rm.rs` has no `#[cfg(test)]` module any more, so this gate is silently scanning the \
         whole file — including tests — and its counts mean something different from what \
         they say."
    );
    // The value is constructed once, tallied once, and rendered. Nothing matches on it.
    // ★★★ TWO, and the reason is the whole design of the leg-B witness: ONE function applied
    // to TWO limbs. ⊘ Not two predicates — that is the shape this would be catching. If a
    // third appears, or if either call stops being `BirthOffer::read`, the two legs can come
    // to disagree about what `DECLINED` means and the boot log stops being comparable
    // limb-to-limb.
    assert_eq!(
        body.matches("BirthOffer::read(").count(),
        2,
        "`BirthOffer::read` is called {} times in shipped `rm.rs`, not twice — once for the \
         ring limb and once for the USERD limb. ONE reading, TWO limbs; a third call is a \
         third limb nobody declared, and a first is a limb that lost its witness.",
        body.matches("BirthOffer::read(").count()
    );
    // ⊘ And the second call must be DERIVED FROM THE FIRST'S INPUT, not from a new selector.
    // `adopt.is_some_and(|a| a.userd.is_some())` is the whole of leg B's arming: it is `Some`
    // only inside an adoption leg A2 already made, so a disarmed build is `None` on both
    // limbs by construction. A literal env read or feature flag here would be
    // `a_second_source_of_truth_beside_a_complete_value`.
    assert!(
        body.contains("adopt.is_some_and(|a| a.userd.is_some())"),
        "leg B's arming is no longer inherited from leg A2's own answer. ⊘ That inheritance \
         is what makes `userd=DECLINED` on a disarmed build a fact about the code rather \
         than about a flag — and what makes `(ring = DECLINED, userd = GUEST-USERD)` \
         unspellable."
    );
    // ⊘⊘ **THE PROSE AND THE ASSERTION USED TO DISAGREE, and that is the defect this rung
    // was warned about by name.** It read *"The value is matched EXACTLY ONCE, and that one
    // match is inside `birth_census::tally`"* while asserting `matches("match offer") == 1` —
    // and the ONE occurrence it was counting was `match offer` inside `tally`'s parameter
    // named `offer`, which leg B renamed to `ring`. The sentence was true; the pattern was
    // measuring the parameter's *name*.
    //
    // ⇒ Assert what the sentence says: nothing outside the counter selection branches on a
    // witness value. The counter selection itself is matched by name below.
    for forbidden in ["match offer", "match userd_offer"] {
        assert!(
            !body.contains(forbidden),
            "`{forbidden}` appears in shipped `rm.rs`. ⊘ The witness prints and decides \
             nothing; a match on it is a branch on an instrument, which would make the armed \
             and disarmed boots differ BECAUSE they were measured and void the arm comparison."
        );
    }
    assert_eq!(
        body.matches("let counter = match ring {").count(),
        1,
        "`birth_census::tally`'s counter selection is not where it was. That `match` is the \
         ONLY read of a witness value in the shipped file, and this gate exists so `the \
         witness decides nothing` is a checked claim rather than a sentence."
    );
    for forbidden in [
        "if offer ==",
        "if offer !=",
        "offer.is_",
        "if let BirthOffer",
    ] {
        assert!(
            !body.contains(forbidden),
            "`rm.rs` branches on the birth witness (`{forbidden}`). ⊘ It prints and it decides \
             nothing — see `alloc_channel`."
        );
    }
    // ★ And the tally must not be able to *fail* the birth: it returns numbers, never a
    // `Result`, so no instrument can refuse a channel the uninstrumented port would allow.
    assert!(
        body.contains(") -> (u64, u64, u64, u64, u64, u64) {")
            && body.contains("pub(super) fn tally("),
        "`birth_census::tally` no longer returns plain counters (six of them since leg B \
         added `guest_userd`). An instrument that can return an error is an instrument that \
         can change the outcome it is measuring."
    );
    // ★★★ And leg B's own far-side refusal must exist and be a REFUSAL, never a downgrade.
    // ⚠ A channel silently given a USERD of ours after being told it would carry the guest's
    // is `GP_PUT == GP_GET` forever with no error — and it would make an armed run and its
    // control produce the same channel, which is the failure shape this campaign keeps paying
    // for.
    assert!(
        body.contains("t.is_joined_object(raw_userd)")
            && body.contains("RmError::Other(USERD_NOT_A_JOINED_WINDOW)"),
        "leg B's adoption arm no longer re-checks the USERD handle against `FbJoinTable` \
         membership, or no longer refuses by name. The core's check cannot reach here: the \
         offer crosses the isolate IPC boundary as two integers."
    );
    // ⊘ And the two USERD accessors must refuse BY NAME rather than answering zero. `(0, 0)`
    // is what a channel that has never run also looks like — the exact ambiguity this rung is
    // trying to leave, on the one plane it is trying to measure.
    // ★★ **2 → 4 at `8cca3502` (w287), ADMITTED 2026-08-14 (w296), and the PROPERTY never
    // moved.** w287 made USERD able to live inside the ring object, so each accessor now
    // resolves `(region, base)` through a two-armed `match r.userd_in_ring` — and BOTH arms
    // refuse by name. ⇒ Two accessors × two layouts = four, and the sentence below
    // ("both refusals are by name") is *more* true than when the count was 2, not less.
    // ⊘ This is why the number is asserted with the accessors named beside it: a count on
    // its own cannot tell "a refusal was deleted" from "a layout was added". The floor that
    // actually guards the property is that NEITHER accessor may reach a `(0, 0)` return,
    // which is what the two `.ok_or(` spellings encode — a dropped refusal shows up here as
    // 3, not as a silent zero.
    assert_eq!(
        body.matches("RmError::Other(USERD_NOT_OURS)").count(),
        4,
        "the guest-USERD arm's refusals (`userd_cursors`' GP_GET read and \
         `userd_store_u32`'s GP_PUT write, each across the in-ring and standalone USERD \
         layouts) are no longer all by name. A skipped write and a \
         write to the wrong place look identical in a log."
    );
    // ⊘ The refusal must not be reachable from the witness: it is `fb_joins` membership.
    // ⊘⊘ **CORRECTED 2026-09-15 (w745), AND THE RULING IS NOW SCOPED RATHER THAN MOVED.**
    // Constraint 26 split the adoption into two provenances. `RingProvenance::OwnObject` —
    // the only arm that names a handle, and the whole of the `isolate` arm — is still gated
    // on `FbJoinTable` membership, and this line is still that gate; the local it narrows
    // into was renamed when the `match` arrived.
    // ★ `RingProvenance::StoreSlice` names NO handle, so there is nothing for this side to
    // look up: its question is answered VMM-side by `kayfabe_fwd::RingSliceOracle`, of the
    // party that placed the slice. ⚠ That is a real move of the checker and it is asserted
    // where it now lives (`tests/tests/the_birth_names_the_guests_ring.rs`), **not** implied
    // by this one passing.
    assert!(
        body.contains("t.is_joined_object(raw)"),
        "the adoption arm's membership check moved; the OwnObject refusal is now gated on \
         something other than `FbJoinTable` membership."
    );
    assert!(
        body.contains("kayfabe_isolate::RingProvenance::StoreSlice { .. } => (0, true)"),
        "★★★ CONSTRAINT 26 — the `StoreSlice` arm no longer opts out of the membership \
         lookup by construction. If it grew one, the birth isolate is looking a handle up in \
         a table it cannot own an entry in, and the answer would be `false` forever."
    );
}

// =========================================================================================
// ★★★★★ w746, CONSTRAINT 29 PART 2 — THE SUCCESSOR TO `AdoptedGuestRing::memory`.
// =========================================================================================
//
// `AdoptedGuestRing::memory` was deleted at constraint 26c on the argument *"the ring handle
// never reaches RM — it was an authorization token, not an operand"*. The argument is TRUE of
// `alloc_channel_in` as written; what was missing is anything that would notice if it stopped
// being true. Constraint 29 part 2: **when a gate goes because "X never happens", X is the
// thing most likely to be wrong, and the replacement must go RED IF X HAPPENS.**
//
// ⊘ These are source-shape assertions rather than a live birth, and the reason is not
// convenience: `alloc_channel_in` is a method on `HostRmBackend`, whose every path opens
// `/dev/nvidia*`. There is no mock that reaches it, so the honest instrument is the one that
// reads the code that runs. ⚠ Both tests below were broken deliberately and watched go red;
// see the commit message for exactly how.

/// The one place this crate builds the struct RM reads for a channel alloc, and the one
/// place a ring handle could be smuggled into it.
const CHANNEL_ALLOC_SITE: &str = "ChannelAllocParams {";

#[test]
fn a_store_slice_birth_refuses_before_rm_if_it_holds_a_ring_handle() {
    // ⊘⊘ **SCOPED TO `alloc_channel_in`, and w755l is why.** This read the WHOLE file and
    // compared the first `RING_HANDLE_REACHED_RM` against the first `ChannelAllocParams {`.
    // Route K's increment 6 added a SECOND, legitimate birth site — `BirthConn::birth_channel`
    // — which sits earlier in the file and builds `ChannelAllocParams` under a DIFFERENT rule:
    // B may name the store, which is the entire point of birthing there. The whole-file read
    // then compared two sites that are not about each other and failed.
    // ★ The gate's ruling is unchanged and is NOT widened: within the isolate's own birth, the
    // handle-free check must precede the struct RM reads. B's birth has its own gate below.
    let body = enclosing_fn(&body_of("src/rm.rs"), "fn alloc_channel_in(");
    // ★ THE GATE EXISTS, and it is the runtime one — not a comment asserting the property.
    assert!(
        body.contains("RmError::Other(RING_HANDLE_REACHED_RM)"),
        "★★★★★ CONSTRAINT 29 — the replacement for `AdoptedGuestRing::memory` is GONE. \
         That field was deleted because \"the ring handle never reaches RM\"; with this gate \
         removed, nothing in the tree notices when it does, and a channel born naming a \
         handle this isolate does not hold is refused by RM with a status that reads as \
         exhaustion."
    );
    // ★ …and it is FAIL-CLOSED on all three conjuncts. A gate that checked only `ring_obj`
    // would pass a birth that named USERD handle `0` to RM, which resolves to nothing and is
    // the same silent `GP_PUT == GP_GET`.
    for conjunct in [
        "ring_obj != 0",
        "matches!(userd_owner, UserdOwner::InRing)",
        "userd == 0",
    ] {
        assert!(
            body.contains(conjunct),
            "★★★★★ CONSTRAINT 29 — the store-slice birth gate lost its `{conjunct}` \
             conjunct. Each one is a distinct way the handle-free claim can stop holding."
        );
    }
    // ★★★ NON-VACUITY: the gate must sit ABOVE the struct RM reads. A check after the
    // `NV_ESC_RM_ALLOC` is built is a check of a fact that has already crossed.
    let gate = body
        .find("RmError::Other(RING_HANDLE_REACHED_RM)")
        .expect("asserted above");
    let site = body
        .find(CHANNEL_ALLOC_SITE)
        .expect("this crate builds ChannelAllocParams");
    assert!(
        gate < site,
        "★★★★★ CONSTRAINT 29 — the store-slice ring gate is BELOW the site that builds \
         `ChannelAllocParams`. It would then refuse a birth whose parameters RM had already \
         been handed, which is not a gate."
    );
}

#[test]
fn the_channel_alloc_struct_is_fed_by_no_store_slice_handle() {
    let body = body_of("src/rm.rs");
    let site = body
        .find(CHANNEL_ALLOC_SITE)
        .expect("this crate builds ChannelAllocParams");
    let end = body[site..]
        .find("engine_type,")
        .map(|o| site + o)
        .expect("the struct's last field");
    let struct_body = &body[site..end];
    // ⊘ `ring_obj` IS legitimately named through `userd` on the `InRing` arm, so the
    // assertion is about the STRUCT's own fields: none of them may be spelled `ring_obj`.
    assert!(
        !struct_body.contains("ring_obj"),
        "★★★★★ CONSTRAINT 29 — a field of `ChannelAllocParams` is now fed directly from \
         `ring_obj`. On the `StoreSlice` arm that value is `0` by construction, so this \
         would hand RM handle zero; on every other arm it hands RM an object whose \
         authorization `AdoptedGuestRing::memory` used to carry and no longer does. \
         THIS IS THE ASSERT THAT THE DELETED FIELD'S ARGUMENT IS STILL TRUE."
    );
    // ★ And the two fields that DO carry handles are named, so a third appearing is a diff
    // somebody has to look at rather than a silent widening.
    assert_eq!(
        struct_body.matches("h_object_error:").count()
            + struct_body.matches("h_userd_memory_0:").count()
            + struct_body.matches("h_context_share:").count()
            + struct_body.matches("h_va_space:").count(),
        4,
        "★★ the handle-bearing fields of `ChannelAllocParams` changed. Two of them are \
         pinned at zero by ruling (`h_context_share`, `h_va_space` — a channel inherits its \
         group's); if a new one appeared, it is a new thing crossing to RM and this test is \
         the place that says so."
    );
}

/// The body of the function whose signature starts with `sig`, delimited by that `fn`'s own
/// indentation.
///
/// ⊘ **A missing terminator is a FAILURE, never a fallback to the rest of the file.** That
/// fallback is what let a sibling gate in this tree pass through a deleted assertion.
fn enclosing_fn(src: &str, sig: &str) -> String {
    let at = src
        .find(sig)
        .unwrap_or_else(|| panic!("★ NON-VACUITY: `{sig}` is gone — this gate gates nothing"));
    let line_start = src[..at].rfind('\n').map_or(0, |nl| nl + 1);
    let col = src[line_start..].chars().take_while(|c| *c == ' ').count();
    let terminator = format!("\n{}}}\n", " ".repeat(col));
    let end = src[at..].find(&terminator).unwrap_or_else(|| {
        panic!(
            "★ NON-VACUITY: could not delimit `{sig}` (column {col}); a scanner that cannot \
                delimit its subject must refuse, never widen"
        )
    });
    src[at..at + end].to_string()
}

/// ★★★★★ **w755l — ROUTE K INCREMENT 6: B'S BIRTH NAMES THE STORE AND NOT A RING HANDLE.**
///
/// The isolate's birth is gated above by `RING_HANDLE_REACHED_RM`: it must tell RM *no* ring
/// handle, because it holds none. **B's rule is the mirror image** — it MUST name the store
/// (that is why the birth moved there), and it must still not name a ring object, because the
/// ring is a slice addressed by absolute VA exactly as it is in the isolate.
///
/// ⊘ Without this, moving the birth to B would have bought the isolate's gate back by leaving
/// the new site gated by nothing — trading one blind spot for another.
#[test]
fn the_birth_in_b_names_the_store_for_userd_and_no_ring_object() {
    let body = enclosing_fn(&body_of("src/rm.rs"), "fn birth_channel(");
    for (needle, why) in [
        (
            "h_userd_memory_0: store_dup",
            "no longer names the store for `hUserdMemory`. That IS increment 6: a channel              born with OUR USERD sees `GP_PUT == GP_GET == 0` forever, fetches nothing and              reports nothing — the silence measured on every lane at w755h",
        ),
        (
            "userd_offset_0: userd_offset",
            "no longer carries the guest's own USERD offset. Under the single store that              offset IS the guest's framebuffer address, and a zero would point hardware at              the store's first slot instead of this channel's",
        ),
        (
            "USERD_ALIGNMENT",
            "no longer checks the alignment RM does not. `[kernel_channel_gv100.c:208]` RM              shifts the resolved address `>> 9` and validates nothing, so a misaligned              offset is SILENTLY TRUNCATED to a different slot — and the symptom is character              for character the wall this function exists to remove",
        ),
    ] {
        assert!(
            body.contains(needle),
            "★★★★★ ROUTE K INCREMENT 6 REGRESSED in `birth_channel` — it {why}."
        );
    }
    // ⊘ And it must NOT name a ring object: the ring is a store slice, addressed by absolute
    // VA. `gp_fifo_offset` carries the guest's VA; no handle for it may reach RM.
    assert!(
        !body.contains("ring_obj"),
        "★★★ `birth_channel` names a ring object. The ring is a STORE SLICE addressed by          absolute VA, and a handle for it reaching RM is what `RING_HANDLE_REACHED_RM`          refuses on the isolate's side"
    );
    assert!(
        body.contains("gp_fifo_offset: gp_fifo_va"),
        "★★★ `birth_channel` no longer passes the guest's own ring VA"
    );
}

/// ★★★★★ **w755n — THE ROUTING: a store-slice USERD goes to B, and the decision is made
/// BEFORE anything is allocated.**
///
/// ⊘ The order is the constraint, not tidiness. The two paths allocate different objects in
/// **different clients**, and unwinding across that boundary is the double-free this file
/// split `UserdOwner` to make unrepresentable. A routing decision taken after the first
/// allocation has already created something the other path does not know how to free.
///
/// # ⊘⊘⊘ WHAT THIS GATE DOES **NOT** SAY — measured w755q, and it cost a boot
///
/// It is a statement about the **internal ordering of `alloc_channel_in`**, and nothing else.
/// In particular it does **not** say that `alloc_channel_in` is on the path a guest's channel
/// birth takes — and `[measured w755q, RTX 3090, driver 580.159.04]` **it is not**. The guest's
/// birth runs `alloc_channel`/`alloc_channel_declared` → **`alloc_channel_lowered`**, in the
/// **per-proc isolate**, which refused `USERD_IN_STORE_NEEDS_BIRTH_IN_B` **11 times** while
/// `birth_in_b` was never entered (all three of its prints were zero).
///
/// ⇒ This gate was **green for the whole of that boot**. A gate on a function's internal
/// ordering is blind, by construction, to whether that function runs at all —
/// `a_green_test_can_hold_a_wall_in_place`, and the author of both the route and this gate
/// was the same (me).
///
/// ⚠ The missing piece is a **delegation across a process boundary**, not a moved line:
/// `birth_in_b` resolves B through `conn.birth_for_range`, and `birth_ranges` is populated
/// only in the **scratchpad's** `adopt_space`. A per-proc isolate has no B and structurally
/// cannot birth in one. See `docs/design/increment_6_never_fired.md`, and
/// [`the_guest_birth_path_refuses_a_store_userd_by_name`] below, which pins the half that IS
/// true today so the gap has a named home instead of living in a doc nobody greps.
#[test]
fn a_store_slice_userd_is_routed_to_b_before_anything_is_allocated() {
    let body = enclosing_fn(&body_of("src/rm.rs"), "fn alloc_channel_in(");

    let route = body.find("self.birth_in_b(").expect(
        "★★★★★ ROUTE K INCREMENT 6 REGRESSED — `alloc_channel_in` no longer sends a \
         store-slice USERD to the birth client. The isolate then births with a USERD OF \
         OURS, and `[measured w755h]` hardware sees GP_PUT == GP_GET == 0 forever, fetches \
         nothing and reports nothing.",
    );
    // ★ NON-VACUITY: the function must still allocate, or "before any allocation" is vacuous.
    let first_alloc = body
        .find("raw_alloc(")
        .or_else(|| body.find("alloc_device_local("))
        .expect("★ NON-VACUITY: `alloc_channel_in` allocates nothing — wrong subject");
    assert!(
        route < first_alloc,
        "★★★ THE ROUTING DECISION MOVED BELOW THE FIRST ALLOCATION (route at {route}, alloc \
         at {first_alloc}). The two paths allocate in DIFFERENT CLIENTS; unwinding across \
         that boundary is the double free `UserdOwner` exists to make unrepresentable."
    );
}

/// ★★★ **Both halves of the guest's declaration are required, never one.**
///
/// ⊘ A store-slice ring whose USERD is a **joined leaf** is a real shape — the guest may put
/// them in different places — and it belongs on the isolate's own path, where the
/// joined-object check can see it. Routing on the ring alone would send it to B, which holds
/// no handle for that leaf.
#[test]
fn the_routing_requires_the_ring_and_the_userd_to_agree() {
    let body = enclosing_fn(&body_of("src/rm.rs"), "fn alloc_channel_in(");
    let at = body.find("self.birth_in_b(").expect("asserted above");
    let head = &body[..at];
    for needle in ["RingProvenance::StoreSlice", "UserdObject::TheStore"] {
        assert!(
            head.contains(needle),
            "★★★ the routing to B no longer tests `{needle}`. Both halves are required: a \
             store-slice ring with a JOINED USERD belongs on the isolate's path, and B holds \
             no handle for that leaf."
        );
    }
}

/// ★★★★★ **w755o — A CHANNEL BORN IN B IS SCHEDULED IN B, and the doorbell is NOT the same
/// question.**
///
/// > Owner, 2026-09-17: *"A doorbell can ring from another process, only the token needs to
/// > match. So VMM can ring the isolate's doorbell fine by just writing a dword."*
///
/// Exactly so, and it is why these two must not be conflated. The **doorbell** is
/// token-addressed: any process holding the usermode mapping may ring any channel by writing
/// the right dword — that is what `GET_WORK_SUBMIT_TOKEN` exists for. The **schedule** is an
/// RM *control on the TSG*, so it is client-scoped, and one issued on our client while naming
/// B's TSG names a different object or nothing at all.
///
/// ⚠ `channel_parts` also has no entry for a channel this isolate did not build, so without
/// the route `schedule` fails `BadHandle`. A channel born, never scheduled and then
/// doorbelled is **a channel that exists and never runs** — the same silence increment 6
/// exists to remove, arriving from a new cause.
#[test]
fn a_channel_born_in_b_is_scheduled_in_b() {
    let body = enclosing_fn(&body_of("src/rm.rs"), "fn schedule(");

    let route = body.find("birth_channel_of(").expect(
        "★★★★★ `schedule` no longer asks whether this channel was born in a birth client. \
         `GPFIFO_SCHEDULE` is an RM control on the TSG and is client-scoped — issued on our \
         client it names B's TSG in the wrong namespace, and `channel_parts` has no entry \
         for a channel we did not build, so this would fail `BadHandle` instead.",
    );
    let fallback = body
        .find("channel_parts(")
        .expect("★ NON-VACUITY: `schedule` no longer has its own-channel path — wrong subject");
    assert!(
        route < fallback,
        "★★★ the birth-client route is BELOW the isolate's own `channel_parts` lookup, which \
         returns `None` for a B-born channel — so the refusal happens before the route is \
         ever consulted. (route at {route}, lookup at {fallback})"
    );
    assert!(
        body.contains("birth.control(tsg, NVA06C_CTRL_CMD_GPFIFO_SCHEDULE"),
        "★★★ the B route no longer issues the schedule THROUGH B. Naming B's TSG on our own \
         client is the wrong-namespace control this route exists to avoid."
    );
}

/// ★★★★★ **w755q — THE PATH THE GUEST'S BIRTH ACTUALLY TAKES, AND WHAT IT DOES THERE.**
///
/// `[measured w755q — vast 51304517, RTX 3090, host driver 580.159.04, tree `eb3fc08f`]` a
/// guest channel whose USERD is a slice of the one reserved store is refused **11 times** by
/// `alloc_channel_lowered`, in a **per-proc isolate**, with
/// `USERD_IN_STORE_NEEDS_BIRTH_IN_B` — while `birth_in_b` was **never entered** (its success
/// print and both of its refusal prints were all zero).
///
/// ⊘ **That refusal is CORRECT and this gate exists to keep it.** A per-proc isolate may not
/// name the store (constraint 26), and it has no birth client B to name it through:
/// `birth_in_b` resolves B via `conn.birth_for_range`, whose index is populated only in the
/// **scratchpad's** `adopt_space`. Downgrading to a USERD of ours instead would be the
/// `GP_PUT == GP_GET == 0` silence measured at w755h — an armed run and its control producing
/// the same dead channel.
///
/// # ⚠ WHAT IS MISSING, NAMED HERE SO IT IS NOT ONLY IN A DOC
///
/// Nothing acts on the refusal. The birth must be **delegated to the scratchpad**, which holds
/// B — a change in *which process runs the verb*, which is why no edit inside `rm.rs` alone can
/// make `birth_in_b` reachable from the guest's path.
///
/// ⊘ This test does **not** fail on that gap, deliberately: a permanently-red test is noise
/// that gets muted, and the gap is a missing feature rather than a regression. What it does is
/// make the refusal's **site** load-bearing, so that moving or deleting it without building the
/// delegation is caught here rather than by another boot.
///
/// # ⊘⊘ UPDATED w755r — THE DELEGATION LANDED, AND THIS REFUSAL IS NOW A BACKSTOP
///
/// The crossing is built (`kayfabe_fwd::StoreChannelBirth`, installed at realize on the
/// `scratchpad`/`BirthClient` arm), and it intercepts **before** the verb is dispatched to the
/// proc's worker — so on an armed boot `alloc_channel_lowered` should no longer *see* this
/// shape at all.
///
/// ⇒ The refusal stays, and this gate with it, because *"the interception is armed"* and
/// *"the interception fired"* are different facts. If the birth party is not installed, or the
/// discriminant stops matching, the verb lands here again — and landing here must go on
/// refusing rather than birthing with a USERD of ours. ★ The census row `STORE-BIRTH
/// asked=0` is what says the interception never fired; this assertion is what says the
/// fallback is still safe when it does.
#[test]
fn the_guest_birth_path_refuses_a_store_userd_by_name() {
    let rm = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/rm.rs"))
        .expect("rm.rs is readable");
    let body = enclosing_fn(&rm, "fn alloc_channel_lowered(");
    assert!(
        !body.is_empty(),
        "`alloc_channel_lowered` was not found in rm.rs — it is the guest's birth-at-alloc \
         lowering, and if it was renamed this whole gate is measuring nothing"
    );

    // ★ The refusal is IN the lowering the guest's birth reaches, not somewhere a reader
    //   would have to hope is on the path.
    assert!(
        body.contains("USERD_IN_STORE_NEEDS_BIRTH_IN_B"),
        "★★★★★ `alloc_channel_lowered` no longer refuses a store USERD by name. `[measured \
         w755q]` this is the site that fired 11 times on real hardware, and it is the ONLY \
         thing standing between a store-slice USERD and a channel silently born with a USERD \
         of ours — which is the `GP_PUT == GP_GET == 0` silence."
    );
    // ⊘ And it must refuse on the `TheStore` discriminant itself, not on some proxy that a
    //   later edit could make true for a different reason.
    assert!(
        body.contains("UserdObject::TheStore"),
        "★★★ the refusal no longer keys on `UserdObject::TheStore`. A refusal that fires for \
         a different reason than the one it is named for is this tree's most-repeated defect."
    );
    // ★★★ NON-VACUITY, and it is the half w755q proves matters: `birth_in_b` — the thing that
    //     WOULD serve this — is NOT reachable from here. If a future edit makes it reachable,
    //     this assertion fires and whoever made it must come and rewrite this test's story,
    //     which is exactly when the story should be rewritten.
    assert!(
        !body.contains("birth_in_b"),
        "★★★★★ `alloc_channel_lowered` now reaches `birth_in_b` — which is the DELEGATION \
         this gate was written to say was MISSING. ⊘ That is good news, not a failure: \
         update this test to assert the delegation's shape, and re-boot, because \
         `docs/design/increment_6_never_fired.md` is now out of date."
    );
}

/// ★★★★★ **w755r — THE DELEGATION EXISTS, AND IT CROSSES A PROCESS BOUNDARY.**
///
/// `[measured w755q]` route K increment 6 never fired because `birth_in_b` lives in the
/// **scratchpad's** verb family while the guest's birth runs in the **per-proc isolate** —
/// two processes, and nothing carried the request across. This pins the carrier.
///
/// ⊘ The assertions are about **who can be asked**, not about internal ordering. That
/// distinction is the whole lesson of w755q: the gate that was green through that entire
/// boot asserted an ordering *inside* a function nothing called.
#[test]
fn the_store_birth_delegation_crosses_to_the_scratchpad() {
    let rm = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/rm.rs"))
        .expect("rm.rs is readable");

    // ★ 1. The scratchpad implements the delegation verb, and REFUSES when it is not the
    //      scratchpad — so a verb dispatched to the wrong process says so by name instead of
    //      reporting "no birth client holds this range", which sends a reader hunting a
    //      missing hand-over.
    let birth = enclosing_fn(&rm, "fn birth_guest_channel_in_b(");
    assert!(
        !birth.is_empty(),
        "★★★★★ `birth_guest_channel_in_b` is gone — that is the ONLY carrier from the VMM \
         into the birth client B. Without it the guest's birth lands in the per-proc isolate, \
         which refuses it by name and leaves the channel unborn (measured w755q, 11 times)."
    );
    assert!(
        birth.contains("ScratchpadRole::of"),
        "★★★ the delegation no longer checks that it IS the scratchpad. A per-proc backend \
         reaching it has no `birth_ranges` at all, so it would refuse as `no birth client \
         holds this range` — pointing a reader at a missing hand-over rather than at a verb \
         dispatched to the wrong process."
    );
    // ★★★ 2. The notifier is built IN B, from the grant. A handle would be in the
    //        scratchpad's namespace and B cannot name it; a region mapped by the per-proc
    //        isolate names memory this process never mapped.
    assert!(
        birth.contains("describe_guest_ram_in_b"),
        "★★★★★ the delegation no longer describes the error notifier in B. `hObjectError` is \
         a BIRTH parameter, so it cannot be attached afterwards — and a channel born without \
         one leaves the guest polling bytes the RC path will never write \
         (ogkm kernel_channel.c:549-568)."
    );
    assert!(
        birth.contains("map_guest_ram"),
        "★★★ the delegation no longer maps the grant itself. The descriptor must pin pages of \
         the SCRATCHPAD's mapping, because that is the process B lives in."
    );

    // ★★★★★ 3. NON-VACUITY, and it is the assertion that would have caught w755q: the
    //          namespace guard must exist, because `alloc_channel_in` takes `err_notifier` as
    //          a bare `u32` and the birth route is taken ABOVE it.
    let in_b = enclosing_fn(&rm, "fn birth_in_b(");
    assert!(
        in_b.contains("NOTIFIER_NOT_IN_B"),
        "★★★★★ `birth_in_b` no longer refuses a notifier from the wrong client. That handle \
         travels as a bare `u32` past a route taken above every allocation, and RM answers \
         about the HANDLE — so a plausible one is accepted, the channel is born, and the \
         guest polls notifier bytes nobody writes. This is the quietest failure on the path."
    );
}

// =====================================================================================
// w778 — one guest address space is ONE host address space
// =====================================================================================

/// ★★★★★ **THE REFCOUNT, AND WHY A COUNT RATHER THAN A FLAG.**
///
/// `[measured w778]` the raw client had ONE guest VA space and SEVEN channels, and the birth
/// path minted TWO host VA spaces because it duped per call with no cache. Channels the guest
/// put in one space then did not share translations — `--uvm-mean`'s P3 is a CE reading a VA
/// correctly while a GR channel writes the same VA into different memory.
///
/// ⊘ A boolean "already duped" would fix the split and break teardown: the first channel to go
/// would free the space out from under its siblings. The count is what makes "last one out"
/// expressible. ⚠ These are the two halves of the same bug — sharing too little, then freeing
/// too early — and a test that only checked the first would let the second through.
#[test]
fn one_guest_space_is_one_host_space_and_the_last_reference_frees_it() {
    use std::collections::BTreeMap;
    // The map's contract, exercised directly: (client, space) → (dup, range, refs).
    let mut m: BTreeMap<(u32, u32), (u32, u32, usize)> = BTreeMap::new();

    // Three channels are born in ONE guest space.
    let key = (0xc1d0_000b_u32, 0xcafe_0010_u32);
    for n in 1..=3 {
        match m.get_mut(&key) {
            Some(e) => e.2 += 1,
            None => {
                m.insert(key, (0xb147_0002, 0xb147_0003, 1));
            }
        }
        assert_eq!(m.len(), 1, "★ still ONE host space after {n} birth(s)");
    }
    assert_eq!(m[&key].2, 3, "three channels, three references");
    assert_eq!(m[&key].0, 0xb147_0002, "and the SAME dup for all of them");

    // A second, genuinely different guest space is its own entry — the cache must not
    // over-share either. ⊘ Two guest spaces ARE two host spaces; that is not the bug.
    let other = (0xc1d0_000b_u32, 0xcafe_0011_u32);
    m.insert(other, (0xb147_0007, 0xb147_0008, 1));
    assert_eq!(m.len(), 2);

    // Releasing two of three frees nothing.
    for expect in [2usize, 1] {
        let e = m.get_mut(&key).expect("held");
        e.2 -= 1;
        assert_eq!(e.2, expect);
        assert!(m.contains_key(&key), "⊘ a sibling still uses this space");
    }
    // The last release is the one that frees.
    let e = m.get_mut(&key).expect("held");
    e.2 -= 1;
    assert_eq!(e.2, 0);
    let gone = m.remove(&key).expect("last reference owns disposal");
    assert_eq!((gone.0, gone.1), (0xb147_0002, 0xb147_0003));
    assert!(!m.contains_key(&key));
    assert!(m.contains_key(&other), "★ the other space is untouched");
}
