// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★★ **Native oracle for the kernel-GR tier** (owner rulings 2026-10-07, `OWNER_RULINGS.md` §S:
//! "native first", before any Windows run). Bare metal, no QEMU, no guest.
//!
//! The harness plays a guest kernel's GR channel exactly as Windows drove it in run31: its GPFIFO,
//! pushbuffer, semaphore and USERD live in the store at the guest's kernel VAs (`0x1_2023_0000`,
//! so the segment's own `SEM_ADDR` words, `0x1_2023_b000`, are used unchanged). The words are run31's
//! logged 32, then a semaphore release. It checks, on the real GPU:
//!
//! - the host channel is born USER and the tier's assertion admits it; host objects of
//!   `FERMI_TWOD_A` and `KEPLER_INLINE_TO_MEMORY_B` exist on it;
//! - T-mode re-authors the segment and the ENGINE completes it: the guest semaphore is written
//!   natively, the guest's `GP_GET` is written by the engine (the CPU store count stays 0);
//! - which host events wake on that completion (`FIFO_EVENT_MTHD` and GR0, measured separately);
//! - a hostile entry (`LOAD_MME_INSTRUCTION_RAM`) is refused by name and not retired;
//! - run31's CE segment (software subchannel 5 bound to 1) completes on a CE T-mode ring with the
//!   inert-bind rule, and a later software method on subchannel 5 is refused by name;
//! - everything is released (channel, ring object, mapping, CPU view).
//!
//! It does NOT check the guest-vector relay (no VMM here); that is the Windows run's evidence.

use kf_abi::submit::{fifo, gp_entry, method_header_inc};
use kf_chan::host::{GuestUserd, HostRing, Publisher, Pumped, TranslatedChannel};
use kf_chan::ring::{GuestMemory, RingRefusal, TranslatedRing};
use kf_chan::tmode::{GrConfig, Rows, Span};
use kf_chan::translated::{Refusal, Target, Window};
use kf_cuda::abi::kf_format_ver2;
use kf_cuda::walk::{WalkCfg, WalkKernel};
use kf_harness::Ledger as Checks;
use kf_host::{HostRm, MapPerm};
use kf_linux_raw::{DevDir, PollTimeout, Poller, ReadyTokens};
use std::cell::Cell;

const STORE_BYTES: u64 = 64 << 20;
const RAM_BYTES: u64 = 16 << 20;
/// The guest channel memory in the store, and its kernel VA (run31's semaphore page lies inside).
const CHAN: u64 = 0x0100_0000;
const VA_CHAN: u64 = 0x1_2023_0000;
const CHAN_BYTES: u64 = 16 * 4096;
const GPFIFO: u64 = 0x0;
const ENTRIES: u32 = 32;
const SEG: u64 = 0x1000;
/// Run31's semaphore offsets: GR `0x1_2023_b000`, CE `0x1_2023_b060`.
const SEM_GR: u64 = 0xb000;
const SEM_CE: u64 = 0xb060;
const USERD: u64 = 0xc000;
/// A second channel's memory (the CE ring), at its own VA.
const CHAN2: u64 = 0x0200_0000;
const P1: u32 = 0x6A0B_0001;
const P2: u32 = 0x6A0B_0002;
const GR0_NOTIFIER: u32 = 12;

/// Run31's GR segment (c1d00015:ff040001, GP 0), the 32 words logged.
const RUN31_GR: [u32; 32] = [
    0x2002_0017,
    0x2023_b000,
    0x0000_0001,
    0x2001_4000,
    0x0000_a140,
    0x2001_6000,
    0x0000_902d,
    0x2001_6222,
    0x0000_0001,
    0x2001_60a4,
    0x0000_0000,
    0x2001_60a7,
    0x0000_0000,
    0x2001_60ab,
    0x0000_0003,
    0x2001_6201,
    0x0000_00cf,
    0x2004_6210,
    0,
    1,
    0,
    1,
    0x2004_6214,
    0,
    0,
    0,
    0,
    0x2004_6230,
    0,
    1,
    0,
    1,
];
/// Run31's CE segment (c1d00013:ff040000, GP 0), all 7 words.
const RUN31_CE: [u32; 7] = [
    0x2002_0017,
    0x2023_b060,
    0x0000_0001,
    0x2001_8000,
    0x0000_c7b5,
    0x2001_a000,
    0x0000_0001,
];

fn main() {
    let mut l = Checks::default();
    if let Err(e) = run(&mut l) {
        l.check("run", false, e);
    }
    let v = l.verdict();
    println!("GR_TIER_VERDICT={}", if v { "PASS" } else { "FAIL" });
    std::process::exit(if v { 0 } else { 1 });
}

