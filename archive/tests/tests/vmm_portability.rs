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

// ---------------------------------------------------------------------------------
// 4. ★★ The guest-RAM map — the one central check every backend's `gpa_read` /
//    `gpa_write` must make (`l1_os_shell.md` §6.1/§6.3/§10.1 item 6).
// ---------------------------------------------------------------------------------

use kayfabe_vmm::{GuestRamMap, RamRegionId, RamSpan, RegionKind};

const RAM_LO: RamRegionId = RamRegionId(1);
const RAM_HI: RamRegionId = RamRegionId(2);
const DEV: RamRegionId = RamRegionId(3);

/// A machine shaped like a real one: RAM below the 4 GiB hole, a device BAR **inside**
/// that RAM (a guest can and does re-program a BAR over RAM), a genuine hole, and a
/// second RAM region above 4 GiB.
///
/// ```text
///   0x0000_0000 .. 0x8000_0000   RAM_LO
///   0x8000_0000 .. 0x8001_0000   DEV      (a BAR mapped over RAM)
///   0x8001_0000 .. 0xC000_0000   RAM_LO   (the remainder — offsets must survive the punch)
///   0xC000_0000 .. 1_0000_0000   nothing
///   1_0000_0000 .. 2_0000_0000   RAM_HI
/// ```
fn machine() -> GuestRamMap {
    let mut m = GuestRamMap::new();
    m.declare(RAM_LO, RegionKind::Ram, 0, 0xC000_0000).unwrap();
    m.declare(DEV, RegionKind::Device, 0x8000_0000, 0x1_0000)
        .unwrap();
    m.declare(RAM_HI, RegionKind::Ram, 0x1_0000_0000, 0x1_0000_0000)
        .unwrap();
    m
}

/// ★★★ **The refusal that keeps the §6.3 ABBA unconstructible.** A GPA landing on a
/// device window is [`VmmError::NonRamGpa`] — and its near neighbour, a GPA landing on
/// nothing at all, is [`VmmError::BadGpa`]. The two must never start reporting as each
/// other (`testing_doctrine.md` §2 rule 3): only the first one means *the guest tried to
/// make us take a lock we do not own*.
///
/// Non-vacuity arm included per rule 2: the same call, one page lower, **succeeds**.
#[test]
fn a_device_gpa_is_refused_by_name_and_an_unbacked_one_is_a_different_name() {
    let m = machine();

    // Non-vacuity: RAM immediately below the BAR resolves, so the refusals below are
    // about WHAT is at the address and not about the resolver being broken.
    assert_eq!(
        m.resolve(0x7FFF_F000, 0x1000),
        Ok(RamSpan {
            region: RAM_LO,
            offset: 0x7FFF_F000,
            len: 0x1000
        })
    );

    assert_eq!(
        m.resolve(0x8000_0000, 4),
        Err(VmmError::NonRamGpa { gpa: 0x8000_0000 }),
        "a descriptor aimed at a device register window is refused BY NAME — this is \
         the guest-steerable lock inversion, not an ordinary read miss"
    );
    assert_eq!(
        m.resolve(0x8000_0FFC, 4),
        Err(VmmError::NonRamGpa { gpa: 0x8000_0FFC }),
        "…anywhere inside it, not only at its base"
    );
    assert_eq!(
        m.resolve(0xC000_0000, 4),
        Err(VmmError::BadGpa { gpa: 0xC000_0000 }),
        "the 4 GiB hole is backed by NOTHING — the near neighbour, and a different name"
    );
}

/// ★ A read that **starts in RAM and runs into the device window** is refused, naming
/// the boundary. This is the case a start-address-only check would serve: on a QEMU
/// backend `flatview_*_continue` walks region by region, so the second step is the one
/// that calls `prepare_mmio_access` — the lock is taken *after* the first bytes were
/// already a legal memcpy. `[src] v10.2.0 system/physmem.c:3289-3315`, `:3250`.
#[test]
fn a_read_straddling_ram_into_a_device_window_is_refused_at_the_boundary() {
    let m = machine();

    assert_eq!(
        m.resolve(0x7FFF_FFF8, 0x10),
        Err(VmmError::NonRamGpa { gpa: 0x8000_0000 }),
        "a straddling range names the FIRST byte that is not RAM, which is the boundary \
         — the byte a start-only check would never look at"
    );
    // And straddling out of RAM into a HOLE is the other name, at its own boundary.
    assert_eq!(
        m.resolve(0xBFFF_FFF8, 0x10),
        Err(VmmError::BadGpa { gpa: 0xC000_0000 })
    );
    // Non-vacuity: the same length, one byte lower, fits entirely in RAM and resolves.
    assert_eq!(
        m.resolve(0x7FFF_FFF0, 0x10),
        Ok(RamSpan {
            region: RAM_LO,
            offset: 0x7FFF_FFF0,
            len: 0x10
        })
    );
}

