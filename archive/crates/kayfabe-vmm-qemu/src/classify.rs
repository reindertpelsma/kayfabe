//! ★★★ The **one** place a reported topology section is proven to be host RAM.
//!
//! `l2_qemu_adapter.md` §5.3's normative box, verbatim:
//!
//! ```text
//! Ram  ⟺  is_ram && !is_ram_device && !rom_device && !section.readonly
//!          && !section.nonvolatile
//! Device otherwise — including "we do not recognise it"
//! ```
//!
//! ## Why this is a module and not one line inside the listener
//!
//! Because the rule is **positive** — *prove RAM*, never *refuse the devices we thought
//! of* — and a positive rule decays by having a second evaluation site that forgets one
//! conjunct. The gate that would normally catch that (the GPA-accessor gate: exactly one
//! core-side classification site) is scoped to the pure crates and says nothing about an
//! adapter, so the containment here is structural: [`is_ram`] is the only function in
//! this crate that reads [`SectionFacts`] fields, and [`classify`] is the only caller of
//! [`is_ram`].
//!
//! ## The three directions the obvious test is wrong in
//!
//! §5.3 is a finding, and each direction has a positive case in the suite rather than a
//! comment here:
//!
//! 1. a **device-RAM** region reports itself RAM and may still be real MMIO;
//! 2. a **ROM-device** region reports itself RAM, serves reads directly, and sends
//!    **writes to callbacks** — memcpy-ing a guest write into one bypasses the owning
//!    device's write path;
//! 3. a **read-only** section is not a write target at all, and read-only is a
//!    first-class property of the *section*.
//!
//! Directions 1 and 3 have public predicates at the pinned version. Direction 2 does
//! **not**: the two public spellings both admit a ROM-device with its direct-read mode
//! turned off, which is still not memcpy-able. So the shim reads the field itself and
//! reports it as [`SectionFacts::is_rom_device`] — a deliberate reach into a foreign
//! struct, recorded in §5.3 and again here rather than hidden behind a helper that would
//! read as an API.

use kayfabe_vmm::RegionKind;

use crate::host::SectionFacts;

/// ★★★ **The check.** True only for plain host RAM.
///
/// Written as one conjunction rather than a chain of early returns so that a reader can
/// see the whole rule at once, and so that a mutation that deletes any single conjunct
/// changes the answer for exactly one of the suite's positive cases.
#[must_use]
pub fn is_ram(f: &SectionFacts) -> bool {
    f.is_ram && !f.is_ram_device && !f.is_rom_device && !f.readonly && !f.nonvolatile
}

/// The classification the topology listener declares a section with.
///
/// ★ Everything that is not proven RAM is declared [`RegionKind::Device`] — **declared,
/// not omitted**. A device guest-physical address must report
/// [`kayfabe_vmm::VmmError::NonRamGpa`] and not its near neighbour
/// [`kayfabe_vmm::VmmError::BadGpa`]; omitting the section would silently merge the two
/// (`testing_doctrine.md` §2 rule 3).
#[must_use]
pub fn classify(f: &SectionFacts) -> RegionKind {
    if is_ram(f) {
        RegionKind::Ram
    } else {
        RegionKind::Device
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★★ Every single-field deviation from plain RAM must classify as a device — the
    /// sweep §5.3 asks for, rather than the one witness a hand-written case gives.
    ///
    /// The loop mutates **one** field of `plain_ram()` at a time, which is what makes it
    /// a test of the conjunction rather than of any one conjunct: deleting a term from
    /// [`is_ram`] leaves four of the five rows passing and breaks exactly one, and the
    /// failure message names which.
    #[test]
    fn exactly_one_shape_is_ram_and_every_single_field_deviation_is_a_device() {
        let plain = SectionFacts::plain_ram();
        assert_eq!(
            classify(&plain),
            RegionKind::Ram,
            "plain host RAM is the ONE admitted shape; if this fails the rule admits \
             nothing at all and every other row below passes vacuously"
        );

        /// One named single-field departure from plain RAM.
        type Deviation = (&'static str, fn(&mut SectionFacts));
        let deviations: [Deviation; 5] = [
            ("is_ram cleared", |f| f.is_ram = false),
            ("a device-RAM region", |f| f.is_ram_device = true),
            ("a ROM-device region", |f| f.is_rom_device = true),
            ("a read-only section", |f| f.readonly = true),
            ("a non-volatile section", |f| f.nonvolatile = true),
        ];
        for (what, mutate) in deviations {
            let mut f = SectionFacts::plain_ram();
            mutate(&mut f);
            assert_eq!(
                classify(&f),
                RegionKind::Device,
                "{what}: the rule is PROVE RAM, so this section must be declared a \
                 device — admitting it is a memcpy into something that is not plain \
                 memory, and on three of these five that is a write the owning device \
                 was supposed to see"
            );
            assert!(
                !is_ram(&f),
                "{what}: `classify` and `is_ram` must never disagree — a second \
                 evaluation site is exactly the decay this module exists to prevent"
            );
        }
    }

    /// ★ The device shape the listener reports for an ordinary register window, asserted
    /// apart from the RAM shape, so `device()` cannot quietly become `plain_ram()`.
    #[test]
    fn an_ordinary_device_section_is_never_ram_however_its_other_flags_move() {
        // Sweep the four modifier flags over a section that is not RAM at all: none of
        // them may rescue it, because `is_ram` is a necessary conjunct.
        for ram_device in [false, true] {
            for rom_device in [false, true] {
                for readonly in [false, true] {
                    for nonvolatile in [false, true] {
                        let f = SectionFacts {
                            is_ram: false,
                            is_ram_device: ram_device,
                            is_rom_device: rom_device,
                            readonly,
                            nonvolatile,
                        };
                        assert_eq!(
                            classify(&f),
                            RegionKind::Device,
                            "a section that does not report itself RAM can never be \
                             classified RAM, whatever else is set ({f:?})"
                        );
                    }
                }
            }
        }
        assert_eq!(
            classify(&SectionFacts::device()),
            RegionKind::Device,
            "the named constructor must agree with the sweep"
        );
    }
}
