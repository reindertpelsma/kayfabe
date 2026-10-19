//! P1's full-ring-only GET experiment must be absent from production builds.

#[test]
fn the_experiment_has_a_separate_opt_in_feature_and_cannot_be_default() {
    let manifest = include_str!("../Cargo.toml");
    assert!(manifest.contains("diag-get-writeback = []"));
    assert!(!manifest.contains("default = [\"diag-get-writeback\"]"));
    let relay = include_str!("../src/user_turn.rs");
    assert!(
        relay
            .contains("#[cfg(feature = \"diag-get-writeback\")]\n    pub diag_full_ring_get: bool")
    );
}

#[test]
fn suppressed_writeback_retains_the_real_guest_visible_cursor() {
    let relay = include_str!("../src/user_turn.rs");
    assert!(relay.contains("if !self.defer_guest_get()"));
    assert!(
        relay.contains(
            "self.written_get = get;\n                            self.guest_get = None;"
        )
    );
}

#[test]
fn a_new_full_ring_is_serviced_before_loading_its_next_entry() {
    let relay = include_str!("../src/user_turn.rs");
    let put = relay.split_once("fn put_step(").unwrap().1;
    let guard = put.find("self.diag_full_ring_get && self.put_full && self.guest_get.is_some()");
    assert!(
        guard.is_some(),
        "missing full-ring writeback before entry/gate work"
    );
    assert!(guard.unwrap() < put.find("io.begin_entry(put)").unwrap());
}
