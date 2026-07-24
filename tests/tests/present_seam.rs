//! Batch-4 the abstract present/display seam (`execution_plane.md` §2.6/§3.3):
//! GR-graphics's home. A GR-graphics context routes its scanout buffer to an abstract
//! `Present` sink (MockPresent here; QEMU/PRIME later), and the present-complete is
//! fed back as a synthetic vblank on the OWNING proc's completion queue — display
//! stays hypervisor/host-agnostic, NEVER NVKMS.
//!
//! Invariant/contract tests (decision #15), mock-driven, GPU-free.

#![allow(clippy::unusual_byte_groupings)]

use nvkvm_arch::ids::{HClient, Pdb};
use nvkvm_completion::OsEventRef;
use nvkvm_core::gpu::Gpu;
use nvkvm_core::gpa::GpaSpace;
use nvkvm_fwd::{FwdFault, present_scanout};
use nvkvm_mocks::{MockArch, MockIsolateFactory, MockPresent};
use nvkvm_tests::{Scenario, identical_handles};
use nvkvm_vmm::{FbMeta, PresentError, RamHandle};

const PDB: Pdb = Pdb(0x3401_000);

fn graphics_gpu() -> Gpu {
    let arch = Box::new(MockArch::new());
    let (factory, _rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process(HClient(0xAA), PDB, identical_handles(0x10, 0x11));
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
    gpu
}

fn fb() -> (RamHandle, FbMeta) {
    (
        RamHandle { token: 0x1234, covers: None },
        FbMeta { width: 1920, height: 1080, stride: 1920 * 4, format: 0 },
    )
}

/// A GR-graphics scanout routes to the `Present` sink and the present-complete becomes
/// a synthetic vblank on the OWNING proc's completion queue.
#[test]
fn scanout_routes_to_present_and_feeds_vblank() {
    let mut gpu = graphics_gpu();
    let pid = *gpu.by_pdb.get(&PDB).unwrap();
    let mut present = MockPresent::new();
    let (buffer, meta) = fb();

    let seq = present_scanout(&mut gpu, pid, &mut present, buffer.clone(), meta)
        .expect("scanout presents");

    // The buffer reached the sink with its geometry (host-agnostic — the mock is a
    // stand-in for QEMU/PRIME).
    assert_eq!(present.presented, vec![(buffer, meta)], "scanout buffer routed to Present");
    // The present-complete is a synthetic vblank on the OWNING proc's queue.
    assert!(gpu.procs[&pid].completion.has_outstanding(), "vblank observed as completion");
    // A second present advances the vblank sequence (monotonic frames).
    let (b2, m2) = fb();
    let seq2 = present_scanout(&mut gpu, pid, &mut present, b2, m2).unwrap();
    assert_eq!(seq2, seq + 1, "vblank sequence is monotonic");
}

/// The synthetic vblank flows through the existing completion plane (post + drain):
/// GR-graphics reuses the SAME completion machinery — no new plane.
#[test]
fn vblank_flows_through_the_completion_plane() {
    let mut gpu = graphics_gpu();
    let pid = *gpu.by_pdb.get(&PDB).unwrap();
    let mut present = MockPresent::new();
    let (buffer, meta) = fb();

    let seq = present_scanout(&mut gpu, pid, &mut present, buffer, meta).unwrap();
    let batch = gpu.pump_completions().expect("vblank posts");
    assert_eq!(batch.events, vec![OsEventRef(seq)], "the vblank rides the normal completion batch");
    gpu.completions_drained();
}

/// A present failure is a loud fault, never a silent drop.
#[test]
fn present_failure_is_a_loud_fault() {
    let mut gpu = graphics_gpu();
    let pid = *gpu.by_pdb.get(&PDB).unwrap();
    let mut present = MockPresent::new();
    present.fail_next = Some(PresentError::Unsupported("no display"));
    let (buffer, meta) = fb();

    assert!(matches!(
        present_scanout(&mut gpu, pid, &mut present, buffer, meta),
        Err(FwdFault::Present(PresentError::Unsupported(_)))
    ));
    // Nothing was observed on the completion queue for the failed present.
    assert!(!gpu.procs[&pid].completion.has_outstanding(), "no vblank on a failed present");
}
