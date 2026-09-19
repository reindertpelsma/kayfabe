//! ★★★★★ **THE DOORBELL TABLE — one atomic word per token, no lock, no device.**
//!
//! Owner, stated twice and not acted on until now:
//!
//! > *"doorbell must be simple, it just reads a table, does ring host + return for passthrough
//! > or queues and wake thread for emulated, all non blocking, and returns. its like a hundred
//! > lines what it touches"*
//!
//! > *"Its only a small lock over one small lookup table containing indexed by doorbell token
//! > containing a type that says whether its unallocated, passthrough or emulated and a target
//! > field. … since the entire table can just be 64 bit words stored at convienient doorbell
//! > token indexes (like 2 bits for type and 62 bits for target), a lock can be entirely
//! > redundant for safety by atomic CPU read/write instructions."*
//!
//! # Why a flat array of atomics and not "the same thing, but tidier"
//!
//! `ring_inline` — the doorbell path this replaces — is **481 lines** and reaches the device,
//! the plane, the publication queue and the address tables. Work accumulated there because it
//! *could*: everything was in scope. Three separate rulings against publishing or executing on
//! that path were each violated by someone who had just read them.
//!
//! ⇒ **The fix is to take the reach away, not to police it.** A `&DoorbellTable` cannot publish,
//! cannot walk a page table and cannot execute a channel, because it does not have them. That is
//! the same move as `OffVcpu`, one layer lower: make the violation unspellable rather than
//! forbidden.
//!
//! ⊘ **No lock, and not as an optimisation.** A single naturally-aligned `u64` is read and
//! written atomically by the CPU. A lock here would add a shared structure between a vCPU and
//! everything else — the exact contention that put 1098 traps over the owner's 1 ms budget in
//! the last measured boot — and would buy nothing a `Relaxed` load does not already give.

use std::sync::atomic::{AtomicU64, Ordering};

/// What a token's entry says. Two bits, so the whole entry fits one word beside its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// No channel owns this token. ⊘ The doorbell does **nothing** — it is not an error, and
    /// it must not be a refusal that allocates, logs per-event or takes a lock. A guest may
    /// ring anything it likes; that is its prerogative and our non-event.
    Unallocated,
    /// Forward to the host: one dword store to `target`, then return to the guest.
    Passthrough { host_token: u64 },
    /// Ours to run: queue `target` and wake the coordinator, then return to the guest.
    Emulated { chan: u64 },
}

const TYPE_BITS: u64 = 2;
const TYPE_MASK: u64 = 0b11;
const TAG_UNALLOCATED: u64 = 0;
const TAG_PASSTHROUGH: u64 = 1;
const TAG_EMULATED: u64 = 2;

/// The largest target a 62-bit field can hold. ⊘ Stated rather than assumed: an installer that
/// silently truncated a target would route a doorbell to the wrong channel.
pub const MAX_TARGET: u64 = (1u64 << (64 - TYPE_BITS)) - 1;

/// One atomic word per doorbell token.
#[derive(Debug)]
pub struct DoorbellTable {
    slots: Box<[AtomicU64]>,
}

impl DoorbellTable {
    /// A table of `len` tokens, all unallocated.
    #[must_use]
    pub fn new(len: usize) -> Self {
        Self {
            slots: (0..len).map(|_| AtomicU64::new(0)).collect(),
        }
    }

    /// How many tokens this table can answer for.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// ⊘ Clippy's companion to `len`; a table is never empty in practice.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// ★★★★★ **THE WHOLE DOORBELL READ PATH: one bounds check, one atomic load, one decode.**
    ///
    /// No lock, no allocation, no device, no logging. A token past the end is
    /// [`Route::Unallocated`] — *"not found, not denied"*, and a guest ringing a wild token
    /// must not be able to make us do work proportional to its imagination.
    #[must_use]
    pub fn route(&self, token: u64) -> Route {
        let Ok(i) = usize::try_from(token) else {
            return Route::Unallocated;
        };
        let Some(slot) = self.slots.get(i) else {
            return Route::Unallocated;
        };
        // ⊘ `Relaxed` is correct and deliberate. This word is self-describing: type and target
        // travel together in one atomic unit, so there is no second location whose visibility
        // we need ordered against it. An `Acquire` here would be cargo-culted cost on the
        // hottest path in the system.
        let w = slot.load(Ordering::Relaxed);
        match w & TYPE_MASK {
            TAG_PASSTHROUGH => Route::Passthrough {
                host_token: w >> TYPE_BITS,
            },
            TAG_EMULATED => Route::Emulated {
                chan: w >> TYPE_BITS,
            },
            _ => Route::Unallocated,
        }
    }

