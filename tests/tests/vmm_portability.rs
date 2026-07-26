//! ★ The hypervisor-portability contract of the `kayfabe-vmm` ports
//! (`l1_os_shell.md` §6.0/§6.3, `kayfabe-vmm` crate docs, decision #39).
//!
//! The claim under test is not "QEMU works" — no backend exists yet. It is the
//! **shape** claim the whole port rests on:
//!
//! > A second hypervisor backend costs exactly one adapter crate: no trait change,
//! > no core change.
//!
//! Three things that claim reduces to, each of which was free to pin today and
//! expensive to repair once two adapters exist:
//!
//! 1. **`Device` is a shared port, not an exclusive one.** Its entry points take
//!    `&self`, so an adapter may shard per-`Proc` exactly as `kayfabe_rt::SharedDevice`
//!    does. This file drives one `Device` from four threads *concurrently*; under the
//!    previous `&mut self` signature the test would not compile, which is the entire
//!    point — a backend whose bus dispatches MMIO through a `&self` callback would have
//!    had to wrap the device in a whole-device lock and throw the sharding away.
//! 2. **`Vmm` is implementable outside `kayfabe-mocks`, in seven groups.** `SecondVmm`
//!    below is a complete non-mock impl in ~60 lines. It is also the standing proof that
//!    the memory-lock primitive really did leave the trait (§6.7 item 5): if group 8
//!    came back, this file stops compiling.
//! 3. **The two backend-conditional refusals are refusals, not silence.** `IntxLevel`
//!    and an un-shared guest-RAM backing both fault with an exact
//!    [`VmmError::Unsupported`] — never a dropped interrupt, never a lazily-discovered
//!    `SIGBUS` at first guest DMA.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use core::ops::Range;
use core::time::Duration;

use kayfabe_util::Instant;
use kayfabe_vmm::{
    BarId, CoreEvent, CoreEventKind, Device, HostRegion, IrqSpec, Prot, RamHandle, SlotId,
    TrapMode, Vmm, VmmError,
};

// ---------------------------------------------------------------------------------
// A second `Vmm`, written outside the mocks crate — the "one adapter crate" claim in
// miniature. Deliberately shaped like the awkward backend rather than the easy one:
// it models a VM **launched without a shareable memory backing** and a target with no
// legacy interrupt controller.
// ---------------------------------------------------------------------------------

/// Refusal text is part of the contract here: an operator reading a log must be able to
/// tell a deployment mistake from a bug, because no code gate can catch a launch flag.
const NO_SHARED_BACKING: &str = "guest RAM was not launched with a shareable backing";
const NO_LEGACY_INTX: &str = "legacy INTx needs a userspace IOAPIC (x86_64 only)";

#[derive(Default)]
struct SecondVmm {
    ram: Vec<u8>,
    next_slot: u64,
    irqs: Vec<IrqSpec>,
    deferred: Vec<(Duration, CoreEvent)>,
    /// False = the VM was launched without a shareable guest-RAM backing.
    shared_backing: bool,
}

impl SecondVmm {
    fn new(shared_backing: bool) -> Self {
        SecondVmm {
            ram: vec![0; 0x1000],
            shared_backing,
            ..SecondVmm::default()
        }
    }

    fn slot(&mut self) -> SlotId {
        let id = SlotId(self.next_slot);
        self.next_slot += 1;
        id
    }
}

impl Vmm for SecondVmm {
    fn gpa_read(&mut self, gpa: u64, buf: &mut [u8]) -> Result<(), VmmError> {
        let start = usize::try_from(gpa).map_err(|_| VmmError::BadGpa { gpa })?;
        let end = start
            .checked_add(buf.len())
            .filter(|e| *e <= self.ram.len())
            .ok_or(VmmError::BadGpa { gpa })?;
        buf.copy_from_slice(&self.ram[start..end]);
        Ok(())
    }

    fn gpa_write(&mut self, gpa: u64, buf: &[u8]) -> Result<(), VmmError> {
        let start = usize::try_from(gpa).map_err(|_| VmmError::BadGpa { gpa })?;
        let end = start
            .checked_add(buf.len())
            .filter(|e| *e <= self.ram.len())
            .ok_or(VmmError::BadGpa { gpa })?;
        self.ram[start..end].copy_from_slice(buf);
        Ok(())
    }

