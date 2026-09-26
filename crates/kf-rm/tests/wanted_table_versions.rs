//! ★★★ The per-control version gate (`docs/design/V3_DRIVER_MATRIX.md` §4.4) quantifies over
//! every served control, and a name it cannot find must be a RED test — never a silently
//! skipped gate (`the_missing_table_row_never_denies`).

use kf_abi::DriverVersion;
use kf_abi::generated::matrix::{ALL_STRUCTS, MEASURED};
use kf_rm::inittables::{WantedTable, layout_differs_from_bench};

/// Every params type a served control declares is in the driver matrix — a typo in a
/// `c_type()` name would otherwise disable that control's version gate without a sound.
#[test]
fn every_served_controls_params_type_is_measured() {
    let mut named = 0;
    for w in WantedTable::ALL {
        let Some(ct) = w.c_type() else { continue };
        named += 1;
        assert!(
            ALL_STRUCTS.iter().any(|r| r.name == ct),
            "{w:?} declares params type {ct}, which the driver matrix does not carry — add it to \
             tools/drivermatrix/consumed.txt and regenerate"
        );
    }
    assert!(named >= 40, "only {named} served controls declare a params type");
}

/// ★ Every served control passes the gate at every measured 580.x tag (the layouts the
/// encoders were written for), and the gate FIRES where the matrix says a layout differs.
#[test]
fn the_gate_passes_the_580_branch_and_fires_where_the_layout_moved() {
    for v in MEASURED.iter().filter(|v| v.major == 580) {
        for w in WantedTable::ALL {
            if let Some(ct) = w.c_type() {
                assert_eq!(layout_differs_from_bench(ct, *v), None, "{w:?} ({ct}) at {v}");
            }
        }
    }
    // Known-positives, measured: FB_GET_INFO_V2 is 460 bytes at 575.x, KGR_GET_GLOBAL_SM_ORDER
    // 26912 at 575.x, INTR_GET_KERNEL_TABLE 2068 at every tag <= 575.64.05.
    let v575 = DriverVersion { major: 575, minor: 57, patch: 8 };
    assert_eq!(layout_differs_from_bench("NV2080_CTRL_FB_GET_INFO_V2_PARAMS", v575), Some(460));
    assert_eq!(
        layout_differs_from_bench("NV2080_CTRL_INTERNAL_STATIC_KGR_GET_GLOBAL_SM_ORDER_PARAMS", v575),
        Some(26912)
    );
    assert_eq!(
        layout_differs_from_bench("NV2080_CTRL_INTERNAL_INTR_GET_KERNEL_TABLE_PARAMS", v575),
        Some(2068)
    );
    // An unmeasured version and an unknown type both mean "no statement" — the size check stays.
    assert_eq!(
        layout_differs_from_bench("NV2080_CTRL_FB_GET_INFO_V2_PARAMS", DriverVersion { major: 580, minor: 159, patch: 3 }),
        None
    );
    assert_eq!(layout_differs_from_bench("NO_SUCH_TYPE", v575), None);
}
