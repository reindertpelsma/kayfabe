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
//!   `nvkms-evo3.c:1057`); no scaler, no TMO, no 3-plane/rotation — features a virtual engine need not
//!   claim, so NVKMS never asks for them.
//! - ⊘ CORRECTED 2026-10-14 (the line above said "no planar"): SEMI-planar YUV IS claimed
//!   (`IHUB_COMMON_CAPC.SUPPORT_SEMI_PLANAR`, Turing's `CAPA.SUPPORT_PLANAR`): the engine executes those
//!   windows and the console composes them; a real AD104 claims it (the plane oracle, `page()` below).

use crate::class::{ClassTable, get, put};
use crate::regs::Regs;

/// Bytes in the capabilities page (`NV_PDISP_FE_SW` is `0x640000..=0x640FFF`).
pub const PAGE: usize = 0x1000;

/// Why the page could not be authored (the missing name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing(pub String);

/// The `TMDS_MAX` field value the page authors for every SOR (`NVC573_SOR_CLK_CAP_TMDS_MAX_INIT`).
///
/// # Errors
/// [`Missing`] when the class table lacks it.
pub fn tmds_max_field(t: &ClassTable) -> Result<u32, Missing> {
    t.v(0xC573, "SOR_CLK_CAP_TMDS_MAX_INIT")
        .ok_or_else(|| Missing("NVC573_SOR_CLK_CAP_TMDS_MAX_INIT".into()))
}

/// The same clock in kHz, the way NVKMS reads it back (`maxTMDSClkKHz = TMDS_MAX * 10000`,
/// `nvkms-evo3.c:5185`).
///
/// # Errors
/// [`Missing`] when the class table lacks it or the product overflows.
pub fn tmds_max_khz(t: &ClassTable) -> Result<u32, Missing> {
    tmds_max_field(t)?
        .checked_mul(10_000)
        .ok_or_else(|| Missing("TMDS_MAX kHz overflow".into()))
}

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

/// Capabilities for the opt-in SDR path: one DIRECT10 table per stage,
/// surface loading and one bounded linear TMO with no chroma correction.
/// Sizes are implementation limits, not board facts.
/// The public CA73 layout reports an OLUT sample count instead of legacy LOGSZ.
/// Its capacity comes from the bounded color decoder, excluding the four-entry header.
pub fn sdr_page(
    t: &ClassTable,
    r: &Regs,
    caps: u32,
    _core: u32,
    heads: u32,
    windows: u32,
) -> Result<CapsPage, Missing> {
    let mut p = page(t, r, caps, heads, windows)?;
    let olut = if t.v(caps, "POSTCOMP_HDR_CAPB__SIZE_1").is_some() {
        let samples = u32::try_from(crate::color::LUT_BYTES / 8 - 4)
            .map_err(|_| Missing("output LUT sample capacity".into()))?;
        ("POSTCOMP_HDR_CAPB", "SZ", samples)
    } else {
        ("POSTCOMP_HEAD_HDR_CAPB", "LOGSZ", 10)
    };
    for (register, stage, count, size_field, size) in [
        ("PRECOMP_WIN_PIPE_HDR_CAPB", "ILUT", windows, "LOGSZ", 10),
        (olut.0, "OLUT", heads, olut.1, olut.2),
    ] {
        let max = t
            .v(caps, &format!("{register}__SIZE_1"))
            .filter(|n| count <= *n && *n as usize <= PAGE / 4)
            .ok_or_else(|| Missing(format!("NV{caps:04X}_{register} count")))?;
        for i in 0..count.min(max) {
            let off = t
                .a(caps, register, i)
                .filter(|n| *n % 4 == 0 && (*n as usize) < PAGE)
                .ok_or_else(|| Missing(format!("NV{caps:04X}_{register}({i})")))?;
            let mut word = p.word(off);
            for (suffix, value) in [
                (size_field, size),
                ("LOGNR", 0),
                ("DIRECT", 1),
                ("SFCLOAD", 1),
            ] {
                let name = format!("{register}_{stage}_{suffix}");
                let field = t
                    .f(caps, &name)
                    .ok_or_else(|| Missing(format!("NV{caps:04X}_{name}")))?;
                if field.0 >= 32
                    || field.1 > field.0
                    || u64::from(value) >= (1_u64 << (field.0 - field.1 + 1))
                {
                    return Err(Missing(format!("NV{caps:04X}_{name} value bound")));
                }
                let value = if suffix == "DIRECT" || suffix == "SFCLOAD" {
                    t.v(caps, &format!("{name}_TRUE"))
                        .filter(|v| *v == value)
                        .ok_or_else(|| Missing(format!("NV{caps:04X}_{name}_TRUE")))?
                } else {
                    value
                };
                word = put(word, field, value);
            }
            if let Some((_, v)) = p.words.iter_mut().find(|(at, _)| *at == off) {
                *v = word;
            } else if word != 0 {
                p.words.push((off, word));
            }
        }
    }
    for i in 0..windows {
        for (register, values) in [
            ("PRECOMP_WIN_PIPE_HDR_CAPA", vec![("TMO_PRESENT", 1)]),
            (
                "PRECOMP_WIN_PIPE_HDR_CAPD",
                vec![("TMO_LOGSZ", 10), ("TMO_LOGNR", 0), ("TMO_SFCLOAD", 1)],
            ),
        ] {
            let off = t
                .a(caps, register, i)
                .filter(|n| *n % 4 == 0 && (*n as usize) < PAGE)
                .ok_or_else(|| Missing(format!("NV{caps:04X}_{register}({i})")))?;
            let mut word = p.word(off);
            for (suffix, value) in values {
                let name = format!("{register}_{suffix}");
                let f = t.f(caps, &name).ok_or_else(|| Missing(name.clone()))?;
                if f.0 >= 32 || f.1 > f.0 || u64::from(value) >= (1_u64 << (f.0 - f.1 + 1)) {
                    return Err(Missing(format!("{name} value bound")));
                }
                if suffix.ends_with("PRESENT") || suffix.ends_with("SFCLOAD") {
                    t.v(caps, &format!("{name}_TRUE"))
                        .filter(|v| *v == value)
                        .ok_or_else(|| Missing(name.clone()))?;
                }
                word = put(word, f, value);
            }
            if let Some((_, v)) = p.words.iter_mut().find(|(at, _)| *at == off) {
                *v = word;
            } else {
                p.words.push((off, word));
            }
        }
    }
    p.words.sort_unstable_by_key(|(off, _)| *off);
    Ok(p)
}

