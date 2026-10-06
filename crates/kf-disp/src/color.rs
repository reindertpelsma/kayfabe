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
    Dma { handle: u32, offset: u64 },
    /// GB20x: physical video-memory address. Other targets are refused.
    Vidmem(u64),
}

impl Binding {
    /// Bound every header and endpoint byte, including checked address arithmetic.
    pub fn span(self, dma: Option<&CtxDma>) -> Result<u64, &'static str> {
        match self {
            Self::Vidmem(addr) => addr
                .checked_add(LUT_BYTES)
                .map(|_| addr)
                .ok_or("LUT address overflow"),
            Self::Dma { offset, .. } => {
                let d = dma.ok_or("LUT context DMA missing")?;
                if d.target != Target::Vidmem || d.block_linear {
                    return Err("LUT requires linear video memory");
                }
                d.span(offset, LUT_BYTES).ok_or("LUT exceeds context DMA")
            }
        }
    }
}

/// DIRECT10, unmirrored, fixed-size table, plus optional interpolation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lut {
    pub binding: Binding,
    pub interpolate: bool,
}

/// An output transform: signed coefficients stored in the class's encoded S5.14 format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Output {
    pub lut: Option<Lut>,
    /// Encoded coefficients have two trailing zeros, so decode by dividing by 65536.
    pub matrix: [i32; 12],
}

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
        if get(v, self.size) != 1029
            || get(v, self.mode) != self.direct10
            || get(v, self.mirror) != 0
        {
            return Err("LUT requires 1029-entry unmirrored DIRECT10");
        }
        Ok(Lut {
            binding,
            interpolate: get(v, self.interpolate) != 0,
        })
    }
}

/// Resolve and decode one window from its armed bank. Active CSC/TMO and nonidentity FMT
/// are deliberately outside the first SDR subset; never silently turn them into identity.
pub fn input(
    t: &ClassTable,
    c: u32,
    read: impl Fn(u32) -> u32,
) -> Result<Option<Lut>, &'static str> {
    for name in [
        "SET_CSC00CONTROL",
        "SET_CSC01CONTROL",
        "SET_CSC10CONTROL",
        "SET_CSC11CONTROL",
        "SET_CSC0LUT_CONTROL",
        "SET_CSC1LUT_CONTROL",
    ] {
        let m = t.v(c, name).ok_or("missing input CSC vocabulary")?;
        let enable = t
            .f(c, &format!("{name}_ENABLE"))
            .ok_or("missing input CSC enable")?;
        if get(read(m), enable) != 0 {
            return Err("active input CSC is outside SDR subset");
        }
    }
    let tmo = Address::resolve(
        t,
        c,
        None,
        if t.v(c, "SET_CONTEXT_DMA_TMO").is_some() {
            "TMO"
        } else {
            "TMO_LUT"
        },
        8,
    )
    .ok_or("missing TMO vocabulary")?;
    if tmo.read(&read)?.is_some() {
        return Err("active TMO is outside SDR subset");
    }
    let mut fmt = [0; 12];
    for (i, v) in fmt.iter_mut().enumerate() {
        let name = format!("SET_FMT_COEFFICIENT_C{}{}", i / 4, i % 4);
        *v = read(t.v(c, &name).ok_or("missing FMT vocabulary")?);
    }
    if fmt != IDENTITY.map(|v| v as u32) && fmt != [0; 12] {
        return Err("nonidentity FMT is outside SDR subset");
    }
    let a = Address::resolve(t, c, None, "ILUT", 0).ok_or("missing ILUT vocabulary")?;
    let ctl = Control::resolve(t, c, None, "SET_ILUT_CONTROL").ok_or("missing ILUT control")?;
    a.read(&read)?.map(|b| ctl.read(b, &read)).transpose()
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
