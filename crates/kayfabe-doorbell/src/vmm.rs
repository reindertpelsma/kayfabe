//! The VMM-agnostic seam — §3's *"`kf-core`: VMM-agnostic device interface"*.
//!
//! ## ⊘ No QEMU anywhere below this line
//!
//! `[owner, 2026-09-21]` *"keep an agnostic interface for VMM … in the rest of the code base then
//! no QEMU specific things for v3 (apart from tests)."*
//!
//! ## ★★★ VALIDATED AGAINST CLOUD HYPERVISOR'S REAL TRAITS, not a guess
//!
//! Checked against a clone of `cloud-hypervisor` at `HEAD`:
//!
//! | CH | signature |
//! |---|---|
//! | `vm-device/src/bus.rs:32` | `trait BusDeviceSync: Send + Sync` |
//! | `:34` | `fn read(&self, base: u64, offset: u64, data: &mut [u8])` |
//! | `:36` | `fn write(&self, base: u64, offset: u64, data: &[u8]) -> Option<Arc<Barrier>>` |
//! | `vm-device/src/interrupt/mod.rs:176` | `fn trigger(&self, index: InterruptIndex) -> Result<()>` |
//! | `hypervisor/src/vm.rs:521-525` | `trait VmOps { guest_mem_read/write, mmio_read/write }` |
//!
//! ★★★ **The finding that shapes this file: CH's `BusDeviceSync` takes `&self`.** It assumes the
//! device is internally synchronised and callable **concurrently**, with no global lock. That is
//! *exactly* v3's model — [`crate::plane::Plane::trap_write`] already takes `&self` and is
//! lock-free by construction (§3, §48).
//!
//! ⇒ **v3 is more naturally Cloud-Hypervisor-shaped than QEMU-shaped.** QEMU takes its global lock
//! on every MMIO dispatch and needs the exit-site patch §3 describes to get out of it; CH needs no
//! patch at all for that property. ⚠ That inverts the usual assumption that QEMU is the easy
//! target, and it is a reason to keep this seam honest rather than let a BQL-shaped habit leak in.
//!
//! ## The three differences the seam must absorb
//!
//! | | QEMU | Cloud Hypervisor |
//! |---|---|---|
//! | payload | value + size | `&[u8]` slice |
//! | address | offset within the region | `base` **and** `offset` |
//! | write result | `void` | `Option<Arc<Barrier>>` — a barrier the VMM waits on |
//!
//! ⊘ The barrier is the one that cannot be bolted on later: a return type must exist from the
//! start, or every call site has to change. [`MmioOutcome`] carries it, and QEMU simply drops it.

use crate::readtrap::ReadPolicy;
use crate::trap::Action;

/// Which BAR a guest access landed in. ⊘ `base`/`offset` is CH's shape; QEMU gives only the
/// offset, so the adapter supplies the BAR it registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Bar(pub u8);

/// What the VMM must do after a guest MMIO write.
///
/// ⊘ Deliberately not `()`: Cloud Hypervisor's `BusDeviceSync::write` returns
/// `Option<Arc<Barrier>>`, and a seam that returned nothing could never express it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MmioOutcome {
    /// Nothing further. ⇒ CH returns `None`; QEMU returns.
    Done,
    /// Wake one worker on the worker eventfd.
    WakeWorker,
    /// Wake the register drainer — ⊘ a *different* eventfd (§5.3).
    WakeDrainer,
    /// Ring a real host doorbell inline, on this vCPU. §5: no queue, no wake, no lock.
    RingHostInline { host_token: u32 },
    /// ⊘ The privileged ring is full: raise a guest-visible fault. §5.4, never wait.
    PoisonDevice,
    /// ★ The VMM must not return until the effect is visible.
    /// ⇒ CH: hand back an `Arc<Barrier>`. ⊘ QEMU: it has no equivalent and does not need one,
    /// because its dispatch is already serialised — dropping this is correct there, not a gap.
    Barrier,
}

