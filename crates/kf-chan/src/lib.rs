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

pub mod ce_implicit;
pub mod census;
pub mod completions;
pub mod dbfast;
pub mod grtables;
pub mod host;
pub mod passthrough;
pub mod ptnsi;
pub mod ring;
pub mod swmethod;
pub mod tmode;
pub mod translated;
pub mod tspace_unsafe;
pub mod ttables;
pub mod user_batch;
pub mod user_consumption;
pub mod user_gate;
pub mod user_lazy;
pub mod user_lifecycle;
pub mod user_loop;
pub mod user_progress;
pub mod user_ring;
pub mod user_scan;
pub mod user_storage;
pub mod user_stream;
pub mod user_turn;
pub mod userd_relay;
pub mod video_generated;
pub mod vtables;
pub mod worker;
