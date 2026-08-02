//! ★★ The **safe** half of the hypervisor shim — every decision the adapter makes when a
//! foreign C shim calls into it, expressed without a single address.
//!
//! # Why this module exists at all, and why it is not in [`crate::shim_unsafe`]
//!
//! `l2_qemu_adapter.md` §2.3 draws the seam as *"the C never calls Rust logic and Rust never
//! calls C logic; both call primitives"*. That leaves an unstated question: **where does the
//! translation live?** A status code is not a primitive — somebody has to decide that
//! [`kayfabe_vmm::VmmError::Unsupported`] carrying [`kayfabe_vmm_qemu::BELOW_FLOOR`] is a
//! *refusal to realize* and not a *malformed request*, and that decision is logic.
//!
//! Putting it beside the address arithmetic would make it untestable without a hypervisor,
//! which is the exact trap `l2_qemu_adapter.md` §10's stage table is built to avoid: stages
//! Q0 and Q1 are machine-free *on purpose*, and a translation layer that could only be
//! exercised from inside a hypervisor would quietly move this crate's testable surface to
//! zero.
//!
//! So the split inside the crate mirrors the split between the crates:
//!
//! ```text
//!   C shim  ──▶  shim_unsafe.rs   (addresses, the keyword, ~1 line per call)
//!                      │  &dyn QemuHost, &[BarPlacement], plain integers
//!                      ▼
//!                  shim.rs        (this module — every decision, zero addresses)
//!                      │
//!                      ▼
//!            kayfabe_vmm_qemu::QemuMachine
//! ```
//!
//! Everything here is driven by [`kayfabe_vmm_qemu::mock_host::MockQemuHost`] in
//! `tests/shim_logic.rs`, with no hypervisor present.
//!
//! # ★ The status vocabulary is NARROWER than the error type, deliberately
//!
//! [`Status`] has five variants and [`kayfabe_vmm::VmmError`] has five that do not line up
//! with them. That is not sloppiness: a C caller can act on *"the operator must change the
//! command line"* ([`Status::Busy`]), *"this build can never work"* ([`Status::Unsupported`])
//! and *"we asked for something impossible"* ([`Status::Malformed`]), and cannot act on the
//! difference between a bad guest-physical address and a bad region id. **The diagnostic
//! sentence carries the detail** — [`classify`] returns the error's own `&'static str`
//! wherever it has one, so nothing is lost, it is merely not branched on.

use std::sync::Arc;
use std::time::Instant;

use kayfabe_device::{ChipError, ChipProfile, NanoClock, RamRefused, RegPlane};
use kayfabe_vmm::{BarId, Vmm, VmmError};
use kayfabe_vmm_qemu::host::{BarPlacement, MrHandle, QemuHost, SectionDesc, SectionFacts};
use kayfabe_vmm_qemu::slots::SlotPlane;
use kayfabe_vmm_qemu::{MachineConfig, QemuMachine, QemuVmm};

/// The wire ABI this build speaks.
///
/// ★ It is checked in **both** directions — the C shim refuses an archive whose
/// [`ABI_VERSION`] disagrees, and [`crate::shim_unsafe::kayfabe_shim_realize`] refuses an ops
/// table whose `abi_version` disagrees. One-sided version checks were the exact shape of the
/// hypervisor's own per-build module stamp lesson (`l2_qemu_adapter.md` §2.1): a mismatch
/// that is not refused is a mismatch that is executed.
/// ★ Bumped to **2** at stage Q4, when the register plane's entry points were added. The
/// number is checked in both directions, so a hypervisor built against the ABI-1 header and
/// linked against this archive is a named refusal at realize rather than a call into an
/// entry point that did not exist.
/// ★ Bumped to **3** when [`KayfabeRegAudit`] gained `ptimer_reads`. A field added to a
/// counter structure is exactly the change that would otherwise pass every check and then
/// have the archive write one `u64` past the end of a C caller's allocation: the `sizeof`
/// handshake covers the ops table and the realize configuration, and it does not cover this
/// structure. The version does.
/// ★ Bumped to **4** at stage Q5, for the same reason twice over: two entry points were
/// added (`kayfabe_shim_regs_attach_ram` / `_detach_ram`) and [`KayfabeRegWrite`] grew four
/// fields. The entry points alone would be a link error on a stale shim, which is loud; the
/// structure is the quiet one — an old shim would allocate the ABI-3 layout and this
/// archive would write 32 bytes past the end of it.
///
/// ★ Bumped to **6** at `#102` stage C, for the ABI-3 reason exactly: [`KayfabeRegAudit`]
/// gained `fb_window_reads` / `fb_window_writes`, so an ABI-5 shim would allocate the old
/// layout and this archive would write 16 bytes past the end of it. Nothing but the version
/// stands between those two — the `sizeof` handshake does not cover this structure.
///
///
/// ★ Bumped to **7** when [`KayfabeRegAudit`] gained the object bridge's refusal census
/// (`bridge_refusals`, `bridge_refusal_len`, `bridge_refusal`). Same ABI-3 reason a third
/// time, and the growth is the largest yet — an ABI-6 shim would allocate a structure
/// [`BRIDGE_REFUSAL_SLOTS`] rows short and this archive would write well past the end of
/// it. The `sizeof` handshake still does not cover this structure; the version does.
///
/// ★ Bumped to **8** at `#146`, the BAR0 moving window, and it is the ABI-3 reason a fourth
/// time in **both** structures at once: [`KayfabeRegAudit`] gained six framebuffer counters
/// and [`KayfabeRegWrite`] gained the framebuffer refusal's four fields. An ABI-7 shim would
/// allocate both old layouts and this archive would write past the end of each.
///
/// ★ Bumped to **9** at `#149`, the translated BAR2 window, and it is the ABI-3 reason a
/// fifth time: [`KayfabeRegAudit`] gained five fields (`bar2_reads`, `bar2_writes`,
/// `bar2_faults`, `bar_pde_updates`, `bar2_root_entry`), so an ABI-8 shim would allocate
/// the old layout and this archive would write forty bytes past the end of it. Nothing but
/// the version stands between those two.
///
/// ⊘ [`KayfabeRegWrite`] did **not** grow this time, deliberately: a refused *translated*
/// write carries a **virtual** address, and putting it in a field named `fb_phys` would be
/// the aliasing defect one layer up. The virtual address is the region offset the shim
/// already has, and the sentence crosses in the `fault` pair that already exists.
///
/// ★ Bumped to **10** at `#151`, interrupt delivery, and it is the ABI-3 reason a sixth
/// time in **both** structures at once: [`KayfabeRegAudit`] gained the interrupt tree's
/// three counters and [`KayfabeRegWrite`] gained `raise_cpu_intr`. An ABI-9 shim would
/// allocate both old layouts and this archive would write past the end of each.
///
/// ⚠ `[measured]` — this version check is not a formality, and it fired on this very rung:
/// the first `irq1` boot attempt refused to start with *"this shim speaks wire ABI 10 and
/// the archive it was linked against speaks 9"*, because the header had been bumped and
/// this constant had not. ⊘ Without it the boot would have run, the shim would have read
/// `raise_cpu_intr` out of four bytes the archive never wrote, and the failure would have
/// been an interrupt delivered — or not — at random.
///
/// ★ Bumped to **11** at `execution_plane_increments.md` **E1**, the isolate-plane census,
/// and it is the ABI-3 reason a seventh time: [`KayfabeRegAudit`] gained five fields plus a
/// [`ISOLATE_REFUSAL_LEN`]-byte sentence, so an ABI-10 shim would allocate the old layout
/// and this archive would write past the end of it.
///
/// ⊘ [`KayfabeRegWrite`] did **not** grow: an isolate refusal is a property of the device's
/// forwarding plane over a whole boot, not of any one register write, and putting it on a
/// per-write structure would make an operator read it once per access and still not know
/// how many isolates there were.
///
/// ★ Bumped to **12** at `execution_plane_increments.md` **E2**, the usermode doorbell
/// transport, and it is the ABI-3 reason an eighth time — in **both** structures at once.
/// [`KayfabeRegAudit`] gained three counters, a token and a
/// [`DOORBELL_REFUSAL_LEN`]-byte refusal; [`KayfabeRegWrite`] gained `doorbell`,
/// `doorbell_token` and the `doorbell_kind` pair. An ABI-11 shim would allocate both old
/// layouts and this archive would write past the end of each.
///
/// ★ Bumped to **13** at `#128`, the ABI-3 reason a ninth time: [`KayfabeRegAudit`] gained
/// `ptimer_writes_refused`, so an ABI-12 shim would allocate the old layout and this
/// archive would write eight bytes past the end of it. The version is the only thing
/// standing between those two.
///
/// ★★ **`KayfabeRegWrite` DID grow this time, unlike at E1, and the difference is the
/// point.** An isolate refusal is a property of a whole boot; a doorbell is a property of
/// **one write** — and E2's acceptance is that *this* guest store, at *this* instant,
/// reached the core. A per-boot counter alone cannot be stamped against a timeline the
/// device does not write, and stamping is the whole of the attribution
/// (`a_boolean_witness_cannot_attribute`).
///
/// [`KayfabeRegWrite`]: crate::shim_unsafe::KayfabeRegWrite
pub const ABI_VERSION: u32 = 13;

/// What a shim entry point tells its C caller.
///
/// `#[repr(i32)]` because these values are the FFI contract, not an implementation detail;
/// `kayfabe_shim.h` names the same five numbers, and a test asserts they agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(i32)]
pub enum Status {
    /// The call did what it said.
    Ok = 0,
    /// The host or the OS refused a specific operation. Recoverable in principle.
    Refused = 1,
    /// ★ A conflicting *requirer* is present — `l2_qemu_adapter.md` §8.5's `-EBUSY` arm.
    /// Distinct from [`Status::Refused`] because it is an operator's configuration mistake,
    /// and the two send a reader to different places (`testing_doctrine.md` §2 rule 3).
    Busy = 2,
    /// This machine can never run this device: below the version floor, or not accelerated.
    /// Distinct from [`Status::Refused`] because retrying cannot help.
    Unsupported = 3,
    /// ★ The **call** was wrong, not the machine: a mismatched ABI, an out-of-range register
    /// index, a handle that is not one of ours. Never produced by
    /// [`kayfabe_vmm_qemu::QemuMachine`] — it is the FFI layer's own vocabulary, and it is
    /// separate so that "our C shim has a bug" never reads as "your host refused".
    Malformed = 4,
}

impl Status {
    /// The wire value, for a caller that has to put it in an `int32_t`.
    #[must_use]
    pub fn code(self) -> i32 {
        self as i32
    }
}

/// ★★ Translate the adapter's error into the wire vocabulary **and keep the sentence**.
///
/// The `&'static str` is the point. `kayfabe_vmm_qemu`'s refusals are written as operator
/// prose — [`kayfabe_vmm_qemu::BELOW_FLOOR`] explains *why* the floor exists — and flattening
/// them to a number at the seam would throw away the only part a person reads.
#[must_use]
pub fn classify(e: &VmmError) -> (Status, &'static str) {
    match e {
        VmmError::Unsupported(m) => (Status::Unsupported, m),
        VmmError::HostRefused { what, .. } => (Status::Refused, what),
        VmmError::BadGpa { .. } => (
            Status::Refused,
            "a guest-physical range no region covers as a unit",
        ),
        VmmError::NonRamGpa { .. } => (
            Status::Refused,
            "a guest-physical range that resolves to a device, not to host memory",
        ),
        VmmError::BadSlot(_) => (Status::Refused, "an unknown memory-plane region id"),
    }
}

