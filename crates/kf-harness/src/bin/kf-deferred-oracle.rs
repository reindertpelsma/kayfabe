//! NV50_DEFERRED_API (class 0x5080) ORACLE — a NATIVE RM client exercising the deferred API for
//! real on the host GPU, from a GR/compute-capable channel (the falsification probe's CE channel
//! could not bind the class: `traces/deferred_api_falsify_20261007/README.md`). Owner direction,
//! 2026-10-07: kayfabe keeps class 5080 Translated-only, but a Linux client CAN use it, so this is
//! the ORACLE kayfabe's Translated handling must match — asserting PASS on real hardware now,
//! and (later, unbuilt here) through kayfabe.
//!
//! **DEFAULT OFF.** Nothing runs unless `KF_DEFERRED_ORACLE=1`. This never runs as part of `cargo
//! test`; it is a hardware tool, run by hand on the owner's box (`AGENTS.md` box rules).
//!
//! ## Why this is its own ring, not `kf_chan::host::HostRing`
//!
//! Any `HostRing` built on `ENGINE_TYPE_GRAPHICS` sets a real GR context and then routes every
//! `push()` through `route_graphics_ce`, which authors `SET_OBJECT` only for a class in its
//! GR-tier allowlist or a DMA-copy class (`kf-chan/src/host.rs`) — class `0x5080` is neither, and
//! that router is the OTHER agent's branch's to extend (`claude/deferred-translated-20261007`,
//! kf-rm/kf-chan). So this binary builds its own minimal single-slot ring directly on `kf_host`'s
//! public API (the same primitives `HostRing` itself is built from: `birth_channel`, `map`,
//! `arm_cpu_view`, `doorbell`), reusing `kf_chan`'s pure, non-RM helpers (`LEGACY_LAYOUT`,
//! `fence_words`, `ring_gp_entry` — logic, not routing) where that avoids re-deriving hardware
//! encodings. **No `kf-chan`/`kf-rm` production code is read-written here.**
//!
//! ## The subchannel correction
//!
//! The falsification probe bound the 5080 object on subchannel 1. OWNER_RULINGS.md §S item 4
//! and the real driver both reserve subchannels 5-7 for software classes
//! (`kf-chan/src/host.rs`'s own GR-tier comment). This oracle binds on subchannel 5
//! (`SUBCH_SW`). Whether subchannel-numbering or runlist membership (or both) explains the
//! probe's Xid 32 is exactly what a GR-channel run now measures.
//!
//! ## Falsifiers, stated before the first run
//!
//! - **P0** (positive control): `SET_OBJECT(0x5080)` on `SUBCH_SW` of this GR channel completes
//!   its fence (`bind_fence_ok=true`); `DMA_INVALIDATE_TLB` registers, `0x200` triggers it, and
//!   the re-register probe shows it executed. FALSIFIER: a bind-fence timeout/Xid, or the
//!   re-register probe showing the entry untouched, falsifies "a GR channel is sufficient" (or
//!   "subchannel 5-7 is sufficient") and leaves every later phase UNMEASURED, exactly as the CE
//!   run did.
//! - **EIGHT**: each of the eight commands registers (inner `cmd` is never checked at
//!   registration — ogkm `deferred_api.c`); trigger status matches
//!   `kf_harness::deferred_model::expected_status(cmd, who)` for the privilege this process
//!   actually runs at (`UserRoot` for `euid==0`, else `User`). FALSIFIER: a PRIVILEGED command
//!   succeeding as non-root, or `GPU_EVICT_CTX` succeeding as root (root is `UserRoot`, not
//!   `Kernel`) — either would be a privilege-gate escape.
//! - **F1/F2/F3** (cross-object/channel/client): an entry on object A is NOT executed by `0x200`
//!   fired on B / another channel / another client. FALSIFIER: A reads "executed" after any of
//!   these.
//! - **F5** (garbage handles): unregistered handles never execute; blast radius is the firing
//!   channel only. FALSIFIER: a bystander client/channel dies, or an MMU fault reaches
//!   `nvidia-uvm`.
//! - **F6** (rate): a tight loop of valid `DMA_INVALIDATE_TLB` (EXPLICIT-delete) triggers does not
//!   degrade a concurrent well-behaved channel disproportionately more than an equal rate of
//!   ordinary semaphore-release methods, both against a no-contention baseline. Wall-clock capped
//!   (owner instruction, 2026-10-07): each leg of F6 runs for `KF_DF2_F6_MS` (default 1500 ms),
//!   never looped past that.
//!
//! Every phase prints `DF2_<NAME>=...`. MEASURED vs INFERRED is stated in-line per command: the
//! four `NON_PRIVILEGED` commands get LEGAL parameters (the client's own VA space for
//! `DMA_INVALIDATE_TLB`; best-effort shaped-but-unverified params naming this client/channel for
//! the three GR binds — no real zcull/PM/preemption context was brought up, so a non-`NV_OK`
//! status for those three is NOT evidence against the privilege gate). The four
//! `PRIVILEGED`/kernel-only commands are measured ONLY for the privilege gate (zeroed params): a
//! non-root run proves the gate refuses before touching parameters; a root run reaching the
//! parameter-dependent handler is reported honestly as "gate passed, operational result out of
//! this run's scope" rather than claimed as a verified real effect.

