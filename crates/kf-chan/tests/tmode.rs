// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★★★★ P1+P2 inc C (`docs/design/V3_P1P2_TSPACE.md` §7) — the T-mode rewriter, GPU-free: every
//! emitted word authored, every emitted address inside a window, every emitted method allowlisted.
use kf_abi::submit::{ce, method_header_decode, method_header_inc};
use kf_chan::tmode::{
    CHUNK_BYTES, Ir, MAX_PIECES, Rows, Shadow, Span, bind, chunk, decode, resolve_spans,
};
use kf_chan::translated::{CeState, Refusal, Target, Window, rewrite};
use kf_chan::tspace_unsafe::TWindows;
use kf_host::MapPerm;
use std::collections::BTreeMap;

const FB: (u64, u64) = (0x1_2000_0000, 0x2_EFBE_0000); // the store window, `[0, carve)`
const RAM: (u64, u64) = (0x4_1000_0000, 0x2_0000_0000);
const LIMIT: u64 = (1 << 40) - (4 << 30);
const CARVE: u64 = FB.1;
const FB_LEN: u64 = CARVE + 0x1042_0000;
const SUB: u32 = 4;

fn windows() -> TWindows {
    TWindows::new(FB, RAM, LIMIT).unwrap()
}

/// The guest VA space's placement rows: `va -> (len, ram, off, perm)`.
#[derive(Default, Clone)]
struct MockRows(BTreeMap<u64, (u64, bool, u64, MapPerm)>);
impl MockRows {
    fn row(mut self, va: u64, len: u64, ram: bool, off: u64) -> Self {
        self.0.insert(va, (len, ram, off, MapPerm::READ_WRITE));
        self
    }
    fn row_perm(mut self, va: u64, len: u64, ram: bool, off: u64, perm: MapPerm) -> Self {
        self.0.insert(va, (len, ram, off, perm));
        self
    }
}
impl Rows for MockRows {
    fn resolve(&self, va: u64, len: u64) -> Result<Vec<Span>, u64> {
        resolve_spans(va, len, |at| {
            let (&start, &(n, ram, off, perm)) = self.0.range(..=at).next_back()?;
            (at < start + n).then(|| (ram, off + (at - start), start + n - at, perm))
        })
    }
    fn dma_to_file_range(&self, dma: u64, len: u64) -> Option<u64> {
        (dma.checked_add(len)? <= RAM.1).then_some(dma)
    }
}

fn is_ce(c: u32) -> bool {
    matches!(c, 0xc5b5 | 0xc6b5 | 0xc7b5 | 0xc8b5 | 0xc9b5 | 0xcab5)
}

fn m(sub: u32, method: u32, args: &[u32]) -> Vec<u32> {
    let mut v = vec![method_header_inc(sub, method, args.len() as u32).unwrap()];
    v.extend_from_slice(args);
    v
}

/// Decode and bind a whole segment (every item bound now).
fn run(pb: &[u32], rows: &dyn Rows) -> Result<Vec<u32>, Refusal> {
    let ir = decode(pb, is_ce, &mut Default::default(), None)?;
    let mut out = Vec::new();
    for it in &ir {
        bind(it, rows, &windows(), &mut out)?;
    }
    Ok(out)
}

/// Every `(subchannel, method, value)` write in normalised output.
fn writes(words: &[u32]) -> Vec<(u32, u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let h = method_header_decode(words[i]).unwrap();
        assert_eq!(h.arg_words, 1, "T-mode emits one method per header");
        out.push((h.subchannel, h.method, words[i + 1]));
        i += 2;
    }
    out
}

/// ★ The EXPLICIT allowlist of what T-mode may emit — written here, independently of the tables
/// under test, so a table that lets an unclassified address method through FAILS (b).
const ALLOWED_HOST: [u32; 15] = [
    0x00, 0x18, 0x1C, 0x20, 0x24, 0x28, 0x2C, 0x30, 0x34, 0x50, 0x64, 0x68, 0x6C, 0x78, 0x80,
];
const ALLOWED_CE: [u32; 12] = [
    0x248, 0x24C, 0x300, 0x410, 0x414, 0x418, 0x41C, 0x700, 0x704, 0x708, 0x754, 0x100,
];
/// The authored ADDRESS registers, as `(first, second)` pairs the perimeter always emits together.
const ADDRESS_PAIRS: [(u32, u32, bool); 5] = [
    (0x400, 0x404, false), // OFFSET_IN_UPPER, _LOWER (CE)
    (0x408, 0x40C, false), // OFFSET_OUT_UPPER, _LOWER
    (0x240, 0x244, false), // SET_SEMAPHORE_A, _B
    (0x10, 0x14, true),    // SEMAPHOREA (39:32), SEMAPHOREB
    (0x5C, 0x60, true),    // SEM_ADDR_LO, SEM_ADDR_HI (LO first)
];

