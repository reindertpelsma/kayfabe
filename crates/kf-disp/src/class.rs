//! ★ The display channel classes' methods, fields and caps registers — DERIVED from ogkm's class
//! headers (`tools/derive_display_classes.sh`), per family, never hand-typed.
//!
//! Lookups name the method the way the header does, without the class prefix: `method(0xC67E,
//! "SET_OFFSET")` reads `NVC67E_SET_OFFSET`. A name the TSV lacks is `None`, and the engine treats a
//! `None` as "this class does not have that method" — never as offset 0.

use std::collections::HashMap;
use std::sync::OnceLock;

/// One derived row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// A plain value.
    V(u64),
    /// A bit field `hi:lo`.
    F(u8, u8),
    /// `X(i)` = base + i*stride.
    A(u64, u64),
    /// `X(a,b)` = base + a*s1 + b*s2.
    A2(u64, u64, u64),
}

/// The class table of one ogkm version.
#[derive(Debug, Default)]
pub struct ClassTable {
    /// The ogkm version.
    pub version: String,
    rows: HashMap<String, Row>,
}

impl ClassTable {
    /// Parse a TSV produced by `tools/derive_display_classes.sh`.
    #[must_use]
    pub fn parse(tsv: &str) -> ClassTable {
        let mut t = ClassTable::default();
        for line in tsv.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            let n = |i: usize| f.get(i).and_then(|s| s.parse::<u64>().ok());
            let row = match f.first() {
                Some(&"VERSION") => {
                    t.version = f.get(1).map(|s| (*s).to_string()).unwrap_or_default();
                    continue;
                }
                Some(&"V") => n(2).map(Row::V),
                Some(&"F") => n(2).zip(n(3)).and_then(|(h, l)| Some(Row::F(u8::try_from(h).ok()?, u8::try_from(l).ok()?))),
                Some(&"A") => n(2).zip(n(3)).map(|(b, s)| Row::A(b, s)),
                Some(&"A2") => n(2).zip(n(3)).zip(n(4)).map(|((b, s1), s2)| Row::A2(b, s1, s2)),
                _ => None,
            };
            if let (Some(r), Some(name)) = (row, f.get(1)) {
                t.rows.insert((*name).to_string(), r);
            }
        }
        t
    }

    fn key(class: u32, name: &str) -> String {
        format!("NV{class:04X}_{name}")
    }

    /// A plain value `NV<class>_<name>` (a method offset or an enum value).
    #[must_use]
    pub fn v(&self, class: u32, name: &str) -> Option<u32> {
        match self.rows.get(&Self::key(class, name))? {
            Row::V(v) => u32::try_from(*v).ok(),
            _ => None,
        }
    }

    /// A field `NV<class>_<name>` as `(hi, lo)`.
    #[must_use]
    pub fn f(&self, class: u32, name: &str) -> Option<(u8, u8)> {
        match self.rows.get(&Self::key(class, name))? {
            Row::F(h, l) => Some((*h, *l)),
            _ => None,
        }
    }

    /// A one-parameter method `NV<class>_<name>(i)`.
    #[must_use]
    pub fn a(&self, class: u32, name: &str, i: u32) -> Option<u32> {
        match self.rows.get(&Self::key(class, name))? {
            Row::A(b, s) => u32::try_from(b + u64::from(i) * s).ok(),
            _ => None,
        }
    }

    /// A two-parameter method `NV<class>_<name>(a, b)`.
    #[must_use]
    pub fn a2(&self, class: u32, name: &str, a: u32, b: u32) -> Option<u32> {
        match self.rows.get(&Self::key(class, name))? {
            Row::A2(base, s1, s2) => u32::try_from(base + u64::from(a) * s1 + u64::from(b) * s2).ok(),
            _ => None,
        }
    }

    /// The index `i` if `method` is `NV<class>_<name>(i)` for `i < n`.
    #[must_use]
    pub fn index_of(&self, class: u32, name: &str, method: u32, n: u32) -> Option<u32> {
        match self.rows.get(&Self::key(class, name))? {
            Row::A(b, s) if *s > 0 => {
                let m = u64::from(method);
                (m >= *b && (m - b) % s == 0 && (m - b) / s < u64::from(n)).then(|| ((m - b) / s) as u32)
            }
            _ => None,
        }
    }

    /// A field of the class-less `NV_DISP_NOTIFIER` family (`__0_STATUS` …).
    #[must_use]
    pub fn notifier_field(&self, name: &str) -> Option<(u8, u8)> {
        match self.rows.get(&format!("NV_DISP_NOTIFIER{name}"))? {
            Row::F(h, l) => Some((*h, *l)),
            _ => None,
        }
    }

    /// A value of the class-less `NV_DISP_NOTIFIER` family.
    #[must_use]
    pub fn notifier_value(&self, name: &str) -> Option<u32> {
        match self.rows.get(&format!("NV_DISP_NOTIFIER{name}"))? {
            Row::V(v) => u32::try_from(*v).ok(),
            _ => None,
        }
    }
}

