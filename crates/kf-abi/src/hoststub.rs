//! ★ 2026-10-07 (Windows Code43): the host-owned stub set for a Windows guest — controls with no
//! public body that kayfabe answers `NV_OK` with zeroed params, under owner ruling §S as
//! **ASSUMED, owner to confirm** (`docs/OWNER_RULINGS.md` §S, "§S applied to `0x2081010d`").
//!
//! Every id here is GENERATED (`hoststub_generated.rs`, by `tools/windows-ctrl-export/derive.py`):
//! the interface value comes from public OGKM 580.65.06 (`g_finn_rm_api.h`), the message number
//! from `tools/windows-ctrl-export/stubs.txt` (the bisect that selected it), and `paramSize` and
//! the export flags from the control's `NVOC_EXPORTED_METHOD_DEF` row in the pinned retail Windows
//! 580.88 `nvlddmkm.sys` (OGKM 580.65.06 `src/nvidia/inc/libraries/nvoc/runtime.h:73-84`).
//!
//! ⊘ A cell is per driver build: it applies only to a guest whose declared identity is that
//! Windows build at that wire version (the same identity rule as [`crate::sw_runlist`]). No
//! nearest-version fallback and no Linux guest.

#[path = "hoststub_generated.rs"]
mod generated;

/// One stubbed control, as generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StubControl {
    /// The control id: `(interface << 8) | message`.
    pub cmd: u32,
    /// The OGKM FINN interface macro the high 24 bits come from.
    pub interface: &'static str,
    /// The message number (the only non-public part; see `stubs.txt`).
    pub message: u32,
    /// `paramSize` of the retail export row: the exact params size the guest must declare.
    pub param_size: usize,
    /// `flags` of the retail export row (`RMCTRL_FLAGS_*`, OGKM `control.h`).
    pub export_flags: u32,
    /// Where the export row is in the pinned PE (audit identity only).
    pub export_row_rva: u32,
    /// Why the control is in the set.
    pub reason: &'static str,
}

/// The stub set of one guest driver build.
#[derive(Debug, Clone, Copy)]
pub struct StubCell {
    /// Public wire-layout tag the Windows build speaks.
    pub wire_version: crate::DriverVersion,
    /// Published Windows build name.
    pub windows_name: &'static str,
    /// The stubbed controls.
    pub controls: &'static [StubControl],
}

/// The cell for `version`, if one is generated. No nearest-version fallback.
#[must_use]
pub fn cell(version: crate::DriverVersion) -> Option<&'static StubCell> {
    generated::CELLS.iter().find(|c| c.wire_version == version)
}

impl StubCell {
    /// Whether the guest's `SET_GUEST_SYSTEM_INFO` payload declares exactly this build. An
    /// identity selects a cell; it is not a security boundary (a hostile guest can lie, and every
    /// answer in the cell is inert).
    #[must_use]
    pub fn matches_identity(&self, payload: &[u8]) -> bool {
        crate::guestsysinfo::ReportedDriver::decode(payload).is_ok_and(|id| {
            id.version == Some(self.wire_version)
                && id.twin.is_some_and(|t| t.win_name == self.windows_name)
        })
    }

    /// The stubbed control `cmd`, if it is in this cell.
    #[must_use]
    pub fn control(&self, cmd: u32) -> Option<&'static StubControl> {
        self.controls.iter().find(|c| c.cmd == cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_generated_ids_compose_from_their_parts_and_are_unique() {
        for c in generated::CELLS {
            let mut ids: Vec<u32> = c.controls.iter().map(|s| s.cmd).collect();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), c.controls.len());
            for s in c.controls {
                assert_eq!(s.cmd & 0xff, s.message);
                assert!(s.param_size < 0x1_0000);
            }
        }
    }

    #[test]
    fn the_bisect_control_is_generated_for_windows_580_88_only() {
        let v = crate::DriverVersion {
            major: 580,
            minor: 65,
            patch: 6,
        };
        let c = cell(v).expect("580.65.06 cell");
        assert_eq!(c.windows_name, "580.88");
        let s = c.control(0x2081_010d).expect("the bisect's control");
        assert_eq!(s.interface, "FINN_NV2081_BINAPI_INTERFACE_ID");
        assert_eq!(s.param_size, 0);
        // NON_PRIVILEGED (0x8) | ROUTE_TO_VGPU_HOST (0x200) | GSP_PLUGIN_FOR_VGPU_GSP (0x10000).
        assert_eq!(s.export_flags, 0x1_0208);
        assert!(c.control(0x2080_a801).is_none());
        assert!(
            cell(crate::DriverVersion {
                major: 580,
                minor: 159,
                patch: 4
            })
            .is_none()
        );
    }
}
