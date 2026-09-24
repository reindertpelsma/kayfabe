//! Shared harness for the `rmrpc_*` bridge tests — ported from the old
//! `tests/tests/rmrpc_bridge.rs` harness section, with the old `Gpu` (a `Proc` spine over
//! isolates and address tables) replaced by [`Objs`]: the v3 object graph behind the
//! [`RmObjects`] seam, recording every page-directory statement it is handed.
//!
//! ⊘ The old "projection" oracle (`kayfabe_core::project::Boundaries`) is gone with the
//! projection. Its role — *"the same graph from wire bytes as from hand-written events"* — is
//! played by [`Snapshot`], which compares the graph itself: every live node (key, incarnation,
//! parent, KIND, facts — deliberately NOT the class id, for `Boundaries`' reason: two roots of
//! different root classes must compare equal), every dup edge, and every client declaration.
//!
//! Included by `#[path]`; each including file must also declare `mod rpcwire;` at its root.

#![allow(dead_code)]

use kf_abi::GuestOs;
use kf_abi::versions::{BENCH_DRIVER, DriverAbiTable, table_for};
use kf_arch::ClientKind;
use kf_arch::ids::{ClassId, HClient, HObject, Pdb};
use kf_gsp::RpcCommand;
use kf_rm::rmgraph::{AllocFacts, ClientKey, NodeKey, RmEvent, RmGraph, RmGraphError};
use kf_rm::rmrpc::{
    BridgeRefusal, GraphPolicy, ObjectsRefusal, PageDirStatement, RmObjects, Translation, translate,
};

use crate::rpcwire::{self as w, fn_id};

/// The bench driver's wire table.
pub fn abi() -> &'static DriverAbiTable {
    table_for(BENCH_DRIVER).expect("the bench driver is supported")
}

/// Turn a whole RPC **message** (envelope + body) into the decoded command the transport
/// would hand us: classify `function`, and take the payload as everything after the 32-byte
/// envelope.
///
/// # Panics
/// If the envelope is malformed.
pub fn command(msg: &[u8]) -> RpcCommand {
    let env = abi().decode_rpc_envelope(msg).expect("well-formed envelope");
    RpcCommand {
        function: kf_rm::abi::FUNCTIONS.classify(env.function),
        code: env.function,
        sequence: env.sequence,
        payload: abi().rpc_payload(msg).expect("payload").to_vec(),
        elements: 1,
        delivered: Vec::new(),
    }
}

/// `translate` over a whole message.
pub fn xlate(msg: &[u8]) -> Result<Translation, BridgeRefusal> {
    translate(abi(), GuestOs::Linux, &command(msg))
}

/// A `GSP_RM_ALLOC` message declaring a client root, built by the independent builder.
pub fn root_alloc_msg(class: u32, h_client: u32, process_id: u32) -> Vec<u8> {
    w::message(fn_id::GSP_RM_ALLOC, 1, &w::client_root_alloc_body(class, h_client, process_id))
}

/// A `FREE` message shaped the way `rpcRmApiFree_GSP` shapes one.
pub fn free_msg(h_client: u32, h_object: u32) -> Vec<u8> {
    w::message(fn_id::FREE, 2, &w::driver_free_body(h_client, h_object))
}

/// Handles for `SET_PAGE_DIRECTORY` — three handles in three different places.
pub mod spd {
    /// The client the control is issued in — the RPC body's `hClient`.
    pub const C: u32 = 0xc1d0_0071;
    /// The Device the control is issued **against** — the RPC body's `hObject`. Dropped:
    /// `PageDirStatement` has nowhere to put it.
    pub const DEV: u32 = 0x5c00_0001;
    /// The VASpace the page directory belongs to — a **params** field, not a header one.
    pub const VAS: u32 = 0x5c00_0010;
    /// The page-directory base.
    pub const PDB: u64 = 0x0000_0003_4100_0000;
}

/// A `GSP_RM_CONTROL`/`SET_PAGE_DIRECTORY` message, built by the independent builder.
pub fn set_page_dir_msg(h_client: u32, h_device: u32, h_vaspace: u32, pdb: u64, flags: u32) -> Vec<u8> {
    w::message(
        fn_id::GSP_RM_CONTROL,
        3,
        &w::control_body(
            h_client,
            h_device,
            w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
            32,
            w::RMAPI_RPC_FLAGS_NONE,
            &w::set_page_dir_params(pdb, 512, flags, h_vaspace, 0, 1, 0),
        ),
    )
}

