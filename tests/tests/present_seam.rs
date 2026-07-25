//! Batch-4 the abstract present/display seam (`execution_plane.md` §2.6/§3.3):
//! GR-graphics's home. Both halves of the seam (seam audit GR-2):
//!
//! - **producer** — `RmBackend::export_surface`: the OWNING proc's isolate exports a
//!   host render-target memory object as a [`SurfaceHandle`] (the C-proven
//!   `PRIME_HANDLE_TO_FD` dma-buf export, `present_path_b_done`);
//! - **consumer** — `Present::present` takes that [`SurfaceHandle`] (host VRAM —
//!   guest-RAM `RamHandle`s no longer typecheck into present), and the
//!   present-complete is fed back as a synthetic vblank on the OWNING proc's
//!   completion queue — display stays hypervisor/host-agnostic, NEVER NVKMS.
//!
//! Invariant/contract tests (decision #15), mock-driven, GPU-free.

#![allow(clippy::unusual_byte_groupings)]

use nvkvm_arch::ids::GpuId;
use nvkvm_arch::ids::{HClient, Pdb};
use nvkvm_completion::OsEventRef;
use nvkvm_core::gpa::GpaSpace;
use nvkvm_core::gpu::Gpu;
use nvkvm_fwd::{FwdFault, present_scanout};
use nvkvm_isolate::{HostHandle, RmError};
use nvkvm_mocks::{
    MockArch, MockIsolateFactory, MockPresent, RmVerb, SharedRecorder, mock_classes as mc,
};
use nvkvm_tests::{Scenario, identical_handles};
use nvkvm_vmm::{FbMeta, PresentError, SurfaceHandle};

const PDB: Pdb = Pdb(0x3401_000);

fn graphics_gpu() -> (Gpu, SharedRecorder) {
    let arch = Box::new(MockArch::new());
    let (factory, recorder) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process(HClient(0xAA), PDB, identical_handles(0x10, 0x11));
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
    (gpu, recorder)
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

/// ★ GR-2, the full seam chain — producer to vblank: a render-target memory object on
/// the OWNING proc's isolate is exported to a [`SurfaceHandle`]
/// (`RmBackend::export_surface`, the isolate-side PRIME export), that surface is
/// presented, and the present-complete lands as a synthetic vblank on the OWNER's
/// completion queue. The seam has both halves and they plug together — with no
/// graphics pipeline built.
#[test]
fn render_target_exports_to_surface_presents_and_vblanks() {
    let (mut gpu, recorder) = graphics_gpu();
    let pid = *gpu.spine.by_pdb.get(&(GpuId::ZERO, PDB)).unwrap();

    // Producer: a host render-target memory object, exported by the OWNING proc's
    // OWN isolate to a presentable surface.
    let mut worker = gpu
        .procs
        .get_mut(&pid)
        .expect("proc")
        .isolates
        .get_mut(&GpuId::ZERO)
        .unwrap()
        .checkout()
        .expect("the isolate's pool has an idle worker");
    let (target, surface) = worker.with_rm(|rm| {
        let target = rm
            .alloc(HostHandle(0), mc::MEMORY, &[])
            .expect("render-target memory allocs");
        let surface = rm
            .export_surface(target)
            .expect("render target exports to a surface");
        (target, surface)
    });
    {
        let log = recorder.lock().unwrap();
        assert!(
            log.log.iter().any(|(_, v)| matches!(
                v,
                RmVerb::ExportSurface { memory, surface: s } if *memory == target && *s == surface
            )),
            "the export ran through the isolate's RM verb surface"
        );
    }

    // Consumer: present that surface; the vblank rides the OWNER's completion queue.
    let mut present = MockPresent::new();
    let meta = FbMeta {
        width: 640,
        height: 480,
        stride: 640 * 4,
        format: 0,
    };
    let seq = present_scanout(&mut gpu, pid, &mut present, surface, meta)
        .expect("exported surface presents");
    assert_eq!(
        present.presented,
        vec![(surface, meta)],
        "the EXPORTED surface was presented"
    );
    assert!(
        gpu.procs[&pid].completion.has_outstanding(),
        "present-complete = vblank observed"
    );
    let batch = gpu.pump_completions(GpuId::ZERO).expect("vblank posts");
    assert_eq!(
        batch.events,
        vec![OsEventRef(seq)],
        "the vblank rides the owner's batch"
    );
}

/// Exporting an unknown/foreign memory object is a LOUD `BadHandle` — a surface is
/// never silently minted for a render target this isolate does not own.
#[test]
fn exporting_an_unknown_render_target_is_a_loud_fault() {
    let (mut gpu, _rec) = graphics_gpu();
    let pid = *gpu.spine.by_pdb.get(&(GpuId::ZERO, PDB)).unwrap();
    let mut worker = gpu
        .procs
        .get_mut(&pid)
        .expect("proc")
        .isolates
        .get_mut(&GpuId::ZERO)
        .unwrap()
        .checkout()
        .expect("the isolate's pool has an idle worker");
    let bogus = HostHandle(0xdead_beef);
    assert_eq!(
        worker.with_rm(|rm| rm.export_surface(bogus)),
        Err(RmError::BadHandle(bogus))
    );
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
