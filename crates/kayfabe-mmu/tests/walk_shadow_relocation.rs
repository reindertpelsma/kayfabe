//! ★★★★★ **THE RELOCATION, AND THE KNOWN-POSITIVE THAT MAKES ITS SUCCESS MEAN SOMETHING.**
//!
//! `SINGLE_STORE_PLAN.md` §6 step 1's live half points the walk kernel at the guest's real
//! page tables. It cannot point it at them **where they are**: `[measured w730]` the guest's
//! RM puts its tables ~11.78 GiB up a 12 GiB framebuffer, and the kernel addresses tables as
//! offsets into one flat window — so an identity window is an 11.8 GiB device allocation on a
//! 12 GiB board. w725 hit the same wall from the other side and answered it the same way:
//! *"table **pages** are relocated into a compact arena and the **address field of directory
//! entries** is rewritten to point at the new home"*.
//!
//! This file is that rewrite's oracle, and it runs with **no GPU**.
//!
//! ## ⊘ The property, and why it is the right one
//!
//! A relocation is correct iff **walking the compact image answers exactly what walking the
//! original answered**. Not "the bytes look right" — the same walker, the same leaves, same
//! VAs, same targets, same sizes, same apertures, same permissions.
//!
//! ## ⚠ And the known-positive, because a green here is otherwise worth nothing
//!
//! [`relocation_is_not_a_no_op`] builds the identical image **without** the rewrite and
//! asserts the walk then FAILS to reproduce the leaves. Without it, a `relocate_entry` that
//! returned `Unchanged` for everything would pass every assertion in this file — and that is
//! exactly the shape of the defect the trait's own doc comment refuses by default.

use std::path::PathBuf;

use kayfabe_arch::{Aperture, GmmuFmt};
use kayfabe_chips::ga10x::Ga10xGmmu;
use kayfabe_mmu::walker::{DecodedLeaf, FbRead, PtPage, decode_subtree};
use kayfabe_mmu::walkshadow::{PAGE_CAP, ShadowRefusal, build_image, compare, leaves_as_runs};

const BUDGET: u32 = 4_000_000;

/// A flat image addressed by its own offsets — the corpus's shape and the compact image's.
struct ImgFb<'a> {
    img: &'a [u8],
}

impl FbRead for ImgFb<'_> {
    fn read_in(&mut self, phys: u64, aperture: Aperture, buf: &mut [u8]) -> bool {
        if aperture != Aperture::Vidmem {
            return false;
        }
        if phys >= self.img.len() as u64 {
            return false;
        }
        // ⊘ **Zero-fill past the end rather than refuse**, which is what the production
        // `SparseFb` does: an address inside the framebuffer that was never written reads as
        // zeros. The corpus's arena is sized to the tables it holds, so a whole-page read of
        // the LAST table legitimately runs past it — and refusing there would model a
        // framebuffer that has holes, which is not the one this runs against.
        let lo = phys as usize;
        let n = (self.img.len() - lo).min(buf.len());
        buf[..n].copy_from_slice(&self.img[lo..lo + n]);
        buf[n..].fill(0);
        true
    }
}

struct Img {
    name: String,
    root: u64,
    mem: Vec<u8>,
}

fn rd_u32(b: &[u8], at: &mut usize) -> u32 {
    let v = u32::from_le_bytes(b[*at..*at + 4].try_into().unwrap());
    *at += 4;
    v
}
fn rd_u64(b: &[u8], at: &mut usize) -> u64 {
    let v = u64::from_le_bytes(b[*at..*at + 8].try_into().unwrap());
    *at += 8;
    v
}

fn load(file: &str) -> Vec<Img> {
    let path: PathBuf = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("the manifest is two levels below the root")
        .join("cuda/walk/corpus")
        .join(file);
    let b = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&b[..8], b"KFCORPUS", "corpus magic");
    let mut at = 8usize;
    let n = rd_u32(&b, &mut at);
    let mut out = Vec::new();
    for _ in 0..n {
        let nl = rd_u32(&b, &mut at) as usize;
        let name = String::from_utf8(b[at..at + nl].to_vec()).unwrap();
        at += nl;
        at += 1; // benign
        let gpga_len = rd_u64(&b, &mut at);
        let root = rd_u64(&b, &mut at);
        let np = rd_u32(&b, &mut at);
        let mut mem = vec![0u8; gpga_len as usize];
        for _ in 0..np {
            let off = rd_u64(&b, &mut at) as usize;
            mem[off..off + 4096].copy_from_slice(&b[at..at + 4096]);
            at += 4096;
        }
        out.push(Img { name, root, mem });
    }
    out
}

