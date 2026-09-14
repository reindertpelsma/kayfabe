//! ★★★★★ **THE BAR1 SIZING RELATION, QUERIED AND PRINTED** — `THE_CONSTRAINTS.md` §22 item 3.
//!
//! > **(b) The real relation is a SIZING constraint, not an impossibility:**
//! >
//! > ```text
//! > all-resident works  <=>  advertised_guest_BAR1 + our_headroom  <=  host_BAR1
//! > ```
//! >
//! > ⊘ **But we choose the left-hand side.** The guest's BAR1 aperture is **advertised by
//! > us**, not inherited: advertise 128 MiB and the same board leaves ~125 MiB of headroom.
//!
//! # ⊘ Why this is a CENSUS and not a refusal, today
//!
//! Nothing holds a BAR1 device view yet — increment 3 is blocked on decision (b)'s owner
//! ruling — so there is no consumption to bound and a refusal would refuse every boot for a
//! design that is not switched on. ⇒ It **reports**, so the number is on record *before* the
//! design depends on it, and so the ruling can be given against a measurement rather than
//! against an estimate.
//!
//! ⚠ **A check that reports is not a check that gates**, and this file is the former on
//! purpose. The moment BAR1 views exist, this becomes the budget: same query, same relation,
//! and `verdict()` already computes the answer.
//!
//! # ★★ §22(b)'s own rule: NOTHING MAY COMPARE AGAINST A LITERAL
//!
//! > *"256 MiB is ONE measured board, not a spec. BAR1 size varies by board and by whether the
//! > host enabled **ReBAR**; datacenter parts ship with a large BAR1 natively. ⇒ It is a
//! > **queryable property**."*
//!
//! So the host's BAR1 is **read from sysfs**, per boot, and there is no constant anywhere in
//! this file to compare against. ⊘ A board whose BAR1 cannot be read produces
//! [`HostBar1::Unknown`] and a census line that says so — never a default that happens to be
//! this bench's number.
//!
//! ⊘ **No device descriptor is involved.** `/sys/bus/pci/devices/*/resource` is a text file;
//! reading it is not what decision (b) is about, which is whether the VMM may hold an open
//! `/dev/nvidia<N>`. This file deliberately stays on the side of that line that needs no
//! ruling.

/// What the host's BAR1 aperture is, per boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostBar1 {
    /// Read from sysfs: this many bytes, on the PCI address it was read from.
    Bytes(u64),
    /// ⊘ Could not be determined. **Not a default** — the census says so by name, because a
    /// sizing verdict computed against a guessed host aperture is worse than no verdict.
    Unknown(&'static str),
}

/// NVIDIA's PCI vendor id, as sysfs spells it. ⊘ Not a BAR size — this is an identity, and
/// §22(b)'s "no literals" rule is about the *aperture*, which is queried below.
const NVIDIA_VENDOR: &str = "0x10de";

/// Parse one `/sys/bus/pci/devices/<addr>/resource` file and return BAR `n`'s length.
///
/// Each line is `start end flags`, in hex, one per BAR. A BAR that is not implemented reads
/// `0x0 0x0 0x0`, and its length is 0 — which is **not** an error and must not be reported as
/// one.
///
/// ⊘ Returns `None` only when the file does not have that many lines or a line will not parse:
/// a malformed answer is an absence, never a zero, because zero is a legitimate value here.
#[must_use]
pub fn bar_len_from_resource(text: &str, n: usize) -> Option<u64> {
    let line = text.lines().nth(n)?;
    let mut it = line.split_whitespace();
    let start = u64::from_str_radix(it.next()?.trim_start_matches("0x"), 16).ok()?;
    let end = u64::from_str_radix(it.next()?.trim_start_matches("0x"), 16).ok()?;
    if end < start {
        return None;
    }
    if start == 0 && end == 0 {
        return Some(0);
    }
    Some(end - start + 1)
}

