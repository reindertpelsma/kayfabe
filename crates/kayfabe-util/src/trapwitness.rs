//! ★★★★★ **The per-thread GUEST-TRAP witness, and the token that cannot be obtained
//! beneath it** — the mechanism clauses **(a)** and **(b)** of `INLINE-SAFE` never had
//! (`docs/design/blocking_and_completion_model.md` §1, §4, §6; task #261).
//!
//! # 0. What was missing, in the doc's own words
//!
//! > *"⊘ **Clauses (a) and (b) have no mechanism at all.** (c) is getting one via `w300`.
//! > (a) and (b) are currently **prose in this file**, which by this repo's own history
//! > means they will be violated by a well-meaning patch."*
//!
//! Three measured instances of exactly that, all after the prose was written:
//! **w317** — a 3.70 s teardown disposal on the vCPU thread under the BQL;
//! **w319** — 13 313 serialized cross-process round trips on one drain;
//! **w306** — an isolate call reachable only from the vCPU trap path.
//!
//! # 1. ⊘ THE RULING THIS MODULE OVERTURNS, AND EXACTLY HOW FAR
//!
//! `kayfabe_core::channel_kind::TrapContract` states, correctly for what it had:
//!
//! > *"⇒ **Rust cannot express *"this call is not on the vCPU thread"*.** Thread identity
//! > is not in any type here …"*
//!
//! ★ **That is true of a TYPE ALONE and false of a type composed with a per-thread
//! witness.** The composition is three facts, and no two of them suffice:
//!
//! | carried by | fact |
//! |---|---|
//! | the private field + no public struct literal | **this token was MINTED by the constructor**, not fabricated |
//! | [`OffTrap::claim`]'s check of [`in_trap`] | the minting thread was **not inside a guest trap** at that instant |
//! | `!Send` + `!Sync` (a `PhantomData<*mut ()>`) | it is **still on the thread that minted it** |
//!
//! ⇒ holding an [`OffTrap`] means *"the thread executing this line was off-trap when it
//! asked, and is the same thread"*. That is thread identity, expressed.
//!
//! ⊘ **It is still not absolute, and the residue has a name rather than a footnote:**
//! [`OffTrap::inline_under_bql`] mints one **on** a trap thread. That is deliberate — see
//! §3 — and it is the single hole, it is **counted**, and the count is the content of
//! `INLINE-SAFE` clause (b): *what is still allowed to run inline, and is it bounded.*
//!
//! # 2. ★★ THE TREE HAS ALREADY RULED AGAINST A TOKEN MINTED EARLY — and this obeys it
//!
//! [`crate::lock::BlockingSection`]'s own doc: *"a capability minted while lock-free must
//! not launder a later acquisition past the invariant"*. The same hazard exists here in a
//! different axis: a token minted off-trap must not launder a **later** trap entry.
//!
//! Two answers, both required:
//! - `!Send`/`!Sync` stops the token **crossing** to a trap thread at all;
//! - [`OffTrap::still_off_trap`] **re-asserts at the verb**, not at the mint — so a token
//!   held across a re-entrant trap on its own thread (a nested MMIO dispatch) panics at the
//!   host verb, exactly where `assert_lock_free` panics.
//!
//! # 3. ⊘ WHY THERE IS A LEGAL WAY TO MINT ONE ON A TRAP THREAD
//!
//! Because a rule with no exception is a rule that gets deleted. The tree's own precedent
//! is one module over: [`crate::lockwitness::assert_lock_free`] is the rule and
//! [`crate::lockwitness::assert_only_ranks`] is the **enumerated** exception, and the
//! exception exists so a genuinely-bounded inline site can be *declared* rather than have
//! the whole gate switched off around it.
//!
//! ⇒ [`OffTrap::inline_under_bql`] is that exception. It takes a `&'static str` reason, it
//! bumps [`inline_exceptions`], and the census test in the crate that uses it pins how many
//! call sites exist. **A boot prints the ratio.** The target state of this campaign is
//! `inline_exceptions == 0`, at which point the gate is absolute and the census says so.
//!
//! # 4. Purely generic (decision #2)
//!
//! Nothing here names a GPU, a doorbell, a VMM or a driver. It is a per-thread boolean and
//! a token, exactly as generic as [`crate::lockwitness`]'s bit mask.
//!
//! # 5. ⚠ WHAT THIS CANNOT SEE — read before trusting a green build
//!
//! - **It sees traps that INSTALL a [`TrapGuard`].** A guest entry path that forgets to is
//!   invisible, and every call beneath it will mint a clean [`OffTrap`]. The install sites
//!   are counted by a census for that reason — a type cannot see a caller that never names
//!   it (`kayfabe-rt/tests/compile_fail.rs`' own statement of the ceiling).
//! - **It says nothing about DURATION.** Clause (b) is a bound, and this module only
//!   supplies the *predicate* it attaches to. [`TrapGuard`] therefore also records the
//!   longest hold it has ever seen, so "how long may the residue run" is answerable from
//!   the same instrument rather than from a second one that can disagree.
//! - **It is not a lock-order check.** Clause (c) is [`crate::lockwitness`]'s, and the two
//!   are complements: (c) asks *what do I hold*, this asks *where am I*.

