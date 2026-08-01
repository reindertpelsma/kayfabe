//! ★★★ **Does `unrealize` → `realize` RECOVER the emulated device, or does it leak?**
//!
//! The owner's requirement, 2026-07-31: *"if the guest bricks kayfabe emulator state at any
//! point, it should be possible to unload and reload device to restore, restarting emulated
//! device, like similar for real gpu"* — the semantics a real card gets from
//! `rmmod nvidia; modprobe nvidia`, or from a function-level reset.
//!
//! ## Why this file exists
//!
//! The C research artifact flatly cannot do this. Its own working notes say *"the emulated
//! GSP's WPR2 state only resets on a full QEMU restart, so each clean run needs a fresh
//! boot"*, which is why every experiment in that tree begins by rebooting a VM; and task
//! `#64` found its `publish` resetting the COMMAND ring cursor, which wedged GPU restart
//! permanently. Both are *reload* failures.
//!
//! Here the plumbing exists — `kayfabe_shim_realize` / `kayfabe_shim_unrealize`, and stage
//! Q5's `regs_detach_ram` withdrawing the guest-RAM port before the memory plane goes — but
//! **realize-after-unrealize had never been exercised as a recovery cycle, only as
//! teardown**. Nobody knew whether it worked. This file is the measurement.
//!
//! ## The property, stated so it can fail
//!
//! > After a full unload → reload cycle, the device is indistinguishable from first boot,
//! > and the machine it leaves behind is the machine it found.
//!
//! Two halves, because a device has two kinds of state and they leak differently: the
//! device's *own* state (the emulated GSP, the register plane, the memory plane's ledger),
//! and the *machine's* state that the device merely borrowed (memslot numbers, the
//! migration blocker, the discard policy, references onto the hypervisor's regions). A
//! reload rebuilds the first from nothing; only conservation can save the second.
//!
//! ## ★★★ How "indistinguishable" is checked: DERIVED, not enumerated
//!
//! [`DeviceState`] compares three **whole values** — [`KayfabeAudit`], [`KayfabeRegAudit`]
//! and [`kayfabe_device::PlaneResidue`] — each of whose `PartialEq` is `#[derive]`d over
//! every field it has. A field added to any of them is compared by this test **on the day
//! it is added, with no edit here**. That is deliberate and it is this repository's
//! most-repeated defect shape in reverse: a gate quantified over a hand-written list stops
//! covering what the list stopped naming, silently and with zero red tests.
//!
//! ★★★ **And "derived" has to mean derived all the way down, which it did not.** This file
//! shipped on 2026-07-31 naming `unclaimed_offsets` as its one non-derived member. It was
//! not the only one: the derivation stopped a level *above* the state, because
//! `Shim::audit`, `Regs::audit` and the snapshot itself were hand-written projections. Two
//! guest-driven samples (`fb_window`, `fault_buffer`) were added to the register plane in
//! the days after and were in **no** snapshot at all; twenty of the memory plane's
//! thirty-one counters never crossed the seam. `#130` closed the class rather than the
//! instances: every projection now **destructures its source with no `..`**, so the next
//! field is `error[E0027]` at the projection instead of a silence. See
//! [`kayfabe_device::RegPlane::residue`] and [`Shim::audit`].
//!
//! Two members of the plane are still absent and cannot be otherwise: the guest-RAM port
//! and the command policy are `Box<dyn …>` with no equality. They are the shell's wiring
//! rather than the device's state, they are `_`-bound *by name* in `residue`'s pattern, and
//! they are asserted in [`a_reload_clears_what_a_power_on_reset_keeps`] rather than left
//! silent.
//!
//! ## What this file does NOT observe — stated, not implied
//!
//! **Isolate lifetime is not observable here, and it is not covered.** At stage Q5 the
//! device seam has no isolate in it: `kayfabe-qemu-raw` depends on the memory plane, the
//! register plane and the chip table, and on nothing that holds a host RM object
//! (`kayfabe-isolate`, `kayfabe-isolate-host` and `kayfabe-core` are all absent from its
//! manifest, and the word does not appear in the crate). So the question *"does the isolate
//! die with the device?"* has no answer at this seam — not because it was checked and found
//! fine, but because there is nothing yet to check. When forwarding is wired into *this*
//! seam, a reload that reattaches to a live isolate holding stale host RM objects is a
//! cross-lifetime hazard the C measured (kernel clients hold refcounted references into
//! **user** memory), and this file is where the assertion belongs.
//!
//! ★ **It does have an answer one crate over, and it is measured there.** An isolate is
//! owned by a `kayfabe_core::gpu::Proc`, which is owned by the `Spine`, which is owned by
//! the core device — so the same requirement is asked and answered against *that* device in
//! `tests/tests/device_reload_isolates.rs`, including for a proc so wedged that ordinary
//! reclamation provably never frees it. What is missing here is the wiring between the two
//! devices, not the property.

use std::sync::Arc;

use kayfabe_qemu_raw::shim::{
    BarDesc, KayfabeAudit, KayfabeRegAudit, Regs, SectionWire, Shim, ShimConfig,
};
use kayfabe_vmm::BarId;
use kayfabe_vmm_qemu::host::{BlockerId, MrHandle, SectionDesc, SectionFacts};
use kayfabe_vmm_qemu::mock_host::{MockPolicy, MockQemuHost, MockSlotPlane, MockSlotRecord};

// =====================================================================================
// The device, exactly as the C shim builds and destroys one
// =====================================================================================

/// Guest-physical bases well inside every host's physical-address width. ★ Not a
/// convenience: a guest-physical address above the host CPU's own width is refused, which
/// is 46 bits on some machines this suite runs on.
const BAR0_BASE: u64 = 0x0000_0000_C000_0000;
const BAR1_BASE: u64 = 0x0000_0004_0000_0000;
const BAR2_BASE: u64 = 0x0000_0008_0000_0000;
const PAGE: u64 = 4096;

