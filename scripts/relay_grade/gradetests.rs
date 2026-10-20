/// The commit of `tests/relay_stress.rs` this copy was generated from.
const HARNESS_FROM: &str = "ab6193aff";

impl Scenario {
    /// GRADER G3: 8..32 channels (hot / cold / ring-filling / mixed) in 1..4 address spaces, 1..3 workers, every other knob as
    /// `from_seed` draws it (GPU stalls / bursts / interrupt jitter, lock holder, busy sites, SYNC_WAIT density, chaos, doorbell
    /// styles: duplicate / late / skipped).
    fn grade(seed: u64, secs: f64) -> Self {
        let mut s = Self::from_seed(seed, secs);
        let mut r = Rng::new(seed, 77);
        let n = r.range(8, 32) as usize;
        let mut fillers = 0;
        s.roles = (0..n)
            .map(|_| match r.below(8) {
                0..=2 => Role::Hot,
                3..=4 => Role::Cold,
                5 if fillers < 2 => {
                    fillers += 1;
                    Role::Filler
                }
                _ => Role::Mixed,
            })
            .collect();
        s.n_as = r.range(1, 4) as usize;
        s.workers = r.range(1, 3) as usize;
        s
    }
}

fn grade_run(sc: &Scenario) -> Outcome {
    let o = run_scenario(sc);
    if failed_run(&o) {
        report(&o);
    }
    o
}

fn grade_seeds(seeds: impl Iterator<Item = u64>, secs: f64) -> Vec<String> {
    let mut failed = Vec::new();
    for seed in seeds {
        let sc = Scenario::grade(seed, secs);
        let o = grade_run(&sc);
        if failed_run(&o) || o.entries == 0 {
            failed.push(format!(
                "seed {seed} (clients={} as={} workers={}): {:?} {:?}",
                sc.roles.len(),
                sc.n_as,
                sc.workers,
                o.violations.first(),
                o.soft.first()
            ));
        }
    }
    failed
}

/// G3 (default suite): 24 seeds x 0.3 s, strict, infinite park. `RS_GRADE_SEEDS` / `RS_GRADE_SECS` override.
#[test]
fn g3_stress_default_seed_list() {
    let n = env_u("RS_GRADE_SEEDS", 24);
    let failed = grade_seeds(1000..1000 + n, env_f("RS_GRADE_SECS", 0.3));
    assert!(failed.is_empty(), "{HARNESS_FROM}: {} failed:\n{}", failed.len(), failed.join("\n"));
}

/// G3 (deep run): the fixed list of 300 short seeds and 5 long ones (60 s each). About 8 minutes.
#[test]
#[ignore = "deep: 300 x 0.5 s + 5 x 60 s"]
fn g3_stress_full_seed_list() {
    let mut failed = grade_seeds(0..300, 0.5);
    failed.extend(grade_seeds(5000..5005, env_f("RS_LONG_SECS", 60.0)));
    assert!(failed.is_empty(), "{} failed:\n{}", failed.len(), failed.join("\n"));
}

/// G3 (nightly): `RS_FROM`..`RS_TO` (default 0..5000), `RS_SECS` per seed.
#[test]
#[ignore = "nightly: RS_FROM..RS_TO seeds"]
fn g3_stress_nightly_range() {
    let failed = grade_seeds(env_u("RS_FROM", 0)..env_u("RS_TO", 5000), env_f("RS_SECS", 0.3));
    assert!(failed.is_empty(), "{} failed:\n{}", failed.len(), failed.join("\n"));
}