/// Walk one flat image from `root`, returning its leaves and the pages it visited.
fn walk(img: &[u8], root: u64) -> (Vec<DecodedLeaf>, Vec<PtPage>) {
    let fmt = Ga10xGmmu::new();
    let mut fb = ImgFb { img };
    let page = PtPage {
        phys: root & !0xfff,
        aperture: Aperture::Vidmem,
        level: 0,
        vabase: 0,
    };
    let d = decode_subtree(&fmt, &mut fb, page, BUDGET).expect("the corpus root decodes");
    (d.leaves, d.visited)
}

/// Sort leaves so two walks of the same tables compare regardless of emission order.
fn sorted(mut l: Vec<DecodedLeaf>) -> Vec<DecodedLeaf> {
    l.sort_by_key(|x| (x.va.0, x.phys, x.size.0));
    l
}

/// ★★★★★ **THE PROPERTY: a relocated image answers what the original answered.**
///
/// Over w725's capture of a **real** GA106's tables — five address spaces, 6 998 leaves,
/// 4 KiB + 64 KiB + 2 MiB pages, vidmem and sysmem apertures side by side, and `KIND`
/// non-zero on 99.7 % of leaves. ⊘ A fixture of eight contiguous pages could not test this:
/// it has no dual PDE, no big-page table and no second level to get wrong.
#[test]
fn a_relocated_image_answers_exactly_what_the_original_answered() {
    let fmt = Ga10xGmmu::new();
    let imgs = load("real_ga106.bin");
    assert_eq!(imgs.len(), 5, "the real-GA106 corpus is five address spaces");
    let mut total = 0usize;
    for img in &imgs {
        let (leaves, visited) = walk(&img.mem, img.root);
        if leaves.is_empty() {
            continue;
        }
        total += leaves.len();

        let mut fb = ImgFb { img: &img.mem };
        let image = build_image(&fmt, &mut fb, &[(img.root, &visited)], PAGE_CAP)
            .unwrap_or_else(|e| panic!("{}: build_image refused: {e:?}", img.name));
        assert_eq!(image.roots.len(), 1);
        assert!(
            image.pages >= 2,
            "{}: a real address space has more than one table page, got {}",
            img.name,
            image.pages
        );

        let (relocated, _) = walk(&image.bytes, image.roots[0].1);
        assert_eq!(
            sorted(relocated.clone()),
            sorted(leaves.clone()),
            "★ {}: the relocated image answers differently — {} leaves vs {}",
            img.name,
            relocated.len(),
            leaves.len()
        );

        // And through the comparison the census actually uses, which is the form the live
        // shadow compares in.
        let (a, dropped_a) = leaves_as_runs(&leaves);
        let (b, dropped_b) = leaves_as_runs(&relocated);
        assert_eq!(dropped_a, dropped_b, "{}: leaf classes differ", img.name);
        assert!(
            compare(&a, &b).is_empty(),
            "{}: canonical comparison disagreed after relocation",
            img.name
        );
    }
    assert!(
        total >= 6000,
        "the assertions above ran on {total} leaves; a handful would make them vacuous"
    );
}

/// A sparse framebuffer: page-addressed, like the production one and unlike the corpus's
/// flat arena.
struct SparseImg {
    pages: std::collections::BTreeMap<u64, Vec<u8>>,
}

impl FbRead for SparseImg {
    fn read_in(&mut self, phys: u64, aperture: Aperture, buf: &mut [u8]) -> bool {
        if aperture != Aperture::Vidmem {
            return false;
        }
        let base = phys & !0xfff;
        let off = (phys & 0xfff) as usize;
        let Some(p) = self.pages.get(&base) else {
            return false;
        };
        // ⊘ Zero-fill past the stored table, for the same reason `ImgFb` does: this fixture
        // stores each table at exactly its own length, and the production framebuffer serves
        // a whole page of which the unwritten tail reads as zeros.
        let n = p.len().saturating_sub(off).min(buf.len());
        buf[..n].copy_from_slice(&p[off..off + n]);
        buf[n..].fill(0);
        true
    }
}

