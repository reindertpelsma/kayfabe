//! NV50_DEFERRED_API (class 0x5080) passthrough-safety falsification — a NATIVE,
//! unprivileged RM client that MEASURES real-hardware behaviour. It changes no
//! kayfabe production path; it only talks to the host RM the way any userspace
//! client does.
//!
//! The hypothesis under test (owner's worry) and the full falsifier list live in
//! `traces/deferred_api_falsify_20261007/README.md`, written BEFORE the first run.
//! Summary of the falsifiers this binary measures:
//!
//! - P0  POSITIVE CONTROL. A handle registered on object O and triggered by 0x200
//!       on O's own subchannel MUST read back as executed (re-register succeeds).
//!       No cross-* "PASS" below is trustworthy unless P0 passes: it proves the
//!       0x200 encoding actually fires the handler on this chip/driver.
//! - F1  cross-object, same channel/client: entry on O_A is NOT run by 0x200 on O_B.
//! - F2  cross-channel, same client: fired from another channel's pushbuffer.
//! - F3  cross-client: fired by a different RM client.
//! - F4  no escalation: a user client that registers a PRIVILEGED/kernel cmd and
//!       fires it gets a failure at trigger and no effect (run as a non-root user).
//! - F5  garbage handles: unregistered handles never execute; record blast radius.
//! - F6  rate/DoS: a tight EXPLICIT-delete loop vs an equivalent ordinary-method
//!       spam, both against a baseline.
//! - F7a arbitrary-handle registrability: which hApiHandle values a host client can
//!       register (0, 1, 0x40000000+N, 0xffffffff, 0xcafe0000-range, own handles).
//! - F7b a dedicated host client per guest gives its own handle namespace.
//! - F8  a stray 0x200 (and a SET_OBJECT for class 0x5080) in a channel with NO
//!       5080 object — the case that matters if we simply refuse 5080 on Passthrough.
//!
//! Observable (DELETE_IMPLICIT): "executed" == re-registering the same handle now
//! SUCCEEDS; "untouched" == re-register fails with INVALID_OBJECT_HANDLE (0x33).
//!
//! Output: one `DF_...=` line per measured fact (machine-greppable). Raw logs are
//! filtered on the host; nothing large is printed here.

use kf_chan::host::HostRing;
use kf_host::{HostRm, RmError, VaSpace};
use kf_linux_raw::DevDir;

// ─── RM status codes (ogkm-580 nvstatuscodes.h) ───
const NV_ERR_INSERT_DUPLICATE_NAME: u32 = 0x19;
const NV_ERR_INVALID_DATA: u32 = 0x25;
const NV_ERR_INVALID_OBJECT_HANDLE: u32 = 0x33;
const NV_ERR_INVALID_PARAM_STRUCT: u32 = 0x3A;

// ─── class + controls (ogkm-580 cl5080.h / ctrl5080.h) ───
const NV50_DEFERRED_API_CLASS: u32 = 0x5080;
const NV5080_CTRL_CMD_DEFERRED_API_V2: u32 = 0x5080_0103;
const NV5080_CTRL_CMD_REMOVE_API: u32 = 0x5080_0102;
/// sizeof(NV5080_CTRL_DEFERRED_API_V2_PARAMS): 20-byte prefix, 8-aligned union
/// api_bundle@24 whose largest member is NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS (560).
/// P0 validates this against RM (INVALID_PARAM_STRUCT ⇒ wrong size).
const DEFERRED_V2_SIZE: usize = 584;
const BUNDLE_OFF: usize = 24;

// ─── inner deferred commands (ogkm-580 ctrl2080*.h) ───
const CMD_DMA_INVALIDATE_TLB: u32 = 0x2080_2502; // NON_PRIVILEGED
const CMD_GPU_PROMOTE_CTX: u32 = 0x2080_012b; // PRIVILEGED
const CMD_GPU_INITIALIZE_CTX: u32 = 0x2080_012d; // PRIVILEGED
const CMD_GPU_EVICT_CTX: u32 = 0x2080_012c; // kernel-only
const CMD_FIFO_UPDATE_CHANNEL_INFO: u32 = 0x2080_1116; // PRIVILEGED

// ─── deferred-api flags (ctrl5080.h) ───
const FLAGS_DELETE_IMPLICIT: u32 = 0x0; // auto-delete after execute
const FLAGS_DELETE_EXPLICIT: u32 = 0x1;

// ─── pushbuffer ───
const SW_TRIGGER_METHOD: u32 = 0x200; // owner's background; byte-offset units
const SUBCH_A: u32 = 1;
const SUBCH_B: u32 = 2;

