//! ★ E2 (EXPERIMENT BRANCH `claude/sparse-hw-20261026`, never on master): is the hardware's sparse
//! "debug page" a CROSS-PROCESS DATA CHANNEL? Bare metal, no kayfabe, unprivileged raw RM.
//!
//! Each process is its own RM client with its own bare VA space and its own `NVOS32_ALLOC_FLAGS_SPARSE`
//! reservations (`S64`: 64 KiB, `S2M`: 2 MiB, at fixed VAs). A copy engine writes a tagged pattern
//! into sparse windows (`writer`) and every process reads sparse windows through the copy engine into
//! a POISONED real buffer (`reader` and `writer`'s own read-back).
//!
//! ```text
//! kf-sparse-dbgpage-probe selftest              # positive control + one write/read round
//! kf-sparse-dbgpage-probe writer <secs> <tag>   # process A: writes pattern <tag> repeatedly + own read-back
//! kf-sparse-dbgpage-probe reader <secs>         # process B: reads only
//! ```
//!
//! Hypothesis: every word read from a sparse window is 0. Falsifier: any non-zero, non-poison word
//! (E2_LEAK). INVALID (not a verdict): a copy that did not release, poison left behind, a failed
//! positive control.

use kf_harness::CeRig;
use kf_host::{HostRm, MapBacking, VaSpace};
use kf_linux_raw::{CachePolicy, DevDir, HostOffset, VolatileRegion};

const SPARSE_FLAGS: u32 = 0x10 | 0x400 | 0x8_0000 | 0x0400_0000;
const BUF: u64 = 0x1_0000;
const POISON: u32 = 0xDEAD_0000;

/// (name, VA, length, windows (offset, length)).
type Region = (&'static str, u64, u64, &'static [(u64, u64)]);
const REGIONS: [Region; 2] = [
    (
        "S64",
        0x2_0000_0000,
        0x1_0000,
        &[
            (0, 0x1000),
            (0x4000, 0x1000),
            (0x8000, 0x1000),
            (0xC000, 0x1000),
        ],
    ),
    (
        "S2M",
        0x2_0020_0000,
        0x20_0000,
        &[
            (0, 0x1000),
            (0x1_0000, 0x1000),
            (0x10_0000, 0x1000),
            (0x1F_F000, 0x1000),
        ],
    ),
];
/// Whole-64 KiB windows, done every 8th round.
const FULL: [(&str, u64, u64); 2] = [
    ("S64", 0x2_0000_0000, 0x1_0000),
    ("S2M", 0x2_0020_0000, 0x1_0000),
];

#[derive(Default, Debug, Clone, Copy)]
struct Tally {
    reads: u64,
    words: u64,
    zero: u64,
    poison: u64,
    other: u64,
    copy_fail: u64,
    first_other: Option<(u64, u32)>,
}

fn pattern(tag: u32, round: u32, idx: u64) -> u32 {
    (tag << 24) | ((round & 0xFF) << 16) | (idx as u32 & 0xFFFF)
}

struct Rig {
    rm: HostRm,
    space: VaSpace,
    ce: CeRig,
    src_cpu: VolatileRegion,
    dst_cpu: VolatileRegion,
    src_va: u64,
    dst_va: u64,
    real_va: u64,
    #[allow(dead_code)]
    keep: Vec<kf_linux_raw::CharDevice>,
}

