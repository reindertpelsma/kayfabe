//! # kayfabe-rmrpc — the GSP → core bridge
//!
//! `docs/design/gsp_core_bridge.md`. One decoded GSP RPC ([`kayfabe_gsp::RpcCommand`])
//! becomes one declared protocol fact ([`kayfabe_core::rmgraph::RmEvent`]), a
//! known-and-inert nothing, or a **named refusal**. Nothing else.
//!
//! Before this crate existed, `RpcCommand` had zero references outside the crate that
//! defines it and `Gpu::apply`'s input was synthesised by tests. This is the seam that
//! joins them, and it is the only crate in the tree permitted to name both types (§1.2 —
//! CI greps for it).
//!
//! ## ★★ It is STATELESS, and that is the design
//!
//! [`translate`] is a free function of one message. There is **no handle table, no
//! seen-set, no dedup cache, no mapping from a guest handle to anything of ours**, and
//! there must never be one:
//!
//! - `hClient`/`hObject` values are recycled **by RM's own design** — caller-supplied
//!   handles are honoured verbatim, and RM's generator has no free list and no quarantine
//!   (the citations are on `kayfabe_core::rmgraph::ResourceKey` and `::ClientId`). Any
//!   memory of a handle value here would eventually refuse, dedup or mis-attribute a
//!   *legal* recycle, which hangs a conforming guest.
//! - identity is minted **inside** `RmGraph::apply`, from the live set
//!   (`RmGraph::next_incarnation` / `::next_client_incarnation`). This crate cannot mint
//!   one because it cannot see the live set, and it must never acquire the ability.
//! - a *replayed* message therefore maps to the *identical* event, which is exactly what
//!   makes the graph's idempotent-retry tolerance (`RmGraphError::ConflictingAlloc`'s
//!   doc) reachable. **A bridge with a dedup cache would break that** (§4.3).
//!
//! ★ If a future stage wants a per-handle cache here for performance, it is re-opening the
//! §12.41/§12.42 identity bug class and must be refused unless the cache is keyed by
//! something the graph itself minted.
//!
//! ## What this crate's one real duty is: **namespace attribution**
//!
//! > The namespace is **always** the RPC body's own `hClient`. Never a params field.
//! > Never inferred.
//!
//! Enforced structurally: the header's client is read once, at the top of each arm, and
//! the params decoders are not given it — so a params-derived client cannot be substituted
//! without a visible signature change. The C's counter-example is `GPU_PROMOTE_CTX`, which
//! reads `hChanClient` from `params+12` and never looks at the envelope's own field
//! (`nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:2283`). Sometimes right, and
//! unprincipled: a params field naming a *different* client is a cross-namespace
//! reference, which is a different fact and needs its own event, not a silent
//! substitution.
//!
//! The one place the fact is declared twice is the client root, where the header's
//! `hClient` and `NV0000_ALLOC_PARAMETERS.hClient` must agree — and a disagreement is
//! [`BridgeRefusal::ClientHandleDisagrees`], never a pick.
//!
//! ## What it deliberately does NOT do
//!
//! - **Read guest memory.** `CommandPolicy::respond` takes no guest-memory port, and
//!   `gl11_region_arguments.md` §2.2a makes that absence the reason the GSP command queue
//!   is not a lock-path region. It is easy to honour here: `rpc_gsp_rm_alloc_v03_00`'s
//!   `params[]` is an **inline flexible array**, not a guest address — the guest already
//!   copied the params into the queue.
//! - **Look anything up.** No handle is resolved, no table is consulted, and it is never
//!   asked whether a referenced object exists. So `MISS = FAULT, never reverse-resolve`
//!   has nothing to bind to here: it governs the address table, whose owner is
//!   `kayfabe_mmu::AddressTable`, and the three-way MISS/DEFER/FAULT taxonomy belongs to
//!   `RmGraph::apply`. **The bridge's duty is not to pre-empt any of them** — it emits the
//!   declared fact and lets `apply` decide (§3.4).
//! - **Touch host state.** No worker, no host RM, no isolate.
//!
//! ## Refusals are loud, and they still get answered
//!
//! *"An unrecognised or malformed RPC is a LOUD REFUSAL, never a best-effort guess or a
//! silent drop."* This is an **authorised deviation from the C**, so the C's behaviour is
//! named rather than omitted: it answers an unknown RPC **affirmatively** — `memcpy(resp,
//! cmd, 4096)` (`C:2737`) then `nvkvm_m3_post_status(…, 0 /* NV_OK */)` (`C:3326`) — with
//! no allowlist, no counter and, outside `-trace`, no log line at all.
//!
//! But a refusal is **not a drop**: the guest blocks in `_issueRpcAndWait`, which calls
//! `rpcRecvPoll(pGpu, pRpc, expectedFunc, expectedSequence)`
//! (`ogkm-610: src/nvidia/src/kernel/vgpu/rpc.c:1990`, `ogkm-580: :1972`), and on a GSP
//! client that is `_kgspRpcRecvPoll` — a `for (;;)` that drains the message queue until a
//! message whose `(function, sequence)` both match arrives, or `gpuCheckTimeout` fires
//! (`ogkm-610: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:2962-2998`, the pair-match at
//! `:1825-1826`; `ogkm-580: kernel_gsp.c:2391-2429`, the pair-match at `:1776-1777` —
//! structurally the same loop in both trees). So an
//! unanswered command hangs it for the whole RPC timeout. The caller therefore still posts
//! a reply, carrying [`BridgeRefusal::rpc_result`]. Refusals surface in three places and
//! all three are mandatory: the reply, the trace ([`Faulted`], so they are *countable* —
//! the invariant is a bound like "zero refusals over a clean boot", never an absence), and
//! the `Result` itself.
//!
//! ## The two halves, and which is which
//!
//! - [`translate`] — B1. A **free function of one message**: bytes in, one declared fact
//!   or a named refusal out. Stateless, pure, and the subject of everything above.
//! - [`Reassembler`] — B6 ([`reasm`]). The one stateful value in the crate: it joins a
//!   run of `CONTINUATION_RECORD` fragments back into the message the guest meant, under
//!   two mandatory bounds. It decides **nothing** — it concatenates, bounds, and hands
//!   the whole to [`translate`], which applies every rule exactly once. And it holds no
//!   handle: a byte buffer plus four numbers, keyed by nothing the guest supplies,
//!   dropped the instant the message completes or refuses.
//! - [`GraphPolicy`] — B2 ([`policy`]). The `kayfabe_gsp::CommandPolicy` the boot FSM
//!   calls for every command a guest posts: reassemble → translate → `Gpu::apply` → a
//!   reply. It is the only thing here that holds a `&mut Gpu`, and it decodes nothing.
//!
//! The split is what keeps the rule above true while the stage that *must* touch state
//! lands: [`translate`] still has nowhere to put a handle cache, and the state B6 does add
//! is the one shape the rule permits — bounded, handle-free, and singular.
//!
//! ## Scope of this stage (B1 + B2 + B3 + B4 + B5 + B6 — the build order is complete)
//!
//! `GSP_RM_ALLOC`, `FREE`, the **one modelled control**,
//! `GSP_RM_CONTROL`/`SET_PAGE_DIRECTORY` — which is where a VASpace acquires the `Pdb` the
//! data plane routes on — `DUP_OBJECT` at B5, and `CONTINUATION_RECORD` at B6. **That
//! completes the object model's four verbs** — create, reference, destroy, and the one
//! control that carries a data-plane identity — **and the transport fragment that carries
//! any of them when it does not fit.** Everything else is a named refusal, including the
//! classes with no entry in the class table ([`BridgeRefusal::UnmappedAllocClass`]).
//!
//! ★ **The "known, mapped, arm not built" state is now empty**, and its variant is gone
//! rather than kept as a placeholder: `CONTINUATION_RECORD` was the last id in it. The two
//! states that remain — "known and inert" ([`Translation::Inert`]) and "not known at all"
//! ([`BridgeRefusal::UnknownFunction`]) — are still deliberately distinct, because
//! collapsing them is how the C ended up answering everything `NV_OK`.
//!
//! ## ★★ Does a `CONTINUATION_RECORD` earn its own reply? Settled at B6, and it is
//! **two** questions
//!
//! `gsp_core_bridge.md` §2.6/§7 item 4 left this `[unverified]` and required B6 to settle
//! it before shipping. `_issueRpcLarge` (`ogkm-610: rpc.c:2058-2244`,
//! `ogkm-580: :2038-2223` — the same function, and byte-identical apart from 610's
//! `rpcGetVgpuMessageHeader(pRpc)` replacing 580's `vgpu_rpc_message_header_v` macro)
//! answers it in two halves:
//!
//! - **Sending**, the guest does *not* await a reply per fragment. It posts every
//!   fragment (`rpcSendMessage` per fragment, `pRpc->sequence` incrementing —
//!   `NV_ASSERT(lastSequence == firstSequence + recordCount)`) and then waits **once** at
//!   `(expectedFunc, firstSequence)`.
//! - **Receiving**, it does — but only when the head was issued **bidirectionally**. With
//!   `bBidirectional && recordCount > 0` it then polls
//!   `rpcRecvPoll(…, NV_VGPU_MSG_FUNCTION_CONTINUATION_RECORD, waitSequence)` with
//!   `waitSequence` incrementing, until the reply bytes fill the request's own
//!   `bufSize` (`ogkm-610: rpc.c:2186-2226`, `ogkm-580: :2164-2205`).
//!
//! ⇒ For the **one** fragmenting function this bridge translates — fn 76, issued
//! `_issueRpcAndWaitLarge(…, NV_TRUE)` (`ogkm-610: rpc.c:10856`, `ogkm-580: :11051`) — a reply
//! per fragment is **required**, and withholding one hangs the guest for the whole RPC
//! timeout. `GspFsm::answer` already posts one, echoing each fragment's own
//! `(function, sequence)` and its own length, which is exactly the pair and the arithmetic
//! that loop consumes. **No transport change was needed and none was made.**
//!
//! ★ And the status lands in the right place *by construction*: the driver reads
//! `rpc_result` from `pVgpuRpcHeader` **after** that loop — i.e. from the **last** fragment
//! it received (`ogkm-610: rpc.c:2230-2241`, `ogkm-580: :2209-2220` — same test, same
//! collapse). The last fragment is precisely the one on which
//! [`Reassembler`] completes and [`translate`] runs, so the head and the intermediate
//! fragments ack `NV_OK` ([`Translation::Held`]) and the real outcome rides the final
//! reply.
//!
//! ⚠ **The named gap, because the answer is not uniform.** `SET_REGISTRY` fragments
//! through `_issueRpcAsyncLarge` (`ogkm-610: rpc.c:10533`, `ogkm-580: :10728`), which is
//! `bWait = NV_FALSE`: that guest awaits **no** reply, and `RpcFunction::SetRegistry`'s
//! own `Disposition::NoReply` says so. Its *continuations* nevertheless take
//! `RpcFunction::ContinuationRecord`'s disposition, which is `Reply` — so a registry table
//! over 4064 bytes would draw spurious status posts. It is not fixable from this crate:
//! `Disposition` is computed in `kayfabe_gsp::GspFsm::answer` from the arriving function
//! alone, `CommandPolicy::respond` returns `Option<Reply>` with no "post nothing" value,
//! and making a fragment inherit its head's disposition would put a second copy of the
//! reassembly state inside the FSM. **Recorded as a `kayfabe-gsp` question, not silently
//! absorbed.**
//!
//! ★ **B3 is the class table** ([`kayfabe_abi::versions::DriverAbiTable::alloc_params`]):
//! client root, Device, VASpace, TSG, CtxShare, channel, and the two engine objects — the
//! classes a CUDA process's subgraph is made of. Three things it deliberately does *not*
//! recover, each because `RmEvent`/`AllocFacts` has nowhere to put it:
//!
//! - a channel's **`engineType`**, the only wire fact separating a GR channel from a CE
//!   channel (they share one `hClass`). The engine reaches the core through the
//!   engine-object refinement in `kayfabe_core::project` instead.
//! - a memory object's **`mem_phys`**. `gsp_core_bridge.md` §6 lists it under B3, and it
//!   is unbuildable *twice over*: `NV_MEMORY_ALLOCATION_PARAMS.offset`/`address` are
//!   `[OUT]` in the guest→GSP direction (RM picks the address and returns it), and the
//!   only consumer of `AllocFacts::mem_phys` is `Gpu::sync_rpc_mappings`, driven by
//!   `RmEvent::MapMemoryDma` — which §2.7 proves has **no producer on this wire at all**.
//!   A decoder for it would be a field nothing reads, derived from a value the guest has
//!   not yet been told.
//! - a **TSG's `engineType`** and a **CtxShare's `subctxId`**, for the same
//!   nowhere-to-put-it reason.
//!
//! ★ Two whole RPCs are **not** on the roadmap because they never reach the wire:
//! `MAP_MEMORY_DMA`/`UNMAP_MEMORY_DMA` are HAL stubs on every GSP-client part, so
//! `RmEvent::MapMemoryDma` has **no producer here** and never will (§2.7, three
//! independent oracles). The address table's populate sources are `GPU_PROMOTE_CTX` and
//! the copy-engine page-table-write capture, and both belong to `kayfabe-fwd`.