/// ★★★ [`classify`], plus the one thing realize can recover that the general case cannot.
///
/// # The finding this exists to work around, stated rather than smoothed over
///
/// [`kayfabe_vmm_qemu::host::HostError::Busy`] is a **named variant** — its own rustdoc says
/// so, and says why: it is an operator's configuration mistake and its near neighbour is
/// [`kayfabe_vmm_qemu::host::HostError::Refused`], which `testing_doctrine.md` §2 rule 3
/// requires to stay apart. The adapter's own error translation **flattens it anyway**: it
/// becomes `VmmError::HostRefused { errno: Some(KERNEL_EBUSY) }`, so by the time an error
/// reaches this seam the *class* is gone and only the number survives. The port's trait
/// rustdoc claims the opposite ("carries it out to the caller instead of flattening it to a
/// class"); what it actually carries is the **sentence** and the **number**.
///
/// So the class is reconstructed here, and **only for realize**, because that is the only
/// place the reconstruction is exact. At realize the operations that can refuse are the
/// memslot-ceiling query, the migration blocker and the discard disable, and only the last
/// of those can produce this number — [`kayfabe_vmm_qemu::slots::KERNEL_EBUSY`]'s own
/// documentation names it as that arm. Applying the same rule to a runtime reservation would
/// be wrong: a kernel that returns `EBUSY` for a memslot is not an operator's mistake, and
/// [`classify`] is deliberately left blunt for exactly that reason.
#[must_use]
pub fn classify_realize(e: &VmmError) -> (Status, &'static str) {
    if let VmmError::HostRefused {
        what,
        errno: Some(n),
    } = e
        && *n == kayfabe_vmm_qemu::slots::KERNEL_EBUSY
    {
        return (Status::Busy, what);
    }
    classify(e)
}

/// ★ Map a PCI base-address-register **index** to the port's [`BarId`].
///
/// # Why this refuses rather than saturating
///
/// A PCI device has six base-address registers and this port names three. A C shim that
/// hands us index 5 has a bug in its region table, and the failure we want is a named
/// [`Status::Malformed`] at that moment — not a silent aliasing onto [`BarId::Bar2`], which
/// would make a reservation land in a register the hypervisor may well be backing, defeating
/// [`QemuHost::bar_is_unbacked_reservation`] by arriving with the wrong question.
#[must_use]
pub fn bar_from_index(index: u32) -> Option<BarId> {
    match index {
        0 => Some(BarId::Bar0),
        1 => Some(BarId::Bar1),
        2 => Some(BarId::Bar2),
        _ => None,
    }
}

/// The index a [`BarId`] came from — the inverse of [`bar_from_index`].
#[must_use]
pub fn bar_index(bar: BarId) -> u32 {
    match bar {
        BarId::Bar0 => 0,
        BarId::Bar1 => 1,
        BarId::Bar2 => 2,
    }
}

/// One base-address register as the C shim's realize-time table describes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BarDesc {
    /// PCI base-address-register index, as the shim's region table names it.
    pub index: u32,
    /// The guest-physical base the register is currently programmed at.
    pub base: u64,
    /// The register's length in bytes.
    pub len: u64,
}

/// What realize was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShimConfig {
    /// Whether the machine's RAM was created from a shareable backing
    /// ([`kayfabe_vmm_qemu::MachineConfig::shareable_ram`]).
    pub shareable_ram: bool,
    /// Every base-address register the device realized, in table order.
    pub bars: Vec<BarDesc>,
}

impl ShimConfig {
    /// Typed form, or the reason it cannot be formed.
    ///
    /// # Errors
    /// [`Status::Malformed`] for an out-of-range register index or a duplicate one.
    ///
    /// ★ The **duplicate** arm is the one worth naming. Two rows claiming the same register
    /// would declare the same guest-physical range twice in [`kayfabe_vmm::GuestRamMap`], and
    /// the second declaration is not guaranteed to be the one that refuses — so the failure
    /// would surface later, somewhere else, as a range that resolves to the wrong length.
    /// `l2_qemu_adapter.md` §3.3's whole argument is that the region table is *the*
    /// enumeration; a table that can contradict itself is not one.
    pub fn placements(&self) -> Result<Vec<BarPlacement>, (Status, &'static str)> {
        let mut out: Vec<BarPlacement> = Vec::with_capacity(self.bars.len());
        for b in &self.bars {
            let Some(bar) = bar_from_index(b.index) else {
                return Err((
                    Status::Malformed,
                    "a base-address-register index this port does not name; the shim's region \
                     table and this port disagree about how many registers the device has",
                ));
            };
            if out.iter().any(|p| p.bar == bar) {
                return Err((
                    Status::Malformed,
                    "two rows of the shim's region table claim the same base-address \
                     register; the table is meant to be the one enumeration and cannot \
                     contradict itself",
                ));
            }
            out.push(BarPlacement {
                bar,
                base: b.base,
                len: b.len,
            });
        }
        Ok(out)
    }
}

/// One topology section, as a listener callback reports it, in plain integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionWire {
    /// Opaque backend-scoped region identity.
    pub mr: u64,
    /// Guest-physical base of the section.
    pub gpa: u64,
    /// Length in bytes.
    pub len: u64,
    /// Byte offset of the section's first byte within the region's backing.
    pub offset_within_region: u64,
    /// The region reports itself as memory.
    pub is_ram: bool,
    /// The region is a *device* memory region — direct-access-shaped, possibly registers.
    pub is_ram_device: bool,
    /// Reads are direct, writes go to callbacks.
    pub is_rom_device: bool,
    /// The section is read-only.
    pub readonly: bool,
    /// The section is non-volatile.
    pub nonvolatile: bool,
}

impl SectionWire {
    /// The typed form the adapter's classifier consumes.
    #[must_use]
    pub fn desc(self) -> SectionDesc {
        SectionDesc {
            mr: MrHandle(self.mr),
            gpa: self.gpa,
            len: self.len,
            offset_within_region: self.offset_within_region,
            facts: SectionFacts {
                is_ram: self.is_ram,
                is_ram_device: self.is_ram_device,
                is_rom_device: self.is_rom_device,
                readonly: self.readonly,
                nonvolatile: self.nonvolatile,
            },
        }
    }
}

/// ★ The counters a C caller can read back, so an acceptance test outside this process can
/// assert on something other than an exit code.
///
/// `#[repr(C)]` and `u64`-only: it is copied into a C structure field for field, and a
/// layout with no addresses in it cannot carry a lifetime across the seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct KayfabeAudit {
    /// Reservations currently mapped.
    pub live_windows: u64,
    /// Memslots currently installed in the kernel.
    pub live_memslots: u64,
    /// Cumulative memslot installs.
    pub memslot_installs: u64,
    /// ★★★ Regions handed to the *hypervisor* to back. Must be zero forever —
    /// `host_execution_plane.md` §1 as a single number.
    pub regions_published: u64,
    /// Topology sections the listener added.
    pub topology_adds: u64,
    /// Topology sections the listener removed.
    pub topology_dels: u64,
    /// Times the latched base-address register was re-read and compared — the non-vacuity
    /// half of the move detector.
    pub bar_base_checks: u64,
    /// Times a register was found somewhere other than where it was latched.
    pub bar_moves_detected: u64,
    /// Memory-plane operations refused because the device was already unrealized.
    pub ops_refused_after_unrealize: u64,
}

/// The realized device — what the C shim holds behind its opaque handle.
#[derive(Debug)]
pub struct Shim {
    machine: QemuMachine,
}

impl Shim {
    /// `l2_qemu_adapter.md` §8.1's realize, entered from a foreign shim.
    ///
    /// # Errors
    /// [`Status::Malformed`] if the register table cannot be formed; otherwise whatever
    /// [`QemuMachine::realize`] refused, [`classify`]-ed.
    ///
    /// ★ **No reservation is installed here, and that is a decision.** At the moment a PCI
    /// device realizes, its base-address registers are unprogrammed — firmware assigns them
    /// afterwards — so a realize-time reservation would have to invent a base. The C
    /// artifact reaches the same conclusion from the other direction and installs lazily, on
    /// first use (`C: src/qemu/nvkvm_mmap_host.c:241-243`). Reservations are installed from
    /// [`Shim::install_window`] once a base exists, which `host_execution_plane.md` §1.6
    /// finding 1 makes legal at any time.
    pub fn realize(
        cfg: &ShimConfig,
        host: Arc<dyn QemuHost>,
        slots: Arc<dyn SlotPlane>,
    ) -> Result<Shim, (Status, &'static str)> {
        let bars = cfg.placements()?;
        let machine = QemuMachine::realize(
            MachineConfig {
                shareable_ram: cfg.shareable_ram,
                bars,
                windows: Vec::new(),
                traps: Vec::new(),
            },
            host,
            slots,
        )
        .map_err(|e| classify_realize(&e))?;
        Ok(Shim { machine })
    }

    /// The realized machine, for a caller that needs more than this seam exposes.
    #[must_use]
    pub fn machine(&self) -> &QemuMachine {
        &self.machine
    }

    /// §8.3's unrealize.
    pub fn unrealize(&self) {
        self.machine.unrealize();
    }

    /// A reservation over a guest-physical range, once a base-address register has one.
    ///
    /// # Errors
    /// [`classify`]-ed. The arm that matters is
    /// [`kayfabe_vmm_qemu::WINDOW_IN_A_BACKED_BAR`]: a register the hypervisor backs gets a
    /// hypervisor-managed memslot of its own over the same range as ours, and only one of the
    /// two can win.
    pub fn install_window(&self, gpa: u64, len: u64) -> Result<u64, (Status, &'static str)> {
        self.machine
            .install_ram_window(gpa, len)
            .map(|r| r.0)
            .map_err(|e| classify(&e))
    }

    /// The listener's add callback.
    ///
    /// # Errors
    /// [`classify`]-ed.
    pub fn region_add(&self, s: SectionWire) -> Result<(), (Status, &'static str)> {
        self.machine.region_add(s.desc()).map_err(|e| classify(&e))
    }

    /// The listener's delete callback.
    pub fn region_del(&self, gpa: u64, len: u64) {
        self.machine.region_del(gpa, len);
    }

    /// ★ The *preventer*: what a configuration-space write override calls before letting a
    /// base-address-register write through.
    ///
    /// # Errors
    /// [`Status::Malformed`] for a register index this port does not name;
    /// [`Status::Unsupported`] naming [`kayfabe_vmm_qemu::BAR_MOVED_UNDER_US`] once a memslot
    /// has been installed into that register.
    pub fn bar_move_requested(&self, index: u32) -> Result<(), (Status, &'static str)> {
        let Some(bar) = bar_from_index(index) else {
            return Err((
                Status::Malformed,
                "a base-address-register index this port does not name",
            ));
        };
        self.machine
            .bar_move_requested(bar)
            .map_err(|e| classify(&e))
    }

    /// The *detector*: what a configuration-space write override calls afterwards.
    ///
    /// # Errors
    /// [`Status::Malformed`] for a register index this port does not name. The move itself is
    /// not an error here — it is recorded in [`KayfabeAudit::bar_moves_detected`], because this
    /// arm exists precisely for the case the preventer did not cover.
    pub fn note_bar_mapping(
        &self,
        index: u32,
        base: Option<u64>,
    ) -> Result<(), (Status, &'static str)> {
        let Some(bar) = bar_from_index(index) else {
            return Err((
                Status::Malformed,
                "a base-address-register index this port does not name",
            ));
        };
        self.machine.note_bar_mapping(bar, base);
        Ok(())
    }

