//! ★ 2026-10-07 (Windows Code43, run34): the two `NV0080` FIFO controls a Windows kernel client
//! sends for each new kernel channel, read at the GUEST's measured layout.
//!
//! | control | flags (`g_device_nvoc.c`, 580.159.04) | what kayfabe does |
//! |---|---|---|
//! | `NV0080_CTRL_CMD_FIFO_GET_LATENCY_BUFFER_SIZE` | `0x50048`: NON_PRIVILEGED, ROUTE_TO_PHYSICAL, GSP_PLUGIN_FOR_VGPU_GSP, PHYSICAL_IMPLEMENTED_ON_VGPU_GUEST (`:640-654`) | answered from the HOST's own reply to the same control, asked at realize per advertised engine ([`LatencyRow`]) |
//! | `NV0080_CTRL_CMD_FIFO_SET_CHANNEL_PROPERTIES` | `0x10248`: NON_PRIVILEGED, ROUTE_TO_PHYSICAL, ROUTE_TO_VGPU_HOST, GSP_PLUGIN_FOR_VGPU_GSP (`:657-669`) | property `ENGINETIMESLICEINMICROSECONDS` only: a real timeslice on the channel's own host group (`kf_rm::chanlink`) |
//!
//! ⊘ Both are `ROUTE_TO_PHYSICAL`: a GSP-client RM has no body for them (the non-VF HAL of
//! `GET_LATENCY_BUFFER_SIZE` is `NV_ASSERT_PRECOMP(0); return NV_ERR_NOT_SUPPORTED`,
//! `ogkm-580.159.04: src/nvidia/generated/g_device_nvoc.h:1100-1103`, and
//! `deviceCtrlCmdFifoSetChannelProperties_IMPL` is in no open file). The only open body is the
//! vGPU guest's (`deviceCtrlCmdFifoGetLatencyBufferSize_VF`,
//! `src/nvidia/src/kernel/gpu/fifo/kernel_fifo_ctrl.c:983-1008`): it looks `engineID` up in a
//! per-engine table the vGPU host supplies, copies `gpEntries`/`pbEntries`, and refuses an engine
//! the table lacks with `NV_ERR_INVALID_ARGUMENT`. [`answer`] does the same over the host's table.
//!
//! Every command id, value and layout here comes from the driver matrix
//! (`crate::generated::matrix`, measured per ogkm tag); a version where one is absent gets
//! `None`, and the caller leaves the control to the links that answered it before.

use crate::DriverVersion;
use crate::generated::matrix as m;
use crate::matrix::Resolved;

/// `NV_ERR_INVALID_ARGUMENT` — the `_VF` body's answer for an engine its table lacks.
pub const NV_ERR_INVALID_ARGUMENT: u32 = 0x1f;

/// Where one version's `NV0080_CTRL_FIFO_GET_LATENCY_BUFFER_SIZE_PARAMS` puts its fields, and the
/// command id at that version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LatencyLayout {
    /// `NV0080_CTRL_CMD_FIFO_GET_LATENCY_BUFFER_SIZE` at the version.
    pub cmd: u32,
    /// `sizeof` the params.
    pub size: usize,
    /// `engineID` (`[IN]`, an `NV2080_ENGINE_TYPE`).
    pub engine_off: usize,
    /// `gpEntries` (`[OUT]`).
    pub gp_off: usize,
    /// `pbEntries` (`[OUT]`, 32-byte rows).
    pub pb_off: usize,
}

impl LatencyLayout {
    /// The layout at `version`; `None` where the control or its struct is not measured there.
    #[must_use]
    pub fn at(version: DriverVersion) -> Option<LatencyLayout> {
        let cmd = m::CTRL_CMDS_NV0080_CTRL_CMD_FIFO_GET_LATENCY_BUFFER_SIZE
            .at_u32(version)
            .ok()??;
        let l = Resolved::of(&m::NV0080_CTRL_FIFO_GET_LATENCY_BUFFER_SIZE_PARAMS, version).ok()?;
        Some(LatencyLayout {
            cmd,
            size: l.size(),
            engine_off: l.need("engineID").ok()?.off(),
            gp_off: l.need("gpEntries").ok()?.off(),
            pb_off: l.need("pbEntries").ok()?.off(),
        })
    }

