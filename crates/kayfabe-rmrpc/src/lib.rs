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
//! But a refusal is **not a drop**: the guest blocks in `_issueRpcAndWait` polling
//! `(function, sequence)` (`ogkm: src/nvidia/src/kernel/vgpu/rpc.c:9146-9170`), so an
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
//! - [`GraphPolicy`] — B2 ([`policy`]). The `kayfabe_gsp::CommandPolicy` the boot FSM
//!   calls for every command a guest posts: translate → `Gpu::apply` → a reply. It is the
//!   only thing here that holds a `&mut Gpu`, and it decodes nothing.
//!
//! The split is what keeps the rule above true while a stage that *must* touch state
//! lands: the applying half cannot grow a handle cache without going through
//! [`translate`], which has nowhere to put one.
//!
//! ## Scope of this stage (B1 + B2 + B3)
//!
//! `GSP_RM_ALLOC` and `FREE`. Everything else is a named refusal, including the functions
//! the design maps but whose arms are not built ([`BridgeRefusal::NotYetTranslated`] —
//! control/dup/continuation) and the classes with no entry in the class table
//! ([`BridgeRefusal::UnmappedAllocClass`]). Those are deliberately **not**
//! [`BridgeRefusal::UnknownFunction`]: "known and inert", "known and not yet built" and
//! "not known at all" are three different states, and collapsing them is how the C ended
//! up answering everything `NV_OK`.
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

mod policy;

pub use policy::{GraphPolicy, RefusalCensus};

use kayfabe_abi::versions::{AllocParams, DriverAbiTable};
use kayfabe_abi::wire::AbiError;
use kayfabe_abi::{NV_ERR_NOT_SUPPORTED, client_kind_from_process_id, rpc_params_are_serialized};
use kayfabe_arch::ids::{ClassId, HClient, HObject};
use kayfabe_core::gpu::GpuError;
use kayfabe_core::rmgraph::{AllocFacts, RESERVED_CLIENT, RmEvent};
use kayfabe_gsp::{RpcCommand, RpcFunction};
use kayfabe_trace::{FaultTag, Faulted};

/// What one RPC means to the object model.
///
/// Deliberately three-valued rather than `Option<RmEvent>`: "this RPC carries no
/// object-model content" is a *conclusion* about a known function, and it must not be
/// spelled the same way as "we could not translate it".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

/// Every way [`translate`] can refuse, by name.
///
/// Each variant carries the numbers a reader needs. There is no catch-all: an opaque
/// variant would force `is_err()` assertions, which `testing_doctrine.md` §2 forbids.
///
/// ★ **The enum only carries what this stage can produce.** `gsp_core_bridge.md` §4.1
/// sketches a wider set; the continuation-reassembly refusals are B6's and nothing here
/// can construct one, and a variant nothing can construct is a variant no test can bite.
/// [`Self::Graph`] was in that position at B1 and left the enum out for exactly that
/// reason; B2's [`GraphPolicy`] applies, so it exists now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeRefusal {
    /// An id the function table does not name at all. The **third state**: not inert, not
    /// staged, simply unrecognised.
    UnknownFunction {
        /// The raw wire id.
        code: u32,
    },
    /// A function the design maps to an `RmEvent` but whose arm is not built yet
    /// (`GSP_RM_CONTROL` → B4, `DUP_OBJECT` → B5, `CONTINUATION_RECORD` → B6).
    ///
    /// Distinct from [`Self::UnknownFunction`] on purpose: this one is a **staging**
    /// fact about our port, and the day it fires in a real boot it says which stage to
    /// build next rather than "the guest sent something strange".
    NotYetTranslated {
        /// The raw wire id.
        code: u32,
    },
    /// An **event** id arrived in the guest's *command* queue. `GSP_INIT_DONE` and
    /// `POST_EVENT` are things we send; a guest that posts one is not speaking the
    /// protocol.
    EventFromGuest {
        /// The raw wire id.
        code: u32,
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
    /// Fires on a *declared bit* (`ogkm: rpc.c:11018-11022`), never on a length
    /// heuristic. Which classes set it is `[unverified]`; if a boot-path class turns out
    /// to, this refusal is where that is discovered rather than a mis-decode.
    SerializedParams {
        /// `hClass`.
        class: u32,
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
    /// (RM stamps the second — `ogkm: src/nvidia/src/kernel/rmapi/client.c:226-227`), so
    /// a disagreement means we have mis-decoded, not that the guest meant something
    /// clever.
    ClientHandleDisagrees {
        /// The RPC header's `hClient` — the authoritative namespace.
        header: u32,
        /// The alloc params' `hClient`.
        params: u32,
    },
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
    /// [`GpuError`] is carried whole, and [`Faulted`] **delegates** to it, so the census
    /// records *which protocol rule* was broken (`RmGraphError::FreeUnknown` vs
    /// `::ConflictingAlloc` are different findings) rather than one flat "the graph said
    /// no" — the same argument `kayfabe_core`'s own `impl Faulted for GpuError` makes.
    Graph(GpuError),
}

impl From<AbiError> for BridgeRefusal {
    fn from(e: AbiError) -> Self {
        BridgeRefusal::Abi(e)
    }
}

impl BridgeRefusal {
    /// The `NV_STATUS` a reply carries when this refusal is what happened.
    ///
    /// ★ **One value, named, and knowingly provisional** (§4.2's `[open]`). It is
    /// `NV_ERR_NOT_SUPPORTED` for every variant, which is the status the C uses for the
    /// two controls it deliberately fails, and RM's reaction to it — setting
    /// `SKIP_COPYOUT`, so the guest leaves its own params buffer alone — is the behaviour
    /// that makes a *wrong* choice here a real bug rather than a cosmetic one. Picking a
    /// per-variant status needs an `NV_STATUS` table that does not exist yet; B4 revisits
    /// it. Until then the answer is one value with one citation, not a guess per arm.
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
            BridgeRefusal::NotYetTranslated { .. } => FaultTag("BridgeRefusal::NotYetTranslated"),
            BridgeRefusal::EventFromGuest { .. } => FaultTag("BridgeRefusal::EventFromGuest"),
            BridgeRefusal::UnmappedAllocClass { .. } => {
                FaultTag("BridgeRefusal::UnmappedAllocClass")
            }
            BridgeRefusal::SerializedParams { .. } => FaultTag("BridgeRefusal::SerializedParams"),
            BridgeRefusal::ParamsSizeExceedsPayload { .. } => {
                FaultTag("BridgeRefusal::ParamsSizeExceedsPayload")
            }
            BridgeRefusal::ClientHandleDisagrees { .. } => {
                FaultTag("BridgeRefusal::ClientHandleDisagrees")
            }
            BridgeRefusal::ReservedClient => FaultTag("BridgeRefusal::ReservedClient"),
            BridgeRefusal::Abi(_) => FaultTag("BridgeRefusal::Abi"),
            // ★ Delegated, not flattened: `kayfabe_core`'s own `impl Faulted for
            // GpuError` delegates for the same reason, so a graph refusal is countable
            // by the rule it broke. The refusal VALUE still says it came through the
            // bridge; the tag says what actually went wrong, which is what a census is
            // for.
            BridgeRefusal::Graph(e) => e.fault_tag(),
        }
    }
}

