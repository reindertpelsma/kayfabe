//! ★★★ **v3 gate 4 — doorbells drive WORKERS; several Translated channels at once.**
//!
//! Gate 3 pumped one channel from one thread. This is the plane v3 actually runs:
//! - **vCPU threads** ring tokens through `kf_trap::TrapPath` exactly as the MMIO trap will — stamp
//!   + RUNG in one CAS, bit then summary, bump, and an eventfd write only when a worker is parked;
//! - **worker threads** (`kf_chan::worker`) scan, claim, pump, release, and park on the wake word;
//! - **host completions are internal rings** of the channel's own token, so a channel's pump is
//!   serialized by its token state — the gate asserts it never contends;
//! - the guest's channels live in **guest RAM** (GPFIFO, pushbuffer, USERD, semaphore behind
//!   SYSMEM PTEs), so the ledger's sysmem rows and the RAM read path are exercised;
//! - one channel splits at a `MEM_OP` (a walk on a worker thread, CUDA context bound there);
//! - a hostile thread rings an UNKNOWN token and the userspace-mappable arm in a tight loop: it must
//!   produce no action at all.

use kf_abi::submit::{SET_OBJECT, USERD_GP_GET, USERD_GP_PUT, ce, gp_entry, method_header_inc};
use kf_chan::host::{GuestUserd, HostRing, Publisher, TranslatedChannel};
use kf_chan::ring::{GuestMemory, TranslatedRing};
use kf_chan::translated::{Target, Window};
use kf_chan::completions::Completions;
use kf_chan::worker::{COMPLETIONS_TAG, WORKER_EFD_TAG, WorkerStats};
use kf_core::{HostOps, HostSlice, Owner, Plane, Step, Translatable, VmCaps, Vmm};
use kf_cuda::abi::kf_format_ver2;
use kf_cuda::walk::{WalkCfg, WalkKernel};
use kf_harness::Ledger as Checks;
use kf_harness::tables::Tree;
use kf_host::{HostRm, VaSpace};
use kf_linux_raw::{Backing, CachePolicy, DevDir, HostOffset as At, HostPageSize, Notifier, Poller, VolatileRegion};
use kf_mem::ledger::{Ledger, desired_from_leaves, plan_reconcile};
use kf_trap::{Action, Class, Route};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const STORE_BYTES: u64 = 256 << 20;
const PT_BASE: u64 = 0x0100_0000;
const PT_BYTES: usize = 4 << 20;
const RAM_BYTES: u64 = 64 << 20;
const CHANNELS: u32 = 3;
const ENTRIES: u32 = 64;
const PER_CHANNEL: u32 = 24;
const SPLIT_AT: u32 = 12;
/// Per channel, in guest RAM: GPFIFO, USERD, semaphore, segments.
const R_CH: u64 = 0x100_0000;
const CH_STRIDE: u64 = 0x4_0000;
const GPFIFO: u64 = 0x0;
const USERD: u64 = 0x1000;
const SEM: u64 = 0x2000;
const SEGS: u64 = 0x1_0000;
const SEG_STRIDE: u64 = 0x200;
/// FB data.
const SRC: u64 = 0x0400_0000;
const DST: u64 = 0x0800_0000;
const COPY: u64 = 0x1000;
const VA_K: u64 = 0x20_0000_0000;
const TOKEN_TABLE: usize = 64;
const HOSTILE_TOKEN: u32 = 40;
const WORKERS: usize = 2;

fn main() {
    let mut l = Checks::default();
    if let Err(e) = run(&mut l) {
        l.check("run", false, e);
    }
    let v = l.verdict();
    println!("GATE4_VERDICT={}", if v { "PASS" } else { "FAIL" });
    std::process::exit(if v { 0 } else { 1 });
}

fn m(sub: u32, method: u32, args: &[u32]) -> Vec<u32> {
    let mut v = vec![method_header_inc(sub, method, args.len() as u32).expect("header")];
    v.extend_from_slice(args);
    v
}
fn hi(a: u64) -> u32 {
    (a >> 32) as u32
}
fn lo(a: u64) -> u32 {
    (a & 0xFFFF_FFFF) as u32
}
fn src(c: u32) -> u64 {
    SRC + u64::from(c) * 0x10_0000
}
fn dst(c: u32, k: u32) -> u64 {
    DST + u64::from(c) * 0x10_0000 + u64::from(k) * COPY
}
fn ch(c: u32) -> u64 {
    R_CH + u64::from(c) * CH_STRIDE
}
fn va(c: u32) -> u64 {
    VA_K + u64::from(c) * CH_STRIDE
}
fn pattern(seed: u32) -> Vec<u8> {
    (0..(COPY / 4) as u32).flat_map(|i| (seed ^ i).to_le_bytes()).collect()
}