/// ★ (a) every address pair decodes inside a window; (b) every other pair is allowlisted.
fn check(words: &[u32], w: &TWindows) -> Result<(), String> {
    let wr = writes(words);
    let mut i = 0;
    while i < wr.len() {
        let (sub, mm, v) = wr[i];
        if let Some(&(a, b, _)) = ADDRESS_PAIRS.iter().find(|p| p.0 == mm) {
            let Some(&(s2, m2, v2)) = wr.get(i + 1) else {
                return Err(format!("address register {mm:#x} without its partner"));
            };
            if s2 != sub || m2 != b {
                return Err(format!("address register {mm:#x} followed by {m2:#x}"));
            }
            let va = if a == 0x5C {
                (u64::from(v2) << 32) | u64::from(v)
            } else {
                (u64::from(v) << 32) | u64::from(v2)
            };
            if !w.contains(va, 4) {
                return Err(format!("address {va:#x} ({mm:#x}) is outside both windows"));
            }
            i += 2;
            continue;
        }
        if ADDRESS_PAIRS.iter().any(|p| p.1 == mm) {
            return Err(format!("orphan address register {mm:#x}"));
        }
        let ok = if mm < 0x100 {
            ALLOWED_HOST.contains(&mm)
        } else {
            sub <= 4 && ALLOWED_CE.contains(&mm)
        };
        if !ok {
            return Err(format!(
                "({sub}, {mm:#x}) is neither allowlisted nor an authored address"
            ));
        }
        i += 1;
    }
    Ok(())
}

/// A deterministic PRNG (xorshift) — the streams are random but reproducible.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[(self.next() % v.len() as u64) as usize]
    }
}

const VA_FB: u64 = 0x2_0000_0000; // a vidmem row
const VA_RAM: u64 = 0x7f00_0000_0000; // a sysmem row (UVM's high kernel VAs)
fn rows() -> MockRows {
    MockRows::default()
        .row(VA_FB, 0x10_0000, false, 0x40_0000)
        .row(VA_RAM, 0x1000, true, 0x10_0000)
        .row(VA_RAM + 0x1000, 0x1000, true, 0x30_0000) // discontiguous next page
}

/// One random method write, covering every address register, every trigger and the refused set.
fn random_write(r: &mut Rng) -> (u32, u32, u32) {
    let host = [
        0x10, 0x14, 0x18, 0x1C, 0x20, 0x24, 0x28, 0x2C, 0x30, 0x34, 0x50, 0x5C, 0x60, 0x64, 0x68,
        0x6C, 0x78, 0x80, 0x84, 0x04, 0x7C,
    ];
    let cem = [
        0x240, 0x244, 0x248, 0x24C, 0x260, 0x264, 0x300, 0x400, 0x404, 0x408, 0x40C, 0x410, 0x414,
        0x418, 0x41C, 0x700, 0x704, 0x708, 0x21C, 0x220, 0x224, 0x254, 0x258, 0x25C, 0x140, 0x500,
        0x514, 0x6FC, 0x70C, 0x750, 0x7FC,
    ];
    let vals = [
        0,
        1,
        2,
        0x182,
        0x186,
        0x2,
        0x10,
        0x1000,
        VA_FB as u32,
        (VA_FB >> 32) as u32,
        VA_RAM as u32,
        (VA_RAM >> 32) as u32,
        0x40_0000,
        0xFFFF_FFFE,
        0x0100_0004,
    ];
    let bad = [
        0x84u32, 0x04, 0x7C, 0x21C, 0x220, 0x224, 0x254, 0x258, 0x25C, 0x140, 0x500, 0x514, 0x7FC,
    ];
    let mm = match r.next() % 20 {
        0 => r.pick(&bad),
        1..=6 => r.pick(&host),
        _ => r.pick(&cem),
    };
    let sub = if mm < 0x100 {
        r.pick(&[0, SUB])
    } else {
        r.pick(&[SUB, SUB, 0, 6])
    };
    (sub, mm, r.pick(&vals))
}

/// Encode writes with a random header form (incrementing, non-incrementing, immediate, legacy,
/// and the occasional SUB-DEVICE-MASK header).
fn encode(r: &mut Rng, ws: &[(u32, u32, u32)]) -> Vec<u32> {
    let mut v = Vec::new();
    for &(sub, mm, val) in ws {
        match r.next() % 8 {
            0 if val < 0x2000 => v.push((4 << 29) | (val << 16) | (sub << 13) | (mm >> 2)), // IMMD
            1 => {
                v.push((3 << 29) | (1 << 16) | (sub << 13) | (mm >> 2)); // NON_INC
                v.push(val);
            }
            2 => {
                v.push((1 << 18) | (mm & 0x1FFC) | (sub << 13)); // legacy GRP0 incrementing
                v.push(val);
            }
            3 if r.next().is_multiple_of(16) => v.push((1 << 16) | (1 << 4)), // SET_SUBDEVICE_MASK
            _ => v.extend(m(sub, mm, &[val])),
        }
    }
    v
}

/// ★★★★★ §7 test 1 — **`every_emitted_pair_is_allowlisted_or_an_authored_address`.** Random method
/// streams through T-mode: the EMITTED word stream is decoded and (a) every address pair lies in a
/// window, (b) every other pair is on the explicit allowlist — whatever the stream, refused or not.
#[test]
fn every_emitted_pair_is_allowlisted_or_an_authored_address() {
    let w = windows();
    let rows = rows();
    let mut r = Rng(0x9E37_79B9_7F4A_7C15);
    let (mut emitted, mut refused) = (0usize, 0usize);
    for _ in 0..20_000 {
        let mut ws: Vec<(u32, u32, u32)> = vec![(SUB, 0, 0xc7b5)];
        for _ in 0..(r.next() % 12 + 1) {
            ws.push(random_write(&mut r));
        }
        let pb = encode(&mut r, &ws);
        let ir = match decode(&pb, is_ce, &mut Default::default(), None) {
            Ok(ir) => ir,
            Err(_) => {
                refused += 1;
                continue;
            }
        };
        for it in &ir {
            let mut out = Vec::new();
            match bind(it, &rows, &w, &mut out) {
                Ok(_) => {
                    emitted += out.len();
                    check(&out, &w).unwrap_or_else(|e| panic!("{e}: {it:x?} from {ws:x?}"));
                }
                Err(_) => {
                    refused += 1;
                    assert!(out.is_empty(), "a refused item emits nothing");
                }
            }
        }
    }
    assert!(
        emitted > 3_000 && refused > 100,
        "the streams exercised both arms: {emitted} {refused}"
    );
}

