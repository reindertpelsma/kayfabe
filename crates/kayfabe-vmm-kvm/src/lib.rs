//! # `kayfabe-vmm-kvm` — the **first real `Vmm`**, over a KVM-direct harness
//!
//! `l1_os_shell.md` §10, stage **M2-c**, and its decision box:
//!
//! > **★★ M2-c builds against the KVM-direct harness — NOT QEMU.** … The harness has **no
//! > VMM-global lock, no serialized dispatch loop and no foreign locks**, so M2-c can get
//! > the memory plane right — plan/execute/commit, two-tier coarse/fine, the
//! > drop-the-lock discipline — **without simultaneously fighting lock ordering imposed by
//! > someone else's design**. … And it is the strongest portability guarantee available to
//! > us: **the first real `Vmm` implementation being non-QEMU** proves the port is
//! > VMM-agnostic *by construction*.
//!
//! Everything here is a **real** operation: a real `/dev/kvm` descriptor, a real VM, real
//! `KVM_SET_USER_MEMORY_REGION` memslots, real `mmap`ed windows, real `MAP_FIXED`
//! placements, a real sealed `memfd` behind guest RAM. Not one of them is a
//! `BTreeMap::insert`, and that difference is the whole point of the stage — see
//! [§ What this stage is for].
//!
//! [§ What this stage is for]: #what-this-stage-is-for
//!
//! ## What this stage is for
//!
//! §6.1 classifies every `Vmm` method as in-lock legal or illegal, and §6.2 says the
//! assert *"will panic on the existing code"*. `l1_os_shell.md`'s gap **O1** states the
//! problem the other way round and more usefully: *nothing in the suite can distinguish a
//! `memcpy` from a `KVM_SET_USER_MEMORY_REGION`.* `MockVmm::map_guest` is an insert and
//! **cannot block**; this one is a syscall that waits for an SRCU grace period across
//! every vCPU (§6.7: 230–460 µs p50, 1–4 ms tail, measured). So R1 — *no blocking call
//! under any lock* — gets its first real test here, and the assert moves to the syscall.
//!
//! ## ★★ The two locks, and why there are exactly two
//!
//! This is the design decision the milestone turned on, and it is a **consequence** of R1
//! rather than a convenience.
//!
//! | lock | who takes it | may a syscall run under it? |
//! |---|---|---|
//! | **the view** (leaf) | `gpa_read`/`gpa_write`, entered with a **ranked** lock held | **never** |
//! | **the installer** | the memory-plane ops, entered lock-free | **never** — see below |
//!
//! The naive adapter is one mutex around everything, which is exactly the shape the mock
//! harness's `SharedVmm` has. With a mock behind it that is fine: every critical section
//! is a `BTreeMap` operation. With *this* backend it is a live inversion — a thread
//! holding the core's rank-0 lock calls `gpa_read`, blocks on the adapter's mutex, and
//! waits out a peer's memslot ioctl. **§6.3's ABBA, rebuilt out of our own parts, with
//! every existing assert green**, because `kayfabe_util::lockwitness` only knows about
//! *ranked* locks and an adapter's lock has no rank (§12.43 residual 2, now closed by
//! [`leaf`]).
//!
//! So the plane is split by *what the critical section costs*: the view lock's is a
//! `BTreeMap` probe and an `Arc` clone, and the copy itself runs **outside** it, held
//! alive by that `Arc`. §12.43 §3 describes the leaf lock's critical section as *"a
//! bounded memcpy"*; it does not have to be one, and making it a lookup instead is
//! strictly better — a 4 KiB pushbuffer read no longer serialises against a 4 KiB
//! pushbuffer read on another thread.
//!
//! And the installer lock does **not** wrap the syscalls either, which is §6.2's
//! plan/execute/commit applied to the memory plane:
//!
//! > **plan** (installer held — pure bookkeeping) → **execute** (every lock dropped, the
//! > `mmap`/ioctl happens here) → **commit** (installer re-taken, **R5 re-validated**
//! > against the window's *presence*, then the view lock updated).
//!
//! `assert_leaf_free` sits at the top of the execute phase of every op, so *"the syscall
//! ran with no lock at all"* is a mechanism, not a comment.
//!
//! ## ★★ The `Arc` is a *release* handle too — why removed windows are retired, not dropped
//!
//! The paragraph above is the whole argument for the copy running outside the view lock,
//! and it has a second half §14.8 F7(b) never wrote down. The `Arc` an accessor holds keeps
//! the mapping alive — and it also **owns** it: if that clone is the last one, the accessor
//! is the thread that runs `munmap`. `gpa_read`/`gpa_write` are in-lock **legal**, entered
//! with one of the core's *ranked* locks held, so that release is an R1 violation:
//!
//! ```text
//! R1 no-blocking-under-lock violation (l1_concurrency.md §3.3):
//!   munmap (dropping a guest-physical window) while holding rank(s) [0]
//! ```
//!
//! Neither witness could have predicted it. The ranked one is the one that fires, but only
//! on the unlucky interleaving; the leaf one sees nothing, correctly, because no adapter
//! lock is held and no critical section is any longer than it looks. **The defect is in
//! ownership, not in locking**, so the fix is an ownership one:
//! [`KvmMachine::remove_window`] hands the mapping to [`Plane::retire`] instead of dropping
//! it, the machine keeps one reference no accessor can take away, and the release happens
//! in [`Plane::collect_retired`] at a door that has just been proved lock-free on both
//! halves. An accessor's clone is then never the last one — structurally, not by timing.
//! [`AuditReport::window_releases_deferred`] and [`AuditReport::window_mappings_released`]
//! are the two instruments: the first is the non-vacuity half (the removal really did land
//! on a window someone was reading), the second says the address space came back.
//!
//! ## ★ The two tiers (§6.7), which the trait's one method group covers
//!
//! > **One memslot per window (or per arena grant) — never one per published object.**
//!
//! - **coarse** — [`KvmMachine::install_ram_window`] / [`KvmMachine::remove_window`] and
//!   [`Vmm::map_read_native`]: an `mmap` plus a `KVM_SET_USER_MEMORY_REGION`. Frequency:
//!   per window, i.e. per proc create/destroy.
//! - **fine** — [`Vmm::map_guest`] / [`Vmm::unmap_guest`]: a `MAP_FIXED` placement inside
//!   an already-installed window. **No KVM ioctl at all.** Frequency: per publication.
//!
//! Both are syscalls and both are in-lock illegal; what differs is the cost by orders of
//! magnitude, which is why §6.7 corrects §6.2's cost model while keeping its shape. The
//! **memslot-frequency gate** (§9.3) is [`AuditReport::memslot_installs`] against
//! [`AuditReport::placements_made`]: installs must scale with windows, never with
//! publications.
//!
//! ## ★ Where `GuestRamMap` comes from here — installs, never queries
//!
//! `kayfabe_vmm::GuestRamMap`'s rustdoc says the map *"is maintained by installs we
//! perform … it is never a query issued per access"*. In this backend that is not a
//! convention but the physical truth, and the two halves check each other:
//!
//! - a [`RegionKind::Ram`] region **is** a live memslot — the guest's access lands in our
//!   window with no exit;
//! - a [`RegionKind::Device`] region **is the absence of one** — KVM has nothing for that
//!   guest-physical address and exits to userspace.
//!
//! [`KvmMachine::audit`] asserts they agree, which is a consistency check the mock cannot
//! even express.
//!
//! ## What this crate deliberately does NOT do
//!
//! Named, because an honest absence is a design statement:
//!
//! - **No vCPUs.** No `KVM_CREATE_VCPU`, no `KVM_RUN`, no MMIO exit dispatch. The trap
//!   *registration* is real and its consistency with the memslot map is asserted; the
//!   *dispatch* is M2-d. §10's "one variable, not two", one level down.
//! - **No interrupt controller.** [`Vmm::raise_irq`] writes a real notify descriptor —
//!   the irqfd-shaped discharge §6.1 makes its in-lock legality conditional on — but
//!   nothing is wired to receive it until there is a vCPU to receive it.
//! - **No isolate.** [`Vmm::export_ram`] mints a real duplicated, sealed descriptor; the
//!   process that maps it is M2-d.
//! - **A virtual clock.** [`Vmm::now`] is advanced explicitly by
//!   [`KvmMachine::advance`], sharing `kayfabe_vmm::DeferQueue` with the mock (§6.4).
//!   A wall clock here would buy nothing and would make every suite that composes this
//!   backend non-deterministic, which §8.3 forbids.
//!
//! ## ★★★ The seam map: what this crate shares with `kayfabe-vmm-qemu`, measured
//!
//! **Reported, not acted on.** Hoisting is an owner decision and a large refactor; what is
//! recorded here is the measurement, so the decision is made against numbers. Taken over
//! the two `lib.rs` files, comparing same-named items line by line with comments and blanks
//! stripped (2026-07-29):
//!
//! | item | qemu | kvm | identical | note |
//! |---|---|---|---|---|
//! | `map_guest` | 88 | 92 | **77 (84%)** | the plan/execute/commit dance, `Prot`, R5 revalidate |
//! | `export_ram` | 33 | 33 | **28 (85%)** | identical but for the refusal constant |
//! | `unmap_guest` | 39 | 34 | 25 (64%) | |
//! | `AuditReport::report` | 35 | 32 | 22 (63%) | field-for-field, watermarks and all |
//! | `map_read_native` | 25 | 25 | **19 (76%)** | |
//! | `collect_retired` | 18 | 18 | **17 (94%)** | |
//! | `register_backing` | 17 | 17 | **17 (100%)** | byte-identical |
//! | `retire` | 9 | 12 | 9 (75%) | #57's ownership fix, twice |
//! | `Plane::view` / `installer` / `about_to_syscall` | 31 | 31 | 22 (71%) | the lock discipline |
//! | `Audit::{new,bump,note_ranked}` | 20 | 20 | **18 (90%)** | |
//! | `dup_owned`, `advance`, `defer`, `now`, `resolve_region`, `page_size`, `audit` | 28 | 28 | **28 (100%)** | byte-identical |
//!
//! **405 identical lines across same-named items**, of which roughly 120 are byte-identical
//! whole functions. `map_guest` alone is 77.
//!
//! ★ The `map_guest` row is now an **under**-count and deliberately not re-measured: #89
//! (2026-08-01) deleted this crate's per-window generation, so the two commit blocks are
//! the same code where they used to differ. The table is a dated snapshot, and a row
//! re-measured by a different method on a different day is worse than a stale one that
//! says when it was taken.
//!
//! ### Genuinely hypervisor-specific — a third backend must write these itself
//!
//! - *QEMU*: the `QemuHost` trait and the topology listener (`region_add`/`region_del`/
//!   `classify`), the BAR latch and its move tripwire, `realize`/`unrealize` lifecycle and
//!   `assert_live`, the migration blocker and `ram_block_discard_disable`, the `SlotPlane`
//!   seam, and finding 3 (our reservations are served from *our* map and stay `Device` in
//!   the region map).
//! - *KVM-direct*: `Kvm`/`KvmVm` ownership, `vcpu.rs` and exit classification,
//!   `GuestRamMap::declare`/`undeclare` (here a RAM region **is** a memslot), the notifier.
//! - *Both, but oppositely*: slot-number **direction** — see [`slotnum`].
//!
//! ### The shared core a `kayfabe-vmm-common` would hold
//!
//! `map_guest`/`unmap_guest`'s phase structure, `retire`/`collect_retired`, the whole
//! `Audit`/`AuditReport` ledger and its non-vacuity discipline, the `view`/`installer`/
//! `about_to_syscall` lock protocol, `export_ram`/`register_backing`, the virtual clock,
//! `host_refused`/`dup_owned`, the tier/span algebra (`kayfabe_vmm_qemu::slots::spans`
//! versus this crate's `memslot_spans` — the same function, written twice), the trap-mode
//! rule (`trap_is_physical` here has no counterpart yet: this crate still inlines it), and
//! slot-number allocation behind one release contract.
//!
//! ### What a Cloud Hypervisor port would be forced to fork **today**
//!
//! All of the second list. CH is a Rust VMM that owns its own `KvmVm`, so it would copy
//! `kayfabe-vmm-kvm` — and thereby inherit a third copy of `map_guest` (92 lines), a third
//! `Audit` (32), a third `retire`, a third clock, and a third slot allocator. It would also
//! inherit whichever of the two crates' **bug fixes had not yet been ported at the moment
//! it copied**, which is the real cost: the two `set_trap` implementations are 45% identical
//! and disagreed about whether anybody reads the registration for a month.

