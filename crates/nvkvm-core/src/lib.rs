//! # nvkvm-core — the Mode-2 logic core's composition root
//!
//! A **pure state machine over guest-supplied bytes**: no QEMU types, no syscalls, no
//! OS knowledge, no real-time reads, no NVIDIA struct layouts (Axis A is quarantined
//! to `nvkvm-abi`; Axis B behavior is behind `nvkvm-arch::Arch`). "Is the core
//! OS/VMM-free?" is the invariant the whole rewrite is judged on
//! (`mode2_rust_rewrite_architecture.md` §4).
//!
//! ## Structure (arch doc §4.3)
//!
//! - [`rmgraph`] — ★ THE SOURCE OF TRUTH: the RM resource graph (clients → devices →
//!   VASpaces/TSGs/CtxShares/Channels + DUP edges), built from abstract [`rmgraph::RmEvent`]s.
//!   Every edge is a **declared protocol fact**, so the graph — and everything derived
//!   from it — is order-independent (decision #14, §4.3.1a).
//! - [`project`] — pure projections of the graph: `by_pdb`, `by_vchid`, and the `Proc`
//!   grouping. Deterministic functions of the graph; never accreted from event order.
//! - [`gpu`] — the runtime ownership spine: [`gpu::Gpu`] holds the graph + derived
//!   [`gpu::Proc`]s; each `Proc` owns its four planes (address = per-[`gpu::Vas`],
//!   exec = per-[`gpu::Channel`]/[`gpu::ExecPlane`], completion, isolate + GPA arena).
//! - [`gpa`] — the guest-physical window and its **per-process arenas** (§4.3.3):
//!   disjoint by construction, so the `ALREADY-MAPPED` collision class cannot occur.
//!
//! ## The anti-C-duplication property
//!
//! This crate depends on the `Arch`, `Isolate`, `RmBackend` **traits** — never on a
//! concrete architecture, driver version, hypervisor, or OS. Bringing up a real
//! architecture is `impl Arch for <Gen>` in an adapter crate with **zero edits here**;
//! the mock-driven test suite is the standing proof (it runs this exact code against a
//! fake architecture).

pub mod gpa;
pub mod gpu;
pub mod project;
pub mod rmgraph;

use nvkvm_arch::ids::HClient;

/// A derived guest-process identity. NOT a hardware concept ("there is no GPU
/// process" — decision #14): purely the label of one dup-connected component of the
/// RM graph, used to key the *grouping* planes (isolate, GPA arena, completion,
/// lifecycle). Address ops key on [`nvkvm_arch::ids::Pdb`] (per-`Vas`), exec ops on
/// [`nvkvm_arch::ids::VChid`] (per-`Channel`) — never on `ProcId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProcId(pub u32);

/// A per-`Proc` channel slot id (dense, core-assigned; the guest-facing identity of
/// the channel remains its [`nvkvm_arch::ids::VChid`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChanId(pub u32);

/// Kernel-vs-user traffic delineation as a **type**, not an exclusion list
/// (arch doc §4.3.1): forging a completion for [`Traffic::Proc`] traffic is
/// unrepresentable in the forge path, which is typed to [`Traffic::System`]
/// (lesson L5 / the #12 finishPayload rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Traffic {
    /// Guest-kernel / scrubber / CeUtils traffic — routed to the system `Proc`.
    System,
    /// A guest userspace process's traffic.
    Proc(ProcId),
}

/// The anchor of a `Proc`: the smallest client handle in its dup-connected
/// component. A deterministic, order-independent label used to keep `Proc`
/// state stable across graph re-derivations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProcAnchor(pub HClient);