/// Extract field `(hi, lo)` from `v`.
#[must_use]
pub fn get(v: u32, (hi, lo): (u8, u8)) -> u32 {
    let width = u32::from(hi.saturating_sub(lo)) + 1;
    let mask = if width >= 32 { u32::MAX } else { (1u32 << width) - 1 };
    (v >> lo) & mask
}

/// Place `x` into field `(hi, lo)` of `v`.
#[must_use]
pub fn put(v: u32, (hi, lo): (u8, u8), x: u32) -> u32 {
    let width = u32::from(hi.saturating_sub(lo)) + 1;
    let mask = if width >= 32 { u32::MAX } else { (1u32 << width) - 1 };
    (v & !(mask << lo)) | ((x & mask) << lo)
}

/// The class table for a guest driver version, or `None` when it has not been derived.
#[must_use]
pub fn for_version(version: &str) -> Option<&'static ClassTable> {
    static V580_159_04: OnceLock<ClassTable> = OnceLock::new();
    match version {
        "580.159.04" => Some(V580_159_04.get_or_init(|| ClassTable::parse(include_str!("../data/classes-580.159.04.tsv")))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The derived C67x methods are the header's (`clc67d.h:80`, `clc67e.h`): UPDATE `0x200`, the
    /// window's ISO context DMA array at `0x240 + 4i`, OFFSET at `0x260 + 4i`, the notifier STATUS
    /// field `31:30`; Blackwell's window class has surface addresses instead of context DMAs.
    #[test]
    fn the_derived_methods_are_the_headers() {
        let t = for_version("580.159.04").expect("derived");
        assert_eq!(t.v(0xC67D, "UPDATE"), Some(0x200));
        assert_eq!(t.a(0xC67E, "SET_CONTEXT_DMA_ISO", 1), Some(0x244));
        assert_eq!(t.a(0xC67E, "SET_OFFSET", 0), Some(0x260));
        assert_eq!(t.index_of(0xC67E, "SET_OFFSET", 0x264, 6), Some(1));
        assert_eq!(t.notifier_field("__0_STATUS"), Some((31, 30)));
        assert_eq!(t.f(0xC67E, "SET_SIZE_HEIGHT"), Some((31, 16)));
        assert_eq!(t.a(0xC67D, "HEAD_SET_RASTER_SIZE", 1), Some(0x2064 + 0x400));
        assert!(t.a(0xCA7E, "SET_CONTEXT_DMA_ISO", 0).is_none(), "GB20x names surfaces by address");
        assert!(t.a(0xCA7E, "SET_SURFACE_ADDRESS_LO_ISO", 0).is_some());
        assert_eq!(get(0x0438_0780, (31, 16)), 0x438);
        assert_eq!(put(0, (31, 30), 2), 0x8000_0000);
    }
}