/// The offsets the guest driver's own bring-up writes, transcribed rather than derived from
/// the register model — this file must be the *second* description, the one that disagrees
/// when the first one moves. `NV_PGSP` = `0x110000`, `NV_PSEC` = `0x840000`;
/// `NV_PFALCON_FALCON_{MAILBOX0,MAILBOX1,CPUCTL}` = `0x040`, `0x044`, `0x100`;
/// `NV_PGSP_QUEUE_HEAD(0)` = `0x110c00`
/// (`ogkm-580: src/common/inc/swref/published/ampere/ga102/dev_gsp.h:27,29,38`).
const GSP_CPUCTL: u64 = 0x0011_0100;
const GSP_MAILBOX0: u64 = 0x0011_0040;
const GSP_MAILBOX1: u64 = 0x0011_0044;
const GSP_QUEUE_HEAD0: u64 = 0x0011_0C00;
const SEC2_CPUCTL: u64 = 0x0084_0100;
const SEC2_MAILBOX0: u64 = 0x0084_0040;
const STARTCPU: u64 = 0x2;

/// Where the test guest keeps its LibOS boot-args array, and how much of it the bind walks
/// — the array's own declared maximum, 4096 entries at 32 bytes
/// (`ogkm-580: src/common/uproc/os/common/include/libos_init_args.h:31, 49-56`).
const BOOT_ARGS_GPA: u64 = 0x1_0000_0000;
const LIBOS_ARRAY_LEN: u64 = 4096 * 32;
/// A second reported section, so the reference ledger has more than one row in it — a
/// conservation bug that only ever loses one row is a bug a one-row fixture cannot see.
const SECOND_RAM_GPA: u64 = 0x2_0000_0000;
const SECOND_RAM_LEN: u64 = 0x1_0000;

/// An offset no source in the chip's table claims, for the unclaimed sample.
///
/// ★★ It used to be `0x0077_7777`, and that was **wrong in a way this fixture could not
/// see**: `0x0077_7777` is inside `PRAMIN`, the framebuffer window, so what this test
/// called "an offset nobody owns" was device memory. Two independent fixtures in this
/// repository had picked a `PRAMIN` address for exactly that reason — the conflation
/// `kayfabe_device::FbWindow` exists to end. See `plane`'s module docs.
const NOBODYS_OFFSET: u64 = 0x0055_5555;

/// An offset inside `PRAMIN` — the framebuffer window that lives *inside* the register
/// aperture, i.e. **device memory**, which is a different fact from an unclaimed register
/// and is classified before it (`kayfabe_device::FbWindow`, `#102` stage C).
///
/// ★ It is exactly the address [`NOBODYS_OFFSET`] used to be, and the reason is the point:
/// this fixture once called `0x0077_7777` "an offset nobody owns" and it was never that.
/// Now it is used for what it actually is.
const PRAMIN_OFFSET: u64 = 0x0077_7777;

fn cfg() -> ShimConfig {
    ShimConfig {
        shareable_ram: true,
        bars: vec![
            BarDesc {
                index: 0,
                base: BAR0_BASE,
                len: 16 << 20,
            },
            BarDesc {
                index: 1,
                base: BAR1_BASE,
                len: 1 << 30,
            },
            BarDesc {
                index: 2,
                base: BAR2_BASE,
                len: 1 << 30,
            },
        ],
    }
}

/// A machine: the host and the memslot plane, both of which **outlive every device on
/// them**. That is the whole point — a fresh one per device would make conservation
/// unfalsifiable.
struct Machine {
    host: Arc<MockQemuHost>,
    slots: Arc<MockSlotPlane>,
}

impl Machine {
    fn new() -> Machine {
        let host = Arc::new(MockQemuHost::with_policy(MockPolicy::default()));
        host.place_bar(BarId::Bar0, BAR0_BASE);
        host.place_bar(BarId::Bar1, BAR1_BASE);
        host.place_bar(BarId::Bar2, BAR2_BASE);
        Machine {
            host,
            slots: Arc::new(MockSlotPlane::new(509, PAGE)),
        }
    }
}

/// One device life: the two planes the C shim holds behind its two opaque handles.
struct Device {
    shim: Shim,
    regs: Regs,
}

/// `nvkvm_realize` followed by the configuration-space write that programs a base-address
/// register, in the C's order and for the C's reasons.
///
/// The register plane is built **first** and needs no base-address register — a guest
/// driver's first act is to read chip-identity registers, and the answer is a function of
/// the chip table alone. The memory plane cannot realize until a base exists, because it
/// installs memslots at one; `attach_ram` is the join, and it is a separate call because
/// the order is fixed by the hypervisor rather than by us.
fn load_device(m: &Machine) -> Device {
    let regs = Regs::create(0).expect("the default chip is servable");
    let shim = Shim::realize(&cfg(), m.host.clone(), m.slots.clone())
        .expect("a cooperative accelerated machine realizes");
    regs.attach_ram(&shim);
    Device { shim, regs }
}

/// `nvkvm_exit`, in ITS order, minus the parts the archive does not own.
///
/// ★ The register plane's guest-RAM port holds a handle onto the memory plane and the
/// register surface keeps answering across this teardown **by design**, so the port is
/// withdrawn *before* the plane it points into is unrealized. The register plane is
/// destroyed last, after the memory plane is gone, because a topology callback still in
/// flight can reach neither.
///
/// ★ The one step deliberately absent is `memory_listener_unregister`, which the C shell
/// performs and the archive has no primitive for — see
/// [`the_topology_listener_is_the_shells_to_withdraw_and_the_archive_says_so`].
fn unload_device(d: Device) {
    d.regs.detach_ram();
    d.shim.unrealize();
    drop(d.regs);
    drop(d.shim);
}

// =====================================================================================
// The two observations
// =====================================================================================

