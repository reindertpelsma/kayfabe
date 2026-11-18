//! ★ 2026-11-18 — **SPARSE RESIDENCY, GPU-free** (`docs/design/V3_SPARSE_RESIDENCY_RCA.md`).
//!
//! `[measured, box kf-openguest-172333, traces/open_rows_guest_20261117/confirm]` the Vulkan CTS
//! sparse-residency cases (`multisampled_image_sparse_residency.rgba16ui.samples_4`,
//! `texel_buffers.sparse_residency.*_size_2_24_r32ui`) time out in the kf3 guest with host Xid 109,
//! and their fully bound twins pass in the same boot.
//!
//! The guest-to-host path these tests follow:
//! - The guest's sparse VA reservation (`NV50_MEMORY_VIRTUAL` + `NVOS32_ALLOC_FLAGS_SPARSE`) is
//!   handled in the guest's own kernel RM and never reaches kf: a GSP client defaults to
//!   `bSplitVasManagementServerClientRm = NV_TRUE` (`ogkm-580 gpu_registry.c:171-182`), which skips
//!   the RPC (`virtual_mem.c:458-470`). `[measured]` the binding and residency cases issue the same
//!   288 `RmAlloc` RPCs.
//! - The guest RM sparsifies the range in its OWN page tables (`gpu_vaspace.c:1577-1600`) by writing
//!   SPARSE PTEs, `VALID=0 VOL=1` (`kern_gmmu_gm200.c:46-70`). Those bytes are the only form in which
//!   kf can see the sparse state.
//! - The walker COUNTS such a PTE (`kf_walk.cu` `kf_pte_sparse` → `sparse_slots`) and publishes no
//!   run for it, so `apply` never hears of the slot.
//! - The host twin's guest range is an `NV50_MEMORY_VIRTUAL` without `_SPARSE`
//!   (`kf_host::channel::NVOS32_RESERVE_FLAGS`). Its unmapped VAs are therefore INVALID (they fault)
//!   rather than SPARSE (they read zero).
//!
//! The invariants, by name:
//! - **S1 (sparse reads zero)**: a guest VA whose guest PTE is SPARSE is [`HostPte::Sparse`] on the
//!   host twin, never [`HostPte::Invalid`]. RED today: [`s1_guest_sparse_slots_are_sparse_on_the_host`].
//! - **S2 (invalid still faults)**: a guest VA the guest left INVALID (never mapped, or a plain
//!   unmap) is [`HostPte::Invalid`] on the host, never [`HostPte::Sparse`]. A blanket `_SPARSE`
//!   reservation would turn real faults, and the UVM demand-paging faults kf cannot service yet
//!   (`V3_BUILD.md` C: today they fail LOUDLY, a forwarded RC and CUDA 719), into silent zero reads.
//!   GREEN; it is the guard on any fix.

use super::*;

/// The model's reservation: 4 MiB at 4.5 GiB (the first guest range, `GUEST_VA_RANGES[0]`).
const R: u64 = (1 << 32) + (1 << 29);
const SLOT: u64 = 0x1_0000;
/// The residency pattern of the CTS buffer case (`sparseBindNdx += 2`) over 2 MiB: 32 slots of
/// 64 KiB, the even ones bound, the odd ones left SPARSE by the guest.
const SLOTS: u64 = 32;

fn slot(i: u64) -> u64 {
    R + i * SLOT
}

/// The walker's diff for the bound slots: one 64 KiB leaf per even slot. The odd slots contribute
/// NO run (`kf_walk.cu`: a SPARSE PTE only increments `sparse_slots`).
fn residency_runs() -> Vec<DiffRun> {
    (0..SLOTS)
        .step_by(2)
        .map(|i| DiffRun {
            leaf: SLOT,
            ..map_run(slot(i), SLOT, true, 0x4000_0000 + i * SLOT, false)
        })
        .collect()
}

/// Build the residency image on every apply path and return the model and the committed set.
fn residency(batching: bool, low: bool) -> (Sim, BTreeMap<u64, Committed>) {
    let sim = Sim::new(space(R, R + (4 << 20)));
    let mut c = BTreeMap::new();
    {
        let m = SimMirror::new(&sim, batching, low);
        let out = apply_and_commit(&m, &residency_runs(), &mut c);
        assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
    }
    (sim, c)
}

const PATHS: [(bool, bool); 3] = [(false, false), (true, false), (true, true)];

