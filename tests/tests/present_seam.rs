//! Batch-4 the abstract present/display seam (`execution_plane.md` §2.6/§3.3):
//! GR-graphics's home.
//!
//! - **consumer** — `Present::present` takes a [`SurfaceHandle`] (host VRAM —
//!   guest-RAM `RamHandle`s no longer typecheck into present), and the
//!   present-complete is fed back as a synthetic vblank on the OWNING proc's
//!   completion queue — display stays hypervisor/host-agnostic, NEVER NVKMS.
//!
//! ⊘⊘ **THE PRODUCER HALF IS GONE** (`ORPHANS_wire_or_discard.md`, 2026-09-12).
//! `RmBackend::export_surface` — the seam's other half, added by seam audit GR-2b so the
//! trait would not have to grow a method later — was deleted: no `Worker` wrapper ever
//! existed, so nothing outside a test could reach it, and the host impl was a stub
//! returning not-implemented. The two tests that drove it (through the `Worker::with_rm`
//! escape hatch) went with it. ⚠ So this file now tests ONE half of a seam, and a
//! `SurfaceHandle` is minted by nothing: the tests construct one directly.
//!
//! Invariant/contract tests (decision #15), mock-driven, GPU-free.

#![allow(clippy::unusual_byte_groupings)]

use kayfabe_arch::ids::GpuId;
use kayfabe_arch::ids::{HClient, Pdb};
use kayfabe_completion::OsEventRef;
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_fwd::{FwdFault, present_scanout};
use kayfabe_mocks::{MockArch, MockIsolateFactory, MockPresent, SharedRecorder};
use kayfabe_tests::{Guarded, Scenario, identical_handles};
use kayfabe_vmm::{FbMeta, PresentError, SurfaceHandle};

const PDB: Pdb = Pdb(0x3401_000);

fn graphics_gpu() -> (Guarded<Gpu>, SharedRecorder) {
    let arch = Box::new(MockArch::new());
    let (factory, recorder) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process(HClient(0xAA), PDB, identical_handles(0x10, 0x11));
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
    (
        Guarded::new("present_seam::graphics_gpu", gpu, recorder.clone()),
        recorder,
    )
}

fn fb() -> (SurfaceHandle, FbMeta) {
    (
        SurfaceHandle(0x1234),
        FbMeta {
            width: 1920,
            height: 1080,
            stride: 1920 * 4,
            format: 0,
        },
    )
}

/// A GR-graphics scanout routes to the `Present` sink and the present-complete becomes
/// a synthetic vblank on the OWNING proc's completion queue.
#[test]
fn scanout_routes_to_present_and_feeds_vblank() {
    let (mut gpu, _rec) = graphics_gpu();
    let pid = *gpu.spine.by_pdb.get(&(GpuId::ZERO, PDB)).unwrap();
    let mut present = MockPresent::new();
    let (buffer, meta) = fb();

    let seq = present_scanout(&mut gpu, pid, &mut present, buffer, meta).expect("scanout presents");

    // The surface reached the sink with its geometry (host-agnostic — the mock is a
    // stand-in for QEMU/PRIME).
    assert_eq!(
        present.presented,
        vec![(buffer, meta)],
        "scanout surface routed to Present"
    );
    // The present-complete is a synthetic vblank on the OWNING proc's queue.
    assert!(
        gpu.procs[&pid].completion.has_outstanding(),
        "vblank observed as completion"
    );
    // A second present advances the vblank sequence (monotonic frames).
    let (b2, m2) = fb();
    let seq2 = present_scanout(&mut gpu, pid, &mut present, b2, m2).unwrap();
    assert_eq!(seq2, seq + 1, "vblank sequence is monotonic");
}

/// The synthetic vblank flows through the existing completion plane (post + drain):
/// GR-graphics reuses the SAME completion machinery — no new plane.
#[test]
fn vblank_flows_through_the_completion_plane() {
    let (mut gpu, _rec) = graphics_gpu();
    let pid = *gpu.spine.by_pdb.get(&(GpuId::ZERO, PDB)).unwrap();
    let mut present = MockPresent::new();
    let (buffer, meta) = fb();

    let seq = present_scanout(&mut gpu, pid, &mut present, buffer, meta).unwrap();
    let batch = gpu.pump_completions(GpuId::ZERO).expect("vblank posts");
    assert_eq!(
        batch.events,
        vec![OsEventRef(seq)],
        "the vblank rides the normal completion batch"
    );
    gpu.completions_drained(GpuId::ZERO);
}

/// A present failure is a loud fault, never a silent drop.
#[test]
fn present_failure_is_a_loud_fault() {
    let (mut gpu, _rec) = graphics_gpu();
    let pid = *gpu.spine.by_pdb.get(&(GpuId::ZERO, PDB)).unwrap();
    let mut present = MockPresent::new();
    present.fail_next = Some(PresentError::Unsupported("no display"));
    let (buffer, meta) = fb();

    assert!(matches!(
        present_scanout(&mut gpu, pid, &mut present, buffer, meta),
        Err(FwdFault::Present(PresentError::Unsupported(_)))
    ));
    // Nothing was observed on the completion queue for the failed present.
    assert!(
        !gpu.procs[&pid].completion.has_outstanding(),
        "no vblank on a failed present"
    );
}
