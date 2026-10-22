//! HARDWARE-GATED (needs a CUDA GPU; not run in CI, not run on the author's box): the output
//! kernel's mirrored-table path against the CPU oracle's hand-computed pixels
//! (`kf_disp::color::model`, tests `a_hand_computed_negative_channel` and
//! `digital_vibrance_on_a_pure_primary_blackens_the_negative_channels`). Synthetic fixtures only:
//! no host display settings, no guest memory.
//!
//! `cargo run -p kf-cuda --example color_mirror -- <host PCI BDF>`
//!
//! This checks the KERNEL against the reading "MIRROR = odd extension". It cannot tell whether that
//! reading is what the display engine does; that needs the real engine (a VFIO or bare-metal run
//! with a signed OCSC0 and a mirrored OLUT, scanout captured), see the PR's validation plan.
use kf_cuda::display::{ColorFixture, ComposeLayer, DisplayGpu};

/// A header-less-test table: four 8-byte header entries, then `n` RGB+padding entries.
fn table(n: usize, f: impl Fn(usize) -> [u16; 3]) -> Vec<u8> {
    let mut bytes = vec![0xa5; 32];
    for i in 0..n {
        for v in f(i).into_iter().chain([0]) {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    bytes
}

/// The encoded OCSC0 of saturation `s` about Rec.709 luma, without a bias.
fn saturation(s: f64) -> [i32; 12] {
    let luma = [0.2126, 0.7152, 0.0722];
    let mut m = [0; 12];
    for r in 0..3 {
        for c in 0..3 {
            let v = (1.0 - s) * luma[c] + if r == c { s } else { 0.0 };
            m[r * 4 + c] = (v * 65536.0).round() as i32;
        }
    }
    m
}

fn main() -> Result<(), String> {
    let bdf = std::env::args()
        .nth(1)
        .ok_or("usage: color_mirror <host PCI BDF>")?;
    let mut gpu = DisplayGpu::bring_up_on(&bdf).map_err(|e| e.to_string())?;
    let layer = ComposeLayer {
        src: 0,
        extent: 32,
        block_linear: false,
        pitch: 32,
        block_height_log2: 0,
        x0_bytes: 0,
        y0: 0,
        width: 8,
        rows: 1,
        ox: 0,
        oy: 0,
        flags: 4,
        a_s: 255,
        b_s: 0,
        a_d: 0,
        b_d: 0,
    };
    let surface = [0xffu8; 32];
    // a lifted black, as a brightness slider makes it: lut(x) = 0.25 + 0.5 x, UNORM16
    let lifted = table(1025, |i| [(16384 + i * 32) as u16; 3]);
    let run = |gpu: &mut DisplayGpu,
               input_rgb: [u16; 3],
               matrix: &[i32; 12],
               mirror: bool,
               interpolate: bool| {
        let input = table(1025, |_| input_rgb);
        let fixture = ColorFixture {
            surface: &surface,
            input: &input,
            output: &lifted,
            layer: &layer,
            matrix,
            size: (8, 1),
            interpolate,
            output_mirror: mirror,
            mutate_input: None,
            rearm: false,
            tone: None,
        };
        gpu.selftest_color(&fixture).map_err(|e| e.to_string())
    };
    let all = |pixels: &[u8], rgb: [u8; 3]| {
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[..3] == rgb && p[3] == 255)
    };

    // src (0.25, 0.75, 0) -> R' = R - G / 2 = -0.125, G' = 0.75, B' = 0
    let mut m = [65536, 0, 0, 0, 0, 65536, 0, 0, 0, 0, 65536, 0];
    m[1] = -32768;
    let (quarter, three_quarters) = (0x3400, 0x3a00); // FP16
    let px = run(&mut gpu, [quarter, three_quarters, 0], &m, true, false)?;
    assert!(all(&px, [0, 160, 64]), "mirrored: {px:?}");
    println!("COLOR_GPU mirror_negative_channel PASS rgb=0,160,64 (odd extension, clamped)");
    let px = run(&mut gpu, [quarter, three_quarters, 0], &m, false, false)?;
    assert!(all(&px, [64, 160, 64]), "unmirrored: {px:?}");
    println!("COLOR_GPU unmirrored_negative_channel PASS rgb=64,160,64 (first sample)");

    // pure red at saturation 1.5: R' = 1.3937 (clamps to the last sample), G' = B' = -0.1063
    let m = saturation(1.5);
    let px = run(&mut gpu, [0x3c00, 0, 0], &m, true, true)?;
    assert!(all(&px, [192, 0, 0]), "vibrance mirrored: {px:?}");
    println!("COLOR_GPU mirror_digital_vibrance PASS rgb=192,0,0");
    let px = run(&mut gpu, [0x3c00, 0, 0], &m, false, true)?;
    assert!(all(&px, [192, 64, 64]), "vibrance unmirrored: {px:?}");
    println!("COLOR_GPU unmirrored_digital_vibrance PASS rgb=192,64,64");
    Ok(())
}
