//! HARDWARE-GATED (needs a CUDA GPU; not run by `cargo test`): PR #92's output kernel ON A GPU,
//! over the same hue / vibrance / boundary configuration table and table shapes the GPU-free tests
//! use (`kf-disp/tests/color_hue_vibrance.rs`, `color_output_ptx.rs`), on synthetic frames only:
//! host-authored bytes, no guest memory, no host display settings.
//!
//! `cargo run --release -p kf-cuda --example color_hue_vibrance -- <host PCI BDF> [--out FILE.json] [--quick]`
//!
//! Every GPU byte is compared with the CPU oracle (`kf_disp::color::model`, the kernel's own `f32`
//! arithmetic: must be EXACT) and with the independent `f64` reference
//! (`kf-disp/tests/color_support/reference.rs`: must fall in its rounding interval). The frame
//! is B,G,R,A in memory (`kf_color_output` packs `q << (16 - 8c)`).
//!
//! Output: one `COLOR_HV_GPU <case> PASS|FAIL` line per case, `COLOR_HV_GPU_SUMMARY ...`, and with
//! `--out` a JSON verdict `{"verdict": "PASS"|"FAIL", ...}`. Exit 0 only when every case passed.
//!
//! What a PASS proves: the PTX as JITed and run by this GPU and driver computes what the oracle
//! and reference say, for these inputs. Not: what the display engine's MIRROR does (odd vs even,
//! `V3_SDR_COLOR.md` falsifier), nor what the Windows driver programs (the Windows lane).
#[path = "../../kf-disp/tests/color_support/reference.rs"]
mod reference;

use kf_cuda::display::{ColorFixture, ComposeLayer, DisplayGpu};
use kf_disp::color::model;
use reference::RefTable;

