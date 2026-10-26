//! ★ **WALK REFUSAL DUMP — MEASUREMENT ONLY** (`KF3_WALK_REFUSAL_DUMP=1`; unset in the production
//! profile, and nothing here changes a decision).
//!
//! The walk kernel's refusal samples ([`kf_cuda::refusal_samples`]) name ONE entry: its VA, its raw
//! bits, its decoded target. That cannot say whether a "misaligned 512 MiB leaf" at a shift-29
//! directory level (VER2 PD1) is a real leaf, a directory entry the format misread, or an entry of
//! a table that is not (any more) a directory of this space. This module, on the VA-manager thread
//! (never a vCPU, never under a lock a vCPU takes), reads the guest's tables from the store the
//! walk read them from ([`kf_cuda::WalkKernel::read_store`]) and prints, per sampled refusal:
//!
//! - (a) the raw entry and its fields decoded BOTH ways the format allows (as a PTE: valid,
//!   aperture, volatile, kind; as a PDE: aperture, address) — from the walk's own descriptor;
//! - the path from the space's root down to the table that holds it (each directory entry raw);
//! - (b) the 4 KiB page the entry names: a summary (valid-bit count, kinds, how many 8-byte
//!   entries name consecutive pages, how many 16-byte entries look like dual PDEs) and its first
//!   [`DUMP_ENTRIES`] 8-byte words — page-table bytes, no other guest data is printed;
//! - (c) [`DUMP_ENTRIES`] entries around the slot in the same table;
//! - (d) the same slot re-read on later polls ([`WATCH_REREADS`] times, ≥ [`WATCH_GAP_MS`] apart):
//!   does it change, and does the directory entry above it move.
//!
//! Bounded: [`DUMP_REPORTS`] reports per process, [`DUMP_SAMPLES`] samples per report, a fixed
//! number of 8-byte / 4 KiB reads per sample, [`WATCHES`] watch points. Every value read here is
//! guest-controlled; it is only printed, with checked arithmetic and bounded reads.

use kf_cuda::abi::{KF_DIRS, KF_PS_NONE, KfField, KfFormat, KfRefusalSample};
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Reports dumped per process.
pub const DUMP_REPORTS: u32 = 12;
/// Samples dumped per report.
pub const DUMP_SAMPLES: usize = 4;
/// Entries printed per table / page.
pub const DUMP_ENTRIES: usize = 16;
/// Watch points kept at once.
pub const WATCHES: usize = 8;
/// Re-reads per watch point.
pub const WATCH_REREADS: u32 = 6;
/// Minimum gap between two re-reads of one watch point.
pub const WATCH_GAP_MS: u64 = 200;
/// The `KFWR_AP_*` value of local video memory (`kf_walk.h`).
const AP_VIDMEM: u8 = 0;

/// Is the measurement on (`KF3_WALK_REFUSAL_DUMP` set to anything but `0`)? Read once.
#[must_use]
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("KF3_WALK_REFUSAL_DUMP").is_ok_and(|v| !v.is_empty() && v != "0")
    })
}

/// `((raw >> lo) & mask(bits)) << shift`, total for any descriptor (a 0-bit field is 0).
#[must_use]
pub fn field(raw: u64, f: KfField) -> u64 {
    if f.bits == 0 || f.lo >= 64 {
        return 0;
    }
    let mask = if f.bits >= 64 {
        u64::MAX
    } else {
        (1u64 << f.bits).wrapping_sub(1)
    };
    let v = (raw >> f.lo) & mask;
    v.checked_shl(u32::from(f.shift)).unwrap_or(0)
}

fn bit(raw: u64, b: u8) -> u64 {
    if b >= 64 { 0 } else { (raw >> b) & 1 }
}

/// The raw aperture nibble.
#[must_use]
pub fn aperture(fmt: &KfFormat, raw: u64) -> u8 {
    let f = KfField {
        lo: fmt.ap_lo,
        bits: fmt.ap_bits,
        shift: 0,
        pad: 0,
    };
    u8::try_from(field(raw, f)).unwrap_or(0)
}

