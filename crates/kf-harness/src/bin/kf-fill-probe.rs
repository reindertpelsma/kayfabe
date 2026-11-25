//! Probe of the `unmapped-reads-zero` fill (`GuestVaFill`) against host-RM page-table capacity, with no VM.
//!
//! The question (finding F1 of `traces/fill_probe_20261124/PREREGISTRATION.md`): a Windows guest
//! kernel's flat FB alias, one 4 KiB-pinned store slice of `0xefa00000` bytes at `0x120000000`,
//! fails in host RM's page walk (`mmuWalkMap ... NV_ERR_NO_MEMORY`, NVOS46 status `0x51`) when the
//! static guest ranges are reserved SPARSE (`GuestVaFill::Zero`), and never with the fill off. This
//! probe builds the same space shape a declared Windows guest gets (`MirrorVaStart::GuestFloor`
//! unless told otherwise), the same store (`HostRm::reserve_gpga`) and the same map
//! (`MapBacking::SharedSlice`, FIXED), with optional map/unmap churn before the alias map.
//!
//! ```text
//! kf-fill-probe <zero|fault> <floor|default> <alias_len_hex> <churn_regions> <rounds> [extra_spaces] [split|low] [--leave-free-mib=N]
//! ```
//!
//! - `churn_regions` 4 KiB maps per round, each in its own 2 MiB region, mapped and unmapped at once:
//!   the first `CHURN_LOW_REGIONS` of a round lie in `[0x120000000, +4 GiB)` (the alias range), the
//!   rest above `1 TiB`. Each round uses fresh regions, so every region is touched once.
//! - `low` layout: every churn region in the first static range `[0x120000000, 960 GiB)` instead.
//! - `extra_spaces` more spaces of the same shape are allocated first (Windows has one per process).
//!
//! - `--leave-free-mib=N` (anywhere on the command line): VRAM fillers leave about N MiB free after the store.
//!
//! Output: `MEASURE` lines (host FB used from `nvidia-smi` at each step, the map's answer) and one
//! `FILL_PROBE_RESULT` line. A failed alias map is a result, not a probe failure: the exit code is 0
//! when every step ran, 1 when a step other than the alias map was refused.
//!
//! Every object is this process's own; nothing is shared with a guest.

use kf_harness::Ledger;
use kf_host::channel::{GuestVaFill, MirrorVaStart};
use kf_host::{HostRm, MapBacking, RmError, VaSpace};
use kf_linux_raw::DevDir;

const P: u64 = 0x1000;
const LEAF: u64 = 0x20_0000;
/// The store: 4 GiB (fb-mb 4096), or the alias length rounded up to 1 GiB when that is larger.
const STORE_MIN: u64 = 4 << 30;
const MAX_ALIAS: u64 = 16 << 30;
const ALIAS_VA: u64 = 0x1_2000_0000;
/// Regions per round that lie in the alias range (`[0x120000000, +4 GiB)` holds 2048 regions).
const CHURN_LOW_REGIONS: u64 = 256;
const HIGH_BASE: u64 = 1 << 40;
/// One TLB invalidate per this many deferred map/unmap pairs.
const FLUSH_EVERY: u64 = 4096;

fn fb_used_mib() -> String {
    std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.used", "--format=csv,noheader,nounits"])
        .output()
        .map_or_else(
            |e| format!("nvidia-smi:{e}"),
            |o| String::from_utf8_lossy(&o.stdout).trim().to_string(),
        )
}

fn fb_free_mib() -> Option<u64> {
    let o = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.free", "--format=csv,noheader,nounits"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&o.stdout).trim().parse().ok()
}

/// `--leave-free-mib=N`: allocate 256 MiB VRAM fillers until at most N MiB stay free, so a 24 GB
/// die has the headroom of a smaller one.
fn fill_vram(rm: &HostRm, l: &mut Ledger, leave_free_mib: Option<u64>) -> Result<Vec<u32>, String> {
    let Some(target) = leave_free_mib else {
        return Ok(Vec::new());
    };
    const CHUNK: u64 = 256 << 20;
    let mut held = Vec::new();
    while fb_free_mib().is_some_and(|f| f > target + 256) {
        held.push(
            rm.alloc_device_local(CHUNK)
                .map_err(|e| format!("filler: {e:?}"))?,
        );
    }
    l.measure(
        "filler",
        format!("{} x 256 MiB, free now {:?} MiB", held.len(), fb_free_mib()),
    );
    Ok(held)
}

fn snap(l: &mut Ledger, step: &str) {
    std::thread::sleep(std::time::Duration::from_millis(500));
    l.measure("fb_used_mib", format!("{step} {}", fb_used_mib()));
}

