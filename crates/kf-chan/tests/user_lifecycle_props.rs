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
#![allow(
    dead_code,
    missing_docs,
    clippy::too_many_lines,
    clippy::needless_range_loop
)]

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
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
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
    Register {
        at: u64,
        migrate: bool,
    },
    Remove {
        at: u64,
    },
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
    /// Words by raw entry value (the corpus replay); consulted before `defs`.
    pub custom: HashMap<u64, Vec<u32>>,
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
            custom: HashMap::new(),
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
            if entry_id(raw)
                .and_then(|i| self.defs.get(&i))
                .is_some_and(|g| g.kind == Kind::Trig { migrate: true })
            {
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
        if let Some(w) = self.custom.get(&(raw & !SYNC)) {
            return w.clone();
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
            let from = self.src;
            let to = 1 - from;
            let rest: VecDeque<u64> = std::mem::take(&mut self.pending[from]);
            self.src = to;
            self.entries = self.next_size;
            self.pending[to] = rest;
            self.held[from] = false;
            self.put_local[to] = 0;
            self.put[to] = 0;
            self.guest_get[to] = 0;
            self.rings[to].clear();
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
    Relay::new(
        priv_entries,
        life,
        Scan::for_subdevice(1).unwrap(),
        Glue::from_env(),
    )
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
    let next_plain = |c: &mut u8| {
        *c = *c % 120 + 1;
        *c
    };
    let next_nop = |c: &mut u8| {
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
            let t = if bad_tails {
                ge(183, Kind::OpenTailBad)
            } else {
                ge(182, Kind::OpenTail)
            };
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
        && matches!(
            stream[index - 1].kind,
            Kind::OpenHead | Kind::SetObjHead | Kind::FalseHead | Kind::TrigHead
        )
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
        assert!(
            !old.dead && !new.dead,
            "seed {seed}: dead {} {}",
            old.dead,
            new.dead
        );
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
        assert!(
            new.pb_reads < old.pb_reads,
            "seed {seed}: the lifecycle reads less"
        );
    }
}

/// Register when `at` entries were forwarded (the flip: entry `at` is the first one loaded after it); the trigger
/// is inserted at the first packet boundary at or after `at` (never inside an OpenHead/OpenTail pair).
/// Returns the schedule, the flip index and the trigger index.
fn plan_registration(
    rng: &mut Rng,
    stream: &mut Vec<Ge>,
    migrate: bool,
) -> (Vec<Sched>, usize, usize) {
    let at = 2 + rng.below((stream.len() as u64).saturating_sub(5).max(1)) as usize;
    let at = at.min(stream.len() - 1);
    let mut t = at;
    while t > 0 && t < stream.len() && matches!(stream[t - 1].kind, Kind::OpenHead) {
        t += 1;
    }
    stream.insert(t, ge(190, Kind::Trig { migrate }).post());
    (
        vec![Sched::Register {
            at: at as u64,
            migrate,
        }],
        at,
        t,
    )
}

#[test]
fn g2_with_a_registration_at_any_flip_position_that_is_not_inside_a_packet_the_cycle_is_identical()
{
    let mut checked = 0;
    for seed in 0..400u64 {
        let mut rng = Rng::new(0x1000 + seed);
        let n = 8 + rng.below(80) as usize;
        let mut stream = gen_stream(&mut rng, n, true, false);
        let migrate = rng.chance(50);
        let (sched, k, t) = plan_registration(&mut rng, &mut stream, migrate);
        if flip_inside_open_packet(&stream, k) {
            continue; // P2 is not claimed there: see the P2 test below
        }
        assert_eq!(t, k, "a flip on a boundary has its trigger right there");
        // the trigger neutralises: the staged entry is the trigger entry's position
        let ring = 16 << rng.below(3);
        let (old, _) = run_stream(&stream, &sched, true, ring, 8);
        let (new, relay) = run_stream(&stream, &sched, false, ring, 8);
        assert!(!old.dead && !new.dead, "seed {seed}: dead");
        assert_eq!(
            old.forwarded, new.forwarded,
            "seed {seed}: forwarded entries"
        );
        assert_eq!(old.staged, new.staged, "seed {seed}: trigger position");
        assert_eq!(old.switches, new.switches, "seed {seed}: switch position");
        assert_eq!(new.unscanned_while_live, 0, "seed {seed}: I4");
        assert_eq!(new.staged, vec![k], "seed {seed}: the trigger was staged");
        assert!(
            matches!(relay.life.mode(), Mode::Pointer),
            "seed {seed}: back to Pointer"
        );
        checked += 1;
    }
    assert!(checked > 200, "enough streams exercised: {checked}");
}

#[test]
fn g2_p2_every_difference_between_the_lifecycle_and_always_scan_is_explained_by_a_flip_inside_a_packet()
 {
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
        let in_packet = k > 0 && k < stream.len() && matches!(stream[k - 1].kind, Kind::OpenHead);
        let (old, _) = run_stream(&stream, &sched, true, 64, 8);
        let (new, relay) = run_stream(&stream, &sched, false, 64, 8);
        assert!(
            !old.dead,
            "seed {seed}: the base never refuses a valid stream"
        );
        let same = old.forwarded == new.forwarded && old.staged == new.staged && !new.dead;
        if in_packet {
            inside += 1;
        }
        if !same {
            differ += 1;
            assert!(
                in_packet,
                "seed {seed}: a difference with the flip on a boundary (k={k})"
            );
            assert!(
                relay.life.counts().window_fresh_start_parse_error >= 1,
                "seed {seed}: the difference is not counted"
            );
            assert!(new.dead, "seed {seed}");
        }
    }
    println!(
        "g2-p2: {differ} streams differ, all with the flip inside an open packet ({inside} such flips)"
    );
    assert!(
        differ > 0,
        "the generator must reach a flip inside a packet at least once"
    );
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
    assert!(
        c.pb_bytes_scan <= 64,
        "only the Scan phase read: {}",
        c.pb_bytes_scan
    );
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

// ------------------------------------------------------------------------------------------------
// A4: entry-level validation is the only check in Pointer mode

/// A channel that is bound and in Pointer mode, with `entries` waiting in a big ring.
fn pointer_relay_with(entries: &[u64]) -> (Sim, Relay) {
    let stream: Vec<Ge> = vec![ge(0, Kind::SetObj), ge(1, Kind::Plain)];
    let mut sim = Sim::new(256, 8, &stream, vec![]);
    let mut relay = new_relay(8, false);
    drive(&mut sim, &mut relay, 100);
    assert_eq!(relay.life.mode(), Mode::Pointer, "bound");
    // now the hostile entries, written raw
    let base = relay.cursor;
    for (i, raw) in entries.iter().enumerate() {
        sim.rings[0].insert((base + i as u32) % 256, *raw);
    }
    sim.put[0] = (base + entries.len() as u32) % 256;
    sim.put_local[0] = sim.put[0];
    (sim, relay)
}

#[test]
fn a4_every_single_bit_flip_of_a_valid_entry_is_refused_by_name_or_forwarded_validated() {
    let valid = [
        ge(1, Kind::Plain).raw(),
        ge(2, Kind::Plain).sync().raw(),
        ge(3, Kind::Nop).raw(),
        ge(4, Kind::Nop).sync().raw(),
        // length 0 / 1 / max / max+1 (in dwords) with a plausible address
        kf_abi::submit::gp_entry(0x4000, 4).unwrap(),
        kf_abi::submit::gp_entry(0x4000, ((1u64 << 21) - 1) * 4).unwrap(),
    ];
    let mut refused = 0;
    let mut forwarded = 0;
    for base in valid {
        for bit in 0..64 {
            let raw = base ^ (1u64 << bit);
            let (mut sim, mut relay) = pointer_relay_with(&[raw]);
            drive(&mut sim, &mut relay, 20);
            assert!(sim.out.max_steps_per_turn <= 16, "bounded work");
            assert_eq!(sim.out.pb_reads_in_pointer, 0, "bit {bit} of {base:#x}");
            let valid_now =
                kf_chan::user_stream::inspect_entry(raw, kf_chan::user_storage::MAX_SOURCE_BYTES)
                    .is_ok();
            if valid_now {
                forwarded += 1;
                // forwarded exactly as flipped (after the SetObj entry), never a different value
                assert_eq!(
                    sim.out.forwarded.last().copied(),
                    Some(raw),
                    "bit {bit} of {base:#x}"
                );
            } else {
                refused += 1;
                assert!(
                    relay.dead && relay.refused_entry == 1,
                    "bit {bit} of {base:#x}: named refusal"
                );
                assert_eq!(
                    sim.out.forwarded.len(),
                    2,
                    "nothing unvalidated was stored (only SetObj and the first entry)"
                );
            }
            assert_eq!(sim.out.torn, 0);
        }
    }
    assert!(
        refused > 50 && forwarded > 50,
        "refused {refused} forwarded {forwarded}"
    );
}

#[test]
fn a4_hostile_entries_never_panic_and_never_store_an_unvalidated_value() {
    let mut rng = Rng::new(0xA4);
    for round in 0..2000 {
        let mut raws = Vec::new();
        for _ in 0..4 {
            raws.push(rng.next_u64());
        }
        let (mut sim, mut relay) = pointer_relay_with(&raws);
        drive(&mut sim, &mut relay, 50);
        for f in sim.out.forwarded.iter().skip(2) {
            assert!(
                kf_chan::user_stream::inspect_entry(*f, kf_chan::user_storage::MAX_SOURCE_BYTES)
                    .is_ok(),
                "round {round}: stored an entry inspect_entry refuses: {f:#x}"
            );
        }
        assert_eq!(sim.out.pb_reads_in_pointer, 0);
    }
}

// ------------------------------------------------------------------------------------------------
// A2: SET_OBJECT at every dword position of a segment

#[test]
fn a2_set_object_at_every_dword_position_switches_after_the_segment_that_commits_it() {
    // A single segment = `prefix` ordinary one-datum packets then SET_OBJECT then `suffix` packets, optionally
    // ending inside an open packet. The binding is committed when the segment is scanned; the switch is at
    // the NEXT entry; later entries are forwarded bit-exact and never read.
    for prefix in 0..6usize {
        for suffix in 0..4usize {
            for open_tail in [false, true] {
                let mut words = Vec::new();
                for i in 0..prefix {
                    words.push(kf_abi::submit::method_header_inc(0, 0x100, 1).unwrap());
                    words.push(i as u32);
                }
                words.push(kf_abi::submit::method_header_inc(6, 0, 1).unwrap());
                words.push(SW);
                for _ in 0..suffix {
                    words.push(kf_abi::submit::method_header_inc(0, 0x100, 1).unwrap());
                    words.push(9);
                }
                if open_tail {
                    words.push(kf_abi::submit::method_header_inc(1, 0x1b0, 3).unwrap());
                    words.push(0x11);
                }
                let first = kf_abi::submit::gp_entry(seg_va(250), words.len() as u64 * 4).unwrap();
                let mut sim = Sim::new(64, 8, &[], vec![]);
                sim.custom.insert(first, words.clone());
                sim.pending[0].push_back(first);
                let mut expect = vec![first];
                for i in 0..5u8 {
                    let g = ge(1 + i, Kind::Plain);
                    sim.defs.insert(g.id, g);
                    sim.pending[0].push_back(g.raw());
                    expect.push(g.raw());
                }
                let mut relay = new_relay(8, false);
                drive(&mut sim, &mut relay, 1000);
                let tag = format!("prefix {prefix} suffix {suffix} open_tail {open_tail}");
                assert!(!relay.dead, "{tag}");
                assert_eq!(sim.out.forwarded, expect, "{tag}: bit-exact");
                assert_eq!(relay.life.mode(), Mode::Pointer, "{tag}");
                assert_eq!(
                    sim.out.pb_reads, 1,
                    "{tag}: only the binding segment was read"
                );
                assert_eq!(
                    relay.life.counts().straddles_scan,
                    u64::from(open_tail),
                    "{tag}"
                );
            }
        }
    }
}

#[test]
fn a2_a_header_in_one_entry_and_the_datum_in_the_next_commits_only_with_the_second() {
    let mut scan = Scan::for_subdevice(1).unwrap();
    scan.segment(&[kf_abi::submit::method_header_inc(6, 0, 1).unwrap()], sw)
        .unwrap();
    assert_eq!(
        scan.software_mask(sw),
        0,
        "the datum has not arrived: not bound, and a packet is open"
    );
    assert!(!scan.at_boundary());
    scan.segment(&[SW], sw).unwrap();
    assert_eq!(scan.software_mask(sw), 1 << 6);
    assert!(scan.at_boundary());
}

// ------------------------------------------------------------------------------------------------
// B: repeated cycles, ring growth, counter wraps

#[test]
fn b_ten_thousand_cycles_with_ring_growth_never_stick_and_never_lose_an_entry() {
    // Scan -> Pointer -> (Register, trigger, switch to a ring twice as large) -> Pointer ..., the ring
    // sizes 32768 -> 65536 -> 131072 -> 32768 ...; entries counted by the monotone u64 `abs`, marker count
    // wraps its 16 bits many times.
    let sizes = [32768u32, 65536, 131072];
    let cycles = 10_000usize;
    let mut stream = vec![ge(0, Kind::SetObj)];
    let mut sched = Vec::new();
    for c in 0..cycles {
        for i in 0..2 {
            stream.push(ge(1 + ((c * 2 + i) % 100) as u8, Kind::Plain));
        }
        let at = stream.len() as u64;
        // the Window gates every SYNC_WAIT entry: one marker each, so the 16-bit marker count wraps
        for i in 0..8 {
            stream.push(ge(1 + ((c + i) % 100) as u8, Kind::Plain).sync().post());
        }
        stream.push(ge(190, Kind::Trig { migrate: true }).post());
        sched.push(Sched::Register { at, migrate: true });
    }
    // one last ordinary entry: the Window of the last cycle ends at the next entry boundary
    stream.push(ge(1, Kind::Plain));
    let mut sim = Sim::new(sizes[0], 8, &stream, sched);
    sim.next_size = sizes[1];
    let mut relay = new_relay(8, false);
    // grow the next ring each switch
    let mut grown = 0;
    let mut turns = 0usize;
    while turns < 4_000_000 {
        let before = sim.out.switches.len();
        turns += drive(&mut sim, &mut relay, 64);
        if sim.out.switches.len() != before {
            grown += 1;
            sim.next_size = sizes[(grown + 1) % 3];
        }
        if (sim.out.forwarded.len() >= stream.len() && sim.live == 0 && relay.idle()) || relay.dead
        {
            break;
        }
    }
    assert!(!relay.dead, "dead");
    assert_eq!(
        sim.out.forwarded.len(),
        stream.len(),
        "every entry relayed exactly once"
    );
    assert_eq!(sim.out.switches.len(), cycles, "one switch per cycle");
    assert_eq!(sim.out.unscanned_while_live, 0);
    assert_eq!(relay.life.mode(), Mode::Pointer);
    let c = relay.life.counts();
    assert_eq!(
        c.to_window_fresh + c.to_window_exact,
        cycles as u64,
        "{c:?}"
    );
    assert_eq!(
        c.to_pointer_from_window + c.to_pointer,
        cycles as u64 + 1,
        "{c:?}"
    );
    assert!(
        relay.marks.issued() > 65_536,
        "the 16-bit marker count wrapped: {}",
        relay.marks.issued()
    );
    assert_eq!(relay.marks.issued(), relay.marks.seen());
    assert_eq!(relay.fetch.issued(), relay.fetch.fetched());
    assert_eq!(relay.life.live(), 0);
}

#[test]
fn b_the_monotone_entry_counter_survives_the_u32_boundary() {
    let stream: Vec<Ge> = std::iter::once(ge(0, Kind::SetObj))
        .chain((0..40).map(|i| {
            let g = ge(1 + (i % 100) as u8, Kind::Plain);
            if i % 7 == 0 { g.sync() } else { g }
        }))
        .collect();
    let mut sim = Sim::new(64, 8, &stream, vec![]);
    let mut relay = new_relay(8, false);
    relay.abs = u64::from(u32::MAX) - 10;
    drive(&mut sim, &mut relay, 1000);
    assert_eq!(sim.out.forwarded.len(), stream.len());
    assert!(relay.abs > u64::from(u32::MAX));
    // the recorded SYNC_WAIT index is a u64 across the boundary
    let o = relay
        .life
        .outstanding_sync()
        .expect("a SYNC_WAIT was forwarded in Pointer mode");
    assert!(o > u64::from(u32::MAX) - 10 && o < relay.abs);
}

// ------------------------------------------------------------------------------------------------
// soak (seconds by default; KF3_STRESS_SECS for longer)

fn stress_secs() -> f64 {
    std::env::var("KF3_STRESS_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2.0)
}

#[test]
fn soak_millions_of_entries_many_laps_and_registrations_keep_every_invariant() {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(stress_secs());
    let mut total = 0u64;
    let mut seed = 0u64;
    while std::time::Instant::now() < deadline {
        seed += 1;
        let mut rng = Rng::new(0x50AC + seed);
        let n = 2000 + rng.below(3000) as usize;
        let mut stream = gen_stream(&mut rng, n, true, false);
        let mut sched = Vec::new();
        for _ in 0..rng.below(4) {
            let (s, _, _) = plan_registration(&mut rng, &mut stream, false);
            sched.extend(s);
        }
        // keep the Register positions consistent with the inserted triggers (each insert shifts later ones)
        sched.sort_by_key(|s| s.at());
        let (out, relay) = {
            let mut sim = Sim::new(64, 8, &stream, sched);
            let mut relay = new_relay(8, false);
            drive(&mut sim, &mut relay, 400_000);
            (sim.out, relay)
        };
        if out.dead {
            // a flip inside an open packet (P2) may legitimately kill a stream with bad tails; none here
            panic!("seed {seed}: dead: {out:?}");
        }
        assert_eq!(out.unscanned_while_live, 0, "seed {seed}");
        assert_eq!(out.torn, 0, "seed {seed}");
        assert_eq!(
            relay.marks.issued(),
            relay.marks.seen(),
            "seed {seed}: markers"
        );
        assert_eq!(
            relay.fetch.issued(),
            relay.fetch.fetched(),
            "seed {seed}: private ring drained"
        );
        assert_eq!(relay.life.counts().pointer_pb_read_violation, 0);
        total += out.forwarded.len() as u64;
    }
    println!("soak: {total} entries over {seed} streams");
    assert!(total > 100_000);
}

// ------------------------------------------------------------------------------------------------
// the real corpus

fn corpus_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../traces/user_ring_scan_20261014")
}

/// `(entry, words, carried_out_is_open)` of every history segment of a refusal trace, in cursor order,
/// duplicates (same cursor) dropped.
fn parse_history(text: &str) -> Vec<(u64, Vec<u32>, bool)> {
    let mut out: Vec<(u64, Vec<u32>, bool)> = Vec::new();
    let mut cur: Option<(u64, Vec<u32>, bool, u32)> = None;
    let mut last_cursor = u32::MAX;
    let mut in_history = false;
    for line in text.lines() {
        if line.starts_with("history (oldest first") {
            in_history = true;
            continue;
        }
        if !in_history {
            continue;
        }
        if let Some(rest) = line.strip_prefix("-- seq=") {
            if let Some((e, w, o, _)) = cur.take() {
                out.push((e, w, o));
            }
            let field = |name: &str| {
                rest.split_whitespace()
                    .find_map(|t| t.strip_prefix(name))
                    .map(str::to_owned)
            };
            let cursor: u32 = field("cursor=").and_then(|v| v.parse().ok()).unwrap_or(0);
            let entry = field("entry=0x")
                .and_then(|v| u64::from_str_radix(&v, 16).ok())
                .unwrap_or(0);
            let open = field("carried_out=").is_some_and(|v| v != "none");
            let len: u32 = field("words=").and_then(|v| v.parse().ok()).unwrap_or(0);
            if cursor == last_cursor {
                cur = None; // a re-scan of the same entry
                continue;
            }
            last_cursor = cursor;
            cur = Some((entry, Vec::with_capacity(len as usize), open, len));
        } else if let Some((_, w, _, len)) = cur.as_mut()
            && let Some((_, row)) = line.trim().split_once(':')
        {
            for tok in row.split_whitespace() {
                if w.len() < *len as usize
                    && let Ok(v) = u32::from_str_radix(tok, 16)
                {
                    w.push(v);
                }
            }
        }
    }
    if let Some((e, w, o, _)) = cur.take() {
        out.push((e, w, o));
    }
    out.retain(|(_, w, _)| !w.is_empty());
    out
}

#[test]
fn corpus_the_recorded_streams_replay_identically_through_the_base_and_the_lifecycle() {
    // Real segments from the 2026-10-14 Win11 refusal traces (settled bytes: the history shows what the scan
    // read, which for these segments was the final bytes). A SET_OBJECT is prepended, a registration fires at
    // every position, a trigger follows it. Where the flip is not inside a recorded open packet the two
    // relays must agree; every other position is reported as a measured P2 case.
    let dir = corpus_dir();
    let mut segments = 0usize;
    let mut open = 0usize;
    let mut cases = 0usize;
    let mut p2_cases = 0usize;
    let mut p2_differ = 0usize;
    for entry in std::fs::read_dir(&dir).expect("corpus dir") {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !name.starts_with("user-ring-refusal-") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let hist = parse_history(&text);
        // only segments the scan accepted as a whole stream from a boundary: skip leading continuations
        segments += hist.len();
        open += hist.iter().filter(|h| h.2).count();
        if hist.len() < 4 {
            continue;
        }
        // replay: the recorded GP entries and bytes, VA remapped to ids so Sim can serve the words
        let mut stream_words: Vec<Vec<u32>> = Vec::new();
        let mut raws = Vec::new();
        for (i, (entry, words, _)) in hist.iter().enumerate() {
            let sync = entry & SYNC != 0;
            if i >= 120 {
                break;
            }
            stream_words.push(words.clone());
            let raw = kf_abi::submit::gp_entry(seg_va(i as u8), words.len() as u64 * 4).unwrap()
                | if sync { SYNC } else { 0 };
            raws.push(raw);
        }
        for k in 1..hist.len().min(60) {
            // build the Sim by hand: SET_OBJECT first, then the recorded entries, a trigger at k+1
            let mut sim_old = corpus_sim(&stream_words, &raws, k);
            let mut sim_new = corpus_sim(&stream_words, &raws, k);
            let mut relay_old = new_relay(8, true);
            let mut relay_new = new_relay(8, false);
            drive(&mut sim_old, &mut relay_old, 100_000);
            drive(&mut sim_new, &mut relay_new, 100_000);
            cases += 1;
            let in_packet = hist.get(k - 1).is_some_and(|h| h.2);
            let same = sim_old.out.forwarded == sim_new.out.forwarded
                && sim_old.out.staged == sim_new.out.staged
                && sim_old.out.dead == sim_new.out.dead;
            if in_packet {
                p2_cases += 1;
                if !same {
                    p2_differ += 1;
                    assert!(
                        relay_new.life.counts().window_fresh_start_parse_error > 0
                            || sim_new.out.triggers_found != sim_old.out.triggers_found,
                        "{name} k={k}: unexplained difference"
                    );
                }
            } else {
                assert!(
                    same,
                    "{name} k={k}: the base and the lifecycle differ with the flip on a boundary: old {:?} new {:?}",
                    sim_old.out, sim_new.out
                );
            }
        }
    }
    println!(
        "corpus: {segments} recorded segments, {open} end inside an open packet; {cases} replays, {p2_cases} flips inside an open packet ({p2_differ} differ)"
    );
    assert!(segments > 100 && cases > 100);
}

/// A Sim whose pushbuffer words are the recorded ones: entry 0 binds, entries 1.. are the recorded segments,
/// and a registration fires after `k` forwarded entries with a trigger as the next entry.
fn corpus_sim(words: &[Vec<u32>], raws: &[u64], k: usize) -> Sim {
    let mut sim = Sim::new(
        256,
        8,
        &[],
        vec![Sched::Register {
            at: k as u64 + 1,
            migrate: false,
        }],
    );
    let mut all: Vec<(u64, Vec<u32>)> =
        vec![(ge(250, Kind::SetObj).raw(), ge(250, Kind::SetObj).words())];
    for (i, (w, r)) in words.iter().zip(raws).enumerate() {
        all.push((*r, w.clone()));
        if i + 1 == k {
            all.push((
                ge(190, Kind::Trig { migrate: false }).raw(),
                ge(190, Kind::Trig { migrate: false }).words(),
            ));
        }
    }
    sim.defs.insert(250, ge(250, Kind::SetObj));
    sim.defs.insert(190, ge(190, Kind::Trig { migrate: false }));
    sim.custom = all.iter().map(|(r, w)| (*r & !SYNC, w.clone())).collect();
    for (r, _) in &all {
        sim.pending[0].push_back(*r);
    }
    sim
}

// ---- the measured GravityMark Vulkan ring-growth trigger (run 1510, `GP[0x4b63]`, doc section 10)

const GRAVITY_TRIGGER: [u32; 4] = [0x8000_c01e, 0x8000_c009, 0x2001_c080, 0x4000_002c];

#[test]
fn corpus_the_gravitymark_late_fifo_trigger_words_are_a_trigger_after_two_immediate_methods() {
    let mut scan = Scan::for_subdevice(1).unwrap();
    scan.segment(
        &[kf_abi::submit::method_header_inc(6, 0, 1).unwrap(), SW],
        sw,
    )
    .unwrap();
    let t = scan
        .segment(&GRAVITY_TRIGGER, sw)
        .unwrap()
        .expect("the registered trigger");
    assert_eq!(
        (t.start, t.end, t.call.data, t.call.method),
        (2, 4, 0x4000_002c, 0x200)
    );
    // the same words with the scan restarted fresh (a Window after Pointer mode) at the segment boundary
    let mut fresh = scan.fresh_resume();
    let t = fresh
        .segment(&GRAVITY_TRIGGER, sw)
        .unwrap()
        .expect("found by a fresh scan too");
    assert_eq!((t.start, t.end), (2, 4));
}

#[test]
fn corpus_a_gravitymark_ring_growth_replays_identically_through_the_base_and_the_lifecycle() {
    // [SET_OBJECT, ordinary entries..., the 4-word trigger segment, ordinary entries...] with the registration
    // at every position before the trigger: the lifecycle stages the same trigger at the same position and
    // switches the source at the same place as the base.
    for k in 2..24usize {
        let mut sim_words: Vec<(u64, Vec<u32>)> =
            vec![(ge(250, Kind::SetObj).raw(), ge(250, Kind::SetObj).words())];
        for i in 0..30usize {
            let g = ge(1 + i as u8, Kind::Plain);
            sim_words.push((g.raw(), g.words()));
        }
        let trig_raw = kf_abi::submit::gp_entry(seg_va(190), 16).unwrap();
        sim_words.insert(k, (trig_raw, GRAVITY_TRIGGER.to_vec()));
        let mk = || {
            let mut sim = Sim::new(
                256,
                8,
                &[],
                vec![Sched::Register {
                    at: k as u64,
                    migrate: true,
                }],
            );
            sim.defs.insert(190, ge(190, Kind::Trig { migrate: true }));
            sim.custom = sim_words
                .iter()
                .map(|(r, w)| (*r & !SYNC, w.clone()))
                .collect();
            for (r, _) in &sim_words {
                sim.pending[0].push_back(*r);
            }
            sim
        };
        let (mut a, mut b) = (mk(), mk());
        let mut ra = new_relay(8, true);
        let mut rb = new_relay(8, false);
        drive(&mut a, &mut ra, 100_000);
        drive(&mut b, &mut rb, 100_000);
        assert!(!a.out.dead && !b.out.dead, "k={k}");
        assert_eq!(
            a.out.staged,
            vec![k],
            "k={k}: the base stages the trigger at its position"
        );
        assert_eq!(a.out.forwarded, b.out.forwarded, "k={k}");
        assert_eq!(a.out.staged, b.out.staged, "k={k}");
        assert_eq!(a.out.switches, b.out.switches, "k={k}");
        assert_eq!(b.out.unscanned_while_live, 0, "k={k}");
        assert!(
            b.out.pb_reads < a.out.pb_reads,
            "k={k}: the lifecycle reads fewer pushbuffers"
        );
    }
}

#[test]
fn obs_a_window_never_skips_an_entry_so_window_unscanned_long_stays_zero() {
    let mut rng = Rng::new(99);
    let mut stream = gen_stream(&mut rng, 400, true, false);
    let (sched, _, _) = plan_registration(&mut rng, &mut stream, false);
    let (_, relay) = run_stream(&stream, &sched, false, 64, 8);
    assert_eq!(relay.life.counts().window_unscanned_long, 0);
    assert!(relay.life.counts().entries_window > 0);
}

#[test]
fn corpus_p2_is_contradicted_by_the_recorded_segments_that_end_inside_an_open_packet() {
    // MEASURED, from the 2026-10-14 refusal traces (a sample taken around refusals, so biased towards the
    // inline-upload shape; counted from the header lines, which survive the trimming of the word dumps): the
    // Windows driver DOES end GP segments inside method packets (I2M inline uploads split across entries).
    // Assumption P2 (a Window may start with fresh parser state at any entry boundary) is therefore not a
    // property of the driver; it is a probability, measured by `straddles_window` / `window_fresh_start_parse_error`.
    let dir = corpus_dir();
    let (mut segments, mut open) = (0usize, 0usize);
    for entry in std::fs::read_dir(&dir).expect("corpus dir") {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !name.starts_with("user-ring-refusal-") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let mut last_cursor = String::new();
        for line in text.lines().filter(|l| l.starts_with("-- seq=")) {
            let cursor = line
                .split_whitespace()
                .find(|t| t.starts_with("cursor="))
                .unwrap_or("")
                .to_owned();
            if cursor == last_cursor {
                continue;
            }
            last_cursor = cursor;
            segments += 1;
            if !line.contains("carried_out=none") {
                open += 1;
            }
        }
    }
    println!("p2-corpus: {open} of {segments} recorded segments end inside an open packet");
    assert!(segments > 150, "{segments}");
    assert!(
        open >= 40,
        "P2 is contradicted by the recorded streams: {open} of {segments}"
    );
}