/// ★ §7 test 1's NEGATIVE CONTROLS — the checker can SEE both defect classes: (a) a guest VA
/// forwarded as written by today's rewriter, (b) a method a forwarder passes through.
#[test]
fn the_property_checker_catches_a_forwarder() {
    struct Wn;
    impl Window for Wn {
        fn translate(&self, _: Target, p: u64, _: u64) -> Option<u64> {
            Some(FB.0 + p)
        }
    }
    // (a) today's rewriter forwards a VIRTUAL copy's guest VAs unchanged.
    let mut pb = m(SUB, 0, &[0xc7b5]);
    pb.extend(m(SUB, ce::OFFSET_IN_UPPER, &[0x7f00, 0, 0x7f00, 0x1000]));
    pb.extend(m(SUB, ce::LAUNCH_DMA, &[0x182]));
    let out = rewrite(&pb, is_ce, &mut CeState::default(), &Wn).unwrap();
    let words: Vec<u32> = out
        .into_iter()
        .flat_map(|p| match p {
            kf_chan::translated::Piece::Words(w) => w,
            kf_chan::translated::Piece::Invalidate { .. } => vec![],
        })
        .collect();
    let e = check(&words, &windows()).unwrap_err();
    assert!(e.contains("outside both windows"), "{e}");
    // (b) a forwarder that passes SET_MONITORED_FENCE_SIGNAL_ADDR_* / SET_RENDER_ENABLE_A through.
    for mm in [0x220, 0x224, 0x254] {
        let fwd = m(SUB, mm, &[0x1000]);
        let e = check(&fwd, &windows()).unwrap_err();
        assert!(e.contains("neither allowlisted"), "{mm:#x}: {e}");
    }
    // …and a raw SUB-DEVICE-MASK header in the output is not a method pair at all.
    assert!(std::panic::catch_unwind(|| check(&[(1 << 16) | (1 << 4), 0], &windows())).is_err());
}

/// ★★ §7 test 2 — **`a_guest_address_value_is_never_emitted`**, both cases: with no covering row the
/// operand is REFUSED by name (never forwarded); with one, the emitted address is exactly
/// `window(resolve(va))` — so the property is not vacuous on a refusal.
#[test]
fn a_guest_address_value_is_never_emitted() {
    let ring_va = LIMIT + 0x10_0000; // a T-space ring-region VA no row covers
    let mut pb = m(SUB, 0, &[0xc7b5]);
    pb.extend(m(
        SUB,
        ce::OFFSET_IN_UPPER,
        &[(ring_va >> 32) as u32, ring_va as u32],
    ));
    pb.extend(m(
        SUB,
        ce::OFFSET_OUT_UPPER,
        &[(VA_FB >> 32) as u32, VA_FB as u32],
    ));
    pb.extend(m(SUB, ce::LINE_LENGTH_IN, &[0x100]));
    pb.extend(m(SUB, ce::LAUNCH_DMA, &[0x182]));
    assert_eq!(
        run(&pb, &rows()),
        Err(Refusal::VirtualUnresolved {
            va: ring_va,
            at: ring_va
        })
    );
    // Paired: a covering row — the guest's VA becomes the window address of its backing.
    let rows = rows().row(ring_va, 0x1000, false, 0x80_0000);
    let wr = writes(&run(&pb, &rows).unwrap());
    let get = |mm: u32| wr.iter().find(|w| w.1 == mm).map(|w| w.2).unwrap();
    let src = (u64::from(get(0x400)) << 32) | u64::from(get(0x404));
    let dst = (u64::from(get(0x408)) << 32) | u64::from(get(0x40C));
    assert_eq!(src, FB.0 + 0x80_0000, "window(resolve(ring_va))");
    assert_eq!(dst, FB.0 + 0x40_0000, "window(resolve(VA_FB))");
    assert!(
        !wr.iter()
            .any(|w| w.2 == ring_va as u32 || w.2 == VA_FB as u32 && w.1 >= 0x400)
    );
}

