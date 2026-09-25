//! ★★★★★ **v3 gate 9 — the walk kernel's DIFF is the protocol's, report for report.**
//! (Owner design + COMMIT-ON-ACK ruling 2026-09-25; `kf_cuda::diffmodel` is the spec.)
//!
//! The GPU half (`kf_diff_slots`, `kf_diff_emit`, `kf_commit_kernel`) and the Rust model are two
//! implementations of one protocol; this gate runs them side by side on the SAME random guest
//! tables, with the SAME random host verdicts, and requires every report to be identical — the
//! old CUDA suite's round-trip closure (`t_roundtrip_*`: apply(model, delta) == full walk),
//! re-aimed at commit-on-ack: after the host acknowledges everything, the slot expresses exactly
//! the walk and the next diff is empty. Per format (VER2 and VER3 — every family first-class).
//!
//! Then the throughput it exists for (`V3_P5_PORT_MAP.md` Q8): 13 000 separate guest-RAM pages in
//! ONE space, one added per walk — `--ce-client-guest-ram`'s shape. Every report must be ONE run,
//! and the GPU time per walk is measured at 1 000 and at 13 000 rows: the diff must not grow with
//! the space (the old emission was one GPU thread, ~0.53 µs per row, per walk).
//!
//! No RM, no QEMU: the tables live in an uploaded image; only the walk kernel runs.

use kf_cuda::abi::{KFWR_ACK_APPLIED, KFWR_ACK_FAILED, KFWR_ACK_HELD, KFWR_OP_MAP, KFWR_OP_UNMAP, KFWR_RF_HELD, KfMapRun};
use kf_cuda::abi::{kf_format_ver2, kf_format_ver3};
use kf_cuda::diffmodel::{self, AckCode, Committed};
use kf_cuda::walk::{DeviceImage, WalkCfg, WalkEntry, WalkKernel};
use kf_harness::Ledger as Checks;
use kf_harness::tables::{Tree, Tree3};
use std::collections::BTreeMap;

const IMG_BYTES: usize = 64 << 20;
const PT_A: u64 = 0x10_0000;
const PT_B: u64 = 0x50_0000;
const PT_BYTES: usize = 4 << 20;
/// Data pages (vidmem GPGA or guest-RAM GPA) live in `[DATA, IMG_BYTES)`.
const DATA: u64 = 16 << 20;
const PAGE: u64 = 0x1000;

fn main() {
    let mut l = Checks::default();
    for (tag, v3) in [("ver2", false), ("ver3", true)] {
        if let Err(e) = differential(&mut l, tag, v3) {
            l.check(if v3 { "ver3_run" } else { "ver2_run" }, false, e);
        }
    }
    if let Err(e) = throughput(&mut l) {
        l.check("throughput_run", false, e);
    }
    let v = l.verdict();
    println!("GATE9_VERDICT={}", if v { "PASS" } else { "FAIL" });
    std::process::exit(if v { 0 } else { 1 });
}

/// The guest's tables in either format, plus the harness's own record of what it mapped
/// (va → (at, sys)) — the model's walk is built from THAT, never from the GPU's report.
enum GuestTree {
    V2(Tree),
    V3(Tree3),
}

impl GuestTree {
    fn new(v3: bool, base: u64) -> GuestTree {
        if v3 { GuestTree::V3(Tree3::new(base, PT_BYTES)) } else { GuestTree::V2(Tree::new(base, PT_BYTES)) }
    }
    fn map(&mut self, va: u64, at: u64, sys: bool) {
        match (self, sys) {
            (GuestTree::V2(t), false) => t.map4k(va, at),
            (GuestTree::V2(t), true) => t.map4k_sys(va, at),
            (GuestTree::V3(t), false) => t.map4k(va, at),
            (GuestTree::V3(t), true) => t.map4k_sys(va, at),
        }
    }
    fn unmap(&mut self, va: u64) {
        match self {
            GuestTree::V2(t) => t.unmap4k(va),
            GuestTree::V3(t) => t.unmap4k(va),
        }
    }
    fn root(&self) -> u64 {
        match self {
            GuestTree::V2(t) => t.root,
            GuestTree::V3(t) => t.root,
        }
    }
    fn origin(&self) -> u64 {
        match self {
            GuestTree::V2(t) => t.img.origin,
            GuestTree::V3(t) => t.img.origin,
        }
    }
    fn used(&self) -> &[u8] {
        match self {
            GuestTree::V2(t) => &t.img.mem[..t.img.used()],
            GuestTree::V3(t) => &t.img.mem[..t.img.used()],
        }
    }
}

