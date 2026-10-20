//! Structural checks of the drainer/act split, supplementing real cell tests.
//! Source pins do not simulate a GPU or prove host ioctl latency.

fn diagnostic_drainer() -> &'static str {
    include_str!("chan.rs")
        .split("    pub fn bar0trace_dump(")
        .nth(1)
        .expect("diagnostic entry")
        .split("    fn bar0trace_dump_act(")
        .next()
        .expect("diagnostic drainer half")
}

#[test]
fn translated_diagnostic_drainer_never_takes_the_pump_lock() {
    assert!(
        !diagnostic_drainer().contains("slot.lock()"),
        "default-off diagnostic still blocks the drainer behind a pump"
    );
}

#[test]
fn translated_diagnostic_drainer_queues_host_views_to_the_act_thread() {
    let drainer: String = diagnostic_drainer()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert!(
        !drainer.contains(".lock("),
        "drainer may not acquire a pump guard"
    );
    assert!(
        drainer.contains("self.defer("),
        "diagnostic must run as an act"
    );
    assert!(
        !drainer.contains("arm_cpu_view"),
        "host ioctl on the drainer"
    );
    assert!(
        !drainer.contains("release_cpu_view"),
        "host ioctl on the drainer"
    );
}

#[test]
fn translated_worker_enters_the_handoff_cell() {
    let source = include_str!("chan.rs");
    let serve = source
        .split("    pub fn serve(")
        .nth(1)
        .unwrap()
        .split("    pub fn pt_doorbells(")
        .next()
        .unwrap();
    assert!(
        serve.contains("slot.enter()"),
        "worker bypasses the handoff cell"
    );
}

#[test]
fn translated_worker_cannot_skip_registration_with_a_raw_try_lock() {
    let source = include_str!("chan.rs");
    let serve: String = source
        .split("    pub fn serve(")
        .nth(1)
        .unwrap()
        .split("    pub fn pt_doorbells(")
        .next()
        .unwrap()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert!(
        !serve.contains("slot.try_lock()"),
        "lost doorbell would have no obligation"
    );
}

#[test]
fn translated_group_schedule_holds_one_reply_for_every_member() {
    let source = include_str!("chan.rs");
    let group = source
        .split("        if !members.is_empty() {")
        .nth(1)
        .unwrap()
        .split("        // ★ P5b: a user channel")
        .next()
        .unwrap();
    let compact: String = group.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(compact.contains("returnself.defer("));
    assert!(compact.contains("apply_translated_schedule(slot,client,object,*ht,enable)?"));
    assert!(!compact.contains("ChanAnswer::Done"));
    assert!(
        compact.contains("Option<Vec<_>>"),
        "no silently dropped group member"
    );
}
