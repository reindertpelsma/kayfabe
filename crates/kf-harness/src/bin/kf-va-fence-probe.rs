//! ★ 2026-11-18 — **where host RM places a GR twin's own buffers, with and without
//! `NV_VASPACE_ALLOCATION_FLAGS_VA_INTERNAL_LIMIT`** (`docs/design/V3_SPARSE_RESIDENCY_RCA.md` §7,
//! option C research).
//!
//! Question: can the guest ranges go WITHOUT kf's static fence reservation? They can if host RM's
//! internal placements (GR context buffers) stay out of `[4.5 GiB, 1 TiB - 64 GiB)`.
//! `VA_INTERNAL_LIMIT` routes internal allocations into `[4 GiB, 4.5 GiB)` (`vaspace_api.c:370-374`).
//! On a GSP host, client RM reserves that window for server RM (`gpu_vaspace.c:340-344`). So
//! the code-reading prediction is that GR context creation FAILS with the flag.
//!
//! For each arm (`default`, `internal_limit`): a bare space, a ring mapped FIXED in host RM's hole
//! (`HOST_HOLE_LO`, so our own map is never inside the guest range), a GR channel, its compute
//! object (RM builds the GR context here), schedule. Then: one FIXED reservation over the whole first
//! guest range, and a 2 MiB scan of its first 16 GiB that names the first occupied chunk.
//!
//! Falsifiers, stated before the run:
//! - Control `default` with `big_resv=OK` and no occupied chunk means the probe cannot see host RM's
//!   placements, and the arm proves nothing.
//! - `internal_limit` with `compute=OK` and `big_resv=OK` refutes the prediction: the fence could go.
//!
//! Prints `FENCE_PROBE ...` lines only; no guest, no GPU work submitted.

use kf_abi::submit::ENGINE_TYPE_GRAPHICS;
use kf_host::channel::{GUEST_VA_RANGES, HOST_HOLE_LO};
use kf_host::{HostRm, MapBacking, RingSpec};
use kf_linux_raw::DevDir;

const VA_INTERNAL_LIMIT: u32 = 1 << 7; // nvos.h NV_VASPACE_ALLOCATION_FLAGS_VA_INTERNAL_LIMIT
const RING_BYTES: u64 = 0x1_0000;
const CHUNK: u64 = 0x20_0000;
const SCAN: u64 = 16 << 30;

fn main() {
    let dev = match DevDir::open(c"/dev") {
        Ok(d) => d,
        Err(e) => {
            println!("FENCE_PROBE error=open_dev {e:?}");
            std::process::exit(2);
        }
    };
    let rm = match HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes) {
        Ok(r) => r,
        Err(e) => {
            println!("FENCE_PROBE error=open {e}");
            std::process::exit(2);
        }
    };
    println!("FENCE_PROBE driver={}", rm.driver_version());
    for (arm, flags) in [("default", 0u32), ("internal_limit", VA_INTERNAL_LIMIT)] {
        println!("{}", run_arm(&rm, arm, flags));
    }
}

fn run_arm(rm: &HostRm, arm: &str, flags: u32) -> String {
    let mut out = format!("FENCE_PROBE arm={arm} flags={flags:#x}");
    let space = match rm.alloc_vaspace_probe_flags(flags) {
        Ok(s) => s,
        Err(e) => return format!("{out} space=ERR({e:?})"),
    };
    out.push_str(" space=OK");
    let ring = match rm.alloc_device_local(RING_BYTES) {
        Ok(r) => r,
        Err(e) => return format!("{out} ring_obj=ERR({e:?})"),
    };
    let at = HOST_HOLE_LO + (1 << 30);
    let ring_va = match rm.map(space, ring, MapBacking::Dedicated, 0, RING_BYTES, Some(at), false) {
        Ok(v) => v,
        Err(e) => return format!("{out} ring_map=ERR({e:?})"),
    };
    let chan = match rm.birth_channel(
        space,
        ENGINE_TYPE_GRAPHICS,
        RingSpec {
            gp_fifo_va: ring_va,
            gp_fifo_entries: 128,
            userd_memory: ring,
            userd_offset: 0x8000,
            err_notifier: 0,
        },
    ) {
        Ok(c) => c,
        Err(e) => return format!("{out} channel=ERR({e:?})"),
    };
    out.push_str(" channel=OK");
    match rm.alloc_compute_object(chan) {
        Ok((_, class)) => out.push_str(&format!(" compute=OK({class:#x})")),
        Err(e) => return format!("{out} compute=ERR({e:?})"),
    }
    match rm.schedule(chan) {
        Ok(()) => out.push_str(" schedule=OK"),
        Err(e) => out.push_str(&format!(" schedule=ERR({e:?})")),
    }
    let (lo, hi) = GUEST_VA_RANGES[0];
    match rm.reserve_va(space.space, lo, hi - lo) {
        Ok(h) => {
            out.push_str(" big_resv=OK");
            let _ = rm.free(h);
        }
        Err(e) => out.push_str(&format!(" big_resv=ERR({e:?})")),
    }
    let mut occupied = Vec::new();
    let mut va = lo;
    while va < lo + SCAN && occupied.len() < 8 {
        match rm.reserve_va(space.space, va, CHUNK) {
            Ok(h) => {
                let _ = rm.free(h);
            }
            Err(_) => occupied.push(va),
        }
        va += CHUNK;
    }
    out.push_str(&format!(
        " occupied_chunks_first16g=[{}]",
        occupied.iter().map(|v| format!("{v:#x}")).collect::<Vec<_>>().join(",")
    ));
    rm.free_vaspace(space);
    out
}
