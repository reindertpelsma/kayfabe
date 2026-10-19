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
}

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

/// ★ The manager's observation of the port — exactly the production loop's step
/// (`device.rs`, `va_loop`): read the armed request, skip a sequence already taken, queue it.
fn observe(
    m: &mut VaManager<ModelWalker, FakeHost>,
    port: &InvalidatePort,
    last: &mut Option<u64>,
) {
    if let Some(req) = port.armed_request()
        && *last != Some(req.seq)
    {
        *last = Some(req.seq);
        m.on_invalidate(req, port.trigger());
    }
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
                v.extend([Kind::A, Kind::B, Kind::All].map(Ev::Prep));
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

    fn digest(&self) -> String {
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
    let mut ids: HashMap<String, usize> = HashMap::new();
    let mut edges: Vec<Vec<usize>> = Vec::new();
    let mut queue: VecDeque<(Vec<Ev>, usize)> = VecDeque::new();
    ids.insert(World::new().digest(), 0);
    edges.push(Vec::new());
    queue.push_back((Vec::new(), 0));
    while let Some((path, id)) = queue.pop_front() {
        let w = replay(&path);
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
                    let n = edges.len();
                    ids.insert(d, n);
                    edges.push(Vec::new());
                    let mut p = path.clone();
                    p.push(ev);
                    queue.push_back((p, n));
                    n
                }
            };
            edges[id].push(nid);
        }
    }
    rep.states = edges.len();
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
        "interleave[{name}] n={} polite={}: {} states, {} transitions, {} terminal states, {:.0} complete schedules, max pending {}, violations: {:?}",
        cfg.n,
        cfg.polite,
        rep.states,
        rep.transitions,
        rep.terminals,
        rep.schedules,
        rep.max_pending,
        rep.first.keys().collect::<Vec<_>>()
    );
}

/// RM's discipline (the next arm only after the bit reads idle): every invariant, exhaustively.
#[test]
fn polite_guest_n4_every_invariant_over_every_interleaving() {
    let cfg = Cfg {
        n: 4,
        polite: true,
        memo: true,
    };
    let rep = explore(cfg);
    show("polite", cfg, &rep);
    rep.assert_holds(&["I1", "I2", "I3", "I5", "I6"]);
}

/// ★ The adversarial guest (a timed-out invalidate, a hostile guest): arms back to back, any pdb,
/// any mix of specific and ALL_PDB, during every window of the manager. I1/I2: no stale clear, no
/// lost re-arm, no arm missed (a specific is never dropped because another space was walked).
#[test]
fn adversarial_guest_n3_no_stale_clear_and_no_lost_arm() {
    let cfg = Cfg {
        n: 3,
        polite: false,
        memo: true,
    };
    let rep = explore(cfg);
    show("adversarial", cfg, &rep);
    rep.assert_holds(&["I1", "I2"]);
}

/// ★ I3: the pending work is bounded by the spaces, not by the arms.
#[test]
fn adversarial_guest_n4_pending_work_is_bounded() {
    let cfg = Cfg {
        n: 4,
        polite: false,
        memo: true,
    };
    let rep = explore(cfg);
    show("adversarial", cfg, &rep);
    rep.assert_holds(&["I3"]);
}

/// I5 and I6 over the adversarial schedules.
#[test]
fn adversarial_guest_n3_walks_never_exceed_arms_and_progress_needs_no_timer() {
    let cfg = Cfg {
        n: 3,
        polite: false,
        memo: true,
    };
    let rep = explore(cfg);
    rep.assert_holds(&["I5", "I6"]);
}

/// The digest memo hides nothing: with and without it the same violations are found.
#[test]
fn the_state_digest_memo_hides_no_schedule() {
    let a = explore(Cfg {
        n: 2,
        polite: false,
        memo: true,
    });
    let b = explore(Cfg {
        n: 2,
        polite: false,
        memo: false,
    });
    assert_eq!(
        a.first.keys().collect::<Vec<_>>(),
        b.first.keys().collect::<Vec<_>>()
    );
    assert_eq!(a.max_pending, b.max_pending);
    assert_eq!(a.terminals > 0, b.terminals > 0);
    show(
        "memo",
        Cfg {
            n: 2,
            polite: false,
            memo: true,
        },
        &a,
    );
    show(
        "nomemo",
        Cfg {
            n: 2,
            polite: false,
            memo: false,
        },
        &b,
    );
}

/// Run `path`, then let the manager and the GPU finish (the guest does nothing more), and return
/// the world at quiescence.
fn settle_world(path: &[Ev]) -> World {
    let mut w = replay(path);
    let cfg = Cfg {
        n: 0,
        polite: false,
        memo: true,
    };
    // The guest first completes the arm it is in the middle of.
    while let Some(&e) = w.enabled(cfg).first() {
        w.apply(e);
        if w.enabled(cfg).is_empty() {
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