#![allow(clippy::module_name_repetitions)]

pub mod leaf;
pub mod slotnum;
pub mod vcpu;

use core::ops::Range;
use core::time::Duration;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use kayfabe_linux_raw::{
    Backing, GuestWindow, HostOffset, HostPageSize, Kvm, KvmMemslot, KvmVm, Notifier, RawError,
    SharedRam, geometry,
};
use kayfabe_util::Instant;
use kayfabe_vmm::{
    BarId, CoreEvent, DeferQueue, GuestRamMap, HostRegion, IrqSpec, Prot, RamHandle, RamRegionId,
    RegionKind, SlotId, TrapMode, Vmm, VmmError,
};

use crate::slotnum::MemslotNumbers;

/// Refusal text for the deployment fact no code gate can observe (§4.4.1).
const NO_SHARED_BACKING: &str =
    "guest RAM was not created with a shareable backing; an isolate cannot map it";

/// ★★ Refusal text for a read-native overlay whose write-trap span has no device behind
/// it. Named, because it is the one refusal that prevents a **silently dropped guest
/// store** rather than a failed setup.
const TRAP_SPAN_OUTSIDE_EVERY_BAR: &str = "a read-native write-trap span (rounded to whole host pages) that is not inside any \
     realized BAR — the guest's store would exit, classify as unclaimed, and be dropped";

/// Refusal text for a write-only trap registration with no read-only memslot under it.
const WRITE_TRAP_WITHOUT_A_READ_ONLY_SLOT: &str = "a write-only trap over a range no read-native overlay traps — reads are served from \
     RAM only if a memslot exists, and writes exit only if that memslot is READ-ONLY; \
     install the overlay with `map_read_native` first";

// =====================================================================================
// Configuration
// =====================================================================================

/// One emulated BAR's guest-physical placement, as the machine is realized with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BarPlacement {
    /// Which BAR.
    pub bar: BarId,
    /// Guest-physical base the guest programmed it to.
    pub base: u64,
    /// Length in bytes.
    pub len: u64,
}

/// What to realize.
#[derive(Debug, Clone)]
pub struct MachineConfig {
    /// Whether guest RAM is created from a **shareable** backing. `false` models a VM
    /// launched without `share=on`: everything still works until the first
    /// [`Vmm::export_ram`], which then refuses loudly rather than handing back
    /// copy-on-write pages (§4.4.1).
    pub shareable_ram: bool,
    /// The BARs, whose ranges are declared [`RegionKind::Device`] at realize.
    pub bars: Vec<BarPlacement>,
}

impl Default for MachineConfig {
    fn default() -> Self {
        MachineConfig {
            shareable_ram: true,
            bars: Vec::new(),
        }
    }
}

// =====================================================================================
// The audit — the conservation ledger and the R1 witnesses
// =====================================================================================

/// What the adapter observed about itself. Every field is an *instrument*, and every
/// instrument here has a documented non-vacuity condition, because two of this project's
/// last three rounds found a witness that could not fail (`l1_concurrency.md` §12.43 N12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditReport {
    /// Windows currently mapped.
    pub live_windows: u64,
    /// Memslots currently installed in the kernel.
    pub live_memslots: u64,
    /// `MAP_FIXED` placements currently live inside windows.
    pub live_placements: u64,
    /// Total bytes of window address space currently mapped.
    pub window_bytes: u64,
    /// High-water marks — the **non-vacuity** halves of the three counters above.
    pub peak_windows: u64,
    /// High-water mark of live memslots.
    pub peak_memslots: u64,
    /// High-water mark of live placements.
    pub peak_placements: u64,
    /// ★ Cumulative `KVM_SET_USER_MEMORY_REGION` **installs** — the §9.3 memslot-frequency
    /// gate's numerator. Must scale with windows, never with publications.
    pub memslot_installs: u64,
    /// ★ Cumulative `MAP_FIXED` placements — the gate's denominator.
    pub placements_made: u64,
    /// ★★ Memslot numbers re-issued from the allocator's free list — the **non-vacuity**
    /// half of [`crate::slotnum`]'s adjudication. A suite where this stays zero would pass
    /// identically against the never-recycling allocator that was replaced, so a churn test
    /// that does not assert it is a churn test that proves nothing.
    pub slot_numbers_recycled: u64,
    /// `(min, max)` of `lockwitness::held_depth()` observed at `gpa_read`/`gpa_write`.
    /// `max == 0` means the in-lock hazard was never exercised; `min == u32::MAX` means
    /// no access happened at all. **Both halves are load-bearing.**
    pub accessor_ranked_depth: (u32, u32),
    /// `(min, max)` of `lockwitness::held_depth()` observed at a syscall-shaped method.
    /// R1 says this must be `(0, 0)` — and `min == u32::MAX` means no syscall ran, which
    /// is the vacuous case the max alone cannot distinguish.
    pub syscall_ranked_depth: (u32, u32),
    /// Max adapter leaf depth observed **at the copy** in `gpa_read`/`gpa_write`. Must be
    /// `0`: the memcpy runs outside the view lock.
    pub copy_leaf_depth_max: u32,
    /// Max adapter leaf depth observed **while the view lock was held**. Must be `≥ 1`, or
    /// the leaf witness is measuring nothing.
    pub view_leaf_depth_max: u32,
    /// Guest-physical accesses served.
    pub accesses_served: u64,
    /// Guest-physical accesses refused by the region map.
    pub accesses_refused: u64,
    /// Memory-plane ops that failed **in the execute phase** — a real host refusal.
    pub host_refusals: u64,
    /// ★ Memory-plane ops that reached **commit** and were undone by the R5 re-validation
    /// because their window died in the gap.
    pub r5_revalidation_failures: u64,
    /// ★ MMIO exits a vCPU took, device-wide — the **non-vacuity** half of the two
    /// counters below. Zero means no guest ran, which is the vacuous case in which
    /// `ram_declared_exits == 0` means nothing at all.
    pub guest_exits: u64,
    /// ★★ **Residual N13's instrument.** Guest accesses that exited to userspace at a
    /// guest-physical address the region map still calls RAM. On KVM a RAM region *is* a
    /// live memslot, so this is the map and the kernel disagreeing — the state a teardown
    /// that deletes the memslot before undeclaring the region passes through. **Must be
    /// zero**; see [`vcpu::ExitClass::RamDeclared`].
    pub ram_declared_exits: u64,
    /// The first such address, so a failure names one rather than merely counting.
    pub first_ram_declared_exit: Option<u64>,
    /// ★★ Window removals that found an accessor **still holding the mapping**, and so
    /// handed its release to the machine instead of performing it inline
    /// ([`Plane::retire`]). This is the non-vacuity half of the R1 fix: with no live
    /// accessor the deferral never happens, and a test that never reaches this state
    /// proves nothing about who performs the `munmap`.
    pub window_releases_deferred: u64,
    /// ★ Retired window mappings actually released — one `munmap` each, always by a
    /// thread that is lock-free by contract. At quiescence this must equal the number of
    /// windows removed, or a mapping is parked forever.
    pub window_mappings_released: u64,
}

#[derive(Debug, Default)]
struct Audit {
    live_windows: AtomicU64,
    live_memslots: AtomicU64,
    live_placements: AtomicU64,
    window_bytes: AtomicU64,
    peak_windows: AtomicU64,
    peak_memslots: AtomicU64,
    peak_placements: AtomicU64,
    memslot_installs: AtomicU64,
    placements_made: AtomicU64,
    slot_numbers_recycled: AtomicU64,
    accessor_depth_min: AtomicU32,
    accessor_depth_max: AtomicU32,
    syscall_depth_min: AtomicU32,
    syscall_depth_max: AtomicU32,
    copy_leaf_max: AtomicU32,
    view_leaf_max: AtomicU32,
    accesses_served: AtomicU64,
    accesses_refused: AtomicU64,
    host_refusals: AtomicU64,
    r5_failures: AtomicU64,
    guest_exits: AtomicU64,
    ram_declared_exits: AtomicU64,
    first_ram_declared_exit: AtomicU64,
    window_releases_deferred: AtomicU64,
    window_mappings_released: AtomicU64,
}

