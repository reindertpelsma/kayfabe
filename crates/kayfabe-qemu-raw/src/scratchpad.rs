//! ★★★★★ **THE VM-LIFETIME SCRATCHPAD ISOLATE, AND THE ONE OBJECT IT RESERVES** —
//! `SINGLE_STORE_PLAN.md` increment 1, `gpga_is_one_reserved_object.md`.
//!
//! # ⊘ What did not exist before this module
//!
//! `[surveyed w721, re-checked here]` the device model is entirely per-proc — `procs:
//! BTreeMap<ProcId, RankedMutex<Proc>>` — and an isolate is spawned **lazily, on the
//! guest's first RM event** (`kayfabe_core::gpu::Spine::defer_isolate`, the "E0b" lazy
//! spawn). **Nothing in kayfabe lived for the VM.** The reserved object, and later a CUDA
//! context and the walk kernel, must exist **before the guest's first instruction**, so
//! there had to be an isolate that is not any guest process's.
//!
//! ⇒ This one belongs to the `(vm, gpu)` pair. It is **owned by the shell**, not installed
//! into a `Proc`, which is deliberate:
//!
//! - `Spine::install_isolate` is keyed by `(ProcId, GpuId)`, and there is no `ProcId` that
//!   means *"the VM"*. Borrowing `Gpu::SYSTEM_PROC` would give it one, and the system proc
//!   is a real participant in the guest's object graph — its isolate is reaped when it
//!   quiesces, which is precisely the lifetime this must not have.
//! - Three tests exist specifically to keep isolate spawns guest-caused
//!   (`tests/tests/isolate_spawn_is_guest_caused.rs` and its two neighbours). They assert a
//!   property of the **device**: that realizing a `Gpu` spawns nothing. That property is
//!   still true, and this module must not make it false — it spawns **beside** the device,
//!   never through it.
//!
//! # ★★★ Why spawning here is a latency WIN and not just a lifetime change
//!
//! `[measured w470]` the isolate spawn is the recorded worst vCPU stall in the device:
//! `Regs::write → SharedDevice::materialize_pending → materialize_one →
//! HostIsolateFactory::spawn_host → proto::read_frame → UnixStream::read → __recv` — a
//! literal blocking socket read, **inside the guest's MMIO exit**, `worst_trap` 1.62 s.
//! A vCPU inside an MMIO exit is not preemptible (`the_write_trap_contract`), so that stall
//! freezes a core of the guest.
//!
//! ⇒ Doing the same spawn at device-realize costs the guest **nothing**, because the guest
//! is not running yet. ⊘ This does not *remove* the lazy spawn — the per-proc isolates are
//! still spawned on first touch, and this increment deletes nothing (`SINGLE_STORE_PLAN.md`'s
//! sequencing rule). It adds one isolate that never had to be lazy.
//!
//! # ⚠ The three invariants this module is written against
//!
//! 1. **No blocking work on the vCPU thread.** [`Scratchpad::bring_up`] takes an
//!    [`OffTrap`](kayfabe_util::trapwitness::OffTrap) witness, which **panics** if the
//!    calling thread is inside a guest trap. It is not a comment; a future caller that
//!    moved this onto a trap path would be told by name.
//! 2. **No blocking work under any lock, on any thread.** Everything here runs with zero
//!    ranked locks held — `Worker::execute`/`with_rm` assert exactly that
//!    (`assert_lock_free`), and `IsolateBox::new`/`drop` assert it again at birth and death.
//!    The shell holds this in a plain field, so nothing about reaching it takes a lock.
//! 3. **Re-validate after re-arming a lock.** Not applicable: this path takes none.
//!
//! # ⊘ It is OFF by default, and the census says so on BOTH arms
//!
//! [`SCRATCHPAD_ENV`] gates the whole module. With the gate off nothing is spawned, nothing
//! is reserved, and the advertised framebuffer size is the compile-time one — byte for byte
//! the behaviour that shipped. The census line is printed either way, because a
//! configuration that only announces itself when enabled makes the control arm's log
//! indistinguishable from an older binary's.

use std::sync::Arc;

use kayfabe_rt::GpuId;
use kayfabe_isolate::{HostHandle, IsolateBox, IsolateFactory, IsolateId, RmError};
use kayfabe_util::trapwitness::OffTrap;
use crate::shim::Status;

/// ★★★★★ **The gate.** Three arms, and the third is the design's own rule made reachable.
///
/// | value | what happens |
/// |---|---|
/// | unset / `off` | **the default.** Nothing is spawned, nothing is reserved, and the
/// advertised framebuffer is the compiled one. Byte for byte what shipped. |
/// | `on` | spawn the VM-lifetime isolate, reserve, **report**. A refused reservation is
/// recorded in the census and the VM starts anyway. |
/// | `require` | as `on`, and a reservation that is not held **refuses the device** —
/// `gpga_is_one_reserved_object.md`: *"If that fails, the VM does not start."* |
///
/// # ⊘ Why `on` and `require` are two arms and not one
///
/// The design's rule is `require`, and it is right for the product. It is **wrong for the
/// measurement this increment exists to take**: a device that refuses to realize produces no
/// teardown census, no guest, and no answer to *"what would the host have given us?"* — it
/// produces a QEMU that exits with a status. ⇒ `on` is how the question gets asked and
/// `require` is how the answer gets enforced, and collapsing them would mean the first
/// failed reservation destroys the evidence about why it failed.
///
/// ⊘ A value naming none of the three is **refused**, not defaulted. Both directions of a
/// silent default are bad here and they are bad in opposite ways: defaulting a typo to `off`
/// makes a boot the operator believes is armed run the control arm, and defaulting it to
/// `on` reserves the host's entire framebuffer on a boot nobody asked for it on.

/// ★★★★★ **w734 — THE MEASURED READ RATE THROUGH A DEVICE VIEW OF THE RESERVED OBJECT**, in
/// bytes per second, or **0 for "never measured on this boot"**.
///
/// ⊘⊘ Zero is not a slow rate and must never be read as one. It means the probe did not run
/// (its gate is off) or refused — and the FB-IO census says which of `MEASURED` and `ASSUMED`
/// it used, rather than silently multiplying by somebody else's number. That distinction is
/// the entire point of w734: the switch's cost has been quoted for two documents as a
/// measured fact when only one of its two terms was ever measured.
pub static DEVICE_VIEW_READ_BPS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub const SCRATCHPAD_ENV: &str = "KAYFABE_SCRATCHPAD";

/// Which arm of [`SCRATCHPAD_ENV`] this boot runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScratchpadArm {
    /// The default. Nothing is spawned and nothing changes.
    Off,
    /// Spawn, reserve, report — and start the VM either way.
    Measure,
    /// Spawn, reserve, and refuse the device if the reservation is not held.
    Require,
}

impl ScratchpadArm {
    /// Whether this arm spawns anything at all.
    #[must_use]
    pub fn is_armed(self) -> bool {
        !matches!(self, ScratchpadArm::Off)
    }

    /// The token this arm prints in the census.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ScratchpadArm::Off => "off",
            ScratchpadArm::Measure => "on",
            ScratchpadArm::Require => "require",
        }
    }
}

/// ★★★★★ **THE CUDA GATE** — `SINGLE_STORE_PLAN.md` increment 4, `THE_CONSTRAINTS.md`
/// §w724d. `off` (the default) | `on`.
///
/// When `on`, the VM-lifetime scratchpad isolate is spawned from a **glibc-linked** second
/// image, brings CUDA all the way up **before** entering its sandbox — `cuInit` →
/// `cuDeviceGet` → `cuCtxCreate` → `cuModuleLoadData` (the PTX JIT) → a real launch against a
/// synthetic page-table image it builds itself — and then runs two probes **after** the
/// namespace and the privilege drop.
///
/// # ⊘ A PEER of [`SCRATCHPAD_ENV`] and not a third arm of it
///
/// The two are orthogonal and a boot must be able to arm either alone: the reservation is
/// about **video memory**, this is about **a process's build and its sandbox ordering**. ⊘ A
/// third arm would also have made `require` mean two things at once.
///
/// ⚠ It is a strict extension: with it off, the scratchpad isolate is the same static-musl
/// image every other isolate is, sandboxed before it touches anything.
pub const SCRATCHPAD_CUDA_ENV: &str = "KAYFABE_SCRATCHPAD_CUDA";