/// ★★ §7 test 3 — **`no_guest_word_is_copied`.** Reserved `LAUNCH_DMA` bits are REFUSED, never
/// cleared; the fields stock sets survive authoring; `SET_OBJECT` is the class alone; MEMBAR and L2
/// `MEM_OP` A–C are authored, a guest's non-zero A–C refused.
#[test]
fn no_guest_word_is_copied() {
    let launch_with = |w: u32| {
        let mut pb = m(SUB, 0, &[0xc7b5]);
        pb.extend(m(SUB, ce::LAUNCH_DMA, &[w]));
        run(&pb, &MockRows::default())
    };
    // VPRMODE, FORCE_RMWDISABLE, RESERVED_START_OF_COPY, RESERVED_ERR_CODE: refused.
    for bit in [11, 22, 23, 24, 28, 31] {
        assert!(
            matches!(launch_with(1 << bit), Err(Refusal::LaunchField { .. })),
            "bit {bit}"
        );
    }
    // DISABLE_PLC (26) and FLUSH_TYPE GL (25) survive (a semaphore-free, data-free launch).
    let wr = writes(&launch_with((1 << 26) | (1 << 25) | (1 << 2)).unwrap());
    let l = wr.iter().find(|w| w.1 == ce::LAUNCH_DMA).unwrap().2;
    assert_eq!(l, (1 << 26) | (1 << 25) | (1 << 2));
    // SET_OBJECT: the class alone; upper bits refused.
    let wr = writes(&run(&m(SUB, 0, &[0xc7b5]), &MockRows::default()).unwrap());
    assert_eq!(wr, vec![(SUB, 0, 0xc7b5)]);
    assert!(matches!(
        run(&m(SUB, 0, &[0x001f_c7b5]), &MockRows::default()),
        Err(Refusal::FieldValue { .. })
    ));
    // MEMBAR: authored from MEMBAR_TYPE; a non-zero MEM_OP_A refused.
    let membar = |a: u32, c: u32| run(&m(0, 0x28, &[a, 0, c, 5 << 27]), &MockRows::default());
    assert_eq!(
        writes(&membar(0, 1).unwrap()),
        vec![(0, 0x28, 0), (0, 0x2C, 0), (0, 0x30, 1), (0, 0x34, 5 << 27)]
    );
    assert!(membar(0xdead, 0).is_err() && membar(0, 7).is_err());
    // L2: A-C zero, D the operation alone.
    assert_eq!(
        writes(&run(&m(0, 0x28, &[0, 0, 0, 0xe << 27]), &MockRows::default()).unwrap()),
        vec![
            (0, 0x28, 0),
            (0, 0x2C, 0),
            (0, 0x30, 0),
            (0, 0x34, 0xe << 27)
        ]
    );
    assert!(run(&m(0, 0x28, &[0, 1, 0, 0xe << 27]), &MockRows::default()).is_err());
}

const PB_VA: u64 = 0x2_0010_0000; // CeUtils' pushbuffer (vidmem)
const FINISH: u64 = 0x800;
const SEMA: u64 = 0x810;

/// ★★ §7 test 4 — **`ceutils_shape_rewrites_both_completions`** (`channel_utils.c:600-760`): a
/// physical LOCAL_FB memset by window arithmetic, the CE release and the legacy host release both
/// at `window(resolve(pbGpuVA + off))`, the host one below 2^40.
#[test]
fn ceutils_shape_rewrites_both_completions() {
    let rows = MockRows::default().row(PB_VA, 0x1000, false, 0x10_0000);
    let fin = PB_VA + FINISH;
    let sem = PB_VA + SEMA;
    let mut pb = m(SUB, 0, &[0xc7b5]);
    pb.extend(m(SUB, ce::SET_REMAP_CONST_A, &[0]));
    pb.extend(m(
        SUB,
        ce::SET_REMAP_COMPONENTS,
        &[ce::REMAP_DST_SEL_CONST_A],
    ));
    pb.extend(m(SUB, ce::SET_DST_PHYS_MODE, &[0])); // LOCAL_FB
    pb.extend(m(SUB, ce::OFFSET_OUT_UPPER, &[0, 0x20_0000]));
    pb.extend(m(SUB, ce::LINE_LENGTH_IN, &[0x1000]));
    pb.extend(m(
        SUB,
        ce::SET_SEMAPHORE_A,
        &[(fin >> 32) as u32, fin as u32, 7],
    ));
    // NON_PIPELINED | FLUSH | one-word release | dst PITCH | REMAP | DST_TYPE PHYSICAL
    pb.extend(m(
        SUB,
        ce::LAUNCH_DMA,
        &[2 | (1 << 2) | (1 << 3) | (1 << 8) | (1 << 10) | (1 << 13)],
    ));
    // channelAddHostSema: RELEASE, 4BYTE, RELEASE_WFI DIS
    pb.extend(m(
        SUB,
        0x10,
        &[(sem >> 32) as u32, sem as u32, 9, 2 | (1 << 20) | (1 << 24)],
    ));
    let out = run(&pb, &rows).unwrap();
    check(&out, &windows()).unwrap();
    let wr = writes(&out);
    let pair = |a: u32, b: u32| {
        let i = wr.iter().position(|w| w.1 == a).unwrap();
        assert_eq!(wr[i + 1].1, b);
        (u64::from(wr[i].2) << 32) | u64::from(wr[i + 1].2)
    };
    assert_eq!(
        pair(0x408, 0x40C),
        FB.0 + 0x20_0000,
        "memset: window arithmetic"
    );
    assert_eq!(
        pair(0x240, 0x244),
        FB.0 + 0x10_0000 + FINISH,
        "CE release resolved"
    );
    let host = pair(0x10, 0x14);
    assert_eq!(host, FB.0 + 0x10_0000 + SEMA, "host release resolved");
    assert!(host < 1 << 40);
    let l = wr.iter().find(|w| w.1 == ce::LAUNCH_DMA).unwrap().2;
    assert_eq!(l & (3 << 12), 0, "both operands VIRTUAL");
    assert_eq!(
        wr.iter().find(|w| w.1 == 0x1C).unwrap().2,
        2 | (1 << 20) | (1 << 24)
    );
    // Hopper: SET_MEMORY_SCRUB_PARAMETERS accepted; the fast scrub becomes a zero fill.
    let mut pb = m(SUB, 0, &[0xc8b5]);
    pb.extend(m(SUB, 0x6FC, &[0]));
    pb.extend(m(SUB, ce::SET_DST_PHYS_MODE, &[0]));
    pb.extend(m(SUB, ce::OFFSET_OUT_UPPER, &[0, 0x20_0000]));
    pb.extend(m(SUB, ce::LINE_LENGTH_IN, &[0x1000]));
    pb.extend(m(
        SUB,
        ce::LAUNCH_DMA,
        &[2 | (1 << 7) | (1 << 8) | (1 << 12) | (1 << 13) | (1 << 23) | (1 << 26)],
    ));
    let wr = writes(&run(&pb, &rows).unwrap());
    let l = wr.iter().find(|w| w.1 == ce::LAUNCH_DMA).unwrap().2;
    assert_eq!(l & (1 << 23), 0, "no fast scrub reaches the engine");
    assert_ne!(l & (1 << 10), 0, "a remap fill");
    assert!(wr.contains(&(SUB, ce::SET_REMAP_CONST_A, 0)));
}

