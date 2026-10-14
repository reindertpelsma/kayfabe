//! ★ 2026-10-14 — **SYNC_WAIT / ring stress for the USER-ring relay** (`docs/design/V3_USER_RING_EVENT_LOOP_20261014.md`
//! §8). A raw RM client that creates its OWN channels and authors their GPFIFO, USERD and
//! pushbuffer contents, then checks that the GPU executed EXACTLY what was written. Runs natively
//! (the bare-metal control: no relay) and in a Linux guest under `KF3_DEBUG=1
//! KF3_DEBUG_FORCE_MIGRATABLE=1` (the same channels take the migratable USER-ring relay).
//!
//! Only host methods are used (`SEM_ADDR_*`/`SEM_PAYLOAD_*`/`SEM_EXECUTE`: release, reduction-add,
//! acquire), so no engine object is needed and the expected memory is exact:
//!  * each UNIT `k` releases `payload(k)` at `slot(k).value` and adds 1 at `slot(k).count`: a dropped
//!    unit leaves the slot 0, a REPLAYED one leaves `count == 2`, a mis-ordered one is caught by the
//!    acquire chain of the late cases;
//!  * each cut/SYNC_WAIT/`LENGTH == 0` placement is a different split of the SAME word stream, so the
//!    memory result must not depend on it.
//!
//! Families (all deterministic from `--seed`):
//!  `split`  every single cut position of a 3-unit stream x SYNC_WAIT on/off for the second segment
//!  `random` random cuts / SYNC_WAIT masks / `LENGTH == 0` control entries (first, middle, last,
//!           back to back) / doorbell mode (per entry, one at the end, PUT ahead of the doorbell,
//!           skipped doorbells)
//!  `late`   pushbuffers written LATE: a segment ends with an acquire on a gate semaphore, the next
//!           entry carries SYNC_WAIT and its bytes are POISON (an illegal method header) until the CPU
//!           writes them and then opens the gate -- the hardware fetches them only after the gate, and
//!           a relay that reads them earlier refuses the poison and kills the channel
//!  `wrap`   many rounds on a small ring (the cursor laps it; every entry is a SYNC_WAIT candidate)
//!  `storm`  one PUT, a burst of doorbells
//!  `conc`   three channels at once
//!
//! Output: one line per family (`STRESS family=... pass=N fail=M ...` with round latency p50/p99/max
//! in microseconds) and a final `STRESS_VERDICT=PASS|FAIL`. Exit 0 only on PASS. Time-based waits are
//! STIMULI (letting the relay run before the late bytes are written) or bounded failure detection --
//! never what a result depends on.
#![allow(clippy::too_many_lines)]

use kf_abi::submit::{
    ENGINE_TYPE_GRAPHICS, USERD_GP_GET, USERD_GP_PUT, USERD_REFERENCE_C56F, fifo, gp_entry, method_header_inc,
};
use kf_host::{Channel, HostRm, MapNode, MapPerm, RingSpec, VaSpace, ViewAccess};
use kf_linux_raw::{Backing, CachePolicy, DevDir, HostOffset, HostPageSize, VolatileRegion};
use std::time::{Duration, Instant};

const BYTES: u64 = 64 << 10;
/// GPFIFO inside the ring buffer.
const GP_OFF: u64 = 0x1000;
/// Pushbuffer area inside the ring buffer.
const PB_OFF: u64 = 0x4000;
const PB_WORDS: usize = ((BYTES - PB_OFF) / 4) as usize;
/// Semaphore pages inside the output buffer: unit slots (16 bytes: value, count), gates (4 bytes).
const SLOT_OFF: u64 = 0x1000;
const GATE_OFF: u64 = 0x100;
const MAX_UNITS: usize = 200;
/// An illegal method header (SEC_OP RESERVED): the poison a late segment holds before its bytes exist.
const POISON: u32 = 0xC000_1234;

fn hdr(method: u32, count: u32) -> Result<u32, String> {
    method_header_inc(0, method, count).ok_or_else(|| format!("method header {method:#x}x{count}"))
}

struct Buffer {
    memory: u32,
    _node: kf_linux_raw::CharDevice,
    cookie: u64,
    cpu: VolatileRegion,
    va: Option<u64>,
}

