//! ★★★ **v3 gate 6 — a PASSTHROUGH channel on the GR engine: compute, on the mirrored VAS.**
//!
//! Gate 5 on `ENGINE_TYPE_GRAPHICS` with the family's COMPUTE object (host RM builds the GR
//! context), and the push CUDA itself uses for a small `cuMemcpyHtoD` on a real GA106
//! (`nvidia-gpu-passthrough/docs/reference/native_dataplane_cup2_ga106.md` §"15 dwords"): the
//! compute class's inline-to-memory unit with the payload as a LITERAL, then a four-word
//! `SET_REPORT_SEMAPHORE` release — on subchannel 1, as CUDA binds it. `THE_TRANSLATED_PLANE.md`
//! §9 step 7: GR on the mirrored VAS.
//!
//! The harness plays a guest USER process: its own page-table root, its GPFIFO / pushbuffer /
//! semaphore / USERD in the store at its own VAs, VIRTUAL operands only. v3 must:
//! - mirror the process's VA space by walking ITS root (the same walk + reconcile as the kernel's);
//! - birth the host twin over the guest's own GPFIFO VA and the guest's own USERD, at creation
//!   (RM zeroes the USERD it adopts — proved by poisoning it first);
//! - ring the doorbell INLINE from the trap (`Action::RingHostInline`) — no worker, no bit, no wake;
//! - never read the guest's pushbuffer: the engine fetches it, and writes the guest's `GP_GET`.
//! Completion is the guest's own business (it polls its semaphore), exactly as on bare metal.

use kf_abi::submit::{ENGINE_TYPE_GRAPHICS, SET_OBJECT, USERD_GP_GET, USERD_GP_PUT, gp_entry, method_header_inc};
use kf_chan::passthrough::{GuestChannel, UserdAt, birth};
use kf_cuda::abi::kf_format_ver2;
use kf_cuda::walk::{WalkCfg, WalkKernel};
use kf_harness::Ledger as Checks;
use kf_harness::tables::Tree;
use kf_host::HostRm;
use kf_linux_raw::DevDir;
use kf_harness::publish::publish;
use kf_mem::ledger::HostVas;
use kf_trap::{Action, Class, PrivRing, Route, RungBitmap, TokenWord, TrapPath, WakeWord};

const STORE_BYTES: u64 = 256 << 20;
const PT_BASE: u64 = 0x0100_0000;
const PT_BYTES: usize = 4 << 20;
/// The process's ring page(s): GPFIFO, USERD, semaphore, pushbuffer.
const U_MEM: u64 = 0x0300_0000;
const U_PAGES: u64 = 16;
const GPFIFO: u64 = 0x0;
const USERD: u64 = 0x1000;
const SEM: u64 = 0x2000;
const PB: u64 = 0x4000;
const PB_STRIDE: u64 = 0x100;
const ENTRIES: u32 = 64;
const SUBMITS: u32 = 32;
const DATA_A: u64 = 0x0400_0000;
const DATA_B: u64 = 0x0410_0000;
const CHUNK: u64 = 0x1000;
const CHUNKS: u64 = 16;
/// The process's VAs (below 2^40: a GP entry carries 40 address bits).
const VA_U: u64 = 0x10_0000_0000;
const VA_A: u64 = 0x10_4000_0000;
const VA_B: u64 = 0x10_8000_0000;
const TOKEN: u32 = 5;

fn main() {
    let mut l = Checks::default();
    if let Err(e) = run(&mut l) {
        l.check("run", false, e);
    }
    let v = l.verdict();
    println!("GATE6_VERDICT={}", if v { "PASS" } else { "FAIL" });
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
fn bytes(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|x| x.to_le_bytes()).collect()
}