/// The translation a `SET_PAGE_DIRECTORY` must produce — **written by hand**, never derived.
/// ⊘ v3: was `Translation::Event(RmEvent::SetPageDir { .. })`; the statement now goes to the
/// memory plane, not the graph.
pub fn expected_set_page_dir(client: u32, vaspace: u32, pdb: u64) -> Translation {
    Translation::PageDir(PageDirStatement {
        client: HClient(client),
        vaspace: HObject(vaspace),
        pdb: Pdb(pdb),
        // ⊘ The TEST default. The PRODUCTION path must never assume it.
        pdb_aperture: Some(kf_arch::Aperture::Vidmem),
    })
}

/// The event a client-root alloc of `(client, pid)` must produce — written by hand.
pub fn expected_root_event(client: u32, class: u32, kind: ClientKind) -> RmEvent {
    RmEvent::Alloc {
        client: HClient(client),
        // ★ The normalisation: the wire says `hParent = hObject = 0`, and RM's own rule
        // (`rs_server.c:625`, `hResource = hClient`) says the root's handle IS the client.
        parent: HObject(client),
        handle: HObject(client),
        class: ClassId(class),
        facts: AllocFacts { client_kind: Some(kind), ..Default::default() },
    }
}

/// ★ The v3 object model a test drives: the graph, plus a record of every page-directory
/// statement handed to [`RmObjects::page_dir`] (the memory plane's input, P4).
#[derive(Debug, Clone)]
pub struct Objs {
    /// The graph.
    pub graph: RmGraph,
    /// Every statement accepted, in order.
    pub page_dirs: Vec<PageDirStatement>,
    /// A snapshot of the graph after every event the seam applied — so a test can inspect
    /// the graph MID-run while a `GraphPolicy` still borrows it (the old `policy.gpu()`).
    pub history: Vec<Snapshot>,
}

impl Objs {
    /// Apply one event to the graph directly (the old `gpu.apply`).
    pub fn apply(&mut self, ev: RmEvent) -> Result<(), RmGraphError> {
        self.graph.apply(ev)
    }
}

impl RmObjects for Objs {
    fn apply(&mut self, ev: RmEvent, _params: &[u8]) -> Result<(), ObjectsRefusal> {
        self.graph.apply(ev).map_err(ObjectsRefusal::Graph)?;
        self.history.push(snapshot(&self.graph));
        Ok(())
    }

    /// ★ The namespace rule the old graph applied to `SetPageDir` (§12.38: no event may name a
    /// namespace that does not exist) now has to be applied by whoever receives the statement,
    /// because the graph no longer sees it. This test double applies it; a real memory plane
    /// must too.
    fn page_dir(&mut self, st: PageDirStatement) -> Result<(), ObjectsRefusal> {
        if !self.graph.client_declarations().keys().any(|k| k.client == st.client) {
            return Err(ObjectsRefusal::Graph(RmGraphError::UndeclaredClient(st.client)));
        }
        self.page_dirs.push(st);
        Ok(())
    }
}

/// A fresh object model on an Ampere host (the family the old `WireClassArch` spoke).
pub fn fresh_objects() -> Objs {
    Objs { graph: RmGraph::new(kf_chip::Family::Ampere), page_dirs: Vec::new(), history: Vec::new() }
}

/// ★ The graph as a comparable value — the v3 stand-in for the old projection oracle.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// `(key, incarnation, parent, kind, facts)` of every live resource, sorted by key.
    pub nodes: Vec<(NodeKey, u32, HObject, kf_arch::ObjectKind, AllocFacts)>,
    /// Every `(alias, origin)` dup edge, resolved and parked.
    pub dups: Vec<(NodeKey, NodeKey)>,
    /// Every live client declaration.
    pub decls: Vec<(ClientKey, ClientKind)>,
}

impl Snapshot {
    /// The live user-client namespaces.
    pub fn user_clients(&self) -> Vec<HClient> {
        self.decls
            .iter()
            .filter(|(_, k)| matches!(k, ClientKind::User { .. }))
            .map(|(k, _)| k.client)
            .collect()
    }

