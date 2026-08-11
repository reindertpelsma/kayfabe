//! ★★★★★ **§16.80 — a `GSP_RM_ALLOC` of an engine class reaches the HOST.**
//!
//! # ⊘ The gap this file closes, stated as it was found
//!
//! `docs/design/gpu_promote_ctx.md` §16.79 REFUTED *"the host RM already builds the
//! context because we forward the `0xc7c0` alloc"* with: **we do not forward it.** The
//! Case-1 path was fully built — `route`/`plan`/`commit`/`exec`/`forward_engine_object`,
//! `DeviceShell::forward_engine_object`, `RmBackend::alloc_engine_object` — and had **zero
//! production callers**; every reference in the tree was a test.
//!
//! ★ And the tests were the reason nobody noticed: `tests/tests/engine_context.rs` pins the
//! forward thoroughly, and **every one of its cases calls `forward_engine_object` directly**.
//! A suite that only ever enters a subsystem through its own front door cannot tell
//! "correct" from "unreachable" — `admitted_and_served_are_different_gates`, and the
//! discriminator is always *who calls it in production*.
//!
//! ⇒ This file enters through the **wire**: real `GSP_RM_ALLOC` bytes, through the real
//! `ObjectPolicy`/`Bridge` chain the port installs, and asserts what the **host** saw.
//! Nothing here calls a `kayfabe_fwd::` function by name.
//!
//! ⊘ **What it does not claim.** It is not a boot: no guest, no hypervisor, no GPU, and a
//! `MockIsolateFactory` in place of a real isolate — so it says the call is *issued*, never
//! that a real driver accepts it (`only_live_boots_are_proof`).

#![allow(clippy::unusual_byte_groupings)]

use kayfabe_abi::GuestOs;
use kayfabe_abi::versions::{BENCH_DRIVER, DriverAbiTable, table_for};
use kayfabe_arch::ids::{ClassId, EngineKind, GpuId, HClient, HObject, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_fwd::{EngineParentMiss, FwdFault};
use kayfabe_gsp::RpcCommand;
use kayfabe_mocks::{MockArch, MockIsolateFactory, RmVerb, SharedRecorder, WireClassArch};
use kayfabe_rmrpc::ObjectPolicy;
use kayfabe_tests::gspworld::FUNCTIONS;
use kayfabe_tests::rpcwire::{self as w, RpcScript};

// ---------------------------------------------------------------------------------
// The compute subgraph, in wire bytes
// ---------------------------------------------------------------------------------

/// Handles, deliberately unrelated to their classes so a mix-up is visible.
mod h {
    pub const C: u32 = 0xc1d0_0071;
    pub const PID: u32 = 0x0000_ab21;
    pub const DEV: u32 = 0x5c00_0001;
    pub const VAS: u32 = 0x5c00_0010;
    pub const TSG: u32 = 0x5c00_0012;
    pub const CTXSHARE: u32 = 0x5c00_0013;
    /// The GR channel.
    pub const GR: u32 = 0x5c00_0019;
    /// The CE channel — **adjacent** to the GR handle, so a route that mixed the two up
    /// would still produce a plausible answer instead of an obvious one.
    pub const CE: u32 = 0x5c00_001a;
    /// `AMPERE_COMPUTE_B` on the GR channel.
    pub const GR_OBJ: u32 = 0x5c00_0020;
    /// `AMPERE_DMA_COPY_B` on the CE channel.
    pub const CE_OBJ: u32 = 0x5c00_0021;
    pub const DEVICE_INSTANCE: u32 = 0;
}

const GR_VCHID: VChid = VChid(0x21);
const CE_VCHID: VChid = VChid(0x22);
const PDB: u64 = 0x3401_000;

fn abi() -> &'static DriverAbiTable {
    table_for(BENCH_DRIVER).expect("the bench driver has a wire table")
}

fn gr_flags() -> u32 {
    MockArch::userd_flags_for(GR_VCHID)
}
fn ce_flags() -> u32 {
    MockArch::userd_flags_for(CE_VCHID)
}