fn status_of(e: &RmError) -> Option<u32> {
    match e {
        RmError::Other(s) => Some(*s),
        RmError::InsufficientPermissions => Some(0x1B),
        RmError::NoMemory => Some(0x1F_FFFF), // sentinel, not an RM code
        _ => None,
    }
}

/// Build a DEFERRED_API_V2 param block.
fn defer_params(h: u32, cmd: u32, flags: u32, h_vaspace: u32) -> [u8; DEFERRED_V2_SIZE] {
    let mut p = [0u8; DEFERRED_V2_SIZE];
    p[0..4].copy_from_slice(&h.to_le_bytes());
    p[4..8].copy_from_slice(&cmd.to_le_bytes());
    p[8..12].copy_from_slice(&flags.to_le_bytes());
    // hClientVA@12, hDeviceVA@16 left 0 (used only by the FillPteMem bundle).
    if cmd == CMD_DMA_INVALIDATE_TLB {
        // NV2080_CTRL_DMA_INVALIDATE_TLB_PARAMS { NvU32 engine; NvHandle hVASpace; }
        // engine@BUNDLE_OFF left 0; hVASpace@BUNDLE_OFF+4.
        p[BUNDLE_OFF + 4..BUNDLE_OFF + 8].copy_from_slice(&h_vaspace.to_le_bytes());
    }
    p
}

/// A 5080 software object bound to one subchannel of a channel.
struct Def {
    handle: u32,
    class_engine: u32,
    subch: u32,
}

/// One RM client with a VA space and a channel; carries up to two 5080 objects.
struct Unit {
    rm: HostRm,
    space: VaSpace,
    ring: HostRing,
}

impl Unit {
    fn open() -> Result<Unit, String> {
        let dev = DevDir::open(c"/dev").map_err(|e| format!("dev: {e:?}"))?;
        let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
            .map_err(|e| e.to_string())?;
        let space = rm.alloc_vaspace_bare().map_err(|e| format!("vaspace: {e:?}"))?;
        let ring = HostRing::new(&rm, space)?;
        Ok(Unit { rm, space, ring })
    }

    /// Allocate a 5080 object under the channel and read its SET_OBJECT classEngineID.
    fn alloc_5080(&self, subch: u32) -> Result<Def, String> {
        let want = self.rm.mint();
        let handle = self
            .rm
            .raw_alloc(
                self.ring.channel().chan,
                want,
                NV50_DEFERRED_API_CLASS,
                None,
                &mut [0u8; 0],
            )
            .map_err(|e| format!("alloc 5080: {e:?}"))?;
        self.rm.remember(handle, self.ring.channel().chan);
        // NV906F_CTRL_GET_CLASS_ENGINEID: word0=hObject(in), word1=classEngineID(out).
        let mut p = [0u8; 16];
        p[0..4].copy_from_slice(&handle.to_le_bytes());
        self.rm
            .raw_control(self.ring.channel().chan, kf_host::channel::NV906F_CTRL_GET_CLASS_ENGINEID, &mut p)
            .map_err(|e| format!("class_engine_id: {e:?}"))?;
        let class_engine = u32::from_le_bytes([p[4], p[5], p[6], p[7]]);
        Ok(Def { handle, class_engine, subch })
    }

    /// Bind the object to its subchannel (SET_OBJECT), submitted and fenced once.
    fn bind(&mut self, d: &Def) -> Result<(), String> {
        let hdr = kf_abi::submit::method_header_inc(d.subch, kf_abi::submit::SET_OBJECT, 1)
            .ok_or("SET_OBJECT header")?;
        self.submit(&[hdr, d.class_engine])
    }

    /// Fire the software trigger 0x200 with `data` on `subch`, then drain.
    fn fire(&mut self, subch: u32, data: u32) -> Result<(), String> {
        let hdr = kf_abi::submit::method_header_inc(subch, SW_TRIGGER_METHOD, 1)
            .ok_or("0x200 header")?;
        self.submit(&[hdr, data])
    }