/// Constructor-only diagnostic: advertise the source-defined TMO capability.
/// Must be paired with `Engine::new_constructor_probe`, which refuses every
/// display method. This is not a tone-mapping implementation.
pub fn constructor_probe_page(
    t: &ClassTable,
    r: &Regs,
    caps: u32,
    heads: u32,
    windows: u32,
) -> Result<CapsPage, Missing> {
    let mut p = page(t, r, caps, heads, windows)?;
    let name = "PRECOMP_WIN_PIPE_HDR_CAPA_TMO_PRESENT";
    let field = t
        .f(caps, name)
        .ok_or_else(|| Missing(format!("NV{caps:04X}_{name}")))?;
    let yes = t
        .v(caps, &format!("{name}_TRUE"))
        .ok_or_else(|| Missing(format!("NV{caps:04X}_{name}_TRUE")))?;
    for i in 0..windows {
        let off = t
            .a(caps, "PRECOMP_WIN_PIPE_HDR_CAPA", i)
            .ok_or_else(|| Missing(format!("NV{caps:04X}_PRECOMP_WIN_PIPE_HDR_CAPA({i})")))?;
        // The ordinary author already validated this word's alignment and bounds.
        let (_, word) = p
            .words
            .iter_mut()
            .find(|(at, _)| *at == off)
            .ok_or_else(|| Missing(format!("existing caps word {off:#x}")))?;
        *word = put(*word, field, yes);
    }
    Ok(p)
}

/// Constructor-only ILUT discriminator, layered on the TMO constructor probe.
/// Adds only source-defined ILUT surface loading; leaves DIRECT and size/count
/// fields untouched. Must use `Engine::new_constructor_probe`: no display
/// method may execute and no ILUT processing is implemented by this declaration.
pub fn ilut_constructor_probe_page(
    t: &ClassTable,
    r: &Regs,
    caps: u32,
    heads: u32,
    windows: u32,
) -> Result<CapsPage, Missing> {
    let mut p = constructor_probe_page(t, r, caps, heads, windows)?;
    let register = "PRECOMP_WIN_PIPE_HDR_CAPB";
    let name = "PRECOMP_WIN_PIPE_HDR_CAPB_ILUT_SFCLOAD";
    let field = t
        .f(caps, name)
        .ok_or_else(|| Missing(format!("NV{caps:04X}_{name}")))?;
    let yes = t
        .v(caps, &format!("{name}_TRUE"))
        .ok_or_else(|| Missing(format!("NV{caps:04X}_{name}_TRUE")))?;
    let count = t
        .v(caps, &format!("{register}__SIZE_1"))
        .filter(|count| windows <= *count && *count as usize <= PAGE / 4)
        .ok_or_else(|| Missing(format!("NV{caps:04X}_{register}__SIZE_1 bound")))?;
    let (high, low) = field;
    if high >= 32 || low > high || u64::from(yes) >= (1_u64 << (high - low + 1)) {
        return Err(Missing(format!("NV{caps:04X}_{name} field/value bound")));
    }
    for i in 0..windows.min(count) {
        let off = t
            .a(caps, register, i)
            .filter(|off| *off % 4 == 0 && (*off as usize) < PAGE)
            .ok_or_else(|| Missing(format!("NV{caps:04X}_{register}({i}) page bound")))?;
        if let Some((_, word)) = p.words.iter_mut().find(|(at, _)| *at == off) {
            *word = put(*word, field, yes);
        } else {
            p.words.push((off, put(0, field, yes)));
        }
    }
    p.words.sort_unstable_by_key(|(off, _)| *off);
    Ok(p)
}