/// ★★ §7 test 5 — **`uvm_pte_write_inline_source_splits_at_a_page_seam`.** A virtual source over two
/// discontiguous sysmem pushbuffer pages, a physical destination: two launches, bytes preserved,
/// the first keeps the guest's transfer type, only the last carries the release / interrupt / flush
/// and is NON_PIPELINED.
#[test]
fn uvm_pte_write_inline_source_splits_at_a_page_seam() {
    let src = VA_RAM + 0xF00; // 0x100 bytes in page 0, 0x100 in page 1
    let mut pb = m(SUB, 0, &[0xc7b5]);
    pb.extend(m(SUB, ce::SET_DST_PHYS_MODE, &[0]));
    pb.extend(m(
        SUB,
        ce::OFFSET_IN_UPPER,
        &[(src >> 32) as u32, src as u32, 0, 0x20_1000],
    ));
    pb.extend(m(SUB, ce::LINE_LENGTH_IN, &[0x200]));
    pb.extend(m(
        SUB,
        ce::SET_SEMAPHORE_A,
        &[(VA_FB >> 32) as u32, VA_FB as u32, 5],
    ));
    // PIPELINED | FLUSH | one-word | NON_BLOCKING intr | both PITCH | DST PHYSICAL
    let guest = 1 | (1 << 2) | (1 << 3) | (2 << 5) | (1 << 7) | (1 << 8) | (1 << 13);
    pb.extend(m(SUB, ce::LAUNCH_DMA, &[guest]));
    let out = run(&pb, &rows()).unwrap();
    check(&out, &windows()).unwrap();
    let wr = writes(&out);
    let launches: Vec<u32> = wr
        .iter()
        .filter(|w| w.1 == ce::LAUNCH_DMA)
        .map(|w| w.2)
        .collect();
    assert_eq!(launches.len(), 2, "{wr:x?}");
    let lens: Vec<u32> = wr
        .iter()
        .filter(|w| w.1 == ce::LINE_LENGTH_IN)
        .map(|w| w.2)
        .collect();
    assert_eq!(lens, vec![0x100, 0x100], "bytes preserved");
    assert_eq!(launches[0] & 3, 1, "the first keeps the guest's PIPELINED");
    assert_eq!(launches[1] & 3, 2, "the last is NON_PIPELINED");
    assert_eq!(
        launches[0] & ((3 << 3) | (3 << 5) | (1 << 2)),
        0,
        "no release/intr/flush first"
    );
    assert_eq!(
        launches[1] & ((3 << 3) | (3 << 5) | (1 << 2)),
        (1 << 3) | (2 << 5) | (1 << 2)
    );
    let srcs: Vec<u64> = wr
        .windows(2)
        .filter(|p| p[0].1 == 0x400)
        .map(|p| (u64::from(p[0].2) << 32) | u64::from(p[1].2))
        .collect();
    assert_eq!(
        srcs,
        vec![RAM.0 + 0x10_0F00, RAM.0 + 0x30_0000],
        "each page's own backing"
    );
    let dsts: Vec<u64> = wr
        .windows(2)
        .filter(|p| p[0].1 == 0x408)
        .map(|p| (u64::from(p[0].2) << 32) | u64::from(p[1].2))
        .collect();
    assert_eq!(
        dsts,
        vec![FB.0 + 0x20_1000, FB.0 + 0x20_1100],
        "the destination advances"
    );
    assert_eq!(wr.iter().filter(|w| w.1 == 0x240).count(), 1, "one release");
}

/// ★★ §7 test 6 — **`split_binds_after_the_walk`.** `[virtual A, TLB invalidate, virtual B]`: the
/// rows change between the split and B's bind, and B binds to the NEW backing; binding at fetch
/// (the negative control) binds it to the old one.
#[test]
fn split_binds_after_the_walk() {
    let va = VA_FB + 0x1000;
    let copy = |pb: &mut Vec<u32>| {
        pb.extend(m(
            SUB,
            ce::OFFSET_IN_UPPER,
            &[
                (va >> 32) as u32,
                va as u32,
                (va >> 32) as u32,
                va as u32 + 0x800,
            ],
        ));
        pb.extend(m(SUB, ce::LINE_LENGTH_IN, &[0x100]));
        pb.extend(m(SUB, ce::LAUNCH_DMA, &[0x182]));
    };
    let mut pb = m(SUB, 0, &[0xc7b5]);
    copy(&mut pb);
    pb.extend(m(0, 0x28, &[0, 0, 0x0020_1000, 9 << 27]));
    copy(&mut pb);
    let ir = decode(&pb, is_ce, &mut Default::default(), None).unwrap();
    let split = ir
        .iter()
        .position(|i| matches!(i, Ir::Invalidate { .. }))
        .unwrap();
    let old = MockRows::default().row(VA_FB, 0x10_0000, false, 0x40_0000);
    let new = MockRows::default().row(VA_FB, 0x10_0000, false, 0x90_0000);
    let src_of = |words: &[u32]| {
        let wr = writes(words);
        let i = wr.iter().position(|w| w.1 == 0x400).unwrap();
        (u64::from(wr[i].2) << 32) | u64::from(wr[i + 1].2)
    };
    let bind_b = |rows: &MockRows| {
        let mut out = Vec::new();
        for it in &ir[split + 1..] {
            bind(it, rows, &windows(), &mut out).unwrap();
        }
        out
    };
    // ★ Bound at push, after the walk published the new rows.
    assert_eq!(src_of(&bind_b(&new)), FB.0 + 0x90_1000);
    // ⊘ Negative control: bound at fetch, before the walk — the stale backing.
    assert_eq!(src_of(&bind_b(&old)), FB.0 + 0x40_1000);
}