use std::cell::Cell;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};

thread_local! {
    /// Nesting depth of guest-trap dispatches on THIS thread. A depth rather than a
    /// boolean: a trap handler that re-enters the dispatcher (an MMIO write whose side
    /// effect dispatches another) must not clear the flag on the inner guard's `Drop`.
    static TRAP_DEPTH: Cell<u32> = const { Cell::new(0) };
    /// Cumulative trap entries on THIS thread — instrumentation, and the known-positive a
    /// census needs: `0` means the guard was never installed, which is a different fact
    /// from "no trap happened".
    static TRAP_ENTRIES: Cell<u64> = const { Cell::new(0) };
}

/// Tokens minted by [`OffTrap::claim`] — i.e. host work that ran off the trap thread.
static OFF_TRAP_CLAIMS: AtomicU64 = AtomicU64::new(0);
/// Tokens minted by [`OffTrap::inline_under_bql`] — **the residue**, and the number this
/// campaign is driving to zero.
static INLINE_EXCEPTIONS: AtomicU64 = AtomicU64::new(0);
/// The longest trap dispatch this process has observed, in microseconds. ⊘ Whole-process
/// and monotonic: it answers *"has clause (b) ever been at risk"*, never *"what is the
/// current hold"*.
static WORST_TRAP_US: AtomicU64 = AtomicU64::new(0);

/// Is THIS thread currently inside a guest-trap dispatch?
///
/// ⊘ *This thread*. A `false` here says nothing about any other vCPU — which is the point:
/// the question a host verb needs answered is about its own stack, not about the machine.
#[must_use]
pub fn in_trap() -> bool {
    TRAP_DEPTH.with(Cell::get) > 0
}

/// How deeply nested this thread's trap dispatches are (0 when off-trap).
#[must_use]
pub fn trap_depth() -> u32 {
    TRAP_DEPTH.with(Cell::get)
}

/// Cumulative trap entries on THIS thread. ★ The known-positive a census needs: a
/// `TrapGuard` that is never installed leaves this at `0` while everything else looks
/// armed.
#[must_use]
pub fn trap_entries() -> u64 {
    TRAP_ENTRIES.with(Cell::get)
}

/// How many [`OffTrap`]s were claimed honestly (off a trap thread), process-wide.
#[must_use]
pub fn off_trap_claims() -> u64 {
    OFF_TRAP_CLAIMS.load(Ordering::Relaxed)
}

/// ★★★ How many [`OffTrap`]s were minted by the **enumerated inline exception**,
/// process-wide — the residue clause (b) is about. **Target: 0.**
#[must_use]
pub fn inline_exceptions() -> u64 {
    INLINE_EXCEPTIONS.load(Ordering::Relaxed)
}

/// The longest single trap dispatch observed by any thread, in microseconds. `0` means
/// **UNMEASURED** — no guard has closed yet — and never "instantaneous".
#[must_use]
pub fn worst_trap_us() -> u64 {
    WORST_TRAP_US.load(Ordering::Relaxed)
}

