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
/// ★ Bumped to **14** at `execution_plane_increments.md` **§8.2.2**, the ABI-3 reason a
/// tenth time: [`KayfabeRegAudit`] gained the four GPFIFO-ring census fields, so an
/// ABI-13 shim would allocate the old layout and this archive would write 32 bytes past
/// the end of it.
///
/// ⊘ [`KayfabeRegWrite`] did **not** grow, for E1's reason rather than E2's: the address
/// a channel declares for its ring is a property of an RPC, not of any one register
/// write, and stamping it per-access would make an operator read it thousands of times
/// and still not know how many rings there were.
///
/// ★ Bumped to **15** for the control census, the ABI-3 reason an eleventh time:
/// [`KayfabeRegAudit`] gained the served-control rows and the notifier-arming rows
/// ([`SERVED_CONTROL_SLOTS`], [`NOTIFIER_ARMING_SLOTS`]), so an ABI-14 shim would allocate
/// the old layout and this archive would write past the end of it.
///
/// ★ Bumped to **16** when the notifier probe moved from a process env var to the
/// `probe-arm-notifier` **device property**: `kayfabe_shim_regs_create` gained the
/// probe-string arguments (a signature change is an ABI change even with no struct
/// growth), and [`KayfabeRegAudit`] gained `probe_arm_len` / `probe_arm` so the boot's
/// own report states the probe set it actually ran with — three boots ran probe-off
/// while looking armed from the launching shell, which is the failure the property and
/// the report field jointly kill.
///
/// ★ Bumped to **17** for the VA-space page-directory publication census, the ABI-3 reason
/// a twelfth time: [`KayfabeRegAudit`] gained three counters and
/// [`GVAS_PUBLICATION_SLOTS`] × [`KayfabeGvasPublication`] rows, so an ABI-16 shim would
/// allocate the old layout and this archive would write well past the end of it.
///
/// ⊘ [`KayfabeRegWrite`] did **not** grow, for E1's reason rather than E2's: a page-
/// directory publication is a property of an RPC over a whole boot, not of any one
/// register write, and stamping it per-access would make an operator read it thousands of
/// times and still not know how many address spaces there were.
///
/// ★ Bumped to **19** at `execution_plane_increments.md` **§14.15 / E10e item (c)**, the
/// ABI-3 reason a thirteenth time, and in **both** structures at once. [`KayfabeRegAudit`]
/// gained `doorbell_local_serving` ([`KayfabeDoorbellServing`]), so an ABI-18 shim would
/// allocate the old layout and this archive would write past the end of it;
/// [`KayfabeRegWrite`]'s `doorbell` field gained a fourth value
/// (`DOORBELL_SERVED_LOCAL`) — which alone would not need a bump, but a shim that did not
/// know the value would print a shell-executed copy as an ordinary forwarded one, and a
/// report that cannot tell emulation from forwarding is the one thing this device's
/// evidence must never do.
///
/// ★★ **`KayfabeRegWrite` grew a VALUE and not a field, for E2's reason.** Which doorbell
/// served a submission is a property of *that write*, at *that* instant, and the whole
/// point of the timestamped per-doorbell line is attribution.
///
/// [`KayfabeRegWrite`]: crate::shim_unsafe::KayfabeRegWrite
/// ★ Bumped to **20** for the channel-bind census, the ABI-3 reason a fourteenth time:
/// [`KayfabeRegAudit`] gained two counters and [`CHANNEL_BIND_SLOTS`] ×
/// [`KayfabeChannelBind`] rows, so an ABI-19 shim would allocate the old layout and this
/// archive would write well past the end of it.
///
/// ⊘ [`KayfabeRegWrite`] did **not** grow, for E1's reason rather than E2's: which engine
/// a channel is bound to is a property of an RPC over a whole boot, not of any one
/// register write, and stamping it per-access would make an operator read it thousands of
/// times and still not know how many channels there were.
///
/// [`DOORBELL_SERVED_LOCAL`]: crate::shim_unsafe::DOORBELL_SERVED_LOCAL
/// ★ Bumped to **23** for the ledger-saturation repair, the ABI-3 reason a fifteenth time:
/// [`UNSERVICED_SLOTS`] and [`SERVED_CONTROL_SLOTS`] both went 32 → 64, so an ABI-22 shim
/// would allocate the old layout and this archive would write 512 bytes past the end of the
/// unserviced array alone.
///
/// ⊘ The width is the smaller half of the change. `unserviced_len` now carries the **true**
/// distinct count rather than the sample's clamped length — an ABI-22 reader would have
/// indexed `unserviced[0..unserviced_len]` out of bounds the first time a boot exceeded the
/// cap, which is the second reason this could not be a silent widening.
///
/// ★ Bumped to **28** at §16.6, the ABI-3 reason a sixteenth time and in **two** widths at
/// once: [`GVAS_PUBLICATION_SLOTS`] went 8 → 32 (4 800 bytes of extra rows) and
/// [`DOORBELL_REFUSAL_LEN`] went 448 → 1024 (576 more bytes in each of the two sentence
/// structs). An ABI-27 shim would allocate all three old layouts and this archive would
/// write well past the end of every one of them.
///
/// ⊘ And, as at ABI-23, the width is the smaller half. Both caps were **silent**: the
/// publication array clipped the one row six boots' worth of refusals named
/// (`(0xc1d0000a, 0xcaf00005)` sat past the eighth), and the sentence buffer truncated with
/// no marker, so a clipped refusal read as a complete one. [`copy_sentence`] now stamps a
/// visible `[CLIPPED …]` tail, which is a behaviour change an ABI-27 reader must not see
/// half of.
///
/// ★ Bumped to **29** at §16.8, the ABI-3 reason a seventeenth time: [`DOORBELL_REFUSAL_LEN`]
/// went 1024 → 2048 in **both** sentence structs, so an ABI-28 shim would allocate the old
/// layout and this archive would write 1 024 bytes past the end of each. `[measured, boot
/// `row1_44b7d69`]` the 502-byte sentence that boot emitted is why the 448 before it was not
/// a precaution, and §16.8's framebuffer dump can reach ~1 260 bytes on the refusing path.
///
/// ★ Bumped to **30** at §16.13, the ABI-3 reason an eighteenth time: [`KayfabeRegAudit`]
/// gained the framebuffer residency census (`fb_resident_valid` / `_lo` / `_hi` / `_pages`),
/// so an ABI-29 shim would allocate the old layout and this archive would write 32 bytes
/// past the end of it.
///
/// ★ Bumped to **31** at §16.16, the ABI-3 reason a nineteenth time: [`KayfabeRegAudit`]
/// gained the first-writer census (`fb_origin_by_writer`, five words) and the GPFIFO
/// forward search (`fb_sweep_*`, five words), so an ABI-30 shim would allocate the old
/// layout and this archive would write **80 bytes** past the end of it.
pub const ABI_VERSION: u32 = 31;

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
/// that the hypervisor passes no pointer it has to size.
///
/// ⊘⊘ **This doc used to say "`unserviced_len` reports the truth even when it exceeds this,
/// so a full array is never mistaken for a complete list" — and that was FALSE.**
/// `unserviced_len` was filled from the *sample's* length, which
/// `kayfabe_device::unserviced::UnservicedLog::note` clamps to the cap, so it could never
/// exceed it and a full array read exactly like a complete one. `[measured 2026-08-09]`
/// boot `gt1431_ff7a0ea` printed `32 distinct` from a saturated 32-slot list, and
/// `execution_plane_increments.md` §14.31 concluded from a resulting miss that a control
/// *"never reaches the emulated GSP"*. It does.
///
/// ★ Now true rather than asserted: `unserviced_len` is
/// [`kayfabe_device::unserviced::UnservicedLog::distinct`], which counts before the
/// capacity test, and the C shell prints an explicit truncation line when it exceeds this.
/// The width is 64 as well, so the boot that found this has headroom.
pub const UNSERVICED_SLOTS: usize = 64;

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
///
/// # ★★ 448 → 1024, and the 448 was a SATURATING report nobody had audited
///
/// `[measured 2026-08-09, boot `vaspan_994bbdc`]` the refusal sentence this buffer carried
/// was **292 bytes** of a 448-byte array — 156 of headroom — and §16.6's rung adds the
/// deciding publication's four `PdeLevel`s to it, which is ~180 bytes more. ⇒ ~472 bytes
/// into a 448-byte array: the levels would have been clipped off the END, which is exactly
/// where the new information is. And the copy was a bare `min()`: a clipped sentence and a
/// complete one produced **the same** log line, differing only in that the interesting tail
/// was gone. ⊘ Standing rule (b) — *audit every bounded collection for which side of the
/// boundary it sits on* — and this one sits on the report side, where saturation is
/// indistinguishable from a short answer.
///
/// ⇒ Widened **and** made loud: [`copy_sentence`] stamps a `[CLIPPED …]` tail, so the
/// failure mode is now a visible statement instead of an absence.
///
/// # ★ 1024 → 2048 at §16.8, and the 448 is now MEASURED to have been fatal
///
/// `[measured 2026-08-09, boot `row1_44b7d69`, rev `44b7d69e3`]` the sentence that boot
/// actually emitted is **502 bytes** — `wc -c` over the text after the refusal kind in
/// `traces/guest_boots/run_row1_44b7d69_qemu.log`. ⊘ At the old 448 it would have been cut
/// **54 bytes short, silently**, and the 54 bytes at the end are `L2=…` and `L3=…`: the two
/// deepest published levels, which are half of §16.8's entire finding. The widening was not
/// precautionary.
///
/// §16.8's framebuffer dump adds ~380 bytes of hex and census on the good path — and up to
/// ~760 on the refusing path, because [`fb_level_dump`] carries the **store's own sentence**
/// and `kayfabe_device::fbwin::OUTSIDE_FRAMEBUFFER` alone is ~190 bytes. ⊘ Sized against the
/// **refusing** path, not the good one: a diagnostic that fits only when nothing went wrong
/// is a diagnostic that clips exactly when it is read.
pub const DOORBELL_REFUSAL_LEN: usize = 2048;
/// How many bytes of a doorbell refusal's **kind** the audit carries.
///
/// ★ [`BRIDGE_REFUSAL_TAG_LEN`]'s width and for its reason: a `FaultTag` is a
/// `&'static str` from a fixed finite set, and 64 bytes covers every one of them with room
/// to spare.
pub const DOORBELL_KIND_LEN: usize = 64;

/// How many bytes of a published page-directory LEVEL the §16.8 dump shows.
///
/// ★ 32, because that is the `size` **every one of the eleven publications declares for its
/// root** (`[measured 2026-08-09, boot `row1_44b7d69`]`: `level[0] … size 0x20` on all of
/// them), so a root's dump is the whole root and not a prefix of it. The deeper levels
/// declare `0x1000` and are shown as a 32-byte head plus a non-zero census over the whole
/// page — the census is what answers *"is anything there at all"*, which is §16.8's actual
/// question, and 4 KiB of hex in a refusal sentence would be unreadable and would not fit.
pub const FB_DUMP_HEAD: usize = 32;

/// How many bytes of a level the §16.8 dump COUNTS non-zero bytes over.
///
/// ⊘ A page, because *"the head is zero"* and *"the page is empty"* are different findings
/// and the first is what a 32-byte window can see. A page-directory whose first entries are
/// invalid but whose later ones are not would read as empty through the head alone.
pub const FB_DUMP_CENSUS: usize = 4096;

/// ★★★★ **What OUR framebuffer actually holds at one published level** —
/// `execution_plane_increments.md` §16.8's rung, and it is deliberately a dump rather than
/// a verdict.
///
/// # ⊘ The question, stated so it can only have measured answers
///
/// `[measured 2026-08-09, boot `row1_44b7d69`]` the eleven publications split in two: nine
/// carry roots at `~0x2efa_xxxx` (≈ 11.7 GiB, this GA106's framebuffer size) whose levels
/// **descend**, and two carry four **ascending, consecutive, 4 KiB** pages from `0x0` and
/// from `0x4000` — contiguous with each other, the signature of offsets into one buffer
/// rather than of physical pages. Our walk reads both families as framebuffer physical
/// addresses; the second descends successfully and lands on an unwritten page, which
/// decodes as *"the ring is empty"* instead of faulting.
///
/// ⇒ Two outcomes, two different fixes, and the bytes decide:
///
/// - **plausible page-directory entries at `0x4000`/`0x5000`** ⇒ there is a real pool there
///   and what we lack is its **base**;
/// - **zero, or bytes unrelated to a page directory** ⇒ the walk has been descending
///   **noise** and `V:0x20000` is a coincidence.
///
/// ⊘ **It prints and it concludes nothing.** No base is inferred, no aperture is
/// re-decoded, nothing is emitted the guest did not ask for. `refused=` is its own outcome:
/// an address the store does not back at all is a third answer, and it must not read as
/// zeros ([`kayfabe_device::fbwin::FbStore::read`] returns **zero and `Ok`** for an
/// unwritten address *inside* the framebuffer, so refused and empty are genuinely
/// different facts here).
fn fb_level_dump(plane: &kayfabe_device::plane::RegPlane, label: &str, phys: u64) -> String {
    let mut head = [0u8; FB_DUMP_HEAD];
    let head_s = match plane.fb_peek(phys, &mut head) {
        Err(why) => return format!(" {label}@0x{phys:x}=REFUSED({why})"),
        Ok(()) => head.iter().fold(String::new(), |mut a, b| {
            use core::fmt::Write as _;
            let _ = write!(a, "{b:02x}");
            a
        }),
    };
    // ⊘ The census is a SEPARATE read and its failure is reported separately: a store that
    // backs 32 bytes and refuses the page is a fact, not a reason to drop the head we have.
    let mut page = vec![0u8; FB_DUMP_CENSUS];
    let nz = match plane.fb_peek(phys, &mut page) {
        Err(_) => "?".to_string(),
        Ok(()) => page.iter().filter(|b| **b != 0).count().to_string(),
    };
    // ★★★★ RESIDENCY, beside the byte census — because the byte census ALONE cannot answer
    // the question it looks like it answers. `[measured 2026-08-09, boot `bar1_03a679f`]`
    // the ring's page dumped `nz0/4096`, and a sparse store returns zeros for a page nobody
    // ever wrote, so *"never written"* and *"written with zeros"* produce the identical
    // line. Residency separates them, and ⊘ `res?` — the store cannot say — is a third
    // answer that must not read as either.
    let res = match plane.fb_is_resident(phys) {
        None => "res?",
        Some(true) => "resY",
        Some(false) => "resN-NEVER-WRITTEN",
    };
    // ★★★★ §16.16 — WHO CREATED THIS PAGE, beside whether it exists. `resY` says a write
    // landed; it does not say through which aperture, and *that* is what names a write
    // path. ⊘ Absent (`by-` printed as `by?`) is its own answer and must not read as
    // `UNATTRIBUTED`: the first means the store records no origin for this frame — which
    // for a non-resident frame is simply the truth — while the second is a positive claim
    // that some caller wrote it **without naming itself**. See `kayfabe_device::FbWriter`.
    let by = plane.fb_page_origin(phys).map_or_else(
        || "by?".to_string(),
        |o| format!("by{}#{}", o.by.tag(), o.seq),
    );
    format!(" {label}@0x{phys:x}={head_s} nz{nz}/{FB_DUMP_CENSUS} {res} {by}")
}

