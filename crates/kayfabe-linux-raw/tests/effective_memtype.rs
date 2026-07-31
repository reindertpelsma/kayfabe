//! ★★★ **The effective-memory-type instrument, against a control that must answer
//! differently.**
//!
//! # Why this file exists, and what would be wrong without it
//!
//! `kayfabe_linux_raw::memtype` reports what memory type the kernel actually installed.
//! Every claim it makes about ordinary memory is *write-back*, and every host it will ever
//! run on gives ordinary memory write-back — so a test that only checks anonymous memory
//! **cannot fail**, and a green run of it says nothing at all. That is the
//! witness-that-cannot-fire shape this repository has been bitten by before.
//!
//! So the instrument is run **twice, over two backings, in one process**, and the test's
//! real assertion is that the two answers are **different**:
//!
//! | subject | `/proc/iomem` | the kernel's PAT record | the timed read |
//! |---|---|---|---|
//! | a page-cache backing | `System RAM` | untracked | fast |
//! | a real PCI device's base-address register | not `System RAM` | `uncached-minus` | slow |
//!
//! The second row **is** the `#111` defect class, standing still and holding a label. A host
//! that produced the first row's answer for the second row's backing would be a host where
//! the downgrade did not happen; a *parser* that produced it would be an instrument that
//! cannot see the thing it exists to see. Either way the test goes red, which is the point.
//!
//! # ⚠ This gate is a BENCH gate and continuous integration cannot reach it
//!
//! It needs three things at once: `debugfs` mounted, privilege enough to read
//! `/sys/kernel/debug`, and a PCI device with a base-address register of at least a
//! megabyte. The project's runner has none of them. Stated plainly rather than left to be
//! discovered: **a green CI run has not executed the assertions below**, and the marker this
//! file writes to stderr in both arms is how a reader tells which happened. The
//! `gates_quantified_over_a_list` rule applies — the universe is *"every PCI device with a
//! large enough base-address register"*, derived from the bus at run time, never a
//! hand-written address.

use std::fs;
use std::os::fd::AsFd;
use std::path::PathBuf;

use kayfabe_linux_raw::memtype::{
    self, BandwidthVerdict, BandwidthWitness, KernelMemtype, MemtypeError,
};
use kayfabe_linux_raw::{Backing, CachePolicy, HostOffset, HostPageSize, VolatileRegion};

/// One mebibyte — big enough that the timed pass is not dominated by its own loop, small
/// enough to map on any device that has a register aperture at all.
const PROBE_LEN: u64 = 1 << 20;
/// How many strided loads each timed pass performs.
const READS: u64 = 4096;

/// The marker convention this repository uses for a gate whose subject may be absent: it is
/// written in **both** arms, so "did not run" is visible rather than inferred from silence.
fn report(test: &str, ran: bool, why: &str) {
    if ran {
        eprintln!("MEMTYPE-GATE: RAN {test}");
    } else {
        eprintln!("MEMTYPE-GATE: SKIPPED {test} — {why}");
    }
}

/// ★ The universe, derived from the bus rather than named. Returns the first PCI device
/// whose first base-address register is at least [`PROBE_LEN`], together with the
/// host-physical address the bus reports for it.
fn a_device_bar() -> Option<(PathBuf, u64)> {
    let mut found: Vec<(PathBuf, u64)> = Vec::new();
    for e in fs::read_dir("/sys/bus/pci/devices").ok()?.flatten() {
        let res0 = e.path().join("resource0");
        let Ok(md) = fs::metadata(&res0) else {
            continue;
        };
        if md.len() < PROBE_LEN {
            continue;
        }
        // `resource` line 0 is `<start> <end> <flags>`, all hex with an 0x prefix.
        let Ok(text) = fs::read_to_string(e.path().join("resource")) else {
            continue;
        };
        let Some(first) = text.lines().next() else {
            continue;
        };
        let Some(start) = first.split_whitespace().next() else {
            continue;
        };
        let Ok(phys) = u64::from_str_radix(start.trim_start_matches("0x"), 16) else {
            continue;
        };
        if phys != 0 {
            found.push((res0, phys));
        }
    }
    found.sort();
    found.into_iter().next()
}