    /// Push words, fence, and wait for completion (bounded).
    fn submit(&mut self, words: &[u32]) -> Result<(), String> {
        match self.ring.push(words)? {
            Ok(()) => {}
            Err(_) => return Err("push busy".into()),
        }
        let seq = match self.ring.fence(&self.rm)? {
            Ok(s) => s,
            Err(_) => return Err("fence busy".into()),
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if self.ring.completed()? == seq {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(format!("fence timeout seq={seq}"));
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    fn register(&self, d: &Def, h: u32, cmd: u32, flags: u32) -> Result<(), RmError> {
        let mut p = defer_params(h, cmd, flags, self.space.space);
        self.rm
            .raw_control(d.handle, NV5080_CTRL_CMD_DEFERRED_API_V2, &mut p)
    }

    fn remove(&self, d: &Def, h: u32) -> Result<(), RmError> {
        let mut p = h.to_le_bytes();
        self.rm.raw_control(d.handle, NV5080_CTRL_CMD_REMOVE_API, &mut p)
    }

    /// Non-destructive probe: is `h` still registered on `d`?
    /// Re-register with a benign cmd: Err(INVALID_OBJECT_HANDLE) ⇒ still present;
    /// Ok ⇒ it was gone (we just added it — remove it again to leave no trace).
    fn is_registered(&self, d: &Def, h: u32) -> Result<bool, String> {
        match self.register(d, h, CMD_DMA_INVALIDATE_TLB, FLAGS_DELETE_EXPLICIT) {
            Err(e) if status_of(&e) == Some(NV_ERR_INVALID_OBJECT_HANDLE) => Ok(true),
            Ok(()) => {
                let _ = self.remove(d, h);
                Ok(false)
            }
            Err(e) => Err(format!("probe register: {e:?} (status {:?})", status_of(&e))),
        }
    }

    fn free(mut self) {
        let _ = self.ring.release(&self.rm);
        let _ = self.rm.free_channel(self.ring.channel());
        self.rm.free_vaspace(self.space);
    }
}

fn main() {
    let phase = std::env::var("KF_DF_PHASE").unwrap_or_else(|_| "all".into());
    println!(
        "DF_START euid={} phase={phase} driver={}",
        euid(),
        probe_driver()
    );
    let phases: Vec<&str> = if phase == "all" {
        vec!["p0", "f1", "f2", "f3", "f4", "f5", "f6", "f7a", "f7b", "f8"]
    } else {
        phase.split(',').collect()
    };
    let mut any_fail = false;
    for p in phases {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_phase(p)));
        match r {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                println!("DF_{}=BLOCKED {e}", p.to_uppercase());
                any_fail = true;
            }
            Err(_) => {
                println!("DF_{}=PANIC", p.to_uppercase());
                any_fail = true;
            }
        }
    }
    println!("DF_DONE fail={any_fail}");
}

fn probe_driver() -> String {
    Unit::open().map(|u| {
        let v = u.rm.driver_version().to_string();
        u.free();
        v
    }).unwrap_or_else(|e| format!("open-failed:{e}"))
}

fn run_phase(p: &str) -> Result<(), String> {
    match p {
        "p0" => p0_positive_control(),
        "f1" => f1_cross_object(),
        "f2" => f2_cross_channel(),
        "f3" => f3_cross_client(),
        "f4" => f4_no_escalation(),
        "f5" => f5_garbage_handles(),
        "f6" => f6_rate(),
        "f7a" => f7a_arbitrary_handles(),
        "f7b" => f7b_dedicated_client(),
        "f8" => f8_no_object(),
        other => Err(format!("unknown phase {other}")),
    }
}

/// P0 — positive control: a handle fired on its OWN object must read back executed.
fn p0_positive_control() -> Result<(), String> {
    let mut u = Unit::open()?;
    let d = u.alloc_5080(SUBCH_A)?;
    u.bind(&d)?;
    let h = 0x5151_0001;
    // First confirm registration works and the param size is right.
    match u.register(&d, h, CMD_DMA_INVALIDATE_TLB, FLAGS_DELETE_IMPLICIT) {
        Ok(()) => {}
        Err(e) if status_of(&e) == Some(NV_ERR_INVALID_PARAM_STRUCT) => {
            u.free();
            return Err(format!("register INVALID_PARAM_STRUCT — DEFERRED_V2_SIZE={DEFERRED_V2_SIZE} wrong"));
        }
        Err(e) => {
            u.free();
            return Err(format!("register failed: {e:?} status={:?}", status_of(&e)));
        }
    }
    // Sanity: a duplicate register must now fail with INVALID_OBJECT_HANDLE.
    let dup = u.register(&d, h, CMD_DMA_INVALIDATE_TLB, FLAGS_DELETE_IMPLICIT);
    let dup_ok = matches!(&dup, Err(e) if status_of(e) == Some(NV_ERR_INVALID_OBJECT_HANDLE));
    println!("DF_P0_OBSERVABLE dup_reject={dup_ok}");
    // Fire 0x200 on the object's own subchannel.
    u.fire(d.subch, h)?;
    let executed = !u.is_registered(&d, h)?;
    u.free();
    println!(
        "DF_P0={} evidence=same-object 0x200(h) executed={executed} dup_reject={dup_ok}",
        if executed && dup_ok { "PASS(positive-control-fires)" } else { "FAIL(trigger-did-not-fire)" }
    );
    if executed && dup_ok {
        Ok(())
    } else {
        Err("positive control did not fire; cross-* negatives below are NOT trustworthy".into())
    }
}