    /// The counters, in the wire shape.
    ///
    /// ★★★ **The source is DESTRUCTURED with no `..`, and that is the whole design.**
    /// `AuditReport` carries thirty-five counters and this wire value carries nine. Written
    /// as `a.field` nine times, the other twenty-six are invisible *and so is the
    /// twenty-seventh*: a counter added to the memory plane reaches nobody outside the
    /// process, and no test in this repository can go red about it — the exact
    /// shrinking-universe failure the `#130` recovery work was written to end. Binding
    /// every field by name turns "should this cross the seam?" into `error[E0027]` on the
    /// commit that adds it.
    ///
    /// ⊘ The twenty-six `_`-bound names are **not** a claim that they do not matter. They
    /// are peaks, depth witnesses and internal accounting whose consumer is
    /// [`crate::shim::Shim`]'s own tests rather than the C shell; if one of them ever needs
    /// to reach an operator, the wire struct and [`ABI_VERSION`] move together.
    ///
    /// ★ The four `plan_*` / `*_plan_reservations` counters (#145) are adjudicated the same
    /// way and stay inside: three of them can only move under a genuine two-thread race on
    /// one guest-physical range, which is a defect in whatever is calling `map_guest` and
    /// not a device condition an operator can act on, and the fourth (`live_plan_reservations`)
    /// is an invariant that must read zero at quiescence — a thing to ASSERT, not to report.
    #[must_use]
    pub fn audit(&self) -> KayfabeAudit {
        // ★★★ EXHAUSTIVE. The missing `..` is load-bearing — see this method's docs.
        let kayfabe_vmm_qemu::AuditReport {
            live_windows,
            live_memslots,
            memslot_installs,
            regions_published,
            topology_adds,
            topology_dels,
            bar_base_checks,
            bar_moves_detected,
            ops_refused_after_unrealize,
            live_placements: _,
            window_bytes: _,
            peak_windows: _,
            peak_placements: _,
            placements_made: _,
            peak_memslots: _,
            slot_numbers_recycled: _,
            accessor_ranked_depth: _,
            syscall_ranked_depth: _,
            own_copy_leaf_depth_max: _,
            host_copy_leaf_depth_min: _,
            view_leaf_depth_max: _,
            accesses_served: _,
            accesses_refused: _,
            host_refusals: _,
            r5_revalidation_failures: _,
            topology_generation: _,
            irqs_raised: _,
            window_releases_deferred: _,
            window_mappings_released: _,
            live_plan_reservations: _,
            peak_plan_reservations: _,
            plan_conflicts: _,
            plan_reservations_abandoned: _,
        } = self.machine.audit();
        KayfabeAudit {
            live_windows,
            live_memslots,
            memslot_installs,
            regions_published,
            topology_adds,
            topology_dels,
            bar_base_checks,
            bar_moves_detected,
            ops_refused_after_unrealize,
        }
    }
}

// =====================================================================================
// The register plane (stage Q4) — the safe half
// =====================================================================================

/// ★★★ **Stage Q5: the register plane's guest-RAM port**, over the realized memory plane.
///
/// # What this joins, and why it needed a type
///
/// The two planes are separate objects with separate lifetimes — the register plane is
/// built at the device's `realize`, the memory plane only once a base-address register has
/// been programmed — so `kayfabe_device::RegPlane` is constructed with
/// [`kayfabe_device::RefusingRam`] and the shell installs the real port later, through
/// [`kayfabe_device::RegPlane::set_ram`]. This is the thing it installs.
///
/// # ★★★ What it bought, MEASURED
///
/// Run of record, task #124: 2026-07-31, at commit `3fb3fca`, on the QEMU 10.2.4 + KVM
/// bench (`-device nvkvm-gpu`, 3 vCPU / 2 GiB), guest Ubuntu kernel 6.8.0-136 with the
/// **stock, unpatched** open NVIDIA 580.159.04 module, driven by `nvidia-smi`.
///
/// Before this port, the guest's GSP bring-up ended at
/// `GspStatusQueueInit: msgqRxLink failed: -7` followed by
/// `_kgspBootGspRm: unexpected WPR2 already up`, because the LibOS boot-args write at
/// `+0x110044` was refused `GspFault::GuestRam`. With it, the register trace shows that
/// same write accepted (`MAILBOX0 = 0x20259000`, `MAILBOX1 = 0`) and **neither NVRM line
/// appears at all**; the device's own audit closed the boot at
/// *"faults 0, guest-RAM refusals 0"* over 2 813 reads and 870 writes, and the driver ran
/// on into `RmInitAdapter`'s device pre-initialisation.
///
/// ★ Where it stops now is **one layer up and nothing to do with memory**: the guest asks
/// the GSP for its engine-info and interrupt tables, the command policy in force is
/// `kayfabe_gsp::EchoOk`, and an echoed reply carries no table — so RM reports
/// `pEngineInfo->engineInfoList != NULL` failing, `NV_ERR_NO_MEMORY` out of
/// `kfifoGetHostDeviceInfoTable_HAL`, and bails. That is a *protocol* wall, which is the
/// shape a memory wall turns into once memory works.
///
/// # ★★ It is still a REFUSER, and that is the whole design
///
/// [`kayfabe_vmm::Vmm::gpa_read`]/[`kayfabe_vmm::Vmm::gpa_write`] resolve through
/// [`kayfabe_vmm::GuestRamMap`], which proves a range lies wholly inside one region
/// **declared as memory** before anything is copied, and refuses otherwise. So the
/// addresses this port serves are exactly the ones the hypervisor's own topology listener
/// reported as RAM, and:
///
/// - an address nothing backs is [`kayfabe_vmm::VmmError::BadGpa`] — refused;
/// - an address that resolves to a *device* register window (another device's BAR, the
///   platform's MMIO, **our own trapped registers**) is
///   [`kayfabe_vmm::VmmError::NonRamGpa`] — refused, and separately, because serving it
///   would mean re-entering the register plane through the memory plane.
///
/// Neither ever reads as zero. That is the property the previous stage's named refusal
/// bought and the property this stage must not spend: a plausible answer to an address we
/// do not back is how a guest is sent into a loop nobody can see.
///
/// ★ **The reason survives the crossing.** `RamRefused` carries a `why`, and it is filled
/// from the error's own variant rather than `map_err(|_| …)`-ed away — the two refusals
/// above are near neighbours by address and completely different findings, and a port that
/// reported them identically would cost a boot to tell apart.
///
/// # Cheap to hold
///
/// [`kayfabe_vmm_qemu::QemuVmm`] is a handle onto the machine's plane, not a copy of it,
/// so installing one costs an `Arc` clone and the register plane's lock is the only
/// serialization added.
#[derive(Debug)]
pub struct MachineRam {
    vmm: QemuVmm,
}

impl MachineRam {
    /// A port onto one realized machine's guest memory.
    #[must_use]
    pub fn new(vmm: QemuVmm) -> MachineRam {
        MachineRam { vmm }
    }

    /// The refusing sentence for one adapter error, **by variant**.
    ///
    /// ★ Every arm is written out. A catch-all would compile and would be the exact
    /// `map_err(|_| …)` the GPA-accessor gate's failure text forbids one crate over: the
    /// discarded variant is the finding.
    fn why(e: &VmmError) -> &'static str {
        match e {
            VmmError::BadGpa { .. } => {
                "no guest-physical region covers that range as a unit; nothing is there, so \
                 there is nothing to read and answering zero would be an invention"
            }
            VmmError::NonRamGpa { .. } => {
                "that range resolves to a device register window, not to guest memory; the \
                 emulated GSP may only follow the guest's pointers into RAM"
            }
            VmmError::BadSlot(_) => {
                "the region the range resolved into is no longer installed; the memory plane \
                 retired it under us"
            }
            VmmError::Unsupported(m) => m,
            VmmError::HostRefused { what, .. } => what,
        }
    }
}

impl kayfabe_device::GuestRam for MachineRam {
    fn read(&mut self, gpa: u64, buf: &mut [u8]) -> Result<(), RamRefused> {
        let len = buf.len();
        self.vmm.gpa_read(gpa, buf).map_err(|e| RamRefused {
            gpa,
            len,
            why: MachineRam::why(&e),
        })
    }

    fn write(&mut self, gpa: u64, bytes: &[u8]) -> Result<(), RamRefused> {
        self.vmm.gpa_write(gpa, bytes).map_err(|e| RamRefused {
            gpa,
            len: bytes.len(),
            why: MachineRam::why(&e),
        })
    }
}

/// The driver version the emulated GSP answers as.
///
/// ★★ **Hardcoded, and named here as the one place a bolt-on starts.** The device has no
/// way to *ask* which driver a guest is about to load — the answer is only knowable from
/// traffic the guest has not sent yet — so a version must be chosen before the first
/// register is answered. This is the bench's version, which is the version the whole port
/// is derived against ([`kayfabe_abi::versions::BENCH_DRIVER`]).
///
/// What makes it a bolt-on point rather than a wall: [`kayfabe_device::abi::gsp_abi_for`]
/// takes any version, refuses below its floor rather than nearest-neighbouring, and the
/// table it reads is already keyed on the full `major.minor.patch`. Supporting a second
/// guest driver is a table row plus a way to select it — a device property, or the
/// version-detection traffic itself — and no code below this line changes.
pub const GUEST_DRIVER: kayfabe_abi::DriverVersion = kayfabe_abi::versions::BENCH_DRIVER;

/// What a chip's device must put in configuration space, in the wire shape.
///
/// `#[repr(C)]` for the same reason [`KayfabeAudit`] is: it is copied into a C structure
/// field for field. Field order is fixed for natural alignment so the two spellings cannot
/// differ by padding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct KayfabeChipIdentity {
    /// Must equal [`ABI_VERSION`].
    pub abi_version: u32,
    /// Must equal `size_of::<KayfabeChipIdentity>()`.
    pub struct_size: u32,
    /// The register aperture's length, per the chip table.
    pub regs_aperture_len: u64,
    /// ★★ The framebuffer window's length, per the chip table — the **same** number the
    /// emulated GSP answers `NV2080_CTRL_CMD_BUS_GET_PCI_BAR_INFO` with. A shell whose own
    /// registration differs must refuse to realize; see `kayfabe_abi::pcibars`.
    pub fb_window_len: u64,
    /// The instance/`BAR2` window's length, likewise.
    pub inst_window_len: u64,
    /// `(base << 16) | (sub << 8) | prog_if`.
    pub class_code: u32,
    /// PCI vendor id.
    pub vendor_id: u16,
    /// PCI device id.
    pub device_id: u16,
    /// Subsystem vendor id.
    pub subsystem_vendor_id: u16,
    /// Subsystem device id.
    pub subsystem_id: u16,
    /// How many message-signalled vectors to offer.
    pub msix_vectors: u16,
    /// PCI revision id.
    pub revision: u8,
    /// Padding, so the layout is the same on every ABI that cares.
    pub reserved: u8,
}

/// How many distinct unserviced commands [`KayfabeRegAudit`] carries.
///
/// ★ A fixed array rather than a caller-supplied buffer: the shim's whole discipline is
/// that the hypervisor passes no pointer it has to size. `unserviced_len` reports the
/// truth even when it exceeds this, so a full array is never mistaken for a complete list.
pub const UNSERVICED_SLOTS: usize = 32;

/// The low half of a packed [`KayfabeRegAudit::unserviced`] entry when the function was not
/// a `GSP_RM_CONTROL` — i.e. there is no control command to name.
///
/// ⊘ Deliberately not `0`: `0` is a legal `NV*_CTRL_CMD_*` value shape and *"we could not
/// decode it"* must not read as *"command zero"*.
pub const UNSERVICED_NO_CMD: u32 = 0xFFFF_FFFF;

/// How many distinct bridge-refusal tags [`KayfabeRegAudit`] carries.
///
/// ★ Sized against the **whole** closed set: `kayfabe_rmrpc::BridgeRefusal` has fewer than
/// this many `FaultTag`s, so a boot cannot overflow it. `bridge_refusal_len` reports the
/// truth regardless, for the same reason [`UNSERVICED_SLOTS`]'s does.
pub const BRIDGE_REFUSAL_SLOTS: usize = 32;

/// How many bytes of a refusal's [`kayfabe_trace::FaultTag`] [`KayfabeBridgeRefusal`] holds.
///
/// ★★ The name crosses the seam **by value**, not as a pointer, and that is not a style
/// choice: the host-pointer gate forbids a host address in any file that is not
/// `*_unsafe.rs`, and smuggling one through as a `u64` would defeat the gate rather than
/// satisfy it. Copying 64 bytes once per boot teardown is free, and it means the C shell
/// prints a **name** without this crate publishing a second table of numeric codes that
/// could drift from `BridgeRefusal::fault_tag`'s match.
pub const BRIDGE_REFUSAL_TAG_LEN: usize = 64;