/// What the walker reports for `pages` — runs coalesced exactly as the kernel coalesces (VA and
/// backing contiguous, identical flags): the model's input.
fn walk_of(pages: &BTreeMap<u64, (u64, bool)>) -> Vec<KfMapRun> {
    let mut out: Vec<KfMapRun> = Vec::new();
    for (&va, &(at, sys)) in pages {
        let flags = if sys { 2 } else { 0 };
        if let Some(l) = out.last_mut()
            && l.va + l.len == va
            && l.gpga + l.len == at
            && l.flags == flags
        {
            l.len += PAGE;
            continue;
        }
        out.push(KfMapRun { va, gpga: at, len: PAGE, flags, op: KFWR_OP_MAP, pdb_index: 0 });
    }
    out
}

/// The comparable part of a run: op, va, len, backing, aperture, held.
fn key(r: &KfMapRun) -> (u16, u64, u64, u64, u32, bool) {
    (r.op, r.va, r.len, r.gpga, r.flags & 7, r.flags & KFWR_RF_HELD != 0)
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn write_tree(k: &WalkKernel, img: &DeviceImage, t: &GuestTree) -> Result<(), String> {
    k.write_image(img, t.origin(), t.used()).map_err(|e| e.to_string())
}

/// One entry of a live GPU report: its runs.
fn entry_runs(r: &kf_cuda::Report, i: usize) -> Vec<KfMapRun> {
    let p = r.pdbs[i];
    r.runs[p.first_run as usize..(p.first_run + p.run_count) as usize].to_vec()
}

#[allow(clippy::too_many_lines)]
fn differential(l: &mut Checks, tag: &str, v3: bool) -> Result<(), String> {
    let fmt = if v3 { kf_format_ver3() } else { kf_format_ver2() };
    let cfg = WalkCfg { table_version: fmt.table_version, ..WalkCfg::default() };
    let mut k = WalkKernel::bring_up_on(cfg, fmt, kf_cuda::walk::WalkDevice::PciBusId(&kf_harness::gate_bdf()?)).map_err(|e| format!("{tag}: {e}"))?;
    let img = k.upload(&vec![0u8; IMG_BYTES]).map_err(|e| e.to_string())?;
    // Slot 3 walks tree A, slot 5 walks tree B — and at step 40 slot 5's object MOVES its root
    // to tree A2 (the slot is the object's: the new root is diffed against what it placed).
    let mut trees = [GuestTree::new(v3, PT_A), GuestTree::new(v3, PT_B), GuestTree::new(v3, PT_B + (PT_BYTES as u64))];
    let mut pages: [BTreeMap<u64, (u64, bool)>; 3] = Default::default();
    let mut model: BTreeMap<u32, Committed> = BTreeMap::new();
    let mut r = Rng(0x2545_F491_4F6C_DD1D ^ u64::from(v3));
    let cap = cfg.runs_per_pdb as usize;
    let (mut steps, mut runs_seen, mut mismatches, mut failed_codes, mut held_codes) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut first_mismatch: Option<String> = None;
    let mut b_tree = 1usize;
    let mut resets = 0usize;
    for step in 0..160 {
        // ── the guest edits its tables ──
        for t in 0..3 {
            let base = 0x40_0000_0000u64 * (t as u64 + 1);
            for _ in 0..(1 + r.below(12)) {
                let va = base + r.below(512) * PAGE;
                match r.below(6) {
                    0 | 1 => {
                        trees[t].unmap(va);
                        pages[t].remove(&va);
                    }
                    2 if !pages[t].is_empty() => {
                        // Grow a neighbour contiguously (the walker coalesces it).
                        if let Some((&pv, &(pa, s))) = pages[t].range(..va).next_back() {
                            let (nv, na) = (pv + PAGE, pa + PAGE);
                            if na < IMG_BYTES as u64 {
                                trees[t].map(nv, na, s);
                                pages[t].insert(nv, (na, s));
                            }
                        }
                    }
                    _ => {
                        let sys = r.below(4) == 0;
                        let at = DATA + r.below(((IMG_BYTES as u64) - DATA) / PAGE) * PAGE;
                        trees[t].map(va, at, sys);
                        pages[t].insert(va, (at, sys));
                    }
                }
            }
            write_tree(&k, &img, &trees[t])?;
        }
        if step == 40 {
            b_tree = 2;
        }
        if step == 100 {
            // The object behind slot 5 is gone: its slot is released and emptied.
            k.reset_slot(5).map_err(|e| e.to_string())?;
            model.remove(&5);
            resets += 1;
        }
        let entries = [WalkEntry { pdb: trees[0].root(), slot: 3 }, WalkEntry { pdb: trees[b_tree].root(), slot: 5 }];
        let rep = k.refresh_image(&img, &entries).map_err(|e| format!("{tag} step {step}: {e}"))?;
        rep.validate().map_err(|e| format!("{tag} step {step}: {e}"))?;
        rep.require_diff().map_err(|e| format!("{tag} step {step}: {e}"))?;
        if rep.truncated() || rep.header.refusals != 0 {
            return Err(format!("{tag} step {step}: flags={:#x} refusals={} mask={:#x}", rep.header.flags, rep.header.refusals, rep.header.refuse_mask));
        }
        // ── the model, on the harness's own record of the tables ──
        let mut codes = vec![KFWR_ACK_FAILED; rep.runs.len()];
        for (i, (slot, which)) in [(3u32, 0usize), (5u32, b_tree)].into_iter().enumerate() {
            let walk = walk_of(&pages[which]);
            let com = model.get(&slot).cloned().unwrap_or_default();
            let want = diffmodel::diff(&com, &walk, cap);
            let got = entry_runs(&rep, i);
            runs_seen += got.len();
            let (a, b): (Vec<_>, Vec<_>) = (got.iter().map(key).collect(), want.runs.iter().map(key).collect());
            if a != b || rep.pdbs[i].reserved != slot {
                mismatches += 1;
                first_mismatch.get_or_insert_with(|| {
                    format!("step {step} slot {slot}: gpu={a:x?}\n  model={b:x?}")
                });
            }
            // ── the host's verdict: random refusals, never a map over a refused unmap ──
            let mut failed_unmaps: Vec<(u64, u64)> = Vec::new();
            let mut mc: Vec<AckCode> = Vec::with_capacity(want.runs.len());
            for x in &want.runs {
                let blocked = x.op == KFWR_OP_MAP && failed_unmaps.iter().any(|&(s, e)| x.va < e && s < x.va + x.len);
                let c = if blocked || r.below(5) == 0 {
                    AckCode::Failed
                } else if x.op == KFWR_OP_MAP && r.below(20) == 0 {
                    AckCode::Held
                } else {
                    AckCode::Applied
                };
                if x.op == KFWR_OP_UNMAP && c == AckCode::Failed {
                    failed_unmaps.push((x.va, x.va + x.len));
                }
                mc.push(c);
            }
            let first = rep.pdbs[i].first_run as usize;
            for (j, c) in mc.iter().enumerate() {
                codes[first + j] = match c {
                    AckCode::Failed => KFWR_ACK_FAILED,
                    AckCode::Applied => KFWR_ACK_APPLIED,
                    AckCode::Held => KFWR_ACK_HELD,
                };
                failed_codes += usize::from(*c == AckCode::Failed);
                held_codes += usize::from(*c == AckCode::Held);
            }
            model.insert(slot, diffmodel::commit(&com, &want.runs, &mc));
        }
        k.ack(rep.header.generation, codes).map_err(|e| e.to_string())?;
        steps += 1;
    }
    l.check(
        if v3 { "ver3_gpu_diff_is_the_model_diff" } else { "ver2_gpu_diff_is_the_model_diff" },
        mismatches == 0 && runs_seen > 100 && failed_codes > 10 && held_codes > 0 && resets == 1,
        format!(
            "{steps} walks, {runs_seen} runs compared, {mismatches} mismatching entries, verdicts failed={failed_codes} held={held_codes}{}",
            first_mismatch.map(|m| format!(" FIRST: {m}")).unwrap_or_default()
        ),
    );
    // ── closure: acknowledge everything until quiet; the GPU's slot then says what the walk says ──
    let mut quiet = false;
    for _ in 0..4 {
        let entries = [WalkEntry { pdb: trees[0].root(), slot: 3 }, WalkEntry { pdb: trees[b_tree].root(), slot: 5 }];
        let rep = k.refresh_image(&img, &entries).map_err(|e| e.to_string())?;
        if rep.runs.is_empty() {
            quiet = true;
            break;
        }
        let n = rep.runs.len();
        // Apply in the model too, so the final comparison is of the same history.
        for (i, (slot, which)) in [(3u32, 0usize), (5u32, b_tree)].into_iter().enumerate() {
            let com = model.get(&slot).cloned().unwrap_or_default();
            let want = diffmodel::diff(&com, &walk_of(&pages[which]), cap);
            let _ = i;
            model.insert(slot, diffmodel::commit(&com, &want.runs, &vec![AckCode::Applied; want.runs.len()]));
        }
        k.ack(rep.header.generation, vec![KFWR_ACK_APPLIED; n]).map_err(|e| e.to_string())?;
    }
    let closed = [(3u32, 0usize), (5u32, b_tree)].iter().all(|&(s, w)| {
        diffmodel::coverage(&model.get(&s).cloned().unwrap_or_default().flat()) == diffmodel::coverage(&walk_of(&pages[w]))
    });
    l.check(
        if v3 { "ver3_settled_slots_are_quiet_and_closed" } else { "ver2_settled_slots_are_quiet_and_closed" },
        quiet && closed,
        format!("quiet={quiet} closed={closed}"),
    );
    k.release(img);
    Ok(())
}

/// ★ Q8's shape: 13 000 separate guest-RAM pages, one added per walk.
fn throughput(l: &mut Checks) -> Result<(), String> {
    const ROWS: u64 = 13_000;
    let mut k = WalkKernel::bring_up_on(WalkCfg::default(), kf_format_ver2(), kf_cuda::walk::WalkDevice::PciBusId(&kf_harness::gate_bdf()?)).map_err(|e| e.to_string())?;
    let img = k.upload(&vec![0u8; IMG_BYTES]).map_err(|e| e.to_string())?;
    let mut tree = Tree::new(PT_A, PT_BYTES);
    let mut one_run = 0u64;
    let mut gpu_at = |n: u64, v: &mut Vec<u64>, us: u64| {
        if (n > 900 && n <= 1100) || n > ROWS - 200 {
            v.push(us);
        }
    };
    let (mut early, mut late, mut wall_late) = (Vec::new(), Vec::new(), Vec::new());
    let t_all = std::time::Instant::now();
    for i in 0..ROWS {
        let va = 0x1_2000_0000 + i * PAGE;
        // Separate pages: the backing skips a page each time, so nothing coalesces (13 000 runs).
        tree.map4k_sys(va, DATA + (2 * i * PAGE) % ((IMG_BYTES as u64) - DATA));
        k.write_image(&img, tree.img.origin, &tree.img.mem[..tree.img.used()]).map_err(|e| e.to_string())?;
        let t0 = std::time::Instant::now();
        k.submit_image(&img, &[WalkEntry { pdb: tree.root, slot: 0 }]).map_err(|e| e.to_string())?;
        let c = k.wait(10_000).map_err(|e| e.to_string())?;
        let wall = u64::try_from(t0.elapsed().as_micros()).unwrap_or(u64::MAX);
        let n = c.report.runs.len();
        if n == 1 && c.report.runs[0].va == va && c.report.runs[0].op == KFWR_OP_MAP {
            one_run += 1;
        }
        let rows = i + 1;
        gpu_at(rows, if rows <= 1100 { &mut early } else { &mut late }, c.gpu_us);
        if rows > ROWS - 200 {
            wall_late.push(wall);
        }
        k.ack(c.report.header.generation, vec![KFWR_ACK_APPLIED; n]).map_err(|e| e.to_string())?;
    }
    let med = |v: &mut Vec<u64>| {
        v.sort_unstable();
        v.get(v.len() / 2).copied().unwrap_or(0)
    };
    let (e, la, w) = (med(&mut early), med(&mut late), med(&mut wall_late));
    l.measure(
        "throughput",
        format!(
            "rows={ROWS} one_run_reports={one_run} gpu_us_p50@1000={e} gpu_us_p50@13000={la} wall_us_p50@13000={w} total_s={:.1}",
            t_all.elapsed().as_secs_f64()
        ),
    );
    l.check("every_added_page_is_a_one_run_diff", one_run == ROWS, format!("{one_run} of {ROWS}"));
    // ⊘ The old serial emission cost ~0.53 µs per row per walk: +6 400 µs between 1 000 and 13 000
    // rows. The bound allows the parallel diff's own O(n / threads) growth, not that.
    l.check("the_diff_does_not_grow_with_the_space", la <= e + 1_000, format!("gpu p50 {e} us @1000 rows -> {la} us @13000 rows"));
    k.release(img);
    Ok(())
}