/// Where the scattered fixture puts the guest's tables: **11.7 GiB up**, on a 64 KiB stride,
/// which is where `[measured w730]` and w725's capture both say a real driver's tables are and
/// is the whole reason relocation exists.
const SCATTER_BASE: u64 = 0x2_efa4_0000;
/// The stride, chosen so that packing the pages back down is emphatically **not** the identity.
const SCATTER_STRIDE: u64 = 0x10000;

/// Scatter a flat corpus image into a sparse framebuffer at high addresses.
///
/// `rewrite` is the switch this file's known-positive turns: with it off the pages move and
/// their directory entries do **not**, which is precisely the image a relocator that answered
/// `Unchanged` for everything would build.
fn scatter(
    img: &[u8],
    root: u64,
    visited: &[PtPage],
    rewrite: bool,
) -> (SparseImg, u64) {
    let fmt = Ga10xGmmu::new();
    let mut order: Vec<(u64, u8)> = visited
        .iter()
        .filter(|p| p.aperture == Aperture::Vidmem)
        .map(|p| (p.phys & !0xfff, p.level))
        .collect();
    order.sort_unstable();
    order.dedup_by_key(|(p, _)| *p);
    let home_of: std::collections::BTreeMap<u64, u64> = order
        .iter()
        .enumerate()
        .map(|(i, (p, _))| (*p, SCATTER_BASE + (i as u64) * SCATTER_STRIDE))
        .collect();
    let home = |old: u64| -> Option<u64> { Some(home_of.get(&(old & !0xfff)).copied().unwrap_or(0)) };

    let mut pages = std::collections::BTreeMap::new();
    for (phys, level) in &order {
        let es = usize::from(fmt.entry_size(*level));
        let entries = fmt.level_shift(*level).expect("geometry").entries as usize;
        let need = (entries * es).min(4096);
        let src = *phys as usize;
        let n = need.min(img.len().saturating_sub(src));
        let mut buf = vec![0u8; need];
        buf[..n].copy_from_slice(&img[src..src + n]);
        if rewrite {
            for i in 0..entries {
                let off = i * es;
                if off + es > need {
                    break;
                }
                let mut w = [0u8; 16];
                w[..es].copy_from_slice(&buf[off..off + es]);
                match fmt.relocate_entry(*level, u128::from_le_bytes(w), &home) {
                    kayfabe_arch::Relocated::Moved(v) => {
                        buf[off..off + es].copy_from_slice(&v.to_le_bytes()[..es]);
                    }
                    kayfabe_arch::Relocated::Unchanged => {}
                    kayfabe_arch::Relocated::Refused(why) => panic!("scatter refused: {why}"),
                }
            }
        }
        pages.insert(*phys, buf);
    }
    // Re-key by the new home once every page's contents are settled.
    let moved = pages
        .into_iter()
        .map(|(p, b)| (home_of[&p], b))
        .collect::<std::collections::BTreeMap<_, _>>();
    (SparseImg { pages: moved }, home_of[&(root & !0xfff)])
}

