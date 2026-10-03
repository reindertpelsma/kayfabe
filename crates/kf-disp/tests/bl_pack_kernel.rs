// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ The GPU-copy rung's pack kernel (`cuda/display/kf_bl_pack.cu`) RUN ON THE HOST: the same source
//! the NVPTX back-end compiles (`make_pack_ptx.sh`), built here as plain C with `-DKF_HOST` by the
//! system C compiler, driven over every (CTA, thread) the device launch would run, and compared
//! byte for byte with `kf_disp::vramslot::pack_reference` — for block heights 0..=5, odd sizes, a
//! frame whose right edge cuts a 16-byte chunk, and a permuted GOB layout. It is the kernel's
//! logic, not the GPU: the box self-test (`selftest_bl_pack`) compares the real launch with the
//! same reference before any slot is exported.

use kf_disp::vramslot::{GOB_GA106, GobLayout, pack_reference, slot_geom};
use std::path::{Path, PathBuf};
use std::process::Command;

fn kernel_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cuda/display/kf_bl_pack.cu")
}

/// The harness: reads `w h gpr bh gobs y0 y1 x4 y2 x5` from argv, the staging frame from
/// `argv[11]`, writes the slot to `argv[12]` — every (block, thread) of the launch, in order.
const HARNESS: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#define KF_HOST 1
#include KF_SOURCE
int main(int argc, char **argv) {
    if (argc != 13) return 2;
    unsigned v[10];
    for (int i = 0; i < 10; i++) v[i] = (unsigned)strtoul(argv[i + 1], 0, 10);
    unsigned w = v[0], h = v[1], gpr = v[2], bh = v[3];
    unsigned long long gobs = v[4];
    FILE *f = fopen(argv[11], "rb");
    if (!f) return 3;
    unsigned int *src = malloc((size_t)w * h * 4);
    if (fread(src, 4, (size_t)w * h, f) != (size_t)w * h) return 4;
    fclose(f);
    kf_u4 *dst = malloc(gobs * 512);
    memset(dst, 0xA5, gobs * 512); /* never-written bytes show */
    unsigned blocks = (unsigned)((gobs + 7) / 8);
    for (unsigned b = 0; b < blocks; b++)
        for (unsigned t = 0; t < KF_PACK_THREADS; t++)
            kf_bl_pack_chunk(src, dst, w, h, gpr, bh, gobs, v[5], v[6], v[7], v[8], v[9], b, t);
    f = fopen(argv[12], "wb");
    if (!f || fwrite(dst, 512, gobs, f) != gobs) return 5;
    fclose(f);
    return 0;
}
"#;

fn build(dir: &Path) -> PathBuf {
    let c = dir.join("harness.c");
    std::fs::write(&c, HARNESS).unwrap();
    let exe = dir.join("harness");
    let src = kernel_source();
    let out = Command::new(std::env::var("CC").unwrap_or_else(|_| "cc".into()))
        .args(["-std=c99", "-O1", "-Wall", "-Werror", "-o"])
        .arg(&exe)
        .arg(format!("-DKF_SOURCE=\"{}\"", src.display()))
        .arg("-x")
        .arg("c")
        .arg(&c)
        .output()
        .expect("the system C compiler (proto_mirror.rs needs it too)");
    assert!(
        out.status.success(),
        "the kernel source does not build as C: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    exe
}

fn run(exe: &Path, dir: &Path, w: u32, h: u32, bh: u32, layout: &GobLayout, case: &str) {
    let g = slot_geom(w, h, bh).unwrap();
    // a pattern that never repeats inside a row or between rows, with no zero byte
    let staging: Vec<u8> = (0..u64::from(w) * u64::from(h) * 4)
        .map(|i| ((i.wrapping_mul(2_654_435_761) >> 7) as u8) | 1)
        .collect();
    let sin = dir.join(format!("{case}.in"));
    let sout = dir.join(format!("{case}.out"));
    std::fs::write(&sin, &staging).unwrap();
    let st = Command::new(exe)
        .args(
            [
                w,
                h,
                g.gobs_per_row,
                bh,
                u32::try_from(g.gobs()).unwrap(),
                layout.y0,
                layout.y1,
                layout.x4,
                layout.y2,
                layout.x5,
            ]
            .map(|v| v.to_string()),
        )
        .arg(&sin)
        .arg(&sout)
        .status()
        .unwrap();
    assert!(st.success(), "{case}: the harness failed ({st})");
    let got = std::fs::read(&sout).unwrap();
    let want = pack_reference(layout, &staging, &g).unwrap();
    assert_eq!(got.len(), want.len(), "{case}: extent");
    if let Some(i) = (0..got.len()).find(|&i| got[i] != want[i]) {
        panic!(
            "{case}: slot byte {i} (GOB {}, chunk {}) is {:#04x}, the reference says {:#04x}",
            i / 512,
            (i % 512) / 16,
            got[i],
            want[i]
        );
    }
}

/// ★ The kernel equals the reference for every block height and odd sizes, the GA106 layout and a
/// permuted one — and every byte of the extent is written (the harness pre-fills `0xA5`, which the
/// reference never has in the padding).
#[test]
fn the_pack_kernel_writes_the_reference_slot() {
    let dir = std::env::temp_dir().join(format!("kf-blpack-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = build(&dir);
    for bh in 0..=5 {
        run(&exe, &dir, 70, 40, bh, &GOB_GA106, &format!("70x40-h{bh}"));
    }
    for (w, h) in [(1, 1), (17, 9), (3, 130), (129, 257)] {
        run(&exe, &dir, w, h, 4, &GOB_GA106, &format!("{w}x{h}"));
    }
    run(&exe, &dir, 1366, 768, 4, &GOB_GA106, "1366x768");
    let permuted = GobLayout {
        y0: 4,
        y1: 6,
        x4: 5,
        y2: 7,
        x5: 8,
    };
    run(&exe, &dir, 70, 40, 4, &permuted, "permuted");
    let _ = std::fs::remove_dir_all(&dir);
}

/// ⊘ The known-positive: the harness CAN fail — the reference with a different layout than the
/// kernel was given disagrees, so a green above is not a comparison that cannot see.
#[test]
fn the_comparison_catches_a_wrong_layout() {
    let g = slot_geom(70, 40, 4).unwrap();
    let staging: Vec<u8> = (0..70 * 40 * 4).map(|i| (i % 253) as u8 | 1).collect();
    let a = pack_reference(&GOB_GA106, &staging, &g).unwrap();
    let swapped = GobLayout {
        x4: 5,
        y1: 6,
        ..GOB_GA106
    };
    let b = pack_reference(&swapped, &staging, &g).unwrap();
    assert_ne!(a, b, "the Tegra-order swap moves 16-byte chunks");
}
