// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ The console's YUV window kernel (`cuda/display/kf_yuv.cu`) RUN ON THE HOST: the same source the
//! NVPTX back-end compiles (`make_yuv_ptx.sh`), built as plain C with `-DKF_HOST`, driven over every
//! (CTA, thread) of the launch and compared byte for byte with `kf_disp::scanout::yuv_reference` —
//! pitch and block-linear planes, 4:2:0 / 4:2:2 / 4:4:4, both chroma orders, 8-bit and 16-bit
//! semi-planar, packed YUY2/UYVY, nearest and bilinear scaling (up, down, odd origins), a destination
//! the frame edge cuts, and arbitrary FMT matrices. Plus known-answer tests of the colour arithmetic
//! against NVKMS's own FMT tables (`ogkm-595.84: nvidia-modeset/src/nvkms-evo3.c` FMTMatrix).

use kf_disp::scanout::{
    BT709_LIMITED, YUV_PACKED_UYVY, YUV_PACKED_YUYV, YUV_SEMI_PLANAR_8, YUV_SEMI_PLANAR_16, YuvPlan,
    yuv_convert, yuv_reference, yuv_to_xrgb,
};
use std::path::{Path, PathBuf};
use std::process::Command;

fn kernel_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cuda/display/kf_yuv.cu")
}

/// argv: 19 numbers (bl yp cp bh sx0 sy0 sw sh ox oy dw dh fw fh sxl syl vu layout filter), 12
/// matrix words, then ys, cs, out paths.
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
    if (argc != 35) return 2;
    (void)&kf_yuv_row; (void)&kf_yuv_row_f;
    unsigned v[19]; int m[12];
    for (int i = 0; i < 19; i++) v[i] = (unsigned)strtoul(argv[i + 1], 0, 10);
    for (int i = 0; i < 12; i++) m[i] = (int)strtol(argv[i + 20], 0, 10);
    size_t yn, cn; unsigned char *ys = slurp(argv[32], &yn), *cs = slurp(argv[33], &cn);
    kf_yuv_args a = { ys, cs, v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7], v[8], v[9], v[10],
                      v[11], v[12], v[13], v[14], v[15], v[16], v[17], v[18],
                      { m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8], m[9], m[10], m[11] } };
    unsigned fw = v[12], fh = v[13];
#ifdef FLOAT_DST
    float *dst = calloc((size_t)fw * fh, 16);
    for (unsigned row = 0; row < v[11]; row++)
        for (unsigned t = 0; t < KF_YUV_THREADS; t++)
            kf_yuv_row_f(&a, dst, row, t);
    FILE *f = fopen(argv[34], "wb"); fwrite(dst, 16, (size_t)fw * fh, f); fclose(f);
    return 0;
#else
    unsigned int *dst = calloc((size_t)fw * fh, 4);
    for (unsigned row = 0; row < v[11]; row++)
        for (unsigned t = 0; t < KF_YUV_THREADS; t++)
            kf_yuv_row(&a, dst, row, t);
    FILE *f = fopen(argv[34], "wb"); fwrite(dst, 4, (size_t)fw * fh, f); fclose(f);
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
    let ys = noise(l.y_extent as usize, 1);
    // a packed format reads its chroma from plane 0: the reference gets the same bytes twice
    let cs = if l.layout >= YUV_PACKED_YUYV {
        ys.clone()
    } else {
        noise(l.c_extent as usize, 7)
    };
    let (yp, cp_, op) = (
        dir.join(format!("{name}.y")),
        dir.join(format!("{name}.c")),
        dir.join(format!("{name}.out")),
    );
    std::fs::write(&yp, &ys).unwrap();
    std::fs::write(&cp_, &cs).unwrap();
    let mut args: Vec<String> = [
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
        l.layout,
        l.filter,
    ]
    .iter()
    .map(u32::to_string)
    .collect();
    args.extend(l.fmt.iter().map(i32::to_string));
    let st = Command::new(exe)
        .args(&args)
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
        .args(&args)
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
        layout: YUV_SEMI_PLANAR_8,
        filter: 0,
        fmt: BT709_LIMITED,
    }
}

