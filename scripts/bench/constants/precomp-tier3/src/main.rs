//! Privileged read-only PRECOMP sweep. No guest, submissions, writes or arbitrary offsets.
use kf_arch::ids::GpuId;
use kf_disp::class::{ClassTable, Row};
use kf_linux_raw::{DevDir, capability};
use std::fmt::Write;

fn tag(die: &str, driver: &str, date: &str) -> Result<String, String> {
    if !matches!(die, "GB202" | "GB203" | "GB205" | "GB206" | "GB207")
        || !driver.bytes().all(|c| c.is_ascii_digit() || c == b'.')
        || driver.is_empty()
        || date.len() != 10
        || date.bytes().enumerate().any(|(i, c)| {
            if i == 4 || i == 7 {
                c != b'-'
            } else {
                !c.is_ascii_digit()
            }
        })
    {
        return Err("invalid measurement provenance".into());
    }
    Ok(format!("measured:{die},{driver},{date}"))
}

fn rows(
    provenance: &str,
    table: &ClassTable,
    class: u32,
    windows: u32,
    addresses: &[u32],
    values: &[u32],
) -> Result<String, String> {
    if windows == 0
        || windows > 32
        || addresses.len() != windows as usize * 6
        || values.len() != addresses.len()
        || values.iter().all(|v| *v == 0)
        || values.iter().all(|v| *v == u32::MAX)
    {
        return Err("incomplete or inaccessible measurement".into());
    }
    let mut text = String::new();
    for (i, (address, value)) in addresses.iter().zip(values).enumerate() {
        let kind = ['A', 'B', 'C', 'D', 'E', 'F'][i % 6];
        let prefix = format!("PRECOMP_WIN_PIPE_HDR_CAP{kind}_");
        writeln!(text, "{provenance}\tword\twindow={}\ttype={kind}\taddress={address:#010x}\tvalue={value:#010x}", i / 6).map_err(|e| e.to_string())?;
        let mut fields = table.rows_under(class, &prefix);
        fields.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, row) in fields {
            if let Row::F(hi, lo) = row {
                let mask = u32::MAX >> (31 - hi) & (u32::MAX << lo);
                writeln!(
                    text,
                    "{provenance}\tfield\twindow={}\tname={name}\tbits={hi}:{lo}\tvalue={}",
                    i / 6,
                    (value & mask) >> lo
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(text)
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let date = args
        .next()
        .ok_or("usage: kf-precomp-tier3 UTC-DATE [GPU-INDEX]")?;
    let gpu = args
        .next()
        .unwrap_or_else(|| "0".into())
        .parse::<u32>()
        .map_err(|e| format!("GPU index: {e}"))?;
    if args.next().is_some() {
        return Err("unexpected argument; no register argument supported".into());
    }
    let caps = capability::current_thread_caps().map_err(|e| e.to_string())?;
    if !caps.holds_effective(capability::CAP_SYS_ADMIN) {
        return Err("requires effective CAP_SYS_ADMIN; no escalation attempted".into());
    }
    let dev = DevDir::open(c"/dev").map_err(|e| e.to_string())?;
    let rm = kf_host::HostRm::open(&dev, GpuId(gpu), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    let (architecture, implementation, _) = rm.arch_info();
    let die_name = kf_chip::DieId::new(architecture, implementation)
        .name()
        .ok_or("own host die has no generated name")?;
    let provenance = tag(die_name, rm.driver_version(), &date)?;
    let row = kf_chip::display::display_for(architecture, implementation)
        .ok_or("own host die has no public display row")?;
    let table = kf_disp::class::for_version(rm.driver_version())
        .ok_or("no exact public class vocabulary")?;
    let regs = kf_disp::regs::Regs::for_ip(rm.driver_version(), row.ip_version)
        .ok_or("no exact public register vocabulary")?;
    let die = kf_disp::capsderive::Die {
        architecture,
        implementation,
    };
    let mut evidence = String::new();
    let _snapshot = kf_disp::hostcaps::query(table, &regs, row, die, |addresses| {
        let values = rm.measure_precomp_capabilities(addresses)?;
        evidence = rows(
            &provenance,
            table,
            row.classes.caps,
            row.windows,
            addresses,
            &values,
        )?;
        Ok(values)
    })?;
    print!("{evidence}");
    println!("PRECOMP_TIER3=OK {provenance} CAP_SYS_ADMIN=effective READ32_GLOBAL_only=1");
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("PRECOMP_TIER3=REFUSED {e}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_blackwell_die_gets_its_own_provenance() {
        for d in ["GB202", "GB203", "GB205", "GB206", "GB207"] {
            assert_eq!(
                tag(d, "595.91.07", "2026-10-23").unwrap(),
                format!("measured:{d},595.91.07,2026-10-23")
            );
        }
    }
    #[test]
    fn invalid_identity_or_date_cannot_label_a_measurement() {
        for (d, v, t) in [
            ("GB204", "595.91.07", "2026-10-23"),
            ("GB205", "", "2026-10-23"),
            ("GB205", "595.91.07", "today"),
        ] {
            assert!(tag(d, v, t).is_err());
        }
    }
    fn output(values: &[u32]) -> Result<String, String> {
        let table = ClassTable::parse("F\tNVCA73_PRECOMP_WIN_PIPE_HDR_CAPC_TEST\t7\t4\n");
        rows(
            "measured:GB205,595.91.07,2026-10-23",
            &table,
            0xca73,
            1,
            &[1, 2, 3, 4, 5, 6],
            values,
        )
    }
    #[test]
    fn field_rows_decode_the_public_layout_of_the_actual_reply() {
        let s = output(&[1, 2, 0xa0, 4, 5, 6]).unwrap();
        assert!(s.contains("bits=7:4\tvalue=10"));
        assert!(
            s.lines()
                .all(|l| l.starts_with("measured:GB205,595.91.07,2026-10-23\t"))
        );
    }
    #[test]
    fn a_short_page_is_not_published_as_complete() {
        assert!(output(&[1, 2]).is_err());
    }
    #[test]
    fn an_inaccessible_page_is_not_published_as_complete() {
        assert!(output(&[u32::MAX; 6]).is_err());
    }
    #[test]
    fn an_empty_page_is_not_published_as_complete() {
        assert!(output(&[0; 6]).is_err());
    }
}
