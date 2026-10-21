//! ★ 2026-10-17 — **Lazy observe inside the REAL per-entry turn** (`kf_chan::user_turn::Relay` with
//! `Relay::with_lazy`): rule A (cache the guest `GP_PUT` until the cursor reaches it) and rule B (read the private
//! `USERD` only every N-th turn, or when something forces it), with the invalidation rules, the counters and the D6
//! write-back guard. `docs/design/V3_USER_RING_LIFECYCLE_20261016.md`, "Lazy observe (production)".
//!
//! Deterministic, one mechanism per test, over a programmable [`Io`] that records every call the relay makes. The pure
//! predicates have their own tests (`user_lazy_model.rs`, the unit tests in `user_lazy.rs`); the interleaving model, the
//! property tests and the thread stress run the same relay with lazy observe ON (`user_lifecycle_*`).
#![allow(missing_docs, dead_code, clippy::too_many_lines)]

#[path = "lifecycle_support/mod.rs"]
mod support;

use kf_chan::user_gate::Marks;
use kf_chan::user_lazy::{CREDIT_FLOOR, LazyConfig, OBSERVE_INTERVAL};
use kf_chan::user_lifecycle::{Lifecycle, Mode};
use kf_chan::user_loop::Need;
use kf_chan::user_scan::{Fetch, Scan};
use kf_chan::user_turn::{
    Ev, Fail, GpSegment, Io, Phase, PubKind, Relay, Staged, Step, SwView, Turn,
};
use std::collections::HashMap;
use support::{Ge, Kind, ge, sw};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    HwWords,
    WriteGuestGet(u32),
    GpPut,
    Load(u32, bool),
    Epoch,
    ReadPb(u64),
    Entry(PubKind, u64),
    Gate,
    Marker,
}

struct Mock {
    entries: u32,
    ring: Vec<u64>,
    put: u32,
    epoch: u64,
    live: u32,
    pb: HashMap<u64, Vec<u32>>,
    calls: Vec<Call>,
    next_scan: Scan,
    /// Entries the PBDMA has popped (absolute), reported as the private `GP_GET`.
    popped: u64,
    priv_entries: u32,
    /// Markers the PBDMA has processed (the tagged `REFERENCE`).
    markers_done: u64,
    walk: bool,
    /// The slot values at which a `Load` happened in the turn, with the `GP_PUT` the relay used (I8 bookkeeping).
    loads: Vec<(u32, u32)>,
}

impl Mock {
    fn new(entries: u32, priv_entries: u32) -> Self {
        Self {
            entries,
            ring: vec![0; entries as usize],
            put: 0,
            epoch: 0,
            live: 0,
            pb: HashMap::new(),
            calls: Vec::new(),
            next_scan: Scan::default(),
            popped: 0,
            priv_entries,
            markers_done: 0,
            walk: false,
            loads: Vec::new(),
        }
    }
    fn put_entry(&mut self, slot: u32, g: &Ge) {
        self.ring[slot as usize] = g.raw();
        self.pb.insert(g.raw(), g.words());
    }
    fn count(&self, c: &Call) -> usize {
        self.calls.iter().filter(|x| *x == c).count()
    }
    fn count_writebacks(&self) -> usize {
        self.calls
            .iter()
            .filter(|c| matches!(c, Call::WriteGuestGet(_)))
            .count()
    }
    /// The PBDMA pops everything the relay issued.
    fn pop_all(&mut self, r: &Relay) {
        self.popped = r.fetch.issued();
    }
}

