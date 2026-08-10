//! ★★★ `crate::ceresolve` — the walk from a page-directory root the GUEST PUBLISHED.
//!
//! # ⊘ What these tests are written to catch, in the order it would bite
//!
//! The resolver's failure modes are all *silent*: it either answers the wrong address or it
//! answers the right one for the wrong VA space, and in both cases everything downstream
//! looks healthy. So the assertions here are on **exact variants** and on the *distinction*
//! between two near-neighbour findings, never on `is_some()`
//! (`docs/design/testing_doctrine.md`).
//!
//! The format below is a deliberately tiny two-level one rather than GA10x's: this crate
//! does not depend on `kayfabe-chips`, and — more to the point — a test that borrowed the
//! real format would be asserting the *format's* behaviour again instead of the resolver's.
//! `crates/kayfabe-chips/tests/ga10x_gmmu.rs` owns the format; this owns the walk's
//! preconditions and its refusal vocabulary.

use kayfabe_abi::gvaspacepdes::{GMMU_FMT_MAX_LEVELS, PdeLevel, ServerReservedPdes};
use kayfabe_arch::{Aperture, GmmuFmt, GmmuVersion, LevelShift, PageSize, PdeEdge, PteDecode};
use kayfabe_device::ceresolve::{
    CeAddrLimits, CeResolve, Demand, GMMU_APERTURE_INVALID, GMMU_APERTURE_SYS_COH,
    GMMU_APERTURE_VIDEO, published_root, resolve,
};
use kayfabe_device::gvaspub::{GvasPubLog, GvasPublication};
use kayfabe_mmu::walker::FbRead;

/// A two-level format: level 0 indexes bits `[30:21]` (512 slots of 8 bytes), level 1 is a
/// 2 MiB leaf table. Entries are `[valid:1][sparse:1][aperture_sys:1][addr << 12]`.
#[derive(Debug)]
struct TinyFmt;

const E_VALID: u128 = 1;
const E_SPARSE: u128 = 2;
const E_SYS: u128 = 4;

/// The address field holds the page frame, so an entry naming byte address `a` stores
/// `a >> 12` at bit 12 — the same shape every real format has, and the reason `decode_entry`
/// shifts it back rather than returning the field.
fn pde(next: u64) -> u128 {
    E_VALID | (u128::from(next >> 12) << 12)
}
fn leaf(phys: u64) -> u128 {
    E_VALID | (u128::from(phys >> 12) << 12)
}
fn leaf_sys(phys: u64) -> u128 {
    E_VALID | E_SYS | (u128::from(phys >> 12) << 12)
}

impl GmmuFmt for TinyFmt {
    fn version(&self) -> GmmuVersion {
        GmmuVersion::Ver2
    }
    fn page_sizes(&self) -> &[PageSize] {
        &[PageSize(2 << 20)]
    }
    fn entry_size(&self, level: u8) -> u8 {
        if level < 2 { 8 } else { 0 }
    }
    fn levels(&self) -> u8 {
        2
    }
    fn level_shift(&self, level: u8) -> Option<LevelShift> {
        match level {
            0 => Some(LevelShift {
                shift: 30,
                entries: 512,
            }),
            1 => Some(LevelShift {
                shift: 21,
                entries: 512,
            }),
            _ => None,
        }
    }
    fn decode_entry(&self, level: u8, raw: u128) -> PteDecode {
        if raw & E_SPARSE != 0 {
            return PteDecode::Sparse;
        }
        if raw & E_VALID == 0 {
            return PteDecode::Invalid;
        }
        let phys = ((raw >> 12) as u64) << 12;
        let aperture = if raw & E_SYS != 0 {
            Aperture::SysmemCoherent
        } else {
            Aperture::Vidmem
        };
        if level == 0 {
            PteDecode::Pde {
                edge: PdeEdge {
                    next: phys,
                    aperture,
                    child_level: 1,
                },
                also: None,
            }
        } else {
            PteDecode::Leaf {
                phys,
                aperture,
                size: PageSize(2 << 20),
                read_only: false,
            }
        }
    }
}

/// A flat byte image standing in for the framebuffer the guest wrote its tables into.
struct Fb(Vec<u8>);

impl Fb {
    fn new() -> Fb {
        Fb(vec![0u8; 0x8000])
    }
    fn put(&mut self, at: u64, e: u128) {
        let at = at as usize;
        self.0[at..at + 8].copy_from_slice(&(e as u64).to_le_bytes());
    }
}

impl FbRead for Fb {
    fn read(&mut self, phys: u64, buf: &mut [u8]) -> bool {
        let at = phys as usize;
        match self.0.get(at..at + buf.len()) {
            Some(s) => {
                buf.copy_from_slice(s);
                true
            }
            None => false,
        }
    }
}

const ROOT: u64 = 0x1000;
const L1: u64 = 0x2000;
/// The VA the tree below maps: root slot 1 (`va >> 30`), leaf slot 2 (`(va >> 21) & 511`).
const VA: u64 = (1 << 30) | (2 << 21) | 0x1234;
const LEAF_PHYS: u64 = 0x4000;

fn tree() -> Fb {
    let mut fb = Fb::new();
    fb.put(ROOT + 8, pde(L1));
    fb.put(L1 + 16, leaf(LEAF_PHYS));
    fb
}

