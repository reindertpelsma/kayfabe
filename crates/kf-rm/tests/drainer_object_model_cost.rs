//! ★ Drainer verification 2026-10-09 (`docs/design/V3_NONSTALL_THREADS.md` §9, items G1-G3): how long
//! ONE guest RPC can keep the register drainer inside the object model.
//!
//! The drainer applies every `GSP_RM_ALLOC` / `DUP_OBJECT` / `GSP_RM_FREE` to `RmGraph` synchronously
//! (`ObjectPolicy` → `GraphObjects::apply`, `rmgraph.rs`), and the graph holds up to
//! `MAX_LIVE_HANDLES` = 2^18 guest-chosen handles. The allocations below are real `GSP_RM_ALLOC` wire
//! bytes through `rmrpc::translate` (a permitted engine class, no params, and a `hParent` /
//! `hObject` the translation passes on verbatim), so what is shown is reachable from guest bytes at
//! the object seat. (I did not drive the other ~20 links of the served chain; none of them refuses
//! an engine-class alloc under an unknown parent as far as `translate` can tell.)
//!
//! Timing tests: build with `--release` (the debug build is ~10-30x slower) and run
//! `cargo test --release -p kf-rm --test drainer_object_model_cost -- --ignored --nocapture
//! --test-threads=1`. The bound is the task's: one drainer-side call must return in < 50 ms.

use kf_abi::{GuestOs, generated::classes, versions};
use kf_arch::{
    ClientKind,
    ids::{ClassId, HClient, HObject},
};
use kf_gsp::{RpcCommand, RpcFunction};
use kf_rm::rmgraph::{AllocFacts, NodeKey, RmEvent};
use kf_rm::rmrpc::{self, GraphObjects, RmObjects, Translation};
use std::time::{Duration, Instant};

const BOUND: Duration = Duration::from_millis(50);