/// G2: the kernel's view of sleep safety under lock holders, SYNC_WAIT-heavy guests and 1..3 workers: never every worker asleep
/// in `ep_poll` with an empty ready set while a relay holds an obligation without a wake source.
#[test]
fn g2_sleep_safety_kernel_view() {
    let mut failed = Vec::new();
    for seed in 0..12u64 {
        let mut sc = Scenario::grade(2000 + seed, 0.4);
        sc.workers = 1 + (seed as usize % 3);
        sc.sync_p = [32, 256][seed as usize % 2];
        if seed % 2 == 0 {
            sc.holder = HolderCfg {
                on: true,
                hold_us_max: 2000,
                rings_after_p: [0, 1024][seed as usize / 2 % 2],
                gap_us_max: 1000,
            };
        }
        let o = grade_run(&sc);
        if let Some(v) = o.violations.iter().chain(o.soft.iter()).next() {
            failed.push(format!("seed {}: {v}", 2000 + seed));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// G4: every scenario of a fixed list runs with the blocking loop and with the same loop forced never to block (an always-ready
/// fd): both strict-green; a red only with blocking is the efficiency layer creating a bug, a red only when spinning hides one.
#[test]
fn g4_spin_reference_stress_equivalence() {
    let mut bad = Vec::new();
    for seed in 3000..3008u64 {
        let sc = Scenario::grade(seed, 0.3);
        let b = grade_run(&sc);
        let s = grade_run(&Scenario {
            force_spin: true,
            ..sc.clone()
        });
        match (failed_run(&b), failed_run(&s)) {
            (false, false) => {}
            (true, false) => bad.push(format!("seed {seed}: RED ONLY WHEN BLOCKING: {:?}", b.violations.first())),
            (false, true) => bad.push(format!("seed {seed}: red only when spinning (the park hides it): {:?}", s.violations.first())),
            (true, true) => bad.push(format!("seed {seed}: red both ways: {:?}", b.violations.first())),
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// G5: after the guests stop, the workers stay blocked for ONE SECOND (no park, no timeout-0 look, no serve), and the wakeups per
/// doorbell are bounded (parks <= 4 x doorbells + 16).
#[test]
fn g5_efficiency_stress_idle_blocks_for_one_second() {
    for (seed, workers) in [(1u64, 1usize), (2, 2), (3, 3)] {
        let mut sc = Scenario::from_seed(seed, 0.5);
        sc.roles = vec![Role::Cold; 8];
        sc.workers = workers;
        sc.holder.on = false;
        sc.synth_hot = 0;
        sc.quiet_ms = 1000;
        let o = grade_run(&sc);
        assert!(!failed_run(&o), "W={workers}: {:?} {:?}", o.violations, o.soft);
        assert!(o.quiet_checked, "W={workers}: the run never settled, the quiet window was not checked (vacuous)");
        assert!(
            o.parks <= 4 * o.rings + 16,
            "W={workers}: {} parks for {} doorbells",
            o.parks,
            o.rings
        );
        println!("W={workers}: parks={} doorbells={} entries={}", o.parks, o.rings, o.entries);
    }
}

/// G6: the baton -- 2 workers, 4 self-re-ringing hot relays (100 us turns), one cold sync client: strict (a response later than
/// `RS_BOUND_MS` while the workers served more than `RS_STARVE_SERVES` tokens is a failure); wall-clock p99 / max reported.
#[test]
fn g6_fairness_stress_baton() {
    for seed in [1u64, 2, 3] {
        let o = grade_run(&Scenario::baton(seed, 1.5));
        println!(
            "baton seed {seed}: resp p99<={}us max={:.2}ms max_serves_during_a_resp={}",
            o.resp_p99_us, o.resp_max_ms, o.max_serves_during_resp
        );
        assert!(!failed_run(&o), "seed {seed}: {:?} {:?}", o.violations, o.soft);
        assert!(o.entries > 20, "seed {seed}: the cold client barely ran");
    }
}

/// G6: hot relays with huge guest-ring deltas (ring fillers, 32767 entries per burst) do not delay a cold neighbour.
#[test]
fn g6_fairness_stress_huge_delta_beside_cold() {
    for seed in [11u64, 12] {
        let mut sc = Scenario::baton(seed, 1.5);
        sc.roles = vec![Role::Filler, Role::Filler, Role::Cold];
        sc.synth_hot = 0;
        sc.turn_us = 0;
        let o = grade_run(&sc);
        println!("huge-delta seed {seed}: resp p99<={}us max={:.2}ms", o.resp_p99_us, o.resp_max_ms);
        assert!(!failed_run(&o), "seed {seed}: {:?} {:?}", o.violations, o.soft);
    }
}

/// G7: guest rings (32768) and the private ring (512) driven to full and drained under GPU stalls, SYNC_WAIT gates and credit
/// waits: no deadlock, every entry exactly once in order, the guest's GP_GET reaches GP_PUT.
#[test]
fn g7_ring_fill_and_backpressure() {
    for (seed, workers) in [(21u64, 1usize), (22, 2), (23, 3)] {
        let mut sc = Scenario::from_seed(seed, 0.8);
        sc.roles = vec![Role::Filler, Role::Filler, Role::Cold, Role::Mixed];
        sc.workers = workers;
        sc.sync_p = 32;
        sc.gpu = GpuCfg {
            burst: 64,
            stall_p: 40,
            stall_us_max: 3000,
            cost_us: 0,
            barrier_us_max: 200,
            irq_jitter_us: 300,
        };
        let o = grade_run(&sc);
        assert!(!failed_run(&o), "W={workers}: {:?} {:?}", o.violations, o.soft);
        assert!(o.entries > 10_000, "W={workers}: the rings were not filled ({} entries)", o.entries);
    }
}