/// ★★★ The device's own state, as a value that can be compared to another device's.
///
/// Three of the four members are whole values with derived equality (see the module docs);
/// the fourth exists because the register plane's state struct holds trait objects and
/// cannot derive any.
#[derive(Debug, PartialEq, Eq)]
struct DeviceState {
    /// The memory plane's complete audit — nine counters, compared as one value.
    memory_plane: KayfabeAudit,
    /// The register plane's audit **in the shape the C shell reads it**. Kept beside the
    /// residue below rather than replaced by it: this is the value that crosses the
    /// `#[repr(C)]` seam, and a reload that was clean inside the process while the wire
    /// value carried the first life's numbers would be a real defect and an invisible one.
    register_plane: KayfabeRegAudit,
    /// ★★★ **The register plane's residue, WHOLE** — counters, the emulated GSP, the
    /// unclaimed sample, the framebuffer-window sample, the unserviced list and the
    /// fault-buffer registrations, as one derived-equality value.
    ///
    /// ★★ It replaces three hand-named members, and the reason is a defect this file had
    /// **already contracted**. When it shipped it named `unclaimed_offsets` as its one
    /// non-derived member and argued, correctly, that a hand-written list stops covering
    /// what it stops naming. By the next task `RegPlane::fb_window_sample` and
    /// `RegPlane::fault_buffer_sample` had been added beside it — both guest-driven, both
    /// bounded, both survivors of a `device_reset` — and both outside every snapshot in
    /// this file. Nothing went red, and nothing could have.
    ///
    /// [`kayfabe_device::RegPlane::residue`] is built by **destructuring the plane and its
    /// locked state with no `..`**, so the next such field is `error[E0027]` on the commit
    /// that adds it. That is the difference between a guarantee and a list.
    register_residue: kayfabe_device::PlaneResidue,
}

fn observe(d: &Device) -> DeviceState {
    DeviceState {
        memory_plane: d.shim.audit(),
        register_plane: d.regs.audit(),
        register_residue: d.regs.plane().residue(),
    }
}

/// ★★★ What the device borrowed from the machine and must give back. A leaked row here is
/// invisible to the device — it is by construction only observable from the machine's side,
/// which is why a device-only snapshot cannot be the whole property.
#[derive(Debug, PartialEq, Eq)]
struct MachineResidue {
    /// Outstanding migration blockers. A leaked one leaves the machine unmigratable
    /// forever, and nothing ever tells the operator why.
    blockers: Vec<BlockerId>,
    /// Whether guest-driven RAM discard is still disabled — a machine-wide facility.
    discard_disabled: bool,
    /// ★★ Every region **this device still has a claim on**, with its outstanding count.
    ///
    /// ★ Filtered on `refs > 0`, and that is the property rather than a convenience. A
    /// region the machine still *lists* at refcount zero is the guest's own memory, which
    /// outlives every device on the bus and always will; what a device must give back is
    /// its **reference**, and a leaked one is a region the hypervisor can never finalize.
    /// Comparing the raw list instead would make the fixture's own `mint_foreign` calls
    /// read as device residue — a test that fails for a reason the device cannot fix is
    /// the same defect as one that cannot fail at all.
    pinned_regions: Vec<(MrHandle, u64)>,
    /// Memslots still live in the kernel.
    live_memslots: Vec<MockSlotRecord>,
    /// Installs that were never cleared. Non-zero means a slot is live in a kernel nobody
    /// is going to tell about it.
    uncleared_installs: u64,
    /// ★★★ Installs that silently REPLACED a live slot with the same number. The kernel
    /// does not report this and neither does the mock; it is counted so its absence can be
    /// asserted. Non-zero after a reload means the second life was handed a number the
    /// first life still owned.
    silent_slot_replaces: u64,
}

fn residue(m: &Machine) -> MachineResidue {
    MachineResidue {
        blockers: m.host.blockers(),
        discard_disabled: m.host.discard_disabled(),
        pinned_regions: m
            .host
            .live_regions()
            .into_iter()
            .filter(|(_, refs)| *refs > 0)
            .collect(),
        live_memslots: m.slots.live(),
        uncleared_installs: m.slots.installs() - m.slots.clears(),
        silent_slot_replaces: m.slots.replaces(),
    }
}

/// ★ Deliberately field-by-field: this direction is what the C shim performs by hand, and
/// writing it out is what would catch an inverted or dropped fact.
fn wire_of(d: SectionDesc) -> SectionWire {
    SectionWire {
        mr: d.mr.0,
        gpa: d.gpa,
        len: d.len,
        offset_within_region: d.offset_within_region,
        is_ram: d.facts.is_ram,
        is_ram_device: d.facts.is_ram_device,
        is_rom_device: d.facts.is_rom_device,
        readonly: d.facts.readonly,
        nonvolatile: d.facts.nonvolatile,
    }
}

// =====================================================================================
// ★★ MEAN, not happy path: the dirty states a guest can actually reach
// =====================================================================================

