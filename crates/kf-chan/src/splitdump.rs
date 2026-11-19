//! ⚠ DIAGNOSTIC (`KF3_SPLIT_DUMP=1`, default OFF), 2026-11-19: the in-QEMU reboot hang (a guest
//! reboot inside one QEMU, then CUDA `cuCtxCreate` never returns). In that hang, every split walk of
//! UVM's root (`pdb 0x201000`) reported no run. This module records two things so a grep for
//! `SPLITDUMP` can tell which side is missing:
//!
//! - every Translated CE copy's DESTINATION, as the store/guest-RAM ranges it resolved to
//!   ([`record_copy`], called by [`crate::tmode`]), kept in a small ring;
//! - per split walk, what the store holds at the walked page-table page (formatted by
//!   [`page_line`]; the store read itself is the VA thread's, in `kf-qemu`).
//!
//! The DEVICE-RESET epoch ([`bump_epoch`]) is printed in every line, and each pdb is dumped at most
//! [`DUMPS_PER_PDB`] times per epoch ([`take_dump_slot`]).
//!
//! ⊘ With the variable unset, every entry point returns at its first line ([`on`] is false) and no
//! state is touched. The Translated plane's output does not depend on this module in either case:
//! [`record_copy`] only reads the resolved ranges.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// The env switch.
pub const ENV: &str = "KF3_SPLIT_DUMP";
/// Dumps per pdb per device-reset epoch.
pub const DUMPS_PER_PDB: u32 = 5;
/// Copies kept in the ring (the newest win).
pub const RING: usize = 64;
/// The range the reset hang's UVM page tables live in (store offsets `0x200000..0x210000`).
pub const PT_RANGE: (u64, u64) = (0x20_0000, 0x21_0000);

/// Whether the diagnostic is on (`KF3_SPLIT_DUMP=1`), read once.
#[must_use]
pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| env_says_on(std::env::var_os(ENV).as_deref()))
}

/// The switch's rule, separate so it is testable: exactly `"1"`.
#[must_use]
pub fn env_says_on(v: Option<&std::ffi::OsStr>) -> bool {
    v.is_some_and(|v| v == "1")
}

static EPOCH: AtomicU64 = AtomicU64::new(0);

/// The current device-reset epoch (0 before the first reset of this process).
#[must_use]
pub fn epoch() -> u64 {
    EPOCH.load(Ordering::Relaxed)
}

/// A device reset began: the next lines belong to a new epoch. No-op when off.
pub fn bump_epoch() {
    if on() {
        EPOCH.fetch_add(1, Ordering::Relaxed);
    }
}

/// One resolved destination run of a copy: `ram` (guest RAM) or the store, its offset and bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DstRun {
    /// Guest RAM (`true`) or the store (`false`).
    pub ram: bool,
    /// Offset in that backing.
    pub off: u64,
    /// Bytes.
    pub len: u64,
}

/// One recorded copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyRec {
    /// The epoch it was recorded in.
    pub epoch: u64,
    /// The copy's total destination bytes.
    pub bytes: u64,
    /// Its destination runs (at most 4 kept).
    pub dst: Vec<DstRun>,
}

static COPIES: Mutex<VecDeque<CopyRec>> = Mutex::new(VecDeque::new());

/// Record a Translated copy's resolved destination (`(ram, off, len)` per run). No-op when off.
pub fn record_copy(dst: impl IntoIterator<Item = (bool, u64, u64)>) {
    if !on() {
        return;
    }
    let runs: Vec<DstRun> = dst
        .into_iter()
        .map(|(ram, off, len)| DstRun { ram, off, len })
        .collect();
    let bytes = runs.iter().map(|r| r.len).sum();
    let rec = CopyRec {
        epoch: epoch(),
        bytes,
        dst: runs.into_iter().take(4).collect(),
    };
    if let Ok(mut q) = COPIES.lock() {
        if q.len() >= RING {
            q.pop_front();
        }
        q.push_back(rec);
    }
}

/// Take every copy recorded since the last call.
#[must_use]
pub fn take_copies() -> Vec<CopyRec> {
    COPIES
        .lock()
        .map(|mut q| q.drain(..).collect())
        .unwrap_or_default()
}

static DUMPS: Mutex<BTreeMap<(u64, u64), u32>> = Mutex::new(BTreeMap::new());

/// Whether `pdb` may be dumped once more in the current epoch; counts the dump when it may.
#[must_use]
pub fn take_dump_slot(pdb: u64) -> bool {
    take_dump_slot_in(&DUMPS, epoch(), pdb)
}

fn take_dump_slot_in(m: &Mutex<BTreeMap<(u64, u64), u32>>, epoch: u64, pdb: u64) -> bool {
    let Ok(mut m) = m.lock() else {
        return false;
    };
    let n = m.entry((epoch, pdb)).or_insert(0);
    if *n >= DUMPS_PER_PDB {
        return false;
    }
    *n += 1;
    true
}

/// What a page-table page holds: its non-zero 8-byte entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageSummary {
    /// Non-zero entries.
    pub nonzero: usize,
    /// Byte offset of the first non-zero entry.
    pub first_off: Option<usize>,
    /// The first four non-zero values.
    pub first: Vec<u64>,
}

/// Summarise `page` as little-endian 8-byte entries.
#[must_use]
pub fn summarize(page: &[u8]) -> PageSummary {
    let mut s = PageSummary {
        nonzero: 0,
        first_off: None,
        first: Vec::new(),
    };
    let (entries, _) = page.as_chunks::<8>();
    for (i, c) in entries.iter().enumerate() {
        let v = u64::from_le_bytes(*c);
        if v != 0 {
            s.nonzero += 1;
            s.first_off.get_or_insert(i * 8);
            if s.first.len() < 4 {
                s.first.push(v);
            }
        }
    }
    s
}

