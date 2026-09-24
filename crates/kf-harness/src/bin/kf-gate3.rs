//! ★★★ **v3 gate 3 — a guest KERNEL copy-engine channel, Translated onto the real engine.**
//!
//! The harness plays the guest kernel's RM CeUtils channel (`THE_TRANSLATED_PLANE.md` §19, §24.2):
//! its page tables, GPFIFO, pushbuffer, semaphore and USERD all live in the one store object at the
//! guest's own kernel VAs; its data operands are FB-PHYSICAL. v3's Translated plane must:
//! - read the guest's GP entries and segments through OUR mappings (ledger VA → store offset);
//! - rewrite PHYSICAL operands onto the identity window and run them on OUR host ring;
//! - rewrite PHYSICAL SYSMEM operands onto the guest-RAM window (one `OS_DESCRIPTOR` over the
//!   guest's memfd, §6), both directions;
//! - let the engine write the guest's semaphore NATIVELY, through the mirrored kernel VAS;
//! - split at the guest's `MEM_OP` invalidate: everything before it COMPLETES, then walk + publish,
//!   then the rest — proved by a VIRTUAL copy through a VA the guest mapped just before the split;
//! - author the guest's `GP_GET` only on completion;
//! - refuse a hostile entry by name, and run nothing of it.
//! Every pump runs on the doorbell or on an event-fd wake; nothing polls a completion.

use kf_abi::submit::{SET_OBJECT, ce, gp_entry, method_header_inc};
use kf_chan::host::{GuestUserd, HostRing, Publisher, Pumped, TranslatedChannel};
use kf_chan::ring::{GuestMemory, RingRefusal, TranslatedRing};
use kf_chan::translated::{Refusal, Target, Window};
use kf_chip::Family;
use kf_cuda::abi::kf_format_ver2;
use kf_cuda::walk::{WalkCfg, WalkKernel};
use kf_harness::Ledger as Checks;
use kf_harness::tables::Tree;
use kf_host::{HostRm, VaSpace};
use kf_linux_raw::{DevDir, PollTimeout, Poller, ReadyTokens};
use kf_mem::ledger::{Ledger, desired_from_leaves, plan_reconcile};
use std::cell::RefCell;

const STORE_BYTES: u64 = 256 << 20;
const PT_BASE: u64 = 0x0100_0000;
const PT_BYTES: usize = 4 << 20;
/// The guest kernel channel's own memory: GPFIFO, segments, semaphore, USERD.
const CHAN: u64 = 0x0300_0000;
const CHAN_PAGES: u64 = 16;
const GPFIFO: u64 = 0x0;
const ENTRIES: u32 = 32;
const SEG: u64 = 0x1000;
const SEM: u64 = 0x8000;
const USERD: u64 = 0x9000;
/// Data, all FB-physical (store offsets).
const SRC: u64 = 0x0400_0000;
const DST: u64 = 0x0410_0000;
const FILL: u64 = 0x0420_0000;
const NEW: u64 = 0x0430_0000;
const DST2: u64 = 0x0440_0000;
const DST3: u64 = 0x0450_0000;
const BYTES: u64 = 0x1_0000;
/// Guest RAM: a memfd, and two guest-physical addresses in it.
const RAM_BYTES: u64 = 64 << 20;
const R_SRC: u64 = 0x10_0000;
const R_DST: u64 = 0x20_0000;
/// The guest kernel's VAs.
const VA_CHAN: u64 = 0x20_0000_0000;
const VA_NEW: u64 = 0x20_4000_0000;
const P1: u32 = 0x5C0B_0001;
const P2: u32 = 0x5C0B_0002;
const P3: u32 = 0x5C0B_0003;

fn main() {
    let mut l = Checks::default();
    if let Err(e) = run(&mut l) {
        l.check("run", false, e);
    }
    let v = l.verdict();
    println!("GATE3_VERDICT={}", if v { "PASS" } else { "FAIL" });
    std::process::exit(if v { 0 } else { 1 });
}