/// ★★★★ **The §16.8 dump, for the REFUSING row and for a CONTROL row chosen from the
/// table** — `L0` and `L1` of each.
///
/// # ⊘ The control is DERIVED, never written down
///
/// §16.8's rung names `0x2efa9b000` — the CeUtils VA space's `levels[1]` in boot
/// `row1_44b7d69`. ⊘ That number **may not be hard-coded**: the guest's physical memory
/// allocator re-allocates every boot, and §14's own proof that our translation is real
/// rather than constant was that one VA resolved to two different physical addresses across
/// two boots. A literal here would read correctly on exactly one boot and silently dump an
/// unrelated page on every other, which is `a_table_does_not_decide_behaviour` wearing a
/// hex number.
///
/// So the control is picked **from the publication table**: the first row whose root
/// differs from the refusing row's, printed **with its own `(hClient, hObject)`** so a
/// reader can see which VA space they are comparing against rather than trusting that the
/// right one was chosen. ⊘ If there is no other row, the comparison is stated absent — an
/// empty control must not read as a matching one.
fn fb_dump_pair(
    plane: &kayfabe_device::plane::RegPlane,
    pubs: &kayfabe_device::gvaspub::GvasPubSnapshot,
    client: u32,
    vaspace: u32,
) -> String {
    let Some(bad) = pubs.roots.get(&(client, vaspace)) else {
        return String::new();
    };
    let bad_root = bad.pdes.root().phys_address;
    let mut out = fb_level_dump(plane, "fbL0", bad_root);
    if bad.pdes.num_levels > 1 {
        out.push_str(&fb_level_dump(
            plane,
            "fbL1",
            bad.pdes.levels[1].phys_address,
        ));
    }
    // ⊘ The control names ITSELF. A dump labelled only "control" is a dump whose subject the
    // reader has to infer, and inferring which VA space a number came from is exactly what
    // §16.2 wall 1 cost a boot.
    match pubs
        .roots
        .iter()
        .find(|(_, p)| p.pdes.root().phys_address != bad_root && p.pdes.num_levels > 1)
    {
        None => out.push_str(" ctl=NO-OTHER-ROOT-PUBLISHED"),
        Some(((cc, co), p)) => {
            out.push_str(&format!(" ctl=0x{cc:x}/0x{co:x}"));
            out.push_str(&fb_level_dump(plane, "ctlL0", p.pdes.root().phys_address));
            out.push_str(&fb_level_dump(
                plane,
                "ctlL1",
                p.pdes.levels[1].phys_address,
            ));
        }
    }
    out
}

/// ★★★ **Copy a diagnostic sentence into a fixed wire buffer, and SAY SO when it did not
/// fit** — returning the number of bytes written.
///
/// # ⊘ Why a clipped sentence must not look like a short one
///
/// Every sentence buffer in this ABI was filled by `let take = s.len().min(LEN)` and a
/// `copy_from_slice`. That is byte-correct and **diagnostically silent**: a 500-byte
/// refusal in a 448-byte array printed 448 bytes with nothing to say it had been cut, so
/// an operator reading a boot log sees a sentence that ends early and reads it as *the
/// whole finding*. This project has now paid for that shape nine times in one night under
/// other names (a fixture that normalised the field away, an eight-row sample used as a
/// lookup, two ledgers full at their caps) — the general rule being: **a bounded
/// collection must be able to report its own saturation**, or absence and truncation are
/// the same observation.
///
/// ★ The marker carries the sentence's **true length**, so the reader learns not only that
/// it was clipped but by how much — which is what decides whether the buffer needs widening
/// or the sentence needs shortening.
///
/// ⊘ Truncation lands on a **character** boundary, never a byte: these sentences carry
/// `⊘`, `★` and `—`, and a cut mid-UTF-8 prints as a replacement character in the one line
/// an operator reads.
///
/// ⚠ The marker is ASCII by construction, so appending it can never itself split a
/// character. In the degenerate case where the buffer is too small to hold even the marker,
/// the marker's own head wins the buffer: a reader must always be able to tell that
/// something was dropped, and *"nothing legible fits"* is still that statement.
#[must_use]
pub fn copy_sentence(dst: &mut [u8], s: &str) -> u64 {
    if s.len() <= dst.len() {
        dst[..s.len()].copy_from_slice(s.as_bytes());
        return s.len() as u64;
    }
    let marker = format!(" [CLIPPED, sentence was {} bytes]", s.len());
    let mb = marker.as_bytes();
    if mb.len() >= dst.len() {
        let take = dst.len();
        dst[..take].copy_from_slice(&mb[..take]);
        return take as u64;
    }
    let mut take = dst.len() - mb.len();
    while take > 0 && !s.is_char_boundary(take) {
        take -= 1;
    }
    dst[..take].copy_from_slice(&s.as_bytes()[..take]);
    dst[take..take + mb.len()].copy_from_slice(mb);
    (take + mb.len()) as u64
}

/// ★★★★ **The WHOLE publication row for one `(hClient, hVASpace)`, all levels** — the
/// instrument §16.6 is, and the one thing six consecutive boots could not print.
///
/// # ⊘ Why the root address alone was not enough, and it is a MEASURED gap
///
/// `[measured 2026-08-09, boots `uvm1_b731e3c` … `vaspan_994bbdc`]` every one of those
/// boots refused the same doorbell and named the same pair —
/// `(hClient 0xc1d0000a, hVASpace 0xcaf00005)` — and every one of them printed its root as
/// `0x4000/ap1/sh47` and nothing else, while the eight-row census sample stopped before the
/// row itself (§16.3 fixed the *lookup*, not the *report*). §16.5's anomaly is that
/// `0x4000` sits nowhere near the `~0x2efa_xxxx` every other root in the boot occupies,
/// and separating its three causes needs fields the root projection does not carry:
///
/// | field printed here | the outcome it separates |
/// |---|---|
/// | `arm` (`cmd`) | **decoded from the wrong arm** — `0x90f10106` is a client VA space, `0x20800a9f` is the GPU group's global one, and only the first names a `hVASpace` in its header |
/// | `x` (`count`) | **a STALE publication last-write-wins picked**: `> 1` means this pair was published more than once and the table kept the later body |
/// | `L0.size` | a **real root RM had not yet backed** — `[measured]` every healthy root in the boot publishes `size 0x20`, i.e. 32 bytes of root PDE, so a different size is a different kind of object |
/// | `L1..L3` | whether the levels *below* the root are the same `~0x2efa_xxxx`/`0x1000` shape as a working VA space's, or move with the root |
///
/// ⊘ Read out of [`kayfabe_device::gvaspub::GvasPubSnapshot::roots`] — the **same** map
/// `kayfabe_device::ceresolve::published_root` looks in — so the row printed is by
/// construction the row that decided the walk, not a second projection that can disagree
/// with it. `execution_plane_increments.md` §16.2 wall 1 was exactly two projections of one
/// fact disagreeing, with the weaker one load-bearing.
///
/// ★ An **absent** row states the table's completeness beside itself. *"No row for this
/// pair"* means *"the guest never published one"* only while
/// [`kayfabe_device::gvaspub::GvasPubSnapshot::roots_refused`] is zero; §16.3 is the boot
/// where that distinction was the whole bug, and a reader must not have to go and find the
/// other line to know which sentence they are reading.
///
/// ⊘ `pub` so a test can drive the FORMATTER without a guest — and that is all such a test
/// proves. Whether this string reaches a boot log is decided by
/// [`Shim::addressing_probe`]'s caller and by [`DOORBELL_REFUSAL_LEN`], and the only oracle
/// for that is a boot. (Observability failure #6 of 2026-08-09: an acceptance predicate
/// satisfied by a test calling the function directly.)
#[must_use]
pub fn publication_row(
    pubs: &kayfabe_device::gvaspub::GvasPubSnapshot,
    client: u32,
    vaspace: u32,
) -> String {
    let Some(p) = pubs.roots.get(&(client, vaspace)) else {
        return format!(
            " row=ABSENT-FROM-ROOT-TABLE({} rows, {} REFUSED-BY-CAP)",
            pubs.roots.len(),
            pubs.roots_refused
        );
    };
    let mut levels = String::new();
    // ⊘ `num_levels` and NOT `levels.len()`: entries at or past it are decoded so the
    // re-encode is faithful and carry no meaning (`kayfabe_abi::gvaspacepdes`), so printing
    // them would put addresses in the log that the guest never claimed. Clamped because the
    // count came off the wire.
    let n = (p.pdes.num_levels as usize).min(p.pdes.levels.len());
    for (i, lv) in p.pdes.levels.iter().take(n).enumerate() {
        levels.push_str(&format!(
            " L{i}=0x{:x}/sz0x{:x}/ap{}/sh{}",
            lv.phys_address, lv.size, lv.aperture, lv.page_shift
        ));
    }
    format!(
        " row=arm0x{:08x} x{} lv{}/{} pgsz0x{:x} sd0x{:x}/{} va[0x{:x}..0x{:x}]{levels}",
        p.cmd,
        p.count,
        p.pdes.num_levels,
        p.pdes.levels.len(),
        p.pdes.page_size,
        p.pdes.h_subdevice,
        p.pdes.subdevice_id,
        p.pdes.virt_addr_lo,
        p.pdes.virt_addr_hi,
    )
}

/// How many distinct `(cmd, rpc_result)` served-control rows [`KayfabeRegAudit`] carries.
///
/// ★ Matches `kayfabe_device::census::SERVED_SAMPLE_MAX`. Here the claim really does hold:
/// `served_len` is `CensusSnapshot::served_distinct`, a counter kept beside the sample and
/// incremented before the capacity test — which is exactly what
/// [`UNSERVICED_SLOTS`]'s length was not. `[measured 2026-08-09]` boot `gt1431_ff7a0ea`
/// reported 32 distinct served rows against a 32-slot array, so the next control this port
/// served would have been counted and not shown; 64 is that headroom.
pub const SERVED_CONTROL_SLOTS: usize = 64;

/// How many distinct notifier-arming rows [`KayfabeRegAudit`] carries.
pub const NOTIFIER_ARMING_SLOTS: usize = 16;

/// How many distinct channel-bind rows [`KayfabeRegAudit`] carries.
///
/// ★ Matches `kayfabe_device::census::BIND_SAMPLE_MAX`, and `bind_len` reports the truth
/// even when it exceeds this — a full array is never mistaken for a complete list.
pub const CHANNEL_BIND_SLOTS: usize = 16;

/// The `ce_index` for a bind naming something that is not a copy engine, or whose params
/// were too short. Mirrors `kayfabe_device::census::BIND_NOT_A_COPY_ENGINE`.
///
/// ⊘ Not `0`: `0` is CE0, and CE0 is one of the two indices this chip's captured interrupt
/// table publishes with `vectorNonStall = INVALID`.
pub const BIND_NOT_A_COPY_ENGINE: u32 = 0xFFFF_FFFF;

/// The `rpc_result` recorded for an arming **no policy answered** (the FSM refused it by
/// name), and for an arming field the params were too short to hold.
///
/// ⊘ Deliberately not `0`: `0` is `NV_OK`, and *"nothing answered"* must never read as
/// *"served fine"*. Mirrors `kayfabe_device::census::ARMING_NO_REPLY`.
pub const CTRL_NO_REPLY: u32 = 0xFFFF_FFFF;

/// One row of the served-control census, in the wire shape: a control, the `rpc_result` it
/// was answered with, and how often.
///
/// ★★★ **The half of the command stream the unserviced list is structurally blind to.**
/// A refusal that ANSWERS (`rpc_result != 0`, e.g. `InitTablePolicy::refuse()`) never
/// reaches the terminal ledger — `0x20800301` was the control named in the guest line that
/// killed a boot while being absent from every list the report printed. Keyed on the
/// **pair**: one control can be served `NV_OK` and later refused, and folding those rows
/// together would erase exactly that distinction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct KayfabeServedControl {
    /// The `NV*_CTRL_CMD_*` id.
    pub cmd: u32,
    /// The `rpc_result` answered. `0` = served; non-zero = served-but-REFUSED.
    pub rpc_result: u32,
    /// How many times this exact pair was answered.
    pub count: u64,
}

/// One row of the notifier-arming census (`0x20800301`), in the wire shape.
///
/// ★★ The handles are the point: RM's already-armed rule is per-subdevice
/// (`ogkm-580: subdevice_ctrl_event_kernel.c:126-131`), and these rows are what MEASURED
/// the device's old device-global `notify_actions` aliasing two subdevices' armings of one
/// index (boot `census_probe35` at `6c51da7` — served then refused `0x56`, two rows,
/// different `object` handles). The state is per-subdevice now; the handles stay in the
/// rows so the same regression would reprint the same two-row signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct KayfabeNotifierArming {
    /// `hClient` from the control header.
    pub client: u32,
    /// `hObject` — the subdevice the arming arrived on.
    pub object: u32,
    /// The notifier index, or [`CTRL_NO_REPLY`] if the params were too short to hold one.
    pub event: u32,
    /// The action, with the same too-short marker.
    pub action: u32,
    /// The `rpc_result` answered, or [`CTRL_NO_REPLY`] if no policy answered.
    pub rpc_result: u32,
    /// Padding, so the layout is the same on every ABI that cares.
    pub reserved: u32,
    /// How many times this exact row arrived.
    pub count: u64,
}

/// One row of the channel-bind census (`0xa06f0104`), in the wire shape.
///
/// ★★★ **This is the only place the scrubber's chosen copy engine becomes observable to
/// this device.** `ceutilsGetFirstAsyncCe` picks it inside the guest
/// (`ogkm-580: ce_utils.c:66-81`) and `kchannelBindToRunlist_IMPL` RPCs it to us as
/// `engineType` (`ogkm-580: kernel_channel.c:2762-2785`). Which CE that is decides whether
/// a non-stall interrupt vector exists for it at all — the captured `GA106_INTR_TABLE`
/// gives CE0 and CE1 `vectorNonStall = INVALID` and CE2/CE3/CE4 a real vector.
///
/// See `kayfabe_device::census::ChannelBind` for why the answer cannot be inferred from
/// the device-info table this port itself serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct KayfabeChannelBind {
    /// `hClient` from the control header.
    pub client: u32,
    /// `hObject` — the channel being bound.
    pub object: u32,
    /// `engineType` in **`NV2080_ENGINE_TYPE` space**, raw, or [`CTRL_NO_REPLY`] if the
    /// params were too short to hold one.
    pub engine_type: u32,
    /// Which copy engine that names, or [`BIND_NOT_A_COPY_ENGINE`].
    pub ce_index: u32,
    /// The `rpc_result` answered, or [`CTRL_NO_REPLY`] if no policy answered.
    pub rpc_result: u32,
    /// Padding, so the layout is the same on every ABI that cares.
    pub reserved: u32,
    /// How many times this exact row arrived.
    pub count: u64,
}

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

