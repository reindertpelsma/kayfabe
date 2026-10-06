//! Allocation-only diagnostic. No host RM verbs, memory, scheduler, or completion.
//! Native zero maxTSGs selects real default-capacity double buffers; this experiment
//! deliberately supplies only an unscheduled guest graph object, not those effects.

use crate::rmgraph::{AllocFacts, NodeKey, RmEvent, RmGraph, SoftwareRunlistProbe};
use crate::rmrpc::{BridgeRefusal, ObjectsRefusal};
use kf_abi::{sw_runlist, versions::DriverAbiTable};
use kf_arch::{
    ObjectKind,
    ids::{ClassId, HClient, HObject},
};
use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

/// Typed request after exact layout/identity/feature/advertised-engine checks.
#[derive(Debug, Clone, Copy)]
pub struct Declaration {
    pub(crate) client: HClient,
    pub(crate) parent: HObject,
    pub(crate) handle: HObject,
    pub(crate) class: ClassId,
    pub(crate) engine: u32,
}

pub(crate) struct Probe {
    logged: AtomicU8,
    driver: DriverAbiTable,
    identity: Arc<AtomicBool>,
    engines: [u32; kf_abi::gspstaticinfo::ENGINE_CAPS_WORDS],
}

pub(crate) struct IdentityObserver {
    driver: DriverAbiTable,
    identity: Arc<AtomicBool>,
    observed: u8,
}

impl Probe {
    pub(crate) fn new(
        driver: DriverAbiTable,
        engines: [u32; kf_abi::gspstaticinfo::ENGINE_CAPS_WORDS],
    ) -> (Self, IdentityObserver) {
        let identity = Arc::new(AtomicBool::new(false));
        (
            Self {
                logged: AtomicU8::new(0),
                driver,
                identity: identity.clone(),
                engines,
            },
            IdentityObserver {
                driver,
                identity,
                observed: 0,
            },
        )
    }

    pub(crate) fn decode(&self, cmd: &RpcCommand) -> Option<Result<Declaration, BridgeRefusal>> {
        if cmd.function != RpcFunction::RmAlloc {
            return None;
        }
        let req = self.driver.decode_rpc_alloc(cmd.wire_body()).ok()?;
        if !sw_runlist::is_probe_class(req.class) {
            return None;
        }
        let decode = || {
            let cell = sw_runlist::cell(self.driver.driver_version())
                .filter(|c| c.class == req.class)
                .ok_or_else(|| refusal("software-runlist probe: unsupported driver cell"))?;
            if !self.identity.load(Ordering::Relaxed) {
                return Err(refusal("software-runlist probe: unmeasured guest identity"));
            }
            // Refuse all unsupported flags, not only the serialization bit.
            if req.params_flags != 0 {
                return Err(refusal(
                    "software-runlist probe: unsupported parameter flags",
                ));
            }
            let params = super::rmrpc::alloc_params_window(&self.driver, cmd.wire_body())
                .ok_or_else(|| refusal("software-runlist probe: truncated parameters"))?;
            let params = cell
                .decode(params)
                .ok_or_else(|| refusal("software-runlist probe: unsupported layout"))?;
            if self
                .logged
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                    (n < 16).then(|| n + 1)
                })
                .is_ok()
            {
                eprintln!(
                    "kf-rm: EXPERIMENT software-runlist request engine={} maxTSGs={} qosIntrEnableMask={:#x}; metadata only",
                    params.engine, params.max_tsgs, params.qos
                );
            }
            if params.max_tsgs != 0 || params.qos != 0 {
                return Err(refusal(
                    "software-runlist probe: native capacity/QoS not implemented",
                ));
            }
            let engine = params.engine;
            if !self
                .engines
                .get(engine as usize / 32)
                .is_some_and(|word| word & (1 << (engine % 32)) != 0)
            {
                return Err(refusal("software-runlist probe: engine was not advertised"));
            }
            if req.client == 0
                || req.handle == 0
                || req.handle == req.parent
                || req.handle == req.client
            {
                return Err(refusal(
                    "software-runlist probe: invalid client/object handle",
                ));
            }
            Ok(Declaration {
                client: HClient(req.client),
                parent: HObject(req.parent),
                handle: HObject(req.handle),
                class: ClassId(req.class),
                engine,
            })
        };
        Some(decode())
    }
}

