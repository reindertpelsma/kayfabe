//! Deferred API constructor contract: real graph state, no deferred execution claimed.
use kf_abi::{GuestOs, generated::classes, versions};
use kf_arch::{
    ClientKind, ObjectKind,
    ids::{ClassId, HClient, HObject},
};
use kf_gsp::{RpcCommand, RpcFunction};
use kf_rm::{
    rmgraph::{AllocFacts, NodeKey, RmEvent},
    rmrpc::{self, GraphObjects, RmObjects, Translation},
};

fn event(parent: u32, handle: u32, class: u32, facts: AllocFacts) -> RmEvent {
    RmEvent::Alloc {
        client: HClient(1),
        parent: HObject(parent),
        handle: HObject(handle),
        class: ClassId(class),
        facts,
    }
}
fn objects() -> GraphObjects {
    let mut o = GraphObjects::new(kf_chip::Family::Ada);
    for ev in [
        event(
            1,
            1,
            classes::NV01_ROOT_CLIENT,
            AllocFacts {
                client_kind: Some(ClientKind::User { pid: 123 }),
                ..Default::default()
            },
        ),
        event(
            1,
            2,
            classes::NV01_DEVICE_0,
            AllocFacts {
                device_instance: Some(0),
                ..Default::default()
            },
        ),
        event(
            2,
            3,
            classes::AMPERE_CHANNEL_GPFIFO_A,
            AllocFacts::default(),
        ),
    ] {
        o.apply(ev, &[]).unwrap();
    }
    o
}
fn command(parent: u32, params: &[u8]) -> RpcCommand {
    let mut payload: Vec<_> = [
        1,
        parent,
        4,
        classes::NV50_DEFERRED_API_CLASS,
        0,
        params.len() as u32,
        0,
        0,
    ]
    .into_iter()
    .flat_map(u32::to_le_bytes)
    .collect();
    payload.extend(params);
    RpcCommand {
        function: RpcFunction::RmAlloc,
        code: 103,
        sequence: 1,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}
fn decode(parent: u32, params: &[u8]) -> RmEvent {
    let abi = versions::table_for(versions::BENCH_DRIVER).unwrap();
    match rmrpc::translate(abi, GuestOs::Windows, &command(parent, params)).unwrap() {
        Translation::Event(ev) => ev,
        other => panic!("expected constructor event, got {other:?}"),
    }
}
#[test]
fn deferred_api_constructor_registers_optional_notification_and_channel_lifetime() {
    for (params, notify) in [(&[][..], false), (&[0][..], false), (&[1][..], true)] {
        let mut o = objects();
        let ev = decode(3, params);
        o.apply(ev, params).unwrap();
        o.apply(ev, params).unwrap(); // exact retry retains one object
        let key = NodeKey::new(HClient(1), HObject(4));
        let n = o.graph.node(key).unwrap();
        assert_eq!(n.facts.deferred_api_notify, Some(notify));
        assert!(
            !matches!(n.kind, ObjectKind::EngineObject { .. }),
            "software object has no GPU twin"
        );
        o.apply(
            RmEvent::Free {
                client: HClient(1),
                handle: HObject(3),
            },
            &[],
        )
        .unwrap();
        assert!(
            o.graph.node(key).is_none(),
            "parent channel destruction removes its software object"
        );
    }
}
#[test]
fn deferred_api_constructor_refuses_wrong_parent_shape_and_conflicting_retry() {
    let mut o = objects();
    let before = o.graph.nodes().count();
    for parent in [0, 1, 2, 99] {
        assert!(o.apply(decode(parent, &[1]), &[1]).is_err());
        assert_eq!(o.graph.nodes().count(), before);
    }
    let abi = versions::table_for(versions::BENCH_DRIVER).unwrap();
    for params in [&[2][..], &[1, 0][..]] {
        assert!(rmrpc::translate(abi, GuestOs::Windows, &command(3, params)).is_err());
    }
    o.apply(decode(3, &[0]), &[0]).unwrap();
    assert!(
        o.apply(decode(3, &[1]), &[1]).is_err(),
        "cannot rewrite a live object's policy"
    );
    assert_eq!(
        o.graph
            .node(NodeKey::new(HClient(1), HObject(4)))
            .unwrap()
            .facts
            .deferred_api_notify,
        Some(false)
    );
}

// ★ OWNER_RULINGS §U (2026-10-07): the registration controls, served from the object graph's
// per-object tables through the object seat — what the real GSP answered in VFIO 8/9/10.

fn control(client: u32, object: u32, cmd: u32, params: &[u8]) -> RpcCommand {
    let abi = versions::table_for(versions::BENCH_DRIVER).unwrap();
    let w = abi.rm_control_wire();
    let mut payload = vec![0u8; w.params_off + params.len()];
    payload[0..4].copy_from_slice(&client.to_le_bytes());
    payload[4..8].copy_from_slice(&object.to_le_bytes());
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[w.params_size_off..w.params_size_off + 4]
        .copy_from_slice(&(params.len() as u32).to_le_bytes());
    payload[w.params_off..].copy_from_slice(params);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 76,
        sequence: 1,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

fn v1(handle: u32, cmd: u32, flags: u32) -> Vec<u8> {
    let mut p = vec![0u8; 584];
    p[0..4].copy_from_slice(&handle.to_le_bytes());
    p[4..8].copy_from_slice(&cmd.to_le_bytes());
    p[8..12].copy_from_slice(&flags.to_le_bytes());
    p
}

fn seat() -> (rmrpc::ObjectPolicy, std::sync::Arc<kf_rm::defapi::Registry>) {
    use kf_gsp::CommandPolicy;
    let reg = std::sync::Arc::new(kf_rm::defapi::Registry::new(
        kf_rm::defapi::Bounds::default(),
    ));
    let o = objects().with_deferred_api(reg.clone());
    let abi = versions::table_for(versions::BENCH_DRIVER).unwrap();
    let mut p = rmrpc::ObjectPolicy::over(
        abi,
        GuestOs::Windows,
        Box::new(o),
        rmrpc::ReasmLimits::default(),
    );
    // The 5080 object, as Windows allocates it: `paramsSize` 0 under its channel.
    let r = p.respond(&command(3, &[])).expect("alloc answered");
    assert_eq!(r.rpc_result, 0);
    (p, reg)
}

#[test]
fn registration_controls_are_served_from_the_objects_own_table() {
    use kf_gsp::CommandPolicy;
    let (mut p, reg) = seat();
    let key = kf_rm::defapi::ObjKey {
        client: 1,
        object: 4,
    };
    // VFIO 10 idx 3066: echoed, NV_OK.
    let c = control(1, 4, 0x5080_0101, &v1(0x4000_0002, 0x2080_012d, 0));
    let r = p.respond(&c).expect("served");
    assert_eq!(r.rpc_result, 0);
    assert_eq!(r.body, c.payload, "the params come back unchanged");
    assert_eq!(reg.entries(key).len(), 1);
    // Duplicate on the object, a live handle of the client (the channel, 3), 0, hClient.
    for h in [0x4000_0002, 3, 0, 1] {
        let r = p.respond(&control(1, 4, 0x5080_0101, &v1(h, 0x2080_012b, 0)));
        assert_eq!(r.unwrap().rpc_result, 0x33, "{h:#x}");
    }
    // `_INTERNAL` (the V2 params, same size here) registers too; a wrong size does not.
    assert_eq!(
        p.respond(&control(1, 4, 0x5080_0104, &v1(9, 0x2080_2502, 0)))
            .unwrap()
            .rpc_result,
        0
    );
    assert_eq!(
        p.respond(&control(1, 4, 0x5080_0101, &[0u8; 100]))
            .unwrap()
            .rpc_result,
        0x3a
    );
    // A control naming the CHANNEL (not a 5080 object) is refused by handle.
    assert_eq!(
        p.respond(&control(1, 3, 0x5080_0101, &v1(5, 0, 0)))
            .unwrap()
            .rpc_result,
        0x33
    );
    // `_REMOVE_API`: NV_OK, then NV_ERR_GENERIC.
    let rm = |p: &mut rmrpc::ObjectPolicy| {
        p.respond(&control(1, 4, 0x5080_0102, &9u32.to_le_bytes()))
            .unwrap()
            .rpc_result
    };
    assert_eq!(rm(&mut p), 0);
    assert_eq!(rm(&mut p), 0xffff);
    // The channel's free takes the object and its table.
    use kf_gsp::RpcCommand as C;
    let free = C {
        function: RpcFunction::Free,
        code: 10,
        sequence: 1,
        payload: [1u32, 2, 3, 0]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect(),
        elements: 1,
        delivered: Vec::new(),
    };
    let _ = p.respond(&free);
    assert!(
        reg.entries(key).is_empty(),
        "defapiDestruct: the tree goes with the object"
    );
    assert_eq!(reg.stats().0, 0);
}

/// Without a registry (a seat built as before), a 5080 control falls through untouched.
#[test]
fn without_tables_the_control_falls_through() {
    use kf_gsp::CommandPolicy;
    let abi = versions::table_for(versions::BENCH_DRIVER).unwrap();
    let mut p = rmrpc::ObjectPolicy::over(
        abi,
        GuestOs::Windows,
        Box::new(objects()),
        rmrpc::ReasmLimits::default(),
    );
    assert!(p.respond(&command(3, &[])).is_some());
    assert!(
        p.respond(&control(1, 4, 0x5080_0101, &v1(2, 0, 0)))
            .is_none()
    );
}
