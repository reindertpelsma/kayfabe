// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! Manual exact-equality gate, never a source of product capabilities.
//! cargo run -p kf-disp --example caps_oracle -- 580.65.06 0x160 0x6 ORACLE.txt
//! Reads a complete sparse native page (WORD rows and '# end words=N'), or the
//! existing two-column AD104 test fixture. Unlisted aligned words mean zero.
//! Exit 0: all 4096 bytes equal. Exit 1: differences. Exit 2: invalid input.

use kf_chip::display::display_for;
use kf_disp::caps::{CapsPage, PAGE, diff_pages};
use kf_disp::capsderive::{Derived, Die, HostFacts, derive, shipped_measured, shipped_rows};
use kf_disp::{class, regs::Regs};
use std::collections::BTreeMap;
use std::io::Read;
use std::process::ExitCode;

const MAX_ORACLE_BYTES: u64 = 256 * 1024;

fn hex(s: &str) -> Result<u32, String> {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|e| e.to_string())
}

fn oracle(text: &str) -> Result<CapsPage, String> {
    let mut words = BTreeMap::new();
    let mut native = false;
    let mut fixture = false;
    let mut end = None;
    for line in text.lines() {
        if let Some(n) = line.strip_prefix("# end words=") {
            if end
                .replace(n.trim().parse::<usize>().map_err(|e| e.to_string())?)
                .is_some()
            {
                return Err("multiple native pages/end markers; select one complete page".into());
            }
        }
        let f: Vec<_> = line.split_whitespace().collect();
        let row = if f.first() == Some(&"WORD") {
            if f.len() != 3 || end.is_some() {
                return Err("invalid/late WORD row".into());
            }
            native = true;
            Some((f[1], f[2]))
        } else if f.first().is_some_and(|s| s.starts_with("0x")) {
            if f.len() != 2 {
                return Err("invalid fixture row".into());
            }
            fixture = true;
            Some((f[0], f[1]))
        } else {
            None
        };
        if let Some((off, value)) = row {
            let (off, value) = (hex(off)?, hex(value)?);
            if off % 4 != 0 || off as usize >= PAGE || words.insert(off, value).is_some() {
                return Err(format!(
                    "duplicate, unaligned or out-of-page offset {off:#x}"
                ));
            }
        }
    }
    if words.is_empty() || (native && fixture) || (native && end != Some(words.len())) {
        return Err("empty, mixed or incomplete oracle (native end count required)".into());
    }
    Ok(CapsPage {
        base: 0x0064_0000,
        words: words.into_iter().collect(),
    })
}

fn differences(t: &class::ClassTable, caps: u32, d: &Derived, native: &CapsPage) -> Vec<String> {
    let fields = diff_pages(t, caps, &d.page, native);
    (0..PAGE as u32).step_by(4).filter_map(|off| {
        let (a, b) = (d.page.word(off), native.word(off));
        if a == b { return None; }
        let description = fields.iter().filter(|f| f.offset == off).map(|f| {
            if f.reg == "?" || f.field == "(undefined bits)" {
                format!("unknown:{off:#05x}:{}.{}", f.reg, f.field)
            } else {
                format!("header:{}[{:?}].{}", f.reg, f.index, f.field)
            }
        }).collect::<Vec<_>>().join(";");
        let basis = d.provenance.iter().find(|p| p.offset == off)
            .map_or("zero: no derivation claims this word", |p| p.why.as_str());
        // A header names a field's encoding, not its hardware value. Neither
        // a difference nor a native value establishes a fix or a cause.
        Some(format!("{off:#05x}\t{a:#010x}\t{b:#010x}\t{description}\t{basis}\tunknown native-value derivation; oracle only"))
    }).collect()
}

