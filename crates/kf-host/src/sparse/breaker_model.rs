
// ── Adversarial op-sequence model (breaker, 2026-10-26) ───────────────────────────────────────
//
// A deterministic xorshift generator draws hostile sequences of the verbs the VA-manager side
// issues (plain map, exact/partial range unmap, sparse admit, sparse retire with and without a
// replacement, retry at refresh, space recycle, transient RM refusals). A model RM (blocks and
// page states, with the RM's real rules: a reservation inside a live block is refused, a free
// takes its mappings with it, an unmap inside a sparse block returns the page to sparse) checks
// the invariants after EVERY step. A failing sequence is shrunk by greedy op removal and printed.

use std::collections::BTreeSet;

mod fz {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Op {
        /// plain map (va, len) of a currently unmapped row
        Map(u64, u64),
        /// unmap `[va, va+len)` (a sub-range of a mapped row or a whole row)
        Unmap(u64, u64),
        Admit(u64, u64),
        Retire(u64, u64, bool),
        Retry,
        Clear,
        /// the n-th next reserve call is refused by the RM (1 = the very next)
        FailNth(usize),
    }

    #[derive(Default)]
    pub struct Model {
        pub blocks: RefCell<BTreeMap<u32, (u64, u64, bool)>>,
        pub maps: RefCell<BTreeMap<u64, u64>>,
        next: Cell<u32>,
        pub fail_in: Cell<usize>,
        pub destroyed: RefCell<Vec<u64>>,
    }
    impl Model {
        pub fn new() -> Self {
            let m = Self::default();
            m.blocks
                .borrow_mut()
                .insert(1, (LO, LO + WIDE * NORMAL_GRAIN, false));
            m.next.set(2);
            m
        }
        pub fn sparse_at(&self, va: u64) -> bool {
            self.blocks
                .borrow()
                .values()
                .any(|&(lo, hi, s)| s && lo <= va && va < hi)
        }
        pub fn sparse_pages(&self) -> BTreeSet<u64> {
            let mut out = BTreeSet::new();
            for &(lo, hi, s) in self.blocks.borrow().values() {
                if s {
                    out.extend((lo..hi).step_by(PAGE as usize));
                }
            }
            out
        }
        pub fn state(&self, va: u64) -> Option<u64> {
            self.maps
                .borrow()
                .get(&va)
                .copied()
                .or_else(|| self.sparse_at(va).then_some(0))
        }
    }
    impl Ops for Model {
        fn reserve(&self, lo: u64, len: u64, sparse: bool) -> Result<u32, RmError> {
            let n = self.fail_in.get();
            if n > 0 {
                self.fail_in.set(n - 1);
                if n == 1 {
                    return Err(RmError::NoMemory);
                }
            }
            if self
                .blocks
                .borrow()
                .values()
                .any(|&(a, b, _)| lo < b && a < lo + len)
            {
                return Err(RmError::NoMemory);
            }
            let h = self.next.get();
            self.next.set(h + 1);
            self.blocks.borrow_mut().insert(h, (lo, lo + len, sparse));
            Ok(h)
        }
        fn release(&self, h: u32) -> Result<(), RmError> {
            let (lo, hi, _) = self.blocks.borrow_mut().remove(&h).ok_or(RmError::NoMemory)?;
            // The RM takes every mapping of a freed reservation with it.
            let gone: Vec<u64> = self
                .maps
                .borrow()
                .keys()
                .copied()
                .filter(|&p| lo <= p && p < hi)
                .collect();
            for p in gone {
                self.maps.borrow_mut().remove(&p);
                self.destroyed.borrow_mut().push(p);
            }
            Ok(())
        }
    }

    fn wanted_pages(t: &Reservations) -> BTreeSet<u64> {
        let mut out = BTreeSet::new();
        for b in t.blocks.values() {
            for p in (b.lo..b.hi.min(b.lo + NORMAL_GRAIN)).step_by(PAGE as usize) {
                if Reservations::wanted(b, p, p + PAGE) {
                    out.insert(p);
                }
            }
        }
        out
    }

