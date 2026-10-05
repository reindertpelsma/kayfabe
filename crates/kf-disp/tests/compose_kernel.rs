// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ The display plane's kernels (`cuda/display/kf_scanout.ptx`: `kf_compose`, and since
//! 2026-10-04 `kf_sum`, `display-max-fps`'s change detector) RUN ON THE HOST. The compose kernel: the committed, hand-written PTX is parsed and EXECUTED here, instruction by instruction,
//! for every (CTA, thread) of the launch kf-cuda makes (one CTA per rectangle row, 256 threads),
//! and the frame it leaves is compared byte for byte with `kf_disp::scanout::compose_reference`
//! — the CPU reference the display worker's bring-up self-test also uses on the GPU.
//!
//! The PTX is hand-written (no nvcc, no generated source to build twice, unlike
//! `kf_bl_pack.cu`), so the only way to run "the same arithmetic" on the host is to run the text
//! itself. The interpreter implements exactly the instructions the kernel uses and PANICS on any
//! other, so a kernel edit that adds one fails here until it is modelled; every load and store must
//! fall inside the source extent or the frame the launch was given, so the kernel's own bounds
//! claim ("every address this kernel forms lies in [src, src + extent)") is checked too. It is the
//! kernel's logic, not the GPU: the PTX JIT and the hardware are graded by the box self-test.

use kf_disp::pace::{digest, row_sums};
use kf_disp::scanout::{
    COMPOSE_ALPHA, COMPOSE_OPAQUE, COMPOSE_SWAP_RB, COMPOSE_XOR, LayerPlan, compose_reference,
};
use std::collections::HashMap;

const PTX: &str = include_str!("../../../cuda/display/kf_scanout.ptx");
/// `kf_sum`'s own module (a JIT refusal there must not cost the compose kernel).
const SUM_PTX: &str = include_str!("../../../cuda/display/kf_sum.ptx");
/// Threads per CTA, as `kf_cuda::display::DisplayGpu::launch_compose` launches.
const NTID: u32 = 256;
const SRC_BASE: u64 = 0x1_0000_0000;
const DST_BASE: u64 = 0x2_0000_0000;

#[derive(Debug, Clone)]
enum Op {
    Reg(String),
    Imm(i64),
    Special(&'static str),
    Mem(String),
}

#[derive(Debug, Clone)]
struct Ins {
    pred: Option<(bool, String)>,
    op: String,
    args: Vec<Op>,
}

struct Kernel {
    params: Vec<String>,
    code: Vec<Ins>,
    labels: HashMap<String, usize>,
}

fn operand(t: &str) -> Op {
    let t = t.trim();
    if let Some(inner) = t.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        return Op::Mem(inner.trim().to_string());
    }
    match t {
        "%ctaid.x" => return Op::Special("ctaid"),
        "%tid.x" => return Op::Special("tid"),
        "%ntid.x" => return Op::Special("ntid"),
        _ => {}
    }
    if t.starts_with('%') || t.starts_with('$') {
        return Op::Reg(t.to_string());
    }
    Op::Imm(
        t.parse()
            .unwrap_or_else(|_| panic!("an operand the interpreter cannot read: {t}")),
    )
}

/// The entry `name`: its parameter names in order, its instructions, its labels.
fn kernel(name: &str) -> Kernel {
    let text = if name == "kf_sum" { SUM_PTX } else { PTX };
    assert!(text.is_ascii(), "the PTX parser refuses other bytes");
    let start = text
        .find(&format!(".visible .entry {name}("))
        .expect("the entry");
    let rest = &text[start..];
    let (head, body) = rest.split_once('{').expect("a body");
    let params = head
        .split_once('(')
        .unwrap()
        .1
        .split(')')
        .next()
        .unwrap()
        .split(',')
        .map(|p| p.split_whitespace().last().unwrap().to_string())
        .collect();
    let body = body.split("\n}").next().expect("the body ends");
    let mut code = Vec::new();
    let mut labels = HashMap::new();
    for raw in body.lines() {
        let line = raw.split("//").next().unwrap().trim();
        if line.is_empty() || line.starts_with('.') {
            continue;
        }
        if let Some(l) = line.strip_suffix(':') {
            labels.insert(l.to_string(), code.len());
            continue;
        }
        let line = line.strip_suffix(';').expect("an instruction ends with ;");
        let (pred, line) = match line.strip_prefix('@') {
            Some(r) => {
                let (p, rest) = r.split_once(char::is_whitespace).unwrap();
                let (neg, reg) = p.strip_prefix('!').map_or((false, p), |q| (true, q));
                (Some((neg, reg.to_string())), rest.trim())
            }
            None => (None, line),
        };
        let (op, args) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let args = if args.trim().is_empty() {
            Vec::new()
        } else {
            args.split(',').map(operand).collect()
        };
        code.push(Ins {
            pred,
            op: op.to_string(),
            args,
        });
    }
    Kernel {
        params,
        code,
        labels,
    }
}