impl Io for Mock {
    fn hw_words(&mut self) -> Turn<(u32, u32)> {
        self.calls.push(Call::HwWords);
        let reference = if self.markers_done == 0 {
            0
        } else {
            kf_chan::user_gate::MARK_TAG | (self.markers_done as u32 & 0xffff)
        };
        Ok(((self.popped as u32) & (self.priv_entries - 1), reference))
    }
    fn write_guest_get(&mut self, get: u32) -> Turn<bool> {
        self.calls.push(Call::WriteGuestGet(get));
        Ok(true)
    }
    fn src_entries(&mut self) -> u32 {
        self.entries
    }
    fn max_entry_bytes(&mut self) -> u64 {
        kf_chan::user_storage::MAX_SOURCE_BYTES
    }
    fn walk_busy(&mut self) -> bool {
        self.walk
    }
    fn gp_put(&mut self) -> Turn<Option<u32>> {
        self.calls.push(Call::GpPut);
        Ok(Some(self.put))
    }
    fn load_dword(&mut self, slot: u32, hi: bool) -> Turn<u32> {
        self.calls.push(Call::Load(slot, hi));
        let v = self.ring[slot as usize];
        Ok(if hi { (v >> 32) as u32 } else { v as u32 })
    }
    fn registry_epoch(&mut self) -> u64 {
        self.calls.push(Call::Epoch);
        self.epoch
    }
    fn registry_live(&mut self) -> u32 {
        self.live
    }
    fn software(&mut self, _refresh: bool) -> Turn<SwView> {
        Ok(SwView {
            have: true,
            gone: false,
        })
    }
    fn sw_matches(&self, value: u32) -> bool {
        sw(value)
    }
    fn read_pb(&mut self, entry: u64, _seg: &GpSegment) -> Turn<(Vec<u32>, bool)> {
        self.calls.push(Call::ReadPb(entry));
        Ok((
            self.pb
                .get(&(entry & !support::SYNC))
                .cloned()
                .unwrap_or_default(),
            false,
        ))
    }
    fn stage(&mut self, scan: &Scan, words: &[u32]) -> Turn<Staged> {
        let mut next = scan.clone();
        match next.segment(words, sw) {
            Err(e) => Err(Fail::Refused(format!("USER_RING_SCAN {e:?}"))),
            Ok(t) => {
                self.next_scan = next;
                Ok(Staged { rare: t.is_some() })
            }
        }
    }
    fn publish_entry(
        &mut self,
        kind: PubKind,
        fetch: &mut Fetch,
        generation: u64,
        guest_get: u32,
        entry: u64,
    ) -> Turn<()> {
        fetch
            .publish(Some((generation, guest_get)))
            .map_err(str::to_owned)?;
        self.calls.push(Call::Entry(kind, entry));
        Ok(())
    }
    fn publish_gate(&mut self, fetch: &mut Fetch, marks: &mut Marks) -> Turn<u64> {
        fetch.publish(None).map_err(str::to_owned)?;
        fetch.publish(None).map_err(str::to_owned)?;
        let (n, _) = marks.issue().map_err(str::to_owned)?;
        self.calls.push(Call::Gate);
        Ok(n)
    }
    fn publish_marker(&mut self, fetch: &mut Fetch, marks: &mut Marks) -> Turn<()> {
        fetch.publish(None).map_err(str::to_owned)?;
        marks.issue().map_err(str::to_owned)?;
        self.calls.push(Call::Marker);
        Ok(())
    }
    fn event(&mut self, _e: Ev) {}
}

fn relay(io: &Mock, cfg: LazyConfig) -> Relay {
    Relay::new(
        io.priv_entries,
        Lifecycle::new(),
        Scan::for_subdevice(1).unwrap(),
    )
    .unwrap()
    .with_lazy(cfg)
}

/// One turn, standing in for the pump's segment machinery after a scanned segment (non-rare path).
fn turn(r: &mut Relay, io: &mut Mock) -> Result<(), Fail> {
    r.claim();
    let mut guard = 0;
    while !r.idle() {
        guard += 1;
        assert!(guard < 64, "a turn must be bounded");
        match r.step(io)? {
            Step::Scanned { rare } => {
                assert!(!rare, "no trigger in these tests");
                r.fetch
                    .publish(Some((r.generation, r.next_get)))
                    .map_err(str::to_owned)?;
                r.scan = io.next_scan.clone();
                let open = r.scan.carried().is_some();
                r.life.segment_scanned(open);
                r.cursor = r.next_get;
                r.note_relayed(r.raw);
                r.unnotified = true;
                r.again = true;
            }
            Step::Segment | Step::Shadow => panic!("not used here"),
            Step::More | Step::Idle { .. } => {}
        }
    }
    Ok(())
}

/// A relay with the SET_OBJECT scanned and its first Pointer entry forwarded (cursor 2), guest ring `entries` slots.
fn pointer_relay(io: &mut Mock, cfg: LazyConfig) -> Relay {
    let mut r = relay(io, cfg);
    io.put_entry(0, &ge(0, Kind::SetObj));
    io.put = 1;
    turn(&mut r, io).unwrap();
    io.put_entry(1, &ge(1, Kind::Plain));
    io.put = 2;
    turn(&mut r, io).unwrap();
    assert_eq!(r.life.mode(), Mode::Pointer);
    io.calls.clear();
    r
}

/// The guest publishes entries `from..to` (plain) and moves `GP_PUT` to `to`.
fn feed(io: &mut Mock, from: u32, to: u32) {
    for slot in from..to {
        io.put_entry(slot % io.entries, &ge((slot % 100) as u8 + 1, Kind::Plain));
    }
    io.put = to % io.entries;
}