/// One line naming every number this module owns, for a boot log.
///
/// ⊘ Prints `worst_trap_us` with an explicit UNMEASURED arm: an absent measurement and a
/// zero one are different facts, and this tree has paid for reading one as the other.
#[must_use]
pub fn census() -> String {
    let worst = worst_trap_us();
    format!(
        "TRAPWITNESS off_trap_claims={} inline_exceptions={} worst_trap={} (target: \
         inline_exceptions=0)",
        off_trap_claims(),
        inline_exceptions(),
        if worst == 0 {
            "UNMEASURED (no guard has closed)".to_string()
        } else {
            format!("{worst}us")
        },
    )
}

/// ★★★ **The RAII marker installed at every guest-trap entry.**
///
/// Install it at the *outermost* boundary the guest can cross — the MMIO dispatch — and
/// nowhere else. One per entry: nesting is counted, so an inner guard's `Drop` cannot
/// un-mark an outer trap.
///
/// ⊘ Not `Send`: a guard is a statement about the stack it sits on.
#[derive(Debug)]
pub struct TrapGuard {
    start: std::time::Instant,
    _not_send: PhantomData<*mut ()>,
}

impl TrapGuard {
    /// Mark this thread as executing a guest trap until the guard drops.
    #[must_use]
    pub fn enter() -> Self {
        TRAP_DEPTH.with(|d| d.set(d.get() + 1));
        TRAP_ENTRIES.with(|c| c.set(c.get() + 1));
        Self {
            start: std::time::Instant::now(),
            _not_send: PhantomData,
        }
    }
}

impl Drop for TrapGuard {
    fn drop(&mut self) {
        // ⊘ Only the OUTERMOST guard publishes a duration: an inner one measures a
        // sub-interval, and reporting it as "a trap hold" would understate the worst case
        // by exactly the nesting.
        let depth = TRAP_DEPTH.with(|d| {
            let n = d.get().saturating_sub(1);
            d.set(n);
            n
        });
        if depth == 0 {
            let us = u64::try_from(self.start.elapsed().as_micros()).unwrap_or(u64::MAX);
            WORST_TRAP_US.fetch_max(us, Ordering::Relaxed);
        }
    }
}

/// ★★★★★ **The witness that the bearer is NOT executing inside a guest trap.**
///
/// # The teeth, and they are three separate things
///
/// 1. The field is **private** and there is no public struct literal, so an
///    [`OffTrap`] cannot be *named* into existence outside this module. Pinned by
///    `tests/ui/name_an_off_trap.rs`.
/// 2. [`OffTrap::claim`] **panics** when [`in_trap`] holds, so it cannot be *obtained* on a
///    trap thread. Pinned by a runtime known-positive.
/// 3. It is **`!Send` and `!Sync`**, so one obtained on a worker cannot be *carried* to a
///    vCPU. Pinned by `tests/ui/send_an_off_trap.rs`.
///
/// ⇒ A function that takes `&OffTrap` **cannot be called from a guest trap** except through
/// [`OffTrap::inline_under_bql`], which says so in its own name and counts itself.
///
/// # ⊘ What it does NOT prove
///
/// *"A caller that never names it"* — the ceiling `kayfabe-rt/tests/compile_fail.rs`
/// already states. If a host verb exists that does not take one, this type has nothing to
/// say about it. That is why the gate is placed at **one door**
/// (`kayfabe_isolate::Worker::execute`, the single entry to a host RM verb) rather than
/// sprinkled: one signature quantifies over every call site in the workspace, including
/// ones written tomorrow.
#[derive(Debug)]
pub struct OffTrap {
    /// The reason the bearer gave, for a panic message.
    what: &'static str,
    /// ★ Whether this token came from the **enumerated exception** rather than an honest
    /// claim, so a consumer can print which and a census can refuse to conflate them.
    inline: bool,
    /// `!Send` + `!Sync` — the token is a statement about **this thread**, and a statement
    /// that can be posted to another thread is not one.
    _not_send: PhantomData<*mut ()>,
}