fn m(sub: u32, method: u32, args: &[u32]) -> Vec<u32> {
    let mut v = vec![method_header_inc(sub, method, args.len() as u32).expect("header")];
    v.extend_from_slice(args);
    v
}

/// The release a guest appends: payload, then `SEM_EXECUTE` RELEASE with WFI.
fn release(payload: u32) -> Vec<u32> {
    let mut v = m(0, fifo::SEM_PAYLOAD_LO, &[payload, 0]);
    v.extend(m(
        0,
        fifo::SEM_EXECUTE,
        &[fifo::SEM_EXECUTE_RELEASE_32BIT | fifo::SEM_EXECUTE_RELEASE_WFI_EN],
    ));
    v
}

/// One guest channel's placement: `[va, va+bytes)` → store offset `off`.
struct Guest<'a> {
    walk: &'a WalkKernel,
    va: u64,
    off: u64,
    gp_get_cpu_stores: Cell<u32>,
}

impl Rows for Guest<'_> {
    fn resolve(&self, va: u64, len: u64) -> Result<Vec<Span>, u64> {
        let end = va.checked_add(len).ok_or(va)?;
        if va < self.va || end > self.va + CHAN_BYTES {
            return Err(va.max(self.va + CHAN_BYTES).min(va));
        }
        Ok(vec![Span {
            ram: false,
            off: self.off + (va - self.va),
            len,
            perm: MapPerm::READ_WRITE,
        }])
    }
    fn dma_to_file_range(&self, _: u64, _: u64) -> Option<u64> {
        None
    }
}

struct Mem<'a, 'b>(&'a Guest<'b>);
impl GuestMemory for Mem<'_, '_> {
    fn read(&mut self, va: u64, out: &mut [u8]) -> Result<(), String> {
        let s = self
            .0
            .resolve(va, out.len() as u64)
            .map_err(|at| format!("{at:#x} not placed"))?;
        self.0
            .walk
            .read_store(s[0].off, out)
            .map_err(|e| e.to_string())
    }
    fn rows(&self) -> Option<&dyn Rows> {
        Some(self.0)
    }
}

struct Userd<'a, 'b>(&'a Guest<'b>);
impl GuestUserd for Userd<'_, '_> {
    fn gp_put(&mut self) -> Result<u32, String> {
        let mut b = [0u8; 4];
        self.0
            .walk
            .read_store(self.0.off + USERD + kf_abi::submit::USERD_GP_PUT, &mut b)
            .map_err(|e| e.to_string())?;
        Ok(u32::from_le_bytes(b))
    }
    fn set_gp_get(&mut self, v: u32) -> Result<(), String> {
        self.0
            .gp_get_cpu_stores
            .set(self.0.gp_get_cpu_stores.get() + 1);
        self.0
            .walk
            .write_store(
                self.0.off + USERD + kf_abi::submit::USERD_GP_GET,
                &v.to_le_bytes(),
            )
            .map_err(|e| e.to_string())
    }
}

struct NoSplit;
impl Publisher for NoSplit {
    fn invalidated(&mut self, pdb: Option<u64>) -> Result<kf_chan::host::Split, String> {
        Err(format!("no split expected ({pdb:x?})"))
    }
}

/// T-mode never uses the legacy window (every operand goes through the T-space windows).
struct NoWindow;
impl Window for NoWindow {
    fn translate(&self, _: Target, _: u64, _: u64) -> Option<u64> {
        None
    }
}

fn is_ce(c: u32) -> bool {
    kf_chip::is_any_dma_copy_class(c)
}

fn word(walk: &WalkKernel, off: u64) -> Result<u32, String> {
    let mut b = [0u8; 4];
    walk.read_store(off, &mut b).map_err(|e| e.to_string())?;
    Ok(u32::from_le_bytes(b))
}

/// Write `segs` as GP entries 0.. of the channel at store `off` / VA `va`.
fn stage(walk: &WalkKernel, off: u64, va: u64, segs: &[&[u32]]) -> Result<(), String> {
    walk.write_store(off, &vec![0u8; CHAN_BYTES as usize])
        .map_err(|e| e.to_string())?;
    let mut at = SEG;
    for (gp, seg) in segs.iter().enumerate() {
        let bytes: Vec<u8> = seg.iter().flat_map(|w| w.to_le_bytes()).collect();
        walk.write_store(off + at, &bytes)
            .map_err(|e| e.to_string())?;
        let e = gp_entry(va + at, 4 * seg.len() as u64).ok_or("gp entry")?;
        walk.write_store(off + GPFIFO + 8 * gp as u64, &e.to_le_bytes())
            .map_err(|e| e.to_string())?;
        at += 0x1000;
    }
    Ok(())
}