/// ★★★ **E10e — a doorbell the SHELL served itself, in the wire shape**: one sentence
/// naming what the CPU copy-engine executor did.
///
/// ⊘ **A separate structure from [`KayfabeDoorbellRefusal`] rather than a reuse of it.**
/// The two carry the same bytes and mean opposite things, and a header in which a serving
/// is declared as a refusal is a header that reads as a bug to the next person — the same
/// "two facts, two types" argument [`kayfabe_device::DoorbellReport`]'s third arm makes one
/// crate over. It carries no `kind`, because there is only one way to be served locally and
/// a constant name would be a field that never varies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct KayfabeDoorbellServing {
    /// The sentence's bytes, NUL-padded.
    pub text: [u8; DOORBELL_REFUSAL_LEN],
    /// How many bytes of [`Self::text`] are the sentence.
    pub len: u64,
    /// ⊘ Non-zero exactly when the shell served a doorbell itself — the validity flag, for
    /// [`KayfabeDoorbellRefusal::present`]'s reason.
    pub present: u64,
}

impl Default for KayfabeDoorbellServing {
    fn default() -> KayfabeDoorbellServing {
        KayfabeDoorbellServing {
            text: [0; DOORBELL_REFUSAL_LEN],
            len: 0,
            present: 0,
        }
    }
}

/// How many distinct VA-space page-directory publications [`KayfabeRegAudit`] carries.
///
/// ★ Matches `kayfabe_device::gvaspub::GVAS_PUBLICATION_SAMPLE_MAX`, and `gvas_pub_len`
/// reports the truth even when it exceeds this — a full array is never mistaken for a
/// complete list.
///
/// ★★★ **8 → 32 at §16.6**, and the eight was hiding the row the whole rung is about:
/// `[measured 2026-08-09]` six consecutive boots published **11 distinct** VA spaces and
/// printed the first eight, so `(hClient 0xc1d0000a, hObject 0xcaf00005)` — the pair every
/// one of those boots names in its doorbell refusal — had its body printed in **none** of
/// them. See `kayfabe_device::gvaspub::GVAS_PUBLICATION_SAMPLE_MAX`.
pub const GVAS_PUBLICATION_SLOTS: usize = 32;

/// `GMMU_FMT_MAX_LEVELS` — the `levels[]` bound the publication's own ABI declares
/// (`ogkm-580: ctrl/ctrl90f1.h:37`).
pub const GVAS_MAX_LEVELS: usize = kayfabe_abi::gvaspacepdes::GMMU_FMT_MAX_LEVELS;

/// One published page-directory level, in the wire shape.
///
/// ⊘ `page_shift` is widened from the `NvU8` it is on NVIDIA's wire to a `u32` here. This
/// is **our** structure, not theirs — the narrowing that matters already happened in
/// `kayfabe_abi::gvaspacepdes::PdeLevel` — and a `u8` would have put three bytes of
/// implicit padding into a layout that is hand-mirrored in C.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct KayfabePdeLevel {
    /// Physical address of this level instance. ⚠ A **guest** physical address, in the
    /// guest's own frame of reference; nothing here translates it.
    pub phys_address: u64,
    /// Bytes allocated for this level instance.
    pub size: u64,
    /// `GMMU_APERTURE_*`. ★ A real fork and not decoration: the receiver maps
    /// `GMMU_APERTURE_VIDEO → ADDR_FBMEM` and `SYS_{COH,NONCOH} → ADDR_SYSMEM` and asserts
    /// on anything else (`ogkm-580: gpu_vaspace.c:4503-4511`).
    pub aperture: u32,
    /// The level's page shift. `[measured 2026-08-08]` on GA106 the four levels are
    /// `47, 38, 29, 21` (`traces/real_ga106/`, the §14.9 census).
    pub page_shift: u32,
}

/// ★★★ **One VA-space page-directory publication, in the wire shape** — `0x90f10106` /
/// `0x20800a9f`, the guest telling us where its page directories live.
///
/// `[measured 2026-08-08]` over `traces/real_ga106/rpc_transcript_real_ga106.txt` (a real
/// 580.159.04 driver on a real GA106): `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` — the only
/// control the port turns into a page-directory base — occurs **zero** times in the whole
/// boot, while these two ids occur four and one times respectively. So this row is the
/// *only* thing a boot can say about its own address spaces, and until it existed the port
/// decoded these publications, answered them `NV_OK`, and dropped the value.
///
/// ★★ [`Self::object`] is what makes a row mean anything: the client arm is issued with
/// `rmCtrlParams.hObject = hVASpace` (`ogkm-580: gpu_vaspace.c:5174-5177`), so the RPC
/// header — not the params — names *which* VA space these levels root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct KayfabeGvasPublication {
    /// `0x90f10106` (a VA space under a client's device) or `0x20800a9f` (the GPU group's
    /// global VA space). Kept apart because the two arms are chosen on *who owns the VA
    /// space*.
    pub cmd: u32,
    /// `hClient` from the RPC control header.
    pub client: u32,
    /// ★★★ `hObject` — **the VA space itself**.
    pub object: u32,
    /// How many of [`Self::levels`] are meaningful. `4` on GA106.
    pub num_levels: u32,
    /// VA coverage of the level being reserved.
    pub page_size: u64,
    /// First GPU VA of the reserved range.
    pub virt_addr_lo: u64,
    /// **Last** GPU VA of the range, inclusive — so `hi + 1` is what is page-aligned.
    pub virt_addr_hi: u64,
    /// `hSubDevice`; `0` means *"use `subdevice_id`"*.
    pub h_subdevice: u32,
    /// `subDeviceId`.
    pub subdevice_id: u32,
    /// How many times this exact row arrived.
    pub count: u64,
    /// The published levels. ★ **`levels[0]` is the ROOT** —
    /// `_gvaspacePopulatePDEentries` fills them top-down from `pFmt->pRoot`
    /// (`ogkm-580: gpu_vaspace.c:3974-4031`) and the receiver consumes them bottom-up
    /// (`:4492`). Entries at or past [`Self::num_levels`] carry no meaning.
    pub levels: [KayfabePdeLevel; GVAS_MAX_LEVELS],
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// ★★★★ **The framebuffer's residency EXTENT** — the lowest and highest resident
    /// addresses, and the page count, beside the byte total.
    ///
    /// # ⊘ Why a total was not enough, and it is a MEASURED gap
    ///
    /// `[measured 2026-08-09, boot `bar1_03a679f`]` the report said `resident 368640 bytes`
    /// — 90 pages — and the boot existed to answer *"is the ring's page one of them?"*,
    /// which a total cannot. A total is a summary of a set; the **set** is what decides,
    /// and its shape (clustered or spread) is what says whether the resident pages came
    /// from one write path or several.
    ///
    /// ⊘ **`fb_resident_valid` is the precondition and it is carried, not implied.** A
    /// store that backs no memory at all has no residency to report, and `lo = hi = 0`
    /// would be a positive claim about a device with no framebuffer port — the same error
    /// as decoding an empty capture to zeros. Zero here means *"there was no store to
    /// ask"*, and the C shell prints a different sentence for it.
    pub fb_resident_valid: u64,
    /// The lowest resident framebuffer address. Meaningless unless
    /// [`Self::fb_resident_valid`] is non-zero **and** [`Self::fb_resident_pages`] is.
    pub fb_resident_lo: u64,
    /// The highest resident framebuffer address, same conditions.
    pub fb_resident_hi: u64,
    /// How many 4 KiB pages are resident — the same fact as
    /// [`Self::fb_resident_bytes`] / 4096, carried so the C shell need not divide and so a
    /// disagreement between the two is visible.
    pub fb_resident_pages: u64,
    /// ★★★★ §16.16 — **the first-writer census**: how many resident pages each writer was
    /// FIRST to touch, indexed by `kayfabe_device::FbWriter::index` (PRAMIN, BAR1, BAR2,
    /// EXEC, UNATTRIBUTED).
    ///
    /// # ⊘ Read the UNATTRIBUTED slot before reading any other
    ///
    /// `[measured 2026-08-09, tree `e394b69`]` §16.15 built the whole tagging mechanism and
    /// wired **none** of it — `write_tagged` had no caller anywhere in the repo, so every
    /// framebuffer write took `FbStore::write`'s default and recorded `Unattributed`. A
    /// boot of that tree would have printed `UNATTRIBUTED 90` and nothing else. ★ That is
    /// why this array is worth reading as a whole and not as four interesting numbers plus
    /// a remainder: a large `UNATTRIBUTED` slot means *"a write path is not instrumented"*,
    /// which is a fact about **us**, and it must never be read as a fact about the guest.
    ///
    /// ⊘ Precondition: [`Self::fb_resident_valid`]. All-zero from an archive that never
    /// wrote the struct is the honest non-claim, exactly as for the residency extent.
    pub fb_origin_by_writer: [u64; 5],
    /// ★★★★ §16.16 — **the forward search for the ring.** See [`FbRingSweep`] for why the
    /// converse question had to be asked and why it is independent of the walk.
    ///
    /// How many resident frames were swept, out of how many exist. ⊘ The pair is carried so
    /// *"nothing found"* can never be read as *"we looked everywhere"* under truncation.
    pub fb_sweep_swept: u64,
    /// How many swept frames carried at least `RINGLIKE_MIN` GPFIFO-entry-shaped qwords.
    pub fb_sweep_ringlike: u64,
    /// The best-scoring frame's framebuffer address. ⊘ Meaningless unless
    /// [`Self::fb_sweep_ringlike`] is non-zero, and the C shell prints a different sentence
    /// when it is zero rather than printing `0x0` as an address.
    pub fb_sweep_best: u64,
    /// That frame's score.
    pub fb_sweep_best_score: u64,
    /// `kayfabe_device::FbWriter::index` of that frame's first writer **plus one**, so zero
    /// is *"no origin recorded"* and never `PRAMIN`. See [`FbRingSweep::best_writer_plus1`].
    pub fb_sweep_best_writer_plus1: u64,
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
    /// ★★★ §14.18: CE completions this device **announced** with the bound engine's
    /// `vectorNonStall`. See `kayfabe_device::Counters::nonstall_raises`.
    pub nonstall_raises: u64,
    /// ★★★ §14.18: CE completions it could **not** announce. ⊘ The number that must be
    /// zero — every one of them is work that happened and was never notified.
    pub nonstall_unvectored: u64,
    /// ★★ §14.18: of the raises, how many the guest's own `LEAF_EN` would hide from its
    /// non-stall scan. See `kayfabe_device::Counters::nonstall_masked`.
    pub nonstall_masked: u64,
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
    /// ★★★ **E10e** — the **last** doorbell the shell's own CPU copy-engine executor
    /// served, and what it did. See [`kayfabe_device::DoorbellLog::last_local_serving`]
    /// for why this one is last where the refusal above is first.
    pub doorbell_local_serving: KayfabeDoorbellServing,
    /// ★★★ **§8.2.2** — channel allocs whose params declared a GPFIFO ring, decoded and
    /// counted. See `kayfabe_rmrpc::RingCensus` for what the census is *for*; this is its
    /// wire shape.
    ///
    /// ⊘ Counted at TRANSLATION, so an alloc the graph then refused is still counted. The
    /// question this instrument asks is what the **guest** named, not what we accepted.
    pub gpfifo_ring_declarations: u64,
    /// Of those, how many named a **non-zero** ring address.
    pub gpfifo_ring_nonzero: u64,
    /// The first non-zero ring address a channel declared — `gpFifoOffset`, verbatim.
    ///
    /// ★★★ **It is a GPU VIRTUAL address.** `[src]` `ogkm-580: ctrl2080fifo.h:809` names
    /// the field *"Gpfifo Virtual Offset"*, and `mem_utils_gm107.c:1232` computes it as
    /// `pbGpuVA + channelPbSize`. That is a reading of the driver, said as one.
    ///
    /// What this field is *for* is the other half: `kayfabe_arch::PushRange::gpa` feeds an
    /// address of exactly this kind to `Vmm::gpa_read` with no walk, so printing the
    /// number beside the guest's own RAM extent is what turns the reading into an
    /// observation. `[measured]` at rev `c93930d`, boots `e5ring1` / `e5ring2g` —
    /// `docs/design/execution_plane_increments.md` §8.2.3.
    pub gpfifo_ring_va: u64,
    /// `gpFifoEntries` that came with [`Self::gpfifo_ring_va`], or `0` if none did.
    ///
    /// ⊘ [`Self::gpfifo_ring_nonzero`] is the validity flag for both, and it is not
    /// redundant: `gpFifoOffset = 0` is a declaration the driver makes **on purpose**
    /// (`ogkm-580: kernel_graphics.c:2420-2424`), so a single field could not tell
    /// *"declared address zero"* from *"declared nothing"*. Same argument as
    /// [`Self::doorbell_last_token_valid`].
    pub gpfifo_ring_entries: u64,
    /// ★★★ **The served-control census** — every `GSP_RM_CONTROL` a policy answered,
    /// including repeats and rows past [`SERVED_CONTROL_SLOTS`].
    ///
    /// The third state the report could not previously express. `unserviced` says what
    /// nothing answered; `bridge_refusal` says what the object bridge refused by tag; this
    /// says what WAS answered and with what result — so "id absent everywhere" finally
    /// means *never issued* rather than being consistent with served-fine as well.
    pub served_total: u64,
    /// Distinct `(cmd, rpc_result)` rows seen — the truth even past the array.
    pub served_len: u64,
    /// The rows, in first-seen order.
    pub served: [KayfabeServedControl; SERVED_CONTROL_SLOTS],
    /// ★★ Every `0x20800301` arming seen, answered or not, including repeats.
    pub arming_total: u64,
    /// Distinct arming rows seen — the truth even past the array.
    pub arming_len: u64,
    /// The rows, in first-seen order, with the handles they arrived on.
    pub armings: [KayfabeNotifierArming; NOTIFIER_ARMING_SLOTS],
    /// ★★★ Every `0xa06f0104` seen, answered or not, including repeats.
    pub bind_total: u64,
    /// Distinct bind rows seen — the truth even past the array.
    pub bind_len: u64,
    /// The rows, in first-seen order. See [`KayfabeChannelBind`].
    pub binds: [KayfabeChannelBind; CHANNEL_BIND_SLOTS],
    /// ★★★ **The VA-space page-directory publications** — every publication that decoded,
    /// including repeats and rows past [`GVAS_PUBLICATION_SLOTS`].
    ///
    /// See [`KayfabeGvasPublication`] for why this is the only boot-path statement of a
    /// page-directory root at all.
    pub gvas_pub_total: u64,
    /// Distinct publication rows seen — the truth even past the array.
    pub gvas_pub_len: u64,
    /// ⊘ Publications that arrived and **did not decode**. A separate number rather than
    /// an absent row: *"the guest published something we could not read"* and *"the guest
    /// published nothing"* are different diagnoses and only one of them is our defect.
    pub gvas_pub_undecodable: u64,
    /// ★★★★ **Publications the AUTHORITATIVE ROOT TABLE refused** — the number whose
    /// healthy value is zero, and the only thing that says
    /// `kayfabe_device::gvaspub::GvasPubSnapshot::roots` is still COMPLETE.
    ///
    /// ⊘ It crosses because of what its predecessor's absence cost. `[measured 2026-08-09,
    /// boot `uvm1_b731e3c`]` the resolver looked a VA space up in the eight-row *report*
    /// sample while the boot published **11 distinct**, so three address spaces were
    /// answered `CeResolve::NoPublication` — *"the guest published no page-directory
    /// root"* — about a guest that had published one. The table is now separate and holds
    /// `GVAS_ROOT_TABLE_MAX`; this is what makes its completeness an OBSERVATION rather
    /// than an assumption, and a non-zero value invalidates every `NoPublication` refusal
    /// in the same boot.
    pub gvas_pub_roots_refused: u64,
    /// ★★★ **§14.23 — publications the FRONT SEAT saw**, i.e. arrived on one of
    /// `kayfabe_rmrpc::PUBLICATION_CONTROLS`.
    ///
    /// ⊘ Counted by a *different* link from [`Self::gvas_pub_total`] and deliberately not
    /// folded into it: that one is the recorder's (it decodes and logs), this one is the
    /// observer's (it decodes and **declares into the object model**). Two numbers that
    /// should agree, produced independently — so a front seat that was never filled reads
    /// as `0` beside a non-zero `gvas_pub_total` instead of hiding behind it.
    pub gvas_pub_seen: u64,
    /// ★★★ **§14.23 — publications the OBJECT MODEL ACCEPTED.** The number that says
    /// `Vas::pdb` was populated from the guest's own statement, and therefore the number a
    /// claim about the page-directory plane is allowed to cite.
    ///
    /// Its refusals are named in the bridge-refusal census
    /// (`BridgeRefusal::PublishedPdes*`), not here.
    pub gvas_pub_applied: u64,
    /// Translations of a claimed publication control that were not an `RmEvent` —
    /// unreachable by construction and counted rather than asserted, because this runs on a
    /// vCPU thread where a panic aborts the VM.
    pub gvas_pub_unexpected: u64,
    /// The rows, in first-seen order.
    pub gvas_pub: [KayfabeGvasPublication; GVAS_PUBLICATION_SLOTS],
    /// ★ How many notifier indices the `probe-arm-notifier` device property named — the
    /// probe set this boot actually ran with, as recorded by the plane's census at
    /// construction from the same value the event-plane arm consults. `0` in every
    /// shipping boot. Reported so a boot's own output proves its probe set: the
    /// predecessor env var ran three boots probe-off while looking armed from the
    /// launching shell.
    pub probe_arm_len: u64,
    /// The indices, in the order the property named them.
    pub probe_arm: [u32; PROBE_ARM_SLOTS],
    /// ★★★ **§14.41 — replayable fault buffers the guest registered, and this port
    /// ANSWERED `NV_OK` to.** Every arrival of `0x20800a9b`, including repeats.
    ///
    /// It is in the report for one reason, and it is not the count. Answering this control
    /// is what lets `cuInit` past `faultbufConstruct_IMPL`, and it buys **registration
    /// only** — nothing here raises a replayable fault or advances
    /// `MMU_FAULT_BUFFER_PUT(1)`. A served row in the control census reads as *"handled"*,
    /// which is exactly the too-capable-mock reading this project keeps being bitten by, so
    /// the C printer emits [`kayfabe_abi::faultbuffer::DELIVERY_UNBUILT`] beside this number
    /// whenever it is non-zero. ⇒ **Every boot that serves the control also reports what the
    /// control did not buy.**
    ///
    /// ⚠ A value **> 1** is a finding, not noise: the physical receiver returns
    /// `NV_ERR_NOT_SUPPORTED` on a second registration while one is live
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:3117`) and this port does not
    /// model that, deliberately (its `0x20800a9c` partner is unserved, so the state could
    /// only ever latch shut). The repeats are counted here so the day one arrives the
    /// decision is made against a boot's own output rather than against this paragraph.
    pub fault_buffers_registered: u64,
    /// `faultBufferSize` of the FIRST registration, in bytes, or `0` if none decoded.
    ///
    /// ⊘ The first, not the last: a re-registration is the interesting event and
    /// [`Self::fault_buffers_registered`] is what reveals one. Reported beside
    /// [`Self::fault_buffer_pages`] so the two can be checked against each other —
    /// `align_up(size) / 4096` — rather than believed separately.
    pub fault_buffer_size: u64,
    /// How many PTE entries the guest actually filled for that first registration.
    ///
    /// ★ The stock GA106 value is **49**, which is `0x20800a59`'s own advertised
    /// `replayableFaultBufferSize` of `0x31000` divided by `RM_PAGE_SIZE`. A number that is
    /// not 49 on a stock boot means the two controls disagree.
    pub fault_buffer_pages: u64,
    /// Registrations whose params did **not** decode.
    ///
    /// ⊘ Its own counter rather than a silence: *"the guest never asked"* and *"the guest
    /// asked in a shape we could not read"* are different findings, and the second means
    /// this port's layout is wrong.
    pub fault_buffers_malformed: u64,
    /// ★★★ **CLIENT SHADOW fault buffers the guest registered** (`0x20800a9d`), and this port
    /// answered `NV_OK` to.
    ///
    /// ⊘ Counted **separately** from [`Self::fault_buffers_registered`], and the separation is
    /// the point rather than tidiness. The two controls carry different promises: answering
    /// `0x20800a9b` says a register *we* serve will keep reading empty; answering this one says
    /// **we** will write fault packets into pages of the guest's own sysmem
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:1589-1593` — *"GSP will be writing
    /// the fault packets to these buffers"*). One number could not say which promise a boot
    /// took on, and the printer emits a different sentence for each.
    pub shadow_fault_buffers_registered: u64,
    /// `shadowFaultBufferSize` of the FIRST shadow registration, in bytes, or `0`.
    ///
    /// ★ The stock GA106 value is `0x120c20`, which is `0x20800a59`'s own advertised
    /// `nonReplayableFaultBufferSize`. Anything else on a stock boot means the two controls
    /// disagree about a buffer the guest has already allocated.
    pub shadow_fault_buffer_size: u64,
    /// Pages the guest filled for it — `align_up(size)/4096 + align_up(metadataSize)/4096`
    /// (`ogkm-580: kern_gmmu.c:1601`), **289** for the stock size.
    pub shadow_fault_buffer_pages: u64,
    /// `shadowFaultBufferType` of that first registration, **raw**.
    ///
    /// ⚠ `0` is non-replayable and is the only value reachable with Confidential Compute off;
    /// `1` (replayable shadow) needs CC (`ogkm-580: mmu_fault_buffer_ctrl.c:148`), so seeing it
    /// — or anything else — is a **finding** this port deliberately does not refuse on, because
    /// refusing would model a path no measurement has reached.
    pub shadow_fault_buffer_type: u64,
    /// Shadow registrations whose params did **not** decode.
    pub shadow_fault_buffers_malformed: u64,
    /// ★★★ **ACCESS-COUNTER notification buffers the guest registered** (`0x20800a1d`).
    ///
    /// ⊘ A third count, for the third buffer, and this one is the sharpest: it is the only
    /// buffer whose **size** this port also invents (`ga10x`'s
    /// `ACCESS_COUNTER_NOTIFY_BUFFER_ENTRIES_ADVERTISED`, an admitted fiction). The printer
    /// says both halves — we told the guest how big it is, and we never put anything in it.
    ///
    /// ⚠ **`0` here after a `cuInit` is a FINDING, not a quiet success.** The control is only
    /// reachable once BAR0 `0xB83110` stops reading zero; before §14.41 it could never arrive,
    /// so its absence from every previous ledger was evidence of nothing.
    pub access_cntr_buffers_registered: u64,
    /// `bufferSize` of the first access-counter registration, in bytes, or `0`.
    ///
    /// ★ `8192` is what this port's own advertised 256 entries × 32 bytes implies. Anything
    /// else means the register and the registration disagree.
    pub access_cntr_buffer_size: u64,
    /// Pages the guest filled for it — `2` for the advertised size.
    pub access_cntr_buffer_pages: u64,
    /// Access-counter registrations whose params did **not** decode.
    pub access_cntr_buffers_malformed: u64,
}

