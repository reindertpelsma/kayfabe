//! GPU-free tests of the two "do not look again yet" rules of the USER-ring relay turn
//! (`kf_chan::user_lazy`): the cached guest `GP_PUT` (A) and the interval-gated observation of the private
//! `USERD` (B). Deterministic, no clock, no environment; exhaustive over small state spaces.
//!
//! Layout: (A) a breadth-first exploration of EVERY reachable state of a guest-writer / doorbell / relay-turn model
//! (ring of `R` slots, bounded total entries, ring migrations to a LOWER PUT), checking the invariants on every
//! transition and a "drain from here" liveness check from every state; (B) credits, `SyncGate` / `Marks` monotonicity,
//! the forced-observation liveness model with a never-firing periodic interval, and the N = 1 identity over a
//! randomised schedule. The relay turn is transliterated from `Pump::turn_body` (read PUT, `put == cursor` => idle,
//! else relay the entry at the cursor).
//!
//! MUTATION CHECK (run by hand, see the campaign report): each rule was broken once in `user_lazy.rs` and the named
//! test failed. Rules: `select` ignores the generation; `select` returns the cache when `put == cursor`;
//! `must_observe` ignores `prev_waiting`; `must_observe` ignores the credit floor; `Marks` trusts a stale word (not a
//! `user_lazy` rule: guarded by `stale_reference_never_releases_a_gate_early`).
use kf_chan::user_gate::{Gate, Marks, SyncGate};
use kf_chan::user_lazy::{CREDIT_FLOOR, OBSERVE_INTERVAL, PutCache, must_observe};
use kf_chan::user_scan::Fetch;
use std::collections::{HashSet, VecDeque};

// ------------------------------------------------------------------------------------------------
// (A) the cached guest GP_PUT

/// Guest ring slots; one slot is always empty (the hardware ring rule).
const R: u32 = 4;
/// Total entries the guest may ever write per generation in the exploration (forces ring wrap-around: > R).
const MAX_ENTRIES: u32 = 7;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Sys {
    generation: u64,
    /// Entries physically written by the guest this generation (absolute count).
    written: u32,
    /// Entries covered by the `GP_PUT` the guest has stored (absolute count).
    put: u32,
    /// Absolute `put` value the guest has rung the doorbell for.
    rung: u32,
    /// Entries the relay has relayed this generation (absolute); the cursor is `consumed % R`.
    consumed: u32,
    /// The relay's worker is woken (a doorbell arrived, or it is mid-batch) and will take a turn.
    woken: bool,
    cache: PutCache,
    enabled: bool,
    /// Generations already used (for the `Migrate` bound).
    migrations: u8,
}

#[derive(Clone, Copy, Debug)]
enum Ev {
    /// The guest writes the next entry into its ring (needs a free slot).
    Write,
    /// The guest stores `GP_PUT = written`.
    SetPut,
    /// The guest rings the doorbell for the PUT it stored.
    Ring,
    /// The relay takes a turn (only when woken).
    Turn,
    /// The guest switches to a new, migrated ring whose PUT is `j` (possibly LOWER than before), entries 0..j written.
    Migrate(u32),
}

fn dist(from: u32, to: u32) -> u32 {
    to.wrapping_sub(from) % R
}