    /// The live kernel-client namespaces.
    pub fn kernel_clients(&self) -> Vec<HClient> {
        self.decls.iter().filter(|(_, k)| matches!(k, ClientKind::Kernel)).map(|(k, _)| k.client).collect()
    }
}

/// Snapshot a graph.
pub fn snapshot(g: &RmGraph) -> Snapshot {
    let mut nodes: Vec<_> = g.nodes().map(|n| (n.key, n.incarnation, n.parent, n.kind, n.facts)).collect();
    nodes.sort_by_key(|n| (n.0, n.1));
    Snapshot {
        nodes,
        dups: g.dups().collect(),
        decls: g.client_declarations().into_iter().map(|(k, (_, kind))| (k, kind)).collect(),
    }
}

/// Snapshot an [`Objs`].
pub fn boundaries(o: &Objs) -> Snapshot {
    snapshot(&o.graph)
}

/// Apply hand-written events to a fresh graph — the reference side of the oracle.
pub fn boundaries_of_events(events: &[RmEvent]) -> Snapshot {
    let mut o = fresh_objects();
    for ev in events {
        o.apply(*ev).expect("the reference scenario is legal");
    }
    boundaries(&o)
}

/// Translate and apply one message, returning whatever refused first.
pub fn drive(o: &mut Objs, msg: &[u8]) -> Result<(), BridgeRefusal> {
    match xlate(msg)? {
        Translation::Event(ev) => {
            o.apply(ev).expect("the graph accepts this fixture");
            Ok(())
        }
        Translation::PageDir(st) => {
            o.page_dirs.push(st);
            Ok(())
        }
        Translation::Inert => Ok(()),
        // ★ Unreachable, and asserted rather than swallowed: `translate` holds no state.
        Translation::Held => panic!("`translate` has no state and cannot hold a fragment"),
    }
}

/// Drive whole RPC **messages** through the policy, with no ring and no FSM.
pub fn deliver_all(policy: &mut GraphPolicy<'_>, msgs: &[Vec<u8>]) -> Vec<Result<Translation, BridgeRefusal>> {
    msgs.iter().map(|m| policy.deliver(&command(m))).collect()
}

/// An object model built by delivering a whole script through `GraphPolicy`, asserting it
/// refused nothing.
pub fn objects_from_script(script: &w::RpcScript) -> Objs {
    let mut o = fresh_objects();
    {
        let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut o);
        for (i, out) in deliver_all(&mut policy, &script.messages()).into_iter().enumerate() {
            let _ = out.unwrap_or_else(|e| panic!("message {i} of the script refused: {e:?}"));
        }
        assert!(policy.census().is_empty(), "a clean script refuses nothing");
    }
    o
}

/// An `RpcCommand` for a message **without** the envelope sanity panic: the payload is
/// taken as-is after 32 bytes (for fragment tests that build bodies directly).
pub fn raw_command(function: u32, sequence: u32, payload: Vec<u8>) -> RpcCommand {
    RpcCommand {
        function: kf_rm::abi::FUNCTIONS.classify(function),
        code: function,
        sequence,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

/// What one scripted run through the policy produced.
///
/// ⊘ v3 stand-in for the old `run_through_transport`, which booted a `gspworld::GspWorld`
/// (an independent re-implementation of the guest driver's msgq path, `tests/src/gspworld.rs`,
/// not carried into v3) and posted the script through the real command ring. Here each step
/// is handed straight to [`GraphPolicy`]'s `CommandPolicy::respond`, so `replies[i]` is the
/// reply the policy gave step `i` — the refusal wire format, without the transport.
pub struct Run {
    /// One reply per step, in order.
    pub replies: Vec<kf_gsp::Reply>,
    /// The refusal census.
    pub census: kf_rm::rmrpc::RefusalCensus,
    /// Events applied.
    pub applied: u64,
    /// Known-and-inert commands.
    pub inert: u64,
    /// Fragments held.
    pub held: u64,
}

/// Post `steps` through the policy's `respond`, in order, with `o` behind it.
pub fn run_through_policy(steps: &[w::Step], o: &mut Objs) -> Run {
    use kf_gsp::CommandPolicy;
    let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, o);
    let replies = steps
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let msg = w::message(s.function, 0x1000 + i as u32, &s.body);
            policy.respond(&command(&msg)).expect("GraphPolicy answers every command")
        })
        .collect();
    Run {
        replies,
        census: policy.census(),
        applied: policy.applied(),
        inert: policy.inert(),
        held: policy.held(),
    }
}