impl Buffer {
    fn new(rm: &HostRm, space: VaSpace, perm: Option<MapPerm>) -> Result<Self, String> {
        let memory = rm.alloc_device_local(BYTES).map_err(|e| format!("memory: {e:?}"))?;
        let va = perm
            .map(|p| rm.map_window_perm(space, memory, BYTES, false, p))
            .transpose()
            .map_err(|e| format!("map: {e:?}"))?;
        if va.is_some_and(|v| v.checked_add(BYTES).is_none_or(|e| e > 1 << 40)) {
            return Err("mapping exceeds the 40-bit GP/semaphore address width".into());
        }
        let (node, cookie) = rm
            .arm_cpu_view(MapNode::Gpu, memory, 0, BYTES, ViewAccess::ReadWrite)
            .map_err(|e| format!("view: {e:?}"))?;
        let cpu = VolatileRegion::map(
            Backing::DeviceFile { fd: node.as_fd() },
            BYTES,
            CachePolicy::Uncached,
            HostPageSize::query(),
        )
        .map_err(|e| format!("mmap: {e:?}"))?;
        let b = Self { memory, _node: node, cookie, cpu, va };
        for off in (0..BYTES).step_by(4) {
            b.store(off, 0)?;
        }
        Ok(b)
    }
    fn store(&self, off: u64, value: u32) -> Result<(), String> {
        self.cpu.store_u32(HostOffset::new(off), value).map_err(|e| format!("store: {e:?}"))
    }
    fn load(&self, off: u64) -> Result<u32, String> {
        self.cpu.load_u32(HostOffset::new(off)).map_err(|e| format!("load: {e:?}"))
    }
    fn close(self, rm: &HostRm, space: VaSpace) {
        drop(self.cpu);
        let _ = rm.release_cpu_view(kf_host::CpuViewRelease { h_memory: self.memory, p_linear_address: self.cookie });
        if let Some(va) = self.va {
            let _ = rm.unmap(space, va, false);
        }
        let _ = rm.free(self.memory);
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn coin(&mut self, per_cent: u64) -> bool {
        self.below(100) < per_cent
    }
}

// ---------------------------------------------------------------------------------------------
// the word stream

fn sem_release(w: &mut Vec<u32>, va: u64, value: u32) -> Result<(), String> {
    w.push(hdr(fifo::SEM_ADDR_LO, 4)?);
    w.extend_from_slice(&[va as u32, (va >> 32) as u32 & 0xff, value, 0]);
    w.push(hdr(fifo::SEM_EXECUTE, 1)?);
    w.push(fifo::SEM_EXECUTE_RELEASE_32BIT | fifo::SEM_EXECUTE_RELEASE_WFI_EN);
    Ok(())
}
/// `NVC56F_SEM_EXECUTE`: OPERATION_REDUCTION (6), REDUCTION_IADD (5) at 30:27, UNSIGNED (31).
const SEM_REDUCE_ADD_U32: u32 = 6 | (5 << 27) | (1 << 31);
fn sem_reduce_add(w: &mut Vec<u32>, va: u64, value: u32) -> Result<(), String> {
    w.push(hdr(fifo::SEM_ADDR_LO, 4)?);
    w.extend_from_slice(&[va as u32, (va >> 32) as u32 & 0xff, value, 0]);
    w.push(hdr(fifo::SEM_EXECUTE, 1)?);
    w.push(SEM_REDUCE_ADD_U32);
    Ok(())
}
fn sem_acquire(w: &mut Vec<u32>, va: u64, value: u32) -> Result<(), String> {
    w.push(hdr(fifo::SEM_ADDR_LO, 5)?);
    w.extend_from_slice(&[va as u32, (va >> 32) as u32 & 0xff, value, 0, fifo::SEM_EXECUTE_ACQUIRE_32BIT]);
    Ok(())
}

/// One entry of a plan's ring: a slice of the word stream, or a `LENGTH == 0` control entry.
#[derive(Clone, Debug)]
enum Ent {
    Seg { from: usize, to: usize, sync: bool },
    Ctl { sync: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Doorbell {
    /// PUT and doorbell after every entry.
    Each,
    /// One PUT and one doorbell at the end.
    End,
    /// PUT after every entry, ONE doorbell at the end (PUT runs ahead of the doorbell).
    PutAhead,
    /// PUT after every entry, a doorbell every third entry and at the end.
    Skipped,
}

/// A late gate: the stream words `[from, to)` are poison until the gate opens; the gate semaphore
/// is acquired by the segment ending right before `from`.
#[derive(Clone, Debug)]
struct Late {
    from: usize,
    to: usize,
    gate: u32,
    /// Index into `ents` of the entry that ends with the acquire.
    before_ent: usize,
}

#[derive(Clone, Debug)]
struct Plan {
    name: String,
    words: Vec<u32>,
    units: usize,
    ents: Vec<Ent>,
    mode: Doorbell,
    late: Vec<Late>,
    /// Stimulus: milliseconds to let the relay run before the late bytes are written.
    settle_ms: u64,
}

struct Unit {
    slot: usize,
    payload: u32,
}
fn units_words(out_va: u64, units: &[Unit], w: &mut Vec<u32>) -> Result<(), String> {
    for u in units {
        let va = out_va + SLOT_OFF + 16 * u.slot as u64;
        sem_release(w, va, u.payload)?;
        sem_reduce_add(w, va + 4, 1)?;
    }
    Ok(())
}

fn cut_into(len: usize, cuts: &[usize]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    for &c in cuts {
        if c > from && c < len {
            out.push((from, c));
            from = c;
        }
    }
    out.push((from, len));
    out
}

fn random_cuts(r: &mut Rng, len: usize, n: usize) -> Vec<usize> {
    let mut v: Vec<usize> = (0..n).map(|_| 1 + r.below(len as u64 - 1) as usize).collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// `max_units`, `max_cuts`, `max_ents` bound the plan (a small ring cannot hold a big plan).
fn plan_random(
    name: String,
    r: &mut Rng,
    out_va: u64,
    mode: Option<Doorbell>,
    (max_units, max_cuts, max_ents): (u64, u64, usize),
) -> Result<Plan, String> {
    let n = 1 + r.below(max_units) as usize;
    let units: Vec<Unit> = (0..n).map(|k| Unit { slot: k, payload: 0x1000 + k as u32 }).collect();
    let mut words = Vec::new();
    units_words(out_va, &units, &mut words)?;
    let ncut = r.below(max_cuts + 1) as usize;
    let cuts = random_cuts(r, words.len(), ncut);
    let mut ents = Vec::new();
    let ctl = |r: &mut Rng, ents: &mut Vec<Ent>| {
        while r.coin(30) {
            ents.push(Ent::Ctl { sync: r.coin(50) });
        }
    };
    ctl(r, &mut ents); // first
    for (from, to) in cut_into(words.len(), &cuts) {
        ents.push(Ent::Seg { from, to, sync: r.coin(50) });
        ctl(r, &mut ents); // between, back to back, last
    }
    while ents.len() > max_ents {
        let Some(i) = ents.iter().rposition(|e| matches!(e, Ent::Ctl { .. })) else {
            break;
        };
        ents.remove(i);
    }
    let mode = mode.unwrap_or(match r.below(4) {
        0 => Doorbell::Each,
        1 => Doorbell::End,
        2 => Doorbell::PutAhead,
        _ => Doorbell::Skipped,
    });
    Ok(Plan { name, words, units: n, ents, mode, late: Vec::new(), settle_ms: 0 })
}

fn plan_late(name: String, r: &mut Rng, out_va: u64) -> Result<Plan, String> {
    let phases = 2 + r.below(3) as usize;
    let mut words = Vec::new();
    let mut ents = Vec::new();
    let mut late = Vec::new();
    let mut slot = 0usize;
    for p in 0..phases {
        let n = 1 + r.below(3) as usize;
        let units: Vec<Unit> = (0..n)
            .map(|_| {
                let u = Unit { slot, payload: 0x2000 + slot as u32 };
                slot += 1;
                u
            })
            .collect();
        let phase_from = words.len();
        units_words(out_va, &units, &mut words)?;
        // the phase ends with the acquire of its OWN gate, except the last one
        let last = p + 1 == phases;
        if !last {
            let gate_va = out_va + GATE_OFF + 4 * p as u64;
            sem_acquire(&mut words, gate_va, 1)?;
        }
        let phase_to = words.len();
        // extra cuts inside the phase (never between the acquire's words: it is one packet, and the
        // gating needs the acquire packet complete inside the segment before the late one)
        let body_end = if last { phase_to } else { phase_to - 6 };
        let ncut = r.below(3) as usize;
        let cuts: Vec<usize> = random_cuts(r, body_end - phase_from, ncut)
            .into_iter()
            .map(|c| phase_from + c)
            .collect();
        let mut seg_from = phase_from;
        let mut bounds: Vec<usize> = cuts;
        bounds.push(phase_to);
        for to in bounds {
            if to <= seg_from {
                continue;
            }
            // the first segment of a late phase carries SYNC_WAIT (it IS the gated entry); the others random
            let first_of_phase = seg_from == phase_from;
            let sync = if p > 0 && first_of_phase { true } else { r.coin(40) };
            if p > 0 && first_of_phase {
                // optionally a LENGTH==0 SYNC_WAIT control entry right at the boundary
                if r.coin(30) {
                    ents.push(Ent::Ctl { sync: true });
                }
                late.push(Late { from: phase_from, to: usize::MAX, gate: (p - 1) as u32, before_ent: 0 });
            }
            ents.push(Ent::Seg { from: seg_from, to, sync });
            seg_from = to;
        }
    }
    let total = words.len();
    // patch each late range: [phase_from, next phase_from or total)
    let froms: Vec<usize> = late.iter().map(|l| l.from).collect();
    for (i, l) in late.iter_mut().enumerate() {
        l.to = froms.get(i + 1).copied().unwrap_or(total);
    }
    // the first entry of a late phase may not be preceded by a control entry that precedes the
    // acquire-ending segment: `before_ent` is the acquire-ending segment (the entry before the
    // optional control entry), recomputed exactly here
    let mut fixed = Vec::new();
    for l in &late {
        let seg_idx = ents
            .iter()
            .position(|e| matches!(e, Ent::Seg { from, .. } if *from == l.from))
            .ok_or("late phase has no entry")?;
        let mut b = seg_idx - 1;
        if matches!(ents[b], Ent::Ctl { .. }) {
            b -= 1;
        }
        fixed.push(Late { before_ent: b, ..l.clone() });
    }
    let units = slot;
    Ok(Plan {
        name,
        words,
        units,
        ents,
        mode: if r.coin(50) { Doorbell::End } else { Doorbell::PutAhead },
        late: fixed,
        settle_ms: [0, 1, 5][r.below(3) as usize],
    })
}

// ---------------------------------------------------------------------------------------------
// one channel

struct Chan<'a> {
    rm: &'a HostRm,
    space: VaSpace,
    ring: Buffer,
    out: Buffer,
    userd: Buffer,
    chan: Channel,
    entries: u32,
    put: u32,
    ring_va: u64,
    out_va: u64,
    /// Completion poll interval (0 = spin with yield: the `rate` family measures microseconds).
    poll: Duration,
}

impl<'a> Chan<'a> {
    fn new(rm: &'a HostRm, engine: u32, entries: u32) -> Result<Self, String> {
        let space = rm.alloc_vaspace_bare().map_err(|e| format!("space: {e:?}"))?;
        let ring = Buffer::new(rm, space, Some(MapPerm::READ_WRITE))?;
        let out = Buffer::new(rm, space, Some(MapPerm::READ_WRITE))?;
        let userd = Buffer::new(rm, space, None)?;
        let ring_va = ring.va.ok_or("ring VA absent")?;
        let out_va = out.va.ok_or("out VA absent")?;
        let chan = rm
            .birth_channel(
                space,
                engine,
                RingSpec {
                    gp_fifo_va: ring_va + GP_OFF,
                    gp_fifo_entries: entries,
                    userd_memory: userd.memory,
                    userd_offset: 0,
                    err_notifier: 0,
                },
            )
            .map_err(|e| format!("birth: {e:?}"))?;
        chan.assert_user().map_err(str::to_string)?;
        rm.schedule(chan).map_err(|e| format!("schedule: {e:?}"))?;
        Ok(Self { rm, space, ring, out, userd, chan, entries, put: 0, ring_va, out_va, poll: Duration::from_micros(300) })
    }

    fn close(self) {
        let _ = self.rm.free_channel(self.chan);
        for b in [self.ring, self.out, self.userd] {
            b.close(self.rm, self.space);
        }
        self.rm.free_vaspace(self.space);
    }

    fn slot_value(&self, k: usize) -> Result<u32, String> {
        self.out.load(SLOT_OFF + 16 * k as u64)
    }
    fn slot_count(&self, k: usize) -> Result<u32, String> {
        self.out.load(SLOT_OFF + 16 * k as u64 + 4)
    }
    fn gp_get(&self) -> Result<u32, String> {
        self.userd.load(USERD_GP_GET)
    }
    fn publish_put(&self, put: u32) -> Result<(), String> {
        kf_linux_raw::release_fence();
        self.userd.store(USERD_GP_PUT, put)?;
        kf_linux_raw::release_fence();
        Ok(())
    }
    fn ring_db(&self) -> Result<(), String> {
        self.rm.doorbell(self.chan.token).map_err(|e| format!("doorbell: {e:?}"))
    }
    fn write_entry(&self, index: u32, raw: u64) -> Result<(), String> {
        let at = GP_OFF + u64::from(index % self.entries) * 8;
        self.ring.store(at, raw as u32)?;
        self.ring.store(at + 4, (raw >> 32) as u32)
    }

    /// Run a plan on this channel; `Ok(latency_us)` or the reason it failed.
    fn run(&mut self, plan: &Plan, timeout: Duration) -> Result<u64, String> {
        if plan.ents.len() as u32 >= self.entries {
            return Err("plan has more entries than the ring".into());
        }
        if plan.words.len() > PB_WORDS || plan.units > MAX_UNITS {
            return Err("plan too large".into());
        }
        // fresh result memory
        for k in 0..plan.units {
            self.out.store(SLOT_OFF + 16 * k as u64, 0)?;
            self.out.store(SLOT_OFF + 16 * k as u64 + 4, 0)?;
        }
        for g in 0..8 {
            self.out.store(GATE_OFF + 4 * g, 0)?;
        }
        // pushbuffer: real words, POISON in the late ranges
        let is_late = |i: usize| plan.late.iter().any(|l| i >= l.from && i < l.to);
        for (i, &w) in plan.words.iter().enumerate() {
            self.ring.store(PB_OFF + 4 * i as u64, if is_late(i) { POISON } else { w })?;
        }
        // GP entries
        let base = self.put;
        let mut raws = Vec::new();
        for e in &plan.ents {
            raws.push(match *e {
                Ent::Seg { from, to, sync } => {
                    let g = gp_entry(self.ring_va + PB_OFF + 4 * from as u64, 4 * (to - from) as u64)
                        .ok_or("GP encoding")?;
                    g | (u64::from(sync) << 63)
                }
                Ent::Ctl { sync } => u64::from(sync) << 63,
            });
        }
        let t0 = Instant::now();
        match plan.mode {
            Doorbell::End => {
                for (i, &raw) in raws.iter().enumerate() {
                    self.write_entry(base + i as u32, raw)?;
                }
                self.put = base + raws.len() as u32;
                self.publish_put(self.put % self.entries)?;
                self.ring_db()?;
            }
            Doorbell::Each | Doorbell::PutAhead | Doorbell::Skipped => {
                for (i, &raw) in raws.iter().enumerate() {
                    self.write_entry(base + i as u32, raw)?;
                    self.put = base + i as u32 + 1;
                    self.publish_put(self.put % self.entries)?;
                    let ring = match plan.mode {
                        Doorbell::Each => true,
                        Doorbell::Skipped => i % 3 == 2,
                        _ => false,
                    };
                    if ring {
                        self.ring_db()?;
                    }
                }
                self.ring_db()?;
            }
        }
        // late bytes: wait for the entries before each gate to be fetched, let the relay run, write
        // the bytes, open the gate
        for l in &plan.late {
            // the entry that ends with the acquire has BEGUN once GP_GET (the next entry to begin) is
            // past it; a following control entry may begin (and wait) too, so "at least"
            let need = l.before_ent as u32 + 1;
            let t = Instant::now();
            while (self.gp_get()? + self.entries - base % self.entries) % self.entries < need
                && t.elapsed() < Duration::from_millis(500)
            {
                std::thread::sleep(Duration::from_micros(200));
            }
            std::thread::sleep(Duration::from_millis(plan.settle_ms));
            for i in l.from..l.to {
                self.ring.store(PB_OFF + 4 * i as u64, plan.words[i])?;
            }
            kf_linux_raw::release_fence();
            self.out.store(GATE_OFF + 4 * u64::from(l.gate), 1)?;
            kf_linux_raw::release_fence();
            // one more doorbell: harmless, and exercises "a doorbell while the gate drains"
            self.ring_db()?;
        }
        // completion: every unit's count, then the ring consumed (GP_GET write-back)
        let deadline = Instant::now() + timeout;
        loop {
            let mut done = true;
            for k in 0..plan.units {
                if self.slot_count(k)? == 0 {
                    done = false;
                    break;
                }
            }
            if done && self.gp_get()? == self.put % self.entries {
                break;
            }
            if Instant::now() > deadline {
                let missing: Vec<usize> = (0..plan.units).filter(|k| self.slot_count(*k).unwrap_or(0) == 0).collect();
                return Err(format!(
                    "TIMEOUT after {timeout:?}: units not executed {missing:?}, GP_GET {} GP_PUT {} (put {})",
                    self.gp_get()?,
                    self.userd.load(USERD_GP_PUT)?,
                    self.put % self.entries
                ));
            }
            if self.poll.is_zero() {
                std::thread::yield_now();
            } else {
                std::thread::sleep(self.poll);
            }
        }
        let latency = t0.elapsed().as_micros() as u64;
        // settle, then exactness: value, count == 1 (no drop, no replay)
        if !self.poll.is_zero() {
            std::thread::sleep(Duration::from_millis(10));
        }
        for k in 0..plan.units {
            let (v, c) = (self.slot_value(k)?, self.slot_count(k)?);
            if v != 0x1000 + k as u32 && v != 0x2000 + k as u32 {
                return Err(format!("unit {k}: value {v:#x} (not its payload)"));
            }
            if c != 1 {
                return Err(format!("unit {k}: executed {c} times"));
            }
        }
        Ok(latency)
    }
}

// ---------------------------------------------------------------------------------------------
// families

#[derive(Default)]
struct Tally {
    pass: u32,
    fail: u32,
    lat: Vec<u64>,
    first: Vec<String>,
}
impl Tally {
    fn record(&mut self, plan: &Plan, r: Result<u64, String>) -> bool {
        match r {
            Ok(l) => {
                self.pass += 1;
                self.lat.push(l);
                true
            }
            Err(why) => {
                self.fail += 1;
                if self.first.len() < 4 {
                    self.first.push(format!(
                        "{}: {why}\n      mode={:?} ents={:?} late={:?} settle_ms={} words={}",
                        plan.name, plan.mode, plan.ents, plan.late, plan.settle_ms, plan.words.len()
                    ));
                }
                false
            }
        }
    }
    fn report(&mut self, family: &str) {
        self.lat.sort_unstable();
        let pick = |q: f64| self.lat.get(((self.lat.len() as f64 - 1.0) * q) as usize).copied().unwrap_or(0);
        println!(
            "STRESS family={family} pass={} fail={} latency_us p50={} p99={} max={}",
            self.pass,
            self.fail,
            pick(0.5),
            pick(0.99),
            self.lat.last().copied().unwrap_or(0)
        );
        for f in &self.first {
            println!("STRESS_FAILURE {family} {f}");
        }
    }
}

/// Run `plans` on a channel; a failed plan replaces the channel (it may be dead or stuck on a gate).
fn run_family<'a>(
    rm: &'a HostRm,
    engine: u32,
    entries: u32,
    plans: impl Iterator<Item = Result<Plan, String>>,
    timeout: Duration,
    t: &mut Tally,
) -> Result<(), String> {
    let mut ch = Chan::new(rm, engine, entries)?;
    for p in plans {
        let p = p?;
        let r = ch.run(&p, timeout);
        let ok = t.record(&p, r);
        if !ok {
            // open every gate so a stuck acquire releases, then start over on a fresh channel
            for g in 0..8 {
                let _ = ch.out.store(GATE_OFF + 4 * g, 1);
            }
            ch.close();
            ch = Chan::new(rm, engine, entries)?;
        }
    }
    ch.close();
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |name: &str, default: &str| -> String {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1).cloned())
            .unwrap_or_else(|| default.to_string())
    };
    let seed: u64 = arg("--seed", "20261014").parse().unwrap_or(20_261_014);
    let scale: u64 = arg("--scale", "1").parse().unwrap_or(1);
    let families = arg("--families", "split,random,late,wrap,storm,conc");
    let timeout = Duration::from_secs(arg("--timeout-s", "20").parse().unwrap_or(20));
    let engine = if arg("--engine", "gr") == "gr" { ENGINE_TYPE_GRAPHICS } else { kf_abi::submit::ENGINE_TYPE_COPY0 };

