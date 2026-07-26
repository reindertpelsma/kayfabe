//! # kayfabe-core — the Mode-2 logic core's composition root
//!
//! A **pure state machine over guest-supplied bytes**: no QEMU types, no syscalls, no
//! OS knowledge, no real-time reads, no NVIDIA struct layouts (Axis A is quarantined
//! to `kayfabe-abi`; Axis B behavior is behind `kayfabe-arch::Arch`). "Is the core
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
//! - [`reactor`] — ★ the **completion-source reactor port** (`l1_concurrency.md` §6,
//!   decision #37): an opaque-handle source registry plus the routing knowledge
//!   *"source S signalled → which proc → what to do"*. PURE — no host descriptors, no
//!   syscalls, ever (§6.2, CI-gated); the OS half is L1's adapter.
//!
//! ## The anti-C-duplication property
//!
//! This crate depends on the `Arch`, `Isolate`, `RmBackend` **traits** — never on a
//! concrete architecture, driver version, hypervisor, or OS. Bringing up a real
//! architecture is `impl Arch for <Gen>` in an adapter crate with **zero edits here**;
//! the mock-driven test suite is the standing proof (it runs this exact code against a
//! fake architecture).
//!
//! ## ★★ MISS = FAULT, and the ONE declared exception: **DEFER** — in TWO flavours
//!
//! `mode2_address_table.md`'s founding rule is *MISS = FAULT*: a lookup that misses is a
//! loud, typed refusal, never a fallback walk, never a guess, never a default. It stays.
//!
//! But the codebase has always had a second answer, and having it is **correct**: a
//! `MapMemoryDma` that arrives before its VASpace's `SET_PAGE_DIRECTORY` is *deferred*,
//! not faulted, because the guest legitimately maps before it binds a page directory
//! (`Gpu::sync_rpc_mappings`). Which answer a site gives was, until this audit, decided
//! site by site by whoever wrote it — and undocumented judgement is what drifts. So the
//! split is **declared at every miss site**.
//!
//! ★★★ **The criterion was corrected in `l1_concurrency.md` §12.38.** §12.30 asked *"could
//! this fact still arrive?"*. The right question is *"could this fact still arrive **in a
//! legal protocol trace**?"* — and asking the first one is how a cross-process isolation
//! break was classified as a deferral. **Order-independence holds where the NVIDIA
//! protocol ALLOWS an ordering, not wherever an ordering is merely expressible. If RM
//! would error on "use before exist", so must we.** There are therefore **three**
//! categories, and the first pass collapsed two of them:
//!
//! | # | the fact is… | answer | why |
//! |---|---|---|---|
//! | **1** | **DEFER (protocol)** — the guest may *legally* send this before that | **DEFER** — skip, park, re-plan, or wait; mutate nothing, record nothing | the protocol is order-tolerant here (decision #4); turning a legal transient absence into a refusal makes the SAME facts in a different order produce a different end state |
//! | **2** | ★ **DEFER (observation)** — the protocol DOES order it, but **we may not have observed the earlier fact** | **DEFER**, and the label is load-bearing | measured, not assumed: only **25 of 82** dups reach GSP (`docs/reference/rm_semantics_measured.md` §3), so a dup's *source object* may genuinely be one RM saw and we did not. Faulting here **hangs a legal guest** |
//! | **3** | **FAULT** — the protocol forbids the ordering **and** we would have observed the earlier fact | **FAULT** (MISS=FAULT proper) — typed, loud, mutation-free | RM refuses it, so no legal trace is lost; deferring it is the vulnerability shape (a hostile handle staying silently inert instead of refused) |
//!
//! The dividing line between **2** and **3** in practice is *which fact is missing*, not
//! which site is asking: a **client root** (`NV01_ROOT`) is always on the GSP wire — it is
//! literally where `AllocFacts::client_kind` comes from — so its absence is category 3
//! (`RmGraphError::UndeclaredClient`). An **object-level** fact (an object's parent, a
//! VASpace alloc, a dup's source object) may not be, so its absence stays category 2 even
//! where RM would have ordered it.
//!
//! Four things about this taxonomy are load-bearing:
//!
//! 1. **The category is a property of the SITE, not of the absence.** The *same* missing
//!    fact is a DEFER in derivation and a FAULT at use. A channel with no declared VAS
//!    materializes no `Vas` (`Gpu::sync_proc_to_boundary` — deferred, it may yet be
//!    declared) and takes `FwdFault::NoVas` the instant a doorbell rings it
//!    (`kayfabe_fwd::gate_working_set_in` — at ring time there is no "later"). Both are
//!    right, and neither is a weakening of the other: **deferral in derivation is what
//!    makes the fault at use exact.**
//! 2. **A DEFER must be recoverable by a fact arriving.** Every deferring site is
//!    re-evaluated from scratch on the next `Gpu::apply` (`project` is pure and
//!    `sync_rpc_mappings` is idempotent), so "deferred" means *"the answer changes when
//!    the fact lands"*, never *"silently dropped"*. A deferral with no re-evaluation path
//!    is a hang, and is a bug — which is why `tests/miss_taxonomy.rs` proves resolution
//!    for every deferring site rather than merely proving it does not crash.
//! 3. **Getting the category wrong is asymmetric, in opposite directions.** A FAULT that
//!    should be a DEFER is a hung or spuriously-refused guest (§12.9's own lesson: the
//!    first symptom was a hang, not an assertion). A DEFER that should be a FAULT is a
//!    *security* question — §12.38's squat is the worked example. That is why the
//!    categories are written down at the site rather than inferred from the return type:
//!    `Option<T>` cannot tell them apart, and several resolvers here legitimately return
//!    one `None` for both.
//! 4. **Category 2 must be said out loud.** "This defers because the protocol permits it"
//!    and "this defers because we might not have seen the earlier fact" look identical in
//!    the code and are completely different claims — and only the second one survives a
//!    change to what we observe. §12.30's inventory lacked the distinction, and that is
//!    precisely why it mis-filed the one site that mattered.
//!
//! ## ★ The concurrency contract (decision #17)
//!
//! The core WILL be invoked concurrently from multiple vCPUs (different guest
//! processes ringing doorbells / allocating / mapping at once). The contract is
//! **"thread-safe by default, exceptions explicit"** — and it is *enforced*, not
//! aspirational:
//!
//! - **Every core type is `Send + Sync`**, compile-time-asserted at the bottom of
//!   each defining crate (`kayfabe_util::assert_send_sync!`): sneaking in an `Rc`, a
//!   `Cell`, or an un-bounded trait object **fails the build**, on every
//!   `cargo check`. The workspace-wide `#![forbid(unsafe_code)]` closes the other
//!   half: safe Rust is data-race-free by construction, so a safe-code data race
//!   cannot exist in these crates *at all* — only the shape of the caller's
//!   synchronization is left to decide, never memory safety. (The lint stays
//!   workspace-wide forever; the companion **`*_unsafe.rs` naming rule** — CI's
//!   *Unsafe-surface gate* — is what will make the L1 adapter's one audited
//!   relaxation enumerable with `ls`. Stated in full in `kayfabe-rt`'s crate docs,
//!   because that is the neighbourhood it will land in.)
//! - **No interior mutability, anywhere.** Core state is plain owned data
//!   (`BTreeMap`/`Vec`/newtypes); there is no `Mutex`, `RefCell`, or atomic in any
//!   logic crate. **All mutation takes `&mut self`** — the borrow checker forbids
//!   concurrent calls, and the *caller* provides exclusivity (a device-global lock,
//!   a per-`Proc` shard, an actor loop — any strategy is sound because the core
//!   presumes none). **All reads take `&self`** and are concurrent-safe: any number
//!   of threads may share `&Gpu` and resolve/route/inspect in parallel, lock-free.
//! - **No thread-hostile exceptions exist.** The audit for this milestone found
//!   none to document: no core type needs `!Sync`. The single *relaxation* is
//!   `dyn RmBackend` (`Send` but not `Sync` — reachable only through
//!   `Isolate::rm(&mut self)`, so a shared reference to one is unrepresentable;
//!   documented in `kayfabe-isolate`).
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
pub mod reactor;
pub mod rmgraph;

