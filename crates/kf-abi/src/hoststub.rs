//! ★ 2026-10-07 (Windows Code43): the host-owned stub set for a Windows guest - controls with no
//! public body that kayfabe answers `NV_OK` with zeroed params, under owner ruling §S as
//! **ASSUMED, owner to confirm** (`docs/OWNER_RULINGS.md` §S, "§S applied to `0x2081010d`").
//!
//! ★ 2026-10-31 (provenance fix): the table below is HAND-MAINTAINED, and every item carries a
//! [`crate::provenance::Row`] with a source tag. The earlier generated table also carried two
//! values read from a closed driver binary (the control's export flags and the row's position in
//! the file); both are gone, nothing branched on them (they were only logged), and the generator
//! that read the binary is deleted. What remains is publicly verifiable:
//!
//! - the interface value `0x208101` is OGKM 580.65.06's `FINN_NV2081_BINAPI_INTERFACE_ID`
//!   (`src/common/sdk/nvidia/inc/g_finn_rm_api.h:425`);
//! - the message number `0xd` (so the id `0x2081010d`) and the declared params size `0` are
//!   MEASURED: the Windows 580.88 guest sends this control with zero params bytes
//!   (`traces/windows_code43_walls_20261007/run41-requests.log:95`), and the run38-41 bisect
//!   (`traces/windows_code43_walls_20261007/README.md`, "Run41 result") showed that answering it
//!   alone moves StartDevice past VFIO 2861.
//!
//! ⊘ A cell is per driver build: it applies only to a guest whose declared identity is that
//! Windows build at that wire version (the same identity rule as [`crate::sw_runlist`]). No
//! nearest-version fallback and no Linux guest.

use crate::provenance::{Evidence, Row};

/// One stubbed control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StubControl {
    /// The control id: `(interface_id << 8) | message`.
    pub cmd: u32,
    /// The OGKM FINN interface macro the high 24 bits come from.
    pub interface: &'static str,
    /// That macro's value in OGKM 580.65.06.
    pub interface_id: u32,
    /// The message number (the low byte of the id; taken from the wire, see the module documentation).
    pub message: u32,
    /// The exact params size the guest must declare (taken from the wire; see `rows`).
    pub param_size: usize,
    /// Why the control is in the set.
    pub reason: &'static str,
    /// The source of each item: `interface_id`, `message`, `param_size`.
    pub rows: &'static [Row],
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

/// The items every [`StubControl`] must give a provenance row for.
pub const CONTROL_ITEMS: &[&str] = &["interface_id", "message", "param_size"];

/// The Windows 580.88 cell. Public sources only; the bisect that selected the control is
/// `traces/windows_code43_walls_20261007/README.md` ("Run41 result").
static CELLS: &[StubCell] = &[StubCell {
    wire_version: crate::DriverVersion {
        major: 580,
        minor: 65,
        patch: 6,
    },
    windows_name: "580.88",
    controls: &[StubControl {
        cmd: 0x2081_010d,
        interface: "FINN_NV2081_BINAPI_INTERFACE_ID",
        interface_id: 0x20_8101,
        message: 0xd,
        param_size: 0,
        reason: "bisect-run38-41",
        rows: &[
            Row {
                item: "interface_id",
                source: "ogkm-580.65.06:src/common/sdk/nvidia/inc/g_finn_rm_api.h:425",
                evidence: &[],
            },
            // The guest sends this id on the wire: refused in run40 (the guest read a non-OK
            // status), answered in run41 and again by the product stub in run53.
            Row {
                item: "message",
                source: "measured:AD104,580.88,2026-10-07",
                evidence: &[
                    Evidence {
                        at: "traces/windows_code43_walls_20261007/run40-requests.log:95",
                        needle: "fn76/0x2081010d",
                    },
                    Evidence {
                        at: "traces/windows_code43_walls_20261007/run41-requests.log:95",
                        needle: "DIAG-ZERO-OK 0x2081010d",
                    },
                ],
            },
            // The declared params size on the wire is 0 (the diagnostic zeroed exactly the
            // declared size); the product stub answers only a declared size equal to this and
            // was answered at run53.
            Row {
                item: "param_size",
                source: "measured:AD104,580.88,2026-10-07",
                evidence: &[
                    Evidence {
                        at: "traces/windows_code43_walls_20261007/run41-requests.log:95",
                        needle: "0 params byte(s) zeroed",
                    },
                    Evidence {
                        at: "traces/windows_code43_walls_20261007/run53-requests.log:526",
                        needle: "cmd=0x2081010d",
                    },
                ],
            },
        ],
    }],
}];

/// The cell for `version`, if one exists. No nearest-version fallback.
#[must_use]
pub fn cell(version: crate::DriverVersion) -> Option<&'static StubCell> {
    CELLS.iter().find(|c| c.wire_version == version)
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
    fn the_ids_compose_from_their_parts_and_are_unique() {
        for c in CELLS {
            let mut ids: Vec<u32> = c.controls.iter().map(|s| s.cmd).collect();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), c.controls.len());
            for s in c.controls {
                assert_eq!(s.cmd & 0xff, s.message);
                assert_eq!(s.cmd >> 8, s.interface_id);
                assert!(s.param_size < 0x1_0000);
            }
        }
    }

    /// ★ 2026-10-31: the table was generated until this date. The values the Windows path reads
    /// (cell key, identity, control id, interface, message, declared size, reason) are pinned here
    /// to the generated table's bytes, so the hand-maintained rewrite changes none of them.
    #[test]
    fn the_table_is_byte_identical_to_the_generated_one_it_replaced() {
        assert_eq!(CELLS.len(), 1);
        let c = &CELLS[0];
        assert_eq!(
            c.wire_version,
            crate::DriverVersion {
                major: 580,
                minor: 65,
                patch: 6
            }
        );
        assert_eq!(c.windows_name, "580.88");
        assert_eq!(c.controls.len(), 1);
        let s = &c.controls[0];
        assert_eq!(s.cmd, 0x2081010d);
        assert_eq!(s.interface, "FINN_NV2081_BINAPI_INTERFACE_ID");
        assert_eq!(s.message, 0xd);
        assert_eq!(s.param_size, 0);
        assert_eq!(s.reason, "bisect-run38-41");
    }

    #[test]
    fn every_row_has_an_accepted_source_tag() {
        for c in CELLS {
            for s in c.controls {
                crate::provenance::check_rows(s.rows, CONTROL_ITEMS).expect("provenance rows");
            }
        }
    }

    #[test]
    fn the_bisect_control_exists_for_windows_580_88_only() {
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
