//! # `kayfabe-shell` — the L1 **OS shell**: the real reactor loop
//!
//! `l1_os_shell.md` §3 (decision 8), stage **M2-d**. The core owns the completion-source
//! *model* (`kayfabe_core::reactor`: an opaque `CompletionSource`, a `SourceRegistry`, a
//! typed `Dispatch`, a `WakeRequest`); this crate owns everything below it — the table
//! from a handle to a real host readiness primitive, the real wait, and the real wake.
//!
//! ```text
//!   isolate relay / worker death / notify
//!            │  (host readiness primitives WE own — §3.5(b))
//!            ▼
//!      ┌───────────┐   drain    ┌─────────────┐   SourceSignal   ┌────────────┐
//!      │  Reactor  │──────────► │  Registrar  │ ───────────────► │   Inbox    │
//!      │  (thread) │            │ (handle→fd) │                  │            │
//!      └───────────┘                                             └──────┬─────┘
//!         no core state (law 9)                                         │ wake
//!                                                                       ▼
//!                                                            Executor (its own thread)
//!                                                            dispatch → Device::event
//! ```
//!
//! Two sections carry the weight and both are worth reading before this code:
//! [`sources`] for **why the loop has a table at all** (a correction to §3.2, and a second
//! one to §3.4(c)), and [`reactor`] for **the F1 wake-count gate as a quantity**.
//!
//! ## What this stage deliberately did NOT build
//!
//! Named, because an honest absence is a design statement:
//!
//! - **The core-side source cap.** §3.8 asks for `SourceRegistry::register` to grow a
//!   `Result` with a per-`(proc, gpu)` bound. The **shell** half is here and real — the
//!   device-global cap and the descriptor budget derived from the process's actual
//!   `RLIMIT_NOFILE` ([`sources::Registrar::bound`]) — and it is the half that keeps a
//!   hostile arm loop from exhausting descriptors in the VMM process. The per-`(proc, gpu)`
//!   half is a core signature change that touches every arming call site, and it belongs in
//!   the same change as the guest-driven arming path it bounds, which does not exist yet.
//! - **`SourceKind::IsolateExit`.** §3.1's new class, and the process descriptor behind it.
//!   There is no isolate process to watch until M2-d's sibling stage lands one; the class
//!   would be a variant nothing constructs.
//! - **The isolate relay thread** (§3.5(b)). This crate's [`sources::HostSource::Counter`]
//!   *is* the seam it writes into, and [`sources::HostSource::Foreign`] is the rejected
//!   alternative, present only so its refusal is exercised.
//! - **A deadline timer.** Both `Vmm` backends drive `defer` off an explicit virtual clock;
//!   a real timer descriptor would make every suite that composes them non-deterministic
//!   (§8.3 forbids it) and would buy nothing today.

pub mod reactor;
pub mod sources;

pub use reactor::{
    CONTROL_TOKEN, MAX_UNPRODUCTIVE_STREAK, Reactor, ReactorFault, ReactorHandle, ReactorStats,
};
pub use sources::{
    HostSource, MAX_ARMED_SOURCES, MAX_REMEMBERED_RETIRED, ReactorError, Registrar, Resolved,
    SourceShape,
};
