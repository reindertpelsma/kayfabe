// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! Native Inbox scope + the historical lifecycle harness driving the REAL per-entry Relay.
//! Clock-free: delays below are scheduler steps, conditional on the explicit fair suffix.
#[path = "../../../kf-chan/tests/lifecycle_support/mod.rs"]
mod lifecycle;
use super::{Inbox, SplitTarget, VasKey};
use kf_chan::user_lifecycle::{Lifecycle, Mode};
use kf_chan::user_scan::Scan;
use lifecycle::{Env, Ev, Ge, Glue, Kind, Pub, Relay, ge};
use std::collections::{HashSet, VecDeque};
use std::hash::{Hash, Hasher};
const A: VasKey = VasKey(0x100);
const B: VasKey = VasKey(0x200);

/// Reconstruct the native queue->running->ACK state, independently of its predicate.
fn native(own: u8, other: u8) -> Inbox {
    let inbox = Inbox::new().unwrap();
    for (phase, key, token) in [(own, B, 7), (other, A, 9)] {
        if phase == 1 || phase == 2 {
            let ticket = inbox.request_user_split(token, key).ok().unwrap();
            if phase == 2 {
                let taken = inbox.take_split_requests();
                assert!(
                    taken
                        .iter()
                        .any(|(t, k)| *t == ticket && *k == SplitTarget::Space(key))
                );
            }
        }
    }
    let keys = [(own, B), (other, A)]
        .into_iter()
        .filter(|(phase, _)| *phase == 2)
        .map(|(_, key)| key)
        .collect();
    inbox.set_walk_busy(own == 2 || other == 2);
    inbox.set_walk_scope(keys, false, None);
    inbox
}

#[derive(Clone, Debug)]
struct Io {
    own: u8,
    other: u8,
    source: usize,
    entries: [Vec<Ge>; 2],
    put: [u32; 2],
    guest_get: [u32; 2],
    log: Vec<Pub>,
    popped: usize,
    processed: usize,
    marker: u64,
    registered: u8,
    epoch: u64,
    live: u32,
    migrated: bool,
    reads: u32,
    violations: u32,
}
impl Io {
    fn ready(&self) -> bool {
        !native(self.own, self.other).source_gate(B, 7).held()
    }
    fn id(&self, raw: u64) -> Option<Ge> {
        self.entries
            .iter()
            .flatten()
            .find(|g| g.raw() == raw)
            .copied()
    }
}
impl Env for Io {
    fn walk_busy(&self) -> bool {
        !self.ready()
    }
    fn publication_ready(&self) -> bool {
        self.ready()
    }
    fn src_entries(&self) -> u32 {
        8
    }
    fn gp_put(&mut self) -> u32 {
        self.put[self.source]
    }
    fn gp_dword(&mut self, slot: u32, hi: bool) -> u32 {
        let raw = self.entries[self.source]
            .get(slot as usize)
            .map_or(0, |g| g.raw());
        if hi { (raw >> 32) as u32 } else { raw as u32 }
    }
    fn registry_epoch(&mut self) -> u64 {
        self.epoch
    }
    fn registry_live(&mut self) -> u32 {
        self.live
    }
    fn hw_get(&mut self) -> u32 {
        (self.popped as u32) & 3
    }
    fn hw_reference(&mut self) -> u32 {
        if self.marker == 0 {
            0
        } else {
            kf_chan::user_gate::Marks::value(self.marker)
        }
    }
    fn read_pb(&mut self, raw: u64) -> Vec<u32> {
        self.reads += 1;
        self.id(raw).unwrap().words()
    }
    fn publish(&mut self, p: Pub) {
        assert!(
            self.own != 1 && self.own != 2,
            "published through own pending mutation"
        );
        if let Pub::Guest { tested, stored, .. } = p {
            assert_eq!(tested, stored);
        }
        self.log.push(p);
    }
    fn write_guest_get(&mut self, get: u32) {
        self.guest_get[self.source] = get;
    }
    fn trigger_migrates(&mut self, _: u64) -> bool {
        true
    }
    fn executed(&mut self, migrate: bool) {
        self.live = 0;
        self.epoch += 1;
        if migrate {
            self.source = 1;
            self.migrated = true;
        }
    }
    fn event(&mut self, e: Ev) {
        if matches!(e, Ev::BeyondPut) {
            self.violations += 1;
        }
    }
}