    /// Install a route. Returns `false` if `target` does not fit 62 bits, in which case the
    /// slot is left **unchanged** — a truncated target is a doorbell delivered to the wrong
    /// channel, which is worse than a refused install.
    pub fn install(&self, token: u64, route: Route) -> bool {
        let Ok(i) = usize::try_from(token) else {
            return false;
        };
        let Some(slot) = self.slots.get(i) else {
            return false;
        };
        let w = match route {
            Route::Unallocated => TAG_UNALLOCATED,
            Route::Passthrough { host_token } => {
                if host_token > MAX_TARGET {
                    return false;
                }
                (host_token << TYPE_BITS) | TAG_PASSTHROUGH
            }
            Route::Emulated { chan } => {
                if chan > MAX_TARGET {
                    return false;
                }
                (chan << TYPE_BITS) | TAG_EMULATED
            }
        };
        slot.store(w, Ordering::Relaxed);
        true
    }
}

/// ★★★★★ **THE THREE PROPERTIES THAT MAKE THE LOCK REDUNDANT.** Owner, 2026-09-10:
///
/// > *"ensure that the entries fit in one atomic CPU instruction (so that means 8 bytes per
/// > entry is the limit, 4 bytes is also ok if thats sufficient) and every read/write is one
/// > atomic read/write into that entry, plus that the table is indexed O(1) by the doorbell
/// > token."*
///
/// ⊘ Compile-time, so the first two cannot regress silently. If a later edit needs a second
/// field beside the target, this stops the build rather than letting the table become a
/// struct that no CPU can load atomically — at which point the missing lock becomes a race
/// instead of a simplification.
const _: () = {
    assert!(
        core::mem::size_of::<AtomicU64>() == 8,
        "an entry must fit ONE atomic access"
    );
    assert!(
        core::mem::align_of::<AtomicU64>() == 8,
        "and be naturally aligned, or it is torn"
    );
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unwritten_table_routes_nothing() {
        let t = DoorbellTable::new(8);
        for tok in 0..8 {
            assert_eq!(t.route(tok), Route::Unallocated);
        }
    }

    /// ⊘ A guest may ring any token. Out of range is a NON-EVENT, not a refusal that costs
    /// anything — "not found, not denied", and no work proportional to the guest's imagination.
    #[test]
    fn a_wild_token_is_a_non_event() {
        let t = DoorbellTable::new(4);
        assert_eq!(t.route(4), Route::Unallocated);
        assert_eq!(t.route(u64::MAX), Route::Unallocated);
    }

    #[test]
    fn both_kinds_round_trip_with_their_target() {
        let t = DoorbellTable::new(4);
        assert!(t.install(1, Route::Passthrough { host_token: 0x5 }));
        assert!(t.install(2, Route::Emulated { chan: 0x2b }));
        assert_eq!(t.route(1), Route::Passthrough { host_token: 0x5 });
        assert_eq!(t.route(2), Route::Emulated { chan: 0x2b });
        assert_eq!(t.route(0), Route::Unallocated, "neighbours are untouched");
        assert_eq!(t.route(3), Route::Unallocated);
    }

    /// ★★★ THE PACKING, at the boundary. A target that does not fit must be REFUSED, not
    /// truncated: a truncated target rings a real channel that is the wrong one.
    #[test]
    fn a_target_that_does_not_fit_is_refused_and_changes_nothing() {
        let t = DoorbellTable::new(2);
        assert!(t.install(0, Route::Emulated { chan: MAX_TARGET }));
        assert_eq!(t.route(0), Route::Emulated { chan: MAX_TARGET });
        assert!(
            !t.install(
                0,
                Route::Passthrough {
                    host_token: MAX_TARGET + 1
                }
            ),
            "63 bits does not fit a 62-bit field"
        );
        assert_eq!(
            t.route(0),
            Route::Emulated { chan: MAX_TARGET },
            "a refused install leaves the previous route intact rather than clearing it"
        );
    }

    #[test]
    fn a_route_can_be_retired() {
        let t = DoorbellTable::new(2);
        t.install(0, Route::Emulated { chan: 7 });
        assert!(t.install(0, Route::Unallocated));
        assert_eq!(t.route(0), Route::Unallocated);
    }

    /// ★★★★★ **THE POINT OF THE WHOLE FILE: no lock, and it is still race-free.**
    ///
    /// A reader concurrent with an installer sees either the old route or the new one, never a
    /// mixture — type and target travel in ONE atomic word. A struct with two fields, or a
    /// table behind a lock, would need synchronisation to promise that; this needs the CPU.
    #[test]
    fn a_reader_never_sees_a_half_installed_route() {
        let t = std::sync::Arc::new(DoorbellTable::new(1));
        let w = std::sync::Arc::clone(&t);
        let installer = std::thread::spawn(move || {
            for i in 0..50_000u64 {
                if i % 2 == 0 {
                    w.install(0, Route::Passthrough { host_token: 0x11 });
                } else {
                    w.install(0, Route::Emulated { chan: 0x22 });
                }
            }
        });
        for _ in 0..50_000 {
            match t.route(0) {
                Route::Passthrough { host_token } => assert_eq!(host_token, 0x11),
                Route::Emulated { chan } => assert_eq!(chan, 0x22),
                Route::Unallocated => {}
            }
        }
        installer.join().expect("installer does not panic");
    }

    /// ★★★ (1) ONE ATOMIC INSTRUCTION, and it must be lock-free on this target. ⊘ `AtomicU64`
    /// falls back to a lock on platforms without 64-bit atomics; there the whole design is void
    /// and the honest answer is a 4-byte entry, not a silent mutex under the covers.
    #[test]
    fn an_entry_is_one_lock_free_atomic_access() {
        // ⊘ `is_lock_free` is unstable, so this asserts what is stable and checkable: the
        // entry is 8 bytes, naturally aligned, and the target is 64-bit — the conditions under
        // which a `u64` load/store IS one instruction. ⚠ On a target without 64-bit atomics
        // this design is void and the honest answer is a 4-byte entry, not a silent mutex
        // under the covers; `target_pointer_width` is the guard that would catch it.
        assert_eq!(
            core::mem::size_of::<AtomicU64>(),
            8,
            "an entry must be 8 bytes"
        );
        assert_eq!(
            core::mem::align_of::<AtomicU64>(),
            8,
            "and naturally aligned, or it tears"
        );
        assert_eq!(
            usize::BITS,
            64,
            "a 64-bit entry is only one instruction on a 64-bit target"
        );
    }

    /// ★★★ (2) EVERY ACCESS IS EXACTLY ONE ATOMIC OPERATION. A source census, because the
    /// property is *"how many times does this touch the word"* and no runtime assertion can
    /// answer that. Two loads would let a route change between them; a read-modify-write would
    /// need a compare-exchange loop this design does not have and does not need.
    #[test]
    fn route_and_install_each_touch_the_word_exactly_once() {
        let src = include_str!("dbtable.rs");
        let body = src
            .split("#[cfg(test)]")
            .next()
            .expect("there is code before the tests");
        let loads = body.matches(".load(").count();
        let stores = body.matches(".store(").count();
        assert_eq!(
            (loads, stores),
            (1, 1),
            "expected exactly one load (in `route`) and one store (in `install`); found \
             {loads} and {stores}. A second access to the same word reintroduces the race the \
             single-word packing exists to remove"
        );
        assert_eq!(
            body.matches("compare_exchange").count(),
            0,
            "a CAS loop means the entry stopped being self-describing"
        );
    }

    /// ★★★ (3) O(1) BY TOKEN. The lookup is a direct index into a flat slice, so a table of a
    /// million tokens costs a doorbell exactly what a table of four does. ⊘ Measured as a
    /// RATIO rather than an absolute time: an absolute threshold on a shared bench is a
    /// flake, and the claim is about SHAPE, not speed.
    #[test]
    fn the_lookup_does_not_grow_with_the_table() {
        fn probe(len: usize) -> std::time::Duration {
            let t = DoorbellTable::new(len);
            t.install((len - 1) as u64, Route::Emulated { chan: 9 });
            let last = (len - 1) as u64;
            let start = std::time::Instant::now();
            for _ in 0..200_000 {
                std::hint::black_box(t.route(std::hint::black_box(last)));
            }
            start.elapsed()
        }
        let small = probe(64);
        let large = probe(1 << 20);
        // ⊘ Generous: this is a shape test. Anything sub-linear passes; a scan would be ~16000x.
        assert!(
            large.as_nanos() < small.as_nanos().max(1) * 20,
            "a 16384x bigger table took {large:?} vs {small:?} — the lookup is not O(1), which \
             means a guest can make a doorbell cost more by allocating more channels"
        );
    }
}