/// F1 — entry on O_A must not be run by 0x200 fired on O_B (same channel/client).
fn f1_cross_object() -> Result<(), String> {
    let mut u = Unit::open()?;
    let a = u.alloc_5080(SUBCH_A)?;
    let b = u.alloc_5080(SUBCH_B)?;
    u.bind(&a)?;
    u.bind(&b)?;
    let h = 0x5151_1001;
    u.register(&a, h, CMD_DMA_INVALIDATE_TLB, FLAGS_DELETE_IMPLICIT)
        .map_err(|e| format!("register on A: {e:?}"))?;
    u.fire(b.subch, h)?; // fire on O_B's subchannel
    let still = u.is_registered(&a, h)?;
    // clean up A's entry
    let _ = u.remove(&a, h);
    u.free();
    println!(
        "DF_F1={} evidence=entry-on-A fired-0x200(h)-on-B A_still_registered={still}",
        if still { "PASS(cross-object-isolated)" } else { "FAIL(cross-object-executed)" }
    );
    Ok(())
}

/// F2 — same, but 0x200 fired from a SECOND channel's pushbuffer (same client).
fn f2_cross_channel() -> Result<(), String> {
    let dev = DevDir::open(c"/dev").map_err(|e| format!("dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    // Two channels (two VA spaces) under ONE client.
    let space_a = rm.alloc_vaspace_bare().map_err(|e| format!("vaspace A: {e:?}"))?;
    let space_b = rm.alloc_vaspace_bare().map_err(|e| format!("vaspace B: {e:?}"))?;
    let ring_a = HostRing::new(&rm, space_a)?;
    let mut ring_b = HostRing::new(&rm, space_b)?;
    // O_A on channel A, O_B on channel B.
    let alloc = |ring: &HostRing, subch: u32| -> Result<Def, String> {
        let want = rm.mint();
        let handle = rm
            .raw_alloc(ring.channel().chan, want, NV50_DEFERRED_API_CLASS, None, &mut [0u8; 0])
            .map_err(|e| format!("alloc 5080: {e:?}"))?;
        rm.remember(handle, ring.channel().chan);
        let mut p = [0u8; 16];
        p[0..4].copy_from_slice(&handle.to_le_bytes());
        rm.raw_control(ring.channel().chan, kf_host::channel::NV906F_CTRL_GET_CLASS_ENGINEID, &mut p)
            .map_err(|e| format!("class_engine_id: {e:?}"))?;
        Ok(Def { handle, class_engine: u32::from_le_bytes([p[4], p[5], p[6], p[7]]), subch })
    };
    let a = alloc(&ring_a, SUBCH_A)?;
    let b = alloc(&ring_b, SUBCH_A)?;
    // bind + fire on B
    let bind = |ring: &mut HostRing, d: &Def| -> Result<(), String> {
        let hdr = kf_abi::submit::method_header_inc(d.subch, kf_abi::submit::SET_OBJECT, 1)
            .ok_or("SET_OBJECT header")?;
        submit_on(ring, &rm, &[hdr, d.class_engine])
    };
    bind(&mut ring_b, &b)?;
    let h = 0x5151_2001;
    let mut pa = defer_params(h, CMD_DMA_INVALIDATE_TLB, FLAGS_DELETE_IMPLICIT, space_a.space);
    rm.raw_control(a.handle, NV5080_CTRL_CMD_DEFERRED_API_V2, &mut pa)
        .map_err(|e| format!("register on A: {e:?}"))?;
    // fire 0x200(h) on channel B
    let hdr = kf_abi::submit::method_header_inc(b.subch, SW_TRIGGER_METHOD, 1).ok_or("0x200 header")?;
    submit_on(&mut ring_b, &rm, &[hdr, h])?;
    // probe A
    let mut probe = defer_params(h, CMD_DMA_INVALIDATE_TLB, FLAGS_DELETE_EXPLICIT, space_a.space);
    let still = match rm.raw_control(a.handle, NV5080_CTRL_CMD_DEFERRED_API_V2, &mut probe) {
        Err(e) if status_of(&e) == Some(NV_ERR_INVALID_OBJECT_HANDLE) => true,
        Ok(()) => {
            let mut rp = h.to_le_bytes();
            let _ = rm.raw_control(a.handle, NV5080_CTRL_CMD_REMOVE_API, &mut rp);
            false
        }
        Err(e) => return Err(format!("probe: {e:?}")),
    };
    let mut rp = h.to_le_bytes();
    let _ = rm.raw_control(a.handle, NV5080_CTRL_CMD_REMOVE_API, &mut rp);
    // teardown
    let _ = ring_b.release(&rm);
    let _ = rm.free_channel(ring_b.channel());
    let mut ra = ring_a;
    let _ = ra.release(&rm);
    let _ = rm.free_channel(ra.channel());
    rm.free_vaspace(space_a);
    rm.free_vaspace(space_b);
    println!(
        "DF_F2={} evidence=entry-on-chanA fired-0x200(h)-on-chanB A_still_registered={still}",
        if still { "PASS(cross-channel-isolated)" } else { "FAIL(cross-channel-executed)" }
    );
    Ok(())
}

fn submit_on(ring: &mut HostRing, rm: &HostRm, words: &[u32]) -> Result<(), String> {
    match ring.push(words)? {
        Ok(()) => {}
        Err(_) => return Err("push busy".into()),
    }
    let seq = match ring.fence(rm)? {
        Ok(s) => s,
        Err(_) => return Err("fence busy".into()),
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if ring.completed()? == seq {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("fence timeout seq={seq}"));
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// F3 — entry on client-1's object must not be run by 0x200 on client-2's object.
fn f3_cross_client() -> Result<(), String> {
    let mut u1 = Unit::open()?;
    let mut u2 = Unit::open()?; // a second, independent RM client (distinct hClient)
    let a = u1.alloc_5080(SUBCH_A)?;
    let b = u2.alloc_5080(SUBCH_A)?;
    u2.bind(&b)?;
    let h = 0x5151_3001;
    u1.register(&a, h, CMD_DMA_INVALIDATE_TLB, FLAGS_DELETE_IMPLICIT)
        .map_err(|e| format!("register on client1/A: {e:?}"))?;
    u2.fire(b.subch, h)?; // client-2 fires 0x200(h) on its own object
    let still = u1.is_registered(&a, h)?;
    let _ = u1.remove(&a, h);
    u1.free();
    u2.free();
    println!(
        "DF_F3={} evidence=entry-on-client1 fired-0x200(h)-on-client2 c1_still_registered={still}",
        if still { "PASS(cross-client-isolated)" } else { "FAIL(cross-client-executed)" }
    );
    Ok(())
}

/// F4 — a user client registers PRIVILEGED/kernel cmds and fires them: must fail at
/// trigger with no effect. Registration is NOT expected to check the cmd.
fn f4_no_escalation() -> Result<(), String> {
    let mut u = Unit::open()?;
    let d = u.alloc_5080(SUBCH_A)?;
    u.bind(&d)?;
    let cmds = [
        ("GPU_PROMOTE_CTX", CMD_GPU_PROMOTE_CTX),
        ("GPU_INITIALIZE_CTX", CMD_GPU_INITIALIZE_CTX),
        ("FIFO_UPDATE_CHANNEL_INFO", CMD_FIFO_UPDATE_CHANNEL_INFO),
        ("GPU_EVICT_CTX", CMD_GPU_EVICT_CTX),
    ];
    let euid = euid();
    for (name, cmd) in cmds {
        let h = 0x5151_4000 + cmd;
        let reg = u.register(&d, h, cmd, FLAGS_DELETE_IMPLICIT);
        let reg_status = match &reg {
            Ok(()) => "OK".to_string(),
            Err(e) => format!("{:?}", status_of(e)),
        };
        // Fire regardless; a registered privileged entry should fail at dispatch.
        // The channel staying alive and the entry being consumed-with-failure is the pass.
        let fired = if reg.is_ok() { u.fire(d.subch, h).is_ok() } else { false };
        // After firing, is the entry gone (consumed) and did the channel survive?
        let consumed = if reg.is_ok() {
            match u.is_registered(&d, h) {
                Ok(present) => {
                    if present {
                        let _ = u.remove(&d, h);
                    }
                    !present
                }
                Err(_) => false, // channel/object died ⇒ cannot probe
            }
        } else {
            false
        };
        println!(
            "DF_F4_CMD name={name} cmd={cmd:#x} euid={euid} register={reg_status} fired_ok={fired} entry_consumed={consumed}"
        );
    }
    // Did the client/channel survive the whole sequence?
    let survived = u.alloc_5080(SUBCH_B).is_ok();
    u.free();
    let pass = euid != 0 && survived;
    println!(
        "DF_F4={} evidence=non-root({}) privileged-cmds-registered-and-fired channel_survived={survived}",
        if pass { "MEASURED(see DF_F4_CMD lines)" } else { "CHECK(run as non-root)" },
        euid != 0
    );
    Ok(())
}

/// F5 — unregistered/garbage handles never execute; record blast radius.
fn f5_garbage_handles() -> Result<(), String> {
    // A bystander channel/object in a SEPARATE client that must stay alive throughout.
    let mut victim = Unit::open()?;
    let vd = victim.alloc_5080(SUBCH_A)?;
    victim.bind(&vd)?;
    let vh = 0x5151_5099;
    victim
        .register(&vd, vh, CMD_DMA_INVALIDATE_TLB, FLAGS_DELETE_EXPLICIT)
        .map_err(|e| format!("victim register: {e:?}"))?;

    let mut u = Unit::open()?;
    let d = u.alloc_5080(SUBCH_A)?;
    u.bind(&d)?;
    let mut garbage: Vec<u32> = vec![0, 1, 0xffff_ffff, 0x4000_0000, 0x4000_0001, 0x4000_0100];
    // also real handles of THIS client's own live objects
    garbage.push(u.ring.channel().chan);
    garbage.push(u.ring.channel().tsg);
    garbage.push(u.space.space);
    garbage.push(d.handle);
    let mut channel_alive = true;
    for g in garbage {
        if !channel_alive {
            println!("DF_F5_FIRE handle={g:#x} skipped=channel-already-dead");
            continue;
        }
        let r = u.fire(SUBCH_A, g);
        let alive = r.is_ok();
        if !alive {
            channel_alive = false;
        }
        println!(
            "DF_F5_FIRE handle={g:#x} fire_ok={alive} detail={}",
            r.err().unwrap_or_default()
        );
    }
    // victim must still be registered (nothing executed it) and its client alive
    let victim_untouched = victim.is_registered(&vd, vh).unwrap_or(false);
    let victim_alive = victim.alloc_5080(SUBCH_B).is_ok();
    let _ = victim.remove(&vd, vh);
    victim.free();
    let firing_recovered = u.alloc_5080(SUBCH_B).is_ok();
    u.free();
    println!(
        "DF_F5={} evidence=garbage-0x200 firing_channel_recovered={firing_recovered} victim_entry_untouched={victim_untouched} victim_client_alive={victim_alive}",
        if victim_untouched && victim_alive {
            "PASS(no-execute, victim-isolated)"
        } else {
            "FAIL(garbage-had-effect)"
        }
    );
    Ok(())
}

/// F6 — tight EXPLICIT-delete fire loop vs an equivalent ordinary-method spam,
/// each measured against a no-contention baseline on a concurrent channel.
fn f6_rate() -> Result<(), String> {
    const ITERS: u32 = 2000;
    // Worker channel whose round-trip latency we measure under three conditions.
    let mut worker = Unit::open()?;
    let measure = |worker: &mut Unit| -> Result<f64, String> {
        let n = 200u32;
        let t = std::time::Instant::now();
        for _ in 0..n {
            worker.fire(SUBCH_A, 0)?; // a 0x200 with handle 0 (no-op trigger) as a cheap round trip
        }
        Ok(t.elapsed().as_secs_f64() * 1e6 / f64::from(n))
    };
    // Give the worker a bound object so its own 0x200 is well-formed.
    let wd = worker.alloc_5080(SUBCH_A)?;
    worker.bind(&wd)?;

    // Baseline: worker alone.
    let base = measure(&mut worker)?;

    // Spammer in a separate client.
    let mut spam = Unit::open()?;
    let sd = spam.alloc_5080(SUBCH_A)?;
    spam.bind(&sd)?;
    let sh = 0x5151_6001;
    spam.register(&sd, sh, CMD_DMA_INVALIDATE_TLB, FLAGS_DELETE_EXPLICIT)
        .map_err(|e| format!("spam register: {e:?}"))?;

    // Condition A: spam valid deferred-API triggers (EXPLICIT entry, re-fired).
    let ta = std::time::Instant::now();
    for _ in 0..ITERS {
        spam.fire(SUBCH_A, sh)?;
    }
    let spam_defer_us = ta.elapsed().as_secs_f64() * 1e6 / f64::from(ITERS);
    let under_defer = measure(&mut worker)?;

    // Condition B: spam ordinary methods — the same channel firing 0x200 with handle 0
    // (a plain unknown-handle software method; an "ordinary method" of equivalent cost).
    let tb = std::time::Instant::now();
    for _ in 0..ITERS {
        spam.fire(SUBCH_A, 0)?;
    }
    let spam_ord_us = tb.elapsed().as_secs_f64() * 1e6 / f64::from(ITERS);
    let under_ord = measure(&mut worker)?;

    let _ = spam.remove(&sd, sh);
    spam.free();
    worker.free();
    let ratio_defer = under_defer / base;
    let ratio_ord = under_ord / base;
    println!(
        "DF_F6 baseline_us={base:.2} under_defer_spam_us={under_defer:.2} under_ordinary_spam_us={under_ord:.2} ratio_defer={ratio_defer:.2} ratio_ordinary={ratio_ord:.2} spam_defer_us={spam_defer_us:.2} spam_ordinary_us={spam_ord_us:.2}"
    );
    // "disproportionate" is a judgement; flag only if deferred spam is clearly worse than ordinary.
    let disproportionate = ratio_defer > ratio_ord * 1.5 && ratio_defer > 2.0;
    println!(
        "DF_F6={} evidence=deferred-spam-vs-ordinary-spam disproportionate={disproportionate}",
        if disproportionate { "FAIL(disproportionate-DoS)" } else { "PASS(no-disproportionate-DoS)" }
    );
    Ok(())
}

/// F7a — which hApiHandle values a single host client can register.
fn f7a_arbitrary_handles() -> Result<(), String> {
    let u = Unit::open()?;
    let d = u.alloc_5080(SUBCH_A)?;
    // candidate handles
    let mut cands: Vec<(String, u32)> = vec![
        ("zero".into(), 0),
        ("one".into(), 1),
        ("0x40000000".into(), 0x4000_0000),
        ("0x40000005".into(), 0x4000_0005),
        ("0xffffffff".into(), 0xffff_ffff),
        ("0xcafe0000".into(), 0xcafe_0000),
        ("0xcafe0001".into(), 0xcafe_0001),
        ("fw_range_0xc9f00000".into(), 0xc9f0_0000),
        ("gen_range_0xcaf00000".into(), 0xcaf0_0000),
        ("own_channel".into(), u.ring.channel().chan),
        ("own_tsg".into(), u.ring.channel().tsg),
        ("own_vaspace".into(), u.space.space),
        ("own_5080_object".into(), d.handle),
    ];
    for n in 0..5u32 {
        cands.push((format!("0x40000100+{n}"), 0x4000_0100 + n));
    }
    for (name, h) in cands {
        let r = u.register(&d, h, CMD_DMA_INVALIDATE_TLB, FLAGS_DELETE_EXPLICIT);
        let (reg, st) = match &r {
            Ok(()) => ("REGISTERED", None),
            Err(e) => ("REJECTED", status_of(e)),
        };
        if r.is_ok() {
            let _ = u.remove(&d, h);
        }
        let stname = match st {
            Some(NV_ERR_INVALID_OBJECT_HANDLE) => "INVALID_OBJECT_HANDLE".into(),
            Some(NV_ERR_INSERT_DUPLICATE_NAME) => "INSERT_DUPLICATE_NAME".into(),
            Some(NV_ERR_INVALID_PARAM_STRUCT) => "INVALID_PARAM_STRUCT".into(),
            Some(NV_ERR_INVALID_DATA) => "INVALID_DATA".into(),
            Some(s) => format!("{s:#x}"),
            None => "-".into(),
        };
        println!("DF_F7A_HANDLE name={name} handle={h:#x} result={reg} status={stname}");
    }
    u.free();
    println!("DF_F7A=MEASURED evidence=see DF_F7A_HANDLE lines");
    Ok(())
}

/// F7b — a dedicated host client per guest gives its own handle namespace: a handle
/// that collides in client-1 registers fine in a fresh client-2.
fn f7b_dedicated_client() -> Result<(), String> {
    let u1 = Unit::open()?;
    let d1 = u1.alloc_5080(SUBCH_A)?;
    // Pick a handle equal to one of u1's own live resources (rejected in u1).
    let collide = u1.ring.channel().chan;
    let in_u1 = u1.register(&d1, collide, CMD_DMA_INVALIDATE_TLB, FLAGS_DELETE_EXPLICIT);
    let u1_reject = in_u1.is_err();
    if in_u1.is_ok() {
        let _ = u1.remove(&d1, collide);
    }
    // Fresh client-2: the same numeric handle is just a number in a different namespace.
    let u2 = Unit::open()?;
    let d2 = u2.alloc_5080(SUBCH_A)?;
    let in_u2 = u2.register(&d2, collide, CMD_DMA_INVALIDATE_TLB, FLAGS_DELETE_EXPLICIT);
    let u2_ok = in_u2.is_ok();
    if in_u2.is_ok() {
        let _ = u2.remove(&d2, collide);
    }
    u1.free();
    u2.free();
    println!(
        "DF_F7B={} evidence=handle={collide:#x} rejected_in_owning_client={u1_reject} accepted_in_dedicated_client={u2_ok}",
        if u2_ok { "MEASURED(dedicated-namespace-registers-it)" } else { "CHECK" }
    );
    Ok(())
}

/// F8 — a stray 0x200 (and a SET_OBJECT for class 0x5080) in a channel with NO 5080
/// object. The fallback case if we simply refuse 5080 allocs on Passthrough.
fn f8_no_object() -> Result<(), String> {
    // A bystander in a separate client that must stay alive.
    let mut victim = Unit::open()?;
    let vd = victim.alloc_5080(SUBCH_A)?;
    victim.bind(&vd)?;
    let vh = 0x5151_8099;
    victim
        .register(&vd, vh, CMD_DMA_INVALIDATE_TLB, FLAGS_DELETE_EXPLICIT)
        .map_err(|e| format!("victim register: {e:?}"))?;

    // A plain channel with NO 5080 object bound on SUBCH_A.
    let mut u = Unit::open()?;
    // (do NOT alloc or bind a 5080 object)
    let cases: [(&str, u32); 3] = [
        ("handle_zero", 0),
        ("plausible_handle", 0x5151_8001),
        ("other_handle", 0x4000_0001),
    ];
    let mut alive = true;
    for (name, h) in cases {
        if !alive {
            println!("DF_F8_STRAY case={name} handle={h:#x} skipped=channel-dead");
            continue;
        }
        let r = u.fire(SUBCH_A, h);
        if r.is_err() {
            alive = false;
        }
        println!(
            "DF_F8_STRAY case={name} handle={h:#x} fire_ok={} detail={}",
            r.is_ok(),
            r.err().unwrap_or_default()
        );
    }
    // SET_OBJECT for class 0x5080 with no such object allocated, then 0x200.
    if alive {
        let hdr = kf_abi::submit::method_header_inc(SUBCH_B, kf_abi::submit::SET_OBJECT, 1)
            .ok_or("SET_OBJECT header")?;
        let r = u.submit(&[hdr, NV50_DEFERRED_API_CLASS]);
        let set_ok = r.is_ok();
        println!(
            "DF_F8_SETOBJECT class=0x5080 no_object_allocated submit_ok={set_ok} detail={}",
            r.err().unwrap_or_default()
        );
        if set_ok {
            let r2 = u.fire(SUBCH_B, 0x5151_8002);
            let fire_ok = r2.is_ok();
            println!(
                "DF_F8_SETOBJECT_THEN_FIRE fire_ok={fire_ok} detail={}",
                r2.err().unwrap_or_default()
            );
            if !fire_ok {
                alive = false;
            }
        } else {
            alive = false;
        }
    }
    let firing_recovered = u.alloc_5080(SUBCH_A).is_ok();
    u.free();
    let victim_untouched = victim.is_registered(&vd, vh).unwrap_or(false);
    let victim_alive = victim.alloc_5080(SUBCH_B).is_ok();
    let _ = victim.remove(&vd, vh);
    victim.free();
    println!(
        "DF_F8={} evidence=no-5080-object stray-0x200+SETOBJECT firing_channel_recovered={firing_recovered} last_alive={alive} victim_untouched={victim_untouched} victim_alive={victim_alive}",
        if victim_untouched && victim_alive {
            "MEASURED(victim-isolated; see DF_F8_* for firing-channel fate)"
        } else {
            "FAIL(bystander-affected)"
        }
    );
    Ok(())
}

/// Effective uid, read from `/proc/self/status` ("Uid: real effective saved fs").
/// Pure safe Rust — no `unsafe`, so the no-unsafe-outside-perimeter gate holds.
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