/// The whole compute bring-up, in the order a guest sends it. ★ The engine objects come
/// **after** their channels, which is what makes the forward's "route through the channel
/// the alloc names" reachable at all.
fn script() -> RpcScript {
    let mut s = RpcScript::new();
    s.client_root(w::NV01_ROOT, h::C, h::PID)
        .device(h::C, h::C, h::DEV, h::DEVICE_INSTANCE)
        .vaspace(h::C, h::DEV, h::VAS)
        .set_page_dir(h::C, h::DEV, h::VAS, PDB, w::PDB_FLAGS_ALL_CHANNELS)
        .tsg(h::C, h::DEV, h::TSG, h::VAS)
        .ctxshare(h::C, h::VAS, h::CTXSHARE, h::VAS)
        .channel(h::C, h::TSG, h::GR, gr_flags(), 0, h::VAS)
        .channel(h::C, h::TSG, h::CE, ce_flags(), h::CTXSHARE, 0)
        .engine_object(h::C, h::GR, h::GR_OBJ, w::AMPERE_COMPUTE_B)
        .engine_object(h::C, h::CE, h::CE_OBJ, w::AMPERE_DMA_COPY_B);
    s
}

fn command(msg: &[u8]) -> RpcCommand {
    let env = abi()
        .decode_rpc_envelope(msg)
        .expect("well-formed envelope");
    RpcCommand {
        function: FUNCTIONS.classify(env.function),
        code: env.function,
        sequence: env.sequence,
        payload: abi().rpc_payload(msg).expect("payload").to_vec(),
        elements: 1,
        // ★ The whole message after the envelope, not the `length`-bounded view: an
        // alloc's `params[]` live past the declared length, and this fixture's whole
        // subject is those bytes.
        delivered: msg[kayfabe_abi::view::RpcEnvelope::SIZE..].to_vec(),
    }
}

/// A `Gpu` with NVIDIA's real class ids (so `0xc7c0` classifies as the driver's) and a
/// **recording** isolate factory (so the host side is observable).
fn wire_gpu() -> (Gpu, SharedRecorder) {
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x1000_0000_0000, 0x1_0000_0000);
    let gpu =
        Gpu::new(Box::new(WireClassArch::new()), Box::new(factory), gpa).expect("device realizes");
    (gpu, rec)
}

/// Drive the script through the policy the port installs, asserting every message was
/// accepted — a refusal here would make any host-side conclusion meaningless.
fn drive(script: &RpcScript) -> (ObjectPolicy, SharedRecorder) {
    let (gpu, rec) = wire_gpu();
    let mut policy = ObjectPolicy::new(
        abi(),
        GuestOs::Linux,
        gpu,
        kayfabe_device::ga10x::GA106_ENGINES,
    );
    for msg in script.messages() {
        let _ = policy
            .deliver(&command(&msg))
            .expect("every message in this script is conforming");
    }
    assert!(
        policy.census().is_empty(),
        "★ the fixture itself was refused, so nothing below is about the forward: {:?}",
        policy.census()
    );
    (policy, rec)
}

/// Every `AllocEngineObject` the host was asked for, as `(host channel, class, params)`.
fn host_engine_objects(rec: &SharedRecorder) -> Vec<(u64, ClassId, Vec<u8>)> {
    rec.lock()
        .expect("recorder")
        .log
        .iter()
        .filter_map(|(_iso, v)| match v {
            RmVerb::AllocEngineObject {
                chan,
                class,
                params,
                ..
            } => Some((chan.raw(), *class, params.clone())),
            _ => None,
        })
        .collect()
}

// =================================================================================
// The rung
// =================================================================================

/// ★★★★★ **THE RUNG.** A guest `GSP_RM_ALLOC` of `AMPERE_COMPUTE_B` causes a host
/// engine-object alloc. Before this wiring the host saw **nothing** for it.
#[test]
fn a_guest_alloc_of_ampere_compute_b_reaches_the_host() {
    let (_policy, rec) = drive(&script());
    let objs = host_engine_objects(&rec);
    let classes: Vec<u32> = objs.iter().map(|(_, c, _)| c.0).collect();
    assert_eq!(
        classes,
        vec![w::AMPERE_COMPUTE_B, w::AMPERE_DMA_COPY_B],
        "★ the host was asked for exactly the two engine classes the guest allocated, \
         in the order it allocated them"
    );
    // ★★ …and on TWO DIFFERENT host channels. The GR and CE guest handles are adjacent
    // (`0x…19` / `0x…1a`), so a route that resolved both to one channel would still have
    // produced two host allocs and passed the assertion above.
    assert_ne!(
        objs[0].0, objs[1].0,
        "★★ both engine objects landed on ONE host channel — the `hParent` hop collapsed \
         two guest channels into one: {objs:?}"
    );
}

