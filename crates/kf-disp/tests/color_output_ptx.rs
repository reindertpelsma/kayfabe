// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! The COMMITTED PTX of `kf_color_output` (`cuda/display/kf_color.ptx`, generated from
//! `kf_color.cu` by `make_color_ptx.sh`) RUN ON THE HOST: a small interpreter executes the entry's
//! text, instruction by instruction, for every (CTA, thread) of the launch kf-cuda makes (one CTA
//! per row, 256 threads, `kf_cuda::display::DisplayGpu::color_output`), on synthetic FP32 frames.
//! The frame it leaves is compared byte for byte with the CPU oracle
//! (`kf_disp::color::model::output_rgb`, packed B,G,R,A as the kernel packs it) and with the
//! independent `f64` reference (`color_support/reference.rs`).
//!
//! Why: the PR #92 review (L6) found that the only GPU-free guards of the kernel were source-text
//! pins. This runs the kernel's ARITHMETIC as committed, so a regenerated PTX that changed the
//! mirror branch, the clamp, the index, the interpolation, the packing or the parameter order
//! fails here without a GPU. It is the kernel's logic, not the GPU: the PTX JIT and the hardware
//! are graded by `kf-cuda/examples/color_hue_vibrance.rs` on a GPU.
//!
//! The interpreter implements exactly the instructions the entry uses and PANICS on any other,
//! and every load and store must fall inside the buffer the launch passed (the source frame, the
//! LUT snapshot, the destination frame, the parameter block).

#[path = "color_support/reference.rs"]
mod reference;

use kf_disp::color::model;
use reference::{RefTable, Rng};
use std::collections::HashMap;

const PTX: &str = include_str!("../../../cuda/display/kf_color.ptx");
const NTID: u32 = 256;
const SRC: u64 = 0x1_0000_0000;
const DST: u64 = 0x2_0000_0000;
const LUT: u64 = 0x3_0000_0000;
const PARAM: u64 = 0x4_0000_0000;

#[derive(Debug, Clone)]
enum Op {
    /// a register, by its index in the thread's file (names resolved at parse time)
    Reg(usize),
    Imm(u64),
    Special(&'static str),
    /// `[reg+off]`
    Mem(usize, i64),
    /// `[param+off]`, a parameter by name
    PMem(String, i64),
    /// a parameter's address (`mov.b64 %rd, name`)
    Sym(String),
    /// a branch target, resolved to an instruction index
    Label(usize),
}

#[derive(Debug, Clone)]
struct Ins {
    pred: Option<(bool, usize)>,
    op: String,
    args: Vec<Op>,
}

struct Kernel {
    code: Vec<Ins>,
    /// registers in the file
    nregs: usize,
    /// parameter name -> byte offset in the parameter block
    params: HashMap<String, u64>,
}

fn operand(t: &str, regs: &mut HashMap<String, usize>, labels: &mut Vec<String>) -> Op {
    let mut reg = |r: &str| {
        let n = regs.len();
        *regs.entry(r.to_string()).or_insert(n)
    };
    let t = t.trim();
    if let Some(inner) = t.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
        let (base, off) = match inner.find('+') {
            Some(i) => (&inner[..i], inner[i + 1..].parse::<i64>().unwrap()),
            None => (inner, 0),
        };
        return if base.starts_with('%') {
            Op::Mem(reg(base), off)
        } else {
            Op::PMem(base.to_string(), off)
        };
    }
    if t == "%tid.x" {
        return Op::Special("tid");
    }
    if t == "%ctaid.x" {
        return Op::Special("ctaid");
    }
    if t.starts_with('%') {
        return Op::Reg(reg(t));
    }
    if let Some(h) = t.strip_prefix("0f") {
        return Op::Imm(u64::from(u32::from_str_radix(h, 16).unwrap()));
    }
    if t.starts_with("$L") {
        // resolved after the whole body is read
        labels.push(t.to_string());
        return Op::Label(labels.len() - 1);
    }
    if let Ok(v) = t.parse::<i64>() {
        return Op::Imm(v as u64);
    }
    Op::Sym(t.to_string())
}

