// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ The console's YUV window kernel (`cuda/display/kf_yuv.cu`) RUN ON THE HOST: the same source the
//! NVPTX back-end compiles (`make_yuv_ptx.sh`), built as plain C with `-DKF_HOST`, driven over every
//! (CTA, thread) of the launch and compared byte for byte with `kf_disp::scanout::yuv_reference` —
//! pitch and block-linear planes, 4:2:0 / 4:2:2 / 4:4:4, both chroma orders, an upscale, a downscale,
//! an odd size, and a destination the frame edge cuts. Plus known-answer tests of the colour
//! arithmetic (NV12 values with known RGB results).

use kf_disp::scanout::{YuvPlan, yuv_reference, yuv_to_xrgb};
use std::path::{Path, PathBuf};
use std::process::Command;

fn kernel_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cuda/display/kf_yuv.cu")
}

/// argv: bl yp cp bh sx0 sy0 sw sh ox oy dw dh fw fh sxl syl vu, then ys, cs, out paths.
const HARNESS: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#define KF_HOST 1
#include KF_SOURCE
static unsigned char *slurp(const char *p, size_t *n) {
    FILE *f = fopen(p, "rb"); if (!f) exit(3);
    fseek(f, 0, SEEK_END); *n = (size_t)ftell(f); fseek(f, 0, SEEK_SET);
    unsigned char *b = malloc(*n + 1); if (fread(b, 1, *n, f) != *n) exit(4); fclose(f); return b;
}
int main(int argc, char **argv) {
    if (argc != 21) return 2;
    (void)&kf_yuv_row; (void)&kf_yuv_row_f;
    /* argv[1..17] numbers, [18] ys, [19] cs, [20] out */
    unsigned v[17];
    for (int i = 0; i < 17; i++) v[i] = (unsigned)strtoul(argv[i + 1], 0, 10);
    size_t yn, cn; unsigned char *ys = slurp(argv[18], &yn), *cs = slurp(argv[19], &cn);
    unsigned fw = v[12], fh = v[13];
#ifdef FLOAT_DST
    float *dst = calloc((size_t)fw * fh, 16);
    for (unsigned row = 0; row < v[11]; row++)
        for (unsigned t = 0; t < KF_YUV_THREADS; t++)
            kf_yuv_row_f(ys, cs, dst, v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7], v[8], v[9],
                         v[10], v[11], fw, fh, v[14], v[15], v[16], row, t);
    FILE *f = fopen(argv[20], "wb"); fwrite(dst, 16, (size_t)fw * fh, f); fclose(f);
    return 0;
#else
    unsigned int *dst = calloc((size_t)fw * fh, 4);
    for (unsigned row = 0; row < v[11]; row++)
        for (unsigned t = 0; t < KF_YUV_THREADS; t++)
            kf_yuv_row(ys, cs, dst, v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7], v[8], v[9],
                       v[10], v[11], fw, fh, v[14], v[15], v[16], row, t);
    FILE *f = fopen(argv[20], "wb"); fwrite(dst, 4, (size_t)fw * fh, f); fclose(f);
    return 0;
#endif
}
"#;