/// ★ §7 test 7 — **`chunking_keeps_every_piece_under_the_cap`**: a 256 KiB launch-dense segment
/// expands well past the host ring's half-pushbuffer limit, and is cut into pieces of at most
/// [`CHUNK_BYTES`].
#[test]
fn chunking_keeps_every_piece_under_the_cap() {
    let rows = rows();
    let mut pb = m(SUB, 0, &[0xc7b5]);
    pb.extend(m(SUB, ce::SET_SRC_PHYS_MODE, &[0, 0])); // SRC and DST: LOCAL_FB
    while pb.len() * 4 < 256 << 10 {
        pb.extend(m(SUB, ce::LAUNCH_DMA, &[0x182 | (1 << 12) | (1 << 13)])); // phys->phys
    }
    let ir = decode(&pb, is_ce, &mut Default::default(), None).unwrap();
    let pieces = chunk(&ir, |it, out| bind(it, &rows, &windows(), out).map(|_| ())).unwrap();
    let total: usize = pieces.iter().map(Vec::len).sum();
    assert!(
        4 * total > 416 << 10,
        "the segment expanded past the half-pushbuffer limit"
    );
    assert!(pieces.len() > 1);
    for p in &pieces {
        assert!(
            4 * p.len() <= CHUNK_BYTES,
            "a piece of {} bytes",
            4 * p.len()
        );
    }
}

/// ★★ §7 test 8 — **`refusals_by_name`** (the T-mode half; the inc-A half is in
/// `translated_rewrite.rs`).
#[test]
fn refusals_by_name() {
    let rows = rows()
        .row_perm(
            0x9_0000_0000,
            0x1000,
            false,
            0x60_0000,
            MapPerm {
                read_only: true,
                ..MapPerm::READ_WRITE
            },
        )
        .row_perm(
            0xA_0000_0000,
            0x1000,
            false,
            0x61_0000,
            MapPerm {
                atomic_disable: true,
                ..MapPerm::READ_WRITE
            },
        );
    let ce_seg = |setup: &[(u32, &[u32])], launch: u32| {
        let mut pb = m(SUB, 0, &[0xc7b5]);
        for &(mm, a) in setup {
            pb.extend(m(SUB, mm, a));
        }
        pb.extend(m(SUB, ce::LAUNCH_DMA, &[launch]));
        run(&pb, &rows)
    };
    let off = |va: u64| [(va >> 32) as u32, va as u32, (va >> 32) as u32, va as u32];
    // Block-linear virtual operands.
    assert_eq!(
        ce_seg(&[(0x400, &off(VA_FB))], 0x2),
        Err(Refusal::BlockLinearVirtual)
    );
    // A discontiguous multi-line virtual operand; a contiguous one is rewritten.
    let ml = |va: u64| {
        ce_seg(
            &[(0x400, &off(va)), (0x410, &[0x800, 0x800, 0x800, 3])],
            0x182 | (1 << 9),
        )
    };
    assert_eq!(ml(VA_RAM), Err(Refusal::MultiLineDiscontiguous));
    assert!(ml(VA_FB).is_ok());
    // A write or a release through a read-only row; a reduction through an atomic-disabled one.
    assert!(matches!(
        ce_seg(
            &[
                (0x408, &off(0x9_0000_0000)[..2]),
                (0x418, &[4]),
                (0x708, &[ce::REMAP_DST_SEL_CONST_A])
            ],
            0x182 | (1 << 10)
        ),
        Err(Refusal::ReadOnlyRow { .. })
    ));
    let sem = |va: u64, launch: u32| ce_seg(&[(0x240, &[(va >> 32) as u32, va as u32, 1])], launch);
    assert!(matches!(
        sem(0x9_0000_0000, 1 << 3),
        Err(Refusal::ReadOnlyRow { .. })
    ));
    assert!(matches!(
        sem(0xA_0000_0000, (1 << 3) | (1 << 19) | (6 << 14)),
        Err(Refusal::AtomicDisabledRow { .. })
    ));
    assert!(
        sem(0xA_0000_0000, 1 << 3).is_ok(),
        "a plain release through it is fine"
    );
    // 65 pieces: a source over 65 discontiguous pages.
    let mut many = MockRows::default();
    for k in 0..=MAX_PIECES as u64 {
        many = many.row(0x5_0000_0000 + k * 0x1000, 0x1000, true, k * 0x2000);
    }
    let mut pb = m(SUB, 0, &[0xc7b5]);
    pb.extend(m(SUB, 0x400, &[5, 0, 0, 0x20_0000]));
    pb.extend(m(SUB, 0x418, &[0x1000 * (MAX_PIECES as u32 + 1)]));
    pb.extend(m(SUB, ce::SET_DST_PHYS_MODE, &[0]));
    pb.extend(m(SUB, ce::LAUNCH_DMA, &[0x182 | (1 << 13)]));
    assert_eq!(
        run(&pb, &many),
        Err(Refusal::TooManyPieces {
            pieces: MAX_PIECES + 1
        })
    );
    // Unknown SEMAPHORED and SEM_EXECUTE operations; SUB-DEVICE-MASK; an unclassified CE method.
    assert!(matches!(
        run(&m(0, 0x10, &[0, 0, 0, 3]), &rows),
        Err(Refusal::HostSemOp { .. })
    ));
    assert!(matches!(
        run(&m(0, 0x6C, &[7]), &rows),
        Err(Refusal::HostSemOp { .. })
    ));
    let mut pb = m(SUB, 0, &[0xc7b5]);
    pb.push((1 << 16) | (1 << 4));
    assert!(matches!(
        run(&pb, &rows),
        Err(Refusal::SubDeviceMask { .. })
    ));
    let mut pb = m(SUB, 0, &[0xc7b5]);
    pb.extend(m(SUB, 0x7FC, &[0]));
    assert_eq!(
        run(&pb, &rows),
        Err(Refusal::Unclassified {
            subch: SUB,
            method: 0x7FC
        })
    );
    // An unclassified host method; INTERRUPT BLOCKING; SEMAPHORE_TYPE 3.
    assert!(matches!(
        run(&m(0, 0x7C, &[0]), &rows),
        Err(Refusal::Unclassified { .. })
    ));
    assert!(matches!(
        ce_seg(&[], 1 << 5),
        Err(Refusal::LaunchField { .. })
    ));
    assert!(matches!(
        ce_seg(&[], 3 << 3),
        Err(Refusal::LaunchField { .. })
    ));
}