/// The target address the descriptor decodes from `raw` (local or sys spec by the nibble).
#[must_use]
pub fn target(fmt: &KfFormat, raw: u64) -> u64 {
    let ap = usize::from(aperture(fmt, raw));
    let sys = fmt.addr_sel.get(ap).copied().unwrap_or(0) != 0;
    field(raw, if sys { fmt.addr_sys } else { fmt.addr_local })
}

/// (a) One entry decoded both ways the format allows.
#[must_use]
pub fn decode(fmt: &KfFormat, raw: u64) -> String {
    let ap = aperture(fmt, raw);
    let pde_ap = fmt.pde_ap_map.get(usize::from(ap)).copied().unwrap_or(0xFF);
    let pte_ap = fmt.pte_ap_map.get(usize::from(ap)).copied().unwrap_or(0xFF);
    format!(
        "bit0={} ap={} vol={} priv={} ro={} kind={:#x} pcf={:#x} target={:#x} | as-PTE ap->{} | as-PDE ap->{}",
        bit(raw, fmt.valid_bit),
        ap,
        bit(raw, fmt.bit_volatile),
        bit(raw, fmt.bit_privilege),
        bit(raw, fmt.bit_read_only),
        field(raw, fmt.kind),
        field(raw, fmt.pcf),
        target(fmt, raw),
        ap_name(pte_ap),
        if ap == fmt.pde_ap_invalid {
            "INVALID"
        } else {
            ap_name(pde_ap)
        },
    )
}

fn ap_name(a: u8) -> &'static str {
    match a {
        0 => "vid",
        1 => "peer",
        2 => "syscoh",
        3 => "sysnc",
        _ => "none",
    }
}

/// One directory entry on the path from the root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    /// The descriptor's `dir[]` index.
    pub level: usize,
    /// The table it was read from.
    pub table: u64,
    /// Its index in that table.
    pub idx: u64,
    /// The raw entry.
    pub raw: u64,
}

/// The index `va` selects at `dir[k]` (0 for an inactive or malformed level).
#[must_use]
pub fn index_at(fmt: &KfFormat, k: usize, va: u64) -> u64 {
    let Some(d) = fmt.dir.get(k) else { return 0 };
    if d.va_lo >= 64 || d.entries == 0 {
        return 0;
    }
    (va >> d.va_lo) & u64::from(d.entries).wrapping_sub(1)
}

/// Walk from `pdb` down to the table of `dir[level]` that holds `va`, reading each directory
/// entry with `read8`. Returns the path and the table at `level` (None if a step was not a
/// vidmem directory, or a read failed). At most `KF_DIRS` reads.
pub fn path_to(
    fmt: &KfFormat,
    pdb: u64,
    va: u64,
    level: usize,
    read8: &mut dyn FnMut(u64) -> Option<u64>,
) -> (Vec<Step>, Option<u64>) {
    let mut steps = Vec::new();
    let mut table = pdb;
    let first = usize::from(fmt.first_dir);
    for k in first..level.min(KF_DIRS) {
        let Some(d) = fmt.dir.get(k) else {
            return (steps, None);
        };
        let idx = index_at(fmt, k, va);
        let Some(at) = idx
            .checked_mul(u64::from(d.entry_bytes))
            .and_then(|o| table.checked_add(o))
        else {
            return (steps, None);
        };
        let Some(raw) = read8(at) else {
            return (steps, None);
        };
        steps.push(Step {
            level: k,
            table,
            idx,
            raw,
        });
        // A leaf above the target level, or a non-vidmem / invalid directory: the path ends.
        if d.leaf_ps != KF_PS_NONE && bit(raw, fmt.valid_bit) == 1 {
            return (steps, None);
        }
        let ap = aperture(fmt, raw);
        if ap == fmt.pde_ap_invalid
            || fmt.pde_ap_map.get(usize::from(ap)).copied() != Some(AP_VIDMEM)
        {
            return (steps, None);
        }
        table = target(fmt, raw);
    }
    (steps, Some(table))
}

