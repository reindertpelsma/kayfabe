//! # ★★★★★ **The first guest byte** — `OS_DESCRIPTOR` over guest RAM, mapped FIXED at the
//! guest's own VA (`docs/design/guest_ram_crossing.md` §5.8, step 3).
//!
//! Step 1 found the hypervisor's guest-RAM block and its **extent**. Step 2 made the
//! hypervisor **state** where that block appears in the guest's physical space. Neither
//! mapped a byte: `GuestRamPlane::honour` had no production caller and no
//! [`kayfabe_isolate::GuestRamGrant`] was ever constructed. This file is the contract for
//! the chain that constructs one.
//!
//! # ★★★ What this suite can and cannot judge, stated first
//!
//! It is **mock-driven and GPU-free**, so it judges the **chain** — order, idempotence,
//! placement enforcement, refusal names, unwinding — and it judges **nothing at all** about
//! whether RM accepts an `OS_DESCRIPTOR` over guest pages. ⊘ That is one ioctl on one real
//! driver and it is measured on the bench, in a boot log, and nowhere else. A green run
//! here is exactly as strong as [`kayfabe_mocks::MockRmBackend::describe_guest_ram`]'s own
//! docs say it is.
//!
//! # ★★ The one property that would be worthless if asserted the obvious way
//!
//! *"the mapping was placed where it was asked"* is checkable **only** against a backend
//! that could have placed it somewhere else. A double that always echoes `at` proves
//! nothing — it is [an echo is unverifiable by its reply] in test form. So
//! [`a_relocated_fixed_map_is_refused_and_everything_it_built_is_unwound`] installs a
//! backend that **deliberately relocates**, and asserts that the chain refuses by name and
//! frees what it had already built. The passing case beside it is what makes the refusal a
//! statement about placement rather than about the chain failing in general.

use std::sync::Arc;

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_isolate::{
    GuestRamGrant, GuestRamMapped, HostHandle, IsolateId, RmError, VerbPlan, VerbReply, Worker,
    WorkerId,
};
use kayfabe_mmu::{AddressFault, Binding};
use kayfabe_mocks::watchdog;
use kayfabe_mocks::{MockArch, MockIsolateFactory, MockRmBackend, RmVerb, SharedRecorder};
use kayfabe_rt::device::{LockMode, SharedDevice};
use kayfabe_tests::{Guarded, Scenario, identical_handles};
use kayfabe_vmm::Prot;

const GPU: GpuId = GpuId::ZERO;
const CLIENT: HClient = HClient(0xA0);
const PDB: Pdb = Pdb(0x3400_0000);
const GR: VChid = VChid(0x100);
const CE: VChid = VChid(0x200);
const MEM: HObject = HObject(0x6000_0000);

/// The ring's guest VA — the shape `w208`/`w209` measured (`0x420064000`), because a test
/// whose addresses look nothing like the ones the boot uses cannot be read beside the boot.
const RING_VA: GpuVa = GpuVa(0x4_2006_4000);
/// The ring's guest-PHYSICAL page. ⊘ Deliberately not `0x237fe000`: that number is one
/// boot's answer for one channel, and `[measured 2026-08-10, boot `w209_ffc80f8_real`]`
/// three channels of a single boot resolved to three different pages. Nothing may key on it.
const RING_GPA: u64 = 0x0768_a000;
/// The offset into the guest-RAM descriptor those bytes live at, **as a hypervisor would
/// state it**. ⊘ Deliberately NOT equal to [`RING_GPA`]: identity holds on the bench's
/// `-m 2048` and breaks at `-m 8G`, and a fixture that made them equal could not tell a
/// correct chain from one that re-derived the offset from the address.
const RING_FILE_OFFSET: u64 = 0x1_0000_0000 + 0x0768_a000;
const PIN_LEN: u64 = 4096;
const GUEST_RAM_BYTES: u64 = 0x2_0000_0000;

fn grant() -> GuestRamGrant {
    GuestRamGrant::originated_by_the_vmm(RING_FILE_OFFSET, PIN_LEN, Prot::ReadWrite)
}

