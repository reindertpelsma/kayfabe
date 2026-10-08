//! ★★★ **Physical-operand oracle — does the hardware honour a PHYSICAL CE operand on an
//! UNPRIVILEGED channel?** (owner's Windows channel-policy question, `OWNER_RULINGS.md` §U.3,
//! `the_three_channel_kinds.md` §3.)
//!
//! Kayfabe only ever creates `PRIVILEGE_USER` host channels (`PRIVILEGED_CHANNEL=0`), and births
//! them with `DENY_PHYSICAL_MODE_CE` as a belt. The owner's forward-unknown proposal rests on a
//! hardware fact this binary exercises directly: a `LAUNCH_DMA` with `SRC_TYPE=PHYSICAL` naming the
//! physical address of a page THIS TEST OWNS (a known pattern), destination VIRTUAL — does the
//! pattern arrive at the destination? The run and its evidence live in the trace, not here.
//!
//! - **honoured** — the destination holds the known pattern ⇒ the hypothesis (hardware ignores or
//!   faults physical on an unprivileged channel) is FALSE for this engine/arm. NOT cleared.
//! - **refused/ignored** — the copy faults (Xid, channel RC), or completes without delivering the
//!   pattern ⇒ physical is not honoured. Cleared for the forward-unknown use.
//!
//! ⊘ **SAFETY, enforced by construction.** The physical SOURCE is only ever the address of memory
//! this process allocated and filled (a VRAM object it owns, or a pinned sysmem page it owns). The
//! destination is ALWAYS virtual — never a physical write. A physical address this test did not
//! allocate is never named. A channel RC (Xid) is a RESULT, recorded, not a failure.
//!
//! One ARM per process invocation, so a channel RC in one arm cannot poison the next and the
//! orchestrator can confirm the host desktop is alive between runs:
//!
//! ```text
//! kf-phys-oracle <arm> <deny>
//!   arm  = virt | phys_fb | phys_sysmem
//!   deny = 1 (production: DENY_PHYSICAL_MODE_CE=TRUE) | 0 (unprivileged, belt off)
//! ```
//!
//! `virt` is the rig control: a virtual→virtual copy that MUST deliver, proving a null result in a
//! `phys_*` arm is the physical operand's doing, not a broken rig.

use kf_abi::submit::{
    ENGINE_TYPE_COPY0, NV0041_CTRL_CMD_GET_SURFACE_PHYS_ATTR, Nv0041SurfacePhysAttr, SET_OBJECT,
    USERD_GP_PUT, ce, gp_entry, method_header_inc,
};
use kf_harness::{CE_SUBCHANNEL, Ledger};
use kf_host::{HostRm, MapBacking, RingSpec};
use kf_linux_raw::{Backing, CachePolicy, DevDir, HostOffset, HostPageSize, HostProt};

const OBJ_BYTES: u64 = 0x1_0000;
const COPY_LEN: u32 = 4096;
const RING_BYTES: u64 = 0x1_0000;
const GPFIFO_OFF: u64 = 0x1000;
const GPFIFO_ENTRIES: u32 = 64;
const SEM_OFF: u64 = 0x2000;
const USERD_OFF: u64 = 0x3000;
const PB_OFF: u64 = 0x4000;
/// The known source pattern. Word `i` is `PATTERN_BASE | i`, so a partial or aliased delivery is
/// distinguishable from the real one.
const PATTERN_BASE: u32 = 0x5A1D_0000;
/// The destination sentinel written before the copy — if it survives, nothing was delivered.
const DST_SENTINEL: u32 = 0xDEAD_0000;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arm = args.get(1).map(String::as_str).unwrap_or("virt");
    let deny = args.get(2).map(String::as_str).unwrap_or("1") != "0";
    let mut l = Ledger::default();
    println!(
        "PHYS_ORACLE_START arm={arm} deny_physical_mode_ce={} pid={}",
        deny,
        std::process::id()
    );
    if let Err(e) = run(&mut l, arm, deny) {
        l.check("run", false, e);
    }
    let v = l.verdict();
    println!(
        "PHYS_ORACLE_VERDICT arm={arm} {}",
        if v { "PASS" } else { "FAIL" }
    );
    std::process::exit(i32::from(!v));
}

