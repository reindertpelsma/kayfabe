//! ★★★★★ **w395 — THE GSP SUBMIT IS A SCHEDULE, at the register plane, on the DEFAULT arm.**
//!
//! `KAYFABE_GSP_SUBMIT_ASYNC` is absent in this process, so these run the shipping default
//! (`on`): a `QUEUE_HEAD` store validates and returns. What must survive the deferral,
//! measured here rather than asserted:
//!
//! 1. **E8, the stale-binding refusal, still fires IN THE STORE.** A guest that dropped its
//!    queues (Booter Unload → `Halted`) and is still ringing is refused by name on the vCPU,
//!    with zero guest RAM read — not accepted, kicked, and refused later on a worker where
//!    no trap can be attributed.
//! 2. **E12, the healthy pre-bind doorbell, is classified and NOT kicked.** Nothing is owed
//!    for a ring that does not exist yet.
//! 3. The worker started (the arm is `on`) and the census prints all zeros for a boot that
//!    never bound — *"armed and idle"* is a positive observation.
//!
//! ⊘ The bound case — a store that DOES kick, and a worker that DOES service — needs a
//! LibOS array and a message queue in guest RAM, which this mock fixture does not build.
//! `kayfabe-crec/tests/cap1_deferred_submit.rs` covers it over 359 062 recorded records, and
//! only a live boot covers the worker racing the guest.

use std::sync::Arc;

use kayfabe_qemu_raw::shim::{BarDesc, Regs, SectionWire, Shim, ShimConfig};
use kayfabe_device::Faulted;
use kayfabe_vmm::BarId;
use kayfabe_vmm_qemu::host::{SectionDesc, SectionFacts};
use kayfabe_vmm_qemu::mock_host::{MockPolicy, MockQemuHost, MockSlotPlane};

const BAR0_BASE: u64 = 0x0000_0000_C000_0000;
const BAR1_BASE: u64 = 0x0000_0004_0000_0000;
const BAR2_BASE: u64 = 0x0000_0008_0000_0000;
const PAGE: u64 = 4096;

/// The offsets the guest driver's own bring-up writes, transcribed
/// (`ogkm-580: src/common/inc/swref/published/ampere/ga102/dev_gsp.h`).
const GSP_CPUCTL: u64 = 0x0011_0100;
const GSP_MAILBOX0: u64 = 0x0011_0040;
const GSP_MAILBOX1: u64 = 0x0011_0044;
const GSP_QUEUE_HEAD0: u64 = 0x0011_0C00;
const SEC2_CPUCTL: u64 = 0x0084_0100;
const SEC2_MAILBOX0: u64 = 0x0084_0040;
const STARTCPU: u64 = 0x2;
/// The Booter **Unload** argument (`kayfabe_device::ga10x::SEC2_BOOTER_UNLOAD`), transcribed
/// so this file is the second description.
const SEC2_BOOTER_UNLOAD: u64 = 0xff;

const BOOT_ARGS_GPA: u64 = 0x1_0000_0000;
const LIBOS_ARRAY_LEN: u64 = 4096 * 32;

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

struct Device {
    shim: Shim,
    regs: Regs,
}

fn load_device(m: &Machine) -> Device {
    let regs = Regs::create(0).expect("the default chip is servable");
    let shim = Shim::realize(&cfg(), m.host.clone(), m.slots.clone())
        .expect("a cooperative accelerated machine realizes");
    regs.attach_ram(&shim);
    Device { shim, regs }
}

fn unload_device(d: Device) {
    d.regs.detach_ram();
    d.shim.unrealize();
}

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
        fd_backed: d.backing.is_some(),
        backing_dev: d.backing.map_or(0, |b| b.dev),
        backing_ino: d.backing.map_or(0, |b| b.ino),
        file_offset_of_region: d.backing.map_or(0, |b| b.file_offset_of_region),
    }
}