/// One guest proc on GPU0 whose isolates can see `guest_ram` bytes of guest memory.
fn device(
    guest_ram: Option<u64>,
) -> (
    Guarded<Arc<SharedDevice>>,
    kayfabe_core::ProcId,
    SharedRecorder,
) {
    let (factory, recorder) = MockIsolateFactory::with_pool_size(2);
    let factory = match guest_ram {
        Some(b) => factory.with_guest_ram(b),
        None => factory,
    };
    let gpa = GpaSpace::new(0x10_0000_0000..0x1000_0000_0000, 0x10_0000_0000);
    let mut gpu = Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("realizes");
    let mut s = Scenario::new();
    s.compute_process_on_gpu(CLIENT, PDB, identical_handles(GR.0, CE.0), None);
    s.memory(CLIENT, HObject(0x5c00_0001), MEM, 0x9_0000_0000);
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies");
    }
    kayfabe_tests::guest_schedules_every_channel(&mut gpu);
    let pid = gpu.spine.by_pdb[&(GPU, PDB)];
    (
        Guarded::new(
            "guest_ram_pin::device",
            Arc::new(SharedDevice::new(gpu, LockMode::Sharded)),
            recorder.clone(),
        ),
        pid,
        recorder,
    )
}

/// Declare, in this proc's address table, that the guest itself has `RING_VA` bound to
/// `RING_GPA` in `aperture` — i.e. that the guest's own page tables say so.
///
/// ⊘ This is the fixture standing in for the **populate pass**, not for the resolver. The
/// production caller reads exactly this table, and the reason it can is that
/// `SharedDoorbell::pin_ring_guest_ram` runs after `PT-DECODE` has committed the binding.
fn guest_binds(device: &SharedDevice, pid: kayfabe_core::ProcId, aperture: Aperture) {
    device
        .with_proc_mut(pid, |p| {
            let vas = p.vases.get_mut(&(GPU, PDB)).expect("the compute VAS");
            vas.table
                .bind(
                    PDB,
                    RING_VA,
                    PIN_LEN,
                    // ⊘ The `host: Option<HostBacking>` parameter this helper used to take
                    // was `None` at all four call sites; `Binding::declared_by_guest` is
                    // that fact as a type. A host-backed fixture would be a different
                    // question (kind 3), and the pin's subject is what the GUEST declared.
                    Binding::declared_by_guest(RING_GPA, aperture)
                        .expect("the fixture declares a kind the guest can declare"),
                )
                .expect("the fixture's own bind is well-formed");
        })
        .expect("the proc is live");
}

fn verbs(rec: &SharedRecorder) -> Vec<&'static str> {
    rec.lock()
        .expect("recorder")
        .log
        .iter()
        .map(|(_, v)| match v {
            RmVerb::AllocVaSpace { .. } => "vas",
            RmVerb::AllocSysmem { .. } => "sysmem",
            RmVerb::MapGuestRam { .. } => "map_guest_ram",
            RmVerb::DescribeGuestRam { .. } => "describe",
            RmVerb::MapGpuVa { .. } => "map_gpu_va",
            RmVerb::UnmapGpuVa { .. } => "unmap_gpu_va",
            RmVerb::Free { .. } => "free",
            _ => "other",
        })
        .collect()
}

// ---------------------------------------------------------------------------------
// 1 — ★★★★★ THE CHAIN
// ---------------------------------------------------------------------------------

