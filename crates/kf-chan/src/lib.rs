//! ★★★★★ **v3 `kf-chan` — channels, with no CPU executor anywhere.**
//!
//! Three kinds, one host verb set (`kf-host::channel`):
//! - **Passthrough** — the guest's own ring and USERD, adopted at birth; its doorbell is a fenced
//!   store inline in the trap.
//! - **Translated** — the guest KERNEL's copy-engine channels, whose pushbuffers name PHYSICAL
//!   operands: copied, rewritten onto the identity window ([`translated`]), and executed on the
//!   REAL engine from our own ring. `MEM_OP` is a split point: walk + publish, then resume.
//! - **Emulated** — executed by the VMM worker through the host RM client (§37).
//!
//! Completions arrive as host events on an fd (`kf-host::event`), never inline.
//!
//! ★ [`dbfast`] (2026-09-30): the doorbell's optional fast TRANSPORT — a KVM ioeventfd per live
//! token, serviced by the register drainer, handled exactly as the trap handles it.

pub mod census;
pub mod completions;
pub mod dbfast;
pub mod host;
pub mod passthrough;
pub mod ring;
pub mod tmode;
pub mod translated;
pub mod tspace_unsafe;
pub mod ttables;
pub mod worker;
