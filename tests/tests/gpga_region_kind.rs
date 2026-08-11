//! ★★★★★ **THE FOUR-KIND GPGA TAXONOMY, AS A DECIDED PROPERTY** — owner rulings,
//! 2026-08-11; `docs/design/gpga_region_kind.md`.
//!
//! > 1. *"A GPGA region is exactly ONE of four kinds, DECIDED AT ALLOCATION/BIND, not
//! >    derived later: unallocated / fake framebuffer / real GPU memory /
//! >    DMA-to-guest-physical."*
//! > 3. *"no fake FB ever can be mapped to a real GPU VA of an isolate except the
//! >    scratchpad."*
//!
//! # ⊘ What this file is FOR, given that the property is a type
//!
//! Most of ruling 3 is enforced by [`kayfabe_mmu::Binding`]'s private fields: there is no
//! spelling of *"fake framebuffer with a host object"* that compiles. A test cannot watch a
//! compile error, so what is asserted here is the part a type cannot state:
//!
//! - the constructors are **total over their input space** and refuse exactly the forbidden
//!   cells — swept, not sampled, so a widening of the guard is caught;
//! - the kind a caller declared is the kind every consumer **reads back**, unchanged;
//! - the decision→executor mapping, which is what the old fall-through got wrong;
//! - ⚠ and **kind 1's hazard**, which is not neutral and is the half most easily forgotten.
//!
//! ★ The integration half — that the one production chain which used to violate ruling 3 is
//! refused by name and does not drop the guest's own row — lives in `fb_leaf_backing.rs`,
//! because it needs a live `SharedDevice` and the real plan/execute/commit spine.

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{GpuId, GpuVa, Pdb};
use kayfabe_isolate::{HostHandle, IsolateId};
use kayfabe_mmu::{AddressTable, BackingBytes, Binding, HostBacking, RegionKind, RegionKindFault};

const GPU: GpuId = GpuId::ZERO;
const PDB: Pdb = Pdb(0x340_1000);
const VA: GpuVa = GpuVa(0x2_0020_0000);
const LEN: u64 = 0x1000;

/// Every aperture the tree has. Swept rather than sampled: a guard that widens to admit one
/// more aperture must fail here, and a three-case `match` in a test cannot notice a fourth.
const APERTURES: [Aperture; 4] = [
    Aperture::Vidmem,
    Aperture::SysmemCoherent,
    Aperture::SysmemNonCoherent,
    Aperture::Peer,
];

/// Every `BackingBytes` the tree has, swept for the same reason [`APERTURES`] is: a guard
/// that widens to admit one more declaration must fail here.
const BYTES: [BackingBytes; 3] = [
    BackingBytes::SoleBacking,
    BackingBytes::ShadowsGuestMemory,
    BackingBytes::JoinsGuestWindow,
];

fn backing(bytes: BackingBytes) -> HostBacking {
    HostBacking::whole(
        HostHandle::new(IsolateId::new(1, GPU), 9),
        // Address identity: a host-backed binding's host VA IS the VA it is bound at.
        VA.0,
        bytes,
    )
}

// =====================================================================================
// 1 — THE DECISION
// =====================================================================================

/// ★★★ **The guest's own aperture decides kinds 2 and 4, and `Peer` decides NOTHING.**
///
/// ⊘ This is not the old fall-through wearing a new name, and the difference is where it is
/// asked. The fall-through was *"a binding exists and has no host object, therefore
/// fiction"* — asked at classify time, by a consumer with no idea who bound it, and it
/// swallowed [`Aperture::Peer`] silently into `Representability::Fabricated`. `[measured
/// 2026-08-11, gpga_region_kind.md §1.1]` This is asked at bind time, of the only authority
/// Mode 2 has (§0.1: the guest's stock RM allocates over the framebuffer we advertise and
/// never asks us), and it **refuses** the aperture no kind describes.
#[test]
fn the_guests_declared_aperture_decides_the_kind_and_peer_is_refused() {
    let seen: Vec<Result<RegionKind, RegionKindFault>> = APERTURES
        .iter()
        .map(|&a| Binding::declared_by_guest(0x1000_0000, a).map(|b| b.kind()))
        .collect();
    assert_eq!(
        seen,
        vec![
            Ok(RegionKind::FakeFramebuffer),
            Ok(RegionKind::GuestPhysDma),
            Ok(RegionKind::GuestPhysDma),
            Err(RegionKindFault::PeerHasNoKind),
        ],
        "★ `Vidmem` is the framebuffer we fabricate (kind 2); sysmem is the guest's own \
         physical pages (kind 4); `Peer` is a second GPU's framebuffer, which this device \
         does not back and no kind describes — refused, never fabricated"
    );
}