/// Bring the guest up to WPR2-up with a bind ATTEMPTED (the mock's LibOS array is zeros, so
/// the bind refuses and the queue stays `Unbound` — exactly the state the E12 case needs).
fn boot_to_wpr2_up(m: &Machine, d: &Device) {
    let libos = m
        .host
        .mint_foreign(BOOT_ARGS_GPA, LIBOS_ARRAY_LEN, SectionFacts::plain_ram());
    d.shim
        .region_add(wire_of(libos))
        .expect("plain memory is taken");
    let _ = d.regs.write(0, GSP_CPUCTL, 4, STARTCPU);
    let _ = d
        .regs
        .write(0, GSP_MAILBOX0, 4, BOOT_ARGS_GPA & 0xFFFF_FFFF);
    let _ = d.regs.write(0, GSP_MAILBOX1, 4, BOOT_ARGS_GPA >> 32);
    let _ = d.regs.write(0, SEC2_MAILBOX0, 4, 0);
    let _ = d.regs.write(0, SEC2_CPUCTL, 4, STARTCPU);
}

#[test]
fn a_pre_bind_doorbell_is_classified_e12_and_kicks_nothing() {
    let m = Machine::new();
    let d = load_device(&m);
    boot_to_wpr2_up(&m, &d);
    let lane = d
        .regs
        .plane()
        .gsp_submit_lane()
        .expect("the default arm is `on`, so the plane must hold an armed lane");
    let before = d.regs.plane().counters();

    let out = d.regs.write(0, GSP_QUEUE_HEAD0, 4, 1);
    assert!(out.claimed);
    assert_eq!(out.fault, None, "a pre-bind doorbell is healthy 580 boot order, not a fault");
    assert!(
        !out.gsp_service_scheduled,
        "nothing is owed for a ring that does not exist yet"
    );
    assert_eq!(out.commands, 0);
    assert_eq!(out.transitions, 1, "exactly E12");
    assert_eq!(lane.depth(), 0);
    assert_eq!(lane.stats().queued, 0, "E12 must not kick the lane");
    let after = d.regs.plane().counters();
    assert_eq!(after.commands, before.commands);
    assert_eq!(after.faults, before.faults);
    unload_device(d);
}

#[test]
fn the_stale_binding_refusal_e8_still_fires_in_the_store_on_the_deferred_arm() {
    let m = Machine::new();
    let d = load_device(&m);
    boot_to_wpr2_up(&m, &d);
    // The Booter Unload: WPR2 down, `Halted`, and — the load-bearing half — a binding has
    // now existed in this device life, so an unbound doorbell is the attack signature.
    let _ = d.regs.write(0, SEC2_MAILBOX0, 4, SEC2_BOOTER_UNLOAD);
    let _ = d.regs.write(0, SEC2_CPUCTL, 4, STARTCPU);
    assert_eq!(
        d.regs.plane().phase(),
        kayfabe_device::BootPhase::Halted,
        "the fixture must reach Halted or the assertion below is about the wrong gate"
    );
    let lane = d.regs.plane().gsp_submit_lane().expect("armed");
    let faults_before = d.regs.plane().counters().faults;

    let out = d.regs.write(0, GSP_QUEUE_HEAD0, 4, 1);
    assert!(out.claimed);
    assert_eq!(
        out.fault,
        Some(kayfabe_gsp::GspFault::QueueNotBound.fault_tag().0),
        "E8 must refuse BY NAME, in the store, on the vCPU — not be accepted and kicked"
    );
    assert!(!out.gsp_service_scheduled, "a refused doorbell owes nothing");
    assert_eq!(lane.stats().queued, 0, "E8 must not kick the lane");
    assert_eq!(
        d.regs.plane().counters().faults,
        faults_before + 1,
        "the refusal is counted where the control counted it"
    );
    unload_device(d);
}

/// ★ Armed and idle is a positive observation: the lane exists on the default arm, the
/// worker was started at attach and joined at detach, and a boot that never bound carries a
/// census of zeros — not an absent line.
#[test]
fn the_default_arm_arms_the_lane_and_a_boot_that_never_binds_carries_a_zero_census() {
    let m = Machine::new();
    let d = load_device(&m);
    let lane = d
        .regs
        .plane()
        .gsp_submit_lane()
        .expect("KAYFABE_GSP_SUBMIT_ASYNC absent ⇒ `on` ⇒ armed");
    let c = lane.census();
    for want in ["GSPQUEUE queued=0", "taken=0", "passes=0", "commands=0", "depth=0"] {
        assert!(c.contains(want), "{want:?} missing from {c:?}");
    }
    assert_eq!(d.regs.plane().gsp_service_orphaned(), 0);
    unload_device(d);
    assert!(lane.stopping(), "detach must stop the worker's lane");
}
