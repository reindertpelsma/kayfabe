//! ★ **Unprivileged control probe — which of the inventory's controls does the HOST answer to a
//! client with no `CAP_SYS_ADMIN`?** (constants ruling §AF/§AH tier 2; `V3_WINDOWS_ONLY_CONSTANTS_20261013.md`.)
//!
//! RM's administrator check is `capable(CAP_SYS_ADMIN)` (`os_is_administrator`,
//! `os-interface.c:382`, `nv-linux.h:502`), not uid. kayfabe starts unprivileged, so a value this
//! probe can read is a value kayfabe can ask the host for at runtime: no table, no sweep.
//!
//! The probe REFUSES to run with `CAP_SYS_ADMIN` in its effective set (a measurement taken as an
//! admin says nothing about what kayfabe can do) unless `KF_PROBE_ALLOW_ADMIN=1`, and it only
//! runs when `KF_PROBE_I_KNOW=1` (it sends controls with ZEROED inputs to the real host GPU; the
//! list below holds queries only, and the one known action control is excluded).
//!
//! Usage: `KF_PROBE_I_KNOW=1 kf-ctl-probe [extra.tsv]` (extra lines: `kind<TAB>cmd_hex<TAB>size`,
//! kind = device | subdevice | binapi). Output: one `PROBE` line per (control, size) with the RM
//! status, by name, and a `PROBE_SUMMARY` count per status. Statuses are RM's own and are not
//! interpreted here: `NOT_SUPPORTED` = the host does not answer it to this client,
//! `INSUFFICIENT_PERMISSIONS` = needs admin, `INVALID_PARAM_STRUCT` = the size is wrong (try the
//! next size: RM checks the size of known controls), `OK` = answered (the reply is printed as hex
//! prefix only; it is a measurement, not a table entry).
//!
//! Status names: `nvstatuscodes.h`, host driver 595.91.07 (ogkm).
//! NOT probed: NV0073 display controls (the display-common object needs its own allocation).

use kf_host::HostRm;
use kf_linux_raw::DevDir;

/// (kind, cmd, sizes to try). Sizes are the reply sizes the real GPU returned in the five
/// observer captures (inventory `M:` notes); a control with no recorded size gets a ladder.
const LADDER: &[u32] = &[
    4, 8, 12, 16, 20, 24, 28, 32, 64, 128, 256, 512, 1024, 2048, 4096,
];
const BUILTIN: &[(&str, u32, &[u32])] = &[
    ("subdevice", 0x2080_852a, &[1784]),
    ("subdevice", 0x2080_852e, &[528]),
    ("subdevice", 0x2080_852f, &[776]),
    ("subdevice", 0x2080_853a, &[1456]),
    ("subdevice", 0x2080_853b, LADDER),
    ("subdevice", 0x2080_8524, &[4]),
    ("subdevice", 0x2080_9001, &[8]),
    ("subdevice", 0x2080_9004, &[1544]),
    ("subdevice", 0x2080_9009, &[8]),
    ("subdevice", 0x2080_9064, &[520]),
    ("subdevice", 0x2080_a02f, &[1]),
    ("subdevice", 0x2080_a060, &[28]),
    ("subdevice", 0x2080_a088, &[12]),
    ("subdevice", 0x2080_a618, &[8620]),
    ("subdevice", 0x2080_a619, LADDER),
    ("subdevice", 0x2080_a630, &[1160]),
    ("subdevice", 0x2080_a70a, &[26]),
    ("subdevice", 0x2080_a801, &[1028]),
    ("subdevice", 0x2080_b201, &[2188]),
    ("subdevice", 0x2080_2806, &[4]),
    ("subdevice", 0x2080_2801, LADDER),
    ("subdevice", 0x2080_2a12, LADDER),
    ("subdevice", 0x2080_121f, LADDER),
    ("subdevice", 0x2080_0a2e, LADDER),
    ("subdevice", 0x2080_0a3f, LADDER),
    ("subdevice", 0x2080_0a80, LADDER),
    ("subdevice", 0x2080_0a87, LADDER),
    ("subdevice", 0x2080_0ab8, LADDER),
    ("subdevice", 0x2080_0b03, LADDER),
    ("device", 0x0080_9908, &[16]),
    ("binapi", 0x2081_0108, &[992]),
    ("binapi", 0x2081_010d, &[0, 4, 8]),
];
/// Action controls kept out on purpose: `0x20801111` submits a software runlist.
const EXCLUDED: &[u32] = &[0x2080_1111];