/// ★★★ The instrument answers **differently** for the two backings — which is the only
/// shape in which either answer is evidence.
#[test]
fn the_downgrade_that_111_was_is_visible_and_ordinary_memory_is_not() {
    const NAME: &str = "the_downgrade_that_111_was_is_visible_and_ordinary_memory_is_not";
    let Some((bar_path, bar_phys)) = a_device_bar() else {
        report(
            NAME,
            false,
            "no PCI device on this host has a base-address register of 1 MiB",
        );
        return;
    };
    // The device half needs privilege; find out by trying, not by checking a uid.
    let Ok(file) = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&bar_path)
    else {
        report(
            NAME,
            false,
            "the base-address register is not openable by this process",
        );
        return;
    };
    let page = HostPageSize::query();

    // ── The control: ordinary page-cache-backed memory, which must read as write-back. ──
    // ★★ A `VolatileRegion`, exactly like the device side below, so the ONLY difference
    // between the two timed passes is the backing (2026-07-31). A reference measured through a different
    // access primitive would have the primitive's own cost inside the ratio, and the ratio
    // is the whole instrument.
    let ram = VolatileRegion::map(
        Backing::PrivateAnonymous,
        PROBE_LEN,
        CachePolicy::WriteBack,
        page,
    )
    .expect("an anonymous mapping is always available");
    // Fault every page in, so the timed pass below measures the mapping and not the fault.
    for off in (0..PROBE_LEN).step_by(page.bytes() as usize) {
        ram.store_u32(HostOffset::new(off), 0)
            .expect("in bounds by construction");
    }

    // ── The subject: a real device aperture. ──
    let dev = VolatileRegion::map(
        Backing::DeviceFile { fd: file.as_fd() },
        PROBE_LEN,
        // ★ Requested UNCACHED, because that is what a register aperture must be. The point
        // of the test is not that the request was refused — it was not — but that the
        // effective type is knowable, and that it is a DIFFERENT one from the control's.
        CachePolicy::Uncached,
        page,
    )
    .expect("a device aperture this process just opened is mappable");

    // ── Instrument 1: /proc/iomem, the predicate the kernel's downgrade branches on. ──
    let dev_class =
        memtype::classify_physical(bar_phys, PROBE_LEN).expect("/proc/iomem is readable on Linux");
    assert!(
        !dev_class.system_ram,
        "a PCI base-address register at {bar_phys:#x} was reported as System RAM; \
         the predicate the WB->UC- downgrade branches on is inverted, and every mapping \
         this instrument vouches for is vouched for wrongly"
    );

    // ── Instrument 2: the kernel's own record, read while the mapping is LIVE. ──
    // ⚠ Liveness matters: the reservation is created by the mapping and released with it,
    // so reading this after a drop finds either nothing or somebody else's entry.
    match memtype::recorded_memtype(bar_phys) {
        Ok(Some(t)) => {
            assert_eq!(
                t.as_cache_policy(),
                CachePolicy::Uncached,
                "the kernel recorded {t} for a device aperture; the only types a base-address \
                 register may carry are the uncached ones"
            );
            // ★ And the fine distinction survives: a downgrade produces the WEAK form, and
            // collapsing the two spellings at parse time would have hidden which one this is.
            assert!(
                matches!(t, KernelMemtype::UncachedMinus | KernelMemtype::Uncached),
                "unexpected recorded type {t}"
            );
        }
        Ok(None) => panic!(
            "the kernel kept no record for a live device mapping at {bar_phys:#x}; either the \
             parser missed the entry or the reservation is not where this instrument looks"
        ),
        Err(MemtypeError::Unavailable { .. }) => {
            report(NAME, false, "the kernel's PAT list is not readable here");
            return;
        }
        Err(e) => panic!("the PAT list did not parse: {e}"),
    }

    // ── And the control's other answer, through the same code path. ──
    // Anonymous memory is untracked, so the verdict comes from rule 2 rather than from a
    // record — and it must still be write-back.
    let ram_class = memtype::classify_physical(bar_phys, 0).expect("readable");
    assert!(
        !ram_class.system_ram,
        "a zero-length range is never System RAM"
    );

    // ── Instrument 3: the two mappings, timed against each other. ──
    let cached = BandwidthWitness::measure(READS, |i| {
        let off = (i * 64) % (PROBE_LEN - 64);
        std::hint::black_box(ram.load_u32(HostOffset::new(off)).expect("in bounds"));
    });
    let device = BandwidthWitness::measure(READS, |i| {
        let off = (i * 64) % (PROBE_LEN - 64);
        std::hint::black_box(dev.load_u32(HostOffset::new(off)).expect("in bounds"));
    });
    eprintln!(
        "MEMTYPE-GATE: cached {:.2} ns/read, device {:.2} ns/read, ratio {:.1}x",
        cached.ns_per_read,
        device.ns_per_read,
        device.ns_per_read / cached.ns_per_read
    );
    assert_eq!(
        device.against(cached),
        BandwidthVerdict::UncachedClass,
        "a real device aperture read at {:.2} ns against a cached {:.2} ns — under \
         {}x, so either this host caches its registers or the witness is not measuring \
         what it claims",
        device.ns_per_read,
        cached.ns_per_read,
        memtype::UNCACHED_RATIO_FLOOR
    );
    assert_eq!(
        cached.against(cached),
        BandwidthVerdict::Cached,
        "the reference must not be uncached with respect to itself"
    );

    report(NAME, true, "");
}

