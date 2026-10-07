//! ★★★ **Is a covered window address actually NATIVE?** — answered by counting exits, in a
//! real machine, with a real vCPU.
//!
//! # Why a value is not evidence, and what is
//!
//! The claim the GPGA viewer index's consumer makes is *"a guest access to an address the
//! index says is covered becomes a native memory access — no trap"*. The tempting way to
//! test that is to write a byte into the backing, have the guest read it, and check the
//! value. ⊘ **That test is worthless**, and it is worthless in the precise way this project
//! has been bitten by before: a guest read of an address with *no* memslot takes an MMIO
//! exit, we serve it, and the guest gets the right value anyway. The correct answer arrives
//! either way, so the observation does not discriminate.
//!
//! What discriminates is the **exit count**. A load served by the second-level page tables
//! produces **no exit at all**; a load that traps produces one. So the guest below runs the
//! same instructions over the same number of iterations against two addresses that differ
//! only in whether the installer's layout covers them, and the assertion is on the
//! exit counters in `VcpuReport`.
//!
//! # The guest, and why its shape makes the count unambiguous
//!
//! `kayfabe_tests::probe_loop_image` is three instructions: load from an immediate address,
//! store the loaded value through `ebx`, jump back. `ebx` points at a **trapped** register
//! aperture, so **every iteration produces exactly one write exit** — which is the
//! denominator. A read exit is therefore never ambiguous: it can only have come from the
//! load, and the load is the thing under test.
//!
//! | the load's address | memslot? | exits it causes |
//! |---|---|---|
//! | inside a covered run | yes | **none** |
//! | inside an observe run | no | **one per iteration**, classified `Unclaimed` |
//!
//! ★ The second row is the non-vacuity control, and it is not optional: *"no read exits"* is
//! exactly as true of a working memslot as of a vCPU that never ran. `mmio_writes > 0` is
//! asserted alongside it for the same reason.
//!
//! ⚠ **The counter to assert on is `unclaimed_exits`, not `mmio_reads`, and the first draft
//! of this file got that wrong.** `mmio_reads` counts loads *dispatched into a device*, so
//! only an exit inside a declared aperture reaches it. A load at a guest-physical address
//! nothing claims is `ExitClass::Unclaimed` and increments a different counter. Asserting
//! the wrong one made the control read zero — i.e. it said *"the uncovered page did not
//! trap"*, which was false. The control caught it; had this file only asserted the covered
//! case it would have been green and meaningless.
//!
//! # ⚠ Two machines, and it is deliberate
//!
//! The layout is computed by the real `ViewInstaller` against a real `ViewerIndex` — through
//! the QEMU adapter, which is where the installer lives. It is then installed into a real
//! `KvmMachine`, which is the adapter that has vCPUs. The thing under test is the
//! **layout's meaning**: that the runs the index caused to be covered are the runs a real
//! processor reaches without leaving the guest.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use kayfabe_arch::ids::GpuId;
use kayfabe_arch::{Aperture, FbWindow};
use kayfabe_isolate::IsolateId;
use kayfabe_linux_raw::HostPageSize;
use kayfabe_mmu::gpga::{GpgaRegion, ObjectChange, ObjectId, ViewerIndex, ViewerKind};
use kayfabe_vmm::{BarId, CoreEvent, Device, Vmm};
use kayfabe_vmm_qemu::slots::Tier;
use kayfabe_vmm_qemu::viewer_install::{
    BackingUnknown, HostBacking, ObjectBacking, ViewInstaller, tier_at,
};

/// Where the guest's code page lives.
const GPA_CODE: u64 = 0x1000_0000;
/// Where the viewer window lives — two pages, of which the index covers the first.
const GPA_WINDOW: u64 = 0x1200_0000;
/// The trapped register aperture the guest stores through, so every iteration exits once.
const GPA_BAR0: u64 = 0x7000_0000;
/// How long that aperture is.
const BAR0_LEN: u64 = 0x40_0000;
/// How many exits each run is allowed. A **count**, never a duration.
const EXIT_BUDGET: u64 = 64;

/// A device that answers a trapped store and counts nothing else — the smallest thing that
/// makes `ebx` a real exit rather than a fault.
#[derive(Debug, Default)]
struct Sink;

impl Device for Sink {
    fn mmio_read(&self, _vmm: &mut dyn Vmm, _bar: BarId, _off: u64, _size: u8) -> u64 {
        0
    }
    fn mmio_write(&self, _vmm: &mut dyn Vmm, _bar: BarId, _off: u64, _size: u8, _val: u64) {}
    fn event(&self, _vmm: &mut dyn Vmm, _ev: CoreEvent) {}
}