impl Default for KayfabeRegAudit {
    /// ⊘ Hand-written rather than derived, and the reason is a language bound rather than a
    /// design one: `[T; N]` implements `Default` only up to `N == 32`, and
    /// [`UNSERVICED_SLOTS`] / [`SERVED_CONTROL_SLOTS`] are 64. Every field is its own type's
    /// default, so an all-zero audit still means *"nothing happened"* exactly as before.
    fn default() -> KayfabeRegAudit {
        KayfabeRegAudit {
            reads: Default::default(),
            writes: Default::default(),
            boot_reg_reads: Default::default(),
            ptimer_reads: Default::default(),
            ptimer_writes_refused: Default::default(),
            rom_reads: Default::default(),
            gsp_reads: Default::default(),
            gsp_writes: Default::default(),
            unclaimed_reads: Default::default(),
            unclaimed_writes: Default::default(),
            fb_window_reads: Default::default(),
            fb_window_writes: Default::default(),
            fb_reads: Default::default(),
            fb_writes: Default::default(),
            fb_refusals: Default::default(),
            bar2_reads: Default::default(),
            bar2_writes: Default::default(),
            bar2_faults: Default::default(),
            bar_pde_updates: Default::default(),
            bar2_root_entry: Default::default(),
            bar0_window_reads: Default::default(),
            bar0_window_writes: Default::default(),
            fb_resident_bytes: Default::default(),
            fb_resident_valid: Default::default(),
            fb_resident_lo: Default::default(),
            fb_resident_hi: Default::default(),
            fb_resident_pages: Default::default(),
            fb_origin_by_writer: Default::default(),
            fb_sweep_swept: Default::default(),
            fb_sweep_ringlike: Default::default(),
            fb_sweep_best: Default::default(),
            fb_sweep_best_score: Default::default(),
            fb_sweep_best_writer_plus1: Default::default(),
            faults: Default::default(),
            ram_refusals: Default::default(),
            irq_requests: Default::default(),
            cpu_intr_accesses: Default::default(),
            cpu_intr_raises: Default::default(),
            cpu_intr_masked: Default::default(),
            nonstall_raises: Default::default(),
            nonstall_unvectored: Default::default(),
            nonstall_masked: Default::default(),
            commands: Default::default(),
            commands_unserviced: Default::default(),
            unserviced_len: Default::default(),
            unserviced: [0; UNSERVICED_SLOTS],
            bridge_refusals: Default::default(),
            bridge_refusal_len: Default::default(),
            bridge_refusal: Default::default(),
            isolates_materialized: Default::default(),
            isolates_live: Default::default(),
            isolates_no_plane: Default::default(),
            isolates_spawn_failed: Default::default(),
            isolate_refusal: Default::default(),
            doorbells: Default::default(),
            doorbells_served: Default::default(),
            doorbells_refused: Default::default(),
            doorbell_last_token: Default::default(),
            doorbell_last_token_valid: Default::default(),
            doorbell_refusal: Default::default(),
            doorbell_local_serving: Default::default(),
            gpfifo_ring_declarations: Default::default(),
            gpfifo_ring_nonzero: Default::default(),
            gpfifo_ring_va: Default::default(),
            gpfifo_ring_entries: Default::default(),
            served_total: Default::default(),
            served_len: Default::default(),
            served: [KayfabeServedControl::default(); SERVED_CONTROL_SLOTS],
            arming_total: Default::default(),
            arming_len: Default::default(),
            armings: Default::default(),
            bind_total: Default::default(),
            bind_len: Default::default(),
            binds: Default::default(),
            gvas_pub_total: Default::default(),
            gvas_pub_len: Default::default(),
            gvas_pub_undecodable: Default::default(),
            gvas_pub_roots_refused: Default::default(),
            gvas_pub_seen: Default::default(),
            gvas_pub_applied: Default::default(),
            gvas_pub_unexpected: Default::default(),
            gvas_pub: Default::default(),
            probe_arm_len: Default::default(),
            probe_arm: Default::default(),
            fault_buffers_registered: Default::default(),
            fault_buffer_size: Default::default(),
            fault_buffer_pages: Default::default(),
            fault_buffers_malformed: Default::default(),
            shadow_fault_buffers_registered: Default::default(),
            shadow_fault_buffer_size: Default::default(),
            shadow_fault_buffer_pages: Default::default(),
            shadow_fault_buffer_type: Default::default(),
            shadow_fault_buffers_malformed: Default::default(),
            access_cntr_buffers_registered: Default::default(),
            access_cntr_buffer_size: Default::default(),
            access_cntr_buffer_pages: Default::default(),
            access_cntr_buffers_malformed: Default::default(),
        }
    }
}