use kf_abi::submit::{SET_OBJECT, USERD_GP_PUT, method_header_inc};
use kf_chan::host::LEGACY_LAYOUT;
use kf_chan::tspace_unsafe::{fence_words, ring_gp_entry};
use kf_harness::deferred_model::{self as model, Privilege};
use kf_host::{HostRm, RmError, VaSpace};
use kf_linux_raw::DevDir;
use kf_linux_raw::HostOffset as At;

// ─── RM status codes (ogkm-580 nvstatuscodes.h) — same as the falsify probe ───
const NV_ERR_INVALID_OBJECT_HANDLE: u32 = 0x33;
const NV_ERR_INVALID_PARAM_STRUCT: u32 = 0x3A;

const NV50_DEFERRED_API_CLASS: u32 = 0x5080;
const NV5080_CTRL_CMD_DEFERRED_API_V2: u32 = 0x5080_0103;
const NV5080_CTRL_CMD_REMOVE_API: u32 = 0x5080_0102;
const DEFERRED_V2_SIZE: usize = 584;
const BUNDLE_OFF: usize = 24;

const SW_TRIGGER_METHOD: u32 = 0x200;
/// Software classes live on subchannels 5-7 (OWNER_RULINGS.md §S item 4; `kf-chan/src/host.rs`'s
/// GR-tier comment). The falsify probe used subchannel 1; this is the correction.
const SUBCH_SW: u32 = 5;
const SUBCH_SW_B: u32 = 6;
/// The GR/compute object itself, hardware convention.
const SUBCH_GR: u32 = 0;

fn status_of(e: &RmError) -> Option<u32> {
    match e {
        RmError::Other(s) => Some(*s),
        RmError::InsufficientPermissions => Some(0x1B),
        _ => None,
    }
}

fn defer_params(h: u32, cmd: u32, flags: u32, extra: &[(usize, u32)]) -> [u8; DEFERRED_V2_SIZE] {
    let mut p = [0u8; DEFERRED_V2_SIZE];
    p[0..4].copy_from_slice(&h.to_le_bytes());
    p[4..8].copy_from_slice(&cmd.to_le_bytes());
    p[8..12].copy_from_slice(&flags.to_le_bytes());
    for &(off, val) in extra {
        p[BUNDLE_OFF + off..BUNDLE_OFF + off + 4].copy_from_slice(&val.to_le_bytes());
    }
    p
}

/// No `unsafe` needed: `/proc/self/status`'s `Uid:` line gives real/effective/saved/fs uid as
/// plain text (same technique `kf-deferred-falsify.rs` uses).
fn euid() -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| {
                l.strip_prefix("Uid:")
                    .and_then(|r| r.split_whitespace().nth(1))
                    .and_then(|v| v.parse::<u32>().ok())
            })
        })
        .unwrap_or(u32::MAX)
}

fn caller_privilege() -> Privilege {
    if euid() == 0 {
        Privilege::UserRoot
    } else {
        Privilege::User
    }
}

/// One host RM client with a VA space and a single-slot GR-runlist ring it owns directly (no
/// `kf_chan` routing — see the module docs).
struct GrUnit {
    rm: HostRm,
    space: VaSpace,
    cpu: kf_linux_raw::VolatileRegion,
    _node: kf_linux_raw::CharDevice,
    cookie: u64,
    mem: u32,
    maps: Vec<u64>,
    va: u64,
    chan: kf_host::Channel,
    put: u32,
    seq: u32,
    /// `(handle, class)` of the compute object this ring's GR context runs. Kept for the
    /// lifetime of the channel (freed with it); read only at construction.
    _compute: (u32, u32),
    /// `(handle, class_engine, class_id, engine_id)` of the one 5080 object this ring owns.
    defapi: Option<(u32, u32, u32, u32)>,
}

const RING_BYTES: u64 = kf_chan::host::RING_BYTES;
const GPFIFO_ENTRIES: u32 = 512;

