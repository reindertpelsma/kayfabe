//! **Stage B2** — the adapter that joins the two halves: a [`kf_gsp::CommandPolicy`]
//! whose answer to a command is *what the object model made of it*.
//!
//! [`translate`](super::translate) is a pure function and stays one. This module is the only
//! thing in [`super`] that **applies**, and it applies through ONE seam: [`RmObjects`]. Nothing
//! here decodes a byte.
//!
//! ## ⊘ v3: `ObjectModel` → [`RmObjects`]
//!
//! The old seam, `ObjectModel` (`kayfabe-rmrpc/src/policy.rs:616`), was implemented by the old
//! `Gpu` over its `Proc` spine, isolates and address tables, and carried eleven methods — the
//! promote join, the isolate census, channel scheduling, bind, ctxsw mode, engine-object
//! forwarding, control relays and a VAS census. v3 keeps the object-model half and names the
//! rest as later seams:
//!
//! - [`RmObjects::apply`] — the declared protocol fact. The implementor keeps the
//!   [`crate::rmgraph::RmGraph`] and makes host twins through `kf_host::HostRm` (`raw_alloc`,
//!   `raw_alloc_nested`, `free` — `V3_P2_PORT_MAP.md` §3). kf-rm makes no host call.
//! - [`RmObjects::page_dir`] — the guest's page-directory statement, for the memory plane (P4).
//! - The channel verbs (schedule, bind, preempt, ctxsw, engine objects, relays) are P5 and are
//!   not on this trait yet; their controls reach the unserviced ledger and are refused by name.
//!
//! [`GraphObjects`] is the host-free implementation: the graph alone. It is what the tests and
//! any register-only configuration install.
//!
//! ## ★ The three places a refusal surfaces
//!
//! `gsp_core_bridge.md` §4.2 requires all three:
//!
//! 1. **On the wire.** [`GraphPolicy::respond`] returns `Some(Reply)` with a non-zero
//!    `rpc_result` — never `None`, which is what the FSM turns into `cmd.ack(0)`, i.e. the C's
//!    affirmative echo. A refusal is **not** a drop: the guest is blocked in
//!    `_issueRpcAndWait` polling `(function, sequence)`, so an unanswered command hangs it for
//!    the whole RPC timeout.
//! 2. **Countably.** [`GraphPolicy::census`] — a per-[`FaultTag`] tally, so an invariant can
//!    be a *bound* ("zero refusals over a clean boot script") rather than an absence.
//! 3. **In the return value.** [`GraphPolicy::deliver`] is the `Result` form, and it is what a
//!    test asserts an exact variant against.
//!
//! ## ★ The state question
//!
//! The bridge mints nothing and remembers nothing: [`translate`](super::translate) is still a
//! free function of one message. What a policy here holds is the object model (the graph's
//! state, not the bridge's), bounded handle-free counters, and one [`Reassembler`] — none of
//! them keyed by an `hClient` or an `hObject`, so none can refuse, deduplicate or mis-attribute
//! a legal recycle.
//!
//! ⊘ Dropped with the old address table: `holds_for_refresh` driven by a VAS-table
//! fingerprint. v3's RPC-map hold is the memory plane's synchronization point, not a policy
//! heuristic.

use std::collections::{BTreeMap, BTreeSet};

use kf_abi::versions::DriverAbiTable;
use kf_abi::{DriverAbi, GuestOs};
use kf_gsp::{CommandPolicy, Reply, RpcCommand};
use kf_trace::FaultTag;

use super::{
    BridgeRefusal, Faulted, PageDirStatement, ReasmLimits, Reassembled, Reassembler, Translation,
    translate,
};
use crate::rmgraph::{RmEvent, RmGraph, RmGraphError};

/// How many refusals of each kind happened, by [`FaultTag`].
///
/// ★ **Bounded by construction, and that is why it may exist at all.** `fault_tag` is a
/// total function from a refusal to one of a *fixed, finite* set of `&'static str`s — the
/// [`BridgeRefusal`] variants, plus (through the delegating `Graph` arm) the
/// `RmGraphError`/`GpuError` variants. So this map cannot grow past that set however much
/// traffic a hostile guest sends, and it is keyed by **nothing the guest supplies**: no
/// handle, no client, no sequence number. A per-command log would be neither, and would
/// be a guest-reachable unbounded allocation of exactly the shape `GpuError::SpineCapacity`
/// exists to refuse.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefusalCensus {
    counts: BTreeMap<FaultTag, usize>,
    ids: BTreeMap<FaultTag, BTreeSet<u32>>,
}