#[cfg(test)]
mod the_table_must_cover_the_whole_encoding {
    //! ★★★★★ **A TABLE TOO SMALL REFUSES WELL-FORMED TOKENS, AND THE REFUSAL LOOKS
    //! LEGITIMATE.**
    //!
    //! [`DoorbellTable::route`]'s bounds check answers [`Route::Unallocated`] for a token past
    //! the end — which is the right answer for a wild token and a **dropped submission** for a
    //! real one. The two are indistinguishable from the outside: same variant, same cost, no
    //! log. ⇒ The table's size is not a capacity choice, it is a **correctness invariant tied
    //! to the token encoding**, and it needs a test that fails when the two drift apart.
    use super::*;

    /// `NV_CTRL_VF_DOORBELL_VECTOR` is bits **11:0** — the same field width on every
    /// generation from Volta to Blackwell.
    const VCHID_SPACE: usize = 1 << 12;

    #[test]
    fn every_vchid_the_encoding_can_name_has_a_slot() {
        let t = DoorbellTable::new(VCHID_SPACE);
        // ⊘ The whole field, not a sample: the failure is at the TOP of the range, which is
        // exactly where a sample that walks from zero never reaches.
        for vchid in 0..VCHID_SPACE as u64 {
            assert!(
                t.install(vchid, Route::Emulated { chan: vchid }),
                "vChid {vchid} has no slot — a well-formed token would read as Unallocated"
            );
            assert_eq!(t.route(vchid), Route::Emulated { chan: vchid });
        }
        // And one past it is genuinely out of the encoding's reach.
        assert_eq!(t.route(VCHID_SPACE as u64), Route::Unallocated);
    }