/// Whether `value` arms the CUDA scratchpad — the pure half, and the only statement of the
/// default.
///
/// # Errors
/// [`Status::Unsupported`] if `value` names neither state. **Absent is not an error**; it is
/// `false`.
pub fn scratchpad_cuda_from(value: Option<&str>) -> Result<bool, (Status, &'static str)> {
    match value {
        None | Some("off") => Ok(false),
        Some("on") => Ok(true),
        Some(_) => Err((
            Status::Unsupported,
            "KAYFABE_SCRATCHPAD_CUDA does not name a state: the only values are `off` (the \
             default) and `on`. It is not defaulted, because arming it makes ONE isolate \
             dynamically linked and sandboxed LATE — a change to a security boundary must be \
             asked for, never fallen into by a typo.",
        )),
    }
}

/// Whether this process arms the CUDA scratchpad.
///
/// # Errors
/// Whatever [`scratchpad_cuda_from`] refused with.
pub fn selected_scratchpad_cuda() -> Result<bool, (Status, &'static str)> {
    let raw = std::env::var_os(SCRATCHPAD_CUDA_ENV);
    let value = raw
        .as_ref()
        .map(|v| v.to_str().unwrap_or("\u{fffd}invalid"));
    scratchpad_cuda_from(value)
}

/// ★★★★★ **THE DEVICE-VIEW CROSSING'S ARM** — `off` (the default) | `probe`.
///
/// `probe` exercises, in a real boot, the crossing the owner's ruling of 2026-09-14
/// authorised (`bar1_passthrough_device_local_host_visible.md` §4 item 1): the scratchpad
/// isolate arms a CPU view of **the reserved object**, hands the `/dev/nvidia<N>` node to the
/// VMM over `SCM_RIGHTS`, the VMM `mmap`s it, **closes the descriptor immediately**, reads and
/// writes a word through it, and releases the view.
///
/// ⊘ It does **not** install a guest memslot. Placing one inside BAR1 without the mirror that
/// decides *which* guest page maps where would be inventing a layout, and a boot that failed
/// would not distinguish "the crossing is broken" from "we put it in the wrong place". The
/// mapping is made with `GuestWindow::place_device_view` — the exact verb
/// `QemuMachine::install_device_window` uses — so what is proven is the whole chain up to the
/// memslot call, and the memslot call itself is already exercised by the BAR0 counter page.
///
/// ⚠ **EXPIRY**: deleted when the BAR1 mirror drives `install_device_window` in production —
/// at that point the crossing is exercised by the thing that uses it, and a probe that
/// duplicates it is the *"unwired but still compiling"* cruft §w724g names.
pub const DEVICE_VIEW_ENV: &str = "KAYFABE_DEVICE_VIEW";

/// Whether `value` arms the device-view probe — the pure half, and the only statement of the
/// default.
///
/// # Errors
/// [`Status::Unsupported`] if `value` names neither state. **Absent is not an error**; it is
/// `false`.
pub fn device_view_from(value: Option<&str>) -> Result<bool, (Status, &'static str)> {
    match value {
        None | Some("off") => Ok(false),
        Some("probe") => Ok(true),
        Some(_) => Err((
            Status::Unsupported,
            "KAYFABE_DEVICE_VIEW does not name a state: the only values are `off` (the \
             default) and `probe`. It is not defaulted, because arming it makes this process \
             hold a `/dev/nvidia<N>` descriptor — transiently, and only under the owner's \
             conditional ruling — and that is not something a typo should decide.",
        )),
    }
}

/// Whether this process arms the device-view probe.
///
/// # Errors
/// Whatever [`device_view_from`] refused with.
pub fn selected_device_view() -> Result<bool, (Status, &'static str)> {
    let raw = std::env::var_os(DEVICE_VIEW_ENV);
    let value = raw
        .as_ref()
        .map(|v| v.to_str().unwrap_or("\u{fffd}invalid"));
    device_view_from(value)
}

/// ★★ **Where the reservation probe starts halving from, in MiB.** Only read when
/// [`SCRATCHPAD_ENV`] is armed.
///
/// ⊘ It is a *starting point*, not a request: the probe halves down to a floor and then
/// bisects, so a start above the card's capacity costs one refused ioctl and nothing else.
/// The default is the GA106 bench card's advertised 12 GiB.
pub const SCRATCHPAD_START_MB_ENV: &str = "KAYFABE_SCRATCHPAD_START_MB";

/// The default start of the reservation probe, in MiB — a 12 GiB GA106.
pub const DEFAULT_START_MB: u64 = 12_288;

/// Whether `value` arms the scratchpad — the pure half of [`selected_scratchpad`], and the
/// **only** statement of the default.
///
/// ⊘ It is a free function taking the value rather than reading the environment itself, for
/// the reason `selected_dirty_gate`'s post-mortem records one module over: two statements of
/// one default, in one file, with the caller silently winning, is this tree's most expensive
/// recurring class. The caller here does nothing but supply the environment.
///
/// # Errors
/// [`Status::Unsupported`] if `value` names neither state. **Absent is not an error**; it is
/// `false`.
pub fn scratchpad_from(value: Option<&str>) -> Result<ScratchpadArm, (Status, &'static str)> {
    match value {
        None | Some("off") => Ok(ScratchpadArm::Off),
        Some("on") => Ok(ScratchpadArm::Measure),
        Some("require") => Ok(ScratchpadArm::Require),
        Some(_) => Err((
            Status::Unsupported,
            "KAYFABE_SCRATCHPAD does not name a state: the only values are `off` (the \
             default), `on` (spawn, reserve, report) and `require` (as `on`, and refuse the \
             device if the reservation is not held). It is not defaulted, because both \
             directions are wrong in opposite ways — a typo defaulted to `off` runs the \
             control arm on a boot the operator believes is armed, and one defaulted to `on` \
             reserves the host's whole framebuffer on a boot nobody asked for it on.",
        )),
    }
}

/// Whether this process arms the scratchpad.
///
/// # Errors
/// Whatever [`scratchpad_from`] refused with.
pub fn selected_scratchpad() -> Result<ScratchpadArm, (Status, &'static str)> {
    let raw = std::env::var_os(SCRATCHPAD_ENV);
    let value = raw
        .as_ref()
        .map(|v| v.to_str().unwrap_or("\u{fffd}invalid"));
    scratchpad_from(value)
}

/// Where the reservation probe starts, in MiB.
///
/// ⊘ An unparseable or zero value falls back to [`DEFAULT_START_MB`] and **says so**: this
/// one is a tuning knob for a probe that refuses gracefully, not a switch that changes what
/// the boot means, so refusing the VM over it would be the wrong severity.
#[must_use]
pub fn selected_start_mb() -> u64 {
    match std::env::var(SCRATCHPAD_START_MB_ENV) {
        Err(_) => DEFAULT_START_MB,
        Ok(v) => match v.parse::<u64>() {
            Ok(mb) if mb > 0 => mb,
            _ => {
                eprintln!(
                    "kayfabe: SCRATCHPAD ⚠ {SCRATCHPAD_START_MB_ENV}={v} is not a positive \
                     number of MiB — using the default {DEFAULT_START_MB}"
                );
                DEFAULT_START_MB
            }
        },
    }
}

