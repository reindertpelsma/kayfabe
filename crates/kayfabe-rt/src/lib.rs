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

pub mod device;
pub mod executor;
pub mod inbox;
pub mod lock;

/// ★ The id this shell's own entry points **require** a caller to name, re-exported.
///
/// `SharedDevice::doorbell` takes a [`GpuId`], so a composition root cannot call it without
/// naming one — and a crate whose public API demands a type it does not re-export forces
/// every consumer to take a dependency it otherwise would not (`kayfabe-qemu-raw`'s
/// manifest says at length why it declines to depend on `kayfabe-arch`). Re-exporting the
/// id is API hygiene, not a new edge: it is a plain newtype over an integer and carries no
/// architecture with it.
pub use kayfabe_arch::ids::GpuId;
/// ★ #177 — the two handle types `SharedDevice::schedule_channel` takes, re-exported so
/// the QEMU shim (which does not depend on `kayfabe-arch`) can name them.
pub use kayfabe_arch::ids::{HClient, HObject};

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