/// ★★★★ **§16.56 — how many distinct ids are kept per tag.**
///
/// ⊘ A cap rather than a full set, and it is the guest-reachable-allocation rule this
/// module already states for the map itself: the tag set is closed and cannot grow with
/// traffic, but the `hClass` a guest sends is a **guest-supplied value** and an uncapped
/// set of them is an unbounded allocation a hostile guest drives directly.
///
/// ★ The cap is safe *because the count is not capped*: `RefusalCensus::of` still reports
/// every refusal, so a saturated id list can never read as a complete one — the report
/// prints `n` ids beside a larger count, which is a visible truncation rather than a silent
/// one (`a_saturated_instrument_looks_exactly_like_absence`).
pub const REFUSAL_DETAIL_CAP: usize = 8;

impl RefusalCensus {
    /// How many refusals carried `tag`.
    #[must_use]
    pub fn of(&self, tag: FaultTag) -> usize {
        self.counts.get(&tag).copied().unwrap_or(0)
    }

    /// Every tag seen, with its count, in tag order.
    pub fn tags(&self) -> impl Iterator<Item = (FaultTag, usize)> + '_ {
        self.counts.iter().map(|(&t, &n)| (t, n))
    }

    /// ★★★★ **§16.56 — the ids refused under `tag`**, ascending, at most
    /// [`REFUSAL_DETAIL_CAP`] of them. Empty for tags that are not about an id.
    ///
    /// ⊘ This is the answer to *"which class did we refuse?"*, a question no `grep` over
    /// any committed device log could answer before it existed — see
    /// [`crate::BridgeRefusal::fault_id`] for the measurement.
    pub fn ids(&self, tag: FaultTag) -> impl Iterator<Item = u32> + '_ {
        self.ids.get(&tag).into_iter().flatten().copied()
    }

    /// Total refusals, across every tag.
    #[must_use]
    pub fn total(&self) -> usize {
        self.counts.values().sum()
    }

    /// Nothing has been refused.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }
}

/// ★★★ **The refusal census as a handle the composition root can still read after it has
/// given the policy away** — the instrument the 2026-08-01 `alloc1` boot did without.
///
/// # Why this type exists, and what it costs to not have it
///
/// `[measured]` boot `alloc1` at rev `2ced035` (`docs/design/boot_measured_2026_08_01.md`
/// §6): every `GSP_RM_ALLOC` was refused `ParamsSizeExceedsPayload` **inside the bridge**,
/// and the only way anyone could tell was that `fn 103` was *missing* from the unserviced
/// ledger's six lines. A bridge refusal answers the command — with a non-zero
/// `rpc_result` — so it never reaches [`crate::policy`]'s terminal recorders, and the port
/// had no channel that said *"the bridge refused something"*. The diagnosis was
/// **by absence**, which is precisely what `kayfabe_device::unserviced::UnservicedLedger`
/// was built to abolish for the other half of the chain.
///
/// ⊘ The obstruction was ownership, not instrumentation. `ObjectPolicy` **owns** its
/// object model, is installed as a `Box<dyn CommandPolicy>`, and is therefore unreachable from
/// the composition root the moment it is boxed — so a census that lived only behind
/// `&self` could be read by a test and by nothing else. This handle is clonable and is
/// kept by the root, exactly as `UnservicedLog` is.
///
/// ★ **One store, not two.** [`Bridge`] records here and nowhere else, and
/// [`ObjectPolicy::census`] is a *snapshot* taken from this. A mirror kept beside the
/// original would be the "two lists that agree today" shape this repository has been
/// bitten by repeatedly; there is nothing here to drift from.
///
/// The bound argument in [`RefusalCensus`]'s docs is unweakened: the key is still a
/// [`FaultTag`] from a fixed finite set and still nothing the guest supplies.
#[derive(Debug, Clone, Default)]
pub struct SharedRefusalCensus(std::sync::Arc<std::sync::Mutex<RefusalCensus>>);

impl SharedRefusalCensus {
    /// A fresh, empty census.
    #[must_use]
    pub fn new() -> SharedRefusalCensus {
        SharedRefusalCensus::default()
    }

    /// A point-in-time copy — the value form every existing reader asserts against.
    #[must_use]
    pub fn snapshot(&self) -> RefusalCensus {
        let g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        g.clone()
    }