fn pattern(seed: u32) -> Vec<u8> {
    (0..(BYTES / 4) as u32).flat_map(|i| (seed ^ i).to_le_bytes()).collect()
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

/// The shared state every adapter needs: the walk kernel's store window and our ledger.
struct Guest<'a> {
    rm: &'a HostRm,
    ram_view: &'a kf_linux_raw::MappedRegion,
    walk: WalkKernel,
    dptr: u64,
    ledger: Ledger,
    space: VaSpace,
    store: u32,
    ram_desc: u32,
    root: u64,
    walks: Vec<Option<u64>>,
}

impl Guest<'_> {
    fn publish(&mut self, tag: &str) -> Result<(usize, usize), String> {
        let t0 = std::time::Instant::now();
        let r = self.walk.refresh(self.dptr, STORE_BYTES, &[self.root]).map_err(|e| e.to_string())?;
        r.validate().map_err(|e| format!("{tag}: report {e}"))?;
        // Guest RAM is one memfd from GPA 0 (no hole in this harness's layout).
        let ram_offset = |gpa: u64, len: u64| gpa.checked_add(len).filter(|&e| e <= RAM_BYTES).map(|_| gpa);
        let desired = desired_from_leaves(r.runs.iter().map(|m| (m.va, m.gpga, m.len, m.aperture())), STORE_BYTES, &ram_offset)
            .map_err(|e| format!("{tag}: {e:?}"))?;
        let plan = plan_reconcile(&self.ledger.rows(), &desired);
        let a = self.ledger.apply(self.rm, self.space, self.store, Some(self.ram_desc), &plan);
        println!(
            "MEASURE publish_{tag} runs={} kept={} mapped={} unmapped={} refused={} us={}{}",
            r.runs.len(), plan.kept, a.mapped, a.unmapped, a.refused, t0.elapsed().as_micros(),
            a.first_refusal.map(|f| format!(" FIRST-REFUSAL[{f}]")).unwrap_or_default()
        );
        if a.refused > 0 {
            return Err(format!("{tag}: {} refused", a.refused));
        }
        Ok((a.mapped, a.unmapped))
    }
}

struct Mem<'a, 'b>(&'a RefCell<Guest<'b>>);
impl GuestMemory for Mem<'_, '_> {
    fn read(&mut self, va: u64, out: &mut [u8]) -> Result<(), String> {
        let g = self.0.borrow();
        let (ram, off) = g.ledger.resolve(va, out.len() as u64).ok_or(format!("{va:#x} not mapped by us"))?;
        if ram {
            return g.ram_view.read_into(kf_linux_raw::HostOffset::new(off), out).map_err(|e| format!("{e:?}"));
        }
        g.walk.read_at(g.dptr + off, out).map_err(|e| e.to_string())
    }
}

struct Userd<'a, 'b>(&'a RefCell<Guest<'b>>);
impl GuestUserd for Userd<'_, '_> {
    fn gp_put(&mut self) -> Result<u32, String> {
        let g = self.0.borrow();
        let mut b = [0u8; 4];
        g.walk.read_at(g.dptr + CHAN + USERD + kf_abi::submit::USERD_GP_PUT, &mut b).map_err(|e| e.to_string())?;
        Ok(u32::from_le_bytes(b))
    }
    fn set_gp_get(&mut self, v: u32) -> Result<(), String> {
        let g = self.0.borrow();
        g.walk
            .write_at(g.dptr + CHAN + USERD + kf_abi::submit::USERD_GP_GET, &v.to_le_bytes())
            .map_err(|e| e.to_string())
    }
}

