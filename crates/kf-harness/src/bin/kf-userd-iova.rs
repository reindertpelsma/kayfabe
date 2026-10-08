// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★ **USERD IOVA spike — does a SEPARATE one-page OS descriptor get a USERD address host RM
//! accepts, where a page inside the one 8 GiB guest-RAM descriptor does not?** (owner-level
//! question from the Windows run: `docs/design/V3_USERD_RELAY.md`, `the_three_channel_kinds.md`
//! §1.1.) Native only, like `kf-phys-oracle`: not in default CI, one case per process.
//!
//! ## The question, and what is known
//!
//! Windows' USERD lives in a slot of the guest RM's pool in guest system memory. The Passthrough
//! twin cannot adopt it: host RM refuses the channel with `NV_ERR_INVALID_ADDRESS` (0x1e) at
//! `kernel_channel_gv100.c:211-219`, because the page's host DMA address (an IOVA from the
//! host's translated IOMMU domain) was `0x7ff9_f969_b000` and `kchannelIsUserdAddrSizeValid_GA100`
//! wants `addr >> 32` to fit `NV_RAMRL_ENTRY_CHAN_USERD_PTR_HI_HW` (8 bits on this family, i.e.
//! `addr < 2^40`).
//!
//! **HYPOTHESIS (inferred, untested):** the page's IOVA is high only because it sits inside
//! kayfabe's single 8 GiB guest-RAM OS descriptor, whose one contiguous IOVA block cannot fit
//! below 4 GiB (Linux dma-iommu tries a 32-bit IOVA first for PCI devices and falls back to the
//! full device DMA mask, which RM sets to 47 bits); a SEPARATE tiny OS descriptor over just the
//! USERD page would get a low IOVA and pass.
//!
//! **FALSIFIER (stated before the run):** a channel birth whose `hUserdMemory` is a separate
//! one-page OS descriptor fails with the same invalid-address status.
//!
//! ## The cases (one process each)
//!
//! ```text
//! kf-userd-iova <case> [offset_hex]
//!   ctl_vram      rig control: USERD in a VRAM object (must be born)
//!   big           (A) ONE 8 GiB OS descriptor over the memfd; USERD = page at <offset>
//!   small_before  (B) a one-page descriptor over memfd page <offset> created BEFORE the big
//!                     8 GiB descriptor; USERD = the small one
//!   small_after   (B) the same, created AFTER the big descriptor
//!   small_alone   control: the one-page descriptor only, no big descriptor at all
//! ```
//!
//! Each case prints `MEASURE userd_iova` (the page's GPU-visible address, read through
//! `NV0041_CTRL_CMD_GET_SURFACE_PHYS_ATTR`, the same control `kf-phys-oracle` uses) and one
//! `USERD_IOVA_RESULT` line: `born` or `REFUSED status=0x..`, and whether the address fits 40 bits.
//! A refusal is a RESULT, not a failure of the harness; only a rig error (or a failed control)
//! fails the process.
//!
//! ⊘ **SAFETY, enforced by construction.** Everything is the process's own: a memfd it made, an
//! OS descriptor over it, a VRAM ring it allocated. The channel is born and freed; it is never
//! scheduled and no work is submitted, so nothing runs on the engine. The only host-memory page
//! RM sees is one of the memfd's. A hard in-process timeout (`KF_USERD_TIMEOUT_S`, default 150)
//! ends the process.

use kf_abi::submit::{ENGINE_TYPE_COPY0, Nv0041SurfacePhysAttr, NV0041_CTRL_CMD_GET_SURFACE_PHYS_ATTR};
use kf_harness::Ledger;
use kf_host::{HostRm, MapBacking, RingSpec, RmError};
use kf_linux_raw::{Backing, CachePolicy, DevDir, HostOffset, HostPageSize, HostProt, SharedRam};

