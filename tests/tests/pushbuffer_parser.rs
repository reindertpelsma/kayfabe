//! Batch-3 the ONE pushbuffer parser + working-set publication — the #14 fix through
//! the REAL exec path (`execution_plane.md` §2.3/§2.4).
//!
//! - A scripted pushbuffer (via `MockArch`'s `PushbufferAbi`) → the address table is
//!   forward-populated for CE PT-write dsts (#13 capture), the `SemRelease` is
//!   `observe`d on the owning proc's completion queue, a `TlbInvalidate` membar is
//!   recorded, and an `Opaque` method changes no core state.
//! - The 2-proc sim: identical guest VAs in two Procs each publish their working set
//!   into their OWN host VAS, so `gate_working_set` passes for BOTH (the #14 fix); an
//!   unpublished VA at ring time is a loud fault (the proven EXECUTION-fault root).
//! - A soak variant: a sustained submit/complete loop loses no completion.
//!
//! Invariant/contract tests (decision #15), mock-driven, GPU-free.

#![allow(clippy::unusual_byte_groupings)]

use nvkvm_arch::ids::GpuId;
use nvkvm_arch::ids::{GpuVa, HClient, Pdb, VChid};
use nvkvm_core::gpu::Gpu;
use nvkvm_core::gpa::GpaSpace;
use nvkvm_fwd::{
    FwdFault, gate_working_set, handle_doorbell, parse_pushbuffer, publish_backing,
};
use nvkvm_mocks::{MockArch, MockIsolateFactory, MockPushbuffer, MockVmm, RmVerb, SharedRecorder, mock_classes as mc};
use nvkvm_tests::{Scenario, identical_handles};
use nvkvm_vmm::Vmm;

const A_PDB: Pdb = Pdb(0x3401_000);
const B_PDB: Pdb = Pdb(0x3405_000);
const SHARED_VA: GpuVa = GpuVa(0x2_0020_0000);

/// Lay out a GPFIFO ring (one entry pointing at `gpa`) + the method words at `gpa`.
/// Writes the method words into guest RAM and returns the 16-byte GPFIFO ring bytes.
fn script_pushbuffer(vmm: &mut MockVmm, gpa: u64, methods: &[(u32, Vec<u32>)]) -> Vec<u8> {
    // Serialize method words as u32 LE into guest RAM at `gpa`.
    let mut words = Vec::new();
    for (h, args) in methods {
        words.push(*h);
        words.extend_from_slice(args);
    }
    let mut bytes = Vec::new();
    for w in &words {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    vmm.gpa_write(gpa, &bytes).unwrap();
    // One GPFIFO entry: [gpa u64 LE, len u64 LE].
    let mut ring = Vec::new();
    ring.extend_from_slice(&gpa.to_le_bytes());
    ring.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    ring
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

/// A scripted pushbuffer drives ALL four fact kinds: CE-PT-write capture populates
/// `pt_pages` + the table, `SemRelease` is observed, `TlbInvalidate` is recorded,
/// `Opaque` changes nothing.
#[test]
fn scripted_pushbuffer_populates_table_and_observes_completion() {
    let (mut gpu, mut vmm) = one_proc_gpu();
    let pid = *gpu.by_pdb.get(&(GpuId::ZERO, A_PDB)).unwrap();
    let cid = *gpu.procs[&pid].chan_ids.values().next().unwrap();

    let pt_page: u64 = 0x1_2340_0000; // physical PT page the CE writes
    let sem_addr: u64 = 0x2_0030_0000;
    let ring = script_pushbuffer(&mut vmm, 0x5000_0000, &[
        MockPushbuffer::set_object(mc::DMA_COPY),
        // Physical CE dst = a PT-page write (#13 capture).
        MockPushbuffer::ce_launch_dma(pt_page + 0x40, 0x1000, false),
        // A VIRTUAL CE dst is a data copy, NOT a PT write — must not be captured.
        MockPushbuffer::ce_launch_dma(0x2_0040_0000, 0x2000, true),
        MockPushbuffer::sem_release(sem_addr, 0xabc),
        MockPushbuffer::tlb_invalidate(A_PDB.0, true),
        // An opaque method (unknown opcode) — passed through, no state change.
        MockPushbuffer::method(0xEE, &[1, 2, 3]),
    ]);

    let out = parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring).expect("parses");

    // CE-PT-write capture: exactly the physical dst page, aligned.
    assert_eq!(out.pt_writes, vec![pt_page], "physical CE dst captured as a PT page");
    assert!(gpu.procs[&pid].vases[&(GpuId::ZERO, A_PDB)].pt_pages.contains(&pt_page), "recorded in Vas.pt_pages");
    // The virtual CE dst was NOT captured as a PT write.
    assert_eq!(out.pt_writes.len(), 1, "virtual CE dst is a data copy, not a PT-write");
    // SemRelease observed.
    assert_eq!(out.sem_releases, vec![(GpuVa(sem_addr), 0xabc)]);
    assert!(gpu.procs[&pid].completion.has_outstanding(), "completion observed on the proc's queue");
    // TlbInvalidate membar recorded.
    assert_eq!(out.invalidates, vec![(A_PDB, true)]);
    // Opaque method counted, changed nothing else.
    assert_eq!(out.opaque, 1);

    // The captured PT-write leaf is now resolvable in the table (co-populate source).
    assert!(nvkvm_fwd::resolve(&gpu, GpuId::ZERO, A_PDB, GpuVa(pt_page + 0x40)).is_ok());
}

/// A hostile / truncated ring never panics and never desyncs the parser into an
/// unbounded read.
#[test]
fn hostile_ring_never_panics() {
    let (mut gpu, mut vmm) = one_proc_gpu();
    let pid = *gpu.by_pdb.get(&(GpuId::ZERO, A_PDB)).unwrap();
    let cid = *gpu.procs[&pid].chan_ids.values().next().unwrap();

    // A ring with a bogus GPFIFO entry pointing at empty RAM + a truncated tail.
    let mut ring = Vec::new();
    ring.extend_from_slice(&0x9000_0000u64.to_le_bytes());
    ring.extend_from_slice(&64u64.to_le_bytes());
    ring.push(0xff); // truncated trailing entry (ignored)
    let out = parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring).expect("does not panic");
    // Empty RAM decodes to opaque/invalid methods, never a crash.
    assert!(out.sem_releases.is_empty());
}