/// How many bytes of the isolate plane's refusal sentence [`KayfabeRegAudit`] carries (E1).
///
/// ★ Longer than [`BRIDGE_REFUSAL_TAG_LEN`] because it is not a tag: a spawn failure's text
/// is `format!`ed from the host's own error at the failing step — *"spawning the embedded
/// isolate: …"*, *"worker socketpair: …"* — and truncating it to a tag width would cut off
/// exactly the `errno` an operator acts on. It crosses **by value**, never as a pointer,
/// for [`BRIDGE_REFUSAL_TAG_LEN`]'s reason.
pub const ISOLATE_REFUSAL_LEN: usize = 192;

/// [`KayfabeRegAudit::isolate_refusal_kind`] — no live isolate refuses.
///
/// ⊘ Deliberately the zero value, and it is the only one that is safe to be zero: an
/// all-zero audit means "nothing happened", and "nothing refused" is a true reading of
/// that. The two *kinds* below are non-zero so that a struct the archive never wrote can
/// never be read as a specific diagnosis.
pub const ISOLATE_REFUSAL_NONE: u64 = 0;
/// [`KayfabeRegAudit::isolate_refusal_kind`] — `kayfabe_isolate::RefusalKind::NoPlane`:
/// this build has no forwarding plane and none was attempted.
pub const ISOLATE_REFUSAL_NO_PLANE: u64 = 1;
/// [`KayfabeRegAudit::isolate_refusal_kind`] — `kayfabe_isolate::RefusalKind::SpawnFailed`:
/// ★ a real plane was asked for and could not be built. **The one that means the host is
/// wrong.**
pub const ISOLATE_REFUSAL_SPAWN_FAILED: u64 = 2;

/// ★★★ **E2** — how many bytes of a doorbell refusal's **sentence** [`KayfabeRegAudit`]
/// carries, and how many of its **kind**.
///
/// Two arrays and not one, for the reason [`KayfabeIsolateRefusal`] separates its `kind`
/// from its `text`: the kind is a stable name a check may branch on
/// (`FwdFault::MalformedToken` ≠ `FwdFault::UnknownVchid` — two different diagnoses with
/// two different fixes), and the sentence is the variant's payload, which is prose. A
/// single blob would make the only machine-readable half a substring search.
pub const DOORBELL_REFUSAL_LEN: usize = 192;
/// How many bytes of a doorbell refusal's **kind** the audit carries.
///
/// ★ [`BRIDGE_REFUSAL_TAG_LEN`]'s width and for its reason: a `FaultTag` is a
/// `&'static str` from a fixed finite set, and 64 bytes covers every one of them with room
/// to spare.
pub const DOORBELL_KIND_LEN: usize = 64;

/// ★★★ **E2 — a refused guest doorbell, in the wire shape**: the fault's stable kind and
/// one sentence.
///
/// Mirrors [`KayfabeIsolateRefusal`]'s shape (NUL-**padded**, explicit lengths, `Default`
/// written out because the arrays are wider than the derive covers) for its reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct KayfabeDoorbellRefusal {
    /// The fault's stable name, NUL-padded — e.g. `FwdFault::UnknownVchid`.
    pub kind: [u8; DOORBELL_KIND_LEN],
    /// The sentence's bytes, NUL-padded.
    pub text: [u8; DOORBELL_REFUSAL_LEN],
    /// How many bytes of [`Self::kind`] are the name.
    pub kind_len: u64,
    /// How many bytes of [`Self::text`] are the sentence.
    pub len: u64,
    /// ⊘ **Non-zero exactly when a doorbell was refused**, and the validity flag for
    /// everything above: a kind of length zero is not a reserved value (an archive that
    /// never wrote this struct also leaves it zero), so a reader needs a field that is
    /// zero *only* in the never-happened case. This is it.
    pub present: u64,
}

impl Default for KayfabeDoorbellRefusal {
    fn default() -> KayfabeDoorbellRefusal {
        KayfabeDoorbellRefusal {
            kind: [0; DOORBELL_KIND_LEN],
            text: [0; DOORBELL_REFUSAL_LEN],
            kind_len: 0,
            len: 0,
            present: 0,
        }
    }
}

/// One row of the bridge's refusal census: a `FaultTag`, and how many carried it.
///
/// ★★★ **The instrument boot `alloc1` did without.** `[measured]` 2026-08-01, boot
/// `alloc1` at **rev `2ced035`** (`docs/design/boot_measured_2026_08_01.md` §6): a refusal
/// raised *inside* the bridge answers the guest's command, so it never reaches the
/// unserviced ledger, and the only evidence it happened was `fn 103` being **absent** from
/// a list of six. Diagnosis-by-absence is exactly what the ledger exists to abolish. See
/// `kayfabe_rmrpc::SharedRefusalCensus` for why the obstruction was ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct KayfabeBridgeRefusal {
    /// The tag's bytes, NUL-padded. Not NUL-*terminated* when the name is exactly
    /// [`BRIDGE_REFUSAL_TAG_LEN`] long, so the C side prints with an explicit precision
    /// rather than trusting a terminator.
    pub tag: [u8; BRIDGE_REFUSAL_TAG_LEN],
    /// How many bytes of [`Self::tag`] are the name. Never more than the array.
    pub tag_len: u64,
    /// How many refusals carried it.
    pub count: u64,
}

impl Default for KayfabeBridgeRefusal {
    fn default() -> KayfabeBridgeRefusal {
        KayfabeBridgeRefusal {
            tag: [0; BRIDGE_REFUSAL_TAG_LEN],
            tag_len: 0,
            count: 0,
        }
    }
}

/// ★★★ **E1 — the isolate plane's refusal, in the wire shape.**
///
/// One sentence and its **kind**, and the kind is the point: a check keyed on a word is
/// satisfied by writing the word, so the C shell branches on
/// [`ISOLATE_REFUSAL_SPAWN_FAILED`] rather than grepping the prose for "spawn".
///
/// Mirrors [`KayfabeBridgeRefusal`]'s shape (NUL-**padded**, explicit length, `Default`
/// written out because the array is wider than the derive covers) for its reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct KayfabeIsolateRefusal {
    /// The sentence's bytes, NUL-padded. Not NUL-*terminated* when the text exactly fills
    /// the array, so the C side prints with an explicit precision.
    pub text: [u8; ISOLATE_REFUSAL_LEN],
    /// How many bytes of [`Self::text`] are the sentence. Never more than the array; a
    /// longer sentence is **truncated**, which is visible because this stops short of the
    /// full text rather than silently re-wrapping.
    pub len: u64,
    /// [`ISOLATE_REFUSAL_NONE`], [`ISOLATE_REFUSAL_NO_PLANE`] or
    /// [`ISOLATE_REFUSAL_SPAWN_FAILED`].
    pub kind: u64,
}

impl Default for KayfabeIsolateRefusal {
    fn default() -> KayfabeIsolateRefusal {
        KayfabeIsolateRefusal {
            text: [0; ISOLATE_REFUSAL_LEN],
            len: 0,
            kind: ISOLATE_REFUSAL_NONE,
        }
    }
}

/// The register plane's counters, in the wire shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct KayfabeRegAudit {
    /// Register reads dispatched into the plane.
    pub reads: u64,
    /// Register writes dispatched into the plane.
    pub writes: u64,
    /// Reads answered from the chip's silicon constants.
    pub boot_reg_reads: u64,
    /// Reads answered from the free-running nanosecond counter.
    pub ptimer_reads: u64,
    /// ★ Writes to the free-running nanosecond counter, refused by name (`#128`).
    pub ptimer_writes_refused: u64,
    /// Reads answered from the ROM window.
    pub rom_reads: u64,
    /// Reads answered by the GSP register model.
    pub gsp_reads: u64,
    /// Writes the GSP register model claimed.
    pub gsp_writes: u64,
    /// ★ Reads no source claimed, answered with a defaulted zero.
    pub unclaimed_reads: u64,
    /// Writes no source claimed, dropped.
    pub unclaimed_writes: u64,
    /// ★★★ Reads that landed in a framebuffer window — device memory, not a register.
    /// Carried across the seam because this is the only channel the C shell reads, and a
    /// boot that scribbles at the framebuffer must be able to say so from outside the
    /// process. See `kayfabe_device::FbWindow`.
    pub fb_window_reads: u64,
    /// Writes that landed in a framebuffer window and were therefore **dropped**.
    pub fb_window_writes: u64,
    /// ★★★ `#146` — reads **served** from the device's framebuffer through the BAR0
    /// moving window.
    pub fb_reads: u64,
    /// ★★★ `#146` — writes that **landed** in the device's framebuffer.
    pub fb_writes: u64,
    /// ★★★ `#146` — framebuffer accesses the store **refused, by name**.
    ///
    /// ⊘ The number an operator reads to answer *"did this boot drop a framebuffer
    /// write?"* — the question `kbusVerifyBar2` used to be the only answer to, hundreds of
    /// operations after the fact.
    pub fb_refusals: u64,
    /// ★★★ `#149` — reads **served through the GMMU** from the translated instance/`BAR2`
    /// window.
    pub bar2_reads: u64,
    /// ★★★ `#149` — writes **served through the GMMU** into it.
    pub bar2_writes: u64,
    /// ★★★ `#149` — translated accesses this port **refused, by name**: an unrooted
    /// aperture, an unmapped virtual address, or a leaf in an aperture it cannot serve.
    ///
    /// ⊘ The number that distinguishes *"the walk never happened"* from *"the walk
    /// happened and landed somewhere else"*. `kbusVerifyBar2`'s `NV_ERR_MEMORY_ERROR`
    /// cannot tell those apart; this and `bar2_writes` together can.
    pub bar2_faults: u64,
    /// ★★★ `#149` — how many bus-aperture roots the guest published (`UPDATE_BAR_PDE`),
    /// and how many bodies were refused, packed `updates << 32 | refusals`.
    ///
    /// ⚠ Packed rather than two fields because the guest **ignores this command's
    /// status**, so both halves are only ever read together: *"did the root arrive, and
    /// did we take it?"* is one question.
    pub bar_pde_updates: u64,
    /// ★★★ `#149` — the BAR2 root entry the guest published, verbatim, or `0` if none.
    ///
    /// ⊘ Zero is ambiguous **on purpose and it is disambiguated by
    /// [`KayfabeRegAudit::bar_pde_updates`]**: the guest really does publish `0` to unroot
    /// the aperture on teardown (`ogkm-580: kern_bus_gm107.c:2137`), so the value alone
    /// cannot say whether one arrived. The count can.
    pub bar2_root_entry: u64,
    /// `#146` — reads of `NV_PBUS_BAR0_WINDOW` itself.
    pub bar0_window_reads: u64,
    /// `#146` — writes to `NV_PBUS_BAR0_WINDOW`, i.e. the guest re-pointing its window.
    pub bar0_window_writes: u64,
    /// `#146` — how many bytes of framebuffer the store is holding for this device life.
    pub fb_resident_bytes: u64,
    /// Faults the emulated GSP raised.
    pub faults: u64,
    /// Guest-RAM accesses the plane's RAM port refused.
    pub ram_refusals: u64,
    /// Times a write asked for the status-queue interrupt to be announced.
    pub irq_requests: u64,
    /// `#151`: accesses to the `CPU_INTR` tree, reads and writes together.
    pub cpu_intr_accesses: u64,
    /// `#151`: `CPU_INTR_LEAF_TRIGGER` writes that latched a vector — the number of
    /// message-signalled interrupts the register plane asked the shell to deliver.
    pub cpu_intr_raises: u64,
    /// `#151`: of those, how many real silicon would have masked. See
    /// `kayfabe_device::cpuintr::TriggerOutcome::would_be_masked`.
    pub cpu_intr_masked: u64,
    /// Commands decoded off the guest's command queue.
    pub commands: u64,
    /// ★★ Of those, the ones **no policy answered**, and which the emulated GSP therefore
    /// refused by name. Includes repeats and anything past [`UNSERVICED_SLOTS`].
    pub commands_unserviced: u64,
    /// How many entries of [`KayfabeRegAudit::unserviced`] are populated.
    pub unserviced_len: u64,
    /// ★★★ **The list a boot is worth.** Distinct unserviced commands, packed
    /// `(function << 32) | cmd`, with [`UNSERVICED_NO_CMD`] in the low half for a function
    /// that is not a `GSP_RM_CONTROL` (or whose header would not decode).
    ///
    /// It is in the counters struct rather than behind a second entry point on purpose:
    /// one call, one `#[repr(C)]` value, no second pointer for the shim to get wrong. See
    /// `kayfabe_device::unserviced` for why the guest cannot be asked this question — RM
    /// logs `NV_ERR_NOT_SUPPORTED` quietly, so without this the list costs one boot per
    /// entry.
    pub unserviced: [u64; UNSERVICED_SLOTS],
    /// ★★★ Refusals raised **inside the object bridge**, across every tag.
    ///
    /// ⊘ Disjoint from [`Self::commands_unserviced`] by construction, and the disjointness
    /// is the whole point: a bridge refusal *answers* the command (with a non-zero
    /// `rpc_result`), so the chain's terminal ledger never sees it. Before this field the
    /// two together did not cover the command stream, and the gap was invisible.
    pub bridge_refusals: u64,
    /// How many entries of [`KayfabeRegAudit::bridge_refusal`] are populated.
    pub bridge_refusal_len: u64,
    /// The census, one row per tag, in tag order.
    pub bridge_refusal: [KayfabeBridgeRefusal; BRIDGE_REFUSAL_SLOTS],
    /// ★★★ **E1/E0b — how many isolates this device has ever materialized.**
    ///
    /// ⊘ **Zero is a finding, not a blank.** Since E0b the isolate is spawned by a *guest*
    /// RM event rather than by `Gpu::realize`, so `0` means the guest never got as far as
    /// an accepted `GSP_RM_ALLOC` — a completely different diagnosis from "it spawned and
    /// refuses", and one that was the same silence before this number existed.
    ///
    /// ⊘ And it is **not** the instrument that attributes a spawn to the guest: it is
    /// written by the code under test. `scripts/bench/e0_isolate_witness.sh` is, because
    /// it stamps host `/proc` sightings against a timeline this device does not write.
    pub isolates_materialized: u64,
    /// How many isolates the device holds right now (live procs, the system proc, and
    /// retired-but-unreaped procs).
    pub isolates_live: u64,
    /// Of those, how many refuse because this build has **no forwarding plane**
    /// (`KAYFABE_ISOLATES` unset or `stillborn`). Expected, not a fault.
    pub isolates_no_plane: u64,
    /// ★ Of those, how many refuse because a real plane was asked for and **could not be
    /// built**. The number that means the host is wrong — `bench_rebuild_notes.md` §5 row
    /// 7 is exactly the fact that this used to be indistinguishable from the line above.
    pub isolates_spawn_failed: u64,
    /// One refusal sentence, and its kind. `SpawnFailed` outranks `NoPlane` when both are
    /// present: a plane that broke is more actionable than one that was never installed.
    pub isolate_refusal: KayfabeIsolateRefusal,
    /// ★★★ **E2** — guest MMIO writes that landed on the usermode doorbell register, i.e.
    /// work-submit tokens the guest rang. See `kayfabe_device::Counters::doorbells`: this
    /// is the **arrival** count and it is not reducible by anything the core decides.
    pub doorbells: u64,
    /// Of those, the ones the core **served** — a `DoorbellOutcome` came back.
    pub doorbells_served: u64,
    /// Of those, the ones the core **refused, by name**.
    ///
    /// ★ `doorbells == doorbells_served + doorbells_refused`, always. Neither can absorb
    /// the other, so *"the transport works and the routing does not"* is a readable state
    /// rather than a silence — which is exactly what E2 expects to see before E5.
    pub doorbells_refused: u64,
    /// The last token the guest stored, and its own validity flag below.
    pub doorbell_last_token: u64,
    /// ⊘ Non-zero iff [`Self::doorbell_last_token`] means anything.
    ///
    /// ⚠ **Two fields for one fact, and the second is not redundant**: token `0` is a
    /// legal work-submit token (runlist 0, channel 0), so a single field could not tell
    /// *"rang channel 0"* from *"never rang"*. The same argument `fb_landed_valid` already
    /// carries one aperture over.
    pub doorbell_last_token_valid: u64,
    /// The **first** doorbell the core refused — kind and sentence.
    ///
    /// ⊘ First, not last: a flood of identical rings must not be able to push the
    /// diagnosis out of the one line a teardown report has room for.
    pub doorbell_refusal: KayfabeDoorbellRefusal,
}