pub mod fault;
mod policy;
mod reasm;

pub use fault::{FaultEmitRefusal, rc_triggered_for};
pub use policy::{GraphPolicy, RefusalCensus};
pub use reasm::{MAX_CONTINUATIONS, MAX_REASSEMBLED_BODY, ReasmLimits, Reassembled, Reassembler};

use kayfabe_abi::capability::{AllocPermit, ControlPermit, Denial, PassthroughRule};
use kayfabe_abi::versions::{AllocParams, ControlParams, DriverAbiTable};
use kayfabe_abi::wire::AbiError;
use kayfabe_abi::{
    ClientKindRuleUnknown, GuestOs, NV_ERR_NOT_SUPPORTED, rpc_params_are_serialized,
};
use kayfabe_arch::ids::{ClassId, ControlCmd, HClient, HObject, Pdb};
use kayfabe_core::gpu::GpuError;
use kayfabe_core::rmgraph::{AllocFacts, NodeKey, RESERVED_CLIENT, RmEvent};
use kayfabe_gsp::{RpcCommand, RpcFunction};
use kayfabe_trace::{FaultTag, Faulted};

/// What one RPC means to the object model.
///
/// Deliberately three-valued rather than `Option<RmEvent>`: "this RPC carries no
/// object-model content" is a *conclusion* about a known function, and it must not be
/// spelled the same way as "we could not translate it".
///
/// ★ **Not `Copy`.** [`Self::CtxPromotion`] carries a variable number of declared ranges,
/// so it owns a `Vec`. The alternative — a fixed 16-slot array — would have made every
/// `Translation` 600-plus bytes to move on every RPC, and would have duplicated NVIDIA's
/// array bound into a third place. The bound belongs to the ABI layer (the wire) and to
/// `kayfabe_core::promote::MAX_PROMOTED_RANGES` (the core), and those two are checked
/// against each other rather than silently agreeing.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub enum Translation {
    /// One declared protocol fact, to be handed to `Gpu::apply`.
    ///
    /// One RPC produces at most one event today, and the type says so. `GSP_RM_ALLOC`,
    /// `FREE`, `DUP_OBJECT` and the one modelled control are each exactly one fact.
    Event(RmEvent),
    /// **Known and inert.** The RPC is one this port recognises and which carries nothing
    /// the object model can express: guest system description (`SET_GUEST_SYSTEM_INFO`,
    /// `GSP_SET_SYSTEM_INFO`, `SET_REGISTRY`), a query whose *reply* is the device data
    /// model's job (`GET_GSP_STATIC_INFO`), or a transport/lifecycle event the FSM already
    /// owns (`UNLOADING_GUEST_DRIVER` — and note that one is not a graph teardown and not
    /// even a reliable unload signal: `[measured]` `rmmod` emits no fn-47. RM's object
    /// teardown is the `FREE` stream).
    Inert,
    /// **B6 — a fragment of a larger message, consumed into reassembly.** There is no
    /// object-model content *yet*, which is a third thing and not a shade of
    /// [`Self::Inert`]: an inert RPC is a complete message this port has concluded carries
    /// nothing, while a held fragment is an incomplete message whose meaning is still
    /// arriving. Collapsing them would make "the guest sent a large control" and "the
    /// guest sent its registry table" the same observation.
    ///
    /// ★★ **[`translate`] never returns this, and [`GraphPolicy`] does.** The asymmetry
    /// is the crate's whole shape restated: `translate` is a free function of one message
    /// and cannot know that another message preceded this one, so from its view a
    /// continuation record is [`BridgeRefusal::ContinuationWithoutHead`]. Only
    /// [`GraphPolicy`], which owns the one bounded piece of state in the crate, can
    /// produce a `Held`. `translate_never_holds` pins that so the state cannot migrate
    /// into the free function unnoticed.
    Held,
    /// ★★ **An ADDRESS-plane fact**, not an object-model one — `GPU_PROMOTE_CTX`.
    ///
    /// A fourth answer rather than a fourth shade of [`Self::Event`], because the two
    /// go to different places and a caller must not be able to confuse them.
    /// `RmEvent` is applied to the [`kayfabe_core::rmgraph::RmGraph`] and reshapes the
    /// object model; a promotion declares VA → physical bindings and goes to the address
    /// table of ONE `Vas`, touching the graph not at all. Folding it into `Event` would
    /// mean inventing an `RmEvent` variant for a message that declares no resource, no
    /// edge and no handle — and `Gpu::apply` would then be the site that decides which
    /// plane a fact belongs to, which is exactly the collapse this crate exists to
    /// prevent.
    ///
    /// It is also *not* [`Self::Inert`]: inert means "recognised, and carries nothing".
    /// This carries the only address facts a GSP client's RPC stream ever declares.
    CtxPromotion(kayfabe_core::promote::CtxPromotion),
}