    fn map_guest(
        &mut self,
        _gpa: u64,
        _len: u64,
        _backing: HostRegion,
        _prot: Prot,
    ) -> Result<SlotId, VmmError> {
        Ok(self.slot())
    }

    fn unmap_guest(&mut self, _slot: SlotId) -> Result<(), VmmError> {
        Ok(())
    }

    fn set_trap(
        &mut self,
        _bar: BarId,
        _range: Range<u64>,
        _mode: TrapMode,
    ) -> Result<(), VmmError> {
        Ok(())
    }

    fn raise_irq(&mut self, irq: IrqSpec) -> Result<(), VmmError> {
        // ★ The backend-conditional refusal, per `IrqSpec::IntxLevel`'s rustdoc.
        if matches!(irq, IrqSpec::IntxLevel(_)) {
            return Err(VmmError::Unsupported(NO_LEGACY_INTX));
        }
        self.irqs.push(irq);
        Ok(())
    }

    fn export_ram(&mut self, slice: Option<Range<u64>>) -> Result<RamHandle, VmmError> {
        // ★ The deployment precondition, refused at the FIRST export rather than at
        // first guest DMA (`Vmm::export_ram` rustdoc, `l1_os_shell.md` §4.4.1).
        if !self.shared_backing {
            return Err(VmmError::Unsupported(NO_SHARED_BACKING));
        }
        Ok(RamHandle {
            token: 1,
            covers: slice,
        })
    }

    fn defer(&mut self, after: Duration, event: CoreEvent) {
        self.deferred.push((after, event));
    }

    fn now(&self) -> Instant {
        Instant::ZERO
    }

    fn map_read_native(
        &mut self,
        _gpa: u64,
        _len: u64,
        _backing: HostRegion,
        _write_trap: Option<Range<u64>>,
    ) -> Result<SlotId, VmmError> {
        Ok(self.slot())
    }
}

// ---------------------------------------------------------------------------------
// A `Device` that shards, to prove the port permits it.
// ---------------------------------------------------------------------------------

/// One lock per "proc", plus a shared counter — the shape `kayfabe_rt::SharedDevice`
/// already has, expressed through the declared port. Nothing here needs `&mut self`.
struct ShardedDevice {
    shards: Vec<Mutex<u64>>,
    events: AtomicU64,
}

impl ShardedDevice {
    fn shard_of(&self, off: u64) -> &Mutex<u64> {
        &self.shards[(off as usize / 0x1000) % self.shards.len()]
    }
}

impl Device for ShardedDevice {
    fn mmio_read(&self, _vmm: &mut dyn Vmm, _bar: BarId, off: u64, _size: u8) -> u64 {
        *self.shard_of(off).lock().expect("uncontended in this test")
    }

    fn mmio_write(&self, _vmm: &mut dyn Vmm, _bar: BarId, off: u64, _size: u8, val: u64) {
        *self.shard_of(off).lock().expect("uncontended in this test") += val;
    }