fn put(walk: &WalkKernel, off: u64, v: u32) -> Result<(), String> {
    walk.write_store(off + USERD + kf_abi::submit::USERD_GP_PUT, &v.to_le_bytes())
        .map_err(|e| e.to_string())
}

/// Poll `fd` once for `ms`: did it become ready?
fn ready_within(fd: std::os::fd::BorrowedFd<'_>, ms: u32) -> Result<bool, String> {
    let p = Poller::create().map_err(|e| format!("epoll: {e:?}"))?;
    p.watch(fd, 1).map_err(|e| format!("watch: {e:?}"))?;
    let mut r = ReadyTokens::new();
    Ok(p.wait(&mut r, PollTimeout::Millis(ms))
        .map_err(|e| format!("wait: {e:?}"))?
        > 0)
}

#[allow(clippy::too_many_lines)]
fn run(l: &mut Checks) -> Result<(), String> {
    let dev = DevDir::open(c"/dev").map_err(|e| format!("open /dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    let res = rm
        .reserve_gpga(STORE_BYTES)
        .map_err(|e| format!("reserve: {e:?}"))?;
    let store = res.handle;
    let fd = rm
        .export_to_new_fd(store)
        .map_err(|e| format!("export: {e:?}"))?;
    let mut walk = WalkKernel::bring_up_on(
        WalkCfg::default(),
        kf_format_ver2(),
        kf_cuda::walk::WalkDevice::PciBusId(&rm.card().bdf()),
    )
    .map_err(|e| e.to_string())?;
    walk.import_store(fd.fd_number(), STORE_BYTES)
        .map_err(|e| e.to_string())?;

    // ── a T-space like kf3's: bare space, ring region reserved first, windows bottom-up ─────
    let mut space = rm
        .alloc_vaspace_bare()
        .map_err(|e| format!("space: {e:?}"))?;
    let region = 4u64 << 30;
    let region_base = kf_chan::host::RING_VA_LIMIT - region;
    let resv = rm
        .reserve_va(space.space, region_base, region)
        .map_err(|e| format!("ring region: {e:?}"))?;
    space.guest[0] = kf_host::channel::GuestVaRange {
        handle: resv,
        lo: region_base,
        hi: region_base + region,
    };
    let fb_base = rm
        .map_window(space, store, STORE_BYTES, false)
        .map_err(|e| format!("store window: {e:?}"))?;
    let ram = kf_linux_raw::SharedRam::create(RAM_BYTES).map_err(|e| format!("memfd: {e:?}"))?;
    let ram_view = kf_linux_raw::MappedRegion::map(
        kf_linux_raw::Backing::SharedFile {
            fd: ram.as_backing_fd(),
            offset: 0,
        },
        RAM_BYTES,
        kf_linux_raw::HostProt::ReadWrite,
        kf_linux_raw::CachePolicy::WriteBack,
        kf_linux_raw::HostPageSize::query(),
    )
    .map_err(|e| format!("map ram: {e:?}"))?;
    let desc = rm
        .alloc_os_descriptor(&ram_view, kf_linux_raw::HostOffset::new(0), RAM_BYTES)
        .map_err(|e| format!("ram descriptor: {e:?}"))?;
    let ram_base = rm
        .map_window(space, desc, RAM_BYTES, false)
        .map_err(|e| format!("ram window: {e:?}"))?;
    let win = kf_chan::tspace_unsafe::TWindows::new(
        (fb_base, STORE_BYTES),
        (ram_base, RAM_BYTES),
        region_base,
    )?;
    l.measure(
        "tspace",
        format!("fb={fb_base:#x}+{STORE_BYTES:#x} ram={ram_base:#x}+{RAM_BYTES:#x} rings={region_base:#x}"),
    );

    // ── the GR channel ───────────────────────────────────────────────────────────────────────
    let g = Guest {
        walk: &walk,
        va: VA_CHAN,
        off: CHAN,
        gp_get_cpu_stores: Cell::new(0),
    };
    let mut gp0 = RUN31_GR.to_vec();
    gp0.extend(release(P1));
    let gp1 = m(3, 0x0118, &[0]); // NV902D_LOAD_MME_INSTRUCTION_RAM — hostile
    stage(&walk, CHAN, VA_CHAN, &[&gp0, &gp1])?;
    walk.write_store(CHAN + SEM_GR, &0u32.to_le_bytes())
        .map_err(|e| e.to_string())?;

    let mut host = HostRing::on_engine_layout(
        &rm,
        space,
        kf_abi::submit::ENGINE_TYPE_GRAPHICS,
        Some(region_base),
        kf_chan::host::TSPACE_LAYOUT,
    )?;
    let stamp = host.channel().assert_user();
    l.check(
        "host_channel_asserted_user",
        stamp.is_ok(),
        format!("{:?} chan={:x?}", stamp, host.channel()),
    );
    let objects = host.admit_gr_tier(&rm);
    let classes: Vec<u32> = objects
        .as_ref()
        .map(|o| o.iter().map(|&(c, _)| c).collect())
        .unwrap_or_default();
    l.check(
        "gr_tier_objects_allocated",
        classes == [0x902d, 0xa140],
        format!("{objects:x?} context={:x?}", host.gr_context()),
    );
    let gp_get_at = win
        .fb(CHAN + USERD + kf_abi::submit::USERD_GP_GET, 4)
        .ok_or("USERD GP_GET outside the store window")?;
    let mut ring = TranslatedRing::new_tmode(VA_CHAN + GPFIFO, ENTRIES, 0);
    ring.set_gr(GrConfig {
        tier: true,
        inert_sw_subch: true,
    });
    let mut chan = TranslatedChannel::new(ring, host, 1);
    chan.set_tspace(win, false);
    chan.set_gpu_gp_get(gp_get_at)?;

    // The completion edges, each polled on its own fd: FIFO_EVENT_MTHD (kf3's session fd) and GR0.
    let done = kf_chan::completions::Completions::open(&rm, 64)?;
    let gr0 = rm.open_event_fd().map_err(|e| format!("gr0 fd: {e:?}"))?;
    rm.alloc_os_event(rm.subdevice(), GR0_NOTIFIER, true, &gr0)
        .map_err(|e| format!("gr0 event: {e:?}"))?;
    rm.arm_repeat(GR0_NOTIFIER)
        .map_err(|e| format!("gr0 arm: {e:?}"))?;
    let idle35 = ready_within(done.event_fd(), 200)?;
    let idle_gr0 = ready_within(gr0.as_fd(), 200)?;

    put(&walk, CHAN, 1)?;
    let t0 = std::time::Instant::now();
    let pump = |chan: &mut TranslatedChannel, g: &Guest| {
        chan.pump(
            &rm,
            &done,
            &mut Mem(g),
            &mut Userd(g),
            &mut NoSplit,
            is_ce,
            &NoWindow,
        )
    };
    let mut state = pump(&mut chan, &g).map_err(|e| format!("pump: {e:?}"))?;
    let woke35 = ready_within(done.event_fd(), 1000)?;
    let woke_gr0 = ready_within(gr0.as_fd(), 300)?;
    l.measure(
        "completion_edges",
        format!(
            "before doorbell: fifo_event_mthd={idle35} gr0={idle_gr0}; after: fifo_event_mthd={woke35} gr0={woke_gr0}"
        ),
    );
    let deadline = t0 + std::time::Duration::from_secs(3);
    while !(state == Pumped::Caught && chan.last_gp_get() == Some(1))
        && std::time::Instant::now() < deadline
    {
        let _ = ready_within(gr0.as_fd(), 100)?;
        state = pump(&mut chan, &g).map_err(|e| format!("pump: {e:?}"))?;
    }
    let (fetched, submissions, _) = chan.counts();
    l.measure(
        "gr_translated",
        format!(
            "fetched={fetched} submissions={submissions} gr_methods={} us={}",
            chan.gr_counts().0,
            t0.elapsed().as_micros()
        ),
    );
    l.check(
        "gr_segment_reauthored",
        chan.gr_counts().0 == 17,
        format!("gr_methods={}", chan.gr_counts().0),
    );
    let sem = word(&walk, CHAN + SEM_GR)?;
    l.check(
        "guest_semaphore_written_by_engine",
        sem == P1,
        format!("sem={sem:#x} want={P1:#x}"),
    );
    let gp_get = word(&walk, CHAN + USERD + kf_abi::submit::USERD_GP_GET)?;
    l.check(
        "gp_get_written_by_engine",
        gp_get == 1 && g.gp_get_cpu_stores.get() == 0 && chan.gpu_gp_get() == (true, 1),
        format!(
            "guest GP_GET={gp_get} cpu_stores={} engine={:?}",
            g.gp_get_cpu_stores.get(),
            chan.gpu_gp_get()
        ),
    );
    // kf3's worker pumps on the session fd (FIFO_EVENT_MTHD): that edge must fire for the ring's
    // fence NSI, or a GR-tier completion would be seen only at the next doorbell. (GR0's own
    // notifier is only reported above, not required.)
    l.check(
        "completion_wakes_session_fd",
        woke35,
        format!("fifo_event_mthd={woke35} gr0={woke_gr0}"),
    );

    // Hostile: GP 1 must be refused by name, and GP_GET must not move.
    put(&walk, CHAN, 2)?;
    let r = pump(&mut chan, &g);
    let named = matches!(
        &r,
        Err(kf_chan::host::ChanError::Ring(RingRefusal::Rewrite {
            gp: 1,
            why: Refusal::GrMethod { method: 0x118, name, .. }
        })) if name.contains("LOAD_MME")
    );
    l.check("hostile_mme_refused_by_name", named, format!("{r:?}"));
    std::thread::sleep(std::time::Duration::from_millis(50));
    let gp_get = word(&walk, CHAN + USERD + kf_abi::submit::USERD_GP_GET)?;
    l.check(
        "hostile_entry_not_retired",
        gp_get == 1,
        format!("guest GP_GET={gp_get}"),
    );

    // ── run31's CE segment on a CE T-mode ring, with the inert software-subchannel rule ─────
    let host_ce = (0..10u32)
        .filter_map(kf_abi::submit::engine_type_copy)
        .find(|&et| rm.ce_is_grce(et) == Ok(false))
        .ok_or("no async copy engine")?;
    let g2 = Guest {
        walk: &walk,
        va: VA_CHAN,
        off: CHAN2,
        gp_get_cpu_stores: Cell::new(0),
    };
    let mut c0 = RUN31_CE.to_vec();
    c0.extend(release(P2));
    let c1 = m(5, 0x0100, &[0]);
    stage(&walk, CHAN2, VA_CHAN, &[&c0, &c1])?;
    let host2 = HostRing::on_engine_layout(
        &rm,
        space,
        host_ce,
        Some(region_base + kf_chan::host::RING_BYTES),
        kf_chan::host::TSPACE_LAYOUT,
    )?;
    let mut ring2 = TranslatedRing::new_tmode(VA_CHAN + GPFIFO, ENTRIES, 0);
    ring2.set_gr(GrConfig {
        tier: false,
        inert_sw_subch: true,
    });
    let mut chan2 = TranslatedChannel::new(ring2, host2, 2);
    chan2.set_tspace(win, false);
    put(&walk, CHAN2, 1)?;
    let t1 = std::time::Instant::now();
    let mut st2 = pump(&mut chan2, &g2).map_err(|e| format!("ce pump: {e:?}"))?;
    while !(st2 == Pumped::Caught && chan2.last_gp_get() == Some(1))
        && t1.elapsed() < std::time::Duration::from_secs(3)
    {
        let _ = ready_within(done.event_fd(), 100)?;
        st2 = pump(&mut chan2, &g2).map_err(|e| format!("ce pump: {e:?}"))?;
    }
    let sem2 = word(&walk, CHAN2 + SEM_CE)?;
    l.check(
        "ce_inert_bind_completes_on_engine",
        sem2 == P2 && chan2.last_gp_get() == Some(1) && chan2.gr_counts().1 == 1,
        format!(
            "sem={sem2:#x} want={P2:#x} gp_get={:?} inert_binds={}",
            chan2.last_gp_get(),
            chan2.gr_counts().1
        ),
    );
    put(&walk, CHAN2, 2)?;
    let r2 = pump(&mut chan2, &g2);
    l.check(
        "inert_subchannel_method_refused_by_name",
        matches!(
            r2,
            Err(kf_chan::host::ChanError::Ring(RingRefusal::Rewrite {
                gp: 1,
                why: Refusal::InertSubchannelMethod {
                    subch: 5,
                    method: 0x100,
                    value: 1
                }
            }))
        ),
        format!("{r2:?}"),
    );

    // ── release: channels, ring objects, mappings, CPU views ────────────────────────────────
    let mut freed = Vec::new();
    for c in [&mut chan, &mut chan2] {
        let ch = rm.free_channel(c.host().channel());
        let rel = c.release_host(&rm);
        freed.push(format!("{ch:?} {rel:?}"));
    }
    l.check(
        "resources_released",
        freed
            .iter()
            .all(|f| f.starts_with("Ok(())") && !f.contains("REFUSED")),
        freed.join(" | "),
    );
    Ok(())
}