/// Every way [`translate`] can refuse, by name.
///
/// Each variant carries the numbers a reader needs. There is no catch-all: an opaque
/// variant would force `is_err()` assertions, which `testing_doctrine.md` §2 forbids.
///
/// ★ **The enum only carries what this stage can produce**, and B6 moves that line in
/// both directions: the five continuation refusals arrive because [`Reassembler`] can now
/// construct every one of them, and `NotYetTranslated` **leaves** because nothing can
/// construct it any more. A variant nothing can construct is a variant no test can bite.
/// [`Self::Graph`] was in that position at B1 and left the enum out for exactly that
/// reason; B2's [`GraphPolicy`] applies, so it exists now.
///
/// ★ Two names differ from `gsp_core_bridge.md` §4.1's sketch and each difference is a
/// finding rather than a preference: [`Self::ContinuationOverflow`] carries `declared`
/// (the head's own number) rather than an accumulated `total`, because it fires **at the
/// head** and there is no accumulation yet; and [`Self::ContinuationOverrun`] and
/// [`Self::ContinuationCountExceeded`] are not in the sketch at all — the first because
/// the arithmetic needs an answer for "the fragments carried more than the head declared"
/// that is not a clamp, the second because §2.6's "a maximum continuation count" is a
/// bound the size bound provably does not imply (a zero-length fragment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeRefusal {
    /// An id the function table does not name at all. The **third state**: not inert, not
    /// staged, simply unrecognised.
    UnknownFunction {
        /// The raw wire id.
        code: u32,
    },
    /// ★ A `CONTINUATION_RECORD` with **no head in flight** (§2.6 bound 3).
    ///
    /// A continuation carries no function of its own, so it can never *become* a head:
    /// there is nothing in it to say what the reassembled message would mean. From
    /// [`translate`]'s stateless view there is never a head, which is why this is also
    /// what a bare fn-71 refuses with when the free function is called directly — and
    /// [`Reassembler`] is the only thing in the tree that can answer otherwise.
    ///
    /// ★★ This variant **replaced `NotYetTranslated`** at B6, and the replacement is the
    /// point rather than a rename: `CONTINUATION_RECORD` was the last id in the
    /// "known, mapped, arm not built" state, so with B6 built nothing can construct that
    /// variant and an unconstructable variant is one no test can bite. The staging state
    /// is now empty and the enum says so.
    ContinuationWithoutHead {
        /// The raw wire id.
        code: u32,
    },
    /// A **new head arrived while one was in flight** (§2.6 bound 4).
    ///
    /// `[inferred]` `_issueRpcLarge` writes every fragment of one message before it
    /// returns, under the GPU lock (`ogkm-610: rpc.c:2074-2145`,
    /// `ogkm-580: :2053-2124`), so a second message
    /// beginning mid-run is not a legal trace — category 3, refused rather than
    /// reconciled.
    ///
    /// Carries the **interrupting** message's id, not the abandoned head's: the head is
    /// gone by construction (every refusal drops it) and the actionable number is the one
    /// that broke the run.
    ContinuationInterleaved {
        /// The raw wire id of the message that interrupted.
        code: u32,
    },
    /// A head declared a total larger than the bridge will ever hold
    /// ([`ReasmLimits::max_body`]).
    ///
    /// Fires **at the head**, before a byte is reserved — `declared` is a guest-supplied
    /// `u32` plus the fixed header, so testing it after the allocation would be a
    /// four-gigabyte allocation on demand.
    ContinuationOverflow {
        /// The total the head's own body declared.
        declared: usize,
        /// The bound.
        max: usize,
    },
    /// More continuation records than [`ReasmLimits::max_continuations`].
    ///
    /// ★ Not implied by [`Self::ContinuationOverflow`]: a **zero-length** continuation
    /// makes no progress towards the size bound, so without a count bound a guest holds a
    /// head open for an unbounded number of messages. Bounded memory is not bounded work.
    ContinuationCountExceeded {
        /// How many the guest had sent when it was refused.
        continuations: u32,
        /// The bound.
        max: u32,
    },
    /// The fragments carried **more** bytes than the head declared.
    ///
    /// ★ The design sketch (§4.1) did not name this one and the arithmetic requires it:
    /// the alternative is `body.truncate(declared)`, which manufactures a struct the
    /// guest did not send — `abi_struct_truncation` with extra steps. A declared number
    /// and the bytes disagreeing is a refusal here, exactly as it is in
    /// [`Self::ParamsSizeExceedsPayload`] and [`Self::ControlParamsSizeMismatch`].
    ContinuationOverrun {
        /// Bytes that would have been held.
        have: usize,
        /// What the head declared.
        declared: usize,
    },
    /// An **event** id arrived in the guest's *command* queue. `GSP_INIT_DONE` and
    /// `POST_EVENT` are things we send; a guest that posts one is not speaking the
    /// protocol.
    EventFromGuest {
        /// The raw wire id.
        code: u32,
    },
    /// ★★★ **The boundary refuses this class outright** — [`kayfabe_abi::capability`],
    /// the port of the C's nvproxy-derived default-deny allowlist
    /// (`C: src/qemu/nvkvm_fe_alloc_allowlist.h`).
    ///
    /// ## Why this is a *different* refusal from [`Self::UnmappedAllocClass`]
    ///
    /// They answer different questions, and collapsing them would lose the one that
    /// matters. `UnmappedAllocClass` says *"the guest may allocate this and we have not
    /// built the decoder"* — a **modelling gap**, and the arm that becomes a decoder as
    /// the port grows. This says *"the guest may not allocate this at all"* — a
    /// **policy** answer that does not change when the port grows, and the only one of
    /// the two that is still a refusal after every decoder is written.
    ///
    /// ★ That distinction is the whole gap being closed. Before this variant, the only
    /// thing standing between a guest and an arbitrary class was whether we happened to
    /// decode it, so *adding a decoder widened the security boundary as a side effect*.
    /// The gate now runs first and the two are independent.
    ///
    /// Carries the [`Denial`] so a census can tell a class we deliberately refuse (with a
    /// reason, e.g. `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR`) from one nobody has ever seen.
    AllocClassNotPermitted {
        /// `hClass`.
        class: u32,
        /// Deliberately refused, or simply not on the list.
        denial: Denial,
    },
    /// ★★★ **The boundary refuses this control outright** — the control half of
    /// [`Self::AllocClassNotPermitted`], ported from
    /// `C: src/qemu/nvkvm_ctrl_allowlist.h`.
    ///
    /// Distinct from [`Self::UnknownControl`] for the same reason and in the same
    /// direction: `UnknownControl` is *"permitted, and this port has no arm for it"* —
    /// which is where §1.2's `Translation::Forward` lands when `classify_control` grows
    /// one — while this is *"not permitted, and a `Forward` arm must never see it"*.
    /// Checking the permit **before** the params table is what makes that true: an
    /// unlisted command is refused before anything decodes a byte of its payload.
    ControlNotPermitted {
        /// `cmd`.
        cmd: u32,
        /// Deliberately refused, or simply not on the list.
        denial: Denial,
    },
    /// A `GSP_RM_ALLOC` whose class this stage has no `AllocFacts` decoder for.
    ///
    /// ★ Not a silent `AllocFacts::default()`. §2.2b sanctions default facts for classes
    /// whose decoder is merely missing — a channel with no declared VASpace materialises
    /// no `Vas` and hangs at its first doorbell rather than answering wrongly — but that
    /// is B3's argument to make per class, with a decoder and an offsets assertion behind
    /// it. At B1 the honest answer is a refusal that names the class.
    UnmappedAllocClass {
        /// `hClass`.
        class: u32,
    },
    /// `flags & RMAPI_RPC_FLAGS_SERIALIZED`: `params[]` is FINN-serialized, so it is not
    /// the flat `#[repr(C)]` struct and **every** per-class offset would be wrong.
    ///
    /// Fires on a *declared bit* (`ogkm-610: rpc.c:11018-11022`,
    /// `ogkm-580: :11212-11216` — same code), never on a length
    /// heuristic. Which classes set it is `[unverified]`; if a boot-path class turns out
    /// to, this refusal is where that is discovered rather than a mis-decode.
    SerializedParams {
        /// `hClass`.
        class: u32,
    },
    /// The same bit on the **control** side: `rmapiRpcFlags & RMAPI_RPC_FLAGS_SERIALIZED`
    /// (`ogkm-610: rpc.c:10805-10806`, `ogkm-580: :11000-11001`).
    ///
    /// A separate variant rather than a reused one, because it carries a different
    /// number and answers a different open question: `gsp_core_bridge.md` §7 item 3 asks
    /// which *alloc classes* serialize, and which *controls* do is not the same list.
    ///
    /// ★ It fires on `NVBIT(1)` alone. `rmapiRpcFlags` also carries
    /// `RMAPI_RPC_FLAGS_COPYOUT_ON_ERROR` = `NVBIT(0)`, set independently
    /// (`ogkm-610: rpc.c:10802-10803`, `ogkm-580: :10997-10998`), so a `!= 0` test on the
    /// whole word would refuse
    /// every control that merely asked for copy-out-on-error.
    SerializedControlParams {
        /// `cmd`.
        cmd: u32,
    },
    /// The guest declared a `paramsSize` larger than the payload that arrived. Guest-
    /// declared numbers are assertions, not facts: refused with both, never clamped to
    /// the smaller (clamping is how a truncated struct gets zero-extended into a
    /// plausible one — `abi_struct_truncation`).
    ParamsSizeExceedsPayload {
        /// What the guest said.
        declared: u32,
        /// What was actually there, after the fixed header.
        available: usize,
    },
    /// A client-root alloc whose two declarations of its own handle disagree: the RPC
    /// header's `hClient` and `NV0000_ALLOC_PARAMETERS.hClient` are the same fact twice
    /// (RM stamps the second — `ogkm-610: src/nvidia/src/kernel/rmapi/client.c:225-227`,
    /// `ogkm-580: client.c:219-221`, which is the same store behind an extra
    /// `status == NV_OK` guard), so
    /// a disagreement means we have mis-decoded, not that the guest meant something
    /// clever.
    ClientHandleDisagrees {
        /// The RPC header's `hClient` — the authoritative namespace.
        header: u32,
        /// The alloc params' `hClient`.
        params: u32,
    },
    /// ★★★ **A client root from a guest OS whose privilege rule we do not have.**
    ///
    /// The configured [`GuestOs`] profile has no [`kayfabe_abi::ClientKindRule`], so
    /// `NV0000_ALLOC_PARAMETERS.processID` cannot be turned into a
    /// [`kayfabe_arch::ClientKind`] — and this refuses instead of picking one.
    ///
    /// ## Why it is a refusal and not a default
    ///
    /// Until 2026-07-29 there was no profile: the bridge applied the **Linux** rule to
    /// every guest, unconditionally and silently. That rule is
    /// `processID == KERNEL_PID (0xFFFF_FFFF) → Kernel`, and the driver gates the
    /// sentinel on `RMCFG_FEATURE_PLATFORM_UNIX`
    /// (`ogkm-580: src/nvidia/inc/kernel/vgpu/rpc.h:67-77` /
    /// `ogkm-610: rpc.h:67-77`, byte-identical) — so on a Windows guest every
    /// kernel-privileged RM client declares a real pid, classifies as
    /// `ClientKind::User { pid }`, and **joins whatever isolate that pid names**.
    /// Decision #14 groups a host isolate per user pid, so the WDDM kernel's clients
    /// would have shared a blast radius with a guest process.
    ///
    /// ★★ That was not a wrong answer that stops something — it was a *plausible* answer
    /// produced in silence, on the one path whose job is to decide who may reach whose
    /// memory. Replacing it with a different silent answer is not a fix, so the arm for
    /// an OS whose rule is unmeasured is this: named, counted by [`Faulted::fault_tag`],
    /// and `NV_ERR_NOT_SUPPORTED` on the guest's own client root — which fails its boot
    /// at the first RPC, loudly, instead of running it wrong.
    ///
    /// Carries the profile and the unclassified `processID` verbatim, so a census entry
    /// distinguishes "misconfigured guest OS" from "the rule changed under us".
    ClientKindRuleUnknown(ClientKindRuleUnknown),
    /// A `GSP_RM_CONTROL` whose `cmd` this port does not model — **and B4's answer to
    /// `gsp_core_bridge.md` §7 item 6, which asked for one.**
    ///
    /// §1.2 sketches `Translation::Forward { client, object, cmd, params }` for the
    /// control long tail, to be handed to `kayfabe_fwd::classify_control`. That
    /// classification table **does not exist**, so today a `Forward` would be a value
    /// every caller drops — which is the C's `NV_OK` echo with a Rust type on it
    /// (`nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:3214-3226`), and the whole
    /// point of §4 is that the two failure directions are not symmetric.
    ///
    /// So B4 picks **refuse**, on the crate's own consumer-first rule: a variant nothing
    /// consumes is a variant no test can bite. The choice is deliberately the reversible
    /// one — when `classify_control` lands, *this arm* is where `Forward` is emitted
    /// instead, and every control that reaches it is already named in the census.
    ///
    /// ★ Narrowed: a control admitted by a **rule** rather than by a table row is
    /// [`Self::GspRuleControlUnserviced`] instead, because the two are not the same
    /// finding. This arm now means *"a row (or nothing) named it and we have no decoder"*.
    UnknownControl {
        /// `cmd`.
        cmd: u32,
    },
    /// ★★★ **A control the capability gate admitted by a *rule* — meaning "the GSP
    /// services this" — which this GSP does not service.** The default answer to the
    /// GSS-legacy long tail, and the thing that keeps it from being answered by accident.
    ///
    /// # 1. Why this is not [`Self::UnknownControl`]
    ///
    /// Both are "permitted, no decoder". They are different findings because they were
    /// permitted for different reasons, and only one of the reasons is *void here*:
    ///
    /// - `UnknownControl` — a table row (or the absence of one plus a table lookup) put
    ///   the command in scope. Nothing about that says who answers it.
    /// - **this** — [`kayfabe_abi::capability::PassthroughRule`] put it in scope, and the
    ///   rule's whole content is *"the GPU System Processor implements this, so its params
    ///   hold no application pointers"* (nvproxy,
    ///   `gvisor/pkg/sentry/devices/nvproxy/frontend.go:769-780`). In **Mode 1** that is a
    ///   complete argument, because the ioctl is replayed on a real host `/dev/nvidia*` and
    ///   a real GSP answers. In **Mode 2 the guest's GSP is ours**, so the rule has
    ///   admitted precisely the set of commands *with nothing behind them* — it decided
    ///   "may the guest send it?", never "what do we answer?".
    ///
    /// Collapsing the two would hide the second question inside the first, which is a
    /// re-run of the collapse this crate's module doc already names as how the C ended up
    /// answering everything `NV_OK`.
    ///
    /// # 2. Why the default is a **refusal** and not a forward or a replay
    ///
    /// The C research artifact resolved one concrete instance of this — the cudart
    /// initialisation-gate cluster, `0x2080_9009` / `0x2080_9001` / `0x2080_9064`, all
    /// three GSS-legacy and all three non-privileged
    /// (`cmd & RM_GSS_LEGACY_MASK_PRIVILEGED != RM_GSS_LEGACY_MASK_PRIVILEGED`) — as
    /// **forward to the host GPU, replay a capture if the forward fails**
    /// (`C: src/qemu/nvkvm_gpu_emul.c:3328-3395`). That resolution is right and it is
    /// where this arm should eventually go. Neither half is available as a *default*:
    ///
    /// - **Forward** needs `kayfabe_fwd::classify_control`, which does not exist. A
    ///   `Translation::Forward` nothing consumes is the C's `NV_OK` echo with a Rust type
    ///   on it — the argument [`Self::UnknownControl`] already makes, unchanged.
    /// - **Replay** is definitionally unavailable for an *unknown* command: you can only
    ///   replay what was captured, and the C captured exactly three. A replay table is the
    ///   right home for those three when someone measures them (and it is a
    ///   [`kayfabe_abi`] table row, not a logic-crate edit, so it costs no change here) —
    ///   but it cannot be the answer for the tail.
    ///
    /// So the default is the one honest remaining answer, and it is chosen the same
    /// reversible way B4 chose: when a forward arm lands, *this* is the site it replaces,
    /// and every command that reached it is already named and counted in the census.
    ///
    /// # 3. ★★★ Why a refusal must be delivered at the **envelope**, which is the part
    /// that was not obvious
    ///
    /// The C's measured failure mode was not a crash. Its default echo returned all-zeros
    /// under `NV_OK`; cudart read `0` where it expected real data and aborted with
    /// `cudaErrorInitializationError(3)` **silently** — the rejection was in the reply
    /// *payload*, not an errno and not a log line (`C: src/qemu/nvkvm_gpu_emul.c:3335-3360`).
    /// A wrong number is worse than a crash because nothing reports it.
    ///
    /// Reading the guest's own receive path shows that failure has a **worse Mode-2 form
    /// than the C ever hit**, and shows exactly which field prevents it. In
    /// `rpcRmApiControl_GSP` (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:10855`,
    /// `ogkm-610: :10660`) there are two independent status words, and they are not
    /// interchangeable:
    ///
    /// | word | who sets it | what it gates |
    /// |---|---|---|
    /// | the RPC **envelope**'s `rpc_result` | [`kayfabe_gsp::Reply::rpc_result`] | `_issueRpcAndWait` returns non-`NV_OK` (`ogkm-580: rpc.c:1994`, `ogkm-610: :2012`), so the entire `if (status == NV_OK) { … }` block is skipped |
    /// | `rpc_params->status`, inside the control body | a body we would have to forge | only the copy-out, and only conditionally |
    ///
    /// Inside that block, two things happen to a *successful* reply and **neither is
    /// reachable once the envelope says no**:
    ///
    /// 1. **Copy-out.** The skip is *conditional*, not automatic:
    ///    `if (rpc_params->status != NV_OK && !(rmapiRpcFlags & RMAPI_RPC_FLAGS_COPYOUT_ON_ERROR))`
    ///    (`ogkm-580: rpc.c:11065-11069`, `ogkm-610: :10870-10874`). The guest sets that bit
    ///    itself, on the wire, for any control carrying `RMCTRL_FLAGS_COPYOUT_ON_ERROR`
    ///    (`ogkm-580: rpc.c:10997-10998`, `ogkm-610: :10802-10803`) — so for those commands a
    ///    body-level failure still copies our bytes to the caller.
    /// 2. **★★ The guest CACHES the answer.** GSS-legacy controls have their own cache
    ///    path in the guest's CPU-RM, on both the set and the get side:
    ///    `rmapiControlCacheSetUnchecked(hClient, hObject, cmd, rpc_params->params, …)`
    ///    (`ogkm-580: rpc.c:11098-11103`, `ogkm-610: :10903-10908`) and, on the next call,
    ///    `else if (IsGssLegacyCall(cmd)) rmapiControlCacheGetUnchecked(…)` followed by
    ///    `if (rmctrlCacheStatus == NV_OK) goto done;` (`ogkm-580: rpc.c:10962-10971`,
    ///    `ogkm-610: :10766-10775`) — i.e. **the RPC never reaches us again**.
    ///
    ///    ★ And the branch's own predicate reads `rpc_params->rmctrlFlags` and
    ///    `rpc_params->rmctrlAccessRight` — *fields the replying GSP fills*
    ///    (`rmapiControlIsCacheable`, `ogkm-580: src/nvidia/src/kernel/rmapi/rmapi_cache.c:152-174`,
    ///    byte-identical at `ogkm-610: :152-174`). So **whether the guest permanently caches
    ///    our answer is decided by bytes we put in the reply.** Today the echo path happens
    ///    not to cache, purely because `RpcCommand::ack` reflects the request, in which the
    ///    guest zeroed `rmctrlFlags` itself (`ogkm-580: rpc.c:10990-10991`,
    ///    `ogkm-610: :10795-10796`), and `!(0 & RMCTRL_FLAGS_CACHEABLE_ANY)` is false. That
    ///    is an accident, and it is exactly the class of accident this refusal exists to
    ///    stop being load-bearing.
    ///
    /// ⇒ The one reply that is safe *regardless of any byte in the body* is one whose
    /// **envelope** `rpc_result` is non-zero, because the guest short-circuits before both
    /// hazards. [`BridgeRefusal::rpc_result`] is that word and it is
    /// [`kayfabe_abi::NV_ERR_NOT_SUPPORTED`] for every refusal, so the guarantee is
    /// structural rather than per-variant. `gss_legacy_answer.rs` is the test that watches
    /// it hold.
    ///
    /// # 4. Mode 1 is untouched
    ///
    /// This is a `GSP_RM_CONTROL` decoder on the Mode-2 RPC transport. Mode 1 forwards a
    /// guest *ioctl* to a real host driver, which is nvproxy's exact situation, where
    /// pass-through is correct — and nothing on that path routes through this crate.
    GspRuleControlUnserviced {
        /// `cmd`.
        cmd: u32,
        /// Which rule admitted it. Carried, not dropped: `GssLegacy` is half the command
        /// space and `BinApi` is one class, so a census that could not tell them apart
        /// would answer "which rule is costing us?" with a shrug.
        rule: PassthroughRule,
    },
    /// ★★ **A control that moves a VASpace's page-directory binding, which this port
    /// cannot express.** Distinct from [`Self::UnknownControl`] because the consequence is
    /// distinct: an unmodelled control is a fact we do not have, while a *dropped*
    /// page-directory declaration is a `Vas` that will never route and a channel that
    /// defers at its first doorbell **forever**, with nothing anywhere saying why.
    ///
    /// The three commands and the evidence are on
    /// [`kayfabe_abi::versions::ControlParams::PageDirNotModelled`]. The headline is
    /// `NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES` (`0x90f10106`), which settles
    /// `gsp_core_bridge.md` §7 item 1 *against* the design's assumption: on a bare-metal
    /// GSP client it is issued at construct time for every split-VAS-eligible VASpace and
    /// carries the root page directory as `levels[0].physAddress`, so for an ordinary
    /// RM-managed VAS it is the **only** message that carries a PDB —
    /// `SET_PAGE_DIRECTORY` asserts out for anything that is not
    /// `SHARED_MANAGEMENT`/`IS_EXTERNALLY_OWNED`
    /// (`ogkm-580: src/nvidia/src/kernel/mem_mgr/gpu_vaspace.c:3109`).
    ///
    /// This refusal is therefore the stage's most valuable output: it is the named,
    /// countable record of exactly which control the address plane is still missing.
    PageDirControlNotModelled {
        /// `cmd`.
        cmd: u32,
    },
    /// A control declared a `paramsSize` that is not its command's own struct size.
    ///
    /// Not "at least": `NV_RM_RPC_CONTROL` is called with `sizeof(…)` verbatim
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/dma.c:508-518`), so a different
    /// number is a guest that means a different struct — and taking the first `SIZE`
    /// bytes of it would be `abi_struct_truncation` with extra steps.
    ControlParamsSizeMismatch {
        /// `cmd`.
        cmd: u32,
        /// What the guest said.
        declared: u32,
        /// What the command's params struct actually measures.
        expected: usize,
    },
    /// ★ A `SET_PAGE_DIRECTORY` naming `hVASpace == 0`.
    ///
    /// This is **not** "no VASpace declared". NVIDIA's own header says what it is, in
    /// both vendored trees verbatim: *"handle for the allocated VA space that this
    /// control call should operate on. **If it's 0, it assumes to use the implicit
    /// allocated VA space associated with the client/device pair**"*
    /// (`ogkm-610: ctrl0080dma.h:782-785`, `ogkm-580: ctrl0080dma.h:812-815`).
    ///
    /// That implicit VAS is a real object the RPC does not name and the graph has no node
    /// for. Passing `HObject(0)` through would attach the PDB to a node key the guest
    /// never declared, where it parks silently and forever — a fact landing in the wrong
    /// component, which is the exact shape this project has already been bitten by twice.
    /// So it is refused, and the refusal names the gap.
    ImplicitVaspace,
    /// `hClient == 0`. `NV01_NULL_OBJECT` is not a namespace; the core reserves
    /// `HClient(0)` as its system anchor and would refuse it too. Refused here as a
    /// well-formedness property of the *message* — which needs no graph state, and so is
    /// not a second copy of the graph's category-3 rule.
    ReservedClient,
    /// A layout decoder refused: short buffer, impossible length. Carried whole, because
    /// [`AbiError`] already names the struct and both numbers.
    Abi(AbiError),
    /// ★★ **The graph refused an otherwise well-formed fact.** B2's variant: nothing in
    /// B1 applied, so nothing could construct it, and an uncatchable variant is an
    /// unbiteable test.
    ///
    /// This is the arm that keeps §3.4 honest in **both** directions. [`translate`]
    /// deliberately does not ask whether a referenced object exists — that is a *lookup*,
    /// and a lookup here would be a second, weaker copy of a rule `RmGraph::apply`
    /// already owns. But not pre-empting the graph is only half of it: the answer must
    /// not be swallowed either, and the C's failure was exactly that — it accepted
    /// everything and replied `NV_OK`
    /// (`nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:3326`).
    ///
    /// ★ It is a real surface, not a formality. `RmGraph` does **not** tolerate every
    /// `Free`: only the *teardown-verb exemption* in `undeclared_namespace` lets a free
    /// name an undeclared **namespace**, so a free of an undeclared **object** — or a
    /// double-free of a client root — is `RmGraphError::FreeUnknown`. That is faithful RM
    /// behaviour and it reaches the guest as a non-zero `rpc_result`.
    ///
    /// ★★ **B5 widened the surface to the whole namespace rule**, because `DUP_OBJECT` is
    /// the only verb that names **two** client namespaces. Four graph refusals are now
    /// reachable from wire bytes that B4 could not construct at all:
    /// `RmGraphError::ConflictingDup` (a dst handle already bound to a different
    /// resource), `::UndeclaredClient` for a dup's **dst** *and* — separately — for its
    /// **src**, and `::ReservedClient` from a `hClientSrc` of zero. The last one is the
    /// evidence that this arm is doing real work rather than decorating: it is a
    /// well-formedness question the bridge deliberately does **not** answer locally, and
    /// the answer still reaches the guest, named.
    ///
    /// [`GpuError`] is carried whole, and [`Faulted`] **delegates** to it, so the census
    /// records *which protocol rule* was broken (`RmGraphError::FreeUnknown` vs
    /// `::ConflictingAlloc` are different findings) rather than one flat "the graph said
    /// no" — the same argument `kayfabe_core`'s own `impl Faulted for GpuError` makes.
    Graph(GpuError),
    /// ★★ **The address-plane join refused the promotion.**
    ///
    /// The [`Self::Graph`] arm's counterpart for the other plane, and a separate variant
    /// for the same reason that one exists separately from the decode refusals: the
    /// bridge does not answer these questions locally (it resolves nothing and looks
    /// nothing up), and the answer must still reach the guest as a non-zero
    /// `rpc_result` rather than being swallowed into an `NV_OK`.
    ///
    /// The census tag **delegates** to the fault, so a promotion refused for naming a
    /// foreign address space is a different finding from one refused for colliding with
    /// a live binding.
    Promote(kayfabe_core::promote::PromoteFault),
}

impl From<AbiError> for BridgeRefusal {
    fn from(e: AbiError) -> Self {
        BridgeRefusal::Abi(e)
    }
}

impl From<ClientKindRuleUnknown> for BridgeRefusal {
    fn from(e: ClientKindRuleUnknown) -> Self {
        BridgeRefusal::ClientKindRuleUnknown(e)
    }
}

impl BridgeRefusal {
    /// The `NV_STATUS` a reply carries when this refusal is what happened.
    ///
    /// ★★ **B4 revisited `gsp_core_bridge.md` §4.2's `[open]` and kept one value — with
    /// an argument instead of a precedent.** The full evidence is on
    /// [`kayfabe_abi::NV_ERR_NOT_SUPPORTED`] and
    /// [`kayfabe_abi::NV_VGPU_MSG_RESULT_VMIOP_BASE`]; the three facts that decided it:
    ///
    /// 1. the guest **collapses** every `rpc_result` at or above `0xFF000000` to one
    ///    indistinguishable `NV_ERR_GENERIC` (`ogkm-610: rpc.c:2023-2026`,
    ///    `ogkm-580: :2004-2007`; `_issueRpcLarge` repeats the identical collapse at
    ///    `ogkm-610: :2237-2240` / `ogkm-580: :2216-2219`), so a status
    ///    above the base cannot say anything at all — `0x56` is below it and arrives
    ///    verbatim;
    /// 2. `rpcRmApiControl_GSP` already lists `NV_ERR_NOT_SUPPORTED` among the statuses
    ///    it logs *quietly* (`ogkm-610: rpc.c:10913-10920`, `ogkm-580: :11108-11115`) — it
    ///    is an ordinary outcome to
    ///    the driver, not an anomaly;
    /// 3. the obvious alternative, `NV_VGPU_MSG_RESULT_RPC_API_CONTROL_NOT_SUPPORTED`
    ///    (`0xFF100009`), is translated back to a real `NV_STATUS` only on the vGPU
    ///    `RM_API_CONTROL` path and **not** on fn 76
    ///    (`ogkm-610: rpc.c:5432-5437`, `ogkm-580: :5425-5430` vs
    ///    `rpcRmApiControl_GSP`), so on our path it would reach the RM caller as a value
    ///    that is not an `NV_STATUS`.
    ///
    /// It stays **one value for every variant** because nothing observed constrains a
    /// split: the *variant* is what the census records, and inventing a per-refusal
    /// status table would be a table of guesses in the one place where a wrong entry is
    /// invisible until a guest trips it. The distinction lives in
    /// [`Faulted::fault_tag`], which costs the guest nothing.
    #[must_use]
    pub const fn rpc_result(self) -> u32 {
        NV_ERR_NOT_SUPPORTED
    }
}

impl Faulted for BridgeRefusal {
    /// Exhaustive by construction, so a new refusal variant cannot reach the wire without
    /// reaching the trace.
    fn fault_tag(&self) -> FaultTag {
        match self {
            BridgeRefusal::UnknownFunction { .. } => FaultTag("BridgeRefusal::UnknownFunction"),
            BridgeRefusal::ContinuationWithoutHead { .. } => {
                FaultTag("BridgeRefusal::ContinuationWithoutHead")
            }
            BridgeRefusal::ContinuationInterleaved { .. } => {
                FaultTag("BridgeRefusal::ContinuationInterleaved")
            }
            BridgeRefusal::ContinuationOverflow { .. } => {
                FaultTag("BridgeRefusal::ContinuationOverflow")
            }
            BridgeRefusal::ContinuationCountExceeded { .. } => {
                FaultTag("BridgeRefusal::ContinuationCountExceeded")
            }
            BridgeRefusal::ContinuationOverrun { .. } => {
                FaultTag("BridgeRefusal::ContinuationOverrun")
            }
            BridgeRefusal::EventFromGuest { .. } => FaultTag("BridgeRefusal::EventFromGuest"),
            BridgeRefusal::UnmappedAllocClass { .. } => {
                FaultTag("BridgeRefusal::UnmappedAllocClass")
            }
            // ★ Split by [`Denial`], not flat. "A class we deliberately refuse" and "a
            // class nobody has ever seen" are the two findings a security census exists
            // to tell apart: the first is a guest doing something we named, the second is
            // a guest exploring. One tag for both would make a probe indistinguishable
            // from an unimplemented feature — which is exactly the collapse
            // `ClientKindRuleUnknown`'s comment argues the *other* way about, because
            // there the contained value was not a rule and here it is.
            BridgeRefusal::AllocClassNotPermitted {
                denial: Denial::Refused { .. },
                ..
            } => FaultTag("BridgeRefusal::AllocClassNotPermitted::Refused"),
            BridgeRefusal::AllocClassNotPermitted {
                denial: Denial::NotOnAllowlist,
                ..
            } => FaultTag("BridgeRefusal::AllocClassNotPermitted::NotOnAllowlist"),
            BridgeRefusal::ControlNotPermitted {
                denial: Denial::Refused { .. },
                ..
            } => FaultTag("BridgeRefusal::ControlNotPermitted::Refused"),
            BridgeRefusal::ControlNotPermitted {
                denial: Denial::NotOnAllowlist,
                ..
            } => FaultTag("BridgeRefusal::ControlNotPermitted::NotOnAllowlist"),
            BridgeRefusal::SerializedParams { .. } => FaultTag("BridgeRefusal::SerializedParams"),
            BridgeRefusal::SerializedControlParams { .. } => {
                FaultTag("BridgeRefusal::SerializedControlParams")
            }
            BridgeRefusal::UnknownControl { .. } => FaultTag("BridgeRefusal::UnknownControl"),
            // ★ Split by [`PassthroughRule`], for the reason `AllocClassNotPermitted`
            // splits by `Denial`: the contained value IS a rule, and the two rules are
            // different-sized holes. `GssLegacy` is half the command space and is where
            // the C's cudart cluster lives; `BinApi` is one class. A census that merged
            // them could not answer which rule the long tail is actually arriving through,
            // which is the one number that decides what gets a forward arm first.
            BridgeRefusal::GspRuleControlUnserviced {
                rule: PassthroughRule::GssLegacy,
                ..
            } => FaultTag("BridgeRefusal::GspRuleControlUnserviced::GssLegacy"),
            BridgeRefusal::GspRuleControlUnserviced {
                rule: PassthroughRule::BinApi,
                ..
            } => FaultTag("BridgeRefusal::GspRuleControlUnserviced::BinApi"),
            BridgeRefusal::PageDirControlNotModelled { .. } => {
                FaultTag("BridgeRefusal::PageDirControlNotModelled")
            }
            BridgeRefusal::ControlParamsSizeMismatch { .. } => {
                FaultTag("BridgeRefusal::ControlParamsSizeMismatch")
            }
            BridgeRefusal::ImplicitVaspace => FaultTag("BridgeRefusal::ImplicitVaspace"),
            BridgeRefusal::ParamsSizeExceedsPayload { .. } => {
                FaultTag("BridgeRefusal::ParamsSizeExceedsPayload")
            }
            BridgeRefusal::ClientHandleDisagrees { .. } => {
                FaultTag("BridgeRefusal::ClientHandleDisagrees")
            }
            // ★ Flat, not delegated. The contained value names an OS and a `processID`,
            // neither of which is a *rule* the census could count — what the census wants
            // to say is "a client root arrived that this profile cannot classify", and
            // that is one finding however many pids reach it.
            BridgeRefusal::ClientKindRuleUnknown(_) => {
                FaultTag("BridgeRefusal::ClientKindRuleUnknown")
            }
            BridgeRefusal::ReservedClient => FaultTag("BridgeRefusal::ReservedClient"),
            BridgeRefusal::Abi(_) => FaultTag("BridgeRefusal::Abi"),
            // ★ Delegated, not flattened: `kayfabe_core`'s own `impl Faulted for
            // GpuError` delegates for the same reason, so a graph refusal is countable
            // by the rule it broke. The refusal VALUE still says it came through the
            // bridge; the tag says what actually went wrong, which is what a census is
            // for.
            BridgeRefusal::Graph(e) => e.fault_tag(),
            // ★ Delegated for the same reason `Graph` is.
            BridgeRefusal::Promote(e) => e.fault_tag(),
        }
    }
}