fn kernel(name: &str) -> Kernel {
    let start = PTX
        .find(&format!(".entry {name}("))
        .unwrap_or_else(|| panic!("no entry {name}"));
    let text = &PTX[start..];
    let body_at = text.find('{').unwrap();
    let header = &text[..body_at];
    let end = text[body_at..].find("\n}").unwrap() + body_at;
    // parameter layout by the PTX ABI: each at its alignment, in order
    let mut params = HashMap::new();
    let mut off = 0_u64;
    for line in header.lines().filter(|l| l.contains(".param")) {
        let l = line.trim().trim_end_matches(',');
        let pname = l.split_whitespace().last().unwrap();
        let (pname, size, align) = if let Some(i) = pname.find('[') {
            let n: u64 = pname[i + 1..pname.len() - 1].parse().unwrap();
            let align = if l.contains(".align 4") { 4 } else { 1 };
            (&pname[..i], n, align)
        } else if l.contains(".u64") {
            (pname, 8, 8)
        } else {
            assert!(l.contains(".u32"), "parameter type: {l}");
            (pname, 4, 4)
        };
        off = off.next_multiple_of(align);
        params.insert(pname.to_string(), off);
        off += size;
    }
    let mut code = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    let mut regs: HashMap<String, usize> = HashMap::new();
    let mut targets: Vec<String> = Vec::new();
    for raw in text[body_at + 1..end].lines() {
        let line = raw.split("//").next().unwrap().trim();
        if line.is_empty() || line.starts_with(".reg") || line == "{" || line == "}" {
            continue;
        }
        if let Some(l) = line.strip_suffix(':') {
            at.insert(l.to_string(), code.len());
            continue;
        }
        let line = line
            .strip_suffix(';')
            .unwrap_or_else(|| panic!("no ';': {line}"));
        let (pred, rest) = if let Some(r) = line.strip_prefix('@') {
            let (p, rest) = r.split_once(char::is_whitespace).unwrap();
            let (want, p) = match p.strip_prefix('!') {
                Some(p) => (false, p),
                None => (true, p),
            };
            let n = regs.len();
            (Some((want, *regs.entry(p.to_string()).or_insert(n))), rest)
        } else {
            (None, line)
        };
        let rest = rest.trim();
        let (op, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let args = if args.trim().is_empty() {
            vec![]
        } else {
            args.split(',')
                .map(|a| operand(a, &mut regs, &mut targets))
                .collect()
        };
        code.push(Ins {
            pred,
            op: op.to_string(),
            args,
        });
    }
    for ins in &mut code {
        for a in &mut ins.args {
            if let Op::Label(i) = a {
                *i = at[&targets[*i]];
            }
        }
    }
    Kernel {
        code,
        nregs: regs.len(),
        params,
    }
}

struct Region {
    base: u64,
    bytes: Vec<u8>,
    writable: bool,
    name: &'static str,
}

struct Mem(Vec<Region>);

impl Mem {
    fn span(&mut self, addr: u64, n: usize, write: bool) -> &mut [u8] {
        for r in &mut self.0 {
            if addr >= r.base && addr + n as u64 <= r.base + r.bytes.len() as u64 {
                assert!(
                    !write || r.writable,
                    "store into read-only {} at {addr:#x}",
                    r.name
                );
                let o = (addr - r.base) as usize;
                return &mut r.bytes[o..o + n];
            }
        }
        panic!("access of {n} bytes at {addr:#x} is outside every buffer the launch passed");
    }
    fn load(&mut self, addr: u64, n: usize) -> u64 {
        let s = self.span(addr, n, false);
        let mut b = [0u8; 8];
        b[..n].copy_from_slice(s);
        u64::from_le_bytes(b)
    }
    fn store(&mut self, addr: u64, n: usize, v: u64) {
        self.span(addr, n, true)
            .copy_from_slice(&v.to_le_bytes()[..n]);
    }
}

struct Thread<'k> {
    k: &'k Kernel,
    /// value and written-yet per register (predicates share the file)
    regs: Vec<Option<u64>>,
    tid: u32,
    ctaid: u32,
}

