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
    assert!(core::mem::size_of::<AtomicU64>() == 8, "an entry must fit ONE atomic access");
    assert!(core::mem::align_of::<AtomicU64>() == 8, "and be naturally aligned, or it is torn");
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
            !t.install(0, Route::Passthrough { host_token: MAX_TARGET + 1 }),
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
        assert_eq!(core::mem::size_of::<AtomicU64>(), 8, "an entry must be 8 bytes");
        assert_eq!(core::mem::align_of::<AtomicU64>(), 8, "and naturally aligned, or it tears");
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