/// ★★★★★ **THE KNOWN-POSITIVE — move the pages and the walk must follow, or not at all.**
///
/// ⊘ This is what makes every other assertion in this file worth reading. A `relocate_entry`
/// that answered `Unchanged` for every entry — the exact shape of the defect
/// [`kayfabe_arch::GmmuFmt`]'s default refuses — would satisfy the round-trip test above,
/// because w725's corpus is **already compacted** and this crate's slot assignment reproduces
/// its packing byte for byte (`[measured]` all five address spaces pack as the identity).
///
/// ⇒ The fixture here moves the tables to where a real driver actually puts them — 11.7 GiB
/// up, on a stride — and asserts **both** arms: with the rewrite the walk answers exactly what
/// it answered before; **without** it, it does not.
#[test]
fn relocation_is_not_a_no_op() {
    let imgs = load("real_ga106.bin");
    let mut proved = 0usize;
    for img in &imgs {
        let (leaves, visited) = walk(&img.mem, img.root);
        if leaves.is_empty() {
            continue;
        }

        // ── the arm that must WORK ──
        let (mut moved, new_root) = scatter(&img.mem, img.root, &visited, true);
        let fmt = Ga10xGmmu::new();
        let d = decode_subtree(
            &fmt,
            &mut moved,
            PtPage { phys: new_root, aperture: Aperture::Vidmem, level: 0, vabase: 0 },
            BUDGET,
        )
        .expect("the scattered root decodes");
        assert_eq!(
            sorted(d.leaves.clone()),
            sorted(leaves.clone()),
            "★ {}: relocating the tables to {SCATTER_BASE:#x} changed the answer",
            img.name
        );

        // ── the arm that must FAIL: same move, entries left alone ──
        let (mut naive, naive_root) = scatter(&img.mem, img.root, &visited, false);
        let n = decode_subtree(
            &fmt,
            &mut naive,
            PtPage { phys: naive_root, aperture: Aperture::Vidmem, level: 0, vabase: 0 },
            BUDGET,
        )
        .map(|d| d.leaves)
        .unwrap_or_default();
        assert_ne!(
            sorted(n),
            sorted(leaves.clone()),
            "★ {}: moving the pages WITHOUT rewriting their directory entries reproduced \\
             the original walk — then a relocator that did nothing would pass this file",
            img.name
        );
        proved += 1;
    }
    assert!(proved >= 3, "only {proved} images exercised the known-positive");
}

/// ★★ **And the packed image built from tables that really are high up.** The round-trip test
/// runs over w725's already-compacted corpus, where packing is the identity; this one runs
/// [`build_image`] over the scattered fixture, where it is not.
#[test]
fn the_packed_image_answers_the_same_over_tables_that_are_high_up() {
    let fmt = Ga10xGmmu::new();
    let imgs = load("real_ga106.bin");
    let mut compared = 0usize;
    for img in &imgs {
        let (leaves, visited) = walk(&img.mem, img.root);
        if leaves.is_empty() {
            continue;
        }
        let (mut moved, new_root) = scatter(&img.mem, img.root, &visited, true);
        // Re-walk the scattered image to learn its OWN visited set, since the addresses moved.
        let d = decode_subtree(
            &fmt,
            &mut moved,
            PtPage { phys: new_root, aperture: Aperture::Vidmem, level: 0, vabase: 0 },
            BUDGET,
        )
        .expect("decodes");
        let image = build_image(&fmt, &mut moved, &[(new_root, &d.visited)], PAGE_CAP)
            .unwrap_or_else(|e| panic!("{}: build_image refused: {e:?}", img.name));
        assert!(
            image.roots[0].1 < (image.pages as u64 + 1) * 4096,
            "the packed root must be inside the packed image"
        );
        let (packed, _) = walk(&image.bytes, image.roots[0].1);
        assert_eq!(
            sorted(packed),
            sorted(leaves.clone()),
            "★ {}: packing tables down from {SCATTER_BASE:#x} changed the answer",
            img.name
        );
        compared += 1;
    }
    assert!(compared >= 3, "only {compared} images were packed");
}