    fn record(&self, r: &BridgeRefusal) {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let tag = r.fault_tag();
        *g.counts.entry(tag).or_default() += 1;
        // ★★★★ §16.56 — the id beside the tag. ⊘ The COUNT above is incremented
        // unconditionally and the id set below is capped, so truncation can never subtract
        // from the census; it can only stop naming.
        if let Some(id) = r.fault_id() {
            let set = g.ids.entry(tag).or_default();
            if set.len() < REFUSAL_DETAIL_CAP {
                set.insert(id);
            }
        }
    }
}

/// ★★★ **Why the object model refused** — by name, never a flat "no".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectsRefusal {
    /// The graph refused the event (a protocol rule, named by the variant).
    Graph(RmGraphError),
    /// The host refused to make the twin object. `what` names the verb; the detail is the
    /// implementor's to log.
    Host {
        /// The host verb that refused.
        what: &'static str,
    },
    /// A fact this object model does not model yet, named — e.g. a page-directory statement
    /// before the memory plane exists (P4).
    NotModelled {
        /// What is not modelled.
        what: &'static str,
    },
}

impl Faulted for ObjectsRefusal {
    fn fault_tag(&self) -> FaultTag {
        match self {
            // ★ Delegated: which protocol rule broke is the finding.
            ObjectsRefusal::Graph(e) => e.fault_tag(),
            ObjectsRefusal::Host { .. } => FaultTag("ObjectsRefusal::Host"),
            ObjectsRefusal::NotModelled { .. } => FaultTag("ObjectsRefusal::NotModelled"),
        }
    }
}

/// ★★★ **The object model, as the seam a later crate implements.** Narrow on purpose: see
/// this module's header for what the old eleven-method `ObjectModel` became.
pub trait RmObjects: Send {
    /// Explicit checked RAM/FB registration seam. Default implementations refuse.
    fn memory_list(
        &mut self,
        _request: kf_abi::memory_list::Declaration,
    ) -> Result<RmEvent, ObjectsRefusal> {
        Err(ObjectsRefusal::NotModelled {
            what: "memory-list registration unavailable",
        })
    }

    /// Explicit diagnostic seam; object implementations refuse unless they support
    /// the bounded graph-only metadata operation. Never delegates to a host alloc.
    fn software_runlist_probe(
        &mut self,
        _request: crate::sw_runlist_probe::Declaration,
    ) -> Result<RmEvent, ObjectsRefusal> {
        Err(ObjectsRefusal::NotModelled {
            what: "software-runlist allocation diagnostic unavailable",
        })
    }
    /// Apply one declared protocol fact. `params` is the alloc's own params window (empty for
    /// every other event), so an implementor making a host twin has the guest's declaration
    /// without re-decoding the wire.
    ///
    /// # Errors
    /// [`ObjectsRefusal`], by name.
    fn apply(&mut self, ev: RmEvent, params: &[u8]) -> Result<(), ObjectsRefusal>;

    /// The guest's page-directory statement for one VA space — the memory plane's input (P4).
    ///
    /// # Errors
    /// [`ObjectsRefusal`], by name.
    fn page_dir(&mut self, st: PageDirStatement) -> Result<(), ObjectsRefusal>;

    /// ★ OWNER_RULINGS §U: a class-5080 registration control (`ctl`) on `(client, object)` —
    /// the status the GSP answers, or `None` when this seat holds no deferred-API tables (the
    /// control then falls through and is refused as before). Default: no tables.
    fn deferred_api(
        &mut self,
        _abi: &kf_abi::defapi::DefApiAbi,
        _client: u32,
        _object: u32,
        _ctl: kf_abi::defapi::Control,
        _params: &[u8],
    ) -> Option<u32> {
        None
    }
}

/// ★ The host-free [`RmObjects`]: the object graph alone.
///
/// `page_dir` refuses [`ObjectsRefusal::NotModelled`] — the statement's consumer is the memory
/// plane, which this configuration does not have, and accepting it silently would be the
/// "recording is not forwarding" shape the old tree measured (`control 0x90f10106 result
/// 0x00000000 x4`, nothing downstream).
#[derive(Debug, Clone)]
pub struct GraphObjects {
    /// The graph.
    pub graph: RmGraph,
    ram: Option<std::sync::Arc<dyn crate::memory_list::GuestRamAuthority>>,
    fb: Option<kf_chip::bar0::FbLayout>,
    /// ★ OWNER_RULINGS §U: the VM's deferred-API tables (shared with the channel plane, which
    /// looks entries up at the trigger). `None`: 5080 controls fall through, refused as before.
    deferred: Option<std::sync::Arc<crate::defapi::Registry>>,
}