/// ⚠ A GUESS for diagnostics only, not a decoder kf relies on: where a version-3 GMMU PDE points
/// (aperture bits 2:1 = 1 for vidmem; address bits 51:12), if it names vidmem.
#[must_use]
pub fn pde_vidmem_target(raw: u64) -> Option<u64> {
    ((raw >> 1) & 0x3 == 1).then_some(raw & 0x000f_ffff_ffff_f000)
}

/// One `SPLITDUMP page` line.
#[must_use]
pub fn page_line(
    epoch: u64,
    what: &str,
    slot: Option<u32>,
    pdb: u64,
    at: u64,
    s: &PageSummary,
) -> String {
    let first: Vec<String> = s.first.iter().map(|v| format!("{v:#018x}")).collect();
    format!(
        "kf3: SPLITDUMP epoch={epoch} {what} slot={slot:?} pdb={pdb:#x} page={at:#x} nonzero={} first_off={} first=[{}]",
        s.nonzero,
        s.first_off.map_or("none".into(), |o| format!("{o:#x}")),
        first.join(",")
    )
}

/// One `SPLITDUMP copy` line: the copy's destination and whether it touches `PT_RANGE` or any of
/// `pages` (4 KiB store pages walked by this dump).
#[must_use]
pub fn copy_line(c: &CopyRec, pages: &[u64]) -> String {
    let hits =
        |r: &DstRun, lo: u64, hi: u64| !r.ram && r.off < hi && r.off.saturating_add(r.len) > lo;
    let pt = c.dst.iter().any(|r| hits(r, PT_RANGE.0, PT_RANGE.1));
    let walked = c
        .dst
        .iter()
        .any(|r| pages.iter().any(|&p| hits(r, p, p.saturating_add(0x1000))));
    let runs: Vec<String> = c
        .dst
        .iter()
        .map(|r| {
            format!(
                "{}+{:#x}/{:#x}",
                if r.ram { "ram" } else { "store" },
                r.off,
                r.len
            )
        })
        .collect();
    format!(
        "kf3: SPLITDUMP epoch={} copy bytes={:#x} dst=[{}] overlaps_0x200000_0x20ffff={pt} overlaps_walked_pages={walked}",
        c.epoch,
        c.bytes,
        runs.join(",")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⊘ Default OFF: unset or any value but "1" leaves the diagnostic off.
    #[test]
    fn only_exactly_one_turns_it_on() {
        assert!(!env_says_on(None));
        for v in ["", "0", "true", "yes", "11"] {
            assert!(!env_says_on(Some(std::ffi::OsStr::new(v))), "{v:?}");
        }
        assert!(env_says_on(Some(std::ffi::OsStr::new("1"))));
    }

    /// ⊘ With the variable unset (the test environment), recording and epochs are no-ops.
    #[test]
    fn off_records_nothing() {
        if on() {
            return; // a developer ran the suite with the diagnostic on
        }
        record_copy([(false, 0x20_1000, 8)]);
        bump_epoch();
        assert!(take_copies().is_empty());
        assert_eq!(epoch(), 0);
    }

    #[test]
    fn five_dumps_per_pdb_per_epoch() {
        let m = Mutex::new(BTreeMap::new());
        for _ in 0..DUMPS_PER_PDB {
            assert!(take_dump_slot_in(&m, 1, 0x20_1000));
        }
        assert!(!take_dump_slot_in(&m, 1, 0x20_1000));
        assert!(take_dump_slot_in(&m, 1, 0x20_0000), "another pdb");
        assert!(take_dump_slot_in(&m, 2, 0x20_1000), "the next epoch");
    }

    #[test]
    fn a_page_is_summarised_by_its_nonzero_entries() {
        let mut p = vec![0u8; 4096];
        assert_eq!(summarize(&p).nonzero, 0);
        p[0x10..0x18].copy_from_slice(&0x5f5a_5374_6e65_494eu64.to_le_bytes());
        p[0x800..0x808].copy_from_slice(&0x2023_0003u64.to_le_bytes());
        let s = summarize(&p);
        assert_eq!(s.nonzero, 2);
        assert_eq!(s.first_off, Some(0x10));
        assert_eq!(s.first, vec![0x5f5a_5374_6e65_494e, 0x2023_0003]);
        assert!(
            page_line(2, "root", Some(22), 0x20_1000, 0x20_1000, &s)
                .starts_with("kf3: SPLITDUMP epoch=2 root")
        );
    }

    #[test]
    fn copy_overlap_flags() {
        let c = CopyRec {
            epoch: 2,
            bytes: 0x20,
            dst: vec![DstRun {
                ram: false,
                off: 0x20_1ff0,
                len: 0x20,
            }],
        };
        let l = copy_line(&c, &[0x20_2000]);
        assert!(l.contains("overlaps_0x200000_0x20ffff=true"), "{l}");
        assert!(l.contains("overlaps_walked_pages=true"), "{l}");
        let ram = CopyRec {
            dst: vec![DstRun {
                ram: true,
                off: 0x20_1000,
                len: 8,
            }],
            ..c
        };
        assert!(copy_line(&ram, &[0x20_1000]).contains("overlaps_0x200000_0x20ffff=false"));
    }

    #[test]
    fn pde_guess_takes_vidmem_only() {
        assert_eq!(pde_vidmem_target(0x20_2000 | 0b010), Some(0x20_2000));
        assert_eq!(pde_vidmem_target(0x20_2000 | 0b100), None, "sysmem");
        assert_eq!(pde_vidmem_target(0), None);
    }
}
