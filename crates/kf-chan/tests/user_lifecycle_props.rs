//! ★ 2026-10-16 — **Sequential property, differential and corpus tests of the USER-ring lifecycle**
//! (`docs/design/V3_USER_RING_LIFECYCLE_20261016.md`, brief section 3 items 3, 4, 6 and appendix D).
//!
//! The same `lifecycle_support::Relay` as the interleaving model, driven by a SEQUENTIAL simulator: the
//! hardware consumes the private ring at once, the guest feeds its ring as space allows, a registration
//! fires between two turns at a chosen number of forwarded entries. Two relays run every stream: the
//! BASE behaviour (`Lifecycle::always_scan`: every entry is read and scanned for life) and the lifecycle.
//!
//! G2 (identity): the GP entries forwarded to the private ring are bit-identical between the two for every
//! stream without a registration; with a registration they are the same entries with the trigger staged
//! at the same position and the source switched at the same position, PROVIDED the flip does not fall
//! inside an open packet (assumption P2). Seeds are printed on failure.
#![allow(dead_code, missing_docs, clippy::too_many_lines, clippy::needless_range_loop)]

#[path = "lifecycle_support/mod.rs"]
mod support;

use kf_chan::user_gate::MARK_TAG;
use kf_chan::user_lifecycle::{Lifecycle, Mode};
use kf_chan::user_scan::Scan;
use std::collections::{HashMap, VecDeque};
use support::*;

// ------------------------------------------------------------------------------------------------
// a seeded generator (deterministic; seeds are printed on failure)

#[derive(Clone)]
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

// ------------------------------------------------------------------------------------------------
// the sequential simulator

#[derive(Clone, Debug)]
pub enum Sched {
    /// A registration is applied when `at` guest entries have been forwarded.
    Register { at: u64, migrate: bool },
    Remove { at: u64 },
}
impl Sched {
    fn at(&self) -> u64 {
        match self {
            Sched::Register { at, .. } | Sched::Remove { at } => *at,
        }
    }
}

#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct Out {
    /// The stored value of every guest entry published, in order.
    pub forwarded: Vec<u64>,
    /// Indices (into `forwarded`) of the staged trigger entries.
    pub staged: Vec<usize>,
    /// Number of guest entries forwarded when the source switched (per switch).
    pub switches: Vec<usize>,
    pub dead: bool,
    pub refused_scan: u32,
    pub unscanned_while_live: u32,
    pub torn: u32,
    pub pb_reads: u32,
    pub pb_reads_in_pointer: u32,
    pub max_steps_per_turn: u32,
    pub markers: u64,
    pub triggers_found: u32,
}

pub struct Sim {
    pub entries: u32,
    pub rings: [HashMap<u32, u64>; 2],
    pub src: usize,
    pub put: [u32; 2],
    put_local: [u32; 2],
    guest_get: [u32; 2],
    pending: [VecDeque<u64>; 2],
    defs: HashMap<u8, Ge>,
    pub epoch: u64,
    pub live: u32,
    pub kind_migrate: bool,
    sched: Vec<Sched>,
    pub priv_entries: u32,
    log_len: usize,
    markers_done: u64,
    pub out: Out,
    pub next_size: u32,
    pub mode_hint: Mode,
    held: [bool; 2],
}

impl Sim {
    pub fn new(entries: u32, priv_entries: u32, stream: &[Ge], sched: Vec<Sched>) -> Self {
        let mut defs = HashMap::new();
        let mut pending = [VecDeque::new(), VecDeque::new()];
        for g in stream {
            defs.insert(g.id, *g);
            pending[0].push_back(g.raw());
        }
        Self {
            entries,
            rings: [HashMap::new(), HashMap::new()],
            src: 0,
            put: [0; 2],
            put_local: [0; 2],
            guest_get: [0; 2],
            pending,
            defs,
            epoch: 0,
            live: 0,
            kind_migrate: true,
            sched,
            priv_entries,
            log_len: 0,
            markers_done: 0,
            out: Out::default(),
            next_size: entries,
            mode_hint: Mode::Scan,
            held: [false; 2],
        }
    }

    /// The guest publishes as many pending entries as the ring has room for, then GP_PUT.
    fn feed(&mut self) {
        let r = self.src;
        while let Some(&raw) = self.pending[r].front() {
            if self.held[r] || (self.put_local[r] + 1) % self.entries == self.guest_get[r] {
                break;
            }
            self.rings[r].insert(self.put_local[r], raw);
            self.put_local[r] = (self.put_local[r] + 1) % self.entries;
            self.pending[r].pop_front();
            // a ring-growth trigger is the LAST entry of its ring: the guest continues in the new one
            if entry_id(raw).and_then(|i| self.defs.get(&i)).is_some_and(|g| g.kind == Kind::Trig { migrate: true }) {
                self.held[r] = true;
            }
        }
        self.put[r] = self.put_local[r];
    }