/// Translate one decoded GSP RPC into what it means to the object model.
///
/// A **pure function of one message**: no `&mut self`, no guest memory, no host state, no
/// lookup, no minted identity. See the crate docs for why each of those is load-bearing.
///
/// # Errors
///
/// [`BridgeRefusal`], by variant. A refusal is never a drop — the caller still owes the
/// guest a reply carrying [`BridgeRefusal::rpc_result`].
pub fn translate(abi: &DriverAbiTable, cmd: &RpcCommand) -> Result<Translation, BridgeRefusal> {
    match cmd.function {
        RpcFunction::RmAlloc => translate_alloc(abi, &cmd.payload),
        RpcFunction::Free => translate_free(abi, &cmd.payload),
        // Known and inert — three different reasons, collapsed here only because the
        // *answer* is the same. See `Translation::Inert`.
        RpcFunction::SetGuestSystemInfo
        | RpcFunction::GetGspStaticInfo
        | RpcFunction::UnloadingGuestDriver
        | RpcFunction::GspSetSystemInfo
        | RpcFunction::SetRegistry => Ok(Translation::Inert),
        // Known, mapped by the design, arm not built. Never `Inert`: the fact matters.
        RpcFunction::RmControl | RpcFunction::DupObject | RpcFunction::ContinuationRecord => {
            Err(BridgeRefusal::NotYetTranslated { code: cmd.code })
        }
        // Ours to send, never to receive.
        RpcFunction::InitDone | RpcFunction::PostEvent => {
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
/// NV01_NULL_OBJECT, NV01_ROOT, …)` (`ogkm: src/nvidia/inc/kernel/vgpu/rpc.h:85-87`) and
/// `rpcRmApiAlloc_GSP` copies all three through verbatim (`ogkm: rpc.c:11007-11009`).
/// The core, meanwhile, requires `parent == handle` for a root
/// (`kayfabe_core::rmgraph::RmEvent::Alloc`'s doc). Passing `0/0` through would create a
/// node at `(client, HObject(0))` whose relationship to the namespace is accidental.
///
/// The fix is **NVIDIA's own rule**, not an invention: in RM the `hClient` *is* its root
/// object's handle — `serverAllocClient` writes `pParams->hResource = hClient`
/// (`ogkm: src/nvidia/src/libraries/resserv/src/rs_server.c:625`).
///
/// It applies to the client root **and to nothing else**: every other class's `hParent`
/// and `hObject` are copied verbatim, which is why the normalisation cannot leak into a
/// class that did not ask for it.
fn translate_alloc(abi: &DriverAbiTable, payload: &[u8]) -> Result<Translation, BridgeRefusal> {
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
                // ★ The one genuinely guest-OS-shaped value in the whole bridge. The
                // `KERNEL_PID` sentinel branch that produces it is gated on
                // `RMCFG_FEATURE_PLATFORM_UNIX` (`ogkm: rpc.h:67-77`), so on a non-UNIX
                // guest a kernel-privileged client declares a *real* pid and this
                // classification would be wrong in the direction that folds a guest
                // process into the guest kernel's isolate. The seam is already the right
                // shape — one total function of one declared field — so the fix is a
                // **guest-OS profile** selected at realize beside the ABI table, never an
                // `if` here. Nothing else in this crate is OS-aware.
                // (`gsp_core_bridge.md` §1.5.)
                client_kind: Some(client_kind_from_process_id(facts.process_id)),
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

/// `FREE` (fn 10) → [`RmEvent::Free`].
///
/// `rpc_free_v03_00` *is* `NVOS00_PARAMETERS_v03_00` (`ogkm: g_rpc-structures.h:162-167`),
/// filled by `rpcRmApiFree_GSP` as `hRoot = hClient`, `hObjectParent = NV01_NULL_OBJECT`,
/// `hObjectOld = hObject` (`ogkm: rpc.c:11147-11149`) — so the existing ioctl-side decoder
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