/// ★ Punching a device window out of a larger RAM declaration must carry the remainder's
/// **offset into its own backing** forward. Get this wrong and a legal read past the BAR
/// silently returns the wrong bytes — a memcpy from the wrong host page, with no error
/// anywhere. Asserted as an exact offset, not as "it resolved".
#[test]
fn the_remainder_after_a_punched_window_keeps_its_offset_into_its_backing() {
    let m = machine();

    assert_eq!(
        m.resolve(0x8001_0000, 0x1000),
        Ok(RamSpan {
            region: RAM_LO,
            offset: 0x8001_0000,
            len: 0x1000
        }),
        "the RAM above the punched-out BAR is still RAM_LO at its ORIGINAL offset"
    );
    assert_eq!(
        m.resolve(0x1_0000_0000, 8),
        Ok(RamSpan {
            region: RAM_HI,
            offset: 0,
            len: 8
        }),
        "a separately declared region starts at offset 0 in its own backing"
    );
}

/// ★ The two argued exemptions in [`GuestRamMap::resolve`]'s contract, pinned so they
/// cannot be widened by accident: a **zero-length** access still names an address, and a
/// range must lie in **one** region.
#[test]
fn the_resolvers_two_exemptions_are_exactly_these_two() {
    let m = machine();

    assert_eq!(
        m.resolve(0x8000_0000, 0),
        Err(VmmError::NonRamGpa { gpa: 0x8000_0000 }),
        "a zero-length access aimed at a device register is still refused — the rule \
         'every GPA we touch was proven RAM' is total, with no per-backend exception"
    );
    assert_eq!(
        m.resolve(0x1000, 0),
        Ok(RamSpan {
            region: RAM_LO,
            offset: 0x1000,
            len: 0
        }),
        "…and a zero-length access to RAM is served, with len 0"
    );

    // One region only: RAM_LO ends at the hole, RAM_HI starts after it. Even if they
    // were adjacent, one `RamSpan` names one backing.
    let mut adjacent = GuestRamMap::new();
    adjacent
        .declare(RAM_LO, RegionKind::Ram, 0, 0x1000)
        .unwrap();
    adjacent
        .declare(RAM_HI, RegionKind::Ram, 0x1000, 0x1000)
        .unwrap();
    assert_eq!(
        adjacent.resolve(0xFF8, 0x10),
        Err(VmmError::BadGpa { gpa: 0x1000 }),
        "a range that leaves its region is not backed AS A UNIT, even when the next \
         region is RAM and adjacent — one span, one backing"
    );
}

/// ★ A range that leaves the 64-bit space is un-formable and is refused as `BadGpa`
/// (*nothing is there*), never as `NonRamGpa` (*a device is there*) and never as a
/// wrap-around read. The `all_ram` map is the strongest place to assert it: with the
/// **entire** space declared RAM there is nothing else the refusal could be about.
#[test]
fn an_unformable_range_is_refused_even_when_every_declared_byte_is_ram() {
    let m = GuestRamMap::all_ram(RAM_LO);

    assert_eq!(
        m.resolve(0xFFFF_FFFF_FFFF_F000, 0x1000),
        Ok(RamSpan {
            region: RAM_LO,
            offset: 0xFFFF_FFFF_FFFF_F000,
            len: 0x1000
        }),
        "a range ending exactly at 2^64 is formable"
    );
    assert_eq!(
        m.resolve(0xFFFF_FFFF_FFFF_F000, 0x1001),
        Err(VmmError::BadGpa {
            gpa: 0xFFFF_FFFF_FFFF_F000
        }),
        "one byte more leaves the address space — refused, not wrapped"
    );
    assert_eq!(
        m.resolve(u64::MAX, 2),
        Err(VmmError::BadGpa { gpa: u64::MAX })
    );
}

/// ★ [`GuestRamMap::undeclare`] is the window-teardown side, and it must leave a HOLE
/// rather than a device window — the two are different refusals and mean different
/// things to whoever reads the fault.
#[test]
fn undeclaring_a_window_leaves_a_hole_not_a_device() {
    let mut m = machine();
    assert_eq!(
        m.resolve(0x1_0000_0000, 8).map(|s| s.region),
        Ok(RAM_HI),
        "non-vacuity: it resolved before the teardown"
    );
    m.undeclare(0x1_0000_0000, 0x1_0000_0000);
    assert_eq!(
        m.resolve(0x1_0000_0000, 8),
        Err(VmmError::BadGpa { gpa: 0x1_0000_0000 }),
        "a torn-down window is backed by nothing — never silently still RAM"
    );
}
