//! Canonical resolved policy, independent of shared/own block organization.
//! The fixture was captured from pre-extension revision 8ab92bf4, not this candidate.

use kf_abi::capability::*;
use kf_arch::ids::{ClassId, ControlCmd};
use std::fmt::Write;

pub fn existing_policy_snapshot() -> String {
    let mut out = String::new();
    writeln!(
        out,
        "GSS_MASK={RM_GSS_LEGACY_MASK:08x} BINAPI_CLASS={NV2081_BINAPI_CLASS:08x}"
    )
    .unwrap();
    let boundaries = [
        ("550.54.04", &CAPS_550_54_04),
        ("550.90.07", &CAPS_550_90_07),
        ("555.42.02", &CAPS_555_42_02),
        ("560.28.03", &CAPS_560_28_03),
        ("570.86.15", &CAPS_570_86_15),
        ("575.51.02", &CAPS_575_51_02),
        ("580.65.06", &CAPS_580_65_06),
        ("610.43.02", &CAPS_610_43_02),
    ];
    for (version, table) in boundaries {
        writeln!(out, "BOUNDARY {version}").unwrap();
        let mut rows = Vec::new();
        for row in table.all_controls() {
            rows.push(format!(
                "CONTROL {:08x} {} {:?} => {:?}",
                row.cmd,
                row.name,
                row.origin,
                table.control(ControlCmd(row.cmd))
            ));
        }
        for row in table.all_classes() {
            rows.push(format!(
                "CLASS {:08x} {} {:?} => {:?}",
                row.class,
                row.name,
                row.origin,
                table.alloc_class(ClassId(row.class))
            ));
        }
        for row in table.all_denied_controls() {
            rows.push(format!(
                "DENY_CONTROL {:08x} {} {:?} => {:?}",
                row.id,
                row.name,
                row.why,
                table.control(ControlCmd(row.id))
            ));
        }
        for row in table.all_denied_classes() {
            rows.push(format!(
                "DENY_CLASS {:08x} {} {:?} => {:?}",
                row.id,
                row.name,
                row.why,
                table.alloc_class(ClassId(row.id))
            ));
        }
        // Explicit representatives of unknown, legacy-bit, binary-API and overlapping rules.
        // Existing capability unit tests additionally pin denial precedence and rule coverage.
        for id in [
            0,
            0x7fff,
            0x8000,
            0xffff,
            0x20810000,
            0x20817fff,
            0x20818000,
            0x2081ffff,
            0xdead0001,
            0xdead8001,
            u32::MAX,
        ] {
            rows.push(format!(
                "PROBE_CONTROL {id:08x} => {:?}",
                table.control(ControlCmd(id))
            ));
            rows.push(format!(
                "PROBE_CLASS {id:08x} => {:?}",
                table.alloc_class(ClassId(id))
            ));
        }
        rows.sort();
        for row in rows {
            writeln!(out, "{row}").unwrap();
        }
    }
    out
}