impl GrUnit {
    fn open() -> Result<GrUnit, String> {
        let dev = DevDir::open(c"/dev").map_err(|e| format!("dev: {e:?}"))?;
        let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
            .map_err(|e| e.to_string())?;
        let space = rm.alloc_vaspace_bare().map_err(|e| format!("vaspace: {e:?}"))?;
        let mem = rm
            .alloc_device_local(RING_BYTES)
            .map_err(|e| format!("ring obj: {e:?}"))?;
        let va = rm
            .map(space, mem, kf_host::MapBacking::Dedicated, 0, RING_BYTES, None, false)
            .map_err(|e| {
                let _ = rm.free(mem);
                format!("map ring: {e:?}")
            })?;
        let undo_map = |rm: &HostRm, va: u64, mem: u32| {
            let _ = rm.unmap(space, va, false);
            let _ = rm.free(mem);
        };
        let (node, cookie) = match rm.arm_cpu_view(
            kf_host::MapNode::Gpu,
            mem,
            0,
            RING_BYTES,
            kf_host::ViewAccess::ReadWrite,
        ) {
            Ok(v) => v,
            Err(e) => {
                undo_map(&rm, va, mem);
                return Err(format!("cpu view: {e:?}"));
            }
        };
        let cpu = match kf_linux_raw::VolatileRegion::map(
            kf_linux_raw::Backing::DeviceFile { fd: node.as_fd() },
            RING_BYTES,
            kf_linux_raw::CachePolicy::Uncached,
            kf_linux_raw::HostPageSize::query(),
        ) {
            Ok(r) => r,
            Err(e) => {
                let _ = rm.release_cpu_view(kf_host::CpuViewRelease {
                    h_memory: mem,
                    p_linear_address: cookie,
                });
                undo_map(&rm, va, mem);
                return Err(format!("cpu mmap: {e:?}"));
            }
        };
        let chan = match rm.birth_channel(
            space,
            kf_abi::submit::ENGINE_TYPE_GRAPHICS,
            kf_host::RingSpec {
                gp_fifo_va: va + LEGACY_LAYOUT.gpfifo_off,
                gp_fifo_entries: GPFIFO_ENTRIES,
                userd_memory: mem,
                userd_offset: LEGACY_LAYOUT.userd_off,
                err_notifier: 0,
            },
        ) {
            Ok(c) => c,
            Err(e) => {
                drop(cpu);
                let _ = rm.release_cpu_view(kf_host::CpuViewRelease {
                    h_memory: mem,
                    p_linear_address: cookie,
                });
                undo_map(&rm, va, mem);
                return Err(format!("birth GR channel: {e:?}"));
            }
        };
        let compute = match rm.alloc_compute_object(chan) {
            Ok(c) => c,
            Err(e) => {
                let _ = rm.free_channel(chan);
                drop(cpu);
                let _ = rm.release_cpu_view(kf_host::CpuViewRelease {
                    h_memory: mem,
                    p_linear_address: cookie,
                });
                undo_map(&rm, va, mem);
                return Err(format!("compute object: {e:?}"));
            }
        };
        let mut u = GrUnit {
            rm,
            space,
            cpu,
            _node: node,
            cookie,
            mem,
            maps: vec![va],
            va,
            chan,
            put: 0,
            seq: 0,
            _compute: compute,
            defapi: None,
        };
        // Schedule the channel onto its runlist FIRST — an unscheduled channel's PBDMA never
        // fetches a GPFIFO entry at all, doorbell or not. (Measured 2026-10-07 run 1: submitting
        // before scheduling just timed out with no Xid, nothing fetched — a harness bug, not a
        // hardware refusal; fixed here.)
        if let Err(e) = u.rm.schedule(chan) {
            u.free();
            return Err(format!("schedule: {e:?}"));
        }
        // ⊘ Measured 2026-10-07 run 2: binding the compute class on subchannel 0 here (before
        // touching the 5080 object at all) faulted with Xid 13 "Graphics Exception: Class 0xc9c0
        // Subchannel 0x0 Mismatch" (self-harm only — the bystander desktop and GPU state were
        // unaffected; see `traces/` for the full line). That bind was this binary's own
        // precaution, not something the task needs: `birth_channel` already typed the channel
        // `ENGINE_TYPE_GRAPHICS`, which is what puts it on the GR runlist. So it is skipped,
        // default off behind `KF_DF2_BIND_COMPUTE=1`, and the 5080 bind is tried on a channel
        // whose subchannel 0 was never touched by any `SET_OBJECT` at all.
        if std::env::var_os("KF_DF2_BIND_COMPUTE").as_deref() == Some(std::ffi::OsStr::new("1")) {
            let hdr = method_header_inc(SUBCH_GR, SET_OBJECT, 1).ok_or("SET_OBJECT header")?;
            if let Err(e) = u.submit(&[hdr, compute.1]) {
                u.free();
                return Err(format!("bind compute object: {e}"));
            }
        }
        Ok(u)
    }

