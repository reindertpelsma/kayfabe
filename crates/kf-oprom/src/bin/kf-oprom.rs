// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! `kf-oprom` — pack and inspect kf3 boot-display option ROMs from scripts.
//!
//! ```text
//! kf-oprom pack --efi kf-gop.efi --out gop.rom --vendor 0x1234 --device 0x1111 --class 0x030000 \
//!               --bar 0 [--offset 0] --width 1152 --height 648 [--edid edid.bin]
//! kf-oprom parse gop.rom          # every image, then the KFGP descriptor if there is one
//! kf-oprom pe kf-gop.efi          # header fields, then kf3's acceptance check
//! ```
//!
//! The geometry is always the authored one (`Geometry::for_mode`). Output is `key=value` lines so a
//! shell can `grep` them; exit status 0 only when the command did what it says.

use std::process::ExitCode;

use kf_oprom::desc::{BootFramebuffer, Descriptor, Geometry};
use kf_oprom::rom::{self, ClassCode};
use kf_oprom::{Identity, pack, pe};

fn num(s: &str) -> Result<u64, String> {
    let r = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u64::from_str_radix(h, 16),
        None => s.parse(),
    };
    r.map_err(|e| format!("{s}: {e}"))
}

fn arg<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn need<'a>(args: &'a [String], name: &str) -> Result<&'a str, String> {
    arg(args, name).ok_or_else(|| format!("missing {name}"))
}

fn narrow<T: TryFrom<u64>>(v: u64, name: &str) -> Result<T, String> {
    T::try_from(v).map_err(|_| format!("{name} {v:#x} out of range"))
}

fn cmd_pack(args: &[String]) -> Result<(), String> {
    let efi_path = need(args, "--efi")?;
    let out = need(args, "--out")?;
    let pe_bytes = std::fs::read(efi_path).map_err(|e| format!("{efi_path}: {e}"))?;
    let id = Identity {
        vendor: narrow(num(need(args, "--vendor")?)?, "--vendor")?,
        device: narrow(num(need(args, "--device")?)?, "--device")?,
        class: ClassCode::from_u24(narrow(num(need(args, "--class")?)?, "--class")?),
    };
    let geometry = Geometry::for_mode(
        narrow(num(need(args, "--width")?)?, "--width")?,
        narrow(num(need(args, "--height")?)?, "--height")?,
    )
    .map_err(|e| e.to_string())?;
    let fb = BootFramebuffer {
        bar: narrow(num(need(args, "--bar")?)?, "--bar")?,
        offset: arg(args, "--offset").map(num).transpose()?.unwrap_or(0),
        geometry,
    };
    let edid = match arg(args, "--edid") {
        Some(p) => std::fs::read(p).map_err(|e| format!("{p}: {e}"))?,
        None => Vec::new(),
    };
    let rom = pack(&pe_bytes, &id, &fb, &edid).map_err(|e| e.to_string())?;
    std::fs::write(out, &rom).map_err(|e| format!("{out}: {e}"))?;
    println!(
        "rom={out} rom_bytes={} pe_bytes={}",
        rom.len(),
        pe_bytes.len()
    );
    println!(
        "width={} height={} pitch={} fb_size={:#x} bar={} offset={:#x}",
        geometry.width, geometry.height, geometry.pitch, geometry.fb_size, fb.bar, fb.offset
    );
    Ok(())
}

fn cmd_parse(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("usage: kf-oprom parse <rom>")?;
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    println!("rom_bytes={}", bytes.len());
    for (i, img) in rom::images(&bytes).enumerate() {
        let img = img.map_err(|e| e.to_string())?;
        let p = img.pcir;
        println!(
            "image={i} offset={:#x} bytes={:#x} pcir_offset={:#x} vendor={:#06x} device={:#06x} pcir_rev={} pcir_len={:#x} class={:02x}{:02x}{:02x} code_type={} indicator={:#04x}",
            img.offset,
            img.bytes.len(),
            p.offset,
            p.vendor,
            p.device,
            p.revision,
            p.len,
            p.class.base,
            p.class.sub,
            p.class.prog_if,
            p.code_type,
            p.indicator
        );
        if let Some(e) = img.efi {
            println!(
                "image={i} efi_init_blocks={} subsystem={} machine={:#06x} compression={:?} image_header_offset={:#x}",
                e.init_blocks, e.subsystem, e.machine, e.compression, e.image_header_offset
            );
        }
    }
    match Descriptor::find(&bytes) {
        Ok(d) => {
            let g = d.fb.geometry;
            println!(
                "kfgp=1 vendor={:#06x} device={:#06x} bar={} offset={:#x} width={} height={} pitch={} fb_size={:#x} edid_bytes={}",
                d.vendor,
                d.device,
                d.fb.bar,
                d.fb.offset,
                g.width,
                g.height,
                g.pitch,
                g.fb_size,
                d.edid.len()
            );
        }
        Err(e) => println!("kfgp=0 reason=\"{e}\""),
    }
    Ok(())
}

fn cmd_pe(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("usage: kf-oprom pe <efi>")?;
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let info = pe::inspect(&bytes).map_err(|e| e.to_string())?;
    println!(
        "pe={path} file_bytes={} machine={:#06x} subsystem={} size_of_image={:#x} signed={} abi_marker={}",
        bytes.len(),
        info.machine,
        info.subsystem,
        info.size_of_image,
        info.cert_table.is_some(),
        info.has_abi_marker
    );
    pe::check(&bytes).map_err(|e| format!("refused: {e}"))?;
    println!("kf3_accepts=1");
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("pack") => cmd_pack(&args[1..]),
        Some("parse") => cmd_parse(&args[1..]),
        Some("pe") => cmd_pe(&args[1..]),
        _ => Err("usage: kf-oprom pack|parse|pe ... (see the crate's src/bin/kf-oprom.rs)".into()),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("kf-oprom: {e}");
            ExitCode::FAILURE
        }
    }
}