impl Audit {
    /// ★ Hand-written, and `l1_concurrency.md` §12.43's N12 is why: a DERIVED `Default`
    /// gives both `min` watermarks `0`, which makes the lower half of every span assertion
    /// vacuously true — a witness reporting a **constant** would still pass. The minima
    /// start at `u32::MAX` so "never observed" is distinguishable from "observed zero".
    fn new() -> Self {
        Audit {
            accessor_depth_min: AtomicU32::new(u32::MAX),
            syscall_depth_min: AtomicU32::new(u32::MAX),
            // Same argument one axis over: `0` is a legitimate guest-physical address, so
            // "no contradiction was ever seen" needs a value the guest cannot produce.
            first_ram_declared_exit: AtomicU64::new(u64::MAX),
            ..Audit::default()
        }
    }

    fn bump(count: &AtomicU64, peak: &AtomicU64, delta: i64) {
        let now = if delta >= 0 {
            count.fetch_add(delta as u64, Ordering::SeqCst) + delta as u64
        } else {
            count.fetch_sub(delta.unsigned_abs(), Ordering::SeqCst) - delta.unsigned_abs()
        };
        peak.fetch_max(now, Ordering::SeqCst);
    }

    fn note_ranked(min: &AtomicU32, max: &AtomicU32, d: u32) {
        min.fetch_min(d, Ordering::SeqCst);
        max.fetch_max(d, Ordering::SeqCst);
    }

    fn report(&self) -> AuditReport {
        let g = |a: &AtomicU64| a.load(Ordering::SeqCst);
        let h = |a: &AtomicU32| a.load(Ordering::SeqCst);
        AuditReport {
            live_windows: g(&self.live_windows),
            live_memslots: g(&self.live_memslots),
            live_placements: g(&self.live_placements),
            window_bytes: g(&self.window_bytes),
            peak_windows: g(&self.peak_windows),
            peak_memslots: g(&self.peak_memslots),
            peak_placements: g(&self.peak_placements),
            memslot_installs: g(&self.memslot_installs),
            placements_made: g(&self.placements_made),
            slot_numbers_recycled: g(&self.slot_numbers_recycled),
            accessor_ranked_depth: (h(&self.accessor_depth_min), h(&self.accessor_depth_max)),
            syscall_ranked_depth: (h(&self.syscall_depth_min), h(&self.syscall_depth_max)),
            copy_leaf_depth_max: h(&self.copy_leaf_max),
            view_leaf_depth_max: h(&self.view_leaf_max),
            accesses_served: g(&self.accesses_served),
            accesses_refused: g(&self.accesses_refused),
            host_refusals: g(&self.host_refusals),
            r5_revalidation_failures: g(&self.r5_failures),
            guest_exits: g(&self.guest_exits),
            ram_declared_exits: g(&self.ram_declared_exits),
            first_ram_declared_exit: match g(&self.first_ram_declared_exit) {
                u64::MAX => None,
                gpa => Some(gpa),
            },
            window_releases_deferred: g(&self.window_releases_deferred),
            window_mappings_released: g(&self.window_mappings_released),
        }
    }
}

// =====================================================================================
// The plane
// =====================================================================================

/// What `gpa_read`/`gpa_write` read, under the **view** lock, for a bounded probe only.
#[derive(Debug, Default)]
pub(crate) struct View {
    pub(crate) regions: GuestRamMap,
    windows: BTreeMap<RamRegionId, Arc<GuestWindow>>,
    /// ★★ **The guest-physical spans a read-native window traps WRITES over**, by the
    /// region that owns them — the rounded-outward sub-range that got its own *read-only*
    /// memslot in [`KvmMachine::install_window`].
    ///
    /// It lives here, next to [`View::regions`], and not in the [`Installer`], for one
    /// reason: [`Plane::classify_exit`] runs on the exit path and must see the trap spans
    /// and the region map **under the same lock**. That is what keeps the happens-before
    /// argument in `vcpu.rs`'s docs a proof — a second lock on the exit path would let a
    /// `remove_window` land between the two reads, and the classification would then be
    /// about two different instants.
    write_traps: BTreeMap<RamRegionId, Range<u64>>,
}

/// One installed window, as the **installer** knows it.
#[derive(Debug)]
struct Window {
    gpa: u64,
    len: u64,
    window: Arc<GuestWindow>,
    /// One entry for an ordinary window; up to three for a read-native overlay, whose
    /// write-trap sub-range needs its own read-only slot (§6.7 item 4: protection is a
    /// slot property, so it is a *window* property, never a per-object one).
    memslots: Vec<KvmMemslot>,
    /// The shareable backing behind it, if guest RAM was created shareable.
    ram: Option<Arc<SharedRam>>,
    /// Live `MAP_FIXED` placements inside it, by the slot id that names them.
    placements: BTreeMap<SlotId, (u64, u64)>,
}

/// The **installer**'s state: everything authoritative about the memory plane.
#[derive(Debug)]
struct Installer {
    windows: BTreeMap<RamRegionId, Window>,
    /// Which window a fine-tier `SlotId` was placed in.
    placement_owner: BTreeMap<SlotId, RamRegionId>,
    /// Which window a coarse-tier `SlotId` names.
    window_slot: BTreeMap<SlotId, RamRegionId>,
    /// Trap ranges registered through [`Vmm::set_trap`], **read** by
    /// [`KvmMachine::assert_map_matches_the_kernel`].
    ///
    /// ★ It used to be write-only — pushed here and consulted nowhere, not even by an
    /// accessor — which is the same "reads as protection" shape as #89: a registration
    /// that looks like it configures something and configures nothing. `set_trap` checks
    /// its precondition **at registration time**, but a memslot installed afterwards can
    /// falsify it (install a RAM window over a read-write-trapped BAR range and the
    /// guest's access is served silently, with the trap surviving only in this vector).
    /// The assertion re-checks both modes against the live plane, which is the only thing
    /// that makes keeping the vector worth more than deleting it.
    traps: Vec<(BarId, Range<u64>, TrapMode)>,
    exports: Vec<std::os::fd::OwnedFd>,
    next_slot_id: u64,
    /// ★★ Kernel memslot numbers, **recycled through a free list** — see
    /// [`crate::slotnum`] for the adjudication that changed this. It used to be a bare
    /// ascending cursor on the stated ground that *"a recycled slot number is
    /// indistinguishable from a stale one in a kernel log, and the ceiling is in the
    /// hundreds"*, which is the same policy the sibling QEMU adapter's own docs record as
    /// the C's **measured** exhaustion (`C: nvkvm_mmap_host.c:382-389`, *"exhausted the
    /// pool after a few CUDA processes"*). Neither doc cited the other.
    memslot_numbers: MemslotNumbers,
    /// ★★★ **Strictly monotonic; region ids are minted here and NEVER recycled**, and that
    /// is a load-bearing fact rather than an implementation convenience: it is the whole
    /// reason [`Vmm::map_guest`]'s R5 re-validation can be a presence test. `RamRegionId`
    /// is the key a commit re-looks-up, and a key that is never re-issued cannot suffer
    /// ABA — `Some(w)` at commit is necessarily *the same* [`Window`] the plan read. See
    /// the commit block for the argument this replaced.
    next_region: u64,
    /// ★★ **Removed windows whose mapping has not been released yet** — see
    /// [`Plane::retire`]. The machine holds one reference to each, so an accessor's clone
    /// can never be the last one and an accessor can therefore never be the thread that
    /// `munmap`s.
    retired: Vec<Arc<GuestWindow>>,
}

#[derive(Debug)]
pub(crate) struct Plane {
    /// ★ The subsystem handle, **kept**. M2-c dropped it at the end of `realize` because
    /// memslots only need the VM descriptor; a vCPU needs `/dev/kvm` itself (the size of
    /// the shared run structure is a property of the subsystem, not of the VM), so the
    /// handle now lives as long as the machine.
    kvm: Kvm,
    vm: Arc<KvmVm>,
    page: HostPageSize,
    shareable_ram: bool,
    bars: Vec<BarPlacement>,
    notifier: Notifier,
    /// ★ The **leaf**: taken by the in-lock-legal accessors, beneath one of the core's
    /// ranked locks. Critical section = a `BTreeMap` probe. Never a syscall.
    view: Mutex<View>,
    /// ★ The **installer**: taken by the memory-plane ops, which are entered lock-free.
    /// Critical section = bookkeeping. Never a syscall either — see the crate docs.
    installer: Mutex<Installer>,
    clock: Mutex<(Instant, DeferQueue)>,
    audit: Audit,
}

impl Plane {
    /// Take the view lock, noting the adapter-leaf depth for the R1 witness.
    pub(crate) fn view(&self) -> (MutexGuard<'_, View>, leaf::Held) {
        let g = self
            .view
            .lock()
            .expect("the view leaf lock is never poisoned");
        let held = leaf::Held::enter();
        self.audit
            .view_leaf_max
            .fetch_max(leaf::depth(), Ordering::SeqCst);
        (g, held)
    }

    fn installer(&self) -> (MutexGuard<'_, Installer>, leaf::Held) {
        let g = self
            .installer
            .lock()
            .expect("the installer lock is never poisoned");
        let held = leaf::Held::enter();
        self.audit
            .view_leaf_max
            .fetch_max(leaf::depth(), Ordering::SeqCst);
        (g, held)
    }

    /// ★ Every syscall-shaped entry point starts here. Both halves of R1, at the door of
    /// the phase that actually performs the syscall.
    pub(crate) fn about_to_syscall(&self, what: &str) {
        Audit::note_ranked(
            &self.audit.syscall_depth_min,
            &self.audit.syscall_depth_max,
            kayfabe_util::lockwitness::held_depth(),
        );
        leaf::assert_leaf_free(what);
        // The ranked half is asserted inside `kayfabe-linux-raw` at every syscall
        // (§4.5), so it is deliberately NOT repeated here: one witness, three
        // enforcement sites, zero drift.
        //
        // ★ Every door that reaches this line has just been proved lock-free on both
        // halves, which makes it — and only it — a legal place to `munmap` a window an
        // accessor was still reading when it was removed. See `retire`.
        self.collect_retired();
    }