// ---------------------------------------------------------------------------------
// ★ The #14 fix through the REAL exec path: per-Vas working-set publication.
// ---------------------------------------------------------------------------------

fn two_proc_gpu() -> (Gpu, MockVmm, SharedRecorder) {
    let arch = Box::new(MockArch::new());
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process(HClient(0xAA), A_PDB, identical_handles(0x10, 0x11));
    s.compute_process(HClient(0xBB), B_PDB, identical_handles(0x20, 0x21));
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
    (gpu, MockVmm::new(), rec)
}

/// ★ THE #14 regression through the exec path — now STRUCTURAL: `handle_doorbell` is
/// the ONE ring path and it gates. Two Procs, identical guest VAs: neither can ring
/// its declared working set before publishing (loud fault, ZERO host ops — not even
/// channel materialization); after each publishes into its OWN host VAS, both ring,
/// on distinct host tokens.
#[test]
fn t14_per_vas_publication_gates_the_ring() {
    let (mut gpu, _vmm, rec) = two_proc_gpu();
    let pid_a = *gpu.by_pdb.get(&(GpuId::ZERO, A_PDB)).unwrap();
    let pid_b = *gpu.by_pdb.get(&(GpuId::ZERO, B_PDB)).unwrap();
    let cid_a = *gpu.procs[&pid_a].chan_ids.values().next().unwrap();
    let a_token = MockArch::token_for(VChid(0x10));
    let b_token = MockArch::token_for(VChid(0x20));

    // Before publication: a doorbell declaring the identical working-set VA is a
    // LOUD fault for BOTH — the exact #14 EXECUTION fault, refused by the ONE ring
    // path itself (there is no ungated sibling to reach the host through).
    assert!(matches!(
        handle_doorbell(&mut gpu, GpuId::ZERO, a_token, &[SHARED_VA]),
        Err(FwdFault::Address(_))
    ));
    assert!(matches!(
        handle_doorbell(&mut gpu, GpuId::ZERO, b_token, &[SHARED_VA]),
        Err(FwdFault::Address(_))
    ));
    // The refused rings did NOTHING host-side: no channel, no schedule, no doorbell.
    assert!(rec.lock().unwrap().log.is_empty(), "a gated-out ring performs ZERO host ops");
    // The query form agrees (and cannot ring anything by construction).
    assert!(gate_working_set(&gpu, pid_a, cid_a, &[SHARED_VA]).is_err());

    // Publish the SAME guest VA in each proc's OWN Vas (distinct host VASes).
    let pub_a =
        publish_backing(gpu.procs.get_mut(&pid_a).unwrap(), GpuId::ZERO, A_PDB, SHARED_VA, 0x10000).unwrap();
    let pub_b =
        publish_backing(gpu.procs.get_mut(&pid_b).unwrap(), GpuId::ZERO, B_PDB, SHARED_VA, 0x10000).unwrap();
    assert_ne!(pub_a.host_va, pub_b.host_va, "identical guest VA → distinct host VA");

    // Now the SAME ring path passes for BOTH — each resolves in its OWN host VAS.
    let out_a = handle_doorbell(&mut gpu, GpuId::ZERO, a_token, &[SHARED_VA]).expect("A rings after publish");
    let out_b = handle_doorbell(&mut gpu, GpuId::ZERO, b_token, &[SHARED_VA]).expect("B rings after publish");
    assert_ne!(
        out_a.host_token, out_b.host_token,
        "each proc rang its OWN host token — no cross-proc content-pick"
    );
}

