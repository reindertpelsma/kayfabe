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
//! **w317** — a 3.70 s teardown disposal on the vCPU thread under the VMM's global lock;
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
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;

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

/// w481 — how many one-second buckets the phase profile keeps. 1024 s is longer than any
/// boot this bench runs, so in practice nothing wraps.
const PHASE_BUCKETS: usize = 1024;
/// The worst trap seen in each one-second bucket since the first trap.
static PHASE_WORST_US: [AtomicU64; PHASE_BUCKETS] =
    [const { AtomicU64::new(0) }; PHASE_BUCKETS];
/// How many traps exceeded [`SLOW_TRAP_US`] in each bucket.
static PHASE_SLOW: [AtomicU64; PHASE_BUCKETS] = [const { AtomicU64::new(0) }; PHASE_BUCKETS];
/// The highest bucket index reached, so the census knows where the profile ends.
static PHASE_HIGH_SEC: AtomicU64 = AtomicU64::new(0);

/// The instant the first trap closed — the origin of the phase profile. ⊘ Deliberately the
/// first TRAP and not process start: everything before the guest touches a register is
/// irrelevant to a trap profile and would only pad the front with empty buckets.
/// The wall clock at which the phase profile's origin was taken, as seconds since the Unix
/// epoch. ⊘ Recorded beside the monotonic origin rather than derived from it: a monotonic
/// instant cannot be compared with anything a harness wrote into a log.
fn trap_epoch_wall() -> u64 {
    static WALL: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *WALL.get_or_init(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    })
}

fn trap_epoch() -> &'static std::time::Instant {
    static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    EPOCH.get_or_init(|| {
        let _ = trap_epoch_wall();
        std::time::Instant::now()
    })
}

/// ★★★ **w481 — the phase profile, one line.** Worst trap per 10-second window, so the
/// device bring-up and the workload can be told apart at a glance.
///
/// ⊘ Prints the window INDEX with each figure. A bare series of numbers cannot be aligned
/// against anything the harness logged, and a profile nobody can locate in time is a shape
/// without a story.
#[must_use]
pub fn phase_profile() -> String {
    let high = PHASE_HIGH_SEC.load(Ordering::Relaxed) as usize;
    if high == 0 && PHASE_WORST_US[0].load(Ordering::Relaxed) == 0 {
        return "TRAP-PHASES none — no trap has closed".to_string();
    }
    // ⊘⊘ **SELF-LOCATING, w481b.** The first cut printed window indices relative to an
    // origin it never disclosed, so the profile could not be aligned against anything the
    // harness logged — the exact defect its own doc comment warned about. The origin's wall
    // clock and the current offset are now on the line, so any log timestamp can be mapped
    // into a window by subtraction.
    let mut out = format!(
        "TRAP-PHASES origin={} now=+{}s worst_us/slow per 10s window [",
        trap_epoch_wall(),
        trap_epoch().elapsed().as_secs(),
    );
    let mut w = 0usize;
    while w * 10 <= high {
        let (mut worst, mut slow) = (0u64, 0u64);
        for sec in w * 10..(w * 10 + 10).min(high + 1) {
            worst = worst.max(PHASE_WORST_US[sec % PHASE_BUCKETS].load(Ordering::Relaxed));
            slow += PHASE_SLOW[sec % PHASE_BUCKETS].load(Ordering::Relaxed);
        }
        if worst > 0 || slow > 0 {
            out.push_str(&format!("{}s:{worst}/{slow} ", w * 10));
        }
        w += 1;
    }
    out.push(']');
    out
}
/// ★★★★★ **WHICH trap held the longest — because a scalar names no site to fix.**
///
/// **Owner ruling 2026-09-09**: *"no inline blocking executions in mmio traps. just general
/// rule. real gpu also never holds any mmio write for milliseconds right. like rpc mmio
/// starts the operation, the block is for example a semaphore."*
///
/// [`worst_trap_us`] is the conformance metric for that rule across **every** MMIO trap —
/// [`TrapGuard`] wraps both the register read and the register write path. But a bare
/// maximum says only *that* something held 1.86 s, not *what*, and this campaign has now
/// twice paid for a single number that fit several repair plans equally well
/// (`inline_exceptions` over four sites; the GEMM ratio over launch-vs-compute).
///
/// So the worst hold carries its site. Encoded as one `u64` — `(bar << 56) | offset` — and
/// updated only by the thread that actually won [`WORST_TRAP_US`]'s `fetch_max`, so the two
/// cannot disagree about which trap they describe.
/// ⊘ Racy in principle: two threads can win the max and the identity in different orders.
/// The census therefore prints the pair as *"the worst hold, and the site that most recently
/// claimed it"* rather than asserting they are one event — an honest weaker claim beats a
/// strong one the mechanism cannot support.
static WORST_TRAP_SITE: AtomicU64 = AtomicU64::new(u64::MAX);
/// How many traps exceeded [`SLOW_TRAP_US`] — the rule's violation count, not its extreme.
/// ⊘ A maximum is one event and can be dismissed as an outlier; a COUNT cannot.
static SLOW_TRAPS: AtomicU64 = AtomicU64::new(0);

/// ★ Slow traps by decade: `1-10ms`, `10-100ms`, `100ms-1s`, `>1s`.
///
/// ⊘ Four buckets and not a percentile: the owner's rule is a hard ceiling — *"a trap may not
/// take longer than a millisecond"* — so what matters is HOW FAR over, not where the median
/// sits. A boot with 187 traps in the 1-10 ms bucket and one over a second is a different
/// problem from 187 traps over a second, and `worst`+`count` cannot tell them apart.
static SLOW_TRAP_BUCKETS: [AtomicU64; 4] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

/// The site of the most recent slow trap — survives a larger trap elsewhere taking the global
/// maximum with it.
static SLOW_TRAP_SITE_LAST: AtomicU64 = AtomicU64::new(0);
/// A trap holding longer than this is a violation of the owner's 2026-09-09 rule. 1 ms is
/// deliberately generous: real MMIO posts in nanoseconds, so anything at millisecond scale is
/// already the wrong shape, and a threshold set at the hardware's own cost would report every
/// trap and rank nothing.
pub const SLOW_TRAP_US: u64 = 1_000;

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