/// A backing source that places every object at its own GPGA offset.
struct Identity;

impl ObjectBacking for Identity {
    fn backing_for(
        &self,
        _object: ObjectId,
        _aperture: Aperture,
        gpga_base: u64,
        _len: u64,
    ) -> Result<HostBacking, BackingUnknown> {
        Ok(HostBacking::VmmOwned { offset: gpga_base })
    }
}

/// Build the index and the installer's layout: page 0 covered by an object, page 1 not.
///
/// Returns the layout, so the assertions below are about **what the installer decided**
/// rather than about a constant this file chose.
fn layout_from_the_index(page: u64) -> kayfabe_vmm_qemu::viewer_install::ViewLayout {
    let host = Arc::new(kayfabe_vmm_qemu::mock_host::MockQemuHost::with_policy(
        kayfabe_vmm_qemu::mock_host::MockPolicy::default(),
    ));
    host.place_bar(BarId::Bar0, GPA_BAR0);
    host.place_bar(BarId::Bar1, GPA_WINDOW);
    let slots = Arc::new(kayfabe_vmm_qemu::mock_host::MockSlotPlane::new(509, page));
    let m = kayfabe_vmm_qemu::QemuMachine::realize(
        kayfabe_vmm_qemu::MachineConfig {
            shareable_ram: true,
            bars: vec![
                kayfabe_vmm_qemu::host::BarPlacement {
                    bar: BarId::Bar0,
                    base: GPA_BAR0,
                    len: BAR0_LEN,
                },
                kayfabe_vmm_qemu::host::BarPlacement {
                    bar: BarId::Bar1,
                    base: GPA_WINDOW,
                    len: 64 * page,
                },
            ],
            windows: vec![],
            traps: vec![],
        },
        host,
        slots as _,
    )
    .expect("the mock host places both apertures");

    let mut ix = ViewerIndex::new();
    let viewer = ix.add_view(ViewerKind::Window(FbWindow::InstanceWindow));
    // ★ ONE object, one page long. The window is two pages, so the index's answer for the
    // second page is "nothing is here" — which is what must translate into "it traps".
    let change = ObjectChange::Allocated {
        region: GpgaRegion::new(Aperture::Vidmem, 0, page).expect("well-formed"),
        owner: IsolateId::new(1, GpuId(0)),
    };
    let plan = ix.plan(&change).expect("empty index");
    ix.apply(&plan).expect("fresh plan");
    ix.map_into_view(
        viewer,
        GpgaRegion::new(Aperture::Vidmem, 0, 2 * page).expect("well-formed"),
        0,
    )
    .expect("a fresh view over the whole window");

    let mut inst = ViewInstaller::new(
        FbWindow::InstanceWindow,
        viewer,
        GPA_WINDOW,
        2 * page,
        HostPageSize::query(),
    );
    inst.drain_and_install(&mut ix, &Identity, &m)
        .expect("ordinary host memory backs it");
    inst.layout().clone()
}

/// Run the guest for `EXIT_BUDGET` exits, loading from `probe`, and report what it did.
fn run_probing(probe: u64, backed: &[(u64, u64)]) -> kayfabe_vmm_kvm::vcpu::VcpuReport {
    let machine = kayfabe_vmm_kvm::KvmMachine::realize(kayfabe_vmm_kvm::MachineConfig {
        shareable_ram: true,
        bars: vec![kayfabe_vmm_kvm::BarPlacement {
            bar: BarId::Bar0,
            base: GPA_BAR0,
            len: BAR0_LEN,
        }],
    })
    .expect("/dev/kvm is present — this test is gated on exactly that");
    let page = machine.page_size().bytes();

    let _code = machine
        .install_ram_window(GPA_CODE, page)
        .expect("the code page");
    // ★ Only the runs the installer's layout marked native get a window. The others get
    // nothing, which is the whole experiment.
    let mut live = Vec::new();
    for &(gpa, len) in backed {
        live.push(
            machine
                .install_ram_window(gpa, len)
                .expect("a covered run is installable"),
        );
    }

    let mut vmm = machine.vmm();
    vmm.gpa_write(
        GPA_CODE,
        &kayfabe_tests::probe_loop_image(
            u32::try_from(probe).expect("the probe address is below 4 GiB"),
        ),
    )
    .expect("the image lands in the code page");

    let mut runner = machine
        .create_vcpu(0, Arc::new(Sink) as _)
        .expect("a real vCPU");
    // `ebx` is a trapped aperture offset, so every iteration stores through it and exits.
    runner
        .enter_at(GPA_CODE, GPA_BAR0)
        .expect("flat protected mode");
    let stop = AtomicBool::new(false);
    let _ = runner
        .run_until(&stop, EXIT_BUDGET)
        .expect("KVM_RUN succeeds");
    let r = runner.report();
    drop(runner);
    for w in live {
        machine.remove_window(w).expect("teardown");
    }
    stop.store(true, Ordering::Release);
    r
}