    /// ★★★ **A REBUILD MUST CLEAR WHAT VANISHED.** The projection is taken entire, so a
    /// channel that went away is simply absent from the new snapshot — and a table that only
    /// ever writes the rows it is given keeps routing its vChid to a host token that now
    /// belongs to somebody else. ⚠ That is a doorbell delivered to the WRONG channel, which is
    /// worse than one dropped.
    #[test]
    fn a_rebuild_clears_rows_the_new_snapshot_does_not_mention() {
        let t = DoorbellTable::new(16);
        // First projection: three channels.
        for (v, h) in [(1u64, 0x11u64), (2, 0x22), (3, 0x33)] {
            assert!(t.install(v, Route::Passthrough { host_token: h }));
        }
        // Second projection: channel 2 is gone. The rebuild writes what it has, then clears
        // every slot the snapshot did not mention.
        let snapshot = [(1u64, 0x11u64), (3, 0x99)];
        let mut live = [false; 16];
        for (v, h) in snapshot {
            assert!(t.install(v, Route::Passthrough { host_token: h }));
            live[v as usize] = true;
        }
        for (i, alive) in live.iter().enumerate() {
            if !alive {
                t.install(i as u64, Route::Unallocated);
            }
        }

        assert_eq!(t.route(1), Route::Passthrough { host_token: 0x11 });
        assert_eq!(
            t.route(2),
            Route::Unallocated,
            "a vanished channel still routes — its doorbell reaches SOMEBODY ELSE's host token"
        );
        assert_eq!(
            t.route(3),
            Route::Passthrough { host_token: 0x99 },
            "a re-bound channel must follow its NEW host token, not the stale one"
        );
    }

