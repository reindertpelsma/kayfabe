//! ★★★★★ **w755c — THE SIZE PROBE MUST NOT SPEAK FOR THE REAL RESERVATION.**
//!
//! `HostRmBackend::largest_reservable_mb` finds the largest reservable size by **really
//! allocating and freeing**, roughly thirteen times. The real store is then reserved once, at
//! the size it found.
//!
//! ⊘⊘ Both used to go through one verb. With the w755c opportunistic reservation that verb
//! also sets [`STORE_IS_CONTIGUOUS_AND_ALIGNED`] — the flag the store-slice page-size decision
//! reads — and prints a headline naming a reservation. So every throwaway probe step would
//! have overwritten the flag and claimed a store that does not exist yet.
//!
//! ⚠ **It would usually have been right**, because the last probe step is usually the size the
//! real reservation then uses. "Usually right by accident of ordering" is exactly
//! `correct_by_accident_under_a_temporary_condition`, which this campaign recorded five times
//! in two days. ⇒ split into two verbs, and gated here.

fn rm_src() -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/rm.rs"))
        .expect("rm.rs is where this gate says it is")
}

/// The body of `largest_reservable_mb`, delimited by the `fn`'s own indentation.
fn largest_reservable_body(src: &str) -> String {
    // ⊘⊘ **THERE ARE TWO `largest_reservable_mb`, and the FIRST is a one-line trait forwarder
    // (`Ok(HostRmBackend::largest_reservable_mb(self, start_mb))`).** The first draft of this
    // gate took `src.find(..)` and delimited that one — a body with no reservations at all —
    // and its own non-vacuity assert is what caught it. ⇒ pick the occurrence that actually
    // bisects, and assert exactly one does, so a future third copy is a failure rather than a
    // coin toss.
    let bodies: Vec<usize> = src
        .match_indices("fn largest_reservable_mb")
        .map(|(at, _)| at)
        .filter(|at| src[*at..(*at + 3000).min(src.len())].contains("reserve_gpga"))
        .collect();
    assert_eq!(
        bodies.len(),
        1,
        "★ NON-VACUITY: {} definitions of `largest_reservable_mb` reserve anything — this          gate cannot tell which one it is supposed to be reading",
        bodies.len()
    );
    let at = bodies[0];
    let line_start = src[..at].rfind('\n').map_or(0, |nl| nl + 1);
    let col = src[line_start..].chars().take_while(|c| *c == ' ').count();
    let terminator = format!("\n{}}}\n", " ".repeat(col));
    // ⊘ A missing terminator is a FAILURE, not a fallback to the rest of the file — that
    // fallback is what made a sibling gate green through a deleted assertion.
    let end = src[at..].find(&terminator).unwrap_or_else(|| {
        panic!("★ NON-VACUITY: could not delimit `largest_reservable_mb` (column {col})")
    });
    src[at..at + end].to_string()
}

#[test]
fn the_size_probe_reserves_through_the_probe_verb_only() {
    let src = rm_src();
    let body = largest_reservable_body(&src);

    // ★ NON-VACUITY: it must actually reserve something, or the rule below is about nothing.
    let probes = body.matches("reserve_gpga_probe(").count();
    assert!(
        probes >= 2,
        "★ NON-VACUITY: the bisection makes {probes} probe reservations; it is supposed to \
         halve to a floor and then bisect, so fewer than two means this gate is reading the \
         wrong function"
    );

    // ⊘ Matched on the spelling that is NOT the probe verb: `reserve_gpga(` would also match
    // `reserve_gpga_probe(`'s prefix, so the count is taken by difference.
    let all = body.matches("reserve_gpga").count();
    assert_eq!(
        all, probes,
        "★★★ THE SIZE PROBE IS RESERVING THROUGH THE REAL VERB. Every throwaway bisection \
         step would then set `STORE_IS_CONTIGUOUS_AND_ALIGNED` — the flag the store-slice \
         page-size decision reads — and print a headline naming a store that does not exist \
         yet. It would usually be right, because the last probe step is usually the real \
         size, which is what makes it dangerous rather than obvious."
    );
}

/// ★★★ **Only the real verb may touch the flag, and it must touch it on BOTH paths.**
///
/// ⊘ A verb that set it only on success would leave a stale `true` from a previous boot's
/// contiguous reservation after a fallback — and the page-size decision would then omit the
/// 4 KiB pin on a noncontiguous store, which is the exact configuration the pin exists for.
#[test]
fn the_flag_is_set_on_both_reservation_paths_and_nowhere_else() {
    let src = rm_src();
    let writes = src
        .matches("STORE_IS_CONTIGUOUS_AND_ALIGNED.store(")
        .count();
    assert_eq!(
        writes, 2,
        "expected exactly two writes to the reservation-shape flag (contiguous success and \
         noncontiguous fallback), found {writes} — one path is not recording its outcome, and \
         a stale value is read as a fact about this boot's store"
    );
    assert!(
        src.contains("STORE_IS_CONTIGUOUS_AND_ALIGNED.store(true"),
        "the contiguous path does not record success"
    );
    assert!(
        src.contains("STORE_IS_CONTIGUOUS_AND_ALIGNED.store(false"),
        "the fallback does not record that it fell back"
    );
    // ★ And the probe verb must not be able to reach either write.
    let body = largest_reservable_body(&src);
    assert!(
        !body.contains("STORE_IS_CONTIGUOUS_AND_ALIGNED"),
        "the size probe writes the reservation-shape flag directly"
    );
}
