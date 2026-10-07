//! ★★★★★ **A VA SPACE IS THE RM OBJECT, NOT ITS PAGE-DIRECTORY BASE (w556).**
//!
//! Owner, 2026-09-12: *"Why would you key on base, I think on rm object/channel is correct?
//! Multiple entry tables can exist right against multiple bases?"*
//!
//! `Proc::vases` used to be keyed `(GpuId, Pdb)`. The base is not an identity — it is an
//! attribute that arrives late, from `SET_PAGE_DIRECTORY`, and often never arrives at all.
//! Keying on it failed in **both** directions, and this file pins one test per direction plus
//! the refusal that replaced the silent third case.
//!
//! `[measured w554, bench boot, the raw client PASSING]` what the old key produced: the guest
//! published geometry for **12** VA spaces and exactly **one** base reached us. The rooted
//! space learned **286** rows; every other space shared one bucket holding **25**.
//!
//! ⊘ These are CORE tests, deliberately. The property is about the runtime map's key and the
//! graph that fills it — no isolate, no host, no boot. A test that needed a device to see it
//! would not have been written, which is why the defect survived to be found on hardware.

use kayfabe_arch::ids::{GpuId, HClient, HObject, Pdb};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::rmgraph::{AllocFacts, RmEvent};
use kayfabe_mocks::{MockIsolateFactory, WireClassArch, mock_classes};
use kayfabe_tests::Scenario;

const CLIENT: HClient = HClient(0xC1D0_00A1);
const DEVICE: HObject = HObject(0xC1D0_0101);
const VAS_ONE: HObject = HObject(0xCAFE_0004);
const VAS_TWO: HObject = HObject(0xCAFE_0096);
const BASE_ONE: Pdb = Pdb(0x0020_1000);
const BASE_TWO: Pdb = Pdb(0x0040_1000);
const GPU: GpuId = GpuId(0);

fn bare() -> Gpu {
    let (factory, _rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    Gpu::new(
        std::sync::Arc::new(WireClassArch::new()),
        Box::new(factory),
        gpa,
    )
    .expect("device realizes")
}

/// A client with a device and `n` VA spaces, none of which declares a page-directory base.
fn world(vaspaces: &[HObject]) -> Gpu {
    let mut g = bare();
    let mut s = Scenario::new();
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: HObject(CLIENT.0),
        handle: HObject(CLIENT.0),
        class: mock_classes::CLIENT,
        facts: kayfabe_tests::user_client(CLIENT),
    });
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: HObject(CLIENT.0),
        handle: DEVICE,
        class: mock_classes::DEVICE,
        facts: AllocFacts {
            device_instance: Some(0),
            ..Default::default()
        },
    });
    for &v in vaspaces {
        s.push(RmEvent::Alloc {
            client: CLIENT,
            parent: DEVICE,
            handle: v,
            class: mock_classes::VASPACE,
            facts: AllocFacts::default(),
        });
    }
    for ev in s.events {
        g.apply(ev).expect("the scenario applies cleanly");
    }
    g
}

fn declare_base(g: &mut Gpu, vaspace: HObject, pdb: Pdb) {
    g.apply(RmEvent::SetPageDir {
        client: CLIENT,
        vaspace,
        pdb,
        pdb_aperture: Some(kayfabe_arch::Aperture::Vidmem),
    })
    .expect("a page-directory declaration applies");
}

fn spaces(g: &Gpu) -> Vec<(HObject, Option<Pdb>)> {
    let mut out: Vec<(HObject, Option<Pdb>)> = g
        .procs
        .values()
        .flat_map(|p| p.vases.values())
        .map(|v| (HObject(0), v.pdb))
        .collect();
    out.sort_by_key(|(_, p)| p.map(|p| p.0));
    out
}

/// ★★★★★ **DIRECTION ONE — two rootless spaces must stay TWO.**
///
/// ⊘⊘ Under the old `(GpuId, Pdb)` key they were not merely merged, they were DROPPED: the
/// materialization read `filter_map(|f| Some((f.gpu?, f.pdb?)))`, so a space with no declared
/// base produced no runtime `Vas` at all. `[measured w554]` that was eleven of twelve spaces
/// in a passing boot, and their channels' operands then resolved against whatever shared
/// bucket existed — a table belonging to a DIFFERENT address space.
///
/// ⚠ The assertion is on the COUNT, because that is the whole property. A VA bound in one
/// space resolving to a row from another produces no fault and no counter; the only thing
/// that can catch it is the two spaces being two objects.
#[test]
fn two_va_spaces_with_no_declared_base_stay_two() {
    let g = world(&[VAS_ONE, VAS_TWO]);
    let live = spaces(&g);
    assert_eq!(
        live.len(),
        2,
        "two VASpace objects with no page-directory base must be two runtime spaces; found \
         {live:?}. A count of 1 is the MERGE (one row table for two address spaces) and a \
         count of 0 is the DROP (the old key discarded them entirely) — both were real."
    );
    assert!(
        live.iter().all(|(_, pdb)| pdb.is_none()),
        "neither space declared a base, so neither may report one: {live:?}. A defaulted \
         `Pdb(0)` here is exactly the sentinel that made them share a key."
    );
}