/// Translate one decoded GSP RPC into what it means to the object model.
///
/// A **pure function of one message**: no `&mut self`, no guest memory, no host state, no
/// lookup, no minted identity. See the crate docs for why each of those is load-bearing.
///
/// ## ★ Two declared keys, not one
///
/// `abi` is Axis A — *which driver version's layouts do these bytes have?* `guest_os` is
/// `four_axes_of_variation.md`'s fourth axis — *which OS built that driver?* They are
/// independent, and the doc is explicit that collapsing them is a mistake
/// (*"do not collapse guest OS into the version key"*), so they arrive as two parameters
/// and never as one table.
///
/// Both are **declared at realize**, never sniffed. The guest OS in particular is
/// undetectable on the wire — it is a `#define` in the guest driver's build — so a
/// function that inferred it from traffic would be inferring an isolation boundary from a
/// value the guest chooses ([`GuestOs`]).
///
/// # Errors
///
/// [`BridgeRefusal`], by variant. A refusal is never a drop — the caller still owes the
/// guest a reply carrying [`BridgeRefusal::rpc_result`].
pub fn translate(
    abi: &DriverAbiTable,
    guest_os: GuestOs,
    cmd: &RpcCommand,
) -> Result<Translation, BridgeRefusal> {
    match cmd.function {
        RpcFunction::RmAlloc => translate_alloc(abi, guest_os, &cmd.payload),
        RpcFunction::Free => translate_free(abi, &cmd.payload),
        RpcFunction::RmControl => translate_control(abi, &cmd.payload),
        RpcFunction::DupObject => translate_dup(abi, &cmd.payload),
        // Known and inert — three different reasons, collapsed here only because the
        // *answer* is the same. See `Translation::Inert`.
        RpcFunction::SetGuestSystemInfo
        | RpcFunction::GetGspStaticInfo
        | RpcFunction::UnloadingGuestDriver
        | RpcFunction::GspSetSystemInfo
        | RpcFunction::SetRegistry => Ok(Translation::Inert),
        // ★★ B6, and the one arm whose answer is a property of this function's
        // signature. A continuation record is a *transport fragment*: it carries a raw
        // byte slice and no function of its own, so one message's worth of it cannot be
        // translated by anything, ever. `translate` is a function of ONE message and from
        // that view there is never a head in flight — so the honest answer here is the
        // no-head refusal, and [`Reassembler`] (which holds the head, and is the only
        // thing in the tree that may) is what turns a *run* of them into a message.
        //
        // The staging state this arm used to be in — "known, mapped, arm not built" — is
        // now empty, and `BridgeRefusal::NotYetTranslated` is gone with it.
        RpcFunction::ContinuationRecord => {
            Err(BridgeRefusal::ContinuationWithoutHead { code: cmd.code })
        }
        // Ours to send, never to receive.
        //
        // ★ `RcTriggered` joins this arm rather than getting one of its own, and the
        // direction is what makes it safe to fold: it is the simulated-fault carrier
        // *we* post (`fault::rc_triggered_for`), and a guest that sends one is a guest
        // telling us its own hardware faulted. There is no reading of that which is not
        // hostile or broken, so it is refused by the same name as the other two.
        RpcFunction::InitDone | RpcFunction::PostEvent | RpcFunction::RcTriggered => {
            Err(BridgeRefusal::EventFromGuest { code: cmd.code })
        }
        RpcFunction::Other(code) => Err(BridgeRefusal::UnknownFunction { code }),
    }
}