/// The big object: the size of kayfabe's guest-RAM OS descriptor on the Windows VM.
const BIG_BYTES: u64 = 8 << 30;
const PAGE: u64 = 4096;
const RING_BYTES: u64 = 0x1_0000;
const GPFIFO_OFF: u64 = 0x1000;
const GPFIFO_ENTRIES: u32 = 64;
const VRAM_USERD_OFF: u64 = 0x3000;
/// 2^40: the largest USERD address `NV_RAMRL_ENTRY_CHAN_USERD_PTR_HI_HW` (8 bits) can carry.
const USERD_ADDR_LIMIT: u64 = 1 << 40;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let case = args.get(1).map(String::as_str).unwrap_or("ctl_vram").to_owned();
    let offset = args
        .get(2)
        .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0);
    let secs = std::env::var("KF_USERD_TIMEOUT_S")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(150);
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(secs));
        println!("USERD_IOVA_TIMEOUT after {secs}s");
        std::process::exit(3);
    });
    let mut l = Ledger::default();
    println!(
        "USERD_IOVA_START case={case} offset={offset:#x} pid={}",
        std::process::id()
    );
    if let Err(e) = run(&mut l, &case, offset) {
        l.check("run", false, e);
    }
    let v = l.verdict();
    println!(
        "USERD_IOVA_VERDICT case={case} offset={offset:#x} {}",
        if v { "PASS" } else { "FAIL" }
    );
    std::process::exit(i32::from(!v));
}

/// The page's GPU-visible address through `GET_SURFACE_PHYS_ATTR` at `offset` in `obj`.
fn phys_attr(rm: &HostRm, obj: u32, offset: u64) -> Result<(u64, u32), String> {
    let mut buf = [0u8; Nv0041SurfacePhysAttr::SIZE];
    Nv0041SurfacePhysAttr::encode_query(offset, &mut buf).map_err(|e| format!("encode: {e:?}"))?;
    rm.raw_control(obj, NV0041_CTRL_CMD_GET_SURFACE_PHYS_ATTR, &mut buf)
        .map_err(|e| format!("GET_SURFACE_PHYS_ATTR refused: {e:?}"))?;
    let a = Nv0041SurfacePhysAttr::decode(&buf).map_err(|e| format!("decode: {e:?}"))?;
    Ok((a.mem_offset, a.mem_aperture))
}

/// An OS descriptor over `[off, off+len)` of the memfd, through its own view. The view is leaked
/// on purpose: RM pinned the pages, the view is only the address RM read, and the process is short.
fn describe(rm: &HostRm, ram: &SharedRam, off: u64, len: u64) -> Result<u32, String> {
    let view = kf_linux_raw::MappedRegion::map(
        Backing::SharedFile {
            fd: ram.as_backing_fd(),
            offset: off,
        },
        len,
        HostProt::ReadWrite,
        CachePolicy::WriteBack,
        HostPageSize::query(),
    )
    .map_err(|e| format!("memfd view {off:#x}+{len:#x}: {e:?}"))?;
    let view: &'static kf_linux_raw::MappedRegion = Box::leak(Box::new(view));
    let t0 = std::time::Instant::now();
    let obj = rm
        .alloc_os_descriptor(view, HostOffset::new(0), len)
        .map_err(|e| format!("os descriptor {off:#x}+{len:#x}: {e:?}"))?;
    println!(
        "MEASURE descriptor_made memfd[{off:#x}+{len:#x}) obj={obj:#x} in {} ms",
        t0.elapsed().as_millis()
    );
    Ok(obj)
}