fn describe(r: &Result<u64, RmError>) -> String {
    match r {
        Ok(va) => format!("OK va={va:#x}"),
        Err(RmError::Other(kf_host::VA_ALREADY_MAPPED)) => {
            "REFUSED nvos46_status=0x51 (kf-host reports it as VA_ALREADY_MAPPED)".to_string()
        }
        Err(e) => format!("REFUSED {e:?}"),
    }
}

/// The VA of churn region `i` of round `round`.
fn churn_va(round: u64, i: u64, per_round: u64) -> u64 {
    let low = CHURN_LOW_REGIONS.min(per_round);
    if i < low {
        // Spread over the 2048 regions of the alias range; offset inside the region varies so the
        // map is not always a region's first page.
        let region = (round * low + i) % 2048;
        ALIAS_VA + region * LEAF + ((i % 512) * P)
    } else {
        let region = round * (per_round - low) + (i - low);
        HIGH_BASE + region * LEAF + ((i % 512) * P)
    }
}

/// The 2 MiB regions of the first static guest range, `[0x120000000, HOST_HOLE_LO)`.
const LOW_RANGE_REGIONS: u64 = (kf_host::channel::HOST_HOLE_LO - ALIAS_VA) / LEAF;

/// The VA of churn region `i` of round `round` in the `low` layout: every region in the first
/// static guest range, each one once.
fn churn_va_low(round: u64, i: u64, per_round: u64) -> Option<u64> {
    let region = round * per_round + i;
    (region < LOW_RANGE_REGIONS).then(|| ALIAS_VA + region * LEAF + ((i % 512) * P))
}

struct Args {
    leave_free_mib: Option<u64>,
    low_layout: bool,
    fill: GuestVaFill,
    start: MirrorVaStart,
    alias_len: u64,
    regions: u64,
    rounds: u64,
    extra_spaces: u64,
}

fn parse() -> Result<Args, String> {
    let all: Vec<String> = std::env::args().skip(1).collect();
    parse_from(&all)
}

fn parse_from(all: &[String]) -> Result<Args, String> {
    const LEAVE: &str = "--leave-free-mib=";
    let mut leave_free_mib = None;
    let mut a: Vec<String> = Vec::new();
    for s in all {
        if let Some(v) = s.strip_prefix(LEAVE) {
            leave_free_mib = Some(v.parse::<u64>().map_err(|e| format!("{LEAVE}{v}: {e}"))?);
        } else {
            a.push(s.clone());
        }
    }
    if a.len() < 5 {
        return Err("usage: kf-fill-probe <zero|fault> <floor|default> <alias_len_hex> <churn_regions> <rounds> [extra_spaces] [split|low] [--leave-free-mib=N]".into());
    }
    let fill = match a[0].as_str() {
        "zero" => GuestVaFill::Zero,
        "fault" => GuestVaFill::Fault,
        o => return Err(format!("fill {o}")),
    };
    let start = match a[1].as_str() {
        "floor" => MirrorVaStart::GuestFloor,
        "default" => MirrorVaStart::HostDefault,
        o => return Err(format!("start {o}")),
    };
    let hex =
        |s: &str| u64::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|e| e.to_string());
    let num = |s: &str| s.parse::<u64>().map_err(|e| e.to_string());
    let alias_len = hex(&a[2])?;
    if alias_len == 0 || alias_len % P != 0 || alias_len > MAX_ALIAS {
        return Err(format!("alias_len {alias_len:#x}"));
    }
    Ok(Args {
        fill,
        start,
        alias_len,
        regions: num(&a[3])?,
        rounds: num(&a[4])?,
        extra_spaces: a.get(5).map_or(Ok(0), |s| num(s))?,
        low_layout: a.get(6).is_some_and(|s| s == "low"),
        leave_free_mib,
    })
}

fn main() {
    let args = match parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("kf-fill-probe: {e}");
            std::process::exit(2);
        }
    };
    let mut l = Ledger::default();
    match run(&mut l, &args) {
        Ok(alias) => {
            println!(
                "FILL_PROBE_RESULT fill={:?} start={:?} alias_len={:#x} churn={}x{} extra_spaces={} alias_map={alias}",
                args.fill, args.start, args.alias_len, args.regions, args.rounds, args.extra_spaces
            );
        }
        Err(e) => {
            println!("FILL_PROBE_ERROR {e}");
            std::process::exit(1);
        }
    }
}

