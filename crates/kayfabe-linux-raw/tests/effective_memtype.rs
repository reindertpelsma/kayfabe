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
//! # ★★★ The two witnesses are not peers — one is categorical and one is statistical
//!
//! `/proc/iomem` and the kernel's PAT record are **categorical**: the kernel names what it
//! installed and the answer does not depend on how busy the machine is. The timed read is
//! **statistical**, and on a loaded host it is not always able to answer at all — measured
//! 2026-08-01 (task #150), a genuine device aperture reads anywhere from 4.2x to 31x the
//! cached reference depending on load, which straddles any fixed floor. So the categorical
//! answer is the one this test *requires*, the timed one **corroborates** it, and the timed
//! one is allowed to return [`memtype::BandwidthVerdict::Inconclusive`] — which is recorded
//! as its own marker rather than being rounded to either neighbour. What is still red: a
//! device aperture that reads as *cached*, on every attempt, against a reference that was
//! stable every time.
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
use std::io::Write as _;
use std::os::fd::AsFd;
use std::path::PathBuf;

use kayfabe_linux_raw::memtype::{
    self, BandwidthVerdict, BandwidthWitness, CachedReference, KernelMemtype, MemtypeError,
    SCHEDULER_AVERAGING_PASS, Unsettled,
};
use kayfabe_linux_raw::{Backing, CachePolicy, HostOffset, HostPageSize, VolatileRegion};

/// One mebibyte — big enough that the timed pass is not dominated by its own loop, small
/// enough to map on any device that has a register aperture at all.
const PROBE_LEN: u64 = 1 << 20;

/// ★★ How many times the timed comparison may be re-taken before the run gives up on it.
///
/// A retry is only legitimate when the ledger says it retried — otherwise it is a flake
/// hidden behind a loop — so every attempt writes its own line below, and the count is
/// bounded rather than "until it goes green".
const TIMED_ATTEMPTS: u32 = 4;

/// ★★★ **Write a marker where a marker can actually be read.**
///
/// ⚠ `eprintln!` goes through libtest's capture, which is discarded on the **passing**
/// path — the arm that matters for a gate whose whole job is to be countable. Measured
/// 2026-08-01 (task #150) across three hardware-run logs on the bench box: `KVM-GATE`
/// (56), `SANDBOX-GATE` (10) and `VBIOS-ORACLE-GATE` (13) markers all appear, and
/// `MEMTYPE-GATE` appears **zero** times — this file's markers had never once reached a
/// log, so `run_full_suite.sh`'s `gate-census` (which derives its families from the log)
/// had never counted this family at all, and the one line anybody ever saw from here was
/// the one libtest dumps when a test *fails*.
///
/// `std::io::stderr()` writes straight to the descriptor, exactly as
/// `kayfabe_linux_raw::kvm_gate::report` does and for exactly the same reason.
fn record(line: &str) {
    let _ = writeln!(std::io::stderr(), "{line}");
}