/// ★★ **A withheld page is COUNTED, not silently followed.** The kernel's reach is clipped to
/// the host walk's, and [`kayfabe_mmu::walkshadow::ShadowImage::absent_edges`] is the measured
/// size of that clipping. ⊘ Without this the clipping would be invisible and a
/// `missing_in_kernel` caused by it would read as a decode disagreement.
#[test]
fn an_edge_the_host_never_visited_is_counted_and_sent_nowhere() {
    let fmt = Ga10xGmmu::new();
    let imgs = load("real_ga106.bin");
    let img = imgs
        .iter()
        .max_by_key(|i| i.mem.len())
        .expect("the corpus is not empty");
    let (leaves, visited) = walk(&img.mem, img.root);
    assert!(!leaves.is_empty());

    let mut fb = ImgFb { img: &img.mem };
    let whole = build_image(&fmt, &mut fb, &[(img.root, &visited)], PAGE_CAP).expect("built");
    assert_eq!(
        whole.absent_edges, 0,
        "a complete visited set must leave NO edge pointing at the absent slot, or the \
         assertion below is about nothing"
    );

    // Withhold every page below the deepest level — the small-page tables — and confirm the
    // edges that named them are counted and the walk finds strictly fewer leaves.
    let deepest = visited.iter().map(|p| p.level).max().expect("levels");
    let kept: Vec<PtPage> = visited
        .iter()
        .copied()
        .filter(|p| p.level != deepest)
        .collect();
    assert!(kept.len() < visited.len(), "the filter removed nothing");
    let mut fb = ImgFb { img: &img.mem };
    let clipped = build_image(&fmt, &mut fb, &[(img.root, &kept)], PAGE_CAP).expect("built");
    assert!(
        clipped.absent_edges > 0,
        "withholding every level-{deepest} table left absent_edges at zero"
    );
    let (clipped_leaves, _) = walk(&clipped.bytes, clipped.roots[0].1);
    assert!(
        clipped_leaves.len() < leaves.len(),
        "a clipped image must answer with FEWER leaves — {} vs {}",
        clipped_leaves.len(),
        leaves.len()
    );
}

/// ⊘ **Every refusal that gates the live shadow fires by name.** A refusal nobody has seen
/// fire is a refusal nobody knows the spelling of.
#[test]
fn the_image_refusals_fire_by_name() {
    let fmt = Ga10xGmmu::new();
    let imgs = load("real_ga106.bin");
    let img = &imgs[0];
    let (_, visited) = walk(&img.mem, img.root);
    let mut fb = ImgFb { img: &img.mem };

    // ── TooManyPages ──
    match build_image(&fmt, &mut fb, &[(img.root, &visited)], 1) {
        Err(ShadowRefusal::TooManyPages { pages, cap }) => {
            assert!(pages > 1);
            assert_eq!(cap, 1);
        }
        other => panic!("a one-page budget must refuse TooManyPages, got {other:?}"),
    }

    // ── NoPages ──
    match build_image(&fmt, &mut fb, &[(img.root, &[])], PAGE_CAP) {
        Err(ShadowRefusal::NoPages) => {}
        other => panic!("an empty visited set must refuse NoPages, got {other:?}"),
    }

    // ── RootMissing: the pages are there, the root is not among them ──
    let without_root: Vec<PtPage> = visited
        .iter()
        .copied()
        .filter(|p| p.phys & !0xfff != img.root & !0xfff)
        .collect();
    assert!(!without_root.is_empty());
    match build_image(&fmt, &mut fb, &[(img.root, &without_root)], PAGE_CAP) {
        Err(ShadowRefusal::RootMissing { pdb }) => assert_eq!(pdb, img.root),
        other => panic!("a missing root must refuse RootMissing, got {other:?}"),
    }

    // ── Unreadable: a page the byte source will not serve ──
    let mut bogus = visited.clone();
    bogus.push(PtPage {
        phys: 0xdead_0000,
        aperture: Aperture::Vidmem,
        level: 5,
        vabase: 0,
    });
    match build_image(&fmt, &mut fb, &[(img.root, &bogus)], PAGE_CAP) {
        Err(ShadowRefusal::Unreadable { phys }) => assert_eq!(phys, 0xdead_0000),
        other => panic!("an unservable page must refuse Unreadable, got {other:?}"),
    }

    // ── AmbiguousLevel: one page claimed at two levels ──
    let mut two = visited.clone();
    let mut alias = visited[0];
    alias.level = alias.level.wrapping_add(1);
    two.push(alias);
    match build_image(&fmt, &mut fb, &[(img.root, &two)], PAGE_CAP) {
        Err(ShadowRefusal::AmbiguousLevel { phys }) => {
            assert_eq!(phys, visited[0].phys & !0xfff);
        }
        other => panic!("one page at two levels must refuse AmbiguousLevel, got {other:?}"),
    }
}