impl GraphObjects {
    /// Attach trusted live QEMU guest-RAM authority; no host GPU allocation.
    #[must_use]
    pub fn with_guest_ram(
        mut self,
        ram: std::sync::Arc<dyn crate::memory_list::GuestRamAuthority>,
    ) -> Self {
        self.ram = Some(ram);
        self
    }
    /// Attach this VM's immutable framebuffer layout; only usable store spans may register.
    #[must_use]
    pub fn with_guest_framebuffer(mut self, layout: kf_chip::bar0::FbLayout) -> Self {
        self.fb = Some(layout);
        self
    }
    /// Bounded future-consumer read, requiring a live graph object and RAM generation.
    pub fn read_memory_list(
        &self,
        key: crate::rmgraph::NodeKey,
        offset: u64,
        bytes: &mut [u8],
    ) -> bool {
        crate::memory_list::read(&self.graph, self.ram.as_ref(), key, offset, bytes)
    }
    /// Bounded future-consumer write with the same lifetime/authority checks.
    pub fn write_memory_list(
        &self,
        key: crate::rmgraph::NodeKey,
        offset: u64,
        bytes: &[u8],
    ) -> bool {
        crate::memory_list::write(&self.graph, self.ram.as_ref(), key, offset, bytes)
    }

    /// A fresh graph for `family`.
    #[must_use]
    pub fn new(family: kf_chip::Family) -> GraphObjects {
        GraphObjects {
            graph: RmGraph::new(family),
            ram: None,
            fb: None,
            deferred: None,
        }
    }

    /// ★ OWNER_RULINGS §U: serve the class-5080 registration controls into `registry` (the
    /// device's one registry, shared with the channel plane).
    #[must_use]
    pub fn with_deferred_api(mut self, registry: std::sync::Arc<crate::defapi::Registry>) -> Self {
        self.deferred = Some(registry);
        self
    }

    /// The graph half of [`RmObjects::apply`].
    fn apply_graph(&mut self, ev: RmEvent, _params: &[u8]) -> Result<(), ObjectsRefusal> {
        if let RmEvent::Alloc {
            client,
            parent,
            facts,
            ..
        } = ev
            && facts.deferred_api_notify.is_some()
        {
            let key = crate::rmgraph::NodeKey::new(client, parent);
            if !self
                .graph
                .allocated_node(key)
                .is_some_and(|n| matches!(n.kind, kf_arch::ObjectKind::Channel { .. }))
            {
                return Err(ObjectsRefusal::NotModelled {
                    what: "deferred API constructor: parent is not an original live channel",
                });
            }
        }
        self.graph.apply(ev).map_err(ObjectsRefusal::Graph)
    }

    /// `key` is a live `NV50_DEFERRED_API_CLASS` object whose parent is a live channel.
    fn is_deferred_api_object(&self, key: crate::rmgraph::NodeKey) -> bool {
        self.graph.allocated_node(key).is_some_and(|n| {
            n.class.0 == kf_abi::generated::classes::NV50_DEFERRED_API_CLASS
                && self
                    .graph
                    .allocated_node(crate::rmgraph::NodeKey::new(key.client, n.parent))
                    .is_some_and(|p| matches!(p.kind, kf_arch::ObjectKind::Channel { .. }))
        })
    }
}

impl RmObjects for GraphObjects {
    fn memory_list(
        &mut self,
        request: kf_abi::memory_list::Declaration,
    ) -> Result<RmEvent, ObjectsRefusal> {
        crate::memory_list::allocate(
            &mut self.graph,
            self.ram.as_ref(),
            self.fb.as_ref(),
            request,
        )
    }

