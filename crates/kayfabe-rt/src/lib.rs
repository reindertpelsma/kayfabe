//! # kayfabe-rt — the L1 threaded shell (stage 2 of L1-M1)
//!
//! The thin threaded wrapper `l1_concurrency.md` §2 draws around the pure core:
//! two ranked locks, one inbox, one executor. This crate is an **adapter** — it is
//! deliberately outside the CI hexagonal-boundary grep list (it exists to own
//! locks and threads-adjacent structure the logic crates must never contain) —
//! but it is still pure `std`: **no OS waiting primitive, no host descriptor, no
//! syscall lives here.** Stage 3 / L2 supply the real reactor loop and the real
//! notifiable source; the seams they plug into ([`inbox::Inbox::try_pop`]
//! returning `None`, [`device::SharedDevice::take_pending_wake`]) are explicit.
//!
//! - [`lock`] — the ranked locks and the ASSERTED invariants **R1** (no blocking
//!   under any lock — [`lock::BlockingSection`]) and **R3** (strictly-increasing
//!   rank, one lock per rank), always-on, per §3.3.
//! - [`device`] — [`device::SharedDevice`]: the lock-swap over the core's #35
//!   sharding shape, in BOTH configurations ([`device::LockMode`]) from day one
//!   (§8.2 / review P5), with the `Mutex::get_mut` mechanic that lets spine ops
//!   drive every proc with zero rank-1 acquisitions.
//! - [`inbox`] — the executor inbox (rank 2, the one concurrent L1 structure),
//!   producer side typed to touch zero core state (inherited law 9).
//! - [`executor`] — drains [`inbox::CoreEvent`]s against the device under the
//!   normal locks; `SourceRegistry::dispatch` → typed effect, faults surfaced
//!   loudly.
//!
//! The lock discipline this crate enforces at runtime is exactly the one
//! `tests/tests/concurrency_stress.rs` documents as convention for the mock
//! harness — the ranks make it mechanical (§2: "the same discipline … the ranks
//! make it mechanical").
//!
//! ## ★ The `*_unsafe.rs` naming rule (workspace-wide; CI-enforced)
//!
//! Stated here because this is the adapter side of the hexagon — when the OS shell
//! grows real syscalls (`kayfabe-linux-raw`), *this* is the neighbourhood the one
//! audited relaxation will live in.
//!
//! The workspace sets `unsafe_code = "forbid"` (root `Cargo.toml`,
//! `[workspace.lints.rust]`) and **that does not change**: the lint is what bans the
//! escape hatch. The naming rule is what makes the eventual *exception* auditable:
//!
//! > **An auditor must be able to enumerate the entire escape-hatch surface with
//! > `ls`.** Every `.rs` file that uses the keyword is named `*_unsafe.rs`.
//!
//! CI enforces it (`.github/workflows/ci.yml`, the *Unsafe-surface gate*), and it is
//! deliberately blunt: a mention in a comment, doc or string trips it too. That is not
//! pedantry — a gate whose verdict depends on reading intent is one that eventually
//! gets mis-read, and the cost is one-sided (prose can be reworded; a block cannot).
//! The gate greps whole words, so naming the **lint** (`unsafe_code`) or the **suffix**
//! (`_unsafe.rs`) never trips it. Writing about the rule therefore stays possible
//! without an allowlist — which is exactly why there is no allowlist to negotiate.

/// ★ E10c — the shell's CPU copy-engine executor. Ordinary safe code: it accesses guest
/// memory only through the `Vmm`/`FbStore` traits, which re-validate against bounds the
/// audited raw crates own (`l1_os_shell.md` §4.1/§4.2.1.1), so the unsound surface stays
/// contained and this file carries no `_unsafe` suffix.
pub mod ceutils;
pub mod completion_watch;
pub mod cpu_ce;
pub mod device;
pub mod executor;
pub mod inbox;
pub mod lock;