/// ★★★★★ **RULING 3, SWEPT: every cell of the host-backing input space, and the forbidden
/// ones refuse.**
///
/// ⊘ Two independent spellings of *"fake framebuffer at a real GPU VA"*, and they fail
/// **independently**, which is why both are here and why the sweep is exhaustive:
///
/// | the caller | caught by |
/// |---|---|
/// | honest about the address, silent about the shadow | the [`Aperture::Vidmem`] test |
/// | honest about the shadow, over an innocent aperture | [`BackingBytes::ShadowsGuestMemory`] |
///
/// ★ A single test would let either half be deleted while the other kept the file green.
/// `[measured 2026-08-11, w228]` the chain that used to build this state produced
/// `placed_as_asked=true` **and blank** — self-concealing in a boot, which is why it must be
/// caught at construction.
///
/// # ★★★ THE CARVE-OUT, and why it does not weaken either spelling
///
/// [`BackingBytes::JoinsGuestWindow`] is ruling **4** — the scratchpad — and it is the only
/// cell of this sweep that admits [`Aperture::Vidmem`]. It changes neither test above:
/// `Vidmem` + `SoleBacking` is still refused (a silent caller gains nothing) and
/// `ShadowsGuestMemory` is still refused under every aperture (an honest shadow gains
/// nothing). ⇒ The admit is bought by a **third, distinct declaration**, not by relaxing the
/// aperture test — which is the shape that would re-open `w228`'s *"two memories"* chain.
#[test]
fn ruling_3_refuses_every_fake_fb_at_a_real_gpu_va_and_admits_everything_else() {
    let mut admitted = 0;
    let mut refused = 0;
    for &aperture in &APERTURES {
        for &bytes in &BYTES {
            let got = Binding::real_gpu_memory(0x1000_0000, aperture, backing(bytes));
            let expected: Result<RegionKind, RegionKindFault> = match (aperture, bytes) {
                (Aperture::Peer, _) => Err(RegionKindFault::PeerHasNoKind),
                (_, BackingBytes::ShadowsGuestMemory) => {
                    Err(RegionKindFault::FakeFbAtRealGpuVa { aperture })
                }
                // ★★★ The carve-out, and it is the ONLY cell in which a `Vidmem` aperture is
                // admitted. See `BackingBytes::JoinsGuestWindow`.
                (_, BackingBytes::JoinsGuestWindow) => Ok(RegionKind::RealGpuMemory),
                (Aperture::Vidmem, _) => Err(RegionKindFault::FakeFbAtRealGpuVa { aperture }),
                _ => Ok(RegionKind::RealGpuMemory),
            };
            assert_eq!(
                got.map(|b| b.kind()),
                expected,
                "aperture={aperture:?} bytes={bytes:?}"
            );
            if got.is_ok() {
                admitted += 1;
            } else {
                refused += 1;
            }
        }
    }
    // ★ Non-vacuity, in both directions: a guard that refused everything and a guard that
    // refused nothing would each satisfy a one-sided sweep.
    assert_eq!(
        (admitted, refused),
        (5, 7),
        "★ the sweep must observe BOTH answers — five admitted cells (the two sysmem \
         apertures with a sole backing, and the three non-`Peer` apertures with a joined \
         one) and seven refused"
    );
}