/// ★★★ **What became of the reservation.** Five outcomes, and they are deliberately five
/// rather than an `Option<u64>`.
///
/// ⊘ *"No size"* is not one fact. The isolate never coming up, the probe being unavailable,
/// the probe answering **zero**, and the reservation refusing **after** a probe said a size
/// was available are four different statements about the host, and collapsing them into
/// `None` is how an empty artefact comes to read as benign. They are distinguished here so
/// the census can name which one happened.
#[derive(Debug)]
pub enum Reservation {
    /// The isolate came up with no worker to check out — a stillborn plane, or a pool of
    /// zero. Nothing was asked of RM at all.
    NoWorker {
        /// The isolate's own refusal text, if it has one.
        why: String,
    },
    /// The probe verb itself refused. On a backend with no RM connection this is the
    /// trait's named default (`NV_ERR_NOT_SUPPORTED`), which is the honest answer.
    ProbeRefused {
        /// How RM (or the seam) refused.
        why: String,
    },
    /// ★ The probe ran and answered **zero MiB**: nothing down to its floor could be
    /// reserved. ⊘ A real measurement about this host, and not the same fact as
    /// [`Reservation::ProbeRefused`].
    NothingReservable,
    /// The probe named a size and the reservation of that size then refused.
    Refused {
        /// What the probe said was available, in MiB.
        probed_mb: u64,
        /// How the reservation refused.
        why: String,
    },
    /// ★★★★★ The object is **held**, for the life of this VM.
    Held {
        /// Its size in MiB — the number the guest's framebuffer size is derived from.
        mb: u64,
        /// The handle, in the scratchpad isolate's namespace.
        obj: HostHandle,
    },
}

impl Reservation {
    /// The reserved size in MiB, or `None` if nothing is held.
    #[must_use]
    pub fn held_mb(&self) -> Option<u64> {
        match self {
            Reservation::Held { mb, .. } => Some(*mb),
            _ => None,
        }
    }

    /// A short token for the census. ⊘ Every arm has one, including the successful one, so a
    /// grader greps a value rather than an absence.
    #[must_use]
    pub fn token(&self) -> &'static str {
        match self {
            Reservation::NoWorker { .. } => "NO_WORKER",
            Reservation::ProbeRefused { .. } => "PROBE_REFUSED",
            Reservation::NothingReservable => "NOTHING_RESERVABLE",
            Reservation::Refused { .. } => "RESERVE_REFUSED",
            Reservation::Held { .. } => "HELD",
        }
    }

    /// The refusal text, for the census. Empty when there is nothing to explain.
    ///
    /// ⊘ The `RESERVE_REFUSED` arm carries **`probed_mb` as well as the error**, because
    /// *"reserving 11 808 MiB refused"* and *"reserving 512 MiB refused"* are opposite facts
    /// about the host and the token alone cannot tell them apart. A refusal line without the
    /// size attempted is a refusal nobody can act on.
    #[must_use]
    pub fn why(&self) -> String {
        match self {
            Reservation::NoWorker { why } | Reservation::ProbeRefused { why } => why.clone(),
            Reservation::Refused { why, probed_mb } => {
                format!("the probe said {probed_mb} MiB and reserving it refused: {why}")
            }
            Reservation::NothingReservable | Reservation::Held { .. } => String::new(),
        }
    }
}

/// ★★★★★ **The VM-lifetime scratchpad isolate.** One per `(vm, gpu)`, spawned at device
/// realize, dropped when the device is.
///
/// ⚠ Dropping this drops the [`IsolateBox`], which kills and reaps the child process — and
/// that frees the reserved object with it, because the whole RM object tree hangs off the
/// isolate's client. ⇒ **the reservation's lifetime IS this struct's lifetime**, with no
/// separate release step to forget. `IsolateBox::drop` asserts lock-free, so this must not
/// be dropped under a ranked lock.

/// ★★★★★ **w734 — THE IDENTITY-WINDOW INVARIANT, MADE CATCHABLE BY A BOOT.**
///
/// > **`SINGLE_STORE_PLAN.md` §5's expiry note:** *"§3 makes GPGA one reserved device-local
/// > object which `gpga_is_one_reserved_object.md` has the scratchpad map **whole, at a fixed
/// > base** — an address is `X + gpga_offset`. ⇒ **after §3 the window is identity and
/// > `build_image` is retired.**"*
///
/// An identity window means the walk kernel can dereference a guest page-table address
/// **directly**, with one bounds check against the window's length, and no relocation, no
/// staged image, no H2D copy and no `MAX_REFRESHES` ceiling. Everything that retires with §3
/// retires *because of this one property*.
///
/// # ⊘⊘⊘ AND IT IS DESTROYED BY ADVERTISING MORE THAN WAS RESERVED
///
/// The kernel addresses table pages as offsets into one flat window
/// (`KfArgs::win = {base, len}`, every dereference bounds-checked against `win.len`). A window
/// that answers for the guest's tables **at their own GPGA** must be as long as the highest
/// table page's address. ⇒ if the guest is told it has `N` bytes of framebuffer and we hold
/// fewer than `N`, its RM will place tables at addresses **outside the object** — the guest's
/// own RM puts them at the **top** — and the kernel's bounds check refuses every one of them.
///
/// ⚠ **The margin is thin by design and it is measured, not assumed.** `[measured]`
/// `span_pages → 3868.7 MiB` inside `RESERVED_MB=4096`: **94.5 % of what the guest was told**,
/// 227.3 MiB of headroom. ⇒ this is not a comfortable inequality with a safety factor; it is
/// an invariant that holds because `derived_from_reservation` moves the guest's tables **down
/// with the reservation**, and it fails the moment anything advertises independently of it.
///
/// ⊘ w730's *"the tables live ~11.8 GiB up a 12 GiB board, so an identity window is
/// unaffordable"* is a fact about the **advertised** size, not about the design. Advertise
/// what was reserved and the same 94.5 % lands inside it.
///
/// # ★ Two checks, at two moments, and they answer different questions
///
/// | | asks | when |
/// |---|---|---|
/// | [`identity_window_verdict`] | *"could the guest even place a table outside the object?"* | **at realize**, before the guest's first instruction — the only moment an operator can act |
/// | [`identity_window_reached`] | *"did it?"* | at teardown, from the arena's own high-water |
///
/// ⊘ The first is the invariant; the second is the **known-positive for the first**. A verdict
/// that says *"identity is possible"* on every boot and is never confronted with what the
/// guest actually did is a check that reports rather than one that gates — the failure this
/// tree names most often. ⚠ And the second alone would be useless: by teardown the boot is
/// over, and a table outside the window is a kernel that refused every dereference, not a
/// number somebody reads afterwards.
///
/// Returns `(identity_is_possible, the sentence)`.
#[must_use]
pub fn identity_window_verdict(advertised_fb_bytes: u64, reserved_bytes: u64) -> (bool, String) {
    let mib = |b: u64| b as f64 / (1024.0 * 1024.0);
    if reserved_bytes == 0 {
        return (
            false,
            "IDENTITY-WINDOW ⊘ NO RESERVATION — nothing is held, so there is no object for a \
             window to be the identity of. ⊘ This is not `identity is impossible`; it is the \
             question not arising, and the fake framebuffer is still what backs the guest."
                .to_string(),
        );
    }
    if advertised_fb_bytes <= reserved_bytes {
        (
            true,
            format!(
                "IDENTITY-WINDOW ✔ POSSIBLE — advertised={:.1} MiB ≤ reserved={:.1} MiB, \
                 headroom={:.1} MiB. ⇒ every framebuffer address the guest can name is an \
                 offset into the reserved object, so the walk kernel's window can be IDENTITY \
                 and relocation, the staged image and its H2D copy, and MAX_REFRESHES all \
                 become retirable. ⚠ Possible, not achieved: `identity_window_reached` at \
                 teardown is what says the guest's own tables landed inside.",
                mib(advertised_fb_bytes),
                mib(reserved_bytes),
                mib(reserved_bytes - advertised_fb_bytes),
            ),
        )
    } else {
        (
            false,
            format!(
                "IDENTITY-WINDOW ⊘⊘⊘ IMPOSSIBLE — advertised={:.1} MiB > reserved={:.1} MiB, \
                 over by {:.1} MiB. ⚠ The guest's own RM places its page tables at the TOP of \
                 what it is told it has, so this is not a rounding worry: the tables will land \
                 OUTSIDE the object and the walk kernel's bounds check will refuse every one \
                 of them. ⇒ Advertise what was reserved (`derived_from_reservation`), or hold \
                 more.",
                mib(advertised_fb_bytes),
                mib(reserved_bytes),
                mib(advertised_fb_bytes - reserved_bytes),
            ),
        )
    }
}

