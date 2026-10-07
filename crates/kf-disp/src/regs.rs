//! ★ The display engine's BAR0 register vocabulary — DERIVED from ogkm's published
//! `disp/<version>/dev_disp.h` headers (`tools/derive_display_regs.sh`), per family, never typed.
//!
//! A GSP-client guest's kernel display HAL compiles against **several** of those headers at once:
//! `v03_00` is the base every NVDisplay family shares, and a family binds a handful of entry points to
//! a newer header (`ogkm-580: src/nvidia/generated/g_kern_disp_nvoc.c` — e.g. GA10x binds
//! `kdispHandleWinSemEvt_v04_01` and `kdispGetVgaWorkspaceBase_v04_00`, GB20x binds
//! `kdispIntrRetrigger_v05_01`, Turing binds neither WinSem entry). [`lineage`] is that list, newest
//! first; a name resolves to the first directory in it that defines the name. ⊘ A name no directory in
//! the lineage defines is `None`, and every caller turns `None` into a refusal — never offset 0.
//!
//! What the plane reads here, and why (`docs/design/V3_DISPLAY.md` §4.2 (B)):
//! - the channel **user areas** (`NV_UDISP_FE_CHN_ASSY_BASEADR_*`, the core's ARMED half) — PUT/GET
//!   and the ARMED mirror NVKMS reads;
//! - `NV_PDISP_FE_CHNCTL_*` / `NV_PDISP_FE_CHNSTATUS_*` — what the CPU-RM polls on a channel's idle and
//!   allocation state (`kern_disp_channel_0300.c`);
//! - the event and interrupt registers the guest ISR reads and write-1-clears
//!   (`kern_disp_0300.c:543-760`, `kern_disp_0401.c`);
//! - `NV_PDISP_FE_CORE_HEAD_STATE(i)` (an active head must read AWAKE), `NV_PDISP_RG_DPCA(i)` (the
//!   scanline/frame counter), and `NV_PDISP_FE_SW` — the capabilities page.

use std::collections::HashMap;
use std::sync::OnceLock;

/// One derived row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// A plain value (a register offset or an enum value).
    V(u64),
    /// A bit field `hi:lo` (also an address range `hi:lo`).
    F(u64, u64),
    /// A linear one-parameter register `X(i)` = base + i·stride.
    A(u64, u64),
    /// A one-parameter field `X(i)` = (hi0 + i·stride):(lo0 + i·stride).
    Fa(u64, u64, u64),
    /// A one-parameter macro that is NOT linear in `i` — named, never evaluated.
    NonLinear,
}

/// Every header directory's rows, as derived.
#[derive(Debug, Default)]
pub struct RegTable {
    /// The ogkm version the TSV was derived from.
    pub version: String,
    rows: HashMap<(String, String), Row>,
}

impl RegTable {
    /// Parse a TSV produced by `tools/derive_display_regs.sh`.
    #[must_use]
    pub fn parse(tsv: &str) -> RegTable {
        let mut t = RegTable::default();
        for line in tsv.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            if f.first() == Some(&"VERSION") {
                t.version = f.get(1).map(|s| (*s).to_string()).unwrap_or_default();
                continue;
            }
            let n = |i: usize| f.get(i).and_then(|s| s.parse::<u64>().ok());
            let row = match f.get(1) {
                Some(&"V") => n(3).map(Row::V),
                Some(&"F") => n(3).zip(n(4)).map(|(h, l)| Row::F(h, l)),
                Some(&"A") => n(3).zip(n(4)).map(|(b, s)| Row::A(b, s)),
                Some(&"FA") => n(3).zip(n(4)).zip(n(5)).map(|((h, l), s)| Row::Fa(h, l, s)),
                Some(&"N") => Some(Row::NonLinear),
                _ => None,
            };
            if let (Some(r), Some(dir), Some(name)) = (row, f.first(), f.get(2)) {
                t.rows.insert(((*dir).to_string(), (*name).to_string()), r);
            }
        }
        t
    }

    fn get(&self, dir: &str, name: &str) -> Option<Row> {
        self.rows.get(&(dir.to_string(), name.to_string())).copied()
    }
}