fn root_at(phys: u64, aperture: u32, page_shift: u8) -> kayfabe_device::ceresolve::VasRoot {
    kayfabe_device::ceresolve::VasRoot {
        phys,
        aperture: kayfabe_device::ceresolve::decode_aperture(aperture),
        aperture_raw: aperture,
        page_shift,
        virt_addr_lo: 0,
        virt_addr_hi: 0,
    }
}

const LIMITS: CeAddrLimits = CeAddrLimits {
    fb_len: 0x8000,
    gpa_limit: None,
};

// =====================================================================================
// The walk itself
// =====================================================================================

#[test]
fn a_published_root_resolves_a_va_two_levels_down_with_its_page_offset_applied() {
    let r = resolve(
        &TinyFmt,
        &mut tree(),
        &root_at(ROOT, GMMU_APERTURE_VIDEO, 30),
        VA,
        LIMITS,
        Demand::from_doorbell(),
    );
    assert_eq!(
        r,
        CeResolve::Resolved {
            // ★ The offset within the 2 MiB page is part of the answer, not the caller's
            // job — a caller that had to add it could add it twice.
            phys: LEAF_PHYS + 0x1234,
            aperture: Aperture::Vidmem,
            page_size: 2 << 20,
            read_only: false,
            level: 1,
        },
        "the whole point of the increment: the guest's own root resolves the guest's own VA"
    );
}

#[test]
fn the_leaf_aperture_is_carried_and_is_not_assumed_to_be_the_roots() {
    // ⚠ `c_ceutils_ring_resolution.md` §4: a CeUtils finishPayload was measured in EACH
    // aperture within ONE run. A resolver that reported the root's aperture for the leaf
    // would answer a guest-RAM address out of a framebuffer.
    let mut fb = tree();
    fb.put(L1 + 16, leaf_sys(LEAF_PHYS));
    let r = resolve(
        &TinyFmt,
        &mut fb,
        &root_at(ROOT, GMMU_APERTURE_VIDEO, 30),
        VA,
        LIMITS,
        Demand::from_doorbell(),
    );
    assert!(
        matches!(
            r,
            CeResolve::Resolved {
                aperture: Aperture::SysmemCoherent,
                ..
            }
        ),
        "a sysmem leaf under a vidmem root must report SYSMEM, got {r:?}"
    );
}

#[test]
fn an_unmapped_slot_is_a_fault_and_not_a_zero() {
    // MISS = FAULT. An all-zero entry is what `MMU_WALK_FILL_INVALID` writes, and reading
    // it as "physical address zero" is the first page of the framebuffer.
    let r = resolve(
        &TinyFmt,
        &mut Fb::new(),
        &root_at(ROOT, GMMU_APERTURE_VIDEO, 30),
        VA,
        LIMITS,
        Demand::from_doorbell(),
    );
    assert!(
        matches!(r, CeResolve::Fault(_)),
        "expected a fault, got {r:?}"
    );
    assert_eq!(r.place(), None, "a refusal must not present as an address");
}

#[test]
fn sparse_and_unmapped_are_distinguishable_findings() {
    // §7 rule 4. Two different bugs live in conflating them, so the report must tell them
    // apart — `describe()` is what a boot prints and it is where the distinction survives.
    let mut sparse = tree();
    sparse.put(L1 + 16, E_SPARSE);
    let s = resolve(
        &TinyFmt,
        &mut sparse,
        &root_at(ROOT, GMMU_APERTURE_VIDEO, 30),
        VA,
        LIMITS,
        Demand::from_doorbell(),
    );
    let mut unmapped = tree();
    unmapped.put(L1 + 16, 0);
    let u = resolve(
        &TinyFmt,
        &mut unmapped,
        &root_at(ROOT, GMMU_APERTURE_VIDEO, 30),
        VA,
        LIMITS,
        Demand::from_doorbell(),
    );
    assert!(matches!(s, CeResolve::Fault(_)) && matches!(u, CeResolve::Fault(_)));
    assert_ne!(s, u, "sparse and unmapped must not collapse to one value");
    assert_ne!(s.describe(), u.describe(), "…nor to one sentence");
    assert_ne!(s.tag(), u.tag(), "…nor to one compact tag");
}

// =====================================================================================
// The preconditions this module owns
// =====================================================================================

#[test]
fn a_sysmem_rooted_publication_is_refused_and_never_read_out_of_the_framebuffer() {
    // ⚠ MEASURED on a real GA106 (2026-07-25): a sysmem-rooted PDB was an executing
    // channel's own root. Walking it against `fb` would read a page of the framebuffer that
    // merely shares the number, and answer confidently.
    let mut fb = tree();
    let r = resolve(
        &TinyFmt,
        &mut fb,
        &root_at(ROOT, GMMU_APERTURE_SYS_COH, 30),
        VA,
        LIMITS,
        Demand::from_doorbell(),
    );
    assert_eq!(
        r,
        CeResolve::RootAperture {
            raw: GMMU_APERTURE_SYS_COH
        }
    );
}