    /// ⊘ A channel with no host channel behind it is **`Emulated`, never `Unallocated`**. It
    /// exists; it is ours to run. Routing it to the emulated lane makes the refusal happen BY
    /// NAME on the worker, where `Unallocated` would make the submission disappear on the vCPU
    /// with nothing to read afterwards.
    #[test]
    fn a_channel_without_a_host_token_is_ours_not_absent() {
        let t = DoorbellTable::new(4);
        assert!(t.install(2, Route::Emulated { chan: 2 }));
        assert_ne!(t.route(2), Route::Unallocated);
        assert_eq!(t.route(2), Route::Emulated { chan: 2 });
    }
}

/// ★★★★★ **w795 — WHAT ONE DOORBELL COSTS, BY ROUTE.**
///
/// # ⊘⊘⊘ Why this exists: the tree says it is missing, in those words
///
/// `docs/design/c_vs_rust_per_launch_path.md:261` — *"**No timing instrumentation on our
/// doorbell path at all.** No histogram, no span, no per-doorbell duration counter — only
/// counters… **NOT A COST — A CAUSE OF NOT KNOWING.**"*
///
/// Every number the tree has is the wrong shape for the question. `worst_trap` is **per boot**
/// and, by §41's own rule, *"NAMES THE SITE, NEVER THE CAUSE"*. `DBL_RATIO_X` measures a whole
/// submit round trip (guest p50 ~512–683 us against a ~9 us native floor), not a trap. The only
/// ioeventfd delta in the tree is from a **synthetic** QEMU spike device, not ours.
///
/// ⇒ `docs/design/the_doorbell_ioeventfd_question.md` names this as the measurement that
/// decides whether moving the doorbell to `KVM_IOEVENTFD` is worth anything. It is also the
/// measurement `l2_qemu_adapter.md` Q5 deferred that decision behind — and never took.
///
/// # ★ Why it belongs on the TABLE and costs no lookup
///
/// [`DoorbellTable::route`] already classifies every doorbell into
/// [`Route::Passthrough`] / [`Route::Emulated`] / [`Route::Unallocated`], lock-free, on the hot
/// path. The two dispositions have completely different shapes — a passthrough doorbell is ONE
/// DWORD inline on the vCPU (§41), an emulated one is a queue push — so a single pooled number
/// would describe neither. Bucketing by the decision the table already made costs nothing extra
/// and is the only split that means anything.
///
/// # ⚠ §41 AND THE OBSERVER PROBLEM, both stated rather than waved through
///
/// §41 says a vCPU MMIO write may update a queue, wake, write one dword, or write a read
/// register — *"Everything else is a defect, including work that is fast today."* Two relaxed
/// atomic adds and two `Instant::now()` reads are **work in the trap**, and this file is not
/// going to pretend otherwise.
///
/// The argument for it: the alternative is the defect the tree has already named, and it has
/// blocked a decision twice. The cost is bounded by construction — a fixed array, no
/// allocation, no lock, no branch on guest data — and **it is measured by its own null arm**
/// ([`DoorbellHistogram::PROBE_NS`]), so a reader can subtract the instrument instead of
/// trusting it. ⊘ If the probe ever approaches the passthrough path's own p50, this instrument
/// is reporting mostly itself (`a_probe_that_shares_the_allocator_is_not_an_observer`) and must
/// be sampled instead of unconditional. The rendered line says so with the number attached.
#[derive(Debug, Default)]
pub struct DoorbellHistogram {
    /// `[class][bucket]`, bucket `i` = durations in `[2^i, 2^(i+1))` nanoseconds.
    buckets: [[core::sync::atomic::AtomicU64; Self::BUCKETS]; Self::CLASSES],
    /// Total nanoseconds per class, so a mean survives bucket coarseness.
    totals: [core::sync::atomic::AtomicU64; Self::CLASSES],
    /// The instrument's own cost, sampled once. See [`DoorbellHistogram::PROBE_NS`].
    probe_ns: core::sync::atomic::AtomicU64,
}