/// ★★★ **DID THE GUEST ACTUALLY STAY INSIDE?** — [`identity_window_verdict`]'s known-positive,
/// taken from the page arena's own high-water rather than from anything this check controls.
///
/// `span_bytes` is the highest framebuffer address the guest caused a page to exist at, which
/// is the arena's `span_pages × 4 KiB`. ⊘ It is a **forward** measurement: the arena indexes
/// by framebuffer address (*"framebuffer address = file offset"*), so its high-water is the
/// guest's own answer to *"how high did you go"* and consults nothing this verdict computed.
///
/// ⚠ `span_bytes == 0` is **VACUOUS**, not a pass. It means no page was ever arena-backed on
/// this boot — the mirror was off, or the arena refused everything — and a run that read it as
/// *"the guest stayed well inside"* would be reading the absence of a measurement as its
/// best possible result.
#[must_use]
pub fn identity_window_reached(span_bytes: u64, reserved_bytes: u64) -> String {
    let mib = |b: u64| b as f64 / (1024.0 * 1024.0);
    if span_bytes == 0 {
        return "IDENTITY-REACHED ⊘⊘ VACUOUS — the page arena's high-water is zero, so no \
                framebuffer page was ever arena-backed on this boot. That is the absence of a \
                measurement, NOT the guest staying inside the reservation."
            .to_string();
    }
    if reserved_bytes == 0 {
        return format!(
            "IDENTITY-REACHED ⊘ NO RESERVATION — the guest reached {:.1} MiB of framebuffer \
             and nothing is held to compare it against.",
            mib(span_bytes)
        );
    }
    let pct = span_bytes as f64 * 100.0 / reserved_bytes as f64;
    if span_bytes <= reserved_bytes {
        format!(
            "IDENTITY-REACHED ✔ INSIDE — the guest's highest framebuffer address was \
             {:.1} MiB, {pct:.1} % of the {:.1} MiB reserved, leaving {:.1} MiB. ★ This is \
             the known-positive for the identity window: the guest's own tables fit in the \
             object, so an identity window would have answered for all of them.",
            mib(span_bytes),
            mib(reserved_bytes),
            mib(reserved_bytes - span_bytes),
        )
    } else {
        format!(
            "IDENTITY-REACHED ⊘⊘⊘ OUTSIDE — the guest reached {:.1} MiB, which is \
             {:.1} MiB ABOVE the {:.1} MiB reserved ({pct:.1} %). ⇒ an identity window CANNOT \
             answer for this boot's tables, whatever the realize-time verdict said, and \
             anything that retires on the strength of identity must NOT be retired.",
            mib(span_bytes),
            mib(span_bytes - reserved_bytes),
            mib(reserved_bytes),
        )
    }
}

#[derive(Debug)]
pub struct Scratchpad {
    /// Which arm of [`SCRATCHPAD_ENV`] this one was brought up under. ⊘ Carried on the
    /// struct rather than re-read, so the census cannot name an arm the bring-up did not run.
    arm: ScratchpadArm,
    id: IsolateId,
    /// ⊘ `Option` only so [`Scratchpad::retire`] can take it and drop it deliberately at a
    /// point of our choosing. It is `Some` for the whole ordinary life of the struct —
    /// **unless** [`Scratchpad::share_for_walk_shadow`] has moved it into
    /// [`Self::walk_shadow`], which is the live shadow's arm.
    iso: Option<IsolateBox>,
    /// ★★★ **The live walk shadow's port**, holding this isolate, when
    /// `KAYFABE_WALK_SHADOW` is on (`SINGLE_STORE_PLAN.md` §6 step 1).
    ///
    /// ⊘ The isolate MOVES here rather than being borrowed. `IsolateBox::checkout` needs
    /// `&mut`, and the sweep reaches the port through a cloned `SharedDoorbell` that cannot
    /// hold a mutable borrow of this struct. One owner behind an `Arc<Mutex<..>>` is the
    /// honest shape; two borrows of one box is not a shape at all.
    ///
    /// ⚠ The reservation hangs off this isolate's RM client either way, so its lifetime is
    /// still exactly this struct's — the `Arc` is cloned only into the doorbell port, which
    /// the device owns.
    walk_shadow: Option<std::sync::Arc<crate::walkshadow::WalkShadowPort>>,
    outcome: Reservation,
    /// Wall time inside `IsolateFactory::spawn`, in microseconds — the quantity w470
    /// measured on the vCPU, measured here where the guest does not pay it.
    spawn_us: u64,
    /// Wall time inside the reservation probe, in microseconds.
    probe_us: u64,
    /// Wall time inside the reservation itself, in microseconds.
    reserve_us: u64,
    /// Whether the isolate offered a worker at all — kept beside [`Self::outcome`] because
    /// `pool_size` can be non-zero on an isolate whose every slot is already retired.
    pool: usize,
    /// ★★★ What the device-view crossing did, for the census. Empty when
    /// [`DEVICE_VIEW_ENV`] is off.
    device_view: String,
    /// ★★★ What the isolate's CUDA bring-up reported, verbatim. Empty when
    /// [`SCRATCHPAD_CUDA_ENV`] is off.
    ///
    /// ⊘ A string and not a parsed struct: every field in it is for a human reading a boot
    /// log, nothing branches on it, and a new field in the isolate must not be a change to
    /// this crate.
    cuda: String,
}