/// Global memory: the launch's two buffers, nothing else.
struct Mem {
    src: Vec<u8>,
    dst: Vec<u8>,
}

impl Mem {
    fn at(&mut self, addr: u64) -> &mut [u8] {
        self.span(addr, 4)
    }

    fn span(&mut self, addr: u64, n: usize) -> &mut [u8] {
        let (base, buf, what) = if addr >= DST_BASE {
            (DST_BASE, &mut self.dst, "frame")
        } else {
            (SRC_BASE, &mut self.src, "source extent")
        };
        let off = addr
            .checked_sub(base)
            .unwrap_or_else(|| panic!("the kernel formed {addr:#x}, below every buffer"));
        let off = usize::try_from(off).unwrap();
        let len = buf.len();
        buf.get_mut(off..off + n).unwrap_or_else(|| {
            panic!("the kernel touched {what} offset {off:#x} (+{n}), outside its {len:#x} bytes")
        })
    }
}

struct Thread<'a> {
    k: &'a Kernel,
    args: &'a HashMap<&'a str, u64>,
    regs: HashMap<String, u64>,
    ctaid: u32,
    tid: u32,
}

impl Thread<'_> {
    fn get(&self, o: &Op) -> u64 {
        match o {
            Op::Reg(r) => *self
                .regs
                .get(r)
                .unwrap_or_else(|| panic!("{r} read before it was written")),
            Op::Imm(v) => *v as u64,
            Op::Special("ctaid") => u64::from(self.ctaid),
            Op::Special("tid") => u64::from(self.tid),
            Op::Special(_) => u64::from(NTID),
            Op::Mem(m) => panic!("a memory operand [{m}] where a value is wanted"),
        }
    }

    fn u32(&self, o: &Op) -> u32 {
        self.get(o) as u32
    }

    fn set(&mut self, o: &Op, v: u64) {
        let Op::Reg(r) = o else {
            panic!("a destination that is not a register: {o:?}")
        };
        self.regs.insert(r.clone(), v);
    }

    fn addr(&self, o: &Op) -> u64 {
        match o {
            Op::Mem(m) => self.get(&Op::Reg(m.clone())),
            _ => panic!("not an address: {o:?}"),
        }
    }

    /// Run to `ret`; panics on any instruction it does not model.
    fn run(&mut self, mem: &mut Mem) {
        let mut pc = 0;
        let mut steps = 0u64;
        loop {
            steps += 1;
            assert!(steps < 10_000_000, "a runaway thread");
            let ins = &self.k.code[pc];
            pc += 1;
            if let Some((neg, p)) = &ins.pred
                && (self.get(&Op::Reg(p.clone())) != 0) == *neg
            {
                continue;
            }
            let a = &ins.args;
            let b32 = |t: &Self, i: usize| t.u32(&a[i]);
            let v: Option<u64> = match ins.op.as_str() {
                "ld.param.u64" | "ld.param.u32" | "ld.param.s32" => {
                    let Op::Mem(name) = &a[1] else { panic!() };
                    let v = *self
                        .args
                        .get(name.rsplit('_').next().unwrap())
                        .unwrap_or_else(|| panic!("no argument for {name}"));
                    Some(if ins.op.ends_with("u64") {
                        v
                    } else {
                        u64::from(v as u32)
                    })
                }
                "mov.u32" => Some(u64::from(b32(self, 1))),
                "mov.u64" => Some(self.get(&a[1])),
                "add.u32" | "add.s32" => Some(u64::from(b32(self, 1).wrapping_add(b32(self, 2)))),
                "add.u64" => Some(self.get(&a[1]).wrapping_add(self.get(&a[2]))),
                "sub.u32" => Some(u64::from(b32(self, 1).wrapping_sub(b32(self, 2)))),
                "shr.u32" => Some(u64::from(
                    b32(self, 1).checked_shr(b32(self, 2)).unwrap_or(0),
                )),
                "shl.b32" => Some(u64::from(
                    b32(self, 1).checked_shl(b32(self, 2)).unwrap_or(0),
                )),
                "shl.b64" => Some(self.get(&a[1]).checked_shl(b32(self, 2)).unwrap_or(0)),
                "and.b32" => Some(u64::from(b32(self, 1) & b32(self, 2))),
                "or.b32" => Some(u64::from(b32(self, 1) | b32(self, 2))),
                "xor.b32" => Some(u64::from(b32(self, 1) ^ b32(self, 2))),
                "mul.wide.u32" => Some(u64::from(b32(self, 1)) * u64::from(b32(self, 2))),
                "mul.lo.s32" | "mul.lo.u32" => {
                    Some(u64::from(b32(self, 1).wrapping_mul(b32(self, 2))))
                }
                "mad.lo.u32" => Some(u64::from(
                    b32(self, 1)
                        .wrapping_mul(b32(self, 2))
                        .wrapping_add(b32(self, 3)),
                )),
                "div.s32" => {
                    let (x, y) = (b32(self, 1) as i32, b32(self, 2) as i32);
                    assert_ne!(y, 0, "a division by zero");
                    Some(u64::from(x.wrapping_div(y) as u32))
                }
                "div.u32" => {
                    assert_ne!(b32(self, 2), 0, "a division by zero");
                    Some(u64::from(b32(self, 1) / b32(self, 2)))
                }
                "max.s32" => Some(u64::from(
                    (b32(self, 1) as i32).max(b32(self, 2) as i32) as u32
                )),
                "min.s32" => Some(u64::from(
                    (b32(self, 1) as i32).min(b32(self, 2) as i32) as u32
                )),
                "min.u32" => Some(u64::from(b32(self, 1).min(b32(self, 2)))),
                "cvt.u64.u32" => Some(u64::from(b32(self, 1))),
                "setp.ge.u32" => Some(u64::from(b32(self, 1) >= b32(self, 2))),
                "setp.ne.u32" => Some(u64::from(b32(self, 1) != b32(self, 2))),
                "setp.eq.u32" => Some(u64::from(b32(self, 1) == b32(self, 2))),
                "selp.b32" => Some(u64::from(if self.get(&a[3]) != 0 {
                    b32(self, 1)
                } else {
                    b32(self, 2)
                })),
                "ld.global.nc.u32" | "ld.global.u32" => {
                    let w = mem.at(self.addr(&a[1]));
                    Some(u64::from(u32::from_le_bytes([w[0], w[1], w[2], w[3]])))
                }
                // the threads run one after another here, so the atomic is a plain add
                "atom.global.add.u64" => {
                    let v = self.get(&a[2]);
                    let w = mem.span(self.addr(&a[1]), 8);
                    let old = u64::from_le_bytes(w[..8].try_into().unwrap());
                    w.copy_from_slice(&old.wrapping_add(v).to_le_bytes());
                    Some(old)
                }
                "st.global.u32" => {
                    let v = b32(self, 1);
                    mem.at(self.addr(&a[0])).copy_from_slice(&v.to_le_bytes());
                    None
                }
                "bra" | "bra.uni" => {
                    let Op::Reg(l) = &a[0] else { panic!() };
                    pc = self.k.labels[l];
                    None
                }
                "ret" => return,
                other => panic!("an instruction the interpreter does not model: {other}"),
            };
            if let Some(v) = v {
                let dst = a[0].clone();
                self.set(&dst, v);
            }
        }
    }
}

