//! **SPARSE RESIDENCY, GPU-free** (`docs/design/V3_SPARSE_RESIDENCY_RCA.md`).
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
//!
//! Option B (`unmapped-reads-zero`, [`GuestVaFill::Zero`]): S1 holds with it ON
//! ([`on_s1_guest_sparse_slots_read_zero`], [`on_s1_an_unbound_slot_returns_to_zero`]), and S2 is then
//! broken BY DESIGN ([`on_s2_consequence_guest_invalid_vas_read_zero_silently`] documents it). The
//! kf3 device default is ON; the model's own default is the unconfigured host, OFF: S2 holds there and
//! S1 is the known gap.

use super::*;
use kf_host::channel::GuestVaFill;

/// The model's reservation: 4 MiB at 4.5 GiB (the first guest range, `GUEST_VA_RANGES[0]`).
const R: u64 = (1 << 32) + (1 << 29);
const R_END: u64 = R + (4 << 20);
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

/// A model space whose static guest range has `fill`.
fn fresh(fill: GuestVaFill) -> Sim {
    let sim = Sim::new(space(R, R_END));
    sim.0.borrow_mut().fill = fill;
    sim
}

/// Build the residency image on one apply path and return the model and the committed set.
fn residency(fill: GuestVaFill, batching: bool, low: bool) -> (Sim, BTreeMap<u64, Committed>) {
    let sim = fresh(fill);
    let mut c = BTreeMap::new();
    {
        let m = SimMirror::new(&sim, batching, low);
        let out = apply_and_commit(&m, &residency_runs(), &mut c);
        assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
    }
    (sim, c)
}

const PATHS: [(bool, bool); 3] = [(false, false), (true, false), (true, true)];

/// The guest-SPARSE pages of the residency image that are NOT [`HostPte::Sparse`] on the host.
fn s1_violations(fill: GuestVaFill) -> Vec<String> {
    let mut bad = Vec::new();
    for (batching, low) in PATHS {
        let (sim, c) = residency(fill, batching, low);
        check(&sim, &c, &BTreeMap::new(), R, R_END).unwrap_or_else(|e| panic!("{e}"));
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
    bad
}

/// Slot 0 after the guest unbinds it back to SPARSE (`gpu_vaspace.c:2279-2284`): the walker emits an
/// UNMAP of the committed placement and nothing else. One mirror for both refreshes: the unmap goes
/// through the row record the map made.
fn unbound_slot_0(fill: GuestVaFill) -> Vec<(bool, bool, HostPte)> {
    PATHS
        .iter()
        .map(|&(batching, low)| {
            let sim = fresh(fill);
            let mut c = BTreeMap::new();
            let m = SimMirror::new(&sim, batching, low);
            let out = apply_and_commit(&m, &residency_runs(), &mut c);
            assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
            let out = apply_and_commit(&m, &[unmap_run(slot(0), &c[&slot(0)])], &mut c);
            assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
            let got = sim.0.borrow().host_pte(slot(0));
            (batching, low, got)
        })
        .collect()
}

/// What a guest-INVALID VA reads as on the host: a 4-page row mapped then plainly unmapped, plus
/// VAs never mapped, inside the static range.
fn guest_invalid_states(fill: GuestVaFill) -> Vec<(bool, bool, u64, HostPte)> {
    let mut out = Vec::new();
    for (batching, low) in PATHS {
        let sim = fresh(fill);
        let mut c = BTreeMap::new();
        let m = SimMirror::new(&sim, batching, low);
        let r = apply_and_commit(&m, &[map_run(R, 4 * P, true, 0x9000, false)], &mut c);
        assert_eq!(r.refused, 0, "{:?}", r.first_refusal);
        let r = apply_and_commit(&m, &[unmap_run(R, &c[&R])], &mut c);
        assert_eq!(r.refused, 0, "{:?}", r.first_refusal);
        let rm = sim.0.borrow();
        for p in [R, R + 3 * P, R + (2 << 20), R_END - P] {
            out.push((batching, low, p, rm.host_pte(p)));
        }
    }
    out
}

/// ★ **S1, RED by default (KNOWN GAP).** Every odd (guest-SPARSE) slot must read zero on the host
/// twin. With the default fill it is INVALID: the host GPU faults (or stalls) where the guest's GPU
/// returns zero. This is the mechanism proposed for the Xid 109 timeouts.
#[test]
#[ignore = "KNOWN-RED S1 (sparse residency, V3_SPARSE_RESIDENCY_RCA.md): with the default fill guest SPARSE PTEs are INVALID on the host twin; run with --ignored"]
fn s1_guest_sparse_slots_are_sparse_on_the_host() {
    let bad = s1_violations(GuestVaFill::Fault);
    assert!(
        bad.is_empty(),
        "S1 broken: {} guest-SPARSE pages are not SPARSE on the host, first: {}",
        bad.len(),
        bad[0]
    );
}

/// ★ S1 after an unbind back to SPARSE, default fill. RED.
#[test]
#[ignore = "KNOWN-RED S1 (sparse residency): with the default fill an unbind back to SPARSE leaves the host INVALID; run with --ignored"]
fn s1_an_unbound_slot_returns_to_sparse_on_the_host() {
    for (batching, low, got) in unbound_slot_0(GuestVaFill::Fault) {
        assert_eq!(
            got,
            HostPte::Sparse,
            "S1: batching={batching} low={low}: slot 0"
        );
    }
}

/// ★ RATCHET for the known gap (default fill): what the host holds at a guest-SPARSE slot, plus the
/// parts of the picture that are right. A default-path fix of S1 fails the INVALID assertion; that
/// edit retires this test and un-ignores the two S1 tests above.
#[test]
fn ratchet_guest_sparse_slots_are_invalid_on_the_host_today() {
    for (batching, low) in PATHS {
        let (sim, c) = residency(GuestVaFill::Fault, batching, low);
        check(&sim, &c, &BTreeMap::new(), R, R_END).unwrap_or_else(|e| panic!("{e}"));
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
                    "KNOWN GAP S1 (batching={batching} low={low} slot {i}): if this now reads SPARSE \
                     with the default fill, the gap is closed: retire this ratchet and un-ignore S1"
                );
            }
        }
    }
}