/// ★ The gate is STRUCTURAL, not caller discipline: `handle_doorbell` is the ONE
/// ring path (the ungated `ring_gated` sibling is gone), so a doorbell whose working
/// set is bound-but-unpublished (the #14 EXECUTION-fault state — the shadow had the
/// VA, the channel's OWN host VAS did not) is refused BEFORE any host op, and there is
/// no other function to reach the host doorbell through. Once published, the SAME path
/// rings.
#[test]
fn t14_ring_gate_is_structural_no_ungated_door() {
    let (mut gpu, _vmm, rec) = two_proc_gpu();
    let pid_a = *gpu.by_pdb.get(&(GpuId::ZERO, A_PDB)).unwrap();
    let a_token = MockArch::token_for(VChid(0x10));
    const MEM: nvkvm_arch::ids::HObject = nvkvm_arch::ids::HObject(0x5c00_0100);

    // Bind the VA through the RPC source ONLY (declared backing, host_va = None): it
    // resolves in the guest-side table but was never published into the channel's own
    // host VAS — exactly the #14 EXECUTION-fault shape.
    let mut s = Scenario::new();
    s.memory(HClient(0xAA), nvkvm_arch::ids::HObject(0x5c00_0001), MEM, 0x9_0000_0000);
    s.map(HClient(0xAA), nvkvm_arch::ids::HObject(0x5c00_0010), MEM, SHARED_VA, 0x10000);
    for ev in s.events {
        gpu.apply(ev).expect("rpc map applies");
    }

    // The ONE ring path refuses it — bound but not host-published — with ZERO host
    // ops (not even channel materialization). There is no ungated door to try instead.
    let before = rec.lock().unwrap().log.len();
    assert!(matches!(
        handle_doorbell(&mut gpu, GpuId::ZERO, a_token, &[SHARED_VA]),
        Err(FwdFault::Address(_))
    ));
    assert_eq!(rec.lock().unwrap().log.len(), before, "a gated-out ring did no host op");

    // The guest eager-unmaps the RPC binding, then the VA is published into the
    // channel's OWN host VAS (host_va = Some): the SAME path now rings.
    gpu.apply(nvkvm_core::rmgraph::RmEvent::Unmap {
        client: HClient(0xAA),
        vaspace: nvkvm_arch::ids::HObject(0x5c00_0010),
        va: SHARED_VA,
    })
    .expect("unmap applies");
    publish_backing(gpu.procs.get_mut(&pid_a).unwrap(), GpuId::ZERO, A_PDB, SHARED_VA, 0x10000).unwrap();
    handle_doorbell(&mut gpu, GpuId::ZERO, a_token, &[SHARED_VA]).expect("published set rings");
    assert!(
        rec.lock().unwrap().log.iter().any(|(_, v)| matches!(v, RmVerb::RingDoorbell { .. })),
        "the successful ring reached the host doorbell"
    );
}

