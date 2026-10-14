//! Bench-only read of the public RC journal; no channel or GPU work is created.
//! Layout is C-compiled from the exact public host-driver tag, not a capture.
//! Raw records may contain diagnostic addresses: keep them on trusted/private storage.
use kf_host::HostRm;
use kf_linux_raw::DevDir;
use std::{fs, path::Path};

fn main() {
    if let Err(e) = run() {
        eprintln!("RC_JOURNAL_PROBE=REFUSED {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err(
            "usage: kf-rc-journal-probe <exact-driver> <C-layout.txt> <new-private-output-dir>"
                .into(),
        );
    }
    let layout: Vec<usize> = fs::read_to_string(&args[1])
        .map_err(|e| e.to_string())?
        .split_whitespace()
        .map(str::parse::<usize>)
        .collect::<Result<_, _>>()
        .map_err(|e| format!("{e:?}"))?;
    if layout.len() != 8 {
        return Err("layout requires eight C-derived fields".into());
    }
    let (count_cmd, record_cmd, count_size, record_size, index_at, length_at, bytes_at, capacity) = (
        layout[0], layout[1], layout[2], layout[3], layout[4], layout[5], layout[6], layout[7],
    );
    // Fixed public control names and bounded, non-overlapping scalar/byte fields.
    if count_cmd != 0x2080_2205
        || record_cmd != 0x2080_2213
        || count_size != 4
        || capacity == 0
        || capacity > 8192
        || record_size > 16384
        || index_at.checked_add(4).is_none_or(|n| n > length_at)
        || length_at.checked_add(4).is_none_or(|n| n > bytes_at)
        || bytes_at
            .checked_add(capacity)
            .is_none_or(|n| n > record_size)
    {
        return Err("C-derived layout outside bounded public query contract".into());
    }
    let dev = DevDir::open(c"/dev").map_err(|e| format!("dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| format!("{e:?}"))?;
    if rm.driver_version() != args[0] {
        return Err("exact driver/header tag mismatch".into());
    }
    let out = Path::new(&args[2]);
    fs::create_dir(out).map_err(|e| format!("{e:?}"))?;
    // Authored, read-only query on our own subdevice. Exact public-header layout;
    // raw_control_opaque preserves RM refusal, never invents a successful answer.
    let query_count = || -> Result<u32, String> {
        let mut bytes = vec![0; count_size];
        let status = rm
            .raw_control_opaque(rm.subdevice(), count_cmd as u32, &mut bytes)
            .map_err(|e| format!("{e:?}"))?;
        if status != 0 {
            return Err(format!("count status={status:#x}"));
        }
        Ok(u32::from_le_bytes(
            bytes[..4].try_into().map_err(|_| "count field")?,
        ))
    };
    let count = query_count()?;
    println!(
        "RC_JOURNAL driver={} count={count} max_records=32",
        rm.driver_version()
    );
    for index in count.saturating_sub(32)..count {
        let mut bytes = vec![0; record_size];
        bytes[index_at..index_at + 4].copy_from_slice(&index.to_le_bytes());
        let status = rm
            .raw_control_opaque(rm.subdevice(), record_cmd as u32, &mut bytes)
            .map_err(|e| format!("{e:?}"))?;
        println!("RC_JOURNAL index={index} status={status:#x}");
        if status != 0 {
            return Err(format!("record {index} status={status:#x}"));
        }
        let n = u32::from_le_bytes(
            bytes[length_at..length_at + 4]
                .try_into()
                .map_err(|_| "length field")?,
        ) as usize;
        if n > capacity {
            return Err(format!("record {index} oversized {n}"));
        }
        fs::write(
            out.join(format!("record-{index}.bin")),
            &bytes[bytes_at..bytes_at + n],
        )
        .map_err(|e| format!("{e:?}"))?;
        println!("RC_JOURNAL index={index} bytes={n}");
    }
    let after = query_count()?;
    println!("RC_JOURNAL count_before={count} count_after={after}");
    if after != count {
        return Err("journal changed during read; snapshot inconsistent".into());
    }
    println!("RC_JOURNAL_PROBE=READ_COMPLETE (bounded records, not a fault-cause verdict)");
    Ok(())
}