/// ★ Query the host's BAR1 aperture, by scanning sysfs for an NVIDIA device.
///
/// ⊘ Takes the **largest** BAR1 it finds. A host with two NVIDIA devices has two, and the one
/// that bounds us is whichever the scratchpad isolate ends up on; taking the largest is the
/// optimistic direction, so the census can never claim less headroom than exists and then be
/// cited as a reason something is impossible. ⚠ When multi-GPU lands this must become
/// per-device, and the census line says which address it read.
#[must_use]
pub fn query_host_bar1() -> (HostBar1, String) {
    let Ok(dir) = std::fs::read_dir("/sys/bus/pci/devices") else {
        return (
            HostBar1::Unknown("/sys/bus/pci/devices is not readable"),
            String::new(),
        );
    };
    let mut best: Option<(u64, String)> = None;
    for e in dir.flatten() {
        let p = e.path();
        let Ok(vendor) = std::fs::read_to_string(p.join("vendor")) else {
            continue;
        };
        if vendor.trim() != NVIDIA_VENDOR {
            continue;
        }
        let Ok(res) = std::fs::read_to_string(p.join("resource")) else {
            continue;
        };
        // BAR1 is index 1 in the resource file, which is the BAR NUMBER and not the
        // "bus BAR" this tree also calls BAR1 — they agree here, and the name of the file
        // being indexed is what makes that checkable.
        if let Some(len) = bar_len_from_resource(&res, 1) {
            let addr = p
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if best.as_ref().is_none_or(|(b, _)| len > *b) {
                best = Some((len, addr));
            }
        }
    }
    match best {
        Some((0, addr)) => (
            HostBar1::Unknown("the NVIDIA device's BAR1 reads as unimplemented (0 bytes)"),
            addr,
        ),
        Some((len, addr)) => (HostBar1::Bytes(len), addr),
        None => (
            HostBar1::Unknown("no NVIDIA PCI device with a readable BAR1 was found"),
            String::new(),
        ),
    }
}

/// ★★★ **Our headroom** — what must be left over after the guest's aperture.
///
/// `[measured w722, GA106]` CUDA context creation itself needs ~3 MiB of BAR1, and with
/// 253 MiB held a host `cudaMalloc` fails with `initialization error`. ⇒ if the guest's
/// aperture eats the pool we lose the ability to launch the walk kernel **that publishes the
/// guest's mappings** — the deadlock shape §22 names.
///
/// ⊘ 16 MiB and not 3: the measured figure is what a context needs at its smallest, this is a
/// budget, and a budget sized to the measured minimum has no margin by construction. ⚠ It is a
/// number this file owns and can be wrong; it is stated here, once, so it is arguable.
pub const OUR_HEADROOM_BYTES: u64 = 16 * 1024 * 1024;

/// The verdict of §22(b)'s relation: `advertised + headroom <= host`.
///
/// ⊘ `None` when the host's BAR1 is unknown — **not** `false`. "We could not tell" and "it
/// does not fit" are different facts and a census that collapsed them would license the wrong
/// decision.
#[must_use]
pub fn verdict(host: HostBar1, advertised: u64) -> Option<bool> {
    match host {
        HostBar1::Unknown(_) => None,
        HostBar1::Bytes(h) => Some(advertised.saturating_add(OUR_HEADROOM_BYTES) <= h),
    }
}

