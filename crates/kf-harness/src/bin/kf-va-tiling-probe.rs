//! ★ 2026-11-18 — **what exact block tiling (option C2) costs host RM** (`docs/design/V3_SPARSE_RESIDENCY_RCA.md`
//! §8, measurements before any design commitment).
//!
//! C2 keeps every byte of a guest range in exactly one host `NV50_MEMORY_VIRTUAL` block: an empty
//! *fence* (INVALID), a *row* block (one guest mapping placed through it) or a `_SPARSE` *micro* block.
//! A fence holds no mapping, so it can be freed and re-reserved around a new row without unmapping
//! anything. This probe prices that on a bare host (no guest, no channel work submitted).
//!
//! Arms (one fresh bare space each, `L = GUEST_VA_RANGES[0].lo` = 4.5 GiB, `S` = 64 GiB):
//! - **M0** baseline: one static reservation `[L, L+S)`, then N FIXED 64 KiB maps through it and N
//!   unmaps (deferred TLB invalidation, one invalidate per batch, as production).
//! - **M1** exact tiling, eager merge: one fence `[L, L+S)`; per map: free the fence, reserve the left
//!   and right remnants, reserve the row block, map. Per unmap: unmap, free the row, free the
//!   neighbouring fences, reserve the merged fence. Same N and VAs as M0.
//! - **M2** eheap scaling: K disjoint 64 KiB blocks (K = 100, 1k, 4k, 16k, 32k, 64k; stop at the first
//!   refusal); at each K, time a reserve+free in a gap and a map+unmap through a block at the low,
//!   middle and high end.
//! - **M3** race window: M1's window (fence freed → last remnant reserved) is in M1's line; here a
//!   second thread places non-FIXED maps through the range object while M1 runs, and counts placements
//!   that land inside `[L, L+S)` and the refusals they cause.
//! - **M4** `_SPARSE` micro blocks: carve 64 KiB and 4 KiB `_SPARSE` blocks out of a fence, next to a
//!   fence, next to each other and next to a row block; map through one and unmap (RM restores sparse).
//!
//! Falsifier for C2's per-row tiling (stated before the run): M2 growing linearly to more than ~1 ms
//! per call at 16k blocks, or M1 costing more than ~10x M0 per guest map, means C2 needs COARSER
//! tiling (row blocks per walk batch, not per row).
//!
//! Prints `TILING ...` lines only.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use kf_host::channel::{GUEST_VA_RANGES, NVOS32_ALLOC_FLAGS_SPARSE, NVOS32_RESERVE_FLAGS};
use kf_host::{HostRm, MapBacking, MapPerm, VaSpace};
use kf_linux_raw::DevDir;

const PAGE: u64 = 0x1_0000;
const SPAN: u64 = 64 << 30;
const SLOTS: u64 = SPAN / PAGE;
const STRIDE: u64 = 1009; // odd, so `i * STRIDE mod SLOTS` is a permutation of the slots
const N: u64 = 1000;
const M2_STEPS: [u64; 6] = [100, 1000, 4000, 16_000, 32_000, 64_000];
const M2_REPS: usize = 21;

fn main() {
    let dev = match DevDir::open(c"/dev") {
        Ok(d) => d,
        Err(e) => {
            println!("TILING error=open_dev {e:?}");
            std::process::exit(2);
        }
    };
    let rm = match HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes) {
        Ok(r) => r,
        Err(e) => {
            println!("TILING error=open {e}");
            std::process::exit(2);
        }
    };
    let lo = GUEST_VA_RANGES[0].0;
    println!(
        "TILING driver={} base={lo:#x} span={SPAN:#x} n={N} page={PAGE:#x}",
        rm.driver_version()
    );
    let mem = match rm.alloc_device_local(PAGE) {
        Ok(m) => m,
        Err(e) => {
            println!("TILING error=alloc_mem {e:?}");
            std::process::exit(2);
        }
    };
    let only = std::env::args().nth(1);
    let want = |arm: &str| only.as_deref().is_none_or(|o| o == arm);
    let mut m0_map = None;
    if want("m0") {
        m0_map = m0(&rm, mem, lo);
    }
    if want("m1") {
        m1(&rm, mem, lo, m0_map);
    }
    if want("m2") {
        m2(&rm, mem, lo);
    }
    if want("m3") {
        m3(&rm, mem, lo);
    }
    if want("m4") {
        m4(&rm, mem, lo);
    }
    let _ = rm.free(mem);
    println!("TILING done");
}

fn slot_va(lo: u64, i: u64) -> u64 {
    lo + ((i * STRIDE) % SLOTS) * PAGE
}

