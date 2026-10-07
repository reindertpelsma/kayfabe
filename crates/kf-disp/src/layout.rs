//! ★ The display plane's wire layouts — DERIVED, never hand-typed (`tools/derive_display_layouts.sh`).
//!
//! The TSV is the compiler's answer for ogkm's own headers: struct sizes, member offsets and sizes,
//! and constant values (`data/layouts-<version>.tsv`). This module only reads it. A name the TSV does
//! not carry is a `None`, and every caller turns `None` into a refusal — never a guessed offset.

use std::collections::HashMap;
use std::sync::OnceLock;

/// One driver version's layouts.
#[derive(Debug, Default)]
pub struct Layouts {
    /// The ogkm version the TSV was derived from.
    pub version: String,
    sizes: HashMap<String, usize>,
    fields: HashMap<(String, String), (usize, usize)>,
    consts: HashMap<String, u64>,
}

impl Layouts {
    /// Parse a TSV produced by `tools/derive_display_layouts.sh`.
    #[must_use]
    pub fn parse(tsv: &str) -> Layouts {
        let mut l = Layouts::default();
        for line in tsv.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                ["VERSION", v] => l.version = (*v).to_string(),
                ["SIZE", s, n] => {
                    if let Ok(n) = n.parse() {
                        l.sizes.insert((*s).to_string(), n);
                    }
                }
                ["FIELD", s, m, off, sz] => {
                    if let (Ok(off), Ok(sz)) = (off.parse(), sz.parse()) {
                        l.fields
                            .insert(((*s).to_string(), (*m).to_string()), (off, sz));
                    }
                }
                ["CONST", c, v] => {
                    if let Ok(v) = u64::from_str_radix(v.trim_start_matches("0x"), 16) {
                        l.consts.insert((*c).to_string(), v);
                    }
                }
                _ => {}
            }
        }
        l
    }

    /// `sizeof(struct)`.
    #[must_use]
    pub fn size(&self, s: &str) -> Option<usize> {
        self.sizes.get(s).copied()
    }

    /// `(offsetof, sizeof)` of `struct.member`.
    #[must_use]
    pub fn field(&self, s: &str, m: &str) -> Option<(usize, usize)> {
        self.fields.get(&(s.to_string(), m.to_string())).copied()
    }

    /// A constant's value.
    #[must_use]
    pub fn konst(&self, c: &str) -> Option<u64> {
        self.consts.get(c).copied()
    }

    /// A constant that fits 32 bits (control ids, enum values).
    #[must_use]
    pub fn k32(&self, c: &str) -> Option<u32> {
        self.konst(c).and_then(|v| u32::try_from(v).ok())
    }
}

/// The layouts for a guest driver version, or `None` when this tree has not derived them.
#[must_use]
pub fn for_version(version: &str) -> Option<&'static Layouts> {
    static V580_159_04: OnceLock<Layouts> = OnceLock::new();
    static V580_65_06: OnceLock<Layouts> = OnceLock::new();
    match version {
        "580.159.04" => Some(
            V580_159_04
                .get_or_init(|| Layouts::parse(include_str!("../data/layouts-580.159.04.tsv"))),
        ),
        // ★ 2026-10-04 (v3-windows): 580.65.06, the Linux twin of Windows 580.88 (same changelist
        // 36308443). Derived by tools/derive_display_layouts.sh from ogkm 580.65.06; the rows are
        // identical to 580.159.04's apart from VERSION.
        "580.65.06" => Some(
            V580_65_06
                .get_or_init(|| Layouts::parse(include_str!("../data/layouts-580.65.06.tsv"))),
        ),
        _ => None,
    }
}

/// A params buffer viewed through a layout: reads and writes by member name, bounds-checked.
pub struct Params<'a> {
    l: &'a Layouts,
    s: &'a str,
    /// The bytes.
    pub buf: Vec<u8>,
}