    /// ★★ **Retire a removed window's mapping: the machine keeps the LAST reference.**
    ///
    /// `KvmVmm::resolve` hands an accessor an `Arc<GuestWindow>` precisely so the memcpy
    /// can run *outside* the view lock (`l1_os_shell.md` §14.8 F2). That `Arc` is a
    /// **release** handle as well as a read handle, and that is the half §14.8 F7(b) never
    /// named when it wrote *"the `Arc` keeps the mapping alive either way"*: if the
    /// accessor's clone is the last one, the `munmap` runs on the **accessor's** thread —
    /// and `gpa_read`/`gpa_write` are in-lock **legal**, entered with one of the core's
    /// ranked locks held (§6.1). A blocking syscall under a rank is exactly R1
    /// (`l1_concurrency.md` §3.3), and it fired in the mean run about one time in six:
    ///
    /// ```text
    /// GuestWindow::drop -> munmap -> lockwitness::assert_lock_free
    ///   "munmap (dropping a guest-physical window) while holding rank(s) [0]"
    ///   <- Arc<GuestWindow> drop <- KvmVmm::gpa_read <- SharedDevice::parse_pushbuffer
    /// ```
    ///
    /// Note what does **not** fix it: the leaf witness sees nothing here (the view lock was
    /// released before the copy, correctly), and no critical section is any longer than it
    /// was. The defect is in *ownership*, not in locking — so the fix is an ownership one.
    /// A removed window's mapping is parked here, with one reference the machine holds and
    /// no accessor can take away, and released by [`Plane::collect_retired`] on a thread
    /// that is lock-free by contract. The accessor's clone is then never the last, so the
    /// accessor can no longer be the thread that releases — structurally, not by timing.
    fn retire(&self, window: Arc<GuestWindow>) {
        // Every other reference this machine had — the view map's, the installer's, and
        // one per memslot — is already gone by the time `remove_window` calls this, so a
        // count above one means an accessor is mid-copy. That is the state the whole
        // mechanism exists for, and a test that never reaches it proves nothing.
        if Arc::strong_count(&window) > 1 {
            self.audit
                .window_releases_deferred
                .fetch_add(1, Ordering::SeqCst);
        }
        {
            let (mut ins, _h) = self.installer();
            ins.retired.push(window);
        }
        self.collect_retired();
    }

    /// Release every retired mapping no accessor still holds, **with every lock dropped**.
    ///
    /// `Arc::strong_count(w) == 1` is exact here rather than advisory: a retired window has
    /// already been removed from the view map *under the view lock*, so `resolve` can never
    /// mint another clone of it and the count only falls. One means this vector is the sole
    /// owner, and dropping it is the `munmap`.
    fn collect_retired(&self) {
        let dead: Vec<Arc<GuestWindow>> = {
            let (mut ins, _h) = self.installer();
            if ins.retired.is_empty() {
                return;
            }
            let (dead, keep): (Vec<_>, Vec<_>) = core::mem::take(&mut ins.retired)
                .into_iter()
                .partition(|w| Arc::strong_count(w) == 1);
            ins.retired = keep;
            dead
        };
        // ★ The `munmap`s themselves: outside the installer lock, outside the view lock,
        // and — because every caller has just passed `about_to_syscall` — outside every
        // ranked lock too. This is the line `GuestWindow::drop`'s `assert_lock_free` is
        // an assertion about.
        leaf::assert_leaf_free("releasing a retired window's mapping");
        self.audit
            .window_mappings_released
            .fetch_add(dead.len() as u64, Ordering::SeqCst);
        drop(dead);
    }
}

/// Map a raw-OS refusal onto the port's vocabulary, keeping the `errno`.
pub(crate) fn host_refused(what: &'static str, e: &RawError) -> VmmError {
    match e {
        RawError::Syscall { errno, .. } => VmmError::HostRefused {
            what,
            errno: *errno,
        },
        RawError::Misaligned { .. }
        | RawError::ZeroLength { .. }
        | RawError::OutOfRange { .. }
        | RawError::LengthOverflow { .. }
        | RawError::TooLargeForHost { .. } => VmmError::Unsupported(what),
        _ => VmmError::Unsupported(what),
    }
}

// =====================================================================================
// The machine
// =====================================================================================

/// A realized KVM-direct machine: the owner of the VM descriptor and the memory plane.
///
/// Hand out [`KvmMachine::vmm`] handles to threads; keep this to drive the coarse tier
/// and to read the [`AuditReport`].
#[derive(Debug)]
pub struct KvmMachine {
    plane: Arc<Plane>,
}

impl KvmMachine {
    /// Open `/dev/kvm`, create a VM, and declare the configured BARs as device windows.
    ///
    /// # Errors
    /// [`VmmError::HostRefused`] if `/dev/kvm` is absent or not permitted — a **deployment
    /// fact**, refused loudly at realize in the §9.3 pattern, never a silent fallback.
    /// [`VmmError::Unsupported`] if the kernel's KVM API version is not one these structs
    /// are written for.
    ///
    /// # Panics
    /// If called with any ranked lock held (R1).
    pub fn realize(cfg: MachineConfig) -> Result<Self, VmmError> {
        let kvm = Kvm::open().map_err(|e| host_refused("opening the KVM device", &e))?;
        let vm = kvm
            .create_vm()
            .map_err(|e| host_refused("creating a VM", &e))?;
        let max_memslots = vm
            .max_memslots()
            .map_err(|e| host_refused("querying the memslot ceiling", &e))?;
        let notifier =
            Notifier::create().map_err(|e| host_refused("creating a notify descriptor", &e))?;

        let mut regions = GuestRamMap::new();
        for b in &cfg.bars {
            regions
                .declare(
                    // A device region id distinct from every RAM region's: the top bit
                    // keeps the two spaces apart so a resolved span can never confuse a
                    // BAR with a window.
                    RamRegionId((1u64 << 63) | b.base),
                    RegionKind::Device,
                    b.base,
                    b.len,
                )
                .map_err(|_| VmmError::Unsupported("a BAR that leaves the 64-bit space"))?;
        }

        Ok(KvmMachine {
            plane: Arc::new(Plane {
                kvm,
                vm: Arc::new(vm),
                page: HostPageSize::query(),
                shareable_ram: cfg.shareable_ram,
                bars: cfg.bars,
                notifier,
                view: Mutex::new(View {
                    regions,
                    windows: BTreeMap::new(),
                    write_traps: BTreeMap::new(),
                }),
                installer: Mutex::new(Installer {
                    windows: BTreeMap::new(),
                    placement_owner: BTreeMap::new(),
                    window_slot: BTreeMap::new(),
                    traps: Vec::new(),
                    exports: Vec::new(),
                    next_slot_id: 1,
                    memslot_numbers: MemslotNumbers::new(max_memslots),
                    next_region: 1,
                    retired: Vec::new(),
                }),
                clock: Mutex::new((Instant::ZERO, DeferQueue::new())),
                audit: Audit::new(),
            }),
        })
    }

    /// A `Vmm` handle. Cheap to clone; hand one to each thread.
    ///
    /// `Vmm` methods take `&mut self` and the trait is `Send` but not `Sync` — the
    /// adapter owns its synchronisation (the trait's threading contract). A handle is
    /// therefore per-thread, and the state behind it is shared.
    #[must_use]
    pub fn vmm(&self) -> KvmVmm {
        KvmVmm {
            plane: Arc::clone(&self.plane),
        }
    }

    /// The host page size this machine's geometry is expressed in.
    #[must_use]
    pub fn page_size(&self) -> HostPageSize {
        self.plane.page
    }

    /// The kernel's memslot ceiling for this VM.
    #[must_use]
    pub fn memslot_ceiling(&self) -> u32 {
        self.plane
            .installer
            .lock()
            .expect("installer")
            .memslot_numbers
            .ceiling()
    }

    /// ★★ **The coarse tier.** Map `len` bytes of host memory and install it as a memslot
    /// at `gpa` — one `mmap` and one `KVM_SET_USER_MEMORY_REGION`, at proc-create
    /// frequency and never at publication frequency (§6.7).
    ///
    /// The window is fully readable from the instant it exists (anonymous zero-fill, or a
    /// sealed shareable backing when the machine was realized with one), because a live
    /// memslot names the whole range whether or not anything has been published into it.
    ///
    /// # Errors
    /// - [`VmmError::Unsupported`] — a misaligned or empty request, or the adapter's slot
    ///   ceiling.
    /// - [`VmmError::HostRefused`] — the kernel refused: `EEXIST` for a guest-physical
    ///   range another memslot already covers, `EINVAL` past the ceiling, `ENOMEM` for a
    ///   window the address space cannot hold. **On any of these the window is dropped
    ///   and nothing is recorded** — the partial-failure path leaks neither a mapping nor
    ///   a region declaration.
    ///
    /// # Panics
    /// If called with any ranked lock or any adapter lock held (R1).
    pub fn install_ram_window(&self, gpa: u64, len: u64) -> Result<RamRegionId, VmmError> {
        self.install_window(gpa, len, None, "installing a RAM window")
            .map(|(region, _)| region)
    }