#[test]
fn gmmu_aperture_video_is_one_and_the_constants_are_the_enums_own_order() {
    // ⚠⚠ MEASURED 2026-08-08 (boot `run_p35_a34025b`): these were assumed from the PDE
    // *field* encoding and every value but INVALID was wrong, so the walling channel's own
    // `aperture 1` root was refused as PEER. The enum is unnumbered
    // (`ogkm-580: gmmu_fmt.h:280-325`), so its ORDER is the encoding — and SYS_NONCOH
    // precedes SYS_COH, which is the reverse of every other list in this port.
    assert_eq!(GMMU_APERTURE_INVALID, 0);
    assert_eq!(GMMU_APERTURE_VIDEO, 1);
    assert_eq!(kayfabe_device::ceresolve::GMMU_APERTURE_PEER, 2);
    assert_eq!(kayfabe_device::ceresolve::GMMU_APERTURE_SYS_NONCOH, 3);
    assert_eq!(GMMU_APERTURE_SYS_COH, 4);
    assert_eq!(
        kayfabe_device::ceresolve::decode_aperture(GMMU_APERTURE_VIDEO),
        Some(Aperture::Vidmem),
        "the value a real GA106 publishes for a GSP-managed page directory"
    );
}

#[test]
fn gmmu_aperture_invalid_is_a_declaration_and_decodes_to_no_aperture() {
    // ⊘ Zero is *"this sub-level is absent"* (`gmmu_fmt.h:281-285`), not video memory. A
    // decoder that answered `Vidmem` for it would walk a root the guest said is not there.
    assert_eq!(
        kayfabe_device::ceresolve::decode_aperture(GMMU_APERTURE_INVALID),
        None
    );
    let r = resolve(
        &TinyFmt,
        &mut tree(),
        &root_at(ROOT, GMMU_APERTURE_INVALID, 30),
        VA,
        LIMITS,
        Demand::from_doorbell(),
    );
    assert_eq!(
        r,
        CeResolve::RootAperture {
            raw: GMMU_APERTURE_INVALID
        }
    );
}

#[test]
fn an_undefined_root_aperture_reports_the_number_the_guest_actually_sent() {
    let r = resolve(
        &TinyFmt,
        &mut tree(),
        &root_at(ROOT, 9, 30),
        VA,
        LIMITS,
        Demand::from_doorbell(),
    );
    assert_eq!(
        r,
        CeResolve::RootAperture { raw: 9 },
        "an aperture the header does not define must be reported verbatim, not as a decoded \
         substitute — the number is the diagnosis"
    );
}

#[test]
fn the_start_level_comes_from_the_published_page_shift_and_a_wrong_one_is_refused() {
    // ⊘ Not assumed to be zero. `bar2_phys` derives BAR2's level the same way and for the
    // same reason: the entry alone does not say which format row it belongs to.
    let r = resolve(
        &TinyFmt,
        &mut tree(),
        &root_at(ROOT, GMMU_APERTURE_VIDEO, 47),
        VA,
        LIMITS,
        Demand::from_doorbell(),
    );
    assert_eq!(r, CeResolve::NoRootLevel { page_shift: 47 });
}

#[test]
fn a_page_shift_naming_the_leaf_level_starts_there_rather_than_at_the_root() {
    // The derivation is a real lookup, not a `!= 0` check: seeded at shift 21 the walk
    // starts at level 1, so a root address pointing straight at a leaf table resolves.
    let r = resolve(
        &TinyFmt,
        &mut tree(),
        &root_at(L1, GMMU_APERTURE_VIDEO, 21),
        VA,
        LIMITS,
        Demand::from_doorbell(),
    );
    assert!(
        matches!(r, CeResolve::Resolved { phys, level: 1, .. } if phys == LEAF_PHYS + 0x1234),
        "got {r:?}"
    );
}

#[test]
fn a_leaf_at_or_beyond_the_framebuffer_limit_is_refused_as_a_torn_entry() {
    // §7 rule 5, the cheap torn-read detector: a stale high half moves the address out of
    // range. ⊘ It must be a REFUSAL — an out-of-range address that reads as a resolution is
    // §6.2(1)'s wrong physical page.
    let mut fb = tree();
    fb.put(L1 + 16, leaf(0x8000));
    let r = resolve(
        &TinyFmt,
        &mut fb,
        &root_at(ROOT, GMMU_APERTURE_VIDEO, 30),
        VA,
        LIMITS,
        Demand::from_doorbell(),
    );
    assert_eq!(
        r,
        CeResolve::AddressOutOfRange {
            phys: 0x8000 + 0x1234,
            aperture: Aperture::Vidmem,
            limit: 0x8000
        }
    );
}

#[test]
fn a_sysmem_leaf_is_not_bounded_by_the_framebuffer_limit() {
    // ⊘ The two number spaces are unrelated; bounding a guest-physical address by the
    // framebuffer's length would refuse every sysmem mapping above the FB size, which is
    // most of guest RAM. `CeAddrLimits::gpa_limit` is `None` and the module says so.
    let mut fb = tree();
    fb.put(L1 + 16, leaf_sys(0x40000));
    let r = resolve(
        &TinyFmt,
        &mut fb,
        &root_at(ROOT, GMMU_APERTURE_VIDEO, 30),
        VA,
        LIMITS,
        Demand::from_doorbell(),
    );
    assert!(
        matches!(
            r,
            CeResolve::Resolved {
                aperture: Aperture::SysmemCoherent,
                ..
            }
        ),
        "got {r:?}"
    );
}