/// (b) What a 4 KiB page looks like, from its 512 8-byte words.
#[must_use]
pub fn page_summary(fmt: &KfFormat, words: &[u64]) -> String {
    let mut zero = 0u32;
    let mut valid = 0u32;
    let mut consecutive = 0u32;
    let mut kinds: [u32; 4] = [0; 4]; // kind 0, kind 6, kind 7, other — the generic kinds
    let mut prev: Option<u64> = None;
    for &w in words {
        if w == 0 {
            zero = zero.saturating_add(1);
            prev = None;
            continue;
        }
        if bit(w, fmt.valid_bit) == 1 {
            valid = valid.saturating_add(1);
            let t = target(fmt, w);
            if prev.and_then(|p| p.checked_add(0x1000)) == Some(t) {
                consecutive = consecutive.saturating_add(1);
            }
            prev = Some(t);
            let slot = match field(w, fmt.kind) {
                0 => 0,
                6 => 1,
                7 => 2,
                _ => 3,
            };
            if let Some(k) = kinds.get_mut(slot) {
                *k = k.saturating_add(1);
            }
        } else {
            prev = None;
        }
    }
    // As 16-byte dual PDEs: the low word's valid bit clear and a non-invalid aperture in either
    // half (the high word's aperture sits at the same bit positions as the low word's, + 64).
    let mut dual = 0u32;
    for &[lo, hi] in words.as_chunks::<2>().0 {
        if bit(lo, fmt.valid_bit) == 0
            && (aperture(fmt, lo) != fmt.pde_ap_invalid || aperture(fmt, hi) != fmt.pde_ap_invalid)
        {
            dual = dual.saturating_add(1);
        }
    }
    format!(
        "words={} zero={zero} valid8={valid} consecutive4k={consecutive} kind[0]={} kind[6]={} kind[7]={} kind[other]={} dual16_present={dual}/{}",
        words.len(),
        kinds[0],
        kinds[1],
        kinds[2],
        kinds[3],
        words.len() / 2
    )
}

fn words_of(bytes: &[u8]) -> Vec<u64> {
    bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|c| u64::from_le_bytes(*c))
        .collect()
}

fn hexlist(words: &[u64]) -> String {
    words
        .iter()
        .map(|w| format!("{w:#018x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn wall() -> String {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}", d.as_secs(), d.subsec_millis())
}

/// A slot to re-read later (d).
#[derive(Debug, Clone)]
struct Watch {
    pdb: u64,
    va: u64,
    level: usize,
    last_table: Option<u64>,
    last_raw: Option<u64>,
    left: u32,
    at: Instant,
}

static REPORTS: AtomicU32 = AtomicU32::new(0);
static WATCH: Mutex<Vec<Watch>> = Mutex::new(Vec::new());

/// Reads the store: `read(off, buf)` fills `buf` from store offset `off`, false on refusal.
pub type ReadFn<'a> = &'a mut dyn FnMut(u64, &mut [u8]) -> bool;

fn read8_with(read: &mut dyn FnMut(u64, &mut [u8]) -> bool) -> impl FnMut(u64) -> Option<u64> + '_ {
    move |off| {
        let mut b = [0u8; 8];
        read(off, &mut b).then(|| u64::from_le_bytes(b))
    }
}

