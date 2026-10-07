//! Synthetic GPU tone-stage oracle. No host display settings or guest memory.
use kf_cuda::display::{ColorFixture, ColorPipeline, ColorTone, ComposeLayer, DisplayGpu};
include!("tmo_fixture_data.inc");

fn table(value: u16) -> Vec<u8> {
    let mut bytes = Vec::new();
    let header = (0..16).fold(0_u64, |v, i| v | (4 << (i * 3)));
    for _ in 0..4 {
        bytes.extend_from_slice(&header.to_le_bytes());
    }
    for _ in 0..1025 {
        for v in [value, value, value, 0] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    bytes
}
fn identity() -> Vec<u8> {
    let mut bytes = vec![0; 32];
    for i in 0..1025_u16 {
        for v in [i.min(1023) << 6; 3].into_iter().chain([0]) {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    bytes
}
fn main() -> Result<(), String> {
    let bdf = std::env::args()
        .nth(1)
        .ok_or("usage: color_tmo <host PCI BDF>")?;
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
    let unit = [65536, 0, 0, 0, 0, 65536, 0, 0, 0, 0, 65536, 0];
    let bypass = ColorPipeline {
        matrices: [unit; 4],
        segments: [[0; 64]; 2],
        entries: [[0; 1025]; 2],
        enable: [false; 2],
    };
    let mut pq = bypass.clone();
    pq.matrices = [REC709RGBTOLMS, LMSTOICTCP, ICTCPTOLMS, LMSTOREC709RGB];
    pq.segments[0][..33].copy_from_slice(&OETF_P_Q512_SEG_SIZES_LOG2);
    pq.segments[1].copy_from_slice(&EOTF_P_Q512_SEG_SIZES_LOG2);
    pq.entries[0][..OETF_P_Q512_ENTRIES.len()].copy_from_slice(&OETF_P_Q512_ENTRIES);
    pq.entries[1][..EOTF_P_Q512_ENTRIES.len()].copy_from_slice(&EOTF_P_Q512_ENTRIES);
    pq.enable = [true; 2];
    let surface = [255_u8; 32];
    let input = table(0x3800); // FP16 0.5 on every component.
    let output = identity();
    let zero = table(0);
    let white = table(65532);
    let fixture = ColorFixture {
        surface: &surface,
        input: &input,
        output: &output,
        layer: &layer,
        matrix: &unit,
        size: (8, 1),
        interpolate: true,
        mutate_input: None,
        rearm: false,
        tone: Some(ColorTone {
            table: &zero,
            pipeline: &bypass,
            mutate: None,
            rearm: false,
        }),
    };
    let run = |gpu: &mut DisplayGpu, f: &ColorFixture<'_>| {
        gpu.selftest_color(f).map_err(|e| e.to_string())
    };
    let pixels = run(&mut gpu, &fixture)?;
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [128, 0, 128, 255])
    );
    println!("TMO_GPU intensity_only PASS Ct/I/Cp=0.5,0,0.5; not RGB gamma");
    let tone = ColorTone {
        table: &zero,
        pipeline: &pq,
        mutate: None,
        rearm: false,
    };
    let pq_fixture = ColorFixture {
        tone: Some(tone),
        ..fixture
    };
    let pixels = run(&mut gpu, &pq_fixture)?;
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [0, 0, 0, 255]),
        "zero intensity pixels: {pixels:?}"
    );
    println!("TMO_GPU pq_pipeline_zero PASS full OGKM CSC/PQ program rgb=0,0,0");
    let pixels = run(
        &mut gpu,
        &ColorFixture {
            tone: Some(ColorTone {
                mutate: Some(&white),
                ..tone
            }),
            ..pq_fixture
        },
    )?;
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [0, 0, 0, 255])
    );
    println!("TMO_GPU immutable_snapshot PASS modified source retains armed zero curve");
    let pixels = run(
        &mut gpu,
        &ColorFixture {
            tone: Some(ColorTone {
                mutate: Some(&white),
                rearm: true,
                ..tone
            }),
            ..pq_fixture
        },
    )?;
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [255, 255, 255, 255])
    );
    println!("TMO_GPU rearm_snapshot PASS fresh token consumes white curve");
    let mut segmented = zero.clone();
    let mut header = u64::from_le_bytes(segmented[..8].try_into().unwrap());
    header = (header & !0x1ff) | 3 | (3 << 3) | (5 << 6);
    segmented[..8].copy_from_slice(&header.to_le_bytes());
    let tiny_input = table(0x1c00); // exact FP16 1/256: first zone, fraction 1/4.
    segmented[(4 + 2) * 8..(4 + 2) * 8 + 6].copy_from_slice(&[0, 64, 0, 64, 0, 64]);
    segmented[(4 + 4) * 8..(4 + 4) * 8 + 6].copy_from_slice(&[0, 192, 0, 192, 0, 192]);
    let pixels = run(
        &mut gpu,
        &ColorFixture {
            input: &tiny_input,
            tone: Some(ColorTone {
                table: &segmented,
                pipeline: &bypass,
                ..tone
            }),
            ..fixture
        },
    )?;
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [1, 64, 1, 255]),
        "variable segment pixels: {pixels:?}"
    );
    println!(
        "TMO_GPU variable_segment_header PASS nonuniform first zone samples index2, not fixed index4"
    );
    // 64 zones with one sample each: exact minimum table extent, including endpoint.
    let mut compact = vec![0_u8; 69 * 8];
    for entry in compact[32..].as_chunks_mut::<8>().0 {
        entry[..6].copy_from_slice(&[0, 64, 0, 64, 0, 64]);
    }
    let pixels = run(
        &mut gpu,
        &ColorFixture {
            tone: Some(ColorTone {
                table: &compact,
                pipeline: &bypass,
                ..tone
            }),
            ..fixture
        },
    )?;
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [128, 64, 128, 255]),
        "compact tone pixels: {pixels:?}"
    );
    println!("TMO_GPU compact_table PASS 65 sample entries, exact 552-byte source extent");
    let mut invalid_compact = compact.clone();
    invalid_compact[0] = 7; // Header demands more samples than the declared source extent.
    let error = run(
        &mut gpu,
        &ColorFixture {
            tone: Some(ColorTone {
                table: &invalid_compact,
                pipeline: &bypass,
                ..tone
            }),
            ..fixture
        },
    )
    .unwrap_err();
    assert!(error.contains("TMO LUT"), "wrong compact refusal: {error}");
    println!("TMO_GPU compact_bounds PASS hostile header refuses within authored extent");
    for bad in [0, 1, 2] {
        let mut invalid = zero.clone();
        if bad == 0 {
            invalid[0] = 0;
        } else if bad == 1 {
            invalid[32] = 1;
        } else {
            invalid[..32].fill(255);
        }
        let error = run(
            &mut gpu,
            &ColorFixture {
                tone: Some(ColorTone {
                    table: &invalid,
                    ..tone
                }),
                ..pq_fixture
            },
        )
        .unwrap_err();
        assert!(error.contains("TMO LUT"), "wrong refusal: {error}");
    }
    println!("TMO_GPU invalid_table PASS unsupported VSS and unequal intensity channels refuse");
    Ok(())
}