    /// The request kayfabe authors for the host: `engineID = engine`, the outputs zero.
    #[must_use]
    pub fn request(&self, engine: u32) -> Vec<u8> {
        let mut p = vec![0u8; self.size];
        put(&mut p, self.engine_off, engine);
        p
    }

    /// The host's reply to [`Self::request`] as a row; `None` if it is short or names another
    /// engine than the one asked.
    #[must_use]
    pub fn reply(&self, asked: u32, bytes: &[u8]) -> Option<LatencyRow> {
        if bytes.len() != self.size || get(bytes, self.engine_off)? != asked {
            return None;
        }
        Some(LatencyRow {
            engine_id: asked,
            gp_entries: get(bytes, self.gp_off)?,
            pb_entries: get(bytes, self.pb_off)?,
        })
    }
}

/// One engine's latency-buffer sizing, as the HOST's RM answered it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LatencyRow {
    /// `engineID` asked (`NV2080_ENGINE_TYPE`).
    pub engine_id: u32,
    /// `gpEntries`.
    pub gp_entries: u32,
    /// `pbEntries`.
    pub pb_entries: u32,
}

/// Why a guest `GET_LATENCY_BUFFER_SIZE` was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatencyRefusal {
    /// The params are not the version's struct.
    Size(usize),
    /// The host gave no answer for this engine (the `_VF` body's INVALID_ARGUMENT case).
    NoHostRow(u32),
}

/// ★ Answer a guest request in place from the host's rows: `engineID` stays the guest's,
/// `gpEntries`/`pbEntries` are the host's for that engine.
///
/// # Errors
/// [`LatencyRefusal`].
pub fn answer(
    layout: &LatencyLayout,
    rows: &[LatencyRow],
    params: &mut [u8],
) -> Result<LatencyRow, LatencyRefusal> {
    if params.len() != layout.size {
        return Err(LatencyRefusal::Size(params.len()));
    }
    let engine = get(params, layout.engine_off).ok_or(LatencyRefusal::Size(params.len()))?;
    let row = *rows
        .iter()
        .find(|r| r.engine_id == engine)
        .ok_or(LatencyRefusal::NoHostRow(engine))?;
    put(params, layout.gp_off, row.gp_entries);
    put(params, layout.pb_off, row.pb_entries);
    Ok(row)
}

/// Where one version's `NV0080_CTRL_FIFO_SET_CHANNEL_PROPERTIES_PARAMS` puts its fields, the
/// command id, and the one property kayfabe serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelPropsLayout {
    /// `NV0080_CTRL_CMD_FIFO_SET_CHANNEL_PROPERTIES` at the version.
    pub cmd: u32,
    /// `sizeof` the params.
    pub size: usize,
    /// `hChannel`.
    pub channel_off: usize,
    /// `property`.
    pub property_off: usize,
    /// `value` (`NvU64`).
    pub value_off: usize,
    /// `NV0080_CTRL_FIFO_SET_CHANNEL_PROPERTIES_ENGINETIMESLICEINMICROSECONDS`.
    pub engine_timeslice_us: u32,
}

/// A decoded `SET_CHANNEL_PROPERTIES` request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelProperty {
    /// `hChannel`, in the caller's client.
    pub channel: u32,
    /// `property`.
    pub property: u32,
    /// `value`.
    pub value: u64,
}

impl ChannelPropsLayout {
    /// The layout at `version`; `None` where any part is not measured there.
    #[must_use]
    pub fn at(version: DriverVersion) -> Option<ChannelPropsLayout> {
        let cmd = m::CTRL_CMDS_NV0080_CTRL_CMD_FIFO_SET_CHANNEL_PROPERTIES
            .at_u32(version)
            .ok()??;
        let engine_timeslice_us =
            m::CTRL_VALUES_NV0080_CTRL_FIFO_SET_CHANNEL_PROPERTIES_ENGINETIMESLICEINMICROSECONDS
                .at_u32(version)
                .ok()??;
        let l = Resolved::of(&m::NV0080_CTRL_FIFO_SET_CHANNEL_PROPERTIES_PARAMS, version).ok()?;
        let value = l.need("value").ok()?;
        if value.bytes()? != 8 {
            return None;
        }
        Some(ChannelPropsLayout {
            cmd,
            size: l.size(),
            channel_off: l.need("hChannel").ok()?.off(),
            property_off: l.need("property").ok()?.off(),
            value_off: value.off(),
            engine_timeslice_us,
        })
    }