/// Nanosecond samples, summarised as microseconds.
#[derive(Default)]
struct Stat(Vec<u64>);

impl Stat {
    fn push(&mut self, t: Instant) {
        self.0
            .push(u64::try_from(t.elapsed().as_nanos()).unwrap_or(u64::MAX));
    }
    fn mean_us(&self) -> f64 {
        if self.0.is_empty() {
            return 0.0;
        }
        self.0.iter().map(|&v| v as f64).sum::<f64>() / self.0.len() as f64 / 1000.0
    }
    fn show(&mut self) -> String {
        if self.0.is_empty() {
            return "n=0".into();
        }
        self.0.sort_unstable();
        let q = |p: usize| self.0[(self.0.len() - 1) * p / 100] as f64 / 1000.0;
        format!(
            "n={} p50={:.1}us p99={:.1}us max={:.1}us mean={:.1}us",
            self.0.len(),
            q(50),
            q(99),
            q(100),
            self.mean_us()
        )
    }
}

fn space(rm: &HostRm, arm: &str) -> Option<VaSpace> {
    match rm.alloc_vaspace_bare() {
        Ok(s) => Some(s),
        Err(e) => {
            println!("TILING arm={arm} space=ERR({e:?})");
            None
        }
    }
}

fn map_at(rm: &HostRm, h: u32, mem: u32, va: u64) -> Result<u64, kf_host::RmError> {
    rm.map_in(
        h,
        mem,
        MapBacking::Dedicated,
        0,
        PAGE,
        va,
        true,
        0,
        MapPerm::READ_WRITE,
    )
}

/// M0: one static reservation, N maps and N unmaps through it. Returns the mean us per map.
fn m0(rm: &HostRm, mem: u32, lo: u64) -> Option<f64> {
    let s = space(rm, "M0")?;
    let h = match rm.reserve_va(s.space, lo, SPAN) {
        Ok(h) => h,
        Err(e) => {
            println!("TILING arm=M0 resv=ERR({e:?})");
            rm.free_vaspace(s);
            return None;
        }
    };
    let (mut mp, mut um) = (Stat::default(), Stat::default());
    let mut err = String::new();
    for i in 0..N {
        let t = Instant::now();
        if let Err(e) = map_at(rm, h, mem, slot_va(lo, i)) {
            err = format!(" map_err_at={i}({e:?})");
            break;
        }
        mp.push(t);
    }
    let t = Instant::now();
    let inv = rm.invalidate_tlb(s).is_ok();
    let inv_us = t.elapsed().as_micros();
    for i in 0..u64::try_from(mp.0.len()).unwrap_or(0) {
        let t = Instant::now();
        if let Err(e) = rm.unmap_in(h, slot_va(lo, i), 0, true) {
            err.push_str(&format!(" unmap_err_at={i}({e:?})"));
            break;
        }
        um.push(t);
    }
    let _ = rm.invalidate_tlb(s);
    let mean = mp.mean_us();
    println!(
        "TILING arm=M0 calls_per_map=1 calls_per_unmap=1 map[{}] unmap[{}] invalidate_ok={inv} invalidate_us={inv_us}{err}",
        mp.show(),
        um.show()
    );
    let _ = rm.free(h);
    rm.free_vaspace(s);
    Some(mean)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Fence,
    Row,
    Sparse,
}

#[derive(Clone, Copy)]
struct Blk {
    hi: u64,
    kind: Kind,
    h: u32,
}

/// Exact tiling of `[lo, hi)`: every byte in exactly one block. Probe-local; `kf-mem`'s model is the
/// design's reference.
struct Tiling<'a> {
    rm: &'a HostRm,
    space: u32,
    blocks: BTreeMap<u64, Blk>,
    calls: u64,
    peak: usize,
    window: Stat,
}

impl<'a> Tiling<'a> {
    fn new(rm: &'a HostRm, space: u32, lo: u64, hi: u64) -> Result<Self, String> {
        let h = rm
            .reserve_va(space, lo, hi - lo)
            .map_err(|e| format!("fence_resv({e:?})"))?;
        let mut blocks = BTreeMap::new();
        blocks.insert(
            lo,
            Blk {
                hi,
                kind: Kind::Fence,
                h,
            },
        );
        Ok(Self {
            rm,
            space,
            blocks,
            calls: 1,
            peak: 1,
            window: Stat::default(),
        })
    }