/// ★★★ The guest's **own params bytes** are what the host is handed.
///
/// ⊘ This is the half that a wiring "good enough to boot" would have skipped. Every
/// engine class decodes to `AllocParams::NoDeclaredFacts`, whose arm in `translate_alloc`
/// is `AllocFacts::default()` — so `RmEvent::Alloc` carries **no bytes** and a forward
/// written off the event alone can only pass `&[]`. `kayfabe_rmrpc::alloc_params_window`
/// exists so it does not have to, and this is what proves it is wired.
#[test]
fn the_hosts_params_are_the_guests_own_bytes_not_an_empty_slice() {
    let (_policy, rec) = drive(&script());
    let objs = host_engine_objects(&rec);
    for (chan, class, params) in &objs {
        assert_eq!(
            params.len(),
            8,
            "★ the host got {} params bytes for class {:#06x} on host channel {chan:#x}, \
             and the guest's message declared 8. An empty slice here is the whole \
             difference between the HOST running the guest's alloc and us re-encoding it.",
            params.len(),
            class.0,
        );
    }
    assert_eq!(objs.len(), 2, "non-vacuity: the loop above ran");
}

/// ★★ The channel is materialized on the host **before** the object, and the object is
/// allocated ON it — the ordering `RmBackend` requires, asserted from the wire side.
#[test]
fn the_host_channel_is_materialized_before_the_object_that_needs_it() {
    let (_policy, rec) = drive(&script());
    let log = rec.lock().expect("recorder");
    let mut chans: Vec<u64> = Vec::new();
    for (_iso, v) in log.log.iter() {
        match v {
            RmVerb::AllocChannel { handle, .. } => chans.push(handle.raw()),
            RmVerb::AllocEngineObject { chan, .. } => assert!(
                chans.contains(&chan.raw()),
                "★ an engine object was allocated on host channel {:#x}, which had not \
                 been allocated yet. Channels so far: {chans:#x?}",
                chan.raw(),
            ),
            _ => {}
        }
    }
    assert_eq!(chans.len(), 2, "non-vacuity: two host channels were built");
}

/// ⊘ **The gate is the arch's own class table, and it is NOT a second list here.** Every
/// non-engine alloc in the script — client root, device, VASpace, TSG, ctxshare, the two
/// channels — passes through the same call and must produce **no** host engine object.
///
/// ★ This is what makes it safe to call the forward unconditionally on every `Alloc`: if
/// the gate leaked, a client root would be forwarded as an engine object.
#[test]
fn no_non_engine_alloc_reaches_the_host_as_an_engine_object() {
    let mut s = RpcScript::new();
    s.client_root(w::NV01_ROOT, h::C, h::PID)
        .device(h::C, h::C, h::DEV, h::DEVICE_INSTANCE)
        .vaspace(h::C, h::DEV, h::VAS)
        .set_page_dir(h::C, h::DEV, h::VAS, PDB, w::PDB_FLAGS_ALL_CHANNELS)
        .tsg(h::C, h::DEV, h::TSG, h::VAS)
        .ctxshare(h::C, h::VAS, h::CTXSHARE, h::VAS)
        .channel(h::C, h::TSG, h::GR, gr_flags(), 0, h::VAS);
    let (policy, rec) = drive(&s);
    assert_eq!(
        host_engine_objects(&rec),
        vec![],
        "★ a non-engine class was forwarded as an engine object"
    );
    // ★★★ …and it must be refused BY THE CLASS, not by something downstream that happens
    // to say no. `[measured 2026-08-10, boot `w221_49dc3ec_grfwd`]` this assertion did not
    // exist and the route delegated the class check to its LAST hop, so a client root came
    // back as `EngineObjectParent { … NotAChannel }` — a true sentence about a question the
    // class had already answered — and nineteen such lines ate a bounded diagnostic before
    // the one alloc it was built for arrived. ⊘ "No host verb was issued" is satisfied by
    // every wrong refusal as well as by the right one.
    let gpu = policy.gpu().expect("a bare Gpu model");
    for (class, what) in [
        (w::NV01_ROOT, "the client root"),
        (0x0080, "the device"),
        (w::FERMI_VASPACE_A, "the VA space"),
        (w::AMPERE_CHANNEL_GPFIFO_A, "the channel itself"),
    ] {
        assert_eq!(
            kayfabe_fwd::route_engine_object_by_parent(
                &gpu.spine,
                HClient(h::C),
                HObject(h::GR),
                ClassId(class),
            ),
            Err(FwdFault::NotAnEngine(ClassId(class))),
            "★ {what} ({class:#06x}) is refused by something other than its CLASS, even              though its parent here IS a channel — so the class gate is not the first hop"
        );
    }
    // ★ …and the host saw nothing AT ALL, which is the stronger statement: a channel is
    // materialized lazily, by the first engine object or doorbell that needs it, and
    // seven allocs that build one must not have materialized anything.
    assert!(
        rec.lock().expect("recorder").log.is_empty(),
        "★ the host was touched by a script containing no engine object: {:?}",
        rec.lock().expect("recorder").log,
    );
}

