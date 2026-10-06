//! Bounded SDR colour programs. Method addresses and fields come from the class table.
//! LUT bytes remain on the GPU: this module plans reads, never transforms guest pixels.

use crate::class::{ClassTable, get};
use crate::inst::{CtxDma, Target};

/// Four 8-byte VSS header entries and 1025 8-byte RGB/padding entries (nvkms-types.h).
pub const LUT_BYTES: u64 = (4 + 1025) * 8;

/// A LUT's guest binding, before resolving the instance-memory context DMA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    /// Turing through Ada: byte offset inside a context DMA.
    Dma {
        /// Guest context-DMA handle, resolved with the owning client/channel.
        handle: u32,
        /// Byte offset inside the resolved context DMA.
        offset: u64,
    },
    /// GB20x: physical video-memory address. Other targets are refused.
    Vidmem(u64),
}

impl Binding {
    /// Bound every header and endpoint byte, including checked address arithmetic.
    pub fn span(self, dma: Option<&CtxDma>) -> Result<u64, &'static str> {
        self.span_bytes(dma, LUT_BYTES)
    }

    /// Resolve a bounded authored table extent, including the header and endpoint.
    pub fn span_bytes(self, dma: Option<&CtxDma>, bytes: u64) -> Result<u64, &'static str> {
        if !(40..=LUT_BYTES).contains(&bytes) || !bytes.is_multiple_of(8) {
            return Err("LUT extent outside fixed bound");
        }
        let addr = match self {
            Self::Vidmem(addr) => addr
                .checked_add(bytes)
                .map(|_| addr)
                .ok_or("LUT address overflow"),
            Self::Dma { offset, .. } => {
                let d = dma.ok_or("LUT context DMA missing")?;
                if d.target != Target::Vidmem || d.block_linear {
                    return Err("LUT requires linear video memory");
                }
                d.span(offset, bytes).ok_or("LUT exceeds context DMA")
            }
        }?;
        if addr & 7 != 0 {
            return Err("LUT entry address is misaligned");
        }
        Ok(addr)
    }
}

/// Bounded LUT binding: direct input/output table or segmented linear tone table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lut {
    /// Number of sample entries, including the endpoint, excluding four header entries.
    pub entries: u32,
    /// Context-DMA or physical video-memory source.
    pub binding: Binding,
    /// Interpolate between adjacent entries instead of taking the lower entry.
    pub interpolate: bool,
}

/// An output transform: signed coefficients stored in the class's encoded S5.14 format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Output {
    /// Output table, or bypass when disabled.
    pub lut: Option<Lut>,
    /// Encoded coefficients have two trailing zeros, so decode by dividing by 65536.
    pub matrix: [i32; 12],
}

/// Identity matrix in the coefficient word's encoded units.
pub const IDENTITY: [i32; 12] = [65536, 0, 0, 0, 0, 65536, 0, 0, 0, 0, 65536, 0];

#[derive(Debug, Clone, Copy)]
enum Address {
    Dma {
        handle: u32,
        offset: u32,
        shift: u32,
    },
    Physical {
        hi: u32,
        lo: u32,
        target: (u8, u8),
        nvm: u32,
        enable: (u8, u8),
    },
}

impl Address {
    fn resolve(t: &ClassTable, c: u32, head: Option<u32>, name: &str, shift: u32) -> Option<Self> {
        let method = |n: &str| head.map_or_else(|| t.v(c, n), |h| t.a(c, n, h));
        if let (Some(handle), Some(offset)) = (
            method(&format!(
                "{}SET_CONTEXT_DMA_{name}",
                if head.is_some() { "HEAD_" } else { "" }
            )),
            method(&format!(
                "{}SET_OFFSET_{name}",
                if head.is_some() { "HEAD_" } else { "" }
            )),
        ) {
            return Some(Self::Dma {
                handle,
                offset,
                shift,
            });
        }
        let prefix = if head.is_some() { "HEAD_" } else { "" };
        let lo = format!("{prefix}SET_SURFACE_ADDRESS_LO_{name}");
        Some(Self::Physical {
            hi: method(&format!("{prefix}SET_SURFACE_ADDRESS_HI_{name}"))?,
            lo: method(&lo)?,
            target: t.f(c, &format!("{lo}_TARGET"))?,
            nvm: t.v(c, &format!("{lo}_TARGET_PHYSICAL_NVM"))?,
            enable: t.f(c, &format!("{lo}_ENABLE"))?,
        })
    }