/// The walk kernel + our ledger: what a split touches, behind one lock (a worker-side lock, never a
/// vCPU's; in the product it is the VA manager thread's).
struct Mm {
    walk: WalkKernel,
    dptr: u64,
    ledger: Ledger,
    walks: Vec<Option<u64>>,
}

struct Shared<'a> {
    rm: &'a HostRm,
    ram: &'a VolatileRegion,
    mm: &'a Mutex<Mm>,
    space: VaSpace,
    store: u32,
    ram_desc: u32,
    root: u64,
}

impl Shared<'_> {
    fn publish(&self, tag: &str) -> Result<(usize, usize), String> {
        let mut g = self.mm.lock().map_err(|_| "mm poisoned")?;
        g.walk.make_current().map_err(|e| e.to_string())?;
        let dptr = g.dptr;
        let r = g.walk.refresh(dptr, STORE_BYTES, &[self.root]).map_err(|e| e.to_string())?;
        r.validate().map_err(|e| format!("{tag}: report {e}"))?;
        let layout = |gpa: u64, len: u64| gpa.checked_add(len).filter(|&e| e <= RAM_BYTES).map(|_| gpa);
        let desired = desired_from_leaves(r.runs.iter().map(|m| (m.va, m.gpga, m.len, m.aperture())), STORE_BYTES, &layout)
            .map_err(|e| format!("{tag}: {e:?}"))?;
        let plan = plan_reconcile(&g.ledger.rows(), &desired);
        let a = g.ledger.apply(self.rm, self.space, self.store, Some(self.ram_desc), &plan);
        if a.refused > 0 {
            return Err(format!("{tag}: {} refused, first {:?}", a.refused, a.first_refusal));
        }
        Ok((a.mapped, a.unmapped))
    }
}

struct Io<'a, 'b> {
    s: &'a Shared<'b>,
    c: u32,
}
impl GuestMemory for Io<'_, '_> {
    fn read(&mut self, at: u64, out: &mut [u8]) -> Result<(), String> {
        let (ram, off) = {
            let g = self.s.mm.lock().map_err(|_| "mm poisoned")?;
            g.ledger.resolve(at, out.len() as u64).ok_or(format!("{at:#x} not mapped by us"))?
        };
        if !ram {
            let g = self.s.mm.lock().map_err(|_| "mm poisoned")?;
            g.walk.make_current().map_err(|e| e.to_string())?;
            return g.walk.read_at(g.dptr + off, out).map_err(|e| e.to_string());
        }
        if off % 4 != 0 || out.len() % 4 != 0 {
            return Err(format!("unaligned guest-RAM read {off:#x}+{}", out.len()));
        }
        for (i, chunk) in out.chunks_exact_mut(4).enumerate() {
            let w = self.s.ram.load_u32(At::new(off + 4 * i as u64)).map_err(|e| format!("{e:?}"))?;
            chunk.copy_from_slice(&w.to_le_bytes());
        }
        Ok(())
    }
}
impl GuestUserd for Io<'_, '_> {
    fn gp_put(&mut self) -> Result<u32, String> {
        self.s.ram.load_u32(At::new(ch(self.c) + USERD + USERD_GP_PUT)).map_err(|e| format!("{e:?}"))
    }
    fn set_gp_get(&mut self, v: u32) -> Result<(), String> {
        self.s.ram.store_u32(At::new(ch(self.c) + USERD + USERD_GP_GET), v).map_err(|e| format!("{e:?}"))
    }
}
impl Publisher for Io<'_, '_> {
    fn invalidated(&mut self, pdb: Option<u64>) -> Result<(), String> {
        self.s.mm.lock().map_err(|_| "mm poisoned")?.walks.push(pdb);
        if pdb.is_some_and(|p| p != self.s.root) {
            return Err(format!("invalidate names {pdb:x?}, not the kernel root"));
        }
        self.s.publish("at_split").map(|_| ())
    }
}