    fn software_runlist_probe(
        &mut self,
        request: crate::sw_runlist_probe::Declaration,
    ) -> Result<RmEvent, ObjectsRefusal> {
        crate::sw_runlist_probe::allocate(&mut self.graph, request)
    }
    fn deferred_api(
        &mut self,
        abi: &kf_abi::defapi::DefApiAbi,
        client: u32,
        object: u32,
        ctl: kf_abi::defapi::Control,
        params: &[u8],
    ) -> Option<u32> {
        let reg = self.deferred.clone()?;
        let key = crate::rmgraph::NodeKey::new(
            kf_arch::ids::HClient(client),
            kf_arch::ids::HObject(object),
        );
        // A control on anything but a live 5080 object: the handle names no such object
        // (`resControlLookup` never reaches the 5080 body).
        if !self.is_deferred_api_object(key) {
            return Some(kf_abi::defapi::NV_ERR_INVALID_OBJECT_HANDLE);
        }
        let graph = &self.graph;
        Some(crate::defapi::serve(
            &reg,
            abi,
            crate::defapi::ObjKey { client, object },
            ctl,
            params,
            |h| {
                graph
                    .node(crate::rmgraph::NodeKey::new(
                        kf_arch::ids::HClient(client),
                        kf_arch::ids::HObject(h),
                    ))
                    .is_some()
            },
        ))
    }

    fn apply(&mut self, ev: RmEvent, params: &[u8]) -> Result<(), ObjectsRefusal> {
        let freed = matches!(ev, RmEvent::Free { .. });
        let r = self.apply_graph(ev, params);
        if freed && let Some(reg) = self.deferred.clone() {
            // ★ `defapiDestruct_IMPL`: a 5080 object freed (itself, its channel, its client)
            // takes its whole tree with it.
            reg.prune(|k| {
                self.is_deferred_api_object(crate::rmgraph::NodeKey::new(
                    kf_arch::ids::HClient(k.client),
                    kf_arch::ids::HObject(k.object),
                ))
            });
        }
        r
    }

    fn page_dir(&mut self, _st: PageDirStatement) -> Result<(), ObjectsRefusal> {
        Err(ObjectsRefusal::NotModelled {
            what: "page-directory statement: the memory plane is P4",
        })
    }
}

/// The shared half of [`GraphPolicy`] and [`ObjectPolicy`]: reassemble → translate → apply.
struct Bridge {
    memory_list_probe: bool,
    sw_runlist_probe: Option<crate::sw_runlist_probe::Probe>,
    abi: DriverAbiTable,
    guest_os: GuestOs,
    reasm: Reassembler,
    census: SharedRefusalCensus,
    applied: u64,
    inert: u64,
    held: u64,
    page_dirs: u64,
    /// Called only after a complete, translated object event has been accepted. A held
    /// fragment's NV_OK is a transport acknowledgement, not an accepted allocation.
    accepted: Option<Box<dyn FnMut(&RpcCommand) + Send>>,
}

impl Bridge {
    fn new(abi: DriverAbiTable, guest_os: GuestOs, limits: ReasmLimits) -> Bridge {
        Bridge {
            sw_runlist_probe: None,
            memory_list_probe: false,
            abi,
            guest_os,
            reasm: Reassembler::with_limits(limits),
            census: SharedRefusalCensus::default(),
            applied: 0,
            inert: 0,
            held: 0,
            page_dirs: 0,
            accepted: None,
        }
    }

    fn deliver(
        &mut self,
        objects: &mut dyn RmObjects,
        cmd: &RpcCommand,
    ) -> Result<Translation, BridgeRefusal> {
        let abi = &self.abi;
        let guest_os = self.guest_os;
        let outcome = self.reasm.accept(abi, cmd).and_then(|r| {
            let whole: &RpcCommand = match &r {
                Reassembled::Whole => cmd,
                Reassembled::Held => return Ok(Translation::Held),
                Reassembled::Complete(full) => full,
            };
            let probe_request = self.sw_runlist_probe.as_ref().and_then(|p| p.decode(whole));
            let memory_cell = self
                .memory_list_probe
                .then(|| kf_abi::memory_list::cell(abi.driver_version()))
                .flatten();
            let is_memory = memory_cell
                .is_some_and(|c| whole.function == kf_gsp::RpcFunction::Other(c.function()));
            let is_probe = probe_request.is_some() || is_memory;
            let t = if is_memory {
                let request = memory_cell.and_then(|c| c.decode(&whole.payload)).ok_or(
                    BridgeRefusal::Objects(ObjectsRefusal::NotModelled {
                        what: "memory-list: unsupported or malformed descriptor",
                    }),
                )?;
                Translation::Event(
                    objects
                        .memory_list(request)
                        .map_err(BridgeRefusal::Objects)?,
                )
            } else if let Some(request) = probe_request {
                Translation::Event(
                    objects
                        .software_runlist_probe(request?)
                        .map_err(BridgeRefusal::Objects)?,
                )
            } else {
                translate(abi, guest_os, whole)?
            };
            match t {
                Translation::Event(ev) => {
                    let params = if matches!(ev, RmEvent::Alloc { .. }) {
                        super::alloc_params_window(abi, whole.wire_body()).unwrap_or(&[])
                    } else {
                        &[]
                    };
                    if !is_probe {
                        objects.apply(ev, params).map_err(BridgeRefusal::Objects)?;
                    }
                    if let Some(accepted) = &mut self.accepted {
                        accepted(whole);
                    }
                }
                Translation::PageDir(st) => objects.page_dir(st).map_err(BridgeRefusal::Objects)?,
                Translation::Inert | Translation::Held => {}
            }
            Ok(t)
        });
        match &outcome {
            Ok(Translation::Event(_)) => self.applied = self.applied.saturating_add(1),
            Ok(Translation::PageDir(_)) => self.page_dirs = self.page_dirs.saturating_add(1),
            Ok(Translation::Inert) => self.inert = self.inert.saturating_add(1),
            Ok(Translation::Held) => self.held = self.held.saturating_add(1),
            Err(r) => self.census.record(r),
        }
        outcome
    }