/// How many probe-arm indices [`KayfabeRegAudit`] carries — the full
/// [`kayfabe_abi::eventnotify::PROBE_ARM_MAX`], so unlike the sampled censuses this one
/// is never clipped: parse refuses more, so `probe_arm_len` ≤ the array by construction.
pub const PROBE_ARM_SLOTS: usize = kayfabe_abi::eventnotify::PROBE_ARM_MAX;

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

    fn schedule_channel(
        &mut self,
        client: kayfabe_rt::HClient,
        object: kayfabe_rt::HObject,
        enable: bool,
    ) -> Result<kayfabe_core::gpu::ScheduleAck, kayfabe_core::gpu::ScheduleFault> {
        self.0.schedule_channel(client, object, enable)
    }

    fn bind_channel(
        &mut self,
        client: kayfabe_rt::HClient,
        object: kayfabe_rt::HObject,
        rm_engine_type: u32,
    ) -> Result<kayfabe_core::gpu::BindAck, kayfabe_core::gpu::BindFault> {
        self.0.bind_channel(client, object, rm_engine_type)
    }

    /// `None`, and that is the whole of what a sharded shell can honestly say: the graph
    /// lives inside a device lock and a proc lock, and a `&Gpu` handed out here would
    /// outlive both guards.
    fn as_gpu(&self) -> Option<&kayfabe_core::gpu::Gpu> {
        None
    }

    fn as_gpu_mut(&mut self) -> Option<&mut kayfabe_core::gpu::Gpu> {
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
struct SharedDoorbell {
    device: Arc<kayfabe_rt::device::SharedDevice>,
    /// ★★★ The register plane this port is installed in — **weak**, because the plane owns
    /// this port and a strong handle would be a cycle that never frees.
    ///
    /// It is here for one purpose and it is an observing one: when the core refuses a
    /// doorbell, the plane is what can say **why the channel's own addresses do or do not
    /// resolve** — it holds the guest's published page-directory roots
    /// (`kayfabe_device::gvaspub`) and the framebuffer the guest wrote its page tables into
    /// through BAR2. Without it a `NoVas` refusal names the absence and nothing else, and
    /// `execution_plane_increments.md` §14.12 asked for exactly the missing half:
    /// *"are the intermediate entries on the path to `0x4_2000_0000` actually present in our
    /// emulated FB? A miss is a fault."*
    plane: std::sync::Weak<RegPlane>,
    /// ★★★ **E10e** — the shell state the CPU copy-engine executor needs, shared with
    /// [`Regs`] because the two halves are installed at different times: the port is built
    /// at device realize and the guest-memory handle only exists once the memory plane has
    /// a base address (see [`Regs::attach_ram`]).
    ce: Arc<CeShellState>,
    /// ★★★ **§14.24 — is the shell's own CPU copy-engine executor the ONLY executor this
    /// build has?** Decided once, at realize, from [`selected_isolate_plane`].
    ///
    /// # ⊘⊘ Why this replaced a `vas_pdb.is_none()` test, and it is a MEASURED refutation
    ///
    /// [`SharedDoorbell::try_ce_submission`]'s precondition 2 used to read *"`vas_pdb` must
    /// be `None`. A channel the core can address is the core's."* That inference — **the
    /// core can ADDRESS it, therefore the core can SERVE it** — was true only while the
    /// port did not know the channel's address space, and §14.23 made it know.
    ///
    /// `[measured 2026-08-08, boot pub1_3e43e9a, rev 3e43e9a]`: with the guest's own
    /// page-directory publication reaching `Vas::pdb`, `facts.vas_pdb` became `Some` for the
    /// CeUtils scrubber's channel, this executor declined it as *"not ours"*, the doorbell
    /// fell through to a forwarding plane that is **`Stillborn` in every shipping build**,
    /// and the report read `doorbells: 1 arrived, 0 served, 1 REFUSED
    /// [FwdFault::IsolateRetired]` where the previous revision read `2 arrived, 2 served
    /// [CpuCe::ServedLocally]`. `memmgrMemSet` then timed out (`NV_ERR_TIMEOUT 0x65` at
    /// `mem_mgr.c:463`), `ce_utils.c:349` failed its `lastCompletedPayload ==
    /// lastSubmittedPayload` assertion, and `RmInitAdapter failed! (0x25:0x65:1249)`.
    ///
    /// ⇒ **The milestone had been resting on the port's ignorance.** `nvidia-smi` enumerated
    /// a device because this executor served the scrubber's copy, and it served that copy
    /// *because* the channel's address space was unknown to us. A correct fact took the
    /// executor away — which is §14.21's shape exactly, one plane over: an accurate port
    /// state is fatal when a fallback was keyed on the inaccuracy.
    ///
    /// ★ So the question the gate asks is now the question it always meant: not *"can the
    /// core address this channel?"* but **"is there any other executor?"**. When
    /// [`IsolatePlane::Stillborn`] is installed the answer is no, by that plane's own
    /// declared meaning ([`STILLBORN_WHY`]: *"no host verb can be issued"*), and the shell's
    /// CPU executor is not a fallback — it is the executor.
    ///
    /// ⊘ **Not a fallback-after-refusal.** The decision is made from the composition root's
    /// own declared choice, before any doorbell arrives; nothing here retries a refused
    /// submission on a second path. A build that selects a real isolate plane keeps the old
    /// routing exactly — a channel the core can address goes to the core.
    local_ce_is_the_only_executor: bool,
}

/// ★★★ **E10e — what the shell owns on behalf of the CPU copy-engine executor.**
///
/// Two things, and neither of them can live in the register plane:
///
/// - the **guest-memory port**. `kayfabe_rt::cpu_ce` takes a `&mut dyn Vmm` and uses three
///   of its methods (`gpa_read`, `gpa_write`, `raise_irq`). §14.15 obstacle 3 offered two
///   ways to reach it — unify the guest-RAM port across `Vmm` and
///   `kayfabe_device::GuestRam`, or *"the driver runs where the `Vmm` is and the plane
///   hands out its stores"*. This is the second: the executor's signature is unchanged, so
///   its completion interrupt goes through the real hypervisor port rather than through a
///   new one invented for it. ⊘ It is the **same** [`QemuVmm`] handle [`MachineRam`] wraps
///   — one description of guest memory, two users — and it is `None` outside
///   `attach_ram`/`detach_ram`, which is a refusal rather than a null check: a CE
///   submission arriving while the memory plane is detached is refused by name.
/// - the per-channel **GPFIFO cursor**. See `kayfabe_rt::ceutils::GpCursor` for why the
///   ring's read position is state the shell must keep rather than derive.
#[derive(Debug, Default)]
struct CeShellState {
    /// The memory plane, once realized. See the type docs.
    vmm: std::sync::Mutex<Option<QemuVmm>>,
    /// Per `(proc, chan)` GPFIFO read cursors.
    cursors:
        std::sync::Mutex<std::collections::BTreeMap<(u32, u32), kayfabe_rt::ceutils::GpCursor>>,
}

impl core::fmt::Debug for SharedDoorbell {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SharedDoorbell")
            .field("gpu", &DOORBELL_TARGET_GPU.0)
            .field("plane", &self.plane.upgrade().is_some())
            .finish_non_exhaustive()
    }
}

/// The GPU a `nvkvm-gpu` device is. See [`SharedDoorbell`]'s docs.
const DOORBELL_TARGET_GPU: kayfabe_rt::GpuId = kayfabe_rt::GpuId(0);

/// One refused doorbell report, so the three refusal sites in [`SharedDoorbell`] cannot
/// come to disagree about the shape of a refusal.
fn refused(
    token: u64,
    kind: kayfabe_device::FaultTag,
    why: String,
) -> kayfabe_device::DoorbellReport {
    kayfabe_device::DoorbellReport::Refused {
        token,
        refusal: kayfabe_device::DoorbellRefused { kind, why },
    }
}

impl kayfabe_device::DoorbellPort for SharedDoorbell {
    fn ring(&self, token: u64) -> kayfabe_device::DoorbellReport {
        // ★★★ **E10e — the CPU copy-engine branch is tried FIRST, and only for a channel
        // the core cannot serve at all.** `try_ce_submission` answers `None` unless the
        // routed channel has **no `Vas`** (`vas_pdb == None`), which is precisely the case
        // `plan_doorbell` refuses `NoVas`. So no channel changes hands: this arm serves
        // exactly the doorbells that were refused before it existed, and every other
        // doorbell takes the forwarding path below, unchanged.
        if let Some(report) = self.try_ce_submission(token) {
            return report;
        }
        match self.device.doorbell(DOORBELL_TARGET_GPU, token, &[]) {
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
                    // ★★★ The refusal, **plus what this channel's own addresses resolve
                    // to**. See `SharedDoorbell::addressing_probe`.
                    why: format!("{f:?}{}", self.addressing_probe(token)),
                },
            },
        }
    }
}

/// ★★★ **The finishPayload semaphore's offset from the GPFIFO ring's base** — `0x8004`.
///
/// `[src]` `ogkm-580: channel_utils.c:242-250, 671-672`: `gpfifo_va = pbGpuVA +
/// channelPbSize` and `finishPayloadOffset = channelPbSize + GPFIFO_SIZE (0x8000) + 4`, so
/// the difference is `GPFIFO_SIZE + 4` and is **independent of `channelPbSize`** — which is
/// the whole reason it can be derived from the ring address alone. The C artifact derived
/// the same constant and its arithmetic reproduces on our own boot: `0x120064000 + 0x8004 =
/// 0x12006c004`, and the guest printed exactly that
/// (`c_ceutils_ring_resolution.md` §4; `execution_plane_increments.md` §14.11).
const FINISH_PAYLOAD_FROM_RING: u64 = 0x8004;

/// How many bytes of the ring the probe reads — **one** GPFIFO entry.
///
/// ⊘ One, not the ring. The probe's question is *"does this channel's addressing resolve"*,
/// and one entry answers it; reading 4096 entries would be a guest-sized copy performed for
/// a diagnostic, and the first entry is the only one the submission is guaranteed to have
/// written.
const PROBE_RING_BYTES: usize = kayfabe_abi::submit::GP_ENTRY_SIZE as usize;

impl SharedDoorbell {
    /// ★★★ **E10e item (c) — SERVE a doorbell on a VAS-less copy-engine channel, on the
    /// CPU, in the shell.** `None` means *"not ours"*, and the forwarding path runs.
    ///
    /// # ⊘ The four preconditions, and why each one is a refusal to act rather than a check
    ///
    /// 1. **The channel's declared facts must exist** — `ce_channel_facts` failing means the
    ///    token did not route, which is the *core's* refusal to report, not ours.
    /// 2. **The core must be able to SERVE the channel**, not merely address it — i.e.
    ///    `vas_pdb` is `Some` *and* this build installed a forwarding plane. ⊘ The `and` is
    ///    §14.24's correction and it was measured, not reasoned: see
    ///    [`SharedDoorbell::local_ce_is_the_only_executor`] for the boot in which the first
    ///    half alone cost the adapter.
    /// 3. **A published VA space and a declared ring**, or there is nothing to resolve.
    /// 4. **A memory plane.** Between realize and `attach_ram` there is none, and a CE
    ///    submission then is refused by name rather than served out of a null.
    ///
    /// # ⚠ The cursor is committed only on success
    ///
    /// `run_submission` takes the cursor **by value** and hands the advanced one back only
    /// in its success value. A refused submission therefore leaves the ring exactly where it
    /// was, so the guest's own retry (`[measured 2026-08-08, boot
    /// run_p2_c89899a]`: `channelWaitForFinishPayload` retries once before failing) re-reads
    /// the entry it could not run rather than skipping past it. A cursor advanced through a
    /// refusal would turn one loud failure into a silently dropped copy — `#13`'s `CE-DROP`
    /// by another route.
    ///
    /// # ⚠ Lock order: plane, then core
    ///
    /// The plane's session is taken first and `SharedDevice::with_pushbuffer` (rank 0) runs
    /// inside it. That is the established direction — the command-policy chain already calls
    /// the core under the plane's mutex — and it is why the whole executor lives out here
    /// rather than inside `apply_pushbuffer`, which holds a rank-1 proc lock.
    /// ⊘ `ce_channel_facts` is called and **completed** before the plane lock is taken.
    fn try_ce_submission(&self, token: u64) -> Option<kayfabe_device::DoorbellReport> {
        let facts = self
            .device
            .ce_channel_facts(DOORBELL_TARGET_GPU, token)
            .ok()?;
        // ★★★ §14.24 — see `SharedDoorbell::local_ce_is_the_only_executor` for the boot
        // that turned this from `vas_pdb.is_none()` into a question about EXECUTORS.
        if facts.vas_pdb.is_some() && !self.local_ce_is_the_only_executor {
            return None; // the core can address AND serve this channel; it is not ours.
        }
        let (vaspace, ring_va) = (facts.vaspace?, facts.ring_va?);
        let plane = self.plane.upgrade()?;
        let chan = kayfabe_rt::ceutils::CeUtilsChannel {
            client: facts.client,
            vaspace,
            ring_va,
            ring_entries: facts.ring_entries,
        };
        let key = (facts.proc.0, facts.chan.0);
        let cursor = *self
            .ce
            .cursors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            .unwrap_or(&kayfabe_rt::ceutils::GpCursor::default());

        let mut held = self.ce.vmm.lock().unwrap_or_else(|e| e.into_inner());
        let Some(vmm) = held.as_mut() else {
            return Some(refused(
                token,
                kayfabe_device::FaultTag("Shim::NoMemoryPlane"),
                "the memory plane is not attached, so a copy-engine submission has no guest \
                 memory to read or write; refused rather than served out of nothing"
                    .to_string(),
            ));
        };
        // ⊘ The walk's authorisation, as a value: the guest rang THIS channel's doorbell,
        // so the addresses of THIS submission are past their publication window
        // (`gmmu_publication_discipline.md` §6.1 / §7 rule 1).
        let demand = kayfabe_device::ceresolve::Demand::from_doorbell();
        let outcome = plane.ce_session(facts.client, vaspace, demand, |ce| {
            self.device.with_pushbuffer(|pb| {
                kayfabe_rt::ceutils::run_submission(ce, pb, vmm, chan, cursor)
            })
        });
        drop(held);

        let Some(outcome) = outcome else {
            // No publication for this `(hClient, hVASpace)` — a fact about the guest, and
            // the one refusal `ce_session` answers before a byte is read.
            return Some(refused(
                token,
                kayfabe_device::FaultTag("CeResolve::NoPublication"),
                format!(
                    "no page-directory root was published for (hClient 0x{:x}, hVASpace \
                     0x{vaspace:x}){}",
                    facts.client,
                    self.addressing_probe(token)
                ),
            ));
        };
        Some(match outcome {
            Ok(run) => {
                self.ce
                    .cursors
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(key, run.cursor);
                kayfabe_device::DoorbellReport::ServedLocally {
                    token,
                    proc: facts.proc.0,
                    chan: facts.chan.0,
                    // ★★★ §14.18 — carried ONLY on the success arm, and that placement is
                    // the promise: the plane latches this engine's non-stall vector off a
                    // `ServedLocally`, so an engine reaching it is an engine whose copy
                    // really ran. ⊘ The refusal arm below carries none, because a refused
                    // submission moved no bytes and owes no notification.
                    engine: facts.bound_engine,
                    note: run.describe(),
                }
            }
            Err(r) => refused(
                token,
                kayfabe_device::Faulted::fault_tag(&r.fault),
                format!("{}{}", r.describe(), self.addressing_probe(token)),
            ),
        })
    }