/// Dump the samples of one report (`pdb_of(entry)` names a sample's space). Returns the lines
/// (printed by the caller).
pub fn dump_report(
    fmt: &KfFormat,
    samples: &[KfRefusalSample],
    pdb_of: &dyn Fn(u16) -> Option<u64>,
    read: ReadFn<'_>,
) -> Vec<String> {
    let mut out = Vec::new();
    if samples.is_empty() || REPORTS.fetch_add(1, Ordering::Relaxed) >= DUMP_REPORTS {
        return out;
    }
    let w = wall();
    for s in samples.iter().take(DUMP_SAMPLES) {
        let pdb = pdb_of(s.entry);
        out.push(format!(
            "kf3: walkdump wall={w} space={} va={:#x} level={} bit={:#x} raw={:#018x} gpga={:#x} ps={:#x}: {}",
            pdb.map_or_else(|| "?".to_string(), |p| format!("{p:#x}")),
            s.va,
            s.level,
            s.bit,
            s.raw,
            s.gpga,
            s.ps_bytes,
            decode(fmt, s.raw)
        ));
        let level = usize::from(s.level);
        if let Some(pdb) = pdb {
            let (steps, table) = {
                let mut r8 = read8_with(&mut *read);
                path_to(fmt, pdb, s.va, level, &mut r8)
            };
            for st in &steps {
                out.push(format!(
                    "kf3: walkdump   path dir[{}] table={:#x} idx={:#x} raw={:#018x}: {}",
                    st.level,
                    st.table,
                    st.idx,
                    st.raw,
                    decode(fmt, st.raw)
                ));
            }
            // (c) the neighbours of the slot in its own table.
            if let (Some(t), Some(d)) = (table, fmt.dir.get(level)) {
                let idx = index_at(fmt, level, s.va);
                let eb = u64::from(d.entry_bytes);
                let half = (DUMP_ENTRIES / 2) as u64;
                let lo = idx
                    .saturating_sub(half)
                    .min(u64::from(d.entries).saturating_sub(DUMP_ENTRIES as u64));
                let mut buf = vec![0u8; DUMP_ENTRIES.saturating_mul(d.entry_bytes as usize)];
                let off = lo.checked_mul(eb).and_then(|o| t.checked_add(o));
                if let Some(off) = off
                    && read(off, &mut buf)
                {
                    let ws = words_of(&buf);
                    out.push(format!(
                        "kf3: walkdump   table dir[{level}] at {t:#x} slot {idx:#x} (entries {lo:#x}..): {}",
                        hexlist(&ws)
                    ));
                    let check = ws
                        .get(
                            usize::try_from(idx.saturating_sub(lo).saturating_mul(eb / 8))
                                .unwrap_or(usize::MAX),
                        )
                        .copied();
                    out.push(format!(
                        "kf3: walkdump   slot re-read now: {} (sample raw {:#018x})",
                        check.map_or_else(|| "?".to_string(), |c| format!("{c:#018x}")),
                        s.raw
                    ));
                }
                if let Ok(mut wl) = WATCH.lock()
                    && wl.len() < WATCHES
                    && !wl.iter().any(|x| x.pdb == pdb && x.va == s.va)
                {
                    wl.push(Watch {
                        pdb,
                        va: s.va,
                        level,
                        last_table: Some(t),
                        last_raw: Some(s.raw),
                        left: WATCH_REREADS,
                        at: Instant::now(),
                    });
                }
            }
        }
        // (b) the page the entry names, read the way the walk reads it.
        let page = s.gpga & !0xfffu64;
        let mut buf = vec![0u8; 4096];
        if read(page, &mut buf) {
            let ws = words_of(&buf);
            out.push(format!(
                "kf3: walkdump   target page {page:#x}: {}",
                page_summary(fmt, &ws)
            ));
            out.push(format!(
                "kf3: walkdump   target page {page:#x} first {DUMP_ENTRIES}: {}",
                hexlist(ws.get(..DUMP_ENTRIES).unwrap_or(&[]))
            ));
        } else {
            out.push(format!(
                "kf3: walkdump   target page {page:#x}: not readable from the store"
            ));
        }
    }
    out
}