impl Sys {
    fn new(enabled: bool) -> Self {
        Sys { generation: 1, written: 0, put: 0, rung: 0, consumed: 0, woken: false, cache: PutCache::default(), enabled, migrations: 0 }
    }
    fn cursor(&self) -> u32 {
        self.consumed % R
    }
    /// One relay turn; returns Err(why) when an invariant breaks. Mirrors `Pump::turn_body`'s PUT handling.
    fn turn(&mut self) -> Result<(), String> {
        let cursor = self.cursor();
        let sel = self.cache.select(self.enabled, self.generation, cursor);
        let (put_m, fresh) = match sel {
            Some(p) => (p, false),
            None => (self.put % R, true),
        };
        self.cache.record(self.generation, put_m);
        // I8: the PUT in hand is never beyond the real PUT, measured from the cursor.
        if dist(cursor, put_m) > dist(cursor, self.put % R) {
            return Err(format!("I8: cached PUT {put_m} beyond real PUT {} (cursor {cursor})", self.put % R));
        }
        if put_m == cursor {
            if !fresh {
                return Err("idle decision taken on a CACHED PUT".into());
            }
            self.woken = false;
            return Ok(());
        }
        // The entry at the cursor must be one the guest published before the PUT in hand was read.
        if self.consumed >= self.put {
            return Err(format!("read an entry at/after the loaded PUT (consumed {} put {})", self.consumed, self.put));
        }
        self.consumed += 1;
        Ok(())
    }
    fn apply(&mut self, e: Ev) -> Result<bool, String> {
        match e {
            Ev::Write => {
                if self.written >= MAX_ENTRIES || self.written - self.consumed >= R - 1 {
                    return Ok(false);
                }
                self.written += 1;
            }
            Ev::SetPut => {
                if self.put == self.written {
                    return Ok(false);
                }
                self.put = self.written;
            }
            Ev::Ring => {
                if self.rung == self.put && self.woken {
                    return Ok(false);
                }
                self.rung = self.put;
                self.woken = true;
            }
            Ev::Turn => {
                if !self.woken {
                    return Ok(false);
                }
                self.turn()?;
            }
            Ev::Migrate(j) => {
                if self.migrations >= 2 || self.written == 0 {
                    return Ok(false);
                }
                self.migrations += 1;
                self.generation += 1;
                self.consumed = 0;
                self.written = j;
                self.put = j;
                self.rung = j;
                self.woken = true; // the migration trigger's own doorbell
                // the relay's cursor is re-based by the migration; the cache is NOT touched here (the generation key must suffice)
            }
        }
        Ok(true)
    }
    /// From here: deliver the pending PUT store and doorbell, then let the relay run to quiescence; it must have
    /// relayed everything the guest published (no lost wakeup, nothing duplicated).
    fn drain_and_check(&self) -> Result<(), String> {
        let mut s = self.clone();
        if s.put != s.written {
            s.put = s.written;
        }
        if s.rung != s.put {
            s.rung = s.put;
            s.woken = true;
        }
        for _ in 0..(4 * MAX_ENTRIES + 8) {
            if !s.woken {
                break;
            }
            s.turn()?;
        }
        if s.woken {
            return Err("relay still woken after the bounded drain (livelock)".into());
        }
        if s.consumed != s.put {
            return Err(format!("LOST WAKEUP: relay idle with consumed {} < published {}", s.consumed, s.put));
        }
        Ok(())
    }
}

fn explore(enabled: bool) -> Result<usize, String> {
    let mut seen: HashSet<Sys> = HashSet::new();
    let mut q: VecDeque<Sys> = VecDeque::new();
    let s0 = Sys::new(enabled);
    seen.insert(s0.clone());
    q.push_back(s0);
    let mut events = vec![Ev::Write, Ev::SetPut, Ev::Ring, Ev::Turn];
    events.extend((0..R - 1).map(Ev::Migrate));
    while let Some(s) = q.pop_front() {
        s.drain_and_check().map_err(|e| format!("{e} @ {s:?}"))?;
        for &e in &events {
            let mut n = s.clone();
            match n.apply(e) {
                Ok(true) => {
                    if seen.insert(n.clone()) {
                        q.push_back(n);
                    }
                }
                Ok(false) => {}
                Err(why) => return Err(format!("{why} after {e:?} @ {s:?}")),
            }
        }
    }
    Ok(seen.len())
}

#[test]
fn put_cache_every_reachable_state_keeps_i8_order_and_the_doorbell_contract() {
    let n = explore(true).unwrap();
    eprintln!("put_cache exploration: {n} reachable states");
    assert!(n > 500, "the exploration must not be vacuous (states = {n})");
}

#[test]
fn put_cache_disabled_is_the_reference_behaviour() {
    explore(false).unwrap();
}

#[test]
fn a_cached_put_is_never_used_when_the_cursor_has_reached_it() {
    let mut c = PutCache::default();
    c.record(1, 5);
    assert_eq!(c.select(true, 1, 5), None, "cursor == cached PUT forces a fresh read");
    assert_eq!(c.select(true, 1, 3), Some(5));
    assert_eq!(c.select(false, 1, 3), None, "disabled never selects");
}

