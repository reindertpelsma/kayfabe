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
            IdentityObserver { driver, identity },
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
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
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
        None
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
                Box::new(GraphObjects { graph: graph() }),
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