fn on() -> LazyConfig {
    LazyConfig::ON
}

/// Run `n` turns, returning per turn whether it read the private `USERD`, `GP_PUT`, and whether the guest ring looked full.
fn run(r: &mut Relay, io: &mut Mock, n: usize) -> Vec<(bool, bool, bool)> {
    let mut out = Vec::new();
    for _ in 0..n {
        io.calls.clear();
        turn(r, io).unwrap();
        out.push((
            io.count(&Call::HwWords) > 0,
            io.count(&Call::GpPut) > 0,
            r.put_full,
        ));
    }
    out
}

// ------------------------------------------------------------------------------------------------
// A: the GP_PUT cache

#[test]
fn a_a_pointer_turn_behind_the_cached_put_does_not_read_gp_put() {
    let mut io = Mock::new(64, 256);
    let mut r = pointer_relay(&mut io, on());
    feed(&mut io, 2, 12);
    let t = run(&mut r, &mut io, 9);
    assert!(t[0].1, "the first turn reads GP_PUT");
    assert!(
        t[1..].iter().all(|x| !x.1),
        "the next eight are behind the cached PUT: {t:?}"
    );
    assert_eq!(r.cursor, 11);
    assert_eq!(r.lazy.counts.put_cache_hits, 8);
}

#[test]
fn a_off_reads_gp_put_on_every_turn_like_the_base() {
    for cfg in [
        LazyConfig::OFF,
        LazyConfig::from_switches(Some("0"), Some("0")),
        on().with_put_cache(false),
    ] {
        let mut io = Mock::new(64, 256);
        let mut r = pointer_relay(&mut io, cfg);
        feed(&mut io, 2, 12);
        let t = run(&mut r, &mut io, 9);
        assert!(
            t.iter().all(|x| x.1),
            "{cfg:?}: every turn reads GP_PUT: {t:?}"
        );
        assert_eq!(r.lazy.counts.put_cache_hits, 0);
    }
}

#[test]
fn a_a_cached_put_is_never_used_when_the_cursor_has_reached_it_and_idle_happens_only_on_a_fresh_read()
 {
    let mut io = Mock::new(64, 256);
    let mut r = pointer_relay(&mut io, on());
    feed(&mut io, 2, 5);
    run(&mut r, &mut io, 3); // cursor 5 == the cached PUT
    assert_eq!(r.cursor, 5);
    io.calls.clear();
    // the guest has meanwhile published two more entries (the doorbell for them is on its way)
    feed(&mut io, 5, 7);
    turn(&mut r, &mut io).unwrap();
    assert_eq!(
        io.count(&Call::GpPut),
        1,
        "cursor == cached PUT: the turn reads fresh"
    );
    assert_eq!(r.cursor, 6, "and found the new work");
    // nothing more is published: the idle decision (end_batch) is taken on a fresh read of the SAME value
    turn(&mut r, &mut io).unwrap();
    assert_eq!(r.cursor, 7);
    io.calls.clear();
    turn(&mut r, &mut io).unwrap();
    assert_eq!(io.count(&Call::GpPut), 1, "idle only on a fresh read");
    assert!(
        !io.calls.iter().any(|c| matches!(c, Call::Load(..))),
        "nothing at or beyond GP_PUT is read"
    );
    assert!(r.need != Need::Again);
}

#[test]
fn a_work_published_behind_the_cache_is_relayed_in_order_when_the_cursor_reaches_it() {
    let mut io = Mock::new(64, 256);
    let mut r = pointer_relay(&mut io, on());
    feed(&mut io, 2, 6);
    run(&mut r, &mut io, 2);
    feed(&mut io, 6, 20); // more work while the relay is still behind the cached PUT=6
    run(&mut r, &mut io, 40);
    assert_eq!(
        r.cursor, 20,
        "every entry up to the last GP_PUT was relayed"
    );
    assert_eq!(r.relayed, 20);
}