/// Put the device into every dirty state this seam can reach, at points a guest drives:
///
/// 1. **Guest RAM attached and walked** (Q5) — the emulated GSP follows the guest's own
///    boot-args pointer into real reported memory and scans the LibOS region array.
/// 2. **Mid-GSP-boot** — FWSEC has run, the boot-args mailbox pair is latched, the Booter
///    has loaded and **WPR2 is up**. This is the exact state the C could only leave by
///    restarting the hypervisor.
/// 3. **The command-queue doorbell rung on a queue that never bound** — refused by name,
///    which is a dirty state of its own.
///
///    ⚠ **The ring cursors do NOT advance here, and saying otherwise would be a false
///    claim about this fixture.** Advancing them needs a queue that actually bound, which
///    needs a LibOS region array carrying an `RMARGS` entry *seeded into guest memory*, and
///    the mock machine has no way to write bytes into a reported region — it can only
///    report what the device wrote. `#64`'s cursor case is therefore covered where it is
///    reachable, at the state machine's own seam:
///    `tests/tests/gsp_boot.rs::a_device_reset_after_a_life_that_moved_every_cursor_is_total`,
///    which drives four doorbells through a bound queue and asserts the reset against the
///    cold value as a whole. Measured: a `device_reset` patched to carry `cmd_read_ptr`
///    across is caught there and by nothing here.
/// 4. **Memslots and slot numbers held** — a reservation installed into a base-address
///    register.
/// 5. **The base-address tripwire latched** — the move detector has sampled a register.
/// 6. **Two reported RAM sections outstanding** — references taken and never deleted,
///    which is what a device being unloaded under a live guest actually looks like.
/// 7. **The unclaimed sample polluted** — a guest poking offsets nobody owns.
/// 8. **The framebuffer-window sample polluted** — a guest scribbling in `PRAMIN`.
///
/// ⊘ **One member of the residue this fixture cannot dirty, stated rather than implied:**
/// `PlaneResidue::fault_buffers`. A fault-buffer registration arrives as a `GSP_RM_CONTROL`
/// off the *command queue*, which needs a queue that bound, which needs an `RMARGS` entry
/// seeded into guest memory — and the mock machine can only report what the device wrote
/// (see (3) below). It is carried in the compared value because the point of the value is
/// that it is total; it is not claimed to be exercised here.
fn dirty(m: &Machine, d: &Device) {
    // ★ ONE description of the sequence. This used to be a second copy of the steps below,
    // in a slightly different order, and a fixture that documents a fixture is a fixture
    // that drifts from it.
    corrupt_to(m, d, BootPoint::FbWindowScribbled);
}

// =====================================================================================
// ★★★ MEAN, part two: the boot has POINTS, and a reload must work from every one
// =====================================================================================

/// ★★★ **How far into the boot the guest got before it bricked the device.**
///
/// [`dirty`] drives the whole sequence, which answers *"can the device recover from a
/// fully-dirtied life?"* and only that. It is the easy question. A device can be perfectly
/// recoverable from its end state and unrecoverable from the middle — that is exactly the
/// shape of the C artifact's `#64`, where `publish` reset the command ring's cursor and so
/// the *half-booted* device was the one that could never restart — and it is the shape of
/// its WPR2 note, where the emulated GSP is stuck precisely because a partial bring-up
/// latched something a rebuild does not clear.
///
/// So the cycle is driven from **every prefix of the bring-up**, in the driver's own order.
/// Each variant is the state after its own step and every step before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BootPoint {
    /// Realized, and the guest has not touched it. A reload from *here* must also work: a
    /// device that only recovers once something has happened is a device whose reload path
    /// is doing the recovery.
    Cold,
    /// The hypervisor has reported the guest's RAM; the device holds a reference per
    /// section. Nothing has been written yet.
    RegionsReported,
    /// `FWSEC` released: `NV_PFALCON_FALCON_CPUCTL.STARTCPU` on the GSP falcon.
    FwsecReleased,
    /// The LibOS boot-args pointer pair is latched and the emulated GSP has **followed it
    /// into guest memory** and walked the region array.
    BootArgsPublished,
    /// The Booter has run on `SEC2` and **WPR2 is up** — the state the C research artifact
    /// could leave only by restarting the hypervisor.
    Wpr2Up,
    /// ★ The command-queue doorbell rung on a queue that never bound: refused by name, and
    /// a dirty state of its own. This is the *stale-queue latch* shape — the reset blocker
    /// the C artifact measured on real hardware on 2026-07-25, and its own bench lifecycle
    /// findings put it **ahead of** WPR2 as the thing that actually stops a restart.
    DoorbellOnAnUnboundQueue,
    /// Memslots installed and the base-address tripwire latched.
    MemslotsAndLatch,
    /// The guest has polluted the unclaimed sample by poking offsets nobody owns.
    UnclaimedPolluted,
    /// ★ The guest has scribbled in `PRAMIN` — the framebuffer window inside the register
    /// aperture. A separate point from [`BootPoint::UnclaimedPolluted`] because it is a
    /// separate fact: a *dropped framebuffer write can be a dropped page-table entry*, and
    /// it is recorded in its own bounded sample.
    FbWindowScribbled,
}

/// Every point, in boot order. ★ A `const` array rather than a `#[test]` per point, so
/// adding a point adds it to every quantified test at once — the same reason
/// [`DeviceState`] is compared whole.
const BOOT_POINTS: [BootPoint; 9] = [
    BootPoint::Cold,
    BootPoint::RegionsReported,
    BootPoint::FwsecReleased,
    BootPoint::BootArgsPublished,
    BootPoint::Wpr2Up,
    BootPoint::DoorbellOnAnUnboundQueue,
    BootPoint::MemslotsAndLatch,
    BootPoint::UnclaimedPolluted,
    BootPoint::FbWindowScribbled,
];

