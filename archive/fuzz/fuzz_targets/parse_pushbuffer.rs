//! Coverage-guided fuzz of the ONE pushbuffer parser (`kayfabe_fwd::parse_pushbuffer`)
//! — the one place raw, adversarial guest *bytes* reach the Mode-2 core.
//!
//! The invariant under fuzz (boundary-1 posture): for ANY input bytes the decoder
//! (a) never panics / never overflows, (b) respects its byte-total + per-range caps
//! (bounded work), and (c) only ever emits well-formed structured ops or a clean
//! `Err`. Any bound table entry must resolve back to its own binding (no torn state).
//!
//! Input is `arbitrary`-structured so the fuzzer reaches DEEP paths (real GPFIFO
//! ranges + method words) instead of bouncing off early rejects: a bounded set of
//! declared `(va, len)` GPFIFO entries plus the method-region bytes those entries
//! point at, all backed into the mock guest RAM before the parse.
//!
//! ★★★ A GPFIFO entry names a GPU **virtual** address, and `read_pushbuffer` resolves it
//! through the issuing channel's address table before it fetches a byte
//! (`execution_plane_increments.md` §8.2.3). So the harness installs ONE wide binding at
//! `kayfabe_tests::PB_VA_BIAS` — without it every generated entry would refuse as an
//! address-table MISS and the guest-memory read would go **unreachable**, which is a
//! coverage collapse that no assertion here would have noticed.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use kayfabe_arch::ids::{GpuId, GpuVa, HClient, Pdb};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_fwd::parse_pushbuffer;
use kayfabe_mocks::{MockArch, MockIsolateFactory, MockVmm};
use kayfabe_tests::{PB_VA_BIAS, Scenario, bind_ring_at, identical_handles, pb_va};
use kayfabe_vmm::Vmm;

const A_PDB: Pdb = Pdb(0x3401_000);
/// Window the fuzzed GPFIFO entries may point their ranges at (mirrors the tests).
const REGION_GPA: u64 = 0x5000_0000;

/// A structured, bounded fuzz input. `raw_ring` is fed to the parser verbatim as the
/// GPFIFO ring bytes; `entries` are *also* materialized into a valid ring so the
/// fuzzer easily reaches the method-decode paths (and `region` is what those ranges
/// resolve to in guest RAM). `use_structured` switches between the two so the fuzzer
/// explores both the hand-rolled-byte and the well-formed-ring shapes.
#[derive(Arbitrary, Debug)]
struct Input {
    use_structured: bool,
    /// Declared GPFIFO entries `(gpa_choice, len)` — bounded count so total work stays
    /// small; `len` is intentionally allowed to be huge/zero/wrapping.
    entries: Vec<(u8, u64)>,
    /// The method-region bytes the ranges resolve to (the actual method words).
    region: Vec<u8>,
    /// Raw ring bytes, used when `use_structured` is false (fully hostile shape).
    raw_ring: Vec<u8>,
}

fn one_proc_gpu() -> (Gpu, MockVmm) {
    let arch = Box::new(MockArch::new());
    let (factory, _rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process(HClient(0xAA), A_PDB, identical_handles(0x10, 0x11));
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
    (gpu, MockVmm::new())
}

/// Build a 16-byte-per-entry mock GPFIFO ring from declared `(gpa, len)` pairs. The
/// `gpa_choice` byte selects among a few plausible windows (the backed region, a
/// near-`u64::MAX` window to probe wrap paths, and an unbacked window).
fn build_ring(entries: &[(u8, u64)]) -> Vec<u8> {
    let mut ring = Vec::with_capacity(entries.len().min(32) * 16);
    for &(choice, len) in entries.iter().take(32) {
        // Four VA shapes, and the fourth is deliberately OUTSIDE the harness's binding so
        // the address-table MISS arm is reached as often as the read arm.
        let va = match choice % 4 {
            0 => pb_va(REGION_GPA).0,
            1 => pb_va(REGION_GPA).0 + u64::from(choice) * 4, // mid-region, unaligned
            2 => 0xFFFF_FFFF_FFFF_F000,                       // near-top: caps/wrap + MISS
            _ => pb_va(0x9000_0000).0,                        // bound, unbacked (reads zero)
        };
        ring.extend_from_slice(&va.to_le_bytes());
        ring.extend_from_slice(&len.to_le_bytes());
    }
    ring
}

fuzz_target!(|input: Input| {
    let (mut gpu, mut vmm) = one_proc_gpu();
    let pid = *gpu.spine.by_pdb.get(&(GpuId::ZERO, A_PDB)).unwrap();
    let cid = *gpu.procs[&pid].chan_ids.values().next().unwrap();

    // Back the method region the GPFIFO ranges resolve to.
    if !input.region.is_empty() {
        vmm.gpa_write(REGION_GPA, &input.region).unwrap();
    }
    // The one wide binding — see the module docs. `[PB_VA_BIAS, +512 GiB)` -> GPA 0, which
    // is exactly `pb_va` realized as a mapping.
    bind_ring_at(&mut gpu, pid, cid, GpuVa(PB_VA_BIAS), 0, 1 << 39);

    let ring = if input.use_structured {
        build_ring(&input.entries)
    } else {
        input.raw_ring.clone()
    };

    // (a) Never panics — a Result, at worst a loud fault.
    let _ = parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring);

    // (c) No torn state: every bound VA resolves back to its OWN binding.
    let vas = &gpu.procs[&pid].vases[&(GpuId::ZERO, A_PDB)];
    for (va, _len, b) in vas.table.iter() {
        let got = vas.table.resolve(A_PDB, GpuVa(va)).map(|(x, _)| x.phys);
        assert_eq!(
            got,
            Ok(b.phys),
            "a bound VA must resolve to its own binding"
        );
    }
});
