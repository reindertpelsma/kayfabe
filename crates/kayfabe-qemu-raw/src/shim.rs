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
/// [`KayfabeRegWrite`]: crate::shim_unsafe::KayfabeRegWrite
pub const ABI_VERSION: u32 = 9;

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

/// The realized register plane — what the C shim holds behind its second opaque handle.
#[derive(Debug)]
pub struct Regs {
    plane: RegPlane,
    /// ★★★ The object bridge's refusal census, kept **here** because the policy that owns
    /// it is boxed into the chain and is unreachable afterwards. See
    /// [`kayfabe_rmrpc::SharedRefusalCensus`] for the boot that had to be diagnosed by the
    /// absence of a line instead.
    refusals: kayfabe_rmrpc::SharedRefusalCensus,
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
        let (objects, refusals) = object_policy(abi.driver)?;
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
        Ok(Regs { plane, refusals })
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
            commands,
            commands_unserviced,
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
        KayfabeRegAudit {
            reads,
            writes,
            boot_reg_reads,
            ptimer_reads,
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
            commands,
            commands_unserviced,
            unserviced_len: sample.len() as u64,
            unserviced,
            bridge_refusals,
            bridge_refusal_len,
            bridge_refusal,
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
/// 2. **`StillbornIsolates`** spawns nothing. There is no forwarding plane in this build,
///    and no host GPU on the bench that measured the wall, so every isolate is retired at
///    birth and every verb refuses through the core's own backpressure path. ⊘ A verb that
///    *succeeded* here would be the mock wall in the product.
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
);

fn object_policy(
    driver: kayfabe_abi::versions::DriverAbiTable,
) -> Result<ObjectLink, (Status, &'static str)> {
    let isolates = kayfabe_isolate::StillbornIsolates::new(
        "this build has no forwarding plane: the object model accepts protocol facts and \
         no host verb can be issued",
    );
    let gpu = kayfabe_core::gpu::Gpu::new(
        Box::new(kayfabe_chips::Ga10xArch::new()),
        Box::new(isolates),
        kayfabe_core::gpa::GpaSpace::new(OBJECT_GPA_WINDOW, OBJECT_GPA_ARENA),
    )
    .map_err(|_| {
        (
            Status::Unsupported,
            "the object model could not realize: its guest-physical window cannot supply \
             the system proc an arena",
        )
    })?;
    let policy = kayfabe_rmrpc::ObjectPolicy::new(
        &driver,
        // ★ The fourth axis, DECLARED and never sniffed. The guest OS is a `#define` in
        // the guest driver's build and is undetectable on the wire, so a port that
        // inferred it would be inferring an isolation boundary from a coincidence. This
        // build answers as the bench's guest, and the day it must answer as another one,
        // this becomes a realize-time property beside the driver version — not an `if`.
        kayfabe_abi::GuestOs::Linux,
        gpu,
    );
    // ★ The handle is taken BEFORE the policy is boxed, because afterwards there is no
    // `ObjectPolicy` left to ask — that is the whole reason the census had to become a
    // shared store rather than a field behind `&self`.
    let refusals = policy.refusal_census();
    Ok((Box::new(policy), refusals))
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