// =====================================================================================
// Choosing the root — the join that makes the walk legitimate
// =====================================================================================

fn publication(client: u32, object: u32, phys: u64) -> GvasPublication {
    let mut levels = [PdeLevel {
        phys_address: 0,
        size: 0,
        aperture: 0,
        page_shift: 0,
    }; GMMU_FMT_MAX_LEVELS];
    levels[0] = PdeLevel {
        phys_address: phys,
        size: 0x20,
        aperture: GMMU_APERTURE_VIDEO,
        page_shift: 47,
    };
    GvasPublication {
        cmd: 0x90f1_0106,
        client,
        object,
        pdes: ServerReservedPdes {
            h_subdevice: 0,
            subdevice_id: 0,
            page_size: 0x20_0000,
            virt_addr_lo: 0x1_0000_0000,
            virt_addr_hi: 0x1_1fff_ffff,
            num_levels: 4,
            levels,
        },
        count: 1,
    }
}

#[test]
fn the_root_is_keyed_on_hobject_as_well_as_hclient() {
    // ★★★ §14.10 decision 2. One client publishing two VA spaces is the MEASURED shape of
    // the boot this exists for; a lookup on `hClient` alone answers with whichever it
    // published last — an address space that is not the channel's.
    let log = GvasPubLog::new();
    log.note(publication(0xc1e0_0006, 0x0a, 0x2efa_9000));
    log.note(publication(0xc1e0_0006, 0x0c, 0xdead_0000));
    let snap = log.snapshot();
    assert_eq!(
        published_root(&snap, 0xc1e0_0006, 0x0a).unwrap().phys,
        0x2efa_9000
    );
    assert_eq!(
        published_root(&snap, 0xc1e0_0006, 0x0c).unwrap().phys,
        0xdead_0000
    );
}

#[test]
fn a_pair_that_published_nothing_gets_no_root_and_no_neighbours() {
    // ⊘ The C's resolver ladder ended in a blind any-VAS probe and it collapsed two
    // clients' semaphores onto one page (`c_ceutils_ring_resolution.md` §5.9). There is no
    // fallback here: the absence is the answer.
    let log = GvasPubLog::new();
    log.note(publication(0xc1e0_0006, 0x0a, 0x2efa_9000));
    let snap = log.snapshot();
    assert_eq!(
        published_root(&snap, 0xc1e0_0005, 0x0a),
        None,
        "wrong client must not match"
    );
    assert_eq!(
        published_root(&snap, 0xc1e0_0006, 0x0b),
        None,
        "wrong object must not match"
    );
    assert_eq!(published_root(&snap, 0, 0), None);
}

#[test]
fn a_republication_of_one_pair_wins_over_the_earlier_tree() {
    // A VA space torn down and rebuilt differs in nothing but arrival order, and the
    // current tree is the later one.
    let log = GvasPubLog::new();
    log.note(publication(0xc1e0_0006, 0x0a, 0x1111_0000));
    log.note(publication(0xc1e0_0006, 0x0a, 0x2222_0000));
    assert_eq!(
        published_root(&log.snapshot(), 0xc1e0_0006, 0x0a)
            .unwrap()
            .phys,
        0x2222_0000
    );
}

#[test]
fn a_publication_carries_its_root_aperture_and_page_shift_through_unchanged() {
    let log = GvasPubLog::new();
    let mut p = publication(0xc1e0_0006, 0x0a, 0x2efa_9000);
    p.pdes.levels[0].aperture = GMMU_APERTURE_SYS_COH;
    p.pdes.levels[0].page_shift = 47;
    log.note(p);
    let r = published_root(&log.snapshot(), 0xc1e0_0006, 0x0a).unwrap();
    assert_eq!(r.aperture, Some(Aperture::SysmemCoherent));
    assert_eq!(r.aperture_raw, GMMU_APERTURE_SYS_COH);
    assert_eq!(r.page_shift, 47);
}

// =====================================================================================
// §16.10 — THE PER-LEVEL TRACE: which SLOT the descent consumes, and what it says
// =====================================================================================

/// A tree in which **entry 0 is populated and is NOT the entry the walk consumes** — the
/// real boot's shape, and the only shape in which this test can fail.
///
/// ⊘⊘ **Written because the first version could not fail.** The bite-check for §16.10's
/// selection rule — replace *"the covering slot"* with `children.first()`, i.e. exactly what
/// §16.9's dump read — left the test **GREEN**, because `tree()` puts one child on the root
/// and one leaf on the L1 page, so entry 0 *was* the covering slot. A fixture with one
/// candidate cannot distinguish a selector from a constant. `[measured 2026-08-09, boot
/// `fbd1_f760a4b`]` the guest's real tree is the opposite: entry 0 is written at every level
/// **and** the ring's level-2 index is 9. So both levels here carry a **decoy at slot 0**
/// and the real edge further along.
const DECOY_L1: u64 = 0x3000;
const DECOY_LEAF: u64 = 0x6000;

fn tree_with_decoys() -> Fb {
    let mut fb = Fb::new();
    fb.put(ROOT, pde(DECOY_L1)); // slot 0 — populated, and NOT on the path.
    fb.put(ROOT + 8, pde(L1)); // slot 1 — `VA >> 30`.
    fb.put(L1, leaf(DECOY_LEAF)); // slot 0 — populated, and NOT on the path.
    fb.put(L1 + 16, leaf(LEAF_PHYS)); // slot 2 — `(VA >> 21) & 511`.
    fb
}