/// Read the physical address of memory object `obj` through
/// `NV0041_CTRL_CMD_GET_SURFACE_PHYS_ATTR` (`offset 0`). Returns `(phys_addr, aperture)` with
/// aperture `0` VIDMEM, `1` SYSMEM.
fn phys_attr(rm: &HostRm, obj: u32) -> Result<(u64, u32), String> {
    let mut buf = [0u8; Nv0041SurfacePhysAttr::SIZE];
    Nv0041SurfacePhysAttr::encode_query(0, &mut buf).map_err(|e| format!("encode: {e:?}"))?;
    rm.raw_control(obj, NV0041_CTRL_CMD_GET_SURFACE_PHYS_ATTR, &mut buf)
        .map_err(|e| format!("GET_SURFACE_PHYS_ATTR refused: {e:?}"))?;
    let a = Nv0041SurfacePhysAttr::decode(&buf).map_err(|e| format!("decode: {e:?}"))?;
    Ok((a.mem_offset, a.mem_aperture))
}

/// A CE copy push: `src → dst` of `len` bytes, releasing `payload` at `sem_va`. `src_physical`
/// chooses `SRC_TYPE` and (when physical) the `SET_SRC_PHYS_MODE` target. The destination is
/// ALWAYS virtual.
fn copy_push(
    ce_class: u32,
    src: u64,
    dst: u64,
    len: u32,
    sem_va: u64,
    payload: u32,
    src_physical: Option<u32>,
) -> Option<Vec<u32>> {
    let sub = CE_SUBCHANNEL;
    let mut w = vec![method_header_inc(sub, SET_OBJECT, 1)?, ce_class];
    if let Some(target) = src_physical {
        w.push(method_header_inc(sub, ce::SET_SRC_PHYS_MODE, 1)?);
        w.push(target);
    }
    w.extend([
        method_header_inc(sub, ce::OFFSET_IN_UPPER, 4)?,
        (src >> 32) as u32,
        (src & 0xFFFF_FFFF) as u32,
        (dst >> 32) as u32,
        (dst & 0xFFFF_FFFF) as u32,
        method_header_inc(sub, ce::LINE_LENGTH_IN, 2)?,
        len,
        1,
        method_header_inc(sub, ce::SET_SEMAPHORE_A, 3)?,
        (sem_va >> 32) as u32,
        (sem_va & 0xFFFF_FFFF) as u32,
        payload,
    ]);
    let mut flags = ce::LAUNCH_TRANSFER_NON_PIPELINED
        | ce::LAUNCH_FLUSH_ENABLE
        | ce::LAUNCH_SEMAPHORE_RELEASE_ONE_WORD
        | ce::LAUNCH_SRC_PITCH
        | ce::LAUNCH_DST_PITCH
        | ce::LAUNCH_MULTI_LINE_DISABLE
        | ce::LAUNCH_DST_VIRTUAL;
    flags |= if src_physical.is_some() {
        ce::LAUNCH_SRC_PHYSICAL
    } else {
        ce::LAUNCH_SRC_VIRTUAL
    };
    w.extend([method_header_inc(sub, ce::LAUNCH_DMA, 1)?, flags]);
    Some(w)
}