#[test]
fn a_ring_generation_switch_invalidates_the_cache_even_for_a_lower_put() {
    let mut c = PutCache::default();
    c.record(1, 3);
    assert_eq!(c.select(true, 1, 0), Some(3));
    assert_eq!(c.select(true, 2, 0), None, "new generation: never the old PUT");
    c.record(2, 1); // the migrated ring's PUT is LOWER than the old one
    assert_eq!(c.select(true, 2, 0), Some(1));
    c.invalidate();
    assert_eq!(c.select(true, 2, 0), None);
}

#[test]
fn wrap_around_is_handled_in_modular_arithmetic() {
    // cursor 3, cached PUT 1 (the PUT wrapped): entries 3, 0 are valid, the cache is used until the cursor reaches 1
    let mut c = PutCache::default();
    c.record(7, 1);
    assert_eq!(c.select(true, 7, 3), Some(1));
    assert_eq!(c.select(true, 7, 0), Some(1));
    assert_eq!(c.select(true, 7, 1), None);
}

// ------------------------------------------------------------------------------------------------
// (B) the interval-gated observation of the private USERD

#[test]
fn must_observe_is_forced_by_waiting_open_segment_low_credit_and_interval_one() {
    for turn in 0..100u64 {
        for interval in [1u64, 2, 16, 4096] {
            assert!(must_observe(turn, interval, true, 10_000, false), "prev_waiting forces a look (liveness)");
            assert!(must_observe(turn, interval, false, 10_000, true), "an open segment forces a look");
            assert!(must_observe(turn, interval, false, CREDIT_FLOOR - 1, false), "low credits force a look (no deadlock on a full ring)");
        }
        assert!(must_observe(turn, 1, false, 10_000, false), "interval 1 = today's behaviour: every turn");
        assert!(must_observe(turn, 0, false, 10_000, false), "interval 0 is treated as 1");
    }
}

#[test]
fn must_observe_without_a_reason_looks_once_per_interval() {
    for interval in [2u64, 4, OBSERVE_INTERVAL, 64] {
        let looks = (1..=1024u64).filter(|&t| must_observe(t, interval, false, 10_000, false)).count() as u64;
        assert_eq!(looks, 1024 / interval, "interval {interval}");
    }
}

#[test]
fn stale_get_never_yields_more_credits_than_the_real_ring_has() {
    for entries in [4u32, 8, 16] {
        for issued in 0..entries - 1 {
            for real in 0..=issued {
                for stale in 0..=real {
                    let mk = |get: u32| {
                        let mut f = Fetch::new(entries).unwrap();
                        for _ in 0..issued {
                            f.publish(None).unwrap();
                        }
                        f.observe(get % entries).unwrap();
                        f.available()
                    };
                    let truth = entries - 1 - issued + real; // free slots given the pops that REALLY happened
                    assert!(mk(stale) <= mk(real), "entries {entries} issued {issued} real {real} stale {stale}");
                    assert!(mk(real) <= truth && mk(stale) <= truth, "credits above the real ring's free slots: entries {entries} issued {issued} real {real} stale {stale}");
                }
            }
        }
    }
}

#[test]
fn stale_reference_never_releases_a_gate_early() {
    // SyncGate::entry is monotone in `processed`: Proceed on a stale count implies Proceed on the real (larger) one.
    let raw = 1u64 << 63;
    for witness in 1..=4u64 {
        for real in 0..=5u64 {
            for stale in 0..=real {
                let mut a = SyncGate::default();
                let mut b = SyncGate::default();
                assert_eq!(a.entry(raw, 1, 3, stale), Gate::PublishBarrier);
                assert_eq!(b.entry(raw, 1, 3, real), Gate::PublishBarrier);
                a.published(witness);
                b.published(witness);
                let ga = a.entry(raw, 1, 3, stale);
                let gb = b.entry(raw, 1, 3, real);
                if ga == Gate::Proceed {
                    assert_eq!(gb, Gate::Proceed, "stale released early: witness {witness} stale {stale} real {real}");
                    assert!(stale >= witness);
                }
                if gb == Gate::Wait {
                    assert_eq!(ga, Gate::Wait);
                }
            }
        }
    }
    // A REFERENCE word naming a marker that was never issued (garbage, or the guest's own SET_REFERENCE) implies NO progress.
    for issued in 0..=4u64 {
        let mut m = Marks::default();
        for _ in 0..issued {
            m.issue().unwrap();
        }
        for future in issued + 1..=issued + 6 {
            assert_eq!(m.peek(Marks::value(future)), m.seen(), "a marker not yet issued must not count (issued {issued}, word names {future})");
        }
        assert_eq!(m.peek(0x1234_5678), m.seen(), "an untagged word implies no progress");
    }
    // Marks::peek of a stale REFERENCE word never implies more progress than a fresher word.
    for issued in 1..=6u64 {
        for real in 0..=issued {
            for stale in 0..=real {
                let mut m = Marks::default();
                for _ in 0..issued {
                    m.issue().unwrap();
                }
                let word = |n: u64| if n == 0 { 0 } else { Marks::value(n) };
                assert!(m.peek(word(stale)) <= m.peek(word(real)));
                assert!(m.peek(word(real)) <= issued);
            }
        }
    }
}

