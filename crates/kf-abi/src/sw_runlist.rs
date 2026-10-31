//! Exact-cell wire evidence for the default-off software-runlist allocation diagnostic.
//! The public SDK defines the parameters, but not the class number: that is MEASURED on the wire.
//! This evidence is deliberately separate from normal capabilities.
//!
//! ★ 2026-10-31 (provenance fix): the table below is HAND-MAINTAINED and every item carries a
//! [`crate::provenance::Row`] with a source tag. It was generated until this date from the pinned
//! closed Windows driver (the class registration row, the caller's control instructions); that
//! generator and everything only it supplied are gone. The class and the control's size are now
//! the values the guest puts on the wire (committed traces), the parameter layouts are OGKM
//! 580.65.06 (`nvos.h`, `g_rpc-structures.h`). No value in this file is read from a closed binary.

use crate::provenance::{Evidence, Row};

/// One layout set and the numbers seen on the wire; no family/die facts.
#[derive(Debug, Clone, Copy)]
pub struct EvidenceCell {
    /// Public wire-layout tag, independently associated with the Windows build.
    pub wire_version: crate::DriverVersion,
    /// Published Windows build name.
    pub windows_name: &'static str,
    /// The class number the guest allocates (seen on the wire; not an OGKM define).
    pub class: u32,
    /// `sizeof(NV_SWRUNLIST_ALLOCATION_PARAMS)` in OGKM 580.65.06.
    pub size: usize,
    /// Offset of `engineId` in that struct.
    pub engine: usize,
    /// Offset of `maxTSGs` in that struct.
    pub max_tsgs: usize,
    /// Offset of `qosIntrEnableMask` in that struct.
    pub qos: usize,
    /// The control code the guest sends after the allocation (seen on the wire); observational only,
    /// never an allowlist.
    pub observed_control: u32,
    /// The exact parameter size the guest declares for that control (seen on the wire).
    pub observed_control_size: usize,
    /// Compiled fixed `rpc_alloc_memory_v13_01` size; its PTE tail is not decoded.
    pub alloc_memory_size: usize,
    /// Compiled scalar prefix fields only: name, offset, width. No PTE/address fields.
    pub alloc_memory_fields: &'static [(&'static str, usize, usize)],
    /// The source of each item in [`ITEMS`].
    pub rows: &'static [Row],
}

/// The items every [`EvidenceCell`] must give a provenance row for.
pub const ITEMS: &[&str] = &[
    "class",
    "params_layout",
    "params_binding",
    "observed_control",
    "alloc_memory_layout",
];

/// The Windows 580.88 cell (public wire tag 580.65.06).
static CELLS: &[EvidenceCell] = &[EvidenceCell {
    wire_version: crate::DriverVersion {
        major: 580,
        minor: 65,
        patch: 6,
    },
    windows_name: "580.88",
    class: 0xb297,
    size: 12,
    engine: 0,
    max_tsgs: 4,
    qos: 8,
    observed_control: 0x2080_1111,
    observed_control_size: 40,
    alloc_memory_size: 56,
    alloc_memory_fields: &[
        ("hClient", 0, 4),
        ("hDevice", 4, 4),
        ("hMemory", 8, 4),
        ("hClass", 12, 4),
        ("flags", 16, 4),
        ("pteAdjust", 20, 4),
        ("format", 24, 4),
        ("length", 32, 8),
        ("pageCount", 40, 4),
    ],
    rows: &[
        // The number appears in the guest's RmAlloc on the wire (refused in the pool probe,
        // accepted under the opt-in in run53).
        Row {
            item: "class",
            source: "measured:AD104,580.88,2026-10-05",
            evidence: &[
                Evidence {
                    at: "traces/windows_pool_20261005/probe-b/qemu.log:1122",
                    needle: "RmAlloc seq=0 class=0xb297",
                },
                Evidence {
                    at: "traces/windows_code43_walls_20261007/run53-requests.log:196",
                    needle: "RmAlloc seq=0 class=0xb297",
                },
                Evidence {
                    at: "traces/windows_code43_walls_20261007/run53-requests.log:197",
                    needle: "RmAlloc seq=0 class=0xb297",
                },
            ],
        },
        // The struct layout (size and the three field offsets) is OGKM's.
        Row {
            item: "params_layout",
            source: "ogkm-580.65.06:src/common/sdk/nvidia/inc/nvos.h:2924-2929",
            evidence: &[],
        },
        // That the class's parameters ARE this struct is shown by a working boot: with the opt-in
        // the allocation decodes as exactly this 12-byte layout (maxTSGs 0, QoS 0, an advertised
        // engine) and is accepted. This proves the decode, not the field meanings.
        Row {
            item: "params_binding",
            source: "measured:AD104,580.88,2026-10-08",
            evidence: &[
                Evidence {
                    at: "traces/windows_code43_walls_20261007/run53-command.json:75",
                    needle: "KF3_SW_RUNLIST_PROBE",
                },
                Evidence {
                    at: "traces/windows_code43_walls_20261007/run53-requests.log:196",
                    needle: "class=0xb297 client=0xc1d00002 parent=0xff030000 handle=0xff008050 result=0x0",
                },
            ],
        },
        // The guest sends this control with 40 declared bytes (the observer logs only a 40-byte
        // body, so this shows 40 is a declared size, not that no other is ever sent).
        Row {
            item: "observed_control",
            source: "measured:AD104,580.88,2026-10-07",
            evidence: &[Evidence {
                at: "traces/windows_code43_walls_20261007/run42-requests.log:71",
                needle: "control=0x20801111 params_bytes=40",
            }],
        },
        Row {
            item: "alloc_memory_layout",
            source: "ogkm-580.65.06:src/nvidia/generated/g_rpc-structures.h:79-91",
            evidence: &[],
        },
    ],
}];

