//! ★ `NV0073_CTRL_CMD_SYSTEM_GET_CAPS_V2`'s `capsTbl`, DERIVED (2026-10-11, `V3_DISPLAY.md` §4.15).
//!
//! The header: "the set of display capabilities … supported features and required workarounds for
//! the display engine(s) within the device, each represented by a byte offset into the table and a
//! bit position within that byte" (`ogkm-595.84 ctrl0073system.h`). So a bit is a fact about the
//! ENGINE this model presents, and every bit below has a rule over facts the model owns, never a
//! constant byte string:
//!
//! | bit | rule |
//! |---|---|
//! | `HDMI_2_0_SUPPORTED` | the TMDS clock the capabilities page authors exceeds 340 MHz, HDMI 1.4b's ceiling (HDMI 2.0 raised the TMDS character rate to 600 MHz). NVKMS reads the same field: `maxTMDSClkKHz = TMDS_MAX * 10000` (`nvkms-evo3.c:5185`) |
//! | `SINGLE_HEAD_MST_SUPPORTED` | the family's caps class defines a DP-capable SOR (`SOR_CAP_DP_A`) |
//! | `SINGLE_HEAD_DUAL_SST_SUPPORTED` | the class defines both `SOR_CAP_DP_A` and `SOR_CAP_DP_B` and the engine has at least two SORs (two SST links feed one head) |
//! | `CROSS_BAR_SUPPORTED` | this model answers `NV0073_CTRL_CMD_DFP_ASSIGN_SOR` (the crossbar's control; the routing is fixed, connector `i` on SOR `i`) |
//! | `AA_FOS_GAMMA_COMP_SUPPORTED`, `KSV_SRM_VALIDATION_SUPPORTED`, `GLITCHLESS_MODESET_SUPPORTED` | RM-software features of the display engine generation: nothing in the open headers, the class tables or the model derives them, so they come from the family's [`RmFeatures`] row, kept by hand, and only for an IP version observed on hardware. Any other IP version claims none (named, [`rm_features`]) |
//!
//! No workaround bit (`*_BUG_*`, `RASTER_LOCK_NEEDS_MIO_POWER`, `HDMI21_SW_ACR_BUG_3275257`) is ever
//! set: this engine has none of those defects. Offsets and masks come from the compiled headers
//! (`layouts-*.tsv`, `CAP_<name>_BYTE` / `_MASK`), so a bit this tree's layouts lack is refused by name.

use crate::caps::{Missing, tmds_max_khz};
use crate::class::ClassTable;
use crate::layout::Layouts;

/// HDMI 1.4b's TMDS character-rate ceiling in kHz (HDMI 1.4b specification); above it the source is
/// an HDMI 2.0 transmitter.
pub const HDMI_1_4_TMDS_MAX_KHZ: u32 = 340_000;

/// The RM-software features of a display engine generation that no open source derives (see the
/// module table). A hand-kept FAMILY row, not a per-die table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RmFeatures {
    /// `NV0073_CTRL_SYSTEM_CAPS_AA_FOS_GAMMA_COMP_SUPPORTED`.
    pub aa_fos_gamma_comp: bool,
    /// `NV0073_CTRL_SYSTEM_CAPS_KSV_SRM_VALIDATION_SUPPORTED`.
    pub ksv_srm_validation: bool,
    /// `NV0073_CTRL_SYSTEM_CAPS_GLITCHLESS_MODESET_SUPPORTED`.
    pub glitchless_modeset: bool,
}

