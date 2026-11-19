//! ★ **Model-based random test of the slot pool** (2026-11-18): the real `Capacity` against an obviously-correct oracle.
//!
//! STATUS: LIVE 2026-11-18. Why it exists: the slot-pool ceiling death (`replay_slot_ceiling.rs`, mechanism M2 of the
//! Wukong runs) was a STATE-MACHINE bug: every single step looked reasonable and the refusal came from the
//! composition of a policy (first-fit, preferred then required) with a layout (free runs, tails). A replay
//! pins one logged state; this test explores the space around it.
//!
//! ## What is compared
//!
//! A seeded generator (xorshift64*, no dependency) makes long random sequences of `create` (a first region via
//! `plan`), `release` and `grow` (`slot_needed`, which is the allocate / relocate / grow-in-place decision),
//! with sizes biased to the edges: exactly the free run after the slot, one grain past it, the largest free
//! block +-1, the pool and `max_cap` +-1, power-of-two edges, and hostile values (0, `u32::MAX`). Each step runs
//! through the REAL code and through [`Oracle`]: a bitmap of 256-run cells, first-fit by brute force, written
//! for this test. After EVERY step the test asserts:
//!
//! 1. no two regions overlap, all lie inside the pool and none exceeds `max_cap` (the hostile-guest bounds);
//! 2. NEVER-REFUSE-WHEN-FEASIBLE: when the oracle finds any placement that honours the policy (a new disjoint
//!    region of the required size, or growth into the free run after the slot) the real code did not refuse,
//!    and when no placement exists it did refuse, by name, and changed nothing;
//! 3. the decision is the policy's: preferred first-fit, else required first-fit, else in place (preferred,
//!    else required); the real `Move` equals the oracle's;
//! 4. accounting: every slot's region, the free total and the NUMBER of free runs (so coalescing) equal the
//!    oracle's, and so do the `grows` and `ceilings` counters;
//! 5. determinism: the same seed gives the same trace hash.
//!
//! The device is the recording kind of `replay_slot_ceiling.rs`: a `Move` is checked (a copy only for a
//! relocation, whole old length, destination disjoint from every other live region), never executed.
//!
//! ## Running
//!
//! Normal suite: a short seeded run. Heavy campaign (millions of steps), release build:
//!
//! ```text
//! KF_MODEL_SEEDS=2000 KF_MODEL_STEPS=5000 cargo test --release -p kf-cuda --test model_capacity -- --ignored --nocapture
//! ```
//!
//! CANNOT show: timing, GPU effects, the walk pool (kept out of the way: `walk_pool == slot_pool`), or the
//! device-side ordering of copies and the re-walk (read from `walk.rs`, see `replay_slot_ceiling.rs`).
//! A failure prints the seed and step: the minimal red test is that seed made into a fixed case.

use kf_cuda::capacity::{Capacity, Move, Region};
use std::collections::BTreeMap;

const CELL: u32 = 256;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[self.below(v.len() as u64) as usize]
    }
}

/// The reference: occupancy per cell, written to be obviously right, not fast.
#[derive(Clone)]
struct Oracle {
    owner: Vec<Option<u32>>,
    regions: BTreeMap<u32, Region>,
    max_cap: u32,
    slot_default: u32,
    grows: u64,
    ceilings: u64,
}

fn cells(runs: u32) -> usize {
    (runs / CELL) as usize
}

