//! ★★★★★ **THE BOUNDED ARM-THEN-RETRY — one loop, three callers, no feature gate.**
//!
//! Cut B (w737) wrote this inside `barmirror`, whose whole module is gated behind
//! `host-isolates`. w740 gave it a **third** caller — the CeUtils doorbell path in `shim`,
//! which is not gated — so it moved here rather than being spelled a second time.
//!
//! ⊘ The move is the whole change: the function, its doc, and its five inline tests are
//! byte-identical to cut B's. `SINGLE_STORE_PLAN.md`'s own rule for this shape is that a
//! loop which can only be checked by booting is a loop nobody checks; the tests below are
//! why it lives in a file of its own and not at a call site.

/// ★★★★★ **CUT B — THE BOUNDED ARM-THEN-RETRY, AS A PURE FUNCTION.**
///
/// `attempt` produces a value and says whether it is **good**; `arm` says whether it changed
/// anything. The loop runs `attempt` once, then at most `retries` more times, and only while
/// `arm` returns `true`. Returns the last value and how many retries were spent.
///
/// # ⊘⊘⊘ Why this is a free function and not two loops at the two call sites
///
/// The two callers — `resolve_arming` and `premap_window` — need the same three properties and
/// **cannot be built in a `cargo test`**: a `BarMirror` needs a `QemuMachine`. A loop that can
/// only be checked by booting is a loop nobody checks. ⇒ the part that can be wrong on its own
/// lives here, where the inline tests below fire each property:
///
/// 1. **A FIXED TRIP COUNT** (`THE_CONSTRAINTS.md` §20 invariant 1). `attempt`'s input is a
///    guest-authored page-table pointer; a loop that ended when the walk succeeded would let
///    the guest's own tables choose how long this thread runs.
/// 2. **`arm` returning false ENDS IT.** *"Nothing was armed"* covers *"the drain declined"*
///    (a vCPU), *"the aperture refused"* and *"there was no port"*, and retrying helps in none
///    of them — ⚠ and the first is a **vCPU inside an MMIO exit**, which is the thread that
///    must not spin.
/// 3. **The value comes back either way.** A caller that got only `Err` on give-up could not
///    report what it last saw, and `window_leaves`' failing shape is an `Ok` that is SHORT.
pub(crate) fn arm_then_retry<T>(
    retries: u32,
    mut attempt: impl FnMut() -> (T, bool),
    mut arm: impl FnMut() -> bool,
) -> (T, u32) {
    let (mut value, mut good) = attempt();
    let mut used = 0u32;
    // ⊘ `for`, never `while !good`: the bound is the loop's own, not the data's.
    for _ in 0..retries {
        if good || !arm() {
            break;
        }
        used += 1;
        let (v, g) = attempt();
        value = v;
        good = g;
    }
    (value, used)
}

#[cfg(test)]
mod arm_then_retry_tests {
    //! ★★★★★ **CUT B's RETRY LOOP, and each of its three properties fired.**
    //!
    //! ⊘ These exist because the two production call sites cannot be built in a `cargo test` —
    //! a [`BarMirror`] needs a `QemuMachine` — so the part that can be wrong on its own was
    //! made a free function. ⚠ Every assertion here is a **known-positive**: each one fails if
    //! the loop stops doing the thing, rather than merely not crashing.

    use super::arm_then_retry;
    use std::cell::Cell;

    /// ★ **THE HAPPY PATH: it retries exactly as far as it has to, and no further.**
    #[test]
    fn it_stops_the_moment_the_attempt_is_good() {
        let n = Cell::new(0u32);
        let (v, used) = arm_then_retry(
            8,
            || {
                n.set(n.get() + 1);
                (n.get(), n.get() == 3)
            },
            || true,
        );
        assert_eq!(v, 3);
        assert_eq!(
            used, 2,
            "one initial attempt plus TWO retries, then it stops"
        );
        assert_eq!(n.get(), 3, "and the attempt ran three times, not eight");
    }

    /// ⊘⊘⊘ **THE FIXED TRIP COUNT — §20's first structural invariant, fired.**
    ///
    /// The attempt never succeeds and the arm always claims progress: a loop that ended on the
    /// data would run forever, and a guest's own page tables are what feed the data.
    #[test]
    fn an_attempt_that_never_succeeds_costs_exactly_the_bound() {
        let n = Cell::new(0u32);
        let (_, used) = arm_then_retry(
            5,
            || {
                n.set(n.get() + 1);
                ((), false)
            },
            || true,
        );
        assert_eq!(used, 5, "★ the bound, exactly — not one more");
        assert_eq!(
            n.get(),
            6,
            "one initial attempt plus five retries. ⚠ If this ever becomes unbounded the \
             symptom is a WEDGED BOOT with no error, because every iteration looks like work."
        );
    }

    /// ★★★★★ **CUT B ITEM 5 — AN ARM THAT DID NOTHING ENDS THE LOOP, AND IT ENDS IT AT ONCE.**
    ///
    /// ⊘ *"Nothing was armed"* covers three states — the drain **declined** (a vCPU inside an
    /// MMIO exit), the aperture **refused**, and there is **no port**. Retrying helps in none
    /// of them, and the first is on the one thread that must never spin.
    #[test]
    fn an_arm_that_changes_nothing_ends_the_loop_immediately() {
        let attempts = Cell::new(0u32);
        let arms = Cell::new(0u32);
        let (_, used) = arm_then_retry(
            16,
            || {
                attempts.set(attempts.get() + 1);
                ((), false)
            },
            || {
                arms.set(arms.get() + 1);
                false
            },
        );
        assert_eq!(used, 0);
        assert_eq!(
            attempts.get(),
            1,
            "the initial attempt, and nothing after it"
        );
        assert_eq!(
            arms.get(),
            1,
            "★★★ THE KNOWN-POSITIVE: the arm is asked ONCE. A loop that could not tell \
             `armed nothing` from `try again` would have asked sixteen times — on a vCPU, \
             sixteen IPC round trips inside one MMIO exit."
        );
    }

    /// ⊘ **THE LAST VALUE COMES BACK ON GIVE-UP.** `window_leaves`' failing shape is an `Ok`
    /// that is SHORT, so a caller that got only a refusal could not report what it saw — and
    /// premap would print nothing and publish nothing.
    #[test]
    fn the_value_survives_a_give_up() {
        let n = Cell::new(0u32);
        let (v, used) = arm_then_retry(
            2,
            || {
                n.set(n.get() + 1);
                (format!("attempt {}", n.get()), false)
            },
            || true,
        );
        assert_eq!(used, 2);
        assert_eq!(
            v, "attempt 3",
            "the LAST value, not the first and not a default"
        );
    }

    /// ⊘ A bound of zero is a caller that does not want a retry, and it must still attempt
    /// once. ⚠ Stated because `0` is the value a future gate would use to turn cut B off, and
    /// an off switch that skipped the attempt would break the arena arm.
    #[test]
    fn a_bound_of_zero_still_attempts_once() {
        let n = Cell::new(0u32);
        let (_, used) = arm_then_retry(
            0,
            || {
                n.set(n.get() + 1);
                ((), false)
            },
            || panic!("the arm must not be asked when no retry is allowed"),
        );
        assert_eq!(used, 0);
        assert_eq!(n.get(), 1);
    }
}