/// A `GSP_RM_ALLOC` of a no-params engine class, as the guest's RM would put it on the ring.
fn rpc_alloc(client: u32, parent: u32, handle: u32, class: u32) -> RpcCommand {
    let payload: Vec<u8> = [client, parent, handle, class, 0, 0, 0, 0]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    RpcCommand {
        function: RpcFunction::RmAlloc,
        code: 103,
        sequence: 1,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

fn event_of(client: u32, parent: u32, handle: u32, class: u32) -> RmEvent {
    let abi = versions::table_for(versions::BENCH_DRIVER).unwrap();
    match rmrpc::translate(
        abi,
        GuestOs::Linux,
        &rpc_alloc(client, parent, handle, class),
    )
    .expect("the translation accepts it")
    {
        Translation::Event(ev) => ev,
        other => panic!("expected an event, got {other:?}"),
    }
}

fn objects_with_a_client() -> GraphObjects {
    let mut o = GraphObjects::new(kf_chip::Family::Ada);
    o.apply(
        RmEvent::Alloc {
            client: HClient(1),
            parent: HObject(1),
            handle: HObject(1),
            class: ClassId(classes::NV01_ROOT_CLIENT),
            facts: AllocFacts {
                client_kind: Some(ClientKind::User { pid: 7 }),
                ..Default::default()
            },
        },
        &[],
    )
    .unwrap();
    o
}

/// G1 — `RmGraph::free_subtree` (rmgraph.rs:1885-1925) finds a freed handle's descendants by
/// re-scanning the WHOLE handle table until a pass adds nothing. Handles are visited in key order, so
/// a chain whose handles DESCEND (each child's number below its parent's) adds one handle per pass:
/// depth x table size. One `GSP_RM_FREE` of the top of such a chain keeps the drainer inside it for
/// as long as that takes (measured, release build, in the exploratory run for this section:
/// depth 2000 = 122 ms, 5000 = 0.96 s, 10000 = 4.0 s; the cap allows 262 144 handles).
#[test]
#[ignore = "FINDING G1: one RM_FREE of a 3000-deep descending chain stalls the drainer for hundreds of ms (quadratic fixpoint, rmgraph.rs:1898)"]
fn one_rm_free_of_a_deep_chain_must_return_within_50_ms() {
    let mut o = objects_with_a_client();
    let top = 0x1000_0000u32;
    o.apply(event_of(1, 1, top, classes::ADA_COMPUTE_A), &[])
        .unwrap();
    let mut parent = top;
    for i in 1..3000u32 {
        let h = top - i;
        o.apply(event_of(1, parent, h, classes::ADA_COMPUTE_A), &[])
            .unwrap();
        parent = h;
    }
    let t = Instant::now();
    o.apply(
        RmEvent::Free {
            client: HClient(1),
            handle: HObject(top),
        },
        &[],
    )
    .unwrap();
    let took = t.elapsed();
    eprintln!("G1: one RM_FREE of a 3000-deep chain: {took:?}");
    assert!(took < BOUND, "one RM_FREE took {took:?}");
}

/// G3 — `RmGraph::cache_targets` (rmgraph.rs:1486) runs on EVERY `Device` alloc and collects and
/// re-walks every resource that has no GPU target yet. Objects whose parent names nothing never get
/// one, so a guest can park up to 2^18 of them and make each later Device alloc pay for all of them.
#[test]
#[ignore = "FINDING G3: a Device alloc with ~260 000 unrouted objects takes ~66 ms on the verification box (rmgraph.rs:1776-1778)"]
fn a_device_alloc_must_return_within_50_ms_whatever_the_guest_parked() {
    let mut o = objects_with_a_client();
    for i in 0..260_000u32 {
        // hParent names nothing: the object can never route to a GPU.
        o.apply(
            event_of(1, 0x7000_0000, 0x1000 + i, classes::ADA_COMPUTE_A),
            &[],
        )
        .unwrap();
    }
    let t = Instant::now();
    o.apply(
        RmEvent::Alloc {
            client: HClient(1),
            parent: HObject(1),
            handle: HObject(0x100),
            class: ClassId(classes::NV01_DEVICE_0),
            facts: AllocFacts {
                device_instance: Some(0),
                ..Default::default()
            },
        },
        &[],
    )
    .unwrap();
    let took = t.elapsed();
    eprintln!("G3: one Device alloc with 260 000 unrouted objects: {took:?}");
    assert!(took < BOUND, "a Device alloc took {took:?}");
}

/// G2 — `RmGraph::resolve_pending_dups` (rmgraph.rs:2004) scans every parked dup edge on EVERY
/// alloc and dup; `free_subtree` also `retain`s over them (:1950). A guest can park 2^18 edges by
/// duping objects that never arrive. Measured here: the per-event tax at the cap. It is under the
/// 50 ms bound (so this test PASSES when run) but it is ~11 ms of drainer time per alloc, 1000x
/// the cost of an alloc on an honest graph — recorded as a number, not as a pass.
#[test]
#[ignore = "timing (run with --release): prints the per-alloc tax of 262 000 parked dups; passes the 50 ms bound"]
fn parked_dups_tax_every_alloc_but_stay_under_the_bound_at_the_cap() {
    let mut o = objects_with_a_client();
    for i in 0..262_000u32 {
        o.apply(
            RmEvent::Dup {
                src: NodeKey::new(HClient(1), HObject(0x4000_0000 + i)),
                dst: NodeKey::new(HClient(1), HObject(0x2000_0000 + i)),
            },
            &[],
        )
        .unwrap();
    }
    let t = Instant::now();
    o.apply(event_of(1, 1, 0x100, classes::ADA_COMPUTE_A), &[])
        .unwrap();
    let took = t.elapsed();
    eprintln!("G2: one alloc with 262 000 parked dups: {took:?}");
    assert!(took < BOUND, "one alloc took {took:?}");
}
