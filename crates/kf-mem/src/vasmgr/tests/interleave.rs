//! ★★★ **The refresh protocol under every interleaving** (owner requirement, 2026-10-20).
//!
//! The VA manager thread is serial. A guest `MMU_INVALIDATE` trigger (BAR0 `0xB830B0`, bit 31)
//! is armed in the vCPU's trap and cleared by the manager only after a walk. The requirement:
//! refreshes coalesce like doorbells — at most one follow-up walk waits behind the one in
//! flight; the LAST arm is completed by a walk that STARTED after it; no arm is ever missed; no
//! backlog can build; nothing needs a timer.
//!
//! This is a small explicit model checker (stateless BFS with a state-digest memo and replay)
//! over the REAL pieces: the real [`InvalidatePort`] (the same type the trap writes), the real
//! [`VaManager`] (`on_invalidate`, `on_walk_ready`, `pump`), and the test rig's model walker
//! (a snapshot at submit, a result only when the harness lets the "GPU" finish). The only things
//! modelled are the schedule and the oracle.
//!
//! Actors (one logical guest, one manager thread, one GPU):
//! - guest: `Prep(kind)` (PTE write(s) + PDB latch writes), `Arm` (the trigger write, i.e.
//!   `Trigger::arm_next`), `Sig` (the eventfd write that follows the arm). Unless `polite`, the
//!   guest may start its next arm at once (the guest's invalidate timed out, or it is hostile);
//!   `polite` is RM's discipline: the next arm only after the bit reads idle.
//! - manager iteration, split into its three windows so a guest step may land between any two:
//!   `Drain` (eventfd drain), `Observe` (read the armed request, `on_invalidate`), `Ready`
//!   (`on_walk_ready`). An iteration starts only on an event: the eventfd or the walker's fd.
//!   ⊘ NO timer exists in the model, so a state with outstanding work and nothing enabled is a
//!   proof that progress would need a timer (I6).
//! - GPU: `GpuFinish` releases the walk in flight.
//!
//! Oracle (independent of the code under test): arm `k` is RETIRED when every space it names has
//! been covered by a walk that was SUBMITTED after `k` armed and has been COLLECTED.
//! - I1 no stale clear: bit clear => every arm so far is retired (and the host mapping holds every
//!   PTE the guest wrote before its last arm). A specific arm is retired only by a walk of ITS
//!   space (I4: a specific is never dropped because another space's walk completed).
//! - I2 no lost re-arm: an arm always leaves the bit busy; at quiescence the bit is idle.
//! - I3 pending work is bounded by the number of spaces, never by the number of arms.
//! - I5 walks submitted <= arms.
//! - I6 quiescence (no event enabled): nothing pending or in flight, every arm retired.

use super::*;
use std::collections::{BTreeMap, HashMap, VecDeque};

/// I3's bound for the two spaces of the rig: one request that carries the trigger's sequence plus
/// at most one scope-only refresh of the other space.
const BOUND_PENDING: usize = 2;

/// A guard against a configuration whose state space would not fit in memory.
const MAX_STATES: usize = 60_000_000;