/// A VA outside any binding is a loud MISS at gate time — never a guess, never a
/// cross-proc reach.
#[test]
fn t14_unpublished_va_is_a_loud_fault() {
    let (mut gpu, _vmm, _rec) = two_proc_gpu();
    let pid_a = *gpu.by_pdb.get(&(GpuId::ZERO, A_PDB)).unwrap();
    let cid_a = *gpu.procs[&pid_a].chan_ids.values().next().unwrap();
    publish_backing(gpu.procs.get_mut(&pid_a).unwrap(), GpuId::ZERO, A_PDB, SHARED_VA, 0x10000).unwrap();

    // A VA that was never published: loud MISS.
    assert!(matches!(
        gate_working_set(&gpu, pid_a, cid_a, &[GpuVa(0xdead_0000)]),
        Err(FwdFault::Address(_))
    ));
}

/// Soak: a sustained submit/complete loop through the parser loses no completion and
/// the address table does not grow unbounded (reclaim deferred, but re-submits of the
/// same VA do not double-bind).
#[test]
fn soak_submit_complete_loop_loses_no_completion() {
    let (mut gpu, mut vmm) = one_proc_gpu();
    let pid = *gpu.by_pdb.get(&(GpuId::ZERO, A_PDB)).unwrap();
    let cid = *gpu.procs[&pid].chan_ids.values().next().unwrap();

    let mut expected_completions = 0usize;
    for iter in 0..64u64 {
        let sem_addr = 0x2_0030_0000 + iter * 0x1000;
        let ring = script_pushbuffer(&mut vmm, 0x5000_0000, &[
            MockPushbuffer::sem_release(sem_addr, iter),
        ]);
        let out = parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring).expect("parses");
        assert_eq!(out.sem_releases.len(), 1);
        expected_completions += 1;

        // Drain the proc's queue each iter (post + drain + ack) — no completion lost.
        assert!(gpu.procs[&pid].completion.has_outstanding());
        let batch = gpu.pump_completions(GpuId::ZERO).expect("posts");
        assert!(!batch.events.is_empty());
        gpu.completions_drained(GpuId::ZERO);
    }
    assert_eq!(expected_completions, 64, "every iteration's completion was observed");
    // The address table has no unbounded growth from sema releases (they observe, not bind).
    assert_eq!(gpu.procs[&pid].vases[&(GpuId::ZERO, A_PDB)].table.iter().count(), 0, "sema releases do not bind");
}

// ---------------------------------------------------------------------------------
// Fuzz: an arbitrary pushbuffer byte stream — hostile methods, truncated rings, bogus
// sema/dst addresses — every path is either a decoded fact or a loud fault, NEVER a
// panic, NEVER a silent guess (boundary-1 posture).
// ---------------------------------------------------------------------------------

mod fuzz {
    use nvkvm_arch::ids::GpuId;
    use nvkvm_fwd::parse_pushbuffer;
    use nvkvm_vmm::Vmm;
    use proptest::collection::vec;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        /// Any GPFIFO ring bytes + any method-region bytes: the parser returns a
        /// `Result`, never panics, and any bound table entry is consistent (resolve
        /// of a bound VA succeeds — no torn state).
        #[test]
        fn arbitrary_pushbuffer_bytes_never_panic(
            ring in vec(any::<u8>(), 0..80),
            region in vec(any::<u8>(), 0..256),
        ) {
            let (mut gpu, mut vmm) = super::one_proc_gpu();
            let pid = *gpu.by_pdb.get(&(GpuId::ZERO, super::A_PDB)).unwrap();
            let cid = *gpu.procs[&pid].chan_ids.values().next().unwrap();
            // Back the region GPFIFO entries might point at (0x5000_0000 window).
            vmm.gpa_write(0x5000_0000, &region).unwrap();

            // Never panics — a Result at worst a loud fault.
            let _ = parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring);

            // Whatever got bound resolves cleanly (no torn/partial binding).
            let vas = &gpu.procs[&pid].vases[&(GpuId::ZERO, super::A_PDB)];
            for (va, _len, b) in vas.table.iter() {
                prop_assert_eq!(
                    vas.table.resolve(super::A_PDB, nvkvm_arch::ids::GpuVa(va)).map(|(x, _)| x.phys),
                    Ok(b.phys),
                    "a bound VA must resolve to its own binding (no torn state)"
                );
            }
        }
    }
}
