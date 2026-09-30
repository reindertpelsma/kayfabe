//! ★ The capabilities page (`NV_PDISP_FE_SW`, BAR0 `0x640000`) our virtual display engine presents —
//! AUTHORED from the family's derived caps-class layout (`NVC373/C573/C673/C773/CA73_*`,
//! `tools/derive_display_classes.sh`), never captured from a board.
//!
//! NVKMS allocates the family's `…73_DISP_CAPABILITIES` object, maps it (the CPU-RM maps this BAR0
//! page, `kdispGetDisplayCapsBaseAndSize_v03_00`, `kern_disp_0300.c:153-173`) and parses it
//! (`nvEvoGetCapabilities3` → `EvoParseCapabilityNotifier3` + the class's parser,
//! `ogkm-580: nvkms-evo3.c:5524-6180`). ⊘ An all-zero page is a display with NO windows:
//! `UsableWindowCount` is 0 and NVKMS falls to the pre-NVDisplay base/overlay path, which a C6
//! class set cannot serve — so the page is part of M1, not decoration.
//!
//! What we present (the monitor and the engine are ours — `V3_DISPLAY.md` §4.7):
//! - `SYS_CAP`: heads `0..heads` exist, and one SOR per head (connector `i` is on SOR `i`,
//!   `model.rs`); `SYS_CAPB`: windows `0..windows` exist, contiguous from 0 (NVKMS asserts it);
//! - per SOR: `SINGLE_TMDS_A`/`_B` and `DUAL_TMDS`, and `HDMI_FRL` where the class has the field
//!   (NVKMS asserts it equals the HAL's `supportsHDMIFRL`); TMDS clock up to the family's own
//!   `SOR_CLK_CAP_TMDS_MAX_INIT` (600 MHz);
//! - per head: the maximum pixel clock of NVKMS's own `NVC373_HEAD_CLK_CAP` work-around define
//!   (`nvkms-evo3.c:5616-5621`), whose published home is `v05_01`'s `NV_PDISP_FE_SW_HEAD_CLK_CAP`
//!   (`0x6405E8` = the page + `0x5e8`) with its reset value;
//! - per window: the CSC matrices and LUTs NVKMS programs unconditionally (CSC11 is asserted,
//!   `nvkms-evo3.c:1057`); no scaler, no TMO, no planar/rotation — features a virtual engine need not
//!   claim, so NVKMS never asks for them.

use crate::class::{ClassTable, put};
use crate::regs::Regs;

/// Bytes in the capabilities page (`NV_PDISP_FE_SW` is `0x640000..=0x640FFF`).
pub const PAGE: usize = 0x1000;

/// Why the page could not be authored (the missing name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing(pub String);

/// ★ The page: `(byte offset within the page, value)` for every non-zero word, and its BAR0 base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapsPage {
    /// BAR0 offset of the page (`DRF_BASE(NV_PDISP_FE_SW)`).
    pub base: u64,
    /// Non-zero words.
    pub words: Vec<(u32, u32)>,
}

impl CapsPage {
    /// The word at byte offset `off` of the page.
    #[must_use]
    pub fn word(&self, off: u32) -> u32 {
        self.words
            .iter()
            .find(|(o, _)| *o == off)
            .map_or(0, |(_, v)| *v)
    }
}