impl Scratchpad {
    /// ★★★ **Spawn the isolate and reserve the object.** Blocking, and deliberately so: at
    /// device realize the guest does not exist yet, which is the whole argument for doing it
    /// here.
    ///
    /// # Panics
    /// Through [`OffTrap::claim`], if a caller ever moves this onto a guest-trap thread.
    /// That is not recoverable and must not be: the spawn is a fork plus a blocking socket
    /// read, and inside an MMIO exit it freezes a core of the guest.
    #[must_use]
    pub fn bring_up(
        factory: &Arc<dyn IsolateFactory>,
        gpu: GpuId,
        start_mb: u64,
        arm: ScratchpadArm,
        cuda: bool,
        device_view: Option<&DupFn>,
    ) -> Scratchpad {
        // ⊘ The witness is claimed FIRST, before anything blocking happens, so the assertion
        // fires at the top of the operation rather than partway through one.
        let off = OffTrap::claim("bringing up the VM-lifetime scratchpad isolate");

        // ★ `IsolateId`'s proc field is a bare `u32` and this one is **not a `ProcId`**. It
        // is `u32::MAX` — the same value `IsolateId::NONE` uses — so that if this id ever
        // reaches a per-proc lookup it collides with no live proc rather than aliasing
        // `ProcId(0)`, the system proc, whose isolate has a different lifetime entirely.
        let id = IsolateId::new(SCRATCHPAD_PROC, gpu);

        let t0 = std::time::Instant::now();
        // ⊘ `IsolateBox::new` asserts lock-free and leaf-free at birth. We hold neither.
        let mut iso = IsolateBox::new(factory.spawn(id));
        let spawn_us = micros(t0);

        let pool = iso.pool_size();
        let refusal = iso.refusal().map(|r| format!("{:?}: {}", r.kind, r.why));

        let mut probe_us = 0;
        let mut reserve_us = 0;
        let mut cuda_report = String::new();
        let mut device_view_report = String::new();
        let outcome = match iso.checkout() {
            None => Reservation::NoWorker {
                why: refusal.unwrap_or_else(|| {
                    "the isolate offered no worker and gave no reason".to_string()
                }),
            },
            Some(mut worker) => {
                let t1 = std::time::Instant::now();
                let probed = worker.with_rm(&off, |rm| rm.largest_reservable_mb(start_mb));
                probe_us = micros(t1);
                let outcome = match probed {
                    Err(e) => Reservation::ProbeRefused { why: why(&e) },
                    Ok(0) => Reservation::NothingReservable,
                    Ok(mb) => {
                        let t2 = std::time::Instant::now();
                        // ⊘⊘ **NOT `alloc_vidmem`.** `SINGLE_STORE_PLAN.md` §2 says the verb
                        // already exists as `Request::AllocVidmem` (wire tag 19); it does
                        // not. That verb is `ATTR_CONTIGUOUS_VIDMEM` with `alignment: len`,
                        // and a contiguous, 11.8 GiB-aligned 11.8 GiB request refuses on
                        // fragmentation — a refusal that is not about capacity, on the one
                        // call whose refusal is supposed to mean *"there is not enough
                        // video memory"*. See `RmBackend::reserve_gpga`.
                        let got = worker.with_rm(&off, |rm| rm.reserve_gpga(mb << 20));
                        reserve_us = micros(t2);
                        match got {
                            Ok(obj) => Reservation::Held { mb, obj },
                            Err(e) => Reservation::Refused {
                                probed_mb: mb,
                                why: why(&e),
                            },
                        }
                    }
                };
                // ★★★★★ **THE DEVICE-VIEW CROSSING**, on the same worker, immediately
                // after the reservation — because the object it views is the one just
                // reserved, and a probe over a different object would prove a different
                // thing.
                if let (Some(dup), Reservation::Held { obj, .. }) = (device_view, &outcome) {
                    device_view_report = probe_device_view(&mut worker, &off, id, *obj, dup);
                }
                // ★★★ THE CUDA REPORT, read off the same worker. ⊘ It is a READ: the
                // bring-up and both probes already ran at the isolate's startup, before and
                // after its sandbox. Nothing here can cause them, which is the point —
                // by the time a worker answers anything, the isolate is already sandboxed.
                if cuda {
                    cuda_report = match worker.with_rm(&off, |rm| rm.cuda_walk_report()) {
                        Ok(line) => line,
                        Err(e) => format!("CUDA_WALK=UNREPORTED why={e:?}"),
                    };
                }
                // ⊘ The worker goes back whatever happened. A slot left checked out is a
                // pool that never quiesces, which turns a failed reservation into a hang at
                // teardown — a second, unrelated failure attributed to the first.
                iso.checkin(worker);
                outcome
            }
        };

        Scratchpad {
            arm,
            device_view: device_view_report,
            cuda: cuda_report,
            id,
            iso: Some(iso),
            walk_shadow: None,
            outcome,
            spawn_us,
            probe_us,
            reserve_us,
            pool,
        }
    }

    /// The reserved size in MiB, or `None` if nothing is held.
    #[must_use]
    pub fn reserved_mb(&self) -> Option<u64> {
        self.outcome.held_mb()
    }

    /// What became of the reservation.
    #[must_use]
    pub fn outcome(&self) -> &Reservation {
        &self.outcome
    }

    /// What the device-view crossing did, verbatim. Empty when its gate is off.
    #[must_use]
    pub fn device_view_report(&self) -> &str {
        &self.device_view
    }

    /// What the isolate's CUDA bring-up reported, verbatim. Empty when the CUDA gate is off.
    #[must_use]
    pub fn cuda_report(&self) -> &str {
        &self.cuda
    }

    /// This isolate's id.
    #[must_use]
    pub fn id(&self) -> IsolateId {
        self.id
    }

    /// ★★★ **THE CENSUS LINE.** Printed at `at`, on every arm, including the arms where
    /// nothing was reserved.
    ///
    /// ⊘ `up=` is a fact about the isolate and `reservation=` is a fact about RM, and they
    /// are separate fields because *"the isolate came up and RM refused"* and *"the isolate
    /// never came up"* are the two diagnoses this whole increment has to be able to tell
    /// apart. A single "failed" would make them one silence.
    pub fn census(&self, at: &str) {
        // ★★★★★ THE DEVICE-VIEW LINE, on its own and on both arms. ⊘ Separate from the
        // reservation and the CUDA lines because the three gates are independent: a reader
        // must be able to see "the object is held, CUDA is off, the crossing worked" without
        // parsing one line for three facts.
        eprintln!(
            "kayfabe: SCRATCHPAD-DEVICE-VIEW AT {at}: {}",
            if self.device_view.is_empty() {
                "DEVICE_VIEW=DISARMED (set KAYFABE_DEVICE_VIEW=probe to arm)".to_string()
            } else {
                self.device_view.clone()
            }
        );
        // ★★★ THE CUDA LINE, on its own, and printed on BOTH arms. ⊘ Separate from the
        // reservation line because the two gates are independent: a reader must be able to
        // see "the object is held and CUDA is off" without parsing one line for two facts.
        eprintln!(
            "kayfabe: SCRATCHPAD-CUDA AT {at}: {}",
            if self.cuda.is_empty() {
                "CUDA_WALK=DISARMED (set KAYFABE_SCRATCHPAD_CUDA=on to arm)".to_string()
            } else {
                self.cuda.clone()
            }
        );
        let mb = self.reserved_mb().unwrap_or(0);
        eprintln!(
            "kayfabe: SCRATCHPAD AT {at}: arm={arm} up={up} proc={proc} gpu={gpu} pool={pool} \
             reservation={token} RESERVED_MB={mb} spawn_ms={spawn:.3} probe_ms={probe:.3} \
             reserve_ms={reserve:.3}{why} ⇒ {verdict}",
            arm = self.arm.as_str(),
            up = self.iso.is_some() || self.walk_shadow.is_some(),
            proc = self.id.proc(),
            gpu = self.id.gpu().0,
            pool = self.pool,
            token = self.outcome.token(),
            spawn = self.spawn_us as f64 / 1000.0,
            probe = self.probe_us as f64 / 1000.0,
            reserve = self.reserve_us as f64 / 1000.0,
            why = if self.outcome.why().is_empty() {
                String::new()
            } else {
                format!(" why=\"{}\"", self.outcome.why())
            },
            verdict = match &self.outcome {
                Reservation::Held { .. } => "the VM's video memory is ONE host RM object",
                _ => "NO reserved object — the advertised framebuffer is the compile-time one",
            },
        );
    }

    /// The census line printed when the gate is **off**. A free function on the type so the
    /// two arms' lines are written next to each other and cannot drift apart.
    ///
    /// ⊘ Printed unconditionally, because a boot's log must distinguish *"the control arm"*
    /// from *"a binary that predates the arm"*.
    pub fn census_disarmed(at: &str) {
        eprintln!(
            "kayfabe: SCRATCHPAD AT {at}: arm=off up=false pool=0 reservation=DISARMED \
             RESERVED_MB=0 ⇒ no VM-lifetime isolate; the advertised framebuffer is the \
             compile-time one (set {SCRATCHPAD_ENV}=on to arm)"
        );
    }

    /// ★★★ **The `require` arm's refusal** — `gpga_is_one_reserved_object.md`: *"If that
    /// fails, the VM does not start."*
    ///
    /// `Ok(())` on the `on` arm whatever happened, and on the `require` arm only when the
    /// object is held.
    ///
    /// # Errors
    /// [`Status::Unsupported`] with the reason, when `require` is armed and nothing is held.
    /// ⊘ The census line has already been printed by the time this is consulted, so the
    /// refusal never costs the reader the diagnosis of *why* — which is the whole reason
    /// `on` and `require` are separate arms.
    pub fn enforce(&self) -> Result<(), (Status, &'static str)> {
        enforce_arm(self.arm, &self.outcome)
    }

