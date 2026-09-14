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

    /// ⊘ "We could not tell" is not "it does not fit". Collapsing them would license the
    /// wrong decision in whichever direction the collapse chose.
    #[test]
    fn an_unknown_host_aperture_yields_no_verdict_rather_than_false() {
        assert_eq!(verdict(HostBar1::Unknown("no device"), 1), None);
    }
}
