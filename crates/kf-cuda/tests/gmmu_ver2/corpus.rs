//! The walk kernel's differential corpus, decoded on the CPU and compared with the frozen
//! expected output the kernel itself is tested against.
//!
//! `cuda/walk/kf_tests.cu` decodes `cuda/walk/corpus/corpus.bin` (15 synthetic address spaces,
//! built by `kf_corpus.cpp` from the raw `dev_mmu.h` field definitions) and `real_ga106.bin`
//! (5 address spaces a stock NVIDIA 580.159.04 guest driver wrote on a real GA106) on a GPU and
//! compares every leaf with `rust_leaves.txt` / `real_leaves.txt`. Those two text files were the
//! output of the retired `kayfabe-mmu` walker. Until this test they were frozen expected output
//! that nothing re-derived; now the ogkm-derived reference decoder in `reference.rs` recomputes
//! them from the same binary inputs, so a change to the `.txt` that the kernel's author makes to
//! quiet the kernel (or a regeneration from the kernel's own output) is caught here without a GPU.
//!
//! What is compared, and what deliberately is not: `(va, phys, page size, aperture, read_only)`
//! per leaf -- the fields both decoders carry -- sorted. Order is not compared (the kernel's
//! emission order is asserted by its own suite) and neither are atomic-disable / volatile /
//! privilege / KIND, which the `.txt` format has no column for. The `#` header lines of the
//! `.txt` files are not compared: they name the archived decoder that wrote them.
//!
//! `KF_WRITE_EXPECTED=1 cargo test -p kf-cuda --test gmmu_ver2 corpus` rewrites the two files
//! from the reference (data lines only; the header is kept as found). That is a regeneration
//! from THIS decoder: review the diff against the kernel's run before committing it.

use std::path::PathBuf;

use crate::reference::{Aperture, Image, Leaf, Page, Ver2, ZeroChild, decode_subtree};

/// Entries one walk may examine: generous, the corpus is tiny, and exhausting it would make an
/// empty answer look like a decode difference.
const BUDGET: u32 = 4_000_000;

struct Corpus {
    name: String,
    benign: bool,
    root: u64,
    img: Image,
}

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cuda/walk/corpus")
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

/// `kf_corpus.cpp`'s container: `"KFCORPUS"`, image count, then per image a length-prefixed
/// name, a `benign` byte, the GPGA length, the root, a page count and `(offset, 4096 bytes)`
/// pairs.
fn load(file: &str) -> Vec<Corpus> {
    let path = corpus_dir().join(file);
    let b = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&b[..8], b"KFCORPUS", "{file}: magic");
    let mut at = 8usize;
    let n = rd_u32(&b, &mut at);
    let mut out = Vec::new();
    for _ in 0..n {
        let name_len = rd_u32(&b, &mut at) as usize;
        let name = String::from_utf8(b[at..at + name_len].to_vec()).unwrap();
        at += name_len;
        let benign = b[at] != 0;
        at += 1;
        let len = rd_u64(&b, &mut at);
        let root = rd_u64(&b, &mut at);
        let pages = rd_u32(&b, &mut at);
        let mut img = Image {
            len,
            ..Image::default()
        };
        for _ in 0..pages {
            let off = rd_u64(&b, &mut at);
            assert_eq!(
                off % 4096,
                0,
                "{file}/{name}: unaligned page offset {off:#x}"
            );
            assert!(
                off + 4096 <= len,
                "{file}/{name}: page {off:#x} outside the image"
            );
            let mut page = Box::new([0u8; 4096]);
            page.copy_from_slice(&b[at..at + 4096]);
            at += 4096;
            img.pages.insert(off, page);
        }
        out.push(Corpus {
            name,
            benign,
            root,
            img,
        });
    }
    assert_eq!(at, b.len(), "{file}: trailing bytes after the last image");
    out
}

fn decode(c: &Corpus, zero_child: ZeroChild) -> Vec<Leaf> {
    let root = Page {
        phys: c.root,
        aperture: Aperture::Vidmem,
        level: 0,
        vabase: 0,
    };
    decode_subtree(&Ver2::ga10x(), &c.img, root, BUDGET, zero_child)
        .unwrap_or_else(|e| panic!("{}: exhausted its budget: {e:?}", c.name))
        .leaves
}

/// `image <name> <benign> <count>` / `leaf <va> <phys> <size> <aperture> <read_only>` lines, the
/// sorted-row format of the frozen files.
fn render(corpus: &[Corpus], zero_child: ZeroChild) -> (Vec<String>, usize) {
    let mut lines = Vec::new();
    let mut total = 0usize;
    for c in corpus {
        let mut rows: Vec<(u64, u64, u64, u32, u8)> = decode(c, zero_child)
            .iter()
            .map(|l| {
                (
                    l.va,
                    l.phys,
                    l.size.0,
                    l.aperture.code(),
                    u8::from(l.read_only),
                )
            })
            .collect();
        rows.sort_unstable();
        total += rows.len();
        lines.push(format!(
            "image {} {} {}",
            c.name,
            u8::from(c.benign),
            rows.len()
        ));
        for r in rows {
            lines.push(format!(
                "leaf {:x} {:x} {:x} {} {}",
                r.0, r.1, r.2, r.3, r.4
            ));
        }
    }
    (lines, total)
}

