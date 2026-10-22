//! The CPU reference of the head's output stage: what `kf_color_output` (cuda/display/kf_color.cu)
//! computes for one pixel, step for step and in the same `f32` order (the kernel is built with
//! `-ffp-contract=off`, so there is no fused multiply-add on either side).
//!
//! It is an ORACLE, not an executor: nothing in the product calls it to produce a frame (no CPU
//! executor for GPU work). It exists so the kernel's arithmetic is written down where a GPU-free
//! test can reason about it, and so a hardware run can be compared against it value for value.
//! The input frame is the FP16/FP32 composition the compose kernel leaves; the table is the head's
//! OLUT as a snapshot (`entries` = the 257 or 1025 samples after the four header entries).

/// The head's OLUT as the kernel samples it.
#[derive(Debug, Clone, Copy)]
pub struct Table<'a> {
    /// RGB samples (UNORM16 each; the padding word is not modelled), 257 or 1025 of them.
    pub entries: &'a [[u16; 3]],
    /// Interpolate between adjacent samples instead of taking the lower one.
    pub interpolate: bool,
    /// `HEAD_SET_OLUT_CONTROL_MIRROR`: the guest armed the mirrored form, read as the odd extension
    /// of the table, `lut(-x) = -lut(x)` (see [`value`]). `[inferred]`: ogkm names the bit and
    /// NVKMS never sets it; nothing public says what it does.
    pub mirror: bool,
}

/// Clamp to `[0, 1]`; a NaN becomes 0 (the kernel's `sat`).
#[must_use]
pub fn sat(x: f32) -> f32 {
    if x.is_nan() || x <= 0.0 {
        0.0
    } else if x < 1.0 {
        x
    } else {
        1.0
    }
}

/// One channel of the table at `x`, the kernel's `lookup` for the output stage: the table spans
/// `[0, 1]`, so the index is clamped there first (`x` may be negative or above 1).
#[must_use]
pub fn lookup(t: &Table<'_>, x: f32, c: usize) -> f32 {
    let n = t.entries.len();
    if n != 257 && n != 1025 {
        return 0.0;
    }
    let pos = sat(x) * (n - 1) as f32;
    let i = (pos as usize).min(n - 2);
    let fa = f32::from(t.entries[i][c]) / 65536.0;
    let fb = f32::from(t.entries[i + 1][c]) / 65536.0;
    if t.interpolate {
        fa + (fb - fa) * (pos - i as f32)
    } else {
        fa
    }
}

/// The table at `x` for channel `c`, including the negative side of a mirrored table: the odd
/// extension, `-lookup(-x)` for `x < 0`. An unmirrored table clamps a negative `x` to its first
/// sample. A NaN is not negative (it samples the first entry, as the kernel's `sat` makes it).
#[must_use]
pub fn value(t: &Table<'_>, x: f32, c: usize) -> f32 {
    if t.mirror && x < 0.0 {
        -lookup(t, -x, c)
    } else {
        lookup(t, x, c)
    }
}

/// One pixel through OCSC0 (`matrix`, the encoded 3x4 with 65536 = 1.0) and the OLUT (or bypass),
/// to the 8-bit channels the console frame holds.
#[must_use]
pub fn output_rgb(src: [f32; 3], matrix: &[i32; 12], lut: Option<&Table<'_>>) -> [u8; 3] {
    std::array::from_fn(|c| {
        let m = &matrix[c * 4..c * 4 + 4];
        let v = src[0] * (m[0] as f32 / 65536.0)
            + src[1] * (m[1] as f32 / 65536.0)
            + src[2] * (m[2] as f32 / 65536.0)
            + m[3] as f32 / 65536.0;
        let v = lut.map_or_else(|| sat(v), |t| value(t, v, c));
        ((sat(v) * 256.0) as u32).min(255) as u8
    })
}

/// What the NVIDIA app's Hue and Digital Vibrance sliders do to the head's OCSC0, as test data.
/// `[inferred]` the exact coefficients the app programs are not in git (run 4500's evidence is on
/// the trusted host); these are the textbook luma-preserving hue rotation and saturation matrices,
/// which have the property the failure needs: signed coefficients, so a channel can go negative.
#[cfg(test)]
pub(crate) mod fixture {
    /// Encoded 21-bit-field coefficient (65536 = 1.0), the form `color::output` decodes.
    fn q(x: f64) -> i32 {
        (x * 65536.0).round() as i32
    }