    fn read(self, read: &impl Fn(u32) -> u32) -> Result<Option<Binding>, &'static str> {
        match self {
            Self::Dma {
                handle,
                offset,
                shift,
            } => {
                let h = read(handle);
                Ok((h != 0).then(|| Binding::Dma {
                    handle: h,
                    offset: u64::from(read(offset)) << shift,
                }))
            }
            Self::Physical {
                hi,
                lo,
                target,
                nvm,
                enable,
            } => {
                let low = read(lo);
                if get(low, enable) == 0 {
                    return Ok(None);
                }
                if get(low, target) != nvm {
                    return Err("LUT physical target is not video memory");
                }
                Ok(Some(Binding::Vidmem(
                    (u64::from(read(hi)) << 32) | u64::from(low & !15),
                )))
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Control {
    method: u32,
    size: (u8, u8),
    mode: (u8, u8),
    direct10: u32,
    interpolate: (u8, u8),
    mirror: (u8, u8),
}

impl Control {
    fn resolve(t: &ClassTable, c: u32, head: Option<u32>, name: &str) -> Option<Self> {
        Some(Self {
            method: head.map_or_else(|| t.v(c, name), |h| t.a(c, name, h))?,
            size: t.f(c, &format!("{name}_SIZE"))?,
            mode: t.f(c, &format!("{name}_MODE"))?,
            direct10: t.v(c, &format!("{name}_MODE_DIRECT10"))?,
            interpolate: t.f(c, &format!("{name}_INTERPOLATE"))?,
            mirror: t.f(c, &format!("{name}_MIRROR"))?,
        })
    }
    fn read(self, binding: Binding, read: &impl Fn(u32) -> u32) -> Result<Lut, &'static str> {
        let v = read(self.method);
        let mask = |(hi, lo): (u8, u8)| (u32::MAX >> (31 - hi + lo)) << lo;
        let supported = mask(self.size) | mask(self.mode) | mask(self.interpolate);
        if get(v, self.size) != 1029
            || get(v, self.mode) != self.direct10
            || get(v, self.mirror) != 0
            || v & !supported != 0
        {
            return Err("LUT requires 1029-entry unmirrored DIRECT10");
        }
        Ok(Lut {
            entries: 1025,
            binding,
            interpolate: get(v, self.interpolate) != 0,
        })
    }
}

/// Decode the FP16 input table. The surrounding stages are decoded by `pipeline`.
pub fn input(
    t: &ClassTable,
    c: u32,
    read: impl Fn(u32) -> u32,
) -> Result<Option<Lut>, &'static str> {
    let mut fmt = [0; 12];
    for (i, v) in fmt.iter_mut().enumerate() {
        let name = format!("SET_FMT_COEFFICIENT_C{}{}", i / 4, i % 4);
        *v = read(t.v(c, &name).ok_or("missing FMT vocabulary")?);
    }
    let a = Address::resolve(t, c, None, "ILUT", 0).ok_or("missing ILUT vocabulary")?;
    let binding = a.read(&read)?;
    if fmt != IDENTITY.map(|v| v as u32) {
        return Err("nonidentity FMT is outside SDR subset");
    }
    let ctl = Control::resolve(t, c, None, "SET_ILUT_CONTROL").ok_or("missing ILUT control")?;
    binding.map(|b| ctl.read(b, &read)).transpose()
}

/// An indexed inline CSC table. Fixed extents never depend on guest data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineLut {
    /// Log2 samples per segment: 33 logarithmic input or 64 linear output segments.
    pub segments: [Option<u8>; 64],
    /// UNORM16 CSC0 or FP16 CSC1 entries, including the interpolation endpoint.
    pub entries: [Option<u16>; 1025],
}
impl Default for InlineLut {
    fn default() -> Self {
        Self {
            segments: [None; 64],
            entries: [None; 1025],
        }
    }
}