/// Constructor-only TMO surface-loading discriminator, layered on the ILUT probe.
/// Adds only source-defined TMO surface loading, retaining DIRECT and size/count fields.
/// Must use `Engine::new_constructor_probe`: no TMO methods or LUT work are implemented.
pub fn tmo_surface_constructor_probe_page(
    t: &ClassTable,
    r: &Regs,
    caps: u32,
    heads: u32,
    windows: u32,
) -> Result<CapsPage, Missing> {
    let mut p = ilut_constructor_probe_page(t, r, caps, heads, windows)?;
    let register = "PRECOMP_WIN_PIPE_HDR_CAPD";
    let name = "PRECOMP_WIN_PIPE_HDR_CAPD_TMO_SFCLOAD";
    let field = t
        .f(caps, name)
        .ok_or_else(|| Missing(format!("NV{caps:04X}_{name}")))?;
    let yes = t
        .v(caps, &format!("{name}_TRUE"))
        .ok_or_else(|| Missing(format!("NV{caps:04X}_{name}_TRUE")))?;
    let count = t
        .v(caps, &format!("{register}__SIZE_1"))
        .filter(|count| windows <= *count && *count as usize <= PAGE / 4)
        .ok_or_else(|| Missing(format!("NV{caps:04X}_{register}__SIZE_1 bound")))?;
    let (high, low) = field;
    if high >= 32 || low != high || yes == 0 || u64::from(yes) >= (1_u64 << (high - low + 1)) {
        return Err(Missing(format!("NV{caps:04X}_{name} field/value bound")));
    }
    for i in 0..windows.min(count) {
        let off = t
            .a(caps, register, i)
            .filter(|off| *off % 4 == 0 && (*off as usize) < PAGE)
            .ok_or_else(|| Missing(format!("NV{caps:04X}_{register}({i}) page bound")))?;
        if let Some((_, word)) = p.words.iter_mut().find(|(at, _)| *at == off) {
            *word = put(*word, field, yes);
        } else {
            p.words.push((off, put(0, field, yes)));
        }
    }
    p.words.sort_unstable_by_key(|(off, _)| *off);
    Ok(p)
}

/// Constructor-only OLUT surface-loading discriminator, layered on the TMO probe.
/// Adds only source-defined OLUT surface loading.
/// Must use `Engine::new_constructor_probe`: no methods or LUT work are implemented.
pub fn olut_constructor_probe_page(
    t: &ClassTable,
    r: &Regs,
    caps: u32,
    heads: u32,
    windows: u32,
) -> Result<CapsPage, Missing> {
    let mut p = tmo_surface_constructor_probe_page(t, r, caps, heads, windows)?;
    let register = "POSTCOMP_HEAD_HDR_CAPB";
    let name = "POSTCOMP_HEAD_HDR_CAPB_OLUT_SFCLOAD";
    let field = t
        .f(caps, name)
        .ok_or_else(|| Missing(format!("NV{caps:04X}_{name}")))?;
    let yes = t
        .v(caps, &format!("{name}_TRUE"))
        .ok_or_else(|| Missing(format!("NV{caps:04X}_{name}_TRUE")))?;
    let count = t
        .v(caps, &format!("{register}__SIZE_1"))
        .filter(|count| heads <= *count && *count as usize <= PAGE / 4)
        .ok_or_else(|| Missing(format!("NV{caps:04X}_{register}__SIZE_1 bound")))?;
    let (high, low) = field;
    if high >= 32 || low != high || yes == 0 || u64::from(yes) >= (1_u64 << (high - low + 1)) {
        return Err(Missing(format!("NV{caps:04X}_{name} field/value bound")));
    }
    for i in 0..heads.min(count) {
        let off = t
            .a(caps, register, i)
            .filter(|off| *off % 4 == 0 && (*off as usize) < PAGE)
            .ok_or_else(|| Missing(format!("NV{caps:04X}_{register}({i}) page bound")))?;
        if let Some((_, word)) = p.words.iter_mut().find(|(at, _)| *at == off) {
            *word = put(*word, field, yes);
        } else {
            p.words.push((off, put(0, field, yes)));
        }
    }
    p.words.sort_unstable_by_key(|(off, _)| *off);
    Ok(p)
}

/// The caps registers of every family, by the header's own names (class prefix dropped). Used to read a
/// page back field by field (`diff_pages`); the authoring code names the ones it writes itself.
const CAP_REGISTERS: [&str; 27] = [
    "SYS_CAP",
    "SYS_CAPB",
    "SYS_CAPC",
    "SYS_CAPD",
    "SYS_CAPE",
    "IHUB_COMMON_CAPA",
    "IHUB_COMMON_CAPB",
    "IHUB_COMMON_CAPC",
    "IHUB_COMMON_CAPD",
    "IHUB_COMMON_CAPE",
    "IHUB_COMMON_CAPF",
    "SOR_CAP",
    "SOR_CLK_CAP",
    "POSTCOMP_HEAD_HDR_CAPA",
    "POSTCOMP_HEAD_HDR_CAPB",
    "POSTCOMP_HEAD_HDR_CAPC",
    "POSTCOMP_HEAD_HDR_CAPD",
    "POSTCOMP_HEAD_HDR_CAPE",
    "POSTCOMP_HEAD_HDR_CAPF",
    "PRECOMP_WIN_PIPE_HDR_CAPA",
    "PRECOMP_WIN_PIPE_HDR_CAPB",
    "PRECOMP_WIN_PIPE_HDR_CAPC",
    "PRECOMP_WIN_PIPE_HDR_CAPD",
    "PRECOMP_WIN_PIPE_HDR_CAPE",
    "PRECOMP_WIN_PIPE_HDR_CAPF",
    "HEAD_CLK_CAP",
    "WINDOW_CAP",
];

/// One field that differs between two pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDiff {
    /// Byte offset of the word in the page.
    pub offset: u32,
    /// The register's header name (`PRECOMP_WIN_PIPE_HDR_CAPA`), or `?` for a word no class header names.
    pub reg: String,
    /// The register's array index, when it is an array.
    pub index: Option<u32>,
    /// The field's header name, `(undefined bits)` for bits no field covers, `word` for an unnamed word.
    pub field: String,
    /// The field's value in the first page.
    pub a: u32,
    /// The field's value in the second page.
    pub b: u32,
}