    /// One run of a sequence. Returns the first violated invariant, named.
    pub fn run(ops: &[Op], limit: usize, lenient: bool) -> Result<(), String> {
        let m = Model::new();
        let mut t = Reservations::new(
            &[GuestVaRange {
                handle: 1,
                lo: LO,
                hi: LO + WIDE * NORMAL_GRAIN,
            }],
            limit,
        );
        // truth: pages the guest has live as plain maps; sparse intent the host accepted
        let mut live: BTreeMap<u64, u64> = BTreeMap::new();
        let mut honoured: BTreeSet<u64> = BTreeSet::new();
        let mut ever_wanted: BTreeSet<u64> = BTreeSet::new();
        let mut serial = 0x1000u64;
        for (i, op) in ops.iter().enumerate() {
            let at = |why: &str| format!("op #{i} {op:?}: {why}");
            let prev_sparse = m.sparse_pages();
            match *op {
                Op::FailNth(n) => m.fail_in.set(n),
                Op::Map(va, len) => {
                    if (va..va + len)
                        .step_by(PAGE as usize)
                        .any(|p| live.contains_key(&p))
                    {
                        continue;
                    }
                    let pieces = t
                        .route_map(&m, RANGE, va, len, !t.poisoned)
                        .map_err(|e| at(&format!("a plain map was refused: {e:?}")))?;
                    serial += 1;
                    for &(h, pva, plen) in &pieces {
                        if h != RANGE {
                            let (lo, hi, _) = *m
                                .blocks
                                .borrow()
                                .get(&h)
                                .ok_or_else(|| at("piece names a freed/unknown reservation"))?;
                            if pva < lo || pva + plen > hi {
                                return Err(at("piece outside its reservation"));
                            }
                        }
                        for p in (pva..pva + plen).step_by(PAGE as usize) {
                            m.maps.borrow_mut().insert(p, serial << 24 | p);
                            live.insert(p, serial << 24 | p);
                            honoured.remove(&p);
                            ever_wanted.remove(&p);
                        }
                    }
                }
                Op::Unmap(va, len) => {
                    let pieces = t
                        .pieces(RANGE, va, len)
                        .map_err(|e| at(&format!("an unmap could not route: {e:?}")))?;
                    for &(_, pva, plen) in &pieces {
                        for p in (pva..pva + plen).step_by(PAGE as usize) {
                            m.maps.borrow_mut().remove(&p);
                            live.remove(&p);
                        }
                        t.mark(pva, plen, false);
                    }
                }
                Op::Admit(va, len) => {
                    let before_sparse = m.sparse_pages();
                    let before_wanted = wanted_pages(&t);
                    match t.admit(&m, va, len) {
                        Ok(true) => {
                            for p in (va..va + len).step_by(PAGE as usize) {
                                if m.state(p).is_none() {
                                    return Err(at("admit said true but a page is plain invalid"));
                                }
                                if !live.contains_key(&p) {
                                    honoured.insert(p);
                                }
                                ever_wanted.insert(p);
                            }
                        }
                        Ok(false) => {
                            // held / declined: it must not have taken pages of OTHER runs
                            let after = m.sparse_pages();
                            for p in after.difference(&before_sparse) {
                                if !(va <= *p && *p < va + len) && !ever_wanted.contains(p) {
                                    return Err(at(&format!(
                                        "an unhonoured run sparsified {p:#x}, outside its range"
                                    )));
                                }
                            }
                            // a held run may still be honoured later
                            for p in (va..va + len).step_by(PAGE as usize) {
                                ever_wanted.insert(p);
                            }
                        }
                        Err(_) => {
                            if m.sparse_pages() != before_sparse {
                                return Err(at("a refused admission left sparse state behind (I3)"));
                            }
                            if wanted_pages(&t) != before_wanted {
                                return Err(at(
                                    "a refused admission left sparse intent behind (I3)",
                                ));
                            }
                        }
                    }
                }
                Op::Retire(va, len, keep) => {
                    let ok = t.retire(&m, va, len, keep).is_ok();
                    for p in (va..va + len).step_by(PAGE as usize) {
                        // retirement is the guest's change: even a partial (refused) retire
                        // only moves pages toward the requested state
                        honoured.remove(&p);
                        if ok {
                            ever_wanted.remove(&p);
                        }
                    }
                }
                Op::Retry => {
                    let _ = t.retry(&m);
                }
                Op::Clear => {
                    let was_live = !live.is_empty();
                    if t.clear(&m).is_ok() {
                        if was_live && !lenient {
                            return Err(at("a space with live maps was recycled (I1)"));
                        }
                        honoured.clear();
                        ever_wanted.clear();
                        if m.blocks.borrow().values().any(|b| b.2) {
                            return Err(at("sparse blocks survive a recycle"));
                        }
                    } else {
                        // a refused recycle makes the space ineligible for reuse: end of run
                        return Ok(());
                    }
                }
            }
            // ── invariants after every step ──
            let first_lost = m.destroyed.borrow().first().copied();
            if let Some(p) = first_lost {
                if !lenient {
                    return Err(at(&format!(
                        "the host freed a reservation that held live mapping {p:#x} (I1)"
                    )));
                }
                // known occupancy defect (breaker_i1_*): forget what the host lost, go on
                for p in m.destroyed.borrow_mut().drain(..) {
                    live.remove(&p);
                }
            }
            for (&p, &v) in &live {
                if m.maps.borrow().get(&p) != Some(&v) {
                    return Err(at(&format!("live page {p:#x} changed in the host (I1)")));
                }
            }
            for &p in &honoured {
                if !live.contains_key(&p) && m.state(p).is_none() {
                    return Err(at(&format!(
                        "honoured sparse page {p:#x} became plain invalid (I2)"
                    )));
                }
            }
            for p in m.sparse_pages().difference(&prev_sparse) {
                if !ever_wanted.contains(p) && !live.contains_key(p) {
                    return Err(at(&format!(
                        "page {p:#x} absorbs although no sparse leaf covers it (I2)"
                    )));
                }
            }
            let tree: BTreeSet<u32> = t.handles().into_iter().collect();
            let rm: BTreeSet<u32> = m.blocks.borrow().keys().copied().collect();
            if tree != rm {
                return Err(at(&format!(
                    "tree {tree:?} and RM {rm:?} disagree on reservations"
                )));
            }
            for &p in live.keys() {
                let b = t
                    .blocks
                    .range(..=p)
                    .next_back()
                    .map(|(_, b)| b)
                    .filter(|b| p < b.hi)
                    .ok_or_else(|| at("a live page is outside every block"))?;
                if !lenient && b.handle != 0 && !Reservations::occupied(b) {
                    return Err(at(&format!(
                        "live page {p:#x} sits in a block the tree thinks is empty (I1)"
                    )));
                }
            }
            if t.blocks.len() > limit.max(1) + 1 {
                return Err(at("the block bound is exceeded"));
            }
        }
        Ok(())
    }