/// Liveness + safety of a SYNC_WAIT-gated stream with the observation interval set so large that the periodic look
/// NEVER fires: the forced look (prev_waiting) alone must release every gate, and none may be released early.
#[derive(Clone)]
struct GateSys {
    entries: Vec<bool>, // guest entries: SYNC_WAIT or not
    k: usize,
    gate: SyncGate,
    marks: Marks,
    real_processed: u64,
    witness_of: Vec<Option<u64>>,
    prev_waiting: bool,
    turn_no: u64,
    interval: u64,
    reads: Vec<usize>,
}

impl GateSys {
    fn new(entries: Vec<bool>, interval: u64) -> Self {
        let n = entries.len();
        GateSys { entries, k: 0, gate: SyncGate::default(), marks: Marks::default(), real_processed: 0, witness_of: vec![None; n], prev_waiting: true, turn_no: 0, interval, reads: vec![] }
    }
    fn env_process_marker(&mut self) -> bool {
        if self.real_processed < self.marks.issued() {
            self.real_processed += 1;
            return true;
        }
        false
    }
    fn turn(&mut self) -> Result<bool, String> {
        if self.k >= self.entries.len() {
            return Ok(false);
        }
        self.turn_no += 1;
        if must_observe(self.turn_no, self.interval, self.prev_waiting, 10_000, false) {
            let word = if self.real_processed == 0 { 0 } else { Marks::value(self.real_processed) };
            self.marks.observe(word);
        }
        let raw = if self.entries[self.k] { 1u64 << 63 } else { 0 };
        match self.gate.entry(raw, 1, self.k as u32, self.marks.seen()) {
            Gate::Proceed => {
                if self.entries[self.k] {
                    let w = self.witness_of[self.k].ok_or("sync entry read without a published gate")?;
                    if self.real_processed < w {
                        return Err(format!("entry {} read before its gate marker ran (real {} < witness {w})", self.k, self.real_processed));
                    }
                }
                self.reads.push(self.k);
                self.k += 1;
                self.prev_waiting = false;
            }
            Gate::PublishBarrier => {
                let (count, _) = self.marks.issue().map_err(str::to_owned)?;
                self.gate.published(count);
                self.witness_of[self.k] = Some(count);
                self.prev_waiting = true;
            }
            Gate::Wait => self.prev_waiting = true,
        }
        Ok(true)
    }
}

fn gate_explore(entries: Vec<bool>, interval: u64, depth: usize) {
    fn rec(s: &GateSys, depth: usize, interval: u64) {
        // liveness from every state: the environment finishes all markers, the relay keeps turning
        let mut t = s.clone();
        for _ in 0..(6 * t.entries.len() + 6) {
            while t.env_process_marker() {}
            if !t.turn().expect("safety") {
                break;
            }
        }
        assert_eq!(t.reads, (0..t.entries.len()).collect::<Vec<_>>(), "liveness: every entry read, in order, once (interval {interval})");
        if depth == 0 {
            return;
        }
        let mut a = s.clone();
        if a.env_process_marker() {
            rec(&a, depth - 1, interval);
        }
        let mut b = s.clone();
        if b.turn().expect("safety") {
            rec(&b, depth - 1, interval);
        }
    }
    rec(&GateSys::new(entries, interval), depth, interval);
}

#[test]
fn sync_wait_stream_is_safe_and_live_even_when_the_periodic_look_never_fires() {
    gate_explore(vec![true, false, true], 4096, 9);
    gate_explore(vec![true, true, false, true], 1 << 40, 8);
}