    /// Allocate the one 5080 object this ring will use, and read its class/engine back.
    fn alloc_5080(&mut self) -> Result<(u32, u32, u32, u32), String> {
        let want = self.rm.mint();
        let handle = self
            .rm
            .raw_alloc(self.chan.chan, want, NV50_DEFERRED_API_CLASS, None, &mut [0u8; 0])
            .map_err(|e| format!("alloc 5080: {e:?}"))?;
        self.rm.remember(handle, self.chan.chan);
        let mut p = [0u8; 16];
        p[0..4].copy_from_slice(&handle.to_le_bytes());
        self.rm
            .raw_control_native(self.chan.chan, kf_host::channel::NV906F_CTRL_GET_CLASS_ENGINEID, &mut p)
            .map_err(|e| format!("class_engine_id: {e:?}"))?;
        let class_engine = u32::from_le_bytes([p[4], p[5], p[6], p[7]]);
        let class_id = u32::from_le_bytes([p[8], p[9], p[10], p[11]]);
        let engine_id = u32::from_le_bytes([p[12], p[13], p[14], p[15]]);
        let info = (handle, class_engine, class_id, engine_id);
        self.defapi = Some(info);
        Ok(info)
    }

    fn bind_5080(&mut self, subch: u32) -> Result<(), String> {
        let (_, class_engine, ..) = self.defapi.ok_or("no 5080 object allocated")?;
        let hdr = method_header_inc(subch, SET_OBJECT, 1).ok_or("SET_OBJECT header")?;
        self.submit(&[hdr, class_engine])
    }

    fn fire(&mut self, subch: u32, data: u32) -> Result<(), String> {
        let hdr = method_header_inc(subch, SW_TRIGGER_METHOD, 1).ok_or("0x200 header")?;
        self.submit(&[hdr, data])
    }

    /// Store `words` at pushbuffer offset 0, queue the one GPFIFO entry, ring the doorbell, and
    /// wait (bounded) for the fence this call appends. Strictly serial: never call this again
    /// before it returns.
    fn submit(&mut self, words: &[u32]) -> Result<(), String> {
        let seq = self.seq.wrapping_add(1);
        let fence_va = self.va + LEGACY_LAYOUT.fence_off;
        let mut all = words.to_vec();
        all.extend(fence_words(fence_va, seq).ok_or("fence words: bad VA")?);
        let n = 4u64 * all.len() as u64;
        if n > LEGACY_LAYOUT.pb_bytes {
            return Err("submission exceeds the pushbuffer".into());
        }
        for (i, w) in all.iter().enumerate() {
            self.cpu
                .store_u32(At::new(4 * i as u64), *w)
                .map_err(|e| format!("pb store: {e:?}"))?;
        }
        let entry =
            ring_gp_entry(self.va, LEGACY_LAYOUT.pb_bytes, 0, n).ok_or("gp entry encode")?;
        let gp_off = LEGACY_LAYOUT.gpfifo_off + 8 * u64::from(self.put % GPFIFO_ENTRIES);
        self.cpu
            .store_u32(At::new(gp_off), entry as u32)
            .map_err(|e| format!("gpfifo lo: {e:?}"))?;
        self.cpu
            .store_u32(At::new(gp_off + 4), (entry >> 32) as u32)
            .map_err(|e| format!("gpfifo hi: {e:?}"))?;
        self.put = self.put.wrapping_add(1);
        self.cpu
            .store_u32(At::new(LEGACY_LAYOUT.userd_off + USERD_GP_PUT), self.put)
            .map_err(|e| format!("gp_put: {e:?}"))?;
        self.rm
            .doorbell(self.chan.token)
            .map_err(|e| format!("doorbell: {e:?}"))?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let got = self
                .cpu
                .load_u32(At::new(LEGACY_LAYOUT.fence_off))
                .map_err(|e| format!("fence load: {e:?}"))?;
            if got == seq {
                self.seq = seq;
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(format!("fence timeout want={seq} got={got}"));
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    fn register(&self, h: u32, cmd: u32, flags: u32, extra: &[(usize, u32)]) -> Result<(), RmError> {
        let (obj, ..) = self.defapi.ok_or(RmError::Other(0xFFFF_FFFE))?;
        let mut p = defer_params(h, cmd, flags, extra);
        self.rm.raw_control_native(obj, NV5080_CTRL_CMD_DEFERRED_API_V2, &mut p)
    }

    fn remove(&self, h: u32) -> Result<(), RmError> {
        let (obj, ..) = match self.defapi {
            Some(d) => d,
            None => return Err(RmError::Other(0xFFFF_FFFE)),
        };
        let mut p = h.to_le_bytes();
        self.rm.raw_control_native(obj, NV5080_CTRL_CMD_REMOVE_API, &mut p)
    }

    /// Non-destructive: `Ok(true)` if `h` is still registered on this ring's 5080 object.
    fn is_registered(&self, h: u32) -> Result<bool, String> {
        match self.register(h, model::CMD_DMA_INVALIDATE_TLB, model::FLAGS_DELETE_EXPLICIT, &[]) {
            Err(e) if status_of(&e) == Some(NV_ERR_INVALID_OBJECT_HANDLE) => Ok(true),
            Ok(()) => {
                let _ = self.remove(h);
                Ok(false)
            }
            Err(e) => Err(format!("probe register: {e:?} (status {:?})", status_of(&e))),
        }
    }

    fn free(self) {
        // `VolatileRegion` has no `Drop` unmap of its own (short-lived diagnostic process; the
        // mapping goes away at exit either way) — order here matters only for the RM calls, which
        // follow `kf_chan::host::HostRing::release`'s own order: CPU-view release, GPU unmap, free.
        let _ = self.rm.free_channel(self.chan);
        let _ = self.rm.release_cpu_view(kf_host::CpuViewRelease {
            h_memory: self.mem,
            p_linear_address: self.cookie,
        });
        for &m in &self.maps {
            let _ = self.rm.unmap(self.space, m, false);
        }
        let _ = self.rm.free(self.mem);
        self.rm.free_vaspace(self.space);
    }
}

fn main() {
    if std::env::var_os("KF_DEFERRED_ORACLE").as_deref() != Some(std::ffi::OsStr::new("1")) {
        println!(
            "DF2_RESULT=OFF: set KF_DEFERRED_ORACLE=1 to run this hardware oracle (default off; \
             see the module docs for the falsifiers stated before the first run)"
        );
        return;
    }
    let phase = std::env::var("KF_DF2_PHASE").unwrap_or_else(|_| "p0".into());
    let who = caller_privilege();
    println!(
        "DF2_START euid={} privilege={who:?} phase={phase} subch_sw={SUBCH_SW}",
        euid()
    );
    let phases: Vec<&str> = if phase == "all" {
        vec!["p0", "eight", "f1", "f2", "f3", "f5", "f6"]
    } else {
        phase.split(',').collect()
    };
    let mut any_fail = false;
    for p in phases {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_phase(p, who)));
        match r {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                println!("DF2_{}=BLOCKED {e}", p.to_uppercase());
                any_fail = true;
            }
            Err(_) => {
                println!("DF2_{}=PANIC", p.to_uppercase());
                any_fail = true;
            }
        }
    }
    println!("DF2_DONE fail={any_fail}");
}