/// ★ The header directories a family's kernel display HAL compiles against, NEWEST FIRST, keyed by
/// the IP version physical RM reports (`kf_chip::display::DisplayRow::ip_version`). `None` for an
/// IP version this table has no lineage for — refused, never guessed.
///
/// Sources (`ogkm-580: src/nvidia/generated/g_kern_disp_nvoc.c`): `DISPv0400` (TU10x) binds the
/// `_v03_00` entry points and `kdispGetVgaWorkspaceBase_v04_00`, and stubs the WinSem pair out;
/// `DISPv0401` (GA10x) adds `kdispReadPendingWinSemIntr_v04_01` / `kdispHandleWinSemEvt_v04_01`;
/// `DISPv0404` (AD10x) adds `kdispGetBaseOffset_v04_02`; `DISPv0502` (GB20x) adds
/// `kdispIntrRetrigger_v05_01` and the head HAL's `_v05_02` interrupt enable.
#[must_use]
pub fn lineage(ip_version: u32) -> Option<&'static [&'static str]> {
    match ip_version {
        0x0400_0000 => Some(&["v04_00", "v03_00"]),
        0x0401_0000 => Some(&["v04_01", "v04_00", "v03_00"]),
        0x0404_0000 => Some(&["v04_02", "v04_01", "v04_00", "v03_00"]),
        0x0502_0000 => Some(&["v05_02", "v05_01", "v04_02", "v04_01", "v04_00", "v03_00"]),
        _ => None,
    }
}

/// ★ One family's register vocabulary: the table seen through a lineage.
#[derive(Debug, Clone, Copy)]
pub struct Regs {
    t: &'static RegTable,
    lineage: &'static [&'static str],
}

impl Regs {
    /// The registers of the display IP `ip_version` answers with, or `None` when the IP version has
    /// no lineage or the guest driver's version has no derived table.
    #[must_use]
    pub fn for_ip(version: &str, ip_version: u32) -> Option<Regs> {
        Some(Regs {
            t: table_for(version)?,
            lineage: lineage(ip_version)?,
        })
    }

    fn row(&self, name: &str) -> Option<Row> {
        self.lineage.iter().find_map(|d| self.t.get(d, name))
    }

    /// A plain value.
    #[must_use]
    pub fn v(&self, name: &str) -> Option<u64> {
        match self.row(name)? {
            Row::V(v) => Some(v),
            _ => None,
        }
    }

    /// A 32-bit plain value (an offset in BAR0, an enum).
    #[must_use]
    pub fn v32(&self, name: &str) -> Option<u32> {
        self.v(name).and_then(|v| u32::try_from(v).ok())
    }

    /// A field or address range `(hi, lo)`.
    #[must_use]
    pub fn f(&self, name: &str) -> Option<(u64, u64)> {
        match self.row(name)? {
            Row::F(h, l) => Some((h, l)),
            _ => None,
        }
    }

    /// A 32-bit field `(hi, lo)` as the `class` module's bit helpers take it.
    #[must_use]
    pub fn f32(&self, name: &str) -> Option<(u8, u8)> {
        let (h, l) = self.f(name)?;
        (h < 32 && l <= h).then_some((h as u8, l as u8))
    }

    /// A linear one-parameter register `X(i)`.
    #[must_use]
    pub fn a(&self, name: &str, i: u32) -> Option<u64> {
        match self.row(name)? {
            Row::A(b, s) => b.checked_add(u64::from(i).checked_mul(s)?),
            _ => None,
        }
    }