/// ★★★ **Idempotency across the WIRE**, not just across a direct call. A guest that
/// re-sends its engine-object alloc — which the protocol permits, and which a retrying
/// guest does — must not cause a second host object.
#[test]
fn a_replayed_engine_object_alloc_causes_exactly_one_host_object() {
    let mut s = script();
    s.engine_object(h::C, h::GR, h::GR_OBJ, w::AMPERE_COMPUTE_B)
        .engine_object(h::C, h::GR, h::GR_OBJ, w::AMPERE_COMPUTE_B);
    let (_policy, rec) = drive(&s);
    let compute: Vec<_> = host_engine_objects(&rec)
        .into_iter()
        .filter(|(_, c, _)| c.0 == w::AMPERE_COMPUTE_B)
        .collect();
    assert_eq!(
        compute.len(),
        1,
        "★ three identical guest allocs became {} host objects: {compute:?}",
        compute.len()
    );
}

// =================================================================================
// The route's own refusals — MISS = FAULT, each hop named
// =================================================================================

/// Every [`EngineParentMiss`] is reachable and distinguishable. ⊘ A single "unroutable"
/// would make "the guest named a handle that does not exist" and "this port has not
/// resolved the channel's GPU yet" — a hostile message and a deferral — one number.
#[test]
fn each_parent_miss_is_named_separately() {
    let (policy, _rec) = drive(&script());
    let gpu = policy.gpu().expect("a bare Gpu model");

    // A class the arch does not call an engine — checked FIRST, before the graph is
    // touched, which is what makes calling this on every alloc cheap.
    assert_eq!(
        kayfabe_fwd::route_engine_object_by_parent(
            &gpu.spine,
            HClient(h::C),
            HObject(h::GR),
            ClassId(w::FERMI_VASPACE_A),
        ),
        Err(FwdFault::NotAnEngine(ClassId(w::FERMI_VASPACE_A))),
    );
    // A parent handle nothing was ever allocated at.
    assert_eq!(
        kayfabe_fwd::route_engine_object_by_parent(
            &gpu.spine,
            HClient(h::C),
            HObject(0xdead_beef),
            ClassId(w::AMPERE_COMPUTE_B),
        ),
        Err(FwdFault::EngineObjectParent {
            client: HClient(h::C),
            object: HObject(0xdead_beef),
            why: EngineParentMiss::NoNode,
        }),
    );
    // A parent that exists and is not a channel. ⊘ The TSG is the interesting one: it is
    // the channel's own parent, so a route that walked UP would find a channel and
    // succeed — with a channel the guest did not name.
    assert_eq!(
        kayfabe_fwd::route_engine_object_by_parent(
            &gpu.spine,
            HClient(h::C),
            HObject(h::TSG),
            ClassId(w::AMPERE_COMPUTE_B),
        ),
        Err(FwdFault::EngineObjectParent {
            client: HClient(h::C),
            object: HObject(h::TSG),
            why: EngineParentMiss::NotAChannel,
        }),
    );
    // And the happy path resolves to the GR channel's own vChid — non-vacuity for the
    // three refusals above, and the proof the derivation agrees with `by_vchid`.
    let route = kayfabe_fwd::route_engine_object_by_parent(
        &gpu.spine,
        HClient(h::C),
        HObject(h::GR),
        ClassId(w::AMPERE_COMPUTE_B),
    )
    .expect("the GR channel routes");
    assert_eq!((route.gpu, route.vchid), (GpuId::ZERO, GR_VCHID));
    assert_eq!(route.engine, EngineKind::GrCompute);
    // ★ …and the CE channel routes to ITS vChid, off the same derivation — a
    // `vchid_from_userd_flags` that ignored its argument would pass the GR case alone.
    let ce = kayfabe_fwd::route_engine_object_by_parent(
        &gpu.spine,
        HClient(h::C),
        HObject(h::CE),
        ClassId(w::AMPERE_DMA_COPY_B),
    )
    .expect("the CE channel routes");
    assert_eq!((ce.gpu, ce.vchid), (GpuId::ZERO, CE_VCHID));
    assert_eq!(ce.engine, EngineKind::Ce);
}