/// Translate a chip-table refusal into the wire vocabulary, keeping the sentence.
///
/// ★ Every arm is [`Status::Unsupported`] rather than [`Status::Refused`], and that is the
/// distinction the type already draws: a chip row that does not exist, or one whose sources
/// overlap, cannot be fixed by retrying. It is a property of this build.
#[must_use]
pub fn classify_chip(e: &ChipError) -> (Status, &'static str) {
    match e {
        ChipError::NoChipForDevice { .. } => (
            Status::Unsupported,
            "this build has no emulated-chip profile for that PCI device id, and there is \
             deliberately no nearest-neighbour fallback: answering a driver as a chip we do \
             not model surfaces as a failure a thousand registers later",
        ),
        ChipError::VbiosProfileMissing { .. } => (
            Status::Unsupported,
            "the chip row has no synthetic-VBIOS row behind it, so the identity this device \
             would claim has no ROM; the two are keyed on the same PCI device id precisely \
             so they cannot disagree",
        ),
        ChipError::Vbios(_) => (
            Status::Unsupported,
            "the synthetic VBIOS for this chip could not be built; its profile declares a \
             geometry the guest driver's own bounds checks would reject",
        ),
        ChipError::RomTooLargeForWindow { .. } => (
            Status::Unsupported,
            "the generated ROM does not fit the ROM window the chip declares; the guest \
             would parse a truncated image and fail far from here",
        ),
        ChipError::OverlappingSources { .. } => (
            Status::Unsupported,
            "two of the chip's declared read sources cover one offset; the read path asks \
             them in a fixed order, so the loser would silently never be consulted",
        ),
        ChipError::OutsideAperture { .. } => (
            Status::Unsupported,
            "the chip declares a register or window outside its own register aperture, so \
             the guest could never address it",
        ),
        ChipError::WindowWithoutItsRegister { .. } => (
            Status::Unsupported,
            "the chip declares a PRAMIN window and no NV_PBUS_BAR0_WINDOW register to move \
             it, or the register and no window; the two are one mechanism, and an aperture \
             nothing can move shows framebuffer address zero forever without saying so",
        ),
        ChipError::NoFaultMethodBufferSize { .. } => (
            Status::Unsupported,
            "the chip row states no copy-engine fault method buffer size, and this device \
             will not invent one: the value is not derivable from any tree — the GSP-side \
             handler is firmware and the control is kernel-privileged — so it must be \
             MEASURED on a part of this generation. Serving a zero instead is not a weaker \
             answer, it is the guest's RmInitAdapter failing 0x25:0x1f:1249 from a \
             zero-length memdescCreate, with nothing naming this row",
        ),
        ChipError::BarTableDisagreesWithAperture { .. } => (
            Status::Unsupported,
            "the chip states its register aperture's size twice — as regs_aperture_len and \
             as row 0 of its BAR table — and the two differ; one is what the hypervisor \
             registers and the other is what the guest's RM is told, and nothing logs",
        ),
    }
}

/// Resolve a chip row. `0` means "the table's default".
///
/// # Errors
/// [`Status::Unsupported`], [`classify_chip`]-ed.
pub fn chip_for(device_id: u16) -> Result<&'static ChipProfile, (Status, &'static str)> {
    if device_id == 0 {
        return Ok(kayfabe_device::default_chip());
    }
    kayfabe_device::chip_for_device_id(device_id).map_err(|e| classify_chip(&e))
}

/// The identity a chip's device claims, in the wire shape.
///
/// # Errors
/// [`classify_chip`]-ed.
pub fn chip_identity(device_id: u16) -> Result<KayfabeChipIdentity, (Status, &'static str)> {
    let chip = chip_for(device_id)?;
    let id = kayfabe_device::identity_for(chip).map_err(|e| classify_chip(&e))?;
    Ok(KayfabeChipIdentity {
        abi_version: ABI_VERSION,
        struct_size: size_of::<KayfabeChipIdentity>() as u32,
        regs_aperture_len: chip.regs_aperture_len,
        fb_window_len: id.fb_window_len,
        inst_window_len: id.inst_window_len,
        class_code: id.class_code,
        vendor_id: id.vendor_id,
        device_id: id.device_id,
        subsystem_vendor_id: id.subsystem_vendor_id,
        subsystem_id: id.subsystem_id,
        msix_vectors: id.msix_vectors,
        revision: id.revision,
        reserved: 0,
    })
}

/// ★★★ The device's free-running nanosecond counter, driven by the host's monotonic clock.
///
/// **This is why it is in the adapter and not in the device crate.** Reading real time is an
/// OS capability, and `kayfabe-device` is one of the pure logic crates — it may model a
/// counter and say where a chip exposes it, and it may not know what o'clock it is. So the
/// device declares [`kayfabe_device::NanoClock`] and this crate is the one that satisfies it.
///
/// ★★ It is a **host** monotonic clock rather than the hypervisor's virtual one, and the
/// difference is a real if small departure from the C artifact, which samples
/// `QEMU_CLOCK_VIRTUAL` (`C: src/qemu/nvkvm_gpu_emul.c:1523-1528`). Sampling the
/// hypervisor's clock would mean a new primitive in the shim's function-pointer table, i.e.
/// a hypervisor concept crossing into a decision this crate makes. The two agree except
/// while the machine is stopped — under a debugger, or across a migration — and what a
/// guest observes then is a counter that jumped forward, which is a thing real silicon does
/// to a driver whose vCPU was descheduled. Worth revisiting if a stopped machine ever needs
/// to look stopped to the guest; not worth a table entry now.
///
/// ---
/// ⚠⚠⚠ **BOOT-ONLY STOPGAP — not the finished design** (`#128`,
/// `docs/design/register_plane_read_native.md` §4). The whole `HostMonotonicClock` /
/// `QEMU_CLOCK_VIRTUAL` question above is a debate between two **wrong** answers: both are
/// CPU-side clocks, and the guest's timestamps have to be in the **host GPU's** timebase
/// because that is where its compute actually runs. Which CPU clock we pick changes how
/// wrong `cudaEventElapsedTime` is, not whether it is wrong.
///
/// The finished design replaces this port entirely with a read-only memslot over the host
/// GPU's own register page. ★ **MEASURED 2026-08-02** that the mapping this needs is
/// obtainable by a capability-less process on a real GA106 —
/// `docs/design/read_native_timer_measured.md`. See [`kayfabe_device::NanoClock`] for the
/// full argument and the standing rule that outlives this type.
#[derive(Debug)]
struct HostMonotonicClock {
    origin: Instant,
}

impl HostMonotonicClock {
    fn new() -> HostMonotonicClock {
        HostMonotonicClock {
            origin: Instant::now(),
        }
    }
}

