//! ★ 2026-11-18 — **GUEST PTE → HOST PTE, A GPU-FREE MODEL-BASED DIFFERENTIAL FUZZ**
//! (`docs/design/V3_PTE_MODEL_FUZZ.md`).
//!
//! The pipeline under test is production's, end to end, minus the GPU:
//! guest page-table bytes → the walk kernel's SOURCE (`cuda/walk/kf_walk.cu`, its serial walk run on
//! the CPU through `cuda/walk/kf_pte_fuzz_walk.cpp`, built here with `g++`) → the commit-on-ack diff
//! (`kf_cuda::diffmodel`, the kernel's Rust spec) → `PermPolicy::diff_run` → [`apply_entry`] →
//! the real [`crate::batch::BatchedVas`] over the host-RM model ([`SimMirror`], whose mappings
//! record the kind and permissions each map verb carried) → the diff model's commit.
//!
//! The ORACLE is [`super::pte_oracle`], written from the ogkm field definitions (every field cited
//! to its `file:line`), not from kayfabe's constants. The properties, per step of a seeded sequence
//! of guest table edits:
//! - **P1 (fields)**: every host page translates exactly as the guest's: backing, KIND (its
//!   uncompressed layout), READ_ONLY, VOLATILE, ATOMIC_DISABLE (when the policy carries it).
//! - **P2 (hostile guest)**: no host page outside what the guest's own translation names and the
//!   host authorised (the store below the carve-out, guest RAM through the VMM layout); no
//!   PRIVILEGED leaf on a user twin; no peer.
//! - **P3 (unmap unmaps)**: no host page where the guest's translation is INVALID or SPARSE.
//! - **P4 (idempotence)**: an unchanged guest walks to an empty diff and leaves the host as it was.
//! - **P5 (bounded work)**: host verbs per refresh are bounded by the diff.
//! - **No silent absence**: a page the guest maps and the host does not is accounted to a refusal
//!   by name (a FAILED acknowledgement or a walker refusal bit).
//!
//! Every mismatch is classified; the classes are counted per campaign and a class that is not a
//! documented known gap fails the test with its minimal reproduction.
//!
//! Commands (`V3_PTE_MODEL_FUZZ.md` §4):
//! - quick (part of `cargo test -p kf-mem`): `pte_fuzz_quick_*`.
//! - campaign: `KF_FUZZ_SECONDS=600 cargo test -p kf-mem --release -- --ignored pte_fuzz_campaign --nocapture`.

use super::pte_oracle::{self as o, Aperture, Geo, Mem, Xlate};
use super::*;
use crate::apply::PermPolicy;
use kf_cuda::abi::{KFWR_ACK_APPLIED, KFWR_ACK_HELD, KFWR_OP_MAP, KfMapRun};
use kf_cuda::diffmodel::{self, AckCode};
use std::collections::BTreeMap as Map;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::OnceLock;

/// Guest vidmem (the store): 1 GiB. The firmware carve-out is its last 64 MiB.
const STORE: u64 = 1 << 30;
const CARVE: u64 = STORE - (64 << 20);
/// Guest RAM: two 1 GiB chunks, GPA `[0, 1G)` → file `[0, 1G)` and `[4G, 5G)` → `[1G, 2G)`.
const RAM_A: (u64, u64, u64) = (0, 1 << 30, 0);
const RAM_B: (u64, u64, u64) = (4 << 30, 1 << 30, 1 << 30);
/// The guest VA reservation of the model space (`GUEST_VA_RANGES[0]`-like).
const R0: u64 = (1 << 32) + (1 << 29);
const RLEN: u64 = 1 << 30;

fn ram_offset(gpa: u64, len: u64) -> Option<u64> {
    let end = gpa.checked_add(len)?;
    [RAM_A, RAM_B]
        .into_iter()
        .find(|&(b, l, _)| gpa >= b && end <= b + l)
        .map(|(b, _, f)| f + (gpa - b))
}

// ═══ the walker: the kernel's source, on the CPU ═════════════════════════════════════════════════

fn walker_bin() -> &'static PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        // `KF_PTE_FUZZ_CU_DIR`: walk with another revision's `kf_walk.cu` (the before/after
        // equivalence of a kernel change, `V3_PTE_MODEL_FUZZ.md` §4); default this tree's.
        let dir = std::env::var_os("KF_PTE_FUZZ_CU_DIR").map_or_else(
            || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cuda/walk"),
            PathBuf::from,
        );
        let out = std::env::temp_dir().join(format!("kf_pte_fuzz_walk-{}", std::process::id()));
        let st = Command::new("g++")
            .args(["-std=c++14", "-O2", "-w", "-o"])
            .arg(&out)
            .arg(dir.join("kf_pte_fuzz_walk.cpp"))
            .status()
            .expect("g++ must be present: the walk kernel's source is the thing under test");
        assert!(st.success(), "g++ failed to build kf_pte_fuzz_walk.cpp");
        out
    })
}

/// One walk's result.
#[derive(Debug, Clone, Default)]
pub struct Walked {
    /// The `KFWR_R_*` bits.
    pub refuse: u32,
    /// The SPARSE census.
    pub sparse: u32,
    /// The walk stopped (budget).
    pub stop: u32,
    /// The runs, as the walker emits them (MAP, VA-ascending per class).
    pub runs: Vec<KfMapRun>,
}

/// The walker process (one per test thread).
pub struct Walker {
    child: Child,
    tx: ChildStdin,
    rx: BufReader<ChildStdout>,
}

impl Walker {
    /// Spawn it.
    #[must_use]
    pub fn new() -> Self {
        let mut child = Command::new(walker_bin())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn the CPU walker");
        let tx = child.stdin.take().expect("stdin");
        let rx = BufReader::new(child.stdout.take().expect("stdout"));
        Walker { child, tx, rx }
    }

    /// Walk `mem` from `root` with the descriptor's `leaf_ps` per directory level.
    pub fn walk(&mut self, ver3: bool, leaf_ps: [u8; 5], mem: &Mem, root: u64) -> Walked {
        let mut b = Vec::with_capacity(64 + mem.pages.len() * 4104);
        b.extend_from_slice(&0x4b46_5054u32.to_le_bytes());
        let tv = if ver3 { kf_cuda::abi::KF_TBL_VER3 } else { kf_cuda::abi::KF_TBL_VER2 };
        b.extend_from_slice(&tv.to_le_bytes());
        b.extend_from_slice(&leaf_ps);
        b.extend_from_slice(&[0; 3]);
        b.extend_from_slice(&mem.len.to_le_bytes());
        b.extend_from_slice(&root.to_le_bytes());
        b.extend_from_slice(&u32::try_from(mem.pages.len()).expect("pages").to_le_bytes());
        for (off, p) in &mem.pages {
            b.extend_from_slice(&off.to_le_bytes());
            for w in p.iter() {
                b.extend_from_slice(&w.to_le_bytes());
            }
        }
        self.tx.write_all(&b).expect("walker stdin");
        self.tx.flush().expect("flush");
        let mut line = String::new();
        self.rx.read_line(&mut line).expect("walker J line");
        let f: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(f.first(), Some(&"J"), "walker said {line:?}");
        let h = |i: usize| u64::from_str_radix(f[i], 16).expect("hex");
        let mut w = Walked {
            refuse: h(1) as u32,
            sparse: h(3) as u32,
            stop: h(4) as u32,
            runs: Vec::new(),
        };
        for _ in 0..h(5) {
            line.clear();
            self.rx.read_line(&mut line).expect("walker R line");
            let r: Vec<u64> = line
                .split_whitespace()
                .skip(1)
                .map(|x| u64::from_str_radix(x, 16).expect("hex"))
                .collect();
            w.runs.push(KfMapRun {
                va: r[0],
                gpga: r[1],
                len: r[2],
                flags: r[3] as u32,
                op: KFWR_OP_MAP,
                pdb_index: 0,
            });
        }
        w
    }
}