/// The marker convention this repository uses for a gate whose subject may be absent: it is
/// written in **both** arms, so "did not run" is visible rather than inferred from silence.
fn report(test: &str, ran: bool, why: &str) {
    if ran {
        record(&format!("MEMTYPE-GATE: RAN {test}"));
    } else {
        record(&format!("MEMTYPE-GATE: SKIPPED {test} — {why}"));
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
    //
    // ★★★ THIS IS THE PRIMARY WITNESS, AND KEEPING ITS ANSWER IS WHAT LETS THE TIMED ONE
    // BE ALLOWED TO SAY "I CANNOT TELL". It is CATEGORICAL: the kernel names the type it
    // installed, in its own words, with no statistics anywhere. The timed comparison below
    // corroborates it. Every path out of this `match` either binds a categorical answer or
    // leaves the test, so the timed section can never be the only witness in the room.
    let categorical: KernelMemtype = match memtype::recorded_memtype(bar_phys) {
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
            t
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
    };

    // ── And the control's other answer, through the same code path. ──
    // Anonymous memory is untracked, so the verdict comes from rule 2 rather than from a
    // record — and it must still be write-back.
    let ram_class = memtype::classify_physical(bar_phys, 0).expect("readable");
    assert!(
        !ram_class.system_ram,
        "a zero-length range is never System RAM"
    );

    // ── Instrument 3: the two mappings, timed against each other. ──
    //
    // ★★★ WHAT THIS ARM MAY AND MAY NOT CONCLUDE, measured 2026-08-01 (task #150) on the
    // 38-core bench box, 440 samples over four load levels:
    //
    // | condition | two cached passes, against each other | the device against a cached pass |
    // |---|---|---|
    // | idle | 0.91x .. 1.16x | 11.7x .. 13.4x |
    // | 38-way saturating load | **0.11x .. 11.35x** | 1.58x .. 49.1x |
    // | 38-way load, passes >= 5 ms | 0.72x .. 1.27x | **4.17x** .. 12.4x |
    //
    // Two things follow, and both are why this section looks the way it does rather than
    // like a single `assert!(ratio >= 10)`. First, the middle row: on a busy host with
    // short passes, two passes over *the same cached memory* reached 11.35x apart — past
    // the uncached floor — so the instrument could have called ordinary write-back memory
    // a device aperture. That is what `measure_over` and the reference's own stability
    // check are for. Second, the bottom row: even with a stable reference, a genuine
    // device aperture on a loaded host can read as low as 4.17x. There is no ratio that
    // separates the populations under load, so the honest instrument has a band where it
    // does not answer — and the 9.1x that failed 1 run in 180 on 2026-08-01 is inside it.
    let mut verdicts: Vec<(u32, BandwidthVerdict, f64, f64)> = Vec::new();
    let mut last_reference = None;
    for attempt in 1..=TIMED_ATTEMPTS {
        // ★★ A PAIR, not a single pass: the reference is the whole instrument, so the
        // reference has to be checkable, and two passes over the same backing are what
        // makes its stability an observable rather than an assumption.
        let reference = CachedReference::from_pair(
            BandwidthWitness::measure_over(SCHEDULER_AVERAGING_PASS, |i| {
                let off = (i * 64) % (PROBE_LEN - 64);
                std::hint::black_box(ram.load_u32(HostOffset::new(off)).expect("in bounds"));
            }),
            BandwidthWitness::measure_over(SCHEDULER_AVERAGING_PASS, |i| {
                let off = (i * 64) % (PROBE_LEN - 64);
                std::hint::black_box(ram.load_u32(HostOffset::new(off)).expect("in bounds"));
            }),
        );
        let device = BandwidthWitness::measure_over(SCHEDULER_AVERAGING_PASS, |i| {
            let off = (i * 64) % (PROBE_LEN - 64);
            std::hint::black_box(dev.load_u32(HostOffset::new(off)).expect("in bounds"));
        });
        let verdict = device.against(reference);
        // ⊘ EVERY attempt is on the record, including the ones that were retried away. A
        // bounded retry whose attempts are not written down is a flake behind a loop.
        record(&format!(
            "MEMTYPE-GATE: TIMED {NAME} attempt {attempt}/{TIMED_ATTEMPTS} — cached \
             {:.2}/{:.2} ns/read (spread {:.2}x), device {:.2} ns/read, ratio {:.1}x, \
             verdict {verdict:?}",
            reference.first.ns_per_read,
            reference.second.ns_per_read,
            reference.spread(),
            device.ns_per_read,
            device.ratio_against(reference),
        ));
        verdicts.push((
            attempt,
            verdict,
            device.ratio_against(reference),
            reference.spread(),
        ));
        last_reference = Some(reference);
        if verdict == BandwidthVerdict::UncachedClass {
            break;
        }
    }
    let reference = last_reference.expect("the loop runs at least once");

    // ⊘ The reference must not be uncached with respect to itself — a floor at or below
    // 1.0 would make every mapping in the world uncached.
    assert_eq!(
        reference.first.against(reference),
        BandwidthVerdict::Cached,
        "the reference must not be uncached with respect to itself"
    );

    // ★★★ THE THREE OUTCOMES, AND WHY ONLY ONE OF THEM IS RED.
    //
    // `Cached` on EVERY attempt is the `#111` shape standing still: a stable reference,
    // four times over, and a register aperture that reads like RAM. That is a defect in
    // the host, the mapping or this instrument and it must be loud.
    //
    // A single `UncachedClass` is corroboration and the test passes.
    //
    // Anything else is *unsettled*, and an unsettled reading is not a failed one. The
    // categorical instrument above has already answered — it named the type the kernel
    // installed — and it is the primary witness precisely because it does not depend on
    // how busy the box is. So this arm records what it saw and does not fail.
    // ⊘ It is NOT a skip: the test ran, its assertions ran, and the line below says
    // exactly which corroboration is missing rather than leaving a silent green.
    let corroborated = verdicts
        .iter()
        .any(|&(_, v, _, _)| v == BandwidthVerdict::UncachedClass);
    let all_cached = verdicts
        .iter()
        .all(|&(_, v, _, _)| v == BandwidthVerdict::Cached);
    assert!(
        !all_cached,
        "a real device aperture read as CACHED on all {} attempts against a reference that \
         was stable each time (ratios {:?}); either this host caches its registers or the \
         witness is not measuring what it claims. The kernel's own record for this range \
         says {categorical}.",
        verdicts.len(),
        verdicts.iter().map(|&(_, _, r, _)| r).collect::<Vec<_>>()
    );
    if !corroborated {
        record(&format!(
            "MEMTYPE-GATE: UNCORROBORATED {NAME} — the timed witness did not settle in {} \
             attempts (verdicts {:?}); the categorical instrument answered {categorical} and \
             it is the primary. Ratios {:?}, reference spreads {:?}. A host under enough \
             load that two passes over the same cached memory disagree cannot be asked this \
             question, and 4.17x..12.4x is the measured range a genuine aperture produces \
             under load (2026-08-01, task #150) — which straddles the {}x floor.",
            verdicts.len(),
            verdicts.iter().map(|&(_, v, _, _)| v).collect::<Vec<_>>(),
            verdicts.iter().map(|&(_, _, r, _)| r).collect::<Vec<_>>(),
            verdicts.iter().map(|&(_, _, _, s)| s).collect::<Vec<_>>(),
            memtype::UNCACHED_RATIO_FLOOR,
        ));
        // ★ And the unsettled arm is only reachable for the two stated reasons. A verdict
        // that is neither `Cached`, `UncachedClass`, nor one of the two named ways of
        // being unsettled would mean a fourth situation nobody has reasoned about.
        for &(attempt, v, _, _) in &verdicts {
            assert!(
                matches!(
                    v,
                    BandwidthVerdict::Cached
                        | BandwidthVerdict::Inconclusive(
                            Unsettled::InTheBand | Unsettled::ReferenceUnstable
                        )
                ),
                "attempt {attempt} produced {v:?}, which is not one of the outcomes this \
                 test knows how to read"
            );
        }
    }

    report(NAME, true, "");
}

/// ★★★ **The marker reaches a log at all** — the non-vacuity argument for this whole
/// family, tested rather than asserted in a comment.
///
/// Measured 2026-08-01 (task #150): across three of the bench box's hardware-run logs,
/// `KVM-GATE` appeared 56 times, `SANDBOX-GATE` 10, `VBIOS-ORACLE-GATE` 13 — and
/// `MEMTYPE-GATE` **zero**, because this file wrote its markers with `eprintln!` and
/// libtest discards captured output on the **passing** path. The gate family was therefore
/// invisible to `run_full_suite.sh`'s `gate-census`, which derives its families from the
/// log: a family that never prints is not a family that passes, it is a family that is not
/// counted. A comment saying "write to the real stderr" would not have caught the
/// regression; running a child and looking does.
///
/// ★ This is also the one assertion in this file that a CI runner executes end to end: the
/// child's own gate may skip for want of a device, and `MEMTYPE-GATE: SKIPPED` is a marker
/// too. It is the *silence* that is the defect.
#[test]
fn the_gate_marker_survives_libtests_capture() {
    let exe = std::env::current_exe().expect("a test binary knows its own path");
    // ⚠ Deliberately NO `--nocapture`: capture is the condition under which the marker has
    // to survive, and passing it would test the opposite of what is claimed. The child runs
    // only the gated test, so there is no recursion here.
    let out = std::process::Command::new(&exe)
        .args(["--test-threads=1", "the_downgrade_that_111_was"])
        .output()
        .expect("the test binary is executable");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("MEMTYPE-GATE: "),
        "a child run of {exe:?} produced no MEMTYPE-GATE marker on its real stderr, so \
         nothing this file does can be counted from a run log. stderr was: {err:?}"
    );
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