    /// The shared body of the coarse tier. `read_native` carries the write-trap sub-range
    /// that needs its own read-only memslot.
    fn install_window(
        &self,
        gpa: u64,
        len: u64,
        read_native_trap: Option<Range<u64>>,
        what: &'static str,
    ) -> Result<(RamRegionId, SlotId), VmmError> {
        let p = &self.plane;
        p.about_to_syscall(what);

        // ---- PLAN (installer held; pure bookkeeping, no syscall) -------------------
        let (region, slot_id, memslot_numbers, spans) = {
            let (mut ins, _h) = p.installer();
            if !geometry::is_aligned(gpa, p.page) || !geometry::is_aligned(len, p.page) || len == 0
            {
                return Err(VmmError::Unsupported(
                    "a window whose base or length is not a whole number of host pages",
                ));
            }
            let spans = memslot_spans(gpa, len, read_native_trap.as_ref(), p.page)?;
            // ★★ A write-trap span outside every realized BAR is REFUSED, and this is the
            // half of the read-native overlay that was never designed. The read-only
            // memslot really does send the guest's store out to userspace — but
            // `Plane::classify_exit` can only route that store to a device by finding the
            // address inside a BAR. Outside one there is no device to hand it to, so the
            // store would be classified `Unclaimed` and **silently dropped**. Refusing at
            // install is the only place the caller can still be told; at the exit it is
            // one lost guest write and no diagnosis.
            //
            // The check is on the **rounded** span, not the requested one, because the
            // rounded span is what physically becomes read-only: a 4 KiB request on a
            // 64 KiB-page host traps a whole 64 KiB, and every byte of it must have a
            // device behind it.
            if let Some((off, l, _)) = spans.iter().find(|(_, _, ro)| *ro) {
                let (t0, t1) = (gpa + off, gpa + off + l);
                if !p.bars.iter().any(|b| t0 >= b.base && t1 <= b.base + b.len) {
                    return Err(VmmError::Unsupported(TRAP_SPAN_OUTSIDE_EVERY_BAR));
                }
            }
            let numbers = ins
                .memslot_numbers
                .alloc(spans.len())
                .map_err(VmmError::Unsupported)?;
            p.audit
                .slot_numbers_recycled
                .store(ins.memslot_numbers.recycled(), Ordering::SeqCst);
            let region = RamRegionId(ins.next_region);
            ins.next_region += 1;
            let slot_id = SlotId(ins.next_slot_id);
            ins.next_slot_id += 1;
            (region, slot_id, numbers, spans)
        };

        // ---- EXECUTE (every lock dropped; this is where the syscalls happen) -------
        leaf::assert_leaf_free(what);
        let outcome = (|| -> Result<Window, VmmError> {
            let window = Arc::new(GuestWindow::create(len, p.page).map_err(|e| {
                p.audit.host_refusals.fetch_add(1, Ordering::SeqCst);
                host_refused("a window mapping", &e)
            })?);
            let ram = if p.shareable_ram {
                let r = Arc::new(SharedRam::create(len).map_err(|e| {
                    p.audit.host_refusals.fetch_add(1, Ordering::SeqCst);
                    host_refused("a shareable guest-RAM backing", &e)
                })?);
                window
                    .place(
                        HostOffset::ZERO,
                        len,
                        Backing::SharedFile {
                            fd: r.as_backing_fd(),
                            offset: 0,
                        },
                    )
                    .map_err(|e| {
                        p.audit.host_refusals.fetch_add(1, Ordering::SeqCst);
                        host_refused("placing the guest-RAM backing", &e)
                    })?;
                Some(r)
            } else {
                None
            };
            let mut memslots = Vec::new();
            for (n, (off, l, ro)) in memslot_numbers.iter().zip(spans.iter()) {
                let slot = KvmMemslot::install(
                    Arc::clone(&p.vm),
                    *n,
                    gpa + off,
                    Arc::clone(&window),
                    *off,
                    *l,
                    *ro,
                )
                .map_err(|e| {
                    p.audit.host_refusals.fetch_add(1, Ordering::SeqCst);
                    host_refused("a memslot install", &e)
                })?;
                p.audit.memslot_installs.fetch_add(1, Ordering::SeqCst);
                memslots.push(slot);
            }
            Ok(Window {
                gpa,
                len,
                window,
                memslots,
                ram,
                placements: BTreeMap::new(),
            })
        })();

        // ★ The partial-failure path. `outcome` owns everything that was created, so
        // dropping it here `munmap`s the window and clears whatever memslots did install
        // — before any of it was recorded anywhere. Nothing leaks and nothing is
        // half-declared, which is the property a mock's infallible insert cannot have.
        //
        // ★★ The numbers go back too, and they go back **after** that drop — the ordering
        // [`MemslotNumbers::release`]'s contract demands, since each drop is the clearing
        // ioctl. This arm was a plain `outcome?` while numbers were never recycled: a leak
        // that a never-recycling allocator makes invisible, because it leaks anyway.
        let installed = match outcome {
            Ok(i) => i,
            Err(e) => {
                let (mut ins, _h) = p.installer();
                for n in memslot_numbers {
                    ins.memslot_numbers.release(n);
                }
                return Err(e);
            }
        };
        let n_slots = installed.memslots.len() as u64;
        let win_handle = Arc::clone(&installed.window);
        // The rounded, guest-physical span that is read-only in the kernel — i.e. exactly
        // the addresses at which a guest STORE exits and a guest LOAD does not.
        let trap_span = spans
            .iter()
            .find(|(_, _, ro)| *ro)
            .map(|(off, l, _)| (gpa + off)..(gpa + off + l));

        // ---- COMMIT (installer, then the view) ------------------------------------
        {
            let (mut ins, _h) = p.installer();
            ins.window_slot.insert(slot_id, region);
            ins.windows.insert(region, installed);
            // ★ The credits, under the lock that owns the state — the same argument as
            // `map_guest`'s commit, one tier up: a `remove_window` racing this one debits
            // `live_windows`/`live_memslots`/`window_bytes` under this lock, so crediting
            // outside it lets the debit run first and wrap the counter.
            Audit::bump(&p.audit.live_windows, &p.audit.peak_windows, 1);
            Audit::bump(
                &p.audit.live_memslots,
                &p.audit.peak_memslots,
                n_slots as i64,
            );
            p.audit.window_bytes.fetch_add(len, Ordering::SeqCst);
        }
        {
            let (mut v, _h) = p.view();
            v.regions
                .declare(region, RegionKind::Ram, gpa, len)
                .map_err(|_| VmmError::Unsupported("a window that leaves the 64-bit space"))?;
            v.windows.insert(region, win_handle);
            // ★ Under the SAME lock as the declaration. The two facts a classification
            // needs — "this range is RAM" and "a store into it exits" — are published
            // atomically, so no exit can observe one without the other.
            if let Some(t) = trap_span {
                v.write_traps.insert(region, t);
            }
        }
        Ok((region, slot_id))
    }

    /// ★★ Remove a window: the guest-physical range stops resolving **first**, then the
    /// memslot is cleared and the mapping released.
    ///
    /// The order is the whole correctness argument. Undeclaring under the view lock (no
    /// syscall) makes every subsequent `gpa_read` refuse with [`VmmError::BadGpa`]; a
    /// reader that had *already* resolved holds an `Arc` on the window, so its copy
    /// completes against memory that is still mapped. There is no instant at which a
    /// resolved offset points at a released mapping — which is why §12.43's *"no `RamGpa`
    /// proof token"* ruling costs nothing here.
    ///
    /// # Errors
    /// [`VmmError::BadSlot`] if the region is unknown.
    ///
    /// # Panics
    /// If called with any ranked lock or any adapter lock held (R1).
    pub fn remove_window(&self, region: RamRegionId) -> Result<(), VmmError> {
        let p = &self.plane;
        p.about_to_syscall("removing a window (a memslot DELETE plus a munmap)");

        // ---- PLAN + the visibility half of COMMIT, both lock-only ------------------
        let taken = {
            let (mut ins, _h) = p.installer();
            let Some(w) = ins.windows.remove(&region) else {
                return Err(VmmError::BadSlot(SlotId(region.0)));
            };
            // ★★ **The removal from `ins.windows` above IS the R5 signal**, and nothing
            // else is needed. There used to be an `ins.generation += 1` here, stamped into
            // every window at install and re-checked at `map_guest`'s commit — see that
            // block for why the check could not fail.
            ins.placement_owner.retain(|_, r| *r != region);
            ins.window_slot.retain(|_, r| *r != region);
            // ★ The debits, under the same lock that took the state away — see the note
            // in `map_guest`'s commit. `w.placements` is authoritative only while this
            // guard is held: the instant it is dropped, a concurrent `map_guest` commit
            // may add one more.
            Audit::bump(&p.audit.live_windows, &p.audit.peak_windows, -1);
            Audit::bump(
                &p.audit.live_memslots,
                &p.audit.peak_memslots,
                -(w.memslots.len() as i64),
            );
            Audit::bump(
                &p.audit.live_placements,
                &p.audit.peak_placements,
                -(w.placements.len() as i64),
            );
            p.audit.window_bytes.fetch_sub(w.len, Ordering::SeqCst);
            w
        };
        {
            let (mut v, _h) = p.view();
            v.regions.undeclare(taken.gpa, taken.len);
            v.windows.remove(&region);
            // ★ And the write-trap span goes with it, in the same critical section. A
            // stale entry would be worse than a missing one: it would make a LATER exit
            // at that address dispatch into a device on the strength of a read-only
            // memslot that no longer exists.
            v.write_traps.remove(&region);
        }

        // ---- EXECUTE (every lock dropped): the memslot DELETEs happen in `drop`, here.
        // The `munmap` does NOT — an accessor may still be copying out of this mapping,
        // and if its `Arc` were the last one it would perform the release itself, under
        // the ranked lock `gpa_read` is legally entered with. See `Plane::retire`.
        leaf::assert_leaf_free("dropping a window's memslots and mapping");
        let Window {
            window,
            memslots,
            ram,
            ..
        } = taken;
        // ★★★ The ordering [`MemslotNumbers::release`]'s contract demands: read the numbers,
        // drop the live slots FIRST — each `Drop` is the clearing ioctl — and only then hand
        // the numbers back. A number returned before its slot was cleared turns the next
        // install from an ADD into a silent REPLACE.
        let numbers: Vec<u32> = memslots.iter().map(KvmMemslot::slot).collect();
        drop(memslots);
        drop(ram);
        {
            let (mut ins, _h) = p.installer();
            for n in numbers {
                ins.memslot_numbers.release(n);
            }
        }
        p.retire(window);
        Ok(())
    }

    /// Declare `[gpa, gpa+len)` a **device register window** — another emulated device's
    /// BAR, the platform's own MMIO, one of ours.
    ///
    /// No syscall: on KVM a device region **is** the absence of a memslot, so declaring
    /// one is exactly *not* installing anything. Guest-physical accesses to it refuse
    /// with [`VmmError::NonRamGpa`], which is the guest-steerable inversion §10.1 item 6
    /// names and the only signal it happened.
    ///
    /// # Errors
    /// [`VmmError::Unsupported`] if the range leaves the 64-bit space.
    pub fn declare_device_window(&self, gpa: u64, len: u64) -> Result<(), VmmError> {
        let (mut v, _h) = self.plane.view();
        v.regions
            .declare(
                RamRegionId((1u64 << 63) | gpa),
                RegionKind::Device,
                gpa,
                len,
            )
            .map_err(|_| VmmError::Unsupported("a device window that leaves the 64-bit space"))
    }