fn f(x: u64) -> f32 {
    f32::from_bits(x as u32)
}
fn b(x: f32) -> u64 {
    u64::from(x.to_bits())
}
/// PTX `min.f32`: if one operand is NaN, the other.
fn fmin(a: f32, c: f32) -> f32 {
    if a.is_nan() {
        c
    } else if c.is_nan() {
        a
    } else {
        a.min(c)
    }
}

impl Thread<'_> {
    fn get(&self, o: &Op) -> u64 {
        match o {
            Op::Reg(r) => {
                self.regs[*r].unwrap_or_else(|| panic!("register {r} read before written"))
            }
            Op::Imm(v) => *v,
            Op::Special("tid") => u64::from(self.tid),
            Op::Special("ctaid") => u64::from(self.ctaid),
            Op::Sym(s) => PARAM + self.k.params[s],
            other => panic!("not a value: {other:?}"),
        }
    }
    fn u32(&self, o: &Op) -> u32 {
        self.get(o) as u32
    }
    fn p(&self, o: &Op) -> bool {
        match o {
            Op::Reg(r) => {
                self.regs[*r].unwrap_or_else(|| panic!("predicate {r} read before written")) != 0
            }
            other => panic!("not a predicate: {other:?}"),
        }
    }
    fn set(&mut self, o: &Op, v: u64) {
        let Op::Reg(r) = o else {
            panic!("not a register: {o:?}")
        };
        self.regs[*r] = Some(v);
    }
    fn setp(&mut self, o: &Op, v: bool) {
        let Op::Reg(r) = o else {
            panic!("not a predicate: {o:?}")
        };
        self.regs[*r] = Some(u64::from(v));
    }
    fn addr(&self, o: &Op) -> u64 {
        match o {
            Op::Mem(r, off) => self.regs[*r]
                .expect("address register")
                .wrapping_add(*off as u64),
            Op::PMem(name, off) => (PARAM + self.k.params[name]).wrapping_add(*off as u64),
            other => panic!("not an address: {other:?}"),
        }
    }

    fn run(&mut self, mem: &mut Mem) {
        let mut pc = 0;
        let mut steps = 0_u64;
        loop {
            steps += 1;
            assert!(steps < 10_000_000, "runaway thread");
            let ins = &self.k.code[pc];
            pc += 1;
            if let Some((want, p)) = ins.pred
                && (self.regs[p].expect("predicate") != 0) != want
            {
                continue;
            }
            let a = &ins.args;
            match ins.op.as_str() {
                "ret" => return,
                "bra" | "bra.uni" => {
                    let Op::Label(l) = a[0] else { panic!("{a:?}") };
                    pc = l;
                }
                "ld.param.b32" => {
                    let v = mem.load(self.addr(&a[1]), 4);
                    self.set(&a[0], v);
                }
                "ld.param.b64" => {
                    let v = mem.load(self.addr(&a[1]), 8);
                    self.set(&a[0], v);
                }
                "ld.global.b32" => {
                    let v = mem.load(self.addr(&a[1]), 4);
                    self.set(&a[0], v);
                }
                "ld.global.b16" => {
                    let v = mem.load(self.addr(&a[1]), 2);
                    self.set(&a[0], v);
                }
                "st.global.b32" => {
                    let v = self.get(&a[1]) & 0xffff_ffff;
                    mem.store(self.addr(&a[0]), 4, v);
                }
                "cvta.to.global.u64" | "mov.b64" | "mov.u32" | "mov.b32" => {
                    let v = self.get(&a[1]);
                    let v = if ins.op.ends_with("32") {
                        v & 0xffff_ffff
                    } else {
                        v
                    };
                    self.set(&a[0], v);
                }
                "cvt.u64.u32" => {
                    let v = u64::from(self.u32(&a[1]));
                    self.set(&a[0], v);
                }
                "add.s32" => {
                    let v = self.u32(&a[1]).wrapping_add(self.u32(&a[2]));
                    self.set(&a[0], u64::from(v));
                }
                "add.s64" => {
                    let v = self.get(&a[1]).wrapping_add(self.get(&a[2]));
                    self.set(&a[0], v);
                }
                "shl.b32" => {
                    let v = self.u32(&a[1]).wrapping_shl(self.u32(&a[2]));
                    self.set(&a[0], u64::from(v));
                }
                "shl.b64" => {
                    let v = self.get(&a[1]).wrapping_shl(self.u32(&a[2]));
                    self.set(&a[0], v);
                }
                "or.b32" => {
                    let v = self.u32(&a[1]) | self.u32(&a[2]);
                    self.set(&a[0], u64::from(v));
                }
                "min.u32" => {
                    let v = self.u32(&a[1]).min(self.u32(&a[2]));
                    self.set(&a[0], u64::from(v));
                }
                "mul.wide.u32" => {
                    let v = u64::from(self.u32(&a[1])) * u64::from(self.u32(&a[2]));
                    self.set(&a[0], v);
                }
                "cvt.rn.f32.u32" => {
                    let v = self.u32(&a[1]) as f32;
                    self.set(&a[0], b(v));
                }
                "cvt.rn.f32.s32" => {
                    let v = self.u32(&a[1]) as i32 as f32;
                    self.set(&a[0], b(v));
                }
                "cvt.rn.f32.u16" => {
                    let v = f32::from(self.get(&a[1]) as u16);
                    self.set(&a[0], b(v));
                }
                "cvt.rzi.u32.f32" => {
                    // round toward zero, saturating; NaN -> 0 (PTX), as Rust's `as`
                    let v = f(self.get(&a[1])) as u32;
                    self.set(&a[0], u64::from(v));
                }
                "mul.rn.f32" => {
                    let v = f(self.get(&a[1])) * f(self.get(&a[2]));
                    self.set(&a[0], b(v));
                }
                "add.rn.f32" => {
                    let v = f(self.get(&a[1])) + f(self.get(&a[2]));
                    self.set(&a[0], b(v));
                }
                "sub.rn.f32" => {
                    let v = f(self.get(&a[1])) - f(self.get(&a[2]));
                    self.set(&a[0], b(v));
                }
                "neg.f32" => {
                    let v = -f(self.get(&a[1]));
                    self.set(&a[0], b(v));
                }
                "min.f32" => {
                    let v = fmin(f(self.get(&a[1])), f(self.get(&a[2])));
                    self.set(&a[0], b(v));
                }
                "selp.f32" => {
                    let v = if self.p(&a[3]) {
                        self.get(&a[1])
                    } else {
                        self.get(&a[2])
                    };
                    self.set(&a[0], v);
                }
                "setp.ge.u32" => {
                    let v = self.u32(&a[1]) >= self.u32(&a[2]);
                    self.setp(&a[0], v);
                }
                "setp.lt.u32" => {
                    let v = self.u32(&a[1]) < self.u32(&a[2]);
                    self.setp(&a[0], v);
                }
                "setp.eq.b32" => {
                    let v = self.u32(&a[1]) == self.u32(&a[2]);
                    self.setp(&a[0], v);
                }
                "setp.ne.b32" => {
                    let v = self.u32(&a[1]) != self.u32(&a[2]);
                    self.setp(&a[0], v);
                }
                "setp.eq.b64" => {
                    let v = self.get(&a[1]) == self.get(&a[2]);
                    self.setp(&a[0], v);
                }
                // ordered comparisons: false on NaN
                "setp.gt.f32" => {
                    let v = f(self.get(&a[1])) > f(self.get(&a[2]));
                    self.setp(&a[0], v);
                }
                "setp.lt.f32" => {
                    let v = f(self.get(&a[1])) < f(self.get(&a[2]));
                    self.setp(&a[0], v);
                }
                // unordered: true on NaN
                "setp.geu.f32" => {
                    let (x, y) = (f(self.get(&a[1])), f(self.get(&a[2])));
                    self.setp(&a[0], x.is_nan() || y.is_nan() || x >= y);
                }
                "and.pred" => {
                    let v = self.p(&a[1]) && self.p(&a[2]);
                    self.setp(&a[0], v);
                }
                "or.pred" => {
                    let v = self.p(&a[1]) || self.p(&a[2]);
                    self.setp(&a[0], v);
                }
                other => {
                    panic!("kf_color_output uses `{other}`, which this interpreter does not model")
                }
            }
        }
    }
}

