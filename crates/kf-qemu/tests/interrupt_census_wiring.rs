//! PF1: the observer must reach the real publication and interrupt paths.

#[test]
fn the_measurement_is_explicitly_opt_in() {
    let chan = include_str!("../src/chan.rs");
    assert!(
        chan.contains("KF3_DIAG_INTR"),
        "missing default-off interrupt census"
    );
    assert!(chan.contains("Census::from_setting"));
}

#[test]
fn real_publications_and_delivery_have_observer_hooks() {
    let device = include_str!("../src/device.rs");
    let pump = include_str!("../src/userpump.rs");
    assert!(
        device.contains("diag.delivery("),
        "eventfd outcome is unobserved"
    );
    assert_eq!(pump.matches("diag.marker(").count(), 3);
}

#[cfg(feature = "diag-intr")]
#[test]
fn disabled_builds_compile_out_every_hook() {
    for source in [
        include_str!("../src/device.rs"),
        include_str!("../src/chan.rs"),
        include_str!("../src/userpump.rs"),
    ] {
        for (at, _) in source.match_indices("if let Some(diag)") {
            let prefix = &source[at.saturating_sub(80)..at];
            assert!(prefix.contains("#[cfg(feature = \"diag-intr\")]"));
        }
    }
}