#[test]
fn sync_wait_stream_is_safe_and_live_with_the_production_interval() {
    gate_explore(vec![false, true, true, false], OBSERVE_INTERVAL, 9);
}

/// N = 1 and the cache disabled are bit-identical to the reference relay (one fresh PUT read and one observation per
/// turn, the same entries relayed in the same order) over a randomised schedule.
#[test]
fn n_equals_one_is_bit_identical_to_the_reference_relay() {
    let mut rng = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    for _ in 0..2000 {
        let mut lazy = Sys::new(false);
        let mut refr = Sys::new(false);
        let (mut turns_l, mut turns_r) = (0u32, 0u32);
        let mut observes = 0u32;
        for step in 0..60u32 {
            let e = match next() % 5 {
                0 => Ev::Write,
                1 => Ev::SetPut,
                2 => Ev::Ring,
                _ => Ev::Turn,
            };
            let a = lazy.apply(e).unwrap();
            let b = refr.apply(e).unwrap();
            assert_eq!(a, b);
            if a && matches!(e, Ev::Turn) {
                turns_l += 1;
                turns_r += 1;
                // N = 1: every turn observes
                assert!(must_observe(u64::from(step) + 1, 1, false, 10_000, false));
                observes += 1;
            }
            assert_eq!(lazy, refr);
        }
        assert_eq!(turns_l, observes);
        assert_eq!(turns_l, turns_r);
        lazy.drain_and_check().unwrap();
    }
}