/// A snapshot: four 8-byte header entries, then `n` RGB+padding entries.
fn snapshot(entries: &[[u16; 3]]) -> Vec<u8> {
    let mut bytes = vec![0xa5; 32];
    for e in entries {
        for v in e.iter().chain([&0]) {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    bytes
}

/// FP16 bits of a value that FP16 holds exactly (k/256 or 1.0 here).
fn f16_exact(x: f32) -> u16 {
    if x == 0.0 {
        return 0;
    }
    let b = x.to_bits();
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    assert!(
        (1..31).contains(&exp) && b & 0x1fff == 0,
        "{x} is not an exact normal FP16"
    );
    ((exp as u16) << 10) | ((b >> 13) & 0x3ff) as u16
}

/// The source value the input table gives to component byte `k`.
fn level(k: u8) -> f32 {
    if k == 255 { 1.0 } else { f32::from(k) / 256.0 }
}

/// The FP16 input table: byte `k` indexes entry `4k` (`kf_color_compose`, DIRECT10).
fn input_table() -> Vec<u8> {
    let mut e = vec![[0_u16; 3]; 1025];
    for k in 0..=255_u8 {
        e[usize::from(k) * 4] = [f16_exact(level(k)); 3];
    }
    snapshot(&e)
}

struct Outcome {
    name: String,
    pass: bool,
    detail: String,
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bdf = args
        .first()
        .ok_or("usage: color_hue_vibrance <host PCI BDF> [--out FILE.json] [--quick]")?
        .clone();
    let out = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|i| args.get(i + 1).cloned());
    let quick = args.iter().any(|a| a == "--quick");
    let mut gpu = DisplayGpu::bring_up_on(&bdf).map_err(|e| e.to_string())?;

    // a 9^3 colour grid, one pixel each, one row of 729 (> 256: the stride loop runs)
    let bytes = [0_u8, 32, 64, 96, 128, 160, 192, 224, 255];
    let mut pixels = Vec::new();
    for &r in &bytes {
        for &g in &bytes {
            for &b in &bytes {
                pixels.push([r, g, b]);
            }
        }
    }
    let w = pixels.len() as u32;
    let surface: Vec<u8> = pixels
        .iter()
        .flat_map(|p| [p[2], p[1], p[0], 0xff])
        .collect();
    let layer = ComposeLayer {
        src: 0,
        extent: surface.len() as u64,
        block_linear: false,
        pitch: w * 4,
        block_height_log2: 0,
        x0_bytes: 0,
        y0: 0,
        width: w,
        rows: 1,
        ox: 0,
        oy: 0,
        flags: 4,
        a_s: 255,
        b_s: 0,
        a_d: 0,
        b_d: 0,
    };
    let input = input_table();

    let tables = reference::tables();
    let pick = |name: &str, n: usize| {
        tables
            .iter()
            .find(|t| t.name == name && t.entries.len() == n)
            .expect("table")
    };
    let chosen = [
        pick("lifted black 0.25+0.5x", 1025),
        pick("staircase", 1025),
        pick("per-channel", 257),
        pick("identity", 257),
    ];
    let mut configs = reference::configurations();
    if quick {
        configs.retain(|c| {
            !c.name.starts_with("hue") || c.name.ends_with(" 0") || c.name.contains(" 90")
        });
    }

    let mut results = Vec::new();
    let mut pixels_checked = 0_u64;
    for cfg in &configs {
        for nt in chosen {
            for (interpolate, mirror) in [(false, true), (true, true), (true, false)] {
                let name = format!(
                    "{} | {} x{} interp={interpolate} mirror={mirror}",
                    cfg.name,
                    nt.name,
                    nt.entries.len()
                );
                let output = snapshot(&nt.entries);
                let fixture = ColorFixture {
                    surface: &surface,
                    input: &input,
                    output: &output,
                    layer: &layer,
                    matrix: &cfg.matrix,
                    size: (w, 1),
                    interpolate,
                    output_mirror: mirror,
                    mutate_input: None,
                    rearm: false,
                    tone: None,
                };
                let got = gpu.selftest_color(&fixture).map_err(|e| e.to_string());
                let mt = model::Table {
                    entries: &nt.entries,
                    interpolate,
                    mirror,
                };
                let rt = RefTable {
                    entries: &nt.entries,
                    interpolate,
                    mirror,
                };
                let outcome = match got {
                    Err(e) => Outcome {
                        name,
                        pass: false,
                        detail: format!("error={e}"),
                    },
                    Ok(frame) if frame.len() != pixels.len() * 4 => Outcome {
                        name,
                        pass: false,
                        detail: format!("readback {} bytes", frame.len()),
                    },
                    Ok(frame) => {
                        let mut bad = Vec::new();
                        for (i, p) in pixels.iter().enumerate() {
                            let src = p.map(level);
                            let px = &frame[i * 4..i * 4 + 4];
                            let rgb = [px[2], px[1], px[0]];
                            let want = model::output_rgb(src, &cfg.matrix, Some(&mt));
                            let span =
                                reference::output(src.map(f64::from), &cfg.matrix, Some(&rt));
                            if rgb != want
                                || px[3] != 255
                                || (0..3).any(|c| !span[c].admits(rgb[c]))
                            {
                                bad.push(format!(
                                    "px{i} src={p:?} gpu_rgb={rgb:?} a={} oracle={want:?} ref={span:?}",
                                    px[3]
                                ));
                            }
                        }
                        pixels_checked += pixels.len() as u64;
                        Outcome {
                            name,
                            pass: bad.is_empty(),
                            detail: if bad.is_empty() {
                                format!("pixels={}", pixels.len())
                            } else {
                                format!("mismatches={} first={}", bad.len(), bad[0])
                            },
                        }
                    }
                };
                println!(
                    "COLOR_HV_GPU {} {} {}",
                    outcome.name,
                    if outcome.pass { "PASS" } else { "FAIL" },
                    outcome.detail
                );
                results.push(outcome);
            }
        }
    }
    let failed = results.iter().filter(|r| !r.pass).count();
    let summary = format!(
        "COLOR_HV_GPU_SUMMARY cases={} pass={} fail={failed} pixels={pixels_checked} bdf={bdf} quick={quick}",
        results.len(),
        results.len() - failed
    );
    println!("{summary}");
    if let Some(path) = out {
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let fails: Vec<String> = results
            .iter()
            .filter(|r| !r.pass)
            .take(20)
            .map(|r| {
                format!(
                    "{{\"case\":\"{}\",\"detail\":\"{}\"}}",
                    esc(&r.name),
                    esc(&r.detail)
                )
            })
            .collect();
        let json = format!(
            "{{\"lane\":\"color_hue_vibrance_gpu\",\"classification\":\"MEASURED\",\"verdict\":\"{}\",\"cases\":{},\"failed\":{failed},\"pixels\":{pixels_checked},\"bdf\":\"{}\",\"quick\":{quick},\"first_failures\":[{}]}}\n",
            if failed == 0 { "PASS" } else { "FAIL" },
            results.len(),
            esc(&bdf),
            fails.join(",")
        );
        std::fs::write(&path, json).map_err(|e| format!("{path}: {e}"))?;
    }
    if failed > 0 {
        return Err(format!("{failed} of {} cases failed", results.len()));
    }
    Ok(())
}
