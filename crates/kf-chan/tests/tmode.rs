// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★★★★ P1+P2 inc C (`docs/design/V3_P1P2_TSPACE.md` §7) — the T-mode rewriter, GPU-free: every
//! emitted word authored, every emitted address inside a window, every emitted method allowlisted.
use kf_abi::submit::{ce, method_header_decode, method_header_inc};
use kf_chan::tmode::{
    CHUNK_BYTES, Changed, Ir, MAX_PIECES, Operand, Rows, Shadow, Span, bind, chunk, decode,
    resolve_spans,
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

/// One commit of a rows log: `(epoch, lo, hi, when)`.
type Commit = (u64, u64, u64, std::time::Instant);

/// The guest VA space's placement rows: `va -> (len, ram, off, perm)`, and — for the stale-bind
/// tests — a commit log with the current epoch (`None`: no log kept).
#[derive(Default, Clone)]
struct MockRows(
    BTreeMap<u64, (u64, bool, u64, MapPerm)>,
    Option<(u64, Vec<Commit>)>,
);
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
    fn resolve_epoch(&self, va: u64, len: u64) -> (Result<Vec<Span>, u64>, u64) {
        (self.resolve(va, len), self.1.as_ref().map_or(0, |l| l.0))
    }
    fn changed_since(&self, epoch: u64, va: u64, len: u64) -> Changed {
        let Some((_, log)) = &self.1 else {
            return Changed::Unknown;
        };
        if log.first().is_some_and(|c| c.0 > epoch + 1) {
            return Changed::Unknown; // the log no longer reaches back to `epoch`
        }
        log.iter()
            .find(|c| c.0 > epoch && c.1 < va + len && va < c.2)
            .map_or(Changed::No, |c| Changed::At(c.3))
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
/// The CE methods every tier may emit (`clc5b5.h` … `clc7b5.h`; `C8B5`+ inherit them).
const ALLOWED_CE: [u32; 10] = [
    0x248, 0x300, 0x410, 0x414, 0x418, 0x41C, 0x700, 0x704, 0x708, 0x100,
];
/// ★ Per tier (review fix 2026-10-04): `SET_SEMAPHORE_PAYLOAD_UPPER` (`0x24C`) exists from `C7B5`
/// (`clc7b5.h:53-54`), `REQ_ATTR` (`0x754`) only on `CAB5` (`clcab5.h:29-41`).
fn allowed_ce(class: u32, mm: u32) -> bool {
    ALLOWED_CE.contains(&mm) || (mm == 0x24C && class >= 0xc7b5) || (mm == 0x754 && class == 0xcab5)
}
/// The authored ADDRESS registers, as `(first, second)` pairs the perimeter always emits together.
const ADDRESS_PAIRS: [(u32, u32, bool); 5] = [
    (0x400, 0x404, false), // OFFSET_IN_UPPER, _LOWER (CE)
    (0x408, 0x40C, false), // OFFSET_OUT_UPPER, _LOWER
    (0x240, 0x244, false), // SET_SEMAPHORE_A, _B
    (0x10, 0x14, true),    // SEMAPHOREA (39:32), SEMAPHOREB
    (0x5C, 0x60, true),    // SEM_ADDR_LO, SEM_ADDR_HI (LO first)
];

/// ★ (a) every address pair decodes inside a window; (b) every other pair is allowlisted (for the
/// C7B5 tier). The bare form — the negative controls feed it a forwarder's output.
fn check(words: &[u32], w: &TWindows) -> Result<(), String> {
    check_full(words, w, 0xc7b5, None).map(|_| ())
}

/// What one bound item's operands were, as the GUEST named them (the decoded IR): a virtual side's
/// footprint must lie inside the placement rows it resolved through.
#[derive(Clone, Copy, Default)]
struct Kinds {
    src_virtual: bool,
    dst_virtual: bool,
}

impl Kinds {
    fn of(ir: &Ir) -> Kinds {
        match ir {
            Ir::Launch(l) => Kinds {
                src_virtual: matches!(l.src, Some((Operand::Virtual(_), _))),
                dst_virtual: matches!(l.dst, Some((Operand::Virtual(_), _))),
            },
            _ => Kinds::default(),
        }
    }
}

/// The register state the checker tracks from the EMITTED words — the values the engine runs with.
#[derive(Default)]
struct Regs {
    line_len: u32,
    line_count: u32,
    pitch_in: u32,
    pitch_out: u32,
    remap: u32,
    off_in: Option<u64>,
    off_out: Option<u64>,
    sema: Option<u64>,
}

/// Bytes each side of an emitted `LAUNCH_DMA` touches — derived HERE from the class header's field
/// layout (`clc7b5.h`: `LAUNCH_DMA` 1:0 transfer, 4:3 semaphore type, 9 multi-line, 10 remap, 27
/// payload size; `SET_REMAP_COMPONENTS` 2:0/6:4/10:8/14:12 DST_X..W, 17:16 component size, 21:20
/// source components, 25:24 destination components), independently of the code under test.
fn launch_footprint(r: &Regs, launch: u32) -> (Option<u64>, Option<u64>, Option<u64>) {
    let moves = launch & 3 != 0;
    let remap = launch & (1 << 10) != 0;
    let multi = launch & (1 << 9) != 0;
    let comp = u64::from((r.remap >> 16) & 3) + 1;
    let n_src = u64::from((r.remap >> 20) & 3) + 1;
    let n_dst = (r.remap >> 24) & 3;
    let reads = !remap || (0..=n_dst).any(|c| (r.remap >> (4 * c)) & 7 <= 3);
    let (se, de) = if remap {
        (comp * n_src, comp * (u64::from(n_dst) + 1))
    } else {
        (1, 1)
    };
    let lines = if multi { u64::from(r.line_count) } else { 1 };
    let ext = |elem: u64, pitch: u32| {
        let line = u64::from(r.line_len) * elem;
        if lines <= 1 {
            line
        } else {
            u64::from(pitch) * (lines - 1) + line
        }
    };
    let sema = match (launch >> 3) & 3 {
        0 => None,
        2 => Some(16),
        _ if launch & (1 << 27) != 0 => Some(8),
        _ => Some(4),
    };
    (
        (moves && reads).then(|| ext(se, r.pitch_in)),
        moves.then(|| ext(de, r.pitch_out)),
        sema,
    )
}

/// The bytes a host semaphore word touches (`clc56f.h:83-107`, `:214-244`), derived here.
fn host_sem_bytes(method: u32, word: u32) -> u64 {
    if method == 0x1C {
        if word & 0x1F == 2 && word & (1 << 24) == 0 {
            16
        } else {
            4
        }
    } else if word & 7 == 1 && word & (1 << 25) != 0 {
        16
    } else if word & (1 << 24) != 0 {
        8
    } else {
        4
    }
}

/// The window address `a`'s backing: `(guest RAM, offset)`.
fn backing(a: u64) -> (bool, u64) {
    if a >= RAM.0 {
        (true, a - RAM.0)
    } else {
        (false, a - FB.0)
    }
}

/// `[a, a+n)` (a window address) lies inside the placement rows: ONE resolved span — a run of
/// rows contiguous in both VA and backing — covers its backing whole.
fn inside_rows(rows: &MockRows, a: u64, n: u64) -> bool {
    let (ram, off) = backing(a);
    let Some((&va, &(_, _, roff, _))) = rows
        .0
        .iter()
        .find(|&(_, &(len, r, o, _))| r == ram && off >= o && off < o + len)
    else {
        return false;
    };
    let va = va + (off - roff);
    matches!(rows.resolve(va, n.max(1)).as_deref(), Ok([s]) if s.ram == ram && s.off == off)
}

/// ★ The full checker (review fix 2026-10-04): (a) every address pair inside a window WITH THE
/// ENGINE'S WHOLE FOOTPRINT, computed from the emitted footprint registers and trigger word — not
/// its first 4 bytes; (a') with `rows`, a virtual side's or a semaphore's footprint inside ONE span
/// of the rows it resolved through (so a mis-sized extent that would run past a row is caught);
/// (b) every other pair allowlisted for the bound `class`. Returns the launches checked.
fn check_full(
    words: &[u32],
    w: &TWindows,
    class: u32,
    ir: Option<(&Ir, &MockRows)>,
) -> Result<usize, String> {
    let wr = writes(words);
    let kinds = ir.map(|(i, _)| Kinds::of(i)).unwrap_or_default();
    let mut r = Regs::default();
    let mut launches = 0;
    let mut host_addr: Option<u64> = None;
    let mut i = 0;
    let place = |what: &str, a: u64, n: u64, virt: bool| -> Result<(), String> {
        if !w.contains(a, n.max(1)) {
            return Err(format!("{what} {a:#x}+{n:#x} reaches outside both windows"));
        }
        if virt
            && let Some((_, rows)) = ir
            && !inside_rows(rows, a, n)
        {
            return Err(format!(
                "{what} {a:#x}+{n:#x} runs past the placement row it resolved through"
            ));
        }
        Ok(())
    };
    while i < wr.len() {
        let (sub, mm, v) = wr[i];
        if let Some(&(a, b, host)) = ADDRESS_PAIRS.iter().find(|p| p.0 == mm) {
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
            match (host, a) {
                (true, _) => host_addr = Some(va),
                (false, 0x400) => r.off_in = Some(va),
                (false, 0x408) => r.off_out = Some(va),
                _ => r.sema = Some(va),
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
            sub <= 4 && allowed_ce(class, mm)
        };
        if !ok {
            return Err(format!(
                "({sub}, {mm:#x}) is neither allowlisted nor an authored address"
            ));
        }
        match mm {
            0x418 => r.line_len = v,
            0x41C => r.line_count = v,
            0x410 => r.pitch_in = v,
            0x414 => r.pitch_out = v,
            0x708 => r.remap = v,
            0x1C | 0x6C => {
                let a = host_addr
                    .take()
                    .ok_or(format!("host semaphore trigger {mm:#x} with no address"))?;
                place("host semaphore", a, host_sem_bytes(mm, v), true)?;
            }
            0x300 => {
                launches += 1;
                let (src, dst, sema) = launch_footprint(&r, v);
                for (what, need, at, virt) in [
                    ("source", src, r.off_in.take(), kinds.src_virtual),
                    ("destination", dst, r.off_out.take(), kinds.dst_virtual),
                    ("CE semaphore", sema, r.sema.take(), true),
                ] {
                    match (need, at) {
                        (Some(n), Some(a)) => place(what, a, n, virt)?,
                        (Some(_), None) => {
                            return Err(format!("a launch touches its {what} with no address"));
                        }
                        (None, _) => {}
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    Ok(launches)
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
        0x514, 0x6FC, 0x70C, 0x750, 0x7FC, 0x754,
    ];
    // ★ Review fix 2026-10-04: footprint-bearing values too — remap words with two destination
    // components, multi-line / remap / four-word-release / two-word-payload launches, 16-byte and
    // 64-bit host semaphore words, and addresses near a row's end.
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
        0x0103_0054,
        0x582,
        0x382,
        0x192,
        0x0800_018A,
        0x0200_0001,
        0x0100_0001,
        0x0100_0002,
        0xFF0,
        0xF_FFF0,
        0x100,
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
    let mut per_tier = std::collections::BTreeMap::<u32, (usize, usize)>::new();
    for _ in 0..20_000 {
        // ★ Review fix 2026-10-04: the bound class is drawn from all six tiers, so the rows that
        // differ by tier (`0x24C` from C7B5, `0x6FC` from C8B5, `0x754` on CAB5, the 25-bit
        // upper masks, the 57-bit `SEM_ADDR_HI`) are all exercised.
        let class = r.pick(&TIERS);
        let mut ws: Vec<(u32, u32, u32)> = vec![(SUB, 0, class)];
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
                    let n = check_full(&out, &w, class, Some((it, &rows)))
                        .unwrap_or_else(|e| panic!("{e}: {it:x?} from {ws:x?}"));
                    let t = per_tier.entry(class).or_default();
                    t.0 += out.len();
                    t.1 += n;
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
    for class in TIERS {
        let (words, launches) = per_tier.get(&class).copied().unwrap_or_default();
        assert!(
            words > 100 && launches > 0,
            "tier {class:#x} emitted {words} words, {launches} launches"
        );
    }
}

/// The six CE classes T-mode binds (`Tier`).
const TIERS: [u32; 6] = [0xc5b5, 0xc6b5, 0xc7b5, 0xc8b5, 0xc9b5, 0xcab5];

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
    // ★ (a') review fix 2026-10-04: the checker sees a FOOTPRINT past the row an operand resolved
    // through — an emitter that sized a 16-byte release, or a two-component remap, as if it were
    // its first 4 bytes. Words a perimeter bypass would emit, checked against the rows.
    let rows = MockRows::default()
        .row(VA_FB, 0x1000, false, 0x40_0000)
        .row(VA_FB + 0x1000, 0x1000, false, 0x90_0000);
    let host_ir = decode(
        &m(0, 0x10, &[2, 0xFF8, 7, 2]),
        is_ce,
        &mut Default::default(),
        None,
    )
    .unwrap()
    .remove(0);
    let at = FB.0 + 0x40_0FF8; // the row's last 8 bytes
    let mut bad = m(0, 0x10, &[(at >> 32) as u32]);
    bad.extend(m(0, 0x14, &[at as u32]));
    bad.extend(m(0, 0x18, &[7]));
    bad.extend(m(0, 0x1C, &[2])); // RELEASE, 16 bytes
    let e = check_full(&bad, &windows(), 0xc7b5, Some((&host_ir, &rows))).unwrap_err();
    assert!(e.contains("runs past the placement row"), "{e}");
    let mut fill = m(SUB, 0, &[0xc7b5]);
    fill.extend(m(SUB, ce::OFFSET_OUT_UPPER, &[2, 0xFC0]));
    fill.extend(m(
        SUB,
        ce::LAUNCH_DMA,
        &[2 | (1 << 7) | (1 << 8) | (1 << 10)],
    ));
    let fill_ir = decode(&fill, is_ce, &mut Default::default(), None)
        .unwrap()
        .remove(1);
    let at = FB.0 + 0x40_0FC0;
    let mut bad = m(SUB, ce::LINE_LENGTH_IN, &[0x10]);
    bad.extend(m(
        SUB,
        ce::SET_REMAP_COMPONENTS,
        &[4 | (5 << 4) | (3 << 16) | (1 << 24)],
    ));
    bad.extend(m(SUB, ce::OFFSET_OUT_UPPER, &[(at >> 32) as u32]));
    bad.extend(m(SUB, ce::OFFSET_OUT_UPPER + 4, &[at as u32]));
    bad.extend(m(
        SUB,
        ce::LAUNCH_DMA,
        &[2 | (1 << 7) | (1 << 8) | (1 << 10)],
    ));
    let e = check_full(&bad, &windows(), 0xc7b5, Some((&fill_ir, &rows))).unwrap_err();
    assert!(e.contains("runs past the placement row"), "{e}");
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
    // SET_REMAP_COMPONENTS: an unnamed bit (bit 31; 3, 7, 11, 15, 18-19, 22-23, 26-31 are unnamed
    // in `clc7b5.h:181-228`) is refused, never re-emitted (review fix 2026-10-04).
    for bit in [3, 15, 19, 23, 31] {
        let mut pb = m(SUB, 0, &[0xc7b5]);
        pb.extend(m(SUB, ce::SET_REMAP_COMPONENTS, &[4 | (1 << bit)]));
        assert!(
            matches!(
                run(&pb, &MockRows::default()),
                Err(Refusal::FieldValue { .. })
            ),
            "remap bit {bit}"
        );
    }
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
    // Hopper: SET_MEMORY_SCRUB_PARAMETERS accepted; CeUtils' fast scrub — its pattern in
    // CONST_A/CONST_B, one-byte components (`channelFillPbFastScrub`, `channel_utils.c:617-628`) —
    // becomes a remap fill of the same bytes with the same pattern.
    let mut pb = m(SUB, 0, &[0xc8b5]);
    pb.extend(m(SUB, ce::SET_REMAP_CONST_A, &[0]));
    pb.extend(m(SUB, 0x704, &[0]));
    pb.extend(m(SUB, ce::SET_REMAP_COMPONENTS, &[4 | 5]));
    pb.extend(m(SUB, 0x6FC, &[0]));
    pb.extend(m(SUB, ce::SET_DST_PHYS_MODE, &[0]));
    pb.extend(m(SUB, ce::OFFSET_OUT_UPPER, &[0, 0x20_0000]));
    pb.extend(m(SUB, ce::LINE_LENGTH_IN, &[0x1000]));
    pb.extend(m(
        SUB,
        ce::LAUNCH_DMA,
        &[2 | (1 << 7) | (1 << 8) | (1 << 12) | (1 << 13) | (1 << 23) | (1 << 26)],
    ));
    let out = run(&pb, &rows).unwrap();
    check_full(&out, &windows(), 0xc8b5, None).unwrap();
    let wr = writes(&out);
    let l = wr.iter().find(|w| w.1 == ce::LAUNCH_DMA).unwrap().2;
    assert_eq!(l & (1 << 23), 0, "no fast scrub reaches the engine");
    assert_ne!(l & (1 << 10), 0, "a remap fill");
    assert!(wr.contains(&(SUB, ce::SET_REMAP_CONST_A, 0)));
    assert!(
        wr.contains(&(SUB, ce::LINE_LENGTH_IN, 0x1000)),
        "one-byte elements"
    );
}

/// Decode and bind `pb` on a channel bound to `class`, checking every bound item's output with
/// the full footprint checker against `rows`.
fn run_checked(pb: &[u32], rows: &MockRows, class: u32) -> Result<Vec<u32>, Refusal> {
    let ir = decode(pb, is_ce, &mut Default::default(), None)?;
    let mut all = Vec::new();
    for it in &ir {
        let mut out = Vec::new();
        bind(it, rows, &windows(), &mut out)?;
        check_full(&out, &windows(), class, Some((it, rows)))
            .unwrap_or_else(|e| panic!("{e}: {it:x?}"));
        all.extend(out);
    }
    Ok(all)
}

/// ★★ Review fix 2026-10-04 (`V3_P1P2_TSPACE.md` §3.4, §3.8): **a Hopper+ fast scrub keeps the
/// guest's pattern.** UVM's `memset_8` puts a 64-bit value in `CONST_A`/`CONST_B` with
/// `DST_X = CONST_A`, `DST_Y = CONST_B`, four-byte components ×2, and scrubs `LINE_LENGTH_IN`
/// BYTES (`uvm_hopper_ce.c:232-258`, `:298-310`); T-mode authors a remap fill of the same bytes
/// with the same pattern — never a zero fill — counting the line in 8-byte elements. A scrub whose
/// remap reads a source, or whose bytes are not whole elements, is refused by name.
#[test]
fn a_hopper_scrub_keeps_the_guests_pattern() {
    let value: u64 = 0x0123_4567_89AB_CDEF;
    let remap8 = 4 | (5 << 4) | (3 << 16) | (1 << 24);
    let scrub = |remap: u32, bytes: u32| {
        let mut pb = m(SUB, 0, &[0xc8b5]);
        pb.extend(m(
            SUB,
            ce::SET_REMAP_CONST_A,
            &[value as u32, (value >> 32) as u32, remap],
        ));
        pb.extend(m(SUB, 0x6FC, &[0]));
        pb.extend(m(SUB, ce::SET_DST_PHYS_MODE, &[0]));
        pb.extend(m(SUB, ce::OFFSET_OUT_UPPER, &[0, 0x20_0000]));
        pb.extend(m(SUB, ce::LINE_LENGTH_IN, &[bytes]));
        // NON_PIPELINED | both PITCH | DST PHYSICAL | MEMORY_SCRUB | DISABLE_PLC (UVM's word).
        pb.extend(m(
            SUB,
            ce::LAUNCH_DMA,
            &[2 | (1 << 7) | (1 << 8) | (1 << 13) | (1 << 23) | (1 << 26)],
        ));
        run_checked(&pb, &MockRows::default(), 0xc8b5)
    };
    let wr = writes(&scrub(remap8, 0x1000).unwrap());
    assert!(
        wr.contains(&(SUB, ce::SET_REMAP_CONST_A, 0x89AB_CDEF)),
        "{wr:x?}"
    );
    assert!(
        wr.contains(&(SUB, 0x704, 0x0123_4567)),
        "CONST_B: the pattern's high word"
    );
    assert!(wr.contains(&(SUB, ce::SET_REMAP_COMPONENTS, remap8)));
    assert!(
        wr.contains(&(SUB, ce::LINE_LENGTH_IN, 0x200)),
        "0x1000 bytes = 0x200 eight-byte elements"
    );
    let l = wr.iter().find(|w| w.1 == ce::LAUNCH_DMA).unwrap().2;
    assert_eq!(
        l & ((1 << 23) | (3 << 12)),
        0,
        "no scrub, no physical operand"
    );
    assert_ne!(l & (1 << 10), 0, "a remap fill");
    // A remap that reads a source is no pattern; bytes not whole elements are refused.
    assert!(matches!(
        scrub(0, 0x1000),
        Err(Refusal::FieldValue { what, .. }) if what.contains("reads a source")
    ));
    assert!(matches!(
        scrub(remap8, 0x1004),
        Err(Refusal::FieldValue { what, .. }) if what.contains("whole pattern elements")
    ));
}

/// ★★★ Review fix 2026-10-04 — **the engine's whole footprint stays inside the row it resolved
/// through**, for the three shapes the old 4-byte check could not see: a 16-byte release ending at
/// a row's edge (host `SEMAPHORED`, CE four-word, `SEM_EXECUTE` timestamped), a remap with two
/// destination components across a page seam, and a multi-line operand whose `PITCH` exceeds its
/// `LINE_LENGTH`. Each case is bound, then checked by the full checker (footprint computed from
/// the EMITTED registers); one byte further is refused by name.
#[test]
fn the_whole_footprint_stays_inside_its_row() {
    // VA_FB: one 4 KiB row; the next page is a DISCONTIGUOUS row (another backing).
    let rows = MockRows::default()
        .row(VA_FB, 0x1000, false, 0x40_0000)
        .row(VA_FB + 0x1000, 0x1000, false, 0x90_0000)
        .row(VA_RAM, 0x1000, true, 0x10_0000)
        .row(VA_RAM + 0x1000, 0x1000, true, 0x30_0000)
        .row(0x3_0000_0000, 0x10_0000, false, 0x60_0000);
    let edge = VA_FB + 0xFF0;
    // Host SEMAPHORED RELEASE, 16 bytes (payload + timestamp).
    let host = |va: u64| m(0, 0x10, &[(va >> 32) as u32, va as u32, 7, 2]);
    run_checked(&host(edge), &rows, 0xc7b5).expect("16 bytes at the row's end");
    assert_eq!(
        run_checked(&host(edge + 8), &rows, 0xc7b5),
        Err(Refusal::SemaphoreSpansRows {
            va: edge + 8,
            bytes: 16
        })
    );
    // SEM_EXECUTE RELEASE with RELEASE_TIMESTAMP: 16 bytes.
    let sx = |va: u64| {
        m(
            0,
            0x5C,
            &[va as u32, (va >> 32) as u32, 7, 0, 1 | (1 << 25)],
        )
    };
    run_checked(&sx(edge), &rows, 0xc7b5).expect("timestamped at the row's end");
    assert!(run_checked(&sx(edge + 8), &rows, 0xc7b5).is_err());
    // CE SEMAPHORE_TYPE 2 (four-word release): 16 bytes.
    let ce4 = |va: u64| {
        let mut pb = m(SUB, 0, &[0xc7b5]);
        pb.extend(m(
            SUB,
            ce::SET_SEMAPHORE_A,
            &[(va >> 32) as u32, va as u32, 9],
        ));
        pb.extend(m(SUB, ce::LAUNCH_DMA, &[2 << 3]));
        pb
    };
    run_checked(&ce4(edge), &rows, 0xc7b5).expect("four-word release at the row's end");
    assert!(run_checked(&ce4(edge + 8), &rows, 0xc7b5).is_err());
    // Remap, 4-byte components x2 (8-byte elements), a virtual destination 0x40 bytes before a
    // page seam: 0x10 elements = 0x80 bytes — split at the seam, 8 elements each side.
    let fill = |va: u64| {
        let mut pb = m(SUB, 0, &[0xc7b5]);
        pb.extend(m(
            SUB,
            ce::SET_REMAP_CONST_A,
            &[
                0x1111_1111,
                0x2222_2222,
                4 | (5 << 4) | (3 << 16) | (1 << 24),
            ],
        ));
        pb.extend(m(
            SUB,
            ce::OFFSET_OUT_UPPER,
            &[(va >> 32) as u32, va as u32],
        ));
        pb.extend(m(SUB, ce::LINE_LENGTH_IN, &[0x10]));
        pb.extend(m(
            SUB,
            ce::LAUNCH_DMA,
            &[2 | (1 << 7) | (1 << 8) | (1 << 10)],
        ));
        pb
    };
    let wr = writes(&run_checked(&fill(VA_RAM + 0xFC0), &rows, 0xc7b5).expect("split at the seam"));
    let lens: Vec<u32> = wr
        .iter()
        .filter(|w| w.1 == ce::LINE_LENGTH_IN)
        .map(|w| w.2)
        .collect();
    assert_eq!(
        lens,
        vec![8, 8],
        "8 eight-byte elements on each side of the seam"
    );
    // Multi-line: 4 lines of 0x40 bytes at PITCH_OUT 0x100 = 0x340 bytes, inside one row.
    let ml = |va: u64| {
        let mut pb = m(SUB, 0, &[0xc7b5]);
        pb.extend(m(
            SUB,
            ce::OFFSET_IN_UPPER,
            &[
                (va >> 32) as u32,
                va as u32 + 0x8_0000,
                (va >> 32) as u32,
                va as u32,
            ],
        ));
        pb.extend(m(SUB, 0x410, &[0x40, 0x100, 0x40, 4]));
        pb.extend(m(
            SUB,
            ce::LAUNCH_DMA,
            &[2 | (1 << 7) | (1 << 8) | (1 << 9)],
        ));
        pb
    };
    let base = 0x3_0000_0000u64;
    run_checked(&ml(base), &rows, 0xc7b5).expect("a wide-pitch multi-line copy inside its row");
    // The same copy 0x300 bytes before the row's end reaches past it.
    assert!(run_checked(&ml(base + 0x10_0000 - 0x300), &rows, 0xc7b5).is_err());
}

/// ★ Review fix 2026-10-04: a Hopper+ host `SEM_EXECUTE` whose `SEM_ADDR_HI` is wider than 8
/// bits (UVM's high kernel VAs) is RESOLVED on a wide tier — and the address emitted is a window
/// address below 2^40; on a pre-Hopper tier the same word is refused (the field is 8 bits there).
#[test]
fn a_wide_sem_addr_hi_is_resolved_on_hopper_and_refused_before() {
    let sem = VA_RAM + 0x10;
    let pb = |class: u32| {
        let mut pb = m(SUB, 0, &[class]);
        pb.extend(m(0, 0x5C, &[sem as u32, (sem >> 32) as u32, 7, 0, 1]));
        pb
    };
    for class in [0xc8b5, 0xc9b5, 0xcab5] {
        let wr = writes(&run_checked(&pb(class), &rows(), class).expect("wide tier"));
        let lo = wr.iter().find(|w| w.1 == 0x5C).unwrap().2;
        let hi = wr.iter().find(|w| w.1 == 0x60).unwrap().2;
        let at = (u64::from(hi) << 32) | u64::from(lo);
        assert_eq!(at, RAM.0 + 0x10_0010, "window(resolve(va)) on {class:#x}");
        assert!(hi <= 0xFF);
    }
    for class in [0xc5b5, 0xc6b5, 0xc7b5] {
        assert!(
            matches!(run(&pb(class), &rows()), Err(Refusal::FieldValue { .. })),
            "{class:#x}"
        );
    }
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

/// ★ §7 test 7, the other shape: a segment that is ONE long run of address-free methods (no
/// trigger to cut at) is still cut into pieces under the cap — the decoder bounds every
/// [`Ir::Words`] item.
#[test]
fn a_long_run_of_address_free_methods_is_still_chunked() {
    let mut pb = Vec::new();
    while pb.len() * 4 < 256 << 10 {
        pb.extend(m(0, 0x78, &[0])); // WFI
    }
    let ir = decode(&pb, is_ce, &mut Default::default(), None).unwrap();
    let pieces = chunk(&ir, |it, out| {
        bind(it, &rows(), &windows(), out).map(|_| ())
    })
    .unwrap();
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

/// ★★ Review fix 2026-10-04 — **the shadow binds a segment's post-invalidate items after the
/// split, as T-mode does.** UVM writes PTEs, invalidates and uses the new mapping in ONE push:
/// the item after the invalidate must not be bound at fetch (its row does not exist until the
/// walk), so the shadow holds it and binds it at the next observed segment — by when the default
/// path has run the split. ⊘ Negative control: binding it at fetch counts a resolution miss.
#[test]
fn the_shadow_binds_after_the_segments_split() {
    let new_va = 0x6_0000_0000u64;
    let mut pb = m(SUB, 0, &[0xc7b5]);
    pb.extend(m(0, 0x28, &[0, 0, 0x0020_1000, 9 << 27])); // TLB invalidate (a split)
    pb.extend(m(
        SUB,
        ce::OFFSET_IN_UPPER,
        &[(new_va >> 32) as u32, new_va as u32, 0, 0x20_0000],
    ));
    pb.extend(m(SUB, ce::SET_DST_PHYS_MODE, &[0]));
    pb.extend(m(SUB, ce::LINE_LENGTH_IN, &[0x100]));
    pb.extend(m(SUB, ce::LAUNCH_DMA, &[0x182 | (1 << 13)]));
    let before = rows(); // at fetch: the new mapping is not placed yet
    let after = rows().row(new_va, 0x1000, false, 0x70_0000); // the split's walk placed it
    let mut sh = Shadow::default();
    sh.observe(&pb, is_ce, &before, &windows());
    assert_eq!(
        (sh.launches, sh.resolve_miss),
        (0, 0),
        "held, not bound at fetch"
    );
    assert!(sh.line().contains("unbound_at_free=1"), "{}", sh.line());
    // The next segment arrives only after the split ran: the held launch binds to the new row.
    sh.observe(&m(0, 0x78, &[0]), is_ce, &after, &windows());
    assert_eq!(
        (sh.launches, sh.resolve_miss, sh.bound_after_split),
        (1, 0, 1),
        "{}",
        sh.line()
    );
    assert!(sh.line().contains("unbound_at_free=0"));
    // ⊘ Negative control: the same launch bound at fetch misses.
    let ir = decode(&pb, is_ce, &mut Default::default(), None).unwrap();
    let launch = ir.iter().find(|i| matches!(i, Ir::Launch(_))).unwrap();
    assert!(matches!(
        bind(launch, &before, &windows(), &mut Vec::new()),
        Err(Refusal::VirtualUnresolved { .. })
    ));
}

/// ★ Review fix 2026-10-04 — **`KF3_NEGCTL_SHADOW`, the shadow counters' positive control**: with
/// it every observed segment moves `unclassified`, `unknown_field` and `resolve_miss` through the
/// real decode and bind, and the channel's own decode is unaffected; without it a clean segment
/// moves none.
#[test]
fn the_shadow_positive_control_moves_every_counter() {
    let clean = m(0, 0x78, &[0]);
    let mut sh = Shadow::default();
    sh.observe(&clean, is_ce, &rows(), &windows());
    assert_eq!(
        (sh.unclassified, sh.unknown_field, sh.resolve_miss),
        (0, 0, 0)
    );
    let mut sh = Shadow::with_negctl(true);
    sh.observe(&clean, is_ce, &rows(), &windows());
    assert_eq!(
        (sh.unclassified, sh.unknown_field, sh.resolve_miss),
        (1, 1, 1),
        "{}",
        sh.line()
    );
    assert_eq!(sh.items, 1, "the real segment still bound");
    assert!(sh.line().ends_with("NEGCTL"));
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

/// ★ §7 test 13 — **the stale-bind counter** (review fix 2026-10-04): a bound resolution is
/// checked at its fence's retire against the rows' COMMIT LOG — a change committed while the
/// runner last saw the fence INCOMPLETE is `stale` (the gated count); one committed after that (the
/// guest saw its release, then unmapped and remapped — what UVM does routinely) is `late`, never
/// gated; no change, or a change elsewhere, is neither; a log that no longer reaches the bind is
/// `indeterminate`. ⊘ The positive control (`KF3_NEGCTL_STALE_BIND`) makes `stale` move.
#[test]
fn the_stale_bind_counter_and_its_positive_control() {
    use kf_chan::tmode::{StaleBind, bind_rec};
    use std::time::{Duration, Instant};
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
    let t0 = Instant::now();
    let at = |ms: u64| t0 + Duration::from_millis(ms);
    // Rows at epoch 7 when bound; a commit log with what happened after.
    let rows_with = |log: Vec<Commit>, first: u64| {
        let mut r = MockRows::default().row(VA_FB, 0x1000, false, 0x40_0000);
        let mut l = vec![(first, 0, 0, t0)];
        l.extend(log);
        r.1 = Some((7, l));
        r
    };
    let bound = |rows: &MockRows| {
        let mut rec = Vec::new();
        for it in &ir {
            bind_rec(it, rows, &windows(), &mut Vec::new(), &mut rec).unwrap();
        }
        assert!(
            rec.iter().all(|b| b.epoch == 7),
            "the epoch is recorded at bind"
        );
        rec
    };
    // (log after the bind, negctl) -> (stale, late, indeterminate), with the fence pushed at 0 ms,
    // seen incomplete at 10 ms and complete at 20 ms.
    let over = |lo: u64| (lo, lo + 0x1000); // the row both operands resolve through
    type Case = (Vec<Commit>, u64, bool, (u64, u64, u64));
    let cases: [Case; 6] = [
        (vec![], 7, false, (0, 0, 0)),
        // A walk changed the operand's rows at 5 ms: the fence was still incomplete at 10 ms.
        (
            vec![(8, over(VA_FB).0, over(VA_FB).1, at(5))],
            7,
            false,
            (2, 0, 0),
        ),
        // The same change at 15 ms — after the last incomplete reading: late, not gated.
        (
            vec![(8, over(VA_FB).0, over(VA_FB).1, at(15))],
            7,
            false,
            (0, 2, 0),
        ),
        // A change somewhere else: nothing.
        (
            vec![(8, 0x9_0000_0000, 0x9_0000_1000, at(5))],
            7,
            false,
            (0, 0, 0),
        ),
        // The log no longer reaches back to the bind's epoch.
        (vec![], 9, false, (0, 0, 2)),
        // The positive control: stale with nothing changed.
        (vec![], 7, true, (2, 0, 0)),
    ];
    for (log, first, negctl, want) in cases {
        let rows = rows_with(log, first);
        let mut s = StaleBind::default();
        s.negctl = negctl;
        s.record(3, bound(&rows), t0);
        s.observe(2, &rows, at(10));
        assert_eq!(s.checked, 0, "fence 3 has not completed at 2");
        s.observe(3, &rows, at(20));
        assert_eq!(s.checked, 2);
        assert_eq!(
            (s.stale, s.late, s.indeterminate),
            want,
            "negctl={negctl} first={first}"
        );
    }
}

/// ★ Review fix 2026-10-04 (§3.5) — **an operand with no row yet is handed back, not refused,
/// by the pusher**: everything bound before it is pushed, the rest comes back UNBOUND with the
/// refusal the runner makes only when nothing can bring the row ([`kf_chan::host::wait_on`]: a
/// walk pending on the space, else an unfinished host acquire ahead of it).
#[test]
fn an_unresolved_operand_is_handed_back_unbound() {
    use kf_chan::host::{WaitOn, wait_on};
    use kf_chan::tmode::{Pushed, is_acquire, push_bound};
    let late_va = 0x6_0000_0000u64;
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
    pb.extend(copy(VA_FB));
    // A host acquire (SEM_EXECUTE ACQ_CIRC_GEQ) on a semaphore another channel releases.
    pb.extend(m(
        0,
        0x5C,
        &[VA_FB as u32 + 0x80, (VA_FB >> 32) as u32, 1, 0, 3],
    ));
    pb.extend(copy(late_va)); // its row arrives only with the other channel's walk
    let ir = decode(&pb, is_ce, &mut Default::default(), None).unwrap();
    assert_eq!(ir.iter().filter(|i| is_acquire(i)).count(), 1);
    let mut pushes = Vec::new();
    let r = push_bound(&ir, &rows(), &windows(), &mut Vec::new(), |w| {
        pushes.push(w.to_vec());
        Ok(true)
    })
    .unwrap();
    let Pushed::Unresolved { rest, pieces, why } = r else {
        panic!("{r:?}")
    };
    assert_eq!(pieces, 1, "everything before it was pushed");
    assert!(matches!(why, Refusal::VirtualUnresolved { va, .. } if va == late_va));
    assert!(
        matches!(rest[0], Ir::Launch(_)),
        "the unresolved item heads the rest, unbound"
    );
    // Re-pushed after the walk landed its row: bound to it.
    let after = rows().row(late_va, 0x1000, false, 0x70_0000);
    let r = push_bound(&rest, &after, &windows(), &mut Vec::new(), |_| Ok(true)).unwrap();
    assert!(matches!(r, Pushed::All { .. }));
    // The runner's decision.
    assert_eq!(wait_on(true, false), Some(WaitOn::Walk));
    assert_eq!(wait_on(true, true), Some(WaitOn::Walk));
    assert_eq!(wait_on(false, true), Some(WaitOn::Acquire));
    assert_eq!(wait_on(false, false), None, "unmapped: refused by name");
}