/// Drive the bring-up as far as `upto`, inclusive, and stop.
///
/// ★ One description of the sequence, not two: [`dirty`] is this function run to the end.
/// A second copy would be a fixture that drifts from the fixture it documents.
fn corrupt_to(m: &Machine, d: &Device, upto: BootPoint) {
    let want = |p: BootPoint| {
        BOOT_POINTS
            .iter()
            .position(|q| *q == p)
            .expect("a known point")
            <= BOOT_POINTS
                .iter()
                .position(|q| *q == upto)
                .expect("a known point")
    };

    if want(BootPoint::RegionsReported) {
        let libos = m
            .host
            .mint_foreign(BOOT_ARGS_GPA, LIBOS_ARRAY_LEN, SectionFacts::plain_ram());
        d.shim
            .region_add(wire_of(libos))
            .expect("plain memory is taken");
        let extra = m
            .host
            .mint_foreign(SECOND_RAM_GPA, SECOND_RAM_LEN, SectionFacts::plain_ram());
        d.shim
            .region_add(wire_of(extra))
            .expect("plain memory is taken");
    }
    if want(BootPoint::FwsecReleased) {
        let _ = d.regs.write(0, GSP_CPUCTL, 4, STARTCPU);
    }
    if want(BootPoint::BootArgsPublished) {
        let _ = d
            .regs
            .write(0, GSP_MAILBOX0, 4, BOOT_ARGS_GPA & 0xFFFF_FFFF);
        let _ = d.regs.write(0, GSP_MAILBOX1, 4, BOOT_ARGS_GPA >> 32);
    }
    if want(BootPoint::Wpr2Up) {
        let _ = d.regs.write(0, SEC2_MAILBOX0, 4, 0);
        let _ = d.regs.write(0, SEC2_CPUCTL, 4, STARTCPU);
    }
    if want(BootPoint::DoorbellOnAnUnboundQueue) {
        let _ = d.regs.write(0, GSP_QUEUE_HEAD0, 4, 1);
    }
    if want(BootPoint::MemslotsAndLatch) {
        d.shim
            .install_window(BAR1_BASE, 64 * PAGE)
            .expect("a reservation in a register the hypervisor does not back");
        d.shim
            .bar_move_requested(1)
            .expect_err("a reservation latches its register, and the preventer must say so");
    }
    if want(BootPoint::UnclaimedPolluted) {
        let _ = d.regs.read(0, NOBODYS_OFFSET, 4);
        let _ = d.regs.write(0, NOBODYS_OFFSET + 8, 4, 0xDEAD_BEEF);
    }
    if want(BootPoint::FbWindowScribbled) {
        let _ = d.regs.read(0, PRAMIN_OFFSET, 4);
        let _ = d.regs.write(0, PRAMIN_OFFSET + 8, 4, 0xDEAD_BEEF);
    }
}

// =====================================================================================
// ★★★ THE NON-VACUITY GATE — and it runs before the property, not after
// =====================================================================================

/// ★★★ **What would the recovery test fail on?** This.
///
/// A recovery test that passes because it measures nothing is worse than no test: it turns
/// an unknown into a false assurance. So the dirtying is asserted to be *visible through
/// the very comparison the property uses*, field by named field, before any cycle happens.
/// If [`dirty`] ever stops dirtying — a refused write, a moved register offset, a guest-RAM
/// port that silently stopped being attached — this goes red HERE, naming which plane went
/// quiet, instead of making the reload test green for the wrong reason.
#[test]
fn the_dirty_state_is_visible_through_the_comparison_the_property_uses() {
    let m = Machine::new();
    let clean = Machine::new();

    let device = load_device(&m);
    let untouched = observe(&device);
    let control = load_device(&clean);
    assert_eq!(
        untouched,
        observe(&control),
        "two devices realized the same way must start out equal, or the comparison is \
         measuring the realize path rather than the guest's traffic"
    );

    dirty(&m, &device);
    let soiled = observe(&device);

    assert_ne!(
        soiled, untouched,
        "★ the dirtying must be observable AT ALL through this comparison"
    );

    // …and now which parts of it, because `assert_ne!` on a four-member struct passes if
    // any ONE member moved, and three of the four could have gone quiet unnoticed.
    assert_ne!(
        soiled.register_residue.gsp, untouched.register_residue.gsp,
        "the emulated GSP must have moved"
    );
    assert!(
        soiled.register_residue.gsp.phase().wpr2_up(),
        "★★ WPR2 must be UP — this is the exact latch the C artifact could only clear by \
         restarting the hypervisor, and cycling a device that never raised it would prove \
         nothing about reload. Phase reached: {:?}",
        soiled.register_residue.gsp.phase()
    );
    assert!(
        soiled.memory_plane.live_memslots > 0,
        "a reservation must be holding memslots"
    );
    assert_eq!(
        soiled.memory_plane.topology_adds, 2,
        "both reported sections must have been taken"
    );
    assert_eq!(
        soiled.memory_plane.topology_dels, 0,
        "★ and NEITHER deleted — a device unloaded under a live guest is exactly the case \
         where the shell never gets to replay the deletions"
    );
    assert!(
        soiled.memory_plane.bar_base_checks > untouched.memory_plane.bar_base_checks,
        "the base-address tripwire must have sampled"
    );
    assert!(
        device.shim.bar_move_requested(1).is_err(),
        "★ and the latch is SET — the reloaded device's must be cold again"
    );
    assert!(
        soiled.register_plane.gsp_writes >= 6,
        "the register plane must have served the boot sequence, got {}",
        soiled.register_plane.gsp_writes
    );
    assert!(
        !soiled.register_residue.unclaimed.is_empty(),
        "the unclaimed sample must be polluted"
    );
    // ★★★ Each residue member that this fixture can reach is asserted BY NAME, and not
    // because the struct-level `assert_ne!` above is weak — because it is *too strong to
    // be informative*. A member that stopped being carried, or one nothing dirties, leaves
    // `assert_ne!` green on the strength of a sibling, which is how `fb_window` and
    // `fault_buffers` sat outside every snapshot in this file for a week without a red
    // test. These lines are what a removed member fails on.
    assert!(
        !soiled.register_residue.fb_window.is_empty(),
        "★★ the framebuffer-window sample must be polluted — the guest scribbled in \
         `PRAMIN`, and a dropped write there can be a dropped page-table entry, which is \
         why it is a fact of its own and not an unclaimed register"
    );
    assert!(
        soiled.register_residue.counters.fb_window_reads > 0
            && soiled.register_residue.counters.fb_window_writes > 0,
        "…and counted, in both directions: {:?}",
        soiled.register_residue.counters
    );
    assert_eq!(
        soiled.register_residue.fault_buffers_registered, 0,
        "⊘ **STATED, not asserted-as-a-property**: this fixture cannot reach a fault-buffer \
         registration at all. It arrives as a `GSP_RM_CONTROL` off the command queue, which \
         needs a queue that bound, which needs an `RMARGS` entry seeded into guest memory — \
         and the mock machine can only report what the device wrote. The member is carried \
         because the compared value must be total; it is NOT exercised here, and this line \
         is here so that nobody reads its inclusion as coverage. If it ever goes red, the \
         fixture gained a bound queue and this file's `dirty` should use it."
    );

    // The machine must be visibly encumbered too, or the conservation half is vacuous.
    let held = residue(&m);
    assert_eq!(held.blockers.len(), 1, "the migration blocker is held");
    assert!(held.discard_disabled, "the discard policy is held");
    assert!(
        !held.live_memslots.is_empty(),
        "memslots are live in the kernel"
    );
    assert_eq!(
        held.pinned_regions.len(),
        2,
        "★★ a reference is OUTSTANDING on each of the two reported sections — the row the \
         reload test turns on, and the one the archive used to keep forever: {:?}",
        held.pinned_regions
    );

    unload_device(device);
    unload_device(control);
}