#[test]
fn a_i8_no_slot_at_or_beyond_the_put_in_use_is_ever_loaded_and_the_cache_never_runs_past_the_ring()
{
    let mut io = Mock::new(16, 256);
    let mut r = pointer_relay(&mut io, on());
    // many laps, the guest publishes in bursts
    let mut next = 2u32;
    let mut total = 2u64;
    for burst in 0..60u32 {
        let n = 1 + burst % 7;
        // never overrun the ring: leave one slot between the guest's PUT and the GET it sees
        let room = (r.written_get + 16 - (next % 16) - 1) % 16;
        let n = n.min(room);
        feed(&mut io, next, next + n);
        next += n;
        total += u64::from(n);
        for _ in 0..(n + 3) {
            io.calls.clear();
            let before = r.cursor;
            turn(&mut r, &mut io).unwrap();
            io.pop_all(&r);
            let slots: Vec<u32> = io
                .calls
                .iter()
                .filter_map(|c| {
                    if let Call::Load(s, false) = c {
                        Some(*s)
                    } else {
                        None
                    }
                })
                .collect();
            for s in slots {
                assert_eq!(s, before, "only the slot at the cursor is loaded");
                let ahead = io.put.wrapping_sub(s) & 15;
                assert!(
                    ahead != 0,
                    "the slot {s} is at the guest's current GP_PUT {}",
                    io.put
                );
            }
        }
    }
    assert_eq!(r.relayed, total, "nothing lost, nothing duplicated");
}

#[test]
fn a_scan_and_window_turns_always_read_gp_put_fresh() {
    let mut io = Mock::new(64, 256);
    let mut r = relay(&io, on());
    // Scan: the SET_OBJECT is not bound yet
    io.put_entry(0, &ge(9, Kind::Plain));
    io.put_entry(1, &ge(10, Kind::Plain));
    io.put_entry(2, &ge(11, Kind::Plain));
    io.put = 3;
    for _ in 0..3 {
        io.calls.clear();
        assert_eq!(r.life.mode(), Mode::Scan);
        turn(&mut r, &mut io).unwrap();
        assert_eq!(io.count(&Call::GpPut), 1, "Scan reads GP_PUT on every turn");
        assert_eq!(io.count(&Call::HwWords), 1, "Scan observes on every turn");
    }
    assert_eq!(r.lazy.counts.put_cache_hits, 0);
    assert_eq!(r.lazy.cached(), None, "nothing is remembered in Scan");
}

#[test]
fn a_every_mode_change_drops_the_cache_and_the_window_reads_fresh() {
    let mut io = Mock::new(64, 256);
    let mut r = pointer_relay(&mut io, on());
    feed(&mut io, 2, 30);
    run(&mut r, &mut io, 3);
    assert_eq!(r.lazy.cached(), Some((1, 30)), "Pointer mode caches");
    // a registration appears; the guest publishes more behind the cached PUT. The Window opens at the ANCHOR (the
    // GP_PUT the last clean turn read = 30): until the cursor gets there the relay is still a Pointer relay on the cache.
    io.epoch = 1;
    io.live = 1;
    feed(&mut io, 30, 36);
    let mut hits = 0;
    for _ in 0..60 {
        io.calls.clear();
        turn(&mut r, &mut io).unwrap();
        if r.life.mode() == Mode::Window {
            break;
        }
        hits += usize::from(io.count(&Call::GpPut) == 0);
    }
    assert_eq!(r.life.mode(), Mode::Window, "flipped at the anchor");
    assert!(
        hits >= 20,
        "the entries below the anchor were forwarded on the cache: {hits}"
    );
    assert_eq!(
        r.cursor, 31,
        "the Window started at the anchor (30) and its first entry was scanned"
    );
    assert_eq!(
        r.lazy.counts.inval_mode_change, 0,
        "the flip turn itself was a Pointer turn"
    );
    assert_eq!(
        r.lazy.cached(),
        Some((1, 36)),
        "it read GP_PUT fresh at the cursor == cached PUT"
    );
    // the first Window turn starts by dropping the cache and reads GP_PUT fresh
    io.calls.clear();
    turn(&mut r, &mut io).unwrap();
    assert_eq!(
        r.lazy.counts.inval_mode_change, 1,
        "Pointer -> Window dropped the cache"
    );
    assert_eq!(r.lazy.cached(), None);
    assert_eq!(
        io.count(&Call::GpPut),
        1,
        "the first Window turn reads GP_PUT fresh"
    );
    // and every other Window turn does too
    for _ in 0..3 {
        io.calls.clear();
        turn(&mut r, &mut io).unwrap();
        assert_eq!(r.life.mode(), Mode::Window);
        assert_eq!(
            io.count(&Call::GpPut),
            1,
            "Window reads GP_PUT on every turn"
        );
        assert_eq!(io.count(&Call::HwWords), 1, "Window observes on every turn");
        assert_eq!(r.lazy.cached(), None);
    }
    // the registration is gone: Window -> Pointer; the cache starts empty again and refills from a fresh read
    io.live = 0;
    io.epoch = 2;
    feed(&mut io, 36, 50);
    let mut back = false;
    for _ in 0..60 {
        io.calls.clear();
        turn(&mut r, &mut io).unwrap();
        io.pop_all(&r);
        if r.life.mode() == Mode::Pointer {
            back = true;
            break;
        }
    }
    assert!(back, "the Window closed");
    io.calls.clear();
    turn(&mut r, &mut io).unwrap();
    assert_eq!(
        io.count(&Call::GpPut),
        1,
        "the first Pointer turn after a Window reads fresh"
    );
    assert!(r.lazy.cached().is_some());
}

