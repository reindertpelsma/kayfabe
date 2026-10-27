//! Bounded GR bind intent. Decoding/authoring is added by the tests-first adapter change.

use crate::DriverVersion;

/// A logical guest target, never a host handle authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    /// Logical owner client.
    pub client: u32,
    /// Logical channel or TSG.
    pub channel: u32,
    /// Scalar mode and buffer intent.
    pub kind: Kind,
}

/// SDK bind variants; the maximum pointer array is a public ABI bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Zcull save/restore state.
    Zcull { va: u64, mode: u32 },
    /// PM save/restore state.
    Pm { va: u64, mode: u32 },
    /// Graphics/compute preemption state.
    Preemption {
        flags: u32,
        gfxp: u32,
        cilp: u32,
        buffers: [u64; 9],
    },
}

// These IDs are the public SDK ctrl0080fifo.h enumeration, identical in580/595.
// They are selectors, not captured per-die sizes.
/// Zcull context buffer selector.
pub const ZCULL: u32 = 8;
/// PM context buffer selector.
pub const PM: u32 = 9;
/// Compute preemption buffer selector.
pub const COMPUTE: u32 = 10;
/// Graphics preemption buffer selector.
pub const GRAPHICS: u32 = 11;
/// Preemption pointer-slot selectors; slot7 is a CPU pointer, not a GPU VA.
pub const PREEMPT: [Option<u32>; 9] = [
    Some(GRAPHICS),
    Some(12),
    Some(13),
    Some(14),
    Some(15),
    Some(21),
    Some(22),
    None,
    Some(25),
];

use crate::{generated::matrix as m, matrix::Resolved};

fn schema(v: DriverVersion, cmd: u32) -> Option<Resolved> {
    let rows = [
        (
            &m::CTRL_CMDS_NV2080_CTRL_CMD_GR_CTXSW_ZCULL_BIND,
            &m::NV2080_CTRL_GR_CTXSW_ZCULL_BIND_PARAMS,
        ),
        (
            &m::CTRL_CMDS_NV2080_CTRL_CMD_GR_CTXSW_PM_BIND,
            &m::NV2080_CTRL_GR_CTXSW_PM_BIND_PARAMS,
        ),
        (
            &m::CTRL_CMDS_NV2080_CTRL_CMD_GR_CTXSW_PREEMPTION_BIND,
            &m::NV2080_CTRL_GR_CTXSW_PREEMPTION_BIND_PARAMS,
        ),
    ];
    rows.into_iter().find_map(|(id, layout)| {
        (id.at_u32(v).ok().flatten()? == cmd)
            .then(|| Resolved::of(layout, v).ok())
            .flatten()
    })
}
fn range(l: &Resolved, name: &'static str, bytes: usize) -> Option<std::ops::Range<usize>> {
    let f = l.need(name).ok()?;
    (f.bytes()? == bytes && f.off().checked_add(bytes)? <= l.size())
        .then_some(f.off()..f.off() + bytes)
}
fn word(l: &Resolved, p: &[u8], name: &'static str) -> Option<u32> {
    Some(u32::from_le_bytes(
        p.get(range(l, name, 4)?)?.try_into().ok()?,
    ))
}
fn quad(l: &Resolved, p: &[u8], name: &'static str) -> Option<u64> {
    Some(u64::from_le_bytes(
        p.get(range(l, name, 8)?)?.try_into().ok()?,
    ))
}

/// Decode one bounded union member. The registration layer validates the outer union size.
/// Unknown layouts, short members and nonzero engine routes refuse.
pub fn decode(v: DriverVersion, cmd: u32, bytes: &[u8]) -> Option<Request> {
    let l = schema(v, cmd)?;
    let p = bytes.get(..l.size())?;
    if l.need("grRouteInfo").is_ok()
        && (word(&l, p, "grRouteInfo.flags")? != 0 || quad(&l, p, "grRouteInfo.route")? != 0)
    {
        return None;
    }
    let client = word(&l, p, "hClient")?;
    let channel = word(&l, p, "hChannel")?;
    let kind = if l.need("zcullMode").is_ok() {
        Kind::Zcull {
            va: quad(&l, p, "vMemPtr")?,
            mode: word(&l, p, "zcullMode")?,
        }
    } else if l.need("pmMode").is_ok() {
        Kind::Pm {
            va: quad(&l, p, "vMemPtr")?,
            mode: word(&l, p, "pmMode")?,
        }
    } else {
        let f = l.need("vMemPtrs").ok()?;
        let count = f.bytes()? / 8;
        if ![8, 9].contains(&count) {
            return None;
        }
        let mut buffers = [0; 9];
        for (i, out) in buffers.iter_mut().enumerate().take(count) {
            let at = f.off().checked_add(i * 8)?;
            *out = u64::from_le_bytes(p.get(at..at + 8)?.try_into().ok()?);
        }
        Kind::Preemption {
            flags: word(&l, p, "flags")?,
            gfxp: word(&l, p, "gfxpPreemptMode")?,
            cilp: word(&l, p, "cilpPreemptMode")?,
            buffers,
        }
    };
    Some(Request {
        client,
        channel,
        kind,
    })
}