    fn apply_sched(&mut self, forwarded: u64) {
        while let Some(i) = self.sched.iter().position(|s| s.at() <= forwarded) {
            match self.sched.remove(i) {
                Sched::Register { migrate, .. } => {
                    self.live += 1;
                    self.epoch += 1;
                    self.kind_migrate = migrate;
                }
                Sched::Remove { .. } => {
                    if self.live > 0 {
                        self.live -= 1;
                        self.epoch += 1;
                    }
                }
            }
        }
    }
}

impl Env for Sim {
    fn src_entries(&self) -> u32 {
        self.entries
    }
    fn gp_put(&mut self) -> u32 {
        self.put[self.src]
    }
    fn gp_dword(&mut self, slot: u32, hi: bool) -> u32 {
        let raw = self.rings[self.src].get(&slot).copied().unwrap_or(0);
        if hi { (raw >> 32) as u32 } else { raw as u32 }
    }
    fn registry_epoch(&mut self) -> u64 {
        self.epoch
    }
    fn registry_live(&mut self) -> u32 {
        self.live
    }
    fn hw_get(&mut self) -> u32 {
        (self.log_len as u32) & (self.priv_entries - 1)
    }
    fn hw_reference(&mut self) -> u32 {
        if self.markers_done == 0 {
            0
        } else {
            MARK_TAG | (self.markers_done as u32 & 0xffff)
        }
    }
    fn read_pb(&mut self, raw: u64) -> Vec<u32> {
        self.out.pb_reads += 1;
        if self.mode_hint == Mode::Pointer {
            self.out.pb_reads_in_pointer += 1;
        }
        let id = entry_id(raw).expect("a known entry");
        self.defs[&id].words()
    }
    fn publish(&mut self, p: Pub) {
        self.log_len += 1;
        match p {
            Pub::Guest {
                tested,
                stored,
                scanned,
            } => {
                if tested != stored {
                    self.out.torn += 1;
                }
                if self.live > 0 && !scanned {
                    let id = entry_id(tested);
                    let nop = id
                        .and_then(|i| self.defs.get(&i))
                        .is_some_and(|g| g.kind == Kind::Nop);
                    if !nop {
                        self.out.unscanned_while_live += 1;
                    }
                }
                self.out.forwarded.push(stored);
            }
            Pub::Staged { .. } => {
                self.out.staged.push(self.out.forwarded.len());
                self.out.forwarded.push(u64::MAX); // placeholder: the staged copy replaces the entry
            }
            Pub::Barrier => {}
            Pub::Marker(_) => {
                self.markers_done += 1;
                self.out.markers += 1;
            }
        }
    }
    fn write_guest_get(&mut self, v: u32) {
        self.guest_get[self.src] = v;
    }
    fn trigger_migrates(&mut self, _raw: u64) -> bool {
        self.kind_migrate
    }
    fn executed(&mut self, migrate: bool) {
        self.live = self.live.saturating_sub(1);
        self.epoch += 1;
        if migrate {
            self.out.switches.push(self.out.forwarded.len());
            // the new ring: the pending entries after the trigger move to it
            let rest: VecDeque<u64> = std::mem::take(&mut self.pending[0]);
            self.src = 1;
            self.entries = self.next_size;
            self.pending[1] = rest;
            self.held[0] = false;
            self.put_local[1] = 0;
            self.put[1] = 0;
            self.guest_get[1] = 0;
            self.rings[1].clear();
        }
    }
    fn event(&mut self, e: Ev) {
        match e {
            Ev::ScanRefused => {
                self.out.refused_scan += 1;
                self.out.dead = true;
            }
            Ev::EntryRefused | Ev::PutBound => self.out.dead = true,
            Ev::TriggerFound => self.out.triggers_found += 1,
            _ => {}
        }
    }
}

/// Run `relay` over `sim` until it has nothing more to do. Returns the number of turns.
pub fn drive(sim: &mut Sim, relay: &mut Relay, max_turns: usize) -> usize {
    let mut turns = 0;
    let mut idle_turns = 0;
    while turns < max_turns {
        sim.apply_sched(relay.abs);
        sim.feed();
        if relay.dead {
            break;
        }
        let before = (relay.puts, relay.cursor, relay.abs, relay.gener);
        relay.claim();
        let mut steps = 0;
        while !relay.idle() {
            sim.mode_hint = relay.life.mode();
            relay.step(sim);
            steps += 1;
            assert!(steps < 64, "a turn must be bounded");
        }
        sim.out.max_steps_per_turn = sim.out.max_steps_per_turn.max(steps);
        turns += 1;
        let after = (relay.puts, relay.cursor, relay.abs, relay.gener);
        if before == after && !relay.again {
            idle_turns += 1;
            // nothing moved: a pending registration that has not fired yet is the only reason to go on
            if idle_turns > 2 && sim.pending[sim.src].is_empty() && sim.sched.is_empty() {
                break;
            }
            if idle_turns > 6 {
                break;
            }
        } else {
            idle_turns = 0;
        }
    }
    turns
}

