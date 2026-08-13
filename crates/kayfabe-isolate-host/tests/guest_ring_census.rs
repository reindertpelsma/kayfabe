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
        4,
        "★ The definition, the ONE use (the `RingSource::Ours` arm of `alloc_channel_in`, \
         which is the only place our own 64-entry ring is described), and two in the unit \
         test that checks it is a power of two and fits. ⊘ A fifth is a submission path \
         that went back to assuming every ring is ours — invisible on our own channels, \
         64-against-4096 wrong on the guest's.",
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
        4,
        "The definition, the `Ours` layout, `submit_entry`'s slot address, and the unit \
         test. ⊘ Every one of them is about OUR ring object's internal layout. The guest's \
         ring has its own, which is why `submit_entry` refuses a handed-in ring by name \
         before it computes an offset at all.",
    ),
    (
        "src/rm.rs",
        "alloc_device_local(RING_OBJECT_BYTES)",
        2,
        "★★★ G1, as a count: the ring (on the `Ours` arm ONLY) and USERD (on both arms, \
         because USERD is ours on every channel we allocate). A third is a ring allocated \
         for a channel that was handed one — the exact blocker this rung removed, growing \
         back.",
    ),
    (
        "src/rm.rs",
        "RingOwner::HandedIn",
        5,
        "The five places provenance decides something: the `Guest` arm's tag, the empty \
         unwind set, the absent CPU map, `submit_entry`'s refusal, and the teardown that \
         must not unmap or free the guest's ring.",
    ),
    (
        "src/rm.rs",
        "RING_NOT_OURS",
        4,
        "The named status, the two ring accessors that answer it, and `submit_entry`'s \
         early refusal. ⊘ These are the assertion that no CPU view exists — G4 stated as an \
         answer rather than as an omission.",
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
    assert_eq!(
        body.matches("RingSource::Guest(").count(),
        6,
        "`RingSource::Guest` is constructed or matched somewhere new. ⊘ SIX is the ruling \
         (three before leg B, five before w288) and each one is a different job: TWO \
         constructions — `alloc_channel_over_guest_ring` and its \
         `_with_error_notifier` twin — TWO arms in `alloc_channel_in` deciding the RING — one \
         provenance (allocate, or do not), one layout (our offsets, or the caller's) — and TWO \
         more deciding the USERD, in the same shape and for the same reason. ★ They are \
         deliberately not one arm each: the ring arms straddle a failure that must unwind \
         between them, and the USERD arms are a second axis entirely — a channel can adopt the \
         guest's ring and keep a USERD of ours, which is what every leg-A boot before this one \
         did. A seventh site means one of the two guest arms is reachable from a path that did \
         not state it."
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
    let fwd = sibling_body("kayfabe-fwd", "src/lib.rs");
    assert_eq!(
        fwd.matches("adopted_guest_ring(spine, proc, chan, cgpu)")
            .count(),
        1,
        "`adopted_guest_ring` is called from somewhere other than the single birth site in \
         `plan_engine_object`, or from nowhere. Exactly one call is what makes the isolate's \
         `adopt: None` readable as *the armed path ran and produced nothing*."
    );
    assert!(
        fwd.contains("adopt: if channel.is_none() {"),
        "The consult is no longer gated on `channel.is_none()` alone. ⊘ A second condition \
         here — a flag, a feature, an env read — would make `DECLINED` ambiguous again, which \
         is the exact state this rung exists to leave."
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
    assert_eq!(
        body.matches("RmError::Other(USERD_NOT_OURS)").count(),
        2,
        "the guest-USERD arm's two refusals (`userd_cursors`' GP_GET read and \
         `userd_store_u32`'s GP_PUT write) are no longer both by name. A skipped write and a \
         write to the wrong place look identical in a log."
    );
    // ⊘ The refusal must not be reachable from the witness: it is `fb_joins` membership.
    assert!(
        body.contains("t.is_joined_object(raw_memory)"),
        "the adoption arm's membership check moved; the refusal is now gated on something \
         other than `FbJoinTable` membership."
    );
}
