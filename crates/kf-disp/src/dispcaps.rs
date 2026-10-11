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
//! | `SINGLE_HEAD_MST_SUPPORTED` | unsupported: the authored SOR capabilities and connectors are TMDS-only |
//! | `SINGLE_HEAD_DUAL_SST_SUPPORTED` | unsupported: the model refuses slave displays |
//! | `CROSS_BAR_SUPPORTED` | this model answers `NV0073_CTRL_CMD_DFP_ASSIGN_SOR` (the crossbar's control; the routing is fixed, connector `i` on SOR `i`) |
//! | `AA_FOS_GAMMA_COMP_SUPPORTED`, `KSV_SRM_VALIDATION_SUPPORTED`, `GLITCHLESS_MODESET_SUPPORTED` | unsupported: a measurement of physical RM does not implement gamma compensation, HDCP validation or a glitchless modeset contract in this model |
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

/// The facts the table is a function of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineFacts {
    /// Highest TMDS clock the capabilities page presents, kHz.
    pub tmds_max_khz: u32,
    /// This model answers the SOR crossbar's control.
    pub sor_routing: bool,
}

impl EngineFacts {
    /// Derive the TMDS-only model facts from its class table and
    /// whether the model implements the crossbar control. DP stream capabilities remain absent
    /// on every family: class field definitions alone do not implement DP support.
    ///
    /// # Errors
    /// [`Missing`] when the class table lacks the TMDS clock the page itself needs.
    pub fn derive(t: &ClassTable, sor_routing: bool) -> Result<EngineFacts, Missing> {
        Ok(EngineFacts {
            tmds_max_khz: tmds_max_khz(t)?,
            sor_routing,
        })
    }

    /// The capability names set, in table order. Workaround bits are never in it.
    #[must_use]
    pub fn caps(&self) -> Vec<&'static str> {
        [
            (
                self.tmds_max_khz > HDMI_1_4_TMDS_MAX_KHZ,
                "HDMI_2_0_SUPPORTED",
            ),
            (self.sor_routing, "CROSS_BAR_SUPPORTED"),
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

    fn tables() -> (&'static ClassTable, &'static Layouts) {
        (
            crate::class::for_version("580.65.06").expect("classes"),
            crate::layout::for_version("580.65.06").expect("layouts"),
        )
    }

    fn facts(routing: bool) -> EngineFacts {
        let (t, _) = tables();
        EngineFacts::derive(t, routing).expect("facts")
    }

    fn table(f: &EngineFacts) -> Vec<u8> {
        caps_table(tables().1, f).expect("table")
    }

    /// The TMDS-only model omits DP stream bits even on a family whose real hardware supports
    /// them. Each implemented capability still follows its own model fact.
    #[test]
    fn the_ada_model_advertises_only_implemented_features_and_each_rule_moves_its_own_bit() {
        let f = facts(true);
        assert_eq!(table(&f), [0x00, 0x0c]);
        assert_eq!(f.caps(), ["HDMI_2_0_SUPPORTED", "CROSS_BAR_SUPPORTED"]);
        // the model does not answer the crossbar control: no CROSS_BAR, nothing else moves
        assert_eq!(table(&facts(false)), [0x00, 0x04]);
        // TMDS: 340 MHz is HDMI 1.4b's ceiling, one kHz above is HDMI 2.0
        let mut g = f;
        g.tmds_max_khz = HDMI_1_4_TMDS_MAX_KHZ;
        assert_eq!(table(&g), [0x00, 0x08]);
        g.tmds_max_khz = HDMI_1_4_TMDS_MAX_KHZ + 1;
        assert_eq!(table(&g), [0x00, 0x0c]);
        // no workaround bit is ever set, whatever the facts
        for routing in [false, true] {
            let t = table(&facts(routing));
            assert_eq!(t[0] & 0x7e, 0);
            assert_eq!(t[1] & 0x50, 0);
        }
    }

    /// Hostile layouts: a bit the layouts lack, a table size that is not the member's, or a bit
    /// outside the table is refused by name (never a guessed position).
    #[test]
    fn a_layout_that_lacks_a_bit_or_the_table_size_is_refused_by_name() {
        let base = include_str!("../data/layouts-580.65.06.tsv");
        let f = facts(true);
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
        assert!(EngineFacts::derive(&none, true).is_err());
    }
}