struct Windows {
    fb: u64,
    ram: u64,
}
impl Window for Windows {
    fn translate(&self, t: Target, phys: u64, len: u64) -> Option<u64> {
        let end = phys.checked_add(len)?;
        match t {
            Target::LocalFb if end <= STORE_BYTES => Some(self.fb + phys),
            Target::CoherentSysmem | Target::NonCoherentSysmem if end <= RAM_BYTES => Some(self.ram + phys),
            _ => None,
        }
    }
}

fn is_ce(c: u32) -> bool {
    matches!(c, 0xc6b5 | 0xc7b5 | 0xc8b5 | 0xc9b5 | 0xcab5)
}

struct Channels<'a, 'b> {
    s: &'a Shared<'b>,
    done: &'a Completions,
    win: &'a Windows,
    /// `(host token, channel)` — the plane names a channel by its HOST token.
    chans: Vec<(u32, Mutex<(TranslatedChannel, Option<String>)>)>,
    contended: AtomicU64,
    /// §8 says a Translated channel is NEVER forged; any forge is a defect this gate counts.
    forged: AtomicU64,
    poisoned: AtomicU64,
}

impl Channels<'_, '_> {
    fn slot(&self, host_token: u32) -> Option<(u32, &Mutex<(TranslatedChannel, Option<String>)>)> {
        self.chans.iter().enumerate().find(|(_, (h, _))| *h == host_token).map(|(i, (_, m))| (i as u32, m))
    }
}

/// ★ The harness's `HostOps`: the plane decides (claim, submission, §8 completion rule); this only
/// runs the Translated pump for the channel the plane names.
impl HostOps for Channels<'_, '_> {
    fn ring_host(&self, _host_token: u32) {}
    fn run_translated(&self, host_token: u32, _up_to_seq: u64) -> bool {
        self.serve(host_token);
        true
    }
    fn run_emulated(&self, _host_token: u32, _up_to_seq: u64) {}
    fn apply_register(&self, _bar: u8, _offset: u32, _value: u64, _width: u8) {}
    fn operands_translatable(&self, host_token: u32, _up_to_seq: u64) -> Translatable {
        // A channel whose pump REFUSED is dead: the plane then poisons (kernel) or faults (user).
        match self.slot(host_token) {
            Some((_, m)) if m.try_lock().is_ok_and(|g| g.1.is_some()) => Translatable::No,
            Some(_) => Translatable::Yes,
            None => Translatable::No,
        }
    }
    fn forge_completion(&self, _host_token: u32) {
        self.forged.fetch_add(1, Ordering::Relaxed);
    }
    fn refuse_and_poison(&self, _host_token: u32) {
        self.poisoned.fetch_add(1, Ordering::Relaxed);
    }
    fn fault_channel(&self, _host_token: u32) {}
    fn map_guest_slice(&self, _slice: HostSlice) {}
    fn teardown_step(&self, _step: Step) {}
}

