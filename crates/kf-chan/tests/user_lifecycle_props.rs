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
use kf_chan::user_lifecycle::{Lifecycle, LifecycleMode, MarkerLatch, Mode};
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
    /// I8: reads at or beyond the `GP_PUT` loaded in the turn.
    pub beyond_put: u32,
    /// SYNC_WAIT gates published (barrier + marker pairs).
    pub barriers: u32,
    /// Indices (into `forwarded`) of the entries counted by `unscanned_while_live`.
    pub unscanned_at: Vec<usize>,
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
    pending: [VecDeque<(u64, bool)>; 2],
    /// For every entry the guest has published, in order: was it published after a reply (`post`)?
    fed_post: VecDeque<bool>,
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
    /// A cooperative guest publishes the entries marked `post` (published after a registration's reply) only
    /// after the reply: one credit per applied registration, consumed by the first entry of a run of `post` entries.
    post_credit: u32,
    in_post: bool,
    /// The guest publishes at most this many entries per `feed` (its `GP_PUT` then lands wherever that is).
    pub chunk: usize,
    /// Hostile: after every `feed`, valid-looking POISON entries sit in the slots just above `GP_PUT`.
    pub poison_above_put: bool,
    /// The channel's software object was freed.
    pub gone: bool,
    /// Words by raw entry value (the corpus replay); consulted before `defs`.
    pub custom: HashMap<u64, Vec<u32>>,
    /// PHASE 1c: the guest pauses after having published this many entries in total until the relay has READ a
    /// `GP_PUT` equal to its cursor at that point (so the submission end is observed as a boundary).
    pub pauses: Vec<usize>,
    /// The relay's cursor (entries relayed), set by `drive` before every feed.
    pub relayed_seen: u64,
    /// Entries published when the relay last read a caught-up `GP_PUT` (`usize::MAX` = never).
    pub observed: usize,
    /// Lazy-observe tests: the private ring is consumed at `rate` entries per turn instead of at once (`None`).
    pub rate: Option<usize>,
    /// ... but only every `rate_div`-th turn (0 or 1: every turn).
    pub rate_div: usize,
    ticks: usize,
    hw_pos: usize,
    marker_at: Vec<usize>,
    /// Counts of the reads the relay made.
    pub reads_put: u64,
    pub reads_hw: u64,
    /// The relay's cursor, set by `drive` before every feed.
    pub cursor_seen: u32,
    /// Per turn (when `log_turns`): did it read the private `USERD`, and did the guest ring look full at its `GP_PUT`?
    pub log_turns: bool,
    pub turn_log: Vec<(bool, bool)>,
    hw_flag: bool,
    /// The most entries the relay had consumed beyond what the guest's `GP_GET` shows, over the feeds that found the
    /// guest ring full as the guest sees it (the guest waits for a write-back).
    pub blocked_run: u32,
    pub blocked_max: u32,
    /// FNV-1a over EVERY call the relay made on this environment (name, arguments, result): the fingerprint of the
    /// whole interaction, for the bit-identity test against the base relay.
    pub trace: u64,
}

impl Sim {
    pub fn new(entries: u32, priv_entries: u32, stream: &[Ge], sched: Vec<Sched>) -> Self {
        let mut defs = HashMap::new();
        let mut pending = [VecDeque::new(), VecDeque::new()];
        for g in stream {
            defs.insert(g.id, *g);
            pending[0].push_back((g.raw(), g.post_reply));
        }
        Self {
            entries,
            rings: [HashMap::new(), HashMap::new()],
            src: 0,
            put: [0; 2],
            put_local: [0; 2],
            guest_get: [0; 2],
            pending,
            fed_post: VecDeque::new(),
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
            post_credit: 0,
            in_post: false,
            chunk: usize::MAX,
            poison_above_put: false,
            gone: false,
            custom: HashMap::new(),
            pauses: Vec::new(),
            relayed_seen: 0,
            observed: usize::MAX,
            trace: 0xcbf2_9ce4_8422_2325,
            rate: None,
            rate_div: 1,
            ticks: 0,
            hw_pos: 0,
            marker_at: Vec::new(),
            reads_put: 0,
            reads_hw: 0,
            blocked_run: 0,
            blocked_max: 0,
            cursor_seen: 0,
            log_turns: false,
            turn_log: Vec::new(),
            hw_flag: false,
        }
    }

    /// The PBDMA consumes `rate` more private entries (lag mode only).
    fn tick(&mut self) {
        if let Some(r) = self.rate {
            self.ticks += 1;
            if self.ticks.is_multiple_of(self.rate_div.max(1)) {
                self.hw_pos = (self.hw_pos + r).min(self.log_len);
            }
        }
    }