    /// Create a host backing an isolate would have handed us, and name it as the port
    /// does. `HostRegion::id` is backend-scoped and opaque by contract; here it indexes
    /// this machine's registry.
    ///
    /// # Errors
    /// [`VmmError::HostRefused`] if the host refuses the backing.
    ///
    /// # Panics
    /// If called with any ranked lock or any adapter lock held (R1).
    pub fn register_backing(&self, len: u64) -> Result<HostRegion, VmmError> {
        let p = &self.plane;
        p.about_to_syscall("creating a host backing");
        let ram = SharedRam::create(len).map_err(|e| {
            p.audit.host_refusals.fetch_add(1, Ordering::SeqCst);
            host_refused("a host backing", &e)
        })?;
        let dup = ram
            .dup_for_export()
            .map_err(|e| host_refused("duplicating a host backing", &e))?;
        let (mut ins, _h) = p.installer();
        ins.exports.push(dup);
        Ok(HostRegion {
            id: ins.exports.len() as u64 - 1,
            offset: 0,
        })
    }

    /// ★ Query the guest-physical **region map** directly, without touching a window.
    ///
    /// Exists because a bite-check found that nothing could tell *"the region was
    /// undeclared"* from *"the region is still declared RAM but its window is gone"* —
    /// both arrive at `gpa_read` as [`VmmError::BadGpa`], so a `remove_window` that
    /// forgot to undeclare passed every test. They are **not** the same state: a stale
    /// RAM declaration is a guest-physical range the map promises is memory and the
    /// kernel has no memslot for, which is exactly the disagreement
    /// [`KvmMachine::assert_map_matches_the_kernel`] exists to forbid.
    ///
    /// # Errors
    /// As `GuestRamMap::resolve`.
    pub fn resolve_region(&self, gpa: u64, len: u64) -> Result<kayfabe_vmm::RamSpan, VmmError> {
        let (v, _h) = self.plane.view();
        v.regions.resolve(gpa, len)
    }

    /// The audit / conservation ledger.
    #[must_use]
    pub fn audit(&self) -> AuditReport {
        self.plane.audit.report()
    }

    /// ★ The **consistency check the mock cannot express**: every region the map calls RAM
    /// is covered by a live memslot, and every region it calls a device is covered by
    /// none. Returns the number of regions checked, so a caller can assert non-vacuity.
    ///
    /// # Panics
    /// If the two disagree — which would mean the map and the kernel's flat view had
    /// drifted, and the guest would be the first to notice.
    #[must_use]
    pub fn assert_map_matches_the_kernel(&self) -> usize {
        let (v, _h) = self.plane.view();
        let (ins, _h2) = self.plane.installer();
        let mut checked = 0usize;
        for (region, window) in &v.windows {
            let w = ins
                .windows
                .get(region)
                .unwrap_or_else(|| panic!("{region:?} resolves as RAM but has no window"));
            assert!(
                !w.memslots.is_empty(),
                "{region:?} resolves as RAM but has no live memslot — on KVM a RAM region \
                 IS a memslot, and a range with none exits to userspace instead"
            );
            assert!(
                Arc::ptr_eq(window, &w.window),
                "{region:?}'s view and installer disagree about which mapping backs it"
            );
            checked += 1;
        }
        assert_eq!(
            checked,
            ins.windows.len(),
            "the installer holds windows the view cannot resolve — a mapping the guest \
             can reach and the region map denies is the inverse of the §10.1 item 6 hazard"
        );
        // ★★★ **What is deliberately NOT asserted here, and why.**
        //
        // The obvious companion to [`crate::slotnum`]'s release contract is *"no two live
        // windows hold the same kernel memslot number"*. It was written, and then removed,
        // because **it could not be made to fire.** Every route by which an allocator hands
        // out a number that is already live is caught earlier and more sharply somewhere
        // else, as four separate bite-checks measured on this kernel:
        //
        // - re-issuing a live number to a second window at a different guest-physical base
        //   is `EINVAL` from `KVM_SET_USER_MEMORY_REGION` (a move to a new host address is
        //   not a thing KVM does), so the install fails and no duplicate is recorded;
        // - re-issuing it over an *overlapping* range is the kernel's `EEXIST`;
        // - a number returned to the pool twice trips
        //   [`slotnum::MemslotNumbers::release`]'s own assertion first;
        // - and a slot that really did get double-installed fails **loudly at teardown**, in
        //   `kayfabe_linux_raw`'s clearing assertion — measured in the QEMU adapter's
        //   `kvm_differential`.
        //
        // A duplicate that survives to be observed here needs the one arm the kernel accepts
        // silently (same number, same base, same size, different host mapping), which no
        // path through this adapter can produce. An assertion nobody could ever see fail is
        // the exact defect this whole audit is about, so it is a comment instead.
        // ★★ And every registered trap is still PHYSICALLY a trap. `set_trap` checked its
        // precondition once, at registration; a window installed afterwards can falsify
        // it, and until this loop existed `Installer::traps` was never read by anything,
        // so nothing could notice. Both modes are checked against the live plane:
        for (bar, range, mode) in &ins.traps {
            let Some(b) = self.plane.bars.iter().find(|b| b.bar == *bar) else {
                continue;
            };
            let (t0, t1) = (b.base + range.start, b.base + range.end);
            match mode {
                // A read-write trap IS the absence of a memslot. If the range resolves,
                // something installed RAM under it and the guest's access is now served
                // inside the guest — the trap survives only in this vector.
                TrapMode::ReadWrite => assert!(
                    v.regions.resolve(t0, t1 - t0).is_err(),
                    "{bar:?}+{range:?} is registered as a read-write trap but a live \
                     memslot now serves it — the guest's access never leaves the guest"
                ),
                // A write-only trap IS a read-only memslot. Without one, reads are still
                // served but writes are served too, silently.
                TrapMode::WriteOnly => assert!(
                    v.write_traps.values().any(|t| t0 >= t.start && t1 <= t.end),
                    "{bar:?}+{range:?} is registered as a write-only trap but no \
                     read-native overlay's read-only span covers it — its writes land in \
                     RAM instead of exiting"
                ),
            }
        }
        checked
    }

    /// Advance the virtual clock, returning every deferred event that became due.
    pub fn advance(&self, d: Duration) -> Vec<CoreEvent> {
        let mut c = self.plane.clock.lock().expect("clock");
        c.0 = c.0.advanced(d);
        let now = c.0;
        c.1.due(now)
    }

    /// How many notify edges have accumulated (drains them).
    ///
    /// # Errors
    /// [`VmmError::HostRefused`].
    ///
    /// # Panics
    /// If called with any ranked lock held (R1).
    pub fn drain_irqs(&self) -> Result<u64, VmmError> {
        self.plane
            .notifier
            .drain()
            .map_err(|e| host_refused("draining the notify descriptor", &e))
    }
}

/// The `(offset, len, guest_readonly)` memslot spans one window needs.
///
/// One span for an ordinary window. For a read-native overlay the write-trap sub-range —
/// **rounded outward to whole host pages**, because read-native versus trapped is a *page*
/// attribute in every hypervisor's mapping machinery (16 KiB or 64 KiB on arm64, not
/// 4 KiB) — becomes its own read-only slot, and the remainder either side stays
/// read-write. That is §6.7 item 4 exactly: protection is a slot property, so it is a
/// window property, and it is never expressed per published object.
fn memslot_spans(
    gpa: u64,
    len: u64,
    trap: Option<&Range<u64>>,
    page: HostPageSize,
) -> Result<Vec<(u64, u64, bool)>, VmmError> {
    let Some(t) = trap else {
        return Ok(vec![(0, len, false)]);
    };
    if t.start < gpa || t.end > gpa + len || t.start >= t.end {
        return Err(VmmError::Unsupported(
            "a write-trap sub-range that is not inside the range it overlays",
        ));
    }
    let (rounded_start, rounded_len) = geometry::round_out(t.start - gpa, t.end - t.start, page)
        .map_err(|_| VmmError::Unsupported("a write-trap sub-range that cannot be page-aligned"))?;
    let rounded_end = (rounded_start + rounded_len).min(len);
    let mut out = Vec::new();
    if rounded_start > 0 {
        out.push((0, rounded_start, false));
    }
    out.push((rounded_start, rounded_end - rounded_start, true));
    if rounded_end < len {
        out.push((rounded_end, len - rounded_end, false));
    }
    Ok(out)
}

// =====================================================================================
// The Vmm impl
// =====================================================================================

/// A per-thread handle onto a [`KvmMachine`]'s memory plane.
#[derive(Debug, Clone)]
pub struct KvmVmm {
    plane: Arc<Plane>,
}

impl KvmVmm {
    /// The audit / conservation ledger of the machine behind this handle.
    #[must_use]
    pub fn audit(&self) -> AuditReport {
        self.plane.audit.report()
    }

    /// ★ Resolve a guest-physical range to a window and an offset — **the one place a
    /// guest-chosen address is proven to be host RAM**, and the only critical section the
    /// view lock has.
    ///
    /// The `Arc` clone is what lets the copy happen outside the lock: a concurrent
    /// [`KvmMachine::remove_window`] can undeclare the region and clear its memslot while
    /// we copy, and the mapping still cannot go away underneath us.
    fn resolve(&self, gpa: u64, len: u64) -> Result<(Arc<GuestWindow>, u64), VmmError> {
        Audit::note_ranked(
            &self.plane.audit.accessor_depth_min,
            &self.plane.audit.accessor_depth_max,
            kayfabe_util::lockwitness::held_depth(),
        );
        let (v, _held) = self.plane.view();
        let span = v.regions.resolve(gpa, len).inspect_err(|_| {
            self.plane
                .audit
                .accesses_refused
                .fetch_add(1, Ordering::SeqCst);
        })?;
        let w = v.windows.get(&span.region).cloned().ok_or_else(|| {
            self.plane
                .audit
                .accesses_refused
                .fetch_add(1, Ordering::SeqCst);
            VmmError::BadGpa { gpa }
        })?;
        // ★ Released EXPLICITLY, before the caller copies. The lifetime would do it at the
        // end of this function anyway; writing it out is what makes the property visible
        // at the line that owns it, and what a bite-check can neuter — `copy_leaf_depth_max`
        // is the assertion, and this is the code it is an assertion about.
        let offset = span.offset;
        core::mem::drop(v);
        core::mem::drop(_held);
        Ok((w, offset))
    }

    /// Note that a copy is about to run, and witness that it is running **outside** the
    /// view lock.
    fn about_to_copy(&self) {
        self.plane
            .audit
            .copy_leaf_max
            .fetch_max(leaf::depth(), Ordering::SeqCst);
        self.plane
            .audit
            .accesses_served
            .fetch_add(1, Ordering::SeqCst);
    }
}