/// ★★★ **The claim, measured (2026-07-31): a covered address is reached with ZERO exits,
/// and an uncovered one is not.**
#[test]
fn an_address_the_index_covers_is_reached_without_leaving_the_guest() {
    kayfabe_linux_raw::require_kvm!(
        "an_address_the_index_covers_is_reached_without_leaving_the_guest"
    );
    let page = HostPageSize::query().bytes();
    let layout = layout_from_the_index(page);

    // ── What the installer decided, asserted before it is relied on. ──
    assert_eq!(
        layout.mappings(),
        1,
        "the index covers exactly one object, so the layout is one mapping: {layout:?}"
    );
    assert_eq!(
        tier_at(&layout, 0),
        Tier::Passthrough,
        "page 0 holds the object, so it must be native"
    );
    assert_eq!(
        tier_at(&layout, page),
        Tier::Observe,
        "page 1 holds nothing, so under miss = fault it must trap"
    );

    let backed: Vec<(u64, u64)> = layout
        .covered
        .iter()
        .map(|r| (GPA_WINDOW + r.view_off, r.len))
        .collect();

    // ── The subject: the guest loads from the COVERED page. ──
    let native = run_probing(GPA_WINDOW, &backed);
    assert!(
        native.mmio_writes > 0,
        "★ NON-VACUITY: the guest must actually have run. It stored through the trapped \
         aperture {} times; zero would mean the report below is about a vCPU that never \
         executed anything",
        native.mmio_writes
    );
    assert_eq!(
        native.unclaimed_exits, 0,
        "★★★ THE CLAIM. A load from an address the GPGA viewer index covers exited to us \
         {} times. It must exit ZERO times — the second-level page tables serve it, and \
         'passthrough' means exactly that and nothing softer. The guest performed {} \
         iterations, so this is not a run that was too short to observe anything",
        native.unclaimed_exits, native.mmio_writes
    );
    assert_eq!(native.mmio_reads, 0, "and no load reached a device either");
    // ★ Every exit the covered run took was the store through the trapped aperture. Nothing
    // else exited at all, which is the strongest form of the statement.
    assert_eq!(
        native.exits, native.mmio_writes,
        "the covered run's ONLY exits are the {} deliberate stores; {} exits happened",
        native.mmio_writes, native.exits
    );
    assert_eq!(
        native.ram_declared_exits, 0,
        "no exit may happen at an address the region map calls RAM"
    );

    // ── The control: the SAME guest, the SAME iteration count, one page further on. ──
    let trapped = run_probing(GPA_WINDOW + page, &backed);
    assert!(
        trapped.unclaimed_exits > 0,
        "★★★ THE CONTROL, and without it the assertion above is unfalsifiable. A load from \
         an address the index does NOT cover must trap; it produced {} read exits. If this \
         is zero, then the zero above was a property of the instrument rather than of the \
         mapping",
        trapped.unclaimed_exits
    );
    // ★★ And the shape of the difference is the one the mechanism predicts: the covered
    // guest exits ONCE per iteration (the store) and the uncovered one exits TWICE (the
    // load and the store), so at an equal exit budget the uncovered guest completes about
    // half as many iterations. A merely-different number could be noise; this is the
    // signature.
    assert!(
        trapped.mmio_writes * 2 <= native.mmio_writes + 1,
        "at an equal exit budget the trapping guest must complete about half the iterations \
         ({} vs {}), because each of its iterations costs two exits instead of one",
        trapped.mmio_writes,
        native.mmio_writes
    );
    // And the two runs really are the same experiment: same code, same budget, differing
    // only in which page the immediate names.
    assert!(trapped.mmio_writes > 0, "the control guest ran too");
    assert_ne!(
        native.unclaimed_exits, trapped.unclaimed_exits,
        "the covered and uncovered cases must be DISTINGUISHABLE; identical counts mean the \
         experiment did not vary what it thought it varied"
    );
    eprintln!(
        "GPGA-NATIVE: covered page -> {} total exits ({} stores, {} unclaimed reads); \
         uncovered page -> {} total exits ({} stores, {} unclaimed reads)",
        native.exits,
        native.mmio_writes,
        native.unclaimed_exits,
        trapped.exits,
        trapped.mmio_writes,
        trapped.unclaimed_exits
    );
}