    fn respond(&mut self, objects: &mut dyn RmObjects, cmd: &RpcCommand) -> Reply {
        match self.deliver(objects, cmd) {
            Ok(_) => Reply {
                rpc_result: 0, // NV_OK
                body: cmd.payload.clone(),
            },
            Err(r) => refusal_reply(r),
        }
    }

    fn debug_as(&self, name: &'static str, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct(name)
            .field("driver", &self.abi.version())
            .field("applied", &self.applied)
            .field("page_dirs", &self.page_dirs)
            .field("inert", &self.inert)
            .field("held", &self.held)
            .field("in_flight", &self.reasm.in_flight())
            .field("census", &self.census)
            .finish_non_exhaustive()
    }
}

/// ★★ **B2 — the policy that answers EVERY command from the object model.** The C
/// differential's shape: it *is* the policy, so it never declines.
///
/// ⚠ Never install it inside the served chain: it would silence the unserviced ledger (use
/// [`ObjectPolicy`] there). It borrows its object model, so a test can inspect the graph after.
pub struct GraphPolicy<'a> {
    bridge: Bridge,
    objects: &'a mut dyn RmObjects,
}

impl<'a> GraphPolicy<'a> {
    /// Build a policy over `objects` for one guest driver and guest OS.
    #[must_use]
    pub fn new(
        abi: &DriverAbiTable,
        guest_os: GuestOs,
        objects: &'a mut dyn RmObjects,
    ) -> GraphPolicy<'a> {
        GraphPolicy::with_limits(abi, guest_os, objects, ReasmLimits::default())
    }

    /// [`Self::new`] with explicit reassembly bounds.
    #[must_use]
    pub fn with_limits(
        abi: &DriverAbiTable,
        guest_os: GuestOs,
        objects: &'a mut dyn RmObjects,
        limits: ReasmLimits,
    ) -> GraphPolicy<'a> {
        GraphPolicy {
            bridge: Bridge::new(*abi, guest_os, limits),
            objects,
        }
    }

    /// The reassembler, for tests that assert on in-flight state.
    #[must_use]
    pub fn reassembler(&self) -> &Reassembler {
        &self.bridge.reasm
    }

    /// A snapshot of the refusal census.
    #[must_use]
    pub fn census(&self) -> RefusalCensus {
        self.bridge.census.snapshot()
    }

    /// The shared census handle.
    #[must_use]
    pub fn refusal_census(&self) -> SharedRefusalCensus {
        self.bridge.census.clone()
    }

    /// Events applied.
    #[must_use]
    pub fn applied(&self) -> u64 {
        self.bridge.applied
    }

    /// Page-directory statements handed to the object model.
    #[must_use]
    pub fn page_dirs(&self) -> u64 {
        self.bridge.page_dirs
    }

    /// Known-and-inert commands.
    #[must_use]
    pub fn inert(&self) -> u64 {
        self.bridge.inert
    }

    /// Continuation fragments held.
    #[must_use]
    pub fn held(&self) -> u64 {
        self.bridge.held
    }

    /// The `Result` form of [`CommandPolicy::respond`] — what a test asserts a variant against.
    ///
    /// # Errors
    /// The [`BridgeRefusal`], by name.
    pub fn deliver(&mut self, cmd: &RpcCommand) -> Result<Translation, BridgeRefusal> {
        self.bridge.deliver(self.objects, cmd)
    }
}