/// Launch `kf_compose` for `l` over `src` (the store from `l.src`) into `frame` (`fw` x `fh`),
/// the way `DisplayGpu::launch_compose` does: `rows` CTAs of [`NTID`] threads.
fn launch(l: &LayerPlan, src: &[u8], frame: &mut Vec<u8>, fw: u32, fh: u32) {
    let k = kernel("kf_compose");
    let v = |x: u32| u64::from(x);
    let s = |x: i32| u64::from(x as u32);
    let args: HashMap<&str, u64> = [
        ("src", SRC_BASE),
        ("dst", DST_BASE),
        ("layout", v(u32::from(l.block_linear))),
        ("pitch", v(l.pitch)),
        ("bh", v(l.block_height_log2)),
        ("x0b", v(l.x0_bytes)),
        ("y0", v(l.y0)),
        ("width", v(l.width)),
        ("ox", v(l.ox)),
        ("oy", v(l.oy)),
        ("dpitch", v(fw * 4)),
        ("fw", v(fw)),
        ("fh", v(fh)),
        ("flags", v(l.flags)),
        ("as", s(l.a_s)),
        ("bs", s(l.b_s)),
        ("ad", s(l.a_d)),
        ("bd", s(l.b_d)),
    ]
    .into_iter()
    .collect();
    for p in &k.params {
        assert!(
            args.contains_key(p.rsplit('_').next().unwrap()),
            "the kernel takes {p}, which the launch does not pass"
        );
    }
    let mut mem = Mem {
        src: src[..usize::try_from(l.extent).unwrap()].to_vec(),
        dst: std::mem::take(frame),
    };
    for ctaid in 0..l.rows {
        for tid in 0..NTID {
            Thread {
                k: &k,
                args: &args,
                regs: HashMap::new(),
                ctaid,
                tid,
            }
            .run(&mut mem);
        }
    }
    *frame = mem.dst;
}

