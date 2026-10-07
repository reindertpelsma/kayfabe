//! ★ 2026-10-07 (Windows Code43, run42): `NV2080_CTRL_CMD_INTERNAL_GR_GET_FECS_TRACE_HW_ENABLE`
//! at the GUEST's measured layout. Id and layout come from the driver matrix
//! (`crate::generated::matrix`, measured per ogkm tag); a version where either is absent gets
//! `None`, and the control stays unserviced.

use crate::DriverVersion;
use crate::generated::matrix as m;
use crate::matrix::Resolved;

/// Where one version's `NV2080_CTRL_INTERNAL_GR_FECS_TRACE_HW_ENABLE_PARAMS` puts its fields, and
/// the GET command id at that version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FecsTraceLayout {
    /// `NV2080_CTRL_CMD_INTERNAL_GR_GET_FECS_TRACE_HW_ENABLE` at the version.
    pub cmd: u32,
    /// `sizeof` the params.
    pub size: usize,
    /// `grRouteInfo` (16 bytes: `flags`, `route`).
    pub route_off: usize,
    /// `sizeof grRouteInfo`.
    pub route_size: usize,
    /// `bEnable` (`NvBool`, one byte).
    pub enable_off: usize,
}

impl FecsTraceLayout {
    /// The layout at `version`; `None` where the control or its struct is not measured there.
    #[must_use]
    pub fn at(version: DriverVersion) -> Option<FecsTraceLayout> {
        let cmd = m::CTRL_CMDS_NV2080_CTRL_CMD_INTERNAL_GR_GET_FECS_TRACE_HW_ENABLE
            .at_u32(version)
            .ok()??;
        let l = Resolved::of(
            &m::NV2080_CTRL_INTERNAL_GR_FECS_TRACE_HW_ENABLE_PARAMS,
            version,
        )
        .ok()?;
        let route = l.need("grRouteInfo").ok()?;
        let enable = l.need("bEnable").ok()?;
        Some(FecsTraceLayout {
            cmd,
            size: l.size(),
            route_off: route.off(),
            route_size: route.bytes()?,
            enable_off: enable.off(),
        })
    }

    /// Whether `params` names the default route: an all-zero `grRouteInfo`, i.e. `flags` =
    /// `NV2080_CTRL_GR_ROUTE_INFO_FLAGS_TYPE_NONE` (0, `ogkm-580.65.06:
    /// src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080gr.h:52`) and no route — the device's only
    /// GR engine, which is all kayfabe offers (no MIG).
    #[must_use]
    pub fn route_is_default(&self, params: &[u8]) -> bool {
        params
            .get(self.route_off..self.route_off + self.route_size)
            .is_some_and(|r| r.iter().all(|b| *b == 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_580_65_06_layout_is_the_measured_one() {
        let l = FecsTraceLayout::at(DriverVersion {
            major: 580,
            minor: 65,
            patch: 6,
        })
        .expect("measured");
        assert_eq!(l.cmd, 0x2080_0a38);
        assert_eq!(
            (l.size, l.route_off, l.route_size, l.enable_off),
            (24, 0, 16, 16)
        );
        let mut p = [0u8; 24];
        assert!(l.route_is_default(&p));
        p[8] = 1;
        assert!(!l.route_is_default(&p));
    }
}