/// ★★★★★ **The whole rung, in one assertion set**: a pin issues
/// `map_guest_ram → describe_guest_ram → map_gpu_va`, in that order, at the **guest's own
/// VA**, and it allocates **no host sysmem at all**.
///
/// ★ The last clause is the one that distinguishes this chain from
/// [`kayfabe_fwd::publish_backing`] and it is asserted rather than described: `publish`
/// mints host memory and maps it at the guest's address, which is right for a range the
/// guest has never written and **wrong for a ring the guest polls**. If a future edit
/// folded the two chains together, `sysmem` would appear in this list.
#[test]
fn a_pin_maps_describes_and_places_the_guests_own_pages_at_the_guests_own_va() {
    let _wd = watchdog(
        "guest_ram_pin::the_chain",
        std::time::Duration::from_secs(60),
    );
    let (device, pid, rec) = device(Some(GUEST_RAM_BYTES));
    guest_binds(&device, pid, Aperture::SysmemCoherent);

    let p = device
        .pin_guest_ram(GPU, PDB, RING_VA, grant())
        .expect("the pin runs");

    assert_eq!(
        p.host_va, RING_VA.0,
        "★ placed_as_asked: address identity is the whole point — the guest's pushbuffer \
         names this number and the host MMU walks for it"
    );
    assert!(!p.already, "the first pin did the work");
    assert_eq!(
        verbs(&rec),
        vec!["vas", "map_guest_ram", "describe", "map_gpu_va"],
        "★ the chain threaded its own intermediates, and ⊘ NO `sysmem` appears — the bytes \
         under this mapping are the GUEST's"
    );

    // ★★ The grant that crossed is the one the VMM minted, byte for byte. A chain that
    // recomputed the offset from the guest-physical address would pass every assertion
    // above and fail this one — which is why `RING_FILE_OFFSET != RING_GPA`.
    let log = rec.lock().expect("recorder");
    let (offset, len, prot) = log
        .log
        .iter()
        .find_map(|(_, v)| match v {
            RmVerb::MapGuestRam {
                offset, len, prot, ..
            } => Some((*offset, *len, *prot)),
            _ => None,
        })
        .expect("the mapping verb was recorded");
    assert_eq!(
        (offset, len),
        (RING_FILE_OFFSET, PIN_LEN),
        "⊘ the isolate was instructed with the VMM's FILE OFFSET, never with a \
         guest-physical address it could have re-derived one from"
    );
    assert_eq!(
        prot,
        Prot::ReadWrite,
        "the ring's semaphore is written by the engine; a read-only grant fails at the \
         ioctl for a reason the status will not name"
    );
}

/// ★★★ **A second pin at the same VA issues NO verbs at all.**
///
/// ⊘ Not an optimisation. The production caller sits on a **doorbell**, which repeats; a
/// second `OS_DESCRIPTOR` plus a second *fixed* `map_dma` at an occupied address is
/// answered by RM with `0x51 NV_ERR_NO_MEMORY`, and that status cannot be told apart from
/// genuine exhaustion. ⇒ The idempotence is what keeps a legible failure legible.
#[test]
fn a_second_pin_at_the_same_va_is_an_idempotent_replay_and_issues_no_verbs() {
    let _wd = watchdog(
        "guest_ram_pin::idempotent",
        std::time::Duration::from_secs(60),
    );
    let (device, pid, rec) = device(Some(GUEST_RAM_BYTES));
    guest_binds(&device, pid, Aperture::SysmemCoherent);

    let first = device
        .pin_guest_ram(GPU, PDB, RING_VA, grant())
        .expect("first");
    let before = verbs(&rec).len();
    let second = device
        .pin_guest_ram(GPU, PDB, RING_VA, grant())
        .expect("second");

    assert!(!first.already);
    assert!(second.already, "the replay says so, out loud");
    assert_eq!(
        (second.host_va, second.memory),
        (first.host_va, first.memory),
        "a replay reports the LIVE pin's objects, never fresh ones"
    );
    assert_eq!(
        verbs(&rec).len(),
        before,
        "⊘ not one host verb ran on the replay — a shorter chain would still have built a \
         second OS_DESCRIPTOR over pages that are already pinned"
    );
}

// ---------------------------------------------------------------------------------
// 2 — ★★★★ THE REFUSALS, each by NAME
// ---------------------------------------------------------------------------------

/// ★★★ A VA whose binding is **not sysmem** is refused by name.
///
/// The guest's own page tables say these bytes live in the framebuffer. Pinning the
/// hypervisor's RAM at that address would publish guest memory under an address the guest
/// uses for something else — and it would look exactly like a working pin.
#[test]
fn a_vidmem_binding_is_refused_by_name_and_nothing_is_built() {
    let _wd = watchdog("guest_ram_pin::vidmem", std::time::Duration::from_secs(60));
    let (device, pid, rec) = device(Some(GUEST_RAM_BYTES));
    guest_binds(&device, pid, Aperture::Vidmem);

    let e = device
        .pin_guest_ram(GPU, PDB, RING_VA, grant())
        .expect_err("refused");
    assert!(
        matches!(
            e,
            kayfabe_fwd::FwdFault::GuestRamNotSysmem {
                va: RING_VA,
                aperture: Aperture::Vidmem
            }
        ),
        "the refusal NAMES the aperture it found, not merely that it refused: {e:?}"
    );
    assert!(
        verbs(&rec).is_empty(),
        "refused in the PLAN phase, so there is nothing to orphan"
    );
}