impl Vmm for KvmVmm {
    fn gpa_read(&mut self, gpa: u64, buf: &mut [u8]) -> Result<(), VmmError> {
        let (window, offset) = self.resolve(gpa, buf.len() as u64)?;
        // ★ A zero-length access still names an address and was proven above; serving it
        // is a no-op, and the raw layer refuses an empty copy by design.
        if buf.is_empty() {
            self.about_to_copy();
            return Ok(());
        }
        self.about_to_copy();
        window
            .read_into(HostOffset::new(offset), buf)
            .map_err(|e| host_refused("a guest-physical read", &e))
    }

    fn gpa_write(&mut self, gpa: u64, buf: &[u8]) -> Result<(), VmmError> {
        let (window, offset) = self.resolve(gpa, buf.len() as u64)?;
        if buf.is_empty() {
            self.about_to_copy();
            return Ok(());
        }
        self.about_to_copy();
        window
            .write_from(HostOffset::new(offset), buf)
            .map_err(|e| host_refused("a guest-physical write", &e))
    }

    /// ★★ **The fine tier** — a `MAP_FIXED` placement inside an already-installed window.
    /// **No KVM ioctl at all** (§6.7): `mmap_lock`, no `slots_lock`, no SRCU grace period,
    /// no cost that scales with vCPU count. Still a syscall, so still in-lock illegal —
    /// §6.2's shape stands and only its cost model changes.
    fn map_guest(
        &mut self,
        gpa: u64,
        len: u64,
        backing: HostRegion,
        prot: Prot,
    ) -> Result<SlotId, VmmError> {
        let p = &self.plane;
        p.about_to_syscall("map_guest (a MAP_FIXED placement inside a window)");

        // ---- PLAN -----------------------------------------------------------------
        let (slot_id, region, window, offset) = {
            let (mut ins, _h) = p.installer();
            if prot == Prot::ReadOnly {
                // §6.7 item 4: KVM's read-only flag is a SLOT property, so it cannot be
                // expressed for one object inside a shared read-write window. The design
                // answer is to partition into an RO window and an RW window and place the
                // object in the right one — never to mint a slot to make one page
                // read-only. Refusing is what keeps that from being "fixed" the cheap way.
                return Err(VmmError::Unsupported(
                    "per-object read-only protection inside a read-write window — \
                     protection is a WINDOW property (§6.7 item 4); place the object in \
                     a read-only window instead",
                ));
            }
            let Some((region, off)) = ins
                .windows
                .iter()
                .find(|(_, w)| gpa >= w.gpa && gpa.saturating_add(len) <= w.gpa + w.len)
                .map(|(r, w)| (*r, gpa - w.gpa))
            else {
                return Err(VmmError::BadGpa { gpa });
            };
            let w = ins.windows.get(&region).expect("just found");
            if w.placements
                .values()
                .any(|(o, l)| off < o + l && *o < off + len)
            {
                return Err(VmmError::Unsupported(
                    "a placement overlapping one already live in the same window",
                ));
            }
            let window = Arc::clone(&w.window);
            let slot_id = SlotId(ins.next_slot_id);
            ins.next_slot_id += 1;
            (slot_id, region, window, off)
        };

        // ---- EXECUTE (every lock dropped) -----------------------------------------
        leaf::assert_leaf_free("map_guest's MAP_FIXED");
        {
            let (ins, _h) = p.installer();
            let fd = usize::try_from(backing.id)
                .ok()
                .and_then(|i| ins.exports.get(i))
                .map(std::os::fd::AsFd::as_fd);
            let Some(fd) = fd else {
                return Err(VmmError::Unsupported(
                    "a host backing id this backend never minted",
                ));
            };
            // The borrow of `ins` must not outlive the lock, and the placement is a
            // syscall — so the descriptor is duplicated into an owned handle and the lock
            // is dropped before `place` runs. A `BorrowedFd` that outlived the guard
            // would be exactly the "hold a lock across a syscall" shape this crate exists
            // to make impossible.
            let owned = std::fs::File::from(
                dup_owned(fd).map_err(|e| host_refused("duplicating a host backing", &e))?,
            );
            drop(ins);
            drop(_h);
            leaf::assert_leaf_free("map_guest's MAP_FIXED");
            window
                .place(
                    HostOffset::new(offset),
                    len,
                    Backing::SharedFile {
                        fd: std::os::fd::AsFd::as_fd(&owned),
                        offset: backing.offset,
                    },
                )
                .map_err(|e| {
                    p.audit.host_refusals.fetch_add(1, Ordering::SeqCst);
                    host_refused("a placement inside a window", &e)
                })?;
        }
        p.audit.placements_made.fetch_add(1, Ordering::SeqCst);

        // ---- COMMIT + R5 -----------------------------------------------------------
        //
        // ★★★ **R5's token is the window's PRESENCE, not a version counter** — the same
        // ruling the sibling adapter already made (`kayfabe-vmm-qemu`'s `map_guest`), and
        // it is re-derived here rather than copied, because an argument that fits one seam
        // need not fit the other.
        //
        // What stood here was `Some(w) if w.generation == generation`, against a `u64`
        // stamped into each [`Window`] at install from an `Installer`-wide counter that
        // `remove_window` bumped. It reads as an ABA defence and is not one: it **cannot
        // fail**. The key this commit re-looks-up is the `RamRegionId` minted from
        // [`Installer::next_region`], which only ever increments and is never recycled, so
        // no second window can ever appear under a region id a first one vacated. `Some(w)`
        // therefore implies *the same* `Window` object the plan phase read — and a
        // `Window`'s fields other than `placements` are never mutated after insert — so the
        // stamped generation was, necessarily, the one the plan had already read out of the
        // very same struct. A constant compared against itself.
        //
        // ⊘ And it was NOT worth making real. A counter can only carry information the
        // key does not, i.e. re-identification across a recycled key, and that is
        // structurally impossible above. Bumping it on *placement* changes instead would
        // give it something to say — but the wrong thing: it would refuse a concurrent
        // publication into a different part of a perfectly live window at publication
        // frequency, converting a real (and separate) hazard into a livelock.
        //
        // The presence test, by contrast, is falsifiable and has been **watched fail**:
        // `crates/kayfabe-vmm-kvm/tests/r5_revalidation.rs` races a `remove_window` into
        // the gap and asserts the refusal and the ledger (2026-08-01).
        let (mut ins, _h) = p.installer();
        match ins.windows.get_mut(&region) {
            Some(w) => {
                w.placements.insert(slot_id, (offset, len));
                ins.placement_owner.insert(slot_id, region);
                // ★ The ledger moves WITH the state it counts, under the lock that owns
                // the state. Outside it, a concurrent `remove_window` — which subtracts
                // `w.placements.len()`, i.e. *including this one* — can land its debit
                // before this credit and drive the counter through zero. That is a real
                // `attempt to subtract with overflow` in debug and a silent wrap in
                // release, and it is not a counting bug: it is the ledger and the state
                // moving under different locks. Three atomic RMWs are not a syscall, so
                // R1 has nothing to say about doing them here.
                Audit::bump(&p.audit.live_placements, &p.audit.peak_placements, 1);
                drop(_h);
                drop(ins);
                Ok(slot_id)
            }
            _ => {
                // ★ R5. The window died in the gap between plan and commit. Our `Arc`
                // kept the mapping alive, so the placement we made is not a
                // use-after-free — it is simply invisible to the guest, and the whole
                // window is released when the last handle drops. Refuse by the name the
                // guest would have got had it asked a moment later.
                drop(_h);
                drop(ins);
                p.audit.r5_failures.fetch_add(1, Ordering::SeqCst);
                Err(VmmError::BadGpa { gpa })
            }
        }
    }

    /// Remove a mapping — **both tiers**, because the trait's one method covers both
    /// (§6.7's correction to §6.1's table).
    ///
    /// A fine-tier slot is *restored* to anonymous backing, never `munmap`ped (§6.7 item
    /// 3): a hole in the window's VMA would leave the live memslot pointing at a gap. A
    /// coarse-tier slot removes its window.
    fn unmap_guest(&mut self, slot: SlotId) -> Result<(), VmmError> {
        let p = &self.plane;
        p.about_to_syscall("unmap_guest");

        // ---- PLAN -----------------------------------------------------------------
        let plan = {
            let (mut ins, _h) = p.installer();
            if let Some(region) = ins.window_slot.get(&slot).copied() {
                Some(Err(region))
            } else if let Some(region) = ins.placement_owner.remove(&slot) {
                let w = ins
                    .windows
                    .get_mut(&region)
                    .ok_or(VmmError::BadSlot(slot))?;
                let (off, len) = w.placements.remove(&slot).ok_or(VmmError::BadSlot(slot))?;
                Some(Ok((Arc::clone(&w.window), off, len)))
            } else {
                None
            }
        };
        match plan {
            None => Err(VmmError::BadSlot(slot)),
            // Coarse: delegate to the window path, which is lock-free by construction.
            Some(Err(region)) => KvmMachine {
                plane: Arc::clone(&self.plane),
            }
            .remove_window(region),
            // Fine: restore, outside every lock.
            Some(Ok((window, off, len))) => {
                leaf::assert_leaf_free("unmap_guest's restore");
                window
                    .restore(HostOffset::new(off), len)
                    .map_err(|e| host_refused("restoring anonymous backing", &e))?;
                // ★ This debit deliberately stays OUTSIDE the lock, unlike the three
                // above, and the asymmetry is the point. It cannot underflow: this slot's
                // credit landed under the lock before `map_guest` returned it, the plan
                // phase above already took the placement out of `w.placements`, so a
                // racing `remove_window` cannot debit it a second time — and only the
                // pairs that CAN race need to move together. Deferring it past the syscall
                // is what keeps `live_placements` non-zero when the restore fails, i.e.
                // what keeps the conservation assertion able to see a real leak.
                Audit::bump(&p.audit.live_placements, &p.audit.peak_placements, -1);
                Ok(())
            }
        }
    }

