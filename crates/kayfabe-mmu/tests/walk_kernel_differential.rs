//! The **differential oracle** for `cuda/walk`: the same GA10x VER2 tables
//! decoded by two independent implementations.
//!
//! - this side — `kayfabe_chips::ga10x::Ga10xGmmu` + [`decode_subtree`], the
//!   tested Rust decoder;
//! - the other side — the CUDA walk kernel, which decodes the identical corpus on
//!   a GPU and compares against the file this test writes.
//!
//! The corpus (`cuda/walk/corpus/corpus.bin`) is built by `cuda/walk/kf_corpus.cpp`
//! from the raw `dev_mmu.h` field definitions, sharing no code with either decoder.
//!
//! ## What is compared, and what deliberately is not
//!
//! Only the fields BOTH decoders carry: `(va, phys, page size, aperture,
//! read_only)`. [`kayfabe_mmu::walker::DecodedLeaf`] has no atomic-disable,
//! volatile or privilege bit, so the corpus does not set them and the comparison
//! does not claim them. ⊘ Nor is ORDER compared: both sides sort. The walk
//! kernel's emission order is asserted by its own suite; what is at stake here is
//! DECODE agreement.
//!
//! ## Regenerating
//!
//! `KF_WRITE_EXPECTED=1 cargo test -p kayfabe-mmu --test walk_kernel_differential`
//! rewrites `cuda/walk/corpus/rust_leaves.txt`. Without it the test COMPARES, so
//! the committed file cannot drift away from what this walker actually says.

use std::path::PathBuf;

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::GpuVa;
use kayfabe_chips::ga10x::Ga10xGmmu;
use kayfabe_mmu::walker::{FbRead, PtPage, decode_subtree};

/// Matches `cuda/walk/kf_walk.h`'s `KFWR_AP_*`.
fn ap_code(a: Aperture) -> u32 {
    match a {
        Aperture::Vidmem => 0,
        Aperture::Peer => 1,
        Aperture::SysmemCoherent => 2,
        Aperture::SysmemNonCoherent => 3,
    }
}

struct ImgFb<'a> {
    img: &'a [u8],
}

impl FbRead for ImgFb<'_> {
    fn read_in(&mut self, phys: u64, aperture: Aperture, buf: &mut [u8]) -> bool {
        // ⊘ The corpus IS the fabricated aperture, exactly as the production
        // source is: a page table named in system memory cannot be served from it.
        if aperture != Aperture::Vidmem {
            return false;
        }
        let Some(end) = phys.checked_add(buf.len() as u64) else {
            return false;
        };
        if end > self.img.len() as u64 {
            return false;
        }
        let lo = phys as usize;
        buf.copy_from_slice(&self.img[lo..lo + buf.len()]);
        true
    }
}

struct Img {
    name: String,
    benign: bool,
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

fn load_corpus(path: &PathBuf) -> Vec<Img> {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&b[..8], b"KFCORPUS", "corpus magic");
    let mut at = 8usize;
    let n = rd_u32(&b, &mut at);
    let mut out = Vec::new();
    for _ in 0..n {
        let nl = rd_u32(&b, &mut at) as usize;
        let name = String::from_utf8(b[at..at + nl].to_vec()).unwrap();
        at += nl;
        let benign = b[at] != 0;
        at += 1;
        let gpga_len = rd_u64(&b, &mut at);
        let root = rd_u64(&b, &mut at);
        let np = rd_u32(&b, &mut at);
        let mut mem = vec![0u8; gpga_len as usize];
        for _ in 0..np {
            let off = rd_u64(&b, &mut at) as usize;
            mem[off..off + 4096].copy_from_slice(&b[at..at + 4096]);
            at += 4096;
        }
        out.push(Img { name, benign, root, mem });
    }
    out
}

/// Entries this walk may examine. Generous: the corpus is tiny, and exhausting it
/// would make an empty answer look like a decode difference.
const BUDGET: u32 = 4_000_000;

#[test]
fn walk_kernel_differential_corpus() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cuda/walk/corpus");
    let imgs = load_corpus(&dir.join("corpus.bin"));
    assert!(imgs.len() >= 10, "corpus is suspiciously small: {}", imgs.len());

    let fmt = Ga10xGmmu::new();
    let mut text = String::new();
    text.push_str("# kayfabe-mmu::walker decode of cuda/walk/corpus/corpus.bin\n");
    text.push_str("# image <name> <benign 0|1> <leaf count>\n");
    text.push_str("# leaf <va-hex> <phys-hex> <size-hex> <aperture 0..3> <read_only 0|1>\n");

    let mut total = 0usize;
    for im in &imgs {
        let mut fb = ImgFb { img: &im.mem };
        let root = PtPage {
            phys: im.root,
            aperture: Aperture::Vidmem,
            level: 0,
            vabase: 0,
        };
        let d = decode_subtree(&fmt, &mut fb, root, BUDGET)
            .unwrap_or_else(|e| panic!("{} exhausted its budget or faulted whole: {e:?}", im.name));
        let mut rows: Vec<(u64, u64, u64, u32, u8)> = d
            .leaves
            .iter()
            .map(|l| {
                let GpuVa(va) = l.va;
                (va, l.phys, l.size.0, ap_code(l.aperture), u8::from(l.read_only))
            })
            .collect();
        rows.sort_unstable();
        total += rows.len();
        text.push_str(&format!("image {} {} {}\n", im.name, u8::from(im.benign), rows.len()));
        for r in &rows {
            text.push_str(&format!("leaf {:x} {:x} {:x} {} {}\n", r.0, r.1, r.2, r.3, r.4));
        }
    }
    // ⚠ A differential that decoded nothing on both sides agrees perfectly and
    // proves nothing.
    assert!(total > 1200, "the corpus decoded to only {total} leaves");

    let expected = dir.join("rust_leaves.txt");
    if std::env::var_os("KF_WRITE_EXPECTED").is_some() {
        std::fs::write(&expected, &text).unwrap();
        eprintln!("wrote {} ({total} leaves)", expected.display());
        return;
    }
    let have = std::fs::read_to_string(&expected)
        .unwrap_or_else(|e| panic!("{}: {e} -- run with KF_WRITE_EXPECTED=1", expected.display()));
    if have != text {
        let a: Vec<&str> = have.lines().collect();
        let b: Vec<&str> = text.lines().collect();
        for i in 0..a.len().max(b.len()) {
            let x = a.get(i).copied().unwrap_or("<missing>");
            let y = b.get(i).copied().unwrap_or("<missing>");
            assert_eq!(x, y, "rust_leaves.txt line {}", i + 1);
        }
        panic!("rust_leaves.txt differs in length only");
    }
}