#[test]
fn a_a_stale_cache_from_before_a_window_is_never_used_after_it() {
    // The cursor passes the cached PUT's slot while in a Window (it is not consulted there). Back in Pointer mode a
    // surviving cache would name a PUT BEHIND the cursor, i.e. an enormous modular distance: I8.
    let mut io = Mock::new(16, 256);
    let mut r = pointer_relay(&mut io, on());
    feed(&mut io, 2, 5);
    run(&mut r, &mut io, 1);
    assert_eq!(r.lazy.cached(), Some((1, 5)));
    io.epoch = 1;
    io.live = 1;
    // the guest keeps going while the Window is open
    feed(&mut io, 5, 12);
    for _ in 0..30 {
        turn(&mut r, &mut io).unwrap();
        io.pop_all(&r);
        if r.life.mode() == Mode::Window {
            break;
        }
    }
    assert_eq!(r.life.mode(), Mode::Window);
    io.live = 0;
    io.epoch = 2;
    for _ in 0..30 {
        turn(&mut r, &mut io).unwrap();
        io.pop_all(&r);
        if r.life.mode() == Mode::Pointer {
            break;
        }
    }
    assert_eq!(r.life.mode(), Mode::Pointer);
    assert!(
        r.cursor > 5,
        "the cursor is past the old cached PUT: {}",
        r.cursor
    );
    io.calls.clear();
    feed(&mut io, 12, 14);
    turn(&mut r, &mut io).unwrap();
    assert_eq!(
        io.count(&Call::GpPut),
        1,
        "fresh read after the mode change"
    );
    assert_ne!(r.lazy.cached().map(|c| c.1), Some(5));
    let loads: Vec<&Call> = io
        .calls
        .iter()
        .filter(|c| matches!(c, Call::Load(..)))
        .collect();
    assert!(loads.len() <= 2, "at most one entry was loaded: {loads:?}");
}

#[test]
fn a_a_ring_migration_drops_the_cache_even_when_the_new_ring_has_a_lower_put() {
    let mut io = Mock::new(64, 256);
    let mut r = pointer_relay(&mut io, on());
    feed(&mut io, 2, 40);
    run(&mut r, &mut io, 2);
    assert_eq!(r.lazy.cached(), Some((1, 40)));
    // ring growth: the relay switches to a new source ring, generation 2, cursor 0
    r.switch_source(2);
    assert_eq!(r.lazy.cached(), None);
    assert_eq!(r.lazy.counts.inval_migration, 1);
    // the new ring's GP_PUT is LOWER than the old cached one
    for slot in 0..8 {
        io.put_entry(slot, &ge(slot as u8 + 50, Kind::Plain));
    }
    io.put = 8;
    io.calls.clear();
    turn(&mut r, &mut io).unwrap();
    assert_eq!(io.count(&Call::GpPut), 1, "fresh read in the new ring");
    assert_eq!(r.lazy.cached(), Some((2, 8)));
}

#[test]
fn a_walk_in_progress_ends_the_turn_before_any_read_even_with_a_valid_cache() {
    let mut io = Mock::new(64, 256);
    let mut r = pointer_relay(&mut io, on());
    feed(&mut io, 2, 20);
    run(&mut r, &mut io, 2);
    io.walk = true;
    io.calls.clear();
    turn(&mut r, &mut io).unwrap();
    assert!(
        !io.calls.iter().any(|c| matches!(
            c,
            Call::GpPut | Call::Load(..) | Call::Epoch | Call::Entry(..)
        )),
        "a pending mapping walk: nothing is read: {:?}",
        io.calls
    );
    io.walk = false;
    io.calls.clear();
    turn(&mut r, &mut io).unwrap();
    assert!(
        io.calls.iter().any(|c| matches!(c, Call::Entry(..))),
        "and the relay goes on afterwards"
    );
}

#[test]
fn a_a_gp_put_outside_the_ring_is_refused_and_never_cached() {
    let mut io = Mock::new(16, 256);
    let mut r = pointer_relay(&mut io, on());
    io.put = 16;
    let e = turn(&mut r, &mut io).unwrap_err();
    assert_eq!(e, Fail::Refused("USER_RING_PUT_BOUND".into()));
    assert_eq!(r.lazy.cached(), None);
}