/// ★★★ A VA the core has **already host-published** is refused by name.
///
/// `publish_backing` put host sysmem at this address. Demanding the same host GPU VA again
/// is the `0x51` collision, and it is refused here — where the cause is still legible —
/// rather than at the ioctl, where it is not.
#[test]
fn an_already_host_published_va_is_refused_by_name() {
    let _wd = watchdog("guest_ram_pin::taken", std::time::Duration::from_secs(60));
    let (device, _pid, rec) = device(Some(GUEST_RAM_BYTES));
    // ★ The squatter is a REAL publication, made through the real verb, not a fabricated
    // `HostBacking`. A hand-built one would name a handle no ledger ever minted — which is
    // a shape the teardown post-condition correctly reports as a dangling object, and
    // would have made this test fail for a reason that has nothing to do with its subject.
    device
        .publish_backing(GPU, PDB, RING_VA, PIN_LEN)
        .expect("host sysmem is published at the ring's VA");
    let before = verbs(&rec).len();

    let e = device
        .pin_guest_ram(GPU, PDB, RING_VA, grant())
        .expect_err("refused");
    assert!(
        matches!(
            e,
            kayfabe_fwd::FwdFault::GuestRamAddressTaken { va: RING_VA, .. }
        ),
        "{e:?}"
    );
    assert_eq!(
        verbs(&rec).len(),
        before,
        "refused in the PLAN phase — not one host verb ran after the publication"
    );
}

/// ★★ An **unbound** VA is a MISS, and a miss is a FAULT — the address plane's law,
/// restated where a pin asks the question.
///
/// ⊘ The point is that there is no "pin it speculatively and find out": the pin's
/// guest-physical address comes from this very table, so an unbound VA means the caller
/// had no address to have derived.
#[test]
fn an_unbound_va_is_a_miss_and_a_miss_is_a_fault() {
    let _wd = watchdog("guest_ram_pin::miss", std::time::Duration::from_secs(60));
    let (device, pid, _rec) = device(Some(GUEST_RAM_BYTES));
    let _ = pid;
    let e = device
        .pin_guest_ram(GPU, PDB, RING_VA, grant())
        .expect_err("refused");
    assert!(
        matches!(e, kayfabe_fwd::FwdFault::Address(AddressFault::Miss { .. })),
        "{e:?}"
    );
}

/// ★★★ **The deployment refusal, and it is the DEFAULT.** An isolate that was never
/// granted guest RAM refuses the mapping by name, and the chain unwinds the host VAS it
/// had already allocated.
///
/// ⚠ This is the majority deployment — a VM launched without a shared memory backing — so
/// it must be the shape that is asserted, not the exception.
#[test]
fn an_isolate_with_no_guest_ram_refuses_by_name_and_unwinds_what_it_built() {
    let _wd = watchdog(
        "guest_ram_pin::unavailable",
        std::time::Duration::from_secs(60),
    );
    let (device, pid, rec) = device(None);
    guest_binds(&device, pid, Aperture::SysmemCoherent);

    let e = device
        .pin_guest_ram(GPU, PDB, RING_VA, grant())
        .expect_err("refused");
    assert!(
        matches!(
            e,
            kayfabe_fwd::FwdFault::Rm {
                err: RmError::GuestRamUnavailable,
                ..
            }
        ),
        "a deployment fact, refused by its own name rather than as a host resource \
         condition: {e:?}"
    );
    let seen = verbs(&rec);
    assert!(
        seen.contains(&"vas") && seen.contains(&"free"),
        "the host VAS the chain allocated before the refusal was FREED, not orphaned: \
         {seen:?}"
    );
}

// ---------------------------------------------------------------------------------
// 3 — ★★★★★ PLACEMENT, asserted against a backend that COULD have relocated it
// ---------------------------------------------------------------------------------

/// A backend that behaves exactly like the double except that `map_gpu_va` puts the
/// mapping **one page away** from where it was asked to.
///
/// ⊘ This is not a hostile-guest model; it is the *silent* failure `#102` names. RM is
/// free to place a mapping wherever it likes unless `DMA_OFFSET_FIXED_TRUE` makes
/// `dmaOffset` an [IN] parameter, and a downgraded placement produces a channel that is
/// created, schedulable, rings a doorbell — and whose pushbuffer walks the host MMU into
/// nothing (`Xid 31 FAULT_PDE`).
#[derive(Debug)]
struct Relocating(MockRmBackend);