/// Every named register word of the class: `(byte offset, register, index)`.
fn register_words(t: &ClassTable, caps: u32) -> Vec<(u32, &'static str, Option<u32>)> {
    let mut v = Vec::new();
    for reg in CAP_REGISTERS {
        let size = t.v(caps, &format!("{reg}__SIZE_1")).unwrap_or(1);
        if let Some(off) = t.v(caps, reg) {
            v.push((off, reg, None));
        } else {
            for i in 0..size {
                if let Some(off) = t.a(caps, reg, i) {
                    v.push((off, reg, Some(i)));
                }
            }
        }
    }
    v
}

/// ★ Compare two pages of one caps class FIELD BY FIELD with the class header's own field names: every
/// differing field of every named register, bits no field names, and words no header names.
#[must_use]
pub fn diff_pages(t: &ClassTable, caps: u32, a: &CapsPage, b: &CapsPage) -> Vec<FieldDiff> {
    use crate::class::Row;
    let named = register_words(t, caps);
    let mut offs: std::collections::BTreeSet<u32> = a.words.iter().map(|(o, _)| *o).collect();
    offs.extend(b.words.iter().map(|(o, _)| *o));
    let mut out = Vec::new();
    for off in offs {
        let (wa, wb) = (a.word(off), b.word(off));
        if wa == wb {
            continue;
        }
        let Some((_, reg, index)) = named.iter().find(|(o, _, _)| *o == off) else {
            out.push(FieldDiff {
                offset: off,
                reg: "?".into(),
                index: None,
                field: "word".into(),
                a: wa,
                b: wb,
            });
            continue;
        };
        let mut covered = 0_u32;
        for (name, row) in t.rows_under(caps, &format!("{reg}_")) {
            let Row::F(hi, lo) = row else { continue };
            if hi > 31 || lo > hi {
                continue;
            }
            let mask = (u32::MAX >> (31 - u32::from(hi))) & (u32::MAX << u32::from(lo));
            covered |= mask;
            let (fa, fb) = (get(wa, (hi, lo)), get(wb, (hi, lo)));
            if fa != fb {
                out.push(FieldDiff {
                    offset: off,
                    reg: (*reg).into(),
                    index: *index,
                    field: name[reg.len() + 1..].into(),
                    a: fa,
                    b: fb,
                });
            }
        }
        let rest = (wa ^ wb) & !covered;
        if rest != 0 {
            out.push(FieldDiff {
                offset: off,
                reg: (*reg).into(),
                index: *index,
                field: "(undefined bits)".into(),
                a: wa & rest,
                b: wb & rest,
            });
        }
    }
    out
}