const NV_OK: u32 = 0;
/// `nvstatuscodes.h` (host 595.91.07).
fn status_name(s: u32) -> &'static str {
    match s {
        0x00 => "OK",
        0x02 => "BUFFER_TOO_SMALL",
        0x0f => "GPU_IS_LOST",
        0x1a => "INSUFFICIENT_RESOURCES",
        0x1b => "INSUFFICIENT_PERMISSIONS",
        0x1f => "INVALID_ARGUMENT",
        0x22 => "INVALID_CLASS",
        0x24 => "INVALID_COMMAND",
        0x25 => "INVALID_DATA",
        0x2e => "INVALID_LIMIT",
        0x31 => "INVALID_OBJECT",
        0x33 => "INVALID_OBJECT_HANDLE",
        0x36 => "INVALID_OBJECT_PARENT",
        0x39 => "INVALID_OWNER",
        0x3a => "INVALID_PARAM_STRUCT",
        0x3f => "INVALID_REQUEST",
        0x40 => "INVALID_STATE",
        0x51 => "NO_MEMORY",
        0x55 => "NOT_READY",
        0x56 => "NOT_SUPPORTED",
        0x57 => "OBJECT_NOT_FOUND",
        0x59 => "OPERATING_SYSTEM",
        _ => "OTHER",
    }
}

/// `CapEff` from `/proc/self/status`, if readable.
fn cap_eff() -> Option<u64> {
    let t = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = t.lines().find(|l| l.starts_with("CapEff:"))?;
    u64::from_str_radix(line.trim_start_matches("CapEff:").trim(), 16).ok()
}