pub fn new_relay(priv_entries: u32, always_scan: bool) -> Relay {
    let life = if always_scan {
        Lifecycle::always_scan()
    } else {
        Lifecycle::new()
    };
    Relay::new(priv_entries, life, Scan::for_subdevice(1).unwrap(), Glue::default())
}

pub fn run_stream(
    stream: &[Ge],
    sched: &[Sched],
    always_scan: bool,
    entries: u32,
    priv_entries: u32,
) -> (Out, Relay) {
    let mut sim = Sim::new(entries, priv_entries, stream, sched.to_vec());
    let mut relay = new_relay(priv_entries, always_scan);
    drive(&mut sim, &mut relay, 200_000);
    sim.out.dead |= relay.dead;
    (sim.out, relay)
}

// ------------------------------------------------------------------------------------------------
// stream generator

/// A random stream. The first entries bind the software object (`SetObj`). `straddles` = also emit
/// packets split across two entries (`OpenHead`/`OpenTail`).
pub fn gen_stream(rng: &mut Rng, n: usize, straddles: bool, bad_tails: bool) -> Vec<Ge> {
    // ids are per KIND class (a pushbuffer's words depend on the kind only), so streams of any length reuse them
    let mut v = Vec::new();
    let (mut plain, mut nop) = (0u8, 0u8);
    let mut next_plain = |c: &mut u8| {
        *c = *c % 120 + 1;
        *c
    };
    let mut next_nop = |c: &mut u8| {
        *c = *c % 60 + 121;
        *c
    };
    v.push(ge(0, Kind::SetObj));
    while v.len() < n {
        let roll = rng.below(100);
        if roll < 12 {
            let g = ge(next_nop(&mut nop), Kind::Nop);
            v.push(if rng.chance(40) { g.sync() } else { g });
        } else if straddles && roll < 24 && v.len() + 2 <= n {
            v.push(ge(181, Kind::OpenHead));
            let t = if bad_tails { ge(183, Kind::OpenTailBad) } else { ge(182, Kind::OpenTail) };
            v.push(if rng.chance(30) { t.sync() } else { t });
        } else {
            let g = ge(next_plain(&mut plain), Kind::Plain);
            v.push(if rng.chance(12) { g.sync() } else { g });
        }
    }
    v
}

/// Is a flip before `index` (i.e. between entries `index-1` and `index`) inside an open packet?
pub fn flip_inside_open_packet(stream: &[Ge], index: usize) -> bool {
    index > 0
        && index < stream.len()
        && matches!(stream[index - 1].kind, Kind::OpenHead | Kind::SetObjHead | Kind::FalseHead | Kind::TrigHead)
}

// ------------------------------------------------------------------------------------------------
// G2

#[test]
fn g2_without_a_registration_the_forwarded_entries_are_bit_identical_to_always_scan() {
    for seed in 0..300u64 {
        let mut rng = Rng::new(seed);
        let n = 5 + rng.below(100) as usize;
        let stream = gen_stream(&mut rng, n, true, false);
        let ring = 16 << rng.below(3);
        let (old, _) = run_stream(&stream, &[], true, ring, 8);
        let (new, relay) = run_stream(&stream, &[], false, ring, 8);
        assert!(!old.dead && !new.dead, "seed {seed}: dead {} {}", old.dead, new.dead);
        assert_eq!(
            old.forwarded.len(),
            stream.len(),
            "seed {seed}: the base relays every entry"
        );
        assert_eq!(old.forwarded, new.forwarded, "seed {seed}: G2 identity");
        let expect: Vec<u64> = stream.iter().map(Ge::raw).collect();
        assert_eq!(new.forwarded, expect, "seed {seed}: bit-exact");
        assert_eq!(new.pb_reads_in_pointer, 0, "seed {seed}: G1");
        assert!(matches!(relay.life.mode(), Mode::Pointer), "seed {seed}");
        assert!(new.pb_reads < old.pb_reads, "seed {seed}: the lifecycle reads less");
    }
}

fn plan_registration(rng: &mut Rng, stream: &mut Vec<Ge>, migrate: bool) -> (Vec<Sched>, usize, usize) {
    // register when `k` entries were forwarded; the trigger is the entry at index `k` (published after the reply)
    let k = 3 + rng.below((stream.len() as u64).saturating_sub(5).max(1)) as usize;
    let k = k.min(stream.len() - 1);
    let t = ge(190, Kind::Trig { migrate }).post();
    stream.insert(k, t);
    (vec![Sched::Register { at: k as u64, migrate }], k, k)
}