fn run_phase(p: &str, who: Privilege) -> Result<(), String> {
    match p {
        "p0" => p0(),
        "eight" => eight(who),
        "f1" => f1(),
        "f2" => f2(),
        "f3" => f3(),
        "f5" => f5(),
        "f6" => f6(),
        "f3-child" => f3_child(),
        other => Err(format!("unknown phase {other}")),
    }
}

/// P0 — positive control on a GR channel, subchannel 5.
fn p0() -> Result<(), String> {
    let mut u = GrUnit::open()?;
    let (handle, class_engine, class_id, engine_id) = u.alloc_5080()?;
    println!(
        "DF2_P0_CLASSENGINE object={handle:#x} class_engine={class_engine:#x} \
         class_id={class_id:#x} engine_id={engine_id:#x} subch={SUBCH_SW}"
    );
    if let Err(e) = u.bind_5080(SUBCH_SW) {
        u.free();
        return Err(format!(
            "SET_OBJECT(5080) on subch {SUBCH_SW} of a GR channel: {e} — bind_fence_ok=false"
        ));
    }
    println!("DF2_P0_SETOBJECT bind_fence_ok=true");
    let h = 0x5252_0001;
    match u.register(h, model::CMD_DMA_INVALIDATE_TLB, model::FLAGS_DELETE_IMPLICIT, &[(
        4,
        u.space.space,
    )]) {
        Ok(()) => {}
        Err(e) if status_of(&e) == Some(NV_ERR_INVALID_PARAM_STRUCT) => {
            u.free();
            return Err(format!("register INVALID_PARAM_STRUCT: DEFERRED_V2_SIZE={DEFERRED_V2_SIZE} wrong"));
        }
        Err(e) => {
            u.free();
            return Err(format!("register failed: {e:?} status={:?}", status_of(&e)));
        }
    }
    let dup = u.register(h, model::CMD_DMA_INVALIDATE_TLB, model::FLAGS_DELETE_IMPLICIT, &[]);
    let dup_ok = matches!(&dup, Err(e) if status_of(e) == Some(NV_ERR_INVALID_OBJECT_HANDLE));
    println!("DF2_P0_OBSERVABLE dup_reject={dup_ok}");
    if let Err(e) = u.fire(SUBCH_SW, h) {
        u.free();
        return Err(format!("0x200 trigger did not complete: {e}"));
    }
    let executed = !u.is_registered(h)?;
    u.free();
    println!(
        "DF2_P0={} evidence=same-object 0x200(h) executed={executed} dup_reject={dup_ok}",
        if executed && dup_ok { "PASS(positive-control-fires)" } else { "FAIL(trigger-did-not-fire)" }
    );
    if executed && dup_ok {
        Ok(())
    } else {
        Err("positive control did not fire; every later phase is UNMEASURED".into())
    }
}