// =====================================================================================
// ★★★ THE PROPERTY
// =====================================================================================

/// ★★★ **The headline: a reloaded device is indistinguishable from a first boot.**
///
/// The first life is driven into every dirty state [`dirty`] can reach — WPR2 up, guest RAM
/// attached and walked, ring cursors rung, memslots and reference counts held — and then
/// unloaded and reloaded **on the same machine**. The reloaded device is compared, as a
/// whole value, against a control device realized identically on a machine that has never
/// carried one.
///
/// The comparison is derived (module docs), so this asserts *every* field of the memory
/// audit, the register audit and the emulated GSP — not the ones that were interesting on
/// the day it was written.
#[test]
fn a_reloaded_device_is_indistinguishable_from_a_first_boot() {
    let m = Machine::new();

    let first = load_device(&m);
    dirty(&m, &first);
    assert!(
        observe(&first).register_residue.gsp.phase().wpr2_up(),
        "precondition: the device really is bricked-shaped before the reload"
    );
    unload_device(first);

    let second = load_device(&m);

    let virgin = Machine::new();
    let control = load_device(&virgin);

    assert_eq!(
        observe(&second),
        observe(&control),
        "★★★ a reloaded device must be indistinguishable from a first boot — every \
         counter, every field of the emulated GSP's state machine, and the unclaimed \
         sample"
    );
    // ★ The base-address latch is not in the audit — it is a piece of state whose only
    // observation is the preventer's answer, so it gets its own line rather than being
    // trusted to the snapshot.
    second
        .shim
        .bar_move_requested(1)
        .expect("★★ the reloaded device's base-address latch is cold, as a first boot's is");

    unload_device(second);
    unload_device(control);
}

/// ★★★ **The other half: the machine a device leaves behind is the machine it found.**
///
/// Not the same statement as the one above, and it cannot be derived from it: everything
/// here is invisible from inside the device. A leaked blocker, a leaked reference or an
/// uncleared memslot leaves a *reloaded* device looking perfect while the machine
/// underneath it accumulates.
///
/// ★ Measured, and it is a **finding**: before 2026-07-31 this test was red on
/// `pinned_regions`, with six rows — one per reported section per life. `unrealize` reset its view with `*v = View::default()`, dropping
/// the map that held every reported section's `MrHandle` — an opaque name, not an owning
/// value — so the reference `region_add` took was never released. The shipping C shell
/// masked it by unregistering the topology listener first (QEMU's
/// `memory_listener_unregister` replays `region_del` over every flat range,
/// `system/memory.c:3112-3137`), which means the archive's own contract was false and only
/// one caller's ordering made it true. Fixed in `QemuMachine::unrealize` by draining the
/// map and releasing what is left, which is safe under both orderings.
#[test]
fn the_machine_a_reloaded_device_leaves_behind_is_the_machine_it_found() {
    let m = Machine::new();
    let pristine = residue(&m);

    for _ in 0..3 {
        let d = load_device(&m);
        dirty(&m, &d);
        unload_device(d);
    }

    let after = residue(&m);
    assert_eq!(
        after.blockers, pristine.blockers,
        "an unwithdrawn blocker leaves the machine permanently unmigratable"
    );
    assert_eq!(
        after.discard_disabled, pristine.discard_disabled,
        "a device that leaves discard disabled has taken a machine-wide facility away"
    );
    assert_eq!(
        after.live_memslots, pristine.live_memslots,
        "a memslot live in a kernel nobody is going to tell"
    );
    assert_eq!(
        after.uncleared_installs, 0,
        "every install must have been cleared"
    );
    assert_eq!(
        after.silent_slot_replaces, 0,
        "★★★ a later life was handed a memslot number an earlier life still owned — the \
         install the kernel turns from an ADD into a silent REPLACE, and neither the \
         kernel nor this mock reports it"
    );
    // ★★ THE ROW THIS TEST WAS WRITTEN FOR, stated on its own so its failure names itself
    // rather than arriving inside a six-field struct diff. Three lives, two sections each:
    // an unbalanced ledger shows six.
    assert!(
        after.pinned_regions.is_empty(),
        "★★★ {} region reference(s) survived the device that took them — the hypervisor \
         can never finalize these, and every further reload adds more: {:?}",
        after.pinned_regions.len(),
        after.pinned_regions
    );

    assert_eq!(
        after, pristine,
        "★★★ and NOTHING ELSE moved either — the whole residue, so a row added to \
         `MachineResidue` is covered without an edit here"
    );
}

/// ★★ Non-vacuity for the conservation test: the instrument really does see a leak.
///
/// Three lives, two reported sections each, and **none of them deleted**. If the residue
/// snapshot could not see an outstanding reference, the test above would be green on an
/// archive that pinned every region a guest ever showed it.
#[test]
fn the_residue_snapshot_can_see_an_outstanding_reference() {
    let m = Machine::new();
    let d = load_device(&m);
    dirty(&m, &d);

    let held = residue(&m);
    assert_eq!(
        held.pinned_regions.len(),
        2,
        "while the device is LIVE both references must be outstanding and visible, or the \
         conservation test is asserting the absence of something it could never detect"
    );

    // …and they go away when the shell deletes the sections, which is the other ordering.
    d.shim.region_del(BOOT_ARGS_GPA, LIBOS_ARRAY_LEN);
    d.shim.region_del(SECOND_RAM_GPA, SECOND_RAM_LEN);
    assert!(
        residue(&m).pinned_regions.is_empty(),
        "the deletion path releases them — so the two orderings are separable"
    );
    unload_device(d);
}

