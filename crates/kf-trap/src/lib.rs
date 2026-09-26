//! ★★★★★ **v3 `kf-trap` — the vCPU trap path and the doorbell plane** (`THE_ARCHITECTURE_v3.md` §2).
//!
//! Copied from the old tree's `kayfabe-doorbell` (built v3-native at w823: 48 tests, 0/60 flake,
//! an exhaustive interleaving model) — only the modules the trap path needs:
//!
//! | module | job |
//! |---|---|
//! | [`token`] | one `u64` per token: route, state, stamp, opaque host token — every CAS |
//! | [`bitmap`] | the rung bitmap + summary — scanning only, never CAS'd; rotating start |
//! | [`wake`] | the wakeup word — `{work_seq:56, workers_polling:8}`, park/unpark |
//! | [`ring`] | the privileged MPSC ring the register drainer consumes in order |
//! | [`shadow`] | write semantics for privileged registers |
//! | [`timer`] | the time registers refused by name, per FAMILY (`kf_chip::Family`) |
//! | [`trap`] | THE trap: doorbell / userspace-mappable (do nothing) / privileged |
//! | [`fspemem`] | Hopper+/Blackwell: FSP's RM EMEM channel (auto-increment data port + FSP's COT reply), served on the vCPU |
//! | [`mmuinval`] | the MMU invalidate registers: PDB latch + the trigger that arms, publishes, and reads busy until the VA manager clears it (P4) |
//! | [`pramin`] | the PRAMIN window-base register per family (decode + slot plan; P4) |
//! | [`memmap`] | the per-family BAR memory map: backed / trap-write / hole (read exits only for read side effects: PIO auto-increment ports, and — w828 — Hopper+'s read-started memop token registers) |
//! | [`trappolicy`] | which writes trap, where the doorbell lives (BAR0 / Hopper+ BAR1), `may_trap_read` |
//! | [`vmm`] | the device↔hypervisor seam (`VmmOps`: memslots, guest RAM, irq, wakes) |
//! | [`model`] | the exhaustive interleaving check (SC; a falsifier, not a proof of orderings) |
//!
//! ⊘ No `unsafe`, no OS call, no lock, no allocation on the vCPU path. The trap SAYS whether a
//! syscall is owed ([`trap::Action`]); the caller performs it.

pub mod bar1db;
pub mod bitmap;
pub mod cacheop;
pub mod cpuintr;
pub mod fspemem;
pub mod memmap;
pub mod mmuinval;
pub mod model;
pub mod pramin;
pub mod ring;
pub mod shadow;
pub mod timer;
pub mod token;
pub mod trap;
pub mod trappolicy;
pub mod vmm;
pub mod wake;

pub use bitmap::RungBitmap;
pub use mmuinval::{Invalidate, InvalidatePort, InvalidateRegs, InvalidateRequest, PdbAperture, PortWrite};
pub use ring::{PrivRing, Push, RegWrite};
pub use shadow::{Cell, ClearOutcome, Trigger, WriteSemantics};
pub use token::{Claim, Release, Route, State, Token, TokenWord};
pub use trap::{Action, Class, TrapPath};
pub use wake::{Wake, WakeWord};

/// §5.2's bound on the re-act loop. ⊘ Not tuning: it is the inner boundary. A guest process
/// ringing its own channel in a tight loop would otherwise pin a worker forever.
pub const REACT_ROUNDS: u32 = 8;

#[cfg(test)]
mod tests;