/// The display IP versions whose [`RmFeatures`] were observed on hardware.
///
/// `[observed 2026-10-11]` `DISPv0404` (Ada, `kf_chip::display::ADA`): a real RTX 4070 (AD104),
/// Windows 580.88, VFIO reference, 15 of 15 `0x730101` answers `NV_OK` with `capsTbl = 81 2f`: byte 0
/// `0x81` = `AA_FOS_GAMMA_COMP | KSV_SRM_VALIDATION`, byte 1 `0x2f` = `SINGLE_HEAD_MST |
/// SINGLE_HEAD_DUAL_SST | HDMI_2_0 | CROSS_BAR | GLITCHLESS_MODESET`. The capture is the ORACLE for
/// the whole table (the derived bits must reproduce it); it is the DATA only for the three bits
/// above that nothing else derives, and is inferred to hold for the family (AD102..AD107 share
/// `DISPv0404`), not measured on those dies.
pub const OBSERVED_RM_FEATURES: &[(u32, RmFeatures)] = &[(
    0x0404_0000,
    RmFeatures {
        aa_fos_gamma_comp: true,
        ksv_srm_validation: true,
        glitchless_modeset: true,
    },
)];

/// The [`RmFeatures`] for a display IP version: the observed row, or `None` for an IP version no
/// capture covered (it then claims none of the three, never a neighbour's).
#[must_use]
pub fn rm_features(ip_version: u32) -> Option<RmFeatures> {
    OBSERVED_RM_FEATURES
        .iter()
        .find(|(v, _)| *v == ip_version)
        .map(|(_, f)| *f)
}

/// The facts the table is a function of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineFacts {
    /// Highest TMDS clock the capabilities page presents, kHz.
    pub tmds_max_khz: u32,
    /// The caps class defines a DP-capable SOR.
    pub dp_sor: bool,
    /// The class defines both DP link bits and the engine has two or more SORs.
    pub dp_sor_pair: bool,
    /// This model answers the SOR crossbar's control.
    pub sor_routing: bool,
    /// [`rm_features`] of the IP version (all `false` when unobserved).
    pub rm: RmFeatures,
}

impl EngineFacts {
    /// Derive the facts for a family: its caps class `caps_class` in `t`, `heads` heads (one SOR
    /// each, `caps::page`), `ip_version`, and whether the model implements the crossbar control.
    ///
    /// # Errors
    /// [`Missing`] when the class table lacks the TMDS clock the page itself needs.
    pub fn derive(
        t: &ClassTable,
        caps_class: u32,
        ip_version: u32,
        heads: u32,
        sor_routing: bool,
    ) -> Result<EngineFacts, Missing> {
        let dp_a = t.f(caps_class, "SOR_CAP_DP_A").is_some();
        let dp_b = t.f(caps_class, "SOR_CAP_DP_B").is_some();
        Ok(EngineFacts {
            tmds_max_khz: tmds_max_khz(t)?,
            dp_sor: dp_a,
            dp_sor_pair: dp_a && dp_b && heads >= 2,
            sor_routing,
            rm: rm_features(ip_version).unwrap_or_default(),
        })
    }

    /// The capability names set, in table order. Workaround bits are never in it.
    #[must_use]
    pub fn caps(&self) -> Vec<&'static str> {
        [
            (self.rm.aa_fos_gamma_comp, "AA_FOS_GAMMA_COMP_SUPPORTED"),
            (self.rm.ksv_srm_validation, "KSV_SRM_VALIDATION_SUPPORTED"),
            (self.dp_sor, "SINGLE_HEAD_MST_SUPPORTED"),
            (self.dp_sor_pair, "SINGLE_HEAD_DUAL_SST_SUPPORTED"),
            (
                self.tmds_max_khz > HDMI_1_4_TMDS_MAX_KHZ,
                "HDMI_2_0_SUPPORTED",
            ),
            (self.sor_routing, "CROSS_BAR_SUPPORTED"),
            (self.rm.glitchless_modeset, "GLITCHLESS_MODESET_SUPPORTED"),
        ]
        .into_iter()
        .filter_map(|(on, n)| on.then_some(n))
        .collect()
    }
}