    /// Register a trapped MMIO range on `bar`.
    ///
    /// On KVM a trapped range **is** a range with no memslot: the guest's access has
    /// nowhere to land and KVM exits to userspace. So this refuses a range that a live
    /// memslot covers — with a memslot in place the access would be served silently and
    /// the trap would never fire, which is a trap that exists only in our bookkeeping.
    fn set_trap(&mut self, bar: BarId, range: Range<u64>, mode: TrapMode) -> Result<(), VmmError> {
        let p = &self.plane;
        p.about_to_syscall("set_trap");
        let Some(b) = p.bars.iter().find(|b| b.bar == bar) else {
            return Err(VmmError::Unsupported(
                "a BAR this machine was not realized with",
            ));
        };
        if range.start >= range.end || range.end > b.len {
            return Err(VmmError::Unsupported("a trap range outside its BAR"));
        }
        let gpa = b.base + range.start;
        let len = range.end - range.start;
        {
            let (v, _h) = p.view();
            match (v.regions.resolve(gpa, len), mode) {
                // Read-write trapping needs the range to be a device region, i.e. to have
                // no memslot at all.
                (Ok(_), TrapMode::ReadWrite) => {
                    return Err(VmmError::Unsupported(
                        "a read-write trap over a range a live memslot already serves — \
                         the guest's access would never leave the guest",
                    ));
                }
                // ★ Write-only trapping is the read-native overlay's other half: reads
                // are served from RAM, so the range MUST resolve — and the memslot that
                // serves them must be the READ-ONLY one `map_read_native` installed.
                // "It resolves" alone was the weaker claim, and it accepted an ordinary
                // read-write window, over which a write does not exit at all: a
                // registration that reads as protection and is none.
                (Err(e), TrapMode::WriteOnly) => return Err(e),
                (Ok(_), TrapMode::WriteOnly)
                    if !v
                        .write_traps
                        .values()
                        .any(|t| gpa >= t.start && gpa + len <= t.end) =>
                {
                    return Err(VmmError::Unsupported(WRITE_TRAP_WITHOUT_A_READ_ONLY_SLOT));
                }
                _ => {}
            }
        }
        let (mut ins, _h) = p.installer();
        ins.traps.push((bar, range, mode));
        Ok(())
    }

    /// ★ The one **in-lock-legal** syscall: a single write to a non-blocking notify
    /// descriptor, `l1_os_shell.md` §4.5's named exception.
    ///
    /// It declares the ranks it permits in `kayfabe-linux-raw`'s signature rather than
    /// asserting lock-freedom, which is what makes §6.1's *"yes — the single named
    /// exception"* a mechanism instead of a claim. `grep -rn _under_lock` enumerates the
    /// entire set.
    fn raise_irq(&mut self, irq: IrqSpec) -> Result<(), VmmError> {
        match irq {
            IrqSpec::Msix(_) => self
                .plane
                .notifier
                // Ranks 0 and 1 are the two the core can be holding when a completion is
                // encoded and delivered (§12.7's observe→pump→encode→write→IRQ edge).
                .signal_under_lock(
                    kayfabe_util::lockwitness::bit(0) | kayfabe_util::lockwitness::bit(1),
                )
                .map_err(|e| host_refused("a notify-descriptor write", &e)),
            // Backend-conditional by design (the trait's own rustdoc): this harness has
            // no interrupt controller at all, so a legacy line is unimplementable here.
            // A refusal, never a silently dropped injection.
            IrqSpec::IntxLevel(_) => Err(VmmError::Unsupported(
                "legacy INTx needs an interrupt controller this harness does not create",
            )),
        }
    }

    /// Export guest RAM for an isolate — a real duplicated, sealed descriptor.
    ///
    /// ★ Refuses loudly when the machine was realized without a shareable backing
    /// (§4.4.1): *"an adapter launched without it MUST fail this call with
    /// `VmmError::Unsupported` at the first export — loudly, at device realize, never at
    /// first guest DMA."*
    fn export_ram(&mut self, slice: Option<Range<u64>>) -> Result<RamHandle, VmmError> {
        let p = &self.plane;
        p.about_to_syscall("export_ram");
        if !p.shareable_ram {
            return Err(VmmError::Unsupported(NO_SHARED_BACKING));
        }
        let ram = {
            let (ins, _h) = p.installer();
            let want = slice.clone();
            ins.windows
                .values()
                .find(|w| match &want {
                    None => true,
                    Some(s) => s.start >= w.gpa && s.end <= w.gpa + w.len,
                })
                .and_then(|w| w.ram.clone())
        };
        let Some(ram) = ram else {
            return Err(VmmError::Unsupported(
                "no shareable window covers the requested slice",
            ));
        };
        leaf::assert_leaf_free("export_ram's descriptor duplication");
        let dup = ram
            .dup_for_export()
            .map_err(|e| host_refused("duplicating guest RAM", &e))?;
        let (mut ins, _h) = p.installer();
        ins.exports.push(dup);
        Ok(RamHandle {
            token: ins.exports.len() as u64 - 1,
            covers: slice,
        })
    }

    fn defer(&mut self, after: Duration, event: CoreEvent) {
        let mut c = self.plane.clock.lock().expect("clock");
        let now = c.0;
        c.1.push(now, after, event);
    }

    fn now(&self) -> Instant {
        self.plane.clock.lock().expect("clock").0
    }

    /// ★ The read-native overlay (§6.1 group 7) — and on KVM it is not an emulation of the
    /// rom-device pattern, it **is** one: a read-only memslot serves reads from RAM with
    /// no exit and takes writes out to userspace.
    ///
    /// `write_trap` is interpreted as a **guest-physical** sub-range of `[gpa, gpa+len)`
    /// and is rounded **outward to whole host pages**, per the trait's own note: a caller
    /// that reasons in 4 KiB gets more pages trapped than it asked for on an arm64 host,
    /// which is correct and quietly slower.
    fn map_read_native(
        &mut self,
        gpa: u64,
        len: u64,
        backing: HostRegion,
        write_trap: Option<Range<u64>>,
    ) -> Result<SlotId, VmmError> {
        let machine = KvmMachine {
            plane: Arc::clone(&self.plane),
        };
        let (region, slot) = machine.install_window(
            gpa,
            len,
            write_trap,
            "map_read_native (a read-native window)",
        )?;
        // The overlay's contents come from the named backing; a read-native window whose
        // pages were anonymous zeroes would serve reads natively and serve the wrong
        // value, which is worse than trapping.
        if backing.id != u64::MAX {
            let mut this = self.clone();
            if let Err(e) = this.map_guest(gpa, len, backing, Prot::ReadWrite) {
                // Partial failure: unwind the window rather than leave a live memslot
                // over pages nobody filled.
                machine.remove_window(region)?;
                return Err(e);
            }
        }
        Ok(slot)
    }
}

/// Duplicate a borrowed descriptor into an owned one, so the borrow (and the lock it came
/// from) can be released **before** the syscall that uses it runs. This crate is
/// `unsafe_code = "forbid"` — the raw crate is the workspace's only exception — so it goes
/// through safe `std`, which is `dup` underneath.
fn dup_owned(fd: std::os::fd::BorrowedFd<'_>) -> Result<std::os::fd::OwnedFd, RawError> {
    fd.try_clone_to_owned().map_err(|e| RawError::Syscall {
        call: "dup",
        errno: e.raw_os_error(),
    })
}

kayfabe_util::assert_send_sync!(KvmVmm, KvmMachine, AuditReport);

#[cfg(test)]
mod tests {
    use super::*;

    /// ★★ **Which spans of a read-native window are registered read-only** — the caller
    /// half of `KVM_MEM_READONLY`, and the half no other test reaches.
    ///
    /// `kayfabe-linux-raw` proves the *mechanism*: a slot installed read-only faults a
    /// guest store out and keeps the backing intact. Nothing proved that this crate ever
    /// **asks** for it. `a_read_native_overlay_installs_a_read_only_slot_over_the_rounded_write_trap`
    /// counts the slots — three, one per span — which is the split, not the protection: it
    /// passes unchanged if all three spans are registered writable, and a read-native
    /// overlay whose trapped page is plain RAM is §6.1 group 7's rom-device pattern
    /// silently not happening.
    ///
    /// So this asserts the third element of every span, by exact content, over every shape
    /// the rounding produces — trap in the middle, at either end, spanning the whole
    /// window, and straddling a page boundary — plus the structural invariants that make
    /// the table mean something: the spans **tile** the window with no gap and no overlap,
    /// and **exactly one** of them is read-only when a trap was asked for.
    #[test]
    fn a_read_native_windows_trapped_span_is_the_only_read_only_one() {
        let p = HostPageSize::query();
        let b = p.bytes();
        let gpa = 0x1000_0000_u64;
        let len = 4 * b;

        for (what, trap, expected) in [
            ("no write trap at all", None, vec![(0, len, false)]),
            (
                "one byte inside the second page",
                Some(gpa + b + 8..gpa + b + 9),
                vec![(0, b, false), (b, b, true), (2 * b, 2 * b, false)],
            ),
            (
                "one byte at the very start",
                Some(gpa..gpa + 1),
                vec![(0, b, true), (b, 3 * b, false)],
            ),
            (
                "one byte at the very end",
                Some(gpa + len - 1..gpa + len),
                vec![(0, 3 * b, false), (3 * b, b, true)],
            ),
            (
                "the whole window",
                Some(gpa..gpa + len),
                vec![(0, len, true)],
            ),
            (
                "a range straddling a page boundary",
                Some(gpa + b - 1..gpa + b + 1),
                vec![(0, 2 * b, true), (2 * b, 2 * b, false)],
            ),
        ] {
            let spans = memslot_spans(gpa, len, trap.as_ref(), p)
                .unwrap_or_else(|e| panic!("{what}: the spans must be computable ({e:?})"));
            assert_eq!(
                spans, expected,
                "{what}: the trapped span — and ONLY the trapped span — must be registered \
                 read-only; a `false` here maps the guest's rom-device range as plain \
                 writable RAM and every other assertion in this crate still passes"
            );

            // The table has to be a partition of the window, or "which span is read-only"
            // is a statement about a span that does not cover what it claims to.
            let mut at = 0_u64;
            for (offset, span_len, _) in &spans {
                assert_eq!(
                    *offset, at,
                    "{what}: the spans must tile the window with no gap and no overlap \
                     ({spans:?})"
                );
                at += *span_len;
            }
            assert_eq!(at, len, "{what}: the spans must cover the whole window");

            assert_eq!(
                spans.iter().filter(|(_, _, ro)| *ro).count(),
                usize::from(trap.is_some()),
                "{what}: a window with a write trap has EXACTLY one read-only span, and \
                 one without has none ({spans:?})"
            );
        }
    }
}