impl CommandPolicy for IdentityObserver {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function == RpcFunction::SetGuestSystemInfo {
            let admitted = sw_runlist::cell(self.driver.driver_version())
                .is_some_and(|c| c.matches_identity(&cmd.payload))
                && kf_abi::guestsysinfo::decode_declared_vgx(&cmd.payload).ok()
                    == self.driver.vgx_version();
            // Every new declaration replaces the old one; malformed/changed identities revoke.
            self.identity.store(admitted, Ordering::Relaxed);
        }
        if let Some(record) = self.request_observation(cmd) {
            eprintln!("kf-rm: EXPERIMENT software-runlist diagnostic {record}; observation only");
        }
        None
    }
}

impl IdentityObserver {
    /// A fixed, bounded diagnostic. It never supplies a reply, touches guest memory,
    /// reads transport padding, or interprets the opaque control words as pointers.
    fn request_observation(&mut self, cmd: &RpcCommand) -> Option<String> {
        use std::fmt::Write;
        if self.observed >= 16 || !self.identity.load(Ordering::Relaxed) {
            return None;
        }
        let cell = sw_runlist::cell(self.driver.driver_version())?;
        let mut record = String::new();
        if cmd.function == RpcFunction::RmControl {
            let request = self.driver.decode_rpc_control(&cmd.payload).ok()?;
            // Consume only the declared RPC payload, never delivered queue padding.
            // Exact body size also excludes partial or unexpected extended requests.
            if request.cmd != cell.observed_control
                || request.rmapi_rpc_flags != 0
                || cell.observed_control_size != 40
                || request.params_size != 40
            {
                return None;
            }
            let end = request.params_at.checked_add(cell.observed_control_size)?;
            if cmd.payload.len() != end {
                return None;
            }
            let params = cmd.payload.get(request.params_at..end)?;
            write!(
                record,
                "control={:#010x} params_bytes=40 raw_u32_le=[",
                request.cmd
            )
            .ok()?;
            for (index, word) in params.as_chunks::<4>().0.iter().enumerate() {
                if index != 0 {
                    record.push(',');
                }
                write!(record, "{:08x}", u32::from_le_bytes(*word)).ok()?;
            }
            record.push(']');
        } else if let RpcFunction::Other(code) = cmd.function {
            let function =
                kf_abi::generated::matrix::RPC_FUNCTIONS_NV_VGPU_MSG_FUNCTION_ALLOC_MEMORY
                    .at(self.driver.driver_version())
                    .ok()
                    .flatten()?;
            if u64::from(code) != function || cmd.payload.len() < cell.alloc_memory_size {
                return None;
            }
            // Source-known scalar prefix. The exact one-inline-entry case also gets
            // a bounded raw observation below; neither observation admits semantics.
            write!(
                record,
                "function={code} ALLOC_MEMORY payload_bytes={} prefix_only=true",
                cmd.payload.len()
            )
            .ok()?;
            for &(name, offset, width) in cell.alloc_memory_fields {
                let bytes = cmd.payload.get(offset..offset.checked_add(width)?)?;
                let value = match width {
                    4 => u64::from(u32::from_le_bytes(bytes.try_into().ok()?)),
                    8 => u64::from_le_bytes(bytes.try_into().ok()?),
                    _ => return None,
                };
                write!(record, " {name}={value:#x}").ok()?;
            }
            // Exact single-inline-entry record only: two bounded words, no guest-sized
            // tail or indirection read. Observation does not admit or dereference it.
            if cmd.payload.len() == cell.alloc_memory_size.checked_add(8)? {
                let start = cell.alloc_memory_size.checked_sub(8)?;
                let words = cmd.payload.get(start..)?.as_chunks::<8>().0;
                if let [descriptor, entry] = words {
                    write!(
                        record,
                        " raw_descriptor_u64_le=[{:016x},{:016x}]",
                        u64::from_le_bytes(*descriptor),
                        u64::from_le_bytes(*entry)
                    )
                    .ok()?;
                }
            }
        } else {
            return None;
        }
        self.observed += 1;
        Some(record)
    }
}

fn refusal(what: &'static str) -> BridgeRefusal {
    BridgeRefusal::Objects(ObjectsRefusal::NotModelled { what })
}