/// The stages surrounding TMO, in hardware order CSC00, CSC01, CSC10, CSC11.
#[derive(Debug, Clone)]
pub struct Pipeline {
    /// Signed S5.14 coefficients in the source's encoded S5.16 units.
    pub matrices: [[i32; 12]; 4],
    /// CSC0 logarithmic FP16-to-UNORM16 and CSC1 linear UNORM16-to-FP16 LUTs.
    pub inline: [Option<InlineLut>; 2],
    /// Linear 64-segment UNORM16 intensity curve, or bypass.
    pub tmo: Option<Lut>,
}

/// Decode a fully armed program, refusing unsupported controls and incomplete tables.
pub fn pipeline(
    t: &ClassTable,
    c: u32,
    read: impl Fn(u32) -> u32,
    inline: &[InlineLut; 2],
) -> Result<Pipeline, &'static str> {
    let mut p = Pipeline {
        matrices: [IDENTITY; 4],
        inline: [None, None],
        tmo: None,
    };
    for (stage, matrix) in p.matrices.iter_mut().enumerate() {
        let name = format!("SET_CSC{}{}CONTROL", stage / 2, stage % 2);
        let ctl = read(t.v(c, &name).ok_or("missing CSC control")?);
        let enable = t
            .f(c, &format!("{name}_ENABLE"))
            .ok_or("missing CSC enable")?;
        let mask = ((1_u32 << (enable.0 - enable.1 + 1)) - 1) << enable.1;
        if ctl & !mask != 0 {
            return Err("unsupported CSC control bits");
        }
        if get(ctl, enable) == 0 {
            continue;
        }
        for (i, v) in matrix.iter_mut().enumerate() {
            let n = format!(
                "SET_CSC{}{}COEFFICIENT_C{}{}",
                stage / 2,
                stage % 2,
                i / 4,
                i % 4
            );
            let f = t
                .f(c, &format!("{n}_VALUE"))
                .ok_or("missing CSC coefficient field")?;
            let bits = u32::from(f.0 - f.1 + 1);
            let raw = read(t.v(c, &n).ok_or("missing CSC coefficient")?);
            let word = get(raw, f);
            if raw != word << f.1 {
                return Err("unsupported CSC coefficient bits");
            }
            *v = ((word << (32 - bits)) as i32) >> (32 - bits);
        }
    }
    for (stage, table) in inline.iter().enumerate() {
        let name = format!("SET_CSC{stage}LUT_CONTROL");
        let ctl = read(t.v(c, &name).ok_or("missing CSC LUT control")?);
        let en = t
            .f(c, &format!("{name}_ENABLE"))
            .ok_or("missing CSC LUT enable")?;
        let interp = t
            .f(c, &format!("{name}_INTERPOLATE"))
            .ok_or("missing CSC LUT interpolation")?;
        let mirror = t
            .f(c, &format!("{name}_MIRROR"))
            .ok_or("missing CSC LUT mirror")?;
        let supported = (1 << en.1) | (1 << interp.1);
        if ctl & !supported != 0 || get(ctl, mirror) != 0 {
            return Err("unsupported CSC LUT control");
        }
        if get(ctl, en) == 0 {
            continue;
        }
        if get(ctl, interp) != 1 {
            return Err("CSC LUT interpolation required");
        }
        let count = if stage == 0 { 33 } else { 64 };
        let mut entries = 0_usize;
        for seg in &table.segments[..count] {
            let log = seg.ok_or("CSC LUT segment uninitialized")?;
            if log > 7 {
                return Err("CSC LUT segment out of range");
            }
            entries += 1_usize << log;
        }
        if entries > 1024 || table.entries[..=entries].iter().any(Option::is_none) {
            return Err("CSC LUT incomplete or exceeds fixed extent");
        }
        // CSC1 outputs FP16. No infinities, NaNs or negatives may reach composition.
        if stage == 1
            && table.entries[..=entries]
                .iter()
                .any(|v| v.is_some_and(|h| h >= 0x7c00))
        {
            return Err("CSC1 LUT requires finite nonnegative FP16");
        }
        p.inline[stage] = Some(table.clone());
    }
    let name = if t.v(c, "SET_CONTEXT_DMA_TMO").is_some() {
        "TMO"
    } else {
        "TMO_LUT"
    };
    let address = Address::resolve(t, c, None, name, 8).ok_or("missing TMO vocabulary")?;
    if let Some(binding) = address.read(&read)? {
        let ctl = read(t.v(c, "SET_TMO_CONTROL").ok_or("missing TMO control")?);
        let field = |n: &str| {
            t.f(c, &format!("SET_TMO_CONTROL_{n}"))
                .ok_or("missing TMO field")
        };
        let size = field("SIZE")?;
        let interp = field("INTERPOLATE")?;
        let sat = field("SAT_MODE")?;
        let mask = |f: (u8, u8)| (u32::MAX >> (31 - f.0 + f.1)) << f.1;
        if !(69..=1029).contains(&get(ctl, size))
            || get(ctl, sat) != 2
            || ctl & !(mask(size) | mask(interp) | mask(sat)) != 0
        {
            return Err("TMO requires bounded linear no-correction program");
        }
        // OGKM TMO_LUT_SETTINGS_NO_CORRECTION. Other chroma correction policies refuse.
        for (method, fields) in [
            ("SET_TMO_LOW_INTENSITY_ZONE", vec![("END", 1280)]),
            (
                "SET_TMO_LOW_INTENSITY_VALUE",
                vec![
                    ("LIN_WEIGHT", 256),
                    ("NON_LIN_WEIGHT", 256),
                    ("THRESHOLD", 255),
                ],
            ),
            (
                "SET_TMO_MEDIUM_INTENSITY_ZONE",
                vec![("START", 4960), ("END", 4961)],
            ),
            (
                "SET_TMO_MEDIUM_INTENSITY_VALUE",
                vec![
                    ("LIN_WEIGHT", 256),
                    ("NON_LIN_WEIGHT", 256),
                    ("THRESHOLD", 255),
                ],
            ),
            ("SET_TMO_HIGH_INTENSITY_ZONE", vec![("START", 10640)]),
            (
                "SET_TMO_HIGH_INTENSITY_VALUE",
                vec![
                    ("LIN_WEIGHT", 256),
                    ("NON_LIN_WEIGHT", 256),
                    ("THRESHOLD", 255),
                ],
            ),
        ] {
            let word = read(t.v(c, method).ok_or("missing TMO zone method")?);
            let mut expected = 0;
            for (suffix, value) in fields {
                let f = t
                    .f(c, &format!("{method}_{suffix}"))
                    .ok_or("missing TMO zone field")?;
                expected = crate::class::put(expected, f, value);
            }
            if word != expected {
                return Err("TMO chroma correction outside supported subset");
            }
        }
        p.tmo = Some(Lut {
            entries: get(ctl, size) - 4,
            binding,
            interpolate: get(ctl, interp) != 0,
        });
    }
    Ok(p)
}

