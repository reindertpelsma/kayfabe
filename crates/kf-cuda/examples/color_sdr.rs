//! GPU tests on synthetic LUT fixtures. No host display settings or guest memory.
use kf_cuda::display::{ComposeLayer, DisplayGpu};

fn table(rgb: [u16; 3]) -> Vec<u8> {
    let mut bytes = vec![0xa5; 32];
    for _ in 0..1025 {
        for v in rgb.into_iter().chain([0]) {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    bytes
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bdf = std::env::args()
        .nth(1)
        .ok_or("usage: color_sdr <host PCI BDF>")?;
    let mut gpu = DisplayGpu::bring_up_on(&bdf)?;
    let l = ComposeLayer {
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
    let matrix = [65536, 0, 0, 0, 0, 65536, 0, 0, 0, 0, 65536, 0];
    let surface = [0xffu8; 32];
    let input = table([0x3800, 0x3c00, 0]); // FP16 [0.5, 1.0, 0.0]
    let mut identity = vec![0xa5; 32];
    for i in 0..1025u16 {
        for v in [i.min(1023) << 6; 3].into_iter().chain([0]) {
            identity.extend_from_slice(&v.to_le_bytes());
        }
    }
    let pixels = gpu.selftest_color(&surface, &input, &identity, &l, &matrix, 8, 1)?;
    assert!(pixels.chunks_exact(4).all(|p| p == [0, 255, 128, 255]));
    println!("COLOR_GPU input_fp16_nonidentity PASS pixels=8 expected_rgb=128,255,0");
    let output = table([0, 32768, 65535]);
    let pixels = gpu.selftest_color(&surface, &input, &output, &l, &matrix, 8, 1)?;
    assert!(pixels.chunks_exact(4).all(|p| p == [255, 128, 0, 255]));
    println!("COLOR_GPU output_unorm_nonidentity PASS pixels=8 expected_rgb=0,128,255");
    for invalid in [0x7c00, 0x7e00, 0xbc00, 0x4000] {
        let input = table([invalid, 0, 0]);
        assert!(
            gpu.selftest_color(&surface, &input, &identity, &l, &matrix, 8, 1)
                .is_err()
        );
    }
    println!("COLOR_GPU invalid_fp16 PASS infinity,nan,negative,hdr refused");
    Ok(())
}
