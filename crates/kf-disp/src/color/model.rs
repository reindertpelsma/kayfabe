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
    /// `HEAD_SET_OLUT_CONTROL_MIRROR`: the guest armed the mirrored form. As the kernel ships
    /// today it has no effect (the program gate admits a mirrored table only where no input can be
    /// negative, so the form cannot matter).
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
        let v = lut.map_or_else(|| sat(v), |t| lookup(t, v, c));
        ((sat(v) * 256.0) as u32).min(255) as u8
    })
}