/// The `capsTbl` bytes for `facts`, with each bit's byte and mask from the compiled headers.
///
/// # Errors
/// [`Missing`] when a named bit, or the table size, is absent from `l`, or a bit lies outside the
/// table (never a guessed position).
pub fn caps_table(l: &Layouts, facts: &EngineFacts) -> Result<Vec<u8>, Missing> {
    let size = l
        .konst("NV0073_CTRL_SYSTEM_CAPS_TBL_SIZE")
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| {
            Some(*n)
                == l.field("NV0073_CTRL_SYSTEM_GET_CAPS_V2_PARAMS", "capsTbl")
                    .map(|f| f.1)
        })
        .ok_or_else(|| Missing("NV0073_CTRL_SYSTEM_CAPS_TBL_SIZE".into()))?;
    let mut tbl = vec![0u8; size];
    for name in facts.caps() {
        let byte = l
            .konst(&format!("CAP_{name}_BYTE"))
            .and_then(|b| usize::try_from(b).ok())
            .filter(|b| *b < size)
            .ok_or_else(|| Missing(format!("CAP_{name}_BYTE")))?;
        let mask = l
            .konst(&format!("CAP_{name}_MASK"))
            .and_then(|m| u8::try_from(m).ok())
            .filter(|m| m.count_ones() == 1)
            .ok_or_else(|| Missing(format!("CAP_{name}_MASK")))?;
        tbl[byte] |= mask;
    }
    Ok(tbl)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADA_ROW: &kf_chip::display::DisplayRow = &kf_chip::display::ADA;

    fn tables() -> (&'static ClassTable, &'static Layouts) {
        (
            crate::class::for_version("580.65.06").expect("classes"),
            crate::layout::for_version("580.65.06").expect("layouts"),
        )
    }

    fn facts(row: &kf_chip::display::DisplayRow, routing: bool) -> EngineFacts {
        let (t, _) = tables();
        EngineFacts::derive(t, row.classes.caps, row.ip_version, row.heads, routing).expect("facts")
    }

    fn table(f: &EngineFacts) -> Vec<u8> {
        caps_table(tables().1, f).expect("table")
    }

    /// The vector (real RTX 4070, AD104, 15 of 15 answers): `81 2f`. The Ada model's own facts
    /// reproduce it; each bit's rule is then moved on its own to show it is the rule, not the byte.
    #[test]
    fn the_ada_facts_derive_the_observed_table_and_each_rule_moves_its_own_bit() {
        let f = facts(ADA_ROW, true);
        assert_eq!(table(&f), [0x81, 0x2f]);
        assert_eq!(
            f.caps(),
            [
                "AA_FOS_GAMMA_COMP_SUPPORTED",
                "KSV_SRM_VALIDATION_SUPPORTED",
                "SINGLE_HEAD_MST_SUPPORTED",
                "SINGLE_HEAD_DUAL_SST_SUPPORTED",
                "HDMI_2_0_SUPPORTED",
                "CROSS_BAR_SUPPORTED",
                "GLITCHLESS_MODESET_SUPPORTED"
            ]
        );
        // the model does not answer the crossbar control: no CROSS_BAR, nothing else moves
        assert_eq!(table(&facts(ADA_ROW, false)), [0x81, 0x27]);
        // TMDS: 340 MHz is HDMI 1.4b's ceiling, one kHz above is HDMI 2.0
        let mut g = f;
        g.tmds_max_khz = HDMI_1_4_TMDS_MAX_KHZ;
        assert_eq!(table(&g), [0x81, 0x2b]);
        g.tmds_max_khz = HDMI_1_4_TMDS_MAX_KHZ + 1;
        assert_eq!(table(&g), [0x81, 0x2f]);
        // no DP-capable SOR in the class: neither MST nor dual SST
        g.dp_sor = false;
        g.dp_sor_pair = false;
        assert_eq!(table(&g), [0x81, 0x2c]);
        // no workaround bit is ever set, whatever the facts
        for routing in [false, true] {
            let t = table(&facts(ADA_ROW, routing));
            assert_eq!(t[0] & 0x7e, 0);
            assert_eq!(t[1] & 0x50, 0);
        }
    }

    /// A family whose RM-software bits were not observed claims none of them, and the class table
    /// - not a copy of Ada's bytes - decides the rest. Turing's class defines both DP link bits too.
    #[test]
    fn other_families_follow_their_own_class_table_and_claim_no_unobserved_rm_bit() {
        for row in [
            &kf_chip::display::TURING,
            &kf_chip::display::AMPERE,
            &kf_chip::display::BLACKWELL_GB20X,
        ] {
            assert_eq!(rm_features(row.ip_version), None, "{}", row.chips);
            assert_eq!(table(&facts(row, true)), [0x00, 0x0f], "{}", row.chips);
        }
        assert!(rm_features(ADA_ROW.ip_version).is_some());
        // a class table without the DP link bits: a different table from the same rule
        let (t, _) = tables();
        assert!(t.f(ADA_ROW.classes.caps, "SOR_CAP_DP_A").is_some());
        assert!(t.f(0xDEAD, "SOR_CAP_DP_A").is_none());
        let f = EngineFacts::derive(t, 0xDEAD, ADA_ROW.ip_version, 4, true).unwrap();
        assert!(!f.dp_sor && !f.dp_sor_pair);
        assert_eq!(table(&f), [0x81, 0x2c]);
        // one head: two SST links cannot feed it from two SORs
        let one =
            EngineFacts::derive(t, ADA_ROW.classes.caps, ADA_ROW.ip_version, 1, true).unwrap();
        assert!(one.dp_sor && !one.dp_sor_pair);
        assert_eq!(table(&one), [0x81, 0x2d]);
    }

    /// Hostile layouts: a bit the layouts lack, a table size that is not the member's, or a bit
    /// outside the table is refused by name (never a guessed position).
    #[test]
    fn a_layout_that_lacks_a_bit_or_the_table_size_is_refused_by_name() {
        let base = include_str!("../data/layouts-580.65.06.tsv");
        let f = facts(ADA_ROW, true);
        let without = |name: &str| -> String {
            base.lines()
                .filter(|l| !l.contains(name))
                .collect::<Vec<_>>()
                .join("\n")
        };
        for (needle, want) in [
            (
                "CAP_CROSS_BAR_SUPPORTED_MASK",
                "CAP_CROSS_BAR_SUPPORTED_MASK",
            ),
            ("CAP_HDMI_2_0_SUPPORTED_BYTE", "CAP_HDMI_2_0_SUPPORTED_BYTE"),
            (
                "NV0073_CTRL_SYSTEM_CAPS_TBL_SIZE",
                "NV0073_CTRL_SYSTEM_CAPS_TBL_SIZE",
            ),
        ] {
            let l = Layouts::parse(&without(needle));
            assert_eq!(caps_table(&l, &f), Err(Missing(want.into())), "{needle}");
        }
        // a table size of 1 puts byte 1 outside it
        let small = base.replace(
            "NV0073_CTRL_SYSTEM_CAPS_TBL_SIZE\t0x2",
            "NV0073_CTRL_SYSTEM_CAPS_TBL_SIZE\t0x1",
        );
        assert!(caps_table(&Layouts::parse(&small), &f).is_err());
        // a mask that is not one bit
        let wide = base.replace(
            "CAP_CROSS_BAR_SUPPORTED_MASK\t0x8",
            "CAP_CROSS_BAR_SUPPORTED_MASK\t0x18",
        );
        assert_eq!(
            caps_table(&Layouts::parse(&wide), &f),
            Err(Missing("CAP_CROSS_BAR_SUPPORTED_MASK".into()))
        );
        // a class table lacking the TMDS clock cannot derive
        let none = ClassTable::parse("");
        assert!(EngineFacts::derive(&none, 0xC773, ADA_ROW.ip_version, 4, true).is_err());
    }
}