    /// Decode exactly the version's struct; `None` for any other length.
    #[must_use]
    pub fn decode(&self, params: &[u8]) -> Option<ChannelProperty> {
        if params.len() != self.size {
            return None;
        }
        let lo = get(params, self.value_off)?;
        let hi = get(params, self.value_off + 4)?;
        Some(ChannelProperty {
            channel: get(params, self.channel_off)?,
            property: get(params, self.property_off)?,
            value: u64::from(lo) | (u64::from(hi) << 32),
        })
    }
}

fn get(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn put(b: &mut [u8], at: usize, v: u32) {
    if let Some(s) = at.checked_add(4).and_then(|e| b.get_mut(at..e)) {
        s.copy_from_slice(&v.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::versions::BENCH_DRIVER;

    const V580_65_06: DriverVersion = DriverVersion {
        major: 580,
        minor: 65,
        patch: 6,
    };

    /// Both audited guest contracts and the bench carry the controls at the SDK's ids.
    #[test]
    fn both_controls_are_measured_at_the_guest_contracts() {
        for v in [V580_65_06, BENCH_DRIVER] {
            let l = LatencyLayout::at(v).expect("latency layout");
            assert_eq!(
                (l.size, l.engine_off, l.gp_off, l.pb_off),
                (12, 0, 4, 8),
                "{v}"
            );
            let p = ChannelPropsLayout::at(v).expect("channel props layout");
            assert_eq!(
                (p.size, p.channel_off, p.property_off, p.value_off),
                (16, 0, 4, 8),
                "{v}"
            );
            assert_eq!(p.engine_timeslice_us, 0);
            assert_eq!(l.cmd >> 8, p.cmd >> 8, "both in the NV0080 FIFO interface");
        }
    }

    /// `[measured vfio-10 2855, the 2026-10-05 VFIO boot, RTX 4070]` the real GPU's reply for COPY2 (`engineID 0xb`): `gpEntries 0x20`,
    /// `pbEntries 0xe00`. [`answer`] reproduces that shape from a host row and refuses an engine
    /// the host did not answer for.
    #[test]
    fn the_answer_is_the_host_row_for_the_guests_engine() {
        let l = LatencyLayout::at(V580_65_06).unwrap();
        let rows = [LatencyRow {
            engine_id: 0xb,
            gp_entries: 0x20,
            pb_entries: 0xe00,
        }];
        let mut p = l.request(0xb);
        answer(&l, &rows, &mut p).unwrap();
        assert_eq!(
            p,
            [0x0b, 0, 0, 0, 0x20, 0, 0, 0, 0x00, 0x0e, 0, 0],
            "vfio-10 2855 reply bytes"
        );
        let mut other = l.request(0x1);
        assert_eq!(
            answer(&l, &rows, &mut other),
            Err(LatencyRefusal::NoHostRow(1))
        );
        assert_eq!(
            answer(&l, &rows, &mut [0u8; 8]),
            Err(LatencyRefusal::Size(8))
        );
        assert_eq!(l.reply(0xb, &p), Some(rows[0]));
        assert_eq!(l.reply(0xc, &p), None, "a reply for another engine");
    }

    /// `[measured vfio-10 2860, the 2026-10-05 VFIO boot, RTX 4070]` `hChannel ff04000a, property 0, value 0xfa0` (4000 µs).
    #[test]
    fn the_timeslice_request_decodes() {
        let l = ChannelPropsLayout::at(V580_65_06).unwrap();
        let bytes = [
            0x0a, 0x00, 0x04, 0xff, 0, 0, 0, 0, 0xa0, 0x0f, 0, 0, 0, 0, 0, 0,
        ];
        assert_eq!(
            l.decode(&bytes),
            Some(ChannelProperty {
                channel: 0xff04_000a,
                property: 0,
                value: 4000
            })
        );
        assert_eq!(l.decode(&bytes[..12]), None);
    }
}