/// A declared handle field, as [`AllocFacts`] models it: `NV01_NULL_OBJECT` is the
/// protocol's way of saying *nothing is declared here*, and the core spells that `None`
/// (`AllocFacts::h_vaspace`: *"`None` models `hVASpace=0` (GSP-managed)"*).
///
/// ★ Written once and used by all three VAS-declaring classes, because the alternative —
/// `Some(HObject(0))` — is a node key the graph would then try to resolve, and a failed
/// resolve of a handle the guest never declared is a MISS the guest cannot fix. The
/// difference is DEFER-forever versus correctly-nothing.
fn declared_handle(handle: u32) -> Option<HObject> {
    (handle != 0).then_some(HObject(handle))
}

/// `GSP_RM_ALLOC` (fn 103) → [`RmEvent::Alloc`].
///
/// ## The class table
///
/// Which params shape a class carries is [`kayfabe_abi::versions::DriverAbiTable::alloc_params`]'s
/// answer, not this function's: every NVIDIA class *number* stays behind decision #2's
/// quarantine and this crate names none. An unmapped class is
/// [`BridgeRefusal::UnmappedAllocClass`] — **not** a silent `AllocFacts::default()`.
///
/// ★ That refusal is the deviation from `gsp_core_bridge.md` §2.2b, which sanctions
/// default facts for a class whose decoder is merely missing on the argument that a
/// channel with no declared VASpace *hangs at its first doorbell rather than answering
/// wrongly*. The argument is sound for a channel and **false for the classes above it**:
/// a Device with no declared `deviceInstance` is unroutable, and a client root with no
/// declared `client_kind` is a hard `RmGraphError::UndeclaredClientKind` by design. A
/// blanket default cannot tell those apart, so the default is a refusal and each class
/// argues its way out of it with a decoder and an offsets assertion behind it.
///
/// ## ★ The client-root normalisation, and why it is required
///
/// On the wire a client root arrives with `hParent = hObject = NV01_NULL_OBJECT = 0`: the
/// driver's own macro calls `AllocWithHandle(pRmApi, hclient, NV01_NULL_OBJECT,
/// NV01_NULL_OBJECT, NV01_ROOT, …)`
/// (`ogkm-610: src/nvidia/inc/kernel/vgpu/rpc.h:85-87`; `ogkm-580:` byte-identical at the
/// same lines) and
/// `rpcRmApiAlloc_GSP` copies all three through verbatim (`ogkm-610: rpc.c:11007-11009`,
/// `ogkm-580: :11201-11203`).
/// The core, meanwhile, requires `parent == handle` for a root
/// (`kayfabe_core::rmgraph::RmEvent::Alloc`'s doc). Passing `0/0` through would create a
/// node at `(client, HObject(0))` whose relationship to the namespace is accidental.
///
/// The fix is **NVIDIA's own rule**, not an invention: in RM the `hClient` *is* its root
/// object's handle — `serverAllocClient` writes `pParams->hResource = hClient`
/// (`ogkm-610: src/nvidia/src/libraries/resserv/src/rs_server.c:625`; `ogkm-580:`
/// byte-identical at the same line).
///
/// It applies to the client root **and to nothing else**: every other class's `hParent`
/// and `hObject` are copied verbatim, which is why the normalisation cannot leak into a
/// class that did not ask for it.
fn translate_alloc(
    abi: &DriverAbiTable,
    guest_os: GuestOs,
    payload: &[u8],
) -> Result<Translation, BridgeRefusal> {
    let h = abi.decode_rpc_alloc(payload)?;

    // ── The namespace, read ONCE, from the header, before anything else. Everything
    // below is a property of the message; nothing below may replace this value.
    let client = HClient(h.client);
    if client == RESERVED_CLIENT {
        return Err(BridgeRefusal::ReservedClient);
    }

    // ── Is `params[]` even the shape every offset below assumes? Asked before the
    // offsets are used, and asked of a declared bit rather than of a length.
    if rpc_params_are_serialized(h.params_flags) {
        return Err(BridgeRefusal::SerializedParams { class: h.class });
    }

    // ── The declared params window. `paramsSize` is the guest's assertion about its own
    // message, so it is bounded by what arrived and never trusted past it.
    let declared = h.params_size as usize;
    let Some(params) = payload
        .get(h.params_at..)
        .and_then(|tail| tail.get(..declared))
    else {
        return Err(BridgeRefusal::ParamsSizeExceedsPayload {
            declared: h.params_size,
            available: payload.len().saturating_sub(h.params_at),
        });
    };

    let class = ClassId(h.class);

    // ── ★★★ The capability gate, and it runs BEFORE the params table on purpose.
    // "May the guest allocate this?" is a policy question whose answer must not depend
    // on whether this port happens to have a decoder — otherwise writing a decoder
    // silently widens the boundary. Default-deny (`kayfabe_abi::capability`).
    if let AllocPermit::Denied(denial) = abi.capabilities().alloc_class(class) {
        return Err(BridgeRefusal::AllocClassNotPermitted {
            class: h.class,
            denial,
        });
    }

    let Some(shape) = abi.alloc_params(class) else {
        return Err(BridgeRefusal::UnmappedAllocClass { class: h.class });
    };

    // ── The one class whose *edge* is not what the wire says. Handled first and
    // separately so the normalisation is visibly unreachable from every other arm.
    if shape == AllocParams::ClientRoot {
        // The 8-byte prefix contract — `hClient` and `processID`, the only two fields of
        // `NV0000_ALLOC_PARAMETERS` with more than one oracle. See `ClientAllocFacts`.
        let facts = abi.decode_client_alloc_facts(params)?;
        if facts.h_client != h.client {
            return Err(BridgeRefusal::ClientHandleDisagrees {
                header: h.client,
                params: facts.h_client,
            });
        }
        let root = HObject(h.client);
        return Ok(Translation::Event(RmEvent::Alloc {
            client,
            parent: root,
            handle: root,
            class,
            facts: AllocFacts {
                // ★★ The one genuinely guest-OS-shaped value in the whole bridge, and as
                // of 2026-07-29 the profile that decides it is a **parameter of this
                // function** rather than an assumption baked into a free function.
                //
                // The `KERNEL_PID` sentinel branch that produces it is gated on
                // `RMCFG_FEATURE_PLATFORM_UNIX` (`ogkm-580: rpc.h:67-77` /
                // `ogkm-610: rpc.h:67-77`, byte-identical), so on a non-UNIX guest a
                // kernel-privileged client declares a *real* pid — and classifying it as
                // `ClientKind::User` folded the guest kernel's RM clients into a guest
                // process's blast radius, silently. The comment that used to sit here
                // named that defect and predicted this exact fix
                // (*"a guest-OS profile selected at realize beside the ABI table, never
                // an `if` here"*); what it could not do was make anything happen.
                //
                // So this is a `?`, not a branch: a profile with no rule REFUSES
                // (`BridgeRefusal::ClientKindRuleUnknown`) and the guest sees
                // `NV_ERR_NOT_SUPPORTED` on its client root. Nothing else in this crate is
                // OS-aware. (`gsp_core_bridge.md` §1.5, `four_axes_of_variation.md` §1.)
                client_kind: Some(guest_os.client_kind_from_process_id(facts.process_id)?),
                ..Default::default()
            },
        }));
    }

    let facts = match shape {
        // Unreachable: handled above, and left as an explicit arm rather than a `_` so a
        // future shape cannot join this match by accident.
        AllocParams::ClientRoot => {
            return Err(BridgeRefusal::UnmappedAllocClass { class: h.class });
        }
        AllocParams::Device => AllocFacts {
            // ★ Required, not optional. `deviceId` is a mandatory field of
            // `NV0080_ALLOC_PARAMETERS`, so a real Device always declares one, and the
            // core refuses to route an object whose Device ancestor declared none rather
            // than defaulting it to GPU 0 (`RmGraph::gpu_of`).
            device_instance: Some(abi.decode_device_alloc_facts(params)?.device_id),
            ..Default::default()
        },
        AllocParams::Tsg => AllocFacts {
            h_vaspace: declared_handle(abi.decode_tsg_alloc_facts(params)?.h_vaspace),
            ..Default::default()
        },
        AllocParams::CtxShare => AllocFacts {
            h_vaspace: declared_handle(abi.decode_ctxshare_alloc_facts(params)?.h_vaspace),
            ..Default::default()
        },
        AllocParams::Channel => {
            let c = abi.decode_channel_alloc_facts(params)?;
            AllocFacts {
                h_vaspace: declared_handle(c.h_vaspace),
                h_ctx_share: declared_handle(c.h_ctx_share),
                // ★ Copied verbatim and interpreted by nobody here. `Arch::vchid_from_userd_flags`
                // is the only thing that reads it, which is the seam that has to move when
                // a real arch replaces the mock — `gsp_core_bridge.md` §6 names "that the
                // `userd_flags`→`VChid` recovery matches real silicon" as precisely what
                // this stage cannot prove.
                userd_flags: c.flags,
                ..Default::default()
            }
        }
        // A mapped class that declares nothing: its params are never read, so a hostile
        // one is bytes we do not look at. The `paramsSize` bound above still applied.
        AllocParams::NoDeclaredFacts => AllocFacts::default(),
    };

    Ok(Translation::Event(RmEvent::Alloc {
        client,
        // ★ Verbatim. The namespace is the header's `hClient` (read once, at the top);
        // the edge is the header's own `hParent`/`hObject`. No params field may name
        // either — that would be the C's `GPU_PROMOTE_CTX` substitution.
        parent: HObject(h.parent),
        handle: HObject(h.handle),
        class,
        facts,
    }))
}