impl kayfabe_isolate::RmBackend for Relocating {
    fn alloc(
        &mut self,
        parent: HostHandle,
        class: kayfabe_arch::ids::ClassId,
        params: &[u8],
    ) -> Result<HostHandle, RmError> {
        self.0.alloc(parent, class, params)
    }
    fn alloc_vaspace(&mut self) -> Result<HostHandle, RmError> {
        self.0.alloc_vaspace()
    }
    fn alloc_sysmem(&mut self, len: u64) -> Result<HostHandle, RmError> {
        self.0.alloc_sysmem(len)
    }
    fn alloc_vidmem(&mut self, len: u64) -> Result<HostHandle, RmError> {
        self.0.alloc_vidmem(len)
    }
    fn alloc_channel(
        &mut self,
        vas: HostHandle,
        engine: kayfabe_arch::ids::EngineKind,
        hosting: Option<kayfabe_isolate::HostedObject<'_>>,
        adopt: Option<kayfabe_isolate::AdoptedGuestRing>,
    ) -> Result<kayfabe_isolate::ChannelHandles, RmError> {
        self.0.alloc_channel(vas, engine, hosting, adopt)
    }
    fn alloc_engine_object(
        &mut self,
        chan: HostHandle,
        class: kayfabe_arch::ids::ClassId,
        params: &[u8],
    ) -> Result<HostHandle, RmError> {
        self.0.alloc_engine_object(chan, class, params)
    }
    fn schedule(&mut self, chan: HostHandle) -> Result<(), RmError> {
        self.0.schedule(chan)
    }
    fn free(&mut self, obj: HostHandle) -> Result<(), RmError> {
        self.0.free(obj)
    }
    fn control(
        &mut self,
        obj: HostHandle,
        cmd: kayfabe_arch::ids::ControlCmd,
        payload: &mut [u8],
    ) -> Result<(), RmError> {
        self.0.control(obj, cmd, payload)
    }
    fn map_gpu_va(
        &mut self,
        vas: HostHandle,
        memory: HostHandle,
        len: u64,
        at: GpuVa,
    ) -> Result<u64, RmError> {
        // ★ The whole point of this double: it SUCCEEDS, and it lies about where.
        let honest = self.0.map_gpu_va(vas, memory, len, at)?;
        Ok(honest + 0x1000)
    }
    fn unmap_gpu_va(&mut self, vas: HostHandle, gpu_va: u64) -> Result<(), RmError> {
        self.0.unmap_gpu_va(vas, gpu_va)
    }
    fn ring_doorbell(&mut self, host_token: u64) -> Result<(), RmError> {
        self.0.ring_doorbell(host_token)
    }
    fn ce_copy(&mut self, vas: HostHandle, sub: kayfabe_isolate::CeSubCopy) -> Result<(), RmError> {
        self.0.ce_copy(vas, sub)
    }
    fn fb_read(&mut self, phys: u64, buf: &mut [u8]) -> Result<bool, RmError> {
        self.0.fb_read(phys, buf)
    }
    fn export_surface(
        &mut self,
        memory: HostHandle,
    ) -> Result<kayfabe_vmm::SurfaceHandle, RmError> {
        self.0.export_surface(memory)
    }
    fn export_backing(
        &mut self,
        want: kayfabe_isolate::ExportRequest,
    ) -> Result<kayfabe_isolate::ExportedBacking, RmError> {
        self.0.export_backing(want)
    }
    /// ★★★ Relocating too, and deliberately: `join_fb_leaf` is the second chain whose whole
    /// claim is address identity, so a double that relocated `map_gpu_va` and NOT this one
    /// would leave the join's placement check unexercised by the very fixture written to
    /// exercise placement.
    fn join_fb_leaf(
        &mut self,
        vas: HostHandle,
        len: u64,
        at: kayfabe_arch::ids::GpuVa,
        phys: u64,
    ) -> Result<kayfabe_isolate::FbLeafJoined, RmError> {
        let mut joined = self.0.join_fb_leaf(vas, len, at, phys)?;
        joined.host_va = at.0 + 0x1000;
        Ok(joined)
    }
    fn fb_join_peek(
        &mut self,
        phys: u64,
        buf: &mut [u8],
        poke: Option<u32>,
    ) -> Result<bool, RmError> {
        self.0.fb_join_peek(phys, buf, poke)
    }
    fn map_guest_ram(&mut self, g: GuestRamGrant) -> Result<GuestRamMapped, RmError> {
        self.0.map_guest_ram(g)
    }
    fn unmap_guest_ram(&mut self, m: GuestRamMapped) -> Result<(), RmError> {
        self.0.unmap_guest_ram(m)
    }
    fn describe_guest_ram(&mut self, m: GuestRamMapped) -> Result<HostHandle, RmError> {
        self.0.describe_guest_ram(m)
    }
}