/// ★★ §7 test 9 (the `kf-chan` half) — **`carve_out_is_excluded`**: a LOCAL_FB operand at the
/// carve-out, inside it, or at the store's end is refused; one just below is rewritten; a virtual
/// row that resolves into the carve-out is refused.
#[test]
fn carve_out_is_excluded() {
    let fb_op = |at: u64, len: u32| {
        let mut pb = m(SUB, 0, &[0xc7b5]);
        pb.extend(m(SUB, ce::SET_DST_PHYS_MODE, &[0]));
        pb.extend(m(
            SUB,
            ce::SET_REMAP_COMPONENTS,
            &[ce::REMAP_DST_SEL_CONST_A],
        ));
        pb.extend(m(
            SUB,
            ce::OFFSET_OUT_UPPER,
            &[(at >> 32) as u32, at as u32],
        ));
        pb.extend(m(SUB, ce::LINE_LENGTH_IN, &[len]));
        pb.extend(m(
            SUB,
            ce::LAUNCH_DMA,
            &[2 | (1 << 8) | (1 << 13) | (1 << 10)],
        ));
        run(&pb, &MockRows::default())
    };
    for at in [CARVE, CARVE + 4, FB_LEN - 4] {
        assert!(
            matches!(fb_op(at, 4), Err(Refusal::Untranslatable { .. })),
            "{at:#x}"
        );
    }
    assert!(
        matches!(fb_op(CARVE - 4, 8), Err(Refusal::Untranslatable { .. })),
        "straddles"
    );
    assert!(fb_op(CARVE - 8, 8).is_ok());
    let rows = MockRows::default().row(VA_FB, 0x1000, false, CARVE);
    let mut pb = m(SUB, 0, &[0xc7b5]);
    pb.extend(m(
        SUB,
        ce::OFFSET_OUT_UPPER,
        &[(VA_FB >> 32) as u32, VA_FB as u32],
    ));
    pb.extend(m(SUB, ce::LINE_LENGTH_IN, &[4]));
    pb.extend(m(
        SUB,
        ce::SET_REMAP_COMPONENTS,
        &[ce::REMAP_DST_SEL_CONST_A],
    ));
    pb.extend(m(SUB, ce::LAUNCH_DMA, &[0x182 | (1 << 10)])); // a remap fill: no source
    assert!(matches!(
        run(&pb, &rows),
        Err(Refusal::OutsideWindow { ram: false, .. })
    ));
}

/// ★ The shadow counts what T-mode WOULD do, by name, changing nothing.
#[test]
fn the_shadow_counts_would_refuse_and_pieces() {
    let mut sh = Shadow::default();
    let mut pb = m(SUB, 0, &[0xc7b5]);
    pb.extend(m(
        SUB,
        ce::OFFSET_IN_UPPER,
        &[(VA_RAM >> 32) as u32, VA_RAM as u32 + 0xF00, 0, 0x20_0000],
    ));
    pb.extend(m(SUB, ce::SET_DST_PHYS_MODE, &[0]));
    pb.extend(m(SUB, ce::LINE_LENGTH_IN, &[0x200]));
    pb.extend(m(SUB, ce::LAUNCH_DMA, &[0x182 | (1 << 13)]));
    sh.observe(&pb, is_ce, &rows(), &windows());
    assert_eq!((sh.launches, sh.max_pieces), (1, 2));
    sh.observe(&m(SUB, 0x7FC, &[0]), is_ce, &rows(), &windows());
    let mut miss = m(SUB, ce::OFFSET_IN_UPPER, &[0x77, 0, 0x77, 0]);
    miss.extend(m(SUB, ce::LAUNCH_DMA, &[0x182]));
    sh.observe(&miss, is_ce, &rows(), &windows());
    assert_eq!((sh.unclassified, sh.resolve_miss), (1, 1));
    let l = sh.line();
    assert!(
        l.contains("max_pieces=2")
            && l.contains("unclassified:1")
            && l.contains("virtual_unresolved:1"),
        "{l}"
    );
}