// ------------------------------------------------------------------------------------------------
// B: the private USERD

#[test]
fn b_the_private_userd_is_read_on_the_interval_and_not_in_between() {
    let mut io = Mock::new(64, 512);
    let mut r = pointer_relay(&mut io, on());
    let skipped0 = r.lazy.counts.obs_skipped;
    // plenty of credit, nothing waiting, the guest publishes a long stream
    feed(&mut io, 2, 60);
    let t = run(&mut r, &mut io, 48);
    let reads = t.iter().filter(|x| x.0).count();
    assert!(
        (2..=4).contains(&reads),
        "about one look per {OBSERVE_INTERVAL} turns, got {reads} in 48: {t:?}"
    );
    assert_eq!(r.lazy.counts.obs_skipped - skipped0, (48 - reads) as u64);
    assert!(r.lazy.counts.obs_forced_interval as usize >= reads.saturating_sub(1));
}

#[test]
fn b_off_observes_on_every_turn_like_the_base() {
    for cfg in [
        LazyConfig::OFF,
        on().with_interval(1),
        LazyConfig::from_switches(None, Some("0")),
    ] {
        let mut io = Mock::new(64, 512);
        let mut r = pointer_relay(&mut io, cfg);
        feed(&mut io, 2, 60);
        let t = run(&mut r, &mut io, 30);
        assert!(
            t.iter().all(|x| x.0),
            "{cfg:?}: every turn reads the private USERD"
        );
        assert_eq!(r.lazy.counts.obs_skipped, 0);
    }
}

#[test]
fn b_a_turn_after_a_wait_always_observes() {
    let mut io = Mock::new(64, 512);
    let mut r = pointer_relay(&mut io, on());
    // the relay caught up: end_batch publishes a marker and waits for it
    run(&mut r, &mut io, 1);
    let waited = (0..4).find(|_| {
        io.calls.clear();
        turn(&mut r, &mut io).unwrap();
        matches!(r.need, Need::Marker { .. } | Need::Popped { .. })
    });
    assert!(
        waited.is_some(),
        "the relay ended a turn waiting: {:?}",
        r.need
    );
    let before = r.lazy.counts.obs_forced_wait;
    io.calls.clear();
    turn(&mut r, &mut io).unwrap();
    assert_eq!(
        io.count(&Call::HwWords),
        1,
        "the turn after a wait reads the private USERD"
    );
    assert_eq!(r.lazy.counts.obs_forced_wait, before + 1);
}

#[test]
fn b_a_wait_forces_a_look_on_every_turn_until_it_is_over() {
    let mut io = Mock::new(64, 512);
    let mut r = pointer_relay(&mut io, on());
    feed(&mut io, 2, 4);
    run(&mut r, &mut io, 2);
    // GP_PUT == cursor: marker, wait. The PBDMA does NOT pop for a while.
    let mut looks = 0;
    for _ in 0..40 {
        io.calls.clear();
        turn(&mut r, &mut io).unwrap();
        if matches!(r.need, Need::Marker { .. } | Need::Popped { .. }) {
            looks += 1;
            let next = {
                io.calls.clear();
                turn(&mut r, &mut io).unwrap();
                io.count(&Call::HwWords)
            };
            assert_eq!(next, 1, "while waiting, every turn looks (wait #{looks})");
            if looks >= 6 {
                return;
            }
        }
    }
    panic!("the relay never waited");
}

#[test]
fn b_fewer_than_the_credit_floor_forces_a_look_every_turn() {
    let mut io = Mock::new(256, 128);
    let mut r = pointer_relay(&mut io, on());
    let guest_runs = 120u32;
    feed(&mut io, 2, guest_runs);
    // the PBDMA pops nothing: credits shrink by one per entry
    let mut forced = 0;
    for _ in 0..guest_runs {
        io.calls.clear();
        let low = r.fetch.available() < CREDIT_FLOOR;
        let waiting = !matches!(r.need, Need::Idle | Need::Again);
        let before = r.lazy.counts.obs_forced_low_credit;
        if turn(&mut r, &mut io).is_err() {
            break;
        }
        if low {
            assert_eq!(
                io.count(&Call::HwWords),
                1,
                "credits {} < {CREDIT_FLOOR}: look",
                r.fetch.available()
            );
            // a pending wait is the first reason; otherwise the credit floor is
            if !waiting {
                assert_eq!(r.lazy.counts.obs_forced_low_credit, before + 1);
            }
            forced += 1;
        }
    }
    assert!(forced >= 3, "the ring really got low on credit: {forced}");
}