/// The census line. ⊘ Printed on every boot, armed or not: the relation is a property of the
/// board and the advertised size, not of any gate, and it is wanted **before** anything
/// depends on it.
pub fn census(advertised_guest_bar1: u64) {
    let (host, addr) = query_host_bar1();
    let mib = |b: u64| b / (1024 * 1024);
    let (host_txt, verdict_txt) = match (host, verdict(host, advertised_guest_bar1)) {
        (HostBar1::Unknown(why), _) => (
            format!("UNKNOWN ({why})"),
            "⊘ NO VERDICT — the host aperture could not be read, and a sizing verdict against \
             a guessed one would be worse than none"
                .to_string(),
        ),
        (HostBar1::Bytes(h), Some(true)) => (
            format!("{} MiB at {addr}", mib(h)),
            format!(
                "✔ FITS — {} MiB advertised + {} MiB headroom ≤ {} MiB host, so an \
                 all-resident BAR1 is a sizing question this board answers yes to",
                mib(advertised_guest_bar1),
                mib(OUR_HEADROOM_BYTES),
                mib(h)
            ),
        ),
        (HostBar1::Bytes(h), Some(false)) => (
            format!("{} MiB at {addr}", mib(h)),
            format!(
                "⊘⊘ DOES NOT FIT — {} MiB advertised + {} MiB headroom > {} MiB host. ★ §22(b): \
                 we CHOOSE the left-hand side. Advertising {} MiB leaves {} MiB of headroom on \
                 this same board, and a {}-MiB-BAR1 GA106 is a real hardware configuration — a \
                 different truthful board, not a lie.",
                mib(advertised_guest_bar1),
                mib(OUR_HEADROOM_BYTES),
                mib(h),
                mib(h / 2),
                mib(h - h / 2),
                mib(h / 2),
            ),
        ),
        (HostBar1::Bytes(_), None) => unreachable!("Bytes always yields a verdict"),
    };
    eprintln!(
        "kayfabe: BAR1-BUDGET host_bar1={host_txt} advertised_guest_bar1={} MiB \
         headroom={} MiB ⇒ {verdict_txt}",
        mib(advertised_guest_bar1),
        mib(OUR_HEADROOM_BYTES),
    );
}

/// ★★★★★ **§w727 — BAR1 IS AN OPERATOR-SELECTABLE SIZE, LIKE VRAM.**
///
/// > **Owner, 2026-09-14:** *"just like VRAM size, where you can select how much vram to give
/// > to the guest, so can you select how much bar1/bar2 to give as option with also minimums
/// > set to function."*
///
/// `[measured w726/e36]` on the bench GA106 the shipped 256 MiB advertisement **does not fit**:
/// `256 + 16 > 256`. §22(b) already established that **we choose the guest's side**, so this is
/// the knob that makes the relation satisfiable rather than a refusal.
///
/// # ⚠ The three things §w727 says the knob must respect
///
/// 1. **PCI BAR sizes are POWERS OF TWO.** 64 / 128 / 256 MiB, never an arbitrary number — *"a
///    'select any size' knob that accepts 100 MiB is a bug the guest's enumeration finds, not
///    us."* [`Bar1Choice::parse`] refuses a non-power-of-two by name.
/// 2. **The minimum is a measurement, and it is PROVISIONAL.** `[measured]` the LLM workload's
///    BAR1 working set is **912 pages ≈ 3.6 MiB**, so 64 MiB has ~18x margin — *"but that is
///    ONE workload"*. [`BAR1_MIN_BYTES`] is set conservatively and says so.
/// 3. **Refuse at startup, loudly, never silently clamp.** ⊘ And *"do not gate on it before
///    anything consumes BAR1 views — a refusal that fires every boot for a design not yet
///    switched on is noise that teaches people to ignore the check."* ⇒ [`Bar1Choice::check`]
///    returns the verdict; the **caller** decides whether it refuses, and today only the armed
///    device-view path does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bar1Choice {
    /// The guest BAR1 aperture this boot advertises, in bytes.
    pub bytes: u64,
}

/// ★★ **The provisional minimum.** 64 MiB — the smallest power of two that clears the measured
/// 3.6 MiB working set by ~18x.
///
/// ⚠ **Provisional, and §w727 says so in as many words**: the measurement is *one workload*.
/// The justified minimum is whatever the census across the **guest suite** says, and until that
/// exists this number is conservative on purpose. ⊘ Do not lower it on the strength of another
/// single workload.
pub const BAR1_MIN_BYTES: u64 = 64 * 1024 * 1024;