    pub struct Rng(pub u64);
    impl Rng {
        pub fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        pub fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// A hostile row: tiles clustered at both ends and the middle, edge-hugging offsets, and
    /// lengths from one page to several tiles.
    pub fn row(r: &mut Rng) -> (u64, u64) {
        let tiles = [0, 1, 2, 3, 7, 8, 9, 31, 32, 33, 61, 62, 63];
        let tile = tiles[r.below(tiles.len() as u64) as usize];
        let off = [0, 1, 15, 16, 17, 256, 400, 496, 511][r.below(9) as usize] * PAGE;
        let len = [1, 1, 2, 15, 16, 17, 32, 64, 255, 511, 512, 513, 1024][r.below(13) as usize]
            * PAGE;
        let va = LO + tile * NORMAL_GRAIN + off;
        let end = (LO + WIDE * NORMAL_GRAIN).min(va + len);
        (va, end - va)
    }

    pub fn draw(seed: u64, n: usize, fail_ok: bool) -> Vec<Op> {
        let mut r = Rng(seed | 1);
        let mut ops = Vec::new();
        let mut mapped: Vec<(u64, u64)> = Vec::new();
        for _ in 0..n {
            let (va, len) = row(&mut r);
            match r.below(12) {
                0..=3 => {
                    ops.push(Op::Map(va, len));
                    mapped.push((va, len));
                }
                4 | 5 => {
                    if mapped.is_empty() {
                        continue;
                    }
                    let (mv, ml) = mapped[r.below(mapped.len() as u64) as usize];
                    let pg = ml / PAGE;
                    let a = r.below(pg);
                    let l = 1 + r.below(pg - a);
                    ops.push(Op::Unmap(mv + a * PAGE, l * PAGE));
                }
                6..=8 if r.below(2) == 0 => ops.push(Op::Admit(va, len)),
                6..=8 => ops.push(Op::Admit(
                    va / SPARSE_GRAIN * SPARSE_GRAIN,
                    SPARSE_GRAIN * (1 + r.below(3)),
                )),
                9 => ops.push(Op::Retire(va, len, r.below(2) == 0)),
                10 => ops.push(if r.below(3) == 0 { Op::Clear } else { Op::Retry }),
                _ if fail_ok => ops.push(Op::FailNth(1 + r.below(4) as usize)),
                _ => {}
            }
        }
        ops
    }