fn main() {
    if std::env::var("KF_PROBE_I_KNOW").as_deref() != Ok("1") {
        eprintln!(
            "kf-ctl-probe sends controls with zeroed inputs to the real host GPU; set KF_PROBE_I_KNOW=1 to run it"
        );
        std::process::exit(2);
    }
    let caps = cap_eff();
    let admin = caps.is_some_and(|c| c & (1 << 21) != 0); // CAP_SYS_ADMIN = 21
    println!(
        "PROBE_START pid={} uid={} CapEff={} cap_sys_admin={}",
        std::process::id(),
        // SAFETY-free: read the uid from /proc instead of libc
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|t| t
                .lines()
                .find(|l| l.starts_with("Uid:"))
                .map(str::to_string))
            .unwrap_or_default()
            .replace('\t', " "),
        caps.map_or("unknown".into(), |c| format!("{c:#x}")),
        admin
    );
    if admin && std::env::var("KF_PROBE_ALLOW_ADMIN").as_deref() != Ok("1") {
        eprintln!(
            "refusing: this process has CAP_SYS_ADMIN, so its answers are not what an unprivileged kayfabe gets (KF_PROBE_ALLOW_ADMIN=1 to override, then the results are labelled admin)"
        );
        std::process::exit(3);
    }
    let mut rows: Vec<(String, u32, Vec<u32>)> = BUILTIN
        .iter()
        .map(|(k, c, s)| ((*k).to_string(), *c, s.to_vec()))
        .collect();
    if let Some(path) = std::env::args().nth(1) {
        for line in std::fs::read_to_string(&path).unwrap_or_default().lines() {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() == 3 {
                if let (Ok(c), Ok(sz)) = (
                    u32::from_str_radix(f[1].trim_start_matches("0x"), 16),
                    f[2].parse::<u32>(),
                ) {
                    rows.push((f[0].to_string(), c, vec![sz]));
                }
            }
        }
    }
    let dev = DevDir::open(c"/dev").expect("open /dev");
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())
        .expect("host RM session");
    println!("PROBE_SESSION driver={}", rm.driver_version());
    let mut counts: std::collections::BTreeMap<&'static str, u32> =
        std::collections::BTreeMap::new();
    // ★ Lane C 2026-10-14: `KF_PROBE_GSS_ROWS=1` asks exactly what kayfabe asks at realize — every
    // `kf_abi::gssreplay::ROWS` request (zero background; plus the 0xa5 background for non-exact
    // rows, as `kf_rm::hostquery::query_gss_replay` does) — and prints RM's status and how many
    // bytes the host changed. Reply bytes are not printed (unknown layouts; nothing board-specific
    // leaves the host). Decides, per host driver, which rows kayfabe can be answered at runtime.
    if std::env::var("KF_PROBE_GSS_ROWS").as_deref() == Ok("1") {
        let sub = rm.subdevice_handle();
        for row in kf_abi::gssreplay::ROWS {
            let mut probes = vec![("zero", row.request(), row.request())];
            if !row.exact {
                let bg = row.request_over(kf_abi::gssreplay::PROBE_BACKGROUND);
                probes.push(("bg", bg.clone(), bg));
            }
            for (label, mut payload, sent) in probes {
                match rm.raw_control_opaque(sub, row.cmd, &mut payload) {
                    Ok(status) => {
                        let name = status_name(status);
                        *counts.entry(name).or_insert(0) += 1;
                        let changed = payload.iter().zip(&sent).filter(|(a, b)| a != b).count();
                        println!(
                            "GSSROW cmd={:#010x} size={} exact={} inputs={:x?} probe={label} status={status:#x} {name} changed_bytes={changed}",
                            row.cmd, row.size, row.exact, row.inputs
                        );
                    }
                    Err(e) => println!(
                        "GSSROW cmd={:#010x} size={} probe={label} ioctl_error={e:?}",
                        row.cmd, row.size
                    ),
                }
            }
        }
        for (name, n) in counts {
            println!("PROBE_SUMMARY {name}={n}");
        }
        println!("PROBE_END admin_run={admin} mode=gss_rows");
        return;
    }
    for (kind, cmd, sizes) in rows {
        if EXCLUDED.contains(&cmd) {
            println!("PROBE kind={kind} cmd={cmd:#010x} size=- status=- EXCLUDED (action control)");
            continue;
        }
        let object = match kind.as_str() {
            "device" => rm.device_handle(),
            "subdevice" => rm.subdevice_handle(),
            other => {
                println!(
                    "PROBE kind={other} cmd={cmd:#010x} size=- status=- NOT_PROBED (no object of this kind in v1)"
                );
                continue;
            }
        };
        let mut done;
        for size in sizes {
            let mut payload = vec![0u8; size as usize];
            match rm.raw_control_opaque(object, cmd, &mut payload) {
                Ok(status) => {
                    let name = status_name(status);
                    *counts.entry(name).or_insert(0) += 1;
                    let head: String = payload
                        .iter()
                        .take(16)
                        .map(|b| format!("{b:02x}"))
                        .collect();
                    println!(
                        "PROBE kind={kind} cmd={cmd:#010x} size={size} status={status:#x} {name}{}",
                        if status == NV_OK {
                            format!(" reply_head={head}")
                        } else {
                            String::new()
                        }
                    );
                    // keep laddering only while RM says the SIZE is wrong
                    done = status != 0x3a && status != 0x1f && status != 0x02;
                }
                Err(e) => {
                    println!("PROBE kind={kind} cmd={cmd:#010x} size={size} ioctl_error={e:?}");
                    done = true;
                }
            }
            if done {
                break;
            }
        }
    }
    for (name, n) in counts {
        println!("PROBE_SUMMARY {name}={n}");
    }
    println!("PROBE_END admin_run={admin}");
}