fn setup() -> Result<Rig, String> {
    let dev = DevDir::open(c"/dev").map_err(|e| format!("open /dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    println!("E2_SESSION driver={} pid={}", rm.driver_version(), std::process::id());
    let space = rm.alloc_vaspace_bare().map_err(|e| format!("vaspace: {e:?}"))?;
    for (name, va, len, _) in REGIONS {
        match rm.reserve_va_flags(space.space, va, len, SPARSE_FLAGS) {
            Ok(h) => println!("E2_RESERVE {name} {va:#x}+{len:#x} SPARSE ACCEPTED handle={h:#x}"),
            Err(e) => return Err(format!("E2_RESERVE {name} {va:#x}+{len:#x} SPARSE REFUSED {e:?}")),
        }
    }
    let mut keep = Vec::new();
    let mut obj = |what: &str| -> Result<(u64, VolatileRegion), String> {
        let o = rm.alloc_device_local(BUF).map_err(|e| format!("{what}: {e:?}"))?;
        let va = rm
            .map(space, o, MapBacking::Dedicated, 0, BUF, None, false)
            .map_err(|e| format!("map {what}: {e:?}"))?;
        let (node, cpu) = rm
            .map_cpu(o, BUF, CachePolicy::Uncached)
            .map_err(|e| format!("cpu {what}: {e:?}"))?;
        keep.push(node);
        Ok((va, cpu))
    };
    let (src_va, src_cpu) = obj("src")?;
    let (dst_va, dst_cpu) = obj("dst")?;
    let (real_va, _real_cpu) = obj("real")?;
    let ce = CeRig::new(&rm, space)?;
    Ok(Rig { rm, space, ce, src_cpu, dst_cpu, src_va, dst_va, real_va, keep })
}

impl Rig {
    fn fill_src(&self, tag: u32, round: u32, words: u64) -> Result<(), String> {
        for i in 0..words {
            self.src_cpu
                .store_u32(HostOffset::new(i * 4), pattern(tag, round, i))
                .map_err(|e| format!("{e:?}"))?;
        }
        Ok(())
    }

    fn poison_dst(&self, words: u64) -> Result<(), String> {
        for i in 0..words {
            self.dst_cpu
                .store_u32(HostOffset::new(i * 4), POISON | (i as u32 & 0xFFFF))
                .map_err(|e| format!("{e:?}"))?;
        }
        Ok(())
    }

    /// Write `len` bytes of the current src pattern to `va`.
    fn write(&mut self, va: u64, len: u64) -> Result<bool, String> {
        let s = self.ce.copy(&self.rm, self.src_va, va, len as u32)?;
        Ok(s.ce_released && s.fence_landed)
    }

    /// Read `len` bytes at `va` into the poisoned dst and tally.
    fn read(&mut self, va: u64, len: u64, t: &mut Tally) -> Result<(), String> {
        let words = len / 4;
        self.poison_dst(words)?;
        let s = self.ce.copy(&self.rm, va, self.dst_va, len as u32)?;
        t.reads += 1;
        if !(s.ce_released && s.fence_landed) {
            t.copy_fail += 1;
            return Ok(());
        }
        for i in 0..words {
            let w = self
                .dst_cpu
                .load_u32(HostOffset::new(i * 4))
                .map_err(|e| format!("{e:?}"))?;
            t.words += 1;
            if w == 0 {
                t.zero += 1;
            } else if w & 0xFFFF_0000 == POISON && w & 0xFFFF == (i as u32 & 0xFFFF) {
                t.poison += 1;
            } else {
                t.other += 1;
                if t.first_other.is_none() {
                    t.first_other = Some((i, w));
                }
            }
        }
        Ok(())
    }
}

fn positive_control(r: &mut Rig, tag: u32) -> Result<bool, String> {
    r.fill_src(tag, 1, 0x400)?;
    let w = r.write(r.real_va, 0x1000)?;
    r.poison_dst(0x400)?;
    let s = r.ce.copy(&r.rm, r.real_va, r.dst_va, 0x1000)?;
    let mut ok = w && s.ce_released;
    let mut bad = 0u64;
    for i in 0..0x400u64 {
        let v = r
            .dst_cpu
            .load_u32(HostOffset::new(i * 4))
            .map_err(|e| format!("{e:?}"))?;
        if v != pattern(tag, 1, i) {
            bad += 1;
        }
    }
    ok &= bad == 0;
    println!("E2_POSITIVE_CONTROL real-VA CE write+read: wrote={w} read_released={} mismatching_words={bad} => {}", s.ce_released, if ok { "OK" } else { "BROKEN" });
    Ok(ok)
}

fn report(role: &str, name: &str, t: &Tally) {
    println!(
        "E2_TALLY role={role} {name} reads={} words={} zero={} poison={} other={} copy_fail={} first_other={:?}",
        t.reads, t.words, t.zero, t.poison, t.other, t.copy_fail, t.first_other
    );
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mode = a.get(1).map_or("selftest", String::as_str).to_owned();
    let secs: u64 = a.get(2).and_then(|v| v.parse().ok()).unwrap_or(5);
    let tag: u32 = a.get(3).and_then(|v| v.parse().ok()).unwrap_or(0xA5);
    println!("E2_START mode={mode} secs={secs} tag={tag:#x}");
    let rc = match run(&mode, secs, tag) {
        Ok(v) => v,
        Err(e) => {
            println!("E2_ERROR {e}");
            println!("E2_VERDICT role={mode} INVALID");
            2
        }
    };
    std::process::exit(rc);
}

fn run(mode: &str, secs: u64, tag: u32) -> Result<i32, String> {
    let mut r = setup()?;
    let pc = positive_control(&mut r, tag)?;
    let writer = mode == "writer" || mode == "selftest";
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    let mut wrote_fail = 0u64;
    let mut wrote = 0u64;
    let mut round = 0u32;
    let mut tallies = std::collections::BTreeMap::<String, Tally>::new();
    loop {
        if writer {
            r.fill_src(tag, round, 0x400)?;
            for (name, base, _, wins) in REGIONS {
                for (off, len) in wins {
                    wrote += 1;
                    if !r.write(base + off, *len)? {
                        wrote_fail += 1;
                    }
                    let _ = name;
                }
            }
            if round % 8 == 0 {
                r.fill_src(tag, round, 0x4000)?;
                for (_, base, len) in FULL {
                    wrote += 1;
                    if !r.write(base, len)? {
                        wrote_fail += 1;
                    }
                }
            }
        }
        for (name, base, _, wins) in REGIONS {
            for (off, len) in wins {
                let key = format!("{name}+{off:#x}/{len:#x}");
                let t = tallies.entry(key).or_default();
                r.read(base + off, *len, t)?;
            }
        }
        if round % 8 == 0 {
            for (name, base, len) in FULL {
                let key = format!("{name}+0/{len:#x}(full)");
                let t = tallies.entry(key).or_default();
                r.read(base, len, t)?;
            }
        }
        round += 1;
        if std::time::Instant::now() >= deadline {
            break;
        }
    }
    let mut all = Tally::default();
    for (k, t) in &tallies {
        report(mode, k, t);
        all.reads += t.reads;
        all.words += t.words;
        all.zero += t.zero;
        all.poison += t.poison;
        all.other += t.other;
        all.copy_fail += t.copy_fail;
        if all.first_other.is_none() {
            all.first_other = t.first_other;
        }
    }
    report(mode, "ALL", &all);
    println!("E2_WRITES role={mode} rounds={round} writes={wrote} write_not_released={wrote_fail}");
    let verdict = if !pc || all.copy_fail > 0 || wrote_fail > 0 || all.poison > 0 {
        "INVALID"
    } else if all.other > 0 {
        "LEAK"
    } else {
        "NO_LEAK"
    };
    println!("E2_VERDICT role={mode} {verdict} (zero_words={} other_words={})", all.zero, all.other);
    Ok(match verdict {
        "NO_LEAK" => 0,
        "LEAK" => 3,
        _ => 2,
    })
}