/// Graph-only allocation: explicit direct owned parents, ordinary resource quotas/lifetime.
pub(crate) fn allocate(
    graph: &mut RmGraph,
    request: Declaration,
) -> Result<RmEvent, ObjectsRefusal> {
    let reject = || ObjectsRefusal::NotModelled {
        what: "software-runlist probe: missing/wrong/foreign parent",
    };
    let sub_key = NodeKey::new(request.client, request.parent);
    let sub = graph
        .allocated_node(sub_key)
        .filter(|n| n.kind == ObjectKind::Subdevice && n.key == sub_key)
        .ok_or_else(reject)?;
    let dev_key = NodeKey::new(request.client, sub.parent);
    let dev = graph
        .allocated_node(dev_key)
        .filter(|n| n.kind == ObjectKind::Device && n.key == dev_key)
        .ok_or_else(reject)?;
    // The direct client root and physical target must already exist. No parked parents.
    let root_key = NodeKey::new(request.client, dev.parent);
    if !graph
        .allocated_node(root_key)
        .is_some_and(|n| n.kind == ObjectKind::Client && n.key == root_key)
        || graph.gpu_of(sub_key).is_none()
        || graph.gpu_of(sub_key) != graph.gpu_of(dev_key)
    {
        return Err(reject());
    }
    let event = RmEvent::Alloc {
        client: request.client,
        parent: request.parent,
        handle: request.handle,
        class: request.class,
        facts: AllocFacts {
            software_runlist_probe: Some(SoftwareRunlistProbe {
                engine_type: request.engine,
                subdevice: sub.id(),
                device: dev.id(),
            }),
            ..Default::default()
        },
    };
    graph.apply(event).map_err(ObjectsRefusal::Graph)?;
    Ok(event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rmrpc::{GraphObjects, ObjectPolicy, RmObjects};
    use kf_arch::ClientKind;

    fn driver() -> DriverAbiTable {
        *kf_abi::versions::table_for(kf_abi::DriverVersion {
            major: 580,
            minor: 65,
            patch: 6,
        })
        .unwrap()
    }
    fn command(function: RpcFunction, payload: Vec<u8>) -> RpcCommand {
        RpcCommand {
            function,
            code: 0,
            sequence: 1,
            payload,
            elements: 1,
            delivered: Vec::new(),
        }
    }
    fn identity() -> RpcCommand {
        use kf_abi::guestsysinfo::*;
        let twin = kf_abi::generated::windows_twins::WINDOWS_TWINS
            .iter()
            .find(|t| t.win_name == "580.88")
            .unwrap();
        let mut p = vec![0; SET_GUEST_SYSTEM_INFO_SIZE];
        let vgx = driver().vgx_version().unwrap();
        p[VGX_MAJOR_OFF..VGX_MAJOR_OFF + 4].copy_from_slice(&vgx.major.to_le_bytes());
        p[VGX_MINOR_OFF..VGX_MINOR_OFF + 4].copy_from_slice(&vgx.minor.to_le_bytes());
        p[GUEST_DRIVER_VERSION_OFF..GUEST_DRIVER_VERSION_OFF + twin.win_name.len()]
            .copy_from_slice(twin.win_name.as_bytes());
        p[GUEST_VERSION_OFF..GUEST_VERSION_OFF + twin.win_branch.len()]
            .copy_from_slice(twin.win_branch.as_bytes());
        p[GUEST_CL_NUM_OFF..GUEST_CL_NUM_OFF + 4].copy_from_slice(&twin.win_cl.to_le_bytes());
        command(RpcFunction::SetGuestSystemInfo, p)
    }
    fn fixture() -> (Probe, IdentityObserver, RpcCommand) {
        let mut engines = [0; kf_abi::gspstaticinfo::ENGINE_CAPS_WORDS];
        engines[0] = 1 << kf_abi::submit::ENGINE_TYPE_GRAPHICS;
        let (probe, mut observer) = Probe::new(driver(), engines);
        observer.respond(&identity());
        // Independent nvos.h layout: engineId@0, maxTSGs@4, qosIntrEnableMask@8.
        let mut p = vec![0; 44];
        for (off, v) in [(0, 1u32), (4, 3), (8, 4), (12, 0xb297), (20, 12), (32, 1)] {
            p[off..off + 4].copy_from_slice(&v.to_le_bytes());
        }
        (probe, observer, command(RpcFunction::RmAlloc, p))
    }
    fn event(client: u32, parent: u32, handle: u32, class: u32, facts: AllocFacts) -> RmEvent {
        RmEvent::Alloc {
            client: HClient(client),
            parent: HObject(parent),
            handle: HObject(handle),
            class: ClassId(class),
            facts,
        }
    }
    fn add_namespace(graph: &mut RmGraph, c: u32) {
        graph
            .apply(event(
                c,
                c,
                c,
                0x41,
                AllocFacts {
                    client_kind: Some(ClientKind::User { pid: c }),
                    ..Default::default()
                },
            ))
            .unwrap();
        graph
            .apply(event(
                c,
                c,
                2,
                0x80,
                AllocFacts {
                    device_instance: Some(0),
                    ..Default::default()
                },
            ))
            .unwrap();
        graph
            .apply(event(c, 2, 3, 0x2080, AllocFacts::default()))
            .unwrap();
    }
    fn graph() -> RmGraph {
        let mut graph = RmGraph::new(kf_chip::Family::Ampere);
        add_namespace(&mut graph, 1);
        graph
    }
    fn key(client: u32, handle: u32) -> NodeKey {
        NodeKey::new(HClient(client), HObject(handle))
    }
    fn request() -> Declaration {
        let (p, _, cmd) = fixture();
        p.decode(&cmd).unwrap().unwrap()
    }

    fn scheduling_control() -> RpcCommand {
        let wire = driver().rm_control_wire();
        let mut payload = vec![0; wire.params_off + 40];
        payload[8..12].copy_from_slice(&0x20801111u32.to_le_bytes());
        payload[wire.params_size_off..wire.params_size_off + 4]
            .copy_from_slice(&40u32.to_le_bytes());
        for (index, byte) in payload[wire.params_off..].iter_mut().enumerate() {
            *byte = index as u8;
        }
        command(RpcFunction::RmControl, payload)
    }

    #[test]
    fn diagnostic_control_observes_exact_words_without_changing_or_answering() {
        let (_, mut observer, _) = fixture();
        let cmd = scheduling_control();
        let before = cmd.clone();
        let record = observer.request_observation(&cmd).unwrap();
        assert!(record.contains("params_bytes=40 raw_u32_le=[03020100,07060504"));
        assert!(record.ends_with("27262524]"));
        assert!(record.len() < 200);
        assert!(observer.respond(&cmd).is_none());
        assert_eq!(cmd, before);
    }

    #[test]
    fn diagnostic_never_reads_short_extended_serialized_or_transport_padding() {
        for (offset, value) in [
            (8, 0x20801110u32),
            (16, 0),
            (16, 39),
            (16, 41),
            (16, u32::MAX),
            (20, 1),
            (20, 2),
        ] {
            let (_, mut observer, _) = fixture();
            let mut cmd = scheduling_control();
            cmd.payload[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(observer.request_observation(&cmd).is_none());
            assert_eq!(observer.observed, 0);
        }
        for length in [0, 12, 39, 40, 79, 81, 256] {
            let (_, mut observer, _) = fixture();
            let mut cmd = scheduling_control();
            cmd.delivered = cmd.payload.clone();
            cmd.payload.resize(length, 0);
            assert!(observer.request_observation(&cmd).is_none());
            assert_eq!(observer.observed, 0);
        }
    }

    #[test]
    fn alloc_memory_diagnostic_uses_only_compiled_scalar_prefix_not_pte_bytes() {
        let (_, mut observer, _) = fixture();
        let mut payload = vec![0; 5000];
        payload[12..16].copy_from_slice(&0x3eu32.to_le_bytes());
        payload[32..40].copy_from_slice(&0x1234000u64.to_le_bytes());
        payload[40..44].copy_from_slice(&3u32.to_le_bytes());
        payload[44..].fill(0xfe);
        let mut cmd = command(RpcFunction::Other(4), payload);
        let record = observer.request_observation(&cmd).unwrap();
        assert!(record.contains("ALLOC_MEMORY payload_bytes=5000 prefix_only=true"));
        assert!(
            record.contains("hClass=0x3e")
                && record.contains("length=0x1234000")
                && record.contains("pageCount=0x3")
        );
        assert!(!record.contains("fefe"));
        assert!(record.len() < 350);
        assert!(observer.respond(&cmd).is_none());
        cmd.delivered = cmd.payload.clone();
        cmd.payload.truncate(55);
        assert!(observer.request_observation(&cmd).is_none());
        cmd.payload.resize(56, 0);
        cmd.function = RpcFunction::Other(5);
        assert!(observer.request_observation(&cmd).is_none());
    }

    #[test]
    fn alloc_memory_single_entry_observation_is_bounded_and_does_not_admit_it() {
        let (_, mut observer, _) = fixture();
        let mut payload = vec![0; 64];
        payload[48..56].copy_from_slice(&0x10000_u64.to_le_bytes());
        payload[56..64].copy_from_slice(&0x123456_u64.to_le_bytes());
        let cmd = command(RpcFunction::Other(4), payload);
        let before = cmd.clone();
        let record = observer.request_observation(&cmd).unwrap();
        assert!(record.contains("raw_descriptor_u64_le=[0000000000010000,0000000000123456]"));
        assert!(record.len() < 450);
        assert!(observer.respond(&cmd).is_none());
        assert_eq!(cmd, before);
        let mut extended = cmd;
        extended.payload.push(0xff);
        let record = observer.request_observation(&extended).unwrap();
        assert!(!record.contains("raw_descriptor"));
    }

    #[test]
    fn observation_identity_gate_and_combined_sixteen_record_budget_are_strict() {
        let (_, mut observer, _) = fixture();
        let control = scheduling_control();
        let memory = command(RpcFunction::Other(4), vec![0; 56]);
        observer.respond(&command(RpcFunction::SetGuestSystemInfo, vec![]));
        assert!(observer.request_observation(&control).is_none());
        assert!(observer.request_observation(&memory).is_none());
        observer.respond(&identity());
        for index in 0..100 {
            let record =
                observer.request_observation(if index % 2 == 0 { &control } else { &memory });
            assert_eq!(record.is_some(), index < 16);
        }
        assert_eq!(observer.observed, 16);
        observer.respond(&identity());
        assert!(
            observer.request_observation(&control).is_none(),
            "new identity does not reset this chain's cap"
        );
    }

    #[test]
    fn exact_cell_identity_is_required_and_revoked_by_bad_handshake() {
        let (probe, mut observer, cmd) = fixture();
        assert!(probe.decode(&cmd).unwrap().is_ok());
        observer.respond(&command(RpcFunction::SetGuestSystemInfo, vec![]));
        assert!(probe.decode(&cmd).unwrap().is_err());
        let mut linux = identity();
        linux.payload[24..280].fill(0);
        linux.payload[24..33].copy_from_slice(b"580.65.06");
        observer.respond(&linux);
        assert!(probe.decode(&cmd).unwrap().is_err());
        observer.respond(&identity());
        assert!(probe.decode(&cmd).unwrap().is_ok());
        for version in kf_abi::generated::matrix::MEASURED {
            assert_eq!(
                sw_runlist::cell(*version).is_some(),
                *version == driver().driver_version()
            );
        }
    }

    #[test]
    fn parameter_lengths_features_serialization_unknown_engines_and_handles_fail_closed() {
        for (off, value) in [
            (20, 0),
            (20, 11),
            (20, 13),
            (20, u32::MAX),
            (24, 1),
            (24, 2),
            (24, u32::MAX),
            (32, 0),
            (32, 9),
            (32, u32::MAX),
            (36, 1),
            (40, 1),
            (0, 0),
            (8, 0),
            (8, 1),
            (8, 3),
        ] {
            let (probe, _, mut cmd) = fixture();
            cmd.payload[off..off + 4].copy_from_slice(&value.to_le_bytes());
            assert!(
                probe.decode(&cmd).unwrap().is_err(),
                "offset={off} value={value}"
            );
        }
        let (probe, _, mut cmd) = fixture();
        cmd.payload.pop();
        assert!(probe.decode(&cmd).unwrap().is_err());
    }

    #[test]
    fn typed_allocation_retries_conflicts_free_parent_and_client_lifetimes() {
        for free in [4, 3, 2, 1] {
            let mut g = graph();
            allocate(&mut g, request()).unwrap();
            let node = *g.node(key(1, 4)).unwrap();
            assert_eq!(node.kind, ObjectKind::SoftwareRunlistProbe);
            assert_eq!(node.facts.software_runlist_probe.unwrap().engine_type, 1);
            allocate(&mut g, request()).unwrap();
            assert_eq!(g.node(key(1, 4)), Some(&node));
            let mut conflict = request();
            conflict.engine = 9;
            assert!(allocate(&mut g, conflict).is_err());
            g.apply(RmEvent::Free {
                client: HClient(1),
                handle: HObject(free),
            })
            .unwrap();
            assert!(g.node(key(1, 4)).is_none());
            if free != 4 {
                assert!(allocate(&mut g, request()).is_err());
            }
        }
    }

    #[test]
    fn wrong_missing_foreign_and_alias_parents_are_refused_without_mutation() {
        for parent in [0, 1, 2, 99] {
            let mut g = graph();
            let mut r = request();
            r.parent = HObject(parent);
            assert!(allocate(&mut g, r).is_err());
            assert_eq!(g.nodes().count(), 3);
        }
        let mut g = graph();
        add_namespace(&mut g, 10);
        g.apply(RmEvent::Dup {
            src: key(10, 3),
            dst: key(1, 7),
        })
        .unwrap();
        let mut r = request();
        r.parent = HObject(7);
        assert!(allocate(&mut g, r).is_err());
        g.apply(RmEvent::Dup {
            src: key(1, 3),
            dst: key(10, 8),
        })
        .unwrap();
        g.apply(RmEvent::Free {
            client: HClient(1),
            handle: HObject(3),
        })
        .unwrap();
        g.apply(RmEvent::Dup {
            src: key(10, 8),
            dst: key(1, 3),
        })
        .unwrap();
        assert!(
            allocate(&mut g, request()).is_err(),
            "recycled origin value is still an alias"
        );
    }

    #[test]
    fn other_class_collision_cannot_be_overwritten_or_freed_by_rejected_alloc() {
        let mut g = graph();
        g.apply(event(1, 3, 4, 0x1234, AllocFacts::default()))
            .unwrap();
        let previous = *g.node(key(1, 4)).unwrap();
        assert!(allocate(&mut g, request()).is_err());
        assert_eq!(g.node(key(1, 4)), Some(&previous));
    }

    #[test]
    fn object_policy_default_denies_and_optin_accepts_without_control_widening() {
        let (probe, _, cmd) = fixture();
        let make = || {
            ObjectPolicy::over(
                &driver(),
                kf_abi::GuestOs::Linux,
                Box::new({
                    let mut objects = GraphObjects::new(kf_chip::Family::Ada);
                    objects.graph = graph();
                    objects
                }),
                Default::default(),
            )
        };
        let mut normal = make();
        assert_ne!(normal.respond(&cmd).unwrap().rpc_result, 0);
        let mut experimental = make().with_sw_runlist_probe(probe);
        assert_eq!(experimental.respond(&cmd).unwrap().rpc_result, 0);
        assert_eq!(experimental.applied(), 1);
        assert!(
            !driver()
                .capabilities()
                .alloc_class(ClassId(0xb297))
                .is_permitted()
        );
        for code in [0x20801110u32, 0x20801111] {
            let mut p = vec![0; 80];
            p[8..12].copy_from_slice(&code.to_le_bytes());
            let control = command(RpcFunction::RmControl, p);
            assert!(experimental.respond(&control).is_none());
            assert!(normal.respond(&control).is_none());
        }
    }

    #[test]
    fn allocation_consumes_normal_graph_quota() {
        let mut g = graph();
        for handle in 5..=crate::rmgraph::MAX_LIVE_HANDLES as u32 + 1 {
            g.apply(event(1, 3, handle, 0x1234, AllocFacts::default()))
                .unwrap();
        }
        assert_eq!(g.nodes().count(), crate::rmgraph::MAX_LIVE_HANDLES);
        assert!(matches!(
            allocate(&mut g, request()),
            Err(ObjectsRefusal::Graph(
                crate::rmgraph::RmGraphError::CapacityExceeded(_)
            ))
        ));
    }

    #[test]
    fn unfamiliar_object_backend_refuses_the_diagnostic_seam() {
        struct Backend;
        impl RmObjects for Backend {
            fn apply(&mut self, _: RmEvent, _: &[u8]) -> Result<(), ObjectsRefusal> {
                panic!("must never call normal/backend alloc")
            }
            fn page_dir(
                &mut self,
                _: crate::rmrpc::PageDirStatement,
            ) -> Result<(), ObjectsRefusal> {
                unreachable!()
            }
        }
        let (probe, _, cmd) = fixture();
        let mut policy = ObjectPolicy::over(
            &driver(),
            kf_abi::GuestOs::Linux,
            Box::new(Backend),
            Default::default(),
        )
        .with_sw_runlist_probe(probe);
        assert_ne!(policy.respond(&cmd).unwrap().rpc_result, 0);
    }
}
