//! What the `negctl` feature does to the compiled library (`STATUS: LIVE, 2026-11-25`).
//!
//! The control switches are `KF3_NEGCTL_*` environment reads that exist only in a `negctl` build (a bench
//! build; the release kit does not set the feature). That a default build holds no such name is checked on the
//! built archive by `scripts/ci/negctl_artifact.py` (this test binary embeds source text, so it cannot be scanned).

/// The controls reachable from outside their module: constants in a default build, off with no variable in a bench build.
#[test]
fn the_public_controls_are_off_unless_a_variable_turns_them_on() {
    #[cfg(not(feature = "negctl"))]
    const {
        assert!(!crate::tspace::negctl_carve());
        assert!(!crate::tspace::negctl_oversize());
        assert!(!crate::mem::negctl_twin_window());
    }
    #[cfg(feature = "negctl")]
    {
        assert!(!crate::tspace::negctl_carve());
        assert!(!crate::tspace::negctl_oversize());
        assert!(!crate::mem::negctl_twin_window());
    }
}

/// The `realize` line that names a bench build is behind the gate, and the scripts that run a control read it.
#[test]
fn the_realize_line_is_gated_and_the_control_scripts_look_for_it() {
    const LINE: &str = "NEGCTL feature compiled in (bench build)";
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let read = |rel: &str| {
        std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
    };
    let dev = read("crates/kf-qemu/src/device.rs");
    let at = dev.find(LINE).expect("device.rs prints the line");
    let before = &dev[..at];
    let attr = before
        .rfind("#[cfg(feature = \"negctl\")]")
        .expect("a gate before the line");
    assert!(
        !before[attr..].contains(';'),
        "the gate must be on the statement that prints the line"
    );
    assert_eq!(dev.matches(LINE).count(), 1);
    for script in [
        "scripts/bench/sec/libcuda_channel_census.sh",
        "scripts/p1p2/tspace_log_gate.py",
    ] {
        assert!(
            read(script).contains(LINE),
            "{script} must look for the realize line"
        );
    }
}