/// ★★★★ §16.65 — the doorbell **routing verdict** and the per-engine census's shape,
/// re-exported for the same reason [`GpuId`] is.
///
/// ⊘ [`device::DoorbellRoute`] rather than `EngineKind` itself: the shim must be able to
/// act on *which executor owns this doorbell* without naming an engine vocabulary, and
/// re-exporting the verdict keeps `kayfabe-arch` out of its manifest exactly as that
/// manifest demands. The count and the labels come with it because a census that could not
/// print an empty bucket would report a partition it had not measured.
pub use device::{DoorbellRoute, ENGINE_KIND_COUNT, engine_kind_names};
/// ★★★★★ §16.96 — the engine-object **deferral** vocabulary, re-exported for exactly the
/// reason [`ClassId`] and [`FwdFault`] are: the QEMU shim is the frame that drains the latch
/// (`Regs::write`, the outermost frame on the vCPU's MMIO trap holding no ranked lock), so it
/// must be able to NAME both what was admitted and what the drain then did — including the
/// bound's refusal, which is a fact about **us** and would otherwise be unprintable.
pub use device::{EngineForwardRun, ForwardAdmission, MAX_PENDING_ENGINE_FORWARDS};
/// ★★★★★ The GR passthrough route's decision half — what the shell's port DOES with a
/// routing verdict, re-exported for [`DoorbellRoute`]'s own reason one step further along.
///
/// ⊘ [`device::shell_disposition`] rather than the shim re-deriving it: §16.65's whole
/// finding was that a routing rule written twice comes to disagree, and the shim's copy was
/// a `!=` against one variant — which is why `HostGr` and `Unserved` shared a bucket they do
/// not belong in.
pub use device::{ShellDisposition, shell_disposition};
/// ★★★★★ **w288** — where a guest channel asked to be told about its own death, re-exported
/// for [`Pdb`]'s single reason: the QEMU shim is the **only** party that may turn the GPA in
/// `Sysmem { gpa }` into a `kayfabe_isolate::GuestRamGrant`, and its own manifest forbids an
/// edge to `kayfabe-arch` (*"the shim names no architecture"*).
///
/// ⊘ A three-state enum over an address; it carries no architecture. And the three states
/// must survive the crossing intact — `Unreachable` is a gap in **us** and `None` is the
/// guest waiving error reporting, which lead a reader to different files.
pub use kayfabe_arch::fault::ErrorNotifier;
/// ★★★★★ §16.80 — the class id and the two Case-1 engine-object types, re-exported for
/// the same reason and under the same rule: `SharedDevice::forward_engine_object_by_parent`
/// takes and returns them, and the QEMU shim implements `ObjectModel` over it. A
/// [`ClassId`] is a newtype over a `u32` and carries no architecture; `FwdFault` is the
/// forwarding plane's own refusal vocabulary, which the shim must be able to PRINT — a
/// refusal the composition root can only report as "an error" is the shape
/// `a_wall_that_can_carry_no_name` records.
pub use kayfabe_arch::ids::ClassId;
/// ★★★★★ **w288 TIER 2** — the control-command newtype
/// [`device::SharedDevice::relay_channel_control`] takes, re-exported for [`Pdb`]'s single
/// reason: the QEMU shim implements the port over it and its own manifest forbids an edge to
/// `kayfabe-arch` (*"the shim names no architecture"*). ⊘ A newtype over an integer; naming
/// a command id is not naming an architecture.
pub use kayfabe_arch::ids::ControlCmd;
/// ★ The id this shell's own entry points **require** a caller to name, re-exported.
///
/// `SharedDevice::doorbell` takes a [`GpuId`], so a composition root cannot call it without
/// naming one — and a crate whose public API demands a type it does not re-export forces
/// every consumer to take a dependency it otherwise would not (`kayfabe-qemu-raw`'s
/// manifest says at length why it declines to depend on `kayfabe-arch`). Re-exporting the
/// id is API hygiene, not a new edge: it is a plain newtype over an integer and carries no
/// architecture with it.
pub use kayfabe_arch::ids::GpuId;
/// ★ §5.8 — the address type [`device::SharedDevice::pin_guest_ram`] takes, re-exported for
/// the same reason and under the same rule: the shim must be able to NAME a guest VA
/// without taking an edge to `kayfabe-arch`, which its own manifest forbids. ⊘ A newtype
/// over an integer; it carries no architecture.
pub use kayfabe_arch::ids::GpuVa;
/// ★★★★★ **LEG A** — the page-directory-base identity, re-exported for the same single
/// reason [`HClient`]/[`HObject`] are: the QEMU shim must be able to *name* the address
/// space a framebuffer leaf is joined into, and its own manifest forbids an edge to
/// `kayfabe-arch` (*"the shim names no architecture"*). ⊘ A newtype over an integer; it
/// carries no architecture, and re-exporting it keeps that manifest rule true rather than
/// negotiating it.
pub use kayfabe_arch::ids::Pdb;
/// ★ #177 — the two handle types `SharedDevice::schedule_channel` takes, re-exported so
/// the QEMU shim (which does not depend on `kayfabe-arch`) can name them.
pub use kayfabe_arch::ids::{HClient, HObject};
/// ★★★★★ **w288 TIER 2** — the outcome types of
/// [`device::SharedDevice::relay_channel_control`], re-exported for [`Pdb`]'s reason: the
/// policy layer and the QEMU shim both have to NAME them, and neither may take an edge
/// this one already owns.
pub use kayfabe_fwd::{
    ChannelControlRelay, ChannelControlRelayFault, EngineObjectForwarded, FbLeafBacking,
    FbLeafRange, FwdFault,
};

// The concurrency contract (decision #17), compile-time-asserted for the shell's
// public types. `BlockingSection` is deliberately ABSENT: it is `!Send` by
// construction (its asserts are against the constructing thread's lock state).
// Guards are `!Send` as all `std` guards are — the rank bookkeeping relies on it.
kayfabe_util::assert_send_sync!(
    lock::LockRank,
    lock::RankedRwLock<u8>,
    lock::RankedMutex<u8>,
    device::SharedDevice,
    device::LockMode,
    device::SignalOutcome,
    inbox::CoreEvent,
    inbox::InboxSender,
    inbox::Inbox,
    executor::Effect,
    executor::Executor,
    executor::Parker,
);
kayfabe_util::assert_send_sync!(dyn executor::ExecutorWaker);