/// The expected file's data lines (everything that is not a `#` comment).
fn expected(file: &str) -> Vec<String> {
    let path = corpus_dir().join(file);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

fn check(bin: &str, txt: &str, min_images: usize, min_leaves: usize) {
    let corpus = load(bin);
    assert!(
        corpus.len() >= min_images,
        "{bin} is suspiciously small: {}",
        corpus.len()
    );
    let (have, total) = render(&corpus, ZeroChild::IsATable);
    // A differential that decoded nothing on both sides agrees perfectly and proves nothing.
    assert!(total > min_leaves, "{bin} decoded to only {total} leaves");

    if std::env::var_os("KF_WRITE_EXPECTED").is_some() {
        let path = corpus_dir().join(txt);
        let old = std::fs::read_to_string(&path).unwrap();
        let mut out: String = old
            .lines()
            .take_while(|l| l.starts_with('#'))
            .map(|l| format!("{l}\n"))
            .collect();
        for l in &have {
            out.push_str(l);
            out.push('\n');
        }
        std::fs::write(&path, out).unwrap();
        eprintln!("wrote {} ({total} leaves)", path.display());
        return;
    }

    let want = expected(txt);
    for i in 0..want.len().max(have.len()) {
        let w = want.get(i).map_or("<missing>", String::as_str);
        let h = have.get(i).map_or("<missing>", String::as_str);
        assert_eq!(
            h,
            w,
            "{txt} data line {}: reference (left) vs frozen file (right)",
            i + 1
        );
    }
}

/// 15 synthetic address spaces: ten benign shapes (single page, 1024-page run, split runs, all
/// page sizes, both halves of a dual entry, sparse holes, a shared page table, all four
/// apertures, leaves spread across every level, a full big-page table) and five hostile ones
/// (self-cycle, pointer past the image, all-ones, type confusion, two-cycle).
#[test]
fn the_reference_recomputes_the_synthetic_corpus_leaves() {
    check("corpus.bin", "rust_leaves.txt", 10, 1200);
}

/// Five address spaces a REAL driver wrote. `kf_real_tables.py` documents the extraction and its
/// self-consistency proof (all 177 856 BAR2 writes in the trace translate through the rebuilt
/// tables). 6 986 of the 7 008 real leaf PTEs carry a non-zero KIND (bits 63:56) and 6 017 a
/// non-zero COMPTAGLINE, encodings no synthetic case contains; both decoders must mask them off,
/// which is what `PTE_ADDRESS_VID = 32:8` sitting below both guarantees.
#[test]
fn the_reference_recomputes_the_real_driver_corpus_leaves() {
    check("real_ga106.bin", "real_leaves.txt", 5, 6000);
}

/// The corpus must not depend on the one rule the archived decoder and ogkm disagree on: whether
/// a directory entry whose sub-table address is zero is a table at offset 0 (ogkm, the aperture
/// is the validity) or a null pointer to skip (the archived decoder). If it did, the frozen files
/// would pin the superseded rule and the kernel's
/// `KF_PDE_ADDR_ZERO_IS_A_TABLE` fix would contradict them.
#[test]
fn the_corpus_is_insensitive_to_the_null_child_rule() {
    for (bin, txt) in [
        ("corpus.bin", "rust_leaves.txt"),
        ("real_ga106.bin", "real_leaves.txt"),
    ] {
        let corpus = load(bin);
        let (a, _) = render(&corpus, ZeroChild::IsATable);
        let (b, _) = render(&corpus, ZeroChild::Skipped);
        assert_eq!(a, b, "{bin}: the null-child rule changes the decode");
        assert_eq!(b, expected(txt), "{txt}: still the archived rule's answer");
    }
}

/// Non-vacuity of the expected files themselves: every image's header line must state its own
/// leaf count, and the benign/hostile split must be the one `kf_corpus.cpp` declares.
#[test]
fn the_frozen_files_are_self_consistent() {
    for (bin, txt) in [
        ("corpus.bin", "rust_leaves.txt"),
        ("real_ga106.bin", "real_leaves.txt"),
    ] {
        let corpus = load(bin);
        let lines = expected(txt);
        let mut at = 0;
        for c in &corpus {
            let h: Vec<&str> = lines[at].split(' ').collect();
            assert_eq!(h[..2], ["image", c.name.as_str()], "{txt}: image order");
            assert_eq!(
                h[2],
                if c.benign { "1" } else { "0" },
                "{txt}/{}: benign flag",
                c.name
            );
            let n: usize = h[3].parse().unwrap();
            assert!(
                lines[at + 1..at + 1 + n]
                    .iter()
                    .all(|l| l.starts_with("leaf ")),
                "{txt}/{}: header promises {n} leaves",
                c.name
            );
            at += 1 + n;
        }
        assert_eq!(at, lines.len(), "{txt}: lines after the last image");
    }
}

/// Some of the hostile images must be what makes them hostile -- a cycle that the walk bounds
/// rather than follows -- and the walk must come back with faults or no leaves, never a hang.
/// (The budget is 4 M entries; a walk that followed a cycle without the depth bound would exhaust
/// it and fail `decode`.)
#[test]
fn the_hostile_images_decode_without_exhausting_the_budget() {
    let corpus = load("corpus.bin");
    let hostile: Vec<&Corpus> = corpus.iter().filter(|c| !c.benign).collect();
    assert_eq!(hostile.len(), 5);
    for c in hostile {
        let root = Page {
            phys: c.root,
            aperture: Aperture::Vidmem,
            level: 0,
            vabase: 0,
        };
        let t = decode_subtree(&Ver2::ga10x(), &c.img, root, BUDGET, ZeroChild::IsATable)
            .unwrap_or_else(|e| panic!("{}: {e:?}", c.name));
        assert!(
            t.pages_visited < 10_000,
            "{}: {} pages",
            c.name,
            t.pages_visited
        );
    }
}