/// Why a requested BAR1 size was refused. ⊘ Each arm names **which** rule it broke: "bad size"
/// is not actionable, and the operator's fix differs per arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bar1Refusal {
    /// Not a power of two. The guest's own PCI enumeration would find this before we did.
    NotPowerOfTwo(u64),
    /// Below [`BAR1_MIN_BYTES`] — too small for the driver to function.
    BelowMinimum {
        /// What was asked for.
        asked: u64,
        /// The floor.
        min: u64,
    },
    /// ★ It does not fit beside our own headroom in the host's aperture. Carries every term, so
    /// the operator can see which one to change.
    DoesNotFit {
        /// The guest aperture asked for.
        asked: u64,
        /// What this boot reserves for its own CUDA context and channels.
        headroom: u64,
        /// What the board actually has.
        host: u64,
        /// ★ The largest power of two that WOULD fit, or 0 if none does.
        largest_that_fits: u64,
    },
}

impl core::fmt::Display for Bar1Refusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mib = |b: u64| b / (1024 * 1024);
        match self {
            Bar1Refusal::NotPowerOfTwo(b) => write!(
                f,
                "a guest BAR1 of {} bytes is not a power of two. PCI BAR sizes are powers of \
                 two; a device that advertises anything else is one the guest's own \
                 enumeration rejects, and it would fail somewhere that looks nothing like \
                 this option. Choose 64, 128 or 256 MiB.",
                b
            ),
            Bar1Refusal::BelowMinimum { asked, min } => write!(
                f,
                "a guest BAR1 of {} MiB is below the {} MiB minimum. ⚠ That minimum is \
                 PROVISIONAL — it clears the one measured working set (912 pages ≈ 3.6 MiB) by \
                 ~18x, and the justified figure is whatever the guest suite's census says. \
                 Refused rather than clamped: a guest whose BAR is too small for its driver \
                 fails somewhere unrecognisable.",
                mib(*asked),
                mib(*min)
            ),
            Bar1Refusal::DoesNotFit {
                asked,
                headroom,
                host,
                largest_that_fits,
            } => write!(
                f,
                "a guest BAR1 of {} MiB does not fit: {} MiB + {} MiB headroom > {} MiB host \
                 aperture. ★ §22(b): we CHOOSE the guest's side, so this is an option to \
                 change, not a wall. {}",
                mib(*asked),
                mib(*asked),
                mib(*headroom),
                mib(*host),
                if *largest_that_fits == 0 {
                    "⊘ No power of two fits on this board at all — the headroom itself exceeds \
                     the aperture, which is a different problem."
                        .to_string()
                } else {
                    format!(
                        "The largest that fits here is {} MiB, and a {}-MiB-BAR1 GA106 is a \
                         real hardware configuration — a different truthful board, not a lie.",
                        mib(*largest_that_fits),
                        mib(*largest_that_fits)
                    )
                }
            ),
        }
    }
}

impl Bar1Choice {
    /// Parse an operator's choice, in **MiB**.
    ///
    /// # Errors
    /// [`Bar1Refusal::NotPowerOfTwo`] or [`Bar1Refusal::BelowMinimum`]. ⊘ Fit is **not**
    /// checked here: it depends on the board, and a parse that consulted the board could not
    /// be tested without one.
    pub fn parse(mib: u64) -> Result<Bar1Choice, Bar1Refusal> {
        let bytes = mib.saturating_mul(1024 * 1024);
        if bytes == 0 || !bytes.is_power_of_two() {
            return Err(Bar1Refusal::NotPowerOfTwo(bytes));
        }
        if bytes < BAR1_MIN_BYTES {
            return Err(Bar1Refusal::BelowMinimum {
                asked: bytes,
                min: BAR1_MIN_BYTES,
            });
        }
        Ok(Bar1Choice { bytes })
    }

