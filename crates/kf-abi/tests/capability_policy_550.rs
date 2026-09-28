#[path = "support/capability_snapshot.rs"]
mod capability_snapshot;

#[test]
fn pre_535_extension_550_and_newer_policies_are_identical() {
    // Full IDs, names, provenance, denials and decision outputs; equal counts are insufficient.
    // See traces/capability_535_545_audit_20260928/README.md for baseline provenance.
    assert_eq!(
        capability_snapshot::existing_policy_snapshot(),
        include_str!("fixtures/capability_550_610_before_535.txt")
    );
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
