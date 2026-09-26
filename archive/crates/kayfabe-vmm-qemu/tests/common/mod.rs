//! One device shape, shared by every test binary, so the whole suite describes **one**
//! device rather than six differently-configured ones.
//!
//! `#![allow(dead_code)]` because integration-test binaries each compile this module
//! separately and none of them uses all of it — the alternative is a per-file copy, which
//! is how two files end up describing two devices.

#![allow(dead_code)]

use std::sync::Arc;

use kayfabe_linux_raw::HostPageSize;
use kayfabe_vmm::{BarId, TrapMode};
use kayfabe_vmm_qemu::host::{BarPlacement, SectionFacts};
use kayfabe_vmm_qemu::mock_host::{MockPolicy, MockQemuHost, MockSlotPlane};
use kayfabe_vmm_qemu::{MachineConfig, QemuMachine, TrapSpec, WindowSpec};

/// The register BAR: trapped, and where the read-native window is installed at runtime.
pub const BAR0_BASE: u64 = 0x8000_0000;
/// The aperture BAR: where the realize-time reservation lives.
pub const BAR1_BASE: u64 = 0x9000_0000;
/// Guest RAM, as the topology listener will report it. Deliberately far from both BARs.
pub const FOREIGN_RAM: u64 = 0x1000_0000;

/// The memslot ceiling the mock kernel reports. A real one reports hundreds; this is in
/// the same class and is deliberately **not** round, so a test that accidentally hardcodes
/// a slot number rather than deriving it from the ceiling stands out.
pub const MOCK_CEILING: u32 = 509;

/// The host page size every offset in the suite is expressed in.
#[must_use]
pub fn page() -> u64 {
    HostPageSize::query().bytes()
}

/// Where the reservation starts.
#[must_use]
pub fn window_gpa() -> u64 {
    BAR1_BASE
}

/// How long the reservation is.
#[must_use]
pub fn window_len() -> u64 {
    8 * page()
}

/// Where the runtime read-native window is installed.
#[must_use]
pub fn overlay_gpa() -> u64 {
    BAR0_BASE + 16 * page()
}

/// How long it is.
#[must_use]
pub fn overlay_len() -> u64 {
    2 * page()
}

/// Its write-trap sub-range, in guest-physical addresses.
#[must_use]
pub fn overlay_trap() -> core::ops::Range<u64> {
    overlay_gpa()..overlay_gpa() + page()
}

/// The device's BAR layout.
///
/// ★ 1024 pages each, which is larger than any single test needs on purpose: the slot
/// allocator's exhaustion test installs a whole budget's worth of two-page windows, and a
/// BAR that ran out of *address space* first would make that test refuse for the wrong
/// reason — and pass, because a refusal is a refusal.
#[must_use]
pub fn bars() -> Vec<BarPlacement> {
    let p = page();
    vec![
        BarPlacement {
            bar: BarId::Bar0,
            base: BAR0_BASE,
            len: 1024 * p,
        },
        BarPlacement {
            bar: BarId::Bar1,
            base: BAR1_BASE,
            len: 1024 * p,
        },
    ]
}

/// The device's realize-time configuration.
#[must_use]
pub fn config() -> MachineConfig {
    let p = page();
    MachineConfig {
        shareable_ram: true,
        bars: bars(),
        windows: vec![WindowSpec::passthrough(window_gpa(), window_len())],
        traps: vec![
            TrapSpec {
                bar: BarId::Bar0,
                range: 0..16 * p,
                mode: TrapMode::ReadWrite,
            },
            TrapSpec {
                bar: BarId::Bar0,
                range: 16 * p..18 * p,
                mode: TrapMode::WriteOnly,
            },
        ],
    }
}

/// A cooperative host whose BARs are already programmed where the configuration says.
///
/// ★ Programming them is not a formality: a window in a BAR the guest has not programmed
/// is refused by name, which is the C's own guard, so a host that skipped this step would
/// make every realize fail.
#[must_use]
pub fn host_with(policy: MockPolicy) -> Arc<MockQemuHost> {
    let h = Arc::new(MockQemuHost::with_policy(policy));
    h.place_bar(BarId::Bar0, BAR0_BASE);
    h.place_bar(BarId::Bar1, BAR1_BASE);
    h
}

/// The mock kernel's memslot plane.
#[must_use]
pub fn slot_plane() -> Arc<MockSlotPlane> {
    Arc::new(MockSlotPlane::new(MOCK_CEILING, page()))
}

/// A realized machine, the mock host and the mock kernel — all three kept, because most of
/// this suite's assertions are about what one of the two doubles was asked for rather than
/// about what we returned.
#[must_use]
pub fn machine() -> (QemuMachine, Arc<MockQemuHost>, Arc<MockSlotPlane>) {
    machine_with(MockPolicy::default(), config())
}

/// The same, with a named policy and configuration.
///
/// # Panics
/// If realize refuses — every caller of this helper expects it to succeed, and the tests
/// that expect a refusal call [`QemuMachine::realize`] directly so they can assert the
/// exact variant.
#[must_use]
pub fn machine_with(
    policy: MockPolicy,
    cfg: MachineConfig,
) -> (QemuMachine, Arc<MockQemuHost>, Arc<MockSlotPlane>) {
    let host = host_with(policy);
    let slots = slot_plane();
    let m = QemuMachine::realize(
        cfg,
        Arc::clone(&host) as Arc<_>,
        Arc::clone(&slots) as Arc<_>,
    )
    .expect("the device realizes");
    (m, host, slots)
}

/// Plain host RAM, as the listener reports it.
#[must_use]
pub fn ram_facts() -> SectionFacts {
    SectionFacts::plain_ram()
}

/// The five single-field departures from plain RAM, named, so a sweep can report which
/// one it was looking at.
#[must_use]
pub fn non_ram_shapes() -> Vec<(&'static str, SectionFacts)> {
    let mut out = Vec::new();
    let mut f = SectionFacts::plain_ram();
    f.is_ram = false;
    out.push(("a region that does not report itself RAM", f));
    let mut f = SectionFacts::plain_ram();
    f.is_ram_device = true;
    out.push(("a device-RAM region (a pass-through-mapped BAR)", f));
    let mut f = SectionFacts::plain_ram();
    f.is_rom_device = true;
    out.push((
        "a ROM-device region (writes go to the owner's callbacks)",
        f,
    ));
    let mut f = SectionFacts::plain_ram();
    f.readonly = true;
    out.push(("a read-only section", f));
    let mut f = SectionFacts::plain_ram();
    f.nonvolatile = true;
    out.push(("a non-volatile section", f));
    out
}
