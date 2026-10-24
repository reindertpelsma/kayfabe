// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! An INDEPENDENT reference of the head's output stage (OCSC0 -> OLUT -> clamp -> 8 bits), in
//! `f64`, written from the reading in `docs/design/V3_SDR_COLOR.md` and PR #92 — not from
//! `kf_disp::color::model` and not from `cuda/display/kf_color.cu`:
//!
//! * the pre-table value of channel `c` is `sum_i src[i] * m[c][i] + m[c][3]`, coefficients in
//!   the encoded unit (65536 = 1.0);
//! * the table spans `[0, 1]`: the input is clamped there, scaled by `entries - 1`, the lower
//!   sample is `floor`, at most `entries - 2`; interpolation is linear between it and the next;
//!   samples are UNORM16 read as `u / 65536`;
//! * `[inferred]` a MIRRORED table's negative side is the odd extension, `lut(-x) = -lut(x)`; an
//!   unmirrored table clamps a negative input to its first sample;
//! * the result is clamped to `[0, 1]` and quantised as `min(floor(y * 256), 255)`.
//!
//! The kernel and the CPU oracle compute in `f32` with a fixed operation order. A reference in
//! `f64` can disagree with them only where a rounding difference crosses a decision: the sign of
//! the pre-table value, an integer table position (no interpolation), or a quantisation step.
//! [`output`] therefore returns, per channel, the interval of results over a small perturbation of
//! the pre-table value (`EPS`, far above `f32` error for the coefficient range, far below any
//! colour step); a correct `f32` implementation lands inside it. Tests also check that almost all
//! intervals are a single value, so the comparison stays sharp.
#![allow(dead_code)]

/// Perturbation of the pre-table value that covers `f32` rounding (|v| < 50, so f32 error is
/// below 4e-6) and cannot hide a wrong branch or a wrong sample.
pub const EPS: f64 = 1e-5;

/// A table as the reference reads it.
#[derive(Clone, Copy)]
pub struct RefTable<'a> {
    /// RGB UNORM16 samples, 257 or 1025 of them.
    pub entries: &'a [[u16; 3]],
    /// Linear interpolation between adjacent samples.
    pub interpolate: bool,
    /// Odd extension on the negative side.
    pub mirror: bool,
}

/// The pre-table value of channel `c`.
#[must_use]
pub fn pre_table(src: [f64; 3], m: &[i32; 12], c: usize) -> f64 {
    let k = |i: usize| f64::from(m[c * 4 + i]) / 65536.0;
    src[0] * k(0) + src[1] * k(1) + src[2] * k(2) + k(3)
}

/// The table on `[0, 1]` (input clamped).
#[must_use]
pub fn table(t: &RefTable<'_>, x: f64, c: usize) -> f64 {
    let n = t.entries.len();
    assert!(n == 257 || n == 1025, "reference: {n} entries");
    let x = if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) };
    let pos = x * (n - 1) as f64;
    let i = (pos.floor() as usize).min(n - 2);
    let a = f64::from(t.entries[i][c]) / 65536.0;
    let b = f64::from(t.entries[i + 1][c]) / 65536.0;
    if t.interpolate {
        a + (b - a) * (pos - i as f64)
    } else {
        a
    }
}

/// The table at any real input, with the odd extension when mirrored.
#[must_use]
pub fn value(t: &RefTable<'_>, v: f64, c: usize) -> f64 {
    if t.mirror && v < 0.0 {
        -table(t, -v, c)
    } else {
        table(t, v, c)
    }
}

/// Clamp and quantise to the console's 8 bits.
#[must_use]
pub fn quantise(y: f64) -> i32 {
    let y = if y.is_nan() { 0.0 } else { y.clamp(0.0, 1.0) };
    ((y * 256.0).floor() as i32).min(255)
}

/// One channel's output at pre-table value `v` (`None` = OLUT bypass: clamp only).
#[must_use]
pub fn channel(t: Option<&RefTable<'_>>, v: f64, c: usize) -> i32 {
    quantise(t.map_or(v, |t| value(t, v, c)))
}

/// The accepted interval of one channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// Smallest result over the perturbation.
    pub lo: i32,
    /// Largest result over the perturbation.
    pub hi: i32,
}

impl Span {
    /// Does an implementation's byte fall inside?
    #[must_use]
    pub fn admits(self, got: u8) -> bool {
        (self.lo..=self.hi).contains(&i32::from(got))
    }
    /// A single value: the comparison is exact here.
    #[must_use]
    pub fn sharp(self) -> bool {
        self.lo == self.hi
    }
}

/// The reference output of one pixel: per channel, the interval of results over `v ± EPS`.
#[must_use]
pub fn output(src: [f64; 3], m: &[i32; 12], t: Option<&RefTable<'_>>) -> [Span; 3] {
    std::array::from_fn(|c| {
        let v = pre_table(src, m, c);
        let r = [v - EPS, v, v + EPS].map(|v| channel(t, v, c));
        Span {
            lo: *r.iter().min().unwrap(),
            hi: *r.iter().max().unwrap(),
        }
    })
}