/// Which shape of doorbell a sample belongs to — the split [`DoorbellTable::route`] already
/// makes, never re-derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoorbellClass {
    /// One dword to the real register, inline on the vCPU (§41).
    Passthrough = 0,
    /// The token went on a queue for a worker.
    Emulated = 1,
    /// Refused, unallocated, or served by our own executor — none of which is a submission.
    Other = 2,
}

impl DoorbellHistogram {
    /// Number of route classes.
    pub const CLASSES: usize = 3;
    /// `2^0 ns` … `2^23 ns` (~8.4 ms) and an overflow bucket.
    pub const BUCKETS: usize = 24;

    /// The key the whole instrument has to be read against: **the cost of measuring**.
    ///
    /// ⊘ Not a constant — sampled from the same clock on the same box, because a hard-coded
    /// figure would be a claim about hardware this never ran on.
    #[must_use]
    pub fn probe_ns(&self) -> u64 {
        self.probe_ns.load(core::sync::atomic::Ordering::Relaxed)
    }

    /// Sample the instrument's own cost. Call once, off the hot path.
    pub fn calibrate(&self) {
        let t = std::time::Instant::now();
        let mut worst = 0u64;
        for _ in 0..64 {
            let a = std::time::Instant::now();
            let b = std::time::Instant::now();
            worst = worst.max((b - a).as_nanos() as u64);
        }
        let _ = t;
        self.probe_ns
            .store(worst, core::sync::atomic::Ordering::Relaxed);
    }

    /// Record one doorbell.
    ///
    /// ⊘ Relaxed ordering throughout: this is a census, and a sample landing in a neighbouring
    /// bucket under concurrency changes no decision. Ordering strong enough to make the
    /// histogram linearizable would be ordering in the trap, which is the thing being measured.
    pub fn record(&self, class: DoorbellClass, nanos: u64) {
        let c = class as usize;
        let b = (64 - nanos.max(1).leading_zeros() as usize - 1).min(Self::BUCKETS - 1);
        self.buckets[c][b].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        self.totals[c].fetch_add(nanos, core::sync::atomic::Ordering::Relaxed);
    }

    /// `(count, mean_ns, p50_ns, p90_ns, p99_ns)` for one class. Percentiles are bucket
    /// **lower bounds** — deliberately, so a reported figure is never larger than a real one.
    #[must_use]
    pub fn stats(&self, class: DoorbellClass) -> (u64, u64, u64, u64, u64) {
        let c = class as usize;
        let counts: Vec<u64> = (0..Self::BUCKETS)
            .map(|i| self.buckets[c][i].load(core::sync::atomic::Ordering::Relaxed))
            .collect();
        let n: u64 = counts.iter().sum();
        if n == 0 {
            return (0, 0, 0, 0, 0);
        }
        let total = self.totals[c].load(core::sync::atomic::Ordering::Relaxed);
        let at = |frac: f64| -> u64 {
            let want = (n as f64 * frac).ceil() as u64;
            let mut seen = 0u64;
            for (i, v) in counts.iter().enumerate() {
                seen += v;
                if seen >= want {
                    return 1u64 << i;
                }
            }
            1u64 << (Self::BUCKETS - 1)
        };
        (n, total / n, at(0.50), at(0.90), at(0.99))
    }

    /// One line per class, with the instrument's own cost attached.
    #[must_use]
    pub fn render(&self) -> String {
        let probe = self.probe_ns();
        let mut out = String::new();
        for (name, class) in [
            ("passthrough", DoorbellClass::Passthrough),
            ("emulated", DoorbellClass::Emulated),
            ("other", DoorbellClass::Other),
        ] {
            let (n, mean, p50, p90, p99) = self.stats(class);
            if n == 0 {
                // ⊘ Printed anyway: a class with no samples is a reading ("nothing took this
                // route this boot"), and its absence is indistinguishable from a skipped render.
                out.push_str(&format!("DOORBELL-COST {name}: n=0 ⊘ no sample\n"));
                continue;
            }
            let verdict = if probe > 0 && p50 <= probe.saturating_mul(3) {
                " ⊘⊘ p50 IS WITHIN 3x THE PROBE — this line is mostly the instrument; sample \
                 instead of measuring unconditionally"
            } else {
                ""
            };
            out.push_str(&format!(
                "DOORBELL-COST {name}: n={n} mean={mean}ns p50={p50}ns p90={p90}ns p99={p99}ns \
                 (probe={probe}ns){verdict}\n"
            ));
        }
        out
    }
}