    /// ★★★ **What this channel's own addresses resolve to** — appended to a refusal so the
    /// boot states the finding instead of leaving it to be inferred.
    ///
    /// # ⊘ This is an OBSERVER. It serves nothing and it changes nothing the guest can see.
    ///
    /// The core has already refused; this runs afterwards and its entire output is text in
    /// a report. It does not populate `Channel::vas_pdb`, does not create a `Vas` and does
    /// not relax the refusal — `execution_plane_increments.md` §14.8 measured why that
    /// order is binding: granting the CeUtils channel a VAS *before* an executor is
    /// reachable turns a loud, correct `NoVas` into a doorbell reporting **Served** over
    /// work that did not happen.
    ///
    /// # ★★ Why walking here is permitted at all
    ///
    /// `gmmu_publication_discipline.md` §6.1/§7 rule 1: a walk is safe **iff** it is
    /// triggered by a real translation demand, so that it runs strictly after the guest's
    /// own publication window for those addresses. **A doorbell is that demand** — the
    /// guest wrote the ring, published the mappings the work touches, ran §3's flush, and
    /// only then wrote the token. ⊘ And it is the *only* commit point on this path: §5
    /// measured **both** invalidate transports at zero here, so nothing else could serve as
    /// the trigger. The permission is carried as a value
    /// ([`kayfabe_device::ceresolve::Demand::from_doorbell`]) precisely so a future
    /// prefetch cannot acquire it by editing a comment.
    ///
    /// # The three addresses, and why each one
    ///
    /// 1. **the ring** — `gpFifoOffset`, a GPU virtual address the channel itself declared;
    /// 2. **the first GPFIFO entry's target**, read out of the ring and decoded — the
    ///    pushbuffer the submission points at. This is the step that proves the chain
    ///    rather than one address of it;
    /// 3. **the finishPayload semaphore**, at [`FINISH_PAYLOAD_FROM_RING`] — the word the
    ///    guest is polling while it times out, so its aperture is the `#12` question.
    ///
    /// Returns the empty string when there is nothing to say (no plane, no channel facts,
    /// no declared VA space or ring) — an empty suffix leaves the refusal exactly as it was.
    fn addressing_probe(&self, token: u64) -> String {
        let Some(plane) = self.plane.upgrade() else {
            return String::new();
        };
        let Ok(facts) = self.device.ce_channel_facts(DOORBELL_TARGET_GPU, token) else {
            return String::new();
        };
        // ⊘ A channel that named no VA space has no address space to resolve in, and a
        // channel that declared no ring has no address to resolve. Neither is a walk we may
        // invent an argument for.
        //
        // ★★ REPORT THEM SEPARATELY. This used to collapse both misses into the one string
        // `vas=none ring=none`, which reads as *"the channel declared neither"* — and that
        // is a claim, not an observation. `[measured 2026-08-09, boots us1445/pu1448]` the
        // refused doorbell had a ring and no VA space: `AllocParams::Channel` sets
        // `gp_fifo_ring: Some(..)` unconditionally (`kayfabe-rmrpc/src/lib.rs:1269-1272`)
        // while `h_vaspace` goes through `declared_handle`, so the two are not even capable
        // of being absent together on that path. ⇒ an auditor reading the old string had to
        // open three source files to work out which half was missing. A diagnostic that
        // conflates two different facts is a diagnostic that sends its reader somewhere else.
        // ★★★★ §16.16 — THE OTHER PROJECTION OF THE VA SPACE, printed beside the one the
        // walk uses. `vaspace` is DERIVED (inherited through CtxShare/TSG by
        // `resolve_channel_vas`); `vaspace_declared` is what the channel's own alloc params
        // said. ⊘ Ring IDENTITY is closed from source — the VA we walk is the guest's
        // `gpFifoOffset` verbatim — but the TABLE we walk it in is not, and no refinement of
        // a descent can audit the choice of tree it descends. `dec=NONE` beside `vas=0x…`
        // is not an error; it is the statement that the tree is entirely our inference.
        let declared = facts
            .vaspace_declared
            .map_or_else(|| "NONE".to_string(), |v| format!("0x{v:x}"));
        // ★★★★ §16.16 — THE USERD CANARY, declared. ⊘ Three distinct strings for three
        // distinct facts, and collapsing any two would destroy the discrimination this
        // exists for: `UNREADABLE` = the driver boundary has no pinned layout for the
        // field, `h0` = the guest declared handle **zero** (a real declaration meaning "RM,
        // allocate USERD for me"), and a handle = an object the guest named. ⚠ `off=` is
        // printed unconditionally because a NON-ZERO offset that a consumer ignores makes
        // hardware see `GP_PUT == GP_GET` forever with no error anywhere — a silent stall
        // indistinguishable from the symptom under investigation.
        let userd = facts.userd.map_or_else(
            || " userd=UNREADABLE-AT-THIS-BOUNDARY".to_string(),
            |u| format!(" userd=h0x{:x}/off0x{:x}", u.handle, u.offset),
        );
        let (vaspace, ring_va) = match (facts.vaspace, facts.ring_va) {
            (Some(v), Some(r)) => (v, r),
            (None, Some(r)) => {
                return format!(
                    " | c=0x{:x} vas=NONE-DECLARED dec={declared}{userd} ring=0x{r:x}",
                    facts.client
                );
            }
            (Some(v), None) => {
                return format!(
                    " | c=0x{:x} vas=0x{v:x} dec={declared}{userd} ring=NONE-DECLARED",
                    facts.client
                );
            }
            (None, None) => {
                return format!(
                    " | c=0x{:x} vas=NONE-DECLARED dec={declared}{userd} ring=NONE-DECLARED",
                    facts.client
                );
            }
        };
        let demand = kayfabe_device::ceresolve::Demand::from_doorbell;
        let root = plane.published_root(facts.client, vaspace);
        // ★★★★ §16.6 — THE WHOLE ROW THE LOOKUP CHOSE, read out of the SAME table
        // `published_root` reads. Six boots named this pair in a refusal and printed no
        // body for it; see [`publication_row`] for what each field decides.
        let pubs = plane.gvas_publications();
        let row = publication_row(&pubs, facts.client, vaspace);
        // ★★★★ §16.8's rung: what does OUR framebuffer actually hold at the addresses this
        // row published, and at a WORKING row's? See [`fb_level_dump`].
        let fbdump = fb_dump_pair(&plane, &pubs, facts.client, vaspace);
        // ★★★★ §16.10's rung: which SLOT the descent consumes at every level, and what that
        // slot says. §16.9 dumped entry 0 of each level, and entry 0 is not the entry this
        // walk looks at. See `kayfabe_device::ceresolve::walk_trace` — the same decoder,
        // deliberately not a second one.
        //
        // ⊘ Printed BESIDE `rng=`, which is `resolve`'s own answer for the same address, so
        // the two projections are compared by a reader rather than trusted apart. A trace
        // whose terminal leaf disagrees with `rng=` is itself the finding.
        let walk = plane.published_walk_trace(facts.client, vaspace, ring_va);
        let ring = plane.resolve_published_va(facts.client, vaspace, ring_va, demand());
        // ★★★★ §16.12 — THE RING'S OWN PAGE. §16.10 proved the walk lands on `V:0x20000`
        // correctly; the open question is whether OUR framebuffer has ever had a byte
        // written there. ⊘ Addressed by the resolution's OWN answer, never by a literal:
        // the leaf is a per-boot address and hard-coding one would read correctly on
        // exactly one boot (§16.9's control-row argument, one level in).
        let ringpage = ring
            .vidmem_phys()
            .map_or_else(String::new, |phys| fb_level_dump(&plane, "fbRING", phys));
        let fin = plane.resolve_published_va(
            facts.client,
            vaspace,
            ring_va.wrapping_add(FINISH_PAYLOAD_FROM_RING),
            demand(),
        );
        // The pushbuffer the ring's first entry points at — read the entry, decode it, walk
        // its target. ⊘ Every step can fail and every failure is reported as itself: a ring
        // that would not read and a ring that read as a malformed entry are different facts.
        let mut gp = [0u8; PROBE_RING_BYTES];
        let pb = match plane.read_published_va(facts.client, vaspace, ring_va, &mut gp, demand()) {
            Err(e) => format!("ringread={}", e.describe()),
            Ok(_) => match kayfabe_abi::submit::gp_entry_decode(u64::from_le_bytes(gp)) {
                None => format!("gp0=0x{:016x} NOT-A-GP-ENTRY", u64::from_le_bytes(gp)),
                Some(d) => format!(
                    "gp0=0x{:x}+{:#x} pb={}",
                    d.gpu_va,
                    d.len_bytes,
                    plane
                        .resolve_published_va(facts.client, vaspace, d.gpu_va, demand())
                        .tag()
                ),
            },
        };
        format!(
            " | c=0x{:x} vas=0x{vaspace:x} dec={declared}{userd} root={} ring=0x{ring_va:x} rng={} fin={} {pb}{}{row}{fbdump}{ringpage} walk:{walk}",
            facts.client,
            root.map_or_else(
                || "none".to_string(),
                // ★★★ `virtAddrLo..Hi` PRINTED, and they were carried and dropped. `VasRoot`
                // has held them since it existed, documented *"carried for the report
                // only"*, and no report ever showed one. `[measured 2026-08-09, boot
                // `bar1_6ba1bd5`]` that became the deciding fact: the refusing channel's
                // root is `0x4000` while every root the census DOES print sits around
                // `0x2efa_xxxx`, and whether the published levels even COVER the ring's
                // address is not answerable without this pair. ⊘ A field carried for a
                // report that never prints it is a field nobody can use.
                |r| format!(
                    "0x{:x}/ap{}/sh{}/va[0x{:x}..0x{:x}]",
                    r.phys, r.aperture_raw, r.page_shift, r.virt_addr_lo, r.virt_addr_hi
                )
            ),
            ring.tag(),
            fin.tag(),
            self.ring_scan(facts.client, vaspace, ring_va, facts.ring_entries),
        )
    }

    /// ★★★ **Which GPFIFO entries of this ring are NON-ZERO** — the observation that
    /// separates *"the guest wrote its entry somewhere we did not look"* from *"we are
    /// reading the wrong store"*.
    ///
    /// # ⊘ Why one entry was not enough, and it is a MEASURED ambiguity
    ///
    /// `[measured 2026-08-09, boot `uvm2_d0fbac0`]` the UVM channel `cuInit` walls on
    /// resolved end to end and then refused:
    ///
    /// ```text
    /// [FwdFault::PushTooFragmented] … | c=0xc1d0000a vas=0xcaf00005 root=0x4000/ap1/sh47
    ///   ring=0x121010000 rng=V:0x20000 fin=V:0x28004 gp0=0x0000000000000000 NOT-A-GP-ENTRY
    /// ```
    ///
    /// The walk works; entry **0** is zero. Two completely different causes produce that
    /// byte-for-byte, and the fix differs:
    ///
    /// 1. the guest's `GP_PUT` is not `0` — UVM submitted at some other index (a control
    ///    GPFIFO entry, or a ring whose cursor did not start at zero), and the entry is
    ///    *there*;
    /// 2. we are reading a store the guest never wrote — the ring's leaf resolved to this
    ///    device's emulated framebuffer (`V:`) while the CeUtils ring resolves to guest RAM
    ///    (`S:`), and an aperture confusion reads a page of zeros that decodes as *"no
    ///    work"* rather than faulting.
    ///
    /// A scan answers it: **any** non-zero entry means (1) and names the index; **all** zero
    /// across the declared ring means (2) is live, and that is the whole point — an absence
    /// over one sample and an absence over the whole ring are different findings.
    ///
    /// ⊘ **An OBSERVER**, like the walk above it: it reads, it formats, and it changes
    /// nothing. It runs only on a refusal, so it costs a boot that is already failing.
    /// ⚠ Bounded at [`RING_SCAN_ENTRIES`] regardless of what the channel declared — the
    /// entry count is a guest-supplied number and a diagnostic must not become a
    /// guest-sized read.
    fn ring_scan(&self, client: u32, vaspace: u32, ring_va: u64, entries: u32) -> String {
        let Some(plane) = self.plane.upgrade() else {
            return String::new();
        };
        let n = (entries as usize).clamp(1, RING_SCAN_ENTRIES);
        let demand = kayfabe_device::ceresolve::Demand::from_doorbell();
        let mut nonzero: Vec<String> = Vec::new();
        let mut unread = 0usize;
        for i in 0..n {
            let at = ring_va.wrapping_add((i * PROBE_RING_BYTES) as u64);
            let mut gp = [0u8; PROBE_RING_BYTES];
            match plane.read_published_va(client, vaspace, at, &mut gp, demand) {
                Err(_) => unread += 1,
                Ok(_) => {
                    let raw = u64::from_le_bytes(gp);
                    if raw != 0 && nonzero.len() < RING_SCAN_REPORT {
                        nonzero.push(format!("[{i}]=0x{raw:016x}"));
                    }
                }
            }
        }
        // ⊘ The scanned count is stated, so "all zero" can never be read as "we looked at
        // the whole ring" when the declared entry count exceeded the bound.
        format!(
            " scan={}/{} declared, unread={}, nonzero={}",
            n,
            entries,
            unread,
            if nonzero.is_empty() {
                "NONE — every scanned entry is ZERO".to_string()
            } else {
                nonzero.join(" ")
            }
        )
    }
}

/// ★★★★ **THE FORWARD SEARCH FOR THE RING** — §16.16, and it is the one measurement in
/// this file that never consults the walker.
///
/// # ⊘ Why a forward search, when the walk is already "correct end to end"
///
/// Every instrument this campaign has built so far asks the *same* question in the *same*
/// direction: **take the guest's declared ring VA, descend the guest's page tables, and
/// look at where we land.** `[measured 2026-08-09, boot `res1_fc21926`]` that lands on
/// framebuffer offset `0x20000`, and `0x20000` is `resN-NEVER-WRITTEN`.
///
/// ★ Every one of those instruments shares a premise — *that the table we descended is the
/// table the guest wrote the ring through*. `ce_channel_facts` derives the VA space from
/// `Channel::vas_origin`, not from anything the channel declared, and its own comment
/// records that **this exact attribution has already been wrong once on this exact
/// channel**. ⊘ A second projection of a computation cannot audit the first. So no refinement
/// of the descent can decide whether the descent is aimed at the right table.
///
/// This asks the **converse**, and it consults nothing the descent produced: *"is there a
/// page ANYWHERE in our framebuffer whose bytes look like a GPFIFO ring?"* The two answers
/// are independent, and together they discriminate:
///
/// | ring-like page found | at `0x20000` | reading |
/// |---|---|---|
/// | no | — | the ring's bytes are **not in our framebuffer at all** — they went to sysmem, to BAR1's discard, or nowhere. The write path is the defect. |
/// | yes | yes | impossible while `0x20000` is not resident; would refute the residency census itself. |
/// | yes | **no** | ★ the guest wrote its ring, we **caught** it, and we are **descending the wrong table** to find it. The address plane is the defect, not the write path. |
///
/// # What counts as "ring-like", and why the bar is where it is
///
/// A GPFIFO ring is an array of 8-byte entries. [`kayfabe_abi::submit::gp_entry_decode`]
/// alone is far too weak a sieve — it rejects only a zero length field, so roughly any
/// non-trivial qword "decodes". ⊘ A sieve that accepts noise would report every page of
/// page-table entries as a ring. So an entry counts only when it also carries a **non-zero
/// target** and a length that is **plausible for a pushbuffer** ([`GP_LEN_MAX`]); and a
/// *page* counts only at [`RINGLIKE_MIN`] such entries, because one qword that happens to
/// decode is a coincidence and a run of them is a structure.
///
/// ⊘ **It concludes nothing and it changes nothing.** It reads resident pages, counts, and
/// returns numbers for the report. Nothing is emitted the guest did not ask for, no address
/// is inferred, and a score is not a claim that a page IS a ring — it is a claim about how
/// many of its qwords have the shape.
#[derive(Debug, Clone, Copy, Default)]
struct FbRingSweep {
    /// How many resident frames were examined. ⊘ Bounded by [`SWEEP_FRAMES_MAX`]; the
    /// bound is reported beside the total so "none found" can never be read as "we looked
    /// at all of them" when it was truncated.
    /// ⊘ The total to compare it against is [`KayfabeRegAudit::fb_resident_pages`], which
    /// the report already carries — deliberately NOT re-counted here, so a truncation shows
    /// up as two fields from two different reads disagreeing rather than as one field
    /// silently agreeing with itself.
    swept: u64,
    /// How many swept frames scored at least [`RINGLIKE_MIN`].
    ringlike: u64,
    /// The best-scoring frame's framebuffer address. Meaningless unless
    /// [`Self::ringlike`] is non-zero.
    best: u64,
    /// That frame's score — how many of its 512 qwords had the shape.
    best_score: u64,
    /// [`kayfabe_device::FbWriter::index`] of that frame's FIRST writer, plus one, so that
    /// **zero means "no origin was recorded"** rather than naming `PRAMIN`. ⊘ The
    /// zero-direction is the decision here: an audit struct the archive never wrote is all
    /// zeros, and zero must be the honest non-claim.
    best_writer_plus1: u64,
}