#[test]
fn b_an_open_segment_forces_a_look() {
    let mut io = Mock::new(64, 512);
    let mut r = pointer_relay(&mut io, on());
    // find a turn that would skip
    feed(&mut io, 2, 30);
    run(&mut r, &mut io, 2);
    r.segment_pending = true;
    for _ in 0..20 {
        io.calls.clear();
        r.claim();
        let s1 = r.step(&mut io).unwrap(); // Obs
        assert!(matches!(s1, Step::More));
        assert_eq!(
            io.count(&Call::HwWords),
            1,
            "a segment is open: the private USERD is read"
        );
        r.step(&mut io).unwrap(); // Wb
        assert_eq!(r.step(&mut io).unwrap(), Step::Segment);
        r.phase = Phase::Idle;
    }
    assert_eq!(r.lazy.counts.obs_forced_open_segment, 20);
}

#[test]
fn b_a_skipped_observation_never_over_counts_credit_and_the_next_look_catches_up() {
    let mut io = Mock::new(64, 512);
    let mut r = pointer_relay(&mut io, on());
    feed(&mut io, 2, 40);
    run(&mut r, &mut io, 1);
    // the PBDMA pops everything issued so far while the relay is NOT looking
    io.pop_all(&r);
    let real_free = 511u32 - (r.fetch.issued() - io.popped) as u32;
    let mut looked = false;
    for _ in 0..OBSERVE_INTERVAL + 1 {
        io.calls.clear();
        let free_before = r.fetch.available();
        assert!(
            free_before <= real_free,
            "the relay never believes in more credit than the ring has"
        );
        turn(&mut r, &mut io).unwrap();
        if io.count(&Call::HwWords) > 0 {
            looked = true;
            break;
        }
    }
    assert!(looked, "within one interval the relay looks");
}

#[test]
fn b_the_guests_gp_get_write_back_lags_by_at_most_the_interval() {
    let mut io = Mock::new(64, 512);
    let mut r = pointer_relay(&mut io, on());
    feed(&mut io, 2, 62);
    run(&mut r, &mut io, 1);
    io.pop_all(&r);
    let mut turns = 0;
    loop {
        io.calls.clear();
        turn(&mut r, &mut io).unwrap();
        turns += 1;
        if io.count_writebacks() > 0 {
            break;
        }
        assert!(
            turns <= OBSERVE_INTERVAL,
            "the write-back is at most {OBSERVE_INTERVAL} turns late"
        );
    }
}

#[test]
fn b_the_interval_never_exceeds_half_a_small_guest_ring() {
    for entries in [4u32, 8, 16, 32] {
        let mut io = Mock::new(entries, 512);
        let mut r = pointer_relay(&mut io, on());
        let cap = u64::from(entries / 2).max(1).min(OBSERVE_INTERVAL);
        let mut next = 2u32;
        let mut since = 0u64;
        let mut worst = 0u64;
        for _ in 0..200 {
            // keep the relay fed: one entry per turn, never overrunning the ring
            let room = (r.written_get + entries - (next % entries) - 1) % entries;
            if room > 0 && r.cursor == next % entries {
                feed(&mut io, next, next + 1);
                next += 1;
            }
            io.calls.clear();
            turn(&mut r, &mut io).unwrap();
            io.pop_all(&r);
            if io.count(&Call::HwWords) > 0 {
                since = 0;
            } else {
                since += 1;
                worst = worst.max(since);
            }
        }
        assert!(
            worst < cap.max(1),
            "ring of {entries}: {worst} turns without a look, cap {cap}"
        );
    }
}

#[test]
fn b_d6_a_guest_ring_that_looks_full_is_written_back_by_the_very_next_turn() {
    let mut io = Mock::new(64, 512);
    let mut r = pointer_relay(&mut io, on());
    // the guest filled its ring up to the GET it sees (written_get = 0 at birth, so PUT = 63 is "full")
    feed(&mut io, 2, 63);
    let t = run(&mut r, &mut io, 40);
    let mut checked = 0;
    for w in t.windows(2) {
        let (obs, _, full) = w[0];
        if !obs && full {
            assert!(
                w[1].0,
                "a turn that skipped its look and saw a full guest ring is followed by a look: {t:?}"
            );
            checked += 1;
        }
    }
    assert!(
        checked > 0,
        "the scenario produced a skipped turn with a full ring: {t:?}"
    );
    assert!(r.lazy.counts.obs_forced_ring_full > 0);
}