    fn reserve(&mut self, lo: u64, hi: u64, kind: Kind) -> Result<(), String> {
        let flags = if kind == Kind::Sparse {
            NVOS32_RESERVE_FLAGS | NVOS32_ALLOC_FLAGS_SPARSE
        } else {
            NVOS32_RESERVE_FLAGS
        };
        self.calls += 1;
        let h = self
            .rm
            .reserve_va_probe_flags(self.space, lo, hi - lo, flags)
            .map_err(|e| format!("resv_{kind:?}[{lo:#x},{hi:#x})({e:?})"))?;
        self.blocks.insert(lo, Blk { hi, kind, h });
        self.peak = self.peak.max(self.blocks.len());
        Ok(())
    }

    fn release(&mut self, lo: u64) -> Result<Blk, String> {
        let b = self.blocks.remove(&lo).ok_or("no_block")?;
        self.calls += 1;
        self.rm
            .free(b.h)
            .map_err(|e| format!("free[{lo:#x}]({e:?})"))?;
        Ok(b)
    }

    /// Carve `[va, va+len)` out of the fence that holds it as a block of `kind`.
    fn carve(&mut self, va: u64, len: u64, kind: Kind) -> Result<(), String> {
        let (&flo, &f) = self
            .blocks
            .range(..=va)
            .next_back()
            .ok_or("no_fence_below")?;
        if f.kind != Kind::Fence || f.hi < va + len {
            return Err(format!("not_in_fence({:?})", f.kind));
        }
        self.release(flo)?;
        let t = Instant::now();
        if flo < va {
            self.reserve(flo, va, Kind::Fence)?;
        }
        if va + len < f.hi {
            self.reserve(va + len, f.hi, Kind::Fence)?;
        }
        self.reserve(va, va + len, kind)?;
        self.window.push(t);
        Ok(())
    }

    /// Return the block at `va` to fence and merge it with fence neighbours (eager merge).
    fn uncarve(&mut self, va: u64) -> Result<(), String> {
        let b = self.release(va)?;
        let t = Instant::now();
        let mut lo = va;
        let mut hi = b.hi;
        if let Some((&llo, l)) = self.blocks.range(..va).next_back()
            && l.kind == Kind::Fence
            && l.hi == va
        {
            self.release(llo)?;
            lo = llo;
        }
        if let Some(r) = self.blocks.get(&hi).copied()
            && r.kind == Kind::Fence
        {
            self.release(hi)?;
            hi = r.hi;
        }
        self.reserve(lo, hi, Kind::Fence)?;
        self.window.push(t);
        Ok(())
    }

    fn handle_at(&self, va: u64) -> Option<u32> {
        self.blocks.get(&va).map(|b| b.h)
    }

    fn free_all(&mut self) {
        for (_, b) in std::mem::take(&mut self.blocks) {
            let _ = self.rm.free(b.h);
        }
    }
}

/// One tiled run: map N rows then unmap them. Returns the line's body.
fn tiled_run(t: &mut Tiling<'_>, mem: u32, lo: u64, stop_on: &AtomicBool) -> String {
    let (mut mp, mut um) = (Stat::default(), Stat::default());
    let mut err = String::new();
    let mut mapped = 0;
    let c0 = t.calls;
    for i in 0..N {
        if stop_on.load(Ordering::Relaxed) {
            break;
        }
        let va = slot_va(lo, i);
        let tm = Instant::now();
        let r = t.carve(va, PAGE, Kind::Row).and_then(|()| {
            t.calls += 1;
            let h = t.handle_at(va).ok_or("no_row")?;
            map_at(t.rm, h, mem, va)
                .map(|_| ())
                .map_err(|e| format!("map({e:?})"))
        });
        if let Err(e) = r {
            err = format!(" map_err_at={i}:{e}");
            break;
        }
        mp.push(tm);
        mapped += 1;
    }
    let calls_map = t.calls - c0;
    let c1 = t.calls;
    for i in 0..mapped {
        let va = slot_va(lo, i);
        let tm = Instant::now();
        t.calls += 1;
        let r = t
            .handle_at(va)
            .ok_or_else(|| "no_row".to_string())
            .and_then(|h| {
                t.rm.unmap_in(h, va, 0, true)
                    .map_err(|e| format!("unmap({e:?})"))
            })
            .and_then(|()| t.uncarve(va));
        if let Err(e) = r {
            err.push_str(&format!(" unmap_err_at={i}:{e}"));
            break;
        }
        um.push(tm);
    }
    let calls_unmap = t.calls - c1;
    let per = |c: u64| {
        if mapped == 0 {
            0.0
        } else {
            c as f64 / mapped as f64
        }
    };
    format!(
        "mapped={mapped} calls_per_map={:.2} calls_per_unmap={:.2} peak_blocks={} map[{}] unmap[{}] window[{}]{err}",
        per(calls_map),
        per(calls_unmap),
        t.peak,
        mp.show(),
        um.show(),
        t.window.show()
    )
}

