//! ★★★ **Micro-reservation probe — THE decision point for batching the unreserved low range**
//! (`docs/design/V3_BATCHED_MAP.md` §8.1). NOT run by this branch; the owner runs it on the GPU host.
//!
//! The question: can an UNPRIVILEGED host client reserve a SMALL fixed VA range inside
//! `[1 MiB, 4.5 GiB)` of a twin space (`NV50_MEMORY_VIRTUAL`, `FIXED_ADDRESS_ALLOCATE | LAZY |
//! VIRTUAL`, `kf_host::HostRm::reserve_va`), map a scattered batch THROUGH it, and unmap part of it
//! EXACTLY (the rest still readable by the GPU)? `[measured gfx8]` RM refused ONE LARGE reservation
//! of `[1 MiB, 4 GiB)` with `NoMemory`; in ogkm-595.84 a FIXED reservation is a FIXED `eheapAlloc`
//! (`gpu_vaspace.c:1374-1386`, `NV_ERR_NO_MEMORY` when ANY existing heap block overlaps), and the
//! only VA the client RM itself withholds in a GSP-client space is the split window
//! `[4 GiB, 4.5 GiB)` (`gpu_vaspace.c:394-467`, `g_gpu_vaspace_nvoc.h:90-91`) — so a small range
//! over FREE VA is expected to be accepted (inferred, which is what this measures).
//!
//! ```text
//! KF3_WIN_USER_CHANNELS_PASSTHROUGH=1 kf-micro-reserve-probe reserve        # the decision
//! KF3_WIN_USER_CHANNELS_PASSTHROUGH=1 kf-micro-reserve-probe nv01-control   # the root cause, on hardware
//! ```
//!
//! - `reserve` (safe): a census of small reservations at Windows-like VAs (and the old LARGE one),
//!   then: reserve 8 pages at `0x4030000`, map 8 pages of a VRAM object of ours THROUGH it as ONE
//!   mapping, unmap page 3 by range, invalidate, and CE-copy pages 0 and 7 (must deliver their
//!   pattern), re-map page 3 into the hole (must deliver), take it all down, free the reservation,
//!   and show its VA is free again. PASS ⇒ batch the low range with micro reservations
//!   (`KF3_BATCH_MICRO_RESERVE=1` → default). FAIL at `reserve_*` ⇒ leaf-granular maps there.
//! - `nv01-control` (raises ONE host Xid 31 on this test's OWN channel — a result, not a failure):
//!   the same partial unmap through the space's `NV01` range. A FIXED one-page map at the remnant's
//!   start then SUCCEEDS (⇔ RM freed the remnant's whole VA block) and a CE read of page 7 FAULTS
//!   (`FAULT_PTE`); at process exit dmesg shows `NULL != pMemBlock @ gpu_vaspace.c:1639`. That is
//!   runs 242/243 reproduced in isolation.
//!
//! ⊘ Every object is this process's own; nothing is shared with a guest.

use kf_harness::{CeRig, Ledger};
use kf_host::{HostRm, MapBacking, MapPerm};
use kf_linux_raw::{CachePolicy, DevDir, HostOffset};

const P: u64 = 0x1000;
const PAGES: u64 = 8;
const OBJ_BYTES: u64 = 0x1_0000;
const PATTERN: u32 = 0x5EED_0000;
/// The run-243 neighbourhood (`0x4034000` faulted): an unreserved Windows process VA.
const V_RESERVE: u64 = 0x0403_0000;
const V_CONTROL: u64 = 0x0503_0000;

fn word(page: u64, w: u64) -> u32 {
    PATTERN | ((page as u32) << 8) | (w as u32 & 0xFF)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arm = args.get(1).map_or("reserve", String::as_str);
    let mut l = Ledger::default();
    println!("MICRO_RESERVE_START arm={arm} pid={}", std::process::id());
    if let Err(e) = run(&mut l, arm) {
        l.check("run", false, e);
    }
    let v = l.verdict();
    println!(
        "MICRO_RESERVE_VERDICT arm={arm} {}",
        if v { "PASS" } else { "FAIL" }
    );
    std::process::exit(i32::from(!v));
}