/// Round up to the grain, at least `min` (the policy's size rule, restated).
fn grain(n: u64, min: u32) -> u64 {
    let n = n.max(u64::from(min));
    n.div_ceil(u64::from(CELL)) * u64::from(CELL)
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    NoGrowth,
    Moved(Move),
    Refused(&'static str),
}

impl Oracle {
    fn new(pool: u32, max_cap: u32, slot_default: u32) -> Oracle {
        Oracle {
            owner: vec![None; cells(pool)],
            regions: BTreeMap::new(),
            max_cap,
            slot_default,
            grows: 0,
            ceilings: 0,
        }
    }

    fn free_cells(&self) -> usize {
        self.owner.iter().filter(|o| o.is_none()).count()
    }

    /// Maximal runs of free cells.
    fn free_runs(&self) -> usize {
        let mut n = 0;
        let mut prev_free = false;
        for o in &self.owner {
            let f = o.is_none();
            if f && !prev_free {
                n += 1;
            }
            prev_free = f;
        }
        n
    }

    /// Every offset (in runs) where `n` free cells start, lowest first: brute force.
    fn placements(&self, n_runs: u32) -> Vec<u32> {
        let n = cells(n_runs);
        let mut out = Vec::new();
        if n == 0 || n > self.owner.len() {
            return out;
        }
        for o in 0..=(self.owner.len() - n) {
            if self.owner[o..o + n].iter().all(Option::is_none) {
                out.push(o as u32 * CELL);
            }
        }
        out
    }

    /// Free runs directly after `r`.
    fn tail_after(&self, r: Region) -> u32 {
        let mut t = 0;
        let mut c = cells(r.off + r.cap);
        while c < self.owner.len() && self.owner[c].is_none() {
            t += CELL;
            c += 1;
        }
        t
    }

    fn set(&mut self, r: Region, v: Option<u32>) {
        for c in cells(r.off)..cells(r.off + r.cap) {
            self.owner[c] = v;
        }
    }

    fn create(&mut self, s: u32) -> Result<Region, ()> {
        let want = grain(0, self.slot_default) as u32;
        match self.placements(want).first() {
            Some(&off) => {
                let r = Region { off, cap: want };
                self.set(r, Some(s));
                self.regions.insert(s, r);
                Ok(r)
            }
            None => {
                self.ceilings += 1;
                Err(())
            }
        }
    }

    fn release(&mut self, s: u32) {
        if let Some(r) = self.regions.remove(&s) {
            self.set(r, None);
        }
    }

    /// Is there ANY placement of `req` runs for slot `s`'s growth (disjoint region or in place)? Independent of
    /// the policy order: this is the feasibility the real code must never refuse.
    fn feasible(&self, old: Region, req: u32) -> bool {
        !self.placements(req).is_empty() || self.tail_after(old) >= req - old.cap
    }

    fn grow(&mut self, s: u32, need: u32) -> Verdict {
        let old = self.regions[&s];
        if need <= old.cap {
            return Verdict::NoGrowth;
        }
        if need > self.max_cap {
            self.ceilings += 1;
            return Verdict::Refused("one space may hold");
        }
        let max = u64::from(self.max_cap);
        let pref = grain(u64::from(need) + u64::from(need) / 2, self.slot_default).min(max) as u32;
        let req = grain(u64::from(need), self.slot_default).min(max) as u32;
        let to = if let Some(&o) = self.placements(pref).first() {
            Some(Region { off: o, cap: pref })
        } else if let Some(&o) = self.placements(req).first() {
            Some(Region { off: o, cap: req })
        } else if self.tail_after(old) >= pref - old.cap {
            Some(Region {
                off: old.off,
                cap: pref,
            })
        } else if self.tail_after(old) >= req - old.cap {
            Some(Region {
                off: old.off,
                cap: req,
            })
        } else {
            None
        };
        // feasibility, stated independently of the order above
        assert_eq!(
            to.is_some(),
            self.feasible(old, req),
            "oracle policy and oracle feasibility disagree"
        );
        match to {
            None => {
                self.ceilings += 1;
                Verdict::Refused("slot pool exhausted")
            }
            Some(to) => {
                if to.off == old.off {
                    // in place: take only the extension
                    self.set(
                        Region {
                            off: old.off + old.cap,
                            cap: to.cap - old.cap,
                        },
                        Some(s),
                    );
                } else {
                    self.set(old, None);
                    self.set(to, Some(s));
                }
                self.regions.insert(s, to);
                self.grows += 1;
                Verdict::Moved(Move {
                    slot: s,
                    from: old,
                    to,
                })
            }
        }
    }
}

/// What the generator can ask for, kept for the failure message.
#[derive(Debug, Clone, Copy)]
enum Op {
    Create(u32),
    Release(u32),
    Grow(u32, u32),
}

struct Run {
    cap: Capacity,
    oracle: Oracle,
    pool: u32,
    max_cap: u32,
    max_slots: u32,
    hash: u64,
    steps: u64,
    grows: u64,
    refusals: u64,
    in_place: u64,
    relocations: u64,
}

fn fnv(h: &mut u64, v: u64) {
    *h ^= v;
    *h = h.wrapping_mul(0x0000_0100_0000_01B3);
}

impl Run {
    fn new(rng: &mut Rng) -> Run {
        let pool_cells = rng.pick(&[8u32, 12, 16, 24, 32, 48, 64, 128, 256, 1024, 4096]);
        let pool = pool_cells * CELL;
        let slot_default = CELL * rng.pick(&[1u32, 2, 4]).min(pool_cells);
        let max_cap = (CELL * (rng.below(u64::from(pool_cells)) as u32 + 1)).max(slot_default);
        let max_slots = 128;
        let cap = Capacity::new(max_slots, pool, pool, max_cap, slot_default).expect("config");
        Run {
            cap,
            oracle: Oracle::new(pool, max_cap, slot_default),
            pool,
            max_cap,
            max_slots,
            hash: 0xcbf2_9ce4_8422_2325,
            steps: 0,
            grows: 0,
            refusals: 0,
            in_place: 0,
            relocations: 0,
        }
    }

    /// An edge-biased `need` for growing `s`.
    fn need_for(&mut self, rng: &mut Rng, s: u32) -> u32 {
        let old = self.oracle.regions[&s];
        let tail = self.oracle.tail_after(old);
        let largest = {
            let mut best = 0u32;
            let mut cur = 0u32;
            for o in &self.oracle.owner {
                if o.is_none() {
                    cur += CELL;
                    best = best.max(cur);
                } else {
                    cur = 0;
                }
            }
            best
        };
        let pow2 = 1u32 << rng.below(24);
        let base = match rng.below(14) {
            0 => old.cap + tail,
            1 => old.cap + tail + CELL,
            2 => old.cap + tail.saturating_sub(CELL),
            3 => largest,
            4 => largest + CELL,
            5 => self.pool,
            6 => self.max_cap,
            7 => pow2,
            8 => old.cap,
            9 => old.cap + 1,
            10 => rng.below(u64::from(self.pool) * 5 / 4 + 2) as u32,
            11 => u32::MAX - rng.below(2) as u32,
            12 => 0,
            _ => old.cap + rng.below(u64::from(tail) + u64::from(CELL)) as u32,
        };
        // +-1 run and +-1 grain around the base
        match rng.below(5) {
            0 => base.saturating_sub(1),
            1 => base.saturating_add(1),
            2 => base.saturating_sub(CELL),
            3 => base.saturating_add(CELL),
            _ => base,
        }
    }

    fn pick_op(&mut self, rng: &mut Rng, w_create: u64, w_release: u64) -> Op {
        let held: Vec<u32> = self.oracle.regions.keys().copied().collect();
        let free_slots: Vec<u32> = (0..self.max_slots)
            .filter(|s| !self.oracle.regions.contains_key(s))
            .collect();
        let r = rng.below(100);
        if (held.is_empty() || r < w_create) && !free_slots.is_empty() {
            return Op::Create(rng.pick(&free_slots));
        }
        if held.is_empty() {
            return Op::Release(0);
        }
        let s = rng.pick(&held);
        if r < w_create + w_release {
            Op::Release(s)
        } else {
            Op::Grow(s, self.need_for(rng, s))
        }
    }

    fn step(&mut self, op: Op, seed: u64) {
        self.steps += 1;
        let ctx = format!(
            "seed {seed} step {} op {op:?} pool {} max_cap {}",
            self.steps, self.pool, self.max_cap
        );
        match op {
            Op::Create(s) => {
                let want = self.oracle.create(s);
                let got = self.cap.plan(&[s]);
                match (want, got) {
                    (Ok(r), Ok(lay)) => {
                        assert_eq!(self.cap.slot_region(s), Some(r), "{ctx}: first region");
                        assert_eq!(lay.slot_off[s as usize], r.off, "{ctx}");
                        assert_eq!(lay.slot_cap[s as usize], r.cap, "{ctx}");
                        fnv(&mut self.hash, u64::from(r.off) << 32 | u64::from(r.cap));
                    }
                    (Err(()), Err(e)) => {
                        assert!(e.contains("slot pool exhausted"), "{ctx}: {e}");
                        assert!(self.cap.slot_region(s).is_none(), "{ctx}");
                        self.refusals += 1;
                        fnv(&mut self.hash, 0xE0);
                    }
                    (w, g) => panic!("{ctx}: oracle {w:?} but real {:?}", g.map(|_| ())),
                }
            }
            Op::Release(s) => {
                self.oracle.release(s);
                self.cap.release(s);
                fnv(&mut self.hash, 0xF0 | u64::from(s));
            }
            Op::Grow(s, need) => {
                let v = self.oracle.grow(s, need);
                let got = self.cap.slot_needed(s, need);
                match (&v, &got) {
                    (Verdict::NoGrowth, Ok(None)) => fnv(&mut self.hash, 0xA0),
                    (Verdict::Moved(m), Ok(Some(g))) => {
                        assert_eq!(g, m, "{ctx}: the move is the policy's");
                        self.check_move(g, &ctx);
                        self.grows += 1;
                        if g.copy().is_none() {
                            self.in_place += 1;
                        } else {
                            self.relocations += 1;
                        }
                        fnv(
                            &mut self.hash,
                            u64::from(g.to.off) << 32 | u64::from(g.to.cap),
                        );
                    }
                    (Verdict::Refused(what), Err(e)) => {
                        assert!(e.contains(what), "{ctx}: `{e}` should name `{what}`");
                        self.refusals += 1;
                        fnv(&mut self.hash, 0xE1);
                    }
                    _ => panic!(
                        "{ctx}: oracle {v:?} but real {got:?} (never refuse when feasible, never grow when not)"
                    ),
                }
            }
        }
        self.check_state(&ctx);
    }

    /// The recording device: what the walker would be told, checked, never executed.
    fn check_move(&self, m: &Move, ctx: &str) {
        match m.copy() {
            None => assert_eq!(m.to.off, m.from.off, "{ctx}: in place keeps the offset"),
            Some((dst, src, runs)) => {
                assert_eq!(
                    (dst, src, runs),
                    (m.to.off, m.from.off, m.from.cap),
                    "{ctx}: whole old length"
                );
                assert!(
                    m.to.off + m.to.cap <= m.from.off || m.from.off + m.from.cap <= m.to.off,
                    "{ctx}: a copy whose source and destination overlap: {m:?}"
                );
            }
        }
        assert!(
            m.to.cap >= m.from.cap && m.to.cap <= self.max_cap,
            "{ctx}: {m:?}"
        );
        assert!(
            u64::from(m.to.off) + u64::from(m.to.cap) <= u64::from(self.pool),
            "{ctx}: past the pool"
        );
    }

    /// 1 and 4: bounds and accounting, after every step.
    fn check_state(&self, ctx: &str) {
        let mut real: Vec<(u32, Region)> = (0..self.max_slots)
            .filter_map(|s| self.cap.slot_region(s).map(|r| (s, r)))
            .collect();
        let want: Vec<(u32, Region)> = self.oracle.regions.iter().map(|(&s, &r)| (s, r)).collect();
        assert_eq!(real, want, "{ctx}: the region table equals the oracle's");
        real.sort_by_key(|&(_, r)| r.off);
        for w in real.windows(2) {
            assert!(
                w[0].1.off + w[0].1.cap <= w[1].1.off,
                "{ctx}: overlap {w:?}"
            );
        }
        for (_, r) in &real {
            assert!(
                r.cap > 0 && r.cap <= self.max_cap,
                "{ctx}: {r:?} vs max_cap"
            );
            assert!(
                u64::from(r.off) + u64::from(r.cap) <= u64::from(self.pool),
                "{ctx}: {r:?} past the pool"
            );
        }
        let free = self.cap.slot_pool_free();
        assert_eq!(
            free,
            (self.oracle.free_cells() as u64) * u64::from(CELL),
            "{ctx}: free runs"
        );
        let used: u64 = real.iter().map(|(_, r)| u64::from(r.cap)).sum();
        assert_eq!(used + free, u64::from(self.pool), "{ctx}: partition");
        // the number of free runs, read from the geometry line: coalesced, never adjacent
        let g = self.cap.slot_geometry(0, 1);
        let n: usize = g
            .split(" free ")
            .nth(1)
            .and_then(|t| t.split(" in ").nth(1))
            .and_then(|t| t.split(' ').next())
            .and_then(|t| t.parse().ok())
            .unwrap_or_else(|| panic!("{ctx}: no free-run count in `{g}`"));
        assert_eq!(
            n,
            self.oracle.free_runs(),
            "{ctx}: number of free runs in `{g}`"
        );
        assert_eq!(self.cap.stats.grows, self.oracle.grows, "{ctx}: grows");
        assert_eq!(
            self.cap.stats.ceilings, self.oracle.ceilings,
            "{ctx}: ceilings"
        );
    }
}

struct Outcome {
    hash: u64,
    steps: u64,
    grows: u64,
    in_place: u64,
    relocations: u64,
    refusals: u64,
}

/// One seed: `steps` operations on a fresh random configuration.
fn run_seed(seed: u64, steps: u64) -> Outcome {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    for _ in 0..4 {
        rng.next();
    }
    let mut r = Run::new(&mut rng);
    // per-seed workload: how often slots are made and dropped (many small holes, or a near-full pool)
    let (w_create, w_release) = rng.pick(&[(30, 20), (45, 15), (20, 30), (55, 5), (10, 10)]);
    for _ in 0..steps {
        let op = r.pick_op(&mut rng, w_create, w_release);
        r.step(op, seed);
    }
    Outcome {
        hash: r.hash,
        steps: r.steps,
        grows: r.grows,
        in_place: r.in_place,
        relocations: r.relocations,
        refusals: r.refusals,
    }
}

fn campaign(first: u64, seeds: u64, steps: u64) -> Outcome {
    let mut total = Outcome {
        hash: 0,
        steps: 0,
        grows: 0,
        in_place: 0,
        relocations: 0,
        refusals: 0,
    };
    for seed in first..first + seeds {
        let o = run_seed(seed, steps);
        total.hash ^= o.hash.rotate_left((seed % 63) as u32);
        total.steps += o.steps;
        total.grows += o.grows;
        total.in_place += o.in_place;
        total.relocations += o.relocations;
        total.refusals += o.refusals;
    }
    total
}

/// The short seeded run of the normal suite. It must also REACH the interesting cases, or it proves nothing:
/// growth in place, relocation and refusals all occur.
#[test]
fn the_real_pool_equals_the_oracle_on_seeded_random_sequences() {
    let o = campaign(1, 40, 1_200);
    assert!(o.in_place > 150, "in-place growths reached: {}", o.in_place);
    assert!(
        o.relocations > 150,
        "relocations reached: {}",
        o.relocations
    );
    assert!(o.refusals > 500, "refusals reached: {}", o.refusals);
    assert!(o.grows > 600, "{}", o.grows);
}

/// 5: the same seeds give the same trace.
#[test]
fn the_model_run_is_deterministic() {
    let a = campaign(7, 10, 600);
    let b = campaign(7, 10, 600);
    assert_eq!(a.hash, b.hash);
    assert_eq!(
        (a.grows, a.refusals, a.in_place),
        (b.grows, b.refusals, b.in_place)
    );
    let c = campaign(8, 10, 600);
    assert_ne!(
        a.hash, c.hash,
        "different seeds differ (the hash sees the trace)"
    );
}

/// Heavy campaign: `KF_MODEL_SEEDS` seeds (default 2000) of `KF_MODEL_STEPS` steps (default 5000) from
/// `KF_MODEL_FIRST` (default 1000). Release build advised (see the module doc).
#[test]
#[ignore = "heavy campaign: millions of steps, see the module doc for the command"]
fn heavy_campaign_pool_equals_oracle() {
    let env = |k: &str, d: u64| {
        std::env::var(k)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(d)
    };
    let (first, seeds, steps) = (
        env("KF_MODEL_FIRST", 1000),
        env("KF_MODEL_SEEDS", 2000),
        env("KF_MODEL_STEPS", 5000),
    );
    let o = campaign(first, seeds, steps);
    eprintln!(
        "model campaign: {} steps over {seeds} seeds from {first}: {} grows ({} in place, {} relocations), {} refusals, trace {:016x}",
        o.steps, o.grows, o.in_place, o.relocations, o.refusals, o.hash
    );
}