/// ★★★★★ **A fixed map that lands elsewhere is refused, and everything it built is
/// unwound** — asserted at the worker seam, against a backend that really does relocate.
///
/// ⚠ Driven through [`Worker::execute`] rather than through the device, because the
/// property is the **verb chain's**, and reaching it through the device would require
/// installing this backend in a factory — which buys nothing and hides which layer holds
/// the check. The check lives in `Worker::execute`, so that is where it is asserted.
#[test]
fn a_relocated_fixed_map_is_refused_and_everything_it_built_is_unwound() {
    let _wd = watchdog(
        "guest_ram_pin::relocated",
        std::time::Duration::from_secs(60),
    );
    let iso = IsolateId::new(7, GPU);
    let rec: SharedRecorder = Arc::default();
    let mock = MockRmBackend::standalone(iso, WorkerId(0), Arc::clone(&rec))
        .with_guest_ram(GUEST_RAM_BYTES);
    let mut w = Worker::new(iso, WorkerId(0), Box::new(Relocating(mock)));

    let failure = w
        .execute(&VerbPlan::PinGuestRam {
            host_vas: None,
            grant: grant(),
            at: RING_VA,
        })
        .expect_err("a relocated placement must never be adopted");

    match failure.err {
        RmError::PlacementRefused { want, got } => {
            assert_eq!(want, RING_VA.0);
            assert_eq!(got, RING_VA.0 + 0x1000, "the double relocated by one page");
        }
        other => panic!("wrong refusal: {other:?} — the placement check did not run"),
    }
    assert!(
        failure.orphans.is_empty(),
        "★ the chain's own unwind disposed of the mapping, the OS_DESCRIPTOR and the host \
         VAS it had allocated; a non-empty residue here is the G4 leak this type exists to \
         make visible: {:?}",
        failure.orphans
    );
    let seen = verbs(&rec);
    assert!(
        seen.contains(&"unmap_gpu_va") && seen.contains(&"free"),
        "the relocated mapping was undone and its object freed: {seen:?}"
    );
}

/// ★★ **The control for the test above** — the same chain on an honest backend places the
/// mapping and reports it.
///
/// ⊘ Without this, the refusal above would pass equally well on a chain that could never
/// succeed at all, and it would be a statement about the chain rather than about placement.
#[test]
fn and_the_same_chain_on_an_honest_backend_places_it_exactly() {
    let _wd = watchdog("guest_ram_pin::honest", std::time::Duration::from_secs(60));
    let iso = IsolateId::new(7, GPU);
    let rec: SharedRecorder = Arc::default();
    let mock = MockRmBackend::standalone(iso, WorkerId(0), Arc::clone(&rec))
        .with_guest_ram(GUEST_RAM_BYTES);
    let mut w = Worker::new(iso, WorkerId(0), Box::new(mock));

    let reply = w
        .execute(&VerbPlan::PinGuestRam {
            host_vas: None,
            grant: grant(),
            at: RING_VA,
        })
        .expect("the chain runs");
    match reply {
        VerbReply::GuestRamPinned {
            host_vas,
            mapped,
            memory,
            host_va,
        } => {
            assert_eq!(host_va, RING_VA.0);
            assert_eq!(mapped.len, PIN_LEN);
            assert!(
                host_vas.is_some(),
                "the chain allocated the host VAS itself"
            );
            assert!(
                mapped.region != memory,
                "★ the MAPPING and the RM OBJECT are two different names for two different \
                 things — freeing one does not release the other, which is exactly why the \
                 reply carries both"
            );
        }
        other => panic!("wrong reply: {other:?}"),
    }
}