    /// Rec.709 luma weights.
    const LUMA: [f64; 3] = [0.2126, 0.7152, 0.0722];

    /// The W3C `feColorMatrix hueRotate` matrix for `deg` degrees (rows sum to 1: grey is fixed).
    pub fn hue(deg: f64) -> [i32; 12] {
        let (s, c) = deg.to_radians().sin_cos();
        let m = [
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
        ];
        pack(m)
    }

    /// Saturation `s` about the luma axis (`s` > 1 pushes primaries past the gamut: negatives).
    pub fn saturation(s: f64) -> [i32; 12] {
        let mut m = [[0.0; 3]; 3];
        for (r, row) in m.iter_mut().enumerate() {
            for (c, v) in row.iter_mut().enumerate() {
                *v = (1.0 - s) * LUMA[c] + if r == c { s } else { 0.0 };
            }
        }
        pack(m)
    }

    /// The 3x4 encoding with the OCSC0 rounding bias Windows programs (`0x20`) in the offset column.
    fn pack(m: [[f64; 3]; 3]) -> [i32; 12] {
        let mut out = [0; 12];
        for (r, row) in m.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                out[r * 4 + c] = q(*v);
            }
            out[r * 4 + 3] = 0x20;
        }
        out
    }

    /// A DIRECT10 table whose black is lifted, as a brightness slider makes it:
    /// `lut(x) = 0.25 + 0.5 x`, UNORM16, 1025 samples.
    pub fn lifted_black() -> Vec<[u16; 3]> {
        (0..1025_u32)
            .map(|i| [(16384 + i * 32) as u16; 3])
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{hue, lifted_black, saturation};
    use super::*;

    const IDENTITY_BIAS: [i32; 12] = [65536, 0, 0, 0x20, 0, 65536, 0, 0x20, 0, 0, 65536, 0x20];

    /// The lifted-black table as a real number: `0.25 + 0.5 x` on `[0, 1]`.
    fn table64(x: f64) -> f64 {
        0.25 + 0.5 * x.clamp(0.0, 1.0)
    }

    /// An f64 oracle written from the reading under test, not from the kernel: the pre-table value
    /// from the encoded coefficients, the table applied to its magnitude and given the value's
    /// sign when `mirror`, the result clamped and quantised to 8 bits.
    fn expect(src: [f64; 3], m: &[i32; 12], mirror: bool) -> [i32; 3] {
        std::array::from_fn(|c| {
            let v = pre_table(src, m, c);
            let y = if mirror && v < 0.0 {
                -table64(-v)
            } else {
                table64(v)
            };
            ((y.clamp(0.0, 1.0) * 256.0).floor() as i32).min(255)
        })
    }

    fn pre_table(src: [f64; 3], m: &[i32; 12], c: usize) -> f64 {
        let k = |i: usize| f64::from(m[c * 4 + i]) / 65536.0;
        src[0] * k(0) + src[1] * k(1) + src[2] * k(2) + k(3)
    }

    fn grid() -> Vec<[f32; 3]> {
        let l = [0.0_f32, 0.25, 0.5, 0.75, 1.0];
        let mut out = Vec::new();
        for r in l {
            for g in l {
                for b in l {
                    out.push([r, g, b]);
                }
            }
        }
        out
    }

    fn within_one(a: [u8; 3], b: [i32; 3]) -> bool {
        a.iter().zip(b).all(|(&a, b)| (i32::from(a) - b).abs() <= 1)
    }

    /// Every hue angle and saturation against the colour grid: how many channels were negative
    /// before the table, and the mismatches against the oracle.
    fn sweep(mirror: bool) -> (usize, Vec<String>) {
        let entries = lifted_black();
        let t = Table {
            entries: &entries,
            interpolate: true,
            mirror,
        };
        let matrices: Vec<(String, [i32; 12])> = (0..360)
            .step_by(15)
            .map(|d| (format!("hue {d}"), hue(f64::from(d))))
            .chain(
                [0.25, 0.5, 1.0, 1.5, 2.0, 3.0].map(|s| (format!("saturation {s}"), saturation(s))),
            )
            .collect();
        let (mut negative, mut bad) = (0, Vec::new());
        for (name, m) in &matrices {
            for src in grid() {
                let s64 = src.map(f64::from);
                let got = output_rgb(src, m, Some(&t));
                let want = expect(s64, m, mirror);
                negative += (0..3).filter(|&c| pre_table(s64, m, c) < -1e-3).count();
                if !within_one(got, want) {
                    bad.push(format!("{name} {src:?}: got {got:?}, want {want:?}"));
                }
            }
        }
        (negative, bad)
    }

    /// `[inferred]` MIRROR is the odd extension of the table: `lut(-x) = -lut(x)`. Behind the
    /// signed hue and saturation matrices the 8-bit result is the table of the magnitude with the
    /// value's sign, clamped: a channel that went negative is black, whatever the table's black is.
    #[test]
    fn a_mirrored_table_behind_hue_and_saturation_matrices_is_odd_symmetric() {
        let (negative, bad) = sweep(true);
        assert!(
            negative > 500,
            "the sweep must reach the negative side ({negative})"
        );
        assert!(
            bad.is_empty(),
            "{} mismatches, first: {:?}",
            bad.len(),
            &bad[..bad.len().min(3)]
        );
    }

    /// Unmirrored (the ordinary case, and what ships today): the table index is clamped to [0, 1],
    /// so a negative input samples the table's first entry, the lifted black.
    #[test]
    fn an_unmirrored_table_samples_its_black_for_a_negative_input() {
        let (negative, bad) = sweep(false);
        assert!(negative > 500);
        assert!(
            bad.is_empty(),
            "{} mismatches, first: {:?}",
            bad.len(),
            &bad[..bad.len().min(3)]
        );
    }

    /// Hand-computed, no oracle code: src (0.25, 0.75, 0), R' = R - G/2 = -0.125. Table
    /// 0.25 + 0.5 x. Mirrored: R is -(0.25 + 0.0625), clamped to black; G' = 0.75 -> 0.625 -> 160;
    /// B' = 0 -> the first sample 0.25 -> 64. Unmirrored: R' samples the first entry: 64.
    #[test]
    fn a_hand_computed_negative_channel() {
        let entries = lifted_black();
        let mut m = IDENTITY_BIAS;
        m[1] = -32768; // R' = R - G / 2
        m[3] = 0;
        m[7] = 0;
        m[11] = 0;
        let src = [0.25, 0.75, 0.0];
        let t = |mirror| Table {
            entries: &entries,
            interpolate: false,
            mirror,
        };
        assert_eq!(output_rgb(src, &m, Some(&t(true))), [0, 160, 64]);
        assert_eq!(output_rgb(src, &m, Some(&t(false))), [64, 160, 64]);
    }

    /// Hand-computed: pure red at saturation 1.5 -> R' = 1.5 - 0.5 * 0.2126 = 1.3937 (past 1),
    /// G' = -0.5 * 0.2126 = -0.106, B' = -0.5 * 0.2126 as well. Mirrored: R' clamps to the table's
    /// last sample (0.75 -> 192); G' and B' are negative -> black. An EVEN extension would give
    /// lut(0.106) -> 77 on G and B instead, which is what a hardware run must rule out.
    #[test]
    fn digital_vibrance_on_a_pure_primary_blackens_the_negative_channels() {
        let entries = lifted_black();
        let t = Table {
            entries: &entries,
            interpolate: true,
            mirror: true,
        };
        assert_eq!(
            output_rgb([1.0, 0.0, 0.0], &saturation(1.5), Some(&t)),
            [192, 0, 0]
        );
    }

    /// Where no channel is negative the mirror flag changes nothing (the case the shipped gate
    /// admits).
    #[test]
    fn mirror_is_inert_for_nonnegative_input() {
        let entries = lifted_black();
        let mk = |mirror| Table {
            entries: &entries,
            interpolate: true,
            mirror,
        };
        for m in [IDENTITY_BIAS, saturation(1.0)] {
            for src in grid() {
                assert_eq!(
                    output_rgb(src, &m, Some(&mk(true))),
                    output_rgb(src, &m, Some(&mk(false))),
                    "{src:?}"
                );
            }
        }
    }
}