fn build(dir: &Path, float_dst: bool) -> PathBuf {
    let c = dir.join("harness.c");
    std::fs::write(&c, HARNESS).unwrap();
    let exe = dir.join(if float_dst { "harness_f" } else { "harness" });
    let out = Command::new(std::env::var("CC").unwrap_or_else(|_| "cc".into()))
        .args(["-std=c99", "-O1", "-Wall", "-Werror", "-o"])
        .arg(&exe)
        .arg(format!("-DKF_SOURCE=\"{}\"", kernel_source().display()))
        .args(if float_dst {
            vec!["-DFLOAT_DST=1"]
        } else {
            vec![]
        })
        .arg("-x")
        .arg("c")
        .arg(&c)
        .output()
        .expect("the system C compiler");
    assert!(
        out.status.success(),
        "the kernel source does not build as C: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    exe
}

fn noise(n: usize, seed: u64) -> Vec<u8> {
    (0..n as u64)
        .map(|i| (i.wrapping_add(seed).wrapping_mul(2_654_435_761) >> 11) as u8)
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn case(exe: &Path, exe_f: &Path, dir: &Path, name: &str, l: &YuvPlan, fw: u32, fh: u32) {
    let (ys, cs) = (noise(l.y_extent as usize, 1), noise(l.c_extent as usize, 7));
    let (yp, cp_, op) = (
        dir.join(format!("{name}.y")),
        dir.join(format!("{name}.c")),
        dir.join(format!("{name}.out")),
    );
    std::fs::write(&yp, &ys).unwrap();
    std::fs::write(&cp_, &cs).unwrap();
    let args = [
        u32::from(l.block_linear),
        l.y_pitch,
        l.c_pitch,
        l.block_height_log2,
        l.sx0,
        l.sy0,
        l.sw,
        l.sh,
        l.ox,
        l.oy,
        l.dw,
        l.dh,
        fw,
        fh,
        l.sub_x_log2,
        l.sub_y_log2,
        u32::from(l.vu_first),
    ];
    let st = Command::new(exe)
        .args(args.iter().map(u32::to_string))
        .arg(&yp)
        .arg(&cp_)
        .arg(&op)
        .status()
        .unwrap();
    assert!(st.success(), "{name}: harness {st:?}");
    let got = std::fs::read(&op).unwrap();
    let mut want = vec![0u8; (fw * fh * 4) as usize];
    yuv_reference(l, &ys, &cs, &mut want, fw, fh).unwrap();
    assert_eq!(got.len(), want.len());
    assert!(
        got == want,
        "{name}: the kernel body and yuv_reference differ"
    );
    // the FP32 variant (the colour pipeline's frame): the same pixels as value / 255, alpha 1
    let st = Command::new(exe_f)
        .args(args.iter().map(u32::to_string))
        .arg(&yp)
        .arg(&cp_)
        .arg(&op)
        .status()
        .unwrap();
    assert!(st.success(), "{name}: float harness {st:?}");
    let gotf = std::fs::read(&op).unwrap();
    assert_eq!(gotf.len(), want.len() * 4);
    for (i, px) in want.chunks(4).enumerate() {
        let f = |o: usize| f32::from_le_bytes(gotf[i * 16 + o..i * 16 + o + 4].try_into().unwrap());
        let (r, g, b, a) = (f(0), f(4), f(8), f(12));
        if px[3] == 0xff {
            let want_f = [px[2], px[1], px[0]].map(|v| f32::from(v) / 255.0);
            assert!(
                (r - want_f[0]).abs() < 1e-6
                    && (g - want_f[1]).abs() < 1e-6
                    && (b - want_f[2]).abs() < 1e-6
                    && a == 1.0,
                "{name}: float pixel {i}"
            );
        } else {
            assert!(
                r == 0.0 && g == 0.0 && b == 0.0 && a == 0.0,
                "{name}: untouched pixel {i}"
            );
        }
    }
    // something was written (the opaque alpha byte)
    assert!(
        want.chunks(4).any(|p| p[3] == 0xff),
        "{name}: nothing composed"
    );
}

fn plan(bl: bool, pitch_y: u32, pitch_c: u32, bh: u32) -> YuvPlan {
    // extents are generous: the reference refuses a read past them
    YuvPlan {
        window: 4,
        y_src: 0,
        y_extent: if bl { 1 << 20 } else { 1 << 16 },
        c_src: 0,
        c_extent: if bl { 1 << 20 } else { 1 << 16 },
        block_linear: bl,
        y_pitch: pitch_y,
        c_pitch: pitch_c,
        block_height_log2: bh,
        sx0: 0,
        sy0: 0,
        sw: 64,
        sh: 48,
        ox: 0,
        oy: 0,
        dw: 64,
        dh: 48,
        sub_x_log2: 1,
        sub_y_log2: 1,
        vu_first: true,
    }
}

#[test]
fn the_kernel_body_is_the_reference_for_every_layout_and_scale() {
    let dir = std::env::temp_dir().join(format!("kf-yuv-kernel-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = build(&dir, false);
    let exe_f = build(&dir, true);
    let fw = 120;
    let fh = 90;
    // pitch, unscaled, 4:2:0 VU
    case(
        &exe,
        &exe_f,
        &dir,
        "pitch420",
        &plan(false, 128, 128, 0),
        fw,
        fh,
    );
    // upscale and downscale, an offset origin and destination, a destination the frame edge cuts
    let mut up = plan(false, 128, 128, 0);
    (up.sx0, up.sy0, up.sw, up.sh, up.ox, up.oy, up.dw, up.dh) = (6, 4, 31, 23, 20, 10, 101, 85);
    case(&exe, &exe_f, &dir, "upscale", &up, fw, fh);
    let mut down = plan(false, 128, 128, 0);
    (down.dw, down.dh, down.ox, down.oy) = (17, 11, 3, 5);
    case(&exe, &exe_f, &dir, "downscale", &down, fw, fh);
    // 4:2:2, 4:4:4, UV order
    let mut p422 = plan(false, 128, 128, 0);
    (p422.sub_y_log2, p422.vu_first) = (0, false);
    case(&exe, &exe_f, &dir, "p422", &p422, fw, fh);
    let mut p444 = plan(false, 128, 128, 0);
    (p444.sub_x_log2, p444.sub_y_log2) = (0, 0);
    case(&exe, &exe_f, &dir, "p444", &p444, fw, fh);
    // block-linear, every block height; 128 GOBs-wide enough for 64 luma bytes / 128 chroma bytes
    for bh in 0..=3 {
        let mut p = plan(true, 4, 4, bh);
        (p.sx0, p.sy0, p.sw, p.sh, p.dw, p.dh) = (5, 3, 50, 41, 90, 70);
        case(&exe, &exe_f, &dir, &format!("bl{bh}"), &p, fw, fh);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Known answers of the NV12/NV21 arithmetic (BT.709, limited range): black, white, grey, and the
/// primaries, as the 8.8 fixed-point formula gives them.
#[test]
fn yuv_known_values() {
    let px = |y, u, v| yuv_to_xrgb(y, u, v) & 0x00ff_ffff;
    assert_eq!(px(16, 128, 128), 0x0000_0000, "limited-range black");
    assert_eq!(px(235, 128, 128), 0x00ff_ffff, "limited-range white");
    assert_eq!(
        px(126, 128, 128),
        0x0080_8080,
        "mid grey (126 -> 128 after the 255/219 gain)"
    );
    // BT.709 red / green / blue at full saturation (integer coefficients: within 2 of the ideal)
    let near = |got: u32, want: [i32; 3]| {
        let c = [(got >> 16) & 255, (got >> 8) & 255, got & 255].map(|x| x as i32);
        assert!(
            c.iter().zip(want).all(|(g, w)| (g - w).abs() <= 2),
            "{c:?} vs {want:?}"
        );
    };
    near(px(63, 102, 240), [255, 0, 0]);
    near(px(173, 42, 26), [0, 255, 0]);
    near(px(32, 240, 118), [0, 0, 255]);
    assert_eq!(yuv_to_xrgb(0, 0, 0) >> 24, 0xff, "opaque");
}