impl core::fmt::Debug for GraphPolicy<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.bridge.debug_as("GraphPolicy", f)
    }
}

impl CommandPolicy for GraphPolicy<'_> {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        Some(self.bridge.respond(self.objects, cmd))
    }
}

static REFUSALS_LOGGED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many refusals are named on stderr before the census carries the rest.
const REFUSAL_LOG_MAX: u64 = 24;

/// ★ **The refusal wire format**: `rpc_result = NV_ERR_NOT_SUPPORTED`, empty body — and one
/// line the first [`REFUSAL_LOG_MAX`] times, because the guest sees only the status and this
/// line is the only place the REASON exists. Shared by every site so they cannot drift.
fn refusal_reply(r: BridgeRefusal) -> Reply {
    let n = REFUSALS_LOGGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    if n <= REFUSAL_LOG_MAX {
        eprintln!(
            "kf-rm: RPC-REFUSED #{n} {r:?} \u{21d2} answered NV_ERR_NOT_SUPPORTED (0x56). \u{2298} The \
             guest sees only the status; this line is the only place the REASON exists. \
             (printing the first {REFUSAL_LOG_MAX}; the total rides the refusal census)"
        );
    }
    Reply {
        rpc_result: r.rpc_result(),
        body: Vec::new(),
    }
}

/// ★★★ The **object-declaring verbs**, as a chain link the served chain can hold
/// ([`crate::ObjectLinks::objects`]).
///
/// [`GraphPolicy`] claims **every** command, which would make the unserviced ledger — the one
/// instrument that can say *"what has this port not built yet"* — permanently empty. This one
/// claims a **declared, closed set of RPC functions** ([`OBJECT_VERBS`]) and returns `None` for
/// everything else, byte for byte leaving every other link exactly as it was.
/// The explicit default-off MemoryList experiment adds only its audited fn4 cell.
///
/// ⊘ It does NOT claim `GSP_RM_CONTROL`: that would take it away from
/// [`crate::inittables::InitTablePolicy`]. The old `OBJECT_CONTROLS` (schedule, bind, preempt,
/// ctxsw mode, MC service, promote, relays) are the channel plane — P5.
///
/// Owns its object model because a chain link is `Box<dyn CommandPolicy>` (`'static`).
pub struct ObjectPolicy {
    bridge: Bridge,
    objects: Box<dyn RmObjects>,
    /// ★ OWNER_RULINGS §U: the deferred-API surface at the served version (`None` where the
    /// matrix does not measure it — its controls then fall through, refused as before).
    defapi: Option<kf_abi::defapi::DefApiAbi>,
    /// 5080 controls served (for the bounded log).
    defapi_served: u64,
}

/// The RPC functions [`ObjectPolicy`] claims. **Closed, and public, so a test can quantify
/// over it rather than restate it.**
///
/// ★ `DUP_OBJECT` (fn 21) is here on a MEASUREMENT (old §16.38): a boot refused it by name
/// when it was absent, and UVM's `DUP_OBJECT` of the compute client's VA space is the normal
/// flow.
pub const OBJECT_VERBS: &[kf_gsp::RpcFunction] = &[
    kf_gsp::RpcFunction::RmAlloc,
    kf_gsp::RpcFunction::Free,
    kf_gsp::RpcFunction::DupObject,
];

impl ObjectPolicy {
    pub(crate) fn with_memory_list_probe(mut self) -> Self {
        self.bridge.memory_list_probe = true;
        self
    }

    pub(crate) fn with_sw_runlist_probe(mut self, probe: crate::sw_runlist_probe::Probe) -> Self {
        self.bridge.sw_runlist_probe = Some(probe);
        self
    }
    /// Attach the display registry at the acceptance boundary, after reassembly and
    /// `RmObjects::apply`, never at a speculative/held RPC reply. This observer cannot
    /// answer or change an object verdict.
    pub(crate) fn with_accepted_observer(
        mut self,
        observer: impl FnMut(&RpcCommand) + Send + 'static,
    ) -> Self {
        self.bridge.accepted = Some(Box::new(observer));
        self
    }

    /// Build the link over `objects`.
    #[must_use]
    pub fn over(
        abi: &DriverAbiTable,
        guest_os: GuestOs,
        objects: Box<dyn RmObjects>,
        limits: ReasmLimits,
    ) -> ObjectPolicy {
        ObjectPolicy {
            bridge: Bridge::new(*abi, guest_os, limits),
            objects,
            defapi: kf_abi::defapi::DefApiAbi::at(abi.driver_version()),
            defapi_served: 0,
        }
    }