/// The exact value a reader would compute without the perturbation (for messages).
#[must_use]
pub fn output_exact(src: [f64; 3], m: &[i32; 12], t: Option<&RefTable<'_>>) -> [i32; 3] {
    std::array::from_fn(|c| channel(t, pre_table(src, m, c), c))
}

// ---- configurations: what the NVIDIA app's sliders plausibly program -----------------------
//
// `[inferred]` The exact coefficients the Windows driver programs for a Hue or Digital Vibrance
// position are not recorded anywhere in git (run 4500 had no method trace). These are the two
// textbook constructions a driver can use for each control, both signed, both grey-preserving
// (rows sum to 1), plus their products, plus the encoding's boundary values. The lane
// `scripts/bench/windows/hue_vibrance_lane.sh` logs the real ones (`colour program` lines).

/// Rec.709 luma weights.
pub const LUMA: [f64; 3] = [0.2126, 0.7152, 0.0722];

/// Windows' OCSC0 rounding bias (run 99, measured), in the offset column.
pub const WINDOWS_BIAS: i32 = 0x20;

/// Encoded coefficient, 65536 = 1.0.
#[must_use]
pub fn q(x: f64) -> i32 {
    (x * 65536.0).round() as i32
}

/// 3x3 real matrix plus an offset per row to the encoded 3x4.
#[must_use]
pub fn pack(m: [[f64; 3]; 3], bias: i32) -> [i32; 12] {
    let mut out = [0; 12];
    for (r, row) in m.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            out[r * 4 + c] = q(*v);
        }
        out[r * 4 + 3] = bias;
    }
    out
}

/// The W3C `feColorMatrix hueRotate` construction (luma-ish axis, rows sum to 1).
#[must_use]
pub fn hue_w3c(deg: f64) -> [[f64; 3]; 3] {
    let (s, c) = deg.to_radians().sin_cos();
    [
        [
            0.213 + c * 0.787 - s * 0.213,
            0.715 - c * 0.715 - s * 0.715,
            0.072 - c * 0.072 + s * 0.928,
        ],
        [
            0.213 - c * 0.213 + s * 0.143,
            0.715 + c * 0.285 + s * 0.140,
            0.072 - c * 0.072 - s * 0.283,
        ],
        [
            0.213 - c * 0.213 - s * 0.787,
            0.715 - c * 0.715 + s * 0.715,
            0.072 + c * 0.928 + s * 0.072,
        ],
    ]
}

/// Hue as a rotation in the BT.709 CbCr plane: RGB -> YCbCr, rotate (Cb, Cr), back.
#[must_use]
pub fn hue_ycbcr(deg: f64) -> [[f64; 3]; 3] {
    let [kr, _, kb] = LUMA;
    let kg = 1.0 - kr - kb;
    let to = [
        [kr, kg, kb],
        [-kr / (2.0 * (1.0 - kb)), -kg / (2.0 * (1.0 - kb)), 0.5],
        [0.5, -kg / (2.0 * (1.0 - kr)), -kb / (2.0 * (1.0 - kr))],
    ];
    let back = [
        [1.0, 0.0, 2.0 * (1.0 - kr)],
        [
            1.0,
            -2.0 * kb * (1.0 - kb) / kg,
            -2.0 * kr * (1.0 - kr) / kg,
        ],
        [1.0, 2.0 * (1.0 - kb), 0.0],
    ];
    let (s, c) = deg.to_radians().sin_cos();
    let rot = [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]];
    mul(back, mul(rot, to))
}

/// Saturation `s` about the luma axis (Digital Vibrance; `s > 1` pushes primaries negative).
#[must_use]
pub fn saturation(s: f64) -> [[f64; 3]; 3] {
    std::array::from_fn(|r| {
        std::array::from_fn(|c| (1.0 - s) * LUMA[c] + if r == c { s } else { 0.0 })
    })
}

/// 3x3 product `a * b`.
#[must_use]
pub fn mul(a: [[f64; 3]; 3], b: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..3).map(|k| a[r][k] * b[k][c]).sum()))
}

/// One named OCSC0 configuration of the table.
pub struct Config {
    /// What it is, for messages.
    pub name: String,
    /// Encoded 3x4.
    pub matrix: [i32; 12],
}