/// NVKMS's REC601 8-bit limited-range FMT matrix (`nvkms-evo3.c` FMTMatrix row
/// `FMT_COEFF_TYPE_REC601_YUV_8BPC_LTD_TO_RGB_16BPC_FULL`), sign-extended from 21 bits — a test
/// oracle (public source), never product data.
fn sx21(v: u32) -> i32 {
    ((v << 11) as i32) >> 11
}
fn nvkms_601_ltd() -> [i32; 12] {
    [
        0x19A29, 0x12B3C, 0, 0x1F2038, 0x1F2F14, 0x12B3C, 0x1F9B52, 0x8819, 0, 0x12B3C, 0x20668,
        0x1EEA18,
    ]
    .map(sx21)
}
fn nvkms_709_ltd() -> [i32; 12] {
    [
        0x1CCB7, 0x12B3C, 0, 0x1F06F1, 0x1F770C, 0x12B3C, 0x1FC933, 0x4D2D, 0, 0x12B3C, 0x21EDD,
        0x1EDDDE,
    ]
    .map(sx21)
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
    // upscale and downscale (nearest and bilinear), an offset origin and destination, a
    // destination the frame edge cuts
    for filter in [0, 1] {
        let mut up = plan(false, 128, 128, 0);
        (up.sx0, up.sy0, up.sw, up.sh, up.ox, up.oy, up.dw, up.dh) = (6, 4, 31, 23, 20, 10, 101, 85);
        up.filter = filter;
        case(&exe, &exe_f, &dir, &format!("upscale{filter}"), &up, fw, fh);
        let mut down = plan(false, 128, 128, 0);
        (down.dw, down.dh, down.ox, down.oy) = (17, 11, 3, 5);
        down.filter = filter;
        case(&exe, &exe_f, &dir, &format!("downscale{filter}"), &down, fw, fh);
        // an odd origin with 4:2:0 chroma (the chroma clamp at both ends)
        let mut odd = plan(false, 128, 128, 0);
        (odd.sx0, odd.sy0, odd.sw, odd.sh, odd.dw, odd.dh) = (7, 5, 33, 27, 70, 61);
        odd.filter = filter;
        case(&exe, &exe_f, &dir, &format!("odd{filter}"), &odd, fw, fh);
    }
    // 4:2:2, 4:4:4, UV order
    let mut p422 = plan(false, 128, 128, 0);
    (p422.sub_y_log2, p422.vu_first) = (0, false);
    case(&exe, &exe_f, &dir, "p422", &p422, fw, fh);
    let mut p444 = plan(false, 128, 128, 0);
    (p444.sub_x_log2, p444.sub_y_log2) = (0, 0);
    case(&exe, &exe_f, &dir, "p444", &p444, fw, fh);
    // 16-bit containers (P010-class): luma 2 bytes a pixel, chroma 4 bytes a pair
    let mut p010 = plan(false, 128, 128, 0);
    p010.layout = YUV_SEMI_PLANAR_16;
    case(&exe, &exe_f, &dir, "p010", &p010, fw, fh);
    p010.filter = 1;
    (p010.sx0, p010.sw, p010.dw, p010.dh) = (3, 41, 97, 77);
    case(&exe, &exe_f, &dir, "p010-scaled", &p010, fw, fh);
    // packed 4:2:2 (one plane), both orders, nearest and bilinear
    for (name, layout) in [("yuyv", YUV_PACKED_YUYV), ("uyvy", YUV_PACKED_UYVY)] {
        let mut p = plan(false, 128, 128, 0);
        (p.layout, p.sub_y_log2, p.vu_first) = (layout, 0, false);
        case(&exe, &exe_f, &dir, name, &p, fw, fh);
        (p.sx0, p.sy0, p.sw, p.sh, p.dw, p.dh, p.filter) = (5, 2, 51, 40, 110, 80, 1);
        case(&exe, &exe_f, &dir, &format!("{name}-scaled"), &p, fw, fh);
    }
    // the window's own matrix (NVKMS's BT.601 limited, and a hostile one: huge, negative)
    let mut m601 = plan(false, 128, 128, 0);
    m601.fmt = nvkms_601_ltd();
    case(&exe, &exe_f, &dir, "fmt601", &m601, fw, fh);
    m601.fmt = [
        -(1 << 20),
        (1 << 20) - 1,
        -(1 << 20),
        -(1 << 20),
        (1 << 20) - 1,
        0,
        (1 << 20) - 1,
        (1 << 20) - 1,
        3,
        -7,
        65_536,
        0,
    ];
    case(&exe, &exe_f, &dir, "fmt-extreme", &m601, fw, fh);
    // block-linear, every block height; 128 GOBs-wide enough for 64 luma bytes / 128 chroma bytes
    for bh in 0..=3 {
        let mut p = plan(true, 4, 4, bh);
        (p.sx0, p.sy0, p.sw, p.sh, p.dw, p.dh) = (5, 3, 50, 41, 90, 70);
        case(&exe, &exe_f, &dir, &format!("bl{bh}"), &p, fw, fh);
        p.filter = 1;
        case(&exe, &exe_f, &dir, &format!("bl{bh}-bilinear"), &p, fw, fh);
        p.layout = YUV_SEMI_PLANAR_16;
        case(&exe, &exe_f, &dir, &format!("bl{bh}-16"), &p, fw, fh);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Known answers of the conversion: the console's own BT.709 limited matrix (black, white, grey,
/// the primaries), and NVKMS's FMT matrices through the same arithmetic — what the guest programs is
/// what the console uses.
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
    // BT.709 red / green / blue at full saturation (within 2 of the ideal)
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
    // NVKMS's own matrices: 709 gives the same answers; 601 the 601 ones (red = 81,90,240)
    let s = |v: u8| u32::from(v) << 8;
    let n709 = nvkms_709_ltd();
    near(
        yuv_convert(&n709, s(63), s(102), s(240)) & 0xff_ffff,
        [255, 0, 0],
    );
    assert_eq!(
        yuv_convert(&n709, s(235), s(128), s(128)) & 0xff_ffff,
        0xff_ffff
    );
    assert_eq!(yuv_convert(&n709, s(16), s(128), s(128)) & 0xff_ffff, 0);
    let n601 = nvkms_601_ltd();
    near(
        yuv_convert(&n601, s(81), s(90), s(240)) & 0xff_ffff,
        [255, 0, 0],
    );
    near(
        yuv_convert(&n601, s(145), s(54), s(34)) & 0xff_ffff,
        [0, 255, 0],
    );
    // 10-bit in a 16-bit container: 940 (white) << 6 and 64 (black) << 6
    assert_eq!(
        yuv_convert(&n709, 940 << 6, 512 << 6, 512 << 6) & 0xff_ffff,
        0xff_ffff
    );
    assert_eq!(yuv_convert(&n709, 64 << 6, 512 << 6, 512 << 6) & 0xff_ffff, 0);
}