#[allow(clippy::too_many_lines)]
fn run(l: &mut Checks) -> Result<(), String> {
    let dev = DevDir::open(c"/dev").map_err(|e| format!("open /dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes).map_err(|e| e.to_string())?;
    let gr_class = rm.compute_class_id().ok_or("this family declares no compute class")?;
    let res = rm.reserve_gpga(STORE_BYTES).map_err(|e| format!("reserve: {e:?}"))?;
    let store = res.handle;
    let fd = rm.export_to_new_fd(store).map_err(|e| format!("export: {e:?}"))?;
    let mut walk = WalkKernel::bring_up_on(WalkCfg::default(), kf_format_ver2(), kf_cuda::walk::WalkDevice::PciBusId(&rm.card().bdf())).map_err(|e| e.to_string())?;
    walk.import_store(fd.fd_number(), STORE_BYTES).map_err(|e| e.to_string())?;
    let w = |walk: &WalkKernel, off: u64, b: &[u8]| walk.write_store(off, b).map_err(|e| e.to_string());
    let rd = |walk: &WalkKernel, off: u64, n: usize| -> Result<Vec<u8>, String> {
        let mut b = vec![0u8; n];
        walk.read_store(off, &mut b).map_err(|e| e.to_string())?;
        Ok(b)
    };
    let word = |walk: &WalkKernel, off: u64| -> Result<u32, String> {
        let b = rd(walk, off, 4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };

    // ── the guest process: its own tables, data and ring memory ─────────────────────────────
    let mut tree = Tree::new(PT_BASE, PT_BYTES);
    for p in 0..U_PAGES {
        tree.map4k(VA_U + p * 4096, U_MEM + p * 4096);
    }
    for p in 0..CHUNKS {
        tree.map4k(VA_A + p * 4096, DATA_A + p * 4096);
        tree.map4k(VA_B + p * 4096, DATA_B + p * 4096);
    }
    w(&walk, PT_BASE, &tree.img.mem)?;
    let a: Vec<u8> = (0..(CHUNK * CHUNKS / 4) as u32).flat_map(|i| (0xA11C_0000 ^ i).to_le_bytes()).collect();
    w(&walk, DATA_A, &a)?;
    w(&walk, DATA_B, &vec![0u8; (CHUNK * CHUNKS) as usize])?;
    w(&walk, U_MEM, &vec![0u8; (U_PAGES * 4096) as usize])?;
    // Poison the USERD: if the host twin adopts THIS block, RM's birth zeroes it.
    w(&walk, U_MEM + USERD, &vec![0x5Au8; 512])?;

    // ── v3: mirror the process's VA space, then birth its twin over ITS ring and USERD ────────
    let space = rm.alloc_vaspace().map_err(|e| format!("vaspace: {e:?}"))?;
    let target = HostVas { rm: &rm, space, store, ram_obj: None };
    let pubd = publish(&mut walk, 0, tree.root, &target, STORE_BYTES, &|_, _| None)?;
    let ap = &pubd.applied;
    l.check("process_vas_mirrored", ap.refused == 0 && ap.mapped >= 1, format!("diff_runs={} mapped={} refused={}", pubd.runs, ap.mapped, ap.refused));

    let t0 = std::time::Instant::now();
    let chan = birth(&rm, space, GuestChannel {
        gpfifo_va: VA_U + GPFIFO,
        entries: ENTRIES,
        userd: UserdAt::Store { store, off: U_MEM + USERD },
        engine: ENGINE_TYPE_GRAPHICS,
        err_ctx: 0,
    })?;
    l.measure("birth", format!("token={:#x} us={}", chan.token, t0.elapsed().as_micros()));
    let userd_after = rd(&walk, U_MEM + USERD, 512)?;
    l.check("rm_adopted_the_guest_userd", userd_after.iter().all(|&b| b == 0), "the poisoned block is zeroed ⇒ it IS the twin's USERD");

    // ── the doorbell plane: a PASSTHROUGH token rings inline ─────────────────────────────────
    let tokens: Vec<TokenWord> = (0..64).map(|_| TokenWord::new()).collect();
    if !tokens[TOKEN as usize].allocate_fresh(Route::Passthrough, chan.token) {
        return Err("token not fresh".into());
    }
    let (bits, wake, dwake, ring) = (RungBitmap::new(), WakeWord::new(), WakeWord::new(), PrivRing::new());
    let trap = TrapPath {
        tokens: &tokens,
        bits: &bits,
        worker_wake: &wake,
        drainer_wake: &dwake,
        ring: &ring,
        token_mask: 63,
        timer: kf_trap::timer::TIMER_GV100,
    };

    // ── the guest: write a segment, a GP entry, GP_PUT; ring; poll ITS semaphore ────────────
    let mut lat = Vec::new();
    let mut not_inline = 0;
    // What the guest writes: 16 literal dwords per submission, into VA_B + k·64.
    let lit = |k: u32, i: u32| 0x6E00_0000 | (k << 8) | i;
    for k in 0..SUBMITS {
        let dst = VA_B + u64::from(k) * 64;
        let mut seg = if k == 0 { m(1, SET_OBJECT, &[gr_class]) } else { Vec::new() };
        seg.extend(m(1, 0x188, &[hi(dst), lo(dst)])); // OFFSET_OUT_UPPER, OFFSET_OUT
        seg.extend(m(1, 0x180, &[64, 1])); // LINE_LENGTH_IN = 64 bytes, LINE_COUNT = 1
        seg.extend(m(1, 0x1b0, &[0x41])); // LAUNCH_DMA: PITCH, FLUSH_DISABLE, SYSMEMBAR_DISABLE — CUDA's value
        let data: Vec<u32> = (0..16).map(|i| lit(k, i)).collect();
        seg.push(kf_abi::submit::method_header_non_inc(1, 0x1b4, 16).ok_or("noninc header")?); // LOAD_INLINE_DATA
        seg.extend(&data);
        seg.extend(m(1, 0x1b00, &[hi(VA_U + SEM), lo(VA_U + SEM), k + 1, 0])); // REPORT_SEMAPHORE A-D: release, 4 words
        let at = PB + u64::from(k) * PB_STRIDE;
        w(&walk, U_MEM + at, &bytes(&seg))?;
        let e = gp_entry(VA_U + at, 4 * seg.len() as u64).ok_or("gp entry")?;
        w(&walk, U_MEM + GPFIFO + 8 * u64::from(k % ENTRIES), &e.to_le_bytes())?;
        w(&walk, U_MEM + USERD + USERD_GP_PUT, &((k + 1) % ENTRIES).to_le_bytes())?;
        let t = std::time::Instant::now();
        match trap.write(Class::Doorbell, 0, 0x90, u64::from(TOKEN), 4) {
            Action::RingHostInline { host_token } => rm.doorbell(host_token).map_err(|e| format!("doorbell: {e:?}"))?,
            _ => not_inline += 1,
        }
        let deadline = t + std::time::Duration::from_secs(2);
        while word(&walk, U_MEM + SEM)? != k + 1 {
            if std::time::Instant::now() > deadline {
                return Err(format!("submission {k}: semaphore never reached {} (sem={:#x} gp_get={})", k + 1, word(&walk, U_MEM + SEM)?, word(&walk, U_MEM + USERD + USERD_GP_GET)?));
            }
        }
        lat.push(t.elapsed().as_micros());
    }
    lat.sort_unstable();
    let pct = |p: usize| lat.get((lat.len() - 1) * p / 100).copied().unwrap_or(0);
    l.check("every_ring_was_inline", not_inline == 0, format!("{not_inline} of {SUBMITS} took another arm"));
    let want: Vec<u8> = (0..SUBMITS).flat_map(|k| (0..16).flat_map(move |i| lit(k, i).to_le_bytes())).collect();
    l.check("gr_ran_the_guest_ring_unparsed", rd(&walk, DATA_B, want.len())? == want, format!("{SUBMITS} I2M literal writes through the mirrored process VAS"));
    let gp_get = word(&walk, U_MEM + USERD + USERD_GP_GET)?;
    // ★ 2026-09-26: only a family whose engine writes USERD GP_GET can be held to it — Blackwell's
    // channel classes have no such word (`kf_chip::Family::engine_writes_userd_gp_get`).
    let (arch, implementation, _) = rm.arch_info();
    let family = kf_chip::Family::from_arch(arch, implementation).map_err(|e| format!("{e:?}"))?;
    if family.engine_writes_userd_gp_get() {
        l.check("hardware_advanced_the_guest_gp_get", gp_get == SUBMITS % ENTRIES, format!("guest USERD GP_GET={gp_get} (written by the engine, not by us)"));
    } else {
        l.measure("hardware_advanced_the_guest_gp_get", format!("NOT A FACT ON {family:?}: the channel class has no USERD GP_GET (read {gp_get}); every semaphore above is the completion"));
    }
    let mut scan = Vec::new();
    l.check("no_worker_ever_saw_the_token", bits.scan(&mut scan, 64) == 0 && wake.seen() == 0, format!("bits={scan:?} wake_seq={}", wake.seen()));
    l.measure("ring_to_semaphore_us", format!("p50={} p90={} max={} (includes the harness's DtoH poll)", pct(50), pct(90), pct(100)));
    let _ = &mut walk;
    Ok(())
}