fn m1(rm: &HostRm, mem: u32, lo: u64, m0_map: Option<f64>) {
    let Some(s) = space(rm, "M1") else { return };
    let mut t = match Tiling::new(rm, s.space, lo, lo + SPAN) {
        Ok(t) => t,
        Err(e) => {
            println!("TILING arm=M1 {e}");
            rm.free_vaspace(s);
            return;
        }
    };
    let never = AtomicBool::new(false);
    let mean_before = Instant::now();
    let body = tiled_run(&mut t, mem, lo, &never);
    let _ = rm.invalidate_tlb(s);
    let ratio = m0_map.map_or(String::new(), |m0| {
        let m1 = mean_before.elapsed().as_secs_f64() * 1e6 / (2 * N) as f64;
        format!(" mean_us_per_op_incl_unmap={m1:.1} m0_map_mean_us={m0:.1}")
    });
    println!("TILING arm=M1 {body}{ratio}");
    t.free_all();
    rm.free_vaspace(s);
}

fn timed<T>(f: impl FnOnce() -> Result<T, kf_host::RmError>, st: &mut Stat) -> Result<T, String> {
    let t = Instant::now();
    let r = f().map_err(|e| format!("{e:?}"));
    st.push(t);
    r
}

fn m2(rm: &HostRm, mem: u32, lo: u64) {
    let Some(s) = space(rm, "M2") else { return };
    let gap = 2 * PAGE;
    let mut handles: Vec<u32> = Vec::new();
    let mut ceiling = None;
    for &k in &M2_STEPS {
        let mut fill = Stat::default();
        while (handles.len() as u64) < k {
            let j = handles.len() as u64;
            let t = Instant::now();
            match rm.reserve_va(s.space, lo + j * gap, PAGE) {
                Ok(h) => {
                    handles.push(h);
                    fill.push(t);
                }
                Err(e) => {
                    ceiling = Some(format!("K={j} err={e:?}"));
                    break;
                }
            }
        }
        if ceiling.is_some() {
            break;
        }
        let mut line = format!("TILING arm=M2 K={k} fill_last_step[{}]", fill.show());
        for (pos, j) in [("low", 0), ("mid", k / 2), ("high", k - 1)] {
            let (mut rs, mut fr, mut mp, mut um) = (
                Stat::default(),
                Stat::default(),
                Stat::default(),
                Stat::default(),
            );
            let mut err = String::new();
            let gva = lo + j * gap + PAGE;
            let bva = lo + j * gap;
            let bh = handles[usize::try_from(j).unwrap_or(0)];
            for _ in 0..M2_REPS {
                let r = timed(|| rm.reserve_va(s.space, gva, PAGE), &mut rs)
                    .and_then(|h| timed(|| rm.free(h), &mut fr))
                    .and_then(|()| timed(|| map_at(rm, bh, mem, bva), &mut mp))
                    .and_then(|_| timed(|| rm.unmap_in(bh, bva, 0, true), &mut um));
                if let Err(e) = r {
                    err = format!(" err={e}");
                    break;
                }
            }
            line.push_str(&format!(
                " {pos}:resv[{}] free[{}] map[{}] unmap[{}]{err}",
                rs.show(),
                fr.show(),
                mp.show(),
                um.show()
            ));
        }
        println!("{line}");
    }
    let _ = rm.invalidate_tlb(s);
    println!(
        "TILING arm=M2 ceiling={} blocks_held={}",
        ceiling.unwrap_or_else(|| "none_up_to_64000".into()),
        handles.len()
    );
    let t = Instant::now();
    for h in handles.drain(..) {
        let _ = rm.free(h);
    }
    println!("TILING arm=M2 teardown_ms={}", t.elapsed().as_millis());
    rm.free_vaspace(s);
}