/// ★★★★★ **DIRECTION TWO — a base arriving LATE attaches to the space it belongs to.**
///
/// A rootless space is nameable before its root is known. When the declaration arrives it must
/// land on that same space, not mint a second one.
#[test]
fn a_base_declared_later_attaches_to_the_space_that_already_existed() {
    let mut g = world(&[VAS_ONE]);
    assert_eq!(spaces(&g).len(), 1, "non-vacuity: the space exists first");

    declare_base(&mut g, VAS_ONE, BASE_ONE);

    let live = spaces(&g);
    assert_eq!(
        live.len(),
        1,
        "declaring a base must not mint a second space: {live:?}"
    );
    assert_eq!(
        live[0].1,
        Some(BASE_ONE),
        "the space must report the base that was declared for it"
    );
}

/// ★★★★★ **DIRECTION THREE — a REBIND keeps one space, and keeps its rows.**
///
/// ⊘ Re-binding is protocol-legal and `rmgraph`'s own arm says so: *"UNSET/SET_PAGE_DIRECTORY;
/// last declaration wins"*. Under a base-keyed map the rebind landed on a NEW key, silently
/// splitting the space in two and orphaning every row the first half had learned — while the
/// space still looked healthy from every angle.
///
/// ⚠ This is the direction the owner named and I had not: I found the merge on my own and
/// missed the split entirely.
#[test]
fn rebinding_the_base_does_not_split_the_space() {
    let mut g = world(&[VAS_ONE]);
    declare_base(&mut g, VAS_ONE, BASE_ONE);
    assert_eq!(spaces(&g), vec![(HObject(0), Some(BASE_ONE))]);

    declare_base(&mut g, VAS_ONE, BASE_TWO);

    let live = spaces(&g);
    assert_eq!(
        live.len(),
        1,
        "a rebind must move ONE space's attribute, not create a second: {live:?}"
    );
    assert_eq!(
        live[0].1,
        Some(BASE_TWO),
        "last declaration wins, on the space that already existed"
    );
}

/// ★★★ **Two spaces may not report ONE base — and it is refused UPSTREAM, by name.**
///
/// ⊘⊘ I wrote this test expecting to pin `Proc::vas_by_pdb` answering `None` on ambiguity.
/// It does do that, and it is unreachable: the projection refuses the second declaration
/// outright with `PdbCollision`, carrying **both** resource keys. That is a stronger
/// guarantee than the lookup's, so the lookup's arm is a belt-and-braces second line and
/// this test pins the first one.
///
/// ★ The refusal names both claimants. A collision reported as *"a base is contested"*
/// without saying which two objects contest it would leave the operator with the one fact
/// they cannot derive.
#[test]
fn two_spaces_may_not_declare_the_same_base() {
    let mut g = world(&[VAS_ONE, VAS_TWO]);
    declare_base(&mut g, VAS_ONE, BASE_ONE);

    let second = g.apply(RmEvent::SetPageDir {
        client: CLIENT,
        vaspace: VAS_TWO,
        pdb: BASE_ONE,
        pdb_aperture: Some(kayfabe_arch::Aperture::Vidmem),
    });

    let err = second.expect_err(
        "a second space declaring a base that is already claimed must be REFUSED: accepting \
         it is how two address spaces come to share one identity",
    );
    let text = format!("{err:?}");
    assert!(
        text.contains("PdbCollision"),
        "the refusal must name the collision, not a generic projection failure: {text}"
    );
    assert!(
        text.contains(&format!("{:?}", VAS_ONE.0)) || text.contains(&format!("{:x}", VAS_ONE.0)),
        "and it must carry the INCUMBENT's identity so an operator can see both claimants: \
         {text}"
    );

    // ⊘ And the incumbent is untouched — a refused declaration changes nothing.
    let live = spaces(&g);
    assert_eq!(live.len(), 2, "both spaces still exist: {live:?}");
    assert_eq!(
        live.iter().filter(|(_, p)| *p == Some(BASE_ONE)).count(),
        1,
        "exactly one space reports the contested base: {live:?}"
    );
}

/// ★★★ **A rootless space is addressable but NOT sweepable, and the code says so in one place.**
///
/// ⚠ Pinned because it is the honest limit of the rekeying and it would otherwise be easy to
/// read w556 as having fixed more than it did. `Spine::vas_keys` walks spaces BY BASE — it is
/// what the publication pass iterates — so a space with none is not published. The rekeying
/// made that ONE statement instead of a merged table; it did not make it untrue.
#[test]
fn a_space_with_no_base_is_named_but_not_enumerated_by_base() {
    let g = world(&[VAS_ONE]);
    assert_eq!(spaces(&g).len(), 1, "it exists as an object");
    assert!(
        g.procs
            .values()
            .all(|p| p.vases.values().all(|v| v.pdb.is_none())),
        "and it reports no base"
    );
    assert!(
        g.procs
            .values()
            .all(|p| p.vas_by_pdb(GPU, BASE_ONE).is_none()),
        "so nothing finds it by one — which is the truth, not a bug: there is nothing to \
         match it against, and a sweep starts at the root it does not have"
    );
}