impl NanoClock for HostMonotonicClock {
    fn now_ns(&self) -> u64 {
        // ★ Saturating, not wrapping: a counter that went backwards is the one thing
        // `NanoClock`'s contract forbids, and `as u64` on an overflowing `u128` would do
        // exactly that. The elapsed nanoseconds of a process cannot reach `u64::MAX`
        // (584 years), so this cannot be reached — it is here so that if it ever were, the
        // counter would stick rather than reverse.
        u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

// =====================================================================================
// ★★★ E2 — the join between a trapped BAR write and `kayfabe_rt::SharedDevice`
// =====================================================================================

/// ★★★ **The object model, as the shell owns it** — `kayfabe_rmrpc::ObjectModel`
/// implemented over the L1 shared device.
///
/// # Why the `Gpu` moved here, and what it bought
///
/// `ObjectPolicy` used to **own** its `Gpu`, and its own docs called that *"a stage fact,
/// not a design"* that would end *"the day the doorbell path also wants it"*. This is that
/// day: `docs/design/execution_plane_increments.md` **E2** routes a guest MMIO write to
/// `SharedDevice::doorbell`, and it must route into **the same** object model the guest's
/// `GSP_RM_ALLOC`s populated. A second `Gpu` behind the doorbell would give a transport
/// that is trivially green and a routing table that can never resolve — the shape this
/// project calls a plausible wrong answer.
///
/// ⊘ **Nothing else changed.** The bridge's meaning of a command, its reassembly ordering,
/// its four counters and its census are one implementation
/// (`kayfabe_rmrpc::policy::Bridge`), driven through the same two calls.
// ⊘ Hand-written `Debug` rather than derived: `SharedDevice` is not `Debug` and must not
// become it — the whole object model in a panic message is unreadable, which is the same
// argument `kayfabe_rmrpc::ObjectPolicy` already makes about its `Gpu`.
#[derive(Clone)]
struct SharedObjectModel(Arc<kayfabe_rt::device::SharedDevice>);

impl core::fmt::Debug for SharedObjectModel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SharedObjectModel")
            .field("mode", &self.0.mode())
            .finish_non_exhaustive()
    }
}

impl kayfabe_rmrpc::ObjectModel for SharedObjectModel {
    fn apply(
        &mut self,
        ev: kayfabe_core::rmgraph::RmEvent,
    ) -> Result<(), kayfabe_core::gpu::GpuError> {
        self.0.apply(ev)
    }

    fn promote_ctx(
        &mut self,
        p: &kayfabe_core::promote::CtxPromotion,
    ) -> Result<kayfabe_core::promote::PromoteJoin, kayfabe_core::promote::PromoteFault> {
        self.0.promote_ctx(p)
    }

    fn publish_isolate_census(&self, to: &kayfabe_core::gpu::SharedIsolateCensus) {
        to.publish(self.0.isolate_census());
    }

    /// `None`, and that is the whole of what a sharded shell can honestly say: the graph
    /// lives inside a device lock and a proc lock, and a `&Gpu` handed out here would
    /// outlive both guards.
    fn as_gpu(&self) -> Option<&kayfabe_core::gpu::Gpu> {
        None
    }
}

/// ★★★ **E2 — the doorbell port**: a guest store to `NV_VIRTUAL_FUNCTION_DOORBELL` becomes
/// one `SharedDevice::doorbell` call, and its answer becomes a
/// [`kayfabe_device::DoorbellReport`].
///
/// # What is deliberately not decided here
///
/// - **The token is passed through, whole.** Decoding it is `Ga10xArch::decode_doorbell`'s
///   job (increment E3, settled against RM's own compiled encoder — see
///   `docs/design/doorbell_token_encoding.md`), and a second, weaker decode in the shim
///   would be exactly the "two descriptions of one fact" this port refuses elsewhere.
/// - **The working set is empty**, and that is honest rather than lax: recovering which
///   VAs a submission touches means parsing the ring, which is increment **E4**. An empty
///   working set is `plan_doorbell`'s documented *"this submission touches no tracked VA"*
///   — there is nothing to gate on and no host state at risk. ⚠ It is **not** a bypass of
///   the #14 gate: the gate still runs, over an empty set. E4/E5 fill it.
/// - **The target GPU is [`GpuId::ZERO`]**, because this device is one GPU — the same id
///   `Gpu::realize` carves the system proc's arena for. The day a shim realizes two, this
///   comes from the device instance and not from a constant.
struct SharedDoorbell(Arc<kayfabe_rt::device::SharedDevice>);

impl core::fmt::Debug for SharedDoorbell {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SharedDoorbell")
            .field("gpu", &DOORBELL_TARGET_GPU.0)
            .finish_non_exhaustive()
    }
}

/// The GPU a `nvkvm-gpu` device is. See [`SharedDoorbell`]'s docs.
const DOORBELL_TARGET_GPU: kayfabe_rt::GpuId = kayfabe_rt::GpuId(0);

impl kayfabe_device::DoorbellPort for SharedDoorbell {
    fn ring(&self, token: u64) -> kayfabe_device::DoorbellReport {
        match self.0.doorbell(DOORBELL_TARGET_GPU, token, &[]) {
            Ok(o) => kayfabe_device::DoorbellReport::Served {
                token,
                proc: o.proc.0,
                chan: o.chan.0,
                host_token: o.host_token,
                scheduled_now: o.scheduled_now,
            },
            // ★ A **kind** and a sentence, which is increment E1's standard: the kind comes
            // from the fault type's own exhaustive `Faulted::fault_tag` (so a new variant
            // fails `kayfabe-fwd`'s build until it is named) and the sentence is the
            // variant's payload, which carries the token, the decoded vChid or whichever
            // of those the refusal is about.
            Err(f) => kayfabe_device::DoorbellReport::Refused {
                token,
                refusal: kayfabe_device::DoorbellRefused {
                    kind: kayfabe_device::Faulted::fault_tag(&f),
                    why: format!("{f:?}"),
                },
            },
        }
    }
}

/// The realized register plane — what the C shim holds behind its second opaque handle.
///
/// ⊘ Hand-written [`core::fmt::Debug`] since E2, because `SharedDevice` deliberately has
/// none — see [`SharedObjectModel`].
pub struct Regs {
    plane: RegPlane,
    /// ★★★ **E2** — the L1 shell that owns the object model, held here because **two**
    /// paths now reach it: the object bridge (boxed into the register plane's served
    /// chain, and unreachable afterwards) and the doorbell port. Before E2 there was one
    /// path and it could own the `Gpu` outright.
    ///
    /// ⊘ Held for the doorbell port's sake and read by nothing else in this struct: the
    /// two censuses below are still the shell's own channels, for the reason each states.
    ///
    /// ⚠ `#[allow(dead_code)]` because the *field* is never read — the live reference is the
    /// clone inside the doorbell port, which the register plane owns. Dropping the field
    /// would be wrong anyway: it is what keeps this device's object model alive for exactly
    /// as long as the device, and a shell that let it go would leave the plane's port
    /// holding the last handle to a graph nobody can reach.
    #[allow(dead_code)]
    device: Arc<kayfabe_rt::device::SharedDevice>,
    /// ★★★ The object bridge's refusal census, kept **here** because the policy that owns
    /// it is boxed into the chain and is unreachable afterwards. See
    /// [`kayfabe_rmrpc::SharedRefusalCensus`] for the boot that had to be diagnosed by the
    /// absence of a line instead.
    refusals: kayfabe_rmrpc::SharedRefusalCensus,
    /// ★★★ E1 — the isolate plane's census, kept here for the reason
    /// [`Regs::refusals`] is: the policy that owns the object model is boxed into the
    /// chain and unreachable afterwards.
    isolates: kayfabe_core::gpu::SharedIsolateCensus,
}

impl core::fmt::Debug for Regs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Regs")
            .field("plane", &self.plane)
            .finish_non_exhaustive()
    }
}

impl Regs {
    /// Build the register plane for a chip. `0` selects the table's default row.
    ///
    /// # Errors
    /// [`classify_chip`]-ed, or [`Status::Unsupported`] if the guest driver version this
    /// build answers as has no wire table.
    pub fn create(device_id: u16) -> Result<Regs, (Status, &'static str)> {
        let chip = chip_for(device_id)?;
        let abi = kayfabe_device::abi::gsp_abi_for(GUEST_DRIVER).map_err(|_| {
            (
                Status::Unsupported,
                "this build has no wire table for the guest driver version its register \
                 plane answers as; the table is keyed on the full major.minor.patch and \
                 refuses below its floor rather than nearest-neighbouring",
            )
        })?;
        let (objects, refusals, isolates, device) = object_policy(abi.driver)?;
        let plane = RegPlane::with_objects(
            chip,
            abi,
            Box::new(HostMonotonicClock::new()),
            Some(objects),
        )
        .map_err(|e| classify_chip(&e))?;
        // ★★★ **THE COMPOSITION ROOT'S FRAMEBUFFER DECISION, made here and nowhere else.**
        //
        // `kayfabe_device::RegPlane` is built with `RefusingFb`, so a shell that never made
        // this decision gets a device that says *"there is no framebuffer here"* rather than
        // one that behaves like an empty one. This is the shell, and it decides.
        //
        // ⊘ **Why a shell-owned sparse store and not the isolate's `FbRead`**, which is
        // where owner decision (b) put framebuffer content: three reasons, all read off the
        // two seams' own signatures and lifetimes (`[inferred]`, stated in full in
        // `kayfabe_device::fbwin::FbStore`'s docs), none of them about layering. The
        // short one is that `kbusVerifyBar2` runs inside `RmInitAdapter`, **before the
        // first client root exists** — there is no `Proc`, no isolate and no worker to
        // borrow a byte from. The day the data plane exists, convergence is an `FbStore`
        // implementation that delegates, installed through this same call.
        //
        // ★ Sized from the chip row's own `fb_length` — the SAME number the emulated GSP
        // answers `NV2080_CTRL_CMD_FB_GET_INFO` and `GA106_FB_REGIONS` with. A store
        // smaller than what the device advertises would refuse an address the guest was
        // promised, which is a refusal we would have manufactured ourselves.
        plane.set_fb(Box::new(kayfabe_device::SparseFb::new(chip.fb_length)));
        // ★★★ **THE COMPOSITION ROOT'S PAGE-TABLE-FORMAT DECISION** (`#149`), made here
        // and nowhere else, for exactly the reasons the framebuffer decision above is.
        //
        // `kayfabe_device::RegPlane` is built with **no** format, so a shell that never
        // made this decision gets a device whose translated apertures refuse by name
        // rather than one that invents a stride. This is the shell, and it decides.
        //
        // ★★ **The same type `kayfabe_chips::Ga10xArch::mmu` answers with**, and that is
        // the whole of why it is a port. A `GmmuFmt` is an Axis-B seam whose real
        // implementation belongs in an arch-adapter crate; making it a `ChipProfile` row
        // would put a second copy of one chip's page-table format in a second crate, which
        // is the defect `kayfabe_chips::ga10x`'s own `gsp()` docs refuse for the register
        // model one seam over. This root already holds both crates, so it is the one place
        // that can join them without either naming the other.
        plane.set_mmu(Box::new(kayfabe_chips::Ga10xGmmu::new()));
        // ★★★ **THE COMPOSITION ROOT'S DOORBELL DECISION** (`execution_plane_increments.md`
        // E2), made here and nowhere else, for exactly the reasons the two decisions above
        // are.
        //
        // `kayfabe_device::RegPlane` is built with `RefusingDoorbell`, so a shell that never
        // made this decision gets a device that **counts** a guest ring and says, by name,
        // that it forwarded nothing — rather than one that swallows a submission and looks
        // healthy. This is the shell, and it decides.
        //
        // ★ The port is a `SharedDevice` handle and not a second object model; see
        // [`SharedObjectModel`] for why that identity is the whole increment.
        plane.set_doorbell(Box::new(SharedDoorbell(Arc::clone(&device))));
        Ok(Regs {
            plane,
            device,
            refusals,
            isolates,
        })
    }

    /// The plane, for a caller that needs more than this seam exposes.
    #[must_use]
    pub fn plane(&self) -> &RegPlane {
        &self.plane
    }

    /// ★★★ **Stage Q5.** Give the register plane the realized machine's guest memory.
    ///
    /// # Why it is a separate call and not a constructor argument
    ///
    /// The order is fixed by the hypervisor, not by us: a PCI device realizes — and builds
    /// its register plane — while its base-address registers are still unprogrammed, and
    /// the memory plane cannot realize until one has a base, because it installs slots at
    /// it. So there is a real interval during which registers are being answered and there
    /// is no memory plane to answer *from*, and that interval must have a defined
    /// behaviour rather than a null check. It does: [`kayfabe_device::RefusingRam`], which
    /// refuses by name.
    ///
    /// Idempotent, and re-attachable: [`kayfabe_device::RegPlane::set_ram`] takes `&self`
    /// and the plane's own lock, so a plane already answering registers on one vCPU
    /// acquires memory without being rebuilt and without a window in which it answers
    /// something else.
    pub fn attach_ram(&self, shim: &Shim) {
        self.plane
            .set_ram(Box::new(MachineRam::new(shim.machine().vmm())));
    }