impl Default for Walker {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Walker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ═══ the family axis ═════════════════════════════════════════════════════════════════════════════

/// The oracle's geometry of `f` (`g_kern_gmmu_nvoc.c:670-690`).
#[must_use]
pub fn geo_of(f: kf_chip::Family) -> Geo {
    match f {
        kf_chip::Family::Turing => Geo::Gp10x,
        kf_chip::Family::Ampere | kf_chip::Family::Ada => Geo::Ga10x,
        kf_chip::Family::Hopper => Geo::Gh10x,
        kf_chip::Family::Blackwell => Geo::Gb10x,
    }
}

/// The descriptor production hands the walker for `f` (`kf_qemu::device` realize).
#[must_use]
pub fn production_format(f: kf_chip::Family) -> kf_cuda::abi::KfFormat {
    crate::apply::walk_format(f)
}

/// All five families.
pub const FAMILIES: [kf_chip::Family; 5] = [
    kf_chip::Family::Turing,
    kf_chip::Family::Ampere,
    kf_chip::Family::Ada,
    kf_chip::Family::Hopper,
    kf_chip::Family::Blackwell,
];

// ═══ the encoder (the oracle's field definitions, run backwards) ═════════════════════════════════

/// A VALID PTE word.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn enc_pte(geo: Geo, ap: u64, pa: u64, kind: u8, ro: bool, vol: bool, prv: bool, atom: bool) -> u64 {
    let b = |x: bool| u64::from(x);
    if geo.ver3() {
        let pcf = b(vol) | b(prv) << 1 | b(ro) << 2 | b(atom) << 3;
        1 | (ap & 3) << 1 | pcf << 3 | u64::from(kind & 0xF) << 8 | ((pa >> 12) & ((1 << 40) - 1)) << 12
    } else {
        let a = if ap & 3 >= 2 { (pa >> 12) & ((1 << 46) - 1) } else { (pa >> 12) & ((1 << 25) - 1) };
        1 | (ap & 3) << 1 | b(vol) << 3 | b(prv) << 5 | b(ro) << 6 | b(atom) << 7 | a << 8 | u64::from(kind) << 56
    }
}

/// A (non-dual) PDE word naming a vidmem table at `a`.
#[must_use]
pub fn enc_pde(geo: Geo, ap: u64, a: u64) -> u64 {
    if geo.ver3() {
        (ap & 3) << 1 | ((a >> 12) & ((1 << 40) - 1)) << 12
    } else {
        (ap & 3) << 1 | ((a >> 12) & ((1 << 46) - 1)) << 8
    }
}

/// A dual PDE's big half naming a table at `a` (`None` = invalid).
#[must_use]
pub fn enc_big(geo: Geo, a: Option<u64>) -> u64 {
    match a {
        None => 0,
        Some(a) if geo.ver3() => 1 << 1 | ((a >> 8) & ((1 << 44) - 1)) << 8,
        Some(a) => 1 << 1 | ((a >> 8) & ((1 << 29) - 1)) << 4,
    }
}

/// A SPARSE PTE (VER2 VALID=0 VOL=1; VER3 PCF=1).
#[must_use]
pub fn enc_sparse(geo: Geo) -> u64 {
    // VER2 `_VOL 3:3` with VALID clear; VER3 `PCF 7:3 = SPARSE (1)` — the same word on both.
    let _ = geo;
    1 << 3
}

/// An UNMAPPED big PTE (VER2 PRIVILEGE=1; VER3 PCF=3).
#[must_use]
pub fn enc_unmapped(geo: Geo) -> u64 {
    if geo.ver3() { 3 << 3 } else { 1 << 5 }
}

// ═══ the guest: an image built and edited by a seeded generator ═════════════════════════════════

/// Where a leaf sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// A directory level that may hold a page (index into `Geo::dirs`).
    Dir(usize),
    /// PD0's 2 MiB page.
    Huge,
    /// A 64 KiB big PTE.
    Big,
    /// A 4 KiB small PTE.
    Small,
}

/// A guest address space under construction.
pub struct Guest {
    /// The family's oracle geometry.
    pub geo: Geo,
    /// Guest vidmem.
    pub mem: Mem,
    /// The root table.
    pub root: u64,
    next: u64,
    tables: std::collections::BTreeSet<u64>,
    /// Every entry address written as a leaf (for later edits): `(entry offset, slot, va)`.
    pub leaves: Vec<(u64, Slot, u64)>,
}

impl Guest {
    /// An empty space.
    #[must_use]
    pub fn new(geo: Geo) -> Self {
        let mut g = Guest {
            geo,
            mem: Mem {
                len: STORE,
                pages: Map::new(),
            },
            root: 0,
            next: 1 << 20,
            tables: std::collections::BTreeSet::new(),
            leaves: Vec::new(),
        };
        g.root = g.alloc(4096);
        g
    }

    fn alloc(&mut self, bytes: u64) -> u64 {
        let a = self.next.div_ceil(bytes.max(4096)) * bytes.max(4096);
        self.next = a + bytes.max(4096);
        assert!(self.next < (256 << 20), "table arena exhausted");
        self.tables.insert(a);
        a
    }

    /// The table a PDE at `entry` names, made when absent.
    fn child(&mut self, entry: u64, bytes: u64) -> u64 {
        let raw = self.mem.word(entry).unwrap_or(0);
        if let o::Pde::Table(1, a) = o::decode_pde(self.geo, raw)
            && self.tables.contains(&a)
            && bits_ok(raw, self.geo)
        {
            return a;
        }
        let a = self.alloc(bytes);
        self.mem.set(entry, enc_pde(self.geo, 1, a));
        a
    }

    /// The PD0 entry (offset) covering `va`, building the directories above it.
    fn pd0_entry(&mut self, va: u64) -> u64 {
        let dirs = self.geo.dirs();
        let mut tbl = self.root;
        for (k, &(lo, hi, _)) in dirs.iter().enumerate() {
            let i = (va >> lo) & ((1 << (hi - lo + 1)) - 1);
            let entry = tbl + i * 8;
            let child_bytes = if k + 1 < dirs.len() {
                let (l, h, _) = dirs[k + 1];
                8u64 << (h - l + 1)
            } else {
                16 << 8
            };
            tbl = self.child(entry, child_bytes);
        }
        tbl + ((va >> o::PD0_LO) & 0xFF) * 16
    }