/// A byte pattern that never repeats inside a row or between rows.
fn pattern(n: usize, seed: u64) -> Vec<u8> {
    (0..n as u64)
        .map(|i| ((i.wrapping_add(seed).wrapping_mul(2_654_435_761)) >> 9) as u8)
        .collect()
}

fn layer(flags: u32, f: (i32, i32, i32, i32)) -> LayerPlan {
    LayerPlan {
        window: 0,
        src: 0,
        extent: 0,
        block_linear: false,
        pitch: 0,
        block_height_log2: 0,
        x0_bytes: 0,
        y0: 0,
        width: 0,
        rows: 0,
        ox: 0,
        oy: 0,
        flags,
        a_s: f.0,
        b_s: f.1,
        a_d: f.2,
        b_d: f.3,
        ilut: None,
    }
}

/// A pitch rectangle `w` x `rows` at `(ox, oy)`, source rows `pitch` bytes apart.
fn pitch(mut l: LayerPlan, w: u32, rows: u32, pitch: u32, at: (u32, u32)) -> LayerPlan {
    l.pitch = pitch;
    l.width = w;
    l.rows = rows;
    (l.ox, l.oy) = at;
    l.extent = u64::from(rows - 1) * u64::from(pitch) + u64::from(w) * 4;
    l
}

/// Run kernel and reference on the same inputs (the frame pre-filled with `seed`'s pattern, so
/// blends and XOR see something below) and compare every byte.
fn same(case: &str, l: &LayerPlan, fw: u32, fh: u32, seed: u64) {
    let src = pattern(usize::try_from(l.extent).unwrap() + 64, seed);
    let base = pattern((fw * fh * 4) as usize, seed ^ 0x5a5a);
    let mut want = base.clone();
    compose_reference(l, &src, None, &mut want, fw, fh).unwrap_or_else(|e| panic!("{case}: {}", e.0));
    let mut got = base.clone();
    launch(l, &src, &mut got, fw, fh);
    if let Some(i) = (0..got.len()).find(|&i| got[i] != want[i]) {
        panic!(
            "{case}: frame ({}, {}) byte {}: the kernel wrote {:#04x}, the reference {:#04x}",
            (i / 4) % fw as usize,
            i / 4 / fw as usize,
            i % 4,
            got[i],
            want[i]
        );
    }
    assert_ne!(got, base, "{case}: the launch changed nothing");
}

/// The factor pairs `kf_disp::scanout` maps NVKMS's cursor and window blends onto (opaque,
/// premultiplied, straight, and both with a surface alpha of 128).
const BLENDS: [(i32, i32, i32, i32); 5] = [
    (255, 0, 0, 0),
    (255, 0, 255, -255),
    (0, 255, 255, -255),
    (128, 0, 255, -128),
    (0, 128, 255, -128),
];