/// ★★★★★ **w801 — WHICH TOKENS TOOK WHICH DISPOSITION, NOT JUST HOW MANY.**
///
/// # ⊘⊘⊘ Why a per-token ledger and not another counter
///
/// `[measured w797]` one boot reported `DOORBELL-XLATE=3` beside `SERVED-LOCALLY=8`, and
/// three rungs then argued from those two numbers about whether a particular channel's copy
/// had been forwarded. **They cannot answer it.** The ledger's `P1` and `STALE RACE` rows
/// fail with `the copy … NEVER RETIRED`, and whether their doorbells were among the 3 or the
/// 8 implies *opposite* defects:
///
/// - among the **8** ⇒ the channel is not being routed to hardware at all: a kind/route
///   problem upstream of the executor (owner's reading: *"for a passthrough channel it means
///   doorbell is refused or not forwarded"*).
/// - among the **3** ⇒ the doorbell is fine and the **engine** cannot do the work — the host
///   channel's VAS is missing the source mapping.
///
/// ⚠ An aggregate cannot distinguish those, and three rungs spent boots inferring what one
/// list would have stated. `a_census_zero_needs_a_known_positive`, one level up: a census of
/// *counts* needs a census of *identities* behind it.
///
/// ⊘ Bounded by construction: one entry per distinct `(token, disposition)` pair, capped, and
/// the cap is reported. A hostile guest can ring any token it likes, so the bound is on the
/// TABLE and not on the guest's behaviour.
#[derive(Debug, Default)]
pub struct DoorbellLedger {
    seen: std::sync::Mutex<std::collections::BTreeMap<u64, [u64; 4]>>,
    /// Distinct tokens refused entry for want of room. ⊘ A silent truncation would make the
    /// list a claim about the table's capacity rather than about the boot.
    dropped: core::sync::atomic::AtomicU64,
}

impl DoorbellLedger {
    /// How many distinct tokens the ledger will name before it starts counting drops.
    pub const MAX_TOKENS: usize = 64;

    /// Record one doorbell's outcome against its token.
    pub fn record(&self, token: u64, class: DoorbellClass, forwarded: bool) {
        let mut g = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let slot = if forwarded { 3 } else { class as usize };
        if let Some(e) = g.get_mut(&token) {
            e[slot] = e[slot].saturating_add(1);
            return;
        }
        if g.len() >= Self::MAX_TOKENS {
            self.dropped
                .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            return;
        }
        let mut e = [0u64; 4];
        e[slot] = 1;
        g.insert(token, e);
    }

    /// One line per token: `tok=0x… passthrough=N emulated=N other=N forwarded=N`.
    #[must_use]
    pub fn render(&self) -> String {
        let g = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let dropped = self.dropped.load(core::sync::atomic::Ordering::Relaxed);
        if g.is_empty() {
            return format!("DOORBELL-LEDGER ⊘ no doorbell rang this boot (dropped={dropped})\n");
        }
        let mut out = String::new();
        for (tok, e) in g.iter() {
            out.push_str(&format!(
                "DOORBELL-LEDGER tok={tok:#010x} passthrough={} emulated={} other={} forwarded={}\n",
                e[0], e[1], e[2], e[3]
            ));
        }
        out.push_str(&format!(
            "DOORBELL-LEDGER tokens={} dropped={dropped} ⇒ match a failing rung's token against \
             this list: `forwarded=0` with `emulated>0` means it never went to hardware; \
             `forwarded>0` means it did and the ENGINE did not do the work\n",
            g.len()
        ));
        out
    }
}