/// The eight commands + one unknown: register, trigger, observe — at this process's real
/// privilege. Only `DMA_INVALIDATE_TLB` gets LEGAL parameters (this client's own VA space);
/// every other command gets ZEROED parameters — a gate-only measurement for the three GR binds
/// (`NON_PRIVILEGED`: a non-OK status is not evidence against the gate, since the gate is
/// already open) and for the four `PRIVILEGED`/kernel-only commands (a non-root run proves the
/// gate refuses before any parameter is touched; a root run reaching the handler is reported as
/// "gate passed, operational effect out of this run's scope" — see the module docs).
fn eight(who: Privilege) -> Result<(), String> {
    let mut u = GrUnit::open()?;
    u.alloc_5080()?;
    u.bind_5080(SUBCH_SW)?;
    let va_extra = vec![(4usize, u.space.space)];
    for cmd in model::EIGHT.iter().copied().chain([0x2080_dead]) {
        let name = model::cmd_name(cmd);
        let h = 0x5252_1000 + (cmd & 0xff);
        let extra: Vec<(usize, u32)> = if cmd == model::CMD_DMA_INVALIDATE_TLB {
            va_extra.clone()
        } else {
            Vec::new()
        };
        let reg = u.register(h, cmd, model::FLAGS_DELETE_IMPLICIT, &extra);
        let reg_status = reg.as_ref().err().and_then(status_of);
        println!("DF2_EIGHT_REG cmd={name} euid={} register={:?} status={reg_status:?}", euid(), reg.is_ok());
        if reg.is_err() {
            continue;
        }
        let fired = u.fire(SUBCH_SW, h);
        match fired {
            Ok(()) => {
                let consumed = !u.is_registered(h)?;
                let want = model::expected_status(cmd, who);
                println!(
                    "DF2_EIGHT_TRIGGER cmd={name} who={who:?} consumed={consumed} want_status={want:#x} \
                     note={}",
                    if cmd == model::CMD_DMA_INVALIDATE_TLB {
                        "LEGAL params (own VA space) — real effect"
                    } else if matches!(
                        cmd,
                        model::CMD_GR_CTXSW_ZCULL_BIND
                            | model::CMD_GR_CTXSW_PM_BIND
                            | model::CMD_GR_CTXSW_PREEMPTION_BIND
                    ) {
                        "best-effort params, no real zcull/PM/preemption context — a non-OK status \
                         here is NOT evidence against the privilege gate"
                    } else if cmd == 0x2080_dead {
                        "unknown command"
                    } else {
                        "zeroed params — gate-only: a root PASS here means the GATE passed, the \
                         operational effect is OUT OF SCOPE (no legal GPU_PROMOTE_CTX/INITIALIZE_CTX \
                         /FIFO_UPDATE_CHANNEL_INFO params were constructed)"
                    }
                );
                if !consumed {
                    let _ = u.remove(h);
                }
            }
            Err(e) => {
                println!("DF2_EIGHT_TRIGGER cmd={name} who={who:?} BLOCKED={e}");
                let _ = u.remove(h);
            }
        }
    }
    u.free();
    Ok(())
}

/// F1 — entry on object A not run by 0x200 on object B (same channel/client).
fn f1() -> Result<(), String> {
    let mut u = GrUnit::open()?;
    let a = u.alloc_5080()?;
    u.bind_5080(SUBCH_SW)?;
    // A second 5080 object, bound on the OTHER software subchannel of the SAME channel.
    let want = u.rm.mint();
    let b_handle = u
        .rm
        .raw_alloc(u.chan.chan, want, NV50_DEFERRED_API_CLASS, None, &mut [0u8; 0])
        .map_err(|e| format!("alloc B: {e:?}"))?;
    u.rm.remember(b_handle, u.chan.chan);
    let hdr = method_header_inc(SUBCH_SW_B, SET_OBJECT, 1).ok_or("SET_OBJECT header")?;
    u.submit(&[hdr, a.1])?; // same class_engine value binds either object's class on hw
    let h = 0x5252_2001;
    u.register(h, model::CMD_DMA_INVALIDATE_TLB, model::FLAGS_DELETE_IMPLICIT, &[])
        .map_err(|e| format!("register on A: {e:?}"))?;
    u.fire(SUBCH_SW_B, h)?; // fire on B's subchannel
    let still = u.is_registered(h)?;
    if !still {
        let _ = u.remove(h);
    }
    u.free();
    println!(
        "DF2_F1={} evidence=A_untouched_after_0x200_on_B still_registered={still}",
        if still { "PASS" } else { "FAIL(cross-object execution)" }
    );
    if still { Ok(()) } else { Err("F1 falsified".into()) }
}