fn run(args: &[String]) -> Result<bool, String> {
    if args.len() != 4 {
        return Err("usage: caps_oracle VERSION ARCH_HEX IMPL_HEX ORACLE.txt".into());
    }
    let die = Die {
        architecture: hex(&args[1])?,
        implementation: hex(&args[2])?,
    };
    let row = display_for(die.architecture, die.implementation).ok_or("unknown display family")?;
    let table = class::for_version(&args[0]).ok_or("no exact class table")?;
    let regs = Regs::for_ip(&args[0], row.ip_version).ok_or("no exact register table")?;
    let branch = args[0].split('.').next().ok_or("invalid version")?;
    let derived = derive(
        table,
        &regs,
        row,
        die,
        true,
        shipped_rows(),
        shipped_measured(),
        branch,
        &HostFacts::default(),
    )
    .map_err(|e| format!("derivation refused: {e:?}"))?;
    let mut text = String::new();
    std::fs::File::open(&args[3])
        .map_err(|e| e.to_string())?
        .take(MAX_ORACLE_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() as u64 > MAX_ORACLE_BYTES {
        return Err("oracle exceeds input bound".into());
    }
    let native = oracle(&text)?;
    let rows = differences(table, row.classes.caps, &derived, &native);
    println!(
        "# version={} architecture={:#x} implementation={:#x} caps={:#x} sdr=1 bytes={} differences={} unmeasured={:?}",
        args[0],
        die.architecture,
        die.implementation,
        row.classes.caps,
        PAGE,
        rows.len(),
        derived.unmeasured
    );
    println!("offset\tderived\tnative_oracle\theader_or_unknown\tderived_basis\tnative_basis");
    for line in &rows {
        println!("{line}");
    }
    eprintln!(
        "EXACT_NATIVE_PAGE_EQUAL={} differing_words={}",
        rows.is_empty(),
        rows.len()
    );
    Ok(rows.is_empty())
}

fn main() -> ExitCode {
    match run(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("caps_oracle: {e}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blackwell_tiling_words_use_their_published_class_names() {
        let t = class::for_version("580.65.06").unwrap();
        let native = oracle("0x020 0x00000001\n").unwrap();
        let d = Derived {
            page: CapsPage {
                base: native.base,
                words: vec![],
            },
            provenance: vec![],
            unmeasured: vec![],
        };
        let diff = differences(t, 0xCA73, &d, &native);
        assert_eq!(diff.len(), 1);
        assert!(diff[0].contains("header:SYS_CAPC"), "{}", diff[0]);
        assert!(!diff[0].contains("unknown:0x020"));
    }
    #[test]
    fn sparse_native_page_requires_its_complete_end_count() {
        assert_eq!(
            oracle("WORD 0x004 0x12\n# end words=1\n").unwrap().word(0),
            0
        );
        for bad in [
            "",
            "WORD 0x004 0x12\n",
            "WORD 0x004 0x12\n# end words=2\n",
            "WORD 0x004 0x12\nWORD 0x004 0x12\n# end words=2\n",
            "WORD 0x001 0x12\n# end words=1\n",
            "WORD 0x1000 0x12\n# end words=1\n",
            "WORD 0x004 0x12\n# end words=1\nWORD 0x008 0x1\n",
            "WORD 0x004 0x12\n0x008 0x1\n# end words=2\n",
        ] {
            assert!(oracle(bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn checks_zero_words_and_reports_unpublished_cells_without_deriving_them() {
        let t = class::for_version("580.65.06").unwrap();
        let p = oracle("0x008 0x123\n").unwrap();
        let d = Derived {
            page: p.clone(),
            provenance: vec![],
            unmeasured: vec![],
        };
        assert!(differences(t, 0xC773, &d, &p).is_empty());
        let n = oracle("0x00c 0x456\n").unwrap();
        let diff = differences(t, 0xC773, &d, &n);
        assert_eq!(diff.len(), 2);
        assert!(diff[0].contains("unknown:0x008"));
        assert!(diff[1].contains("unknown:0x00c"));
        assert_eq!(d.page, p, "comparison must never patch the derived page");
    }
}