/// ★★★★ **The trace names the slot the walk actually uses at every level, and its terminal
/// answer AGREES with `resolve`'s.**
///
/// `[measured 2026-08-09, boot `fbd1_f760a4b`]` §16.9 dumped **entry 0** of each published
/// level of the refusing VA space and found real, well-formed PDEs — and entry 0 is not the
/// entry that walk consumes (`(0x121010000 >> 29) & 0x1ff = 9`). ⇒ every byte that dump read
/// was a byte the failing walk never looks at.
///
/// ⊘ The load-bearing property is **agreement**: the trace selects its slot from
/// `decode_page`'s own `vabase` stamps, and if that selection ever disagreed with
/// `walker::translate`'s descent the trace would be a second projection of one fact — §16.2
/// wall 1, which cost a boot. Asserted here rather than hoped for.
#[test]
fn the_walk_trace_names_the_consumed_slot_at_each_level_and_ends_where_resolve_does() {
    let t = kayfabe_device::ceresolve::walk_trace(
        &TinyFmt,
        &mut tree_with_decoys(),
        &root_at(ROOT, GMMU_APERTURE_VIDEO, 30),
        VA,
    );
    // Two children on the root, and the walk must take the SECOND — slot 1, `VA >> 30`.
    assert!(
        t.contains(&format!("L0@0x{ROOT:x}/by?[ch2 lf0 sp0 inv510]")),
        "the level's whole census, so 'how many entries are written' is visible — and `/by?` \
         because this fixture's store cannot answer who wrote a page, which is ⊘ NOT the \
         same as saying nobody did: {t}"
    );
    // ⊘ The decoy must appear NOWHERE as a consumed slot. This is the assertion the
    // bite-check made necessary: without it, `children.first()` passes.
    assert!(
        !t.contains(&format!("->0x{DECOY_L1:x}/")),
        "⊘ entry 0 is populated and is NOT on the path — a selector that took it would be \
         reading the bytes §16.9 read: {t}"
    );
    assert!(
        !t.contains(&format!("->0x{DECOY_LEAF:x}/")),
        "⊘ and the same one level down: {t}"
    );
    assert!(
        t.contains(&format!("=PDE@0x{:x}->0x{L1:x}/Vidmem", 1u64 << 30)),
        "the CONSUMED slot, by its own vabase, not entry 0: {t}"
    );
    assert!(
        t.contains(&format!(
            "=LEAF@0x{:x}->0x{LEAF_PHYS:x}/Vidmem",
            VA & !0x1f_ffff
        )),
        "and the leaf the descent lands on: {t}"
    );
    // ★★★ The agreement property. `resolve` adds the page offset; the trace reports the
    // leaf's base. They must name the same page or one of them is wrong about this device.
    let CeResolve::Resolved { phys, .. } = resolve(
        &TinyFmt,
        &mut tree_with_decoys(),
        &root_at(ROOT, GMMU_APERTURE_VIDEO, 30),
        VA,
        LIMITS,
        Demand::from_doorbell(),
    ) else {
        panic!("the fixture resolves");
    };
    // ⊘ The relation is `resolve = leaf_base + (va & (page_size - 1))`, NOT a mask of
    // `resolve`: this fixture's `LEAF_PHYS` is deliberately **not** 2 MiB-aligned, so
    // masking the resolved address would produce `0` and the assertion would be testing
    // arithmetic instead of agreement. `suspect_the_instrument_first` — the first draft of
    // this line masked, went red, and the code was right.
    let leaf_base = phys - (VA & ((2 << 20) - 1));
    assert!(
        t.contains(&format!("->0x{leaf_base:x}/Vidmem")),
        "⊘ the trace and the resolver must land on the SAME page — two projections of one \
         fact disagreeing is what §16.2 wall 1 cost a boot: trace={t} resolve=0x{phys:x}"
    );
}

/// ⊘ **A slot the guest never wrote is `NO-COVERING-SLOT`, not a leaf and not an error.**
///
/// `PageDecode::invalid` is a **count** with no addresses, so an invalid slot cannot be
/// reported positively — it is reported by the covering slot appearing in none of
/// `children`/`leaves`/`sparse`. That is the observation §16.10 exists to make, and it must
/// be distinguishable from `SPARSE` (the guest declared the range absent) and from
/// `UNREADABLE` (we could not read the page at all).
///
/// ⊘ **The terminal used to be spelled `INVALID-SLOT` and the rename is the point** (§16.73):
/// six sections read that word as *"the VA is unmapped"* when a dual slot made it mean
/// *"not on the branch I chose"*. `NO-COVERING-SLOT` is a statement about **one table**, and
/// the walk's own verdict is `walkend=`.
#[test]
fn a_level_whose_covering_slot_was_never_written_reports_no_covering_slot() {
    let mut fb = Fb::new();
    fb.put(ROOT + 8, pde(L1)); // slot 1 only; the L1 page is left entirely blank.
    let t = kayfabe_device::ceresolve::walk_trace(
        &TinyFmt,
        &mut fb,
        &root_at(ROOT, GMMU_APERTURE_VIDEO, 30),
        VA,
    );
    assert!(
        t.contains("=NO-COVERING-SLOT"),
        "an empty table maps nothing, and that is a finding: {t}"
    );
    assert!(
        !t.contains("=LEAF"),
        "⊘ an unwritten slot must never read as a mapping: {t}"
    );
    // ★ And the WALK's verdict, stated once and apart from the per-level terminal — the
    // sentence a reader is entitled to take as the answer.
    assert!(
        t.ends_with(" walkend=NO-LEAF-ON-ANY-BRANCH"),
        "the verdict must be present and must be the last word: {t}"
    );
    // And the level census says it from the other side: every slot invalid.
    assert!(
        t.contains(&format!("L1@0x{L1:x}/by?[ch0 lf0 sp0 inv512]")),
        "the census must show the page is wholly unwritten: {t}"
    );
}

