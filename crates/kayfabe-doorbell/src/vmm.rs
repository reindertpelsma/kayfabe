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

    /// Install a host mapping as guest-physical memory.
    ///
    /// ⊘⊘⊘ **`readonly` IS NOT A DETAIL — IT IS HOW THE WHOLE TRAP POLICY IS EXPRESSED.** A
    /// read-only memslot serves **reads from DRAM with no exit** and sends **writes to
    /// [`GpuDevice::mmio_write`]**, which is exactly `Disposition::ShadowWriteTrapped` — the
    /// default for almost all of BAR0, and what *"no read traps"* means in implementation terms.
    /// `readonly = false` is `Disposition::PlainRam` (PRAMIN, BAR1, BAR2): no exit in either
    /// direction.
    ///
    /// ⊘ CH: `KVM_MEM_READONLY` on the user memory region. QEMU: `memory_region_init_rom_device`,
    /// which the C artifact already used for page `0x110000` and which booted a real driver.
    /// ⚠ **QEMU trap, measured (`m582`):** the BAR must be a **container** `MemoryRegion` with
    /// these added as subregions. A leaf `memory_region_init_io` with overlays **silently fails to
    /// reach KVM** — everything looks configured while reads keep exiting.
    ///
    /// ★ A region with **no** memslot at all is `Disposition::Hole`: both reads and writes exit.
    /// It is expressed by *not calling this*, which is the point — see [`crate::memmap`].
    fn install_memslot(
        &self,
        gpa: u64,
        len: u64,
        host: HostMapping,
        readonly: bool,
    ) -> Result<SlotId, VmmError>;
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
    /// ⚠ **On the product target, only writes trap.** Reads are served from DRAM the guest reads
    /// directly, with no exit at all — see [`crate::memmap`].
    fn mmio_write(&self, bar: Bar, offset: u64, data: &[u8]) -> MmioOutcome;

    /// ⊘⊘⊘ **THE READ VERB, AND ITS NAME IS ITS CONTRACT.** Called **only** for a page the map
    /// marks [`crate::memmap::Disposition::Hole`] — never as a general read path.
    ///
    /// ★ There is exactly one reason a hole exists, and it is not performance: a register whose
    /// **read has a side effect the guest verifies**. Today that is the **falcon PIO
    /// auto-increment data port** (`EMEMD`/`DMEMD`) — armed once with `AINCR`, each read advances
    /// a hardware cursor, and ogkm then *asserts the cursor moved*, so no shadow can satisfy it.
    ///
    /// ⇒ **On Turing, Ampere and Ada this method is DEAD CODE**, because
    /// [`crate::memmap::holes_for`] is empty for them — asserted by
    /// `the_current_product_target_has_no_read_exits_at_all`. It exists so Hopper and Blackwell
    /// can boot at all, and the map — not this trait — is what decides whether it is ever reached.
    ///
    /// ⚠ The default **refuses**: an adapter that has not thought about holes returns zeros rather
    /// than inventing a read path, and a family with no holes never calls it.
    fn mmio_read_hole(&self, _bar: Bar, _offset: u64, data: &mut [u8]) -> MmioOutcome {
        data.fill(0);
        MmioOutcome::Done
    }
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