    /// Retire and drop the isolate deliberately, before the rest of the shell goes.
    ///
    /// ⚠ Must be called with **no ranked lock held** — `IsolateBox::drop` asserts it. Safe
    /// to call twice; the second call does nothing.
    pub fn retire(&mut self) {
        if let Some(mut iso) = self.iso.take() {
            iso.retire();
        }
        // ⊘ Reached through the `Arc` when the live shadow took the box. Retiring is a
        // statement to the isolate, not a drop, so doing it here is correct even though the
        // doorbell port still holds a handle — and the port's own `Arc` going away later is
        // what actually reaps the child.
        if let Some(p) = self.walk_shadow.take() {
            p.retire();
        }
    }

    /// ★★★★★ **HAND THE ISOLATE TO THE LIVE WALK SHADOW** — `SINGLE_STORE_PLAN.md` §6 step 1.
    ///
    /// Returns the port to clone into the doorbell, or `None` when there is no isolate to
    /// give (the bring-up got no worker, or this was already called).
    ///
    /// ⊘ **One isolate per VM, and this is it.** §4 is explicit: *"No other isolate loads
    /// CUDA (~135 MB of mappings and hundreds of ms of context creation). One walk isolate
    /// per VM."* Spawning a second one for the shadow would be a second CUDA context and a
    /// second reservation-holding client.
    pub fn share_for_walk_shadow(
        &mut self,
    ) -> Option<std::sync::Arc<crate::walkshadow::WalkShadowPort>> {
        if self.walk_shadow.is_none() {
            let iso = self.iso.take()?;
            self.walk_shadow = Some(std::sync::Arc::new(
                crate::walkshadow::WalkShadowPort::new(iso),
            ));
        }
        self.walk_shadow.clone()
    }

    /// The shadow's census line, or `None` when the shadow was never armed.
    #[must_use]
    pub fn walk_shadow_census(&self) -> Option<String> {
        self.walk_shadow.as_ref().map(|p| p.census_line())
    }
}

/// ★★★ **The `require` rule, as a pure function — and the ONLY statement of it.**
///
/// ⊘ Separated from [`Scratchpad::enforce`] so it can be tested over all five outcomes
/// without a GPU, a factory or a child process. A rule that can only be exercised by booting
/// is a rule whose every arm but one is unmeasured, and the arms that matter here are the
/// four failing ones.
///
/// # Errors
/// [`Status::Unsupported`] when `require` is armed and nothing is held.
pub fn enforce_arm(
    arm: ScratchpadArm,
    outcome: &Reservation,
) -> Result<(), (Status, &'static str)> {
    match (arm, outcome) {
        (ScratchpadArm::Require, Reservation::Held { .. }) | (ScratchpadArm::Measure, _) => Ok(()),
        (ScratchpadArm::Require, _) => Err((
            Status::Unsupported,
            "KAYFABE_SCRATCHPAD=require and the one reserved video-memory object was NOT \
             held. `gpga_is_one_reserved_object.md`: the guest's framebuffer is one host RM \
             object or the VM does not start — an allocation that can fail later, on a \
             refresh path where nothing can recover, is what reserving up front exists to \
             make impossible. The SCRATCHPAD census line printed immediately above names \
             which step refused.",
        )),
        // ⊘ Unreachable: `Off` never builds a `Scratchpad` at all. Stated rather than
        // silently folded into an `Ok`, so a future arm cannot inherit permissiveness.
        (ScratchpadArm::Off, _) => Ok(()),
    }
}

/// ⊘ **Not a `ProcId`.** See [`Scratchpad::bring_up`] — `u32::MAX` is the value
/// `IsolateId::NONE` uses, chosen so this id can never alias a live proc's.
///
/// ★★★ **It is also the discriminator `kayfabe-isolate-host` uses to decide which isolate
/// gets the glibc-linked CUDA image.** That crate sits BELOW this one and cannot import the
/// constant, so it restates it as `SCRATCHPAD_ISOLATE_PROC`; the two are pinned equal by
/// `the_scratchpad_proc_id_agrees_across_the_seam`.
pub const SCRATCHPAD_PROC: u32 = u32::MAX;

fn micros(t: std::time::Instant) -> u64 {
    u64::try_from(t.elapsed().as_micros()).unwrap_or(u64::MAX)
}

/// Render an [`RmError`] for the census. ⊘ `Debug` and not a classification: the census
/// prints what happened, and a caller that wants to branch has the enum.
fn why(e: &RmError) -> String {
    format!("{e:?}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_is_off_and_is_not_an_error() {
        assert_eq!(scratchpad_from(None), Ok(ScratchpadArm::Off));
    }

    #[test]
    fn all_three_arms_parse() {
        assert_eq!(scratchpad_from(Some("off")), Ok(ScratchpadArm::Off));
        assert_eq!(scratchpad_from(Some("on")), Ok(ScratchpadArm::Measure));
        assert_eq!(scratchpad_from(Some("require")), Ok(ScratchpadArm::Require));
        assert!(!ScratchpadArm::Off.is_armed());
        assert!(ScratchpadArm::Measure.is_armed());
        assert!(ScratchpadArm::Require.is_armed());
    }

    /// ⊘ The gate refuses a value it does not recognise rather than defaulting it. A typo
    /// that silently ran the control arm would be a boot the operator believes is armed.
    #[test]
    fn a_value_naming_neither_state_is_refused() {
        assert!(scratchpad_from(Some("1")).is_err());
        assert!(scratchpad_from(Some("yes")).is_err());
        assert!(scratchpad_from(Some("")).is_err());
    }

    /// ★ The five outcomes are five, and each has its own census token. A test over the
    /// tokens is what stops a later edit collapsing two of them into one word.
    #[test]
    fn every_outcome_has_its_own_token() {
        let outcomes = [
            Reservation::NoWorker { why: String::new() },
            Reservation::ProbeRefused { why: String::new() },
            Reservation::NothingReservable,
            Reservation::Refused {
                probed_mb: 6144,
                why: String::new(),
            },
            Reservation::Held {
                mb: 11_808,
                obj: HostHandle::NULL,
            },
        ];
        let tokens: std::collections::BTreeSet<&str> =
            outcomes.iter().map(Reservation::token).collect();
        assert_eq!(tokens.len(), outcomes.len(), "two outcomes share a token");
    }

    /// ⊘ Only the held arm reports a size. `NothingReservable` in particular must not read
    /// as "zero megabytes were reserved and that is fine".
    #[test]
    fn only_the_held_arm_reports_a_size() {
        assert_eq!(Reservation::NothingReservable.held_mb(), None);
        assert_eq!(
            Reservation::Refused {
                probed_mb: 6144,
                why: String::new()
            }
            .held_mb(),
            None
        );
        assert_eq!(
            Reservation::Held {
                mb: 11_808,
                obj: HostHandle::NULL
            }
            .held_mb(),
            Some(11_808)
        );
    }

    /// ★★★ **The `require` rule over ALL FIVE outcomes.** The four failing ones are what
    /// this rule is for, and they are the ones a boot would never exercise on a healthy box.
    #[test]
    fn require_refuses_every_outcome_but_held_and_measure_refuses_none() {
        let failing = [
            Reservation::NoWorker { why: String::new() },
            Reservation::ProbeRefused { why: String::new() },
            Reservation::NothingReservable,
            Reservation::Refused {
                probed_mb: 6144,
                why: String::new(),
            },
        ];
        for o in &failing {
            assert!(
                enforce_arm(ScratchpadArm::Require, o).is_err(),
                "`require` must refuse {}",
                o.token()
            );
            // ⊘ The measuring arm refuses NOTHING — that is the whole reason it is a
            // separate arm: a device that refuses to realize leaves no census behind.
            assert!(
                enforce_arm(ScratchpadArm::Measure, o).is_ok(),
                "`on` must not refuse {}",
                o.token()
            );
        }
        let held = Reservation::Held {
            mb: 11_808,
            obj: HostHandle::NULL,
        };
        assert!(enforce_arm(ScratchpadArm::Require, &held).is_ok());
        assert!(enforce_arm(ScratchpadArm::Measure, &held).is_ok());
    }

    /// ⊘ The scratchpad's isolate id must not alias any proc's. `ProcId` is dense from 0, so
    /// the test that matters is that this one is not small.
    #[test]
    fn the_scratchpad_id_cannot_alias_a_live_proc() {
        assert_eq!(SCRATCHPAD_PROC, u32::MAX);
        let id = IsolateId::new(SCRATCHPAD_PROC, GpuId::ZERO);
        assert_ne!(id, IsolateId::new(0, GpuId::ZERO));
    }
}