/// ★★★★ **THE DUAL PDE — the trace must follow BOTH halves, because the resolver does.**
///
/// # ⊘ The defect this is the regression test for, measured before it was written
///
/// `[measured 2026-08-10, boot `w207_4395ebd_real`]` one `RING-PROJ … DESCENT` line carried
/// `rng=S:0x22e86000` — resolved — and, in the same string, `walk: … L4@0x2efa7ef00[ch0 lf0
/// sp0 inv32]=INVALID-SLOT`. Both were honest. GA10x's `L_PD0` slot is **dual**: it names a
/// small-page sub-table *and* a big-page one, `decode_page` stamps **both children with the
/// same `vabase`**, `walker::translate` tries both, and the trace's `max_by_key` followed
/// exactly one (the last maximum — the 32-slot big-page table, 2 MiB ÷ 64 KiB). The ring is
/// mapped by the small-page sibling.
///
/// ⇒ **A tracer whose job is to explain a resolver is defective by construction when it
/// disagrees with it.** The fixture below is the shape the tree could not previously
/// express — `TinyFmt::decode_entry` hard-coded `also: None`, so no test in the workspace
/// ran `walk_trace` over a dual slot at all.
///
/// ★★★★ **It runs the fixture BOTH WAYS, and that is not thoroughness — it is the only
/// arrangement that can fail.** `decode_page` pushes `edge` then `also`, so `max_by_key`
/// returns the **last** maximum: a fixture whose answer sits in `also` is resolved *by the
/// bug*, and a fixture whose answer sits in `edge` is resolved by `children.first()`. ⊘ Either
/// single placement is a test that passes against the defect it names — the shape
/// `injection_measures_necessity_never_sufficiency` warns about, and my first draft of this
/// test had exactly it (answer in `also`; the pre-fix code passed).
#[test]
fn a_dual_slot_is_traced_through_both_halves_and_the_trace_agrees_with_resolve() {
    // `DualFmt` is `TinyFmt` with one difference: level 0's PDE names a second sub-table,
    // derived from the first so the fixture writes ONE entry and gets two children.
    #[derive(Debug)]
    struct DualFmt;
    /// Where the sibling of a level-0 PDE lives, relative to the edge it names.
    const SIB: u64 = 0x1000;
    impl GmmuFmt for DualFmt {
        fn version(&self) -> GmmuVersion {
            TinyFmt.version()
        }
        fn page_sizes(&self) -> &[PageSize] {
            TinyFmt.page_sizes()
        }
        fn entry_size(&self, level: u8) -> u8 {
            TinyFmt.entry_size(level)
        }
        fn levels(&self) -> u8 {
            TinyFmt.levels()
        }
        fn level_shift(&self, level: u8) -> Option<LevelShift> {
            TinyFmt.level_shift(level)
        }
        fn decode_entry(&self, level: u8, raw: u128) -> PteDecode {
            match TinyFmt.decode_entry(level, raw) {
                PteDecode::Pde { edge, also: None } => PteDecode::Pde {
                    // ⊘ The DECOY is `edge` — the half returned FIRST — and the answering
                    // table is `also`. A tracer that takes either "the first" or "the last"
                    // child without descending both fails one of the two.
                    edge: PdeEdge {
                        next: edge.next,
                        ..edge
                    },
                    also: Some(PdeEdge {
                        next: edge.next + SIB,
                        ..edge
                    }),
                },
                other => other,
            }
        }
    }

    // (which half holds the mapping, a name for the failure message)
    for (answering_table, which) in [(L1, "edge (the FIRST half)"), (L1 + SIB, "also (the SECOND half)")] {
        let mut fb = Fb::new();
        fb.put(ROOT + 8, pde(L1)); // slot 1 = `VA >> 30`; names L1 and, dually, L1 + SIB.
        // The other half is left ENTIRELY BLANK — the measured shape of the big-page
        // sibling: present, reachable, and holding nothing for this VA.
        fb.put(answering_table + 16, leaf(LEAF_PHYS));

        let t = kayfabe_device::ceresolve::walk_trace(
            &DualFmt,
            &mut fb,
            &root_at(ROOT, GMMU_APERTURE_VIDEO, 30),
            VA,
        );
        assert!(
            t.contains("/dual1of2"),
            "★ a fork must be NAMED as a fork, so a reader is never shown one branch and \
             told it was the tree — answer in {which}: {t}"
        );
        assert!(
            t.contains(&format!("=LEAF@0x{:x}->0x{LEAF_PHYS:x}/Vidmem", VA & !0x1f_ffff)),
            "★★★ the trace must reach the leaf, whichever half holds it — this is the \
             assertion that goes red if the selection becomes a pick again. Answer in \
             {which}: {t}"
        );
        assert!(
            t.ends_with(" walkend=LEAF"),
            "and the verdict must say the walk answered — answer in {which}: {t}"
        );
        // ★★★ THE AGREEMENT PROPERTY, which is the whole reason this test exists: the
        // tracer and the resolver must not be able to disagree on a dual slot.
        let CeResolve::Resolved { phys, .. } = resolve(
            &DualFmt,
            &mut fb,
            &root_at(ROOT, GMMU_APERTURE_VIDEO, 30),
            VA,
            LIMITS,
            Demand::from_doorbell(),
        ) else {
            panic!(
                "⊘ the RESOLVER must answer here too — if it does not, this fixture is not \
                 the measured shape and the test is about nothing. Answer in {which}"
            );
        };
        let leaf_base = phys - (VA & ((2 << 20) - 1));
        assert!(
            t.contains(&format!("->0x{leaf_base:x}/Vidmem")),
            "⊘ trace and resolver on the SAME page — answer in {which}: trace={t} \
             resolve=0x{phys:x}"
        );
    }
}