/// The complete public three-word allocation declaration; all values are untrusted.
#[derive(Debug, Clone, Copy)]
pub struct Params {
    /// `NV2080_ENGINE_TYPE_*`, not an RM engine or physical runlist index.
    pub engine: u32,
    /// The value the Windows guest sends is zero (seen on the wire); `nvos.h` sizes the runlist from it.
    pub max_tsgs: u32,
    /// Requested QoS interrupt mask.
    pub qos: u32,
}

/// Look up only evidenced driver cells. No nearest-version or per-die fallback.
#[must_use]
pub fn cell(version: crate::DriverVersion) -> Option<&'static EvidenceCell> {
    CELLS.iter().find(|c| c.wire_version == version)
}

/// Recognize the diagnostic's class even on unsupported wire cells, to refuse it.
#[must_use]
pub fn is_probe_class(class: u32) -> bool {
    CELLS.iter().any(|c| c.class == class)
}

impl EvidenceCell {
    /// This identity selects an evidence cell, never a privilege/security boundary.
    #[must_use]
    pub fn matches_identity(&self, payload: &[u8]) -> bool {
        crate::guestsysinfo::ReportedDriver::decode(payload).is_ok_and(|id| {
            id.version == Some(self.wire_version)
                && id.twin.is_some_and(|t| t.win_name == self.windows_name)
        })
    }

    /// Decode exactly the compiled layout. Feature policy belongs to the caller.
    #[must_use]
    pub fn decode(&self, params: &[u8]) -> Option<Params> {
        if params.len() != self.size {
            return None;
        }
        let read = |off: usize| -> Option<u32> {
            Some(u32::from_le_bytes(
                params.get(off..off.checked_add(4)?)?.try_into().ok()?,
            ))
        };
        Some(Params {
            engine: read(self.engine)?,
            max_tsgs: read(self.max_tsgs)?,
            qos: read(self.qos)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v() -> crate::DriverVersion {
        crate::DriverVersion {
            major: 580,
            minor: 65,
            patch: 6,
        }
    }

    /// ★ 2026-10-31: the table was generated until this date. Every value the diagnostic reads is
    /// pinned here to the generated table's bytes, so the hand-maintained rewrite changes none.
    #[test]
    fn the_cell_is_byte_identical_to_the_generated_one_it_replaced() {
        assert_eq!(CELLS.len(), 1);
        let c = cell(v()).expect("580.65.06 cell");
        assert_eq!(c.windows_name, "580.88");
        assert_eq!(c.class, 0xb297);
        assert_eq!((c.size, c.engine, c.max_tsgs, c.qos), (12, 0, 4, 8));
        assert_eq!(c.observed_control, 0x2080_1111);
        assert_eq!(c.observed_control_size, 40);
        assert_eq!(c.alloc_memory_size, 56);
        assert_eq!(
            c.alloc_memory_fields,
            &[
                ("hClient", 0, 4),
                ("hDevice", 4, 4),
                ("hMemory", 8, 4),
                ("hClass", 12, 4),
                ("flags", 16, 4),
                ("pteAdjust", 20, 4),
                ("format", 24, 4),
                ("length", 32, 8),
                ("pageCount", 40, 4),
            ]
        );
        assert!(is_probe_class(0xb297));
        assert!(!is_probe_class(0xb298));
        assert!(
            cell(crate::DriverVersion {
                major: 580,
                minor: 159,
                patch: 4
            })
            .is_none()
        );
    }

    #[test]
    fn decode_reads_exactly_the_three_words_and_nothing_else() {
        let c = cell(v()).unwrap();
        let mut p = [0u8; 12];
        p[0..4].copy_from_slice(&7u32.to_le_bytes());
        p[4..8].copy_from_slice(&0u32.to_le_bytes());
        p[8..12].copy_from_slice(&9u32.to_le_bytes());
        let d = c.decode(&p).expect("12 bytes decode");
        assert_eq!((d.engine, d.max_tsgs, d.qos), (7, 0, 9));
        assert!(c.decode(&p[..11]).is_none());
        assert!(c.decode(&[0u8; 13]).is_none());
    }

    #[test]
    fn every_row_has_an_accepted_source_tag() {
        for c in CELLS {
            crate::provenance::check_rows(c.rows, ITEMS).expect("provenance rows");
        }
    }
}