/// ★ **S2, GREEN with the default fill, the guard on any default-path fix.** A VA the guest left
/// INVALID (never mapped, or unmapped by a plain unmap) stays INVALID on the host, so a GPU access
/// faults as it does on bare metal and fails loudly in the guest. A default that reserved the guest
/// ranges with `_SPARSE` (the blanket fix) fails here.
#[test]
fn s2_guest_invalid_vas_stay_invalid_on_the_host() {
    for (batching, low, p, got) in guest_invalid_states(GuestVaFill::Fault) {
        assert_eq!(
            got,
            HostPte::Invalid,
            "S2: batching={batching} low={low}: guest-INVALID VA {p:#x} must fault on the host"
        );
    }
}

/// ★ **ON (`unmapped-reads-zero=on`): S1 holds.** Every guest-SPARSE page of the residency image
/// reads zero on the host, on every apply path; the bound slots stay mapped exactly (`check`).
#[test]
fn on_s1_guest_sparse_slots_read_zero() {
    let bad = s1_violations(GuestVaFill::Zero);
    assert!(
        bad.is_empty(),
        "S1 with the fill ON: {} pages, first: {}",
        bad.len(),
        bad[0]
    );
}

/// ★ ON: an unbind back to SPARSE reads zero again (host RM returns an unmap inside a `_SPARSE`
/// block to sparse, `gpu_vaspace.c:2279-2284`).
#[test]
fn on_s1_an_unbound_slot_returns_to_zero() {
    for (batching, low, got) in unbound_slot_0(GuestVaFill::Zero) {
        assert_eq!(
            got,
            HostPte::Sparse,
            "ON: batching={batching} low={low}: slot 0"
        );
    }
}

/// ★ **ON: the S2 CONSEQUENCE, documented so nobody is surprised.** With the fill ON, a VA the
/// guest left INVALID ALSO reads zero on the host: no fault, no RC, a silent zero. This is the
/// price of option B (`V3_SPARSE_RESIDENCY_RCA.md` §5; `docs/howto/installing.md`). If this test
/// ever fails, the property no longer means what its documentation says.
#[test]
fn on_s2_consequence_guest_invalid_vas_read_zero_silently() {
    for (batching, low, p, got) in guest_invalid_states(GuestVaFill::Zero) {
        assert_eq!(
            got,
            HostPte::Sparse,
            "ON: batching={batching} low={low}: guest-INVALID VA {p:#x} reads zero (documented)"
        );
    }
}

/// ★ ON is scoped to the static guest ranges: a VA outside them (here: below the reservation, the
/// unreserved `NV01` range) still faults, and so does a micro reservation's unmapped VA.
#[test]
fn on_is_scoped_to_the_static_guest_ranges() {
    for (batching, low) in PATHS {
        let sim = fresh(GuestVaFill::Zero);
        let rm = sim.0.borrow();
        assert_eq!(
            rm.host_pte(R - P),
            HostPte::Invalid,
            "batching={batching} low={low}"
        );
        assert_eq!(
            rm.host_pte(R_END),
            HostPte::Invalid,
            "batching={batching} low={low}"
        );
    }
    let sim = fresh(GuestVaFill::Zero);
    sim.0.borrow_mut().resv.insert(0x77, (0x10_0000, 0x20_0000));
    assert_eq!(
        sim.0.borrow().host_pte(0x10_0000),
        HostPte::Invalid,
        "a micro reservation keeps the plain flags"
    );
}
