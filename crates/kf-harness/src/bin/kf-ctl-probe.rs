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
    // ★ Lane C 2026-10-14: `KF_PROBE_EXTRA_ONLY=1` skips the built-in rows; an extra line may carry
    // a 4th column of request words `off=value,...` (hex) — a measured request shape, never guessed.
    let extra_only = std::env::var("KF_PROBE_EXTRA_ONLY").as_deref() == Ok("1");
    type ProbeRow = (String, u32, Vec<u32>, Vec<(usize, u32)>);
    let mut rows: Vec<ProbeRow> = if extra_only {
        Vec::new()
    } else {
        BUILTIN
            .iter()
            .map(|(k, c, s)| ((*k).to_string(), *c, s.to_vec(), Vec::new()))
            .collect()
    };
    let hex = |t: &str| u32::from_str_radix(t.trim().trim_start_matches("0x"), 16);
    if let Some(path) = std::env::args().nth(1) {
        for line in std::fs::read_to_string(&path).unwrap_or_default().lines() {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() == 3 || f.len() == 4 {
                let inputs: Option<Vec<(usize, u32)>> = match f.get(3) {
                    None => Some(Vec::new()),
                    Some(spec) => spec
                        .split(',')
                        .map(|kv| {
                            let (k, v) = kv.split_once('=')?;
                            Some((hex(k).ok()? as usize, hex(v).ok()?))
                        })
                        .collect(),
                };
                if let (Ok(c), Ok(sz), Some(inputs)) = (hex(f[1]), f[2].parse::<u32>(), inputs) {
                    rows.push((f[0].to_string(), c, vec![sz], inputs));
                }
            }
        }
    }
    let dev = DevDir::open(c"/dev").expect("open /dev");
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())
        .expect("host RM session");
    println!("PROBE_SESSION driver={}", rm.driver_version());
    // ★ Lane C 2026-10-14 (resumed): the die as the HOST states it (`MC_GET_ARCH_INFO`, the same two words kayfabe derives
    // `DieId` from at realize), so a rental's evidence names the die by the host's own answer, not by a PCI-id table.
    {
        let (arch, imp, _) = rm.arch_info();
        let die = kf_chip::die::DieId::new(arch, imp);
        println!(
            "PROBE_DIE arch={arch:#x} impl={imp:#x} die={}",
            die.name().unwrap_or("unnamed")
        );
    }
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
    // ★ Lane C 2026-10-14 (resumed): `KF_PROBE_ZBC=1` — the ZBC clear table of the host GPU, read-only and unprivileged:
    // `GET_ZBC_CLEAR_TABLE_SIZE` then `GET_ZBC_CLEAR_TABLE_ENTRY` for every index of each of the three tables, on a
    // `GF100_ZBC_CLEAR` object allocated under the subdevice exactly as kayfabe's own `rmfacts` does at realize. On an idle
    // rental the occupied entries are the defaults physical RM installs (`kf-rm/src/zbc.rs` carries a GA102 capture of
    // them to every die: matrix F-1). The table is GPU-global, so on a shared host this would also show other processes'
    // entries: run it only on a box nothing else uses. Values are clear colours/depths/stencils: no board data.
    if std::env::var("KF_PROBE_ZBC").as_deref() == Ok("1") {
        use kf_abi::zbc as z;
        let sub = rm.subdevice_handle();
        let want = rm.mint();
        let obj = match rm.raw_alloc(sub, want, 0x9096, None, &mut []) {
            Ok(h) => h,
            Err(e) => {
                println!("ZBC alloc_error={e:?}");
                println!("PROBE_END admin_run={admin} mode=zbc");
                return;
            }
        };
        for t in z::TableType::ALL {
            let mut p = vec![0u8; z::GET_SIZE_PARAMS_SIZE];
            z::put(&mut p, 2, t.wire());
            match rm.raw_control_opaque(obj, z::GET_ZBC_CLEAR_TABLE_SIZE, &mut p) {
                Ok(0) => {
                    let (start, end) = (z::word(&p, 0).unwrap_or(0), z::word(&p, 1).unwrap_or(0));
                    println!("ZBC table={} size start={start} end={end}", t.wire());
                    for idx in start..=end.min(start + 63) {
                        let mut e = vec![0u8; z::GET_ENTRY_PARAMS_SIZE];
                        z::put(&mut e, 11, idx); // index
                        z::put(&mut e, 13, t.wire()); // tableType
                        match rm.raw_control_opaque(obj, z::GET_ZBC_CLEAR_TABLE_ENTRY, &mut e) {
                            Ok(0) => {
                                let w: Vec<String> = (0..10)
                                    .map(|i| format!("{:08x}", z::word(&e, i).unwrap_or(0)))
                                    .collect();
                                println!(
                                    "ZBC table={} index={idx} valid={} format={} value={}",
                                    t.wire(),
                                    z::word(&e, 12).unwrap_or(0),
                                    z::word(&e, 10).unwrap_or(0),
                                    w.join(" ")
                                );
                            }
                            Ok(st) => println!(
                                "ZBC table={} index={idx} status={st:#x} {}",
                                t.wire(),
                                status_name(st)
                            ),
                            Err(er) => {
                                println!("ZBC table={} index={idx} ioctl_error={er:?}", t.wire())
                            }
                        }
                    }
                }
                Ok(st) => println!(
                    "ZBC table={} size status={st:#x} {}",
                    t.wire(),
                    status_name(st)
                ),
                Err(er) => println!("ZBC table={} size ioctl_error={er:?}", t.wire()),
            }
        }
        println!("PROBE_END admin_run={admin} mode=zbc");
        return;
    }
    // ★ Lane C 2026-10-14 (resumed): `KF_PROBE_SCAN=0xCMD,0xCMD:LO:HI` — which `paramsSize` does this host accept for a control?
    // The Windows-only GSS-legacy controls have no open header, and their size changes between firmware branches (580: 852e 528,
    // 595: 596). A zero request of every size LO..=HI step 4 (default 4..=16384) is sent to the SUBDEVICE and the sizes whose
    // status is not INVALID_ARGUMENT/INVALID_PARAM_STRUCT are printed with their status and the number of non-zero reply
    // words. Zero requests only (the real Windows requests are all-zero for these controls); unprivileged; reply bytes are not printed.
    if let Ok(spec) = std::env::var("KF_PROBE_SCAN") {
        let sub = rm.subdevice_handle();
        for item in spec.split(',').filter(|x| !x.is_empty()) {
            let mut it = item.split(':');
            let cmd = it
                .next()
                .and_then(|c| u32::from_str_radix(c.trim_start_matches("0x"), 16).ok());
            let lo: usize = it.next().and_then(|x| x.parse().ok()).unwrap_or(4);
            let hi: usize = it.next().and_then(|x| x.parse().ok()).unwrap_or(16384);
            let Some(cmd) = cmd else {
                println!("SCAN bad_spec={item}");
                continue;
            };
            let mut hits = 0;
            let mut size = lo;
            while size <= hi {
                let mut payload = vec![0u8; size];
                if let Ok(status) = rm.raw_control_opaque(sub, cmd, &mut payload)
                    && status != 0x1f
                    && status != 0x3a
                {
                    let nz = payload
                        .chunks(4)
                        .filter(|w| w.iter().any(|b| *b != 0))
                        .count();
                    println!(
                        "SCAN cmd={cmd:#010x} size={size} status={status:#x} {} nonzero_words={nz}",
                        status_name(status)
                    );
                    hits += 1;
                }
                size += 4;
            }
            println!("SCAN cmd={cmd:#010x} range={lo}..={hi} accepted_sizes={hits}");
        }
        println!("PROBE_END admin_run={admin} mode=scan");
        return;
    }
    for (kind, cmd, sizes, inputs) in rows {
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
            for &(o, v) in &inputs {
                if let Some(w) = payload.get_mut(o..o + 4) {
                    w.copy_from_slice(&v.to_le_bytes());
                }
            }
            match rm.raw_control_opaque(object, cmd, &mut payload) {
                Ok(status) => {
                    let name = status_name(status);
                    *counts.entry(name).or_insert(0) += 1;
                    // ★ Lane C: `KF_PROBE_HEAD=<n>` widens the printed prefix (at most 64 bytes) for
                    // replies whose words are known not to carry board data (the cudart gate).
                    let head_len = std::env::var("KF_PROBE_HEAD")
                        .ok()
                        .and_then(|v| v.parse::<usize>().ok())
                        .map_or(16, |n| n.min(64));
                    let head: String = payload
                        .iter()
                        .take(head_len)
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