    /// The entry offset of a leaf of `slot` at `va`, building every table above it.
    pub fn entry(&mut self, slot: Slot, va: u64) -> u64 {
        let dirs = self.geo.dirs();
        if let Slot::Dir(k) = slot {
            let mut tbl = self.root;
            for (j, &(lo, hi, _)) in dirs.iter().enumerate() {
                let i = (va >> lo) & ((1 << (hi - lo + 1)) - 1);
                if j == k {
                    return tbl + i * 8;
                }
                let (l, h, _) = dirs[j + 1];
                tbl = self.child(tbl + i * 8, 8u64 << (h - l + 1));
            }
            unreachable!("dir level {k}");
        }
        let e = self.pd0_entry(va);
        if slot == Slot::Huge {
            return e;
        }
        let lo = self.mem.word(e).unwrap_or(0);
        let hi = self.mem.word(e + 8).unwrap_or(0);
        let (mut big, mut small) = (None, None);
        if lo & 1 == 0
            && let o::Pde::Table(1, a) = o::decode_dual_big(self.geo, lo)
            && self.tables.contains(&a)
        {
            big = Some(a);
        }
        if let o::Pde::Table(1, a) = o::decode_dual_small(self.geo, hi)
            && self.tables.contains(&a)
        {
            small = Some(a);
        }
        if slot == Slot::Big && big.is_none() {
            big = Some(self.alloc(256));
        }
        if slot == Slot::Small && small.is_none() {
            small = Some(self.alloc(4096));
        }
        self.mem.set(e, enc_big(self.geo, big));
        self.mem.set(e + 8, small.map_or(0, |a| enc_pde(self.geo, 1, a)));
        let off = va & ((1 << o::PD0_LO) - 1);
        match slot {
            Slot::Big => big.expect("big") + (off >> o::BIG_LO) * 8,
            _ => small.expect("small") + (off >> o::SMALL_LO) * 8,
        }
    }

    /// Write `raw` as the leaf of `slot` at `va`.
    pub fn put(&mut self, slot: Slot, va: u64, raw: u64) {
        let e = self.entry(slot, va);
        self.mem.set(e, raw);
        self.leaves.push((e, slot, va));
    }
}

/// A PDE this generator wrote (no stray bits) — a hostile edit is never followed as ours.
fn bits_ok(raw: u64, geo: Geo) -> bool {
    let mut a = raw;
    a &= !(3 << 1);
    if geo.ver3() {
        a &= !(((1u64 << 40) - 1) << 12);
    } else {
        a &= !(((1u64 << 46) - 1) << 8);
    }
    a == 0
}

/// `log2` of a slot's page size.
#[must_use]
pub fn slot_log2(geo: Geo, s: Slot) -> u32 {
    match s {
        Slot::Dir(k) => geo.dirs()[k].0,
        Slot::Huge => o::PD0_LO,
        Slot::Big => o::BIG_LO,
        Slot::Small => o::SMALL_LO,
    }
}

/// The seeded generator.
pub struct Gen {
    /// The PRNG.
    pub rng: Rng,
}

impl Gen {
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[self.rng.below(v.len() as u64) as usize]
    }
    fn p(&mut self, pct: u64) -> bool {
        self.rng.below(100) < pct
    }

    /// A slot, any the family's geometry has a page level for, plus the ones it has not.
    pub fn slot(&mut self, geo: Geo) -> Slot {
        let n = geo.dirs().len();
        match self.rng.below(100) {
            0..=44 => Slot::Small,
            45..=69 => Slot::Big,
            70..=87 => Slot::Huge,
            88..=95 => Slot::Dir(n - 1),
            _ => Slot::Dir(n - 2),
        }
    }

    /// A page-aligned VA for a leaf of `1 << l2` bytes.
    pub fn va(&mut self, geo: Geo, l2: u32) -> u64 {
        let top = if geo.ver3() { 57 } else { 49 };
        let a = 1u64 << l2;
        // Big leaves stay inside the guest reservation (outside it the host maps one 4 KiB page
        // per mapping, D1, and a 512 MiB leaf there is 131 072 model mappings).
        if l2 >= 29 || (l2 >= 21 && self.rng.below(100) < 95) {
            return (R0 + self.rng.below(RLEN)) & !(a - 1);
        }
        let v = match self.rng.below(100) {
            0..=79 => R0 - (8 << 20) + self.rng.below(48 << 20),
            80..=86 => self.rng.below(64 << 20),
            87..=93 => (1u64 << top) - (1 + self.rng.below(4)) * a,
            _ => self.rng.next() & ((1u64 << top) - 1),
        };
        v & !(a - 1)
    }

    /// A VALID PTE word for a page of `1 << l2` bytes: every field combination, biased to the
    /// boundaries.
    pub fn pte(&mut self, geo: Geo, l2: u32) -> u64 {
        let a = 1u64 << l2;
        let ap = match self.rng.below(100) {
            0..=54 => 0,
            55..=74 => 2,
            75..=86 => 3,
            87..=91 => 1,
            _ => self.rng.below(4),
        };
        let mut pa = if ap == 0 || ap == 1 {
            match self.rng.below(100) {
                0..=79 => self.rng.below(CARVE / a) * a,
                80..=84 => CARVE.saturating_sub(a),
                85..=88 => CARVE,
                89..=91 => STORE - a.min(STORE),
                92..=94 => STORE,
                95..=96 => (1u64 << 37).saturating_sub(a),
                _ => self.rng.next() & ((1 << 40) - 1) & !(a - 1),
            }
        } else {
            match self.rng.below(100) {
                0..=44 => self.rng.below(RAM_A.1 / a) * a,
                45..=79 => RAM_B.0 + self.rng.below(RAM_B.1 / a) * a,
                80..=84 => RAM_A.1.saturating_sub(a),
                85..=89 => RAM_A.1,
                90..=93 => RAM_B.0 + RAM_B.1,
                _ => self.rng.next() & ((1 << 52) - 1) & !(a - 1),
            }
        };
        if self.p(4) {
            pa |= 0x1000 * (1 + self.rng.below(15)); // misaligned for a big page
        }
        // Straddle the carve-out base with a big page (the flat FB alias's last leaf).
        if ap == 0 && l2 > 12 && self.p(3) {
            pa = CARVE.saturating_sub(a / 2);
        }
        let kind = match self.rng.below(100) {
            0..=29 => 0x00,
            30..=49 => 0x06,
            50..=79 => self.rng.below(16) as u8,
            80..=89 => self.pick(&[0x01, 0x05, 0x08, 0x0B, 0x0E]),
            _ => self.rng.next() as u8,
        };
        let mut raw = enc_pte(geo, ap, pa, kind, self.p(25), self.p(20), self.p(12), self.p(15));
        if geo.ver3() && self.p(10) {
            raw |= 1 << 7; // the PCF's ACD bit
        }
        if self.p(4) {
            raw ^= 1 << (8 + self.rng.below(56)); // one stray bit anywhere above the flags
        }
        raw
    }

    /// An invalid PTE word.
    pub fn invalid(&mut self, geo: Geo) -> u64 {
        match self.rng.below(5) {
            0 => 0,
            1 => enc_sparse(geo),
            2 => enc_unmapped(geo),
            3 => self.rng.next() & !1,
            _ => (self.rng.below(32) << 3) & !1,
        }
    }
}

