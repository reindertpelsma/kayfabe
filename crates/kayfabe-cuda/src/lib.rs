//! ★★★★★ **THE WALK KERNEL, INSIDE KAYFABE** — `SINGLE_STORE_PLAN.md` increment 4.
//!
//! `cuda/walk` is a standalone proving ground: a GPU, a buffer, a kernel and 58 assertions,
//! with nothing from the kayfabe runtime in it. This crate is the other half — the same
//! kernel, as **PTX committed to this tree**, driven by a `dlopen`ed CUDA **driver API** from
//! inside the scratchpad isolate.
//!
//! # What this crate deliberately is NOT
//!
//! ⊘ **Not the walker wired into refresh.** That is increment 6, and `SINGLE_STORE_PLAN.md`
//! §w724c records why it is a *gate* rather than a step. This crate proves only that the
//! kernel **can run in-process and return a correct answer**.
//!
//! ⊘ **Not a CUDA dependency of the workspace.** Nothing here links `libcuda` at build time;
//! the library is found at run time and every failure to find it is a **named refusal**
//! ([`driver::CudaError::NoLibrary`], carrying `dlerror()` verbatim). So a machine with no
//! NVIDIA driver builds and tests this crate exactly like one that has it.
//!
//! # ⊘⊘⊘ THE PREMISE, MEASURED RATHER THAN ASSUMED
//!
//! `[measured 2026-09-14, locally, no GPU]` a **musl static-pie** Rust binary's `dlopen`
//! returns `NULL` with `dlerror()` = `"Dynamic loading not supported"` — for `libcuda.so.1`,
//! `libc.so.6` and `libm.so.6` **alike**. ⇒ the blocker `THE_CONSTRAINTS.md` §w724d names is
//! real, and it is *more* general than it says: it is not that `libcuda` is the wrong kind of
//! shared object, it is that **there is no dynamic linker in that process at all**. Every
//! isolate in this tree is that binary. ⇒ the scratchpad isolate has to be a **different
//! build**, which is what §w724d prescribes.

pub mod abi;
pub mod driver_unsafe;
pub mod selftest;
pub mod synth;
pub mod walk;

pub use driver_unsafe::{Cuda, CudaError};
pub use walk::{Report, ReportError, WalkCfg, WalkKernel, WALK_PTX};