/// A hand-written event stream — the reference half of the wire-vs-events oracle (the old
/// `kayfabe_tests::Scenario`, reduced to what these tests use).
#[derive(Debug, Clone, Default)]
pub struct Scenario {
    /// The events, in script order.
    pub events: Vec<RmEvent>,
}

impl Scenario {
    /// Empty.
    pub fn new() -> Scenario {
        Scenario::default()
    }

    /// Append one event.
    pub fn push(&mut self, ev: RmEvent) -> &mut Scenario {
        self.events.push(ev);
        self
    }
}

/// ★ The class ids the hand-written REFERENCE side uses — real NVIDIA ids (v3 classifies
/// through `kf_chip`, no mock arch), deliberately DIFFERENT from the ones the byte side sends
/// wherever the family lists a second class of the same kind, so an agreement between the two
/// sides is an agreement about classification and facts, never about a class number travelling
/// twice. (Replaces the old `kayfabe_mocks::mock_classes`.)
pub mod ref_classes {
    use kf_arch::ids::ClassId;
    /// `NV01_ROOT_CLIENT` (the bytes mostly send `NV01_ROOT`).
    pub const CLIENT: ClassId = ClassId(0x41);
    /// `NV01_DEVICE_0`.
    pub const DEVICE: ClassId = ClassId(0x80);
    /// `FERMI_VASPACE_A`.
    pub const VASPACE: ClassId = ClassId(0x90f1);
    /// `KEPLER_CHANNEL_GROUP_A`.
    pub const TSG: ClassId = ClassId(0xa06c);
    /// `FERMI_CONTEXT_SHARE_A`.
    pub const CTXSHARE: ClassId = ClassId(0x9067);
    /// `TURING_CHANNEL_GPFIFO_A` — Ampere lists it; the bytes send `AMPERE_CHANNEL_GPFIFO_A`.
    pub const CHANNEL_GR: ClassId = ClassId(0xc46f);
    /// `AMPERE_COMPUTE_A` — the bytes send `AMPERE_COMPUTE_B`.
    pub const COMPUTE: ClassId = ClassId(0xc6c0);
    /// `AMPERE_DMA_COPY_A` — the bytes send `AMPERE_DMA_COPY_B`.
    pub const DMA_COPY: ClassId = ClassId(0xc6b5);
}

/// The `NVOS04_FLAGS_CHANNEL_USERD_INDEX_*` word a guest declares for virtual channel `chid`
/// — the old `MockArch::userd_flags_for`, whose field layout is the open driver's
/// (`VALUE 10:8`, `FIXED 11`, `PAGE_VALUE 20:12`, `PAGE_FIXED 21`). The graph stores it
/// opaquely (`AllocFacts::userd_flags`), so any word works; this one keeps the old fixtures'
/// bytes (and therefore the hand-written hex) identical.
pub fn userd_flags_for(chid: u16) -> u32 {
    let chid = u32::from(chid);
    let value = chid % 8;
    let page = (chid / 8) & 0x1FF;
    (1 << 21) | (page << 12) | (value << 8)
}

/// Old `kayfabe_tests::RING_ENTRIES`: one page of 16-byte entries.
pub const RING_ENTRIES: u32 = 4096 / 16;

/// Old `kayfabe_tests::ring_va_for`: the (biased) guest VA a fixture channel's ring is
/// declared at.
pub fn ring_va_for(chid: u16) -> u64 {
    0x80_0000_0000u64.wrapping_add(0x5100_0000 + (u64::from(chid) << 12))
}

/// Old `kayfabe_tests::ring_fact_for`: the declared ring fact.
pub fn ring_fact_for(chid: u16) -> kf_rm::rmgraph::GpFifoRing {
    kf_rm::rmgraph::GpFifoRing { va: ring_va_for(chid), entries: RING_ENTRIES }
}