#[derive(Clone, Debug)]
struct Node {
    relay: Relay,
    io: Io,
    flag: bool,
    modes: u8,
    credit: bool,
}
impl Node {
    fn new(lazy: bool) -> Self {
        let mut relay = Relay::new(
            4,
            Lifecycle::default(),
            Scan::for_subdevice(1).unwrap(),
            Glue::default(),
        );
        if lazy {
            relay = relay.with_lazy(kf_chan::user_lazy::LazyConfig::ON);
        }
        // An already-published three-entry prefix fills the four-slot private ring.
        // Its pops have no marker edge: the worker must exercise CreditWait/Popped.
        for _ in 0..3 {
            relay.fetch.publish(None).unwrap();
        }
        Self {
            relay,
            io: Io {
                own: 0,
                other: 0,
                source: 0,
                entries: [
                    vec![
                        ge(0, Kind::SetObj),
                        ge(1, Kind::Plain),
                        ge(2, Kind::Trig { migrate: true }),
                    ],
                    vec![ge(3, Kind::Plain)],
                ],
                put: [2, 1],
                guest_get: [0, 0],
                log: vec![Pub::Barrier; 3],
                popped: 0,
                processed: 0,
                marker: 0,
                registered: 0,
                epoch: 0,
                live: 0,
                migrated: false,
                reads: 0,
                violations: 0,
            },
            flag: true,
            modes: 0,
            credit: false,
        }
    }
    fn key(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.relay.key().hash(&mut h);
        let e = &self.io;
        (
            e.own,
            e.other,
            e.source,
            e.put,
            e.guest_get,
            &e.log,
            e.popped,
            e.processed,
            e.marker,
            e.registered,
            e.epoch,
            e.live,
        )
            .hash(&mut h);
        (e.migrated, self.flag, self.modes, self.credit).hash(&mut h);
        h.finish()
    }
    fn check(&mut self) {
        assert_eq!(self.io.violations, 0, "I8");
        assert!(!self.relay.dead, "unexpected refusal");
        self.modes |= match self.relay.life.mode() {
            Mode::Scan => 1,
            Mode::Pointer => 2,
            Mode::Window => 4,
            Mode::Dead => panic!("dead lifecycle"),
        };
        self.credit |= matches!(self.relay.need, kf_chan::user_loop::Need::Popped { .. });
    }
    fn worker(&mut self) -> bool {
        if self.relay.idle() {
            let mut satisfied = self.relay.wait_satisfied(&mut self.io);
            // Popped waits are unannounced: the production loop explicitly keeps looking.
            satisfied |= matches!(self.relay.need, kf_chan::user_loop::Need::Popped { .. })
                && self.io.popped as u64 > self.relay.fetch.fetched();
            if !self.flag && !self.relay.again && !satisfied {
                return false;
            }
            self.flag = false;
            if satisfied {
                self.relay.clear_waiting();
            }
            self.relay.claim();
        } else {
            let before = self.relay.phase;
            let reads = self.io.reads;
            self.relay.step(&mut self.io);
            if before == kf_chan::user_turn::Phase::PbRead
                && self.relay.life.mode() == Mode::Pointer
            {
                assert_eq!(self.io.reads, reads, "I1");
            }
        }
        self.check();
        true
    }
    fn gpu(&mut self) -> bool {
        if self.io.popped < self.io.log.len() {
            self.io.popped += 1;
            return true;
        }
        if self.io.processed < self.io.popped {
            if let Pub::Marker(n) = self.io.log[self.io.processed] {
                self.io.marker = n;
                self.flag = true; // the real marker event is a registered wake source.
            }
            self.io.processed += 1;
            return true;
        }
        false
    }
    fn exact_output(&self) {
        let ids: Vec<u8> = self
            .io
            .log
            .iter()
            .filter_map(|p| match p {
                Pub::Guest { stored, .. } => self.io.id(*stored).map(|g| g.id),
                Pub::Staged { tested } => self.io.id(*tested).map(|g| g.id),
                _ => None,
            })
            .collect();
        assert_eq!(ids, [0, 1, 2, 3], "I2 exact once and in order");
    }

    fn complete(&self) -> bool {
        self.io.migrated && self.io.guest_get[1] == 1 && self.io.processed == self.io.log.len()
    }
    fn successors(&self) -> Vec<(u8, Self)> {
        let mut out = vec![];
        for actor in 0..5 {
            let mut n = self.clone();
            let progress = match actor {
                0 => n.worker(),
                1 => n.gpu(),
                2 if n.io.own < 3 => {
                    n.io.own += 1;
                    n.flag = true;
                    true
                }
                3 if n.io.other < 3 => {
                    n.io.other += 1;
                    n.flag = true;
                    true
                }
                4 if n.io.registered == 0 && n.relay.cursor >= 2 => {
                    n.io.registered = 1;
                    n.io.live = 1;
                    n.io.epoch += 1;
                    n.flag = true;
                    true
                }
                4 if n.io.registered == 1 => {
                    n.io.registered = 2;
                    n.io.put[0] = 3;
                    n.flag = true;
                    true
                }
                _ => false,
            };
            if progress {
                n.check();
                if n.key() != self.key() {
                    out.push((actor, n));
                }
            }
        }
        out
    }
}