/// F2 — entry on object A (channel C1) not run by 0x200 fired from channel C2's own ring.
fn f2() -> Result<(), String> {
    let mut a_unit = GrUnit::open()?;
    let a = a_unit.alloc_5080()?;
    a_unit.bind_5080(SUBCH_SW)?;
    let h = 0x5252_3001;
    a_unit
        .register(h, model::CMD_DMA_INVALIDATE_TLB, model::FLAGS_DELETE_IMPLICIT, &[])
        .map_err(|e| format!("register on A: {e:?}"))?;

    let mut b_unit = GrUnit::open()?;
    let _ = b_unit.alloc_5080()?;
    b_unit.bind_5080(SUBCH_SW)?;
    // Fire the SAME handle number on B's own object/channel — B has no entry h, so this also
    // exercises "unknown on B", but the property under test is A's isolation, checked below.
    let fired = b_unit.fire(SUBCH_SW, h);
    b_unit.free();
    if let Err(e) = fired {
        a_unit.free();
        return Err(format!("fire on B: {e}"));
    }
    let still = a_unit.is_registered(h)?;
    if !still {
        let _ = a_unit.remove(h);
    }
    let _ = a; // keep alive for clarity; not otherwise used
    a_unit.free();
    println!(
        "DF2_F2={} evidence=A_untouched_after_0x200_on_other_channel still_registered={still}",
        if still { "PASS" } else { "FAIL(cross-channel execution)" }
    );
    if still { Ok(()) } else { Err("F2 falsified".into()) }
}

/// F3 — entry on A (process 1's object) not run by 0x200 fired by a SECOND process.
fn f3() -> Result<(), String> {
    let mut u = GrUnit::open()?;
    u.alloc_5080()?;
    u.bind_5080(SUBCH_SW)?;
    let h = 0x5252_4001;
    u.register(h, model::CMD_DMA_INVALIDATE_TLB, model::FLAGS_DELETE_IMPLICIT, &[])
        .map_err(|e| format!("register: {e:?}"))?;

    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let out = std::process::Command::new(exe)
        .env("KF_DEFERRED_ORACLE", "1")
        .env("KF_DF2_PHASE", "f3-child")
        .env("KF_DF2_F3_HANDLE", format!("{h:#x}"))
        .output()
        .map_err(|e| format!("spawn child: {e}"))?;
    let child_out = String::from_utf8_lossy(&out.stdout);
    print!("{child_out}");
    if !out.status.success() {
        println!("DF2_F3_CHILD_STDERR={}", String::from_utf8_lossy(&out.stderr));
    }

    let still = u.is_registered(h)?;
    if !still {
        let _ = u.remove(h);
    }
    u.free();
    println!(
        "DF2_F3={} evidence=A_untouched_after_0x200_from_second_process still_registered={still}",
        if still { "PASS" } else { "FAIL(cross-client execution)" }
    );
    if still { Ok(()) } else { Err("F3 falsified".into()) }
}

/// The second process F3 spawns: its own client, its own object, fires the SAME handle number.
fn f3_child() -> Result<(), String> {
    let h = std::env::var("KF_DF2_F3_HANDLE")
        .ok()
        .and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok())
        .ok_or("KF_DF2_F3_HANDLE missing")?;
    let mut u = GrUnit::open()?;
    u.alloc_5080()?;
    u.bind_5080(SUBCH_SW)?;
    u.fire(SUBCH_SW, h)?;
    u.free();
    println!("DF2_F3_CHILD fired_handle={h:#x}");
    Ok(())
}

/// F5 — garbage/unregistered handles: nothing executes, blast radius is the firing channel only.
fn f5() -> Result<(), String> {
    let mut victim = GrUnit::open()?;
    victim.alloc_5080()?;
    victim.bind_5080(SUBCH_SW)?;
    let vh = 0x5252_5001;
    victim
        .register(vh, model::CMD_DMA_INVALIDATE_TLB, model::FLAGS_DELETE_IMPLICIT, &[])
        .map_err(|e| format!("register victim entry: {e:?}"))?;

    let mut firer = GrUnit::open()?;
    firer.alloc_5080()?;
    firer.bind_5080(SUBCH_SW)?;
    for (name, h) in [
        ("zero", 0u32),
        ("one", 1u32),
        ("0x40000000", 0x4000_0000u32),
        ("0xffffffff", 0xFFFF_FFFFu32),
    ] {
        match firer.fire(SUBCH_SW, h) {
            Ok(()) => println!("DF2_F5_STRAY case={name} handle={h:#x} fire_ok=true"),
            Err(e) => {
                println!("DF2_F5_STRAY case={name} handle={h:#x} fire_ok=false detail={e}");
                // The firing channel may now be dead; stop firing more on it (owner rule: do not
                // retry/loop past an unpredicted failure).
                break;
            }
        }
    }
    firer.free();
    let still = victim.is_registered(vh)?;
    if !still {
        let _ = victim.remove(vh);
    }
    victim.free();
    println!(
        "DF2_F5={} evidence=victim_untouched_after_stray_triggers victim_still_registered={still}",
        if still { "PASS" } else { "FAIL(stray handle executed a bystander's entry)" }
    );
    if still { Ok(()) } else { Err("F5 falsified".into()) }
}