/// Decode one head. Core class arrays, including GB20x's different stride, are derived.
pub fn output(
    t: &ClassTable,
    c: u32,
    head: u32,
    read: impl Fn(u32) -> u32,
) -> Result<Output, &'static str> {
    let a = Address::resolve(t, c, Some(head), "OLUT", 8).ok_or("missing OLUT vocabulary")?;
    let ctl = Control::resolve(t, c, Some(head), "HEAD_SET_OLUT_CONTROL")
        .ok_or("missing OLUT control")?;
    let lut = a.read(&read)?.map(|b| ctl.read(b, &read)).transpose()?;
    let mut matrix = IDENTITY;
    for stage in [0, 1] {
        let name = format!("HEAD_SET_OCSC{stage}CONTROL");
        let m = t.a(c, &name, head).ok_or("missing output CSC vocabulary")?;
        let enable = t
            .f(c, &format!("{name}_ENABLE"))
            .ok_or("missing output CSC enable")?;
        if get(read(m), enable) == 0 {
            continue;
        }
        if stage != 0 {
            return Err("active OCSC1 is outside SDR subset");
        }
        for (i, v) in matrix.iter_mut().enumerate() {
            let n = format!("HEAD_SET_OCSC0COEFFICIENT_C{}{}", i / 4, i % 4);
            let f = t
                .f(c, &format!("{n}_VALUE"))
                .ok_or("missing OCSC coefficient field")?;
            let bits = u32::from(f.0 - f.1 + 1);
            let word = get(read(t.a(c, &n, head).ok_or("missing OCSC coefficient")?), f);
            *v = ((word << (32 - bits)) as i32) >> (32 - bits);
        }
    }
    if lut.is_some() {
        let norm = t
            .a(c, "HEAD_SET_OLUT_FP_NORM_SCALE", head)
            .ok_or("missing OLUT normalization")?;
        if read(norm) != u32::MAX {
            return Err("nonunity OLUT normalization is outside SDR subset");
        }
    }
    Ok(Output { lut, matrix })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn word(t: &ClassTable, c: u32, name: &str, head: Option<u32>) -> u32 {
        head.map_or_else(|| t.v(c, name), |h| t.a(c, name, h))
            .unwrap()
    }
    fn set(
        b: &mut HashMap<u32, u32>,
        t: &ClassTable,
        c: u32,
        name: &str,
        head: Option<u32>,
        v: u32,
    ) {
        b.insert(word(t, c, name, head), v);
    }
    fn control(t: &ClassTable, c: u32, n: &str, interp: bool) -> u32 {
        crate::class::put(
            crate::class::put(
                u32::from(interp),
                t.f(c, &format!("{n}_MODE")).unwrap(),
                t.v(c, &format!("{n}_MODE_DIRECT10")).unwrap(),
            ),
            t.f(c, &format!("{n}_SIZE")).unwrap(),
            1029,
        )
    }
    fn bind(b: &mut HashMap<u32, u32>, t: &ClassTable, c: u32, h: Option<u32>, n: &str, off: u32) {
        let p = if h.is_some() { "HEAD_" } else { "" };
        let dma = format!("{p}SET_CONTEXT_DMA_{n}");
        if h.map_or_else(|| t.v(c, &dma), |h| t.a(c, &dma, h))
            .is_some()
        {
            set(b, t, c, &dma, h, 0x123);
            set(b, t, c, &format!("{p}SET_OFFSET_{n}"), h, off);
        } else {
            let lo = format!("{p}SET_SURFACE_ADDRESS_LO_{n}");
            let v = crate::class::put(
                0x12340,
                t.f(c, &format!("{lo}_TARGET")).unwrap(),
                t.v(c, &format!("{lo}_TARGET_PHYSICAL_NVM")).unwrap(),
            ) | 1;
            set(b, t, c, &lo, h, v);
            set(b, t, c, &format!("{p}SET_SURFACE_ADDRESS_HI_{n}"), h, 1);
        }
    }

    #[test]
    fn every_family_and_table_decodes_real_bindings_and_units() {
        for version in ["580.65.06", "580.159.04"] {
            let t = crate::class::for_version(version).unwrap();
            for (win, core) in [
                (0xc57e, 0xc57d),
                (0xc67e, 0xc67d),
                (0xc67e, 0xc77d),
                (0xca7e, 0xca7d),
            ] {
                let mut b = HashMap::new();
                for (i, v) in IDENTITY.iter().enumerate() {
                    set(
                        &mut b,
                        t,
                        win,
                        &format!("SET_FMT_COEFFICIENT_C{}{}", i / 4, i % 4),
                        None,
                        *v as u32,
                    );
                }
                bind(&mut b, t, win, None, "ILUT", 0x101);
                set(
                    &mut b,
                    t,
                    win,
                    "SET_ILUT_CONTROL",
                    None,
                    control(t, win, "SET_ILUT_CONTROL", false),
                );
                let i = input(t, win, |m| b.get(&m).copied().unwrap_or(0))
                    .unwrap()
                    .unwrap();
                if win != 0xca7e {
                    assert_eq!(
                        i.binding,
                        Binding::Dma {
                            handle: 0x123,
                            offset: 0x101
                        },
                        "ILUT offset is bytes"
                    );
                } else {
                    assert_eq!(i.binding, Binding::Vidmem(0x100012340));
                }
                assert!(!i.interpolate);
                bind(&mut b, t, core, Some(3), "OLUT", 0x21);
                set(
                    &mut b,
                    t,
                    core,
                    "HEAD_SET_OLUT_CONTROL",
                    Some(3),
                    control(t, core, "HEAD_SET_OLUT_CONTROL", true),
                );
                set(
                    &mut b,
                    t,
                    core,
                    "HEAD_SET_OLUT_FP_NORM_SCALE",
                    Some(3),
                    u32::MAX,
                );
                let o = output(t, core, 3, |m| b.get(&m).copied().unwrap_or(0)).unwrap();
                assert!(o.lut.unwrap().interpolate);
                if core != 0xca7d {
                    assert_eq!(
                        o.lut.unwrap().binding,
                        Binding::Dma {
                            handle: 0x123,
                            offset: 0x2100
                        }
                    );
                }
                assert_eq!(o.matrix, IDENTITY);
                set(
                    &mut b,
                    t,
                    win,
                    "SET_ILUT_CONTROL",
                    None,
                    control(t, win, "SET_ILUT_CONTROL", false) | 2,
                );
                assert!(
                    input(t, win, |m| b.get(&m).copied().unwrap_or(0)).is_err(),
                    "mirror is not silently accepted"
                );
                set(&mut b, t, core, "HEAD_SET_OLUT_FP_NORM_SCALE", Some(3), 1);
                assert!(output(t, core, 3, |m| b.get(&m).copied().unwrap_or(0)).is_err());
            }
        }
    }

    #[test]
    fn lut_bounds_include_header_endpoint_and_refuse_system_memory() {
        let b = Binding::Dma {
            handle: 1,
            offset: 256,
        };
        let mut d = CtxDma {
            target: Target::Vidmem,
            base: 4096,
            limit: 4096 + 256 + LUT_BYTES - 1,
            block_linear: false,
            writable: false,
        };
        assert_eq!(b.span(Some(&d)), Ok(4352));
        d.limit -= 1;
        assert!(b.span(Some(&d)).is_err());
        d.limit += 1;
        d.target = Target::Sysmem;
        assert!(b.span(Some(&d)).is_err());
        assert!(Binding::Vidmem(u64::MAX - 4).span(None).is_err());
    }

    #[test]
    fn ocsc_signed_coefficients_and_rounding_bias_are_preserved() {
        let t = crate::class::for_version("580.159.04").unwrap();
        let c = 0xc77d;
        let mut b = HashMap::new();
        set(&mut b, t, c, "HEAD_SET_OCSC0CONTROL", Some(3), 1);
        for (i, v) in IDENTITY.iter().enumerate() {
            set(
                &mut b,
                t,
                c,
                &format!("HEAD_SET_OCSC0COEFFICIENT_C{}{}", i / 4, i % 4),
                Some(3),
                *v as u32,
            );
        }
        set(&mut b, t, c, "HEAD_SET_OCSC0COEFFICIENT_C03", Some(3), 32);
        set(
            &mut b,
            t,
            c,
            "HEAD_SET_OCSC0COEFFICIENT_C01",
            Some(3),
            (-65536i32) as u32 & 0x1fffff,
        );
        let o = output(t, c, 3, |m| b.get(&m).copied().unwrap_or(0)).unwrap();
        assert_eq!((o.matrix[1], o.matrix[3]), (-65536, 32));
        set(&mut b, t, c, "HEAD_SET_OCSC1CONTROL", Some(3), 1);
        assert!(output(t, c, 3, |m| b.get(&m).copied().unwrap_or(0)).is_err());
        assert!(
            input(&ClassTable::default(), 0xc67e, |_| 0).is_err(),
            "missing vocabulary fails closed"
        );
    }
    #[test]
    fn tmo_controls_and_bindings_decode_in_every_family_and_refuse_other_chroma_policies() {
        for version in ["580.65.06", "580.159.04"] {
            let t = crate::class::for_version(version).unwrap();
            for c in [0xc57e, 0xc67e, 0xca7e] {
                let mut b = HashMap::new();
                let name = if t.v(c, "SET_CONTEXT_DMA_TMO").is_some() {
                    "TMO"
                } else {
                    "TMO_LUT"
                };
                bind(&mut b, t, c, None, name, 0x20);
                let ctl = crate::class::put(
                    crate::class::put(1, t.f(c, "SET_TMO_CONTROL_SIZE").unwrap(), 1029),
                    t.f(c, "SET_TMO_CONTROL_SAT_MODE").unwrap(),
                    2,
                );
                set(&mut b, t, c, "SET_TMO_CONTROL", None, ctl);
                for (method, fields) in [
                    ("SET_TMO_LOW_INTENSITY_ZONE", vec![("END", 1280)]),
                    (
                        "SET_TMO_LOW_INTENSITY_VALUE",
                        vec![
                            ("LIN_WEIGHT", 256),
                            ("NON_LIN_WEIGHT", 256),
                            ("THRESHOLD", 255),
                        ],
                    ),
                    (
                        "SET_TMO_MEDIUM_INTENSITY_ZONE",
                        vec![("START", 4960), ("END", 4961)],
                    ),
                    (
                        "SET_TMO_MEDIUM_INTENSITY_VALUE",
                        vec![
                            ("LIN_WEIGHT", 256),
                            ("NON_LIN_WEIGHT", 256),
                            ("THRESHOLD", 255),
                        ],
                    ),
                    ("SET_TMO_HIGH_INTENSITY_ZONE", vec![("START", 10640)]),
                    (
                        "SET_TMO_HIGH_INTENSITY_VALUE",
                        vec![
                            ("LIN_WEIGHT", 256),
                            ("NON_LIN_WEIGHT", 256),
                            ("THRESHOLD", 255),
                        ],
                    ),
                ] {
                    let v = fields.into_iter().fold(0, |word, (suffix, value)| {
                        crate::class::put(
                            word,
                            t.f(c, &format!("{method}_{suffix}")).unwrap(),
                            value,
                        )
                    });
                    set(&mut b, t, c, method, None, v);
                }
                let tables = std::array::from_fn(|_| InlineLut::default());
                let p = pipeline(t, c, |m| b.get(&m).copied().unwrap_or(0), &tables).unwrap();
                assert!(p.tmo.unwrap().interpolate);
                if c != 0xca7e {
                    assert_eq!(
                        p.tmo.unwrap().binding,
                        Binding::Dma {
                            handle: 0x123,
                            offset: 0x2000
                        }
                    );
                }
                for size in [69, 261, 1029] {
                    set(
                        &mut b,
                        t,
                        c,
                        "SET_TMO_CONTROL",
                        None,
                        crate::class::put(ctl, t.f(c, "SET_TMO_CONTROL_SIZE").unwrap(), size),
                    );
                    let p = pipeline(t, c, |m| b.get(&m).copied().unwrap_or(0), &tables).unwrap();
                    assert_eq!(p.tmo.unwrap().entries, size - 4);
                }
                for size in [0, 68, 1030, 2047] {
                    set(
                        &mut b,
                        t,
                        c,
                        "SET_TMO_CONTROL",
                        None,
                        crate::class::put(ctl, t.f(c, "SET_TMO_CONTROL_SIZE").unwrap(), size),
                    );
                    assert!(pipeline(t, c, |m| b.get(&m).copied().unwrap_or(0), &tables).is_err());
                }
                set(
                    &mut b,
                    t,
                    c,
                    "SET_TMO_CONTROL",
                    None,
                    ctl ^ (1 << t.f(c, "SET_TMO_CONTROL_SAT_MODE").unwrap().1),
                );
                assert!(pipeline(t, c, |m| b.get(&m).copied().unwrap_or(0), &tables).is_err());
            }
        }
    }

    #[test]
    fn inline_program_requires_complete_bounded_initialized_tables() {
        let t = crate::class::for_version("580.159.04").unwrap();
        let c = 0xc67e;
        let mut b = HashMap::new();
        set(&mut b, t, c, "SET_CSC1LUT_CONTROL", None, 17);
        let mut tables = std::array::from_fn(|_| InlineLut::default());
        assert!(pipeline(t, c, |m| b.get(&m).copied().unwrap_or(0), &tables).is_err());
        tables[1].segments.fill(Some(0));
        tables[1].entries[..65].fill(Some(0));
        assert!(pipeline(t, c, |m| b.get(&m).copied().unwrap_or(0), &tables).is_ok());
        tables[1].entries[2] = Some(0x7c00);
        assert!(pipeline(t, c, |m| b.get(&m).copied().unwrap_or(0), &tables).is_err());
        tables[1].entries[2] = Some(0);
        tables[1].segments.fill(Some(7));
        assert!(pipeline(t, c, |m| b.get(&m).copied().unwrap_or(0), &tables).is_err());
    }
}
