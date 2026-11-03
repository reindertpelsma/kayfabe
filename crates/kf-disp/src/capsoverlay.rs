// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ⊘ **DIAGNOSTIC ONLY — never a shipped behaviour, and no boot needs it.** Lay words of THIS die's own
//! capabilities page, as `tools/read_display_caps_page.py` printed it (a root read-only BAR0 read on a box
//! that is ours), over the derived page, for a hardware bisection of what a closed guest display driver
//! needs (the method of `V3_DISPLAY_CAPS_DERIVATION_20261014.md` §3, AD104). A word found needed this way
//! goes into the product as a T1 rule (a named register: the virtual engine's own fact) or a T3 row
//! (`data/caps_measured.tsv`, an unnamed word), never as this overlay.
//!
//! Hostile input: the read file is operator input, the page is guest-visible. The parse is bounded
//! (at most 1024 words, 4-aligned offsets inside the 4 KiB page, one header naming the PCI id) and the
//! caller refuses a read whose PCI id is not the host GPU's own ([`PageRead::is_die`]): a neighbour's
//! page is never laid over (OWNER_RULINGS §AH).
//!
//! 2026-11-03, Blackwell Windows lane (`traces/blackwell_win_20261103/PREREG.md`).

use crate::caps::{CapsPage, PAGE};
use std::collections::BTreeMap;

/// A parsed page read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRead {
    /// PCI vendor id from the read's header.
    pub vendor: u16,
    /// PCI device id from the read's header.
    pub device: u16,
    /// Every non-zero word the read printed, `(offset, value)`.
    pub words: Vec<(u32, u32)>,
}

impl PageRead {
    /// Is this read of the PCI function whose sysfs `vendor`/`device` files say `vendor`/`device`?
    #[must_use]
    pub fn is_die(&self, vendor: u16, device: u16) -> bool {
        self.vendor == vendor && self.device == device
    }

    fn word(&self, off: u32) -> u32 {
        self.words
            .iter()
            .find(|(o, _)| *o == off)
            .map_or(0, |(_, v)| *v)
    }
}

fn hex(s: &str) -> Option<u32> {
    u32::from_str_radix(s.strip_prefix("0x")?, 16).ok()
}

/// Parse the text `tools/read_display_caps_page.py` prints.
///
/// # Errors
/// By name: no `# pci … vendor … device …` header, a malformed `WORD` line, an offset outside the page
/// or not 4-aligned, a repeated offset, or more than `PAGE / 4` words.
pub fn parse_read(text: &str) -> Result<PageRead, String> {
    let mut id = None;
    let mut words = BTreeMap::new();
    for (n, line) in text.lines().enumerate() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.first() == Some(&"#") && f.get(1) == Some(&"pci") {
            let at = |k: &str| {
                f.iter()
                    .position(|x| *x == k)
                    .and_then(|i| f.get(i + 1))
                    .and_then(|v| hex(v))
                    .and_then(|v| u16::try_from(v).ok())
            };
            id = Some((
                at("vendor").ok_or(format!("line {}: no vendor", n + 1))?,
                at("device").ok_or(format!("line {}: no device", n + 1))?,
            ));
        } else if f.first() == Some(&"WORD") {
            let (Some(off), Some(val), 3) = (
                f.get(1).and_then(|s| hex(s)),
                f.get(2).and_then(|s| hex(s)),
                f.len(),
            ) else {
                return Err(format!("line {}: malformed WORD line", n + 1));
            };
            if off % 4 != 0 || off as usize + 4 > PAGE {
                return Err(format!("line {}: offset {off:#x} outside the page", n + 1));
            }
            if words.insert(off, val).is_some() {
                return Err(format!("line {}: offset {off:#x} repeated", n + 1));
            }
            if words.len() > PAGE / 4 {
                return Err("more words than the page holds".into());
            }
        }
    }
    let (vendor, device) = id.ok_or("no '# pci … vendor … device …' header")?;
    Ok(PageRead {
        vendor,
        device,
        words: words.into_iter().collect(),
    })
}

/// The bisection groups, one bit each (offsets are the shared NVDisplay caps layout, `clc373.h` ..
/// `clca73.h`; a group is an operator's choice, it claims nothing).
pub const GROUPS: [(&str, u32); 8] = [
    ("sys", 1 << 0),      // 0x000 SYS_CAP, 0x004 SYS_CAPB
    ("bwexist", 1 << 1),  // 0x020 SYS_CAPC, 0x02c SYS_CAPD, 0x030 LINK_CAP, 0x034 SYS_CAPE (CA73)
    ("unnamed", 1 << 2),  // 0x008, 0x00c, 0x01c, 0x038..0x0ff
    ("ihub", 1 << 3),     // 0x010..0x01b, 0x024, 0x028
    ("sor", 1 << 4),      // 0x100..0x5e3, 0x608..0x67f
    ("headclk", 1 << 5),  // 0x5e4..0x607
    ("postcomp", 1 << 6), // 0x680..0x77f
    ("precomp", 1 << 7),  // 0x780..0xfff
];