/// Build one random guest.
pub fn random_guest(g: &mut Gen, geo: Geo, leaves: usize) -> Guest {
    let mut gu = Guest::new(geo);
    for _ in 0..leaves {
        let s = g.slot(geo);
        let l2 = slot_log2(geo, s);
        let va = g.va(geo, l2);
        let raw = if g.p(85) { g.pte(geo, l2) } else { g.invalid(geo) };
        gu.put(s, va, raw);
    }
    gu
}

/// One edit of an existing guest: rewrite a leaf, clear it, re-kind it, flip a permission, make it
/// sparse, or add a new leaf.
pub fn edit(g: &mut Gen, gu: &mut Guest) {
    let geo = gu.geo;
    if gu.leaves.is_empty() || g.p(25) {
        let s = g.slot(geo);
        let l2 = slot_log2(geo, s);
        let va = g.va(geo, l2);
        let raw = g.pte(geo, l2);
        gu.put(s, va, raw);
        return;
    }
    let (e, s, _) = gu.leaves[g.rng.below(gu.leaves.len() as u64) as usize];
    let old = gu.mem.word(e).unwrap_or(0);
    let l2 = slot_log2(geo, s);
    let new = match g.rng.below(7) {
        0 => 0,
        1 => enc_sparse(geo),
        2 => g.pte(geo, l2),
        3 if geo.ver3() => old ^ (1 << (8 + g.rng.below(4))),
        3 => old ^ (1 << (56 + g.rng.below(8))),
        4 => old ^ (1 << g.pick(&[3u64, 4, 5, 6, 7])),
        5 => old ^ 1,
        _ => g.invalid(geo),
    };
    gu.mem.set(e, new);
}

// ═══ the host side and one refresh ═══════════════════════════════════════════════════════════════

/// The target: the mirror, optionally as a USER twin (it withholds privileged leaves).
pub struct Twin<'a> {
    /// The mirror.
    pub m: SimMirror<'a>,
    /// A user twin.
    pub user: bool,
}

impl MapTarget for Twin<'_> {
    fn gpu_space(&self) -> bool {
        true
    }
    fn withholds_privileged(&self) -> bool {
        self.user
    }
    fn map(&self, d: &Desired, defer: bool) -> Result<Mapped, String> {
        self.m.map(d, defer)
    }
    fn map_batch(&self, rows: &[Desired], defer: bool) -> Result<(), String> {
        self.m.map_batch(rows, defer)
    }
    fn map_sked(&self, s: &SkedRow, defer: bool) -> Result<Mapped, String> {
        MapTarget::map_sked(&self.m, s, defer)
    }
    fn unmap(&self, va: u64, defer: bool) -> Result<(), String> {
        self.m.unmap(va, defer)
    }
    fn unmap_range(&self, va: u64, len: u64, defer: bool) -> Result<(), String> {
        self.m.unmap_range(va, len, defer)
    }
    fn invalidate(&self) -> Result<(), String> {
        Ok(())
    }
    fn own_view(&self, va: u64, end: u64) -> Option<crate::ledger::OwnView> {
        self.m.own_view(va, end)
    }
    fn begin_refresh(&self) {
        self.m.begin_refresh();
    }
}

/// The run-time knobs of one case.
#[derive(Debug, Clone, Copy)]
pub struct Knobs {
    /// The family.
    pub fam: kf_chip::Family,
    /// `per_map_kind` (host ≥ 580.65.06).
    pub per_map_kind: bool,
    /// `KF3_CARRY_ATOMIC_DISABLE=1`.
    pub carry_atomic: bool,
    /// A user twin (withholds privileged leaves).
    pub user: bool,
    /// Batched maps (`KF3_NO_BATCHED_MAP` unset).
    pub batching: bool,
    /// Micro reservations below the guest ranges.
    pub low: bool,
}

/// What one refresh did, for the checks.
#[derive(Debug, Default, Clone)]
pub struct Refresh {
    /// The walk.
    pub walk: Walked,
    /// The diff's runs.
    pub runs: Vec<DiffRun>,
    /// The apply.
    pub applied: crate::apply::Applied,
}

/// One refresh: walk, diff against `com`, apply, commit.
pub fn refresh(
    w: &mut Walker,
    gu: &Guest,
    k: &Knobs,
    t: &Twin<'_>,
    com: &mut diffmodel::Committed,
) -> Refresh {
    let f = production_format(k.fam);
    let mut leaf_ps = [f.dir[0].leaf_ps, f.dir[1].leaf_ps, f.dir[2].leaf_ps, f.dir[3].leaf_ps, f.dir[4].leaf_ps];
    // `KF_PTE_FUZZ_NO_UNCODED=1`: the descriptor without PF-2's level (what a walker that predates
    // `KF_PS_UNCODED` can be handed) — for the before/after equivalence only.
    if std::env::var_os("KF_PTE_FUZZ_NO_UNCODED").is_some() {
        for l in &mut leaf_ps {
            if *l == kf_cuda::abi::KF_PS_UNCODED {
                *l = kf_cuda::abi::KF_PS_NONE;
            }
        }
    }
    let walk = w.walk(f.table_version == kf_cuda::abi::KF_TBL_VER3, leaf_ps, &gu.mem, gu.root);
    let pol = PermPolicy {
        carry_atomic_disable: k.carry_atomic,
    };
    let d = diffmodel::diff_with(com, &walk.runs, 1 << 20, pol.key_perm());
    let runs: Vec<DiffRun> = d.runs.iter().map(|r| pol.diff_run(r)).collect();
    let ro = |g: u64, l: u64| ram_offset(g, l);
    let cfg = ApplyCfg {
        store_bytes: STORE,
        grain: P,
        ram_offset: &ro,
        usermode: k.fam.usermode_mmio(),
        per_map_kind: k.per_map_kind,
        carve: CARVE,
        carve_refuse: true,
    };
    let applied = apply_entry(t, &runs, &cfg);
    let codes: Vec<AckCode> = applied
        .codes
        .iter()
        .map(|&c| match c {
            KFWR_ACK_APPLIED => AckCode::Applied,
            KFWR_ACK_HELD => AckCode::Held,
            _ => AckCode::Failed,
        })
        .collect();
    *com = diffmodel::commit(com, &d.runs, &codes);
    Refresh { walk, runs, applied }
}

// ═══ the comparison ══════════════════════════════════════════════════════════════════════════════