/// ★★★★★ **THE CARVE-OUT IS BOUGHT BY THE DECLARATION, NOT BY THE APERTURE** — the mutant
/// this file exists to kill.
///
/// ⊘ The tempting repair for the framebuffer join was *"stop refusing on the aperture"*, and
/// it is the one repair that must not be made: it would re-admit `Vidmem` +
/// [`BackingBytes::SoleBacking`], which is `w228`'s chain wearing an innocent word, and
/// `Vidmem` + [`BackingBytes::ShadowsGuestMemory`], which is that chain saying so out loud.
/// The sweep above would still pass an aperture-blind guard on nine of its twelve cells, so
/// the three that separate the two designs are asserted here **by name**.
#[test]
fn only_the_join_admits_a_vidmem_aperture_and_the_other_two_declarations_still_refuse() {
    for bytes in [BackingBytes::SoleBacking, BackingBytes::ShadowsGuestMemory] {
        assert_eq!(
            Binding::real_gpu_memory(0x1000_0000, Aperture::Vidmem, backing(bytes)).err(),
            Some(RegionKindFault::FakeFbAtRealGpuVa {
                aperture: Aperture::Vidmem
            }),
            "★ ruling 3 stands for {bytes:?}: the emulated framebuffer is the only video \
             memory in this design, so a host object at a `Vidmem` address is a SECOND \
             memory unless the caller declares the window itself was re-pointed"
        );
    }
    let joined = Binding::real_gpu_memory(
        0x1000_0000,
        Aperture::Vidmem,
        backing(BackingBytes::JoinsGuestWindow),
    )
    .expect("★ ruling 4 — an OS_DESCRIPTOR over host pages the guest's own window now maps");
    assert_eq!(joined.kind(), RegionKind::RealGpuMemory);
    assert_eq!(
        joined.aperture(),
        Aperture::Vidmem,
        "⊘ the aperture is NOT corrected to sysmem. It records what the GUEST declared, and \
         `Binding::phys` is a framebuffer offset — calling it sysmem would make \
         `is_guest_ram()` true of a number `Vmm::gpa_read` must never be handed, and would \
         route the CPU plane to `GuestRam` when the joined bytes are reached through the \
         framebuffer store"
    );
    assert!(
        !joined.is_guest_ram(),
        "★ the consequence, asserted rather than assumed"
    );
    assert!(
        BackingBytes::JoinsGuestWindow.dissolves_fake_framebuffer(),
        "and the carve-out is read from ONE place"
    );
    for bytes in [BackingBytes::SoleBacking, BackingBytes::ShadowsGuestMemory] {
        assert!(!bytes.dissolves_fake_framebuffer(), "{bytes:?}");
    }
}

/// ★★★ **A kind-3 binding cannot exist without its object, and a kind-2 binding cannot
/// acquire one.**
///
/// ⊘ The old shape made both writable. `Binding` was a struct literal with a public
/// `host: Option<HostBacking>`, so *"real GPU memory, backed by nothing"* — which is exactly
/// what the `Fabricated` fall-through meant — and *"fake framebuffer, backed by a host
/// object"* — ruling 3's forbidden state — were both one keystroke away at every bind site.
#[test]
fn a_kinds_relationship_to_a_host_object_is_settled_by_its_kind() {
    for &aperture in &APERTURES {
        let Ok(declared) = Binding::declared_by_guest(0x1000_0000, aperture) else {
            continue;
        };
        assert!(
            declared.host().is_none(),
            "a guest declaration names no host object: {aperture:?}"
        );
        assert!(
            !declared.kind().may_be_host_mapped() || declared.kind() == RegionKind::GuestPhysDma,
            "only kind 2 is forbidden a host mapping"
        );
    }
    let real = Binding::real_gpu_memory(
        0x1000_0000,
        Aperture::SysmemCoherent,
        backing(BackingBytes::SoleBacking),
    )
    .expect("kind 3");
    assert!(
        real.host().is_some(),
        "kind 3 carries its object — `real_gpu_memory` takes it as an argument, so \
         `RealGpuMemory` with no backing is not a state anyone can write down"
    );
    assert!(
        !RegionKind::FakeFramebuffer.may_be_host_mapped(),
        "★ ruling 3 as a total function, and it is what `real_gpu_memory` consults"
    );
}

// =====================================================================================
// 2 — THE READ-BACK, and kind 1
// =====================================================================================