impl From<Action> for MmioOutcome {
    fn from(a: Action) -> MmioOutcome {
        match a {
            Action::None => MmioOutcome::Done,
            Action::WakeWorker => MmioOutcome::WakeWorker,
            Action::WakeDrainer => MmioOutcome::WakeDrainer,
            Action::RingHostInline { host_token } => MmioOutcome::RingHostInline { host_token },
            Action::PoisonDevice => MmioOutcome::PoisonDevice,
            // ⊘ A refusal is complete at the trap and owes the VMM nothing.
            Action::RefusedByName => MmioOutcome::Done,
        }
    }
}

/// What the device model asks of its host VMM.
///
/// ⚠ **Every method here exists in both QEMU and Cloud Hypervisor.** A verb that only one of them
/// can provide belongs in that adapter, not in this trait — otherwise the seam is QEMU's shape
/// wearing an agnostic name.
pub trait VmmOps: Send + Sync {
    /// Read guest RAM. ⊘ CH: `VmOps::guest_mem_read`. QEMU: `address_space_read`.
    fn guest_read(&self, gpa: u64, buf: &mut [u8]) -> Result<(), VmmError>;
    /// Write guest RAM.
    fn guest_write(&self, gpa: u64, buf: &[u8]) -> Result<(), VmmError>;

    /// Install a host page as guest-physical memory — the doorbell page, the read shadow.
    /// ⊘ §6.2: this is the **only** memslot the design installs, and it is the one place a vCPU
    /// may block.
    fn install_memslot(&self, gpa: u64, len: u64, host: HostMapping) -> Result<SlotId, VmmError>;
    fn remove_memslot(&self, slot: SlotId) -> Result<(), VmmError>;

    /// Deliver an interrupt. ⊘ CH: `InterruptSourceGroup::trigger(index)`. QEMU: `msix_notify`.
    /// ⚠ §8: registration is **per engine**, so `index` names an engine, never a channel.
    fn raise_irq(&self, index: u32) -> Result<(), VmmError>;

    /// ★ Workers and the drainer park on these; the trap only ever *says* one is owed.
    fn signal_worker(&self);
    fn signal_drainer(&self);
}

/// An opaque handle to a host mapping the adapter owns.
/// ⊘ §hostverb: **not an address.** Safe code holds this; the pointer lives behind the adapter's
/// own `unsafe`, so a bug here cannot become a bad dereference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostMapping(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmmError {
    BadGpa,
    NoSlots,
    Refused,
}

/// The device model, as a VMM sees it. ⊘ `&self` throughout, because CH requires it and because
/// §3's trap path is lock-free anyway.
pub trait GpuDevice: Send + Sync {
    /// ⚠ **Only writes trap.** §5: reads are served from DRAM the guest reads directly, with no
    /// exit — except the ~524 pages the read-trap allowlist names, which reach [`Self::mmio_read`].
    fn mmio_write(&self, bar: Bar, offset: u64, data: &[u8]) -> MmioOutcome;

    /// Serve a read that the allowlist says must trap.
    /// ⊘ Returns the policy as well as the value so the adapter can tell *"served from shadow"*
    /// from *"we were asked about a page we do not trap"* — which is a bug in the adapter's
    /// region registration, not a guest error.
    fn mmio_read(&self, bar: Bar, offset: u64, data: &mut [u8]) -> ReadPolicy;
}

/// ⊘ CH hands a byte slice; QEMU hands a value and a size. This is the one conversion the seam
/// owns, so neither adapter re-derives it.
pub fn value_of(data: &[u8]) -> u64 {
    let mut v = 0u64;
    for (i, b) in data.iter().take(8).enumerate() {
        v |= (*b as u64) << (8 * i);
    }
    v
}

pub fn store_value(data: &mut [u8], v: u64) {
    for (i, b) in data.iter_mut().take(8).enumerate() {
        *b = (v >> (8 * i)) as u8;
    }
}