/// ★ Pitch layers: opaque, every blend with and without source alpha, red/blue swapped — a
/// rectangle offset in the frame, rows narrower than their pitch.
#[test]
fn the_kernel_composes_pitch_layers_as_the_reference_does() {
    let (fw, fh) = (64, 12);
    same(
        "opaque",
        &pitch(layer(COMPOSE_OPAQUE, BLENDS[0]), 50, 7, 256, (3, 2)),
        fw,
        fh,
        1,
    );
    for (i, f) in BLENDS.iter().enumerate() {
        for alpha in [0, COMPOSE_ALPHA] {
            let l = pitch(layer(alpha, *f), 37, 5, 160, (9, 4));
            same(
                &format!("blend {f:?} alpha {alpha}"),
                &l,
                fw,
                fh,
                10 + i as u64,
            );
        }
    }
    same(
        "swapped, blended",
        &pitch(
            layer(COMPOSE_ALPHA | COMPOSE_SWAP_RB, BLENDS[1]),
            20,
            3,
            80,
            (0, 0),
        ),
        fw,
        fh,
        3,
    );
    same(
        "swapped, opaque",
        &pitch(
            layer(COMPOSE_OPAQUE | COMPOSE_SWAP_RB, BLENDS[0]),
            20,
            3,
            80,
            (40, 9),
        ),
        fw,
        fh,
        4,
    );
}

/// ★ §O (2026-10-04): the XOR blend — the colour XORed into a NON-black frame, alpha ignored (the
/// pattern's alpha bytes vary), factors ignored (garbage ones given); swapped too, and a 300-pixel
/// row so the threads' stride loop runs past 256.
#[test]
fn the_kernel_xors_the_colour_into_what_lies_below() {
    let (fw, fh) = (320, 6);
    same(
        "xor",
        &pitch(layer(COMPOSE_XOR, (7, -3, 99, 1)), 64, 6, 256, (11, 0)),
        fw,
        fh,
        21,
    );
    same(
        "xor, alpha flag set too",
        &pitch(
            layer(COMPOSE_XOR | COMPOSE_ALPHA, (0, 0, 0, 0)),
            16,
            2,
            64,
            (0, 1),
        ),
        fw,
        fh,
        22,
    );
    same(
        "xor, swapped",
        &pitch(
            layer(COMPOSE_XOR | COMPOSE_SWAP_RB, (0, 0, 0, 0)),
            16,
            2,
            64,
            (5, 3),
        ),
        fw,
        fh,
        23,
    );
    same(
        "xor, a 300-pixel row",
        &pitch(layer(COMPOSE_XOR, (0, 0, 0, 0)), 300, 2, 1200, (10, 2)),
        fw,
        fh,
        24,
    );
    // and it is not the blend: a white, alpha-0 pixel inverts what lies below
    let l = pitch(layer(COMPOSE_XOR, (0, 0, 255, 0)), 1, 1, 4, (0, 0));
    let mut f = vec![0x12, 0x34, 0x56, 0x00];
    launch(&l, &[0xff, 0xff, 0xff, 0x00], &mut f, 1, 1);
    assert_eq!(f, [0xed, 0xcb, 0xa9, 0x00]);
}

/// ★ Block-linear windows (the GOB swizzle of `kf_disp::scanout::bl_offset`): the rectangle's
/// origin inside a block in both axes, block heights 1 and 16 GOBs, opaque and blended.
#[test]
fn the_kernel_reads_block_linear_surfaces_as_the_reference_does() {
    for (bh, gpr, block_rows, x0b, y0, w, rows) in [
        (1u32, 4u32, 3u64, 4u32, 3u32, 62u32, 37u32),
        (4, 4, 1, 68, 9, 40, 100),
    ] {
        let mut l = layer(COMPOSE_OPAQUE, BLENDS[0]);
        l.block_linear = true;
        l.pitch = gpr;
        l.block_height_log2 = bh;
        (l.x0_bytes, l.y0, l.width, l.rows) = (x0b, y0, w, rows);
        (l.ox, l.oy) = (5, 2);
        l.extent = block_rows * u64::from(gpr) * (512 << bh);
        same(&format!("bl bh={bh} opaque"), &l, 70, 110, u64::from(bh));
        l.flags = COMPOSE_ALPHA;
        (l.a_s, l.b_s, l.a_d, l.b_d) = BLENDS[1];
        same(
            &format!("bl bh={bh} blended"),
            &l,
            70,
            110,
            7 + u64::from(bh),
        );
    }
}

/// The kernel's own clipping: a rectangle past the frame's right and bottom edges writes nothing
/// outside it (the host clips first; the kernel does not rely on it), and reads stay in the extent.
#[test]
fn the_kernel_clips_at_the_frame_and_reads_only_its_extent() {
    same(
        "past the right and bottom",
        &pitch(layer(COMPOSE_XOR, (0, 0, 0, 0)), 40, 9, 160, (30, 4)),
        48,
        8,
        31,
    );
}