    /// An indexed field `X(i)` as `(hi, lo)` of a 32-bit register.
    #[must_use]
    pub fn fa(&self, name: &str, i: u32) -> Option<(u8, u8)> {
        match self.row(name)? {
            Row::Fa(h, l, s) => {
                let (h, l) = (h + u64::from(i) * s, l + u64::from(i) * s);
                (h < 32 && l <= h).then_some((h as u8, l as u8))
            }
            _ => None,
        }
    }
}

impl RegTable {
    /// A plain value in ONE named header directory — for the rare register a family's own driver
    /// reaches through a hand-written define rather than its lineage's headers (the caller cites it).
    #[must_use]
    pub fn v_in(&self, dir: &str, name: &str) -> Option<u64> {
        match self.get(dir, name)? {
            Row::V(v) => Some(v),
            _ => None,
        }
    }

    /// A linear register in ONE named header directory (see [`Self::v_in`]).
    #[must_use]
    pub fn a_in(&self, dir: &str, name: &str, i: u32) -> Option<u64> {
        match self.get(dir, name)? {
            Row::A(b, s) => b.checked_add(u64::from(i).checked_mul(s)?),
            _ => None,
        }
    }

    /// A field in ONE named header directory (see [`Self::v_in`]).
    #[must_use]
    pub fn f_in(&self, dir: &str, name: &str) -> Option<(u64, u64)> {
        match self.get(dir, name)? {
            Row::F(h, l) => Some((h, l)),
            _ => None,
        }
    }
}

impl Regs {
    /// The table under this lineage (for [`RegTable::v_in`]-style lookups).
    #[must_use]
    pub fn table(&self) -> &'static RegTable {
        self.t
    }
}