struct Pub<'a, 'b>(&'a RefCell<Guest<'b>>);
impl Publisher for Pub<'_, '_> {
    fn invalidated(&mut self, pdb: Option<u64>) -> Result<(), String> {
        let mut g = self.0.borrow_mut();
        g.walks.push(pdb);
        if pdb.is_some_and(|p| p != g.root) {
            return Err(format!("invalidate names {pdb:x?}, not the kernel root {:#x}", g.root));
        }
        g.publish("at_split").map(|_| ())
    }
}

/// Guest FB-physical `p` ⇒ `fb + p` inside the store; guest-physical `g` ⇒ `ram + g` inside RAM.
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

#[allow(clippy::too_many_lines)]
fn run(l: &mut Checks) -> Result<(), String> {
    let dev = DevDir::open(c"/dev").map_err(|e| format!("open /dev: {e:?}"))?;
    let pick = |a: u32, i: u32| Family::from_arch(a, i).ok().map(Family::host_classes);
    let rm = HostRm::open(&dev, kf_arch::ids::GpuId(0), &pick).map_err(|e| e.to_string())?;
    let ce_class = rm.ce_class_id();
    let res = rm.reserve_gpga(STORE_BYTES).map_err(|e| format!("reserve: {e:?}"))?;
    let store = res.handle;
    let fd = rm.export_to_new_fd(store).map_err(|e| format!("export: {e:?}"))?;
    let walk = WalkKernel::bring_up(WalkCfg::default(), kf_format_ver2()).map_err(|e| e.to_string())?;
    let dptr = walk.import_store(fd.fd_number(), STORE_BYTES).map_err(|e| e.to_string())?;
    let space = rm.alloc_vaspace().map_err(|e| format!("vaspace: {e:?}"))?;
    let t0 = std::time::Instant::now();
    let base = rm.map_window(space, store, STORE_BYTES, true).map_err(|e| format!("identity window: {e:?}"))?;
    l.measure("identity_window", format!("base={base:#x} bytes={STORE_BYTES:#x} us={} (GROWS_DOWN, read back)", t0.elapsed().as_micros()));
    let ram = kf_linux_raw::SharedRam::create(RAM_BYTES).map_err(|e| format!("memfd: {e:?}"))?;
    let ram_view = kf_linux_raw::MappedRegion::map(
        kf_linux_raw::Backing::SharedFile { fd: ram.as_backing_fd(), offset: 0 },
        RAM_BYTES,
        kf_linux_raw::HostProt::ReadWrite,
        kf_linux_raw::CachePolicy::WriteBack,
        kf_linux_raw::HostPageSize::query(),
    )
    .map_err(|e| format!("map ram: {e:?}"))?;
    let t0 = std::time::Instant::now();
    let desc = rm
        .alloc_os_descriptor(&ram_view, kf_linux_raw::HostOffset::new(0), RAM_BYTES)
        .map_err(|e| format!("ram descriptor: {e:?}"))?;
    let ram_base = rm.map_window(space, desc, RAM_BYTES, true).map_err(|e| format!("ram window: {e:?}"))?;
    let overlap = ram_base < base + STORE_BYTES && base < ram_base + RAM_BYTES;
    l.measure("ram_window", format!("base={ram_base:#x} bytes={RAM_BYTES:#x} us={}", t0.elapsed().as_micros()));
    l.check("windows_distinct", !overlap, format!("fb={base:#x} ram={ram_base:#x}"));
    ram_view.write_from(kf_linux_raw::HostOffset::new(R_SRC), &pattern(0x5A5A_0000)).map_err(|e| format!("{e:?}"))?;
    let win = Windows { fb: base, ram: ram_base };

    // ── the guest kernel: tables, data, and its channel's memory ──────────────────────────
    let mut tree = Tree::new(PT_BASE, PT_BYTES);
    for i in 0..CHAN_PAGES {
        tree.map4k(VA_CHAN + i * 4096, CHAN + i * 4096);
    }
    let w = |off: u64, b: &[u8]| walk.write_at(dptr + off, b).map_err(|e| e.to_string());
    w(PT_BASE, &tree.img.mem)?;
    w(SRC, &pattern(0xC0DE_0000))?;
    w(NEW, &pattern(0x0E1E_0000))?;
    w(DST, &vec![0u8; BYTES as usize])?;
    w(DST2, &vec![0u8; BYTES as usize])?;
    w(DST3, &vec![0u8; BYTES as usize])?;
    w(FILL, &vec![0xABu8; BYTES as usize])?;
    w(CHAN, &vec![0u8; (CHAN_PAGES * 4096) as usize])?;

    // GP 0 — the scrub's shape: a remapped CONSTANT fill, then a copy, both FB-PHYSICAL, with the
    // semaphore released at a kernel VA.
    let pitch = ce::LAUNCH_SRC_PITCH | ce::LAUNCH_DST_PITCH;
    let mut g0 = m(4, SET_OBJECT, &[ce_class]);
    g0.extend(m(4, ce::SET_REMAP_CONST_A, &[0]));
    g0.extend(m(4, ce::SET_REMAP_COMPONENTS, &[(3 << 16) | ce::REMAP_DST_SEL_CONST_A]));
    g0.extend(m(4, ce::SET_DST_PHYS_MODE, &[0]));
    g0.extend(m(4, ce::OFFSET_OUT_UPPER, &[hi(FILL), lo(FILL)]));
    g0.extend(m(4, ce::LINE_LENGTH_IN, &[(BYTES / 4) as u32]));
    g0.extend(m(4, ce::LAUNCH_DMA, &[ce::LAUNCH_TRANSFER_NON_PIPELINED
        | ce::LAUNCH_FLUSH_ENABLE
        | ce::LAUNCH_REMAP_ENABLE
        | ce::LAUNCH_DST_PHYSICAL
        | pitch]));
    g0.extend(m(4, ce::SET_SRC_PHYS_MODE, &[0]));
    g0.extend(m(4, ce::OFFSET_IN_UPPER, &[hi(SRC), lo(SRC), hi(DST), lo(DST)]));
    g0.extend(m(4, ce::LINE_LENGTH_IN, &[BYTES as u32]));
    g0.extend(m(4, ce::SET_SEMAPHORE_A, &[hi(VA_CHAN + SEM), lo(VA_CHAN + SEM), P1]));
    g0.extend(m(4, ce::LAUNCH_DMA, &[ce::LAUNCH_TRANSFER_NON_PIPELINED
        | ce::LAUNCH_FLUSH_ENABLE
        | ce::LAUNCH_SEMAPHORE_RELEASE_ONE_WORD
        | ce::LAUNCH_SRC_PHYSICAL
        | ce::LAUNCH_DST_PHYSICAL
        | pitch]));
    // GP 1 — the guest mapped VA_NEW just before this; it invalidates, then copies THROUGH it.
    let root = tree.root;
    let mut g1 = m(0, 0x28, &[0, 0, lo(root) & 0xFFFF_F000, (9 << 27) | (hi(root) & 0x07FF_FFFF)]);
    g1.extend(m(4, ce::OFFSET_IN_UPPER, &[hi(VA_NEW), lo(VA_NEW), hi(DST2), lo(DST2)]));
    g1.extend(m(4, ce::SET_SEMAPHORE_A, &[hi(VA_CHAN + SEM), lo(VA_CHAN + SEM), P2]));
    g1.extend(m(4, ce::LAUNCH_DMA, &[ce::LAUNCH_TRANSFER_NON_PIPELINED
        | ce::LAUNCH_FLUSH_ENABLE
        | ce::LAUNCH_SEMAPHORE_RELEASE_ONE_WORD
        | ce::LAUNCH_DST_PHYSICAL
        | pitch]));
    // GP 2 — UVM's shape: guest-RAM → FB, then FB → guest-RAM, sysmem operands PHYSICAL.
    let both_phys = ce::LAUNCH_TRANSFER_NON_PIPELINED | ce::LAUNCH_FLUSH_ENABLE | ce::LAUNCH_SRC_PHYSICAL | ce::LAUNCH_DST_PHYSICAL | pitch;
    let mut g2 = m(4, ce::SET_SRC_PHYS_MODE, &[1, 0]); // SRC coherent sysmem, DST local FB
    g2.extend(m(4, ce::OFFSET_IN_UPPER, &[hi(R_SRC), lo(R_SRC), hi(DST3), lo(DST3)]));
    g2.extend(m(4, ce::LAUNCH_DMA, &[both_phys]));
    g2.extend(m(4, ce::SET_SRC_PHYS_MODE, &[0, 1])); // SRC local FB, DST coherent sysmem
    g2.extend(m(4, ce::OFFSET_IN_UPPER, &[hi(SRC), lo(SRC), hi(R_DST), lo(R_DST)]));
    g2.extend(m(4, ce::SET_SEMAPHORE_A, &[hi(VA_CHAN + SEM), lo(VA_CHAN + SEM), P3]));
    g2.extend(m(4, ce::LAUNCH_DMA, &[both_phys | ce::LAUNCH_SEMAPHORE_RELEASE_ONE_WORD]));
    // GP 3 — hostile: a PEERMEM destination.
    let mut g3 = m(4, ce::SET_DST_PHYS_MODE, &[3]);
    g3.extend(m(4, ce::LAUNCH_DMA, &[ce::LAUNCH_TRANSFER_NON_PIPELINED | ce::LAUNCH_DST_PHYSICAL | pitch]));
    let words = |v: &[u32]| -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() };
    let mut seg_off = SEG;
    for (gp, seg) in [&g0, &g1, &g2, &g3].into_iter().enumerate() {
        w(CHAN + seg_off, &words(seg))?;
        let e = gp_entry(VA_CHAN + seg_off, 4 * seg.len() as u64).ok_or("gp entry")?;
        w(CHAN + GPFIFO + 8 * gp as u64, &e.to_le_bytes())?;
        seg_off += 0x1000;
    }

    // ── v3: the kernel VAS is mirrored by walking the guest's own tables ─────────────────────
    let guest = RefCell::new(Guest { rm: &rm, ram_view: &ram_view, walk, dptr, ledger: Ledger::default(), space, store, ram_desc: desc, root, walks: Vec::new() });
    let (mapped, _) = guest.borrow_mut().publish("boot")?;
    l.check("kernel_vas_published", mapped >= 1, format!("{mapped} runs"));
    let host = HostRing::new(&rm, space)?;
    let poller = Poller::create().map_err(|e| format!("epoll: {e:?}"))?;
    poller.watch(host.event_fd(), 1).map_err(|e| format!("watch: {e:?}"))?;
    let mut chan = TranslatedChannel::new(TranslatedRing::new(VA_CHAN + GPFIFO, ENTRIES, 0), host);

    // The guest maps VA_NEW (it will invalidate in GP 1), then rings entries 0, 1 and 2.
    let mut t = tree;
    for i in 0..BYTES / 4096 {
        t.map4k(VA_NEW + i * 4096, NEW + i * 4096);
    }
    guest.borrow().walk.write_at(dptr + PT_BASE, &t.img.mem).map_err(|e| e.to_string())?;
    let put = |g: &RefCell<Guest>, v: u32| -> Result<(), String> {
        let g = g.borrow();
        g.walk
            .write_at(g.dptr + CHAN + USERD + kf_abi::submit::USERD_GP_PUT, &v.to_le_bytes())
            .map_err(|e| e.to_string())
    };
    put(&guest, 3)?;

    // The worker: pump on the doorbell, then on every wake, until caught up and retired.
    let t0 = std::time::Instant::now();
    let deadline = t0 + std::time::Duration::from_secs(3);
    let mut wakes = 0u32;
    let mut pumps = 0u32;
    let pump = |chan: &mut TranslatedChannel| {
        chan.pump(&rm, &mut Mem(&guest), &mut Userd(&guest), &mut Pub(&guest), is_ce, &win)
    };
    let mut state = pump(&mut chan).map_err(|e| format!("pump: {e:?}"))?;
    pumps += 1;
    while !(state == Pumped::Caught && chan.last_gp_get() == Some(3)) && std::time::Instant::now() < deadline {
        let mut ready = ReadyTokens::new();
        if poller.wait(&mut ready, PollTimeout::Millis(500)).map_err(|e| format!("wait: {e:?}"))? > 0 {
            wakes += 1;
        }
        state = pump(&mut chan).map_err(|e| format!("pump: {e:?}"))?;
        pumps += 1;
    }
    let us = t0.elapsed().as_micros();
    let (fetched, submissions, splits) = chan.counts();
    l.measure("translated", format!("fetched={fetched} submissions={submissions} splits={splits} wakes={wakes} pumps={pumps} us={us}"));

    let g = guest.borrow();
    let rd = |off: u64, n: usize| -> Result<Vec<u8>, String> {
        let mut b = vec![0u8; n];
        g.walk.read_at(g.dptr + off, &mut b).map_err(|e| e.to_string())?;
        Ok(b)
    };
    let word = |off: u64| -> Result<u32, String> {
        let b = rd(off, 4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    l.check("forwarded_per_token", fetched == 3 && submissions >= 3, format!("fetched={fetched} submissions={submissions}"));
    l.check("fill_zeroed_by_engine", rd(FILL, BYTES as usize)?.iter().all(|&b| b == 0), "FILL all zero (was 0xAB)");
    l.check("phys_copy_by_engine", rd(DST, BYTES as usize)? == pattern(0xC0DE_0000), "DST == SRC pattern");
    l.check(
        "split_walked_the_named_root",
        g.walks == vec![Some(root)],
        format!("walks={:x?} root={root:#x}", g.walks),
    );
    l.check("virtual_copy_after_split", rd(DST2, BYTES as usize)? == pattern(0x0E1E_0000), "DST2 == NEW pattern (read through VA_NEW)");
    l.check("sysmem_to_fb_by_engine", rd(DST3, BYTES as usize)? == pattern(0x5A5A_0000), "DST3 == guest-RAM R_SRC pattern");
    let mut back = vec![0u8; BYTES as usize];
    ram_view.read_into(kf_linux_raw::HostOffset::new(R_DST), &mut back).map_err(|e| format!("{e:?}"))?;
    l.check("fb_to_sysmem_by_engine", back == pattern(0xC0DE_0000), "guest-RAM R_DST == SRC pattern");
    let sem = word(CHAN + SEM)?;
    l.check("guest_semaphore_written_natively", sem == P3, format!("sem={sem:#x} want={P3:#x}"));
    let gp_get = word(CHAN + USERD + kf_abi::submit::USERD_GP_GET)?;
    l.check("gp_get_authored_on_completion", gp_get == 3, format!("guest GP_GET={gp_get}"));
    drop(g);

    // Hostile: GP 3 must be refused by name, and GP_GET must not move.
    put(&guest, 4)?;
    let r = pump(&mut chan);
    let named = matches!(r, Err(kf_chan::host::ChanError::Ring(RingRefusal::Rewrite { gp: 3, why: Refusal::PeerOperand })));
    l.check("hostile_entry_refused_by_name", named, format!("{r:?}"));
    let gp_get = {
        let g = guest.borrow();
        let mut b = [0u8; 4];
        g.walk.read_at(g.dptr + CHAN + USERD + kf_abi::submit::USERD_GP_GET, &mut b).map_err(|e| e.to_string())?;
        u32::from_le_bytes(b)
    };
    l.check("hostile_entry_not_retired", gp_get == 3, format!("guest GP_GET={gp_get}"));
    Ok(())
}
