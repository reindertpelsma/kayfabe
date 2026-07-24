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
//!
//! ## ★ The concurrency contract (decision #17)
//!
//! The core WILL be invoked concurrently from multiple vCPUs (different guest
//! processes ringing doorbells / allocating / mapping at once). The contract is
//! **"thread-safe by default, exceptions explicit"** — and it is *enforced*, not
//! aspirational:
//!
//! - **Every core type is `Send + Sync`**, compile-time-asserted at the bottom of
//!   each defining crate (`nvkvm_util::assert_send_sync!`): sneaking in an `Rc`, a
//!   `Cell`, or an un-bounded trait object **fails the build**, on every
//!   `cargo check`. The workspace-wide `#![forbid(unsafe_code)]` closes the other
//!   half: safe Rust is data-race-free by construction, so a safe-code data race
//!   cannot exist in these crates *at all* — only the shape of the caller's
//!   synchronization is left to decide, never memory safety.
//! - **No interior mutability, anywhere.** Core state is plain owned data
//!   (`BTreeMap`/`Vec`/newtypes); there is no `Mutex`, `RefCell`, or atomic in any
//!   logic crate. **All mutation takes `&mut self`** — the borrow checker forbids
//!   concurrent calls, and the *caller* provides exclusivity (a device-global lock,
//!   a per-`Proc` shard, an actor loop — any strategy is sound because the core
//!   presumes none). **All reads take `&self`** and are concurrent-safe: any number
//!   of threads may share `&Gpu` and resolve/route/inspect in parallel, lock-free.
//! - **No thread-unsafe exceptions exist.** The audit for this milestone found
//!   none to document: no core type needs `!Sync`. The single *relaxation* is
//!   `dyn RmBackend` (`Send` but not `Sync` — reachable only through
//!   `Isolate::rm(&mut self)`, so a shared reference to one is unrepresentable;
//!   documented in `nvkvm-isolate`).
//! - **Ports the core *stores* carry the bound; ports passed as arguments don't.**
//!   `Arch`, `Isolate`, `IsolateFactory` live inside `Gpu` and are `Send + Sync`
//!   supertraits. `Vmm` (`Send`), `Present`, `FbRead`, `TraceSink` are only ever
//!   *arguments* (`&mut dyn`), so their synchronization belongs to whoever owns
//!   them — the adapter.
//!
//! **The architectural payoff — per-`Proc` parallelism:** because each [`gpu::Proc`]
//! owns all four of its planes (address/exec/completion/isolate + arena) and the
//! per-proc entry points take `&mut Proc` (not `&mut Gpu`), two vCPUs driving
//! *different* guest processes can mutate their `Proc`s **simultaneously with no
//! shared lock** — disjoint `&mut` borrows out of `Gpu::procs` are safe by the
//! borrow checker, and the state they reach (arena, host VAS, isolate, completion
//! queue) is disjoint by construction (#14's isolation, cashed in as concurrency).
//! Only device-global state — `RmGraph` mutation (`Gpu::apply`), routing-map
//! refresh, the `DeliveryPlane`'s single drain gate — needs coarser exclusivity.
//! The concrete locking *strategy* is the L1 OS layer's decision;
//! `tests/concurrency_stress.rs` proves the realistic one (device-global
//! `RwLock` + split per-`Proc` borrows) over millions of interleaved ops, and is
//! the suite to run under ThreadSanitizer (invocation documented there).

pub mod gpa;
pub mod gpu;
pub mod project;
pub mod rmgraph;

use nvkvm_arch::ids::HClient;

// The concurrency contract, compile-time-asserted (decision #17): every public
// type of the core — including `Gpu` itself, whose `Box<dyn Arch>`/`Box<dyn
// Isolate>` fields are exactly where a missing bound would hide.
nvkvm_util::assert_send_sync!(
    ProcId,
    ChanId,
    Traffic,
    ProcAnchor,
    gpu::Gpu,
    gpu::Proc,
    gpu::Vas,
    gpu::Channel,
    gpu::GpuTarget,
    gpu::ExecPlane,
    gpu::PollState,
    gpu::GpuError,
    gpa::GpaSpace,
    gpa::GpaArena,
    gpa::GpaError,
    rmgraph::RmGraph,
    rmgraph::RmEvent,
    rmgraph::RmGraphError,
    rmgraph::NodeKey,
    rmgraph::AllocFacts,
    rmgraph::Mapping,
    rmgraph::RmNode,
    project::Boundaries,
    project::ProcBoundary,
    project::ChannelFacts,
    project::VasFacts,
    project::ProjectionError,
);

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