/// `GSP_RM_CONTROL` (fn 76) → [`RmEvent::SetPageDir`], for exactly one `cmd`.
///
/// ## Which control, and the hole that is now READ rather than suspected
///
/// ★★★ **This heading said "measured" and the claim below is a source reading.** Nothing
/// was run: the answer comes from an assert in `gpu_vaspace.c`, not from a boot. The
/// distinction is load-bearing here rather than pedantic, because what the reading
/// establishes is that this arm is **not sufficient** — and "not sufficient" is precisely
/// the kind of conclusion a live boot can refute and a source read cannot. Restated at
/// the level actually held, with the citation left exactly as specific as it was.
///
/// [`kayfabe_abi::versions::DriverAbiTable::control_params`] is the table; this function
/// names no command number. It has three outcomes and they are three different
/// statements:
///
/// - `SetPageDir` — `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY`, the one control this port
///   turns into a fact;
/// - `PageDirNotModelled` — a control **known** to move a VASpace's page-directory
///   binding, refused by name ([`BridgeRefusal::PageDirControlNotModelled`]);
/// - not in the table at all — [`BridgeRefusal::UnknownControl`], §7 item 6's decision.
///
/// ★★ The middle one is the finding. `gsp_core_bridge.md` §7 item 1 asked *"which control
/// carries the compute VAS's PDB"* and warned that if the answer is `0x90f10106` then this
/// design produces no `SetPageDir` for it at all. That is the answer: on a bare-metal GSP
/// client `SET_PAGE_DIRECTORY` reaches the wire **only** for a `SHARED_MANAGEMENT` /
/// `IS_EXTERNALLY_OWNED` VASpace — i.e. UVM's — because the handler asserts on exactly
/// that (`ogkm-580: src/nvidia/src/kernel/mem_mgr/gpu_vaspace.c:3109`), while every
/// ordinary RM-managed VAS declares its root through
/// `NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES` at construct time. So this arm is
/// necessary and **not sufficient**, and the refusal is what says so out loud instead of a
/// channel deferring forever with no record.
///
/// ## The namespace, again
///
/// `client` is `hdr.hClient`, the RPC body's own field, read once at the top. The C's
/// counter-example is `GPU_PROMOTE_CTX`, where it reads `hChanClient` out of `params+12`
/// and never looks at the envelope's (`nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:2283`).
/// `hObject` — the Device the control is issued *against* — is **dropped**:
/// [`RmEvent::SetPageDir`] has nowhere to put it, and the VASpace is named by a params
/// field, not by it.
///
/// ## What else is dropped, and the one that will matter
///
/// `numEntries`, `chId`, `subDeviceId`, `pasid` — none has a home in `RmEvent`. And
/// **`aperture`** (`flags[1:0]`), which is the interesting one:
/// [`kayfabe_abi::view::PdbAperture`] decodes it and `RmEvent::SetPageDir` has nowhere to
/// put it, so a vidmem-rooted and a sysmem-rooted page directory become *the same event*.
///
/// That is safe **exactly as long as `Pdb` is only ever a key**, which today it is:
/// `kayfabe_mmu::AddressTable` takes a `Pdb` to name a table and to name a fault, and
/// nothing in the tree dereferences one. The day a walker follows a PDB it must know
/// whether the address is a framebuffer offset or a guest-physical address — two different
/// address spaces — and `kayfabe_arch::ids::Pdb`'s own doc currently assumes the first
/// (*"a per-GPU FB address"*). Recorded here because that is the moment this drop stops
/// being free.
fn translate_control(abi: &DriverAbiTable, payload: &[u8]) -> Result<Translation, BridgeRefusal> {
    let h = abi.decode_rpc_control(payload)?;

    // ── The namespace, read ONCE, from the header, before anything else.
    let client = HClient(h.client);
    if client == RESERVED_CLIENT {
        return Err(BridgeRefusal::ReservedClient);
    }

    // ── Is `params[]` even the shape the offsets below assume? A declared bit, and only
    // the one bit — the neighbouring `COPYOUT_ON_ERROR` is not our business.
    if rpc_params_are_serialized(h.rmapi_rpc_flags) {
        return Err(BridgeRefusal::SerializedControlParams { cmd: h.cmd });
    }

    // ── The declared params window, bounded by what actually arrived.
    let declared = h.params_size as usize;
    let Some(params) = payload
        .get(h.params_at..)
        .and_then(|tail| tail.get(..declared))
    else {
        return Err(BridgeRefusal::ParamsSizeExceedsPayload {
            declared: h.params_size,
            available: payload.len().saturating_sub(h.params_at),
        });
    };

    // ── ★★★ The capability gate. Before the params table, for the reason spelled out on
    // `BridgeRefusal::ControlNotPermitted`: an unlisted command is refused before a byte
    // of its payload is decoded, and a future `Forward` arm can never be handed one.
    let permit = abi.capabilities().control(ControlCmd(h.cmd));
    if let ControlPermit::Denied(denial) = permit {
        return Err(BridgeRefusal::ControlNotPermitted { cmd: h.cmd, denial });
    }

    let shape = match abi.control_params(ControlCmd(h.cmd)) {
        Some(shape) => shape,
        // ★★★ The permitted-but-unmodelled tail, split by WHY it was permitted. A control
        // admitted by a rule was admitted on the premise that a GSP services it — which
        // in Mode 2 names our own fake GSP, so the premise is the refusal. The whole
        // argument, and the guest-side control flow that makes an envelope-level refusal
        // the only safe answer, is on `BridgeRefusal::GspRuleControlUnserviced`.
        //
        // ★ Ordering, and it is load-bearing: this sits AFTER the params-table lookup, so
        // it can only ever refine the arm that was already a refusal. A control this port
        // models is answered by its decoder whether or not its command word happens to
        // have bit 15 set — none of the modelled six does today, and this arm does not
        // depend on that staying true.
        None => {
            return Err(match permit.passthrough_rule() {
                Some(rule) => BridgeRefusal::GspRuleControlUnserviced { cmd: h.cmd, rule },
                None => BridgeRefusal::UnknownControl { cmd: h.cmd },
            });
        }
    };
    if shape == ControlParams::PageDirNotModelled {
        return Err(BridgeRefusal::PageDirControlNotModelled { cmd: h.cmd });
    }

    // ── The guest's own size assertion against the struct's actual size. Exact, per
    // §4.3: the mismatch is refused rather than resolved in either direction.
    if let Some(expected) = shape.params_size()
        && declared != expected
    {
        return Err(BridgeRefusal::ControlParamsSizeMismatch {
            cmd: h.cmd,
            declared: h.params_size,
            expected,
        });
    }

    // ★★ The address-plane control. Decoded here, applied nowhere near here: this
    // function resolves nothing and looks nothing up, so `hObject` leaves as the guest's
    // own handle and the join resolves it against the live graph at the moment it acts.
    if shape == ControlParams::PromoteCtx {
        return translate_promote_ctx(abi, client, params);
    }

    let p = abi.decode_set_page_dir(params)?;
    // ★ Zero is not "unspecified" here — it names the client/device pair's *implicit*
    // VASpace, an object this RPC does not identify. See `BridgeRefusal::ImplicitVaspace`.
    if p.h_vaspace == 0 {
        return Err(BridgeRefusal::ImplicitVaspace);
    }
    Ok(Translation::Event(RmEvent::SetPageDir {
        client,
        vaspace: HObject(p.h_vaspace),
        pdb: Pdb(p.phys_address),
    }))
}