/// (d) Re-read the watched slots that are due. Cheap when nothing is watched.
pub fn rewatch(fmt: &KfFormat, read: ReadFn<'_>) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(mut wl) = WATCH.lock() else { return out };
    if wl.is_empty() {
        return out;
    }
    let w = wall();
    for x in wl.iter_mut() {
        if x.left == 0 || x.at.elapsed() < Duration::from_millis(WATCH_GAP_MS) {
            continue;
        }
        x.left = x.left.saturating_sub(1);
        x.at = Instant::now();
        let (steps, table) = {
            let mut r8 = read8_with(&mut *read);
            path_to(fmt, x.pdb, x.va, x.level, &mut r8)
        };
        let raw = table.and_then(|t| {
            let d = fmt.dir.get(x.level)?;
            let off = index_at(fmt, x.level, x.va).checked_mul(u64::from(d.entry_bytes))?;
            let mut b = [0u8; 8];
            read(t.checked_add(off)?, &mut b).then(|| u64::from_le_bytes(b))
        });
        let above = steps
            .last()
            .map_or_else(|| "?".to_string(), |s| format!("{:#018x}", s.raw));
        out.push(format!(
            "kf3: walkdump watch wall={w} space={:#x} va={:#x} left={} above={above} table={} ({}) slot={} ({})",
            x.pdb,
            x.va,
            x.left,
            table.map_or_else(|| "none".to_string(), |t| format!("{t:#x}")),
            if table == x.last_table { "same" } else { "MOVED" },
            raw.map_or_else(|| "none".to_string(), |r| format!("{r:#018x}")),
            if raw == x.last_raw { "same" } else { "CHANGED" },
        ));
        x.last_table = table;
        x.last_raw = raw;
    }
    wl.retain(|x| x.left > 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_cuda::abi::KfDir;

    /// The VER2 descriptor as `kf_walk.cu kf_format_ver2` writes it (Ampere/Ada: PD1 leaf-capable).
    fn ver2() -> KfFormat {
        let f = |lo, bits, shift| KfField {
            lo,
            bits,
            shift,
            pad: 0,
        };
        let d = |active, va_lo, va_hi: u8, entry_bytes, leaf_ps| KfDir {
            active,
            va_lo,
            entry_bytes,
            leaf_ps,
            entries: 1u16 << (va_hi - va_lo + 1),
            pad: 0,
        };
        KfFormat {
            abi_version: kf_cuda::abi::KF_ABI_VERSION,
            table_version: kf_cuda::abi::KF_TBL_VER2,
            dir: [
                d(0, 0, 0, 8, KF_PS_NONE),
                d(1, 47, 48, 8, KF_PS_NONE),
                d(1, 38, 46, 8, KF_PS_NONE),
                d(1, 29, 37, 8, 3),
                d(1, 21, 28, 16, 2),
            ],
            big_va_lo: 16,
            small_va_lo: 12,
            big_entry_bytes: 8,
            small_entry_bytes: 8,
            big_entries: 32,
            small_entries: 512,
            big_ps: 1,
            small_ps: 0,
            root_align: 4096,
            first_dir: 1,
            sparse_twin: 0,
            pde_pcf_bits: 0,
            pde_sparse_ats: 0,
            valid_bit: 0,
            ap_lo: 1,
            ap_bits: 2,
            pde_ap_invalid: 0,
            pte_ap_map: [0, 1, 2, 3],
            pde_ap_map: [0xFF, 0, 2, 3],
            addr_sel: [0, 0, 1, 1],
            addr_local: f(8, 25, 12),
            addr_sys: f(8, 46, 12),
            big_addr_local: f(4, 29, 8),
            big_addr_sys: f(4, 50, 8),
            bit_volatile: 3,
            bit_privilege: 5,
            bit_read_only: 6,
            bit_atomic_disable: 7,
            pcf: f(0, 0, 0),
            pcf_sparse: 0,
            pad1: [0; 3],
            kind: f(56, 8, 0),
            ps_log2: [12, 16, 21, 29],
        }
    }

    #[test]
    fn decodes_the_run_2990_entries_both_ways() {
        let f = ver2();
        // [measured run 2990] raw=0x0600000000ee9601 level=3 gpga=0xee96000
        let l = decode(&f, 0x0600_0000_00ee_9601);
        assert!(l.contains("bit0=1 ap=0 vol=0"), "{l}");
        assert!(l.contains("kind=0x6"), "{l}");
        assert!(l.contains("target=0xee96000"), "{l}");
        assert!(
            l.contains("as-PTE ap->vid") && l.contains("as-PDE ap->INVALID"),
            "{l}"
        );
        // raw=0x060000001dc2f10f: valid, sys non-coherent, volatile
        let l = decode(&f, 0x0600_0000_1dc2_f10f);
        assert!(
            l.contains("bit0=1 ap=3 vol=1") && l.contains("target=0x1dc2f1000"),
            "{l}"
        );
    }

    #[test]
    fn path_walks_to_the_pd1_table_and_stops_on_a_non_directory() {
        let f = ver2();
        // root 0x1000 -> PD2 table 0x2000 -> PD1 table 0x3000 (vidmem PDEs: ap=1 => 0b010)
        let mut mem = std::collections::HashMap::new();
        let va = 0x0196_e000_0000_u64;
        let i1 = index_at(&f, 1, va);
        let i2 = index_at(&f, 2, va);
        mem.insert(0x1000 + i1 * 8, (0x2000u64 >> 12 << 8) | 0b010);
        mem.insert(0x2000 + i2 * 8, (0x3000u64 >> 12 << 8) | 0b010);
        let mut r8 = |o: u64| Some(mem.get(&o).copied().unwrap_or(0));
        let (steps, t) = path_to(&f, 0x1000, va, 3, &mut r8);
        assert_eq!(steps.len(), 2);
        assert_eq!(t, Some(0x3000));
        // An invalid PD2 entry ends the path with no table.
        let mut r0 = |_o: u64| Some(0u64);
        let (steps, t) = path_to(&f, 0x1000, va, 3, &mut r0);
        assert_eq!((steps.len(), t), (1, None));
    }

    #[test]
    fn summary_tells_a_4k_pte_table_from_a_dual_pde_table() {
        let f = ver2();
        // A 4 KiB PTE table: consecutive vidmem pages, kind 6, valid.
        let pt: Vec<u64> = (0..512u64)
            .map(|i| 0x0600_0000_0000_0001 | ((0xee93 + i) << 8))
            .collect();
        let s = page_summary(&f, &pt);
        assert!(
            s.contains("valid8=512")
                && s.contains("consecutive4k=511")
                && s.contains("kind[6]=512"),
            "{s}"
        );
        assert!(s.contains("dual16_present=0/256"), "{s}");
        // A PD0 table: dual entries, small half present (hi word aperture vidmem), bit0 clear.
        let mut pd0 = vec![0u64; 512];
        for e in pd0.as_chunks_mut::<2>().0 {
            e[1] = (0x4000u64 >> 12 << 8) | 0b010;
        }
        let s = page_summary(&f, &pd0);
        assert!(
            s.contains("valid8=0") && s.contains("dual16_present=256/256"),
            "{s}"
        );
    }

    #[test]
    fn dump_is_bounded_and_watch_rereads_are_bounded() {
        let f = ver2();
        let s = KfRefusalSample {
            va: 0x0196_e000_0000,
            raw: 0x0600_0000_00ee_9601,
            gpga: 0xee96000,
            ps_bytes: 0x2000_0000,
            bit: 0x400,
            level: 3,
            entry: 0,
        };
        let samples = vec![s; 10];
        let reads = std::cell::Cell::new(0u32);
        let mut read = |_o: u64, b: &mut [u8]| {
            reads.set(reads.get() + 1);
            b.fill(0);
            true
        };
        let lines = dump_report(&f, &samples, &|_| Some(0x1000), &mut read);
        // DUMP_SAMPLES samples, each: header + path(1: PD3 entry 0 => invalid) + target summary + first
        assert!(lines.len() <= DUMP_SAMPLES * 8, "{}", lines.len());
        assert!(reads.get() <= (DUMP_SAMPLES as u32) * 8, "{}", reads.get());
        let mut n = 0;
        for _ in 0..(REPORTS_FOR_TEST) {
            let _ = dump_report(&f, &samples, &|_| Some(0x1000), &mut read);
            n += 1;
        }
        assert!(n > 0);
        // After the cap no more report is dumped.
        assert!(dump_report(&f, &samples, &|_| Some(0x1000), &mut read).is_empty());
    }
    const REPORTS_FOR_TEST: u32 = DUMP_REPORTS;
}