#[allow(clippy::too_many_lines)]
fn run(l: &mut Ledger, arm: &str) -> Result<(), String> {
    let dev = DevDir::open(c"/dev").map_err(|e| format!("open /dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    l.measure("session", format!("driver {}", rm.driver_version()));
    l.measure(
        "space_start",
        format!("{:?}", kf_host::channel::MirrorVaStart::from_env()),
    );
    let space = rm.alloc_vaspace().map_err(|e| format!("vaspace: {e:?}"))?;

    // ── the census: small FIXED reservations at Windows-like VAs, and the old LARGE one ──────
    let census: [(u64, u64, &str); 6] = [
        (0x0040_0000, 0x8000, "4 MiB, 8 pages"),
        (V_RESERVE, 0x8000, "run-243 VA, 8 pages"),
        (0x1453_0000, 0x1_0000, "0x14530000, 64 KiB"),
        (0x8000_0000, 0x20_0000, "2 GiB, 2 MiB"),
        (0xFFE0_0000, 0x20_0000, "just below 4 GiB, 2 MiB"),
        (
            0x10_0000,
            (4 << 30) - 0x10_0000,
            "LARGE [1 MiB, 4 GiB) (the gfx8 refusal)",
        ),
    ];
    let mut accepted = 0;
    for (va, len, what) in census {
        match rm.reserve_va(space.space, va, len) {
            Ok(h) => {
                accepted += 1;
                l.measure(
                    "reserve_census",
                    format!("{what} {va:#x}+{len:#x}: ACCEPTED"),
                );
                let _ = rm.free(h);
            }
            Err(e) => l.measure(
                "reserve_census",
                format!("{what} {va:#x}+{len:#x}: refused {e:?}"),
            ),
        }
    }
    l.check(
        "reserve_small_accepted",
        accepted >= 4,
        format!("{accepted}/6 accepted (the 5 small ones decide)"),
    );

    // ── the objects: a VRAM source of 8 patterned pages, a VRAM destination, a CE rig ────────
    let src = rm
        .alloc_device_local(OBJ_BYTES)
        .map_err(|e| format!("src: {e:?}"))?;
    let (_sn, src_cpu) = rm
        .map_cpu(src, OBJ_BYTES, CachePolicy::Uncached)
        .map_err(|e| format!("cpu src: {e:?}"))?;
    for p in 0..PAGES {
        for w in 0..(P / 4) {
            src_cpu
                .store_u32(HostOffset::new(p * P + w * 4), word(p, w))
                .map_err(|e| format!("fill: {e:?}"))?;
        }
    }
    let dst = rm
        .alloc_device_local(OBJ_BYTES)
        .map_err(|e| format!("dst: {e:?}"))?;
    let dst_va = rm
        .map(space, dst, MapBacking::Dedicated, 0, OBJ_BYTES, None, false)
        .map_err(|e| format!("map dst: {e:?}"))?;
    let (_dn, dst_cpu) = rm
        .map_cpu(dst, OBJ_BYTES, CachePolicy::Uncached)
        .map_err(|e| format!("cpu dst: {e:?}"))?;
    let mut rig = CeRig::new(&rm, space)?;
    // Copy one page from `va` into dst page `slot` and say whether page `page`'s pattern arrived.
    let mut read = |rm: &HostRm, va: u64, slot: u64, page: u64| -> Result<bool, String> {
        for w in 0..4 {
            dst_cpu
                .store_u32(HostOffset::new(slot * P + w * 4), 0xDEAD_0000)
                .map_err(|e| format!("stamp: {e:?}"))?;
        }
        let s = rig.copy(rm, va, dst_va + slot * P, P as u32)?;
        let got = dst_cpu
            .load_u32(HostOffset::new(slot * P + 4))
            .map_err(|e| format!("read: {e:?}"))?;
        Ok(s.ce_released && got == word(page, 1))
    };

    match arm {
        "reserve" => {
            let h = rm
                .reserve_va(space.space, V_RESERVE, PAGES * P)
                .map_err(|e| format!("reserve {V_RESERVE:#x}: {e:?}"))?;
            rm.map_in(
                h,
                src,
                MapBacking::SharedSlice,
                0,
                PAGES * P,
                V_RESERVE,
                false,
                0,
                MapPerm::READ_WRITE,
            )
            .map_err(|e| format!("map through the reservation: {e:?}"))?;
            l.check(
                "whole_read_before",
                read(&rm, V_RESERVE + 7 * P, 0, 7)?,
                "page 7 via the reservation",
            );
            rm.unmap_in(h, V_RESERVE + 3 * P, P, false)
                .map_err(|e| format!("partial unmap: {e:?}"))?;
            rm.invalidate_tlb(space)
                .map_err(|e| format!("invalidate: {e:?}"))?;
            l.check(
                "remnant_left_reads",
                read(&rm, V_RESERVE, 1, 0)?,
                "page 0 after unmapping page 3",
            );
            l.check(
                "remnant_right_reads",
                read(&rm, V_RESERVE + 7 * P, 2, 7)?,
                "page 7 after unmapping page 3",
            );
            rm.map_in(
                h,
                src,
                MapBacking::SharedSlice,
                3 * P,
                P,
                V_RESERVE + 3 * P,
                false,
                0,
                MapPerm::READ_WRITE,
            )
            .map_err(|e| format!("re-map into the hole: {e:?}"))?;
            rm.invalidate_tlb(space)
                .map_err(|e| format!("invalidate: {e:?}"))?;
            l.check(
                "hole_remapped_reads",
                read(&rm, V_RESERVE + 3 * P, 3, 3)?,
                "page 3 re-mapped",
            );
            rm.unmap_in(h, V_RESERVE, PAGES * P, false)
                .map_err(|e| format!("teardown range: {e:?}"))?;
            rm.free(h).map_err(|e| format!("free reservation: {e:?}"))?;
            let back = rm.map_kind(
                space,
                src,
                MapBacking::SharedSlice,
                0,
                P,
                Some(V_RESERVE),
                false,
                0,
                MapPerm::READ_WRITE,
            );
            l.check(
                "va_free_after",
                back.is_ok(),
                format!("NV01 map at the freed VA: {back:?}"),
            );
            Ok(())
        }
        "nv01-control" => {
            rm.map_kind(
                space,
                src,
                MapBacking::SharedSlice,
                0,
                PAGES * P,
                Some(V_CONTROL),
                false,
                0,
                MapPerm::READ_WRITE,
            )
            .map_err(|e| format!("map through the NV01 range: {e:?}"))?;
            l.check(
                "control_whole_read_before",
                read(&rm, V_CONTROL + 7 * P, 0, 7)?,
                "page 7 before",
            );
            rm.unmap_range(space, V_CONTROL + 3 * P, P, false)
                .map_err(|e| format!("partial unmap: {e:?}"))?;
            rm.invalidate_tlb(space)
                .map_err(|e| format!("invalidate: {e:?}"))?;
            // Signature 1: the remnant's VA block is free (a FIXED map at its start lands).
            let probe = rm.map_kind(
                space,
                src,
                MapBacking::SharedSlice,
                0,
                P,
                Some(V_CONTROL),
                false,
                0,
                MapPerm::READ_WRITE,
            );
            l.measure("control_probe_map_at_remnant", format!("{probe:?}"));
            l.check(
                "nv01_split_freed_the_block",
                probe.is_ok(),
                "Ok ⇔ RM freed the remnant's VA block (the root cause)",
            );
            if probe.is_ok() {
                let _ = rm.unmap(space, V_CONTROL, false);
            }
            // Signature 2: page 7 (a remnant RM still lists) no longer reads — EXPECTED Xid 31 on
            // this test's own channel; the copy's semaphore never lands.
            let r7 = read(&rm, V_CONTROL + 7 * P, 1, 7).unwrap_or(false);
            l.measure("control_remnant_read", format!("{r7}"));
            l.check(
                "nv01_remnant_lost",
                !r7,
                "a remnant of a split NV01 mapping faults (expected)",
            );
            println!(
                "MICRO_RESERVE_NOTE check dmesg: Xid 31 FAULT_PTE at {:#x}, and at exit NULL != pMemBlock @ gpu_vaspace.c:1639",
                V_CONTROL + 7 * P
            );
            Ok(())
        }
        other => Err(format!("unknown arm {other:?} (reserve | nv01-control)")),
    }
}