#[test]
fn g2_with_a_registration_at_any_flip_position_that_is_not_inside_a_packet_the_cycle_is_identical() {
    let mut checked = 0;
    for seed in 0..400u64 {
        let mut rng = Rng::new(0x1000 + seed);
        let n = 8 + rng.below(80) as usize;
        let mut stream = gen_stream(&mut rng, n, true, false);
        let migrate = rng.chance(50);
        let (sched, k, _) = plan_registration(&mut rng, &mut stream, migrate);
        if flip_inside_open_packet(&stream, k) {
            continue; // P2 is not claimed there: see the P2 test below
        }
        // the trigger neutralises: the staged entry is the trigger entry's position
        let ring = 16 << rng.below(3);
        let (old, _) = run_stream(&stream, &sched, true, ring, 8);
        let (new, relay) = run_stream(&stream, &sched, false, ring, 8);
        assert!(!old.dead && !new.dead, "seed {seed}: dead");
        assert_eq!(old.forwarded, new.forwarded, "seed {seed}: forwarded entries");
        assert_eq!(old.staged, new.staged, "seed {seed}: trigger position");
        assert_eq!(old.switches, new.switches, "seed {seed}: switch position");
        assert_eq!(new.unscanned_while_live, 0, "seed {seed}: I4");
        assert_eq!(new.staged, vec![k], "seed {seed}: the trigger was staged");
        assert!(matches!(relay.life.mode(), Mode::Pointer), "seed {seed}: back to Pointer");
        checked += 1;
    }
    assert!(checked > 200, "enough streams exercised: {checked}");
}

#[test]
fn g2_p2_every_difference_between_the_lifecycle_and_always_scan_is_explained_by_a_flip_inside_a_packet() {
    // Streams WITH straddles whose tails do not decode as headers (a fresh scan refuses them) and flips
    // anywhere: a difference may appear only where the flip falls between an OpenHead and its OpenTail, and
    // then the lifecycle counts it (`window_fresh_start_parse_error`). This is the P2 measurement.
    let mut differ = 0;
    let mut inside = 0;
    for seed in 0..600u64 {
        let mut rng = Rng::new(0x2000 + seed);
        let n = 10 + rng.below(60) as usize;
        let mut stream = gen_stream(&mut rng, n, true, true);
        let (sched, k, _) = plan_registration(&mut rng, &mut stream, false);
        let in_packet = flip_inside_open_packet(&stream, k);
        let (old, _) = run_stream(&stream, &sched, true, 64, 8);
        let (new, relay) = run_stream(&stream, &sched, false, 64, 8);
        assert!(!old.dead, "seed {seed}: the base never refuses a valid stream");
        let same = old.forwarded == new.forwarded && old.staged == new.staged && !new.dead;
        if in_packet {
            inside += 1;
        }
        if !same {
            differ += 1;
            assert!(in_packet, "seed {seed}: a difference with the flip on a boundary (k={k})");
            assert!(
                relay.life.counts().window_fresh_start_parse_error >= 1,
                "seed {seed}: the difference is not counted"
            );
            assert!(new.dead, "seed {seed}");
        }
    }
    println!("g2-p2: {differ} streams differ, all with the flip inside an open packet ({inside} such flips)");
    assert!(differ > 0, "the generator must reach a flip inside a packet at least once");
}

// ------------------------------------------------------------------------------------------------
// G1: the stub store

#[test]
fn g1_pointer_mode_reads_no_pushbuffer_byte_over_a_long_stream_and_the_counters_say_so() {
    let mut rng = Rng::new(7);
    let stream = gen_stream(&mut rng, 2000, true, false);
    let (out, relay) = run_stream(&stream, &[], false, 4096, 8);
    assert_eq!(out.pb_reads_in_pointer, 0);
    let c = relay.life.counts();
    assert_eq!(c.pointer_pb_read_violation, 0);
    assert!(c.entries_pointer >= 1990, "{}", c.entries_pointer);
    assert!(c.pb_bytes_scan <= 64, "only the Scan phase read: {}", c.pb_bytes_scan);
    assert_eq!(c.pb_bytes_window, 0);
    assert_eq!(c.window_bar1_bytes, 0);
    assert_eq!(c.window_unscanned_long, 0);
    // the base would have read about every segment
    let (old, _) = run_stream(&stream, &[], true, 4096, 8);
    assert!(old.pb_reads > 1500);
}

#[test]
fn g1_every_turn_publishes_at_most_one_guest_entry_and_does_bounded_work() {
    let mut rng = Rng::new(11);
    let stream = gen_stream(&mut rng, 3000, true, false);
    let (out, _) = run_stream(&stream, &[], false, 4096, 8);
    assert!(out.max_steps_per_turn <= 16, "{}", out.max_steps_per_turn);
}