/// ★★ §7 test 6, the runner half — **a `Busy` stash re-binds at the next push.** The ring takes
/// the first piece and refuses the second: the refused items come back UNBOUND, and pushing them
/// later binds them against the rows as they are THEN. Only the pushed piece's resolutions are
/// recorded for the stale-bind counter.
#[test]
fn a_busy_stash_is_unbound_and_rebinds_at_the_next_push() {
    // Two shapes: the ring fills at the LAST piece (2 chunks), and in the middle (5 chunks).
    for chunks in [2usize, 5] {
        busy_stash(chunks);
    }
}

fn busy_stash(chunks: usize) {
    use kf_chan::tmode::{Pushed, push_bound};
    let copy = |va: u64| {
        let mut pb = m(
            SUB,
            ce::OFFSET_IN_UPPER,
            &[(va >> 32) as u32, va as u32, 0, 0x20_0000],
        );
        pb.extend(m(SUB, ce::LINE_LENGTH_IN, &[0x100]));
        pb.extend(m(SUB, ce::LAUNCH_DMA, &[0x182 | (1 << 13)]));
        pb
    };
    let mut pb = m(SUB, 0, &[0xc7b5]);
    pb.extend(m(SUB, ce::SET_DST_PHYS_MODE, &[0]));
    // Enough launches that the bound output needs `chunks` CHUNK_BYTES pieces (48 bytes each).
    let n = (chunks - 1) * CHUNK_BYTES / 48 + 16;
    for _ in 0..n {
        pb.extend(copy(VA_FB));
    }
    let ir = decode(&pb, is_ce, &mut Default::default(), None).unwrap();
    let old = MockRows::default().row(VA_FB, 0x1000, false, 0x40_0000);
    let new = MockRows::default().row(VA_FB, 0x1000, false, 0x90_0000);
    let mut pushes: Vec<Vec<u32>> = Vec::new();
    let mut rec = Vec::new();
    let r = push_bound(&ir, &old, &windows(), &mut rec, |w| {
        if pushes.is_empty() {
            pushes.push(w.to_vec());
            Ok(true)
        } else {
            Ok(false) // the ring is full
        }
    })
    .unwrap();
    let Pushed::Busy { rest, pieces } = r else {
        panic!("{r:?}")
    };
    assert_eq!(pieces, 1);
    assert!(4 * pushes[0].len() <= CHUNK_BYTES);
    assert!(!rest.is_empty() && rest.len() < ir.len());
    let recorded = rec.len();
    assert!(recorded > 0, "the pushed piece's resolutions are recorded");
    // Later — after a walk moved the row — the stash is bound against the NEW rows.
    let mut later = Vec::new();
    let r = push_bound(&rest, &new, &windows(), &mut rec, |w| {
        later.push(w.to_vec());
        Ok(true)
    })
    .unwrap();
    assert!(matches!(r, Pushed::All { .. }));
    let srcs: Vec<u64> = later
        .iter()
        .flat_map(|w| writes(w))
        .collect::<Vec<_>>()
        .windows(2)
        .filter(|p| p[0].1 == 0x400)
        .map(|p| (u64::from(p[0].2) << 32) | u64::from(p[1].2))
        .collect();
    assert!(!srcs.is_empty());
    assert!(
        srcs.iter().all(|&a| a == FB.0 + 0x90_0000),
        "re-bound to the new backing"
    );
    assert!(rec.len() > recorded);
    // Nothing lost, nothing doubled: every launch reached the ring exactly once.
    let launches = pushes
        .iter()
        .chain(later.iter())
        .flat_map(|w| writes(w))
        .filter(|w| w.1 == ce::LAUNCH_DMA)
        .count();
    assert_eq!(launches, n);
}

/// ★ §7 test 13 — **the stale-bind counter**: a resolution re-checked at retire against unchanged
/// rows is not stale; against rows a walk changed it is; and its positive control
/// (`KF3_NEGCTL_STALE_BIND`) makes it move with nothing changed.
#[test]
fn the_stale_bind_counter_and_its_positive_control() {
    use kf_chan::tmode::{StaleBind, bind_rec};
    let mut pb = m(SUB, 0, &[0xc7b5]);
    pb.extend(m(
        SUB,
        ce::OFFSET_IN_UPPER,
        &[
            (VA_FB >> 32) as u32,
            VA_FB as u32,
            (VA_FB >> 32) as u32,
            VA_FB as u32 + 0x800,
        ],
    ));
    pb.extend(m(SUB, ce::LINE_LENGTH_IN, &[0x100]));
    pb.extend(m(SUB, ce::LAUNCH_DMA, &[0x182]));
    let ir = decode(&pb, is_ce, &mut Default::default(), None).unwrap();
    let rows_a = MockRows::default().row(VA_FB, 0x1000, false, 0x40_0000);
    let rows_b = MockRows::default().row(VA_FB, 0x1000, false, 0x50_0000);
    let bound = |rows: &MockRows| {
        let mut rec = Vec::new();
        for it in &ir {
            bind_rec(it, rows, &windows(), &mut Vec::new(), &mut rec).unwrap();
        }
        rec
    };
    for (negctl, retire_rows, want_stale) in
        [(false, &rows_a, 0), (false, &rows_b, 2), (true, &rows_a, 2)]
    {
        let mut s = StaleBind::default();
        s.negctl = negctl;
        s.record(7, bound(&rows_a));
        s.retire(6, retire_rows);
        assert_eq!(s.checked, 0, "fence 7 has not completed at 6");
        s.retire(7, retire_rows);
        assert_eq!((s.checked, s.stale), (2, want_stale), "negctl={negctl}");
    }
}