/// Encode fresh host bytes from bounded intent. Caller supplies owned client/channel handles.
pub fn encode(v: DriverVersion, request: Request) -> Option<(u32, Vec<u8>)> {
    let id = match request.kind {
        Kind::Zcull { .. } => &m::CTRL_CMDS_NV2080_CTRL_CMD_GR_CTXSW_ZCULL_BIND,
        Kind::Pm { .. } => &m::CTRL_CMDS_NV2080_CTRL_CMD_GR_CTXSW_PM_BIND,
        Kind::Preemption { .. } => &m::CTRL_CMDS_NV2080_CTRL_CMD_GR_CTXSW_PREEMPTION_BIND,
    }
    .at_u32(v)
    .ok()
    .flatten()?;
    let l = schema(v, id)?;
    let mut p = vec![0; l.size()];
    let mut put = |name, value: &[u8]| -> Option<()> {
        p.get_mut(range(&l, name, value.len())?)?
            .copy_from_slice(value);
        Some(())
    };
    put("hClient", &request.client.to_le_bytes())?;
    put("hChannel", &request.channel.to_le_bytes())?;
    match request.kind {
        Kind::Zcull { va, mode } | Kind::Pm { va, mode } => {
            put("vMemPtr", &va.to_le_bytes())?;
            let name = if matches!(request.kind, Kind::Pm { .. }) {
                "pmMode"
            } else {
                "zcullMode"
            };
            put(name, &mode.to_le_bytes())?;
        }
        Kind::Preemption {
            flags,
            gfxp,
            cilp,
            buffers,
        } => {
            put("flags", &flags.to_le_bytes())?;
            put("gfxpPreemptMode", &gfxp.to_le_bytes())?;
            put("cilpPreemptMode", &cilp.to_le_bytes())?;
            let f = l.need("vMemPtrs").ok()?;
            let count = f.bytes()? / 8;
            if ![8, 9].contains(&count) || buffers[count..].iter().any(|p| *p != 0) {
                return None;
            }
            for (i, va) in buffers.iter().enumerate().take(count) {
                let at = f.off().checked_add(i * 8)?;
                p.get_mut(at..at + 8)?.copy_from_slice(&va.to_le_bytes());
            }
        }
    }
    Some((id, p))
}

/// Host-authored NP size query: no guest route or prepopulated size.
pub fn properties_query(v: DriverVersion, engine: u32) -> Option<(u32, Vec<u8>)> {
    let l = Resolved::of(&m::NV2080_CTRL_GR_GET_ENGINE_CONTEXT_PROPERTIES_PARAMS, v).ok()?;
    let cmd = m::CTRL_CMDS_NV2080_CTRL_CMD_GR_GET_ENGINE_CONTEXT_PROPERTIES
        .at_u32(v)
        .ok()
        .flatten()?;
    let mut p = vec![0; l.size()];
    p.get_mut(range(&l, "engineId", 4)?)?
        .copy_from_slice(&engine.to_le_bytes());
    Some((cmd, p))
}

/// Decode host query output only after a successful native status.
pub fn properties_reply(v: DriverVersion, p: &[u8]) -> Option<(u64, u64)> {
    let l = Resolved::of(&m::NV2080_CTRL_GR_GET_ENGINE_CONTEXT_PROPERTIES_PARAMS, v).ok()?;
    if p.len() != l.size() || p.get(range(&l, "bInfoPopulated", 1)?)? != [1] {
        return None;
    }
    Some((
        u64::from(word(&l, p, "size")?),
        u64::from(word(&l, p, "alignment")?),
    ))
}