impl OffTrap {
    /// ★ **Claim the witness.** The ordinary door.
    ///
    /// `what` names the operation about to run, so a violation's message says which
    /// blocking thing was about to happen inside a guest trap.
    ///
    /// # Panics
    /// If this thread is currently inside a [`TrapGuard`]. That is not a recoverable
    /// condition: it means a host round trip was about to run with the BQL held, freezing
    /// every vCPU and QEMU's main loop — the failure `blocking_and_completion_model.md` §0
    /// exists to describe.
    #[must_use]
    pub fn claim(what: &'static str) -> Self {
        assert!(
            !in_trap(),
            "INLINE-SAFE violation (blocking_and_completion_model.md §1 clause (a)/(b)): \
             `{what}` asked for an off-trap witness while this thread is {depth} guest-trap \
             dispatch(es) deep. Every guest MMIO write arrives with the QEMU BQL held, so \
             this call would stall EVERY vCPU and QEMU's main loop, not just the ringing \
             one. Move the work to a worker and return to VM entry, or — if and only if it \
             is bounded and guest-independent — declare it with \
             `OffTrap::inline_under_bql`, which counts itself.",
            depth = trap_depth(),
        );
        OFF_TRAP_CLAIMS.fetch_add(1, Ordering::Relaxed);
        Self {
            what,
            inline: false,
            _not_send: PhantomData,
        }
    }

    /// ⚠ **THE ENUMERATED EXCEPTION** — mint a witness *on* a trap thread, declaring that
    /// the work beneath it is bounded and guest-independent.
    ///
    /// This is [`crate::lockwitness::assert_only_ranks`]'s role one axis over: the rule is
    /// [`OffTrap::claim`], and a genuinely-bounded inline site is **declared** rather than
    /// having the gate switched off around it.
    ///
    /// ⊘ It bumps [`inline_exceptions`], and the census in the consuming crate pins how
    /// many call sites may exist. **Adding one is a design decision with a number attached,
    /// which is the whole difference between this and a comment.**
    #[must_use]
    pub fn inline_under_bql(what: &'static str) -> Self {
        INLINE_EXCEPTIONS.fetch_add(1, Ordering::Relaxed);
        Self {
            what,
            inline: true,
            _not_send: PhantomData,
        }
    }

    /// ★★★★★ **THE ONE HELPER THE PRODUCTION HOST-VERB PATH USES — and the number this
    /// campaign is driving to zero rides on it.**
    ///
    /// It asks [`in_trap`] and takes the honest branch either way:
    ///
    /// | where the caller actually is | branch | counter |
    /// |---|---|---|
    /// | on a worker (publication deferred) | [`OffTrap::claim`] | [`off_trap_claims`] |
    /// | on a vCPU inside a guest trap | [`OffTrap::inline_under_bql`] | [`inline_exceptions`] |
    ///
    /// # ⊘ Why this is NOT `a_fallback_keyed_on_our_own_ignorance`
    ///
    /// That shape is *"we could not tell, so we assumed"*. This is the opposite: it is keyed
    /// on a fact we **measure directly on this thread**, and it **reports which branch it
    /// took** rather than folding both into one word. A boot that ends with
    /// `inline_exceptions=0` has proved the whole host-verb path ran off the trap; one that
    /// ends non-zero says exactly how many did not.
    ///
    /// # ⚠ THE CEILING, stated where it can be read rather than inferred
    ///
    /// While this helper exists, the gate **cannot panic in production** — a trap-thread
    /// caller gets a declared exception instead of a refusal. What the gate buys is
    /// therefore the *`VerbPlan::gated_doorbell` upgrade*: **omission → commission.** A new
    /// host verb can no longer land on the trap path by nobody noticing; it lands by someone
    /// naming a site that a census counts. ⇒ The two instruments are complements — the type
    /// guards the boundary, `off_trap_census.rs` guards the mint — and the campaign's finish
    /// line is the census set going empty, at which point this helper can be deleted and
    /// [`OffTrap::claim`] becomes the only door.
    #[must_use]
    pub fn at_a_host_verb(what: &'static str) -> Self {
        if in_trap() {
            Self::inline_under_bql(what)
        } else {
            Self::claim(what)
        }
    }