/// How the shell turns an isolate-minted export token into a descriptor in **this** process.
///
/// ⊘ A function and not the `ExportDirectory` itself, so this module needs no dependency on
/// the isolate-host crate's registry types and so a test can supply a double.
pub type DupFn = dyn Fn(IsolateId, u64) -> Option<std::os::fd::OwnedFd>;

/// ★★★★★ **THE CROSSING, EXERCISED IN A REAL BOOT** — the owner's ruling of 2026-09-14
/// (`bar1_passthrough_device_local_host_visible.md` §4 item 1) turned into working code.
///
/// Arms a CPU view of the **reserved object**, receives the `/dev/nvidia<N>` node over
/// `SCM_RIGHTS`, `mmap`s it through the same verb `QemuMachine::install_device_window` uses,
/// writes and reads back a word, and releases the view.
///
/// # ⚠ THE RULING'S THREE CONDITIONS, AND WHERE EACH ONE LIVES
///
/// 1. **No escape on the descriptor — only `mmap`.** ⊘ Enforced by construction *here*: the
///    only thing done with `fd` below is `place_device_view`, and the binding it came from
///    exposes no way to issue an escape. This is the whole of the safety argument, and it is a
///    property of this function's body.
/// 2. **Closed the moment `mmap` returns.** `drop(fd)` is immediate and is the line after the
///    mapping, not the end of a scope. The VMA keeps the `struct file`, so the mapping
///    outlives it — which the read-back below then *proves*, because a mapping over a closed
///    descriptor that did not survive would fault rather than answer.
/// 3. **The crossing is `SCM_RIGHTS`** — inside `ProxyRmBackend::call_for_device_view`.
///
/// ⊘ And the release is not optional: `[measured w722]` `munmap` + `close` returns **nothing**
/// to the host's BAR1 pool, silently.
#[cfg(feature = "host-isolates")]
fn probe_device_view(
    worker: &mut kayfabe_isolate::Worker,
    off: &OffTrap,
    id: IsolateId,
    obj: HostHandle,
    dup: &DupFn,
) -> String {
    use kayfabe_linux_raw::{GuestWindow, HostOffset, HostPageSize};
    use std::os::fd::AsFd;

    // ⊘ One page, at offset 0. The probe's job is the CROSSING, not capacity: a large view
    // would consume BAR1 aperture that the sizing constraint (§w727) has already shown is
    // scarce, and would prove nothing the first page does not.
    let page = HostPageSize::query().bytes();
    let view = match worker.export_device_view(obj, 0, page, true) {
        Ok(v) => v,
        Err(e) => return format!("DEVICE_VIEW=EXPORT_REFUSED why={e:?}"),
    };
    let Some(fd) = dup(id, view.token) else {
        // ⊘ Release first: the view exists in the isolate whether or not we can see it, and
        // leaking it would consume aperture for the rest of the boot.
        let _ = worker.release_device_view(&view);
        return format!(
            "DEVICE_VIEW=NO_DESCRIPTOR token={} ⇒ the isolate minted a view this process \
             could not dup; the export directory does not know that isolate",
            view.token
        );
    };
    let _ = off;

    let win = match GuestWindow::create(view.mmap_len, HostPageSize::query()) {
        Ok(w) => w,
        Err(e) => {
            drop(fd);
            let _ = worker.release_device_view(&view);
            return format!("DEVICE_VIEW=NO_WINDOW why={e:?}");
        }
    };
    // ★★ The same verb `install_device_window` uses. `writable = true` is the VMA's
    // protection — what THIS process may do — and is independent of any guest slot tier.
    let placed = win.place_device_view(HostOffset::ZERO, view.mmap_len, fd.as_fd(), true);
    // ★★★ **CONDITION 2, and it is this line.** The descriptor is closed the instant the
    // mapping exists — not at the end of the scope, not on the error path only.
    drop(fd);
    if let Err(e) = placed {
        let _ = worker.release_device_view(&view);
        return format!("DEVICE_VIEW=MMAP_REFUSED why={e:?}");
    }

    // ★★★ The read-back is what makes condition 2 a MEASUREMENT rather than an assertion: if
    // the VMA had not survived the `close`, this would fault instead of answering.
    const SENTINEL: u32 = 0xD0DE_0001;
    let wrote = win.store_u32(HostOffset::ZERO, SENTINEL);
    let mut buf = [0u8; 4];
    let read = win.read_into(HostOffset::ZERO, &mut buf).map(|()| u32::from_le_bytes(buf));
    let released = worker.release_device_view(&view);

    // ★★★★★ **w734 — THE RATE, MEASURED ON THE PATH THAT WILL CARRY IT.**
    //
    // ⊘⊘⊘ `SINGLE_STORE_PLAN.md` and `THE_CONSTRAINTS.md` §w724c both cost the switch as
    // `bytes ÷ 48 MiB/s`. `[surveyed w734]` that 48 MiB/s is quoted in both with no citation
    // to a measurement of THIS path — a CPU `memcpy` out of a device view of the **reserved
    // object** — and the whole "there is no working intermediate, it does not boot" ruling is
    // that rate times an unmeasured byte volume. w734b measured the volume. This measures the
    // rate, on the same boot, through the same verb, over the same object.
    //
    // ⚠ It costs BAR1 aperture for its duration and gives it straight back through the
    // release verb (`[measured w722]` `munmap` + `close` returns **nothing**), and it costs
    // wall time at realize, where the guest does not exist yet. ⊘ Both are why it is a
    // bounded probe and not a sweep.
    let rate = probe_device_view_rate(worker, off, id, obj, dup);

    match (wrote, read) {
        (Ok(()), Ok(got)) if got == SENTINEL => format!(
            "DEVICE_VIEW=OK {rate} mmap_len=0x{:x} sentinel_roundtrip=true released={} ⇒ the \
             scratchpad isolate armed a view of the RESERVED OBJECT, the node crossed by \
             SCM_RIGHTS, this process mapped it, CLOSED the descriptor, and the mapping \
             survived — the owner's conditional ruling, exercised end to end",
            view.mmap_len,
            released.is_ok(),
        ),
        (Ok(()), Ok(got)) => format!(
            "DEVICE_VIEW=WRONG_VALUE wrote=0x{SENTINEL:x} read=0x{got:x} released={} ⇒ the \
             mapping answered, and with the wrong bytes. ⚠ That is worse than a refusal: it \
             means this is not the memory we think it is.",
            released.is_ok()
        ),
        (Err(e), _) | (_, Err(e)) => format!(
            "DEVICE_VIEW=IO_REFUSED why={e:?} released={}",
            released.is_ok()
        ),
    }
}