/// The largest pushbuffer length, in bytes, a GPFIFO entry may claim and still count
/// toward a page's ring-likeness. `GP_ENTRY1_LENGTH` is 21 bits of **dwords**, so the
/// field can express 8 MiB; a real UVM push is a few hundred bytes. ⊘ A generous bound
/// (1 MiB) rather than a tight one: this sieve exists to exclude noise, and a tight bound
/// would start excluding real entries and turn a found ring into a miss.
const GP_LEN_MAX: u64 = 1 << 20;

/// How many shaped qwords a page needs before it is called ring-like. One is a
/// coincidence; a run is a structure.
const RINGLIKE_MIN: u64 = 4;

/// ⊘ A bound on the sweep, for [`SharedDoorbell::ring_scan`]'s reason: the resident set is
/// guest-sized, and a diagnostic must not become a guest-sized read. `[measured
/// 2026-08-09, boot `res1_fc21926`]` the real set was **90** frames, so this is ~90x
/// headroom and the truncation arm should never fire — but it is reported if it does.
const SWEEP_FRAMES_MAX: usize = 8192;

/// Run [`FbRingSweep`] over the plane's framebuffer. [`None`] when there is no store to
/// ask — ⊘ NOT `Some(default)`, which would assert an empty framebuffer.
fn fb_ring_sweep(plane: &kayfabe_device::plane::RegPlane) -> Option<FbRingSweep> {
    let frames = plane.fb_resident_frames()?;
    let mut out = FbRingSweep::default();
    let mut page = vec![0u8; kayfabe_device::fbwin::FB_PAGE as usize];
    for phys in frames.into_iter().take(SWEEP_FRAMES_MAX) {
        out.swept += 1;
        // ⊘ A frame the store will not hand back is skipped and NOT scored zero: "refused"
        // and "contains nothing ring-shaped" are different facts, and only the second is a
        // measurement about the guest.
        if plane.fb_peek(phys, &mut page).is_err() {
            continue;
        }
        let score = page
            .chunks_exact(8)
            .filter(|w| {
                let raw = u64::from_le_bytes(w[..8].try_into().unwrap_or([0; 8]));
                kayfabe_abi::submit::gp_entry_decode(raw)
                    .is_some_and(|d| d.gpu_va != 0 && d.len_bytes <= GP_LEN_MAX)
            })
            .count() as u64;
        if score >= RINGLIKE_MIN {
            out.ringlike += 1;
            if score > out.best_score {
                out.best = phys;
                out.best_score = score;
                // ★ The origin is read for the frame the sweep CHOSE, from the store's own
                // map — never re-derived from the address.
                out.best_writer_plus1 = plane
                    .fb_page_origin(phys)
                    .map_or(0, |o| o.by.index() as u64 + 1);
            }
        }
    }
    Some(out)
}

/// How many GPFIFO entries [`SharedDoorbell::ring_scan`] reads. See its docs for why it is
/// a bound and not the channel's declared count.
const RING_SCAN_ENTRIES: usize = 64;

/// How many non-zero entries the scan NAMES. The rest are still counted by the scan's own
/// range, which is printed beside it.
const RING_SCAN_REPORT: usize = 4;

/// The realized register plane — what the C shim holds behind its second opaque handle.
///
/// ⊘ Hand-written [`core::fmt::Debug`] since E2, because `SharedDevice` deliberately has
/// none — see [`SharedObjectModel`].
pub struct Regs {
    plane: Arc<RegPlane>,
    /// ★★★ **E2** — the L1 shell that owns the object model, held here because **two**
    /// paths now reach it: the object bridge (boxed into the register plane's served
    /// chain, and unreachable afterwards) and the doorbell port. Before E2 there was one
    /// path and it could own the `Gpu` outright.
    ///
    /// ⊘ Held for the doorbell port's sake, and it is what keeps this device's object
    /// model alive for exactly as long as the device: a shell that let it go would leave
    /// the plane's port holding the last handle to a graph nobody can reach.
    ///
    /// ★ **E6.** It used to carry `#[allow(dead_code)]` because the *field* was never read.
    /// [`Regs::object_model`] reads it now, which is what makes debt Q24 assertable by
    /// running rather than by counting `Gpu::new` in this file's own source.
    device: Arc<kayfabe_rt::device::SharedDevice>,
    /// ★★★ The object bridge's refusal census, kept **here** because the policy that owns
    /// it is boxed into the chain and is unreachable afterwards. See
    /// [`kayfabe_rmrpc::SharedRefusalCensus`] for the boot that had to be diagnosed by the
    /// absence of a line instead.
    refusals: kayfabe_rmrpc::SharedRefusalCensus,
    /// ★★★ §8.2.2 — the GPFIFO-ring census, kept here for [`Regs::refusals`]'s reason.
    /// Recorder-only: nothing in this device reads it, and the only thing it changes is
    /// that a boot can *state* the address the guest named for a ring.
    rings: kayfabe_rmrpc::SharedRingCensus,
    /// ★★★ E1 — the isolate plane's census, kept here for the reason
    /// [`Regs::refusals`] is: the policy that owns the object model is boxed into the
    /// chain and unreachable afterwards.
    isolates: kayfabe_core::gpu::SharedIsolateCensus,
    /// ★★★ **§14.23** — what the publication seat saw and what the object model accepted,
    /// kept here for [`Regs::refusals`]' reason: the observer is boxed into the chain's
    /// front seat and is unreachable afterwards.
    ///
    /// ⊘ It is the **non-vacuity** half of every claim about the page-directory plane: a
    /// boot reporting no publication refusals and `seen = 0` is a seat that was never
    /// filled, and without this number that boot is indistinguishable from a healthy one.
    publications: kayfabe_rmrpc::SharedPublicationCensus,
    /// ★★★ **E10e** — the CPU copy-engine executor's shell state, shared with the doorbell
    /// port. See [`CeShellState`]; this handle exists so [`Regs::attach_ram`] can install
    /// the memory plane into a port that was built before one existed.
    ce: Arc<CeShellState>,
}

impl core::fmt::Debug for Regs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Regs")
            .field("plane", &self.plane)
            .finish_non_exhaustive()
    }
}

impl Regs {
    /// Build the register plane for a chip. `0` selects the table's default row. The
    /// notifier probe is empty — the shipping configuration, and the reason this is the
    /// constructor every test uses.
    ///
    /// # Errors
    /// As [`Regs::create_probed`].
    pub fn create(device_id: u16) -> Result<Regs, (Status, &'static str)> {
        Regs::create_probed(device_id, "")
    }

