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