/// ★★★★★ **w734 — HOW FAST IS A CPU `memcpy` THROUGH A DEVICE VIEW OF THE RESERVED OBJECT?**
///
/// # ⊘⊘⊘ Why this number, and not the one already written down
///
/// `SINGLE_STORE_PLAN.md`'s ordering rule (*"§6 MUST PRECEDE §3"*) and `THE_CONSTRAINTS.md`
/// §w724c's *"it does not boot"* are the same arithmetic: **store bytes ÷ 48 MiB/s**. w734b
/// measured the numerator for the first time. ⊘ The denominator is quoted in both documents
/// with no citation to a measurement of **this** path, and a rate measured on some other
/// aperture is exactly the input this tree keeps being burned by — *a ruling's date and its
/// architecture are both part of the citation*.
///
/// ⇒ This measures it where it will be paid: [`kayfabe_isolate::Worker::export_device_view`]
/// over the reserved object → `SCM_RIGHTS` → `place_device_view` → `copy_nonoverlapping`,
/// which is byte for byte what a `read`/`write` on a device-backed `FbStore` would do.
///
/// # ★★★ Four numbers, and the last two are the ones nobody has costed
///
/// | | what it decides |
/// |---|---|
/// | `rd` MiB/s | the walk's cost after the switch — the plan's whole ordering argument |
/// | `wr` MiB/s | `kbusVerifyBar2`, the CPU CE executor, every boot-time store write |
/// | `arm_us` | ⚠ **per view.** A device view is mapped from file offset **0 only**, so a non-contiguous working set needs ONE ARMED NODE PER RUN — this is the per-run tax, and nothing had costed it |
/// | `rel_us` | the same on the way out; and `[measured w722]` skipping it returns **zero** aperture |
///
/// ⊘ `PROBE_BYTES` is deliberately small. §22 item 3 measured host BAR1 at 256 MiB **shared
/// with the host driver**, so a probe sized to impress would compete with the thing it is
/// measuring for. 2 MiB is ~0.8 % of the aperture and is released immediately.
///
/// ⚠ **EXPIRY (§w724g):** deleted when a device-backed `FbStore` exists and reports its own
/// throughput from production traffic. At that point this measures a path the device already
/// measures, and two sources for one fact is the defect this tree names most often.
#[cfg(feature = "host-isolates")]
fn probe_device_view_rate(
    worker: &mut kayfabe_isolate::Worker,
    off: &OffTrap,
    id: IsolateId,
    obj: HostHandle,
    dup: &DupFn,
) -> String {
    use kayfabe_linux_raw::{GuestWindow, HostOffset, HostPageSize};
    use std::os::fd::AsFd;

    /// 2 MiB — see the doc comment. ⊘ Not a tunable: a knob here would make two boots'
    /// numbers incomparable without anyone noticing which arm they were read from.
    const PROBE_BYTES: u64 = 2 * 1024 * 1024;
    /// ⊘ Three passes, and the **minimum** is reported rather than the mean. A `memcpy` out
    /// of an uncached device mapping has a floor and a long tail (scheduler, host-driver
    /// contention); the floor is a property of the bus and the tail a property of the box. A
    /// mean blends them and moves run to run.
    const PASSES: u32 = 3;

    let t_arm = std::time::Instant::now();
    let view = match worker.export_device_view(obj, 0, PROBE_BYTES, true) {
        Ok(v) => v,
        Err(e) => return format!("rate=UNMEASURED why=EXPORT_REFUSED:{e:?}"),
    };
    let Some(fd) = dup(id, view.token) else {
        let _ = worker.release_device_view(&view);
        return "rate=UNMEASURED why=NO_DESCRIPTOR".to_string();
    };
    let win = match GuestWindow::create(view.mmap_len, HostPageSize::query()) {
        Ok(w) => w,
        Err(e) => {
            drop(fd);
            let _ = worker.release_device_view(&view);
            return format!("rate=UNMEASURED why=NO_WINDOW:{e:?}");
        }
    };
    let placed = win.place_device_view(HostOffset::ZERO, view.mmap_len, fd.as_fd(), true);
    // ★ Condition 2 of the ruling, here too: the descriptor goes the instant the mapping
    // exists — not at the end of the scope, not on the error path only.
    drop(fd);
    let arm_us = micros(t_arm);
    if let Err(e) = placed {
        let _ = worker.release_device_view(&view);
        return format!("rate=UNMEASURED why=MMAP_REFUSED:{e:?} arm_us={arm_us}");
    }
    let _ = off;

    let n = view.mmap_len.min(PROBE_BYTES);
    let Ok(len) = usize::try_from(n) else {
        let _ = worker.release_device_view(&view);
        return "rate=UNMEASURED why=LENGTH_NOT_HOST_SIZED".to_string();
    };
    let mut buf = vec![0u8; len];
    let mut rd_us = u64::MAX;
    let mut wr_us = u64::MAX;
    let mut failed: Option<String> = None;
    for _ in 0..PASSES {
        let t = std::time::Instant::now();
        if let Err(e) = win.read_into(HostOffset::ZERO, &mut buf) {
            failed = Some(format!("READ:{e:?}"));
            break;
        }
        // ⊘ `.max(1)` guards the division below, and it is a FLOOR on the reported time, so
        // it can only make the rate look SLOWER than it was. A guard that flattered the
        // number would be the one direction that matters here.
        rd_us = rd_us.min(micros(t).max(1));
        let t = std::time::Instant::now();
        // ★ Writing back exactly what was read leaves the object's bytes unchanged, which
        // matters: this runs over the reserved object the guest's framebuffer will live in.
        if let Err(e) = win.write_from(HostOffset::ZERO, &buf) {
            failed = Some(format!("WRITE:{e:?}"));
            break;
        }
        wr_us = wr_us.min(micros(t).max(1));
    }

    let t_rel = std::time::Instant::now();
    let released = worker.release_device_view(&view).is_ok();
    let rel_us = micros(t_rel);
    drop(win);

    if let Some(why) = failed {
        return format!("rate=UNMEASURED why={why} arm_us={arm_us} rel_us={rel_us}");
    }
    let mibps = |us: u64| (n as f64) * 1e6 / (us as f64) / (1024.0 * 1024.0);
    // ★ Published so the FB-IO census can multiply the volume it MEASURED by a rate that was
    // also measured, on this boot, on this board — instead of by the 48 MiB/s the plan
    // inherited. ⊘ Reads only; the write rate is reported but not published, because the
    // census's dominant term is the walk and the walk reads.
    DEVICE_VIEW_READ_BPS.store(
        ((n as f64) * 1e6 / (rd_us as f64)) as u64,
        std::sync::atomic::Ordering::Relaxed,
    );
    format!(
        "rate[rd={rd:.1}MiB/s wr={wr:.1}MiB/s over={kib}KiB arm_us={arm_us} rel_us={rel_us} released={released}]",
        rd = mibps(rd_us),
        wr = mibps(wr_us),
        kib = n / 1024,
    )
}

/// ⊘ No isolate plane, no view, no rate — said by name, because *an unmeasured rate* and *a
/// slow one* are the two facts this probe exists to keep apart.
#[cfg(not(feature = "host-isolates"))]
fn probe_device_view_rate(
    _worker: &mut kayfabe_isolate::Worker,
    _off: &OffTrap,
    _id: IsolateId,
    _obj: HostHandle,
    _dup: &DupFn,
) -> String {
    "rate=UNMEASURED why=NO_ISOLATE_PLANE".to_string()
}

/// ⊘ Without the isolate plane there is no isolate to arm a view in, and this arm says so by
/// name rather than being absent — an unarmed boot and a boot built without the feature are
/// different facts.
#[cfg(not(feature = "host-isolates"))]
fn probe_device_view(
    _worker: &mut kayfabe_isolate::Worker,
    _off: &OffTrap,
    _id: IsolateId,
    _obj: HostHandle,
    _dup: &DupFn,
) -> String {
    "DEVICE_VIEW=NO_ISOLATE_PLANE reason=\"this binary was built without `host-isolates`\""
        .to_string()
}