use kayfabe_arch::ids::HClient;

// The concurrency contract, compile-time-asserted (decision #17): every public
// type of the core — including `Gpu` itself, whose `Box<dyn Arch>`/`Box<dyn
// Isolate>` fields are exactly where a missing bound would hide.
kayfabe_util::assert_send_sync!(
    ProcId,
    ChanId,
    Traffic,
    ProcAnchor,
    gpu::Gpu,
    gpu::Spine,
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
    rmgraph::ResourceKey,
    rmgraph::AllocFacts,
    rmgraph::Mapping,
    rmgraph::RmNode,
    project::Boundaries,
    project::ProcBoundary,
    project::ChannelFacts,
    project::VasFacts,
    project::ProjectionError,
    reactor::CompletionSource,
    reactor::SourceKind,
    reactor::Dispatch,
    reactor::SourceFault,
    reactor::SourceRegistry,
    reactor::WakeRequest,
);

/// A derived guest-process identity. NOT a hardware concept ("there is no GPU
/// process" — decision #14): purely the label of one dup-connected component of
/// **declared user clients** in the RM graph (★ §12.27 — a dup into a *kernel* client
/// is a reference, not a merge, and every kernel client belongs to the reserved system
/// component), used to key the *grouping* planes (isolate, GPA arena, completion,
/// lifecycle). Address ops key on [`kayfabe_arch::ids::Pdb`] (per-`Vas`), exec ops on
/// [`kayfabe_arch::ids::VChid`] (per-`Channel`) — never on `ProcId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProcId(pub u32);

/// A per-`Proc` channel slot id (dense, core-assigned; the guest-facing identity of
/// the channel remains its [`kayfabe_arch::ids::VChid`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChanId(pub u32);

/// Kernel-vs-user traffic delineation as a **type**, not an exclusion list
/// (arch doc §4.3.1): forging a completion for [`Traffic::Proc`] traffic is
/// unrepresentable in the forge path, which is typed to [`Traffic::System`]
/// (lesson L5 / the #12 finishPayload rule).
///
/// **Status: deliberate seam, vocabulary-only today** (consolidation review §5.2
/// item 4, decision #33). No current signature consumes it — the forge path's
/// enforcement is structural (`kayfabe-fwd::signal_golden_capture` writes to
/// `Gpu::system` directly, so a user-proc forge is unrepresentable without the
/// type). It is kept as the named rule the design ledger cites (threat model,
/// regression-matrix rows 7/11) and as the L1 seam: when the real GSP queue's
/// traffic classification ports, L1 decides whether to thread `Traffic` through
/// the observe/forge signatures or fold it — deferred there, not guessed here.
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
///
/// ★ §12.27: `HClient(0)` is RESERVED as the **system** component's anchor
/// (`kayfabe_core::project::SYSTEM_ANCHOR`) and is refused as guest input by
/// `RmGraph::apply`, so a user component can never carry it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProcAnchor(pub HClient);