/// The table: hue sweeps (both constructions, every 5 degrees, with and without the bias),
/// vibrance sweeps (saturation 0..3), hue x vibrance products, and the encoding's extremes.
#[must_use]
pub fn configurations() -> Vec<Config> {
    let mut out = Vec::new();
    let mut add = |name: String, matrix: [i32; 12]| out.push(Config { name, matrix });
    for d in (0..360).step_by(5) {
        let d = f64::from(d);
        add(format!("hue_w3c {d}"), pack(hue_w3c(d), WINDOWS_BIAS));
        add(format!("hue_ycbcr {d}"), pack(hue_ycbcr(d), WINDOWS_BIAS));
        add(format!("hue_w3c {d} no-bias"), pack(hue_w3c(d), 0));
    }
    for s in [
        0.0, 0.1, 0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0,
    ] {
        add(format!("saturation {s}"), pack(saturation(s), WINDOWS_BIAS));
        add(format!("saturation {s} no-bias"), pack(saturation(s), 0));
    }
    for d in (0..360).step_by(30) {
        for s in [0.5, 1.5, 2.0, 3.0] {
            let d = f64::from(d);
            add(
                format!("hue_w3c {d} x saturation {s}"),
                pack(mul(saturation(s), hue_w3c(d)), WINDOWS_BIAS),
            );
        }
    }
    // the S5.14 field's extremes: +-16.0 (the field is 21 bits, 65536 = 1.0)
    let max = (1 << 20) - 1;
    let min = -(1 << 20);
    add(
        "field max diagonal".into(),
        [max, 0, 0, 0, 0, max, 0, 0, 0, 0, max, 0],
    );
    add(
        "field min diagonal".into(),
        [min, 0, 0, 0, 0, min, 0, 0, 0, 0, min, 0],
    );
    add(
        "field min offsets".into(),
        [65536, 0, 0, min, 0, 65536, 0, min, 0, 0, 65536, min],
    );
    add(
        "field max offsets".into(),
        [65536, 0, 0, max, 0, 65536, 0, max, 0, 0, 65536, max],
    );
    add(
        "smallest negative offset".into(),
        [65536, 0, 0, -1, 0, 65536, 0, -1, 0, 0, 65536, -1],
    );
    add(
        "R minus G (hand case)".into(),
        [65536, -32768, 0, 0, 0, 65536, 0, 0, 0, 0, 65536, 0],
    );
    add(
        "identity + bias".into(),
        pack(saturation(1.0), WINDOWS_BIAS),
    );
    add("all zero".into(), [0; 12]);
    out
}

// ---- tables ----------------------------------------------------------------------------------

/// A named OLUT shape.
pub struct NamedTable {
    /// What it is.
    pub name: &'static str,
    /// Samples.
    pub entries: Vec<[u16; 3]>,
}

/// UNORM16 of `x` in [0, 1].
#[must_use]
pub fn unorm(x: f64) -> u16 {
    (x.clamp(0.0, 1.0) * 65535.0).round() as u16
}

/// Tables of both sizes: identity, the lifted black of a brightness slider, a 2.2 gamma, an
/// inverting ramp, a staircase (adjacent samples far apart, so a wrong index shows), per-channel
/// different, all-black and all-white.
#[must_use]
pub fn tables() -> Vec<NamedTable> {
    let mut out = Vec::new();
    for n in [257_usize, 1025] {
        let x = |i: usize| i as f64 / (n - 1) as f64;
        let mk = |name: &'static str, f: &dyn Fn(f64) -> [f64; 3]| NamedTable {
            name,
            entries: (0..n).map(|i| f(x(i)).map(unorm)).collect(),
        };
        out.push(mk("identity", &|x| [x; 3]));
        out.push(mk("lifted black 0.25+0.5x", &|x| [0.25 + 0.5 * x; 3]));
        out.push(mk("gamma 2.2", &|x| [x.powf(1.0 / 2.2); 3]));
        out.push(mk("inverting 1-x", &|x| [1.0 - x; 3]));
        out.push(mk("staircase", &|x| {
            [((x * 8.0).floor() / 8.0).min(1.0); 3]
        }));
        out.push(mk("per-channel", &|x| [x, x * x, 0.1 + 0.9 * x]));
        out.push(mk("black", &|_| [0.0; 3]));
        out.push(mk("white", &|_| [1.0; 3]));
    }
    out
}

/// Source values: FP16-exact multiples of 1/256 in [0, 1] (the compose stage leaves FP16).
#[must_use]
pub fn grid(levels: &[u32]) -> Vec<[f32; 3]> {
    let l: Vec<f32> = levels.iter().map(|&k| k as f32 / 256.0).collect();
    let mut out = Vec::new();
    for &r in &l {
        for &g in &l {
            for &b in &l {
                out.push([r, g, b]);
            }
        }
    }
    out
}

// ---- deterministic generator for the bounded property tests ----------------------------------

/// SplitMix64: deterministic, seedable, dependency-free.
pub struct Rng(pub u64);

impl Rng {
    /// Next 64 random bits.
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    /// Uniform in `0..n`.
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    /// A random bit.
    pub fn coin(&mut self) -> bool {
        self.next() & 1 == 1
    }
}

/// Sign-extend a 21-bit field word, written independently of the decoder's shift trick.
#[must_use]
pub fn sign_extend_21(word: u32) -> i32 {
    let w = (word & 0x1f_ffff) as i32;
    if w & (1 << 20) != 0 { w - (1 << 21) } else { w }
}