/// The derived table for a guest driver version, or `None` when this tree has not derived it.
#[must_use]
pub fn table_for(version: &str) -> Option<&'static RegTable> {
    static V580_159_04: OnceLock<RegTable> = OnceLock::new();
    static V580_65_06: OnceLock<RegTable> = OnceLock::new();
    match version {
        "580.159.04" => Some(
            V580_159_04
                .get_or_init(|| RegTable::parse(include_str!("../data/regs-580.159.04.tsv"))),
        ),
        // ★ 2026-10-04 (v3-windows): 580.65.06, the Linux twin of Windows 580.88 (same changelist
        // 36308443). Derived by tools/derive_display_regs.sh from ogkm 580.65.06; the rows are
        // identical to 580.159.04's apart from VERSION.
        "580.65.06" => Some(
            V580_65_06.get_or_init(|| RegTable::parse(include_str!("../data/regs-580.65.06.tsv"))),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ga10x() -> Regs {
        Regs::for_ip("580.159.04", 0x0401_0000).expect("GA10x lineage")
    }

    /// ★ The GA10x register vocabulary is the headers' (values as printed in `v03_00`/`v04_00`/
    /// `v04_01/dev_disp.h`): channel user areas, the core's ARMED half, the event registers the ISR
    /// reads, CORE_HEAD_STATE, the caps page.
    #[test]
    fn ga10x_registers_are_the_headers() {
        let r = ga10x();
        assert_eq!(r.v("NV_UDISP_FE_CHN_ASSY_BASEADR_CORE"), Some(0x0068_0000));
        assert_eq!(
            r.v("NV_UDISP_FE_CHN_ARMED_BASEADR_CORE"),
            Some(0x0068_0000 + 32768)
        );
        assert_eq!(
            r.a("NV_UDISP_FE_CHN_ASSY_BASEADR_WIN", 3),
            Some(0x0069_3000)
        );
        assert_eq!(
            r.a("NV_UDISP_FE_CHN_ASSY_BASEADR_WINIM", 0),
            Some(0x006B_0000)
        );
        assert_eq!(
            r.a("NV_UDISP_FE_CHN_ASSY_BASEADR_CURS", 2),
            Some(0x006D_A000)
        );
        assert_eq!(r.v("NV_PDISP_FE_RM_INTR_STAT_CTRL_DISP"), Some(0x0061_1C30));
        assert_eq!(
            r.f32("NV_PDISP_FE_RM_INTR_STAT_CTRL_DISP_AWAKEN"),
            Some((8, 8))
        );
        assert_eq!(
            r.f32("NV_PDISP_FE_RM_INTR_STAT_CTRL_DISP_WIN_SEM"),
            Some((9, 9)),
            "v04_01"
        );
        assert_eq!(r.v("NV_PDISP_FE_EVT_STAT_AWAKEN_WIN"), Some(0x0061_1858));
        assert_eq!(r.v("NV_PDISP_FE_EVT_STAT_AWAKEN_OTHER"), Some(0x0061_185C));
        assert_eq!(
            r.a("NV_PDISP_FE_EVT_STAT_HEAD_TIMING", 1),
            Some(0x0061_1804)
        );
        assert_eq!(
            r.a("NV_PDISP_FE_CORE_HEAD_STATE", 1),
            Some(0x0061_2078 + 0x800),
            "v04_00"
        );
        assert_eq!(r.f("NV_PDISP_FE_SW"), Some((0x0064_0FFF, 0x0064_0000)));
        assert_eq!(r.v("NV_PDISP_FE_CHNSTATUS_CORE"), Some(0x0061_0630));
        assert_eq!(r.a("NV_PDISP_FE_CHNSTATUS_WIN", 1), Some(0x0061_0668));
        assert_eq!(r.fa("NV_PDISP_FE_EVT_STAT_AWAKEN_WIN_CH", 5), Some((5, 5)));
        assert_eq!(r.f32("NV_DMA_TARGET_NODE"), Some((1, 0)));
        assert_eq!(r.v("NV_DMA_SIZE"), Some(20));
    }

    /// ⊘ A non-linear macro is never evaluated as base + stride; an unknown IP version, name or
    /// driver version is `None`, never a guess; Turing does not see GA10x's `v04_01` WinSem rows.
    #[test]
    fn nothing_is_guessed() {
        let r = ga10x();
        assert_eq!(
            r.a("NV_UDISP_FE_CHN_ASSY_BASEADR", 1),
            None,
            "non-linear in the header"
        );
        assert_eq!(r.v("NV_PDISP_NO_SUCH_REGISTER"), None);
        assert!(
            Regs::for_ip("580.159.04", 0x0300_0000).is_none(),
            "no lineage for Volta"
        );
        assert!(
            Regs::for_ip("535.309.01", 0x0401_0000).is_none(),
            "an underived driver"
        );
        let tu = Regs::for_ip("580.159.04", 0x0400_0000).unwrap();
        assert_eq!(
            tu.f32("NV_PDISP_FE_RM_INTR_STAT_CTRL_DISP_WIN_SEM"),
            None,
            "Turing has no WinSem"
        );
        assert_eq!(
            tu.v("NV_PDISP_FE_RM_INTR_STAT_CTRL_DISP"),
            Some(0x0061_1C30)
        );
    }

    /// ★ Within every lineage, a name two directories define carries the SAME value in both — so
    /// newest-first resolution never changes an answer (it only adds names).
    #[test]
    fn a_lineage_never_redefines_a_name() {
        let t = table_for("580.159.04").unwrap();
        for ip in [0x0400_0000u32, 0x0401_0000, 0x0404_0000, 0x0502_0000] {
            let dirs = lineage(ip).unwrap();
            let mut seen: HashMap<&str, Row> = HashMap::new();
            for d in dirs {
                for ((dir, name), row) in &t.rows {
                    if dir == d {
                        if let Some(prev) = seen.get(name.as_str()) {
                            assert_eq!(prev, row, "{ip:#x}: {name} differs across {dirs:?}");
                        }
                        seen.insert(name.as_str(), *row);
                    }
                }
            }
        }
    }
}