    fn event(&self, _vmm: &mut dyn Vmm, _ev: CoreEvent) {
        self.events.fetch_add(1, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------------
// 1. `Device` is a SHARED port
// ---------------------------------------------------------------------------------

/// Four threads drive one `Device` concurrently, each owning its own `Vmm` (the trait
/// is `Send` and passed as `&mut dyn`, so it is per-thread by construction). Disjoint
/// shards are touched under disjoint locks — the per-`Proc` sharding the #14 fix
/// exists for, reached **through the port** rather than around it.
///
/// This test is a compile-time assertion as much as a runtime one: with
/// `Device::mmio_write(&mut self, …)` an `Arc<dyn Device>` cannot be written through at
/// all, so the file would not build.
#[test]
fn device_port_admits_per_shard_concurrency() {
    const THREADS: u64 = 4;
    const WRITES: u64 = 64;

    let dev: Arc<dyn Device> = Arc::new(ShardedDevice {
        shards: (0..THREADS).map(|_| Mutex::new(0)).collect(),
        events: AtomicU64::new(0),
    });

    thread::scope(|scope| {
        for t in 0..THREADS {
            let dev = Arc::clone(&dev);
            scope.spawn(move || {
                let mut vmm = SecondVmm::new(true);
                for _ in 0..WRITES {
                    dev.mmio_write(&mut vmm, BarId::Bar0, t * 0x1000, 4, 1);
                }
                dev.event(&mut vmm, CoreEvent::Deferred(CoreEventKind::DeferredReap));
            });
        }
    });

    let mut vmm = SecondVmm::new(true);
    for t in 0..THREADS {
        assert_eq!(
            dev.mmio_read(&mut vmm, BarId::Bar0, t * 0x1000, 4),
            WRITES,
            "shard {t} must have accumulated exactly its own thread's writes"
        );
    }
}

// ---------------------------------------------------------------------------------
// 2. The backend-conditional refusals — exact variants, never `is_err()`
// ---------------------------------------------------------------------------------

/// The only interrupt shape the core emits is `Msix(0)`, so a backend that cannot
/// express a legacy line is fully usable — but it must **say so** rather than drop the
/// injection, which would present as a permanently missing completion.
#[test]
fn legacy_intx_is_refused_by_exact_variant_while_msix_succeeds() {
    let mut vmm = SecondVmm::new(true);

    assert_eq!(vmm.raise_irq(IrqSpec::Msix(0)), Ok(()));
    assert_eq!(
        vmm.raise_irq(IrqSpec::IntxLevel(true)),
        Err(VmmError::Unsupported(NO_LEGACY_INTX))
    );
    assert_eq!(
        vmm.irqs,
        vec![IrqSpec::Msix(0)],
        "a refused INTx must leave no trace of a delivered interrupt"
    );
}

/// Guest RAM belongs to the VMM and is shareable with an isolate **only if the VM was
/// launched that way** (`--memory shared=on` / `memory-backend-*,share=on`). Identical
/// on every backend, so portability-neutral — but it is a *deployment* fact, which is
/// exactly the class of precondition no code gate can catch, so the refusal is the only
/// place it can be caught at all.
#[test]
fn export_ram_without_a_shared_backing_refuses_at_the_first_export() {
    let mut unshared = SecondVmm::new(false);
    assert_eq!(
        unshared.export_ram(Some(0..0x1000)),
        Err(VmmError::Unsupported(NO_SHARED_BACKING))
    );
    // The whole-RAM export is refused identically: the precondition is about the
    // BACKING, never about how much of it a caller asked for.
    assert_eq!(
        unshared.export_ram(None),
        Err(VmmError::Unsupported(NO_SHARED_BACKING))
    );

    let mut shared = SecondVmm::new(true);
    assert_eq!(
        shared.export_ram(Some(0x1000..0x2000)),
        Ok(RamHandle {
            token: 1,
            covers: Some(0x1000..0x2000),
        })
    );
}

// ---------------------------------------------------------------------------------
// 3. Bad-GPA addressing on a non-mock backend — the exact fault, not a panic
// ---------------------------------------------------------------------------------

/// `SecondVmm` is dense rather than sparse (unlike `MockVmm`), so it is the impl that
/// can actually run off the end. Pinned because `BadGpa` is the one `VmmError` variant
/// a real adapter produces on the hot path, and "checked, then exact" is the contract.
#[test]
fn out_of_range_gpa_faults_with_the_offending_address() {
    let mut vmm = SecondVmm::new(true);
    let mut buf = [0u8; 8];

    assert_eq!(vmm.gpa_read(0xFF8, &mut buf), Ok(()));
    assert_eq!(
        vmm.gpa_read(0xFFC, &mut buf),
        Err(VmmError::BadGpa { gpa: 0xFFC }),
        "a read straddling the end of guest RAM names the address it started at"
    );
    assert_eq!(
        vmm.gpa_write(u64::MAX, &buf),
        Err(VmmError::BadGpa { gpa: u64::MAX })
    );
}
