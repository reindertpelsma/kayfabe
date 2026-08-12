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
use kayfabe_vmm_qemu::host::{
    BarPlacement, MrHandle, QemuHost, SectionBacking, SectionDesc, SectionFacts,
};
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
///
/// ★ Bumped to **32** at §16.18, the ABI-3 reason a twentieth time: [`KayfabeRegAudit`]
/// gained the framebuffer aperture's five words (`bar1_reads`, `bar1_writes`,
/// `bar1_faults`, `bar1_pde_base`, `bar1_root_published`), so an ABI-31 shim would allocate
/// the old layout and this archive would write **40 bytes** past the end of it.
///
/// ★★★★ Bumped to **33** at §16.30, the ABI-3 reason a twenty-first time:
/// [`KayfabeRegAudit`] gained the `0x00801813 SET_PAGE_DIRECTORY` install record
/// (`set_page_dir_*`, nine words), so an ABI-32 shim would allocate the old layout and
/// this archive would write **72 bytes** past the end of it.
///
/// ★★★★ Bumped to **34** at §16.40, the ABI-3 reason a twenty-second time:
/// [`KayfabeRegAudit`] gained the promote-ctx diagnosis (`promote_diag` —
/// [`PROMOTE_DIAG_LEN`] bytes — plus `promote_diag_len`), so an ABI-33 shim would allocate
/// the old layout and this archive would write **2 056 bytes** past the end of it.
///
/// ⊘ And this one is an instrument being **un-gated**, not a new measurement: the
/// VA-space census it carries has existed since §15 and was reachable only from inside a
/// doorbell refusal. `[measured 2026-08-09]` `census[` appears in exactly two of the
/// seventeen committed boot logs, and in none since doorbells began to be served — the
/// address plane's only diagnostic was gated on the execution plane failing. See
/// [`kayfabe_core::gpu::Gpu::vas_census_string`].
/// ★★★★ Bumped to **35** at §16.56, the ABI-3 reason a twenty-third time:
/// [`KayfabeBridgeRefusal`] gained [`Self::ids`] and `ids_len`
/// ([`REFUSAL_IDS_PER_TAG`] words plus a length), so an ABI-34 shim would allocate the old
/// layout and this archive would write **`32 × 40` = 1 280 bytes** past the end of it.
///
/// ⊘ And this one closes a hole in the *reporting* rather than adding a measurement:
/// `[measured 2026-08-10, over traces/guest_boots/*_qemu.log]` `grep -c hClass` over every
/// committed device log returns **0** — this port had never once named a class it refused.
/// `NotOnAllowlist x10` was the whole report, and answering *which ten* meant reading the
/// **guest's** dmesg (§16.55.4), a plane we neither own nor always capture.
/// ★★★★ Bumped to **36** at §16.65, the ABI-3 reason a twenty-fourth time:
/// [`KayfabeRegAudit`] gained the per-engine doorbell census
/// ([`KayfabeRegAudit::doorbells_by_engine`], `doorbells_engine_unrouted`) and the
/// served split ([`KayfabeRegAudit::doorbells_served_locally`] / `_forwarded`), so an ABI-35
/// shim would allocate the old layout and this archive would write past the end of it.
///
/// ⊘ And this one exists because a **count could not answer the question it was being
/// asked**: `[measured 2026-08-10, boots s49/s50]` `448 arrived / 354 served / 94 refused`,
/// with a **16-line** bounded sample beside it as the only per-channel evidence. Two
/// different refutations of §16.65's routing hypothesis — *"`EngineKind` does not partition
/// doorbell traffic"* and *"the engine refinement never reached UVM's channels"* — produce
/// the **same** three numbers. A census that cannot separate them is not an instrument.
/// ★★★★★ Bumped to **37** at §16.76, the ABI-3 reason a twenty-fifth time:
/// [`KayfabeRegAudit`] gained the whole os-event wakeup plane — the registry's counts, the
/// flow-control gate's, the GSP stall-vector raises and the `IRQSCLR` opener — so an ABI-36
/// shim would allocate the old layout and this archive would write past the end of it.
///
/// ⊘ And this one exists because **two opposite findings produce the same silence**: a
/// `cuCtxCreate` that never returns looks identical whether this device never woke the
/// waiter or woke it with an unchanged semaphore behind it. `os_event_woke_with_nothing`
/// is the field that separates them, and it is worth an ABI bump for that alone.
pub const ABI_VERSION: u32 = 38;

/// ★★★★ §16.65 — how many engine buckets the doorbell census has. Must equal
/// `KAYFABE_ENGINE_KINDS` and `kayfabe_rt::ENGINE_KIND_COUNT`.
pub const ENGINE_KINDS: usize = kayfabe_rt::ENGINE_KIND_COUNT;

/// ★★★★ §16.56 — how many refused ids each `FaultTag` row carries across the ABI. Must
/// equal `KAYFABE_REFUSAL_IDS_PER_TAG` and `kayfabe_rmrpc::REFUSAL_DETAIL_CAP`.
pub const REFUSAL_IDS_PER_TAG: usize = 8;

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
    /// ★★ The region has a backing file the hypervisor could identify. When false the three
    /// fields below are meaningless and are **not** read.
    ///
    /// ⊘ A separate flag rather than a sentinel in `backing_ino`, because zero is a legal
    /// inode number on some filesystems and "the value that means unmeasured" must not be a
    /// value the measurement can produce.
    pub fd_backed: bool,
    /// `st_dev` of the backing file. Meaningful only when `fd_backed`.
    pub backing_dev: u64,
    /// `st_ino` of the backing file. Meaningful only when `fd_backed`.
    pub backing_ino: u64,
    /// Byte offset into the backing file at which the **region** begins. Meaningful only
    /// when `fd_backed`.
    pub file_offset_of_region: u64,
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
            backing: self.fd_backed.then_some(SectionBacking {
                dev: self.backing_dev,
                ino: self.backing_ino,
                file_offset_of_region: self.file_offset_of_region,
            }),
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

/// ★★★★ §16.40 — how many bytes of the promote-ctx diagnosis cross the ABI.
///
/// Sized like [`DOORBELL_REFUSAL_LEN`] and for the same measured reason: the sentence
/// carries a per-channel VA-space census, and `s25_01d12e6_cup2`'s census alone is 512
/// bytes at six channels. A `cup2` boot that reaches `cuCtxCreate` holds more. ⊘
/// [`copy_sentence`] stamps a visible `[CLIPPED …]` tail rather than truncating silently,
/// so a clipped diagnosis can never read as a complete one.
pub const PROMOTE_DIAG_LEN: usize = 2048;

/// ★★★★ §16.40 — how many promote-ctx refusal KINDS cross the ABI.
///
/// `kayfabe_core::promote::PromoteFault` has ten variants, so this is bounded by a fixed
/// finite set and never by anything the guest supplies. Four is every kind any boot has
/// produced (`s35`/`s36`: two), and `promote_diag_len` reports the truth past the array so
/// a full one is never mistaken for a complete list.
pub const PROMOTE_DIAG_SLOTS: usize = 4;
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
    /// ★★★★ **§16.56 — the IDENTIFIERS the tag cannot carry**: the `hClass` values under
    /// an `AllocClassNotPermitted` / `UnmappedAllocClass` row, the `cmd` values under a
    /// `ControlNotPermitted` / `UnknownControl` row. Ascending; entries at or past
    /// [`Self::ids_len`] carry no meaning.
    ///
    /// ⊘ **A `FaultTag` is a `&'static str`**, so a refusal *about a value* lost that
    /// value the instant it became a census key — and the census keys by tag alone. The
    /// consequence is measured, not feared: no committed device log has ever printed an
    /// `hClass`, so no `grep` over our own evidence could answer *"which class did we
    /// refuse?"*. A method prescribed on that basis — *"enumerate the refused classes,
    /// then filter"* — could not have terminated.
    pub ids: [u32; REFUSAL_IDS_PER_TAG],
    /// How many entries of [`Self::ids`] are populated. ★ Capped at
    /// [`REFUSAL_IDS_PER_TAG`] while [`Self::count`] is **not** capped, so a truncated id
    /// list can never read as a complete one: `n` ids beside a larger count is a visible
    /// truncation (`a_saturated_instrument_looks_exactly_like_absence`).
    pub ids_len: u64,
}

/// ★★★★ §16.40 — one promote-ctx refusal KIND, with the address plane's state at the first
/// refusal carrying it.
///
/// Mirrors [`KayfabeBridgeRefusal`]'s shape (NUL-**padded**, explicit lengths, `Default`
/// written out because the arrays are wider than the derive covers) for its reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct KayfabePromoteDiag {
    /// The [`kayfabe_trace::FaultTag`]'s bytes, NUL-padded.
    pub tag: [u8; BRIDGE_REFUSAL_TAG_LEN],
    /// How many bytes of [`Self::tag`] are the name.
    pub tag_len: u64,
    /// The sentence: the fault's own fields, then the VA-space census. NUL-padded.
    pub text: [u8; PROMOTE_DIAG_LEN],
    /// How many bytes of [`Self::text`] are the sentence.
    pub text_len: u64,
}

impl Default for KayfabePromoteDiag {
    fn default() -> KayfabePromoteDiag {
        KayfabePromoteDiag {
            tag: [0; BRIDGE_REFUSAL_TAG_LEN],
            tag_len: 0,
            text: [0; PROMOTE_DIAG_LEN],
            text_len: 0,
        }
    }
}

impl Default for KayfabeBridgeRefusal {
    fn default() -> KayfabeBridgeRefusal {
        KayfabeBridgeRefusal {
            tag: [0; BRIDGE_REFUSAL_TAG_LEN],
            tag_len: 0,
            count: 0,
            ids: [0; REFUSAL_IDS_PER_TAG],
            ids_len: 0,
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
    /// ★★★★ `#16.18` — reads served through the GMMU from the translated framebuffer/`BAR1`
    /// window.
    pub bar1_reads: u64,
    /// ★★★★ `#16.18` — writes served through it.
    pub bar1_writes: u64,
    /// ★★★★ `#16.18` — framebuffer-aperture accesses refused **by name**.
    pub bar1_faults: u64,
    /// ★★★★ `#16.18` — **the precondition every other `bar1_*` number needs**: the
    /// framebuffer address this port told the guest BAR1's root page directory sits at
    /// (`GspStaticConfigInfo.bar1PdeBase`), or `0` for a chip row with no
    /// framebuffer-aperture address model.
    ///
    /// ⊘ Carried rather than implied. A boot in which `bar1_writes` and `bar1_faults` are
    /// both zero says two completely different things depending on this field: with a root,
    /// the guest never touched the aperture; without one, we never had anywhere to put a
    /// byte and the zeros are about us.
    pub bar1_pde_base: u64,
    /// ★★★★ `#16.18` — `1` iff the guest ever published a BAR1 root over `UPDATE_BAR_PDE`.
    ///
    /// ⊘ **Expected to be `0`, and a `1` would be a refutation.** `NV_RM_RPC_UPDATE_BAR_PDE`
    /// has two call sites in `ogkm-580` and both pass `NV_RPC_UPDATE_PDE_BAR_2`; the whole
    /// of [`Self::bar1_pde_base`]'s reason for existing is that BAR1's root travels the
    /// other way. This field is what makes that claim **measured on every boot** instead of
    /// argued once from a grep — the same reason `bar_pde_updates` carries a count beside a
    /// value.
    pub bar1_root_published: u64,
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
    /// ★★★★★ §16.76: os-event batches this device **announced** — one GSP stall vector
    /// (155 on GA106) latched and one message delivered per batch, never per event.
    pub gsp_event_raises: u64,
    /// §16.76: batches posted that could **not** be announced — the chip's captured
    /// interrupt table named no usable `MC_ENGINE_IDX_GSP` stall vector. Must be zero.
    pub gsp_event_unvectored: u64,
    /// §16.76: of the raises, how many the guest's own `LEAF_EN` would hide.
    pub gsp_event_masked: u64,
    /// ★★★ §16.76: `IRQSCLR` writes — **the opener**, the only thing that reopens the
    /// event flow-control gate. Zero here with a non-zero `gsp_event_raises` means the gate
    /// is latched shut after one batch.
    pub status_irq_cleared: u64,
    /// §16.76: distinct `(hClient, hEvent)` os-events ever registered.
    pub os_events_registered: u64,
    /// §16.76: registrations a guest `FREE` retired. ⊘ Posting to a dead pair desyncs the
    /// SHARED status queue and wedges the whole RPC path — see `kayfabe_device::osevent`.
    pub os_events_retired: u64,
    /// §16.76: registrations live at teardown.
    pub os_events_live: u64,
    /// §16.76: `NV01_EVENT_OS_EVENT` allocs whose params this port could not read.
    pub os_events_malformed: u64,
    /// §16.76: registrations refused because the table was full. ⊘ It refuses, never
    /// evicts: an eviction would silently stop waking a waiter that is still there.
    pub os_events_overflowed: u64,
    /// §16.76: `POST_EVENT` messages put on the wire.
    pub os_event_posted: u64,
    /// §16.76: batches delivered — one interrupt each.
    pub os_event_batches: u64,
    /// ★★★ §16.76: delivery attempts the flow-control gate refused. Healthy in steady
    /// state; large beside `os_event_batches == 1` means the gate is stuck.
    pub os_event_gated: u64,
    /// §16.76: attempts made before the guest drained `GSP_INIT_DONE`.
    pub os_event_not_running: u64,
    /// §16.76: attempts that posted nothing at all — the ring refused the first message.
    pub os_event_failed: u64,
    /// ★★★★★ §16.76: batches announced with **no newly-served doorbell behind them** — a
    /// wakeup with nothing to see, because none of the guest's work executed. ⊘ NOT a call
    /// to write a semaphore: the host GPU DMAs that into guest RAM on the passthrough path
    /// and this VMM is not in it. Read this before concluding delivery worked.
    pub os_event_woke_with_nothing: u64,
    /// §16.76: `doorbells_served` as of the last announced batch — the only honest proxy
    /// for *"the guest's work ran, so the host GPU has something to DMA"*.
    pub os_event_last_join_served: u64,
    /// §16.76: of those, how many were **forwarded** — the passthrough path, where the host
    /// GPU writes the release semaphore into guest RAM and this device is not involved.
    pub os_event_last_join_forwarded: u64,
    /// §16.76: how many local servings were new at the last announced batch. ⊘ Zero is the
    /// finding.
    pub os_event_last_join_advanced: u64,
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
    /// ★★★★ **§16.62.3 — of the served, the ones the SHELL's own CPU executor ran.** See
    /// `kayfabe_device::Counters::doorbells_served_locally` for why *"354 served"* was a
    /// number nobody could read.
    pub doorbells_served_locally: u64,
    /// ★★ Of the served, the ones handed to a **host** channel.
    /// `locally + forwarded == served`, always.
    pub doorbells_served_forwarded: u64,
    /// ★★★★ **§16.65 — THE PER-ENGINE DOORBELL CENSUS**, bucketed by
    /// `kayfabe_rt::EngineKind::index` in [`ENGINE_KINDS`] order. See [`DoorbellCensus`].
    ///
    /// ⊘ A fixed array with a name-table beside it in C, never a list of pairs: an empty
    /// bucket is a **measurement** (*"no NVENC channel rang"*), and a sparse encoding would
    /// make it indistinguishable from *"we did not look"* — the oracle's fifth-limit
    /// mistake, one plane over.
    pub doorbells_by_engine: [u64; ENGINE_KINDS],
    /// ★ Doorbells whose channel did not resolve at all, so no engine could be named.
    /// `sum(doorbells_by_engine) + doorbells_engine_unrouted == doorbells`, always.
    pub doorbells_engine_unrouted: u64,
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
    /// ★★★★ **§16.40 — the FIRST refused `GPU_PROMOTE_CTX`, with the address plane's state
    /// as it stood at that instant.** NUL-padded; [`Self::promote_diag_len`] is the length.
    ///
    /// ⊘ **Empty is a finding, not a blank.** A zero length means no promotion was ever
    /// refused, which — read beside the `0x2080012b` rows in the served-control census —
    /// discriminates "every promotion succeeded" from "none arrived". It never means the
    /// instrument was off.
    ///
    /// See `kayfabe_rmrpc::SharedPromoteDiag` for why one sentence rather than a census,
    /// and `kayfabe_core::gpu::Gpu::vas_census_string` for why it is sampled at the
    /// refusal instead of here.
    pub promote_diag: [KayfabePromoteDiag; PROMOTE_DIAG_SLOTS],
    /// How many **distinct** promote-refusal kinds were latched — the truth even past
    /// [`PROMOTE_DIAG_SLOTS`]. `0` = no promotion was ever refused.
    pub promote_diag_len: u64,
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
    /// ★★★★ §16.30 — how many `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` (`0x00801813`)
    /// commands this port **accepted**, including re-installations.
    ///
    /// ⚠ **`0` here is a FINDING and not a quiet success.** `[measured 2026-08-09, boots
    /// `s26_0484a3b_cup2` / `s27_c73d3ab_uvm`]` the control arrived and was refused, and
    /// RM's rollback (`ogkm-580: dma.c:531-551`) is what fires the one `dmesg` line unique
    /// to `cuInit`'s window. A boot in which this stays `0` did not test the rung.
    pub set_page_dir_total: u64,
    /// How many arrived and were **refused** — FINN-serialized, or a declared `paramsSize`
    /// that is not `sizeof`. Non-zero invalidates the record beside it.
    pub set_page_dir_refused: u64,
    /// ★★★ Whether [`Self::set_page_dir_h_vaspace`] and its siblings mean anything.
    ///
    /// ⊘⊘ **Load-bearing, and the sharpest `_valid` in this struct.** `hVASpace == 0` is a
    /// **real handle value** — it names the client/device pair's implicit VA space
    /// (`ogkm-580: ctrl0080dma.h:812-815`) — so a reported `0` with no `_valid` beside it
    /// cannot be told from *"no SET ever arrived"*. Every other field here has the same
    /// hazard at `0`. Reading any of them without this one first is how an absence gets
    /// decoded as a measurement.
    pub set_page_dir_valid: u64,
    /// `hClient` from the RPC control header of the most recent accepted `SET`.
    pub set_page_dir_client: u64,
    /// `hObject` from that header — **`hDevice`**, not the VA space
    /// (`ogkm-580: dma.c:508-518`). ⚠ The opposite convention from `0x90f10106`, whose
    /// header `hObject` **is** the VA space; see [`Self::gvas_pub`].
    pub set_page_dir_object: u64,
    /// ★★★ `hVASpace` from the **params** — the VA space this root is installed into,
    /// reported exactly as it arrived.
    ///
    /// ⊘ **Not interpreted here and not interpreted by the printer.** Whether this boot
    /// sends `0` (the Device's implicit VA space) or a real handle (a user VA space, which
    /// is what UVM allocates) is the open question §16.30 exists to answer from a boot
    /// rather than from header semantics.
    pub set_page_dir_h_vaspace: u64,
    /// `physAddress` — where the guest put the page directory, in the aperture named by
    /// [`Self::set_page_dir_flags`]. Guest-physical.
    pub set_page_dir_phys: u64,
    /// `numEntries` — the directory's size in entries.
    ///
    /// ★ Carried because it, not the address, decides RM's next three checks
    /// (`ogkm-580: gpu_vaspace.c:3093-3097`): a root smaller than the RM-managed region of
    /// the VA heap fails `commit` *after* this port answered `NV_OK`.
    pub set_page_dir_num_entries: u64,
    /// `flags`, raw — aperture in bits `1:0`, plus `ALL_CHANNELS`, `EXTEND_VASPACE`,
    /// `IGNORE_CHANNEL_BUSY`.
    pub set_page_dir_flags: u64,
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
            bar1_reads: Default::default(),
            bar1_writes: Default::default(),
            bar1_faults: Default::default(),
            bar1_pde_base: Default::default(),
            bar1_root_published: Default::default(),
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
            gsp_event_raises: Default::default(),
            gsp_event_unvectored: Default::default(),
            gsp_event_masked: Default::default(),
            status_irq_cleared: Default::default(),
            os_events_registered: Default::default(),
            os_events_retired: Default::default(),
            os_events_live: Default::default(),
            os_events_malformed: Default::default(),
            os_events_overflowed: Default::default(),
            os_event_posted: Default::default(),
            os_event_batches: Default::default(),
            os_event_gated: Default::default(),
            os_event_not_running: Default::default(),
            os_event_failed: Default::default(),
            os_event_woke_with_nothing: Default::default(),
            os_event_last_join_served: Default::default(),
            os_event_last_join_forwarded: Default::default(),
            os_event_last_join_advanced: Default::default(),
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
            doorbells_served_locally: Default::default(),
            doorbells_served_forwarded: Default::default(),
            doorbells_by_engine: Default::default(),
            doorbells_engine_unrouted: Default::default(),
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
            promote_diag: [KayfabePromoteDiag::default(); PROMOTE_DIAG_SLOTS],
            promote_diag_len: Default::default(),
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
            set_page_dir_total: Default::default(),
            set_page_dir_refused: Default::default(),
            set_page_dir_valid: Default::default(),
            set_page_dir_client: Default::default(),
            set_page_dir_object: Default::default(),
            set_page_dir_h_vaspace: Default::default(),
            set_page_dir_phys: Default::default(),
            set_page_dir_num_entries: Default::default(),
            set_page_dir_flags: Default::default(),
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

/// ★★★★★ §16.80 — how many engine-object forward outcomes get printed per process life,
/// **PER OUTCOME CLASS** (forwards and refusals have separate budgets).
///
/// ⊘ Bounded, because a hostile guest can issue allocs in a loop and a diagnostic that a
/// guest can turn into unbounded host output is a denial-of-service with a log format.
/// 32 is effectively unbounded for the real workload — `cuCtxCreate` allocs **one**
/// `AMPERE_COMPUTE_B` and one `AMPERE_DMA_COPY_B` — and the running totals on every line
/// mean the bound costs detail, never the count.
///
/// # ⊘⊘ §16.105 — IT USED TO BE ONE SHARED BUDGET, AND THAT BUDGET IS WHY "12" EXISTS
///
/// `[measured 2026-08-11, boots `w250_acbb9a3_hostdmesg2` and `w251_acbb9a3_cel_hostdmesg`,
/// both committed under `traces/guest_boots/`]` — the two boots printed **18 forwards and
/// 12 refusals**, the 12th refusal is `seen=32`, and it carries the bound marker. The host
/// driver logged **14** `chandesConstruct_IMPL` failures in the same boots. Three separate
/// hypotheses were raised and refuted for that 14-vs-12 gap (§16.102, §16.103) while the
/// difference was **the last two refusals falling off the end of a shared 32-line budget**:
/// 18 + 14 = 32 + 2.
///
/// ⇒ ★★ **A count read off a TRUNCATED census is a count of what was printed.** The old
/// wording — *"the bound costs detail, never the count"* — was true only of `[seen=…]`,
/// which nothing downstream ever read; the numbers people actually counted were the lines.
/// Splitting the budget per class makes the real workload's whole shape (18 + 14) printable
/// while leaving a hostile guest bounded at 2 × 32 lines.
///
/// ⊘ It does not make truncation impossible — it makes it **visible per class**, because
/// the marker is now emitted for whichever class hits its own bound.
///
/// # ⊘⊘ §16.107 — 32 WAS STILL TOO SMALL, AND IT SATURATED IN THE VERY NEXT BOOT
///
/// `[measured 2026-08-11, boot `w255_76477ab_cel_runlist`]` — with §16.106's fix the 14
/// refusals became forwards, and the forward class printed **exactly 32** with the bound
/// marker on its last line. ⇒ the same defect, one rung later, **on the other class, in a
/// census this file had just rewritten**: `forwarded=32` was a lower bound wearing the
/// shape of a total. ★ A saturated instrument does not become safe because its author
/// knows about the last one.
///
/// So the bound is **256 per class** — eight times the largest shape ever observed — and,
/// more importantly, the *rows* are what it caps. See [`report_engine_forward`]: past the
/// bound the three **totals** keep being printed on a doubling schedule, so a guest can
/// buy silence for the detail and never for the count. That is the same shape every other
/// bounded census in this tree already had (`kayfabe_device::census`'s `*_total` /
/// `*_distinct` beside a capped `Vec`), and its absence here is what made §16.105
/// possible.
const ENGINE_FWD_REPORT_MAX: u64 = 256;

/// Outcomes seen / of those, forwarded / of those, refused. Process-global for
/// [`ISOLATE_PLANE_ENV`]'s stated reason: this is bench instrumentation on a
/// one-device-per-process bench, and a per-device home costs a shim-ABI change.
static ENGINE_FWD_SEEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static ENGINE_FWD_OK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// ★ §16.105 — counted in its OWN atomic rather than derived as `seen - ok`, so the
/// per-class budget below is exact under the concurrent `Relaxed` increments rather than
/// approximately right (`no_instant_at_which_it_is_live_and_complete`).
static ENGINE_FWD_REFUSED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// ★★★ §16.105 — how a refusal names **the host channel the alloc was attempted on**.
///
/// The identity comes out of [`kayfabe_isolate::VerbFailure::on`], through
/// `kayfabe_fwd::FwdFault::Rm { on, .. }`; see that field for why nothing on this side can
/// re-derive it (the channel is materialized inside the failing chain and freed by its
/// unwind).
///
/// ⊘ **`NONE` is printed, never omitted.** A missing field reads as "no channel involved";
/// an explicit `NONE` says the fault carried no target, which is a different fact and is
/// the one that distinguishes *"the object alloc failed on a channel"* from *"the chain
/// never got as far as a channel"*.
fn refusal_host_chan(fault: &kayfabe_rt::FwdFault) -> String {
    match fault {
        kayfabe_rt::FwdFault::Rm { on: Some(h), .. } => format!("{:#x}", h.raw()),
        _ => "NONE".to_string(),
    }
}

/// What [`report_engine_forward`] emits for one outcome — the decision, separated from the
/// printing so the whole policy is testable (the same shape `host_version_gate` uses one
/// crate over).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EngineFwdReport {
    /// The full row: class, handles, verdict, and the running counts.
    Row,
    /// ★ Past the per-class row budget — the three **totals** only.
    TotalsOnly,
    /// Nothing. The counts are still advancing and a later `TotalsOnly` will say so.
    Silent,
}

/// ★★★★★ **§16.107 — THE ROWS ARE CAPPED; THE COUNTS ARE NOT.**
///
/// `nth` is this outcome's index **within its own class** (forwards and refusals have
/// separate row budgets); `seen` is the index across **all** outcomes.
///
/// - `nth <= ENGINE_FWD_REPORT_MAX` ⇒ [`EngineFwdReport::Row`].
/// - past it, [`EngineFwdReport::TotalsOnly`] whenever `seen` is a power of two ⇒ a guest
///   issuing `n` allocations buys silence for the *detail* at a cost of ~`log2(n)` lines,
///   and can never buy silence for the *count*.
///
/// # ⊘ Why the totals schedule is keyed on `seen` and not on `nth`
///
/// `nth` advances only within one class. A workload that saturates the forward budget and
/// then issues nothing but forwards would never advance the refusal `nth`, so a schedule
/// keyed on `nth` could stall in exactly the case that needs reporting. `seen` advances on
/// every outcome, so the totals keep coming whatever the mix is.
///
/// # ⊘⊘ What this exists to prevent, twice measured
///
/// `[w250/w251]` a **shared** 32-row budget made `18 + 2 + 12 = 32` read as a total; three
/// rungs were spent explaining a 14-vs-12 gap that was two unprinted lines (§16.105).
/// `[w255]` the per-class split then saturated the OTHER class at exactly 32 forwards, in
/// this same census, one rung later (§16.106.6). ⇒ raising the number alone would fix an
/// instance and leave the class. **This function is the fix for the class**: past the
/// bound the census keeps stating what it saw, so a number printed here can be read as a
/// total by construction rather than by luck.
fn engine_fwd_report_action(nth: u64, seen: u64) -> EngineFwdReport {
    if nth <= ENGINE_FWD_REPORT_MAX {
        EngineFwdReport::Row
    } else if seen.is_power_of_two() {
        EngineFwdReport::TotalsOnly
    } else {
        EngineFwdReport::Silent
    }
}

/// Print one engine-object forward outcome into the boot's own `run_<tag>_qemu.log`.
///
/// ★ It prints the **whole** outcome — reused/materialized/host handle on success, the
/// exact `FwdFault` variant on refusal — because "how many were forwarded" is the number
/// that has already been over-read once on this project: `forwarded_counts_intent_not_work`
/// is a count that meant nothing was forwarded. A variant name cannot be read that way.
fn report_engine_forward(
    client: kayfabe_rt::HClient,
    parent: kayfabe_rt::HObject,
    class: kayfabe_rt::ClassId,
    params_len: usize,
    out: &Result<kayfabe_rt::EngineObjectForwarded, kayfabe_rt::FwdFault>,
) {
    use std::sync::atomic::Ordering::Relaxed;
    let seen = ENGINE_FWD_SEEN.fetch_add(1, Relaxed) + 1;
    // ★ `nth` is this outcome's index WITHIN ITS OWN CLASS — the number the per-class
    // budget is spent against. See `ENGINE_FWD_REPORT_MAX`.
    let (ok, refused, nth) = match out {
        Ok(_) => {
            let ok = ENGINE_FWD_OK.fetch_add(1, Relaxed) + 1;
            (ok, ENGINE_FWD_REFUSED.load(Relaxed), ok)
        }
        Err(_) => {
            let refused = ENGINE_FWD_REFUSED.fetch_add(1, Relaxed) + 1;
            (ENGINE_FWD_OK.load(Relaxed), refused, refused)
        }
    };
    match engine_fwd_report_action(nth, seen) {
        EngineFwdReport::Row => {}
        EngineFwdReport::TotalsOnly => {
            eprintln!(
                "kayfabe: ENGINE-OBJECT CENSUS [seen={seen} forwarded={ok} refused={refused}] \
                 ⊘ ROWS are capped at {ENGINE_FWD_REPORT_MAX} per outcome class; THESE THREE \
                 COUNTS ARE NOT, and are the only numbers here that may be read as totals"
            );
            return;
        }
        EngineFwdReport::Silent => return,
    }
    let verdict = match out {
        Ok(f) => format!(
            "FORWARDED engine={:?} host_object={:#x} materialized_channel={} reused={}",
            f.engine,
            f.host_object.raw(),
            f.materialized_channel,
            f.reused,
        ),
        // ★★★ §16.105 — `host_chan=` is the JOIN KEY, printed beside the variant rather
        // than only inside its `Debug`, so a reader can group refusals by the channel they
        // were attempted on without parsing a derive.
        Err(e) => format!("REFUSED host_chan={} {e:?}", refusal_host_chan(e)),
    };
    eprintln!(
        "kayfabe: ENGINE-OBJECT class={:#06x} client={:#x} parent={:#x} params={}B \
         → {verdict} [seen={seen} forwarded={ok} refused={refused}]{}",
        class.0,
        client.0,
        parent.0,
        params_len,
        if nth == ENGINE_FWD_REPORT_MAX {
            " ⊘ REPORT BOUND REACHED for this outcome class — later ones are counted, not printed"
        } else {
            ""
        },
    );
}

/// ★★ **§16.96 — the drain's own budget, and it is a fraction of the guest's.**
///
/// The verb runs inside the vCPU's MMIO trap, so its duration is time the guest spends
/// *not* polling — it is charged directly against `_kgspRpcRecvPoll`'s deadline, which is
/// `defaultus + defaultus/2` = **6 s** (`ogkm-580: kernel_gsp.c:2379` over
/// `arch/nvalloc/unix/src/os.c:1978`). ⚠ Three consecutive RPC timeouts mark the GPU **for
/// reset** (`RPC_TIMEOUT_GPU_RESET_THRESHOLD`, `kernel_gsp.c:2455`), so overrunning is
/// terminal rather than slow.
///
/// ⊘ **One second, not six**, deliberately: a warning that only fires *at* the deadline
/// fires when the guest has already timed out, which is a post-mortem, not an instrument. A
/// host RM round-trip that takes a second has already gone wrong.
const ENGINE_FWD_DRAIN_BUDGET: std::time::Duration = std::time::Duration::from_secs(1);

/// ★★★ **§16.96 — run the latched engine-object forwards and REPORT, lock-free.**
///
/// Called from `Regs::write` after `RegPlane::write` has returned. Separate from the
/// `ObjectModel` impl on purpose: that impl runs under the plane's rank-0 mutex and may only
/// *admit*; this runs with no lock and is the only place a forward's real outcome exists.
fn report_engine_forward_drain(device: &kayfabe_rt::device::SharedDevice) {
    let t0 = Instant::now();
    let runs = device.run_pending_engine_forwards();
    if runs.is_empty() {
        // ⊘ The overwhelmingly common case, and it must cost nothing beyond the one rank-1
        // acquisition `run_pending_engine_forwards` already paid — the same cost
        // `materialize_pending` pays on every register write.
        return;
    }
    let elapsed = t0.elapsed();
    for r in &runs {
        report_engine_forward(r.client, r.parent, r.class, r.params_len, &r.out);
    }
    if elapsed >= ENGINE_FWD_DRAIN_BUDGET {
        // ★★ LOUD, and NOT subject to `ENGINE_FWD_REPORT_MAX`. The report cap exists so a
        // chatty census cannot drown a serial log; an overrun is the opposite — it is the
        // one line whose absence would be read as health. See [`ENGINE_FWD_DRAIN_BUDGET`]
        // for the 6 s clock this is a fraction of, and for why three of them reset the GPU.
        eprintln!(
            "kayfabe: ⚠⚠⚠ ENGINE-FORWARD DRAIN OVERRUN — {} forward(s) took {:?}, over the \
             {:?} budget. ⚠ This time is charged against the guest's `_kgspRpcRecvPoll` \
             deadline of 6 s (ogkm-580 kernel_gsp.c:2379 over os.c:1978); THREE consecutive \
             RPC timeouts mark the GPU for reset (:2455). classes=[{}]",
            runs.len(),
            elapsed,
            ENGINE_FWD_DRAIN_BUDGET,
            runs.iter()
                .map(|r| format!("{:#06x}", r.class.0))
                .collect::<Vec<_>>()
                .join(","),
        );
    }
}

impl kayfabe_rmrpc::ObjectModel for SharedObjectModel {
    fn apply(
        &mut self,
        ev: kayfabe_core::rmgraph::RmEvent,
    ) -> Result<(), kayfabe_core::gpu::GpuError> {
        // ⊘⊘ `apply_deferring`, NEVER `apply`. `[measured 2026-08-11, §16.88]` every
        // production call reaching here arrives from `RegPlane::write` with the plane's
        // **rank-0** mutex held, six crates up — `apply` would `clone`+`execveat` a sandboxed
        // child under it and abort QEMU on the guest's first `GSP_RM_ALLOC`.
        // ★ The spawn stays LATCHED in the spine; `Regs::write` drains it once the plane's
        // guard is down. See `SharedDevice::apply_deferring`.
        self.0.apply_deferring(ev)
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

    fn schedule_group(
        &mut self,
        client: kayfabe_rt::HClient,
        object: kayfabe_rt::HObject,
        enable: bool,
    ) -> Result<kayfabe_core::gpu::ScheduleGroupAck, kayfabe_core::gpu::ScheduleGroupFault> {
        self.0.schedule_group(client, object, enable)
    }

    /// ★★★★ §16.59 — the shell's seat for `0x20801210`. ⚠ It exists **because**
    /// `SharedObjectModel::as_gpu` is `None` by design: an arm written against `as_gpu`
    /// would refuse on every real boot and pass every bare-`Gpu` test.
    fn set_ctxsw_preemption_mode(
        &self,
        client: kayfabe_rt::HClient,
        h_channel: kayfabe_rt::HObject,
    ) -> Result<kayfabe_core::gpu::CtxswPreemptionAck, kayfabe_core::gpu::CtxswPreemptionFault>
    {
        self.0.set_ctxsw_preemption_mode(client, h_channel)
    }

    fn bind_channel(
        &mut self,
        client: kayfabe_rt::HClient,
        object: kayfabe_rt::HObject,
        rm_engine_type: u32,
    ) -> Result<kayfabe_core::gpu::BindAck, kayfabe_core::gpu::BindFault> {
        self.0.bind_channel(client, object, rm_engine_type)
    }

    /// ★★★★★ §16.80 — the shell's seat for the Case-1 engine-object forward, and **the
    /// place its outcome is named**.
    ///
    /// `Bridge::deliver` discards the `Result` on purpose (the guest's alloc has already
    /// been answered, and turning a host refusal into an alloc failure would be a
    /// different experiment). So if this implementation does not report, the forward is
    /// unobservable — `a_diagnostic_gated_on_the_failure` in advance rather than after.
    ///
    /// ⊘ `NotAnEngine` is silent by design: it is the gate, not an event. Every alloc the
    /// guest makes passes through here and all but a handful are clients, devices, memory
    /// and VA spaces.
    fn forward_engine_object(
        &mut self,
        client: kayfabe_rt::HClient,
        parent: kayfabe_rt::HObject,
        class: kayfabe_rt::ClassId,
        params: &[u8],
    ) -> kayfabe_rmrpc::EngineObjectOutcome {
        // ⊘⊘ `forward_engine_object_deferring`, NEVER `forward_engine_object_by_parent`.
        // `[measured 2026-08-11, §16.91, `traces/boots/w239/`]` every production call
        // reaching here arrives from `RegPlane::write` with the plane's **rank-0** mutex
        // held, six crates up — the direct call issues a host RM ioctl there and aborts
        // QEMU on the guest's first engine-object `GSP_RM_ALLOC`:
        //   `R1 no-blocking-under-lock violation: issuing a host RM verb while holding
        //    rank(s) [0]`
        // ★ The request stays LATCHED in the device; `Regs::write` runs it once the plane's
        // guard is down and reports the outcome there. See
        // `SharedDevice::forward_engine_object_deferring` for why THIS verb may be latched
        // at all when the spawn's sibling argument said it could not.
        match self
            .0
            .forward_engine_object_deferring(client, parent, class, params)
        {
            // ⊘ Silent by design: it is the gate, not an event. Every alloc the guest makes
            // passes through here and all but a handful are clients, devices, memory and VA
            // spaces. Reported as `Served` because it is genuinely resolved — no host
            // round-trip is owed and none will happen.
            kayfabe_rt::ForwardAdmission::NotAnEngine(c) => {
                kayfabe_rmrpc::EngineObjectOutcome::Served(Err(kayfabe_rt::FwdFault::NotAnEngine(
                    c,
                )))
            }
            // ★ ADMITTED ONLY. ⊘ Deliberately NOT reported here: a line printed now would
            // say *"forwarded"* about a verb that has not run, which is
            // `forwarded_counts_intent_not_work` reproduced exactly. The drain reports.
            kayfabe_rt::ForwardAdmission::Latched { .. } => {
                kayfabe_rmrpc::EngineObjectOutcome::Deferred
            }
            // ★★ THE BOUND, and it is LOUD and unbounded-by-the-report-cap: an engine
            // object the guest asked for and we never even attempted is a hole in the
            // census, and a hole that only shows up as a missing line is the shape
            // `no_counter_fired_is_not_no_record_exists` warns about.
            kayfabe_rt::ForwardAdmission::LatchFull { pending, bound } => {
                eprintln!(
                    "kayfabe: ⚠⚠⚠ ENGINE-FORWARD LATCH FULL — REFUSED BY NAME. \
                     class={:#06x} client={:#x} parent={:#x} params={}B pending={pending} \
                     bound={bound}. ⊘ This forward was NEVER ATTEMPTED. The guest's alloc \
                     was still answered locally, so the guest sees no error and this line \
                     is the only record.",
                    class.0,
                    client.0,
                    parent.0,
                    params.len(),
                );
                kayfabe_rmrpc::EngineObjectOutcome::DeferralFull { pending, bound }
            }
        }
    }

    /// `None`, and that is the whole of what a sharded shell can honestly say: the graph
    /// lives inside a device lock and a proc lock, and a `&Gpu` handed out here would
    /// outlive both guards.
    /// ★★★★ §16.40 — the census the sharded shell CAN answer, where `as_gpu` cannot.
    ///
    /// ⊘ This is the impl that makes the promote diagnosis work on a real boot:
    /// [`kayfabe_rt::device::SharedDevice::channel_vas_census`] walks the live set one
    /// rank-1 lock at a time (R3), which is the only legal read here, and the formatting
    /// is `kayfabe_core`'s so it cannot drift from the doorbell path's.
    fn vas_census(&self, mark: Option<kayfabe_core::ChanId>) -> String {
        kayfabe_core::gpu::format_vas_census(&self.0.channel_vas_census(), mark)
    }

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
    /// ★★★★★ §5.8 — the filesystem identity of the guest-RAM block this root ADOPTED, or
    /// `None` when the crossing is not armed.
    ///
    /// ⊘ **Carried from the composition root, never re-derived**, for [`Regs`]'s own copy's
    /// reason: re-taking the descriptor census here would be a SECOND selection of "which
    /// block is guest RAM", and two projections of one fact have been measured disagreeing
    /// three times in this project.
    ///
    /// ★ It is also the arming flag for the whole pin path below. `None` ⇒ this port does
    /// not print one guest-RAM line, which is what keeps the negative control
    /// byte-comparable to the armed run.
    guest_ram_backing: Option<kayfabe_vmm_qemu::layout::BackingId>,
    /// ★★★★★ §5.12 — which arm of the framebuffer-leaf **join** this boot is running, from
    /// the composition root's own reading of [`FB_JOIN_ENV`].
    ///
    /// ★ Like `guest_ram_backing` above, it is the arming flag for the whole path:
    /// [`FbJoinArm::Off`] ⇒ this port materializes nothing and prints not one `GR-FB-JOIN`
    /// line, which is what keeps the arming control comparable to the armed run line for
    /// line. ⊘ Read only by the `host-isolates` arm; the value is still CARRIED so the two
    /// builds differ in what they can do rather than in what they can say.
    #[cfg_attr(not(feature = "host-isolates"), allow(dead_code))]
    fb_join: FbJoinArm,
    /// ★★★★★ §5.12 — the route from a backing token to a descriptor this process can `mmap`
    /// ([`kayfabe_isolate_host::isolate::ExportDirectory`]).
    ///
    /// # ⊘ Why the shell needs one at all
    ///
    /// The core holds `Box<dyn Isolate>` and must: it has no business knowing an isolate is a
    /// process that owns file descriptors. So a shell that has just been told *"leaf X is
    /// joined, its backing is token T"* has **no path** from the core back to the registry T
    /// indexes. This handle is that path, cloned off the factory at the composition root
    /// before the factory is boxed into the object model.
    ///
    /// `None` in an archive whose isolate plane is stillborn — there is no isolate to hand up
    /// a descriptor, and a directory would be an empty table pretending otherwise.
    ///
    /// ⊘ **The `allow` is the ALIAS's cost, paid where it falls.** Without `host-isolates`
    /// this type is `()`, every reader of the field is `#[cfg]`-ed out with the join chain,
    /// and `dead_code` is *correct*: the shipped archive genuinely carries an inert unit here.
    /// It is scoped to that configuration only, so a real dead field in the feature-on build
    /// is still a `-D warnings` failure. ⚠ CI's clippy job is
    /// `cargo clippy --workspace --all-targets`, which does **not** enable the feature
    /// (`fb_leaf_crossing.md` §0.2), so the feature-off arm is the one CI judges.
    #[cfg_attr(not(feature = "host-isolates"), allow(dead_code))]
    exports: FbExportDir,
    /// ★★★★★ **Which arm of the GR doorbell route this boot runs**, from the composition
    /// root's own reading of [`GR_ROUTE_ENV`] — see that constant for why it is armed at all
    /// and `docs/design/gr_doorbell_passthrough.md` for what each arm can and cannot prove.
    ///
    /// ★ Like `fb_join` above it is read ONCE, at the root, and carried. ⊘ Never re-read at
    /// a doorbell: an arming flag consulted twice is a run that can change its mind halfway
    /// through a boot, and the two arms of this experiment differ only in a routing
    /// decision — the least visible thing that could drift.
    gr_route: GrRouteArm,
    /// ★★★★★ **LEG 4's arm** — whether the guest-RAM pin is given a second source, the
    /// pushbuffer VAs this channel's own GPFIFO entries name. See [`GUEST_PUSHBUF_ENV`] and
    /// [`SharedDoorbell::pin_pushbuffer_guest_ram`].
    ///
    /// ★ Read ONCE at the composition root and carried, for `gr_route`'s reason exactly.
    /// ⊘ It is a **third** selector rather than a rider on [`GUEST_RING_ENV`], so a boot can
    /// run leg A without leg 4 and the arms of one experiment differ in one variable each —
    /// which `w263`'s harness did not achieve and said so in its own RESULT §3.1.
    guest_pushbuf: GuestPushbufArm,
}

/// ★★★★★ **§5.12 — a joined framebuffer range, as the device crate's port sees it.**
///
/// `kayfabe_device` is pure: it holds no descriptor and performs no `mmap`, so the memory
/// behind a join reaches it as a `dyn kayfabe_device::FbJoined`. This is the one
/// implementation, and it is four lines because that is genuinely all the join is on this
/// side — the guest's framebuffer window reads and writes an `mmap` of the same `memfd` the
/// isolate described to RM.
///
/// ⊘ **No length check of its own.** `MappedRegion` bounds every access against the extent it
/// was mapped with and answers `RawError` outside it, and a second check here would be a
/// second source of truth for one extent. What this adds is the *name* of the refusal, so a
/// store's `FbRefused` carries a sentence rather than a `Debug`.
#[cfg(feature = "host-isolates")]
#[derive(Debug)]
struct MappedFb(kayfabe_linux_raw::MappedRegion);

/// [`MappedFb`]'s one sentence when an access falls outside the mapping.
#[cfg(feature = "host-isolates")]
const JOINED_OUT_OF_EXTENT: &str = "that access falls outside the joined backing's own extent; the mapping bounds it and \
     this port refuses rather than wrapping, because a wrapped framebuffer access would read \
     another part of the same leaf and look like a plausible answer";

#[cfg(feature = "host-isolates")]
impl kayfabe_device::FbJoined for MappedFb {
    fn len(&self) -> u64 {
        self.0.len_bytes()
    }

    fn read(&self, off: u64, buf: &mut [u8]) -> Result<(), &'static str> {
        self.0
            .read_into(kayfabe_linux_raw::HostOffset::new(off), buf)
            .map_err(|_| JOINED_OUT_OF_EXTENT)
    }

    fn write(&mut self, off: u64, bytes: &[u8]) -> Result<(), &'static str> {
        self.0
            .write_from(kayfabe_linux_raw::HostOffset::new(off), bytes)
            .map_err(|_| JOINED_OUT_OF_EXTENT)
    }
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
///
/// ★★★ §16.79 — how many route-refused pushbuffers get dumped per device life.
///
/// Small, and the smallness is the argument: the question ("is this golden-context init or
/// user compute?") is decided by the FIRST submissions on the channel, and 86 rings of one
/// channel would otherwise bury them. ⊘ A guest that rings a refused channel in a loop moves
/// a counter and cannot fill a disk.
const GR_PUSHBUFFER_DUMPS_MAX: u32 = 2;

/// How many method words each dump prints. Enough to carry a `SET_OBJECT`, a context-buffer
/// setup run and a report semaphore; the dump says how many it did not show.
const GR_PUSHBUFFER_METHODS_MAX: usize = 256;

#[derive(Debug, Default)]
struct CeShellState {
    /// The memory plane, once realized. See the type docs.
    vmm: std::sync::Mutex<Option<QemuVmm>>,
    /// Per `(proc, chan)` GPFIFO read cursors.
    cursors:
        std::sync::Mutex<std::collections::BTreeMap<(u32, u32), kayfabe_rt::ceutils::GpCursor>>,
    /// ★★★★ Per `(proc, chan)` **method accumulators** — the engine state the channel's own
    /// pushbuffer built up, kept between doorbells because that is where hardware keeps it.
    ///
    /// ⊘ Keyed identically to [`CeShellState::cursors`] and committed on the same arm, so a
    /// channel's read position and its latched engine state can never be attributed to
    /// different channels or advanced independently. `[measured 2026-08-09, boot
    /// s21_dbf853a_cup2]` a per-doorbell accumulator made every UVM push after the first
    /// decode to `Opaque` — UVM binds its CE class once and fires forever after.
    states:
        std::sync::Mutex<std::collections::BTreeMap<(u32, u32), kayfabe_rt::ceutils::MethodState>>,
    /// ★★★★ **§16.65 — THE PER-ENGINE DOORBELL CENSUS.** See [`DoorbellCensus`].
    census: std::sync::Mutex<DoorbellCensus>,
    /// ★★★ §16.79 — how many route-refused (GR) pushbuffers have been dumped this device
    /// life. Bounded by [`GR_PUSHBUFFER_DUMPS_MAX`]: `cuCtxCreate` rings one GR channel 86
    /// times and the first submissions are the ones that decide the question.
    gr_dumps: std::sync::Mutex<u32>,
    /// ★★★★★ **§16.81 — how many `Ce` doorbells [`forwarding_plane_owns_ce`]'s system-proc
    /// term actually CHANGED THE ANSWER FOR** this device life.
    ///
    /// ⊘ **Counted and printed, not inferred.** The term is a no-op on both historic arms
    /// (`local` fails the second conjunct, a `Stillborn` plane fails it too), so a boot in
    /// which it never fires and a build in which it does not exist produce identical guest
    /// behaviour. The only thing that separates them is this line — and a rung that read
    /// *"the guest lived"* as evidence for the term would be inferring from an instrument
    /// that never ran, which is the trap this campaign has paid for repeatedly.
    ///
    /// ⇒ Every occurrence prints, carrying its own running index, so the largest index in a
    /// boot log **is** the total and no row is ever elided into looking absent.
    ///
    /// # ★★★★ Why this is an `AtomicU64` and not the `Mutex<u64>` it shipped as
    ///
    /// `[measured 2026-08-11, `b6c5442`]` it shipped as `std::sync::Mutex<u64>` and turned
    /// `tests/tests/unranked_locks.rs` RED — an unranked lock on the vCPU path with no
    /// ruling on whether anything may block beneath it. Reading the call site answered the
    /// question the wrong way: the guard was alive across the `eprintln!` **and** across the
    /// `String` its `pdb` argument builds, so acquiring the process-global stderr
    /// `ReentrantLock`, at least one `write(2)` on it (stderr is unbuffered, so a long line
    /// can be several) and a heap allocation all ran beneath it. ⊘ Exactly ONE allocation,
    /// not two — `map_or_else` evaluates a single arm, and the difference is the sort of
    /// thing this file's own comments are held to. `l1_concurrency.md` §3.3 R1 is *"no
    /// blocking call under ANY lock, ever — no
    /// potentially-blocking syscall at all"*, and `CeShellState::gr_dumps` two fields up is
    /// the same counter written the safe way, with its classification in
    /// `unranked_locks.rs` saying so in as many words: it *"takes it inside its own block
    /// and DROPS it before the dump does anything — every `eprintln!` … outside that
    /// scope"*.
    ///
    /// ⊘ **Narrowing the critical section would only have made it legal.** A counter has no
    /// invariant to protect: there is nothing a reader could observe torn, and the one
    /// property this field's docs claim — *"the largest index in a boot log is the total"* —
    /// is what `fetch_add` returns by construction. So the lock is deleted rather than
    /// classified, which is the difference between removing the hazard and annotating it.
    /// ⚠ This is **not** a way of silencing the gate: the gate's subject is a *critical
    /// section*, and an atomic increment has no *beneath*.
    ///
    /// ## ⊘ `l1_concurrency.md` §4.2 says *"no atomics"* — and this tree measurably means
    /// *"no hand-rolled lock-free SYNCHRONISATION"*
    ///
    /// The rule's own reason is *"keeps TSan a meaningful ceiling and makes `loom`
    /// unnecessary"*, and neither is engaged by a `Relaxed` statistics counter: TSan models
    /// atomics exactly (it flags the *non*-atomic race), and `loom` exists for orderings a
    /// counter that publishes nothing does not have. ★ The practice already says so, in the
    /// two places most relevant to this one: `kayfabe-device/src/plane.rs`'s
    /// `PlaneCounters` is an entire struct of `AtomicU64` on the vCPU MMIO path, kept out of
    /// `PlaneState` *"so that `RegPlane::counters` never blocks behind a doorbell being
    /// serviced"* — the identical argument, made first — and this very file's
    /// [`ENGINE_FWD_SEEN`]/[`ENGINE_FWD_OK`] are `AtomicU64` counted `Relaxed`. ⇒ A
    /// `Mutex<u64>` counter was the anomaly here, not the atomic that replaces it.
    sysproc_kept: std::sync::atomic::AtomicU64,
    /// ★★★★★ **THE COMPLETION OBSERVER'S WATCH LIST** — the completions the guest DECLARED,
    /// and what has been read at their addresses.
    ///
    /// Written by the vCPU thread (declare, a map insert) and read by the reactor thread
    /// (observe). ⊘ It is an `Arc` because those are two threads, and a leaf mutex because
    /// nothing beneath it may block — see `kayfabe_rt::completion_watch`'s module docs for
    /// the split and for the three things the observer is structurally unable to do.
    watch: std::sync::Arc<kayfabe_rt::completion_watch::WatchList>,
    /// ★★★★★ The observer's reactor thread, once started. See [`Regs::attach_ram`].
    #[cfg(feature = "host-isolates")]
    observer: std::sync::Mutex<Option<ObserverThread>>,
}

/// ★★★★★ **THE FIRST PRODUCTION `Reactor` IN THIS TREE** — the completion observer's loop.
///
/// `docs/design/completion_wait_architecture.md` §0.1 measured `Reactor::new`,
/// `Executor::new`, `register_source`, `arm_counter`, `arm_channel`, `deliver_completions`
/// and `poll_completions` at **zero production call sites**, and §7 R3 states the
/// consequence: *"the owner's suspected shape (one thread, N per-op registrations) is the
/// right one **and it already exists**. The work is a composition root, not a design."*
/// This is that composition root, built for the one job that needs it.
///
/// # ⊘ What runs on which thread, and why that is the whole point
///
/// The vCPU thread **declares** (a decode, one address resolution, a map insert — see
/// `SharedDoorbell::declare_gr_completion`) and returns. This thread **observes**: it blocks
/// in a real `epoll_wait` on the reactor's own control descriptor, and between waits it
/// reads the declared addresses out of guest RAM through its own `QemuVmm` handle.
///
/// ⚠ **Nine blocking sites on the guest-facing path stay nine.** Nothing here is ever
/// entered from a vCPU, and nothing a vCPU calls waits on it.
///
/// # ⚠ Reading guest RAM off the vCPU thread — the argument, stated rather than assumed
///
/// `QemuVmm` is `Clone + Send + Sync` (`kayfabe_vmm_qemu`'s own `assert_send_sync!`), holds
/// only an `Arc<Plane>` with leaf mutexes, contains no raw pointer, and calls no `bql_lock`
/// — the adapter's crate docs state there is not one call to it in the whole crate. The C
/// side is a `memcpy` off `memory_region_get_ram_ptr` guarded by `memory_region_is_ram`
/// (`qemu/hw/misc/nvkvm/nvkvm.c:1159`), chosen over `address_space_rw` precisely so it takes
/// no global lock. The copy itself runs **inside** the plane's `view` mutex.
///
/// ⊘ What that argument does NOT cover, and what the shutdown ordering exists for: the
/// foreign region's liveness rests on a `memory_region_ref` taken in topology callbacks that
/// do arrive under the BQL. So this thread is stopped **and joined** in
/// [`Regs::detach_ram`], before the handle is dropped — a reader still running against a
/// machine that has released its slots is the one hazard here, and it is closed by ordering
/// rather than by hope.
/// How long the reactor blocks in one `epoll_wait` before sweeping the watch list anyway.
///
/// ⊘ **Not a busy-poll and not a spin.** The loop is woken by a real descriptor whenever a
/// completion is declared; this bound exists only so a DEADLINE can be reported by a thread
/// nobody is going to poke again. `kayfabe-linux-raw` has no `timerfd` — deliberately, and
/// its own module docs say the day it becomes real is *"when something outside a test has to
/// be woken by a deadline nobody is waiting on"*. ⚠ That day is this loop; a `timerfd` source
/// is the correct successor and this constant is the stand-in, named as one.
#[cfg(feature = "host-isolates")]
const OBSERVER_TICK_MS: u32 = 250;

/// ★★★★★ **THE OBSERVE HALF.** One thread: block in `epoll_wait`, then read every declared
/// address out of guest RAM and say what is there.
///
/// # ⊘ It is handed a READER and nothing else
///
/// The closure below is `(gpa, &mut [u8; 4]) -> Result<(), String>`. `WatchList::sweep`
/// cannot write, cannot raise, and cannot resolve — those capabilities are not in the type
/// it is given. That is the structural guarantee behind *"the VMM owes the notification, not
/// the write"*: this code is unable to forge a completion even if a later edit wanted it to.
///
/// # ⚠ The one refusal it must never swallow
///
/// A `gpa_read` that fails becomes `Verdict::ReadRefused`, which is a statement about the
/// **instrument**. It is never folded into `NotObserved`, because *"we could not look"* and
/// *"we looked and it was not there"* are the two answers this whole rung exists to keep
/// apart.
#[cfg(feature = "host-isolates")]
fn observer_loop(
    reactor: &mut kayfabe_shell::Reactor,
    watch: &std::sync::Arc<kayfabe_rt::completion_watch::WatchList>,
    stop: &std::sync::atomic::AtomicBool,
    mut vmm: kayfabe_vmm_qemu::QemuVmm,
) {
    use kayfabe_vmm::Vmm as _;
    while !stop.load(std::sync::atomic::Ordering::Acquire) {
        // ⊘ ONE wait per iteration, so the sweep below runs between every pair of waits.
        // `run_with` returns `Ok(())` when the budget is spent OR when shutdown was
        // requested; the two are told apart by asking the reactor, not by the return value.
        let outcome = reactor.run_with(kayfabe_linux_raw::PollTimeout::Millis(OBSERVER_TICK_MS), 1);
        let verdicts = watch.sweep(std::time::Instant::now(), &mut |gpa, buf| {
            vmm.gpa_read(gpa, buf).map_err(|e| format!("{e:?}"))
        });
        for v in &verdicts {
            eprintln!("kayfabe: {}", v.line());
        }
        match outcome {
            Ok(()) => {}
            // ★ The F1 refusal is LOUD and stops the loop rather than spinning. It cannot
            // fire vacuously: it means a ready token produced no work 16 waits running.
            Err(fault) => {
                eprintln!(
                    "kayfabe: COMPLETION-OBSERVER ⊘ REACTOR FAULT {fault:?} — the loop \
                     STOPPED. Every later COMPLETION-WATCH line is absent by construction."
                );
                return;
            }
        }
    }
}

#[cfg(feature = "host-isolates")]
#[derive(Debug)]
struct ObserverThread {
    handle: kayfabe_shell::ReactorHandle,
    /// The counter source the vCPU pokes when it declares something new. ⊘ A real armed
    /// registration, so the loop is a reactor and not a sleep: `arm_counter` had zero
    /// production callers before this.
    poke: std::sync::Arc<kayfabe_linux_raw::Notifier>,
    /// ⊘ OURS, not the reactor's. `Reactor::run_with` answers `Ok(())` both for *"the wait
    /// budget is spent"* and for *"shutdown was requested"*, and this loop must tell those
    /// apart to know whether to sweep again. Reading the reactor's own flag is not offered,
    /// and inventing a second meaning for its return value would be a guess.
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

/// ★★★★ **§16.65 — how the arriving doorbells PARTITION by the engine of the channel they
/// routed to.** The instrument this rung's routing change is read against.
///
/// # ⊘ Why a census and not the sixteen logged lines
///
/// `[measured 2026-08-10, boots s49/s50]` the doorbell population was `448 arrived / 354
/// served / 94 refused`, and the only per-channel evidence beside it was **16 logged
/// lines** — a bounded sample, capped by the log's own slot count. A sample can say *"a GR
/// channel was refused"*; it cannot say *"the refused population **is** the GR population"*,
/// and those are the two outcomes this rung has to tell apart. Without the partition, the
/// refutation *"`EngineKind` does not partition doorbell traffic"* and the refutation *"the
/// engine refinement never reached UVM's channels"* produce the **same** count and are
/// indistinguishable.
///
/// ⊘ Tallied where the routing decision is made, from the **same** `CeChannelFacts` the
/// decision reads — never re-resolved. §16.64's own named failure was a probe that
/// re-derived what the serving path had already decided and printed the disagreement as
/// fact.
#[derive(Debug, Clone, Copy, Default)]
struct DoorbellCensus {
    /// Doorbells whose channel resolved, bucketed by `kayfabe_rt::EngineKind::index`.
    by_engine: [u64; kayfabe_rt::ENGINE_KIND_COUNT],
    /// ★ Doorbells whose channel **did not resolve at all** — `ce_channel_facts` refused,
    /// so there is no engine to bucket them under.
    ///
    /// ⊘ A bucket of its own and never folded into `Other`: `Other` is *"an engine the core
    /// routes but does not interpret"*, a fact about a channel we found; this is *"we found
    /// no channel"*. Folding them would let a routing failure read as an exotic engine.
    unrouted: u64,
}

/// ★★★★★ **Route B's framebuffer source** — the emulated framebuffer, answered for the
/// forwarding path's vidmem ring reads.
///
/// # ⊘ Weak, and the `false`/`None` on a dead plane is deliberate
///
/// The plane owns the doorbell port which owns this, so a strong handle would be a cycle.
/// A plane that is gone answers *"this source cannot serve the range"* (`false`) and
/// *"cannot tell you"* (`None`) — never *"the page was never written"*, which is a positive
/// claim about the guest that a dead plane is in no position to make.
///
/// ⚠ **Consulted with NO ranked core lock held** — `kayfabe_rt::device::forward_ring` fetches
/// in its unlocked phase, because these calls take the plane's rank-0 mutex and `core → plane`
/// is the inversion `check_acquire` refuses (§16.87).
#[derive(Debug)]
struct PlaneFbSource {
    plane: std::sync::Weak<RegPlane>,
}

impl kayfabe_fwd::FbSource for PlaneFbSource {
    fn read(&self, phys: u64, buf: &mut [u8]) -> bool {
        match self.plane.upgrade() {
            Some(p) => kayfabe_mmu::walker::FbRead::read(&mut p.pt_bytes(), phys, buf),
            None => false,
        }
    }

    fn page_written(&self, phys: u64) -> Option<bool> {
        let p = self.plane.upgrade()?;
        // ★★★ RESIDENCY, not bytes. `page_writer` answers `Some` only for a page the store
        // has a first-writer record for; `None` from the store means nothing ever wrote it.
        // ⊘ The store CAN answer, so this returns `Some(..)` either way — the `None` arm
        // above is reserved for "there is no store to ask", which is a different fact.
        Some(kayfabe_mmu::walker::FbRead::page_writer(&p.pt_bytes(), phys).is_some())
    }
}

/// ★★★ **The environment variable that registers the source** — route B's only switch.
///
/// # ★★ STATUS 2026-08-11 (w258) — **LIVE, and its premise has NOT lapsed.** Measured.
///
/// ⊘⊘ **A standing summary says route B "exists to remove a refusal whose count is now 0",
/// i.e. that it is kept only for history. `traces/boots/w246/README.md` REFUTES that**, and the
/// refutation is the whole point of the four-corner square that boot ran:
///
/// | corner | `KAYFABE_PT_WITNESS_EXEC` | `KAYFABE_RING_VIDMEM` | `PushbufferAperture` |
/// |---|---|---|---|
/// | A / B | off | off / **on** | 0 (unreachable — `RING-VA-UNBOUND` 8) |
/// | **C** | **on** | off | **8** |
/// | **D** | **on** | **on** | **0** |
///
/// ⇒ **the count is 0 BECAUSE route B is on, not despite it.** C vs D is one variable and it is
/// this flag: the refusal it removes reads **8** with it off and **0** with it on. Removing the
/// code would restore the 8. ★ The zero is route B's *output*, and reading an output as evidence
/// the input is unnecessary is the same shape as `A DIAGNOSTIC gated on the failure`.
///
/// ★ **What DID lapse is the paragraph below's citation, not its content.** It cites `[w237]`
/// and predates the square, so it never records (a) that route B has **fired**, or (b) the
/// precondition that decides whether it can. Both, from `w246` corner D — all 8 `proc 2`
/// doorbells, one line shape, `RING bytes=65536 cursor=0 live=1 spans=0`:
/// - **64 KiB is read out of our own emulated framebuffer and decoded correctly.** `spans=0` is
///   the RIGHT answer, not a failure: the pushbuffer is a semaphore-release-only `LAUNCH_DMA`
///   (`0x14 & LAUNCH_TRANSFER_MASK == LAUNCH_TRANSFER_NONE`, `kayfabe-abi/src/submit.rs:2042`,
///   `ogkm-580 clc7b5.h:86`) — a launch that moves **no bytes**. There is no copy to forward.
/// - ⊘ **Route B is UNREACHABLE unless `KAYFABE_PT_WITNESS_EXEC` is armed.** With the witness
///   off, `plan_gpfifo_ring` returns `RingVaUnbound` at `kayfabe-fwd/src/lib.rs:4258` — *before*
///   `VidmemRoute` is computed (`:4277`). `w245` measured route B alone changing **nothing** and
///   concluded "route B is unreachable"; `w246` scoped that within the hour to a **configuration,
///   not the code**. ⇒ Never measure this flag with the witness disarmed.
///
/// ⚠ **`CE-SUBMIT` is 0 in all four corners and nothing executed.** Route B enumerates a ring;
/// it does not submit work. No line above may be read as the first forwarded work.
///
/// ⊘ Unset ⇒ no source ⇒ `kayfabe_fwd::VidmemRoute::Refuse`, byte-identical to the tree
/// before route B existed. `[w237]` This is a **MEASUREMENT** switch: the owner's 2026-08-07
/// ruling scopes itself to kernel-originated copy-engine work and these doorbells are user
/// `proc 2`, so what may happen *after* the ring is enumerated is not settled here.
const RING_VIDMEM_ENV: &str = "KAYFABE_RING_VIDMEM";

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

/// ★★★ How many attribute→decode rounds one doorbell will run — see
/// [`SharedDoorbell::decode_cpu_pt_writes`].
///
/// # Why more than one, and why a fixed few
///
/// `Spine::pt_page_owner` knows a page-table page only once something published it: **roots**
/// by declaration, everything deeper only after a decode learned it. `[measured 2026-08-10,
/// boot `w208_797a6bc_real`]` the walling ring's tree is five levels of pages the guest's CPU
/// wrote — `L0 #39, L1 #40, L2 #41, L3 #67, L5 #68` — so at the first drain **one** of the
/// five is attributable and the other four are *"not yet"*. Each round decodes what it could
/// attribute, which publishes that subtree's pages, which makes the next round's leftovers
/// attributable. A GA10x tree is at most 6 levels; 8 is that with slack, and the loop also
/// stops the moment a round attributes nothing.
///
/// ⊘ Not "until fixpoint": an unbounded loop over a set the guest writes is a guest-driven
/// stall on the vCPU that took the trap.
const PT_DECODE_ROUNDS: usize = 8;

/// What [`SharedDoorbell::decode_cpu_pt_writes`]'s rounds added up to, for one line.
///
/// ⊘ Every field is carried separately and none is folded into another. `unwitnessed`,
/// `unreachable` and `sparse` are the design **working** (a leaf whose page nobody was seen
/// to write must not bind); `faults`, `refusals` and `reach_faults` are three different
/// components failing — the guest's page tables, our address table, and our reachability
/// shadow. A reader that cannot tell those five apart debugs the wrong one.
#[derive(Debug, Default)]
struct PtDecodeTally {
    bound: usize,
    unchanged: usize,
    repointed: usize,
    unbound: usize,
    learned: usize,
    meta_refused: usize,
    published: usize,
    publish_refused: usize,
    unwitnessed: usize,
    unreachable: usize,
    sparse: usize,
    dropped: usize,
    refusals: usize,
    faults: usize,
    reach_faults: usize,
    retired: usize,
    pass_vas_gone: usize,
    /// The first fault of any kind, whole — see the assignment site for why a count is not
    /// enough.
    first_fault: Option<String>,
}

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
        // ★★★★ §16.64 — ⊘ **THIS COMMENT ASSERTED THE OPPOSITE OF THE CODE**, and it is the
        // first sentence a reader of the doorbell path meets.
        //
        // What stood here: *"`try_ce_submission` answers `None` unless the routed channel has
        // no `Vas` (`vas_pdb == None`) … So no channel changes hands."* The gate it describes
        // is [`SharedDoorbell::try_ce_submission`]'s
        // `facts.vas_pdb.is_some() && !self.local_ce_is_the_only_executor`, and the second
        // conjunct is missing from that sentence entirely.
        //
        // ⇒ On the **shipping** configuration the sentence is exactly backwards.
        // `local_ce_is_the_only_executor` is `isolate_plane == IsolatePlane::Stillborn`,
        // which is the DEFAULT, so `!self.local_ce_is_the_only_executor` is `false` and the
        // early return **never fires**. This arm therefore claims **every** doorbell whose
        // channel has a `vaspace` and a `ring_va` — `Ce` and `GrCompute` alike — and it
        // answers `Some(..)` **terminally**: a refusal here returns at the `if` below and
        // the forwarding path underneath is never reached. Channels very much do change
        // hands.
        //
        // ★ That is not a defect being introduced here and this rung does not change it (the
        // executor partition is `w202`'s increment). It is recorded because the *count* it
        // governs is what any doorbell measurement is read against: `[measured 2026-08-10,
        // boots `s45_748a207_tsgsched` → `s49_57bd756_declroot2`]` the refused population
        // moved `187 → 94` once the root resolved, so a majority of those doorbells were
        // real CE work blocked behind a false refusal — and the remainder are refused by a
        // name that is true. ⊘ The field's own doc at [`SharedDoorbell`] is correct; only
        // this sentence was stale, and being first is what made it costly.
        let mut seen: Option<kayfabe_rt::device::CeChannelFacts> = None;
        if let Some(report) = self.try_ce_submission(token, &mut seen) {
            return report;
        }
        // ★★★★ **§16.71 — THE OTHER PROJECTION OF THE RING, printed on the doorbell that
        // is about to be forwarded**, so the two resolvers' answers appear on the same
        // boot, for the same token, with the identity each one carries.
        //
        // `[measured 2026-08-10, boots `w205_227194f_ctl` / `_real`]` §16.70.6 recorded two
        // ring addresses — `0x120064000` and `0x420064000` — for what it called one token,
        // and stated in as many words that it could not tell *"RM placed it differently"*
        // from *"the two paths are looking at different channels"*. The reason is visible
        // above: this executor resolves the ring through `ce_channel_facts` and the
        // forwarding path resolves it through `kayfabe_fwd::read_gpfifo_ring`, and **no
        // boot has ever run both on one doorbell with either one naming its object**.
        //
        // ⊘ These are NOT two independent measurements and this line does not claim they
        // are: both ultimately read `AllocFacts::gp_fifo_ring` off the node
        // `rmgraph.node_of_resource` returns. What they do not share is the *instant* and
        // the *lock acquisition* — this one completes before `SharedDevice::doorbell`
        // re-routes the same token — so a disagreement between them is a statement about
        // LIFETIME, and an agreement is a statement that the resolver seam is a
        // table-population question and nothing else. That is the discrimination §16.70.6
        // asked for; it is not corroboration and must not be read as any.
        //
        // ⊘ Printed only on the forwarding fall-through, which on the `Stillborn` control
        // plane is reached by no routed doorbell at all (`try_ce_submission` claims every
        // one of them terminally) — so the control's committed census stays byte-identical.
        if let Some(f) = &seen {
            // ★★★★★ **§16.72 — THE PUBLISHED-ROOT DESCENT, ASKED ABOUT THE RING THE
            // FORWARDING PATH IS ABOUT TO FAIL ON, ON THE SAME LINE AS ITS KEY.**
            //
            // §16.71.5 retired §16.70.4's headline (*"the walker finds it every time; the
            // forwarding path misses"*) because its evidence was a serving of the **other
            // channel of the pair**: the control's `sem fin va=0x12006c004` is
            // `0x120064000 + 0x8004`, the `0xc1e00005`-class ring, while the doorbell that
            // walls names the `0xc1e00006`-class ring `0x420064000`. ⇒ **The descent has
            // never been asked about `0x420064000` at all.**
            //
            // This appends `addressing_probe_facts` — the descent already used by the three
            // CE-executor refusal sites — to the line that carries `key=`, `pdb=` and
            // `ring=`. ⊘ **The join is the point.** §16.71.4's whole finding is that a
            // "discrepancy" was an artefact of comparing two lines that shared no key; the
            // fix is not a better number but the owner printed beside it, so this answer is
            // never separable from the channel it is an answer about.
            //
            // ⊘ **It could not be reached before, and not merely because it was unwired.**
            // `addressing_probe` runs on refusals, and `[measured 2026-08-10, boots
            // `w206_8a2280b_ctl`/`_real`]` **neither arm ran it once**: the real arm refused
            // nothing (`3 arrived, 3 served, 0 REFUSED`) and every one of the control's 86
            // refusals is `Route::NotACopyEngineChannel`, raised above the probe's three
            // sites and carrying none. ⚠ And even a refusal would not have printed it here —
            // only the **first** refusal's `why` reaches the census summary.
            //
            // ⊘ **Observationally neutral on the control, by the same argument that already
            // guards this block**: on the `Stillborn` plane `try_ce_submission` claims every
            // routed doorbell terminally and `unrouted=0`, so this fall-through is reached by
            // no doorbell at all and `RING-PROJ 0` stays `RING-PROJ 0`.
            //
            // ⚠ Locks: called exactly where the refusal sites call it — with `ce.vmm`
            // **not** held (that lock is taken below, at `let mut held`) and no cursor lock
            // outstanding. This is a read-only observer; it serves nothing.
            eprintln!(
                "kayfabe: RING-PROJ token={token:#010x} proc={} chan={} vchid={} \
                 key=0x{:x}:0x{:x} engine={} vas={} dec={} pdb={} ring={} entries={} \
                 (projection: ce_channel_facts) DESCENT{}",
                f.proc.0,
                f.chan.0,
                f.vchid,
                f.chan_key.0,
                f.chan_key.1,
                f.engine_name(),
                f.vaspace
                    .map_or_else(|| "NONE".to_string(), |v| format!("0x{v:x}")),
                f.vaspace_declared
                    .map_or_else(|| "NONE".to_string(), |v| format!("0x{v:x}")),
                f.vas_pdb
                    .map_or_else(|| "NONE".to_string(), |p| format!("0x{:x}", p.0)),
                f.ring_va
                    .map_or_else(|| "NONE-DECLARED".to_string(), |v| format!("0x{v:x}")),
                f.ring_entries,
                self.addressing_probe_facts(*f),
            );
        } else {
            // ⊘ The token did not route at all, so there is no second projection to print.
            // ★ Said out loud rather than skipped: a missing line is indistinguishable from
            // an instrument that did not run, and this rung's whole subject is a number
            // that was printed without anything naming what it belonged to.
            eprintln!(
                "kayfabe: RING-PROJ token={token:#010x} UNROUTED — ce_channel_facts \
                 resolved no channel for this token, so this doorbell has ONE projection, \
                 not two"
            );
        }
        // ★★★★★ **§16.74 — G1+G2+G3, RUN BEFORE THE RING IS READ.**
        //
        // The order is the whole point and it is not stylistic: `forward_ring` below reads
        // this channel's GPFIFO ring through `AddressTable::binding_at`, and for four
        // consecutive rungs that lookup has answered `RING-VA-UNBOUND va=0x420064000 →
        // NOTHING FORWARDED`. §16.73's ruling says why — the table is INCOMPLETE, because
        // the transport that published that mapping is the guest's CPU through BAR2 and
        // nothing witnessed it. This is the populate pass for that transport, and it must
        // commit **before** the read that consumes it or it may as well not have run.
        //
        // ⊘ It is NOT a second address plane and it resolves nothing: it latches witnessed
        // pages, decodes them with the walker every other path uses, and forward-populates
        // the one authoritative per-VAS table. Miss is still fault; the table is still
        // never reverse-resolved.
        //
        // ⚠ Printed unconditionally on this path, including `drained=0`. A populate pass
        // that ran and found nothing and a populate pass that did not run are different
        // facts, and only one of them is about the guest.
        //
        // ★★★★★ **§16.82 — and BEFORE it, the transport G1 does not have.** The order is the
        // same argument one comment up: a page witnessed after the pass that would have
        // decoded it is a page that binds a doorbell too late. See
        // [`Self::witness_executor_fb_pages`] for the census that says why this is 96.8 % of
        // the pages in this boot, and why the disarmed arm is the control.
        eprintln!(
            "kayfabe: PT-DECODE token={token:#010x}{}{}",
            self.witness_executor_fb_pages(),
            self.decode_cpu_pt_writes()
        );
        // ★★★★★ **§16.82 — WHY the ring's VA is not bound, asked of the VAS that would have
        // to bind it, on the same doorbell and joined by `proc`/`pdb`/`va`.**
        //
        // ⊘ Printed BEFORE the pin and before `forward_ring`, and unconditionally on this
        // path, so the state it reports is the state those two are about to consult — not the
        // state they left behind. ⚠ It runs after the populate pass above deliberately: the
        // question is *"did the pass bind it?"*, and asking before the pass would answer a
        // question nobody has.
        if let Some(f) = &seen {
            if let (Some(pdb), Some(ring_va)) = (f.vas_pdb, f.ring_va) {
                eprintln!(
                    "kayfabe: {}",
                    self.device.vas_bind_census(
                        f.proc,
                        DOORBELL_TARGET_GPU,
                        pdb,
                        kayfabe_rt::GpuVa(ring_va)
                    )
                );
            } else {
                // ⊘ An absence with a name: the census needs both a table to look in and an
                // address to look up, and which one is missing is a different fact.
                eprintln!(
                    "kayfabe: VAS-BIND-CENSUS token={token:#010x} proc={} chan={} \
                     NOT-ASKED pdb={} ring_va={} (no address space and/or no declared ring \
                     — there is nothing to ask this table about)",
                    f.proc.0,
                    f.chan.0,
                    f.vas_pdb.map_or("NONE".into(), |p| format!("0x{:x}", p.0)),
                    f.ring_va.map_or("NONE".into(), |v| format!("0x{v:x}")),
                );
            }
        }
        // ★★★★★ **§5.8 — THE FIRST GUEST BYTE.** Ordered HERE, and the order is the whole
        // argument: the pin resolves the ring's VA through the address table, and the
        // table only carries that binding because the populate pass one line up has just
        // committed it. Before it, this would read `AddressFault::Miss` on every doorbell
        // and the miss would be an artefact of ordering rather than a fact about the guest.
        //
        // ⊘ Silent — not merely quiet — when the crossing is not armed. See
        // `SharedDoorbell::guest_ram_backing`.
        if let Some(line) = self.pin_ring_guest_ram(token, seen.as_ref()) {
            eprintln!("kayfabe: {line}");
        }
        // ★★★★★ **LEG 4 — AND IT MUST BE THE LINE ABOVE `doorbell`, not below it.**
        //
        // The order is leg A1's argument one plane over: `SharedDevice::doorbell` below is
        // what rings the host channel, and a mapping installed after the ring has been rung
        // is a mapping installed after the engine has already faulted for it. This is the
        // populate pass for the pushbuffer plane and it must commit **before** the consumer.
        //
        // ⊘ It returns a `String` and gates nothing. Whether the doorbell below is forwarded
        // does not depend on whether one entry read, which is the opacity pin's property
        // (`tests/tests/doorbell_is_forwarded_without_reading_the_ring.rs`) and is preserved
        // here by construction rather than by care: there is no branch to preserve it in.
        //
        // ⊘ Silent — not merely quiet — on the disarmed arm, so the control's log stays
        // byte-comparable. See `SharedDoorbell::guest_pushbuf`.
        if let Some(line) = self.pin_pushbuffer_guest_ram(token, seen.as_ref()) {
            eprintln!("kayfabe: {line}");
        }
        // ★★★ **The forwarding path is now GIVEN THE RING.** Until it was, `Served` here
        // meant, in `execution_plane_increments.md` §15.5's own words, *"we rang a doorbell
        // on a host channel into which the guest's methods were never copied"* — and the
        // only function in the tree that observes a real host completion
        // (`HostRmBackend::await_semaphore`) was reachable from no guest action in any
        // build. `SharedDevice::doorbell` reads this channel's own GPFIFO ring through the
        // port below and forwards the copy-engine work it carries.
        //
        // ⚠⚠ **THE LOCK THIS WIDENS, stated rather than left to be found.** The *direction*
        // is the established one — `try_ce_submission` above already holds this same
        // unranked mutex across `ce_session` and the rank-0 device read. What is new is
        // what may happen beneath it: `SharedDevice::doorbell` checks a worker out of the
        // isolate pool, and a saturated pool **parks** the caller in
        // `PoolGate::wait_for_return`. `CeShellState::vmm` is a bare `std::sync::Mutex`
        // that nobody ranked, so `assert_lock_free`'s witness cannot see it and passes
        // **vacuously** while it is held (`kayfabe-util/src/lockwitness.rs:9-21`), and
        // `tests/tests/unranked_locks.rs:76` scopes its scanner to
        // `["kayfabe-device", "kayfabe-rt"]` — this crate is on the vCPU path and is not in
        // that list, so no gate will say so either
        // (`docs/design/completion_wait_architecture.md` §2.2).
        //
        // ⊘ `[NOT MEASURED]` — it is bounded in practice today for reasons outside this
        // file: every MMIO write arrives with the BQL held, so a second doorbell cannot
        // begin anyway, and this path is only reachable with `KAYFABE_ISOLATES` set. That
        // is an argument for why it has not bitten, **not** an argument that it cannot.
        // The fix, when the wait plane is built, is to read the ring under this mutex and
        // release it before any host verb — the same plan/execute/commit split R1 forces
        // one layer down.
        //
        // ⊘ `None` when the memory plane is not attached — between realize and
        // `attach_ram` there is no guest memory to read a ring out of, and refusing the
        // doorbell for that would refuse traffic that is served today.
        let mut held = self.ce.vmm.lock().unwrap_or_else(|e| e.into_inner());
        let port = held.as_mut().map(|v| v as &mut dyn kayfabe_vmm::Vmm);
        let rung = self.device.doorbell(port, DOORBELL_TARGET_GPU, token, &[]);
        drop(held);
        match rung {
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

/// How many bytes of the refused submission's **pushbuffer** the header census reads.
///
/// ⊘ A bound, and it is printed beside what it produced (`pbm[Nw of MB]`) so a truncation
/// is visible rather than silent — `RING_PAGE_DUMPS`' own lesson. 128 bytes covers both
/// shapes this path sees: RM's CeUtils block is `CE_METHOD_SIZE_PER_BLOCK` = `0x64`, and
/// `[measured 2026-08-09, boot s20_25295aa_cup2]` the refused UVM push was `0x68`.
const PROBE_PUSH_BYTES: usize = 128;

/// How many framed methods the census prints. Enough for a whole CeUtils block (7 runs) or
/// a UVM `channel_init` push, and a hard stop against a hostile pushbuffer of tiny runs.
const PROBE_PUSH_METHODS: usize = 12;

/// ★★★★ §16.64 — **which of the two sources answered for a channel's page-directory root**,
/// as a value both the executor and the report read.
///
/// ⊘ The provenance is a variant rather than a `bool` or a bare `Option<VasRoot>` because
/// the two sources know **different things** and a reader must be able to tell them apart:
/// a published root carries the guest's own `pageShift` and VA window, while a declared one
/// is a base the object model resolved by resource identity with its geometry derived from
/// the installed format. Collapsing them would make a report say "root=…" without saying
/// which question it answered.
#[derive(Debug, Clone, Copy)]
enum DoorbellRoot {
    /// This device's publication table answered — `(hClient, hVASpace)` keyed.
    Published(kayfabe_device::ceresolve::VasRoot),
    /// ★ The **object model's** base for the VA space this channel resolved to, walkable.
    /// The arm that exists because a UVM-managed VA space publishes on a transport the
    /// table above never sees, under a dup handle it could not match anyway.
    Declared(kayfabe_device::ceresolve::VasRoot),
    /// Neither source knows of a root. ⊘ Genuinely nobody — not "we did not look".
    Absent,
    /// A base exists but no walkable root could be derived from it, carrying the base and
    /// the derivation's own refusal so the report never restates this as `NoPublication`.
    Underivable(u64, kayfabe_device::ceresolve::CeResolve),
}

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
    /// ★★★★ **§16.64 — the two root sources, resolved ONCE, for every reader.**
    ///
    /// # ⊘ Why this is a function and not two call sites
    ///
    /// `[measured 2026-08-10, boot `s49_57bd756_declroot2`]` the serving path had already
    /// been taught the second source while [`SharedDoorbell::addressing_probe`] had not, and
    /// the result is this campaign's own named failure printed verbatim: the boot **served
    /// 93 doorbells** through a declared root while the probe beside them said
    /// `root=none rng=NOPUB row=ABSENT-FROM-ROOT-TABLE`. ⇒ *Two projections of one fact,
    /// disagreeing, with the weaker one the only thing a reader sees.* A future rung reading
    /// that line would have concluded the root still does not resolve.
    ///
    /// ⊘ So the order — publication table first, then the object model's own base — lives
    /// here and nowhere else. A probe that re-derived it could drift from the answer it is
    /// describing, which is exactly what it did.
    fn doorbell_root(
        plane: &kayfabe_device::RegPlane,
        client: u32,
        vaspace: u32,
        vas_pdb: Option<u64>,
    ) -> DoorbellRoot {
        if let Some(root) = plane.published_root(client, vaspace) {
            return DoorbellRoot::Published(root);
        }
        // ⊘ `None` here is a channel with genuinely no VA space (route 4's device-default
        // shape) — a real absence, never papered over with a zero.
        let Some(pdb) = vas_pdb else {
            return DoorbellRoot::Absent;
        };
        match plane.root_from_declared_pdb(pdb) {
            Ok(root) => DoorbellRoot::Declared(root),
            Err(why) => DoorbellRoot::Underivable(pdb, why),
        }
    }

    /// ⊘ `seen` is an OUT-parameter, not a convenience. [`SharedDoorbell::ring`] needs the
    /// facts this function already resolved, and §16.64's boot is this campaign's own
    /// instance of what a second `ce_channel_facts` call costs: *"two resolutions of one
    /// fact can disagree, and the weaker one is the only thing a reader sees."* So the
    /// facts are handed **out of the one resolution**, never resolved again.
    /// ★★★★★ §16.79 — print the raw method stream of a route-refused (GR) submission, at
    /// most [`GR_PUSHBUFFER_DUMPS_MAX`] times per device life.
    ///
    /// ⊘ **It decides nothing and names nothing.** See
    /// [`kayfabe_rt::ceutils::dump_submission_methods`] for why the numbers are the answer
    /// and a decode would be this port's opinion about a class it has no codec for.
    ///
    /// ⊘ Every arm that cannot produce a dump SAYS SO with the reason. An absent section
    /// would read as *"the ring was empty"*, which is the one thing it must never be
    /// mistaken for — an empty capture is evidence of nothing.
    fn dump_gr_pushbuffer_once(&self, token: u64, facts: &kayfabe_rt::device::CeChannelFacts) {
        {
            let mut n = self.ce.gr_dumps.lock().unwrap_or_else(|e| e.into_inner());
            if *n >= GR_PUSHBUFFER_DUMPS_MAX {
                return;
            }
            *n += 1;
        }
        let head = format!(
            "kayfabe: GR-PUSHBUFFER token={token:#010x} engine={} ",
            facts.engine_name()
        );
        let (Some(vaspace), Some(ring_va)) = (facts.vaspace, facts.ring_va) else {
            eprintln!(
                "{head}⊘ NO DUMP: the channel declared vaspace={:?} ring_va={:?}",
                facts.vaspace, facts.ring_va
            );
            return;
        };
        let Some(plane) = self.plane.upgrade() else {
            eprintln!("{head}⊘ NO DUMP: the register plane is gone");
            return;
        };
        let root = match SharedDoorbell::doorbell_root(
            &plane,
            facts.client,
            vaspace,
            facts.vas_pdb.map(|p| p.0),
        ) {
            DoorbellRoot::Published(r) | DoorbellRoot::Declared(r) => r,
            DoorbellRoot::Absent => {
                eprintln!("{head}⊘ NO DUMP: this channel has no VA space root at all");
                return;
            }
            DoorbellRoot::Underivable(pdb, why) => {
                eprintln!(
                    "{head}⊘ NO DUMP: root underivable from pdb 0x{pdb:x}: {}",
                    why.kind()
                );
                return;
            }
        };
        let chan = kayfabe_rt::ceutils::CeUtilsChannel {
            client: facts.client,
            vaspace,
            ring_va,
            ring_entries: facts.ring_entries,
        };
        // ⊘ The channel's OWN cursor, read and not written — the dump must not move a
        // submission the port is about to refuse.
        let cursor = *self
            .ce
            .cursors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(facts.proc.0, facts.chan.0))
            .unwrap_or(&kayfabe_rt::ceutils::GpCursor::default());
        let mut held = self.ce.vmm.lock().unwrap_or_else(|e| e.into_inner());
        let Some(vmm) = held.as_mut() else {
            eprintln!("{head}⊘ NO DUMP: the memory plane is not attached");
            return;
        };
        let demand = kayfabe_device::ceresolve::Demand::from_doorbell();
        let out = plane.ce_session_with_root(&root, demand, |ce| {
            self.device.with_pushbuffer(|pb| {
                kayfabe_rt::ceutils::dump_submission_methods(
                    ce,
                    pb,
                    vmm,
                    chan,
                    cursor,
                    GR_PUSHBUFFER_METHODS_MAX,
                )
            })
        });
        match out {
            Ok(dump) => eprintln!("{head}{dump}"),
            Err(refusal) => eprintln!("{head}⊘ NO DUMP: {}", refusal.describe()),
        }
    }

    /// ★★★★★ **DECLARE the completion this route-refused submission asks for.**
    ///
    /// The whole of what the vCPU thread does for the observer: one bounded ring read (the
    /// same one [`SharedDoorbell::dump_gr_pushbuffer_once`] performs, through the same
    /// helper), a decode of the guest's own `SET_REPORT_SEMAPHORE` operand, **one**
    /// resolution of its address, and a map insert. No host verb, no pool checkout, no
    /// blocking call, nothing that can park. The nine blocking sites on this path stay nine.
    ///
    /// ⊘ **Every arm that cannot declare SAYS SO with its reason, once.** An absent line
    /// would read as *"the guest declared no completion"*, which is the one thing it must
    /// never be mistaken for.
    fn declare_gr_completion(&self, token: u64, facts: &kayfabe_rt::device::CeChannelFacts) {
        // ★★★ FIRST, above every gate: "the observer was reached" is a different fact from
        // "the observer declared something", and a single counter cannot separate them.
        self.ce.watch.attempt();
        let head = format!("kayfabe: COMPLETION-DECLARE token={token:#010x} ");
        let say_once = |why: String| {
            let mut n = self.ce.gr_dumps.lock().unwrap_or_else(|e| e.into_inner());
            if *n <= GR_PUSHBUFFER_DUMPS_MAX {
                *n += 1;
                eprintln!("{head}⊘ NOT DECLARED: {why}");
            }
        };
        let (Some(vaspace), Some(ring_va)) = (facts.vaspace, facts.ring_va) else {
            say_once(format!(
                "the channel declared vaspace={:?} ring_va={:?}",
                facts.vaspace, facts.ring_va
            ));
            return;
        };
        let Some(plane) = self.plane.upgrade() else {
            say_once("the register plane is gone".into());
            return;
        };
        let root = match SharedDoorbell::doorbell_root(
            &plane,
            facts.client,
            vaspace,
            facts.vas_pdb.map(|p| p.0),
        ) {
            DoorbellRoot::Published(r) | DoorbellRoot::Declared(r) => r,
            DoorbellRoot::Absent => {
                say_once("this channel has no VA space root at all".into());
                return;
            }
            DoorbellRoot::Underivable(pdb, why) => {
                say_once(format!(
                    "root underivable from pdb 0x{pdb:x}: {}",
                    why.kind()
                ));
                return;
            }
        };
        let chan = kayfabe_rt::ceutils::CeUtilsChannel {
            client: facts.client,
            vaspace,
            ring_va,
            ring_entries: facts.ring_entries,
        };
        // ⊘ The channel's OWN cursor, read and NOT written — this must not move a submission
        // the port is about to refuse.
        let cursor = *self
            .ce
            .cursors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(facts.proc.0, facts.chan.0))
            .unwrap_or(&kayfabe_rt::ceutils::GpCursor::default());
        let out = {
            let mut held = self.ce.vmm.lock().unwrap_or_else(|e| e.into_inner());
            let Some(vmm) = held.as_mut() else {
                drop(held);
                say_once("the memory plane is not attached".into());
                return;
            };
            let demand = kayfabe_device::ceresolve::Demand::from_doorbell();
            plane.ce_session_with_root(&root, demand, |ce| {
                self.device.with_pushbuffer(|pb| {
                    kayfabe_rt::ceutils::observe_declared_completion(ce, pb, vmm, chan, cursor)
                })
            })
            // ⚠ Every lock this arm took — the memory-plane mutex, the plane session and
            // the rank-0 device read — is released HERE, before anything is declared and
            // before anything is printed. The declare below runs lock-free by construction.
        };
        let observed = match out {
            Ok(Some(o)) => o,
            // ⊘ A submission that declares no report semaphore. A FACT about the guest's
            // bytes, not a failure — every CE-only push answers exactly this.
            Ok(None) => return,
            Err(refusal) => {
                say_once(format!(
                    "the submission could not be read: {}",
                    refusal.describe()
                ));
                return;
            }
        };
        let (decl, site) = observed.declared;
        let key = kayfabe_rt::completion_watch::WatchKey {
            proc: facts.proc,
            chan: facts.chan,
            va: decl.va.0,
        };
        let before = self.ce.watch.stats().declared;
        self.ce
            .watch
            .declare(key, decl, site.clone(), std::time::Instant::now());
        if self.ce.watch.stats().declared > before {
            // ★ Printed on the FIRST declaration of each completion only — 86 doorbells on
            // one channel are one completion, and 86 identical lines would bury it.
            // ★★★ POKE THE OBSERVER — a real write to a real armed eventfd, from the vCPU
            // thread, with **no ranked lock held**: the session, the memory-plane mutex and
            // the rank-0 device read were all released above, before the declare. `signal`
            // asserts exactly that (R1), so a future edit that moves this under a lock
            // panics loudly instead of deadlocking quietly.
            self.poke_observer();
            eprintln!(
                "{head}proc={} chan={} engine={} → DECLARED va=0x{:x} payload=0x{:08x} \
                 awaken={} four_words={} op={} subch={} class=0x{:04x} site={site:?} \
                 (⊘ the observer WATCHES this address; it will never write it)",
                facts.proc.0,
                facts.chan.0,
                facts.engine_name(),
                decl.va.0,
                decl.payload,
                u8::from(decl.awaken),
                u8::from(decl.four_words),
                decl.operation,
                decl.subch,
                decl.class_id,
            );
            // ★★★★★ **THE ADDRESS CENSUS — S1's boundary, as a number instead of an
            // argument.** `docs/design/gr_execution_boundary.md`.
            //
            // The completion observer proved ONE address binds. Opening `S1` means letting
            // the host GR engine dereference **every** address these bytes name, in whatever
            // VA space the host channel is bound to — so the containable question is not
            // *"can we route the doorbell"* but *"does a host VA space exist in which all of
            // these land in THIS guest's memory and nothing else does"*.
            //
            // ⊘ Printed on the same first-declaration branch as the line above, so it is
            // ONE line per GrCompute channel and not one per doorbell, and it costs no
            // second read: the census came off the same `read_submission_methods` and the
            // same walk (`ceutils::ObservedSubmission`).
            //
            // ★ `mme=` is the row that decides the shape of any answer. Guest-authored MME
            // microcode makes the method stream unbounded by inspection — the expander's
            // output IS methods — so a method-level allowlist cannot be sound and the VA
            // space is the only boundary left. §2 of the design doc.
            let bound = observed
                .census
                .iter()
                .filter(|(_, s)| !matches!(s, kayfabe_rt::completion_watch::Site::Unresolved(_)))
                .count();
            eprintln!(
                "kayfabe: GR-ADDRESS-CENSUS proc={} chan={} class=0x{:04x} operands={} \
                 bound={} unbound={} mme_dwords={} ⊘ a census of what the host GR engine \
                 WOULD dereference; nothing here is executed and nothing here is permission",
                facts.proc.0,
                facts.chan.0,
                decl.class_id,
                observed.census.len(),
                bound,
                observed.census.len() - bound,
                observed.mme_dwords,
            );
            for (op, s) in &observed.census {
                eprintln!(
                    "      {:<40} m=0x{:04x} sub={} va=0x{:x} → {s:?}",
                    op.name, op.method, op.subch, op.va.0
                );
            }
            // ★★★★★ **THE SECOND CROSSING, driven off the census that measured it.**
            self.back_census_framebuffer_leaves(facts, &observed.census);
        }
    }

    /// ★★★★★ **§5.12 — JOIN every framebuffer leaf this census named, so the leaf the
    /// guest reads and the leaf the engine reads are ONE memory.**
    ///
    /// `fb_cpu_view.md` §4. ⊘ **This REPLACES `w228`'s `back_census_framebuffer_leaves`**,
    /// which backed the same leaves with real host **vidmem and no CPU view** — the engine
    /// reading the card object and the guest reading the shell's `SparseFb`, silently, in
    /// both directions. The two are not layers and not fallbacks for each other: a leaf
    /// served by both would have two host objects at one guest VA. The vidmem chain is still
    /// expressible ([`kayfabe_rt::FbLeafBacking::Vidmem`]) and has **no caller**.
    ///
    /// # ★★★ The order, and why it is what makes the whole thing safe
    ///
    /// 1. **Join** — an isolate round trip, with **no plane lock held**: mint a fabricated
    ///    backing, map it there, describe it to RM, place it at the leaf's VA.
    /// 2. **Adopt + map** — `dup` the descriptor out of this isolate's export registry and
    ///    `mmap` it here. This is the guest's view.
    /// 3. **Establish + install** — one hold of the plane lock
    ///    ([`kayfabe_device::RegPlane::join_fb`]): copy what the guest has ALREADY written
    ///    into the backing, then make the range live.
    ///
    /// ★ Step 3 is what answers the owner's *"mapping after execution seems racy to me"*.
    /// It is racy — once the engine has written the real object and the guest has written the
    /// fabricated one there is **no correct merge**, only a choice about which writes to
    /// lose. The establishment copy removes the question rather than answering it: after it
    /// there is one memory, so there is never a merge.
    ///
    /// # ⊘ What a green line here still does NOT mean
    ///
    /// - **Nothing executed.** No doorbell is routed and no engine is pointed at anything;
    ///   ⊘ **CORRECTED 2026-08-11** — this used to read *"`Route::NotACopyEngineChannel`
    ///   refuses every `GrCompute` doorbell one function below, exactly as at `w228`"*, and
    ///   that is now true only of the **default** `KAYFABE_GR_ROUTE=refuse` arm. On
    ///   `passthrough` the doorbell IS routed, and this join runs on the way past it — the
    ///   two calls above `return None` are the same two the refusal arm makes, in the same
    ///   order, precisely so the armed arm stays log-comparable to its control.
    ///   ⇒ The claim that survives is the one that was load-bearing: **nothing executed.**
    ///   `gr_doorbell_passthrough.md` §0.3 — the host GR channel's ring and its `GP_PUT` are
    ///   both ours on either arm, so the host engine fetches nothing. **The guest did not
    ///   move.**
    ///
    /// - ⚠ **And the leaf this join reaches is NOT the ring's.** It is driven off
    ///   `observed.census`, the operand census recovered from the pushbuffer decode — the
    ///   addresses the *methods* dereference. `[measured 2026-08-11, w260]` the three leaves
    ///   it joined were `fb_phys 0x400000/0x600000/0x800000`; the GR **ring** lives at
    ///   `fb_phys 0x1000000` (`guest_ring_adoption.md` §4, five boots, two resolvers) and
    ///   nothing presents it here. ⇒ The blocker `b9025b4` named is now a **caller gap, not
    ///   a missing primitive**, and it is the next question on this path.
    /// - **The leaf is host SYSMEM.** A named performance divergence from the C artifact,
    ///   with its reason on [`kayfabe_isolate::FbLeafJoined`]. Card memory is precisely what
    ///   cannot carry a guest-reachable CPU view.
    /// - **`GuestRam` and `Unresolved` rows are untouched by construction** — the `match`
    ///   below has one arm, and they are this pass's standing negative controls.
    #[cfg(feature = "host-isolates")]
    #[allow(clippy::too_many_lines)]
    fn back_census_framebuffer_leaves(
        &self,
        facts: &kayfabe_rt::device::CeChannelFacts,
        census: &[(
            kayfabe_rt::completion_watch::AddressOperand,
            kayfabe_rt::completion_watch::Site,
        )],
    ) {
        use kayfabe_rt::completion_watch::Site;
        let head = format!(
            "kayfabe: GR-FB-JOIN proc={} chan={}",
            facts.proc.0, facts.chan.0
        );
        if !self.fb_join.armed() {
            // ⊘ Silent. The arming control's log must not contain a line the armed run's does
            // not, or the two stop being comparable — which is the whole use of a control.
            // The absence IS the statement, and `KAYFABE_FB_JOIN` is reported in the startup
            // census either way.
            return;
        }
        let Some(pdb) = facts.vas_pdb else {
            eprintln!(
                "{head} → NO PDB (the channel's VA space did not resolve, so there is no \
                 address space to join INTO; ⊘ not a miss — nothing was asked of the host)"
            );
            return;
        };
        let (Some(exports), Some(plane)) = (self.exports.as_ref(), self.plane.upgrade()) else {
            eprintln!(
                "{head} → ⊘ NOT ARMABLE: exports_directory={} plane={} — this build has no \
                 route from a backing token to a descriptor, or the register plane is gone. \
                 ⊘ Nothing was asked of the host and no leaf was touched",
                self.exports.is_some(),
                self.plane.upgrade().is_some(),
            );
            return;
        };
        // ★ leaf VA → what the join answered. Keyed by the leaf and not by the operand,
        // because two operands can fall in ONE leaf and the second must replay rather than
        // ask for a second fixed map at an occupied address — which RM answers `0x51`, a
        // status that ⊘ cannot be told apart from real exhaustion.
        let mut joined: std::collections::BTreeMap<u64, (u64, u64)> =
            std::collections::BTreeMap::new();
        // ★★ Leaves whose chain refused ANYWHERE, kept apart from `joined` because the two
        // answer different questions. `joined` is *"is this leaf host-backed"* and feeds the
        // re-stated census, which must NOT render a refused leaf as `HostBackedFb`. This is
        // *"has this leaf already been attempted"*, and it is what stops a second census
        // operand in the same leaf re-attempting: `release_unadopted_fb_leaf` STAGES the
        // unmap rather than performing it, so the address is still occupied and RM would
        // answer the second FIXED map `0x51` — collision-or-exhaustion, ⊘ indistinguishable.
        let mut refused: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
        // Which leaves reached step 3, so the both-directions probe below has a range it
        // knows the isolate is holding rather than one it hopes it is.
        let mut live: Vec<(u64, u64)> = Vec::new();
        let isolate = kayfabe_isolate::IsolateId::new(facts.proc.0, DOORBELL_TARGET_GPU);
        for (op, site) in census {
            let Site::Framebuffer { leaf, .. } = site else {
                continue;
            };
            if joined.contains_key(&leaf.va) || refused.contains(&leaf.va) {
                continue;
            }
            // ★★★ THE CENSUS SOURCE. The four steps live in `join_one_fb_leaf`, shared
            // with the RING source (`Regs::adopt_pending_channel_rings`) so the ordering
            // that makes a join safe exists exactly once.
            match join_one_fb_leaf(
                &head,
                op.name,
                &self.device,
                &plane,
                exports,
                self.fb_join,
                isolate,
                pdb,
                *leaf,
            ) {
                Some(j) => {
                    joined.insert(leaf.va, (j.host_va, j.memory));
                    if let Some(len) = j.installed {
                        live.push((leaf.phys, len));
                    }
                }
                None => {
                    refused.insert(leaf.va);
                }
            }
        }
        self.probe_joined_leaves(&head, facts, &live, &plane);
        // ★★★ THE RE-STATEMENT. Same operands, same order, same walk — only the backing
        // column can have changed, and it changed because of the replies printed above.
        eprintln!(
            "kayfabe: GR-ADDRESS-CENSUS (RE-STATED AFTER JOINING) proc={} chan={} \
             joined_leaves={} live_views={} ⊘ still nothing executed and still nothing is \
             permission",
            facts.proc.0,
            facts.chan.0,
            joined.len(),
            live.len(),
        );
        for (op, site) in census {
            let restated = match site {
                Site::Framebuffer { phys, leaf } => match joined.get(&leaf.va) {
                    Some(&(host_va, memory)) => Site::HostBackedFb {
                        phys: *phys,
                        leaf: *leaf,
                        host_va,
                        memory,
                    },
                    // ⊘ Unchanged, and it MUST be: a leaf whose join refused is not backed,
                    // and rendering it as anything else would make the refusal unreadable.
                    None => site.clone(),
                },
                other => other.clone(),
            };
            eprintln!(
                "      {:<40} m=0x{:04x} sub={} va=0x{:x} → {restated:?}",
                op.name, op.method, op.subch, op.va.0
            );
        }
    }

    /// ★★★★★ **BOTH DIRECTIONS, over a leaf the census actually named** — the measurement
    /// this rung exists to produce, and the arm the negative control is watched to fail.
    ///
    /// # ★★★ Which line do I expect the control to execute?
    ///
    /// `kayfabe_linux_raw::Backing::PrivateAnonymous`'s arm of the `mmap` argument
    /// computation (`crates/kayfabe-linux-raw/src/mapping_unsafe.rs:344-347`), yielding
    /// `MAP_PRIVATE|MAP_ANONYMOUS` where the armed run yields `MAP_SHARED` — **one property**,
    /// with the identical isolate chain, the identical establishment copy and the identical
    /// probe either side of it.
    ///
    /// ★★ And its fail arm is not "zeros". Direction 2 reads back **direction 1's own
    /// pattern**, still sitting in the private pages this run wrote it into, because the
    /// isolate's poke went to the memfd and never reached them. A control that merely
    /// returned zeros would be consistent with a mapping that was never written at all; this
    /// one demonstrates both views are live, hold different bytes, and are read by the same
    /// loop. (`fb_cpu_view.md` §3.2 measured exactly this shape on real hardware.)
    ///
    /// # ⊘ Why the pattern is per word
    ///
    /// Word *i* is `base + i`. A read that returned a zero fill, a truncated length or a
    /// different buffer's bytes cannot match — whereas a whole-buffer compare against one
    /// repeated word passes on any single correct word.
    #[cfg(feature = "host-isolates")]
    fn probe_joined_leaves(
        &self,
        head: &str,
        facts: &kayfabe_rt::device::CeChannelFacts,
        live: &[(u64, u64)],
        plane: &kayfabe_device::RegPlane,
    ) {
        /// How many bytes of a leaf the probe exercises.
        ///
        /// ⊘ Not the whole 2 MiB leaf: this runs inside a doorbell, and the reply travels in
        /// one frame. 4 KiB is a page — enough that a truncation, a misaddressing or a
        /// wrong-buffer answer cannot match, and small enough that the instrument cannot
        /// become the thing that stalls the plane.
        const PROBE: usize = 4096;
        let Some(&(phys, _)) = live.first() else {
            eprintln!(
                "{head} ⊘ NO PROBE: no leaf reached a live view this doorbell, so there is \
                 nothing to ask about. ⊘ That is the absence of a measurement, NOT a \
                 measurement of absence"
            );
            return;
        };
        // ⊘ Derived from what THIS boot produced — the leaf's own framebuffer address — so
        // the patterns differ run to run and a stale buffer cannot masquerade as a match.
        let g2h = (phys as u32) ^ 0x5a5a_5a5b;
        let h2g = !g2h;
        let image = |base: u32| -> Vec<u8> {
            let mut v = vec![0u8; PROBE];
            for (i, w) in v.chunks_exact_mut(4).enumerate() {
                w.copy_from_slice(&base.wrapping_add(i as u32).to_le_bytes());
            }
            v
        };
        let first_mismatch = |want: &[u8], got: &[u8]| -> Option<(usize, u32, u32)> {
            (0..PROBE / 4).find_map(|i| {
                let w = u32::from_le_bytes(want[4 * i..4 * i + 4].try_into().unwrap_or_default());
                let g = u32::from_le_bytes(got[4 * i..4 * i + 4].try_into().unwrap_or_default());
                (w != g).then_some((i, g, w))
            })
        };

        // ---- DIRECTION 1: guest view → isolate view. Written through the register plane's
        // own framebuffer store, i.e. the exact path a guest PRAMIN/BAR write takes.
        let want1 = image(g2h);
        if let Err(e) = plane.fb_poke(phys, &want1) {
            eprintln!("{head} ⊘ PROBE ABORTED: the guest-side write refused `{e}`");
            return;
        }
        let mut got1 = vec![0u8; PROBE];
        // ★ ONE round trip carries both directions: the read is of what direction 1 wrote,
        // and the poke is direction 2's stimulus. See `RmBackend::fb_join_peek`.
        let covered = match self.device.fb_join_peek(
            facts.proc,
            DOORBELL_TARGET_GPU,
            phys,
            &mut got1,
            Some(h2g),
        ) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{head} ⊘ PROBE ABORTED: the isolate refused the peek `{e:?}`");
                return;
            }
        };
        if !covered {
            eprintln!(
                "{head} ⚠ PROBE MISS: the isolate holds NO joined range covering \
                 fb_phys=0x{phys:x}. ⊘ That is not zeros and not a mismatch — it is the \
                 isolate saying it never joined this leaf, which contradicts the line above"
            );
            return;
        }
        match first_mismatch(&want1, &got1) {
            None => eprintln!(
                "{head} ★ DIRECTION 1 (guest view → isolate view) fb_phys=0x{phys:x} \
                 AGREES over {} words: what this device's framebuffer window wrote is what \
                 the isolate's own mapping — the one RM describes to the GPU — holds",
                PROBE / 4
            ),
            Some((i, got, want)) => eprintln!(
                "{head} ⊘ DIRECTION 1 (guest view → isolate view) DISAGREES at word {i} \
                 (got 0x{got:08x}, want 0x{want:08x}) of {}",
                PROBE / 4
            ),
        }

        // ---- DIRECTION 2: isolate view → guest view. The poke above already wrote it.
        let want2 = image(h2g);
        let mut got2 = vec![0u8; PROBE];
        if let Err(e) = plane.fb_peek(phys, &mut got2) {
            eprintln!("{head} ⊘ PROBE ABORTED: the guest-side read refused `{e}`");
            return;
        }
        match first_mismatch(&want2, &got2) {
            None => eprintln!(
                "{head} ★ DIRECTION 2 (isolate view → guest view) fb_phys=0x{phys:x} \
                 AGREES over {} words: what the isolate wrote is what this device's \
                 framebuffer window reads",
                PROBE / 4
            ),
            Some((i, got, want)) => {
                eprintln!(
                    "{head} ⊘ DIRECTION 2 (isolate view → guest view) DISAGREES at word {i} \
                     (got 0x{got:08x}, want 0x{want:08x}) of {}",
                    PROBE / 4
                );
                if got == g2h.wrapping_add(i as u32) {
                    eprintln!(
                        "{head}   ★★ AND THE VALUE READ BACK IS DIRECTION 1'S OWN PATTERN, \
                         not zeros — so BOTH views are live and hold DIFFERENT bytes. That \
                         is the negative control firing exactly as `fb_cpu_view.md` §3.2 \
                         measured it, and zeros alone could not have shown it"
                    );
                }
            }
        }
    }

    /// No isolate plane in this archive, so no second crossing to arm.
    #[cfg(not(feature = "host-isolates"))]
    #[allow(clippy::unused_self)]
    fn back_census_framebuffer_leaves(
        &self,
        _facts: &kayfabe_rt::device::CeChannelFacts,
        _census: &[(
            kayfabe_rt::completion_watch::AddressOperand,
            kayfabe_rt::completion_watch::Site,
        )],
    ) {
    }

    /// ★ Tell the observer's reactor there is something new to look at.
    ///
    /// ⊘ Fire-and-forget by design: a saturated counter (`EAGAIN`) means nobody has drained
    /// this source in 2^64 signals, which is the observer being gone — and the observer
    /// being gone must not turn a doorbell into a failure. The declaration is already in the
    /// watch list either way; the poke only decides whether it is looked at now or at the
    /// next tick.
    #[cfg(feature = "host-isolates")]
    fn poke_observer(&self) {
        if let Some(o) = self
            .ce
            .observer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            let _ = o.poke.signal();
        }
    }

    /// No observer in this archive; see [`Regs::start_completion_observer`].
    #[cfg(not(feature = "host-isolates"))]
    #[allow(clippy::unused_self)]
    fn poke_observer(&self) {}

    fn try_ce_submission(
        &self,
        token: u64,
        seen: &mut Option<kayfabe_rt::device::CeChannelFacts>,
    ) -> Option<kayfabe_device::DoorbellReport> {
        // ★★★★ §16.65 — **the census is taken HERE**, at the top of the one function every
        // doorbell passes through (`ring` calls it unconditionally, first), and from the
        // same `CeChannelFacts` the routing decision below reads. ⊘ Not from a second
        // `ce_channel_facts` call in `ring`: two resolutions of one fact can disagree, and
        // §16.64's boot is this campaign's own instance of a probe printing the
        // disagreement as the only thing a reader sees.
        let facts = match self.device.ce_channel_facts(DOORBELL_TARGET_GPU, token) {
            Ok(facts) => facts,
            Err(_) => {
                // ⊘ Counted, then handed on unchanged: a doorbell whose channel did not
                // resolve is not this executor's, and it never was. The tally exists so
                // that `arrived` minus the buckets is zero by construction and a
                // disappearing doorbell cannot hide in the gap.
                self.ce
                    .census
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .unrouted += 1;
                return None;
            }
        };
        self.ce
            .census
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .by_engine[facts.engine_index()] += 1;
        // ★ Handed out HERE — after the census and before any gate — so a doorbell this
        // function declines on ANY of its arms still leaves its caller the facts. ⊘ A
        // store placed after the routing gate below would be absent on exactly the
        // population `ring` goes on to forward, which is the population the report is for.
        *seen = Some(facts);
        // ★★★★ **§16.65 — THE ROUTING STATEMENT, and it comes BEFORE every other gate.**
        //
        // The gate below asks about the isolate *plane*, never about the *engine*, and on
        // the shipping (`Stillborn`) configuration it never fires at all, so this arm
        // claimed every doorbell with a VA space and a ring regardless of which engine the
        // channel belongs to. `[measured 2026-08-10, boot s51_d502ac6_engroute]` that was
        // **86 doorbells wide**, and the per-engine histogram shipped with this gate is the
        // only thing that could say so: `GrCompute=86 Ce=362`, `86 + 362 = 448`.
        //
        // ⊘ **AND THE EVIDENCE §16.65 CITED FOR IT WAS A DIFFERENT DOORBELL'S.** This
        // comment used to open by naming the executor's first refusal —
        // `SubmissionHasNoLaunch { methods: 3, opaque: 2 }` — as *"a GR pushbuffer decoded
        // by the CE codec"*. Measured false on all three of `s49`/`s50`/`s51`: that
        // refusal prints its own pushbuffer, and it reads `SET_OBJECT →
        // AMPERE_DMA_COPY_B`, i.e. a **CE** push on a **CE** channel at the **CE**
        // executor, exactly where it belonged. `w202` could not move it and did not; it
        // was §16.66's four-word semaphore release. ★ The routing defect was real and the
        // fix is right; the sentence that motivated it was about something else. When a
        // diagnostic carries its own evidence, read that evidence before theorising.
        //
        // ⊘ **Nothing was ever forged** — the pushbuffer codec is class-gated
        // (`kayfabe-chips/src/ga10x.rs`, `kayfabe-arch/src/lib.rs`), so a GR ring decodes to
        // `Opaque` and no CE launch can be synthesised out of it. The defect is *routing*:
        // the doorbell reached an executor that can never serve it, and the name it was
        // refused by described the **pushbuffer's shape** instead of the **routing
        // mistake** — a refusal that is true of the bytes and silent about the cause.
        //
        // ⚠ **What this changes outside the measured configuration, stated rather than
        // discovered later.** With a real forwarding plane (`KAYFABE_ISOLATES` set) a
        // `GrCompute` channel with a `vas_pdb` used to return `None` here and fall through
        // to `SharedDevice::doorbell`. It is now refused by name instead. That is
        // deliberate: §15.5's own words for what that fall-through achieved are *"we rang a
        // doorbell on a host channel into which the guest's methods were never copied"*,
        // and GR forwarding needs a host channel that SHADOWS the guest's plus the
        // `OS_DESCRIPTOR` primitive, neither of which is built
        // (`ce_executor_tree.md:107-126`). A true refusal outranks a forwarded no-op.
        // ⊘ `Ce` is unaffected on both planes — it falls through to exactly today's gate.
        // ★★★★★ **THE GR PASSTHROUGH ROUTE — three answers where there were two.**
        //
        // What stood here was `if route != DoorbellRoute::CpuCe`, which forced `HostGr` and
        // `Unserved` into ONE bucket — the exact distinction `DoorbellRoute` exists to keep
        // (*"GR is the destination the ladder is walking toward … `Unserved` is an engine
        // nobody has designed a path for"*). The decision now goes through
        // `kayfabe_rt::shell_disposition`, which is exhaustive over the route and so cannot
        // silently acquire a fourth engine.
        //
        // ⚠ **THIS RE-OPENS A PATH THAT WAS CLOSED ON EVIDENCE**, and the evidence is the
        // paragraph below, which stands unamended. Read `GR_ROUTE_ENV` and
        // `docs/design/gr_doorbell_passthrough.md` §0.2-§0.3 before reading a boot that ran
        // the armed arm: the host GR channel's ring **and** its `GP_PUT` are both ours, so
        // the host engine fetches nothing on either arm. The armed arm buys the TRANSPORT —
        // the first `ring_doorbell` ever issued for a GR host token — and nothing else.
        //
        // ⊘ The default is `Refuse` and is byte-identical to every boot before this one.
        //
        // ⊘ Computed ONCE into a binding and matched below, never asked twice: §16.64's
        // finding is that two resolutions of one fact can disagree, and this one is read on
        // both sides of a `return`.
        let route = facts.route();
        let disposition = kayfabe_rt::shell_disposition(route, self.gr_route.gr_passthrough());
        if disposition == kayfabe_rt::ShellDisposition::HandToCore {
            // ★★★★★ **PASSTHROUGH, and the ORDER of these three statements is the ruling.**
            //
            // The two calls below are the SAME two the refusal arm makes, in the same order,
            // and they are **DEBUG and OBSERVATION only**: `dump_gr_pushbuffer_once` is
            // bounded and print-only (*"advances no cursor, writes no state"*), and
            // `declare_gr_completion` declares a watch and — on the `KAYFABE_FB_JOIN` armed
            // arm — drives the framebuffer-leaf join off the operand census it recovers.
            //
            // ⚠⚠ **NEITHER MAY GATE THE FORWARD, and neither can**: both return `()`. There
            // is no `?`, no early return and no branch on their outcome between here and the
            // `return None` below. That is the rung brief's requirement — *"ring resolution
            // / pushbuffer reads / method decode are DEBUG: flag-gated, non-fatal, and they
            // must never gate whether the doorbell is forwarded"* — held by the SHAPE of the
            // code rather than by a reader checking. It is the same property
            // `tests/tests/doorbell_is_forwarded_without_reading_the_ring.rs` pins one crate
            // down, and `gr_doorbell_route.rs` pins here.
            //
            // ⊘ They are kept rather than dropped because the armed arm must stay
            // log-comparable to its control: a boot that stopped printing `GR-PUSHBUFFER`
            // and `GR-ADDRESS-CENSUS` the moment the route opened could not be diffed
            // against the refusing arm at all, and the FB join would silently disarm with
            // it.
            self.dump_gr_pushbuffer_once(token, &facts);
            self.declare_gr_completion(token, &facts);
            // ★★★ **HAND IT TO THE CORE.** `None` here is not *"not ours"* as a shrug — it
            // is this port's one vocabulary for *"the core's ring path serves this"*, and it
            // is the same answer the CE arm gives at `forwarding_plane_owns_ce` below.
            // `SharedDoorbell::ring` then calls `SharedDevice::doorbell`, which routes the
            // GUEST token, materializes and schedules the host channel if the engine-object
            // path has not already, and rings the **HOST** token.
            //
            // ⊘ No lock is held across this return: `try_ce_submission` takes the census
            // mutex and releases it, and `dump_gr_pushbuffer_once` takes and drops the
            // memory-plane lock inside itself. The core's ranked locks are acquired after.
            return None;
        }
        if disposition == kayfabe_rt::ShellDisposition::RefuseByRoute {
            // ★★★★★ §16.79 — READ THE PUSHBUFFER BEFORE REFUSING IT.
            //
            // The refusal below is true and stays. But it is refusing by ROUTE, and a route
            // is a fact about the channel's engine, not about what the guest put in the
            // ring. `[measured 2026-08-10, w216]` `cuCtxCreate` rings GrCompute token
            // `0x00000007` 86 times while `cup2` performs **zero kernel launches**, so the
            // traffic is either user compute or golden-context initialisation — two
            // completely different rungs that the channel's class cannot separate.
            //
            // ⊘ BOUNDED to the first few refusals and PRINT-ONLY: it advances no cursor,
            // writes no state, and takes the memory-plane lock only here, before the CE path
            // below takes it. A refusal that returns above this point is unaffected.
            self.dump_gr_pushbuffer_once(token, &facts);
            // ★★★★★ **THE COMPLETION OBSERVER — DECLARE.**
            //
            // The refusal below is right and stays: this executor cannot serve a GR
            // submission and a true refusal outranks a forwarded no-op (§16.80.1). But
            // refusing is not the same as being blind. `[measured 2026-08-10, boot
            // `w218_cb6adcc_grfull`]` the pushbuffer this doorbell carries ends in
            // `SET_REPORT_SEMAPHORE` naming GPU VA `0x2_0440fff0`, payload `1`,
            // `AWAKEN_ENABLE = 0` — so what `cuCtxCreate` is waiting for is **a value
            // appearing at an address**, and that is a thing a VMM can WATCH without
            // serving anything.
            //
            // ⊘ **This declares; it never completes.** The observer has no writer and
            // raises no vector — the payload is a literal immediate in the guest's own
            // bytes, so writing it here without running the work is precisely the
            // credit-shortcut the C artifact named and refused. The verdict is emitted on
            // the reactor thread; see [`Regs::start_completion_observer`].
            //
            // ⊘ That name used to read `Regs::spawn_completion_observer`, which **does not
            // exist and never did** — `[measured 2026-08-11, git grep over the whole repo]`
            // one hit, this comment. A cross-reference nobody can follow is the cheapest
            // possible instance of `a_correct_citation_narrowed_by_the_reading`: it looks
            // like provenance and resolves to nothing.
            //
            // ★★★★★ **AND THIS BRANCH IS THE TREE'S ONE EXISTING INSTANCE OF THE OWNER'S
            // 2026-08-11 TRAP CONTRACT** — `TrapContract::ScheduleAndReturn`. It declares
            // on the vCPU (a decode, one resolution, a map insert), pokes an eventfd, and
            // returns; the work of *looking* happens on the observer thread. ⚠ It is the
            // shape, not the discharge: what is scheduled here is an OBSERVATION, and the
            // emulated arm's actual work still runs inline further down this function
            // (`kayfabe_rt::ceutils::run_submission`, under the FSM mutex and the BQL).
            //
            // ⚠ Runs on EVERY route-refused doorbell, unlike the bounded dump above: a
            // declaration is idempotent (first one wins) and a watch that was declared only
            // for the first two doorbells would be a watch on a sample, not on the channel.
            self.declare_gr_completion(token, &facts);
            return Some(refused(
                token,
                kayfabe_device::FaultTag("Route::NotACopyEngineChannel"),
                format!(
                    "this channel's engine is {} (route {route:?}), so its pushbuffer is \
                     not copy-engine work and the shell's CPU copy-engine executor is the \
                     wrong executor for it; refused by the ROUTING fact rather than \
                     decoded by a codec that can only decline{}",
                    facts.engine_name(),
                    // ★★★ **WHICH refusal this is** — added with the GR route, because
                    // from here on the same tag covers two different situations and a boot
                    // log could not tell them apart: *"GR, and the route is DISARMED on
                    // this run"* is a configuration, while *"NVENC, and no path exists"* is
                    // a gap. ⊘ A refusal whose name is true of both is the shape §16.65's
                    // own comment records as costly — a name that is true of the bytes and
                    // silent about the cause.
                    match route {
                        kayfabe_rt::DoorbellRoute::HostGr => format!(
                            ". ⊘ THE GR ROUTE EXISTS AND IS DISARMED ON THIS RUN: \
                             {}={} (default `refuse`). Set it to `passthrough` to hand this \
                             doorbell to the core — and read \
                             `docs/design/gr_doorbell_passthrough.md` §0.3 first, because \
                             the host engine fetches nothing on either arm",
                            GR_ROUTE_ENV,
                            self.gr_route.as_str(),
                        ),
                        kayfabe_rt::DoorbellRoute::CpuCe | kayfabe_rt::DoorbellRoute::Unserved =>
                            ". ⊘ No executor and no core ring path has been designed for \
                             this engine at all — this is a GAP, not a disarmed flag"
                                .to_string(),
                    },
                ),
            ));
        }
        // ★★★ §14.24 — see `SharedDoorbell::local_ce_is_the_only_executor` for the boot
        // that turned this from `vas_pdb.is_none()` into a question about EXECUTORS.
        // ★★★★★ §16.81 — and the THIRD term, which is [`forwarding_plane_owns_ce`]'s whole
        // subject: **whose proc is this?** See that function for the rule and the boot.
        if forwarding_plane_owns_ce(
            facts.kind,
            facts.vas_pdb.is_some(),
            self.local_ce_is_the_only_executor,
        ) {
            return None; // the core can address AND serve this channel; it is not ours.
        }
        // ★★★★★ §16.81 — SAY SO WHEN THE NEW TERM IS WHAT KEPT THIS DOORBELL, and only then.
        //
        // The condition below is exactly *"the first two terms said hand it away and the
        // third said no"*. ⊘ It is not `facts.proc == SYSTEM_PROC`: on the `local` arm and on
        // a `Stillborn` plane this doorbell was already the shell's, and printing there would
        // make the line mean *"a system-proc doorbell arrived"* — a different, much weaker
        // statement that would also make the two historic arms' logs stop being byte-
        // comparable to their own committed predecessors.
        //
        // ⇒ On `local` this prints **zero** times and the control stays byte-identical; on
        // `host` a non-zero count is the term doing the work, and `0` on `host` would mean
        // the fix never fired and any survival is somebody else's.
        // ★★★★★ 2026-08-11 — the third conjunct reads the SAME declared kind the gate
        // above does, rather than a second re-derivation of it. It was
        // `facts.proc == Gpu::SYSTEM_PROC`: identical truth value (both come off one pass
        // over one `ProcBoundary`), and a diagnostic that re-derives what the decision
        // beside it was told is how a log comes to disagree with the branch it describes.
        if facts.vas_pdb.is_some()
            && !self.local_ce_is_the_only_executor
            && facts.kind == kayfabe_core::channel_kind::GuestChannelKind::Emulated
        {
            // ★★★★ `fetch_add`, NOT a mutex — see [`CeShellState::sysproc_kept`] for the
            // gate that found the lock and for why deleting it beats classifying it. The
            // index is still unique and still 1-based, so *"the largest index in the log is
            // the total"* survives verbatim; `Relaxed` is enough because nothing is
            // published through this counter — no reader learns about any other memory from
            // it, and the only consumer is the human reading the line it is printed on.
            let n = self
                .ce
                .sysproc_kept
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                + 1;
            // ⊘ Deliberately AFTER the increment and beneath no guard at all: the
            // `eprintln!` takes the process-global stderr lock and issues a `write(2)`, and
            // the `pdb` argument below allocates a `String`. R1 (`l1_concurrency.md` §3.3)
            // forbids every one of those beneath a lock, and this site used to do all three.
            // ★★★★★ **THE OWNER'S 2026-08-11 TRAP CONTRACT, READ AND REPORTED AS VIOLATED
            // — on the one line that fires exactly when it is.**
            //
            // This branch is reached only for a `GuestChannelKind::Emulated` channel whose
            // doorbell the shell has just decided to keep, i.e. exactly when this thread is
            // about to run the guest kernel's CE work itself. The kind's declared contract
            // is `TrapContract::ScheduleAndReturn`, and `may_run_on_the_vcpu_thread()` is
            // `false` — so the line below states the rule and states that we are breaking
            // it, in the same breath, rather than leaving a reader to join two documents.
            //
            // ⊘ **Reported, not enforced, and the type says why**: nothing in Rust can see
            // that this call is on a vCPU thread, and the emulated arm's handler is not yet
            // a separable object for a witness token to guard. `[measured 2026-08-11]` the
            // trap is inline end to end — QEMU BQL → `kayfabe_shim_regs_write` →
            // `RegPlane::ring_doorbell` (RwLock read held across it) → here →
            // `ceutils::run_submission` under the FSM mutex — with no spawn, no channel
            // send and no queue push anywhere on it.
            //
            // ⚠ The contract's name is a `&'static str` and costs no allocation; this site
            // is already deliberately beneath no guard, per the note above.
            let contract = facts.kind.trap_contract();
            eprintln!(
                "kayfabe: CE-SYSPROC-KEPT #{n} token={token:#010x} proc={} chan={} \
                 kind={} contract={contract} pdb={} — `l1_concurrency.md` §12.26: the \
                 SYSTEM proc has no data plane and its CeUtils scrub is FORGED, never \
                 forwarded, so this doorbell is the shell's whatever KAYFABE_CE_EXECUTOR \
                 says. ⊘ The forwarding hand-off stays armed for every USER proc. \
                 ⚠ OWNER 2026-08-11: this channel's contract is `{contract}` and the work \
                 below runs INLINE ON THIS THREAD — the rung that discharges it is a \
                 scheduling seam, not a rename.",
                facts.proc.0,
                facts.chan.0,
                facts.kind,
                facts
                    .vas_pdb
                    .map_or_else(|| "NONE".to_string(), |p| format!("0x{:x}", p.0)),
            );
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
        // ★ The channel's accumulator, or a fresh one on its first-ever doorbell.
        let state = *self
            .ce
            .states
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            .unwrap_or(&kayfabe_rt::ceutils::MethodState::new());

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
        let mut run = |root: &kayfabe_device::ceresolve::VasRoot| {
            plane.ce_session_with_root(root, demand, |ce| {
                self.device.with_pushbuffer(|pb| {
                    kayfabe_rt::ceutils::run_submission(ce, pb, vmm, chan, cursor, state)
                })
            })
        };
        // ★★★★ §16.64 — TWO ROOT SOURCES, tried in the order of what each one KNOWS.
        //
        // 1. This device's own publication table, keyed `(hClient, hVASpace)`. It answers
        //    for every RM-managed VA space and is the path `pdb=Y ×8` and the four CeUtils
        //    `SERVED-LOCAL` lines already take. ⊘ Tried FIRST and unchanged, so nothing
        //    that works today can be routed differently by what follows.
        // 2. The **object model's** base for the VA space this channel actually resolved
        //    to. `[measured 2026-08-10, boot `s45_748a207_tsgsched`]` 187 of 448 doorbells
        //    were refused `NoPublication` on a root the graph was holding the whole time:
        //    a UVM-managed VAS publishes through `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY`
        //    (source 1 watches only the two `0x90f1`/`0x2080` RPC arms — see
        //    `kayfabe_device::gvaspub::is_pde_publication`) and it publishes under UVM's
        //    **dup** handle, while the channel resolves to the **origin** handle. Both
        //    sides resolve correctly, to different handles of one resource, so no
        //    handle-keyed table can join them — but `facts.vas_pdb` is resolved by resource
        //    IDENTITY (`RmGraph::pdb_of_resource`) and is therefore already the right base.
        //
        // ⊘ The order is not a preference, it is a claim about knowledge: source 1 carries
        // the guest's own published `pageShift` and VA window, source 2 derives the shift
        // from the installed format because the control has no field for one. A root that
        // states its own geometry beats one that infers it.
        // ⊘ Selected by [`SharedDoorbell::doorbell_root`], which the PROBE also calls —
        // see its docs for the boot in which these were two sites and disagreed in the log.
        let root = match SharedDoorbell::doorbell_root(
            &plane,
            facts.client,
            vaspace,
            facts.vas_pdb.map(|p| p.0),
        ) {
            DoorbellRoot::Published(r) | DoorbellRoot::Declared(r) => Some(r),
            // ⊘ A channel with genuinely no VA space. A real absence, falling through to
            // the refusal below — never papered over with a zero.
            DoorbellRoot::Absent => None,
            // ⊘ The derivation's own refusal is REPORTED BY ITS OWN NAME rather than
            // collapsed into `NoPublication`. "The guest published nothing" and "we could
            // not size the format's root level" are different diagnoses and exactly one of
            // them is our defect.
            DoorbellRoot::Underivable(phys, why) => {
                drop(held);
                return Some(refused(
                    token,
                    kayfabe_device::FaultTag("CeResolve::DeclaredRootUnusable"),
                    format!(
                        "the object model resolved page-directory base 0x{phys:x} for \
                         (hClient 0x{:x}, hVASpace 0x{vaspace:x}), but a walkable root \
                         could not be derived from it: {}{}",
                        facts.client,
                        why.describe(),
                        self.addressing_probe(token)
                    ),
                ));
            }
        };
        let outcome = root.as_ref().map(&mut run);
        drop(held);

        let Some(outcome) = outcome else {
            // ⊘ NEITHER source had a root. §16.64 narrowed what this sentence may claim:
            // it used to say "no page-directory root was published", which was **false
            // about the guest** whenever a UVM-managed VAS had published one through a
            // transport this device's table does not watch. Reaching here now also means
            // the object model holds no base for the VA space the channel resolved to —
            // i.e. nobody, on either side, knows of a root.
            return Some(refused(
                token,
                kayfabe_device::FaultTag("CeResolve::NoPublication"),
                format!(
                    "no page-directory root is known for (hClient 0x{:x}, hVASpace \
                     0x{vaspace:x}) — neither this device's publication table nor the \
                     object model's own base for the VA space this channel resolved to{}",
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
                // ⊘ Committed on the SAME arm as the cursor and nowhere else: a refused
                // submission must leave the channel exactly where it was.
                self.ce
                    .states
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(key, run.state);
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
    /// ★★★★ §16.25 — every live channel's VA-space resolution, **grouped by outcome**, with
    /// the refused channel marked.
    ///
    /// # Why grouped rather than one row per channel
    ///
    /// Two reasons, and the second is the load-bearing one.
    ///
    /// 1. **Budget.** The whole refusal sentence is [`DOORBELL_REFUSAL_LEN`] = 2048 bytes.
    ///    24 channels × a full row would clip, and although [`copy_sentence`] stamps a clip
    ///    marker (so a clipped sentence is at least *known* to be clipped), a report that
    ///    routinely loses its tail is a report whose tail nobody can cite.
    /// 2. ★ **Grouping IS the comparison.** The question this exists to answer is
    ///    *"what is different about the refused channels?"*. Collapsing channels that share
    ///    an outcome answers it directly: if the served and the refused channels fall into
    ///    the same group, the route is **not** the discriminator and this rung is refuted on
    ///    the spot; if they split, the group boundary names the difference. A flat list
    ///    would leave that comparison for a human to do by eye across 24 lines.
    ///
    /// ⊘ Exemplars are capped at [`kayfabe_core::gpu::VAS_CENSUS_EXEMPLARS`] per group —
    /// ★ **the one the census actually applies**, in the core function that builds the line.
    /// A private `CENSUS_EXEMPLARS` sat here saying the same `3` and was read by nothing;
    /// it is deleted rather than `allow`ed, because a second constant that agrees today is
    /// how the cap and the sentence describing it drift apart. The cap is **reported**
    /// (`+N more`) rather than silently applied — an elided row must never read as an
    /// absent one, which is the C oracle's `dlen=0` mistake in miniature.
    /// ★★★★ §16.27 — what the walling channel's own client namespace holds, by kind.
    ///
    /// Answers the one fork §16.25 left open (see
    /// [`kayfabe_rt::device::SharedDevice::namespace_census`]): is there a `VaSpace` under
    /// the Device this channel is parented on?
    ///
    /// ⊘ **`VaSpace` rows are printed IN FULL and never elided**, because they are the
    /// answer; everything else is summarised as a count per kind. A cap that dropped the
    /// one row the question is about would reproduce §16.25's mistake one level down.
    fn namespace_census_line(&self, client: u32) -> String {
        let rows = self.device.namespace_census(client);
        if rows.is_empty() {
            // ⊘ TRUE and loud: a refusal naming a client whose namespace holds nothing
            // means the refusal and the graph disagree about whether the client exists.
            return format!(" ns[c0x{client:x} EMPTY-NAMESPACE]");
        }
        let mut kinds: Vec<(String, usize)> = Vec::new();
        for r in &rows {
            let k = format!("{:?}", r.kind);
            match kinds.iter_mut().find(|(n, _)| *n == k) {
                Some((_, c)) => *c += 1,
                None => kinds.push((k, 1)),
            }
        }
        let mut out = format!(" ns[c0x{client:x} {} objs", rows.len());
        for (k, c) in &kinds {
            out.push_str(&format!(" {c}x{k}"));
        }
        // ★ Every VaSpace, in full: handle, the Device it hangs off, and whether it has
        // ever been bound. `pdb=NONE` is a THIRD answer, distinct from both forks — the
        // VASpace exists, the fourth route would find it, and it would still address
        // nothing.
        let mut any = false;
        for r in rows.iter().filter(|r| r.is_vaspace()) {
            any = true;
            out.push_str(&match r.pdb {
                Some(p) => format!(
                    " | VAS h0x{:x} parent0x{:x} pdb=0x{p:x}",
                    r.handle, r.parent
                ),
                None => format!(
                    " | VAS h0x{:x} parent0x{:x} pdb=NONE-BOUND",
                    r.handle, r.parent
                ),
            });
        }
        if !any {
            // ★★★ The MINT fork, stated positively rather than by the absence of a row —
            // an enumerated namespace with no VaSpace is a measurement; a report that just
            // never mentioned one is not.
            out.push_str(" | NO-VASPACE-IN-NAMESPACE");
        }
        out.push(']');
        out
    }

    /// The census for a doorbell refusal — the same instrument the promote path latches,
    /// over the same formatter (`kayfabe_core::gpu::format_vas_census`).
    ///
    /// ⊘ This used to carry its own copy of the grouping and printing. See
    /// `kayfabe_rt::device::ChannelVasRow` for why the second copy was removed and why the
    /// two *sources* nonetheless stay separate.
    fn vas_census_line(&self, refused: kayfabe_core::ChanId) -> String {
        kayfabe_core::gpu::format_vas_census(&self.device.channel_vas_census(), Some(refused))
    }

    fn addressing_probe(&self, token: u64) -> String {
        let Ok(facts) = self.device.ce_channel_facts(DOORBELL_TARGET_GPU, token) else {
            return String::new();
        };
        self.addressing_probe_facts(facts)
    }

    /// ★★★★ **§16.72 — the probe over facts SOMEBODY ELSE ALREADY RESOLVED.**
    ///
    /// [`SharedDoorbell::addressing_probe`] takes a token and resolves the channel itself,
    /// which is right for the three refusal sites (they have no facts in hand). It is
    /// **wrong** for the forwarding fall-through, which is holding
    /// [`kayfabe_rt::device::CeChannelFacts`] in `seen` — a second `ce_channel_facts` call
    /// there would be §16.64's defect exactly: *"a probe that re-derives what it describes
    /// can disagree with it, and this one did, in the direction that reads as still
    /// broken"*. §16.71.2(3) already paid for this once and took the out-parameter; this
    /// split is what lets the walk reuse it rather than re-derive it.
    ///
    /// ⊘ **No resolver is changed and nothing here is served.** Identical body, identical
    /// output, one fewer resolution.
    /// ★★★★★ **G1 + G2 + G3 — the CPU transport's page-table writes, attributed, decoded
    /// and published, at the guest's own commit point.**
    ///
    /// `execution_plane_increments.md` §16.73.8's three wirings, joined here because this is
    /// the one place that holds both halves: the **plane** (which witnessed the writes and
    /// holds the bytes) and the **device** (which owns the address table). Neither crate may
    /// name the other's state, and neither is the composition root.
    ///
    /// # What each link is
    ///
    /// 1. **G1 — the witness.** `RegPlane::drain_pt_witness` — pages the guest's CPU wrote
    ///    through a framebuffer window, recorded at the same statement that stamps
    ///    `/byBAR2`. Until this rung `Vas::pt_pages` was fed **only** by a CE pushbuffer
    ///    parse, so everything this transport published stayed a miss.
    /// 2. **G2 — the consumer.** `SharedDevice::decode_pt_writes`, which had a definition,
    ///    two test call sites and **no production caller** — the
    ///    `a_declared_capability_reachable_from_nowhere` shape, for the fourth time in this
    ///    campaign. It is called here, at the doorbell, which `ceresolve`'s module doc
    ///    already names as the guest's own submit fence and the only commit point on this
    ///    path.
    /// 3. **G3 — the byte source.** `RegPlane::pt_bytes`, the device's own `FbStore`, and
    ///    **not** `kayfabe_fwd::IsolateFb` — whose production backend is
    ///    `Err(NOT_ON_THIS_RUNG)` and which reads the fabricated aperture, a different
    ///    store. See `SharedDevice::decode_pt_writes_from` for the seam and the measurement
    ///    that decides it.
    ///
    /// # ⊘ Where it is called from, and why the control stays byte-identical
    ///
    /// From the **forwarding fall-through only**, beside the `RING-PROJ` block and for that
    /// block's stated reason: on the `Stillborn` control plane `try_ce_submission` claims
    /// every routed doorbell terminally, so no doorbell reaches this line and the control's
    /// committed census cannot move. ⚠ That is a claim about a code path, and the boot is
    /// what tests it — `PT-DECODE` must read **0 lines** on the control.
    ///
    /// # ⚠ Locks
    ///
    /// Called with **no** lock held — before `self.ce.vmm` is taken below, and `ring` is
    /// documented as running with no plane lock out. The plane's mutex is taken and released
    /// inside each `PlanePtBytes::read`; the core's ranked locks are taken and released
    /// inside each phase of the pass. ⊘ The two are never nested, in either direction.
    /// ★★★★★ **THE FIRST PRODUCTION `GuestRamGrant`** — pin the page of GUEST RAM the
    /// channel's ring lives in into the host VAS, at the guest's own VA
    /// (`guest_ram_crossing.md` §5.8, step 3).
    ///
    /// Returns the line to print, or `None` when the crossing is not armed.
    ///
    /// # ★★★ Where every number comes from, because that is the rule this rung exists for
    ///
    /// | number | source | why not somewhere else |
    /// |---|---|---|
    /// | the ring's **VA** | the channel's own declared `gp_fifo_ring` ([`kayfabe_rt::device::CeChannelFacts::ring_va`]) | it is the guest's, and address identity means the host mapping must land on exactly it |
    /// | the ring's **GPA** | the core's address table, resolved in the channel's own `Vas` | the guest's page tables are the authority on what backs its own VA; ⊘ **the leaf address `0x237fe000` §16.73 measured is NOT reusable** — three channels of one boot resolved to `0x768a000`, `0x802d000` and `0x206cf000` |
    /// | the **file offset** | [`kayfabe_vmm_qemu::QemuVmm::resolve_guest_ram`], the hypervisor's own stated layout | ⊘ **never derived from the GPA.** Identity holds on `-m 2048` and breaks silently at `-m 8G`; that is `layout`'s entire reason for existing |
    /// | the **length** | [`Self::RING_PIN_BYTES`] | see that constant — it is a measurement, not a default |
    ///
    /// ⊘ **No number in the grant was proposed by the isolate, and none was checked
    /// against itself.** `mode2_isolate_memory_boundary.md` §3.
    ///
    /// # ⊘ What this does NOT do, stated before anyone reads a green line as more
    ///
    /// It pins **one page**, of **one** channel's ring, and **nothing consumes it**. The
    /// forwarding plane still reads the ring through `Vmm::gpa_read` as before and the host
    /// GPU is never pointed at the pinned mapping on this rung. What is established is the
    /// two facts the rung was set: a real RM object exists over guest RAM, and a **fixed**
    /// `map_dma` placed it at the guest's own VA. ⊘ That is not a shadow channel.
    ///
    /// # ⚠ Locks
    ///
    /// The layout is read under `self.ce.vmm`'s unranked mutex, which is **released before**
    /// the pin — [`kayfabe_rt::device::SharedDevice::pin_guest_ram`] runs host ioctls and
    /// may park on the isolate pool, and holding an unranked mutex across that is the exact
    /// shape the `ring` body's own lock note warns about one screen down.
    fn pin_ring_guest_ram(
        &self,
        token: u64,
        facts: Option<&kayfabe_rt::device::CeChannelFacts>,
    ) -> Option<String> {
        let backing = self.guest_ram_backing?;
        let head = format!(
            "GUEST-RAM PIN token={token:#010x} dev={} ino={}",
            backing.dev, backing.ino
        );
        // ⊘ Every early return below still prints. A pass that ran and found nothing to do
        // and a pass that did not run are different facts about the boot, and only one of
        // them is about the guest — the same rule `PT-DECODE` states one function down.
        let Some(f) = facts else {
            return Some(format!(
                "{head} → NO CHANNEL (the token routed to no channel, so there is no ring \
                 and nothing to pin)"
            ));
        };
        let Some(ring_va) = f.ring_va else {
            return Some(format!(
                "{head} proc={} chan={} → NO RING VA DECLARED (the channel named no \
                 `gp_fifo_ring`; ⊘ not a miss — there is no address to pin AT)",
                f.proc.0, f.chan.0
            ));
        };
        let Some(pdb) = f.vas_pdb else {
            return Some(format!(
                "{head} proc={} chan={} ring=0x{ring_va:x} → NO PDB (the channel's VA space \
                 did not resolve, so there is no address space to pin INTO)",
                f.proc.0, f.chan.0
            ));
        };
        let who = format!(
            "{head} proc={} chan={} pdb=0x{:x} ring=0x{ring_va:x}",
            f.proc.0, f.chan.0, pdb.0
        );
        // ---- 1. the GPA, from the guest's own page tables (via the core's table) -------
        let va = kayfabe_rt::GpuVa(ring_va);
        let (binding, off) = match self.device.resolve(DOORBELL_TARGET_GPU, pdb, va) {
            Ok(r) => r,
            Err(e) => {
                return Some(format!(
                    "{who} → UNRESOLVED {e:?} (the address table does not bind this VA; \
                     ⊘ MISS = FAULT, and nothing here guesses a guest-physical address)"
                ));
            }
        };
        // ★★★★★ **§16.82 — THE APERTURE, ASKED BEFORE THE NUMBER IS CALLED A `gpa`.**
        //
        // ⊘ Until this rung the next line read `let gpa = binding.phys + off;` with **no
        // aperture test**, and the whole function — its name, its log tag `GUEST-RAM PIN`,
        // its `resolve_guest_ram` call — asserts guest RAM about whatever came back.
        // `kayfabe_mmu::Binding::phys` is documented as *"interpretation depends on
        // `aperture`; for sysmem this is a guest-physical address"*, so a **vidmem** binding
        // would have handed the hypervisor's layout a **framebuffer** address and pinned the
        // guest RAM page that happens to share the number.
        //
        // ⚠ **It has never fired only because the lookup above has never succeeded.** The
        // same boot that walls here (`w232c`) resolves this exact VA through the descent and
        // reports `rng=V:0x1024000` — `V` is *this device's framebuffer*
        // (`kayfabe_device::ceresolve::CeResolve::tag`). ⇒ the first doorbell that populates
        // this table takes this branch, and without the check it would take the other one
        // silently. `kayfabe_fwd::push_range_gpas` already refuses exactly this, by name,
        // eleven lines of a different file away; this is the same refusal at the second
        // consumer of the same table.
        if !binding.is_guest_ram() {
            return Some(format!(
                "{who} → NOT IN GUEST RAM (the table binds this VA in aperture {:?} at \
                 0x{:x}; `Binding::phys` is a guest-physical address ONLY for sysmem, so \
                 there is no file offset to ask the layout for. ⊘ Refused by name — nothing \
                 here reinterprets a framebuffer address as a GPA)",
                binding.aperture(),
                binding.phys().saturating_add(off)
            ));
        }
        let gpa = binding.phys().saturating_add(off);
        // ★★★★★ **G6 — HOW MUCH OF THE RING, and it is DERIVED from the guest's own
        // declaration rather than chosen.**
        //
        // [measured 2026-08-10, `run_w229b_b66bd44_execvas_real_qemu.log`] the entry counts
        // this guest declares are **4096** (the CE channel behind the doorbells we forward),
        // **1024** and **32**. A GPFIFO entry is 8 bytes, so those rings are **32 KiB**,
        // **8 KiB** and 256 bytes — and the one page this used to pin is **one eighth** of
        // the first. ⇒ The old constant was not merely conservative: seven eighths of the
        // queue hardware would fetch from was never described to RM at all.
        //
        // ⊘ And a bigger constant is still the wrong shape, for the reason
        // `RING_PIN_BYTES`'s own docs record: four consecutive guest **virtual** pages of
        // this ring resolve to four scattered guest **physical** pages, and an
        // `OS_DESCRIPTOR` describes ONE contiguous host range. So the length is derived and
        // then **split at every discontinuity** — one descriptor per contiguous run, walked
        // below.
        let want = u64::from(f.ring_entries).saturating_mul(Self::GP_FIFO_ENTRY_BYTES);
        if want == 0 {
            return Some(format!(
                "{who} gpa=0x{gpa:x} → NO EXTENT (the channel declared {} entries, so the \
                 ring is zero bytes long; ⊘ not a miss — there is nothing to pin)",
                f.ring_entries
            ));
        }
        if !ring_va.is_multiple_of(Self::RING_PIN_BYTES) {
            return Some(format!(
                "{who} gpa=0x{gpa:x} want={want} → UNALIGNED RING VA (the walk below steps in \
                 host pages and cannot express a ring that starts mid-page; ⊘ refused by \
                 name rather than silently rounded, which would pin bytes the guest did not \
                 name)"
            ));
        }
        // ---- 1b. the RUNS, from the guest's own page tables, one lookup per page --------
        //
        // ⊘ Every page is resolved through the address table exactly as the first one was.
        // Nothing here assumes the next page follows the last: that assumption is what the
        // measurement above refutes, and adopting it would produce a plausible file offset
        // for bytes that live somewhere else.
        let pages = want.div_ceil(Self::RING_PIN_BYTES);
        let mut runs: Vec<(u64, u64, u64)> = Vec::new(); // (va, gpa, len)
        let mut unresolved: Option<(u64, String)> = None;
        for i in 0..pages {
            let pva = ring_va + i * Self::RING_PIN_BYTES;
            let Ok((b, o)) = self
                .device
                .resolve(DOORBELL_TARGET_GPU, pdb, kayfabe_rt::GpuVa(pva))
            else {
                unresolved = Some((pva, format!("page {i} of {pages}")));
                break;
            };
            let pgpa = b.phys().saturating_add(o);
            match runs.last_mut() {
                Some((_, rgpa, rlen)) if *rgpa + *rlen == pgpa => *rlen += Self::RING_PIN_BYTES,
                _ => runs.push((pva, pgpa, Self::RING_PIN_BYTES)),
            }
        }
        // ★ The last run is trimmed to the ring's true end: a 4096-entry ring is a whole
        // number of pages, a 32-entry one is 256 bytes, and pinning the rest of that page
        // would be describing memory the guest did not name to this channel.
        if let Some((va0, _, rlen)) = runs.last_mut() {
            let end = ring_va + want;
            if *va0 + *rlen > end {
                *rlen = end - *va0;
            }
        }
        let geometry = format!(
            " | ★ GEOMETRY: {} entries x {} = {want} bytes = {pages} pages \
             in {} contiguous run(s){}",
            f.ring_entries,
            Self::GP_FIFO_ENTRY_BYTES,
            runs.len(),
            match &unresolved {
                Some((pva, at)) => format!(
                    " — ⚠ TRUNCATED: {at} (va 0x{pva:x}) does not resolve, so the runs below \
                     cover only the prefix that does"
                ),
                None => String::new(),
            }
        );
        let Some(&(_, _, len)) = runs.first() else {
            return Some(format!(
                "{who} gpa=0x{gpa:x} → NOT ONE PAGE RESOLVED{geometry}"
            ));
        };
        // ---- 2. the file offset, from the HYPERVISOR's own stated layout ---------------
        //
        // ⊘ Two different questions are asked of two different sources here, and that is
        // the point: the guest's page tables say WHICH guest-physical bytes, and the
        // hypervisor says where those bytes live in the descriptor. Deriving the second
        // from the first is the identity shortcut `layout.rs` refuses.
        let (resolved, control) = {
            let held = self.ce.vmm.lock().unwrap_or_else(|e| e.into_inner());
            let Some(vmm) = held.as_ref() else {
                drop(held);
                return Some(format!(
                    "{who} gpa=0x{gpa:x} → NO MEMORY PLANE (between realize and \
                     `attach_ram` there is no layout to resolve against)"
                ));
            };
            // ★ Every run is asked of the hypervisor separately, under ONE hold of the
            // layout: two runs resolved either side of a layout change would be two answers
            // about two different machines.
            let resolved: Vec<_> = runs
                .iter()
                .map(|&(va, rgpa, rlen)| {
                    (va, rgpa, rlen, vmm.resolve_guest_ram(backing, rgpa, rlen))
                })
                .collect();
            // ★★★ THE NEGATIVE CONTROL, taken from the SAME table, in the SAME instant, on
            // the SAME line. A control read at a different moment could differ for a
            // reason that has nothing to do with the mechanism.
            //
            // ⊘ The probe address is DERIVED, not a constant: one page past the top of the
            // highest run stated right now, so it is outside every stated run **by
            // construction** on any machine and any `-m`. A hardcoded address would be
            // outside on this bench and could silently be inside on another — and a
            // control that passes because of the machine it ran on is not a control.
            let top = vmm
                .stated_guest_ram(backing)
                .iter()
                .map(|r| r.gpa_end())
                .max()
                .unwrap_or(0);
            let probe = u64::try_from(top).unwrap_or(u64::MAX).saturating_add(len);
            let control = (probe, vmm.resolve_guest_ram(backing, probe, len));
            drop(held);
            (resolved, control)
        };
        let control = match control.1 {
            Err(r) => format!(
                " | ✅ NEGATIVE CONTROL: gpa=0x{:x} (one page past the top of every stated \
                 run) REFUSED BY NAME as `{}` — no clamping, no best-effort, no \
                 probably-identity",
                control.0,
                r.name()
            ),
            Ok(bad) => format!(
                " | ⚠⚠ NEGATIVE CONTROL DID NOT FIRE: gpa=0x{:x} was ANSWERED with file \
                 offset 0x{:x}. Either the layout grew past the top this probe was derived \
                 from between the two reads, or the resolver is answering outside what it \
                 was told. ⊘ Read every line above with that in mind",
                control.0, bad.file_offset
            ),
        };
        // ---- 3+4. one grant and one pin PER CONTIGUOUS RUN -----------------------------
        //
        // ⊘ Every run is reported, including the ones that refuse, and the report says which
        // run it is out of how many. A loop that stopped at the first refusal would make
        // "the ring is one run and it failed" and "the ring is eight runs and the second
        // failed" the same line.
        let total = resolved.len();
        let mut lines: Vec<String> = Vec::new();
        let mut pinned = 0usize;
        let mut bytes_pinned = 0u64;
        let mut wall = false;
        for (i, &(rva, rgpa, rlen, ref run)) in resolved.iter().enumerate() {
            let at = format!(
                "{who} run {}/{total} va=0x{rva:x} gpa=0x{rgpa:x} len={rlen}",
                i + 1
            );
            let run = match run {
                Ok(r) => r,
                Err(r) => {
                    lines.push(format!(
                        "{at} → REFUSED BY NAME `{}` (the hypervisor stated no run covering \
                         this guest-physical address for the block we adopted; ⊘ NOT clamped \
                         and NOT assumed identity)",
                        r.name()
                    ));
                    continue;
                }
            };
            // ★ Read-WRITE, and it is a decision rather than the wider default. These pages
            // carry the channel's GPFIFO and, at `+0x8004`, its finishPayload semaphore —
            // memory the ENGINE writes. `OS_DESCRIPTOR` hands RM a host VA it walks with
            // `pin_user_pages`, and pinning for a DMA the device writes needs a writable
            // mapping. ⊘ The narrower grant is not the safer one here; it is the one that
            // fails at the ioctl for a reason the status code will not name.
            let grant = kayfabe_isolate::GuestRamGrant::originated_by_the_vmm(
                run.file_offset,
                rlen,
                kayfabe_vmm::Prot::ReadWrite,
            );
            match self
                .device
                .pin_guest_ram(DOORBELL_TARGET_GPU, pdb, kayfabe_rt::GpuVa(rva), grant)
            {
                Ok(p) => {
                    pinned += 1;
                    bytes_pinned += rlen;
                    lines.push(format!(
                        "{at} → file offset 0x{:x} → {} memory={:#x} host_va=0x{:x} \
                         placed_as_asked={}",
                        run.file_offset,
                        if p.already {
                            "ALREADY PINNED (idempotent replay)"
                        } else {
                            "PINNED"
                        },
                        p.memory.raw(),
                        p.host_va,
                        p.host_va == rva,
                    ));
                }
                Err(e) => {
                    if matches!(e, kayfabe_rt::FwdFault::SystemDataPlane) {
                        wall = true;
                    }
                    lines.push(format!(
                        "{at} → file offset 0x{:x} → REFUSED {e:?}",
                        run.file_offset
                    ));
                }
            }
        }
        // ★★★★★ **THE WALL, and it is NAMED rather than left to be decoded.**
        //
        // `SystemDataPlane` is not a defect and must not be read as one. Every doorbell that
        // reaches this fall-through on this bench belongs to the **system proc** —
        // RmInitAdapter's CE scrubber, client `0xc1e0…`, a KERNEL client — and
        // `l1_concurrency.md` §12.26 forbids the system proc a data plane: its work is
        // FORGED to its own completion queue, never forwarded, precisely so it can never
        // hold host state whose reclaim has no defined point. That rule's own docs say the
        // day it must be re-opened, it is re-opened **deliberately**, "with a refcount or a
        // global quiesce point — not discovered afterwards".
        //
        // ⊘ So this line is where a rung stops, not where a fix goes. Relaxing the guard
        // here to make a pin happen would be deleting a lifetime boundary to enable an
        // internal capability, which is `same_class_id_opposite_directions` exactly.
        let verdict = if pinned == total {
            format!(
                " | ★ ALL {total} RUN(S) PINNED, {bytes_pinned} of {want} bytes — one REAL \
                 host RM object (`NV01_MEMORY_SYSTEM_OS_DESCRIPTOR`) per contiguous run now \
                 exists over the guest's own pages, each mapped FIXED at the guest's own VA. \
                 ⊘ Nothing consumes them yet"
            )
        } else if wall {
            format!(
                " | ⊘ {pinned} of {total} run(s) pinned — REFUSED `SystemDataPlane`, THE \
                 WALL, and it is a STANDING DESIGN RULE, not a defect. This channel belongs \
                 to the SYSTEM proc (the guest kernel's own client), and `l1_concurrency.md` \
                 §12.26 gives the system proc no data plane: its work is FORGED, never \
                 forwarded, so it can hold no host state whose reclaim has no defined point. \
                 ★ Everything BEFORE it succeeded — every run's VA resolved, its GPA came out \
                 of the guest's own page tables, and the hypervisor's stated layout answered \
                 with a file offset. ⇒ What is unbuilt is a LIFETIME for system-proc host \
                 state, and re-opening §12.26 is an owner decision"
            )
        } else {
            format!(
                " | ⚠ {pinned} of {total} run(s) pinned, {bytes_pinned} of {want} bytes. ⚠ If \
                 a line below names `PlacementRefused`, that fixed map landed somewhere else \
                 and was UNWOUND rather than adopted. ⚠ If one names an RM status `0x51`, \
                 that is `NV_ERR_NO_MEMORY` and it is COLLISION-OR-EXHAUSTION — the two are \
                 indistinguishable from the status alone and neither reads as success"
            )
        };
        Some(format!(
            "{who} gpa=0x{gpa:x}{geometry}{verdict}{control}\n    {}",
            lines.join("\n    ")
        ))
    }

    /// ★★★★★ **LEG 4 — PIN THE PUSHBUFFER PAGES THE RING'S OWN GPFIFO ENTRIES NAME.**
    ///
    /// # ⊘⊘ THE BRIEF SAID `join_fb_leaf`, AND THAT IS THE WRONG PLANE — measured
    ///
    /// `w263` produced eight host `Xid 31 FAULT_PDE ACCESS_TYPE_VIRT_READ` at eight addresses
    /// byte-exact the pushbuffer VAs the guest's `gp[0]` entries name, and the rung brief read
    /// those addresses as *"Vidmem … and `join_fb_leaf` already reaches them"*.
    ///
    /// **They are in GUEST RAM.** `[measured, traces/boots/w263/run_w263_ring_qemu.log, all 8
    /// channels, BOTH arms]` `gp[0]@0x200218000=0x202400000+0x20 pb=**S**:0x3d45f000`, and
    /// `kayfabe_device::ceresolve::CeResolve::tag`'s own doc is the authority on the letter:
    /// *"`V` = this device's framebuffer, **`S` = guest RAM**, `P` = peer"*. The `Vidmem` in
    /// the brief is the **ring's** aperture (`rng=V:0x1024000`), and the
    /// `FwdFault::PushbufferAperture { va: GpuVa(8592179200) }` it cites decodes to
    /// `0x200224000` — the ring's VA, not a pushbuffer's.
    ///
    /// ⇒ [`kayfabe_rt::ceutils::resolve_leaf_of`] answers `(Site::GuestRam, **None**)` for a
    /// sysmem resolution and says why in its own comment — *"it is not this source's to join:
    /// the guest-RAM pin owns that plane"*. A join built here would have refused eight times
    /// and joined nothing.
    ///
    /// # ★★★ SO THIS IS [`Self::pin_ring_guest_ram`]'S MISSING SOURCE, NOT A NEW MECHANISM
    ///
    /// That function is the whole chain — VA → the core's address table → GPA → **aperture
    /// check** → the hypervisor's own stated layout → file offset → `GuestRamGrant` → one
    /// `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` per contiguous run, mapped **FIXED at the guest's
    /// own VA**. It is asked about exactly one address, **the ring's**, which is in Vidmem, so
    /// on `w263` it refused all eight by name and correctly. ⇒ The pin has never pinned one
    /// byte on a live guest, and not because it is broken: nobody hands it an address in the
    /// aperture it serves. The addresses in that aperture are on the same log line.
    ///
    /// This is **leg A1's shape one plane over**: the primitive works, the source list is
    /// short. `docs/design/w264_pushbuffer_pin_prereg.md` §0.
    ///
    /// # ⚠ THE STRIDE IS DERIVED, NEVER ASSUMED
    ///
    /// The `0x200000` spacing is what *this* workload produced. Every address below comes out
    /// of **its own entry**, exactly as leg A1 derives a ring from a channel. The observed
    /// stride is *printed* so a reader can see it was observed — ⊘ it is never read.
    ///
    /// # ⚠ BOUNDED, AND THE BOUND IS LOUD
    ///
    /// A ring has up to 4096 entries. [`PUSHBUF_MAX_EXTENTS`] and [`PUSHBUF_MAX_PAGES`] cap
    /// the work per doorbell, and an overflow prints `⚠ CAPPED` with the count dropped **and
    /// the first dropped VA**. ⊘ A cap that truncated silently would be a false green — the
    /// same class as a `dlen=0` oracle row and a zero-byte bench artefact.
    ///
    /// # ⊘ OPACITY
    ///
    /// Reading a GPFIFO entry **to learn where to map** is supply-side work. Nothing here
    /// decodes a method, classifies work, or gates whether any doorbell is forwarded: every
    /// arm returns a `String` and the caller prints it. The opacity pin's property —
    /// *"whether a doorbell is forwarded must not depend on whether its ring can be read"* —
    /// is untouched, because this returns no decision.
    ///
    /// # ⚠ Locks
    ///
    /// Identical to [`Self::pin_ring_guest_ram`]'s and for its reasons: the plane's own mutex
    /// is taken and released inside each `read_va_from_root`; `self.ce.vmm` is held only
    /// across the layout reads and dropped **before** any pin, because
    /// [`kayfabe_rt::device::SharedDevice::pin_guest_ram`] runs host ioctls and may park on
    /// the isolate pool.
    #[allow(clippy::too_many_lines)]
    fn pin_pushbuffer_guest_ram(
        &self,
        token: u64,
        facts: Option<&kayfabe_rt::device::CeChannelFacts>,
    ) -> Option<String> {
        // ⊘ SILENT when disarmed — not merely quiet. The control's log must not contain a
        // line the armed run's does not, or the two stop being comparable, which is the whole
        // use of a control. The arm itself is on disk, printed once at the root.
        if !self.guest_pushbuf.pins() {
            return None;
        }
        let backing = self.guest_ram_backing?;
        let head = format!(
            "PB-PIN token={token:#010x} dev={} ino={}",
            backing.dev, backing.ino
        );
        // ⊘ Every early return below still PRINTS. A pass that ran and found nothing to do
        // and a pass that did not run are different facts about the boot, and only one of
        // them is about the guest.
        let Some(f) = facts else {
            return Some(format!(
                "{head} → NO CHANNEL (the token routed to no channel, so there is no ring to \
                 read entries out of)"
            ));
        };
        let (Some(ring_va), Some(pdb), Some(vaspace)) = (f.ring_va, f.vas_pdb, f.vaspace) else {
            return Some(format!(
                "{head} proc={} chan={} → NOTHING TO READ: ring_va={:?} vas_pdb={:?} \
                 vaspace={:?}. ⚠ `ring_va = Some(0)` would be a VALUE, not a blank",
                f.proc.0, f.chan.0, f.ring_va, f.vas_pdb, f.vaspace
            ));
        };
        let who = format!(
            "{head} proc={} chan={} pdb=0x{:x} ring=0x{ring_va:x} entries={}",
            f.proc.0, f.chan.0, pdb.0, f.ring_entries
        );
        let Some(plane) = self.plane.upgrade() else {
            return Some(format!("{who} → NO PLANE (the register plane is gone)"));
        };
        let root = match SharedDoorbell::doorbell_root(&plane, f.client, vaspace, Some(pdb.0)) {
            DoorbellRoot::Published(r) | DoorbellRoot::Declared(r) => r,
            DoorbellRoot::Absent => {
                return Some(format!(
                    "{who} → NO ROOT (this channel has no VA space root, so its ring VA \
                     cannot be walked and no entry can be read)"
                ));
            }
            DoorbellRoot::Underivable(p, why) => {
                return Some(format!(
                    "{who} → ROOT UNDERIVABLE from pdb 0x{p:x}: {}",
                    why.kind()
                ));
            }
        };
        // ---- 1. THE ENTRIES, read through the SAME descent every other probe uses --------
        //
        // ⊘ Not a second reader. `read_va_from_root` is what `ring_scan` and the `gp[idx]`
        // probe already call, so a disagreement between this pass and the line beside it in
        // the log would be a fact about lifetime, never about two decoders.
        let demand = kayfabe_device::ceresolve::Demand::from_doorbell();
        let n = (f.ring_entries as usize).clamp(1, RING_SCAN_ENTRIES);
        let mut extents: Vec<(usize, u64, u64)> = Vec::new(); // (index, va, len)
        let mut unread = 0usize;
        let mut zero = 0usize;
        let mut control_entries = 0usize;
        let mut dropped_extents = 0usize;
        let mut first_dropped: Option<(usize, u64)> = None;
        for i in 0..n {
            let at = ring_va.wrapping_add((i * PROBE_RING_BYTES) as u64);
            let mut gp = [0u8; PROBE_RING_BYTES];
            if plane.read_va_from_root(&root, at, &mut gp, demand).is_err() {
                unread += 1;
                continue;
            }
            let raw = u64::from_le_bytes(gp);
            if raw == 0 {
                zero += 1;
                continue;
            }
            // ⊘ `None` here is a CONTROL entry (`LENGTH == 0`), and `gp_entry_decode`'s own
            // doc says why it must not be read as an address: entry1's low byte is `OPCODE`
            // and not `GET_HI`, so an address read out of it is a pointer the guest never
            // named. Counted as its own kind rather than folded into "unread".
            let Some(d) = kayfabe_abi::submit::gp_entry_decode(raw) else {
                control_entries += 1;
                continue;
            };
            // ⊘ De-duplicated on the pair the guest wrote, not on the page: two entries that
            // name the same bytes are one extent, and two that name different lengths at one
            // address are two facts.
            if extents.iter().any(|&(_, v, l)| v == d.gpu_va && l == d.len_bytes) {
                continue;
            }
            if extents.len() >= PUSHBUF_MAX_EXTENTS {
                dropped_extents += 1;
                first_dropped.get_or_insert((i, d.gpu_va));
                continue;
            }
            extents.push((i, d.gpu_va, d.len_bytes));
        }
        // ★ The OBSERVED stride, PRINTED and never read. `0x200000` is this workload's, and a
        // rung that encoded it would read correctly on exactly one boot.
        let stride = match extents.as_slice() {
            [_] | [] => "n/a (fewer than two extents)".to_string(),
            rest => {
                let mut s: Vec<u64> = rest.windows(2).map(|w| w[1].1.wrapping_sub(w[0].1)).collect();
                s.dedup();
                if s.len() == 1 {
                    format!("0x{:x} (uniform, OBSERVED — ⊘ never assumed)", s[0])
                } else {
                    format!("{} distinct gaps — NOT uniform", s.len())
                }
            }
        };
        let scan = format!(
            "{who} → SCAN {n} of {} entries: {} extent(s), unread={unread} zero={zero} \
             control_entries={control_entries} stride={stride}{}",
            f.ring_entries,
            extents.len(),
            match first_dropped {
                Some((i, va)) => format!(
                    " | ⚠⚠ CAPPED at {PUSHBUF_MAX_EXTENTS} extents — {dropped_extents} \
                     DROPPED, first at entry [{i}] va=0x{va:x}. ⊘ This pass is INCOMPLETE and \
                     must not be read as a full one"
                ),
                None => String::new(),
            }
        );
        if unread == n {
            return Some(format!(
                "{scan}\n    ⊘ NOTHING WAS READ: all {n} entries failed to resolve, so this \
                 says NOTHING about the ring's contents — it is a resolution failure, \
                 restated"
            ));
        }
        if extents.is_empty() {
            return Some(format!(
                "{scan}\n    ⊘ NO EXTENT NAMED: the ring holds no entry that names method \
                 words right now. ⊘ Not a miss — there is nothing to pin AT"
            ));
        }
        // ---- 2. THE PAGES, one address-table lookup each --------------------------------
        //
        // ⊘ Nothing here assumes the next page follows the last, in either space. Every page
        // is asked of the table separately, exactly as `pin_ring_guest_ram` does, because that
        // assumption is what `RING_PIN_BYTES`' own docs measure to be false.
        let page = Self::RING_PIN_BYTES;
        let (pages, dropped_pages, first_dropped_page) = pushbuffer_pages(&extents, page);
        let mut resolved_pages: Vec<(u64, u64)> = Vec::new(); // (va, gpa), VA-sorted
        let mut misses: Vec<String> = Vec::new();
        let mut wrong_aperture: Vec<String> = Vec::new();
        for &pva in &pages {
            match self
                .device
                .resolve(DOORBELL_TARGET_GPU, pdb, kayfabe_rt::GpuVa(pva))
            {
                Err(e) => {
                    if misses.len() < PUSHBUF_REPORT {
                        misses.push(format!("va=0x{pva:x}:{e:?}"));
                    }
                }
                // ★★★ THE APERTURE, ASKED BEFORE THE NUMBER IS CALLED A `gpa` — the same
                // refusal `pin_ring_guest_ram` makes, at the second consumer of one table.
                // ⊘ A vidmem binding here would hand the hypervisor's layout a FRAMEBUFFER
                // address and pin the guest-RAM page that happens to share the number.
                Ok((b, _)) if !b.is_guest_ram() => {
                    if wrong_aperture.len() < PUSHBUF_REPORT {
                        wrong_aperture.push(format!(
                            "va=0x{pva:x}:{:?}@0x{:x}",
                            b.aperture(),
                            b.phys()
                        ));
                    }
                }
                Ok((b, off)) => resolved_pages.push((pva, b.phys().saturating_add(off))),
            }
        }
        let table = format!(
            "{scan}\n    TABLE: {} page(s) asked, {} resolved in guest RAM, {} MISS{}, {} \
             NOT-IN-GUEST-RAM{}{}",
            pages.len(),
            resolved_pages.len(),
            pages.len() - resolved_pages.len() - wrong_aperture.len(),
            if misses.is_empty() {
                String::new()
            } else {
                format!(" [{}]", misses.join(" "))
            },
            wrong_aperture.len(),
            if wrong_aperture.is_empty() {
                String::new()
            } else {
                format!(" [{}]", wrong_aperture.join(" "))
            },
            match first_dropped_page {
                Some(va) => format!(
                    " | ⚠⚠ CAPPED at {PUSHBUF_MAX_PAGES} pages — {dropped_pages} DROPPED, \
                     first va=0x{va:x}. ⊘ INCOMPLETE"
                ),
                None => String::new(),
            }
        );
        if resolved_pages.is_empty() {
            return Some(format!(
                "{table}\n    ⊘ NOT ONE PAGE RESOLVED IN GUEST RAM — nothing was asked of the \
                 hypervisor and nothing was pinned. ⚠ A `MISS` here is a statement about the \
                 POPULATE side of the address table, NOT about the mechanism: the descent on \
                 the same line resolves these VAs (`pb=S:…`). Two projections of one fact, \
                 disagreeing"
            ));
        }
        // ---- 3. THE RUNS — contiguous in BOTH spaces ------------------------------------
        //
        // ⊘ VA contiguity as well as GPA contiguity, and the VA half is not decoration: each
        // run becomes ONE object placed FIXED at `run.va`, so two GPA-adjacent pages at
        // non-adjacent VAs are two mappings, not one.
        let runs = pushbuffer_runs(&resolved_pages, page);
        // ---- 4. the file offsets, from the HYPERVISOR's OWN stated layout ----------------
        let (layout, control) = {
            let held = self.ce.vmm.lock().unwrap_or_else(|e| e.into_inner());
            let Some(vmm) = held.as_ref() else {
                drop(held);
                return Some(format!(
                    "{table}\n    → NO MEMORY PLANE (between realize and `attach_ram` there \
                     is no layout to resolve against)"
                ));
            };
            let layout: Vec<_> = runs
                .iter()
                .map(|&(va, gpa, len)| (va, gpa, len, vmm.resolve_guest_ram(backing, gpa, len)))
                .collect();
            // ★★★ THE NEGATIVE CONTROL, from the SAME table in the SAME instant, and its
            // address is DERIVED — one page past the top of the highest stated run — so it is
            // outside every stated run by construction on any machine and any `-m`. A control
            // that passes because of the box it ran on is not a control.
            let top = vmm
                .stated_guest_ram(backing)
                .iter()
                .map(|r| r.gpa_end())
                .max()
                .unwrap_or(0);
            let probe = u64::try_from(top).unwrap_or(u64::MAX).saturating_add(page);
            let control = (probe, vmm.resolve_guest_ram(backing, probe, page));
            drop(held);
            (layout, control)
        };
        let control = match control.1 {
            Err(r) => format!(
                " | ✅ NEGATIVE CONTROL: gpa=0x{:x} (one page past the top of every stated \
                 run) REFUSED BY NAME as `{}`",
                control.0,
                r.name()
            ),
            Ok(bad) => format!(
                " | ⚠⚠ NEGATIVE CONTROL DID NOT FIRE: gpa=0x{:x} was ANSWERED with file \
                 offset 0x{:x}. ⊘ Read every line above with that in mind",
                control.0, bad.file_offset
            ),
        };
        // ---- 5. one grant and one pin PER CONTIGUOUS RUN ---------------------------------
        //
        // ⊘ Every run is reported, including the ones that refuse, and the report says which
        // run it is out of how many — a loop that stopped at the first refusal would make
        // "one run and it failed" and "eight runs and the second failed" the same line.
        let total = layout.len();
        let mut lines: Vec<String> = Vec::new();
        let mut pinned = 0usize;
        let mut bytes = 0u64;
        for (i, &(rva, rgpa, rlen, ref run)) in layout.iter().enumerate() {
            let at = format!(
                "{who} pb run {}/{total} va=0x{rva:x} gpa=0x{rgpa:x} len={rlen}",
                i + 1
            );
            let run = match run {
                Ok(r) => r,
                Err(r) => {
                    lines.push(format!(
                        "{at} → REFUSED BY NAME `{}` (the hypervisor stated no run covering \
                         this guest-physical address; ⊘ NOT clamped, NOT assumed identity)",
                        r.name()
                    ));
                    continue;
                }
            };
            // ★ Read-WRITE for `pin_ring_guest_ram`'s reason: `OS_DESCRIPTOR` hands RM a host
            // VA it walks with `pin_user_pages`, and a mapping the device may write needs a
            // writable one. ⊘ The narrower grant is not the safer one; it is the one that
            // fails at the ioctl with a status that will not name why.
            let grant = kayfabe_isolate::GuestRamGrant::originated_by_the_vmm(
                run.file_offset,
                rlen,
                kayfabe_vmm::Prot::ReadWrite,
            );
            match self
                .device
                .pin_guest_ram(DOORBELL_TARGET_GPU, pdb, kayfabe_rt::GpuVa(rva), grant)
            {
                Ok(p) => {
                    pinned += 1;
                    bytes += rlen;
                    lines.push(format!(
                        "{at} → file offset 0x{:x} → {} memory={:#x} host_va=0x{:x} \
                         placed_as_asked={}",
                        run.file_offset,
                        if p.already {
                            "ALREADY PINNED (idempotent replay)"
                        } else {
                            "PINNED"
                        },
                        p.memory.raw(),
                        p.host_va,
                        p.host_va == rva,
                    ));
                }
                Err(e) => lines.push(format!(
                    "{at} → file offset 0x{:x} → REFUSED {e:?}",
                    run.file_offset
                )),
            }
        }
        let verdict = if pinned == total {
            format!(
                " | ★★★★★ ALL {total} PUSHBUFFER RUN(S) PINNED, {bytes} bytes — the pages the \
                 guest's OWN GPFIFO entries name are now described to host RM and mapped FIXED \
                 at the guest's own VAs. ⊘ This says NOTHING about whether the host channel is \
                 bound to a VA space in which those VAs resolve"
            )
        } else {
            format!(
                " | ⚠ {pinned} of {total} run(s) pinned, {bytes} bytes. ⚠ `PlacementRefused` \
                 means the fixed map landed elsewhere and was UNWOUND rather than adopted; RM \
                 status `0x51` is `NV_ERR_NO_MEMORY` and is COLLISION-OR-EXHAUSTION — \
                 indistinguishable from the status alone, and neither reads as success"
            )
        };
        Some(format!(
            "{table}\n    RUNS: {} contiguous run(s) over {} page(s){verdict}{control}\n    {}",
            runs.len(),
            resolved_pages.len(),
            lines.join("\n    ")
        ))
    }

    /// One GPFIFO entry, in bytes. ⊘ Not a tunable: it is the width of the hardware
    /// structure `gpFifoEntries` counts, and the multiplier that turns the guest's declared
    /// count into an extent.
    const GP_FIFO_ENTRY_BYTES: u64 = 8;

    /// ★★ **How much of the ring this rung pins: ONE host page.**
    ///
    /// ⊘ It looks like a placeholder and is a measurement. `[measured 2026-08-10, boot
    /// `w209_ffc80f8_real`]` the ring's own descent prints its first four pages and their
    /// guest-physical addresses are **not contiguous**:
    ///
    /// ```text
    /// fbRING[p0]@va0x420064000=S:0x768a000  fbRING[p1]@va0x420065000=S:0x521c000
    /// fbRING[p2]@va0x420066000=S:0x8505000  fbRING[p3]@va0x420067000=S:0x764f000
    /// ```
    ///
    /// ⇒ Four consecutive guest **virtual** pages, four scattered guest **physical** pages.
    /// `OS_DESCRIPTOR` describes ONE contiguous host range, so "one descriptor per
    /// contiguous run" is, for this ring, **one descriptor per page** — and the leaf the
    /// walk reaches says so itself (`sz0x1000`).
    ///
    /// ★ So a larger constant here would not pin more of the ring; it would ask the layout
    /// for a range whose *guest-physical* contiguity nothing has established, and get a
    /// plausible file offset for bytes that live somewhere else. The multi-page ring is a
    /// LOOP over runs, not a bigger number — and that loop belongs with the consumer that
    /// needs the whole ring, which does not exist yet.
    const RING_PIN_BYTES: u64 = 4096;

    /// ★★★★★ **§16.82 — WITNESS THE PAGES *OUR OWN EXECUTOR* WROTE**, which G1's transport
    /// cannot see. ⊘ Armed by [`PT_WITNESS_EXEC_ENV`]; **off by default**, so an unarmed boot
    /// is byte-identical to `b6c5442`'s and is this rung's own negative control.
    ///
    /// # ★★★ The gap, MEASURED, and it is a transport gap and not an ordering one
    ///
    /// G1 takes its witness inside the framebuffer **window** write path
    /// (`kayfabe_device::plane`, the `FbWriter::Window(w)` arm) — PRAMIN, BAR1, BAR2 and
    /// nothing else. The shell's CPU copy-engine executor writes the same store through
    /// `FbStore::write_tagged(.., FbWriter::Executor)` (`kayfabe_rt::cpu_ce`) and is
    /// **structurally invisible** to it.
    ///
    /// `[measured 2026-08-11, boot `w232c_6fcedac`]` that is not a corner:
    ///
    /// > `framebuffer FIRST-WRITER census: PRAMIN 21 / BAR1 41 / BAR2 88 / EXEC 4538 /
    /// > UNATTRIBUTED 0 page(s)`
    ///
    /// **4538 of 4688 resident pages (96.8 %) were created by the executor**, and the four
    /// page-table pages of the walling channel's own tree are four of them
    /// (`L0@0x201000/byEXEC#104 … L3@0x204000/byEXEC#107`). So every leaf under them is
    /// `reachable-but-unwitnessed`, which `kayfabe_mmu::reach::ReachShadow::settle` refuses to
    /// bind **by design** (hole 2) — and the address table stays empty for that VAS.
    ///
    /// ⊘ **The contrast is the attribution, not the reasoning.** `[measured 2026-08-10, boot
    /// `w208_797a6bc_real`]` the *system* proc's CeUtils tree reads `EXEC 0 / BAR2 50`, its
    /// leaves bound, and `w209` read that ring. One transport, two populations.
    ///
    /// # ⊘ Why witnessing these is CORRECT and not a widening of the trust rule
    ///
    /// §6.1's rule is *"a leaf binds only if the guest was **seen** to write its page"*. A page
    /// our executor wrote is a page the guest asked us to write, at an address the guest chose,
    /// with bytes the guest supplied — it is *more* directly witnessed than a window write, not
    /// less. What the rule excludes is **residue**: pages nobody was seen to write. Those are
    /// exactly the pages this does **not** add, because a non-resident frame has no origin.
    ///
    /// ⊘ It claims nothing about a page being a page table. `Spine::pt_page_owner` decides that
    /// at the drain and a page nothing owns is requeued, unchanged from G1.
    ///
    /// ⚠ **First-writer, so a page created by a window and later rewritten by the executor is
    /// NOT added here** — it was already witnessed at its creation. The two transports overlap
    /// only where they should.
    ///
    /// Returns the line to print. ⊘ It prints on the disarmed arm too, saying so: an
    /// instrument that is silent when off cannot be told from one that is not wired.
    fn witness_executor_fb_pages(&self) -> String {
        let armed = selected_pt_witness_exec();
        let Some(plane) = self.plane.upgrade() else {
            return " | EXEC-WITNESS no-plane".to_string();
        };
        if !armed {
            return format!(
                " | EXEC-WITNESS DISARMED ({PT_WITNESS_EXEC_ENV} unset or `off`) — the \
                 executor's pages are NOT witnessed, which is `b6c5442`'s behaviour exactly"
            );
        }
        let Some(frames) = plane.fb_resident_frames() else {
            return " | EXEC-WITNESS ARMED but the store cannot enumerate frames".to_string();
        };
        let total = frames.len();
        let pages: Vec<u64> = frames
            .into_iter()
            .filter(|&p| {
                plane
                    .fb_page_origin(p)
                    .is_some_and(|o| o.by == kayfabe_device::fbwin::FbWriter::Executor)
            })
            .collect();
        let exec = pages.len();
        // ⊘ The SAME queue G1's window writes go into, so the drain, the attribution, the
        // requeue and the cap are one mechanism with one set of counters — never a second
        // path that could be right about a page the first is wrong about.
        let refused = plane.requeue_pt_witness(pages);
        format!(
            " | EXEC-WITNESS ARMED resident={total} by-executor={exec} refused-at-cap={refused}"
        )
    }

    fn decode_cpu_pt_writes(&self) -> String {
        let Some(plane) = self.plane.upgrade() else {
            return String::new();
        };
        let mut pending = plane.drain_pt_witness();
        let drained = pending.len();
        if drained == 0 {
            // ⊘ Printed, not skipped. `[measured 2026-08-10, boot `w208_797a6bc_real`]` the
            // first-writer census reads `BAR2 50 / PRAMIN 21 / EXEC 0`, so a drain of zero
            // on the real arm would say the witness is not on the path the census names —
            // a finding about **this instrument**, and a missing line could not carry it.
            return " | PT-DECODE drained=0 (the CPU transport wrote nothing this window)"
                .to_string();
        }
        // ★ Zero-sized and stateless (`Ga10xGmmu` is a unit struct), so this is the same
        // *value* the composition root installed with `plane.set_mmu` — not a second
        // format that could drift from it.
        let fmt = kayfabe_chips::Ga10xGmmu::new();
        let (mut latched, mut vas_gone, mut rounds) = (0usize, 0usize, 0usize);
        // ⊘ A local tally rather than a folded `PtDecodeOutcome`, for one reason that is
        // about linkage and not about style: this crate does not depend on `kayfabe-fwd`
        // and must not start to. The shipped archive's edge set is itself a security
        // surface (see this crate's manifest on `host-isolates`), and a diagnostic is not a
        // reason to widen it.
        let mut acc = PtDecodeTally::default();
        while rounds < PT_DECODE_ROUNDS && !pending.is_empty() {
            let w = self
                .device
                .witness_cpu_pt_pages(DOORBELL_TARGET_GPU, &pending);
            latched += w.latched;
            vas_gone += w.vas_gone;
            pending = w.unattributed;
            if w.procs.is_empty() {
                // Nothing became attributable this round, so nothing will next round
                // either — the index only grows from a decode, and no decode ran.
                break;
            }
            rounds += 1;
            for pid in w.procs {
                let mut fb = plane.pt_bytes();
                let Some(out) = self.device.decode_pt_writes_from(pid, &fmt, &mut fb) else {
                    continue;
                };
                acc.bound += out.bound;
                acc.unchanged += out.unchanged;
                acc.repointed += out.repointed;
                acc.unbound += out.unbound;
                acc.learned += out.meta_learned;
                acc.meta_refused += out.meta_refused;
                acc.published += out.pages_published;
                acc.publish_refused += out.pages_publish_refused;
                acc.unwitnessed += out.unwitnessed;
                acc.unreachable += out.unreachable;
                acc.sparse += out.sparse;
                acc.pass_vas_gone += out.vas_gone;
                acc.dropped += out.dropped.len();
                acc.refusals += out.refusals.len();
                acc.faults += out.faults.len();
                acc.reach_faults += out.reach_faults.len();
                acc.retired += out.retired.len();
                // ★ The FIRST fault, whole, beside the count. A count says a subtree was
                // unreadable; only the fault says which address and why, and this pass has
                // three different kinds that a single number cannot tell apart.
                if acc.first_fault.is_none() {
                    if let Some(f) = out.faults.first() {
                        acc.first_fault = Some(format!("{f:?}"));
                    } else if let Some(r) = out.refusals.first() {
                        acc.first_fault = Some(format!("{r:?}"));
                    } else if let Some(r) = out.reach_faults.first() {
                        acc.first_fault = Some(format!("{r:?}"));
                    }
                }
            }
        }
        // ⊘ THE LEFTOVERS GO BACK. A page the index cannot name an owner for is not a page
        // that was not written, and the witness is the only record that it was.
        let requeue_refused = plane.requeue_pt_witness(pending.iter().copied());
        let st = plane.pt_witness_stats();
        format!(
            " | PT-DECODE drained={drained} latched={latched} unowned_vas={vas_gone} \
             requeued={} rounds={rounds} → bound={} unchanged={} repointed={} unbound={} \
             learned={} published={}/{} meta_refused={} unwitnessed={} unreachable={} \
             sparse={} dropped={} refusals={} faults={} reach_faults={} retired={} \
             pass_vas_gone={} first={} [witness writes={} pending={} refused={}+{}]",
            pending.len(),
            acc.bound,
            acc.unchanged,
            acc.repointed,
            acc.unbound,
            acc.learned,
            acc.published,
            acc.publish_refused,
            acc.meta_refused,
            acc.unwitnessed,
            acc.unreachable,
            acc.sparse,
            acc.dropped,
            acc.refusals,
            acc.faults,
            acc.reach_faults,
            acc.retired,
            acc.pass_vas_gone,
            acc.first_fault.as_deref().unwrap_or("NONE"),
            st.writes,
            st.pending,
            st.refused,
            requeue_refused,
        )
    }

    fn addressing_probe_facts(&self, facts: kayfabe_rt::device::CeChannelFacts) -> String {
        let Some(plane) = self.plane.upgrade() else {
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
        // ★★★★★ LEG B — and the fourth string is the one that changes what is buildable:
        // `phys=` is the address the guest's OWN KERNEL resolved this channel's USERD to,
        // off the same params, with `off=` already folded in. ⊘ Printed beside `h`/`off`
        // and never instead of them: the declaration and the resolution are two parties'
        // numbers about one object, and a boot that shows only the second cannot tell a
        // mis-located params block from a channel that declared no USERD.
        let userd = facts.userd.map_or_else(
            || " userd=UNREADABLE-AT-THIS-BOUNDARY".to_string(),
            |u| {
                format!(
                    " userd=h0x{:x}/off0x{:x}/{}",
                    u.handle,
                    u.offset,
                    u.resolved_tag()
                )
            },
        );
        // ★★★★ §16.25 — THE ROUTES, and the CENSUS TO READ THEM AGAINST.
        //
        // `[measured 2026-08-08, boot `s23_10a769c_cup2`]` 15 of 24 doorbells refused
        // `FwdFault::NoVas(ChanId(3))` and printed `vas=NONE-DECLARED dec=NONE` — a string
        // that reports only that **the channel itself** declared no `hVASpace`.
        // `project::resolve_channel_vas` has THREE routes (own → CtxShare's → parent TSG's)
        // and all three returned `None`; nothing said which of them ran, what each hit, or
        // whether the CtxShare and TSG objects were found at all. A null that cannot tell
        // its three causes apart sends its reader to guess, and four consecutive rungs were
        // framed on guesses.
        //
        // ⊘ The census is here and not only on the refused channel because **nine doorbells
        // in that same boot were SERVED**. Whatever resolves their VA space is the control:
        // any field that reads the same on a served channel and a refused one is not the
        // field that explains the refusal. ⚠ It is emitted on the REFUSAL path only.
        // ★★★★ §16.28 — and WHICH of the two possible producers filled `vas=`. The route
        // string already carries `dev=dev-default(...)`, but `vas=` alone cannot say
        // whether it came from a live VASpace resource or from route 4's *name*, and those
        // two mean different things about what exists — so the discriminator is printed
        // explicitly rather than left to be inferred from a sibling field.
        //
        // ⊘ `devdef=NONE` on a channel that took a declared route is the ordinary case and
        // says so; it is not an absence of information.
        let devdef = facts.vaspace_device_default.map_or_else(
            || " devdef=NONE".to_string(),
            |h| format!(" devdef=0x{h:x}"),
        );
        let routes = format!("{devdef} route[{}]", facts.vas_route);
        let census = self.vas_census_line(facts.chan);
        // ★★★★ §16.27 — and WHAT THE WALLING CHANNEL'S OWN NAMESPACE HOLDS.
        //
        // §16.25 measured the shape (declares nothing, parent is a Device) and cited RM's
        // answer for it (the DEVICE'S DEFAULT VA SPACE, `vaspace.c:178`). The one thing it
        // could not say is whether that default VASpace is an object we were sent — which
        // decides whether the missing fourth route is a LOOKUP or a MINT. ⊘ The refusal
        // never enumerated the namespace, so "no VASpace was mentioned" was unmeasured,
        // not empty. This enumerates it.
        let ns = self.namespace_census_line(facts.client);
        let (vaspace, ring_va) = match (facts.vaspace, facts.ring_va) {
            (Some(v), Some(r)) => (v, r),
            (None, Some(r)) => {
                return format!(
                    " | c=0x{:x} vas=NONE-DECLARED dec={declared}{userd} ring=0x{r:x}{routes}{census}{ns}",
                    facts.client
                );
            }
            (Some(v), None) => {
                return format!(
                    " | c=0x{:x} vas=0x{v:x} dec={declared}{userd} ring=NONE-DECLARED{routes}{census}{ns}",
                    facts.client
                );
            }
            (None, None) => {
                return format!(
                    " | c=0x{:x} vas=NONE-DECLARED dec={declared}{userd} ring=NONE-DECLARED{routes}{census}{ns}",
                    facts.client
                );
            }
        };
        let demand = kayfabe_device::ceresolve::Demand::from_doorbell;
        // ★★★★ §16.64 — THE SAME SELECTION THE EXECUTOR MADE, not a second one.
        //
        // `[measured 2026-08-10, boot `s49_57bd756_declroot2`]` this line read
        // `plane.published_root(...)` while the executor beside it had two sources, and the
        // boot printed `root=none rng=NOPUB row=ABSENT-FROM-ROOT-TABLE` on doorbells it had
        // just **served** — 93 of them. ⊘ A probe that re-derives what it describes can
        // disagree with it, and this one did, in the direction that reads as "still broken".
        let sel = SharedDoorbell::doorbell_root(
            &plane,
            facts.client,
            vaspace,
            facts.vas_pdb.map(|p| p.0),
        );
        // ★ WHICH SOURCE ANSWERED, printed. `root=` alone cannot say whether the geometry
        // beside it is the guest's own published `pageShift` or one derived from the
        // installed format, and those are different claims.
        let rootsrc = match sel {
            DoorbellRoot::Published(_) => " rootsrc=published",
            DoorbellRoot::Declared(_) => " rootsrc=declared(object-model)",
            DoorbellRoot::Absent => " rootsrc=NONE",
            DoorbellRoot::Underivable(..) => " rootsrc=UNDERIVABLE",
        };
        let root = match sel {
            DoorbellRoot::Published(r) | DoorbellRoot::Declared(r) => Some(r),
            DoorbellRoot::Absent | DoorbellRoot::Underivable(..) => None,
        };
        // ★★★★ §16.6 — THE WHOLE ROW THE LOOKUP CHOSE, read out of the SAME table
        // `published_root` reads. Six boots named this pair in a refusal and printed no
        // body for it; see [`publication_row`] for what each field decides.
        let pubs = plane.gvas_publications();
        let row = publication_row(&pubs, facts.client, vaspace);
        // ★★★★ §16.8's rung: what does OUR framebuffer actually hold at the addresses this
        // row published, and at a WORKING row's? See [`fb_level_dump`].
        let fbdump = fb_dump_pair(&plane, &pubs, facts.client, vaspace);
        // ★★★★★ **LEG B — GP_GET, READ WHERE HARDWARE WRITES IT.** See `fb_userd_cursors`.
        let fbuserd = fb_userd_cursors(&plane, facts.userd);
        // ★★★★ §16.10's rung: which SLOT the descent consumes at every level, and what that
        // slot says. §16.9 dumped entry 0 of each level, and entry 0 is not the entry this
        // walk looks at. See `kayfabe_device::ceresolve::walk_trace` — the same decoder,
        // deliberately not a second one.
        //
        // ⊘ Printed BESIDE `rng=`, which is `resolve`'s own answer for the same address, so
        // the two projections are compared by a reader rather than trusted apart. A trace
        // whose terminal leaf disagrees with `rng=` is itself the finding.
        let walk = root.as_ref().map_or_else(
            || " walk=NO-ROOT".to_string(),
            |r| plane.walk_trace_from_root(r, ring_va),
        );
        let ring = root
            .as_ref()
            .map_or(kayfabe_device::ceresolve::CeResolve::NoPublication, |r| {
                plane.resolve_va_from_root(r, ring_va, demand())
            });
        // ★★★★ §16.12 — THE RING'S OWN PAGE. §16.10 proved the walk lands on `V:0x20000`
        // correctly; the open question is whether OUR framebuffer has ever had a byte
        // written there. ⊘ Addressed by the resolution's OWN answer, never by a literal:
        // the leaf is a per-boot address and hard-coding one would read correctly on
        // exactly one boot (§16.9's control-row argument, one level in).
        // ★★★★ §16.17 — EVERY PAGE THE RING SPANS, and the semaphore's page beside them.
        // §16.16 dumped the leaf page alone and called the result "the ring's frame"; a
        // 1024-entry ring is 8 KiB and occupies TWO pages, so that sentence was true of
        // half the ring. See [`RING_PAGE_DUMPS`].
        let ringpage = self.ring_pages(root.as_ref(), ring_va, facts.ring_entries);
        let fin = root
            .as_ref()
            .map_or(kayfabe_device::ceresolve::CeResolve::NoPublication, |r| {
                plane.resolve_va_from_root(
                    r,
                    ring_va.wrapping_add(FINISH_PAYLOAD_FROM_RING),
                    demand(),
                )
            });
        // The pushbuffer the entry AT THE CURSOR points at — read the entry, decode it, walk
        // its target. ⊘ Every step can fail and every failure is reported as itself: a ring
        // that would not read and a ring that read as a malformed entry are different facts.
        //
        // ★★★★ **AT THE CURSOR, not at index 0.** `[measured 2026-08-09, boot
        // s19_1dfde1b_cup2]` token `0x00010003` was SERVED at `14:15:46.427` and REFUSED at
        // `14:15:46.624` — the same channel, twice, and the ring held **two** non-zero
        // entries (`[0]=0x…0120000000 [1]=0x…0120400000`). This probe read index `0`
        // unconditionally, so the refusal's whole account of the submission —
        // `gp0=0x120000000+0x28 pb=S:0x41400000` — described the entry the **earlier,
        // successful** doorbell had already consumed. ⊘ A diagnostic pointed at the wrong
        // object does not merely fail to help; it corroborates whatever is read into it.
        // The index is printed so the pairing is on the page rather than assumed.
        let idx = if facts.ring_entries == 0 {
            0
        } else {
            self.ce
                .cursors
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&(facts.proc.0, facts.chan.0))
                .map_or(0, |c| c.next % facts.ring_entries)
        };
        let gp_va = ring_va.wrapping_add(u64::from(idx) * PROBE_RING_BYTES as u64);
        let mut gp = [0u8; PROBE_RING_BYTES];
        let pb = match root.as_ref().map_or(
            Err(kayfabe_device::plane::PublishedVaRead::Unresolved(
                kayfabe_device::ceresolve::CeResolve::NoPublication,
            )),
            |r| plane.read_va_from_root(r, gp_va, &mut gp, demand()),
        ) {
            Err(e) => format!("gp[{idx}]@0x{gp_va:x} ringread={}", e.describe()),
            Ok(_) => match kayfabe_abi::submit::gp_entry_decode(u64::from_le_bytes(gp)) {
                None => format!(
                    "gp[{idx}]@0x{gp_va:x}=0x{:016x} NOT-A-GP-ENTRY",
                    u64::from_le_bytes(gp)
                ),
                Some(d) => format!(
                    "gp[{idx}]@0x{gp_va:x}=0x{:x}+{:#x} pb={} {}",
                    d.gpu_va,
                    d.len_bytes,
                    plane
                        .resolve_va_from_root(
                            root.as_ref().expect("pb only decodes behind a root"),
                            d.gpu_va,
                            demand(),
                        )
                        .tag(),
                    self.push_headers(
                        &plane,
                        root.as_ref().expect("pb only decodes behind a root"),
                        d.gpu_va,
                        d.len_bytes,
                    )
                ),
            },
        };
        format!(
            " | c=0x{:x} vas=0x{vaspace:x} dec={declared}{userd}{fbuserd} root={}{rootsrc} ring=0x{ring_va:x} rng={} fin={} {pb}{}{row}{fbdump}{ringpage} walk:{walk}",
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
            self.ring_scan(root.as_ref(), ring_va, facts.ring_entries),
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
    /// ★★★★ **EVERY framebuffer page the ring occupies, plus the semaphore's** — §16.17.
    ///
    /// # ⊘⊘ The defect this repairs, and it sat under the campaign's headline claim
    ///
    /// `[measured 2026-08-09, boot `res1_fc21926`]` the report read
    /// `fbRING@0x20000 … nz0/4096 resN-NEVER-WRITTEN` and that line was relayed — by me —
    /// as *"the ring's frame was never written"*. The channel declares **1024** entries;
    /// 1024 x 8 = **8192 bytes**, so entries 0-511 live in `0x20000` and entries 512-1023
    /// live in **`0x21000`, which nothing ever asked about**. The claim was true of half
    /// the ring and was read as a statement about all of it.
    ///
    /// ★ **A third address in the same allocation**, and it discriminates independently:
    /// the finishPayload semaphore at [`FINISH_PAYLOAD_FROM_RING`] (`ring + 0x8004`) — which
    /// is itself corroboration that the ring is 1024 entries, since `0x8000` is exactly
    /// 1024 x 8 and the semaphore sits immediately past the end. If the guest wrote the
    /// semaphore area, **that** page is resident even when the ring pages are not, which
    /// separates *"this allocation is entirely unwritten"* from *"we are reading the wrong
    /// address for the ring specifically"*.
    ///
    /// ⊘ Each page is asked for **by the resolver's own answer**, never by adding 0x1000 to
    /// a previous result: a ring whose pages are not contiguous in the framebuffer is
    /// exactly the case a literal stride would silently mis-report. ⊘ And the count dumped
    /// is printed beside the count required, so a truncation at [`RING_PAGE_DUMPS`] is
    /// visible rather than silent — this whole method exists because a bound went unread.
    /// ★★★★ **The refused submission's method headers, FRAMED** — subchannel, method offset,
    /// form and argument count for each, straight out of the pushbuffer the refused GPFIFO
    /// entry points at.
    ///
    /// # ⊘ It exists because `opaque == methods` is a count, and a count names no engine
    ///
    /// `[measured 2026-08-09, boot s20_25295aa_cup2]` the refusal reported
    /// `methods: 7, opaque: 7, set_object: None` — seven method pairs read, and the chip's
    /// codec recognized **not one** of them. That is a real finding and it is still not an
    /// answer: *"these are another engine's methods"* and *"we framed the bytes wrong and
    /// every header is garbage"* both produce `7/7`, and they are opposite bugs. The
    /// subchannel and the method offset separate them in one line — CE's `LAUNCH_DMA` is
    /// `0x300`, the host FIFO's `SEM_ADDR_LO` is `0x5c`, and a mis-framed read produces
    /// neither but nonsense that climbs.
    ///
    /// ⊘ **OBSERVER.** It reads guest memory the refusal has already been decided over,
    /// through the same resolver, on the same doorbell demand, and returns text. It latches
    /// nothing and it cannot relax anything.
    ///
    /// ★ Framed by [`kayfabe_abi::submit::method_header_decode`] — the crate that owns the
    /// wire format — and **not** by a second parser written here. A header it cannot decode
    /// stops the walk and says so, because continuing past one would invent the offsets of
    /// everything after it.
    fn push_headers(
        &self,
        plane: &kayfabe_device::RegPlane,
        root: &kayfabe_device::ceresolve::VasRoot,
        push_va: u64,
        len_bytes: u64,
    ) -> String {
        let take = (len_bytes as usize).min(PROBE_PUSH_BYTES) & !3;
        if take == 0 {
            return "pbm=EMPTY-RANGE".to_string();
        }
        let mut buf = vec![0u8; take];
        let demand = kayfabe_device::ceresolve::Demand::from_doorbell();
        if let Err(e) = plane.read_va_from_root(root, push_va, &mut buf, demand) {
            return format!("pbm=UNREADABLE({})", e.describe());
        }
        let words: Vec<u32> = buf
            .chunks_exact(4)
            .map(|w| u32::from_le_bytes(w.try_into().unwrap_or([0; 4])))
            .collect();
        let mut out = format!("pbm[{}w of {}B]:", words.len(), len_bytes);
        let mut i = 0usize;
        let mut shown = 0usize;
        while i < words.len() && shown < PROBE_PUSH_METHODS {
            let h = words[i];
            let Some(d) = kayfabe_abi::submit::method_header_decode(h) else {
                out.push_str(&format!(" [{shown}]=0x{h:08x}/UNDECODABLE-HEADER"));
                break;
            };
            out.push_str(&format!(
                " [{shown}]sub{}/m0x{:x}/{:?}/n{}",
                d.subchannel, d.method, d.form, d.arg_words
            ));
            // ⊘ The first argument, for the runs where it is the whole fact (a `SET_OBJECT`'s
            // class, a semaphore's address half). Printed as itself, never interpreted here.
            if d.arg_words > 0 && i + 1 < words.len() {
                out.push_str(&format!("=0x{:x}", words[i + 1]));
            }
            i += 1 + d.arg_words;
            shown += 1;
        }
        out
    }

    fn ring_pages(
        &self,
        root: Option<&kayfabe_device::ceresolve::VasRoot>,
        ring_va: u64,
        entries: u32,
    ) -> String {
        let Some(plane) = self.plane.upgrade() else {
            return String::new();
        };
        let demand = kayfabe_device::ceresolve::Demand::from_doorbell();
        // How many 4 KiB pages the DECLARED ring spans, rounded up. ⊘ Derived from the
        // guest's own entry count, never from a constant.
        let span = (u64::from(entries) * PROBE_RING_BYTES as u64)
            .div_ceil(4096)
            .max(1);
        let want = span as usize;
        let take = want.min(RING_PAGE_DUMPS);
        let mut out = String::new();
        // ⊘ Checked ONCE, before the loop: a per-iteration check that returns would report
        // "no root" as if it were a property of page 0.
        let Some(root) = root else {
            return " fbRING=NO-ROOT (neither source has a page-directory root)".to_string();
        };
        for i in 0..take {
            let va = ring_va.wrapping_add((i as u64) * 4096);
            let r = plane.resolve_va_from_root(root, va, demand);
            match r.vidmem_phys() {
                // ⊘ A page that does not resolve to video memory is reported as itself and
                // not skipped: "the ring's second page resolves elsewhere" is a finding.
                None => {
                    out.push_str(&format!(" fbRING[p{i}]@va0x{va:x}=NOT-VIDMEM({})", r.tag()));
                }
                Some(phys) => out.push_str(&fb_level_dump(&plane, &format!("fbRING[p{i}]"), phys)),
            }
        }
        if take < want {
            out.push_str(&format!(
                " ⊘ BOUNDED-DUMP: {take} of {want} page(s) the ring spans"
            ));
        }
        // ★ The semaphore's page — the third address, and the one that can be resident
        // while both ring pages are not.
        let sem_va = ring_va.wrapping_add(FINISH_PAYLOAD_FROM_RING);
        let sem = plane.resolve_va_from_root(root, sem_va, demand);
        match sem.vidmem_phys() {
            None => out.push_str(&format!(" fbFIN@va0x{sem_va:x}=NOT-VIDMEM({})", sem.tag())),
            Some(phys) => out.push_str(&fb_level_dump(&plane, "fbFIN", phys)),
        }
        out
    }

    fn ring_scan(
        &self,
        root: Option<&kayfabe_device::ceresolve::VasRoot>,
        ring_va: u64,
        entries: u32,
    ) -> String {
        let Some(plane) = self.plane.upgrade() else {
            return String::new();
        };
        let n = (entries as usize).clamp(1, RING_SCAN_ENTRIES);
        let demand = kayfabe_device::ceresolve::Demand::from_doorbell();
        // ⊘ §16.63's rule, one source further out: a scan with no root read NOTHING, and
        // must not render as a scan that read zeroes.
        let Some(root) = root else {
            return " scan=NO-ROOT — nothing was read, so this says NOTHING about the ring"
                .to_string();
        };
        let mut nonzero: Vec<String> = Vec::new();
        let mut unread = 0usize;
        for i in 0..n {
            let at = ring_va.wrapping_add((i * PROBE_RING_BYTES) as u64);
            let mut gp = [0u8; PROBE_RING_BYTES];
            match plane.read_va_from_root(root, at, &mut gp, demand) {
                Err(_) => unread += 1,
                Ok(_) => {
                    let raw = u64::from_le_bytes(gp);
                    if raw != 0 && nonzero.len() < RING_SCAN_REPORT {
                        nonzero.push(format!("[{i}]=0x{raw:016x}"));
                    }
                }
            }
        }
        // ⊘⊘ THE SENTENCE CHANGES WHEN THE READING IS PARTIAL — printing the bound was not
        // enough. `[measured 2026-08-09]` this line honestly said `scan=64/1024 declared …
        // nonzero=NONE` and was still read as "the ring is empty", licensing a claim about
        // a whole ring from one sixteenth of it. A reader must not have to do the division,
        // so a truncated scan now says so in words and a complete one says THAT in words
        // too — otherwise the absence of a warning is itself ambiguous.
        // ★★★★★ **NOTHING READ IS NOT EVERYTHING ZERO — and this line asserted the second
        // when the first was true.** `[measured 2026-08-10, boots s45/s46/s47/s48]` every
        // one of them printed
        //
        // ```text
        // scan=1024/1024 declared (COMPLETE: every declared entry was read), unread=1024,
        //   nonzero=NONE — every scanned entry is ZERO
        // ```
        //
        // and **both** parenthetical clauses were false. `read_published_va` answers
        // `Err(Unresolved(NoPublication))` *before it touches any store*
        // (`kayfabe-device/src/plane.rs`, `published_root` → early return), so a
        // `NoPublication` makes all `n` reads fail: `unread == n`, and `nonzero` is empty
        // because nothing was ever appended to it, not because the entries were zero. The
        // old `coverage` was computed from the **loop bound alone**, with no reference to
        // `unread`, so it said `COMPLETE` about a scan that read nothing.
        //
        // ⇒ The line restated `NoPublication` as if it were independent evidence about the
        // ring — the same finding twice, the second time wearing a different instrument's
        // clothes (`measure_at_the_boundary_not_inside`: two of your own computations
        // agreeing is not corroboration). ⚠ It was cited as evidence in
        // `execution_plane_increments.md` §16.57.3 and §16.60.4 before it was checked.
        //
        // ⊘ The guard is `unread == n`, not `unread > 0`: a partial read really did scan
        // `n - unread` entries and *those* are legitimately reported.
        ring_scan_sentence(n, entries, unread, &nonzero)
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

/// ★★★★★ **The `ring_scan` sentence — a free function so it can be tested without a plane,
/// because it asserted a FALSEHOOD on four committed boots.**
///
/// `[measured 2026-08-10, boots s45_748a207_tsgsched, s46_1a9e93c_abi35,
/// s47_81582e3_ctxsw and s48_4f5b357_cwait]` every one of them printed
///
/// ```text
/// scan=1024/1024 declared (COMPLETE: every declared entry was read), unread=1024,
///   nonzero=NONE — every scanned entry is ZERO
/// ```
///
/// and **both** parenthetical clauses were false.
/// [`kayfabe_device::plane::MemoryPlane::read_published_va`] answers
/// `Err(Unresolved(NoPublication))` *before it touches any store*, so a `NoPublication`
/// makes all `n` reads fail: `unread == n`, and `nonzero` is empty because nothing was ever
/// appended to it — **not** because the entries were zero. The old `coverage` clause was
/// computed from the **loop bound alone**, with no reference to `unread`, so it said
/// `COMPLETE` about a scan that read nothing.
///
/// ⇒ The line restated `CeResolve::NoPublication` as if it were independent evidence about
/// the ring: the same finding twice, the second time wearing a different instrument's
/// clothes (`measure_at_the_boundary_not_inside` — two of your own computations agreeing is
/// not corroboration). ⚠ And it was **cited as evidence** in
/// `docs/design/execution_plane_increments.md` §16.57.3 and §16.60.4 before anyone checked
/// what produced it.
///
/// ⊘ The guard is `unread == n`, **not** `unread > 0`: a partial read really did scan
/// `n - unread` entries, and those are legitimately reported — with the denominator said
/// out loud, which is `RING_SCAN_ENTRIES`' own rule.
fn ring_scan_sentence(n: usize, entries: u32, unread: usize, nonzero: &[String]) -> String {
    if unread == n {
        return format!(
            " ⊘ NOTHING WAS READ: all {n} of {entries} declared entries failed to resolve, \
             so this scan says NOTHING about the ring's contents — it is the resolution \
             failure above, restated"
        );
    }
    let coverage = if (n as u32) < entries {
        format!(
            " ⊘ BOUNDED-READING: {n} of {entries} entries — a write at any entry >= {n} is INVISIBLE here"
        )
    } else {
        " (COMPLETE: every declared entry was read)".to_string()
    };
    let found = if !nonzero.is_empty() {
        nonzero.join(" ")
    } else if unread == 0 {
        "NONE — every scanned entry is ZERO".to_string()
    } else {
        // ⊘ The denominator is the entries that RESOLVED, never the loop bound: saying
        // "every scanned entry is ZERO" over a partial read is the same conflation this
        // function exists to refuse, one order of magnitude smaller.
        format!("NONE among the {} entries that RESOLVED", n - unread)
    };
    format!(" scan={n}/{entries} declared{coverage}, unread={unread}, nonzero={found}")
}

/// How many GPFIFO entries [`SharedDoorbell::ring_scan`] reads.
///
/// # ⊘⊘ It was 64 against a channel that declared 1024, and that is 6.25 %
///
/// `[measured 2026-08-09, boots `res1_fc21926` and `s16_5fcd259`]` the refusing UVM channel
/// declares `entries: 1024` and the scan reported `scan=64/1024 declared, unread=0,
/// nonzero=NONE`. ★ The line was **honest** — it printed its own bound, exactly as the
/// discipline requires — and it was **still** read, by a careful reader, as *"the ring is
/// empty"*. It licensed a headline claim about a **whole ring** from **one sixteenth** of
/// it: a guest write at any entry ≥ 64 was structurally invisible.
///
/// ★★★ The transferable rule, and it is stronger than "print your precondition": when the
/// bound and the declared size **differ**, the sentence itself must change — a reader
/// should not have to do the division. [`SharedDoorbell::ring_scan`] now says
/// `BOUNDED-READING` in words when it truncates.
///
/// ⚠ Still a bound and not the guest's number, for the original reason: the entry count is
/// guest-supplied and a diagnostic must not become a guest-sized read. 4096 entries is
/// 32 KiB of probe reads, covers the 1024 every channel this campaign has seen declares,
/// and leaves the refusal arm reachable rather than decorative.
const RING_SCAN_ENTRIES: usize = 4096;

/// ★★ **How many DISTINCT pushbuffer extents one doorbell's pin pass will take**
/// ([`SharedDoorbell::pin_pushbuffer_guest_ram`]).
///
/// ⊘ A bound, not a tuning knob, and it exists because the count is **guest-influenced**: a
/// 4096-entry ring may name 4096 different pushbuffers, and each one costs an isolate round
/// trip on the vCPU's own MMIO trap. `[measured, w263, all 8 walling channels]` each ring holds
/// exactly **one** non-zero entry (`nonzero=[0]=0x…`, `scan=1024/1024`), so 64 is two orders
/// of margin over the only workload anyone has measured — ⚠ which also means a green
/// *"never capped"* is evidence the cap was **not exercised**, never that it is right.
///
/// ⚠ Overflow is **reported with the count dropped and the first dropped VA**. A cap that
/// truncated silently would be a false green in the same class as a `dlen=0` oracle row: the
/// artefact reads as complete and only its content says otherwise.
const PUSHBUF_MAX_EXTENTS: usize = 64;

/// ★★ **How many host pages one doorbell's pin pass will describe.** 512 × 4 KiB = 2 MiB.
///
/// ⊘ Separate from [`PUSHBUF_MAX_EXTENTS`] because they bound different guest freedoms: one
/// long extent and many short ones cost the same table lookups per *page*, and a cap on
/// extents alone would let a single entry with a 21-bit `LENGTH` field ask for 8 MiB of pins.
/// Overflow is reported the same way and for the same reason.
const PUSHBUF_MAX_PAGES: usize = 512;

/// How many refused addresses [`SharedDoorbell::pin_pushbuffer_guest_ram`] names in its
/// report before it stops naming them and only counts.
///
/// ⊘ The **count** is never truncated — only the sample is. A line that said "some pages
/// missed" without a number is the shape that lets a partial pass read as a whole one.
const PUSHBUF_REPORT: usize = 4;

/// ★★★ **Every host page the named pushbuffer extents touch, and what the cap DROPPED.**
///
/// The pure half of [`SharedDoorbell::pin_pushbuffer_guest_ram`]'s step 2, extracted so the
/// bound is testable without a plane, a hypervisor or a GPU — which is the only way the
/// **overflow** arm can be exercised at all. `[measured, w263]` every ring in that boot names
/// one extent, so the cap is unreachable from the only live workload anyone has;
/// ⊘ a green *"never capped"* on a boot is evidence the cap was **not exercised**.
///
/// Returns `(pages, dropped, first_dropped)`. ⚠ `first_dropped` is the first VA that did not
/// make it in, so *"some were dropped"* can never be printed without one of them named.
///
/// ⊘ **Every address is derived from its own extent.** No stride is inferred and none is
/// applied: an extent that begins mid-page contributes the page it begins in, and one that
/// spans a boundary contributes both. `w263`'s `0x200000` spacing is an OBSERVATION about one
/// workload and this function cannot see it.
fn pushbuffer_pages(
    extents: &[(usize, u64, u64)],
    page: u64,
) -> (std::collections::BTreeSet<u64>, usize, Option<u64>) {
    let mut pages: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
    let mut dropped = 0usize;
    let mut first_dropped: Option<u64> = None;
    for &(_, va, len) in extents {
        // ⊘ A zero-length extent is unreachable from `gp_entry_decode` (it answers `None` for
        // `LENGTH == 0`) but is handled rather than assumed away: `len - 1` on a zero would
        // wrap, and the wrapped value would name a page at the top of the address space.
        if len == 0 {
            continue;
        }
        let first = va & !(page - 1);
        let last = va.saturating_add(len - 1) & !(page - 1);
        let mut p = first;
        loop {
            // ★ The cap counts DISTINCT pages, so a page already admitted is never charged
            // twice — otherwise two extents sharing a page would spend two slots and the
            // number in the report would not be the number of pages.
            if pages.contains(&p) {
                // already admitted
            } else if pages.len() >= PUSHBUF_MAX_PAGES {
                dropped += 1;
                first_dropped.get_or_insert(p);
            } else {
                pages.insert(p);
            }
            if p >= last {
                break;
            }
            p = p.saturating_add(page);
        }
    }
    (pages, dropped, first_dropped)
}

/// ★★★ **Coalesce VA-sorted `(va, gpa)` pages into runs contiguous in BOTH spaces.**
///
/// One run becomes one `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` placed **FIXED at `run.va`**, so
/// the VA half of the test is not decoration: two GPA-adjacent pages at non-adjacent VAs are
/// two mappings and merging them would describe bytes at an address the guest did not name.
///
/// ⊘ Extracted for [`pushbuffer_pages`]' reason — and because `RING_PIN_BYTES`' own docs
/// measure four consecutive guest **virtual** pages resolving to four scattered guest
/// **physical** ones, so the many-runs case is the expected one and must be pinned by a test
/// rather than hoped for.
fn pushbuffer_runs(pages: &[(u64, u64)], page: u64) -> Vec<(u64, u64, u64)> {
    let mut runs: Vec<(u64, u64, u64)> = Vec::new();
    for &(va, gpa) in pages {
        match runs.last_mut() {
            Some((rva, rgpa, rlen)) if *rva + *rlen == va && *rgpa + *rlen == gpa => *rlen += page,
            _ => runs.push((va, gpa, page)),
        }
    }
    runs
}

/// How many framebuffer pages [`SharedDoorbell::ring_pages`] will dump for one ring.
///
/// ⊘⊘ **The ring SPANS MORE THAN ONE PAGE and we probed exactly one.** 1024 entries x 8
/// bytes = **8192**, so entries 0-511 live in the leaf page and entries 512-1023 live in
/// the **next** one. `[measured 2026-08-09, boot `res1_fc21926`]` the report said
/// `fbRING@0x20000 … resN-NEVER-WRITTEN` and **never asked about `0x21000`** — so
/// *"the ring's frame was never written"* was a statement about **half the ring**.
///
/// ★ 4 pages covers a 2048-entry ring; the count actually dumped is printed beside the
/// count required, so a truncation is visible rather than silent.
const RING_PAGE_DUMPS: usize = 4;

/// How many non-zero entries the scan NAMES. The rest are still counted by the scan's own
/// range, which is printed beside it.
const RING_SCAN_REPORT: usize = 4;

/// What joining ONE framebuffer leaf produced, as the two call sites need it.
///
/// ⊘ `installed` is `Some(len)` **only** when step 3 returned `Ok` — i.e. when the guest's
/// framebuffer window actually points at the joined pages. A leaf that reached step 1 and
/// refused at step 3 has a real host object and a guest view that is still the shell's own
/// `SparseFb`, which is *two memories* — and rendering that as a join is exactly the row
/// `fb-join` used to write and `w260` removed.
#[cfg(feature = "host-isolates")]
#[derive(Debug, Clone, Copy)]
struct JoinedLeaf {
    /// Where RM placed the host object. Compared against the leaf's own VA by the caller.
    host_va: u64,
    /// The host memory object — ★ **this is the handle `GuestRing::memory` wants.**
    memory: u64,
    /// `Some(len)` once the view is live; see the type doc.
    installed: Option<u64>,
}

/// ★★★★★ **JOIN ONE FRAMEBUFFER LEAF — the four steps, in the one order that is safe**, so
/// that the census source and the ring source cannot come to disagree about what a join is.
///
/// ⊘ **Extracted rather than copied.** The ordering below is the whole of `w260`'s safety
/// argument (join with no plane lock held → adopt+map → establish+install under one hold →
/// bind, and *nothing is bound until step 3 returns `Ok`*). A second call site that spelled
/// it again would be a second source of truth for an ordering whose failure mode is silent
/// (`a_second_source_of_truth_beside_a_complete_value`).
///
/// `what` names the SOURCE that presented this leaf — an operand's name, or the channel's
/// own ring — so a reader of the boot log can tell the two apart without counting lines.
#[cfg(feature = "host-isolates")]
#[allow(clippy::too_many_arguments)]
fn join_one_fb_leaf(
    head: &str,
    what: &str,
    device: &kayfabe_rt::device::SharedDevice,
    plane: &RegPlane,
    exports: &kayfabe_isolate_host::isolate::ExportDirectory,
    fb_join: FbJoinArm,
    isolate: kayfabe_isolate::IsolateId,
    pdb: kayfabe_rt::Pdb,
    leaf: kayfabe_rt::completion_watch::FbLeaf,
) -> Option<JoinedLeaf> {
    // ---- 1. THE JOIN. No plane lock held: this is a round trip to another process.
    let backed = match device.back_fb_leaf(
        DOORBELL_TARGET_GPU,
        pdb,
        kayfabe_rt::GpuVa(leaf.va),
        leaf.len,
        leaf.phys,
        kayfabe_rt::FbLeafBacking::Joined,
    ) {
        Ok(b) => b,
        Err(e) => {
            eprintln!(
                "{head} {what} leaf va=0x{:x} len=0x{:x} fb_phys=0x{:x} → ⊘ REFUSED BY NAME \
                 `{e:?}` — ⊘ NOT retried, NOT downgraded to the vidmem chain, and nothing was \
                 adopted. ⚠ If this is `Rm(NoMemory)` it is status 0x51, which is \
                 collision-or-exhaustion and CANNOT be told apart",
                leaf.va, leaf.len, leaf.phys
            );
            return None;
        }
    };
    let Some(backing) = backed.backing else {
        // A replay. The view was installed by the call that did the work; a second
        // descriptor would be a second lifetime for one file.
        eprintln!(
            "{head} {what} leaf va=0x{:x} → ALREADY JOINED (idempotent replay; no second \
             object, no second descriptor, no second establishment copy) memory={:#x} \
             host_va=0x{:x}",
            leaf.va,
            backed.memory.raw(),
            backed.host_va,
        );
        return Some(JoinedLeaf {
            host_va: backed.host_va,
            memory: backed.memory.raw(),
            installed: None,
        });
    };
    // ---- 2. ADOPT + MAP. ★★ The ONE property the negative control changes is the
    // `Backing` variant below; everything either side of it is this same code.
    let Some(fd) = exports.dup(isolate, backing.token) else {
        eprintln!(
            "{head} {what} leaf va=0x{:x} → ⚠ THE BACKING CROSSED AND THE VMM COULD NOT CLAIM \
             IT: token={} is not in {isolate:?}'s export registry. The host object EXISTS and \
             is placed; the guest's view does not. ⊘ RELEASED and NOT bound — the row would \
             otherwise declare a join that never happened",
            leaf.va, backing.token
        );
        device.release_unadopted_fb_leaf(DOORBELL_TARGET_GPU, pdb, backed.host_va, backed.memory);
        return None;
    };
    let region = match kayfabe_linux_raw::MappedRegion::map(
        match fb_join {
            FbJoinArm::Shared => kayfabe_linux_raw::Backing::SharedFile {
                fd: std::os::fd::AsFd::as_fd(&fd),
                offset: backing.offset,
            },
            FbJoinArm::Private | FbJoinArm::Off => kayfabe_linux_raw::Backing::PrivateAnonymous,
        },
        backing.len,
        kayfabe_linux_raw::HostProt::ReadWrite,
        kayfabe_linux_raw::CachePolicy::WriteBack,
        kayfabe_linux_raw::HostPageSize::query(),
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "{head} {what} leaf va=0x{:x} → ⚠ THE VMM'S OWN MAPPING FAILED {e:?} — the \
                 host object exists and is placed and the guest's view does not. ⊘ RELEASED \
                 and NOT bound",
                leaf.va
            );
            device.release_unadopted_fb_leaf(
                DOORBELL_TARGET_GPU,
                pdb,
                backed.host_va,
                backed.memory,
            );
            return None;
        }
    };
    // ---- 3. ESTABLISH + INSTALL, in ONE hold of the plane lock.
    //
    // ★★★★★ **AND NOTHING IS BOUND UNTIL THIS RETURNS `Ok`.**
    match plane.join_fb(leaf.phys, Box::new(MappedFb(region))) {
        Ok(est) => {
            eprintln!(
                "{head} {what} leaf va=0x{:x} len=0x{:x} fb_phys=0x{:x} → JOINED ({}) \
                 memory={:#x} host_va=0x{:x} placed_as_asked={} established={} bytes over {} \
                 page(s), of which {} NON-ZERO — ★ ONE memory. ⚠ The leaf is host SYSMEM, a \
                 named divergence from the C",
                leaf.va,
                leaf.len,
                leaf.phys,
                fb_join.as_str(),
                backed.memory.raw(),
                backed.host_va,
                backed.host_va == leaf.va,
                est.copied,
                est.pages,
                est.nonzero,
            );
            if est.copied == 0 {
                eprintln!(
                    "{head}   ⊘ the establishment copy was VACUOUS for this leaf: no page of \
                     it was resident, so nothing the guest had written came across. That is \
                     CORRECT (an unwritten leaf is zeros either way) and it is NOT evidence \
                     that the copy works"
                );
            }
            // ---- 4. BIND, and only now.
            if let Err(e) = device.adopt_joined_fb_leaf(
                DOORBELL_TARGET_GPU,
                pdb,
                kayfabe_rt::FbLeafRange {
                    va: kayfabe_rt::GpuVa(leaf.va),
                    len: leaf.len,
                    phys: leaf.phys,
                },
                &backed,
            ) {
                eprintln!(
                    "{head} {what} leaf va=0x{:x} → ⚠ THE VIEW IS INSTALLED AND THE BIND \
                     REFUSED `{e:?}` — the guest's window and the host object are ONE memory \
                     and the address table does not say so, so nothing will point an engine \
                     here. ⊘ The host mapping is released; the install stands",
                    leaf.va
                );
                return None;
            }
            Some(JoinedLeaf {
                host_va: backed.host_va,
                memory: backed.memory.raw(),
                installed: Some(backing.len),
            })
        }
        Err(e) => {
            eprintln!(
                "{head} {what} leaf va=0x{:x} fb_phys=0x{:x} → ⚠ THE INSTALL REFUSED \
                 phys=0x{:x} len={} why=`{}` — this device still serves that range from its \
                 own pages. ⊘ RELEASED and NOT bound",
                leaf.va, leaf.phys, e.phys, e.len, e.why
            );
            device.release_unadopted_fb_leaf(
                DOORBELL_TARGET_GPU,
                pdb,
                backed.host_va,
                backed.memory,
            );
            None
        }
    }
}

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
    /// ★★★★ §16.40 — the first refused `GPU_PROMOTE_CTX`, latched with the address plane's
    /// state at that instant. See `kayfabe_rmrpc::SharedPromoteDiag`.
    promote_diag: kayfabe_rmrpc::SharedPromoteDiag,
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
    /// ★★★ §5.7 — the filesystem identity of the guest-RAM block the composition root
    /// adopted, or `None` when the crossing is not armed.
    ///
    /// It is here rather than beside the descriptor because this is the object that meets
    /// the *memory* plane: [`Regs::attach_ram`] is the one place where the block we hold and
    /// the hypervisor's stated topology are both in scope, and joining them is what turns an
    /// extent into a layout.
    guest_ram_backing: Option<kayfabe_vmm_qemu::layout::BackingId>,
    /// ★★★★★ **LEG A** — which arm of the guest-ring adoption this boot runs, from the
    /// composition root's own reading of [`GUEST_RING_ENV`]. Carried, never re-read: an
    /// arming flag consulted twice is a boot that can change its mind halfway through.
    guest_ring: GuestRingArm,
    /// ★★★★★ §5.12 — the join's arm, needed HERE and not only on [`SharedDoorbell`] because
    /// the ring source runs on the register-write path, before the doorbell port exists for
    /// this channel. ⊘ The SAME value, cloned from the root — not a second reading.
    #[cfg_attr(not(feature = "host-isolates"), allow(dead_code))]
    fb_join: FbJoinArm,
    /// ★★★★★ §5.12 — the route from a backing token to a descriptor, for
    /// [`Regs::adopt_pending_channel_rings`]. Same handle, same reason, as
    /// [`SharedDoorbell::exports`].
    #[cfg_attr(not(feature = "host-isolates"), allow(dead_code))]
    exports: FbExportDir,
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
        let (
            links,
            refusals,
            promote_diag,
            rings,
            isolates,
            publications,
            isolate_plane,
            device,
            guest_ram_backing,
            exports,
        ) = object_policy(abi.driver, chip.engines)?;
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
        // ★★★★★ §16.80 — the SECOND composition-root selector, read exactly once, here.
        // ⊘ A different variable from `KAYFABE_ISOLATES`, so this is not the "two readings
        // of one selector" the field's doc forbids; it is one reading of each.
        let ce_executor = selected_ce_executor()?;
        // ★ §5.9 — read HERE, at the composition root, beside every other plane decision,
        // and carried into the doorbell port. ⊘ Never re-read at a doorbell: an arming
        // flag consulted twice is a run that can change its mind halfway through a boot.
        let fb_join = selected_fb_join()?;
        // ★★★★★ The GR route's arm — read HERE, beside every other plane decision, exactly
        // once, and carried into the doorbell port. See [`GR_ROUTE_ENV`].
        let gr_route = selected_gr_route()?;
        // ★★★★★ LEG A's arm — read ONCE, here, at the composition root, beside the two
        // arms it composes with. See [`GUEST_RING_ENV`].
        let guest_ring = selected_guest_ring()?;
        // ★★★★★ LEG 4's arm — read ONCE, here, beside every other plane decision, and its
        // own variable rather than a rider on leg A's. See [`GUEST_PUSHBUF_ENV`].
        let guest_pushbuf = selected_guest_pushbuf()?;
        // ★★ PRINTED, because both arms of a two-arm experiment must be distinguishable
        // from the boot's own on-disk evidence. `boot_nvkvm.sh` sends this stderr to
        // `run_<tag>_qemu.log`, which `boot_capture.sh` phase 6 carries into the repository
        // — so the configuration a boot ran with is committed beside its result, rather
        // than living in whichever shell exported the variables.
        eprintln!(
            "kayfabe: EXECUTORS isolate_plane={} ce_executor={} ⇒ \
             local_ce_is_the_only_executor={}",
            isolate_plane.as_str(),
            ce_executor.as_str(),
            isolate_plane == IsolatePlane::Stillborn || ce_executor == CeExecutorChoice::Local,
        );
        // ★★★★★ w237 — ROUTE B, and it is OFF unless this variable says otherwise.
        // ⊘ The registration IS the switch (`SharedDevice::set_fb_source`): with no source,
        // `read_gpfifo_ring` refuses vidmem ranges exactly as before. Printed unconditionally
        // and on BOTH arms, because a configuration that only announces itself when enabled
        // makes the control arm's log indistinguishable from an older binary's.
        let ring_vidmem = std::env::var(RING_VIDMEM_ENV)
            .map(|v| v == "1" || v.eq_ignore_ascii_case("on"))
            .unwrap_or(false);
        eprintln!(
            "kayfabe: RING-VIDMEM {}={} ⇒ route B {}",
            RING_VIDMEM_ENV,
            std::env::var(RING_VIDMEM_ENV).unwrap_or_else(|_| "<unset>".to_string()),
            if ring_vidmem { "ON" } else { "OFF (default)" },
        );
        if ring_vidmem {
            let src: Arc<dyn kayfabe_fwd::FbSource> = Arc::new(PlaneFbSource {
                plane: Arc::downgrade(&plane),
            });
            if device.set_fb_source(src).is_err() {
                eprintln!("kayfabe: RING-VIDMEM ⊘ a framebuffer source was ALREADY registered");
            }
        }
        // ★★★★★ §5.12 — THE ARMING, PRINTED, on every arm including `off`.
        //
        // ⚠ `[measured, this campaign]` a boot can run with a plane **off** and still produce
        // a full `dmesg`, a full serial log, a full census and `RC=0` — with not one line of
        // the changed code having run. Every signal says the experiment happened. The only
        // cure is that the boot's own on-disk evidence states the arming, so a reader can
        // tell an armed run from its control without trusting whichever shell exported the
        // variables. `boot_nvkvm.sh` sends this stderr to `run_<tag>_qemu.log`.
        //
        // ★ `exports_directory` is printed beside it because the arm alone is not sufficient:
        // an armed boot in a build with no route from a backing token to a descriptor joins
        // nothing, and would otherwise look identical here.
        eprintln!(
            "kayfabe: FB-JOIN arm={} exports_directory={} ⇒ leaves are {}",
            fb_join.as_str(),
            {
                #[cfg(feature = "host-isolates")]
                {
                    exports.is_some()
                }
                #[cfg(not(feature = "host-isolates"))]
                {
                    false
                }
            },
            match fb_join {
                FbJoinArm::Off => "NOT materialized at all (the arming control)",
                FbJoinArm::Shared => "JOINED — one backing, two mappings, ONE memory",
                FbJoinArm::Private =>
                    "the NEGATIVE CONTROL — the VMM's view is MAP_PRIVATE|MAP_ANONYMOUS, so \
                     the two views must DISAGREE in both directions",
            },
        );
        // ★★★★★ THE GR ROUTE'S ARMING, PRINTED, on every arm including the default.
        //
        // ⚠ For `FB-JOIN`'s reason, and one sharper here: the two arms of THIS experiment
        // differ in a single routing decision, so a control boot and a disarmed evidence
        // boot produce **identical** logs — no `GR-PASSTHROUGH` line, and a full census of
        // `Route::NotACopyEngineChannel` refusals — unless the arming itself is on disk.
        // `boot_nvkvm.sh` sends this stderr to `run_<tag>_qemu.log`, which `boot_capture.sh`
        // phase 6 carries into the repository.
        eprintln!(
            "kayfabe: GR-ROUTE arm={} ⇒ a GrCompute doorbell is {}",
            gr_route.as_str(),
            match gr_route {
                GrRouteArm::Refuse =>
                    "REFUSED by name (Route::NotACopyEngineChannel) — the default and the \
                     control",
                GrRouteArm::Passthrough =>
                    "HANDED TO THE CORE — routed, the host channel materialized/scheduled, \
                     and its HOST token rung. ⊘ The host engine still fetches NOTHING: the \
                     channel's ring and its GP_PUT are both ours (gr_doorbell_passthrough.md \
                     §0.3)",
            },
        );
        // ★★★★★ LEG A'S ARMING, PRINTED, on every arm including `off`.
        //
        // ⚠ For `FB-JOIN`'s reason and one sharper: `back_census_framebuffer_leaves` has an
        // EMPTY `#[cfg(not(host-isolates))]` twin, so a build without the feature runs the
        // whole of leg A as a silent no-op and exits 0. `host_isolates=` is therefore printed
        // as a **compiled** fact, not as a hope, beside the arm.
        eprintln!(
            "kayfabe: GUEST-RING arm={} host_isolates={} ⇒ the channel's own GPFIFO ring is {}",
            guest_ring.as_str(),
            cfg!(feature = "host-isolates"),
            match guest_ring {
                GuestRingArm::Off =>
                    "NOT presented to the framebuffer join (the control — the join's only \
                     source stays the OPERAND census, exactly as at w260)",
                GuestRingArm::Ring =>
                    "WALKED to its framebuffer leaf and that leaf is JOINED, at the \
                     engine-object latch — i.e. BEFORE the host channel that would name it is \
                     born. ⊘ Supply side only: the host channel still declares OUR ring and \
                     OUR USERD (legs A2 and B)",
            },
        );
        // ★★★★★ LEG 4'S ARMING, PRINTED, on every arm including `off`.
        //
        // ⚠ For `GUEST-RING`'s reason and one sharper: the whole of this leg is a pass that
        // prints and decides nothing, so a boot with it silently disarmed produces a log that
        // differs from an armed one only by the absence of lines — which is exactly what a
        // build that compiled it out also produces. `guest_ram_backing=` is printed beside the
        // arm because the arm alone is not sufficient: the pin path is armed by the adopted
        // guest-RAM block, and an armed boot with no block pins nothing and would otherwise
        // look identical here.
        eprintln!(
            "kayfabe: GUEST-PUSHBUF arm={} guest_ram_backing={} ⇒ the pushbuffer VAs this \
             channel's GPFIFO entries name are {}",
            guest_pushbuf.as_str(),
            guest_ram_backing.is_some(),
            match guest_pushbuf {
                GuestPushbufArm::Off =>
                    "NOT presented to the guest-RAM pin (the control — the pin's only source \
                     stays the channel's RING VA, exactly as at w263, where it refused all \
                     eight `NOT IN GUEST RAM` because a ring is in Vidmem)",
                GuestPushbufArm::Pin =>
                    "RESOLVED through the address table and PINNED, one OS_DESCRIPTOR per \
                     contiguous run, mapped FIXED at the guest's own VA. ⊘ Supply side only: \
                     nothing here says the host channel is bound to a VA space in which those \
                     VAs resolve",
            },
        );
        // ⊘ CLONED, not re-taken. `ExportDirectory` is `Arc`-backed and cloneable for
        // exactly this reason: two ports need the same registry, and a second
        // `export_directory()` call would be a SECOND selection of "which registry".
        // ⊘ The `allow` is the ALIAS's cost, paid where it falls — the same shape
        // `SharedDoorbell::exports` documents. Without `host-isolates`, `FbExportDir` is
        // `()`, so this really is a unit binding; scoping the allow to that configuration
        // keeps a genuinely unit-valued binding in the feature-ON build a hard error.
        #[cfg_attr(
            not(feature = "host-isolates"),
            allow(clippy::let_unit_value, clippy::clone_on_copy)
        )]
        let exports_for_regs = exports.clone();
        plane.set_doorbell(Box::new(SharedDoorbell {
            device: Arc::clone(&device),
            plane: Arc::downgrade(&plane),
            ce: Arc::clone(&ce),
            // ★★★ §14.24 / ★★★★★ §16.80 — from the composition root's OWN selector
            // readings, not from a second one. Two terms, and the second one is a
            // measured refutation of the first standing alone; see
            // [`CE_EXECUTOR_ENV`] and [`ce_executor_from`].
            local_ce_is_the_only_executor: isolate_plane == IsolatePlane::Stillborn
                || ce_executor == CeExecutorChoice::Local,
            guest_ram_backing,
            fb_join,
            exports,
            gr_route,
            guest_pushbuf,
        }));
        Ok(Regs {
            plane,
            device,
            refusals,
            promote_diag,
            rings,
            isolates,
            publications,
            ce,
            guest_ram_backing,
            guest_ring,
            fb_join,
            exports: exports_for_regs,
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

    /// ★★★ **The completion observer's instruments** — an observability seam, in the same
    /// sense [`Regs::audit`] is, and the only way a test can ask whether a guest doorbell
    /// REACHED the observer.
    ///
    /// ⊘ Hands out the list itself rather than a snapshot, because the two numbers that
    /// matter (`attempts` and `declared`) must be read from one instant: a test that read
    /// them from two calls could see a doorbell between them and attribute the gap to the
    /// wiring.
    #[must_use]
    pub fn completion_watch(&self) -> Arc<kayfabe_rt::completion_watch::WatchList> {
        Arc::clone(&self.ce.watch)
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
        self.report_stated_guest_ram_at(shim, Self::ATTACH);
        // ★★★★★ The completion observer's reactor thread. Started HERE and not at realize,
        // for the same reason the memory plane is attached here: before this instant there
        // is no guest memory to observe an address in.
        self.start_completion_observer(shim.machine().vmm());
    }

    /// ★★★★★ **Start the completion observer's reactor loop.** See [`ObserverThread`].
    ///
    /// ⊘ Idempotent and quiet: a second attach finds a live thread and returns. Every
    /// refusal SAYS SO — an observer that failed to start and printed nothing would be
    /// indistinguishable from one that started and saw nothing, which is the exact defect
    /// class this file's §16.79 dump was written against.
    #[cfg(feature = "host-isolates")]
    fn start_completion_observer(&self, vmm: kayfabe_vmm_qemu::QemuVmm) {
        let mut slot = self.ce.observer.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() {
            return;
        }
        let build = || -> Result<ObserverThread, String> {
            let poller = std::sync::Arc::new(
                kayfabe_linux_raw::Poller::create().map_err(|e| format!("{e:?}"))?,
            );
            let registrar = std::sync::Arc::new(
                kayfabe_shell::Registrar::new(poller).map_err(|e| format!("{e:?}"))?,
            );
            // ★ A REAL armed registration, not a bare timeout loop: `arm_counter` had zero
            // production callers until this line. The vCPU signals it on every NEW
            // declaration, so a completion is looked at promptly rather than at the next
            // tick.
            let src = self
                .device
                .register_source(kayfabe_core::reactor::SourceKind::Notify);
            let poke = registrar.arm_counter(src).map_err(|e| format!("{e:?}"))?;
            let (tx, _rx) = kayfabe_rt::inbox::inbox();
            let parker = std::sync::Arc::new(kayfabe_rt::executor::Parker::new());
            let (mut reactor, handle) = kayfabe_shell::Reactor::new(
                registrar,
                tx,
                parker as std::sync::Arc<dyn kayfabe_rt::executor::ExecutorWaker>,
            )
            .map_err(|e| format!("{e:?}"))?;
            let watch = std::sync::Arc::clone(&self.ce.watch);
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let stop_thread = std::sync::Arc::clone(&stop);
            let join = std::thread::Builder::new()
                .name("kayfabe-completion-observer".into())
                .spawn(move || observer_loop(&mut reactor, &watch, &stop_thread, vmm))
                .map_err(|e| format!("{e}"))?;
            Ok(ObserverThread {
                handle,
                poke,
                stop,
                join: Some(join),
            })
        };
        match build() {
            Ok(o) => {
                eprintln!(
                    "kayfabe: COMPLETION-OBSERVER started — one thread, one epoll, one armed \
                     counter source. ⊘ It READS the addresses the guest declared and can \
                     write none of them."
                );
                *slot = Some(o);
            }
            Err(why) => eprintln!(
                "kayfabe: COMPLETION-OBSERVER ⊘ NOT STARTED: {why}. Every COMPLETION-WATCH \
                 line below this point is therefore ABSENT BY CONSTRUCTION and must not be \
                 read as 'nothing was declared'."
            ),
        }
    }

    /// Without the host-isolate feature there is no `kayfabe-shell` in this archive's
    /// dependency graph, so there is no reactor to start. ⊘ Stated as a no-op with a name
    /// rather than an `#[cfg]` around the call site, so the shipping build's behaviour is
    /// visible where the observer is started.
    #[cfg(not(feature = "host-isolates"))]
    #[allow(clippy::unused_self, clippy::needless_pass_by_value)]
    fn start_completion_observer(&self, _vmm: kayfabe_vmm_qemu::QemuVmm) {}

    /// Stop the observer and **join** it. See [`ObserverThread`] for why the join is not
    /// optional: the thread reads guest RAM, and the region it reads is released by the
    /// hypervisor after this returns.
    #[cfg(feature = "host-isolates")]
    fn stop_completion_observer(&self) {
        let taken = self
            .ce
            .observer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(mut o) = taken {
            // ⊘ The flag FIRST, then the wake: a loop woken before the flag is set would go
            // round once more and read guest RAM we are about to release.
            o.stop.store(true, std::sync::atomic::Ordering::Release);
            let _ = o.handle.shutdown();
            if let Some(j) = o.join.take() {
                let _ = j.join();
            }
            let s = self.ce.watch.stats();
            eprintln!(
                "kayfabe: COMPLETION-OBSERVER stopped — declared={} redeclared={} reads={} \
                 verdicts={} still_watching={} (reads=0 with declared>0 means the loop never \
                 ran, NOT that nothing appeared)",
                s.declared,
                s.redeclared,
                s.reads,
                s.verdicts,
                self.ce.watch.live(),
            );
        }
    }

    /// No thread was ever started, so there is none to stop.
    #[cfg(not(feature = "host-isolates"))]
    #[allow(clippy::unused_self)]
    fn stop_completion_observer(&self) {}

    /// ★ The instant the memory plane came up. Named rather than spelled twice, because the
    /// zero-case diagnosis branches on it and a typo would silently pick the wrong reading.
    const ATTACH: &'static str = "MEMORY PLANE ATTACH";

    /// ★★★ §5.7 — **the join**: the block this root adopted, met with the hypervisor's own
    /// statement of where that block appears in the guest's physical space.
    ///
    /// This is the whole of step 2's evidence, and it is printed rather than merely stored
    /// because [`boot_evidence_must_be_asserted_into_the_repo`] applies exactly here: the
    /// layout is the input every later mapping is computed from, and a boot that produced a
    /// wrong one would look identical to a boot that produced a right one until an isolate
    /// read somebody else's page.
    ///
    /// ⊘ **Silent when the crossing is not armed.** A boot that did not ask for guest RAM
    /// must not carry a line about guest RAM's layout — otherwise the negative control stops
    /// being byte-comparable to the armed run, which is the property that makes it a control.
    ///
    /// ⊘ **A zero-run report is printed too, and loudly.** An armed run whose adopted block
    /// matched no stated section is the interesting failure — the descriptor is real, the
    /// topology is real, and the *join* is what is broken — and it is invisible unless the
    /// section count is printed beside the run count.
    pub fn report_stated_guest_ram_at(&self, shim: &Shim, at: &str) {
        let Some(backing) = self.guest_ram_backing else {
            return;
        };
        // ★★★ THE RUNS REPORTED ARE THE `ever` ONES, and that is the correction w225c/w225d
        // forced. The LIVE table is empty at memory-plane attach (the listener sits on the
        // bus-master address space, which the guest has not enabled) and empty again at the
        // exit notifier (teardown replays `region_del` over every range). It was correct
        // throughout the middle, and neither instant a device can reach shows it.
        // ⊘ `resolve` still answers from the LIVE table only — this is the report, not the
        // resolver, and the two must not become one.
        let runs = shim.machine().stated_guest_ram_ever(backing);
        let live = shim.machine().stated_guest_ram(backing).len();
        let sections = shim.machine().stated_sections();
        let c = shim.machine().layout_census();
        let total: u128 = runs.iter().map(|r| u128::from(r.len)).sum();
        eprintln!(
            "kayfabe: ★★★ GUEST-RAM LAYOUT AT {at}, AS THE HYPERVISOR STATED IT — dev={} \
             ino={}: {} contiguous run(s) totalling {total} bytes, out of {sections} stated \
             section(s) LIVE over all backing files ({live} live run(s) for this block). \
             Section funnel: {} reported -> {} classified RAM -> {} carried a backing file, \
             {} later withdrawn. ⊘ Every run below arrived on a topology \
             callback; NOTHING here is derived from the machine type or from `-m`, and a \
             guest-physical address outside these runs is refused by name rather than \
             assumed to be its own file offset. ⚠ This is ONE INSTANT: the listener is \
             registered on the device's bus-master address space, which is empty until the \
             guest enables bus mastering, so an empty report here is a statement about WHEN \
             it was taken and not about the machine.",
            backing.dev,
            backing.ino,
            runs.len(),
            c.seen,
            c.ram,
            c.backed,
            c.forgotten
        );
        if runs.is_empty() {
            // ★★ The funnel, read out as a sentence, because 0-of-0 and 12-of-0 are
            // different defects in different files and a reader should not have to derive
            // which from three integers.
            let where_it_broke = if c.seen == 0 && at == Self::ATTACH {
                // ⊘ NOT a fault, and saying so was the first version's mistake. The listener
                // is registered on the device's bus-master address space, which the guest has
                // not enabled at this instant, so an empty flat view here is the EXPECTED
                // reading. `[measured 2026-08-10, boots w225c-w225e]`: the same run reports
                // 0 sections here and 76 at the end.
                "nothing yet, and at THIS instant that is expected rather than broken — the \
                 guest has not enabled bus mastering, so the address space this device \
                 listens on is still empty. Read the END OF RUN report instead"
            } else if c.seen == 0 {
                "the listener reported NOTHING for the whole run — the topology callback \
                 never fired, or fired before this device had a handle to receive it. That \
                 is an ORDERING fault in the hypervisor shim, not a layout fault"
            } else if c.ram == 0 {
                "sections arrived and NONE classified as plain RAM — every one was a device, \
                 a ROM, read-only or non-volatile. That is a CLASSIFICATION fault; see \
                 `classify::is_ram`"
            } else if c.backed == 0 {
                "RAM sections arrived and NONE carried a backing file — the hypervisor could \
                 not identify a descriptor behind them. That is a fault in the shim's \
                 `fd_backed` fact, not in this module"
            } else {
                "sections stated runs, but for OTHER backing files — the descriptor this \
                 device adopted is not the one behind the guest's RAM. That is a JOIN fault"
            };
            eprintln!(
                "kayfabe: ⊘⊘ ZERO RUNS, and the crossing IS armed — the descriptor was \
                 adopted and {sections} section(s) stated a run, but none named this block. \
                 ★ Where it broke: {where_it_broke}. Nothing can be mapped until this joins; \
                 an empty layout refuses every address, which is correct and is not progress."
            );
        }
        if runs.is_empty() {
            // ★★★ PRINT WHAT WAS THERE. A join that matched nothing is diagnosable only from
            // the OTHER side's keys; without these lines the log says "no match" and the
            // next person goes looking on the wrong side of the seam.
            let seen = shim.machine().layout_backings_seen();
            if seen.is_empty() {
                eprintln!(
                    "kayfabe:   (no backing file stated any run at all, so there is nothing \
                     to have matched against)"
                );
            }
            for (b, n, bytes) in &seen {
                eprintln!(
                    "kayfabe:   stated by dev={} ino={}: {n} section(s), {bytes} bytes{}",
                    b.dev,
                    b.ino,
                    if *b == backing {
                        "  <= THE BLOCK WE ADOPTED"
                    } else {
                        ""
                    }
                );
            }
        }
        for r in &runs {
            eprintln!(
                "kayfabe:   gpa 0x{:016x}..0x{:016x} -> file offset 0x{:016x} ({} bytes){}",
                r.gpa,
                r.gpa_end(),
                r.file_offset,
                r.len,
                if r.is_identity() {
                    " [identity — an OBSERVATION about this run, never a rule]"
                } else {
                    " [★ NON-IDENTITY — a derived layout would have been wrong here]"
                }
            );
        }
    }

    /// The stated guest-RAM runs for the adopted block, in guest-physical order.
    ///
    /// Empty when the crossing is not armed, and empty when it is armed and nothing joined —
    /// ⊘ two very different facts that a caller must not merge. [`Regs::guest_ram_backing`]
    /// distinguishes them.
    #[must_use]
    pub fn stated_guest_ram(&self, shim: &Shim) -> Vec<kayfabe_vmm_qemu::layout::StatedRun> {
        self.guest_ram_backing
            .map(|b| shim.machine().stated_guest_ram(b))
            .unwrap_or_default()
    }

    /// The filesystem identity of the adopted guest-RAM block, or `None` when the crossing
    /// is not armed.
    #[must_use]
    pub fn guest_ram_backing(&self) -> Option<kayfabe_vmm_qemu::layout::BackingId> {
        self.guest_ram_backing
    }

    /// Put the plane back to refusing every guest-memory access, by name.
    ///
    /// ★ The teardown half, and it is **not** optional. The port holds a handle onto the
    /// memory plane; leaving it installed across an unrealize would mean the register
    /// surface — which keeps answering, deliberately — could still be asked to follow a
    /// guest pointer into a machine that has released its slots. Refusing is the honest
    /// answer at that point and it is the one this restores.
    pub fn detach_ram(&self) {
        // ★★★ FIRST, and the order is load-bearing: the observer thread reads guest RAM
        // through its own handle, and the hypervisor releases the regions behind that
        // handle once this returns. Stop and JOIN before anything else is torn down.
        self.stop_completion_observer();
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

    /// ★★★★★ **LEG A1 — JOIN THE CHANNEL'S OWN RING, BEFORE ITS HOST CHANNEL IS BORN.**
    ///
    /// # ★★★ The ordering fact this exists to satisfy, and it is read off the code
    ///
    /// `HostRmBackend::alloc_channel_in`'s guest-ring arm calls `narrow(g.memory)` — a
    /// validation that the handle was minted **by this isolate's RM connection**. ⇒ the host
    /// object over the guest's ring must **exist** before the host channel is born, and the
    /// host GR channel is born inside `commit_engine_object`, which the drain two lines below
    /// this call runs. This is therefore the **last instant** at which *"a host channel is
    /// about to be born for guest channel X"* is knowable and X is still un-born.
    ///
    /// ⊘ **This is NOT what `guest_ring_adoption.md` §3.3 refuted.** §3.3 refuted the claim
    /// that the birth must move so the ring could be **bound** — measured, R31 arm C: a
    /// `gpFifoOffset` at an address nothing was ever mapped at was **accepted**. Binding may
    /// be late. Minting may not. The two are different obligations and only one of them was
    /// discharged.
    ///
    /// # ⊘ Why the operand census could never have supplied this
    ///
    /// `back_census_framebuffer_leaves` is driven by the addresses the **methods**
    /// dereference. `[measured 2026-08-11, w260]` it joined framebuffer leaves `0x400000` /
    /// `0x600000` / `0x800000`; the GR ring's leaf is `0x1000000` and was never presented,
    /// because **a ring is not an operand of the methods it carries**. And it runs at the
    /// **doorbell**, behind a successful pushbuffer decode — long after the birth.
    ///
    /// # ★★★ OPACITY IS PRESERVED BY CONSTRUCTION, not by a rule
    ///
    /// The only thing read here is a **page table**: one VA (`gpFifoOffset`, which the guest
    /// declared in its own channel alloc) walked to its leaf. ⊘ No GPFIFO entry is read, no
    /// pushbuffer is fetched, no method is decoded, nothing is classified. Nothing here gates
    /// whether any work is forwarded: every arm returns `()` and the drain below runs
    /// identically either way.
    ///
    /// # R1
    ///
    /// `RegPlane::write` has returned, so this frame holds **no ranked lock**. The walk takes
    /// the plane's rank-0 mutex for the duration of one resolution and **prints nothing
    /// inside it**; the join — which is a round trip to another process — runs strictly after
    /// that guard is dropped.
    #[cfg(feature = "host-isolates")]
    fn adopt_pending_channel_rings(&self) {
        if !self.guest_ring.adopts_ring() {
            // ⊘ Silent, exactly as `back_census_framebuffer_leaves`' disarmed arm is: the
            // control's log must not contain a line the armed run's does not, or the two stop
            // being comparable. The arming itself is on disk, printed once at the root.
            return;
        }
        let pending = self.device.peek_pending_engine_forwards();
        if pending.is_empty() {
            // The overwhelmingly common case — this runs on every register write.
            return;
        }
        // ★★★★★ THE POSITIVE SIGNAL, emitted on EVERY armed pass that has anything to do,
        // **including the ones that join nothing.** ⚠ Without it *"leg A never executed"* and
        // *"leg A executed and changed nothing"* are identical on every other observable —
        // the same class as a `dlen=0` oracle row and a zero-byte bench artefact.
        let head = "kayfabe: GR-RING-JOIN".to_string();
        let Some(exports) = self.exports.as_ref() else {
            eprintln!(
                "{head} arm={} pending={} → ⊘ NOT ARMABLE: this build has no route from a \
                 backing token to a descriptor (exports_directory=false), so no leaf can be \
                 claimed. ⊘ Nothing was asked of the host",
                self.guest_ring.as_str(),
                pending.len(),
            );
            return;
        };
        eprintln!(
            "{head} arm={} host_isolates=yes exports_directory=true fb_join={} pending={} — \
             the engine-object latch is about to be drained, and every host channel it births \
             is born HERE. ⊘ Nothing below reads a ring byte",
            self.guest_ring.as_str(),
            self.fb_join.as_str(),
            pending.len(),
        );
        for (client, parent, class) in pending {
            let facts = match self
                .device
                .engine_object_channel_facts(client, parent, class)
            {
                Ok(f) => f,
                Err(e) => {
                    eprintln!(
                        "{head} client={:#x} parent={:#x} class={:#06x} → ⊘ NOT ROUTED `{e:?}` \
                         — this alloc names no channel this port can resolve, so there is no \
                         ring to adopt. ⊘ Not a miss: the drain refuses it too",
                        client.0, parent.0, class.0,
                    );
                    continue;
                }
            };
            let (Some(ring_va), Some(pdb), Some(vaspace)) =
                (facts.ring_va, facts.vas_pdb, facts.vaspace)
            else {
                eprintln!(
                    "{head} proc={} chan={} class={:#06x} → ⊘ NOTHING TO ADOPT: ring_va={:?} \
                     vas_pdb={:?} vaspace={:?}. ⚠ `ring_va = Some(0)` would be a VALUE and not \
                     a blank — the driver declares `gpFifoOffset = 0` for its golden-context \
                     channel — so a `None` here is the channel declaring no ring at all",
                    facts.proc.0,
                    facts.chan.0,
                    class.0,
                    facts.ring_va,
                    facts.vas_pdb,
                    facts.vaspace,
                );
                continue;
            };
            let root = match SharedDoorbell::doorbell_root(
                &self.plane,
                facts.client,
                vaspace,
                Some(pdb.0),
            ) {
                DoorbellRoot::Published(r) | DoorbellRoot::Declared(r) => r,
                DoorbellRoot::Absent => {
                    eprintln!(
                        "{head} proc={} chan={} ring=0x{ring_va:x} → ⊘ NO ROOT: this channel \
                         has no VA space root at all, so its ring VA cannot be walked",
                        facts.proc.0, facts.chan.0,
                    );
                    continue;
                }
                DoorbellRoot::Underivable(p, why) => {
                    eprintln!(
                        "{head} proc={} chan={} ring=0x{ring_va:x} → ⊘ ROOT UNDERIVABLE from \
                         pdb 0x{p:x}: {}",
                        facts.proc.0,
                        facts.chan.0,
                        why.kind(),
                    );
                    continue;
                }
            };
            // ★ The walk, and NOTHING is printed inside the guard (R1).
            let (site, leaf) = self.plane.ce_session_with_root(
                &root,
                kayfabe_device::ceresolve::Demand::from_doorbell(),
                |ce| kayfabe_rt::ceutils::resolve_leaf_of(ce, ring_va),
            );
            let Some(leaf) = leaf else {
                eprintln!(
                    "{head} proc={} chan={} ring=0x{ring_va:x} entries={} → ⊘ NOT A \
                     FRAMEBUFFER LEAF: {site:?}. ⚠ `GuestRam` here is a REAL and SERVED case \
                     that belongs to the guest-RAM pin, not to this source; `Unresolved` is a \
                     TIMING fact — the guest had not bound its own ring at the instant its \
                     engine object was latched — and must NOT be read as `the channel \
                     declared no ring`",
                    facts.proc.0, facts.chan.0, facts.ring_entries,
                );
                continue;
            };
            let isolate = kayfabe_isolate::IsolateId::new(facts.proc.0, DOORBELL_TARGET_GPU);
            let what = format!(
                "RING(chan={} entries={} engine={})",
                facts.chan.0,
                facts.ring_entries,
                facts.engine_name(),
            );
            match join_one_fb_leaf(
                &head,
                &what,
                &self.device,
                &self.plane,
                exports,
                self.fb_join,
                isolate,
                pdb,
                leaf,
            ) {
                Some(j) => eprintln!(
                    "{head} proc={} chan={} ring=0x{ring_va:x} entries={} → ★★★★★ THE RING'S \
                     OWN LEAF IS JOINED: memory={:#x} host_va=0x{:x} fb_phys=0x{:x}. ⊘ This is \
                     the SUPPLY side only — the host channel about to be born still declares \
                     OUR ring and OUR USERD, so GP_PUT == GP_GET and the engine fetches \
                     nothing (gr_doorbell_passthrough.md §0.3). Legs A2 and B are what consume \
                     this",
                    facts.proc.0, facts.chan.0, facts.ring_entries, j.memory, j.host_va, leaf.phys,
                ),
                None => eprintln!(
                    "{head} proc={} chan={} ring=0x{ring_va:x} → ⊘ THE RING'S LEAF WAS NOT \
                     JOINED; the refusal above names why. Nothing is bound and the drain below \
                     is unaffected",
                    facts.proc.0, facts.chan.0,
                ),
            }
        }
    }

    /// ⊘ **THE STUB, AND IT IS DELIBERATELY NOT SILENT.**
    ///
    /// `back_census_framebuffer_leaves`' own `#[cfg(not(host-isolates))]` twin has an empty
    /// body, and that is exactly the shape that makes *"the experiment never ran"* read as
    /// *"the experiment ran and changed nothing"* — an archive built without the feature
    /// prints nothing, exits 0, and every other signal says the boot happened. This one says
    /// so, **once**, the first time it is asked to do something.
    #[cfg(not(feature = "host-isolates"))]
    fn adopt_pending_channel_rings(&self) {
        if !self.guest_ring.adopts_ring() {
            return;
        }
        static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
            eprintln!(
                "kayfabe: GR-RING-JOIN arm={} host_isolates=NO ⇒ ⊘ THIS ARCHIVE CANNOT ADOPT A \
                 RING AT ALL. The arm was requested and this build has no isolate plane, so \
                 leg A is a no-op — ⚠ do NOT grade a boot from this binary as `armed and \
                 nothing moved`",
                self.guest_ring.as_str(),
            );
        }
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
        let out = self.plane.write(clamp_bar(bar), off, clamp_size(size), val);
        // ★★★★★ **THE DRAIN, and this line is the whole fix (§16.91).**
        //
        // `RegPlane::write` has returned, so the plane's rank-0 guard is a dropped local and
        // this frame holds **no ranked lock at all** — it is the outermost frame on the vCPU's
        // MMIO trap that still has an `Arc<SharedDevice>`. An isolate spawn decided anywhere
        // inside that call is latched in the spine and runs **here**, lock-free.
        //
        // ⊘ **PULL, not push.** Carrying the batch outward through `CommandPolicy` would force
        // `kayfabe-core` vocabulary across a `kayfabe-gsp` port and cost either a new crate
        // edge or a second identity vocabulary for `(Proc, GpuId)` (§16.90). The latch already
        // lives in `core` and this frame already holds the device, so nothing has to travel.
        //
        // ⚠ **The guest cannot observe anything before this runs.** The vCPU is halted inside
        // its own MMIO trap for the whole of `kayfabe_shim_regs_write`; replies written into
        // guest RAM above are not yet readable by the guest, because the guest is not running.
        // ⇒ the reply-before-spawn window the fix appeared to open does not exist on this
        // thread. A *second* vCPU racing the same proc is the pre-existing
        // `FwdFault::IsolatePending` compare-and-swap, unchanged by this rung.
        //
        // ⊘ `materialize_pending` asserts lock-freedom, so if any of the above is wrong this
        // is refused **by name, here**, rather than by a spawn six crates away.
        self.device.materialize_pending();
        // ★★★★★ **§16.96 — THE SECOND DRAIN, and it is the same fix for the same defect.**
        //
        // The spawn was one of *two* blocking calls the plane's rank-0 mutex was held
        // across; this is the other (`[measured 2026-08-11, §16.91,
        // `traces/boots/w239/`]` — `issuing a host RM verb while holding rank(s) [0]`, via
        // `Bridge::deliver → SharedObjectModel::forward_engine_object → … →
        // Worker::execute`). Everything the block above says about this frame holds
        // verbatim: `RegPlane::write` has returned, its guard is a dropped local, and this
        // is the outermost frame on the vCPU's MMIO trap that still holds the device.
        //
        // ⊘ **AFTER `materialize_pending`, and the order is load-bearing.** A forward routes
        // through an isolate; if this same register write also decided that isolate's spawn,
        // draining the spawn first means the forward finds it installed. Reversed, it would
        // meet `FwdFault::IsolatePending` and pay `verb_op`'s retry.
        //
        // ⚠ **The guest cannot observe anything before this runs** — the vCPU is halted
        // inside its own MMIO trap for the whole of `kayfabe_shim_regs_write`. ⊘ And this
        // drain posts NOTHING into the message queue: it issues a host verb and prints. The
        // driver's `bPollingForRpcResponse` assert (`kernel_gsp.c:2345`) fires on an
        // *induced second RPC* while it polls; a drain that emits no event and no reply
        // cannot induce one. That obligation is discharged by construction, not by care.
        // ★★★★★ **LEG A1 — AND IT MUST BE THE LINE ABOVE THE DRAIN, not below it.**
        // The drain births the host GR channel (`commit_engine_object`), and
        // `alloc_channel_in`'s guest-ring arm `narrow()`s the ring's memory handle — so the
        // object over the guest's ring has to exist BEFORE this next line, or leg A2 has
        // nothing to name. ⊘ Ordering, not preference.
        self.adopt_pending_channel_rings();
        report_engine_forward_drain(&self.device);
        out
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
            bar1_reads,
            bar1_writes,
            bar1_faults,
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
            gsp_event_raises,
            gsp_event_unvectored,
            gsp_event_masked,
            status_irq_cleared,
            commands,
            commands_unserviced,
            doorbells,
            doorbells_served,
            doorbells_served_locally,
            doorbells_served_forwarded,
            doorbells_refused,
        } = self.plane.counters();
        // ★★★★ §16.65 — the per-engine census, read from the SAME shared shell state the
        // routing decision tallies into (`SharedDoorbell::try_ce_submission`). ⊘ Not
        // re-derived from the object model here: a second walk of the channel table could
        // disagree with what the doorbell path actually decided, which is §16.64's own
        // named failure.
        let db_census = *self.ce.census.lock().unwrap_or_else(|e| e.into_inner());
        // ★★★★★ §16.76 — the os-event registry, read from the SAME shared handle the chain
        // link writes into, for `db_census`' reason exactly: a second copy of the fact could
        // disagree with what the delivery path actually did.
        let os_events = self.plane.os_event_log();
        let os_join = os_events.last_join();
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
            // ★★★★ §16.56 — the ids beside the tag. `RefusalCensus::ids` is already capped
            // at `REFUSAL_DETAIL_CAP`; the `zip` is the second bound, so a cap raised on
            // one side alone cannot overrun this array.
            let mut k = 0usize;
            for (slot, id) in row.ids.iter_mut().zip(census.ids(tag)) {
                *slot = id;
                k += 1;
            }
            row.ids_len = k as u64;
            bridge_refusal_len += 1;
        }
        // ⊘ Reported from the census, not from the loop: a set larger than the array must
        // say so, exactly as `unserviced_len` does.
        let bridge_refusal_len = bridge_refusal_len.max(census.tags().count() as u64);
        // ★★★★ §16.40 — the promote diagnosis, latched at the refusal and only copied out
        // here. ⊘ Nothing is SAMPLED at this point: by teardown the CUDA process is gone
        // and its channels with it, so a census taken here would be a true sentence about
        // the wrong instant. `copy_sentence` stamps its own `[CLIPPED …]` tail.
        let diag_rows = self.promote_diag.rows();
        let mut promote_diag = [KayfabePromoteDiag::default(); PROMOTE_DIAG_SLOTS];
        for (row, (tag, text)) in promote_diag.iter_mut().zip(diag_rows.iter()) {
            let tb = tag.0.as_bytes();
            let take = tb.len().min(BRIDGE_REFUSAL_TAG_LEN);
            row.tag[..take].copy_from_slice(&tb[..take]);
            row.tag_len = take as u64;
            row.text_len = copy_sentence(&mut row.text, text);
        }
        // ⊘ From the map, not the loop: a set larger than the array must say so.
        let promote_diag_len = diag_rows.len() as u64;
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
        // ★★★★ **§16.71 — THE ROSTER, printed beside the tally it summarises.**
        //
        // The C line above reports `N declared, M non-zero, first 0x…`. `[measured
        // 2026-08-10]` both `w205` arms printed the same `first 0x120064000` while the real
        // arm's doorbells were resolving `0x420064000`, so ONE boot held both addresses and
        // the line could name the owner of neither. ⇒ §16.70.6's question — *"two ring
        // addresses for one token: two channels, or one channel placed differently?"* —
        // could not be answered from any log this port produced.
        //
        // ⊘ On stderr rather than through the C audit struct: the audit's ring block is
        // four scalars and a roster is a list, and §16.70.1(6) established that the
        // isolate/rt stderr **is** QEMU's stderr and is captured. No ABI change buys
        // nothing here.
        {
            let (rows, dropped) = self.rings.roster();
            eprintln!(
                // ⊘ The runs of spaces this line used to carry were a real defect and are
                // recorded rather than quietly fixed: the string was written through a
                // generator that ate the `\` continuations and left their indentation, and
                // `w206`'s own roster is what showed it. An instrument that garbles its
                // own header is one a reader distrusts before reading its rows.
                "kayfabe: RING-ROSTER {} row(s), {dropped} dropped past the cap (a \
                 non-zero drop count means this list is a PREFIX and its absences prove \
                 nothing)",
                rows.len(),
            );
            for r in &rows {
                eprintln!(
                    "kayfabe:   RING-ROSTER key=0x{:x}:0x{:x} ring=0x{:x} entries={}",
                    r.client, r.handle, r.va, r.entries,
                );
            }
        }
        // ★★★★★ **§16.77 — THE UNCLAIMED CENSUS, and it is the instrument that would have
        // found §16.77's bug in ONE boot instead of five.**
        //
        // `RegPlane::unclaimed_sample` has existed since the plane did, is documented in
        // `kayfabe_device::plane`'s own header as the *which* behind `unclaimed_reads`, and
        // **was never printed anywhere**. So every boot reported a bare count — `w212`:
        // `UNCLAIMED 2888r/2464w` — and an operator could learn how much of the boot was
        // answered with a defaulted zero but never WHICH register got one.
        //
        // ⊘ That is the failure class this repo already has a name for: a diagnostic that
        // exists, crosses no ABI, costs nothing, and is invisible. The offsets that decided
        // `w212` — `NV_PRISCV_RISCV_IRQMASK` at `0xb...111528` and `IRQDEST` at `0x11152c` —
        // were in that vector on every one of the five boots that hunted this wall.
        //
        // ⚠ **BOUNDED, AND IT SAYS SO.** The sample stops at
        // `kayfabe_device::plane::UNCLAIMED_SAMPLE_MAX` distinct `(bar, offset)` pairs, so an
        // ABSENCE from this list proves nothing once the cap is reached — same reading rule
        // as the roster above. It is a first-N sample, not a set.
        {
            let sample = self.plane.unclaimed_sample();
            eprintln!(
                "kayfabe: UNCLAIMED-CENSUS {} distinct (bar, offset) pair(s) answered with a \
                 DEFAULTED ZERO (⊘ bounded first-N sample — an absence here proves nothing; \
                 the totals are the `registers:` line's UNCLAIMED counts)",
                sample.len(),
            );
            for (bar, off) in &sample {
                eprintln!("kayfabe:   UNCLAIMED-CENSUS bar{bar} off=0x{off:06x}");
            }
        }
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
        // ★★★★ §16.30 — read ONCE, so the record and its two counters describe one
        // snapshot. Reading `latest()` again below could straddle a concurrent install and
        // report a `valid` that belongs to a different row than the fields beside it.
        let set_page_dir = self.plane.set_page_dir();
        let (set_page_dir_total, set_page_dir_refused) = self.plane.set_page_dir_counts();
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
            bar1_reads,
            bar1_writes,
            bar1_faults,
            // ★ Read off the SAME chip row `RegPlane::bar1_phys` walks from and
            // `StaticInfoPolicy` publishes, so the report cannot say one address while the
            // walk uses another.
            bar1_pde_base: self.plane.chip().bar1_pde_base,
            bar1_root_published: u64::from(self.plane.bar_pdes().bar1.is_some()),
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
            gsp_event_raises,
            gsp_event_unvectored,
            gsp_event_masked,
            status_irq_cleared,
            os_events_registered: os_events.registered(),
            os_events_retired: os_events.retired(),
            os_events_live: os_events.live().len() as u64,
            os_events_malformed: os_events.malformed(),
            os_events_overflowed: os_events.overflowed(),
            os_event_posted: os_events.posted(),
            os_event_batches: os_events.batches(),
            os_event_gated: os_events.gated(),
            os_event_not_running: os_events.not_running(),
            os_event_failed: os_events.failed(),
            os_event_woke_with_nothing: os_events.woke_with_nothing(),
            os_event_last_join_served: os_join.served,
            os_event_last_join_forwarded: os_join.forwarded,
            os_event_last_join_advanced: os_join.advanced,
            commands,
            commands_unserviced,
            unserviced_len: unserviced_distinct,
            unserviced,
            bridge_refusals,
            bridge_refusal_len,
            bridge_refusal,
            promote_diag,
            promote_diag_len,
            isolates_materialized,
            isolates_live,
            isolates_no_plane,
            isolates_spawn_failed,
            isolate_refusal,
            doorbells,
            doorbells_served,
            doorbells_served_locally,
            doorbells_served_forwarded,
            doorbells_by_engine: db_census.by_engine,
            doorbells_engine_unrouted: db_census.unrouted,
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
            // ★★★★ §16.30 — the `0x00801813` install record. ⊘ `set_page_dir_valid` is
            // written from `Option::is_some` and NOT inferred from any of the values
            // below, because `hVASpace == 0` is a real handle and every one of them is
            // ambiguous at zero.
            set_page_dir_total,
            set_page_dir_refused,
            set_page_dir_valid: u64::from(set_page_dir.is_some()),
            set_page_dir_client: set_page_dir.map_or(0, |r| u64::from(r.client)),
            set_page_dir_object: set_page_dir.map_or(0, |r| u64::from(r.object)),
            set_page_dir_h_vaspace: set_page_dir.map_or(0, |r| u64::from(r.h_vaspace)),
            set_page_dir_phys: set_page_dir.map_or(0, |r| r.phys_address),
            set_page_dir_num_entries: set_page_dir.map_or(0, |r| u64::from(r.num_entries)),
            set_page_dir_flags: set_page_dir.map_or(0, |r| u64::from(r.flags)),
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
    // ★★★★ §16.40 — the first refused `GPU_PROMOTE_CTX`, with the address plane's state as
    // it stood at the refusal. Carried out for the refusal census's reason exactly:
    // afterwards there is no `ObjectPolicy` left to ask.
    kayfabe_rmrpc::SharedPromoteDiag,
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
    // ★★★ §5.7 — the filesystem identity of the guest-RAM block this root ADOPTED, carried
    // out beside the descriptor it was taken from.
    //
    // ⊘ Carried rather than re-derived, and that is the whole reason it is in this tuple.
    // The identity is available at any time by re-taking the descriptor census, and doing
    // so would create a SECOND selection of "which block is guest RAM" — two projections of
    // one fact, which this project has now measured disagreeing three times. There is one
    // selection, in `with_guest_ram`, and this is its answer travelling to the one place
    // that joins the hypervisor's stated layout against it.
    //
    // `None` when the crossing is not armed: nothing was adopted, so nothing is claimed.
    Option<kayfabe_vmm_qemu::layout::BackingId>,
    // ★★★★★ §5.12 — the isolate factory's export directory, cloned off it BEFORE it was
    // boxed into the object model. Carried out for exactly `guest_ram_backing`'s reason: it
    // is the only moment at which the concrete factory is reachable, and re-deriving it
    // afterwards is impossible rather than merely a second source of truth.
    //
    // `None` when the isolate plane is stillborn — no isolate can hand a descriptor up, and
    // an empty directory would pretend otherwise.
    FbExportDir,
);

/// ★★★★★ §5.12 — the composition root's route from a backing token to a descriptor.
///
/// ⊘ **An alias and not a `#[cfg]` field**, for a reason the compiler enforces: attributes
/// are not allowed on tuple-struct fields or in tuple patterns, so a conditional field would
/// have had to become a conditional *tuple shape* — two arities of one type, and every
/// destructuring of it written twice. The alias keeps the shape constant and moves the
/// condition into what the shape CONTAINS, which is where it belongs: an archive with no
/// isolate plane has no directory to carry, and `()` says exactly that.
#[cfg(feature = "host-isolates")]
pub type FbExportDir = Option<kayfabe_isolate_host::isolate::ExportDirectory>;

/// An archive with no isolate plane has no descriptor to route. See the other definition.
#[cfg(not(feature = "host-isolates"))]
pub type FbExportDir = ();

/// Everything [`isolate_factory`] decides in one selection: the factory itself, the
/// guest-RAM block it adopted, and ★★★★★ §5.12's [`FbExportDir`] — the route from a backing
/// token to a descriptor, which can only be taken here, while the CONCRETE factory still
/// exists. One line later it is a `Box<dyn IsolateFactory>` and the route is gone for good.
///
/// ⊘ The three travel together **by type**, not by convention. Each is the *same selection's*
/// answer — the `BackingId` names the block whose descriptor the factory beside it holds, and
/// the [`FbExportDir`] is that same factory's registry — and `isolate_factory`'s own docs
/// already say they *"must never become a second one"*. A named triple says that where the
/// signature is read; the anonymous tuple it replaced said it only in prose.
///
/// ⚠ It also clears `clippy::type_complexity`, which `.github/workflows/ci.yml:452` runs as
/// `-D warnings` and which was failing on `origin/master` from `d95bc10` (2026-08-10) — the
/// commit that added the second element. `[measured 2026-08-11]` a `cargo clippy --workspace
/// --all-targets` with no `-D` reports it as one warning among the build-script notes, which
/// is how it survived: the CI form is the only one that makes it visible. ⊘ This alias
/// **replaces** the two-element `SelectedIsolateFactory` that was introduced for that fix;
/// keeping both would be two names for one selection, which is the defect the fix was for.
pub type IsolatePlaneParts = (
    Box<dyn kayfabe_isolate::IsolateFactory>,
    Option<kayfabe_vmm_qemu::layout::BackingId>,
    FbExportDir,
);

fn object_policy(
    driver: kayfabe_abi::versions::DriverAbiTable,
    // ★★★ E9/§13.6 option (2) — the SAME `ChipProfile::engines` slice the device-info
    // path serves the guest, so the bind check and the advertisement cannot be two
    // descriptions of one silicon.
    engines: &'static [kayfabe_abi::inittables::FifoDeviceEntry],
) -> Result<ObjectLink, (Status, &'static str)> {
    let isolate_plane = selected_isolate_plane()?;
    // ★★★ §4.4's missing link. Read ONCE, here, beside the plane it is checked against —
    // two readings of one environment variable is two facts that can disagree, which is
    // the shape this file already refuses for the probe set and for the isolate plane.
    let guest_ram = selected_guest_ram_source()?;
    let (isolates, guest_ram_backing, exports) = isolate_factory(isolate_plane, guest_ram)?;
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
    )
    // ★★★★★ §16.78 — the bisection budget, ARMED ONLY BY AN EXPLICIT ENVIRONMENT VALUE.
    // Unset (the shipped case) is `None`, which is exactly what `ObjectPolicy::over` already
    // stored, so this call is a no-op on every boot that does not ask for it.
    .with_mc_service_budget(selected_mc_service_budget());
    // ★★ ARMED runs SAY SO, loudly and at the top of the log, because every other line in
    // the run has to be read differently when this is on. ⊘ Silence when unset: a boot that
    // did not ask for the instrument must not carry a line about it.
    if let Some(budget) = selected_mc_service_budget() {
        eprintln!(
            "kayfabe: ⚠ {MC_SERVICE_BUDGET_ENV}={budget} — the MC_SERVICE_INTERRUPTS \
             bisection budget is ARMED. After {budget} NV_OK answers this port refuses \
             0x20801702, which TERMINATES the guest's unbounded 1 Hz wait. ⊘ This run is \
             evidence about WHAT THE GUEST WAS WAITING FOR, and is NOT evidence about what \
             this port serves."
        );
    }
    // ★ The handle is taken BEFORE the policy is boxed, because afterwards there is no
    // `ObjectPolicy` left to ask — that is the whole reason the census had to become a
    // shared store rather than a field behind `&self`.
    let refusals = policy.refusal_census();
    // ★★★★ §16.40 — same taken-before-boxing reason, one increment on.
    let promote_diag = policy.promote_diag();
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
        promote_diag,
        rings,
        isolates,
        publication_census,
        isolate_plane,
        device,
        guest_ram_backing,
        exports,
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

// =====================================================================================
// ★★★★★ THE GUEST-RAM CROSSING — where the isolate's view of guest memory comes from
// =====================================================================================

/// ★★★ The environment variable that arms the guest-RAM crossing.
///
/// `guest_ram_crossing.md` §4.4 named this as the one missing link: the isolate side of
/// the crossing landed on 2026-08-10 and **no VMM code called `with_guest_ram`**, so
/// nothing could reach `OS_DESCRIPTOR`, so nothing could reach the ring.
///
/// ## ⊘ Why the crossing is armed HERE and not by the hypervisor's launch flag
///
/// The tempting shape is *"if guest RAM happens to be a shared `memfd`, use it"* — no new
/// variable, one less thing to set. ⊘ That makes the boundary a **coincidence of how the
/// operator started the VM**. `HostIsolateFactory::with_guest_ram`'s own comment states
/// the rule the other way round: *"a factory that defaulted to granting guest RAM would be
/// granting it on every deployment that never asked, and the grant is the whole
/// boundary."* A hypervisor may be launched with `share=on` for a dozen unrelated reasons
/// (vhost-user, virtiofs, `ivshmem`), and none of them is a decision to let a GPU isolate
/// map the guest's memory.
///
/// ⇒ Two independent facts, and both are required: the operator's **launch** flag makes
/// the descriptor exist, and this variable is the operator **asking for it to cross**.
///
/// ⚠ Same process-global caveat as [`ISOLATE_PLANE_ENV`], for the same reason and with the
/// same owner: a hypervisor with two `nvkvm-gpu` devices gets one answer for both.
pub const GUEST_RAM_ENV: &str = "KAYFABE_GUEST_RAM";

/// ★★ The `memfd` creation name QEMU gives the machine's RAM backend.
///
/// ⊘ **It is the backend TYPE, not your `id=`** — `guest_ram_crossing.md` §1.1 trap 1. A
/// probe keyed on `ram0` (the id the bench's command line gives it) found **nothing** on a
/// boot where guest RAM was open the whole time, and an empty result reads as *"the
/// backend is not there"*. Measured on two physical boxes:
/// `/memfd:memory-backend-memfd (deleted)`.
pub const QEMU_MACHINE_RAM_MEMFD: &str = "memory-backend-memfd";

/// Where an isolate's view of guest RAM comes from, if anywhere.
///
/// ⊘ There is no `Auto`, for [`IsolatePlane`]'s reason: a source that silently degraded to
/// `none` when the descriptor was absent would make an armed run and its negative control
/// indistinguishable — and here it would do so *at the first doorbell*, hours into a boot,
/// rather than at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestRamSource {
    /// No isolate sees any guest memory. **The default**, and what every boot before this
    /// one did. Every `map_guest_ram` is `RmError::GuestRamUnavailable`, by name.
    None,
    /// ★ The hypervisor's own machine-RAM `memfd`, found in **this process** by
    /// [`kayfabe_linux_raw::MemfdCensus`].
    ///
    /// This is `guest_ram_crossing.md` §3's option **(A)** — zero new hypervisor surface —
    /// serving a **(B)**-shaped interface: what crosses is a descriptor plus an extent,
    /// and the only reason it is found by a `/proc/self/fd` census rather than asked for
    /// is that QEMU has no API to ask. A VMM that *does* (Cloud Hypervisor backs guest RAM
    /// with a `memfd` natively under `--memory shared=on`) supplies the same pair without
    /// a census.
    HypervisorMemfd,
}

impl GuestRamSource {
    /// The spelling this source is selected by. Round-trips with [`GuestRamSource::parse`].
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            GuestRamSource::None => "none",
            GuestRamSource::HypervisorMemfd => "memfd",
        }
    }

    /// Parse a source name. Exact and case-sensitive, as every other selector in this file.
    #[must_use]
    pub fn parse(s: &str) -> Option<GuestRamSource> {
        match s {
            "none" => Some(GuestRamSource::None),
            "memfd" => Some(GuestRamSource::HypervisorMemfd),
            _ => None,
        }
    }

    /// Every source this enum can express, for gates that must quantify over the whole set.
    pub const ALL: [GuestRamSource; 2] = [GuestRamSource::None, GuestRamSource::HypervisorMemfd];
}

/// The source named by `value` — the pure half of [`selected_guest_ram_source`].
///
/// # Errors
/// [`Status::Unsupported`] if `value` names no source. **Absent is not an error**; it is
/// [`GuestRamSource::None`].
pub fn guest_ram_source_from(
    value: Option<&str>,
) -> Result<GuestRamSource, (Status, &'static str)> {
    match value {
        None => Ok(GuestRamSource::None),
        Some(v) => GuestRamSource::parse(v).ok_or((
            Status::Unsupported,
            "KAYFABE_GUEST_RAM does not name a guest-RAM source: the only values are \
             `none` (the default) and `memfd`. It is not defaulted, because a typo that \
             silently selected `none` would leave every isolate blind to guest memory \
             while the run looked armed — and the symptom would appear at the first \
             doorbell, not here.",
        )),
    }
}

/// The source [`GUEST_RAM_ENV`] names, or [`GuestRamSource::None`] if it is unset.
///
/// # Errors
/// [`Status::Unsupported`] for a value that names no source, **including a non-UTF-8 one**
/// — which takes the `Some` arm, because it was SET.
fn selected_guest_ram_source() -> Result<GuestRamSource, (Status, &'static str)> {
    match std::env::var_os(GUEST_RAM_ENV) {
        None => Ok(GuestRamSource::None),
        Some(v) => guest_ram_source_from(Some(v.to_str().unwrap_or("\u{fffd}invalid"))),
    }
}

/// ★★ Asking for guest RAM on a plane that has no isolates is refused, **by name and at
/// startup**.
///
/// ⊘ Not tolerated as a harmless no-op. [`IsolatePlane::Stillborn`] retires every isolate
/// at birth, so there is nobody to hold the grant; an operator who set `KAYFABE_GUEST_RAM`
/// and left `KAYFABE_ISOLATES` unset has asked for a crossing that cannot happen, and the
/// run would otherwise look armed and behave exactly like the control. That is the
/// *"an evidence run and its own negative control indistinguishable"* failure this file
/// refuses everywhere else.
///
/// # Errors
/// [`Status::Unsupported`], naming both variables.
pub fn guest_ram_is_reachable_on(
    plane: IsolatePlane,
    source: GuestRamSource,
) -> Result<(), (Status, &'static str)> {
    match (plane, source) {
        (IsolatePlane::Stillborn, GuestRamSource::HypervisorMemfd) => Err((
            Status::Unsupported,
            "KAYFABE_GUEST_RAM=memfd asks for guest memory to cross into an isolate, and \
             KAYFABE_ISOLATES is `stillborn` — the plane that retires every isolate at \
             birth. There is nothing to grant it to. Set KAYFABE_ISOLATES=loopback or \
             =real, or unset KAYFABE_GUEST_RAM: a run that quietly granted nothing would \
             be indistinguishable from its own negative control.",
        )),
        _ => Ok(()),
    }
}

// =====================================================================================
// ★★★★★ §16.80 — WHICH EXECUTOR OWNS `Ce` DOORBELLS, asked separately from which plane
// =====================================================================================

/// ★★★★★ **§16.80** — the environment variable that names the executor for `Ce` doorbells.
///
/// # ⊘⊘ Why this exists: the plane selector was answering a question it cannot answer
///
/// [`SharedDoorbell::local_ce_is_the_only_executor`] was `isolate_plane ==
/// IsolatePlane::Stillborn`, and its own doc states the question correctly — *"is there any
/// other executor?"*. The defect is the **inference**: it read *"a real plane is installed"*
/// as *"a real plane can serve this"*, which is the same shape as the `vas_pdb.is_none()`
/// test it replaced (*the core can ADDRESS it, therefore the core can SERVE it*), one level
/// up. `accuracy_is_fatal_when_a_fallback_was_keyed_on_ignorance`, third instance.
///
/// ⚠ **And the host CE path says so at its own definition.** `kayfabe_isolate_host`'s
/// `HostRmBackend::ce_copy` refuses **two** things unconditionally and by name:
/// `CeExecutor::Ours` (*"needs the isolate's mapping of the fabricated aperture, which does
/// not exist"*) and `CeSource::Constant` (*"a fill is `LAUNCH_DMA` with `REMAP_ENABLE` plus
/// the `SET_REMAP_*` method block, which `kayfabe_abi::submit::ce` does not transcribe"*).
///
/// ★ **`RmInitAdapter`'s CeUtils scrubber is both at once**, so the host plane can serve
/// **zero** of the CE work the guest actually issues:
///
/// `[measured 2026-08-10, boot `w219_fe65678_realbase`, rev `fe65678`, `KAYFABE_ISOLATES=real`]`
/// — `kayfabe-isolate: CE-SUBMIT dst=0x40fa7c000 len=4 by=Ours src=Constant(0) → REFUSED
/// BEFORE SUBMISSION Other(19270)`, ×3, then `NVRM: RmInitNvDevice: *** Cannot load state
/// into the device` → `RmInitAdapter failed! (0x25:0x65:1249)`. Identical signature at
/// `w209_ffc80f8_real` (rev `ffc80f8`), nine commits and a whole interrupt plane earlier.
///
/// ⇒ **Selecting a real plane took the only working CE executor away and put nothing in its
/// place**, so the guest died ~40 s before `cuCtxCreate` exists. Every rung stated as
/// *"needs a live isolate plane"* was therefore unreachable in the shipped shape: turning the
/// plane on is the operation that removes the executor that gets the guest to the rung.
///
/// ⊘ **This is not a fallback-after-refusal and does not degrade anything.** It is a second
/// composition-root choice, made before any doorbell arrives, defaulting to the executor
/// that is measured to work. [`CeExecutorChoice::Host`] keeps the previously-measured arm exactly
/// reachable — a deleted configuration cannot be a control.
pub const CE_EXECUTOR_ENV: &str = "KAYFABE_CE_EXECUTOR";

/// ★★★★★ **§16.81 — DOES THE FORWARDING PLANE OWN THIS `Ce` DOORBELL?** The gate
/// [`SharedDoorbell::try_ce_submission`] asks before it reads a ring, as one predicate
/// instead of an expression inlined at its only call site.
///
/// # ⊘⊘ The term that was missing, and the tree already carried the rule
///
/// The two existing terms ask *"can the core address this channel?"* and *"is there any
/// other executor?"*. Neither asks the question that decides this doorbell:
/// **whose proc is it?**
///
/// `kayfabe_fwd::FwdFault::SystemDataPlane` is that rule, written down, with
/// `l1_concurrency.md` §12.26 behind it and naming this exact workload:
///
/// > *"The SYSTEM proc has no data plane … Guest-kernel work that would need a backing —
/// > **the CeUtils scrub**, the GR golden capture — is **forged** to the system proc's
/// > completion queue, **never forwarded**, so the system proc never mints host memory."*
///
/// ⇒ A [`kayfabe_core::gpu::Gpu::SYSTEM_PROC`] channel is the shell's on **every** plane
/// and under **every** value of [`CE_EXECUTOR_ENV`], because the rule is about the proc's
/// *lifetime regime*, not about which executors happen to be installed. Handing it away is
/// not a configuration that trades performance for reachability — it is a configuration
/// that asks a plane to do something the design forbids it to do.
///
/// # ★★★ The boot that measured the cost, and it printed the rule on the same doorbell
///
/// `[measured 2026-08-10, boot `w231a_ad4ed3c_ceexec_host`, rev `ad4ed3c`,
/// `KAYFABE_ISOLATES=real KAYFABE_CE_EXECUTOR=host`]` — **one** doorbell arrived,
/// `proc=0 chan=1`, this gate handed it to the forwarding plane, and three lines later:
///
/// - `GUEST-RAM PIN token=0x00010002 … proc=0 … ⊘ 0 of 1 run(s) pinned — REFUSED
///   `SystemDataPlane`, THE WALL, and it is a STANDING DESIGN RULE, not a defect.`
/// - `kayfabe-isolate: CE-SUBMIT dst=0x40fa7c000 len=4 by=Ours src=Constant(0) → REFUSED
///   BEFORE SUBMISSION Other(19270)`
/// - `doorbells: 1 arrived, 0 served, 1 REFUSED` → `NVRM: RmInitAdapter failed!
///   (0x25:0x65:1249)`, `nvidia-smi: No devices were found`.
///
/// ⇒ **The pin refused the very hand-off the executor gate had just made, by name, on the
/// same token, in the same boot.** One instrument stated the rule and the other never asked
/// it. ★ That is `no_counter_fired_is_not_no_record_exists` inverted: the record existed, was
/// printed, was correct, and no code read it.
///
/// # ⊘ What this is NOT
///
/// - ⊘ **Not a repair of `ce_copy`'s refusal, which is CORRECT and stays.**
///   `docs/design/ce_executor_tree.md` (owner, 2026-08-07) rules that *"the CPU branch
///   cannot execute in the isolate … so `ce_copy(Ours)` must keep refusing there"*, and calls
///   it *"the security boundary refusing to leak guest memory into the sandbox, working as
///   designed"* — explicitly superseding §12.4's *"the executor is the isolate in both
///   cases"*. The scrubber's fill into fabricated space is `CeExecutor::Ours` and no host
///   engine can ever be pointed at it.
/// - ⊘ **Not a narrowing of the forwarding plane.** Every **user** proc's `Ce` doorbell still
///   falls through exactly as before; the hand-off stays armed for the population it was
///   built for. This removes one proc from it — the one the tree already excluded.
/// - ⊘ **Not conditional on the executor choice**, deliberately. `local`, `host` and any
///   later value get the same answer for the system proc, because §12.26 is not a
///   performance preference.
/// # ★★★★★ 2026-08-11 — THE THIRD TERM NOW READS A DECLARED KIND, AND BOTH OF THE
/// OWNER'S AXES APPEAR IN IT
///
/// The term's subject never changed and its answer never changes: it is the same rule
/// and the same truth table. What changed is that it is no longer this gate's private
/// re-derivation of `proc != SYSTEM_PROC`. It reads
/// [`kayfabe_core::channel_kind::GuestChannelKind`], **declared once** at
/// `project::ProcBoundary::channel_kind` and carried on the channel — the fact this gate
/// was inlining, given a name and one owner.
///
/// ⊘ **And it asks the question through the HOST kind, deliberately.** The rule §12.26
/// states is not *"who is the guest"* but *"whose channel would carry this work"*: a
/// `Ce` doorbell is the forwarding plane's exactly when the host channel that may back
/// it is a [`kayfabe_core::channel_kind::HostChannelKind::Shadow`] — a channel in **that
/// guest process's own isolate**. An emulated channel's permitted host backing is a
/// `Scratchpad`, which is ours, which is why the shell keeps it *"whatever
/// `KAYFABE_CE_EXECUTOR` says"*. `hosted_by` is total and injective
/// (`channel_kind`'s own suite), so this is exactly as strong as
/// `kind == Passthrough` and says why.
///
/// ⚠ **A `ProcId` is no longer accepted here, and that is the point.** The parameter that
/// used to be a raw `ProcId` was one a caller could pass without ever asking this
/// question — which is precisely the shape of the defect: for twelve boots the gate had
/// the proc in hand and no term that read it. A caller now cannot supply anything but a
/// kind, and a kind has exactly one derivation.
#[must_use]
pub fn forwarding_plane_owns_ce(
    kind: kayfabe_core::channel_kind::GuestChannelKind,
    has_vas_pdb: bool,
    local_ce_is_the_only_executor: bool,
) -> bool {
    has_vas_pdb
        && !local_ce_is_the_only_executor
        && kind.hosted_by() == kayfabe_core::channel_kind::HostChannelKind::Shadow
}

/// Which executor owns `Ce` doorbells — [`CE_EXECUTOR_ENV`]'s vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CeExecutorChoice {
    /// The shell's own CPU copy-engine executor, on **every** plane. **The default**, and
    /// the only value under which a guest has ever reached `cuCtxCreate` with a live
    /// isolate plane installed.
    Local,
    /// Hand `Ce` doorbells whose channel the core can address to the forwarding plane —
    /// the arm `p2_29e7c25_planereal`, `w209_ffc80f8_real` and `w219_fe65678_realbase`
    /// measured. ⚠ On a `Stillborn` plane this value changes nothing: there is provably no
    /// other executor, so the first term of the decision still holds.
    Host,
}

impl CeExecutorChoice {
    /// Every executor, so a gate can quantify over the enum rather than over a
    /// hand-written list that shrinks in one place with nothing going red.
    pub const ALL: [CeExecutorChoice; 2] = [CeExecutorChoice::Local, CeExecutorChoice::Host];

    /// The name [`CE_EXECUTOR_ENV`] uses.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            CeExecutorChoice::Local => "local",
            CeExecutorChoice::Host => "host",
        }
    }

    /// Parse an exact, lowercase name. No aliases and no trimming — see
    /// [`isolate_plane_from`] for why a selector that is generous about spelling makes an
    /// evidence run and its own negative control indistinguishable.
    #[must_use]
    pub fn parse(s: &str) -> Option<CeExecutorChoice> {
        match s {
            "local" => Some(CeExecutorChoice::Local),
            "host" => Some(CeExecutorChoice::Host),
            _ => None,
        }
    }
}

/// The executor named by `value` — the pure half of [`selected_ce_executor`].
///
/// # Errors
/// [`Status::Unsupported`] if `value` names no executor. **Absent is not an error**; it is
/// [`CeExecutorChoice::Local`].
pub fn ce_executor_from(value: Option<&str>) -> Result<CeExecutorChoice, (Status, &'static str)> {
    match value {
        None => Ok(CeExecutorChoice::Local),
        Some(v) => CeExecutorChoice::parse(v).ok_or((
            Status::Unsupported,
            "KAYFABE_CE_EXECUTOR does not name an executor: the only values are `local` \
             (the default) and `host`. It is not defaulted, because a typo that silently \
             selected the other executor would make an evidence run and its own negative \
             control indistinguishable.",
        )),
    }
}

/// The executor [`CE_EXECUTOR_ENV`] names, or [`CeExecutorChoice::Local`] if it is unset.
///
/// # Errors
/// [`Status::Unsupported`] for a value that names no executor, **including a non-UTF-8
/// one** — which takes the `Some` arm, because it was SET and must not read as unset.
fn selected_ce_executor() -> Result<CeExecutorChoice, (Status, &'static str)> {
    match std::env::var_os(CE_EXECUTOR_ENV) {
        None => Ok(CeExecutorChoice::Local),
        Some(v) => ce_executor_from(Some(v.to_str().unwrap_or("\u{fffd}invalid"))),
    }
}

/// ★★★★★ **§5.12 — which chain, if any, materializes a framebuffer leaf.**
///
/// ⊘ **This REPLACES `KAYFABE_FB_BACKING`**, which armed `w228`'s vidmem chain. That chain
/// is not extended by this one and is not a fallback for it: a vidmem leaf is real card
/// memory with **no CPU view**, so the engine reads the card object and the guest reads the
/// emulator's own — two memories, silent in both directions. `fb_cpu_view.md` §0.1 measured
/// why the card object cannot grow the missing view, so there is nothing to keep armed.
///
/// | value | what it does |
/// |---|---|
/// | `off` (default) | no leaf is materialized at all; not one line is printed |
/// | `shared` | ★ the join: one backing, two mappings, one memory |
/// | `private` | ★★ **the negative control** |
///
/// ★★★ **`private` changes exactly ONE property** — the VMM maps the isolate's backing
/// `MAP_PRIVATE|MAP_ANONYMOUS` instead of `MAP_SHARED`, i.e.
/// `kayfabe_linux_raw::Backing::PrivateAnonymous`'s arm of the `mmap` argument computation
/// (`crates/kayfabe-linux-raw/src/mapping_unsafe.rs:344-347`). Everything either side of it
/// is the same code, the same isolate chain and the same establishment copy. ⊘ Not "a second
/// memfd", which would be a tautology — two different files obviously hold different bytes.
///
/// ⚠ A value that names nothing is **refused**, not defaulted, for [`CE_EXECUTOR_ENV`]'s
/// reason: a typo that silently disarmed the join would make an evidence run and its own
/// control indistinguishable, and the symptom would appear at the first GR doorbell.
pub const FB_JOIN_ENV: &str = "KAYFABE_FB_JOIN";

/// ★★★★★ **Which arm of the GR doorbell route this boot runs** — the passthrough route's
/// arming, and the ONLY thing standing between a `GrCompute` doorbell and the core's ring
/// path (`docs/design/gr_doorbell_passthrough.md`).
///
/// | value | what it does |
/// |---|---|
/// | `refuse` (default) | today's behaviour, byte for byte: `Route::NotACopyEngineChannel` |
/// | `passthrough` | ★ the doorbell is handed to `kayfabe_rt::device::SharedDevice::doorbell` |
///
/// # ⚠ Why this is armed and not simply switched on
///
/// The arm it opens was **closed on evidence**, at §16.65, and the reason is at the refusal
/// site: a GR doorbell used to fall through to exactly this server and was measured to be
/// *"a doorbell on a host channel into which the guest's methods were never copied"*. That
/// measurement still stands — `gr_doorbell_passthrough.md` §0.3 shows at the code that the
/// host GR channel's ring **and** its `GP_PUT` are both ours, so the engine fetches nothing
/// on either arm. ⇒ The armed arm buys the **transport**, not execution, and the two arms
/// exist so that a boot can say which one it ran from its own committed log.
///
/// ⚠ A value that names no arm is **refused**, not defaulted, for [`FB_JOIN_ENV`]'s reason:
/// a typo that silently disarmed the route would make an evidence run and its own control
/// indistinguishable, and the symptom — *"no GR doorbell was ever forwarded"* — is the
/// control's expected result.
pub const GR_ROUTE_ENV: &str = "KAYFABE_GR_ROUTE";

/// Which arm of the GR doorbell route a boot is running. See [`GR_ROUTE_ENV`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrRouteArm {
    /// The default and the control: refuse by name, exactly as every boot before this one.
    Refuse,
    /// ★ Passthrough: hand the doorbell to the core's ring path.
    Passthrough,
}

impl GrRouteArm {
    /// Every arm, so a test can quantify over them rather than restate the list — the
    /// property `every_ce_executor_round_trips_through_its_own_spelling` relies on one
    /// selector over.
    pub const ALL: [GrRouteArm; 2] = [GrRouteArm::Refuse, GrRouteArm::Passthrough];

    /// One word, for the boot's own log.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            GrRouteArm::Refuse => "refuse",
            GrRouteArm::Passthrough => "passthrough",
        }
    }

    /// Whether a `HostGr` doorbell is handed to the core on this arm — the bool
    /// [`kayfabe_rt::shell_disposition`] takes. ⊘ Named rather than spelled
    /// `== Passthrough` at the call site, so the two enums are joined in one place.
    #[must_use]
    pub fn gr_passthrough(self) -> bool {
        self == GrRouteArm::Passthrough
    }
}

/// Which arm `value` names — the pure half of [`selected_gr_route`].
///
/// # Errors
/// [`Status::Unsupported`] if `value` names no arm. **Absent is not an error**; it is
/// [`GrRouteArm::Refuse`].
pub fn gr_route_from(value: Option<&str>) -> Result<GrRouteArm, (Status, &'static str)> {
    match value {
        None | Some("refuse") => Ok(GrRouteArm::Refuse),
        Some("passthrough") => Ok(GrRouteArm::Passthrough),
        Some(_) => Err((
            Status::Unsupported,
            "KAYFABE_GR_ROUTE does not name an arm: the only values are `refuse` (the \
             default and the control — `Route::NotACopyEngineChannel`, exactly as every \
             boot before this one) and `passthrough` (the doorbell is handed to the core's \
             ring path). It is not defaulted, because a typo that silently disarmed the \
             route would make an evidence run and its own control indistinguishable — and \
             the control's expected result is `no GR doorbell was ever forwarded`, which is \
             precisely what a disarmed evidence run would also show. ⊘ `on`/`1` are not \
             accepted: this is a two-arm experiment, not a boolean.",
        )),
    }
}

/// Which arm [`GR_ROUTE_ENV`] names.
///
/// # Errors
/// [`Status::Unsupported`] for a value that names no arm, **including a non-UTF-8 one** —
/// which takes the `Some` arm, because it was SET and must not read as unset.
fn selected_gr_route() -> Result<GrRouteArm, (Status, &'static str)> {
    match std::env::var_os(GR_ROUTE_ENV) {
        None => Ok(GrRouteArm::Refuse),
        Some(v) => gr_route_from(Some(v.to_str().unwrap_or("\u{fffd}invalid"))),
    }
}

/// ★★★★★ **LEG A — whether this boot gives the framebuffer join a SECOND SOURCE: the
/// channel's own GPFIFO ring, joined BEFORE the host channel is born.**
///
/// | value | what it does |
/// |---|---|
/// | `off` (default) | today's behaviour, byte for byte. Not one `GR-RING-JOIN` line |
/// | `ring` | ★ the channel's declared `gpFifoOffset` is walked to its framebuffer leaf and that leaf is JOINED, at the engine-object latch — i.e. upstream in time of the host channel's birth |
///
/// # ★★★ Why this is a SECOND flag and not a third arm of [`GR_ROUTE_ENV`]
///
/// They arm different legs of the same stool and a boot must be able to run either without
/// the other. `KAYFABE_GR_ROUTE=passthrough` is leg **C** — the doorbell reaches the core.
/// This is leg **A** — the ring the host channel is born over. ⊘ Folding them into one
/// selector would make *"the doorbell was routed"* and *"the ring was joined"* one word, and
/// the whole point of `w260` was that the supply side moving and the execution side moving
/// are **different events** (`the_join_landed_and_the_wall_did_not_move`).
///
/// ⚠ A value that names no arm is **refused**, not defaulted, for [`FB_JOIN_ENV`]'s reason.
pub const GUEST_RING_ENV: &str = "KAYFABE_GUEST_RING";

/// Which arm of the guest-ring adoption a boot is running. See [`GUEST_RING_ENV`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestRingArm {
    /// The default and the control: the join's only source is the operand census, exactly
    /// as at `w260`.
    Off,
    /// ★ The channel's own ring is presented to the join, at the engine-object latch.
    Ring,
}

impl GuestRingArm {
    /// Every arm, so a test can quantify rather than restate.
    pub const ALL: [GuestRingArm; 2] = [GuestRingArm::Off, GuestRingArm::Ring];

    /// One word, for the boot's own log.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            GuestRingArm::Off => "off",
            GuestRingArm::Ring => "ring",
        }
    }

    /// Whether the channel's own ring is presented to the join on this arm.
    #[must_use]
    pub fn adopts_ring(self) -> bool {
        self == GuestRingArm::Ring
    }
}

/// Which arm `value` names — the pure half of [`selected_guest_ring`].
///
/// # Errors
/// [`Status::Unsupported`] if `value` names no arm. **Absent is not an error**; it is
/// [`GuestRingArm::Off`].
pub fn guest_ring_from(value: Option<&str>) -> Result<GuestRingArm, (Status, &'static str)> {
    match value {
        None | Some("off") => Ok(GuestRingArm::Off),
        Some("ring") => Ok(GuestRingArm::Ring),
        Some(_) => Err((
            Status::Unsupported,
            "KAYFABE_GUEST_RING does not name an arm: the only values are `off` (the default \
             and the control — the framebuffer join's only source is the operand census, \
             exactly as at w260) and `ring` (the channel's own declared gpFifoOffset is \
             walked to its framebuffer leaf and that leaf is joined, at the engine-object \
             latch, before the host channel is born). It is not defaulted, because a typo \
             that silently disarmed the adoption would make an evidence run and its own \
             control indistinguishable — and the control's expected result is `no GR-RING-JOIN \
             line was ever printed`, which is precisely what a disarmed evidence run would \
             also show. ⊘ `on`/`1` are not accepted: this is a two-arm experiment, not a \
             boolean.",
        )),
    }
}

/// ★★★★★ **LEG 4 — whether the guest-RAM PIN is given a second source: the pushbuffer VAs
/// this channel's own GPFIFO entries name.**
///
/// | value | what it does |
/// |---|---|
/// | `off` (default) | today's behaviour, byte for byte. Not one `PB-PIN` line |
/// | `pin` | ★ every non-zero, decodable GPFIFO entry of the channel's ring names an extent; each host page of each extent is resolved through the address table, refused unless it binds in **guest RAM**, coalesced into contiguous runs, and pinned FIXED at the guest's own VA |
///
/// # ⊘⊘ WHY THIS IS THE PIN AND NOT THE JOIN — the rung brief said the other one
///
/// The eight `Xid`-faulting pushbuffer VAs of `w263` resolve `pb=**S**:…` — **guest RAM** —
/// on both arms. `join_fb_leaf` serves the framebuffer plane and
/// [`kayfabe_rt::ceutils::resolve_leaf_of`] hands a sysmem resolution back as
/// `(Site::GuestRam, None)` **by construction**, saying in its own comment that *"the
/// guest-RAM pin owns that plane"*. See [`SharedDoorbell::pin_pushbuffer_guest_ram`] §0 and
/// `docs/design/w264_pushbuffer_pin_prereg.md`.
///
/// # ★★★ Why this is a THIRD flag and not a rider on [`GUEST_RING_ENV`]
///
/// Because `w263`'s own RESULT §3.1 records what happens otherwise: its harness exported
/// `KAYFABE_FB_JOIN=shared` on **both** arms, so its control was not its predecessor's and
/// six scorecard rows compared across boots anyway. Arms that differ in **one** variable each
/// need one variable per leg. ⚠ A value that names no arm is **refused**, not defaulted, for
/// [`FB_JOIN_ENV`]'s reason.
pub const GUEST_PUSHBUF_ENV: &str = "KAYFABE_GUEST_PUSHBUF";

/// Which arm of the pushbuffer pin a boot is running. See [`GUEST_PUSHBUF_ENV`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestPushbufArm {
    /// The default and the control: the guest-RAM pin's only source stays the channel's
    /// **ring** VA, exactly as at `w263` — where it refused all eight by name, correctly,
    /// because a ring in this workload is in Vidmem.
    Off,
    /// ★ The pushbuffer VAs the ring's own GPFIFO entries name are presented to the pin.
    Pin,
}

impl GuestPushbufArm {
    /// Every arm, so a test can quantify rather than restate.
    pub const ALL: [GuestPushbufArm; 2] = [GuestPushbufArm::Off, GuestPushbufArm::Pin];

    /// One word, for the boot's own log.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            GuestPushbufArm::Off => "off",
            GuestPushbufArm::Pin => "pin",
        }
    }

    /// Whether the pushbuffer VAs are presented to the guest-RAM pin on this arm.
    #[must_use]
    pub fn pins(self) -> bool {
        self == GuestPushbufArm::Pin
    }
}

/// Which arm `value` names — the pure half of [`selected_guest_pushbuf`].
///
/// # Errors
/// [`Status::Unsupported`] if `value` names no arm. **Absent is not an error**; it is
/// [`GuestPushbufArm::Off`].
pub fn guest_pushbuf_from(value: Option<&str>) -> Result<GuestPushbufArm, (Status, &'static str)> {
    match value {
        None | Some("off") => Ok(GuestPushbufArm::Off),
        Some("pin") => Ok(GuestPushbufArm::Pin),
        Some(_) => Err((
            Status::Unsupported,
            "KAYFABE_GUEST_PUSHBUF does not name an arm: the only values are `off` (the \
             default and the control — the guest-RAM pin's only source stays the channel's \
             ring VA, exactly as at w263) and `pin` (the pushbuffer VAs the ring's own GPFIFO \
             entries name are resolved through the address table and pinned FIXED at the \
             guest's own VAs). It is not defaulted, because a typo that silently disarmed the \
             pin would make an evidence run and its own control indistinguishable — the \
             control's expected result is `no PB-PIN line was ever printed`, which is exactly \
             what a disarmed evidence run also shows. ⊘ `on`/`1` are not accepted: this is a \
             two-arm experiment, not a boolean.",
        )),
    }
}

/// Which arm [`GUEST_PUSHBUF_ENV`] names.
///
/// # Errors
/// [`Status::Unsupported`] for a value that names no arm, **including a non-UTF-8 one** —
/// which takes the `Some` arm, because it was SET and must not read as unset.
fn selected_guest_pushbuf() -> Result<GuestPushbufArm, (Status, &'static str)> {
    match std::env::var_os(GUEST_PUSHBUF_ENV) {
        None => Ok(GuestPushbufArm::Off),
        Some(v) => guest_pushbuf_from(Some(v.to_str().unwrap_or("\u{fffd}invalid"))),
    }
}

/// Which arm [`GUEST_RING_ENV`] names.
///
/// # Errors
/// [`Status::Unsupported`] for a value that names no arm, **including a non-UTF-8 one** —
/// which takes the `Some` arm, because it was SET and must not read as unset.
fn selected_guest_ring() -> Result<GuestRingArm, (Status, &'static str)> {
    match std::env::var_os(GUEST_RING_ENV) {
        None => Ok(GuestRingArm::Off),
        Some(v) => guest_ring_from(Some(v.to_str().unwrap_or("\u{fffd}invalid"))),
    }
}

/// Which arm of the framebuffer-leaf join a boot is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FbJoinArm {
    /// Nothing is materialized. The arming control.
    Off,
    /// ★ The join. `MAP_SHARED` — the VMM's view and the isolate's view are one memory.
    Shared,
    /// ★★ The negative control. `MAP_PRIVATE|MAP_ANONYMOUS` on the VMM's side only.
    Private,
}

impl FbJoinArm {
    /// One word, for the boot's own log.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            FbJoinArm::Off => "off",
            FbJoinArm::Shared => "shared",
            FbJoinArm::Private => "private",
        }
    }

    /// Whether this arm materializes anything at all.
    #[must_use]
    pub fn armed(self) -> bool {
        self != FbJoinArm::Off
    }
}

/// Which arm `value` names — the pure half of [`selected_fb_join`].
///
/// # Errors
/// [`Status::Unsupported`] if `value` names no arm. **Absent is not an error**; it is
/// [`FbJoinArm::Off`].
pub fn fb_join_from(value: Option<&str>) -> Result<FbJoinArm, (Status, &'static str)> {
    match value {
        None | Some("off") => Ok(FbJoinArm::Off),
        Some("shared") => Ok(FbJoinArm::Shared),
        Some("private") => Ok(FbJoinArm::Private),
        Some(_) => Err((
            Status::Unsupported,
            "KAYFABE_FB_JOIN does not name an arm: the only values are `off` (the default), \
             `shared` (the join) and `private` (the negative control). It is not defaulted, \
             because a typo that silently disarmed the join would make an evidence run and \
             its own control indistinguishable — and the symptom would appear at the first \
             GR doorbell, not here. ⊘ `on` was KAYFABE_FB_BACKING's spelling and it is gone: \
             the vidmem chain it armed is superseded, not renamed.",
        )),
    }
}

/// Which arm [`FB_JOIN_ENV`] names.
///
/// # Errors
/// [`Status::Unsupported`] for a value that names no arm, **including a non-UTF-8 one** —
/// which takes the `Some` arm, because it was SET and must not read as unset.
fn selected_fb_join() -> Result<FbJoinArm, (Status, &'static str)> {
    match std::env::var_os(FB_JOIN_ENV) {
        None => Ok(FbJoinArm::Off),
        Some(v) => fb_join_from(Some(v.to_str().unwrap_or("\u{fffd}invalid"))),
    }
}

/// ★★★★★ **§16.82** — the environment variable that arms
/// [`SharedDoorbell::witness_executor_fb_pages`]: witness the framebuffer pages the shell's
/// **own CPU copy-engine executor** created, which G1's window-only transport cannot see.
///
/// ⊘ **Off by default, and refusing an unknown value**, for [`FB_BACKING_ENV`]'s stated
/// reason: with it unset this port witnesses exactly what `b6c5442` witnessed, so the
/// disarmed boot **is** the negative control and a typo cannot silently produce one.
pub const PT_WITNESS_EXEC_ENV: &str = "KAYFABE_PT_WITNESS_EXEC";

/// Whether `value` arms the executor witness — the pure half of [`selected_pt_witness_exec`].
///
/// # Errors
/// [`Status::Unsupported`] if `value` names neither state. **Absent is not an error**; it
/// is `false`.
pub fn pt_witness_exec_from(value: Option<&str>) -> Result<bool, (Status, &'static str)> {
    match value {
        None | Some("off") => Ok(false),
        Some("on") => Ok(true),
        Some(_) => Err((
            Status::Unsupported,
            "KAYFABE_PT_WITNESS_EXEC does not name a state: the only values are `off` (the \
             default) and `on`. It is not defaulted, because the disarmed arm IS this \
             rung's negative control and a typo that silently disarmed it would make the \
             evidence run and the control indistinguishable.",
        )),
    }
}

/// Whether [`PT_WITNESS_EXEC_ENV`] arms the executor witness.
///
/// ⊘ A value that names neither state reads as **disarmed** here rather than aborting the
/// device: this is a diagnostic-plus-populate flag consulted per doorbell, not a
/// composition-root decision, and the line it prints states the arm it took either way.
#[must_use]
fn selected_pt_witness_exec() -> bool {
    match std::env::var_os(PT_WITNESS_EXEC_ENV) {
        None => false,
        Some(v) => {
            pt_witness_exec_from(Some(v.to_str().unwrap_or("\u{fffd}invalid"))).unwrap_or(false)
        }
    }
}

/// ★★★★★ **§16.78** — the environment variable that arms the `MC_SERVICE_INTERRUPTS`
/// bisection budget. See [`kayfabe_rmrpc::ObjectPolicy::with_mc_service_budget`].
pub const MC_SERVICE_BUDGET_ENV: &str = "KAYFABE_MC_SERVICE_BUDGET";

/// The budget [`MC_SERVICE_BUDGET_ENV`] names, or `None` when it is unset.
///
/// ⊘ **A value that does not parse yields `None`, and that is deliberate the opposite way
/// round from [`selected_isolate_plane`].** There, a bad value must fail the build-up
/// because it names something the port would otherwise silently not do. Here, the safe
/// state is *unarmed*: an instrument that turns itself on because somebody typed `yes`
/// would be a diagnostic that fires without being asked, which is the one thing this field
/// must never do. ⚠ It is reported in the teardown census either way, so an armed run and a
/// mistyped one are not confusable after the fact.
#[must_use]
pub fn selected_mc_service_budget() -> Option<u32> {
    std::env::var_os(MC_SERVICE_BUDGET_ENV)?
        .to_str()
        .and_then(|s| s.trim().parse::<u32>().ok())
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
/// ★ The second element is the **adopted guest-RAM block's filesystem identity**, or `None`
/// when nothing was adopted. It leaves this function beside the factory that holds the
/// descriptor, because it is the *same selection's* answer and must never become a second
/// one.
pub fn isolate_factory(
    plane: IsolatePlane,
    guest_ram: GuestRamSource,
) -> Result<IsolatePlaneParts, (Status, &'static str)> {
    guest_ram_is_reachable_on(plane, guest_ram)?;
    match plane {
        IsolatePlane::Stillborn => Ok((
            Box::new(kayfabe_isolate::StillbornIsolates::new(STILLBORN_WHY)),
            None,
            FbExportDir::default(),
        )),
        #[cfg(feature = "host-isolates")]
        IsolatePlane::Loopback => {
            let (f, id) = with_guest_ram(
                kayfabe_isolate_host::HostIsolateFactory::new(
                    kayfabe_isolate_host::RmMode::Loopback,
                ),
                guest_ram,
            )?;
            let exports = f.export_directory();
            Ok((Box::new(f), id, Some(exports)))
        }
        #[cfg(feature = "host-isolates")]
        IsolatePlane::Real => {
            let (f, id) = with_guest_ram(
                kayfabe_isolate_host::HostIsolateFactory::new(kayfabe_isolate_host::RmMode::Real),
                guest_ram,
            )?;
            let exports = f.export_directory();
            Ok((Box::new(f), id, Some(exports)))
        }
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

/// ★★★★★ Route the shim to the hypervisor's **own** guest-RAM descriptor, and refuse by
/// name at startup if it is not there.
///
/// # ⊘⊘ The refusal is here and not at the first doorbell, and that is the whole point
///
/// Whether guest RAM is shareable is a **deployment fact no code gate can observe** — it
/// is a command-line flag on a process we did not start. `RmError::GuestRamUnavailable`
/// exists for exactly that and is the right answer at the seam; but it would surface
/// twenty seconds into a boot, inside a doorbell, as one more refusal in a log full of
/// them. An operator who forgot `NVKVM_RAM_BACKEND=memfd` must be told **before the guest
/// runs an instruction**.
///
/// # ★ The census is PRINTED on refusal, always
///
/// `guest_ram_crossing.md` §1.1 trap 1 is a probe that searched for the wrong name, found
/// nothing, and read as *"the backend is not there"* — on a boot where guest RAM was open
/// at a descriptor the whole time. The absence of a match is not the absence of the thing,
/// and the only cure is showing what **was** seen. So every `memfd` in the process goes to
/// the log on the failing path, with its name, its size and whether it is shared-mapped.
///
/// # Errors
/// [`Status::Unsupported`] when the census cannot be taken, when no shared-mapped `memfd`
/// carries [`QEMU_MACHINE_RAM_MEMFD`], or when more than one does.
#[cfg(feature = "host-isolates")]
fn with_guest_ram(
    factory: kayfabe_isolate_host::HostIsolateFactory,
    source: GuestRamSource,
) -> Result<
    (
        kayfabe_isolate_host::HostIsolateFactory,
        Option<kayfabe_vmm_qemu::layout::BackingId>,
    ),
    (Status, &'static str),
> {
    if source == GuestRamSource::None {
        return Ok((factory, None));
    }
    let census = kayfabe_linux_raw::MemfdCensus::take_of_this_process().map_err(|_| {
        (
            Status::Unsupported,
            "KAYFABE_GUEST_RAM=memfd needs to enumerate this process's own descriptors and \
             /proc/self/fd could not be listed. That is a deployment fault (no /proc), not \
             an empty answer, and it is refused rather than read as `no guest RAM`.",
        )
    })?;
    // ★ The census goes to the log on BOTH paths, before the decision. On the failing path
    // it is the only evidence an operator has; on the succeeding path it is what makes a
    // later "which block did it take?" answerable from the boot log alone.
    for c in census.seen() {
        eprintln!(
            "kayfabe: memfd census — name={:?} bytes={} shared_mapped={} (listed at fd {}, \
             reported only: this number MOVED between two physical benches and is never \
             matched on)",
            c.name(),
            c.bytes(),
            c.shared_mapped(),
            c.listed_as()
        );
    }
    match census.the_only_shared_memfd_named(QEMU_MACHINE_RAM_MEMFD) {
        Ok(found) => {
            // ★★★ §5.7 — the identity is taken HERE, at the one instant a block was
            // selected, and travels with the descriptor. `into_descriptor` consumes the
            // candidate, so this is also the last moment it can be taken without asking the
            // question a second time.
            let backing = kayfabe_vmm_qemu::layout::BackingId::new(found.dev(), found.inode());
            eprintln!(
                "kayfabe: ★★★ GUEST-RAM CROSSING ARMED — adopted the hypervisor's {QEMU_MACHINE_RAM_MEMFD} \
                 block, {} bytes, dev={} ino={}. Every isolate this factory spawns is granted \
                 a view of it at a fixed descriptor number; nothing is mapped until a grant \
                 says so. ⊘ The size is an EXTENT, not a LAYOUT — where in the guest's \
                 physical space these bytes appear is stated separately, by the hypervisor's \
                 own topology callbacks, and is reported when the memory plane attaches.",
                found.bytes(),
                backing.dev,
                backing.ino
            );
            let (fd, bytes) = found.into_descriptor();
            Ok((factory.with_guest_ram(fd, bytes), Some(backing)))
        }
        Err(kayfabe_linux_raw::MemfdRefusal::NoSuchMemfd) => Err((
            Status::Unsupported,
            "KAYFABE_GUEST_RAM=memfd, and no shared-mapped `memfd` named \
             `memory-backend-memfd` is open in this process — see the census above for \
             what IS. Guest RAM is shareable only if the VM was LAUNCHED that way: \
             `-object memory-backend-memfd,id=ram0,size=<N>,share=on -machine \
             memory-backend=ram0`, with `-m <N>` matching exactly. ⊘ Refused here rather \
             than at the first doorbell, and never degraded to a copy: the guest POLLS, so \
             a copy has no trigger point at which to be refreshed.",
        )),
        Err(kayfabe_linux_raw::MemfdRefusal::Ambiguous { .. }) => Err((
            Status::Unsupported,
            "KAYFABE_GUEST_RAM=memfd, and MORE THAN ONE shared-mapped `memfd` named \
             `memory-backend-memfd` is open in this process — see the census above. This \
             is refused rather than resolved by a tie-break: every tie-break available \
             (lowest descriptor, largest block, first listed) is a rule keyed on position, \
             and the descriptor number of guest RAM was measured MOVING between two \
             physical benches running the same image.",
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

/// ★★★★★ **LEG B's COMPLETION READ — `GP_GET` and `GP_PUT` out of the guest's OWN USERD.**
///
/// # ⊘⊘ Why this exists at all, and why it is HERE and not in the isolate
///
/// `traces/boots/w262/RESULT.md` §3.3 names the hole: *"the line says RM was **told** the
/// guest's ring at channel creation. Nothing here reads `GP_GET`."*
/// `admitted_and_served_are_different_gates`.
///
/// And after leg B the isolate **cannot** close it. `[measured, R31 arm B]` a guest-backed
/// `OS_DESCRIPTOR` cannot be CPU-mapped, so `HostRmBackend::userd_cursors` refuses by name
/// (`USERD_NOT_OURS`) on exactly the channels this rung cares about. ⇒ The read has to happen
/// on the side that owns the framebuffer store — here — through the same `fb_peek` the ring
/// dumps use. ★ That is R32's **J2** shape (GPU-write → CPU-read through the *other* mapping
/// of a described memfd), which `[measured 2026-08-11, f58473f]` holds.
///
/// # ★★★ What each answer means, and none of them may be merged
///
/// | token | reading |
/// |---|---|
/// | *(absent)* | the params carried no framebuffer USERD — there is no address to read |
/// | `fbuserd@0x…=REFUSED(…)` | the store would not serve those bytes |
/// | `fbuserd@0x… GET=0 PUT=0 resN-NEVER-WRITTEN` | ⚠ **nobody has written this page at all** — including RM's own zeroing at channel creation, so the channel was never born over it. ⊘ Only meaningful on an **unjoined** page; see below |
/// | `fbuserd@0x… … JOINED-one-memory` | the page is inside a joined leaf, so residency is not a question this store can answer — its local pages for that range were removed at install. The **values** are still live and correct |
/// | `fbuserd@0x… GET=n PUT=m resY` | the live cursors. ★★★★★ `GET == PUT != 0` is the engine having **fetched**; `GET == 0 PUT != 0` is the wall this campaign is on |
///
/// ⚠⚠ **`GET = 0, PUT = 0` is the ambiguous null and is reported with its residency for
/// exactly that reason.** A channel that never ran, a page RM zeroed and nobody advanced, and
/// a page we are reading at the wrong address all produce it. Residency separates the third.
///
/// ⊘ **It reads and it decides nothing.** No branch anywhere consumes this string.
fn fb_userd_cursors(
    plane: &kayfabe_device::plane::RegPlane,
    userd: Option<kayfabe_core::rmgraph::DeclaredUserd>,
) -> String {
    // ⊘ Only the framebuffer arm has an address this store can serve. A `Sysmem` USERD is a
    // real and legal case whose bytes live in guest RAM, and reading its guest-physical
    // address out of the framebuffer would produce a confident wrong number.
    let Some(base) = userd.and_then(|u| u.framebuffer_base()) else {
        return String::new();
    };
    // `GP_GET` and `GP_PUT` are one dword apart at the head of the 512-byte slot; eight bytes
    // is the whole read. ⊘ The offsets are `kayfabe_abi::submit`'s, never spelled here.
    let mut w = [0u8; 8];
    let at = base + kayfabe_abi::submit::USERD_GP_GET;
    match plane.fb_peek(at, &mut w) {
        Err(why) => format!(" fbuserd@0x{at:x}=REFUSED({why})"),
        Ok(()) => {
            // ⊘⊘ **THE JOIN IS CHECKED FIRST, AND FINDING THAT OUT BEFORE THE BOOT IS THE
            // POINT.** `SparseFb::install_join` **removes the local pages** for a joined
            // range — that is what makes the join one memory rather than two — so
            // `is_resident` (which asks `self.pages.contains_key`) answers `Some(false)` for
            // every joined address. ⇒ On the ARMED arm, where the USERD page is inside the
            // joined leaf, an unqualified residency token would print
            // `resN-NEVER-WRITTEN` on a page whose bytes are live and correctly read.
            //
            // ⚠ That is precisely the reading this rung's pre-registration flagged as *"looks
            // exactly like the wall and would actually mean the instrument is at the wrong
            // address"* — and it would have fired on **every** armed row, systematically, in
            // the favourable-looking direction for a wrong conclusion. `FbStore::read` gets
            // this right (the join is checked first there too); `is_resident` was never
            // widened to match, and no caller had asked it about a joined address before.
            let joined = plane
                .joined_fb_ranges()
                .into_iter()
                .any(|(phys, len)| at >= phys && at - phys < len);
            let res = if joined {
                "JOINED-one-memory"
            } else {
                match plane.fb_is_resident(at) {
                    None => "res?",
                    Some(true) => "resY",
                    Some(false) => "resN-NEVER-WRITTEN",
                }
            };
            format!(
                " fbuserd@0x{at:x} GET={} PUT={} {res}",
                u32::from_le_bytes([w[0], w[1], w[2], w[3]]),
                u32::from_le_bytes([w[4], w[5], w[6], w[7]]),
            )
        }
    }
}

/// ★★★★★ **LEG 4's pure half — the page derivation and the run coalescing, quantified.**
///
/// ⊘ These are the two places a boot's `PB-PIN` line can be wrong in a way no boot can show:
/// the **cap** cannot be reached by the only measured workload (`w263` names one extent per
/// ring), and the **many-runs** case is the one `RING_PIN_BYTES`' own docs measure but no
/// green log distinguishes from the one-run case. `a_census_zero_needs_a_known_positive`.
#[cfg(test)]
mod pushbuffer_pin_tests {
    use super::{PUSHBUF_MAX_PAGES, pushbuffer_pages, pushbuffer_runs};

    const PAGE: u64 = 4096;

    /// ★★★ **The measured shape.** `w263`'s eight channels each name ONE extent, 32 bytes
    /// long, at a 2 MiB-aligned VA. One page each, one run each, no cap, no drops.
    #[test]
    fn the_w263_shape_is_one_page_and_one_run_per_extent() {
        // `gp[0]@0x200218000=0x202400000+0x20` — the real entry, byte for byte.
        let (pages, dropped, first) = pushbuffer_pages(&[(0, 0x2_0240_0000, 0x20)], PAGE);
        assert_eq!(pages.iter().copied().collect::<Vec<_>>(), vec![0x2_0240_0000]);
        assert_eq!(dropped, 0);
        assert_eq!(first, None);
        let runs = pushbuffer_runs(&[(0x2_0240_0000, 0x3d45_f000)], PAGE);
        assert_eq!(runs, vec![(0x2_0240_0000, 0x3d45_f000, PAGE)]);
    }

    /// ★★ An extent that **straddles** a page boundary contributes BOTH pages. ⊘ Derived from
    /// the extent's own length, never from a stride — the address the guest named plus the
    /// bytes it named, and nothing else.
    #[test]
    fn an_extent_spanning_a_page_boundary_names_both_pages() {
        let (pages, _, _) = pushbuffer_pages(&[(0, 0x1_0000_0FF0, 0x40)], PAGE);
        assert_eq!(
            pages.iter().copied().collect::<Vec<_>>(),
            vec![0x1_0000_0000, 0x1_0000_1000],
            "0xFF0 + 0x40 crosses into the next page"
        );
    }

    /// ★★★★★ **THE CAP FIRES, IT COUNTS WHAT IT DROPPED, AND IT NAMES ONE.**
    ///
    /// ⚠ This is the arm no boot can exercise, and a cap that truncated silently would be a
    /// **false green** — the report would read as a complete pass. The assertion is on all
    /// three: the admitted count, the dropped count, and that a dropped VA is *named*.
    #[test]
    fn the_page_cap_fires_and_reports_the_count_and_the_first_dropped_address() {
        // One extent long enough to demand twice the cap in pages.
        let want = PUSHBUF_MAX_PAGES as u64 * 2;
        let (pages, dropped, first) = pushbuffer_pages(&[(0, 0x4_0000_0000, want * PAGE)], PAGE);
        assert_eq!(pages.len(), PUSHBUF_MAX_PAGES, "admits exactly the cap");
        assert_eq!(
            dropped,
            (want as usize) - PUSHBUF_MAX_PAGES,
            "every page over the cap is COUNTED, not silently lost"
        );
        assert_eq!(
            first,
            Some(0x4_0000_0000 + PUSHBUF_MAX_PAGES as u64 * PAGE),
            "and the first dropped address is NAMED, so `some were dropped` is never \
             printable without one of them"
        );
    }

    /// ⊘ A page admitted by one extent is not charged again by another — otherwise the number
    /// in the report would not be the number of pages.
    #[test]
    fn two_extents_sharing_a_page_spend_one_slot() {
        let (pages, dropped, _) =
            pushbuffer_pages(&[(0, 0x1000, 0x10), (7, 0x1FF0, 0x10)], PAGE);
        assert_eq!(pages.len(), 1);
        assert_eq!(dropped, 0);
    }

    /// ★★★ **Contiguity is required in BOTH spaces.** `RING_PIN_BYTES`' own docs measure four
    /// consecutive guest **virtual** pages landing on four scattered guest **physical** ones,
    /// so this is the expected shape and not an edge case.
    #[test]
    fn runs_split_on_a_gpa_discontinuity_and_on_a_va_discontinuity() {
        // VA-contiguous, GPA-scattered ⇒ four runs.
        let scattered = pushbuffer_runs(
            &[
                (0x4_2006_4000, 0x0768_a000),
                (0x4_2006_5000, 0x0521_c000),
                (0x4_2006_6000, 0x0850_5000),
                (0x4_2006_7000, 0x0764_f000),
            ],
            PAGE,
        );
        assert_eq!(scattered.len(), 4, "a GPA gap splits: {scattered:?}");

        // GPA-contiguous but a VA HOLE ⇒ still two runs, because each run is placed FIXED at
        // its own VA and merging them would describe bytes at an address nobody named.
        let va_hole = pushbuffer_runs(&[(0x1000, 0xA000), (0x3000, 0xB000)], PAGE);
        assert_eq!(va_hole.len(), 2, "a VA gap splits too: {va_hole:?}");

        // Contiguous in both ⇒ ONE run of two pages. ⊘ The positive control: without it the
        // two assertions above are satisfied by a function that never merges anything.
        let merged = pushbuffer_runs(&[(0x1000, 0xA000), (0x2000, 0xB000)], PAGE);
        assert_eq!(merged, vec![(0x1000, 0xA000, 2 * PAGE)]);
    }

    /// ⊘ A zero-length extent names no page and cannot wrap. Unreachable from
    /// `gp_entry_decode` (which refuses `LENGTH == 0`), handled rather than assumed away.
    #[test]
    fn a_zero_length_extent_names_no_page_and_does_not_wrap() {
        let (pages, dropped, first) = pushbuffer_pages(&[(0, 0x1000, 0)], PAGE);
        assert!(pages.is_empty());
        assert_eq!((dropped, first), (0, None));
    }
}

#[cfg(test)]
mod ring_scan_sentence_tests {
    use super::{
        ENGINE_FWD_REPORT_MAX, EngineFwdReport, engine_fwd_report_action, ring_scan_sentence,
    };

    /// ★★★★★ **The regression this function was extracted for.** A scan in which *nothing
    /// resolved* must not claim completeness and must not claim the entries were zero.
    #[test]
    fn a_scan_that_read_nothing_says_so_and_claims_nothing_about_the_ring() {
        let s = ring_scan_sentence(1024, 1024, 1024, &[]);
        assert!(s.contains("NOTHING WAS READ"), "{s}");
        assert!(
            !s.contains("COMPLETE"),
            "★★★ a scan that resolved zero entries called itself COMPLETE on four committed \
             boots (s45..s48) — that is the exact falsehood this test exists to hold down: {s}"
        );
        assert!(
            !s.contains("every scanned entry is ZERO"),
            "★★★ `nonzero` is empty here because nothing was ever appended to it, not \
             because the entries were zero. Reporting the second is inventing a measurement \
             out of a failure: {s}"
        );
    }

    /// ⊘ Non-vacuity: a scan that DID read reports normally, so the guard above is not
    /// simply suppressing the whole sentence.
    #[test]
    fn a_scan_that_read_everything_still_reports_completeness_and_zeros() {
        let s = ring_scan_sentence(1024, 1024, 0, &[]);
        assert!(s.contains("COMPLETE"), "{s}");
        assert!(s.contains("every scanned entry is ZERO"), "{s}");
        assert!(!s.contains("NOTHING WAS READ"), "{s}");
    }

    /// ★★ A PARTIAL read is a third state and must read as one: it reports what resolved,
    /// with the denominator of the entries that resolved — never the loop bound.
    #[test]
    fn a_partial_read_reports_the_resolved_denominator_and_not_the_loop_bound() {
        let s = ring_scan_sentence(1024, 1024, 1000, &[]);
        assert!(!s.contains("NOTHING WAS READ"), "{s}");
        assert!(
            s.contains("NONE among the 24 entries that RESOLVED"),
            "★ 24 resolved, not 1024 — a partial read that says `every scanned entry is \
             ZERO` is the same conflation one order of magnitude smaller: {s}"
        );
    }

    /// ⊘ A non-zero entry is still reported when some reads failed — the guard must not
    /// swallow a real finding.
    #[test]
    fn a_nonzero_entry_survives_a_partial_read() {
        let s = ring_scan_sentence(64, 1024, 60, &["[7]=0x00000000deadbeef".to_string()]);
        assert!(s.contains("deadbeef"), "{s}");
        assert!(s.contains("BOUNDED-READING"), "{s}");
    }

    /// ★★★★★ **§16.107 — the whole real workload fits in a row budget now**, so the shape
    /// that saturated twice cannot saturate a third time at the same size.
    ///
    /// ⊘ 32 forwards is what `w255` measured with §16.106's fix; 34 outcomes is the whole
    /// census. Both must be **rows**, with room to spare.
    #[test]
    fn the_measured_workload_prints_every_row() {
        for nth in 1..=34 {
            assert_eq!(
                engine_fwd_report_action(nth, nth),
                EngineFwdReport::Row,
                "outcome {nth} of the w255 shape must be a full row"
            );
        }
        // ⊘ A `const` assertion: the bound must clear the largest observed shape by an
        // order of magnitude, not by one, or the next workload saturates it again.
        const { assert!(ENGINE_FWD_REPORT_MAX >= 32 * 8) };
    }

    /// ★★★ **The count can never go silent**, which is the property `forwarded=32` lacked
    /// and the reason this rung exists at all.
    #[test]
    fn past_the_row_budget_the_totals_keep_coming() {
        let over = ENGINE_FWD_REPORT_MAX + 1;
        // Rows stop…
        assert_ne!(engine_fwd_report_action(over, over), EngineFwdReport::Row);
        // …and the totals do not: every power of two still speaks, forever.
        for p in 9..=40 {
            let seen = 1u64 << p;
            assert_eq!(
                engine_fwd_report_action(over, seen),
                EngineFwdReport::TotalsOnly,
                "seen={seen} must still state the totals"
            );
        }
        // ⊘ …and in between it is silent, which is what bounds a hostile guest to ~log2(n).
        assert_eq!(
            engine_fwd_report_action(over, (1u64 << 20) + 1),
            EngineFwdReport::Silent
        );
    }

    /// ⊘ The schedule is keyed on `seen`, not on the per-class `nth` — a workload that
    /// saturates one class and then feeds only that class must not be able to stall the
    /// totals by leaving the other class's index frozen.
    #[test]
    fn the_totals_schedule_does_not_stall_on_one_class() {
        let frozen_other_class = ENGINE_FWD_REPORT_MAX + 1;
        assert_eq!(
            engine_fwd_report_action(frozen_other_class, 4096),
            EngineFwdReport::TotalsOnly,
            "seen advanced, so the census speaks regardless of which class is saturated"
        );
    }
}