// =====================================================================================
// The function-level reset — the OTHER recovery path, and it is not the same one
// =====================================================================================

/// ★★★ **The power-on reset puts the emulated GSP back to cold, in-process.**
///
/// This is the `nvkvm_reset_hold` path — a PCI function-level reset or a guest reboot,
/// which is what a real card gets and what the C artifact could not give one: its WPR2
/// latch only cleared on a full hypervisor restart, and its reset was field-by-field at
/// four separate sites that disagree with each other.
///
/// Asserted against `GspFsm::new` **as a whole value**, so "total" means total.
#[test]
fn a_power_on_reset_puts_the_emulated_gsp_back_to_cold() {
    let m = Machine::new();
    let d = load_device(&m);
    let cold = observe(&d).register_residue.gsp;

    dirty(&m, &d);
    let hot = d.regs.plane().gsp_state();
    assert_ne!(hot, cold, "precondition: the state machine moved");
    assert!(hot.phase().wpr2_up(), "precondition: WPR2 is up");

    d.regs.reset();

    assert_eq!(
        d.regs.plane().gsp_state(),
        cold,
        "★★★ a power-on reset must rebuild the emulated GSP, not clear the fields \
         somebody remembered"
    );
    unload_device(d);
}

/// ★★ **A reset is NOT a reload, and the difference is named rather than left to be
/// discovered.**
///
/// MEASURED, 2026-07-31: `RegPlane::device_reset` rebuilds the state machine and nothing
/// else. Three things survive it, and they are three different kinds of thing:
///
/// - the **counters** and the **unclaimed sample** — diagnostics, not device state, but
///   the sample is bounded at 64 entries and a guest can fill it, so one life's poking can
///   crowd the next life's diagnostics out. Guest-influenced, diagnostic-only.
/// - the **guest-RAM port** and the **command policy** — the shell's wiring, and they
///   survive **by decision** (`plane.rs`: *"the RAM port and the policy survive, because
///   they are the shell's wiring and not the device's state"*). A real function-level
///   reset does not unplug the card from the bus either.
///
/// Only a reload clears the first group, because only a reload destroys the plane. This
/// test exists so that stops being an unwritten assumption: if a future reset is made
/// total, this is the test that says so.
#[test]
fn a_reload_clears_what_a_power_on_reset_keeps() {
    let m = Machine::new();

    let d = load_device(&m);
    dirty(&m, &d);
    d.regs.reset();
    let after_reset = observe(&d);

    assert!(
        !after_reset.register_residue.unclaimed.is_empty(),
        "MEASURED: the unclaimed sample survives a power-on reset"
    );
    assert!(
        after_reset.register_plane.writes > 0,
        "MEASURED: the register plane's counters survive a power-on reset"
    );
    unload_device(d);

    let reloaded = load_device(&m);
    let after_reload = observe(&reloaded);
    assert!(
        after_reload.register_residue.unclaimed.is_empty(),
        "…and a reload does clear it, because a reload destroys the plane"
    );
    assert_eq!(
        after_reload.register_plane,
        KayfabeRegAudit::default(),
        "…and every counter with it"
    );
    unload_device(reloaded);
}

// =====================================================================================
// The seams a reload depends on, each with its own sentence
// =====================================================================================

/// ★★ The guest-RAM port must NOT survive its memory plane.
///
/// The register surface keeps answering across a memory-plane teardown by design, so the
/// port that reaches into the memory plane is withdrawn explicitly rather than implied by
/// a lifetime the C cannot see. A reload that left the old port installed would have the
/// new device's emulated GSP following guest pointers into a machine that has released its
/// slots — and it would look like a working reload until the first bind.
#[test]
fn the_guest_ram_port_is_withdrawn_before_the_plane_it_points_into() {
    let m = Machine::new();
    let d = load_device(&m);
    let libos = m
        .host
        .mint_foreign(BOOT_ARGS_GPA, LIBOS_ARRAY_LEN, SectionFacts::plain_ram());
    d.shim.region_add(wire_of(libos)).expect("taken");

    // With the port and a region behind it, the bind reads guest memory and stops for a
    // PROTOCOL reason — the array carries no RMARGS entry.
    let _ = d.regs.write(0, GSP_CPUCTL, 4, STARTCPU);
    let _ = d
        .regs
        .write(0, GSP_MAILBOX0, 4, BOOT_ARGS_GPA & 0xFFFF_FFFF);
    let w = d.regs.write(0, GSP_MAILBOX1, 4, BOOT_ARGS_GPA >> 32);
    assert_eq!(w.fault, Some("GspFault::RmargsRegionAbsent"));
    assert_eq!(w.ram_refusal, None, "guest memory was readable");

    // Withdraw it, exactly as `nvkvm_exit` does before unrealizing.
    d.regs.detach_ram();
    d.regs.reset();
    let _ = d.regs.write(0, GSP_CPUCTL, 4, STARTCPU);
    let _ = d
        .regs
        .write(0, GSP_MAILBOX0, 4, BOOT_ARGS_GPA & 0xFFFF_FFFF);
    let w = d.regs.write(0, GSP_MAILBOX1, 4, BOOT_ARGS_GPA >> 32);
    assert_eq!(
        w.fault,
        Some("GspFault::GuestRam"),
        "with the port withdrawn every guest-memory access is refused by name"
    );
    assert_eq!(
        w.ram_refusal.expect("a refusal carries its address").why,
        kayfabe_device::plane::NO_RAM_PORT,
        "…and by the RIGHT name: 'there is no port' and 'nothing is mapped there' send a \
         reader to different places"
    );
    unload_device(d);
}