    /// Put the plane back to refusing every guest-memory access, by name.
    ///
    /// ★ The teardown half, and it is **not** optional. The port holds a handle onto the
    /// memory plane; leaving it installed across an unrealize would mean the register
    /// surface — which keeps answering, deliberately — could still be asked to follow a
    /// guest pointer into a machine that has released its slots. Refusing is the honest
    /// answer at that point and it is the one this restores.
    pub fn detach_ram(&self) {
        self.plane.set_ram(Box::new(kayfabe_device::RefusingRam));
    }

    /// Serve one register read.
    #[must_use]
    pub fn read(&self, bar: u32, off: u64, size: u32) -> u64 {
        self.plane
            .read(clamp_bar(bar), off, clamp_size(size))
            .value()
    }

    /// Serve one register write.
    ///
    /// ★ Returns the **port's** outcome, not the wire shape. `KayfabeRegWrite` carries a
    /// raw pointer to the fault's sentence, and a host address may not appear in a file
    /// that is not `*_unsafe.rs` — the host-pointer gate refused the first draft of this,
    /// which is the gate doing exactly its job. The conversion lives one file over, beside
    /// every other structure in this crate that holds an address.
    #[must_use]
    pub fn write(&self, bar: u32, off: u64, size: u32, val: u64) -> kayfabe_device::WriteOutcome {
        self.plane.write(clamp_bar(bar), off, clamp_size(size), val)
    }

    /// Power-on reset.
    pub fn reset(&self) {
        self.plane.device_reset();
    }

    /// The counters, in the wire shape.
    ///
    /// ★★★ **The source is DESTRUCTURED with no `..`** — same obligation, same reason, as
    /// [`Shim::audit`]: a counter added to `kayfabe_device::Counters` and not wired here is
    /// a number the C shell can never read, and nothing goes red. `rustc` refuses the
    /// pattern (E0027) instead.
    #[must_use]
    pub fn audit(&self) -> KayfabeRegAudit {
        // ★★★ EXHAUSTIVE. The missing `..` is load-bearing — see this method's docs.
        let kayfabe_device::Counters {
            reads,
            writes,
            boot_reg_reads,
            ptimer_reads,
            ptimer_writes_refused,
            rom_reads,
            gsp_reads,
            gsp_writes,
            unclaimed_reads,
            unclaimed_writes,
            fb_window_reads,
            fb_window_writes,
            fb_reads,
            fb_writes,
            fb_refusals,
            bar2_reads,
            bar2_writes,
            bar2_faults,
            bar0_window_reads,
            bar0_window_writes,
            faults,
            ram_refusals,
            irq_requests,
            cpu_intr_accesses,
            cpu_intr_raises,
            cpu_intr_masked,
            commands,
            commands_unserviced,
            doorbells,
            doorbells_served,
            doorbells_refused,
        } = self.plane.counters();
        let (bar_pde_updates, bar_pde_refusals) = self.plane.bar_pde_counts();
        // ★ Truncated to what the wire shape holds, and `unserviced_len` says how many —
        // never silently clipped to look complete. The plane's own sample is bounded by
        // the same order of magnitude, so this is a shape conversion and not a policy.
        let sample = self.plane.unserviced_sample();
        let mut unserviced = [0u64; UNSERVICED_SLOTS];
        for (slot, e) in unserviced.iter_mut().zip(sample.iter()) {
            *slot = (u64::from(e.function) << 32) | u64::from(e.cmd.unwrap_or(UNSERVICED_NO_CMD));
        }
        // ★★★ The bridge's own refusals, which reach NO ledger — see
        // [`KayfabeBridgeRefusal`]. Names cross by value; the truncation arm is a real
        // branch rather than a silent `min`, because a clipped tag that still looked like
        // a tag would be the quiet kind of wrong this whole struct exists to prevent.
        let census = self.refusals.snapshot();
        let bridge_refusals = census.total() as u64;
        let mut bridge_refusal = [KayfabeBridgeRefusal::default(); BRIDGE_REFUSAL_SLOTS];
        let mut bridge_refusal_len = 0u64;
        for (row, (tag, n)) in bridge_refusal.iter_mut().zip(census.tags()) {
            let bytes = tag.0.as_bytes();
            let take = bytes.len().min(BRIDGE_REFUSAL_TAG_LEN);
            row.tag[..take].copy_from_slice(&bytes[..take]);
            row.tag_len = take as u64;
            row.count = n as u64;
            bridge_refusal_len += 1;
        }
        // ⊘ Reported from the census, not from the loop: a set larger than the array must
        // say so, exactly as `unserviced_len` does.
        let bridge_refusal_len = bridge_refusal_len.max(census.tags().count() as u64);
        // ★★★ E1 — the isolate plane, DESTRUCTURED with no `..` for [`Shim::audit`]'s
        // reason: a census field added and not wired here is a number the C shell can
        // never read, and nothing goes red. `rustc` refuses the pattern instead.
        let kayfabe_isolate::IsolateCensus {
            materialized: isolates_materialized,
            live: isolates_live,
            no_plane: isolates_no_plane,
            spawn_failed: isolates_spawn_failed,
            first,
        } = self.isolates.snapshot();
        let mut isolate_refusal = KayfabeIsolateRefusal::default();
        if let Some((kind, why)) = first {
            isolate_refusal.kind = match kind {
                kayfabe_isolate::RefusalKind::NoPlane => ISOLATE_REFUSAL_NO_PLANE,
                kayfabe_isolate::RefusalKind::SpawnFailed => ISOLATE_REFUSAL_SPAWN_FAILED,
            };
            // ⊘ Truncated on a CHARACTER boundary, not on a byte: a sentence cut mid-UTF-8
            // would print as a replacement character in the one line an operator reads to
            // find out why their forwarding plane is down.
            let mut take = why.len().min(ISOLATE_REFUSAL_LEN);
            while take > 0 && !why.is_char_boundary(take) {
                take -= 1;
            }
            isolate_refusal.text[..take].copy_from_slice(&why.as_bytes()[..take]);
            isolate_refusal.len = take as u64;
        }
        // ★★★ **E2** — what the doorbell aperture saw, DESTRUCTURED with no `..` for
        // `Shim::audit`'s reason: a field added to `DoorbellLog` and not wired here is a
        // fact the C shell can never read, and nothing goes red. `rustc` refuses instead.
        let kayfabe_device::DoorbellLog {
            last_token,
            first_refusal,
        } = self.plane.doorbell_log();
        let mut doorbell_refusal = KayfabeDoorbellRefusal::default();
        if let Some(r) = first_refusal {
            doorbell_refusal.present = 1;
            let kb = r.kind.0.as_bytes();
            let ktake = kb.len().min(DOORBELL_KIND_LEN);
            doorbell_refusal.kind[..ktake].copy_from_slice(&kb[..ktake]);
            doorbell_refusal.kind_len = ktake as u64;
            // ⊘ Truncated on a CHARACTER boundary, not on a byte, for the reason the
            // isolate sentence above is.
            let mut take = r.why.len().min(DOORBELL_REFUSAL_LEN);
            while take > 0 && !r.why.is_char_boundary(take) {
                take -= 1;
            }
            doorbell_refusal.text[..take].copy_from_slice(&r.why.as_bytes()[..take]);
            doorbell_refusal.len = take as u64;
        }
        KayfabeRegAudit {
            reads,
            writes,
            boot_reg_reads,
            ptimer_reads,
            ptimer_writes_refused,
            rom_reads,
            gsp_reads,
            gsp_writes,
            unclaimed_reads,
            unclaimed_writes,
            fb_window_reads,
            fb_window_writes,
            fb_reads,
            fb_writes,
            fb_refusals,
            bar2_reads,
            bar2_writes,
            bar2_faults,
            bar_pde_updates: (bar_pde_updates << 32) | (bar_pde_refusals & 0xFFFF_FFFF),
            bar2_root_entry: self.plane.bar_pdes().bar2.map_or(0, |p| p.entry),
            bar0_window_reads,
            bar0_window_writes,
            // ★ Read from the plane's residue rather than kept as a counter: it is a
            // LEVEL, not a total, so a counter would be wrong the moment a device reset
            // freed the pages.
            fb_resident_bytes: self.plane.residue().fb_resident_bytes,
            faults,
            ram_refusals,
            irq_requests,
            cpu_intr_accesses,
            cpu_intr_raises,
            cpu_intr_masked,
            commands,
            commands_unserviced,
            unserviced_len: sample.len() as u64,
            unserviced,
            bridge_refusals,
            bridge_refusal_len,
            bridge_refusal,
            isolates_materialized,
            isolates_live,
            isolates_no_plane,
            isolates_spawn_failed,
            isolate_refusal,
            doorbells,
            doorbells_served,
            doorbells_refused,
            doorbell_last_token: last_token.unwrap_or(0),
            doorbell_last_token_valid: u64::from(last_token.is_some()),
            doorbell_refusal,
        }
    }
}

/// ★★★ **The object model this port declares protocol facts into** — the composition
/// root's one call, and the answer to the wall the 2026-08-01 boot measured.
///
/// # What it joins, and what it deliberately does not
///
/// `GSP_RM_ALLOC` and `FREE` become `kayfabe_core::rmgraph::RmEvent`s and go into the
/// **existing** object model: DUP\_OBJECT refcounting, client/device/subdevice parenting,
/// the recycled-namespace defences and the cross-GPU handle gate are all already there and
/// none of them is re-implemented here. `kayfabe_rmrpc::ObjectPolicy` is the link;
/// `kayfabe_device::served_chain` decides where it sits and what it must not claim.
///
/// # ⚠ The three ports this stage has NOT built, named at the site that fakes none of them
///
/// A `Gpu` needs an [`Arch`](kayfabe_arch::Arch), an isolate factory and a guest-physical
/// window. This port has a real answer for exactly one of them, and says so in the values
/// rather than in a comment:
///
/// 1. **`Ga10xArch`** classifies objects from NVIDIA's real class ids and **refuses** every
///    data-plane seam — zero MMU levels, no page sizes, no doorbell decode. It is not a
///    mock: `kayfabe_mocks` is not a dependency of this crate and must never become one.
/// 2. **The isolate plane is now SELECTED, and it still defaults to `StillbornIsolates`**
///    — see [`selected_isolate_plane`]. Unless `KAYFABE_ISOLATES` names another plane,
///    every isolate is retired at birth and every verb refuses through the core's own
///    backpressure path, exactly as before. ⊘ A verb that *succeeded* under the default
///    would be the mock wall in the product; a verb that succeeds under
///    `KAYFABE_ISOLATES=real` is a real host RM ioctl, which is the point.
/// 3. **The GPA window** below is a declared range that nothing installs a memslot from.
///    Its only consumer at this stage is `Gpu::realize`, which carves the system proc an
///    arena out of it; no guest-physical address derived from it reaches the hypervisor.
///    ⚠ The day the data plane exists, this comes from the VMM's installed window
///    (`Shim::install_window`) and not from a constant here — a constant that outlived
///    that day would be two descriptions of one address space.
///
/// # Errors
///
/// [`Status::Unsupported`] if the object model cannot realize. ★ That is a **refusal to
/// realize the device**, not a degraded mode: a register plane whose alloc link is missing
/// answers every `GSP_RM_ALLOC` with the named refusal that stopped the last boot, and
/// serving that silently is how a port comes to be measured for something it is not doing.
type ObjectLink = (
    Box<dyn kayfabe_gsp::CommandPolicy>,
    kayfabe_rmrpc::SharedRefusalCensus,
    kayfabe_core::gpu::SharedIsolateCensus,
    // ★★★ E2 — and the shell itself, because the doorbell port needs the SAME one.
    Arc<kayfabe_rt::device::SharedDevice>,
);

