//! Gate/apply adversary added to the existing lifecycle World and production relay.
//! Apply completion announces work here; actual wake registration is covered separately
//! by relay_fairness and handoff_protocol. External vector delivery uses ens4_replay.py.
use super::*;
use kf_chan::user_turn::Phase;

fn held_turn_reads_put(credit_wait: bool) -> bool {
    let cfg = Cfg::new(spec(ring_full_script(), &[0, 1, 2, 3, 4]));
    let mut w = World::new(&cfg);
    w.env.walk_busy = true;
    w.env.put[0] = 1;
    let raw = ge(0, Kind::SetObj).raw();
    w.env.rings[0][0] = (raw as u32, (raw >> 32) as u32);
    w.env.fill = 1;
    if credit_wait {
        w.relay.need = kf_chan::user_loop::Need::Popped { target: 1 };
        w.relay.lazy.wait_set();
    }
    w.relay.claim();
    for _ in 0..16 {
        w.relay_step();
        if w.relay.idle() {
            break;
        }
    }
    !w.relay.idle() || !w.env.log.is_empty() || w.relay.phase == Phase::Lo
}

#[test]
fn apply_hold_before_put_does_not_read_or_publish_a_new_entry() {
    assert!(!held_turn_reads_put(false));
}

#[test]
fn delayed_apply_during_pointer_credit_wait_does_not_read_new_put() {
    assert!(!held_turn_reads_put(true));
}

#[derive(Clone)]
struct HeldWorld {
    world: World,
    apply: u8,
}
impl HeldWorld {
    fn key(&self) -> (u64, u8) {
        (self.world.key(), self.apply)
    }
    fn successors(&self, cfg: &Cfg) -> Vec<Self> {
        let mut out: Vec<_> = self
            .world
            .successors(cfg)
            .into_iter()
            .filter(|w| w.key() != self.world.key())
            .map(|world| Self {
                world,
                apply: self.apply,
            })
            .collect();
        if self.apply < 2 {
            let mut next = self.clone();
            next.apply += 1;
            next.world.env.walk_busy = next.apply == 1;
            if next.apply == 2 {
                // Announced apply completion: explicit environment assumption.
                next.world.flag = true;
            }
            out.push(next);
        }
        out
    }
}

fn exhaustive(cfg: &Cfg) -> usize {
    let start = HeldWorld {
        world: World::new(cfg),
        apply: 0,
    };
    let mut seen = HashSet::from([start.key()]);
    let mut queue = VecDeque::from([start]);
    let mut terminals = 0;
    while let Some(node) = queue.pop_front() {
        assert!(seen.len() <= 1_000_000, "ENS4 exploration budget exceeded");
        assert_eq!(
            node.world.env.viol,
            0,
            "{:?}",
            viol_names(node.world.env.viol)
        );
        let succ = node.successors(cfg);
        if succ.is_empty() {
            terminals += 1;
            assert_eq!(node.apply, 2);
            assert_eq!(node.world.env.executed, cfg.spec.expected);
        }
        for next in succ {
            if seen.insert(next.key()) {
                queue.push_back(next);
            }
        }
    }
    assert!(terminals > 0);
    println!(
        "ENS4 exhaustive states={} terminals={terminals}",
        seen.len()
    );
    seen.len()
}

#[test]
fn adversary_apply_at_every_relay_microstep_preserves_scan_pointer_delivery() {
    let mut cfg = Cfg::new(spec(
        vec![
            Op::Push(ge(0, Kind::SetObj)),
            Op::Push(p(1)),
            Op::SetPut,
            Op::Ring,
        ],
        &[0, 1],
    ));
    cfg.looks = true;
    assert!(exhaustive(&cfg) > 100);
}

fn seeded(cfg: &Cfg, seed: u64) -> (Vec<(u64, u8)>, bool, bool, bool) {
    let mut node = HeldWorld {
        world: World::new(cfg),
        apply: 0,
    };
    let mut rng = seed;
    let mut trace = Vec::new();
    let mut credit = false;
    let mut migrated = false;
    let mut window = false;
    for _ in 0..8192 {
        let succ = node.successors(cfg);
        if succ.is_empty() {
            assert_eq!(node.world.env.executed, cfg.spec.expected, "seed={seed}");
            return (trace, credit, migrated, window);
        }
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        node = succ[(rng % succ.len() as u64) as usize].clone();
        assert_eq!(node.world.env.viol, 0, "seed={seed}");
        credit |= matches!(
            node.world.relay.need,
            kf_chan::user_loop::Need::Popped { .. }
        );
        migrated |= node.world.env.switched > 0;
        window |= node.world.relay.life.mode() == Mode::Window;
        trace.push(node.key());
    }
    panic!("seed={seed} schedule cap; no successful result claimed");
}

#[test]
fn seeded_migration_replays_the_production_scan_pointer_window_and_invalidation() {
    let mut cfg = Cfg::new(spec(cursor_script(), &[0, 1, 2, 3]));
    cfg.looks = true;
    cfg.lazy = kf_chan::user_lazy::LazyConfig::ON;
    let mut coverage = (false, false);
    for seed in 1..=256 {
        let a = seeded(&cfg, seed);
        let b = seeded(&cfg, seed);
        assert_eq!(a, b, "seed={seed}");
        coverage.0 |= a.2;
        coverage.1 |= a.3;
    }
    assert_eq!(coverage, (true, true));
}