#[test]
fn b_the_guard_does_nothing_when_the_ring_is_not_full() {
    let mut io = Mock::new(64, 512);
    let mut r = pointer_relay(&mut io, on());
    feed(&mut io, 2, 40);
    run(&mut r, &mut io, 30);
    assert_eq!(r.lazy.counts.obs_forced_ring_full, 0);
}

#[test]
fn b_an_open_segment_is_counted_as_its_own_reason_on_every_turn_and_never_skipped() {
    let mut io = Mock::new(64, 512);
    let mut r = pointer_relay(&mut io, on());
    r.segment_pending = true;
    let before = r.lazy.counts;
    for _ in 0..40 {
        r.claim();
        r.step(&mut io).unwrap();
        r.step(&mut io).unwrap();
        assert_eq!(r.step(&mut io).unwrap(), Step::Segment);
        r.phase = Phase::Idle;
    }
    assert_eq!(
        r.lazy.counts.obs_skipped, before.obs_skipped,
        "no turn skipped its look"
    );
    assert_eq!(
        r.lazy.counts.obs_forced_open_segment - before.obs_forced_open_segment,
        40
    );
}

// ------------------------------------------------------------------------------------------------
// invalidation by reason, counters

#[test]
fn x_retire_and_a_refusal_drop_the_cache_and_count_their_reason() {
    let mut io = Mock::new(64, 256);
    let mut r = pointer_relay(&mut io, on());
    feed(&mut io, 2, 30);
    run(&mut r, &mut io, 2);
    assert!(r.lazy.cached().is_some());
    r.retire();
    assert_eq!(r.lazy.cached(), None);
    assert_eq!(r.lazy.counts.inval_retire, 1);
    assert_eq!(r.life.mode(), Mode::Dead);

    // a refusal ends the relay: channel reset
    let mut io = Mock::new(64, 256);
    let mut r = pointer_relay(&mut io, on());
    feed(&mut io, 2, 30);
    run(&mut r, &mut io, 2);
    io.ring[r.cursor as usize] = ge(2, Kind::Plain).raw() | (1 << 40); // reserved bit: refused by name
    let e = turn(&mut r, &mut io).unwrap_err();
    assert!(matches!(e, Fail::Refused(_)));
    assert!(r.dead);
    assert_eq!(r.lazy.cached(), None);
    assert_eq!(r.lazy.counts.inval_reset, 1);
}

#[test]
fn x_every_turn_is_accounted_for_and_the_put_counters_add_up() {
    let mut io = Mock::new(64, 512);
    let mut r = pointer_relay(&mut io, on());
    feed(&mut io, 2, 50);
    let mut total_turns = 2; // the two turns of pointer_relay
    total_turns += run(&mut r, &mut io, 60).len() as u64;
    let c = r.lazy.counts;
    assert_eq!(c.turns, total_turns);
    assert_eq!(
        c.turns,
        c.obs_skipped
            + c.obs_unconditional
            + c.obs_forced_wait
            + c.obs_forced_low_credit
            + c.obs_forced_open_segment
            + c.obs_forced_ring_full
            + c.obs_forced_interval,
        "{c:?}"
    );
    assert_eq!(
        c.put_cache_hits + c.put_fresh_reads,
        c.turns,
        "every turn used exactly one GP_PUT: {c:?}"
    );
    assert!(c.put_cache_hits > 40);
    let d = r.lazy.take_delta();
    assert_eq!(d, c);
}

#[test]
fn x_the_walk_check_runs_before_the_cache_is_consulted_and_is_not_counted_as_a_put_use() {
    let mut io = Mock::new(64, 256);
    let mut r = pointer_relay(&mut io, on());
    feed(&mut io, 2, 20);
    run(&mut r, &mut io, 2);
    io.walk = true;
    let before = r.lazy.counts;
    turn(&mut r, &mut io).unwrap();
    assert_eq!(r.lazy.counts.put_cache_hits, before.put_cache_hits);
    assert_eq!(r.lazy.counts.put_fresh_reads, before.put_fresh_reads);
}

#[test]
fn x_the_lazy_state_does_not_touch_the_environment() {
    // The switches are read ONCE by the owner (`LazyConfig::from_switches`); the relay and the state it carries never
    // read the environment, per turn or otherwise.
    for src in [
        include_str!("../src/user_lazy.rs"),
        include_str!("../src/user_turn.rs"),
    ] {
        let code: String = src
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !code.contains("std::env"),
            "no environment access in the per-turn code"
        );
        assert!(
            !code.contains("env::var"),
            "no environment access in the per-turn code"
        );
    }
}