/// ★★★★★ **PER-REASON ATTRIBUTION FOR THE INLINE RESIDUE — because one total over four
/// sites cannot say which site to fix.**
///
/// `[measured w394, boot w394g]` a boot printed `inline_exceptions=46568` beside
/// `worst_trap=2666348us`, and that pair is the strongest lead this campaign has on parity
/// (342 ms per kernel launch, ~4 doorbell traps at the known ~86 ms cost). But
/// [`inline_exceptions`] is **one scalar over every call site**, and there are four. A
/// number that fits four different repair plans equally well has not told you which to
/// start — the same shape as `a_measurement_that_fits_two_models_is_not_a_measurement`,
/// one level up: there the count could not recover a distribution, here it cannot recover
/// an attribution.
///
/// # ⊘ Why a lock-free table and not a `Mutex<BTreeMap>`
///
/// [`OffTrap::inline_under_bql`] is minted **on the trap thread, under the VMM's global lock**, which is
/// exactly where `l1_concurrency.md` R1 forbids a potentially-blocking site. A mutex here
/// would put the instrument inside the hazard it exists to measure. So: a fixed table of
/// atomics, linear-probed, no allocation, no lock, no syscall.
///
/// Keyed by the `&'static str`'s **pointer**, not its content — the reasons are literals,
/// so pointer identity is exact and needs no comparison of bytes on a trap thread. ⊘ Two
/// distinct literals with identical text would occupy two slots, which is honest (they are
/// two sites) and is why the report prints the text beside each count.
/// ★★★★★ **SLOW TRAPS BY SITE — which register, not just how many.**
///
/// `[measured w461]` the by-decade histogram split the population in two: **one** trap over a
/// second and **169** in the 1-100 ms band, with a clean zero between them. That killed the
/// headline number as a target and left a real one — but `worst_trap` carries a site and the
/// buckets do not, so *"which 169"* was unanswerable.
///
/// ⊘ 32 slots, keyed on the trap's site value (a BAR + offset). A site that does not fit is
/// counted in the overflow rather than displacing one — losing a site silently is how a census
/// starts lying, and this file already carries that lesson for inline reasons.
const SLOW_SITE_SLOTS: usize = 32;
static SLOW_SITE_KEY: [AtomicU64; SLOW_SITE_SLOTS] =
    [const { AtomicU64::new(u64::MAX) }; SLOW_SITE_SLOTS];
static SLOW_SITE_HITS: [AtomicU64; SLOW_SITE_SLOTS] =
    [const { AtomicU64::new(0) }; SLOW_SITE_SLOTS];
static SLOW_SITE_WORST: [AtomicU64; SLOW_SITE_SLOTS] =
    [const { AtomicU64::new(0) }; SLOW_SITE_SLOTS];
/// Slow traps whose site found no free slot. ⊘ Non-zero means the census is INCOMPLETE, which
/// is a different statement from "these are all the sites".
static SLOW_SITE_OVERFLOW: AtomicU64 = AtomicU64::new(0);