impl<'a> Params<'a> {
    /// View `bytes` as `struct` — `None` when the size does not match the derived one.
    #[must_use]
    pub fn new(l: &'a Layouts, s: &'a str, bytes: &[u8]) -> Option<Params<'a>> {
        (l.size(s)? == bytes.len()).then(|| Params {
            l,
            s,
            buf: bytes.to_vec(),
        })
    }

    /// Read a 1-, 2-, 4- or 8-byte member (little-endian), or `None`.
    #[must_use]
    pub fn get(&self, m: &str) -> Option<u64> {
        let (off, sz) = self.l.field(self.s, m)?;
        let b = self.buf.get(off..off + sz)?;
        Some(match sz {
            1 => u64::from(b[0]),
            2 => u64::from(u16::from_le_bytes([b[0], b[1]])),
            4 => u64::from(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
            8 => u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
            _ => return None,
        })
    }

    /// Write a 1-, 2-, 4- or 8-byte member; `false` when the member or width is unknown.
    pub fn set(&mut self, m: &str, v: u64) -> bool {
        let Some((off, sz)) = self.l.field(self.s, m) else {
            return false;
        };
        let Some(b) = self.buf.get_mut(off..off + sz) else {
            return false;
        };
        match sz {
            1 => b[0] = v as u8,
            2 => b.copy_from_slice(&(v as u16).to_le_bytes()),
            4 => b.copy_from_slice(&(v as u32).to_le_bytes()),
            8 => b.copy_from_slice(&v.to_le_bytes()),
            _ => return false,
        }
        true
    }

    /// Write bytes into an array member (`edidBuffer`, `windowHeadMask`), zero-filling the rest of it.
    pub fn set_bytes(&mut self, m: &str, data: &[u8]) -> bool {
        let Some((off, sz)) = self.l.field(self.s, m) else {
            return false;
        };
        if data.len() > sz {
            return false;
        }
        let Some(b) = self.buf.get_mut(off..off + sz) else {
            return false;
        };
        b.fill(0);
        b[..data.len()].copy_from_slice(data);
        true
    }

    /// Read an array member's bytes.
    #[must_use]
    pub fn bytes(&self, m: &str) -> Option<&[u8]> {
        let (off, sz) = self.l.field(self.s, m)?;
        self.buf.get(off..off + sz)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bench's layouts parse, and a few sizes are what the headers say (`ctrl0073system.h`:
    /// three `NvU32`s; `ctrl2080internal.h`: GET_STATIC_INFO 36 bytes).
    #[test]
    fn the_bench_layouts_parse_and_agree_with_the_headers() {
        let l = for_version("580.159.04").expect("derived");
        assert_eq!(l.version, "580.159.04");
        assert_eq!(l.size("NV0073_CTRL_SYSTEM_GET_NUM_HEADS_PARAMS"), Some(12));
        assert_eq!(
            l.field("NV0073_CTRL_SYSTEM_GET_NUM_HEADS_PARAMS", "numHeads"),
            Some((8, 4))
        );
        assert_eq!(
            l.size("NV2080_CTRL_INTERNAL_DISPLAY_GET_STATIC_INFO_PARAMS"),
            Some(36)
        );
        assert_eq!(
            l.k32("NV0073_CTRL_CMD_SYSTEM_GET_CAPS_V2"),
            Some(0x0073_0101)
        );
        assert_eq!(l.k32("NVC370_CTRL_GET_CHANNEL_INFO_STATE_IDLE"), Some(1));
        assert!(
            for_version("535.309.01").is_none(),
            "an underived version is refused, never guessed"
        );
        let mut p =
            Params::new(l, "NV0073_CTRL_SYSTEM_GET_NUM_HEADS_PARAMS", &[0; 12]).expect("view");
        assert!(p.set("numHeads", 4));
        assert_eq!(p.get("numHeads"), Some(4));
        assert!(Params::new(l, "NV0073_CTRL_SYSTEM_GET_NUM_HEADS_PARAMS", &[0; 8]).is_none());
    }

    /// ★ 2026-10-04 (v3-windows): guest 580.65.06 — the Linux twin of Windows 580.88 by changelist —
    /// has all three display tables, and they are the bench's rows apart from VERSION (re-derived
    /// from ogkm 580.65.06 by the three `tools/derive_display_*.sh`). A future re-derivation that
    /// differs fails here, by table, instead of answering a 580.65.06 guest with 580.159.04's rows.
    #[test]
    fn the_580_65_06_tables_are_derived_and_are_the_bench_rows() {
        let rows = |s: &str| -> Vec<String> {
            s.lines()
                .filter(|l| !l.starts_with('#') && !l.starts_with("VERSION\t"))
                .map(str::to_owned)
                .collect()
        };
        for (name, twin, bench) in [
            (
                "layouts",
                include_str!("../data/layouts-580.65.06.tsv"),
                include_str!("../data/layouts-580.159.04.tsv"),
            ),
            (
                "classes",
                include_str!("../data/classes-580.65.06.tsv"),
                include_str!("../data/classes-580.159.04.tsv"),
            ),
            (
                "regs",
                include_str!("../data/regs-580.65.06.tsv"),
                include_str!("../data/regs-580.159.04.tsv"),
            ),
        ] {
            assert!(twin.contains("VERSION\t580.65.06"), "{name}");
            assert_eq!(rows(twin), rows(bench), "{name}");
        }
        assert_eq!(
            for_version("580.65.06").expect("derived").version,
            "580.65.06"
        );
        assert!(crate::class::for_version("580.65.06").is_some());
        assert!(crate::regs::table_for("580.65.06").is_some());
    }
}