/// F6 — rate: a short, wall-clock-capped loop of valid EXPLICIT-delete `DMA_INVALIDATE_TLB`
/// triggers, measured against an equal-duration loop of ordinary semaphore-release submissions,
/// both against a no-contention baseline. Each leg runs for `KF_DF2_F6_MS` (default 1500 ms).
fn f6() -> Result<(), String> {
    let cap_ms: u64 = std::env::var("KF_DF2_F6_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1500);
    let cap = std::time::Duration::from_millis(cap_ms);

    let mut baseline = GrUnit::open()?;
    let (b_count, b_elapsed) = run_submits_for(&mut baseline, cap, None)?;
    baseline.free();
    println!("DF2_F6_BASELINE submits={b_count} elapsed_ms={}", b_elapsed.as_millis());

    let mut ordinary = GrUnit::open()?;
    let (o_count, o_elapsed) = run_submits_for(&mut ordinary, cap, None)?;
    ordinary.free();
    println!("DF2_F6_ORDINARY submits={o_count} elapsed_ms={}", o_elapsed.as_millis());

    let mut deferred = GrUnit::open()?;
    deferred.alloc_5080()?;
    deferred.bind_5080(SUBCH_SW)?;
    let (d_count, d_elapsed) = run_submits_for(&mut deferred, cap, Some(()))?;
    deferred.free();
    println!("DF2_F6_DEFERRED submits={d_count} elapsed_ms={}", d_elapsed.as_millis());

    let o_rate = o_count as f64 / o_elapsed.as_secs_f64().max(1e-9);
    let d_rate = d_count as f64 / d_elapsed.as_secs_f64().max(1e-9);
    let b_rate = b_count as f64 / b_elapsed.as_secs_f64().max(1e-9);
    // PASS = the deferred-API rate is not disproportionately worse than the ordinary-method rate,
    // both relative to the uncontended baseline. "Disproportionate" is read as "less than half the
    // ordinary rate's fraction of baseline" — a coarse, stated threshold, not a tight SLA.
    let o_frac = o_rate / b_rate.max(1e-9);
    let d_frac = d_rate / b_rate.max(1e-9);
    let pass = d_frac >= o_frac / 2.0;
    println!(
        "DF2_F6={} baseline_rate={b_rate:.0}/s ordinary_rate={o_rate:.0}/s \
         ({:.2}x baseline) deferred_rate={d_rate:.0}/s ({:.2}x baseline)",
        if pass { "PASS" } else { "FAIL(disproportionate degradation)" },
        o_frac,
        d_frac
    );
    if pass { Ok(()) } else { Err("F6 falsified".into()) }
}

/// Submit either a plain semaphore-release-shaped no-op (`deferred = None`) or a registered
/// EXPLICIT-delete `DMA_INVALIDATE_TLB` trigger (`deferred = Some(())`, re-registering it each
/// round so the loop always has a live entry to fire) in a tight loop until `cap` elapses.
/// Returns `(submits, elapsed)`.
fn run_submits_for(u: &mut GrUnit, cap: std::time::Duration, deferred: Option<()>) -> Result<(u32, std::time::Duration), String> {
    let va_extra = vec![(4usize, u.space.space)];
    let mut n = 0u32;
    let t0 = std::time::Instant::now();
    while t0.elapsed() < cap {
        if deferred.is_some() {
            let h = 0x5252_6000u32.wrapping_add(n);
            u.register(h, model::CMD_DMA_INVALIDATE_TLB, model::FLAGS_DELETE_EXPLICIT, &va_extra)
                .map_err(|e| format!("F6 register round {n}: {e:?}"))?;
            u.fire(SUBCH_SW, h)?;
            let _ = u.remove(h);
        } else {
            // An ordinary method of the same shape as the fence tail alone: just submit (fence
            // itself already issues a semaphore release + non-stall interrupt).
            u.submit(&[])?;
        }
        n += 1;
    }
    Ok((n, t0.elapsed()))
}