    let dev = match DevDir::open(c"/dev") {
        Ok(d) => d,
        Err(e) => {
            println!("STRESS_VERDICT=FAIL dev: {e:?}");
            std::process::exit(2);
        }
    };
    let rm = match HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes) {
        Ok(rm) => rm,
        Err(e) => {
            println!("STRESS_VERDICT=FAIL open: {e}");
            std::process::exit(2);
        }
    };
    println!("STRESS_START driver={} seed={seed} scale={scale} families={families} engine={engine:#x}", rm.driver_version());
    // the out VA is per channel; plans need it at authoring time, so plans are generated lazily
    // against a probe channel's layout: every channel maps `out` at the same VA (fresh space, first map)
    let mut total_fail = 0u32;
    let mut rng = Rng(seed);
    let want = |f: &str| families.split([',', '+']).any(|x| x == f);

    // The out VA of a fresh space is deterministic: learn it from a throwaway channel.
    let out_va = match Chan::new(&rm, engine, 64) {
        Ok(c) => {
            let v = c.out_va;
            c.close();
            v
        }
        Err(e) => {
            println!("STRESS_VERDICT=FAIL setup: {e}");
            std::process::exit(2);
        }
    };

    if want("rate") {
        // ★ Batch-rate / latency family (before-after comparison of the relay's drain path): N plans of 4
        // units back to back, SYNC_WAIT on every segment but the first, cut (a) at packet BOUNDARIES (every
        // SYNC_WAIT is gateable) or (b) INSIDE packets (every SYNC_WAIT is behind an open packet: the
        // Minecraft shape). Completion is spin-polled; the tally's latency is ring -> last unit, per plan.
        for (variant, open) in [("boundary", false), ("open-packet", true)] {
            let n = 400 * scale;
            let mut t = Tally::default();
            let r = (|| -> Result<(), String> {
                let mut ch = Chan::new(&rm, engine, 64)?;
                ch.poll = Duration::ZERO;
                let t0 = Instant::now();
                for i in 0..n {
                    let units: Vec<Unit> = (0..4).map(|k| Unit { slot: k, payload: 0x1000 + k as u32 }).collect();
                    let mut words = Vec::new();
                    units_words(ch.out_va, &units, &mut words)?;
                    // each unit is 14 words: boundary cuts at 14/28/42; open cuts 3 words into the next unit
                    let cuts: Vec<usize> = if open { vec![17, 31, 45] } else { vec![14, 28, 42] };
                    let ents: Vec<Ent> = cut_into(words.len(), &cuts)
                        .into_iter()
                        .enumerate()
                        .map(|(k, (from, to))| Ent::Seg { from, to, sync: k > 0 })
                        .collect();
                    let p = Plan { name: format!("rate-{variant}#{i}"), words, units: 4, ents, mode: Doorbell::End, late: Vec::new(), settle_ms: 0 };
                    let r = ch.run(&p, timeout);
                    t.record(&p, r);
                }
                let secs = t0.elapsed().as_secs_f64();
                println!("STRESS_RATE variant={variant} plans={n} entries_per_plan=4 sync_waits_per_plan=3 plans_per_s={:.0}", n as f64 / secs);
                ch.close();
                Ok(())
            })();
            if let Err(e) = r {
                println!("STRESS_FAMILY_ERROR rate {variant} {e}");
                t.fail += 1;
            }
            t.report(&format!("rate-{variant}"));
            total_fail += t.fail;
        }
    }
    if want("barrier") {
        // ★ HARDWARE MEASUREMENT (no verdict on the relay): what do GP_GET and the executed work look like
        // while an ACQUIRE stalls entry 0, behind (a) a LENGTH==0 SYNC_WAIT control entry then a segment,
        // (b) a segment that itself carries SYNC_WAIT. Shared by the relay's gate proof ("GP_GET past the
        // witness means the barrier's wait finished"): if the segment behind the barrier EXECUTES while the
        // acquire is unsatisfied, that proof is false.
        for variant in ["ctl-then-seg", "sync-seg"] {
            let r = (|| -> Result<(), String> {
                let mut ch = Chan::new(&rm, engine, 64)?;
                let units = [Unit { slot: 0, payload: 0x1000 }];
                let mut after = Vec::new();
                units_words(ch.out_va, &units, &mut after)?;
                // the witness candidate: SET_REFERENCE (a host method that writes USERD.REFERENCE when the
                // PBDMA PROCESSES it, in order)
                after.push(hdr(fifo::SET_REFERENCE_C56F, 1)?);
                after.push(7);
                let mut first = Vec::new();
                sem_acquire(&mut first, ch.out_va + GATE_OFF, 1)?;
                for (i, &w) in first.iter().chain(after.iter()).enumerate() {
                    ch.ring.store(PB_OFF + 4 * i as u64, w)?;
                }
                let g0 = gp_entry(ch.ring_va + PB_OFF, 4 * first.len() as u64).ok_or("gp")?;
                let g1 = gp_entry(ch.ring_va + PB_OFF + 4 * first.len() as u64, 4 * after.len() as u64).ok_or("gp")?;
                ch.write_entry(0, g0)?;
                let n = if variant == "ctl-then-seg" {
                    ch.write_entry(1, 1 << 63)?;
                    ch.write_entry(2, g1)?;
                    3
                } else {
                    ch.write_entry(1, g1 | (1 << 63))?;
                    2
                };
                ch.put = n;
                ch.publish_put(n)?;
                ch.ring_db()?;
                let mut samples = Vec::new();
                for ms in [1u64, 5, 20, 50] {
                    std::thread::sleep(Duration::from_millis(ms));
                    samples.push((ms, ch.gp_get()?, ch.slot_count(0)?, ch.userd.load(USERD_REFERENCE_C56F)?));
                }
                println!(
                    "STRESS_BARRIER variant={variant} entries={n} while_the_acquire_is_stalled: (after_ms, GP_GET, unit_executed, USERD.REFERENCE)={samples:?}"
                );
                ch.out.store(GATE_OFF, 1)?;
                std::thread::sleep(Duration::from_millis(20));
                println!(
                    "STRESS_BARRIER variant={variant} after_the_gate_opened: GP_GET={} unit_count={} USERD.REFERENCE={}",
                    ch.gp_get()?,
                    ch.slot_count(0)?,
                    ch.userd.load(USERD_REFERENCE_C56F)?
                );
                ch.close();
                Ok(())
            })();
            if let Err(e) = r {
                println!("STRESS_FAMILY_ERROR barrier {variant}: {e}");
            }
        }
    }
    if want("split") {
        // every single cut position of a 3-unit stream, SYNC_WAIT on the second segment or not
        let mut base = Vec::new();
        let units: Vec<Unit> = (0..3).map(|k| Unit { slot: k, payload: 0x1000 + k as u32 }).collect();
        units_words(out_va, &units, &mut base).expect("words");
        let mut plans = Vec::new();
        for cut in 1..base.len() {
            for sync in [false, true] {
                plans.push(Ok(Plan {
                    name: format!("split cut={cut} sync={sync}"),
                    words: base.clone(),
                    units: 3,
                    ents: vec![
                        Ent::Seg { from: 0, to: cut, sync: false },
                        Ent::Seg { from: cut, to: base.len(), sync },
                    ],
                    mode: Doorbell::End,
                    late: Vec::new(),
                    settle_ms: 0,
                }));
            }
        }
        let mut t = Tally::default();
        if let Err(e) = run_family(&rm, engine, 64, plans.into_iter(), timeout, &mut t) {
            println!("STRESS_FAMILY_ERROR split {e}");
            t.fail += 1;
        }
        t.report("split");
        total_fail += t.fail;
    }
    if want("random") {
        let n = 150 * scale;
        let plans: Vec<_> = (0..n).map(|i| plan_random(format!("random#{i}"), &mut rng, out_va, None, (8, 5, 40))).collect();
        let mut t = Tally::default();
        if let Err(e) = run_family(&rm, engine, 64, plans.into_iter(), timeout, &mut t) {
            println!("STRESS_FAMILY_ERROR random {e}");
            t.fail += 1;
        }
        t.report("random");
        total_fail += t.fail;
    }
    if want("late") {
        let n = 60 * scale;
        let plans: Vec<_> = (0..n).map(|i| plan_late(format!("late#{i}"), &mut rng, out_va)).collect();
        let mut t = Tally::default();
        if let Err(e) = run_family(&rm, engine, 64, plans.into_iter(), timeout, &mut t) {
            println!("STRESS_FAMILY_ERROR late {e}");
            t.fail += 1;
        }
        t.report("late");
        total_fail += t.fail;
    }
    if want("wrap") {
        // a 8-entry ring lapped many times; every plan 1-4 entries, half with SYNC_WAIT
        let n = 200 * scale;
        let plans: Vec<_> = (0..n)
            .map(|i| plan_random(format!("wrap#{i}"), &mut rng, out_va, Some(Doorbell::End), (3, 2, 6)))
            .collect();
        let mut t = Tally::default();
        if let Err(e) = run_family(&rm, engine, 8, plans.into_iter(), timeout, &mut t) {
            println!("STRESS_FAMILY_ERROR wrap {e}");
            t.fail += 1;
        }
        t.report("wrap");
        total_fail += t.fail;
    }
    if want("storm") {
        let mut t = Tally::default();
        let r = (|| -> Result<(), String> {
            let mut ch = Chan::new(&rm, engine, 64)?;
            for i in 0..30 * scale {
                let mut p = plan_random(format!("storm#{i}"), &mut rng, out_va, Some(Doorbell::End), (6, 3, 20))?;
                p.name = format!("storm#{i}");
                // the burst: 40 extra doorbells right after the plan's own
                let r = ch.run(&p, timeout);
                for _ in 0..40 {
                    ch.ring_db()?;
                }
                t.record(&p, r);
            }
            ch.close();
            Ok(())
        })();
        if let Err(e) = r {
            println!("STRESS_FAMILY_ERROR storm {e}");
            t.fail += 1;
        }
        t.report("storm");
        total_fail += t.fail;
    }
    if want("conc") {
        // three channels, plans interleaved round-robin: each step authors ONE plan on the next channel
        let mut t = Tally::default();
        let r = (|| -> Result<(), String> {
            let mut chans: Vec<Chan> = (0..3).map(|_| Chan::new(&rm, engine, 64)).collect::<Result<_, _>>()?;
            for i in 0..40 * scale {
                for (c, ch) in chans.iter_mut().enumerate() {
                    let p = if c == 2 {
                        plan_late(format!("conc#{i}.{c}"), &mut rng, ch.out_va)?
                    } else {
                        plan_random(format!("conc#{i}.{c}"), &mut rng, ch.out_va, None, (6, 3, 30))?
                    };
                    let r = ch.run(&p, timeout);
                    t.record(&p, r);
                }
            }
            for ch in chans {
                ch.close();
            }
            Ok(())
        })();
        if let Err(e) = r {
            println!("STRESS_FAMILY_ERROR conc {e}");
            t.fail += 1;
        }
        t.report("conc");
        total_fail += t.fail;
    }
    if total_fail == 0 {
        println!("STRESS_VERDICT=PASS");
    } else {
        println!("STRESS_VERDICT=FAIL failures={total_fail}");
        std::process::exit(1);
    }
}