    /// Greedy delta-debugging: drop ops while the SAME KIND of violation persists.
    pub fn shrink(mut ops: Vec<Op>, limit: usize, lenient: bool) -> (Vec<Op>, String) {
        let kind = |e: &str| {
            e.split(':')
                .nth(1)
                .unwrap_or("")
                .split("0x")
                .next()
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let want = kind(&run(&ops, limit, lenient).unwrap_err());
        loop {
            let mut changed = false;
            let mut i = 0;
            while i < ops.len() {
                let mut cand = ops.clone();
                cand.remove(i);
                match run(&cand, limit, lenient) {
                    Err(e) if kind(&e) == want => {
                        ops = cand;
                        changed = true;
                    }
                    _ => i += 1,
                }
            }
            if !changed {
                break;
            }
        }
        let msg = run(&ops, limit, lenient).unwrap_err();
        (ops, msg)
    }

    pub fn campaign(limit: usize, seeds: u64, len: usize, fail_ok: bool, lenient: bool) -> Option<String> {
        let mut seen: BTreeMap<String, (Vec<Op>, String, u64)> = BTreeMap::new();
        for s in 1..=seeds {
            let ops = draw(s.wrapping_mul(0x9E37_79B9_7F4A_7C15), len, fail_ok);
            if run(&ops, limit, lenient).is_err() {
                let (min, msg) = shrink(ops, limit, lenient);
                let key: String = msg
                    .split(':')
                    .nth(1)
                    .unwrap_or("")
                    .split("0x")
                    .next()
                    .unwrap_or("")
                    .trim()
                    .chars()
                    .take(70)
                    .collect();
                let e = seen.entry(key).or_insert((min.clone(), msg, 0));
                e.2 += 1;
                if min.len() < e.0.len() {
                    e.0 = min;
                }
            }
        }
        if seen.is_empty() {
            return None;
        }
        let mut out = String::new();
        for (k, (ops, msg, n)) in &seen {
            out += &format!("\n[{k}] seeds={n}\n  {msg}\n  minimal: {ops:?}\n");
        }
        Some(out)
    }
}