    /// Fold one call into the trace.
    fn mix(&mut self, tag: u64, a: u64, b: u64) {
        for v in [tag, a, b] {
            self.trace = (self.trace ^ v).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    /// The guest publishes as many pending entries as the ring has room for, then GP_PUT.
    fn feed(&mut self) {
        let r = self.src;
        let mut fed = 0usize;
        let mut blocked = false;
        while let Some(&(raw, post)) = self.pending[r].front() {
            if fed >= self.chunk {
                break;
            }
            fed += 1;
            if self.pauses.contains(&self.fed_post.len()) && self.observed != self.fed_post.len() {
                break;
            }
            if self.held[r] || (self.put_local[r] + 1) % self.entries == self.guest_get[r] {
                if !self.held[r] {
                    blocked = true;
                }
                break;
            }
            if post && !self.in_post {
                if self.post_credit == 0 {
                    break; // the reply has not been seen yet: a cooperative guest waits for it
                }
                self.post_credit -= 1;
            }
            self.in_post = post;
            self.fed_post.push_back(post);
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
        if blocked {
            // how many entries the relay has consumed that the guest's `GP_GET` does not show yet, at a moment when the
            // guest is waiting for exactly that
            let lag = (self.cursor_seen + self.entries - self.guest_get[r]) % self.entries;
            self.blocked_max = self.blocked_max.max(lag);
        }
        if self.poison_above_put {
            // only FREE slots (not yet published by the guest, already consumed by the relay's write-back)
            let free = (self.guest_get[r] + self.entries - self.put_local[r] - 1) % self.entries;
            for k in 0..3u32.min(free) {
                let slot = (self.put_local[r] + k) % self.entries;
                self.rings[r].insert(slot, ge(251, Kind::Plain).raw());
            }
        }
    }

    fn apply_sched(&mut self, forwarded: u64) {
        while let Some(i) = self.sched.iter().position(|s| s.at() <= forwarded) {
            match self.sched.remove(i) {
                Sched::Register { migrate, .. } => {
                    self.post_credit += 1;
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
    fn software_gone(&self) -> bool {
        self.gone
    }
    fn gp_put(&mut self) -> u32 {
        if self.relayed_seen as usize == self.fed_post.len() {
            self.observed = self.fed_post.len();
        }
        let v = self.put[self.src];
        self.reads_put += 1;
        self.mix(1, u64::from(v), 0);
        v
    }
    fn gp_dword(&mut self, slot: u32, hi: bool) -> u32 {
        let raw = self.rings[self.src].get(&slot).copied().unwrap_or(0);
        let v = if hi { (raw >> 32) as u32 } else { raw as u32 };
        self.mix(2, u64::from(slot) << 1 | u64::from(hi), u64::from(v));
        v
    }
    fn registry_epoch(&mut self) -> u64 {
        let v = self.epoch;
        self.mix(3, v, 0);
        v
    }
    fn registry_live(&mut self) -> u32 {
        let v = self.live;
        self.mix(4, u64::from(v), 0);
        v
    }
    fn hw_get(&mut self) -> u32 {
        self.reads_hw += 1;
        self.hw_flag = true;
        let pos = if self.rate.is_some() {
            self.hw_pos
        } else {
            self.log_len
        };
        let v = (pos as u32) & (self.priv_entries - 1);
        self.mix(5, u64::from(v), 0);
        v
    }
    fn cached_put_ok(&mut self, put: u32, cursor: u32) -> bool {
        // the guest only advances `GP_PUT` within a ring generation: a remembered value is never beyond the real one
        let n = self.entries;
        let real = self.put[self.src];
        put < n && (put.wrapping_sub(cursor) & (n - 1)) <= (real.wrapping_sub(cursor) & (n - 1))
    }
    fn hw_reference(&mut self) -> u32 {
        let done = if self.rate.is_some() {
            self.marker_at.iter().filter(|p| **p <= self.hw_pos).count() as u64
        } else {
            self.markers_done
        };
        let v = if done == 0 {
            0
        } else {
            MARK_TAG | (done as u32 & 0xffff)
        };
        self.mix(6, u64::from(v), 0);
        v
    }
    fn read_pb(&mut self, raw: u64) -> Vec<u32> {
        self.mix(7, raw, 0);
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
        self.mix(8, format!("{p:?}").len() as u64, self.log_len as u64);
        for b in format!("{p:?}").bytes() {
            self.mix(9, u64::from(b), 0);
        }
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
                // I4: an entry published AFTER the registration's reply (`post`) must have been scanned.
                // Entries below the anchor were published before the reply and are forwarded unscanned.
                let post = self.fed_post.pop_front().unwrap_or(false);
                if self.live > 0 && !scanned && post {
                    let nop = entry_id(tested)
                        .and_then(|i| self.defs.get(&i))
                        .is_some_and(|g| g.kind == Kind::Nop);
                    if !nop {
                        self.out.unscanned_while_live += 1;
                        self.out.unscanned_at.push(self.out.forwarded.len());
                    }
                }
                self.out.forwarded.push(stored);
            }
            Pub::Staged { .. } => {
                self.fed_post.pop_front();
                self.out.staged.push(self.out.forwarded.len());
                self.out.forwarded.push(u64::MAX); // placeholder: the staged copy replaces the entry
            }
            Pub::Barrier => self.out.barriers += 1,
            Pub::Marker(_) => {
                self.markers_done += 1;
                self.marker_at.push(self.log_len);
                self.out.markers += 1;
            }
        }
    }
    fn write_guest_get(&mut self, v: u32) {
        self.mix(10, u64::from(v), 0);
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
            let rest: VecDeque<(u64, bool)> = std::mem::take(&mut self.pending[from]);
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
        for b in format!("{e:?}").bytes() {
            self.mix(11, u64::from(b), 0);
        }
        match e {
            Ev::ScanRefused => {
                self.out.refused_scan += 1;
                self.out.dead = true;
            }
            Ev::EntryRefused | Ev::PutBound => self.out.dead = true,
            Ev::TriggerFound => self.out.triggers_found += 1,
            Ev::BeyondPut => self.out.beyond_put += 1,
            _ => {}
        }
    }
}

/// Run `relay` over `sim` until it has nothing more to do. Returns the number of turns.
pub fn drive(sim: &mut Sim, relay: &mut Relay, max_turns: usize) -> usize {
    let mut turns = 0;
    let mut idle_turns = 0;
    // a lagging PBDMA needs time to catch up with a relay that waits for it
    let patience = if sim.rate.is_some() { 400 } else { 6 };
    while turns < max_turns {
        sim.tick();
        sim.apply_sched(relay.relayed);
        sim.relayed_seen = relay.relayed;
        sim.cursor_seen = relay.cursor;
        sim.feed();
        if relay.dead {
            break;
        }
        let before = (relay.puts, relay.cursor, relay.relayed, relay.generation);
        sim.hw_flag = false;
        relay.claim();
        let mut steps = 0;
        while !relay.idle() {
            sim.mode_hint = relay.life.mode();
            relay.step(sim);
            steps += 1;
            assert!(steps < 64, "a turn must be bounded");
        }
        if sim.log_turns {
            sim.turn_log.push((sim.hw_flag, relay.put_full));
        }
        sim.out.max_steps_per_turn = sim.out.max_steps_per_turn.max(steps);
        turns += 1;
        let after = (relay.puts, relay.cursor, relay.relayed, relay.generation);
        if before == after && !relay.again {
            idle_turns += 1;
            // nothing moved: a pending registration that has not fired yet is the only reason to go on
            if idle_turns > 2
                && sim.pending[sim.src].is_empty()
                && sim.sched.is_empty()
                && (sim.rate.is_none() || sim.hw_pos >= sim.log_len)
            {
                break;
            }
            if idle_turns > patience {
                break;
            }
        } else {
            idle_turns = 0;
        }
    }
    turns
}

pub fn new_relay(priv_entries: u32, always_scan: bool) -> Relay {
    new_relay_mode(
        priv_entries,
        if always_scan {
            LifecycleMode::AlwaysScan
        } else {
            LifecycleMode::Default
        },
    )
}

pub fn new_relay_mode(priv_entries: u32, mode: LifecycleMode) -> Relay {
    Relay::new(
        priv_entries,
        Lifecycle::new().with_mode(mode),
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
    run_stream_mode(
        stream,
        sched,
        if always_scan {
            LifecycleMode::AlwaysScan
        } else {
            LifecycleMode::Default
        },
        entries,
        priv_entries,
    )
}

pub fn run_stream_mode(
    stream: &[Ge],
    sched: &[Sched],
    mode: LifecycleMode,
    entries: u32,
    priv_entries: u32,
) -> (Out, Relay) {
    let mut sim = Sim::new(entries, priv_entries, stream, sched.to_vec());
    let mut relay = new_relay_mode(priv_entries, mode);
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

/// PHASE 1c (review 2, N2): every measurement-class mode forwards exactly the guest's entries (bit-exact, in order,
/// none lost or duplicated) and each shows its own observable difference: `default` and `no_anchored_windows` read no
/// pushbuffer in Pointer mode, `always_scan` reads every entry and never leaves Scan, `no_pointer_markers` is the
/// default minus the Pointer-mode markers (the wake falls back to a look).
#[test]
fn modes_every_mode_forwards_the_same_entries_and_each_has_its_observable_difference() {
    let mut default_markers = 0;
    for seed in 0..120u64 {
        let mut rng = Rng::new(seed ^ 0x5eed);
        let n = 8 + rng.below(80) as usize;
        let stream = gen_stream(&mut rng, n, true, false);
        let ring = 16 << rng.below(3);
        let expect: Vec<u64> = stream.iter().map(Ge::raw).collect();
        let nonnop = stream.iter().filter(|g| g.kind != Kind::Nop).count() as u64;
        let mut by_mode = Vec::new();
        for mode in LifecycleMode::ALL {
            let (out, relay) = run_stream_mode(&stream, &[], mode, ring, 8);
            assert!(!out.dead, "seed {seed} {mode:?}: dead");
            assert_eq!(
                out.forwarded, expect,
                "seed {seed} {mode:?}: bit-exact, in order"
            );
            assert_eq!(out.beyond_put, 0, "seed {seed} {mode:?}: I8");
            assert_eq!(out.pb_reads_in_pointer, 0, "seed {seed} {mode:?}: G1");
            match mode {
                LifecycleMode::AlwaysScan => {
                    assert_eq!(relay.life.mode(), Mode::Scan, "seed {seed}");
                    assert_eq!(
                        u64::from(out.pb_reads),
                        nonnop,
                        "seed {seed}: every entry is read"
                    );
                    assert_eq!(relay.life.counts().entries_pointer, 0);
                }
                _ => {
                    assert_eq!(relay.life.mode(), Mode::Pointer, "seed {seed} {mode:?}");
                    assert!(u64::from(out.pb_reads) < nonnop, "seed {seed} {mode:?}");
                }
            }
            by_mode.push((mode, out.markers, relay.life.counts().pointer_markers));
        }
        let get = |m: LifecycleMode| by_mode.iter().find(|x| x.0 == m).copied().unwrap();
        assert_eq!(
            get(LifecycleMode::NoPointerMarkers).2,
            0,
            "seed {seed}: no Pointer marker in no_pointer_markers"
        );
        assert_eq!(
            get(LifecycleMode::AlwaysScan).2,
            0,
            "seed {seed}: nor in always_scan (it never leaves Scan)"
        );
        assert!(
            get(LifecycleMode::Default).1 >= get(LifecycleMode::NoPointerMarkers).1,
            "seed {seed}: the default places at least the markers the reduced mode does"
        );
        default_markers += get(LifecycleMode::Default).2;
    }
    assert!(
        default_markers > 0,
        "the default mode places Pointer markers (otherwise the comparison above proves nothing)"
    );
}

/// PHASE 1c item 2 (second review N1). A guest that ends a submission inside a packet while the parser state is known
/// (Scan) falsifies P2': Pointer markers go off VM-wide. Two channels share the latch: channel A sees the open
/// boundary, channel B (which never does) loses its Pointer markers too. Neither loses, duplicates or reorders an
/// entry, and B's unannounced wakes are counted.
#[test]
fn latch_an_open_gp_put_boundary_in_one_channel_switches_pointer_markers_off_for_the_whole_vm() {
    let mut b_without = 0;
    for seed in 0..80u64 {
        let mut rng = Rng::new(seed ^ 0x1a7c);
        let n = 10 + rng.below(60) as usize;
        let mut a_stream = gen_stream(&mut rng, n, false, false);
        a_stream[0] = ge(0, Kind::SetObjOpen);
        a_stream.insert(1, ge(184, Kind::OpenTail));
        let b_stream = gen_stream(&mut rng, n, false, false);
        let ring = 16 << rng.below(2);
        let latch = std::sync::Arc::new(MarkerLatch::default());

        let run =
            |stream: &[Ge], pauses: Vec<usize>, latch: Option<std::sync::Arc<MarkerLatch>>| {
                let mut sim = Sim::new(ring, 8, stream, vec![]);
                sim.pauses = pauses;
                let mut relay = new_relay_mode(8, LifecycleMode::Default);
                if let Some(l) = latch {
                    relay.life = relay.life.clone().with_latch(l);
                }
                drive(&mut sim, &mut relay, 200_000);
                sim.out.dead |= relay.dead;
                (sim.out, relay)
            };
        let a_expect: Vec<u64> = a_stream.iter().map(Ge::raw).collect();
        let (a, ra) = run(&a_stream, vec![1], Some(latch.clone()));
        assert!(!a.dead, "seed {seed}");
        assert_eq!(
            a.forwarded, a_expect,
            "seed {seed}: A loses/duplicates nothing"
        );
        assert_eq!(
            ra.life.counts().put_boundary_open_scan,
            1,
            "seed {seed}: the open boundary was observed"
        );
        assert_eq!(
            ra.life.counts().pointer_markers_latched_off,
            1,
            "seed {seed}"
        );
        assert_eq!(
            ra.life.counts().pointer_markers,
            0,
            "seed {seed}: A places no Pointer marker"
        );
        assert!(latch.is_set());
        let b_expect: Vec<u64> = b_stream.iter().map(Ge::raw).collect();
        let (b, rb) = run(&b_stream, vec![], Some(latch.clone()));
        assert!(!b.dead, "seed {seed}");
        assert_eq!(
            b.forwarded, b_expect,
            "seed {seed}: B loses/duplicates nothing"
        );
        assert_eq!(b.beyond_put, 0);
        assert_eq!(
            rb.life.counts().pointer_markers_latched_off,
            0,
            "seed {seed}: counted once in the VM"
        );
        assert_eq!(
            rb.life.counts().pointer_markers,
            0,
            "seed {seed}: B never saw an open packet but has no Pointer marker"
        );
        assert_eq!(rb.life.mode(), Mode::Pointer);
        // the control: the same B without the latch places Pointer markers
        let (_, rc) = run(&b_stream, vec![], None);
        b_without += rc.life.counts().pointer_markers;
    }
    assert!(
        b_without > 0,
        "the control places Pointer markers, so the comparison above means something"
    );
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

/// `count` registrations in separate regions of the stream, each with its own trigger (a cooperative guest has one
/// reply per trigger): the register position `at` is somewhere in the region, the trigger at the first packet
/// boundary at or after it. Triggers are inserted from the last region to the first and the positions adjusted.
fn plan_registrations_spaced(rng: &mut Rng, stream: &mut Vec<Ge>, count: usize) -> Vec<Sched> {
    let n = stream.len();
    let mut planned = Vec::new();
    for j in (0..count).rev() {
        let (lo, hi) = (2 + j * n / count.max(1), (j + 1) * n / count.max(1) - 3);
        if hi <= lo + 2 {
            continue;
        }
        let at = lo + rng.below((hi - lo) as u64) as usize;
        let mut t = at;
        while t > 0 && t < stream.len() && matches!(stream[t - 1].kind, Kind::OpenHead) {
            t += 1;
        }
        stream.insert(t, ge(190, Kind::Trig { migrate: false }).post());
        planned.push((j, at, t));
    }
    planned.sort();
    let mut sched = Vec::new();
    for (rank, (_, at, _)) in planned.iter().enumerate() {
        sched.push(Sched::Register {
            at: (*at + rank) as u64,
            migrate: false,
        });
    }
    sched
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
        // back to Pointer at the next entry; after a ring switch the new ring's first entries are scanned first
        // (D5), so the Window closes only at the entry after POST_SWITCH_SCAN of them
        let after = stream.len() - t - 1;
        let want_pointer =
            after >= 1 && (!migrate || after > kf_chan::user_lifecycle::POST_SWITCH_SCAN as usize);
        if want_pointer {
            assert!(
                matches!(relay.life.mode(), Mode::Pointer),
                "seed {seed}: back to Pointer ({after} entries after the trigger, migrate={migrate})"
            );
        }
        checked += 1;
    }
    assert!(checked > 200, "enough streams exercised: {checked}");
}

#[test]
fn g2_p2prime_with_every_submission_ending_on_a_packet_boundary_a_registration_anywhere_changes_nothing()
 {
    // Streams WITH straddles whose tails do not decode as headers (a scan restarted inside the packet refuses them)
    // and a registration at ANY position, including between an OpenHead and its OpenTail. The guest ends its
    // submission where the trigger goes (a packet boundary), so the Window starts at the anchor and the lifecycle
    // and the always-scan base agree EXACTLY, with no fresh-start parse error: the property the anchor buys
    // (review D1 / D2). The registration position inside a packet is reached by the generator.
    let mut inside = 0;
    for seed in 0..600u64 {
        let mut rng = Rng::new(0x2000 + seed);
        let n = 10 + rng.below(60) as usize;
        let mut stream = gen_stream(&mut rng, n, true, true);
        let (sched, k, _) = plan_registration(&mut rng, &mut stream, false);
        if k > 0 && k < stream.len() && matches!(stream[k - 1].kind, Kind::OpenHead) {
            inside += 1;
        }
        let (old, _) = run_stream(&stream, &sched, true, 64, 8);
        let (new, relay) = run_stream(&stream, &sched, false, 64, 8);
        assert!(
            !old.dead,
            "seed {seed}: the base never refuses a valid stream"
        );
        assert!(!new.dead, "seed {seed}: the lifecycle died (k={k})");
        assert_eq!(
            old.forwarded, new.forwarded,
            "seed {seed}: forwarded (k={k})"
        );
        assert_eq!(old.staged, new.staged, "seed {seed}: staged (k={k})");
        assert_eq!(
            relay.life.counts().window_fresh_start_parse_error,
            0,
            "seed {seed}"
        );
    }
    println!("g2-p2prime: 600 streams identical; {inside} registered inside an open packet");
    assert!(
        inside > 0,
        "the generator must register inside a packet at least once"
    );
}

#[test]
fn g2_p2prime_a_guest_that_ends_submissions_inside_packets_is_the_documented_residual_and_is_counted()
 {
    // The same streams, but the guest's GP_PUT lands after EVERY entry, wherever that is (assumption P2' FALSE). A
    // difference may then appear only where the anchor fell inside a packet; it is loud (a refusal counted as a
    // fresh-start parse error) and is the case `put_boundary_open_*` measures in Scan mode.
    let mut differ = 0;
    for seed in 0..600u64 {
        let mut rng = Rng::new(0x2000 + seed);
        let n = 10 + rng.below(60) as usize;
        let mut stream = gen_stream(&mut rng, n, true, true);
        let (sched, _, _) = plan_registration(&mut rng, &mut stream, false);
        let run = |always: bool| {
            let mut sim = Sim::new(64, 8, &stream, sched.clone());
            sim.chunk = 1;
            let mut relay = new_relay(8, always);
            drive(&mut sim, &mut relay, 200_000);
            sim.out.dead |= relay.dead;
            (sim.out, relay)
        };
        let (old, _) = run(true);
        let (new, relay) = run(false);
        assert!(!old.dead, "seed {seed}");
        if !(old.forwarded == new.forwarded && old.staged == new.staged && !new.dead) {
            differ += 1;
            assert!(
                relay.life.counts().window_fresh_start_parse_error >= 1
                    || new.triggers_found != old.triggers_found,
                "seed {seed}: an unexplained difference"
            );
        }
    }
    println!("g2-p2prime-residual: {differ} streams differ (all counted)");
    assert!(
        differ > 0,
        "the generator must put an anchor inside a packet at least once"
    );
}

/// The falsifier of P2' (design section 5): in Scan mode, where the parser state is known, a packet that is OPEN
/// where the cursor reaches a `GP_PUT` the worker read is counted, uncapped.
#[test]
fn p2prime_scan_mode_counts_a_packet_open_at_a_gp_put_boundary_and_one_that_is_not() {
    // before the binding: [OpenHead] | [OpenTail] | [SetObj] | plain ...; the guest rings after EVERY entry
    let stream = vec![
        ge(181, Kind::OpenHead),
        ge(182, Kind::OpenTail),
        ge(0, Kind::SetObj),
        ge(1, Kind::Plain),
        ge(2, Kind::Plain),
    ];
    let mut sim = Sim::new(64, 8, &stream, vec![]);
    sim.chunk = 1;
    let mut relay = new_relay(8, false);
    drive(&mut sim, &mut relay, 10_000);
    assert!(!relay.dead);
    let c = relay.life.counts();
    assert_eq!(
        (c.put_boundaries_scan, c.put_boundary_open_scan),
        (3, 1),
        "{c:?}: boundaries after OpenHead (open), OpenTail and SetObj (closed)"
    );
    assert_eq!(c.straddles_scan, 1);
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
                sim.pending[0].push_back((first, false));
                let mut expect = vec![first];
                for i in 0..5u8 {
                    let g = ge(1 + i, Kind::Plain);
                    sim.defs.insert(g.id, g);
                    sim.pending[0].push_back((g.raw(), false));
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
        // enough ordinary entries between two registrations for the previous Window to close (the first
        // POST_SWITCH_SCAN entries after a ring switch are scanned, D5)
        for i in 0..(3 + kf_chan::user_lifecycle::POST_SWITCH_SCAN as usize) {
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
    // ordinary entries after the last ring switch: its first POST_SWITCH_SCAN are scanned (D5), the Window of the
    // last cycle ends at the one after them
    for _ in 0..=kf_chan::user_lifecycle::POST_SWITCH_SCAN {
        stream.push(ge(1, Kind::Plain));
    }
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
    relay.relayed = u64::from(u32::MAX) - 10;
    drive(&mut sim, &mut relay, 1000);
    assert_eq!(sim.out.forwarded.len(), stream.len());
    assert!(relay.relayed > u64::from(u32::MAX));
    // the recorded SYNC_WAIT index is a u64 across the boundary
    let o = relay
        .life
        .outstanding_sync()
        .expect("a SYNC_WAIT was forwarded in Pointer mode");
    assert!(o > u64::from(u32::MAX) - 10 && o < relay.relayed);
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
        let regs = rng.below(4) as usize;
        let sched = plan_registrations_spaced(&mut rng, &mut stream, regs);
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
        assert_eq!(
            out.unscanned_while_live,
            0,
            "seed {seed}: at {:?} sched_len={} stream_len={}",
            out.unscanned_at,
            n,
            stream.len()
        );
        assert_eq!(out.torn, 0, "seed {seed}");
        assert_eq!(
            relay.marks.issued(),
            relay.marks.seen(),
            "seed {seed}: markers {} need={:?} phase={:?} again={} fwd={}/{} mode={:?}",
            relay.key(),
            relay.need,
            relay.phase,
            relay.again,
            out.forwarded.len(),
            stream.len(),
            relay.life.mode()
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

// Exercise the real soak in an isolated process, including its invariants and actual counts.
// A consumed scheduling budget must never choose a shorter seed prefix for this coverage.
fn soak_counts_with_expired_budget(seconds: &str) -> (u64, u64) {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "soak_millions_of_entries_many_laps_and_registrations_keep_every_invariant",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("KF3_STRESS_SECS", seconds)
        .output()
        .expect("run the actual soak with its scheduling budget already consumed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "budget={seconds}: {stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let counts = stdout
        .lines()
        .find_map(|line| line.split_once("soak: ").map(|(_, counts)| counts))
        .expect("actual soak counts");
    let words: Vec<_> = counts.split_whitespace().collect();
    let entries: u64 = words[0].parse().unwrap();
    let streams: u64 = words[3].parse().unwrap();
    assert_eq!(streams, 512, "the full deterministic seed set must run");
    assert!(entries >= 1_000_000, "every scheduled stream must be fully relayed");
    println!("soak coverage proof: budget={seconds} entries={entries} streams={streams}");
    (entries, streams)
}

#[test]
fn soak_zero_elapsed_budget_cannot_truncate_the_seed_set() {
    soak_counts_with_expired_budget("0");
}

#[test]
fn soak_already_expired_nanosecond_budget_cannot_truncate_entries() {
    soak_counts_with_expired_budget("0.000000001");
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
    sim.defs
        .insert(190, ge(190, Kind::Trig { migrate: false }).post());
    sim.custom = all.iter().map(|(r, w)| (*r & !SYNC, w.clone())).collect();
    for (r, _) in &all {
        sim.pending[0].push_back((*r, entry_id(*r) == Some(190)));
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
            sim.defs
                .insert(190, ge(190, Kind::Trig { migrate: true }).post());
            sim.custom = sim_words
                .iter()
                .map(|(r, w)| (*r & !SYNC, w.clone()))
                .collect();
            for (r, _) in &sim_words {
                sim.pending[0].push_back((*r, entry_id(*r) == Some(190)));
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

// ------------------------------------------------------------------------------------------------
// I8: nothing at or beyond the GP_PUT loaded in the turn is read

#[test]
fn i8_entries_just_above_gp_put_are_never_read_or_forwarded_and_gp_put_is_loaded_once_per_turn() {
    let mut rng = Rng::new(0x18);
    for seed in 0..40u64 {
        let mut r = Rng::new(seed + 5000);
        let n = 20 + r.below(60) as usize;
        let stream = gen_stream(&mut rng, n, true, false);
        for chunk in [1usize, 2, 5, usize::MAX] {
            let mut sim = Sim::new(32, 8, &stream, vec![]);
            sim.chunk = chunk;
            sim.poison_above_put = true;
            let mut relay = new_relay(8, false);
            drive(&mut sim, &mut relay, 100_000);
            assert!(!relay.dead, "seed {seed} chunk {chunk}");
            assert_eq!(sim.out.beyond_put, 0, "seed {seed} chunk {chunk}: I8");
            let poison = ge(251, Kind::Plain).raw();
            assert!(
                !sim.out.forwarded.contains(&poison),
                "seed {seed} chunk {chunk}: a poisoned slot above GP_PUT was forwarded"
            );
            let expect: Vec<u64> = stream.iter().map(Ge::raw).collect();
            assert_eq!(sim.out.forwarded, expect, "seed {seed} chunk {chunk}");
        }
    }
}

#[test]
fn i8_a_gp_put_that_moves_backwards_or_leaves_the_ring_reads_nothing_beyond_the_value_loaded() {
    let stream: Vec<Ge> = std::iter::once(ge(0, Kind::SetObj))
        .chain((1..10).map(|i| ge(i, Kind::Plain)))
        .collect();
    for hostile_put in [0u32, 1, 3, 31, 32, 33, 1000, u32::MAX] {
        let mut sim = Sim::new(32, 8, &stream, vec![]);
        let mut relay = new_relay(8, false);
        drive(&mut sim, &mut relay, 1000);
        // now the guest moves GP_PUT to a hostile value and rings
        sim.put[sim.src] = hostile_put;
        for _ in 0..8 {
            relay.claim();
            let mut steps = 0;
            while !relay.idle() {
                sim.mode_hint = relay.life.mode();
                relay.step(&mut sim);
                steps += 1;
                assert!(steps < 64);
            }
            if relay.dead {
                break;
            }
        }
        assert_eq!(sim.out.beyond_put, 0, "put={hostile_put}: I8");
        if hostile_put >= 32 {
            assert!(
                relay.dead,
                "put={hostile_put}: refused by name (USER_RING_PUT_BOUND)"
            );
        }
    }
}

// ------------------------------------------------------------------------------------------------
// D6: a torn-down channel and a freed software object

#[test]
fn d6_a_torn_down_machine_reads_nothing_and_publishes_nothing() {
    let stream: Vec<Ge> = std::iter::once(ge(0, Kind::SetObj))
        .chain((1..20).map(|i| ge(i, Kind::Plain)))
        .collect();
    let mut sim = Sim::new(64, 8, &stream[..6], vec![]);
    let mut relay = new_relay(8, false);
    drive(&mut sim, &mut relay, 1000);
    assert_eq!(sim.out.forwarded.len(), 6);
    relay.life.teardown();
    let (reads, fwd) = (sim.out.pb_reads, sim.out.forwarded.len());
    for g in &stream[6..] {
        sim.pending[0].push_back((g.raw(), false));
        sim.defs.insert(g.id, *g);
    }
    drive(&mut sim, &mut relay, 1000);
    assert_eq!(
        sim.out.forwarded.len(),
        fwd,
        "nothing published after teardown"
    );
    assert_eq!(sim.out.pb_reads, reads, "nothing read after teardown");
    assert!(relay.life.counts().after_teardown >= 1);
}

#[test]
fn d6_pointer_mode_refuses_a_freed_software_object_like_the_scan_path_and_counts_it() {
    let stream: Vec<Ge> = std::iter::once(ge(0, Kind::SetObj))
        .chain((1..20).map(|i| ge(i, Kind::Plain)))
        .collect();
    let mut sim = Sim::new(64, 8, &stream[..5], vec![]);
    let mut relay = new_relay(8, false);
    drive(&mut sim, &mut relay, 1000);
    assert_eq!(relay.life.mode(), Mode::Pointer);
    sim.gone = true;
    for g in &stream[5..] {
        sim.pending[0].push_back((g.raw(), false));
        sim.defs.insert(g.id, *g);
    }
    drive(&mut sim, &mut relay, 1000);
    assert!(
        relay.dead,
        "refused by name (USER_RING_SOFTWARE_OBJECT_GONE)"
    );
    assert_eq!(relay.life.counts().pointer_software_gone, 1);
    assert_eq!(
        sim.out.forwarded.len(),
        5,
        "nothing forwarded for a freed object"
    );
}

// ------------------------------------------------------------------------------------------------
// D4: the sub-device mask state survives the Pointer stretch

#[test]
fn d4_a_stored_sub_device_mask_is_still_stored_when_a_window_scans_its_use() {
    // STORE at setup (scanned, before the binding that starts Pointer mode), a Pointer stretch, a registration, then a USE (header-only control) and the
    // trigger: a scan restarted with the mask state at its birth value refuses the USE ("nothing stored") and the
    // channel is lost.
    let stream = vec![
        ge(1, Kind::MaskStore),
        ge(0, Kind::SetObj),
        ge(2, Kind::Plain),
        ge(3, Kind::Plain),
        ge(4, Kind::Plain),
        ge(5, Kind::MaskUse).post(),
        ge(190, Kind::Trig { migrate: false }).post(),
        ge(6, Kind::Plain),
    ];
    let sched = [Sched::Register {
        at: 5,
        migrate: false,
    }];
    for always in [true, false] {
        let (out, relay) = run_stream(&stream, &sched, always, 32, 8);
        assert!(!out.dead, "always_scan={always}: the channel was lost");
        assert_eq!(
            out.staged,
            vec![6],
            "always_scan={always}: the trigger was staged"
        );
        assert_eq!(out.forwarded.len(), stream.len(), "always_scan={always}");
        assert_eq!(
            relay.life.mode(),
            if always { Mode::Scan } else { Mode::Pointer }
        );
    }
}

// ------------------------------------------------------------------------------------------------
// T3: the gate proof of a window does not survive a ring lap (a second angle on the stale-proof invariant)

#[test]
fn t3_a_second_window_one_ring_lap_later_gates_its_sync_wait_entry_again() {
    // 4-slot guest ring. Window 1 gates its trigger (a SYNC_WAIT entry at slot 1) and proves it at (generation 1,
    // cursor 1). Pointer mode laps the ring. Window 2 starts with a SYNC_WAIT entry at slot 1 again: the proof of
    // window 1 must be gone, so the entry is gated AGAIN (a second barrier + marker), not waved through.
    let stream = vec![
        ge(0, Kind::SetObj),
        ge(190, Kind::Trig { migrate: false }).sync().post(),
        ge(2, Kind::Plain),
        ge(3, Kind::Plain),
        ge(4, Kind::Plain),
        ge(5, Kind::Plain).sync().post(),
    ];
    let sched = [
        Sched::Register {
            at: 1,
            migrate: false,
        },
        Sched::Register {
            at: 5,
            migrate: false,
        },
    ];
    let (out, relay) = run_stream(&stream, &sched, false, 4, 8);
    assert!(!out.dead);
    assert_eq!(out.forwarded.len(), stream.len());
    assert_eq!(out.staged, vec![1], "window 1 staged its trigger");
    assert_eq!(out.unscanned_while_live, 0);
    let c = relay.life.counts();
    assert_eq!(c.to_window_fresh + c.to_window_exact, 2, "{c:?}");
    assert_eq!(
        c.to_window_fresh, 1,
        "window 2 starts after Pointer mode: {c:?}"
    );
    assert_eq!(
        out.barriers, 2,
        "window 1's gate of its SYNC_WAIT trigger + window 2's gate of its SYNC_WAIT entry"
    );
}

// ------------------------------------------------------------------------------------------------
// ★ 2026-10-17 lazy observe (production port): the OFF configuration IS the base relay

/// A long randomised sequence through `Relay::new` (lazy observe OFF: the constructor every other test uses) must make
/// EXACTLY the calls the BASE relay (`736c097d4`, before lazy observe existed) made: every `GP_PUT`, entry dword, epoch,
/// `GP_GET`, `REFERENCE`, publication, write-back and event, in order, with the same arguments and results, and end in the
/// same state. The golden fingerprint was computed by running THIS test body (unchanged except for the constant) on the
/// base commit. It covers all four lifecycle modes, registrations with and without ring growth, straddled packets,
/// SYNC_WAIT entries and partial feeds.
#[test]
fn lazy_off_is_bit_identical_to_the_base_relay_over_a_long_randomised_sequence() {
    use kf_chan::user_lazy::LazyConfig;
    const GOLDEN: u64 = 9_485_311_700_811_988_868;
    let (h, turns_total, forwarded_total) = fingerprint(None);
    assert!(
        turns_total > 20_000 && forwarded_total > 20_000,
        "the sequence is long: {turns_total} turns, {forwarded_total} entries"
    );
    assert_eq!(h, GOLDEN, "{turns_total} turns, {forwarded_total} entries");
    // every spelling of "both rules off" is the base: the constant, the switches at 0, interval 1 with the cache off
    for cfg in [
        LazyConfig::OFF,
        LazyConfig::from_switches(Some("0"), Some("0")),
        LazyConfig::ON.with_put_cache(false).with_interval(1),
        LazyConfig::ON.with_put_cache(false).with_interval(0),
    ] {
        assert_eq!(
            fingerprint(Some(cfg)).0,
            GOLDEN,
            "{cfg:?} is the base relay"
        );
    }
    // the check has teeth: with the rules ON the relay makes different calls (it reads less)
    assert_ne!(fingerprint(Some(LazyConfig::ON)).0, GOLDEN);
    assert_ne!(
        fingerprint(Some(LazyConfig::ON.with_interval(1))).0,
        GOLDEN,
        "A alone changes the calls too"
    );
}

/// The fingerprint of 600 randomised streams (`cfg` = `None`: `Relay::new` untouched), the turns and the entries relayed.
fn fingerprint(cfg: Option<kf_chan::user_lazy::LazyConfig>) -> (u64, usize, usize) {
    const SEEDS: u64 = 600;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut turns_total = 0usize;
    let mut forwarded_total = 0usize;
    for seed in 0..SEEDS {
        let mut rng = Rng::new(0xB05E_0000 + seed);
        let n = 20 + rng.below(480) as usize;
        let straddles = rng.chance(50);
        let mut stream = gen_stream(&mut rng, n, straddles, false);
        let sched = match rng.below(4) {
            0 => Vec::new(),
            1 => {
                let migrate = rng.chance(50);
                plan_registration(&mut rng, &mut stream, migrate).0
            }
            _ => {
                let count = 1 + rng.below(3) as usize;
                plan_registrations_spaced(&mut rng, &mut stream, count)
            }
        };
        let ring = 8u32 << rng.below(4);
        let priv_entries = 8u32 << rng.below(3);
        let mode = LifecycleMode::ALL[rng.below(LifecycleMode::ALL.len() as u64) as usize];
        let mut sim = Sim::new(ring, priv_entries, &stream, sched);
        if rng.chance(60) {
            sim.chunk = 1 + rng.below(40) as usize;
        }
        let mut relay = new_relay_mode(priv_entries, mode);
        if let Some(cfg) = cfg {
            relay = relay.with_lazy(cfg);
        }
        turns_total += drive(&mut sim, &mut relay, 200_000);
        forwarded_total += sim.out.forwarded.len();
        for v in [
            sim.trace,
            relay.cursor.into(),
            relay.relayed,
            relay.generation,
            relay.puts,
            u64::from(relay.dead),
        ] {
            h = (h ^ v).wrapping_mul(0x0000_0100_0000_01b3);
        }
        for b in format!("{:?}|{}", relay.life.counts(), relay.state_key()).bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    (h, turns_total, forwarded_total)
}

// ------------------------------------------------------------------------------------------------
// ★ 2026-10-17 lazy observe ON: the same relay, the same streams

fn lazy_cfgs() -> [kf_chan::user_lazy::LazyConfig; 4] {
    use kf_chan::user_lazy::LazyConfig;
    [
        LazyConfig::ON,
        LazyConfig::ON.with_interval(2),
        LazyConfig::ON.with_interval(3).with_put_cache(false),
        LazyConfig::ON.with_interval(1),
    ]
}

/// One stream through an OFF relay and a lazy relay: the world is the same, so what the guest's entries turn into on the
/// private ring is the same too.
struct Pair {
    off: (Out, Relay, u64, u64, u32),
    on: (Out, Relay, u64, u64, u32),
}

fn run_pair(
    stream: &[Ge],
    sched: &[Sched],
    mode: LifecycleMode,
    ring: u32,
    priv_entries: u32,
    chunk: usize,
    rate: Option<usize>,
    cfg: kf_chan::user_lazy::LazyConfig,
) -> Pair {
    let one = |cfg: Option<kf_chan::user_lazy::LazyConfig>| {
        let mut sim = Sim::new(ring, priv_entries, stream, sched.to_vec());
        sim.chunk = chunk;
        sim.rate = rate;
        let mut relay = new_relay_mode(priv_entries, mode);
        if let Some(cfg) = cfg {
            relay = relay.with_lazy(cfg);
        }
        drive(&mut sim, &mut relay, 400_000);
        sim.out.dead |= relay.dead;
        let (puts, hws, blocked) = (sim.reads_put, sim.reads_hw, sim.blocked_max);
        (sim.out, relay, puts, hws, blocked)
    };
    Pair {
        off: one(None),
        on: one(Some(cfg)),
    }
}

#[test]
fn lazy_forwards_exactly_what_the_base_forwards_for_every_mode_stream_and_registration() {
    let mut put_saved = 0u64;
    let mut hw_saved = 0u64;
    let mut checked = 0;
    for seed in 0..300u64 {
        let mut rng = Rng::new(0x1A2E_0000 + seed);
        let n = 20 + rng.below(300) as usize;
        let straddles = rng.chance(50);
        let mut stream = gen_stream(&mut rng, n, straddles, false);
        let sched = match rng.below(3) {
            0 => Vec::new(),
            1 => {
                let migrate = rng.chance(50);
                plan_registration(&mut rng, &mut stream, migrate).0
            }
            _ => {
                let count = 1 + rng.below(3) as usize;
                plan_registrations_spaced(&mut rng, &mut stream, count)
            }
        };
        let ring = 16u32 << rng.below(4);
        let priv_entries = 128u32 << rng.below(3);
        let mode = LifecycleMode::ALL[rng.below(LifecycleMode::ALL.len() as u64) as usize];
        let chunk = if rng.chance(50) {
            1 + rng.below(40) as usize
        } else {
            usize::MAX
        };
        let rate = if rng.chance(40) {
            Some(1 + rng.below(3) as usize)
        } else {
            None
        };
        for cfg in lazy_cfgs() {
            let p = run_pair(&stream, &sched, mode, ring, priv_entries, chunk, rate, cfg);
            let ctx = format!(
                "seed {seed} {cfg:?} mode {mode:?} ring {ring} priv {priv_entries} chunk {chunk} rate {rate:?}"
            );
            assert_eq!(p.off.0.dead, p.on.0.dead, "{ctx}: dead");
            assert_eq!(p.off.0.forwarded, p.on.0.forwarded, "{ctx}: forwarded");
            assert_eq!(p.off.0.staged, p.on.0.staged, "{ctx}: staged triggers");
            assert_eq!(p.off.0.switches, p.on.0.switches, "{ctx}: ring switches");
            assert_eq!(p.on.0.beyond_put, 0, "{ctx}: I8");
            assert_eq!(p.on.0.torn, 0, "{ctx}: I7");
            assert_eq!(p.on.0.unscanned_while_live, 0, "{ctx}: I4");
            assert_eq!(p.on.1.relayed, p.off.1.relayed, "{ctx}");
            assert_eq!(p.on.1.cursor, p.off.1.cursor, "{ctx}");
            assert_eq!(p.on.1.generation, p.off.1.generation, "{ctx}");
            assert_eq!(
                p.on.1.life.mode(),
                p.off.1.life.mode(),
                "{ctx}: the lifecycle ends in the same mode"
            );
            let (co, cn) = (p.off.1.life.counts(), p.on.1.life.counts());
            assert_eq!(
                (
                    co.entries_scan,
                    co.entries_pointer,
                    co.entries_window,
                    co.post_switch_scanned
                ),
                (
                    cn.entries_scan,
                    cn.entries_pointer,
                    cn.entries_window,
                    cn.post_switch_scanned
                ),
                "{ctx}: the same entries are relayed in the same modes (and 8 per switch)"
            );
            assert_eq!(
                co.pointer_pb_read_violation + cn.pointer_pb_read_violation,
                0,
                "{ctx}"
            );
            assert!(
                p.on.2 <= p.off.2 && p.on.3 <= p.off.3,
                "{ctx}: lazy never reads MORE"
            );
            // the cache is only ever filled in Pointer mode
            let c = p.on.1.lazy.counts;
            assert_eq!(
                c.turns,
                c.obs_skipped
                    + c.obs_unconditional
                    + c.obs_forced_wait
                    + c.obs_forced_low_credit
                    + c.obs_forced_open_segment
                    + c.obs_forced_ring_full
                    + c.obs_forced_interval,
                "{ctx}: every turn is accounted for"
            );
            if cfg == kf_chan::user_lazy::LazyConfig::ON {
                put_saved += p.off.2 - p.on.2;
                hw_saved += p.off.3 - p.on.3;
            }
            checked += 1;
        }
    }
    assert!(checked >= 1200);
    assert!(put_saved > 5_000, "rule A saves GP_PUT reads: {put_saved}");
    assert!(
        hw_saved > 5_000,
        "rule B saves private USERD reads: {hw_saved}"
    );
}

#[test]
fn lazy_every_consumed_entry_reaches_the_guests_gp_get_even_with_a_lagging_pbdma() {
    for seed in 0..120u64 {
        let mut rng = Rng::new(0x1A2E_1000 + seed);
        let n = 30 + rng.below(250) as usize;
        let stream = gen_stream(&mut rng, n, false, false);
        let ring = 16u32 << rng.below(4);
        let rate = Some(1 + rng.below(3) as usize);
        for cfg in lazy_cfgs() {
            let mut sim = Sim::new(ring, 256, &stream, vec![]);
            sim.rate = rate;
            let mut relay = new_relay_mode(256, LifecycleMode::Default).with_lazy(cfg);
            drive(&mut sim, &mut relay, 400_000);
            assert!(!relay.dead, "seed {seed} {cfg:?}");
            assert_eq!(
                sim.out.forwarded.len(),
                stream.len(),
                "seed {seed} {cfg:?}: all relayed"
            );
            assert_eq!(
                sim.guest_get[sim.src], relay.cursor,
                "seed {seed} {cfg:?} ring {ring}: the guest's GP_GET shows every consumed entry once the PBDMA has caught up"
            );
        }
    }
}

#[test]
fn lazy_d6_a_guest_with_a_full_ring_sees_a_get_no_staler_than_the_base_plus_the_interval() {
    let mut worst_gap = 0;
    for seed in 0..60u64 {
        let mut rng = Rng::new(0x1A2E_2000 + seed);
        let n = 100 + rng.below(300) as usize;
        let stream = gen_stream(&mut rng, n, false, false);
        for ring in [8u32, 16, 32, 64, 128] {
            for cfg in lazy_cfgs() {
                let p = run_pair(
                    &stream,
                    &[],
                    LifecycleMode::Default,
                    ring,
                    256,
                    usize::MAX,
                    None,
                    cfg,
                );
                assert!(!p.on.0.dead, "seed {seed}");
                let (off, on) = (p.off.4, p.on.4);
                let n = kf_chan::user_lazy::effective_interval(cfg.interval, ring) as u32;
                assert!(
                    on <= off + n + 2,
                    "seed {seed} ring {ring} {cfg:?}: the guest waited on a full ring while the relay was {on} entries ahead of its GP_GET (base: {off}, interval {n})"
                );
                worst_gap = worst_gap.max(on.saturating_sub(off));
            }
        }
    }
    println!(
        "lazy d6: the guest, blocked on a full ring, saw a GP_GET at most {worst_gap} entries staler than the base"
    );
}

#[test]
fn lazy_i8_entries_just_above_the_gp_put_are_never_read_with_the_cache_on() {
    for seed in 0..40u64 {
        let mut rng = Rng::new(seed + 7000);
        let n = 20 + rng.below(80) as usize;
        let stream = gen_stream(&mut rng, n, true, false);
        for chunk in [1usize, 2, 5, usize::MAX] {
            for cfg in lazy_cfgs() {
                let mut sim = Sim::new(32, 128, &stream, vec![]);
                sim.chunk = chunk;
                sim.poison_above_put = true;
                let mut relay = new_relay(128, false).with_lazy(cfg);
                drive(&mut sim, &mut relay, 100_000);
                assert!(!relay.dead, "seed {seed} chunk {chunk} {cfg:?}");
                assert_eq!(
                    sim.out.beyond_put, 0,
                    "seed {seed} chunk {chunk} {cfg:?}: I8"
                );
                let poison = ge(251, Kind::Plain).raw();
                assert!(
                    !sim.out.forwarded.contains(&poison),
                    "seed {seed} chunk {chunk} {cfg:?}: a poisoned slot was forwarded"
                );
                let expect: Vec<u64> = stream.iter().map(Ge::raw).collect();
                assert_eq!(
                    sim.out.forwarded, expect,
                    "seed {seed} chunk {chunk} {cfg:?}"
                );
            }
        }
    }
}

#[test]
fn lazy_a_hostile_gp_put_never_reads_a_slot_the_relay_has_no_pointer_to_and_never_panics() {
    // The guest rewrites GP_PUT to anything (backwards, outside the ring) while the cache is warm. The relay reads only
    // the slot at its cursor, only below SOME GP_PUT it loaded, and refuses a value outside the ring by name.
    let stream: Vec<Ge> = std::iter::once(ge(0, Kind::SetObj))
        .chain((1..40).map(|i| ge(i, Kind::Plain)))
        .collect();
    for hostile_put in [0u32, 1, 3, 31, 32, 33, 1000, u32::MAX] {
        let mut sim = Sim::new(32, 128, &stream, vec![]);
        sim.chunk = 12;
        let mut relay = new_relay(128, false).with_lazy(kf_chan::user_lazy::LazyConfig::ON);
        for _ in 0..6 {
            sim.tick();
            sim.feed();
            relay.claim();
            while !relay.idle() {
                relay.step(&mut sim);
            }
        }
        sim.put[sim.src] = hostile_put;
        for _ in 0..40 {
            relay.claim();
            let mut steps = 0;
            while !relay.idle() {
                relay.step(&mut sim);
                steps += 1;
                assert!(steps < 64);
            }
        }
        if hostile_put >= 32 {
            assert!(sim.out.dead || relay.dead, "{hostile_put}: refused by name");
        }
        assert!(sim.out.forwarded.len() <= stream.len() + 40, "bounded work");
    }
}

#[test]
fn lazy_a_registration_at_every_position_keeps_i4_with_the_cache_on() {
    // the Window anchor and the cache: whatever the guest published after the reply was scanned, wherever the flip falls
    let mut flips = 0;
    for seed in 0..400u64 {
        let mut rng = Rng::new(0x1A2E_3000 + seed);
        let n = 12 + rng.below(70) as usize;
        let mut stream = gen_stream(&mut rng, n, false, false);
        let migrate = rng.chance(50);
        let (sched, k, _t) = plan_registration(&mut rng, &mut stream, migrate);
        let ring = 16u32 << rng.below(3);
        let chunk = if rng.chance(50) {
            1 + rng.below(12) as usize
        } else {
            usize::MAX
        };
        for cfg in [
            kf_chan::user_lazy::LazyConfig::ON,
            kf_chan::user_lazy::LazyConfig::ON.with_interval(2),
        ] {
            let p = run_pair(
                &stream,
                &sched,
                LifecycleMode::Default,
                ring,
                128,
                chunk,
                None,
                cfg,
            );
            assert!(!p.on.0.dead, "seed {seed} k {k}");
            assert_eq!(
                p.on.0.unscanned_while_live, 0,
                "seed {seed}: I4 with the cache on"
            );
            assert_eq!(p.off.0.forwarded, p.on.0.forwarded, "seed {seed}");
            assert_eq!(
                p.off.0.staged, p.on.0.staged,
                "seed {seed}: the trigger is staged at the same position"
            );
            assert_eq!(p.off.0.switches, p.on.0.switches, "seed {seed}");
            flips += usize::from(p.on.1.life.counts().to_window_fresh > 0);
        }
    }
    assert!(flips > 300, "the flips happened: {flips}");
}

#[test]
fn lazy_the_post_switch_scan_is_eight_entries_per_ring_switch_with_lazy_on() {
    let mut switches = 0;
    for seed in 0..120u64 {
        let mut rng = Rng::new(0x1A2E_4000 + seed);
        let n = 60 + rng.below(80) as usize;
        let mut stream = gen_stream(&mut rng, n, false, false);
        let (sched, _, _) = plan_registration(&mut rng, &mut stream, true);
        for cfg in lazy_cfgs() {
            let p = run_pair(
                &stream,
                &sched,
                LifecycleMode::Default,
                64,
                128,
                usize::MAX,
                None,
                cfg,
            );
            let c = p.on.1.life.counts();
            assert_eq!(p.on.0.switches.len(), p.off.0.switches.len(), "seed {seed}");
            assert_eq!(
                c.post_switch_scanned,
                p.off.1.life.counts().post_switch_scanned,
                "seed {seed} {cfg:?}"
            );
            let after_switch =
                p.on.0
                    .switches
                    .first()
                    .map_or(0, |at| p.on.0.forwarded.len().saturating_sub(*at));
            if after_switch > kf_chan::user_lifecycle::POST_SWITCH_SCAN as usize + 2 {
                assert_eq!(
                    c.post_switch_scanned,
                    kf_chan::user_lifecycle::POST_SWITCH_SCAN as u64 * p.on.0.switches.len() as u64,
                    "seed {seed} {cfg:?}: 8 entries per switch"
                );
            }
            switches += p.on.0.switches.len();
            // the cache was dropped at the switch, the new ring is read from a fresh GP_PUT
            assert!(
                p.on.1.lazy.counts.inval_migration + p.on.1.lazy.counts.inval_mode_change > 0
                    || !cfg.put_cache,
                "seed {seed}"
            );
        }
    }
    assert!(switches > 100);
}

#[test]
fn lazy_d6_a_turn_that_skipped_its_look_and_saw_a_full_guest_ring_is_followed_by_a_look() {
    let mut checked = 0;
    for seed in 0..60u64 {
        let mut rng = Rng::new(0x1A2E_5000 + seed);
        let n = 100 + rng.below(300) as usize;
        let stream = gen_stream(&mut rng, n, false, false);
        for ring in [32u32, 64, 128] {
            let mut sim = Sim::new(ring, 256, &stream, vec![]);
            sim.log_turns = true;
            let mut relay = new_relay(256, false).with_lazy(kf_chan::user_lazy::LazyConfig::ON);
            drive(&mut sim, &mut relay, 400_000);
            assert!(!relay.dead);
            for w in sim.turn_log.windows(2) {
                if !w[0].0 && w[0].1 {
                    assert!(
                        w[1].0,
                        "seed {seed} ring {ring}: full ring seen without a look, and the next turn did not look either"
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 50, "the scenario happens: {checked}");
}

#[test]
fn lazy_a_refused_entry_ends_the_relay_and_a_retire_drops_what_was_cached() {
    // refusal = the guest's channel is reset: the cache goes with the relay
    let stream: Vec<Ge> = std::iter::once(ge(0, Kind::SetObj))
        .chain((1..40).map(|i| ge(i, Kind::Plain)))
        .collect();
    let mut sim = Sim::new(64, 256, &stream, vec![]);
    let mut relay = new_relay(256, false).with_lazy(kf_chan::user_lazy::LazyConfig::ON);
    drive(&mut sim, &mut relay, 10_000);
    assert!(!relay.dead);
    assert!(
        relay.lazy.cached().is_some(),
        "Pointer mode left a remembered GP_PUT"
    );
    // the next entry the guest publishes is invalid (a reserved bit): refused by name
    let slot = relay.cursor;
    sim.rings[sim.src].insert(slot, ge(2, Kind::Plain).raw() | (1 << 40));
    sim.put[sim.src] = (slot + 1) % 64;
    relay.claim();
    while !relay.idle() {
        relay.step(&mut sim);
    }
    assert!(relay.dead, "the relay stopped");
    assert_eq!(relay.lazy.cached(), None);
    assert_eq!(relay.lazy.counts.inval_reset, 1);

    // retire = the channel was freed
    let mut sim = Sim::new(64, 256, &stream, vec![]);
    let mut relay = new_relay(256, false).with_lazy(kf_chan::user_lazy::LazyConfig::ON);
    drive(&mut sim, &mut relay, 10_000);
    assert!(relay.lazy.cached().is_some());
    relay.retire();
    assert_eq!(relay.lazy.cached(), None);
    assert_eq!(relay.lazy.counts.inval_retire, 1);
    assert_eq!(relay.life.mode(), Mode::Dead);
}

#[test]
fn lazy_a_slow_pbdma_and_a_small_private_ring_run_on_the_credit_floor_without_stalling() {
    // 128 private entries: the credit count crosses the floor of 64 whenever the PBDMA lags; each such turn looks, so the
    // stale count never hides a credit that exists, and the run neither stalls nor loses an entry
    let mut low = 0;
    for seed in 0..40u64 {
        let mut rng = Rng::new(0x1A2E_6000 + seed);
        let n = 300 + rng.below(200) as usize;
        let stream = gen_stream(&mut rng, n, false, false);
        let mut sim = Sim::new(512, 128, &stream, vec![]);
        sim.rate = Some(1);
        sim.rate_div = 3;
        let mut relay = new_relay(128, false).with_lazy(kf_chan::user_lazy::LazyConfig::ON);
        drive(&mut sim, &mut relay, 400_000);
        assert!(!relay.dead, "seed {seed}");
        assert_eq!(
            sim.out.forwarded.len(),
            stream.len(),
            "seed {seed}: nothing stalled or lost"
        );
        low += relay.lazy.counts.obs_forced_low_credit;
    }
    assert!(low > 100, "the credit floor really forced looks: {low}");
}
