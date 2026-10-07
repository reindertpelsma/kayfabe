//! GPU tests on synthetic LUT fixtures. No host display settings or guest memory.
use kf_cuda::display::{ColorFixture, ComposeLayer, DisplayGpu};

fn table(rgb: [u16; 3]) -> Vec<u8> {
    let mut bytes = vec![0xa5; 32];
    for _ in 0..1025 {
        for v in rgb.into_iter().chain([0]) {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    bytes
}

fn main() -> Result<(), String> {
    let bdf = std::env::args()
        .nth(1)
        .ok_or("usage: color_sdr <host PCI BDF>")?;
    let mut gpu = DisplayGpu::bring_up_on(&bdf).map_err(|e| e.to_string())?;
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
    let fixture = ColorFixture {
        surface: &surface,
        input: &input,
        output: &identity,
        layer: &l,
        matrix: &matrix,
        size: (8, 1),
        interpolate: false,
        mutate_input: None,
        rearm: false,
        tone: None,
    };
    let pixels = gpu.selftest_color(&fixture).map_err(|e| e.to_string())?;
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [0, 255, 128, 255])
    );
    println!("COLOR_GPU input_fp16_nonidentity PASS pixels=8 expected_rgb=128,255,0");
    let mut compact_input = vec![0_u8; 261 * 8];
    for (c, value) in [0x3800_u16, 0x3400, 0x3a00].into_iter().enumerate() {
        compact_input[(4 + 255) * 8 + c * 2..(4 + 255) * 8 + c * 2 + 2]
            .copy_from_slice(&value.to_le_bytes());
    }
    let pixels = gpu
        .selftest_color(&ColorFixture {
            input: &compact_input,
            ..fixture
        })
        .map_err(|e| e.to_string())?;
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [192, 64, 128, 255]),
        "DIRECT8 input index255: {pixels:?}"
    );
    println!("COLOR_GPU direct8_input PASS exact 2088-byte table, white selects index255");
    let mut compact_output = vec![0_u8; 32];
    for i in 0..257_u16 {
        for v in [i.min(255) << 8; 3].into_iter().chain([0]) {
            compact_output.extend_from_slice(&v.to_le_bytes());
        }
    }
    let pixels = gpu
        .selftest_color(&ColorFixture {
            input: &compact_input,
            output: &compact_output,
            ..fixture
        })
        .map_err(|e| e.to_string())?;
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [192, 64, 128, 255]),
        "DIRECT8 output indices: {pixels:?}"
    );
    println!("COLOR_GPU direct8_output PASS compact ramp indexes 128,64,192");
    let output = table([0, 32768, 65535]);
    let pixels = gpu
        .selftest_color(&ColorFixture {
            output: &output,
            ..fixture
        })
        .map_err(|e| e.to_string())?;
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [255, 128, 0, 255])
    );
    println!("COLOR_GPU output_unorm_nonidentity PASS pixels=8 expected_rgb=0,128,255");
    let alternate = table([0, 0x3800, 0x3c00]);
    let mutated = ColorFixture {
        mutate_input: Some(&alternate),
        ..fixture
    };
    let pixels = gpu.selftest_color(&mutated).map_err(|e| e.to_string())?;
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [0, 255, 128, 255])
    );
    println!("COLOR_GPU immutable_snapshot PASS mutable source changed, armed snapshot retained");
    let pixels = gpu
        .selftest_color(&ColorFixture {
            rearm: true,
            ..mutated
        })
        .map_err(|e| e.to_string())?;
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [255, 128, 0, 255])
    );
    println!("COLOR_GPU rearmed_snapshot PASS new token consumes changed table");
    let mut fractional = identity.clone();
    fractional[(4 + 512) * 8..(4 + 512) * 8 + 2].copy_from_slice(&0u16.to_le_bytes());
    fractional[(4 + 513) * 8..(4 + 513) * 8 + 2].copy_from_slice(&65535u16.to_le_bytes());
    let mut biased = matrix;
    biased[3] = 32; // exactly half an OLUT index, after the FP16 input stage
    let pixels = gpu
        .selftest_color(&ColorFixture {
            output: &fractional,
            matrix: &biased,
            interpolate: true,
            ..fixture
        })
        .map_err(|e| e.to_string())?;
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [0, 255, 127, 255])
    );
    println!("COLOR_GPU fractional_olut_interpolation PASS rgb=127,255,0 at index 512.5");
    for invalid in [0x7c00, 0x7e00, 0xbc00, 0x4000] {
        let input = table([invalid, 0, 0]);
        let error = gpu
            .selftest_color(&ColorFixture {
                input: &input,
                ..fixture
            })
            .unwrap_err();
        assert!(
            error.to_string().contains("non-SDR FP16"),
            "wrong refusal: {error}"
        );
    }
    println!("COLOR_GPU invalid_fp16 PASS infinity,nan,negative,hdr refused by GPU validation");
    Ok(())
}
