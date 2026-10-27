//! NP GR binds over already mapped USER VAs. No map, allocation or guest bytes are forwarded.

use crate::Channel;
use kf_abi::grbind::{self as abi, Kind, Request};

/// Ordinary host effects. Tests use an independent recording fake.
pub trait Host {
    /// Native minimum size/alignment for a generated engine identifier.
    fn properties(&mut self, engine: u32) -> Result<(u64, u64), u32>;
    /// An authored request whose handles have been replaced with owned handles.
    fn bind(&mut self, request: Request) -> Result<(), u32>;
}

/// Validate every nonzero pointer against native extent/alignment and confirmed writable rows.
/// The caller serializes placements and native channel Free; no row lock crosses a host call.
pub fn execute(
    host: &mut impl Host,
    request: Request,
    client: u32,
    channel: Channel,
    mut mapped: impl FnMut(u64, u64) -> bool,
) -> Result<u64, u32> {
    const INVALID: u32 = 0x1f;
    let mut specs = Vec::with_capacity(9);
    match request.kind {
        Kind::Zcull { va, mode } | Kind::Pm { va, mode } => {
            if mode > 2 || (mode != 0 && va == 0) {
                return Err(INVALID);
            }
            let engine = if matches!(request.kind, Kind::Pm { .. }) {
                abi::PM
            } else {
                abi::ZCULL
            };
            if va != 0 {
                specs.push((va, engine, None));
            }
        }
        Kind::Preemption {
            flags,
            gfxp,
            cilp,
            buffers,
        } => {
            if flags & !3 != 0
                || (flags & 2 != 0 && gfxp > 2)
                || (flags & 1 != 0 && cilp > 2)
                || buffers[7] != 0
            {
                return Err(INVALID);
            }
            for (i, va) in buffers.into_iter().enumerate().filter(|(_, va)| *va != 0) {
                let mut engine = abi::PREEMPT[i].ok_or(INVALID)?;
                let both = i == 0 && flags & 3 == 3 && gfxp != 0 && cilp != 0;
                if i == 0 && flags & 1 != 0 && cilp != 0 && !both {
                    engine = abi::COMPUTE;
                }
                specs.push((va, engine, both.then_some(abi::COMPUTE)));
            }
        }
    }
    let mut bytes = 0u64;
    for (va, engine, extra) in specs {
        let (mut len, mut align) = host.properties(engine)?;
        let valid = |len: u64, align: u64| {
            len != 0 && len <= 64 << 20 && align.is_power_of_two() && align <= 64 << 20
        };
        if !valid(len, align) {
            return Err(INVALID);
        }
        if let Some(engine) = extra {
            let (l, a) = host.properties(engine)?;
            if !valid(l, a) {
                return Err(INVALID);
            }
            len = len.max(l);
            align = align.max(a);
        }
        bytes = bytes
            .checked_add(len)
            .filter(|b| *b <= 128 << 20)
            .ok_or(INVALID)?;
        if va % align != 0 || va.checked_add(len).is_none() || !mapped(va, len) {
            return Err(INVALID);
        }
    }
    host.bind(Request {
        client,
        channel: channel.chan,
        ..request
    })?;
    Ok(bytes)
}

struct Native<'a>(&'a crate::HostRm);
impl Host for Native<'_> {
    fn properties(&mut self, engine: u32) -> Result<(u64, u64), u32> {
        let v = self.0.host_abi().version();
        let (cmd, mut p) = abi::properties_query(v, engine).ok_or(0x56u32)?;
        let status = self
            .0
            .raw_control_opaque(self.0.subdevice, cmd, &mut p)
            .map_err(|_| 0x40u32)?;
        if status != 0 {
            return Err(status);
        }
        abi::properties_reply(v, &p).ok_or(0x1fu32)
    }
    fn bind(&mut self, request: Request) -> Result<(), u32> {
        let (cmd, mut p) = abi::encode(self.0.host_abi().version(), request).ok_or(0x56u32)?;
        match self
            .0
            .raw_control_opaque(self.0.subdevice, cmd, &mut p)
            .map_err(|_| 0x40u32)?
        {
            0 => Ok(()),
            status => Err(status),
        }
    }
}
impl crate::HostRm {
    /// Worker-only GR bind with native status preserved; caller validates current placement rows.
    pub fn bind_gr_status(
        &self,
        request: Request,
        channel: Channel,
        mapped: impl FnMut(u64, u64) -> bool,
    ) -> Result<u64, u32> {
        execute(
            &mut Native(self),
            request,
            self.client.raw(),
            channel,
            mapped,
        )
    }
}