/// What the host should hold over one range, by the oracle and kayfabe's documented contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exp {
    /// Mapped: `(ram, backing of the range's first byte)`, the acceptable kinds, the permissions.
    Map {
        /// Guest RAM (else the store).
        ram: bool,
        /// Backing offset of `va`.
        off: u64,
        /// The exact host kind.
        kind: u8,
        /// The alternative host kind accepted (the documented `per_map_kind = false` PITCH).
        alt: u8,
        /// The permissions.
        perm: MapPerm,
    },
    /// Must be absent, and the WALKER must have refused it by name (`KFWR_R_*`).
    WalkRefused(&'static str),
    /// Must be absent, and the APPLY must have refused it by name.
    ApplyRefused(&'static str),
    /// Must be absent (INVALID, SPARSE, or withheld): a host page here is stale or forged.
    Absent(&'static str),
    /// Not checked (the oracle cannot decide, or the leaf is not memory).
    Skip(&'static str),
}

/// The expectation for one oracle range.
#[must_use]
pub fn expect(k: &Knobs, x: &Xlate) -> Vec<(u64, u64, Exp)> {
    let Xlate::Map(l) = x else {
        return vec![];
    };
    let size = 1u64 << l.ps_log2;
    let one = |e: Exp| vec![(l.va, size, e)];
    if l.ps_log2 > 29 {
        return one(Exp::WalkRefused("a page size the walker has no code for"));
    }
    if l.ap == Aperture::Peer {
        return one(Exp::WalkRefused("peer aperture"));
    }
    if l.pa & (size - 1) != 0 {
        return one(Exp::WalkRefused("misaligned leaf"));
    }
    if l.kind == o::KIND_SMSKED && l.ap != Aperture::SysCoherent {
        return one(Exp::Skip("SKED-reflected page"));
    }
    if l.kind == o::KIND_SMSKED && k.fam.usermode_mmio().is_some() {
        return one(Exp::Skip("usermode internal MMIO"));
    }
    let ram = matches!(l.ap, Aperture::SysCoherent | Aperture::SysNonCoherent);
    let off = if ram {
        match ram_offset(l.pa, size) {
            Some(o) => o,
            None => return one(Exp::ApplyRefused("not guest RAM")),
        }
    } else {
        if l.pa.checked_add(size).is_none_or(|e| e > STORE) {
            return one(Exp::WalkRefused("beyond guest vidmem"));
        }
        if l.pa >= CARVE {
            return one(Exp::ApplyRefused("firmware carve-out"));
        }
        l.pa
    };
    if l.privileged && k.user {
        return one(Exp::Absent("privileged leaf on a user twin"));
    }
    let Some(unc) = o::uncompressed(l.kind) else {
        return one(Exp::Absent("a KIND the family does not define (KIND fault on hardware)"));
    };
    let perm = MapPerm {
        read_only: l.ro,
        atomic_disable: k.carry_atomic && l.atomic_disable,
        volatile: l.vol,
    };
    let alt = if !k.per_map_kind && (unc == 0x00 || unc == 0x06) { 0x00 } else { unc };
    let e = Exp::Map { ram, off, kind: unc, alt, perm };
    if !ram && l.pa + size > CARVE {
        let below = CARVE - l.pa;
        return vec![(l.va, below, e), (l.va + below, size - below, Exp::ApplyRefused("carve-out (clipped)"))];
    }
    vec![(l.va, size, e)]
}

/// One host piece: `(va, len, ram, off, kind, perm)`.
type Piece = (u64, u64, bool, u64, u8, MapPerm);

/// The host's mirror mappings as linear pieces.
fn host_pieces(sim: &Sim) -> Vec<Piece> {
    let rm = sim.0.borrow();
    let mut out = Vec::new();
    for m in rm.maps.iter().filter(|m| !m.broken && m.owner == Owner::Mirror) {
        match m.obj {
            RAM_OBJ => out.push((m.va, m.len, true, m.off, m.attr.0, m.attr.1)),
            STORE_OBJ | FOREIGN_OBJ => out.push((m.va, m.len, false, m.off, m.attr.0, m.attr.1)),
            h => {
                let pages = rm.objs.get(&h).cloned().unwrap_or_default();
                for i in 0..m.len / P {
                    let (ram, off) = pages
                        .get(((m.off / P) + i) as usize)
                        .copied()
                        .unwrap_or((false, u64::MAX));
                    out.push((m.va + i * P, P, ram, off, m.attr.0, m.attr.1));
                }
            }
        }
    }
    out.sort_by_key(|p| p.0);
    out
}

/// A property violation, classified.
#[derive(Debug, Clone)]
pub struct Finding {
    /// Its class (the campaign's tally key).
    pub class: &'static str,
    /// What, where.
    pub what: String,
}

/// Classes that are documented KNOWN gaps (each names where): counted, never failing a run.
/// - undefined KIND → PITCH: `V3_PTE_MODEL_FUZZ.md` PF-3, pinned by
///   `findings::pf3_an_undefined_kind_maps_pitch_pinned` (fidelity; the guest reaches only its
///   own memory).
/// - depth/stencil KIND on guest RAM → PITCH: the defect PR #480 fixes (`host_pte_kind`,
///   hardware-confirmed 2026-11-18); listed only while this tree's base lacks #480 — with #480
///   applied a campaign shows zero of it (`V3_PTE_MODEL_FUZZ.md` §5). Remove the entry when #480
///   is merged here.
pub const KNOWN: &[&str] = &[
    "P1 mapped a KIND the family does not define",
    "P1 depth/stencil KIND on guest RAM mapped as another kind",
];

/// ★ Compare the host to the oracle after one refresh.
#[must_use]
pub fn compare(k: &Knobs, gu: &Guest, sim: &Sim, r: &Refresh) -> Vec<Finding> {
    let geo = gu.geo;
    let xl = o::enumerate(geo, &gu.mem, gu.root);
    let mut exp: Vec<(u64, u64, Exp)> = Vec::new();
    for &(va, len, x) in &xl {
        match x {
            Xlate::Map(_) => exp.extend(expect(k, &x)),
            Xlate::Sparse => exp.push((va, len, Exp::Absent("guest SPARSE (PR #493: the host has no SPARSE twin yet)"))),
            Xlate::Undecided(why) => exp.push((va, len, Exp::Skip(why))),
            Xlate::Unreadable(why) => exp.push((va, len, Exp::WalkRefused(why))),
        }
    }
    exp.sort_by_key(|e| e.0);
    let host = host_pieces(sim);
    let mut out = Vec::new();
    let mut push = |class: &'static str, what: String| out.push(Finding { class, what });
    // (a) every host byte: inside an oracle range that maps it, exactly.
    for &(hva, hlen, ram, off, kind, perm) in &host {
        let hend = hva + hlen;
        let mut cur = hva;
        let first = exp.partition_point(|e| e.0 + e.1 <= hva);
        for &(eva, elen, e) in &exp[first..] {
            if eva >= hend {
                break;
            }
            if eva > cur {
                push("P3 host page where the guest maps nothing", format!("host {cur:#x}..{:#x} (ram={ram} off={off:#x})", eva.min(hend)));
            }
            let (s, t) = (eva.max(hva), (eva + elen).min(hend));
            match e {
                Exp::Map { ram: er, off: eo, kind: ek, alt, perm: ep } => {
                    if er != ram || eo + (s - eva) != off + (s - hva) {
                        push("P2 wrong backing", format!("va {s:#x}: host ram={ram} off={:#x}, guest ram={er} off={:#x}", off + (s - hva), eo + (s - eva)));
                    } else if kind != ek && kind != alt {
                        let class = if ram && o::is_depth(ek) {
                            "P1 depth/stencil KIND on guest RAM mapped as another kind"
                        } else {
                            "P1 kind"
                        };
                        push(class, format!("va {s:#x}: host kind {kind:#x}, guest's uncompressed {ek:#x} (ram={ram})"));
                    } else if perm != ep {
                        push("P1 permissions", format!("va {s:#x}: host {perm:?}, guest {ep:?}"));
                    }
                }
                Exp::Skip(_) => {}
                Exp::Absent(why) => push(
                    if why.contains("privileged") { "P2 privileged leaf on a user twin" } else if why.contains("SPARSE") { "P3 host page over guest SPARSE" } else { "P1 mapped a KIND the family does not define" },
                    format!("va {s:#x}..{t:#x}: {why}; host ram={ram} off={:#x} kind={kind:#x}", off + (s - hva)),
                ),
                Exp::WalkRefused(why) | Exp::ApplyRefused(why) => push("P2 host page the host must refuse", format!("va {s:#x}..{t:#x}: {why}; host ram={ram} off={:#x}", off + (s - hva))),
            }
            cur = cur.max(t);
        }
        if cur < hend {
            push("P3 host page where the guest maps nothing", format!("host {cur:#x}..{hend:#x} (ram={ram} off={off:#x})"));
        }
    }
    // (b) every guest-mapped byte the host lacks: accounted to a refusal by name.
    let failed_over = |s: u64, t: u64| {
        r.runs.iter().zip(&r.applied.codes).any(|(x, &c)| !x.unmap && c != KFWR_ACK_APPLIED && c != KFWR_ACK_HELD && x.va < t && s < x.va.saturating_add(x.len))
    };
    for &(eva, elen, e) in &exp {
        let end = eva + elen;
        let covered: u64 = {
            let first = host.partition_point(|h| h.0 + h.1 <= eva);
            host[first..]
                .iter()
                .take_while(|h| h.0 < end)
                .map(|h| (h.0 + h.1).min(end).saturating_sub(h.0.max(eva)))
                .sum()
        };
        match e {
            Exp::Map { .. } if covered < elen => {
                if !failed_over(eva, end) && r.walk.refuse == 0 && r.walk.stop == 0 {
                    push("SILENT absence of a guest mapping", format!("va {eva:#x}+{elen:#x}: {:#x} of it unmapped, no refusal", elen - covered));
                }
            }
            Exp::WalkRefused(why) if r.walk.refuse == 0 && r.walk.stop == 0 => {
                push("SILENT walker drop (no KFWR_R_* refusal)", format!("va {eva:#x}+{elen:#x}: {why}"));
            }
            Exp::ApplyRefused(why)
                if covered == 0
                    && !failed_over(eva, end)
                    && r.walk.refuse == 0
                    // The clipped tail of a straddling leaf is absent by design and named when the
                    // leaf is placed (`carve_clipped_bytes`, logged); a later refresh that keeps
                    // the placement does not name it again.
                    && !why.contains("clipped") =>
            {
                push("SILENT apply drop (no FAILED ack)", format!("va {eva:#x}+{elen:#x}: {why}"));
            }
            _ => {}
        }
    }
    // P5: host verbs bounded by the diff.
    let verbs = r.applied.map_calls + r.applied.unmap_calls;
    if verbs > 2 * r.runs.len() + 2 {
        push("P5 host verbs beyond the diff", format!("{verbs} verbs for {} runs", r.runs.len()));
    }
    out
}

/// One case's outcome.
#[derive(Debug, Default)]
pub struct CaseOut {
    /// Findings, in order.
    pub findings: Vec<Finding>,
    /// Oracle ranges by kind (for the coverage report).
    pub ptes: u64,
    /// A digest of everything the pipeline produced (walks, acks, host image) — the before/after
    /// equivalence key.
    pub digest: u64,
    /// The family.
    pub fam: Option<kf_chip::Family>,
}

/// ★ One case: a random guest, `steps` refreshes with edits between them, then the idempotence
/// check. Deterministic in `seed`.
pub fn run_case(w: &mut Walker, seed: u64, steps: usize) -> CaseOut {
    let mut g = Gen { rng: Rng(seed | 1) };
    let fam = FAMILIES[g.rng.below(5) as usize];
    let k = Knobs {
        fam,
        per_map_kind: g.p(75),
        carry_atomic: g.p(25),
        user: g.p(50),
        batching: g.p(70),
        low: g.p(50),
    };
    let geo = geo_of(fam);
    let leaves = 1 + g.rng.below(24) as usize;
    let mut gu = random_guest(&mut g, geo, leaves);
    let sim = Sim::new(space(R0, R0 + RLEN));
    let t = Twin {
        m: SimMirror::new(&sim, k.batching, k.low),
        user: k.user,
    };
    let mut com = diffmodel::Committed::default();
    let mut out = CaseOut {
        fam: Some(fam),
        ..CaseOut::default()
    };
    let tag = |s: usize| format!("seed={seed:#x} step={s} {k:?}");
    for s in 0..steps {
        if s > 0 {
            for _ in 0..1 + g.rng.below(3) {
                edit(&mut g, &mut gu);
            }
        }
        let r = refresh(w, &gu, &k, &t, &mut com);
        out.ptes += gu.leaves.len() as u64;
        {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            out.digest.hash(&mut h);
            (r.walk.refuse, r.walk.sparse, r.walk.stop).hash(&mut h);
            for x in &r.walk.runs {
                (x.va, x.gpga, x.len, x.flags).hash(&mut h);
            }
            r.applied.codes.hash(&mut h);
            for p in host_pieces(&sim) {
                (p.0, p.1, p.2, p.3, p.4, p.5.read_only, p.5.volatile, p.5.atomic_disable).hash(&mut h);
            }
            out.digest = h.finish();
        }
        for f in compare(&k, &gu, &sim, &r) {
            out.findings.push(Finding { class: f.class, what: format!("{} [{}]", f.what, tag(s)) });
        }
        // P4: the same guest again — an empty diff when nothing was refused, the host unchanged.
        if s + 1 == steps || g.p(30) {
            let before = host_pieces(&sim);
            let r2 = refresh(w, &gu, &k, &t, &mut com);
            let clean = r.applied.refused == 0 && r.applied.priv_withheld == 0 && r.walk.refuse == 0;
            if clean && !r2.runs.is_empty() {
                out.findings.push(Finding {
                    class: "P4 an unchanged guest re-diffs",
                    what: format!("{} runs, first {:x?} [{}]", r2.runs.len(), r2.runs[0], tag(s)),
                });
            }
            if host_pieces(&sim) != before && r2.applied.mapped == 0 && r2.applied.unmapped == 0 {
                out.findings.push(Finding { class: "P4 host changed by an empty refresh", what: tag(s) });
            }
        }
    }
    out
}

fn campaign(seconds: u64, max_cases: u64, base: u64) -> (u64, u64, Map<&'static str, (usize, String)>) {
    let mut w = Walker::new();
    let t0 = std::time::Instant::now();
    let mut all: Map<&'static str, (usize, String)> = Map::new();
    let (mut cases, mut ptes) = (0u64, 0u64);
    // `KF_FUZZ_DIGEST=<file>`: one line per case, `<seed> <family> <digest>`.
    let mut dig = std::env::var_os("KF_FUZZ_DIGEST").map(|p| std::fs::File::create(p).expect("digest file"));
    while cases < max_cases && (seconds == 0 || t0.elapsed().as_secs() < seconds) {
        let seed = base.wrapping_add(cases.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let o = run_case(&mut w, seed, 4);
        if let Some(f) = dig.as_mut() {
            writeln!(f, "{seed:#x} {:?} {:#018x}", o.fam, o.digest).expect("digest write");
        }
        ptes += o.ptes;
        // Tallied as they come (a long campaign must not hold every finding in memory).
        for f in o.findings {
            let e = all.entry(f.class).or_insert((0, f.what));
            e.0 += 1;
        }
        cases += 1;
    }
    (cases, ptes, all)
}

fn report(cases: u64, ptes: u64, t: &Map<&'static str, (usize, String)>) -> String {
    let mut s = format!("{cases} cases, {ptes} leaf-steps\n");
    for (c, (n, ex)) in t {
        let known = if KNOWN.contains(c) { " (KNOWN)" } else { "" };
        s += &format!("  {n:>7}  {c}{known}\n           e.g. {ex}\n");
    }
    s
}

/// ★ The quick run of `cargo test`: a fixed seed set, a few seconds.
#[test]
fn pte_fuzz_quick() {
    let (cases, ptes, t) = campaign(0, 2000, 0x5EED_0001);
    let bad: Vec<_> = t.keys().filter(|c| !KNOWN.contains(c)).collect();
    assert!(bad.is_empty(), "PTE fuzz found new classes:\n{}", report(cases, ptes, &t));
}

/// ★ The campaign: `KF_FUZZ_SECONDS=600 cargo test -p kf-mem --release -- --ignored
/// pte_fuzz_campaign --nocapture` (`KF_FUZZ_SEED` to move the seed).
#[test]
#[ignore = "campaign: KF_FUZZ_SECONDS=<n> cargo test -p kf-mem --release -- --ignored pte_fuzz_campaign --nocapture"]
fn pte_fuzz_campaign() {
    let secs = std::env::var("KF_FUZZ_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
    let max = std::env::var("KF_FUZZ_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(u64::MAX);
    let seed = std::env::var("KF_FUZZ_SEED").ok().and_then(|v| u64::from_str_radix(v.trim_start_matches("0x"), 16).ok()).unwrap_or(0xC0FF_EE00);
    let (cases, ptes, t) = campaign(if max == u64::MAX { secs } else { 0 }, max, seed);
    println!("{}", report(cases, ptes, &t));
    let bad: Vec<_> = t.keys().filter(|c| !KNOWN.contains(c)).collect();
    assert!(bad.is_empty(), "PTE fuzz found new classes");
}

/// Run ONE hand-built guest through one refresh on `fam` (kernel twin, per-map kinds, batching) and
/// return the oracle's findings.
pub fn one_refresh(fam: kf_chip::Family, build: impl FnOnce(&mut Guest)) -> (Vec<Finding>, Refresh) {
    let k = Knobs {
        fam,
        per_map_kind: true,
        carry_atomic: false,
        user: false,
        batching: true,
        low: false,
    };
    let mut gu = Guest::new(geo_of(fam));
    build(&mut gu);
    let sim = Sim::new(space(R0, R0 + RLEN));
    let t = Twin {
        m: SimMirror::new(&sim, k.batching, k.low),
        user: k.user,
    };
    let mut com = diffmodel::Committed::default();
    let mut w = Walker::new();
    let r = refresh(&mut w, &gu, &k, &t, &mut com);
    (compare(&k, &gu, &sim, &r), r)
}

/// ★ Reduced reproductions of what the fuzz found (`V3_PTE_MODEL_FUZZ.md` §5), one test each.
#[cfg(test)]
mod findings {
    use super::*;

    /// The oracle itself: a VER2 and a VER3 leaf round-trip through the encoder and the decoder,
    /// field by field — the encoder and decoder are written separately from the same citations.
    #[test]
    fn oracle_round_trips_every_field() {
        for geo in [Geo::Gp10x, Geo::Gh10x] {
            let raw = enc_pte(geo, 3, 0x1234_5000, 0x0B, true, true, false, true);
            let l = o::decode_pte(geo, raw, 0, 12);
            assert_eq!((l.ap, l.pa, l.ro, l.vol, l.privileged, l.atomic_disable), (Aperture::SysNonCoherent, 0x1234_5000, true, true, false, true), "{geo:?}");
            assert_eq!(l.kind, 0x0B);
            assert_eq!(o::pte_state(geo, enc_sparse(geo)), o::Pte::Sparse);
            assert_eq!(o::pte_state(geo, enc_unmapped(geo)), o::Pte::Unmapped);
        }
    }

    /// ★ FINDING PF-1 (Turing): GP10X's PD1 holds NO page (`kern_gmmu_fmt_gp10x.c:74-80`; the
    /// 512 MiB page is GA10X's, `kern_gmmu_fmt_ga10x.c:55`, bound to GA10x/AD10x only by
    /// `g_kern_gmmu_nvoc.c:679-686`), so bit 0 of a TU10x PD1 entry is not a VALID bit
    /// (`gmmu_fmt.c:64-86`: a directory-only level's entry is always a PDE). The VER2 descriptor
    /// gave every VER2 family a 512 MiB leaf at PD1: on Turing such an entry became a 512 MiB host
    /// mapping of the store where the guest's MMU sees an INVALID PDE (a fault).
    #[test]
    fn pf1_turing_pd1_bit0_is_not_a_512m_page() {
        let word = enc_pte(Geo::Gp10x, 0, 0, 0x00, false, false, false, false);
        let (f, _) = one_refresh(kf_chip::Family::Turing, |g| g.put(Slot::Dir(2), R0, word));
        assert!(f.is_empty(), "Turing PD1 entry with bit 0 set was mapped: {:?}", f.first());
        // The positive control: on Ampere the same entry IS a 512 MiB page, mapped exactly.
        let (f, r) = one_refresh(kf_chip::Family::Ampere, |g| g.put(Slot::Dir(2), R0, word));
        assert!(f.is_empty() && r.applied.mapped >= 1, "Ampere 512 MiB page: {f:?} {:?}", r.applied);
    }

    /// ★ PF-1, the other half: on Turing a PD1 entry that IS a page-directory entry stays one — a
    /// valid PDE is walked (its 4 KiB leaf maps), an INVALID one maps nothing.
    #[test]
    fn pf1_turing_pd1_pdes_stay_pdes() {
        let geo = Geo::Gp10x;
        let (f, r) = one_refresh(kf_chip::Family::Turing, |g| {
            g.put(Slot::Small, R0 + 0x3000, enc_pte(geo, 0, 0x7000, 0x06, false, false, false, false));
            let e = g.entry(Slot::Dir(2), R0 + (1 << 29));
            g.mem.set(e, 0); // an INVALID PD1 entry beside the valid one
        });
        assert!(f.is_empty(), "{f:?}");
        assert_eq!((r.applied.mapped, r.applied.refused), (1, 0), "{:?}", r.applied);
    }

    /// ★ PF-1 positive controls: on every family whose PD1 holds a page, a 512 MiB leaf maps.
    #[test]
    fn pf1_pd1_page_maps_where_the_family_has_one() {
        for fam in [kf_chip::Family::Ampere, kf_chip::Family::Ada, kf_chip::Family::Hopper, kf_chip::Family::Blackwell] {
            let geo = geo_of(fam);
            let n = geo.dirs().len();
            let word = enc_pte(geo, 0, 0, 0x06, false, false, false, false);
            let (f, r) = one_refresh(fam, |g| g.put(Slot::Dir(n - 1), R0, word));
            assert!(f.is_empty() && r.applied.mapped == 1, "{fam:?}: {f:?} {:?}", r.applied);
        }
    }

    /// ★ FINDING PF-2 (Blackwell): GB10X's PD2 holds a 256 GiB page (`kern_gmmu_fmt_gb10x.c:38,60`,
    /// every non-GH100/Ampere/Ada/Turing chip: `g_kern_gmmu_nvoc.c:687-690`), which the walk's
    /// two-bit page-size code cannot name. The VER3 descriptor read a VALID PD2 entry as a PDE
    /// (VER3 PDEs have no valid bit, `kf_dir_present`): a VIDEO-aperture 256 GiB page vanished with
    /// no refusal, and a PEER-aperture one (PTE aperture 1 = PDE VIDEO) sent the walk into the
    /// page's DATA as if it were a PD1 table, publishing leaves the guest's MMU never translates.
    /// Fail closed: such an entry is refused by name (`KFWR_R_UNCODED_PAGE`).
    #[test]
    fn pf2_blackwell_256g_page_is_refused_by_name_never_misread() {
        let geo = Geo::Gb10x;
        // (a) VIDEO: must be a named refusal, not silence.
        let (f, r) = one_refresh(kf_chip::Family::Blackwell, |g| {
            g.put(Slot::Dir(2), 0, enc_pte(geo, 0, 0, 0x06, false, false, false, false));
        });
        assert!(f.is_empty(), "VIDEO 256 GiB page: {f:?}");
        assert_eq!(r.walk.refuse, kf_cuda::abi::KFWR_R_UNCODED_PAGE, "refused by name");
        // (b) PEER over a page whose first bytes look like a PD1 table holding a 512 MiB leaf.
        let (f, r) = one_refresh(kf_chip::Family::Blackwell, |g| {
            let t = 0x0300_0000;
            g.mem.set(t, enc_pte(geo, 0, 0, 0x06, false, false, false, false));
            g.put(Slot::Dir(2), 0, enc_pte(geo, 1, t, 0x00, false, false, false, false));
        });
        assert!(f.is_empty(), "PEER 256 GiB page misread as a PDE: {:?}", f.first());
        assert_eq!(r.applied.mapped, 0, "{:?}", r.applied);
        // Hopper (GH10X) has no such page: there a PD2 entry is a PDE and the same bytes are a table.
        let (f, _) = one_refresh(kf_chip::Family::Hopper, |g| {
            let t = 0x0300_0000;
            g.mem.set(t, enc_pte(Geo::Gh10x, 0, 0, 0x06, false, false, false, false));
            g.put(Slot::Dir(2), 0, enc_pte(Geo::Gh10x, 1, t, 0x00, false, false, false, false));
        });
        assert!(f.is_empty(), "Hopper PD2 is a PDE: {f:?}");
    }

    /// ★ PF-3 (fidelity, PINNED, no change): a VALID PTE whose KIND the family does not define —
    /// `INVALID` 0x07 (`tu102/dev_mmu.h:97`), any VER2 value above 0x0F, or `SMSKED_MESSAGE` on the
    /// SYS_COHERENT aperture of a family without internal-MMIO usermode (Turing–Ada) — is placed
    /// PITCH (`host_pte_kind`'s "anything unknown stays PITCH"). The hardware raises a kind fault
    /// there instead (`NV_PFAULT_FAULT_TYPE_UNSUPPORTED_KIND`, `kernel-open/nvidia-uvm/
    /// uvm_pascal_fault_buffer.c:139-140`; which values fault is `[inferred]`, the headers do not
    /// list it). The mapping names only the guest's own store or RAM, through the same bounds as
    /// any leaf, so the guest can only hurt itself; a host-side "equivalent" would be leaving the
    /// leaf absent (a host fault, Xid 31, on our twin), which buys no isolation. If this changes,
    /// this test is the place to move.
    #[test]
    fn pf3_an_undefined_kind_maps_pitch_pinned() {
        for fam in [kf_chip::Family::Turing, kf_chip::Family::Ada] {
            let geo = geo_of(fam);
            for kind in [0x07u8, 0x10, 0x7F, 0xFF] {
                let (_, r) = one_refresh(fam, |g| {
                    g.put(Slot::Small, R0, enc_pte(geo, 0, 0x40_0000, kind, false, false, false, false));
                    g.put(Slot::Small, R0 + 0x1000, enc_pte(geo, 2, 0x40_0000, kind, false, false, false, false));
                });
                assert_eq!((r.applied.mapped, r.applied.refused), (2, 0), "{fam:?} kind {kind:#x}");
            }
            for ram in [false, true] {
                for kind in [0x07u8, 0x10, 0xFF] {
                    assert_eq!(crate::apply::host_pte_kind(kind, ram, true), kf_chip::PTE_KIND_PITCH);
                }
            }
        }
        // SMSKED over SYS_COHERENT on Ada: no usermode MMIO there, so plain guest RAM, PITCH.
        let (_, r) = one_refresh(kf_chip::Family::Ada, |g| {
            g.put(Slot::Small, R0, enc_pte(Geo::Ga10x, 2, 0x40_0000, 0x0F, false, false, false, false));
        });
        assert_eq!((r.applied.mapped, r.applied.refused), (1, 0));
    }

    /// A plain 4 KiB, 64 KiB and 2 MiB leaf on every family: no finding (the harness's own
    /// positive control).
    #[test]
    fn plain_leaves_translate_exactly_on_every_family() {
        for fam in FAMILIES {
            let geo = geo_of(fam);
            let (f, r) = one_refresh(fam, |g| {
                g.put(Slot::Small, R0, enc_pte(geo, 0, 0x40_0000, 0x06, false, false, false, false));
                g.put(Slot::Big, R0 + (1 << 21), enc_pte(geo, 2, 0x80_0000, 0x00, true, false, false, false));
                g.put(Slot::Huge, R0 + (4 << 21), enc_pte(geo, 0, 0x200_0000, 0x05, false, true, false, false));
            });
            assert!(f.is_empty(), "{fam:?}: {f:?}");
            assert_eq!(r.applied.refused, 0, "{fam:?}: {:?}", r.applied.first_refusal);
        }
    }
}