/// ★ Author the page for caps class `caps` (the family's `…73`), `heads` heads and `windows` windows.
///
/// # Errors
/// [`Missing`] naming the first name the derived tables lack — never a default.
pub fn page(
    t: &ClassTable,
    r: &Regs,
    caps: u32,
    heads: u32,
    windows: u32,
) -> Result<CapsPage, Missing> {
    let m = |n: String| Missing(n);
    let v = |n: &str| t.v(caps, n).ok_or_else(|| m(format!("NV{caps:04X}_{n}")));
    let fa = |n: &str, i: u32| {
        t.fa(caps, n, i)
            .ok_or_else(|| m(format!("NV{caps:04X}_{n}({i})")))
    };
    let f = |n: &str| t.f(caps, n).ok_or_else(|| m(format!("NV{caps:04X}_{n}")));
    let a = |n: &str, i: u32| {
        t.a(caps, n, i)
            .ok_or_else(|| m(format!("NV{caps:04X}_{n}({i})")))
    };
    let (hi, lo) = r
        .f("NV_PDISP_FE_SW")
        .ok_or_else(|| m("NV_PDISP_FE_SW".into()))?;
    if hi < lo || (hi - lo + 1) as usize != PAGE {
        return Err(m(format!(
            "NV_PDISP_FE_SW {lo:#x}..={hi:#x} is not one {PAGE:#x}-byte page"
        )));
    }
    let mut w: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
    let mut set = |off: u32, field: (u8, u8), x: u32| {
        let e = w.entry(off).or_insert(0);
        *e = put(*e, field, x);
    };
    // SYS_CAP / SYS_CAPB
    let sys_cap = v("SYS_CAP")?;
    let sys_capb = v("SYS_CAPB")?;
    for h in 0..heads {
        set(sys_cap, fa("SYS_CAP_HEAD_EXISTS", h)?, 1);
        set(sys_cap, fa("SYS_CAP_SOR_EXISTS", h)?, 1);
    }
    for i in 0..windows {
        set(sys_capb, fa("SYS_CAPB_WINDOW_EXISTS", i)?, 1);
    }
    // SORs
    let tmds_max = t
        .v(0xC573, "SOR_CLK_CAP_TMDS_MAX_INIT")
        .ok_or_else(|| m("NVC573_SOR_CLK_CAP_TMDS_MAX_INIT".into()))?;
    for s in 0..heads {
        let cap = a("SOR_CAP", s)?;
        for fld in [
            "SOR_CAP_SINGLE_TMDS_A",
            "SOR_CAP_SINGLE_TMDS_B",
            "SOR_CAP_DUAL_TMDS",
        ] {
            set(cap, f(fld)?, 1);
        }
        if let Some(frl) = t.f(caps, "SOR_CAP_HDMI_FRL") {
            set(cap, frl, 1);
        }
        set(a("SOR_CLK_CAP", s)?, f("SOR_CLK_CAP_TMDS_MAX")?, tmds_max);
    }
    // heads: NVKMS's NVC373_HEAD_CLK_CAP work-around = v05_01's NV_PDISP_FE_SW_HEAD_CLK_CAP
    let rt = r.table();
    let clk0 = rt
        .a_in("v05_01", "NV_PDISP_FE_SW_HEAD_CLK_CAP", 0)
        .ok_or_else(|| m("v05_01 NV_PDISP_FE_SW_HEAD_CLK_CAP".into()))?;
    let clk1 = rt
        .a_in("v05_01", "NV_PDISP_FE_SW_HEAD_CLK_CAP", 1)
        .ok_or_else(|| m("v05_01 NV_PDISP_FE_SW_HEAD_CLK_CAP".into()))?;
    let pmax = rt
        .f_in("v05_01", "NV_PDISP_FE_SW_HEAD_CLK_CAP_PCLK_MAX")
        .ok_or_else(|| m("…HEAD_CLK_CAP_PCLK_MAX".into()))?;
    let pinit = rt
        .v_in("v05_01", "NV_PDISP_FE_SW_HEAD_CLK_CAP_PCLK_MAX_INIT")
        .ok_or_else(|| m("…PCLK_MAX_INIT".into()))?;
    let pmax = (
        u8::try_from(pmax.0).map_err(|_| m("PCLK_MAX hi".into()))?,
        u8::try_from(pmax.1).map_err(|_| m("PCLK_MAX lo".into()))?,
    );
    for h in 0..heads {
        let off = clk0 + u64::from(h) * (clk1 - clk0);
        let rel = off
            .checked_sub(lo)
            .filter(|r| *r < PAGE as u64)
            .ok_or_else(|| m(format!("HEAD_CLK_CAP({h}) {off:#x} is outside the page")))?;
        set(
            rel as u32,
            pmax,
            u32::try_from(pinit).map_err(|_| m("PCLK_MAX_INIT".into()))?,
        );
    }
    // windows: the CSC stages NVKMS programs
    for i in 0..windows {
        let capa = a("PRECOMP_WIN_PIPE_HDR_CAPA", i)?;
        for fld in [
            "PRECOMP_WIN_PIPE_HDR_CAPA_CSC00_PRESENT",
            "PRECOMP_WIN_PIPE_HDR_CAPA_CSC01_PRESENT",
            "PRECOMP_WIN_PIPE_HDR_CAPA_CSC0LUT_PRESENT",
            "PRECOMP_WIN_PIPE_HDR_CAPA_CSC10_PRESENT",
            "PRECOMP_WIN_PIPE_HDR_CAPA_CSC1LUT_PRESENT",
            "PRECOMP_WIN_PIPE_HDR_CAPA_CSC11_PRESENT",
        ] {
            set(capa, f(fld)?, 1);
        }
    }
    for off in w.keys() {
        if *off as usize + 4 > PAGE || off % 4 != 0 {
            return Err(m(format!("caps word at {off:#x} is outside the page")));
        }
    }
    Ok(CapsPage {
        base: lo,
        words: w.into_iter().filter(|(_, v)| *v != 0).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ga10x() -> CapsPage {
        let t = crate::class::for_version("580.159.04").unwrap();
        let r = Regs::for_ip("580.159.04", 0x0401_0000).unwrap();
        page(t, &r, 0xC673, 4, 8).expect("GA10x caps page")
    }

    /// ★ What NVKMS parses out of the page is the engine we present: four heads and four SORs, eight
    /// usable windows contiguous from 0, a head pixel clock and SOR TMDS clock well above 1080p60's
    /// 148.5 MHz, and the CSC11 matrix NVKMS asserts.
    #[test]
    fn nvkms_reads_four_heads_eight_windows_and_real_clocks() {
        let p = ga10x();
        assert_eq!(p.base, 0x0064_0000);
        assert_eq!(p.word(0) & 0xF, 0xF, "HEAD0..3_EXISTS");
        assert_eq!((p.word(0) >> 8) & 0xFF, 0xF, "SOR0..3_EXISTS");
        assert_eq!(p.word(4), 0xFF, "WINDOW0..7_EXISTS");
        let clk = p.word(0x5e8) & 0xFF;
        assert!(clk * 10_000 > 148_500, "head 0 max pclk {clk}0 MHz");
        let sor_clk = (p.word(1544) >> 16) & 0xFF;
        assert_eq!(sor_clk, 60, "600 MHz TMDS");
        let sor0 = p.word(324);
        assert_eq!(sor0 & (1 << 8), 1 << 8, "SINGLE_TMDS_A");
        assert_eq!(sor0 & (1 << 28), 1 << 28, "HDMI_FRL (C6 HAL asserts it)");
        assert_ne!(p.word(1920) & (1 << 24), 0, "window 0 CSC11");
        assert_eq!(p.word(1664), 0, "no postcomp scaler");
    }
}