/// ★ **S1, RED today (KNOWN GAP).** Every odd (guest-SPARSE) slot must read zero on the host twin.
/// Today it is INVALID: the host GPU faults (or stalls) where the guest's GPU returns zero. This is
/// the mechanism proposed for the Xid 109 timeouts; the open link is in the module doc.
#[test]
#[ignore = "KNOWN-RED S1 (sparse residency, V3_SPARSE_RESIDENCY_RCA.md): guest SPARSE PTEs are not authored on the host twin; run with --ignored"]
fn s1_guest_sparse_slots_are_sparse_on_the_host() {
    let mut bad = Vec::new();
    for (batching, low) in PATHS {
        let (sim, _) = residency(batching, low);
        let rm = sim.0.borrow();
        for i in (1..SLOTS).step_by(2) {
            for p in [slot(i), slot(i) + SLOT - P] {
                let got = rm.host_pte(p);
                if got != HostPte::Sparse {
                    bad.push(format!(
                        "batching={batching} low={low} slot {i} va {p:#x}: {got:?}"
                    ));
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "S1 broken: {} guest-SPARSE pages are not SPARSE on the host, first: {}",
        bad.len(),
        bad[0]
    );
}

/// ★ S1 after a re-bind: the guest unbinds slot 0 and its RM restores it to SPARSE
/// (`gpu_vaspace.c:2279-2284`). The walker then emits an UNMAP of the committed placement and
/// nothing else, so slot 0 must be SPARSE on the host again. RED today.
#[test]
#[ignore = "KNOWN-RED S1 (sparse residency): an unbind back to SPARSE leaves the host INVALID; run with --ignored"]
fn s1_an_unbound_slot_returns_to_sparse_on_the_host() {
    for (batching, low) in PATHS {
        // One mirror for both refreshes: the unmap goes through the row record the map made.
        let sim = Sim::new(space(R, R + (4 << 20)));
        let mut c = BTreeMap::new();
        let m = SimMirror::new(&sim, batching, low);
        let out = apply_and_commit(&m, &residency_runs(), &mut c);
        assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
        let out = apply_and_commit(&m, &[unmap_run(slot(0), &c[&slot(0)])], &mut c);
        assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
        assert_eq!(
            sim.0.borrow().host_pte(slot(0)),
            HostPte::Sparse,
            "S1: batching={batching} low={low}: slot 0 unbound to SPARSE by the guest"
        );
    }
}

/// ★ RATCHET for the known gap: what the host holds TODAY at a guest-SPARSE slot, plus the parts of
/// the picture that are right. A fix of S1 fails the INVALID assertion; that edit retires this test
/// and un-ignores the two S1 tests above.
#[test]
fn ratchet_guest_sparse_slots_are_invalid_on_the_host_today() {
    for (batching, low) in PATHS {
        let (sim, c) = residency(batching, low);
        check(&sim, &c, &BTreeMap::new(), R, R + (4 << 20)).unwrap_or_else(|e| panic!("{e}"));
        let rm = sim.0.borrow();
        for i in 0..SLOTS {
            let got = rm.host_pte(slot(i) + 0x800);
            if i % 2 == 0 {
                assert_eq!(
                    got,
                    HostPte::Mapped((true, 0x4000_0000 + i * SLOT + 0x800)),
                    "a bound slot is mapped exactly (batching={batching} low={low} slot {i})"
                );
            } else {
                assert_eq!(
                    got,
                    HostPte::Invalid,
                    "KNOWN GAP S1 (batching={batching} low={low} slot {i}): if this now reads SPARSE, \
                     the gap is closed: retire this ratchet and un-ignore the S1 tests"
                );
            }
        }
    }
}

/// ★ **S2, GREEN, the guard on any fix.** A VA the guest left INVALID (never mapped, or unmapped
/// by a plain unmap) stays INVALID on the host, so that a GPU access faults the way it does on
/// bare metal and guest UVM demand paging still sees its faults. A fix that reserves the guest
/// ranges with `_SPARSE` (the blanket fix) fails here.
#[test]
fn s2_guest_invalid_vas_stay_invalid_on_the_host() {
    for (batching, low) in PATHS {
        let sim = Sim::new(space(R, R + (4 << 20)));
        let mut c = BTreeMap::new();
        let m = SimMirror::new(&sim, batching, low);
        let out = apply_and_commit(&m, &[map_run(R, 4 * P, true, 0x9000, false)], &mut c);
        assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
        let out = apply_and_commit(&m, &[unmap_run(R, &c[&R])], &mut c);
        assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
        let rm = sim.0.borrow();
        for p in [R, R + 3 * P, R + (2 << 20), R + (4 << 20) - P] {
            assert_eq!(
                rm.host_pte(p),
                HostPte::Invalid,
                "S2: batching={batching} low={low}: guest-INVALID VA {p:#x} must fault on the host"
            );
        }
    }
}