fn object_policy(
    driver: kayfabe_abi::versions::DriverAbiTable,
) -> Result<ObjectLink, (Status, &'static str)> {
    let isolates = isolate_factory(selected_isolate_plane()?)?;
    let gpu = kayfabe_core::gpu::Gpu::new(
        Box::new(kayfabe_chips::Ga10xArch::new()),
        isolates,
        kayfabe_core::gpa::GpaSpace::new(OBJECT_GPA_WINDOW, OBJECT_GPA_ARENA),
    )
    .map_err(|_| {
        (
            Status::Unsupported,
            "the object model could not realize: its guest-physical window cannot supply \
             the system proc an arena",
        )
    })?;
    // ★★★ **E2** — the realized `Gpu` goes into the L1 shell, and the shell is what both
    // the object bridge and the doorbell port declare into. See [`SharedObjectModel`].
    //
    // ★ `LockMode::Sharded` — the #14-gate configuration, in which a per-proc op takes the
    // device *read* lock and then that one proc's mutex. ⊘ Not `Degenerate`: the doorbell
    // path's whole reason for existing is that a guest process's submissions must not
    // serialize behind another's, and choosing the single-lock shape here would make the
    // shipped archive the one configuration the #14 design does not apply to.
    let device = Arc::new(kayfabe_rt::device::SharedDevice::new(
        gpu,
        kayfabe_rt::device::LockMode::Sharded,
    ));
    let policy = kayfabe_rmrpc::ObjectPolicy::over(
        &driver,
        // ★ The fourth axis, DECLARED and never sniffed. The guest OS is a `#define` in
        // the guest driver's build and is undetectable on the wire, so a port that
        // inferred it would be inferring an isolation boundary from a coincidence. This
        // build answers as the bench's guest, and the day it must answer as another one,
        // this becomes a realize-time property beside the driver version — not an `if`.
        kayfabe_abi::GuestOs::Linux,
        Box::new(SharedObjectModel(Arc::clone(&device))),
        kayfabe_rmrpc::ReasmLimits::default(),
    );
    // ★ The handle is taken BEFORE the policy is boxed, because afterwards there is no
    // `ObjectPolicy` left to ask — that is the whole reason the census had to become a
    // shared store rather than a field behind `&self`.
    let refusals = policy.refusal_census();
    // ★★★ E1 — and the isolate plane's own health, for the same reason and by the same
    // mechanism. Before this the only channel that could say "the forwarding plane you
    // asked for did not come up" was a host-side `ps`.
    let isolates = policy.isolate_census();
    Ok((Box::new(policy), refusals, isolates, device))
}

// =====================================================================================
// ★★★ The isolate-plane selector (`execution_plane_increments.md` increment E0)
// =====================================================================================

/// The environment variable that names which isolate plane the composition root installs.
///
/// ★★ **An environment variable and not a QOM property, deliberately, and only for E0.**
/// A QOM property is the right long-term home — it is per-device, it appears in
/// `-device nvkvm-gpu,help`, and it cannot leak from one device to another — but it costs
/// a shim-ABI change plus a C hunk, and E0's whole claim is that the *join* works. Putting
/// the selector on the ABI in the same increment would mean two unrelated things to review
/// and would make the negative control run a different binary. `execution_plane_increments.md`
/// E1 owns moving it.
///
/// ⚠ The consequence, stated rather than discovered later: this is **process-global**, so a
/// hypervisor with two `nvkvm-gpu` devices gets the same plane for both. That is correct
/// for the bench and wrong for a product.
pub const ISOLATE_PLANE_ENV: &str = "KAYFABE_ISOLATES";

/// Why [`IsolatePlane::Stillborn`] refuses — the string the core reports at the seam, and
/// the one master shipped unconditionally.
const STILLBORN_WHY: &str = "this build has no forwarding plane: the object model accepts \
                             protocol facts and no host verb can be issued";

/// Which isolate plane the composition root installs.
///
/// ⊘ **There is no `Auto`, and there is no fallback.** A selector that quietly degraded
/// `real` to `stillborn` when the host GPU was absent would make "the boot behaved exactly
/// as it did before" mean two different things, and the project's own ledger records seven
/// occasions where the instrument was the defect. Every arm this build cannot serve is a
/// refusal to realize the device, named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolatePlane {
    /// Every isolate retired at birth; no child process, no host verb. **The default**, and
    /// what master shipped unconditionally.
    Stillborn,
    /// A real sandboxed child process per `(Proc, GpuId)` — with a **loopback** `RmBackend`
    /// inside it. Real `clone`, real namespaces, real wire protocol, **no NVIDIA ioctl**.
    ///
    /// ★ This exists so the two halves of `real` can fail separately: a spawn that dies
    /// here is a sandbox/namespace/image problem, and a spawn that dies only under `real`
    /// is an RM bring-up problem. Without it those are one symptom.
    Loopback,
    /// A real sandboxed child that opens `/dev/nvidiactl`, `/dev/nvidia<N>` and completes
    /// RM bring-up (`kayfabe_isolate_host::rm::RmConnection::open`, rungs R0–R6b) — i.e.
    /// **real host RM ioctls on the real host GPU.**
    ///
    /// ⚠ **They are issued at device-REALIZE time, not by anything the guest does**, and
    /// this comment used to say the opposite. `Gpu::realize` installs the system proc's
    /// isolate unconditionally, so the child exists before the guest has run a single
    /// instruction; a guest `GSP_RM_ALLOC` then finds it already there and spawns nothing.
    /// `[measured]` 2026-08-01 at rev `e10a6bf` on RTX 3060 / 580.159.04 open: the child's
    /// first sighting is **t+3 s** and the guest opens the device at **t+30–34 s**
    /// (`docs/reference/bench_evidence/e10a6bf_run_e0real2_isolate.log`). Making the spawn
    /// lazy is `execution_plane_increments.md` **E0b**.
    Real,
}

impl IsolatePlane {
    /// The spelling this plane is selected by. Round-trips with [`IsolatePlane::parse`].
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            IsolatePlane::Stillborn => "stillborn",
            IsolatePlane::Loopback => "loopback",
            IsolatePlane::Real => "real",
        }
    }

    /// Parse a plane name. ⊘ Case-sensitive and exact, matching
    /// `kayfabe_isolate_host::RmMode::parse`, whose own test is named
    /// *"an unknown RM mode is refused rather than defaulted to real"*.
    #[must_use]
    pub fn parse(s: &str) -> Option<IsolatePlane> {
        match s {
            "stillborn" => Some(IsolatePlane::Stillborn),
            "loopback" => Some(IsolatePlane::Loopback),
            "real" => Some(IsolatePlane::Real),
            _ => None,
        }
    }

    /// Every plane this enum can express, for gates that must quantify over the whole set
    /// rather than over a list someone can shorten (`gates_quantified_over_a_list`).
    pub const ALL: [IsolatePlane; 3] = [
        IsolatePlane::Stillborn,
        IsolatePlane::Loopback,
        IsolatePlane::Real,
    ];
}

/// The plane named by `value` — the pure half of [`selected_isolate_plane`], so the
/// decision can be tested without touching a process-global.
///
/// # Errors
/// [`Status::Unsupported`] if `value` is not a plane name. **Absent is not an error**; it
/// is [`IsolatePlane::Stillborn`], which is what master shipped.
pub fn isolate_plane_from(value: Option<&str>) -> Result<IsolatePlane, (Status, &'static str)> {
    match value {
        None => Ok(IsolatePlane::Stillborn),
        Some(v) => IsolatePlane::parse(v).ok_or((
            Status::Unsupported,
            "KAYFABE_ISOLATES does not name an isolate plane: the only values are \
             `stillborn` (the default), `loopback` and `real`. It is not defaulted, \
             because a typo that silently selected the refusing plane would make an \
             evidence run and its own negative control indistinguishable.",
        )),
    }
}

/// The plane [`ISOLATE_PLANE_ENV`] names, or [`IsolatePlane::Stillborn`] if it is unset.
///
/// # Errors
/// [`Status::Unsupported`] if the variable is set to something that is not a plane name,
/// **including a non-UTF-8 value** — see [`isolate_plane_from`].
fn selected_isolate_plane() -> Result<IsolatePlane, (Status, &'static str)> {
    match std::env::var_os(ISOLATE_PLANE_ENV) {
        None => Ok(IsolatePlane::Stillborn),
        // ★ A non-UTF-8 value takes the `Some(non-name)` arm rather than the `None` arm:
        // it was SET, so it must not read as unset.
        Some(v) => isolate_plane_from(Some(v.to_str().unwrap_or("\u{fffd}invalid"))),
    }
}

/// Build the factory for `plane`.
///
/// ★★ **Both non-default arms are `Err` in a build without the `host-isolates` feature**,
/// rather than silently stillborn. See [`ISOLATE_PLANE_ENV`] and the feature's own comment
/// in `Cargo.toml`: the feature governs *linkage*, the variable governs *runtime*, and a
/// build that cannot serve what it was asked for says so instead of pretending.
///
/// # Errors
/// [`Status::Unsupported`], naming what this build cannot do.
pub fn isolate_factory(
    plane: IsolatePlane,
) -> Result<Box<dyn kayfabe_isolate::IsolateFactory>, (Status, &'static str)> {
    match plane {
        IsolatePlane::Stillborn => Ok(Box::new(kayfabe_isolate::StillbornIsolates::new(
            STILLBORN_WHY,
        ))),
        #[cfg(feature = "host-isolates")]
        IsolatePlane::Loopback => Ok(Box::new(kayfabe_isolate_host::HostIsolateFactory::new(
            kayfabe_isolate_host::RmMode::Loopback,
        ))),
        #[cfg(feature = "host-isolates")]
        IsolatePlane::Real => Ok(Box::new(kayfabe_isolate_host::HostIsolateFactory::new(
            kayfabe_isolate_host::RmMode::Real,
        ))),
        #[cfg(not(feature = "host-isolates"))]
        IsolatePlane::Loopback | IsolatePlane::Real => Err((
            Status::Unsupported,
            "KAYFABE_ISOLATES asked for a host isolate plane, and this archive was built \
             without the `host-isolates` feature — it does not link \
             `kayfabe-isolate-host` and cannot spawn anything. Rebuild with \
             `--features kayfabe-qemu-raw/host-isolates`.",
        )),
    }
}

/// The object model's guest-physical window. See [`object_policy`] for why it is a
/// constant today and why it must stop being one.
///
/// ⚠ Deliberately **not** near the top of the 48-bit space. `kvm_gpa_limited_by_cpu_paddr_bits`
/// is a trap this project measured on 2026-07-24 (memory
/// `kvm_gpa_limited_by_cpu_paddr_bits`): a hardcoded `0x9000_0000_0000` works on the
/// 48-bit AMD dev box and fails on a 46-bit Intel one, and the failure surfaces as an
/// allocator message that blames the allocator. 64 GiB is above every guest RAM size this bench uses and
/// inside 40 bits, so it cannot be the thing that differs between two hosts.
const OBJECT_GPA_WINDOW: core::ops::Range<u64> = 0x10_0000_0000..0x20_0000_0000;

/// Per-proc arena width inside [`OBJECT_GPA_WINDOW`] — 4 GiB, so the window holds 16.
const OBJECT_GPA_ARENA: u64 = 0x1_0000_0000;

/// A base-address-register index the plane can express.
///
/// ★ Saturating rather than refusing, and this is the one place in this file where that is
/// the right call: this is the *hot path*, reached from a vCPU with no error channel, and a
/// register index above 255 cannot come from a PCI device at all — the hypervisor derives
/// it from its own region table. The register model's own `decode_reg` answers `None` for
/// any base-address register it does not own, so a wrong index reads as unclaimed rather
/// than as another register's value.
fn clamp_bar(bar: u32) -> u8 {
    u8::try_from(bar).unwrap_or(u8::MAX)
}

/// An access width the plane can express. Anything wider than 8 bytes is 8.
fn clamp_size(size: u32) -> u8 {
    u8::try_from(size).unwrap_or(8)
}