// =================================================================================
// §16.109 — the guest's OWN engine declaration, carried instead of re-derived
// =================================================================================

/// The channel `vchid` projects to, in `gpu`'s live core state.
fn engine_of(gpu: &Gpu, vchid: VChid) -> EngineKind {
    let (pid, cid) = *gpu
        .spine
        .by_vchid
        .get(&(GpuId::ZERO, vchid))
        .expect("the channel routes");
    gpu.procs[&pid].channels[&cid].engine
}

/// ★★★★★ **A CE CHANNEL IS A CE CHANNEL BEFORE ITS ENGINE OBJECT EXISTS** — because the
/// guest said so, in the alloc params, and we now read the field it said it in.
///
/// # ⊘ What was actually missing, and it is not what the comment said
///
/// `kayfabe_chips::ga10x`'s `classify` calls the engine *"a **params** fact
/// `RmEvent::Alloc` has nowhere to carry"*, which reads as *"the value is in hand and has
/// no home"*. `[measured 2026-08-11]` it was never in hand: `engineType` sits at **+128**
/// (580) / **+136** (610) and `DriverAbiTable::decode_channel_alloc_facts` stops at
/// `CHANNEL_ALLOC_PREFIX` = **32**, because everything past +32 is the region the two
/// vendored trees disagree about. So the missing thing was a **version-keyed decode**
/// (`kayfabe_abi::notifier::ChannelEngineWire`), not a struct field; a field alone would
/// have carried nothing.
///
/// # What this asserts, and why the ABSENCE of engine objects is the whole point
///
/// The fixture is [`script`] minus its two `engine_object` allocs, plus a declared
/// `engineType` on each channel. With no engine object there is nothing to refine with, so
/// before this rung **both** channels projected as [`EngineKind::GrCompute`] — the
/// class-id guess, since one class serves both. The CE channel's arm is therefore RED
/// before and GREEN after, and the GR channel's arm is the known-positive that stops a
/// "declare everything CE" fix from passing.
#[test]
fn a_declared_engine_decides_the_channel_before_any_engine_object_arrives() {
    let mut s = RpcScript::new();
    s.client_root(w::NV01_ROOT, h::C, h::PID)
        .device(h::C, h::C, h::DEV, h::DEVICE_INSTANCE)
        .vaspace(h::C, h::DEV, h::VAS)
        .set_page_dir(h::C, h::DEV, h::VAS, PDB, w::PDB_FLAGS_ALL_CHANNELS)
        .tsg(h::C, h::DEV, h::TSG, h::VAS)
        .ctxshare(h::C, h::VAS, h::CTXSHARE, h::VAS)
        .channel_on_engine(
            h::C,
            h::TSG,
            h::GR,
            gr_flags(),
            0,
            h::VAS,
            kayfabe_abi::submit::ENGINE_TYPE_GRAPHICS,
        )
        // ★ COPY**2**, not COPY0: an instance the `engine_type_for` fallback does not
        // produce, so a decode that silently answered "the default copy engine" is
        // distinguishable from one that read the guest's number.
        .channel_on_engine(
            h::C,
            h::TSG,
            h::CE,
            ce_flags(),
            h::CTXSHARE,
            0,
            kayfabe_abi::submit::engine_type_copy(2).expect("COPY2"),
        );
    let (policy, _rec) = drive(&s);
    let gpu = policy.gpu().expect("a bare Gpu model");

    assert_eq!(
        engine_of(gpu, CE_VCHID),
        EngineKind::Ce,
        "★★★ the guest declared a copy engine and NO `AMPERE_DMA_COPY_B` exists yet — \
         before this rung the only available answer was the class-id guess `GrCompute`, \
         and the channel became a CE channel only when its engine object arrived"
    );
    assert_eq!(
        engine_of(gpu, GR_VCHID),
        EngineKind::GrCompute,
        "★ the known-positive: a declared GR channel is still GR. Without this arm a fix \
         that answered `Ce` for everything would pass the assertion above."
    );
}