fn m3(rm: &HostRm, mem: u32, lo: u64) {
    let Some(s) = space(rm, "M3") else { return };
    let mem_b = match rm.alloc_device_local(PAGE) {
        Ok(m) => m,
        Err(e) => {
            println!("TILING arm=M3 mem_b=ERR({e:?})");
            rm.free_vaspace(s);
            return;
        }
    };
    let mut t = match Tiling::new(rm, s.space, lo, lo + SPAN) {
        Ok(t) => t,
        Err(e) => {
            println!("TILING arm=M3 {e}");
            rm.free_vaspace(s);
            return;
        }
    };
    let stop = AtomicBool::new(false);
    let b_maps = AtomicU64::new(0);
    let b_inside = AtomicU64::new(0);
    let b_err = AtomicU64::new(0);
    let first_inside = AtomicU64::new(0);
    let body = std::thread::scope(|sc| {
        sc.spawn(|| {
            while !stop.load(Ordering::Relaxed) {
                match rm.map(s, mem_b, MapBacking::Dedicated, 0, PAGE, None, false) {
                    Ok(va) => {
                        b_maps.fetch_add(1, Ordering::Relaxed);
                        if va >= lo && va < lo + SPAN {
                            b_inside.fetch_add(1, Ordering::Relaxed);
                            let _ = first_inside.compare_exchange(
                                0,
                                va,
                                Ordering::Relaxed,
                                Ordering::Relaxed,
                            );
                        }
                        let _ = rm.unmap(s, va, false);
                    }
                    Err(_) => {
                        b_err.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });
        let never = AtomicBool::new(false);
        let body = tiled_run(&mut t, mem, lo, &never);
        stop.store(true, Ordering::Relaxed);
        body
    });
    println!(
        "TILING arm=M3 {body} b_maps={} b_inside_range={} b_first_inside={:#x} b_errs={}",
        b_maps.load(Ordering::Relaxed),
        b_inside.load(Ordering::Relaxed),
        first_inside.load(Ordering::Relaxed),
        b_err.load(Ordering::Relaxed)
    );
    t.free_all();
    let _ = rm.free(mem_b);
    rm.free_vaspace(s);
}

fn m4(rm: &HostRm, mem: u32, lo: u64) {
    let Some(s) = space(rm, "M4") else { return };
    let mut t = match Tiling::new(rm, s.space, lo, lo + SPAN) {
        Ok(t) => t,
        Err(e) => {
            println!("TILING arm=M4 {e}");
            rm.free_vaspace(s);
            return;
        }
    };
    let base = lo + (1 << 30);
    let mut out = String::from("TILING arm=M4");
    let mut st = Stat::default();
    let mut step =
        |name: &str, t: &mut Tiling<'_>, r: &dyn Fn(&mut Tiling<'_>) -> Result<(), String>| {
            let tm = Instant::now();
            let res = r(t);
            st.push(tm);
            match res {
                Ok(()) => out.push_str(&format!(" {name}=OK")),
                Err(e) => out.push_str(&format!(" {name}=ERR({e})")),
            }
        };
    // a 64 KiB sparse block between two fences
    step("sparse64k", &mut t, &|t| t.carve(base, PAGE, Kind::Sparse));
    // an adjacent second sparse block (sparse next to sparse)
    step("sparse64k_adjacent", &mut t, &|t| {
        t.carve(base + PAGE, PAGE, Kind::Sparse)
    });
    // a row block next to the sparse pair, mapped
    step("row_next_to_sparse", &mut t, &|t| {
        t.carve(base + 2 * PAGE, PAGE, Kind::Row)?;
        let h = t.handle_at(base + 2 * PAGE).ok_or("no_row")?;
        map_at(t.rm, h, mem, base + 2 * PAGE)
            .map(|_| ())
            .map_err(|e| format!("map({e:?})"))
    });
    // map THROUGH a sparse block and unmap (RM restores the sparse PTE)
    step("map_in_sparse", &mut t, &|t| {
        let h = t.handle_at(base).ok_or("no_sparse")?;
        map_at(t.rm, h, mem, base)
            .map(|_| ())
            .map_err(|e| format!("map({e:?})"))
    });
    step("unmap_in_sparse", &mut t, &|t| {
        let h = t.handle_at(base).ok_or("no_sparse")?;
        t.rm.unmap_in(h, base, 0, true)
            .map_err(|e| format!("unmap({e:?})"))
    });
    // a 4 KiB sparse block (finer than the 64 KiB big page)
    step("sparse4k", &mut t, &|t| {
        t.carve(base + (1 << 21), 0x1000, Kind::Sparse)
    });
    // a 2 MiB sparse block, 2 MiB aligned
    step("sparse2m", &mut t, &|t| {
        t.carve(base + (8 << 20), 2 << 20, Kind::Sparse)
    });
    // retire a sparse block back into the fence (free + merge)
    step("unsparse64k", &mut t, &|t| t.uncarve(base + PAGE));
    let _ = rm.invalidate_tlb(s);
    println!("{out} steps[{}] blocks={}", st.show(), t.blocks.len());
    t.free_all();
    rm.free_vaspace(s);
}