/// Record one slow trap against its site. Lock-free; called from a `Drop` on a vCPU.
fn note_slow_site(site: u64, us: u64) {
    for i in 0..SLOW_SITE_SLOTS {
        let k = SLOW_SITE_KEY[i].load(Ordering::Relaxed);
        if k == site {
            SLOW_SITE_HITS[i].fetch_add(1, Ordering::Relaxed);
            SLOW_SITE_WORST[i].fetch_max(us, Ordering::Relaxed);
            return;
        }
        if k == u64::MAX
            && SLOW_SITE_KEY[i]
                .compare_exchange(u64::MAX, site, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
        {
            SLOW_SITE_HITS[i].fetch_add(1, Ordering::Relaxed);
            SLOW_SITE_WORST[i].fetch_max(us, Ordering::Relaxed);
            return;
        }
    }
    SLOW_SITE_OVERFLOW.fetch_add(1, Ordering::Relaxed);
}

/// Render the slow-trap sites as one line, busiest first.
///
/// ⊘ Busiest, NOT worst. `[measured w461]` the single worst trap is one event in a whole boot
/// and the 169 that recur are the work; sorting by duration would put the irrelevant one at
/// the top, which is the mistake this census exists to stop repeating.
#[must_use]
pub fn slow_sites_census() -> String {
    let sites = slow_sites();
    if sites.is_empty() {
        return "SLOW-SITES none — no trap exceeded the ceiling, or none was recorded".to_owned();
    }
    let rows: Vec<String> = sites
        .iter()
        .take(8)
        .map(|(site, hits, worst)| {
            format!(
                "bar{}+{:#x}={hits}(worst {worst}us)",
                site >> 56,
                site & 0x00ff_ffff_ffff_ffff
            )
        })
        .collect();
    let over = slow_site_overflow();
    format!(
        "SLOW-SITES {}{}",
        rows.join(" "),
        if over > 0 {
            format!(" ⊘ OVERFLOW={over} — this list is INCOMPLETE")
        } else {
            String::new()
        }
    )
}

/// The slow-trap sites, worst-first: `(site, hits, worst_us)`.
#[must_use]
pub fn slow_sites() -> Vec<(u64, u64, u64)> {
    let mut out: Vec<(u64, u64, u64)> = (0..SLOW_SITE_SLOTS)
        .filter_map(|i| {
            let k = SLOW_SITE_KEY[i].load(Ordering::Acquire);
            if k == u64::MAX {
                return None;
            }
            Some((
                k,
                SLOW_SITE_HITS[i].load(Ordering::Relaxed),
                SLOW_SITE_WORST[i].load(Ordering::Relaxed),
            ))
        })
        .collect();
    out.sort_by_key(|(_, hits, _)| core::cmp::Reverse(*hits));
    out
}

/// Slow traps whose site did not fit the table. ⊘ Non-zero ⇒ the site list is incomplete.
#[must_use]
pub fn slow_site_overflow() -> u64 {
    SLOW_SITE_OVERFLOW.load(Ordering::Relaxed)
}

const INLINE_REASON_SLOTS: usize = 16;
static INLINE_REASON_PTR: [AtomicUsize; INLINE_REASON_SLOTS] =
    [const { AtomicUsize::new(0) }; INLINE_REASON_SLOTS];
/// The reason text itself. ⊘ A `OnceLock` and not a reconstructed pointer: this crate
/// forbids `unsafe`, and rebuilding a `&'static str` from a recorded (ptr, len) needs it.
/// Only the thread that WON the slot's CAS ever calls `set`, so there is no contention and
/// no blocking here either.
static INLINE_REASON_TEXT: [OnceLock<&'static str>; INLINE_REASON_SLOTS] =
    [const { OnceLock::new() }; INLINE_REASON_SLOTS];
static INLINE_REASON_HITS: [AtomicU64; INLINE_REASON_SLOTS] =
    [const { AtomicU64::new(0) }; INLINE_REASON_SLOTS];
/// Mints whose reason found no free slot. ⊘ Counted rather than dropped: a table that
/// silently discards the 17th reason would under-report exactly when the picture got
/// complicated, and read as if it had not.
static INLINE_REASON_OVERFLOW: AtomicU64 = AtomicU64::new(0);

/// Record one inline mint against its reason. Lock-free; safe under the VMM's global lock.
fn note_inline_reason(what: &'static str) {
    let ptr = what.as_ptr() as usize;
    for i in 0..INLINE_REASON_SLOTS {
        let cur = INLINE_REASON_PTR[i].load(Ordering::Relaxed);
        if cur == ptr {
            INLINE_REASON_HITS[i].fetch_add(1, Ordering::Relaxed);
            return;
        }
        if cur == 0
            && INLINE_REASON_PTR[i]
                .compare_exchange(0, ptr, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            let _ = INLINE_REASON_TEXT[i].set(what);
            INLINE_REASON_HITS[i].fetch_add(1, Ordering::Relaxed);
            return;
        }
        // Lost the race for this slot, or it belongs to another reason — probe on.
    }
    INLINE_REASON_OVERFLOW.fetch_add(1, Ordering::Relaxed);
}

/// The inline residue, attributed by reason and ranked heaviest first.
///
/// ⊘ Returns pairs rather than a formatted string so a test can assert on the attribution
/// itself; [`census`] does the formatting.
#[must_use]
pub fn inline_by_reason() -> Vec<(&'static str, u64)> {
    let mut out: Vec<(&'static str, u64)> = Vec::new();
    for i in 0..INLINE_REASON_SLOTS {
        let ptr = INLINE_REASON_PTR[i].load(Ordering::Acquire);
        let hits = INLINE_REASON_HITS[i].load(Ordering::Relaxed);
        if ptr == 0 || hits == 0 {
            continue;
        }
        // ⊘ A claimed slot whose text is not yet visible is counted, not guessed: the
        // claimer sets it immediately after the CAS, so this window is vanishingly small,
        // and printing "(text not yet published)" is honest where inventing one is not.
        match INLINE_REASON_TEXT[i].get() {
            Some(what) => out.push((*what, hits)),
            None => out.push(("⊘ (reason text not yet published)", hits)),
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out
}

/// Mints whose reason did not fit the table. **Must be 0** for [`inline_by_reason`] to be
/// a complete attribution rather than a partial one.
#[must_use]
pub fn inline_reason_overflow() -> u64 {
    INLINE_REASON_OVERFLOW.load(Ordering::Relaxed)
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
            let site = WORST_TRAP_SITE.load(Ordering::Relaxed);
            format!(
                "{worst}us{} slow_traps(>{}us)={} by_decade[1-10ms={} 10-100ms={} \
                 100ms-1s={} >1s={}]",
                if site == u64::MAX {
                    " at=UNATTRIBUTED".to_string()
                } else {
                    format!(" at=bar{}+{:#x}", site >> 56, site & 0x00ff_ffff_ffff_ffff)
                },
                SLOW_TRAP_US,
                SLOW_TRAPS.load(Ordering::Relaxed),
                SLOW_TRAP_BUCKETS[0].load(Ordering::Relaxed),
                SLOW_TRAP_BUCKETS[1].load(Ordering::Relaxed),
                SLOW_TRAP_BUCKETS[2].load(Ordering::Relaxed),
                SLOW_TRAP_BUCKETS[3].load(Ordering::Relaxed)
            )
        },
    ) + &{
        let by = inline_by_reason();
        if by.is_empty() {
            String::new()
        } else {
            let over = inline_reason_overflow();
            format!(
                " | INLINE-BY-REASON{} {}",
                if over == 0 {
                    String::new()
                } else {
                    format!(" ⊘ INCOMPLETE: {over} mint(s) did not fit the table")
                },
                by.iter()
                    .map(|(what, n)| format!("[{n} × {what}]"))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        }
    }
        // ★★★★★ **w471 — the vCPU-blocking census rides the SAME line.** It is the other
        // half of `inline_exceptions`: that counts host verbs minted without an honest
        // off-trap claim, this names every door to a BLOCKING operation a vCPU reached at
        // all. ⊘ Emitted unconditionally, including its explicit empty arm — a census that
        // prints nothing when it found nothing is indistinguishable from one that never ran.
        + " | "
        + &trapcpu::census()
        + " | "
        + &phase_profile()
        + " | "
        + &crate::lockwitness::vcpu_blocking_census()
}

/// ★ w477 — re-exported here only so the boot log has ONE place that prints the censuses.
/// ⊘ The count itself lives in `kayfabe-fwd`, beside the decision it measures.

/// ★★★★★ **w495 — WALL versus CPU, which is the question every other instrument dodged.**
///
/// `bar0+0x110094` has **no decode arm anywhere in this tree** — read returns 0, write is
/// dropped — and it measures milliseconds. Code that does not exist cannot be slow, so the
/// vCPU is not executing. Seven hypotheses have now died looking for what it executes.
///
/// ⊘ Every trap figure in this campaign is **wall clock** (`Instant::elapsed`). If a vCPU
/// thread is descheduled mid-trap — and the box runs 8 vCPUs plus a coordinator plus isolate
/// processes on 11 cores — the measured "trap" includes time the thread was not running. A
/// 20 ms trap could be 50 us of work and the rest waiting for a core.
///
/// ⇒ record BOTH. `cpu_us` far below `wall_us` is descheduling and nothing of ours is at
/// fault; `cpu_us` tracking `wall_us` is real work and the site is worth chasing.
pub mod trapcpu {
    use core::sync::atomic::{AtomicU64, Ordering};

    static SAMPLES: AtomicU64 = AtomicU64::new(0);
    static WORST_WALL_US: AtomicU64 = AtomicU64::new(0);
    /// The CPU time of the trap that had the worst WALL time — the pair that answers the
    /// question. ⊘ Not the worst CPU time independently: two independent maxima from
    /// different traps cannot be compared, which is the mistake this module exists to avoid.
    static WORST_WALL_CPU_US: AtomicU64 = AtomicU64::new(0);
    /// Traps over the slow threshold whose CPU time was under a tenth of their wall time.
    static SLOW_AND_STARVED: AtomicU64 = AtomicU64::new(0);
    /// Traps over the slow threshold that really did burn the CPU.
    static SLOW_AND_BUSY: AtomicU64 = AtomicU64::new(0);

    /// Slow traps that carried an INVOLUNTARY context switch — the scheduler preempted them.
    static SLOW_PREEMPTED: AtomicU64 = AtomicU64::new(0);
    /// Slow traps that carried a VOLUNTARY context switch — the thread BLOCKED on something.
    /// ★ This is the number that decides whether the owner's coarse-lock theory is right.
    static SLOW_BLOCKED: AtomicU64 = AtomicU64::new(0);

    /// Record one trap's wall and CPU cost plus its context switches.
    ///
    /// `vol`/`invol` are the deltas across the trap. ⊘ Both can be zero on a slow trap, which
    /// is itself a finding: neither preempted nor blocked would mean the time went somewhere
    /// neither the scheduler nor a wait explains.
    pub fn note_full(wall_us: u64, cpu_us: u64, vol: u64, invol: u64) {
        if wall_us >= super::SLOW_TRAP_US {
            if invol > 0 {
                SLOW_PREEMPTED.fetch_add(1, Ordering::Relaxed);
            }
            if vol > 0 {
                SLOW_BLOCKED.fetch_add(1, Ordering::Relaxed);
            }
        }
        note(wall_us, cpu_us);
    }

    /// Record one trap's wall and CPU cost, both in microseconds.
    pub fn note(wall_us: u64, cpu_us: u64) {
        SAMPLES.fetch_add(1, Ordering::Relaxed);
        if WORST_WALL_US.fetch_max(wall_us, Ordering::Relaxed) < wall_us {
            WORST_WALL_CPU_US.store(cpu_us, Ordering::Relaxed);
        }
        if wall_us >= super::SLOW_TRAP_US {
            if cpu_us * 10 < wall_us {
                SLOW_AND_STARVED.fetch_add(1, Ordering::Relaxed);
            } else {
                SLOW_AND_BUSY.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// One line. ⊘ Prints its unmeasured arm explicitly rather than a bare zero.
    #[must_use]
    pub fn census() -> String {
        let n = SAMPLES.load(Ordering::Relaxed);
        if n == 0 {
            return "TRAP-CPU unmeasured — the wall/CPU pair was never sampled".to_string();
        }
        let (w, c) = (
            WORST_WALL_US.load(Ordering::Relaxed),
            WORST_WALL_CPU_US.load(Ordering::Relaxed),
        );
        let (starved, busy) = (
            SLOW_AND_STARVED.load(Ordering::Relaxed),
            SLOW_AND_BUSY.load(Ordering::Relaxed),
        );
        format!(
            "TRAP-CPU n={n} worst_wall={w}us cpu_of_that_trap={c}us slow_starved={starved} \
             slow_busy={busy} slow_preempted={} slow_blocked={} (starved = wall over 10x cpu \
             ⇒ NOT RUNNING; preempted = scheduler took the cpu; blocked = the thread WAITED \
             on something, which is ours)",
            SLOW_PREEMPTED.load(Ordering::Relaxed),
            SLOW_BLOCKED.load(Ordering::Relaxed),
        )
    }
}

/// ⊘⊘⊘ **TEMPORARY — THE OVER-BUDGET WATCHDOG. DELETE BEFORE SHIPPING.**
///
/// Owner, 2026-09-12: *"crash when a trap exceeds 1ms. then you know where it happened, why,
/// with dump, and iterate to fix it."*
///
/// ★ Why a crash and not a log line: the census already says WHICH register is slow and has
/// said so all session. What it cannot say is **where the time goes**, because by the time a
/// `TrapGuard` drops the trap is over and a backtrace taken there names the drop site. A
/// watchdog that aborts while the trap is still running freezes the stuck thread in place,
/// so the core carries the answer instead of being sampled for.
///
/// ⊘ `[measured w470]` the alternative — 10 Hz stack sampling — found one 0.4 s park and
/// never once caught the multi-second event. Luck is not an instrument.
///
/// Armed only by `KAYFABE_TRAP_FATAL_US`; absent, `watchdog_arm` returns before doing
/// anything and no thread is spawned.
mod watchdog {
    use super::SLOW_TRAP_US;
    use core::sync::atomic::{AtomicU64, Ordering};

    pub(super) const SLOTS: usize = 64;
    /// Micros-since-epoch at which the trap in this slot started; 0 = free.
    pub(super) static START_US: [AtomicU64; SLOTS] = [const { AtomicU64::new(0) }; SLOTS];
    /// The site of the trap in this slot, for the abort message.
    pub(super) static SITE: [AtomicU64; SLOTS] = [const { AtomicU64::new(0) }; SLOTS];
    /// ⊘ w495 — the kernel thread id occupying this slot, so the watchdog can signal THE
    /// STUCK THREAD rather than the whole process. Supplied by the caller: this crate forbids
    /// `unsafe` and cannot call `gettid` itself.
    pub(super) static TID: [AtomicU64; SLOTS] = [const { AtomicU64::new(0) }; SLOTS];
    /// Set once, by the shim, to a function that signals one thread by tid. `None` leaves the
    /// old whole-process freeze in place, which is the honest fallback rather than silence.
    pub(super) static ALARM_THREAD: std::sync::OnceLock<fn(i32)> = std::sync::OnceLock::new();

    pub(super) fn now_us() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_micros() as u64)
    }

    fn budget_us() -> Option<u64> {
        static B: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
        *B.get_or_init(|| {
            std::env::var("KAYFABE_TRAP_FATAL_US")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .or_else(|| {
                    std::env::var("KAYFABE_TRAP_FATAL")
                        .is_ok()
                        .then_some(SLOW_TRAP_US)
                })
        })
    }

    #[must_use]
    pub(super) fn armed() -> bool {
        budget_us().is_some()
    }

    /// ★★★ **PRAMIN is the enumerated exception and must not trip the watchdog.**
    /// Owner, 2026-09-12: *"just exclude the pramin one"* — and the reason is in the
    /// architecture: the BAR0 window base has **no completion for the guest to wait on**, so
    /// the next read must observe the new window and the remap has to finish inline. It is
    /// bring-up only, so the stall is bounded. Watching it would only ever report the one
    /// stall we have already agreed to.
    ///
    /// Exempt: the window-base register `NV_PBUS_BAR0_WINDOW` (`0x001700`) and the PRAMIN
    /// aperture itself (`0x700000..0x800000`), both on BAR0.
    #[must_use]
    pub(super) fn is_exempt(site: u64) -> bool {
        if site == u64::MAX {
            return false;
        }
        let (bar, off) = (site >> 56, site & 0x00ff_ffff_ffff_ffff);
        bar == 0 && (off == 0x0017_00 || (0x0070_0000..0x0080_0000).contains(&off))
    }

    /// Whether to STOP rather than abort. ⊘ Default is STOP: the owner allowed either, and a
    /// stopped process keeps the stuck thread attachable with `eu-stack -p` while every other
    /// thread is frozen too — an abort would demand core handling for the same information.
    #[must_use]
    fn pause_not_abort() -> bool {
        !std::env::var("KAYFABE_TRAP_FATAL_ABORT").is_ok()
    }

    /// ⊘ TEMPORARY — the pre-forked stopper. Parks on a pipe so that firing costs a write
    /// instead of a fork. Held for the life of the process.
    static STOPPER: std::sync::OnceLock<std::sync::Mutex<Option<std::process::Child>>> =
        std::sync::OnceLock::new();

    fn stopper_start() {
        let pid = std::process::id();
        let child = std::process::Command::new("sh")
            .arg("-c")
            // ⊘⊘ **NO `exec`.** `exec kill` forces an `execve` of /bin/kill, ~1 ms, and
            // `[measured w485]` that was enough for the trap to end before the signal landed
            // — twice, with the stack showing the worker mid-`spawn_host` and no vCPU inside
            // any trap. Without `exec`, `kill` is a shell BUILTIN and the signal is sent from
            // the already-parked process with no exec at all. The `head` runs BEFORE the
            // trigger, so its cost is paid at arm time and never on the firing path.
            .arg(format!("head -c 1 >/dev/null; kill -STOP {pid}"))
            .stdin(std::process::Stdio::piped())
            .spawn()
            .ok();
        let _ = STOPPER.set(std::sync::Mutex::new(child));
    }

    pub(super) fn stopper_fire() {
        use std::io::Write;
        let Some(m) = STOPPER.get() else { return };
        let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = g.as_mut()
            && let Some(stdin) = c.stdin.as_mut()
        {
            let _ = stdin.write_all(b"x");
            let _ = stdin.flush();
        }
        // ⊘ The STOP lands on us shortly after this returns; nothing here waits for it,
        // because the thread that waits is the thread that would have to be stopped.
    }

    /// Spawn the watchdog once, if armed.
    pub(super) fn arm() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        let Some(budget) = budget_us() else {
            return;
        };
        ONCE.call_once(|| {
            stopper_start();
            std::thread::Builder::new()
                .name("kayfabe-trap-watchdog".into())
                .spawn(move || {
                    eprintln!(
                        "kayfabe: ⊘⊘⊘ TRAP WATCHDOG ARMED at {budget}us — THIS IS A DEBUG \
                         BUILD. It ABORTS the process while the offending trap is still on \
                         the stack, so the core names where the time went. Never ship this."
                    );
                    loop {
                        std::thread::sleep(std::time::Duration::from_micros(200));
                        let now = now_us();
                        for i in 0..SLOTS {
                            let st = START_US[i].load(Ordering::Relaxed);
                            if st == 0 || now.saturating_sub(st) < budget {
                                continue;
                            }
                            let site = SITE[i].load(Ordering::Relaxed);
                            eprintln!(
                                "kayfabe: ⊘⊘⊘ TRAP OVER BUDGET — {}us at {} (budget {budget}us). \
                                 FREEZING THE PROCESS WITH THE TRAP STILL RUNNING. Every \
                                 thread is stopped and attachable: `eu-stack -p <pid>`, then \
                                 `kill -CONT <pid>`. ⊘ Says FREEZING, not aborting — the \
                                 first cut of this line said abort while the code sent \
                                 SIGSTOP, and a message that names the wrong action sends \
                                 the reader looking for a core that does not exist.",
                                now.saturating_sub(st),
                                if site == u64::MAX {
                                    "UNATTRIBUTED".to_string()
                                } else {
                                    format!(
                                        "bar{}+{:#x}",
                                        site >> 56,
                                        site & 0x00ff_ffff_ffff_ffff
                                    )
                                },
                            );
                            // ⊘ Flush before aborting: a message lost to buffering would make
                            // the core the ONLY evidence, and a core with no line saying why
                            // reads as an unrelated crash.
                            use std::io::Write;
                            let _ = std::io::stderr().flush();
                            // ⊘ w495 — signal THE STUCK THREAD when the shim installed a
                            // thread-directed alarm: its dump is then taken at the site that
                            // is actually hanging, instead of wherever the freeze happened to
                            // land. `[measured w484/w485]` the whole-process freeze arrived
                            // after the trap had ended, twice, and named nothing.
                            let tid = TID[i].load(Ordering::Relaxed);
                            if tid != 0
                                && let Some(alarm) = ALARM_THREAD.get()
                            {
                                START_US[i].store(0, Ordering::Release);
                                alarm(tid as i32);
                                continue;
                            }
                            if pause_not_abort() {
                                // ⊘ SIGSTOP freezes EVERY thread, including the one still
                                // inside the trap, and leaves the process attachable.
                                // `SIGSTOP` cannot be caught or ignored, so nothing in the
                                // process can decline it.
                                // ⊘⊘ **PRE-FORKED, w484.** The first cut spawned `kill(1)`
                                // here and MISSED: `[measured w484]` it reported 8418us at
                                // `bar0+0x110094`, and by the time the signal landed every
                                // vCPU was back inside `KVM_RUN` — i.e. running guest code,
                                // not trapped. A fork+exec costs milliseconds, which is the
                                // same order as the trap it is trying to freeze.
                                //
                                // ⇒ the stopper is forked ONCE at arm time and parks on a
                                // pipe; firing is a one-byte write, tens of microseconds.
                                stopper_fire();
                                // If somebody continues us, do not re-fire on the same trap.
                                START_US[i].store(0, Ordering::Release);
                            } else {
                                std::process::abort();
                            }
                        }
                    }
                })
                .ok();
        });
    }
}

/// ⊘ TEMPORARY — see [`watchdog`].
fn watchdog_arm() {
    if watchdog::armed() {
        watchdog::arm();
    }
}

/// ⊘ TEMPORARY — an in-flight trap's slot, held for the duration of the trap.
struct InFlight;

/// ⊘ TEMPORARY (w495) — how to learn the calling thread's kernel id, supplied by the shim
/// because this crate forbids `unsafe` and cannot call `gettid` itself.
static CURRENT_TID: std::sync::OnceLock<fn() -> i32> = std::sync::OnceLock::new();

/// ★★★★★ **w592 — THE ONLY THING THAT CAN SETTLE THE OWNER'S RULE ABOUT A SLOW TRAP.**
///
/// Owner, on the one trap still over budget: *"the thread wasn't scheduled doesn't count, but
/// only if that's a vCPU steal, not if it was waiting on a blocking lock in the vCPU thread."*
///
/// ⊘⊘ **Wall time cannot tell those apart, and every claim on this subject so far has been an
/// ARGUMENT.** `[measured w583]` *"the 39.6 ms is not a lock — rank-0 waits measured ZERO and
/// the worst hold is 330 us, so it falls on the schedulable side"* — true as far as it goes, and
/// it establishes only that WE were not holding the thread. It does not establish that the
/// thread was off-CPU rather than spinning in our own code, which is the distinction the rule
/// turns on.
///
/// ★ `wall - thread_cpu` is that distinction, directly: time the trap took minus time the
/// thread actually RAN. Large ⇒ descheduled (the owner's excuse applies). Near zero ⇒ we burned
/// the budget executing, and the excuse does not apply.
///
/// ⚠ **Default OFF, and site-scoped when on.** `CLOCK_THREAD_CPUTIME_ID` is a real syscall — it
/// is not in the vDSO — so reading it on every trap would put two syscalls inside every MMIO
/// exit, which is the very cost this instrument exists to account for. `KAYFABE_TRAP_CPU_SITE`
/// names ONE offset (hex, e.g. `110c00`), and only traps at that site pay. That the instrument
/// must not perturb what it measures is the whole lesson of w586.
static THREAD_CPU_NS: std::sync::OnceLock<fn() -> u64> = std::sync::OnceLock::new();

/// The site `KAYFABE_TRAP_CPU_SITE` selected, as a BAR0 offset. `u64::MAX` = none.
static CPU_SITE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);

/// The worst `(wall_us, cpu_us)` seen at the selected site, packed so one load is consistent.
static CPU_WORST_WALL: AtomicU64 = AtomicU64::new(0);
static CPU_WORST_CPU: AtomicU64 = AtomicU64::new(0);
static CPU_SAMPLES: AtomicU64 = AtomicU64::new(0);

/// Install the thread-CPU-time source. Called once by the shim, where `unsafe` is permitted.
pub fn install_thread_cpu_clock(f: fn() -> u64) {
    let _ = THREAD_CPU_NS.set(f);
    if let Ok(v) = std::env::var("KAYFABE_TRAP_CPU_SITE") {
        if let Ok(off) = u64::from_str_radix(v.trim_start_matches("0x"), 16) {
            CPU_SITE.store(off, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// `(samples, worst wall us, that trap's thread-CPU us)` at the selected site.
#[must_use]
pub fn trap_cpu_census() -> String {
    let n = CPU_SAMPLES.load(Ordering::Relaxed);
    let site = CPU_SITE.load(std::sync::atomic::Ordering::Relaxed);
    if site == u64::MAX {
        return "TRAP-CPU ⊘ NOT ARMED — set KAYFABE_TRAP_CPU_SITE=<hex bar0 offset>.                 ⚠ UNMEASURED, which is not the same as zero off-CPU time."
            .to_string();
    }
    if n == 0 {
        return format!(
            "TRAP-CPU site=bar0+0x{site:x} samples=0 ⊘ the site was armed and never trapped              SLOWLY — nothing to attribute, and that is a real answer rather than a missing one."
        );
    }
    let wall = CPU_WORST_WALL.load(Ordering::Relaxed);
    let cpu = CPU_WORST_CPU.load(Ordering::Relaxed);
    format!(
        "TRAP-CPU site=bar0+0x{site:x} slow_samples={n} worst_wall={wall}us          thread_cpu={cpu}us off_cpu={}us ⇒ {} — the owner's rule: time the thread was NOT          SCHEDULED is excusable (a vCPU steal); time it SPENT RUNNING is not.",
        wall.saturating_sub(cpu),
        if wall.saturating_sub(cpu) * 4 > wall * 3 {
            "DESCHEDULED for most of it"
        } else {
            "RUNNING for most of it ⊘ this is OUR work, not a steal"
        }
    )
}

/// ⊘ TEMPORARY (w495) — install the thread-directed alarm and the tid source. Called once by
/// the shim, which is where `unsafe` is permitted.
///
/// ★ Without this the watchdog falls back to freezing the whole process, which
/// `[measured w484/w485]` arrived AFTER the trap had ended twice and named nothing. A
/// thread-directed signal lands on the thread that is actually stuck.
pub fn install_stall_alarm(alarm: fn(i32), tid_of_current: fn() -> i32) {
    let _ = watchdog::ALARM_THREAD.set(alarm);
    let _ = CURRENT_TID.set(tid_of_current);
}

impl InFlight {
    fn claim(site: u64) -> Option<usize> {
        if !watchdog::armed() || watchdog::is_exempt(site) {
            return None;
        }
        let now = watchdog::now_us().max(1);
        for i in 0..watchdog::SLOTS {
            if watchdog::START_US[i]
                .compare_exchange(0, now, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                watchdog::SITE[i].store(site, Ordering::Relaxed);
                watchdog::TID[i].store(
                    crate::trapwitness::CURRENT_TID.get().map_or(0, |f| f() as u64),
                    Ordering::Relaxed,
                );
                return Some(i);
            }
        }
        // ⊘ Every slot busy: do not block and do not grow. A trap that cannot be watched is
        // simply unwatched, which is the right failure for a debug instrument.
        None
    }

    fn release(slot: Option<usize>) {
        if let Some(i) = slot {
            watchdog::START_US[i].store(0, Ordering::Release);
        }
    }
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
    /// The in-flight slot this trap holds while it runs, released on drop. ⊘ TEMPORARY:
    /// exists only for the over-budget watchdog and goes when that does.
    inflight: Option<usize>,
    /// `(bar << 56) | offset` of the trapped access, or `u64::MAX` if unattributed.
    site: u64,
    start: std::time::Instant,
    /// ★ w592 — thread-CPU nanoseconds at entry, only when this site is the selected one.
    cpu_start_ns: Option<u64>,
    _not_send: PhantomData<*mut ()>,
}

impl TrapGuard {
    /// Mark this thread as executing a guest trap until the guard drops.
    #[must_use]
    pub fn enter() -> Self {
        Self::enter_at(u64::MAX)
    }

    /// Enter a trap whose SITE is known — `site` is `(bar << 56) | offset`, or `u64::MAX`
    /// for "unattributed". See [`WORST_TRAP_SITE`].
    #[must_use]
    pub fn enter_at(site: u64) -> Self {
        // ⊘⊘⊘ **TEMPORARY DEBUG INSTRUMENT — DELETE BEFORE SHIPPING.** Owner, 2026-09-12:
        // *"only during debug ensure this is OFF/deleted later and in prod (temporary to
        // test dont leave it in code), is crash when a trap exceeds 1ms."* Default-off, and
        // `watchdog_arm()` is a no-op unless `KAYFABE_TRAP_FATAL_US` is set.
        watchdog_arm();
        let inflight = InFlight::claim(site);
        TRAP_DEPTH.with(|d| d.set(d.get() + 1));
        TRAP_ENTRIES.with(|c| c.set(c.get() + 1));
        // ⊘ Site-scoped: `(bar << 56) | offset`, and only BAR0 is ever selected.
        let cpu_start_ns = (site == CPU_SITE.load(std::sync::atomic::Ordering::Relaxed))
            .then(|| THREAD_CPU_NS.get().map(|f| f()))
            .flatten();
        Self {
            inflight,
            start: std::time::Instant::now(),
            site,
            cpu_start_ns,
            _not_send: PhantomData,
        }
    }
}

impl Drop for TrapGuard {
    fn drop(&mut self) {
        InFlight::release(self.inflight);
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
            if us >= SLOW_TRAP_US {
                SLOW_TRAPS.fetch_add(1, Ordering::Relaxed);
                // ★★★ w459 — a HISTOGRAM, because a maximum is one event and a count is a
                // total, and neither tells you the shape.
                //
                // `[measured w447-w458]` `worst_trap` sat at ~1.87 s and `slow_traps` at ~187
                // across six boots, and from those two numbers alone I could not tell whether
                // this is **one pathological trap plus 186 merely-slow ones** or **187 traps
                // that are all seconds long**. Those demand opposite fixes, and the ledger has
                // already recorded that *a count and a total cannot recover a distribution*.
                let b = match us {
                    ..=9_999 => 0,          // 1-10 ms
                    ..=99_999 => 1,         // 10-100 ms
                    ..=999_999 => 2,        // 100 ms - 1 s
                    _ => 3,                 // over a second
                };
                SLOW_TRAP_BUCKETS[b].fetch_add(1, Ordering::Relaxed);
                // ⊘ And the worst-per-site, so "which register" survives a later, larger trap
                // elsewhere overwriting the global maximum.
                if self.site != 0 {
                    SLOW_TRAP_SITE_LAST.store(self.site, Ordering::Relaxed);
                }
                note_slow_site(self.site, us);
                // ★ w592 — and for the selected site, how much of it the thread actually RAN.
                if let (Some(t0), Some(f)) = (self.cpu_start_ns, THREAD_CPU_NS.get()) {
                    let cpu_us = f().saturating_sub(t0) / 1_000;
                    CPU_SAMPLES.fetch_add(1, Ordering::Relaxed);
                    if CPU_WORST_WALL.fetch_max(us, Ordering::Relaxed) < us {
                        CPU_WORST_CPU.store(cpu_us, Ordering::Relaxed);
                    }
                }
            }
            // `fetch_max` returns the PREVIOUS value: we won iff it was smaller than ours.
            // ★★★★★ **w481 — WHEN, not just how big.** `worst_trap` over a whole boot is
            // dominated by device bring-up, and the owner's bar is on the WORKLOAD:
            // *"44.8ms during boot can be acceptable. measure worst trap during the raw
            // client. that should be sub ms."* A single scalar cannot answer that, so this
            // keeps the worst trap per one-second bucket since the first trap. The client
            // phase is the tail of the profile, and it is readable directly off the line.
            // ⊘ Wrapping rather than growing: a fixed table cannot leak, and a boot longer
            // than the window folds onto itself visibly rather than silently reallocating
            // under a vCPU.
            {
                let sec = trap_epoch().elapsed().as_secs() as usize;
                PHASE_WORST_US[sec % PHASE_BUCKETS].fetch_max(us, Ordering::Relaxed);
                PHASE_SLOW[sec % PHASE_BUCKETS]
                    .fetch_add(u64::from(us >= SLOW_TRAP_US), Ordering::Relaxed);
                PHASE_HIGH_SEC.fetch_max(sec as u64, Ordering::Relaxed);
            }
            if WORST_TRAP_US.fetch_max(us, Ordering::Relaxed) < us {
                WORST_TRAP_SITE.store(self.site, Ordering::Relaxed);
            }
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
    /// condition: it means a host round trip was about to run with the VMM's global lock held, freezing
    /// every vCPU and the VMM's dispatch loop — the failure `blocking_and_completion_model.md` §0
    /// exists to describe.
    #[must_use]
    pub fn claim(what: &'static str) -> Self {
        assert!(
            !in_trap(),
            "INLINE-SAFE violation (blocking_and_completion_model.md §1 clause (a)/(b)): \
             `{what}` asked for an off-trap witness while this thread is {depth} guest-trap \
             dispatch(es) deep. Every guest MMIO write arrives with the VMM's global lock held, so \
             this call would stall EVERY vCPU and the VMM's dispatch loop, not just the ringing \
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
        note_inline_reason(what);
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
        assert!(
            in_trap(),
            "an inner guard's Drop must not un-mark the outer trap"
        );
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
        assert!(
            !census().contains("UNMEASURED"),
            "a closed guard must publish a hold"
        );
    }
}

#[cfg(test)]
mod inline_attribution {
    //! ★★★★★ **THE RESIDUE, ATTRIBUTED.** `inline_exceptions` is one scalar over four
    //! production call sites, and `[measured w394]` it read 46 568 beside a 2.67 s worst
    //! trap — the strongest parity lead this campaign has. One number over four sites cannot
    //! say which site to fix: it fits every repair plan equally well and endorses none.
    //!
    //! ⊘ **ONE test, not three, and that is forced rather than stylistic.** The table is
    //! process-global and fixed-size, so a test that fills it changes what every other test
    //! can observe — even under `--test-threads=1`, where the order is still not ours to
    //! choose. Splitting these would produce a suite that passes or fails on test ORDER,
    //! which is the instrument being unreliable about itself.
    use super::*;

    #[test]
    fn the_residue_is_ranked_heaviest_first_and_admits_what_it_lost() {
        // ---- 1. RANKING. A list that does not sort is the scalar again, in a costume.
        let heavy: &'static str = "a heavy site";
        let light: &'static str = "a light site";
        for _ in 0..7 {
            note_inline_reason(heavy);
        }
        note_inline_reason(light);

        let by = inline_by_reason();
        let h = by.iter().find(|(w, _)| *w == heavy).expect("heavy missing");
        let l = by.iter().find(|(w, _)| *w == light).expect("light missing");
        assert_eq!(h.1, 7, "{by:?}");
        assert_eq!(l.1, 1, "{by:?}");
        assert!(
            by.iter().position(|(w, _)| *w == heavy).unwrap()
                < by.iter().position(|(w, _)| *w == light).unwrap(),
            "the heavier site must sort first: {by:?}"
        );
        assert_eq!(
            inline_reason_overflow(),
            0,
            "two reasons must not overflow a 16-slot table"
        );
        assert!(!census().contains("INCOMPLETE"), "{}", census());

        // ---- 2. OVERFLOW. The table is fixed-size and CAN lose a reason. It must say so:
        // a partial attribution presented as a whole one is worse than the scalar it
        // replaced, because it looks finished.
        const FILL: [&str; INLINE_REASON_SLOTS] = [
            "f00", "f01", "f02", "f03", "f04", "f05", "f06", "f07", "f08", "f09", "f10",
            "f11", "f12", "f13", "f14", "f15",
        ];
        for w in FILL {
            note_inline_reason(w);
        }
        let before = inline_reason_overflow();
        note_inline_reason("one too many");
        assert!(
            inline_reason_overflow() > before,
            "a reason that found no slot must be COUNTED; dropping it silently would \
             under-report exactly when the picture got complicated"
        );
        assert!(
            census().contains("INCOMPLETE"),
            "and the census must SAY the attribution is partial: {}",
            census()
        );
    }
}

#[cfg(test)]
mod the_mmio_rule_is_measured_not_asserted {
    //! ★★★★★ **OWNER RULING 2026-09-09, as a metric.**
    //!
    //! > *"no inline blocking executions in mmio traps. just general rule. real gpu also
    //! > never holds any mmio write for milliseconds right. like rpc mmio starts the
    //! > operation, the block is for example a semaphore"*
    //!
    //! An MMIO write on real hardware POSTS: it retires and the vCPU walks away, and the
    //! guest's wait is a separate explicit thing (a semaphore, an interrupt). A trap held for
    //! milliseconds is therefore not merely slow, it is **the wrong shape** — it invents a
    //! blocking store the hardware does not have.
    //!
    //! ⊘ [`worst_trap_us`] alone cannot drive that rule: a maximum is ONE event and is
    //! dismissible as an outlier, and a bare number names no site. So the census carries the
    //! worst hold's SITE and a COUNT of violations.
    use super::*;

    #[test]
    fn the_census_names_the_site_of_the_worst_hold_and_counts_the_violations() {
        {
            let _g = TrapGuard::enter_at((1u64 << 56) | 0x8c);
            std::thread::sleep(std::time::Duration::from_micros(SLOW_TRAP_US + 500));
        }
        let line = census();
        assert!(line.contains("at=bar1+0x8c"), "must name the site: {line}");
        assert!(
            line.contains(&format!("slow_traps(>{}us)=", SLOW_TRAP_US)),
            "must count violations, not only report an extreme: {line}"
        );
        assert!(
            !line.contains("slow_traps(>1000us)=0"),
            "a hold longer than the threshold must COUNT as one: {line}"
        );
    }

    /// ⊘ An unattributed guard must say so rather than print a plausible `bar0+0x0`, which is
    /// a real register and would send a reader to the wrong place.
    #[test]
    fn an_unattributed_trap_says_unattributed_and_never_invents_a_register() {
        let site_line = {
            let _g = TrapGuard::enter();
            "entered"
        };
        assert_eq!(site_line, "entered");
        // The census may name an earlier attributed site from another test in this binary;
        // what must never happen is `u64::MAX` decoding to a real-looking register.
        let decoded = format!("at=bar{}+{:#x}", u64::MAX >> 56, u64::MAX & 0x00ff_ffff_ffff_ffff);
        assert!(
            !census().contains(&decoded),
            "the sentinel must not decode as a register: {}",
            census()
        );
    }
}