/// One launch of `kf_color_output` as `DisplayGpu::color_output` makes it: grid = rows, 256
/// threads, parameters (src, dst, width, lut, interpolate, matrix, entries, mirror).
/// `lut` is the snapshot bytes (four header entries, then RGB+padding UNORM16 entries) or none.
#[allow(clippy::too_many_arguments)]
fn launch(
    src: &[[f32; 4]],
    width: u32,
    rows: u32,
    lut: Option<&[u8]>,
    interpolate: bool,
    matrix: &[i32; 12],
    entries: u32,
    mirror: bool,
) -> Vec<u8> {
    let k = kernel("kf_color_output");
    let n = (width * rows) as usize;
    assert_eq!(src.len(), n);
    let mut pb = vec![0u8; 92];
    let put = |pb: &mut Vec<u8>, name: &str, v: &[u8]| {
        let o = k.params[&format!("kf_color_output_param_{name}")] as usize;
        pb[o..o + v.len()].copy_from_slice(v);
    };
    put(&mut pb, "0", &SRC.to_le_bytes());
    put(&mut pb, "1", &DST.to_le_bytes());
    put(&mut pb, "2", &width.to_le_bytes());
    put(
        &mut pb,
        "3",
        &(if lut.is_some() { LUT } else { 0 }).to_le_bytes(),
    );
    put(&mut pb, "4", &u32::from(interpolate).to_le_bytes());
    let m: Vec<u8> = matrix.iter().flat_map(|v| v.to_le_bytes()).collect();
    put(&mut pb, "5", &m);
    put(&mut pb, "6", &entries.to_le_bytes());
    put(&mut pb, "7", &u32::from(mirror).to_le_bytes());
    let mut regions = vec![
        Region {
            base: SRC,
            bytes: src.iter().flatten().flat_map(|v| v.to_le_bytes()).collect(),
            writable: false,
            name: "source frame",
        },
        Region {
            base: DST,
            bytes: vec![0xcd; n * 4],
            writable: true,
            name: "destination frame",
        },
        Region {
            base: PARAM,
            bytes: pb,
            writable: false,
            name: "parameters",
        },
    ];
    if let Some(l) = lut {
        regions.push(Region {
            base: LUT,
            bytes: l.to_vec(),
            writable: false,
            name: "LUT snapshot",
        });
    }
    let mut mem = Mem(regions);
    for ctaid in 0..rows {
        for tid in 0..NTID {
            Thread {
                k: &k,
                regs: vec![None; k.nregs],
                tid,
                ctaid,
            }
            .run(&mut mem);
        }
    }
    mem.0.swap_remove(1).bytes
}

