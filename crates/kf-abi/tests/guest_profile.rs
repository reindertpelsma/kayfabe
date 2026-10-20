//! C6: no-environment configuration contracts for the explicitly selected guest profile.
//! The captured caps probe is excluded by policy; its production replacement is a separate gate.

use kf_abi::guest_profile::{GuestProfile, P19_NAMES};

fn p19_expected() -> Vec<(&'static str, bool)> {
    vec![
        ("KF3_WIN_USER_CHANNELS_PASSTHROUGH", true),
        ("KF3_WIN_TWIN_DEFAPI_OBJECT", true),
        ("KF3_KERNEL_GR_CE", true),
        ("KF3_KERNEL_GR_WORK", true),
        ("KF3_TRANSLATED_CE_RELAY", true),
        ("KF3_DEFERRED_API", true),
        ("KF3_PREEMPT_BIND_PROBE", true),
        ("KF3_DISPLAY_IMP_ENABLE", true),
        ("KF3_DISPLAY_CTRL_PROBE", true),
        ("KF3_ZCULL_BIND_PROBE", true),
        ("KF3_SW_RUNLIST_HOST_OWNED", true),
        ("KF3_ASYNC_PREEMPT", true),
        ("KF3_GFX_POOL_PROBE", true),
        ("KF3_SW_RUNLIST_PROBE", true),
        ("KF3_MEMORY_LIST_PROBE", true),
        ("KF3_DISPLAY_SDR_COLOR", true),
        ("KF3_DISPLAY_CAPS_PROBE", false),
        ("KF3_DISPLAY_LUT_MIRROR", true),
        ("KF3_DISPLAY_ILUT_OFFSET_256", true),
    ]
}

#[test]
fn windows_no_env_configuration_has_one_row_per_p19_selector() {
    let settings = GuestProfile::parse(Some("windows")).unwrap().settings();
    let expected = p19_expected();
    assert_eq!(
        P19_NAMES.as_slice(),
        expected.iter().map(|r| r.0).collect::<Vec<_>>()
    );
    for (name, value) in expected {
        assert_eq!(settings.value(name), Some(value), "{name}");
    }
    assert!(
        settings.derived_caps,
        "CAPS_PROBE must be replaced with derived caps"
    );
}

#[test]
fn windows_dump_preserves_every_required_behavior_without_captured_caps() {
    let values = GuestProfile::Windows.settings();
    let enabled = P19_NAMES
        .iter()
        .filter(|&&n| values.value(n) == Some(true))
        .count();
    assert_eq!(enabled, 18);
    for (name, _) in p19_expected()
        .into_iter()
        .filter(|r| r.0 != "KF3_DISPLAY_CAPS_PROBE")
    {
        assert_eq!(
            values.value(name),
            Some(true),
            "missing Windows behavior {name}"
        );
    }
    assert_eq!(values.value("KF3_DISPLAY_CAPS_PROBE"), Some(false));
    assert!(values.derived_caps);
}

#[test]
fn default_and_explicit_linux_keep_every_p19_behavior_off() {
    assert_eq!(GuestProfile::default(), GuestProfile::parse(None).unwrap());
    for input in [None, Some("linux")] {
        let profile = GuestProfile::parse(input).unwrap();
        assert_eq!(profile, GuestProfile::Linux);
        for name in P19_NAMES {
            assert_eq!(profile.settings().value(name), Some(false), "{name}");
        }
        assert!(!profile.settings().derived_caps);
    }
}

#[test]
fn a_default_profile_never_grants_windows_channel_or_display_behavior() {
    let settings = GuestProfile::default().settings();
    assert_eq!(settings, GuestProfile::Linux.settings());
    assert!(
        P19_NAMES
            .iter()
            .all(|name| settings.value(name) == Some(false))
    );
    assert!(!settings.derived_caps);
}

#[test]
fn invalid_property_values_are_refused_instead_of_guessed() {
    for bad in ["", "auto", "Windows", "linux ", "windows,debug=on", "other"] {
        assert!(GuestProfile::parse(Some(bad)).is_err(), "accepted {bad:?}");
    }
}

#[test]
fn an_unconfigured_alias_cannot_silently_become_linux() {
    for name in ["", "auto", "Windows", "linux ", "windows,debug=on", "other"] {
        let result = GuestProfile::parse(Some(name));
        assert!(result.is_err(), "{name:?} acquired an execution profile");
    }
}

#[test]
fn two_devices_can_select_different_profiles_without_process_state() {
    let linux = GuestProfile::parse(None).unwrap();
    let windows = GuestProfile::parse(Some("windows")).unwrap();
    assert_eq!(linux.settings(), GuestProfile::Linux.settings());
    assert_eq!(windows.settings(), GuestProfile::Windows.settings());
    assert_ne!(linux.settings(), windows.settings());
    assert_eq!(
        GuestProfile::parse(None).unwrap().settings(),
        linux.settings()
    );
}

#[test]
fn unknown_and_diagnostic_names_do_not_change_effective_boot_settings() {
    for profile in [GuestProfile::Linux, GuestProfile::Windows] {
        let settings = profile.settings();
        for name in [
            "KF3_DIAG_STEP_METERS",
            "KF3_DIAG_EVENT_RING",
            "KF3_RPC_TRACE",
            "KF3_MAPLOG",
            "KF3_UNKNOWN",
        ] {
            assert_eq!(
                settings.value(name),
                None,
                "diagnostic entered boot profile: {name}"
            );
        }
    }
}

#[test]
fn every_kept_optional_flag_is_outside_the_unset_boot_profile() {
    let policy = include_str!("../../../docs/design/V3_FLAG_POLICY_BETA_20261020.tsv");
    let mut checked = 0;
    for row in policy.lines().skip(1) {
        let fields: Vec<_> = row.split('\t').collect();
        if fields[3] != "KEEP" {
            continue;
        }
        assert_eq!(fields[4], "false", "{} required to boot", fields[0]);
        assert_eq!(fields[5], "false", "{} active when unset", fields[0]);
        for profile in [GuestProfile::Linux, GuestProfile::Windows] {
            assert_eq!(profile.settings().value(fields[0]), None);
        }
        checked += 1;
    }
    assert!(checked > 20);
}