fn run(l: &mut Ledger, a: &Args) -> Result<String, String> {
    let dev = DevDir::open(c"/dev").map_err(|e| format!("open /dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    l.measure("session", format!("driver {}", rm.driver_version()));
    snap(l, "start");
    rm.set_guest_va_fill(a.fill);
    let store_bytes = STORE_MIN.max(a.alias_len.div_ceil(1 << 30) << 30);
    let store = rm
        .reserve_gpga(store_bytes)
        .map_err(|e| format!("store: {e:?}"))?;
    l.measure(
        "store",
        format!(
            "{store_bytes:#x} contiguous_aligned={}",
            store.contiguous_aligned
        ),
    );
    snap(l, "after_store");
    let fillers = fill_vram(&rm, l, a.leave_free_mib)?;

    let mut extra: Vec<VaSpace> = Vec::new();
    for i in 0..a.extra_spaces {
        extra.push(
            rm.alloc_vaspace_at(a.start)
                .map_err(|e| format!("extra space {i}: {e:?}"))?,
        );
    }
    let space = rm
        .alloc_vaspace_at(a.start)
        .map_err(|e| format!("vaspace: {e:?}"))?;
    for g in &space.guest {
        l.measure(
            "guest_range",
            format!("[{:#x}, {:#x}) reserved={}", g.lo, g.hi, g.handle != 0),
        );
    }
    snap(l, "after_spaces");

    let mut pairs = 0u64;
    for round in 0..a.rounds {
        for i in 0..a.regions {
            let va = if a.low_layout {
                churn_va_low(round, i, a.regions)
                    .ok_or_else(|| format!("churn region {round}/{i} beyond the first range"))?
            } else {
                churn_va(round, i, a.regions)
            };
            let off = (pairs * P) % store_bytes;
            rm.map(
                space,
                store.handle,
                MapBacking::SharedSlice,
                off,
                P,
                Some(va),
                true,
            )
            .map_err(|e| format!("churn map {va:#x}: {e:?}"))?;
            rm.unmap_row(space, va, P, true)
                .map_err(|e| format!("churn unmap {va:#x}: {e:?}"))?;
            pairs += 1;
            if pairs.is_multiple_of(FLUSH_EVERY) {
                rm.invalidate_tlb(space)
                    .map_err(|e| format!("invalidate: {e:?}"))?;
            }
        }
        rm.invalidate_tlb(space)
            .map_err(|e| format!("invalidate: {e:?}"))?;
        snap(l, &format!("after_churn_round_{round} pairs={pairs}"));
    }

    let r = rm.map(
        space,
        store.handle,
        MapBacking::SharedSlice,
        0,
        a.alias_len,
        Some(ALIAS_VA),
        false,
    );
    let verdict = describe(&r);
    l.measure(
        "alias_map",
        format!("{ALIAS_VA:#x}+{:#x}: {verdict}", a.alias_len),
    );
    snap(l, "after_alias_map");
    if r.is_ok() {
        rm.unmap_row(space, ALIAS_VA, a.alias_len, false)
            .map_err(|e| format!("alias unmap: {e:?}"))?;
        snap(l, "after_alias_unmap");
    }
    rm.free_vaspace(space);
    for s in extra {
        rm.free_vaspace(s);
    }
    snap(l, "after_free_spaces");
    for h in fillers {
        let _ = rm.free(h);
    }
    let _ = rm.free(store.handle);
    snap(l, "after_free_store");
    Ok(verdict)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn churn_regions_are_distinct_and_inside_the_static_ranges() {
        let (per, rounds) = (1024u64, 3u64);
        let mut seen = std::collections::BTreeSet::new();
        for r in 0..rounds {
            for i in 0..per {
                let va = churn_va(r, i, per);
                let lo_ok = (ALIAS_VA..ALIAS_VA + (4 << 30)).contains(&va);
                let hi_ok = (HIGH_BASE..1u64 << 47).contains(&va);
                assert!(lo_ok || hi_ok, "{va:#x}");
                assert!(seen.insert(va / LEAF), "region reused: {va:#x}");
            }
        }
    }

    #[test]
    fn leave_free_mib_is_a_command_line_option() {
        let argv = |v: &[&str]| v.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let a = parse_from(&argv(&[
            "zero",
            "--leave-free-mib=3072",
            "floor",
            "efa00000",
            "0",
            "0",
        ]))
        .expect("parses");
        assert_eq!(a.leave_free_mib, Some(3072));
        assert_eq!(a.alias_len, 0xefa0_0000);
        let b = parse_from(&argv(&["fault", "floor", "efa00000", "0", "0"])).expect("parses");
        assert_eq!(b.leave_free_mib, None);
        assert!(
            parse_from(&argv(&[
                "zero",
                "floor",
                "efa00000",
                "0",
                "0",
                "--leave-free-mib=x"
            ]))
            .is_err()
        );
    }

    #[test]
    fn the_low_layout_stays_in_the_first_range_and_refuses_beyond_it() {
        let mut seen = std::collections::BTreeSet::new();
        for r in 0..2 {
            for i in 0..4096 {
                let va = churn_va_low(r, i, 4096).expect("inside");
                assert!(
                    (ALIAS_VA..kf_host::channel::HOST_HOLE_LO).contains(&va),
                    "{va:#x}"
                );
                assert!(seen.insert(va / LEAF));
            }
        }
        assert!(churn_va_low(0, LOW_RANGE_REGIONS, LOW_RANGE_REGIONS + 1).is_none());
    }
}