impl Channels<'_, '_> {
    fn serve(&self, host_token: u32) {
        let Some((c, slot)) = self.slot(host_token) else { return };
        let token = c + 1;
        // ★ The token's BUSY state is the exclusion; this lock must NEVER be contended.
        let mut g = match slot.try_lock() {
            Ok(g) => g,
            Err(_) => {
                self.contended.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };
        let (chan, err) = &mut *g;
        if err.is_some() {
            return;
        }
        let io = || Io { s: self.s, c: token - 1 };
        let (mut mem, mut userd, mut publisher) = (io(), io(), io());
        if let Err(e) = chan.pump(self.s.rm, self.done, &mut mem, &mut userd, &mut publisher, is_ce, self.win) {
            *err = Some(format!("{e:?}"));
        }
    }
}

#[allow(clippy::too_many_lines)]
fn run(l: &mut Checks) -> Result<(), String> {
    let dev = DevDir::open(c"/dev").map_err(|e| format!("open /dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_arch::ids::GpuId(0), &kf_chip::choose_host_classes).map_err(|e| e.to_string())?;
    let ce_class = rm.ce_class_id();
    let res = rm.reserve_gpga(STORE_BYTES).map_err(|e| format!("reserve: {e:?}"))?;
    let store = res.handle;
    let fd = rm.export_to_new_fd(store).map_err(|e| format!("export: {e:?}"))?;
    let walk = WalkKernel::bring_up(WalkCfg::default(), kf_format_ver2()).map_err(|e| e.to_string())?;
    let dptr = walk.import_store(fd.fd_number(), STORE_BYTES).map_err(|e| e.to_string())?;
    let space = rm.alloc_vaspace().map_err(|e| format!("vaspace: {e:?}"))?;
    let fb_base = rm.map_window(space, store, STORE_BYTES, true).map_err(|e| format!("identity window: {e:?}"))?;

    let ram_fd = kf_linux_raw::SharedRam::create(RAM_BYTES).map_err(|e| format!("memfd: {e:?}"))?;
    let page = HostPageSize::query();
    let backing = || Backing::SharedFile { fd: ram_fd.as_backing_fd(), offset: 0 };
    let ram_view = kf_linux_raw::MappedRegion::map(backing(), RAM_BYTES, kf_linux_raw::HostProt::ReadWrite, CachePolicy::WriteBack, page)
        .map_err(|e| format!("map ram: {e:?}"))?;
    let ram_desc = rm.alloc_os_descriptor(&ram_view, At::new(0), RAM_BYTES).map_err(|e| format!("ram descriptor: {e:?}"))?;
    let ram_base = rm.map_window(space, ram_desc, RAM_BYTES, true).map_err(|e| format!("ram window: {e:?}"))?;
    // The view every thread shares: volatile word access, never a byte memcpy racing the guest.
    let ram = VolatileRegion::map(backing(), RAM_BYTES, CachePolicy::WriteBack, page).map_err(|e| format!("ram view: {e:?}"))?;
    let win = Windows { fb: fb_base, ram: ram_base };
    l.measure("windows", format!("fb={fb_base:#x} ram={ram_base:#x}"));

    // ── the guest kernel: channels in guest RAM, mapped by SYSMEM PTEs ───────────────────────
    let mut tree = Tree::new(PT_BASE, PT_BYTES);
    for c in 0..CHANNELS {
        for p in 0..CH_STRIDE / 4096 {
            tree.map4k_sys(va(c) + p * 4096, ch(c) + p * 4096);
        }
    }
    let w = |off: u64, b: &[u8]| walk.write_at(dptr + off, b).map_err(|e| e.to_string());
    w(PT_BASE, &tree.img.mem)?;
    let root = tree.root;
    let put = |off: u64, words: &[u32]| -> Result<(), String> {
        for (i, x) in words.iter().enumerate() {
            ram.store_u32(At::new(off + 4 * i as u64), *x).map_err(|e| format!("{e:?}"))?;
        }
        Ok(())
    };
    for c in 0..CHANNELS {
        w(src(c), &pattern(0xC000_0000 | (c << 20)))?;
        w(DST + u64::from(c) * 0x10_0000, &vec![0u8; (u64::from(PER_CHANNEL) * COPY) as usize])?;
        put(ch(c), &vec![0u32; (SEGS / 4) as usize])?;
        let pitch = ce::LAUNCH_SRC_PITCH | ce::LAUNCH_DST_PITCH;
        for k in 0..PER_CHANNEL {
            let mut seg = if k == 0 { m(4, SET_OBJECT, &[ce_class]) } else { Vec::new() };
            if c == 0 && k == SPLIT_AT {
                seg.extend(m(0, 0x28, &[0, 0, lo(root) & 0xFFFF_F000, (9 << 27) | (hi(root) & 0x07FF_FFFF)]));
            }
            seg.extend(m(4, ce::SET_SRC_PHYS_MODE, &[0, 0]));
            seg.extend(m(4, ce::OFFSET_IN_UPPER, &[hi(src(c)), lo(src(c)), hi(dst(c, k)), lo(dst(c, k))]));
            seg.extend(m(4, ce::LINE_LENGTH_IN, &[COPY as u32]));
            seg.extend(m(4, ce::SET_SEMAPHORE_A, &[hi(va(c) + SEM), lo(va(c) + SEM), k + 1]));
            seg.extend(m(4, ce::LAUNCH_DMA, &[ce::LAUNCH_TRANSFER_NON_PIPELINED
                | ce::LAUNCH_FLUSH_ENABLE
                | ce::LAUNCH_SEMAPHORE_RELEASE_ONE_WORD
                | ce::LAUNCH_SRC_PHYSICAL
                | ce::LAUNCH_DST_PHYSICAL
                | pitch]));
            let at = SEGS + u64::from(k) * SEG_STRIDE;
            put(ch(c) + at, &seg)?;
            let e = gp_entry(va(c) + at, 4 * seg.len() as u64).ok_or("gp entry")?;
            put(ch(c) + GPFIFO + 8 * u64::from(k), &[e as u32, (e >> 32) as u32])?;
        }
    }

    let mm = Mutex::new(Mm { walk, dptr, ledger: Ledger::default(), walks: Vec::new() });
    let shared = Shared { rm: &rm, ram: &ram, mm: &mm, space, store, ram_desc, root };
    let (mapped, _) = shared.publish("boot")?;
    l.check("kernel_vas_published_as_sysmem", mapped >= 1, format!("{mapped} runs (sysmem rows)"));
    mm.lock().map_err(|_| "poisoned")?.walks.clear();

    // ── the doorbell plane: kf_core::Plane, for the host's family ─────────────────────────
    let vmm = Vmm::new();
    let mut plane = Plane::for_family(&vmm, TOKEN_TABLE, (TOKEN_TABLE - 1) as u32, kf_chip::Family::Ampere, 16 << 20);
    let mut caps = VmCaps::from_declared(8, 8, 8, 8);
    let efd = Notifier::create().map_err(|e| format!("eventfd: {e:?}"))?;
    let stats = WorkerStats::default();
    let done = Completions::open(&rm, TOKEN_TABLE)?;
    let mut chans = Vec::new();
    for c in 0..CHANNELS {
        let host = HostRing::new(&rm, space)?;
        let ht = host.channel().token;
        // A guest-KERNEL channel: a translation refusal on it must poison, never fault (§7).
        plane
            .allocate_channel(&mut caps, c + 1, Route::Translated, ht, Owner::Kernel)
            .map_err(|e| format!("token {}: {e:?}", c + 1))?;
        chans.push((ht, Mutex::new((TranslatedChannel::new(TranslatedRing::new(va(c) + GPFIFO, ENTRIES, 0), host, c + 1), None))));
    }
    let channels = Channels {
        s: &shared,
        done: &done,
        win: &win,
        chans,
        contended: AtomicU64::new(0),
        forged: AtomicU64::new(0),
        poisoned: AtomicU64::new(0),
    };
    let plane = &plane;
    let stop = AtomicBool::new(false);
    let hostile_actions = AtomicU64::new(0);
    let wakes_signalled = AtomicU64::new(0);

    let t0 = std::time::Instant::now();
    let outcome: Result<(), String> = std::thread::scope(|sc| {
        for _ in 0..WORKERS {
            let poller = Poller::create().map_err(|e| format!("epoll: {e:?}"))?;
            poller.watch(efd.as_source_fd(), WORKER_EFD_TAG).map_err(|e| format!("watch: {e:?}"))?;
            poller.watch(done.event_fd(), COMPLETIONS_TAG).map_err(|e| format!("watch: {e:?}"))?;
            let (channels, stop, efd, done, stats) = (&channels, &stop, &efd, &done, &stats);
            sc.spawn(move || kf_chan::worker::run(plane, channels, &poller, efd, done, stats, stop, &|_| {}));
        }
        // vCPUs: advance GP_PUT, then ring — exactly what the guest's store to the doorbell does.
        for c in 0..CHANNELS {
            let (ram, efd, wakes_signalled) = (&ram, &efd, &wakes_signalled);
            sc.spawn(move || {
                let mut x = 0x9E37_79B9u32 ^ c;
                for k in 0..PER_CHANNEL {
                    let _ = ram.store_u32(At::new(ch(c) + USERD + USERD_GP_PUT), k + 1);
                    kf_linux_raw::release_fence();
                    if plane.trap_write(Class::Doorbell, 0, 0x90, u64::from(c + 1), 4) == Action::WakeWorker {
                        wakes_signalled.fetch_add(1, Ordering::Relaxed);
                        let _ = efd.signal();
                    }
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    std::thread::sleep(std::time::Duration::from_micros(u64::from(x % 300)));
                }
            });
        }
        // The hostile vCPU: an UNKNOWN token and the userspace-mappable arm, in a tight loop.
        {
            let hostile_actions = &hostile_actions;
            sc.spawn(move || {
                for i in 0..200_000u32 {
                    let a = if i % 2 == 0 {
                        plane.trap_write(Class::Doorbell, 0, 0x90, u64::from(HOSTILE_TOKEN), 4)
                    } else {
                        plane.trap_write(Class::UserspaceMappable, 0, 0x94, 0xDEAD_BEEF, 4)
                    };
                    if a != Action::None {
                        hostile_actions.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        }
        // Wait for every channel to retire everything, then stop the workers.
        let deadline = t0 + std::time::Duration::from_secs(15);
        loop {
            let done = (0..CHANNELS).all(|c| {
                ram.load_u32(At::new(ch(c) + USERD + USERD_GP_GET)).is_ok_and(|g| g == PER_CHANNEL)
            });
            let dead = channels.chans.iter().any(|(_, s)| s.try_lock().is_ok_and(|g| g.1.is_some()));
            if done || dead || std::time::Instant::now() > deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        stop.store(true, Ordering::Release);
        Ok(())
    });
    outcome?;
    let us = t0.elapsed().as_micros();

    let mut errors = Vec::new();
    let mut per = Vec::new();
    for (c, (_, slot)) in channels.chans.iter().enumerate() {
        let g = slot.lock().map_err(|_| "poisoned")?;
        if let Some(e) = &g.1 {
            errors.push(format!("ch{c}: {e}"));
        }
        per.push(format!("ch{c}:{:?}", g.0.counts()));
    }
    l.measure(
        "plane",
        format!(
            "us={us} served={} parks={} host_rings={} vcpu_wakes={} per_channel=[{}]",
            stats.served.load(Ordering::Relaxed),
            stats.parks.load(Ordering::Relaxed),
            stats.host_rings.load(Ordering::Relaxed),
            wakes_signalled.load(Ordering::Relaxed),
            per.join(" ")
        ),
    );
    l.check("no_channel_died", errors.is_empty(), errors.join("; "));
    l.check("translated_never_forged", channels.forged.load(Ordering::Relaxed) == 0 && channels.poisoned.load(Ordering::Relaxed) == 0, format!("forged={} poisoned={} (§8: the GPU writes a Translated completion)", channels.forged.load(Ordering::Relaxed), channels.poisoned.load(Ordering::Relaxed)));
    l.check("pump_never_contended", channels.contended.load(Ordering::Relaxed) == 0, format!("{} contended serves", channels.contended.load(Ordering::Relaxed)));
    l.check("hostile_rings_produce_no_action", hostile_actions.load(Ordering::Relaxed) == 0, format!("{} of 200000", hostile_actions.load(Ordering::Relaxed)));

    let g = mm.lock().map_err(|_| "poisoned")?;
    g.walk.make_current().map_err(|e| e.to_string())?;
    let mut bad = Vec::new();
    for c in 0..CHANNELS {
        let want = pattern(0xC000_0000 | (c << 20));
        for k in 0..PER_CHANNEL {
            let mut got = vec![0u8; COPY as usize];
            g.walk.read_at(g.dptr + dst(c, k), &mut got).map_err(|e| e.to_string())?;
            if got != want {
                bad.push(format!("ch{c}#{k}"));
            }
        }
    }
    l.check("every_copy_landed", bad.is_empty(), format!("{} of {} wrong {:?}", bad.len(), CHANNELS * PER_CHANNEL, bad.iter().take(6).collect::<Vec<_>>()));
    for c in 0..CHANNELS {
        let gp_get = ram.load_u32(At::new(ch(c) + USERD + USERD_GP_GET)).map_err(|e| format!("{e:?}"))?;
        let sem = ram.load_u32(At::new(ch(c) + SEM)).map_err(|e| format!("{e:?}"))?;
        let ok = gp_get == PER_CHANNEL && sem == PER_CHANNEL;
        l.check("channel_retired_and_released", ok, format!("ch{c}: GP_GET={gp_get} sem={sem} (in guest RAM, written by the engine)"));
    }
    l.check("split_walked_once_on_a_worker", g.walks == vec![Some(root)], format!("walks={:x?}", g.walks));
    Ok(())
}