fn run(l: &mut Ledger, case: &str, offset: u64) -> Result<(), String> {
    if !offset.is_multiple_of(PAGE) || offset >= BIG_BYTES {
        return Err(format!("offset {offset:#x} not a page inside the 8 GiB object"));
    }
    let dev = DevDir::open(c"/dev").map_err(|e| format!("open /dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    l.measure("session", format!("driver {}", rm.driver_version()));
    let (_f, arch, _i) = rm.arch_info();
    l.measure("arch", format!("{arch:#x}"));

    // The memfd the harness owns (not the guest's). Lazily populated: only what RM pins is real.
    let ram = SharedRam::create_named(c"kf-userd-iova", BIG_BYTES)
        .map_err(|e| format!("memfd {BIG_BYTES:#x}: {e:?}"))?;

    // The descriptors this case needs, in this case's order.
    let mut keep: Vec<u32> = Vec::new();
    // (object holding USERD, offset of the USERD page inside that object)
    let userd: Option<(u32, u64)> = match case {
        "ctl_vram" => None,
        "big" => {
            let big = describe(&rm, &ram, 0, BIG_BYTES)?;
            keep.push(big);
            Some((big, offset))
        }
        "small_before" => {
            let small = describe(&rm, &ram, offset, PAGE)?;
            let big = describe(&rm, &ram, 0, BIG_BYTES)?;
            keep.extend([small, big]);
            Some((small, 0))
        }
        "small_after" => {
            let big = describe(&rm, &ram, 0, BIG_BYTES)?;
            let small = describe(&rm, &ram, offset, PAGE)?;
            keep.extend([big, small]);
            Some((small, 0))
        }
        "small_alone" => {
            let small = describe(&rm, &ram, offset, PAGE)?;
            keep.push(small);
            Some((small, 0))
        }
        other => return Err(format!("unknown case {other}")),
    };

    // The page's GPU-visible address, as RM reports it (the number the 40-bit check sees).
    let mut iova: Option<u64> = None;
    if let Some((obj, off)) = userd {
        let (a, ap) = phys_attr(&rm, obj, off)?;
        l.measure(
            "userd_iova",
            format!(
                "{a:#x} aperture={ap} (SYSMEM=1) fits_40bit={} bits={}",
                a < USERD_ADDR_LIMIT,
                64 - a.leading_zeros()
            ),
        );
        iova = Some(a);
    }

    let space = rm.alloc_vaspace().map_err(|e| format!("vaspace: {e:?}"))?;
    let ring = rm
        .alloc_device_local(RING_BYTES)
        .map_err(|e| format!("ring obj: {e:?}"))?;
    let ring_va = rm
        .map(space, ring, MapBacking::Dedicated, 0, RING_BYTES, None, false)
        .map_err(|e| format!("map ring: {e:?}"))?;
    rm.invalidate_tlb(space)
        .map_err(|e| format!("invalidate: {e:?}"))?;

    let (userd_memory, userd_offset) = userd.unwrap_or((ring, VRAM_USERD_OFF));
    let born = rm.birth_channel(
        space,
        ENGINE_TYPE_COPY0,
        RingSpec {
            gp_fifo_va: ring_va + GPFIFO_OFF,
            gp_fifo_entries: GPFIFO_ENTRIES,
            userd_memory,
            userd_offset,
            err_notifier: 0,
        },
    );
    let fits = iova.map(|a| a < USERD_ADDR_LIMIT);
    match born {
        Ok(chan) => {
            println!(
                "USERD_IOVA_RESULT case={case} offset={offset:#x} born iova={} fits_40bit={fits:?}",
                iova.map_or_else(|| "vram".to_owned(), |a| format!("{a:#x}"))
            );
            if case == "ctl_vram" {
                l.check("control_born", true, "VRAM USERD birth works (rig is sound)");
            }
            let _ = rm.free(chan.chan);
            let _ = rm.free(chan.tsg);
        }
        Err(e) => {
            let status = match e {
                RmError::Other(s) => format!("{s:#x}"),
                ref other => format!("{other:?}"),
            };
            println!(
                "USERD_IOVA_RESULT case={case} offset={offset:#x} REFUSED status={status} iova={} fits_40bit={fits:?}",
                iova.map_or_else(|| "vram".to_owned(), |a| format!("{a:#x}"))
            );
            // A refusal of a sysmem case is the finding; a refused VRAM control is a broken rig.
            l.check(
                "birth_outcome_recorded",
                case != "ctl_vram",
                format!("refused: {e:?}"),
            );
        }
    }
    if iova.is_some() {
        l.check("outcome_recorded", true, "sysmem case ran to a result");
    }
    for h in keep.into_iter().rev() {
        let _ = rm.free(h);
    }
    Ok(())
}