/// ⊘⊘ **`vidmem_phys` refuses a SYSMEM leaf, and that refusal is the whole point.**
///
/// A vidmem leaf is an offset into this device's framebuffer; a sysmem leaf is a
/// guest-physical address. The two number spaces collide freely, so a framebuffer reader
/// handed a sysmem answer produces *plausible wrong bytes* — the failure the address plane
/// exists to refuse. `c_ceutils_ring_resolution.md` §4 measured a CeUtils finishPayload in
/// **each** aperture within one run, so neither answer is the safe default.
#[test]
fn vidmem_phys_answers_for_a_framebuffer_leaf_and_declines_for_a_guest_ram_one() {
    let vid = resolve(
        &TinyFmt,
        &mut tree(),
        &root_at(ROOT, GMMU_APERTURE_VIDEO, 30),
        VA,
        LIMITS,
        Demand::from_doorbell(),
    );
    assert_eq!(
        vid.vidmem_phys(),
        Some(LEAF_PHYS + 0x1234),
        "a framebuffer leaf yields its offset, page offset included"
    );

    // The same tree with the leaf re-declared as system memory.
    let mut fb = tree();
    fb.put(L1 + 16, leaf_sys(LEAF_PHYS));
    let sys = resolve(
        &TinyFmt,
        &mut fb,
        &root_at(ROOT, GMMU_APERTURE_VIDEO, 30),
        VA,
        LIMITS,
        Demand::from_doorbell(),
    );
    assert!(
        matches!(sys, CeResolve::Resolved { .. }),
        "the walk still SUCCEEDS — this is not a resolution failure: {sys:?}"
    );
    assert_eq!(
        sys.vidmem_phys(),
        None,
        "⊘ …and yet it yields NO framebuffer offset. A guest-physical address read out of \
         the framebuffer at the same number is the silent-wrong-bytes failure, not an edge \
         case: {sys:?}"
    );

    // ⊘ And a refusal is not an address either.
    assert_eq!(CeResolve::NoPublication.vidmem_phys(), None);
}

// =====================================================================================
// §16.13 — RESIDENCY: the census that separates "never written" from "written with zeros"
// =====================================================================================

/// ★★★★ **A page written with zeros and a page never written read IDENTICALLY, and
/// residency is the only thing that tells them apart.**
///
/// `[measured 2026-08-09, boot `bar1_03a679f`]` the framebuffer page the guest's own page
/// tables name for its GPFIFO ring dumped `nz0/4096` — not one non-zero byte. ⊘ That has two
/// causes and they need different fixes, and `FbStore::read` returns *zero and `Ok`* for an
/// unwritten address inside the aperture, so the byte census **cannot** distinguish them.
#[test]
fn residency_separates_a_page_never_written_from_one_written_with_zeros() {
    use kayfabe_device::fbwin::{FbStore, RefusingFb, SparseFb};

    let mut fb = SparseFb::new(1 << 20);
    fb.write(0x8000, &[0u8; 64]).expect("inside the aperture");

    // Both pages read as all-zero…
    let (mut a, mut b) = ([0u8; 32], [0u8; 32]);
    fb.read(0x8000, &mut a).expect("written with zeros");
    fb.read(0x4000, &mut b).expect("never written");
    assert_eq!(a, b, "⊘ the two pages are INDISTINGUISHABLE by their bytes");

    // …and residency tells them apart.
    assert_eq!(
        fb.is_resident(0x8000),
        Some(true),
        "a page something addressed IS resident, even if every byte it wrote was zero"
    );
    assert_eq!(
        fb.is_resident(0x4000),
        Some(false),
        "a page nothing ever addressed is NOT resident — this is the whole instrument"
    );

    let r = fb.residency().expect("a sparse store can answer");
    assert_eq!(r.pages, 1);
    assert_eq!((r.lo, r.hi), (Some(0x8000), Some(0x8000)));

    // ★★★ THE PRECONDITION, carried and not implied. A store that backs no memory has no
    // residency to report, and `Some(0)` would be a positive claim about a device with no
    // framebuffer port — the same error as decoding an empty capture to zeros.
    assert_eq!(
        RefusingFb.residency(),
        None,
        "⊘ NOT Some(0): 'there is no store to ask' and 'nothing is resident' are different \
         facts, and only one of them is about the guest"
    );
    assert_eq!(RefusingFb.is_resident(0x4000), None);
}