    /// ★★★ **Re-assert at the verb, not at the mint** — [`crate::lock::BlockingSection`]'s
    /// ruling, obeyed: *"a capability minted while lock-free must not launder a later
    /// acquisition past the invariant."*
    ///
    /// Call this at the host-verb door, every time. It is a thread-local read; it costs far
    /// less than the round trip it guards.
    ///
    /// # Panics
    /// If the thread entered a trap **after** this token was minted (a re-entrant
    /// dispatch), unless the token is the enumerated exception.
    pub fn still_off_trap(&self, door: &str) {
        assert!(
            self.inline || !in_trap(),
            "INLINE-SAFE violation (blocking_and_completion_model.md §1): `{door}` was \
             reached with an OffTrap minted for `{what}`, but this thread has since entered \
             {depth} guest-trap dispatch(es). A witness minted off-trap must not launder a \
             LATER trap entry past the invariant (`BlockingSection`'s own ruling, one axis \
             over).",
            what = self.what,
            depth = trap_depth(),
        );
    }

    /// Was this token the enumerated inline exception rather than an honest claim?
    ///
    /// ★ Read by the boot log so a run can never report progress without saying which of
    /// its host verbs ran inline.
    #[must_use]
    pub fn is_inline_exception(&self) -> bool {
        self.inline
    }

    /// What the bearer said it was for.
    #[must_use]
    pub fn what(&self) -> &'static str {
        self.what
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The guard nests, and only the outermost close publishes a duration.
    #[test]
    fn the_guard_nests_and_only_the_outermost_close_clears_it() {
        assert!(!in_trap());
        let outer = TrapGuard::enter();
        assert!(in_trap());
        assert_eq!(trap_depth(), 1);
        {
            let _inner = TrapGuard::enter();
            assert_eq!(trap_depth(), 2);
        }
        assert!(in_trap(), "an inner guard's Drop must not un-mark the outer trap");
        drop(outer);
        assert!(!in_trap());
    }

    /// ★ The known-positive for the whole module: a claim inside a trap PANICS.
    #[test]
    fn a_witness_cannot_be_claimed_inside_a_guest_trap() {
        let g = TrapGuard::enter();
        let r = std::panic::catch_unwind(|| OffTrap::claim("a host RM verb"));
        drop(g);
        let e = r.expect_err("claiming an off-trap witness inside a trap must panic");
        let msg = e
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_else(|| "<non-string panic>".into());
        assert!(
            msg.contains("INLINE-SAFE violation"),
            "the panic must name the predicate it enforces, got: {msg}"
        );
    }

    /// …and off-trap it succeeds, so the gate is not vacuous.
    #[test]
    fn a_witness_is_claimable_off_trap() {
        assert!(!in_trap());
        let off = OffTrap::claim("a host RM verb");
        assert!(!off.is_inline_exception());
        off.still_off_trap("the verb door");
    }

    /// ★★ The enumerated exception mints inside a trap, says so, and counts itself.
    #[test]
    fn the_enumerated_exception_mints_inside_a_trap_and_counts_itself() {
        let before = inline_exceptions();
        let _g = TrapGuard::enter();
        let off = OffTrap::inline_under_bql("a bounded emulated copy");
        assert!(off.is_inline_exception());
        // ⊘ And it does NOT panic at the door — that is what "declared" buys.
        off.still_off_trap("the verb door");
        assert_eq!(inline_exceptions(), before + 1);
    }

    /// ★★★ The launder case `BlockingSection`'s ruling names: minted off-trap, carried into
    /// a trap on the same thread, refused AT THE DOOR rather than at the mint.
    #[test]
    fn a_witness_minted_off_trap_does_not_launder_a_later_trap_entry() {
        let off = OffTrap::claim("a host RM verb");
        off.still_off_trap("before any trap");
        let g = TrapGuard::enter();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            off.still_off_trap("the verb door")
        }));
        drop(g);
        assert!(
            r.is_err(),
            "a token held across a re-entrant trap must be refused at the door"
        );
    }

    /// The census names both numbers and refuses to print an unmeasured worst-hold as 0.
    #[test]
    fn the_census_names_the_residue_and_never_prints_unmeasured_as_zero() {
        let line = census();
        assert!(line.contains("inline_exceptions="), "{line}");
        assert!(line.contains("target: inline_exceptions=0"), "{line}");
        // Close a guard so the worst-hold is measured, then check the other arm.
        drop(TrapGuard::enter());
        assert!(!census().contains("UNMEASURED"), "a closed guard must publish a hold");
    }
}
