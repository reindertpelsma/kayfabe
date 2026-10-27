//! SA-04 remaining cache scope: FALSIFIER for limiting backfill to the incoming
//! Device's subtree: a late parent plus an unrelated Device alloc changes the
//! sticky target retained by a foreign alias after the source Device is freed.
use kf_abi::generated::classes as nv;
use kf_arch::ClientKind;
use kf_arch::ids::{ClassId, GpuId, HClient, HObject};
use kf_chip::Family;
use kf_rm::rmgraph::{AllocFacts, NodeKey, RmEvent, RmGraph};
fn target_after_late_parent(backfill: bool) -> Option<GpuId> {
    let (a, b) = (HClient(100), HClient(200));
    let mut g = RmGraph::new(Family::Ada);
    for client in [a, b] {
        g.apply(RmEvent::Alloc {
            client,
            parent: HObject(client.0),
            handle: HObject(client.0),
            class: ClassId(nv::NV01_ROOT_CLIENT),
            facts: AllocFacts {
                client_kind: Some(ClientKind::User { pid: client.0 }),
                ..Default::default()
            },
        })
        .unwrap();
    }
    let alloc = |handle, parent, class, device| RmEvent::Alloc {
        client: a,
        parent: HObject(parent),
        handle: HObject(handle),
        class: ClassId(class),
        facts: AllocFacts {
            device_instance: device,
            ..Default::default()
        },
    };
    g.apply(alloc(10, a.0, nv::NV01_DEVICE_0, Some(0))).unwrap();
    g.apply(alloc(12, 11, nv::FERMI_VASPACE_A, None)).unwrap();
    let alias = NodeKey::new(b, HObject(20));
    g.apply(RmEvent::Dup {
        src: NodeKey::new(a, HObject(12)),
        dst: alias,
    })
    .unwrap();
    g.apply(alloc(11, 10, nv::NV20_SUBDEVICE_0, None)).unwrap();
    if backfill {
        g.apply(alloc(30, a.0, nv::NV01_DEVICE_0, Some(0))).unwrap();
    }
    for handle in [12, 10] {
        g.apply(RmEvent::Free {
            client: a,
            handle: HObject(handle),
        })
        .unwrap();
    }
    g.gpu_of(alias)
}
#[test]
fn an_unrelated_device_backfills_a_late_parent_before_the_origin_becomes_a_ghost() {
    assert_eq!(target_after_late_parent(true), Some(GpuId(0)));
}
#[test]
fn without_the_backfill_event_the_alias_has_no_sticky_target_after_device_free() {
    assert_eq!(target_after_late_parent(false), None);
}
