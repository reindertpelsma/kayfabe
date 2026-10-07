//! CPU affinity for the calling thread — the safe face of [`crate::affinity_unsafe`].
//!
//! ⊘ Deliberately a **diagnostic** facility and not a product one. Nothing in the shipped
//! isolate or shim pins anything; the VMM owns vCPU placement. This exists so a stress rung
//! can reproduce the *topology* a guest actually presents — a small, fixed set of cores with
//! more runnable threads than cores on it — because that is where preemption lands **inside**
//! a held lock, and an unpinned run on a wide box tends never to produce it.

pub use crate::affinity_unsafe::{current_cores, pin_current_thread};