/// ★★ `NV2080_CTRL_CMD_GPU_PROMOTE_CTX` → [`Translation::CtxPromotion`].
///
/// # The two clients, and the rule that needed a SCOPE rather than enforcement
///
/// This crate's governing rule is *"the namespace is always the RPC body's own `hClient`;
/// never a params field, never inferred"*, and the C artifact's promote handler is named
/// in the crate docs as **the** counter-example, because it reads `hChanClient` out of
/// `params+12` and never looks at the envelope at all
/// (`nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:2447`).
///
/// Per `ogkm`, reading `hChanClient` is **correct**. RM sets
/// `params.hChanClient = RES_GET_CLIENT_HANDLE(pChannelDescendant)` and then issues the
/// control with `RES_GET_CLIENT_HANDLE(pSubdevice)` as the envelope client
/// (`ogkm-580: src/nvidia/src/kernel/gpu/gr/kernel_graphics_object.c:130-135`); the two
/// are usually equal and are **not required** to be. So promote-ctx is not a violation of
/// the rule, it is a case the rule's phrasing does not cover, and the fix is to separate
/// the two jobs the phrase conflates:
///
/// | job | source | why |
/// |---|---|---|
/// | **namespace attribution** — which client is acting | the envelope's `hClient` | a params field naming a different client would be a silent cross-namespace substitution |
/// | **object resolution** — whose handle table `hObject` is in | `hChanClient` | the SDK documents `hObject` as living in `hChanClient`'s namespace, and RM populates it accordingly |
///
/// Both are carried out of here, which is the whole difference from the C: it kept only
/// the second, so it could not notice a disagreement between them, let alone refuse one.
/// The refusal itself is not this crate's — it needs the object model — and lives at
/// [`kayfabe_core::promote::PromoteFault::ForeignContextObject`].
///
/// ★ This is the third time a params field has legitimately named a client (`DUP_OBJECT`
/// was the second). The question is never *"envelope or params?"* but **"attribution or
/// resolution?"**; expect a fourth, and write its rule with the two jobs already
/// distinguished.
///
/// # ★ What is dropped
///
/// `ChID` (the SDK calls it deprecated), `engineType` (the address plane does not route
/// on it; the channel's engine is derived from the graph), and — since the legacy shape
/// is refused outright by the decoder — `hVirtMemory`/`virtAddress`/`size`.
fn translate_promote_ctx(
    abi: &DriverAbiTable,
    client: HClient,
    params: &[u8],
) -> Result<Translation, BridgeRefusal> {
    use kayfabe_abi::view::PromoteEntry;
    use kayfabe_core::promote::{CtxPromotion, PromoteDeclined, PromotedRange};

    let p = abi.decode_promote_ctx(params)?;
    let census = p.census();
    let mut ranges = Vec::new();
    for e in p.entries() {
        // ★ ONLY the complete state becomes a range. The other two are counted and
        // dropped — named, never silent (C defect D3): a promote-only entry's
        // `gpuPhysAddr == 0 && size == 0` is the *absence* of a fact, and binding
        // `va → phys 0` would be manufacturing an address, which is what MISS = FAULT
        // forbids.
        if let PromoteEntry::Promotable {
            va,
            len,
            phys,
            aperture,
            buffer_id,
        } = e
        {
            ranges.push(PromotedRange {
                va: kayfabe_arch::ids::GpuVa(va),
                len,
                phys,
                aperture,
                buffer_id,
            });
        }
    }
    Ok(Translation::CtxPromotion(CtxPromotion {
        client,
        chan_client: HClient(p.h_chan_client),
        object: HObject(p.h_object),
        ranges,
        declined: PromoteDeclined {
            initialize_only: census.initialize_only,
            promote_only: census.promote_only,
        },
    }))
}