// =====================================================================================
// §16.64 — the root the OBJECT MODEL resolved, for the VA spaces a handle-keyed
// publication table structurally cannot answer for.
// =====================================================================================

/// ★★★ The page shift is DERIVED from the installed format, never assumed and never a
/// literal.
///
/// `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` has no `pageShift` field, so the root's level
/// has to come from somewhere. It comes from `GmmuFmt::level_shift(0)` — the root of a walk
/// is level 0 of the installed format by definition — and `TinyFmt` says that is **30**,
/// which is deliberately *not* the 47 a real GA106 publishes. A literal would have read
/// correctly on exactly one chip.
#[test]
fn a_declared_root_takes_its_page_shift_from_the_installed_format() {
    let r = kayfabe_device::ceresolve::root_from_declared_pdb(&TinyFmt, 0x0034_1000).unwrap();
    assert_eq!(
        r.page_shift, 30,
        "level 0 of THIS format, not a remembered GA106 number",
    );
    assert_eq!(r.phys, 0x0034_1000);
}

/// ⚠⚠ The two aperture encodings disagree about `0`, and this root must carry the value
/// that means **vidmem in the field's own encoding**.
///
/// `VasRoot::aperture_raw` is declared to hold a `GMMU_APERTURE_*` word, where vidmem is
/// `1` and `0` is `INVALID`. The control this root came from encodes vidmem as `0`
/// (`ogkm-580: ctrl0080dma.h:842-845`). ⊘ Copying the control's word across would decode to
/// `INVALID`; the test that would have caught that is this one.
#[test]
fn a_declared_root_is_vidmem_in_the_gmmu_encoding_and_not_the_controls() {
    let r = kayfabe_device::ceresolve::root_from_declared_pdb(&TinyFmt, 0x0034_1000).unwrap();
    assert_eq!(r.aperture, Some(Aperture::Vidmem));
    assert_eq!(
        r.aperture_raw,
        kayfabe_abi::gvaspacepdes::GMMU_APERTURE_VIDEO,
        "the GMMU word for vidmem is 1; the control's word for vidmem is 0, and they are \
         never converted into one another",
    );
    assert_ne!(r.aperture_raw, 0, "0 is GMMU_APERTURE_INVALID, not vidmem");
}

/// ★★★ A root derived this way WALKS — and is INDISTINGUISHABLE from the published root
/// for the same base.
///
/// ⊘ This is the property that matters and the one a "does it construct?" test would miss:
/// the two provenances must be interchangeable *at the walk*, or the fallback is a
/// different address space wearing the right type. The comparison is against
/// `root_at(ROOT, GMMU_APERTURE_VIDEO, 30)` — the fixture every other test in this file
/// walks — so this is a differential and not a restatement of the constructor.
#[test]
fn a_declared_root_is_indistinguishable_from_the_published_root_for_the_same_base() {
    let declared =
        kayfabe_device::ceresolve::root_from_declared_pdb(&TinyFmt, ROOT).expect("level 0 exists");
    assert_eq!(
        declared,
        root_at(ROOT, GMMU_APERTURE_VIDEO, 30),
        "a root the object model declared must BE the root a publication would have given",
    );
    // ⊘ And it is checked at the WALK too, not only at the struct: equality of fields is a
    // claim about this constructor, while the resolved byte is a claim about the tree.
    assert_eq!(
        resolve(
            &TinyFmt,
            &mut tree(),
            &declared,
            VA,
            LIMITS,
            Demand::from_doorbell(),
        ),
        CeResolve::Resolved {
            phys: LEAF_PHYS + 0x1234,
            aperture: Aperture::Vidmem,
            page_size: 2 << 20,
            read_only: false,
            level: 1,
        },
    );
}

/// ⊘ A format with no level 0 REFUSES BY NAME rather than guessing a stride.
///
/// The same discipline `GmmuFmt::level_shift`'s own doc states: *"an un-enumerated size is
/// a loud fault, never a silent drop."*
#[test]
fn a_declared_root_against_a_format_with_no_root_level_refuses_by_name() {
    struct NoLevels;
    impl GmmuFmt for NoLevels {
        fn version(&self) -> GmmuVersion {
            GmmuVersion::Ver2
        }
        fn page_sizes(&self) -> &[PageSize] {
            &[PageSize(4096)]
        }
        fn entry_size(&self, _l: u8) -> u8 {
            8
        }
        fn levels(&self) -> u8 {
            0
        }
        fn level_shift(&self, _l: u8) -> Option<LevelShift> {
            None
        }
        fn decode_entry(&self, _l: u8, _raw: u128) -> PteDecode {
            PteDecode::Invalid
        }
    }
    assert_eq!(
        kayfabe_device::ceresolve::root_from_declared_pdb(&NoLevels, 0x1000),
        Err(CeResolve::NoRootLevel { page_shift: 0 }),
    );
}
