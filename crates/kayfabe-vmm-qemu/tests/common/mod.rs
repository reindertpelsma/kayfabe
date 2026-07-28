//! One device shape, shared by every test binary, so the whole suite describes **one**
//! device rather than four differently-configured ones.
//!
//! `#![allow(dead_code)]` because integration-test binaries each compile this module
//! separately and none of them uses all of it — the alternative is a per-file copy, which
//! is how two files end up describing two devices.

#![allow(dead_code)]

use std::sync::Arc;

use kayfabe_linux_raw::HostPageSize;
use kayfabe_vmm::{BarId, TrapMode};
use kayfabe_vmm_qemu::host::{BarPlacement, OverlaySpec, SectionFacts};
use kayfabe_vmm_qemu::mock_host::{MockPolicy, MockQemuHost};
use kayfabe_vmm_qemu::{MachineConfig, QemuMachine, TrapSpec, WindowSpec};

/// The register BAR: trapped, and where the read-native overlays live.
pub const BAR0_BASE: u64 = 0x8000_0000;
/// The aperture BAR: where the reservation lives.
pub const BAR1_BASE: u64 = 0x9000_0000;
/// Guest RAM, as the topology listener will report it. Deliberately far from both BARs.
pub const FOREIGN_RAM: u64 = 0x1000_0000;

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

/// Where the one read-native overlay starts.
#[must_use]
pub fn overlay_gpa() -> u64 {
    BAR0_BASE + 16 * page()
}

/// How long the overlay is.
#[must_use]
pub fn overlay_len() -> u64 {
    2 * page()
}

/// The overlay's write-trap sub-range, in guest-physical addresses.
#[must_use]
pub fn overlay_trap() -> core::ops::Range<u64> {
    overlay_gpa()..overlay_gpa() + page()
}

/// The device's realize-time configuration.
#[must_use]
pub fn config() -> MachineConfig {
    let p = page();
    MachineConfig {
        shareable_ram: true,
        bars: vec![
            BarPlacement {
                bar: BarId::Bar0,
                base: BAR0_BASE,
                len: 64 * p,
            },
            BarPlacement {
                bar: BarId::Bar1,
                base: BAR1_BASE,
                len: 64 * p,
            },
        ],
        windows: vec![WindowSpec {
            bar: BarId::Bar1,
            gpa: window_gpa(),
            len: window_len(),
        }],
        overlays: vec![OverlaySpec {
            bar: BarId::Bar0,
            gpa: overlay_gpa(),
            len: overlay_len(),
            write_trap: Some(overlay_trap()),
        }],
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

/// A realized machine and the mock host behind it, both kept, because half the assertions
/// in this suite are about what the host was asked for and not about what we returned.
#[must_use]
pub fn machine() -> (QemuMachine, Arc<MockQemuHost>) {
    machine_with(MockPolicy::default(), config())
}

/// The same, with a named policy and configuration.
///
/// # Panics
/// If realize refuses — every caller of this helper expects it to succeed, and the tests
/// that expect a refusal call [`QemuMachine::realize`] directly so they can assert the
/// exact variant.
#[must_use]
pub fn machine_with(policy: MockPolicy, cfg: MachineConfig) -> (QemuMachine, Arc<MockQemuHost>) {
    let host = Arc::new(MockQemuHost::with_policy(policy));
    let m = QemuMachine::realize(cfg, Arc::clone(&host) as Arc<_>).expect("the device realizes");
    (m, host)
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