// Additional review coverage, 2026-10-17. These oracles use absolute sequence
// numbers and descriptor identities, rather than only comparing two Sys values.
fn random_step(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn put_advancing_behind_the_cache_drains_every_identity_before_fresh_idle() {
    let mut s = Sys::new(true);
    let mut identities = Vec::new();
    let mut slots = [None; R as usize];
    for lap in 0..4 {
        // Publish two, relay one, then advance PUT while the cache still covers
        // the second. Absolute counts deliberately cross the four-slot wrap.
        for _ in 0..2 {
            slots[(s.written % R) as usize] = Some(s.written);
            s.written += 1;
        }
        s.put = s.written;
        s.rung = s.put;
        s.woken = true;
        let before = s.consumed;
        assert_eq!(slots[s.cursor() as usize], Some(before));
        s.turn().unwrap();
        identities.push(before);
        let cached = s.cache.select(true, s.generation, s.cursor()).unwrap();
        slots[(s.written % R) as usize] = Some(s.written);
        s.written += 1;
        s.put = s.written;
        s.rung = s.put; // the LAST doorbell; no later wake is supplied
        assert_eq!(s.cache.select(true, s.generation, s.cursor()), Some(cached));
        for _ in 0..2 {
            let cursor = s.cursor();
            let loaded = s.cache.select(true, s.generation, cursor).unwrap_or(s.put % R);
            assert!(dist(cursor, loaded) > 0);
            assert!(dist(cursor, loaded) <= s.put - s.consumed);
            assert_eq!(slots[cursor as usize], Some(s.consumed));
            identities.push(s.consumed);
            s.turn().unwrap();
        }
        assert_eq!(s.consumed, (lap + 1) * 3);
        assert_eq!(s.cache.select(true, s.generation, s.cursor()), None);
        s.turn().unwrap(); // end_batch / idle is now based on a fresh PUT
        assert!(!s.woken);
    }
    assert_eq!(identities, (0..12).collect::<Vec<_>>());
}

#[test]
fn stale_get_is_conservative_across_many_wraps_and_receipts_are_exact() {
    for entries in [4u32, 8, 128] {
        let mut f = Fetch::new(entries).unwrap();
        let (mut issued, mut real, mut observed) = (0u64, 0u64, 0u64);
        let mut rng = 0x1234_9876_5555_aaaau64;
        for _ in 0..100_000 {
            match random_step(&mut rng) % 3 {
                0 if f.available() > 0 => {
                    issued += 1;
                    f.publish(Some((7, issued as u32))).unwrap();
                }
                1 if real < issued => real += 1,
                _ => {
                    // An ordered but arbitrarily stale hardware sample, never
                    // older than the last sample the relay accepted.
                    let sample = observed + random_step(&mut rng) % (real - observed + 1);
                    let receipts = f.observe((sample % u64::from(entries)) as u32).unwrap();
                    let expected: Vec<_> = (observed + 1..=sample).map(|n| (7, n as u32)).collect();
                    assert_eq!(receipts, expected);
                    observed = sample;
                }
            }
            let real_free = u64::from(entries - 1) - (issued - real);
            assert!(u64::from(f.available()) <= real_free);
            assert_eq!(f.fetched(), observed);
            assert_eq!(f.issued(), issued);
        }
        assert!(issued > u64::from(entries) * 10, "must exercise repeated wraps");
    }
}

#[test]
fn a_full_private_ring_recovers_without_any_periodic_observation() {
    let mut f = Fetch::new(128).unwrap();
    for _ in 0..127 {
        f.publish(None).unwrap();
    }
    assert_eq!(f.available(), 0);
    assert!(f.publish(None).is_err());
    // Every low-credit value, including zero, forces a read on a non-periodic
    // turn. At exactly the floor it is safe to defer until another reason.
    for credit in 0..CREDIT_FLOOR {
        assert!(must_observe(1, u64::MAX, false, credit, false));
    }
    assert!(!must_observe(1, u64::MAX, false, CREDIT_FLOOR, false));
    if must_observe(1, u64::MAX, false, f.available(), false) {
        f.observe(127).unwrap();
    }
    assert_eq!(f.available(), 127);
    assert_eq!(f.publish(Some((2, 1))).unwrap(), (127, 0));
    assert!(must_observe(2, u64::MAX, true, f.available(), false));
    assert!(must_observe(3, u64::MAX, false, f.available(), true));
    assert_eq!(f.observe(0).unwrap(), vec![(2, 1)]);
}

#[test]
fn n_one_matches_an_independent_eager_fifo_and_reference_oracle() {
    let mut rng = 0x0ddc_0ffe_e15e_beefu64;
    let mut f = Fetch::new(128).unwrap();
    let mut m = Marks::default();
    let mut pending = VecDeque::new();
    let (mut issued, mut real_get, mut observed_get) = (0u64, 0u64, 0u64);
    let (mut issued_marks, mut real_ref) = (0u64, 0u64);
    for turn in 1..=200_000 {
        let event = random_step(&mut rng);
        if event & 1 != 0 && pending.len() < 127 {
            issued += 1;
            let receipt = (3, issued as u32);
            pending.push_back(receipt);
            f.publish(Some(receipt)).unwrap();
        }
        if event & 2 != 0 && real_get < issued {
            real_get += 1;
        }
        if event & 4 != 0 {
            issued_marks += 1;
            assert_eq!(m.issue().unwrap().0, issued_marks);
        }
        if event & 8 != 0 && real_ref < issued_marks {
            real_ref += 1;
        }
        assert!(must_observe(turn, 1, event & 16 != 0, f.available(), event & 32 != 0));
        let got = f.observe((real_get % 128) as u32).unwrap();
        let expected: Vec<_> = (observed_get..real_get).map(|_| pending.pop_front().unwrap()).collect();
        assert_eq!(got, expected);
        observed_get = real_get;
        // The eager oracle knows absolute hardware progress independently of
        // Marks and Fetch. Receipt equality also checks guest GET write-back.
        assert_eq!(f.available() as usize, 127 - pending.len());
        let word = if real_ref == 0 { 0 } else { Marks::value(real_ref) };
        assert_eq!(m.observe(word), real_ref);
    }
    assert!(issued > 10_000 && real_ref > 65_536, "exercise GET and REFERENCE wraps");
}

#[test]
fn long_sync_wait_streams_with_lazy_observation_never_read_late_bytes_early() {
    for interval in [4, OBSERVE_INTERVAL, 64, u64::MAX] {
        for seed in 1..=12 {
            let mut rng = seed * 0x1234_5678;
            let entries: Vec<_> = (0..2048).map(|_| random_step(&mut rng) % 3 != 0).collect();
            let mut s = GateSys::new(entries, interval);
            for _ in 0..100_000 {
                if random_step(&mut rng) % 4 == 0 {
                    s.env_process_marker();
                }
                if !s.turn().unwrap() {
                    break;
                }
            }
            assert_eq!(s.reads, (0..2048).collect::<Vec<_>>());
        }
    }
}