/// ★ The reloaded device's memory plane is a NEW one: operations on the old handle stay
/// refused, and the refusal is counted.
///
/// This is what stops a reload from being a reattach. A shell holding a stale handle must
/// get a named refusal rather than a plane that quietly still works, because a plane that
/// quietly still works is a device with two owners.
#[test]
fn the_unloaded_planes_stay_dead_while_the_reloaded_one_serves() {
    let m = Machine::new();
    let first = load_device(&m);
    dirty(&m, &first);
    first.regs.detach_ram();
    first.shim.unrealize();

    let err = first
        .shim
        .install_window(BAR1_BASE, 64 * PAGE)
        .expect_err("the memory plane is gone");
    assert_eq!(
        err,
        (
            kayfabe_qemu_raw::shim::Status::Unsupported,
            kayfabe_vmm_qemu::MEMORY_PLANE_AFTER_UNREALIZE
        )
    );
    assert_eq!(first.shim.audit().ops_refused_after_unrealize, 1);
    drop(first.regs);
    drop(first.shim);

    let second = load_device(&m);
    second
        .shim
        .install_window(BAR1_BASE, 64 * PAGE)
        .expect("★ and the RELOADED plane serves the very operation the dead one refused");
    unload_device(second);
}

/// ★★ The topology listener is the SHELL's to withdraw, and the archive says so by having
/// no way to do it.
///
/// `QemuHost` declares `register_listener` and no unregister: the listener object belongs
/// to the C device, and `nvkvm_exit` calls `memory_listener_unregister` on it. This test
/// pins that division rather than letting the asymmetry read as an oversight — and it is
/// the reason the reference-conservation fix above had to be safe under **both** orderings,
/// since the shell may or may not have replayed the deletions before unrealize runs.
#[test]
fn the_topology_listener_is_the_shells_to_withdraw_and_the_archive_says_so() {
    let m = Machine::new();
    let d = load_device(&m);
    assert!(m.host.listening(), "realize registers it");
    unload_device(d);
    assert!(
        m.host.listening(),
        "★ MEASURED: unrealize does NOT withdraw it, because the archive has no primitive \
         to withdraw it with. The C shell owns the listener object and unregisters it in \
         `nvkvm_exit`, before it unrealizes the plane. If this ever goes red, an \
         unregister primitive was added and this file's `unload_device` must call it."
    );
}

// =====================================================================================
// ★★★ THE CYCLE FROM EVERY POINT OF THE BOOT
// =====================================================================================

/// ★★★ **The non-vacuity gate for [`BOOT_POINTS`], and it runs before the property.**
///
/// "The cycle was driven from eight points" is worth nothing if two of those points leave
/// the device in the same state — that would be one observation reported nine times, which
/// is the shape this repository has caught most often. Every point is asserted to produce a
/// state **distinct from every other**, so a step that silently stopped biting (a refused
/// write, a moved offset, a guest-RAM port that is no longer attached) goes red *here*,
/// naming the two points that collided, instead of making eight reload tests green for the
/// wrong reason.
#[test]
fn every_boot_point_leaves_the_device_in_a_distinct_state() {
    let m = Machine::new();
    let mut seen: Vec<(BootPoint, DeviceState)> = Vec::new();
    for p in BOOT_POINTS {
        let d = load_device(&m);
        corrupt_to(&m, &d, p);
        let s = observe(&d);
        for (q, t) in &seen {
            assert_ne!(
                &s, t,
                "★★ boot points {p:?} and {q:?} are INDISTINGUISHABLE — one of the two \
                 steps between them stopped dirtying the device, so the reload test below \
                 would be measuring the same thing twice",
            );
        }
        seen.push((p, s));
        unload_device(d);
    }
    assert_eq!(seen.len(), BOOT_POINTS.len(), "every point was reached");
}

/// ★★★ **THE HEADLINE, quantified: a reload is a first boot from EVERY point of the boot.**
///
/// `a_reloaded_device_is_indistinguishable_from_a_first_boot` asks the question once, of a
/// fully-dirtied life. This asks it of every prefix — because the failures this requirement
/// exists for are *partial* ones. The C artifact's `#64` was a half-published ring whose
/// cursor a rebuild did not clear; its WPR2 note is a half-brought-up GSP that only a
/// hypervisor restart could leave. A device that recovers from its end state and not from
/// its middle is precisely as unusable as one that recovers from neither, and it passes a
/// single-point test.
///
/// ★ Every cycle runs on **one machine**, so a residue that only appears after the eighth
/// load is visible; the machine is asserted back to pristine at the end, which is the half
/// no device-side snapshot can see.
#[test]
fn a_reload_from_every_point_of_the_boot_is_a_first_boot() {
    let m = Machine::new();
    let pristine = residue(&m);

    let virgin = Machine::new();
    let control = load_device(&virgin);
    let cold = observe(&control);

    for p in BOOT_POINTS {
        let first = load_device(&m);
        corrupt_to(&m, &first, p);
        unload_device(first);

        let second = load_device(&m);
        assert_eq!(
            observe(&second),
            cold,
            "★★★ a device reloaded after a life that reached {p:?} must be \
             indistinguishable from a first boot — every counter, every field of the \
             emulated GSP, both bounded samples, the unserviced list and the fault-buffer \
             registrations",
        );
        // ★ The base-address latch has no counter: its only observation is the preventer's
        // answer, so it is asked rather than trusted to the snapshot.
        second
            .shim
            .bar_move_requested(1)
            .expect("the reloaded device's base-address latch is cold, as a first boot's is");
        unload_device(second);
    }

    unload_device(control);
    assert_eq!(
        residue(&m),
        pristine,
        "★★ and after eighteen device loads on this one machine — nine points, two loads \
         each — it is the machine it started as: no leaked reference, blocker or memslot \
         accumulated one per cycle",
    );
}