/// ⊘ **The declaration does not overwrite the FINER fact.** `engineType` is
/// `NV2080_ENGINE_TYPE_GRAPHICS` for compute and 3D alike, so a declaration of GR is
/// under-determined and the engine object is the only evidence that separates them.
///
/// ★ This is the arm a naive "declaration always wins" gets wrong, and it would be
/// invisible: `kayfabe_chips::ga10x` records that every consumer treats the two GR variants
/// identically, so the regression would change a label nothing reads — until something did.
#[test]
fn a_declared_gr_channel_still_takes_its_refinement_from_the_engine_object() {
    let mut s = RpcScript::new();
    s.client_root(w::NV01_ROOT, h::C, h::PID)
        .device(h::C, h::C, h::DEV, h::DEVICE_INSTANCE)
        .vaspace(h::C, h::DEV, h::VAS)
        .set_page_dir(h::C, h::DEV, h::VAS, PDB, w::PDB_FLAGS_ALL_CHANNELS)
        .tsg(h::C, h::DEV, h::TSG, h::VAS)
        .ctxshare(h::C, h::VAS, h::CTXSHARE, h::VAS)
        .channel_on_engine(
            h::C,
            h::TSG,
            h::GR,
            gr_flags(),
            0,
            h::VAS,
            kayfabe_abi::submit::ENGINE_TYPE_GRAPHICS,
        )
        // The GA10x **3D** object, on the same engine as compute.
        .engine_object(h::C, h::GR, h::GR_OBJ, w::AMPERE_B);
    let (policy, _rec) = drive(&s);
    let gpu = policy.gpu().expect("a bare Gpu model");
    assert_eq!(
        engine_of(gpu, GR_VCHID),
        EngineKind::GrGraphics,
        "★ within GR the OBJECT decides: `engineType` cannot tell 3D from compute, so a \
         declaration that outranked the object here would replace a finer fact with a \
         coarser one and call it an improvement"
    );
}

/// ⊘ **A channel whose params stop at the agreed prefix declares nothing, and the
/// pre-existing derivation is unchanged for it.**
///
/// ★ This is the non-regression arm and it is why the decode may be additive at all: a
/// short params block is `Ok(None)` — *"we could not read it"* — never *"the guest said
/// GR"*, and `None` falls back to exactly the refinement-then-class-default order that
/// shipped before `ChannelEngineWire` existed. Every other test in this file uses
/// [`RpcScript::channel`], which emits 32 bytes, so this arm is what says their greens
/// still mean what they meant.
#[test]
fn a_channel_that_declares_no_engine_falls_back_to_the_refinement_unchanged() {
    let (policy, _rec) = drive(&script());
    let gpu = policy.gpu().expect("a bare Gpu model");
    assert_eq!(
        engine_of(gpu, CE_VCHID),
        EngineKind::Ce,
        "no declaration, but an `AMPERE_DMA_COPY_B` — the refinement still decides"
    );
    assert_eq!(engine_of(gpu, GR_VCHID), EngineKind::GrCompute);
}