    /// Build the register plane with a notifier **probe set** — the
    /// `probe-arm-notifier` device property's comma-separated decimal string.
    ///
    /// ⊘ Parsed **strictly**: junk refuses the device at realize, by name, rather than
    /// booting probe-off — the predecessor env var did exactly that silently, three
    /// boots in a row, and the conclusions drawn from them had to be retracted. The set
    /// in effect is recorded in the plane's census, so the end-of-run report proves
    /// what the boot ran with.
    ///
    /// # Errors
    /// [`classify_chip`]-ed, [`Status::Unsupported`] if the guest driver version this
    /// build answers as has no wire table, or [`Status::Malformed`] for a probe string
    /// that is not a comma-separated decimal list within
    /// [`kayfabe_abi::eventnotify::PROBE_ARM_MAX`] entries.
    pub fn create_probed(device_id: u16, probe_arm: &str) -> Result<Regs, (Status, &'static str)> {
        let probe_arm =
            kayfabe_abi::eventnotify::ProbeArmSet::parse(probe_arm).map_err(|e| match e {
                kayfabe_abi::eventnotify::ProbeArmParseError::NotDecimal => (
                    Status::Malformed,
                    "probe-arm-notifier must be a comma-separated list of decimal \
                     notifier indices; a token failed to parse, and a probe that \
                     silently shrank would be a boot running different instrumentation \
                     than its operator believes — refused instead",
                ),
                kayfabe_abi::eventnotify::ProbeArmParseError::TooMany => (
                    Status::Malformed,
                    "probe-arm-notifier names more indices than the probe set carries; \
                     truncating silently would be a boot running different \
                     instrumentation than its operator believes — refused instead",
                ),
            })?;
        let chip = chip_for(device_id)?;
        let abi = kayfabe_device::abi::gsp_abi_for(GUEST_DRIVER).map_err(|_| {
            (
                Status::Unsupported,
                "this build has no wire table for the guest driver version its register \
                 plane answers as; the table is keyed on the full major.minor.patch and \
                 refuses below its floor rather than nearest-neighbouring",
            )
        })?;
        let (links, refusals, rings, isolates, publications, isolate_plane, device) =
            object_policy(abi.driver, chip.engines)?;
        let plane = RegPlane::with_objects(
            chip,
            abi,
            Box::new(HostMonotonicClock::new()),
            probe_arm,
            links,
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
        // ★★ The plane is `Arc`-ed BEFORE its doorbell port is installed, because the port
        // holds a `Weak` back to it (see [`SharedDoorbell::plane`]). `set_doorbell` takes
        // `&self`, so the order costs nothing and the cycle is broken by construction.
        let plane = Arc::new(plane);
        let ce = Arc::new(CeShellState::default());
        plane.set_doorbell(Box::new(SharedDoorbell {
            device: Arc::clone(&device),
            plane: Arc::downgrade(&plane),
            ce: Arc::clone(&ce),
            // ★★★ §14.24 — from the composition root's OWN selector reading, not from a
            // second one. `Stillborn` means, in that plane's own words, *"no host verb can
            // be issued"*, so nothing but this shell's CPU executor can run a copy.
            local_ce_is_the_only_executor: isolate_plane == IsolatePlane::Stillborn,
        }));
        Ok(Regs {
            plane,
            device,
            refusals,
            rings,
            isolates,
            publications,
            ce,
        })
    }

    /// The plane, for a caller that needs more than this seam exposes.
    #[must_use]
    pub fn plane(&self) -> &RegPlane {
        &self.plane
    }

    /// ★★★ **E6 (debt Q24) — THE object model this root realized**, handed out so the one
    /// property E2 could only assert over *source text* can be asserted by **running**.
    ///
    /// # What it is for, stated exactly
    ///
    /// E2's `⊘ What E2 does NOT establish` item 4 records the gap: the object bridge and
    /// the doorbell port are `Arc::clone`s of one [`kayfabe_rt::device::SharedDevice`]
    /// **by construction**, and *"the behavioural witness — declare a channel through the
    /// bridge, ring its vChid through the doorbell — is an E6 assertion, because nothing
    /// in this port can inject an `RmEvent` chain."* A second `Gpu` behind the doorbell
    /// leaves [`kayfabe_fwd::FwdFault::UnknownVchid`] as the permanent answer **with every
    /// test still green**, which is why a source-quantified check was never enough.
    ///
    /// This is that injection point: the handle returned is the *same* `Arc` the boxed
    /// object policy declares into and the *same* one [`SharedDoorbell`] rings, so a
    /// channel declared through it and then rung through [`Regs::write`] crosses the join
    /// under test rather than a reconstruction of it.
    ///
    /// ⊘ **Nothing in the archive calls this**, and it grants no authority the guest does
    /// not already have: every mutation reachable through the returned handle is one the
    /// object bridge performs on the guest's behalf anyway. It is an *observability* seam,
    /// in the same sense [`Regs::audit`] is.
    #[must_use]
    pub fn object_model(&self) -> Arc<kayfabe_rt::device::SharedDevice> {
        Arc::clone(&self.device)
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
        // ★★★ **E10e** — the same handle, for the CPU copy-engine executor. ⊘ The SAME
        // one, cloned rather than re-derived: `QemuVmm` is a handle onto the machine's
        // memory plane, so two of them are one plane and cannot disagree — which is the
        // property that lets a copy's bytes and its finishPayload travel by one
        // description of guest memory. Installed here for `MachineRam`'s own reason: the
        // memory plane does not exist at device realize.
        *self.ce.vmm.lock().unwrap_or_else(|e| e.into_inner()) = Some(shim.machine().vmm());
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
        // The teardown half, and not optional for the same reason: a copy-engine
        // submission arriving after the machine released its slots must be refused by
        // name, not served against a handle onto memory that is gone.
        *self.ce.vmm.lock().unwrap_or_else(|e| e.into_inner()) = None;
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
            nonstall_raises,
            nonstall_unvectored,
            nonstall_masked,
            commands,
            commands_unserviced,
            doorbells,
            doorbells_served,
            doorbells_refused,
        } = self.plane.counters();
        let (bar_pde_updates, bar_pde_refusals) = self.plane.bar_pde_counts();
        // ★ Truncated to what the wire shape holds, and `unserviced_len` says how many —
        // ⊘⊘ which it did NOT before 2026-08-09: it was `sample.len()`, clamped by the
        // sample's own cap, so it could not report a truncation and a saturated list read
        // as a complete one. It is now the plane's true distinct count.
        let sample = self.plane.unserviced_sample();
        let unserviced_distinct = self.plane.unserviced_distinct();
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
            // find out why their forwarding plane is down — and truncation is now STATED
            // rather than silent, which is the whole of `copy_sentence`'s docs.
            isolate_refusal.len = copy_sentence(&mut isolate_refusal.text, &why);
        }
        // ★★★ **E2** — what the doorbell aperture saw, DESTRUCTURED with no `..` for
        // `Shim::audit`'s reason: a field added to `DoorbellLog` and not wired here is a
        // fact the C shell can never read, and nothing goes red. `rustc` refuses instead.
        let kayfabe_device::DoorbellLog {
            last_token,
            first_refusal,
            last_local_serving,
        } = self.plane.doorbell_log();
        let mut doorbell_local_serving = KayfabeDoorbellServing::default();
        if let Some(note) = last_local_serving {
            doorbell_local_serving.present = 1;
            // ⊘ Truncated on a CHARACTER boundary and SAYING SO, for the reason every
            // sentence here is — see [`copy_sentence`].
            doorbell_local_serving.len = copy_sentence(&mut doorbell_local_serving.text, &note);
        }
        let mut doorbell_refusal = KayfabeDoorbellRefusal::default();
        if let Some(r) = first_refusal {
            doorbell_refusal.present = 1;
            let kb = r.kind.0.as_bytes();
            let ktake = kb.len().min(DOORBELL_KIND_LEN);
            doorbell_refusal.kind[..ktake].copy_from_slice(&kb[..ktake]);
            doorbell_refusal.kind_len = ktake as u64;
            // ⊘ Truncated on a CHARACTER boundary and SAYING SO, for the reason the isolate
            // sentence above is. ★ This is the buffer §16.6 loaded up with a whole
            // publication body, and the one whose silent `min()` would have eaten the
            // deciding levels first — they are at the END of the sentence.
            doorbell_refusal.len = copy_sentence(&mut doorbell_refusal.text, &r.why);
        }
        // ★★★ §8.2.2 — the GPFIFO-ring census. Destructured with no `..` for the reason
        // the isolate census below is: a field added to `RingCensus` and not wired here
        // is a number the C shell can never read, and nothing goes red. `rustc` refuses
        // the pattern instead.
        let kayfabe_rmrpc::RingCensus {
            declarations: gpfifo_ring_declarations,
            nonzero: gpfifo_ring_nonzero,
            first_nonzero: gpfifo_ring_first,
        } = self.rings.snapshot();
        // ★★★ §14.41 — the replayable-fault-buffer registrations. The count is the report's
        // TRIGGER: the C printer emits `DELIVERY_UNBUILT` beside it whenever it is non-zero,
        // so serving `0x20800a9b` and stating what serving it did NOT buy are one act.
        //
        // ⊘ The FIRST sample, not the last, and `total()` rather than `sample().len()` — the
        // sample is capped at `FAULT_BUFFER_SAMPLE_MAX` and a count read off it could never
        // exceed the cap. That is the exact defect `unserviced_len` shipped with
        // (`a_saturated_instrument_looks_exactly_like_absence`); it is not repeated here.
        use kayfabe_device::faultbuffer::FaultBufferNote as Fbn;
        let fault_buffers_registered_n = self.plane.fault_buffers_registered();
        let shadow_fault_buffers_registered = self.plane.shadow_fault_buffers_registered();
        let fault_buffer_sample = self.plane.fault_buffer_sample();
        let (fault_buffer_size, fault_buffer_pages) = fault_buffer_sample
            .iter()
            .find_map(|n| match n {
                Fbn::Registered(r) => Some((u64::from(r.size), r.pages.len() as u64)),
                _ => None,
            })
            .unwrap_or((0, 0));
        let fault_buffers_malformed = fault_buffer_sample
            .iter()
            .filter(|n| matches!(n, Fbn::Malformed { .. }))
            .count() as u64;
        // ★★★ §14.41's second rung. Same shape, and the geometry is reported so the two
        // controls can be checked against each other: `shadow_fault_buffer_size` must be the
        // `nonReplayableFaultBufferSize` this port answers to `0x20800a59`, and the page count
        // must be its own `align_up(size)/4096 + align_up(metadataSize)/4096`.
        let (shadow_fault_buffer_size, shadow_fault_buffer_pages, shadow_fault_buffer_type) =
            fault_buffer_sample
                .iter()
                .find_map(|n| match n {
                    Fbn::ShadowRegistered(r) => Some((
                        u64::from(r.size),
                        r.pages.len() as u64,
                        u64::from(r.buffer_type),
                    )),
                    _ => None,
                })
                .unwrap_or((0, 0, 0));
        let shadow_fault_buffers_malformed = fault_buffer_sample
            .iter()
            .filter(|n| matches!(n, Fbn::ShadowMalformed { .. }))
            .count() as u64;
        let access_cntr_buffers_registered = self.plane.access_cntr_buffers_registered();
        let (access_cntr_buffer_size, access_cntr_buffer_pages) = fault_buffer_sample
            .iter()
            .find_map(|n| match n {
                Fbn::AccessCntrRegistered(r) => Some((u64::from(r.size), r.pages.len() as u64)),
                _ => None,
            })
            .unwrap_or((0, 0));
        let access_cntr_buffers_malformed = fault_buffer_sample
            .iter()
            .filter(|n| matches!(n, Fbn::AccessCntrMalformed { .. }))
            .count() as u64;
        // ★★★ The control census — DESTRUCTURED with no `..` for [`Shim::audit`]'s reason:
        // a field added to `CensusSnapshot` and not wired here is a fact the C shell can
        // never read, and nothing goes red. `rustc` refuses the pattern instead.
        // ★★★★ §16.13 — read BEFORE the struct is assembled, so it is one lock acquisition
        // rather than four inside a literal. ⊘ `None` is carried as `None`, not flattened.
        let fb_residency = self.plane.fb_residency();
        // ★ Taken once, here, beside the residency it is reported with — ⊘ not inside the
        // struct literal, where a second call would sweep a store that had moved on between
        // the two reads and produce a census and a sweep describing different moments.
        let fb_sweep = fb_ring_sweep(&self.plane);
        let kayfabe_device::census::CensusSnapshot {
            probe_arm: probe_arm_set,
            served_total,
            served_distinct: served_len,
            served: served_rows,
            arming_total,
            arming_distinct: arming_len,
            armings: arming_rows,
            bind_total,
            bind_distinct: bind_len,
            binds: bind_rows,
        } = self.plane.control_census();
        // ★★★ The VA-space publications — DESTRUCTURED with no `..` for [`Shim::audit`]'s
        // reason: a field added to `GvasPubSnapshot` and not wired here is a fact the C
        // shell can never read, and nothing goes red. `rustc` refuses the pattern instead.
        let kayfabe_device::gvaspub::GvasPubSnapshot {
            total: gvas_pub_total,
            distinct: gvas_pub_len,
            undecodable: gvas_pub_undecodable,
            sample: gvas_pub_rows,
            // ⊘ The TABLE itself does not cross — it is up to 256 rows of 184-byte bodies
            // and the report is the sample. What crosses is whether it is still COMPLETE,
            // which is the only property a reader of a `NoPublication` refusal needs.
            roots: _gvas_roots,
            roots_refused: gvas_pub_roots_refused,
        } = self.plane.gvas_publications();
        // ★★★ §14.23 — and the SEAT's own numbers, destructured with no `..` for the same
        // reason: a counter added to `PublicationCensus` and not wired here is a fact the C
        // shell can never read.
        let kayfabe_rmrpc::PublicationCensus {
            seen: gvas_pub_seen,
            applied: gvas_pub_applied,
            unexpected: gvas_pub_unexpected,
        } = self.publications.snapshot();
        let mut gvas_pub = [KayfabeGvasPublication::default(); GVAS_PUBLICATION_SLOTS];
        for (slot, r) in gvas_pub.iter_mut().zip(gvas_pub_rows.iter()) {
            let mut levels = [KayfabePdeLevel::default(); GVAS_MAX_LEVELS];
            for (lv, src) in levels.iter_mut().zip(r.pdes.levels.iter()) {
                *lv = KayfabePdeLevel {
                    phys_address: src.phys_address,
                    size: src.size,
                    aperture: src.aperture,
                    page_shift: u32::from(src.page_shift),
                };
            }
            *slot = KayfabeGvasPublication {
                cmd: r.cmd,
                client: r.client,
                object: r.object,
                num_levels: r.pdes.num_levels,
                page_size: r.pdes.page_size,
                virt_addr_lo: r.pdes.virt_addr_lo,
                virt_addr_hi: r.pdes.virt_addr_hi,
                h_subdevice: r.pdes.h_subdevice,
                subdevice_id: r.pdes.subdevice_id,
                count: r.count,
                levels,
            };
        }
        let mut probe_arm = [0u32; PROBE_ARM_SLOTS];
        probe_arm[..probe_arm_set.as_slice().len()].copy_from_slice(probe_arm_set.as_slice());
        let mut served = [KayfabeServedControl::default(); SERVED_CONTROL_SLOTS];
        for (slot, r) in served.iter_mut().zip(served_rows.iter()) {
            *slot = KayfabeServedControl {
                cmd: r.cmd,
                rpc_result: r.rpc_result,
                count: r.count,
            };
        }
        let mut armings = [KayfabeNotifierArming::default(); NOTIFIER_ARMING_SLOTS];
        for (slot, r) in armings.iter_mut().zip(arming_rows.iter()) {
            *slot = KayfabeNotifierArming {
                client: r.client,
                object: r.object,
                event: r.event,
                action: r.action,
                rpc_result: r.rpc_result,
                reserved: 0,
                count: r.count,
            };
        }
        let mut binds = [KayfabeChannelBind::default(); CHANNEL_BIND_SLOTS];
        for (slot, r) in binds.iter_mut().zip(bind_rows.iter()) {
            *slot = KayfabeChannelBind {
                client: r.client,
                object: r.object,
                engine_type: r.engine_type,
                ce_index: r.ce_index,
                rpc_result: r.rpc_result,
                reserved: 0,
                count: r.count,
            };
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
            // ★★★★ §16.13 — the residency CENSUS, with its own precondition. `None` from
            // the store means "there is no framebuffer to ask", which is a different fact
            // from "nothing is resident" and must not be encoded as zeros.
            fb_resident_valid: u64::from(fb_residency.is_some()),
            fb_resident_lo: fb_residency.and_then(|r| r.lo).unwrap_or(0),
            fb_resident_hi: fb_residency.and_then(|r| r.hi).unwrap_or(0),
            fb_resident_pages: fb_residency.map_or(0, |r| r.pages),
            // ★★★★ §16.16 — the first-writer census, taken from the SAME `FbResidency` the
            // extent above comes from, so the two can never describe different snapshots.
            fb_origin_by_writer: fb_residency.map_or([0; 5], |r| r.by_writer),
            // ★★★★ §16.16 — the forward search. ⊘ All zeros when there is no store to ask,
            // which `fb_resident_valid` already distinguishes from an empty framebuffer.
            fb_sweep_swept: fb_sweep.map_or(0, |s| s.swept),
            fb_sweep_ringlike: fb_sweep.map_or(0, |s| s.ringlike),
            fb_sweep_best: fb_sweep.map_or(0, |s| s.best),
            fb_sweep_best_score: fb_sweep.map_or(0, |s| s.best_score),
            fb_sweep_best_writer_plus1: fb_sweep.map_or(0, |s| s.best_writer_plus1),
            faults,
            ram_refusals,
            irq_requests,
            cpu_intr_accesses,
            cpu_intr_raises,
            cpu_intr_masked,
            nonstall_raises,
            nonstall_unvectored,
            nonstall_masked,
            commands,
            commands_unserviced,
            unserviced_len: unserviced_distinct,
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
            doorbell_local_serving,
            gpfifo_ring_declarations,
            gpfifo_ring_nonzero,
            gpfifo_ring_va: gpfifo_ring_first.map_or(0, |(va, _)| va),
            gpfifo_ring_entries: gpfifo_ring_first.map_or(0, |(_, n)| u64::from(n)),
            gvas_pub_total,
            gvas_pub_len,
            gvas_pub_undecodable,
            gvas_pub_roots_refused,
            gvas_pub_seen,
            gvas_pub_applied,
            gvas_pub_unexpected,
            gvas_pub,
            served_total,
            served_len,
            served,
            arming_total,
            arming_len,
            armings,
            bind_total,
            bind_len,
            binds,
            probe_arm_len: probe_arm_set.as_slice().len() as u64,
            probe_arm,
            fault_buffers_registered: fault_buffers_registered_n,
            fault_buffer_size,
            fault_buffer_pages,
            fault_buffers_malformed,
            shadow_fault_buffers_registered,
            shadow_fault_buffer_size,
            shadow_fault_buffer_pages,
            shadow_fault_buffer_type,
            shadow_fault_buffers_malformed,
            access_cntr_buffers_registered,
            access_cntr_buffer_size,
            access_cntr_buffer_pages,
            access_cntr_buffers_malformed,
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
    // ★★★ §14.23 — the TWO seats, as `kayfabe_device::ObjectLinks` builds them: the
    // publication observer (front, cannot answer) and the object policy. Both declare into
    // the one shell below.
    kayfabe_device::ObjectLinks,
    kayfabe_rmrpc::SharedRefusalCensus,
    // ★ §8.2.2 — the GPFIFO-ring census, recorder-only.
    kayfabe_rmrpc::SharedRingCensus,
    kayfabe_core::gpu::SharedIsolateCensus,
    // ★★★ §14.23 — what the publication seat saw and what the model accepted.
    kayfabe_rmrpc::SharedPublicationCensus,
    // ★★★ §14.24 — WHICH isolate plane this build installed, carried out so the doorbell
    // port's executor question is answered from the SAME reading of the selector that built
    // the isolate factory. ⊘ Not re-read at the doorbell site: two readings of one env var
    // is two facts that can disagree, which is the shape this file already refuses for the
    // probe set and for the chip's engine slice.
    IsolatePlane,
    // ★★★ E2 — and the shell itself, because the doorbell port needs the SAME one.
    Arc<kayfabe_rt::device::SharedDevice>,
);

fn object_policy(
    driver: kayfabe_abi::versions::DriverAbiTable,
    // ★★★ E9/§13.6 option (2) — the SAME `ChipProfile::engines` slice the device-info
    // path serves the guest, so the bind check and the advertisement cannot be two
    // descriptions of one silicon.
    engines: &'static [kayfabe_abi::inittables::FifoDeviceEntry],
) -> Result<ObjectLink, (Status, &'static str)> {
    let isolate_plane = selected_isolate_plane()?;
    let isolates = isolate_factory(isolate_plane)?;
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
        engines,
        kayfabe_rmrpc::ReasmLimits::default(),
    );
    // ★ The handle is taken BEFORE the policy is boxed, because afterwards there is no
    // `ObjectPolicy` left to ask — that is the whole reason the census had to become a
    // shared store rather than a field behind `&self`.
    let refusals = policy.refusal_census();
    // ★ §8.2.2 — same taken-before-boxing reason, one increment on.
    let rings = policy.ring_census();
    // ★★★ E1 — and the isolate plane's own health, for the same reason and by the same
    // mechanism. Before this the only channel that could say "the forwarding plane you
    // asked for did not come up" was a host-side `ps`.
    let isolates = policy.isolate_census();
    // ★★★ **§14.23 — the publication seat, over a SECOND handle onto the SAME shell.**
    //
    // That is what `kayfabe_rmrpc::ObjectModel` was made a port for (E2): the doorbell path
    // already holds its own handle onto this exact `SharedDevice`, and a page-directory base
    // landing in a different graph from the one promotions resolve against would be a
    // routing table that can never resolve. ⊘ Not a second `Gpu`; the same one.
    //
    // ★ It shares `refusals`, so a publication this seat refuses appears in the one census
    // the boot report prints rather than in a second tally nothing reads.
    let publications = kayfabe_rmrpc::PublicationObserver::over(
        &driver,
        kayfabe_abi::GuestOs::Linux,
        Box::new(SharedObjectModel(Arc::clone(&device))),
        refusals.clone(),
    );
    // ★ Taken BEFORE the observer is boxed, for the reason the refusal census is:
    // afterwards there is no `PublicationObserver` left to ask.
    let publication_census = publications.census();
    let links = kayfabe_device::ObjectLinks {
        publications: Some(Box::new(publications)),
        objects: Some(Box::new(policy)),
    };
    Ok((
        links,
        refusals,
        rings,
        isolates,
        publication_census,
        isolate_plane,
        device,
    ))
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