/// ⊘ The interpreter is not vacuous: it refuses an instruction it does not model, and a launch
/// whose extent is one byte short of what the kernel reads is caught as an out-of-bounds read.
#[test]
fn the_interpreter_refuses_what_it_does_not_model() {
    let k = kernel("kf_compose");
    assert!(k.code.len() > 100, "{} instructions", k.code.len());
    assert!(
        k.labels.contains_key("$C_xor"),
        "the XOR path is in the PTX"
    );
    let short = std::panic::catch_unwind(|| {
        let mut l = pitch(layer(COMPOSE_OPAQUE, BLENDS[0]), 8, 2, 32, (0, 0));
        let src = pattern(64, 1);
        l.extent -= 1;
        let mut f = vec![0; 8 * 2 * 4];
        launch(&l, &src, &mut f, 8, 2);
    });
    assert!(short.is_err(), "a read past the extent must be caught");
}

/// Launch `kf_sum` over a tight `w` x `h` frame the way `DisplayGpu::compose_checksum` does: `h`
/// CTAs of [`NTID`] threads, the row slots zeroed first; returns the row sums.
fn launch_sum(frame: &[u8], w: u32, h: u32) -> Vec<u64> {
    let k = kernel("kf_sum");
    let args: HashMap<&str, u64> = [
        ("src", SRC_BASE),
        ("out", DST_BASE),
        ("width", u64::from(w)),
        ("pitch", u64::from(w * 4)),
    ]
    .into_iter()
    .collect();
    for p in &k.params {
        assert!(
            args.contains_key(p.rsplit('_').next().unwrap()),
            "the kernel takes {p}, which the launch does not pass"
        );
    }
    let mut mem = Mem {
        src: frame[..(w * h * 4) as usize].to_vec(),
        dst: vec![0; h as usize * 8],
    };
    for ctaid in 0..h {
        for tid in 0..NTID {
            Thread {
                k: &k,
                args: &args,
                regs: HashMap::new(),
                ctaid,
                tid,
            }
            .run(&mut mem);
        }
    }
    mem.dst
        .as_chunks::<8>()
        .0
        .iter()
        .map(|c| u64::from_le_bytes(*c))
        .collect()
}

/// ★ `display-max-fps` (D2): the checksum kernel, run from its PTX text, gives EXACTLY the row sums
/// of `kf_disp::pace::row_sums` — rows narrower than a CTA, one pixel, a row past 256 pixels (the
/// stride loop), and a frame that differs in one pixel gives a different digest. Every read stays in
/// the frame and every write in the row slots (the interpreter's bounds).
#[test]
fn the_sum_kernel_is_the_reference_checksum() {
    for (w, h) in [(1u32, 1u32), (37, 3), (256, 2), (300, 4), (1000, 2)] {
        let f = pattern((w * h * 4) as usize, u64::from(w));
        let got = launch_sum(&f, w, h);
        let want = row_sums(&f, w, h).unwrap();
        assert_eq!(got, want, "{w}x{h}");
        assert_eq!(digest(&got, w, h), digest(&want, w, h));
    }
    let (w, h) = (300, 4);
    let mut f = pattern((w * h * 4) as usize, 9);
    let before = digest(&launch_sum(&f, w, h), w, h);
    f[(2 * w as usize + 299) * 4 + 2] ^= 0x10;
    assert_ne!(
        digest(&launch_sum(&f, w, h), w, h),
        before,
        "one pixel changed"
    );
}

/// ⊘ Not vacuous: a launch given one byte less frame than its rows cover is caught as an
/// out-of-bounds read (the kernel reads exactly `rows x pitch`).
#[test]
fn the_sum_kernel_reads_only_its_frame() {
    let k = kernel("kf_sum");
    assert!(k.labels.contains_key("$S_px") && k.labels.contains_key("$S_add"));
    let short = std::panic::catch_unwind(|| {
        let f = pattern(8 * 2 * 4, 1);
        let k = kernel("kf_sum");
        let args: HashMap<&str, u64> = [
            ("src", SRC_BASE),
            ("out", DST_BASE),
            ("width", 8),
            ("pitch", 32),
        ]
        .into_iter()
        .collect();
        let mut mem = Mem {
            src: f[..f.len() - 1].to_vec(),
            dst: vec![0; 16],
        };
        for ctaid in 0..2 {
            Thread {
                k: &k,
                args: &args,
                regs: HashMap::new(),
                ctaid,
                tid: 7,
            }
            .run(&mut mem);
        }
    });
    assert!(short.is_err(), "a read past the frame must be caught");
}