fn run(l: &mut Ledger, arm: &str, deny: bool) -> Result<(), String> {
    let dev = DevDir::open(c"/dev").map_err(|e| format!("open /dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    l.measure("session", format!("driver {}", rm.driver_version()));
    let (_f, arch, _i) = rm.arch_info();
    l.measure(
        "ce_class",
        format!("{:#06x} arch={arch:#x}", rm.ce_class_id()),
    );

    let space = rm.alloc_vaspace().map_err(|e| format!("vaspace: {e:?}"))?;

    // Destination: a VRAM object we own, mapped VIRTUAL, pre-stamped with a sentinel.
    let dst = rm
        .alloc_device_local(OBJ_BYTES)
        .map_err(|e| format!("dst obj: {e:?}"))?;
    let dst_va = rm
        .map(space, dst, MapBacking::Dedicated, 0, OBJ_BYTES, None, false)
        .map_err(|e| format!("map dst: {e:?}"))?;
    let (_dn, dst_cpu) = rm
        .map_cpu(dst, OBJ_BYTES, CachePolicy::Uncached)
        .map_err(|e| format!("cpu dst: {e:?}"))?;
    for i in 0..(COPY_LEN / 4) {
        dst_cpu
            .store_u32(HostOffset::new(u64::from(i) * 4), DST_SENTINEL | i)
            .map_err(|e| format!("stamp dst: {e:?}"))?;
    }

    // Source and the operand the push will carry.
    let (src_operand, src_physical, _src_keep): (u64, Option<u32>, Box<dyn std::any::Any>) =
        match arm {
            "virt" => {
                let src = rm
                    .alloc_device_local(OBJ_BYTES)
                    .map_err(|e| format!("src obj: {e:?}"))?;
                let src_va = rm
                    .map(space, src, MapBacking::Dedicated, 0, OBJ_BYTES, None, false)
                    .map_err(|e| format!("map src: {e:?}"))?;
                fill_vram(&rm, src)?;
                (src_va, None, Box::new(()))
            }
            "phys_fb" => {
                let src = rm
                    .alloc_device_local(OBJ_BYTES)
                    .map_err(|e| format!("src obj: {e:?}"))?;
                fill_vram(&rm, src)?;
                let (phys, ap) = phys_attr(&rm, src)?;
                l.measure("src_phys_fb", format!("addr={phys:#x} aperture={ap}"));
                if ap != 0 {
                    return Err(format!("expected VIDMEM aperture, got {ap}"));
                }
                (phys, Some(ce::PHYS_MODE_TARGET_LOCAL_FB), Box::new(()))
            }
            "phys_sysmem" => {
                let region = kf_linux_raw::MappedRegion::map(
                    Backing::PrivateAnonymous,
                    OBJ_BYTES,
                    HostProt::ReadWrite,
                    CachePolicy::WriteBack,
                    HostPageSize::query(),
                )
                .map_err(|e| format!("anon region: {e:?}"))?;
                let mut pat = Vec::with_capacity(COPY_LEN as usize);
                for i in 0..(COPY_LEN / 4) {
                    pat.extend_from_slice(&(PATTERN_BASE | i).to_le_bytes());
                }
                region
                    .write_from(HostOffset::new(0), &pat)
                    .map_err(|e| format!("fill sysmem: {e:?}"))?;
                let region: &'static kf_linux_raw::MappedRegion = Box::leak(Box::new(region));
                let obj = rm
                    .alloc_os_descriptor(region, HostOffset::new(0), OBJ_BYTES)
                    .map_err(|e| format!("os descriptor: {e:?}"))?;
                let (phys, ap) = phys_attr(&rm, obj)?;
                l.measure("src_phys_sysmem", format!("addr={phys:#x} aperture={ap}"));
                if ap != 1 {
                    return Err(format!("expected SYSMEM aperture, got {ap}"));
                }
                (
                    phys,
                    Some(ce::PHYS_MODE_TARGET_COHERENT_SYSMEM),
                    Box::new(()),
                )
            }
            other => return Err(format!("unknown arm {other}")),
        };

    rm.invalidate_tlb(space)
        .map_err(|e| format!("invalidate: {e:?}"))?;

    // The ring + channel, with the chosen DENY_PHYSICAL_MODE_CE belt. The reply check proves USER.
    let ring = rm
        .alloc_device_local(RING_BYTES)
        .map_err(|e| format!("ring obj: {e:?}"))?;
    let ring_va = rm
        .map(
            space,
            ring,
            MapBacking::Dedicated,
            0,
            RING_BYTES,
            None,
            false,
        )
        .map_err(|e| format!("map ring: {e:?}"))?;
    let (_rn, ring_cpu) = rm
        .map_cpu(ring, RING_BYTES, CachePolicy::Uncached)
        .map_err(|e| format!("cpu ring: {e:?}"))?;
    let chan = rm
        .birth_channel_with(
            space,
            ENGINE_TYPE_COPY0,
            RingSpec {
                gp_fifo_va: ring_va + GPFIFO_OFF,
                gp_fifo_entries: GPFIFO_ENTRIES,
                userd_memory: ring,
                userd_offset: USERD_OFF,
                err_notifier: 0,
            },
            deny,
        )
        .map_err(|e| format!("birth: {e:?}"))?;
    l.check(
        "channel_born_user",
        chan.born_user.is_some(),
        format!("token={:#x} (reply checked PRIVILEGE_USER)", chan.token),
    );
    rm.alloc_ce_object(chan, ENGINE_TYPE_COPY0)
        .map_err(|e| format!("ce object: {e:?}"))?;
    rm.schedule(chan).map_err(|e| format!("schedule: {e:?}"))?;

    // Encode, place the pushbuffer and one GP entry, ring the doorbell.
    let payload = 0xABCD_1234u32;
    let words = copy_push(
        rm.ce_class_id(),
        src_operand,
        dst_va,
        COPY_LEN,
        ring_va + SEM_OFF,
        payload,
        src_physical,
    )
    .ok_or("push encode")?;
    for (i, w) in words.iter().enumerate() {
        ring_cpu
            .store_u32(HostOffset::new(PB_OFF + 4 * i as u64), *w)
            .map_err(|e| format!("pb store: {e:?}"))?;
    }
    ring_cpu
        .store_u32(HostOffset::new(SEM_OFF), 0)
        .map_err(|e| format!("sem clear: {e:?}"))?;
    let entry = gp_entry(ring_va + PB_OFF, 4 * words.len() as u64).ok_or("gp entry")?;
    ring_cpu
        .store_u32(HostOffset::new(GPFIFO_OFF), entry as u32)
        .map_err(|e| format!("gp lo: {e:?}"))?;
    ring_cpu
        .store_u32(HostOffset::new(GPFIFO_OFF + 4), (entry >> 32) as u32)
        .map_err(|e| format!("gp hi: {e:?}"))?;
    kf_linux_raw::release_fence();
    ring_cpu
        .store_u32(HostOffset::new(USERD_OFF + USERD_GP_PUT), 1)
        .map_err(|e| format!("gp put: {e:?}"))?;
    kf_linux_raw::release_fence();
    rm.doorbell(chan.token)
        .map_err(|e| format!("doorbell: {e:?}"))?;

    // Hard time-box: poll the CE release word and the destination for up to 3 s.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut ce_released = false;
    while std::time::Instant::now() < deadline {
        if ring_cpu.load_u32(HostOffset::new(SEM_OFF)).unwrap_or(0) == payload {
            ce_released = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let d0 = dst_cpu.load_u32(HostOffset::new(0)).unwrap_or(0);
    let d1 = dst_cpu.load_u32(HostOffset::new(4)).unwrap_or(0);
    let delivered = d0 == PATTERN_BASE && d1 == (PATTERN_BASE | 1);
    let untouched = d0 == DST_SENTINEL && d1 == (DST_SENTINEL | 1);
    l.measure(
        "ce_released",
        format!("{ce_released} (sem word @{SEM_OFF:#x})"),
    );
    l.measure(
        "dst_after",
        format!("word0={d0:#010x} word1={d1:#010x} delivered={delivered} untouched={untouched}"),
    );

    // The verdict per arm. The control MUST deliver; a phys arm RECORDS delivery (= honoured).
    match arm {
        "virt" => l.check(
            "virt_control_delivered",
            delivered && ce_released,
            format!("delivered={delivered} ce_released={ce_released}"),
        ),
        _ => {
            let verdict = if delivered {
                "PHYSICAL_HONOURED"
            } else if ce_released {
                "PHYSICAL_IGNORED_no_pattern_but_completed"
            } else {
                "PHYSICAL_REFUSED_no_completion_likely_RC"
            };
            // The arm ran to a result (we recorded the hardware's response), so it PASSES;
            // whether physical was honoured is the finding, reported here, read into the matrix.
            l.check(
                "behaviour_recorded",
                true,
                format!("{verdict} delivered={delivered} ce_released={ce_released}"),
            );
            println!("PHYS_ORACLE_FINDING arm={arm} deny={deny} {verdict}");
        }
    }
    Ok(())
}

/// Fill the first `COPY_LEN` bytes of VRAM object `obj` with the known pattern via a CPU view.
fn fill_vram(rm: &HostRm, obj: u32) -> Result<(), String> {
    let (_n, cpu) = rm
        .map_cpu(obj, OBJ_BYTES, CachePolicy::Uncached)
        .map_err(|e| format!("cpu src: {e:?}"))?;
    for i in 0..(COPY_LEN / 4) {
        cpu.store_u32(HostOffset::new(u64::from(i) * 4), PATTERN_BASE | i)
            .map_err(|e| format!("fill src: {e:?}"))?;
    }
    Ok(())
}