/// ★★ **The kind a bind declared is the kind the table hands back** — unchanged by the trip
/// through [`AddressTable::bind`]/[`AddressTable::resolve`].
///
/// ⊘ Worth asserting rather than assuming: the whole defect was that the kind was *recomputed
/// downstream* from whatever happened to be in the binding. A table that stored the kind and
/// a consumer that still derived it would look identical to this one until the two answers
/// diverged.
#[test]
fn the_declared_kind_survives_the_table() {
    for (aperture, want) in [
        (Aperture::Vidmem, RegionKind::FakeFramebuffer),
        (Aperture::SysmemCoherent, RegionKind::GuestPhysDma),
    ] {
        let mut t = AddressTable::new();
        t.bind(
            PDB,
            VA,
            LEN,
            Binding::declared_by_guest(0x1000_0000, aperture).expect("declarable"),
        )
        .expect("binds");
        assert_eq!(t.kind_at(VA), Some(want), "{aperture:?}");
        assert_eq!(
            t.resolve(PDB, VA).expect("resolves").0.kind(),
            want,
            "and `resolve` agrees with `kind_at` — one stored fact, two readers"
        );
    }
}

/// ★★★★★ **KIND 1 IS THE ABSENCE OF A ROW — and it is NOT NEUTRAL.**
///
/// `[measured 2026-08-11, gpga_region_kind.md §1.1]` there were **two** derived defaults and
/// they pointed opposite ways: a bound range with no host object fell through to
/// `Representability::Fabricated` (our CPU executor), while a range with **no row at all**
/// became `Representability::Untracked` — which routes to the **real host GPU**.
///
/// (A) removes the first. It does **not** remove the second, and this test is where that is
/// said out loud: deciding the kind at bind fixes what a BOUND range means and can say
/// nothing about a range nobody bound. ⇒ *"the table is incomplete"* is never merely a
/// missing diagnostic; every VA the table does not bind is a VA the classifier hands to a
/// real engine on the strength of knowing nothing about it.
#[test]
fn kind_1_is_the_absence_of_a_row_and_it_routes_to_hardware() {
    let mut t = AddressTable::new();
    assert_eq!(
        t.kind_at(VA),
        None,
        "★ kind 1 — unallocated — is `None`, the SAME `None` `binding_at` gives. A \
         `RegionKind::Unallocated` variant would be a second spelling of one fact"
    );
    assert_eq!(t.binding_at(VA), None, "and the two agree by construction");

    t.bind(
        PDB,
        VA,
        LEN,
        Binding::declared_by_guest(0x1000_0000, Aperture::Vidmem).expect("kind 2"),
    )
    .expect("binds");
    assert_eq!(t.kind_at(VA), Some(RegionKind::FakeFramebuffer));
    // …and unbinding puts it back to kind 1, so the absence is reachable in both directions.
    t.unbind(VA);
    assert_eq!(t.kind_at(VA), None);
}

// =====================================================================================
// 3 — THE DECISION → EXECUTOR MAPPING
// =====================================================================================

/// ★★★ **Which engine each decided kind selects** — the mapping the old fall-through got
/// wrong in both directions.
///
/// ⚠ Read the two `Ours` arms and the `Untracked` arm together, because the pairing is
/// counter-intuitive and was measured rather than reasoned: `[measured 2026-08-11, boots
/// w234a/w234b]` the user proc's framebuffer ranges had **no binding at all** — the hardware
/// arm — until the executor-write witness gave them one, which took them to `Fabricated`,
/// i.e. to the CPU executor addressing the emulated framebuffer the guest actually reads.
/// ⇒ **Populating the address table moves work OFF the hardware arm, not onto it.**
#[test]
fn the_decided_kind_selects_the_executor_and_an_absent_row_selects_hardware() {
    use kayfabe_fwd::{CeExecutor, Representability};
    assert_eq!(
        Representability::HostBacked.executor(),
        CeExecutor::HostCe,
        "kind 3 — real host memory at the guest's own VA — is the only kind a real engine \
         may be pointed at"
    );
    assert_eq!(
        Representability::Fabricated.executor(),
        CeExecutor::Ours,
        "kinds 2 and 4 are ours to execute; their CPU plane is carried beside them"
    );
    assert_eq!(
        Representability::Untracked.executor(),
        CeExecutor::HostCe,
        "⚠ kind 1 — nobody decided — routes to the REAL HOST GPU. The only thing behind \
         that is the #14 ring gate. This is the second derived default, and (A) does not \
         close it."
    );
}