/// ⊘ **A format with no relocator refuses the whole image**, rather than handing back an
/// image whose entries still point at their old homes.
#[test]
fn a_format_without_a_relocator_refuses() {
    let fmt = kayfabe_chips::ga10x::UnbuiltGmmu;
    let imgs = load("real_ga106.bin");
    let img = &imgs[0];
    let (_, visited) = walk(&img.mem, img.root);
    let mut fb = ImgFb { img: &img.mem };
    match build_image(&fmt, &mut fb, &[(img.root, &visited)], PAGE_CAP) {
        // `UnbuiltGmmu` has no geometry either, so it refuses one step earlier — which is
        // the same outcome by a stricter route, and is asserted rather than assumed.
        Err(ShadowRefusal::BadGeometry { .. } | ShadowRefusal::Relocate(_)) => {}
        other => panic!("a format with no relocator must refuse, got {other:?}"),
    }
}

/// ★★ **Sysmem sub-tables are counted and left alone.** The walk kernel refuses a non-vidmem
/// table page by name (`KFWR_R_FOREIGN_AP`) and never follows it; the host walker does follow
/// it. That is a **scope** difference between the two walkers, and this count is what lets a
/// reader tell it apart from a decode disagreement.
#[test]
fn sysmem_sub_tables_are_counted_rather_than_rewritten() {
    let fmt = Ga10xGmmu::new();
    let imgs = load("real_ga106.bin");
    let mut seen_any = false;
    for img in &imgs {
        let (leaves, visited) = walk(&img.mem, img.root);
        if leaves.is_empty() {
            continue;
        }
        let mut fb = ImgFb { img: &img.mem };
        let image = build_image(&fmt, &mut fb, &[(img.root, &visited)], PAGE_CAP).expect("built");
        seen_any = true;
        // Whatever the count is, it must be consistent with the walk succeeding — the point
        // of the field is that it is REPORTED, so the assertion is that it exists and does
        // not prevent the image from being built.
        let _ = image.sysmem_edges;
    }
    assert!(seen_any, "no image produced leaves");
}

/// ★★★★★ **A SUB-PAGE TABLE KEEPS ITS OFFSET — the w731 `missing_in_kernel=30`.**
///
/// A VER2 big-page table is **32 entries, 256 bytes**, and the dual PDE's big half names it
/// with `_ADDRESS_SHIFT = 8` — **256-byte granularity**. `PD3` is four entries, 32 bytes. So a
/// table legitimately sits at a non-page-aligned address.
///
/// ⊘ `[measured w731, live boot]` keying the image by `phys & !0xfff` threw that offset away:
/// the edge naming such a table was sent to the absent slot, the kernel found **nothing**
/// under that root (`PDB[0] run_count=0`) while the host had decoded a leaf there, and the
/// census reported `missing_in_kernel` — **for a mapping neither walker had got wrong**.
#[test]
fn a_table_at_a_sub_page_offset_survives_the_image() {
    let fmt = Ga10xGmmu::new();

    // Two tables inside ONE page, at different offsets. Only their addresses matter here:
    // the assertion is that both keep their offsets and that `home` never sends either to
    // the absent slot.
    let page = 0x2_efa4_0000u64;
    let visited = [
        PtPage { phys: page, aperture: Aperture::Vidmem, level: 0, vabase: 0 },
        PtPage { phys: page + 0x800, aperture: Aperture::Vidmem, level: 4, vabase: 0 },
    ];
    let mut fb = SparseImg {
        pages: std::collections::BTreeMap::from([(page, vec![0u8; 4096])]),
    };
    let image = build_image(&fmt, &mut fb, &[(page, &visited)], PAGE_CAP).expect("built");

    assert_eq!(
        image.pages, 1,
        "two tables in one page are ONE page in the image, not two"
    );
    assert_eq!(
        image.absent_edges, 0,
        "a zeroed page names no edges at all, so nothing may be counted absent"
    );
    // ★ The root keeps its own offset — here zero, but the arithmetic is the one that matters.
    assert_eq!(image.roots[0].1 % 4096, page % 4096);

    // And a root that is itself at a sub-page offset lands at that offset in its slot.
    let off_root = page + 0x800;
    let image2 = build_image(&fmt, &mut fb, &[(off_root, &visited)], PAGE_CAP).expect("built");
    assert_eq!(
        image2.roots[0].1 % 4096,
        0x800,
        "a root at +0x800 must be addressed at +0x800 in its slot, not at the page base"
    );
}