/// Author the ordinary capabilities page from the family's generated layout.
/// Returns [`Missing`] for an absent definition or an out-of-page register.
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
    // NVDisplay's tiled output model (nvkms-evo4.c, EvoGetCapabilitiesCA /
    // UsableHwTilesCount) requires contiguous usable tiles. Our virtual output
    // has one independent, unscaled tile per head; no multi-tile output is
    // advertised. This is virtual topology, not a physical die's fuse mask.
    if t.fa(caps, "SYS_CAPC_TILE_EXISTS", 0).is_some() {
        let off = v("SYS_CAPC")?;
        let limit = v("SYS_CAPC_TILE_EXISTS__SIZE_1")?;
        if heads > limit || off % 4 != 0 || off as usize > PAGE - 4 {
            return Err(m("SYS_CAPC virtual tile topology outside layout".into()));
        }
        for h in 0..heads {
            set(off, fa("SYS_CAPC_TILE_EXISTS", h)?, 1);
        }
    }
    // ★ 2026-10-20 (bw-resume): the per-TILE post-composition pipe of a class that names it
    // `POSTCOMP_HDR_CAPA(i)` (CA73; Turing..Ada name a per-head `POSTCOMP_HEAD_HDR_*` and are untouched).
    // Public NVKMS reads it for every usable tile and asserts OCSC0/OCSC1/HCLPF/OSCAN/DTH/DSC present
    // (ogkm 580.65.06 nvkms-evo4.c:1826-1831, EvoGetHwTileType) — a class invariant, so it is stated
    // for each virtual tile; SCLR and VFILTER stay absent (tile type 1: the virtual tile is unscaled, as
    // the SYS_CAPC block above says). [measured GB202 Windows 580.88] without these words the Windows
    // driver abandons display init after its cursor channels (Code 43).
    if let Some(size) = t.v(caps, "POSTCOMP_HDR_CAPA__SIZE_1") {
        if heads > size {
            return Err(m("POSTCOMP_HDR_CAPA tiles outside layout".into()));
        }
        for i in 0..heads {
            let off = a("POSTCOMP_HDR_CAPA", i)?;
            for fld in [
                "POSTCOMP_HDR_CAPA_OCSC0_PRESENT",
                "POSTCOMP_HDR_CAPA_OCSC1_PRESENT",
                "POSTCOMP_HDR_CAPA_HCLPF_PRESENT",
                "POSTCOMP_HDR_CAPA_OSCAN_PRESENT",
                "POSTCOMP_HDR_CAPA_DTH_PRESENT",
                "POSTCOMP_HDR_CAPA_DSC_PRESENT",
            ] {
                set(off, f(fld)?, v(&format!("{fld}_TRUE"))?);
            }
        }
    }
    // SORs
    let tmds_max = tmds_max_field(t)?;
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
    // ★ 2026-10-14 (`V3_DISPLAY_PLANES_ORACLE.md`): semi-planar YUV windows. kayfabe's engine executes a
    // window of any format (its UPDATE/notifier/latch path is format-blind) and the console composes the
    // semi-planar formats (`scanout::ScanFormats`, `cuda/display/kf_yuv.cu`), so the page says so. NVKMS
    // reads it per class (`nvkms-evo3.c` nvEvoParseCapabilityNotifier6 / EvoParseCapabilityNotifierC5):
    // C673+ `IHUB_COMMON_CAPC.SUPPORT_SEMI_PLANAR`; C573, which has no CAPC, `IHUB_COMMON_CAPA.
    // SUPPORT_PLANAR` ("on Turing ... actually reports whether IHUB supports YUV _semi-planar_ formats").
    // [measured, AD104 under 580.159.04, the plane oracle's bare-metal reference] the real page sets it:
    // nvidia-drm lists NV12/NV21/NV16/NV61/NV24/NV42/P010/P012/P210 on every plane; without the bit the
    // kayfabe guest listed none. 3-plane (`CAPA.SUPPORT_PLANAR` on C673+) stays unclaimed, as on that board.
    let semi = t
        .v(caps, "IHUB_COMMON_CAPC")
        .zip(t.f(caps, "IHUB_COMMON_CAPC_SUPPORT_SEMI_PLANAR"))
        .zip(t.v(caps, "IHUB_COMMON_CAPC_SUPPORT_SEMI_PLANAR_TRUE"))
        .or_else(|| {
            (t.v(caps, "IHUB_COMMON_CAPC").is_none())
                .then(|| {
                    t.v(caps, "IHUB_COMMON_CAPA")
                        .zip(t.f(caps, "IHUB_COMMON_CAPA_SUPPORT_PLANAR"))
                        .zip(t.v(caps, "IHUB_COMMON_CAPA_SUPPORT_PLANAR_TRUE"))
                })
                .flatten()
        });
    if let Some(((off, field), yes)) = semi {
        set(off, field, yes);
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

    #[test]
    fn blackwell_virtual_heads_have_contiguous_single_output_tiles() {
        let t = crate::class::for_version("580.159.04").unwrap();
        let row = &kf_chip::display::BLACKWELL_GB20X;
        let r = Regs::for_ip("580.159.04", row.ip_version).unwrap();
        let p = page(t, &r, row.classes.caps, row.heads, row.windows).unwrap();
        let c = row.classes.caps;
        let word = p.word(t.v(c, "SYS_CAPC").unwrap());
        for i in 0..t.v(c, "SYS_CAPC_TILE_EXISTS__SIZE_1").unwrap() {
            assert_eq!(
                get(word, t.fa(c, "SYS_CAPC_TILE_EXISTS", i).unwrap()),
                u32::from(i < row.heads),
                "tile {i}"
            );
            assert_eq!(
                get(
                    word,
                    t.fa(c, "SYS_CAPC_TILE_SUPPORT_MULTI_TILE", i).unwrap()
                ),
                0
            );
        }
    }

    /// ★ 2026-10-14: every family's authored page claims semi-planar YUV windows in the field its NVKMS
    /// parser reads (C673+ CAPC.SUPPORT_SEMI_PLANAR; Turing CAPA.SUPPORT_PLANAR), and never claims
    /// 3-plane YUV (C673+ CAPA.SUPPORT_PLANAR) — what a real AD104 under 580.159.04 offers.
    #[test]
    fn the_page_claims_semi_planar_and_never_three_plane_yuv() {
        for version in ["580.65.06", "580.159.04"] {
            let t = crate::class::for_version(version).unwrap();
            for row in kf_chip::display::ALL {
                let r = Regs::for_ip(version, row.ip_version).unwrap();
                let c = row.classes.caps;
                let p = page(t, &r, c, row.heads, row.windows).unwrap();
                let capa = p.word(t.v(c, "IHUB_COMMON_CAPA").unwrap());
                let planar =
                    crate::class::get(capa, t.f(c, "IHUB_COMMON_CAPA_SUPPORT_PLANAR").unwrap());
                if let Some(capc) = t.v(c, "IHUB_COMMON_CAPC") {
                    let semi = crate::class::get(
                        p.word(capc),
                        t.f(c, "IHUB_COMMON_CAPC_SUPPORT_SEMI_PLANAR").unwrap(),
                    );
                    assert_eq!((semi, planar), (1, 0), "{version} NV{c:04X}");
                } else {
                    assert_eq!(c, 0xC573, "only Turing reads semi-planar from CAPA");
                    assert_eq!(planar, 1, "{version} NV{c:04X}");
                }
            }
        }
    }

    #[test]
    fn sdr_caps_declare_real_table_limits_with_tmo_in_every_cell() {
        for version in ["580.65.06", "580.159.04"] {
            let t = crate::class::for_version(version).unwrap();
            for row in kf_chip::display::ALL {
                let r = Regs::for_ip(version, row.ip_version).unwrap();
                let c = row.classes.caps;
                let p = sdr_page(t, &r, c, row.classes.core, row.heads, row.windows).unwrap();
                for i in 0..row.windows {
                    let word = p.word(t.a(c, "PRECOMP_WIN_PIPE_HDR_CAPB", i).unwrap());
                    for (suffix, v) in [("LOGSZ", 10), ("LOGNR", 0), ("DIRECT", 1), ("SFCLOAD", 1)]
                    {
                        assert_eq!(
                            crate::class::get(
                                word,
                                t.f(c, &format!("PRECOMP_WIN_PIPE_HDR_CAPB_ILUT_{suffix}"))
                                    .unwrap()
                            ),
                            v
                        );
                    }
                    let a = p.word(t.a(c, "PRECOMP_WIN_PIPE_HDR_CAPA", i).unwrap());
                    assert_eq!(
                        crate::class::get(
                            a,
                            t.f(c, "PRECOMP_WIN_PIPE_HDR_CAPA_TMO_PRESENT").unwrap()
                        ),
                        1
                    );
                }
                if t.f(c, "POSTCOMP_HEAD_HDR_CAPB_OLUT_SFCLOAD").is_some() {
                    for h in 0..row.heads {
                        let word = p.word(t.a(c, "POSTCOMP_HEAD_HDR_CAPB", h).unwrap());
                        assert_eq!(
                            crate::class::get(
                                word,
                                t.f(c, "POSTCOMP_HEAD_HDR_CAPB_OLUT_LOGSZ").unwrap()
                            ),
                            10
                        );
                        assert_eq!(
                            crate::class::get(
                                word,
                                t.f(c, "POSTCOMP_HEAD_HDR_CAPB_OLUT_SFCLOAD").unwrap()
                            ),
                            1
                        );
                    }
                } else {
                    assert!(
                        t.a(row.classes.core, "HEAD_SET_SURFACE_ADDRESS_LO_OLUT", 0)
                            .is_some()
                    );
                }
            }
        }
    }

    #[test]
    fn olut_probe_changes_only_its_surface_load_bit_in_every_display_cell() {
        for version in ["580.65.06", "580.159.04"] {
            let t = crate::class::for_version(version).unwrap();
            for row in kf_chip::display::ALL {
                let r = Regs::for_ip(version, row.ip_version).unwrap();
                let caps = row.classes.caps;
                let ordinary = page(t, &r, caps, row.heads, row.windows).unwrap();
                let previous =
                    tmo_surface_constructor_probe_page(t, &r, caps, row.heads, row.windows)
                        .unwrap();
                let probe = olut_constructor_probe_page(t, &r, caps, row.heads, row.windows);
                let Some(field) = t.f(caps, "POSTCOMP_HEAD_HDR_CAPB_OLUT_SFCLOAD") else {
                    // Blackwell's source omits this field. An earlier family's
                    // discriminator cannot be substituted for it.
                    assert_eq!(caps, 0xCA73);
                    assert_eq!(
                        probe,
                        Err(Missing("NVCA73_POSTCOMP_HEAD_HDR_CAPB_OLUT_SFCLOAD".into()))
                    );
                    continue;
                };
                let probe = probe.unwrap();
                let yes = t
                    .v(caps, "POSTCOMP_HEAD_HDR_CAPB_OLUT_SFCLOAD_TRUE")
                    .unwrap();
                let offsets: Vec<_> = (0..row.heads)
                    .map(|i| t.a(caps, "POSTCOMP_HEAD_HDR_CAPB", i).unwrap())
                    .collect();
                assert_eq!(probe.base, previous.base);
                for offset in (0..PAGE as u32).step_by(4) {
                    let expected = if offsets.contains(&offset) {
                        assert_eq!(crate::class::get(ordinary.word(offset), field), 0);
                        assert_eq!(crate::class::get(previous.word(offset), field), 0);
                        put(previous.word(offset), field, yes)
                    } else {
                        previous.word(offset)
                    };
                    assert_eq!(
                        probe.word(offset),
                        expected,
                        "{version}/{caps:#x}/{offset:#x}"
                    );
                }
            }
        }
    }

    #[test]
    fn olut_probe_refuses_missing_or_invalid_derived_layout() {
        let raw = include_str!("../data/classes-580.65.06.tsv");
        let r = Regs::for_ip("580.65.06", kf_chip::display::ADA.ip_version).unwrap();
        for missing in [
            "F\tNVC773_POSTCOMP_HEAD_HDR_CAPB_OLUT_SFCLOAD\t",
            "V\tNVC773_POSTCOMP_HEAD_HDR_CAPB_OLUT_SFCLOAD_TRUE\t",
            "A\tNVC773_POSTCOMP_HEAD_HDR_CAPB\t",
            "V\tNVC773_POSTCOMP_HEAD_HDR_CAPB__SIZE_1\t",
        ] {
            let text = raw
                .lines()
                .filter(|line| !line.starts_with(missing))
                .collect::<Vec<_>>()
                .join("\n");
            let t = ClassTable::parse(&text);
            assert!(tmo_surface_constructor_probe_page(&t, &r, 0xC773, 4, 8).is_ok());
            assert!(
                olut_constructor_probe_page(&t, &r, 0xC773, 4, 8).is_err(),
                "{missing}"
            );
        }
        for extra in [
            "A\tNVC773_POSTCOMP_HEAD_HDR_CAPB\t4096\t32",
            "A\tNVC773_POSTCOMP_HEAD_HDR_CAPB\t1669\t32",
            "V\tNVC773_POSTCOMP_HEAD_HDR_CAPB__SIZE_1\t3",
            "V\tNVC773_POSTCOMP_HEAD_HDR_CAPB__SIZE_1\t1025",
            "F\tNVC773_POSTCOMP_HEAD_HDR_CAPB_OLUT_SFCLOAD\t32\t32",
            "F\tNVC773_POSTCOMP_HEAD_HDR_CAPB_OLUT_SFCLOAD\t9\t8",
            "V\tNVC773_POSTCOMP_HEAD_HDR_CAPB_OLUT_SFCLOAD_TRUE\t0",
            "V\tNVC773_POSTCOMP_HEAD_HDR_CAPB_OLUT_SFCLOAD_TRUE\t2",
        ] {
            let t = ClassTable::parse(&format!("{raw}\n{extra}\n"));
            assert!(
                olut_constructor_probe_page(&t, &r, 0xC773, 4, 8).is_err(),
                "{extra}"
            );
        }
    }

    #[test]
    fn tmo_surface_probe_changes_only_derived_capd_bit_in_every_display_cell() {
        for version in ["580.65.06", "580.159.04"] {
            let t = crate::class::for_version(version).unwrap();
            for row in kf_chip::display::ALL {
                let r = Regs::for_ip(version, row.ip_version).unwrap();
                let ordinary = page(t, &r, row.classes.caps, row.heads, row.windows).unwrap();
                let ilut =
                    ilut_constructor_probe_page(t, &r, row.classes.caps, row.heads, row.windows)
                        .unwrap();
                let probe = tmo_surface_constructor_probe_page(
                    t,
                    &r,
                    row.classes.caps,
                    row.heads,
                    row.windows,
                )
                .unwrap();
                let field = t
                    .f(row.classes.caps, "PRECOMP_WIN_PIPE_HDR_CAPD_TMO_SFCLOAD")
                    .unwrap();
                let yes = t
                    .v(
                        row.classes.caps,
                        "PRECOMP_WIN_PIPE_HDR_CAPD_TMO_SFCLOAD_TRUE",
                    )
                    .unwrap();
                let offsets: Vec<_> = (0..row.windows)
                    .map(|i| {
                        t.a(row.classes.caps, "PRECOMP_WIN_PIPE_HDR_CAPD", i)
                            .unwrap()
                    })
                    .collect();
                assert_eq!(probe.base, ilut.base);
                for offset in (0..PAGE as u32).step_by(4) {
                    let expected = if offsets.contains(&offset) {
                        assert_eq!(crate::class::get(ilut.word(offset), field), 0);
                        put(ilut.word(offset), field, yes)
                    } else {
                        ilut.word(offset)
                    };
                    assert_eq!(
                        probe.word(offset),
                        expected,
                        "{version}/{:?} {offset:#x}",
                        row.classes
                    );
                }
                assert_eq!(
                    ordinary,
                    page(t, &r, row.classes.caps, row.heads, row.windows).unwrap()
                );
                assert_eq!(
                    ilut,
                    ilut_constructor_probe_page(t, &r, row.classes.caps, row.heads, row.windows)
                        .unwrap()
                );
            }
        }
    }

    #[test]
    fn tmo_surface_probe_refuses_missing_or_invalid_derived_layout() {
        let raw = include_str!("../data/classes-580.65.06.tsv");
        let r = Regs::for_ip("580.65.06", kf_chip::display::ADA.ip_version).unwrap();
        for missing in [
            "F\tNVC773_PRECOMP_WIN_PIPE_HDR_CAPD_TMO_SFCLOAD\t",
            "V\tNVC773_PRECOMP_WIN_PIPE_HDR_CAPD_TMO_SFCLOAD_TRUE\t",
            "A\tNVC773_PRECOMP_WIN_PIPE_HDR_CAPD\t",
            "V\tNVC773_PRECOMP_WIN_PIPE_HDR_CAPD__SIZE_1\t",
        ] {
            let text = raw
                .lines()
                .filter(|line| !line.starts_with(missing))
                .collect::<Vec<_>>()
                .join("\n");
            let t = ClassTable::parse(&text);
            assert!(ilut_constructor_probe_page(&t, &r, 0xC773, 4, 8).is_ok());
            assert!(
                tmo_surface_constructor_probe_page(&t, &r, 0xC773, 4, 8).is_err(),
                "{missing}"
            );
        }
        for extra in [
            "A\tNVC773_PRECOMP_WIN_PIPE_HDR_CAPD\t4096\t32",
            "A\tNVC773_PRECOMP_WIN_PIPE_HDR_CAPD\t1933\t32",
            "V\tNVC773_PRECOMP_WIN_PIPE_HDR_CAPD__SIZE_1\t7",
            "F\tNVC773_PRECOMP_WIN_PIPE_HDR_CAPD_TMO_SFCLOAD\t32\t32",
            "F\tNVC773_PRECOMP_WIN_PIPE_HDR_CAPD_TMO_SFCLOAD\t9\t8",
            "V\tNVC773_PRECOMP_WIN_PIPE_HDR_CAPD_TMO_SFCLOAD_TRUE\t0",
            "V\tNVC773_PRECOMP_WIN_PIPE_HDR_CAPD_TMO_SFCLOAD_TRUE\t2",
        ] {
            let t = ClassTable::parse(&format!("{raw}\n{extra}\n"));
            assert!(
                tmo_surface_constructor_probe_page(&t, &r, 0xC773, 4, 8).is_err(),
                "{extra}"
            );
        }
    }

    #[test]
    fn ilut_probe_changes_only_generated_surface_load_bit_in_every_display_cell() {
        for version in ["580.65.06", "580.159.04"] {
            let t = crate::class::for_version(version).unwrap();
            for row in kf_chip::display::ALL {
                let r = Regs::for_ip(version, row.ip_version).unwrap();
                let ordinary = page(t, &r, row.classes.caps, row.heads, row.windows).unwrap();
                let tmo = constructor_probe_page(t, &r, row.classes.caps, row.heads, row.windows)
                    .unwrap();
                let ilut =
                    ilut_constructor_probe_page(t, &r, row.classes.caps, row.heads, row.windows)
                        .unwrap();
                let field = t
                    .f(row.classes.caps, "PRECOMP_WIN_PIPE_HDR_CAPB_ILUT_SFCLOAD")
                    .unwrap();
                let yes = t
                    .v(
                        row.classes.caps,
                        "PRECOMP_WIN_PIPE_HDR_CAPB_ILUT_SFCLOAD_TRUE",
                    )
                    .unwrap();
                let offsets: Vec<_> = (0..row.windows)
                    .map(|i| {
                        t.a(row.classes.caps, "PRECOMP_WIN_PIPE_HDR_CAPB", i)
                            .unwrap()
                    })
                    .collect();
                for offset in (0..PAGE as u32).step_by(4) {
                    let expected = if offsets.contains(&offset) {
                        assert_eq!(crate::class::get(ordinary.word(offset), field), 0);
                        put(tmo.word(offset), field, yes)
                    } else {
                        tmo.word(offset)
                    };
                    assert_eq!(
                        ilut.word(offset),
                        expected,
                        "{version}/{:?} {offset:#x}",
                        row.classes
                    );
                }
                // Authoring a probe does not mutate a table or alter subsequent default pages.
                assert_eq!(
                    ordinary,
                    page(t, &r, row.classes.caps, row.heads, row.windows).unwrap()
                );
            }
        }
    }

    #[test]
    fn ilut_probe_refuses_missing_or_invalid_derived_layout() {
        let raw = include_str!("../data/classes-580.65.06.tsv");
        let r = Regs::for_ip("580.65.06", kf_chip::display::ADA.ip_version).unwrap();
        for missing in [
            "F\tNVC773_PRECOMP_WIN_PIPE_HDR_CAPB_ILUT_SFCLOAD\t",
            "V\tNVC773_PRECOMP_WIN_PIPE_HDR_CAPB_ILUT_SFCLOAD_TRUE\t",
            "A\tNVC773_PRECOMP_WIN_PIPE_HDR_CAPB\t",
            "V\tNVC773_PRECOMP_WIN_PIPE_HDR_CAPB__SIZE_1\t",
        ] {
            let text = raw
                .lines()
                .filter(|l| !l.starts_with(missing))
                .collect::<Vec<_>>()
                .join("\n");
            let t = ClassTable::parse(&text);
            assert!(constructor_probe_page(&t, &r, 0xC773, 4, 8).is_ok());
            assert!(
                ilut_constructor_probe_page(&t, &r, 0xC773, 4, 8).is_err(),
                "{missing}"
            );
        }
        for extra in [
            "A\tNVC773_PRECOMP_WIN_PIPE_HDR_CAPB\t4096\t32",
            "A\tNVC773_PRECOMP_WIN_PIPE_HDR_CAPB\t1925\t32",
            "V\tNVC773_PRECOMP_WIN_PIPE_HDR_CAPB__SIZE_1\t7",
            "F\tNVC773_PRECOMP_WIN_PIPE_HDR_CAPB_ILUT_SFCLOAD\t32\t32",
        ] {
            let t = ClassTable::parse(&format!("{raw}\n{extra}\n"));
            assert!(
                ilut_constructor_probe_page(&t, &r, 0xC773, 4, 8).is_err(),
                "{extra}"
            );
        }
    }

    #[test]
    fn constructor_probe_changes_only_derived_tmo_fields_in_every_display_cell() {
        for version in ["580.65.06", "580.159.04"] {
            let t = crate::class::for_version(version).unwrap();
            for row in kf_chip::display::ALL {
                let r = Regs::for_ip(version, row.ip_version).unwrap();
                let normal = page(t, &r, row.classes.caps, row.heads, row.windows).unwrap();
                let probe = constructor_probe_page(t, &r, row.classes.caps, row.heads, row.windows)
                    .unwrap();
                let field = t
                    .f(row.classes.caps, "PRECOMP_WIN_PIPE_HDR_CAPA_TMO_PRESENT")
                    .unwrap();
                let yes = t
                    .v(
                        row.classes.caps,
                        "PRECOMP_WIN_PIPE_HDR_CAPA_TMO_PRESENT_TRUE",
                    )
                    .unwrap();
                let mut restored = probe.clone();
                for i in 0..row.windows {
                    let off = t
                        .a(row.classes.caps, "PRECOMP_WIN_PIPE_HDR_CAPA", i)
                        .unwrap();
                    assert_eq!(crate::class::get(normal.word(off), field), 0);
                    assert_eq!(crate::class::get(probe.word(off), field), yes);
                    let (_, word) = restored
                        .words
                        .iter_mut()
                        .find(|(at, _)| *at == off)
                        .unwrap();
                    *word = put(*word, field, 0);
                }
                assert_eq!(normal, restored);
            }
        }
    }

    #[test]
    fn constructor_probe_refuses_missing_capability_semantics() {
        let raw = include_str!("../data/classes-580.65.06.tsv");
        let missing = raw
            .lines()
            .filter(|l| !l.contains("TMO_PRESENT_TRUE"))
            .collect::<Vec<_>>()
            .join("\n");
        let t = ClassTable::parse(&missing);
        let r = Regs::for_ip("580.65.06", kf_chip::display::ADA.ip_version).unwrap();
        assert!(page(&t, &r, 0xC773, 4, 8).is_ok());
        assert!(constructor_probe_page(&t, &r, 0xC773, 4, 8).is_err());
    }

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
