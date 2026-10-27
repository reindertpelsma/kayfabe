//! SECURITY AUDIT 2026-10-21, finding SA-04 (docs/audits/2026-10-21-security-audit.md).
//! Imported as an EXPECTED RED test; it passes since the free paths are indexed.
//!
//! Before the index, `RmGraph::free_subtree` scanned EVERY live handle of EVERY client, at
//! least twice, for each non-root FREE. The stock guest RM frees objects one at a time, and
//! the whole thing runs on the register drainer thread. N leaf frees over N+1 live handles
//! cost O(N^2) map iterations, so an unprivileged guest process holding tens of thousands of
//! objects (or guest root with a handle chain) stalls the drainer, which "may never block"
//! (feedback: the drainer must never block).
//! The expected behaviour is a children index: freeing a leaf costs O(subtree), not
//! O(all live handles).
use kf_abi::generated::classes as nv;
use kf_arch::ClientKind;
use kf_arch::ids::{ClassId, HClient, HObject};
use kf_chip::Family;
use kf_rm::rmgraph::{AllocFacts, RmEvent, RmGraph};

const A: HClient = HClient(0xc1d0_0067);

fn alloc(parent: HObject, handle: HObject, class: u32) -> RmEvent {
    let facts = if class == nv::NV01_DEVICE_0 {
        AllocFacts {
            device_instance: Some(0),
            ..Default::default()
        }
    } else {
        AllocFacts::default()
    };
    RmEvent::Alloc {
        client: A,
        parent,
        handle,
        class: ClassId(class),
        facts,
    }
}

fn run(n: u32) -> (std::time::Duration, std::time::Duration) {
    let mut g = RmGraph::new(Family::Ada);
    g.apply(RmEvent::Alloc {
        client: A,
        parent: HObject(A.0),
        handle: HObject(A.0),
        class: ClassId(nv::NV01_ROOT_CLIENT),
        facts: AllocFacts {
            client_kind: Some(ClientKind::User { pid: A.0 }),
            ..Default::default()
        },
    })
    .unwrap();
    let dev = HObject(0x10);
    g.apply(alloc(HObject(A.0), dev, nv::NV01_DEVICE_0))
        .unwrap();
    let t = std::time::Instant::now();
    for i in 0..n {
        g.apply(alloc(dev, HObject(0x1000_0000 + i), nv::FERMI_VASPACE_A))
            .unwrap();
    }
    let allocs = t.elapsed();
    let t = std::time::Instant::now();
    for i in 0..n {
        g.apply(RmEvent::Free {
            client: A,
            handle: HObject(0x1000_0000 + i),
        })
        .unwrap();
    }
    (allocs, t.elapsed())
}

#[test]
fn freeing_many_leaf_objects_one_by_one_is_not_quadratic() {
    // Doubling N must not quadruple the work: a children index makes each leaf free O(1).
    let (a1, f1) = run(3_000);
    let (a2, f2) = run(6_000);
    eprintln!("SECAUDIT graph: N=3000 alloc {a1:?} free {f1:?}; N=6000 alloc {a2:?} free {f2:?}");
    // 5 s is far above anything a linear implementation needs even in a debug build.
    assert!(
        f2 < std::time::Duration::from_secs(5),
        "6000 leaf frees took {f2:?} (3000 took {f1:?})"
    );
}

// SA-04 release falsifier: leaf removal must not scan unrelated live handles.
// The audit's 6000-object / 5 s oracle ran in debug. These larger
// inventories preserve generous headroom for indexed removal in release.
#[test]
fn thirty_thousand_leaf_frees_fit_the_indexed_work_budget() {
    let (allocs, frees) = run(30_000);
    eprintln!("SA-04 release: n=30000 alloc={allocs:?} free={frees:?}");
    assert!(
        frees < std::time::Duration::from_secs(2),
        "30000 leaf frees took {frees:?}"
    );
}
#[test]
fn forty_five_thousand_leaf_frees_fit_the_indexed_work_budget() {
    let (allocs, frees) = run(45_000);
    eprintln!("SA-04 release: n=45000 alloc={allocs:?} free={frees:?}");
    assert!(
        frees < std::time::Duration::from_secs(2),
        "45000 leaf frees took {frees:?}"
    );
}