    /// ★ Does this choice fit beside our headroom on this board?
    ///
    /// # Errors
    /// [`Bar1Refusal::DoesNotFit`], carrying every term **and** the largest power of two that
    /// would fit — so the message is a fix and not a complaint.
    ///
    /// ⊘ `Ok(())` when the host aperture is **unknown**: refusing a boot because we could not
    /// read sysfs would be the instrument deciding the experiment, and the census already says
    /// the verdict could not be computed.
    pub fn check(self, host: HostBar1) -> Result<(), Bar1Refusal> {
        let HostBar1::Bytes(h) = host else {
            return Ok(());
        };
        if self.bytes.saturating_add(OUR_HEADROOM_BYTES) <= h {
            return Ok(());
        }
        Err(Bar1Refusal::DoesNotFit {
            asked: self.bytes,
            headroom: OUR_HEADROOM_BYTES,
            host: h,
            largest_that_fits: largest_power_of_two_that_fits(h),
        })
    }
}

/// The largest power-of-two guest BAR1 that leaves [`OUR_HEADROOM_BYTES`] free in a `host`-byte
/// aperture, or `0` if none does.
///
/// ⊘ Walks **down** from the host's own size rather than computing it, because the answer must
/// also respect [`BAR1_MIN_BYTES`] — a "largest that fits" below the functional minimum is not
/// a suggestion anyone can take.
#[must_use]
pub fn largest_power_of_two_that_fits(host: u64) -> u64 {
    let mut c = host;
    while c >= BAR1_MIN_BYTES {
        if c.saturating_add(OUR_HEADROOM_BYTES) <= host {
            return c;
        }
        c /= 2;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape sysfs actually writes, taken verbatim from the bench box.
    const REAL: &str = "0x00000000c0000000 0x00000000c0ffffff 0x0000000000040200\n\
                        0x0000009800000000 0x000000980fffffff 0x000000000014220c\n\
                        0x0000000000000000 0x0000000000000000 0x0000000000000000\n\
                        0x0000009810000000 0x0000009811ffffff 0x000000000014220c\n";

    #[test]
    fn a_real_resource_file_parses_to_the_measured_apertures() {
        assert_eq!(bar_len_from_resource(REAL, 0), Some(16 * 1024 * 1024));
        // ★ 256 MiB — the number `bar1_simultaneous_view_ceiling.md` measured, arrived at here
        // by READING the board rather than by a constant.
        assert_eq!(bar_len_from_resource(REAL, 1), Some(256 * 1024 * 1024));
        assert_eq!(bar_len_from_resource(REAL, 3), Some(32 * 1024 * 1024));
    }

    /// ⊘ An unimplemented BAR is **0 bytes, not an error**. Reporting it as absent would make
    /// a board with a gap in its BARs look unreadable.
    #[test]
    fn an_unimplemented_bar_is_zero_and_not_an_absence() {
        assert_eq!(bar_len_from_resource(REAL, 2), Some(0));
    }

    /// ⊘ A malformed line is an **absence**, never a zero — because zero is a legitimate value
    /// here, so the two must not share a representation.
    #[test]
    fn a_malformed_line_is_an_absence_and_not_a_zero() {
        assert_eq!(bar_len_from_resource("garbage\n", 0), None);
        assert_eq!(bar_len_from_resource("0x10 0x0 0x0\n", 0), None, "end < start");
        assert_eq!(bar_len_from_resource(REAL, 9), None, "no such BAR");
    }

    /// ★★★ §22(b)'s relation, and the finding it produces on THIS board: 256 MiB advertised
    /// does **not** fit in a 256 MiB host aperture, and 128 MiB does.
    #[test]
    fn the_sizing_relation_is_what_section_22b_states() {
        let host = HostBar1::Bytes(256 * 1024 * 1024);
        assert_eq!(
            verdict(host, 256 * 1024 * 1024),
            Some(false),
            "advertising the whole host aperture leaves no headroom for our own CUDA context \
             — the deadlock shape §22 names, where the guest's views starve the mechanism that \
             publishes them"
        );
        assert_eq!(verdict(host, 128 * 1024 * 1024), Some(true));
    }

    // ── §w727: the sized option ──────────────────────────────────────────────────────

    /// ⚠ **PCI BAR sizes are POWERS OF TWO.** §w727: *"a 'select any size' knob that accepts
    /// 100 MiB is a bug the guest's enumeration finds, not us."*
    #[test]
    fn a_bar1_size_that_is_not_a_power_of_two_is_refused() {
        for mib in [100, 96, 3, 200, 0] {
            assert!(
                matches!(Bar1Choice::parse(mib), Err(Bar1Refusal::NotPowerOfTwo(_))),
                "{mib} MiB is not a power of two and must be refused"
            );
        }
        for mib in [64, 128, 256] {
            assert!(Bar1Choice::parse(mib).is_ok(), "{mib} MiB is a legal BAR size");
        }
    }

    /// ⊘ **Refused, never clamped.** A guest whose BAR is too small for its driver fails
    /// somewhere unrecognisable; silently growing it would hide which number was wrong.
    #[test]
    fn below_the_minimum_is_refused_rather_than_clamped() {
        let r = Bar1Choice::parse(32).expect_err("32 MiB is below the floor");
        assert!(matches!(r, Bar1Refusal::BelowMinimum { .. }));
        // ★ And the message says the floor is PROVISIONAL — one workload, not the suite.
        assert!(
            r.to_string().contains("PROVISIONAL"),
            "the refusal must say the minimum is provisional; it said: {r}"
        );
    }

    /// ★★★ **THE MEASURED SITUATION ON THE BENCH BOARD**: 256 MiB does not fit in a 256 MiB
    /// host aperture, and the refusal names 128 MiB as the fix rather than merely complaining.
    #[test]
    fn the_bench_board_refuses_256_and_names_128_as_the_fix() {
        let host = HostBar1::Bytes(256 * 1024 * 1024);
        let r = Bar1Choice::parse(256)
            .expect("256 MiB is a legal BAR size")
            .check(host)
            .expect_err("256 + 16 > 256, so it cannot fit");
        match r {
            Bar1Refusal::DoesNotFit {
                largest_that_fits, ..
            } => assert_eq!(
                largest_that_fits,
                128 * 1024 * 1024,
                "the refusal must carry the largest power of two that WOULD fit, so the \
                 message is a fix and not a complaint"
            ),
            other => panic!("wrong refusal: {other:?}"),
        }
        assert!(r.to_string().contains("128 MiB"));
        // ★ and 128 fits, which is the whole point of the knob existing
        assert!(Bar1Choice::parse(128).expect("legal").check(host).is_ok());
    }

    /// ⊘ An unknown host aperture does **not** refuse the boot. Refusing because we could not
    /// read sysfs would be the instrument deciding the experiment.
    #[test]
    fn an_unknown_host_aperture_does_not_refuse_a_choice() {
        assert!(
            Bar1Choice::parse(256)
                .expect("legal")
                .check(HostBar1::Unknown("no device"))
                .is_ok()
        );
    }

    /// ⊘ A board whose headroom exceeds its whole aperture has **no** answer, and
    /// `largest_that_fits` says `0` rather than suggesting something below the functional
    /// minimum — a suggestion nobody could take is worse than none.
    #[test]
    fn a_board_with_no_fitting_size_says_zero_rather_than_something_unusable() {
        assert_eq!(largest_power_of_two_that_fits(32 * 1024 * 1024), 0);
        assert_eq!(largest_power_of_two_that_fits(256 * 1024 * 1024), 128 * 1024 * 1024);
        // ⊘ A 1 GiB aperture fits 512 MiB, not 1 GiB: headroom is not optional.
        assert_eq!(largest_power_of_two_that_fits(1024 * 1024 * 1024), 512 * 1024 * 1024);
    }

    /// ⊘ "We could not tell" is not "it does not fit".    /// ⊘ "We could not tell" is not "it does not fit". Collapsing them would license the
    /// wrong decision in whichever direction the collapse chose.
    #[test]
    fn an_unknown_host_aperture_yields_no_verdict_rather_than_false() {
        assert_eq!(verdict(HostBar1::Unknown("no device"), 1), None);
    }
}