    /// ★ OWNER_RULINGS §U — a class-5080 registration control, served from the object graph's
    /// tables (`crate::defapi`): `NV_OK` with the params echoed (what the real GSP answered
    /// `[measured: the 2026-10-05 VFIO boots 8/9/10]`), or the real body's status. `None`: not a 5080 control this seat
    /// serves (no tables, not permitted at this boundary) — it falls through as before.
    fn deferred_api_control(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        let abi = self.defapi?;
        let h = self.bridge.abi.decode_rpc_control(&cmd.payload).ok()?;
        let ctl = abi.control(h.cmd)?;
        if !self
            .bridge
            .abi
            .capabilities()
            .control(kf_arch::ids::ControlCmd(h.cmd))
            .is_permitted()
        {
            return None;
        }
        let refuse = |status: u32| Reply {
            rpc_result: status,
            body: Vec::new(),
        };
        // A serialized envelope carries no flat params struct to decode.
        if kf_abi::rpc_params_are_serialized(h.rmapi_rpc_flags) {
            return Some(refuse(kf_abi::defapi::NV_ERR_NOT_SUPPORTED));
        }
        let Some(params) = h
            .params_at
            .checked_add(h.params_size as usize)
            .and_then(|e| cmd.payload.get(h.params_at..e))
        else {
            return Some(refuse(kf_abi::defapi::NV_ERR_INVALID_PARAM_STRUCT));
        };
        let status = self
            .objects
            .deferred_api(&abi, h.client, h.object, ctl, params)?;
        self.defapi_served += 1;
        if self.defapi_served <= 256 {
            let handle = params
                .get(0..4)
                .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
            let inner = params
                .get(4..8)
                .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
            eprintln!(
                "kf-rm: DEFERRED-API {:#010x} on {:#x}:{:#x} hApiHandle={handle:#x} cmd={inner:#x} psz={} -> {status:#x}",
                h.cmd,
                h.client,
                h.object,
                params.len()
            );
        }
        if status != kf_abi::defapi::NV_OK {
            return Some(refuse(status));
        }
        let mut body = cmd.payload.clone();
        let st = self.bridge.abi.rm_control_wire().status_off;
        if let Some(w) = body.get_mut(st..st + 4) {
            w.copy_from_slice(&kf_abi::defapi::NV_OK.to_le_bytes());
        }
        Some(Reply {
            rpc_result: kf_abi::defapi::NV_OK,
            body,
        })
    }

    /// Whether this link claims `f`.
    #[must_use]
    pub fn claims(f: kf_gsp::RpcFunction) -> bool {
        OBJECT_VERBS.contains(&f)
    }

    /// A snapshot of the refusal census.
    #[must_use]
    pub fn census(&self) -> RefusalCensus {
        self.bridge.census.snapshot()
    }

    /// The shared census handle — keep it before boxing the link.
    #[must_use]
    pub fn refusal_census(&self) -> SharedRefusalCensus {
        self.bridge.census.clone()
    }

    /// Events applied.
    #[must_use]
    pub fn applied(&self) -> u64 {
        self.bridge.applied
    }

    /// The `Result` form, for claimed and unclaimed commands alike.
    ///
    /// # Errors
    /// The [`BridgeRefusal`], by name.
    pub fn deliver(&mut self, cmd: &RpcCommand) -> Result<Translation, BridgeRefusal> {
        self.bridge.deliver(&mut *self.objects, cmd)
    }
}

impl core::fmt::Debug for ObjectPolicy {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.bridge.debug_as("ObjectPolicy", f)
    }
}

impl CommandPolicy for ObjectPolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function == kf_gsp::RpcFunction::RmControl
            && let Some(r) = self.deferred_api_control(cmd)
        {
            return Some(r);
        }
        let memory = self.bridge.memory_list_probe
            && kf_abi::memory_list::cell(self.bridge.abi.driver_version())
                .is_some_and(|c| cmd.function == kf_gsp::RpcFunction::Other(c.function()));
        if !ObjectPolicy::claims(cmd.function) && !memory {
            return None;
        }
        Some(self.bridge.respond(&mut *self.objects, cmd))
    }
}

kf_util::assert_send_sync!(
    RefusalCensus,
    SharedRefusalCensus,
    ObjectsRefusal,
    GraphObjects
);
