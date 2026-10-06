//! Compare complete capability policies with their audited source fixtures.
#[path = "support/capability_snapshot.rs"]
mod capability_snapshot;

#[test]
fn pre_535_extension_policies_change_only_by_the_audited_software_constructors() {
    // Full IDs, names, provenance, denials and decision outputs; equal counts are insufficient.
    // See traces/capability_535_545_audit_20260928/README.md for baseline provenance.
    // Preserve the historical fixture. Later deltas are NV01_TIMER and the
    // source-audited Deferred API software constructor at580+; no controls added.
    // NV01_TIMER is
    // audited from OGKM in traces/windows_pool_20261005/timer-source-audit.tsv.
    // Pin its complete row and compare every other decision byte-for-byte.
    const TIMER: &str =
        "CLASS 00000004 NV01_TIMER Mode2Rpc => Listed { name: \"NV01_TIMER\", origin: Mode2Rpc }\n";
    const DEFERRED: &str = "CLASS 00005080 NV50_DEFERRED_API_CLASS Mode2Rpc => Listed { name: \"NV50_DEFERRED_API_CLASS\", origin: Mode2Rpc }\n";
    let current = capability_snapshot::existing_policy_snapshot();
    assert_eq!(current.matches(TIMER).count(), 8);
    assert_eq!(current.matches(DEFERRED).count(), 2);
    assert_eq!(
        current.replace(TIMER, "").replace(DEFERRED, ""),
        include_str!("fixtures/capability_550_610_before_535.txt")
    );
}

#[test]
fn timer_allocation_does_not_authorize_timer_controls() {
    use kf_abi::capability::ALL_BOUNDARIES;
    use kf_arch::ids::{ClassId, ControlCmd};
    for table in ALL_BOUNDARIES {
        assert!(table.alloc_class(ClassId(4)).is_permitted());
        // Complete ctrl0004.h command list: NULL and SET_ALARM_NOTIFY.
        for command in [0x0004_0000, 0x0004_0110] {
            assert!(!table.control(ControlCmd(command)).is_permitted());
        }
    }
}

#[test]
fn legacy_shared_groups_match_each_versions_compiled_headers() {
    use kf_abi::capability::{CAPS_535_104_05, CAPS_545_23_06};
    use std::collections::BTreeMap;

    let prefixes = [
        "NV00FD_",
        "NV9096_",
        "NV906F_",
        "NV208F_",
        "NV90E6_",
        "NV_CONF_COMPUTE_",
        "NV_SEMAPHORE_SURFACE_",
    ];
    for (tag, caps, compiled) in [
        (
            "535.104.05",
            &CAPS_535_104_05,
            include_str!("fixtures/capability_shared_535.104.05.tsv"),
        ),
        (
            "545.23.06",
            &CAPS_545_23_06,
            include_str!("fixtures/capability_shared_545.23.06.tsv"),
        ),
    ] {
        let mut values = BTreeMap::new();
        for line in compiled.lines() {
            let cols: Vec<_> = line.split('\t').collect();
            assert_eq!(cols.len(), 3, "malformed compiler evidence at {tag}");
            assert_eq!(cols[0], "legacy_shared_controls");
            let value: u32 = cols[2].parse().expect("compiled integer fits u32");
            assert!(
                values.insert(cols[1], value).is_none(),
                "duplicate evidence row"
            );
        }
        let checked: Vec<_> = caps
            .all_controls()
            .filter(|row| prefixes.iter().any(|prefix| row.name.starts_with(prefix)))
            .collect();
        assert_eq!(
            checked.len(),
            16,
            "review any change to the audited set at {tag}"
        );
        for row in checked {
            assert_eq!(values.get(row.name), Some(&row.cmd), "{tag}: {}", row.name);
        }
    }
}

#[test]
fn deferred_api_constructor_admission_does_not_authorize_deferred_execution_controls() {
    use kf_abi::capability::ALL_BOUNDARIES;
    use kf_arch::ids::ControlCmd;
    for table in ALL_BOUNDARIES {
        // ctrl5080.h: NULL, deprecated registration, remove, V2 registration, internal registration.
        for cmd in [0x50800000, 0x50800101, 0x50800102, 0x50800103, 0x50800104] {
            assert!(!table.control(ControlCmd(cmd)).is_permitted());
        }
    }
}