#[test]
fn full_private_ring_credit_wait_recovers_with_apply_churn_and_looks() {
    let mut cfg = Cfg::new(spec(ring_full_script(), &[0, 1, 2, 3, 4]));
    cfg.priv_entries = 4;
    cfg.looks = true;
    cfg.lazy = kf_chan::user_lazy::LazyConfig::ON;
    let mut credit = false;
    for seed in 1..=256 {
        credit |= seeded(&cfg, seed).1;
    }
    assert!(credit, "must actually exercise the exhausted-credit branch");
}

#[test]
fn host_marker_can_complete_while_apply_still_holds_the_walk_gate() {
    let mut cfg = Cfg::new(spec(vec![Op::Push(p(0)), Op::SetPut, Op::Ring], &[0]));
    cfg.looks = true;
    let mut w = World::new(&cfg);
    w.env.log = vec![Pub::Marker(1)];
    w.env.popped = 1;
    w.env.walk_busy = true;
    let successors = w.successors(&cfg);
    assert!(
        successors
            .iter()
            .any(|n| n.env.markers_done == 1 && n.env.walk_busy)
    );
}
#[test]
fn migration_invalidates_cached_put_even_with_exhausted_private_credit() {
    let cfg = Cfg::new(spec(vec![], &[]));
    let mut w = World::new(&cfg);
    w.relay.lazy = kf_chan::user_lazy::Lazy::new(kf_chan::user_lazy::LazyConfig::ON);
    w.relay.lazy.put_fresh(Mode::Pointer, 1, 3, 8);
    assert!(w.relay.lazy.cached().is_some());
    w.relay.need = kf_chan::user_loop::Need::Popped { target: 1 };
    w.env.walk_busy = true;
    w.relay.switch_source(2);
    assert_eq!(w.relay.lazy.cached(), None);
    assert_eq!(w.relay.lazy.counts.inval_migration, 1);
}
#[test]
fn exhaustive_apply_adversary_includes_window_and_ring_migration() {
    let mut cfg = Cfg::new(spec(cursor_script(), &[0, 1, 2, 3]));
    cfg.looks = true;
    cfg.lazy = kf_chan::user_lazy::LazyConfig::ON;
    assert!(exhaustive(&cfg) > 100);
}
#[test]
fn exhaustive_apply_adversary_includes_full_private_ring_credit() {
    let mut cfg = Cfg::new(spec(ring_full_script(), &[0, 1, 2, 3, 4]));
    cfg.priv_entries = 4;
    cfg.looks = true;
    cfg.lazy = kf_chan::user_lazy::LazyConfig::ON;
    assert!(exhaustive(&cfg) > 100);
}
#[test]
fn migration_to_lower_put_drops_old_cached_value_during_apply() {
    let cfg = Cfg::new(spec(vec![], &[]));
    let mut w = World::new(&cfg);
    w.relay.lazy = kf_chan::user_lazy::Lazy::new(kf_chan::user_lazy::LazyConfig::ON);
    w.relay.lazy.put_fresh(Mode::Pointer, 1, 7, 8);
    w.env.walk_busy = true;
    w.relay.switch_source(2);
    assert_eq!(w.relay.lazy.cached(), None);
    assert_eq!(w.relay.lazy.counts.inval_migration, 1);
}

/// ENS10 H2 falsifier: already-published fetch receipts reach guest GET before PUT is gated.
#[test]
fn ens10_published_fetch_writeback_does_not_wait_for_any_walk_to_finish() {
    for lazy in [
        kf_chan::user_lazy::LazyConfig::OFF,
        kf_chan::user_lazy::LazyConfig::ON,
    ] {
        let mut cfg = Cfg::new(spec(vec![], &[]));
        cfg.lazy = lazy;
        let mut w = World::new(&cfg);
        let generation = w.relay.generation;
        w.relay.fetch.publish(Some((generation, 1))).unwrap();
        w.env.popped = 1;
        w.env.hw_get = 1;
        w.env.walk_busy = true;
        w.relay.claim();
        for _ in 0..16 {
            w.relay_step();
            if w.relay.idle() {
                break;
            }
        }
        assert_eq!(w.env.guest_get[0], 1);
        assert!(w.env.walk_busy);
        assert!(w.env.log.is_empty(), "no new publication while held");
        assert_eq!(w.env.viol, 0);
    }
}

/// A real processed marker is harvestable even while future reads of the same space are held.
#[test]
fn ens10_processed_marker_is_observed_before_the_put_gate_with_lazy_on_and_off() {
    for lazy in [
        kf_chan::user_lazy::LazyConfig::OFF,
        kf_chan::user_lazy::LazyConfig::ON,
    ] {
        let mut cfg = Cfg::new(spec(vec![], &[]));
        cfg.lazy = lazy;
        let mut w = World::new(&cfg);
        let (count, _) = w.relay.marks.issue().unwrap();
        w.env.markers_done = count;
        w.env.walk_busy = true;
        w.relay.claim();
        for _ in 0..16 {
            w.relay_step();
            if w.relay.idle() {
                break;
            }
        }
        assert_eq!(w.relay.marks.seen(), count);
        assert!(w.env.walk_busy);
        assert!(w.env.log.is_empty());
        assert_eq!(w.env.viol, 0);
    }
}