/// A bound on a schedule's length that only has to catch a loop (`show` prints the longest real one).
const MAX_DEPTH: usize = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    A,
    B,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ev {
    Prep(Kind),
    Arm,
    Sig,
    Drain,
    Observe,
    Ready,
    GpuFinish,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pc {
    Idle,
    Drained,
    Observed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gp {
    Idle,
    Prepped(Kind),
    Armed,
}

struct ArmRec {
    kind: Kind,
    /// Walks submitted before this arm: walk `j` is "after" the arm iff `j >= before`.
    before: usize,
    seq: u64,
}

#[derive(Clone, Copy)]
struct Cfg {
    /// Arms the guest makes.
    n: usize,
    /// RM's discipline: the next arm only after the bit reads idle.
    polite: bool,
    /// Prune on state digests (off: every distinct schedule is run).
    memo: bool,
    /// The kinds of arm the guest may make.
    kinds: &'static [Kind],
    /// Breadth-first, keeping the state graph: minimal counterexamples and the exact number of
    /// complete schedules. Off: depth-first over a visited set only (a far smaller footprint).
    count: bool,
}

const ALL_KINDS: &[Kind] = &[Kind::A, Kind::B, Kind::All];

type Rows = BTreeMap<u64, Vec<(u64, u64, u64, u32)>>;

struct World {
    r: Rig,
    last_seq: Option<u64>,
    wake: bool,
    fd_ready: bool,
    pc: Pc,
    gp: Gp,
    started: usize,
    arms: Vec<ArmRec>,
    /// The guest's tables at its last arm.
    snap: Rows,
    walks_done: usize,
    /// An arm left the bit idle (I2: a re-arm must always read busy).
    lost_rearm: bool,
}

/// ★ The manager's observation of the port — the production loop's step (`device.rs`, `va_loop`).
fn observe(
    m: &mut VaManager<ModelWalker, FakeHost>,
    port: &InvalidatePort,
    after: &mut Option<u64>,
) {
    m.observe_port(port, after);
}

impl World {
    fn new() -> World {
        let mut r = rig();
        r.m.walker_mut().hold_walks = true;
        World {
            r,
            last_seq: None,
            wake: false,
            fd_ready: false,
            pc: Pc::Idle,
            gp: Gp::Idle,
            started: 0,
            arms: Vec::new(),
            snap: Rows::new(),
            walks_done: 0,
            lost_rearm: false,
        }
    }

    fn busy(&self) -> bool {
        busy(&self.r.port)
    }

    fn enabled(&self, cfg: Cfg) -> Vec<Ev> {
        let mut v = Vec::new();
        match self.gp {
            Gp::Idle if self.started < cfg.n && (!cfg.polite || !self.busy()) => {
                v.extend(cfg.kinds.iter().copied().map(Ev::Prep));
            }
            Gp::Idle => {}
            Gp::Prepped(_) => v.push(Ev::Arm),
            Gp::Armed => v.push(Ev::Sig),
        }
        match self.pc {
            Pc::Idle if self.wake || self.fd_ready => v.push(Ev::Drain),
            Pc::Idle => {}
            Pc::Drained => v.push(Ev::Observe),
            Pc::Observed => v.push(Ev::Ready),
        }
        let w = self.r.m.walker();
        if w.queued.is_some() && w.hold {
            v.push(Ev::GpuFinish);
        }
        v
    }

    fn apply(&mut self, ev: Ev) {
        match ev {
            Ev::Prep(k) => {
                let i = self.started as u64;
                let spaces: &[u64] = match k {
                    Kind::A => &[PDB_A],
                    Kind::B => &[PDB_B],
                    Kind::All => &[PDB_A, PDB_B],
                };
                for &pdb in spaces {
                    let (va0, at0) = if pdb == PDB_A {
                        (0x1000_0000, 0x0200_0000)
                    } else {
                        (0x2000_0000, 0x0210_0000)
                    };
                    self.r.tables.borrow_mut().entry(pdb).or_default().push((
                        va0 + i * 0x10000,
                        at0 + i * 0x10000,
                        0x1000,
                        0,
                    ));
                }
                // RM writes the PDB registers for a named space and none for ALL_PDB
                // (`kern_gmmu_tu102.c:112`): the latches then keep their stale words.
                if k != Kind::All {
                    let regs = self.r.port.regs();
                    let pdb = if k == Kind::A { PDB_A } else { PDB_B };
                    let (lo, hi) = Invalidate::encode_pdb(pdb, PdbAperture::Vidmem);
                    self.r.port.write(regs.pdb, lo);
                    self.r.port.write(regs.upper_pdb, hi);
                }
                self.gp = Gp::Prepped(k);
            }
            Ev::Arm => {
                let Gp::Prepped(kind) = self.gp else {
                    unreachable!("Arm follows Prep")
                };
                let word =
                    kf_trap::mmuinval::TRIGGER_BIT | 1 | if kind == Kind::All { 0b10 } else { 0 };
                let PortWrite::Publish(req) = self.r.port.write(self.r.port.regs().trigger, word)
                else {
                    panic!("the trigger write did not publish")
                };
                self.arms.push(ArmRec {
                    kind,
                    before: self.r.m.walker().submits.len(),
                    seq: req.seq,
                });
                self.snap = self.r.tables.borrow().clone();
                self.started += 1;
                self.gp = Gp::Armed;
                self.lost_rearm |= !self.busy();
            }
            Ev::Sig => {
                self.wake = true;
                self.gp = Gp::Idle;
            }
            Ev::Drain => {
                self.wake = false;
                self.pc = Pc::Drained;
            }
            Ev::Observe => {
                observe(&mut self.r.m, &self.r.port, &mut self.last_seq);
                self.pc = Pc::Observed;
            }
            Ev::Ready => {
                let out = self.r.m.on_walk_ready(self.r.port.trigger());
                if out.collected {
                    self.walks_done += 1;
                    self.fd_ready = false;
                }
                self.pc = Pc::Idle;
            }
            Ev::GpuFinish => {
                self.r.m.walker_mut().hold = false;
                self.fd_ready = true;
            }
        }
    }

    /// The oracle: has every space `a` names been covered by a collected walk submitted after it?
    fn retired(&self, a: &ArmRec) -> bool {
        let need: &[u64] = match a.kind {
            Kind::A => &[PDB_A],
            Kind::B => &[PDB_B],
            Kind::All => &[PDB_A, PDB_B],
        };
        let subs = &self.r.m.walker().submits;
        need.iter()
            .all(|p| (a.before..self.walks_done.min(subs.len())).any(|j| subs[j].contains(p)))
    }

    /// What the host holds, from the recorded ops (VA ranges of the two spaces are disjoint).
    fn host(&self) -> BTreeMap<u64, u64> {
        let mut h = BTreeMap::new();
        for (op, _) in self.r.ops.borrow().iter() {
            match op {
                Op::Map(va, off, _) => {
                    h.insert(*va, *off);
                }
                Op::Unmap(va) => {
                    h.remove(va);
                }
                _ => {}
            }
        }
        h
    }

    fn check(&self) -> Vec<(&'static str, String)> {
        let mut v = Vec::new();
        let busy = self.busy();
        let late = self.arms.iter().position(|a| !self.retired(a));
        if self.lost_rearm {
            v.push((
                "I2",
                "LOST RE-ARM: a trigger write left the bit reading idle".to_string(),
            ));
        }
        if !busy && let Some(k) = late {
            v.push((
                "I1",
                format!(
                    "STALE CLEAR: the bit reads idle but arm #{k} ({:?}, seq {}) has no collected walk of its space(s) submitted after it",
                    self.arms[k].kind, self.arms[k].seq
                ),
            ));
        }
        if !busy {
            let h = self.host();
            for rows in self.snap.values() {
                for (va, at, _, _) in rows {
                    if h.get(va) != Some(at) {
                        v.push((
                            "I1",
                            format!("idle bit, but the host does not hold the PTE {va:#x} -> {at:#x} the guest wrote before its last arm"),
                        ));
                    }
                }
            }
        }
        if self.r.m.pending() > BOUND_PENDING {
            v.push((
                "I3",
                format!(
                    "pending work is {} after {} arms (bound {BOUND_PENDING}): it grows with the arms",
                    self.r.m.pending(),
                    self.arms.len()
                ),
            ));
        }
        let subs = self.r.m.walker().submits.len();
        if subs > self.arms.len() {
            v.push((
                "I5",
                format!("{subs} walks were submitted for {} arms", self.arms.len()),
            ));
        }
        v
    }

    fn check_terminal(&self) -> Vec<(&'static str, String)> {
        let mut v = Vec::new();
        let late = self.arms.iter().position(|a| !self.retired(a));
        if self.busy() {
            v.push((
                "I2",
                "STUCK BUSY: the bit reads busy at quiescence (nothing left that could clear it)"
                    .to_string(),
            ));
        }
        let t = self.r.port.trigger();
        if t.issued() != t.completed() {
            v.push((
                "I6",
                format!(
                    "{} arms issued but {} completed: an arm was never accounted for",
                    t.issued(),
                    t.completed()
                ),
            ));
        }
        if late.is_some() || self.r.m.pending() > 0 || self.r.m.in_flight() {
            v.push((
                "I6",
                format!(
                    "STUCK without a timer: nothing is enabled but busy={} unretired-arm={late:?} pending={} in_flight={}",
                    self.busy(),
                    self.r.m.pending(),
                    self.r.m.in_flight()
                ),
            ));
        }
        v
    }

    fn digest(&self) -> u128 {
        let text = self.digest_text();
        use std::hash::{Hash, Hasher};
        let mut a = std::collections::hash_map::DefaultHasher::new();
        let mut b = std::collections::hash_map::DefaultHasher::new();
        0xA5A5u16.hash(&mut b);
        text.hash(&mut a);
        text.hash(&mut b);
        (u128::from(a.finish()) << 64) | u128::from(b.finish())
    }

    fn digest_text(&self) -> String {
        let m = &self.r.m;
        let w = m.walker();
        let wants = |ws: &mut dyn Iterator<Item = &Want>| -> String {
            ws.map(|x| match x {
                Want::Invalidate(r, _) => {
                    format!("I{}:{}:{:x}", r.seq, r.inval.all_pdb, r.inval.pdb)
                }
                other => format!("{other:?}"),
            })
            .collect::<Vec<_>>()
            .join(",")
        };
        let slots: BTreeMap<u32, String> = w
            .slots
            .iter()
            .map(|(k, v)| (*k, format!("{v:?}")))
            .collect();
        format!(
            "{:?}|{:?}|{:?}|{:?}|{}|{}|{:?}|{}|{}|{}|[{}]|[{}]|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{}|{:?}|{:?}",
            self.gp,
            self.pc,
            self.started,
            self.last_seq,
            self.wake,
            self.fd_ready,
            self.r.port,
            self.walks_done,
            w.hold,
            w.queued.is_some(),
            wants(&mut m.pending.iter()),
            wants(&mut m.inflight.iter().flat_map(|b| b.wants.iter().map(|x| &x.0))),
            w.submits,
            slots,
            w.verdict,
            w.last,
            w.resets,
            self.r.tables.borrow(),
            m.awaiting(),
            self.host(),
            self.arms
                .iter()
                .map(|a| (a.kind, a.before, a.seq))
                .collect::<Vec<_>>(),
        ) + &format!("{:?}", self.snap)
    }
}

fn replay(path: &[Ev]) -> World {
    let mut w = World::new();
    for &e in path {
        w.apply(e);
    }
    w
}

#[derive(Default)]
struct Report {
    states: usize,
    transitions: usize,
    terminals: usize,
    /// Distinct complete schedules (event sequences from the start to a terminal state).
    schedules: f64,
    max_pending: usize,
    /// The longest schedule explored.
    max_depth: usize,
    /// The shortest schedule that breaks each invariant.
    first: BTreeMap<&'static str, (Vec<Ev>, String)>,
}

impl Report {
    fn note(&mut self, path: &[Ev], ev: Ev, v: Vec<(&'static str, String)>) {
        for (code, why) in v {
            self.first.entry(code).or_insert_with(|| {
                let mut p = path.to_vec();
                p.push(ev);
                (p, why)
            });
        }
    }

    /// Fail with the minimal counterexample of `code`, if any.
    fn assert_holds(&self, codes: &[&str]) {
        for c in codes {
            if let Some((p, why)) = self.first.get(c) {
                panic!(
                    "{c} VIOLATED after {} events: {why}\nschedule: {p:?}",
                    p.len()
                );
            }
        }
    }
}

fn explore(cfg: Cfg) -> Report {
    let mut rep = Report::default();
    let mut ids: HashMap<u128, usize> = HashMap::new();
    let mut next_id = 1usize;
    let mut edges: Vec<Vec<usize>> = Vec::new();
    let mut queue: VecDeque<(Vec<Ev>, usize)> = VecDeque::new();
    ids.insert(World::new().digest(), 0);
    edges.push(Vec::new());
    queue.push_back((Vec::new(), 0));
    loop {
        let next = if cfg.count {
            queue.pop_front()
        } else {
            queue.pop_back()
        };
        let Some((path, id)) = next else { break };
        assert!(
            next_id <= MAX_STATES,
            "the state space exceeds {MAX_STATES} states: shrink the configuration"
        );
        if path.len() > MAX_DEPTH {
            // Every event of a schedule is caused by an arm, a walk or a wake; a schedule longer
            // than the arms can account for is work that keeps making work.
            rep.note(
                &path,
                Ev::Ready,
                vec![(
                    "I6",
                    format!(
                        "LIVELOCK: a schedule of more than {MAX_DEPTH} events for {} arms",
                        cfg.n
                    ),
                )],
            );
            continue;
        }
        let w = replay(&path);
        rep.max_depth = rep.max_depth.max(path.len());
        rep.max_pending = rep.max_pending.max(w.r.m.pending());
        let en = w.enabled(cfg);
        if en.is_empty() {
            rep.terminals += 1;
            let v = w.check_terminal();
            rep.note(&path, Ev::Ready, v);
            continue;
        }
        drop(w);
        for ev in en {
            let mut w2 = replay(&path);
            w2.apply(ev);
            rep.transitions += 1;
            let v = w2.check();
            rep.note(&path, ev, v);
            let d = w2.digest();
            let nid = match ids.get(&d) {
                Some(&n) if cfg.memo => n,
                _ => {
                    let n = next_id;
                    next_id += 1;
                    ids.insert(d, n);
                    if cfg.count {
                        edges.push(Vec::new());
                    }
                    let mut p = path.clone();
                    p.push(ev);
                    queue.push_back((p, n));
                    n
                }
            };
            if cfg.count {
                edges[id].push(nid);
            }
        }
    }
    rep.states = next_id;
    if !cfg.count {
        return rep;
    }
    // Complete schedules = paths in the (acyclic) state graph from the root to a sink.
    let mut memo: Vec<Option<f64>> = vec![None; edges.len()];
    fn paths(s: usize, edges: &[Vec<usize>], memo: &mut Vec<Option<f64>>) -> f64 {
        if let Some(c) = memo[s] {
            return c;
        }
        let c = if edges[s].is_empty() {
            1.0
        } else {
            edges[s].iter().map(|&n| paths(n, edges, memo)).sum()
        };
        memo[s] = Some(c);
        c
    }
    rep.schedules = paths(0, &edges, &mut memo);
    rep
}

fn show(name: &str, cfg: Cfg, rep: &Report) {
    eprintln!(
        "interleave[{name}] n={} polite={}: {} states, {} transitions, {} terminal states, {:.0} complete schedules, longest {} events, max pending {}, violations: {:?}",
        cfg.n,
        cfg.polite,
        rep.states,
        rep.transitions,
        rep.terminals,
        rep.schedules,
        rep.max_depth,
        rep.max_pending,
        rep.first.keys().collect::<Vec<_>>()
    );
}

fn cfg(n: usize, polite: bool, kinds: &'static [Kind], count: bool) -> Cfg {
    Cfg {
        n,
        polite,
        memo: true,
        kinds,
        count,
    }
}

/// RM's discipline (the next arm only after the bit reads idle): every invariant, exhaustively.
#[test]
fn polite_guest_n4_every_invariant_over_every_interleaving() {
    let c = cfg(4, true, ALL_KINDS, true);
    let rep = explore(c);
    show("polite", c, &rep);
    rep.assert_holds(&["I1", "I2", "I3", "I5", "I6"]);
}

/// ★ The adversarial guest (a timed-out invalidate, a hostile guest): arms back to back, any pdb,
/// any mix of specific and ALL_PDB, during every window of the manager. I1/I2/I4: no stale clear,
/// no lost re-arm, no arm missed (a specific is never dropped because another space was walked).
#[test]
fn adversarial_guest_n3_no_stale_clear_and_no_lost_arm() {
    let c = cfg(3, false, ALL_KINDS, true);
    let rep = explore(c);
    show("adversarial", c, &rep);
    rep.assert_holds(&["I1", "I2"]);
}

/// I5 and I6 over the adversarial schedules.
#[test]
fn adversarial_guest_n3_walks_never_exceed_arms_and_progress_needs_no_timer() {
    let c = cfg(3, false, ALL_KINDS, true);
    let rep = explore(c);
    rep.assert_holds(&["I3", "I5", "I6"]);
}

/// ★ I3 with four arms: pending work is bounded by the spaces, not by the arms. (Two specific
/// spaces, every window: about 3 million states, two minutes; the three-kind four-arm space is the
/// next test.) Run with `cargo test --release -p kf-mem interleave -- --ignored --nocapture`.
#[test]
#[ignore = "two minutes; the exhaustive four-arm, two-space schedules"]
fn adversarial_guest_n4_two_spaces_every_invariant() {
    let c = cfg(4, false, &[Kind::A, Kind::B], false);
    let rep = explore(c);
    show("adversarial-AB", c, &rep);
    rep.assert_holds(&["I1", "I2", "I3", "I5", "I6"]);
}

/// The full four-arm space (specific A, specific B, ALL_PDB). Minutes and gigabytes: run with
/// `cargo test --release -p kf-mem interleave -- --ignored --nocapture`.
#[test]
#[ignore = "minutes; the exhaustive four-arm, three-kind space"]
fn adversarial_guest_n4_three_kinds_every_invariant() {
    let c = cfg(4, false, ALL_KINDS, false);
    let rep = explore(c);
    show("adversarial-ABG", c, &rep);
    rep.assert_holds(&["I1", "I2", "I3", "I5", "I6"]);
}

/// The digest memo hides nothing: with and without it the same schedules, the same violations.
#[test]
fn the_state_digest_memo_hides_no_schedule() {
    let ca = cfg(2, false, ALL_KINDS, true);
    let a = explore(ca);
    let mut cb = ca;
    cb.memo = false;
    let b = explore(cb);
    assert_eq!(
        a.first.keys().collect::<Vec<_>>(),
        b.first.keys().collect::<Vec<_>>()
    );
    assert_eq!(a.max_pending, b.max_pending);
    assert_eq!(a.terminals > 0, b.terminals > 0);
    assert_eq!(
        a.schedules as u64, b.terminals as u64,
        "the memoised graph counts exactly the schedules the un-memoised run executes"
    );
    show("memo", ca, &a);
    show("nomemo", cb, &b);
}

/// Run `path`, then let the manager and the GPU finish (the guest does nothing more), and return
/// the world at quiescence.
fn settle_world(path: &[Ev]) -> World {
    let mut w = replay(path);
    let c = cfg(0, false, ALL_KINDS, false);
    // The guest first completes the arm it is in the middle of.
    while let Some(&e) = w.enabled(c).first() {
        w.apply(e);
        if w.enabled(c).is_empty() {
            break;
        }
    }
    w
}

fn assert_clean(w: &World) {
    let v: Vec<_> = w.check().into_iter().chain(w.check_terminal()).collect();
    assert!(v.is_empty(), "violations at quiescence: {v:?}");
}

/// ★ MINIMAL I1 (8 events): arm A is not yet observed; the guest starts its NEXT invalidate and
/// writes B's PDB registers (no trigger yet). The manager then rebuilds A's request from the
/// latches, walks B's space, and clears A's trigger: A's space was never refreshed.
#[test]
fn a_later_pdb_latch_must_not_redirect_an_earlier_arm() {
    let w = settle_world(&[
        Ev::Prep(Kind::A),
        Ev::Arm,
        Ev::Sig,
        Ev::Prep(Kind::B),
        Ev::Drain,
        Ev::Observe,
        Ev::GpuFinish,
        Ev::Ready,
    ]);
    assert_clean(&w);
}

/// ★ MINIMAL I1/I4 (a second arm overwrites an unobserved first): A(pdb A), A, B all armed before
/// the manager looks. The port names only the newest; the earlier specific arms must still be
/// refreshed after their arms, and the clear may come only after that.
#[test]
fn an_unobserved_specific_arm_is_not_lost_when_a_later_arm_overwrites_it() {
    let w = settle_world(&[
        Ev::Prep(Kind::A),
        Ev::Arm,
        Ev::Sig,
        Ev::Prep(Kind::A),
        Ev::Arm,
        Ev::Sig,
        Ev::Prep(Kind::B),
        Ev::Arm,
        Ev::Sig,
        Ev::Drain,
        Ev::Observe,
        Ev::GpuFinish,
        Ev::Ready,
    ]);
    assert_clean(&w);
}

/// ★ I3/I5: a storm of K arms (specific and ALL_PDB, alternating) while ONE walk is in flight.
/// The manager keeps looking after every arm. Pending work must stay bounded by the spaces and
/// exactly ONE follow-up walk must run — the last arm completed by a walk submitted after it.
#[test]
fn a_storm_of_arms_during_one_walk_leaves_one_bounded_follow_up() {
    const K: usize = 200;
    let mut w = World::new();
    let go = |w: &mut World, k: Kind| {
        for e in [
            Ev::Prep(k),
            Ev::Arm,
            Ev::Sig,
            Ev::Drain,
            Ev::Observe,
            Ev::Ready,
        ] {
            w.apply(e);
        }
    };
    go(&mut w, Kind::A); // walk 1 in flight (held on the "GPU")
    assert!(w.r.m.in_flight());
    let mut worst = 0;
    for i in 0..K {
        go(&mut w, [Kind::A, Kind::B, Kind::All][i % 3]);
        worst = worst.max(w.r.m.pending());
    }
    assert!(
        worst <= BOUND_PENDING,
        "pending reached {worst} during a storm of {K} arms (bound {BOUND_PENDING})"
    );
    w.apply(Ev::GpuFinish);
    w.apply(Ev::Drain);
    w.apply(Ev::Observe);
    w.apply(Ev::Ready);
    assert!(w.r.m.in_flight(), "the ONE follow-up walk is in flight");
    assert_eq!(w.r.m.pending(), 0);
    w.apply(Ev::GpuFinish);
    w.apply(Ev::Drain);
    w.apply(Ev::Observe);
    w.apply(Ev::Ready);
    assert_eq!(
        w.r.m.walker().submits.len(),
        2,
        "{K} arms during one walk: one walk, then exactly one follow-up"
    );
    assert_clean(&w);
}

// ---------------------------------------------------------------------------------------------
// Real threads. The model above fixes WHICH interleavings exist; this runs the same port and the
// same manager against real concurrent guests, so the atomics' orderings (record before arm, arm
// before signal, compare-and-clear) are exercised by the hardware too. Seeds fix each guest's
// choices; the OS schedule is whatever it is, and every assertion is schedule-independent.
// ---------------------------------------------------------------------------------------------

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// `(pdb, va, at)` per PTE the guests have written, in write order.
type Ptes = Arc<Mutex<Vec<(u64, u64, u64)>>>;

fn stress(seed: u64, polite: bool) {
    const GUESTS: u64 = 3;
    const ARMS: u64 = 40;
    let port = Arc::new(InvalidatePort::new(
        InvalidateRegs::from_usermode_base(0xBB_0000).unwrap(),
    ));
    let ptes: Ptes = Arc::default();
    // Guests serialise the latch+trigger writes (one vCPU's trap); RM's lock additionally covers
    // the wait for the bit when `polite`.
    let rm_lock = Arc::new(Mutex::new(()));
    let trap_lock = Arc::new(Mutex::new(()));
    let wake = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let completed_ver = Arc::new(AtomicU64::new(0));
    let total_arms = GUESTS * ARMS;

    let mgr = {
        let (port, ptes, wake, done, completed_ver) = (
            port.clone(),
            ptes.clone(),
            wake.clone(),
            done.clone(),
            completed_ver.clone(),
        );
        std::thread::spawn(move || {
            let mut r = rig_with(port.clone());
            let cur_ver = Rc::new(std::cell::Cell::new(0u64));
            {
                // The version of the walk is published the moment the walker REPORTS it (before
                // any clear), so a guest that sees the bit idle finds it there.
                let (cur_ver, completed_ver) = (cur_ver.clone(), completed_ver.clone());
                r.m.walker_mut().on_collect = Some(Box::new(move || {
                    completed_ver.fetch_max(cur_ver.get(), Ordering::AcqRel);
                }));
            }
            {
                let (tables, ptes, cur_ver) = (r.tables.clone(), ptes.clone(), cur_ver.clone());
                let mut synced = 0usize;
                r.m.walker_mut().pre_submit = Some(Box::new(move || {
                    let g = ptes.lock().unwrap();
                    for &(pdb, va, at) in &g[synced..] {
                        tables
                            .borrow_mut()
                            .entry(pdb)
                            .or_default()
                            .push((va, at, 0x1000, 0));
                    }
                    synced = g.len();
                    cur_ver.set(g.len() as u64);
                }));
            }
            let mut rng = Rng(seed ^ 0x9E37_79B9_7F4A_7C15);
            let mut after: Option<u64> = None;
            let started = Instant::now();
            let mut max_pending = 0;
            loop {
                assert!(
                    started.elapsed() < Duration::from_secs(60),
                    "seed {seed}: the manager is stuck (bit busy={}, pending={})",
                    busy(&port),
                    r.m.pending()
                );
                let woke = wake.swap(false, Ordering::AcqRel);
                r.m.observe_port(&port, &mut after);
                for _ in 0..(rng.next() % 64) {
                    std::hint::spin_loop();
                }
                let out = r.m.on_walk_ready(port.trigger());
                max_pending = max_pending.max(r.m.pending());
                assert!(
                    r.m.pending() <= BOUND_PENDING,
                    "seed {seed}: pending {} exceeds {BOUND_PENDING}",
                    r.m.pending()
                );
                if woke || out.collected || r.m.in_flight() || r.m.pending() > 0 {
                    continue;
                }
                if done.load(Ordering::Acquire) {
                    // The guests are gone. Anything still armed that we have not taken is a lost wake.
                    if port.armed_since(after).is_none() {
                        break;
                    }
                    continue;
                }
                // Event-driven: sleep until a guest signals. The timeout is a WATCHDOG only; if it
                // fires with the bit busy and nothing to do, a wake was lost.
                std::thread::park_timeout(Duration::from_secs(10));
                if !wake.load(Ordering::Acquire) && !done.load(Ordering::Acquire) {
                    assert!(
                        !busy(&port) || port.armed_since(after).is_some(),
                        "seed {seed}: LOST WAKE - the bit is busy, nothing is pending and nobody signalled"
                    );
                }
            }
            // Quiescent: idle bit, every PTE any guest wrote is on the host, walks <= arms.
            assert!(!busy(&port), "seed {seed}: the bit is busy at quiescence");
            let host = {
                let mut h = BTreeMap::new();
                for (op, _) in r.ops.borrow().iter() {
                    if let Op::Map(va, off, _) = op {
                        h.insert(*va, *off);
                    }
                }
                h
            };
            for &(_, va, at) in ptes.lock().unwrap().iter() {
                assert_eq!(
                    host.get(&va),
                    Some(&at),
                    "seed {seed}: PTE {va:#x} never reached the host"
                );
            }
            let walks = r.m.walker().submits.len() as u64;
            assert!(
                walks <= total_arms,
                "seed {seed}: {walks} walks for {total_arms} arms"
            );
            (walks, max_pending)
        })
    };
    let mgr_thread = mgr.thread().clone();

    let guests: Vec<_> = (0..GUESTS)
        .map(|g| {
            let (port, ptes, rm_lock, trap_lock, wake, completed_ver, mgr_thread) = (
                port.clone(),
                ptes.clone(),
                rm_lock.clone(),
                trap_lock.clone(),
                wake.clone(),
                completed_ver.clone(),
                mgr_thread.clone(),
            );
            std::thread::spawn(move || {
                let mut rng = Rng(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ (g + 1));
                for i in 0..ARMS {
                    let kind = [Kind::A, Kind::B, Kind::All][(rng.next() % 3) as usize];
                    let _rm = polite.then(|| rm_lock.lock().unwrap());
                    let idx = g * ARMS + i;
                    let my_ver = {
                        let mut p = ptes.lock().unwrap();
                        let spaces: &[u64] = match kind {
                            Kind::A => &[PDB_A],
                            Kind::B => &[PDB_B],
                            Kind::All => &[PDB_A, PDB_B],
                        };
                        for &pdb in spaces {
                            let (va0, at0) = if pdb == PDB_A {
                                (0x1000_0000u64, 0x0200_0000u64)
                            } else {
                                (0x2000_0000, 0x0210_0000)
                            };
                            p.push((pdb, va0 + idx * 0x10000, at0 + idx * 0x10000));
                        }
                        p.len() as u64
                    };
                    {
                        let _t = trap_lock.lock().unwrap();
                        let regs = port.regs();
                        if kind != Kind::All {
                            let pdb = if kind == Kind::A { PDB_A } else { PDB_B };
                            let (lo, hi) = Invalidate::encode_pdb(pdb, PdbAperture::Vidmem);
                            port.write(regs.pdb, lo);
                            port.write(regs.upper_pdb, hi);
                        }
                        let word = kf_trap::mmuinval::TRIGGER_BIT
                            | 1
                            | if kind == Kind::All { 0b10 } else { 0 };
                        assert!(matches!(
                            port.write(regs.trigger, word),
                            PortWrite::Publish(_)
                        ));
                    }
                    wake.store(true, Ordering::Release);
                    mgr_thread.unpark();
                    // The guest polls the bit. Polite: until it reads idle. Hostile: a few reads and on.
                    let t0 = Instant::now();
                    let mut spins = 0u32;
                    loop {
                        if !busy(&port) {
                            // I1 with real atomics: an idle bit means a walk submitted after my
                            // PTE write has been collected.
                            assert!(
                                completed_ver.load(Ordering::Acquire) >= my_ver,
                                "seed {seed}: guest {g} arm {i}: the bit reads idle but no walk of version >= {my_ver} completed"
                            );
                            break;
                        }
                        spins += 1;
                        if !polite && spins > 200 {
                            break;
                        }
                        assert!(
                            t0.elapsed() < Duration::from_secs(60),
                            "seed {seed}: guest {g} arm {i} never saw the bit clear"
                        );
                        std::hint::spin_loop();
                    }
                }
            })
        })
        .collect();
    for g in guests {
        g.join().unwrap();
    }
    done.store(true, Ordering::Release);
    mgr_thread.unpark();
    let (walks, max_pending) = mgr.join().unwrap();
    eprintln!(
        "stress seed {seed} polite={polite}: {total_arms} arms, {walks} walks, max pending {max_pending}"
    );
}

#[test]
fn real_threads_polite_guests_never_see_a_stale_clear_or_a_lost_wake() {
    for seed in 1..=12 {
        stress(seed, true);
    }
}

#[test]
fn real_threads_hostile_arm_storms_lose_no_pte_and_build_no_backlog() {
    for seed in 1..=12 {
        stress(seed, false);
    }
}

// ---------------------------------------------------------------------------------------------
// The merge rules, one by one (the checker above proves them over every schedule; these name them).
// ---------------------------------------------------------------------------------------------

/// A rig whose walks stay on the "GPU" until `release`d, and the manager's `after` cursor.
struct Held {
    r: Rig,
    after: Option<u64>,
}

impl Held {
    fn new() -> Held {
        let mut r = rig();
        r.m.walker_mut().hold_walks = true;
        Held { r, after: None }
    }

    /// The guest arms (PDB latches + trigger) and the manager looks.
    fn arm(&mut self, pdb: u64, all_pdb: bool) -> InvalidateRequest {
        let q = guest_invalidate(&self.r.port, pdb, all_pdb);
        self.r.m.observe_port(&self.r.port, &mut self.after);
        q
    }

    /// The guest arms several times, then the manager looks once.
    fn arm_unobserved(&mut self, pdb: u64, all_pdb: bool) -> InvalidateRequest {
        guest_invalidate(&self.r.port, pdb, all_pdb)
    }

    fn look(&mut self) {
        self.r.m.observe_port(&self.r.port, &mut self.after);
    }

    /// The GPU finishes the walk in flight and the manager collects it.
    fn release(&mut self) -> Reconciled {
        self.r.m.walker_mut().hold = false;
        self.r.m.on_walk_ready(self.r.port.trigger())
    }

    /// What is waiting, as words.
    fn shape(&self) -> Vec<String> {
        let name = |s: Scope| match s {
            Scope::All => "All".to_string(),
            Scope::Pdb(p, _) if p == PDB_A => "A".to_string(),
            Scope::Pdb(p, _) if p == PDB_B => "B".to_string(),
            Scope::Pdb(p, _) => format!("{p:#x}"),
        };
        self.r
            .m
            .pending
            .iter()
            .map(|w| match w {
                Want::Invalidate(r, _) => format!("Inv({})", name(scope_of(r))),
                Want::Refresh(s, _) => format!("Ref({})", name(*s)),
                Want::Root(k) => format!("Root({k:?})"),
                Want::Split { ticket, .. } => format!("Split({ticket})"),
            })
            .collect()
    }
}

/// A global arrival subsumes every waiting specific: they are dropped, not queued beside it.
#[test]
fn a_global_arm_subsumes_every_waiting_specific() {
    let mut h = Held::new();
    h.arm(PDB_A, false);
    assert!(h.r.m.in_flight());
    h.arm(PDB_B, false);
    assert_eq!(h.shape(), ["Inv(B)"]);
    h.arm(PDB_A, false);
    assert_eq!(
        h.shape(),
        ["Inv(A)", "Ref(B)"],
        "the older specific is kept as a scope"
    );
    h.arm(0, true);
    assert_eq!(h.shape(), ["Inv(All)"], "one global covers both");
}

/// A specific arriving after a global keeps the global beside it (the global's scope is not lost,
/// and the specific carries the trigger's sequence); the follow-up walk covers both spaces.
#[test]
fn a_specific_arm_after_a_global_keeps_the_global_scope_and_clears() {
    let mut h = Held::new();
    h.arm(PDB_A, false);
    h.arm(0, true);
    h.arm(PDB_B, false);
    assert_eq!(h.shape(), ["Inv(B)", "Ref(All)"]);
    h.release();
    assert!(h.r.m.in_flight(), "ONE follow-up walk");
    assert_eq!(h.r.m.pending(), 0);
    let out = h.release();
    assert!(!busy(&h.r.port));
    assert_eq!(
        out.completed.last().map(|c| c.1),
        Some(ClearOutcome::Cleared)
    );
    let mut second = h.r.m.walker().submits[1].clone();
    second.sort_unstable();
    assert_eq!(second, vec![PDB_A, PDB_B], "the global's scope was walked");
    assert_eq!(h.r.m.walker().submits.len(), 2);
}

/// ★ I4: a specific refresh is not dropped because ANOTHER space's walk completed.
#[test]
fn a_specific_arm_is_kept_when_another_spaces_walk_completes() {
    let mut h = Held::new();
    h.r.tables
        .borrow_mut()
        .insert(PDB_A, vec![(0x1000_0000, 0x0200_0000, 0x1000, 0)]);
    h.r.tables
        .borrow_mut()
        .insert(PDB_B, vec![(0x2000_0000, 0x0210_0000, 0x1000, 0)]);
    h.arm(PDB_A, false);
    h.arm(PDB_B, false);
    assert_eq!(h.r.m.walker().submits, vec![vec![PDB_A]]);
    let out = h.release();
    assert!(busy(&h.r.port), "A's walk must not clear B");
    assert_eq!(out.completed.len(), 1);
    assert!(
        h.r.m.in_flight(),
        "B's own walk follows, submitted after B's arm"
    );
    h.release();
    assert!(!busy(&h.r.port));
    assert_eq!(h.r.m.walker().submits, vec![vec![PDB_A], vec![PDB_B]]);
    assert!(ops(&h.r).contains(&Op::Map(0x2000_0000, 0x0210_0000, 0x1000)));
}

/// ★ Several arms the manager never looked at: all refreshed by the walk it starts when it does.
#[test]
fn arms_the_manager_never_looked_at_are_all_refreshed_by_one_walk() {
    let mut h = Held::new();
    for i in 0..5u64 {
        let (pdb, va, at) = if i % 2 == 0 {
            (PDB_A, 0x1000_0000 + i * 0x10000, 0x0200_0000 + i * 0x10000)
        } else {
            (PDB_B, 0x2000_0000 + i * 0x10000, 0x0210_0000 + i * 0x10000)
        };
        h.r.tables
            .borrow_mut()
            .entry(pdb)
            .or_default()
            .push((va, at, 0x1000, 0));
        h.arm_unobserved(pdb, false);
    }
    h.look();
    let mut s = h.r.m.walker().submits[0].clone();
    s.sort_unstable();
    assert_eq!(
        s,
        vec![PDB_A, PDB_B],
        "one walk names both spaces the arms named"
    );
    h.release();
    assert!(!busy(&h.r.port));
    assert_eq!(h.r.m.walker().submits.len(), 1);
    assert_eq!(
        ops(&h.r)
            .iter()
            .filter(|o| matches!(o, Op::Map(..)))
            .count(),
        5
    );
    let t = h.r.port.trigger();
    assert_eq!(
        (t.issued(), t.completed()),
        (5, 5),
        "every arm accounted for"
    );
}

/// More unobserved arms than the port remembers: the walk covers everything (the overflow is a
/// global refresh). The FIRST arm names space B and falls out of the port's memory behind a long
/// run of arms that all name A; B's PTE must still reach the host.
#[test]
fn more_unobserved_arms_than_the_port_remembers_refresh_everything() {
    let mut h = Held::new();
    let n = 3 * kf_trap::mmuinval::ARM_RING as u64;
    for i in 0..n {
        let (pdb, va, at) = if i == 0 {
            (PDB_B, 0x2000_0000, 0x0210_0000)
        } else {
            (PDB_A, 0x1000_0000 + i * 0x10000, 0x0200_0000 + i * 0x10000)
        };
        h.r.tables
            .borrow_mut()
            .entry(pdb)
            .or_default()
            .push((va, at, 0x1000, 0));
        h.arm_unobserved(pdb, false);
    }
    h.look();
    h.release();
    assert!(!busy(&h.r.port));
    assert_eq!(
        ops(&h.r)
            .iter()
            .filter(|o| matches!(o, Op::Map(..)))
            .count() as u64,
        n,
        "every PTE any arm wrote reached the host"
    );
    assert!(ops(&h.r).contains(&Op::Map(0x2000_0000, 0x0210_0000, 0x1000)));
}

/// ★ An arm whose record is not there (a vCPU minted its sequence and has not stored the scope
/// yet) is treated as ALL_PDB: it is cleared only after a walk of EVERY space, never after a
/// guess.
#[test]
fn an_arm_whose_record_is_not_there_refreshes_every_space() {
    let mut h = Held::new();
    for (pdb, va, at) in [
        (PDB_A, 0x1000_0000, 0x0200_0000),
        (PDB_B, 0x2000_0000, 0x0210_0000),
    ] {
        h.r.tables
            .borrow_mut()
            .entry(pdb)
            .or_default()
            .push((va, at, 0x1000, 0));
    }
    // Sequence 0 minted and armed with no record at all.
    let t = h.r.port.trigger();
    let s = t.mint();
    t.arm_minted(s);
    h.look();
    let mut walked = h.r.m.walker().submits[0].clone();
    walked.sort_unstable();
    assert_eq!(walked, vec![PDB_A, PDB_B]);
    h.release();
    assert!(!busy(&h.r.port));
    assert_eq!(
        ops(&h.r)
            .iter()
            .filter(|o| matches!(o, Op::Map(..)))
            .count(),
        2
    );
}

/// A hostile guest naming arbitrary page directories cannot grow the queue: a directory no space
/// is rooted at names nothing that can be stale.
#[test]
fn arms_naming_unknown_page_directories_build_no_backlog() {
    let mut h = Held::new();
    h.arm(PDB_A, false);
    for i in 0..500u64 {
        h.arm(0x4000_0000 + i * 0x1000, false);
        assert!(
            h.r.m.pending() <= 1,
            "pending {} after {i} arms",
            h.r.m.pending()
        );
    }
    assert_eq!(
        h.r.m.stats.named_missed, 499,
        "each superseded unknown directory was counted"
    );
}

/// Root walks for one space are not queued twice: they would be the same walk.
#[test]
fn root_walks_for_one_space_are_queued_once() {
    let mut h = Held::new();
    h.arm(PDB_A, false);
    for _ in 0..50 {
        h.r.m.schedule_walk(K_B, h.r.port.trigger());
    }
    assert_eq!(h.shape(), [format!("Root({K_B:?})")]);
    assert_eq!(h.r.m.abandon_life().roots, 1);
}

/// A want that must walk again (a partial diff) is merged with the newer arm that arrived while
/// it walked: one trigger-bearing invalidate in the follow-up walk, not two.
#[test]
fn a_requeued_invalidate_merges_with_the_newer_arm() {
    let mut h = Held::new();
    h.r.m.walker_mut().cap = 2;
    h.r.tables.borrow_mut().insert(
        PDB_A,
        vec![
            (0x1000, 0x10_0000, 0x1000, 0),
            (0x3000, 0x20_0000, 0x1000, 0),
        ],
    );
    h.r.m.walker_mut().hold_walks = false;
    settle(&mut h.r, PDB_A);
    h.r.m.walker_mut().hold_walks = true;
    h.r.tables.borrow_mut().insert(
        PDB_A,
        vec![
            (0x3000, 0x30_0000, 0x1000, 0),
            (0x5000, 0x40_0000, 0x1000, 0),
        ],
    );
    let a1 = h.arm(PDB_A, false);
    let a2 = h.arm(PDB_A, false);
    assert_eq!(h.shape(), ["Inv(A)"]);
    h.release();
    // The partial diff withheld its maps; A1 walks again - merged with A2, not beside it.
    let inflight = h.r.m.inflight.as_ref().expect("walked again at once");
    let seqs: Vec<u64> = inflight
        .wants
        .iter()
        .filter_map(|(w, _, _)| match w {
            Want::Invalidate(r, _) => Some(r.seq),
            _ => None,
        })
        .collect();
    assert_eq!(seqs, vec![a2.seq], "only the newest carries the sequence");
    assert!(a1.seq < a2.seq);
    h.release();
    assert!(!busy(&h.r.port));
}