/// The snapshot layout the kernel reads: four 8-byte header entries, then RGB + padding.
fn snapshot(entries: &[[u16; 3]]) -> Vec<u8> {
    let mut out = vec![0xa5; 32];
    for e in entries {
        for v in e.iter().chain([&0x5a5a]) {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    // the product snapshot is always the full 1029-entry extent (`LUT_BYTES`)
    out.resize(kf_disp::color::LUT_BYTES as usize, 0xee);
    out
}

/// Compare one launch with the oracle (exact) and the reference (interval).
fn check(
    name: &str,
    src: &[[f32; 4]],
    width: u32,
    table: Option<&RefTable<'_>>,
    m: &[i32; 12],
) -> usize {
    let rows = (src.len() as u32).div_ceil(width);
    let snap = table.map(|t| snapshot(t.entries));
    let entries = table.map_or(1025, |t| t.entries.len() as u32);
    let frame = launch(
        src,
        width,
        rows,
        snap.as_deref(),
        table.is_some_and(|t| t.interpolate),
        m,
        entries,
        table.is_some_and(|t| t.mirror),
    );
    let mt = table.map(|t| model::Table {
        entries: t.entries,
        interpolate: t.interpolate,
        mirror: t.mirror,
    });
    let mut bad = Vec::new();
    for (i, px) in src.iter().enumerate() {
        let s = [px[0], px[1], px[2]];
        let got = &frame[i * 4..i * 4 + 4];
        let want = model::output_rgb(s, m, mt.as_ref());
        // the kernel packs q << (16 - 8c) into a little-endian word: B, G, R, A in memory
        let packed = [want[2], want[1], want[0], 255];
        if got != packed {
            bad.push(format!(
                "px {i} src {s:?}: kernel {got:?}, oracle BGRA {packed:?}"
            ));
        }
        let span = reference::output(s.map(f64::from), m, table);
        for c in 0..3 {
            if !span[c].admits(got[2 - c]) {
                bad.push(format!(
                    "px {i} src {s:?} c {c}: kernel {} reference {:?}",
                    got[2 - c],
                    span[c]
                ));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{name}: {} mismatches, first {:#?}",
        bad.len(),
        &bad[..bad.len().min(4)]
    );
    src.len()
}

fn frame_from(grid: &[[f32; 3]]) -> Vec<[f32; 4]> {
    grid.iter().map(|p| [p[0], p[1], p[2], 1.0]).collect()
}

/// The entry's parameter list is the one the launch passes (8 parameters, the matrix by value
/// at 4-byte alignment, `mirror` last), so the interpreter's layout is the GPU's.
#[test]
fn the_committed_entry_has_the_launch_abi() {
    let k = kernel("kf_color_output");
    let mut p: Vec<(u64, &String)> = k.params.iter().map(|(n, o)| (*o, n)).collect();
    p.sort();
    let names: Vec<String> = p.iter().map(|(o, n)| format!("{n}@{o}")).collect();
    assert_eq!(
        names,
        [0, 8, 16, 24, 32, 36, 84, 88]
            .iter()
            .enumerate()
            .map(|(i, o)| format!("kf_color_output_param_{i}@{o}"))
            .collect::<Vec<_>>()
    );
}

/// The hand-computed cases of the hardware example, through the PTX.
#[test]
fn the_ptx_renders_the_four_hand_computed_cases() {
    let lifted: Vec<[u16; 3]> = (0..1025_u32)
        .map(|i| [(16384 + i * 32) as u16; 3])
        .collect();
    let neg = [65536, -32768, 0, 0, 0, 65536, 0, 0, 0, 0, 65536, 0];
    let vib = reference::pack(reference::saturation(1.5), 0);
    for (src, m, interpolate, mirror, want) in [
        ([0.25, 0.75, 0.0], &neg, false, true, [0, 160, 64]),
        ([0.25, 0.75, 0.0], &neg, false, false, [64, 160, 64]),
        ([1.0, 0.0, 0.0], &vib, true, true, [192, 0, 0]),
        ([1.0, 0.0, 0.0], &vib, true, false, [192, 64, 64]),
    ] {
        let snap = snapshot(&lifted);
        let f = launch(
            &[[src[0], src[1], src[2], 1.0]],
            1,
            1,
            Some(&snap),
            interpolate,
            m,
            1025,
            mirror,
        );
        assert_eq!(
            &f[..4],
            &[want[2], want[1], want[0], 255],
            "{src:?} mirror={mirror}"
        );
    }
}

/// Hue and vibrance sweeps (both constructions) x table shapes x mirror x interpolation, on a
/// 300-pixel-wide frame (so the 256-thread stride loop runs) through the PTX.
#[test]
fn the_ptx_matches_oracle_and_reference_on_hue_and_vibrance_sweeps() {
    // 216 colours repeated to 2 rows of 300 (the second CTA, and threads 256..299 of each row)
    let grid: Vec<[f32; 4]> = frame_from(&reference::grid(&[0, 40, 96, 160, 224, 256]))
        .into_iter()
        .cycle()
        .take(600)
        .collect();
    let tables = reference::tables();
    let mut configs = reference::configurations();
    // a subset of the 300+ configurations keeps the interpreter run short; every kind is kept
    configs.retain(|c| {
        !c.name.starts_with("hue")
            || c.name.ends_with(" 0")
            || c.name.contains(" 45")
            || c.name.contains("135")
            || c.name.contains("270")
    });
    let mut px = 0;
    for cfg in &configs {
        px += check(
            &format!("{} bypass", cfg.name),
            &grid,
            300,
            None,
            &cfg.matrix,
        );
        for nt in tables.iter().filter(|t| {
            matches!(
                t.name,
                "lifted black 0.25+0.5x" | "staircase" | "per-channel"
            )
        }) {
            for (interpolate, mirror) in [(false, true), (true, true), (true, false)] {
                let t = RefTable {
                    entries: &nt.entries,
                    interpolate,
                    mirror,
                };
                px += check(
                    &format!(
                        "{} | {} x{} i={interpolate} m={mirror}",
                        cfg.name,
                        nt.name,
                        nt.entries.len()
                    ),
                    &grid,
                    300,
                    Some(&t),
                    &cfg.matrix,
                );
            }
        }
    }
    assert!(px > 400_000, "{px} pixels");
}

/// Bounded random programs through the PTX, including non-finite and out-of-range FP32 inputs
/// (a hostile value must not leave the buffers or crash: the interpreter checks every access).
#[test]
fn the_ptx_agrees_on_random_programs_and_hostile_values() {
    let mut rng = Rng(0x0c0_1052);
    for case in 0..60 {
        let n = if rng.coin() { 1025 } else { 257 };
        let entries: Vec<[u16; 3]> = (0..n)
            .map(|_| [rng.next() as u16, rng.next() as u16, rng.next() as u16])
            .collect();
        let t = RefTable {
            entries: &entries,
            interpolate: rng.coin(),
            mirror: rng.coin(),
        };
        let m: [i32; 12] = std::array::from_fn(|_| reference::sign_extend_21(rng.next() as u32));
        let mut src: Vec<[f32; 4]> = (0..64)
            .map(|_| std::array::from_fn(|_| rng.below(257) as f32 / 256.0))
            .collect();
        if case % 10 == 0 {
            // hostile values only for the bounds/crash check: compare the oracle bytes, which
            // define NaN/inf behaviour, not the reference
            src[0] = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.0];
            src[1] = [1e30, -1e30, 5.0, 1.0];
            let snap = snapshot(&entries);
            let f = launch(
                &src,
                64,
                1,
                Some(&snap),
                t.interpolate,
                &m,
                n as u32,
                t.mirror,
            );
            let mt = model::Table {
                entries: &entries,
                interpolate: t.interpolate,
                mirror: t.mirror,
            };
            for (i, p) in src.iter().enumerate() {
                let w = model::output_rgb([p[0], p[1], p[2]], &m, Some(&mt));
                assert_eq!(
                    &f[i * 4..i * 4 + 4],
                    &[w[2], w[1], w[0], 255],
                    "case {case} px {i}"
                );
            }
        } else {
            check(&format!("random case {case}"), &src, 64, Some(&t), &m);
        }
    }
}

/// A LUT pointer with an entry count other than 257/1025 samples nothing (the kernel returns 0
/// before any read), and the bypass (null LUT) never touches the snapshot region.
#[test]
fn bad_entry_counts_read_no_table_and_bypass_reads_none() {
    let m = reference::pack(reference::hue_w3c(90.0), 0x20);
    let src = frame_from(&reference::grid(&[0, 128, 256]));
    // a 1-byte "snapshot": any table read would leave it and panic
    for entries in [0, 1, 256, 1024, 1026, u32::MAX] {
        let f = launch(&src, 27, 1, Some(&[0u8]), true, &m, entries, true);
        assert!(
            f.chunks(4).all(|p| p == [0, 0, 0, 255]),
            "entries {entries}: {f:?}"
        );
    }
    let f = launch(&src, 27, 1, None, true, &m, 1025, true);
    for (i, p) in src.iter().enumerate() {
        let w = model::output_rgb([p[0], p[1], p[2]], &m, None);
        assert_eq!(&f[i * 4..i * 4 + 4], &[w[2], w[1], w[0], 255]);
    }
}