/// `DUP_OBJECT` (fn 21) → [`RmEvent::Dup`].
///
/// `rpc_dup_object_v03_00` *is* `NVOS55_PARAMETERS_v03_00` — a bare struct with no
/// wrapper and no header of its own (`ogkm-610: g_rpc-structures.h:200-205`,
/// `ogkm-580: :198-203`), whose seven members are character-for-character identical in
/// both vendored trees (`ogkm-610: g_sdk-structures.h:368-377`, `ogkm-580: :366-375`). So the
/// existing ioctl-side decoder applies **verbatim**, which is what
/// `gsp_core_bridge.md` §1.4 claims and what
/// `crates/kayfabe-abi/tests/mean_wire.rs` pins.
///
/// ## ★★ The one place a params field legitimately names a namespace
///
/// The crate's governing rule is *"the namespace is always the RPC body's own `hClient`,
/// never a params field"*, and a `DUP_OBJECT` looks like the counter-example: `hClientSrc`
/// is a client handle sitting in the body, not in the envelope. It is not a
/// counter-example, and the distinction is the whole reason this event has two
/// [`NodeKey`]s:
///
/// - **attribution** — *which namespace is this message acting in* — is `hClient`, read
///   once, at the top, exactly as on every other verb. It becomes `dst.client` and
///   nothing may replace it.
/// - `hClientSrc` is a **cross-namespace reference**: a second, additional namespace the
///   message names, not a substitute for the first. `gsp_core_bridge.md` §2.5 says what to
///   do with one — *"a params field naming a different client … is a different fact and,
///   if we ever need it, needs its own event"* — and `RmEvent::Dup` **is** that event.
///   `DUP_OBJECT` is the only cross-client transfer edge in the RM object model
///   (`RmEvent::Dup`'s own doc), so the fact and the field are the same thing.
///
/// The C's `GPU_PROMOTE_CTX` handler is still the anti-pattern, and now visibly a
/// *different* one: it reads `hChanClient` from `params+12` and **never looks at the
/// envelope's `hClient` at all** (`nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:2283`)
/// — a substitution. Here both are carried, in the two slots the core has for them.
///
/// ## What is dropped, and why neither drop is this crate's to fix
///
/// - **`hParent`** — the destination alias's parent, a genuinely declared fact
///   (`ogkm-580: mem.c:1116` passes `pDstParentRef->hResource`, a real handle, unlike the
///   `FREE` path's always-zero `hObjectParent`). [`RmEvent::Dup`] does not take one:
///   `RmGraph` records a dup as a leaf `HandleRef::Alias`. Adding it is a **core** change,
///   not a bridge change — `gsp_core_bridge.md` §2.4 records the loss and declines it, and
///   this arm inherits the decision rather than re-taking it.
/// - **`flags`** — `NV04_DUP_HANDLE_FLAGS_REJECT_KERNEL_DUP_PRIVILEGE` = `0x1`
///   (`ogkm-610: nvos.h:2276-2277`, `ogkm-580: nvos.h:2275-2276`) is a *privilege* assertion
///   about the duping client, and the core models privilege as `ClientKind`, declared once
///   at the client root. Nowhere to put it, and nothing to do with it.
///
/// ## ★ Zero handles are carried VERBATIM here, and that is deliberate
///
/// A zero in `hObject`/`hObjectSrc` is **not** treated the way
/// [`BridgeRefusal::ImplicitVaspace`] treats a zero `hVASpace`, and the asymmetry is the
/// same one B3 already recorded between an alloc's *edge* fields and its *params* fields:
/// an edge field is the node the message creates or references, so the guest's zero is
/// the guest's own choice of key, landing where the guest put it. A params field naming
/// an object is a *reference to something else*, and NVIDIA documents the zero there as
/// meaning a different object entirely — which is a fact we do not have, hence the
/// refusal.
///
/// `[src]` RM's own reading of a zero destination handle is *"generate one"*
/// (`clientAssignResourceHandle` → `clientGenResourceHandle`,
/// `ogkm-580: src/nvidia/src/libraries/resserv/src/rs_client.c:998-1001`) — but that runs
/// on the **guest's own CPU-side RM**, at `serverCopyResource`
/// (`ogkm-580: rs_server.c:1725`), *before* the resource's copy-constructor issues the
/// RPC with the already-assigned `pDstRef->hResource`
/// (`ogkm-580: mem.c:1116` through `NV_RM_RPC_DUP_OBJECT`, `rpc.h:393-411`). So a zero
/// cannot reach this wire from a conforming guest at all, and the identical argument
/// applies to `GSP_RM_ALLOC`'s `hObject` (`rs_server.c:898`) — which this crate has
/// carried verbatim since B1. Refusing it on one verb and not the other would be a rule
/// with no principle behind it.
///
/// ## What this arm does NOT check, on purpose
///
/// It does not ask whether `src` exists, whether `dst` is free, or whether `hClientSrc`
/// names a live namespace. Those are lookups, and §3.4 gives all three to
/// `RmGraph::apply`, which answers them with a three-category taxonomy this crate has no
/// state to reproduce: an undeclared **namespace** is a FAULT
/// (`RmGraphError::UndeclaredClient`), an unobserved **source object** is a DEFER (the
/// edge parks — `[measured]` only 25 of 82 dups reach the GSP wire at all, so a source RM
/// saw and we did not is *ordinary*), and a dst handle that is already bound is
/// `RmGraphError::ConflictingDup` unless it is an identical re-send.
///
/// ★ That is why `hClientSrc == 0` is **not** refused here while `hClient == 0` is. The
/// envelope's client is this message's attribution and a message with no namespace is
/// malformed on its face, which needs no graph state. The source client is a reference,
/// and a reference into `HClient(0)` is refused by the rule that owns every namespace
/// question — `RmGraph::apply`'s central `RESERVED_CLIENT` gate, which enumerates *both*
/// of a dup's clients precisely so this arm does not have to. It arrives as
/// [`BridgeRefusal::Graph`] and is counted under `RmGraphError::ReservedClient`, which is
/// a strictly more informative tag than a second local copy would produce.
fn translate_dup(abi: &DriverAbiTable, payload: &[u8]) -> Result<Translation, BridgeRefusal> {
    let d = abi.decode_dup(payload)?;

    // ── The namespace, read ONCE, from the header. `NVOS55`'s `hClient` IS the envelope
    // field for this RPC — `rpc_dup_object_v03_00` has no envelope of its own beyond the
    // 32-byte message header, so the body's first word is the attribution.
    let client = HClient(d.dst_client);
    if client == RESERVED_CLIENT {
        return Err(BridgeRefusal::ReservedClient);
    }

    Ok(Translation::Event(RmEvent::Dup {
        src: NodeKey::new(HClient(d.src_client), HObject(d.src_handle)),
        dst: NodeKey::new(client, HObject(d.dst_handle)),
    }))
}

/// `FREE` (fn 10) → [`RmEvent::Free`].
///
/// `rpc_free_v03_00` *is* `NVOS00_PARAMETERS_v03_00` (`ogkm-610: g_rpc-structures.h:162-167`,
/// `ogkm-580: :160-165`),
/// filled by `rpcRmApiFree_GSP` as `hRoot = hClient`, `hObjectParent = NV01_NULL_OBJECT`,
/// `hObjectOld = hObject` (`ogkm-610: rpc.c:11147-11149`, `ogkm-580: :11339-11341`) — so
/// the existing ioctl-side decoder
/// applies verbatim.
///
/// `hObjectParent` is **discarded**: it is always zero on this path and
/// [`RmEvent::Free`] does not take one.
///
/// ★ And the free is emitted **flat** — this function does not ask "is this a client-root
/// free?". The C does, from the equality `fClient == fObj`
/// (`nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:1796`), and
/// `kayfabe_core::rmgraph::HandleRef`'s doc is a written warning against exactly that
/// test: free the origin handle while a dup keeps the resource alive, then dup it back,
/// and the alias becomes indistinguishable from the origin allocation — *"for a
/// `Client`-classed resource the mis-fire is catastrophic"*. The graph already recorded
/// the declaration; the bridge must not re-derive it.
fn translate_free(abi: &DriverAbiTable, payload: &[u8]) -> Result<Translation, BridgeRefusal> {
    let f = abi.decode_free(payload)?;
    let client = HClient(f.client);
    if client == RESERVED_CLIENT {
        return Err(BridgeRefusal::ReservedClient);
    }
    Ok(Translation::Event(RmEvent::Free {
        client,
        handle: HObject(f.handle),
    }))
}

// The concurrency contract, compile-time-asserted (decision #17).
kayfabe_util::assert_send_sync!(Translation, BridgeRefusal);