fn fair_finish(mut n: Node) -> Node {
    n.io.own = 3;
    n.io.other = 3;
    n.flag = true;
    for _ in 0..1024 {
        if n.complete() {
            n.exact_output();
            return n;
        }
        n.worker();
        n.gpu();
        if n.io.registered == 0 && n.relay.cursor >= 2 {
            n.io.registered = 1;
            n.io.live = 1;
            n.io.epoch += 1;
            n.flag = true;
        } else if n.io.registered == 1 {
            n.io.registered = 2;
            n.io.put[0] = 3;
            n.flag = true;
        }
    }
    panic!("fair suffix did not terminate: {n:?}");
}

#[test]
fn exhaustive_native_scopes_interleave_every_relay_step_and_preserve_safety() {
    let start = Node::new(false);
    let mut seen = HashSet::from([start.key()]);
    let mut queue = VecDeque::from([start]);
    let mut terminals = 0;
    while let Some(n) = queue.pop_front() {
        assert!(
            seen.len() <= 4_000_000,
            "frontier cap: inconclusive, not a pass"
        );
        if n.complete() {
            n.exact_output();
            terminals += 1;
            continue;
        }
        // Check a bounded, fair drain from EVERY reachable adversarial prefix.
        fair_finish(n.clone());
        let successors = n.successors();
        assert!(!successors.is_empty(), "stranded state: {n:?}");
        for (_, next) in successors {
            if seen.insert(next.key()) {
                queue.push_back(next);
            }
        }
    }
    assert!(terminals > 0);
    println!(
        "ENS10 exhaustive states={} terminals={terminals}",
        seen.len()
    );
}

#[test]
fn seeded_native_scope_schedules_replay_migration_window_and_credit_wait() {
    let mut coverage = (0, false);
    for seed in 1..=256 {
        let run = || {
            let mut n = Node::new(true);
            let mut rng = seed as u64;
            let mut trace = vec![];
            for _ in 0..256 {
                let choices = n.successors();
                if choices.is_empty() || n.complete() {
                    break;
                }
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                let (actor, next) = choices[(rng % choices.len() as u64) as usize].clone();
                trace.push((actor, next.key()));
                n = next;
            }
            let n = fair_finish(n);
            (trace, n.modes, n.credit)
        };
        let a = run();
        assert_eq!(a, run(), "seed={seed}");
        coverage.0 |= a.1;
        coverage.1 |= a.2;
    }
    assert_eq!(coverage, (7, true));
}

fn foreign_hold(lazy: bool) {
    let mut n = Node::new(lazy);
    n.io.own = 3;
    n.io.other = 2;
    for _ in 0..1024 {
        if n.complete() {
            assert_eq!(n.io.other, 2, "foreign walk must remain unfinished");
            assert!(n.io.migrated);
            n.exact_output();
            return;
        }
        n.worker();
        n.gpu();
        if n.io.registered == 0 && n.relay.cursor >= 2 {
            n.io.registered = 1;
            n.io.live = 1;
            n.io.epoch += 1;
            n.flag = true;
        } else if n.io.registered == 1 {
            n.io.registered = 2;
            n.io.put[0] = 3;
            n.flag = true;
        }
    }
    panic!("B was coupled to indefinitely long A: {n:?}");
}
#[test]
fn disjoint_space_reads_publish_migrate_and_complete_before_the_foreign_walk_ends() {
    foreign_hold(false);
}
#[test]
fn disjoint_space_cached_put_and_credit_progress_do_not_wait_for_a_foreign_walk() {
    foreign_hold(true);
}

fn harvested_during_hold(lazy: bool) {
    let mut n = Node::new(lazy);
    n.io.own = 2;
    n.io.other = 2;
    n.relay.fetch = kf_chan::user_scan::Fetch::new(4).unwrap();
    let generation = n.relay.generation;
    n.relay.fetch.publish(Some((generation, 1))).unwrap();
    n.relay.fetch.publish(None).unwrap();
    let (count, _) = n.relay.marks.issue().unwrap();
    n.io.log = vec![
        Pub::Guest {
            tested: ge(0, Kind::SetObj).raw(),
            stored: ge(0, Kind::SetObj).raw(),
            scanned: true,
        },
        Pub::Marker(count),
    ];
    n.io.popped = 2;
    n.io.processed = 2;
    n.io.marker = count;
    let mut latency = None;
    for tick in 0..16 {
        n.worker();
        if n.relay.marks.seen() == count && n.io.guest_get[0] == 1 {
            latency = Some(tick);
            break;
        }
    }
    assert!(
        latency.is_some_and(|n| n <= 3),
        "bounded fair worker observation"
    );
    assert_eq!((n.io.own, n.io.other), (2, 2));
    assert_eq!(
        n.io.log.len(),
        2,
        "no new source publication through own hold"
    );
}
#[test]
fn host_finished_marker_and_get_are_observed_within_three_worker_steps_during_both_holds() {
    harvested_during_hold(false);
}
#[test]
fn lazy_worker_observes_finished_work_within_three_steps_while_future_reads_stay_held() {
    harvested_during_hold(true);
}