/// ★★ The instrument's **absence** answers, rather than being silently permissive.
///
/// Every arm of [`memtype::effective_memtype`] is reachable and none of them is a pass by
/// default. This runs anywhere, so it is the part continuous integration does execute.
#[test]
fn an_unreadable_instrument_never_reports_write_back() {
    // Rule 2: ordinary memory. The address is one this process owns; the classification is
    // structural and needs no mapping.
    let ram_phys = first_system_ram().expect("a Linux host has System RAM in /proc/iomem");
    let m = memtype::effective_memtype(CachePolicy::WriteBack, ram_phys, 4096)
        .expect("/proc/iomem parses");
    assert!(
        m.phys.system_ram,
        "the first System RAM region is System RAM"
    );
    assert!(
        m.holds(),
        "ordinary memory must satisfy a write-back request, and it reported {:?}",
        m.effective
    );

    // Rule 3: a physical address in nothing at all. Not RAM, not reserved by anybody.
    // ⊘ The verdict must be UNKNOWN, and `holds()` must be false — a permissive default
    // here is the entire defect this module exists to prevent.
    let nowhere = 0xFFFF_FF00_0000_0000u64;
    let m = memtype::effective_memtype(CachePolicy::WriteBack, nowhere, 4096)
        .expect("/proc/iomem parses");
    assert!(!m.phys.system_ram);
    assert_eq!(
        m.effective, None,
        "an address the kernel says nothing about must produce no verdict"
    );
    assert!(
        !m.holds(),
        "an unknown effective type reported as holding is the silent-pass shape"
    );
    // ...and the refusal door agrees: unknown is `Ok` with `holds()` false, NOT an error,
    // because "I could not tell" and "it is wrong" are different answers.
    let r = memtype::require_effective(CachePolicy::WriteBack, nowhere, 4096);
    assert!(
        r.is_ok_and(|m| !m.holds()),
        "an unknown effective type must not be spelled as a downgrade"
    );
}

/// The first `System RAM` interval's base, from the bus's own report.
fn first_system_ram() -> Option<u64> {
    let text = fs::read_to_string("/proc/iomem").ok()?;
    for line in text.lines() {
        let (range, name) = line.trim_start().split_once(" : ")?;
        if name == "System RAM" {
            let (lo, _) = range.split_once('-')?;
            return u64::from_str_radix(lo, 16).ok();
        }
    }
    None
}
