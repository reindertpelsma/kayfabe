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
    //
    // ⊘⊘ **And a FOURTH: `JOINED-one-memory`.** `[measured 2026-08-12, boot `w278b_guest`]`
    // this row printed `nz4/4096 resN-NEVER-WRITTEN` about the SAME PAGE, in one line — the
    // four bytes being the guest's own GPFIFO entry, served correctly by a join whose
    // install had removed the local page the residency question reads. A row that
    // contradicts itself was read as the wall for a whole rung. `fb_page_standing` checks
    // the join first so the contradiction is unrepresentable.
    let res = plane.fb_page_standing(phys).tag();
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

/// ★★★★★ **w288 — HOW MANY BYTES OF THE GUEST'S NOTIFIER THE HOST DESCRIPTOR COVERS.**
///
/// One page. The record itself is 16 bytes (`kayfabe_abi::notifier::NOTIFICATION_SIZE`) and
/// the guest declares no size that reaches this seam —
/// [`kayfabe_rt::ErrorNotifier::Sysmem`] carries an address and nothing else — so a
/// page is the smallest thing that can be *mapped* rather than the smallest thing that is
/// *written*. ⊘ Not a guess about the guest's allocation: an `mmap` of the guest-RAM `memfd`
/// is page-granular whatever we ask for, so anything smaller would be the same mapping with
/// a more confident-looking number on it.
const ERROR_NOTIFIER_GRANT_BYTES: u64 = 0x1000;

/// ★★★★★ **w288 — THE GUEST'S OWN ERROR-NOTIFIER PAGES, RESOLVED INTO A GRANT.**
///
/// The one place in this shim that turns *"the guest declared its notifier at this GPA"*
/// into *"here is the slice of the guest-RAM block the isolate may build an
/// `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` over"*. It is here, and can only be here, because
/// **only the VMM may derive a grant** (`kayfabe_isolate::GuestRamGrant::originated_by_the_vmm`)
/// — the file offset comes from the hypervisor's own stated layout and from nowhere else.
///
/// ⊘ **Every refusal is BY NAME and every one of them yields `None`**, which is a channel
/// born with no notifier — the pre-w288 behaviour exactly, and never a clamp, never a
/// nearby page, never an assumed identity mapping.
///
/// ⊘ **Silent when the guest-RAM crossing is not armed** (`backing == None`), for the reason
/// every other pin pass in this file is silent on its disarmed arm: a control's log must not
/// contain a line the armed run's does not, or the two stop being comparable.
///
/// # ⚠ There is deliberately NO host-side read-back of what this builds
///
/// `[measured, R31 arm B]` a guest-backed `OS_DESCRIPTOR` **cannot be CPU-mapped**
/// (`NV_ERR_NOT_SUPPORTED`, *"memMap_IMPL: CPU mapping not supported for addressSpace:
/// 0x1"*). A read here would fail, and a caller that swallowed the failure would measure
/// nothing while looking like verification. The reader of these bytes is the **guest**.
fn err_notifier_grant(
    ce: &CeShellState,
    backing: Option<kayfabe_vmm_qemu::layout::BackingId>,
    notifier: Option<kayfabe_rt::ErrorNotifier>,
    who: &str,
) -> Option<kayfabe_isolate::GuestRamGrant> {
    let backing = backing?;
    let head = format!(
        "kayfabe: ERROR-NOTIFIER {who} dev={} ino={}",
        backing.dev, backing.ino
    );
    // ★★★ THE GUEST'S OWN DECLARATION IS THE GATE, and its three states are three lines.
    // ⊘ `Unreachable` and `None` are NOT folded: the first is a gap in **us** (the guest
    // asked to be told and named somewhere this port has no write port for) and the second
    // is the guest waiving error reporting. They send a reader to different files.
    let gpa = match notifier {
        Some(kayfabe_rt::ErrorNotifier::Sysmem { gpa }) => gpa,
        Some(kayfabe_rt::ErrorNotifier::Unreachable) => {
            eprintln!(
                "{head} → ⊘ ERROR-NOTIFIER REFUSED NotifierUnreachable (the channel DID declare \
                 a notifier and it is not in guest RAM, so no OS_DESCRIPTOR can be built over \
                 it. ⚠ This is a gap in US, not a guest that waived error reporting)"
            );
            return None;
        }
        None => {
            eprintln!(
                "{head} → ⊘ NONE DECLARED (this channel named no error notifier at all; the \
                 host channel is born with hObjectError = 0, which is every pre-w288 boot)"
            );
            return None;
        }
    };
    // ⚠⚠ **PAGE ALIGNMENT IS A REFUSAL, NOT A ROUNDING.** The grant becomes an `mmap` of the
    // guest-RAM `memfd` at this offset, and `mmap` requires a page-aligned offset — so an
    // unaligned GPA cannot be honoured. ⊘ Aligning DOWN would be worse than refusing: the
    // descriptor's base is what RM hands the GSP as `errorNotifierMem.base`
    // (`ogkm-580: kernel_channel.c:549-568`), so a page-aligned base would put the GSP's
    // 16-byte write at the top of the page instead of at the address the guest declared —
    // a write that lands, reports success, and is read by nobody.
    if !gpa.is_multiple_of(ERROR_NOTIFIER_GRANT_BYTES) {
        eprintln!(
            "{head} gpa=0x{gpa:x} → ⊘ ERROR-NOTIFIER REFUSED NotifierGpaMisaligned (a grant is \
             an mmap offset and must be a multiple of 0x{:x}; aligning down would move the \
             GSP's write off the address the guest declared)",
            ERROR_NOTIFIER_GRANT_BYTES,
        );
        return None;
    }
    // ⊘ The plane lock is held across the layout read and NOTHING else — no host verb, no
    // print — for `pin_completion_guest_ram`'s stated reason.
    let resolved = {
        let held = ce.vmm.lock().unwrap_or_else(|e| e.into_inner());
        held.as_ref()
            .map(|vmm| vmm.resolve_guest_ram(backing, gpa, ERROR_NOTIFIER_GRANT_BYTES))
    };
    match resolved {
        None => {
            eprintln!(
                "{head} gpa=0x{gpa:x} → ⊘ ERROR-NOTIFIER REFUSED NoMemoryPlane (between \
                 `realize` and `attach_ram` there is no stated layout to resolve against)"
            );
            None
        }
        Some(Err(e)) => {
            eprintln!(
                "{head} gpa=0x{gpa:x} len=0x{:x} → ⊘ ERROR-NOTIFIER REFUSED {} ⊘ {e:?} (the \
                 hypervisor's own stated layout does not describe this range; nothing is \
                 clamped and no nearby page is substituted)",
                ERROR_NOTIFIER_GRANT_BYTES,
                e.name(),
            );
            None
        }
        Some(Ok(run)) => {
            eprintln!(
                "{head} gpa=0x{gpa:x} file_offset=0x{:x} len=0x{:x} → GRANTED ReadWrite (the GSP \
                 WRITES these pages: ogkm-580 kernel_channel.c:549-568 sends \
                 errorNotifierMem.base = memdescGetPhysAddr(..), and for a descriptor over guest \
                 pages that base IS the guest's page)",
                run.file_offset, run.len,
            );
            Some(kayfabe_isolate::GuestRamGrant::originated_by_the_vmm(
                run.file_offset,
                run.len,
                // ★★★ **ReadWrite, and it is the GSP that needs it.** A read-only grant would
                // map the pages read-only IN THE ISOLATE, and the `OS_DESCRIPTOR` built over
                // that mapping is what RM pins for its own writer.
                kayfabe_vmm::Prot::ReadWrite,
            ))
        }
    }
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
///
/// ★★★★★ **w288 — `err_notifier_grants` is the VMM's half**, derived by
/// [`Regs::pending_err_notifier_grants`] before this call and applied by key inside the
/// drain. ⊘ Passed through untouched: this function reports, it does not decide.
fn report_engine_forward_drain(
    device: &kayfabe_rt::device::SharedDevice,
    err_notifier_grants: &[kayfabe_rt::device::EngineNotifierGrant],
) {
    let t0 = Instant::now();
    let runs = device.run_pending_engine_forwards(err_notifier_grants);
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

    /// ★★★★★ w303 — the shell's seat for the `0xa06c0105` host-reachability census. ⚠ It
    /// exists for the same measured reason the arm below it does: `as_gpu` is `None` here
    /// by design, so an arm written against it would refuse on every real boot.
    fn group_host_twins(
        &self,
        client: kayfabe_rt::HClient,
        object: kayfabe_rt::HObject,
    ) -> Result<kayfabe_core::gpu::GroupHostTwins, kayfabe_core::gpu::ScheduleGroupFault> {
        self.0.group_host_twins(client, object)
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

    /// ★★★★★ **w288 TIER 2 — the shell's seat for the one-to-one channel-control relay.**
    ///
    /// ⊘ A pure delegation, and it must stay one: every decision — the route, the host twin,
    /// the ack-only refusal — belongs to `SharedDevice::relay_channel_control`, which is the
    /// only party holding the locks that make them consistent. A shell that added a check
    /// here would be a second authority on a fact the device already decided.
    fn relay_channel_control(
        &mut self,
        client: kayfabe_rt::HClient,
        object: kayfabe_rt::HObject,
        cmd: kayfabe_rt::ControlCmd,
        payload: &mut [u8],
    ) -> Result<kayfabe_rt::ChannelControlRelay, kayfabe_rt::ChannelControlRelayFault> {
        self.0.relay_channel_control(client, object, cmd, payload)
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
    /// ★★★★★ **w282's arm** — whether a CE operand page that lands in the emulated
    /// framebuffer has its leaf JOINED, so the executor stays `HostCe`. See
    /// [`OPERAND_JOIN_ENV`] and [`SharedDoorbell::join_operand_fb_leaves`].
    ///
    /// ★ Read ONCE at the composition root and carried, for `gr_route`'s reason exactly, and
    /// its own **sixth** selector rather than a rider on [`GUEST_OPERAND_ENV`] — the pin and
    /// the join serve **disjoint** operand populations (guest RAM vs framebuffer) and a boot
    /// must be able to arm either alone.
    #[cfg_attr(not(feature = "host-isolates"), allow(dead_code))]
    operand_join: OperandJoinArm,
    /// ★★★★★ w290 — which arm of the whole-VAS publication this boot runs. See
    /// [`VAS_PUBLISH_ENV`] and [`SharedDoorbell::publish_vas_rows`].
    ///
    /// ★ Its own **seventh** selector, read ONCE at the composition root and carried, for
    /// `operand_join`'s reason exactly: leg 7 serves the CE operands a pushbuffer names and
    /// this serves every row the guest's page tables declare, so a boot must be able to arm
    /// either alone and a log must say which it had.
    #[cfg_attr(not(feature = "host-isolates"), allow(dead_code))]
    vas_publish: VasPublishArm,
    /// ★★★★★ **w318 — THE DIRTY GATE'S STATE.** See [`DirtyGate`] for the whole argument.
    dirty: DirtyGate,
}

/// ★★★★★ **w318 — THE DIRTY GATE: what the last doorbell already did, so this one need not
/// do it again.**
///
/// # ★ THE MEASUREMENT THIS EXISTS FOR
///
/// `[measured 2026-08-14, w315, real GA106, two boots, shares reproducing to ~1.6 points]`
/// the guest's `cuLaunchKernel` does not return until this crate's MMIO doorbell handler
/// does, and the handler is **86.7 ms of a 90.9 ms submit**. Of that trap:
///
/// | segment | ms/launch | share |
/// |---|---|---|
/// | `vas_publish` | 48.3 | **55.7 %** |
/// | `pt_decode` | 22.3 | **25.7 %** |
/// | `pt_sweep` + `pt_vascensus` | 8.7 | 10.1 % |
/// | the real host RM forward | 3.5 | 4.1 % |
///
/// ⇒ **91.5 % is page-table + publication work, and it publishes nothing**: the two
/// consecutive launch doorbells in that trace print *byte-identical* `PT-DECODE` lines
/// (`drained=162 latched=52 rounds=1 → bound=0 … published=0/0 refusals=1592 straddles=255`)
/// and *byte-identical* publication censuses (`published=0 refused=8 in 43 ms`, the eight
/// refusals all `that framebuffer range is already joined`). The handler re-derives the same
/// answer ~12 times a launch loop and acts on it zero times.
///
/// # ★★★ THE C ARTIFACT GATES EXACTLY THIS, and its shape is the precedent
///
/// `C: src/qemu/nvkvm_gpu_emul.c:580-583` — *"`m2_gr_vas_dirty` → the next doorbell sweeps
/// and rebuilds the set; **otherwise it skips**"*; `:1399-1400` — *"Once dirty, skip until
/// the next sweep consumes it."*; `:284` — a second gate on the walk itself.
///
/// ⊘ **Taken as a precedent that the gate is SOUND, not as a design to transcribe.** The C's
/// gate is armed by *a tracked page-table page having been written*. Ours is armed by
/// **the thing the skipped pass actually reads**, which is stricter and is what makes the
/// skip provable rather than plausible:
///
/// - `vas_publish` reads a `Vas`'s rows and its guest-RAM pins ⇒ armed by
///   [`kayfabe_core::gpu::Vas::publish_epoch`], plus a host term (`joined`) for the state the
///   *verb's outcome* depends on that our record cannot see.
/// - the executor page-table witness re-queues every executor-created framebuffer page ⇒
///   armed by the store's **executor write count** ([`kayfabe_device::FbStore::writes_by`]),
///   because re-decoding pages whose bytes did not change cannot produce a different bind.
///
/// # ⚠⚠ CORRECTNESS DOMINATES, and here is exactly what is being relied on
///
/// `VAS_PUBLISH` ablated **red**: it is one of the relaxations that is not inert, and a
/// publication skipped that the engine then needs is **a GPU fault**, not a slow path. The
/// skip is sound only because the skipped work is a **pure function of state this gate
/// observes**. Three consequences are enforced rather than argued:
///
/// 1. **`None` is UNMEASURED, never clean.** Every source that cannot answer (no plane, a
///    store that does not count, a `Vas` that is not there) **arms**. There is no default-skip
///    anywhere in this type.
/// 2. **An INCOMPLETE pass never marks clean.** A publication that hit its wall budget left
///    candidates unattempted; recording that state as clean would strand them forever. The
///    stamp is taken only on a pass that ran to the end.
/// 3. **Both gates are separately armable and both are ON only when their env says so**, so
///    a boot can ablate either alone and the log always says which arm it ran — the same
///    discipline `KAYFABE_PT_SWEEP` and `KAYFABE_VAS_PUBLISH` already carry.
///
/// ⊘ And the gate **counts its own fires and skips**. A gate that fires on every doorbell
/// and a gate that is working produce the same `trap_ms` if nothing else changed, and only
/// the ratio distinguishes them (w318 pre-registered outcome (B)).
#[derive(Debug, Default)]
struct DirtyGate {
    /// Per-`(proc, gpu, pdb)`: what the last **completed** publication pass saw. Absent = this
    /// key has never been published, which arms.
    ///
    /// ⊘ Only the `host-isolates` build has a publication pass to gate; the *witness* gate
    /// beside it is compiled in every build, which is why the two live in one type and only
    /// this field carries the attribute.
    #[cfg_attr(not(feature = "host-isolates"), allow(dead_code))]
    published: std::sync::Mutex<
        std::collections::HashMap<
            (kayfabe_core::ProcId, kayfabe_rt::GpuId, kayfabe_rt::Pdb),
            PublishStamp,
        >,
    >,
    /// The store's executor write count at the last executor-witness pass. `None` = never
    /// taken, which arms.
    exec_writes: std::sync::Mutex<Option<u64>>,
    /// `(fired, skipped)` for the publication gate and the witness gate, in that order.
    /// ⊘ Printed on every doorbell: see the type docs for why the ratio is the diagnostic.
    counts: std::sync::Mutex<[(u64, u64); 2]>,
}

/// What a **completed** publication pass over one VAS observed. See [`DirtyGate`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct PublishStamp {
    /// [`kayfabe_core::gpu::Vas::publish_epoch`] — our own record of the VAS.
    epoch: (u64, usize),
    /// ★★ **THE HOST TERM.** How many framebuffer ranges the store had joined. The eight
    /// refusals this gate skips are all *"that framebuffer range is already joined"* — an
    /// outcome that depends on host state, not on the table — so a change here must re-arm
    /// even when our own rows are untouched. ⊘ A count, not a set: it moves on every install
    /// and on every release, which is all the gate needs, and building the set per doorbell
    /// would put back a slice of the cost being removed.
    joined: usize,
    /// The census line that pass produced, replayed verbatim on a skip so a boot's log stays
    /// readable and diffable against an ungated one. ⊘ Marked as a replay where it is
    /// printed — a cached line presented as fresh is a second source of truth beside a
    /// complete value.
    line: String,
}

impl DirtyGate {
    /// Index into [`DirtyGate::counts`] for the publication gate.
    const PUBLISH: usize = 0;
    /// Index into [`DirtyGate::counts`] for the executor-witness gate.
    const WITNESS: usize = 1;

    fn tally(&self, which: usize, fired: bool) {
        let mut c = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        if fired {
            c[which].0 = c[which].0.saturating_add(1);
        } else {
            c[which].1 = c[which].1.saturating_add(1);
        }
    }

    /// ★★★ The fire/skip ratio, both gates, as one line. **This is w318's own diagnostic**:
    /// outcome (B) — *"the gate fires and the trap does not drop"* — is only distinguishable
    /// from a gate that is working by these four numbers.
    fn census(&self) -> String {
        let c = *self.counts.lock().unwrap_or_else(|e| e.into_inner());
        let pct = |f: u64, s: u64| {
            let t = f + s;
            if t == 0 {
                "n/a — UNMEASURED, this gate was never consulted".to_string()
            } else {
                format!("{:.1}% skipped", 100.0 * s as f64 / t as f64)
            }
        };
        format!(
            "DIRTY-GATE publish[fired={} skipped={} {}] witness[fired={} skipped={} {}]",
            c[Self::PUBLISH].0,
            c[Self::PUBLISH].1,
            pct(c[Self::PUBLISH].0, c[Self::PUBLISH].1),
            c[Self::WITNESS].0,
            c[Self::WITNESS].1,
            pct(c[Self::WITNESS].0, c[Self::WITNESS].1),
        )
    }
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
    /// ★★★★★ **THE GR CHANNELS WHOSE `GP_GET`/`GP_PUT` THE OBSERVER SAMPLES.** See
    /// [`GrCursorWatch`] and [`GrCursorReader`].
    ///
    /// Written by the vCPU thread (one push per GR channel, at the first declare) and read
    /// by the observer thread on every tick. ⊘ A leaf mutex holding a `Vec` of plain `Copy`
    /// facts: the reader **clones it and drops the guard** before touching the plane or
    /// printing, so nothing blocks beneath it — `gr_dumps`' shape, and the one
    /// `unranked_locks.rs` classifies as safe.
    ///
    /// ⊘⊘ **CORRECTED `[w300, 2026-08-13]`: that last clause was FALSE WHEN WRITTEN.**
    /// `unranked_locks.rs` classified no such row — its scanner could not see a lock spelled
    /// `Arc<Mutex<…>>` (the field test wanted a `:` before the type and found the `<` of the
    /// `Arc`), so this field, and eight others, were never put to anyone for a ruling while
    /// the gate reported **zero unclassified**. The row exists now, and the scanner is pinned
    /// by known-positive fixtures. ★ The lesson is not that the ruling was wrong — it is
    /// right, and `[src]` re-read off the three call sites its row now cites (`:3766` clones
    /// and drops, `:5252` pushes, `:5263` drops before printing) — but that
    /// **a doc citing a gate is not the gate having run**:
    /// this comment asserted a classification into a list that had no such entry, and nothing
    /// checked the assertion because the gate was blind to the very field making it.
    gr_cursors: std::sync::Arc<std::sync::Mutex<Vec<GrCursorWatch>>>,
    /// ★★★★★ The observer's reactor thread, once started. See [`Regs::attach_ram`].
    #[cfg(feature = "host-isolates")]
    observer: std::sync::Mutex<Option<ObserverThread>>,
}

/// ★★★★★ **ONE GR CHANNEL'S CURSOR, LATCHED SO A LATE READER CAN FIND IT** —
/// the owner's `GP_GET` diagnostic, 2026-08-12.
///
/// # ★★★ Why a latch and not a read at the doorbell
///
/// The owner's question is *"if the GPU even tried running, `GP_GET` should advance"*, and it
/// is the right question: it splits the GR wall three ways where every instrument this
/// campaign has run splits it two ways. But `[measured, w267_on]` **each `GrCompute` channel
/// is rung exactly once** (`DOORBELL-REFUSED #5…#12`, one per token), so a cursor read taken
/// on the doorbell path lands **microseconds** after the guest wrote `GP_PUT` — before any
/// engine could have fetched, on either arm. ⇒ A doorbell-time `GET = 0` is not evidence the
/// engine never fetched; it is evidence nobody waited.
///
/// So the doorbell **latches the identity** and the completion observer — which already ticks
/// every 250 ms for the whole of `cup2`'s wall — does the **reading**.
///
/// # ⊘ No new capability, and that was checked before it was written
///
/// [`fb_userd_cursors`] already reads `USERD_GP_GET`/`USERD_GP_PUT` out of the framebuffer
/// store, already checks the **join** before residency (the correction `w266` paid for), and
/// already takes a `DeclaredUserd` rather than an engine. `[verified 2026-08-12]` it has no
/// `GrCompute` caller anywhere: it is reached only through `addressing_probe_facts`, which
/// runs on the forwarding fall-through and the three CE refusal sites, and
/// `grep -c "GET=" run_w267_on_qemu.log` is **9** — eight `RING-PROJ` lines, every one `Ce`,
/// plus the first-refusal summary. ⇒ This rung is a **call site**, not a capability.
#[derive(Debug, Clone, Copy)]
struct GrCursorWatch {
    /// The guest token this channel's doorbell carries — the join key to every other line.
    ///
    /// ⊘ `allow(dead_code)` rather than deletion: the ONE reader is the `GR-CURSOR` line in
    /// the `host-isolates` arm, which a default-feature clippy run does not compile. Removing
    /// the field to satisfy that run would delete the join key from the log the arm exists to
    /// print.
    #[allow(dead_code)]
    token: u64,
    proc: u32,
    chan: u32,
    /// The channel's engine, carried so a reader never has to infer it from the token.
    engine: &'static str,
    /// The USERD the **guest's own kernel** declared for this channel
    /// (`NV_CHANNEL_ALLOC_PARAMS.userdMem`). ⊘ Not ours: after leg B the host channel is born
    /// over this same page (`GR-BIRTH … userd=GUEST-USERD`), which is precisely why reading it
    /// answers a question about the **host** engine's progress.
    userd: kayfabe_core::rmgraph::DeclaredUserd,
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
/// ★★★ **w326 — the capability to drive the revocation drain, as ONE value.**
///
/// ⊘ The device and the gate travel together on purpose: a caller holding the device without
/// the gate could drain concurrently with a vCPU and free the same retired object twice
/// (`crate::reclaimtick`'s hazard). Bundling them makes *"drive the drain"* the only thing
/// that can be handed over, rather than two things that must be remembered together.
#[cfg(feature = "host-isolates")]
struct ReclaimDriver {
    device: std::sync::Arc<kayfabe_rt::device::SharedDevice>,
    tick: std::sync::Arc<crate::reclaimtick::ReclaimTick>,
}

#[cfg(feature = "host-isolates")]
fn observer_loop(
    reactor: &mut kayfabe_shell::Reactor,
    watch: &std::sync::Arc<kayfabe_rt::completion_watch::WatchList>,
    stop: &std::sync::atomic::AtomicBool,
    mut vmm: kayfabe_vmm_qemu::QemuVmm,
    plane: &std::sync::Arc<RegPlane>,
    gr_cursors: &std::sync::Arc<std::sync::Mutex<Vec<GrCursorWatch>>>,
    // ★★★★★ **w326 — THE REVOCATION DRAIN'S DRIVER.** See `crate::reclaimtick` for why
    // this belongs on THIS thread and not on the publication lane: `Revocation` has no
    // route into `pubqueue` by construction, and it must not acquire one.
    //
    // ⊘ ONE parameter and not two, because two would push this signature to 8 arguments and
    //   clippy's `too_many_arguments` is right about it: the device and the gate are one
    //   capability — *"drive the revocation drain"* — and splitting them would let a future
    //   caller hand over the device WITHOUT the gate, which is the double-disposal bug.
    reclaim: &ReclaimDriver,
) {
    use kayfabe_vmm::Vmm as _;
    let mut pages = SemaPageReader::new();
    let mut cursors = GrCursorReader::new();
    while !stop.load(std::sync::atomic::Ordering::Acquire) {
        // ★★★★★ **w326 — THE REVOCATION TICK.** `w323`: *"the budgeted drain carries its
        // remainder to the NEXT REGISTER WRITE, and only the guest produces register
        // writes"* ⇒ a guest that frees and then stops trapping leaves a live host-GPU
        // translation into pages Linux has reused. **A bound discharged only by the
        // adversary is not a bound.** This is the other driver.
        //
        // ★ `OffTrap::claim` and not `at_a_host_verb`: this thread is genuinely off-trap,
        // so the honest mint is the one that PANICS if it ever is not. That is the census
        // row w323 wanted retired — a real claim rather than a counted exception.
        //
        // ⊘ Disarmed, `spend` returns without taking the gate or touching the queue, so the
        // control arm is byte-identical to master.
        let device = &reclaim.device;
        reclaim.tick.spend(|| {
            let off = kayfabe_util::trapwitness::OffTrap::claim("the revocation drain tick");
            off.still_off_trap("draining retired host objects");
            // ⊘ Same ORDER as `Regs::write`'s, and it is mandatory: the reap holds a proc
            // back while its staged queue is non-empty, so a reap before the drain would
            // defer every proc by one tick forever.
            //
            // ⊘⊘ **`pin_reclaim_gone()` IS NOT CALLED HERE, and the first spelling of this
            // tick called it.** It is a **read-only cumulative tally** (`state.read()`;
            // *"one live proc's CUMULATIVE guest-RAM pin reclaim tally"*), not an action —
            // so it disposes nothing, and adding its `released` to this tick's count sums a
            // running total once per tick. `[measured, boot `w326r1`]` that produced
            // `worker_disposed=2064292` over 113 working ticks — 18 268 per tick, i.e. the
            // whole boot's cumulative pin count, re-counted every 250 ms.
            // ★ It was caught because the number was IMPLAUSIBLE, not because anything went
            // red: an instrument that over-reports by 1000× still prints a healthy-looking
            // line. `suspect_the_instrument_first`.
            let t0 = std::time::Instant::now();
            let drain = device.drain_retired_budgeted(RETIRED_DRAIN_CHUNK, || {
                // ★ A budget here too — not for the BQL (we hold none) but so one tick
                // cannot monopolise the thread that also serves the completion watch.
                u64::try_from(t0.elapsed().as_micros()).unwrap_or(u64::MAX)
                    >= RETIRED_DRAIN_BUDGET_US
            });
            let (reaped, _deferred) = device.reap_retired_held();
            // ⊘ `drain.disposed` ONLY — a per-call count. See the note above.
            (
                u64::try_from(drain.disposed).unwrap_or(u64::MAX),
                u64::try_from(reaped).unwrap_or(u64::MAX),
            )
        });
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
        // ★★★★★ THE PAGE, not the word. ⊘ Deliberately AFTER the verdicts, so a dump that
        // shares a `why=verdict` with a `NOT-OBSERVED` line above it is a statement about the
        // same instant that verdict was decided at — the ordering IS the timestamp's meaning.
        pages.look(
            watch,
            &mut vmm,
            if verdicts.is_empty() {
                "tick"
            } else {
                "verdict"
            },
        );
        // ★★★★★ **THE OWNER'S THREE-WAY DISCRIMINATOR, SAMPLED LATE AND REPEATEDLY.**
        // See [`GrCursorWatch`]. ⊘ On the same tick as the page dump and beside it, so the
        // two halves of one question — *"did the engine fetch?"* and *"did it write?"* — are
        // never read from two different instants.
        cursors.look(plane, gr_cursors);
        match outcome {
            Ok(()) => {}
            // ★ The F1 refusal is LOUD and stops the loop rather than spinning. It cannot
            // fire vacuously: it means a ready token produced no work 16 waits running.
            Err(fault) => {
                eprintln!(
                    "kayfabe: COMPLETION-OBSERVER ⊘ REACTOR FAULT {fault:?} — the loop \
                     STOPPED. Every later COMPLETION-WATCH line is absent by construction."
                );
                // ⊘ One last look before giving up. A loop that stops on a reactor fault
                // still holds the only reader of these pages, and the state at the moment it
                // stopped is evidence nobody else can produce.
                pages.look(watch, &mut vmm, "reactor-fault");
                cursors.close("reactor-fault");
                pages.close();
                return;
            }
        }
    }
    // ★ The teardown dump. ⚠ It is the LAST state, not the state during the guest's poll —
    // `stop` is set from `detach_ram`, long after `cup2` has given up. The dumps that answer
    // *"what was in the page while the guest was spinning"* are the `why=tick` /
    // `why=verdict` ones above, and that is why every dump carries `t=+Nms`.
    pages.look(watch, &mut vmm, "final");
    cursors.close("final");
    pages.close();
}

/// ★★★★★ **THE LATE `GP_GET` SAMPLER** — the reading half of [`GrCursorWatch`], and the one
/// instrument in this tree that can answer the owner's three-way question.
///
/// | reading | meaning |
/// |---|---|
/// | `PUT > 0`, `GET == 0` | **the engine never fetched this ring.** Delivery or scheduling; the work never started and the zero semaphore is downstream of a cause upstream of it |
/// | `GET` caught `PUT` | **the work RAN.** A zero semaphore slot is then a *separate*, later failure and the diagnosis changes completely |
/// | `PUT == 0` | the guest never submitted on this channel — the question moves to the guest side entirely |
///
/// # ⊘ Three things it does not do
///
/// 1. ⊘ **It never writes.** It holds an `Arc<RegPlane>` and reads eight bytes through
///    `RegPlane::fb_peek`; there is no write path in this type.
/// 2. ⊘ **It resolves nothing.** The USERD address comes from the guest's own
///    `NV_CHANNEL_ALLOC_PARAMS.userdMem`, latched on the vCPU thread by
///    `SharedDoorbell::latch_gr_cursor`. A second resolution here would be two projections of
///    one fact.
/// 3. ⊘ **It reports STATE, never EVENTS.** A cursor that advanced and wrapped between two
///    samples is one change; a channel that fetched and finished inside one 250 ms tick shows
///    as a single row. The dump can say *"the cursor is here now"*; it can never say
///    *"nothing was fetched"*.
///
/// ⚠ **Prints on FIRST SIGHT and on CHANGE only.** Eight channels × 700 ticks is 5 600 lines
/// of an unchanging cursor, which would bury the one row that matters. The teardown line
/// carries the tick count, so *"it never changed"* and *"it never ran"* stay separable.
#[cfg(feature = "host-isolates")]
#[derive(Debug)]
struct GrCursorReader {
    started: std::time::Instant,
    ticks: u64,
    printed: u64,
    /// Per `(proc, chan)`: the last line printed, so only changes print.
    seen: std::collections::BTreeMap<(u32, u32), String>,
}

#[cfg(feature = "host-isolates")]
impl GrCursorReader {
    fn new() -> Self {
        Self {
            started: std::time::Instant::now(),
            ticks: 0,
            printed: 0,
            seen: std::collections::BTreeMap::new(),
        }
    }

    /// One look at every latched GR channel's cursor.
    fn look(
        &mut self,
        plane: &std::sync::Arc<RegPlane>,
        gr_cursors: &std::sync::Arc<std::sync::Mutex<Vec<GrCursorWatch>>>,
    ) {
        self.ticks += 1;
        // ⊘ CLONED under the lock and read outside it. The vCPU thread pushes into this Vec
        // from the doorbell path, and holding its guard across a plane read — which takes the
        // plane's own mutex — would nest two unranked locks on two threads in two orders.
        let watches: Vec<GrCursorWatch> = {
            let held = gr_cursors.lock().unwrap_or_else(|e| e.into_inner());
            held.clone()
        };
        for w in &watches {
            let cur = fb_userd_cursors(plane, Some(w.userd));
            let cur = if cur.is_empty() {
                // ⊘ `fb_userd_cursors` answers the empty string when the USERD has no
                // framebuffer address. That is a legal case and it is NOT `GET = 0`; it gets
                // its own words so a reader can never fold the two together.
                " ⊘ NO FRAMEBUFFER USERD — this channel's USERD is not in a store this reader \
                 can serve. NOTHING WAS READ; this is not `GET = 0`"
                    .to_string()
            } else {
                cur
            };
            let key = (w.proc, w.chan);
            if self.seen.get(&key) == Some(&cur) {
                continue;
            }
            let first = !self.seen.contains_key(&key);
            self.seen.insert(key, cur.clone());
            self.printed += 1;
            let t = self.started.elapsed().as_millis();
            eprintln!(
                "kayfabe: GR-CURSOR token={:#010x} proc={} chan={} engine={} why={} t=+{t}ms \
                 tick={}{cur}",
                w.token,
                w.proc,
                w.chan,
                w.engine,
                if first { "first" } else { "CHANGED" },
                self.ticks,
            );
        }
    }

    /// The tally, printed unconditionally when the loop exits. ⊘ Its ABSENCE is the only way
    /// to tell *"the reader never got here"* from *"the reader saw nothing move"* — the same
    /// property `SemaPageReader::close` exists for, and the one `w267` §3.2's own assertion
    /// caught missing.
    fn close(&self, why: &str) {
        eprintln!(
            "kayfabe: GR-CURSOR-READER stopped why={why} ticks={} channels={} rows_printed={} \
             elapsed={}ms ⊘ a row prints on FIRST SIGHT and on CHANGE only, so \
             rows_printed == channels means NOT ONE CURSOR EVER MOVED",
            self.ticks,
            self.seen.len(),
            self.printed,
            self.started.elapsed().as_millis(),
        );
    }
}

/// How many bytes one dumped page is. ⊘ 4 KiB because that is the unit
/// `SharedDoorbell::pin_completion_guest_ram` pins (`RING_PIN_BYTES`), so the dump's extent and
/// the pin's extent are the same fact and cannot drift apart.
#[cfg(feature = "host-isolates")]
const SEMA_PAGE_BYTES: usize = 4096;

/// How many non-zero slots one dump LISTS. ⊘ The TOTAL is always exact and printed first; only
/// the enumeration is bounded, and a bounded enumeration says so by name.
///
/// ⚠ Spelled `LISTING-BOUND` in the output and **not** `CAPPED`, deliberately: `CAPPED` is a
/// live regex in `w266_grade.sh` (`R10b`) scoped to leg 4's pin, and `w266`'s own most
/// expensive instrument lesson was a new producer silently re-scoping three consumers that
/// were implicitly scoped by being the only one.
#[cfg(feature = "host-isolates")]
const SEMA_PAGE_SLOTS_LISTED: usize = 192;

/// How many dumps are PRINTED before the reader falls silent. Suppressed dumps are counted
/// exactly and the count is printed at `close`, so silence is never mistaken for stillness.
#[cfg(feature = "host-isolates")]
const SEMA_PAGE_DUMPS_MAX: u64 = 128;

/// Ticks between heartbeat dumps of an UNCHANGED page — 40 × 250 ms = 10 s.
///
/// ⊘ A heartbeat exists because *"the content did not change"* and *"the reader stopped
/// running"* produce the same log otherwise, and only one of them is about the guest.
#[cfg(feature = "host-isolates")]
const SEMA_PAGE_HEARTBEAT_TICKS: u64 = 40;

/// ★★★★★ **THE 4 KiB READER — the rung `w266` could not take, and the capability it needed
/// already existed.**
///
/// # What this answers
///
/// `[measured, w266, real GA106, both arms]` pinning the completion page took the host GPU's
/// eight `Xid 31 … @ 0x2_0440f000 ACCESS_TYPE_VIRT_WRITE` to **zero**, while
/// `COMPLETION-WATCH` stayed `NOT-OBSERVED` on all eight declared addresses. *"No fault"* is
/// consistent with **(a)** a write that landed at a slot nobody watches and **(b)** a write
/// that was never attempted, and `w266` could not separate them **because nothing in the tree
/// read more than four bytes of guest RAM**.
///
/// ⊘ **No new capability.** `Vmm::gpa_read` has always taken a `&mut [u8]` of any length and
/// this thread has always held a `QemuVmm`; `WatchList::declared_sites` has handed out every
/// declared address since leg 5. `[verified 2026-08-12]` the observer's `&mut [u8; 4]` closure
/// was the **only** production `gpa_read` call site in the tree. What was missing was a
/// consumer, and this is it.
///
/// # ⊘ Three things it deliberately does not do
///
/// 1. ⊘ **It never writes.** It holds a `&mut QemuVmm` and therefore *could*, which is exactly
///    why this sentence is here: the payload is a literal immediate in the guest's own bytes,
///    and a VMM that writes it fakes the completion without running the work — the C
///    artifact's named dead end. There is no `gpa_write` in this type; grep it.
/// 2. ⊘ **It resolves nothing.** Every address comes from `declared_sites`, resolved once by
///    the declaring thread under the locks it already held. A second resolver here would be
///    two projections of one fact, this campaign's most expensive failure class.
/// 3. ⊘ **It reports STATE, never EVENTS.** A write of the same bytes is invisible to a
///    content signature, and two writes between two samples are one change. The dump can say
///    *"these bytes are here now"*; it can never say *"nothing was written"*.
#[cfg(feature = "host-isolates")]
#[derive(Debug)]
struct SemaPageReader {
    started: std::time::Instant,
    seq: u64,
    ticks: u64,
    printed: u64,
    suppressed: u64,
    /// Per page-gpa: the last content signature printed. ★ Keyed by the page's guest-physical
    /// address, because that is the identity the pin used and the identity the reader reads.
    seen: std::collections::BTreeMap<u64, u64>,
    /// ★★★★★ **THE WRITE CENSUS** — per `(page, word index)`, the last value this reader saw.
    ///
    /// # ⊘ Why a signature was not enough, and why this is a different question
    ///
    /// [`Self::seen`] answers *"did the page change since I last PRINTED"* and is a print gate.
    /// It cannot answer the question arm 2.2 asks, because a page that is **frozen after
    /// corruption** and a page that is **frozen after nothing** produce the identical steady
    /// signature — and a sample taken 70 s in sees only the steady state. Distinguishing them
    /// needs the **history**: every transition, in order, with its time.
    ///
    /// ★★★ And one transition kind is not merely interesting, it is the C's measured disease.
    /// `how_the_c_passed_the_gr_wall.md` §1: a lagging host CE2 executed stale GPFIFO entries
    /// ~40 s late and DMA-wrote `1,2` **over the live value `0x1e`**; UVM's 32→64-bit wrap
    /// detector read the decrease as a wrap, reconstructed `completed > queued`, and wedged the
    /// channel. **The fix was to delete the second writer.** ⇒ a *decrease* on any of these
    /// words is the signature of a second writer, and this map exists to name it the instant it
    /// happens rather than to be inferred from a still frame afterwards.
    words: std::collections::BTreeMap<(u64, usize), u32>,
    /// Transitions observed, ever — so *"nothing was written"* is a **count**, not an absence.
    transitions: u64,
    /// Transitions in which a **payload** word went DOWN. ★★★ Non-zero here is the M5.38 shape.
    backwards: u64,
    /// ★★ Decreases on a word that is **not** a declared payload — a timestamp low word
    /// carrying, or a slot no declaration names. Counted **apart** from [`Self::backwards`]
    /// because they are a different claim: see the predicate's comment for the measurement
    /// that forced the split. ⊘ Reported, never folded in — a decrease we cannot attribute is
    /// neither evidence of corruption nor evidence of its absence.
    decreases_elsewhere: u64,
}

#[cfg(feature = "host-isolates")]
impl SemaPageReader {
    fn new() -> Self {
        Self {
            started: std::time::Instant::now(),
            seq: 0,
            ticks: 0,
            printed: 0,
            suppressed: 0,
            seen: std::collections::BTreeMap::new(),
            words: std::collections::BTreeMap::new(),
            transitions: 0,
            backwards: 0,
            decreases_elsewhere: 0,
        }
    }

    /// FNV-1a over the page. ⊘ A signature only — it decides whether to PRINT, and never
    /// whether a slot is interesting. Every printed dump enumerates the bytes themselves.
    fn signature(buf: &[u8]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in buf {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x1000_0000_01b3);
        }
        h
    }

    /// One look at every 4 KiB page holding a declared completion.
    fn look(
        &mut self,
        watch: &kayfabe_rt::completion_watch::WatchList,
        vmm: &mut kayfabe_vmm_qemu::QemuVmm,
        why: &str,
    ) {
        use kayfabe_vmm::Vmm as _;
        self.ticks += 1;
        let declared = watch.declared_sites();
        // ⊘ Page → the declarations inside it, so a slot can be attributed to the channel that
        // named it rather than reported as a bare number. Attribution is the whole question:
        // *"whose target is this offset"* is what separates the copy engine's own
        // `SET_SEMAPHORE` from the compute class's `SET_REPORT_SEMAPHORE`.
        let mut by_page: std::collections::BTreeMap<u64, Vec<(u64, u64, u32, u32)>> =
            std::collections::BTreeMap::new();
        for (key, site) in &declared {
            if let kayfabe_rt::completion_watch::Site::GuestRam { gpa } = site {
                let page = gpa & !(SEMA_PAGE_BYTES as u64 - 1);
                by_page.entry(page).or_default().push((
                    gpa & (SEMA_PAGE_BYTES as u64 - 1),
                    key.va,
                    key.proc.0,
                    key.chan.0,
                ));
            }
        }
        for (page_gpa, mut here) in by_page {
            here.sort_unstable();
            let mut buf = vec![0u8; SEMA_PAGE_BYTES];
            let read = vmm.gpa_read(page_gpa, &mut buf);
            self.seq += 1;
            let t = self.started.elapsed().as_millis();
            let head = format!(
                "kayfabe: SEMA-PAGE seq={} why={why} t=+{t}ms gpa=0x{page_gpa:x} \
                 len={SEMA_PAGE_BYTES} declares={}",
                self.seq,
                here.len(),
            );
            let Ok(()) = read else {
                // ⚠ A refusal is about the INSTRUMENT and gets its own word. It is never
                // folded into "the page is empty" — *"we could not look"* and *"we looked and
                // there was nothing"* are the two answers this whole rung exists to separate,
                // and an empty artefact reading as benign is the trap that named itself.
                eprintln!(
                    "{head} → READ-REFUSED ({:?}) ⚠ NOTHING WAS READ. ⊘ This row says nothing \
                     about the completion plane; it is a statement about this reader.",
                    read.unwrap_err()
                );
                self.printed += 1;
                continue;
            };
            let words: Vec<u32> = buf
                .chunks_exact(4)
                .map(|w| u32::from_le_bytes(w.try_into().unwrap_or([0; 4])))
                .collect();
            let nonzero: Vec<usize> = words
                .iter()
                .enumerate()
                .filter(|(_, w)| **w != 0)
                .map(|(i, _)| i)
                .collect();
            // ★★★★★ **THE LIVE WRITE CENSUS — RUN ON EVERY SAMPLE, PRINTED ONLY ON CHANGE.**
            //
            // ⊘ It is deliberately ABOVE the print gate. The gate suppresses ticks, and a
            // census that only ran on printed ticks would miss exactly the transient this
            // exists to catch: a value written and overwritten between two prints is the
            // *second writer*, and it would leave no trace at all.
            //
            // ⚠ It is a SAMPLER, not a watchpoint. It sees a word's value at tick
            // boundaries, so two writes inside one tick read as one transition, and a write
            // that is undone within a tick is invisible. ⇒ `transitions=0` bounds *"no
            // persistent change at this cadence"*, never *"nobody wrote"*. (A DMA write is
            // invisible to x86 debug registers, so a watchpoint is not the stronger
            // instrument it looks like — it is a negative control only.)
            for (i, &w) in words.iter().enumerate() {
                let prev = self.words.insert((page_gpa, i), w);
                let Some(p) = prev else { continue };
                if p == w {
                    continue;
                }
                self.transitions += 1;
                // ★★★★★ **A DECREASE IS ONLY THE M5.38 SIGNATURE ON A *PAYLOAD* WORD.**
                //
                // ⊘⊘ `[measured, w276b_on]` this predicate was `w < p` and it FIRED — on
                // `+0xf78`, `0xff109e00 → 0x1dc832e0`, while `+0xf7c` went
                // `0x18cb1a69 → 0x18cb1a6a` in the SAME sample. That is a 64-bit GPU
                // timestamp **carrying**: the low word wraps and the high word increments.
                // Not a second writer — the *same* writer, one nanosecond later.
                //
                // ⇒ The un-scoped predicate turns the normal behaviour of a `FOUR_WORDS`
                // report into this campaign's most alarming signature, roughly once per
                // 2^32 clock ticks. An instrument that cries the loudest word it has, on a
                // schedule, is worse than one that stays quiet: the next real decrease would
                // be read as another wrap. `[a-falsifier-that-flags-its-own-good-news]`
                //
                // ★ The fix is the attribution this reader ALREADY computes: `whose()`
                // distinguishes the declared payload slot from `+8`/`+12`, which are the
                // timestamp the engine writes. Only the payload is monotonic by contract.
                // ⊘ An `[UNCLAIMED]` word is NOT counted either — we cannot say which role it
                // plays, and *"we do not know"* must not be spelled *"corruption"*. It is
                // still PRINTED, with its own tag, so nothing is hidden.
                let tag = Self::whose((i * 4) as u64, &here);
                let is_payload = tag.starts_with("[GR-REPORT p");
                let down = w < p && is_payload;
                if down {
                    self.backwards += 1;
                } else if w < p {
                    self.decreases_elsewhere += 1;
                }
                // Printed unconditionally, and every one of them: a transition is rare by
                // construction (the steady state is what the heartbeat covers) and the ORDER
                // of these lines is the artefact. ⊘ No cap — a cap here would silently drop
                // the tail of exactly the sequence a corruption story is told in.
                eprintln!(
                    "    SEMA-WRITE t=+{t}ms gpa=0x{page_gpa:x}+0x{:03x} 0x{p:08x} → \
                     0x{w:08x}{tag}{} n={} back={} other_dec={}",
                    i * 4,
                    if down {
                        " ⚠⚠ BACKWARDS ON A PAYLOAD — the M5.38 second-writer signature: UVM \
                         reads any decrease as a 2^32 wrap and wedges the channel"
                    } else if w < p {
                        " ⊘ decrease on a NON-payload word (a timestamp low word carrying, or \
                         an unattributed slot) — NOT the M5.38 signature; see the neighbouring \
                         +4 word for the carry"
                    } else {
                        ""
                    },
                    self.transitions,
                    self.backwards,
                    self.decreases_elsewhere,
                );
            }
            let sig = Self::signature(&buf);
            let first = !self.seen.contains_key(&page_gpa);
            let changed = self.seen.get(&page_gpa) != Some(&sig);
            self.seen.insert(page_gpa, sig);
            // ★ PRINT on: first sight, any content change, any non-tick reason, or the
            // heartbeat. ⊘ The heartbeat is not decoration — without it, "unchanged" and "the
            // reader died" are the same log.
            let due = first
                || changed
                || why != "tick"
                || self.ticks.is_multiple_of(SEMA_PAGE_HEARTBEAT_TICKS);
            if !due || (self.printed >= SEMA_PAGE_DUMPS_MAX && why != "final") {
                if due {
                    self.suppressed += 1;
                }
                continue;
            }
            self.printed += 1;
            eprintln!(
                "{head} nonzero={}/{} sig=0x{sig:016x} first={} changed={}",
                nonzero.len(),
                words.len(),
                u8::from(first),
                u8::from(changed),
            );
            // ---- the declared slots, printed WHETHER OR NOT they are zero ------------------
            //
            // ★★★ A zero here is the answer, not a missing row. `w266`'s eight watches all
            // read `last_seen=0x00000000`; this prints the same word beside the whole 16-byte
            // report the guest asked for (`STRUCTURE_SIZE = FOUR_WORDS`), so *"the payload
            // slot is zero"* and *"the report was never written"* stop being one fact.
            for &(off, va, proc, chan) in &here {
                let i = (off / 4) as usize;
                let body: Vec<String> = (0..4)
                    .map(|k| match words.get(i + k) {
                        Some(w) => format!("0x{w:08x}"),
                        None => "PAST-END".into(),
                    })
                    .collect();
                eprintln!(
                    "    SEMA-PAGE-SLOT gpa=0x{page_gpa:x}+0x{off:03x} va=0x{va:x} proc={proc} \
                     chan={chan} kind=GR-REPORT-SEMAPHORE report16=[{}]",
                    body.join(","),
                );
            }
            // ---- every non-zero slot, with its offset and whose target it is ---------------
            if nonzero.is_empty() {
                // ⊘⊘ SAID IN FULL, because this is a RESULT and it will be read as a failure
                // otherwise. An all-zero page refutes *"the write landed somewhere nobody
                // watches"* outright.
                eprintln!(
                    "    SEMA-PAGE-ZERO gpa=0x{page_gpa:x} ⊘ ALL {} SLOTS ARE ZERO. This is a \
                     MEASUREMENT, not a failed read: {} bytes were read successfully and every \
                     one of them is 0. ⇒ nothing observable has been written to this page.",
                    words.len(),
                    SEMA_PAGE_BYTES,
                );
            } else {
                let shown = nonzero.len().min(SEMA_PAGE_SLOTS_LISTED);
                let mut line = String::new();
                for (n, &i) in nonzero.iter().take(shown).enumerate() {
                    let off = i * 4;
                    let tag = Self::whose(off as u64, &here);
                    line.push_str(&format!(" +0x{off:03x}=0x{:08x}{tag}", words[i]));
                    if n % 6 == 5 || n + 1 == shown {
                        eprintln!("    SEMA-PAGE-NZ gpa=0x{page_gpa:x}{line}");
                        line.clear();
                    }
                }
                if shown < nonzero.len() {
                    eprintln!(
                        "    SEMA-PAGE-NZ gpa=0x{page_gpa:x} ⚠ LISTING-BOUND: {shown} of {} \
                         non-zero slots enumerated. ⊘ The TOTAL above is exact; only this \
                         enumeration is bounded.",
                        nonzero.len(),
                    );
                }
            }
        }
    }

    /// Whose target an offset is — attributed **only** from declarations this device holds.
    ///
    /// ⊘ `[UNCLAIMED]` is the honest answer for everything else and it is not a synonym for
    /// *"the copy engine's"*. `[measured, w266]` the eight `Xid` belong to `engine=Ce`
    /// channels whose `SET_SEMAPHORE_A/B` operand this device has never decoded into a watch,
    /// so an offset outside the declared set is *"nobody we can name"* — and naming it anyway
    /// would be the census counting our own intent instead of the guest's bytes.
    fn whose(off: u64, here: &[(u64, u64, u32, u32)]) -> String {
        for &(d, _, proc, chan) in here {
            if off == d {
                return format!("[GR-REPORT p{proc}c{chan}]");
            }
            // The other three words of a `FOUR_WORDS` report — a timestamp the engine writes,
            // not a payload. Attributed, because a non-zero timestamp beside a zero payload is
            // a completely different finding from a non-zero payload.
            if off > d && off < d + 16 {
                return format!("[GR-REPORT-BODY+{} p{proc}c{chan}]", off - d);
            }
        }
        "[UNCLAIMED]".into()
    }

    /// The reader's own numbers, so silence is legible.
    fn close(&self) {
        eprintln!(
            "kayfabe: SEMA-PAGE-READER stopped — looks={} dumps_printed={} \
             dumps_suppressed={} pages={} (⊘ `dumps_printed=0` with `pages>0` means the \
             reader ran and never found a reason to print; `pages=0` means NOTHING WAS EVER \
             DECLARED IN GUEST RAM and every SEMA-PAGE row is absent by construction, which \
             is a statement about the declare path and not about the page)",
            self.ticks,
            self.printed,
            self.suppressed,
            self.seen.len(),
        );
        // ★★★★★ **THE WRITE CENSUS'S VERDICT, as three numbers on one line.**
        //
        // ⊘ Printed on its own row and always, because the three states it separates are the
        // whole of arm 2.2 and two of them are absences:
        //
        // | reading | what it means |
        // |---|---|
        // | `looks=0` | the reader never ran ⇒ **NOT** "nothing was written" |
        // | `looks>0 transitions=0` | sampled and **frozen from the first look** — a page
        //   frozen after NOTHING |
        // | `transitions>0 backwards=0` | somebody wrote, monotonically — one writer, or
        //   several that never disagreed |
        // | `backwards>0` | ★★★ a **DECREASE**: the M5.38 second-writer shape, the C's own
        //   measured disease, and the thing "delete the second writer" fixed |
        eprintln!(
            "kayfabe: SEMA-WRITE-CENSUS looks={} words_tracked={} transitions={} \
             backwards_on_payload={} decreases_elsewhere={} ⇒ {}",
            self.ticks,
            self.words.len(),
            self.transitions,
            self.backwards,
            self.decreases_elsewhere,
            match (self.ticks, self.transitions, self.backwards) {
                (0, _, _) =>
                    "⊘ THE READER NEVER RAN — this row is NOT evidence that nothing \
                              was written",
                (_, 0, _) =>
                    "FROZEN FROM THE FIRST LOOK — at this cadence nothing ever changed. ⊘ \
                     Bounds 'no persistent change', never 'nobody wrote'",
                (_, _, 0) =>
                    "NO PAYLOAD WENT BACKWARDS — ⊘ scoped to the DECLARED payload words. A \
                     decrease on a timestamp low word is a 64-bit clock carrying and is \
                     counted in `decreases_elsewhere`, not here",
                _ =>
                    "★★★ A PAYLOAD WENT BACKWARDS — the M5.38 second-writer signature. See \
                      the SEMA-WRITE rows for which, when, and whose",
            }
        );
    }
}

#[cfg(all(test, feature = "host-isolates"))]
mod sema_page_reader_tests {
    use super::SemaPageReader;

    /// The eight `w266` declarations, as `look` builds them: `(offset, va, proc, chan)`.
    /// `[measured, w266_on, `run_w266_on_qemu.log`]` — `0x20440ff80 … 0x20440fff0`, 16-byte
    /// stride, `gpa 0x2197ef80 … 0x2197eff0`, all `proc=2`, `chan=0..7`.
    fn w266_declares() -> Vec<(u64, u64, u32, u32)> {
        (0..8u64)
            .map(|c| {
                let off = 0xff0 - c * 0x10;
                (off, 0x2_0440_f000 + off, 2, u32::try_from(c).unwrap())
            })
            .collect()
    }

    /// ★★★★★ **THE ROW THE WHOLE RUNG TURNS ON.** An offset outside every declared report
    /// must come back `[UNCLAIMED]` and **must not** be attributed to the copy engine.
    ///
    /// `[measured, w266]` the eight `Xid` belong to `engine=Ce` channels whose
    /// `SET_SEMAPHORE_A/B` operand this device has never decoded into a watch. If a payload
    /// turns up at, say, `+0x000`, the honest answer is *"nobody we can name"* — writing
    /// `[CE-SEMAPHORE]` there would be the census counting **our inference** instead of the
    /// guest's bytes, which is the failure class `our_census_counts_intent` is named for.
    #[test]
    fn an_offset_no_declaration_names_is_unclaimed_and_is_not_guessed_to_be_the_copy_engines() {
        let d = w266_declares();
        for off in [0x000u64, 0x010, 0x080, 0x400, 0xf70, 0xf7c] {
            let t = SemaPageReader::whose(off, &d);
            assert_eq!(
                t, "[UNCLAIMED]",
                "offset 0x{off:x} is nobody's that we can name"
            );
            assert!(!t.contains("CE"), "★ never guessed: {t}");
        }
    }

    /// Every declared payload slot is attributed to the channel that declared it — by
    /// `proc`/`chan`, so a slot in a shared page is never anonymous.
    #[test]
    fn every_declared_payload_slot_names_its_own_channel() {
        let d = w266_declares();
        assert_eq!(SemaPageReader::whose(0xff0, &d), "[GR-REPORT p2c0]");
        assert_eq!(SemaPageReader::whose(0xf80, &d), "[GR-REPORT p2c7]");
        assert_eq!(SemaPageReader::whose(0xfe0, &d), "[GR-REPORT p2c1]");
    }

    /// ★★★ **The other three words of a `FOUR_WORDS` report are the TIMESTAMP, not the
    /// payload, and conflating them would invert a finding.** A non-zero timestamp beside a
    /// zero payload says *"the engine wrote and the payload is not what the guest waits for"*;
    /// a non-zero payload says the wait is satisfied. They must not render the same.
    #[test]
    fn the_report_body_is_distinguished_from_the_payload_word() {
        let d = w266_declares();
        assert_eq!(SemaPageReader::whose(0xff4, &d), "[GR-REPORT-BODY+4 p2c0]");
        assert_eq!(SemaPageReader::whose(0xff8, &d), "[GR-REPORT-BODY+8 p2c0]");
        assert_eq!(SemaPageReader::whose(0xffc, &d), "[GR-REPORT-BODY+12 p2c0]");
        // ⊘ And the word one past the end of chan 0's report is NOT chan 0's — the reports
        // abut at a 16-byte stride, so an off-by-one here would silently re-label a whole
        // neighbouring channel's slot.
        assert_eq!(SemaPageReader::whose(0xfe0, &d), "[GR-REPORT p2c1]");
    }

    /// ⊘ A signature that cannot tell a written page from a blank one would make every
    /// heartbeat print and every change invisible — the instrument failing in the direction
    /// that looks like data.
    #[test]
    fn the_signature_separates_a_written_page_from_a_blank_one() {
        let blank = vec![0u8; super::SEMA_PAGE_BYTES];
        let mut one = blank.clone();
        one[0xff0] = 1;
        assert_ne!(
            SemaPageReader::signature(&blank),
            SemaPageReader::signature(&one),
            "★ a single byte at the declared offset must change the signature"
        );
        assert_eq!(
            SemaPageReader::signature(&blank),
            SemaPageReader::signature(&vec![0u8; super::SEMA_PAGE_BYTES]),
            "and an unchanged page must not"
        );
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
        // ★★★ RESIDENCY, not bytes — and ★★★★★ **THE JOIN FIRST**, which this did not do.
        //
        // ⊘⊘ **THE DEFECT THIS LINE HAD, measured `[2026-08-12, boot `w278b_guest`]`.** It
        // read `page_writer(...).is_some()` and returned `Some(false)` whenever the store
        // held no first-writer row. `SparseFb::install_join` **deletes those rows** for a
        // joined range — that deletion is correct, it is what makes the leaf one memory —
        // so every joined page answered *"never written"*. The raw CE client's GPFIFO ring
        // sat in exactly such a leaf (`GR-RING-JOIN … leaf va=0x120020000 fb_phys=0x40000
        // → JOINED (shared)`, ring page `0x41000`), its CPU stores through
        // `NV_ESC_RM_MAP_MEMORY` DID land, `FbStore::read` served them back byte-correct —
        // and this method refused the doorbell with `FwdFault::RingFbNeverWritten` anyway.
        //
        // ⇒ A joined page is `None` = **unmeasured**, and `fetch_ring_bytes` refuses only on
        // `Some(false)`. ⚠ The guard is genuinely weaker there and must be: a zero-filled
        // joined page is indistinguishable from a quiet ring, and this store cannot tell.
        // Answering `Some(false)` to keep a guard alive is inventing a fact about the guest.
        p.fb_page_standing(phys).written()
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

/// ★★★★★ `[w281]` **Read the PUSHBUFFER out of our own framebuffer too — its OWN flag.**
///
/// `w279` ended at `FwdFault::PushbufferAperture { va: 0x1_2002_0000 }`: the ring was read
/// through the join, and the pushbuffer the ring *points at* was refused by a hard-coded
/// `VidmemRoute::Refuse` placed there so that boot could attribute the ring's bytes. This
/// is that widening, and it is deliberately **not** `RING_VIDMEM_ENV`: that rung's own
/// result ruled *"as its own flag, never folded into route B"*, because a single flag makes
/// a boot unable to say which of the two reads produced a byte.
///
/// ⊘ **Necessary-not-sufficient alone.** The bytes still come from the `FbSource` route B
/// registers, so `KAYFABE_RING_VIDMEM` must ALSO be on or a vidmem pushbuffer run refuses
/// exactly as before. Both are printed unconditionally, on both arms.
const PUSHBUF_VIDMEM_ENV: &str = "KAYFABE_PUSHBUF_VIDMEM";

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
    /// ★★★★★ **THE THIRD OUTCOME** — leaves the settlement decoded, approved and then
    /// displaced inside its own desired-set map, so they reached neither `bound` nor
    /// `refusals`. See [`kayfabe_mmu::reach::Settlement::shape_collisions`].
    collisions: Vec<kayfabe_mmu::reach::ShapeCollision>,
    /// Byte-identical leaves seen twice — benign, and kept apart from `collisions` so a
    /// duplicate cannot read as a contradiction.
    duplicates: usize,
    /// Every straddle refusal, whole, so [`straddle_census`] can say what differed.
    straddles: Vec<kayfabe_mmu::walker::PopulateRefusal>,
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
        // ★★★★★ w315 — THE SEGMENT BRACKET. Opened at the top of the port so every leg
        // below is a closed interval on ONE clock, on this vCPU thread, inside the MMIO trap
        // the guest is halted for. See `crate::kftime` for what this may and may not say.
        let mut kft = crate::kftime::Segs::start();
        let mut seen: Option<kayfabe_rt::device::CeChannelFacts> = None;
        if let Some(report) = self.try_ce_submission(token, &mut seen) {
            // ⊘ The CE arm returns TERMINALLY, and on the shipping configuration it claims
            // every routed doorbell. A bracket that only closed on the fall-through would
            // therefore measure the path the guest does NOT take, and report `events=0` while
            // looking armed — `a_census_zero_needs_a_known_positive`, exactly.
            kft.mark("ce_terminal");
            crate::kftime::record("doorbell_ce", &mut kft);
            return report;
        }
        kft.mark("ce_try");
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
        kft.mark("ringproj");
        // ★★★★★ **§16.82 — and BEFORE it, the transport G1 does not have.** The order is the
        // same argument one comment up: a page witnessed after the pass that would have
        // decoded it is a page that binds a doorbell too late. See
        // [`Self::witness_executor_fb_pages`] for the census that says why this is 96.8 % of
        // the pages in this boot, and why the disarmed arm is the control.
        // ★★★★★ **THE SWEEP RUNS HERE, AFTER THE DECODE PASS AND BEFORE EVERY CONSUMER.**
        //
        // Two orderings, both load-bearing and both the same argument one plane apart:
        // - **after** `decode_cpu_pt_writes`, because that pass is what sets the dirty bit the
        //   sweep re-arms on; running first would answer with a trigger one doorbell stale;
        // - **before** `forward_ring`, the pins and `SharedDevice::doorbell` below, because a
        //   mapping published after the ring has been rung is a mapping published after the
        //   engine has already faulted for it.
        // ★★★ w315 — the four clauses are evaluated into locals and marked SEPARATELY from
        // the `eprintln!` that consumes them. The print happens on the vCPU under the BQL and
        // is the instrument's own cost as much as the plane's; folding it into `ptdecode`
        // would charge the guest's latency to a page-table pass that may not have run.
        let pt_witness = self.witness_executor_fb_pages();
        kft.mark("pt_witness");
        let pt_decode = self.decode_cpu_pt_writes();
        kft.mark("pt_decode");
        // ★ w313 — the sweep is a SEPARATE clause from the census, and it is silent when
        //   disarmed. The census below is unconditional (w304's fix, kept), so a reader can
        //   tell "the census ran and found nothing" from "the sweep was not armed".
        let pt_sweep = self.sweep_cpu_pt_tables();
        kft.mark("pt_sweep");
        let pt_vascensus = self.vas_census();
        kft.mark("pt_vascensus");
        // ★★★ w318 — the gate's own fire/skip ratio rides HERE, on a line every build emits
        // and every doorbell prints. Pre-registered outcome (B) — *"it fires and the trap does
        // not drop"* — is indistinguishable from a working gate by `trap_ms` alone; only this
        // ratio separates them. ⊘ It is the WHOLE-BOOT running total; the per-doorbell counts
        // are on the `VAS-PUBLISH` line, and the two are different questions.
        // ★★★★★ **w323 — THE TRAP WITNESS'S OWN RATIO, on the line every doorbell prints.**
        //
        // Same argument as the `w318` dirty-gate census beside it: a gate whose fire/skip
        // ratio is not printed is indistinguishable from a gate that never fired. ⊘ This one
        // is the whole grading criterion for "publication is off the BQL": `inline_exceptions`
        // counts host RM verbs that ran **with the BQL held**, and the target state of
        // `publication_off_the_bql.md` is **zero**. It is the WHOLE-BOOT running total.
        //
        // ⚠ `worst_trap` prints `UNMEASURED` rather than `0` when no guard has closed — an
        // absent measurement and an instantaneous one are different facts and this tree has
        // paid for reading one as the other.
        eprintln!(
            "kayfabe: PT-DECODE token={token:#010x}{pt_witness}{pt_decode}{pt_sweep}{pt_vascensus} | {} | {}",
            self.dirty.census(),
            kayfabe_util::trapwitness::census(),
        );
        kft.mark("log_ptdecode");
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
        kft.mark("bindcensus");
        // ★★★★★ **§5.8 — THE FIRST GUEST BYTE.** Ordered HERE, and the order is the whole
        // argument: the pin resolves the ring's VA through the address table, and the
        // table only carries that binding because the populate pass one line up has just
        // committed it. Before it, this would read `AddressFault::Miss` on every doorbell
        // and the miss would be an artefact of ordering rather than a fact about the guest.
        //
        // ⊘ Silent — not merely quiet — when the crossing is not armed. See
        // `SharedDoorbell::guest_ram_backing`.
        if let Some(line) = self.pin_ring_guest_ram(token, seen.as_ref()) {
            kft.mark("pin_ring");
            eprintln!("kayfabe: {line}");
            kft.mark("log_pin_ring");
        } else {
            kft.mark("pin_ring");
        }
        // ★★★★★ **LEG 7 (w282) — AND IT IS HERE FOR LEG 4's REASON, THREE PLANES ON.**
        //
        // `[measured 2026-08-12, w281_client]` with the pushbuffer route armed a real host copy
        // engine fetched the guest's methods and faulted `FAULT_PTE ACCESS_TYPE_VIRT` at the
        // destination operand the guest's own pushbuffer declared; `[w281b_clientsweep]` binding
        // that operand made it resolve to our EMULATED FRAMEBUFFER, which routes the copy to
        // `CeExecutor::Ours` and is refused before submission. Both walls are one missing thing:
        // **the operand is not memory a real engine can be pointed at.** A mapping installed
        // after the ring has been rung is a mapping installed after the engine has already
        // faulted for it, so this runs **above** `SharedDevice::doorbell` exactly as legs 4, 5
        // and 6 do.
        //
        // ⊘ It returns a `String` and gates nothing — same shape, same opacity property.
        //
        // ⚠ It is ordered **after** leg 6 deliberately, and the two are disjoint by
        // construction: leg 6 serves the operand pages that bind in GUEST RAM and refuses a
        // framebuffer binding by name; this serves exactly the ones it refused. Running it
        // first would not change what either does — they partition the same page set — but the
        // order makes the two lines readable as a partition rather than as a race.
        //
        // ⊘ Silent — not merely quiet — on the disarmed arm. See `SharedDoorbell::operand_join`.
        crate::kftime::maybe_inject("operand_join");
        if let Some(line) = self.join_operand_fb_leaves(token, seen.as_ref()) {
            kft.mark("operand_join");
            eprintln!("kayfabe: {line}");
            kft.mark("log_operand_join");
        } else {
            kft.mark("operand_join");
        }
        // ★★★★★ **LEG 8 — w290's publication**, and its position is the C's own invariant:
        // *"a mapping is always backed before the engine that uses it runs."* It is ordered
        // after the decode pass and the sweep (which populate the rows it publishes) and
        // after leg 7 (whose leaves it would otherwise re-ask for and find already backed),
        // and **before** `SharedDevice::doorbell` below — a mapping published after the ring
        // has been rung is a mapping published after the engine has already faulted for it.
        //
        // ⊘ Returns a `String` and gates nothing: no `?`, no early return, no branch between
        // it and the forward. Same shape and same opacity property as legs 4-7.
        //
        // ⊘ Silent — not merely quiet — on the disarmed arm, so the control's log stays
        // byte-comparable.
        crate::kftime::maybe_inject("vas_publish");
        if let Some(line) = self.publish_vas_rows(token, seen.as_ref()) {
            kft.mark("vas_publish");
            eprintln!("kayfabe: {line}");
            kft.mark("log_vas_publish");
        } else {
            kft.mark("vas_publish");
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
        // ★★★★★ **w288 — THE ERROR NOTIFIER, DERIVED BEFORE THE PLANE LOCK IS TAKEN.**
        //
        // ⊘ It has to be: `err_notifier_grant` takes `self.ce.vmm` itself for the layout
        // read, and the mutex two lines below is the same one. Deriving it inside that guard
        // would deadlock on a plain `std::sync::Mutex`.
        //
        // ⊘ It is read off the `seen` facts this doorbell already resolved — NOT a second
        // `ce_channel_facts` call. Two resolutions of one fact can disagree, and this file
        // has already paid for that shape once (`SharedDoorbell::ring`'s own note).
        //
        // ⚠ Consumed only when this doorbell is the one that BIRTHS the host channel;
        // `plan_doorbell` drops it otherwise, because `hObjectError` is a birth parameter.
        let err_notifier = err_notifier_grant(
            &self.ce,
            self.guest_ram_backing,
            seen.as_ref().and_then(|f| f.error_notifier),
            &format!("doorbell token={token:#010x}"),
        );
        kft.mark("err_notifier");
        let mut held = self.ce.vmm.lock().unwrap_or_else(|e| e.into_inner());
        kft.mark("vmm_lock");
        let port = held.as_mut().map(|v| v as &mut dyn kayfabe_vmm::Vmm);
        // ★★★★★ w315 — THE FORWARD. Everything under here is the core's plan/execute split
        // and, on the forwarding arm, a BLOCKING IPC round trip to the isolate child. The
        // sub-total of that IPC is taken separately below, by a counter the child's caller
        // owns, so `core` minus `core_rm_ipc` is the time spent NOT in the host RM.
        crate::kftime::maybe_inject("core");
        #[cfg(feature = "host-isolates")]
        let ipc0 = kayfabe_isolate_host::isolate::ipc_totals();
        let rung = self
            .device
            .doorbell(port, DOORBELL_TARGET_GPU, token, &[], err_notifier);
        kft.mark("core");
        #[cfg(feature = "host-isolates")]
        {
            let ipc1 = kayfabe_isolate_host::isolate::ipc_totals();
            // ⊘ A DERIVED row, not a bracket: it is charged inside `core` and is printed
            // beside it so the two can be subtracted. It must never be added to the
            // marked sum, or `core` would be counted twice — which is why it goes on the
            // line as its own field rather than through `mark`.
            kft.note_nested("core_rm_ipc", ipc1.1.saturating_sub(ipc0.1), ipc1.0.saturating_sub(ipc0.0));
        }
        drop(held);
        kft.mark("vmm_unlock");
        crate::kftime::record("doorbell_fwd", &mut kft);
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

/// ★★★★★ **What [`SharedDoorbell::pin_guest_run`] made of one contiguous run** — and the
/// reason it is a struct rather than a bool is the whole of `w271`.
///
/// ⚠⚠ `w270` reported a **partial** mapping with the same word it used for a complete one
/// (`ALREADY PINNED … placed_as_asked=true`), so the truncation was invisible in our own
/// instrument and was found by a **hardware fault** instead. ⇒ [`PinnedRun::verdict`] has
/// three values that never overlap, and [`PinnedRun::requested`] and
/// [`PinnedRun::described`] are printed **side by side** so a mismatch is read rather than
/// inferred.
struct PinnedRun {
    /// One of three, never shared: `PINNED`, `ALREADY PINNED (… fully covered)`, `GREW …`.
    verdict: &'static str,
    /// How many bytes the run asked for.
    requested: u64,
    /// How many bytes are described to RM over `[va, va+described)` after this call.
    described: u64,
    /// How many of those bytes this call newly described.
    fresh_bytes: u64,
    /// ⊘⊘ **The `OS_DESCRIPTOR` handle ONLY when this run is one segment.** `0` means *"more
    /// than one descriptor covers this run"*, never *"no descriptor"* — read
    /// [`PinnedRun::segments`], which names each one.
    ///
    /// ⚠ `[measured 2026-08-12, w271_pin]` the first cut of this struct filled `memory`,
    /// `host_va` and `placed_as_asked` from *the first segment only, and only when that
    /// segment did host work* — so every `GREW` row printed `memory=0x0
    /// placed_as_asked=false` beside four descriptors that had all landed exactly as asked.
    /// ⇒ **A false report in the same class this rung exists to close, pointed the other
    /// way**: a green situation summarised as red. The per-segment detail was right
    /// throughout; the summary was not.
    memory: u64,
    /// The run's base host GPU VA.
    host_va: u64,
    /// ★ Whether **every segment this call placed** landed where it was asked — vacuously
    /// true when it placed none, because a step-past segment was placed by an earlier call
    /// that made its own assertion.
    placed_as_asked: bool,
    /// ★ Whether the WHOLE requested extent is now described. ⊘ A caller counting placements
    /// must read this and not `verdict`: a run stopped part-way is not a placement.
    covered: bool,
    /// ★★★★★ **Whether this run was a PARTIAL hit that had to be extended.** ⊘ A separate
    /// bool rather than a substring test on [`PinnedRun::verdict`]: a counter that read the
    /// prose would silently stop counting the day the prose was reworded, which is how the
    /// original defect stayed invisible.
    grew: bool,
    /// One entry per segment, in address order — empty for the single-segment case is
    /// deliberately NOT how it works: even one segment records itself, so a reader never has
    /// to guess whether the absence of detail means "one segment" or "not recorded".
    segments: Vec<String>,
}

impl PinnedRun {
    /// The tail of the log line, after `→ file offset 0x…  → `.
    fn line(&self) -> String {
        format!(
            "{} requested={} described={} fresh={} segs={} memory={} host_va=0x{:x} \
             placed_as_asked={} {}",
            self.verdict,
            self.requested,
            self.described,
            self.fresh_bytes,
            self.segments.len(),
            // ⊘ `(several)` rather than `0x0`: a zero handle READS as a failure, and on a
            //   grown run every descriptor in it landed. See the field's own doc.
            if self.segments.len() > 1 {
                "(several — see the per-segment list)".to_string()
            } else {
                format!("{:#x}", self.memory)
            },
            self.host_va,
            self.placed_as_asked,
            self.segments.join(" "),
        )
    }
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
    /// Latch this GR channel's USERD so the observer thread can sample its cursor. See
    /// [`GrCursorWatch`].
    ///
    /// ⊘ Prints **once per channel** and says which of the three things happened: latched, no
    /// USERD declared at all, or a USERD with no framebuffer address. The third is a real and
    /// legal case (a `Sysmem` USERD lives in guest RAM) and it is named rather than folded
    /// into silence, because an absent `GR-CURSOR` row must never read as `GET = 0`.
    fn latch_gr_cursor(&self, token: u64, facts: &kayfabe_rt::device::CeChannelFacts) {
        let fresh = {
            let mut held = self.ce.gr_cursors.lock().unwrap_or_else(|e| e.into_inner());
            if held
                .iter()
                .any(|w| w.proc == facts.proc.0 && w.chan == facts.chan.0)
            {
                None
            } else {
                let Some(userd) = facts.userd else {
                    // ⊘ Nothing to latch, and it is pushed as no row at all rather than as a
                    // row with a hole. The line below still prints, once, because a channel
                    // this instrument cannot see is a fact about the instrument.
                    drop(held);
                    self.say_gr_cursor_once(format!(
                        "kayfabe: GR-CURSOR token={token:#010x} proc={} chan={} engine={} → NOT \
                         LATCHED: this channel declared no USERD this port could read. ⊘ NOT \
                         `GET = 0` — nothing was read",
                        facts.proc.0,
                        facts.chan.0,
                        facts.engine_name(),
                    ));
                    return;
                };
                let w = GrCursorWatch {
                    token,
                    proc: facts.proc.0,
                    chan: facts.chan.0,
                    engine: facts.engine_name(),
                    userd,
                };
                held.push(w);
                Some(w)
            }
            // ⚠ The guard dies HERE, before anything below allocates, formats or writes to
            // stderr. `l1_concurrency.md` §3.3 R1, and the shape `gr_dumps` is classified
            // safe under.
        };
        let Some(w) = fresh else { return };
        // ⊘ The doorbell-instant sample is printed too, and it is NOT the measurement — it is
        // the `t = 0` row the observer's later rows are read against. A cursor that was
        // already advanced when the guest rang and a cursor that advanced afterwards are
        // different findings, and only a first row separates them.
        let at_doorbell = match self.plane.upgrade() {
            Some(plane) => fb_userd_cursors(&plane, Some(w.userd)),
            None => " (no plane)".to_string(),
        };
        eprintln!(
            "kayfabe: GR-CURSOR token={token:#010x} proc={} chan={} engine={} why=doorbell \
             LATCHED{at_doorbell} ⊘ a `GET` read on THIS line is taken microseconds after the \
             guest wrote `PUT` and proves nothing; the observer's later rows are the \
             measurement",
            w.proc, w.chan, w.engine,
        );
    }

    /// One bounded `GR-CURSOR` line for a channel that could not be latched, sharing
    /// [`GR_PUSHBUFFER_DUMPS_MAX`]'s budget with the other once-per-channel notices.
    fn say_gr_cursor_once(&self, line: String) {
        let mut n = self.ce.gr_dumps.lock().unwrap_or_else(|e| e.into_inner());
        if *n <= GR_PUSHBUFFER_DUMPS_MAX {
            *n += 1;
            drop(n);
            eprintln!("{line}");
        }
    }

    fn declare_gr_completion(&self, token: u64, facts: &kayfabe_rt::device::CeChannelFacts) {
        // ★★★ FIRST, above every gate: "the observer was reached" is a different fact from
        // "the observer declared something", and a single counter cannot separate them.
        self.ce.watch.attempt();
        // ★★★★★ **THE OWNER'S `GP_GET` DIAGNOSTIC — LATCHED HERE, READ LATE.** See
        // [`GrCursorWatch`] for why the read cannot happen on this line: each GR channel is
        // rung once, so a cursor sampled here is sampled microseconds after the guest wrote
        // `GP_PUT`, and `GET = 0` at that instant means nothing at all.
        //
        // ⊘ Placed ABOVE every gate in this function, deliberately. A channel whose
        // submission declares no report semaphore returns `Ok(None)` below and would never be
        // latched — and *"this channel declared no completion"* is not a reason to stop asking
        // whether its engine ever ran. The three-way discriminator must cover every GR channel
        // the guest rang, not only the ones that got as far as declaring.
        //
        // ⚠ Idempotent by `(proc, chan)`; the guard is dropped before the `eprintln!`.
        self.latch_gr_cursor(token, facts);
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
    ///   ⊘⊘ **CORRECTED 2026-08-12** — the reason given for it here was *"`gr_doorbell_
    ///   passthrough.md` §0.3: the host GR channel's ring and its `GP_PUT` are both ours on
    ///   either arm, so the host engine fetches nothing"*, and **both halves of that are now
    ///   false**: `[measured, w267_on]` all eight `GrCompute` births read
    ///   `adopt=GUEST-RING userd=GUEST-USERD`. The claim *"nothing executed"* still holds on
    ///   the `refuse` arm — for the simpler reason that no GR doorbell reaches
    ///   `SharedDevice::doorbell` at all, so `rm.schedule`/`rm.ring_doorbell` are never
    ///   called — but it is no longer derivable from the ring or the cursor being ours.
    ///   `docs/design/w268_the_cursor_and_the_arm_prereg.md` §0.1. **The guest did not move.**
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
        // ⚠ **THIS RE-OPENS A PATH THAT WAS CLOSED ON EVIDENCE.** Read `GR_ROUTE_ENV` and
        // `docs/design/gr_doorbell_passthrough.md` §0.2 before reading a boot that ran the
        // armed arm.
        //
        // ⊘⊘ **CORRECTED 2026-08-12 — the sentence that used to stand here is REFUTED, and it
        // is the sentence that made the armed arm look pointless.** It read: *"the host GR
        // channel's ring **and** its `GP_PUT` are both ours, so the host engine fetches
        // nothing on either arm. The armed arm buys the TRANSPORT and nothing else."* That
        // was true of the birth path as it stood on 2026-08-11 (`RingSource::Ours(None)`).
        // **Legs A2 and B moved it.** `[measured 2026-08-12, w267_on, all 16 `GR-BIRTH iso2`
        // lines]` every birth — **eight of them `engine=GrCompute`** — reads
        // `adopt=GUEST-RING userd=GUEST-USERD → alloc_channel_over_guest_ring`: the host GR
        // channel's ring **is** the guest's `0x200200000`, and its `GP_PUT` **is** the word in
        // the guest's own USERD page.
        // ⇒ The armed arm may buy more than transport, and whether it does is a MEASUREMENT
        // (`GR-CURSOR`, `docs/design/w268_the_cursor_and_the_arm_prereg.md`), not a deduction
        // from either sentence. ★ The *posture* is unamended: `refuse` is still the default
        // and arming is still a printed choice with a control arm.
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
    /// ★★★★★ **Pin ONE contiguous run, and DESCRIBE ITS GROWTH** — the primitive all four
    /// guest-RAM pin sources share, and the one place the `(base, extent)` key is honoured.
    ///
    /// # Why this exists at all
    ///
    /// `[measured 2026-08-12, boot `w270_pin`]` the operand source asked for **64 KiB** at a
    /// base already described for **32 KiB**. The core answered `already` — its idempotence
    /// key was the VA — and the caller printed `ALREADY PINNED (idempotent replay) …
    /// placed_as_asked=true`. **The second 32 KiB was never described to RM**, and the host
    /// GPU faulted on the first byte past the described extent, to the byte. ⇒ A green supply
    /// row held the wall in place, and only an independent authority made it visible.
    ///
    /// # The shape, and why the split lives HERE and not in the core
    ///
    /// `kayfabe_fwd::plan_pin_guest_ram` now refuses a growing request by name
    /// ([`kayfabe_fwd::FwdFault::GuestRamPinTooShort`]) and hands back *how much is
    /// described*. It may not do more: **only the VMM may derive a grant** — the whole
    /// content of `GuestRamGrant::originated_by_the_vmm`'s name and of `#238`. So the
    /// remainder's `(file_offset, len)` is computed **here**, from the hypervisor's own
    /// stated run, and re-entered as an ordinary pin at `va + described`.
    ///
    /// ⊘ That is not a new mechanism: a *fragmented* range already becomes several pins at
    /// several bases. Growth just reaches the same shape from the other direction.
    ///
    /// # ⚠ Termination
    ///
    /// Every non-terminal arm advances `covered` by a **strictly positive** number
    /// (`described`, or `free_prefix`), so the walk is bounded by `len`. A zero from either
    /// is treated as terminal rather than retried — otherwise a malformed answer becomes a
    /// spin inside a doorbell, and a doorbell holds the guest's vCPU.
    fn pin_guest_run(
        &self,
        pdb: kayfabe_rt::Pdb,
        va: u64,
        file_offset: u64,
        len: u64,
    ) -> Result<PinnedRun, kayfabe_rt::FwdFault> {
        let mut out = PinnedRun {
            verdict: "⊘ NOTHING ASKED",
            requested: len,
            described: 0,
            fresh_bytes: 0,
            memory: 0,
            host_va: va,
            placed_as_asked: true,
            covered: len == 0,
            grew: false,
            segments: Vec::new(),
        };
        let mut covered = 0u64;
        let mut fresh_segments = 0usize;
        let mut replay_segments = 0usize;
        while covered < len {
            let seg_va = va + covered;
            let seg_len = len - covered;
            let grant = kayfabe_isolate::GuestRamGrant::originated_by_the_vmm(
                file_offset + covered,
                seg_len,
                kayfabe_vmm::Prot::ReadWrite,
            );
            match self.device.pin_guest_ram(
                DOORBELL_TARGET_GPU,
                pdb,
                kayfabe_rt::GpuVa(seg_va),
                grant,
            ) {
                Ok(p) => {
                    if p.already {
                        replay_segments += 1;
                    } else {
                        fresh_segments += 1;
                        out.fresh_bytes += seg_len;
                    }
                    out.segments.push(format!(
                        "[{}@0x{seg_va:x}+{seg_len} described={} memory={:#x} host_va=0x{:x} \
                         placed_as_asked={}]",
                        if p.already { "replay" } else { "fresh" },
                        p.described,
                        p.memory.raw(),
                        p.host_va,
                        p.host_va == seg_va,
                    ));
                    out.memory = p.memory.raw();
                    out.placed_as_asked &= p.host_va == seg_va;
                    // ★ A replay's live pin covers AT LEAST what was asked (a shorter one
                    // refuses above), and a fresh pin described exactly what was asked. Both
                    // finish the run.
                    covered = len;
                }
                // ★★★★★ THE GROWTH ARM. The base is described for fewer bytes than asked;
                // step past what exists and describe the remainder from the same run.
                Err(kayfabe_rt::FwdFault::GuestRamPinTooShort { described, .. })
                    if described > 0 && described < seg_len =>
                {
                    replay_segments += 1;
                    out.segments.push(format!(
                        "[covered@0x{seg_va:x}+{described} (already described; stepping past \
                         it)]"
                    ));
                    covered += described;
                }
                // ★★ The overlap arm, from the other side: something is pinned INSIDE this
                // run. Describe the clear prefix, then re-enter at the obstruction — where
                // the arm above takes over. `free_prefix == 0` is terminal by construction.
                Err(kayfabe_rt::FwdFault::GuestRamPinOverlaps(o)) if o.free_prefix > 0 => {
                    let prefix = o.free_prefix.min(seg_len);
                    let sub = kayfabe_isolate::GuestRamGrant::originated_by_the_vmm(
                        file_offset + covered,
                        prefix,
                        kayfabe_vmm::Prot::ReadWrite,
                    );
                    let p = self.device.pin_guest_ram(
                        DOORBELL_TARGET_GPU,
                        pdb,
                        kayfabe_rt::GpuVa(seg_va),
                        sub,
                    )?;
                    fresh_segments += 1;
                    out.fresh_bytes += prefix;
                    out.segments.push(format!(
                        "[fresh@0x{seg_va:x}+{prefix} (clear prefix below a pin at \
                         0x{:x}+{}) memory={:#x} placed_as_asked={}]",
                        o.existing_base,
                        o.existing_len,
                        p.memory.raw(),
                        p.host_va == seg_va,
                    ));
                    out.memory = p.memory.raw();
                    out.placed_as_asked &= p.host_va == seg_va;
                    covered += prefix;
                }
                Err(e) => return Err(e),
            }
        }
        out.described = covered;
        out.covered = covered >= len;
        // ⚠⚠ THREE WORDS, NEVER SHARED. `w270` printed one green word over both "we did the
        // work" and "it was already complete", and the partial case wore it too. A reader
        // must be able to see growth without inferring it from two numbers.
        // ⊘ ONE descriptor or none is nameable at run level; several are not. Zeroing it is
        //   deliberate and its meaning is documented on the field — a summary that named one
        //   of four descriptors would be worse than one that names none.
        if out.segments.len() > 1 {
            out.memory = 0;
        }
        out.grew = fresh_segments > 0 && replay_segments > 0;
        out.verdict = match (fresh_segments, replay_segments) {
            (0, 0) => "⊘ NOTHING ASKED",
            (_, 0) => "PINNED",
            (0, _) => "ALREADY PINNED (idempotent replay; fully covered)",
            _ => {
                "GREW (partial hit — the base was described SHORT; the remainder is described now)"
            }
        };
        Ok(out)
    }

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
        // ★ `w271`: FRESH bytes and DESCRIBED bytes are separate accumulators, because a
        //   replay adds zero to the first and its whole extent to the second. Folding them
        //   was how a partial mapping read as a complete one.
        let mut bytes_described = 0u64;
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
            // ⇒ The grant is minted inside `pin_guest_run`, which owns the `(base, extent)`
            //   key and describes any remainder rather than replaying a short mapping.
            match self.pin_guest_run(pdb, rva, run.file_offset, rlen) {
                Ok(p) => {
                    if p.covered {
                        pinned += 1;
                    }
                    bytes_pinned += p.fresh_bytes;
                    bytes_described += p.described;
                    lines.push(format!(
                        "{at} → file offset 0x{:x} → {}",
                        run.file_offset,
                        p.line()
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
                " | ★ ALL {total} RUN(S) PLACED, {bytes_described} of {want} bytes \
                 DESCRIBED ({bytes_pinned} of them freshly, this doorbell) — one REAL \
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
                " | ⚠ {pinned} of {total} run(s) placed, {bytes_described} of {want} bytes \
                 described ({bytes_pinned} fresh). ⚠ If \
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

    /// ★★★★★ **THE OPERAND PIN'S ONLY SOURCE — the pages THIS channel's own `LAUNCH_DMA`
    /// operands name, read at THIS doorbell.**
    ///
    /// Returns the line to print (**always** — an arm that found nothing and an arm that did
    /// not run are different facts) and the distinct 4 KiB page VAs the guest's own
    /// `OFFSET_OUT_*`/`OFFSET_IN_*` operands decoded to, **expanded over each extent**.
    ///
    /// ⚠ **Extent, not base page.** A copy longer than a page faults on its later pages too,
    /// and pinning only `dst & !0xfff` would produce a green supply row beside a live fault —
    /// the exact shape `w266` measured one plane over (0 faults *and* 0 completions).
    ///
    /// ⊘ It resolves nothing and it pins nothing. The caller puts every page through the same
    /// address-table lookup the other two passes use, so `miss = fault` is unchanged.
    ///
    /// See [`kayfabe_rt::ceutils::observe_ce_operand_targets`] for the three boots that made
    /// this necessary and for why it is not the `cap2b` class.
    #[cfg(feature = "host-isolates")]
    fn ce_operand_pages(
        &self,
        token: u64,
        f: &kayfabe_rt::device::CeChannelFacts,
        page: u64,
    ) -> (String, std::collections::BTreeSet<u64>) {
        let head = format!("OPERAND-SOURCE-CE token={token:#010x}");
        let none = std::collections::BTreeSet::new();
        let (Some(vaspace), Some(ring_va)) = (f.vaspace, f.ring_va) else {
            return (
                format!(
                    "{head} → NOT ASKED: vaspace={:?} ring_va={:?} — there is no ring to read \
                     this channel's own methods out of. ⊘ Not a miss",
                    f.vaspace, f.ring_va
                ),
                none,
            );
        };
        let Some(plane) = self.plane.upgrade() else {
            return (
                format!("{head} → NO PLANE (the register plane is gone)"),
                none,
            );
        };
        let root = match SharedDoorbell::doorbell_root(
            &plane,
            f.client,
            vaspace,
            f.vas_pdb.map(|p| p.0),
        ) {
            DoorbellRoot::Published(r) | DoorbellRoot::Declared(r) => r,
            DoorbellRoot::Absent => {
                return (
                    format!("{head} → NO ROOT (this channel has no VA space root to walk)"),
                    none,
                );
            }
            DoorbellRoot::Underivable(p, why) => {
                return (
                    format!("{head} → ROOT UNDERIVABLE from pdb 0x{p:x}: {}", why.kind()),
                    none,
                );
            }
        };
        let chan = kayfabe_rt::ceutils::CeUtilsChannel {
            client: f.client,
            vaspace,
            ring_va,
            ring_entries: f.ring_entries,
        };
        // ⊘ The channel's OWN cursor and OWN accumulator, both read and NEITHER written back —
        // `ce_release_pages`' discipline, for its reasons.
        let cursor = *self
            .ce
            .cursors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(f.proc.0, f.chan.0))
            .unwrap_or(&kayfabe_rt::ceutils::GpCursor::default());
        let state = *self
            .ce
            .states
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(f.proc.0, f.chan.0))
            .unwrap_or(&kayfabe_rt::ceutils::MethodState::default());
        let out = {
            let mut held = self.ce.vmm.lock().unwrap_or_else(|e| e.into_inner());
            let Some(vmm) = held.as_mut() else {
                drop(held);
                return (
                    format!("{head} → NO MEMORY PLANE (nothing to read the ring out of)"),
                    none,
                );
            };
            let demand = kayfabe_device::ceresolve::Demand::from_doorbell();
            plane.ce_session_with_root(&root, demand, |ce| {
                self.device.with_pushbuffer(|pb| {
                    kayfabe_rt::ceutils::observe_ce_operand_targets(
                        ce, pb, vmm, chan, cursor, state,
                    )
                })
            })
            // ⚠ Every lock is released HERE, before the caller pins anything.
        };
        let t = match out {
            Ok(t) => t,
            Err(refusal) => {
                return (
                    format!(
                        "{head} → UNREADABLE: {}. ⊘ A statement about this read, NOT about the \
                         guest's bytes",
                        refusal.describe()
                    ),
                    none,
                );
            }
        };
        let mut pages: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
        let mut dropped = 0usize;
        let mut first_dropped: Option<u64> = None;
        let mut writes = 0usize;
        let mut reads = 0usize;
        for e in &t.extents {
            if e.write {
                writes += 1;
            } else {
                reads += 1;
            }
            // ⚠ THE WHOLE EXTENT, page by page. `saturating_*` throughout: a decoded `len`
            // is the guest's number and a hostile one must clamp rather than wrap.
            let first = e.va.0 & !(page - 1);
            let last = e.va.0.saturating_add(e.len.saturating_sub(1)) & !(page - 1);
            let mut p = first;
            loop {
                if pages.len() >= PUSHBUF_MAX_PAGES && !pages.contains(&p) {
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
        // ⊘ THE SAMPLE IS OF EXTENTS, NOT OF PAGES, and it carries the DIRECTION — the only
        // thing that can attribute a surviving `Xid`'s ACCESS_TYPE once both classes are
        // pinned. `w265`: a count cannot see a substitution; these rows are the identity.
        let sample: Vec<String> = t
            .extents
            .iter()
            .take(PUSHBUF_REPORT)
            .map(|e| {
                format!(
                    "{}@0x{:x}+0x{:x}",
                    if e.write { "W" } else { "R" },
                    e.va.0,
                    e.len
                )
            })
            .collect();
        (
            format!(
                "{head} proc={} chan={} engine={:?} → methods={} launches={} opaque={} \
                 release_only={} physical={} operand(s)={} ({writes} write, {reads} read){} ⇒ \
                 {} page(s). ⊘ Every address here is the GUEST's own OFFSET_OUT_*/OFFSET_IN_* \
                 operand, decoded by the chip's codec at THIS doorbell — never a remembered \
                 page{}",
                f.proc.0,
                f.chan.0,
                f.engine,
                t.methods,
                t.launches,
                t.opaque,
                t.release_only,
                t.physical,
                t.extents.len(),
                pushbuffer_sample(&sample, t.extents.len()),
                pages.len(),
                match first_dropped {
                    Some(va) => format!(
                        " | ⚠⚠ CAPPED at {PUSHBUF_MAX_PAGES} pages — {dropped} DROPPED, first \
                         va=0x{va:x}. ⊘ INCOMPLETE"
                    ),
                    None => String::new(),
                }
            ),
            pages,
        )
    }

    /// ★★★★★ **w282 — LEG 7: JOIN the framebuffer leaves this channel's own CE operands
    /// name**, so the executor stays `HostCe` and a real host engine can be pointed at the
    /// guest's own address.
    ///
    /// # ★★★ It is a CALLER, not a mechanism — and that is the whole finding
    ///
    /// Every step below already existed. [`join_one_fb_leaf`] is the four-step join
    /// (`join → adopt+map → establish+install → bind`) that `w260` built and that
    /// [`Regs::back_census_framebuffer_leaves`] and [`Regs::adopt_pending_channel_rings`]
    /// both use. `[measured 2026-08-12]` the reason a CE operand never reached it is that
    /// the census caller hangs off [`Self::declare_gr_completion`], which [`Self::ring`]
    /// calls on the **GR** dispositions only — so **no CE doorbell has ever presented a
    /// leaf to the join.** This presents them.
    ///
    /// # ★★★★★ PER-VAS, STRUCTURALLY — the owner's *"not denied, simply not found"*
    ///
    /// Three independent per-VAS keyings, and **none of them is a policy check**:
    ///
    /// 1. The operand VAs come from [`Self::ce_operand_pages`], which reads **this
    ///    channel's own ring** through **this channel's own** [`DoorbellRoot`].
    /// 2. Each VA is resolved through [`kayfabe_rt::device::SharedDevice::resolve`] keyed by
    ///    **this channel's `Pdb`** — `mode2_address_table.md` §3, *"keyed by VAS … NOT a
    ///    global VA space"*. A VA bound only in another address space is a
    ///    `Miss`, which is §6's `miss = fault`: **not found**, never "found elsewhere".
    /// 3. The leaf is walked by [`kayfabe_rt::ceutils::resolve_leaf_of`] from **the same
    ///    root**, and bound by `join_one_fb_leaf` into **the same `Pdb`**.
    ///
    /// ⊘ There is no arm here that searches other address spaces, and none that falls back to
    /// one on a miss. A miss is reported and the page is skipped. `[asserted]`
    /// `tests/tests/operand_join_is_per_vas.rs`.
    ///
    /// # ★★ CLEANUP — named now, because a join without a release is a leak
    ///
    /// Every join this pass performs has an owner and an end, and both are *stated* so the
    /// release path is a wiring job rather than a redesign:
    ///
    /// | | |
    /// |---|---|
    /// | **owner** | the `(proc, Pdb)` whose table carries the `JoinsGuestWindow` binding — never the channel, which may die while its VAS lives |
    /// | **unit** | one 64 KiB framebuffer leaf, keyed by `leaf.phys`; two operands in one leaf are **one** join and the second replays |
    /// | **lifetime** | from `adopt_joined_fb_leaf` until the binding is dropped from that `Pdb`'s table |
    /// | **the event that ends it** | the guest's own free/unmap of the range, seen as the page-table leaf ceasing to bind — **and a `Pdb`-scoped sweep at address-space teardown as the backstop**, because the free is not guaranteed to cross (see the module's `RESULT` doc) |
    /// | **the primitive** | [`kayfabe_rt::device::SharedDevice::release_unadopted_fb_leaf`] already stages the unmap; the missing half is the *trigger*, not the mechanism |
    ///
    /// ⊘ **Not wired this rung, and the shape admits it rather than assuming it away.** What
    /// is wired is the idempotence that makes a later release correct: this pass never joins
    /// one leaf twice, so a release is a release of one thing.
    ///
    /// # ⊘ It returns a `String` and gates NOTHING
    ///
    /// Same shape as legs 4/5/6: no `?`, no early return and no branch on its outcome between
    /// it and `SharedDevice::doorbell`. Whether the doorbell is forwarded cannot depend on
    /// whether a leaf joined.
    #[cfg(feature = "host-isolates")]
    #[allow(clippy::too_many_lines)]
    fn join_operand_fb_leaves(
        &self,
        token: u64,
        facts: Option<&kayfabe_rt::device::CeChannelFacts>,
    ) -> Option<String> {
        // ⊘ SILENT only on `off`. ★★★ On `assert` the pass RUNS and joins nothing — see
        // [`OperandJoinArm`] for the defect this rung's own control found in the two-arm
        // draft: with `#255` inside the armed path, the control printed zero `#255` lines and
        // the instrument's guaranteed known-positive was unreachable.
        if !self.operand_join.observes() {
            return None;
        }
        let head = format!(
            "OPERAND-JOIN token={token:#010x} arm={}",
            self.operand_join.as_str()
        );
        let Some(f) = facts else {
            return Some(format!(
                "{head} → NO CHANNEL (the token routed to no channel, so there is no VA space \
                 to join INTO)"
            ));
        };
        let Some(pdb) = f.vas_pdb else {
            return Some(format!(
                "{head} proc={} chan={} → NO PDB (this channel's VA space did not resolve, so \
                 there is no address space to join into; ⊘ not a miss — nothing was asked)",
                f.proc.0, f.chan.0
            ));
        };
        let who = format!(
            "{head} proc={} chan={} pdb=0x{:x}",
            f.proc.0, f.chan.0, pdb.0
        );
        // ⚠ NECESSARY-NOT-SUFFICIENT, and said out loud rather than left as an absence: the
        // join's own arm (`KAYFABE_FB_JOIN`) selects `Shared` vs `Private` vs `Off`, and with
        // it `Off` this pass would map PRIVATE ANONYMOUS pages — two memories under a name
        // that says one. ⊘ Refused rather than downgraded.
        // ⊘ Enforced only on the arm that would actually join. On `assert` nothing is mapped,
        // so the mapping arm is irrelevant and aborting here would cost the control the very
        // `#255` verdict it exists to produce.
        if self.operand_join.joins() && !self.fb_join.armed() {
            return Some(format!(
                "{who} → ⊘ NOT ARMABLE: KAYFABE_FB_JOIN is `{}`. The join's mapping arm is what \
                 makes the guest's window and the host object ONE memory; with it disarmed this \
                 pass could only map PRIVATE ANONYMOUS pages, which is the two-memories state \
                 under a name that says the opposite. ⊘ Nothing was asked of the host",
                self.fb_join.as_str()
            ));
        }
        let Some(plane) = self.plane.upgrade() else {
            return Some(format!(
                "{who} → ⊘ NO PLANE (the register plane is gone). ⊘ Nothing was asked of the \
                 host and no leaf was touched"
            ));
        };
        // ⊘ Same scoping: the export directory is the route from a backing token to a
        // descriptor and is needed ONLY to join. `assert` runs without one.
        let exports = match (self.exports.as_ref(), self.operand_join.joins()) {
            (Some(e), _) => Some(e),
            (None, false) => None,
            (None, true) => {
                return Some(format!(
                    "{who} → ⊘ NOT ARMABLE: exports_directory=false — this build has no route \
                     from a backing token to a descriptor. ⊘ Nothing was asked of the host and \
                     no leaf was touched"
                ));
            }
        };
        let Some(vaspace) = f.vaspace else {
            return Some(format!(
                "{who} → NO VASPACE (there is no address space handle to root the walk at)"
            ));
        };
        // ★★★ PER-VAS KEYING #1 and #3's root: THIS channel's own installed page-directory
        // base. ⊘ Nothing below may resolve against any other.
        let root = match SharedDoorbell::doorbell_root(&plane, f.client, vaspace, Some(pdb.0)) {
            DoorbellRoot::Published(r) | DoorbellRoot::Declared(r) => r,
            DoorbellRoot::Absent => {
                return Some(format!(
                    "{who} → NO ROOT (this channel has no VA space root, so no operand VA can \
                     be walked to a leaf)"
                ));
            }
            DoorbellRoot::Underivable(p, why) => {
                return Some(format!(
                    "{who} → ROOT UNDERIVABLE from pdb 0x{p:x}: {}",
                    why.kind()
                ));
            }
        };
        // ★ THE SAME SOURCE the pin uses, at THIS doorbell — never a remembered address and
        // never another pass's read. `ce_operand_pages` takes and releases the memory-plane
        // lock and the plane session inside itself, before anything below runs.
        let page = Self::RING_PIN_BYTES;
        let (source, pages) = self.ce_operand_pages(token, f, page);
        let source = format!("{who}\n    {source}");
        if pages.is_empty() {
            return Some(format!(
                "{source}\n    ⊘ NO OPERAND PAGE TO JOIN. ⚠ Read the counters on the line above \
                 before reading this as an absence — `release_only = launches`, `physical > 0` \
                 and `opaque = methods` are three different facts and none of them is *the \
                 decode failed*"
            ));
        }
        // ---- PHASE 1: CLASSIFY, per-VAS, and pick the candidates ---------------------------
        //
        // ⊘ Three populations, kept apart because they are three different findings and a
        // single count would hide two of them:
        //   * `Miss`            — §6's `miss = fault`. NOT FOUND in this VAS. Skipped, loudly.
        //   * guest RAM         — leg 6's population. Already served; not this pass's to touch.
        //   * framebuffer       — THIS pass's population.
        // ★ And within the framebuffer population, one already carrying a host object is
        //   ALREADY JOINED and must not be asked for a second fixed map at an occupied
        //   address (RM answers `0x51`, which ⊘ cannot be told apart from real exhaustion).
        let mut candidates: Vec<u64> = Vec::new();
        let mut n_miss = 0usize;
        let mut n_guest_ram = 0usize;
        let mut n_already = 0usize;
        let mut misses: Vec<String> = Vec::new();
        let mut fb: Vec<String> = Vec::new();
        for &pva in &pages {
            // ★★★ PER-VAS KEYING #2 — `pdb` is this channel's, and `resolve` has no arm that
            // consults another. A VA bound only elsewhere lands in the `Err` below.
            match self
                .device
                .resolve(DOORBELL_TARGET_GPU, pdb, kayfabe_rt::GpuVa(pva))
            {
                Err(e) => {
                    n_miss += 1;
                    if misses.len() < PUSHBUF_REPORT {
                        misses.push(format!("va=0x{pva:x}:{e:?}"));
                    }
                }
                Ok((b, _)) if b.is_guest_ram() => n_guest_ram += 1,
                // ⊘ `host().is_some()` is the JOINED test and it is read, never derived: a
                // framebuffer range that carries a host materialization is one whose window
                // has already been re-pointed (`BackingBytes::JoinsGuestWindow`), and asking
                // for it again is the `0x51` collision above.
                Ok((b, _)) if b.host().is_some() => {
                    n_already += 1;
                    if fb.len() < PUSHBUF_REPORT {
                        fb.push(format!("va=0x{pva:x}:ALREADY-JOINED"));
                    }
                }
                Ok((b, _)) => {
                    if fb.len() < PUSHBUF_REPORT {
                        fb.push(format!(
                            "va=0x{pva:x}:{:?}@0x{:x}/{:?}",
                            b.aperture(),
                            b.phys(),
                            b.kind()
                        ));
                    }
                    candidates.push(pva);
                }
            }
        }
        let table = format!(
            "{source}\n    OPERAND-JOIN-TABLE: {} page(s) asked, {n_miss} MISS{}, {n_guest_ram} \
             in guest RAM (leg 6's population, untouched here), {n_already} ALREADY JOINED, {} \
             CANDIDATE(S) in the emulated framebuffer{}",
            pages.len(),
            pushbuffer_sample(&misses, n_miss),
            candidates.len(),
            pushbuffer_sample(&fb, n_already + candidates.len()),
        );
        if candidates.is_empty() {
            return Some(format!(
                "{table}\n    ⊘ NOTHING TO JOIN. ⚠ The four counts above are FOUR DIFFERENT \
                 FACTS: a `MISS` says this VAS does not bind that VA at all (§6 — not found, \
                 never denied); `in guest RAM` says leg 6 owns it; `ALREADY JOINED` says a \
                 previous doorbell did this work; and only a zero in ALL of them would mean \
                 the decode found nothing"
            ));
        }
        // ---- PHASE 2: WALK each candidate to its leaf, per-VAS, sessions dropped ------------
        //
        // ⚠ The session is scoped to the closure and released before any host verb, because
        // `join_one_fb_leaf` re-takes the plane lock at its step 3 and checks a worker out of
        // the isolate pool at its step 1. Holding a session across it is a deadlock, not a
        // slowdown.
        //
        // ★ Keyed by `leaf.phys` and de-duplicated HERE rather than inside the join: two
        // operands in one 64 KiB leaf are ONE join, and the second must not be attempted.
        let mut leaves: std::collections::BTreeMap<u64, kayfabe_rt::completion_watch::FbLeaf> =
            std::collections::BTreeMap::new();
        let mut walk_lines: Vec<String> = Vec::new();
        for &pva in &candidates {
            let (site, leaf) = plane.ce_session_with_root(
                &root,
                kayfabe_device::ceresolve::Demand::from_doorbell(),
                |ce| kayfabe_rt::ceutils::resolve_leaf_of(ce, pva),
            );
            match leaf {
                Some(l) => {
                    if leaves.insert(l.phys, l).is_some() {
                        walk_lines.push(format!(
                            "va=0x{pva:x} → leaf fb_phys=0x{:x} (SAME LEAF as an earlier \
                             operand — one join, not two)",
                            l.phys
                        ));
                    } else {
                        walk_lines.push(format!(
                            "va=0x{pva:x} → leaf va=0x{:x} len=0x{:x} fb_phys=0x{:x}",
                            l.va, l.len, l.phys
                        ));
                    }
                }
                // ⊘ `GuestRam` here contradicts the table read one phase up and is REPORTED
                // rather than reconciled: two resolutions of one fact disagreeing is a finding,
                // and preferring either reading is what §16.64 measured costing a week.
                None => walk_lines.push(format!(
                    "va=0x{pva:x} → ⊘ NO FRAMEBUFFER LEAF: {site:?}. ⚠ If this says `GuestRam` \
                     it DISAGREES with this pass's own table read above — do not reconcile it, \
                     read it as the two-sources finding it is"
                )),
            }
        }
        // ---- PHASE 3: JOIN, one leaf at a time, nothing held -------------------------------
        //
        // ★★★ THE ONLY STATEMENT THE `assert` ARM SKIPS. Everything above and everything
        // below runs identically on both arms, so the two logs are line-comparable and the
        // difference between them is this loop and nothing else.
        let isolate = kayfabe_isolate::IsolateId::new(f.proc.0, DOORBELL_TARGET_GPU);
        let mut joined = 0usize;
        let mut refused = 0usize;
        if let Some(exports) = exports.filter(|_| self.operand_join.joins()) {
            for (phys, leaf) in &leaves {
                let what = format!("CE-OPERAND(chan={} fb_phys=0x{phys:x})", f.chan.0);
                match join_one_fb_leaf(
                    &head,
                    &what,
                    &self.device,
                    &plane,
                    exports,
                    self.fb_join,
                    isolate,
                    pdb,
                    *leaf,
                ) {
                    Some(_) => joined += 1,
                    None => refused += 1,
                }
            }
        } else {
            eprintln!(
                "{head} ⊘ ARM IS `assert` — {} leaf/leaves were IDENTIFIED and NOT JOINED. No \
                 host verb was issued, nothing was mapped and nothing was bound. ★ The `#255` \
                 verdict below is therefore this rung's KNOWN-POSITIVE and must read FIRED",
                leaves.len()
            );
        }
        // ---- PHASE 4: THE RE-STATEMENT, and it is the FALSIFIER ------------------------------
        //
        // ★★★★★ Same pages, same table, same `Pdb` — re-read AFTER the joins, so the column
        // that changed can only have changed because of the replies above. ⊘ This is graded on
        // IDENTITY (`still_fabricated` is a LIST of VAs, not a count): `w281b`'s pre-registered
        // falsifier fired on a count while the thing counted was substituted underneath it, and
        // that is the third instance in three rungs.
        let mut still_fabricated: Vec<String> = Vec::new();
        let mut now_host_backed: Vec<String> = Vec::new();
        for &pva in &pages {
            match self
                .device
                .resolve(DOORBELL_TARGET_GPU, pdb, kayfabe_rt::GpuVa(pva))
            {
                Ok((b, _)) if b.is_guest_ram() => {}
                Ok((b, _)) => match b.host_va() {
                    Some(hva) => now_host_backed.push(format!(
                        "va=0x{pva:x}→host_va=0x{hva:x}{}",
                        if hva == pva {
                            ""
                        } else {
                            " ⚠ NOT-AT-THE-GUEST'S-OWN-VA"
                        }
                    )),
                    None => still_fabricated.push(format!(
                        "va=0x{pva:x}:{:?}@0x{:x}",
                        b.aperture(),
                        b.phys()
                    )),
                },
                Err(_) => {}
            }
        }
        Some(format!(
            "{table}\n    WALK: {}\n    JOINED {joined} leaf/leaves, {refused} REFUSED, over {} \
             distinct leaf/leaves\n    {}",
            walk_lines.join("\n          "),
            leaves.len(),
            Self::fake_fb_in_userspace_vas(f, &now_host_backed, &still_fabricated),
        ))
    }

    /// ⊘ **THE STUB, AND IT IS DELIBERATELY NOT SILENT** — `adopt_pending_channel_rings`'
    /// twin's reason, which that function's own docs record as a shape that cost a rung: an
    /// archive built without the feature prints nothing, exits 0, and every other signal says
    /// the boot happened.
    #[cfg(not(feature = "host-isolates"))]
    fn join_operand_fb_leaves(
        &self,
        token: u64,
        _facts: Option<&kayfabe_rt::device::CeChannelFacts>,
    ) -> Option<String> {
        if !self.operand_join.observes() {
            return None;
        }
        Some(format!(
            "OPERAND-JOIN token={token:#010x} host_isolates=NO ⇒ ⊘ THIS ARCHIVE CANNOT JOIN A \
             LEAF AT ALL. The arm was requested and this build has no isolate plane, so leg 7 \
             is a no-op — ⚠ do NOT grade a boot from this binary as `armed and nothing moved`"
        ))
    }

    /// ★★★★★ **LEG 8 — PUBLISH THE GUEST'S DECLARED ROWS INTO THE HOST VAS** (w290).
    ///
    /// # The measurement that commissioned it
    ///
    /// `[measured, boot w290cup2]` the faulting VA was owned by `GUEST-DESCRIBES` **and** by
    /// `TABLE-DESCRIBES` and by **neither** host page table: `HOST-PUBLISHED host_rows=4 of
    /// 16425`. Our shadow is right and hardware walks something else. ⇒ the wall is
    /// **publication, not population**, and `FAULT_PDE` rather than `FAULT_PTE` is that fact
    /// in the Xid's own vocabulary — with nothing published within a terabyte there is no
    /// page *directory* to miss a leaf in.
    ///
    /// # ⊘⊘ WHAT THIS PASS CANNOT DO, AND IT IS RM'S LIMIT RATHER THAN A CHOICE
    ///
    /// The brief that commissioned this said *"coalesce by RUN, not by row — publish
    /// extents"*. **The proven verb cannot be handed a run.** `plan_back_fb_leaf` refuses on
    /// three grounds before any host verb exists (`kayfabe-fwd/src/lib.rs:2328-2371`), and
    /// they pull in opposite directions:
    ///
    /// - `FbLeafGranularity` — *"RM places a fixed mapping in 64 KiB granules"* (`:2244-2247`).
    ///   A run **passes** this; the 4 KiB rows it is made of **cannot**.
    /// - `FbLeafExtent` — the request must be **exactly one table row**, start and length.
    ///   A run **fails** this whenever it spans more than one row.
    ///
    /// ⇒ Coalescing is what the first gate wants and what the third forbids. This pass
    /// therefore publishes **per row**, and [`kayfabe_rt::device::PublishCensus`] reports
    /// `not_granular` so *"how much of the table would run-coalescing have rescued"* is a
    /// **measured number rather than an estimate**. ⚠ Widening `FbLeafExtent` to accept a
    /// multi-row extent is a real change to the fwd plane's commit — one host object would
    /// have to write `host` into many rows, and the reclaim below frees per row — so it is
    /// deliberately **not** smuggled into an instrument rung.
    ///
    /// # ★★★ RECLAIM — ALREADY EXISTS, ON EVERY TEARDOWN ROUTE, AND HERE IS THE CITATION
    ///
    /// The owner's standing rule is that every pin needs an unpin. It is satisfied **by
    /// construction** rather than by new code, because this pass mints nothing new: a leaf
    /// bound by `adopt_joined_fb_leaf` is an ordinary `Binding` carrying a
    /// [`kayfabe_mmu::HostBacking`], and `Spine::stage_dropped_vases`
    /// (`kayfabe-core/src/gpu.rs:3229-3273`) walks `vas.table.iter()` and stages
    /// `unmap`-then-`free` for **every** binding whose `host()` is `Some`. It is reached from
    /// `Spine::vacate` (`gpu.rs:3645-3664`), *"THE ONE REMOVAL POINT"* (`gpu.rs:3622`), on
    /// all three routes: a VAS leaving the live set while the proc lives
    /// (`sync_proc_to_boundary`, `gpu.rs:3117`), clean proc death (`RmEvent::Free` of the
    /// client root ⇒ the component vanishes, `gpu.rs:3903`), and violent death
    /// (`retire_proc`, `gpu.rs:4181-4225`).
    ///
    /// ⊘⊘ **AND THE TRIGGER IS NOT WHAT THE BRIEF NAMED.** There is no UVM plane in this
    /// port to key an unpublish on: we emulate a **GPU**, so the guest's `nvidia-uvm` talks to
    /// the guest's `nvidia.ko` and `uvm_release` / `uvm_va_space_destroy` /
    /// `uvm_va_space_mm_shutdown` are **not observable events here at all** — they reach us
    /// only after the guest driver turns them into `RpcFunction::Free` (fn 10,
    /// `kayfabe-gsp/src/rpc.rs:261`) ⇒ `RmEvent::Free` ⇒ `Spine::refresh`. A `SIGKILL`ed guest
    /// process still gets there, because the guest's own `nvidia.ko` `close()` frees the
    /// client root. The genuinely kernel-guaranteed backstop is the **isolate process
    /// boundary** (§7.0), which is what `retire_proc`'s undrained queue relies on
    /// (`gpu.rs:1812-1817`).
    ///
    /// ⚠ **The residual gap, named rather than left to be found:** there is no per-leaf
    /// release short of VAS death. It is **pre-existing and shared with leg 7** — that leg's
    /// own doc already says *"the missing half is the trigger, not the mechanism … ⊘ Not
    /// wired this rung"* — and this pass widens the population it applies to. Its cost is
    /// **measured on the same line**: `RepointsPublished` / `UnbindsPublished`
    /// (`kayfabe-mmu/src/walker.rs:917-930`, `:956-972`) already print in the sweep's
    /// `by_kind`, so a boot says how often the guest tried to edit a row we had frozen.
    ///
    /// # Ordering
    ///
    /// Runs after the decode pass and the sweep have populated the table and after leg 7, and
    /// **before** `SharedDevice::doorbell` — the C's invariant, *"a mapping is always backed
    /// before the engine that uses it runs"*. ⊘ There is nothing to be lazy against: we
    /// emulate no fault buffer, so there is no fault to publish on demand from.
    ///
    /// ★★★★★ **w292 — `seen` is the CHANNEL THIS DOORBELL ROUTED TO, and it is taken as a
    /// parameter for legs 4-7's reason exactly.** It names the `(proc, pdb)` the ring about to
    /// be rung will be fetched through, which is the only thing that lets
    /// [`VasPublishArm::Drain`] scope its drain to *the VAS about to be doorbelled* instead of
    /// raising a budget across the board. ⊘ It is the facts this doorbell **already resolved**,
    /// never a second `ce_channel_facts` call — two resolutions of one fact can disagree, and
    /// this file has paid for that shape once already.
    #[cfg(feature = "host-isolates")]
    fn publish_vas_rows(
        &self,
        token: u64,
        seen: Option<&kayfabe_rt::device::CeChannelFacts>,
    ) -> Option<String> {
        if !self.vas_publish.observes() {
            return None;
        }
        let head = format!(
            "VAS-PUBLISH token={token:#010x} arm={}",
            self.vas_publish.as_str()
        );
        // ⚠ Same necessary-not-sufficient gate leg 7 states out loud, and refused rather than
        // downgraded: with `KAYFABE_FB_JOIN` off this pass could only map PRIVATE ANONYMOUS
        // pages — two memories under a name that says one.
        // ⊘ Enforced only on the arm that would publish; the census must still run on
        // `assert`, or the control loses the very number it exists to produce.
        if self.vas_publish.publishes() && !self.fb_join.armed() {
            return Some(format!(
                "{head} → ⊘ NOT ARMABLE: KAYFABE_FB_JOIN is `{}`. ⊘ Nothing was asked of the \
                 host",
                self.fb_join.as_str()
            ));
        }
        let Some(plane) = self.plane.upgrade() else {
            return Some(format!(
                "{head} → ⊘ NO PLANE. ⊘ Nothing was asked of the host"
            ));
        };
        let exports = match (self.exports.as_ref(), self.vas_publish.publishes()) {
            (Some(e), _) => Some(e),
            (None, false) => None,
            (None, true) => {
                return Some(format!(
                    "{head} → ⊘ NOT ARMABLE: exports_directory=false — no route from a backing \
                     token to a descriptor. ⊘ Nothing was asked of the host"
                ));
            }
        };
        // ★★★★★ w291 — THE BOUNDED PIN-RATE MEASUREMENT. Runs INSTEAD of the publication
        // pass, never beside it: they touch disjoint populations through different chains,
        // and one line reporting both would be the count that cannot see a substitution.
        // ★★★ On `pinrate` this REPLACES the publication pass; on `both` it PRECEDES it, so
        // one boot carries both halves and the line carries both sentences. ⊘ The two are
        // printed as separate clauses and never summed into one counter — they are different
        // chains over disjoint populations, and one number could not see the substitution.
        let pin_clause = if self.vas_publish.measures_pin_rate() {
            let line = self.measure_guest_ram_pin_rate(&head, seen);
            if !self.vas_publish.publishes() {
                return Some(line);
            }
            Some(line)
        } else {
            None
        };
        let started = std::time::Instant::now();
        let (mut published, mut refused, mut budget_hit) = (0usize, 0usize, false);
        let mut rows: Vec<String> = Vec::new();
        // ★★★★★ **w318 — THE DIRTY GATE'S HOST TERM, read ONCE for the whole pass.**
        //
        // `[measured 2026-08-14, w315 boot `full`]` every one of this pass's eight refusals is
        // *"that framebuffer range is already joined"* — an outcome of **host** state, which
        // `Vas::publish_epoch` cannot see. Without this term the gate would be an epoch of our
        // record gating a verb whose answer is not a function of our record alone, which is
        // the `a_second_source_of_truth_beside_a_complete_value` shape one plane over.
        //
        // ⊘ A count of ranges, not the ranges: it moves on every install and every release,
        // which is all a re-arm needs, and materialising the set per doorbell would put back a
        // slice of the very cost this removes.
        let gate = selected_dirty_gate(DIRTY_GATE_PUBLISH_ENV);
        let joined_now = plane.joined_fb_ranges().len();
        let (mut gate_fired, mut gate_skipped) = (0usize, 0usize);
        // ★★★★★ **w328 — THE DOORBELLED VAS, NAMED HERE TOO.**
        //
        // The drain half already derives it (`measure_guest_ram_pin_rate`'s `drain_target`);
        // this half never did, and that asymmetry is the whole of the breadth question. ⊘ It
        // is derived from the SAME `seen` facts through the SAME two fields, so the two
        // passes cannot come to disagree about which VAS a doorbell is about.
        let scope_target = seen.and_then(|f| f.vas_pdb.map(|p| (f.proc, p)));
        let scope_arm = publish_scope_arm();
        // ⚠⚠ **THE FALLBACK IS THE SAFETY PROPERTY, AND IT IS NOT AN OPTIMISATION.** Scoping
        // with NO target would publish NOTHING at all — strictly worse than master, and it
        // would present as a GPU fault, which is indistinguishable by symptom from the
        // pre-existing drain-truncation intermittent. ⇒ no target ⇒ full breadth, said out
        // loud in the line below rather than inferred from a count.
        // ⊘ AND THE SECOND REFUSAL: a target of `SYSTEM_PROC` is a target that is NEVER
        // ATTEMPTED (§12.26, `shim.rs`'s own `system` guard below). Scoping to it would leave
        // every publishable VAS unvisited while the line still read `scoped=true target=proc0`
        // — the favourable-looking absence this tree has paid for repeatedly.
        let scoped = publish_scope_scoped(scope_arm, scope_target);
        // ★★★★★ **w328 — THE BREADTH'S OWN COST, SPLIT AT THE SOURCE.** Not a fit and not a
        // residual: each VAS's own wall time is attributed to the bucket it belongs to as it
        // is spent. ⊘ `other_*` counts what the breadth DELIVERS beside what it COSTS,
        // because "2 529 ms of BQL" and "and it publishes nothing" are two different claims
        // and only the pair decides whether the breadth is vestigial.
        let (mut pub_target_us, mut pub_other_us) = (0u128, 0u128);
        let (mut pub_other_vases, mut pub_other_published, mut pub_other_refused) =
            (0usize, 0usize, 0usize);
        let (mut pub_target_published, mut pub_scoped_out) = (0usize, 0usize);
        for pid in self.device.live_pids() {
            for (gpu, pdb) in self.device.vas_keys(pid) {
                // ⊘ The isolate is keyed `(proc, gpu)`, so a `Vas` on another GPU has no
                // isolate to mint into here. Skipped and SAID, never silently dropped.
                if gpu != DOORBELL_TARGET_GPU {
                    rows.push(format!(
                        "[proc={} pdb=0x{:x} ⊘ SKIPPED gpu={} != doorbell target]",
                        pid.0, pdb.0, gpu.0
                    ));
                    continue;
                }
                // ★★★★★ **w318 — THE SKIP.** Everything below this point — the census walk
                // over every row of this `Vas`, and the join attempt over every candidate it
                // buckets — is a **pure function of** `(Vas::publish_epoch, joined_now)`. If
                // neither has moved since the last pass **that ran to completion**, re-running
                // it produces the identical census and the identical set of join outcomes.
                //
                // ⚠ Three refusals to skip, and each of them is a case that would otherwise
                // strand real work:
                // - the epoch is **unreadable** (`None`, the `Vas` is gone) ⇒ arm. UNMEASURED
                //   is not clean.
                // - this key has **no stamp** ⇒ arm. A key that appeared this doorbell has
                //   never been published.
                // - the last pass was **incomplete** (wall budget) ⇒ it left candidates
                //   unattempted, so no stamp was taken for it and it arms again below.
                let epoch_now = self.device.vas_publish_epoch(pid, gpu, pdb);
                if gate && let Some(epoch_now) = epoch_now {
                    let cached = self
                        .dirty
                        .published
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get(&(pid, gpu, pdb))
                        .filter(|s| s.epoch == epoch_now && s.joined == joined_now)
                        .cloned();
                    if let Some(s) = cached {
                        gate_skipped += 1;
                        rows.push(format!(
                            "[proc={} pdb=0x{:x} ⊘SKIPPED(w318 dirty gate: epoch={:?} joined={} \
                             unchanged since the last COMPLETED pass) REPLAY-OF-LAST-CENSUS \
                             {}]",
                            pid.0, pdb.0, epoch_now, joined_now, s.line
                        ));
                        continue;
                    }
                }
                // ★★★★★ **w328 — THE SCOPE SKIP.** Above the census, because the census walk
                // itself is the cost: `vas_publish_census` is O(rows of this Vas) and proc 0
                // alone holds 6787 of them. ⊘ Placed BELOW the w318 dirty gate deliberately —
                // the two are independent reasons not to walk a VAS, and collapsing them
                // would make one arm's tally speak for the other's.
                //
                // ⚠ NO STAMP IS TAKEN for a scoped-out VAS. A stamp says *"this census ran to
                // completion and here is what it found"*; taking one here would tell the next
                // doorbell that a VAS we never looked at is clean, which is the
                // publication-silently-never-performed shape this pass's own docs forbid.
                let is_target = scope_target == Some((pid, pdb));
                if scoped && !is_target {
                    pub_scoped_out += 1;
                    rows.push(format!(
                        "[proc={} pdb=0x{:x} ⊘SCOPED-OUT(w328 KAYFABE_PUBLISH_SCOPE=doorbelled: \
                         this is NOT the doorbelled VAS) ⊘ ITS CENSUS WAS NOT TAKEN — the rows \
                         below are UNMEASURED for this doorbell, ⊘ not zero, and NO STAMP WAS \
                         TAKEN]",
                        pid.0, pdb.0
                    ));
                    continue;
                }
                gate_fired += 1;
                let vas_t0 = std::time::Instant::now();
                let c = self
                    .device
                    .vas_publish_census(pid, gpu, pdb, VAS_PUBLISH_LEAF_BUDGET);
                let mut done = 0usize;
                let mut failed = 0usize;
                // ⊘⊘ **THE SYSTEM PROC CAN NEVER HOLD A PUBLICATION, AND THAT IS §12.26 —
                // SO IT IS NOT ATTEMPTED, NOT ATTEMPTED-AND-REFUSED.**
                //
                // `plan_back_fb_leaf` refuses `Gpu::SYSTEM_PROC` by name before any host verb
                // exists (`kayfabe-fwd/src/lib.rs:2318-2323`): the system proc's work is
                // forged precisely so it holds no host state whose reclaim has no defined
                // point, and a framebuffer object is host state.
                //
                // ⚠ `[measured, boot w290cup2]` proc 0 holds **6787 rows** across two VASes.
                // Handing them to the verb would issue 6787 doomed round trips and report
                // them as `refused=6787` — which reads exactly like RM exhaustion and is
                // nothing of the kind. ⇒ The refusal is stated HERE, once, as a property of
                // the proc, and the census still prints so the rows are visible rather than
                // absent.
                let system = pid == kayfabe_core::gpu::Gpu::SYSTEM_PROC;
                if self.vas_publish.publishes() && !system {
                    if let Some(exports) = exports {
                        let isolate = kayfabe_isolate::IsolateId::new(pid.0, gpu);
                        for &(va, len, phys) in &c.candidates {
                            if started.elapsed() > VAS_PUBLISH_WALL_BUDGET {
                                budget_hit = true;
                                break;
                            }
                            let what = format!("VAS-PUBLISH(proc={} pdb=0x{:x})", pid.0, pdb.0);
                            match join_one_fb_leaf(
                                &head,
                                &what,
                                &self.device,
                                &plane,
                                exports,
                                self.fb_join,
                                isolate,
                                pdb,
                                kayfabe_rt::completion_watch::FbLeaf { va, len, phys },
                            ) {
                                Some(_) => done += 1,
                                None => failed += 1,
                            }
                        }
                    }
                }
                published += done;
                refused += failed;
                // ★★★★★ **w328 — ATTRIBUTE THIS VAS's WALL TIME AS IT IS SPENT.**
                //
                // ⊘ The census WALK is inside the bracket as well as the joins, so the two
                // can be separated afterwards by correlating cost against `candidates` —
                // which is what settled the mechanism. `[measured w328, boot w328a1]` with
                // `candidates=0` and a table of **18 277 rows** a pass costs **632 µs**
                // (35 ns/row); with `candidates>0` and the same table it costs **52 094 µs**.
                // ⇒ **the walk is not the cost; ~6.4 ms per `join_one_fb_leaf` attempt is**,
                // and 328 of the boot's ~400 attempts are the same 8 already-joined ranges
                // re-offered 41 times. A bracket around the joins alone could not have shown
                // that, because it could not have priced the walk it excluded.
                let vas_us = vas_t0.elapsed().as_micros();
                if is_target {
                    pub_target_us += vas_us;
                    pub_target_published += done;
                } else {
                    pub_other_us += vas_us;
                    pub_other_vases += 1;
                    pub_other_published += done;
                    pub_other_refused += failed;
                }
                // ★★★★★ **w318 — THE STAMP, and the two conditions on taking it.**
                //
                // 1. **`!budget_hit`.** A pass that ran out of wall budget left candidates
                //    unattempted; stamping it clean would strand them until something else
                //    happened to move the epoch, which is a publication silently never
                //    performed — the exact failure mode the gate's own docs forbid.
                // 2. **The epoch is RE-READ here, after the joins.** A successful join binds
                //    into the table and therefore moves the epoch *during* this pass; stamping
                //    the pre-pass value would make the very next doorbell see a mismatch and
                //    re-run — a gate that can never go clean on a VAS that ever published.
                //    ⊘ Re-reading is also what keeps it CORRECT in the other direction: if
                //    anything else moved the epoch mid-pass, the value stamped is the one this
                //    census actually describes.
                //
                // ⚠⚠ **AND THE STAMP IS BUILT WHOLE BEFORE THE LOCK IS TAKEN** — every field,
                // including `plane.joined_fb_ranges()` and the `format!`. Written the obvious
                // way (as arguments to `insert`) the receiver is locked FIRST and the
                // arguments are evaluated underneath it, which would put **the plane's
                // rank-`Plane` lock beneath this unranked mutex**. `assert_lock_free` cannot
                // see an unranked lock — it masks only ranked ones — so that inversion would
                // pass every assertion in the tree and stall the register plane.
                // ⊘ `tests/tests/unranked_locks.rs` caught it; it is fixed here rather than
                // classified as safe, because the honest classification would have been *"a
                // ranked lock and an allocation run beneath it"*.
                if !budget_hit && let Some(after) = self.device.vas_publish_epoch(pid, gpu, pdb) {
                    let stamp = PublishStamp {
                        epoch: after,
                        joined: plane.joined_fb_ranges().len(),
                        line: format!(
                            "total={} already_host={} already_pinned={} guest_ram={} \
                             not_vidmem={} not_granular={} candidates={} published={done} \
                             refused={failed}",
                            c.total,
                            c.already_host,
                            c.already_pinned,
                            c.guest_ram,
                            c.not_vidmem,
                            c.not_granular,
                            c.candidates_total(),
                        ),
                    };
                    self.dirty
                        .published
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert((pid, gpu, pdb), stamp);
                }
                // ★★ EVERY row of the census, per VAS, with the bucket identity printed. ⊘ A
                // census whose buckets did not sum could report a comfortable zero for a class
                // it never reached, so `sum_ok` is a value and not a comment.
                rows.push(format!(
                    "[proc={} pdb=0x{:x}{} total={} already_host={} already_pinned={} \
                     guest_ram={} not_vidmem={} not_granular={}({} bytes) candidates={}({} \
                     bytes, capped={}) published={done} refused={failed} sum_ok={}]",
                    pid.0,
                    pdb.0,
                    if system {
                        " ⊘SYSTEM-PROC:NEVER-ATTEMPTED(§12.26 — it may hold no host state; \
                         candidates below are REAL and UNPUBLISHABLE, not refused)"
                    } else {
                        ""
                    },
                    c.total,
                    c.already_host,
                    c.already_pinned,
                    c.guest_ram,
                    c.not_vidmem,
                    c.not_granular,
                    c.not_granular_bytes,
                    c.candidates_total(),
                    c.candidate_bytes,
                    c.capped,
                    c.buckets_sum(),
                ));
            }
        }
        // ⊘ Tallied ONCE per doorbell, per VAS visited, and only after the loop: a gate that
        // is consulted N times on one doorbell must not report N doorbells.
        for _ in 0..gate_fired {
            self.dirty.tally(DirtyGate::PUBLISH, true);
        }
        for _ in 0..gate_skipped {
            self.dirty.tally(DirtyGate::PUBLISH, false);
        }
        // ★★★★★ **w328 — THE BREADTH LINE. Both halves, always, on every arm.**
        //
        // ⊘ `arm=` is printed even when unset, so `absent` means an OLD BINARY and never
        // `all` — this tree has paid for a knob whose setting lived only in the launcher's
        // environment. ⊘ `target=` prints `⊘NONE` rather than a plausible pair when this
        // doorbell resolved no channel facts, because `scoped` is FALSE in that case and a
        // reader must be able to see why.
        let w328 = format!(
            "arm={scope_arm} scoped={scoped} target={} scoped_out={pub_scoped_out} \
             target_us={pub_target_us} target_published={pub_target_published} \
             other_vases={pub_other_vases} other_us={pub_other_us} \
             other_published={pub_other_published} other_refused={pub_other_refused} \
             breadth_share={}",
            scope_target.map_or("⊘NONE (no channel facts ⇒ FULL BREADTH, by design)".to_string(), |(p, d)| format!(
                "proc={} pdb=0x{:x}",
                p.0, d.0
            )),
            (pub_other_us * 100).checked_div(pub_target_us + pub_other_us).map_or_else(
                || "⊘UNMEASURED (this pass spent no time in any VAS)".to_string(),
                |p| format!("{p}%"),
            ),
        );
        Some(format!(
            "{}{head} W328SCOPE[{w328}] gate={} this_doorbell[fired={gate_fired} \
             skipped={gate_skipped}] → \
             published={published} refused={refused} in {} ms{} over {} VAS row(s) {}",
            pin_clause
                .map(|l| format!("{l}\nkayfabe: "))
                .unwrap_or_default(),
            if gate { "on" } else { "off" },
            started.elapsed().as_millis(),
            if budget_hit {
                format!(
                    " ⚠⚠ WALL BUDGET {} ms EXHAUSTED — the remaining candidates were NOT \
                     attempted this doorbell; an unpublished row below is NOT thereby a refusal",
                    VAS_PUBLISH_WALL_BUDGET.as_millis()
                )
            } else {
                String::new()
            },
            rows.len(),
            rows.join(" "),
        ))
    }

    /// ★★★★★ **w291 — THE BOUNDED GUEST-RAM PIN-RATE MEASUREMENT.**
    ///
    /// # ⊘ WHAT IT REPLACES, SAID PLAINLY
    ///
    /// `guest_ram_publication_merge.md` costed option (2a) at **"~49 s per VAS"**. That
    /// number was an **EXTRAPOLATION**: leg 8's *framebuffer* rate (34 joins in 101 ms,
    /// ~3 ms each) multiplied by 16 328 guest-RAM rows. A framebuffer join and a guest-RAM
    /// pin are **different chains** — the join mints memory, copies the establishment bytes
    /// and re-points the guest's window; the pin describes pages the guest already owns — so
    /// the extrapolation had no right to speak for it. This measures the real thing.
    ///
    /// # What it does, and what it deliberately does not
    ///
    /// Pins up to [`VAS_PINRATE_ROWS`] guest-RAM rows of every non-system proc's `Vas`
    /// through the **existing** `pin_guest_ram` verb, timing each. ⊘ It writes **nothing**
    /// into `Binding::host`, adds no representation, touches no refcount, and puts no pointer
    /// between the two records. The pins land in `Vas::guest_ram_pins`, where that verb has
    /// always put them. **This is the measurement, not the merge.**
    ///
    /// ★★ **`degrade` is what lets a bounded sample speak about 16 328 rows.** It reports the
    /// mean of the last quarter against the mean of the first quarter. Flat (≈1.0) means the
    /// per-row cost is a constant and the bounded number extrapolates honestly; rising means
    /// it does not, and **that** is the finding rather than the headline rate.
    ///
    /// # ★★★★★ w292 — AND ON [`VasPublishArm::Drain`] IT IS NO LONGER ONLY A MEASUREMENT
    ///
    /// The VAS **this doorbell is about** — `(seen.proc, seen.vas_pdb)`, the address space the
    /// ring about to be rung is fetched through — is drained to empty rather than sampled,
    /// bounded by [`VAS_DRAIN_ROW_CAP`] and [`VAS_DRAIN_WALL_BUDGET`], both of which announce
    /// themselves. **Every other `Vas` keeps the bounded [`VAS_PINRATE_ROWS`] sample**, so the
    /// budget is raised for exactly one address space and `both` remains the control.
    ///
    /// ⊘ Four ways there is no drain, and they are **named exits rather than a silent
    /// sample**, because *"we drained and it was already empty"* and *"we never identified a
    /// target"* are opposite facts that would otherwise print the same line: the arm is not
    /// `drain`; this doorbell resolved no channel facts; the channel declared no `vas_pdb`; or
    /// the doorbelled proc is `SYSTEM_PROC`, whose refusal is a property of the proc (§12.26)
    /// and is **kept**.
    #[cfg(feature = "host-isolates")]
    fn measure_guest_ram_pin_rate(
        &self,
        head: &str,
        seen: Option<&kayfabe_rt::device::CeChannelFacts>,
    ) -> String {
        let Some(backing) = self.guest_ram_backing else {
            return format!(
                "{head} → ⊘ NO GUEST-RAM BACKING (no hypervisor layout to resolve a GPA \
                 against). ⊘ Nothing was asked of the host — this is UNMEASURED, not 0 ms"
            );
        };
        // ★★★★★ **w292 — WHICH VAS THIS DOORBELL IS ABOUT.** `None` on every arm but `drain`,
        // and `None` on `drain` itself whenever the facts cannot name one — which is a
        // DIFFERENT fact from "drained and found nothing", and is reported as one below.
        let drain_target = if self.vas_publish.drains_doorbelled_vas() {
            seen.and_then(|f| f.vas_pdb.map(|p| (f.proc, p)))
        } else {
            None
        };
        let drain_scope = match (self.vas_publish.drains_doorbelled_vas(), seen, drain_target) {
            (false, _, _) => format!(
                "⊘ NO DRAIN: arm=`{}` samples every VAS at {VAS_PINRATE_ROWS} rows/doorbell. \
                 An unpinned row below is UNREACHED, not refused",
                self.vas_publish.as_str()
            ),
            (true, None, _) => "⊘ DRAIN ARMED BUT NO TARGET: this doorbell resolved NO channel \
                                facts, so the VAS it is about has no name here. Every VAS got \
                                the bounded sample — ⚠ THIS LINE IS NOT A DRAIN"
                .to_string(),
            (true, Some(f), None) => format!(
                "⊘ DRAIN ARMED BUT NO TARGET: chan={} declared NO vas_pdb, so there is no \
                 address space to drain. Every VAS got the bounded sample — ⚠ THIS LINE IS \
                 NOT A DRAIN",
                f.chan.0
            ),
            (true, Some(_), Some((pid, pdb))) if pid == kayfabe_core::gpu::Gpu::SYSTEM_PROC => {
                format!(
                    "⊘⊘ THE DOORBELLED VAS IS SYSTEM_PROC (proc={} pdb=0x{:x}) — NOT DRAINED, \
                     AND NOT ATTEMPTED-AND-REFUSED. §12.26: `plan_pin_guest_ram` refuses proc 0 \
                     by name, and its 6787 rows would print as `refused=6144`, which reads \
                     exactly like RM exhaustion and is nothing of the kind",
                    pid.0, pdb.0
                )
            }
            (true, Some(_), Some((pid, pdb))) => format!(
                "★ DRAIN TARGET = proc={} pdb=0x{:x} (the VAS this doorbell's ring is fetched \
                 through); every OTHER VAS keeps the {VAS_PINRATE_ROWS}-row sample",
                pid.0, pdb.0
            ),
        };
        // ★★★★★ **w319 — THE CANDIDATE FIX, AND IT RUNS BEFORE THE BUDGETED DRAIN.**
        //
        // `[measured w319]` the drain below walks the doorbelled VAS in **ascending VA order**
        // (`IntervalMap` is a `BTreeMap<u64, _>`; `iter()` is documented "ascending start
        // order") and is cut off by a clock. ⇒ **whatever it drops, it drops from the TOP of
        // the address space** — and the guest's completion-semaphore page `0x2_0440f000` sits
        // near the top of the `0x2_004…–0x2_047ff000` span the drain covers. That is the whole
        // defect: a boot on the slow side of a 3 s budget stops below it, the engine is rung
        // anyway, and the host MMU reports `FAULT_PDE` on a page no directory was built for.
        //
        // ⊘ **Raising the budget is the WRONG fix even though it works.** The drain is held
        // under the QEMU BQL with every vCPU halted, and `[measured w314]` the surrounding
        // disposal already consumes 2.65–2.92 s of a 4 s `scrubberDestruct` budget. Buying
        // completeness with more BQL is spending headroom that is 73 % gone.
        //
        // ★ This instead makes the **few pages the engine is certain to touch** independent of
        // any budget: the completions the guest has itself DECLARED, de-duplicated to pages.
        // Measured population is **eight declarations at a 16-byte stride ⇒ ONE page**, so the
        // cost is one pin, not 13 313. It is the content of `pin_completion_guest_ram` —
        // deleted at w304 (`f20ab952`) on a "strict superset" argument that is true of the
        // candidate SET and false of the DELIVERY — restored as an ordering guarantee rather
        // than as a second mechanism, and `shim.rs:3851` records that pinning this page took
        // these exact Xids to ZERO at w266.
        //
        // ⊘ **DEFAULT OFF.** `KAYFABE_COMPLETION_PIN=on` arms it. Off ⇒ not one byte differs
        // from master, so the SAME BINARY carries both arms of the fix test and the only
        // variable between them is this flag.
        let mut sema_clause = String::from("⊘ off (KAYFABE_COMPLETION_PIN unset)");
        if completion_pin_armed() {
            let mut pinned_pages = 0usize;
            let mut refused_pages = 0usize;
            let mut skipped = 0usize;
            let mut named: Vec<String> = Vec::new();
            match drain_target {
                None => sema_clause = "⊘ ARMED BUT NO TARGET — this doorbell named no VAS, so \
                                       there is no address space to pin into. ⊘ UNREACHED, \
                                       not `nothing to do`"
                    .to_string(),
                Some((pid, _pdb)) if pid == kayfabe_core::gpu::Gpu::SYSTEM_PROC => {
                    sema_clause = "⊘ ARMED BUT TARGET IS SYSTEM_PROC — refused by name, \
                                   §12.26, exactly as the drain refuses it"
                        .to_string();
                }
                Some((pid, pdb)) => {
                    // ⊘ De-duplicate to PAGES first. Eight declarations at a 16-byte stride
                    // are ONE page, and pinning eight times would read as eight pins in every
                    // tally downstream.
                    let mut pages: std::collections::BTreeSet<(u64, u64)> =
                        std::collections::BTreeSet::new();
                    for (key, site) in &self.ce.watch.declared_sites() {
                        // ★ A pin lands in ONE proc's VA space. A completion another guest
                        // process declared is not this channel's to place. Counted, never
                        // dropped silently.
                        if key.proc != pid {
                            skipped += 1;
                            continue;
                        }
                        let kayfabe_rt::completion_watch::Site::GuestRam { gpa } = site else {
                            skipped += 1;
                            continue;
                        };
                        let mask = Self::RING_PIN_BYTES - 1;
                        pages.insert((key.va & !mask, gpa & !mask));
                    }
                    for (va, gpa) in &pages {
                        let len = Self::RING_PIN_BYTES;
                        let resolved = {
                            let held = self.ce.vmm.lock().unwrap_or_else(|e| e.into_inner());
                            held.as_ref().map(|vmm| vmm.resolve_guest_ram(backing, *gpa, len))
                        };
                        let Some(Ok(run)) = resolved else {
                            named.push(format!("[va=0x{va:x} ⊘UNRESOLVED-BY-VMM]"));
                            continue;
                        };
                        let grant = kayfabe_isolate::GuestRamGrant::originated_by_the_vmm(
                            run.file_offset,
                            len,
                            kayfabe_vmm::Prot::ReadWrite,
                        );
                        match self.device.pin_guest_ram(
                            DOORBELL_TARGET_GPU,
                            pdb,
                            kayfabe_rt::GpuVa(*va),
                            grant,
                        ) {
                            Ok(p) => {
                                pinned_pages += 1;
                                named.push(format!(
                                    "[va=0x{va:x} gpa=0x{gpa:x} host_va=0x{:x} \
                                     placed_as_asked={} {}]",
                                    p.host_va,
                                    p.host_va == *va,
                                    if p.already { "replay" } else { "fresh" },
                                ));
                            }
                            Err(e) => {
                                refused_pages += 1;
                                named.push(format!("[va=0x{va:x} ⊘REFUSED `{e:?}`]"));
                            }
                        }
                    }
                    sema_clause = format!(
                        "★ ARMED proc={} pdb=0x{:x} declared_pages={} pinned={pinned_pages} \
                         refused={refused_pages} skipped={skipped} {}",
                        pid.0,
                        pdb.0,
                        pages.len(),
                        named.join(" ")
                    );
                }
            }
        }
        let mut rows: Vec<String> = Vec::new();
        let (mut total_pins, mut total_us) = (0usize, 0u128);
        let mut refused = 0usize;
        let mut degrade = String::from("n/a");
        // ★★ THE DRAIN'S OWN FOUR FACTS, kept apart from the totals: whether the target VAS was
        // REACHED at all, what it cost ON THE DOORBELL PATH, and — separately — which of the
        // two bounds stopped it. ⊘ `visited=false` beside `pinned=0` is "we never got there";
        // `visited=true` beside `pinned=0` is "there was nothing left to pin". Collapsing them
        // is the absent-artefact-reads-as-favourable class.
        let (mut drain_visited, mut drain_asked, mut drain_pinned) = (false, 0usize, 0usize);
        let (mut drain_refused, mut drain_ms) = (0usize, 0u128);
        let (mut drain_cap_hit, mut drain_budget_hit) = (false, false);
        // ★★★★★ w321 — the two decomposition rows, both `⊘UNMEASURED` until a drain runs.
        let mut drain_census = String::from("⊘ NO DRAIN — the contiguity census is UNMEASURED");
        let mut drain_ipc = String::from("⊘ NO DRAIN — the IPC bracket is UNMEASURED");
        let mut drain_batch = String::from("⊘ NO DRAIN — the batch accounting is UNMEASURED");
        // ★★★★★ **w328 — THE SAME BREADTH QUESTION ON THIS PASS.** The doorbelled VAS is
        // DRAINED here and every other one is SAMPLED at 256 rows; the question is what the
        // sample costs and what it delivers. ⊘ Same fallback rule as the publication half: no
        // target ⇒ full breadth, because scoping to a VAS we cannot name is scoping to none.
        let scope_arm = publish_scope_arm();
        // ⊘ Same two refusals as the publication half, and the SYSTEM_PROC one is load-bearing
        // here too: this loop `continue`s past proc 0 unconditionally, so scoping to it would
        // skip every VAS and pin nothing at all.
        let scoped = publish_scope_scoped(scope_arm, drain_target);
        let (mut pin_other_us, mut pin_other_vases, mut pin_other_pinned) = (0u128, 0usize, 0usize);
        let mut pin_scoped_out = 0usize;
        for pid in self.device.live_pids() {
            // ⊘ Same §12.26 guard the publication pass carries, and for the same reason:
            // `plan_pin_guest_ram` refuses `SYSTEM_PROC` too, so attempting proc 0 would
            // print hundreds of refusals that read exactly like RM exhaustion.
            // ⊘ w292: this guard runs BEFORE the drain target is honoured, deliberately — the
            // refusal is a property of the proc and a budget change may not relax it. The
            // `drain_scope` line above says so when the target IS proc 0.
            if pid == kayfabe_core::gpu::Gpu::SYSTEM_PROC {
                continue;
            }
            for (gpu, pdb) in self.device.vas_keys(pid) {
                if gpu != DOORBELL_TARGET_GPU {
                    continue;
                }
                // ★★★★★ **w292 — THE ONE SCOPED BUDGET CHANGE, AND IT IS THIS PREDICATE.**
                let doorbelled = drain_target == Some((pid, pdb));
                // ★★★★★ **w328 — THE SCOPE SKIP, above `vas_guest_ram_rows`.** That call is
                // what materialises the candidate list, so skipping below it would save the
                // pins and keep the walk. ⊘ The drained VAS is never scoped out: `scoped`
                // implies `drain_target.is_some()`, so exactly one VAS survives the filter.
                if scoped && !doorbelled {
                    pin_scoped_out += 1;
                    continue;
                }
                let cap = if doorbelled {
                    // ★ w319: `vas_drain_row_limit()` IS `VAS_DRAIN_ROW_CAP` unless the
                    // instrument env var is set, so master's behaviour is unchanged.
                    vas_drain_row_limit()
                } else {
                    VAS_PINRATE_ROWS
                };
                let candidates = self.device.vas_guest_ram_rows(pid, gpu, pdb, cap);
                // ⊘ An empty candidate list is skipped on a SAMPLED VAS (it says nothing) and
                // PRINTED on the drained one (it says the drain found the table already
                // complete, which is the whole question this rung asks).
                if candidates.is_empty() && !doorbelled {
                    continue;
                }
                if doorbelled {
                    drain_visited = true;
                    drain_asked = candidates.len();
                    drain_cap_hit = candidates.len() >= cap;
                    // ★★★★★ **w321 — THE CONTIGUITY CENSUS, TAKEN BEFORE A SINGLE PIN.**
                    // O(n) over the rows we are about to walk, and it is the number that
                    // BOUNDS a coalescing fix before one is built. See `drain_contiguity`.
                    drain_census = drain_contiguity(&candidates);
                }
                let vas_started = std::time::Instant::now();
                // ★★★★★ **w321 — THE PARENT-SIDE HALF OF THE DECOMPOSITION.**
                // Read here and again after the loop; the difference is `(calls, µs)` this
                // drain spent blocked in the isolate IPC. Subtract it from `DRAIN_MS` and
                // what is left is OUR OWN cost (route locks, `resolve_guest_ram`, commit).
                // ⊘ Thread-local and monotonic — see `ipc_totals`'s own doc.
                let ipc_before = kayfabe_isolate_host::isolate::ipc_totals();
                let mut vas_refused = 0usize;
                let mut budget_hit = false;
                let mut last_va: Option<u64> = None;
                let mut each_us: Vec<u128> = Vec::new();
                let mut named: Vec<String> = Vec::new();
                // ★★★★★ **w321 — THE COALESCER.** `chunks_for` is the identity on every arm
                // but `KAYFABE_DRAIN_BATCH=coalesce`, where it merges rows that abut in BOTH
                // `va` and `gpa` into one chain, split at 2 MiB. See its own doc for why the
                // 2 MiB is the C's number and not a guess, and `drain_contiguity` for the
                // measurement that says what it can buy.
                let chunks = if doorbelled {
                    chunks_for(&candidates)
                } else {
                    candidates.iter().map(|&r| DrainChunk::one(r)).collect()
                };
                // ⊘ ROWS and CHAINS are counted separately and neither is derived from the
                // other. `pinned == asked` is w319's grading invariant and it is stated in
                // ROWS; `chains` is what the host was actually asked, and the whole fix is
                // the ratio between them. Collapsing them would make the fix invisible in
                // exactly the line that grades it.
                let mut rows_pinned = 0usize;
                let mut rows_refused = 0usize;
                let mut fallback_chains = 0usize;
                let mut chunk_split = 0usize;
                for chunk in &chunks {
                    // ⚠ THE WALL BOUND, and it is checked only on the drained VAS: the sampled
                    // ones are bounded by their row count already, and adding a clock to them
                    // would change the control.
                    if doorbelled && vas_started.elapsed() > vas_drain_wall_budget() {
                        budget_hit = true;
                        drain_budget_hit = true;
                        break;
                    }
                    let (va, gpa, len) = (chunk.va, chunk.gpa, chunk.len);
                    // The file offset comes from the HYPERVISOR's own stated layout, exactly
                    // as legs 4-6 derive it. ⊘ A row the VMM will not resolve is NOT a pin
                    // failure and is not timed — it never reached the verb.
                    let resolved = {
                        let held = self.ce.vmm.lock().unwrap_or_else(|e| e.into_inner());
                        held.as_ref()
                            .map(|vmm| vmm.resolve_guest_ram(backing, gpa, len))
                    };
                    let Some(Ok(run)) = resolved else {
                        // ★★ w321 — A COALESCED CHUNK CAN BE REFUSED FOR A REASON ITS ROWS
                        // WOULD NOT BE: `StraddlesRuns` says the chunk left the hypervisor's
                        // stated run, which is a property of the MERGE and not of any row in
                        // it. ⊘ So the chunk falls back to its own rows rather than being
                        // dropped — the alternative loses up to 512 rows for a boundary the
                        // coalescer invented.
                        if chunk.rows > 1 {
                            chunk_split += 1;
                            let (r_ok, r_no, chains, us_sum) = self.pin_rows_one_by_one(
                                backing,
                                pdb,
                                &candidates[chunk.first_row..chunk.first_row + chunk.rows],
                                &mut named,
                            );
                            rows_pinned += r_ok;
                            rows_refused += r_no;
                            refused += r_no;
                            vas_refused += r_no;
                            fallback_chains += chains;
                            total_pins += chains;
                            total_us += us_sum;
                            if r_ok > 0 {
                                last_va = Some(chunk.last_row_va);
                            }
                        } else {
                            named.push(format!("[va=0x{va:x} ⊘UNRESOLVED-BY-VMM]"));
                        }
                        continue;
                    };
                    let grant = kayfabe_isolate::GuestRamGrant::originated_by_the_vmm(
                        run.file_offset,
                        len,
                        kayfabe_vmm::Prot::ReadWrite,
                    );
                    let t0 = std::time::Instant::now();
                    let r = self.device.pin_guest_ram(
                        DOORBELL_TARGET_GPU,
                        pdb,
                        kayfabe_rt::GpuVa(va),
                        grant,
                    );
                    let us = t0.elapsed().as_micros();
                    match r {
                        Ok(p) => {
                            each_us.push(us);
                            total_pins += 1;
                            total_us += us;
                            rows_pinned += chunk.rows;
                            last_va = Some(chunk.last_row_va);
                            if named.len() < 4 {
                                named.push(format!(
                                    "[va=0x{va:x}+0x{len:x} rows={} gpa=0x{gpa:x} \
                                     host_va=0x{:x} placed_as_asked={} {}{us}us]",
                                    chunk.rows,
                                    p.host_va,
                                    p.host_va == va,
                                    if p.already { "replay " } else { "fresh " },
                                ));
                            }
                        }
                        // ⊘ REFUSED BY NAME, never a tally: `0x51 NV_ERR_NO_MEMORY` is
                        // collision-or-exhaustion and cannot be told apart, so the name is
                        // the only thing that distinguishes "we asked twice" from "the host
                        // is full".
                        //
                        // ★★ w321 — AND A MERGED CHUNK FALLS BACK TO ITS ROWS. A refusal of
                        // a 2 MiB chain would otherwise cost 512 rows for a fault that may
                        // belong to one of them, which is strictly worse than the truncation
                        // this rung exists to remove.
                        Err(e) => {
                            if chunk.rows > 1 {
                                chunk_split += 1;
                                named.push(format!(
                                    "[va=0x{va:x}+0x{len:x} ⊘CHUNK-REFUSED `{e:?}` \
                                     {us}us → FALLING BACK TO {} ROWS]",
                                    chunk.rows
                                ));
                                let (r_ok, r_no, chains, us_sum) = self.pin_rows_one_by_one(
                                    backing,
                                    pdb,
                                    &candidates
                                        [chunk.first_row..chunk.first_row + chunk.rows],
                                    &mut named,
                                );
                                rows_pinned += r_ok;
                                rows_refused += r_no;
                                refused += r_no;
                                vas_refused += r_no;
                                fallback_chains += chains;
                                total_pins += chains;
                                total_us += us_sum;
                                if r_ok > 0 {
                                    last_va = Some(chunk.last_row_va);
                                }
                            } else {
                                refused += 1;
                                vas_refused += 1;
                                rows_refused += 1;
                                if named.len() < 8 {
                                    named
                                        .push(format!("[va=0x{va:x} ⊘REFUSED `{e:?}` {us}us]"));
                                }
                            }
                        }
                    }
                }
                // ⊘ w328 — read in µs as well as ms. A sampled VAS costs single-digit ms, so a
                // ms-granular sum over ~4 of them rounds toward zero and would report the
                // breadth as free.
                let vas_us = vas_started.elapsed().as_micros();
                let vas_ms = vas_started.elapsed().as_millis();
                // ★★ FLAT OR DEGRADING — the property that decides whether 256 rows may
                // speak for 16 328. ⊘ w292: computed per VAS and printed IN the VAS's own row,
                // because a single file-scope `degrade` is the last VAS's answer wearing the
                // whole pass's name.
                let vas_degrade = if each_us.len() >= 8 {
                    let q = each_us.len() / 4;
                    let first: u128 = each_us[..q].iter().sum::<u128>() / q as u128;
                    let last: u128 = each_us[each_us.len() - q..].iter().sum::<u128>() / q as u128;
                    format!("first_q={first}us last_q={last}us")
                } else {
                    format!("n/a — only {} timed pin(s), need 8", each_us.len())
                };
                if doorbelled {
                    // ★★★★★ **w321 — `pinned` IS IN ROWS, AND THAT IS DELIBERATE.**
                    // w319's grading invariant is `pinned == asked` and both terms are ROW
                    // counts. A coalescing fix that reported CHAINS here would make its own
                    // success read as a 11× regression in the one line every lane grades on.
                    // ⊘ The chain count is not lost — it is `W321BATCH`'s `chains=`.
                    drain_pinned = rows_pinned;
                    drain_refused = vas_refused;
                    drain_ms = vas_ms;
                    degrade = vas_degrade.clone();
                    let chains = each_us.len() + fallback_chains;
                    drain_batch = format!(
                        "arm={} chunks={} chains={chains} rows_pinned={rows_pinned} \
                         rows_refused={rows_refused} fallback_chunks={chunk_split} \
                         fallback_chains={fallback_chains} rows_per_chain={}.{:02}",
                        drain_batch_arm(),
                        chunks.len(),
                        if chains == 0 { 0 } else { rows_pinned / chains },
                        if chains == 0 {
                            0
                        } else {
                            (rows_pinned * 100 / chains) % 100
                        },
                    );
                    // ★★★★★ **w321 — CLOSE THE PARENT-SIDE BRACKET AND SUBTRACT.**
                    let ipc_after = kayfabe_isolate_host::isolate::ipc_totals();
                    let calls = ipc_after.0.saturating_sub(ipc_before.0);
                    let us = ipc_after.1.saturating_sub(ipc_before.1);
                    // ⊘ Per CHAIN, not per row: the chain is what crossed the socket, and on
                    // the coalescing arm a per-row figure would divide one round trip by the
                    // rows it happened to cover and report a transport cost that nothing paid.
                    // ★ The per-ROW figure is beside it, because that is what multiplies out
                    // to the drain's cost.
                    drain_ipc = if chains == 0 {
                        format!(
                            "⊘ NO CHAIN WAS ISSUED — ipc_calls={calls} ipc_us={us}, and the \
                             split is UNMEASURED, ⊘ not 0"
                        )
                    } else {
                        let c = chains as u128;
                        let r = std::cmp::max(rows_pinned + vas_refused, 1) as u128;
                        let own = u128::from(vas_ms) * 1000;
                        format!(
                            "ipc_calls={calls} ({} /chain) ipc_us={us} ({} us/chain, \
                             {} us/row) drain_us={own} ours_us={} ipc_share={}%",
                            calls as u128 / c,
                            u128::from(us) / c,
                            u128::from(us) / r,
                            own.saturating_sub(u128::from(us)),
                            if own == 0 { 0 } else { u128::from(us) * 100 / own },
                        )
                    };
                } else {
                    // ★★★★★ **w328 — WHAT THE SAMPLED (non-doorbelled) VASes COST AND DELIVER.**
                    pin_other_us += vas_us;
                    pin_other_vases += 1;
                    pin_other_pinned += each_us.len();
                    if degrade == "n/a" {
                        degrade = vas_degrade.clone();
                    }
                }
                rows.push(format!(
                    "[proc={} pdb=0x{:x} {} asked={} pinned={} refused={} in {vas_ms} ms \
                     last_pinned_va={} degrade[{vas_degrade}]{}{} {}]",
                    pid.0,
                    pdb.0,
                    if doorbelled {
                        "★DRAINED(this doorbell's VAS)"
                    } else {
                        "SAMPLED(bounded — an unpinned row here is UNREACHED, not refused)"
                    },
                    candidates.len(),
                    if doorbelled { rows_pinned } else { each_us.len() },
                    vas_refused,
                    last_va.map_or("⊘NONE".to_string(), |v| format!("0x{v:x}")),
                    if budget_hit {
                        format!(
                            " ⚠⚠ DRAIN WALL BUDGET {} ms EXHAUSTED — THE DRAIN IS INCOMPLETE; \
                             the rows after this point were NOT attempted and are NOT refused",
                            vas_drain_wall_budget().as_millis()
                        )
                    } else {
                        String::new()
                    },
                    if doorbelled && candidates.len() >= vas_drain_row_limit() {
                        format!(
                            " ⚠⚠ DRAIN ROW CAP {} HIT — THE DRAIN IS \
                             INCOMPLETE by construction",
                            vas_drain_row_limit()
                        )
                    } else {
                        String::new()
                    },
                    named.join(" "),
                ));
            }
        }
        let per_row = if total_pins == 0 {
            "⊘ NO ROW WAS PINNED — the per-row rate is UNMEASURED, not 0".to_string()
        } else {
            let us = total_us / total_pins as u128;
            format!(
                "{us} us/row ⇒ 16328 rows would cost {} ms IF FLAT",
                us * 16328 / 1000
            )
        };
        // ★★★★★ **THE DRAIN'S OWN COST ON THE DOORBELL PATH — FOUR FACTS, NEVER A WORD.**
        // `visited` / `asked` / `pinned` / `ms`, plus which bound stopped it, because
        // "complete" and "we ran out" are the difference between a result and a non-result.
        let drain_clause = if !self.vas_publish.drains_doorbelled_vas() {
            "⊘ NOT ARMED".to_string()
        } else if !drain_visited {
            format!(
                "⚠⚠ TARGET NEVER VISITED — the drain did NOT run. The VAS named above was not \
                 among this device's live (proc, pdb) keys on the doorbell target GPU, so \
                 `pinned=0` here is UNREACHED and NOT `already complete`. [{drain_scope}]"
            )
        } else {
            format!(
                "visited=true asked={drain_asked} pinned={drain_pinned} \
                 refused={drain_refused} DRAIN_MS={drain_ms} \
                 W319KNOB[budget_ms={} row_limit={}] complete={} {}{}",
                // ★★★ w319 — THE ARM ANNOUNCES ITSELF, in the same line as the number it
                // moves. ⊘ A knob whose setting is only in the launcher's environment is a
                // number nobody can attribute a log to six weeks from now, and this tree has
                // paid for exactly that ("anchor every metric"). Printed on EVERY boot,
                // including the default one, so `absent` means an OLD BINARY and never `3000`.
                vas_drain_wall_budget().as_millis(),
                vas_drain_row_limit(),
                // ★ COMPLETE means: every row the table offered was attempted, and neither
                // bound cut it short. It is the invariant this rung exists to establish —
                // "a mapping is always backed before the engine that uses it runs".
                !drain_cap_hit && !drain_budget_hit,
                if drain_cap_hit {
                    "⚠⚠ ROW CAP HIT "
                } else {
                    ""
                },
                if drain_budget_hit {
                    "⚠⚠ WALL BUDGET HIT "
                } else {
                    ""
                },
            )
        };
        format!(
            "{head} PINRATE(w291 rate; ★w292 DRAIN — on arm `drain` the doorbelled VAS's rows \
             ARE merged into Binding::host by `commit_pin_guest_ram`, so this is no longer \
             measurement-only) → pinned={total_pins} refused={refused} in {} ms, \
             per_row={per_row}, degrade[{degrade}] SEMAPIN[{sema_clause}] \
             DRAIN[{drain_clause}] SCOPE[{drain_scope}] \
             W321CENSUS[{drain_census}] W321IPC[{drain_ipc}] W321BATCH[{drain_batch}] \
             W328PIN[arm={scope_arm} scoped={scoped} scoped_out={pin_scoped_out} \
             other_vases={pin_other_vases} other_us={pin_other_us} \
             other_pinned={pin_other_pinned} drain_ms={drain_ms}] \
             over {} VAS row(s) {}",
            total_us / 1000,
            rows.len(),
            if rows.is_empty() {
                "⊘ NO CANDIDATE ROW IN ANY NON-SYSTEM VAS — UNMEASURED, not zero".to_string()
            } else {
                rows.join(" ")
            },
        )
    }

    /// ★★★★★ **w321 — THE COALESCER'S FALLBACK: pin a refused chunk's rows ONE AT A TIME.**
    ///
    /// ⊘ **It exists because a merged chunk can be refused for a reason none of its rows
    /// would be.** `StraddlesRuns` is a property of the MERGE — the chunk left the
    /// hypervisor's stated run — and `GuestRamPinOverlaps` can be one too. Dropping the chunk
    /// on such a refusal would lose up to 512 rows for a boundary this file invented, which
    /// is strictly worse than the truncation w321 exists to remove.
    ///
    /// ⇒ The fallback is **exactly master's loop**, over exactly the rows the table stated,
    /// so the worst case of the coalescing arm is master's cost for that chunk **plus one
    /// wasted chain**, and never a missing mapping.
    ///
    /// Returns `(rows_pinned, rows_refused, chains_issued, total_us)`.
    #[cfg(feature = "host-isolates")]
    fn pin_rows_one_by_one(
        &self,
        backing: kayfabe_vmm_qemu::layout::BackingId,
        pdb: kayfabe_rt::Pdb,
        rows: &[(u64, u64, u64)],
        named: &mut Vec<String>,
    ) -> (usize, usize, usize, u128) {
        let (mut ok, mut no, mut chains, mut us_sum) = (0usize, 0usize, 0usize, 0u128);
        for &(va, gpa, len) in rows {
            let resolved = {
                let held = self.ce.vmm.lock().unwrap_or_else(|e| e.into_inner());
                held.as_ref()
                    .map(|vmm| vmm.resolve_guest_ram(backing, gpa, len))
            };
            let Some(Ok(run)) = resolved else {
                if named.len() < 12 {
                    named.push(format!("[va=0x{va:x} ⊘UNRESOLVED-BY-VMM (fallback)]"));
                }
                continue;
            };
            let grant = kayfabe_isolate::GuestRamGrant::originated_by_the_vmm(
                run.file_offset,
                len,
                kayfabe_vmm::Prot::ReadWrite,
            );
            let t0 = std::time::Instant::now();
            let r =
                self.device
                    .pin_guest_ram(DOORBELL_TARGET_GPU, pdb, kayfabe_rt::GpuVa(va), grant);
            us_sum += t0.elapsed().as_micros();
            chains += 1;
            match r {
                Ok(_) => ok += 1,
                Err(e) => {
                    no += 1;
                    if named.len() < 12 {
                        named.push(format!("[va=0x{va:x} ⊘REFUSED `{e:?}` (fallback)]"));
                    }
                }
            }
        }
        (ok, no, chains, us_sum)
    }

    /// ⊘ **THE STUB, AND IT IS DELIBERATELY NOT SILENT** — `join_operand_fb_leaves`' twin's
    /// reason: an archive built without the feature prints nothing, exits 0, and every other
    /// signal says the boot happened.
    #[cfg(not(feature = "host-isolates"))]
    fn publish_vas_rows(
        &self,
        token: u64,
        _seen: Option<&kayfabe_rt::device::CeChannelFacts>,
    ) -> Option<String> {
        if !self.vas_publish.observes() {
            return None;
        }
        Some(format!(
            "VAS-PUBLISH token={token:#010x} host_isolates=NO ⇒ ⊘ THIS ARCHIVE CANNOT PUBLISH A \
             ROW AT ALL. The arm was requested and this build has no isolate plane — ⚠ do NOT \
             grade a boot from this binary as `armed and nothing moved`"
        ))
    }

    /// ★★★★★ **#255 — THE OWNER'S ASSERTION: fake framebuffer must never be what a guest
    /// USERSPACE channel's engine is pointed at.**
    ///
    /// > *"no fake framebuffer at a real GPU VA of an isolate except the scratchpad"* —
    /// > owner, 2026-08-11, and `kayfabe_mmu::RegionKind::FakeFramebuffer`'s own text:
    /// > *"Ruling 2 scopes what this kind is for: **guest-KERNEL channels we emulate** …
    /// > A guest **userspace** mapping landing here is the execution blocker, not the design."*
    ///
    /// # ⊘⊘ WHY IT REPORTS IN EVERY BUILD AND PANICS IN NONE OF THE ONES WE SHIP
    ///
    /// The owner's constraint is explicit: **never asserted in production.** The guest can
    /// drive this condition, and panicking on guest-reachable state hands it a DoS. But a
    /// `#[cfg(debug_assertions)]` body with an **empty sibling** is exactly the shape that
    /// makes *"the check never ran"* indistinguishable from *"the check ran and found
    /// nothing"* — measured, at `shim.rs`'s own `#[cfg(not(host-isolates))]` twin, and it
    /// cost a rung. ⊘ And the bench builds **`--release`** (`scripts/build_qom_shim.sh:37`),
    /// so a debug-only instrument would never execute on the only machine that can run it.
    ///
    /// ⇒ **The verdict is a sentence in every build, and it names which build it is.** The
    /// `debug_assertions` arm adds a panic on top of the same sentence; it does not replace it.
    ///
    /// # ★★★ It has a GUARANTEED KNOWN-POSITIVE, today
    ///
    /// `[measured 2026-08-12, w281b_clientsweep]` the raw CE client's two operands resolve
    /// `Vidmem@0x10000` and `Vidmem@0x20000` with no host object — so on the **`off`** arm this
    /// must print `FIRED`, naming both VAs. ⊘ A zero on that arm means the instrument did not
    /// run, not that the condition is absent: `a census ZERO needs a KNOWN-POSITIVE`.
    #[cfg(feature = "host-isolates")]
    fn fake_fb_in_userspace_vas(
        f: &kayfabe_rt::device::CeChannelFacts,
        now_host_backed: &[String],
        still_fabricated: &[String],
    ) -> String {
        // ★ `ProcId(0)` is `kayfabe_core::gpu::Gpu::SYSTEM_PROC` — the forged system plane,
        // which holds no host state by construction. Every other proc is a **guest process**,
        // and its channels are the userspace population ruling 2 scopes this to.
        let userspace = f.proc.0 != 0;
        let build = if cfg!(debug_assertions) {
            "debug (this sentence is followed by a PANIC when it fires)"
        } else {
            "release (REPORTS ONLY — the owner's ruling: a guest can drive this, so \
             panicking on it is a DoS we hand them)"
        };
        let verdict = if !userspace {
            format!(
                "⊘ NOT ASKED: proc={} is the SYSTEM plane, and ruling 2 scopes kind-2 \
                 framebuffer to the guest-KERNEL channels we emulate. This assertion is about \
                 guest USERSPACE VASes only",
                f.proc.0
            )
        } else if still_fabricated.is_empty() {
            format!(
                "★★★★★ QUIET: not one operand of this guest-userspace channel resolves to \
                 unpublished emulated framebuffer. {} operand page(s) now carry a host object \
                 [{}]. ⊘ QUIET IS NOT PROOF THE ENGINE RAN — it is proof of what the table \
                 says, and only an Xid or a completion says the other thing",
                now_host_backed.len(),
                now_host_backed.join(" ")
            )
        } else {
            format!(
                "★★★ FIRED — {} operand page(s) of a GUEST USERSPACE channel (proc={} chan={}) \
                 resolve to EMULATED FRAMEBUFFER with no host object behind them, which is the \
                 owner's forbidden state and is what routes this copy to CeExecutor::Ours: [{}] \
                 (⊘ graded by ADDRESS, never by count — a count cannot see a substitution)",
                still_fabricated.len(),
                f.proc.0,
                f.chan.0,
                still_fabricated.join(" ")
            )
        };
        let line = format!("★★★★★ #255 FAKE-FB-IN-USERSPACE-VAS build={build} → {verdict}");
        // ⊘ The panic is ADDITIVE and is never on the shipped path. `debug_assert!` rather
        // than `assert!` so the shape cannot be mistaken for a production check by a reader,
        // and the same sentence is already printed either way — so the release build is
        // distinguishable from a positive signal, which is the trap this shape exists to avoid.
        debug_assert!(still_fabricated.is_empty() || !userspace, "{line}");
        line
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
        // ★★★★★ **w318 — THE DIRTY GATE, and it sits HERE rather than at the decode.**
        //
        // This pass hands `requeue_pt_witness` **every** executor-created framebuffer page,
        // unconditionally, on every doorbell. `[measured 2026-08-14, w315 boot `full`]` that
        // is `resident=171 by-executor=53` and it is what keeps `decode_cpu_pt_writes`
        // perpetually non-empty: two consecutive launch doorbells print byte-identical
        // `drained=162 latched=52 rounds=1 → bound=0 … refusals=1592` at **22.3 ms each**.
        // Gating the *decode* would be gating the consumer; the producer is here, and with it
        // quiet the decode's own `latched == 0 ⇒ procs.is_empty() ⇒ break` does the rest —
        // an exit that already exists and that this rung does not have to invent.
        //
        // ⚠ **The arming edge is the store's EXECUTOR WRITE COUNT, not the page set.** The
        // page set is stable by construction (origin is FIRST-writer, so a page joins this
        // population once and never leaves it); what a re-queue can newly teach a decode is
        // that a page's BYTES changed. `FbStore::writes_by` is the only thing that says so.
        //
        // ⊘ `None` — a store that does not count — **arms**. UNMEASURED is not clean, and a
        // gate that read a missing counter as "nothing happened" would silently stop
        // witnessing on any store but `SparseFb`.
        let gate = selected_dirty_gate(DIRTY_GATE_WITNESS_ENV);
        let now = plane.fb_writes_by(kayfabe_device::fbwin::FbWriter::Executor);
        // ⚠ The guard's scope is the `{ }` and nothing else: the decision comes out as a
        // `bool`, and the `tally` (which takes a SECOND unranked mutex) and the `format!`
        // (which allocates) both run after it is dropped. Holding one unranked lock across
        // the acquisition of another is how an ordering nobody wrote down gets established.
        let clean = if gate && let Some(now) = now {
            let mut last = self
                .dirty
                .exec_writes
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let same = *last == Some(now);
            if !same {
                *last = Some(now);
            }
            same
        } else {
            false
        };
        if clean {
            self.dirty.tally(DirtyGate::WITNESS, false);
            return format!(
                " | EXEC-WITNESS ⊘SKIPPED(w318 dirty gate: the executor has written this store \
                 {} times, unchanged since the last pass, so re-queueing its pages could only \
                 re-derive the same decode)",
                now.map_or("⊘UNMEASURED".to_string(), |n| n.to_string()),
            );
        }
        self.dirty.tally(DirtyGate::WITNESS, true);
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
            " | EXEC-WITNESS ARMED resident={total} by-executor={exec} refused-at-cap={refused} \
             exec_writes={} gate={}",
            now.map_or("⊘UNMEASURED".to_string(), |n| n.to_string()),
            if gate { "on" } else { "off" },
        )
    }

    /// ★★★★★ **w329 — EXECUTE the revocations a settlement produced: BOTH halves, in the one
    /// order that is safe, synchronously.**
    ///
    /// # ★★★ The order is the safety argument, and it is not reversible
    ///
    /// 1. **The guest's view first** (`RegPlane::release_fb_join`). While the join is installed
    ///    the framebuffer store serves that range out of the isolate's shared mapping; unmapping
    ///    the host object first would leave the store reading a region that no longer exists —
    ///    a `SIGBUS` inside a guest MMIO access, with no other detector.
    /// 2. **The host object second** — and **only if step 1 said a join was actually there.**
    ///    `release_fb_join` returning `false` means the store held nothing at that offset, which
    ///    is a disagreement between the table and the store; the conservative answer is to leave
    ///    the object alone and count it. ⊘ A leak in that corner is strictly better than a free
    ///    of an object something is still reading through.
    /// 3. **The drain, in this same trap.** `revoke_published_fb_leaf` *stages*; the unmap that
    ///    carries RM's synchronous TLB invalidate happens in `drain_pending_releases`. Per the
    ///    owner-agreed direction ruling, **a revocation is not deferrable**: the invalidate is
    ///    *inside* the ioctl, so deferring the ioctl defers the invalidate by the same interval
    ///    and that interval is a GMMU leak window. It is called here rather than left to the
    ///    next verb or to `w326`'s 250 ms tick.
    ///
    /// Returns the clause to print. ⊘ Prints even when nothing was revoked, because *"the arm
    /// is on and the guest proposed nothing"* and *"the arm is off"* are different facts and a
    /// missing clause could not carry either.
    fn release_revoked_joins(
        &self,
        plane: &RegPlane,
        revoked: &[kayfabe_fwd::RevokedLeaf],
        still_desired: usize,
        remaps_refused: usize,
    ) -> String {
        if revoked.is_empty() {
            return format!(
                " revoked=0 released=0 stranded=0 drained=0 joined_ranges={} \
                 remaps_refused={remaps_refused}",
                plane.joined_fb_ranges().len()
            );
        }
        let (mut released, mut stranded) = (0usize, 0usize);
        let mut first: Option<String> = None;
        for r in revoked {
            if plane.release_fb_join(r.phys) {
                self.device
                    .revoke_published_fb_leaf(r.gpu, r.pdb, r.host_va, r.memory);
                released += 1;
                if first.is_none() {
                    first = Some(format!(
                        "va=0x{:x} len=0x{:x} fb_phys=0x{:x} host_va=0x{:x}",
                        r.va.0, r.len, r.phys, r.host_va
                    ));
                }
            } else {
                // ⊘ The table said this row was a join and the store held nothing at that
                // offset. LOUD, and the object is NOT freed — see the doc above.
                stranded += 1;
                eprintln!(
                    "kayfabe: JOIN-RELEASE ⚠ TABLE/STORE DISAGREE va=0x{:x} fb_phys=0x{:x} — the \
                     address table carried a JoinsGuestWindow row here and the framebuffer store \
                     holds no join at that offset. ⊘ The host object is NOT freed: a leak here is \
                     strictly better than freeing memory something may still be reading through",
                    r.va.0, r.phys
                );
            }
        }
        // ★★★ SYNCHRONOUS, per the direction ruling. See step 3 above.
        let drained = self.device.drain_pending_releases();
        format!(
            " revoked={} released={released} stranded={stranded} drained={drained} \
             joined_ranges={} still_desired={still_desired} remaps_refused={remaps_refused} \
             first=[{}]",
            revoked.len(),
            plane.joined_fb_ranges().len(),
            first.as_deref().unwrap_or("NONE"),
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
        let revoke_policy = selected_join_release();
        let mut revoked: Vec<kayfabe_fwd::RevokedLeaf> = Vec::new();
        let (mut revoked_still_desired, mut remaps_refused) = (0usize, 0usize);
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
                // ★★★ w329 - the policy is read ONCE per pass and carried, never re-read
                // per proc: a variable that could change under a pass would make one boot's
                // arms differ from each other rather than from the control.
                let Some(out) =
                    self.device
                        .decode_pt_writes_revoking(pid, &fmt, &mut fb, revoke_policy.policy())
                else {
                    continue;
                };
                // ★★★★★ THE OBLIGATION, accumulated whole and discharged ONCE below. Per
                // proc would drain the release queue once per proc per doorbell; the rows are
                // already out of the table either way, so the only question is when the host
                // verbs run, and the answer is "this trap, once".
                revoked.extend(out.revoked.iter().copied());
                revoked_still_desired += out.revoked_still_desired;
                remaps_refused += out.remaps_refused;
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
                acc.collisions.extend(out.shape_collisions.iter().copied());
                acc.duplicates += out.duplicate_leaves;
                acc.straddles.extend(out.refusals.iter().copied());
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
        // ★★★★★ **w329 — BOTH HALVES OF THE RELEASE, HERE, SYNCHRONOUSLY.** Ordered after
        // every decode of this pass and before the line is printed, so the counts it reports
        // and the state the publication pass will find are the same state.
        let revoke_clause =
            self.release_revoked_joins(&plane, &revoked, revoked_still_desired, remaps_refused);
        // ⊘ THE LEFTOVERS GO BACK. A page the index cannot name an owner for is not a page
        // that was not written, and the witness is the only record that it was.
        let requeue_refused = plane.requeue_pt_witness(pending.iter().copied());
        let st = plane.pt_witness_stats();
        format!(
            " | PT-DECODE drained={drained} latched={latched} unowned_vas={vas_gone} \
             requeued={} rounds={rounds}{revoke_arm}{revoke_clause} → bound={} unchanged={} \
             repointed={} unbound={} \
             learned={} published={}/{} meta_refused={} unwitnessed={} unreachable={} \
             sparse={} dropped={} refusals={} faults={} reach_faults={} retired={} \
             pass_vas_gone={} first={}{}{} [witness writes={} pending={} refused={}+{}]",
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
            straddle_census(&acc.straddles),
            collision_census(&acc.collisions, acc.duplicates),
            st.writes,
            st.pending,
            st.refused,
            requeue_refused,
            revoke_arm = format!(" | JOIN-RELEASE arm={}", revoke_policy.as_str()),
        )
    }

    /// ★★★★★ **THE WHOLE-VAS SWEEP AT THE DOORBELL** — the C's `enum_gr_sysmem`, driven.
    ///
    /// # ★★★★★ RESTORED 2026-08-14 (w313) — **IT WAS NEVER INERT; IT WAS INERT FOR `cup3`.**
    ///
    /// w304 deleted this pass after measuring it inert on `^CUP3_VAL=43` (`w298ptsweep`,
    /// `w304ptsweep`). ⊘ **`43` is cup3: libcuda, a GR launch.** `R33 arm 1` — the raw CE
    /// client, no libcuda, its own `FERMI_VASPACE_A`, its own operands — **FAILS with this
    /// pass deleted and PASSES with it armed**, measured one-variable-per-boot at `8d258daa`
    /// (`KAYFABE_PT_SWEEP=off` ⇒ FAIL, everything else at its committed default) and bisected
    /// to the deletion merge `d2c58075`. ⇒ *inert for one workload* was read as *inert*, and
    /// the two workloads do not exercise the same publication paths: a raw CE client has no
    /// libcuda to establish its mappings by another route.
    ///
    /// ⚠ **The correctness residual is unchanged and still stands** — see
    /// [`kayfabe_mmu::reach::ReachShadow::witness_swept`] and the owner ruling of 2026-08-12.
    /// This is a relaxation, it is armed by [`PT_SWEEP_ENV`], and a boot's log must state
    /// which arm it ran.
    ///
    /// # Why this exists beside [`Self::decode_cpu_pt_writes`] rather than replacing it
    ///
    /// They are the C's **two** halves and neither is the other's improvement:
    ///
    /// - the decode pass drains what the guest was *seen* to write, and is the source of the
    ///   dirty signal this sweep re-arms on;
    /// - this sweep walks the address space from its **own installed root**, so it finds
    ///   mappings whose writes no transport of ours witnessed — `[measured, w265]` the witness
    ///   covers 3.2 % of the writers.
    ///
    /// ⇒ Running the sweep without the decode pass would have the relaxation and not its
    /// mitigation. The order — decode first, sweep second — is what makes a write that landed
    /// *this* window arm the sweep in the *same* doorbell rather than the next one.
    ///
    /// # ⊘ What a green line here is NOT
    ///
    /// `bound=N` says the address table accepted N mappings. It does **not** say the engine
    /// can reach them, that the ring advanced, or that a completion landed.
    ///
    /// # ⊘⊘ THE FOUR CENSUS ROWS ARE **NOT** EMITTED HERE, AND THAT IS w304'S FIX KEPT
    ///
    /// `GUEST-DESCRIBES` / `TABLE-DESCRIBES` / `HOST-PUBLISHED` / `PROMOTE-PARKED` used to be
    /// printed from *inside this function's* format string, so a boot with the sweep off
    /// printed no census at all and w297's criterion (E) read that absence as a regressed
    /// address plane. They now live in [`Self::vas_census`], which is **unconditional**. This
    /// restore brings back the sweep's *behaviour* and leaves that separation alone: the two
    /// are printed side by side on the same `PT-DECODE` line, and a reader can tell "the
    /// census ran and found nothing" from "the sweep was disarmed".
    ///
    /// ⊘ Silent when disarmed, so the control's log stays byte-comparable.
    fn sweep_cpu_pt_tables(&self) -> String {
        if !selected_pt_sweep() {
            return String::new();
        }
        let Some(plane) = self.plane.upgrade() else {
            return " | PT-SWEEP ⊘ NO PLANE (nothing to read page-table bytes out of)".to_string();
        };
        let fmt = kayfabe_chips::Ga10xGmmu::new();
        let pids = self.device.live_pids();
        let (mut tasks, mut skipped, mut ran, mut trunc, mut pages) = (0usize, 0usize, 0, 0, 0);
        let (mut bound, mut swept_binds, mut unbound, mut unwitnessed) = (0usize, 0, 0, 0);
        let (mut published, mut faults, mut reach_faults, mut refusals) = (0usize, 0, 0, 0);
        // ★★★★★ **`unchanged` AND `dropped`, AND THEY ARE THE READING, NOT DECORATION.**
        //
        // `[measured, w276_on]` the first armed boot read `bound=0 swept_binds=0 pages=79
        // refusals=255` — and **that set of numbers has two opposite readings**:
        //   (a) the sweep found leaves and the table would not take them;
        //   (b) the sweep found leaves that were **already bound**, so there was nothing to add.
        // Only `unchanged` separates them, and it was not printed.
        let (mut unchanged, mut dropped, mut repointed) = (0usize, 0, 0);
        // ★★ And the shadow's own answer to *"was the relaxation even reachable"*: how many
        // pages are admitted ONLY by the sweep. `swept_binds=0` with `swept_only=0` means the
        // witness transport already covered every root-reachable page — a statement about the
        // TRANSPORT. `swept_binds=0` with `swept_only>0` would mean those pages held no
        // bindable leaves — a statement about the GUEST. Two different findings.
        let mut swept_only = 0usize;
        let mut reasons: std::collections::BTreeMap<kayfabe_fwd::SweepReason, usize> =
            std::collections::BTreeMap::new();
        let mut first_fault: Option<String> = None;
        let mut refusal_kinds: std::collections::BTreeMap<&'static str, usize> =
            std::collections::BTreeMap::new();
        let mut refusal_vas: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
        // ★ w327 — the `RepointsPublished`/`UnbindsPublished` subset, kept apart. See the
        // comment at its insert site for why the general list cannot answer for it.
        let mut pubconflict_vas: std::collections::BTreeSet<u64> =
            std::collections::BTreeSet::new();
        let mut straddles: Vec<kayfabe_mmu::walker::PopulateRefusal> = Vec::new();
        let mut collisions: Vec<kayfabe_mmu::reach::ShapeCollision> = Vec::new();
        let mut duplicates = 0usize;
        let revoke_policy = selected_join_release();
        let mut revoked: Vec<kayfabe_fwd::RevokedLeaf> = Vec::new();
        let (mut revoked_still_desired, mut remaps_refused) = (0usize, 0usize);
        for pid in pids {
            // ★ The SAME byte source the decode pass uses. `[measured 2026-08-10, boot
            // `w208_797a6bc_real`]` all five of the walling ring's page-table pages carry
            // `/byBAR2`, so the guest's CPU wrote them into the device's own store — a sweep
            // reading the isolate's aperture instead would walk a tree nobody wrote.
            let mut fb = plane.pt_bytes();
            let Some((plan, out)) =
                self.device
                    .sweep_pt_tables_revoking(pid, &fmt, &mut fb, revoke_policy.policy())
            else {
                continue;
            };
            // ★★★★★ w329 — the sweep proposes unbinds too, and from the SAME settlement
            // machinery, so it carries the same obligation. Accumulated and discharged once
            // below, exactly as the decode pass does.
            revoked.extend(out.revoked.iter().copied());
            revoked_still_desired += out.revoked_still_desired;
            remaps_refused += out.remaps_refused;
            tasks += plan.tasks.len();
            skipped += plan.skipped;
            for r in &plan.reasons {
                *reasons.entry(*r).or_default() += 1;
            }
            ran += out.sweeps_run;
            trunc += out.sweeps_truncated;
            pages += out.pages_swept;
            bound += out.bound;
            swept_binds += out.swept_binds;
            unbound += out.unbound;
            unwitnessed += out.unwitnessed;
            unchanged += out.unchanged;
            repointed += out.repointed;
            dropped += out.dropped.len();
            swept_only += self.device.vas_swept_only(pid);
            published += out.pages_published;
            faults += out.faults.len();
            reach_faults += out.reach_faults.len();
            refusals += out.refusals.len();
            if first_fault.is_none() {
                if let Some(f) = out.faults.first() {
                    first_fault = Some(format!("{f:?}"));
                } else if let Some(r) = out.reach_faults.first() {
                    first_fault = Some(format!("{r:?}"));
                } else if let Some(r) = out.refusals.first() {
                    first_fault = Some(format!("{r:?}"));
                }
            }
            // ★★★★★ **WHICH ADDRESSES THE TABLE REFUSED — by KIND and by VA, not just the
            // first one.** A `first=` that names a different address than the fault reads as
            // *"unrelated"* and is the exact shape of `a_count_cannot_see_a_substitution`.
            // ⊘ Deduped and capped, and the cap SAYS SO.
            for r in &out.refusals {
                let (kind, va) = refusal_kind_va(r);
                *refusal_kinds.entry(kind).or_default() += 1;
                if let Some(v) = va {
                    refusal_vas.insert(v);
                    // ★★★★★ **w327 — THE CAP IS TAKEN FROM A SORTED SET, SO IT ALWAYS SHOWS
                    // THE LOWEST ADDRESSES — AND THE ONES A READER NEEDS ARE THE HIGHEST.**
                    //
                    // `refusal_vas` is a `BTreeSet`, `.take(24)` walks it in ASCENDING order,
                    // and every boot of this campaign therefore prints the same two dozen
                    // `0x203e…`/`0x203f…` kernel addresses while the guest's own operands live
                    // at `0x7xxx_xxxx_xxxx`. `[measured w327]` the failing boots' whole
                    // question was *"is the faulting buffer among the refused VAs"*, and the
                    // list that exists to answer it **cannot reach that far** — it is
                    // `a_count_cannot_see_a_substitution` wearing a list's clothes.
                    //
                    // ★ So the two refusals that mean *"the table is holding a binding the
                    // guest has already reused"* get their own list, and it is printed from
                    // BOTH ENDS. ⊘ Kind-filtered rather than cap-raised, because raising the
                    // cap would print 1339 addresses per pass and bury the answer instead.
                    if matches!(kind, "RepointsPublished" | "UnbindsPublished") {
                        pubconflict_vas.insert(v);
                    }
                }
            }
            straddles.extend(out.refusals.iter().copied());
            collisions.extend(out.shape_collisions.iter().copied());
            duplicates += out.duplicate_leaves;
        }
        // ★★★★★ **w329 — the sweep's half of the release, discharged before the line prints.**
        let revoke_clause =
            self.release_revoked_joins(&plane, &revoked, revoked_still_desired, remaps_refused);
        format!(
            " | PT-SWEEP tasks={tasks} skipped={skipped} ran={ran} truncated={trunc} \
             pages={pages} reasons={reasons:?} JOIN-RELEASE{revoke_clause} → bound={bound} \
             unchanged={unchanged} \
             repointed={repointed} swept_binds={swept_binds} swept_only_pages={swept_only} \
             dropped={dropped} unbound={unbound} unwitnessed={unwitnessed} \
             published={published} faults={faults} reach_faults={reach_faults} \
             refusals={refusals} by_kind={refusal_kinds:?} refused_vas=[{}]{} \
             PUBCONFLICT_VAS[n={} lowest=[{}] highest=[{}]] first={} \
             |{}|{}",
            refusal_vas
                .iter()
                .take(PT_SWEEP_REFUSAL_CAP)
                .map(|v| format!("0x{v:x}"))
                .collect::<Vec<_>>()
                .join(","),
            if refusal_vas.len() > PT_SWEEP_REFUSAL_CAP {
                format!(
                    " ⚠⚠ CAPPED at {PT_SWEEP_REFUSAL_CAP} of {} distinct — an address ABSENT \
                     from this list is NOT thereby un-refused",
                    refusal_vas.len()
                )
            } else {
                String::new()
            },
            pubconflict_vas.len(),
            pubconflict_vas
                .iter()
                .take(PT_SWEEP_REFUSAL_CAP / 2)
                .map(|v| format!("0x{v:x}"))
                .collect::<Vec<_>>()
                .join(","),
            pubconflict_vas
                .iter()
                .rev()
                .take(PT_SWEEP_REFUSAL_CAP / 2)
                .map(|v| format!("0x{v:x}"))
                .collect::<Vec<_>>()
                .join(","),
            first_fault.as_deref().unwrap_or("NONE"),
            straddle_census(&straddles),
            collision_census(&collisions, duplicates),
        )
    }

    /// ★★★★★ **THE PER-VAS ADDRESS-PLANE CENSUS — four pictures of one address space, on one
    /// line, joinable by `proc`/`pdb`/`va` against an `Xid`.**
    ///
    /// `GUEST-DESCRIBES` (what the guest's own tables reach) · `TABLE-DESCRIBES` (what OUR
    /// address table holds) · `HOST-PUBLISHED` (what is actually backed in the host VAS) ·
    /// `PROMOTE-PARKED` (halves the promote control is holding).
    ///
    /// # ⊘⊘⊘ CORRECTED 2026-08-14 (w313) — **THE SWEEP IS BACK. THE CENSUS SPLIT IS NOT.**
    ///
    /// The block below says `sweep_cpu_pt_tables` "is gone". It is not: it was **restored at
    /// w313** because it is *not* inert — `R33 arm 1`, a raw CE client with no libcuda, fails
    /// without it (bisected to the deletion merge `d2c58075`, ablated one-variable-per-boot at
    /// `8d258daa`). ⇒ Read the block below as *"the census stopped being emitted from inside
    /// the sweep's format string"*, which is the half that was right and is kept: this
    /// function is **unconditional** and the sweep prints its own separate `PT-SWEEP` clause.
    ///
    /// # ⊘⊘ w304 — THIS REPLACES `sweep_cpu_pt_tables`, AND THE SWEEP WAS THE ONLY THING
    /// # DELETED. THE CENSUS IS NOT ONLY KEPT, IT IS **UNGATED FOR THE FIRST TIME.**
    ///
    /// `KAYFABE_PT_SWEEP=on` armed a whole-VAS page-table walk that committed every reached
    /// leaf under `Admit::Swept` — a **correctness relaxation**, admitting pages no transport
    /// of ours witnessed the guest writing. It was measured INERT on two independent boots
    /// (`w298ptsweep`, `w304ptsweep`: `^CUP3_VAL=43` with the walk off), so it is gone.
    ///
    /// ★★★ **AND DELETING IT EXPOSED A DEFECT THAT HAD ALREADY COST A GRADING CRITERION.**
    /// The four census rows above were emitted from *inside* the sweep's format string, and
    /// `sweep_cpu_pt_tables` returned an EMPTY STRING when the flag was off — so on any boot
    /// without `KAYFABE_PT_SWEEP` the census printed **nothing at all**. That is why
    /// `w298ptsweep` shows `host_rows = ⊘ABSENT`: **the publication did not stop, its only
    /// reporter did.** w297's regression criterion (E) then read that absence as a failed
    /// address plane and would have called a `43` a regression. ⚠ The sweep's own source even
    /// argued the point one level down — the `pub_ranges`/`parked_halves` loop carried a
    /// comment saying it was *"deliberately OUTSIDE the sweep loop and NOT gated"* — while the
    /// whole function sat behind the flag. `w277` had separately named the same mis-gating for
    /// `TABLE-DESCRIBES` (*"the table dump has nothing to do with the sweep and should not be
    /// gated on it"*) and it was never acted on. ⇒ **A diagnostic gated on an unrelated
    /// feature is a diagnostic that is absent exactly when someone needs it.**
    ///
    /// ⊘ Unconditional now, and it must stay that way: an empty row here means `live_pids()`
    /// was empty, which is a different fact and says so.
    fn vas_census(&self) -> String {
        let pids = self.device.live_pids();
        let (mut reach, mut table, mut published, mut parked) = (
            Vec::<String>::new(),
            Vec::<String>::new(),
            Vec::<String>::new(),
            Vec::<String>::new(),
        );
        for pid in &pids {
            reach.extend(self.device.vas_reachable_ranges(*pid, PT_SWEEP_RANGE_CAP));
            table.extend(self.device.vas_table_ranges(*pid, PT_SWEEP_RANGE_CAP));
            published.extend(self.device.vas_published_ranges(*pid, PT_SWEEP_RANGE_CAP));
            parked.extend(self.device.vas_promote_halves(*pid));
        }
        let none = |v: &Vec<String>, what: &str| {
            if v.is_empty() {
                format!("(no live proc — ⊘ NOT 'nothing is {what}')")
            } else {
                v.join(" ")
            }
        };
        format!(
            " | VAS-CENSUS procs={} | GUEST-DESCRIBES {} | TABLE-DESCRIBES {} \
             | HOST-PUBLISHED {} | PROMOTE-PARKED {}",
            pids.len(),
            none(&reach, "reachable"),
            none(&table, "in the table"),
            none(&published, "published"),
            none(&parked, "parked"),
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
            // ★★★★★ **EVERY argument word, not the first.** Printed as itself, never
            // interpreted here — the interpretation belongs to whoever reads the class header.
            //
            // ⊘⊘ **This line used to print `words[i + 1]` and stop, and the truncation cost a
            // whole rung.** `[measured, w266, `run_w266_on_qemu.log`, all 8 CE channels]` the
            // copy engine's submission is three methods —
            // `sub4/m0x0/n1=0xc7b5` (`SET_OBJECT`, `AMPERE_DMA_COPY_B`),
            // `sub4/m0x240/n3` (`SET_SEMAPHORE_A`/`_B`/`_PAYLOAD`, `clc7b5.h:47-52`) and
            // `sub4/m0x300/n1=0x14` (`LAUNCH_DMA`, `clc7b5.h:84-105`) — and the eight host
            // `Xid 31 … ACCESS_TYPE_VIRT_WRITE` are that semaphore release faulting. The run
            // that names the faulting address rendered as **`=0x2`**: the `_A` half alone,
            // with `_B` (the low 32 bits, i.e. the entire offset within the page) and the
            // payload dropped. ⇒ The address hardware faulted on was **already read, already
            // in this buffer, and thrown away at the print**.
            //
            // ★ The comment this replaces called `words[i + 1]` *"a semaphore's address
            // half"* — it named the defect and kept it. A dump that prints one of three
            // arguments is not a smaller dump; for a multi-word operand it is a **wrong** one,
            // because the half it keeps is the half that carries the least information.
            if d.arg_words > 0 && i + 1 < words.len() {
                let end = (i + 1 + d.arg_words).min(words.len());
                let args: Vec<String> = words[i + 1..end]
                    .iter()
                    .map(|w| format!("0x{w:x}"))
                    .collect();
                out.push_str(&format!("=[{}]", args.join(",")));
                // ⊘ A run whose arguments run past the bytes read is SAID, never silently
                // short — `PROBE_PUSH_BYTES` is a bound and a bound that hides itself is the
                // `dlen=0` class.
                if end < i + 1 + d.arg_words {
                    out.push_str(&format!("/SHORT-{}of{}", end - (i + 1), d.arg_words));
                }
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

/// ★★ **How many host pages one doorbell's pin pass will describe.** 512 × 4 KiB = 2 MiB.
///
/// ⊘ Separate from [`PUSHBUF_MAX_EXTENTS`] because they bound different guest freedoms: one
/// long extent and many short ones cost the same table lookups per *page*, and a cap on
/// extents alone would let a single entry with a 21-bit `LENGTH` field ask for 8 MiB of pins.
/// Overflow is reported the same way and for the same reason.
// ⊘ **`#[cfg(feature = "host-isolates")]`, w304.** The last reader of this item is
// `join_operand_fb_leaves`, which lives in the `host-isolates` arm; the three guest-RAM pins
// that also used it are deleted. A default-feature build would otherwise carry it dead and
// `cargo clippy --workspace --all-targets` (which CI runs WITHOUT `--all-features`) would
// report it under `-D warnings` — the same w296 gate, one deletion later.
#[cfg(feature = "host-isolates")]
const PUSHBUF_MAX_PAGES: usize = 512;

/// How many refused addresses [`SharedDoorbell::pin_pushbuffer_guest_ram`] names in its
/// report before it stops naming them and only counts.
///
/// ⊘ The **count** is never truncated — only the sample is. A line that said "some pages
/// missed" without a number is the shape that lets a partial pass read as a whole one.
// ⊘ **`#[cfg(feature = "host-isolates")]`, w304.** The last reader of this item is
// `join_operand_fb_leaves`, which lives in the `host-isolates` arm; the three guest-RAM pins
// that also used it are deleted. A default-feature build would otherwise carry it dead and
// `cargo clippy --workspace --all-targets` (which CI runs WITHOUT `--all-features`) would
// report it under `-D warnings` — the same w296 gate, one deletion later.
#[cfg(feature = "host-isolates")]
const PUSHBUF_REPORT: usize = 4;

/// ★★★★★ **Render a bounded SAMPLE beside its own true COUNT, and say which it is.**
///
/// ⊘ `v` is capped at [`PUSHBUF_REPORT`]; `n` is the real number. When they differ the
/// rendering says **`SAMPLE of n`** and how many are not shown — because `[a b c d]` printed
/// beside `9 MISS` reads as a list of the nine, and a reader who takes it for one will
/// conclude the other five addresses do not exist.
///
/// ★ This exists because my own first draft of
/// [`SharedDoorbell::pin_pushbuffer_guest_ram`] used `wrong_aperture.len()` — the
/// **sample's** length — as the count, and derived the MISS count by subtracting it. Both
/// numbers would have been wrong the moment a fifth page refused, and both would have looked
/// like measurements. It is the same defect [`ring_scan_sentence`] was extracted from this
/// same file to fix, one instrument later.
// ⊘ **`#[cfg(feature = "host-isolates")]`, w304.** The last reader of this item is
// `join_operand_fb_leaves`, which lives in the `host-isolates` arm; the three guest-RAM pins
// that also used it are deleted. A default-feature build would otherwise carry it dead and
// `cargo clippy --workspace --all-targets` (which CI runs WITHOUT `--all-features`) would
// report it under `-D warnings` — the same w296 gate, one deletion later.
#[cfg(feature = "host-isolates")]
fn pushbuffer_sample(v: &[String], n: usize) -> String {
    if v.is_empty() {
        String::new()
    } else if n > v.len() {
        format!(" [{} … +{} more, SAMPLE of {n}]", v.join(" "), n - v.len())
    } else {
        format!(" [{}]", v.join(" "))
    }
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
    let release = selected_join_release();
    // ---- 0. ★★★★★ **w329 LEG 2 - TAKE OVER A STALE JOIN OF THIS FRAME, BEFORE
    // anything is minted.**
    //
    // Ordered FIRST, and that ordering is the whole reason it is cheap: doing it at the
    // `ALREADY_JOINED` refusal would mean a host object had already been allocated and mapped
    // for a join that then had to be re-attempted, and `RegPlane::join_fb` consumes the
    // region on refusal so the retry would need a second `mmap` too. Asked here, the ordinary
    // four-step join below runs ONCE and installs cleanly.
    //
    // ⊘ This can only fire for a candidate row, which by construction has NO host
    // object of its own - so a join already installed at this frame is necessarily owned by a
    // DIFFERENT VA. See `SharedDevice::supersede_joined_fb_leaf` for what is and is not proven.
    // ★ The store is asked FIRST, and it is the cheap question: `joined_ranges` is
    // tens of entries while the address-table scan below is tens of thousands of rows, and on
    // the overwhelming majority of leaves there is no collision at all.
    if release.supersedes() && plane.fb_join_installed_at(leaf.phys) {
        let over = {
            let l = supersede_ledger().lock().unwrap_or_else(|e| e.into_inner());
            l.get(&leaf.phys).copied().unwrap_or(0) >= SUPERSEDE_CAP_PER_FRAME
        };
        if over {
            eprintln!(
                "{head} {what} leaf va=0x{:x} fb_phys=0x{:x} -> ⊘ SUPERSEDE CAPPED at \
                 {SUPERSEDE_CAP_PER_FRAME} takeovers for this frame. The old join stands and \
                 this leaf stays fabricated. ⚠ The cap exists because the superseded row \
                 is re-proposed by the next settlement, so an uncapped takeover is a ping-pong",
                leaf.va, leaf.phys
            );
        } else if let Some(r) = device.supersede_joined_fb_leaf(
            DOORBELL_TARGET_GPU,
            pdb,
            leaf.phys,
            kayfabe_rt::GpuVa(leaf.va),
        ) {
            // ★★★ TABLE ROW GONE (above), STORE next, HOST last. The store must stop
            // serving out of the region before the host mapping is torn down; the row must
            // stop naming the object before either.
            if plane.release_fb_join(r.phys) {
                device.revoke_published_fb_leaf(r.gpu, r.pdb, r.host_va, r.memory);
                let drained = device.drain_pending_releases();
                *supersede_ledger()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .entry(r.phys)
                    .or_insert(0) += 1;
                eprintln!(
                    "{head} {what} ★★★★★ SUPERSEDED fb_phys=0x{:x}: the guest re-pointed \
                     this frame from va=0x{:x} (len=0x{:x}, host_va=0x{:x}) to va=0x{:x}. Old \
                     row UNBOUND, join RELEASED, host object staged and drained={drained}. \
                     ⊘ The old VA is still DESCRIBED by the guest and now resolves with \
                     no host backing - an engine still pointed there takes a CONTAINED fault",
                    r.phys, r.va.0, r.len, r.host_va, leaf.va
                );
            } else {
                // ⊘ The table named a join this store does not hold. LOUD, and the object
                // is NOT freed - a leak here is strictly better than freeing memory something
                // may still be reading through. ⚠ The row is already unbound, so this
                // orphans it; that is the conservative half of a disagreement we did not make.
                eprintln!(
                    "{head} {what} ⚠⚠ SUPERSEDE ABORTED fb_phys=0x{:x}: the address \
                     table carried a JoinsGuestWindow row at va=0x{:x} and the framebuffer \
                     store holds NO join at that offset. The row is unbound and the host \
                     object is ⊘ NOT freed",
                    r.phys, r.va.0
                );
            }
        }
    }
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
    /// ★★★★★ **w326 — the revocation drain's OWN driver** (`crate::reclaimtick`).
    ///
    /// `w323` measured that the drain's only production caller is `Regs::write`, i.e. a
    /// guest MMIO write — so a guest that frees its host objects and then stops trapping
    /// leaves a live host-GPU translation into pages Linux has reused. **A bound
    /// discharged only by the adversary is not a bound.** This handle is shared with the
    /// off-trap observer thread, which spends the queue on its own 250 ms tick.
    ///
    /// ⊘ It is also the **mutual exclusion**: two concurrent drains of one queue would
    /// double-free a host RM object. The vCPU side only ever `try_lock`s.
    reclaim: Arc<crate::reclaimtick::ReclaimTick>,
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
    /// ★★ **w310** — the last guest-RAM pin-reclaim total this shim printed, so the
    /// `PIN-RELEASE` line fires on **change** rather than on every register write.
    ///
    /// ⊘ An `AtomicUsize` and not a `Cell<PinReclaim>`: [`Regs::write`] takes `&self` and
    /// this device is driven from every vCPU, so the counter must be `Sync`. It holds the
    /// **sum** of all three tallies, which is what makes one atomic enough — any of the
    /// three moving moves the sum, and the line then prints all three.
    last_pin_reclaim: std::sync::atomic::AtomicUsize,
    /// ★★★ **w314 — THE CLAUSE-(b) INSTRUMENT.** The longest single `reap_retired()` this
    /// boot has spent inside [`Regs::write`], in microseconds.
    ///
    /// `docs/design/guest_ram_pin_release.md` §5 names a **pre-existing** clause-(b)
    /// exposure: w303's armed reap puts an *unbounded* disposal on the BQL path, and the
    /// arithmetic (`host_rows = 12 818` × ~240 µs ⇒ ~3 s) sits under `scrubberDestruct`'s
    /// 4 000 ms with every vCPU halted. That is **arithmetic, not a measurement**, and this
    /// field is the measurement.
    ///
    /// ⊘ Printed only when a **new maximum** is reached (`fetch_max` returns the previous),
    /// so it is one line per record rather than one per guest MMIO write — the density trap
    /// the `PIN-RELEASE` line above already documents. ⚠ A boot with no line has **not**
    /// measured zero; it has never reaped, and that is `UNMEASURED` for this instrument.
    max_reap_us: std::sync::atomic::AtomicU64,
    /// ★★★★★ **w317 — the same instrument on the BUDGETED half.** The longest single
    /// [`SharedDevice::drain_retired_budgeted`] this boot has spent inside [`Regs::write`],
    /// in microseconds.
    ///
    /// It exists because the budget is a *claim* and this is its falsifier: the drain is
    /// supposed to stop at [`RETIRED_DRAIN_BUDGET_US`] plus one chunk's overshoot, and the
    /// only way to know it does on real hardware — where the per-disposal cost is whatever
    /// RM makes it — is to measure the thing itself. ⚠ Same reading rule as
    /// [`Regs::max_reap_us`]: **a boot with no line is UNMEASURED, not zero.**
    max_drain_us: std::sync::atomic::AtomicU64,
    /// ★★★ **w317 — the last `deferred_for_drain` this shim printed**, so the
    /// `DRAIN-DEFER` line fires on **change** rather than on every register write.
    ///
    /// ★ It is the trajectory, not the value, that answers the pre-registered outcome (B).
    /// A run that goes `0 → 1 → 0` is the budget working: a proc vacated, its queue was spent
    /// over several traps, and it reaped. A run that goes `0 → 1` and stays there is the
    /// budget having **moved** the cost rather than removed it — and it is unreadable from
    /// any single sample, which is why the line prints every transition.
    last_deferred_for_drain: std::sync::atomic::AtomicUsize,
}

/// ★★★★★ **w317 — THE BUDGET, and what it is a fraction OF.**
///
/// **40 000 µs = 40 ms = 1 % of `scrubberDestruct`'s 4 000 ms** — the shortest *named*
/// guest-side timeout in this tree (`ce_utils.c:349`, quoted in
/// `blocking_and_completion_model.md` §1 as the bound on `INLINE-SAFE` clause (b)).
///
/// # Why a fraction, and why this fraction
///
/// ⊘ **Not "4 s minus epsilon".** The 4 s is one guest operation on one workload; a budget
/// sized to just fit it fails on the next workload with a tighter timeout, and this campaign
/// has already been bitten once by grading a single workload (`relaxation_inert_gate.sh`
/// exists because of it). The number therefore has to buy headroom for timeouts nobody has
/// enumerated yet.
///
/// **1 % buys two independent margins at once:**
/// 1. **A 100× margin on the named bound.** A guest operation whose timeout is 100× tighter
///    than the scrubber's — 40 ms — still survives one full drain, and an operation at the
///    scrubber's own scale survives ~100 of them.
/// 2. **Below human/timer perceptibility.** 40 ms is under one 24 Hz frame and under the
///    10 ms×4 scale at which QEMU's own main loop starts visibly missing timers. A freeze of
///    the whole VM (which is what a BQL hold is — §0 of the blocking model) at this size is
///    indistinguishable from ordinary scheduling jitter.
///
/// # ⚠ And what it is deliberately NOT derived from
///
/// It is **not** `N disposals × the measured per-disposal cost`. w314 measured that exact
/// estimate wrong by **~20×** (`munmap` of a `MAP_SHARED` memfd window RM has
/// `pin_user_pages`-pinned: 35 µs, not the 1–2 µs §5 assumed), and a count-based budget
/// silently degrades by whatever factor that estimate is off. A **time** budget cannot: it
/// re-measures the cost every turn, for free, by construction. The count below is a
/// granularity knob, not the bound.
pub const RETIRED_DRAIN_BUDGET_US: u64 = 40_000;

/// ★★★ **w317 — the granularity, not the budget**, and the value below is **MEASURED, not
/// estimated.** Disposals taken per plan→execute→check-in turn. The deadline is only re-read
/// *between* turns, so the delivered bound is `RETIRED_DRAIN_BUDGET_US + one chunk` and the
/// chunk is the only part of it that a wrong cost estimate can inflate.
///
/// # ⊘⊘ THE FIRST VALUE WAS 64 AND IT WAS WRONG — the bound it delivered was 3× the budget
///
/// `[measured 2026-08-14, vh, real GA106, n=4 cup3 boots at `chunk = 64`]`
/// `max_drain_us` came back **91 470 · 92 566 · 91 833 · 127 330 µs** — three of them
/// `disposed=64 turns=1`, i.e. **one chunk, alone, took ~92 ms** and the 40 ms deadline never
/// got a chance to bind. 64 was chosen against an estimate of ~70 µs per disposal; the truth
/// is ~120–145 µs typically and **~1.3–1.4 ms in the expensive phase**. ⇒ third estimate of
/// this quantity, third time low (w310 §5: 1–2 µs vs 35 µs measured).
///
/// # ★★★★★ AND THE VALUE BELOW IS DERIVED FROM THE ONE NUMBER THAT SETTLES IT
///
/// `[measured 2026-08-14, vh, `w317c1diag`, a THROWAWAY build with `chunk = 1`]` — the
/// discriminator between *"64 uniformly-slow disposals"* and *"one very slow disposal"*, which
/// have **opposite** fixes and which `disposed=64 turns=1` fits equally well:
///
/// ```text
///   DRAIN-TIMING max_drain_us=40068 disposed=231 turns=231 budget_hit=true
///   DRAIN-TIMING max_drain_us=40794 disposed=353 turns=353 budget_hit=true
///   DRAIN-TIMING max_drain_us=43260 disposed=13  turns=13  budget_hit=true
///   ⇒ CUP3_VAL=43  CUP3_RC=0 · DRAIN-DEFER 1 → 0
/// ```
///
/// With the deadline re-read after **every** disposal the worst trap is **43 260 µs**. ⇒ the
/// **worst single disposal is ≤ ~3.3 ms**; there is no monstrous indivisible one, and the
/// expensive phase is uniformly expensive. **A smaller chunk therefore cures the overshoot
/// proportionally** — which was exactly the thing not known when 64 was picked.
///
/// **The rule, stated so the next person can re-derive it:**
/// > `chunk × worst_single_disposal` may contribute **at most a third of the budget**.
///
/// `4 × 3.3 ms ≈ 13 ms` = **33 % of 40 ms** ⇒ delivered bound **≤ 53 ms = 1.3 % of
/// `scrubberDestruct`'s 4 000 ms**.
///
/// ⊘ **Not 1**, even though 1 measured fine: each turn costs a device write-lock acquisition,
/// a `return_worker` round and a `Worker::execute` call, and a backend where `execute` is one
/// IPC per *plan* rather than per verb would pay all of it per disposal. 4 keeps a 4×
/// amortisation of that while conceding only a third of the budget. ⚠ The overhead was **not
/// measured in isolation** — the chunk=1 arm's per-disposal cost (111–173 µs) merely sits in
/// the same range as chunk=64's (121–145 µs), which bounds it as *small*, not as *zero*.
pub const RETIRED_DRAIN_CHUNK: usize = 4;

/// ⊘ **A zero chunk would make the drain a no-op and the reap's `HoldUndrained` gate a
/// PERMANENT defer** — every retired proc held forever, its isolate child never `waitpid`ed,
/// its GPA arena never recycled. That is a strictly worse leak than the stall this rung
/// fixes, and it is one keystroke away. Refused at compile time rather than reasoned about.
const _: () = assert!(
    RETIRED_DRAIN_CHUNK > 0,
    "the drain chunk must make progress"
);

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
        // ★★★★★ w282's arm (LEG 7) — read ONCE, here, and its own variable rather than a rider
        // on w270's: the pin and the join serve DISJOINT operand populations (guest RAM vs
        // emulated framebuffer), so a boot must be able to arm either alone. See
        // [`OPERAND_JOIN_ENV`].
        let operand_join = selected_operand_join()?;
        // ★★★★★ w290 — leg 8's own selector. Parsed here and never re-read, so a boot cannot
        // change arms halfway; echoed below on BOTH arms, because a configuration that only
        // announces itself when enabled makes the control's log indistinguishable from an
        // older binary's.
        let vas_publish = selected_vas_publish()?;
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
        // ★★★★★ `[w281]` THE PUSHBUFFER'S OWN ROUTE — see `PUSHBUF_VIDMEM_ENV`. Printed
        // unconditionally and on BOTH arms, and printed WITH its dependency on route B, so
        // a log says not just "armed" but "armed and reachable".
        let pushbuf_vidmem = std::env::var(PUSHBUF_VIDMEM_ENV)
            .map(|v| v == "1" || v.eq_ignore_ascii_case("on"))
            .unwrap_or(false);
        device.set_pushbuffer_vidmem(pushbuf_vidmem);
        eprintln!(
            // ⊘ The words "route B ON" are deliberately NOT in this line: a grader grepping
            // `RING-VIDMEM .*route B ON` and one grepping this line must not be able to
            // match each other's evidence. Route B's state is reported as `supply=`.
            "kayfabe: PUSHBUF-VIDMEM {}={} ⇒ pushbuffer route {} (⊘ REACHABLE only when the \
             supply is on, and supply={}: the bytes come from route B's FbSource. armed={} \
             reachable={})",
            PUSHBUF_VIDMEM_ENV,
            std::env::var(PUSHBUF_VIDMEM_ENV).unwrap_or_else(|_| "<unset>".to_string()),
            if pushbuf_vidmem {
                "ON"
            } else {
                "OFF (default)"
            },
            if ring_vidmem { "ON" } else { "OFF" },
            pushbuf_vidmem,
            pushbuf_vidmem && ring_vidmem,
        );
        // ★★★★★ **THE SWEEP'S ARM, PRINTED ON BOTH ARMS** — for §5.12's reason below, plus
        // one this arm has and the others do not: it is the only flag in this file that
        // relaxes a **correctness gate** rather than adding a supply or an observation. A boot
        // whose on-disk evidence does not state whether that gate was relaxed cannot be
        // graded, and its control cannot be told apart from an older binary's.
        //
        // ⊘ Read here for the print and re-read per doorbell for the act. `selected_pt_sweep`
        // is a pure function of one environment variable that nothing in this process ever
        // sets, so the two readings cannot disagree — and the printed line is what a grader
        // asserts on.
        //
        // ★★★★★ **w313 — RESTORED. w304 deleted this arm as inert and it is not**: `R33 arm 1`
        // (a raw CE client, no libcuda) FAILS with the sweep off, measured one variable per
        // boot at `8d258daa`, and the regression bisects to the deletion merge `d2c58075`.
        eprintln!(
            "kayfabe: PT-SWEEP arm={} ⇒ the whole-VAS sweep is {} (⊘ when `on`, a leaf may \
             bind because a descent from the address space's OWN INSTALLED ROOT reached it, \
             rather than because the guest was seen to write its page — owner ruling \
             2026-08-12, residual recorded in mode2_address_table.md §6)",
            if selected_pt_sweep() { "on" } else { "off" },
            if selected_pt_sweep() {
                "ARMED"
            } else {
                "DISARMED (⊘ and R33 arm 1 does NOT pass on this arm — w313)"
            },
        );
        // ★★★★★ **w304's CENSUS BANNER, KEPT BESIDE THE SWEEP'S — they are two facts.** The
        // census that used to share the sweep's line is unconditional now, and a grader must
        // be able to tell "the census ran and found nothing" from "the census never ran".
        eprintln!(
            "kayfabe: VAS-CENSUS arm=always ⇒ the per-VAS address-plane census \
             (GUEST-DESCRIBES / TABLE-DESCRIBES / HOST-PUBLISHED / PROMOTE-PARKED) is \
             UNCONDITIONAL. ⚠ It USED TO BE EMITTED FROM INSIDE THE PT-SWEEP LINE, so a boot \
             with the sweep off printed no HOST-PUBLISHED row at all and w297's criterion (E) \
             read that absence as a regressed address plane. The publication never stopped; \
             its only reporter did. ⊘ w313: the sweep itself is BACK (it was not inert), but \
             this row stays ungated — the two are separate clauses on the PT-DECODE line"
        );
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
                     born. ⊘⊘ CORRECTED 2026-08-13 (w284): this banner used to end `the host \
                     channel still declares OUR ring and OUR USERD`, and that is FALSE since \
                     legs A2/B landed — read the `GR-BIRTH` lines below, not this sentence. \
                     A joined leaf is CONSUMED at the birth: `adopt=GUEST-RING` means the host \
                     channel declares the GUEST's ring, and `userd=GUEST-USERD` means its \
                     GP_PUT is the word in the guest's own USERD page. ⊘ What is still only \
                     supply is the CONVERSE: a leaf joined here does not mean any birth \
                     consumed it — `adopt=DECLINED` beside `joined=YES` is a real and \
                     measured row (w283d: the USERD one byte past the leaf's end)",
            },
        );
        // ★★★★★ w282's arm (LEG 7) — printed on BOTH arms and for `GUEST-OPERAND`'s exact
        // reason. `fb_join=` and `host_isolates=` are printed BESIDE it because the arm alone
        // is necessary and not sufficient: leg 7 refuses when the join's mapping arm is `off`
        // (it could then only map private anonymous pages) and it is a compiled no-op without
        // the feature.
        eprintln!(
            "kayfabe: OPERAND-JOIN arm={} fb_join={} host_isolates={} ⇒ a CE operand page that \
             lands in OUR EMULATED FRAMEBUFFER is {}",
            operand_join.as_str(),
            fb_join.as_str(),
            cfg!(feature = "host-isolates"),
            match operand_join {
                OperandJoinArm::Off =>
                    "LEFT THERE, SILENTLY — the default, byte-identical to every boot before \
                     w282. ⊘ Not one OPERAND-JOIN or #255 line and no second read of the ring",
                OperandJoinArm::Assert =>
                    "LEFT THERE and SAID SO — ★ THE CONTROL. Every operand is resolved per-VAS \
                     and classified and #255 states its verdict; NO leaf is joined and NO host \
                     verb is issued. Expected reading: `#255 … FIRED`, which is a POSITIVE \
                     observation rather than an absence (exactly w281b_clientsweep's state, \
                     where both operands resolved to Vidmem with no host object, the \
                     partitioner answered CeExecutor::Ours and ce_copy refused by name)",
                OperandJoinArm::Join =>
                    "WALKED to its framebuffer leaf and that leaf is JOINED — the same four \
                     steps the ring source and the GR operand census already use — so the \
                     guest's window and a real host object are ONE memory and the executor \
                     stays HostCe. ⊘ Supply side only: `the operand is host-backed` and `the \
                     submission retired` are different facts",
            },
        );
        // ★★★★★ w290 — leg 8's arming, echoed on BOTH arms beside leg 7's for the same
        // reason: the two legs serve DIFFERENT populations (a pushbuffer's named CE operands
        // vs every row the guest's page tables declare), so a reader must be able to tell
        // which of them a boot had without opening the shell that exported the variables.
        eprintln!(
            "kayfabe: VAS-PUBLISH arm={} fb_join={} host_isolates={} ⇒ a guest-declared              Vidmem row that is 64 KiB-granular is {}",
            vas_publish.as_str(),
            fb_join.as_str(),
            cfg!(feature = "host-isolates"),
            match vas_publish {
                VasPublishArm::Off =>
                    "LEFT UNPUBLISHED, SILENTLY — the default, byte-identical to every boot                      before w290. ⊘ Not one VAS-PUBLISH line",
                VasPublishArm::Assert =>
                    "LEFT UNPUBLISHED and COUNTED — ★ THE CONTROL. Every Vas is censused and                      every row classified by the gate that would refuse it; NO row is                      published and NO host verb is issued. Expected reading: a non-zero                      `candidates=` beside `published=0`, which is a POSITIVE observation                      rather than an absence",
                VasPublishArm::Publish =>
                    "PUBLISHED — through the identical four-step join leg 7 uses, so the                      guest's window and a real host object are ONE memory at the guest's own                      VA. ⊘ Supply side only: `the row is host-backed` and `the engine                      completed` are different facts. ⚠ A published row is FROZEN against the                      guest's own page-table edits until VAS teardown — watch                      RepointsPublished/UnbindsPublished in the sweep's by_kind",
                VasPublishArm::PinRate =>
                    "LEFT ALONE — ★ this arm publishes NO framebuffer row at all. It is w291's \
                     BOUNDED GUEST-RAM PIN-RATE MEASUREMENT: it pins up to 256 guest-RAM rows \
                     through the EXISTING pin_guest_ram verb and reports the true per-row \
                     cost, replacing the ~49 s/VAS figure that was an EXTRAPOLATION of leg 8's \
                     FRAMEBUFFER rate. ⊘ It writes NOTHING into Binding::host, adds no \
                     representation and touches no refcount — it is the measurement, NOT the \
                     merge",
                VasPublishArm::Both =>
                    "PUBLISHED, AND the guest-RAM half is pinned in the SAME boot — ★ w291 \
                     step 1, which has never been run: leg 8 was off on the (2a) arm and (2a) \
                     was off on leg 8's. ⚠ They cover DISJOINT populations and may simply \
                     SUM; that is a legitimate pre-registered outcome, not a disappointment",
                VasPublishArm::Drain =>
                    "PUBLISHED, AND the VAS THIS DOORBELL IS ABOUT is DRAINED of guest-RAM \
                     rows before the ring is rung — ★★★★★ w292, the C's own invariant (\"a \
                     mapping is always backed before the engine that uses it runs\"). \
                     `[measured w291, boot w290pboth]` the faulting leaf 0x73b1_83700000 was \
                     never published, never refused and NEVER REACHED at 256 rows/doorbell. \
                     ⊘ SCOPED: every OTHER address space keeps the 256-row sample and \
                     SYSTEM_PROC keeps its §12.26 refusal, so the budget is NOT raised across \
                     the board. ⚠ Bounded twice (65536 rows / 3000 ms) and BOTH bounds print \
                     `complete=false` when they fire — a row left unpinned is UNREACHED, not \
                     refused",
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
            operand_join,
            vas_publish,
            // ★ w318 — empty. The gate's first consultation on any key always ARMS, so a
            // fresh port cannot skip work it has never done.
            dirty: DirtyGate::default(),
        }));
        Ok(Regs {
            plane,
            // ⊘ Read ONCE, here, at the composition root — an arming flag consulted twice
            //   is a boot that can change its mind halfway through.
            reclaim: Arc::new(crate::reclaimtick::ReclaimTick::from_env()),
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
            last_pin_reclaim: std::sync::atomic::AtomicUsize::new(0),
            max_reap_us: std::sync::atomic::AtomicU64::new(0),
            max_drain_us: std::sync::atomic::AtomicU64::new(0),
            last_deferred_for_drain: std::sync::atomic::AtomicUsize::new(0),
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
            // ★★★ The plane and the latched GR channels, so this thread can sample
            // `GP_GET`/`GP_PUT` LATE. See [`GrCursorWatch`] for why a doorbell-time read
            // cannot answer the question. ⊘ The SAME plane handle the device answers
            // registers from — cloned, never rebuilt — so the store this reads and the store
            // the descent reads cannot be two stores.
            let plane = std::sync::Arc::clone(&self.plane);
            let gr_cursors = std::sync::Arc::clone(&self.ce.gr_cursors);
            // ★★★★★ w326 — the two handles the revocation tick needs. Both are ALREADY
            // `Arc`s on `Regs`, which is why this driver costs a clone and not a refactor.
            let reclaim_driver = ReclaimDriver {
                device: std::sync::Arc::clone(&self.device),
                tick: std::sync::Arc::clone(&self.reclaim),
            };
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let stop_thread = std::sync::Arc::clone(&stop);
            let join = std::thread::Builder::new()
                .name("kayfabe-completion-observer".into())
                .spawn(move || {
                    observer_loop(
                        &mut reactor,
                        &watch,
                        &stop_thread,
                        vmm,
                        &plane,
                        &gr_cursors,
                        &reclaim_driver,
                    );
                })
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
        // ★★★★★ w315 — THE FINAL CENSUS, printed BEFORE anything is torn down.
        //
        // ⊘ It is not the only census: `kftime::record` prints a running one every
        // `KAYFABE_KFTIME_CENSUS_EVERY` events, so a boot that is killed (143), whose log is
        // truncated, or that never reaches teardown still carries a complete breakdown in
        // whatever prefix survives. `143` and a truncated artefact both READ AS PRESENT —
        // this line existing is not evidence that it ran.
        crate::kftime::report_all("detach_ram");
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
    ///
    /// ★★★★★ **w315 — A READ IS A VMEXIT TOO, and this bench makes that expensive.**
    ///
    /// `[measured 2026-08-14]` the bench host is itself a KVM guest
    /// (`systemd-detect-virt` → `kvm`, `hypervisor` in `/proc/cpuinfo`, nested KVM present),
    /// so our guest runs at **L2** and every MMIO access is a **nested** vmexit
    /// (L2 → L1 → L0). The C artifact attributes a 2.5× throughput gap to exactly this
    /// (`C: docs/MILESTONES.md:12-14` — llama.cpp at 49.9 tok/s on bare metal against 20 on
    /// vast, *"entirely nested-virt vmexit tax"*).
    ///
    /// ⇒ **The exit COUNT matters more than the per-exit handler time**, and a doorbell-only
    /// instrument cannot see it: a guest that *polls* a status register spends its time in
    /// reads, and reads outnumber writes on every driver path this device has. Bracketing
    /// only writes would have reported a fast write path beside an unexplained floor —
    /// outcome (D) arriving disguised as a mystery.
    ///
    /// ⊘ The vmexit itself is **outside** this bracket by construction: it has already
    /// happened when this function is entered. Trap-shaped cost therefore lands in the
    /// analyser's `UNACCOUNTED` row and is bounded by `exits × per-exit cost`, never by a
    /// segment here. See `crate::kftime::segment_shape`.
    #[must_use]
    pub fn read(&self, bar: u32, off: u64, size: u32) -> u64 {
        let mut kft = crate::kftime::Segs::start();
        let v = self
            .plane
            .read(clamp_bar(bar), off, clamp_size(size))
            .value();
        kft.mark("plane_read");
        crate::kftime::record_hot("mmio_read", bar, off, kft.total_us());
        // ⊘ NEVER per-event printed, whatever the arming: reads are the hot path and ~900
        // doorbell lines is an instrument, while ~10^5 read lines is a second workload.
        // `record` honours `per_event`, so this kind is filtered at the call site instead.
        crate::kftime::record_quiet("mmio_read", &mut kft);
        v
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
    /// ★★★★★ **w288 — WHERE EACH ABOUT-TO-BE-BORN HOST CHANNEL'S ERROR NOTIFIER LIVES.**
    ///
    /// One entry per latched engine-object forward whose channel declared a notifier in
    /// guest RAM that this hypervisor's own stated layout can describe. The drain applies
    /// them by key; a latch with no entry gets a channel with **no** notifier, which is the
    /// pre-w288 behaviour and never somebody else's pages.
    ///
    /// # ⊘ Why HERE, at the peek, and not inside the drain
    ///
    /// Two reasons, and both are structural. **(1)** Only the VMM may derive a
    /// [`kayfabe_isolate::GuestRamGrant`] — the file offset comes from *this* object's stated
    /// layout, and nothing below this crate can compute one. **(2)** This is the last instant
    /// at which *"a host channel is about to be born for guest channel X"* is knowable and X
    /// is still un-born, which is [`Regs::adopt_pending_channel_rings`]' own ordering
    /// argument: `hObjectError` is a **birth** parameter, so an object minted after the drain
    /// could never reach the channel that needs it.
    ///
    /// ⊘ **Takes nothing and mutates nothing.** A caller that drained here to inspect the
    /// entries would have run the forwards it meant to prepare for.
    ///
    /// # R1
    ///
    /// `RegPlane::write` has returned, so this frame holds no ranked lock.
    /// `engine_object_channel_facts` is a routed **read** — no host verb, no ioctl — and the
    /// plane's rank-0 mutex is taken and released inside `err_notifier_grant`, around one
    /// layout resolution and nothing else.
    fn pending_err_notifier_grants(&self) -> Vec<kayfabe_rt::device::EngineNotifierGrant> {
        // ⊘ Silent and free when the crossing is not armed, for `err_notifier_grant`'s
        // reason: a control's log must not contain a line the armed run's does not.
        if self.guest_ram_backing.is_none() {
            return Vec::new();
        }
        let pending = self.device.peek_pending_engine_forwards();
        if pending.is_empty() {
            // The overwhelmingly common case — this runs on every register write.
            return Vec::new();
        }
        let mut out = Vec::new();
        for (client, parent, class) in pending {
            // ⊘ A latch this port cannot route is NOT a miss and is NOT reported here: the
            // drain refuses it too, and by its own name. Printing a second refusal for one
            // cause is how one defect comes to read as two.
            let Ok(facts) = self
                .device
                .engine_object_channel_facts(client, parent, class)
            else {
                continue;
            };
            if let Some(grant) = err_notifier_grant(
                &self.ce,
                self.guest_ram_backing,
                facts.error_notifier,
                &format!(
                    "latch client={:#x} parent={:#x} class={:#06x} proc={} chan={}",
                    client.0, parent.0, class.0, facts.proc.0, facts.chan.0
                ),
            ) {
                out.push(kayfabe_rt::device::EngineNotifierGrant {
                    client,
                    parent,
                    class,
                    grant,
                });
            }
        }
        out
    }

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
        // ★★★★★ **w315 — THE OUTER BRACKET, and it is the one that matches what the GUEST
        // measures.** A guest MMIO write is a vmexit: the vCPU is halted for the whole of
        // this function, so `total_us` here is time the guest cannot be doing anything else.
        // ⇒ this is the only host-side number that can be compared to a guest-side latency
        // WITHOUT converting between the two clocks — the containment is structural.
        //
        // ⊘ It brackets EVERY trapped register write, not only doorbells. w311's floor is
        // per-SUBMIT and nobody has shown the submit is the doorbell; if the ~100 ms lives in
        // a GSP RPC poke or some other register, a doorbell-only instrument would report a
        // fast doorbell and a mystery, which is the (D) outcome arriving disguised as (C).
        let mut kft = crate::kftime::Segs::start();
        let out = self.plane.write(clamp_bar(bar), off, clamp_size(size), val);
        kft.mark("plane");
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
        kft.mark("materialize");
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
        kft.mark("ring_adopt");
        // ★★★★★ **w288 — AND IT MUST ALSO BE THE LINE ABOVE THE DRAIN**, for leg A1's exact
        // reason one field over: the drain births the host channel, and `hObjectError` is a
        // birth parameter. See [`Regs::pending_err_notifier_grants`]. ⊘ Ordering, not
        // preference.
        let err_notifier_grants = self.pending_err_notifier_grants();
        kft.mark("err_grants");
        report_engine_forward_drain(&self.device, &err_notifier_grants);
        kft.mark("fwd_drain");
        // ★★★★★ **w303 — THE REAP, AND THIS LINE IS THE WHOLE OF FIX A.**
        //
        // `docs/audits/w301_cancellation_error_leaks.md` §3.1: the per-object teardown chain
        // (`plan_refresh` → `Spine::vacate` → `Proc::retire` → `self.retired` → `Proc::drop`
        // issuing real `Release` verbs) is **built, tested and correct**, and its only
        // non-test caller was `kayfabe_rt::Executor` (`executor.rs:84`), which has **zero
        // production call sites**. ⇒ under QEMU a dead guest process's whole host RM object
        // tree — client, device, subdevice, VAS, TSG, channels, USERD, ctx buffers,
        // `OS_DESCRIPTOR` pins — and its **isolate child process** survived until QEMU
        // exited, and at `MAX_RETIRED_PROCS = 1024` (`kayfabe_core::gpu`) no new guest
        // process could be derived at all. BUILT + ORPHANED; the fix is a composition-root
        // line, not a design.
        //
        // # ⊘ Why HERE, and not at the client-root free
        //
        // `Spine::reap_retired`'s own docs carry the C's P0 lesson (L10): *"reaping the
        // heavy tables AT the client-root free hung the dying context's residual polls, so
        // it reaps at the GSP queue re-handshake instead"*. The core therefore splits
        // **retire** (eager, inside the apply, under the lock) from **reap** (deferred, at
        // an *adapter-declared* quiesce point) — and declaring that point is precisely the
        // obligation this crate had never discharged. `Regs::write` is the shim's
        // re-handshake edge: the guest's `GSP_RM_FREE` chain, the fn-47 idle release and the
        // status-queue re-publish all arrive as register writes, and the retirement they
        // cause is latched inside `RegPlane::write` **on this very call**.
        //
        // # ★ Why it is SAFE here, by the same argument the two drains above already make
        //
        // `RegPlane::write` has returned, so the plane's rank-0 guard is a dropped local and
        // this frame holds **no ranked lock at all**. That matters more for the reap than
        // for the drains: `SharedDevice::reap_retired` releases the device write guard and
        // *then* drops the corpses, whose `Drop` is `waitpid` + namespace teardown — a
        // blocking syscall. `IsolateBox`'s own `Drop` asserts lock-freedom, so if any of
        // this is wrong it is refused **by name, here** (§12.16 G3b).
        //
        // # ⚠ It RETRIES, and that is why one line is enough
        //
        // A proc with a worker still checked out is **not** quiesced; `Spine::reap_retired`
        // puts it straight back on the list (§12.16 G3) and it is reaped at a later quiesce
        // point. Because this edge is every register write, "a later quiesce point" is the
        // guest's next MMIO write — there is no deadline to arm and no thread to own it.
        //
        // ⊘ Cost: one device write-lock acquisition per register write, which is the cost
        // class `materialize_pending` above already pays on this same frame; when
        // `self.retired` is empty the body is a `mem::take` of an empty `Vec`.
        // ★★★★★ **w310 — AND THE PIN RELEASE IS WITNESSED ON THE SAME EDGE.**
        //
        // No new call: the guest-RAM pin release is staged inside `Spine::stage_dropped_vases`,
        // which the refresh already runs, and the staged `Orphans` ride out on the proc's own
        // next worker checkout (`checkout_and_drain`) or on `Proc::drop` at the reap below.
        // ⇒ **the release costs this frame ZERO extra blocking work**; what is added here is
        // the *number*, because `docs/audits/w301_cancellation_error_leaks.md` §3.2 records
        // exactly the shape where a release path exists and nobody can tell whether it ran.
        //
        // ⚠ Printed only on CHANGE, not every register write: this edge is every guest MMIO
        // write, and a per-write line would be the `a_recorder_that_prints_at_teardown` trap
        // pointing the other way — a log so dense the fact is unfindable.
        // ★★★★★ **w326 — THE GATE.** `w323` found this edge is the drain's ONLY driver;
        // `crate::reclaimtick` gives it a second one on our own thread, and the two must
        // never run together — `drain_retired_budgeted` issues its host verbs with no lock
        // held, so two concurrent drains could plan and free the same retired object twice.
        //
        // ⊘ `try_claim_on_trap`, and there is no blocking method on that type at all: a
        // vCPU that waited here would stop every vCPU and QEMU's main loop until a DIFFERENT
        // thread finished host I/O — `INLINE-SAFE` clause (a) violated by construction.
        // Missing the gate is safe and is not a dropped drain: whoever holds it is spending
        // the same queue right now.
        //
        // ⊘ Disarmed, nothing else ever takes the gate, so this always succeeds and the
        // whole block below is byte-identical to master.
        //
        // ⚠ A BLOCK, not an early return: the `kftime` records below this block must run on
        // EVERY trap. An early return would drop the skipped traps out of the timing census
        // entirely, so the arm that skips more would look like the arm with fewer traps.
        if let Some(_reclaim_gate) = self.reclaim.try_claim_on_trap() {
        let pins = self.device.pin_reclaim_gone();
        let total = pins.released + pins.refused_no_host_vas + pins.rows_deduped;
        if total
            != self
                .last_pin_reclaim
                .swap(total, std::sync::atomic::Ordering::Relaxed)
        {
            eprintln!(
                "kayfabe: PIN-RELEASE released={} refused_no_host_vas={} rows_deduped={} \
                 ⇒ that many guest-RAM `OS_DESCRIPTOR`s freed, GPU VAs unmapped and isolate \
                 `mmap` windows `munmap`ed at VAS death, instead of held until QEMU exits",
                pins.released, pins.refused_no_host_vas, pins.rows_deduped,
            );
        }
        // ★★★★★ **w317 — THE BUDGETED DRAIN, AND IT MUST BE THE LINE ABOVE THE REAP.**
        //
        // Ordering, not preference. The reap holds a proc back while it still has drainable
        // staged work (`Spine::reap_retired`'s w317 gate), so this line is what eventually
        // lets the reap below take anything at all. Reversed, the first trap after a proc
        // vacates would reap it with a full queue and `Proc::drop` would issue the whole
        // thing — the 2.65–3.70 s stall w314 measured, unchanged.
        //
        // ⊘ **This is not a new call site in the `INLINE-SAFE` sense.** The verbs it issues
        // are the same verbs `Proc::drop` issued from this same frame at master; what is new
        // is that a bounded number of them run per trap instead of all of them. It holds no
        // ranked lock while executing (R1, asserted inside `Worker::execute`), for the same
        // reason and by the same construction as the reap below.
        //
        // ⚠ The deadline is read BETWEEN turns, so the bound delivered is
        // `RETIRED_DRAIN_BUDGET_US` + one chunk — see both constants' docs.
        let drain_t0 = std::time::Instant::now();
        let drain = self.device.drain_retired_budgeted(RETIRED_DRAIN_CHUNK, || {
            u64::try_from(drain_t0.elapsed().as_micros()).unwrap_or(u64::MAX)
                >= RETIRED_DRAIN_BUDGET_US
        });
        let drain_us = u64::try_from(drain_t0.elapsed().as_micros()).unwrap_or(u64::MAX);
        if drain.turns > 0
            && drain_us
                > self
                    .max_drain_us
                    .fetch_max(drain_us, std::sync::atomic::Ordering::Relaxed)
        {
            // ★ Printed on a NEW MAXIMUM only — the density rule the two lines above already
            // carry. `budget_hit` is on the line because "we stopped early and the rest rides
            // to the next trap" is the mechanism working, and a reader must be able to tell
            // it from "there was nothing left".
            eprintln!(
                "kayfabe: DRAIN-TIMING max_drain_us={drain_us} disposed={} residue={} \
                 turns={} budget_hit={} ⇒ the longest BUDGETED disposal yet inside \
                 Regs::write, with the BQL held. Budget: {RETIRED_DRAIN_BUDGET_US} us \
                 (1% of scrubberDestruct's 4000000 us) + one {RETIRED_DRAIN_CHUNK}-disposal \
                 chunk of overshoot.",
                drain.disposed, drain.residue, drain.turns, drain.budget_hit,
            );
        }
        // ★★★ **w314 — TIME THE DISPOSAL.** See [`Regs::max_reap_us`]. This wraps the call
        // and changes nothing about it: two `Instant`s and a `fetch_max`.
        let reap_t0 = std::time::Instant::now();
        // ⊘ MERGE NOTE — THREE lanes converged on this one call, and none of the conflicts
        // was semantic:
        //   w314 times the disposal against the 4 s `scrubberDestruct` budget (`max_reap_us`),
        //   w315 closes the `reap` segment of the per-doorbell breakdown (`kft.mark`),
        //   w317 REPLACES the call itself — `reap_retired_held` holds a proc back until its
        //        queue empties, which is what took the worst hold from 3.70 s to 54.8 ms.
        // w317's call wins because it is the behaviour change; both instruments are kept
        // because they measure different things and w317 is the reason they now read small.
        // ★ The ORDER is load-bearing and is the only thing the merge had to decide: read
        // the elapsed time and close the segment FIRST, then print. Printing before either
        // would charge w314's `eprintln!` to w315's `reap` segment AND to w314's own
        // `max_reap_us` — an instrument billing itself to the thing it measures, which is the
        // failure class this tree has now paid for several times over.
        // ⚠ And per w317: `max_reap_us` NO LONGER MEANS THE DISPOSAL. What is left here is the
        // isolate child's `waitpid` + namespace teardown (47–54 ms), a floor no budget touches.
        // Read it beside `max_drain_us`, never added to it — they occur on different traps.
        let (reaped, deferred_for_drain) = self.device.reap_retired_held();
        let reap_us = u64::try_from(reap_t0.elapsed().as_micros()).unwrap_or(u64::MAX);
        kft.mark("reap");
        if reap_us
            > self
                .max_reap_us
                .fetch_max(reap_us, std::sync::atomic::Ordering::Relaxed)
        {
            eprintln!(
                "kayfabe: REAP-TIMING max_reap_us={reap_us} reaped={reaped} \
                 ⇒ the longest BLOCKING disposal yet inside Regs::write, with the BQL held \
                 and every vCPU halted. Budget: scrubberDestruct = 4000000 us."
            );
        }
        if reaped > 0 {
            // ★ Visible in the boot log, because "the reap ran" is a claim this tree has
            // twice mistaken for "the reap exists". A zero prints nothing; a non-zero says
            // so once per reap, with what is still outstanding beside it.
            eprintln!(
                "kayfabe: REAP reaped={reaped} still_retired={} deferred_for_drain=\
                 {deferred_for_drain} ⇒ each reaped proc's staged host `Release` verbs went \
                 out and its isolate child was reaped; `deferred_for_drain` are procs held \
                 back because w317's budgeted drain has not emptied their queue yet",
                self.device.retired_len(),
            );
        }
        // ★★★★★ **w317 — THE TRAJECTORY, and it prints on every TRANSITION.**
        //
        // A bound that defers indefinitely is a leak with extra steps. `Spine::reap_retired`'s
        // termination argument (the queue of a retired proc is CLOSED and monotonically
        // decreasing) says this must return to 0; this line is what makes that argument
        // **checkable on a live boot** rather than a paragraph. ⚠ Absent = UNMEASURED: a boot
        // where no proc ever vacated prints nothing, and that is a different fact from
        // "nothing was ever deferred".
        if deferred_for_drain
            != self
                .last_deferred_for_drain
                .swap(deferred_for_drain, std::sync::atomic::Ordering::Relaxed)
        {
            eprintln!(
                "kayfabe: DRAIN-DEFER deferred_for_drain={deferred_for_drain} \
                 still_retired={} ⇒ procs the reap is holding back because their staged \
                 disposal queue is not empty yet. MUST return to 0: the queue of a retired \
                 proc is closed and strictly decreasing, so a value that never falls is the \
                 budget having moved the cost rather than removed it",
                self.device.retired_len(),
            );
        }
        // ⊘ Two kinds, deliberately. A doorbell write and an ordinary register write live in
        // the same census only as `mmio_all`; splitting them means *"the trap is slow"* and
        // *"the doorbell is slow"* are separate readings, and w311's floor is compatible with
        // either. `doorbell` is decided by the plane, so it is read off the outcome rather
        // than re-derived from the offset — two projections of one fact that disagree is this
        // campaign's most expensive failure class.
        }
        crate::kftime::record_hot(
            if out.doorbell.is_some() {
                "mmio_doorbell"
            } else {
                "mmio_other"
            },
            bar,
            off,
            kft.total_us(),
        );
        crate::kftime::record(
            if out.doorbell.is_some() {
                "mmio_doorbell"
            } else {
                "mmio_other"
            },
            &mut kft,
        );
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
        // ★★★★★ **w326 — THE GUEST'S OWN TLB INVALIDATE.** Printed directly under the
        // unclaimed census, because until this rung `0xb830b0`, `0xb830a0` and `0xb830a4`
        // were three rows *of that list* — the guest's exact publish boundary, answered
        // with a defaulted zero, on every boot since M5. Three campaigns concluded from
        // two other transports' measured zeros that no such signal existed.
        //
        // ⊘ Printed unconditionally, armed or not, and every number anchored: an absent
        // line is UNMEASURED and a present line with `triggers=0` is a measured zero.
        // Those are different facts and this tree has paid for confusing them.
        eprintln!("kayfabe: {}", self.plane.mmu_inval().census());
        // ★★★★★ w326 — did the revocation drain get a driver that is not the guest?
        eprintln!("kayfabe: {}", self.reclaim.census());
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
/// # ⊘⊘⊘ CORRECTED 2026-08-13 (w284) — THE "TRANSPORT, NOT EXECUTION" CLAIM BELOW IS REFUTED
///
/// The paragraph below says the host GR channel's ring **and** its `GP_PUT` are both ours,
/// *"so the engine fetches nothing on either arm"*. **Legs A2 and B moved that**, and the
/// refutation has been recorded at `shim.rs:5745` since 2026-08-12 — but it was never
/// propagated **here**, into the doc a reader of the flag hits first, and a `w284` brief was
/// written from the stale half.
/// `[measured, w267_on, all 16 `GR-BIRTH iso2` lines]` every birth — eight `engine=GrCompute`
/// and eight `engine=Ce` — reads `adopt=GUEST-RING userd=GUEST-USERD →
/// alloc_channel_over_guest_ring`: the host channel's ring **is** the guest's, and its
/// `GP_PUT` **is** the word in the guest's own USERD page. `[w263]` `GET=1 PUT=1` on the
/// armed arm against `GET=0 PUT=1` on the control, at the same address in the same boot pair.
/// ⇒ Read the paragraph below as the *reason this flag was introduced*, not as a description
/// of what the armed arm does today. See `docs/design/ce_passthrough_is_already_built.md`.
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

/// ★★★★★ **w282 — whether a CE operand page that lands in OUR EMULATED FRAMEBUFFER has its
/// leaf JOINED, so a real host engine can be pointed at the guest's own number.**
///
/// | value | what it does |
/// |---|---|
/// | `off` (default) | today's behaviour, byte for byte. Not one `OPERAND-JOIN` line |
/// | `join` | ★ every operand page [`SharedDoorbell::ce_operand_pages`] decodes that resolves to a **framebuffer** binding has its 64 KiB leaf put through `join_one_fb_leaf` — the SAME four steps the ring source and the GR operand census already use |
///
/// # ★★★ WHY, and it is a CALLER GAP rather than a missing primitive
///
/// `[measured 2026-08-12, w281_client, real GA106]` with the pushbuffer route on, a real host
/// copy engine **fetched and executed** the guest's own methods and faulted
/// `Xid 31 ENGINE CE0 HUBCLIENT_CE1 … FAULT_PTE ACCESS_TYPE_VIRT` at the destination operand
/// the guest's own pushbuffer declared. `[measured, w281b_clientsweep]` arming the whole-VAS
/// sweep bound both operand VAs — `2 MISS → 0 MISS` — and they resolved to **`Vidmem`**, our
/// fabricated framebuffer, which [`kayfabe_fwd::Representability::Fabricated`] routes to
/// `CeExecutor::Ours`, which `HostRmBackend::ce_copy` refuses by name under a standing owner
/// ruling. ⇒ **Both reachable configurations are walls**, and both are the same missing thing:
/// the operand lives in memory no real engine can resolve.
///
/// ⊘ **The join that fixes it is already built and is simply not called here.**
/// `Regs::back_census_framebuffer_leaves` joins exactly these leaves off the **operand
/// census** — but it is reached only from `SharedDoorbell::declare_gr_completion`, which
/// `SharedDoorbell::ring` calls on the two **GR** dispositions (`HandToCore` and
/// `RefuseByRoute`) and on **no CE path at all**. A CE doorbell's operands therefore reach
/// `Self::pin_operand_guest_ram`, which refuses a framebuffer binding by name with the
/// sentence *"that memory is ours already and needs no descriptor"* — true of the CPU
/// executor and, since `w281`, **measured false of a host engine**.
///
/// ★ So this arm adds **no primitive, no verb and no new authority**: it presents the CE
/// plane's operand leaves to the join the GR plane has been using since `w260`.
///
/// # ⊘ WHAT AN ARMED LINE STILL DOES NOT MEAN
///
/// *"The operand leaf is one memory with a real host object"* and *"the submission retired"*
/// are different facts. This arm produces the first. ⚠ And the join's own scope is unchanged:
/// it is per-VAS by construction — the leaf is walked from **this channel's own installed PDB
/// root** and bound into **this `Pdb`'s** table — so an operand can never name a page reachable
/// only from another address space's root. That is `mode2_address_table.md` §3/§6 and it is
/// asserted rather than assumed; see [`SharedDoorbell::join_operand_fb_leaves`].
pub const OPERAND_JOIN_ENV: &str = "KAYFABE_OPERAND_JOIN";

/// ★★★★★ **w290 — THE WHOLE-VAS PUBLICATION.** See [`VasPublishArm`].
///
/// `[measured, boot w290cup2]` `HOST-PUBLISHED host_rows=4 of 16425` in cup2's own address
/// space, `0 of 533` and `0 of 6254` in the other two. We populate a shadow and never
/// materialize it, so hardware walks an empty host VAS and misses **above the leaf**
/// (`FAULT_PDE`). This arm presents **every qualifying row of every live `Vas`** to the same
/// `join_one_fb_leaf` chain leg 7 already uses — no new primitive, no new verb, no new
/// authority, exactly as [`OPERAND_JOIN_ENV`]'s own doc argues for its leg.
///
/// # ⚠ TWO CONSEQUENCES, PRE-REGISTERED BECAUSE THEY ARE NOT HYPOTHETICAL
///
/// - **A published row becomes immutable to the guest's own page-table edits.** The decoder
///   refuses rather than acting: `PopulateRefusal::RepointsPublished`
///   (`kayfabe-mmu/src/walker.rs:917-930`) and `UnbindsPublished` (`:956-972` — *"Unpublishing
///   needs a worker and an unmap verb … So the refusal is the answer, and the binding
///   stays"*). ⇒ publishing widely converts guest re-mappings into refusals, and **both
///   counters already print on the doorbell line**, so this boot measures its own cost.
/// - **Reclaim is by VAS/proc teardown only.** That is sufficient and it already exists —
///   see the pass's own doc — but there is no per-leaf release short of it.
pub const VAS_PUBLISH_ENV: &str = "KAYFABE_VAS_PUBLISH";

/// Which arm of the CE operand-leaf join a boot is running. See [`OPERAND_JOIN_ENV`].
///
/// # ⊘⊘ WHY THERE ARE THREE ARMS AND NOT TWO — a defect this rung's OWN control found
///
/// `[measured 2026-08-13, boot `w282_clientoff`]` the first draft had two arms and put the
/// `#255` assertion **inside** the armed path. The control therefore printed **zero** `#255`
/// lines — so the instrument's *guaranteed known-positive was unreachable*, and a `QUIET` and
/// a *"never ran"* were the same observation. That is the exact shape
/// `a_census_zero_needs_a_known_positive` and `a_feature_gate_with_a_silent_noop_sibling` name,
/// caught by the control rather than by reading.
///
/// ⇒ [`OperandJoinArm::Assert`] exists so the control **runs the instrument and joins
/// nothing**. It is the control this rung compares against, and its expected reading is
/// `#255 … FIRED`, which is a POSITIVE observation rather than an absence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperandJoinArm {
    /// The default: **silent**, byte-identical to every boot before `w282`. Not one
    /// `OPERAND-JOIN` or `#255` line, and no second read of the ring.
    Off,
    /// ★★★ **CLASSIFY AND ASSERT, JOIN NOTHING** — the rung's control. Every CE operand page
    /// is resolved per-VAS and classified, and `#255` states its verdict; no leaf is joined
    /// and no host verb is issued. ⊘ Behaviourally this is `Off` plus printing, so it
    /// reproduces `w281b_clientsweep` while making the instrument's known-positive visible.
    Assert,
    /// ★ Everything `Assert` does, **and** every framebuffer leaf a CE operand names is
    /// joined.
    Join,
}

impl OperandJoinArm {
    /// Every arm, so a test can quantify rather than restate.
    pub const ALL: [OperandJoinArm; 3] = [
        OperandJoinArm::Off,
        OperandJoinArm::Assert,
        OperandJoinArm::Join,
    ];

    /// One word, for the boot's own log.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            OperandJoinArm::Off => "off",
            OperandJoinArm::Assert => "assert",
            OperandJoinArm::Join => "join",
        }
    }

    /// Whether the pass runs at all — i.e. whether operands are decoded, classified and
    /// put to `#255`. ⊘ True on `Assert` as well as `Join`: that is the whole point of the
    /// third arm.
    #[must_use]
    pub fn observes(self) -> bool {
        self != OperandJoinArm::Off
    }

    /// Whether a framebuffer-resident CE operand leaf is actually joined on this arm.
    #[must_use]
    pub fn joins(self) -> bool {
        self == OperandJoinArm::Join
    }
}

/// Which arm `value` names — the pure half of [`selected_operand_join`].
///
/// # Errors
/// [`Status::Unsupported`] if `value` names no arm. **Absent is not an error**; it is
/// [`OperandJoinArm::Off`].
pub fn operand_join_from(value: Option<&str>) -> Result<OperandJoinArm, (Status, &'static str)> {
    match value {
        None | Some("off") => Ok(OperandJoinArm::Off),
        Some("assert") => Ok(OperandJoinArm::Assert),
        Some("join") => Ok(OperandJoinArm::Join),
        Some(_) => Err((
            Status::Unsupported,
            "KAYFABE_OPERAND_JOIN does not name an arm: the only values are `off` (silent, \
             byte-identical to every boot before w282), `assert` (THE CONTROL — classify every \
             CE operand per-VAS and state #255's verdict, join NOTHING, issue no host verb; \
             its expected reading is `#255 … FIRED`, which is a POSITIVE observation rather \
             than an absence) and `join` (everything `assert` does, plus the framebuffer leaf \
             goes through the same four-step join the ring source and the GR operand census \
             already use, so the guest's window and a real host object are ONE memory and the \
             executor stays HostCe). It is not defaulted, because a typo that silently \
             disarmed the join would make an evidence run and its own control \
             indistinguishable. ⊘ `on`/`1` are not accepted: this is a three-arm experiment, \
             not a boolean.",
        )),
    }
}

/// Which arm [`OPERAND_JOIN_ENV`] names.
///
/// # Errors
/// [`Status::Unsupported`] for a value that names no arm, **including a non-UTF-8 one** —
/// which takes the `Some` arm, because it was SET and must not read as unset.
fn selected_operand_join() -> Result<OperandJoinArm, (Status, &'static str)> {
    match std::env::var_os(OPERAND_JOIN_ENV) {
        None => Ok(OperandJoinArm::Off),
        Some(v) => operand_join_from(Some(v.to_str().unwrap_or("\u{fffd}invalid"))),
    }
}

/// ★★★★★ **w290 — WHICH ARM OF THE WHOLE-VAS PUBLICATION a boot is running.**
///
/// `w290` measured `HOST-PUBLISHED host_rows=4 of 16425` in cup2's own address space, and
/// `0 of 533` / `0 of 6254` in the other two: **we populate our shadow and never materialize
/// it host-side**, which is why hardware faults `FAULT_PDE` — no page *directory* exists
/// within a terabyte of the fault. This arm publishes the guest's declared rows through the
/// same proven chain the CE-operand join uses.
///
/// ⊘ Three arms, not a boolean, for [`OperandJoinArm`]'s reason exactly: the census is the
/// measurement and it must be readable **without** any host verb having run, or a boot that
/// publishes nothing and a boot that was never armed are the same log.
///
/// ⊘ It is **six** arms now, not three, and each new one was added because it needed its own
/// control rather than because it was a bigger version of the last: `pinrate` measures a
/// second chain over a disjoint population, `both` runs the two together, and `drain`
/// ([`VasPublishArm::Drain`], w292) changes exactly one variable against `both`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VasPublishArm {
    /// Silent. Byte-identical to every boot before `w290`.
    Off,
    /// ★★★ **THE CONTROL.** Census every `Vas` and print the classification by name — how
    /// many rows are already host-backed, guest RAM, non-`Vidmem`, refused by RM's 64 KiB
    /// granularity, and how many qualify — but issue **no host verb**. Its expected reading
    /// is a non-zero `candidates=` beside `published=0`, which is a positive observation and
    /// not an absence.
    Assert,
    /// ★★★★★ Everything `assert` does, plus every qualifying row goes through
    /// `join_one_fb_leaf` — the identical four-step chain that moved 4096 correct bytes on
    /// the `w289` CE arm.
    Publish,
    /// ★★★★★ **w291 — THE BOUNDED PIN-RATE MEASUREMENT, AND IT IS NOT THE MERGE.**
    ///
    /// `guest_ram_publication_merge.md` reported option (2a) — one host object per row —
    /// as costing *"~49 s per VAS"*. ⊘ **That was an EXTRAPOLATION of leg 8's FRAMEBUFFER
    /// rate (34 pins in 101 ms) onto 16 328 guest-RAM rows, and a framebuffer join and a
    /// guest-RAM pin are different chains.** This arm replaces the extrapolation with a
    /// measurement: pin [`VAS_PINRATE_ROWS`] guest-RAM rows through the **existing**
    /// `pin_guest_ram` verb and report the true per-row rate, and whether it is flat or
    /// degrades with count.
    ///
    /// ⊘⊘ **It writes NOTHING into `Binding::host`, adds no representation, and does not
    /// touch `HostBacking`.** The pins land in `Vas::guest_ram_pins`, exactly where that
    /// verb has always put them. If the rate is cheap, (2a) needs no ruling at all and
    /// (2b)/(2c) are moot; if it is dear, the owner rules on a number instead of a guess.
    PinRate,
    /// ★★★★★ **w291 step 1 — BOTH HALVES IN ONE BOOT, and they never have been.**
    ///
    /// Leg 8 (framebuffer publication) was OFF on the (2a) arm and (2a)'s pinning was OFF on
    /// leg 8's, so the two have only ever been measured apart. They cover **disjoint
    /// populations** — `Vidmem` rows through `join_one_fb_leaf`, guest-RAM rows through
    /// `pin_guest_ram` — so this arm may simply **sum**, and that is pre-registered as a
    /// legitimate outcome rather than a disappointment. What it settles is whether the
    /// residual `total - host_rows` is one population or two.
    Both,
    /// ★★★★★ **w292 — EVERYTHING `both` DOES, PLUS THE DOORBELLED VAS IS DRAINED TO EMPTY
    /// BEFORE THE RING IS RUNG.**
    ///
    /// # The measurement that commissioned it, and it is a LOOKUP rather than an inference
    ///
    /// `[measured w291, boot w290pboth]` hardware faulted `FAULT_PTE ACCESS_TYPE_VIRT_WRITE @
    /// 0x73b1_83700000`, and that leaf lies **inside a run our own table describes**
    /// (`0x73b182e00000+0xd33000`). `grep -c "73b1837"` over the whole QEMU log is **0**: the
    /// pin pass never published it, never refused it, and **never reached it** — it was still
    /// draining the low region (`0x2004…`) when the boot ended, at [`VAS_PINRATE_ROWS`] = 256
    /// rows per doorbell. The residual was `guest_ram=1075`.
    ///
    /// ⇒ The distance left on that axis is a **budget number, not a defect**, and this arm
    /// spends it. The C's own invariant is *"a mapping is always backed before the engine that
    /// uses it runs"* — a statement about **completeness at the doorbell**, not about a rate.
    ///
    /// # ⊘⊘ SCOPED TO ONE VAS ON PURPOSE — THE BUDGET IS NOT RAISED ACROSS THE BOARD
    ///
    /// The measured cost is for **one** address space: 1075 rows × 276–338 µs = **0.30–0.36 s**,
    /// once. Every other `Vas` keeps the bounded [`VAS_PINRATE_ROWS`] sample it had on
    /// [`Self::Both`], so this arm changes exactly one variable against that control.
    ///
    /// ⊘ And `Gpu::SYSTEM_PROC` keeps its refusal, which is a property of the proc rather than
    /// a budget: `[measured, boot w290cup2]` proc 0 holds **6787 rows**, and attempting them
    /// prints `refused=6144` — a line that reads exactly like RM exhaustion and is nothing of
    /// the kind. If the doorbelled VAS belongs to proc 0, this arm says so and drains nothing.
    ///
    /// ⚠ It is bounded twice regardless, and both bounds announce themselves:
    /// [`VAS_DRAIN_ROW_CAP`] (a guest that grows its tables without bound) and
    /// [`VAS_DRAIN_WALL_BUDGET`] (a host that answers slowly). A row left unpinned by either
    /// is **not thereby a refused row**, and the line says which bound stopped it.
    Drain,
}

impl VasPublishArm {
    /// One word, for the boot's own log.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            VasPublishArm::Off => "off",
            VasPublishArm::Assert => "assert",
            VasPublishArm::Publish => "publish",
            VasPublishArm::PinRate => "pinrate",
            VasPublishArm::Both => "both",
            VasPublishArm::Drain => "drain",
        }
    }

    /// Whether the census runs at all.
    #[must_use]
    pub fn observes(self) -> bool {
        self != VasPublishArm::Off
    }

    /// Whether a qualifying **framebuffer** row is actually published.
    ///
    /// ⊘ `PinRate` is deliberately **not** included: it measures a different chain over a
    /// different population, and folding the two would make one line's `published=` count
    /// two mechanisms. Same reason `joined` is counted apart from `bound` one plane over.
    #[must_use]
    pub fn publishes(self) -> bool {
        matches!(
            self,
            VasPublishArm::Publish | VasPublishArm::Both | VasPublishArm::Drain
        )
    }

    /// Whether the bounded guest-RAM pin-rate measurement runs.
    #[must_use]
    pub fn measures_pin_rate(self) -> bool {
        matches!(
            self,
            VasPublishArm::PinRate | VasPublishArm::Both | VasPublishArm::Drain
        )
    }

    /// ★★★★★ **w292 — whether the VAS THIS DOORBELL IS ABOUT is drained to empty rather
    /// than sampled.**
    ///
    /// ⊘ True of [`VasPublishArm::Drain`] and of nothing else, so `both` stays byte-comparable
    /// as this rung's control. Every VAS that is *not* the doorbelled one keeps the bounded
    /// [`VAS_PINRATE_ROWS`] sample on **every** arm, including this one — the brief's
    /// *"⊘⊘ DO NOT RAISE THE BUDGET BLINDLY ACROSS THE BOARD"*, enforced by the predicate
    /// rather than by a comment.
    #[must_use]
    pub fn drains_doorbelled_vas(self) -> bool {
        matches!(self, VasPublishArm::Drain)
    }
}

/// Which arm `value` names — the pure half of [`selected_vas_publish`].
///
/// # Errors
/// [`Status::Unsupported`] if `value` names no arm. **Absent is not an error**; it is
/// [`VasPublishArm::Off`].
pub fn vas_publish_from(value: Option<&str>) -> Result<VasPublishArm, (Status, &'static str)> {
    match value {
        None | Some("off") => Ok(VasPublishArm::Off),
        Some("assert") => Ok(VasPublishArm::Assert),
        Some("publish") => Ok(VasPublishArm::Publish),
        Some("pinrate") => Ok(VasPublishArm::PinRate),
        Some("both") => Ok(VasPublishArm::Both),
        Some("drain") => Ok(VasPublishArm::Drain),
        Some(_) => Err((
            Status::Unsupported,
            "KAYFABE_VAS_PUBLISH does not name an arm: the only values are `off` (silent, \
             byte-identical to every boot before w290), `assert` (THE CONTROL — census every \
             Vas and classify every row by the gate that would refuse it, publish NOTHING, \
             issue no host verb) and `publish` (everything `assert` does, plus every \
             qualifying row goes through the same four-step join the CE operand arm uses). It \
             is not defaulted, because a typo that silently disarmed the publication would \
             make an evidence run and its own control indistinguishable. ⊘ `on`/`1` are not \
             accepted: this is a multi-arm experiment, not a boolean. `pinrate` is the w291 \
             bounded guest-RAM pin-rate MEASUREMENT — it publishes no framebuffer row, writes \
             nothing into `Binding::host`, and is not the merge. `both` runs leg 8 and the \
             guest-RAM merge together, each bounded. `drain` is `both` plus ONE scoped change: \
             the VAS this doorbell is about is drained to empty before the ring is rung, while \
             every OTHER address space keeps the same bounded sample `both` gives it.",
        )),
    }
}

/// Which arm [`VAS_PUBLISH_ENV`] names.
///
/// # Errors
/// [`Status::Unsupported`] for a value that names no arm, **including a non-UTF-8 one** —
/// which takes the `Some` arm, because it was SET and must not read as unset.
fn selected_vas_publish() -> Result<VasPublishArm, (Status, &'static str)> {
    match std::env::var_os(VAS_PUBLISH_ENV) {
        None => Ok(VasPublishArm::Off),
        Some(v) => vas_publish_from(Some(v.to_str().unwrap_or("\u{fffd}invalid"))),
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

/// ★★★★★ **The environment variable that arms the WHOLE-VAS SWEEP** — the C's `enum_gr_sysmem`
/// (`C: nvkvm_gpu_emul.c:583-591`), driven from the doorbell.
///
/// # ⊘⊘ It arms a RELAXED CORRECTNESS GATE, not merely an instrument
///
/// Every other arm in this file changes what the port *observes* or *supplies*. This one changes
/// what the port is willing to **bind**: with it on, a leaf binds because a walk from the address
/// space's own installed page-directory root reached it, rather than because the guest was seen
/// to write its page. See [`kayfabe_mmu::reach::ReachShadow::witness_swept`] for the argument and
/// for the residual the owner accepted on 2026-08-12.
///
/// ⊘ **Off by default and refusing an unknown value.** With it unset this port binds exactly what
/// it bound before the sweep existed, so the disarmed boot **is** the negative control — and a
/// typo must not be able to produce one silently.
pub const PT_SWEEP_ENV: &str = "KAYFABE_PT_SWEEP";

/// ★★★★★ **w318 — arm the DIRTY GATE on the publication pass.** See [`DirtyGate`] for the
/// measurement, the C's precedent and the correctness argument.
///
/// ⊘ **Off by default**, for the same reason `KAYFABE_PT_SWEEP` is: the ungated boot is this
/// rung's negative control and must remain byte-comparable, and a typo must not be able to
/// make a *correctness-relevant* pass stop running. ⚠ This one is the more dangerous
/// direction of the two — arming it makes work **not happen** — which is exactly why it is
/// opt-in and why an unparseable value reads as `off`.
pub const DIRTY_GATE_PUBLISH_ENV: &str = "KAYFABE_DIRTY_GATE_PUBLISH";

/// ★★★★★ **w318 — arm the DIRTY GATE on the executor page-table witness.** Same defaults and
/// same argument as [`DIRTY_GATE_PUBLISH_ENV`]; separate variable so a boot can ablate one
/// gate at a time and the log always says which arm it ran.
pub const DIRTY_GATE_WITNESS_ENV: &str = "KAYFABE_DIRTY_GATE_WITNESS";

/// Whether `value` arms a w318 dirty gate — the pure half of [`selected_dirty_gate`].
///
/// # Errors
/// [`Status::Unsupported`] if `value` names neither state. **Absent is not an error**; it is
/// `false`.
pub fn dirty_gate_from(value: Option<&str>) -> Result<bool, (Status, &'static str)> {
    match value {
        // ★★★★★ w330 — DEFAULT MOVED off → ON, on measurement.
        // `[w330, KFTIME mmio_doorbell, 400 events, interleaved, one binary]` median
        // 18 741 → 2 197 us (8.5x), p90 86 104 → 4 431 us (19.4x) — reproducing w318's
        // 85.248 → 4.078 ms on different hardware, driver build and guest kernel.
        // `^CUP3_VAL=43` held on every armed boot.
        // ⊘ The MAX is UNCHANGED (2.75 s → 2.82 s): this gate does not touch the worst
        //   trap. That one is the pin drain and it is `KAYFABE_DRAIN_BATCH`'s. The two act
        //   on DIFFERENT STATISTICS of one distribution and neither is sufficient alone.
        None | Some("on") => Ok(true),
        Some("off") => Ok(false),
        Some(_) => Err((
            Status::Unsupported,
            "a KAYFABE_DIRTY_GATE_* variable does not name a state: the only values are `off` \
             (the default) and `on`. It is not defaulted, because the ungated arm IS w318's \
             negative control AND because the armed arm makes a correctness-relevant pass STOP \
             RUNNING on a clean doorbell — a typo that silently armed it would skip a \
             publication nobody decided to skip.",
        )),
    }
}

/// Whether `var` arms its dirty gate.
///
/// ⊘ A value naming neither state reads as **disarmed**. For a flag that *adds* an
/// observation the safe direction is off because an instrument must not fire unasked; for
/// this flag it is off because the armed direction **removes** work, and the safe default
/// for that is always to do the work.
#[must_use]
fn selected_dirty_gate(var: &str) -> bool {
    match std::env::var_os(var) {
        None => false,
        Some(v) => dirty_gate_from(Some(v.to_str().unwrap_or("\u{fffd}invalid"))).unwrap_or(false),
    }
}

/// How many coalesced VA runs one address space may print. See
/// [`kayfabe_rt::device::SharedDevice::vas_reachable_ranges`] — exceeding it is announced, never
/// silent.
/// ★★ **How many qualifying rows one doorbell will attempt per `Vas`.** A cap, and it is
/// stated in the line it bounds: `capped=` says how many were left out, so a short
/// `candidates` list can never read as a complete one.
///
/// ⊘ Sized above the whole measured population rather than tuned: `w290` counted 16425 rows
/// across cup2's entire address space, so a per-VAS budget of 4096 *qualifying* rows cannot
/// bind on any picture this campaign has measured. It exists so a guest that grows its tables
/// without bound cannot turn one doorbell into an unbounded host round trip.
// ⊘ **`#[cfg(feature = "host-isolates")]`, added 2026-08-14 (w296).** Every reader of
// this item lives in the `host-isolates` arm, so a default-feature build compiled the
// no-op sibling and left this dead — `cargo clippy --workspace --all-targets` (which CI
// runs WITHOUT `--all-features`) then reported it under `-D warnings`. ⚠ The gate that
// matters is the one this reveals: **the `host-isolates` arm is never clippy-checked at
// all**, so it carries whatever lints it likes. That is
// `a_feature_gate_with_a_silent_noop_sibling`, one plane over.
#[cfg(feature = "host-isolates")]
const VAS_PUBLISH_LEAF_BUDGET: usize = 4096;

/// ★★★ **How many guest-RAM rows the `pinrate` MEASUREMENT pins.** Bounded on purpose: this
/// is a rate measurement, not the build. A few hundred rows is enough to say whether the
/// per-row cost is flat or degrades, and small enough that a dear rate costs one doorbell
/// rather than the boot.
///
/// ⊘ The number is reported beside the rate, so a reader never has to know it to read the
/// line — and `degrade` below is what makes a *bounded* sample able to speak about 16 328.
// ⊘ **`#[cfg(feature = "host-isolates")]`, added 2026-08-14 (w296).** Every reader of
// this item lives in the `host-isolates` arm, so a default-feature build compiled the
// no-op sibling and left this dead — `cargo clippy --workspace --all-targets` (which CI
// runs WITHOUT `--all-features`) then reported it under `-D warnings`. ⚠ The gate that
// matters is the one this reveals: **the `host-isolates` arm is never clippy-checked at
// all**, so it carries whatever lints it likes. That is
// `a_feature_gate_with_a_silent_noop_sibling`, one plane over.
#[cfg(feature = "host-isolates")]
const VAS_PINRATE_ROWS: usize = 256;

/// ★★★★★ **w292 — how many guest-RAM rows the DRAIN of the doorbelled VAS may take in one
/// doorbell.** The cap that stops a guest growing its tables without bound from turning one
/// MMIO write into an unbounded run of host round trips.
///
/// ⊘ Sized above the whole measured population rather than tuned, exactly as
/// [`VAS_PUBLISH_LEAF_BUDGET`] is: `[measured, boot w290pboth]` cup2's entire address space is
/// **18 269 rows**, of which **1075** were the un-pinned residual. 65 536 therefore cannot
/// bind on any picture this campaign has measured — and if it ever does, the line says
/// `⚠⚠ DRAIN ROW CAP` and a reader knows the drain was **incomplete rather than complete**.
// ⊘ **`#[cfg(feature = "host-isolates")]`, added 2026-08-14 (w296).** Every reader of
// this item lives in the `host-isolates` arm, so a default-feature build compiled the
// no-op sibling and left this dead — `cargo clippy --workspace --all-targets` (which CI
// runs WITHOUT `--all-features`) then reported it under `-D warnings`. ⚠ The gate that
// matters is the one this reveals: **the `host-isolates` arm is never clippy-checked at
// all**, so it carries whatever lints it likes. That is
// `a_feature_gate_with_a_silent_noop_sibling`, one plane over.
#[cfg(feature = "host-isolates")]
const VAS_DRAIN_ROW_CAP: usize = 65536;

/// ★★★★★ **w292 — the wall-clock bound on that drain**, and it is the honest half of the row
/// cap above for [`VAS_PUBLISH_WALL_BUDGET`]'s reason: a count bounds how many rows are
/// *tried*, only a clock bounds how long they take, and every row is a round trip to another
/// process.
///
/// ⊘ **Sized from the MEASUREMENT, and deliberately ~8× above it rather than at it.**
/// `[measured w291, boot w290ppinrate]` a per-row guest-RAM pin costs **276–338 µs and is
/// FLAT**, so the commissioned drain — 1075 rows — is **0.30–0.36 s**. 3 s leaves room for a
/// host that is slower than the one this was measured on **without** letting a pathological
/// reply time hold the vCPU for the length of a boot.
///
/// ⚠ When it fires the line says so loudly and says what it means: **a row left unpinned is
/// not thereby a refused one**, and the drain was INCOMPLETE — which is the difference
/// between *"the leaf was published and hardware still faulted"* and *"we never got to it"*,
/// i.e. between a result and last rung's non-result.
// ⊘ **`#[cfg(feature = "host-isolates")]`, added 2026-08-14 (w296).** Every reader of
// this item lives in the `host-isolates` arm, so a default-feature build compiled the
// no-op sibling and left this dead — `cargo clippy --workspace --all-targets` (which CI
// runs WITHOUT `--all-features`) then reported it under `-D warnings`. ⚠ The gate that
// matters is the one this reveals: **the `host-isolates` arm is never clippy-checked at
// all**, so it carries whatever lints it likes. That is
// `a_feature_gate_with_a_silent_noop_sibling`, one plane over.
#[cfg(feature = "host-isolates")]
const VAS_DRAIN_WALL_BUDGET: std::time::Duration = std::time::Duration::from_millis(3000);

/// ★★★★★ **w321 — THE CONTIGUITY CENSUS: what a COALESCING fix could possibly buy, measured
/// before one is built.**
///
/// # Why this is the first thing w321 does
///
/// The drain costs `rows × ~225 µs`, and `~225 µs` is **three synchronous cross-process
/// round trips** — `VerbPlan::PinGuestRam` is `map_guest_ram` → `describe_guest_ram` →
/// `map_gpu_va`, and each one is its own `Request` over the isolate socket
/// (`kayfabe_isolate_host::isolate::ProxyRmBackend::call`). Two different fixes follow from
/// two different mechanisms and **they need different things to be true**:
///
/// - if the cost is TRANSPORT, one request carrying many rows removes it, and **physical
///   contiguity is irrelevant**;
/// - if the cost is the RM `ioctl`, only **fewer, larger mappings** help — and that is
///   bounded by exactly this census.
///
/// ⊘ `w238` measured *"the GR ring is NOT physically contiguous, so 'one descriptor per run'
/// is one per PAGE"* on **one buffer**. This asks the same question of the **whole drained
/// table**, which is a different population, and answers it with a distribution rather than
/// with a yes/no.
///
/// # What a "run" means here, and why there are two kinds
///
/// A coalesced pin needs BOTH halves contiguous: the guest VAs must abut (or the fixed map
/// would cover addresses the guest did not bind) **and** the guest-physical addresses must
/// abut (or one `OS_DESCRIPTOR` over one `mmap` slice cannot describe them). So:
///
/// - `va_runs` — maximal spans where only `va` abuts. The ceiling if physicality were free.
/// - `pair_runs` — maximal spans where **`va` AND `gpa`** abut. ★ **THIS is the achievable
///   row count of a coalescing fix**, and `rows / pair_runs` is its speedup ceiling.
/// - `va_breaks` / `gpa_breaks` — which half does the breaking. ⚠ Load-bearing: a table
///   broken by VA is SPARSE (nothing to coalesce, and nothing a batched verb fixes either);
///   a table broken by GPA is SCATTERED (a batched verb helps, a coalescer does not).
///
/// ⊘ No square brackets in the returned string: its consumers are `grep -o '…\[[^]]*\]'`
/// matchers, and `w319`'s own attributor was broken for a day by a nested `]`.
#[cfg(feature = "host-isolates")]
fn drain_contiguity(rows: &[(u64, u64, u64)]) -> String {
    if rows.is_empty() {
        return "⊘ NO ROWS — the distribution is UNMEASURED, ⊘ not `contiguous`".to_string();
    }
    let n = rows.len();
    let mut bytes: u64 = 0;
    // len buckets: 4 KiB, <64 KiB, <2 MiB, >= 2 MiB
    let mut len_hist = [0usize; 4];
    let (mut va_runs, mut pair_runs) = (1usize, 1usize);
    let (mut va_breaks, mut gpa_breaks, mut both_breaks) = (0usize, 0usize, 0usize);
    let mut cur_run: u64 = rows[0].2;
    let mut max_run: u64 = rows[0].2;
    // pair-run size buckets: 4 KiB, <64 KiB, <2 MiB, >= 2 MiB
    let mut run_hist = [0usize; 4];
    let bucket = |v: u64| -> usize {
        if v <= 0x1000 {
            0
        } else if v < 0x1_0000 {
            1
        } else if v < 0x20_0000 {
            2
        } else {
            3
        }
    };
    for (i, &(va, gpa, len)) in rows.iter().enumerate() {
        bytes = bytes.saturating_add(len);
        len_hist[bucket(len)] += 1;
        if i == 0 {
            continue;
        }
        let (pva, pgpa, plen) = rows[i - 1];
        let va_ok = pva.checked_add(plen) == Some(va);
        let gpa_ok = pgpa.checked_add(plen) == Some(gpa);
        if !va_ok {
            va_runs += 1;
        }
        if !(va_ok && gpa_ok) {
            pair_runs += 1;
            run_hist[bucket(cur_run)] += 1;
            max_run = max_run.max(cur_run);
            cur_run = len;
            match (va_ok, gpa_ok) {
                (false, false) => both_breaks += 1,
                (true, false) => gpa_breaks += 1,
                (false, true) => va_breaks += 1,
                (true, true) => unreachable!("a pair break with both halves contiguous"),
            }
        } else {
            cur_run = cur_run.saturating_add(len);
        }
    }
    run_hist[bucket(cur_run)] += 1;
    max_run = max_run.max(cur_run);
    format!(
        "rows={n} bytes=0x{bytes:x} len_4k={} len_lt64k={} len_lt2m={} len_ge2m={} \
         va_runs={va_runs} pair_runs={pair_runs} coalesce_ceiling={}.{:02}x \
         break_va_only={va_breaks} break_gpa_only={gpa_breaks} break_both={both_breaks} \
         runsz_4k={} runsz_lt64k={} runsz_lt2m={} runsz_ge2m={} max_run=0x{max_run:x} \
         ⇒ a coalescing fix can reduce {n} host chains to {pair_runs}; a BATCHED-TRANSPORT \
         fix is bounded by neither of these numbers",
        len_hist[0],
        len_hist[1],
        len_hist[2],
        len_hist[3],
        n / pair_runs,
        (n * 100 / pair_runs) % 100,
        run_hist[0],
        run_hist[1],
        run_hist[2],
        run_hist[3],
    )
}

/// ★★★★★ **w321 — one host chain's worth of the drain.**
///
/// On the default arm it is exactly one table row and this type is a wrapper. On
/// `KAYFABE_DRAIN_BATCH=coalesce` it is a MERGED RUN of rows that abut in **both** `va` and
/// `gpa`, and `rows` is how many of them.
///
/// ⊘ `first_row` exists so a refused chunk can fall back to its own rows **by index into the
/// original candidate list**, rather than by re-deriving them from `(va, len)` — a
/// re-derivation would be this file inventing a row boundary the table stated.
#[cfg(feature = "host-isolates")]
#[derive(Debug, Clone, Copy)]
struct DrainChunk {
    va: u64,
    gpa: u64,
    len: u64,
    /// How many table rows this chain covers. **1 on the default arm.**
    rows: usize,
    /// Index of this chunk's first row in the candidate list.
    first_row: usize,
    /// The base VA of the LAST row covered — what `last_pinned_va` must report, because
    /// w319's discriminator is *that VA versus the faulting VA* and a chunk's END is not a
    /// row's base.
    last_row_va: u64,
}

#[cfg(feature = "host-isolates")]
impl DrainChunk {
    fn one((va, gpa, len): (u64, u64, u64)) -> Self {
        Self {
            va,
            gpa,
            len,
            rows: 1,
            first_row: 0,
            last_row_va: va,
        }
    }
}

/// ★★★ **w321 — the split boundary, and it is the C's number, not a guess.**
///
/// This repo's own record of the C's sysmem chunker: it *"starts the first chunk at the run's
/// own VA (`cva = a->va0 + off`), splitting only **at 2 MiB boundaries**"*. Two reasons it is
/// the right bound here as well: it caps how much one `OS_DESCRIPTOR`'s `get_user_pages` and
/// one `map_gpu_va`'s PTE fill can cost inside a single BQL-held ioctl, and it is the
/// granule above which RM's own fixed-placement arithmetic stops being 64 KiB-shaped.
///
/// ⊘ `[measured w321, boot `w321i1`]` it costs almost nothing on this workload: the census
/// found ONE run above 2 MiB (16.8 MiB), so the cap turns 1 179 runs into ~1 186.
#[cfg(feature = "host-isolates")]
const DRAIN_CHUNK_MAX: u64 = 2 << 20;

/// ★★★★★ **w328 — THE BREADTH ARM.**
///
/// `KAYFABE_PUBLISH_SCOPE=doorbelled` restricts **both** doorbell-time passes — the
/// publication census (`publish_vas_rows`) and the guest-RAM pin pass
/// (`measure_guest_ram_pin_rate`) — to the VAS the doorbell is about. **Absent or anything
/// else ⇒ `all` ⇒ byte-identical to master**, so one binary carries both arms.
///
/// # ⚠⚠ THE HAZARD RUNS THE OTHER WAY HERE, AND IT IS NAMED BEFORE THE KNOB IS
///
/// Every other budget in this file risks doing **too little work too slowly**. This one risks
/// **not doing work at all**: a mapping we decline to publish is a mapping the host MMU has no
/// directory for, i.e. a GPU fault — and it is indistinguishable by symptom from
/// `the_drain_budget_truncation.md`'s pre-existing intermittent. ⇒ two refusals are built in:
///
/// 1. **No target ⇒ no scoping.** `scoped` requires the doorbell to have named a VAS. Scoping
///    to a VAS we cannot name is scoping to none, and would publish nothing at all.
/// 2. **No stamp for a VAS we skipped.** The w318 dirty gate's stamp asserts *"this census
///    ran to completion"*. Stamping a scoped-out VAS would tell the next doorbell that a VAS
///    nobody looked at is clean — a publication silently never performed.
///
/// ⊘ **It is an instrument first.** `W328SCOPE`/`W328PIN` print the breadth's cost and its
/// yield on **every** arm including the default one, so what the breadth is worth is a
/// measurement before it is a switch.
#[cfg(feature = "host-isolates")]
fn publish_scope_arm() -> &'static str {
    static V: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    match V
        .get_or_init(|| std::env::var("KAYFABE_PUBLISH_SCOPE").unwrap_or_default())
        .as_str()
    {
        "doorbelled" => "doorbelled",
        _ => "all",
    }
}

/// ★★★★★ **w328 — THE SCOPING PREDICATE, AS A PURE FUNCTION.**
///
/// Both passes ask the same question and **must** answer it identically: the publication
/// census and the guest-RAM pin pass scoping to different VASes on one doorbell would publish
/// one address space and pin another. ⊘ Extracted so the two refusals are testable **offline,
/// without a GPU and without an env var** — a knob whose safety property is only ever
/// exercised on a bench is a wish. Its known-positive/negative pairs are
/// `tests/scope_predicate.rs`.
///
/// `arm` is [`publish_scope_arm`]'s word; `target` is the `(proc, pdb)` this doorbell named.
#[cfg(feature = "host-isolates")]
fn publish_scope_scoped(
    arm: &str,
    target: Option<(kayfabe_core::ProcId, kayfabe_rt::Pdb)>,
) -> bool {
    arm == "doorbelled" && target.is_some_and(|(p, _)| p != kayfabe_core::gpu::Gpu::SYSTEM_PROC)
}

/// ★★★★★ **w321 — THE FIX'S ARM.** `KAYFABE_DRAIN_BATCH=coalesce` merges the drain's rows
/// into contiguous chains. **Absent or anything else ⇒ `off` ⇒ byte-identical to master**, so
/// the SAME BINARY carries both arms and the only variable between them is this word.
#[cfg(feature = "host-isolates")]
fn drain_batch_arm() -> &'static str {
    static V: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    match V
        .get_or_init(|| std::env::var("KAYFABE_DRAIN_BATCH").unwrap_or_default())
        .as_str()
    {
        // ★★★★★ w330 — DEFAULT MOVED off → coalesce, on measurement. `[w330]` the doorbell
        // trap's MAX falls 2 753 760 → 217 190 us (12.7x). ⊘ Its MEDIAN gets 1.6x WORSE
        // (18 741 → 30 311), so graded on a median alone this reads as a REGRESSION — it acts
        // on a different statistic of the same distribution than the dirty gate does.
        // ⊘ `off` is KEPT as the named escape hatch and as w321's negative control.
        "off" => "off",
        _ => "coalesce",
    }
}

/// ★★★★★ **w321 — THE COALESCER.**
///
/// # What it does, and the two facts that make it sound
///
/// Merges consecutive candidate rows whose `va` AND `gpa` both abut into one chain, split at
/// [`DRAIN_CHUNK_MAX`]. **Both halves are required.** VA contiguity alone would place a fixed
/// GPU mapping over addresses the guest did not bind; GPA contiguity alone cannot be
/// described by one `mmap` slice of the guest-RAM `memfd`, which is what one `OS_DESCRIPTOR`
/// is built over.
///
/// ⊘ It merges nothing that `vas_guest_ram_rows` did not already classify: every row in the
/// list is guest RAM, unpinned, non-empty. The merge adds **no** claim about any address —
/// it only stops asking the host the same question 4 KiB at a time.
///
/// # ★★★★★ WHAT IT IS WORTH, MEASURED BEFORE IT WAS BUILT
///
/// `[measured w321, vh, real GA106, boot `w321i1`, tag W321CENSUS]` over the 13 313 rows of
/// the doorbelled VAS at `cuCtxCreate`:
///
/// - `len_4k = 13 312` of 13 313 — the table is **all single pages**;
/// - `va_runs = 3` — in VA the whole 54.5 MiB is **three** contiguous spans;
/// - `pair_runs = 1 179`, `break_va_only = 0`, `break_gpa_only = 1 176`, `break_both = 2`
///   ⇒ **every break is PHYSICAL SCATTER and none is VA sparsity**;
/// - ⇒ `coalesce_ceiling = 11.29×`, `max_run = 0x100_2000` (16.8 MiB).
///
/// ⊘ **`w238`'s constraint is confirmed in kind and refuted in magnitude for this
/// population.** *"The GR ring is not physically contiguous, so one descriptor per run is one
/// per PAGE"* is true of a ring; over the whole drained table the mean run is **11.29 pages**
/// and 754 of the 1 179 runs are single pages while the other 425 carry 12 559 of the rows.
/// ⇒ the mass is in long runs; the count is in short ones.
#[cfg(feature = "host-isolates")]
fn chunks_for(rows: &[(u64, u64, u64)]) -> Vec<DrainChunk> {
    if drain_batch_arm() == "coalesce" {
        coalesce(rows)
    } else {
        rows.iter()
            .enumerate()
            .map(|(i, &r)| DrainChunk {
                first_row: i,
                ..DrainChunk::one(r)
            })
            .collect()
    }
}

/// [`chunks_for`]'s merge, **without the environment read**, so it has known-positives.
///
/// ⊘ Split out for exactly one reason: `drain_batch_arm` is a process-global `OnceLock` over
/// an env var, so a test that exercised the merge through it would set the whole process's
/// arm and could never test the other one. *A criterion nobody has watched fail is a wish*,
/// and an arm that cannot be exercised in a test is worse.
#[cfg(feature = "host-isolates")]
fn coalesce(rows: &[(u64, u64, u64)]) -> Vec<DrainChunk> {
    let mut out: Vec<DrainChunk> = Vec::new();
    for (i, &(va, gpa, len)) in rows.iter().enumerate() {
        if let Some(cur) = out.last_mut()
            && cur.va.checked_add(cur.len) == Some(va)
            && cur.gpa.checked_add(cur.len) == Some(gpa)
            && cur.len.saturating_add(len) <= DRAIN_CHUNK_MAX
        {
            cur.len += len;
            cur.rows += 1;
            cur.last_row_va = va;
            continue;
        }
        out.push(DrainChunk {
            va,
            gpa,
            len,
            rows: 1,
            first_row: i,
            last_row_va: va,
        });
    }
    out
}

/// ★★★★★ **w319 — THE MODULATION KNOB, and it is an INSTRUMENT, not a fix.**
///
/// `KAYFABE_VAS_DRAIN_BUDGET_MS` overrides [`VAS_DRAIN_WALL_BUDGET`] for the doorbelled VAS's
/// drain. **Absent ⇒ byte-identical behaviour to master** (3000 ms), so every existing caller
/// and every committed trace stays comparable.
///
/// # Why it exists
///
/// `[measured w319, from w314's OWN COMMITTED LOGS, zero boots spent]` the two RED cup3 boots
/// of `traces/w314_confirm/` both carry `⚠⚠ DRAIN WALL BUDGET 3000 ms EXHAUSTED`
/// (`pinned=11883/13313` stopping at `last_pinned_va=0x20326a000`, and `pinned=11810/13313`
/// stopping at `0x203221000`); the green boots carry `pinned=13313/13313 DRAIN_MS=2672` and
/// `2898`, reaching `0x2047ff000`. **The faulting page `0x2_0440f000` lies between the two.**
/// ⇒ the drain's own cost (13 313 rows × 199–280 µs = **2.65–3.73 s**) STRADDLES its 3 s
/// budget, so which side of it a boot lands on decides whether the completion-semaphore page
/// is published before the engine writes it.
///
/// ⇒ An intermittent whose rate can be driven **both ways** by one number is an intermittent
/// that has been attributed. This knob is that number, exposed.
#[cfg(feature = "host-isolates")]
fn vas_drain_wall_budget() -> std::time::Duration {
    static V: std::sync::OnceLock<std::time::Duration> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        std::env::var("KAYFABE_VAS_DRAIN_BUDGET_MS")
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map_or(VAS_DRAIN_WALL_BUDGET, std::time::Duration::from_millis)
    })
}

/// ★★★★★ **w319 — THE DETERMINISTIC HALF OF THE SAME KNOB.**
///
/// `KAYFABE_VAS_DRAIN_ROW_LIMIT` caps how many rows of the doorbelled VAS the drain may take,
/// **below** [`VAS_DRAIN_ROW_CAP`]. Absent ⇒ 65 536, i.e. master unchanged.
///
/// ⊘ The wall budget above reproduces the defect the way the defect actually happens, and is
/// therefore the *faithful* knob — but it is a CLOCK, so it truncates at a different row on
/// every boot and cannot give an on-demand repro with a stable fingerprint. This one
/// truncates at a **row count**, which is deterministic. ⇒ Use the row limit to REPRODUCE and
/// the millisecond budget to MODULATE; neither is a fix and neither is on by default.
#[cfg(feature = "host-isolates")]
fn vas_drain_row_limit() -> usize {
    static V: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        std::env::var("KAYFABE_VAS_DRAIN_ROW_LIMIT")
            .ok()
            .and_then(|s| s.trim().parse::<usize>().ok())
            .filter(|n| *n > 0)
            .map_or(VAS_DRAIN_ROW_CAP, |n| n.min(VAS_DRAIN_ROW_CAP))
    })
}

/// ★★★★★ **w319 — arms the completion-page pin that runs AHEAD of the budgeted drain.**
///
/// `KAYFABE_COMPLETION_PIN=on`. Absent or anything else ⇒ **off**, and off is byte-identical
/// to master. ⊘ Deliberately a separate variable from the two drain knobs, so ONE binary can
/// carry the provocation (`KAYFABE_VAS_DRAIN_ROW_LIMIT`) and the fix independently and the
/// only difference between the two arms of the fix test is this flag.
#[cfg(feature = "host-isolates")]
fn completion_pin_armed() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        std::env::var("KAYFABE_COMPLETION_PIN")
            .map(|s| s.trim().eq_ignore_ascii_case("on"))
            .unwrap_or(false)
    })
}

/// ★★★ **The wall-clock budget for one doorbell's publication**, and it is the honest half of
/// the cap above: a count bounds how many leaves are *tried*, only a clock bounds how long
/// they take. Each leaf is a round trip to another process, so a pathological reply time
/// would otherwise stall the doorbell the publication exists to precede.
///
/// ⚠ When it fires the line says so loudly and says what it means: **an unpublished row is
/// not thereby a refused one.** That distinction is the whole reason this is a named budget
/// rather than a silent `break`.
// ⊘ **`#[cfg(feature = "host-isolates")]`, added 2026-08-14 (w296).** Every reader of
// this item lives in the `host-isolates` arm, so a default-feature build compiled the
// no-op sibling and left this dead — `cargo clippy --workspace --all-targets` (which CI
// runs WITHOUT `--all-features`) then reported it under `-D warnings`. ⚠ The gate that
// matters is the one this reveals: **the `host-isolates` arm is never clippy-checked at
// all**, so it carries whatever lints it likes. That is
// `a_feature_gate_with_a_silent_noop_sibling`, one plane over.
#[cfg(feature = "host-isolates")]
const VAS_PUBLISH_WALL_BUDGET: std::time::Duration = std::time::Duration::from_millis(2000);

const PT_SWEEP_RANGE_CAP: usize = 48;

/// How many DISTINCT refused virtual addresses one sweep line may list. See the refusal block
/// in [`SharedDoorbell::sweep_cpu_pt_tables`] — an address absent from a capped list is not
/// thereby un-refused, and the line says so when it truncates.
const PT_SWEEP_REFUSAL_CAP: usize = 24;

/// How many DISTINCT straddle signatures one line may list. Small on purpose: the whole
/// point is that the signature space is tiny (a handful of `(shape, agreement, level, extent)`
/// combinations), and a line that needs more than this is itself the finding.
const STRADDLE_SIG_CAP: usize = 12;

/// ★★★★★ **WHAT ACTUALLY DIFFERS, as a histogram over SIGNATURES rather than a count.**
///
/// `[measured, w276b_on]` the sweep printed `refusals=255 by_kind={"StraddlesLiveBinding": 255}`
/// — a histogram with **one bucket**, over a payload that carried only a virtual address. That
/// number is equally consistent with a page-size mismatch, an extent mismatch, a stale binding,
/// and two populate sources contradicting each other, and **those four want opposite fixes**.
/// `a_count_cannot_see_a_substitution`, one plane over.
///
/// The signature is `(shape, agreement, leaf level, leaf extent, live extent, live published)`.
/// Every one of those six is read off the refusal itself, so a row here is reconstructible
/// without the boot that produced it.
///
/// ⚠ **`agreement` is the field to read first.** `SameMemory` means the two shapes describe the
/// same byte at the same aperture and differ only in granularity — the refusal is still correct
/// (the table holds one shape per range) but neither source is wrong. `Contradicts` means they
/// disagree about what backs the address, which is the `w222` class and is a bug in one of them.
fn straddle_census(refusals: &[kayfabe_mmu::walker::PopulateRefusal]) -> String {
    use kayfabe_mmu::walker::PopulateRefusal as P;
    let mut sigs: std::collections::BTreeMap<
        (
            kayfabe_mmu::walker::StraddleShape,
            kayfabe_mmu::walker::StraddleAgreement,
            u8,
            u64,
            u64,
            bool,
        ),
        usize,
    > = std::collections::BTreeMap::new();
    let mut first: Option<String> = None;
    for r in refusals {
        let P::StraddlesLiveBinding { va, straddle } = r else {
            continue;
        };
        *sigs
            .entry((
                straddle.shape(*va),
                straddle.agreement(*va),
                straddle.level,
                straddle.size.0,
                straddle.live_len,
                straddle.live_published,
            ))
            .or_default() += 1;
        if first.is_none() {
            first = Some(format!(
                "va=0x{:x} size=0x{:x} lvl={} phys=0x{:x}/{:?} \
                 OVER live=[0x{:x}+0x{:x} phys=0x{:x}/{:?} published={}] {:?}/{:?}",
                va.0,
                straddle.size.0,
                straddle.level,
                straddle.phys,
                straddle.aperture,
                straddle.live_start,
                straddle.live_len,
                straddle.live_phys,
                straddle.live_aperture,
                straddle.live_published,
                straddle.shape(*va),
                straddle.agreement(*va),
            ));
        }
    }
    if sigs.is_empty() {
        return " straddles=NONE".to_string();
    }
    let total: usize = sigs.values().sum();
    let contradicting: usize = sigs
        .iter()
        .filter(|((_, a, ..), _)| *a == kayfabe_mmu::walker::StraddleAgreement::Contradicts)
        .map(|(_, n)| *n)
        .sum();
    let n_sigs = sigs.len();
    let shown: Vec<String> = sigs
        .into_iter()
        .take(STRADDLE_SIG_CAP)
        .map(|((shape, agree, level, size, live_len, pubd), n)| {
            format!(
                "{shape:?}/{agree:?}/lvl{level}/leaf0x{size:x}/live0x{live_len:x}/pub{}={n}",
                u8::from(pubd)
            )
        })
        .collect();
    format!(
        " straddles={total} contradicting={contradicting} sigs={{{}}}{} first_straddle=[{}]",
        shown.join(", "),
        if n_sigs > STRADDLE_SIG_CAP {
            format!(" ⚠⚠ CAPPED at {STRADDLE_SIG_CAP} of {n_sigs} signatures")
        } else {
            String::new()
        },
        first.as_deref().unwrap_or("NONE"),
    )
}

/// ★★★★★ **THE THIRD OUTCOME, PRINTED** — leaves that reached neither `bound` nor `refusals`
/// because they lost a key collision inside the settlement's desired-set map.
///
/// See [`kayfabe_mmu::reach::Settlement::shape_collisions`] for why the collision exists at all.
/// Printed as a histogram over `(kept level/size, dropped level/size)` plus one verbatim row,
/// because *"which two producers described this VA"* is the actionable half and a bare count is
/// the thing this campaign keeps paying for.
fn collision_census(
    collisions: &[kayfabe_mmu::reach::ShapeCollision],
    duplicates: usize,
) -> String {
    if collisions.is_empty() {
        return format!(" shape_collisions=0 dup_leaves={duplicates}");
    }
    let mut sigs: std::collections::BTreeMap<(u8, u64, u8, u64), usize> =
        std::collections::BTreeMap::new();
    for c in collisions {
        *sigs
            .entry((
                c.kept.level,
                c.kept.size.0,
                c.dropped.level,
                c.dropped.size.0,
            ))
            .or_default() += 1;
    }
    let first = collisions[0];
    let shown: Vec<String> = sigs
        .into_iter()
        .take(STRADDLE_SIG_CAP)
        .map(|((kl, ks, dl, ds), n)| format!("kept-lvl{kl}/0x{ks:x}_over_lvl{dl}/0x{ds:x}={n}"))
        .collect();
    format!(
        " shape_collisions={} dup_leaves={duplicates} by={{{}}} first_collision=[va=0x{:x} \
         KEPT lvl{}/0x{:x}→0x{:x} from page 0x{:x} | DROPPED lvl{}/0x{:x}→0x{:x} from page 0x{:x}]",
        collisions.len(),
        shown.join(", "),
        first.va.0,
        first.kept.level,
        first.kept.size.0,
        first.kept.phys,
        first.kept.from_page,
        first.dropped.level,
        first.dropped.size.0,
        first.dropped.phys,
        first.dropped.from_page,
    )
}

/// The refusal's kind as a stable word, and the address it is about.
///
/// ⊘ A `&'static str` rather than `format!("{r:?}")` because the kinds are what a histogram is
/// over, and `Debug` embeds the payload — every refusal would be its own bucket and the
/// histogram would be a list. The *addresses* are collected separately, so nothing is lost.
fn refusal_kind_va(r: &kayfabe_mmu::walker::PopulateRefusal) -> (&'static str, Option<u64>) {
    use kayfabe_mmu::walker::PopulateRefusal as P;
    match r {
        P::Refused { va, .. } => ("Refused", Some(va.0)),
        P::RepointsPublished { va, .. } => ("RepointsPublished", Some(va.0)),
        P::StraddlesLiveBinding { va, .. } => ("StraddlesLiveBinding", Some(va.0)),
        P::UnbindsPublished { va } => ("UnbindsPublished", Some(va.0)),
        P::UndecidableKind { va, .. } => ("UndecidableKind", Some(va.0)),
    }
}

/// Whether `value` arms the whole-VAS sweep — the pure half of [`selected_pt_sweep`].
///
/// # Errors
/// [`Status::Unsupported`] if `value` names neither state. **Absent is not an error**; it is
/// `false`.
pub fn pt_sweep_from(value: Option<&str>) -> Result<bool, (Status, &'static str)> {
    match value {
        None | Some("off") => Ok(false),
        Some("on") => Ok(true),
        Some(_) => Err((
            Status::Unsupported,
            "KAYFABE_PT_SWEEP does not name a state: the only values are `off` (the default) \
             and `on`. It is not defaulted, because the disarmed arm IS this rung's negative \
             control AND because the armed arm relaxes a correctness gate — a typo that \
             silently armed it would relax that gate without anyone deciding to.",
        )),
    }
}

/// Whether [`PT_SWEEP_ENV`] arms the whole-VAS sweep.
///
/// ⊘ A value naming neither state reads as **disarmed**, which is the safe direction for a flag
/// that relaxes a gate: an unparseable value must never be able to turn it on.
#[must_use]
fn selected_pt_sweep() -> bool {
    match std::env::var_os(PT_SWEEP_ENV) {
        None => false,
        Some(v) => pt_sweep_from(Some(v.to_str().unwrap_or("\u{fffd}invalid"))).unwrap_or(false),
    }
}

/// ★★★★★ **w329 — arm the RELEASE of a joined framebuffer leaf the guest has unmapped.**
///
/// # ⊘ ON by default, and that is the opposite of [`PT_SWEEP_ENV`]'s default for a reason
///
/// The dirty gates and the sweep default off because the armed arm **removes work** and a typo
/// must not silently skip something nobody decided to skip. This one is the other direction:
/// the disarmed arm is what `w327` measured as a **hard allocation failure** — a freed
/// allocation's framebuffer frames stay joined forever, the guest recycles them, and the first
/// `cuMemsetD32` past the first recycled frame kills the channel with no `Xid` and no `NVRM`
/// line. Defaulting off would mean the fix ships disabled.
///
/// ⇒ `off` is the **negative control**, and it is what makes a one-binary two-arm boot possible:
/// `KAYFABE_JOIN_RELEASE=off` reproduces `w327`'s `28,31` failure exactly, from the same
/// archive that passes with the variable unset.
///
/// ⚠ An unparseable value reads as **on**, i.e. as the default — never as the control. A typo
/// must not be able to silently disarm a correctness fix and leave a boot looking armed.
pub const JOIN_RELEASE_ENV: &str = "KAYFABE_JOIN_RELEASE";

/// ★★★★★ **w329 — the two TRIGGERS, as one arm.** They are different events and only
/// the second one is the event that actually occurs on this workload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinReleaseArm {
    /// ⊘ w327's state: a join is never released. The NEGATIVE CONTROL.
    Off,
    /// Leg 1 — release on the guest's own UNMAP, via
    /// [`kayfabe_mmu::reach::PublishedUnbind::RevokeWholeJoins`]. This is the trigger
    /// `join_operand_fb_leaves`' cleanup table nominates.
    Unmap,
    /// ★★★ Leg 1 **and** leg 2 — also supersede a join whose frame the guest has
    /// re-pointed into a different VA of the same address space. `[measured, w329a1]` leg 1
    /// alone fires 8 times in a whole `28,31` run and the failure survives, because CUDA's
    /// suballocator does not unmap on `cuMemFree`. See
    /// [`kayfabe_rt::device::SharedDevice::supersede_joined_fb_leaf`].
    Supersede,
}

impl JoinReleaseArm {
    /// The settlement policy this arm implies.
    #[must_use]
    pub fn policy(self) -> kayfabe_mmu::reach::PublishedUnbind {
        match self {
            JoinReleaseArm::Off => kayfabe_mmu::reach::PublishedUnbind::Refuse,
            JoinReleaseArm::Unmap | JoinReleaseArm::Supersede => {
                kayfabe_mmu::reach::PublishedUnbind::RevokeWholeJoins
            }
        }
    }

    /// Whether a collision with an existing join may take it over.
    #[must_use]
    pub fn supersedes(self) -> bool {
        matches!(self, JoinReleaseArm::Supersede)
    }

    /// The word a boot's log prints, so a reader never has to infer the arm.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            JoinReleaseArm::Off => "off",
            JoinReleaseArm::Unmap => "on",
            JoinReleaseArm::Supersede => "supersede",
        }
    }
}

/// Which arm `value` names — the pure half of [`selected_join_release`].
///
/// # Errors
/// [`Status::Unsupported`] if `value` names no arm. **Absent is not an error**; it is the fix.
pub fn join_release_from(value: Option<&str>) -> Result<JoinReleaseArm, (Status, &'static str)> {
    match value {
        // ★★★★★ w330 — DEFAULT MOVED `Unmap` → `Supersede`, on measurement.
        // `[w330, fresh GA106, same binary, interleaved, 2/2 vs 2/2]` leg 1's own counters
        // read `revoked=0 released=0` on BOTH arms: the unmap trigger fires ZERO times,
        // because CUDA's suballocator does not unmap on `cuMemFree`. ⇒ `on` was
        // behaviourally IDENTICAL to `off`, so the DEFAULT SHIPPED THE FAILURE: `28,31`
        // gave 0 BWITER rows, 32 `already joined`, and a host `Xid 31 FAULT_PDE` at an
        // address inside the row's own `in_ptr`. With `supersede`: 7 rows, 0 refusals,
        // 0 Xid, 279 takeovers.
        // ⊘ `on` is KEPT as its own word so the old arm stays reachable BY NAME.
        None | Some("supersede") => Ok(JoinReleaseArm::Supersede),
        Some("on") => Ok(JoinReleaseArm::Unmap),
        Some("off") => Ok(JoinReleaseArm::Off),
        Some(_) => Err((
            Status::Unsupported,
            "KAYFABE_JOIN_RELEASE does not name an arm: the values are `on` (the default — a \
             joined framebuffer leaf the guest UNMAPS is released), `supersede` (also take \
             over a join whose frame the guest re-pointed into another VA), and `off` (w327's \
             negative control, in which a join is kept forever and the guest's next allocation \
             over a recycled frame dies rc=719).",
        )),
    }
}

/// Which arm [`JOIN_RELEASE_ENV`] names.
///
/// ⊘ **Read ONCE and cached.** The arm cannot change during a device life, and the two
/// consumers - the settlement pass and the join site - are on the guest's own MMIO path, where
/// one of them runs per framebuffer leaf. Re-reading the environment there would put a
/// `getenv` under the doorbell handler for no fact that can have changed, and would also make
/// the two consumers capable of disagreeing mid-boot, which is exactly the shape
/// `a_second_source_of_truth_beside_a_complete_value` is banked for.
#[must_use]
fn selected_join_release() -> JoinReleaseArm {
    static ARM: std::sync::OnceLock<JoinReleaseArm> = std::sync::OnceLock::new();
    *ARM.get_or_init(|| match std::env::var_os(JOIN_RELEASE_ENV) {
        None => JoinReleaseArm::Supersede,
        Some(v) => join_release_from(Some(v.to_str().unwrap_or("\u{fffd}invalid")))
            .unwrap_or(JoinReleaseArm::Supersede),
    })
}

/// ★★ **How many times ONE framebuffer frame's join may be taken over in a device
/// life.**
///
/// ⚠ Without a cap this is a PING-PONG: the superseded row is re-proposed by the next
/// settlement (the guest still describes that VA), becomes a publication candidate again, and
/// takes the join back — host RM verbs on every doorbell, forever. The cap makes the behaviour
/// bounded and the boot says how often it was reached.
const SUPERSEDE_CAP_PER_FRAME: usize = 4;

/// The per-frame takeover ledger. ⊘ Process-global rather than a field, because it is a
/// COUNTER and not a source of truth: nothing reads it to decide what a frame IS, only to stop
/// an unbounded loop. It is reset by nothing, which is correct — the bound is per device life.
fn supersede_ledger() -> &'static std::sync::Mutex<std::collections::HashMap<u64, usize>> {
    static L: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<u64, usize>>> =
        std::sync::OnceLock::new();
    L.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
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
            //
            // ⊘⊘ **CORRECTED 2026-08-12 (w279): the last clause was FALSE WHEN WRITTEN, and
            // that is what it cost.** Two other callers were already asking `is_resident`
            // about joined addresses — `fb_dump_row` above, and `PlaneFbSource::page_written`
            // on the FORWARDING path. The join check landed here, as a local `if`, and did
            // not travel; on boot `w278b_guest` the same blindness printed
            // `fbRING@0x41000 nz4/4096 resN-NEVER-WRITTEN` and **refused a doorbell** with
            // `FwdFault::RingFbNeverWritten`. ⇒ The check now lives in
            // `RegPlane::fb_page_standing` and `fb_is_resident` is GONE from the plane, so a
            // fourth caller cannot re-acquire the defect. ★ A correction implemented at ONE
            // call site is not a correction; it is a local escape from a shared defect.
            let res = plane.fb_page_standing(at).tag();
            format!(
                " fbuserd@0x{at:x} GET={} PUT={} {res}",
                u32::from_le_bytes([w[0], w[1], w[2], w[3]]),
                u32::from_le_bytes([w[4], w[5], w[6], w[7]]),
            )
        }
    }
}

/// ★ **A TRUNCATED SAMPLE MUST NEVER RENDER AS A COMPLETE LIST.**
///
/// ⊘ w304 — leg 4's page-derivation and run-coalescing tests went with leg 4. What is left is
/// the one property that outlived it, because `pushbuffer_sample` is still what renders every
/// bounded list the remaining passes print.
#[cfg(all(test, feature = "host-isolates"))]
mod pushbuffer_pin_tests {
    use super::pushbuffer_sample;

    /// ★★★★★ **A TRUNCATED SAMPLE MUST NEVER RENDER AS A COMPLETE LIST** — the defect in my
    /// own first draft, caught before any output was read.
    ///
    /// ⚠ `pages.len() == 1` per doorbell in the only measured workload, so the sample cap is
    /// unreachable on a boot and this bug is **invisible to every green log**. That is
    /// precisely why it needs a test rather than a run.
    #[test]
    fn a_truncated_sample_says_it_is_a_sample_and_names_how_many_are_missing() {
        let four: Vec<String> = (0..4).map(|i| format!("va=0x{i:x}")).collect();
        // n == len ⇒ a complete list, rendered plainly.
        let whole = pushbuffer_sample(&four, 4);
        assert!(whole.contains("va=0x0"), "{whole}");
        assert!(
            !whole.contains("SAMPLE"),
            "★ a COMPLETE list must not be labelled a sample: {whole}"
        );
        // n > len ⇒ it must SAY so, and say how many are not shown.
        let part = pushbuffer_sample(&four, 9);
        assert!(
            part.contains("SAMPLE of 9"),
            "★ nine refusals rendered as four with no warning: {part}"
        );
        assert!(
            part.contains("+5 more"),
            "★ the shortfall is not named: {part}"
        );
        // ⊘ Empty renders as nothing at all — never as an empty pair of brackets, which
        // reads as "we looked and found none" when nothing was looked at.
        assert_eq!(pushbuffer_sample(&[], 0), "");
    }
}

/// ★★★★★ **w321 — THE COALESCER'S KNOWN-POSITIVES.**
///
/// Every one of these is a case the boot cannot show me: the census says the production table
/// is 13 313 rows and I get four numbers out of it, so a merge that quietly dropped a row, or
/// merged across a GPA break, would show up as *a slightly different count* and nothing else.
/// ⇒ The properties are asserted here, where they can fail loudly.
#[cfg(all(test, feature = "host-isolates"))]
mod w321_coalesce_tests {
    use super::{DRAIN_CHUNK_MAX, coalesce, drain_contiguity};

    /// The invariant everything else rests on: **every row is covered, exactly once, in
    /// order.** ⊘ Checked by reconstructing the row list from the chunks rather than by
    /// counting — `w281b`'s falsifier fired on a count while the thing counted was
    /// substituted underneath it.
    fn assert_covers(rows: &[(u64, u64, u64)]) {
        let chunks = coalesce(rows);
        let mut i = 0usize;
        for c in &chunks {
            assert_eq!(c.first_row, i, "chunks must tile the row list in order");
            let span: u64 = rows[i..i + c.rows].iter().map(|r| r.2).sum();
            assert_eq!(c.len, span, "a chunk's length is its rows' lengths");
            assert_eq!(c.va, rows[i].0);
            assert_eq!(c.gpa, rows[i].1);
            assert_eq!(c.last_row_va, rows[i + c.rows - 1].0);
            i += c.rows;
        }
        assert_eq!(i, rows.len(), "every row must be in exactly one chunk");
    }

    #[test]
    fn a_gpa_break_splits_the_chunk_even_when_the_vas_abut() {
        // ★ This is the production shape: `break_gpa_only = 1176` of 1 178 breaks. Merging
        // here would describe page B's guest bytes with page A+1's physical address.
        let rows = [
            (0x2_0000_0000, 0x1_0000_0000, 0x1000),
            (0x2_0000_1000, 0x1_0000_1000, 0x1000),
            (0x2_0000_2000, 0x7_0000_0000, 0x1000),
        ];
        let c = coalesce(&rows);
        assert_eq!(c.len(), 2, "{c:?}");
        assert_eq!(c[0].rows, 2);
        assert_eq!(c[0].len, 0x2000);
        assert_eq!(c[1].rows, 1);
        assert_covers(&rows);
    }

    #[test]
    fn a_va_break_splits_the_chunk_even_when_the_gpas_abut() {
        let rows = [
            (0x2_0000_0000, 0x1_0000_0000, 0x1000),
            (0x2_0000_9000, 0x1_0000_1000, 0x1000),
        ];
        let c = coalesce(&rows);
        assert_eq!(c.len(), 2, "a merged chunk would map a VA the guest never bound: {c:?}");
        assert_covers(&rows);
    }

    #[test]
    fn a_perfectly_contiguous_run_splits_at_the_two_mib_bound_and_nowhere_else() {
        let n = 1024usize; // 4 MiB of 4 KiB pages
        let rows: Vec<(u64, u64, u64)> = (0..n)
            .map(|i| {
                let o = (i as u64) * 0x1000;
                (0x2_0000_0000 + o, 0x1_0000_0000 + o, 0x1000)
            })
            .collect();
        let c = coalesce(&rows);
        assert_eq!(c.len(), 2, "4 MiB at a 2 MiB bound is two chunks: {}", c.len());
        assert!(c.iter().all(|k| k.len <= DRAIN_CHUNK_MAX));
        assert_covers(&rows);
    }

    #[test]
    fn a_single_row_is_a_single_chunk_and_the_empty_list_is_no_chunks() {
        assert!(coalesce(&[]).is_empty());
        let rows = [(0x2_0000_0000, 0x1_0000_0000, 0x1000)];
        let c = coalesce(&rows);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].rows, 1);
        assert_eq!(c[0].last_row_va, 0x2_0000_0000);
    }

    /// ⊘ **The census must never call an empty table `contiguous`.** Same class as `dlen=0`:
    /// an absent measurement that decodes to the favourable answer.
    #[test]
    fn the_census_refuses_to_speak_for_an_empty_table() {
        let s = drain_contiguity(&[]);
        assert!(s.contains("UNMEASURED"), "{s}");
        assert!(!s.contains("pair_runs="), "{s}");
    }

    /// The census and the coalescer must agree about how many chains there are, up to the
    /// 2 MiB split — two implementations of one fact, checked against each other.
    #[test]
    fn the_census_pair_runs_and_the_coalescers_chunk_count_agree() {
        let rows = [
            (0x2_0000_0000u64, 0x1_0000_0000u64, 0x1000u64),
            (0x2_0000_1000, 0x1_0000_1000, 0x1000),
            (0x2_0000_2000, 0x7_0000_0000, 0x1000),
            (0x2_0000_3000, 0x7_0000_1000, 0x1000),
            (0x2_0000_4000, 0x9_0000_0000, 0x1000),
        ];
        let s = drain_contiguity(&rows);
        assert!(s.contains("pair_runs=3"), "{s}");
        assert_eq!(coalesce(&rows).len(), 3, "{s}");
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

/// ★★★★★ **w328 — THE SCOPING PREDICATE'S KNOWN-POSITIVE AND KNOWN-NEGATIVE PAIRS.**
///
/// This tree's own banked lesson: *a census zero needs a known-positive*. The dangerous
/// failure of [`publish_scope_scoped`] is not that it scopes when it should not — that is a
/// slow boot. It is that it scopes to **nothing**, publishing no VAS at all, which presents
/// as a GPU fault indistinguishable from `the_drain_budget_truncation.md`'s pre-existing
/// intermittent. Both of its refusals are therefore asserted here, offline, with no GPU and
/// no environment variable in the path.
#[cfg(all(test, feature = "host-isolates"))]
mod w328_scope_predicate_tests {
    use super::publish_scope_scoped;

    fn pdb(v: u64) -> kayfabe_rt::Pdb {
        kayfabe_rt::Pdb(v)
    }

    /// The KNOWN-NEGATIVE: the default arm never scopes, whatever the target is.
    #[test]
    fn the_default_arm_is_master_and_never_scopes() {
        assert!(!publish_scope_scoped("all", None));
        assert!(!publish_scope_scoped(
            "all",
            Some((kayfabe_core::ProcId(2), pdb(0x6000)))
        ));
        // ⊘ Anything that is not the exact word is `all`. A typo'd launcher must fall back to
        // master's breadth, never to a half-armed state.
        assert!(!publish_scope_scoped(
            "doorbelled ",
            Some((kayfabe_core::ProcId(2), pdb(0x6000)))
        ));
    }

    /// The KNOWN-POSITIVE: an armed arm with a real, non-system target does scope.
    #[test]
    fn an_armed_arm_with_a_real_target_scopes() {
        assert!(publish_scope_scoped(
            "doorbelled",
            Some((kayfabe_core::ProcId(2), pdb(0x6000)))
        ));
    }

    /// ⚠⚠ **REFUSAL 1 — NO TARGET ⇒ NO SCOPING.** A doorbell that resolved no channel facts
    /// names no VAS; scoping to a VAS we cannot name is scoping to none, and would publish
    /// nothing at all — strictly worse than master and presenting as a GPU fault.
    #[test]
    fn no_target_falls_back_to_full_breadth() {
        assert!(
            !publish_scope_scoped("doorbelled", None),
            "★★★★★ scoping with no target publishes NOTHING; the fallback to full breadth is \
             the safety property of this rung and not an optimisation"
        );
    }

    /// ⚠⚠ **REFUSAL 2 — A `SYSTEM_PROC` TARGET ⇒ NO SCOPING.** §12.26: proc 0 is never
    /// attempted by either pass. Scoping to it would leave every publishable VAS unvisited
    /// while the log line still read `scoped=true`, which is the favourable-looking absence
    /// this tree has paid for repeatedly.
    #[test]
    fn a_system_proc_target_falls_back_to_full_breadth() {
        assert!(
            !publish_scope_scoped(
                "doorbelled",
                Some((kayfabe_core::gpu::Gpu::SYSTEM_PROC, pdb(0x200000)))
            ),
            "★★★★★ proc 0 is NEVER ATTEMPTED by either pass; scoping to it scopes to nothing"
        );
    }
}