/// The group bit of a page offset.
#[must_use]
pub fn group_of(off: u32) -> u32 {
    match off {
        0x000 | 0x004 => 1 << 0,
        0x020 | 0x02c | 0x030 | 0x034 => 1 << 1,
        0x008 | 0x00c | 0x01c | 0x038..=0x0ff => 1 << 2,
        0x010..=0x01b | 0x024 | 0x028 => 1 << 3,
        0x100..=0x5e3 | 0x608..=0x67f => 1 << 4,
        0x5e4..=0x607 => 1 << 5,
        0x680..=0x77f => 1 << 6,
        _ => 1 << 7,
    }
}

/// The derived `page` with every offset of the selected `groups` (and every offset in `extra`) set to
/// the read's word there (0 where the read printed none). `groups == u32::MAX` and no `extra` gives the
/// read page exactly.
#[must_use]
pub fn overlay(page: &CapsPage, read: &PageRead, groups: u32, extra: &[u32]) -> CapsPage {
    let mut w: BTreeMap<u32, u32> = page.words.iter().copied().collect();
    let mut offs: Vec<u32> = w.keys().copied().collect();
    offs.extend(read.words.iter().map(|(o, _)| *o));
    offs.extend(
        extra
            .iter()
            .copied()
            .filter(|o| o % 4 == 0 && (*o as usize) + 4 <= PAGE),
    );
    for off in offs {
        if group_of(off) & groups != 0 || extra.contains(&off) {
            w.insert(off, read.word(off));
        }
    }
    CapsPage {
        base: page.base,
        words: w.into_iter().filter(|(_, v)| *v != 0).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const READ: &str = "# kf-display-caps-page v1\n\
        # pci 0000:00:07.0 vendor 0x10de device 0x2c05 bar0 0x640000+0x1000 (PROT_READ, 32-bit loads)\n\
        DECISIVE 0x00c 0x00000102\n\
        WORD 0x000 0x00010f0f\n\
        WORD 0x00c 0x00000102\n\
        WORD 0x030 0x0000001f\n\
        WORD 0x144 0x73004310\n";

    fn derived() -> CapsPage {
        CapsPage {
            base: 0x0064_0000,
            words: vec![(0x000, 0xf0f), (0x018, 0x800), (0x144, 0x1000_0b00)],
        }
    }

    #[test]
    fn a_read_parses_with_its_pci_id() {
        let r = parse_read(READ).unwrap();
        assert!(r.is_die(0x10de, 0x2c05));
        assert!(
            !r.is_die(0x10de, 0x2b85),
            "a neighbour's id is not this die"
        );
        assert_eq!(r.words.len(), 4);
    }

    #[test]
    fn all_groups_give_the_read_page_exactly() {
        let r = parse_read(READ).unwrap();
        let p = overlay(&derived(), &r, u32::MAX, &[]);
        assert_eq!(p.words, r.words, "derived-only words (0x018) are cleared");
    }

    #[test]
    fn one_group_changes_only_its_offsets() {
        let r = parse_read(READ).unwrap();
        let p = overlay(&derived(), &r, 1 << 1, &[]);
        assert_eq!(p.word(0x030), 0x1f);
        assert_eq!(p.word(0x000), 0xf0f, "sys untouched");
        assert_eq!(p.word(0x144), 0x1000_0b00, "sor untouched");
        let q = overlay(&derived(), &r, 0, &[0x144, 0x018]);
        assert_eq!(q.word(0x144), 0x7300_4310);
        assert_eq!(q.word(0x018), 0, "an extra offset the read lacks is zeroed");
    }

    #[test]
    fn hostile_reads_are_refused_by_name() {
        assert!(parse_read("WORD 0x000 0x1\n").is_err(), "no header");
        let h = "# pci x vendor 0x10de device 0x2c05\n";
        assert!(
            parse_read(&format!("{h}WORD 0x002 0x1\n")).is_err(),
            "unaligned"
        );
        assert!(
            parse_read(&format!("{h}WORD 0x1000 0x1\n")).is_err(),
            "outside"
        );
        assert!(
            parse_read(&format!("{h}WORD 0x0 0x1\nWORD 0x0 0x2\n")).is_err(),
            "repeat"
        );
        assert!(parse_read(&format!("{h}WORD 0x0\n")).is_err(), "malformed");
        assert!(parse_read("# pci x vendor 0x10de\n").is_err(), "no device");
    }
}
